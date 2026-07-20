//! Parsing and encoding of general-purpose image formats.
//!
//! This crate deliberately owns image-format policy rather than pixel storage.
//! Decoding accepts untrusted encoded data subject to [`CodecLimits`], while
//! encoding converts portable, tightly packed, straight-alpha RGBA8 [`Image`]
//! resources to PNG, JPEG, or WebP. Rendering backends therefore never parse
//! image files or encode image formats.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::{fmt, io::Cursor};

use image::{
    codecs::{jpeg::JpegEncoder, png::PngEncoder, webp::WebPEncoder},
    ExtendedColorType, ImageEncoder, ImageError, ImageFormat, ImageReader, Limits,
};
use skia_image::Image;

/// Stable machine-readable image codec failure code.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CodecErrorCode {
    /// Codec limits are empty or otherwise invalid.
    InvalidLimits,
    /// The encoded input exceeds its configured byte ceiling.
    InputTooLarge,
    /// The decoded dimensions or RGBA8 payload exceed configured limits.
    ImageTooLarge,
    /// The encoded bytes do not identify a supported image format.
    UnsupportedFormat,
    /// A supported encoded image could not be decoded.
    DecodeFailed,
    /// JPEG quality is outside its inclusive 1 through 100 range.
    InvalidJpegQuality,
    /// A Skia image resource could not be encoded.
    EncodeFailed,
}

/// Source-redacted image codec error.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CodecError {
    code: CodecErrorCode,
}

impl CodecError {
    /// Creates an error with a stable code.
    pub const fn new(code: CodecErrorCode) -> Self {
        Self { code }
    }

    /// Returns the stable error code.
    pub const fn code(self) -> CodecErrorCode {
        self.code
    }
}

impl fmt::Display for CodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}", self.code)
    }
}

impl std::error::Error for CodecError {}

/// Resource ceilings applied while decoding untrusted encoded image bytes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CodecLimits {
    max_input_bytes: usize,
    max_pixels: u64,
    max_decoded_bytes: u64,
}

impl CodecLimits {
    /// Creates non-zero codec resource limits.
    pub fn new(
        max_input_bytes: usize,
        max_pixels: u64,
        max_decoded_bytes: u64,
    ) -> Result<Self, CodecError> {
        if max_input_bytes == 0 || max_pixels == 0 || max_decoded_bytes == 0 {
            return Err(CodecError::new(CodecErrorCode::InvalidLimits));
        }
        Ok(Self {
            max_input_bytes,
            max_pixels,
            max_decoded_bytes,
        })
    }

    /// Returns the maximum accepted encoded input length in bytes.
    pub const fn max_input_bytes(self) -> usize {
        self.max_input_bytes
    }

    /// Returns the maximum accepted decoded pixel count.
    pub const fn max_pixels(self) -> u64 {
        self.max_pixels
    }

    /// Returns the maximum accepted decoded RGBA8 byte count.
    pub const fn max_decoded_bytes(self) -> u64 {
        self.max_decoded_bytes
    }
}

impl Default for CodecLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 64 * 1024 * 1024,
            max_pixels: 67_108_864,
            max_decoded_bytes: 256 * 1024 * 1024,
        }
    }
}

/// Output format and settings for an encoded image.
///
/// PNG and lossless WebP preserve straight-alpha RGBA8 pixels. JPEG has no
/// alpha channel, so [`ImageCodec::encode`] discards its alpha channel.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum EncodedImageFormat {
    /// Lossless PNG with an RGBA8 color model.
    Png,
    /// Lossy JPEG with the supplied inclusive quality value from 1 through 100.
    Jpeg {
        /// The JPEG encoder's quality setting, from 1 (lowest) to 100 (highest).
        quality: u8,
    },
    /// Lossless WebP with an RGBA8 color model.
    WebP,
}

/// Stateless codec facade for common image-format parsing and encoding.
pub struct ImageCodec;

impl ImageCodec {
    /// Decodes PNG, JPEG, or WebP bytes using [`CodecLimits::default`].
    pub fn decode(bytes: &[u8]) -> Result<Image, CodecError> {
        Self::decode_with_limits(bytes, CodecLimits::default())
    }

    /// Detects and decodes PNG, JPEG, or WebP bytes subject to `limits`.
    ///
    /// The result is always straight-alpha RGBA8, including for opaque input.
    pub fn decode_with_limits(bytes: &[u8], limits: CodecLimits) -> Result<Image, CodecError> {
        if bytes.len() > limits.max_input_bytes {
            return Err(CodecError::new(CodecErrorCode::InputTooLarge));
        }

        let mut reader = ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|_| CodecError::new(CodecErrorCode::DecodeFailed))?;
        if !matches!(
            reader.format(),
            Some(ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP)
        ) {
            return Err(CodecError::new(CodecErrorCode::UnsupportedFormat));
        }

        let maximum_dimension = u32::try_from(limits.max_pixels).unwrap_or(u32::MAX);
        let mut decoder_limits = Limits::default();
        decoder_limits.max_image_width = Some(maximum_dimension);
        decoder_limits.max_image_height = Some(maximum_dimension);
        decoder_limits.max_alloc = Some(limits.max_decoded_bytes);
        reader.limits(decoder_limits);
        let decoded = reader.decode().map_err(|error| match error {
            ImageError::Limits(_) => CodecError::new(CodecErrorCode::ImageTooLarge),
            _ => CodecError::new(CodecErrorCode::DecodeFailed),
        })?;
        let width = decoded.width();
        let height = decoded.height();
        let pixels = u64::from(width)
            .checked_mul(u64::from(height))
            .ok_or(CodecError::new(CodecErrorCode::ImageTooLarge))?;
        let byte_count = pixels
            .checked_mul(4)
            .ok_or(CodecError::new(CodecErrorCode::ImageTooLarge))?;
        if pixels > limits.max_pixels || byte_count > limits.max_decoded_bytes {
            return Err(CodecError::new(CodecErrorCode::ImageTooLarge));
        }

