use unicode_bidi::{BidiInfo, LTR_LEVEL, Level, ParagraphInfo, RTL_LEVEL};
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    FontFace, FontId, FontMetrics, FontSlant, FontStyle, FontWidth, GlyphOutline,
    GlyphOutlineProvider, GlyphRun, TextError, TextErrorCode,
};

/// Horizontal direction of one shaped text run.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TextDirection {
    /// Glyphs belong to a left-to-right embedding run.
    LeftToRight,
    /// Glyphs belong to a right-to-left embedding run.
    RightToLeft,
}

/// Resource ceilings for one ordered in-memory font collection.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FontCollectionLimits {
    max_faces: usize,
    max_text_bytes: usize,
    max_runs: usize,
    max_glyphs: usize,
}

impl FontCollectionLimits {
    /// Creates positive font, text, run, and glyph ceilings.
    pub const fn new(
        max_faces: usize,
        max_text_bytes: usize,
        max_runs: usize,
        max_glyphs: usize,
    ) -> Result<Self, TextError> {
        if max_faces == 0 || max_text_bytes == 0 || max_runs == 0 || max_glyphs == 0 {
            return Err(TextError::new(TextErrorCode::InvalidLimits));
        }
        Ok(Self {
            max_faces,
            max_text_bytes,
            max_runs,
            max_glyphs,
        })
    }

    /// Returns the maximum number of faces.
    pub const fn max_faces(self) -> usize {
        self.max_faces
    }

    /// Returns the maximum UTF-8 paragraph size.
    pub const fn max_text_bytes(self) -> usize {
        self.max_text_bytes
    }

    /// Returns the maximum number of output runs.
    pub const fn max_runs(self) -> usize {
        self.max_runs
    }

    /// Returns the maximum total shaped glyph count.
    pub const fn max_glyphs(self) -> usize {
        self.max_glyphs
    }
}

impl Default for FontCollectionLimits {
    fn default() -> Self {
        Self {
            max_faces: 256,
            max_text_bytes: 4 * 1024 * 1024,
            max_runs: 1_000_000,
            max_glyphs: 1_000_000,
        }
    }
}

/// One font-specific run positioned in visual paragraph order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShapedRun {
    run: GlyphRun,
    source_start: u32,
    source_end: u32,
    origin_x_bits: i32,
    glyph_offsets_x_bits: Vec<i32>,
    direction: TextDirection,
}

impl ShapedRun {
    /// Borrows the portable glyph run.
    pub const fn glyph_run(&self) -> &GlyphRun {
        &self.run
    }

    /// Returns the inclusive source UTF-8 byte start.
    pub const fn source_start(&self) -> u32 {
        self.source_start
    }

    /// Returns the exclusive source UTF-8 byte end.
    pub const fn source_end(&self) -> u32 {
        self.source_end
    }

    /// Returns the run's visual horizontal origin in Q16.16 canvas units.
    pub const fn origin_x_bits(&self) -> i32 {
        self.origin_x_bits
    }

    /// Borrows additional per-glyph Q16.16 horizontal layout offsets.
    ///
    /// The slice has exactly one entry per glyph and contains zeros unless a
    /// higher-level layout operation, such as justification, moved glyphs.
    pub fn glyph_offsets_x_bits(&self) -> &[i32] {
        &self.glyph_offsets_x_bits
    }

    /// Returns the resolved embedding direction.
    pub const fn direction(&self) -> TextDirection {
        self.direction
    }
}

/// Shaped output for one unwrapped bidi paragraph.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShapedParagraph {
    runs: Vec<ShapedRun>,
    advance_x_bits: i32,
    base_direction: TextDirection,
    metrics: FontMetrics,
}

impl ShapedParagraph {
    /// Borrows font-specific runs in visual left-to-right order.
    pub fn runs(&self) -> &[ShapedRun] {
        &self.runs
    }

    /// Returns the paragraph's horizontal advance in Q16.16 canvas units.
    pub const fn advance_x_bits(&self) -> i32 {
        self.advance_x_bits
    }

    /// Returns the resolved paragraph base direction.
    pub const fn base_direction(&self) -> TextDirection {
        self.base_direction
    }

    /// Returns maximum baseline metrics across fonts used by the paragraph.
    pub const fn metrics(&self) -> FontMetrics {
        self.metrics
    }

