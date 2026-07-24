use std::{fmt, sync::Arc};

use crate::{
    FontFace, FontId, FontMetrics, FontStyle, GlyphBitmap, GlyphId, GlyphOutline,
    GlyphOutlineProvider, GlyphRun, TextDecorationMetrics, TextDirection, TextError,
};

/// Cloneable production handle for one font implementation.
///
/// A typeface supplies nominal character coverage, shaping, metrics, outlines,
/// and optional glyph rasterization. The built-in implementation wraps a
/// portable [`FontFace`]. The backend contract remains private so it can evolve
/// without committing the public API to third-party implementations.
#[derive(Clone)]
pub struct Typeface {
    backend: Arc<dyn TypefaceBackend>,
}

impl Typeface {
    /// Wraps one parsed SFNT face as a production typeface.
    pub fn from_font_face(face: FontFace) -> Self {
        Self {
            backend: Arc::new(face),
        }
    }

    /// Returns the caller-defined stable font identifier.
    pub fn id(&self) -> FontId {
        self.backend.id()
    }

    /// Returns the preferred family name when one is available.
    pub fn family_name(&self) -> Option<&str> {
        self.backend.family_name()
    }

    pub(crate) fn matches_family(&self, family: &str) -> bool {
        self.backend.matches_family(family)
    }

    /// Returns the face's CSS-compatible style.
    pub fn style(&self) -> FontStyle {
        self.backend.style()
    }

    /// Returns the underlying parsed SFNT face when this typeface owns one.
    ///
    /// Synthetic and platform-backed typefaces may return `None`.
    pub fn font_face(&self) -> Option<&FontFace> {
        self.backend.font_face()
    }

    /// Resolves one Unicode scalar to its nominal font-local glyph.
    pub fn glyph_for_character(&self, character: char) -> Result<Option<GlyphId>, TextError> {
        self.backend.glyph_for_character(character)
    }

    /// Returns whether this typeface nominally covers one Unicode scalar.
    pub fn supports_character(&self, character: char) -> Result<bool, TextError> {
        self.glyph_for_character(character)
            .map(|glyph| glyph.is_some())
    }

    /// Rasterizes one glyph when the backend provides a bitmap path.
    pub fn rasterize_glyph(
        &self,
        glyph: GlyphId,
        font_size_bits: i32,
    ) -> Result<Option<GlyphBitmap>, TextError> {
        self.backend.rasterize_glyph(glyph, font_size_bits)
    }

    /// Returns baseline metrics scaled to one positive Q16.16 font size.
    pub fn metrics(&self, font_size_bits: i32) -> Result<FontMetrics, TextError> {
        self.backend.metrics(font_size_bits)
    }

    /// Returns scaled underline metrics when the backend provides them.
    pub fn underline_metrics(
        &self,
        font_size_bits: i32,
    ) -> Result<Option<TextDecorationMetrics>, TextError> {
        self.backend.underline_metrics(font_size_bits)
    }

    /// Returns scaled strike-through metrics when the backend provides them.
    pub fn strike_through_metrics(
        &self,
        font_size_bits: i32,
    ) -> Result<Option<TextDecorationMetrics>, TextError> {
        self.backend.strike_through_metrics(font_size_bits)
    }

    /// Shapes one non-empty UTF-8 segment using backend direction detection.
    pub fn shape(&self, text: &str, font_size_bits: i32) -> Result<GlyphRun, TextError> {
        self.shape_segment(text, font_size_bits, None, 0, None)
    }

    /// Shapes one segment with a BCP 47-style language.
    pub fn shape_with_language(
        &self,
        text: &str,
        font_size_bits: i32,
        language: &str,
    ) -> Result<GlyphRun, TextError> {
        self.shape_segment(text, font_size_bits, None, 0, Some(language))
    }

    /// Shapes one horizontal segment with an explicit direction.
    pub fn shape_with_direction(
        &self,
        text: &str,
        font_size_bits: i32,
        direction: TextDirection,
    ) -> Result<GlyphRun, TextError> {
        self.shape_segment(text, font_size_bits, Some(direction), 0, None)
    }

    /// Shapes one segment with explicit direction and language.
    pub fn shape_with_direction_and_language(
        &self,
        text: &str,
        font_size_bits: i32,
        direction: TextDirection,
        language: &str,
    ) -> Result<GlyphRun, TextError> {
        self.shape_segment(text, font_size_bits, Some(direction), 0, Some(language))
    }

