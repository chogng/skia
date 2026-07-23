//! Bounded PDF 1.7 output for portable display lists.
//!
//! The crate owns PDF lifecycle, page policy, output limits, object writing,
//! an explicit CPU fallback, archival metadata, navigation, tagged content,
//! isolated transparency groups, and outline or embedded-font text. It depends
//! on backend-neutral drawing contracts while `skia-core` remains independent
//! of output formats.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod pdf;

pub use pdf::{
    PageSize, PageSpec, PdfColorPolicy, PdfConformance, PdfDateTime, PdfDocument, PdfEmbeddedFont,
    PdfError, PdfErrorCode, PdfLimits, PdfLinkTarget, PdfMetadata, PdfOptions, PdfStructureTag,
    PdfTextProvider, RasterFallback, UnsupportedBehavior,
};

#[cfg(test)]
#[path = "pdf_tests.rs"]
mod tests;
