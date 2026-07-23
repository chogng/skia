use crate::{
    ClipOp, Color, FillRule, Paint, Path, Rect, SamplingOptions, SaveLayerOptions, Scalar,
    SkiaError, SkiaErrorCode, StrokeCap, StrokeJoin, StrokeOptions, Transform,
};
#[cfg(feature = "text")]
use crate::{Point, TextLayoutEvent, text_layout_events};
use skia_image::Image;
#[cfg(feature = "text")]
use skia_text::{GlyphRun, ShapedParagraph, TextLayout, TextStyleId};

/// Command-buffer-local identifier for an immutable path resource.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PathId(u32);

/// Command-buffer-local identifier for an immutable image resource.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ImageId(u32);

/// Command-buffer-local identifier for an immutable shaped glyph run.
#[cfg(feature = "text")]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GlyphRunId(u32);

/// Backend-neutral drawing operation in declaration order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DrawCommand {
    /// Clears the entire target without inheriting state.
    Clear(Color),
    /// Saves the current transform and clip state.
    Save,
    /// Saves state and redirects following draws into an isolated layer.
    SaveLayer(SaveLayerOptions),
    /// Restores the most recently saved state.
    Restore,
    /// Applies a logical rectangle to the current clip.
    ClipRect {
        /// Logical clip rectangle.
        rect: Rect,
        /// Boolean operation against the current clip.
        op: ClipOp,
    },
    /// Applies a registered path to the current clip.
    ClipPath {
        /// Local path resource.
        path: PathId,
        /// Fill containment rule.
        rule: FillRule,
        /// Boolean operation against the current clip.
        op: ClipOp,
    },
    /// Replaces the transform for following draws.
    SetTransform(Transform),
    /// Concatenates an affine transform onto the current drawing state.
    ConcatTransform(Transform),
    /// Fills one logical rectangle.
    FillRect {
        /// Logical rectangle to fill.
        rect: Rect,
        /// Source paint.
        paint: Paint,
    },
    /// Fills a registered path.
    FillPath {
        /// Local path resource.
        path: PathId,
        /// Fill containment rule.
        rule: FillRule,
        /// Source paint.
        paint: Paint,
    },
    /// Strokes a registered path with backend-neutral geometry options.
    StrokePath {
        /// Local path resource.
        path: PathId,
        /// Cap, join, miter, width, and dash geometry.
        options: StrokeOptions,
        /// Source paint.
        paint: Paint,
    },
    /// Draws a registered image into a logical destination rectangle.
    DrawImage {
        /// Local image resource.
        image: ImageId,
        /// Logical destination rectangle.
        destination: Rect,
        /// Additional source opacity.
        opacity: u8,
        /// Reconstruction filter and edge behavior.
        sampling: SamplingOptions,
        /// Source paint and blend mode.
        paint: Paint,
    },
    /// Draws one registered shaped glyph run.
    #[cfg(feature = "text")]
    DrawGlyphRun {
        /// Local shaped glyph-run resource.
        run: GlyphRunId,
        /// Source paint and blend mode.
        paint: Paint,
    },
    /// Draws one registered glyph run at a logical origin with per-glyph layout offsets.
    #[cfg(feature = "text")]
    DrawPositionedGlyphRun {
        /// Local shaped glyph-run resource.
        run: GlyphRunId,
        /// Logical baseline origin applied before glyph positioning.
        origin: Point,
        /// Q16.16 horizontal layout offset paired with every glyph.
        offsets_x_bits: Vec<i32>,
        /// Source paint and blend mode.
        paint: Paint,
    },
}

/// Immutable portable drawing list and the resources it owns.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisplayList {
    commands: Vec<DrawCommand>,
    paths: Vec<Path>,
    images: Vec<Image>,
    #[cfg(feature = "text")]
    glyph_runs: Vec<GlyphRun>,
}

impl DisplayList {
    /// Borrows commands in declaration order.
    pub fn commands(&self) -> &[DrawCommand] {
        &self.commands
    }
    /// Resolves a local path resource.
    pub fn path(&self, id: PathId) -> Option<&Path> {
        self.paths.get(usize::try_from(id.0).ok()?)
    }
    /// Resolves a local image resource.
    pub fn image(&self, id: ImageId) -> Option<&Image> {
        self.images.get(usize::try_from(id.0).ok()?)
    }
    /// Resolves a local shaped glyph-run resource.
    #[cfg(feature = "text")]
    pub fn glyph_run(&self, id: GlyphRunId) -> Option<&GlyphRun> {
        self.glyph_runs.get(usize::try_from(id.0).ok()?)
    }
}

