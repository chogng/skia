//! Deterministic XPS 1.0 and OpenXPS output for portable display lists.
//!
//! The crate owns fixed-document lifecycle, native fixed-page mapping, bounded
//! raster fallback, and OPC package serialization. It is platform-independent
//! and does not depend on the Windows XPS Object Model.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod opc;
mod xps;

pub use xps::{
    RasterFallback, UnsupportedBehavior, XpsDocument, XpsError, XpsErrorCode, XpsFormat, XpsLimits,
    XpsOptions, XpsPageSize, XpsPageSpec,
};

#[cfg(test)]
#[path = "xps_tests.rs"]
mod tests;
