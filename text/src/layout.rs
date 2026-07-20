use unicode_bidi::{BidiInfo, LTR_LEVEL, RTL_LEVEL};
use unicode_linebreak::{BreakOpportunity, linebreaks};
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    FontCollection, FontMetrics, ShapedParagraph, TextDecorationMetrics, TextDirection, TextError,
    TextErrorCode,
};

/// Decoration lines requested for every non-empty line in one text layout.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum TextDecoration {
    /// Draw no decoration.
    #[default]
    None,
    /// Draw an underline.
    Underline,
    /// Draw a strike-through.
    StrikeThrough,
    /// Draw both an underline and a strike-through.
    UnderlineAndStrikeThrough,
}

impl TextDecoration {
    const fn includes_underline(self) -> bool {
        matches!(self, Self::Underline | Self::UnderlineAndStrikeThrough)
    }

    const fn includes_strike_through(self) -> bool {
        matches!(self, Self::StrikeThrough | Self::UnderlineAndStrikeThrough)
    }
}

/// Horizontal placement policy inside a layout's configured line width.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TextAlignment {
    /// Align to the paragraph's logical start edge.
    Start,
    /// Align to the paragraph's logical end edge.
    End,
    /// Align to the physical left edge.
    Left,
    /// Center each line.
    Center,
    /// Align to the physical right edge.
    Right,
    /// Expand interior breakable Unicode spaces to fill eligible lines.
    ///
    /// By default, paragraph-final lines keep start alignment. Use
    /// [`TextLayoutOptions::with_justify_last_line`] to include them.
    Justify,
}

/// Rendering behavior for one language-specific word-internal break.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TextWordBreakKind {
    /// Wrap without inserting a visible glyph.
    Soft,
    /// Wrap and append a synthetic visible hyphen.
    Hyphenated,
}

/// One UTF-8 byte offset supplied by a language-specific break provider.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TextWordBreak {
    offset: usize,
    kind: TextWordBreakKind,
}

impl TextWordBreak {
    /// Creates one word-internal break candidate.
    pub const fn new(offset: usize, kind: TextWordBreakKind) -> Self {
        Self { offset, kind }
    }

    /// Returns the byte offset relative to the supplied word.
    pub const fn offset(self) -> usize {
        self.offset
    }

    /// Returns whether taking the break inserts a visible hyphen.
    pub const fn kind(self) -> TextWordBreakKind {
        self.kind
    }
}

/// Supplies language-specific UTF-8 break opportunities for one word.
pub trait TextBreakProvider {
    /// Returns break candidates strictly inside `word` for one BCP 47-style language.
    ///
    /// Candidates may be unordered, but each offset must be an
    /// extended-grapheme boundary. The layout engine validates, sorts,
    /// deduplicates, and resource-bounds them.
    fn opportunities(&self, word: &str, language: &str) -> Result<Vec<TextWordBreak>, TextError>;
}

/// Width and work ceilings for greedy Unicode line layout.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TextLayoutOptions {
    max_width_bits: i32,
    max_lines: usize,
    max_shaping_attempts: usize,
    base_direction: Option<TextDirection>,
    alignment: TextAlignment,
    justify_last_line: bool,
    decoration: TextDecoration,
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
            alignment: TextAlignment::Start,
            justify_last_line: false,
            decoration: TextDecoration::None,
        })
    }

    /// Forces the same bidi base direction for every produced line.
    pub const fn with_base_direction(mut self, direction: TextDirection) -> Self {
        self.base_direction = Some(direction);
        self
    }

    /// Selects horizontal placement inside the configured line width.
    pub const fn with_alignment(mut self, alignment: TextAlignment) -> Self {
        self.alignment = alignment;
        self
    }

    /// Controls whether justification also expands paragraph-final lines.
    ///
    /// This option has no effect for alignments other than
    /// [`TextAlignment::Justify`].
    pub const fn with_justify_last_line(mut self, justify: bool) -> Self {
        self.justify_last_line = justify;
        self
    }

    /// Selects decoration lines for every non-empty laid-out line.
    ///
    /// Decoration metrics come from the collection's first face so one
    /// continuous line remains stable across fallback runs.
    pub const fn with_decoration(mut self, decoration: TextDecoration) -> Self {
        self.decoration = decoration;
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

    /// Returns the horizontal alignment policy.
    pub const fn alignment(self) -> TextAlignment {
        self.alignment
    }

    /// Returns whether paragraph-final lines are eligible for justification.
    pub const fn justify_last_line(self) -> bool {
        self.justify_last_line
    }

    /// Returns the requested line-decoration policy.
    pub const fn decoration(self) -> TextDecoration {
        self.decoration
    }
}

