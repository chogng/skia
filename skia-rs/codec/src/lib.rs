//! Decoding and encoding of PNG, JPEG, and WebP image assets.
//!
//! The codec API owns file-format policy. [`Image`] owns pixels and their color
//! interpretation, while [`ImageAsset`] carries file metadata separately.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod jpeg;
mod png;
mod webp;

use std::{
    fmt,
    io::{self, Cursor, Write},
};

use image::{
    DynamicImage, ImageDecoder, ImageEncoder, ImageError, ImageFormat, ImageReader, Limits,
};
use skia_image::{ColorSpace, Image};

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
    /// A still image was supplied where an animation was required.
    NotAnimated,
    /// A supported encoded image could not be decoded.
    DecodeFailed,
    /// Animation dimensions, timing, frame count, or frame placement are invalid.
    InvalidAnimation,
    /// The decoded animation exceeds its configured frame or aggregate byte ceiling.
    AnimationTooLarge,
    /// A supplied metadata payload is malformed for this API.
    InvalidMetadata,
    /// A PNG Deflate compression level is outside the inclusive range 0 through 9.
    InvalidPngCompressionLevel,
    /// JPEG quality is outside the inclusive range 1 through 100.
    InvalidJpegQuality,
    /// WebP quality is outside the inclusive range 0 through 100.
    InvalidWebPQuality,
    /// JPEG encoding was requested for non-opaque pixels without a flattening color.
    TransparentJpeg,
    /// The selected encoder backend cannot implement a requested option.
    UnsupportedEncodeOption,
    /// The encoded output exceeded its configured byte ceiling.
    OutputTooLarge,
    /// An image asset could not be encoded.
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

/// File-format metadata that can be retained independently of pixels.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ImageMetadata {
    exif_tiff: Option<Vec<u8>>,
}

impl ImageMetadata {
    /// Creates metadata with no optional fields.
    pub const fn new() -> Self {
        Self { exif_tiff: None }
    }

    /// Attaches a TIFF-form EXIF payload.
    ///
    /// The payload begins with a TIFF byte-order marker; it must not include a
    /// JPEG APP1 marker or the `Exif\0\0` prefix.
    pub fn with_exif_tiff(mut self, exif_tiff: Vec<u8>) -> Result<Self, CodecError> {
        if !is_tiff_payload(&exif_tiff) {
            return Err(CodecError::new(CodecErrorCode::InvalidMetadata));
        }
        self.exif_tiff = Some(exif_tiff);
        Ok(self)
    }

    /// Borrows the TIFF-form EXIF payload, if present.
    pub fn exif_tiff(&self) -> Option<&[u8]> {
        self.exif_tiff.as_deref()
    }
}

/// Pixels, their color interpretation, and optional file metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageAsset {
    image: Image,
    metadata: ImageMetadata,
}

impl ImageAsset {
    /// Creates an asset from an image with no optional metadata.
    pub const fn new(image: Image) -> Self {
        Self {
            image,
            metadata: ImageMetadata::new(),
        }
    }

    /// Creates an asset from an image and format metadata.
    pub const fn with_metadata(image: Image, metadata: ImageMetadata) -> Self {
        Self { image, metadata }
    }

    /// Borrows the image pixels and color-space interpretation.
    pub const fn image(&self) -> &Image {
        &self.image
    }

    /// Borrows optional format metadata.
    pub const fn metadata(&self) -> &ImageMetadata {
        &self.metadata
    }
}

/// Number of times an animation sequence is played.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AnimationLoop {
    /// Repeat indefinitely.
    Infinite,
    /// Play the complete sequence the supplied non-zero number of times.
    Finite(u32),
}

/// How a frame is combined with pixels already present on its canvas region.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AnimationBlend {
    /// Replace the destination region with the frame pixels.
    Source,
    /// Alpha-composite the frame over the destination region.
    Over,
}

/// How the canvas region is changed after a frame's display duration.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AnimationDisposal {
    /// Keep the displayed pixels for the next frame.
    Keep,
    /// Clear the frame region to the animation background.
    Background,
    /// Restore the frame region to its state before the frame.
    Previous,
}

/// Exact rational animation-frame duration in milliseconds.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FrameDuration {
    numerator_ms: u32,
    denominator: u32,
}

impl FrameDuration {
    /// Creates an exact rational millisecond duration with a non-zero denominator.
    pub fn new(numerator_ms: u32, denominator: u32) -> Result<Self, CodecError> {
        if denominator == 0 {
            return Err(CodecError::new(CodecErrorCode::InvalidAnimation));
        }
        Ok(Self {
            numerator_ms,
            denominator,
        })
    }

    /// Creates an integer millisecond duration.
    pub const fn from_millis(milliseconds: u32) -> Self {
        Self {
            numerator_ms: milliseconds,
            denominator: 1,
        }
    }

