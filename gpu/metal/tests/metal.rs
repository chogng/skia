use skia_core::{
    BlendMode, ClipOp, Color, FillRule, Paint, PathBuilder, Point, Rect, Scalar, StrokeOptions,
    Transform,
};
use skia_gpu::{
    GpuAtlasRect, GpuBackend, GpuCommandBuffer, GpuCommandEncoder, GpuGlyphAtlas, GpuGlyphAtlasKey,
    GpuGlyphQuad, GpuSurfaceDescriptor,
};
use skia_image::Image;
use skia_metal::{MetalBackend, MetalErrorCode};

#[test]
fn metal_backend_allocates_a_native_surface_and_submits_a_clear() {
    let Some(mut backend) = backend_or_skip() else {
        return;
    };
    let mut surface = backend
        .create_surface(GpuSurfaceDescriptor::new(4, 4).unwrap())
        .unwrap();
    let mut commands = GpuCommandEncoder::new(1).unwrap();
    commands.clear(Color::rgba(10, 20, 30, 255)).unwrap();
    backend.submit(&mut surface, &commands.finish()).unwrap();
    assert_eq!(surface.descriptor().width(), 4);
    let pixels = surface.read_rgba8().unwrap();
    assert_eq!(&pixels[0..4], &[10, 20, 30, 255]);
    assert_eq!(&pixels[60..64], &[10, 20, 30, 255]);
}

#[test]
fn metal_backend_fails_closed_for_unimplemented_draw_commands() {
    let Some(mut backend) = backend_or_skip() else {
        return;
    };
    let mut surface = backend
        .create_surface(GpuSurfaceDescriptor::new(4, 4).unwrap())
        .unwrap();
    let mut commands = GpuCommandEncoder::new(1).unwrap();
    commands
        .fill_rect(
            Rect::new(
                Scalar::from_i32(0).unwrap(),
                Scalar::from_i32(0).unwrap(),
                Scalar::from_i32(1).unwrap(),
                Scalar::from_i32(1).unwrap(),
            )
            .unwrap(),
            Paint::new(Color::BLACK).with_blend_mode(BlendMode::Multiply),
        )
        .unwrap();
    assert_eq!(
        backend
            .submit(&mut surface, &commands.finish())
            .unwrap_err()
            .code(),
        MetalErrorCode::UnsupportedCommand
    );

    let mut path = PathBuilder::new(2).unwrap();
    path.move_to(Point::new(Scalar::ZERO, Scalar::ZERO))
        .unwrap();
    path.line_to(Point::new(Scalar::from_i32(2).unwrap(), Scalar::ZERO))
        .unwrap();
    let mut commands = GpuCommandEncoder::new(1).unwrap();
    let path = commands.add_path(path.finish().unwrap()).unwrap();
    commands
        .stroke_path(
            path,
            StrokeOptions::new(Scalar::from_i32(1).unwrap()).unwrap(),
            Paint::new(Color::WHITE),
        )
        .unwrap();
    assert_eq!(
        backend
            .submit(&mut surface, &commands.finish())
            .unwrap_err()
            .code(),
        MetalErrorCode::UnsupportedCommand
    );
}

#[test]
fn metal_backend_fills_transformed_clipped_rectangles_on_hardware() {
    let Some(mut backend) = backend_or_skip() else {
        return;
    };
    let mut surface = backend
        .create_surface(GpuSurfaceDescriptor::new(4, 3).unwrap())
        .unwrap();
    let source = Color::rgba(255, 0, 0, 128);
    let mut commands = GpuCommandEncoder::new(2).unwrap();
    commands.clear(Color::BLACK).unwrap();
    commands.save().unwrap();
    commands.set_transform(Transform::translate(
        Scalar::from_i32(1).unwrap(),
        Scalar::ZERO,
    ));
    commands.clip_rect(rect(0, 0, 2, 2)).unwrap();
    commands
        .fill_rect(rect(0, 0, 3, 3), Paint::new(source))
        .unwrap();
    commands.restore().unwrap();
    backend.submit(&mut surface, &commands.finish()).unwrap();

    let pixels = surface.read_rgba8().unwrap();
    let filled = source
        .composite(Color::BLACK, BlendMode::SourceOver)
        .channels();
    for y in 0..3 {
        for x in 0..4 {
            let expected = if (1..3).contains(&x) && y < 2 {
                filled
            } else {
                Color::BLACK.channels()
            };
            assert_eq!(pixel(&pixels, 4, x, y), expected, "pixel ({x}, {y})");
        }
    }
}

