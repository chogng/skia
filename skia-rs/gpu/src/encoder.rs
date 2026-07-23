use skia_core::{
    BlendMode, ClipOp, Color, FillRule, Paint, Path, Point, Rect, SamplingOptions,
    SaveLayerOptions, SkiaError, SkiaErrorCode, StrokeOptions, Transform,
};
use skia_image::Image;

use crate::{
    GpuClipGeometry, GpuClipId, GpuClipNode, GpuCommand, GpuCommandBuffer, GpuCommandError,
    GpuCommandErrorCode, GpuCommandLimits, GpuGlyphAtlas, GpuGlyphAtlasId, GpuGlyphQuad,
    GpuImageId, GpuPathId,
};

/// Bounded recorder for an immutable GPU command buffer.
#[derive(Debug)]
pub struct GpuCommandEncoder {
    commands: Vec<GpuCommand>,
    clips: Vec<GpuClipNode>,
    paths: Vec<Path>,
    images: Vec<Image>,
    glyph_atlases: Vec<GpuGlyphAtlas>,
    limits: GpuCommandLimits,
    state: GpuState,
    saves: Vec<GpuSave>,
}

#[derive(Clone, Copy, Debug)]
struct GpuSave {
    state: GpuState,
    layer: bool,
}

#[derive(Clone, Copy, Debug)]
struct GpuState {
    transform: Transform,
    scissor: ClipState,
    clip: Option<GpuClipId>,
}

#[derive(Clone, Copy, Debug)]
enum ClipState {
    Unbounded,
    Empty,
    Rect(Rect),
}

impl GpuCommandEncoder {
    /// Creates an encoder with a positive command and per-kind resource ceiling.
    pub fn new(max_commands: usize) -> Result<Self, GpuCommandError> {
        Self::with_limits(GpuCommandLimits::new(
            max_commands,
            max_commands,
            max_commands,
            max_commands,
        )?)
    }

    /// Creates an encoder with independent command, resource, and stack ceilings.
    pub fn with_limits(limits: GpuCommandLimits) -> Result<Self, GpuCommandError> {
        Ok(Self {
            commands: Vec::new(),
            clips: Vec::new(),
            paths: Vec::new(),
            images: Vec::new(),
            glyph_atlases: Vec::new(),
            limits,
            state: GpuState {
                transform: Transform::IDENTITY,
                scissor: ClipState::Unbounded,
                clip: None,
            },
            saves: Vec::new(),
        })
    }

    /// Replaces the logical-to-target transform recorded into following draw commands.
    pub fn set_transform(&mut self, transform: Transform) {
        self.state.transform = transform;
    }

    /// Concatenates an affine transform onto the following draw commands.
    pub fn concat_transform(&mut self, transform: Transform) -> Result<(), GpuCommandError> {
        self.state.transform = self
            .state
            .transform
            .concat(transform)
            .map_err(|_| GpuCommandError::new(GpuCommandErrorCode::NumericOverflow))?;
        Ok(())
    }

    /// Saves the current transform and target-space scissor state.
    pub fn save(&mut self) -> Result<(), GpuCommandError> {
        self.push_save(false)
    }

    /// Saves state and begins one isolated layer command range.
    pub fn save_layer(&mut self, options: SaveLayerOptions) -> Result<(), GpuCommandError> {
        self.preflight_save()?;
        let layer = !matches!(self.state.scissor, ClipState::Empty);
        if layer {
            self.push_unclipped(GpuCommand::SaveLayer {
                options,
                transform: self.state.transform,
                scissor: self.scissor(),
                clip: self.state.clip,
            })?;
        }
        self.saves.push(GpuSave {
            state: self.state,
            layer,
        });
        Ok(())
    }

    /// Restores the most recently saved transform and target-space scissor state.
    pub fn restore(&mut self) -> Result<(), GpuCommandError> {
        let save = self
            .saves
            .last()
            .copied()
            .ok_or(GpuCommandError::new(GpuCommandErrorCode::RestoreUnderflow))?;
        if save.layer {
            self.push_unclipped(GpuCommand::RestoreLayer)?;
        }
        self.saves.pop();
        self.state = save.state;
        Ok(())
    }

    fn push_save(&mut self, layer: bool) -> Result<(), GpuCommandError> {
        self.preflight_save()?;
        self.saves.push(GpuSave {
            state: self.state,
            layer,
        });
        Ok(())
    }

