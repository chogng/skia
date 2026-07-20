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

/// One contiguous source range with a preferred font instance and font size.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TextStyleSpan {
    source_start: u32,
    source_end: u32,
    font: FontId,
    font_size_bits: i32,
}

impl TextStyleSpan {
    /// Creates one non-empty styled UTF-8 byte range.
    pub const fn new(
        source_start: u32,
        source_end: u32,
        font: FontId,
        font_size_bits: i32,
    ) -> Result<Self, TextError> {
        if source_start >= source_end || font_size_bits <= 0 {
            return Err(TextError::new(TextErrorCode::InvalidTextStyleSpan));
        }
        Ok(Self {
            source_start,
            source_end,
            font,
            font_size_bits,
        })
    }

    /// Returns the inclusive source UTF-8 byte start.
    pub const fn source_start(self) -> u32 {
        self.source_start
    }

    /// Returns the exclusive source UTF-8 byte end.
    pub const fn source_end(self) -> u32 {
        self.source_end
    }

    /// Returns the preferred immutable font instance.
    pub const fn font(self) -> FontId {
        self.font
    }

    /// Returns the positive Q16.16 font size.
    pub const fn font_size_bits(self) -> i32 {
        self.font_size_bits
    }
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
    spacing_added_bits: i32,
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

    pub(crate) fn apply_spacing(
        &mut self,
        text: &str,
        source_start: usize,
        source_end: usize,
        letter_spacing_bits: i32,
        word_spacing_bits: i32,
    ) -> Result<(), TextError> {
        if source_start > source_end
            || source_end > text.len()
            || !text.is_char_boundary(source_start)
            || !text.is_char_boundary(source_end)
        {
            return Err(TextError::new(TextErrorCode::InvalidLayout));
        }
        let raw_advance_bits = self
            .advance_x_bits
            .checked_sub(self.spacing_added_bits)
            .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
        let glyph_count = self.runs.iter().try_fold(0_usize, |total, shaped| {
            if shaped.glyph_offsets_x_bits.len() != shaped.run.glyphs().len() {
                return Err(TextError::new(TextErrorCode::InvalidLayout));
            }
            total
                .checked_add(shaped.run.glyphs().len())
                .ok_or(TextError::new(TextErrorCode::NumericOverflow))
        })?;
        let mut clusters = Vec::new();
        clusters
            .try_reserve(glyph_count)
            .map_err(|_| TextError::new(TextErrorCode::AllocationFailed))?;
        for shaped in &self.runs {
            let mut previous_cluster = None;
            for glyph in shaped.run.glyphs() {
                if previous_cluster != Some(glyph.cluster()) {
                    clusters.push(glyph.cluster());
                    previous_cluster = Some(glyph.cluster());
                }
            }
        }
        let mut total_spacing_bits = 0_i32;
        for &cluster in clusters.iter().take(clusters.len().saturating_sub(1)) {
            let word_increment = spacing_character(text, source_start, source_end, cluster)
                .filter(|character| is_expandable_justification_space(*character))
                .map_or(0, |_| word_spacing_bits);
            total_spacing_bits = total_spacing_bits
                .checked_add(letter_spacing_bits)
                .and_then(|value| value.checked_add(word_increment))
                .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
        }
        let final_advance_bits = raw_advance_bits
            .checked_add(total_spacing_bits)
            .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
        if final_advance_bits < 0 {
            return Err(TextError::new(TextErrorCode::InvalidLayout));
        }

        let mut cumulative_spacing_bits = 0_i32;
        let mut cluster_index = 0_usize;
        for shaped in &mut self.runs {
            let glyphs = shaped.run.glyphs();
            let mut first = 0_usize;
            while first < glyphs.len() {
                let cluster = glyphs[first].cluster();
                let mut end = first + 1;
                while end < glyphs.len() && glyphs[end].cluster() == cluster {
                    end += 1;
                }
                shaped.glyph_offsets_x_bits[first..end].fill(cumulative_spacing_bits);
                cluster_index += 1;
                if cluster_index < clusters.len() {
                    let word_increment = spacing_character(text, source_start, source_end, cluster)
                        .filter(|character| is_expandable_justification_space(*character))
                        .map_or(0, |_| word_spacing_bits);
                    cumulative_spacing_bits = cumulative_spacing_bits
                        .checked_add(letter_spacing_bits)
                        .and_then(|value| value.checked_add(word_increment))
                        .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
                }
                first = end;
            }
        }
        if cumulative_spacing_bits != total_spacing_bits {
            return Err(TextError::new(TextErrorCode::InvalidLayout));
        }
        self.advance_x_bits = final_advance_bits;
        self.spacing_added_bits = total_spacing_bits;
        Ok(())
    }

