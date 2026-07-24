use std::fmt;

use skia_core::Color;
use skia_image::{Image, ImageError};

/// Stable failure from test resource creation.
#[derive(Debug)]
pub enum TestGpuResourceError {
    /// The requested dimensions cannot be represented in an owned byte buffer.
    DimensionsOverflow,
    /// Allocating the requested fixture pixels failed.
    AllocationFailed,
    /// The RGBA8 image constructor rejected the requested resource.
    Image(ImageError),
    /// A checkerboard tile has no area.
    EmptyTile,
    /// The image contains a color outside the supported two-color BC1 fixture set.
    UnsupportedColor,
}

impl fmt::Display for TestGpuResourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DimensionsOverflow => formatter.write_str("GPU test image dimensions overflow"),
            Self::AllocationFailed => formatter.write_str("GPU test image allocation failed"),
            Self::Image(_) => formatter.write_str("GPU test image is invalid"),
            Self::EmptyTile => formatter.write_str("GPU test checkerboard tile is empty"),
            Self::UnsupportedColor => {
                formatter.write_str("GPU test image contains an unsupported BC1 fixture color")
            }
        }
    }
}

impl std::error::Error for TestGpuResourceError {}

/// Immutable image retained by a GPU test fixture.
///
/// Backends upload this image when a command references it, while the fixture
/// keeps the portable source alive for the entire test submission.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ManagedImage {
    image: Image,
}

impl ManagedImage {
    /// Wraps one already-normalized portable image.
    pub const fn new(image: Image) -> Self {
        Self { image }
    }

    /// Borrows the immutable image resource.
    pub const fn image(&self) -> &Image {
        &self.image
    }

    /// Consumes the fixture and returns its image resource.
    pub fn into_image(self) -> Image {
        self.image
    }
}

/// Creates RGBA8 image resources used by native-backend tests.
pub struct BackendTextureImageFactory;

impl BackendTextureImageFactory {
    /// Wraps caller-owned tightly packed RGBA8 pixels in one managed fixture.
    pub fn from_rgba8(
        width: u32,
        height: u32,
        pixels: Vec<u8>,
    ) -> Result<ManagedImage, TestGpuResourceError> {
        Image::from_rgba8(width, height, pixels)
            .map(ManagedImage::new)
            .map_err(TestGpuResourceError::Image)
    }

    /// Builds one solid-color image suitable for texture upload tests.
    pub fn solid(
        width: u32,
        height: u32,
        color: Color,
    ) -> Result<ManagedImage, TestGpuResourceError> {
        let pixel_count = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .ok_or(TestGpuResourceError::DimensionsOverflow)?;
        let byte_count = pixel_count
            .checked_mul(4)
            .ok_or(TestGpuResourceError::DimensionsOverflow)?;
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(byte_count)
            .map_err(|_| TestGpuResourceError::AllocationFailed)?;
        for _ in 0..pixel_count {
            pixels.extend_from_slice(&color.channels());
        }
        Self::from_rgba8(width, height, pixels)
    }

    /// Builds one two-color checkerboard image with square tiles.
    pub fn checkerboard(
        width: u32,
        height: u32,
        tile_size: u32,
        first: Color,
        second: Color,
    ) -> Result<ManagedImage, TestGpuResourceError> {
        if tile_size == 0 {
            return Err(TestGpuResourceError::EmptyTile);
        }
        let pixel_count = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .ok_or(TestGpuResourceError::DimensionsOverflow)?;
        let byte_count = pixel_count
            .checked_mul(4)
            .ok_or(TestGpuResourceError::DimensionsOverflow)?;
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(byte_count)
            .map_err(|_| TestGpuResourceError::AllocationFailed)?;
        for y in 0..height {
            for x in 0..width {
                let color = if (x / tile_size + y / tile_size).is_multiple_of(2) {
                    first
                } else {
                    second
                };
                pixels.extend_from_slice(&color.channels());
            }
        }
        Self::from_rgba8(width, height, pixels)
    }
}
