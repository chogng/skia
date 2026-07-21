use std::fmt;

use skia_core::{
    BlendMode, ClipOp, Color, FillRule, Paint, PathBuilder, Point, Rect, SamplingOptions, Scalar,
    StrokeAlign, StrokeCap, StrokeJoin, StrokeOptions, Transform,
};
use skia_gpu::{
    GpuAtlasRect, GpuBackend, GpuClipGeometry, GpuCommand, GpuCommandEncoder, GpuCommandErrorCode,
    GpuCommandLimits, GpuGlyphAtlas, GpuGlyphAtlasKey, GpuGlyphQuad, GpuSurfaceDescriptor,
    software::SoftwareGpuBackend,
};
use skia_image::Image;

fn scalar(value: i32) -> Scalar {
    Scalar::from_i32(value).unwrap()
}

fn point(x: i32, y: i32) -> Point {
    Point::new(scalar(x), scalar(y))
}

fn rect(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
    Rect::new(scalar(left), scalar(top), scalar(right), scalar(bottom)).unwrap()
}

#[derive(Debug)]
struct BackendError;

impl fmt::Display for BackendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("backend failure")
    }
}

impl std::error::Error for BackendError {}

#[derive(Default)]
struct RecordingBackend {
    submitted: Vec<GpuCommand>,
}

impl GpuBackend for RecordingBackend {
    type Surface = GpuSurfaceDescriptor;
    type Error = BackendError;

    fn create_surface(
        &mut self,
        descriptor: GpuSurfaceDescriptor,
    ) -> Result<Self::Surface, Self::Error> {
        Ok(descriptor)
    }

    fn submit(
        &mut self,
        _surface: &mut Self::Surface,
        commands: &skia_gpu::GpuCommandBuffer,
    ) -> Result<(), Self::Error> {
        self.submitted.extend_from_slice(commands.commands());
        Ok(())
    }
}

#[test]
fn gpu_commands_own_resources_and_preserve_submission_order() {
    let mut path = PathBuilder::new(3).unwrap();
    path.move_to(point(0, 0)).unwrap();
    path.line_to(point(2, 0)).unwrap();
    let path = path.finish().unwrap();
    let image = Image::from_rgba8(1, 1, vec![0, 255, 0, 255]).unwrap();

    let mut encoder = GpuCommandEncoder::new(4).unwrap();
    let path = encoder.add_path(path).unwrap();
    let image = encoder.add_image(image).unwrap();
    encoder.clear(Color::WHITE).unwrap();
    encoder.set_transform(Transform::translate(scalar(1), scalar(2)));
    encoder
        .fill_path(path, FillRule::NonZero, Paint::new(Color::BLACK))
        .unwrap();
    encoder
        .draw_image_with_sampling(
            image,
            rect(2, 3, 4, 5),
            128,
            BlendMode::SourceOver,
            SamplingOptions::LINEAR,
        )
        .unwrap();
    let commands = encoder.finish();

    let mut backend = RecordingBackend::default();
    let mut surface = backend
        .create_surface(GpuSurfaceDescriptor::new(12, 8).unwrap())
        .unwrap();
    backend.submit(&mut surface, &commands).unwrap();

    assert_eq!(surface.width(), 12);
    assert_eq!(surface.height(), 8);
    assert!(matches!(
        backend.submitted[0],
        GpuCommand::Clear(Color::WHITE)
    ));
    assert!(matches!(backend.submitted[1], GpuCommand::FillPath { .. }));
    assert!(matches!(backend.submitted[2], GpuCommand::DrawImage { .. }));
    let GpuCommand::DrawImage { sampling, .. } = &backend.submitted[2] else {
        panic!("expected image command");
    };
    assert_eq!(*sampling, SamplingOptions::LINEAR);
    let GpuCommand::FillPath { transform, .. } = &backend.submitted[1] else {
        panic!("expected path command");
    };
    assert_eq!(transform.map_point(point(1, 1)).unwrap(), point(2, 3));
}

#[test]
fn gpu_contract_rejects_invalid_resource_descriptors_and_limits() {
    assert_eq!(
        GpuSurfaceDescriptor::new(0, 1).unwrap_err().code(),
        GpuCommandErrorCode::InvalidSurface
    );
    assert_eq!(
        GpuCommandEncoder::new(0).unwrap_err().code(),
        GpuCommandErrorCode::InvalidLimits
    );
}

