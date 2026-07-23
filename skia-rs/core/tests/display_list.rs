use skia_core::{
    ClipOp, Color, DisplayListBuilder, DrawCommand, FillRule, Paint, SamplingOptions,
    SaveLayerOptions, Scalar, SkiaErrorCode, StrokeAlign, StrokeCap, StrokeJoin, StrokeOptions,
    Transform,
};
#[cfg(feature = "text")]
use skia_core::{FontId, GlyphId, GlyphRun, PositionedGlyph, TextUnit};
use skia_image::Image;

#[cfg(feature = "text")]
fn glyph_run() -> GlyphRun {
    GlyphRun::new(
        FontId::new(9),
        12 << 16,
        1_000,
        vec![PositionedGlyph::new(
            GlyphId::new(3),
            TextUnit::ZERO,
            TextUnit::ZERO,
            TextUnit::from_bits(64),
            TextUnit::ZERO,
        )],
    )
    .expect("valid glyph run")
}

#[test]
#[cfg(feature = "text")]
fn display_list_owns_and_draws_a_glyph_run() {
    let mut builder = DisplayListBuilder::new(8).expect("valid limits");
    let run = builder.add_glyph_run(glyph_run()).expect("store run");
    let paint = Paint::new(Color::rgba(10, 20, 30, 255));

    builder
        .draw_glyph_run(run, paint.clone())
        .expect("record text command");
    let list = builder.finish();

    assert_eq!(
        list.glyph_run(run).expect("stored run").font(),
        FontId::new(9)
    );
    assert_eq!(list.commands(), &[DrawCommand::DrawGlyphRun { run, paint }]);
}

#[test]
fn stroke_command_rejects_non_positive_width() {
    let mut builder = DisplayListBuilder::new(2).expect("valid limits");
    let path = builder.add_path(empty_path()).expect("store path");
    let error = builder
        .stroke_path(
            path,
            skia_core::Scalar::ZERO,
            Paint::new(Color::rgba(0, 0, 0, 255)),
        )
        .expect_err("zero stroke width must fail");

    assert_eq!(error.code(), SkiaErrorCode::InvalidGeometry);
}

#[test]
fn display_list_owns_explicit_stroke_geometry() {
    let mut builder = DisplayListBuilder::new(2).expect("valid limits");
    let path = builder.add_path(empty_path()).expect("store path");
    let options = StrokeOptions::new(Scalar::from_i32(3).expect("width"))
        .expect("stroke")
        .with_align(StrokeAlign::Outside)
        .with_cap(StrokeCap::Square)
        .with_join(StrokeJoin::Bevel)
        .with_dash_pattern(
            &[
                Scalar::from_i32(2).expect("dash"),
                Scalar::from_i32(1).expect("gap"),
            ],
            Scalar::from_i32(1).expect("phase"),
        )
        .expect("dash options");
    let paint = Paint::new(Color::BLACK);
    builder
        .stroke_path_with_options(path, options.clone(), paint.clone())
        .expect("record stroke");

    assert_eq!(
        builder.finish().commands(),
        &[DrawCommand::StrokePath {
            path,
            options,
            paint,
        }]
    );
}

#[test]
fn display_list_records_generic_transform_concatenation() {
    let mut builder = DisplayListBuilder::new(1).expect("valid limits");
    let transform = Transform::translate(
        Scalar::from_i32(3).expect("scalar"),
        Scalar::from_i32(-2).expect("scalar"),
    );
    builder
        .concat_transform(transform)
        .expect("record transform");

    assert_eq!(
        builder.finish().commands(),
        &[DrawCommand::ConcatTransform(transform)]
    );
}

#[test]
fn display_list_records_direct_rectangle_fill() {
    let mut builder = DisplayListBuilder::new(1).expect("valid limits");
    let rect = skia_core::Rect::new(
        Scalar::ZERO,
        Scalar::ZERO,
        Scalar::from_i32(3).expect("right"),
        Scalar::from_i32(2).expect("bottom"),
    )
    .expect("rect");
    let paint = Paint::new(Color::rgba(12, 34, 56, 255));

    builder
        .fill_rect(rect, paint.clone())
        .expect("record rectangle fill");

    assert_eq!(
        builder.finish().commands(),
        &[DrawCommand::FillRect { rect, paint }]
    );
}

#[test]
fn display_list_records_isolated_layer_boundaries() {
    let mut builder = DisplayListBuilder::new(4).expect("builder");
    let options = SaveLayerOptions::new().with_opacity(128);
    let rect = skia_core::Rect::new(
        Scalar::ZERO,
        Scalar::ZERO,
        Scalar::from_i32(2).expect("right"),
        Scalar::from_i32(2).expect("bottom"),
    )
    .expect("rect");
    let paint = Paint::new(Color::RED);
    builder.save_layer(options).expect("save layer");
    builder.fill_rect(rect, paint.clone()).expect("fill");
    builder.restore().expect("restore");
    assert_eq!(
        builder.finish().commands(),
        &[
            DrawCommand::SaveLayer(options),
            DrawCommand::FillRect { rect, paint },
            DrawCommand::Restore,
        ]
    );
}

#[test]
fn display_list_records_explicit_image_sampling() {
    let mut builder = DisplayListBuilder::new(1).expect("valid limits");
    let image = builder
        .add_image(Image::from_rgba8(1, 1, vec![1, 2, 3, 255]).expect("image"))
        .expect("store image");
    let destination = skia_core::Rect::new(
        Scalar::ZERO,
        Scalar::ZERO,
        Scalar::from_i32(2).expect("right"),
        Scalar::from_i32(2).expect("bottom"),
    )
    .expect("rect");
    let paint = Paint::new(Color::WHITE);
    builder
        .draw_image_with_sampling(
            image,
            destination,
            200,
            paint.clone(),
            SamplingOptions::LINEAR,
        )
        .expect("record image");

    assert_eq!(
        builder.finish().commands(),
        &[DrawCommand::DrawImage {
            image,
            destination,
            opacity: 200,
            sampling: SamplingOptions::LINEAR,
            paint,
        }]
    );
}

#[test]
fn display_list_records_rect_and_path_clip_operations() {
    let mut builder = DisplayListBuilder::new(3).expect("valid limits");
    let path = builder.add_path(empty_path()).expect("store path");
    let rect = skia_core::Rect::new(
        Scalar::ZERO,
        Scalar::ZERO,
        Scalar::from_i32(4).expect("right"),
        Scalar::from_i32(3).expect("bottom"),
    )
    .expect("rect");
    builder.clip_rect(rect).expect("intersection clip");
    builder
        .clip_path(path, FillRule::EvenOdd, ClipOp::Difference)
        .expect("path clip");

    assert_eq!(
        builder.finish().commands(),
        &[
            DrawCommand::ClipRect {
                rect,
                op: ClipOp::Intersect,
            },
            DrawCommand::ClipPath {
                path,
                rule: FillRule::EvenOdd,
                op: ClipOp::Difference,
            },
        ]
    );
}

fn empty_path() -> skia_core::Path {
    let mut builder = skia_core::PathBuilder::new(1).expect("valid path limits");
    builder
        .move_to(skia_core::Point::new(
            skia_core::Scalar::ZERO,
            skia_core::Scalar::ZERO,
        ))
        .expect("start path");
    builder.finish().expect("valid one-point path")
}
