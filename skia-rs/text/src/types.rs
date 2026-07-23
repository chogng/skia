use crate::{TextError, TextErrorCode};

/// Opaque stable identifier of a font selected by a font resolver.
///
/// The identifier is intentionally not a platform handle. A display-list
/// consumer supplies the resolver that maps it to embedded, bundled, or
/// system font data.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FontId(u64);

impl FontId {
    /// Creates an application-defined stable font identifier.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the application-defined font identifier.
    pub const fn value(self) -> u64 {
        self.0
    }
}

/// Identifier of one glyph in a [`FontId`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GlyphId(u32);

impl GlyphId {
    /// Creates a glyph identifier from the font's glyph index.
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the font-local glyph index.
    pub const fn value(self) -> u32 {
        self.0
    }
}

/// Signed Q26.6 coordinate used by a shaped glyph run.
///
/// Q26.6 is the common fixed-point output unit for font shaping. The run's
/// font size converts these values into the canvas coordinate system.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TextUnit(i32);

impl TextUnit {
    /// Exact zero.
    pub const ZERO: Self = Self(0);

    /// Creates an exact whole-number text coordinate.
    pub fn from_i32(value: i32) -> Result<Self, TextError> {
        i32::try_from(i64::from(value) * 64)
            .map(Self)
            .map_err(|_| TextError::new(TextErrorCode::NumericOverflow))
    }

    /// Creates a text coordinate from exact Q26.6 storage.
    pub const fn from_bits(bits: i32) -> Self {
        Self(bits)
    }

    /// Returns the exact Q26.6 storage value.
    pub const fn bits(self) -> i32 {
        self.0
    }
}

/// One positioned glyph produced by a text shaper.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PositionedGlyph {
    glyph: GlyphId,
    cluster: u32,
    x: TextUnit,
    y: TextUnit,
    advance_x: TextUnit,
    advance_y: TextUnit,
}

/// Exact UTF-8 source range that produced a shaped [`GlyphRun`].
///
/// Glyph cluster values are absolute byte offsets into the logical paragraph.
/// `text` is the corresponding non-empty substring, while `offset` identifies
/// its first byte in that paragraph. Render backends can use this information
/// for copy/search semantics without attempting to reverse glyph IDs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GlyphRunSource {
    text: String,
    offset: u32,
}

impl GlyphRunSource {
    /// Creates one non-empty source substring at an absolute UTF-8 byte offset.
    pub fn new(text: String, offset: u32) -> Result<Self, TextError> {
        if text.is_empty()
            || u32::try_from(text.len())
                .ok()
                .and_then(|length| offset.checked_add(length))
                .is_none()
        {
            return Err(TextError::new(TextErrorCode::InvalidLayout));
        }
        Ok(Self { text, offset })
    }

    /// Borrows the exact non-empty UTF-8 substring.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the substring's absolute UTF-8 byte offset.
    pub const fn offset(&self) -> u32 {
        self.offset
    }
}

impl PositionedGlyph {
    /// Creates one positioned glyph and its pen advance.
    pub const fn new(
        glyph: GlyphId,
        x: TextUnit,
        y: TextUnit,
        advance_x: TextUnit,
        advance_y: TextUnit,
    ) -> Self {
        Self {
            glyph,
            cluster: 0,
            x,
            y,
            advance_x,
            advance_y,
        }
    }

    /// Creates one positioned glyph with its source UTF-8 cluster offset.
    pub const fn with_cluster(
        glyph: GlyphId,
        cluster: u32,
        x: TextUnit,
        y: TextUnit,
        advance_x: TextUnit,
        advance_y: TextUnit,
    ) -> Self {
        Self {
            glyph,
            cluster,
            x,
            y,
            advance_x,
            advance_y,
        }
    }

    /// Returns the font-local glyph index.
    pub const fn glyph(self) -> GlyphId {
        self.glyph
    }

    /// Returns the byte offset of this glyph's source grapheme cluster.
    pub const fn cluster(self) -> u32 {
        self.cluster
    }

    /// Returns the glyph's shaped horizontal position.
    pub const fn x(self) -> TextUnit {
        self.x
    }

    /// Returns the glyph's shaped vertical position.
    pub const fn y(self) -> TextUnit {
        self.y
    }

    /// Returns the shaped horizontal pen advance.
    pub const fn advance_x(self) -> TextUnit {
        self.advance_x
    }

    /// Returns the shaped vertical pen advance.
    pub const fn advance_y(self) -> TextUnit {
        self.advance_y
    }
}

/// Immutable shaped glyph run ready for a rendering backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GlyphRun {
    font: FontId,
    font_size_bits: i32,
    units_per_em: u16,
    glyphs: Vec<PositionedGlyph>,
    ligature_carets: Vec<LigatureCaret>,
    source: Option<GlyphRunSource>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LigatureCaret {
    pub(crate) glyph_index: usize,
    pub(crate) source_offset: u32,
    pub(crate) x: TextUnit,
}

