use skia_core::{
    BlendMode, ClipOp, Color, ColorFilter, ColorMatrix, FillRule, Gradient, GradientStop,
    ImageFilter, Paint, PathBuilder, Point, Rect, RuntimeShader, RuntimeShaderInstruction,
    RuntimeShaderLimits, RuntimeShaderProgram, SamplingOptions, SaveLayerOptions, Scalar,
    ShaderHandle, StrokeCap, StrokeOptions, TileMode, Transform,
};
use skia_gpu::{
    GpuAtlasRect, GpuBackend, GpuCommandEncoder, GpuGlyphAtlas, GpuGlyphQuad, GpuSurfaceDescriptor,
    software::SoftwareGpuBackend,
};
use skia_image::Image;
use skia_vulkan::{VulkanBackend, VulkanErrorCode};

#[test]
fn vulkan_backend_clears_and_reads_an_offscreen_surface() {
    let Some(mut backend) = backend_or_skip() else {
        return;
    };
    assert!(!backend.device_name().is_empty());
    assert!(
        backend
            .capabilities()
            .supports_surface(GpuSurfaceDescriptor::new(4, 3).expect("descriptor"))
    );
    eprintln!("Vulkan device: {}", backend.device_name());
    if std::env::var_os("SKIA_VULKAN_VALIDATION").is_some() {
        assert!(backend.validation_enabled());
    }
    let mut surface = backend
        .create_surface(GpuSurfaceDescriptor::new(4, 3).expect("descriptor"))
        .expect("surface");
    assert_eq!(surface.descriptor().width(), 4);
    assert_eq!(
        surface
            .read_rgba8()
            .expect_err("uninitialized image")
            .code(),
        VulkanErrorCode::ReadbackFailed
    );
    let expected = Color::rgba(10, 20, 30, 255);
    let mut commands = GpuCommandEncoder::new(2).expect("encoder");
    commands.clear(Color::BLACK).expect("first clear");
    commands.clear(expected).expect("last clear");
    backend
        .submit(&mut surface, &commands.finish())
        .expect("submit clear");

    let pixels = surface.read_rgba8().expect("readback");
    assert_eq!(pixels.len(), 4 * 3 * 4);
    for pixel in pixels.chunks_exact(4) {
        assert_eq!(pixel, expected.channels());
    }

    let replacement = Color::rgba(200, 100, 50, 128);
    let mut commands = GpuCommandEncoder::new(1).expect("encoder");
    commands.clear(replacement).expect("replacement clear");
    backend
        .submit(&mut surface, &commands.finish())
        .expect("resubmit clear");
    for pixel in surface
        .read_rgba8()
        .expect("second readback")
        .chunks_exact(4)
    {
        assert_eq!(pixel, replacement.channels());
    }
}

#[test]
fn vulkan_backend_executes_runtime_shaders() {
    let Some(mut backend) = backend_or_skip() else {
        return;
    };
    let program = RuntimeShaderProgram::new(
        &[
            RuntimeShaderInstruction::ConstantColor {
                destination: 0,
                color: Color::RED,
            },
            RuntimeShaderInstruction::UniformColor {
                destination: 1,
                uniform: 0,
            },
            RuntimeShaderInstruction::LocalX {
                destination: 2,
                start: Scalar::ZERO,
                end: Scalar::from_i32(4).expect("end"),
            },
            RuntimeShaderInstruction::Mix {
                destination: 3,
                first: 0,
                second: 1,
                factor: 2,
            },
            RuntimeShaderInstruction::Return { source: 3 },
        ],
        1,
        RuntimeShaderLimits::default(),
    )
    .expect("program");
    let paint = Paint::new(Color::WHITE).with_shader(ShaderHandle::from_runtime(
        RuntimeShader::new(program, &[Color::BLUE]).expect("runtime shader"),
    ));
    let mut encoder = GpuCommandEncoder::new(2).expect("encoder");
    encoder.clear(Color::BLACK).expect("clear");
    encoder
        .fill_rect(rect(0, 0, 4, 1), paint)
        .expect("runtime fill");
    let descriptor = GpuSurfaceDescriptor::new(4, 1).expect("surface descriptor");
    assert_matches_reference(&mut backend, descriptor, &encoder.finish());
}

