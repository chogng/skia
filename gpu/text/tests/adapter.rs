mod support;

use std::sync::Arc;

use skia_core::{
    Color, FontCollection, FontCollectionLimits, FontFace, FontId, Paint, Point, Scalar,
    TextLayoutOptions, TextStyleId, TextStyleSpan,
};
use skia_gpu::{GpuBackend, GpuCommand, GpuCommandEncoder, GpuSurfaceDescriptor};
use skia_gpu_text::{TextAtlasBuilder, TextAtlasCache, TextAtlasCacheLimits, TextGlyphKey};

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

    let first_style = TextStyleId::new(1);
    let second_style = TextStyleId::new(2);
    let styled = fonts
        .layout_styled_text(
            "AA",
            &[
                TextStyleSpan::new(0, 1, FontId::new(91), 12 << 16)
                    .expect("first span")
                    .with_style_id(first_style),
                TextStyleSpan::new(1, 2, FontId::new(91), 12 << 16)
                    .expect("second span")
                    .with_style_id(second_style),
            ],
            TextLayoutOptions::new(32 << 16).expect("styled options"),
        )
        .expect("styled layout");
    let batches = atlas
        .layout_style_batches(&styled, Point::new(Scalar::ZERO, Scalar::ZERO))
        .expect("position styled batches");
    assert_eq!(batches.len(), 2);
    assert_eq!(batches[0].style_id(), first_style);
    assert_eq!(batches[0].glyphs().len(), 1);
    assert_eq!(batches[1].style_id(), second_style);
    assert_eq!(batches[1].glyphs().len(), 1);

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

#[test]
fn atlas_cache_reuses_supersets_and_evicts_least_recently_used_entries() {
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    for (value, character) in [(201, 'A'), (202, 'B'), (203, 'C')] {
        fonts
            .add_face(
                FontFace::from_bytes(FontId::new(value), toy_font(character)).expect("toy font"),
            )
            .expect("register face");
    }
    let options = || TextLayoutOptions::new(32 << 16).expect("options");
    let layout_a = fonts
        .layout_text("A", 12 << 16, options())
        .expect("layout A");
    let layout_ab = fonts
        .layout_text("AB", 12 << 16, options())
        .expect("layout AB");
    let layout_c = fonts
        .layout_text("C", 12 << 16, options())
        .expect("layout C");

    let limits = TextAtlasCacheLimits::new(32, 32, 4, 2).expect("cache limits");
    let mut cache = TextAtlasCache::new(limits);
    let first_a = cache
        .get_or_insert_layout(&layout_a, &fonts)
        .expect("cache A");
    let second_a = cache
        .get_or_insert_layout(&layout_a, &fonts)
        .expect("reuse A");
    assert!(Arc::ptr_eq(&first_a, &second_a));
    assert!(first_a.gpu_atlas().cache_key().is_some());

    let atlas_ab = cache
        .get_or_insert_layout(&layout_ab, &fonts)
        .expect("cache AB");
    cache
        .get_or_insert_layout(&layout_a, &fonts)
        .expect("touch A");
    cache
        .get_or_insert_layout(&layout_c, &fonts)
        .expect("evict AB");
    let rebuilt_ab = cache
        .get_or_insert_layout(&layout_ab, &fonts)
        .expect("rebuild AB");
    assert!(!Arc::ptr_eq(&atlas_ab, &rebuilt_ab));
    assert_ne!(
        atlas_ab.gpu_atlas().cache_key(),
        rebuilt_ab.gpu_atlas().cache_key()
    );
    assert_eq!(cache.stats().hits(), 2);
    assert_eq!(cache.stats().misses(), 4);
    assert_eq!(cache.stats().evictions(), 2);
    assert_eq!(cache.stats().entries(), 2);

    let mut superset_cache =
        TextAtlasCache::new(TextAtlasCacheLimits::new(32, 32, 4, 1).expect("superset limits"));
    let superset = superset_cache
        .get_or_insert_layout(&layout_ab, &fonts)
        .expect("cache superset");
    let subset = superset_cache
        .get_or_insert_layout(&layout_a, &fonts)
        .expect("reuse superset");
    assert!(Arc::ptr_eq(&superset, &subset));
    superset_cache.clear();
    assert_eq!(superset_cache.stats().entries(), 0);
}
