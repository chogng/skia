//! Deterministic single-canvas SVG output for portable display lists.
//!
//! This crate owns SVG document policy, native vector mapping, embedded image
//! resources, and bounded serialization. It depends on backend-neutral drawing
//! contracts while `skia-core` remains independent of output formats.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod svg;

pub use svg::{SvgCanvasSpec, SvgError, SvgErrorCode, SvgLimits, SvgOptions, SvgWriter};

#[cfg(test)]
#[path = "svg_tests.rs"]
mod tests;
