use std::io::{self, Write};

use skia_core::{
    BlendMode, ClipOp, Color, DisplayList, DisplayListBuilder, FillRule, Gradient, GradientStop,
    Paint, PathBuilder, Point, Rect, Scalar, TileMode, Transform,
};
use skia_image::Image;

use super::*;
use crate::{
    PdfErrorCode as DocumentErrorCode, PdfLimits as DocumentLimits, PdfMetadata as DocumentMetadata,
};

fn scalar(value: i32) -> Scalar {
    Scalar::from_i32(value).expect("test scalar")
}

fn point(x: i32, y: i32) -> Point {
    Point::new(scalar(x), scalar(y))
}

fn rect(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
    Rect::new(scalar(left), scalar(top), scalar(right), scalar(bottom)).expect("test rectangle")
}

fn size(width: i32, height: i32) -> PageSize {
    PageSize::new(scalar(width), scalar(height)).expect("test page size")
}

fn empty_list() -> DisplayList {
    DisplayListBuilder::new(1).expect("builder").finish()
}

fn vector_list() -> DisplayList {
    let mut path = PathBuilder::new(16).expect("path");
    path.move_to(point(10, 10)).expect("move");
    path.line_to(point(90, 10)).expect("line");
    path.quad_to(point(50, 70), point(10, 10))
        .expect("quadratic");
    path.close().expect("close");
    let path = path.finish().expect("finished path");

    let mut builder = DisplayListBuilder::new(16).expect("display list");
    let path = builder.add_path(path).expect("path resource");
    builder.save().expect("save");
    builder
        .clip_rect_with_op(rect(5, 5, 95, 75), ClipOp::Intersect)
        .expect("clip");
    builder
        .concat_transform(Transform::translate(scalar(3), scalar(4)))
        .expect("transform");
    builder
        .fill_path(
            path,
            FillRule::EvenOdd,
            Paint::new(Color::rgba(220, 20, 40, 180)),
        )
        .expect("fill");
    builder.restore().expect("restore");
    builder.finish()
}

fn pdf_for(list: &DisplayList) -> Vec<u8> {
    let mut document = PdfDocument::new(Vec::new(), PdfOptions::default()).expect("document");
    document
        .add_page(PageSpec::new(size(100, 80)), list)
        .expect("page");
    document.finish().expect("pdf bytes")
}

#[test]
fn state_machine_rejects_nested_missing_and_unfinished_pages() {
    let mut document = PdfDocument::new(Vec::new(), PdfOptions::default()).expect("document");
    assert_eq!(
        document.end_page().expect_err("missing page").code(),
        DocumentErrorCode::InvalidState
    );
    document
        .begin_page(PageSpec::new(size(100, 80)))
        .expect("begin");
    assert_eq!(
        document
            .begin_page(PageSpec::new(size(100, 80)))
            .expect_err("nested page")
            .code(),
        DocumentErrorCode::InvalidState
    );
    document.add_display_list(&empty_list()).expect("list");
    assert_eq!(
        document.finish().expect_err("open page").code(),
        DocumentErrorCode::InvalidState
    );
}

#[test]
fn page_geometry_and_zero_limits_are_rejected() {
    assert_eq!(
        PageSize::new(Scalar::ZERO, scalar(10))
            .expect_err("empty width")
            .code(),
        DocumentErrorCode::InvalidPage
    );
    assert_eq!(
        PageSpec::new(size(10, 10))
            .with_content_box(rect(0, 0, 11, 10))
            .expect_err("outside page")
            .code(),
        DocumentErrorCode::InvalidPage
    );
    let options = PdfOptions {
        limits: DocumentLimits {
            max_pages: 0,
            ..DocumentLimits::default()
        },
        ..PdfOptions::default()
    };
    assert_eq!(
        PdfDocument::new(Vec::new(), options)
            .err()
            .expect("invalid limits")
            .code(),
        DocumentErrorCode::InvalidLimits
    );
}

