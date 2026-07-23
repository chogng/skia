use moxcms::ColorProfile;

use super::{AlphaType, ColorSpace, Image, ImageErrorCode, ImageInfo, PixelFormat, premultiply};

#[test]
fn padded_bgra_premultiplied_storage_has_logical_rgba_pixels() {
    let info = ImageInfo::new(
        1,
        2,
        PixelFormat::Bgra8888,
        AlphaType::Premultiplied,
        ColorSpace::Srgb,
    )
    .unwrap();
    let image = Image::from_pixels(
        info,
        8,
        vec![
            premultiply(30, 128),
            premultiply(20, 128),
            premultiply(10, 128),
            128,
            9,
            9,
            9,
            9,
            0,
            0,
            0,
            0,
            8,
            8,
            8,
            8,
        ],
    )
    .unwrap();

    assert_eq!(image.row_bytes(), 8);
    assert_eq!(image.pixel_at(0, 0), Some([10, 20, 30, 128]));
    assert_eq!(image.pixel_at(0, 1), Some([0, 0, 0, 0]));
}

#[test]
fn alpha_declarations_are_validated() {
    let opaque = ImageInfo::new(
        1,
        1,
        PixelFormat::Rgba8888,
        AlphaType::Opaque,
        ColorSpace::Srgb,
    )
    .unwrap();
    assert_eq!(
        Image::from_pixels(opaque, 4, vec![1, 2, 3, 254])
            .unwrap_err()
            .code(),
        ImageErrorCode::InvalidAlpha
    );

    let premultiplied = ImageInfo::new(
        1,
        1,
        PixelFormat::Rgba8888,
        AlphaType::Premultiplied,
        ColorSpace::Srgb,
    )
    .unwrap();
    assert_eq!(
        Image::from_pixels(premultiplied, 4, vec![129, 0, 0, 128])
            .unwrap_err()
            .code(),
        ImageErrorCode::InvalidAlpha
    );
}

#[test]
fn linear_srgb_conversion_changes_encoded_samples_and_preserves_alpha() {
    let image =
        Image::from_rgba8_with_color_space(1, 1, vec![128, 128, 128, 91], ColorSpace::LinearSrgb)
            .unwrap();

    let converted = image.to_rendering_image().unwrap();

    let pixel = converted.pixel_at(0, 0).unwrap();
    assert!((pixel[0] as i16 - 188).abs() <= 1, "{pixel:?}");
    assert_eq!(pixel[0], pixel[1]);
    assert_eq!(pixel[1], pixel[2]);
    assert_eq!(pixel[3], 91);
    assert_eq!(converted.color_space(), &ColorSpace::Srgb);
}

#[test]
fn standard_srgb_icc_profile_is_embeddable_and_validated() {
    let profile = ColorSpace::srgb_icc_profile().unwrap();
    assert!(!profile.is_empty());
    assert!(ColorSpace::from_icc_profile(profile).is_ok());
}

#[test]
fn matrix_icc_conversion_is_applied() {
    let profile = ColorProfile::new_display_p3().encode().unwrap();
    let color_space = ColorSpace::from_icc_profile(profile).unwrap();
    let image =
        Image::from_rgba8_with_color_space(1, 1, vec![128, 200, 100, 255], color_space).unwrap();

    let converted = image.to_rendering_image().unwrap();

    assert_ne!(converted.pixel_at(0, 0), Some([128, 200, 100, 255]));
    assert_eq!(converted.color_space(), &ColorSpace::Srgb);
}

#[test]
fn malformed_icc_is_rejected_instead_of_assumed_srgb() {
    assert_eq!(
        ColorSpace::from_icc_profile(vec![1, 2, 3])
            .unwrap_err()
            .code(),
        ImageErrorCode::UnsupportedColorProfile
    );
    assert_eq!(
        ImageInfo::new(
            1,
            1,
            PixelFormat::Rgba8888,
            AlphaType::Straight,
            ColorSpace::Icc(vec![1, 2, 3]),
        )
        .unwrap_err()
        .code(),
        ImageErrorCode::UnsupportedColorProfile
    );
}

#[test]
fn format_and_alpha_conversion_is_not_reapplied() {
    let source = Image::from_rgba8(1, 1, vec![200, 100, 50, 128]).unwrap();
    let premultiplied = source
        .converted(
            PixelFormat::Bgra8888,
            AlphaType::Premultiplied,
            ColorSpace::Srgb,
        )
        .unwrap();
    assert_eq!(premultiplied.pixels(), &[25, 50, 100, 128]);

    let straight = premultiplied.to_straight_rgba8().unwrap();
    let pixel = straight.pixel_at(0, 0).unwrap();
    assert!((pixel[0] as i16 - 200).abs() <= 1);
    assert!((pixel[1] as i16 - 100).abs() <= 1);
    assert!((pixel[2] as i16 - 50).abs() <= 1);
    assert_eq!(pixel[3], 128);
}

#[test]
fn dimensions_stride_and_storage_length_are_checked() {
    assert_eq!(
        Image::from_rgba8(0, 1, Vec::new()).unwrap_err().code(),
        ImageErrorCode::InvalidDimensions
    );
    let info = ImageInfo::new(
        2,
        2,
        PixelFormat::Rgba8888,
        AlphaType::Straight,
        ColorSpace::Srgb,
    )
    .unwrap();
    assert_eq!(
        Image::from_pixels(info.clone(), 7, vec![0; 14])
            .unwrap_err()
            .code(),
        ImageErrorCode::InvalidRowBytes
    );
    assert_eq!(
        Image::from_pixels(info, 8, vec![0; 15]).unwrap_err().code(),
        ImageErrorCode::InvalidPixels
    );
}
