use crate::{GpuCommandError, GpuCommandErrorCode, GpuSurfaceDescriptor, GpuSurfaceFormat};

/// Independent command, resource, and state-stack ceilings for one encoder.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GpuCommandLimits {
    pub(crate) max_commands: usize,
    pub(crate) max_paths: usize,
    pub(crate) max_images: usize,
    pub(crate) max_clips: usize,
    pub(crate) max_save_depth: usize,
    pub(crate) max_glyphs_per_batch: usize,
}

impl GpuCommandLimits {
    /// Creates positive, bounded limits for one command encoder.
    pub fn new(
        max_commands: usize,
        max_paths: usize,
        max_images: usize,
        max_save_depth: usize,
    ) -> Result<Self, GpuCommandError> {
        if max_commands == 0 || max_paths == 0 || max_images == 0 || max_save_depth == 0 {
            return Err(GpuCommandError::new(GpuCommandErrorCode::InvalidLimits));
        }
        Ok(Self {
            max_commands,
            max_paths,
            max_images,
            max_clips: max_commands,
            max_save_depth,
            max_glyphs_per_batch: max_commands.saturating_mul(1_024),
        })
    }

    /// Replaces the positive immutable clip-node ceiling.
    pub const fn with_max_clips(mut self, max_clips: usize) -> Result<Self, GpuCommandError> {
        if max_clips == 0 {
            return Err(GpuCommandError::new(GpuCommandErrorCode::InvalidLimits));
        }
        self.max_clips = max_clips;
        Ok(self)
    }

    /// Replaces the positive glyph count ceiling for one atlas batch.
    pub const fn with_max_glyphs_per_batch(
        mut self,
        max_glyphs_per_batch: usize,
    ) -> Result<Self, GpuCommandError> {
        if max_glyphs_per_batch == 0 {
            return Err(GpuCommandError::new(GpuCommandErrorCode::InvalidLimits));
        }
        self.max_glyphs_per_batch = max_glyphs_per_batch;
        Ok(self)
    }
}

/// Device/backend limits observable before allocating a GPU target.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GpuCapabilities {
    max_surface_width: u32,
    max_surface_height: u32,
    max_surface_bytes: u64,
}

impl GpuCapabilities {
    /// Creates positive limits for straight-alpha RGBA8 render targets.
    pub fn new(
        max_surface_width: u32,
        max_surface_height: u32,
        max_surface_bytes: u64,
    ) -> Result<Self, GpuCommandError> {
        if max_surface_width == 0 || max_surface_height == 0 || max_surface_bytes == 0 {
            return Err(GpuCommandError::new(GpuCommandErrorCode::InvalidLimits));
        }
        Ok(Self {
            max_surface_width,
            max_surface_height,
            max_surface_bytes,
        })
    }

    /// Returns the largest supported surface width.
    pub const fn max_surface_width(self) -> u32 {
        self.max_surface_width
    }

    /// Returns the largest supported surface height.
    pub const fn max_surface_height(self) -> u32 {
        self.max_surface_height
    }

    /// Returns the largest tightly packed target allocation in bytes.
    pub const fn max_surface_bytes(self) -> u64 {
        self.max_surface_bytes
    }

    /// Returns whether the backend can allocate this portable target shape and format.
    pub fn supports_surface(self, descriptor: GpuSurfaceDescriptor) -> bool {
        descriptor.format() == GpuSurfaceFormat::Rgba8Unorm
            && descriptor.width() <= self.max_surface_width
            && descriptor.height() <= self.max_surface_height
            && descriptor
                .byte_len()
                .is_some_and(|bytes| bytes <= self.max_surface_bytes)
    }
}