#[test]
fn gpu_encoder_scopes_transform_clip_and_resource_limits() {
    let limits = GpuCommandLimits::new(3, 1, 1, 1).unwrap();
    let mut encoder = GpuCommandEncoder::with_limits(limits).unwrap();
    encoder.save().unwrap();
    encoder.set_transform(Transform::translate(scalar(3), scalar(4)));
    encoder.clip_rect(rect(0, 0, 2, 2)).unwrap();
    encoder
        .fill_rect(rect(0, 0, 4, 4), Paint::new(Color::BLACK))
        .unwrap();
    encoder.restore().unwrap();
    encoder
        .fill_rect(rect(0, 0, 1, 1), Paint::new(Color::WHITE))
        .unwrap();

    let mut path = PathBuilder::new(2).unwrap();
    path.move_to(point(0, 0)).unwrap();
    let path = path.finish().unwrap();
    encoder.add_path(path.clone()).unwrap();
    assert_eq!(
        encoder.add_path(path).unwrap_err().code(),
        GpuCommandErrorCode::ResourceLimit
    );
    assert_eq!(
        encoder.restore().unwrap_err().code(),
        GpuCommandErrorCode::RestoreUnderflow
    );

    let commands = encoder.finish();
    let GpuCommand::FillRect {
        transform,
        scissor,
        clip,
        ..
    } = &commands.commands()[0]
    else {
        panic!("expected clipped rectangle command");
    };
    assert_eq!(transform.map_point(point(0, 0)).unwrap(), point(3, 4));
    assert_eq!(*scissor, Some(rect(3, 4, 5, 6)));
    assert_eq!(*clip, None);
    let GpuCommand::FillRect {
        transform,
        scissor,
        clip,
        ..
    } = &commands.commands()[1]
    else {
        panic!("expected restored rectangle command");
    };
    assert_eq!(*transform, Transform::IDENTITY);
    assert_eq!(*scissor, None);
    assert_eq!(*clip, None);
}

#[test]
fn gpu_empty_scissors_skip_draws_but_not_target_clears() {
    let mut encoder = GpuCommandEncoder::new(2).unwrap();
    encoder.clip_rect(rect(0, 0, 1, 1)).unwrap();
    encoder.clip_rect(rect(2, 2, 3, 3)).unwrap();
    encoder
        .fill_rect(rect(0, 0, 3, 3), Paint::new(Color::BLACK))
        .unwrap();
    encoder.clear(Color::WHITE).unwrap();
    let commands = encoder.finish();
    assert_eq!(commands.commands(), &[GpuCommand::Clear(Color::WHITE)]);
}

#[test]
fn gpu_complex_clips_form_shared_immutable_chains() {
    let mut path = PathBuilder::new(5).unwrap();
    path.add_rect(rect(1, 1, 6, 6)).unwrap();
    let mut encoder = GpuCommandEncoder::new(4).unwrap();
    let path = encoder.add_path(path.finish().unwrap()).unwrap();
    encoder
        .clip_path(path, FillRule::NonZero, ClipOp::Intersect)
        .unwrap();
    encoder
        .fill_rect(rect(0, 0, 7, 7), Paint::new(Color::WHITE))
        .unwrap();
    encoder.save().unwrap();
    encoder
        .clip_rect_with_op(rect(2, 2, 5, 5), ClipOp::Difference)
        .unwrap();
    encoder
        .fill_rect(rect(0, 0, 7, 7), Paint::new(Color::BLACK))
        .unwrap();
    encoder.restore().unwrap();
    encoder
        .fill_rect(rect(0, 0, 1, 1), Paint::new(Color::WHITE))
        .unwrap();
    let commands = encoder.finish();

    let clips: Vec<_> = commands
        .commands()
        .iter()
        .map(|command| match command {
            GpuCommand::FillRect { clip, .. } => *clip,
            _ => panic!("expected fill"),
        })
        .collect();
    let root = clips[0].expect("root clip");
    let child = clips[1].expect("child clip");
    assert_eq!(clips[2], Some(root));
    assert_eq!(commands.clip_node(root).unwrap().parent(), None);
    assert_eq!(
        commands.clip_node(root).unwrap().geometry(),
        GpuClipGeometry::Path {
            path,
            rule: FillRule::NonZero,
        }
    );
    assert_eq!(commands.clip_node(child).unwrap().parent(), Some(root));
    assert_eq!(
        commands.clip_node(child).unwrap().geometry(),
        GpuClipGeometry::Rect(rect(2, 2, 5, 5))
    );
    assert_eq!(commands.clip_node(child).unwrap().op(), ClipOp::Difference);
}