#[test]
fn metal_backend_draws_mask_and_color_glyphs_on_hardware() {
    let Some(mut backend) = backend_or_skip() else {
        return;
    };
    let mut surface = backend
        .create_surface(GpuSurfaceDescriptor::new(2, 1).unwrap())
        .unwrap();
    let mut commands = GpuCommandEncoder::new(2).unwrap();
    let atlas = commands
        .add_glyph_atlas(GpuGlyphAtlas::from_image(
            Image::from_rgba8(2, 1, vec![255, 255, 255, 128, 255, 0, 0, 255]).unwrap(),
        ))
        .unwrap();
    commands.clear(Color::BLACK).unwrap();
    commands
        .draw_glyph_batch(
            atlas,
            vec![
                GpuGlyphQuad::new(
                    GpuAtlasRect::new(0, 0, 1, 1).unwrap(),
                    rect(0, 0, 1, 1),
                    true,
                ),
                GpuGlyphQuad::new(
                    GpuAtlasRect::new(1, 0, 1, 1).unwrap(),
                    rect(1, 0, 2, 1),
                    false,
                ),
            ],
            Paint::new(Color::rgba(0, 0, 255, 255)),
        )
        .unwrap();
    backend.submit(&mut surface, &commands.finish()).unwrap();
    assert_eq!(
        surface.read_rgba8().unwrap(),
        [0, 0, 128, 255, 255, 0, 0, 255]
    );
}

#[test]
fn metal_backend_applies_path_and_difference_clip_masks_on_hardware() {
    let Some(mut backend) = backend_or_skip() else {
        return;
    };
    let mut surface = backend
        .create_surface(GpuSurfaceDescriptor::new(7, 7).unwrap())
        .unwrap();
    let mut path = PathBuilder::new(5).unwrap();
    path.add_rect(rect(1, 1, 6, 6)).unwrap();
    let mut commands = GpuCommandEncoder::new(2).unwrap();
    let path = commands.add_path(path.finish().unwrap()).unwrap();
    commands.clear(Color::BLACK).unwrap();
    commands
        .clip_path(path, FillRule::NonZero, ClipOp::Intersect)
        .unwrap();
    commands
        .clip_rect_with_op(rect(2, 2, 5, 5), ClipOp::Difference)
        .unwrap();
    commands
        .fill_rect(rect(0, 0, 7, 7), Paint::new(Color::WHITE))
        .unwrap();
    backend.submit(&mut surface, &commands.finish()).unwrap();

    let pixels = surface.read_rgba8().unwrap();
    assert_eq!(pixel(&pixels, 7, 1, 1), Color::WHITE.channels());
    assert_eq!(pixel(&pixels, 7, 3, 3), Color::BLACK.channels());
    assert_eq!(pixel(&pixels, 7, 5, 5), Color::WHITE.channels());
    assert_eq!(pixel(&pixels, 7, 0, 0), Color::BLACK.channels());
}

#[test]
fn metal_backend_applies_transformed_rect_clip_masks_on_hardware() {
    let Some(mut backend) = backend_or_skip() else {
        return;
    };
    let mut surface = backend
        .create_surface(GpuSurfaceDescriptor::new(6, 6).unwrap())
        .unwrap();
    let mut commands = GpuCommandEncoder::new(2).unwrap();
    commands.clear(Color::BLACK).unwrap();
    commands.set_transform(Transform::new(
        Scalar::ZERO,
        Scalar::from_i32(1).unwrap(),
        Scalar::from_i32(-1).unwrap(),
        Scalar::ZERO,
        Scalar::from_i32(5).unwrap(),
        Scalar::ZERO,
    ));
    commands.clip_rect(rect(1, 1, 4, 3)).unwrap();
    commands.set_transform(Transform::IDENTITY);
    commands
        .fill_rect(rect(0, 0, 6, 6), Paint::new(Color::WHITE))
        .unwrap();
    backend.submit(&mut surface, &commands.finish()).unwrap();

    let pixels = surface.read_rgba8().unwrap();
    assert_eq!(pixel(&pixels, 6, 2, 1), Color::WHITE.channels());
    assert_eq!(pixel(&pixels, 6, 3, 3), Color::WHITE.channels());
    assert_eq!(pixel(&pixels, 6, 1, 2), Color::BLACK.channels());
    assert_eq!(pixel(&pixels, 6, 4, 2), Color::BLACK.channels());
}

