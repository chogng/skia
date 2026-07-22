use std::sync::Arc;

use skia_core::{
    BlendMode, ClipOp, Color, DisplayList, DrawCommand, FillRule, GlyphOutlineProvider, GlyphRun,
    ImageFilter, Paint, Path, Point, PositionedGlyph, Rect, SamplingFilter, SamplingOptions,
    SaveLayerOptions, Scalar, ShapedParagraph, SkiaError, SkiaErrorCode, StrokeCap, StrokeJoin,
    StrokeOptions, TextLayout, TextStyleId, Transform, glyph_outline_path, text_decoration_rects,
};
use skia_image::Image;
use skia_tessellation::{
    DEFAULT_CURVE_STEPS, FlattenedContour, FlatteningLimits, PathFlattener, TessellationErrorCode,
    stroke_mesh,
};

use crate::{
    clip::{apply_clip, mask_index},
    stroke::stroke_bounds,
};

fn map_text_decoration_error(error: skia_core::TextError) -> SkiaError {
    let code = match error.code() {
        skia_core::TextErrorCode::NumericOverflow => SkiaErrorCode::NumericOverflow,
        skia_core::TextErrorCode::ResourceLimit => SkiaErrorCode::ResourceLimit,
        skia_core::TextErrorCode::AllocationFailed => SkiaErrorCode::AllocationFailed,
        _ => SkiaErrorCode::TextResolverFailed,
    };
    SkiaError::new(code)
}

/// Limits for one CPU-owned RGBA8 surface and Canvas state stack.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SurfaceLimits {
    max_pixels: u64,
    max_bytes: u64,
    max_save_depth: usize,
}

impl SurfaceLimits {
    /// Creates checked limits.
    pub fn new(max_pixels: u64, max_bytes: u64, max_save_depth: usize) -> Result<Self, SkiaError> {
        if max_pixels == 0 || max_bytes == 0 || max_save_depth == 0 {
            return Err(SkiaError::new(SkiaErrorCode::InvalidLimits));
        }
        Ok(Self {
            max_pixels,
            max_bytes,
            max_save_depth,
        })
    }
}

impl Default for SurfaceLimits {
    fn default() -> Self {
        Self {
            max_pixels: 67_108_864,
            max_bytes: 256 * 1024 * 1024,
            max_save_depth: 256,
        }
    }
}

/// Complete mutable CPU surface with top-left, tightly packed straight RGBA8 pixels.
#[derive(Debug)]
pub struct Surface {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    limits: SurfaceLimits,
}

impl Surface {
    /// Allocates a transparent, bounded CPU surface.
    pub fn new(width: u32, height: u32, limits: SurfaceLimits) -> Result<Self, SkiaError> {
        if width == 0 || height == 0 {
            return Err(SkiaError::new(SkiaErrorCode::InvalidGeometry));
        }
        let pixels = u64::from(width)
            .checked_mul(u64::from(height))
            .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
        let bytes = pixels
            .checked_mul(4)
            .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
        if pixels > limits.max_pixels || bytes > limits.max_bytes {
            return Err(SkiaError::new(SkiaErrorCode::ResourceLimit));
        }
        let length =
            usize::try_from(bytes).map_err(|_| SkiaError::new(SkiaErrorCode::ResourceLimit))?;
        let mut output = Vec::new();
        output
            .try_reserve_exact(length)
            .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
        output.resize(length, 0);
        Ok(Self {
            width,
            height,
            pixels: output,
            limits,
        })
    }

    /// Returns the device width in pixels.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the device height in pixels.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Borrows the exact row-major RGBA8 pixels.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Starts one canvas state scope over this surface.
    pub fn canvas(&mut self) -> Canvas<'_> {
        let scissor = DeviceRect {
            left: 0,
            top: 0,
            right: i64::from(self.width),
            bottom: i64::from(self.height),
        };
        Canvas {
            surface: self,
            state: State {
                transform: Transform::IDENTITY,
                scissor,
                mask: None,
            },
            saves: Vec::new(),
            layer_buffers: Vec::new(),
        }
    }

    /// Executes a portable display list with the supplied glyph-outline resolver.
    ///
    /// This is the CPU reference implementation of command-layer semantics.
    /// It resolves all resources from the immutable list rather than accepting
    /// backend-local handles.
    pub fn execute_display_list(
        &mut self,
        list: &DisplayList,
        glyphs: &impl GlyphOutlineProvider,
    ) -> Result<(), SkiaError> {
        let mut canvas = self.canvas();
        for command in list.commands() {
            match command {
                DrawCommand::Clear(color) => canvas.clear(*color),
                DrawCommand::Save => canvas.save()?,
                DrawCommand::SaveLayer(options) => canvas.save_layer(*options)?,
                DrawCommand::Restore => canvas.restore()?,
                DrawCommand::ClipRect { rect, op } => {
                    canvas.clip_rect_with_op(ClipRect::new(*rect), *op)?
                }
                DrawCommand::ClipPath { path, rule, op } => {
                    let path = list
                        .path(*path)
                        .ok_or(SkiaError::new(SkiaErrorCode::InvalidResource))?;
                    canvas.clip_path(path, *rule, *op)?;
                }
                DrawCommand::SetTransform(transform) => canvas.set_transform(*transform),
                DrawCommand::ConcatTransform(transform) => canvas.concat(*transform)?,
                DrawCommand::FillRect { rect, paint } => canvas.fill_rect(*rect, *paint)?,
                DrawCommand::FillPath { path, rule, paint } => {
                    let path = list
                        .path(*path)
                        .ok_or(SkiaError::new(SkiaErrorCode::InvalidResource))?;
                    canvas.fill_path(path, *rule, *paint)?;
                }
                DrawCommand::StrokePath {
                    path,
                    options,
                    paint,
                } => {
                    let path = list
                        .path(*path)
                        .ok_or(SkiaError::new(SkiaErrorCode::InvalidResource))?;
                    canvas.stroke_path_with_options(path, options, *paint)?;
                }
                DrawCommand::DrawImage {
                    image,
                    destination,
                    opacity,
                    sampling,
                    paint,
                } => {
                    let image = list
                        .image(*image)
                        .ok_or(SkiaError::new(SkiaErrorCode::InvalidResource))?;
                    canvas.draw_image_with_paint(
                        image,
                        *destination,
                        *opacity,
                        *paint,
                        *sampling,
                    )?;
                }
                DrawCommand::DrawGlyphRun { run, paint } => {
                    let run = list
                        .glyph_run(*run)
                        .ok_or(SkiaError::new(SkiaErrorCode::InvalidResource))?;
                    canvas.draw_glyph_run(run, glyphs, *paint)?;
                }
                DrawCommand::DrawPositionedGlyphRun {
                    run,
                    origin,
                    offsets_x_bits,
                    paint,
                } => {
                    let run = list
                        .glyph_run(*run)
                        .ok_or(SkiaError::new(SkiaErrorCode::InvalidResource))?;
                    canvas.draw_positioned_glyph_run(
                        run,
                        offsets_x_bits,
                        *origin,
                        glyphs,
                        *paint,
                    )?;
                }
            }
        }
        Ok(())
    }
}