/// Bounded recorder for one immutable [`DisplayList`].
#[derive(Debug)]
pub struct DisplayListBuilder {
    commands: Vec<DrawCommand>,
    paths: Vec<Path>,
    images: Vec<Image>,
    #[cfg(feature = "text")]
    glyph_runs: Vec<GlyphRun>,
    max_items: usize,
}

impl DisplayListBuilder {
    /// Creates a builder with one positive per-kind resource and command ceiling.
    pub fn new(max_items: usize) -> Result<Self, SkiaError> {
        if max_items == 0 {
            return Err(SkiaError::new(SkiaErrorCode::InvalidLimits));
        }
        Ok(Self {
            commands: Vec::new(),
            paths: Vec::new(),
            images: Vec::new(),
            #[cfg(feature = "text")]
            glyph_runs: Vec::new(),
            max_items,
        })
    }
    /// Registers an immutable path.
    pub fn add_path(&mut self, path: Path) -> Result<PathId, SkiaError> {
        let id = self.resource_id(self.paths.len())?;
        self.paths
            .try_reserve(1)
            .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
        self.paths.push(path);
        Ok(PathId(id))
    }
    /// Registers an immutable image.
    pub fn add_image(&mut self, image: Image) -> Result<ImageId, SkiaError> {
        let id = self.resource_id(self.images.len())?;
        self.images
            .try_reserve(1)
            .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
        self.images.push(image);
        Ok(ImageId(id))
    }
    /// Registers immutable shaped glyph output.
    #[cfg(feature = "text")]
    pub fn add_glyph_run(&mut self, run: GlyphRun) -> Result<GlyphRunId, SkiaError> {
        let id = self.resource_id(self.glyph_runs.len())?;
        self.glyph_runs
            .try_reserve(1)
            .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
        self.glyph_runs.push(run);
        Ok(GlyphRunId(id))
    }
    /// Records a target-wide clear that ignores canvas state.
    pub fn clear(&mut self, color: Color) -> Result<(), SkiaError> {
        self.push(DrawCommand::Clear(color))
    }
    /// Records a canvas-state save.
    pub fn save(&mut self) -> Result<(), SkiaError> {
        self.push(DrawCommand::Save)
    }
    /// Records an isolated layer whose options apply when it is restored.
    pub fn save_layer(&mut self, options: SaveLayerOptions) -> Result<(), SkiaError> {
        self.push(DrawCommand::SaveLayer(options))
    }
    /// Records a canvas-state restore.
    pub fn restore(&mut self) -> Result<(), SkiaError> {
        self.push(DrawCommand::Restore)
    }
    /// Records an intersection clip rectangle.
    pub fn clip_rect(&mut self, rect: Rect) -> Result<(), SkiaError> {
        self.clip_rect_with_op(rect, ClipOp::Intersect)
    }
    /// Records a rectangle clip with an explicit boolean operation.
    pub fn clip_rect_with_op(&mut self, rect: Rect, op: ClipOp) -> Result<(), SkiaError> {
        self.push(DrawCommand::ClipRect { rect, op })
    }
    /// Records a registered path clip with an explicit fill rule and operation.
    pub fn clip_path(&mut self, path: PathId, rule: FillRule, op: ClipOp) -> Result<(), SkiaError> {
        self.push(DrawCommand::ClipPath { path, rule, op })
    }
    /// Records a replacement canvas transform.
    pub fn set_transform(&mut self, transform: Transform) -> Result<(), SkiaError> {
        self.push(DrawCommand::SetTransform(transform))
    }
    /// Records an affine transform concatenation for following draws.
    pub fn concat_transform(&mut self, transform: Transform) -> Result<(), SkiaError> {
        self.push(DrawCommand::ConcatTransform(transform))
    }
    /// Records a fill of one logical rectangle.
    pub fn fill_rect(&mut self, rect: Rect, paint: Paint) -> Result<(), SkiaError> {
        self.push(DrawCommand::FillRect { rect, paint })
    }
    /// Records a fill of a registered path.
    pub fn fill_path(
        &mut self,
        path: PathId,
        rule: FillRule,
        paint: Paint,
    ) -> Result<(), SkiaError> {
        self.push(DrawCommand::FillPath { path, rule, paint })
    }
    /// Records a positive-width stroke of a registered path.
    pub fn stroke_path(
        &mut self,
        path: PathId,
        width: Scalar,
        paint: Paint,
    ) -> Result<(), SkiaError> {
        let options = StrokeOptions::new(width)?
            .with_cap(StrokeCap::Round)
            .with_join(StrokeJoin::Round);
        self.stroke_path_with_options(path, options, paint)
    }

