//! Backend-neutral GPU submission contracts for `skia`.
//!
//! This crate deliberately contains no Metal, Vulkan, OpenGL, WebGPU, window,
//! thread, or foreign-function binding. Product-specific backend crates own
//! those details and implement [`GpuBackend`]. The command buffer is reusable
//! by renderers, editors, and other clients without coupling them to a
//! particular graphics API.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::fmt;

use skia_core::{
    BlendMode, ClipOp, Color, FillRule, Paint, Path, Point, Rect, SamplingOptions,
    SaveLayerOptions, StrokeOptions, Transform,
};
use skia_image::Image;

/// Stable machine-readable GPU command recording failure.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GpuCommandErrorCode {
    /// A command-buffer ceiling is invalid.
    InvalidLimits,
    /// A GPU surface descriptor has an empty dimension.
    InvalidSurface,
    /// A state restore was requested without a matching save.
    RestoreUnderflow,
    /// The operation needs an unsupported transform mode.
    UnsupportedTransform,
    /// A transform or intermediate geometry calculation overflowed.
    NumericOverflow,
    /// Recording would exceed a configured ceiling.
    ResourceLimit,
    /// A command referred to a resource that is not registered in this encoder.
    InvalidResource,
    /// Recording could not reserve command storage.
    AllocationFailed,
}

/// Source-redacted GPU command recording error.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GpuCommandError {
    code: GpuCommandErrorCode,
}

impl GpuCommandError {
    const fn new(code: GpuCommandErrorCode) -> Self {
        Self { code }
    }

    /// Returns the stable failure code.
    pub const fn code(self) -> GpuCommandErrorCode {
        self.code
    }
}

impl fmt::Display for GpuCommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}", self.code)
    }
}

impl std::error::Error for GpuCommandError {}

/// Bounded dimensions of a GPU render target.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GpuSurfaceDescriptor {
    width: u32,
    height: u32,
}

impl GpuSurfaceDescriptor {
    /// Creates a non-empty GPU render-target descriptor.
    pub fn new(width: u32, height: u32) -> Result<Self, GpuCommandError> {
        if width == 0 || height == 0 {
            return Err(GpuCommandError::new(GpuCommandErrorCode::InvalidSurface));
        }
        Ok(Self { width, height })
    }

    /// Returns the target width in physical pixels.
    pub const fn width(self) -> u32 {
        self.width
    }

    /// Returns the target height in physical pixels.
    pub const fn height(self) -> u32 {
        self.height
    }
}

/// Opaque, command-buffer-local identifier for one immutable vector path.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GpuPathId(u32);

/// Opaque, command-buffer-local identifier for one immutable clip node.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GpuClipId(u32);

/// Opaque, command-buffer-local identifier for one immutable RGBA8 image.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GpuImageId(u32);

/// Opaque, command-buffer-local identifier for one immutable glyph atlas.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GpuGlyphAtlasId(u32);

/// Stable caller-owned identity for reusing one immutable atlas across submissions.
///
/// Backends verify the atlas content before reusing a resource, so accidental
/// key reuse cannot substitute different pixels.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GpuGlyphAtlasKey(u64);

impl GpuGlyphAtlasKey {
    /// Creates one stable atlas cache identity.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the caller-owned identity value.
    pub const fn value(self) -> u64 {
        self.0
    }
}

/// Integer pixel rectangle inside one glyph atlas.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GpuAtlasRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl GpuAtlasRect {
    /// Creates one non-empty atlas pixel rectangle.
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Result<Self, GpuCommandError> {
        if width == 0 || height == 0 {
            return Err(GpuCommandError::new(GpuCommandErrorCode::InvalidResource));
        }
        x.checked_add(width)
            .and_then(|_| y.checked_add(height))
            .ok_or(GpuCommandError::new(GpuCommandErrorCode::NumericOverflow))?;
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }

    /// Returns the left atlas pixel.
    pub const fn x(self) -> u32 {
        self.x
    }

    /// Returns the top atlas pixel.
    pub const fn y(self) -> u32 {
        self.y
    }

    /// Returns the atlas region width.
    pub const fn width(self) -> u32 {
        self.width
    }

    /// Returns the atlas region height.
    pub const fn height(self) -> u32 {
        self.height
    }
}

