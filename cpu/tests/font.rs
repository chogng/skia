use skia_core::{
    Color, FontCollection, FontCollectionLimits, FontFace, FontFeature, FontId, FontLimits,
    FontSlant, FontStyle, FontTag, FontVariation, FontWidth, GlyphId, GlyphOutline,
    GlyphOutlineProvider, Paint, Point, Rect, Scalar, SkiaErrorCode, TextAlignment,
    TextBreakProvider, TextDecoration, TextDirection, TextError, TextErrorCode, TextLayoutOptions,
    TextWordBreak, TextWordBreakKind, Transform,
};
use skia_cpu::{Surface, SurfaceLimits};

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
fn public_font_loader_rejects_malformed_data() {
    let error = FontFace::from_bytes(FontId::new(1), b"not a font".to_vec())
        .expect_err("malformed font must fail");
    assert_eq!(error.code(), TextErrorCode::InvalidFontData);
}

#[test]
fn font_metadata_and_css_like_style_matching_are_deterministic() {
    assert_eq!(
        FontStyle::new(0, FontWidth::Normal, FontSlant::Normal)
            .expect_err("zero weight must fail")
            .code(),
        TextErrorCode::InvalidFontStyle
    );
    assert_eq!(
        FontStyle::new(1_001, FontWidth::Normal, FontSlant::Normal)
            .expect_err("weight above CSS range must fail")
            .code(),
        TextErrorCode::InvalidFontStyle
    );

    let style = |weight, width, slant| FontStyle::new(weight, width, slant).expect("valid style");
    let faces = [
        (
            FontId::new(100),
            "Example Sans",
            style(400, FontWidth::Normal, FontSlant::Normal),
        ),
        (
            FontId::new(101),
            "Example Sans",
            style(700, FontWidth::Normal, FontSlant::Normal),
        ),
        (
            FontId::new(102),
            "Example Sans",
            style(700, FontWidth::Condensed, FontSlant::Normal),
        ),
        (
            FontId::new(103),
            "Example Sans",
            style(400, FontWidth::Normal, FontSlant::Italic),
        ),
        (
            FontId::new(104),
            "Other Family",
            style(400, FontWidth::Normal, FontSlant::Normal),
        ),
        (
            FontId::new(105),
            "Example Sans",
            style(500, FontWidth::Normal, FontSlant::Normal),
        ),
        (
            FontId::new(106),
            "Example Sans",
            style(400, FontWidth::Normal, FontSlant::Oblique),
        ),
    ];
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    for (id, family, face_style) in faces {
        fonts
            .add_face(
                FontFace::from_bytes(id, toy_styled_font(&['A'], family, face_style))
                    .expect("styled font"),
            )
            .expect("add styled font");
    }

    let regular = fonts.face(FontId::new(100)).expect("regular face");
    assert_eq!(regular.family_name(), Some("Example Sans"));
    assert_eq!(regular.style(), FontStyle::NORMAL);
    assert_eq!(
        fonts.face(FontId::new(106)).expect("oblique face").style(),
        style(400, FontWidth::Normal, FontSlant::Oblique)
    );
    assert_eq!(
        fonts
            .match_face(
                "example sans",
                style(700, FontWidth::Normal, FontSlant::Normal)
            )
            .expect("bold match")
            .id(),
        FontId::new(101)
    );
    assert_eq!(
        fonts
            .match_face(
                "Example Sans",
                style(400, FontWidth::Condensed, FontSlant::Normal)
            )
            .expect("width takes priority")
            .id(),
        FontId::new(102)
    );
    assert_eq!(
        fonts
            .match_face(
                "Example Sans",
                style(700, FontWidth::Normal, FontSlant::Italic)
            )
            .expect("slant takes priority")
            .id(),
        FontId::new(103)
    );
    assert_eq!(
        fonts
            .match_face(
                "Example Sans",
                style(400, FontWidth::Normal, FontSlant::Oblique)
            )
            .expect("oblique match")
            .id(),
        FontId::new(106)
    );
    assert_eq!(
        fonts
            .match_face(
                "Example Sans",
                style(420, FontWidth::Normal, FontSlant::Normal)
            )
            .expect("CSS 400-500 preference")
            .id(),
        FontId::new(105)
    );
    assert_eq!(
        fonts
            .match_face(
                "Example Sans",
                style(450, FontWidth::Normal, FontSlant::Normal)
            )
            .expect("CSS 450-500 preference")
            .id(),
        FontId::new(100)
    );
    assert!(
        fonts
            .match_face("Missing Family", FontStyle::NORMAL)
            .is_none()
    );
    assert_eq!(
        fonts
            .match_face_for_families(&["Missing Family", "Other Family"], FontStyle::NORMAL)
            .expect("ordered family fallback")
            .id(),
        FontId::new(104)
    );
}

