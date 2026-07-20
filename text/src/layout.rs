use unicode_bidi::{BidiInfo, LTR_LEVEL, RTL_LEVEL};
use unicode_linebreak::{BreakOpportunity, linebreaks};
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    FontCollection, FontMetrics, ShapedParagraph, TextDirection, TextError, TextErrorCode,
};

/// Width and work ceilings for greedy Unicode line layout.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TextLayoutOptions {
    max_width_bits: i32,
    max_lines: usize,
    max_shaping_attempts: usize,
    base_direction: Option<TextDirection>,
}

impl TextLayoutOptions {
    /// Creates layout options for one positive Q16.16 line width.
    pub fn new(max_width_bits: i32) -> Result<Self, TextError> {
        Self::with_limits(max_width_bits, 100_000, 1_000_000)
    }

    /// Creates layout options with explicit positive line and shaping ceilings.
    pub const fn with_limits(
        max_width_bits: i32,
        max_lines: usize,
        max_shaping_attempts: usize,
    ) -> Result<Self, TextError> {
        if max_width_bits <= 0 || max_lines == 0 || max_shaping_attempts == 0 {
            return Err(TextError::new(TextErrorCode::InvalidLimits));
        }
        Ok(Self {
            max_width_bits,
            max_lines,
            max_shaping_attempts,
            base_direction: None,
        })
    }

    /// Forces the same bidi base direction for every produced line.
    pub const fn with_base_direction(mut self, direction: TextDirection) -> Self {
        self.base_direction = Some(direction);
        self
    }

    /// Returns the maximum line width in Q16.16 canvas units.
    pub const fn max_width_bits(self) -> i32 {
        self.max_width_bits
    }

    /// Returns the maximum output line count.
    pub const fn max_lines(self) -> usize {
        self.max_lines
    }

    /// Returns the maximum number of candidate shaping operations.
    pub const fn max_shaping_attempts(self) -> usize {
        self.max_shaping_attempts
    }

    /// Returns the optional forced bidi base direction.
    pub const fn base_direction(self) -> Option<TextDirection> {
        self.base_direction
    }
}

/// One positioned line in a laid-out text block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShapedLine {
    paragraph: Option<ShapedParagraph>,
    source_start: u32,
    source_end: u32,
    baseline_y_bits: i32,
    hard_break: bool,
    metrics: FontMetrics,
}

impl ShapedLine {
    /// Borrows shaped content, or returns `None` for an empty line.
    pub const fn paragraph(&self) -> Option<&ShapedParagraph> {
        self.paragraph.as_ref()
    }

    /// Returns the inclusive source UTF-8 byte start.
    pub const fn source_start(&self) -> u32 {
        self.source_start
    }

    /// Returns the exclusive source UTF-8 byte end, excluding a line separator.
    pub const fn source_end(&self) -> u32 {
        self.source_end
    }

    /// Returns the baseline position relative to the text-block top.
    pub const fn baseline_y_bits(&self) -> i32 {
        self.baseline_y_bits
    }

    /// Returns whether an explicit mandatory separator ended this line.
    pub const fn hard_break(&self) -> bool {
        self.hard_break
    }

    /// Returns this line's baseline metrics.
    pub const fn metrics(&self) -> FontMetrics {
        self.metrics
    }
}

/// Greedily wrapped, vertically positioned text lines.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextLayout {
    lines: Vec<ShapedLine>,
    width_bits: i32,
    height_bits: i32,
}

impl TextLayout {
    /// Borrows lines in top-to-bottom order.
    pub fn lines(&self) -> &[ShapedLine] {
        &self.lines
    }

    /// Returns the maximum shaped line advance.
    pub const fn width_bits(&self) -> i32 {
        self.width_bits
    }

    /// Returns the sum of all automatic line heights.
    pub const fn height_bits(&self) -> i32 {
        self.height_bits
    }
}

