use skia_core::{
    ClipOp, Color, DisplayListBuilder, FillRule, FontCollection, FontCollectionLimits, Paint, Path,
    PathBuilder, Rect, Scalar, Transform,
};
use skia_cpu::{ClipRect, Surface, SurfaceLimits};

fn scalar(value: i32) -> Scalar {
    Scalar::from_i32(value).expect("scalar")
}

fn rect(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
    Rect::new(scalar(left), scalar(top), scalar(right), scalar(bottom)).expect("rect")
}

fn rect_path(bounds: Rect) -> Path {
    let mut builder = PathBuilder::new(5).expect("path builder");
    builder.add_rect(bounds).expect("rect path");
    builder.finish().expect("path")
}

fn painted(surface: &Surface, x: usize, y: usize) -> bool {
    surface.pixels()[(y * surface.width() as usize + x) * 4 + 3] != 0
}

#[test]
fn path_intersection_clips_pixels() {
    let mut surface = Surface::new(6, 6, SurfaceLimits::default()).expect("surface");
    let path = rect_path(rect(1, 1, 4, 5));
    let mut canvas = surface.canvas();
    canvas
        .clip_path(&path, FillRule::NonZero, ClipOp::Intersect)
        .expect("path clip");
    canvas
        .fill_rect(rect(0, 0, 6, 6), Paint::new(Color::WHITE))
        .expect("fill");
    drop(canvas);

    assert!(painted(&surface, 1, 1));
    assert!(painted(&surface, 3, 4));
    assert!(!painted(&surface, 0, 1));
    assert!(!painted(&surface, 4, 4));
}

#[test]
fn display_list_rectangle_fill_replays_canvas_transform_and_clip_state() {
    let mut surface = Surface::new(6, 5, SurfaceLimits::default()).expect("surface");
    let mut builder = DisplayListBuilder::new(4).expect("display-list limits");
    let clip = rect(1, 1, 4, 4);

    builder.clip_rect(clip).expect("clip");
    builder
        .concat_transform(Transform::translate(scalar(1), Scalar::ZERO))
        .expect("translate");
    builder
        .fill_rect(rect(0, 0, 3, 3), Paint::new(Color::rgba(20, 40, 60, 255)))
        .expect("fill");
    let list = builder.finish();

    surface
        .execute_display_list(&list, &FontCollection::new(FontCollectionLimits::default()))
        .expect("replay");

    assert_eq!(
        &surface.pixels()[4 * (1 * 6 + 1)..][..4],
        &[20, 40, 60, 255]
    );
    assert_eq!(
        &surface.pixels()[4 * (2 * 6 + 3)..][..4],
        &[20, 40, 60, 255]
    );
    assert_eq!(&surface.pixels()[4 * (1 * 6)..][..4], &[0, 0, 0, 0]);
    assert_eq!(&surface.pixels()[4 * (4 * 6 + 3)..][..4], &[0, 0, 0, 0]);
}

#[test]
fn difference_clip_removes_the_new_geometry() {
    let mut surface = Surface::new(6, 6, SurfaceLimits::default()).expect("surface");
    let mut canvas = surface.canvas();
    canvas
        .clip_rect_with_op(ClipRect::new(rect(2, 1, 5, 4)), ClipOp::Difference)
        .expect("difference clip");
    canvas
        .fill_rect(rect(0, 0, 6, 6), Paint::new(Color::WHITE))
        .expect("fill");
    drop(canvas);

    assert!(painted(&surface, 1, 2));
    assert!(!painted(&surface, 2, 1));
    assert!(!painted(&surface, 4, 3));
    assert!(painted(&surface, 5, 3));
}

#[test]
fn transformed_rect_clip_uses_path_geometry() {
    let mut surface = Surface::new(6, 6, SurfaceLimits::default()).expect("surface");
    let mut canvas = surface.canvas();
    canvas.set_transform(Transform::new(
        Scalar::ZERO,
        scalar(1),
        scalar(-1),
        Scalar::ZERO,
        scalar(5),
        Scalar::ZERO,
    ));
    canvas
        .clip_rect(ClipRect::new(rect(1, 1, 4, 3)))
        .expect("rotated clip");
    canvas.set_transform(Transform::IDENTITY);
    canvas
        .fill_rect(rect(0, 0, 6, 6), Paint::new(Color::WHITE))
        .expect("fill");
    drop(canvas);

    assert!(painted(&surface, 2, 1));
    assert!(painted(&surface, 3, 3));
    assert!(!painted(&surface, 1, 2));
    assert!(!painted(&surface, 4, 2));
}

#[test]
fn restore_recovers_the_previous_complex_clip() {
    let mut surface = Surface::new(5, 5, SurfaceLimits::default()).expect("surface");
    let path = rect_path(rect(0, 0, 5, 5));
    let mut canvas = surface.canvas();
    canvas
        .clip_path(&path, FillRule::NonZero, ClipOp::Intersect)
        .expect("complex clip");
    canvas.save().expect("save");
    canvas
        .clip_rect_with_op(ClipRect::new(rect(1, 1, 4, 4)), ClipOp::Difference)
        .expect("difference clip");
    canvas
        .fill_rect(rect(0, 0, 5, 5), Paint::new(Color::rgba(255, 0, 0, 255)))
        .expect("first fill");
    canvas.restore().expect("restore");
    canvas
        .fill_rect(rect(2, 2, 3, 3), Paint::new(Color::rgba(0, 0, 255, 255)))
        .expect("restored fill");
    drop(canvas);

    let center = 4 * (2 * 5 + 2);
    assert_eq!(&surface.pixels()[center..center + 4], &[0, 0, 255, 255]);
    assert!(painted(&surface, 0, 0));
}
