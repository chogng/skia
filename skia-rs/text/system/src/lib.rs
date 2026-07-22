//! Platform font-directory discovery for `skia-text`.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod catalog;

pub use catalog::{
    GenericFontFamily, SystemFontCatalog, SystemFontDiscoveryLimits, SystemFontError,
    SystemFontErrorCode, SystemFontRecord, discover_system_fonts,
};
