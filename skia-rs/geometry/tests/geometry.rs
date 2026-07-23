use skia_error::SkiaErrorCode;
use skia_geometry::{Point, Scalar, Transform};

fn scalar(value: i32) -> Scalar {
    Scalar::from_i32(value).expect("whole scalar")
}

fn point(x: i32, y: i32) -> Point {
    Point::new(scalar(x), scalar(y))
}

#[test]
fn scalar_ratio_rejects_a_zero_denominator() {
    assert_eq!(
        Scalar::from_ratio(1, 0)
            .expect_err("zero denominator")
            .code(),
        SkiaErrorCode::InvalidGeometry
    );
}

#[test]
fn concatenated_transforms_apply_in_documented_order() {
    let transform = Transform::translate(scalar(1), scalar(1))
        .concat(Transform::scale(scalar(2), scalar(3)))
        .expect("concatenated transform");

    assert_eq!(
        transform.map_point(point(1, 1)).expect("mapped point"),
        point(4, 6)
    );
}

#[test]
fn affine_transforms_round_trip_points_and_reject_singular_matrices() {
    let transform = Transform::new(
        scalar(1),
        scalar(1),
        Scalar::ZERO,
        scalar(1),
        scalar(2),
        scalar(3),
    );
    let mapped = transform.map_point(point(4, 5)).expect("map point");
    assert_eq!(mapped, point(6, 12));
    assert_eq!(
        transform
            .inverse()
            .expect("inverse")
            .map_point(mapped)
            .expect("round trip"),
        point(4, 5)
    );

    let singular = Transform::new(
        scalar(1),
        scalar(2),
        scalar(2),
        scalar(4),
        Scalar::ZERO,
        Scalar::ZERO,
    );
    assert_eq!(
        singular.inverse().expect_err("singular transform").code(),
        SkiaErrorCode::InvalidGeometry
    );
}
