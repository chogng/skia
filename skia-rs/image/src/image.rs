use std::fmt;

use moxcms::{
    CicpColorPrimaries, CicpProfile, ColorProfile, DataColorSpace, Layout, MatrixCoefficients,
    ProfileClass, TransferCharacteristics, TransformOptions,
};

/// Stable machine-readable image creation or conversion failure code.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ImageErrorCode {
    /// Image dimensions are empty.
    InvalidDimensions,
    /// Dimensions cannot be represented by the host or their byte count overflowed.
    NumericOverflow,
    /// Pixel storage does not match the declared dimensions and row stride.
    InvalidPixels,
    /// A row stride is smaller than the format's minimum or is not representable.
    InvalidRowBytes,
    /// Stored alpha values violate the declared alpha representation.
    InvalidAlpha,
    /// An ICC profile is malformed, non-RGB, or outside the supported matrix/TRC subset.
    UnsupportedColorProfile,
    /// A color conversion could not be constructed or executed.
    ColorTransformFailed,
    /// Pixel storage allocation failed.
    AllocationFailed,
}

/// Byte ordering for one interleaved eight-bit color pixel.
///
/// Both currently supported formats have real read, conversion, and rendering
/// behavior. New channel depths and planar formats can be added without changing
/// [`ImageInfo`] or renderer resource ownership.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PixelFormat {
    /// Red, green, blue, then alpha bytes.
    Rgba8888,
    /// Blue, green, red, then alpha bytes.
    Bgra8888,
}

impl PixelFormat {
    /// Returns the number of bytes occupied by one pixel.
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Rgba8888 | Self::Bgra8888 => 4,
        }
    }
}

/// Meaning of RGB and alpha values in pixel storage.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AlphaType {
    /// RGB channels are independent of alpha.
    Straight,
    /// RGB channels have already been multiplied by alpha in the same encoded space.
    Premultiplied,
    /// Every stored alpha value is 255.
    Opaque,
}

/// Color interpretation for encoded RGB samples.
///
/// ICC profiles are accepted only when they are valid RGB profiles and the
/// bounded matrix/TRC path can construct a transform to sRGB. LUT, CMYK,
/// device-link, and malformed profiles are rejected instead of being treated
/// as sRGB.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ColorSpace {
    /// IEC 61966-2-1 sRGB transfer function and BT.709/sRGB primaries.
    Srgb,
    /// Linear transfer function with BT.709/sRGB primaries.
    LinearSrgb,
    /// Validated RGB ICC profile bytes.
    Icc(Vec<u8>),
}

impl ColorSpace {
    /// Creates a color space from a supported, non-empty RGB ICC profile.
    pub fn from_icc_profile(profile: Vec<u8>) -> Result<Self, ImageError> {
        let color_space = Self::Icc(profile);
        color_space.validate()?;
        Ok(color_space)
    }

    /// Returns the original ICC bytes when this is an ICC-backed color space.
    pub fn icc_profile(&self) -> Option<&[u8]> {
        match self {
            Self::Srgb | Self::LinearSrgb => None,
            Self::Icc(profile) => Some(profile),
        }
    }

    /// Returns ICC bytes suitable for embedding in an encoded image.
    ///
    /// sRGB can be represented by format defaults, linear sRGB is serialized
    /// as a matrix/TRC profile, and original ICC bytes are preserved exactly.
    pub fn encoded_icc_profile(&self) -> Result<Option<Vec<u8>>, ImageError> {
        match self {
            Self::Srgb => Ok(None),
            Self::LinearSrgb => linear_srgb_profile()
                .encode()
                .map(Some)
                .map_err(|_| ImageError::new(ImageErrorCode::ColorTransformFailed)),
            Self::Icc(profile) => {
                self.validate()?;
                Ok(Some(profile.clone()))
            }
        }
    }

    /// Returns a deterministic matrix/TRC ICC profile for standard sRGB.
    ///
    /// Container formats whose defaults already specify sRGB need not embed
    /// this profile. Output formats with an explicit color-management contract,
    /// such as PDF output intents, can use it to name their default RGB space.
    pub fn srgb_icc_profile() -> Result<Vec<u8>, ImageError> {
        srgb_profile()
            .encode()
            .map_err(|_| ImageError::new(ImageErrorCode::ColorTransformFailed))
    }

    fn validate(&self) -> Result<(), ImageError> {
        if let Self::Icc(profile) = self {
            parse_supported_icc(profile)?
                .create_transform_8bit(
                    Layout::Rgba,
                    &srgb_profile(),
                    Layout::Rgba,
                    TransformOptions::default(),
                )
                .map_err(|_| ImageError::new(ImageErrorCode::UnsupportedColorProfile))?;
        }
        Ok(())
    }

