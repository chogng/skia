//! Pure text-to-GPU data adaptation for atlas-backed glyph rendering.
//!
//! This crate is part of the renderer-integration SPI. Application composition
//! roots use it when preparing text for a selected GPU executor; ordinary
//! rendering code remains on the top-level `skia` facade.
//!
//! This crate converts [`skia_core::TextLayout`] output into target-space vector
//! outline paths or a generic [`skia_gpu::GpuGlyphAtlas`] with positioned
//! [`skia_gpu::GpuGlyphQuad`] values, plus target-space decoration rectangles.
//! It does not borrow command encoders or submit backend work; callers keep
//! resource registration, paint resolution, and draw ordering explicit in
//! `skia-gpu`. A bounded [`TextAtlasCache`] reuses immutable packed atlases
//! across frames.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod atlas;
mod cache;
mod decoration;
mod error;
mod key;
mod layout;
mod outline;

pub use atlas::{TextAtlas, TextAtlasBuilder};
pub use cache::{TextAtlasCache, TextAtlasCacheLimits, TextAtlasCacheStats};
pub use decoration::{TextDecorationBatch, layout_decoration_batches};
pub use error::{TextGpuError, TextGpuErrorCode};
pub use key::{TextAtlasEntry, TextGlyphKey};
pub use layout::TextGlyphBatch;
pub use outline::{TextOutlineBatch, layout_outline_batches};
