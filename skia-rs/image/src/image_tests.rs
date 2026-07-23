use super::{Image, ImageErrorCode};

#[test]
fn image_owns_exact_rgba8_pixels() {
    let image = Image::from_rgba8(2, 1, vec![1, 2, 3, 4, 5, 6, 7, 8]).unwrap();

    assert_eq!(image.pixels(), &[1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(image.pixel_at(1, 0), Some([5, 6, 7, 8]));
    assert_eq!(image.pixel_at(2, 0), None);
    assert_eq!(image.pixel_at(0, 1), None);
}

#[test]
fn image_rejects_empty_and_mismatched_storage() {
    assert_eq!(
        Image::from_rgba8(0, 1, Vec::new()).unwrap_err().code(),
        ImageErrorCode::InvalidDimensions
    );
    assert_eq!(
        Image::from_rgba8(2, 2, vec![0; 3]).unwrap_err().code(),
        ImageErrorCode::InvalidPixels
    );
}