#[test]
fn abort_returns_an_untouched_destination() {
    let mut document = PdfDocument::new(Vec::new(), PdfOptions::default()).expect("document");
    document
        .begin_page(PageSpec::new(size(100, 80)))
        .expect("begin");
    assert!(document.abort().is_empty());
}

#[test]
fn two_pages_have_valid_cross_references_and_distinct_sizes() {
    let mut document = PdfDocument::new(Vec::new(), PdfOptions::default()).expect("document");
    document
        .add_page(PageSpec::new(size(100, 80)), &vector_list())
        .expect("first");
    document
        .add_page(PageSpec::new(size(200, 120)), &empty_list())
        .expect("second");
    let bytes = document.finish().expect("finish");
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.starts_with("%PDF-1.7"));
    assert!(text.contains("/Count 2"));
    assert!(text.contains("/MediaBox [0 0 100 80]"));
    assert!(text.contains("/MediaBox [0 0 200 120]"));
    assert!(text.ends_with("%%EOF\n"));
    validate_xref(&bytes);
}

#[test]
fn vector_clip_transform_fill_rule_and_alpha_are_native() {
    let text = String::from_utf8_lossy(&pdf_for(&vector_list())).into_owned();
    assert!(text.contains("W n"));
    assert!(text.contains("1 0 0 1 3 4 cm"));
    assert!(text.contains("f*"));
    assert!(text.contains("/ca 0.705882"));
    assert!(text.contains(" c\n"));
}

#[test]
fn image_alpha_uses_smask_and_resources_are_deduplicated() {
    let image = Image::from_rgba8(
        2,
        2,
        vec![
            255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 64, 255, 255, 255, 0,
        ],
    )
    .expect("image");
    let mut builder = DisplayListBuilder::new(8).expect("builder");
    let image = builder.add_image(image).expect("resource");
    builder
        .draw_image(image, rect(0, 0, 20, 20), 255, Paint::new(Color::WHITE))
        .expect("first draw");
    builder
        .draw_image(image, rect(30, 0, 50, 20), 255, Paint::new(Color::WHITE))
        .expect("second draw");
    let bytes = pdf_for(&builder.finish());
    let text = String::from_utf8_lossy(&bytes);
    assert_eq!(text.matches("/Subtype /Image").count(), 2);
    assert!(text.contains("/SMask"));
    assert_eq!(text.matches("/Im0 Do").count(), 2);
    validate_xref(&bytes);
}

#[test]
fn metadata_is_escaped_and_non_ascii_uses_utf16() {
    let options = PdfOptions {
        metadata: DocumentMetadata {
            title: Some("A (bounded) PDF".to_owned()),
            author: Some("文档".to_owned()),
            subject: Some("subject".to_owned()),
            keywords: Some("pdf,deterministic".to_owned()),
            creator: Some("tests".to_owned()),
            producer: Some("stable producer".to_owned()),
        },
        ..PdfOptions::default()
    };
    let document = PdfDocument::new(Vec::new(), options).expect("document");
    let text = String::from_utf8_lossy(&document.finish().expect("finish")).into_owned();
    assert!(text.contains("/Title (A \\(bounded\\) PDF)"));
    assert!(text.contains("/Author <FEFF"));
    assert!(text.contains("/Producer (stable producer)"));
    assert!(!text.contains("/CreationDate"));
}

