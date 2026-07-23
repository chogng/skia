use std::io::{self, Write};

use skia_core::{
    BlendMode, ClipOp, Color, DisplayList, DisplayListBuilder, FillRule, FontFace, FontId, GlyphId,
    GlyphOutline, GlyphOutlineProvider, GlyphRun, Gradient, GradientStop, OutlinePoint,
    OutlineSegment, Paint, PathBuilder, Point, PositionedGlyph, Rect, SaveLayerOptions, Scalar,
    TextError, TextUnit, TileMode, Transform,
};
use skia_image::Image;

use super::*;
use crate::{
    PdfErrorCode as DocumentErrorCode, PdfLimits as DocumentLimits, PdfMetadata as DocumentMetadata,
};

#[path = "../../test-support/font.rs"]
mod test_font;

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

struct SingleGlyphProvider(GlyphOutline);

struct EmbeddedTextProvider {
    font: PdfEmbeddedFont,
    source: String,
}

impl PdfTextProvider for EmbeddedTextProvider {
    fn embedded_font(&self, font: FontId) -> Option<PdfEmbeddedFont> {
        (self.font.font() == font).then(|| self.font.clone())
    }

    fn source_text(&self, _run: &GlyphRun) -> Option<String> {
        Some(self.source.clone())
    }
}

impl GlyphOutlineProvider for SingleGlyphProvider {
    fn glyph_outline(
        &self,
        font: FontId,
        glyph: GlyphId,
    ) -> Result<Option<GlyphOutline>, TextError> {
        Ok((self.0.font() == font && self.0.glyph() == glyph).then(|| self.0.clone()))
    }
}

fn text_unit(value: i32) -> TextUnit {
    TextUnit::from_i32(value).expect("small exact coordinate")
}

fn glyph_run() -> GlyphRun {
    GlyphRun::new(
        FontId::new(7),
        10 << 16,
        10,
        vec![PositionedGlyph::new(
            GlyphId::new(3),
            TextUnit::ZERO,
            TextUnit::ZERO,
            TextUnit::ZERO,
            TextUnit::ZERO,
        )],
    )
    .expect("valid run")
}

fn glyph_outline() -> GlyphOutline {
    let point = |x, y| OutlinePoint::new(text_unit(x), text_unit(y));
    GlyphOutline::new(
        FontId::new(7),
        GlyphId::new(3),
        vec![
            OutlineSegment::MoveTo(point(0, 0)),
            OutlineSegment::LineTo(point(10, 0)),
            OutlineSegment::LineTo(point(10, 10)),
            OutlineSegment::LineTo(point(0, 10)),
            OutlineSegment::Close,
        ],
    )
    .expect("valid closed square")
}

