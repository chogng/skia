use skia_core::{FontId, GlyphBitmap, GlyphBitmapFormat, GlyphId};
use skia_gpu::GpuAtlasRect;

/// Stable identity of one rasterized glyph inside a text atlas.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TextGlyphKey {
    font: FontId,
    glyph: GlyphId,
    font_size_bits: i32,
    format: GlyphBitmapFormat,
}

impl TextGlyphKey {
    /// Creates a cache key from one validated glyph bitmap.
    pub const fn from_bitmap(bitmap: &GlyphBitmap) -> Self {
        Self {
            font: bitmap.font(),
            glyph: bitmap.glyph(),
            font_size_bits: bitmap.font_size_bits(),
            format: bitmap.format(),
        }
    }

    pub(crate) const fn new(
        font: FontId,
        glyph: GlyphId,
        font_size_bits: i32,
        format: GlyphBitmapFormat,
    ) -> Self {
        Self {
            font,
            glyph,
            font_size_bits,
            format,
        }
    }

    /// Returns the immutable font-instance identity.
    pub const fn font(self) -> FontId {
        self.font
    }

    /// Returns the font-local glyph identity.
    pub const fn glyph(self) -> GlyphId {
        self.glyph
    }

    /// Returns the Q16.16 raster size.
    pub const fn font_size_bits(self) -> i32 {
        self.font_size_bits
    }

    /// Returns the atlas sample interpretation.
    pub const fn format(self) -> GlyphBitmapFormat {
        self.format
    }
}

/// Packed placement metadata for one text glyph bitmap.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TextAtlasEntry {
    pub(crate) key: TextGlyphKey,
    pub(crate) source: GpuAtlasRect,
    pub(crate) left: i32,
    pub(crate) top: i32,
}

impl TextAtlasEntry {
    /// Returns the text glyph cache key.
    pub const fn key(self) -> TextGlyphKey {
        self.key
    }

    /// Returns the atlas pixel rectangle.
    pub const fn source(self) -> GpuAtlasRect {
        self.source
    }

    /// Returns the raster placement offset right from the glyph origin.
    pub const fn left(self) -> i32 {
        self.left
    }

    /// Returns the raster placement offset above the glyph baseline.
    pub const fn top(self) -> i32 {
        self.top
    }
}
