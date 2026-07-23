use skia_core::{
    Color, DisplayListBuilder, FontCollection, FontCollectionLimits, FontFace, FontId, FontStyle,
    GlyphId, GlyphOutline, GlyphOutlineProvider, Paint, Point, Rect, Scalar, SkiaErrorCode,
    TextAffinity, TextAlignment, TextDecoration, TextDecorationStyle, TextDirection, TextError,
    TextErrorCode, TextLayoutOptions, TextPosition, TextStyleId, TextStyleSpan, TextWordBreak,
    TextWordBreakKind, Transform,
};
use skia_cpu::{Surface, SurfaceLimits};

#[path = "../../test-support/font.rs"]
mod font_support;

use font_support::*;

struct FailingProvider;

impl GlyphOutlineProvider for FailingProvider {
    fn glyph_outline(
        &self,
        _font: FontId,
        _glyph: GlyphId,
    ) -> Result<Option<GlyphOutline>, TextError> {
        Err(TextError::new(TextErrorCode::InvalidFontData))
    }
}

#[test]
fn utf8_text_shapes_and_draws_through_the_cpu_pipeline() {
    let face = FontFace::from_bytes(FontId::new(7), toy_font('A')).expect("load toy font");
    let run = face.shape("AA", 10 << 16).expect("shape UTF-8 text");

    assert_eq!(run.glyphs().len(), 2);
    assert_eq!(run.glyphs()[0].cluster(), 0);
    assert_eq!(run.glyphs()[1].cluster(), 1);
    assert_eq!(run.glyphs()[0].glyph().value(), 1);
    assert_eq!(run.glyphs()[0].advance_x().bits(), 600 * 64);
    assert!(
        face.glyph_outline(face.id(), run.glyphs()[0].glyph())
            .expect("resolve outline")
            .is_some()
    );

    let mut surface = Surface::new(16, 12, SurfaceLimits::default()).expect("surface");
    let mut canvas = surface.canvas();
    canvas.set_transform(Transform::translate(scalar(2), scalar(9)));
    canvas
        .draw_glyph_run(&run, &face, Paint::new(Color::rgba(20, 40, 60, 255)))
        .expect("draw shaped text");
    drop(canvas);

    assert_eq!(pixel(&surface, 3, 4), [20, 40, 60, 255]);
    assert_eq!(pixel(&surface, 9, 4), [20, 40, 60, 255]);
    assert_eq!(pixel(&surface, 15, 11), [0, 0, 0, 0]);
}

