use skia_geometry::{Point, Scalar, Transform};
use skia_path::{ConicWeight, PathBuilder};

use super::{FlatteningLimits, PathFlattener};
use crate::TessellationErrorCode;

fn point(x: i32, y: i32) -> Point {
    Point::new(
        Scalar::from_i32(x).expect("x"),
        Scalar::from_i32(y).expect("y"),
    )
}

#[test]
fn flattens_all_curve_families_with_exact_resolution() {
    let mut builder = PathBuilder::new(5).expect("builder");
    builder.move_to(point(0, 0)).expect("move");
    builder
        .quad_to(point(1, 2), point(2, 0))
        .expect("quadratic");
    builder
        .conic_to(point(3, 2), point(4, 0), ConicWeight::ONE)
        .expect("conic");
    builder
        .cubic_to(point(5, 2), point(6, 2), point(7, 0))
        .expect("cubic");
    builder.close().expect("close");
    let path = builder.finish().expect("path");
    let limits = FlatteningLimits::for_path(&path, 16).expect("limits");

    let flattened = PathFlattener::new(limits)
        .flatten(&path, Transform::IDENTITY)
        .expect("flatten");

    assert_eq!(flattened.contours().len(), 1);
    assert_eq!(flattened.contours()[0].points().len(), 49);
    assert!(flattened.contours()[0].is_closed());
    assert_eq!(flattened.contours()[0].points()[48], point(7, 0));
}

#[test]
fn applies_transform_and_preserves_open_contours() {
    let mut builder = PathBuilder::new(2).expect("builder");
    builder.move_to(point(1, 2)).expect("move");
    builder.line_to(point(3, 4)).expect("line");
    let path = builder.finish().expect("path");
    let limits = FlatteningLimits::for_path(&path, 4).expect("limits");

    let flattened = PathFlattener::new(limits)
        .flatten(
            &path,
            Transform::translate(point(5, 6).x(), point(5, 6).y()),
        )
        .expect("flatten");

    assert_eq!(
        flattened.contours()[0].points(),
        &[point(6, 8), point(8, 10)]
    );
    assert!(!flattened.contours()[0].is_closed());
}

#[test]
fn rejects_output_beyond_explicit_point_limit() {
    let mut builder = PathBuilder::new(2).expect("builder");
    builder.move_to(point(0, 0)).expect("move");
    builder
        .quad_to(point(1, 2), point(2, 0))
        .expect("quadratic");
    let path = builder.finish().expect("path");
    let limits = FlatteningLimits::new(1, 4, 4).expect("limits");

    let error = PathFlattener::new(limits)
        .flatten(&path, Transform::IDENTITY)
        .expect_err("point limit");

    assert_eq!(error.code(), TessellationErrorCode::ResourceLimit);
}