#[test]
fn unsupported_gradient_is_error_or_bounded_page_fallback() {
    let stops = [
        GradientStop::new(Scalar::ZERO, Color::RED).expect("stop"),
        GradientStop::new(scalar(1), Color::BLUE).expect("stop"),
    ];
    let gradient =
        Gradient::linear(point(0, 0), point(100, 0), &stops, TileMode::Clamp).expect("gradient");
    let mut builder = DisplayListBuilder::new(2).expect("builder");
    builder
        .fill_rect(rect(0, 0, 100, 80), Paint::from_gradient(gradient))
        .expect("fill");
    let list = builder.finish();

    let mut strict = PdfDocument::new(Vec::new(), PdfOptions::default()).expect("document");
    strict
        .begin_page(PageSpec::new(size(100, 80)))
        .expect("page");
    strict.add_display_list(&list).expect("list");
    assert_eq!(
        strict.end_page().expect_err("unsupported").code(),
        DocumentErrorCode::Unsupported
    );
    assert!(strict.is_page_open());

    let options = PdfOptions {
        unsupported_behavior: UnsupportedBehavior::RasterizePage,
        raster_fallback: RasterFallback {
            dpi: 72,
            max_pixels: 8_000,
            max_bytes: 32_000,
        },
        ..PdfOptions::default()
    };
    let mut fallback = PdfDocument::new(Vec::new(), options).expect("document");
    fallback
        .add_page(PageSpec::new(size(100, 80)), &list)
        .expect("fallback page");
    let text = String::from_utf8_lossy(&fallback.finish().expect("finish")).into_owned();
    assert!(text.contains("/Subtype /Image"));
}

#[test]
fn standard_pdf_blend_mode_is_native_and_porter_duff_is_explicit() {
    let mut native = DisplayListBuilder::new(1).expect("builder");
    native
        .fill_rect(
            rect(0, 0, 10, 10),
            Paint::new(Color::RED).with_blend_mode(BlendMode::Multiply),
        )
        .expect("fill");
    let text = String::from_utf8_lossy(&pdf_for(&native.finish())).into_owned();
    assert!(text.contains("/BM /Multiply"));

    let mut unsupported = DisplayListBuilder::new(1).expect("builder");
    unsupported
        .fill_rect(
            rect(0, 0, 10, 10),
            Paint::new(Color::RED).with_blend_mode(BlendMode::SourceIn),
        )
        .expect("fill");
    let mut document = PdfDocument::new(Vec::new(), PdfOptions::default()).expect("document");
    document
        .begin_page(PageSpec::new(size(100, 80)))
        .expect("begin");
    document
        .add_display_list(&unsupported.finish())
        .expect("list");
    assert_eq!(
        document.end_page().expect_err("unsupported").code(),
        DocumentErrorCode::Unsupported
    );
}

#[test]
fn deterministic_bytes_and_output_limit_are_enforced() {
    let first = pdf_for(&vector_list());
    let second = pdf_for(&vector_list());
    assert_eq!(first, second);

    let options = PdfOptions {
        limits: DocumentLimits {
            max_output_bytes: 32,
            ..DocumentLimits::default()
        },
        ..PdfOptions::default()
    };
    let document = PdfDocument::new(Vec::new(), options).expect("document");
    assert_eq!(
        document.finish().expect_err("output limit").code(),
        DocumentErrorCode::ResourceLimit
    );
}

#[test]
fn command_and_page_limits_are_predictable() {
    let options = PdfOptions {
        limits: DocumentLimits {
            max_pages: 1,
            max_commands_per_page: 1,
            ..DocumentLimits::default()
        },
        ..PdfOptions::default()
    };
    let mut document = PdfDocument::new(Vec::new(), options).expect("document");
    let mut builder = DisplayListBuilder::new(2).expect("builder");
    builder
        .fill_rect(rect(0, 0, 1, 1), Paint::new(Color::BLACK))
        .expect("one");
    builder
        .fill_rect(rect(1, 1, 2, 2), Paint::new(Color::BLACK))
        .expect("two");
    document
        .begin_page(PageSpec::new(size(10, 10)))
        .expect("begin");
    assert_eq!(
        document
            .add_display_list(&builder.finish())
            .expect_err("command limit")
            .code(),
        DocumentErrorCode::ResourceLimit
    );
}

