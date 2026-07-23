use super::*;

fn metrics() -> TextDecorationMetrics {
    TextDecorationMetrics::from_bits_for_test(1 << 16, 2 << 16)
}

#[test]
fn patterns_expand_with_deterministic_phase() {
    let dashed = text_decoration_rects(
        0,
        20 << 16,
        10 << 16,
        metrics(),
        TextDecorationStyle::Dashed,
    )
    .expect("dashes");
    assert_eq!(dashed.len(), 2);
    assert_eq!((dashed[0].left_bits, dashed[0].right_bits), (0, 6 << 16));
    assert_eq!(
        (dashed[1].left_bits, dashed[1].right_bits),
        (10 << 16, 16 << 16)
    );

    let dotted = text_decoration_rects(
        0,
        10 << 16,
        10 << 16,
        metrics(),
        TextDecorationStyle::Dotted,
    )
    .expect("dots");
    assert_eq!(dotted.len(), 3);
    assert_eq!(
        (dotted[1].left_bits, dotted[1].right_bits),
        (4 << 16, 6 << 16)
    );

    let wavy = text_decoration_rects(0, 8 << 16, 10 << 16, metrics(), TextDecorationStyle::Wavy)
        .expect("wave");
    assert_eq!(wavy.len(), 4);
    assert!(wavy[0].top_bits < wavy[2].top_bits);
    assert!(wavy[0].bottom_bits < wavy[2].bottom_bits);
}