#[test]
fn styled_paragraphs_select_fonts_sizes_and_grapheme_safe_fallback() {
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    fonts
        .add_face(FontFace::from_bytes(FontId::new(150), toy_font('A')).expect("A font"))
        .expect("add A font");
    fonts
        .add_face(
            FontFace::from_bytes(
                FontId::new(151),
                toy_font_for(&['A', '\u{0301}', '\u{05d0}']),
            )
            .expect("fallback font"),
        )
        .expect("add fallback font");
    let text = "A\u{05d0}A";
    let spans = [
        TextStyleSpan::new(0, 1, FontId::new(150), 10 << 16).expect("first span"),
        TextStyleSpan::new(1, 3, FontId::new(150), 20 << 16).expect("fallback span"),
        TextStyleSpan::new(3, 4, FontId::new(151), 15 << 16).expect("last span"),
    ];
    assert_eq!(spans[0].source_start(), 0);
    assert_eq!(spans[0].source_end(), 1);
    assert_eq!(spans[0].font(), FontId::new(150));
    assert_eq!(spans[0].font_size_bits(), 10 << 16);

    let paragraph = fonts
        .shape_styled_paragraph_with_direction(text, &spans, TextDirection::LeftToRight)
        .expect("styled bidi paragraph");
    assert_eq!(paragraph.runs().len(), 3);
    assert_eq!(paragraph.runs()[0].glyph_run().font(), FontId::new(150));
    assert_eq!(paragraph.runs()[0].glyph_run().font_size_bits(), 10 << 16);
    assert_eq!(paragraph.runs()[1].glyph_run().font(), FontId::new(151));
    assert_eq!(paragraph.runs()[1].glyph_run().font_size_bits(), 20 << 16);
    assert_eq!(paragraph.runs()[2].glyph_run().font(), FontId::new(151));
    assert_eq!(paragraph.runs()[2].glyph_run().font_size_bits(), 15 << 16);
    assert_eq!(paragraph.advance_x_bits(), 27 << 16);
    assert_eq!(paragraph.metrics().ascent_bits(), 16 << 16);
    assert_eq!(paragraph.metrics().descent_bits(), 4 << 16);

    let color = [45, 90, 135, 255];
    let mut surface = Surface::new(30, 24, SurfaceLimits::default()).expect("surface");
    let mut canvas = surface.canvas();
    canvas
        .draw_shaped_paragraph(
            &paragraph,
            &fonts,
            Point::new(scalar(1), scalar(17)),
            Paint::new(Color::rgba(color[0], color[1], color[2], color[3])),
        )
        .expect("draw styled paragraph");
    drop(canvas);
    assert_eq!(pixel(&surface, 2, 12), color);
    assert_eq!(pixel(&surface, 8, 4), color);
    assert_eq!(pixel(&surface, 20, 7), color);

    let split_grapheme = [
        TextStyleSpan::new(0, 1, FontId::new(151), 10 << 16).expect("first half"),
        TextStyleSpan::new(1, 3, FontId::new(151), 10 << 16).expect("second half"),
    ];
    assert_eq!(
        fonts
            .shape_styled_paragraph("A\u{0301}", &split_grapheme)
            .expect_err("span must not split a grapheme")
            .code(),
        TextErrorCode::InvalidTextStyleSpan
    );
    assert_eq!(
        fonts
            .shape_styled_paragraph(
                "AA",
                &[TextStyleSpan::new(0, 1, FontId::new(150), 10 << 16).expect("partial")],
            )
            .expect_err("spans must cover text")
            .code(),
        TextErrorCode::InvalidTextStyleSpan
    );
    assert_eq!(
        TextStyleSpan::new(1, 1, FontId::new(150), 10 << 16)
            .expect_err("empty span")
            .code(),
        TextErrorCode::InvalidTextStyleSpan
    );
}

#[test]
fn ordered_fallback_shapes_and_draws_multiple_faces() {
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    fonts
        .add_face(FontFace::from_bytes(FontId::new(10), toy_font('A')).expect("A font"))
        .expect("add A font");
    fonts
        .add_face(FontFace::from_bytes(FontId::new(11), toy_font('B')).expect("B font"))
        .expect("add B font");

    let paragraph = fonts
        .shape_paragraph("AB", 10 << 16)
        .expect("fallback shape");
    assert_eq!(paragraph.base_direction(), TextDirection::LeftToRight);
    assert_eq!(paragraph.advance_x_bits(), 12 << 16);
    assert_eq!(paragraph.runs().len(), 2);
    assert_eq!(paragraph.runs()[0].glyph_run().font(), FontId::new(10));
    assert_eq!(paragraph.runs()[0].source_start(), 0);
    assert_eq!(paragraph.runs()[0].origin_x_bits(), 0);
    assert_eq!(paragraph.runs()[1].glyph_run().font(), FontId::new(11));
    assert_eq!(paragraph.runs()[1].source_start(), 1);
    assert_eq!(paragraph.runs()[1].origin_x_bits(), 6 << 16);

    let mut surface = Surface::new(16, 12, SurfaceLimits::default()).expect("surface");
    let mut canvas = surface.canvas();
    canvas
        .draw_shaped_paragraph(
            &paragraph,
            &fonts,
            Point::new(scalar(2), scalar(9)),
            Paint::new(Color::rgba(70, 80, 90, 255)),
        )
        .expect("draw fallback paragraph");
    drop(canvas);

    assert_eq!(pixel(&surface, 3, 4), [70, 80, 90, 255]);
    assert_eq!(pixel(&surface, 9, 4), [70, 80, 90, 255]);
}

