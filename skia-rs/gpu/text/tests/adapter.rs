use std::sync::Arc;

use skia_core::{
    Color, FontCollection, FontCollectionLimits, FontFace, FontId, Paint, Point, Scalar,
    TextLayoutOptions, TextStyleId, TextStyleSpan,
};
#[cfg(target_os = "macos")]
use skia_core::{TextDecoration, layout_decoration_batches};
use skia_gpu::{GpuBackend, GpuCommand, GpuCommandEncoder, GpuSurfaceDescriptor};
use skia_gpu_text::{TextAtlasBuilder, TextAtlasCache, TextAtlasCacheLimits, TextGlyphKey};

const BASIC_A: &[u8] = include_bytes!("../../../text/tests/fonts/synthetic/basic-a.ttf");
const BASIC_B: &[u8] = include_bytes!("../../../text/tests/fonts/synthetic/basic-b.ttf");
const BASIC_C: &[u8] = include_bytes!("../../../text/tests/fonts/synthetic/basic-c.ttf");
#[cfg(target_os = "macos")]
const DECORATED_A: &[u8] = include_bytes!("../../../text/tests/fonts/synthetic/decorated-a.ttf");

fn font_bytes(fixture: &[u8]) -> Vec<u8> {
    fixture.to_vec()
}

#[test]
fn text_adapter_shapes_packs_and_replays_layout_glyphs() {
    let face = FontFace::from_bytes(FontId::new(91), font_bytes(BASIC_A)).expect("load toy font");
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

#[cfg(target_os = "macos")]
#[test]
fn text_adapter_draws_styled_glyphs_and_decorations_on_metal() {
    use skia_metal::{MetalBackend, MetalErrorCode};

    let mut backend = match MetalBackend::new() {
        Ok(backend) => backend,
        Err(error)
            if error.code() == MetalErrorCode::DeviceUnavailable
                && std::env::var_os("SKIA_REQUIRE_METAL_DEVICE").is_none() =>
        {
            return;
        }
        Err(error) => panic!("unexpected Metal initialization failure: {error}"),
    };
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    fonts
        .add_face(
            FontFace::from_bytes(FontId::new(93), font_bytes(DECORATED_A)).expect("decorated font"),
        )
        .expect("register face");
    let red_style = TextStyleId::new(21);
    let blue_style = TextStyleId::new(22);
    let layout = fonts
        .layout_styled_text(
            "AA",
            &[
                TextStyleSpan::new(0, 1, FontId::new(93), 20 << 16)
                    .expect("red span")
                    .with_style_id(red_style)
                    .with_decoration(TextDecoration::Underline),
                TextStyleSpan::new(1, 2, FontId::new(93), 20 << 16)
                    .expect("blue span")
                    .with_style_id(blue_style)
                    .with_decoration(TextDecoration::StrikeThrough),
            ],
            TextLayoutOptions::new(30 << 16).expect("layout options"),
        )
        .expect("styled layout");
    let mut builder = TextAtlasBuilder::new(64, 64, 4).expect("atlas builder");
    builder
        .insert_layout(&layout, &fonts)
        .expect("pack layout glyphs");
    let atlas = builder.finish().expect("finish atlas");
    let origin = Point::new(Scalar::ZERO, Scalar::from_i32(1).unwrap());
    let glyph_batches = atlas
        .layout_style_batches(&layout, origin)
        .expect("glyph batches");
    let decoration_batches =
        layout_decoration_batches(&layout, origin).expect("decoration batches");

    let mut commands = GpuCommandEncoder::new(8).expect("encoder");
    let atlas_id = commands
        .add_glyph_atlas(atlas.into_gpu_atlas())
        .expect("register atlas");
    commands.clear(Color::TRANSPARENT).expect("clear");
    for batch in glyph_batches {
        let paint = paint_for_style(batch.style_id(), red_style, blue_style);
        commands
            .draw_glyph_batch(atlas_id, batch.into_glyphs(), paint)
            .expect("record glyphs");
    }
    for batch in decoration_batches {
        let paint = paint_for_style(batch.style_id(), red_style, blue_style);
        for rect in batch.into_rects() {
            commands.fill_rect(rect, paint).expect("record decoration");
        }
    }

    let mut surface = backend
        .create_surface(GpuSurfaceDescriptor::new(30, 24).expect("surface descriptor"))
        .expect("surface");
    backend
        .submit(&mut surface, &commands.finish())
        .expect("submit");
    let pixels = surface.read_rgba8().expect("read pixels");
    assert_eq!(pixel(&pixels, 30, 2, 18), Color::RED.channels());
    assert_eq!(pixel(&pixels, 30, 22, 10), Color::BLUE.channels());
    assert_eq!(pixel(&pixels, 30, 22, 18), Color::TRANSPARENT.channels());
}

#[test]
fn atlas_cache_reuses_supersets_and_evicts_least_recently_used_entries() {
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    for (value, fixture) in [(201, BASIC_A), (202, BASIC_B), (203, BASIC_C)] {
        fonts
            .add_face(
                FontFace::from_bytes(FontId::new(value), font_bytes(fixture)).expect("toy font"),
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

#[cfg(target_os = "macos")]
fn paint_for_style(style: TextStyleId, red_style: TextStyleId, blue_style: TextStyleId) -> Paint {
    match style {
        value if value == red_style => Paint::new(Color::RED),
        value if value == blue_style => Paint::new(Color::BLUE),
        _ => panic!("unexpected text style"),
    }
}

#[cfg(target_os = "macos")]
fn pixel(pixels: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
    let offset = (y * width + x) * 4;
    pixels[offset..offset + 4].try_into().unwrap()
}
