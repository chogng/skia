use unicode_bidi::{BidiInfo, LTR_LEVEL, RTL_LEVEL};
use unicode_linebreak::{BreakOpportunity, linebreaks};
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    FontCollection, FontFace, FontMetrics, ShapedParagraph, TextDecorationMetrics, TextDirection,
    TextError, TextErrorCode, TextStyleSpan,
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

/// Behavior when laid-out text would exceed the configured line limit.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum TextOverflow {
    /// Return [`TextErrorCode::ResourceLimit`] without a partial layout.
    #[default]
    Error,
    /// Keep the first `max_lines` and omit all remaining source text.
    Clip,
    /// Replace the visible suffix of the final line with a synthetic ellipsis.
    Ellipsis,
}

/// Selects one visual caret when a source boundary has two layout positions.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TextAffinity {
    /// Prefer the visual position after the preceding source cluster.
    Upstream,
    /// Prefer the visual position before the following source cluster.
    Downstream,
}

/// One UTF-8 source boundary and its visual affinity.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TextPosition {
    source_offset: u32,
    affinity: TextAffinity,
}

impl TextPosition {
    /// Creates one source position without validating it against a layout.
    pub const fn new(source_offset: u32, affinity: TextAffinity) -> Self {
        Self {
            source_offset,
            affinity,
        }
    }

    /// Returns the UTF-8 byte boundary.
    pub const fn source_offset(self) -> u32 {
        self.source_offset
    }

    /// Returns the visual affinity at an ambiguous boundary.
    pub const fn affinity(self) -> TextAffinity {
        self.affinity
    }
}

/// A source position selected by a layout-space point.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TextHitResult {
    position: TextPosition,
    line_index: usize,
}

impl TextHitResult {
    /// Returns the nearest cluster-boundary source position.
    pub const fn position(self) -> TextPosition {
        self.position
    }

    /// Returns the selected zero-based line index.
    pub const fn line_index(self) -> usize {
        self.line_index
    }
}

/// One resolved vertical caret in layout-local Q16.16 coordinates.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TextCaret {
    position: TextPosition,
    line_index: usize,
    x_bits: i32,
    top_bits: i32,
    bottom_bits: i32,
}

impl TextCaret {
    /// Returns the source position represented by this caret.
    pub const fn position(self) -> TextPosition {
        self.position
    }

    /// Returns the zero-based line index.
    pub const fn line_index(self) -> usize {
        self.line_index
    }

    /// Returns the caret's horizontal Q16.16 coordinate.
    pub const fn x_bits(self) -> i32 {
        self.x_bits
    }

    /// Returns the caret's inclusive top Q16.16 coordinate.
    pub const fn top_bits(self) -> i32 {
        self.top_bits
    }

    /// Returns the caret's exclusive bottom Q16.16 coordinate.
    pub const fn bottom_bits(self) -> i32 {
        self.bottom_bits
    }
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
    overflow: TextOverflow,
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
            overflow: TextOverflow::Error,
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
    /// Uniform layouts use the collection's first face. Styled layouts use
    /// the logical line-start span's preferred face and size so one continuous
    /// line remains stable across fallback runs within that line.
    pub const fn with_decoration(mut self, decoration: TextDecoration) -> Self {
        self.decoration = decoration;
        self
    }

