use std::io::{Cursor, Write};

use image::{
    AnimationDecoder, ExtendedColorType, ImageDecoder, ImageEncoder, Limits,
    codecs::webp::{WebPDecoder, WebPEncoder},
    metadata::LoopCount,
};
use skia_image::{ColorSpace, Image};

use super::apply_metadata;
use crate::{
    AnimatedImageAsset, AnimationFrame, AnimationLimits, AnimationLoop, CodecError, CodecErrorCode,
    FrameDuration, ImageAsset, ImageMetadata, MetadataPolicy, WebPMode, WebPOptions,
};

pub(crate) fn encode<W: Write>(
    writer: W,
    asset: &ImageAsset,
    metadata: MetadataPolicy,
    options: WebPOptions,
) -> Result<(), CodecError> {
    if !matches!(options.mode, WebPMode::Lossless) {
        return Err(CodecError::new(CodecErrorCode::UnsupportedEncodeOption));
    }
    let mut encoder = WebPEncoder::new_lossless(writer);
    apply_metadata(&mut encoder, asset, metadata)?;
    let image = asset
        .image
        .to_straight_rgba8()
        .map_err(|_| CodecError::new(CodecErrorCode::EncodeFailed))?;
    encoder
        .write_image(
            image.pixels(),
            image.width(),
            image.height(),
            ExtendedColorType::Rgba8,
        )
        .map_err(|_| CodecError::new(CodecErrorCode::EncodeFailed))
}

/// Decodes animated WebP as complete, composited canvas frames.
///
/// `image` exposes WebP animation frames after blending and disposal have been
/// applied, so every retained frame covers the animation canvas and uses the
/// default source/keep composition semantics.
pub(crate) fn decode_animated(
    bytes: &[u8],
    limits: AnimationLimits,
) -> Result<AnimatedImageAsset, CodecError> {
    let mut decoder = WebPDecoder::new(Cursor::new(bytes))
        .map_err(|_| CodecError::new(CodecErrorCode::DecodeFailed))?;
    if !decoder.has_animation() {
        return Err(CodecError::new(CodecErrorCode::NotAnimated));
    }
    let (width, height) = decoder.dimensions();
    validate_canvas(width, height, limits)?;
    let mut decoder_limits = Limits::default();
    decoder_limits.max_image_width = Some(width);
    decoder_limits.max_image_height = Some(height);
    decoder_limits.max_alloc = Some(limits.codec.max_decoded_bytes);
    decoder
        .set_limits(decoder_limits)
        .map_err(|_| CodecError::new(CodecErrorCode::AnimationTooLarge))?;
    let color_space = match decoder
        .icc_profile()
        .map_err(|_| CodecError::new(CodecErrorCode::DecodeFailed))?
    {
        Some(profile) => ColorSpace::from_icc_profile(profile)
            .map_err(|_| CodecError::new(CodecErrorCode::UnsupportedColorProfile))?,
        None => ColorSpace::Srgb,
    };
    let exif_tiff = decoder
        .exif_metadata()
        .map_err(|_| CodecError::new(CodecErrorCode::DecodeFailed))?;
    let loop_count = map_loop_count(decoder.loop_count());
    let mut frames = Vec::new();
    let mut total_bytes = 0u64;
    for frame in decoder.into_frames() {
        if frames.len() >= limits.max_frames as usize {
            return Err(CodecError::new(CodecErrorCode::AnimationTooLarge));
        }
        let frame = frame.map_err(|_| CodecError::new(CodecErrorCode::DecodeFailed))?;
        let (numerator_ms, denominator) = frame.delay().numer_denom_ms();
        let buffer = frame.into_buffer();
        total_bytes = total_bytes
            .checked_add(buffer.len() as u64)
            .ok_or(CodecError::new(CodecErrorCode::AnimationTooLarge))?;
        if total_bytes > limits.max_total_decoded_bytes {
            return Err(CodecError::new(CodecErrorCode::AnimationTooLarge));
        }
        let image = Image::from_rgba8_with_color_space(
            buffer.width(),
            buffer.height(),
            buffer.into_raw(),
            color_space.clone(),
        )
        .map_err(|_| CodecError::new(CodecErrorCode::DecodeFailed))?;
        frames.push(AnimationFrame::new(
            image,
            FrameDuration::new(numerator_ms, denominator)?,
        ));
    }
    AnimatedImageAsset::new(width, height, frames, loop_count)
        .map(|asset| asset.with_metadata(ImageMetadata { exif_tiff }))
}

fn validate_canvas(width: u32, height: u32, limits: AnimationLimits) -> Result<(), CodecError> {
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or(CodecError::new(CodecErrorCode::AnimationTooLarge))?;
    let bytes = pixels
        .checked_mul(4)
        .ok_or(CodecError::new(CodecErrorCode::AnimationTooLarge))?;
    if pixels > limits.codec.max_pixels || bytes > limits.codec.max_decoded_bytes {
        return Err(CodecError::new(CodecErrorCode::AnimationTooLarge));
    }
    Ok(())
}

fn map_loop_count(loop_count: LoopCount) -> AnimationLoop {
    match loop_count {
        LoopCount::Infinite => AnimationLoop::Infinite,
        LoopCount::Finite(count) => AnimationLoop::Finite(count.get()),
    }
}