#[test]
fn paragraph_draw_restores_canvas_state_after_provider_failure() {
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    fonts
        .add_face(FontFace::from_bytes(FontId::new(40), toy_font('A')).expect("A font"))
        .expect("add font");
    let paragraph = fonts.shape_paragraph("A", 10 << 16).expect("shape");
    let mut surface = Surface::new(10, 10, SurfaceLimits::default()).expect("surface");
    let mut canvas = surface.canvas();
    canvas.set_transform(Transform::translate(scalar(1), Scalar::ZERO));

    assert_eq!(
        canvas
            .draw_shaped_paragraph(
                &paragraph,
                &FailingProvider,
                Point::new(scalar(5), scalar(8)),
                Paint::new(Color::BLACK),
            )
            .expect_err("provider failure must propagate")
            .code(),
        SkiaErrorCode::TextResolverFailed
    );
    canvas
        .fill_rect(
            Rect::new(Scalar::ZERO, Scalar::ZERO, scalar(1), scalar(1)).expect("rect"),
            Paint::new(Color::BLACK),
        )
        .expect("draw after failed paragraph");
    drop(canvas);

    assert_eq!(pixel(&surface, 1, 0), [0, 0, 0, 255]);
    assert_eq!(pixel(&surface, 6, 8), [0, 0, 0, 0]);
}

#[test]
fn font_metrics_and_unicode_soft_wrap_position_lines() {
    let face = FontFace::from_bytes(FontId::new(50), toy_font_for(&[' ', 'A'])).expect("text font");
    let metrics = face.metrics(10 << 16).expect("font metrics");
    assert_eq!(metrics.ascent_bits(), 8 << 16);
    assert_eq!(metrics.descent_bits(), 2 << 16);
    assert_eq!(metrics.line_gap_bits(), 0);
    assert_eq!(metrics.line_height_bits().expect("line height"), 10 << 16);

    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    fonts.add_face(face).expect("add font");
    let layout = fonts
        .layout_text(
            "A A",
            10 << 16,
            TextLayoutOptions::new(12 << 16).expect("layout options"),
        )
        .expect("wrap text");

    assert_eq!(layout.lines().len(), 2);
    assert_eq!(layout.width_bits(), 12 << 16);
    assert_eq!(layout.height_bits(), 20 << 16);
    assert_eq!(
        (
            layout.lines()[0].source_start(),
            layout.lines()[0].source_end()
        ),
        (0, 2)
    );
    assert_eq!(layout.lines()[0].baseline_y_bits(), 8 << 16);
    assert!(!layout.lines()[0].hard_break());
    assert_eq!(
        (
            layout.lines()[1].source_start(),
            layout.lines()[1].source_end()
        ),
        (2, 3)
    );
    assert_eq!(layout.lines()[1].baseline_y_bits(), 18 << 16);
    assert_eq!(
        layout.lines()[1]
            .paragraph()
            .expect("second shaped line")
            .runs()[0]
            .glyph_run()
            .glyphs()[0]
            .cluster(),
        2
    );

    let mut surface = Surface::new(18, 24, SurfaceLimits::default()).expect("surface");
    let mut canvas = surface.canvas();
    canvas
        .draw_text_layout(
            &layout,
            &fonts,
            Point::new(scalar(2), scalar(1)),
            Paint::new(Color::rgba(100, 110, 120, 255)),
        )
        .expect("draw text layout");
    drop(canvas);

    assert_eq!(pixel(&surface, 3, 4), [100, 110, 120, 255]);
    assert_eq!(pixel(&surface, 3, 14), [100, 110, 120, 255]);
}

