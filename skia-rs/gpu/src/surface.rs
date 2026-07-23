use crate::{GpuCommandError, GpuCommandErrorCode};

/// Portable pixel format of a GPU render target.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GpuSurfaceFormat {
    /// Straight-alpha, normalized eight-bit red, green, blue, and alpha channels.
    Rgba8Unorm,
}

/// Bounded dimensions and format of a GPU render target.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GpuSurfaceDescriptor {
    width: u32,
    height: u32,
    format: GpuSurfaceFormat,
}

impl GpuSurfaceDescriptor {
    /// Creates one non-empty straight-alpha RGBA8 target descriptor.
    pub fn new(width: u32, height: u32) -> Result<Self, GpuCommandError> {
        Self::with_format(width, height, GpuSurfaceFormat::Rgba8Unorm)
    }

    /// Creates one non-empty target descriptor with an explicit portable format.
    pub fn with_format(
        width: u32,
        height: u32,
        format: GpuSurfaceFormat,
    ) -> Result<Self, GpuCommandError> {
        if width == 0 || height == 0 {
            return Err(GpuCommandError::new(GpuCommandErrorCode::InvalidSurface));
        }
        Ok(Self {
            width,
            height,
            format,
        })
    }

    /// Returns the target width in physical pixels.
    pub const fn width(self) -> u32 {
        self.width
    }

    /// Returns the target height in physical pixels.
    pub const fn height(self) -> u32 {
        self.height
    }

    /// Returns the portable target pixel format.
    pub const fn format(self) -> GpuSurfaceFormat {
        self.format
    }

    pub(crate) fn byte_len(self) -> Option<u64> {
        u64::from(self.width)
            .checked_mul(u64::from(self.height))
            .and_then(|pixels| pixels.checked_mul(4))
    }
}
