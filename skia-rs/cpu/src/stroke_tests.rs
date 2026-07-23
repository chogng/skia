use skia_core::{Point, Scalar, StrokeAlign, StrokeCap, StrokeJoin, StrokeOptions};

use super::stroke_bounds;
use crate::canvas::{Contour, DeviceRect};

fn scalar(value: i32) -> Scalar {
    Scalar::from_i32(value).expect("scalar")
}

fn sample_contour() -> Contour {
    Contour::new(
        vec![
            Point::new(scalar(10), scalar(20)),
            Point::new(scalar(14), scalar(24)),
        ],
        false,
    )
}

fn coordinates(bounds: DeviceRect) -> (i64, i64, i64, i64) {
    (bounds.left, bounds.top, bounds.right, bounds.bottom)
}

#[test]
fn centered_fractional_radius_rounds_bounds_outward() {
    let options = StrokeOptions::new(scalar(3))
        .expect("stroke")
        .with_join(StrokeJoin::Bevel);

    let bounds = stroke_bounds(&[sample_contour()], &options).expect("bounds");

    assert_eq!(coordinates(bounds), (8, 18, 16, 26));
}

#[test]
fn square_caps_expand_twice_the_centered_radius() {
    let options = StrokeOptions::new(scalar(2))
        .expect("stroke")
        .with_cap(StrokeCap::Square)
        .with_join(StrokeJoin::Bevel);

    let bounds = stroke_bounds(&[sample_contour()], &options).expect("bounds");

    assert_eq!(coordinates(bounds), (8, 18, 16, 26));
}

#[test]
fn miter_limit_controls_the_conservative_extent() {
    let options = StrokeOptions::new(scalar(2))
        .expect("stroke")
        .with_join(StrokeJoin::Miter)
        .with_miter_limit(scalar(3))
        .expect("miter limit");

    let bounds = stroke_bounds(&[sample_contour()], &options).expect("bounds");

    assert_eq!(coordinates(bounds), (7, 17, 17, 27));
}

#[test]
fn non_center_alignment_uses_the_full_stroke_width() {
    for align in [StrokeAlign::Inside, StrokeAlign::Outside] {
        let options = StrokeOptions::new(scalar(2))
            .expect("stroke")
            .with_align(align)
            .with_join(StrokeJoin::Bevel);
        let bounds = stroke_bounds(&[sample_contour()], &options).expect("bounds");

        assert_eq!(coordinates(bounds), (8, 18, 16, 26));
    }
}