#[test]
fn dictionary_hyphenation_wraps_ltr_text_and_draws_synthetic_glyphs() {
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    fonts
        .add_face(
            FontFace::from_bytes(
                FontId::new(65),
                toy_font_for(&['-', 'a', 'e', 'h', 'i', 'n', 'o', 'p', 't', 'y']),
            )
            .expect("hyphenation font"),
        )
        .expect("add font");
    let provider = FixedBreakProvider {
        language: "en-US",
        opportunities: vec![
            TextWordBreak::new(6, TextWordBreakKind::Hyphenated),
            TextWordBreak::new(2, TextWordBreakKind::Hyphenated),
            TextWordBreak::new(2, TextWordBreakKind::Hyphenated),
        ],
    };
    let layout = fonts
        .layout_text_with_break_provider(
            "hyphenation",
            10 << 16,
            TextLayoutOptions::new(31 << 16).expect("options"),
            "en-US",
            &provider,
        )
        .expect("hyphenated layout");

    assert_eq!(layout.lines().len(), 3);
    assert_eq!(
        (
            layout.lines()[0].source_start(),
            layout.lines()[0].source_end()
        ),
        (0, 2)
    );
    assert!(layout.lines()[0].hyphenated());
    assert_eq!(layout.lines()[0].advance_x_bits(), 18 << 16);
    let first = layout.lines()[0].paragraph().expect("first line");
    assert_eq!(first.runs().len(), 2);
    assert_eq!(first.runs()[1].source_start(), 2);
    assert_eq!(first.runs()[1].source_end(), 2);
    assert_eq!(first.runs()[1].glyph_run().glyphs()[0].cluster(), 2);
    assert_eq!(first.runs()[1].origin_x_bits(), 12 << 16);
    assert_eq!(
        layout
            .caret_for_position(TextPosition::new(2, TextAffinity::Upstream))
            .expect("hyphen upstream query")
            .expect("hyphen upstream")
            .x_bits(),
        18 << 16
    );
    assert_eq!(
        layout
            .caret_for_position(TextPosition::new(2, TextAffinity::Downstream))
            .expect("hyphen downstream query")
            .expect("hyphen downstream")
            .line_index(),
        1
    );
    assert_eq!(
        (
            layout.lines()[1].source_start(),
            layout.lines()[1].source_end()
        ),
        (2, 6)
    );
    assert!(layout.lines()[1].hyphenated());
    assert_eq!(
        (
            layout.lines()[2].source_start(),
            layout.lines()[2].source_end()
        ),
        (6, 11)
    );
    assert!(!layout.lines()[2].hyphenated());

    let mut surface = Surface::new(34, 34, SurfaceLimits::default()).expect("surface");
    let mut canvas = surface.canvas();
    canvas
        .draw_text_layout(
            &layout,
            &fonts,
            Point::new(scalar(1), scalar(1)),
            Paint::new(Color::rgba(190, 180, 170, 255)),
        )
        .expect("draw hyphenated text");
    drop(canvas);
    assert_eq!(pixel(&surface, 14, 4), [190, 180, 170, 255]);
}