    fn profile(&self) -> Result<ColorProfile, ImageError> {
        match self {
            Self::Srgb => Ok(srgb_profile()),
            Self::LinearSrgb => Ok(linear_srgb_profile()),
            Self::Icc(profile) => parse_supported_icc(profile),
        }
    }
}

/// Immutable pixel dimensions, storage format, alpha representation, and color space.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageInfo {
    width: u32,
    height: u32,
    pixel_format: PixelFormat,
    alpha_type: AlphaType,
    color_space: ColorSpace,
}

impl ImageInfo {
    /// Creates a non-empty image description.
    pub fn new(
        width: u32,
        height: u32,
        pixel_format: PixelFormat,
        alpha_type: AlphaType,
        color_space: ColorSpace,
    ) -> Result<Self, ImageError> {
        if width == 0 || height == 0 {
            return Err(ImageError::new(ImageErrorCode::InvalidDimensions));
        }
        color_space.validate()?;
        let info = Self {
            width,
            height,
            pixel_format,
            alpha_type,
            color_space,
        };
        info.min_row_bytes()?;
        Ok(info)
    }

    /// Returns the width in pixels.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the height in pixels.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Returns the byte-level pixel format.
    pub const fn pixel_format(&self) -> PixelFormat {
        self.pixel_format
    }

    /// Returns the stored alpha representation.
    pub const fn alpha_type(&self) -> AlphaType {
        self.alpha_type
    }

    /// Returns the color space describing stored RGB samples.
    pub const fn color_space(&self) -> &ColorSpace {
        &self.color_space
    }

    /// Returns the minimum tightly packed row stride.
    pub fn min_row_bytes(&self) -> Result<usize, ImageError> {
        usize::try_from(self.width)
            .ok()
            .and_then(|width| width.checked_mul(self.pixel_format.bytes_per_pixel()))
            .ok_or(ImageError::new(ImageErrorCode::NumericOverflow))
    }