    /// Returns the numerator of the duration in milliseconds.
    pub const fn numerator_ms(self) -> u32 {
        self.numerator_ms
    }

    /// Returns the non-zero denominator of the duration in milliseconds.
    pub const fn denominator(self) -> u32 {
        self.denominator
    }
}

/// One animation frame and its placement and playback semantics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnimationFrame {
    image: Image,
    duration: FrameDuration,
    x: u32,
    y: u32,
    blend: AnimationBlend,
    disposal: AnimationDisposal,
}

impl AnimationFrame {
    /// Creates a full-image frame at the canvas origin.
    pub const fn new(image: Image, duration: FrameDuration) -> Self {
        Self {
            image,
            duration,
            x: 0,
            y: 0,
            blend: AnimationBlend::Source,
            disposal: AnimationDisposal::Keep,
        }
    }

    /// Sets the frame's top-left canvas position.
    pub const fn with_offset(mut self, x: u32, y: u32) -> Self {
        self.x = x;
        self.y = y;
        self
    }

    /// Sets how the frame is combined with the current canvas.
    pub const fn with_blend(mut self, blend: AnimationBlend) -> Self {
        self.blend = blend;
        self
    }

    /// Sets the post-display disposal operation.
    pub const fn with_disposal(mut self, disposal: AnimationDisposal) -> Self {
        self.disposal = disposal;
        self
    }

    /// Borrows the frame pixels.
    pub const fn image(&self) -> &Image {
        &self.image
    }

    /// Returns the frame duration.
    pub const fn duration(&self) -> FrameDuration {
        self.duration
    }

    /// Returns the horizontal canvas offset.
    pub const fn x(&self) -> u32 {
        self.x
    }

    /// Returns the vertical canvas offset.
    pub const fn y(&self) -> u32 {
        self.y
    }

    /// Returns the frame blend operation.
    pub const fn blend(&self) -> AnimationBlend {
        self.blend
    }

    /// Returns the frame disposal operation.
    pub const fn disposal(&self) -> AnimationDisposal {
        self.disposal
    }
}

/// A validated multi-frame image sequence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnimatedImageAsset {
    width: u32,
    height: u32,
    frames: Vec<AnimationFrame>,
    loop_count: AnimationLoop,
    background: [u8; 4],
    metadata: ImageMetadata,
}

impl AnimatedImageAsset {
    /// Creates an animation whose frames fit within a non-empty canvas.
    pub fn new(
        width: u32,
        height: u32,
        frames: Vec<AnimationFrame>,
        loop_count: AnimationLoop,
    ) -> Result<Self, CodecError> {
        if width == 0
            || height == 0
            || frames.is_empty()
            || matches!(loop_count, AnimationLoop::Finite(0))
        {
            return Err(CodecError::new(CodecErrorCode::InvalidAnimation));
        }
        let color_space = frames[0].image.color_space();
        for frame in &frames {
            let right = frame.x.checked_add(frame.image.width());
            let bottom = frame.y.checked_add(frame.image.height());
            if right.is_none_or(|right| right > width)
                || bottom.is_none_or(|bottom| bottom > height)
                || frame.image.color_space() != color_space
            {
                return Err(CodecError::new(CodecErrorCode::InvalidAnimation));
            }
        }
        Ok(Self {
            width,
            height,
            frames,
            loop_count,
            background: [0; 4],
            metadata: ImageMetadata::new(),
        })
    }

    /// Attaches global format metadata.
    pub fn with_metadata(mut self, metadata: ImageMetadata) -> Self {
        self.metadata = metadata;
        self
    }

    /// Sets the RGBA8 animation background color.
    pub const fn with_background(mut self, background: [u8; 4]) -> Self {
        self.background = background;
        self
    }

    /// Returns the canvas width.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the canvas height.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Borrows animation frames in playback order.
    pub fn frames(&self) -> &[AnimationFrame] {
        &self.frames
    }

    /// Returns the sequence loop policy.
    pub const fn loop_count(&self) -> AnimationLoop {
        self.loop_count
    }

    /// Returns the RGBA8 animation background.
    pub const fn background(&self) -> [u8; 4] {
        self.background
    }

    /// Borrows global format metadata.
    pub const fn metadata(&self) -> &ImageMetadata {
        &self.metadata
    }
}

/// Resource ceilings applied while decoding an animation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct AnimationLimits {
    codec: CodecLimits,
    max_frames: u32,
    max_total_decoded_bytes: u64,
}