/// Axis-aligned clipping rectangle in canvas coordinates.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ClipRect(Rect);

impl ClipRect {
    /// Creates a positive-area clip rectangle.
    pub const fn new(rect: Rect) -> Self {
        Self(rect)
    }
}

#[derive(Clone, Debug)]
struct State {
    transform: Transform,
    scissor: DeviceRect,
    mask: Option<Arc<[u8]>>,
}

#[derive(Clone, Copy, Debug)]
struct ActiveLayer {
    options: SaveLayerOptions,
    bounds: DeviceRect,
}

#[derive(Clone, Debug)]
struct SaveRecord {
    state: State,
    layer: Option<ActiveLayer>,
}

/// Mutable CPU drawing context.
pub struct Canvas<'a> {
    surface: &'a mut Surface,
    state: State,
    saves: Vec<SaveRecord>,
    layer_buffers: Vec<Vec<u8>>,
}

impl Canvas<'_> {
    /// Clears all pixels, ignoring the current transform and clip.
    pub fn clear(&mut self, color: Color) {
        for pixel in self.target_pixels_mut().chunks_exact_mut(4) {
            pixel.copy_from_slice(&color.channels());
        }
    }

    /// Saves the current transform and clip state.
    pub fn save(&mut self) -> Result<(), SkiaError> {
        self.push_save(None)
    }

    /// Saves state and redirects following draws into a transparent isolated layer.
    pub fn save_layer(&mut self, options: SaveLayerOptions) -> Result<(), SkiaError> {
        let bounds = if let Some(bounds) = options.bounds() {
            self.transformed_device_rect(bounds)?
                .intersection(self.state.scissor)
        } else {
            self.state.scissor
        };
        let surface_bytes = u64::try_from(self.surface.pixels.len())
            .map_err(|_| SkiaError::new(SkiaErrorCode::ResourceLimit))?;
        let filter_buffers = usize::from(matches!(
            options.filter(),
            Some(ImageFilter::BoxBlur { .. })
        ))
        .checked_mul(2)
        .ok_or(SkiaError::new(SkiaErrorCode::ResourceLimit))?;
        let retained_surfaces = self
            .layer_buffers
            .len()
            .checked_add(filter_buffers)
            .and_then(|value| value.checked_add(2))
            .ok_or(SkiaError::new(SkiaErrorCode::ResourceLimit))?;
        let retained_bytes = surface_bytes
            .checked_mul(
                u64::try_from(retained_surfaces)
                    .map_err(|_| SkiaError::new(SkiaErrorCode::ResourceLimit))?,
            )
            .ok_or(SkiaError::new(SkiaErrorCode::ResourceLimit))?;
        if retained_bytes > self.surface.limits.max_bytes {
            return Err(SkiaError::new(SkiaErrorCode::ResourceLimit));
        }
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(self.surface.pixels.len())
            .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
        pixels.resize(self.surface.pixels.len(), 0);
        self.layer_buffers
            .try_reserve(1)
            .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
        self.push_save(Some(ActiveLayer { options, bounds }))?;
        self.layer_buffers.push(pixels);
        Ok(())
    }

    /// Restores the most recently saved state.
    pub fn restore(&mut self) -> Result<(), SkiaError> {
        let record = self
            .saves
            .pop()
            .ok_or(SkiaError::new(SkiaErrorCode::RestoreUnderflow))?;
        let layer = if record.layer.is_some() {
            Some(
                self.layer_buffers
                    .pop()
                    .ok_or(SkiaError::new(SkiaErrorCode::InvalidResource))?,
            )
        } else {
            None
        };
        self.state = record.state;
        if let (Some(active), Some(pixels)) = (record.layer, layer) {
            self.restore_layer(active, pixels)?;
        }
        Ok(())
    }

    fn push_save(&mut self, layer: Option<ActiveLayer>) -> Result<(), SkiaError> {
        if self.saves.len() == self.surface.limits.max_save_depth {
            return Err(SkiaError::new(SkiaErrorCode::ResourceLimit));
        }
        self.saves
            .try_reserve(1)
            .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
        self.saves.push(SaveRecord {
            state: self.state.clone(),
            layer,
        });
        Ok(())
    }

    /// Replaces the current affine transform.
    pub fn set_transform(&mut self, transform: Transform) {
        self.state.transform = transform;
    }

    /// Appends an affine transform to the current canvas transform.
    pub fn concat(&mut self, transform: Transform) -> Result<(), SkiaError> {
        self.state.transform = self.state.transform.concat(transform)?;
        Ok(())
    }

    /// Intersects the current clip with one transformed axis-aligned rectangle.
    pub fn clip_rect(&mut self, clip: ClipRect) -> Result<(), SkiaError> {
        self.clip_rect_with_op(clip, ClipOp::Intersect)
    }

    /// Applies a transformed rectangle to the current clip.
    pub fn clip_rect_with_op(&mut self, clip: ClipRect, op: ClipOp) -> Result<(), SkiaError> {
        if op == ClipOp::Intersect && self.state.transform.is_axis_aligned() {
            self.state.scissor = self.state.scissor.intersection(self.device_rect(clip.0)?);
            return Ok(());
        }
        let rect = clip.0;
        let transform = self.state.transform;
        let mut points = Vec::new();
        points
            .try_reserve_exact(4)
            .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
        points.push(transform.map_point(Point::new(rect.left(), rect.top()))?);
        points.push(transform.map_point(Point::new(rect.right(), rect.top()))?);
        points.push(transform.map_point(Point::new(rect.right(), rect.bottom()))?);
        points.push(transform.map_point(Point::new(rect.left(), rect.bottom()))?);
        let contour = Contour::new(points, true);
        self.apply_complex_clip(&[contour], FillRule::NonZero, op)
    }

    /// Applies a transformed path to the current clip.
    pub fn clip_path(&mut self, path: &Path, rule: FillRule, op: ClipOp) -> Result<(), SkiaError> {
        let contours = transformed_contours(path, self.state.transform)?;
        if contours.iter().all(|contour| contour.points().len() < 3) {
            return Err(SkiaError::new(SkiaErrorCode::InvalidPath));
        }
        self.apply_complex_clip(&contours, rule, op)
    }

    /// Fills one transformed rectangle.
    pub fn fill_rect(&mut self, rect: Rect, paint: Paint) -> Result<(), SkiaError> {
        let transform = self.state.transform;
        let mut points = Vec::new();
        points
            .try_reserve_exact(4)
            .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
        points.push(transform.map_point(Point::new(rect.left(), rect.top()))?);
        points.push(transform.map_point(Point::new(rect.right(), rect.top()))?);
        points.push(transform.map_point(Point::new(rect.right(), rect.bottom()))?);
        points.push(transform.map_point(Point::new(rect.left(), rect.bottom()))?);
        let contour = Contour::new(points, true);
        self.fill_contours(&[contour], FillRule::NonZero, paint)
    }

    /// Fills a transformed line path using the selected winding rule.
    pub fn fill_path(
        &mut self,
        path: &Path,
        rule: FillRule,
        paint: Paint,
    ) -> Result<(), SkiaError> {
        let contours = transformed_contours(path, self.state.transform)?;
        if contours.iter().all(|contour| contour.points().len() < 3) {
            return Err(SkiaError::new(SkiaErrorCode::InvalidPath));
        }
        self.fill_contours(&contours, rule, paint)
    }

    /// Strokes a transformed path with round caps and round joins.
    ///
    /// The stroke is center-sampled and therefore deterministic. Curves use
    /// the same fixed flattening as [`Canvas::fill_path`].
    pub fn stroke_path(
        &mut self,
        path: &Path,
        width: Scalar,
        paint: Paint,
    ) -> Result<(), SkiaError> {
        let options = StrokeOptions::new(width)?
            .with_cap(StrokeCap::Round)
            .with_join(StrokeJoin::Round);
        self.stroke_path_with_options(path, &options, paint)
    }

    /// Strokes a transformed path with explicit cap, join, miter, and dash geometry.
    ///
    /// Curves use the same deterministic fixed-step flattening as
    /// [`Canvas::fill_path`]. Dash lengths and stroke width are evaluated in
    /// the transformed canvas coordinate space.
    pub fn stroke_path_with_options(
        &mut self,
        path: &Path,
        options: &StrokeOptions,
        paint: Paint,
    ) -> Result<(), SkiaError> {
        let contours = transformed_contours(path, self.state.transform)?;
        if contours.iter().all(|contour| contour.points().len() < 2) {
            return Err(SkiaError::new(SkiaErrorCode::InvalidPath));
        }
        let mesh = stroke_mesh(&contours, options)?;
        let bounds = stroke_bounds(&contours, options)?.intersection(self.state.scissor);
        for y in bounds.top..bounds.bottom {
            for x in bounds.left..bounds.right {
                let sample = pixel_center(x, y)?;
                if mesh.contains(sample)? {
                    self.blend_pixel(x, y, paint)?;
                }
            }
        }
        Ok(())
    }

    /// Draws an immutable RGBA8 bitmap into a transformed destination rectangle.
    ///
    /// Sampling is nearest-neighbor at destination pixel centers. `opacity`
    /// multiplies only the source alpha; it does not tint the source color.
    /// Rotation, reflection, and shear use checked inverse mapping.
    pub fn draw_image(
        &mut self,
        image: &Image,
        destination: Rect,
        opacity: u8,
        blend_mode: BlendMode,
    ) -> Result<(), SkiaError> {
        self.draw_image_with_sampling(
            image,
            destination,
            opacity,
            blend_mode,
            SamplingOptions::NEAREST,
        )
    }

    /// Draws an RGBA8 bitmap with explicit nearest or bilinear sampling.
    ///
    /// Both filters evaluate destination pixel centers and clamp source
    /// coordinates to the image edge. Linear interpolation operates on all
    /// four straight-alpha RGBA8 channels before applying `opacity` to alpha.
    pub fn draw_image_with_sampling(
        &mut self,
        image: &Image,
        destination: Rect,
        opacity: u8,
        blend_mode: BlendMode,
        sampling: SamplingOptions,
    ) -> Result<(), SkiaError> {
        self.draw_image_with_paint(
            image,
            destination,
            opacity,
            Paint::new(Color::WHITE).with_blend_mode(blend_mode),
            sampling,
        )
    }

    /// Draws an RGBA8 bitmap with paint alpha, color filtering, and compositing.
    ///
    /// Paint RGB and gradients do not tint the image. Paint alpha multiplies
    /// sampled alpha, then the optional color filter runs before compositing.
    pub fn draw_image_with_paint(
        &mut self,
        image: &Image,
        destination: Rect,
        opacity: u8,
        paint: Paint,
        sampling: SamplingOptions,
    ) -> Result<(), SkiaError> {
        let inverse = self.state.transform.inverse()?;
        let rectangle = self.transformed_device_rect(destination)?;
        let clipped = rectangle.intersection(self.state.scissor);
        if clipped.left == clipped.right || clipped.top == clipped.bottom {
            return Ok(());
        }
        for y in clipped.top..clipped.bottom {
            for x in clipped.left..clipped.right {
                let local = inverse.map_point(pixel_center(x, y)?)?;
                if local.x() < destination.left()
                    || local.x() >= destination.right()
                    || local.y() < destination.top()
                    || local.y() >= destination.bottom()
                {
                    continue;
                }
                let [red, green, blue, alpha] = match sampling.filter() {
                    SamplingFilter::Nearest => sample_nearest(image, local, destination)?,
                    SamplingFilter::Linear => sample_linear(image, local, destination)?,
                };
                let color = Color::rgba(red, green, blue, alpha)
                    .with_opacity(opacity)
                    .with_opacity(paint.color().alpha());
                self.blend_color(x, y, paint.filter_color(color), paint.blend_mode())?;
            }
        }
        Ok(())
    }

    /// Fills a shaped glyph run through a portable outline provider.
    ///
    /// The provider owns font lookup and outline extraction; this executor
    /// turns its canvas-oriented design coordinates into core paths and uses
    /// the same deterministic fill pipeline as ordinary vector graphics.
    /// Missing glyphs are skipped so a caller can apply deterministic fallback.
    pub fn draw_glyph_run(
        &mut self,
        run: &GlyphRun,
        provider: &impl GlyphOutlineProvider,
        paint: Paint,
    ) -> Result<(), SkiaError> {
        for glyph in run.glyphs() {
            self.draw_positioned_glyph(run, *glyph, provider, paint)?;
        }
        Ok(())
    }

    /// Draws a glyph run at a baseline origin with one Q16.16 offset per glyph.
    pub fn draw_positioned_glyph_run(
        &mut self,
        run: &GlyphRun,
        offsets_x_bits: &[i32],
        origin: Point,
        provider: &impl GlyphOutlineProvider,
        paint: Paint,
    ) -> Result<(), SkiaError> {
        if offsets_x_bits.len() != run.glyphs().len() {
            return Err(SkiaError::new(SkiaErrorCode::InvalidResource));
        }
        self.save()?;
        let draw = (|| {
            self.concat(Transform::translate(origin.x(), origin.y()))?;
            let mut applied_offset_bits = 0_i32;
            for (glyph, offset_bits) in run.glyphs().iter().zip(offsets_x_bits) {
                let delta_bits = offset_bits
                    .checked_sub(applied_offset_bits)
                    .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
                if delta_bits != 0 {
                    self.concat(Transform::translate(
                        Scalar::from_bits(delta_bits),
                        Scalar::ZERO,
                    ))?;
                    applied_offset_bits = *offset_bits;
                }
                self.draw_positioned_glyph(run, *glyph, provider, paint)?;
            }
            Ok(())
        })();
        let restore = self.restore();
        draw.and(restore)
    }

    /// Draws all visual runs of one shaped paragraph at a common baseline origin.
    ///
    /// Each run is translated by its Q16.16 paragraph origin before using the
    /// ordinary glyph-run path. The canvas transform and clip are restored even
    /// when outline resolution or drawing fails.
    pub fn draw_shaped_paragraph(
        &mut self,
        paragraph: &ShapedParagraph,
        provider: &impl GlyphOutlineProvider,
        origin: Point,
        paint: Paint,
    ) -> Result<(), SkiaError> {
        self.draw_shaped_paragraph_resolved(paragraph, provider, origin, &|_| Some(paint))
    }

    /// Draws a shaped paragraph by resolving each run's caller-defined style ID.
    pub fn draw_shaped_paragraph_with_styles(
        &mut self,
        paragraph: &ShapedParagraph,
        provider: &impl GlyphOutlineProvider,
        origin: Point,
        paint_for_style: &impl Fn(TextStyleId) -> Option<Paint>,
    ) -> Result<(), SkiaError> {
        self.draw_shaped_paragraph_resolved(paragraph, provider, origin, paint_for_style)
    }

    fn draw_shaped_paragraph_resolved(
        &mut self,
        paragraph: &ShapedParagraph,
        provider: &impl GlyphOutlineProvider,
        origin: Point,
        paint_for_style: &impl Fn(TextStyleId) -> Option<Paint>,
    ) -> Result<(), SkiaError> {
        for shaped in paragraph.runs() {
            let paint = paint_for_style(shaped.style_id())
                .ok_or(SkiaError::new(SkiaErrorCode::InvalidResource))?;
            let run_origin_bits = origin
                .x()
                .bits()
                .checked_add(shaped.origin_x_bits())
                .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
            self.draw_positioned_glyph_run(
                shaped.glyph_run(),
                shaped.glyph_offsets_x_bits(),
                Point::new(Scalar::from_bits(run_origin_bits), origin.y()),
                provider,
                paint,
            )?;
        }
        Ok(())
    }

    /// Draws all non-empty lines of one laid-out text block from its top-left origin.
    pub fn draw_text_layout(
        &mut self,
        layout: &TextLayout,
        provider: &impl GlyphOutlineProvider,
        origin: Point,
        paint: Paint,
    ) -> Result<(), SkiaError> {
        self.draw_text_layout_resolved(layout, provider, origin, &|_| Some(paint))
    }

    /// Draws a text layout by resolving each run and decoration style ID.
    pub fn draw_text_layout_with_styles(
        &mut self,
        layout: &TextLayout,
        provider: &impl GlyphOutlineProvider,
        origin: Point,
        paint_for_style: &impl Fn(TextStyleId) -> Option<Paint>,
    ) -> Result<(), SkiaError> {
        self.draw_text_layout_resolved(layout, provider, origin, paint_for_style)
    }

    fn draw_text_layout_resolved(
        &mut self,
        layout: &TextLayout,
        provider: &impl GlyphOutlineProvider,
        origin: Point,
        paint_for_style: &impl Fn(TextStyleId) -> Option<Paint>,
    ) -> Result<(), SkiaError> {
        for line in layout.lines() {
            let Some(paragraph) = line.paragraph() else {
                continue;
            };
            let baseline_bits = origin
                .y()
                .bits()
                .checked_add(line.baseline_y_bits())
                .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
            let line_origin_bits = origin
                .x()
                .bits()
                .checked_add(line.offset_x_bits())
                .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
            self.draw_shaped_paragraph_resolved(
                paragraph,
                provider,
                Point::new(
                    Scalar::from_bits(line_origin_bits),
                    Scalar::from_bits(baseline_bits),
                ),
                paint_for_style,
            )?;
            if line.advance_x_bits() <= 0 {
                continue;
            }
            if line.decoration_segments().is_empty() {
                let metrics = [line.underline_metrics(), line.strike_through_metrics()];
                if metrics.iter().all(Option::is_none) {
                    continue;
                }
                let paint = paint_for_style(TextStyleId::DEFAULT)
                    .ok_or(SkiaError::new(SkiaErrorCode::InvalidResource))?;
                for metrics in metrics.into_iter().flatten() {
                    self.draw_decoration_line(
                        line_origin_bits,
                        line_origin_bits
                            .checked_add(line.advance_x_bits())
                            .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?,
                        baseline_bits,
                        metrics,
                        line.decoration_style(),
                        paint,
                    )?;
                }
            } else {
                for segment in line.decoration_segments() {
                    let paint = paint_for_style(segment.style_id())
                        .ok_or(SkiaError::new(SkiaErrorCode::InvalidResource))?;
                    let left_bits = line_origin_bits
                        .checked_add(segment.left_bits())
                        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
                    let right_bits = line_origin_bits
                        .checked_add(segment.right_bits())
                        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
                    for metrics in [
                        segment.underline_metrics(),
                        segment.strike_through_metrics(),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        self.draw_decoration_line(
                            left_bits,
                            right_bits,
                            baseline_bits,
                            metrics,
                            segment.decoration_style(),
                            paint,
                        )?;
                    }
                }
            }
        }
        Ok(())
    }

    fn draw_decoration_line(
        &mut self,
        left_bits: i32,
        right_bits: i32,
        baseline_bits: i32,
        metrics: skia_core::TextDecorationMetrics,
        style: skia_core::TextDecorationStyle,
        paint: Paint,
    ) -> Result<(), SkiaError> {
        for rect in text_decoration_rects(left_bits, right_bits, baseline_bits, metrics, style)
            .map_err(map_text_decoration_error)?
        {
            self.fill_rect(
                Rect::new(
                    Scalar::from_bits(rect.left_bits()),
                    Scalar::from_bits(rect.top_bits()),
                    Scalar::from_bits(rect.right_bits()),
                    Scalar::from_bits(rect.bottom_bits()),
                )?,
                paint,
            )?;
        }
        Ok(())
    }

    fn draw_positioned_glyph(
        &mut self,
        run: &GlyphRun,
        glyph: PositionedGlyph,
        provider: &impl GlyphOutlineProvider,
        paint: Paint,
    ) -> Result<(), SkiaError> {
        let Some(outline) = provider
            .glyph_outline(run.font(), glyph.glyph())
            .map_err(|_| SkiaError::new(SkiaErrorCode::TextResolverFailed))?
        else {
            return Ok(());
        };
        if outline.font() != run.font() || outline.glyph() != glyph.glyph() {
            return Err(SkiaError::new(SkiaErrorCode::TextResolverFailed));
        }
        let Some(path) = glyph_outline_path(run, glyph, &outline)? else {
            return Ok(());
        };
        self.fill_path(&path, FillRule::NonZero, paint)
    }

    fn fill_contours(
        &mut self,
        contours: &[Contour],
        rule: FillRule,
        paint: Paint,
    ) -> Result<(), SkiaError> {
        let bounds = contour_bounds(contours).intersection(self.state.scissor);
        for y in bounds.top..bounds.bottom {
            for x in bounds.left..bounds.right {
                if contains(contours, pixel_center(x, y)?, rule)? {
                    self.blend_pixel(x, y, paint)?;
                }
            }
        }
        Ok(())
    }

    fn device_rect(&self, rect: Rect) -> Result<DeviceRect, SkiaError> {
        let first = self
            .state
            .transform
            .map_point(Point::new(rect.left(), rect.top()))?;
        let second = self
            .state
            .transform
            .map_point(Point::new(rect.right(), rect.bottom()))?;
        Ok(DeviceRect {
            left: floor_q16(first.x().bits()),
            top: floor_q16(first.y().bits()),
            right: ceil_q16(second.x().bits()),
            bottom: ceil_q16(second.y().bits()),
        }
        .normalized())
    }

    fn transformed_device_rect(&self, rect: Rect) -> Result<DeviceRect, SkiaError> {
        let corners = [
            Point::new(rect.left(), rect.top()),
            Point::new(rect.right(), rect.top()),
            Point::new(rect.right(), rect.bottom()),
            Point::new(rect.left(), rect.bottom()),
        ];
        let mut left = i32::MAX;
        let mut top = i32::MAX;
        let mut right = i32::MIN;
        let mut bottom = i32::MIN;
        for corner in corners {
            let point = self.state.transform.map_point(corner)?;
            left = left.min(point.x().bits());
            top = top.min(point.y().bits());
            right = right.max(point.x().bits());
            bottom = bottom.max(point.y().bits());
        }
        Ok(DeviceRect {
            left: floor_q16(left),
            top: floor_q16(top),
            right: ceil_q16(right),
            bottom: ceil_q16(bottom),
        })
    }

    fn apply_complex_clip(
        &mut self,
        contours: &[Contour],
        rule: FillRule,
        op: ClipOp,
    ) -> Result<(), SkiaError> {
        self.state.mask = Some(apply_clip(
            self.surface.width,
            self.surface.height,
            self.state.scissor,
            self.state.mask.as_deref(),
            contours,
            rule,
            op,
        )?);
        Ok(())
    }

    fn blend_pixel(&mut self, x: i64, y: i64, paint: Paint) -> Result<(), SkiaError> {
        let local = if paint.gradient().is_some() {
            self.state
                .transform
                .inverse()?
                .map_point(pixel_center(x, y)?)?
        } else {
            Point::new(Scalar::ZERO, Scalar::ZERO)
        };
        self.blend_color(x, y, paint.source_color(local)?, paint.blend_mode())
    }

    fn restore_layer(&mut self, layer: ActiveLayer, pixels: Vec<u8>) -> Result<(), SkiaError> {
        let pixels = apply_layer_filter(
            pixels,
            self.surface.width,
            self.surface.height,
            layer.options.filter(),
        )?;
        let bounds = layer.bounds.intersection(self.state.scissor);
        for y in bounds.top..bounds.bottom {
            for x in bounds.left..bounds.right {
                let index = pixel_offset(self.surface.width, x, y)?;
                let source = Color::rgba(
                    pixels[index],
                    pixels[index + 1],
                    pixels[index + 2],
                    pixels[index + 3],
                )
                .with_opacity(layer.options.opacity());
                self.blend_color(x, y, source, layer.options.blend_mode())?;
            }
        }
        Ok(())
    }

    fn target_pixels(&self) -> &[u8] {
        self.layer_buffers
            .last()
            .map_or(self.surface.pixels.as_slice(), Vec::as_slice)
    }

    fn target_pixels_mut(&mut self) -> &mut [u8] {
        if let Some(pixels) = self.layer_buffers.last_mut() {
            pixels
        } else {
            &mut self.surface.pixels
        }
    }

    fn blend_color(
        &mut self,
        x: i64,
        y: i64,
        source: Color,
        blend_mode: BlendMode,
    ) -> Result<(), SkiaError> {
        if x < 0
            || y < 0
            || x >= i64::from(self.surface.width)
            || y >= i64::from(self.surface.height)
        {
            return Ok(());
        }
        if let Some(mask) = self.state.mask.as_deref() {
            let index = mask_index(self.surface.width, x, y)?;
            if mask[index] == 0 {
                return Ok(());
            }
        }
        let index = pixel_offset(self.surface.width, x, y)?;
        let pixels = self.target_pixels();
        let destination = Color::rgba(
            pixels[index],
            pixels[index + 1],
            pixels[index + 2],
            pixels[index + 3],
        );
        let result = source.composite(destination, blend_mode);
        self.target_pixels_mut()[index..index + 4].copy_from_slice(&result.channels());
        Ok(())
    }
}