    pub(crate) fn justify_ascii_spaces(
        &mut self,
        text: &str,
        source_start: usize,
        source_end: usize,
        target_advance_bits: i32,
    ) -> Result<bool, TextError> {
        if source_start > source_end
            || source_end > text.len()
            || !text.is_char_boundary(source_start)
            || !text.is_char_boundary(source_end)
            || target_advance_bits <= self.advance_x_bits
        {
            return Ok(false);
        }

        let line = &text[source_start..source_end];
        let first_non_space = line
            .char_indices()
            .find(|(_, character)| *character != ' ')
            .map(|(index, _)| source_start + index);
        let last_non_space_end = line
            .char_indices()
            .rev()
            .find(|(_, character)| *character != ' ')
            .map(|(index, character)| source_start + index + character.len_utf8());
        let (Some(first_non_space), Some(last_non_space_end)) =
            (first_non_space, last_non_space_end)
        else {
            return Ok(false);
        };

        let mut expansion_clusters = Vec::new();
        for (relative, character) in line.char_indices() {
            let cluster = source_start + relative;
            if character == ' '
                && cluster > first_non_space
                && cluster + character.len_utf8() < last_non_space_end
            {
                expansion_clusters
                    .try_reserve(1)
                    .map_err(|_| TextError::new(TextErrorCode::AllocationFailed))?;
                expansion_clusters.push(
                    u32::try_from(cluster)
                        .map_err(|_| TextError::new(TextErrorCode::ResourceLimit))?,
                );
            }
        }
        if expansion_clusters.is_empty() {
            return Ok(false);
        }

        let extra_bits = target_advance_bits
            .checked_sub(self.advance_x_bits)
            .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
        let slot_count = i32::try_from(expansion_clusters.len())
            .map_err(|_| TextError::new(TextErrorCode::ResourceLimit))?;
        let per_slot = extra_bits / slot_count;
        let mut remainder = extra_bits % slot_count;
        let mut cumulative_shift = 0_i32;
        let mut expanded = Vec::new();
        expanded
            .try_reserve_exact(expansion_clusters.len())
            .map_err(|_| TextError::new(TextErrorCode::AllocationFailed))?;
        expanded.resize(expansion_clusters.len(), false);

        for run in &mut self.runs {
            if run.glyph_offsets_x_bits.len() != run.run.glyphs().len() {
                return Err(TextError::new(TextErrorCode::InvalidLayout));
            }
            for (glyph, offset) in run.run.glyphs().iter().zip(&mut run.glyph_offsets_x_bits) {
                *offset = cumulative_shift;
                if let Ok(slot) = expansion_clusters.binary_search(&glyph.cluster())
                    && !expanded[slot]
                {
                    expanded[slot] = true;
                    let increment = per_slot + i32::from(remainder > 0);
                    remainder = remainder.saturating_sub(1);
                    cumulative_shift = cumulative_shift
                        .checked_add(increment)
                        .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
                }
            }
        }
        if expanded.iter().any(|expanded| !expanded) {
            for run in &mut self.runs {
                run.glyph_offsets_x_bits.fill(0);
            }
            return Ok(false);
        }
        if cumulative_shift != extra_bits {
            return Err(TextError::new(TextErrorCode::InvalidLayout));
        }
        self.advance_x_bits = target_advance_bits;
        Ok(true)
    }
}

/// Ordered font faces used for deterministic grapheme-level fallback.
#[derive(Clone, Debug)]
pub struct FontCollection {
    faces: Vec<FontFace>,
    limits: FontCollectionLimits,
}

impl FontCollection {
    /// Creates an empty collection with explicit resource ceilings.
    pub const fn new(limits: FontCollectionLimits) -> Self {
        Self {
            faces: Vec::new(),
            limits,
        }
    }

    /// Appends one fallback face after all existing faces.
    pub fn add_face(&mut self, face: FontFace) -> Result<(), TextError> {
        if self.faces.len() == self.limits.max_faces {
            return Err(TextError::new(TextErrorCode::ResourceLimit));
        }
        if self.faces.iter().any(|existing| existing.id() == face.id()) {
            return Err(TextError::new(TextErrorCode::DuplicateFontId));
        }
        self.faces
            .try_reserve(1)
            .map_err(|_| TextError::new(TextErrorCode::AllocationFailed))?;
        self.faces.push(face);
        Ok(())
    }

    /// Borrows faces in fallback priority order.
    pub fn faces(&self) -> &[FontFace] {
        &self.faces
    }