    fn preflight_save(&mut self) -> Result<(), GpuCommandError> {
        if self.saves.len() == self.limits.max_save_depth {
            return Err(GpuCommandError::new(GpuCommandErrorCode::ResourceLimit));
        }
        self.saves
            .try_reserve(1)
            .map_err(|_| GpuCommandError::new(GpuCommandErrorCode::AllocationFailed))
    }

    /// Intersects the current clip with one transformed rectangle.
    pub fn clip_rect(&mut self, rect: Rect) -> Result<(), GpuCommandError> {
        self.clip_rect_with_op(rect, ClipOp::Intersect)
    }

    /// Applies a transformed rectangle to the current clip.
    pub fn clip_rect_with_op(&mut self, rect: Rect, op: ClipOp) -> Result<(), GpuCommandError> {
        if op != ClipOp::Intersect || !self.state.transform.is_axis_aligned() {
            return self.push_clip(GpuClipGeometry::Rect(rect), op);
        }
        let clip = map_axis_aligned_rect(self.state.transform, rect)?;
        self.state.scissor = match self.state.scissor {
            ClipState::Unbounded => ClipState::Rect(clip),
            ClipState::Empty => ClipState::Empty,
            ClipState::Rect(current) => intersect_rect(current, clip)
                .map(ClipState::Rect)
                .unwrap_or(ClipState::Empty),
        };
        Ok(())
    }

    /// Applies a registered transformed path to the current clip.
    pub fn clip_path(
        &mut self,
        path: GpuPathId,
        rule: FillRule,
        op: ClipOp,
    ) -> Result<(), GpuCommandError> {
        if self.path(path).is_none() {
            return Err(GpuCommandError::new(GpuCommandErrorCode::InvalidResource));
        }
        self.push_clip(GpuClipGeometry::Path { path, rule }, op)
    }

    /// Registers an immutable vector path and returns its command-buffer-local ID.
    pub fn add_path(&mut self, path: Path) -> Result<GpuPathId, GpuCommandError> {
        let id = resource_id(self.paths.len(), self.limits.max_paths)?;
        self.paths
            .try_reserve(1)
            .map_err(|_| GpuCommandError::new(GpuCommandErrorCode::AllocationFailed))?;
        self.paths.push(path);
        Ok(GpuPathId(id))
    }

    /// Registers an immutable image and returns its command-buffer-local ID.
    pub fn add_image(&mut self, image: Image) -> Result<GpuImageId, GpuCommandError> {
        let id = resource_id(self.images.len(), self.limits.max_images)?;
        self.images
            .try_reserve(1)
            .map_err(|_| GpuCommandError::new(GpuCommandErrorCode::AllocationFailed))?;
        self.images.push(image);
        Ok(GpuImageId(id))
    }

    /// Registers one immutable glyph atlas and returns its command-buffer-local ID.
    pub fn add_glyph_atlas(
        &mut self,
        atlas: GpuGlyphAtlas,
    ) -> Result<GpuGlyphAtlasId, GpuCommandError> {
        let id = resource_id(self.glyph_atlases.len(), self.limits.max_images)?;
        self.glyph_atlases
            .try_reserve(1)
            .map_err(|_| GpuCommandError::new(GpuCommandErrorCode::AllocationFailed))?;
        self.glyph_atlases.push(atlas);
        Ok(GpuGlyphAtlasId(id))
    }

    /// Records a full-target clear.
    pub fn clear(&mut self, color: Color) -> Result<(), GpuCommandError> {
        self.push_unclipped(GpuCommand::Clear(color))
    }

    /// Records one logical rectangle fill.
    pub fn fill_rect(&mut self, rect: Rect, paint: Paint) -> Result<(), GpuCommandError> {
        self.push(GpuCommand::FillRect {
            rect,
            paint,
            transform: self.state.transform,
            scissor: self.scissor(),
            clip: self.state.clip,
        })
    }

    /// Records a fill of a registered path.
    pub fn fill_path(
        &mut self,
        path: GpuPathId,
        rule: FillRule,
        paint: Paint,
    ) -> Result<(), GpuCommandError> {
        if self.path(path).is_none() {
            return Err(GpuCommandError::new(GpuCommandErrorCode::InvalidResource));
        }
        self.push(GpuCommand::FillPath {
            path,
            rule,
            paint,
            transform: self.state.transform,
            scissor: self.scissor(),
            clip: self.state.clip,
        })
    }