fn searchable_text_run() -> GlyphRun {
    GlyphRun::new(
        FontId::new(23),
        12 << 16,
        1_000,
        vec![PositionedGlyph::new(
            GlyphId::new(1),
            TextUnit::ZERO,
            TextUnit::ZERO,
            TextUnit::ZERO,
            TextUnit::ZERO,
        )],
    )
    .expect("valid run")
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
            ..DocumentMetadata::default()
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
fn pdf_a_2b_writes_xmp_output_intent_and_deterministic_identifier() {
    let timestamp = PdfDateTime::new(2026, 7, 23, 12, 34, 56).expect("timestamp");
    let options = PdfOptions {
        metadata: DocumentMetadata {
            title: Some("A & B".to_owned()),
            author: Some("Author".to_owned()),
            subject: Some("Subject".to_owned()),
            keywords: Some("pdf,a-2b".to_owned()),
            creator: Some("tests".to_owned()),
            producer: Some("stable producer".to_owned()),
            creation: Some(timestamp),
            modified: Some(timestamp),
        },
        conformance: PdfConformance::PdfA2b,
        ..PdfOptions::default()
    };
    let first = PdfDocument::new(Vec::new(), options.clone())
        .expect("document")
        .finish()
        .expect("finish");
    let second = PdfDocument::new(Vec::new(), options)
        .expect("document")
        .finish()
        .expect("finish");
    assert_eq!(first, second);
    let text = String::from_utf8_lossy(&first);
    assert!(text.contains("/Metadata 4 0 R /OutputIntents [5 0 R]"));
    assert!(text.contains("/S /GTS_PDFA1"));
    assert!(text.contains("pdfaid:part=\"2\" pdfaid:conformance=\"B\""));
    assert!(text.contains("A &amp; B"));
    assert!(text.contains("/CreationDate (D:20260723123456Z)"));
    assert!(text.contains("/ID [<"));
    validate_xref(&first);

    let invalid = PdfOptions {
        conformance: PdfConformance::PdfA2b,
        ..PdfOptions::default()
    };
    let Err(error) = PdfDocument::new(Vec::new(), invalid) else {
        panic!("timestamps required");
    };
    assert_eq!(error.code(), DocumentErrorCode::InvalidMetadata);
}

#[test]
fn links_and_named_destinations_preserve_top_left_coordinates() {
    let mut document = PdfDocument::new(Vec::new(), PdfOptions::default()).expect("document");
    document
        .begin_page(PageSpec::new(size(100, 80)))
        .expect("first page");
    document
        .add_link(
            rect(10, 10, 30, 30),
            PdfLinkTarget::NamedDestination("second-page".to_owned()),
        )
        .expect("internal link");
    document
        .add_link(
            rect(40, 10, 60, 30),
            PdfLinkTarget::Uri("https://example.test/qa".to_owned()),
        )
        .expect("URI link");
    document.end_page().expect("end first page");
    document
        .begin_page(PageSpec::new(size(100, 80)))
        .expect("second page");
    document
        .add_named_destination("second-page".to_owned(), point(15, 25))
        .expect("destination");
    document.end_page().expect("end second page");
    let bytes = document.finish().expect("finish");
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("/Annots ["));
    assert!(text.contains("/Names << /Dests << /Names [ (second-page)"));
    assert!(text.contains("/Dest (second-page)"));
    assert!(text.contains("/URI (https://example.test/qa)"));
    assert!(text.contains("/Rect [10 50 30 70]"));
    assert!(text.contains("/XYZ 15 55 null"));
    validate_xref(&bytes);

    let mut unresolved = PdfDocument::new(Vec::new(), PdfOptions::default()).expect("document");
    unresolved
        .begin_page(PageSpec::new(size(100, 80)))
        .expect("page");
    unresolved
        .add_link(
            rect(0, 0, 10, 10),
            PdfLinkTarget::NamedDestination("missing".to_owned()),
        )
        .expect("link");
    unresolved.end_page().expect("end page");
    let Err(error) = unresolved.finish() else {
        panic!("unresolved destination");
    };
    assert_eq!(error.code(), DocumentErrorCode::InvalidDestination);
}

#[test]
fn bookmarks_reference_named_destinations_and_fail_closed_when_unresolved() {
    let mut document = PdfDocument::new(Vec::new(), PdfOptions::default()).expect("document");
    document
        .add_bookmark("Second page".to_owned(), "second-page".to_owned())
        .expect("bookmark");
    document
        .begin_page(PageSpec::new(size(100, 80)))
        .expect("first page");
    document.end_page().expect("end first page");
    document
        .begin_page(PageSpec::new(size(100, 80)))
        .expect("second page");
    document
        .add_named_destination("second-page".to_owned(), point(10, 20))
        .expect("destination");
    document.end_page().expect("end second page");
    let bytes = document.finish().expect("finish");
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("/Outlines "));
    assert!(text.contains("/PageMode /UseOutlines"));
    assert!(text.contains("/Type /Outlines"));
    assert!(text.contains("/Title (Second page)"));
    assert!(text.contains("/Dest (second-page)"));
    validate_xref(&bytes);

    let mut unresolved = PdfDocument::new(Vec::new(), PdfOptions::default()).expect("document");
    unresolved
        .add_bookmark("Missing".to_owned(), "missing".to_owned())
        .expect("bookmark declaration");
    let Err(error) = unresolved.finish() else {
        panic!("unresolved destination");
    };
    assert_eq!(error.code(), DocumentErrorCode::InvalidDestination);
}