/// Immutable RGBA8 atlas consumed by generic glyph batch commands.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GpuGlyphAtlas {
    image: Image,
    cache_key: Option<GpuGlyphAtlasKey>,
}

impl GpuGlyphAtlas {
    /// Wraps one prepacked RGBA8 image for low-level glyph batch recording.
    pub fn from_image(image: Image) -> Self {
        Self {
            image,
            cache_key: None,
        }
    }

    /// Associates this immutable atlas with a cross-submission cache identity.
    pub const fn with_cache_key(mut self, cache_key: GpuGlyphAtlasKey) -> Self {
        self.cache_key = Some(cache_key);
        self
    }

    /// Borrows the upload-ready straight-alpha RGBA8 atlas image.
    pub const fn image(&self) -> &Image {
        &self.image
    }

    /// Returns the optional cross-submission cache identity.
    pub const fn cache_key(&self) -> Option<GpuGlyphAtlasKey> {
        self.cache_key
    }
}

/// One positioned atlas quad inside a batched glyph command.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GpuGlyphQuad {
    source: GpuAtlasRect,
    destination: Rect,
    mask: bool,
}

impl GpuGlyphQuad {
    /// Creates one atlas sample and logical destination pair.
    pub const fn new(source: GpuAtlasRect, destination: Rect, mask: bool) -> Self {
        Self {
            source,
            destination,
            mask,
        }
    }

    /// Returns the source rectangle inside the atlas.
    pub const fn source(self) -> GpuAtlasRect {
        self.source
    }

    /// Returns the logical destination rectangle.
    pub const fn destination(self) -> Rect {
        self.destination
    }

    /// Returns whether atlas alpha should tint the command paint color.
    pub const fn is_mask(self) -> bool {
        self.mask
    }
}

/// Independent command, resource, and state-stack ceilings for one encoder.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GpuCommandLimits {
    max_commands: usize,
    max_paths: usize,
    max_images: usize,
    max_clips: usize,
    max_save_depth: usize,
    max_glyphs_per_batch: usize,
}

impl GpuCommandLimits {
    /// Creates positive, bounded limits for one command encoder.
    pub fn new(
        max_commands: usize,
        max_paths: usize,
        max_images: usize,
        max_save_depth: usize,
    ) -> Result<Self, GpuCommandError> {
        if max_commands == 0 || max_paths == 0 || max_images == 0 || max_save_depth == 0 {
            return Err(GpuCommandError::new(GpuCommandErrorCode::InvalidLimits));
        }
        Ok(Self {
            max_commands,
            max_paths,
            max_images,
            max_clips: max_commands,
            max_save_depth,
            max_glyphs_per_batch: max_commands.saturating_mul(1_024),
        })
    }

    /// Replaces the positive immutable clip-node ceiling.
    pub const fn with_max_clips(mut self, max_clips: usize) -> Result<Self, GpuCommandError> {
        if max_clips == 0 {
            return Err(GpuCommandError::new(GpuCommandErrorCode::InvalidLimits));
        }
        self.max_clips = max_clips;
        Ok(self)
    }

    /// Replaces the positive glyph count ceiling for one atlas batch.
    pub const fn with_max_glyphs_per_batch(
        mut self,
        max_glyphs_per_batch: usize,
    ) -> Result<Self, GpuCommandError> {
        if max_glyphs_per_batch == 0 {
            return Err(GpuCommandError::new(GpuCommandErrorCode::InvalidLimits));
        }
        self.max_glyphs_per_batch = max_glyphs_per_batch;
        Ok(self)
    }
}

