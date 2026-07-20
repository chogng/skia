//! Backend-neutral drawing semantics and immutable display-list foundations.
//!
//! This crate owns portable paint and display-list semantics.
//! CPU and GPU executors depend on it; it never depends on either executor.
//! The `skia` facade selects the API available to consumers. See
//! `skia/README.md` for the subsystem boundary.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod display_list;
mod paint;

pub use display_list::{DisplayList, DisplayListBuilder, DrawCommand, GlyphRunId, ImageId, PathId};
pub use paint::{BlendMode, Color, Paint};
pub use skia_error::{SkiaError, SkiaErrorCode};
pub use skia_geometry::{Point, Rect, Scalar, Transform};
pub use skia_path::{
    Angle, ArcDirection, ArcStart, ConicWeight, FillRule, Path, PathBounds, PathBuilder, PathVerb,
};
pub use skia_text::{
    FontCollection, FontCollectionLimits, FontFace, FontFeature, FontId, FontLimits, FontMetrics,
    FontSlant, FontStyle, FontTag, FontVariation, FontVariationAxis, FontWidth, GlyphId,
    GlyphOutline, GlyphOutlineProvider, GlyphRun, OutlinePoint, OutlineSegment, PositionedGlyph,
    ShapedLine, ShapedParagraph, ShapedRun, TextAffinity, TextAlignment, TextBreakProvider,
    TextCaret, TextDecoration, TextDecorationMetrics, TextDirection, TextError, TextErrorCode,
    TextHitResult, TextLayout, TextLayoutOptions, TextPosition, TextStyleSpan, TextUnit,
    TextWordBreak, TextWordBreakKind,
};