    /// Resolves one stable face identifier.
    pub fn face(&self, id: FontId) -> Option<&FontFace> {
        self.faces.iter().find(|face| face.id() == id)
    }

    /// Selects the closest style in one named family using CSS-like matching.
    ///
    /// Family comparison is ASCII case-insensitive. Width is matched before
    /// slant and weight; exact ties preserve face insertion order. This method
    /// does not test character coverage, so callers can build an ordered
    /// fallback list independently from family selection.
    pub fn match_face(&self, family: &str, style: FontStyle) -> Option<&FontFace> {
        self.faces
            .iter()
            .enumerate()
            .filter(|(_, face)| {
                face.family_name()
                    .is_some_and(|name| name.eq_ignore_ascii_case(family))
            })
            .min_by_key(|(index, face)| style_match_key(face.style(), style, *index))
            .map(|(_, face)| face)
    }

    /// Selects from the first available family in caller-provided priority order.
    pub fn match_face_for_families(
        &self,
        families: &[&str],
        style: FontStyle,
    ) -> Option<&FontFace> {
        families
            .iter()
            .find_map(|family| self.match_face(family, style))
    }

    /// Shapes one unwrapped paragraph with an automatically detected base direction.
    pub fn shape_paragraph(
        &self,
        text: &str,
        font_size_bits: i32,
    ) -> Result<ShapedParagraph, TextError> {
        self.shape_paragraph_impl(text, font_size_bits, None)
    }

    /// Shapes one unwrapped paragraph with an explicit base direction.
    pub fn shape_paragraph_with_direction(
        &self,
        text: &str,
        font_size_bits: i32,
        base_direction: TextDirection,
    ) -> Result<ShapedParagraph, TextError> {
        self.shape_paragraph_impl(text, font_size_bits, Some(base_direction))
    }

    pub(crate) fn shape_bidi_line(
        &self,
        text: &str,
        font_size_bits: i32,
        bidi: &BidiInfo<'_>,
        paragraph: &ParagraphInfo,
        line: std::ops::Range<usize>,
    ) -> Result<ShapedParagraph, TextError> {
        if line.is_empty() || line.start < paragraph.range.start || line.end > paragraph.range.end {
            return Err(TextError::new(TextErrorCode::InvalidLayout));
        }
        self.shape_bidi_range(text, font_size_bits, bidi, paragraph, line, 0)
    }

    pub(crate) fn default_metrics(&self, font_size_bits: i32) -> Result<FontMetrics, TextError> {
        self.faces
            .first()
            .ok_or(TextError::new(TextErrorCode::EmptyFontCollection))?
            .metrics(font_size_bits)
    }

    pub(crate) const fn limits(&self) -> FontCollectionLimits {
        self.limits
    }

    fn shape_paragraph_impl(
        &self,
        text: &str,
        font_size_bits: i32,
        base_direction: Option<TextDirection>,
    ) -> Result<ShapedParagraph, TextError> {
        if self.faces.is_empty() {
            return Err(TextError::new(TextErrorCode::EmptyFontCollection));
        }
        if font_size_bits <= 0 {
            return Err(TextError::new(TextErrorCode::InvalidFontSize));
        }
        if text.is_empty() {
            return Err(TextError::new(TextErrorCode::EmptyText));
        }
        if text.len() > self.limits.max_text_bytes || text.len() > u32::MAX as usize {
            return Err(TextError::new(TextErrorCode::ResourceLimit));
        }

        let default_level = base_direction.map(direction_level);
        let bidi = BidiInfo::new(text, default_level);
        if bidi.paragraphs.len() != 1 {
            return Err(TextError::new(TextErrorCode::MultipleParagraphs));
        }
        let paragraph = &bidi.paragraphs[0];
        self.shape_bidi_range(
            text,
            font_size_bits,
            &bidi,
            paragraph,
            paragraph.range.clone(),
            0,
        )
    }

