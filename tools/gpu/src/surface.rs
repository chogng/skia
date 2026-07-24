use skia_gpu::{GpuBackend, GpuSurfaceDescriptor};

/// Constructs portable RGBA8 test surfaces for one backend.
#[derive(Debug)]
pub struct BackendSurfaceFactory<B> {
    backend: B,
}

impl<B> BackendSurfaceFactory<B> {
    /// Retains one backend that will own every surface created by this factory.
    pub const fn new(backend: B) -> Self {
        Self { backend }
    }

    /// Borrows the underlying backend for backend-specific test setup.
    pub const fn backend(&self) -> &B {
        &self.backend
    }

    /// Mutably borrows the underlying backend.
    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    /// Consumes the factory and returns its backend.
    pub fn into_backend(self) -> B {
        self.backend
    }
}

impl<B: GpuBackend> BackendSurfaceFactory<B> {
    /// Allocates one non-empty RGBA8 surface through the retained backend.
    pub fn create_rgba8(&mut self, width: u32, height: u32) -> Result<B::Surface, B::Error> {
        let descriptor = GpuSurfaceDescriptor::new(width, height)
            .expect("test surface dimensions are non-empty");
        self.backend.create_surface(descriptor)
    }
}
