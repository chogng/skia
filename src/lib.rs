//! Stable public API for the reusable Skia-like graphics engine.
//!
//! Applications depend only on this facade. Geometry, text, image storage, CPU,
//! GPU, and platform backends remain implementation layers within the Skia
//! workspace.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub use skia_codec::{
    AnimatedImageAsset, AnimationBlend, AnimationDisposal, AnimationFrame, AnimationLimits,
    AnimationLoop, CodecError, CodecErrorCode, CodecLimits, EncodeFormat, EncodeLimits,
    EncodeOptions, EncodeReport, EncodedFormat, EncodedImage, FrameDuration, ImageAsset,
    ImageCodec, ImageMetadata, JpegAlphaHandling, JpegOptimization, JpegOptions, JpegScan,
    JpegSubsampling, MetadataPolicy, PngCompression, PngFilter, PngOptions, WebPMode, WebPOptions,
};
pub use skia_core::{
    BlendMode, BuiltinTextBreakProvider, ClipOp, Color, FontCollection, FontCollectionLimits,
    FontFace, FontFeature, FontId, FontLimits, FontMetrics, FontSlant, FontStyle, FontStyleMatch,
    FontTag, FontVariation, FontVariationAxis, FontWidth, GlyphBitmap, GlyphBitmapFormat, GlyphId,
    GlyphOutline, GlyphOutlineProvider, GlyphRun, OutlinePoint, OutlineSegment, Paint,
    PositionedGlyph, SamplingFilter, SamplingOptions, ShapedLine, ShapedParagraph, ShapedRun,
    StrokeAlign, StrokeCap, StrokeJoin, StrokeOptions, TextAffinity, TextAlignment,
    TextBreakProvider, TextCaret, TextDecoration, TextDecorationMetrics, TextDecorationRect,
    TextDecorationSegment, TextDecorationStyle, TextDirection, TextError, TextErrorCode,
    TextHitResult, TextJustification, TextLayout, TextLayoutOptions, TextOverflow, TextPosition,
    TextSelectionRect, TextStyleId, TextStyleSpan, TextUnit, TextWordBreak, TextWordBreakKind,
    glyph_outline_path, text_decoration_rects,
};
pub use skia_cpu::{Canvas, ClipRect, Surface, SurfaceLimits};
pub use skia_error::{SkiaError, SkiaErrorCode};
pub use skia_geometry::{Point, Rect, Scalar, Transform};
pub use skia_image::{ColorSpace, Image, ImageError, ImageErrorCode};
pub use skia_path::{
    Angle, ArcDirection, ArcStart, ConicWeight, FillRule, Path, PathBounds, PathBuilder, PathVerb,
};
pub use skia_system_fonts::{
    GenericFontFamily, SystemFontCatalog, SystemFontDiscoveryLimits, SystemFontError,
    SystemFontErrorCode, SystemFontRecord, discover_system_fonts,
};
pub use skia_tessellation::{
    ComposePathEffect, CornerPathEffect, DashPathEffect, DiscretePathEffect, PathBooleanLimits,
    PathBooleanOp, PathEffect, PathEffectLimits, SumPathEffect, TrimPathEffect, apply_path_effect,
    compose_path_effects, corner_path, dash_path, discrete_path, path_boolean, stroke_to_path,
    trim_path,
};
