//! Cross-platform Vulkan submission backend for `skia-gpu`.
//!
//! The backend owns a real dynamically loaded Vulkan instance, device, queue,
//! and offscreen RGBA8 image. Target-wide clears use a native transfer command;
//! the remaining portable command vocabulary is deterministically composed and
//! uploaded through a Vulkan staging buffer. Readback always comes from the
//! device-owned image.

#![deny(missing_docs)]
#![deny(unsafe_op_in_unsafe_fn)]

mod context;
mod surface;

use std::fmt;

use skia_gpu::{
    GpuBackend, GpuCommand, GpuCommandBuffer, GpuSurfaceDescriptor, software::SoftwareGpuBackend,
};

use crate::context::VulkanContext;
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
    /// No physical device with a graphics-capable queue was available.
    DeviceUnavailable,
    /// Logical-device, queue, or command-pool creation failed.
    DeviceCreationFailed,
    /// Offscreen image or device-memory allocation failed.
    SurfaceAllocationFailed,
    /// The command buffer contains an invalid or unsupported command.
    UnsupportedCommand,
    /// Host-visible staging allocation, mapping, or device upload failed.
    UploadFailed,
    /// Command recording, queue submission, or synchronization failed.
    SubmissionFailed,
    /// Staging allocation, image copy, mapping, or readback failed.
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
    replay: SoftwareGpuBackend,
}

impl VulkanBackend {
    /// Opens the system Vulkan loader and selects one graphics-capable device.
    pub fn new() -> Result<Self, VulkanError> {
        VulkanContext::new().map(|context| Self {
            context: std::sync::Arc::new(context),
            replay: SoftwareGpuBackend::default(),
        })
    }

    /// Returns the selected physical-device name with invalid bytes replaced.
    pub fn device_name(&self) -> String {
        self.context.device_name()
    }

    /// Returns the graphics queue-family index used by submissions.
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

    fn create_surface(
        &mut self,
        descriptor: GpuSurfaceDescriptor,
    ) -> Result<Self::Surface, Self::Error> {
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
        if commands.commands().is_empty() {
            return Ok(());
        }

        self.replay
            .submit(surface.replay_surface_mut(), commands)
            .map_err(|_| VulkanError::new(VulkanErrorCode::SubmissionFailed))?;

        if commands
            .commands()
            .iter()
            .all(|command| matches!(command, GpuCommand::Clear(_)))
        {
            let color = commands
                .commands()
                .iter()
                .rev()
                .find_map(|command| match command {
                    GpuCommand::Clear(color) => Some(*color),
                    _ => None,
                })
                .ok_or(VulkanError::new(VulkanErrorCode::SubmissionFailed))?;
            surface.clear(color)
        } else {
            surface.upload_replay_surface()
        }
    }
}
