//! Decoding and encoding of PNG, JPEG, and WebP image assets.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod api;

pub use api::{
    AnimatedImageAsset, AnimationBlend, AnimationDisposal, AnimationFrame, AnimationLimits,
    AnimationLoop, CodecError, CodecErrorCode, CodecLimits, EncodeFormat, EncodeLimits,
    EncodeOptions, EncodeReport, EncodedFormat, EncodedImage, FrameDuration, ImageAsset,
    ImageCodec, ImageMetadata, JpegAlphaHandling, JpegOptimization, JpegOptions, JpegScan,
    JpegSubsampling, MetadataPolicy, PngCompression, PngFilter, PngOptions, WebPMode, WebPOptions,
};
