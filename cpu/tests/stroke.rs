use skia_core::{
    Color, Paint, Path, PathBuilder, Point, Scalar, StrokeCap, StrokeJoin, StrokeOptions,
};
use skia_cpu::{Surface, SurfaceLimits};

fn scalar(value: i32) -> Scalar {
    Scalar::from_i32(value).expect("scalar")
}

fn point(x: i32, y: i32) -> Point {
    Point::new(scalar(x), scalar(y))
}

fn line(points: &[(i32, i32)]) -> Path {
    let mut builder = PathBuilder::new(points.len()).expect("path builder");
    builder
        .move_to(point(points[0].0, points[0].1))
        .expect("move");
    for &(x, y) in &points[1..] {
        builder.line_to(point(x, y)).expect("line");
    }
    builder.finish().expect("path")
}

fn closed_path(points: &[(i32, i32)]) -> Path {
    let mut builder = PathBuilder::new(points.len() + 1).expect("path builder");
    builder
        .move_to(point(points[0].0, points[0].1))
        .expect("move");
    for &(x, y) in &points[1..] {
        builder.line_to(point(x, y)).expect("line");
    }
    builder.close().expect("close");
    builder.finish().expect("path")
}

fn draw(path: &Path, options: &StrokeOptions, width: u32, height: u32) -> Surface {
    let mut surface = Surface::new(width, height, SurfaceLimits::default()).expect("surface");
    surface
        .canvas()
        .stroke_path_with_options(path, options, Paint::new(Color::WHITE))
        .expect("stroke");
    surface
}

fn painted(surface: &Surface, x: usize, y: usize) -> bool {
    surface.pixels()[(y * surface.width() as usize + x) * 4 + 3] != 0
}

#[test]
fn stroke_caps_distinguish_butt_round_and_square_geometry() {
    let path = line(&[(4, 4), (8, 4)]);
    let options = |cap| StrokeOptions::new(scalar(4)).expect("stroke").with_cap(cap);
    let butt = draw(&path, &options(StrokeCap::Butt), 12, 9);
    let round = draw(&path, &options(StrokeCap::Round), 12, 9);
    let square = draw(&path, &options(StrokeCap::Square), 12, 9);

    assert!(!painted(&butt, 2, 4));
    assert!(painted(&round, 2, 4));
    assert!(painted(&square, 2, 4));
    assert!(!painted(&round, 2, 2));
    assert!(painted(&square, 2, 2));
}

#[test]
fn stroke_joins_honor_round_bevel_miter_and_miter_limit() {
    let path = line(&[(2, 6), (6, 6), (6, 10)]);
    let options = |join| {
        StrokeOptions::new(scalar(6))
            .expect("stroke")
            .with_join(join)
    };
    let round = draw(&path, &options(StrokeJoin::Round), 12, 12);
    let bevel = draw(&path, &options(StrokeJoin::Bevel), 12, 12);
    let miter = draw(&path, &options(StrokeJoin::Miter), 12, 12);
    let limited = draw(
        &path,
        &options(StrokeJoin::Miter)
            .with_miter_limit(scalar(1))
            .expect("limited miter"),
        12,
        12,
    );

    assert!(painted(&round, 8, 4));
    assert!(!painted(&bevel, 8, 4));
    assert!(painted(&miter, 8, 3));
    assert!(!painted(&limited, 8, 3));
}

#[test]
fn stroke_dash_pattern_and_phase_split_flattened_contours() {
    let path = line(&[(2, 5), (18, 5)]);
    let pattern = [scalar(4), scalar(4)];
    let solid_phase = StrokeOptions::new(scalar(2))
        .expect("stroke")
        .with_dash_pattern(&pattern, Scalar::ZERO)
        .expect("dash");
    let shifted = StrokeOptions::new(scalar(2))
        .expect("stroke")
        .with_dash_pattern(&pattern, scalar(2))
        .expect("shifted dash");
    let solid_phase = draw(&path, &solid_phase, 20, 11);
    let shifted = draw(&path, &shifted, 20, 11);

    for x in [2, 3, 4, 5, 10, 11, 12, 13] {
        assert!(painted(&solid_phase, x, 5), "expected dash at x={x}");
    }
    for x in [6, 7, 8, 9, 14, 15, 16, 17] {
        assert!(!painted(&solid_phase, x, 5), "expected gap at x={x}");
    }
    for x in [2, 3, 8, 9, 10, 11, 16, 17] {
        assert!(painted(&shifted, x, 5), "expected shifted dash at x={x}");
    }
    for x in [4, 5, 6, 7, 12, 13, 14, 15] {
        assert!(!painted(&shifted, x, 5), "expected shifted gap at x={x}");
    }
}

#[test]
fn closed_dash_continues_through_the_contour_seam() {
    let path = closed_path(&[(4, 4), (8, 4), (8, 8), (4, 8)]);
    let options = StrokeOptions::new(scalar(4))
        .expect("stroke")
        .with_join(StrokeJoin::Miter)
        .with_dash_pattern(&[scalar(6), scalar(2)], scalar(1))
        .expect("dash");
    let surface = draw(&path, &options, 12, 12);

    assert!(painted(&surface, 2, 2), "mitered seam must remain joined");
}
