//! Reusable GPU resource fixtures for backend integration tests.
//!
//! The utilities deliberately use backend-neutral images and surfaces. Native
//! texture allocation belongs to each backend, where command-buffer ownership
//! and destruction can be synchronized correctly.

mod compressed;
mod context;
mod image;
mod submission;
mod surface;

pub use compressed::two_color_bc1_compress;
pub use context::GpuContextType;
pub use image::{BackendTextureImageFactory, ManagedImage, TestGpuResourceError};
pub use submission::SubmissionTracker;
pub use surface::BackendSurfaceFactory;

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