#[test]
fn software_replay_applies_path_and_difference_clips() {
    let mut path = PathBuilder::new(5).unwrap();
    path.add_rect(rect(1, 1, 6, 6)).unwrap();
    let mut encoder = GpuCommandEncoder::new(2).unwrap();
    let path = encoder.add_path(path.finish().unwrap()).unwrap();
    encoder
        .clip_path(path, FillRule::NonZero, ClipOp::Intersect)
        .unwrap();
    encoder
        .clip_rect_with_op(rect(2, 2, 5, 5), ClipOp::Difference)
        .unwrap();
    encoder
        .fill_rect(rect(0, 0, 7, 7), Paint::new(Color::WHITE))
        .unwrap();
    let commands = encoder.finish();

    let mut backend = SoftwareGpuBackend::default();
    let mut surface = backend
        .create_surface(GpuSurfaceDescriptor::new(7, 7).unwrap())
        .unwrap();
    backend.submit(&mut surface, &commands).unwrap();

    assert_eq!(pixel(&surface, 1, 1), Color::WHITE.channels());
    assert_eq!(pixel(&surface, 3, 3), Color::TRANSPARENT.channels());
    assert_eq!(pixel(&surface, 5, 5), Color::WHITE.channels());
    assert_eq!(pixel(&surface, 0, 0), Color::TRANSPARENT.channels());
}

#[test]
fn gpu_clip_limits_are_independent_from_draw_commands() {
    let limits = GpuCommandLimits::new(2, 1, 1, 1)
        .unwrap()
        .with_max_clips(1)
        .unwrap();
    let mut encoder = GpuCommandEncoder::with_limits(limits).unwrap();
    encoder
        .clip_rect_with_op(rect(0, 0, 2, 2), ClipOp::Difference)
        .unwrap();
    assert_eq!(
        encoder
            .clip_rect_with_op(rect(0, 0, 1, 1), ClipOp::Difference)
            .unwrap_err()
            .code(),
        GpuCommandErrorCode::ResourceLimit
    );
}

#[test]
fn software_replay_is_a_pixel_oracle_for_gpu_command_state() {
    let image = Image::from_rgba8(1, 1, vec![255, 0, 0, 255]).unwrap();
    let mut encoder = GpuCommandEncoder::new(4).unwrap();
    let image = encoder.add_image(image).unwrap();
    encoder.clear(Color::BLACK).unwrap();
    encoder.save().unwrap();
    encoder.set_transform(Transform::translate(scalar(1), scalar(0)));
    encoder.clip_rect(rect(0, 0, 2, 1)).unwrap();
    encoder
        .fill_rect(rect(0, 0, 3, 1), Paint::new(Color::rgba(0, 0, 255, 255)))
        .unwrap();
    encoder.restore().unwrap();
    encoder
        .draw_image(image, rect(0, 1, 1, 2), 255, BlendMode::SourceOver)
        .unwrap();
    let commands = encoder.finish();

    let mut backend = SoftwareGpuBackend::default();
    let mut surface = backend
        .create_surface(GpuSurfaceDescriptor::new(4, 2).unwrap())
        .unwrap();
    backend.submit(&mut surface, &commands).unwrap();

    assert_eq!(pixel(&surface, 0, 0), [0, 0, 0, 255]);
    assert_eq!(pixel(&surface, 1, 0), [0, 0, 255, 255]);
    assert_eq!(pixel(&surface, 2, 0), [0, 0, 255, 255]);
    assert_eq!(pixel(&surface, 3, 0), [0, 0, 0, 255]);
    assert_eq!(pixel(&surface, 0, 1), [255, 0, 0, 255]);
}

#[test]
fn software_replay_honors_linear_image_sampling() {
    let image = Image::from_rgba8(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]).unwrap();
    let mut encoder = GpuCommandEncoder::new(1).unwrap();
    let image = encoder.add_image(image).unwrap();
    encoder
        .draw_image_with_sampling(
            image,
            rect(0, 0, 4, 1),
            255,
            BlendMode::SourceOver,
            SamplingOptions::LINEAR,
        )
        .unwrap();
    let mut backend = SoftwareGpuBackend::default();
    let mut surface = backend
        .create_surface(GpuSurfaceDescriptor::new(4, 1).unwrap())
        .unwrap();
    backend.submit(&mut surface, &encoder.finish()).unwrap();

    assert_eq!(pixel(&surface, 0, 0), [255, 0, 0, 255]);
    assert_eq!(pixel(&surface, 1, 0), [191, 0, 64, 255]);
    assert_eq!(pixel(&surface, 2, 0), [64, 0, 191, 255]);
    assert_eq!(pixel(&surface, 3, 0), [0, 0, 255, 255]);
}