    pub(crate) fn justify_line(
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
            .find(|(_, character)| !is_expandable_justification_space(*character))
            .map(|(index, _)| source_start + index);
        let last_non_space_end = line
            .char_indices()
            .rev()
            .find(|(_, character)| !is_expandable_justification_space(*character))
            .map(|(index, character)| source_start + index + character.len_utf8());
        let (Some(first_non_space), Some(last_non_space_end)) =
            (first_non_space, last_non_space_end)
        else {
            return Ok(false);
        };

        let mut expansion_clusters = Vec::new();
        for (relative, character) in line.char_indices() {
            let cluster = source_start + relative;
            if is_expandable_justification_space(character)
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
            let mut visual_clusters = Vec::new();
            for run in &self.runs {
                let glyphs = run.run.glyphs();
                let mut first = 0_usize;
                while first < glyphs.len() {
                    let cluster = glyphs[first].cluster();
                    if usize::try_from(cluster)
                        .is_ok_and(|cluster| cluster >= source_start && cluster < source_end)
                    {
                        visual_clusters
                            .try_reserve(1)
                            .map_err(|_| TextError::new(TextErrorCode::AllocationFailed))?;
                        visual_clusters.push(cluster);
                    }
                    first += 1;
                    while first < glyphs.len() && glyphs[first].cluster() == cluster {
                        first += 1;
                    }
                }
            }
            for pair in visual_clusters.windows(2) {
                let first = spacing_character(text, source_start, source_end, pair[0]);
                let second = spacing_character(text, source_start, source_end, pair[1]);
                if first.is_some_and(is_cjk_inter_character_unit)
                    && second.is_some_and(is_cjk_inter_character_unit)
                {
                    expansion_clusters
                        .try_reserve(1)
                        .map_err(|_| TextError::new(TextErrorCode::AllocationFailed))?;
                    expansion_clusters.push(pair[0]);
                }
            }
            expansion_clusters.sort_unstable();
            expansion_clusters.dedup();
            if expansion_clusters.is_empty() {
                return Ok(false);
            }
        }

        let extra_bits = target_advance_bits
            .checked_sub(self.advance_x_bits)
            .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
        let slot_count = i32::try_from(expansion_clusters.len())
            .map_err(|_| TextError::new(TextErrorCode::ResourceLimit))?;
        let per_slot = extra_bits / slot_count;
        let mut remainder = extra_bits % slot_count;
        let mut expanded = Vec::new();
        expanded
            .try_reserve_exact(expansion_clusters.len())
            .map_err(|_| TextError::new(TextErrorCode::AllocationFailed))?;
        expanded.resize(expansion_clusters.len(), false);

        for run in &self.runs {
            if run.glyph_offsets_x_bits.len() != run.run.glyphs().len() {
                return Err(TextError::new(TextErrorCode::InvalidLayout));
            }
            for glyph in run.run.glyphs() {
                if let Ok(slot) = expansion_clusters.binary_search(&glyph.cluster()) {
                    expanded[slot] = true;
                }
            }
        }
        if expanded.iter().any(|expanded| !expanded) {
            return Ok(false);
        }
        if self.runs.iter().any(|run| {
            run.glyph_offsets_x_bits
                .iter()
                .any(|offset| offset.checked_add(extra_bits).is_none())
        }) {
            return Err(TextError::new(TextErrorCode::NumericOverflow));
        }

        let mut cumulative_shift = 0_i32;
        for run in &mut self.runs {
            let glyphs = run.run.glyphs();
            let mut first = 0_usize;
            while first < glyphs.len() {
                let cluster = glyphs[first].cluster();
                let mut end = first + 1;
                while end < glyphs.len() && glyphs[end].cluster() == cluster {
                    end += 1;
                }
                for offset in &mut run.glyph_offsets_x_bits[first..end] {
                    *offset += cumulative_shift;
                }
                if expansion_clusters.binary_search(&cluster).is_ok() {
                    let consumes_remainder = remainder > 0;
                    let increment = per_slot + i32::from(consumes_remainder);
                    if consumes_remainder {
                        remainder -= 1;
                    }
                    cumulative_shift = cumulative_shift
                        .checked_add(increment)
                        .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
                }
                first = end;
            }
        }
        if cumulative_shift != extra_bits || remainder != 0 {
            return Err(TextError::new(TextErrorCode::InvalidLayout));
        }
        self.advance_x_bits = target_advance_bits;
        Ok(true)
    }
}