    /// Records a stroke of a registered path with explicit geometry options.
    pub fn stroke_path_with_options(
        &mut self,
        path: PathId,
        options: StrokeOptions,
        paint: Paint,
    ) -> Result<(), SkiaError> {
        self.push(DrawCommand::StrokePath {
            path,
            options,
            paint,
        })
    }
    /// Records one registered image draw.
    pub fn draw_image(
        &mut self,
        image: ImageId,
        destination: Rect,
        opacity: u8,
        paint: Paint,
    ) -> Result<(), SkiaError> {
        self.draw_image_with_sampling(image, destination, opacity, paint, SamplingOptions::NEAREST)
    }
    /// Records one registered image draw with explicit sampling.
    pub fn draw_image_with_sampling(
        &mut self,
        image: ImageId,
        destination: Rect,
        opacity: u8,
        paint: Paint,
        sampling: SamplingOptions,
    ) -> Result<(), SkiaError> {
        self.push(DrawCommand::DrawImage {
            image,
            destination,
            opacity,
            sampling,
            paint,
        })
    }
    /// Records one registered shaped glyph run draw.
    #[cfg(feature = "text")]
    pub fn draw_glyph_run(&mut self, run: GlyphRunId, paint: Paint) -> Result<(), SkiaError> {
        self.push(DrawCommand::DrawGlyphRun { run, paint })
    }
    /// Records one positioned glyph run with one Q16.16 offset per glyph.
    #[cfg(feature = "text")]
    pub fn draw_positioned_glyph_run(
        &mut self,
        run: GlyphRunId,
        origin: Point,
        offsets_x_bits: Vec<i32>,
        paint: Paint,
    ) -> Result<(), SkiaError> {
        let glyphs = self
            .glyph_runs
            .get(
                usize::try_from(run.0)
                    .map_err(|_| SkiaError::new(SkiaErrorCode::InvalidResource))?,
            )
            .ok_or(SkiaError::new(SkiaErrorCode::InvalidResource))?;
        if offsets_x_bits.len() != glyphs.glyphs().len() {
            return Err(SkiaError::new(SkiaErrorCode::InvalidResource));
        }
        self.push(DrawCommand::DrawPositionedGlyphRun {
            run,
            origin,
            offsets_x_bits,
            paint,
        })
    }

    /// Expands one shaped paragraph into positioned glyph-run commands.
    #[cfg(feature = "text")]
    pub fn draw_shaped_paragraph(
        &mut self,
        paragraph: &ShapedParagraph,
        origin: Point,
        paint: Paint,
    ) -> Result<(), SkiaError> {
        self.draw_shaped_paragraph_with_styles(paragraph, origin, &|_| Some(paint))
    }

    /// Expands one shaped paragraph with caller-resolved per-run paints.
    #[cfg(feature = "text")]
    pub fn draw_shaped_paragraph_with_styles(
        &mut self,
        paragraph: &ShapedParagraph,
        origin: Point,
        paint_for_style: &impl Fn(TextStyleId) -> Option<Paint>,
    ) -> Result<(), SkiaError> {
        self.record_text_transaction(|builder| {
            builder.record_shaped_paragraph(paragraph, origin, paint_for_style)
        })
    }

    /// Expands a text layout into positioned glyph runs and decoration rectangles.
    #[cfg(feature = "text")]
    pub fn draw_text_layout(
        &mut self,
        layout: &TextLayout,
        origin: Point,
        paint: Paint,
    ) -> Result<(), SkiaError> {
        self.draw_text_layout_with_styles(layout, origin, &|_| Some(paint))
    }

