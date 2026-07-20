use std::io::Write;

use image::{
    ExtendedColorType, ImageEncoder,
    codecs::png::{CompressionType, FilterType, PngEncoder},
};

use crate::{
    CodecError, CodecErrorCode, ImageAsset, MetadataPolicy, PngCompression, PngFilter, PngOptions,
    apply_metadata,
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