    /// Records a stroke of a registered path with explicit geometry options.
    pub fn stroke_path(
        &mut self,
        path: GpuPathId,
        options: StrokeOptions,
        paint: Paint,
    ) -> Result<(), GpuCommandError> {
        let source = self
            .path(path)
            .ok_or(GpuCommandError::new(GpuCommandErrorCode::InvalidResource))?;
        if matches!(self.state.scissor, ClipState::Empty) {
            return Ok(());
        }
        if self.commands.len() == self.limits.max_commands {
            return Err(GpuCommandError::new(GpuCommandErrorCode::ResourceLimit));
        }
        let (path, paint) = if let Some(effect) = paint.path_effect() {
            let Some(expanded) = effect
                .apply(source, Transform::IDENTITY)
                .map_err(map_path_effect_error)?
            else {
                return Ok(());
            };
            (self.add_path(expanded)?, paint.without_path_effect())
        } else {
            (path, paint)
        };
        self.push(GpuCommand::StrokePath {
            path,
            options,
            paint,
            transform: self.state.transform,
            scissor: self.scissor(),
            clip: self.state.clip,
        })
    }

    /// Records one draw of a registered RGBA8 image.
    pub fn draw_image(
        &mut self,
        image: GpuImageId,
        destination: Rect,
        opacity: u8,
        blend_mode: BlendMode,
    ) -> Result<(), GpuCommandError> {
        self.draw_image_with_sampling(
            image,
            destination,
            opacity,
            blend_mode,
            SamplingOptions::NEAREST,
        )
    }

    /// Records one draw of a registered RGBA8 image with explicit sampling.
    pub fn draw_image_with_sampling(
        &mut self,
        image: GpuImageId,
        destination: Rect,
        opacity: u8,
        blend_mode: BlendMode,
        sampling: SamplingOptions,
    ) -> Result<(), GpuCommandError> {
        self.draw_image_with_paint(
            image,
            destination,
            opacity,
            Paint::new(Color::WHITE).with_blend_mode(blend_mode),
            sampling,
        )
    }

    /// Records one image draw with paint alpha, color filtering, and compositing.
    pub fn draw_image_with_paint(
        &mut self,
        image: GpuImageId,
        destination: Rect,
        opacity: u8,
        paint: Paint,
        sampling: SamplingOptions,
    ) -> Result<(), GpuCommandError> {
        if self.image(image).is_none() {
            return Err(GpuCommandError::new(GpuCommandErrorCode::InvalidResource));
        }
        self.push(GpuCommand::DrawImage {
            image,
            destination,
            opacity,
            sampling,
            paint,
            transform: self.state.transform,
            scissor: self.scissor(),
            clip: self.state.clip,
        })
    }

    /// Records one pre-positioned glyph batch using a registered atlas.
    pub fn draw_glyph_batch(
        &mut self,
        atlas: GpuGlyphAtlasId,
        glyphs: Vec<GpuGlyphQuad>,
        paint: Paint,
    ) -> Result<(), GpuCommandError> {
        let atlas_image = self
            .glyph_atlas(atlas)
            .ok_or(GpuCommandError::new(GpuCommandErrorCode::InvalidResource))?
            .image();
        if glyphs.len() > self.limits.max_glyphs_per_batch {
            return Err(GpuCommandError::new(GpuCommandErrorCode::ResourceLimit));
        }
        for glyph in &glyphs {
            let source = glyph.source();
            if source
                .x()
                .checked_add(source.width())
                .is_none_or(|right| right > atlas_image.width())
                || source
                    .y()
                    .checked_add(source.height())
                    .is_none_or(|bottom| bottom > atlas_image.height())
            {
                return Err(GpuCommandError::new(GpuCommandErrorCode::InvalidResource));
            }
        }
        if glyphs.is_empty() {
            return Ok(());
        }
        self.push(GpuCommand::DrawGlyphs {
            atlas,
            glyphs,
            paint,
            transform: self.state.transform,
            scissor: self.scissor(),
            clip: self.state.clip,
        })
    }

    /// Publishes commands and their owned resources for later submission.
    pub fn finish(self) -> GpuCommandBuffer {
        GpuCommandBuffer {
            commands: self.commands,
            clips: self.clips,
            paths: self.paths,
            images: self.images,
            glyph_atlases: self.glyph_atlases,
        }
    }