impl AnimationLimits {
    /// Creates animation limits with non-zero frame and aggregate-byte ceilings.
    pub fn new(
        codec: CodecLimits,
        max_frames: u32,
        max_total_decoded_bytes: u64,
    ) -> Result<Self, CodecError> {
        if max_frames == 0 || max_total_decoded_bytes == 0 {
            return Err(CodecError::new(CodecErrorCode::InvalidLimits));
        }
        Ok(Self {
            codec,
            max_frames,
            max_total_decoded_bytes,
        })
    }

    /// Returns the still-image limits applied to the encoded input and canvas.
    pub const fn codec(self) -> CodecLimits {
        self.codec
    }

    /// Returns the maximum decoded frame count.
    pub const fn max_frames(self) -> u32 {
        self.max_frames
    }

    /// Returns the maximum aggregate decoded RGBA8 bytes across all frames.
    pub const fn max_total_decoded_bytes(self) -> u64 {
        self.max_total_decoded_bytes
    }
}

impl Default for AnimationLimits {
    fn default() -> Self {
        Self {
            codec: CodecLimits {
                max_input_bytes: 64 * 1024 * 1024,
                max_pixels: 67_108_864,
                max_decoded_bytes: 256 * 1024 * 1024,
            },
            max_frames: 1024,
            max_total_decoded_bytes: 512 * 1024 * 1024,
        }
    }
}

/// Supported encoded output formats.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum EncodedFormat {
    /// PNG output.
    Png,
    /// JPEG output.
    Jpeg,
    /// WebP output.
    WebP,
}

/// PNG compression policy.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PngCompression {
    /// Fast compression.
    Fast,
    /// Balanced compression.
    Balanced,
    /// Maximum compression selected by the backend.
    Best,
    /// Store image data without Deflate compression.
    Uncompressed,
    /// Exact Deflate level from 0 through 9.
    DeflateLevel(u8),
}

/// PNG per-row filter policy.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PngFilter {
    /// Select the filter adaptively per row.
    Adaptive,
    /// Do not filter rows.
    None,
    /// Use the Sub filter.
    Sub,
    /// Use the Up filter.
    Up,
    /// Use the Average filter.
    Average,
    /// Use the Paeth filter.
    Paeth,
}

/// PNG encoding controls.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PngOptions {
    compression: PngCompression,
    filter: PngFilter,
}

impl PngOptions {
    /// Returns the version-one balanced PNG profile.
    pub const fn balanced_v1() -> Self {
        Self {
            compression: PngCompression::Fast,
            filter: PngFilter::Adaptive,
        }
    }

    /// Sets the compression policy.
    pub const fn with_compression(mut self, compression: PngCompression) -> Self {
        self.compression = compression;
        self
    }

    /// Sets the row filter policy.
    pub const fn with_filter(mut self, filter: PngFilter) -> Self {
        self.filter = filter;
        self
    }
}

/// JPEG chroma-subsampling policy.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum JpegSubsampling {
    /// Retain full chroma resolution.
    Yuv444,
    /// Halve horizontal chroma resolution.
    Yuv422,
    /// Halve horizontal and vertical chroma resolution.
    Yuv420,
}

/// JPEG scan organization.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum JpegScan {
    /// One sequential, baseline-compatible scan.
    Baseline,
    /// Multiple progressive scans.
    Progressive,
}

/// Stable JPEG encoder effort profile.
///
/// The backend may improve its algorithms without changing these semantic
/// speed-versus-size choices.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum JpegOptimization {
    /// Prefer encoding speed and disable expensive compression searches.
    Fast,
    /// Enable the recommended quality and compression optimizations.
    Balanced,
    /// Spend additional time searching for the smallest output.
    Smallest,
}

/// Handling for alpha pixels supplied to a JPEG encoder.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum JpegAlphaHandling {
    /// Reject transparent source pixels.
    Reject,
    /// Flatten pixels over the supplied opaque RGB background.
    Flatten([u8; 3]),
}

/// JPEG encoding controls.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct JpegOptions {
    quality: u8,
    subsampling: JpegSubsampling,
    scan: JpegScan,
    optimization: JpegOptimization,
    alpha: JpegAlphaHandling,
}

impl JpegOptions {
    /// Creates a balanced, baseline-compatible, 4:4:4 JPEG profile.
    pub fn baseline_v1(quality: u8) -> Result<Self, CodecError> {
        if !(1..=100).contains(&quality) {
            return Err(CodecError::new(CodecErrorCode::InvalidJpegQuality));
        }
        Ok(Self {
            quality,
            subsampling: JpegSubsampling::Yuv444,
            scan: JpegScan::Baseline,
            optimization: JpegOptimization::Balanced,
            alpha: JpegAlphaHandling::Reject,
        })
    }

