//! Portable font, shaping, layout, and text-editing resources.
//!
//! This crate deliberately contains neither a system-font dependency nor a
//! platform text API. It owns stable glyph/outline contracts plus portable
//! font parsing, fallback, Unicode shaping, multiline layout, and editing
//! geometry without coupling render backends to platform font handles.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod break_provider;
mod collection;
mod decoration;
mod error;
mod font;
mod layout;
mod line_break;
mod outline;
mod typeface;
mod types;

pub use break_provider::BuiltinTextBreakProvider;
pub use collection::{
    FontCollection, FontCollectionLimits, ShapedParagraph, ShapedRun, TextDecoration,
    TextDirection, TextStyleId, TextStyleSpan,
};
pub use decoration::{TextDecorationRect, TextDecorationStyle, text_decoration_rects};
pub use error::{TextError, TextErrorCode};
pub use font::{
    FontEmbeddingPermission, FontEmbeddingRights, FontFace, FontFeature, FontLimits, FontMetrics,
    FontProgramFormat, FontSlant, FontStyle, FontStyleMatch, FontTag, FontVariation,
    FontVariationAxis, FontWidth, GlyphBitmap, GlyphBitmapFormat, PortableFontProgram,
    TextDecorationMetrics,
};
pub use layout::{
    ShapedLine, TextAffinity, TextAlignment, TextBreakProvider, TextCaret, TextDecorationSegment,
    TextHitResult, TextJustification, TextLayout, TextLayoutOptions, TextOverflow, TextPosition,
    TextSelectionRect, TextWordBreak, TextWordBreakKind,
};
pub use outline::{GlyphOutline, GlyphOutlineProvider, OutlinePoint, OutlineSegment};
pub use typeface::Typeface;
pub(crate) use types::LigatureCaret;
pub use types::{FontId, GlyphId, GlyphRun, GlyphRunSource, PositionedGlyph, TextUnit};

pub(crate) fn valid_language_tag(language: &str) -> bool {
    !language.is_empty()
        && language.len() <= 255
        && language
            .split('-')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_alphanumeric()))
}