#[test]
fn physical_and_logical_alignment_position_ltr_and_rtl_lines() {
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    fonts
        .add_face(FontFace::from_bytes(FontId::new(80), toy_font('A')).expect("A font"))
        .expect("add font");

    let layout = |alignment, direction| {
        fonts
            .layout_text(
                "A",
                10 << 16,
                TextLayoutOptions::new(20 << 16)
                    .expect("options")
                    .with_alignment(alignment)
                    .with_base_direction(direction),
            )
            .expect("aligned layout")
    };

    let start_ltr = layout(TextAlignment::Start, TextDirection::LeftToRight);
    let start_rtl = layout(TextAlignment::Start, TextDirection::RightToLeft);
    let end_rtl = layout(TextAlignment::End, TextDirection::RightToLeft);
    let centered = layout(TextAlignment::Center, TextDirection::LeftToRight);
    let right = layout(TextAlignment::Right, TextDirection::LeftToRight);

    assert_eq!(start_ltr.container_width_bits(), 20 << 16);
    assert_eq!(start_ltr.width_bits(), 6 << 16);
    assert_eq!(start_ltr.lines()[0].advance_x_bits(), 6 << 16);
    assert_eq!(start_ltr.lines()[0].offset_x_bits(), 0);
    assert_eq!(start_rtl.lines()[0].offset_x_bits(), 14 << 16);
    assert_eq!(end_rtl.lines()[0].offset_x_bits(), 0);
    assert_eq!(centered.lines()[0].offset_x_bits(), 7 << 16);
    assert_eq!(right.lines()[0].offset_x_bits(), 14 << 16);

    let mut rtl_fonts = FontCollection::new(FontCollectionLimits::default());
    rtl_fonts
        .add_face(FontFace::from_bytes(FontId::new(81), toy_font('\u{05d0}')).expect("Alef font"))
        .expect("add font");
    let natural_rtl = rtl_fonts
        .layout_text(
            "\u{05d0}",
            10 << 16,
            TextLayoutOptions::new(20 << 16).expect("options"),
        )
        .expect("natural RTL layout");
    assert_eq!(
        natural_rtl.lines()[0]
            .paragraph()
            .expect("RTL line")
            .base_direction(),
        TextDirection::RightToLeft
    );
    assert_eq!(natural_rtl.lines()[0].offset_x_bits(), 14 << 16);

    let mut surface = Surface::new(22, 12, SurfaceLimits::default()).expect("surface");
    let mut canvas = surface.canvas();
    canvas
        .draw_text_layout(
            &right,
            &fonts,
            Point::new(Scalar::ZERO, scalar(1)),
            Paint::new(Color::rgba(130, 140, 150, 255)),
        )
        .expect("draw right-aligned text");
    drop(canvas);

    assert_eq!(pixel(&surface, 1, 4), [0, 0, 0, 0]);
    assert_eq!(pixel(&surface, 15, 4), [130, 140, 150, 255]);
}

#[test]
fn justification_expands_interior_spaces_and_controls_the_final_line() {
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    fonts
        .add_face(
            FontFace::from_bytes(FontId::new(90), toy_font_for(&[' ', 'A'])).expect("text font"),
        )
        .expect("add font");

    let wrapped = fonts
        .layout_text(
            "A A A",
            10 << 16,
            TextLayoutOptions::new(25 << 16)
                .expect("options")
                .with_alignment(TextAlignment::Justify),
        )
        .expect("justified wrap");
    assert_eq!(wrapped.lines().len(), 2);
    assert!(wrapped.lines()[0].justified());
    assert_eq!(wrapped.lines()[0].advance_x_bits(), 25 << 16);
    assert_eq!(wrapped.lines()[0].offset_x_bits(), 0);
    assert_eq!(wrapped.lines()[0].source_end(), 4);
    assert_eq!(
        wrapped.lines()[0].paragraph().expect("first line").runs()[0].glyph_offsets_x_bits(),
        &[0, 0, 1 << 16, 1 << 16]
    );
    assert!(!wrapped.lines()[1].justified());

    let default_final = fonts
        .layout_text(
            "A A",
            10 << 16,
            TextLayoutOptions::new(24 << 16)
                .expect("options")
                .with_alignment(TextAlignment::Justify),
        )
        .expect("default final line");
    assert!(!default_final.lines()[0].justified());
    assert_eq!(default_final.lines()[0].advance_x_bits(), 18 << 16);

    let justified_final = fonts
        .layout_text(
            "A A",
            10 << 16,
            TextLayoutOptions::new(24 << 16)
                .expect("options")
                .with_alignment(TextAlignment::Justify)
                .with_justify_last_line(true),
        )
        .expect("justified final line");
    assert!(justified_final.lines()[0].justified());
    assert_eq!(justified_final.lines()[0].advance_x_bits(), 24 << 16);
    assert_eq!(
        justified_final.lines()[0].paragraph().expect("line").runs()[0].glyph_offsets_x_bits(),
        &[0, 0, 6 << 16]
    );

    let mut surface = Surface::new(26, 12, SurfaceLimits::default()).expect("surface");
    let mut canvas = surface.canvas();
    canvas
        .draw_text_layout(
            &justified_final,
            &fonts,
            Point::new(Scalar::ZERO, scalar(1)),
            Paint::new(Color::rgba(160, 170, 180, 255)),
        )
        .expect("draw justified text");
    drop(canvas);
    assert_eq!(pixel(&surface, 13, 4), [0, 0, 0, 0]);
    assert_eq!(pixel(&surface, 19, 4), [160, 170, 180, 255]);
}