    fn shape_bidi_range(
        &self,
        text: &str,
        font_size_bits: i32,
        bidi: &BidiInfo<'_>,
        paragraph: &ParagraphInfo,
        line: std::ops::Range<usize>,
        source_offset: u32,
    ) -> Result<ShapedParagraph, TextError> {
        let resolved_base = level_direction(paragraph.level);
        let (levels, visual_runs) = bidi.visual_runs(paragraph, line);

        let mut logical_segments = Vec::new();
        for visual_run in visual_runs {
            let direction = level_direction(levels[visual_run.start]);
            let first_segment = logical_segments.len();
            self.append_fallback_segments(text, visual_run, direction, &mut logical_segments)?;
            if direction == TextDirection::RightToLeft {
                logical_segments[first_segment..].reverse();
            }
        }

        let mut runs = Vec::new();
        runs.try_reserve_exact(logical_segments.len())
            .map_err(|_| TextError::new(TextErrorCode::AllocationFailed))?;
        let mut origin_x_bits = 0_i32;
        let mut glyph_count = 0_usize;
        let mut ascent_bits = 0_i32;
        let mut descent_bits = 0_i32;
        let mut line_gap_bits = 0_i32;
        for segment in logical_segments {
            if runs.len() == self.limits.max_runs {
                return Err(TextError::new(TextErrorCode::ResourceLimit));
            }
            let source_start = u32::try_from(segment.start)
                .ok()
                .and_then(|start| start.checked_add(source_offset))
                .ok_or(TextError::new(TextErrorCode::ResourceLimit))?;
            let source_end = u32::try_from(segment.end)
                .ok()
                .and_then(|end| end.checked_add(source_offset))
                .ok_or(TextError::new(TextErrorCode::ResourceLimit))?;
            let face = &self.faces[segment.face_index];
            let run = face.shape_segment(
                &text[segment.start..segment.end],
                font_size_bits,
                Some(segment.direction),
                source_start,
            )?;
            glyph_count = glyph_count
                .checked_add(run.glyphs().len())
                .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
            if glyph_count > self.limits.max_glyphs {
                return Err(TextError::new(TextErrorCode::ResourceLimit));
            }
            let metrics = face.metrics(font_size_bits)?;
            ascent_bits = ascent_bits.max(metrics.ascent_bits());
            descent_bits = descent_bits.max(metrics.descent_bits());
            line_gap_bits = line_gap_bits.max(metrics.line_gap_bits());
            let advance = run_advance_bits(&run)?;
            let mut glyph_offsets_x_bits = Vec::new();
            glyph_offsets_x_bits
                .try_reserve_exact(run.glyphs().len())
                .map_err(|_| TextError::new(TextErrorCode::AllocationFailed))?;
            glyph_offsets_x_bits.resize(run.glyphs().len(), 0);
            runs.push(ShapedRun {
                glyph_offsets_x_bits,
                run,
                source_start,
                source_end,
                origin_x_bits,
                direction: segment.direction,
            });
            origin_x_bits = origin_x_bits
                .checked_add(advance)
                .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
        }

        Ok(ShapedParagraph {
            runs,
            advance_x_bits: origin_x_bits,
            base_direction: resolved_base,
            metrics: FontMetrics::from_bits(ascent_bits, descent_bits, line_gap_bits),
        })
    }

    fn append_fallback_segments(
        &self,
        text: &str,
        source: std::ops::Range<usize>,
        direction: TextDirection,
        output: &mut Vec<LogicalSegment>,
    ) -> Result<(), TextError> {
        let source_text = &text[source.clone()];
        let output_start = output.len();
        for (relative_start, grapheme) in source_text.grapheme_indices(true) {
            let start = source.start + relative_start;
            let end = start + grapheme.len();
            let face_index = self
                .fallback_face(grapheme)?
                .ok_or(TextError::new(TextErrorCode::MissingGlyph))?;
            let can_merge = output.len() > output_start;
            if can_merge
                && let Some(previous) = output.last_mut()
                && previous.face_index == face_index
                && previous.direction == direction
                && previous.end == start
            {
                previous.end = end;
            } else {
                if output.len() == self.limits.max_runs {
                    return Err(TextError::new(TextErrorCode::ResourceLimit));
                }
                output
                    .try_reserve(1)
                    .map_err(|_| TextError::new(TextErrorCode::AllocationFailed))?;
                output.push(LogicalSegment {
                    face_index,
                    start,
                    end,
                    direction,
                });
            }
        }
        Ok(())
    }

    fn fallback_face(&self, grapheme: &str) -> Result<Option<usize>, TextError> {
        for (index, face) in self.faces.iter().enumerate() {
            if supports_grapheme(face, grapheme)? {
                return Ok(Some(index));
            }
        }
        Ok(None)
    }
}

impl GlyphOutlineProvider for FontCollection {
    fn glyph_outline(
        &self,
        font: FontId,
        glyph: crate::GlyphId,
    ) -> Result<Option<GlyphOutline>, TextError> {
        let Some(face) = self.face(font) else {
            return Ok(None);
        };
        face.glyph_outline(font, glyph)
    }
}

