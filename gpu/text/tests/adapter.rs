mod support;

use std::sync::Arc;

use skia_core::{
    Color, FillRule, FontCollection, FontCollectionLimits, FontFace, FontId, Paint, Point, Scalar,
    TextDecoration, TextDecorationStyle, TextLayoutOptions, TextStyleId, TextStyleSpan,
};
use skia_gpu::{GpuBackend, GpuCommand, GpuCommandEncoder, GpuSurfaceDescriptor};
use skia_gpu_text::{
    TextAtlasBuilder, TextAtlasCache, TextAtlasCacheLimits, TextGlyphKey,
    layout_decoration_batches, layout_outline_batches,
};

use support::{toy_font, toy_font_with_decorations};

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
fn text_adapter_emits_vector_outlines_as_generic_paths() {
    let face = FontFace::from_bytes(FontId::new(95), toy_font('A')).expect("load toy font");
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

    let mut encoder = GpuCommandEncoder::new(4).expect("encoder");
    encoder.clear(Color::TRANSPARENT).expect("clear");
    for batch in batches {
        assert_eq!(batch.style_id(), TextStyleId::DEFAULT);
        for path in batch.into_paths() {
            let path = encoder.add_path(path).expect("register glyph path");
            encoder
                .fill_path(path, FillRule::NonZero, Paint::new(Color::BLUE))
                .expect("record glyph path");
        }
    }
    let commands = encoder.finish();
    assert_eq!(
        commands
            .commands()
            .iter()
            .filter(|command| matches!(command, GpuCommand::FillPath { .. }))
            .count(),
        2
    );

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
fn text_adapter_emits_per_style_decoration_rectangles() {
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    fonts
        .add_face(
            FontFace::from_bytes(FontId::new(92), toy_font_with_decorations('A'))
                .expect("decorated font"),
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
fn text_adapter_expands_all_decoration_patterns() {
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    fonts
        .add_face(
            FontFace::from_bytes(FontId::new(94), toy_font_with_decorations('A'))
                .expect("decorated font"),
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
            FontFace::from_bytes(FontId::new(93), toy_font_with_decorations('A'))
                .expect("decorated font"),
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