#[test]
fn font_decorations_use_primary_metrics_across_fallback_and_alignment() {
    let primary = FontFace::from_bytes(
        FontId::new(110),
        toy_styled_font(&[' ', 'A'], "Decorated", FontStyle::NORMAL),
    )
    .expect("decorated primary font");
    let underline = primary
        .underline_metrics(20 << 16)
        .expect("underline query")
        .expect("post metrics");
    assert_eq!(underline.offset_bits(), 2 << 16);
    assert_eq!(underline.thickness_bits(), 2 << 16);
    let strike_through = primary
        .strike_through_metrics(20 << 16)
        .expect("strike-through query")
        .expect("OS/2 metrics");
    assert_eq!(strike_through.offset_bits(), -6 << 16);
    assert_eq!(strike_through.thickness_bits(), 2 << 16);

    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    fonts.add_face(primary).expect("add primary font");
    fonts
        .add_face(FontFace::from_bytes(FontId::new(111), toy_font('B')).expect("fallback font"))
        .expect("add fallback font");
    let layout = fonts
        .layout_text(
            "A B",
            20 << 16,
            TextLayoutOptions::new(40 << 16)
                .expect("options")
                .with_alignment(TextAlignment::Right)
                .with_decoration(TextDecoration::UnderlineAndStrikeThrough),
        )
        .expect("decorated fallback layout");
    let line = &layout.lines()[0];
    assert_eq!(line.offset_x_bits(), 4 << 16);
    assert_eq!(line.underline_metrics(), Some(underline));
    assert_eq!(line.strike_through_metrics(), Some(strike_through));
    assert_eq!(
        line.paragraph().expect("line").runs()[1].glyph_run().font(),
        FontId::new(111)
    );

    let color = [25, 50, 75, 255];
    let mut surface = Surface::new(42, 24, SurfaceLimits::default()).expect("surface");
    let mut canvas = surface.canvas();
    canvas
        .draw_text_layout(
            &layout,
            &fonts,
            Point::new(Scalar::ZERO, scalar(1)),
            Paint::new(Color::rgba(color[0], color[1], color[2], color[3])),
        )
        .expect("draw decorated layout");
    drop(canvas);

    assert_eq!(pixel(&surface, 18, 10), [22, 50, 75, 255]);
    assert_eq!(pixel(&surface, 18, 18), color);
    assert_eq!(pixel(&surface, 2, 18), [0, 0, 0, 0]);
}

#[test]
fn text_decoration_patterns_share_resolved_cpu_geometry() {
    let face = FontFace::from_bytes(
        FontId::new(119),
        toy_styled_font(&['A'], "Decoration Patterns", FontStyle::NORMAL),
    )
    .expect("decorated font");
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    fonts.add_face(face).expect("add decorated font");
    let layout = fonts
        .layout_text(
            "AA",
            20 << 16,
            TextLayoutOptions::new(30 << 16)
                .expect("options")
                .with_decoration(TextDecoration::Underline)
                .with_decoration_style(TextDecorationStyle::Wavy),
        )
        .expect("wavy underline layout");
    assert_eq!(
        layout.lines()[0].decoration_style(),
        TextDecorationStyle::Wavy
    );

    let mut surface = Surface::new(30, 24, SurfaceLimits::default()).expect("surface");
    surface
        .canvas()
        .draw_text_layout(
            &layout,
            &fonts,
            Point::new(Scalar::ZERO, scalar(1)),
            Paint::new(Color::RED),
        )
        .expect("draw wavy underline");

    assert_eq!(pixel(&surface, 0, 16), Color::RED.channels());
    assert_eq!(pixel(&surface, 0, 20), Color::TRANSPARENT.channels());
    assert_eq!(pixel(&surface, 4, 20), Color::RED.channels());
}