/// One positioned line in a laid-out text block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShapedLine {
    paragraph: Option<ShapedParagraph>,
    source_start: u32,
    source_end: u32,
    offset_x_bits: i32,
    advance_x_bits: i32,
    baseline_y_bits: i32,
    hard_break: bool,
    hyphenated: bool,
    justified: bool,
    metrics: FontMetrics,
    underline_metrics: Option<TextDecorationMetrics>,
    strike_through_metrics: Option<TextDecorationMetrics>,
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

    /// Returns the Q16.16 horizontal offset from the text-block origin.
    pub const fn offset_x_bits(&self) -> i32 {
        self.offset_x_bits
    }

    /// Returns the line's final horizontal advance after justification.
    pub const fn advance_x_bits(&self) -> i32 {
        self.advance_x_bits
    }

    /// Returns the baseline position relative to the text-block top.
    pub const fn baseline_y_bits(&self) -> i32 {
        self.baseline_y_bits
    }

    /// Returns whether an explicit mandatory separator ended this line.
    pub const fn hard_break(&self) -> bool {
        self.hard_break
    }

    /// Returns whether a dictionary break appended a synthetic visible hyphen.
    pub const fn hyphenated(&self) -> bool {
        self.hyphenated
    }

    /// Returns whether the line received expanded inter-word spacing.
    pub const fn justified(&self) -> bool {
        self.justified
    }

    /// Returns this line's baseline metrics.
    pub const fn metrics(&self) -> FontMetrics {
        self.metrics
    }

    /// Returns resolved underline metrics when underline drawing was requested.
    pub const fn underline_metrics(&self) -> Option<TextDecorationMetrics> {
        self.underline_metrics
    }

    /// Returns resolved strike-through metrics when strike-through drawing was requested.
    pub const fn strike_through_metrics(&self) -> Option<TextDecorationMetrics> {
        self.strike_through_metrics
    }
}

