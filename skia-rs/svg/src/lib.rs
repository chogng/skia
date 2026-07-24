//! Bounded SVG input and deterministic output for portable display lists.
//!
//! This crate owns SVG viewport, style, local-resource, vector, and embedded
//! image policy; transactionally lowers its supported input profile to a
//! backend-neutral display list; and serializes supported display lists to
//! deterministic UTF-8 SVG. `skia-core` remains independent of formats.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod css;
mod reader;
mod svg;

pub use reader::{
    SvgDocument, SvgReadError, SvgReadErrorCode, SvgReadLimits, SvgReadOptions, SvgReader,
};
pub use svg::{
    SvgCanvasSpec, SvgError, SvgErrorCode, SvgLimits, SvgOptions, SvgPreserveAspectRatio,
    SvgViewBoxAlignment, SvgViewBoxScale, SvgWriter,
};

#[cfg(test)]
#[path = "svg_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "reader_tests.rs"]
mod reader_tests;
