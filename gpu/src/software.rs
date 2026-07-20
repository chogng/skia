//! Deterministic CPU replay for validating GPU command-buffer semantics.
//!
//! This module is not a hardware GPU implementation. It exists so Metal,
//! Vulkan, and WebGPU adapters can be compared with one unambiguous execution
//! of the same [`crate::GpuCommandBuffer`].

use std::fmt;

use skia_core::{Paint, Rect, SkiaError, Transform};
use skia_cpu::{Canvas, ClipRect, Surface, SurfaceLimits};
use skia_image::Image;

use crate::{GpuBackend, GpuCommand, GpuCommandBuffer, GpuGlyphQuad, GpuSurfaceDescriptor};

/// Source-redacted failure from deterministic command replay.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SoftwareGpuError;

impl fmt::Display for SoftwareGpuError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("software GPU command replay failed")
    }
}

impl std::error::Error for SoftwareGpuError {}

/// CPU reference implementation of [`GpuBackend`] for conformance testing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SoftwareGpuBackend {
    limits: SurfaceLimits,
}

impl SoftwareGpuBackend {
    /// Creates one replay backend using explicit CPU surface limits.
    pub const fn new(limits: SurfaceLimits) -> Self {
        Self { limits }
    }
}

impl Default for SoftwareGpuBackend {
    fn default() -> Self {
        Self::new(SurfaceLimits::default())
    }
}

impl GpuBackend for SoftwareGpuBackend {
    type Surface = Surface;
    type Error = SoftwareGpuError;

    fn create_surface(
        &mut self,
        descriptor: GpuSurfaceDescriptor,
    ) -> Result<Self::Surface, Self::Error> {
        Surface::new(descriptor.width(), descriptor.height(), self.limits).map_err(map_error)
    }

    fn submit(
        &mut self,
        surface: &mut Self::Surface,
        commands: &GpuCommandBuffer,
    ) -> Result<(), Self::Error> {
        let mut canvas = surface.canvas();
        for command in commands.commands() {
            match command {
                GpuCommand::Clear(color) => canvas.clear(*color),
                GpuCommand::FillRect {
                    rect,
                    paint,
                    transform,
                    clip,
                } => with_state(&mut canvas, *transform, *clip, |canvas| {
                    canvas.fill_rect(*rect, *paint)
                })?,
                GpuCommand::FillPath {
                    path,
                    rule,
                    paint,
                    transform,
                    clip,
                } => {
                    let path = commands.path(*path).ok_or(SoftwareGpuError)?;
                    with_state(&mut canvas, *transform, *clip, |canvas| {
                        canvas.fill_path(path, *rule, *paint)
                    })?
                }
                GpuCommand::StrokePath {
                    path,
                    options,
                    paint,
                    transform,
                    clip,
                } => {
                    let path = commands.path(*path).ok_or(SoftwareGpuError)?;
                    with_state(&mut canvas, *transform, *clip, |canvas| {
                        canvas.stroke_path_with_options(path, options, *paint)
                    })?
                }
                GpuCommand::DrawImage {
                    image,
                    destination,
                    opacity,
                    blend_mode,
                    transform,
                    clip,
                } => {
                    let image = commands.image(*image).ok_or(SoftwareGpuError)?;
                    with_state(&mut canvas, *transform, *clip, |canvas| {
                        canvas.draw_image(image, *destination, *opacity, *blend_mode)
                    })?
                }
                GpuCommand::DrawGlyphs {
                    atlas,
                    glyphs,
                    paint,
                    transform,
                    clip,
                } => {
                    let atlas = commands.glyph_atlas(*atlas).ok_or(SoftwareGpuError)?;
                    let images: Vec<_> = glyphs
                        .iter()
                        .map(|glyph| {
                            glyph_image(atlas.image(), *glyph, *paint)
                                .map(|image| (image, glyph.destination()))
                        })
                        .collect::<Result<_, _>>()?;
                    with_state(&mut canvas, *transform, *clip, |canvas| {
                        for (image, destination) in &images {
                            canvas.draw_image(image, *destination, u8::MAX, paint.blend_mode())?;
                        }
                        Ok(())
                    })?
                }
            }
        }
        Ok(())
    }
}

fn with_state(
    canvas: &mut Canvas<'_>,
    transform: Transform,
    clip: Option<Rect>,
    draw: impl FnOnce(&mut Canvas<'_>) -> Result<(), SkiaError>,
) -> Result<(), SoftwareGpuError> {
    canvas.save().map_err(map_error)?;
    let result = (|| {
        if let Some(clip) = clip {
            canvas.set_transform(Transform::IDENTITY);
            canvas.clip_rect(ClipRect::new(clip))?;
        }
        canvas.set_transform(transform);
        draw(canvas)
    })();
    let restore = canvas.restore();
    result.and(restore).map_err(map_error)
}

fn map_error(_: SkiaError) -> SoftwareGpuError {
    SoftwareGpuError
}

fn glyph_image(
    atlas: &Image,
    glyph: GpuGlyphQuad,
    paint: Paint,
) -> Result<Image, SoftwareGpuError> {
    let source = glyph.source();
    let length = u64::from(source.width())
        .checked_mul(u64::from(source.height()))
        .and_then(|value| value.checked_mul(4))
        .and_then(|value| usize::try_from(value).ok())
        .ok_or(SoftwareGpuError)?;
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(length)
        .map_err(|_| SoftwareGpuError)?;
    let paint_color = paint.color().channels();
    for y in 0..source.height() {
        for x in 0..source.width() {
            let sample = atlas
                .pixel_at(source.x() + x, source.y() + y)
                .ok_or(SoftwareGpuError)?;
            if glyph.is_mask() {
                pixels.extend_from_slice(&[
                    paint_color[0],
                    paint_color[1],
                    paint_color[2],
                    multiply_alpha(sample[3], paint_color[3]),
                ]);
            } else {
                pixels.extend_from_slice(&[
                    sample[0],
                    sample[1],
                    sample[2],
                    multiply_alpha(sample[3], paint_color[3]),
                ]);
            }
        }
    }
    Image::from_rgba8(source.width(), source.height(), pixels).map_err(|_| SoftwareGpuError)
}

fn multiply_alpha(first: u8, second: u8) -> u8 {
    ((u32::from(first) * u32::from(second) + 127) / 255) as u8
}
