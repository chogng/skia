use skia_text::{
    FontId, GlyphId, GlyphRun, GlyphRunSource, OutlinePoint, OutlineSegment, PositionedGlyph,
    TextErrorCode, TextUnit,
};

fn glyph() -> PositionedGlyph {
    PositionedGlyph::new(
        GlyphId::new(17),
        TextUnit::from_bits(64),
        TextUnit::ZERO,
        TextUnit::from_bits(96),
        TextUnit::ZERO,
    )
}

#[test]
fn glyph_run_preserves_shaper_output() {
    let run = GlyphRun::new(FontId::new(44), 12 << 16, 1_000, vec![glyph()]).expect("valid run");

    assert_eq!(run.font(), FontId::new(44));
    assert_eq!(run.font_size_bits(), 12 << 16);
    assert_eq!(run.units_per_em(), 1_000);
    assert_eq!(run.glyphs(), &[glyph()]);
}

#[test]
fn glyph_run_rejects_ambiguous_input() {
    assert_eq!(
        GlyphRun::new(FontId::new(1), 0, 1_000, vec![glyph()])
            .expect_err("zero size must fail")
            .code(),
        TextErrorCode::InvalidFontSize
    );
    assert_eq!(
        GlyphRun::new(FontId::new(1), 12 << 16, 1_000, Vec::new())
            .expect_err("empty run must fail")
            .code(),
        TextErrorCode::EmptyGlyphRun
    );
}

#[test]
fn positioned_glyph_preserves_source_cluster() {
    let glyph = PositionedGlyph::with_cluster(
        GlyphId::new(17),
        9,
        TextUnit::ZERO,
        TextUnit::ZERO,
        TextUnit::ZERO,
        TextUnit::ZERO,
    );

    assert_eq!(glyph.cluster(), 9);
}

#[test]
fn glyph_run_source_preserves_exact_text_and_validates_clusters() {
    let source = GlyphRunSource::new("中A".to_owned(), 12).expect("source range");
    let glyph = PositionedGlyph::with_cluster(
        GlyphId::new(17),
        12,
        TextUnit::ZERO,
        TextUnit::ZERO,
        TextUnit::ZERO,
        TextUnit::ZERO,
    );
    let run = GlyphRun::new_with_source(FontId::new(44), 12 << 16, 1_000, vec![glyph], source)
        .expect("sourced run");
    let source = run.source().expect("source retained by run");
    assert_eq!(source.text(), "中A");
    assert_eq!(source.offset(), 12);

    let invalid = PositionedGlyph::with_cluster(
        GlyphId::new(17),
        13,
        TextUnit::ZERO,
        TextUnit::ZERO,
        TextUnit::ZERO,
        TextUnit::ZERO,
    );
    assert_eq!(
        GlyphRun::new_with_source(
            FontId::new(44),
            12 << 16,
            1_000,
            vec![invalid],
            GlyphRunSource::new("中A".to_owned(), 12).expect("source range"),
        )
        .expect_err("mid-codepoint cluster must fail")
        .code(),
        TextErrorCode::InvalidLayout
    );
}

#[test]
fn sourced_glyphs_resolve_logical_clusters_even_in_visual_order() {
    let source = GlyphRunSource::new("A中B".to_owned(), 20).expect("source range");
    let glyph = |id, cluster| {
        PositionedGlyph::with_cluster(
            GlyphId::new(id),
            cluster,
            TextUnit::ZERO,
            TextUnit::ZERO,
            TextUnit::ZERO,
            TextUnit::ZERO,
        )
    };
    let run = GlyphRun::new_with_source(
        FontId::new(45),
        12 << 16,
        1_000,
        vec![glyph(1, 24), glyph(2, 20), glyph(3, 21)],
        source,
    )
    .expect("sourced visual run");

    assert_eq!(run.source_text_for_glyph(0), Some("B"));
    assert_eq!(run.source_text_for_glyph(1), Some("A"));
    assert_eq!(run.source_text_for_glyph(2), Some("中"));
    assert_eq!(run.source_text_for_glyph(3), None);
}

#[test]
fn whole_text_units_report_numeric_overflow() {
    assert_eq!(
        TextUnit::from_i32(i32::MAX)
            .expect_err("Q26.6 conversion must be checked")
            .code(),
        TextErrorCode::NumericOverflow
    );
}

#[test]
fn outline_rejects_open_or_out_of_order_contours() {
    let point = OutlinePoint::new(TextUnit::ZERO, TextUnit::ZERO);
    assert_eq!(
        skia_text::GlyphOutline::new(
            FontId::new(1),
            GlyphId::new(1),
            vec![OutlineSegment::LineTo(point)],
        )
        .expect_err("line requires a move")
        .code(),
        TextErrorCode::InvalidOutline
    );
    assert_eq!(
        skia_text::GlyphOutline::new(
            FontId::new(1),
            GlyphId::new(1),
            vec![OutlineSegment::MoveTo(point)],
        )
        .expect_err("open contour is ambiguous")
        .code(),
        TextErrorCode::InvalidOutline
    );
}
