use skia_core::Color;
use skia_gpu::{GpuCommandEncoder, software::SoftwareGpuBackend};

use super::{
    BackendSurfaceFactory, BackendTextureImageFactory, GpuContextType, SubmissionTracker,
    two_color_bc1_compress,
};

#[test]
fn context_types_classify_native_rendering_backends() {
    assert_eq!(GpuContextType::Vulkan.name(), "vulkan");
    assert!(GpuContextType::Metal.is_native());
    assert!(!GpuContextType::Software.is_native());
    assert!(GpuContextType::Software.is_rendering());
}

#[test]
fn managed_image_factory_builds_checkerboard_sources() {
    let image = BackendTextureImageFactory::checkerboard(2, 2, 1, Color::RED, Color::BLUE)
        .expect("checkerboard image");
    assert_eq!(image.image().pixel_at(0, 0), Some(Color::RED.channels()));
    assert_eq!(image.image().pixel_at(1, 0), Some(Color::BLUE.channels()));
    assert_eq!(image.image().pixel_at(0, 1), Some(Color::BLUE.channels()));
}

#[test]
fn two_color_bc1_compression_preserves_transparent_fixture_pixels() {
    let image = BackendTextureImageFactory::from_rgba8(2, 1, vec![0, 0, 0, 0, 255, 0, 0, 255])
        .expect("fixture image");
    let compressed = two_color_bc1_compress(image.image(), Color::RED).expect("compressed image");
    assert_eq!(compressed.len(), 8);
    let indices = u32::from_le_bytes(compressed[4..8].try_into().expect("BC1 indices"));
    assert_eq!(indices & 3, 3);
    assert_eq!((indices >> 2) & 3, 1);
}

#[test]
fn surface_factory_and_tracker_share_one_completed_submission() {
    let mut factory = BackendSurfaceFactory::new(SoftwareGpuBackend::default());
    let mut surface = factory.create_rgba8(1, 1).expect("surface");
    let mut encoder = GpuCommandEncoder::new(1).expect("encoder");
    encoder.clear(Color::GREEN).expect("clear");
    let mut tracker = SubmissionTracker::default();
    tracker
        .submit(factory.backend_mut(), &mut surface, &encoder.finish())
        .expect("submit");
    assert_eq!((tracker.submitted(), tracker.finished()), (1, 1));
    assert!(tracker.is_finished());
}