fn pixel_offset(width: u32, x: i64, y: i64) -> Result<usize, SkiaError> {
    y.checked_mul(i64::from(width))
        .and_then(|value| value.checked_add(x))
        .and_then(|value| value.checked_mul(4))
        .and_then(|value| usize::try_from(value).ok())
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))
}

fn apply_layer_filter(
    mut pixels: Vec<u8>,
    width: u32,
    height: u32,
    filter: Option<ImageFilter>,
) -> Result<Vec<u8>, SkiaError> {
    match filter {
        None => Ok(pixels),
        Some(ImageFilter::Color(filter)) => {
            for pixel in pixels.chunks_exact_mut(4) {
                let color = filter.apply(Color::rgba(pixel[0], pixel[1], pixel[2], pixel[3]));
                pixel.copy_from_slice(&color.channels());
            }
            Ok(pixels)
        }
        Some(ImageFilter::BoxBlur { radius }) => {
            box_blur(pixels, width, height, usize::from(radius))
        }
    }
}

fn box_blur(
    mut pixels: Vec<u8>,
    width: u32,
    height: u32,
    radius: usize,
) -> Result<Vec<u8>, SkiaError> {
    let width = usize::try_from(width).map_err(|_| SkiaError::new(SkiaErrorCode::ResourceLimit))?;
    let height =
        usize::try_from(height).map_err(|_| SkiaError::new(SkiaErrorCode::ResourceLimit))?;
    let kernel = radius
        .checked_mul(2)
        .and_then(|value| value.checked_add(1))
        .ok_or(SkiaError::new(SkiaErrorCode::ResourceLimit))?;
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = pixel[3];
        for channel in &mut pixel[..3] {
            *channel = multiply_255(*channel, alpha);
        }
    }
    let mut horizontal = zeroed_pixels(pixels.len())?;
    let mut prefix = Vec::new();
    prefix
        .try_reserve_exact(width.max(height).saturating_add(1))
        .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
    prefix.resize(width.max(height).saturating_add(1), 0_u64);
    for y in 0..height {
        for channel in 0..4 {
            prefix[0] = 0;
            for x in 0..width {
                let index = (y * width + x) * 4 + channel;
                prefix[x + 1] = prefix[x]
                    .checked_add(u64::from(pixels[index]))
                    .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
            }
            for x in 0..width {
                let start = x.saturating_sub(radius);
                let end = x.saturating_add(radius).saturating_add(1).min(width);
                let sum = prefix[end] - prefix[start];
                horizontal[(y * width + x) * 4 + channel] = rounded_kernel_average(sum, kernel)?;
            }
        }
    }
    let mut output = zeroed_pixels(pixels.len())?;
    for x in 0..width {
        for channel in 0..4 {
            prefix[0] = 0;
            for y in 0..height {
                let index = (y * width + x) * 4 + channel;
                prefix[y + 1] = prefix[y]
                    .checked_add(u64::from(horizontal[index]))
                    .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
            }
            for y in 0..height {
                let start = y.saturating_sub(radius);
                let end = y.saturating_add(radius).saturating_add(1).min(height);
                let sum = prefix[end] - prefix[start];
                output[(y * width + x) * 4 + channel] = rounded_kernel_average(sum, kernel)?;
            }
        }
    }
    for pixel in output.chunks_exact_mut(4) {
        let alpha = pixel[3];
        if alpha == 0 {
            pixel[..3].fill(0);
        } else {
            for channel in &mut pixel[..3] {
                *channel = unpremultiply_255(*channel, alpha);
            }
        }
    }
    Ok(output)
}