    /// Creates the version-one web delivery profile.
    ///
    /// This profile uses quality 85, 4:2:0 chroma, progressive scans, and the
    /// balanced optimization effort. Transparent pixels remain an error until
    /// an explicit alpha handling policy is supplied.
    pub const fn web_v1() -> Self {
        Self {
            quality: 85,
            subsampling: JpegSubsampling::Yuv420,
            scan: JpegScan::Progressive,
            optimization: JpegOptimization::Balanced,
            alpha: JpegAlphaHandling::Reject,
        }
    }

    /// Sets chroma subsampling.
    pub const fn with_subsampling(mut self, subsampling: JpegSubsampling) -> Self {
        self.subsampling = subsampling;
        self
    }

    /// Sets scan organization.
    pub const fn with_scan(mut self, scan: JpegScan) -> Self {
        self.scan = scan;
        self
    }

    /// Sets encoder optimization effort.
    pub const fn with_optimization(mut self, optimization: JpegOptimization) -> Self {
        self.optimization = optimization;
        self
    }

    /// Sets the alpha handling policy.
    pub const fn with_alpha_handling(mut self, alpha: JpegAlphaHandling) -> Self {
        self.alpha = alpha;
        self
    }
}

/// WebP output mode.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum WebPMode {
    /// Lossless VP8L output.
    Lossless,
    /// Lossy VP8 output with its quality setting.
    Lossy {
        /// Inclusive quality from 0 (smallest) through 100 (highest).
        quality: u8,
    },
}

/// WebP encoding controls.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct WebPOptions {
    mode: WebPMode,
}

impl WebPOptions {
    /// Returns the version-one lossless WebP profile.
    pub const fn lossless_v1() -> Self {
        Self {
            mode: WebPMode::Lossless,
        }
    }

    /// Creates a lossy WebP profile with explicit quality.
    pub fn lossy_v1(quality: u8) -> Result<Self, CodecError> {
        if quality > 100 {
            return Err(CodecError::new(CodecErrorCode::InvalidWebPQuality));
        }
        Ok(Self {
            mode: WebPMode::Lossy { quality },
        })
    }
}

/// Output-format-specific encoding controls.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum EncodeFormat {
    /// PNG controls.
    Png(PngOptions),
    /// JPEG controls.
    Jpeg(JpegOptions),
    /// WebP controls.
    WebP(WebPOptions),
}

/// Policy for optional metadata in a newly encoded file.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum MetadataPolicy {
    /// Do not write optional metadata.
    #[default]
    Strip,
    /// Write supported metadata from the source asset.
    Preserve,
}

/// Resource ceiling for encoding output.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct EncodeLimits {
    max_output_bytes: usize,
}

impl EncodeLimits {
    /// Creates a non-zero encoded-output byte ceiling.
    pub fn new(max_output_bytes: usize) -> Result<Self, CodecError> {
        if max_output_bytes == 0 {
            return Err(CodecError::new(CodecErrorCode::InvalidLimits));
        }
        Ok(Self { max_output_bytes })
    }

    /// Returns the encoded-output byte ceiling.
    pub const fn max_output_bytes(self) -> usize {
        self.max_output_bytes
    }
}

impl Default for EncodeLimits {
    fn default() -> Self {
        Self {
            max_output_bytes: 64 * 1024 * 1024,
        }
    }
}

/// Complete policy for one encoding operation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct EncodeOptions {
    format: EncodeFormat,
    metadata: MetadataPolicy,
    limits: EncodeLimits,
}

impl EncodeOptions {
    /// Creates options for a selected output format.
    pub const fn new(format: EncodeFormat) -> Self {
        Self {
            format,
            metadata: MetadataPolicy::Strip,
            limits: EncodeLimits {
                max_output_bytes: 64 * 1024 * 1024,
            },
        }
    }

    /// Sets metadata policy.
    pub const fn with_metadata_policy(mut self, metadata: MetadataPolicy) -> Self {
        self.metadata = metadata;
        self
    }

    /// Sets the encoded-output resource ceiling.
    pub const fn with_limits(mut self, limits: EncodeLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Returns the selected output format controls.
    pub const fn format(&self) -> EncodeFormat {
        self.format
    }
}

/// Result details for a completed encoding operation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct EncodeReport {
    format: EncodedFormat,
    output_bytes: usize,
}

impl EncodeReport {
    /// Returns the encoded file format.
    pub const fn format(self) -> EncodedFormat {
        self.format
    }

    /// Returns the number of bytes written.
    pub const fn output_bytes(self) -> usize {
        self.output_bytes
    }
}

/// In-memory output from [`ImageCodec::encode`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncodedImage {
    bytes: Vec<u8>,
    report: EncodeReport,
}

impl EncodedImage {
    /// Borrows encoded bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Consumes the encoded image and returns its bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// Returns encoding result details.
    pub const fn report(&self) -> EncodeReport {
        self.report
    }
}

