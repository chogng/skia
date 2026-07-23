use std::fmt;

/// Stable machine-readable image creation failure code.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ImageErrorCode {
    /// Image dimensions are empty.
    InvalidDimensions,
    /// Dimensions cannot be represented by the host or their byte count overflowed.
    NumericOverflow,
    /// Pixel storage does not exactly match tightly packed RGBA8 dimensions.
    InvalidPixels,
    /// An ICC color profile is empty.
    InvalidColorProfile,
}

/// Color interpretation for RGBA8 image samples.
///
/// [`ColorSpace::Srgb`] is the canonical color space used by images created
/// with [`Image::from_rgba8`]. An ICC profile is retained exactly as supplied;
/// callers that need conversion must perform it explicitly.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ColorSpace {
    /// Standard sRGB color space.
    Srgb,
    /// An embedded ICC profile that describes the image samples.
    Icc(Vec<u8>),
}

impl ColorSpace {
    /// Creates a color space from a non-empty ICC profile.
    pub fn from_icc_profile(profile: Vec<u8>) -> Result<Self, ImageError> {
        if profile.is_empty() {
            return Err(ImageError::new(ImageErrorCode::InvalidColorProfile));
        }
        Ok(Self::Icc(profile))
    }

    /// Returns the ICC profile when this is an ICC-tagged color space.
    pub fn icc_profile(&self) -> Option<&[u8]> {
        match self {
            Self::Srgb => None,
            Self::Icc(profile) => Some(profile),
        }
    }
}

/// Source-redacted image creation error.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ImageError {
    code: ImageErrorCode,
}

impl ImageError {
    /// Creates an error with a stable code.
    pub const fn new(code: ImageErrorCode) -> Self {
        Self { code }
    }

    /// Returns the stable error code.
    pub const fn code(self) -> ImageErrorCode {
        self.code
    }
}

impl fmt::Display for ImageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}", self.code)
    }
}

impl std::error::Error for ImageError {}

/// Immutable, tightly packed, straight-alpha RGBA8 image data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Image {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    color_space: ColorSpace,
}

impl Image {
    /// Takes ownership of one exact, non-empty RGBA8 pixel buffer.
    pub fn from_rgba8(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, ImageError> {
        Self::from_rgba8_with_color_space(width, height, pixels, ColorSpace::Srgb)
    }

    /// Takes ownership of one exact, non-empty RGBA8 pixel buffer with an
    /// explicit sample color space.
    pub fn from_rgba8_with_color_space(
        width: u32,
        height: u32,
        pixels: Vec<u8>,
        color_space: ColorSpace,
    ) -> Result<Self, ImageError> {
        if width == 0 || height == 0 {
            return Err(ImageError::new(ImageErrorCode::InvalidDimensions));
        }
        let expected = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|value| value.checked_mul(4))
            .ok_or(ImageError::new(ImageErrorCode::NumericOverflow))?;
        if usize::try_from(expected).ok() != Some(pixels.len()) {
            return Err(ImageError::new(ImageErrorCode::InvalidPixels));
        }
        Ok(Self {
            width,
            height,
            pixels,
            color_space,
        })
    }

    /// Returns the image width in pixels.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the image height in pixels.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Borrows the exact row-major RGBA8 pixels.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Returns the color space that describes the stored RGBA8 samples.
    pub const fn color_space(&self) -> &ColorSpace {
        &self.color_space
    }

    /// Returns one RGBA8 pixel, or `None` when coordinates are out of bounds.
    pub fn pixel_at(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let offset = (usize::try_from(y).ok()? * usize::try_from(self.width).ok()?
            + usize::try_from(x).ok()?)
            * 4;
        self.pixels.get(offset..offset + 4)?.try_into().ok()
    }
}

#[cfg(test)]
#[path = "image_tests.rs"]
mod tests;