#[test]
fn gpu_stroke_commands_preserve_options_and_replay_dashes() {
    let mut path = PathBuilder::new(2).unwrap();
    path.move_to(point(2, 5)).unwrap();
    path.line_to(point(18, 5)).unwrap();
    let options = StrokeOptions::new(scalar(2))
        .unwrap()
        .with_align(StrokeAlign::Center)
        .with_cap(StrokeCap::Butt)
        .with_join(StrokeJoin::Bevel)
        .with_dash_pattern(&[scalar(4), scalar(4)], Scalar::ZERO)
        .unwrap();
    let mut encoder = GpuCommandEncoder::new(1).unwrap();
    let path = encoder.add_path(path.finish().unwrap()).unwrap();
    encoder
        .stroke_path(path, options.clone(), Paint::new(Color::WHITE))
        .unwrap();
    let commands = encoder.finish();
    let GpuCommand::StrokePath {
        options: recorded,
        transform,
        scissor,
        clip,
        ..
    } = &commands.commands()[0]
    else {
        panic!("expected stroke command");
    };
    assert_eq!(recorded, &options);
    assert_eq!(*transform, Transform::IDENTITY);
    assert_eq!(*scissor, None);
    assert_eq!(*clip, None);

    let mut backend = SoftwareGpuBackend::default();
    let mut surface = backend
        .create_surface(GpuSurfaceDescriptor::new(20, 11).unwrap())
        .unwrap();
    backend.submit(&mut surface, &commands).unwrap();
    for x in [2, 3, 4, 5, 10, 11, 12, 13] {
        assert_eq!(pixel(&surface, x, 5), Color::WHITE.channels());
    }
    for x in [6, 7, 8, 9, 14, 15, 16, 17] {
        assert_eq!(pixel(&surface, x, 5), Color::TRANSPARENT.channels());
    }
}

#[test]
fn glyph_atlas_batches_tint_masks_and_preserve_color_glyphs() {
    let cache_key = GpuGlyphAtlasKey::new(41);
    let atlas = GpuGlyphAtlas::from_image(
        Image::from_rgba8(2, 1, vec![255, 255, 255, 128, 255, 0, 0, 255]).unwrap(),
    )
    .with_cache_key(cache_key);
    let mut encoder = GpuCommandEncoder::new(2).unwrap();
    let atlas = encoder.add_glyph_atlas(atlas).unwrap();
    encoder.clear(Color::BLACK).unwrap();
    encoder
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
    let commands = encoder.finish();
    assert_eq!(
        commands.glyph_atlas(atlas).unwrap().cache_key(),
        Some(cache_key)
    );
    assert!(matches!(
        commands.commands()[1],
        GpuCommand::DrawGlyphs { ref glyphs, .. } if glyphs.len() == 2
    ));

    let mut backend = SoftwareGpuBackend::default();
    let mut surface = backend
        .create_surface(GpuSurfaceDescriptor::new(2, 1).unwrap())
        .unwrap();
    backend.submit(&mut surface, &commands).unwrap();
    assert_eq!(pixel(&surface, 0, 0), [0, 0, 128, 255]);
    assert_eq!(pixel(&surface, 1, 0), [255, 0, 0, 255]);
}

#[test]
fn glyph_batches_validate_atlas_bounds_and_glyph_limits() {
    let limits = GpuCommandLimits::new(1, 1, 1, 1)
        .unwrap()
        .with_max_glyphs_per_batch(1)
        .unwrap();
    let mut encoder = GpuCommandEncoder::with_limits(limits).unwrap();
    let atlas = encoder
        .add_glyph_atlas(GpuGlyphAtlas::from_image(
            Image::from_rgba8(1, 1, vec![255; 4]).unwrap(),
        ))
        .unwrap();
    let quad = GpuGlyphQuad::new(
        GpuAtlasRect::new(0, 0, 1, 1).unwrap(),
        rect(0, 0, 1, 1),
        true,
    );
    assert_eq!(
        encoder
            .draw_glyph_batch(atlas, vec![quad, quad], Paint::new(Color::BLACK))
            .unwrap_err()
            .code(),
        GpuCommandErrorCode::ResourceLimit
    );
    let outside = GpuGlyphQuad::new(
        GpuAtlasRect::new(1, 0, 1, 1).unwrap(),
        rect(0, 0, 1, 1),
        true,
    );
    assert_eq!(
        encoder
            .draw_glyph_batch(atlas, vec![outside], Paint::new(Color::BLACK))
            .unwrap_err()
            .code(),
        GpuCommandErrorCode::InvalidResource
    );
}

fn pixel(surface: &skia_cpu::Surface, x: usize, y: usize) -> [u8; 4] {
    let offset = (y * surface.width() as usize + x) * 4;
    surface.pixels()[offset..offset + 4].try_into().unwrap()
}