    /// Selects behavior when text would exceed `max_lines`.
    pub const fn with_overflow(mut self, overflow: TextOverflow) -> Self {
        self.overflow = overflow;
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

    /// Returns the configured line-limit overflow behavior.
    pub const fn overflow(self) -> TextOverflow {
        self.overflow
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
    ellipsized: bool,
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

    /// Returns whether a synthetic ellipsis terminates this line.
    pub const fn ellipsized(&self) -> bool {
        self.ellipsized
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
    truncated: bool,
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

    /// Returns whether source text was omitted because of the line limit.
    pub const fn truncated(&self) -> bool {
        self.truncated
    }

    /// Snaps one layout-local Q16.16 point to the nearest shaping-cluster edge.
    ///
    /// Vertical coordinates outside the block clamp to the nearest line.
    /// Horizontal coordinates outside a line clamp to its nearest caret stop.
    /// Ligatures and other multi-codepoint clusters remain atomic.
    pub fn hit_test_point(&self, x_bits: i32, y_bits: i32) -> Result<TextHitResult, TextError> {
        let line_index = self.nearest_line_index(y_bits)?;
        let stops = line_caret_stops(&self.lines[line_index])?;
        let stop = stops
            .iter()
            .min_by_key(|stop| i64::from(stop.x_bits).abs_diff(i64::from(x_bits)))
            .ok_or(TextError::new(TextErrorCode::InvalidLayout))?;
        Ok(TextHitResult {
            position: stop.position,
            line_index,
        })
    }

    /// Resolves a source cluster boundary to a layout-local vertical caret.
    ///
    /// At soft wraps, `Upstream` selects the preceding line end and
    /// `Downstream` selects the following line start. Returns `Ok(None)` when
    /// the offset is not a shaped cluster boundary in this layout.
    pub fn caret_for_position(
        &self,
        position: TextPosition,
    ) -> Result<Option<TextCaret>, TextError> {
        let mut fallback = None;
        for (line_index, line) in self.lines.iter().enumerate() {
            let stops = line_caret_stops(line)?;
            for stop in stops {
                if stop.position.source_offset != position.source_offset {
                    continue;
                }
                let caret = line_caret(line, line_index, position, stop.x_bits)?;
                if stop.position.affinity == position.affinity {
                    return Ok(Some(caret));
                }
                fallback.get_or_insert(caret);
            }
        }
        Ok(fallback)
    }

    fn nearest_line_index(&self, y_bits: i32) -> Result<usize, TextError> {
        let mut nearest = None;
        for (index, line) in self.lines.iter().enumerate() {
            let (top_bits, bottom_bits) = line_box_bounds(line)?;
            let distance = if y_bits < top_bits {
                i64::from(top_bits) - i64::from(y_bits)
            } else if y_bits >= bottom_bits {
                i64::from(y_bits) - i64::from(bottom_bits) + 1
            } else {
                0
            };
            if nearest.is_none_or(|(_, nearest_distance)| distance < nearest_distance) {
                nearest = Some((index, distance));
            }
        }
        nearest
            .map(|(index, _)| index)
            .ok_or(TextError::new(TextErrorCode::InvalidLayout))
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
        self.layout_text_impl(text, LayoutStyle::Uniform(font_size_bits), options, None)
    }

    /// Shapes and greedily wraps UTF-8 with grapheme-safe font and size spans.
    ///
    /// Spans follow the same coverage and boundary rules as
    /// [`FontCollection::shape_styled_paragraph`]. Each candidate line is
    /// reshaped independently so contextual shaping remains valid at wraps.
    pub fn layout_styled_text(
        &self,
        text: &str,
        spans: &[TextStyleSpan],
        options: TextLayoutOptions,
    ) -> Result<TextLayout, TextError> {
        self.layout_text_impl(text, LayoutStyle::Spans(spans), options, None)
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
        self.layout_text_impl(
            text,
            LayoutStyle::Uniform(font_size_bits),
            options,
            Some((language, provider)),
        )
    }

    /// Shapes styled UTF-8 with language-specific word-internal breaks.
    ///
    /// A synthetic hyphen inherits the actual run at the logical break's left
    /// edge, including its font instance, size, and bidi direction.
    pub fn layout_styled_text_with_break_provider(
        &self,
        text: &str,
        spans: &[TextStyleSpan],
        options: TextLayoutOptions,
        language: &str,
        provider: &impl TextBreakProvider,
    ) -> Result<TextLayout, TextError> {
        if !valid_language_tag(language) {
            return Err(TextError::new(TextErrorCode::InvalidLanguage));
        }
        self.layout_text_impl(
            text,
            LayoutStyle::Spans(spans),
            options,
            Some((language, provider)),
        )
    }

    fn layout_text_impl<'a>(
        &'a self,
        text: &'a str,
        style: LayoutStyle<'a>,
        options: TextLayoutOptions,
        language_breaks: Option<(&str, &dyn TextBreakProvider)>,
    ) -> Result<TextLayout, TextError> {
        if self.faces().is_empty() {
            return Err(TextError::new(TextErrorCode::EmptyFontCollection));
        }
        if text.len() > self.limits().max_text_bytes() || text.len() > u32::MAX as usize {
            return Err(TextError::new(TextErrorCode::ResourceLimit));
        }
        match style {
            LayoutStyle::Uniform(font_size_bits) if font_size_bits <= 0 => {
                return Err(TextError::new(TextErrorCode::InvalidFontSize));
            }
            LayoutStyle::Spans(_) if text.is_empty() => {
                return Err(TextError::new(TextErrorCode::EmptyText));
            }
            LayoutStyle::Spans(spans) => self.validate_style_spans(text, spans)?,
            LayoutStyle::Uniform(_) => {}
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
            style,
            options,
            lines: Vec::new(),
            top_bits: 0,
            shaping_attempts: 0,
            total_runs: 0,
            total_glyphs: 0,
            truncated: false,
        };
        if text.is_empty() {
            builder.push_line(0, 0, false, false, false, None)?;
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
            let reaches_line_limit = builder.lines.len() + 1 == options.max_lines;
            let has_hidden_line =
                line_start < text.len() || (trailing_empty && line_start == text.len());
            if reaches_line_limit && has_hidden_line {
                match options.overflow {
                    TextOverflow::Error => builder.push_candidate(candidate)?,
                    TextOverflow::Clip => {
                        builder.truncated = true;
                        builder.push_candidate(candidate)?;
                        break;
                    }
                    TextOverflow::Ellipsis => {
                        builder.truncated = true;
                        let candidate = builder.ellipsize_candidate(candidate)?;
                        builder.push_candidate(candidate)?;
                        break;
                    }
                }
            } else {
                builder.push_candidate(candidate)?;
            }
        }
        if trailing_empty && !builder.truncated {
            builder.push_line(text.len(), text.len(), false, false, false, None)?;
        }
        builder.finish()
    }
}

struct LayoutBuilder<'a> {
    fonts: &'a FontCollection,
    text: &'a str,
    bidi: BidiInfo<'a>,
    style: LayoutStyle<'a>,
    options: TextLayoutOptions,
    lines: Vec<ShapedLine>,
    top_bits: i32,
    shaping_attempts: usize,
    total_runs: usize,
    total_glyphs: usize,
    truncated: bool,
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
            let mut paragraph = match self.style {
                LayoutStyle::Uniform(font_size_bits) => self.fonts.shape_bidi_line(
                    self.text,
                    font_size_bits,
                    &self.bidi,
                    bidi_paragraph,
                    start..content_end,
                )?,
                LayoutStyle::Spans(spans) => self.fonts.shape_styled_bidi_line(
                    self.text,
                    spans,
                    &self.bidi,
                    bidi_paragraph,
                    start..content_end,
                )?,
            };
            if hyphenated {
                self.fonts.append_discretionary_hyphen(
                    &mut paragraph,
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
            ellipsized: false,
        })
    }

