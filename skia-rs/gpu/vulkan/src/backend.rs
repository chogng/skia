use std::sync::Arc;

use skia_gpu::{GpuBackend, GpuCapabilities, GpuCommandBuffer, GpuSurfaceDescriptor};

use crate::{
    VulkanError, VulkanErrorCode, commands, context::VulkanContext, renderer::VulkanRenderer,
    surface::VulkanSurface,
};

/// Dynamically loaded Vulkan instance, device, and graphics queue.
pub struct VulkanBackend {
    context: Arc<VulkanContext>,
    renderer: VulkanRenderer,
}

impl VulkanBackend {
    /// Opens the system Vulkan loader and selects one compute-capable device.
    pub fn new() -> Result<Self, VulkanError> {
        let context = Arc::new(VulkanContext::new()?);
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