#[test]
fn vulkan_backend_matches_runtime_shader_arithmetic() {
    let Some(mut backend) = backend_or_skip() else {
        return;
    };
    let program = RuntimeShaderProgram::new(
        &[
            RuntimeShaderInstruction::ConstantColor {
                destination: 0,
                color: Color::rgba(40, 80, 120, 160),
            },
            RuntimeShaderInstruction::UniformColor {
                destination: 1,
                uniform: 0,
            },
            RuntimeShaderInstruction::Multiply {
                destination: 2,
                first: 0,
                second: 1,
            },
            RuntimeShaderInstruction::Add {
                destination: 3,
                first: 0,
                second: 2,
            },
            RuntimeShaderInstruction::Clamp {
                destination: 4,
                source: 3,
            },
            RuntimeShaderInstruction::Return { source: 4 },
        ],
        1,
        RuntimeShaderLimits::default(),
    )
    .expect("program");
    let paint = Paint::new(Color::WHITE).with_shader(ShaderHandle::from_runtime(
        RuntimeShader::new(program, &[Color::rgba(100, 150, 200, 220)]).expect("runtime"),
    ));
    let mut encoder = GpuCommandEncoder::new(2).expect("encoder");
    encoder.clear(Color::BLACK).expect("clear");
    encoder
        .fill_rect(rect(0, 0, 2, 2), paint)
        .expect("runtime fill");
    let descriptor = GpuSurfaceDescriptor::new(2, 2).expect("surface descriptor");
    assert_matches_reference(&mut backend, descriptor, &encoder.finish());
}

#[test]
fn vulkan_backend_executes_portable_draw_commands() {
    let Some(mut backend) = backend_or_skip() else {
        return;
    };
    let mut surface = backend
        .create_surface(GpuSurfaceDescriptor::new(5, 3).expect("descriptor"))
        .expect("surface");

    let mut clip_path = PathBuilder::new(5).expect("clip path");
    clip_path
        .add_rect(rect(1, 0, 4, 3))
        .expect("clip rectangle");
    let mut fill_path = PathBuilder::new(5).expect("fill path");
    fill_path
        .add_rect(rect(0, 0, 2, 2))
        .expect("fill rectangle");

    let mut commands = GpuCommandEncoder::new(5).expect("encoder");
    let clip_path = commands
        .add_path(clip_path.finish().expect("clip path finish"))
        .expect("clip path resource");
    let fill_path = commands
        .add_path(fill_path.finish().expect("fill path finish"))
        .expect("fill path resource");
    let image = commands
        .add_image(Image::from_rgba8(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]).expect("image"))
        .expect("image resource");

    commands.clear(Color::BLACK).expect("clear");
    commands
        .clip_path(clip_path, FillRule::NonZero, ClipOp::Intersect)
        .expect("complex clip");
    commands.set_transform(Transform::translate(
        Scalar::from_i32(1).expect("x"),
        Scalar::ZERO,
    ));
    commands
        .fill_path(fill_path, FillRule::NonZero, Paint::new(Color::WHITE))
        .expect("path draw");
    commands.set_transform(Transform::IDENTITY);
    commands
        .draw_image_with_sampling(
            image,
            rect(2, 2, 4, 3),
            u8::MAX,
            BlendMode::SourceOver,
            SamplingOptions::NEAREST,
        )
        .expect("image draw");
    backend
        .submit(&mut surface, &commands.finish())
        .expect("portable submission");

    let pixels = surface.read_rgba8().expect("readback");
    assert_eq!(pixel(&pixels, 5, 1, 0), Color::WHITE.channels());
    assert_eq!(pixel(&pixels, 5, 2, 1), Color::WHITE.channels());
    assert_eq!(pixel(&pixels, 5, 0, 0), Color::BLACK.channels());
    assert_eq!(pixel(&pixels, 5, 2, 2), Color::RED.channels());
    assert_eq!(pixel(&pixels, 5, 3, 2), Color::BLUE.channels());
    assert_eq!(pixel(&pixels, 5, 4, 2), Color::BLACK.channels());

    let mut follow_up = GpuCommandEncoder::new(1).expect("follow-up encoder");
    follow_up
        .fill_rect(rect(4, 0, 5, 1), Paint::new(Color::RED))
        .expect("follow-up draw");
    backend
        .submit(&mut surface, &follow_up.finish())
        .expect("follow-up submission");
    let pixels = surface.read_rgba8().expect("follow-up readback");
    assert_eq!(pixel(&pixels, 5, 4, 0), Color::RED.channels());
    assert_eq!(pixel(&pixels, 5, 2, 2), Color::RED.channels());
}

