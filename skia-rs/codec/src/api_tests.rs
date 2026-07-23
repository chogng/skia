use std::{borrow::Cow, io::Cursor};

use image::{DynamicImage, ImageFormat, RgbaImage};
use skia_image::{ColorSpace, Image};

use super::{
    CodecErrorCode, CodecLimits, EncodeFormat, EncodeLimits, EncodeOptions, ImageAsset, ImageCodec,
    ImageMetadata, JpegAlphaHandling, JpegOptimization, JpegOptions, JpegScan, JpegSubsampling,
    MetadataPolicy, PngCompression, PngFilter, PngOptions, WebPOptions,
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
fn decoder_reports_unsupported_embedded_profile() {
    let mut info = png::Info::with_size(1, 1);
    info.color_type = png::ColorType::Rgba;
    info.bit_depth = png::BitDepth::Eight;
    info.icc_profile = Some(Cow::Owned(vec![1, 2, 3]));
    let mut bytes = Vec::new();
    let mut writer = png::Encoder::with_info(&mut bytes, info)
        .unwrap()
        .write_header()
        .unwrap();
    writer.write_image_data(&[0, 0, 0, 255]).unwrap();
    writer.finish().unwrap();

    assert_eq!(
        ImageCodec::decode(&bytes).unwrap_err().code(),
        CodecErrorCode::UnsupportedColorProfile
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
        for (scan, expected_marker) in [(JpegScan::Baseline, 0xc0), (JpegScan::Progressive, 0xc2)] {
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
    let profile = moxcms::ColorProfile::new_display_p3().encode().unwrap();
    let image = Image::from_rgba8_with_color_space(
        1,
        1,
        vec![0, 0, 0, 255],
        ColorSpace::from_icc_profile(profile.clone()).unwrap(),
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
            Some(profile.as_slice())
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