/// Stateless codec facade for common image-format parsing and encoding.
pub struct ImageCodec;

impl ImageCodec {
    /// Decodes PNG, JPEG, or WebP bytes using [`CodecLimits::default`].
    pub fn decode(bytes: &[u8]) -> Result<ImageAsset, CodecError> {
        Self::decode_with_limits(bytes, CodecLimits::default())
    }

    /// Decodes an APNG or animated WebP using [`AnimationLimits::default`].
    pub fn decode_animated(bytes: &[u8]) -> Result<AnimatedImageAsset, CodecError> {
        Self::decode_animated_with_limits(bytes, AnimationLimits::default())
    }

    /// Detects and decodes an APNG or animated WebP subject to animation limits.
    pub fn decode_animated_with_limits(
        bytes: &[u8],
        limits: AnimationLimits,
    ) -> Result<AnimatedImageAsset, CodecError> {
        if bytes.len() > limits.codec.max_input_bytes {
            return Err(CodecError::new(CodecErrorCode::InputTooLarge));
        }
        let reader = ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|_| CodecError::new(CodecErrorCode::DecodeFailed))?;
        match reader.format() {
            Some(ImageFormat::Png) => png::decode_animated(bytes, limits),
            Some(ImageFormat::WebP) => webp::decode_animated(bytes, limits),
            Some(ImageFormat::Jpeg) => Err(CodecError::new(CodecErrorCode::NotAnimated)),
            _ => Err(CodecError::new(CodecErrorCode::UnsupportedFormat)),
        }
    }

    /// Detects and decodes PNG, JPEG, or WebP bytes subject to `limits`.
    pub fn decode_with_limits(bytes: &[u8], limits: CodecLimits) -> Result<ImageAsset, CodecError> {
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
        let mut decoder = reader.into_decoder().map_err(map_decode_error)?;
        let icc_profile = decoder.icc_profile().map_err(map_decode_error)?;
        let exif_tiff = decoder.exif_metadata().map_err(map_decode_error)?;
        let decoded = DynamicImage::from_decoder(decoder).map_err(map_decode_error)?;
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

        let color_space = match icc_profile {
            Some(profile) => ColorSpace::from_icc_profile(profile)
                .map_err(|_| CodecError::new(CodecErrorCode::DecodeFailed))?,
            None => ColorSpace::Srgb,
        };
        let image = Image::from_rgba8_with_color_space(
            width,
            height,
            decoded.into_rgba8().into_raw(),
            color_space,
        )
        .map_err(|_| CodecError::new(CodecErrorCode::DecodeFailed))?;
        Ok(ImageAsset::with_metadata(
            image,
            ImageMetadata { exif_tiff },
        ))
    }

    /// Encodes an image asset to an in-memory file according to explicit options.
    pub fn encode(asset: &ImageAsset, options: &EncodeOptions) -> Result<EncodedImage, CodecError> {
        let mut bytes = Vec::new();
        let report = Self::encode_to(&mut bytes, asset, options)?;
        Ok(EncodedImage { bytes, report })
    }

    /// Encodes a multi-frame asset as APNG.
    ///
    /// Animated WebP encoding currently returns
    /// [`CodecErrorCode::UnsupportedEncodeOption`].
    pub fn encode_animated(
        asset: &AnimatedImageAsset,
        options: &EncodeOptions,
    ) -> Result<EncodedImage, CodecError> {
        let mut bytes = Vec::new();
        let report = Self::encode_animated_to(&mut bytes, asset, options)?;
        Ok(EncodedImage { bytes, report })
    }

    /// Encodes an image asset to a writer according to explicit options.
    pub fn encode_to<W: Write>(
        mut writer: W,
        asset: &ImageAsset,
        options: &EncodeOptions,
    ) -> Result<EncodeReport, CodecError> {
        let mut limited = LimitedWriter::new(&mut writer, options.limits.max_output_bytes);
        let (result, format) = match options.format {
            EncodeFormat::Png(png) => (
                png::encode(&mut limited, asset, options.metadata, png),
                EncodedFormat::Png,
            ),
            EncodeFormat::Jpeg(jpeg) => (
                encode_jpeg(&mut limited, asset, options.metadata, jpeg),
                EncodedFormat::Jpeg,
            ),
            EncodeFormat::WebP(webp) => (
                webp::encode(&mut limited, asset, options.metadata, webp),
                EncodedFormat::WebP,
            ),
        };
        if limited.exceeded {
            return Err(CodecError::new(CodecErrorCode::OutputTooLarge));
        }
        result?;
        Ok(EncodeReport {
            format,
            output_bytes: limited.written,
        })
    }

    /// Encodes a multi-frame asset to a writer as APNG.
    ///
    /// Animated WebP encoding currently returns
    /// [`CodecErrorCode::UnsupportedEncodeOption`].
    pub fn encode_animated_to<W: Write>(
        mut writer: W,
        asset: &AnimatedImageAsset,
        options: &EncodeOptions,
    ) -> Result<EncodeReport, CodecError> {
        let mut limited = LimitedWriter::new(&mut writer, options.limits.max_output_bytes);
        let (result, format) = match options.format {
            EncodeFormat::Png(png) => (
                png::encode_animated(&mut limited, asset, options.metadata, png),
                EncodedFormat::Png,
            ),
            EncodeFormat::Jpeg(_) => (
                Err(CodecError::new(CodecErrorCode::UnsupportedEncodeOption)),
                EncodedFormat::Jpeg,
            ),
            EncodeFormat::WebP(_) => (
                Err(CodecError::new(CodecErrorCode::UnsupportedEncodeOption)),
                EncodedFormat::WebP,
            ),
        };
        if limited.exceeded {
            return Err(CodecError::new(CodecErrorCode::OutputTooLarge));
        }
        result?;
        Ok(EncodeReport {
            format,
            output_bytes: limited.written,
        })
    }
}