#[test]
fn vulkan_backend_matches_the_reference_for_every_command_variant() {
    let Some(mut backend) = backend_or_skip() else {
        return;
    };
    let descriptor = GpuSurfaceDescriptor::new(8, 8).expect("descriptor");
    let mut path = PathBuilder::new(5).expect("path");
    path.add_rect(rect(1, 1, 7, 7)).expect("path rectangle");

    let mut encoder = GpuCommandEncoder::new(8).expect("encoder");
    let path = encoder
        .add_path(path.finish().expect("path finish"))
        .expect("path resource");
    let image = encoder
        .add_image(Image::from_rgba8(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 128]).expect("image"))
        .expect("image resource");
    let atlas = encoder
        .add_glyph_atlas(GpuGlyphAtlas::from_image(
            Image::from_rgba8(1, 1, vec![255, 255, 255, 192]).expect("atlas image"),
        ))
        .expect("atlas resource");

    encoder.clear(Color::BLACK).expect("clear");
    encoder
        .clip_path(path, FillRule::NonZero, ClipOp::Intersect)
        .expect("clip");
    encoder
        .save_layer(
            SaveLayerOptions::new()
                .with_bounds(rect(1, 1, 7, 7))
                .with_opacity(224)
                .with_filter(ImageFilter::box_blur(1).expect("blur")),
        )
        .expect("save layer");
    encoder
        .fill_rect(rect(1, 1, 4, 4), Paint::new(Color::BLUE))
        .expect("fill rectangle");
    encoder
        .fill_path(
            path,
            FillRule::EvenOdd,
            Paint::new(Color::rgba(255, 255, 0, 160)),
        )
        .expect("fill path");
    encoder
        .stroke_path(
            path,
            StrokeOptions::new(Scalar::from_i32(1).expect("stroke width"))
                .expect("stroke")
                .with_cap(StrokeCap::Round),
            Paint::new(Color::WHITE).with_blend_mode(BlendMode::Multiply),
        )
        .expect("stroke path");
    encoder
        .draw_image_with_sampling(
            image,
            rect(2, 2, 6, 4),
            192,
            BlendMode::SourceOver,
            SamplingOptions::LINEAR,
        )
        .expect("draw image");
    encoder
        .draw_glyph_batch(
            atlas,
            vec![GpuGlyphQuad::new(
                GpuAtlasRect::new(0, 0, 1, 1).expect("atlas rectangle"),
                rect(3, 3, 6, 6),
                true,
            )],
            Paint::new(Color::RED),
        )
        .expect("draw glyphs");
    encoder.restore().expect("restore layer");
    let commands = encoder.finish();

    let mut reference = SoftwareGpuBackend::default();
    let mut expected = reference
        .create_surface(descriptor)
        .expect("reference surface");
    reference
        .submit(&mut expected, &commands)
        .expect("reference submission");
    let mut actual = backend.create_surface(descriptor).expect("Vulkan surface");
    backend
        .submit(&mut actual, &commands)
        .expect("Vulkan submission");

    assert_eq!(actual.read_rgba8().expect("readback"), expected.pixels());
}

#[test]
fn vulkan_backend_matches_every_blend_mode() {
    let Some(mut backend) = backend_or_skip() else {
        return;
    };
    let modes = [
        BlendMode::Clear,
        BlendMode::Source,
        BlendMode::Destination,
        BlendMode::SourceOver,
        BlendMode::DestinationOver,
        BlendMode::SourceIn,
        BlendMode::DestinationIn,
        BlendMode::SourceOut,
        BlendMode::DestinationOut,
        BlendMode::SourceAtop,
        BlendMode::DestinationAtop,
        BlendMode::Xor,
        BlendMode::Plus,
        BlendMode::Modulate,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::ColorDodge,
        BlendMode::ColorBurn,
        BlendMode::HardLight,
        BlendMode::SoftLight,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::Hue,
        BlendMode::Saturation,
        BlendMode::Color,
        BlendMode::Luminosity,
    ];
    let descriptor = GpuSurfaceDescriptor::new(modes.len() as u32, 1).expect("descriptor");
    let mut encoder = GpuCommandEncoder::new(modes.len() + 1).expect("encoder");
    encoder
        .clear(Color::rgba(35, 170, 90, 157))
        .expect("destination");
    for (index, mode) in modes.into_iter().enumerate() {
        encoder
            .fill_rect(
                rect(index as i32, 0, index as i32 + 1, 1),
                Paint::new(Color::rgba(220, 45, 180, 113)).with_blend_mode(mode),
            )
            .expect("blend draw");
    }
    assert_matches_reference(&mut backend, descriptor, &encoder.finish());
}

