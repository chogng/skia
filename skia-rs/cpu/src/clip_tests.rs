use skia_core::{ClipOp, FillRule, Point, Scalar, SkiaErrorCode};

use super::{apply_clip, mask_index};
use crate::canvas::{Contour, DeviceRect};

fn scalar(value: i32) -> Scalar {
    Scalar::from_i32(value).expect("scalar")
}

fn rectangular_contour(left: i32, top: i32, right: i32, bottom: i32) -> Contour {
    Contour::new(
        vec![
            Point::new(scalar(left), scalar(top)),
            Point::new(scalar(right), scalar(top)),
            Point::new(scalar(right), scalar(bottom)),
            Point::new(scalar(left), scalar(bottom)),
        ],
        true,
    )
}

#[test]
fn mask_index_checks_signed_coordinates_and_arithmetic() {
    assert_eq!(mask_index(5, 3, 2).expect("mask index"), 13);
    assert_eq!(
        mask_index(5, -1, 0).expect_err("negative x").code(),
        SkiaErrorCode::NumericOverflow
    );
    assert_eq!(
        mask_index(u32::MAX, i64::MAX, i64::MAX)
            .expect_err("overflow")
            .code(),
        SkiaErrorCode::NumericOverflow
    );
}

#[test]
fn intersect_clip_combines_scissor_geometry_and_existing_mask() {
    let contour = rectangular_contour(1, 1, 4, 3);
    let scissor = DeviceRect {
        left: 2,
        top: 0,
        right: 4,
        bottom: 3,
    };
    let mut current = vec![u8::MAX; 4 * 3];
    current[2 * 4 + 3] = 0;

    let mask = apply_clip(
        4,
        3,
        scissor,
        Some(&current),
        &[contour],
        FillRule::NonZero,
        ClipOp::Intersect,
    )
    .expect("clip mask");

    assert_eq!(mask.as_ref(), &[0, 0, 0, 0, 0, 0, 255, 255, 0, 0, 255, 0]);
}

#[test]
fn difference_clip_keeps_only_pixels_outside_new_geometry() {
    let contour = rectangular_contour(1, 1, 3, 3);
    let mask = apply_clip(
        4,
        3,
        DeviceRect {
            left: 0,
            top: 0,
            right: 4,
            bottom: 3,
        },
        None,
        &[contour],
        FillRule::NonZero,
        ClipOp::Difference,
    )
    .expect("difference mask");

    assert_eq!(
        mask.as_ref(),
        &[255, 255, 255, 255, 255, 0, 0, 255, 255, 0, 0, 255]
    );
}