#[test]
fn tagged_display_lists_write_marked_content_and_structure_tree() {
    let mut document = PdfDocument::new(Vec::new(), PdfOptions::default()).expect("document");
    document
        .begin_page(PageSpec::new(size(100, 80)))
        .expect("page");
    document
        .add_tagged_display_list(PdfStructureTag::Paragraph, &vector_list())
        .expect("tagged content");
    document.end_page().expect("end page");
    let bytes = document.finish().expect("finish");
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("/P << /MCID 0 >> BDC"));
    assert!(text.contains("/MarkInfo << /Marked true >> /StructTreeRoot"));
    assert!(text.contains("/StructParents 0"));
    assert!(text.contains("/Type /StructTreeRoot"));
    assert!(text.contains("/Type /StructElem /S /P"));
    assert!(text.contains("/Nums [ 0 ["));
    validate_xref(&bytes);
}

#[test]
fn save_layers_emit_native_isolated_transparency_groups() {
    let mut builder = DisplayListBuilder::new(4).expect("builder");
    builder.clear(Color::WHITE).expect("clear");
    builder
        .save_layer(
            SaveLayerOptions::new()
                .with_bounds(rect(10, 10, 90, 70))
                .with_opacity(180)
                .with_blend_mode(BlendMode::Multiply),
        )
        .expect("save layer");
    builder
        .fill_rect(rect(0, 0, 100, 80), Paint::new(Color::RED))
        .expect("draw in layer");
    builder.restore().expect("restore layer");
    let bytes = pdf_for(&builder.finish());
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("/Subtype /Form"));
    assert!(text.contains("/Group << /S /Transparency /I true /K false >>"));
    assert!(text.contains("/Fm0 Do"));
    assert!(text.contains("/BM /Multiply"));
    assert!(!text.contains("/Subtype /Image"));
    validate_xref(&bytes);
}