fn map_decode_error(error: ImageError) -> CodecError {
    match error {
        ImageError::Limits(_) => CodecError::new(CodecErrorCode::ImageTooLarge),
        _ => CodecError::new(CodecErrorCode::DecodeFailed),
    }
}

fn encode_jpeg<W: Write>(
    writer: W,
    asset: &ImageAsset,
    metadata: MetadataPolicy,
    options: JpegOptions,
) -> Result<(), CodecError> {
    let rgb8 = rgba8_to_rgb8(asset.image.pixels(), options.alpha)?;
    jpeg::encode(
        writer,
        &rgb8,
        asset.image.width(),
        asset.image.height(),
        options,
        asset.image.color_space().icc_profile(),
        if metadata == MetadataPolicy::Preserve {
            asset.metadata.exif_tiff()
        } else {
            None
        },
    )
}

fn apply_metadata<E: ImageEncoder>(
    encoder: &mut E,
    asset: &ImageAsset,
    metadata: MetadataPolicy,
) -> Result<(), CodecError> {
    if let Some(icc_profile) = asset.image.color_space().icc_profile() {
        encoder
            .set_icc_profile(icc_profile.to_vec())
            .map_err(|_| CodecError::new(CodecErrorCode::EncodeFailed))?;
    }
    if metadata == MetadataPolicy::Preserve
        && let Some(exif_tiff) = asset.metadata.exif_tiff()
    {
        encoder
            .set_exif_metadata(exif_tiff.to_vec())
            .map_err(|_| CodecError::new(CodecErrorCode::EncodeFailed))?;
    }
    Ok(())
}

fn rgba8_to_rgb8(rgba8: &[u8], alpha: JpegAlphaHandling) -> Result<Vec<u8>, CodecError> {
    let mut rgb8 = Vec::with_capacity(rgba8.len() / 4 * 3);
    for pixel in rgba8.chunks_exact(4) {
        let [red, green, blue, opacity] = pixel else {
            unreachable!("RGBA8 pixels are four bytes")
        };
        if *opacity == 255 {
            rgb8.extend_from_slice(&[*red, *green, *blue]);
        } else if let JpegAlphaHandling::Flatten(background) = alpha {
            rgb8.extend([
                flatten_component(*red, *opacity, background[0]),
                flatten_component(*green, *opacity, background[1]),
                flatten_component(*blue, *opacity, background[2]),
            ]);
        } else {
            return Err(CodecError::new(CodecErrorCode::TransparentJpeg));
        }
    }
    Ok(rgb8)
}

fn flatten_component(source: u8, alpha: u8, background: u8) -> u8 {
    ((u16::from(source) * u16::from(alpha)
        + u16::from(background) * (u16::from(u8::MAX) - u16::from(alpha))
        + 127)
        / u16::from(u8::MAX)) as u8
}

fn is_tiff_payload(bytes: &[u8]) -> bool {
    matches!(bytes, [b'I', b'I', 42, 0, ..] | [b'M', b'M', 0, 42, ..])
}

struct LimitedWriter<W> {
    writer: W,
    maximum: usize,
    written: usize,
    exceeded: bool,
}

impl<W> LimitedWriter<W> {
    fn new(writer: W, maximum: usize) -> Self {
        Self {
            writer,
            maximum,
            written: 0,
            exceeded: false,
        }
    }
}

