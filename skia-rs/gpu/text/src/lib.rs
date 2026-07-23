//! Pure text-to-GPU data adaptation for atlas-backed glyph rendering.
//!
//! This crate is part of the renderer-integration SPI. Application composition
//! roots use it when preparing text for a selected GPU executor; higher-level
//! rendering adapters remain on the portable responsibility crates.
//!
//! This crate converts [`skia_core::TextLayout`] output into a generic
//! [`skia_gpu::GpuGlyphAtlas`] with positioned [`skia_gpu::GpuGlyphQuad`]
//! values. It does not borrow command encoders or submit backend work; callers
//! keep resource registration, paint resolution, and draw ordering explicit in
//! `skia-gpu`. A bounded [`TextAtlasCache`] reuses immutable packed atlases
//! across frames. Portable text outline and decoration geometry lives in
//! `skia-core`.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod atlas;
mod cache;
mod error;
mod key;
mod layout;

pub use atlas::{TextAtlas, TextAtlasBuilder};
pub use cache::{TextAtlasCache, TextAtlasCacheLimits, TextAtlasCacheStats};
pub use error::{TextGpuError, TextGpuErrorCode};
pub use key::{TextAtlasEntry, TextGlyphKey};
pub use layout::TextGlyphBatch;
