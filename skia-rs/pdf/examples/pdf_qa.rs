use std::{
    env,
    fs::{self, File},
};

use skia_core::{
    BlendMode, ClipOp, Color, DisplayListBuilder, FillRule, FontFace, FontId, Gradient,
    GradientStop, Paint, PathBuilder, Point, Rect, SamplingOptions, SaveLayerOptions, Scalar,
    StrokeCap, StrokeJoin, StrokeOptions, TileMode, Transform,
};
use skia_image::Image;
use skia_pdf::{
    PageSize, PageSpec, PdfDocument, PdfMetadata, PdfOptions, PdfStructureElement,
    PdfStructureOutline, PdfStructureTag, RasterFallback, UnsupportedBehavior,
};

fn scalar(value: i32) -> Scalar {
    Scalar::from_i32(value).expect("representable sample coordinate")
}

fn point(x: i32, y: i32) -> Point {
    Point::new(scalar(x), scalar(y))
}

fn rect(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
    Rect::new(scalar(left), scalar(top), scalar(right), scalar(bottom))
        .expect("positive sample rectangle")
}

fn main() {
    let mut arguments = env::args().skip(1);
    let output = arguments
        .next()
        .expect("usage: pdf_qa <output.pdf> [font.ttf]");
    let text_font = arguments.next();
    let options = PdfOptions {
        metadata: PdfMetadata {
            title: Some("skia-pdf QA".to_owned()),
            author: Some("skia-rs".to_owned()),
            subject: Some("Vector, clipping, transform, alpha image, and fallback QA".to_owned()),
            keywords: Some("pdf,vector,image,alpha,multipage".to_owned()),
            creator: Some("skia-pdf pdf_qa example".to_owned()),
            producer: None,
            ..PdfMetadata::default()
        },
        unsupported_behavior: UnsupportedBehavior::RasterizePage,
        raster_fallback: RasterFallback {
            dpi: 144,
            ..RasterFallback::default()
        },
        structure_outline: PdfStructureOutline::Headings,
        ..PdfOptions::default()
    };
    let writer = File::create(output).expect("create output PDF");
    let mut document = PdfDocument::new(writer, options).expect("create PDF document");

    let mut path = PathBuilder::new(16).expect("path builder");
    path.move_to(point(25, 40)).expect("move");
    path.cubic_to(point(80, 5), point(150, 95), point(205, 40))
        .expect("cubic");
    path.line_to(point(185, 135)).expect("line");
    path.quad_to(point(115, 175), point(45, 135))
        .expect("quadratic");
    path.close().expect("close");
    let path = path.finish().expect("path");

    let pixels = vec![
        255, 30, 30, 255, 30, 255, 30, 180, 30, 30, 255, 100, 255, 255, 255, 0,
    ];
    let image = Image::from_rgba8(2, 2, pixels).expect("sample image");
    let mut page = DisplayListBuilder::new(32).expect("display list");
    page.clear(Color::WHITE).expect("background");
    let path = page.add_path(path).expect("path resource");
    let image = page.add_image(image).expect("image resource");
    page.save().expect("save");
    page.clip_rect(rect(15, 15, 225, 165)).expect("clip");
    page.concat_transform(Transform::translate(scalar(4), scalar(6)))
        .expect("translation");
    page.fill_path(
        path,
        FillRule::EvenOdd,
        Paint::new(Color::rgba(25, 120, 215, 180)),
    )
    .expect("fill path");
    page.stroke_path_with_options(
        path,
        StrokeOptions::new(scalar(4))
            .expect("stroke")
            .with_cap(StrokeCap::Round)
            .with_join(StrokeJoin::Bevel),
        Paint::new(Color::rgba(15, 35, 75, 230)),
    )
    .expect("stroke path");
    page.restore().expect("restore");
    page.draw_image_with_sampling(
        image,
        rect(150, 90, 225, 165),
        220,
        Paint::new(Color::WHITE),
        SamplingOptions::LINEAR,
    )
    .expect("draw image");
    page.save_layer(
        SaveLayerOptions::new()
            .with_bounds(rect(45, 70, 135, 150))
            .with_opacity(150)
            .with_blend_mode(BlendMode::Multiply),
    )
    .expect("save layer");
    page.fill_rect(
        rect(20, 55, 150, 165),
        Paint::new(Color::rgba(255, 210, 35, 255)),
    )
    .expect("layer fill");
    page.restore().expect("restore layer");
    let page = page.finish();
    let first_size = PageSize::new(scalar(240), scalar(180)).expect("page size");
    let heading = document
        .add_structure_element(
            PdfStructureElement::new(PdfStructureTag::Heading1)
                .with_title("Vector transparency page".to_owned()),
            None,
        )
        .expect("heading structure");
    let figure = document
        .add_structure_element(
            PdfStructureElement::new(PdfStructureTag::Figure),
            Some(heading),
        )
        .expect("figure structure");
    document
        .begin_page(PageSpec::new(first_size))
        .expect("begin native vector page");
    document
        .add_structured_display_list(figure, &page)
        .expect("tagged native vector content");
    document.end_page().expect("end native vector page");

    let stops = [
        GradientStop::new(Scalar::ZERO, Color::rgba(245, 80, 90, 255)).expect("stop"),
        GradientStop::new(scalar(1), Color::rgba(60, 80, 230, 180)).expect("stop"),
    ];
    let gradient =
        Gradient::linear(point(0, 0), point(320, 0), &stops, TileMode::Clamp).expect("gradient");
    let mut fallback = DisplayListBuilder::new(8).expect("display list");
    fallback
        .fill_rect(rect(0, 0, 320, 120), Paint::from_gradient(gradient))
        .expect("gradient fill");
    fallback
        .clip_rect_with_op(rect(40, 20, 280, 100), ClipOp::Difference)
        .expect("difference clip");
    fallback
        .fill_rect(
            rect(0, 0, 320, 120),
            Paint::new(Color::rgba(255, 255, 255, 110)),
        )
        .expect("overlay");
    let second_size = PageSize::new(scalar(320), scalar(120)).expect("page size");
    document
        .add_page(PageSpec::new(second_size), &fallback.finish())
        .expect("fallback page");

    let opaque_stops = [
        GradientStop::new(Scalar::ZERO, Color::rgba(20, 145, 220, 255)).expect("stop"),
        GradientStop::new(scalar(1), Color::rgba(235, 90, 50, 255)).expect("stop"),
    ];
    let opaque_gradient =
        Gradient::radial(point(160, 80), scalar(110), &opaque_stops, TileMode::Clamp)
            .expect("gradient");
    let mut native_gradient = DisplayListBuilder::new(4).expect("display list");
    native_gradient
        .fill_rect(rect(0, 0, 320, 160), Paint::from_gradient(opaque_gradient))
        .expect("gradient fill");
    let third_size = PageSize::new(scalar(320), scalar(160)).expect("page size");
    document
        .add_page(PageSpec::new(third_size), &native_gradient.finish())
        .expect("native gradient page");

    if let Some(font_path) = text_font {
        let face = FontFace::from_bytes(
            FontId::new(9_001),
            fs::read(font_path).expect("read QA font"),
        )
        .expect("load QA font");
        let run = face
            .shape("Searchable PDF text", 24 << 16)
            .expect("shape text");
        let offsets = vec![0; run.glyphs().len()];
        let mut text = DisplayListBuilder::new(4).expect("text display list");
        let run = text.add_glyph_run(run).expect("text run");
        text.draw_positioned_glyph_run(
            run,
            point(24, 52),
            offsets,
            Paint::new(Color::rgba(20, 40, 60, 255)),
        )
        .expect("draw text");
        let text_size = PageSize::new(scalar(320), scalar(96)).expect("text page size");
        document
            .add_page_with_embedded_text(PageSpec::new(text_size), &text.finish(), &face)
            .expect("text page");
    }

    document.finish().expect("write PDF");
}