    pub(crate) fn shape_segment(
        &self,
        text: &str,
        font_size_bits: i32,
        direction: Option<TextDirection>,
        cluster_offset: u32,
        language: Option<&str>,
    ) -> Result<GlyphRun, TextError> {
        self.backend
            .shape_segment(text, font_size_bits, direction, cluster_offset, language)
    }

    #[cfg(test)]
    pub(crate) fn from_backend(backend: impl TypefaceBackend) -> Self {
        Self {
            backend: Arc::new(backend),
        }
    }
}

impl From<FontFace> for Typeface {
    fn from(face: FontFace) -> Self {
        Self::from_font_face(face)
    }
}

impl fmt::Debug for Typeface {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Typeface")
            .field("id", &self.id())
            .field("family_name", &self.family_name())
            .field("style", &self.style())
            .field("sfnt", &self.font_face().is_some())
            .finish()
    }
}

impl GlyphOutlineProvider for Typeface {
    fn glyph_outline(
        &self,
        font: FontId,
        glyph: GlyphId,
    ) -> Result<Option<GlyphOutline>, TextError> {
        if font != self.id() {
            return Ok(None);
        }
        self.backend.glyph_outline(glyph)
    }
}

pub(crate) trait TypefaceBackend: fmt::Debug + Send + Sync + 'static {
    fn id(&self) -> FontId;
    fn family_name(&self) -> Option<&str>;
    fn matches_family(&self, family: &str) -> bool {
        self.family_name()
            .is_some_and(|name| name.eq_ignore_ascii_case(family))
    }
    fn style(&self) -> FontStyle;
    fn font_face(&self) -> Option<&FontFace> {
        None
    }
    fn glyph_for_character(&self, character: char) -> Result<Option<GlyphId>, TextError>;
    fn rasterize_glyph(
        &self,
        glyph: GlyphId,
        font_size_bits: i32,
    ) -> Result<Option<GlyphBitmap>, TextError>;
    fn metrics(&self, font_size_bits: i32) -> Result<FontMetrics, TextError>;
    fn underline_metrics(
        &self,
        font_size_bits: i32,
    ) -> Result<Option<TextDecorationMetrics>, TextError>;
    fn strike_through_metrics(
        &self,
        font_size_bits: i32,
    ) -> Result<Option<TextDecorationMetrics>, TextError>;
    fn shape_segment(
        &self,
        text: &str,
        font_size_bits: i32,
        direction: Option<TextDirection>,
        cluster_offset: u32,
        language: Option<&str>,
    ) -> Result<GlyphRun, TextError>;
    fn glyph_outline(&self, glyph: GlyphId) -> Result<Option<GlyphOutline>, TextError>;
}

impl TypefaceBackend for FontFace {
    fn id(&self) -> FontId {
        self.id()
    }

    fn family_name(&self) -> Option<&str> {
        self.family_name()
    }

    fn style(&self) -> FontStyle {
        self.style()
    }

    fn font_face(&self) -> Option<&FontFace> {
        Some(self)
    }

    fn glyph_for_character(&self, character: char) -> Result<Option<GlyphId>, TextError> {
        self.glyph_for_character(character)
    }

    fn rasterize_glyph(
        &self,
        glyph: GlyphId,
        font_size_bits: i32,
    ) -> Result<Option<GlyphBitmap>, TextError> {
        self.rasterize_glyph(glyph, font_size_bits)
    }

    fn metrics(&self, font_size_bits: i32) -> Result<FontMetrics, TextError> {
        self.metrics(font_size_bits)
    }

    fn underline_metrics(
        &self,
        font_size_bits: i32,
    ) -> Result<Option<TextDecorationMetrics>, TextError> {
        self.underline_metrics(font_size_bits)
    }

    fn strike_through_metrics(
        &self,
        font_size_bits: i32,
    ) -> Result<Option<TextDecorationMetrics>, TextError> {
        self.strike_through_metrics(font_size_bits)
    }

    fn shape_segment(
        &self,
        text: &str,
        font_size_bits: i32,
        direction: Option<TextDirection>,
        cluster_offset: u32,
        language: Option<&str>,
    ) -> Result<GlyphRun, TextError> {
        self.shape_segment(text, font_size_bits, direction, cluster_offset, language)
    }

    fn glyph_outline(&self, glyph: GlyphId) -> Result<Option<GlyphOutline>, TextError> {
        GlyphOutlineProvider::glyph_outline(self, self.id(), glyph)
    }
}

#[cfg(test)]
#[path = "typeface_tests.rs"]
mod tests;