#[test]
fn display_list_expands_layout_runs_and_decorations_transactionally() {
    let face = FontFace::from_bytes(
        FontId::new(118),
        toy_styled_font(&['A'], "Display Layout", FontStyle::NORMAL),
    )
    .expect("decorated font");
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    fonts.add_face(face).expect("add decorated font");
    let layout = fonts
        .layout_text(
            "AA",
            20 << 16,
            TextLayoutOptions::new(30 << 16)
                .expect("options")
                .with_decoration(TextDecoration::Underline)
                .with_decoration_style(TextDecorationStyle::Dashed),
        )
        .expect("decorated layout");
    let origin = Point::new(Scalar::ZERO, scalar(1));
    let paint = Paint::new(Color::BLUE);

    let mut direct = Surface::new(30, 24, SurfaceLimits::default()).expect("direct surface");
    direct
        .canvas()
        .draw_text_layout(&layout, &fonts, origin, paint.clone())
        .expect("direct layout");

    let mut builder = DisplayListBuilder::new(64).expect("display-list limits");
    builder
        .draw_text_layout(&layout, origin, paint.clone())
        .expect("record layout");
    let list = builder.finish();
    let mut replay = Surface::new(30, 24, SurfaceLimits::default()).expect("replay surface");
    replay
        .execute_display_list(&list, &fonts)
        .expect("replay layout");
    assert_eq!(direct.pixels(), replay.pixels());

    let mut bounded = DisplayListBuilder::new(1).expect("tight limits");
    assert_eq!(
        bounded
            .draw_text_layout(&layout, origin, paint.clone())
            .expect_err("decoration commands exceed limit")
            .code(),
        SkiaErrorCode::ResourceLimit
    );
    let run = layout.lines()[0].paragraph().unwrap().runs()[0]
        .glyph_run()
        .clone();
    bounded
        .add_glyph_run(run)
        .expect("failed expansion rolls back glyph resources");
    bounded
        .clear(Color::TRANSPARENT)
        .expect("commands rolled back");
}