    fn ellipsize_candidate(
        &mut self,
        candidate: LineCandidate,
    ) -> Result<LineCandidate, TextError> {
        let mut boundaries = Vec::new();
        let source_length = candidate
            .source_end
            .checked_sub(candidate.source_start)
            .ok_or(TextError::new(TextErrorCode::InvalidLayout))?;
        boundaries
            .try_reserve(
                source_length
                    .checked_add(1)
                    .ok_or(TextError::new(TextErrorCode::NumericOverflow))?,
            )
            .map_err(|_| TextError::new(TextErrorCode::AllocationFailed))?;
        boundaries.push(candidate.source_start);
        boundaries.extend(
            self.text[candidate.source_start..candidate.source_end]
                .grapheme_indices(true)
                .map(|(relative, grapheme)| candidate.source_start + relative + grapheme.len()),
        );

        for source_end in boundaries.into_iter().rev() {
            let paragraph = if source_end == candidate.source_start {
                let (face, font_size_bits) = self.line_style(candidate.source_start)?;
                self.fonts.shape_ellipsis_marker(
                    font_size_bits,
                    face.id(),
                    self.direction_at(candidate.source_start),
                    u32::try_from(source_end)
                        .map_err(|_| TextError::new(TextErrorCode::ResourceLimit))?,
                )?
            } else {
                let mut paragraph = self
                    .shape_candidate(candidate.source_start, source_end, source_end, false, false)?
                    .paragraph
                    .ok_or(TextError::new(TextErrorCode::InvalidLayout))?;
                self.fonts.append_ellipsis(
                    &mut paragraph,
                    u32::try_from(source_end)
                        .map_err(|_| TextError::new(TextErrorCode::ResourceLimit))?,
                )?;
                paragraph
            };
            if paragraph.advance_x_bits() <= self.options.max_width_bits
                || source_end == candidate.source_start
            {
                return Ok(LineCandidate {
                    paragraph: Some(paragraph),
                    source_start: candidate.source_start,
                    source_end,
                    raw_end: candidate.raw_end,
                    hard_break: false,
                    hyphenated: false,
                    ellipsized: true,
                });
            }
        }
        Err(TextError::new(TextErrorCode::InvalidLayout))
    }

