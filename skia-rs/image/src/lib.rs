//! Immutable, owned RGBA8 image resources.
//!
//! This crate has no dependency on the drawing core or a rendering backend, so
//! image decoding and storage remain usable independently of display lists.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod image;

pub use image::{ColorSpace, Image, ImageError, ImageErrorCode};
