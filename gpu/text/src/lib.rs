//! Pure text-to-GPU data adaptation for atlas-backed glyph rendering.
//!
//! This crate converts [`skia_core::TextLayout`] output into a generic
//! [`skia_gpu::GpuGlyphAtlas`] and positioned [`skia_gpu::GpuGlyphQuad`] values.
//! It does not borrow command encoders or submit backend work; callers keep
//! resource registration and draw ordering explicit in `skia-gpu`.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod atlas;
mod error;
mod key;
mod layout;

pub use atlas::{TextAtlas, TextAtlasBuilder};
pub use error::{TextGpuError, TextGpuErrorCode};
pub use key::{TextAtlasEntry, TextGlyphKey};
