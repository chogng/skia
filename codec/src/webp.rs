use std::io::Write;

use image::{ExtendedColorType, ImageEncoder, codecs::webp::WebPEncoder};

use crate::{
    CodecError, CodecErrorCode, ImageAsset, MetadataPolicy, WebPMode, WebPOptions, apply_metadata,
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
    encoder
        .write_image(
            asset.image.pixels(),
            asset.image.width(),
            asset.image.height(),
            ExtendedColorType::Rgba8,
        )
        .map_err(|_| CodecError::new(CodecErrorCode::EncodeFailed))
}