    /// Expands a text layout using caller-resolved glyph and decoration paints.
    #[cfg(feature = "text")]
    pub fn draw_text_layout_with_styles(
        &mut self,
        layout: &TextLayout,
        origin: Point,
        paint_for_style: &impl Fn(TextStyleId) -> Option<Paint>,
    ) -> Result<(), SkiaError> {
        let events = text_layout_events(layout, origin)?;
        self.record_text_transaction(|builder| {
            for event in events {
                match event {
                    TextLayoutEvent::GlyphRun {
                        style_id,
                        run,
                        origin,
                        offsets_x_bits,
                    } => {
                        let paint = paint_for_style(style_id)
                            .ok_or(SkiaError::new(SkiaErrorCode::InvalidResource))?;
                        let mut offsets = Vec::new();
                        offsets
                            .try_reserve_exact(offsets_x_bits.len())
                            .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
                        offsets.extend_from_slice(offsets_x_bits);
                        let run = builder.add_glyph_run(run.clone())?;
                        builder.draw_positioned_glyph_run(run, origin, offsets, paint)?;
                    }
                    TextLayoutEvent::Decoration { style_id, rect } => {
                        let paint = paint_for_style(style_id)
                            .ok_or(SkiaError::new(SkiaErrorCode::InvalidResource))?;
                        builder.fill_rect(rect, paint)?;
                    }
                }
            }
            Ok(())
        })
    }

    #[cfg(feature = "text")]
    fn record_text_transaction<T>(
        &mut self,
        operation: impl FnOnce(&mut Self) -> Result<T, SkiaError>,
    ) -> Result<T, SkiaError> {
        let command_count = self.commands.len();
        let glyph_run_count = self.glyph_runs.len();
        let result = operation(self);
        if result.is_err() {
            self.commands.truncate(command_count);
            self.glyph_runs.truncate(glyph_run_count);
        }
        result
    }

    #[cfg(feature = "text")]
    fn record_shaped_paragraph(
        &mut self,
        paragraph: &ShapedParagraph,
        origin: Point,
        paint_for_style: &impl Fn(TextStyleId) -> Option<Paint>,
    ) -> Result<(), SkiaError> {
        for shaped in paragraph.runs() {
            let paint = paint_for_style(shaped.style_id())
                .ok_or(SkiaError::new(SkiaErrorCode::InvalidResource))?;
            let run = shaped.glyph_run();
            if shaped.glyph_offsets_x_bits().len() != run.glyphs().len() {
                return Err(SkiaError::new(SkiaErrorCode::InvalidResource));
            }
            let run_x = origin
                .x()
                .bits()
                .checked_add(shaped.origin_x_bits())
                .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
            let mut offsets_x_bits = Vec::new();
            offsets_x_bits
                .try_reserve_exact(shaped.glyph_offsets_x_bits().len())
                .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
            offsets_x_bits.extend_from_slice(shaped.glyph_offsets_x_bits());
            let run = self.add_glyph_run(run.clone())?;
            self.draw_positioned_glyph_run(
                run,
                Point::new(Scalar::from_bits(run_x), origin.y()),
                offsets_x_bits,
                paint,
            )?;
        }
        Ok(())
    }

    /// Records one command.
    pub fn push(&mut self, command: DrawCommand) -> Result<(), SkiaError> {
        if self.commands.len() == self.max_items {
            return Err(SkiaError::new(SkiaErrorCode::ResourceLimit));
        }
        self.commands
            .try_reserve(1)
            .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
        self.commands.push(command);
        Ok(())
    }
    /// Publishes the list.
    pub fn finish(self) -> DisplayList {
        DisplayList {
            commands: self.commands,
            paths: self.paths,
            images: self.images,
            #[cfg(feature = "text")]
            glyph_runs: self.glyph_runs,
        }
    }
    fn resource_id(&self, length: usize) -> Result<u32, SkiaError> {
        if length == self.max_items {
            return Err(SkiaError::new(SkiaErrorCode::ResourceLimit));
        }
        u32::try_from(length).map_err(|_| SkiaError::new(SkiaErrorCode::ResourceLimit))
    }
}