impl GlyphRun {
    /// Creates a non-empty run with a positive Q16.16 font size.
    pub fn new(
        font: FontId,
        font_size_bits: i32,
        units_per_em: u16,
        glyphs: Vec<PositionedGlyph>,
    ) -> Result<Self, TextError> {
        if font_size_bits <= 0 {
            return Err(TextError::new(TextErrorCode::InvalidFontSize));
        }
        if units_per_em == 0 {
            return Err(TextError::new(TextErrorCode::InvalidUnitsPerEm));
        }
        if glyphs.is_empty() {
            return Err(TextError::new(TextErrorCode::EmptyGlyphRun));
        }
        Ok(Self {
            font,
            font_size_bits,
            units_per_em,
            glyphs,
            ligature_carets: Vec::new(),
            source: None,
        })
    }

    /// Creates one shaped glyph run with its exact logical UTF-8 source range.
    pub fn new_with_source(
        font: FontId,
        font_size_bits: i32,
        units_per_em: u16,
        glyphs: Vec<PositionedGlyph>,
        source: GlyphRunSource,
    ) -> Result<Self, TextError> {
        let run = Self::new(font, font_size_bits, units_per_em, glyphs)?;
        validate_glyph_source(&run.glyphs, &source)?;
        Ok(Self {
            source: Some(source),
            ..run
        })
    }

    pub(crate) fn with_ligature_carets(
        font: FontId,
        font_size_bits: i32,
        units_per_em: u16,
        glyphs: Vec<PositionedGlyph>,
        ligature_carets: Vec<LigatureCaret>,
        source: Option<GlyphRunSource>,
    ) -> Result<Self, TextError> {
        let run = Self::new(font, font_size_bits, units_per_em, glyphs)?;
        if ligature_carets
            .iter()
            .any(|caret| caret.glyph_index >= run.glyphs.len())
        {
            return Err(TextError::new(TextErrorCode::InvalidLayout));
        }
        if let Some(source) = &source {
            validate_glyph_source(&run.glyphs, source)?;
        }
        Ok(Self {
            ligature_carets,
            source,
            ..run
        })
    }

    /// Returns the font selected for this run.
    pub const fn font(&self) -> FontId {
        self.font
    }

    /// Returns the positive Q16.16 point size used by this run.
    pub const fn font_size_bits(&self) -> i32 {
        self.font_size_bits
    }

    /// Returns the font design-unit scale used by glyph positions and outlines.
    pub const fn units_per_em(&self) -> u16 {
        self.units_per_em
    }

    /// Borrows shaped glyphs in visual drawing order.
    pub fn glyphs(&self) -> &[PositionedGlyph] {
        &self.glyphs
    }

    /// Returns the original UTF-8 source range when the run came from this
    /// crate's shaper or was constructed with [`Self::new_with_source`].
    pub fn source(&self) -> Option<&GlyphRunSource> {
        self.source.as_ref()
    }

    /// Returns the exact logical UTF-8 cluster represented by one glyph.
    ///
    /// The result uses logical cluster order rather than visual glyph order,
    /// so it remains correct for right-to-left runs. Multiple glyphs may
    /// return the same string when shaping maps one source cluster to several
    /// glyphs; document backends can preserve the whole run with an
    /// accessibility replacement in that case.
    pub fn source_text_for_glyph(&self, glyph_index: usize) -> Option<&str> {
        let source = self.source.as_ref()?;
        let cluster = self.glyphs.get(glyph_index)?.cluster;
        let source_end = source
            .offset
            .checked_add(u32::try_from(source.text.len()).ok()?)?;
        let end_cluster = self
            .glyphs
            .iter()
            .map(|glyph| glyph.cluster)
            .filter(|candidate| *candidate > cluster)
            .min()
            .unwrap_or(source_end);
        let start = usize::try_from(cluster.checked_sub(source.offset)?).ok()?;
        let end = usize::try_from(end_cluster.checked_sub(source.offset)?).ok()?;
        source.text.get(start..end)
    }

    pub(crate) fn ligature_carets(&self) -> &[LigatureCaret] {
        &self.ligature_carets
    }
}

fn validate_glyph_source(
    glyphs: &[PositionedGlyph],
    source: &GlyphRunSource,
) -> Result<(), TextError> {
    let length = u32::try_from(source.text.len())
        .map_err(|_| TextError::new(TextErrorCode::NumericOverflow))?;
    let end = source
        .offset
        .checked_add(length)
        .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
    for glyph in glyphs {
        if glyph.cluster < source.offset || glyph.cluster >= end {
            return Err(TextError::new(TextErrorCode::InvalidLayout));
        }
        let relative = usize::try_from(glyph.cluster - source.offset)
            .map_err(|_| TextError::new(TextErrorCode::NumericOverflow))?;
        if !source.text.is_char_boundary(relative) {
            return Err(TextError::new(TextErrorCode::InvalidLayout));
        }
    }
    Ok(())
}