fn zeroed_pixels(length: usize) -> Result<Vec<u8>, SkiaError> {
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(length)
        .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
    pixels.resize(length, 0);
    Ok(pixels)
}

fn rounded_kernel_average(sum: u64, kernel: usize) -> Result<u8, SkiaError> {
    let kernel = u64::try_from(kernel).map_err(|_| SkiaError::new(SkiaErrorCode::ResourceLimit))?;
    u8::try_from((sum + kernel / 2) / kernel)
        .map_err(|_| SkiaError::new(SkiaErrorCode::NumericOverflow))
}

fn multiply_255(first: u8, second: u8) -> u8 {
    ((u32::from(first) * u32::from(second) + 127) / 255) as u8
}

fn unpremultiply_255(channel: u8, alpha: u8) -> u8 {
    ((u32::from(channel) * 255 + u32::from(alpha) / 2) / u32::from(alpha)).min(255) as u8
}

fn sample_nearest(image: &Image, sample: Point, destination: Rect) -> Result<[u8; 4], SkiaError> {
    let source_x = nearest_index(
        i64::from(sample.x().bits()) - i64::from(destination.left().bits()),
        i64::from(destination.right().bits()) - i64::from(destination.left().bits()),
        image.width(),
    )?;
    let source_y = nearest_index(
        i64::from(sample.y().bits()) - i64::from(destination.top().bits()),
        i64::from(destination.bottom().bits()) - i64::from(destination.top().bits()),
        image.height(),
    )?;
    image
        .pixel_at(source_x, source_y)
        .ok_or(SkiaError::new(SkiaErrorCode::InvalidResource))
}