    fn direction_at(&self, source_offset: usize) -> TextDirection {
        self.bidi
            .paragraphs
            .iter()
            .find(|paragraph| {
                source_offset >= paragraph.range.start && source_offset <= paragraph.range.end
            })
            .map_or(
                self.options
                    .base_direction
                    .unwrap_or(TextDirection::LeftToRight),
                |paragraph| {
                    if paragraph.level.is_rtl() {
                        TextDirection::RightToLeft
                    } else {
                        TextDirection::LeftToRight
                    }
                },
            )
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
            candidate.ellipsized,
            candidate.paragraph,
        )
    }

    fn push_line(
        &mut self,
        source_start: usize,
        source_end: usize,
        hard_break: bool,
        hyphenated: bool,
        ellipsized: bool,
        paragraph: Option<ShapedParagraph>,
    ) -> Result<(), TextError> {
        if self.lines.len() == self.options.max_lines {
            return Err(TextError::new(TextErrorCode::ResourceLimit));
        }
        let (line_style_face, line_style_size_bits) = self.line_style(source_start)?;
        let metrics = paragraph.as_ref().map_or_else(
            || line_style_face.metrics(line_style_size_bits),
            |paragraph| Ok(paragraph.metrics()),
        )?;
        let underline_metrics =
            if paragraph.is_some() && self.options.decoration.includes_underline() {
                Some(
                    line_style_face
                        .underline_metrics(line_style_size_bits)?
                        .ok_or(TextError::new(TextErrorCode::MissingDecorationMetrics))?,
                )
            } else {
                None
            };
        let strike_through_metrics =
            if paragraph.is_some() && self.options.decoration.includes_strike_through() {
                Some(
                    line_style_face
                        .strike_through_metrics(line_style_size_bits)?
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
            ellipsized,
            justified: false,
            metrics,
            underline_metrics,
            strike_through_metrics,
        });
        Ok(())
    }

    fn line_style(&self, source_start: usize) -> Result<(&FontFace, i32), TextError> {
        match self.style {
            LayoutStyle::Uniform(font_size_bits) => Ok((
                self.fonts
                    .faces()
                    .first()
                    .ok_or(TextError::new(TextErrorCode::EmptyFontCollection))?,
                font_size_bits,
            )),
            LayoutStyle::Spans(spans) => {
                let span_index = if source_start == self.text.len() {
                    spans.len().checked_sub(1)
                } else {
                    Some(spans.partition_point(|span| span.source_end() as usize <= source_start))
                }
                .ok_or(TextError::new(TextErrorCode::InvalidTextStyleSpan))?;
                let span = spans
                    .get(span_index)
                    .filter(|span| {
                        span.source_start() as usize <= source_start
                            && source_start <= span.source_end() as usize
                    })
                    .ok_or(TextError::new(TextErrorCode::InvalidTextStyleSpan))?;
                Ok((
                    self.fonts
                        .face(span.font())
                        .ok_or(TextError::new(TextErrorCode::InvalidTextStyleSpan))?,
                    span.font_size_bits(),
                ))
            }
        }
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
                && !line.ellipsized
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
            truncated: self.truncated,
        })
    }
}

#[derive(Clone, Copy)]
enum LayoutStyle<'a> {
    Uniform(i32),
    Spans(&'a [TextStyleSpan]),
}

#[derive(Clone, Copy)]
struct CaretStop {
    position: TextPosition,
    x_bits: i32,
}