impl FontCollection {
    /// Shapes and greedily wraps UTF-8 using Unicode line-break opportunities.
    pub fn layout_text(
        &self,
        text: &str,
        font_size_bits: i32,
        options: TextLayoutOptions,
    ) -> Result<TextLayout, TextError> {
        if self.faces().is_empty() {
            return Err(TextError::new(TextErrorCode::EmptyFontCollection));
        }
        if font_size_bits <= 0 {
            return Err(TextError::new(TextErrorCode::InvalidFontSize));
        }
        if text.len() > self.limits().max_text_bytes() || text.len() > u32::MAX as usize {
            return Err(TextError::new(TextErrorCode::ResourceLimit));
        }

        let mut breaks = Vec::new();
        breaks
            .try_reserve(text.len().saturating_add(1))
            .map_err(|_| TextError::new(TextErrorCode::AllocationFailed))?;
        breaks.extend(linebreaks(text));

        let mut builder = LayoutBuilder {
            fonts: self,
            text,
            bidi: BidiInfo::new(
                text,
                options.base_direction.map(|direction| match direction {
                    TextDirection::LeftToRight => LTR_LEVEL,
                    TextDirection::RightToLeft => RTL_LEVEL,
                }),
            ),
            font_size_bits,
            options,
            lines: Vec::new(),
            top_bits: 0,
            width_bits: 0,
            shaping_attempts: 0,
            total_runs: 0,
            total_glyphs: 0,
        };
        if text.is_empty() {
            builder.push_line(0, 0, false, None)?;
            return builder.finish();
        }

        let mut line_start = 0_usize;
        let mut trailing_empty = false;
        while line_start < text.len() {
            let mut last_fit: Option<LineCandidate> = None;
            let mut chosen: Option<LineCandidate> = None;
            let first_break = breaks.partition_point(|(index, _)| *index <= line_start);
            for &(raw_end, opportunity) in &breaks[first_break..] {
                let hard_break = opportunity == BreakOpportunity::Mandatory
                    && strip_mandatory_ending(text, line_start, raw_end) < raw_end;
                let content_end = if hard_break {
                    strip_mandatory_ending(text, line_start, raw_end)
                } else {
                    raw_end
                };
                let candidate =
                    builder.shape_candidate(line_start, content_end, raw_end, hard_break)?;
                if candidate.width_bits() <= options.max_width_bits {
                    last_fit = Some(candidate);
                    if opportunity == BreakOpportunity::Mandatory {
                        chosen = last_fit.take();
                        break;
                    }
                } else {
                    chosen = if let Some(fit) = last_fit.take() {
                        Some(fit)
                    } else {
                        Some(builder.force_grapheme_break(line_start, content_end)?)
                    };
                    break;
                }
            }
            let candidate = chosen
                .or(last_fit)
                .ok_or(TextError::new(TextErrorCode::InvalidLayout))?;
            if candidate.raw_end <= line_start {
                return Err(TextError::new(TextErrorCode::InvalidLayout));
            }
            trailing_empty = candidate.hard_break && candidate.raw_end == text.len();
            line_start = candidate.raw_end;
            builder.push_candidate(candidate)?;
        }
        if trailing_empty {
            builder.push_line(text.len(), text.len(), false, None)?;
        }
        builder.finish()
    }
}

struct LayoutBuilder<'a> {
    fonts: &'a FontCollection,
    text: &'a str,
    bidi: BidiInfo<'a>,
    font_size_bits: i32,
    options: TextLayoutOptions,
    lines: Vec<ShapedLine>,
    top_bits: i32,
    width_bits: i32,
    shaping_attempts: usize,
    total_runs: usize,
    total_glyphs: usize,
}