fn nearest_index(offset_bits: i64, extent_bits: i64, source_extent: u32) -> Result<u32, SkiaError> {
    let numerator = i128::from(offset_bits)
        .checked_mul(i128::from(source_extent))
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    let index = numerator / i128::from(extent_bits);
    u32::try_from(index.min(i128::from(source_extent - 1)))
        .map_err(|_| SkiaError::new(SkiaErrorCode::NumericOverflow))
}

#[derive(Clone, Copy)]
struct LinearAxis {
    first: u32,
    second: u32,
    second_weight: i128,
    denominator: i128,
}

fn linear_axis(
    offset_bits: i64,
    extent_bits: i64,
    source_extent: u32,
) -> Result<LinearAxis, SkiaError> {
    let extent = i128::from(extent_bits);
    let numerator = i128::from(offset_bits)
        .checked_mul(2)
        .and_then(|value| value.checked_mul(i128::from(source_extent)))
        .and_then(|value| value.checked_sub(extent))
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    let denominator = extent
        .checked_mul(2)
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    let base = numerator.div_euclid(denominator);
    let second_weight = numerator.rem_euclid(denominator);
    let last = i128::from(source_extent - 1);
    Ok(LinearAxis {
        first: u32::try_from(base.clamp(0, last))
            .map_err(|_| SkiaError::new(SkiaErrorCode::NumericOverflow))?,
        second: u32::try_from((base + 1).clamp(0, last))
            .map_err(|_| SkiaError::new(SkiaErrorCode::NumericOverflow))?,
        second_weight,
        denominator,
    })
}