#[derive(Clone, Copy, Debug)]
struct LogicalSegment {
    face_index: usize,
    start: usize,
    end: usize,
    direction: TextDirection,
}

fn supports_grapheme(face: &FontFace, grapheme: &str) -> Result<bool, TextError> {
    for character in grapheme.chars() {
        if is_default_ignorable_for_fallback(character) || character.is_control() {
            continue;
        }
        if !face.supports_character(character)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn is_default_ignorable_for_fallback(character: char) -> bool {
    matches!(
        character,
        '\u{00ad}'
            | '\u{034f}'
            | '\u{061c}'
            | '\u{115f}'..='\u{1160}'
            | '\u{17b4}'..='\u{17b5}'
            | '\u{180b}'..='\u{180f}'
            | '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{206f}'
            | '\u{3164}'
            | '\u{fe00}'..='\u{fe0f}'
            | '\u{feff}'
            | '\u{ffa0}'
            | '\u{fff0}'..='\u{fff8}'
            | '\u{1bca0}'..='\u{1bca3}'
            | '\u{1d173}'..='\u{1d17a}'
            | '\u{e0000}'..='\u{e0fff}'
    )
}

fn direction_level(direction: TextDirection) -> Level {
    match direction {
        TextDirection::LeftToRight => LTR_LEVEL,
        TextDirection::RightToLeft => RTL_LEVEL,
    }
}

fn level_direction(level: Level) -> TextDirection {
    if level.is_rtl() {
        TextDirection::RightToLeft
    } else {
        TextDirection::LeftToRight
    }
}

fn style_match_key(
    candidate: FontStyle,
    requested: FontStyle,
    insertion_index: usize,
) -> (u8, u16, u8, u8, u16, usize) {
    let (width_group, width_distance) = width_match_rank(candidate.width(), requested.width());
    let slant_rank = slant_match_rank(candidate.slant(), requested.slant());
    let (weight_group, weight_distance) = weight_match_rank(candidate.weight(), requested.weight());
    (
        width_group,
        width_distance,
        slant_rank,
        weight_group,
        weight_distance,
        insertion_index,
    )
}

fn width_match_rank(candidate: FontWidth, requested: FontWidth) -> (u8, u16) {
    let candidate = candidate.class();
    let requested = requested.class();
    if candidate == requested {
        (0, 0)
    } else if requested <= FontWidth::Normal.class() {
        if candidate < requested {
            (1, requested - candidate)
        } else {
            (2, candidate - requested)
        }
    } else if candidate > requested {
        (1, candidate - requested)
    } else {
        (2, requested - candidate)
    }
}

const fn slant_match_rank(candidate: FontSlant, requested: FontSlant) -> u8 {
    match requested {
        FontSlant::Italic => match candidate {
            FontSlant::Italic => 0,
            FontSlant::Oblique => 1,
            FontSlant::Normal => 2,
        },
        FontSlant::Oblique => match candidate {
            FontSlant::Oblique => 0,
            FontSlant::Italic => 1,
            FontSlant::Normal => 2,
        },
        FontSlant::Normal => match candidate {
            FontSlant::Normal => 0,
            FontSlant::Oblique => 1,
            FontSlant::Italic => 2,
        },
    }
}

fn weight_match_rank(candidate: u16, requested: u16) -> (u8, u16) {
    if candidate == requested {
        return (0, 0);
    }
    if (400..450).contains(&requested) {
        if candidate == 500 {
            return (1, 0);
        }
        if candidate < requested {
            return (2, requested - candidate);
        }
        return (3, candidate - requested);
    }
    if (450..=500).contains(&requested) {
        if candidate == 400 {
            return (1, 0);
        }
        if candidate < requested {
            return (2, requested - candidate);
        }
        return (3, candidate - requested);
    }
    if requested <= 500 {
        if candidate < requested {
            (1, requested - candidate)
        } else {
            (2, candidate - requested)
        }
    } else if candidate > requested {
        (1, candidate - requested)
    } else {
        (2, requested - candidate)
    }
}

fn run_advance_bits(run: &GlyphRun) -> Result<i32, TextError> {
    let design_bits = run.glyphs().iter().try_fold(0_i64, |total, glyph| {
        total
            .checked_add(i64::from(glyph.advance_x().bits()))
            .ok_or(TextError::new(TextErrorCode::NumericOverflow))
    })?;
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
