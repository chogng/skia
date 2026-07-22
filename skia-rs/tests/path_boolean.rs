use skia::{
    Color, FillRule, Paint, Path, PathBooleanLimits, PathBooleanOp, PathBuilder, Rect, Scalar,
    SkiaErrorCode, Surface, SurfaceLimits, Transform, path_boolean,
};

fn scalar(value: i32) -> Scalar {
    Scalar::from_i32(value).expect("scalar")
}

fn rect(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
    Rect::new(scalar(left), scalar(top), scalar(right), scalar(bottom)).expect("rect")
}

fn rectangle(left: i32, top: i32, right: i32, bottom: i32) -> Path {
    let mut builder = PathBuilder::new(5).expect("builder");
    builder
        .add_rect(rect(left, top, right, bottom))
        .expect("rectangle");
    builder.finish().expect("path")
}

fn render(path: Option<Path>, width: u32, height: u32) -> Surface {
    let mut surface = Surface::new(width, height, SurfaceLimits::default()).expect("surface");
    if let Some(path) = path {
        surface
            .canvas()
            .fill_path(&path, FillRule::NonZero, Paint::new(Color::WHITE))
            .expect("fill boolean path");
    }
    surface
}

fn painted(surface: &Surface, x: usize, y: usize) -> bool {
    surface.pixels()[(y * surface.width() as usize + x) * 4 + 3] != 0
}

#[test]
fn rectangle_boolean_operations_cover_all_four_set_relations() {
    let subject = rectangle(1, 1, 5, 5);
    let clip = rectangle(3, 0, 7, 3);
    let apply = |operation| {
        render(
            path_boolean(
                &subject,
                &clip,
                operation,
                FillRule::NonZero,
                Transform::IDENTITY,
                PathBooleanLimits::default(),
            )
            .expect("boolean operation"),
            8,
            6,
        )
    };

    let union = apply(PathBooleanOp::Union);
    assert!(painted(&union, 1, 4));
    assert!(painted(&union, 6, 1));

    let intersection = apply(PathBooleanOp::Intersection);
    assert!(painted(&intersection, 3, 1));
    assert!(!painted(&intersection, 2, 1));
    assert!(!painted(&intersection, 3, 3));

    let difference = apply(PathBooleanOp::Difference);
    assert!(painted(&difference, 1, 1));
    assert!(!painted(&difference, 3, 1));
    assert!(painted(&difference, 3, 3));

    let xor = apply(PathBooleanOp::Xor);
    assert!(painted(&xor, 1, 1));
    assert!(painted(&xor, 6, 1));
    assert!(!painted(&xor, 3, 1));
}

#[test]
fn boolean_paths_preserve_holes_transform_and_empty_results() {
    let mut donut = PathBuilder::new(10).expect("donut builder");
    donut.add_rect(rect(0, 0, 6, 6)).expect("outer");
    donut.add_rect(rect(2, 2, 4, 4)).expect("inner");
    let donut = donut.finish().expect("donut");
    let clip = rectangle(1, 1, 5, 5);
    let result = path_boolean(
        &donut,
        &clip,
        PathBooleanOp::Intersection,
        FillRule::EvenOdd,
        Transform::translate(scalar(1), Scalar::ZERO),
        PathBooleanLimits::default(),
    )
    .expect("intersection");
    let surface = render(result, 8, 7);
    assert!(painted(&surface, 2, 1));
    assert!(!painted(&surface, 3, 2));

    let disjoint = rectangle(10, 10, 12, 12);
    assert!(
        path_boolean(
            &clip,
            &disjoint,
            PathBooleanOp::Intersection,
            FillRule::NonZero,
            Transform::IDENTITY,
            PathBooleanLimits::default(),
        )
        .expect("empty intersection")
        .is_none()
    );
}

#[test]
fn boolean_output_limits_fail_closed() {
    let limits = PathBooleanLimits::new(8, 64, 8, 3, 16).expect("limits");
    let error = path_boolean(
        &rectangle(0, 0, 4, 4),
        &rectangle(2, 0, 6, 4),
        PathBooleanOp::Union,
        FillRule::NonZero,
        Transform::IDENTITY,
        limits,
    )
    .expect_err("output point limit");
    assert_eq!(error.code(), SkiaErrorCode::ResourceLimit);
}