#[test]
fn variable_font_instances_validate_axes_and_keep_distinct_identities() {
    let base = FontFace::from_bytes(FontId::new(130), toy_variable_font(&['A'], "Variable Sans"))
        .expect("variable font");
    let weight = FontTag::new(*b"wght");
    assert_eq!(weight.bytes(), *b"wght");
    assert_eq!(base.variation_axes().len(), 1);
    let axis = base.variation_axes()[0];
    assert_eq!(axis.tag(), weight);
    assert_eq!(axis.min_value_bits(), 100 << 16);
    assert_eq!(axis.default_value_bits(), 400 << 16);
    assert_eq!(axis.max_value_bits(), 900 << 16);
    assert!(!axis.hidden());
    assert!(base.variations().is_empty());

    let coordinate = FontVariation::new(weight, 700 << 16);
    assert_eq!(coordinate.tag(), weight);
    assert_eq!(coordinate.value_bits(), 700 << 16);
    let instance = base
        .instantiate_variations(FontId::new(131), &[coordinate])
        .expect("weight instance");
    assert_eq!(instance.id(), FontId::new(131));
    assert_eq!(instance.variations(), &[coordinate]);
    let run = instance.shape("A", 10 << 16).expect("shape instance");
    assert_eq!(run.font(), FontId::new(131));
    assert!(
        instance
            .glyph_outline(run.font(), run.glyphs()[0].glyph())
            .expect("instance outline")
            .is_some()
    );

    let default_instance = base
        .instantiate_variations(FontId::new(132), &[FontVariation::new(weight, 400 << 16)])
        .expect("default instance");
    assert!(default_instance.variations().is_empty());
    for invalid in [
        vec![FontVariation::new(weight, 99 << 16)],
        vec![FontVariation::new(FontTag::new(*b"wdth"), 100 << 16)],
        vec![coordinate, coordinate],
    ] {
        assert_eq!(
            base.instantiate_variations(FontId::new(133), &invalid)
                .expect_err("invalid variation request")
                .code(),
            TextErrorCode::InvalidFontVariation
        );
    }
    assert_eq!(
        base.instantiate_variations(base.id(), &[coordinate])
            .expect_err("instance identity must be distinct")
            .code(),
        TextErrorCode::InvalidFontVariation
    );
}

