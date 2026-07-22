//! Backend-neutral GPU submission contracts for `skia`.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod command;

/// Deterministic CPU replay backend for GPU command conformance tests.
#[cfg(feature = "software")]
pub mod software;

pub use command::{
    GpuAtlasRect, GpuBackend, GpuBackendError, GpuClipGeometry, GpuClipId, GpuClipNode, GpuCommand,
    GpuCommandBuffer, GpuCommandEncoder, GpuCommandError, GpuCommandErrorCode, GpuCommandLimits,
    GpuGlyphAtlas, GpuGlyphAtlasId, GpuGlyphAtlasKey, GpuGlyphQuad, GpuImageId, GpuPathId,
    GpuSurfaceDescriptor,
};