fn line_caret_stops(line: &ShapedLine) -> Result<Vec<CaretStop>, TextError> {
    let Some(paragraph) = line.paragraph() else {
        return Ok(vec![
            CaretStop {
                position: TextPosition::new(line.source_start, TextAffinity::Upstream),
                x_bits: line.offset_x_bits,
            },
            CaretStop {
                position: TextPosition::new(line.source_start, TextAffinity::Downstream),
                x_bits: line.offset_x_bits,
            },
        ]);
    };

    let glyph_count = paragraph.runs().iter().try_fold(0_usize, |total, shaped| {
        if shaped.glyph_offsets_x_bits().len() != shaped.glyph_run().glyphs().len() {
            return Err(TextError::new(TextErrorCode::InvalidLayout));
        }
        total
            .checked_add(shaped.glyph_run().glyphs().len())
            .ok_or(TextError::new(TextErrorCode::NumericOverflow))
    })?;
    let boundary_capacity = glyph_count
        .checked_add(2)
        .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
    let mut boundaries = Vec::new();
    boundaries
        .try_reserve(boundary_capacity)
        .map_err(|_| TextError::new(TextErrorCode::AllocationFailed))?;
    boundaries.extend([line.source_start, line.source_end]);
    for shaped in paragraph.runs() {
        let run = shaped.glyph_run();
        for glyph in run.glyphs() {
            let cluster = glyph.cluster();
            if cluster >= line.source_start && cluster < line.source_end {
                boundaries.push(cluster);
            }
        }
    }
    boundaries.sort_unstable();
    boundaries.dedup();

    let mut stops = Vec::new();
    let stop_capacity = glyph_count
        .checked_mul(2)
        .and_then(|value| value.checked_add(2))
        .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
    stops
        .try_reserve(stop_capacity)
        .map_err(|_| TextError::new(TextErrorCode::AllocationFailed))?;
    let mut synthetic_line_end = None;
    for shaped in paragraph.runs() {
        let run = shaped.glyph_run();
        let mut first = 0_usize;
        while first < run.glyphs().len() {
            let cluster = run.glyphs()[first].cluster();
            let mut end = first + 1;
            while end < run.glyphs().len() && run.glyphs()[end].cluster() == cluster {
                end += 1;
            }
            let (left_bits, right_bits) = glyph_group_bounds(line, shaped, first, end)?;
            if shaped.source_start() == shaped.source_end() && cluster == line.source_end {
                synthetic_line_end = Some(match shaped.direction() {
                    TextDirection::LeftToRight => right_bits,
                    TextDirection::RightToLeft => left_bits,
                });
            } else if cluster >= line.source_start && cluster < line.source_end {
                let boundary_index = boundaries
                    .binary_search(&cluster)
                    .map_err(|_| TextError::new(TextErrorCode::InvalidLayout))?;
                let cluster_end = *boundaries
                    .get(boundary_index + 1)
                    .ok_or(TextError::new(TextErrorCode::InvalidLayout))?;
                let (start_bits, end_bits) = match shaped.direction() {
                    TextDirection::LeftToRight => (left_bits, right_bits),
                    TextDirection::RightToLeft => (right_bits, left_bits),
                };
                stops.push(CaretStop {
                    position: TextPosition::new(cluster, TextAffinity::Downstream),
                    x_bits: start_bits,
                });
                stops.push(CaretStop {
                    position: TextPosition::new(cluster_end, TextAffinity::Upstream),
                    x_bits: end_bits,
                });
            }
            first = end;
        }
    }

    if let Some(x_bits) = synthetic_line_end {
        stops.retain(|stop| {
            stop.position.source_offset != line.source_end
                || stop.position.affinity != TextAffinity::Upstream
        });
        stops.push(CaretStop {
            position: TextPosition::new(line.source_end, TextAffinity::Upstream),
            x_bits,
        });
    }
    if !stops.iter().any(|stop| {
        stop.position.source_offset == line.source_start
            && stop.position.affinity == TextAffinity::Downstream
    }) {
        stops.push(CaretStop {
            position: TextPosition::new(line.source_start, TextAffinity::Downstream),
            x_bits: line_edge_for_direction(line, paragraph.base_direction(), true)?,
        });
    }
    if !stops.iter().any(|stop| {
        stop.position.source_offset == line.source_end
            && stop.position.affinity == TextAffinity::Upstream
    }) {
        stops.push(CaretStop {
            position: TextPosition::new(line.source_end, TextAffinity::Upstream),
            x_bits: line_edge_for_direction(line, paragraph.base_direction(), false)?,
        });
    }
    Ok(stops)
}