#[test]
fn metal_backend_applies_complex_clips_to_glyph_batches_on_hardware() {
    let Some(mut backend) = backend_or_skip() else {
        return;
    };
    let mut surface = backend
        .create_surface(GpuSurfaceDescriptor::new(2, 1).unwrap())
        .unwrap();
    let mut path = PathBuilder::new(5).unwrap();
    path.add_rect(rect(0, 0, 1, 1)).unwrap();
    let mut commands = GpuCommandEncoder::new(2).unwrap();
    let path = commands.add_path(path.finish().unwrap()).unwrap();
    let atlas = commands
        .add_glyph_atlas(GpuGlyphAtlas::from_image(
            Image::from_rgba8(1, 1, vec![255, 255, 255, 255]).unwrap(),
        ))
        .unwrap();
    commands.clear(Color::BLACK).unwrap();
    commands
        .clip_path(path, FillRule::NonZero, ClipOp::Intersect)
        .unwrap();
    commands
        .draw_glyph_batch(
            atlas,
            vec![GpuGlyphQuad::new(
                GpuAtlasRect::new(0, 0, 1, 1).unwrap(),
                rect(0, 0, 2, 1),
                true,
            )],
            Paint::new(Color::WHITE),
        )
        .unwrap();
    backend.submit(&mut surface, &commands.finish()).unwrap();

    assert_eq!(
        surface.read_rgba8().unwrap(),
        [255, 255, 255, 255, 0, 0, 0, 255]
    );
}

#[test]
fn metal_backend_reuses_and_evicts_native_atlas_textures_across_submissions() {
    let Some(mut backend) = backend_or_skip() else {
        return;
    };
    backend.set_atlas_cache_capacity(1);
    let mut surface = backend
        .create_surface(GpuSurfaceDescriptor::new(1, 1).unwrap())
        .unwrap();
    let red = atlas_commands([255, 0, 0, 255], GpuGlyphAtlasKey::new(1));
    let blue = atlas_commands([0, 0, 255, 255], GpuGlyphAtlasKey::new(1));

    backend.submit(&mut surface, &red).unwrap();
    assert_eq!(backend.atlas_cache_stats().uploads(), 1);
    assert_eq!(backend.atlas_cache_stats().hits(), 0);
    backend.submit(&mut surface, &red).unwrap();
    assert_eq!(backend.atlas_cache_stats().uploads(), 1);
    assert_eq!(backend.atlas_cache_stats().hits(), 1);

    backend.submit(&mut surface, &blue).unwrap();
    assert_eq!(backend.atlas_cache_stats().uploads(), 2);
    assert_eq!(backend.atlas_cache_stats().evictions(), 1);
    assert_eq!(backend.atlas_cache_stats().entries(), 1);
    assert_eq!(surface.read_rgba8().unwrap(), [0, 0, 255, 255]);

    backend.submit(&mut surface, &red).unwrap();
    assert_eq!(backend.atlas_cache_stats().uploads(), 3);
    assert_eq!(backend.atlas_cache_stats().evictions(), 2);
    assert_eq!(backend.atlas_cache_stats().retained_bytes(), 4);
    assert_eq!(surface.read_rgba8().unwrap(), [255, 0, 0, 255]);
    backend.set_atlas_cache_byte_limit(0);
    assert_eq!(backend.atlas_cache_stats().entries(), 0);
    assert_eq!(backend.atlas_cache_stats().retained_bytes(), 0);
    assert_eq!(backend.atlas_cache_stats().evictions(), 3);
}

fn atlas_commands(pixel: [u8; 4], cache_key: GpuGlyphAtlasKey) -> GpuCommandBuffer {
    let mut commands = GpuCommandEncoder::new(2).unwrap();
    let atlas = commands
        .add_glyph_atlas(
            GpuGlyphAtlas::from_image(Image::from_rgba8(1, 1, pixel.to_vec()).unwrap())
                .with_cache_key(cache_key),
        )
        .unwrap();
    commands.clear(Color::BLACK).unwrap();
    commands
        .draw_glyph_batch(
            atlas,
            vec![GpuGlyphQuad::new(
                GpuAtlasRect::new(0, 0, 1, 1).unwrap(),
                rect(0, 0, 1, 1),
                false,
            )],
            Paint::new(Color::WHITE),
        )
        .unwrap();
    commands.finish()
}

fn rect(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
    Rect::new(
        Scalar::from_i32(left).unwrap(),
        Scalar::from_i32(top).unwrap(),
        Scalar::from_i32(right).unwrap(),
        Scalar::from_i32(bottom).unwrap(),
    )
    .unwrap()
}

fn pixel(pixels: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
    let offset = (y * width + x) * 4;
    pixels[offset..offset + 4].try_into().unwrap()
}

fn backend_or_skip() -> Option<MetalBackend> {
    match MetalBackend::new() {
        Ok(backend) => Some(backend),
        Err(error)
            if error.code() == MetalErrorCode::DeviceUnavailable
                && std::env::var_os("SKIA_REQUIRE_METAL_DEVICE").is_none() =>
        {
            None
        }
        Err(error) => panic!("unexpected Metal initialization failure: {error}"),
    }
}
