use std::sync::Arc;

use skia_core::{
    BlendMode, ClipOp, Color, DisplayList, DrawCommand, FillRule, GlyphOutline,
    GlyphOutlineProvider, GlyphRun, OutlinePoint, OutlineSegment, Paint, Path, PathBuilder,
    PathVerb, Point, PositionedGlyph, Rect, Scalar, ShapedParagraph, SkiaError, SkiaErrorCode,
    StrokeCap, StrokeJoin, StrokeOptions, TextLayout, TextStyleId, TextUnit, Transform,
};
use skia_image::Image;

use crate::{
    clip::{apply_clip, mask_index},
    stroke::{stroke_bounds, stroke_contains, stroke_pieces},
};

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
                    paint,
                } => {
                    let image = list
                        .image(*image)
                        .ok_or(SkiaError::new(SkiaErrorCode::InvalidResource))?;
                    canvas.draw_image(image, *destination, *opacity, paint.blend_mode())?;
                }
                DrawCommand::DrawGlyphRun { run, paint } => {
                    let run = list
                        .glyph_run(*run)
                        .ok_or(SkiaError::new(SkiaErrorCode::InvalidResource))?;
                    canvas.draw_glyph_run(run, glyphs, *paint)?;
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

/// Mutable CPU drawing context.
pub struct Canvas<'a> {
    surface: &'a mut Surface,
    state: State,
    saves: Vec<State>,
}

