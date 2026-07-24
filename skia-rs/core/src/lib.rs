//! Backend-neutral drawing semantics and immutable display-list foundations.
//!
//! This crate owns portable paint and display-list semantics.
//! CPU and GPU executors depend on it; it never depends on either executor.
//! Consumers depend on this crate directly when they need these contracts. See
//! the workspace `README.md` for the subsystem boundary.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod clip;
mod display_list;
#[cfg(test)]
#[path = "effect_handle_tests.rs"]
mod effect_handle_tests;
mod paint;
mod path_effect;
#[cfg(test)]
#[path = "path_effect_tests.rs"]
mod path_effect_tests;
#[cfg(test)]
#[path = "runtime_shader_tests.rs"]
mod runtime_shader_tests;
mod sampling;
#[cfg(test)]
#[path = "shader_graph_tests.rs"]
mod shader_graph_tests;
mod shaders;
#[cfg(feature = "text")]
mod text_geometry;
#[cfg(feature = "text")]
mod text_path;

pub use clip::ClipOp;
#[cfg(feature = "text")]
pub use display_list::GlyphRunId;
pub use display_list::{DisplayList, DisplayListBuilder, DrawCommand, ImageId, PathId};
pub use paint::{
    BlendMode, Color, ColorFilter, ColorFilterHandle, ColorMatrix, ImageFilter, ImageFilterHandle,
    Paint, SaveLayerOptions,
};
pub use path_effect::{
    PathEffect, PathEffectHandle, PathEffectLimits, apply_path_effect, compose_path_effects,
};
pub use sampling::{SamplingFilter, SamplingOptions};
pub use shaders::{
    BlendShader, Gradient, GradientGeometry, GradientStop, ImageShader, LocalMatrixShader,
    RuntimeShader, RuntimeShaderInstruction, RuntimeShaderLimits, RuntimeShaderProgram, Shader,
    ShaderHandle, TileMode,
};
pub use skia_error::{SkiaError, SkiaErrorCode};
pub use skia_geometry::{Point, Rect, Scalar, Transform};
pub use skia_path::{
    Angle, ArcDirection, ArcStart, ConicWeight, FillRule, Path, PathBounds, PathBuilder, PathVerb,
    StrokeAlign, StrokeCap, StrokeJoin, StrokeOptions,
};
#[cfg(feature = "text")]
pub use skia_text::{
    BuiltinTextBreakProvider, FontCollection, FontCollectionLimits, FontEmbeddingPermission,
    FontEmbeddingRights, FontFace, FontFeature, FontId, FontLimits, FontMetrics, FontProgramFormat,
    FontSlant, FontStyle, FontStyleMatch, FontTag, FontVariation, FontVariationAxis, FontWidth,
    GlyphBitmap, GlyphBitmapFormat, GlyphId, GlyphOutline, GlyphOutlineProvider, GlyphRun,
    GlyphRunSource, OutlinePoint, OutlineSegment, PortableFontProgram, PositionedGlyph, ShapedLine,
    ShapedParagraph, ShapedRun, TextAffinity, TextAlignment, TextBreakProvider, TextCaret,
    TextDecoration, TextDecorationMetrics, TextDecorationRect, TextDecorationSegment,
    TextDecorationStyle, TextDirection, TextError, TextErrorCode, TextHitResult, TextJustification,
    TextLayout, TextLayoutOptions, TextOverflow, TextPosition, TextSelectionRect, TextStyleId,
    TextStyleSpan, TextUnit, TextWordBreak, TextWordBreakKind, text_decoration_rects,
};
#[cfg(feature = "text")]
pub use text_geometry::{
    TextDecorationBatch, TextLayoutEvent, TextOutlineBatch, layout_decoration_batches,
    layout_outline_batches, text_layout_events, text_layout_glyph_events,
};
#[cfg(feature = "text")]
pub use text_path::glyph_outline_path;
