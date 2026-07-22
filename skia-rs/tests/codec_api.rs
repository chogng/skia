use skia::{
    CodecErrorCode, ColorSpace, EncodeFormat, EncodeLimits, EncodeOptions, EncodedFormat, Image,
    ImageAsset, ImageCodec, ImageMetadata, JpegAlphaHandling, JpegOptimization, JpegOptions,
    JpegScan, JpegSubsampling, MetadataPolicy, PngCompression, PngFilter, PngOptions, WebPOptions,
};

fn opaque_asset() -> ImageAsset {
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
fn public_codec_profiles_round_trip_through_the_facade() {
    let asset = opaque_asset();
    let profiles = [
        (
            EncodeOptions::new(EncodeFormat::Png(
                PngOptions::balanced_v1()
                    .with_compression(PngCompression::DeflateLevel(6))
                    .with_filter(PngFilter::Paeth),
            )),
            EncodedFormat::Png,
        ),
        (
            EncodeOptions::new(EncodeFormat::Jpeg(JpegOptions::web_v1())),
            EncodedFormat::Jpeg,
        ),
        (
            EncodeOptions::new(EncodeFormat::WebP(WebPOptions::lossless_v1())),
            EncodedFormat::WebP,
        ),
    ];

    for (options, expected_format) in profiles {
        let encoded = ImageCodec::encode(&asset, &options).unwrap();
        assert_eq!(encoded.report().format(), expected_format);
        assert_eq!(encoded.report().output_bytes(), encoded.bytes().len());
        let decoded = ImageCodec::decode(encoded.bytes()).unwrap();
        assert_eq!(
            (decoded.image().width(), decoded.image().height()),
            (17, 17)
        );
    }
}

#[test]
fn public_jpeg_options_control_the_encoded_file_structure() {
    let asset = opaque_asset();
    let profiles = [
        (
            JpegOptions::baseline_v1(85)
                .unwrap()
                .with_subsampling(JpegSubsampling::Yuv444)
                .with_optimization(JpegOptimization::Fast),
            (0xc0, 0x11),
        ),
        (
            JpegOptions::baseline_v1(85)
                .unwrap()
                .with_subsampling(JpegSubsampling::Yuv422)
                .with_optimization(JpegOptimization::Balanced),
            (0xc0, 0x21),
        ),
        (
            JpegOptions::baseline_v1(85)
                .unwrap()
                .with_subsampling(JpegSubsampling::Yuv420)
                .with_scan(JpegScan::Progressive)
                .with_optimization(JpegOptimization::Smallest),
            (0xc2, 0x22),
        ),
    ];

    for (jpeg, expected_frame) in profiles {
        let encoded =
            ImageCodec::encode(&asset, &EncodeOptions::new(EncodeFormat::Jpeg(jpeg))).unwrap();
        assert_eq!(jpeg_frame(encoded.bytes()), expected_frame);
    }
}

#[test]
fn public_metadata_policy_round_trips_exif_and_icc() {
    let image = Image::from_rgba8_with_color_space(
        1,
        1,
        vec![0, 0, 0, 255],
        ColorSpace::from_icc_profile(vec![1, 2, 3]).unwrap(),
    )
    .unwrap();
    let exif = [b'I', b'I', 42, 0, 8, 0, 0, 0];
    let metadata = ImageMetadata::new().with_exif_tiff(exif.to_vec()).unwrap();
    let asset = ImageAsset::with_metadata(image, metadata);
    let profiles = [
        EncodeOptions::new(EncodeFormat::Png(PngOptions::balanced_v1())),
        EncodeOptions::new(EncodeFormat::Jpeg(JpegOptions::baseline_v1(90).unwrap())),
        EncodeOptions::new(EncodeFormat::WebP(WebPOptions::lossless_v1())),
    ];

    for options in profiles {
        let options = options.with_metadata_policy(MetadataPolicy::Preserve);
        let encoded = ImageCodec::encode(&asset, &options).unwrap();
        let decoded = ImageCodec::decode(encoded.bytes()).unwrap();
        assert_eq!(
            decoded.image().color_space().icc_profile(),
            Some(&[1, 2, 3][..])
        );
        assert_eq!(decoded.metadata().exif_tiff(), Some(&exif[..]));
    }
}

#[test]
fn public_codec_contract_fails_closed_for_unsupported_or_unsafe_requests() {
    let transparent = ImageAsset::new(Image::from_rgba8(1, 1, vec![255, 0, 0, 128]).unwrap());
    let jpeg = EncodeOptions::new(EncodeFormat::Jpeg(JpegOptions::baseline_v1(85).unwrap()));
    assert_eq!(
        ImageCodec::encode(&transparent, &jpeg).unwrap_err().code(),
        CodecErrorCode::TransparentJpeg
    );

    let flattened = EncodeOptions::new(EncodeFormat::Jpeg(
        JpegOptions::baseline_v1(85)
            .unwrap()
            .with_alpha_handling(JpegAlphaHandling::Flatten([255, 255, 255])),
    ));
    assert!(ImageCodec::encode(&transparent, &flattened).is_ok());

    let lossy_webp = EncodeOptions::new(EncodeFormat::WebP(WebPOptions::lossy_v1(80).unwrap()));
    assert_eq!(
        ImageCodec::encode(&opaque_asset(), &lossy_webp)
            .unwrap_err()
            .code(),
        CodecErrorCode::UnsupportedEncodeOption
    );

    let tiny_output = EncodeOptions::new(EncodeFormat::Png(PngOptions::balanced_v1()))
        .with_limits(EncodeLimits::new(1).unwrap());
    assert_eq!(
        ImageCodec::encode(&opaque_asset(), &tiny_output)
            .unwrap_err()
            .code(),
        CodecErrorCode::OutputTooLarge
    );
}