fn sample_linear(image: &Image, sample: Point, destination: Rect) -> Result<[u8; 4], SkiaError> {
    let horizontal = linear_axis(
        i64::from(sample.x().bits()) - i64::from(destination.left().bits()),
        i64::from(destination.right().bits()) - i64::from(destination.left().bits()),
        image.width(),
    )?;
    let vertical = linear_axis(
        i64::from(sample.y().bits()) - i64::from(destination.top().bits()),
        i64::from(destination.bottom().bits()) - i64::from(destination.top().bits()),
        image.height(),
    )?;
    let top_left = image
        .pixel_at(horizontal.first, vertical.first)
        .ok_or(SkiaError::new(SkiaErrorCode::InvalidResource))?;
    let top_right = image
        .pixel_at(horizontal.second, vertical.first)
        .ok_or(SkiaError::new(SkiaErrorCode::InvalidResource))?;
    let bottom_left = image
        .pixel_at(horizontal.first, vertical.second)
        .ok_or(SkiaError::new(SkiaErrorCode::InvalidResource))?;
    let bottom_right = image
        .pixel_at(horizontal.second, vertical.second)
        .ok_or(SkiaError::new(SkiaErrorCode::InvalidResource))?;
    let first_x_weight = horizontal.denominator - horizontal.second_weight;
    let first_y_weight = vertical.denominator - vertical.second_weight;
    let denominator = horizontal
        .denominator
        .checked_mul(vertical.denominator)
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    let mut output = [0_u8; 4];
    for channel in 0..4 {
        let top = i128::from(top_left[channel]) * first_x_weight
            + i128::from(top_right[channel]) * horizontal.second_weight;
        let bottom = i128::from(bottom_left[channel]) * first_x_weight
            + i128::from(bottom_right[channel]) * horizontal.second_weight;
        let value = top
            .checked_mul(first_y_weight)
            .and_then(|value| {
                bottom
                    .checked_mul(vertical.second_weight)
                    .and_then(|bottom| value.checked_add(bottom))
            })
            .and_then(|value| value.checked_add(denominator / 2))
            .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?
            / denominator;
        output[channel] =
            u8::try_from(value).map_err(|_| SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    }
    Ok(output)
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct DeviceRect {
    pub(crate) left: i64,
    pub(crate) top: i64,
    pub(crate) right: i64,
    pub(crate) bottom: i64,
}

impl DeviceRect {
    fn normalized(self) -> Self {
        Self {
            left: self.left.min(self.right),
            top: self.top.min(self.bottom),
            right: self.left.max(self.right),
            bottom: self.top.max(self.bottom),
        }
    }

    pub(crate) fn intersection(self, other: Self) -> Self {
        let left = self.left.max(other.left);
        let top = self.top.max(other.top);
        let right = self.right.min(other.right).max(left);
        let bottom = self.bottom.min(other.bottom).max(top);
        Self {
            left,
            top,
            right,
            bottom,
        }
    }
}

pub(crate) type Contour = FlattenedContour;

pub(crate) fn transformed_contours(
    path: &Path,
    transform: Transform,
) -> Result<Vec<Contour>, SkiaError> {
    let limits =
        FlatteningLimits::for_path(path, DEFAULT_CURVE_STEPS).map_err(map_tessellation_error)?;
    let flattened = PathFlattener::new(limits)
        .flatten(path, transform)
        .map_err(map_tessellation_error)?;
    Ok(flattened.into_contours())
}

fn map_tessellation_error(error: skia_tessellation::TessellationError) -> SkiaError {
    let code = match error.code() {
        TessellationErrorCode::InvalidLimits => SkiaErrorCode::InvalidLimits,
        TessellationErrorCode::NumericOverflow => SkiaErrorCode::NumericOverflow,
        TessellationErrorCode::InvalidPath | TessellationErrorCode::UnsupportedTopology => {
            SkiaErrorCode::InvalidPath
        }
        TessellationErrorCode::ResourceLimit => SkiaErrorCode::ResourceLimit,
        TessellationErrorCode::AllocationFailed => SkiaErrorCode::AllocationFailed,
    };
    SkiaError::new(code)
}

pub(crate) fn contour_bounds(contours: &[Contour]) -> DeviceRect {
    let mut left = i64::MAX;
    let mut top = i64::MAX;
    let mut right = i64::MIN;
    let mut bottom = i64::MIN;
    for point in contours.iter().flat_map(|contour| contour.points()) {
        left = left.min(floor_q16(point.x().bits()));
        top = top.min(floor_q16(point.y().bits()));
        right = right.max(ceil_q16(point.x().bits()));
        bottom = bottom.max(ceil_q16(point.y().bits()));
    }
    DeviceRect {
        left,
        top,
        right,
        bottom,
    }
}

pub(crate) fn contains(
    contours: &[Contour],
    sample: Point,
    rule: FillRule,
) -> Result<bool, SkiaError> {
    let mut parity = false;
    let mut winding = 0_i32;
    for contour in contours {
        if contour.points().len() < 3 {
            continue;
        }
        for (start, end) in contour
            .points()
            .iter()
            .copied()
            .zip(contour.points().iter().copied().cycle().skip(1))
            .take(contour.points().len())
        {
            let start_y = i64::from(start.y().bits());
            let end_y = i64::from(end.y().bits());
            let sample_y = i64::from(sample.y().bits());
            let rising = start_y <= sample_y && sample_y < end_y;
            let falling = end_y <= sample_y && sample_y < start_y;
            if !(rising || falling) {
                continue;
            }
            let dy = end_y
                .checked_sub(start_y)
                .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
            let numerator = i128::from(start.x().bits())
                .checked_mul(i128::from(dy))
                .and_then(|value| {
                    i128::from(sample_y - start_y)
                        .checked_mul(i128::from(
                            i64::from(end.x().bits()) - i64::from(start.x().bits()),
                        ))
                        .and_then(|delta| value.checked_add(delta))
                })
                .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
            let right_of_sample = if dy > 0 {
                numerator > i128::from(sample.x().bits()) * i128::from(dy)
            } else {
                numerator < i128::from(sample.x().bits()) * i128::from(dy)
            };
            if right_of_sample {
                parity = !parity;
                winding += if rising { 1 } else { -1 };
            }
        }
    }
    Ok(match rule {
        FillRule::EvenOdd => parity,
        FillRule::NonZero => winding != 0,
    })
}

pub(crate) fn pixel_center(x: i64, y: i64) -> Result<Point, SkiaError> {
    Ok(Point::new(
        Scalar::from_ratio(
            x.checked_mul(2)
                .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?
                .checked_add(1)
                .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?,
            2,
        )?,
        Scalar::from_ratio(
            y.checked_mul(2)
                .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?
                .checked_add(1)
                .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?,
            2,
        )?,
    ))
}

fn floor_q16(value: i32) -> i64 {
    floor_q16_i64(i64::from(value))
}

pub(crate) fn floor_q16_i64(value: i64) -> i64 {
    if value >= 0 {
        value >> 16
    } else {
        -((-value + 65_535) >> 16)
    }
}

fn ceil_q16(value: i32) -> i64 {
    ceil_q16_i64(i64::from(value))
}

pub(crate) fn ceil_q16_i64(value: i64) -> i64 {
    -floor_q16_i64(value.saturating_neg())
}