/// Greedily wrapped, vertically positioned text lines.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextLayout {
    lines: Vec<ShapedLine>,
    width_bits: i32,
    height_bits: i32,
    container_width_bits: i32,
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

    /// Returns the configured Q16.16 line-container width.
    pub const fn container_width_bits(&self) -> i32 {
        self.container_width_bits
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
        self.layout_text_impl(text, font_size_bits, options, None)
    }

    /// Shapes and wraps UTF-8 with language-specific word-internal breaks.
    ///
    /// Hyphenated breaks append U+2010 HYPHEN, falling back to ASCII `-` when
    /// needed. Soft breaks insert no glyph. A synthetic hyphen maps to the
    /// source break offset without consuming source bytes.
    pub fn layout_text_with_break_provider(
        &self,
        text: &str,
        font_size_bits: i32,
        options: TextLayoutOptions,
        language: &str,
        provider: &impl TextBreakProvider,
    ) -> Result<TextLayout, TextError> {
        if !valid_language_tag(language) {
            return Err(TextError::new(TextErrorCode::InvalidLanguage));
        }
        self.layout_text_impl(text, font_size_bits, options, Some((language, provider)))
    }

    fn layout_text_impl(
        &self,
        text: &str,
        font_size_bits: i32,
        options: TextLayoutOptions,
        language_breaks: Option<(&str, &dyn TextBreakProvider)>,
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

        let breaks = collect_layout_breaks(text, options, language_breaks)?;

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
            shaping_attempts: 0,
            total_runs: 0,
            total_glyphs: 0,
        };
        if text.is_empty() {
            builder.push_line(0, 0, false, false, None)?;
            return builder.finish();
        }

        let mut line_start = 0_usize;
        let mut trailing_empty = false;
        while line_start < text.len() {
            let mut last_fit: Option<LineCandidate> = None;
            let mut chosen: Option<LineCandidate> = None;
            let first_break = breaks.partition_point(|point| point.index <= line_start);
            for &point in &breaks[first_break..] {
                let raw_end = point.index;
                let opportunity = point.opportunity;
                let hard_break = opportunity == BreakOpportunity::Mandatory
                    && strip_mandatory_ending(text, line_start, raw_end) < raw_end;
                let content_end = if hard_break {
                    strip_mandatory_ending(text, line_start, raw_end)
                } else {
                    raw_end
                };
                let candidate = builder.shape_candidate(
                    line_start,
                    content_end,
                    raw_end,
                    hard_break,
                    point.hyphenated,
                )?;
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
            builder.push_line(text.len(), text.len(), false, false, None)?;
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
        hyphenated: bool,
    ) -> Result<LineCandidate, TextError> {
        let paragraph = if start == content_end {
            if hyphenated {
                return Err(TextError::new(TextErrorCode::InvalidWordBreak));
            }
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
            let mut paragraph = self.fonts.shape_bidi_line(
                self.text,
                self.font_size_bits,
                &self.bidi,
                bidi_paragraph,
                start..content_end,
            )?;
            if hyphenated {
                self.fonts.append_discretionary_hyphen(
                    &mut paragraph,
                    self.font_size_bits,
                    u32::try_from(content_end)
                        .map_err(|_| TextError::new(TextErrorCode::ResourceLimit))?,
                )?;
            }
            Some(paragraph)
        };
        Ok(LineCandidate {
            paragraph,
            source_start: start,
            source_end: content_end,
            raw_end,
            hard_break,
            hyphenated,
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
            let candidate = self.shape_candidate(start, end, end, false, false)?;
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
            candidate.hyphenated,
            candidate.paragraph,
        )
    }

    fn push_line(
        &mut self,
        source_start: usize,
        source_end: usize,
        hard_break: bool,
        hyphenated: bool,
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
        let decoration_face = self
            .fonts
            .faces()
            .first()
            .ok_or(TextError::new(TextErrorCode::EmptyFontCollection))?;
        let underline_metrics =
            if paragraph.is_some() && self.options.decoration.includes_underline() {
                Some(
                    decoration_face
                        .underline_metrics(self.font_size_bits)?
                        .ok_or(TextError::new(TextErrorCode::MissingDecorationMetrics))?,
                )
            } else {
                None
            };
        let strike_through_metrics =
            if paragraph.is_some() && self.options.decoration.includes_strike_through() {
                Some(
                    decoration_face
                        .strike_through_metrics(self.font_size_bits)?
                        .ok_or(TextError::new(TextErrorCode::MissingDecorationMetrics))?,
                )
            } else {
                None
            };
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
            offset_x_bits: 0,
            advance_x_bits: 0,
            baseline_y_bits,
            hard_break,
            hyphenated,
            justified: false,
            metrics,
            underline_metrics,
            strike_through_metrics,
        });
        Ok(())
    }

    fn finish(mut self) -> Result<TextLayout, TextError> {
        let mut width_bits = 0;
        let line_count = self.lines.len();
        for (index, line) in self.lines.iter_mut().enumerate() {
            let paragraph_final = line.hard_break || index + 1 == line_count;
            let direction = line.paragraph.as_ref().map_or(
                self.options
                    .base_direction
                    .unwrap_or(TextDirection::LeftToRight),
                |text| text.base_direction(),
            );
            if self.options.alignment == TextAlignment::Justify
                && (!paragraph_final || self.options.justify_last_line)
                && let Some(paragraph) = &mut line.paragraph
            {
                line.justified = paragraph.justify_expandable_spaces(
                    self.text,
                    line.source_start as usize,
                    line.source_end as usize,
                    self.options.max_width_bits,
                )?;
            }
            line.advance_x_bits = line
                .paragraph
                .as_ref()
                .map_or(0, ShapedParagraph::advance_x_bits);
            let free_bits = self
                .options
                .max_width_bits
                .saturating_sub(line.advance_x_bits)
                .max(0);
            let effective_alignment = if self.options.alignment == TextAlignment::Justify {
                TextAlignment::Start
            } else {
                self.options.alignment
            };
            line.offset_x_bits = alignment_offset(effective_alignment, direction, free_bits);
            width_bits = width_bits.max(line.advance_x_bits);
        }
        Ok(TextLayout {
            lines: self.lines,
            width_bits,
            height_bits: self.top_bits,
            container_width_bits: self.options.max_width_bits,
        })
    }
}

