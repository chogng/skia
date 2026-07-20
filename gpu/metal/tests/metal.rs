use skia_core::{Color, Paint, Rect, Scalar};
use skia_gpu::{
    GpuAtlasRect, GpuBackend, GpuCommandEncoder, GpuGlyphAtlas, GpuGlyphQuad, GpuSurfaceDescriptor,
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
            Paint::new(Color::BLACK),
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

fn rect(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
    Rect::new(
        Scalar::from_i32(left).unwrap(),
        Scalar::from_i32(top).unwrap(),
        Scalar::from_i32(right).unwrap(),
        Scalar::from_i32(bottom).unwrap(),
    )
    .unwrap()
}

fn backend_or_skip() -> Option<MetalBackend> {
    match MetalBackend::new() {
        Ok(backend) => Some(backend),
        Err(error) if error.code() == MetalErrorCode::DeviceUnavailable => None,
        Err(error) => panic!("unexpected Metal initialization failure: {error}"),
    }
}
