use std::sync::Arc;

use skia_gpu::{
    GpuBackend, GpuCapabilities, GpuCommandBuffer, GpuSurfaceDescriptor, RuntimeShaderPacketCache,
};

use crate::{
    VulkanError, VulkanErrorCode, commands, context::VulkanContext, renderer::VulkanRenderer,
    surface::VulkanSurface,
};

/// Dynamically loaded Vulkan instance, device, and graphics queue.
pub struct VulkanBackend {
    context: Arc<VulkanContext>,
    renderer: VulkanRenderer,
    runtime_shader_packets: RuntimeShaderPacketCache,
}

/// Observable native runtime-shader pipeline-cache counters.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct VulkanRuntimeShaderPipelineCacheStats {
    hits: u64,
    misses: u64,
    evictions: u64,
    entries: usize,
}

impl VulkanRuntimeShaderPipelineCacheStats {
    /// Returns the number of program-specialized pipelines reused across draws.
    pub const fn hits(self) -> u64 {
        self.hits
    }

    /// Returns the number of specialized pipelines created for new programs.
    pub const fn misses(self) -> u64 {
        self.misses
    }

    /// Returns the number of least-recently-used specialized pipelines evicted.
    pub const fn evictions(self) -> u64 {
        self.evictions
    }

    /// Returns the number of retained specialized pipelines.
    pub const fn entries(self) -> usize {
        self.entries
    }
}

impl VulkanBackend {
    /// Opens the system Vulkan loader and selects one compute-capable device.
    pub fn new() -> Result<Self, VulkanError> {
        let context = Arc::new(VulkanContext::new()?);
        let renderer = VulkanRenderer::new(context.clone())?;
        Ok(Self {
            context,
            renderer,
            runtime_shader_packets: RuntimeShaderPacketCache::default(),
        })
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

    /// Returns reuse and capacity counters for runtime shader native pipelines.
    pub fn runtime_shader_pipeline_cache_stats(&self) -> VulkanRuntimeShaderPipelineCacheStats {
        let (hits, misses, evictions, entries) = self.renderer.specialized_pipeline_stats();
        VulkanRuntimeShaderPipelineCacheStats {
            hits,
            misses,
            evictions,
            entries,
        }
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
        commands::submit(
            &mut self.renderer,
            self.context.clone(),
            surface,
            commands,
            &mut self.runtime_shader_packets,
        )
    }
}