#[test]
fn unsupported_gradient_is_error_or_bounded_page_fallback() {
    let stops = [
        GradientStop::new(Scalar::ZERO, Color::RED).expect("stop"),
        GradientStop::new(scalar(1), Color::BLUE).expect("stop"),
    ];
    let gradient =
        Gradient::linear(point(0, 0), point(100, 0), &stops, TileMode::Repeat).expect("gradient");
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
fn opaque_clamped_gradients_stay_vector_native() {
    let stops = [
        GradientStop::new(Scalar::ZERO, Color::RED).expect("stop"),
        GradientStop::new(scalar(1), Color::BLUE).expect("stop"),
    ];
    let gradient =
        Gradient::linear(point(0, 0), point(100, 0), &stops, TileMode::Clamp).expect("gradient");
    let mut builder = DisplayListBuilder::new(1).expect("builder");
    builder
        .fill_rect(rect(0, 0, 100, 80), Paint::from_gradient(gradient))
        .expect("fill");
    let bytes = pdf_for(&builder.finish());
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("/Shading << /Sh0"));
    assert!(text.contains("/Sh0 sh"));
    assert!(text.contains("/ShadingType 2"));
    assert!(!text.contains("/Subtype /Image"));
    validate_xref(&bytes);
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
fn linear_match_uses_cpu_fallback_for_pdf_transparency() {
    let mut builder = DisplayListBuilder::new(1).expect("builder");
    builder
        .fill_rect(rect(0, 0, 10, 10), Paint::new(Color::rgba(255, 0, 0, 128)))
        .expect("fill");
    let list = builder.finish();

    let strict_options = PdfOptions {
        color_policy: PdfColorPolicy::LinearMatch,
        ..PdfOptions::default()
    };
    let mut strict = PdfDocument::new(Vec::new(), strict_options).expect("document");
    strict
        .begin_page(PageSpec::new(size(100, 80)))
        .expect("begin");
    strict.add_display_list(&list).expect("list");
    assert_eq!(
        strict
            .end_page()
            .expect_err("linear fallback required")
            .code(),
        DocumentErrorCode::Unsupported
    );

    let fallback_options = PdfOptions {
        color_policy: PdfColorPolicy::LinearMatch,
        unsupported_behavior: UnsupportedBehavior::RasterizePage,
        raster_fallback: RasterFallback {
            dpi: 72,
            max_pixels: 8_000,
            max_bytes: 32_000,
        },
        ..PdfOptions::default()
    };
    let mut fallback = PdfDocument::new(Vec::new(), fallback_options).expect("document");
    fallback
        .add_page(PageSpec::new(size(100, 80)), &list)
        .expect("fallback page");
    let text = String::from_utf8_lossy(&fallback.finish().expect("finish")).into_owned();
    assert!(text.contains("/Subtype /Image"));
    assert!(!text.contains("/BM /Multiply"));
}

#[test]
fn linear_match_keeps_opaque_source_over_vectors_native() {
    let mut builder = DisplayListBuilder::new(1).expect("builder");
    builder
        .fill_rect(rect(0, 0, 10, 10), Paint::new(Color::RED))
        .expect("fill");
    let options = PdfOptions {
        color_policy: PdfColorPolicy::LinearMatch,
        ..PdfOptions::default()
    };
    let mut document = PdfDocument::new(Vec::new(), options).expect("document");
    document
        .add_page(PageSpec::new(size(100, 80)), &builder.finish())
        .expect("native vector page");
    let text = String::from_utf8_lossy(&document.finish().expect("finish")).into_owned();
    assert!(!text.contains("/Subtype /Image"));
    assert!(text.contains("1 0 0 rg"));
}

#[test]
fn linear_match_rejects_opaque_non_source_over_blending() {
    let mut builder = DisplayListBuilder::new(1).expect("builder");
    builder
        .fill_rect(
            rect(0, 0, 10, 10),
            Paint::new(Color::RED).with_blend_mode(BlendMode::Multiply),
        )
        .expect("fill");
    let options = PdfOptions {
        color_policy: PdfColorPolicy::LinearMatch,
        ..PdfOptions::default()
    };
    let mut document = PdfDocument::new(Vec::new(), options).expect("document");
    document
        .begin_page(PageSpec::new(size(100, 80)))
        .expect("begin");
    document.add_display_list(&builder.finish()).expect("list");
    assert_eq!(
        document
            .end_page()
            .expect_err("linear fallback required")
            .code(),
        DocumentErrorCode::Unsupported
    );
}

#[test]
fn glyph_outlines_are_emitted_as_vector_paths() {
    let mut builder = DisplayListBuilder::new(2).expect("builder");
    let glyphs = builder.add_glyph_run(glyph_run()).expect("glyphs");
    builder
        .draw_glyph_run(glyphs, Paint::new(Color::rgba(20, 40, 60, 255)))
        .expect("draw glyphs");
    let list = builder.finish();

    let mut unsupported = PdfDocument::new(Vec::new(), PdfOptions::default()).expect("document");
    unsupported
        .begin_page(PageSpec::new(size(100, 80)))
        .expect("begin");
    unsupported.add_display_list(&list).expect("list");
    assert_eq!(
        unsupported
            .end_page()
            .expect_err("no outline provider")
            .code(),
        DocumentErrorCode::UnsupportedText
    );

    let mut document = PdfDocument::new(Vec::new(), PdfOptions::default()).expect("document");
    document
        .add_page_with_glyph_outlines(
            PageSpec::new(size(100, 80)),
            &list,
            &SingleGlyphProvider(glyph_outline()),
        )
        .expect("outlined PDF page");
    let text = String::from_utf8_lossy(&document.finish().expect("finish")).into_owned();
    assert!(text.contains("0 0 m\n10 0 l\n10 10 l\n0 10 l\nh\nf"));
    assert!(!text.contains("/Font"));
}

#[test]
fn embedded_true_type_text_is_searchable_and_uses_actual_text() {
    let face = FontFace::from_bytes(FontId::new(23), test_font::toy_font('A')).expect("face");
    let provider = EmbeddedTextProvider {
        font: PdfEmbeddedFont::from_font_face(&face).expect("embedded font"),
        source: "A".to_owned(),
    };
    let mut builder = DisplayListBuilder::new(2).expect("builder");
    let run = builder.add_glyph_run(searchable_text_run()).expect("run");
    builder
        .draw_glyph_run(run, Paint::new(Color::rgba(20, 40, 60, 255)))
        .expect("draw text");
    let list = builder.finish();
    let mut document = PdfDocument::new(Vec::new(), PdfOptions::default()).expect("document");
    document
        .add_page_with_embedded_text(PageSpec::new(size(100, 80)), &list, &provider)
        .expect("embedded text page");
    let bytes = document.finish().expect("finish");
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("/Subtype /Type0"));
    assert!(text.contains("/FontFile2"));
    assert!(text.contains("/ActualText (A)"));
    assert!(text.contains("/F0 12 Tf"));
    assert!(text.contains("<0001> Tj"));
    validate_xref(&bytes);
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