impl LayoutBuilder<'_> {
    fn shape_candidate(
        &mut self,
        start: usize,
        content_end: usize,
        raw_end: usize,
        hard_break: bool,
    ) -> Result<LineCandidate, TextError> {
        let paragraph = if start == content_end {
            None
        } else {
            self.shaping_attempts = self
                .shaping_attempts
                .checked_add(1)
                .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
            if self.shaping_attempts > self.options.max_shaping_attempts {
                return Err(TextError::new(TextErrorCode::ResourceLimit));
            }
            let bidi_paragraph = self
                .bidi
                .paragraphs
                .iter()
                .find(|paragraph| {
                    start >= paragraph.range.start && content_end <= paragraph.range.end
                })
                .ok_or(TextError::new(TextErrorCode::InvalidLayout))?;
            Some(self.fonts.shape_bidi_line(
                self.text,
                self.font_size_bits,
                &self.bidi,
                bidi_paragraph,
                start..content_end,
            )?)
        };
        Ok(LineCandidate {
            paragraph,
            source_start: start,
            source_end: content_end,
            raw_end,
            hard_break,
        })
    }

    fn force_grapheme_break(
        &mut self,
        start: usize,
        limit: usize,
    ) -> Result<LineCandidate, TextError> {
        let mut best = None;
        for (relative_start, grapheme) in self.text[start..limit].grapheme_indices(true) {
            let end = start + relative_start + grapheme.len();
            let candidate = self.shape_candidate(start, end, end, false)?;
            let fits = candidate.width_bits() <= self.options.max_width_bits;
            if fits || best.is_none() {
                best = Some(candidate);
            }
            if !fits {
                break;
            }
        }
        best.ok_or(TextError::new(TextErrorCode::InvalidLayout))
    }

    fn push_candidate(&mut self, candidate: LineCandidate) -> Result<(), TextError> {
        self.push_line(
            candidate.source_start,
            candidate.source_end,
            candidate.hard_break,
            candidate.paragraph,
        )
    }

    fn push_line(
        &mut self,
        source_start: usize,
        source_end: usize,
        hard_break: bool,
        paragraph: Option<ShapedParagraph>,
    ) -> Result<(), TextError> {
        if self.lines.len() == self.options.max_lines {
            return Err(TextError::new(TextErrorCode::ResourceLimit));
        }
        let metrics = paragraph
            .as_ref()
            .map(ShapedParagraph::metrics)
            .map(Ok)
            .unwrap_or_else(|| self.fonts.default_metrics(self.font_size_bits))?;
        let line_height = metrics.line_height_bits()?;
        let baseline_y_bits = self
            .top_bits
            .checked_add(metrics.ascent_bits())
            .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
        self.top_bits = self
            .top_bits
            .checked_add(line_height)
            .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
        if let Some(paragraph) = &paragraph {
            self.width_bits = self.width_bits.max(paragraph.advance_x_bits());
            self.total_runs = self
                .total_runs
                .checked_add(paragraph.runs().len())
                .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
            self.total_glyphs = self
                .total_glyphs
                .checked_add(
                    paragraph
                        .runs()
                        .iter()
                        .map(|run| run.glyph_run().glyphs().len())
                        .sum::<usize>(),
                )
                .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
            let limits = self.fonts.limits();
            if self.total_runs > limits.max_runs() || self.total_glyphs > limits.max_glyphs() {
                return Err(TextError::new(TextErrorCode::ResourceLimit));
            }
        }
        self.lines
            .try_reserve(1)
            .map_err(|_| TextError::new(TextErrorCode::AllocationFailed))?;
        self.lines.push(ShapedLine {
            paragraph,
            source_start: u32::try_from(source_start)
                .map_err(|_| TextError::new(TextErrorCode::ResourceLimit))?,
            source_end: u32::try_from(source_end)
                .map_err(|_| TextError::new(TextErrorCode::ResourceLimit))?,
            baseline_y_bits,
            hard_break,
            metrics,
        });
        Ok(())
    }

    fn finish(self) -> Result<TextLayout, TextError> {
        Ok(TextLayout {
            lines: self.lines,
            width_bits: self.width_bits,
            height_bits: self.top_bits,
        })
    }
}

struct LineCandidate {
    paragraph: Option<ShapedParagraph>,
    source_start: usize,
    source_end: usize,
    raw_end: usize,
    hard_break: bool,
}

impl LineCandidate {
    fn width_bits(&self) -> i32 {
        self.paragraph
            .as_ref()
            .map_or(0, ShapedParagraph::advance_x_bits)
    }
}

fn strip_mandatory_ending(text: &str, start: usize, end: usize) -> usize {
    let line = &text[start..end];
    if line.ends_with("\r\n") {
        end - 2
    } else if line.ends_with(['\n', '\r', '\u{0085}', '\u{2028}', '\u{2029}']) {
        end - line.chars().next_back().map_or(0, char::len_utf8)
    } else {
        end
    }
}
