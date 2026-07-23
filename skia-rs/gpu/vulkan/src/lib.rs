//! Cross-platform Vulkan submission backend for `skia-gpu`.
//!
//! The backend owns a real dynamically loaded Vulkan instance, device, queue,
//! and offscreen RGBA8 storage target. Portable draw commands execute through
//! a Vulkan compute pipeline; CPU work is limited to command interpretation,
//! geometry expansion, and immutable resource upload.

#![deny(missing_docs)]
#![deny(unsafe_op_in_unsafe_fn)]

mod commands;
mod context;
mod renderer;
mod surface;

use std::fmt;

use skia_gpu::{GpuBackend, GpuCapabilities, GpuCommandBuffer, GpuSurfaceDescriptor};

use crate::context::VulkanContext;
use crate::renderer::VulkanRenderer;
pub use crate::surface::VulkanSurface;

/// Stable machine-readable Vulkan backend failure.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum VulkanErrorCode {
    /// No Vulkan loader could be opened on this machine.
    LoaderUnavailable,
    /// Vulkan instance creation failed.
    InstanceCreationFailed,
    /// Validation was required but `VK_LAYER_KHRONOS_validation` is unavailable.
    ValidationUnavailable,
    /// No physical device with a compute-capable queue was available.
    DeviceUnavailable,
    /// Logical-device, queue, or command-pool creation failed.
    DeviceCreationFailed,
    /// Offscreen target or device-memory allocation failed.
    SurfaceAllocationFailed,
    /// The generated SPIR-V shader module could not be loaded.
    ShaderModuleFailed,
    /// The Vulkan compute pipeline or its descriptor layout could not be created.
    PipelineCreationFailed,
    /// The command buffer contains an invalid or unsupported command.
    UnsupportedCommand,
    /// Host-visible staging allocation, mapping, or device upload failed.
    UploadFailed,
    /// Command recording, queue submission, or synchronization failed.
    SubmissionFailed,
    /// Staging allocation, buffer copy, mapping, or readback failed.
    ReadbackFailed,
}

/// Source-redacted Vulkan backend error.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct VulkanError {
    code: VulkanErrorCode,
}

impl VulkanError {
    pub(crate) const fn new(code: VulkanErrorCode) -> Self {
        Self { code }
    }

    /// Returns the stable failure code.
    pub const fn code(self) -> VulkanErrorCode {
        self.code
    }
}

impl fmt::Display for VulkanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}", self.code)
    }
}

impl std::error::Error for VulkanError {}

/// Dynamically loaded Vulkan instance, device, and graphics queue.
pub struct VulkanBackend {
    context: std::sync::Arc<VulkanContext>,
    renderer: VulkanRenderer,
}

impl VulkanBackend {
    /// Opens the system Vulkan loader and selects one compute-capable device.
    pub fn new() -> Result<Self, VulkanError> {
        let context = std::sync::Arc::new(VulkanContext::new()?);
        let renderer = VulkanRenderer::new(context.clone())?;
        Ok(Self { context, renderer })
    }

    /// Returns the selected physical-device name with invalid bytes replaced.
    pub fn device_name(&self) -> String {
        self.context.device_name()
    }

    /// Returns the compute queue-family index used by submissions.
    pub fn queue_family_index(&self) -> u32 {
        self.context.queue_family_index()
    }

    /// Returns whether the Khronos validation layer was enabled at creation.
    pub fn validation_enabled(&self) -> bool {
        self.context.validation_enabled()
    }
}

impl GpuBackend for VulkanBackend {
    type Surface = VulkanSurface;
    type Error = VulkanError;

    fn capabilities(&self) -> GpuCapabilities {
        self.context.capabilities()
    }

    fn create_surface(
        &mut self,
        descriptor: GpuSurfaceDescriptor,
    ) -> Result<Self::Surface, Self::Error> {
        if !self.capabilities().supports_surface(descriptor) {
            return Err(VulkanError::new(VulkanErrorCode::SurfaceAllocationFailed));
        }
        VulkanSurface::new(self.context.clone(), descriptor)
    }

    fn submit(
        &mut self,
        surface: &mut Self::Surface,
        commands: &GpuCommandBuffer,
    ) -> Result<(), Self::Error> {
        if !surface.belongs_to(&self.context) {
            return Err(VulkanError::new(VulkanErrorCode::UnsupportedCommand));
        }
        commands::submit(&self.renderer, self.context.clone(), surface, commands)
    }
}
