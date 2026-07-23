//! Cross-platform Vulkan submission backend for `skia-gpu`.
//!
//! The backend owns a real dynamically loaded Vulkan instance, device, queue,
//! and offscreen RGBA8 storage target. Portable draw commands execute through
//! a Vulkan compute pipeline; CPU work is limited to command interpretation,
//! geometry expansion, and immutable resource upload.

#![deny(missing_docs)]
#![deny(unsafe_op_in_unsafe_fn)]

mod backend;
mod commands;
mod context;
mod error;
mod renderer;
mod surface;

pub use backend::VulkanBackend;
pub use error::{VulkanError, VulkanErrorCode};
pub use surface::VulkanSurface;