#[test]
fn page_object_and_resource_limits_are_independent() {
    let options = PdfOptions {
        limits: DocumentLimits {
            max_pages: 1,
            ..DocumentLimits::default()
        },
        ..PdfOptions::default()
    };
    let mut document = PdfDocument::new(Vec::new(), options).expect("document");
    document
        .add_page(PageSpec::new(size(10, 10)), &empty_list())
        .expect("first page");
    assert_eq!(
        document
            .begin_page(PageSpec::new(size(10, 10)))
            .expect_err("page limit")
            .code(),
        DocumentErrorCode::ResourceLimit
    );

    let options = PdfOptions {
        limits: DocumentLimits {
            max_objects: 2,
            ..DocumentLimits::default()
        },
        ..PdfOptions::default()
    };
    let document = PdfDocument::new(Vec::new(), options).expect("document");
    assert_eq!(
        document.finish().expect_err("object limit").code(),
        DocumentErrorCode::ResourceLimit
    );

    let image = Image::from_rgba8(1, 1, vec![1, 2, 3, 255]).expect("image");
    let mut builder = DisplayListBuilder::new(2).expect("builder");
    let image = builder.add_image(image).expect("image resource");
    builder
        .draw_image(image, rect(0, 0, 1, 1), 255, Paint::new(Color::WHITE))
        .expect("draw");
    let options = PdfOptions {
        limits: DocumentLimits {
            max_resources: 1,
            ..DocumentLimits::default()
        },
        ..PdfOptions::default()
    };
    let mut document = PdfDocument::new(Vec::new(), options).expect("document");
    document
        .begin_page(PageSpec::new(size(10, 10)))
        .expect("begin");
    document
        .add_display_list(&builder.finish())
        .expect("display list");
    assert_eq!(
        document.end_page().expect_err("resource limit").code(),
        DocumentErrorCode::ResourceLimit
    );
}

#[test]
fn partial_writes_succeed_and_io_failures_keep_their_kind() {
    let writer = PartialWriter {
        bytes: Vec::new(),
        chunk: 3,
        fail_after: None,
    };
    let document = PdfDocument::new(writer, PdfOptions::default()).expect("document");
    let writer = document.finish().expect("partial writer");
    assert!(writer.bytes.starts_with(b"%PDF-1.7"));

    let writer = PartialWriter {
        bytes: Vec::new(),
        chunk: 4,
        fail_after: Some(20),
    };
    let document = PdfDocument::new(writer, PdfOptions::default()).expect("document");
    let error = document.finish().expect_err("I/O failure");
    assert_eq!(error.code(), DocumentErrorCode::Io);
    assert_eq!(error.io_kind(), Some(io::ErrorKind::BrokenPipe));
}

fn validate_xref(bytes: &[u8]) {
    let marker = b"startxref\n";
    let start = bytes
        .windows(marker.len())
        .rposition(|window| window == marker)
        .expect("startxref")
        + marker.len();
    let end = bytes[start..]
        .iter()
        .position(|byte| *byte == b'\n')
        .expect("xref line")
        + start;
    let xref: usize = std::str::from_utf8(&bytes[start..end])
        .expect("xref utf8")
        .parse()
        .expect("xref offset");
    assert_eq!(&bytes[xref..xref + 4], b"xref");

    let xref_text = std::str::from_utf8(&bytes[xref..]).expect("xref text");
    let mut lines = xref_text.lines();
    assert_eq!(lines.next(), Some("xref"));
    let header = lines.next().expect("xref header");
    let count: usize = header
        .split_whitespace()
        .nth(1)
        .expect("xref count")
        .parse()
        .expect("numeric xref count");
    assert_eq!(lines.next(), Some("0000000000 65535 f "));
    for object in 1..count {
        let line = lines.next().expect("xref entry");
        let offset: usize = line[..10].parse().expect("object offset");
        assert!(bytes[offset..].starts_with(format!("{object} 0 obj\n").as_bytes()));
    }
}

#[derive(Debug)]
struct PartialWriter {
    bytes: Vec<u8>,
    chunk: usize,
    fail_after: Option<usize>,
}

impl Write for PartialWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if self
            .fail_after
            .is_some_and(|limit| self.bytes.len() >= limit)
        {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "test failure"));
        }
        let allowed = self
            .fail_after
            .map_or(buffer.len(), |limit| limit.saturating_sub(self.bytes.len()));
        let length = buffer.len().min(self.chunk).min(allowed);
        if length == 0 {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "test failure"));
        }
        self.bytes.extend_from_slice(&buffer[..length]);
        Ok(length)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