    fn byte_len(&self, row_bytes: usize) -> Result<usize, ImageError> {
        if row_bytes < self.min_row_bytes()? {
            return Err(ImageError::new(ImageErrorCode::InvalidRowBytes));
        }
        row_bytes
            .checked_mul(
                usize::try_from(self.height)
                    .map_err(|_| ImageError::new(ImageErrorCode::NumericOverflow))?,
            )
            .ok_or(ImageError::new(ImageErrorCode::NumericOverflow))
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

/// Immutable owned pixels with an explicit row stride, alpha type, and color space.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Image {
    info: ImageInfo,
    row_bytes: usize,
    pixels: Vec<u8>,
}

impl Image {
    /// Takes ownership of tightly packed straight-alpha sRGB RGBA8 pixels.
    pub fn from_rgba8(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, ImageError> {
        Self::from_rgba8_with_color_space(width, height, pixels, ColorSpace::Srgb)
    }

    /// Takes ownership of tightly packed straight-alpha RGBA8 pixels with an
    /// explicit sample color space.
    pub fn from_rgba8_with_color_space(
        width: u32,
        height: u32,
        pixels: Vec<u8>,
        color_space: ColorSpace,
    ) -> Result<Self, ImageError> {
        let info = ImageInfo::new(
            width,
            height,
            PixelFormat::Rgba8888,
            AlphaType::Straight,
            color_space,
        )?;
        let row_bytes = info.min_row_bytes()?;
        Self::from_pixels(info, row_bytes, pixels)
    }

    /// Takes ownership of pixels matching an explicit description and row stride.
    ///
    /// Storage must contain exactly `row_bytes * height` bytes. Padding bytes
    /// at the end of rows are retained but never interpreted as pixels.
    pub fn from_pixels(
        info: ImageInfo,
        row_bytes: usize,
        pixels: Vec<u8>,
    ) -> Result<Self, ImageError> {
        if info.byte_len(row_bytes)? != pixels.len() {
            return Err(ImageError::new(ImageErrorCode::InvalidPixels));
        }
        validate_alpha(&info, row_bytes, &pixels)?;
        Ok(Self {
            info,
            row_bytes,
            pixels,
        })
    }

    /// Returns the complete image description.
    pub const fn info(&self) -> &ImageInfo {
        &self.info
    }

    /// Returns the image width in pixels.
    pub const fn width(&self) -> u32 {
        self.info.width
    }

    /// Returns the image height in pixels.
    pub const fn height(&self) -> u32 {
        self.info.height
    }

    /// Returns the number of stored bytes between adjacent row starts.
    pub const fn row_bytes(&self) -> usize {
        self.row_bytes
    }

    /// Borrows raw storage, including any row padding.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Returns the color space that describes stored RGB samples.
    pub const fn color_space(&self) -> &ColorSpace {
        &self.info.color_space
    }

    /// Returns one logical straight-alpha RGBA8 pixel without color conversion.
    pub fn pixel_at(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width() || y >= self.height() {
            return None;
        }
        let offset = usize::try_from(y)
            .ok()?
            .checked_mul(self.row_bytes)?
            .checked_add(
                usize::try_from(x)
                    .ok()?
                    .checked_mul(self.info.pixel_format.bytes_per_pixel())?,
            )?;
        let stored: [u8; 4] = self.pixels.get(offset..offset + 4)?.try_into().ok()?;
        let mut rgba = match self.info.pixel_format {
            PixelFormat::Rgba8888 => stored,
            PixelFormat::Bgra8888 => [stored[2], stored[1], stored[0], stored[3]],
        };
        match self.info.alpha_type {
            AlphaType::Straight => {}
            AlphaType::Opaque => rgba[3] = u8::MAX,
            AlphaType::Premultiplied if rgba[3] == 0 => rgba = [0; 4],
            AlphaType::Premultiplied => {
                let alpha = rgba[3];
                for channel in &mut rgba[..3] {
                    *channel = unpremultiply(*channel, alpha);
                }
            }
        }
        Some(rgba)
    }

    /// Converts format, alpha representation, and color space into a new image.
    ///
    /// Color conversion always operates on straight RGB values. Premultiplication
    /// is removed before the transform and applied exactly once afterward.
    pub fn converted(
        &self,
        pixel_format: PixelFormat,
        alpha_type: AlphaType,
        color_space: ColorSpace,
    ) -> Result<Self, ImageError> {
        let mut rgba = self.straight_rgba8()?;
        if self.color_space() != &color_space {
            let source = self.color_space().profile()?;
            let destination = color_space.profile()?;
            let transform = source
                .create_transform_8bit(
                    Layout::Rgba,
                    &destination,
                    Layout::Rgba,
                    TransformOptions::default(),
                )
                .map_err(|_| ImageError::new(ImageErrorCode::ColorTransformFailed))?;
            let mut transformed = allocate_zeroed(rgba.len())?;
            transform
                .transform(&rgba, &mut transformed)
                .map_err(|_| ImageError::new(ImageErrorCode::ColorTransformFailed))?;
            rgba = transformed;
        }
        encode_pixels(
            self.width(),
            self.height(),
            rgba,
            pixel_format,
            alpha_type,
            color_space,
        )
    }

    /// Produces tightly packed straight-alpha sRGB RGBA8 rendering pixels.
    pub fn to_rendering_image(&self) -> Result<Self, ImageError> {
        self.converted(PixelFormat::Rgba8888, AlphaType::Straight, ColorSpace::Srgb)
    }

    /// Produces tightly packed straight-alpha RGBA8 storage without changing color space.
    pub fn to_straight_rgba8(&self) -> Result<Self, ImageError> {
        self.converted(
            PixelFormat::Rgba8888,
            AlphaType::Straight,
            self.color_space().clone(),
        )
    }

    fn straight_rgba8(&self) -> Result<Vec<u8>, ImageError> {
        let pixel_count = usize::try_from(self.width())
            .ok()
            .and_then(|width| {
                usize::try_from(self.height())
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .ok_or(ImageError::new(ImageErrorCode::NumericOverflow))?;
        let mut output = Vec::new();
        output
            .try_reserve_exact(
                pixel_count
                    .checked_mul(4)
                    .ok_or(ImageError::new(ImageErrorCode::NumericOverflow))?,
            )
            .map_err(|_| ImageError::new(ImageErrorCode::AllocationFailed))?;
        for y in 0..self.height() {
            for x in 0..self.width() {
                output.extend_from_slice(
                    &self
                        .pixel_at(x, y)
                        .ok_or(ImageError::new(ImageErrorCode::InvalidPixels))?,
                );
            }
        }
        Ok(output)
    }
}

fn validate_alpha(info: &ImageInfo, row_bytes: usize, pixels: &[u8]) -> Result<(), ImageError> {
    let width = usize::try_from(info.width)
        .map_err(|_| ImageError::new(ImageErrorCode::NumericOverflow))?;
    let height = usize::try_from(info.height)
        .map_err(|_| ImageError::new(ImageErrorCode::NumericOverflow))?;
    for y in 0..height {
        let row = &pixels[y * row_bytes..y * row_bytes + width * 4];
        for stored in row.chunks_exact(4) {
            let (red, green, blue, alpha) = match info.pixel_format {
                PixelFormat::Rgba8888 => (stored[0], stored[1], stored[2], stored[3]),
                PixelFormat::Bgra8888 => (stored[2], stored[1], stored[0], stored[3]),
            };
            let valid = match info.alpha_type {
                AlphaType::Straight => true,
                AlphaType::Opaque => alpha == u8::MAX,
                AlphaType::Premultiplied => red <= alpha && green <= alpha && blue <= alpha,
            };
            if !valid {
                return Err(ImageError::new(ImageErrorCode::InvalidAlpha));
            }
        }
    }
    Ok(())
}

fn encode_pixels(
    width: u32,
    height: u32,
    mut rgba: Vec<u8>,
    pixel_format: PixelFormat,
    alpha_type: AlphaType,
    color_space: ColorSpace,
) -> Result<Image, ImageError> {
    for pixel in rgba.chunks_exact_mut(4) {
        match alpha_type {
            AlphaType::Straight => {}
            AlphaType::Opaque if pixel[3] != u8::MAX => {
                return Err(ImageError::new(ImageErrorCode::InvalidAlpha));
            }
            AlphaType::Opaque => {}
            AlphaType::Premultiplied => {
                let alpha = pixel[3];
                for channel in &mut pixel[..3] {
                    *channel = premultiply(*channel, alpha);
                }
            }
        }
        if pixel_format == PixelFormat::Bgra8888 {
            pixel.swap(0, 2);
        }
    }
    let info = ImageInfo::new(width, height, pixel_format, alpha_type, color_space)?;
    let row_bytes = info.min_row_bytes()?;
    Image::from_pixels(info, row_bytes, rgba)
}

fn parse_supported_icc(profile: &[u8]) -> Result<ColorProfile, ImageError> {
    if profile.is_empty() {
        return Err(ImageError::new(ImageErrorCode::UnsupportedColorProfile));
    }
    let parsed = ColorProfile::new_from_slice(profile)
        .map_err(|_| ImageError::new(ImageErrorCode::UnsupportedColorProfile))?;
    let unsupported_class = matches!(
        parsed.profile_class,
        ProfileClass::DeviceLink | ProfileClass::Abstract | ProfileClass::Named
    );
    let missing_matrix_or_trc = parsed.pcs != DataColorSpace::Xyz
        || parsed.red_trc.is_none()
        || parsed.green_trc.is_none()
        || parsed.blue_trc.is_none();
    let has_lut = parsed.lut_a_to_b_perceptual.is_some()
        || parsed.lut_a_to_b_colorimetric.is_some()
        || parsed.lut_a_to_b_saturation.is_some()
        || parsed.lut_b_to_a_perceptual.is_some()
        || parsed.lut_b_to_a_colorimetric.is_some()
        || parsed.lut_b_to_a_saturation.is_some();
    if parsed.color_space != DataColorSpace::Rgb
        || unsupported_class
        || missing_matrix_or_trc
        || has_lut
    {
        return Err(ImageError::new(ImageErrorCode::UnsupportedColorProfile));
    }
    Ok(parsed)
}

fn srgb_profile() -> ColorProfile {
    ColorProfile::new_srgb()
}

fn linear_srgb_profile() -> ColorProfile {
    ColorProfile::new_from_cicp(CicpProfile {
        color_primaries: CicpColorPrimaries::Bt709,
        transfer_characteristics: TransferCharacteristics::Linear,
        matrix_coefficients: MatrixCoefficients::Bt709,
        full_range: true,
    })
}

fn allocate_zeroed(length: usize) -> Result<Vec<u8>, ImageError> {
    let mut output = Vec::new();
    output
        .try_reserve_exact(length)
        .map_err(|_| ImageError::new(ImageErrorCode::AllocationFailed))?;
    output.resize(length, 0);
    Ok(output)
}

fn premultiply(channel: u8, alpha: u8) -> u8 {
    ((u32::from(channel) * u32::from(alpha) + 127) / 255) as u8
}

fn unpremultiply(channel: u8, alpha: u8) -> u8 {
    ((u32::from(channel) * 255 + u32::from(alpha) / 2) / u32::from(alpha)).min(255) as u8
}

#[cfg(test)]
#[path = "image_tests.rs"]
mod tests;