        let rgba8 = decoded.into_rgba8().into_raw();
        let image = Image::from_rgba8(width, height, rgba8)
            .map_err(|_| CodecError::new(CodecErrorCode::DecodeFailed))?;
        Ok(image)
    }

    /// Encodes an [`Image`] as PNG, JPEG, or lossless WebP.
    ///
    /// PNG and WebP retain alpha. JPEG encodes RGB only and therefore discards
    /// the source alpha channel. `quality` for [`EncodedImageFormat::Jpeg`] must
    /// be in the inclusive range 1 through 100.
    pub fn encode(image: &Image, format: EncodedImageFormat) -> Result<Vec<u8>, CodecError> {
        let mut encoded = Vec::new();
        let result = match format {
            EncodedImageFormat::Png => PngEncoder::new(&mut encoded).write_image(
                image.pixels(),
                image.width(),
                image.height(),
                ExtendedColorType::Rgba8,
            ),
            EncodedImageFormat::Jpeg { quality } => {
                if !(1..=100).contains(&quality) {
                    return Err(CodecError::new(CodecErrorCode::InvalidJpegQuality));
                }
                let rgb8 = rgba8_to_rgb8(image.pixels());
                JpegEncoder::new_with_quality(&mut encoded, quality).write_image(
                    &rgb8,
                    image.width(),
                    image.height(),
                    ExtendedColorType::Rgb8,
                )
            }
            EncodedImageFormat::WebP => WebPEncoder::new_lossless(&mut encoded).write_image(
                image.pixels(),
                image.width(),
                image.height(),
                ExtendedColorType::Rgba8,
            ),
        };
        result.map_err(|_| CodecError::new(CodecErrorCode::EncodeFailed))?;
        Ok(encoded)
    }
}

fn rgba8_to_rgb8(rgba8: &[u8]) -> Vec<u8> {
    let mut rgb8 = Vec::with_capacity(rgba8.len() / 4 * 3);
    for pixel in rgba8.chunks_exact(4) {
        rgb8.extend_from_slice(&pixel[..3]);
    }
    rgb8
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use image::{DynamicImage, ImageFormat, RgbaImage};

    use skia_image::Image;

    use super::{CodecErrorCode, CodecLimits, EncodedImageFormat, ImageCodec};

    fn encoded(format: ImageFormat) -> Vec<u8> {
        let source = RgbaImage::from_raw(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 128]).unwrap();
        let mut bytes = Vec::new();
        DynamicImage::ImageRgba8(source)
            .write_to(&mut Cursor::new(&mut bytes), format)
            .unwrap();
        bytes
    }

    #[test]
    fn decodes_png_jpeg_and_webp_to_rgba8_resources() {
        for format in [ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::WebP] {
            let image = ImageCodec::decode(&encoded(format)).unwrap();
            assert_eq!((image.width(), image.height()), (2, 1));
            assert_eq!(image.pixel_at(0, 0).unwrap()[3], 255);
        }
    }

    #[test]
    fn decoder_rejects_unknown_and_over_budget_input() {
        assert_eq!(
            ImageCodec::decode(b"not an image").unwrap_err().code(),
            CodecErrorCode::UnsupportedFormat
        );
        let limits = CodecLimits::new(4, 4, 16).unwrap();
        assert_eq!(
            ImageCodec::decode_with_limits(&encoded(ImageFormat::Png), limits)
                .unwrap_err()
                .code(),
            CodecErrorCode::InputTooLarge
        );
    }

    #[test]
    fn decoder_enforces_decoded_pixel_budget() {
        let limits = CodecLimits::new(1024, 1, 4).unwrap();
        assert_eq!(
            ImageCodec::decode_with_limits(&encoded(ImageFormat::Png), limits)
                .unwrap_err()
                .code(),
            CodecErrorCode::ImageTooLarge
        );
    }

    #[test]
    fn encodes_png_jpeg_and_webp_from_image_resources() {
        let image = Image::from_rgba8(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 128]).unwrap();
        for format in [
            EncodedImageFormat::Png,
            EncodedImageFormat::Jpeg { quality: 90 },
            EncodedImageFormat::WebP,
        ] {
            let encoded = ImageCodec::encode(&image, format)
                .unwrap_or_else(|error| panic!("{format:?} failed: {error:?}"));
            let decoded = ImageCodec::decode(&encoded).unwrap();
            assert_eq!((decoded.width(), decoded.height()), (2, 1));
            if format == EncodedImageFormat::Png || format == EncodedImageFormat::WebP {
                assert_eq!(decoded.pixel_at(1, 0).unwrap()[3], 128);
            }
        }
    }

    #[test]
    fn encoder_rejects_invalid_jpeg_quality() {
        let image = Image::from_rgba8(1, 1, vec![0, 0, 0, 255]).unwrap();
        assert_eq!(
            ImageCodec::encode(&image, EncodedImageFormat::Jpeg { quality: 0 })
                .unwrap_err()
                .code(),
            CodecErrorCode::InvalidJpegQuality
        );
    }
}
