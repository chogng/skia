use std::io::{self, Write};

use skia_core::{
    ClipOp, Color, DisplayList, DisplayListBuilder, FillRule, Gradient, GradientStop, Paint,
    PathBuilder, Point, Rect, SamplingOptions, Scalar, StrokeCap, StrokeJoin, StrokeOptions,
    TileMode, Transform,
};
use skia_image::Image;

use super::{SvgCanvasSpec, SvgErrorCode, SvgLimits, SvgOptions, SvgWriter};

fn scalar(value: i32) -> Scalar {
    Scalar::from_i32(value).expect("scalar")
}

fn point(x: i32, y: i32) -> Point {
    Point::new(scalar(x), scalar(y))
}

fn rect(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
    Rect::new(scalar(left), scalar(top), scalar(right), scalar(bottom)).expect("rect")
}

fn spec() -> SvgCanvasSpec {
    SvgCanvasSpec::new(scalar(120), scalar(80)).expect("canvas")
}

fn svg(list: &DisplayList) -> String {
    String::from_utf8(SvgWriter::encode(spec(), list, SvgOptions::default()).expect("SVG document"))
        .expect("UTF-8 SVG")
}

#[test]
fn vector_commands_keep_geometry_state_clips_and_stroke_options() {
    let mut path = PathBuilder::new(5).expect("path");
    path.move_to(point(1, 2)).expect("move");
    path.line_to(point(20, 2)).expect("line");
    path.quad_to(point(25, 8), point(20, 14))
        .expect("quadratic");
    path.close().expect("close");

    let mut builder = DisplayListBuilder::new(16).expect("display list");
    let path = builder
        .add_path(path.finish().expect("path"))
        .expect("path id");
    builder.save().expect("save");
    builder
        .concat_transform(Transform::translate(scalar(3), scalar(4)))
        .expect("transform");
    builder
        .clip_rect(rect(0, 0, 50, 40))
        .expect("intersect clip");
    builder
        .fill_path(
            path,
            FillRule::EvenOdd,
            Paint::new(Color::rgba(10, 20, 30, 128)),
        )
        .expect("fill");
    let stroke = StrokeOptions::new(scalar(2))
        .expect("stroke")
        .with_cap(StrokeCap::Square)
        .with_join(StrokeJoin::Bevel)
        .with_dash_pattern(&[scalar(3), scalar(2)], scalar(1))
        .expect("dash");
    builder
        .stroke_path_with_options(path, stroke, Paint::new(Color::BLUE))
        .expect("stroke path");
    builder.restore().expect("restore");

    let output = svg(&builder.finish());
    assert!(output.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?><svg"));
    assert!(output.ends_with("</svg>"));
    assert!(output.contains(
        "<clipPath id=\"clip1\" clipPathUnits=\"userSpaceOnUse\"><rect x=\"0\" y=\"0\" \
         width=\"50\" height=\"40\" transform=\"matrix(1 0 0 1 3 4)\"/></clipPath>"
    ));
    assert!(output.contains(
        "<path d=\"M1 2L20 2Q25 8 20 14Z\" fill-rule=\"evenodd\" fill=\"#0A141E\" \
         fill-opacity=\"0.501961\" transform=\"matrix(1 0 0 1 3 4)\"/>"
    ));
    assert!(output.contains(
        "stroke=\"#0000FF\" stroke-width=\"2\" stroke-linecap=\"square\" \
         stroke-linejoin=\"bevel\" stroke-miterlimit=\"4\" stroke-dasharray=\"3 2\" \
         stroke-dashoffset=\"1\""
    ));
    assert!(output.contains("<g><g clip-path=\"url(#clip1)\">"));
    assert!(output.contains("</g></g></svg>"));
}

#[test]
fn gradient_and_image_resources_are_deduplicated() {
    let stops = [
        GradientStop::new(Scalar::ZERO, Color::RED).expect("first stop"),
        GradientStop::new(scalar(1), Color::rgba(0, 0, 255, 128)).expect("second stop"),
    ];
    let gradient =
        Gradient::linear(point(0, 0), point(20, 0), &stops, TileMode::Mirror).expect("gradient");
    let paint = Paint::from_gradient(gradient).with_opacity(128);
    let image = Image::from_rgba8(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 128]).expect("image");

    let mut builder = DisplayListBuilder::new(12).expect("display list");
    let image = builder.add_image(image).expect("image id");
    builder
        .fill_rect(rect(0, 0, 20, 10), paint.clone())
        .expect("first gradient");
    builder
        .fill_rect(rect(20, 0, 40, 10), paint)
        .expect("reused gradient");
    builder
        .draw_image_with_sampling(
            image,
            rect(0, 20, 20, 30),
            200,
            Paint::new(Color::WHITE).with_opacity(128),
            SamplingOptions::LINEAR,
        )
        .expect("first image");
    builder
        .draw_image(image, rect(20, 20, 40, 30), u8::MAX, Paint::default())
        .expect("reused image");

    let output = svg(&builder.finish());
    assert_eq!(occurrences(&output, "<linearGradient"), 1);
    assert_eq!(occurrences(&output, "fill=\"url(#gradient1)\""), 2);
    assert!(output.contains("spreadMethod=\"reflect\""));
    assert!(output.contains("stop-opacity=\"0.501961\""));
    assert_eq!(occurrences(&output, "<symbol id=\"image1\""), 1);
    assert_eq!(occurrences(&output, "<use href=\"#image1\""), 2);
    assert!(output.contains("href=\"data:image/png;base64,iVBORw0KGgo"));
    assert!(output.contains("opacity=\"0.392157\""));
    assert_eq!(occurrences(&output, "image-rendering=\"pixelated\""), 1);
}