fn glyph_group_bounds(
    line: &ShapedLine,
    shaped: &crate::ShapedRun,
    first: usize,
    end: usize,
) -> Result<(i32, i32), TextError> {
    let run = shaped.glyph_run();
    let offsets = shaped.glyph_offsets_x_bits();
    let mut left_bits = i32::MAX;
    let mut right_bits = i32::MIN;
    for (&glyph, &offset_bits) in run.glyphs()[first..end].iter().zip(&offsets[first..end]) {
        let glyph_start = scaled_glyph_coordinate_bits(glyph.x().bits(), run)?;
        let glyph_end_design = glyph
            .x()
            .bits()
            .checked_add(glyph.advance_x().bits())
            .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
        let glyph_end = scaled_glyph_coordinate_bits(glyph_end_design, run)?;
        let origin = line
            .offset_x_bits
            .checked_add(shaped.origin_x_bits())
            .and_then(|value| value.checked_add(offset_bits))
            .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
        let first_edge = origin
            .checked_add(glyph_start)
            .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
        let second_edge = origin
            .checked_add(glyph_end)
            .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
        left_bits = left_bits.min(first_edge.min(second_edge));
        right_bits = right_bits.max(first_edge.max(second_edge));
    }
    if left_bits == i32::MAX {
        return Err(TextError::new(TextErrorCode::InvalidLayout));
    }
    Ok((left_bits, right_bits))
}

fn scaled_glyph_coordinate_bits(design_bits: i32, run: &crate::GlyphRun) -> Result<i32, TextError> {
    let numerator = i128::from(design_bits)
        .checked_mul(i128::from(run.font_size_bits()))
        .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
    let denominator = i128::from(64_i32)
        .checked_mul(i128::from(run.units_per_em()))
        .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
    let rounded = if numerator >= 0 {
        numerator
            .checked_add(denominator / 2)
            .ok_or(TextError::new(TextErrorCode::NumericOverflow))?
            / denominator
    } else {
        -((-numerator
            .checked_add(denominator / 2)
            .ok_or(TextError::new(TextErrorCode::NumericOverflow))?)
            / denominator)
    };
    i32::try_from(rounded).map_err(|_| TextError::new(TextErrorCode::NumericOverflow))
}

fn line_edge_for_direction(
    line: &ShapedLine,
    direction: TextDirection,
    logical_start: bool,
) -> Result<i32, TextError> {
    let right_bits = line
        .offset_x_bits
        .checked_add(line.advance_x_bits)
        .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
    Ok(match (direction, logical_start) {
        (TextDirection::LeftToRight, true) | (TextDirection::RightToLeft, false) => {
            line.offset_x_bits
        }
        (TextDirection::LeftToRight, false) | (TextDirection::RightToLeft, true) => right_bits,
    })
}

fn line_box_bounds(line: &ShapedLine) -> Result<(i32, i32), TextError> {
    let top_bits = line
        .baseline_y_bits
        .checked_sub(line.metrics.ascent_bits())
        .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
    let bottom_bits = top_bits
        .checked_add(line.metrics.line_height_bits()?)
        .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
    Ok((top_bits, bottom_bits))
}

fn line_caret(
    line: &ShapedLine,
    line_index: usize,
    position: TextPosition,
    x_bits: i32,
) -> Result<TextCaret, TextError> {
    let top_bits = line
        .baseline_y_bits
        .checked_sub(line.metrics.ascent_bits())
        .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
    let bottom_bits = line
        .baseline_y_bits
        .checked_add(line.metrics.descent_bits())
        .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
    Ok(TextCaret {
        position,
        line_index,
        x_bits,
        top_bits,
        bottom_bits,
    })
}

struct LineCandidate {
    paragraph: Option<ShapedParagraph>,
    source_start: usize,
    source_end: usize,
    raw_end: usize,
    hard_break: bool,
    hyphenated: bool,
    ellipsized: bool,
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
