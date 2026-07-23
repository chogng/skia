use skia_core::{FillRule, Point, Rect, Scalar, SkiaErrorCode};
use skia_image::Image;

use super::{
    DeviceRect, box_blur, ceil_q16_i64, contains, floor_q16_i64, pixel_center, pixel_offset,
    sample_linear,
};
use crate::canvas::Contour;

fn scalar(value: i32) -> Scalar {
    Scalar::from_i32(value).expect("scalar")
}

fn point(x: i32, y: i32) -> Point {
    Point::new(scalar(x), scalar(y))
}

fn contour(points: &[(i32, i32)]) -> Contour {
    Contour::new(points.iter().map(|&(x, y)| point(x, y)).collect(), true)
}

#[test]
fn fixed_point_rounding_is_directed_across_zero() {
    assert_eq!(floor_q16_i64(1), 0);
    assert_eq!(ceil_q16_i64(1), 1);
    assert_eq!(floor_q16_i64(-1), -1);
    assert_eq!(ceil_q16_i64(-1), 0);
    assert_eq!(floor_q16_i64(-(1 << 16)), -1);
    assert_eq!(ceil_q16_i64(-(1 << 16)), -1);
}

#[test]
fn private_pixel_math_rejects_negative_and_overflowing_coordinates() {
    assert_eq!(pixel_offset(10, 3, 2).expect("offset"), 92);
    assert_eq!(
        pixel_offset(10, -1, 0)
            .expect_err("negative coordinate")
            .code(),
        SkiaErrorCode::NumericOverflow
    );
    assert_eq!(
        pixel_offset(u32::MAX, i64::MAX, i64::MAX)
            .expect_err("overflowing coordinate")
            .code(),
        SkiaErrorCode::NumericOverflow
    );

    let center = pixel_center(2, -3).expect("pixel center");
    assert_eq!(center.x().bits(), (2 << 16) + (1 << 15));
    assert_eq!(center.y().bits(), (-3 << 16) + (1 << 15));
}

#[test]
fn linear_sampling_interpolates_all_four_neighbors() {
    let image = Image::from_rgba8(
        2,
        2,
        vec![
            0, 0, 0, 255, 100, 0, 0, 255, 0, 100, 0, 255, 100, 100, 0, 255,
        ],
    )
    .expect("image");
    let destination =
        Rect::new(Scalar::ZERO, Scalar::ZERO, scalar(2), scalar(2)).expect("destination");

    assert_eq!(
        sample_linear(&image, point(1, 1), destination).expect("sample"),
        [50, 50, 0, 255]
    );
}

#[test]
fn box_blur_operates_in_premultiplied_space() {
    let mut pixels = vec![0; 3 * 3 * 4];
    pixels[(4 * 4)..(4 * 4 + 4)].copy_from_slice(&[255, 0, 0, 255]);

    let blurred = box_blur(pixels, 3, 3, 1).expect("blur");

    for pixel in blurred.chunks_exact(4) {
        assert_eq!(pixel, [255, 0, 0, 28]);
    }
}

#[test]
fn fill_rules_distinguish_nested_contours_with_matching_winding() {
    let contours = [
        contour(&[(0, 0), (4, 0), (4, 4), (0, 4)]),
        contour(&[(1, 1), (3, 1), (3, 3), (1, 3)]),
    ];
    let sample = point(2, 2);

    assert!(!contains(&contours, sample, FillRule::EvenOdd).expect("even-odd"));
    assert!(contains(&contours, sample, FillRule::NonZero).expect("non-zero"));
}

#[test]
fn device_rect_normalization_and_disjoint_intersection_are_bounded() {
    let normalized = DeviceRect {
        left: 8,
        top: 7,
        right: 2,
        bottom: 3,
    }
    .normalized();
    assert_eq!(
        (
            normalized.left,
            normalized.top,
            normalized.right,
            normalized.bottom,
        ),
        (2, 3, 8, 7)
    );

    let intersection = normalized.intersection(DeviceRect {
        left: 10,
        top: 11,
        right: 12,
        bottom: 13,
    });
    assert_eq!(
        (
            intersection.left,
            intersection.top,
            intersection.right,
            intersection.bottom,
        ),
        (10, 11, 10, 11)
    );
}
