use skia_text::{
    FontCollection, FontCollectionLimits, FontFace, FontFeature, FontId, FontLimits, FontSlant,
    FontStyle, FontTag, FontVariation, FontWidth, GlyphBitmapFormat, GlyphId, GlyphOutlineProvider,
    TextAffinity, TextAlignment, TextDecoration, TextDirection, TextErrorCode, TextJustification,
    TextLayoutOptions, TextOverflow, TextPosition, TextStyleSpan, TextWordBreak, TextWordBreakKind,
};

#[path = "../../test-support/font.rs"]
mod font_support;

use font_support::*;

#[test]
fn public_font_loader_rejects_malformed_data() {
    let error = FontFace::from_bytes(FontId::new(1), b"not a font".to_vec())
        .expect_err("malformed font must fail");
    assert_eq!(error.code(), TextErrorCode::InvalidFontData);
}

#[test]
fn font_face_rasterizes_hinted_alpha_glyphs_without_native_font_libraries() {
    let face = FontFace::from_bytes(FontId::new(8), toy_font('A')).expect("load toy font");
    let glyph = face
        .glyph_for_character('A')
        .expect("lookup glyph")
        .expect("toy font covers A");

    let bitmap = face
        .rasterize_glyph(glyph, 12 << 16)
        .expect("rasterize glyph")
        .expect("A has an outline");

    assert_eq!(bitmap.font(), face.id());
    assert_eq!(bitmap.glyph(), glyph);
    assert_eq!(bitmap.font_size_bits(), 12 << 16);
    assert_eq!(bitmap.format(), GlyphBitmapFormat::Alpha8);
    assert_eq!(bitmap.format().bytes_per_pixel(), 1);
    assert!(bitmap.width() > 0);
    assert!(bitmap.height() > 0);
    assert_eq!(
        bitmap.pixels().len(),
        bitmap.width() as usize * bitmap.height() as usize
    );
    assert!(bitmap.pixels().iter().any(|coverage| *coverage != 0));
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
fn shaping_language_propagates_through_fallback_bidi_styles_layout_and_markers() {
    let face = FontFace::from_bytes(
        FontId::new(145),
        toy_localized_font(&['-', 'A', '\u{05d0}', '\u{2026}']),
    )
    .expect("localized font");
    assert_eq!(
        face.shape("A", 10 << 16).expect("default shape").glyphs()[0]
            .glyph()
            .value(),
        1
    );
    assert_eq!(
        face.shape_with_language("A", 10 << 16, "sr")
            .expect("Serbian shape")
            .glyphs()[0]
            .glyph()
            .value(),
        2
    );
    assert_eq!(
        face.shape_with_direction_and_language("A", 10 << 16, TextDirection::LeftToRight, "sr",)
            .expect("directed Serbian shape")
            .glyphs()[0]
            .glyph()
            .value(),
        2
    );
    assert_eq!(
        face.shape_with_language("A", 10 << 16, "en_US")
            .expect_err("invalid language tag")
            .code(),
        TextErrorCode::InvalidLanguage
    );

    let mut fallback_fonts = FontCollection::new(FontCollectionLimits::default());
    fallback_fonts
        .add_face(FontFace::from_bytes(FontId::new(146), toy_font('B')).expect("primary font"))
        .expect("add primary font");
    fallback_fonts
        .add_face(face)
        .expect("add localized fallback");
    let fallback = fallback_fonts
        .shape_paragraph_with_language("A", 10 << 16, "sr")
        .expect("localized fallback");
    assert_eq!(fallback.runs()[0].glyph_run().font(), FontId::new(145));
    assert_eq!(
        fallback.runs()[0].glyph_run().glyphs()[0].glyph().value(),
        2
    );

    let bidi = fallback_fonts
        .shape_paragraph_with_direction_and_language(
            "A\u{05d0}",
            10 << 16,
            TextDirection::LeftToRight,
            "sr",
        )
        .expect("localized bidi paragraph");
    assert_eq!(
        bidi.runs()
            .iter()
            .find(|run| run.source_start() == 0)
            .expect("Latin run")
            .glyph_run()
            .glyphs()[0]
            .glyph()
            .value(),
        2
    );

    let spans = [
        TextStyleSpan::new(0, 1, FontId::new(145), 10 << 16).expect("first span"),
        TextStyleSpan::new(1, 2, FontId::new(145), 12 << 16).expect("second span"),
    ];
    let styled = fallback_fonts
        .shape_styled_paragraph_with_language("AA", &spans, "sr")
        .expect("localized styled paragraph");
    assert!(
        styled
            .runs()
            .iter()
            .flat_map(|run| run.glyph_run().glyphs())
            .all(|glyph| glyph.glyph().value() == 2)
    );
    let styled_layout = fallback_fonts
        .layout_styled_text_with_language(
            "AA",
            &spans,
            TextLayoutOptions::new(20 << 16).expect("styled options"),
            "sr",
        )
        .expect("localized styled layout");
    assert!(
        styled_layout.lines()[0]
            .paragraph()
            .expect("styled line")
            .runs()
            .iter()
            .flat_map(|run| run.glyph_run().glyphs())
            .all(|glyph| glyph.glyph().value() == 2)
    );

    let ellipsized = fallback_fonts
        .layout_text_with_language(
            "AAA",
            10 << 16,
            TextLayoutOptions::with_limits(12 << 16, 1, 64)
                .expect("ellipsis options")
                .with_overflow(TextOverflow::Ellipsis),
            "sr",
        )
        .expect("localized ellipsis");
    assert!(ellipsized.lines()[0].ellipsized());
    assert!(
        ellipsized.lines()[0]
            .paragraph()
            .expect("ellipsis line")
            .runs()
            .iter()
            .flat_map(|run| run.glyph_run().glyphs())
            .all(|glyph| glyph.glyph().value() == 2)
    );

    let provider_layout = fallback_fonts
        .layout_text_with_break_provider(
            "AA",
            10 << 16,
            TextLayoutOptions::new(20 << 16).expect("provider options"),
            "sr",
            &FixedBreakProvider {
                language: "sr",
                opportunities: Vec::new(),
            },
        )
        .expect("provider language also shapes");
    assert!(
        provider_layout.lines()[0]
            .paragraph()
            .expect("provider line")
            .runs()[0]
            .glyph_run()
            .glyphs()
            .iter()
            .all(|glyph| glyph.glyph().value() == 2)
    );
    assert_eq!(
        fallback_fonts
            .layout_text_with_language(
                "A",
                10 << 16,
                TextLayoutOptions::new(20 << 16).expect("invalid-language options"),
                "",
            )
            .expect_err("empty language tag")
            .code(),
        TextErrorCode::InvalidLanguage
    );
}

#[test]
fn styled_layout_wraps_with_per_line_metrics_decorations_and_hyphens() {
    let characters = ['-', 'A', 'a', 'e', 'h', 'i', 'n', 'o', 'p', 't', 'y'];
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    fonts
        .add_face(
            FontFace::from_bytes(
                FontId::new(152),
                toy_styled_font(&characters, "Small", FontStyle::NORMAL),
            )
            .expect("small styled font"),
        )
        .expect("add small font");
    fonts
        .add_face(
            FontFace::from_bytes(
                FontId::new(153),
                toy_styled_font(&characters, "Large", FontStyle::NORMAL),
            )
            .expect("large styled font"),
        )
        .expect("add large font");

    let wrapped_spans = [
        TextStyleSpan::new(0, 2, FontId::new(152), 10 << 16).expect("small span"),
        TextStyleSpan::new(2, 4, FontId::new(153), 20 << 16).expect("large span"),
    ];
    let wrapped = fonts
        .layout_styled_text(
            "AAAA",
            &wrapped_spans,
            TextLayoutOptions::new(15 << 16)
                .expect("options")
                .with_decoration(TextDecoration::UnderlineAndStrikeThrough),
        )
        .expect("styled wrap");
    assert_eq!(wrapped.lines().len(), 3);
    assert_eq!(wrapped.height_bits(), 50 << 16);
    assert_eq!(
        wrapped
            .lines()
            .iter()
            .map(|line| (line.source_start(), line.source_end()))
            .collect::<Vec<_>>(),
        vec![(0, 2), (2, 3), (3, 4)]
    );
    assert_eq!(
        wrapped
            .lines()
            .iter()
            .map(|line| line.baseline_y_bits())
            .collect::<Vec<_>>(),
        vec![8 << 16, 26 << 16, 46 << 16]
    );
    assert_eq!(
        wrapped.lines()[0]
            .underline_metrics()
            .expect("small underline")
            .thickness_bits(),
        1 << 16
    );
    assert_eq!(
        wrapped.lines()[1]
            .underline_metrics()
            .expect("large underline")
            .thickness_bits(),
        2 << 16
    );
    assert_eq!(
        wrapped.lines()[1].paragraph().expect("second line").runs()[0]
            .glyph_run()
            .font(),
        FontId::new(153)
    );

    let hard_break_spans = [
        TextStyleSpan::new(0, 3, FontId::new(152), 10 << 16).expect("first line span"),
        TextStyleSpan::new(3, 6, FontId::new(153), 20 << 16).expect("second line span"),
    ];
    let hard_breaks = fonts
        .layout_styled_text(
            "AA\n\nAA",
            &hard_break_spans,
            TextLayoutOptions::new(30 << 16).expect("options"),
        )
        .expect("styled hard breaks");
    assert_eq!(hard_breaks.lines().len(), 3);
    assert!(hard_breaks.lines()[1].paragraph().is_none());
    assert_eq!(hard_breaks.lines()[1].metrics().ascent_bits(), 16 << 16);
    assert_eq!(hard_breaks.lines()[1].baseline_y_bits(), 26 << 16);
    assert_eq!(hard_breaks.height_bits(), 50 << 16);

    let hyphenation_spans = [
        TextStyleSpan::new(0, 2, FontId::new(152), 10 << 16).expect("hyphen owner"),
        TextStyleSpan::new(2, 11, FontId::new(153), 20 << 16).expect("remaining word"),
    ];
    let hyphenated = fonts
        .layout_styled_text_with_break_provider(
            "hyphenation",
            &hyphenation_spans,
            TextLayoutOptions::new(31 << 16).expect("options"),
            "en-US",
            &FixedBreakProvider {
                language: "en-US",
                opportunities: vec![TextWordBreak::new(2, TextWordBreakKind::Hyphenated)],
            },
        )
        .expect("styled hyphenation");
    let synthetic = &hyphenated.lines()[0]
        .paragraph()
        .expect("first line")
        .runs()[1]
        .glyph_run();
    assert!(hyphenated.lines()[0].hyphenated());
    assert_eq!(synthetic.font(), FontId::new(152));
    assert_eq!(synthetic.font_size_bits(), 10 << 16);

    assert_eq!(
        fonts
            .layout_styled_text(
                "AAAA",
                &[TextStyleSpan::new(0, 3, FontId::new(152), 10 << 16).expect("partial span")],
                TextLayoutOptions::new(20 << 16).expect("options"),
            )
            .expect_err("partial coverage must fail")
            .code(),
        TextErrorCode::InvalidTextStyleSpan
    );
    assert_eq!(
        fonts
            .layout_styled_text(
                "A\r\nA",
                &[
                    TextStyleSpan::new(0, 2, FontId::new(152), 10 << 16).expect("split CRLF left"),
                    TextStyleSpan::new(2, 4, FontId::new(153), 20 << 16).expect("split CRLF right"),
                ],
                TextLayoutOptions::new(20 << 16).expect("options"),
            )
            .expect_err("span must not split a CRLF grapheme")
            .code(),
        TextErrorCode::InvalidTextStyleSpan
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

#[test]
fn line_limit_overflow_clips_or_adds_style_aware_ellipses() {
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    fonts
        .add_face(
            FontFace::from_bytes(FontId::new(61), toy_font_for(&['.', 'A', '\u{2026}']))
                .expect("ellipsis font"),
        )
        .expect("add ellipsis font");
    let clip_options = TextLayoutOptions::with_limits(18 << 16, 2, 128)
        .expect("clip options")
        .with_overflow(TextOverflow::Clip);
    assert_eq!(clip_options.overflow(), TextOverflow::Clip);
    let clipped = fonts
        .layout_text("AAAAAAA", 10 << 16, clip_options)
        .expect("clipped layout");
    assert!(clipped.truncated());
    assert_eq!(clipped.lines().len(), 2);
    assert_eq!(clipped.lines()[1].source_end(), 6);
    assert!(!clipped.lines()[1].ellipsized());

    let ellipsis_options = TextLayoutOptions::with_limits(18 << 16, 2, 128)
        .expect("ellipsis options")
        .with_overflow(TextOverflow::Ellipsis);
    let ellipsized = fonts
        .layout_text("AAAAAAA", 10 << 16, ellipsis_options)
        .expect("ellipsized layout");
    assert!(ellipsized.truncated());
    assert_eq!(ellipsized.lines().len(), 2);
    let last = &ellipsized.lines()[1];
    assert!(last.ellipsized());
    assert!(!last.hyphenated());
    assert!(!last.hard_break());
    assert_eq!((last.source_start(), last.source_end()), (3, 5));
    assert_eq!(last.advance_x_bits(), 18 << 16);
    let paragraph = last.paragraph().expect("ellipsized line");
    assert_eq!(paragraph.runs().len(), 2);
    assert_eq!(
        (
            paragraph.runs()[1].source_start(),
            paragraph.runs()[1].source_end()
        ),
        (5, 5)
    );
    assert_eq!(
        ellipsized
            .caret_for_position(TextPosition::new(5, TextAffinity::Upstream))
            .expect("ellipsis caret query")
            .expect("ellipsis caret")
            .x_bits(),
        18 << 16
    );

    let exact = fonts
        .layout_text("AAAAAA", 10 << 16, ellipsis_options)
        .expect("exact line count");
    assert!(!exact.truncated());
    assert!(!exact.lines()[1].ellipsized());

    let mut period_fonts = FontCollection::new(FontCollectionLimits::default());
    period_fonts
        .add_face(
            FontFace::from_bytes(FontId::new(62), toy_font_for(&['.', 'A']))
                .expect("period fallback font"),
        )
        .expect("add period font");
    let periods = period_fonts
        .layout_text("AAAAAAA", 10 << 16, ellipsis_options)
        .expect("three-period fallback");
    let period_line = &periods.lines()[1];
    assert_eq!(
        (period_line.source_start(), period_line.source_end()),
        (3, 3)
    );
    assert_eq!(
        period_line.paragraph().expect("period line").runs()[0]
            .glyph_run()
            .glyphs()
            .len(),
        3
    );

    let mut styled_fonts = FontCollection::new(FontCollectionLimits::default());
    styled_fonts
        .add_face(
            FontFace::from_bytes(FontId::new(63), toy_font_for(&['A', '\u{2026}']))
                .expect("small font"),
        )
        .expect("add small font");
    styled_fonts
        .add_face(
            FontFace::from_bytes(FontId::new(64), toy_font_for(&['A', '\u{2026}']))
                .expect("large font"),
        )
        .expect("add large font");
    let styled = styled_fonts
        .layout_styled_text(
            "AAAAAAA",
            &[
                TextStyleSpan::new(0, 3, FontId::new(63), 10 << 16).expect("small span"),
                TextStyleSpan::new(3, 7, FontId::new(64), 20 << 16).expect("large span"),
            ],
            TextLayoutOptions::with_limits(24 << 16, 2, 128)
                .expect("styled options")
                .with_overflow(TextOverflow::Ellipsis),
        )
        .expect("styled ellipsis");
    let styled_last = styled.lines()[1].paragraph().expect("styled last line");
    assert_eq!(styled.lines()[1].source_end(), 4);
    assert_eq!(styled_last.runs()[1].glyph_run().font(), FontId::new(64));
    assert_eq!(styled_last.runs()[1].glyph_run().font_size_bits(), 20 << 16);

    let mut rtl_fonts = FontCollection::new(FontCollectionLimits::default());
    rtl_fonts
        .add_face(
            FontFace::from_bytes(
                FontId::new(66),
                toy_font_for(&['\u{05d0}', '\u{05d1}', '\u{05d2}', '\u{2026}']),
            )
            .expect("RTL ellipsis font"),
        )
        .expect("add RTL font");
    let rtl = rtl_fonts
        .layout_text(
            "\u{05d0}\u{05d1}\u{05d2}\u{05d0}\u{05d1}\u{05d2}\u{05d0}",
            10 << 16,
            ellipsis_options,
        )
        .expect("RTL ellipsis");
    let rtl_last = rtl.lines()[1].paragraph().expect("RTL last line");
    assert!(rtl.lines()[1].ellipsized());
    assert_eq!(rtl_last.runs()[0].source_start(), 10);
    assert_eq!(rtl_last.runs()[0].source_end(), 10);
    assert_eq!(rtl_last.runs()[0].origin_x_bits(), 0);
    assert_eq!(rtl_last.runs()[0].direction(), TextDirection::RightToLeft);
    assert_eq!(
        rtl.caret_for_position(TextPosition::new(10, TextAffinity::Upstream))
            .expect("RTL ellipsis caret query")
            .expect("RTL ellipsis caret")
            .x_bits(),
        0
    );

    let empty_last = fonts
        .layout_text(
            "\nA",
            10 << 16,
            TextLayoutOptions::with_limits(18 << 16, 1, 128)
                .expect("empty-line options")
                .with_overflow(TextOverflow::Ellipsis),
        )
        .expect("ellipsis on empty final line");
    assert!(empty_last.truncated());
    assert!(empty_last.lines()[0].ellipsized());
    assert_eq!(
        (
            empty_last.lines()[0].source_start(),
            empty_last.lines()[0].source_end()
        ),
        (0, 0)
    );
    assert_eq!(
        empty_last.lines()[0]
            .paragraph()
            .expect("marker-only paragraph")
            .runs()[0]
            .source_start(),
        0
    );

    let mut missing_fonts = FontCollection::new(FontCollectionLimits::default());
    missing_fonts
        .add_face(FontFace::from_bytes(FontId::new(65), toy_font('A')).expect("plain font"))
        .expect("add plain font");
    assert_eq!(
        missing_fonts
            .layout_text("AAAAAAA", 10 << 16, ellipsis_options)
            .expect_err("missing ellipsis and periods")
            .code(),
        TextErrorCode::MissingGlyph
    );
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
fn hit_testing_and_carets_resolve_wraps_alignment_bidi_and_spacing() {
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    fonts
        .add_face(
            FontFace::from_bytes(FontId::new(82), toy_font_for(&[' ', 'A'])).expect("LTR font"),
        )
        .expect("add LTR font");

    let aligned = fonts
        .layout_text(
            "AAA",
            10 << 16,
            TextLayoutOptions::new(30 << 16)
                .expect("options")
                .with_alignment(TextAlignment::Right),
        )
        .expect("right-aligned layout");
    let caret = aligned
        .caret_for_position(TextPosition::new(1, TextAffinity::Downstream))
        .expect("caret query")
        .expect("cluster boundary");
    assert_eq!(caret.position().source_offset(), 1);
    assert_eq!(caret.position().affinity(), TextAffinity::Downstream);
    assert_eq!(caret.line_index(), 0);
    assert_eq!(caret.x_bits(), 18 << 16);
    assert_eq!(caret.top_bits(), 0);
    assert_eq!(caret.bottom_bits(), 10 << 16);
    let hit = aligned
        .hit_test_point(13 << 16, 5 << 16)
        .expect("aligned hit");
    assert_eq!(hit.line_index(), 0);
    assert_eq!(
        hit.position(),
        TextPosition::new(0, TextAffinity::Downstream)
    );

    let wrapped = fonts
        .layout_text(
            "AAA",
            10 << 16,
            TextLayoutOptions::new(12 << 16).expect("options"),
        )
        .expect("wrapped layout");
    let upstream = wrapped
        .caret_for_position(TextPosition::new(2, TextAffinity::Upstream))
        .expect("upstream query")
        .expect("upstream wrap edge");
    let downstream = wrapped
        .caret_for_position(TextPosition::new(2, TextAffinity::Downstream))
        .expect("downstream query")
        .expect("downstream wrap edge");
    assert_eq!((upstream.line_index(), upstream.x_bits()), (0, 12 << 16));
    assert_eq!((downstream.line_index(), downstream.x_bits()), (1, 0));
    assert_eq!(
        wrapped
            .hit_test_point(0, 15 << 16)
            .expect("second-line hit")
            .position(),
        TextPosition::new(2, TextAffinity::Downstream)
    );
    assert!(
        wrapped
            .caret_for_position(TextPosition::new(4, TextAffinity::Downstream))
            .expect("invalid boundary query")
            .is_none()
    );

    let justified = fonts
        .layout_text(
            "A A",
            10 << 16,
            TextLayoutOptions::new(24 << 16)
                .expect("options")
                .with_alignment(TextAlignment::Justify)
                .with_justify_last_line(true),
        )
        .expect("justified layout");
    assert_eq!(
        justified
            .caret_for_position(TextPosition::new(2, TextAffinity::Upstream))
            .expect("upstream spacing query")
            .expect("upstream spacing edge")
            .x_bits(),
        12 << 16
    );
    assert_eq!(
        justified
            .caret_for_position(TextPosition::new(2, TextAffinity::Downstream))
            .expect("downstream spacing query")
            .expect("downstream spacing edge")
            .x_bits(),
        18 << 16
    );

    let mut rtl_fonts = FontCollection::new(FontCollectionLimits::default());
    rtl_fonts
        .add_face(
            FontFace::from_bytes(FontId::new(83), toy_font_for(&['\u{05d0}', '\u{05d1}']))
                .expect("RTL font"),
        )
        .expect("add RTL font");
    let rtl = rtl_fonts
        .layout_text(
            "\u{05d0}\u{05d1}",
            10 << 16,
            TextLayoutOptions::new(20 << 16).expect("options"),
        )
        .expect("RTL layout");
    assert_eq!(
        rtl.caret_for_position(TextPosition::new(0, TextAffinity::Downstream))
            .expect("RTL start query")
            .expect("RTL start")
            .x_bits(),
        20 << 16
    );
    assert_eq!(
        rtl.caret_for_position(TextPosition::new(4, TextAffinity::Upstream))
            .expect("RTL end query")
            .expect("RTL end")
            .x_bits(),
        8 << 16
    );
    assert_eq!(
        rtl.hit_test_point(8 << 16, 5 << 16)
            .expect("RTL hit")
            .position(),
        TextPosition::new(4, TextAffinity::Upstream)
    );

    let mut mixed_fonts = FontCollection::new(FontCollectionLimits::default());
    mixed_fonts
        .add_face(
            FontFace::from_bytes(FontId::new(84), toy_font_for(&['A', '\u{05d0}']))
                .expect("mixed bidi font"),
        )
        .expect("add mixed font");
    let mixed = mixed_fonts
        .layout_text(
            "A\u{05d0}",
            10 << 16,
            TextLayoutOptions::new(20 << 16).expect("options"),
        )
        .expect("mixed bidi layout");
    assert_eq!(
        mixed
            .caret_for_position(TextPosition::new(1, TextAffinity::Upstream))
            .expect("mixed upstream query")
            .expect("mixed upstream")
            .x_bits(),
        6 << 16
    );
    assert_eq!(
        mixed
            .caret_for_position(TextPosition::new(1, TextAffinity::Downstream))
            .expect("mixed downstream query")
            .expect("mixed downstream")
            .x_bits(),
        12 << 16
    );

    let trailing = fonts
        .layout_text(
            "A\n",
            10 << 16,
            TextLayoutOptions::new(20 << 16).expect("options"),
        )
        .expect("trailing empty line");
    let empty_caret = trailing
        .caret_for_position(TextPosition::new(2, TextAffinity::Downstream))
        .expect("empty-line query")
        .expect("empty-line caret");
    assert_eq!(empty_caret.line_index(), 1);
    assert_eq!(empty_caret.x_bits(), 0);
    assert_eq!(
        (empty_caret.top_bits(), empty_caret.bottom_bits()),
        (10 << 16, 20 << 16)
    );
}

#[test]
fn selection_rects_follow_clusters_wraps_spacing_bidi_and_synthetic_markers() {
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    fonts
        .add_face(
            FontFace::from_bytes(
                FontId::new(87),
                toy_font_for(&['A', '\u{05d0}', '\u{05d1}', '\u{2026}', '\u{4e2d}']),
            )
            .expect("selection font"),
        )
        .expect("add selection font");

    let spaced = fonts
        .layout_text(
            "AAA",
            10 << 16,
            TextLayoutOptions::new(30 << 16)
                .expect("spacing options")
                .with_letter_spacing(2 << 16),
        )
        .expect("spaced layout");
    let selected = spaced.selection_rects(0, 2).expect("spaced selection");
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].line_index(), 0);
    assert_eq!(
        (
            selected[0].left_bits(),
            selected[0].top_bits(),
            selected[0].right_bits(),
            selected[0].bottom_bits(),
        ),
        (0, 0, 14 << 16, 10 << 16)
    );
    let second = spaced.selection_rects(1, 2).expect("second cluster");
    assert_eq!(
        (second[0].left_bits(), second[0].right_bits()),
        (8 << 16, 14 << 16)
    );
    assert!(
        spaced
            .selection_rects(1, 1)
            .expect("collapsed selection")
            .is_empty()
    );

    let wrapped = fonts
        .layout_text(
            "AAAA",
            10 << 16,
            TextLayoutOptions::new(12 << 16).expect("wrap options"),
        )
        .expect("wrapped layout");
    let across_lines = wrapped.selection_rects(1, 3).expect("wrapped selection");
    assert_eq!(across_lines.len(), 2);
    assert_eq!(
        (
            across_lines[0].line_index(),
            across_lines[0].left_bits(),
            across_lines[0].right_bits(),
        ),
        (0, 6 << 16, 12 << 16)
    );
    assert_eq!(
        (
            across_lines[1].line_index(),
            across_lines[1].left_bits(),
            across_lines[1].right_bits(),
        ),
        (1, 0, 6 << 16)
    );

    let bidi = fonts
        .layout_text(
            "A\u{05d0}\u{05d1}A",
            10 << 16,
            TextLayoutOptions::new(30 << 16).expect("bidi options"),
        )
        .expect("bidi layout");
    let discontiguous = bidi.selection_rects(0, 3).expect("bidi selection");
    assert_eq!(discontiguous.len(), 2);
    assert_eq!(
        (
            discontiguous[0].left_bits(),
            discontiguous[0].right_bits(),
            discontiguous[1].left_bits(),
            discontiguous[1].right_bits(),
        ),
        (0, 6 << 16, 12 << 16, 18 << 16)
    );

    let cjk = fonts
        .layout_text(
            "\u{4e2d}\u{4e2d}",
            10 << 16,
            TextLayoutOptions::new(15 << 16)
                .expect("CJK options")
                .with_alignment(TextAlignment::Justify)
                .with_justify_last_line(true),
        )
        .expect("justified CJK layout");
    let cjk_selection = cjk.selection_rects(0, 6).expect("CJK selection");
    assert_eq!(
        (cjk_selection[0].left_bits(), cjk_selection[0].right_bits()),
        (0, 15 << 16)
    );
    assert_eq!(
        cjk.selection_rects(1, 6)
            .expect_err("selection must start on a cluster boundary")
            .code(),
        TextErrorCode::InvalidLayout
    );

    let ellipsized = fonts
        .layout_text(
            "AAAA",
            10 << 16,
            TextLayoutOptions::with_limits(12 << 16, 1, 64)
                .expect("ellipsis options")
                .with_overflow(TextOverflow::Ellipsis),
        )
        .expect("ellipsized layout");
    let visible_source = ellipsized
        .selection_rects(0, 1)
        .expect("visible source selection");
    assert_eq!(visible_source[0].right_bits(), 6 << 16);
    assert_eq!(ellipsized.lines()[0].advance_x_bits(), 12 << 16);
}

#[test]
fn gdef_ligature_carets_drive_hit_testing_carets_and_partial_selection() {
    let face = FontFace::from_bytes(FontId::new(160), toy_ligature_font(Some(&[200, 450])))
        .expect("ligature font");
    let shaped = face.shape("ffi", 10 << 16).expect("shape ffi ligature");
    assert_eq!(shaped.glyphs().len(), 1);
    assert_eq!(shaped.glyphs()[0].glyph(), GlyphId::new(2));
    assert_eq!(shaped.glyphs()[0].cluster(), 0);

    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    fonts.add_face(face).expect("add ligature font");
    let ltr = fonts
        .layout_text(
            "ffi",
            10 << 16,
            TextLayoutOptions::new(20 << 16).expect("options"),
        )
        .expect("LTR ligature layout");
    for affinity in [TextAffinity::Upstream, TextAffinity::Downstream] {
        assert_eq!(
            ltr.caret_for_position(TextPosition::new(1, affinity))
                .expect("first ligature caret query")
                .expect("first ligature caret")
                .x_bits(),
            2 << 16
        );
        assert_eq!(
            ltr.caret_for_position(TextPosition::new(2, affinity))
                .expect("second ligature caret query")
                .expect("second ligature caret")
                .x_bits(),
            (4 << 16) + (1 << 15)
        );
    }
    assert_eq!(
        ltr.hit_test_point(2 << 16, 5 << 16)
            .expect("hit first ligature caret")
            .position()
            .source_offset(),
        1
    );
    let first = ltr.selection_rects(0, 1).expect("select first component");
    assert_eq!((first[0].left_bits(), first[0].right_bits()), (0, 2 << 16));
    let middle = ltr.selection_rects(1, 2).expect("select middle component");
    assert_eq!(
        (middle[0].left_bits(), middle[0].right_bits()),
        (2 << 16, (4 << 16) + (1 << 15))
    );
    let last = ltr.selection_rects(2, 3).expect("select last component");
    assert_eq!(
        (last[0].left_bits(), last[0].right_bits()),
        ((4 << 16) + (1 << 15), 6 << 16)
    );

    let rtl = fonts
        .layout_text(
            "ابج",
            10 << 16,
            TextLayoutOptions::new(20 << 16)
                .expect("options")
                .with_base_direction(TextDirection::RightToLeft)
                .with_alignment(TextAlignment::Left),
        )
        .expect("RTL ligature layout");
    assert_eq!(
        rtl.caret_for_position(TextPosition::new(2, TextAffinity::Downstream))
            .expect("RTL first caret query")
            .expect("RTL first caret")
            .x_bits(),
        (4 << 16) + (1 << 15)
    );
    assert_eq!(
        rtl.caret_for_position(TextPosition::new(4, TextAffinity::Downstream))
            .expect("RTL second caret query")
            .expect("RTL second caret")
            .x_bits(),
        2 << 16
    );
    let rtl_first = rtl
        .selection_rects(0, 2)
        .expect("select RTL first component");
    assert_eq!(
        (rtl_first[0].left_bits(), rtl_first[0].right_bits()),
        ((4 << 16) + (1 << 15), 6 << 16)
    );

    let mut atomic_fonts = FontCollection::new(FontCollectionLimits::default());
    atomic_fonts
        .add_face(
            FontFace::from_bytes(FontId::new(161), toy_ligature_font(None))
                .expect("ligature font without GDEF"),
        )
        .expect("add atomic ligature font");
    let atomic = atomic_fonts
        .layout_text(
            "ffi",
            10 << 16,
            TextLayoutOptions::new(20 << 16).expect("options"),
        )
        .expect("atomic ligature layout");
    assert_eq!(
        atomic
            .caret_for_position(TextPosition::new(1, TextAffinity::Downstream))
            .expect("atomic caret query"),
        None
    );
    assert_eq!(
        atomic
            .selection_rects(0, 1)
            .expect_err("partial atomic ligature selection must fail")
            .code(),
        TextErrorCode::InvalidLayout
    );

    let mut mismatched_fonts = FontCollection::new(FontCollectionLimits::default());
    mismatched_fonts
        .add_face(
            FontFace::from_bytes(FontId::new(162), toy_ligature_font(Some(&[300])))
                .expect("ligature font with mismatched GDEF"),
        )
        .expect("add mismatched ligature font");
    let mismatched = mismatched_fonts
        .layout_text(
            "ffi",
            10 << 16,
            TextLayoutOptions::new(20 << 16).expect("options"),
        )
        .expect("mismatched ligature layout");
    assert_eq!(
        mismatched
            .caret_for_position(TextPosition::new(1, TextAffinity::Downstream))
            .expect("mismatched caret query"),
        None
    );
}

#[test]
fn cluster_spacing_affects_wraps_carets_bidi_justification_and_ellipsis() {
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    fonts
        .add_face(
            FontFace::from_bytes(
                FontId::new(85),
                toy_font_for(&[' ', 'A', '\u{00a0}', '\u{0301}', '\u{2026}']),
            )
            .expect("spacing font"),
        )
        .expect("add spacing font");

    let letter_options = TextLayoutOptions::new(100 << 16)
        .expect("options")
        .with_letter_spacing(2 << 16);
    assert_eq!(letter_options.letter_spacing_bits(), 2 << 16);
    let letters = fonts
        .layout_text("AAA", 10 << 16, letter_options)
        .expect("letter spacing");
    let letter_run = &letters.lines()[0].paragraph().expect("line").runs()[0];
    assert_eq!(letters.lines()[0].advance_x_bits(), 22 << 16);
    assert_eq!(letter_run.glyph_offsets_x_bits(), &[0, 2 << 16, 4 << 16]);
    assert_eq!(
        letters
            .caret_for_position(TextPosition::new(1, TextAffinity::Upstream))
            .expect("upstream spacing caret")
            .expect("upstream boundary")
            .x_bits(),
        6 << 16
    );
    assert_eq!(
        letters
            .caret_for_position(TextPosition::new(1, TextAffinity::Downstream))
            .expect("downstream spacing caret")
            .expect("downstream boundary")
            .x_bits(),
        8 << 16
    );

    let wrapped = fonts
        .layout_text(
            "AAAA",
            10 << 16,
            TextLayoutOptions::new(20 << 16)
                .expect("options")
                .with_letter_spacing(2 << 16),
        )
        .expect("spacing-aware wrap");
    assert_eq!(wrapped.lines().len(), 2);
    assert_eq!(wrapped.lines()[0].source_end(), 2);

    let combined_options = TextLayoutOptions::new(100 << 16)
        .expect("options")
        .with_letter_spacing(1 << 16)
        .with_word_spacing(4 << 16);
    assert_eq!(combined_options.word_spacing_bits(), 4 << 16);
    let combined = fonts
        .layout_text("A A", 10 << 16, combined_options)
        .expect("combined spacing");
    assert_eq!(combined.lines()[0].advance_x_bits(), 24 << 16);
    assert_eq!(
        combined.lines()[0].paragraph().expect("line").runs()[0].glyph_offsets_x_bits(),
        &[0, 1 << 16, 6 << 16]
    );
    let non_breaking = fonts
        .layout_text("A\u{00a0}A", 10 << 16, combined_options)
        .expect("non-breaking spacing");
    assert_eq!(non_breaking.lines()[0].advance_x_bits(), 20 << 16);
    assert_eq!(
        non_breaking.lines()[0].paragraph().expect("line").runs()[0].glyph_offsets_x_bits(),
        &[0, 1 << 16, 2 << 16]
    );

    let grapheme = fonts
        .layout_text(
            "A\u{0301}A",
            10 << 16,
            TextLayoutOptions::new(100 << 16)
                .expect("options")
                .with_letter_spacing(2 << 16),
        )
        .expect("grapheme-safe spacing");
    assert_eq!(
        grapheme.lines()[0].paragraph().expect("line").runs()[0].glyph_offsets_x_bits(),
        &[0, 0, 2 << 16]
    );

    let negative = fonts
        .layout_text(
            "AAA",
            10 << 16,
            TextLayoutOptions::new(100 << 16)
                .expect("options")
                .with_letter_spacing(-(2 << 16)),
        )
        .expect("negative spacing");
    assert_eq!(negative.lines()[0].advance_x_bits(), 14 << 16);
    assert_eq!(
        negative.lines()[0].paragraph().expect("line").runs()[0].glyph_offsets_x_bits(),
        &[0, -(2 << 16), -(4 << 16)]
    );
    assert_eq!(
        fonts
            .layout_text(
                "AAA",
                10 << 16,
                TextLayoutOptions::new(100 << 16)
                    .expect("options")
                    .with_letter_spacing(-(10 << 16)),
            )
            .expect_err("spacing must not make advance negative")
            .code(),
        TextErrorCode::InvalidLayout
    );

    let justified = fonts
        .layout_text(
            "A A",
            10 << 16,
            TextLayoutOptions::new(26 << 16)
                .expect("options")
                .with_letter_spacing(1 << 16)
                .with_word_spacing(2 << 16)
                .with_alignment(TextAlignment::Justify)
                .with_justify_last_line(true),
        )
        .expect("spacing plus justification");
    assert_eq!(justified.lines()[0].advance_x_bits(), 26 << 16);
    assert_eq!(
        justified.lines()[0].paragraph().expect("line").runs()[0].glyph_offsets_x_bits(),
        &[0, 1 << 16, 8 << 16]
    );

    let ellipsized = fonts
        .layout_text(
            "AAAAAAA",
            10 << 16,
            TextLayoutOptions::with_limits(18 << 16, 2, 128)
                .expect("options")
                .with_letter_spacing(2 << 16)
                .with_overflow(TextOverflow::Ellipsis),
        )
        .expect("spacing-aware ellipsis");
    assert_eq!(ellipsized.lines()[0].source_end(), 2);
    assert_eq!(ellipsized.lines()[1].source_end(), 3);
    assert_eq!(ellipsized.lines()[1].advance_x_bits(), 14 << 16);

    let mut rtl_fonts = FontCollection::new(FontCollectionLimits::default());
    rtl_fonts
        .add_face(
            FontFace::from_bytes(FontId::new(86), toy_font_for(&['\u{05d0}', '\u{05d1}']))
                .expect("RTL spacing font"),
        )
        .expect("add RTL font");
    let rtl = rtl_fonts
        .layout_text(
            "\u{05d0}\u{05d1}",
            10 << 16,
            TextLayoutOptions::new(20 << 16)
                .expect("options")
                .with_letter_spacing(2 << 16),
        )
        .expect("RTL spacing");
    assert_eq!(rtl.lines()[0].advance_x_bits(), 14 << 16);
    assert_eq!(
        rtl.caret_for_position(TextPosition::new(2, TextAffinity::Downstream))
            .expect("RTL downstream")
            .expect("RTL downstream caret")
            .x_bits(),
        12 << 16
    );
    assert_eq!(
        rtl.caret_for_position(TextPosition::new(2, TextAffinity::Upstream))
            .expect("RTL upstream")
            .expect("RTL upstream caret")
            .x_bits(),
        14 << 16
    );
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
fn justification_expands_cjk_cluster_boundaries_without_splitting_marks_or_punctuation() {
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    fonts
        .add_face(
            FontFace::from_bytes(
                FontId::new(101),
                toy_font_for(&[' ', '\u{0301}', '\u{3002}', '\u{4e2d}']),
            )
            .expect("CJK font"),
        )
        .expect("add CJK font");
    let justify_final = |width| {
        TextLayoutOptions::new(width)
            .expect("options")
            .with_alignment(TextAlignment::Justify)
            .with_justify_last_line(true)
    };

    let cjk = fonts
        .layout_text(
            "\u{4e2d}\u{4e2d}\u{4e2d}",
            10 << 16,
            justify_final(24 << 16),
        )
        .expect("CJK justification");
    assert!(cjk.lines()[0].justified());
    assert_eq!(cjk.lines()[0].advance_x_bits(), 24 << 16);
    assert_eq!(
        cjk.lines()[0].paragraph().expect("CJK line").runs()[0].glyph_offsets_x_bits(),
        &[0, 3 << 16, 6 << 16]
    );

    let marked = fonts
        .layout_text(
            "\u{4e2d}\u{0301}\u{4e2d}",
            10 << 16,
            justify_final(18 << 16),
        )
        .expect("marked CJK justification");
    assert!(marked.lines()[0].justified());
    assert_eq!(
        marked.lines()[0].paragraph().expect("marked line").runs()[0].glyph_offsets_x_bits(),
        &[0, 0, 6 << 16]
    );

    let punctuated = fonts
        .layout_text(
            "\u{4e2d}\u{3002}\u{4e2d}",
            10 << 16,
            justify_final(24 << 16),
        )
        .expect("punctuated CJK layout");
    assert!(!punctuated.lines()[0].justified());
    assert_eq!(punctuated.lines()[0].advance_x_bits(), 18 << 16);

    let inter_word = fonts
        .layout_text("\u{4e2d} \u{4e2d}", 10 << 16, justify_final(30 << 16))
        .expect("inter-word priority");
    assert!(inter_word.lines()[0].justified());
    assert_eq!(
        inter_word.lines()[0]
            .paragraph()
            .expect("mixed line")
            .runs()[0]
            .glyph_offsets_x_bits(),
        &[0, 0, 12 << 16]
    );

    let wrapped = fonts
        .layout_text(
            "\u{4e2d}\u{4e2d}\u{4e2d}\u{4e2d}",
            10 << 16,
            TextLayoutOptions::new(15 << 16)
                .expect("wrap options")
                .with_alignment(TextAlignment::Justify),
        )
        .expect("wrapped CJK justification");
    assert_eq!(wrapped.lines().len(), 2);
    assert!(wrapped.lines()[0].justified());
    assert_eq!(wrapped.lines()[0].advance_x_bits(), 15 << 16);
    assert!(!wrapped.lines()[1].justified());
    assert_eq!(
        wrapped
            .caret_for_position(TextPosition::new(3, TextAffinity::Upstream))
            .expect("CJK upstream query")
            .expect("CJK upstream")
            .x_bits(),
        6 << 16
    );
    assert_eq!(
        wrapped
            .caret_for_position(TextPosition::new(3, TextAffinity::Downstream))
            .expect("CJK downstream query")
            .expect("CJK downstream")
            .x_bits(),
        9 << 16
    );

    let mut fallback_fonts = FontCollection::new(FontCollectionLimits::default());
    fallback_fonts
        .add_face(
            FontFace::from_bytes(FontId::new(102), toy_font('\u{4e2d}')).expect("Han primary"),
        )
        .expect("add Han primary");
    fallback_fonts
        .add_face(
            FontFace::from_bytes(FontId::new(103), toy_font('\u{6587}')).expect("Han fallback"),
        )
        .expect("add Han fallback");
    let fallback = fallback_fonts
        .layout_text("\u{4e2d}\u{6587}", 10 << 16, justify_final(18 << 16))
        .expect("cross-run CJK justification");
    let runs = fallback.lines()[0]
        .paragraph()
        .expect("fallback CJK line")
        .runs();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].glyph_offsets_x_bits(), &[0]);
    assert_eq!(runs[1].glyph_offsets_x_bits(), &[6 << 16]);
}

#[test]
fn justification_supports_mixed_and_explicit_cross_script_boundaries() {
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    fonts
        .add_face(
            FontFace::from_bytes(FontId::new(109), toy_font_for(&['.', 'A', 'B', '\u{4e2d}']))
                .expect("mixed-script font"),
        )
        .expect("add mixed-script font");
    let options = |justification| {
        TextLayoutOptions::new(18 << 16)
            .expect("options")
            .with_alignment(TextAlignment::Justify)
            .with_justify_last_line(true)
            .with_justification(justification)
    };

    let mixed = fonts
        .layout_text("\u{4e2d}A", 10 << 16, options(TextJustification::Auto))
        .expect("mixed auto justification");
    assert!(mixed.lines()[0].justified());
    assert_eq!(
        mixed.lines()[0].paragraph().expect("line").runs()[0].glyph_offsets_x_bits(),
        &[0, 6 << 16]
    );

    let latin_auto = fonts
        .layout_text("AB", 10 << 16, options(TextJustification::Auto))
        .expect("Latin auto layout");
    assert!(!latin_auto.lines()[0].justified());

    let latin_inter_character = fonts
        .layout_text("AB", 10 << 16, options(TextJustification::InterCharacter))
        .expect("Latin inter-character justification");
    assert!(latin_inter_character.lines()[0].justified());
    assert_eq!(
        latin_inter_character.lines()[0]
            .paragraph()
            .expect("line")
            .runs()[0]
            .glyph_offsets_x_bits(),
        &[0, 6 << 16]
    );

    let cjk_inter_word = fonts
        .layout_text(
            "\u{4e2d}\u{4e2d}",
            10 << 16,
            options(TextJustification::InterWord),
        )
        .expect("CJK inter-word layout");
    assert!(!cjk_inter_word.lines()[0].justified());

    let punctuated = fonts
        .layout_text(
            "A.B",
            10 << 16,
            TextLayoutOptions::new(24 << 16)
                .expect("options")
                .with_alignment(TextAlignment::Justify)
                .with_justify_last_line(true)
                .with_justification(TextJustification::InterCharacter),
        )
        .expect("punctuation-safe layout");
    assert!(!punctuated.lines()[0].justified());
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

    let disabled = fonts
        .layout_styled_text(
            "A",
            &[TextStyleSpan::new(0, 1, FontId::new(120), 10 << 16)
                .expect("span")
                .with_decoration(TextDecoration::None)],
            TextLayoutOptions::new(20 << 16)
                .expect("options")
                .with_decoration(TextDecoration::Underline),
        )
        .expect("span can disable inherited decoration");
    assert!(disabled.lines()[0].decoration_segments().is_empty());
}