#[test]
fn shaping_feature_instances_propagate_through_paragraph_and_layout() {
    let base =
        FontFace::from_bytes(FontId::new(140), toy_kerned_font(&['A'])).expect("kerned font");
    let default_run = base.shape("AA", 10 << 16).expect("default kerning");
    assert_eq!(
        default_run
            .glyphs()
            .iter()
            .map(|glyph| glyph.advance_x().bits())
            .sum::<i32>(),
        1_100 * 64
    );

    let kern = FontTag::new(*b"kern");
    let disabled_feature = FontFeature::new(kern, 0);
    assert_eq!(disabled_feature.tag(), kern);
    assert_eq!(disabled_feature.value(), 0);
    let disabled = base
        .instantiate_features(FontId::new(141), &[disabled_feature])
        .expect("disable kerning");
    assert_eq!(disabled.features(), &[disabled_feature]);
    let disabled_run = disabled.shape("AA", 10 << 16).expect("un-kerned shape");
    assert_eq!(disabled_run.font(), FontId::new(141));
    assert_eq!(
        disabled_run
            .glyphs()
            .iter()
            .map(|glyph| glyph.advance_x().bits())
            .sum::<i32>(),
        1_200 * 64
    );

    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    fonts.add_face(disabled).expect("add feature instance");
    assert_eq!(
        fonts
            .shape_paragraph("AA", 10 << 16)
            .expect("feature paragraph")
            .advance_x_bits(),
        12 << 16
    );
    assert_eq!(
        fonts
            .layout_text(
                "AA",
                10 << 16,
                TextLayoutOptions::new(20 << 16).expect("options"),
            )
            .expect("feature layout")
            .lines()[0]
            .advance_x_bits(),
        12 << 16
    );

    assert_eq!(
        base.instantiate_features(base.id(), &[disabled_feature])
            .expect_err("feature identity must be distinct")
            .code(),
        TextErrorCode::InvalidFontFeature
    );
    assert_eq!(
        base.instantiate_features(
            FontId::new(142),
            &[disabled_feature, FontFeature::new(kern, 1)],
        )
        .expect_err("duplicate tags must fail")
        .code(),
        TextErrorCode::InvalidFontFeature
    );
}