impl<W: Write> Write for LimitedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let available = self.maximum.saturating_sub(self.written);
        if bytes.len() > available {
            self.exceeded = true;
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "encoded output limit",
            ));
        }
        let written = self.writer.write(bytes)?;
        self.written = self.written.saturating_add(written);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use image::{DynamicImage, ImageFormat, RgbaImage};
    use skia_image::{ColorSpace, Image};

    use super::{
        CodecErrorCode, CodecLimits, EncodeFormat, EncodeLimits, EncodeOptions, ImageAsset,
        ImageCodec, ImageMetadata, JpegAlphaHandling, JpegOptimization, JpegOptions, JpegScan,
        JpegSubsampling, MetadataPolicy, PngCompression, PngFilter, PngOptions, WebPOptions,
    };

    fn encoded(format: ImageFormat) -> Vec<u8> {
        let source = RgbaImage::from_raw(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 128]).unwrap();
        let mut bytes = Vec::new();
        DynamicImage::ImageRgba8(source)
            .write_to(&mut Cursor::new(&mut bytes), format)
            .unwrap();
        bytes
    }

    fn opaque_asset() -> ImageAsset {
        ImageAsset::new(Image::from_rgba8(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]).unwrap())
    }

    fn jpeg_test_asset() -> ImageAsset {
        let width = 17;
        let height = 17;
        let mut rgba8 = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            for x in 0..width {
                rgba8.extend([(x * 13) as u8, (y * 11) as u8, ((x + y) * 7) as u8, 255]);
            }
        }
        ImageAsset::new(Image::from_rgba8(width as u32, height as u32, rgba8).unwrap())
    }

    fn jpeg_frame(bytes: &[u8]) -> (u8, u8) {
        assert!(bytes.starts_with(&[0xff, 0xd8]));
        let mut offset = 2;
        while offset < bytes.len() {
            while bytes.get(offset) == Some(&0xff) {
                offset += 1;
            }
            let marker = bytes[offset];
            offset += 1;
            if marker == 0xd9 || marker == 0xda {
                break;
            }
            if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
                continue;
            }
            let length = usize::from(u16::from_be_bytes([bytes[offset], bytes[offset + 1]]));
            assert!(length >= 2);
            let payload = offset + 2;
            if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
                assert_eq!(bytes[payload + 5], 3);
                return (marker, bytes[payload + 7]);
            }
            offset += length;
        }
        panic!("JPEG has no start-of-frame marker");
    }

    #[test]
    fn decodes_png_jpeg_and_webp_to_assets() {
        for format in [ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::WebP] {
            let asset = ImageCodec::decode(&encoded(format)).unwrap();
            assert_eq!((asset.image().width(), asset.image().height()), (2, 1));
            assert_eq!(asset.image().pixel_at(0, 0).unwrap()[3], 255);
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
    fn encodes_new_png_jpeg_and_lossless_webp_profiles() {
        let asset = opaque_asset();
        let options = [
            EncodeOptions::new(EncodeFormat::Png(
                PngOptions::balanced_v1()
                    .with_compression(PngCompression::DeflateLevel(6))
                    .with_filter(PngFilter::Paeth),
            )),
            EncodeOptions::new(EncodeFormat::Jpeg(JpegOptions::baseline_v1(90).unwrap())),
            EncodeOptions::new(EncodeFormat::WebP(WebPOptions::lossless_v1())),
        ];
        for option in options {
            let encoded = ImageCodec::encode(&asset, &option).unwrap();
            let decoded = ImageCodec::decode(encoded.bytes()).unwrap();
            assert_eq!((decoded.image().width(), decoded.image().height()), (2, 1));
        }
    }

    #[test]
    fn jpeg_requires_explicit_alpha_flattening() {
        let transparent = ImageAsset::new(Image::from_rgba8(1, 1, vec![255, 0, 0, 128]).unwrap());
        let option = EncodeOptions::new(EncodeFormat::Jpeg(JpegOptions::baseline_v1(90).unwrap()));
        assert_eq!(
            ImageCodec::encode(&transparent, &option)
                .unwrap_err()
                .code(),
            CodecErrorCode::TransparentJpeg
        );
        let option = EncodeOptions::new(EncodeFormat::Jpeg(
            JpegOptions::baseline_v1(90)
                .unwrap()
                .with_alpha_handling(JpegAlphaHandling::Flatten([255, 255, 255])),
        ));
        assert!(
            !ImageCodec::encode(&transparent, &option)
                .unwrap()
                .bytes()
                .is_empty()
        );
    }

    #[test]
    fn jpeg_encodes_all_public_subsampling_and_scan_modes() {
        let asset = jpeg_test_asset();
        for (subsampling, expected_luma_sampling) in [
            (JpegSubsampling::Yuv444, 0x11),
            (JpegSubsampling::Yuv422, 0x21),
            (JpegSubsampling::Yuv420, 0x22),
        ] {
            let options = EncodeOptions::new(EncodeFormat::Jpeg(
                JpegOptions::baseline_v1(85)
                    .unwrap()
                    .with_subsampling(subsampling),
            ));
            let encoded = ImageCodec::encode(&asset, &options).unwrap();
            assert_eq!(jpeg_frame(encoded.bytes()), (0xc0, expected_luma_sampling));
        }

        let progressive = EncodeOptions::new(EncodeFormat::Jpeg(JpegOptions::web_v1()));
        let encoded = ImageCodec::encode(&asset, &progressive).unwrap();
        assert_eq!(jpeg_frame(encoded.bytes()), (0xc2, 0x22));
    }

    #[test]
    fn jpeg_optimization_profiles_honor_requested_scan_mode() {
        let asset = jpeg_test_asset();
        for optimization in [
            JpegOptimization::Fast,
            JpegOptimization::Balanced,
            JpegOptimization::Smallest,
        ] {
            for (scan, expected_marker) in
                [(JpegScan::Baseline, 0xc0), (JpegScan::Progressive, 0xc2)]
            {
                let options = EncodeOptions::new(EncodeFormat::Jpeg(
                    JpegOptions::baseline_v1(80)
                        .unwrap()
                        .with_scan(scan)
                        .with_optimization(optimization),
                ));
                let encoded = ImageCodec::encode(&asset, &options).unwrap();
                assert_eq!(jpeg_frame(encoded.bytes()).0, expected_marker);
            }
        }
    }

    #[test]
    fn preserves_valid_exif_and_icc_when_requested() {
        let image = Image::from_rgba8_with_color_space(
            1,
            1,
            vec![0, 0, 0, 255],
            ColorSpace::from_icc_profile(vec![1, 2, 3]).unwrap(),
        )
        .unwrap();
        let metadata = ImageMetadata::new()
            .with_exif_tiff(vec![b'I', b'I', 42, 0, 8, 0, 0, 0])
            .unwrap();
        let asset = ImageAsset::with_metadata(image, metadata);
        let options = [
            EncodeOptions::new(EncodeFormat::Png(PngOptions::balanced_v1())),
            EncodeOptions::new(EncodeFormat::Jpeg(JpegOptions::baseline_v1(90).unwrap())),
            EncodeOptions::new(EncodeFormat::WebP(WebPOptions::lossless_v1())),
        ];
        for options in options {
            let options = options.with_metadata_policy(MetadataPolicy::Preserve);
            let decoded =
                ImageCodec::decode(ImageCodec::encode(&asset, &options).unwrap().bytes()).unwrap();
            assert_eq!(
                decoded.image().color_space().icc_profile(),
                Some(&[1, 2, 3][..])
            );
            assert_eq!(
                decoded.metadata().exif_tiff(),
                Some(&[b'I', b'I', 42, 0, 8, 0, 0, 0][..])
            );
        }
    }

    #[test]
    fn rejects_invalid_options_and_limits() {
        assert_eq!(
            JpegOptions::baseline_v1(0).unwrap_err().code(),
            CodecErrorCode::InvalidJpegQuality
        );
        let option = EncodeOptions::new(EncodeFormat::Png(
            PngOptions::balanced_v1().with_compression(PngCompression::DeflateLevel(10)),
        ));
        assert_eq!(
            ImageCodec::encode(&opaque_asset(), &option)
                .unwrap_err()
                .code(),
            CodecErrorCode::InvalidPngCompressionLevel
        );
        let option = EncodeOptions::new(EncodeFormat::Png(PngOptions::balanced_v1()))
            .with_limits(EncodeLimits::new(1).unwrap());
        assert_eq!(
            ImageCodec::encode(&opaque_asset(), &option)
                .unwrap_err()
                .code(),
            CodecErrorCode::OutputTooLarge
        );
    }

    #[test]
    fn animation_rejects_still_webp_and_unavailable_webp_encoding() {
        assert_eq!(
            ImageCodec::decode_animated(&encoded(ImageFormat::WebP))
                .expect_err("still WebP is not animated")
                .code(),
            CodecErrorCode::NotAnimated
        );

        let frame = super::AnimationFrame::new(
            Image::from_rgba8(1, 1, vec![0, 0, 0, 255]).expect("frame image"),
            super::FrameDuration::from_millis(10),
        );
        let animation =
            super::AnimatedImageAsset::new(1, 1, vec![frame], super::AnimationLoop::Infinite)
                .expect("animation");
        let options = EncodeOptions::new(EncodeFormat::WebP(WebPOptions::lossless_v1()));
        assert_eq!(
            ImageCodec::encode_animated(&animation, &options)
                .expect_err("animated WebP encoder is unavailable")
                .code(),
            CodecErrorCode::UnsupportedEncodeOption
        );
    }
}