/// Geometry retained by one immutable complex clip node.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GpuClipGeometry {
    /// One logical rectangle.
    Rect(Rect),
    /// One command-buffer-local path and its containment rule.
    Path {
        /// Path resource local to the command buffer.
        path: GpuPathId,
        /// Containment rule used by the clip.
        rule: FillRule,
    },
}

/// One immutable node in a command buffer's persistent complex-clip chain.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GpuClipNode {
    parent: Option<GpuClipId>,
    geometry: GpuClipGeometry,
    op: ClipOp,
    transform: Transform,
}

impl GpuClipNode {
    /// Returns the preceding complex clip node, if any.
    pub const fn parent(self) -> Option<GpuClipId> {
        self.parent
    }

    /// Returns the logical geometry retained by this node.
    pub const fn geometry(self) -> GpuClipGeometry {
        self.geometry
    }

    /// Returns the boolean operation applied by this node.
    pub const fn op(self) -> ClipOp {
        self.op
    }

    /// Returns the logical-to-target transform captured for this geometry.
    pub const fn transform(self) -> Transform {
        self.transform
    }
}

/// One backend-neutral GPU drawing command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GpuCommand {
    /// Clears the full render target, without inheriting prior state.
    Clear(Color),
    /// Begins one isolated full-surface layer.
    SaveLayer {
        /// Restore-time layer policy.
        options: SaveLayerOptions,
        /// Logical-to-target transform selected when the layer was recorded.
        transform: Transform,
        /// Target-space scissor rectangle active at the layer boundary.
        scissor: Option<Rect>,
        /// Tail of the immutable complex-clip chain active at the layer boundary.
        clip: Option<GpuClipId>,
    },
    /// Restores and composites the most recent isolated layer.
    RestoreLayer,
    /// Fills one axis-aligned logical rectangle.
    FillRect {
        /// Logical rectangle to fill.
        rect: Rect,
        /// Immutable source paint.
        paint: Paint,
        /// Logical-to-target transform selected when the command was recorded.
        transform: Transform,
        /// Target-space scissor rectangle, if active.
        scissor: Option<Rect>,
        /// Tail of the immutable complex-clip chain, if active.
        clip: Option<GpuClipId>,
    },
    /// Fills one registered vector path.
    FillPath {
        /// Path resource local to this command buffer.
        path: GpuPathId,
        /// Containment rule for the path's contours.
        rule: FillRule,
        /// Immutable source paint.
        paint: Paint,
        /// Logical-to-target transform selected when the command was recorded.
        transform: Transform,
        /// Target-space scissor rectangle, if active.
        scissor: Option<Rect>,
        /// Tail of the immutable complex-clip chain, if active.
        clip: Option<GpuClipId>,
    },
    /// Strokes one registered vector path.
    StrokePath {
        /// Path resource local to this command buffer.
        path: GpuPathId,
        /// Cap, join, miter, width, and dash geometry.
        options: StrokeOptions,
        /// Immutable source paint.
        paint: Paint,
        /// Logical-to-target transform selected when the command was recorded.
        transform: Transform,
        /// Target-space scissor rectangle, if active.
        scissor: Option<Rect>,
        /// Tail of the immutable complex-clip chain, if active.
        clip: Option<GpuClipId>,
    },
    /// Draws one registered image into a logical rectangle.
    DrawImage {
        /// Image resource local to this command buffer.
        image: GpuImageId,
        /// Logical destination rectangle.
        destination: Rect,
        /// Additional straight-alpha opacity multiplier.
        opacity: u8,
        /// Reconstruction filter and edge behavior.
        sampling: SamplingOptions,
        /// Alpha, color filter, and compositing state for the source image.
        paint: Paint,
        /// Logical-to-target transform selected when the command was recorded.
        transform: Transform,
        /// Target-space scissor rectangle, if active.
        scissor: Option<Rect>,
        /// Tail of the immutable complex-clip chain, if active.
        clip: Option<GpuClipId>,
    },
    /// Draws one atlas-backed batch of positioned glyph quads.
    DrawGlyphs {
        /// Glyph atlas resource local to this command buffer.
        atlas: GpuGlyphAtlasId,
        /// Positioned quads submitted in visual draw order.
        glyphs: Vec<GpuGlyphQuad>,
        /// Mask color, color-glyph opacity, and compositing mode.
        paint: Paint,
        /// Logical-to-target transform selected when the command was recorded.
        transform: Transform,
        /// Target-space scissor rectangle, if active.
        scissor: Option<Rect>,
        /// Tail of the immutable complex-clip chain, if active.
        clip: Option<GpuClipId>,
    },
}

