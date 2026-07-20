mod support;

use skia_core::{
    Color, FontCollection, FontCollectionLimits, FontFace, FontId, Paint, Point, Scalar,
    TextLayoutOptions,
};
use skia_gpu::{GpuBackend, GpuCommand, GpuCommandEncoder, GpuSurfaceDescriptor};
use skia_gpu_text::{TextAtlasBuilder, TextGlyphKey};

use support::toy_font;

#[test]
fn text_adapter_shapes_packs_and_replays_layout_glyphs() {
    let face = FontFace::from_bytes(FontId::new(91), toy_font('A')).expect("load toy font");
    let glyph = face
        .glyph_for_character('A')
        .expect("lookup glyph")
        .expect("covered glyph");
    let bitmap = face
        .rasterize_glyph(glyph, 12 << 16)
        .expect("rasterize")
        .expect("outline bitmap");
    let key = TextGlyphKey::from_bitmap(&bitmap);
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    fonts.add_face(face).expect("register face");
    let layout = fonts
        .layout_text(
            "AA",
            12 << 16,
            TextLayoutOptions::new(32 << 16).expect("layout options"),
        )
        .expect("layout text");

    let mut builder = TextAtlasBuilder::new(32, 32, 4).expect("atlas builder");
    builder
        .insert_layout(&layout, &fonts)
        .expect("pack layout glyphs");
    let atlas = builder.finish().expect("finish atlas");
    assert!(atlas.entry(key).is_some());
    let glyphs = atlas
        .layout_quads(&layout, Point::new(Scalar::ZERO, Scalar::ZERO))
        .expect("position layout glyphs");

    let mut encoder = GpuCommandEncoder::new(2).expect("encoder");
    let atlas = encoder
        .add_glyph_atlas(atlas.into_gpu_atlas())
        .expect("register atlas");
    encoder.clear(Color::TRANSPARENT).expect("clear");
    encoder
        .draw_glyph_batch(atlas, glyphs, Paint::new(Color::rgba(20, 40, 200, 255)))
        .expect("record text layout");
    let commands = encoder.finish();
    assert!(matches!(
        commands.commands()[1],
        GpuCommand::DrawGlyphs { ref glyphs, .. } if glyphs.len() == 2
    ));

    let mut backend = skia_gpu::software::SoftwareGpuBackend::default();
    let mut surface = backend
        .create_surface(GpuSurfaceDescriptor::new(32, 16).expect("surface descriptor"))
        .expect("surface");
    backend.submit(&mut surface, &commands).expect("replay");
    assert!(
        surface
            .pixels()
            .chunks_exact(4)
            .any(|pixel| pixel[2] > 0 && pixel[3] > 0)
    );
}
