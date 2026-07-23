use crate::{GpuCapabilities, GpuCommandBuffer, GpuSurfaceDescriptor};

/// Backend-specific error contract for GPU device operations.
pub trait GpuBackendError: std::error::Error + Send + Sync + 'static {}

impl<T> GpuBackendError for T where T: std::error::Error + Send + Sync + 'static {}

/// Platform-specific implementation of GPU surface allocation and submission.
///
/// Backends must validate device limits, resource ownership, and command support
/// before submission. This crate does not make a GPU backend authoritative for
/// the CPU reference rasterizer's pixels.
pub trait GpuBackend {
    /// Opaque backend-owned surface or texture target.
    type Surface;
    /// Backend-specific, source-redacted operational failure.
    type Error: GpuBackendError;

    /// Returns allocation limits discovered for the selected device/backend.
    fn capabilities(&self) -> GpuCapabilities;

    /// Allocates one backend-owned render target.
    fn create_surface(
        &mut self,
        descriptor: GpuSurfaceDescriptor,
    ) -> Result<Self::Surface, Self::Error>;

    /// Submits one immutable command buffer to an existing target.
    fn submit(
        &mut self,
        surface: &mut Self::Surface,
        commands: &GpuCommandBuffer,
    ) -> Result<(), Self::Error>;
}