/// Immutable, ordered GPU command buffer with locally owned resources.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GpuCommandBuffer {
    commands: Vec<GpuCommand>,
    clips: Vec<GpuClipNode>,
    paths: Vec<Path>,
    images: Vec<Image>,
    glyph_atlases: Vec<GpuGlyphAtlas>,
}

impl GpuCommandBuffer {
    /// Borrows commands in submission order.
    pub fn commands(&self) -> &[GpuCommand] {
        &self.commands
    }

    /// Resolves an immutable complex clip node referenced by a command.
    pub fn clip_node(&self, id: GpuClipId) -> Option<GpuClipNode> {
        self.clips.get(usize::try_from(id.0).ok()?).copied()
    }

    /// Resolves a path resource referenced by a command in this buffer.
    pub fn path(&self, id: GpuPathId) -> Option<&Path> {
        self.paths.get(usize::try_from(id.0).ok()?)
    }

    /// Resolves an image resource referenced by a command in this buffer.
    pub fn image(&self, id: GpuImageId) -> Option<&Image> {
        self.images.get(usize::try_from(id.0).ok()?)
    }

    /// Resolves a glyph atlas referenced by a command in this buffer.
    pub fn glyph_atlas(&self, id: GpuGlyphAtlasId) -> Option<&GpuGlyphAtlas> {
        self.glyph_atlases.get(usize::try_from(id.0).ok()?)
    }
}

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
        if self.path(path).is_none() {
            return Err(GpuCommandError::new(GpuCommandErrorCode::InvalidResource));
        }
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
            ClipState::Unbounded => None,
            ClipState::Empty => None,
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

fn map_axis_aligned_rect(transform: Transform, rect: Rect) -> Result<Rect, GpuCommandError> {
    let first = transform
        .map_point(Point::new(rect.left(), rect.top()))
        .map_err(|_| GpuCommandError::new(GpuCommandErrorCode::NumericOverflow))?;
    let second = transform
        .map_point(Point::new(rect.right(), rect.bottom()))
        .map_err(|_| GpuCommandError::new(GpuCommandErrorCode::NumericOverflow))?;
    let left = first.x().min(second.x());
    let top = first.y().min(second.y());
    let right = first.x().max(second.x());
    let bottom = first.y().max(second.y());
    Rect::new(left, top, right, bottom)
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

/// Backend-specific error contract for GPU device operations.
pub trait GpuBackendError: std::error::Error + Send + Sync + 'static {}

impl<T> GpuBackendError for T where T: std::error::Error + Send + Sync + 'static {}

/// Platform-specific implementation of GPU surface allocation and submission.
///
/// Backends must validate device limits, resource ownership, and command support
/// before submission. This crate does not make a GPU backend authoritative for
/// the CPU reference rasterizer's pixels.
pub trait GpuBackend {
    /// Opaque backend-owned surface or texture target.
    type Surface;
    /// Backend-specific, source-redacted operational failure.
    type Error: GpuBackendError;

    /// Allocates one backend-owned render target.
    fn create_surface(
        &mut self,
        descriptor: GpuSurfaceDescriptor,
    ) -> Result<Self::Surface, Self::Error>;

    /// Submits one immutable command buffer to an existing target.
    fn submit(
        &mut self,
        surface: &mut Self::Surface,
        commands: &GpuCommandBuffer,
    ) -> Result<(), Self::Error>;
}