#[test]
fn styled_layout_resolves_per_span_paints_and_decorations() {
    let font = FontFace::from_bytes(
        FontId::new(121),
        toy_styled_font(&['A'], "Span Styles", FontStyle::NORMAL),
    )
    .expect("styled font");
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    fonts.add_face(font).expect("add styled font");
    let red_style = TextStyleId::new(1);
    let blue_style = TextStyleId::new(2);
    let spans = [
        TextStyleSpan::new(0, 1, FontId::new(121), 20 << 16)
            .expect("red span")
            .with_style_id(red_style)
            .with_decoration(TextDecoration::Underline),
        TextStyleSpan::new(1, 2, FontId::new(121), 20 << 16)
            .expect("blue span")
            .with_style_id(blue_style)
            .with_decoration(TextDecoration::StrikeThrough),
    ];
    let layout = fonts
        .layout_styled_text(
            "AA",
            &spans,
            TextLayoutOptions::new(30 << 16).expect("options"),
        )
        .expect("styled layout");
    let line = &layout.lines()[0];
    let runs = line.paragraph().expect("line").runs();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].style_id(), red_style);
    assert_eq!(runs[1].style_id(), blue_style);
    assert_eq!(line.decoration_segments().len(), 2);
    assert_eq!(line.decoration_segments()[0].style_id(), red_style);
    assert_eq!(line.decoration_segments()[0].left_bits(), 0);
    assert_eq!(line.decoration_segments()[0].right_bits(), 12 << 16);
    assert_eq!(line.decoration_segments()[1].style_id(), blue_style);
    assert_eq!(line.decoration_segments()[1].left_bits(), 12 << 16);
    assert_eq!(line.decoration_segments()[1].right_bits(), 24 << 16);

    let red = Paint::new(Color::RED);
    let blue = Paint::new(Color::BLUE);
    let mut surface = Surface::new(30, 24, SurfaceLimits::default()).expect("surface");
    surface
        .canvas()
        .draw_text_layout_with_styles(
            &layout,
            &fonts,
            Point::new(Scalar::ZERO, scalar(1)),
            &|style| match style {
                value if value == red_style => Some(red.clone()),
                value if value == blue_style => Some(blue.clone()),
                _ => None,
            },
        )
        .expect("draw styled layout");

    assert_eq!(pixel(&surface, 2, 18), Color::RED.channels());
    assert_eq!(pixel(&surface, 14, 10), Color::BLUE.channels());
    assert_eq!(pixel(&surface, 14, 18), Color::TRANSPARENT.channels());

    let error = surface
        .canvas()
        .draw_text_layout_with_styles(
            &layout,
            &fonts,
            Point::new(Scalar::ZERO, Scalar::ZERO),
            &|style| (style == red_style).then_some(red.clone()),
        )
        .expect_err("missing blue style must fail closed");
    assert_eq!(error.code(), SkiaErrorCode::InvalidResource);

    let patterned = fonts
        .layout_styled_text(
            "AA",
            &[
                spans[0].with_decoration_style(TextDecorationStyle::Dashed),
                spans[1].with_decoration(TextDecoration::Underline),
            ],
            TextLayoutOptions::new(30 << 16)
                .expect("pattern options")
                .with_decoration_style(TextDecorationStyle::Dotted),
        )
        .expect("span pattern overrides");
    assert_eq!(
        patterned.lines()[0].decoration_segments()[0].decoration_style(),
        TextDecorationStyle::Dashed
    );
    assert_eq!(
        patterned.lines()[0].decoration_segments()[1].decoration_style(),
        TextDecorationStyle::Dotted
    );

    let undecorated = fonts
        .layout_styled_text(
            "AA",
            &[
                TextStyleSpan::new(0, 1, FontId::new(121), 20 << 16)
                    .expect("red glyph span")
                    .with_style_id(red_style),
                TextStyleSpan::new(1, 2, FontId::new(121), 20 << 16)
                    .expect("blue glyph span")
                    .with_style_id(blue_style),
            ],
            TextLayoutOptions::new(30 << 16).expect("undecorated options"),
        )
        .expect("undecorated styled layout");
    let resolver = |style| match style {
        value if value == red_style => Some(red.clone()),
        value if value == blue_style => Some(blue.clone()),
        _ => None,
    };
    Surface::new(30, 24, SurfaceLimits::default())
        .expect("undecorated surface")
        .canvas()
        .draw_text_layout_with_styles(
            &undecorated,
            &fonts,
            Point::new(Scalar::ZERO, scalar(1)),
            &resolver,
        )
        .expect("glyph-only styled layout does not resolve default decoration paint");
    DisplayListBuilder::new(16)
        .expect("display-list limits")
        .draw_text_layout_with_styles(&undecorated, Point::new(Scalar::ZERO, scalar(1)), &resolver)
        .expect("glyph-only display list does not resolve default decoration paint");
}

fn scalar(value: i32) -> Scalar {
    Scalar::from_i32(value).expect("small scalar")
}

fn pixel(surface: &Surface, x: usize, y: usize) -> [u8; 4] {
    let offset = (y * surface.width() as usize + x) * 4;
    surface.pixels()[offset..offset + 4]
        .try_into()
        .expect("RGBA pixel")
}
