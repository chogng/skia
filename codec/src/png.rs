use std::{
    borrow::Cow,
    io::{Cursor, Write},
};

use image::{
    AnimationDecoder, ExtendedColorType, ImageDecoder, ImageEncoder, Limits,
    codecs::png::{CompressionType, FilterType, PngEncoder},
    metadata::LoopCount,
};
use png::{BitDepth, BlendOp, ColorType, DeflateCompression, DisposeOp, Encoder, Filter, Info};
use skia_image::{ColorSpace, Image};

use crate::{
    AnimatedImageAsset, AnimationBlend, AnimationDisposal, AnimationFrame, AnimationLimits,
    AnimationLoop, CodecError, CodecErrorCode, FrameDuration, ImageAsset, ImageMetadata,
    MetadataPolicy, PngCompression, PngFilter, PngOptions, apply_metadata,
};

pub(crate) fn encode<W: Write>(
    writer: W,
    asset: &ImageAsset,
    metadata: MetadataPolicy,
    options: PngOptions,
) -> Result<(), CodecError> {
    let compression = match options.compression {
        PngCompression::Fast => CompressionType::Fast,
        PngCompression::Balanced => CompressionType::Default,
        PngCompression::Best => CompressionType::Best,
        PngCompression::Uncompressed => CompressionType::Uncompressed,
        PngCompression::DeflateLevel(level @ 0..=9) => CompressionType::Level(level),
        PngCompression::DeflateLevel(_) => {
            return Err(CodecError::new(CodecErrorCode::InvalidPngCompressionLevel));
        }
    };
    let filter = match options.filter {
        PngFilter::Adaptive => FilterType::Adaptive,
        PngFilter::None => FilterType::NoFilter,
        PngFilter::Sub => FilterType::Sub,
        PngFilter::Up => FilterType::Up,
        PngFilter::Average => FilterType::Avg,
        PngFilter::Paeth => FilterType::Paeth,
    };
    let mut encoder = PngEncoder::new_with_quality(writer, compression, filter);
    apply_metadata(&mut encoder, asset, metadata)?;
    encoder
        .write_image(
            asset.image.pixels(),
            asset.image.width(),
            asset.image.height(),
            ExtendedColorType::Rgba8,
        )
        .map_err(|_| CodecError::new(CodecErrorCode::EncodeFailed))
}

