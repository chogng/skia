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
        })
    }

    pub(crate) fn with_ligature_carets(
        font: FontId,
        font_size_bits: i32,
        units_per_em: u16,
        glyphs: Vec<PositionedGlyph>,
        ligature_carets: Vec<LigatureCaret>,
    ) -> Result<Self, TextError> {
        let run = Self::new(font, font_size_bits, units_per_em, glyphs)?;
        if ligature_carets
            .iter()
            .any(|caret| caret.glyph_index >= run.glyphs.len())
        {
            return Err(TextError::new(TextErrorCode::InvalidLayout));
        }
        Ok(Self {
            ligature_carets,
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

    pub(crate) fn ligature_carets(&self) -> &[LigatureCaret] {
        &self.ligature_carets
    }
}
