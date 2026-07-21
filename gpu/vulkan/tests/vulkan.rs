use skia_core::{Color, Paint, Rect, Scalar};
use skia_gpu::{GpuBackend, GpuCommandEncoder, GpuSurfaceDescriptor};
use skia_vulkan::{VulkanBackend, VulkanErrorCode};

#[test]
fn vulkan_backend_clears_and_reads_an_offscreen_surface() {
    let Some(mut backend) = backend_or_skip() else {
        return;
    };
    assert!(!backend.device_name().is_empty());
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
fn vulkan_backend_fails_closed_for_unimplemented_draws() {
    let Some(mut backend) = backend_or_skip() else {
        return;
    };
    let mut surface = backend
        .create_surface(GpuSurfaceDescriptor::new(2, 2).expect("descriptor"))
        .expect("surface");
    let mut commands = GpuCommandEncoder::new(1).expect("encoder");
    commands
        .fill_rect(
            Rect::new(
                Scalar::ZERO,
                Scalar::ZERO,
                Scalar::from_i32(1).expect("right"),
                Scalar::from_i32(1).expect("bottom"),
            )
            .expect("rect"),
            Paint::new(Color::WHITE),
        )
        .expect("record draw");
    assert_eq!(
        backend
            .submit(&mut surface, &commands.finish())
            .expect_err("unsupported draw")
            .code(),
        VulkanErrorCode::UnsupportedCommand
    );
}

fn backend_or_skip() -> Option<VulkanBackend> {
    match VulkanBackend::new() {
        Ok(backend) => Some(backend),
        Err(error)
            if matches!(
                error.code(),
                VulkanErrorCode::LoaderUnavailable | VulkanErrorCode::DeviceUnavailable
            ) && std::env::var_os("SKIA_REQUIRE_VULKAN_DEVICE").is_none() =>
        {
            None
        }
        Err(error) => panic!("unexpected Vulkan initialization failure: {error}"),
    }
}