fn spacing_character(
    text: &str,
    source_start: usize,
    source_end: usize,
    cluster: u32,
) -> Option<char> {
    let cluster = usize::try_from(cluster).ok()?;
    if cluster < source_start || cluster >= source_end || !text.is_char_boundary(cluster) {
        return None;
    }
    text[cluster..source_end].chars().next()
}

const fn is_expandable_justification_space(character: char) -> bool {
    matches!(
        character,
        '\u{0020}'
            | '\u{1680}'
            | '\u{2000}'..='\u{2006}'
            | '\u{2008}'..='\u{200a}'
            | '\u{205f}'
            | '\u{3000}'
    )
}

const fn is_cjk_inter_character_unit(character: char) -> bool {
    matches!(
        character,
        '\u{1100}'..='\u{11ff}'
            | '\u{2e80}'..='\u{2fdf}'
            | '\u{3040}'..='\u{30ff}'
            | '\u{3100}'..='\u{31bf}'
            | '\u{31f0}'..='\u{31ff}'
            | '\u{3400}'..='\u{4dbf}'
            | '\u{4e00}'..='\u{9fff}'
            | '\u{a960}'..='\u{a97f}'
            | '\u{ac00}'..='\u{d7ff}'
            | '\u{f900}'..='\u{faff}'
            | '\u{ff66}'..='\u{ff9d}'
            | '\u{ffa0}'..='\u{ffdc}'
            | '\u{1b000}'..='\u{1b16f}'
            | '\u{20000}'..='\u{323af}'
    )
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
        self.shape_paragraph_impl(text, font_size_bits, None, None)
    }

    /// Shapes one paragraph with a BCP 47-style OpenType language.
    pub fn shape_paragraph_with_language(
        &self,
        text: &str,
        font_size_bits: i32,
        language: &str,
    ) -> Result<ShapedParagraph, TextError> {
        self.shape_paragraph_impl(text, font_size_bits, None, Some(language))
    }

    /// Shapes one unwrapped paragraph with an explicit base direction.
    pub fn shape_paragraph_with_direction(
        &self,
        text: &str,
        font_size_bits: i32,
        base_direction: TextDirection,
    ) -> Result<ShapedParagraph, TextError> {
        self.shape_paragraph_impl(text, font_size_bits, Some(base_direction), None)
    }

    /// Shapes one paragraph with explicit base direction and language.
    pub fn shape_paragraph_with_direction_and_language(
        &self,
        text: &str,
        font_size_bits: i32,
        base_direction: TextDirection,
        language: &str,
    ) -> Result<ShapedParagraph, TextError> {
        self.shape_paragraph_impl(text, font_size_bits, Some(base_direction), Some(language))
    }

    /// Shapes one styled, unwrapped paragraph with automatic base direction.
    ///
    /// Spans must be ordered, contiguous, cover the entire text, and meet only
    /// at extended-grapheme boundaries. A span's font is tried first for every
    /// grapheme, followed by the collection's normal fallback order.
    pub fn shape_styled_paragraph(
        &self,
        text: &str,
        spans: &[TextStyleSpan],
    ) -> Result<ShapedParagraph, TextError> {
        self.shape_styled_paragraph_impl(text, spans, None, None)
    }

    /// Shapes one styled paragraph with a BCP 47-style OpenType language.
    pub fn shape_styled_paragraph_with_language(
        &self,
        text: &str,
        spans: &[TextStyleSpan],
        language: &str,
    ) -> Result<ShapedParagraph, TextError> {
        self.shape_styled_paragraph_impl(text, spans, None, Some(language))
    }

    /// Shapes one styled, unwrapped paragraph with an explicit base direction.
    pub fn shape_styled_paragraph_with_direction(
        &self,
        text: &str,
        spans: &[TextStyleSpan],
        base_direction: TextDirection,
    ) -> Result<ShapedParagraph, TextError> {
        self.shape_styled_paragraph_impl(text, spans, Some(base_direction), None)
    }

    /// Shapes one styled paragraph with explicit base direction and language.
    pub fn shape_styled_paragraph_with_direction_and_language(
        &self,
        text: &str,
        spans: &[TextStyleSpan],
        base_direction: TextDirection,
        language: &str,
    ) -> Result<ShapedParagraph, TextError> {
        self.shape_styled_paragraph_impl(text, spans, Some(base_direction), Some(language))
    }

    pub(crate) fn shape_bidi_line(
        &self,
        text: &str,
        font_size_bits: i32,
        bidi: &BidiInfo<'_>,
        paragraph: &ParagraphInfo,
        line: std::ops::Range<usize>,
        language: Option<&str>,
    ) -> Result<ShapedParagraph, TextError> {
        if line.is_empty() || line.start < paragraph.range.start || line.end > paragraph.range.end {
            return Err(TextError::new(TextErrorCode::InvalidLayout));
        }
        self.shape_bidi_range(
            text,
            ParagraphStyle::Uniform(font_size_bits),
            bidi,
            paragraph,
            line,
            0,
            language,
        )
    }

    pub(crate) fn shape_styled_bidi_line(
        &self,
        text: &str,
        spans: &[TextStyleSpan],
        bidi: &BidiInfo<'_>,
        paragraph: &ParagraphInfo,
        line: std::ops::Range<usize>,
        language: Option<&str>,
    ) -> Result<ShapedParagraph, TextError> {
        if line.is_empty() || line.start < paragraph.range.start || line.end > paragraph.range.end {
            return Err(TextError::new(TextErrorCode::InvalidLayout));
        }
        self.shape_bidi_range(
            text,
            ParagraphStyle::Spans(spans),
            bidi,
            paragraph,
            line,
            0,
            language,
        )
    }

    pub(crate) fn append_discretionary_hyphen(
        &self,
        paragraph: &mut ShapedParagraph,
        source_offset: u32,
        language: Option<&str>,
    ) -> Result<(), TextError> {
        self.append_synthetic_marker(paragraph, source_offset, &["\u{2010}", "-"], language)
    }

    pub(crate) fn append_ellipsis(
        &self,
        paragraph: &mut ShapedParagraph,
        source_offset: u32,
        language: Option<&str>,
    ) -> Result<(), TextError> {
        self.append_synthetic_marker(paragraph, source_offset, &["\u{2026}", "..."], language)
    }

    pub(crate) fn shape_ellipsis_marker(
        &self,
        font_size_bits: i32,
        preferred_font: FontId,
        direction: TextDirection,
        source_offset: u32,
        language: Option<&str>,
    ) -> Result<ShapedParagraph, TextError> {
        let preferred_face = self
            .faces
            .iter()
            .position(|face| face.id() == preferred_font)
            .ok_or(TextError::new(TextErrorCode::InvalidLayout))?;
        let (face_index, marker) =
            self.select_synthetic_marker(&["\u{2026}", "..."], preferred_face)?;
        let face = &self.faces[face_index];
        let run = face.shape_segment(
            marker,
            font_size_bits,
            Some(direction),
            source_offset,
            language,
        )?;
        if run.glyphs().len() > self.limits.max_glyphs {
            return Err(TextError::new(TextErrorCode::ResourceLimit));
        }
        let advance_x_bits = run_advance_bits(&run)?;
        let mut glyph_offsets_x_bits = Vec::new();
        glyph_offsets_x_bits
            .try_reserve_exact(run.glyphs().len())
            .map_err(|_| TextError::new(TextErrorCode::AllocationFailed))?;
        glyph_offsets_x_bits.resize(run.glyphs().len(), 0);
        let mut runs = Vec::new();
        runs.try_reserve_exact(1)
            .map_err(|_| TextError::new(TextErrorCode::AllocationFailed))?;
        runs.push(ShapedRun {
            run,
            source_start: source_offset,
            source_end: source_offset,
            origin_x_bits: 0,
            glyph_offsets_x_bits,
            direction,
        });
        Ok(ShapedParagraph {
            runs,
            advance_x_bits,
            spacing_added_bits: 0,
            base_direction: direction,
            metrics: face.metrics(font_size_bits)?,
        })
    }

    fn append_synthetic_marker(
        &self,
        paragraph: &mut ShapedParagraph,
        source_offset: u32,
        markers: &[&str],
        language: Option<&str>,
    ) -> Result<(), TextError> {
        if paragraph.runs.len() == self.limits.max_runs {
            return Err(TextError::new(TextErrorCode::ResourceLimit));
        }
        let (anchor_index, anchor) = paragraph
            .runs
            .iter()
            .enumerate()
            .find(|(_, shaped)| shaped.source_end == source_offset)
            .ok_or(TextError::new(TextErrorCode::InvalidLayout))?;
        let marker_direction = anchor.direction;
        let font_size_bits = anchor.run.font_size_bits();
        let preferred_face = self
            .faces
            .iter()
            .position(|face| face.id() == anchor.run.font())
            .ok_or(TextError::new(TextErrorCode::InvalidLayout))?;
        let (face_index, marker) = self.select_synthetic_marker(markers, preferred_face)?;
        let insertion_index = if marker_direction == TextDirection::LeftToRight {
            anchor_index + 1
        } else {
            anchor_index
        };
        let marker_origin_bits = if marker_direction == TextDirection::LeftToRight {
            anchor
                .origin_x_bits
                .checked_add(run_advance_bits(&anchor.run)?)
                .ok_or(TextError::new(TextErrorCode::NumericOverflow))?
        } else {
            anchor.origin_x_bits
        };
        let face = &self.faces[face_index];
        let run = face.shape_segment(
            marker,
            font_size_bits,
            Some(marker_direction),
            source_offset,
            language,
        )?;
        let glyph_count = paragraph
            .runs
            .iter()
            .try_fold(0_usize, |total, shaped| {
                total.checked_add(shaped.run.glyphs().len())
            })
            .and_then(|total| total.checked_add(run.glyphs().len()))
            .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
        if glyph_count > self.limits.max_glyphs {
            return Err(TextError::new(TextErrorCode::ResourceLimit));
        }
        let advance_bits = run_advance_bits(&run)?;
        let final_advance_bits = paragraph
            .advance_x_bits
            .checked_add(advance_bits)
            .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
        if paragraph.runs[insertion_index..]
            .iter()
            .any(|shaped| shaped.origin_x_bits.checked_add(advance_bits).is_none())
        {
            return Err(TextError::new(TextErrorCode::NumericOverflow));
        }
        let mut glyph_offsets_x_bits = Vec::new();
        glyph_offsets_x_bits
            .try_reserve_exact(run.glyphs().len())
            .map_err(|_| TextError::new(TextErrorCode::AllocationFailed))?;
        glyph_offsets_x_bits.resize(run.glyphs().len(), 0);
        paragraph
            .runs
            .try_reserve(1)
            .map_err(|_| TextError::new(TextErrorCode::AllocationFailed))?;

        let metrics = face.metrics(font_size_bits)?;
        paragraph.metrics = FontMetrics::from_bits(
            paragraph.metrics.ascent_bits().max(metrics.ascent_bits()),
            paragraph.metrics.descent_bits().max(metrics.descent_bits()),
            paragraph
                .metrics
                .line_gap_bits()
                .max(metrics.line_gap_bits()),
        );
        let shaped_marker = ShapedRun {
            run,
            source_start: source_offset,
            source_end: source_offset,
            origin_x_bits: marker_origin_bits,
            glyph_offsets_x_bits,
            direction: marker_direction,
        };
        for shaped in &mut paragraph.runs[insertion_index..] {
            shaped.origin_x_bits += advance_bits;
        }
        paragraph.runs.insert(insertion_index, shaped_marker);
        paragraph.advance_x_bits = final_advance_bits;
        Ok(())
    }

    fn select_synthetic_marker<'a>(
        &self,
        markers: &'a [&str],
        preferred_face: usize,
    ) -> Result<(usize, &'a str), TextError> {
        for &marker in markers {
            if let Some(face_index) =
                self.fallback_face_with_preferred(marker, Some(preferred_face))?
            {
                return Ok((face_index, marker));
            }
        }
        Err(TextError::new(TextErrorCode::MissingGlyph))
    }

    pub(crate) const fn limits(&self) -> FontCollectionLimits {
        self.limits
    }

    fn shape_paragraph_impl(
        &self,
        text: &str,
        font_size_bits: i32,
        base_direction: Option<TextDirection>,
        language: Option<&str>,
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
        if language.is_some_and(|language| !crate::valid_language_tag(language)) {
            return Err(TextError::new(TextErrorCode::InvalidLanguage));
        }

        let default_level = base_direction.map(direction_level);
        let bidi = BidiInfo::new(text, default_level);
        if bidi.paragraphs.len() != 1 {
            return Err(TextError::new(TextErrorCode::MultipleParagraphs));
        }
        let paragraph = &bidi.paragraphs[0];
        self.shape_bidi_range(
            text,
            ParagraphStyle::Uniform(font_size_bits),
            &bidi,
            paragraph,
            paragraph.range.clone(),
            0,
            language,
        )
    }

    fn shape_styled_paragraph_impl(
        &self,
        text: &str,
        spans: &[TextStyleSpan],
        base_direction: Option<TextDirection>,
        language: Option<&str>,
    ) -> Result<ShapedParagraph, TextError> {
        if self.faces.is_empty() {
            return Err(TextError::new(TextErrorCode::EmptyFontCollection));
        }
        if text.is_empty() {
            return Err(TextError::new(TextErrorCode::EmptyText));
        }
        if text.len() > self.limits.max_text_bytes || text.len() > u32::MAX as usize {
            return Err(TextError::new(TextErrorCode::ResourceLimit));
        }
        if language.is_some_and(|language| !crate::valid_language_tag(language)) {
            return Err(TextError::new(TextErrorCode::InvalidLanguage));
        }
        self.validate_style_spans(text, spans)?;
        let bidi = BidiInfo::new(text, base_direction.map(direction_level));
        if bidi.paragraphs.len() != 1 {
            return Err(TextError::new(TextErrorCode::MultipleParagraphs));
        }
        let paragraph = &bidi.paragraphs[0];
        self.shape_bidi_range(
            text,
            ParagraphStyle::Spans(spans),
            &bidi,
            paragraph,
            paragraph.range.clone(),
            0,
            language,
        )
    }

    fn shape_bidi_range(
        &self,
        text: &str,
        style: ParagraphStyle<'_>,
        bidi: &BidiInfo<'_>,
        paragraph: &ParagraphInfo,
        line: std::ops::Range<usize>,
        source_offset: u32,
        language: Option<&str>,
    ) -> Result<ShapedParagraph, TextError> {
        let resolved_base = level_direction(paragraph.level);
        let (levels, visual_runs) = bidi.visual_runs(paragraph, line);

        let mut logical_segments = Vec::new();
        for visual_run in visual_runs {
            let direction = level_direction(levels[visual_run.start]);
            let first_segment = logical_segments.len();
            self.append_fallback_segments(
                text,
                visual_run,
                direction,
                style,
                &mut logical_segments,
            )?;
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
                segment.font_size_bits,
                Some(segment.direction),
                source_start,
                language,
            )?;
            glyph_count = glyph_count
                .checked_add(run.glyphs().len())
                .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
            if glyph_count > self.limits.max_glyphs {
                return Err(TextError::new(TextErrorCode::ResourceLimit));
            }
            let metrics = face.metrics(segment.font_size_bits)?;
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
            spacing_added_bits: 0,
            base_direction: resolved_base,
            metrics: FontMetrics::from_bits(ascent_bits, descent_bits, line_gap_bits),
        })
    }

    fn append_fallback_segments(
        &self,
        text: &str,
        source: std::ops::Range<usize>,
        direction: TextDirection,
        style: ParagraphStyle<'_>,
        output: &mut Vec<LogicalSegment>,
    ) -> Result<(), TextError> {
        let source_text = &text[source.clone()];
        let output_start = output.len();
        for (relative_start, grapheme) in source_text.grapheme_indices(true) {
            let start = source.start + relative_start;
            let end = start + grapheme.len();
            let (preferred_face, font_size_bits) = match style {
                ParagraphStyle::Spans(spans) => {
                    let span = spans
                        .get(spans.partition_point(|span| span.source_end as usize <= start))
                        .filter(|span| span.source_start as usize <= start)
                        .ok_or(TextError::new(TextErrorCode::InvalidTextStyleSpan))?;
                    if end > span.source_end as usize {
                        return Err(TextError::new(TextErrorCode::InvalidTextStyleSpan));
                    }
                    (
                        Some(
                            self.faces
                                .iter()
                                .position(|face| face.id() == span.font)
                                .ok_or(TextError::new(TextErrorCode::InvalidTextStyleSpan))?,
                        ),
                        span.font_size_bits,
                    )
                }
                ParagraphStyle::Uniform(font_size_bits) => (None, font_size_bits),
            };
            let face_index = self
                .fallback_face_with_preferred(grapheme, preferred_face)?
                .ok_or(TextError::new(TextErrorCode::MissingGlyph))?;
            let can_merge = output.len() > output_start;
            if can_merge
                && let Some(previous) = output.last_mut()
                && previous.face_index == face_index
                && previous.direction == direction
                && previous.font_size_bits == font_size_bits
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
                    font_size_bits,
                });
            }
        }
        Ok(())
    }

    fn fallback_face_with_preferred(
        &self,
        grapheme: &str,
        preferred: Option<usize>,
    ) -> Result<Option<usize>, TextError> {
        if let Some(index) = preferred
            && supports_grapheme(&self.faces[index], grapheme)?
        {
            return Ok(Some(index));
        }
        for (index, face) in self.faces.iter().enumerate() {
            if Some(index) == preferred {
                continue;
            }
            if supports_grapheme(face, grapheme)? {
                return Ok(Some(index));
            }
        }
        Ok(None)
    }

    pub(crate) fn validate_style_spans(
        &self,
        text: &str,
        spans: &[TextStyleSpan],
    ) -> Result<(), TextError> {
        if spans.is_empty() || spans.len() > self.limits.max_runs {
            return Err(TextError::new(TextErrorCode::InvalidTextStyleSpan));
        }
        let mut expected_start = 0_usize;
        let mut grapheme_boundaries = text
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .chain(std::iter::once(text.len()));
        let mut next_grapheme_boundary = grapheme_boundaries.next();
        for span in spans {
            let start = span.source_start as usize;
            let end = span.source_end as usize;
            if start != expected_start
                || end > text.len()
                || !text.is_char_boundary(start)
                || !text.is_char_boundary(end)
                || self.face(span.font).is_none()
            {
                return Err(TextError::new(TextErrorCode::InvalidTextStyleSpan));
            }
            while next_grapheme_boundary.is_some_and(|boundary| boundary < end) {
                next_grapheme_boundary = grapheme_boundaries.next();
            }
            if next_grapheme_boundary != Some(end) {
                return Err(TextError::new(TextErrorCode::InvalidTextStyleSpan));
            }
            expected_start = end;
        }
        if expected_start != text.len() {
            return Err(TextError::new(TextErrorCode::InvalidTextStyleSpan));
        }
        Ok(())
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
    font_size_bits: i32,
}

#[derive(Clone, Copy)]
enum ParagraphStyle<'a> {
    Uniform(i32),
    Spans(&'a [TextStyleSpan]),
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
