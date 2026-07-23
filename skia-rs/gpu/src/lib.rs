//! Backend-neutral GPU submission contracts for `skia`.
//!
//! This is a renderer-integration SPI for application composition roots and
//! platform backends. Ordinary rendering code uses the top-level `skia`
//! facade instead of recording GPU commands directly.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod backend;
mod command;
mod encoder;
mod error;
mod limits;
mod resource;
mod surface;

/// Deterministic CPU replay backend for GPU command conformance tests.
#[cfg(feature = "software")]
pub mod software;

pub use backend::{GpuBackend, GpuBackendError};
pub use command::{GpuCommand, GpuCommandBuffer};
pub use encoder::GpuCommandEncoder;
pub use error::{GpuCommandError, GpuCommandErrorCode};
pub use limits::{GpuCapabilities, GpuCommandLimits};
pub use resource::{
    GpuAtlasRect, GpuClipGeometry, GpuClipId, GpuClipNode, GpuGlyphAtlas, GpuGlyphAtlasId,
    GpuGlyphAtlasKey, GpuGlyphQuad, GpuImageId, GpuPathId,
};
pub use surface::{GpuSurfaceDescriptor, GpuSurfaceFormat};
