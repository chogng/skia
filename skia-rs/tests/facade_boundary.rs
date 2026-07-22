#[test]
fn public_facade_does_not_transparently_reexport_implementation_crates() {
    let facade = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"))
        .expect("read public facade source");

    assert!(!facade.contains("pub use skia_core::*"));
    assert!(!facade.contains("pub use skia_cpu::*"));
    assert!(!facade.contains("DisplayList"));
    assert!(!facade.contains("DrawCommand"));
    assert!(!facade.contains("ImageId"));
    assert!(!facade.contains("PathId"));
    assert!(!facade.contains("GlyphRunId"));
    assert!(facade.contains("FontFace"));
    assert!(facade.contains("FontLimits"));
    assert!(facade.contains("FontStyle"));
    assert!(facade.contains("FontCollection"));
    assert!(facade.contains("ShapedParagraph"));
    assert!(facade.contains("TextLayout"));
    assert!(facade.contains("TextAlignment"));
    assert!(facade.contains("TextBreakProvider"));
    assert!(facade.contains("FontMetrics"));
    assert!(facade.contains("TextDecoration"));
    assert!(facade.contains("TextDecorationMetrics"));
    assert!(facade.contains("FontTag"));
    assert!(facade.contains("FontVariation"));
    assert!(facade.contains("FontVariationAxis"));
    assert!(facade.contains("FontFeature"));
    assert!(facade.contains("TextStyleSpan"));
}
