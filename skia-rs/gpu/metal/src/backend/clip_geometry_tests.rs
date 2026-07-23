use skia_core::{ClipOp, Color, ConicWeight, FillRule, Paint, PathBuilder, Point, Rect, Scalar};
use skia_gpu::{GpuCommand, GpuCommandEncoder};

use super::{clip_edges, contour_edge_count};

fn point(x: i32, y: i32) -> Point {
    Point::new(
        Scalar::from_i32(x).expect("x"),
        Scalar::from_i32(y).expect("y"),
    )
}

#[test]
fn clip_edges_flatten_every_curve_family_with_fixed_steps() {
    let mut path = PathBuilder::new(5).expect("path limits");
    path.move_to(point(0, 0)).expect("move");
    path.quad_to(point(1, 2), point(2, 0)).expect("quad");
    path.conic_to(point(3, 2), point(4, 0), ConicWeight::ONE)
        .expect("conic");
    path.cubic_to(point(5, 2), point(6, 2), point(7, 0))
        .expect("cubic");
    path.close().expect("close");
    let mut encoder = GpuCommandEncoder::new(1).expect("encoder");
    let path = encoder
        .add_path(path.finish().expect("path"))
        .expect("path");
    encoder
        .clip_path(path, FillRule::NonZero, ClipOp::Intersect)
        .expect("clip");
    encoder
        .fill_rect(
            Rect::new(
                Scalar::ZERO,
                Scalar::ZERO,
                Scalar::from_i32(1).expect("right"),
                Scalar::from_i32(1).expect("bottom"),
            )
            .expect("rect"),
            Paint::new(Color::WHITE),
        )
        .expect("draw");
    let commands = encoder.finish();
    let GpuCommand::FillRect {
        clip: Some(clip), ..
    } = commands.commands()[0]
    else {
        panic!("expected complex clip");
    };
    let edges = clip_edges(&commands, commands.clip_node(clip).expect("clip node"))
        .expect("flattened edges");

    assert_eq!(edges.len(), 49);
}

#[test]
fn explicit_return_to_start_does_not_add_a_duplicate_closing_edge() {
    assert_eq!(
        contour_edge_count(&[point(0, 0), point(2, 0), point(0, 0)]),
        2
    );
}