#[test]
fn output_is_deterministic_and_preserves_an_independent_view_box() {
    let view_box = Rect::new(scalar(-10), scalar(-5), scalar(30), scalar(15)).expect("view box");
    let spec = SvgCanvasSpec::new(scalar(200), scalar(100))
        .expect("canvas")
        .with_view_box(view_box);
    let mut builder = DisplayListBuilder::new(2).expect("display list");
    builder
        .clear(Color::rgba(1, 2, 3, 64))
        .expect("canvas clear");
    let list = builder.finish();

    let first = SvgWriter::encode(spec, &list, SvgOptions::default()).expect("first");
    let second = SvgWriter::encode(spec, &list, SvgOptions::default()).expect("second");
    assert_eq!(first, second);
    let output = String::from_utf8(first).expect("UTF-8");
    assert!(output.contains("width=\"200\" height=\"100\" viewBox=\"-10 -5 40 20\""));
    assert!(output.contains(
        "<rect x=\"-10\" y=\"-5\" width=\"40\" height=\"20\" fill=\"#010203\" \
         fill-opacity=\"0.25098\"/>"
    ));
}

#[test]
fn unsupported_or_unbalanced_commands_do_not_write_a_prefix() {
    let mut unsupported = DisplayListBuilder::new(3).expect("display list");
    unsupported
        .clip_rect_with_op(rect(0, 0, 10, 10), ClipOp::Difference)
        .expect("difference clip");
    let unsupported = unsupported.finish();
    let mut destination = b"existing".to_vec();
    let error = SvgWriter::write(
        &mut destination,
        spec(),
        &unsupported,
        SvgOptions::default(),
    )
    .expect_err("difference clip is not native");
    assert_eq!(error.code(), SvgErrorCode::Unsupported);
    assert_eq!(destination, b"existing");

    let mut unbalanced = DisplayListBuilder::new(2).expect("display list");
    unbalanced.save().expect("save");
    let error = SvgWriter::encode(spec(), &unbalanced.finish(), SvgOptions::default())
        .expect_err("save requires restore");
    assert_eq!(error.code(), SvgErrorCode::InvalidState);
}

#[test]
fn command_path_image_and_output_limits_fail_explicitly() {
    let mut builder = DisplayListBuilder::new(4).expect("display list");
    builder
        .fill_rect(rect(0, 0, 10, 10), Paint::new(Color::RED))
        .expect("first command");
    builder
        .fill_rect(rect(10, 0, 20, 10), Paint::new(Color::BLUE))
        .expect("second command");
    let list = builder.finish();

    let limits = SvgLimits {
        max_commands: 1,
        ..SvgLimits::default()
    };
    let error =
        SvgWriter::encode(spec(), &list, SvgOptions { limits }).expect_err("command ceiling");
    assert_eq!(error.code(), SvgErrorCode::ResourceLimit);

    let limits = SvgLimits {
        max_output_bytes: 32,
        ..SvgLimits::default()
    };
    let error =
        SvgWriter::encode(spec(), &list, SvgOptions { limits }).expect_err("output ceiling");
    assert_eq!(error.code(), SvgErrorCode::ResourceLimit);

    let mut path = PathBuilder::new(2).expect("path");
    path.move_to(point(0, 0)).expect("move");
    path.line_to(point(10, 10)).expect("line");
    let mut builder = DisplayListBuilder::new(2).expect("display list");
    let path = builder
        .add_path(path.finish().expect("path"))
        .expect("path id");
    builder
        .fill_path(path, FillRule::NonZero, Paint::default())
        .expect("path draw");
    let limits = SvgLimits {
        max_path_verbs: 1,
        ..SvgLimits::default()
    };
    let error = SvgWriter::encode(spec(), &builder.finish(), SvgOptions { limits })
        .expect_err("path verb ceiling");
    assert_eq!(error.code(), SvgErrorCode::ResourceLimit);

    let image = Image::from_rgba8(1, 1, vec![1, 2, 3, 255]).expect("image");
    let mut builder = DisplayListBuilder::new(2).expect("display list");
    let image = builder.add_image(image).expect("image id");
    builder
        .draw_image(image, rect(0, 0, 1, 1), u8::MAX, Paint::default())
        .expect("image draw");
    let limits = SvgLimits {
        max_embedded_image_bytes: 1,
        ..SvgLimits::default()
    };
    let error = SvgWriter::encode(spec(), &builder.finish(), SvgOptions { limits })
        .expect_err("encoded image ceiling");
    assert_eq!(error.code(), SvgErrorCode::ResourceLimit);
}

#[test]
fn destination_failure_retains_the_io_category() {
    let list = DisplayListBuilder::new(1).expect("display list").finish();
    let error = SvgWriter::write(&mut FailingWriter, spec(), &list, SvgOptions::default())
        .expect_err("writer failure");
    assert_eq!(error.code(), SvgErrorCode::Io);
    assert_eq!(error.io_kind(), Some(io::ErrorKind::BrokenPipe));
}

fn occurrences(haystack: &str, needle: &str) -> usize {
    haystack.match_indices(needle).count()
}

struct FailingWriter;

impl Write for FailingWriter {
    fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
