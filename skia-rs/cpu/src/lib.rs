//! Deterministic CPU executor for `skia-core` drawing semantics.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod canvas;
mod clip;
mod stroke;

pub use canvas::{Canvas, ClipRect, Surface, SurfaceLimits};
