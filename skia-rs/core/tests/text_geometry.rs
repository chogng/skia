#[path = "support/mod.rs"]
mod support;

use skia_core::{
    FontCollection, FontCollectionLimits, FontFace, FontId, Point, Scalar, TextDecoration,
    TextDecorationStyle, TextLayoutEvent, TextLayoutOptions, TextStyleId, TextStyleSpan,
    layout_decoration_batches, layout_outline_batches, text_layout_events,
};

use support::{BASIC_A, DECORATED_A, font_bytes};

#[test]
fn layout_outline_batches_emit_portable_paths() {
    let face = FontFace::from_bytes(FontId::new(95), font_bytes(BASIC_A)).expect("load toy font");
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    fonts.add_face(face).expect("register face");
    let layout = fonts
        .layout_text(
            "AA",
            12 << 16,
            TextLayoutOptions::new(32 << 16).expect("layout options"),
        )
        .expect("layout text");

    let batches = layout_outline_batches(&layout, &fonts, Point::new(Scalar::ZERO, Scalar::ZERO))
        .expect("outline batches");

    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].style_id(), TextStyleId::DEFAULT);
    assert_eq!(batches[0].paths().len(), 2);
}

#[test]
fn layout_events_keep_glyphs_before_per_style_decorations() {
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    fonts
        .add_face(
            FontFace::from_bytes(FontId::new(92), font_bytes(DECORATED_A)).expect("decorated font"),
        )
        .expect("register face");
    let underline_style = TextStyleId::new(11);
    let strike_style = TextStyleId::new(12);
    let layout = fonts
        .layout_styled_text(
            "AA",
            &[
                TextStyleSpan::new(0, 1, FontId::new(92), 20 << 16)
                    .expect("underline span")
                    .with_style_id(underline_style)
                    .with_decoration(TextDecoration::Underline),
                TextStyleSpan::new(1, 2, FontId::new(92), 20 << 16)
                    .expect("strike span")
                    .with_style_id(strike_style)
                    .with_decoration(TextDecoration::StrikeThrough),
            ],
            TextLayoutOptions::new(30 << 16).expect("layout options"),
        )
        .expect("styled layout");
    let origin = Point::new(Scalar::from_i32(3).unwrap(), Scalar::from_i32(5).unwrap());

    let events = text_layout_events(&layout, origin).expect("layout events");
    assert!(matches!(events[0], TextLayoutEvent::GlyphRun { .. }));
    assert!(matches!(events[1], TextLayoutEvent::GlyphRun { .. }));
    assert!(matches!(
        events[2],
        TextLayoutEvent::Decoration {
            style_id,
            ..
        } if style_id == underline_style
    ));
    assert!(matches!(
        events[3],
        TextLayoutEvent::Decoration {
            style_id,
            ..
        } if style_id == strike_style
    ));

    let batches = layout_decoration_batches(&layout, origin).expect("decoration batches");
    assert_eq!(batches.len(), 2);
    assert_eq!(batches[0].style_id(), underline_style);
    assert_eq!(batches[1].style_id(), strike_style);
    assert_eq!(batches[0].rects().len(), 1);
    assert_eq!(batches[1].rects().len(), 1);

    let line = &layout.lines()[0];
    let line_x = origin.x().bits() + line.offset_x_bits();
    let baseline = origin.y().bits() + line.baseline_y_bits();
    assert_rect_bits(
        batches[0].rects()[0],
        [
            line_x,
            baseline + (1 << 16),
            line_x + (12 << 16),
            baseline + (3 << 16),
        ],
    );
    assert_rect_bits(
        batches[1].rects()[0],
        [
            line_x + (12 << 16),
            baseline - (7 << 16),
            line_x + (24 << 16),
            baseline - (5 << 16),
        ],
    );
}

#[test]
fn layout_decoration_batches_expand_all_patterns() {
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    fonts
        .add_face(
            FontFace::from_bytes(FontId::new(94), font_bytes(DECORATED_A)).expect("decorated font"),
        )
        .expect("register face");

    for (style, expected_rects) in [
        (TextDecorationStyle::Solid, 1),
        (TextDecorationStyle::Dashed, 2),
        (TextDecorationStyle::Dotted, 3),
        (TextDecorationStyle::Wavy, 6),
    ] {
        let layout = fonts
            .layout_text(
                "A",
                20 << 16,
                TextLayoutOptions::new(20 << 16)
                    .expect("options")
                    .with_decoration(TextDecoration::Underline)
                    .with_decoration_style(style),
            )
            .expect("decorated layout");
        let batches = layout_decoration_batches(&layout, Point::new(Scalar::ZERO, Scalar::ZERO))
            .expect("decoration batches");
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].rects().len(), expected_rects, "{style:?}");
    }
}

fn assert_rect_bits(rect: skia_core::Rect, expected: [i32; 4]) {
    assert_eq!(
        [
            rect.left().bits(),
            rect.top().bits(),
            rect.right().bits(),
            rect.bottom().bits(),
        ],
        expected
    );
}