    fn push(&mut self, command: GpuCommand) -> Result<(), GpuCommandError> {
        if matches!(self.state.scissor, ClipState::Empty) {
            return Ok(());
        }
        self.push_unclipped(command)
    }

    fn push_unclipped(&mut self, command: GpuCommand) -> Result<(), GpuCommandError> {
        if self.commands.len() == self.limits.max_commands {
            return Err(GpuCommandError::new(GpuCommandErrorCode::ResourceLimit));
        }
        self.commands
            .try_reserve(1)
            .map_err(|_| GpuCommandError::new(GpuCommandErrorCode::AllocationFailed))?;
        self.commands.push(command);
        Ok(())
    }

    fn path(&self, id: GpuPathId) -> Option<&Path> {
        self.paths.get(usize::try_from(id.0).ok()?)
    }

    fn image(&self, id: GpuImageId) -> Option<&Image> {
        self.images.get(usize::try_from(id.0).ok()?)
    }

    fn glyph_atlas(&self, id: GpuGlyphAtlasId) -> Option<&GpuGlyphAtlas> {
        self.glyph_atlases.get(usize::try_from(id.0).ok()?)
    }

    fn scissor(&self) -> Option<Rect> {
        match self.state.scissor {
            ClipState::Unbounded | ClipState::Empty => None,
            ClipState::Rect(rect) => Some(rect),
        }
    }

    fn push_clip(&mut self, geometry: GpuClipGeometry, op: ClipOp) -> Result<(), GpuCommandError> {
        if matches!(self.state.scissor, ClipState::Empty) {
            return Ok(());
        }
        let id = resource_id(self.clips.len(), self.limits.max_clips)?;
        self.clips
            .try_reserve(1)
            .map_err(|_| GpuCommandError::new(GpuCommandErrorCode::AllocationFailed))?;
        self.clips.push(GpuClipNode {
            parent: self.state.clip,
            geometry,
            op,
            transform: self.state.transform,
        });
        self.state.clip = Some(GpuClipId(id));
        Ok(())
    }
}

fn resource_id(length: usize, max_resources: usize) -> Result<u32, GpuCommandError> {
    if length == max_resources {
        return Err(GpuCommandError::new(GpuCommandErrorCode::ResourceLimit));
    }
    u32::try_from(length).map_err(|_| GpuCommandError::new(GpuCommandErrorCode::ResourceLimit))
}

fn map_path_effect_error(error: SkiaError) -> GpuCommandError {
    let code = match error.code() {
        SkiaErrorCode::NumericOverflow => GpuCommandErrorCode::NumericOverflow,
        SkiaErrorCode::InvalidLimits => GpuCommandErrorCode::InvalidLimits,
        SkiaErrorCode::ResourceLimit => GpuCommandErrorCode::ResourceLimit,
        SkiaErrorCode::AllocationFailed => GpuCommandErrorCode::AllocationFailed,
        SkiaErrorCode::UnsupportedTransform => GpuCommandErrorCode::UnsupportedTransform,
        SkiaErrorCode::InvalidGeometry
        | SkiaErrorCode::InvalidResource
        | SkiaErrorCode::InvalidImage
        | SkiaErrorCode::InvalidPath
        | SkiaErrorCode::RestoreUnderflow
        | SkiaErrorCode::TextResolverFailed => GpuCommandErrorCode::InvalidResource,
    };
    GpuCommandError::new(code)
}

fn map_axis_aligned_rect(transform: Transform, rect: Rect) -> Result<Rect, GpuCommandError> {
    let first = transform
        .map_point(Point::new(rect.left(), rect.top()))
        .map_err(|_| GpuCommandError::new(GpuCommandErrorCode::NumericOverflow))?;
    let second = transform
        .map_point(Point::new(rect.right(), rect.bottom()))
        .map_err(|_| GpuCommandError::new(GpuCommandErrorCode::NumericOverflow))?;
    Rect::new(
        first.x().min(second.x()),
        first.y().min(second.y()),
        first.x().max(second.x()),
        first.y().max(second.y()),
    )
    .map_err(|_| GpuCommandError::new(GpuCommandErrorCode::NumericOverflow))
}

fn intersect_rect(first: Rect, second: Rect) -> Option<Rect> {
    Rect::new(
        first.left().max(second.left()),
        first.top().max(second.top()),
        first.right().min(second.right()),
        first.bottom().min(second.bottom()),
    )
    .ok()
}
