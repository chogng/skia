use crate::{FontId, GlyphId, TextError, TextErrorCode, TextUnit};

/// One exact point in canvas-oriented font design coordinates.
///
/// Positive Y points downward, matching Skia canvas coordinates. A font parser
/// whose source coordinates point upward performs that inversion in its
/// adapter, not in a renderer.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct OutlinePoint {
    x: TextUnit,
    y: TextUnit,
}

impl OutlinePoint {
    /// Creates one exact outline point.
    pub const fn new(x: TextUnit, y: TextUnit) -> Self {
        Self { x, y }
    }

    /// Returns the horizontal design coordinate.
    pub const fn x(self) -> TextUnit {
        self.x
    }

    /// Returns the vertical design coordinate.
    pub const fn y(self) -> TextUnit {
        self.y
    }
}

/// One Bézier operation in a glyph outline.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum OutlineSegment {
    /// Starts one contour.
    MoveTo(OutlinePoint),
    /// Adds a straight segment.
    LineTo(OutlinePoint),
    /// Adds a quadratic Bézier segment.
    QuadTo {
        /// Quadratic control point.
        control: OutlinePoint,
        /// Segment endpoint.
        end: OutlinePoint,
    },
    /// Adds a cubic Bézier segment.
    CubicTo {
        /// First cubic control point.
        first_control: OutlinePoint,
        /// Second cubic control point.
        second_control: OutlinePoint,
        /// Segment endpoint.
        end: OutlinePoint,
    },
    /// Closes the active contour.
    Close,
}

/// Immutable, validated outline for one font-local glyph.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GlyphOutline {
    font: FontId,
    glyph: GlyphId,
    segments: Vec<OutlineSegment>,
}

impl GlyphOutline {
    /// Creates an empty or fully closed glyph outline.
    pub fn new(
        font: FontId,
        glyph: GlyphId,
        segments: Vec<OutlineSegment>,
    ) -> Result<Self, TextError> {
        let mut contour_active = false;
        for segment in &segments {
            match segment {
                OutlineSegment::MoveTo(_) => {
                    if contour_active {
                        return Err(TextError::new(TextErrorCode::InvalidOutline));
                    }
                    contour_active = true;
                }
                OutlineSegment::LineTo(_)
                | OutlineSegment::QuadTo { .. }
                | OutlineSegment::CubicTo { .. } => {
                    if !contour_active {
                        return Err(TextError::new(TextErrorCode::InvalidOutline));
                    }
                }
                OutlineSegment::Close => {
                    if !contour_active {
                        return Err(TextError::new(TextErrorCode::InvalidOutline));
                    }
                    contour_active = false;
                }
            }
        }
        if contour_active {
            return Err(TextError::new(TextErrorCode::InvalidOutline));
        }
        Ok(Self {
            font,
            glyph,
            segments,
        })
    }

    /// Returns the selected font.
    pub const fn font(&self) -> FontId {
        self.font
    }

    /// Returns the font-local glyph index.
    pub const fn glyph(&self) -> GlyphId {
        self.glyph
    }

    /// Borrows closed outline segments in drawing order.
    pub fn segments(&self) -> &[OutlineSegment] {
        &self.segments
    }
}

/// Resolves font-local glyphs into portable Bézier outlines.
///
/// Implementations may use embedded font data, a bundled font collection, or
/// a system font service. Missing glyphs return `Ok(None)` so callers can use
/// deterministic fallback without treating ordinary fallback as an error.
pub trait GlyphOutlineProvider {
    /// Resolves a selected font-local glyph.
    fn glyph_outline(
        &self,
        font: FontId,
        glyph: GlyphId,
    ) -> Result<Option<GlyphOutline>, TextError>;
}
