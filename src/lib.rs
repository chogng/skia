//! Stable public API for the reusable Skia-like graphics engine.
//!
//! Applications depend only on this facade. Geometry, text, image storage, CPU,
//! GPU, and platform backends remain implementation layers within the Skia
//! workspace.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub use skia_codec::{CodecError, CodecErrorCode, CodecLimits, ImageCodec};
pub use skia_core::{
    BlendMode, Color, FontId, GlyphId, GlyphOutline, GlyphOutlineProvider, GlyphRun, OutlinePoint,
    OutlineSegment, Paint, PositionedGlyph, TextError, TextErrorCode, TextUnit,
};
pub use skia_cpu::{Canvas, ClipRect, Surface, SurfaceLimits};
pub use skia_error::{SkiaError, SkiaErrorCode};
pub use skia_geometry::{Point, Rect, Scalar, Transform};
pub use skia_image::{Image, ImageError, ImageErrorCode};
pub use skia_path::{
    Angle, ArcDirection, ArcStart, ConicWeight, FillRule, Path, PathBounds, PathBuilder, PathVerb,
};