impl Canvas<'_> {
    /// Clears all pixels, ignoring the current transform and clip.
    pub fn clear(&mut self, color: Color) {
        for pixel in self.surface.pixels.chunks_exact_mut(4) {
            pixel.copy_from_slice(&color.channels());
        }
    }

    /// Saves the current transform and clip state.
    pub fn save(&mut self) -> Result<(), SkiaError> {
        if self.saves.len() == self.surface.limits.max_save_depth {
            return Err(SkiaError::new(SkiaErrorCode::ResourceLimit));
        }
        self.saves
            .try_reserve(1)
            .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
        self.saves.push(self.state.clone());
        Ok(())
    }

    /// Restores the most recently saved state.
    pub fn restore(&mut self) -> Result<(), SkiaError> {
        self.state = self
            .saves
            .pop()
            .ok_or(SkiaError::new(SkiaErrorCode::RestoreUnderflow))?;
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
        let contour = Contour {
            points,
            closed: true,
        };
        self.apply_complex_clip(&[contour], FillRule::NonZero, op)
    }

    /// Applies a transformed path to the current clip.
    pub fn clip_path(&mut self, path: &Path, rule: FillRule, op: ClipOp) -> Result<(), SkiaError> {
        let contours = transformed_contours(path, self.state.transform)?;
        if contours.iter().all(|contour| contour.points.len() < 3) {
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
        let contour = Contour {
            points,
            closed: true,
        };
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
        if contours.iter().all(|contour| contour.points.len() < 3) {
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
        if contours.iter().all(|contour| contour.points.len() < 2) {
            return Err(SkiaError::new(SkiaErrorCode::InvalidPath));
        }
        let pieces = stroke_pieces(&contours, options)?;
        let bounds = stroke_bounds(&contours, options)?.intersection(self.state.scissor);
        for y in bounds.top..bounds.bottom {
            for x in bounds.left..bounds.right {
                let sample = pixel_center(x, y)?;
                if stroke_contains(&pieces, sample, options)? {
                    self.blend_pixel(x, y, paint)?;
                }
            }
        }
        Ok(())
    }

    /// Draws an immutable RGBA8 bitmap into an axis-aligned destination rectangle.
    ///
    /// Sampling is nearest-neighbor at destination pixel centers. `opacity`
    /// multiplies only the source alpha; it does not tint the source color.
    /// Rotated and sheared bitmap sampling is deliberately rejected until the
    /// inverse-mapping and filtering contract is available.
    pub fn draw_image(
        &mut self,
        image: &Image,
        destination: Rect,
        opacity: u8,
        blend_mode: BlendMode,
    ) -> Result<(), SkiaError> {
        if !self.state.transform.is_axis_aligned() {
            return Err(SkiaError::new(SkiaErrorCode::UnsupportedTransform));
        }
        let rectangle = self.device_rect(destination)?;
        let clipped = rectangle.intersection(self.state.scissor);
        let width = rectangle.right - rectangle.left;
        let height = rectangle.bottom - rectangle.top;
        if width == 0 || height == 0 {
            return Ok(());
        }
        for y in clipped.top..clipped.bottom {
            let source_y = u32::try_from(
                (y - rectangle.top)
                    .checked_mul(i64::from(image.height()))
                    .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?
                    / height,
            )
            .map_err(|_| SkiaError::new(SkiaErrorCode::NumericOverflow))?;
            for x in clipped.left..clipped.right {
                let source_x = u32::try_from(
                    (x - rectangle.left)
                        .checked_mul(i64::from(image.width()))
                        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?
                        / width,
                )
                .map_err(|_| SkiaError::new(SkiaErrorCode::NumericOverflow))?;
                let [red, green, blue, alpha] = image
                    .pixel_at(source_x, source_y)
                    .ok_or(SkiaError::new(SkiaErrorCode::InvalidResource))?;
                let color = Color::rgba(red, green, blue, alpha).with_opacity(opacity);
                self.blend_color(x, y, color, blend_mode)?;
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
            self.save()?;
            let draw = (|| {
                let run_origin_bits = origin
                    .x()
                    .bits()
                    .checked_add(shaped.origin_x_bits())
                    .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
                let run_origin = Scalar::from_bits(run_origin_bits);
                self.concat(Transform::translate(run_origin, origin.y()))?;
                let run = shaped.glyph_run();
                if shaped.glyph_offsets_x_bits().len() != run.glyphs().len() {
                    return Err(SkiaError::new(SkiaErrorCode::InvalidResource));
                }
                let mut applied_offset_bits = 0_i32;
                for (glyph, offset_bits) in run.glyphs().iter().zip(shaped.glyph_offsets_x_bits()) {
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
            draw.and(restore)?;
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
                let paint = paint_for_style(TextStyleId::DEFAULT)
                    .ok_or(SkiaError::new(SkiaErrorCode::InvalidResource))?;
                for metrics in [line.underline_metrics(), line.strike_through_metrics()]
                    .into_iter()
                    .flatten()
                {
                    self.draw_decoration_line(
                        line_origin_bits,
                        line_origin_bits
                            .checked_add(line.advance_x_bits())
                            .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?,
                        baseline_bits,
                        metrics,
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
        paint: Paint,
    ) -> Result<(), SkiaError> {
        let center_bits = baseline_bits
            .checked_add(metrics.offset_bits())
            .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
        let top_bits = center_bits
            .checked_sub(metrics.thickness_bits() / 2)
            .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
        let bottom_bits = top_bits
            .checked_add(metrics.thickness_bits())
            .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
        self.fill_rect(
            Rect::new(
                Scalar::from_bits(left_bits),
                Scalar::from_bits(top_bits),
                Scalar::from_bits(right_bits),
                Scalar::from_bits(bottom_bits),
            )?,
            paint,
        )
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
        let Some(path) = glyph_path(run, glyph, &outline)? else {
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
        self.blend_color(x, y, paint.color(), paint.blend_mode())
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
        let index = y
            .checked_mul(i64::from(self.surface.width))
            .and_then(|value| value.checked_add(x))
            .and_then(|value| value.checked_mul(4))
            .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
        let index =
            usize::try_from(index).map_err(|_| SkiaError::new(SkiaErrorCode::NumericOverflow))?;
        let destination = Color::rgba(
            self.surface.pixels[index],
            self.surface.pixels[index + 1],
            self.surface.pixels[index + 2],
            self.surface.pixels[index + 3],
        );
        let result = source.composite(destination, blend_mode);
        self.surface.pixels[index..index + 4].copy_from_slice(&result.channels());
        Ok(())
    }
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

const CURVE_STEPS: i64 = 16;

#[derive(Debug)]
pub(crate) struct Contour {
    pub(crate) points: Vec<Point>,
    pub(crate) closed: bool,
}

fn glyph_path(
    run: &GlyphRun,
    glyph: PositionedGlyph,
    outline: &GlyphOutline,
) -> Result<Option<Path>, SkiaError> {
    if outline.segments().is_empty() {
        return Ok(None);
    }
    let mut builder = PathBuilder::new(outline.segments().len())?;
    for segment in outline.segments() {
        match *segment {
            OutlineSegment::MoveTo(point) => {
                builder.move_to(scaled_outline_point(run, glyph, point)?)?
            }
            OutlineSegment::LineTo(point) => {
                builder.line_to(scaled_outline_point(run, glyph, point)?)?
            }
            OutlineSegment::QuadTo { control, end } => builder.quad_to(
                scaled_outline_point(run, glyph, control)?,
                scaled_outline_point(run, glyph, end)?,
            )?,
            OutlineSegment::CubicTo {
                first_control,
                second_control,
                end,
            } => builder.cubic_to(
                scaled_outline_point(run, glyph, first_control)?,
                scaled_outline_point(run, glyph, second_control)?,
                scaled_outline_point(run, glyph, end)?,
            )?,
            OutlineSegment::Close => builder.close()?,
        }
    }
    builder.finish().map(Some)
}

fn scaled_outline_point(
    run: &GlyphRun,
    glyph: PositionedGlyph,
    point: OutlinePoint,
) -> Result<Point, SkiaError> {
    Ok(Point::new(
        scaled_text_coordinate(point.x(), glyph.x(), run)?,
        scaled_text_coordinate(point.y(), glyph.y(), run)?,
    ))
}

fn scaled_text_coordinate(
    outline: TextUnit,
    position: TextUnit,
    run: &GlyphRun,
) -> Result<Scalar, SkiaError> {
    let design = i64::from(outline.bits())
        .checked_add(i64::from(position.bits()))
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    let numerator = i128::from(design)
        .checked_mul(i128::from(run.font_size_bits()))
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    let denominator = i128::from(64_i32)
        .checked_mul(i128::from(run.units_per_em()))
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    let rounded = if numerator >= 0 {
        numerator
            .checked_add(denominator / 2)
            .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?
            / denominator
    } else {
        -((-numerator
            .checked_add(denominator / 2)
            .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?)
            / denominator)
    };
    i32::try_from(rounded)
        .map(Scalar::from_bits)
        .map_err(|_| SkiaError::new(SkiaErrorCode::NumericOverflow))
}

pub(crate) fn transformed_contours(
    path: &Path,
    transform: Transform,
) -> Result<Vec<Contour>, SkiaError> {
    let mut contours = Vec::new();
    let mut current = Vec::new();
    for verb in path.verbs() {
        match *verb {
            PathVerb::MoveTo(point) => {
                if !current.is_empty() {
                    push_contour(&mut contours, current, false)?;
                    current = Vec::new();
                }
                push_point(&mut current, transform.map_point(point)?)?;
            }
            PathVerb::LineTo(point) => {
                if current.is_empty() {
                    return Err(SkiaError::new(SkiaErrorCode::InvalidPath));
                }
                push_point(&mut current, transform.map_point(point)?)?;
            }
            PathVerb::QuadTo(control, end) => {
                let start = *current
                    .last()
                    .ok_or(SkiaError::new(SkiaErrorCode::InvalidPath))?;
                flatten_quad(
                    &mut current,
                    start,
                    transform.map_point(control)?,
                    transform.map_point(end)?,
                )?;
            }
            PathVerb::ConicTo(control, end, weight) => {
                let start = *current
                    .last()
                    .ok_or(SkiaError::new(SkiaErrorCode::InvalidPath))?;
                flatten_conic(
                    &mut current,
                    start,
                    transform.map_point(control)?,
                    transform.map_point(end)?,
                    weight,
                )?;
            }
            PathVerb::CubicTo(first_control, second_control, end) => {
                let start = *current
                    .last()
                    .ok_or(SkiaError::new(SkiaErrorCode::InvalidPath))?;
                flatten_cubic(
                    &mut current,
                    start,
                    transform.map_point(first_control)?,
                    transform.map_point(second_control)?,
                    transform.map_point(end)?,
                )?;
            }
            PathVerb::Close => {
                if !current.is_empty() {
                    push_contour(&mut contours, current, true)?;
                    current = Vec::new();
                }
            }
        }
    }
    if !current.is_empty() {
        push_contour(&mut contours, current, false)?;
    }
    if contours.is_empty() {
        return Err(SkiaError::new(SkiaErrorCode::InvalidPath));
    }
    Ok(contours)
}

fn push_contour(
    contours: &mut Vec<Contour>,
    points: Vec<Point>,
    closed: bool,
) -> Result<(), SkiaError> {
    contours
        .try_reserve(1)
        .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
    contours.push(Contour { points, closed });
    Ok(())
}

fn push_point(points: &mut Vec<Point>, point: Point) -> Result<(), SkiaError> {
    points
        .try_reserve(1)
        .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
    points.push(point);
    Ok(())
}

fn flatten_quad(
    output: &mut Vec<Point>,
    start: Point,
    control: Point,
    end: Point,
) -> Result<(), SkiaError> {
    output
        .try_reserve(usize::try_from(CURVE_STEPS).unwrap_or(usize::MAX))
        .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
    for step in 1..=CURVE_STEPS {
        push_point(
            output,
            Point::new(
                bezier2(start.x(), control.x(), end.x(), step)?,
                bezier2(start.y(), control.y(), end.y(), step)?,
            ),
        )?;
    }
    Ok(())
}

fn flatten_conic(
    output: &mut Vec<Point>,
    start: Point,
    control: Point,
    end: Point,
    weight: skia_core::ConicWeight,
) -> Result<(), SkiaError> {
    output
        .try_reserve(usize::try_from(CURVE_STEPS).unwrap_or(usize::MAX))
        .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
    for step in 1..=CURVE_STEPS {
        push_point(
            output,
            Point::new(
                conic_coordinate(start.x(), control.x(), end.x(), weight, step)?,
                conic_coordinate(start.y(), control.y(), end.y(), weight, step)?,
            ),
        )?;
    }
    Ok(())
}

fn conic_coordinate(
    start: Scalar,
    control: Scalar,
    end: Scalar,
    weight: skia_core::ConicWeight,
    step: i64,
) -> Result<Scalar, SkiaError> {
    let inverse = CURVE_STEPS
        .checked_sub(step)
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    let start_weight = i128::from(inverse)
        .checked_mul(i128::from(inverse))
        .and_then(|value| value.checked_mul(i128::from(1_i64 << 16)))
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    let control_weight = i128::from(2_i64)
        .checked_mul(i128::from(inverse))
        .and_then(|value| value.checked_mul(i128::from(step)))
        .and_then(|value| value.checked_mul(i128::from(weight.bits())))
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    let end_weight = i128::from(step)
        .checked_mul(i128::from(step))
        .and_then(|value| value.checked_mul(i128::from(1_i64 << 16)))
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    let denominator = start_weight
        .checked_add(control_weight)
        .and_then(|value| value.checked_add(end_weight))
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    let numerator = i128::from(start.bits())
        .checked_mul(start_weight)
        .and_then(|value| {
            i128::from(control.bits())
                .checked_mul(control_weight)
                .and_then(|middle| value.checked_add(middle))
        })
        .and_then(|value| {
            i128::from(end.bits())
                .checked_mul(end_weight)
                .and_then(|last| value.checked_add(last))
        })
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    rounded_scalar(numerator, denominator)
}

fn flatten_cubic(
    output: &mut Vec<Point>,
    start: Point,
    first_control: Point,
    second_control: Point,
    end: Point,
) -> Result<(), SkiaError> {
    output
        .try_reserve(usize::try_from(CURVE_STEPS).unwrap_or(usize::MAX))
        .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
    for step in 1..=CURVE_STEPS {
        push_point(
            output,
            Point::new(
                bezier3(
                    start.x(),
                    first_control.x(),
                    second_control.x(),
                    end.x(),
                    step,
                )?,
                bezier3(
                    start.y(),
                    first_control.y(),
                    second_control.y(),
                    end.y(),
                    step,
                )?,
            ),
        )?;
    }
    Ok(())
}

fn bezier2(start: Scalar, control: Scalar, end: Scalar, step: i64) -> Result<Scalar, SkiaError> {
    let inverse = CURVE_STEPS
        .checked_sub(step)
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    let value = i128::from(start.bits())
        .checked_mul(i128::from(inverse * inverse))
        .and_then(|value| {
            i128::from(control.bits())
                .checked_mul(i128::from(2 * inverse * step))
                .and_then(|middle| value.checked_add(middle))
        })
        .and_then(|value| {
            i128::from(end.bits())
                .checked_mul(i128::from(step * step))
                .and_then(|last| value.checked_add(last))
        })
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    rounded_scalar(value, i128::from(CURVE_STEPS * CURVE_STEPS))
}

fn bezier3(
    start: Scalar,
    first_control: Scalar,
    second_control: Scalar,
    end: Scalar,
    step: i64,
) -> Result<Scalar, SkiaError> {
    let inverse = CURVE_STEPS
        .checked_sub(step)
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    let value = i128::from(start.bits())
        .checked_mul(i128::from(inverse * inverse * inverse))
        .and_then(|value| {
            i128::from(first_control.bits())
                .checked_mul(i128::from(3 * inverse * inverse * step))
                .and_then(|term| value.checked_add(term))
        })
        .and_then(|value| {
            i128::from(second_control.bits())
                .checked_mul(i128::from(3 * inverse * step * step))
                .and_then(|term| value.checked_add(term))
        })
        .and_then(|value| {
            i128::from(end.bits())
                .checked_mul(i128::from(step * step * step))
                .and_then(|term| value.checked_add(term))
        })
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    rounded_scalar(value, i128::from(CURVE_STEPS * CURVE_STEPS * CURVE_STEPS))
}

fn rounded_scalar(value: i128, divisor: i128) -> Result<Scalar, SkiaError> {
    let half = divisor / 2;
    let value = if value >= 0 {
        value
            .checked_add(half)
            .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?
            / divisor
    } else {
        -((-value
            .checked_add(half)
            .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?)
            / divisor)
    };
    i32::try_from(value)
        .map(Scalar::from_bits)
        .map_err(|_| SkiaError::new(SkiaErrorCode::NumericOverflow))
}

pub(crate) fn contour_bounds(contours: &[Contour]) -> DeviceRect {
    let mut left = i64::MAX;
    let mut top = i64::MAX;
    let mut right = i64::MIN;
    let mut bottom = i64::MIN;
    for point in contours.iter().flat_map(|contour| &contour.points) {
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
        if contour.points.len() < 3 {
            continue;
        }
        for (start, end) in contour
            .points
            .iter()
            .copied()
            .zip(contour.points.iter().copied().cycle().skip(1))
            .take(contour.points.len())
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
