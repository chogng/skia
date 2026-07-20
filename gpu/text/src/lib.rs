//! Pure text-to-GPU data adaptation for atlas-backed glyph rendering.
//!
//! This crate converts [`skia_core::TextLayout`] output into a generic
//! [`skia_gpu::GpuGlyphAtlas`], positioned [`skia_gpu::GpuGlyphQuad`] values,
//! and target-space decoration rectangles. It does not borrow command encoders
//! or submit backend work; callers keep resource registration, paint resolution,
//! and draw ordering explicit in `skia-gpu`. A bounded [`TextAtlasCache`] reuses
//! immutable packed atlases across frames.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod atlas;
mod cache;
mod decoration;
mod error;
mod key;
mod layout;

pub use atlas::{TextAtlas, TextAtlasBuilder};
pub use cache::{TextAtlasCache, TextAtlasCacheLimits, TextAtlasCacheStats};
pub use decoration::{TextDecorationBatch, layout_decoration_batches};
pub use error::{TextGpuError, TextGpuErrorCode};
pub use key::{TextAtlasEntry, TextGlyphKey};
pub use layout::TextGlyphBatch;