pub(crate) fn decode_animated(
    bytes: &[u8],
    limits: AnimationLimits,
) -> Result<AnimatedImageAsset, CodecError> {
    let mut decoder = image::codecs::png::PngDecoder::new(Cursor::new(bytes))
        .map_err(|_| CodecError::new(CodecErrorCode::DecodeFailed))?;
    if !decoder
        .is_apng()
        .map_err(|_| CodecError::new(CodecErrorCode::DecodeFailed))?
    {
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
    let icc_profile = decoder
        .icc_profile()
        .map_err(|_| CodecError::new(CodecErrorCode::DecodeFailed))?;
    let exif_tiff = decoder
        .exif_metadata()
        .map_err(|_| CodecError::new(CodecErrorCode::DecodeFailed))?;
    let color_space = match icc_profile {
        Some(profile) => ColorSpace::from_icc_profile(profile)
            .map_err(|_| CodecError::new(CodecErrorCode::DecodeFailed))?,
        None => ColorSpace::Srgb,
    };
    let animation = decoder
        .apng()
        .map_err(|_| CodecError::new(CodecErrorCode::DecodeFailed))?;
    let loop_count = map_loop_count(animation.loop_count());
    let mut frames = Vec::new();
    let mut total_bytes = 0u64;
    for frame in animation.into_frames() {
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

pub(crate) fn encode_animated<W: Write>(
    writer: W,
    asset: &AnimatedImageAsset,
    metadata: MetadataPolicy,
    options: PngOptions,
) -> Result<(), CodecError> {
    let frame_count = u32::try_from(asset.frames.len())
        .map_err(|_| CodecError::new(CodecErrorCode::InvalidAnimation))?;
    let num_plays = match asset.loop_count {
        AnimationLoop::Infinite => 0,
        AnimationLoop::Finite(count) => count,
    };
    let mut info = Info::with_size(asset.width, asset.height);
    info.color_type = ColorType::Rgba;
    info.bit_depth = BitDepth::Eight;
    if let Some(profile) = asset.frames[0].image.color_space().icc_profile() {
        info.icc_profile = Some(Cow::Owned(profile.to_vec()));
    }
    if metadata == MetadataPolicy::Preserve {
        info.exif_metadata = asset
            .metadata
            .exif_tiff()
            .map(|exif| Cow::Owned(exif.to_vec()));
    }
    let mut encoder = Encoder::with_info(writer, info)
        .map_err(|_| CodecError::new(CodecErrorCode::EncodeFailed))?;
    encoder
        .set_animated(frame_count, num_plays)
        .map_err(|_| CodecError::new(CodecErrorCode::InvalidAnimation))?;
    encoder.set_deflate_compression(map_compression(options.compression)?);
    encoder.set_filter(map_filter(options.filter));
    let mut writer = encoder
        .write_header()
        .map_err(|_| CodecError::new(CodecErrorCode::EncodeFailed))?;
    for frame in &asset.frames {
        let (delay_num, delay_den) = apng_delay(frame.duration)?;
        writer
            .set_frame_delay(delay_num, delay_den)
            .and_then(|_| writer.set_frame_position(frame.x, frame.y))
            .and_then(|_| writer.set_frame_dimension(frame.image.width(), frame.image.height()))
            .and_then(|_| {
                writer.set_blend_op(match frame.blend {
                    AnimationBlend::Source => BlendOp::Source,
                    AnimationBlend::Over => BlendOp::Over,
                })
            })
            .and_then(|_| {
                writer.set_dispose_op(match frame.disposal {
                    AnimationDisposal::Keep => DisposeOp::None,
                    AnimationDisposal::Background => DisposeOp::Background,
                    AnimationDisposal::Previous => DisposeOp::Previous,
                })
            })
            .map_err(|_| CodecError::new(CodecErrorCode::InvalidAnimation))?;
        writer
            .write_image_data(frame.image.pixels())
            .map_err(|_| CodecError::new(CodecErrorCode::EncodeFailed))?;
    }
    writer
        .finish()
        .map_err(|_| CodecError::new(CodecErrorCode::EncodeFailed))
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

fn map_compression(compression: PngCompression) -> Result<DeflateCompression, CodecError> {
    match compression {
        PngCompression::Fast => Ok(DeflateCompression::FdeflateUltraFast),
        PngCompression::Balanced => Ok(DeflateCompression::Level(6)),
        PngCompression::Best => Ok(DeflateCompression::Level(9)),
        PngCompression::Uncompressed => Ok(DeflateCompression::NoCompression),
        PngCompression::DeflateLevel(level @ 0..=9) => Ok(DeflateCompression::Level(level)),
        PngCompression::DeflateLevel(_) => {
            Err(CodecError::new(CodecErrorCode::InvalidPngCompressionLevel))
        }
    }
}

fn map_filter(filter: PngFilter) -> Filter {
    match filter {
        PngFilter::Adaptive => Filter::Adaptive,
        PngFilter::None => Filter::NoFilter,
        PngFilter::Sub => Filter::Sub,
        PngFilter::Up => Filter::Up,
        PngFilter::Average => Filter::Avg,
        PngFilter::Paeth => Filter::Paeth,
    }
}

fn apng_delay(duration: FrameDuration) -> Result<(u16, u16), CodecError> {
    let numerator = u64::from(duration.numerator_ms);
    let denominator = u64::from(duration.denominator)
        .checked_mul(1000)
        .ok_or(CodecError::new(CodecErrorCode::InvalidAnimation))?;
    let divisor = gcd(numerator, denominator);
    let numerator = u16::try_from(numerator / divisor)
        .map_err(|_| CodecError::new(CodecErrorCode::InvalidAnimation))?;
    let denominator = u16::try_from(denominator / divisor)
        .map_err(|_| CodecError::new(CodecErrorCode::InvalidAnimation))?;
    Ok((numerator, denominator))
}

fn gcd(mut left: u64, mut right: u64) -> u64 {
    while right != 0 {
        (left, right) = (right, left % right);
    }
    left.max(1)
}
