//! Metal execution backend for the backend-neutral `skia-gpu` contract.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod backend;

pub use backend::{
    MetalAtlasCacheStats, MetalBackend, MetalError, MetalErrorCode,
    MetalRuntimeShaderPipelineCacheStats, MetalSurface,
};