struct LineCandidate {
    paragraph: Option<ShapedParagraph>,
    source_start: usize,
    source_end: usize,
    raw_end: usize,
    hard_break: bool,
    hyphenated: bool,
}

impl LineCandidate {
    fn width_bits(&self) -> i32 {
        self.paragraph
            .as_ref()
            .map_or(0, ShapedParagraph::advance_x_bits)
    }
}

#[derive(Clone, Copy)]
struct LayoutBreak {
    index: usize,
    opportunity: BreakOpportunity,
    hyphenated: bool,
}

fn collect_layout_breaks(
    text: &str,
    options: TextLayoutOptions,
    language_breaks: Option<(&str, &dyn TextBreakProvider)>,
) -> Result<Vec<LayoutBreak>, TextError> {
    let mut breaks = Vec::new();
    breaks
        .try_reserve(text.len().saturating_add(1))
        .map_err(|_| TextError::new(TextErrorCode::AllocationFailed))?;
    breaks.extend(linebreaks(text).map(|(index, opportunity)| LayoutBreak {
        index,
        opportunity,
        hyphenated: false,
    }));

    if let Some((language, provider)) = language_breaks {
        let mut total_opportunities = 0_usize;
        for (word_start, word) in text.split_word_bound_indices() {
            if !word.chars().any(char::is_alphanumeric) {
                continue;
            }
            let mut opportunities = provider.opportunities(word, language)?;
            total_opportunities = total_opportunities
                .checked_add(opportunities.len())
                .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
            if total_opportunities > options.max_shaping_attempts {
                return Err(TextError::new(TextErrorCode::ResourceLimit));
            }
            opportunities.sort_unstable_by_key(|opportunity| {
                (
                    opportunity.offset,
                    opportunity.kind == TextWordBreakKind::Hyphenated,
                )
            });
            opportunities.dedup_by_key(|opportunity| opportunity.offset);
            breaks
                .try_reserve(opportunities.len())
                .map_err(|_| TextError::new(TextErrorCode::AllocationFailed))?;
            for opportunity in opportunities {
                let relative = opportunity.offset;
                if relative == 0
                    || relative >= word.len()
                    || !word.is_char_boundary(relative)
                    || !is_grapheme_boundary(word, relative)
                {
                    return Err(TextError::new(TextErrorCode::InvalidWordBreak));
                }
                let index = word_start
                    .checked_add(relative)
                    .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
                breaks.push(LayoutBreak {
                    index,
                    opportunity: BreakOpportunity::Allowed,
                    hyphenated: opportunity.kind == TextWordBreakKind::Hyphenated,
                });
            }
        }
    }

    breaks.sort_unstable_by_key(|point| (point.index, point.hyphenated));
    let mut merged: Vec<LayoutBreak> = Vec::new();
    merged
        .try_reserve_exact(breaks.len())
        .map_err(|_| TextError::new(TextErrorCode::AllocationFailed))?;
    for point in breaks {
        if merged
            .last()
            .is_some_and(|existing| existing.index == point.index)
        {
            continue;
        }
        merged.push(point);
    }
    Ok(merged)
}

fn is_grapheme_boundary(text: &str, offset: usize) -> bool {
    text.grapheme_indices(true)
        .any(|(index, _)| index == offset)
}

fn valid_language_tag(language: &str) -> bool {
    !language.is_empty()
        && language.len() <= 255
        && language
            .split('-')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_alphanumeric()))
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

const fn alignment_offset(
    alignment: TextAlignment,
    direction: TextDirection,
    free_bits: i32,
) -> i32 {
    match alignment {
        TextAlignment::Start => match direction {
            TextDirection::LeftToRight => 0,
            TextDirection::RightToLeft => free_bits,
        },
        TextAlignment::End => match direction {
            TextDirection::LeftToRight => free_bits,
            TextDirection::RightToLeft => 0,
        },
        TextAlignment::Left | TextAlignment::Justify => 0,
        TextAlignment::Center => free_bits / 2,
        TextAlignment::Right => free_bits,
    }
}
