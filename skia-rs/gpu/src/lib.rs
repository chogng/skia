//! Backend-neutral GPU submission contracts for `skia`.
//!
//! This is a renderer-integration SPI for application composition roots and
//! platform backends. Higher-level rendering adapters normally use
//! `skia-core` or `skia-cpu` instead of recording GPU commands directly.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod backend;
mod command;
mod encoder;
mod error;
mod limits;
mod resource;
mod runtime_shader;
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
pub use runtime_shader::{
    RUNTIME_SHADER_INSTRUCTION_WORDS, RUNTIME_SHADER_MAX_INSTRUCTIONS, RUNTIME_SHADER_MAX_UNIFORMS,
    RuntimeShaderPacket, runtime_shader_packet,
};
pub use surface::{GpuSurfaceDescriptor, GpuSurfaceFormat};
