use std::io::Write;

use mozjpeg_rs::{Encoder, Preset, Subsampling};

use crate::{CodecError, CodecErrorCode, JpegOptimization, JpegOptions, JpegScan, JpegSubsampling};

pub(crate) fn encode<W: Write>(
    writer: W,
    rgb8: &[u8],
    width: u32,
    height: u32,
    options: JpegOptions,
    icc_profile: Option<&[u8]>,
    exif_tiff: Option<&[u8]>,
) -> Result<(), CodecError> {
    let preset = match (options.optimization, options.scan) {
        (JpegOptimization::Fast, _) => Preset::BaselineFastest,
        (JpegOptimization::Balanced, JpegScan::Baseline) => Preset::BaselineBalanced,
        (JpegOptimization::Balanced, JpegScan::Progressive) => Preset::ProgressiveBalanced,
        (JpegOptimization::Smallest, _) => Preset::ProgressiveSmallest,
    };
    let subsampling = match options.subsampling {
        JpegSubsampling::Yuv444 => Subsampling::S444,
        JpegSubsampling::Yuv422 => Subsampling::S422,
        JpegSubsampling::Yuv420 => Subsampling::S420,
    };
    let mut encoder = Encoder::new(preset)
        .quality(options.quality)
        .progressive(options.scan == JpegScan::Progressive)
        .force_baseline(true)
        .subsampling(subsampling);
    if let Some(profile) = icc_profile {
        encoder = encoder.icc_profile(profile.to_vec());
    }
    if let Some(exif) = exif_tiff {
        encoder = encoder.exif_data(exif.to_vec());
    }
    encoder
        .encode_rgb_to_writer(rgb8, width, height, writer)
        .map_err(|_| CodecError::new(CodecErrorCode::EncodeFailed))
}