#[test]
fn vulkan_backend_matches_gradients_and_color_filters() {
    let Some(mut backend) = backend_or_skip() else {
        return;
    };
    let descriptor = GpuSurfaceDescriptor::new(8, 4).expect("descriptor");
    let stops = [
        GradientStop::new(Scalar::ZERO, Color::RED).expect("first stop"),
        GradientStop::new(Scalar::from_bits(1 << 15), Color::GREEN).expect("middle stop"),
        GradientStop::new(Scalar::from_i32(1).expect("one"), Color::BLUE).expect("last stop"),
    ];
    let gradient = Gradient::linear(
        Point::new(Scalar::ZERO, Scalar::ZERO),
        Point::new(Scalar::from_i32(4).expect("end"), Scalar::ZERO),
        &stops,
        TileMode::Mirror,
    )
    .expect("gradient");
    let matrix = ColorMatrix::new([
        0,
        1 << 16,
        0,
        0,
        8 << 16,
        1 << 16,
        0,
        0,
        0,
        0,
        0,
        0,
        1 << 16,
        0,
        0,
        0,
        0,
        0,
        1 << 16,
        0,
    ]);
    let mut encoder = GpuCommandEncoder::new(6).expect("encoder");
    encoder.clear(Color::BLACK).expect("clear");
    encoder
        .fill_rect(rect(0, 0, 8, 2), Paint::from_gradient(gradient))
        .expect("gradient fill");
    encoder
        .fill_rect(
            rect(0, 2, 4, 4),
            Paint::new(Color::rgba(40, 100, 180, 190))
                .with_color_filter(ColorFilter::Matrix(matrix)),
        )
        .expect("matrix filter");
    encoder
        .save_layer(
            SaveLayerOptions::new().with_filter(ImageFilter::Color(ColorFilter::Blend {
                color: Color::rgba(240, 30, 10, 100),
                mode: BlendMode::Screen,
            })),
        )
        .expect("filtered layer");
    encoder
        .fill_rect(rect(4, 2, 8, 4), Paint::new(Color::rgba(20, 160, 80, 210)))
        .expect("layer fill");
    encoder.restore().expect("restore layer");
    assert_matches_reference(&mut backend, descriptor, &encoder.finish());
}

fn assert_matches_reference(
    backend: &mut VulkanBackend,
    descriptor: GpuSurfaceDescriptor,
    commands: &skia_gpu::GpuCommandBuffer,
) {
    let mut reference = SoftwareGpuBackend::default();
    let mut expected = reference
        .create_surface(descriptor)
        .expect("reference surface");
    reference
        .submit(&mut expected, commands)
        .expect("reference submission");
    let mut actual = backend.create_surface(descriptor).expect("Vulkan surface");
    backend
        .submit(&mut actual, commands)
        .expect("Vulkan submission");
    assert_eq!(actual.read_rgba8().expect("readback"), expected.pixels());
}

fn rect(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
    Rect::new(
        Scalar::from_i32(left).expect("left"),
        Scalar::from_i32(top).expect("top"),
        Scalar::from_i32(right).expect("right"),
        Scalar::from_i32(bottom).expect("bottom"),
    )
    .expect("rect")
}

fn pixel(pixels: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
    let offset = (y * width + x) * 4;
    pixels[offset..offset + 4].try_into().expect("pixel")
}

fn backend_or_skip() -> Option<VulkanBackend> {
    match VulkanBackend::new() {
        Ok(backend) => Some(backend),
        Err(error)
            if matches!(
                error.code(),
                VulkanErrorCode::LoaderUnavailable
                    | VulkanErrorCode::InstanceCreationFailed
                    | VulkanErrorCode::DeviceUnavailable
            ) && std::env::var_os("SKIA_REQUIRE_VULKAN_DEVICE").is_none() =>
        {
            None
        }
        Err(error) => panic!("unexpected Vulkan initialization failure: {error}"),
    }
}