#[test]
fn font_limits_bound_shaping_and_outline_work() {
    let shaping_limits = FontLimits::new(1_024, 8, 1, 32).expect("valid limits");
    let face = FontFace::from_bytes_with_limits(FontId::new(2), toy_font('A'), 0, shaping_limits)
        .expect("load bounded font");
    assert_eq!(
        face.shape("AA", 10 << 16)
            .expect_err("two glyphs exceed the run limit")
            .code(),
        TextErrorCode::ResourceLimit
    );

    let outline_limits = FontLimits::new(1_024, 8, 8, 2).expect("valid limits");
    let face = FontFace::from_bytes_with_limits(FontId::new(3), toy_font('A'), 0, outline_limits)
        .expect("load bounded font");
    assert_eq!(
        face.glyph_outline(face.id(), GlyphId::new(1))
            .expect_err("square outline exceeds two segments")
            .code(),
        TextErrorCode::ResourceLimit
    );

    assert_eq!(
        FontFace::from_bytes_with_limits(FontId::new(4), toy_font('A'), 1, FontLimits::default())
            .expect_err("standalone font has no second face")
            .code(),
        TextErrorCode::InvalidFaceIndex
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
fn bidi_reorders_rtl_fallback_runs_and_preserves_source_clusters() {
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    fonts
        .add_face(FontFace::from_bytes(FontId::new(20), toy_font('\u{05d0}')).expect("Alef font"))
        .expect("add Alef font");
    fonts
        .add_face(FontFace::from_bytes(FontId::new(21), toy_font('\u{05d1}')).expect("Bet font"))
        .expect("add Bet font");

    let paragraph = fonts
        .shape_paragraph("\u{05d0}\u{05d1}", 10 << 16)
        .expect("RTL shape");

    assert_eq!(paragraph.base_direction(), TextDirection::RightToLeft);
    assert_eq!(paragraph.runs().len(), 2);
    assert_eq!(paragraph.runs()[0].glyph_run().font(), FontId::new(21));
    assert_eq!(paragraph.runs()[0].source_start(), 2);
    assert_eq!(paragraph.runs()[0].glyph_run().glyphs()[0].cluster(), 2);
    assert_eq!(paragraph.runs()[0].direction(), TextDirection::RightToLeft);
    assert_eq!(paragraph.runs()[1].glyph_run().font(), FontId::new(20));
    assert_eq!(paragraph.runs()[1].source_start(), 0);
    assert_eq!(paragraph.runs()[1].glyph_run().glyphs()[0].cluster(), 0);
    assert_eq!(paragraph.advance_x_bits(), 12 << 16);
}

#[test]
fn collection_rejects_duplicate_faces_missing_glyphs_and_multiple_paragraphs() {
    let mut fonts = FontCollection::new(
        FontCollectionLimits::new(2, 32, 8, 8).expect("valid collection limits"),
    );
    fonts
        .add_face(FontFace::from_bytes(FontId::new(30), toy_font('A')).expect("A font"))
        .expect("add font");
    assert_eq!(
        fonts
            .add_face(FontFace::from_bytes(FontId::new(30), toy_font('B')).expect("B font"))
            .expect_err("duplicate ID must fail")
            .code(),
        TextErrorCode::DuplicateFontId
    );
    assert_eq!(
        fonts
            .shape_paragraph("B", 10 << 16)
            .expect_err("missing fallback must fail")
            .code(),
        TextErrorCode::MissingGlyph
    );
    assert_eq!(
        fonts
            .shape_paragraph("A\nA", 10 << 16)
            .expect_err("line layout is not paragraph shaping")
            .code(),
        TextErrorCode::MultipleParagraphs
    );
}

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
fn hard_breaks_trailing_empty_lines_and_long_graphemes_are_bounded() {
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    fonts
        .add_face(FontFace::from_bytes(FontId::new(60), toy_font('A')).expect("A font"))
        .expect("add font");

    let hard = fonts
        .layout_text(
            "A\nA",
            10 << 16,
            TextLayoutOptions::new(20 << 16).expect("options"),
        )
        .expect("hard break");
    assert_eq!(hard.lines().len(), 2);
    assert!(hard.lines()[0].hard_break());
    assert_eq!(
        (hard.lines()[0].source_start(), hard.lines()[0].source_end()),
        (0, 1)
    );
    assert_eq!(
        (hard.lines()[1].source_start(), hard.lines()[1].source_end()),
        (2, 3)
    );

    let trailing = fonts
        .layout_text(
            "A\n",
            10 << 16,
            TextLayoutOptions::new(20 << 16).expect("options"),
        )
        .expect("trailing line");
    assert_eq!(trailing.lines().len(), 2);
    assert!(trailing.lines()[1].paragraph().is_none());
    assert_eq!(
        (
            trailing.lines()[1].source_start(),
            trailing.lines()[1].source_end()
        ),
        (2, 2)
    );

    let forced = fonts
        .layout_text(
            "AAA",
            10 << 16,
            TextLayoutOptions::new(7 << 16).expect("options"),
        )
        .expect("forced grapheme wrap");
    assert_eq!(forced.lines().len(), 3);
    assert_eq!(forced.lines()[0].source_end(), 1);
    assert_eq!(forced.lines()[1].source_end(), 2);
    assert_eq!(forced.lines()[2].source_end(), 3);

    assert_eq!(
        fonts
            .layout_text(
                "AAA",
                10 << 16,
                TextLayoutOptions::with_limits(7 << 16, 2, 32).expect("bounded options"),
            )
            .expect_err("line limit must fail")
            .code(),
        TextErrorCode::ResourceLimit
    );
}

struct FixedBreakProvider {
    language: &'static str,
    opportunities: Vec<TextWordBreak>,
}

impl TextBreakProvider for FixedBreakProvider {
    fn opportunities(&self, _word: &str, language: &str) -> Result<Vec<TextWordBreak>, TextError> {
        if language != self.language {
            return Err(TextError::new(TextErrorCode::InvalidLanguage));
        }
        Ok(self.opportunities.clone())
    }
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
fn dictionary_hyphenation_positions_rtl_hyphens_and_rejects_invalid_providers() {
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    fonts
        .add_face(
            FontFace::from_bytes(
                FontId::new(66),
                toy_font_for(&[
                    '-', 'A', 'B', 'a', 'b', '\u{0301}', '\u{05d0}', '\u{05d1}', '\u{05d2}',
                    '\u{05d3}',
                ]),
            )
            .expect("mixed font"),
        )
        .expect("add font");
    let rtl = fonts
        .layout_text_with_break_provider(
            "\u{05d0}\u{05d1}\u{05d2}\u{05d3}",
            10 << 16,
            TextLayoutOptions::new(19 << 16).expect("options"),
            "he",
            &FixedBreakProvider {
                language: "he",
                opportunities: vec![TextWordBreak::new(4, TextWordBreakKind::Hyphenated)],
            },
        )
        .expect("RTL hyphenation");
    assert_eq!(rtl.lines().len(), 2);
    assert!(rtl.lines()[0].hyphenated());
    let paragraph = rtl.lines()[0].paragraph().expect("RTL first line");
    assert_eq!(paragraph.base_direction(), TextDirection::RightToLeft);
    assert_eq!(paragraph.runs()[0].source_start(), 4);
    assert_eq!(paragraph.runs()[0].source_end(), 4);
    assert_eq!(paragraph.runs()[0].origin_x_bits(), 0);
    assert_eq!(paragraph.runs()[1].origin_x_bits(), 6 << 16);

    let soft = fonts
        .layout_text_with_break_provider(
            "ABAB",
            10 << 16,
            TextLayoutOptions::new(12 << 16).expect("options"),
            "th",
            &FixedBreakProvider {
                language: "th",
                opportunities: vec![TextWordBreak::new(2, TextWordBreakKind::Soft)],
            },
        )
        .expect("dictionary soft break");
    assert_eq!(soft.lines().len(), 2);
    assert_eq!(soft.lines()[0].source_end(), 2);
    assert!(!soft.lines()[0].hyphenated());
    assert_eq!(
        soft.lines()[0].paragraph().expect("soft line").runs().len(),
        1
    );

    assert_eq!(
        fonts
            .layout_text_with_break_provider(
                "A",
                10 << 16,
                TextLayoutOptions::new(20 << 16).expect("options"),
                "en_US",
                &FixedBreakProvider {
                    language: "en_US",
                    opportunities: Vec::new(),
                },
            )
            .expect_err("invalid language tag")
            .code(),
        TextErrorCode::InvalidLanguage
    );
    assert_eq!(
        fonts
            .layout_text_with_break_provider(
                "a\u{0301}b",
                10 << 16,
                TextLayoutOptions::new(20 << 16).expect("options"),
                "en",
                &FixedBreakProvider {
                    language: "en",
                    opportunities: vec![TextWordBreak::new(1, TextWordBreakKind::Soft)],
                },
            )
            .expect_err("provider split a grapheme")
            .code(),
        TextErrorCode::InvalidWordBreak
    );
    assert_eq!(
        fonts
            .layout_text_with_break_provider(
                "AAA",
                10 << 16,
                TextLayoutOptions::with_limits(20 << 16, 8, 1).expect("bounded options"),
                "en",
                &FixedBreakProvider {
                    language: "en",
                    opportunities: vec![
                        TextWordBreak::new(1, TextWordBreakKind::Soft),
                        TextWordBreak::new(2, TextWordBreakKind::Soft),
                    ],
                },
            )
            .expect_err("dictionary opportunities exceed work ceiling")
            .code(),
        TextErrorCode::ResourceLimit
    );
}

#[test]
fn soft_wrapped_rtl_line_keeps_the_logical_paragraph_base_direction() {
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    fonts
        .add_face(
            FontFace::from_bytes(FontId::new(70), toy_font_for(&[' ', 'A', '\u{05d0}']))
                .expect("mixed font"),
        )
        .expect("add font");

    let layout = fonts
        .layout_text(
            "A \u{05d0}\u{05d0}",
            10 << 16,
            TextLayoutOptions::new(12 << 16).expect("options"),
        )
        .expect("mixed bidi wrap");
    assert_eq!(layout.lines().len(), 2);
    let rtl_line = layout.lines()[1].paragraph().expect("RTL content");
    assert_eq!(rtl_line.base_direction(), TextDirection::LeftToRight);
    assert_eq!(rtl_line.runs()[0].direction(), TextDirection::RightToLeft);
    assert_eq!(rtl_line.runs()[0].glyph_run().glyphs()[0].cluster(), 4);
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
fn justification_expands_unicode_spaces_but_not_non_breaking_spaces() {
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    fonts
        .add_face(
            FontFace::from_bytes(
                FontId::new(100),
                toy_font_for(&['A', '\u{00a0}', '\u{2007}', '\u{202f}', '\u{3000}']),
            )
            .expect("Unicode space font"),
        )
        .expect("add font");
    let justify_final = TextLayoutOptions::new(24 << 16)
        .expect("options")
        .with_alignment(TextAlignment::Justify)
        .with_justify_last_line(true);

    let ideographic = fonts
        .layout_text("A\u{3000}A", 10 << 16, justify_final)
        .expect("ideographic space layout");
    assert!(ideographic.lines()[0].justified());
    assert_eq!(ideographic.lines()[0].advance_x_bits(), 24 << 16);
    assert_eq!(
        ideographic.lines()[0].paragraph().expect("line").runs()[0].glyph_offsets_x_bits(),
        &[0, 0, 6 << 16]
    );

    for text in ["A\u{00a0}A", "A\u{2007}A", "A\u{202f}A"] {
        let non_breaking = fonts
            .layout_text(text, 10 << 16, justify_final)
            .expect("non-breaking space layout");
        assert!(!non_breaking.lines()[0].justified());
        assert_eq!(non_breaking.lines()[0].advance_x_bits(), 18 << 16);
        assert_eq!(
            non_breaking.lines()[0].paragraph().expect("line").runs()[0].glyph_offsets_x_bits(),
            &[0, 0, 0]
        );
    }
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

    assert_eq!(pixel(&surface, 18, 10), color);
    assert_eq!(pixel(&surface, 18, 18), color);
    assert_eq!(pixel(&surface, 2, 18), [0, 0, 0, 0]);
}

#[test]
fn requested_font_decoration_requires_native_metrics() {
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    fonts
        .add_face(FontFace::from_bytes(FontId::new(120), toy_font('A')).expect("plain font"))
        .expect("add plain font");
    let error = fonts
        .layout_text(
            "A",
            10 << 16,
            TextLayoutOptions::new(20 << 16)
                .expect("options")
                .with_decoration(TextDecoration::Underline),
        )
        .expect_err("font without post metrics must fail");
    assert_eq!(error.code(), TextErrorCode::MissingDecorationMetrics);

    let empty = fonts
        .layout_text(
            "",
            10 << 16,
            TextLayoutOptions::new(20 << 16)
                .expect("options")
                .with_decoration(TextDecoration::Underline),
        )
        .expect("empty lines need no decoration metrics");
    assert_eq!(empty.lines()[0].underline_metrics(), None);
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

fn toy_font(character: char) -> Vec<u8> {
    toy_font_for(&[character])
}

fn toy_font_for(characters: &[char]) -> Vec<u8> {
    build_toy_font(characters, None, false, false)
}

fn toy_styled_font(characters: &[char], family: &str, style: FontStyle) -> Vec<u8> {
    build_toy_font(characters, Some((family, style)), false, false)
}

fn toy_variable_font(characters: &[char], family: &str) -> Vec<u8> {
    build_toy_font(characters, Some((family, FontStyle::NORMAL)), true, false)
}

fn toy_kerned_font(characters: &[char]) -> Vec<u8> {
    build_toy_font(characters, None, false, true)
}

fn build_toy_font(
    characters: &[char],
    metadata: Option<(&str, FontStyle)>,
    variable: bool,
    kerned: bool,
) -> Vec<u8> {
    let mut tables = vec![
        (*b"cmap", cmap_table(characters)),
        (*b"glyf", glyf_table()),
        (*b"head", head_table()),
        (*b"hhea", hhea_table()),
        (*b"hmtx", hmtx_table()),
        (*b"loca", loca_table()),
        (*b"maxp", maxp_table()),
    ];
    if let Some((family, style)) = metadata {
        tables.push((*b"name", name_table(family)));
        tables.push((*b"OS/2", os2_table(style)));
        tables.push((*b"post", post_table()));
    }
    if variable {
        tables.push((*b"fvar", fvar_table()));
    }
    if kerned {
        tables.push((*b"kern", kern_table()));
    }
    tables.sort_unstable_by_key(|(tag, _)| *tag);
    let table_count = u16::try_from(tables.len()).expect("small table count");
    let directory_len = 12 + tables.len() * 16;
    let mut font = vec![0; directory_len];
    put_u32(&mut font, 0, 0x0001_0000);
    put_u16(&mut font, 4, table_count);
    put_u16(&mut font, 6, 64);
    put_u16(&mut font, 8, 2);
    put_u16(&mut font, 10, 48);

    let mut offset = directory_len;
    for (index, (tag, data)) in tables.iter().enumerate() {
        let record = 12 + index * 16;
        font[record..record + 4].copy_from_slice(tag);
        put_u32(
            &mut font,
            record + 8,
            u32::try_from(offset).expect("small font"),
        );
        put_u32(
            &mut font,
            record + 12,
            u32::try_from(data.len()).expect("small table"),
        );
        font.extend_from_slice(data);
        offset += data.len();
        while !offset.is_multiple_of(4) {
            font.push(0);
            offset += 1;
        }
    }
    font
}

fn name_table(family: &str) -> Vec<u8> {
    let encoded: Vec<u8> = family.encode_utf16().flat_map(u16::to_be_bytes).collect();
    let mut table = vec![0; 18];
    put_u16(&mut table, 0, 0);
    put_u16(&mut table, 2, 1);
    put_u16(&mut table, 4, 18);
    put_u16(&mut table, 6, 3);
    put_u16(&mut table, 8, 1);
    put_u16(&mut table, 10, 0x0409);
    put_u16(&mut table, 12, 16);
    put_u16(
        &mut table,
        14,
        u16::try_from(encoded.len()).expect("short family"),
    );
    put_u16(&mut table, 16, 0);
    table.extend(encoded);
    table
}

fn os2_table(style: FontStyle) -> Vec<u8> {
    let mut table = vec![0; 96];
    put_u16(&mut table, 0, 4);
    put_u16(&mut table, 4, style.weight());
    put_u16(&mut table, 6, style.width().class());
    put_i16(&mut table, 26, 100);
    put_i16(&mut table, 28, 300);
    let selection = match style.slant() {
        FontSlant::Normal => 0,
        FontSlant::Italic => 1,
        FontSlant::Oblique => 1 << 9,
    };
    put_u16(&mut table, 62, selection);
    table
}

fn post_table() -> Vec<u8> {
    let mut table = vec![0; 32];
    put_u32(&mut table, 0, 0x0003_0000);
    put_i16(&mut table, 8, -100);
    put_i16(&mut table, 10, 100);
    table
}

fn fvar_table() -> Vec<u8> {
    let mut table = vec![0; 36];
    put_u16(&mut table, 0, 1);
    put_u16(&mut table, 2, 0);
    put_u16(&mut table, 4, 16);
    put_u16(&mut table, 8, 1);
    put_u16(&mut table, 10, 20);
    put_u16(&mut table, 12, 0);
    put_u16(&mut table, 14, 8);
    table[16..20].copy_from_slice(b"wght");
    put_u32(&mut table, 20, 100 << 16);
    put_u32(&mut table, 24, 400 << 16);
    put_u32(&mut table, 28, 900 << 16);
    put_u16(&mut table, 34, 256);
    table
}

fn kern_table() -> Vec<u8> {
    let mut table = vec![0; 24];
    put_u16(&mut table, 2, 1);
    put_u16(&mut table, 6, 20);
    put_u16(&mut table, 8, 1);
    put_u16(&mut table, 10, 1);
    put_u16(&mut table, 12, 6);
    put_u16(&mut table, 18, 1);
    put_u16(&mut table, 20, 1);
    put_i16(&mut table, 22, -100);
    table
}

fn cmap_table(characters: &[char]) -> Vec<u8> {
    let mut characters: Vec<u16> = characters
        .iter()
        .copied()
        .map(|character| {
            u16::try_from(u32::from(character)).expect("toy font supports BMP characters")
        })
        .collect();
    characters.sort_unstable();
    characters.dedup();
    assert!(!characters.is_empty());
    assert!(!characters.contains(&0xffff));
    let segment_count = u16::try_from(characters.len() + 1).expect("small segment count");
    let power = 1_u16 << segment_count.ilog2();
    let search_range = power * 2;
    let entry_selector = u16::try_from(power.ilog2()).expect("small entry selector");
    let segment_count_x2 = segment_count * 2;
    let range_shift = segment_count_x2 - search_range;
    let length = 16 + usize::from(segment_count) * 8;
    let mut table = Vec::new();
    push_u16(&mut table, 0);
    push_u16(&mut table, 1);
    push_u16(&mut table, 3);
    push_u16(&mut table, 1);
    push_u32(&mut table, 12);
    push_u16(&mut table, 4);
    push_u16(&mut table, u16::try_from(length).expect("small cmap"));
    push_u16(&mut table, 0);
    push_u16(&mut table, segment_count_x2);
    push_u16(&mut table, search_range);
    push_u16(&mut table, entry_selector);
    push_u16(&mut table, range_shift);
    for character in &characters {
        push_u16(&mut table, *character);
    }
    push_u16(&mut table, 0xffff);
    push_u16(&mut table, 0);
    for character in &characters {
        push_u16(&mut table, *character);
    }
    push_u16(&mut table, 0xffff);
    for character in &characters {
        push_u16(&mut table, 1_u16.wrapping_sub(*character));
    }
    push_i16(&mut table, 1);
    for _ in 0..segment_count {
        push_u16(&mut table, 0);
    }
    table
}

fn glyf_table() -> Vec<u8> {
    let mut table = Vec::new();
    push_i16(&mut table, 1);
    push_i16(&mut table, 0);
    push_i16(&mut table, 0);
    push_i16(&mut table, 500);
    push_i16(&mut table, 700);
    push_u16(&mut table, 3);
    push_u16(&mut table, 0);
    table.extend([1, 1, 1, 1]);
    for delta in [0, 500, 0, -500] {
        push_i16(&mut table, delta);
    }
    for delta in [0, 0, 700, 0] {
        push_i16(&mut table, delta);
    }
    table
}

fn head_table() -> Vec<u8> {
    let mut table = vec![0; 54];
    put_u32(&mut table, 0, 0x0001_0000);
    put_u32(&mut table, 12, 0x5f0f_3cf5);
    put_u16(&mut table, 18, 1_000);
    put_u16(&mut table, 46, 8);
    put_u16(&mut table, 50, 0);
    table
}

fn hhea_table() -> Vec<u8> {
    let mut table = vec![0; 36];
    put_u32(&mut table, 0, 0x0001_0000);
    put_i16(&mut table, 4, 800);
    put_i16(&mut table, 6, -200);
    put_u16(&mut table, 10, 600);
    put_i16(&mut table, 18, 1);
    put_u16(&mut table, 34, 2);
    table
}

fn hmtx_table() -> Vec<u8> {
    let mut table = Vec::new();
    push_u16(&mut table, 600);
    push_i16(&mut table, 0);
    push_u16(&mut table, 600);
    push_i16(&mut table, 0);
    table
}

fn loca_table() -> Vec<u8> {
    let mut table = Vec::new();
    push_u16(&mut table, 0);
    push_u16(&mut table, 0);
    push_u16(&mut table, 17);
    table
}

fn maxp_table() -> Vec<u8> {
    let mut table = vec![0; 32];
    put_u32(&mut table, 0, 0x0001_0000);
    put_u16(&mut table, 4, 2);
    table
}

fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend(value.to_be_bytes());
}

fn push_i16(bytes: &mut Vec<u8>, value: i16) {
    bytes.extend(value.to_be_bytes());
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend(value.to_be_bytes());
}

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn put_i16(bytes: &mut [u8], offset: usize, value: i16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}
