//! Backend-neutral GPU submission contracts for `skia`.
//!
//! This crate deliberately contains no Metal, Vulkan, OpenGL, WebGPU, window,
//! thread, or foreign-function binding. Product-specific backend crates own
//! those details and implement [`GpuBackend`]. The command buffer is reusable
//! by renderers, editors, and other clients without coupling them to a
//! particular graphics API.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

/// Deterministic CPU replay backend for GPU command conformance tests.
#[cfg(feature = "software")]
pub mod software;

use std::{collections::HashMap, fmt};

use skia_core::{
    BlendMode, Color, FillRule, FontCollection, FontId, GlyphBitmap, GlyphBitmapFormat, GlyphId,
    GlyphRun, Paint, Path, Point, Rect, Scalar, TextError, TextErrorCode, TextLayout, Transform,
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

/// Opaque, command-buffer-local identifier for one immutable RGBA8 image.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GpuImageId(u32);

/// Opaque, command-buffer-local identifier for one immutable glyph atlas.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GpuGlyphAtlasId(u32);

/// Stable identity of one rasterized glyph inside a GPU atlas.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GpuGlyphKey {
    font: FontId,
    glyph: GlyphId,
    font_size_bits: i32,
    format: GlyphBitmapFormat,
}

impl GpuGlyphKey {
    /// Creates a cache key from one validated glyph bitmap.
    pub const fn from_bitmap(bitmap: &GlyphBitmap) -> Self {
        Self {
            font: bitmap.font(),
            glyph: bitmap.glyph(),
            font_size_bits: bitmap.font_size_bits(),
            format: bitmap.format(),
        }
    }

    /// Returns the immutable font-instance identity.
    pub const fn font(self) -> FontId {
        self.font
    }

    /// Returns the font-local glyph identity.
    pub const fn glyph(self) -> GlyphId {
        self.glyph
    }

    /// Returns the Q16.16 raster size.
    pub const fn font_size_bits(self) -> i32 {
        self.font_size_bits
    }

    /// Returns the atlas sample interpretation.
    pub const fn format(self) -> GlyphBitmapFormat {
        self.format
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

/// Packed metadata for one glyph bitmap in an atlas.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GpuGlyphAtlasEntry {
    key: GpuGlyphKey,
    source: GpuAtlasRect,
    left: i32,
    top: i32,
}

impl GpuGlyphAtlasEntry {
    /// Returns the glyph cache key.
    pub const fn key(self) -> GpuGlyphKey {
        self.key
    }

    /// Returns the atlas pixel rectangle.
    pub const fn source(self) -> GpuAtlasRect {
        self.source
    }

    /// Returns the raster placement offset right from the glyph origin.
    pub const fn left(self) -> i32 {
        self.left
    }

    /// Returns the raster placement offset above the glyph baseline.
    pub const fn top(self) -> i32 {
        self.top
    }
}

/// Immutable RGBA8 glyph atlas and its cache-key index.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GpuGlyphAtlas {
    image: Image,
    entries: HashMap<GpuGlyphKey, GpuGlyphAtlasEntry>,
}

impl GpuGlyphAtlas {
    /// Wraps one prepacked RGBA8 image for low-level glyph batch recording.
    ///
    /// The returned atlas has no cache-key index; use [`GpuGlyphAtlasBuilder`]
    /// when text-layout lookup is required.
    pub fn from_image(image: Image) -> Self {
        Self {
            image,
            entries: HashMap::new(),
        }
    }

    /// Borrows the upload-ready straight-alpha RGBA8 atlas image.
    pub const fn image(&self) -> &Image {
        &self.image
    }

    /// Resolves one exact glyph cache key.
    pub fn entry(&self, key: GpuGlyphKey) -> Option<GpuGlyphAtlasEntry> {
        self.entries.get(&key).copied()
    }

    fn glyph_entry(
        &self,
        font: FontId,
        glyph: GlyphId,
        font_size_bits: i32,
    ) -> Option<GpuGlyphAtlasEntry> {
        [GlyphBitmapFormat::Alpha8, GlyphBitmapFormat::Rgba8]
            .into_iter()
            .find_map(|format| {
                self.entry(GpuGlyphKey {
                    font,
                    glyph,
                    font_size_bits,
                    format,
                })
            })
    }
}

/// Bounded shelf packer for reusable GPU glyph atlases.
#[derive(Debug)]
pub struct GpuGlyphAtlasBuilder {
    width: u32,
    height: u32,
    max_glyphs: usize,
    pixels: Vec<u8>,
    entries: HashMap<GpuGlyphKey, GpuGlyphAtlasEntry>,
    cursor_x: u32,
    cursor_y: u32,
    row_height: u32,
}

impl GpuGlyphAtlasBuilder {
    /// Allocates one transparent, bounded RGBA8 atlas.
    pub fn new(width: u32, height: u32, max_glyphs: usize) -> Result<Self, GpuCommandError> {
        if width == 0 || height == 0 || max_glyphs == 0 {
            return Err(GpuCommandError::new(GpuCommandErrorCode::InvalidLimits));
        }
        let length = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|value| value.checked_mul(4))
            .and_then(|value| usize::try_from(value).ok())
            .ok_or(GpuCommandError::new(GpuCommandErrorCode::ResourceLimit))?;
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(length)
            .map_err(|_| GpuCommandError::new(GpuCommandErrorCode::AllocationFailed))?;
        pixels.resize(length, 0);
        Ok(Self {
            width,
            height,
            max_glyphs,
            pixels,
            entries: HashMap::new(),
            cursor_x: 1,
            cursor_y: 1,
            row_height: 0,
        })
    }

    /// Inserts or reuses one exact glyph bitmap.
    pub fn insert(&mut self, bitmap: &GlyphBitmap) -> Result<GpuGlyphAtlasEntry, GpuCommandError> {
        let key = GpuGlyphKey::from_bitmap(bitmap);
        if let Some(entry) = self.entries.get(&key) {
            return Ok(*entry);
        }
        if self.entries.len() == self.max_glyphs || bitmap.width() == 0 || bitmap.height() == 0 {
            return Err(GpuCommandError::new(GpuCommandErrorCode::ResourceLimit));
        }
        let padded_width = bitmap
            .width()
            .checked_add(2)
            .ok_or(GpuCommandError::new(GpuCommandErrorCode::NumericOverflow))?;
        let padded_height = bitmap
            .height()
            .checked_add(2)
            .ok_or(GpuCommandError::new(GpuCommandErrorCode::NumericOverflow))?;
        if padded_width > self.width || padded_height > self.height {
            return Err(GpuCommandError::new(GpuCommandErrorCode::ResourceLimit));
        }
        if self
            .cursor_x
            .checked_add(bitmap.width())
            .and_then(|value| value.checked_add(1))
            .is_none_or(|value| value > self.width)
        {
            self.cursor_x = 1;
            self.cursor_y = self
                .cursor_y
                .checked_add(self.row_height)
                .and_then(|value| value.checked_add(1))
                .ok_or(GpuCommandError::new(GpuCommandErrorCode::NumericOverflow))?;
            self.row_height = 0;
        }
        if self
            .cursor_y
            .checked_add(bitmap.height())
            .and_then(|value| value.checked_add(1))
            .is_none_or(|value| value > self.height)
        {
            return Err(GpuCommandError::new(GpuCommandErrorCode::ResourceLimit));
        }
        let source = GpuAtlasRect {
            x: self.cursor_x,
            y: self.cursor_y,
            width: bitmap.width(),
            height: bitmap.height(),
        };
        self.copy_bitmap(source, bitmap)?;
        let entry = GpuGlyphAtlasEntry {
            key,
            source,
            left: bitmap.left(),
            top: bitmap.top(),
        };
        self.entries
            .try_reserve(1)
            .map_err(|_| GpuCommandError::new(GpuCommandErrorCode::AllocationFailed))?;
        self.entries.insert(key, entry);
        self.cursor_x = self
            .cursor_x
            .checked_add(bitmap.width())
            .and_then(|value| value.checked_add(1))
            .ok_or(GpuCommandError::new(GpuCommandErrorCode::NumericOverflow))?;
        self.row_height = self.row_height.max(bitmap.height());
        Ok(entry)
    }

    /// Rasterizes and inserts every drawable glyph referenced by a text layout.
    pub fn insert_text_layout(
        &mut self,
        layout: &TextLayout,
        fonts: &FontCollection,
    ) -> Result<(), GpuCommandError> {
        for line in layout.lines() {
            let Some(paragraph) = line.paragraph() else {
                continue;
            };
            for shaped in paragraph.runs() {
                let run = shaped.glyph_run();
                let face = fonts
                    .face(run.font())
                    .ok_or(GpuCommandError::new(GpuCommandErrorCode::InvalidResource))?;
                for glyph in run.glyphs() {
                    if let Some(bitmap) = face
                        .rasterize_glyph(glyph.glyph(), run.font_size_bits())
                        .map_err(map_text_error)?
                    {
                        self.insert(&bitmap)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Publishes the immutable atlas and cache-key index.
    pub fn finish(self) -> Result<GpuGlyphAtlas, GpuCommandError> {
        let image = Image::from_rgba8(self.width, self.height, self.pixels)
            .map_err(|_| GpuCommandError::new(GpuCommandErrorCode::InvalidResource))?;
        Ok(GpuGlyphAtlas {
            image,
            entries: self.entries,
        })
    }

    fn copy_bitmap(
        &mut self,
        destination: GpuAtlasRect,
        bitmap: &GlyphBitmap,
    ) -> Result<(), GpuCommandError> {
        let source_stride = usize::try_from(bitmap.width())
            .ok()
            .and_then(|value| value.checked_mul(bitmap.format().bytes_per_pixel()))
            .ok_or(GpuCommandError::new(GpuCommandErrorCode::NumericOverflow))?;
        let destination_stride = usize::try_from(self.width)
            .ok()
            .and_then(|value| value.checked_mul(4))
            .ok_or(GpuCommandError::new(GpuCommandErrorCode::NumericOverflow))?;
        let destination_row_bytes = usize::try_from(bitmap.width())
            .ok()
            .and_then(|value| value.checked_mul(4))
            .ok_or(GpuCommandError::new(GpuCommandErrorCode::NumericOverflow))?;
        for row in 0..bitmap.height() {
            let source_start = usize::try_from(row)
                .ok()
                .and_then(|value| value.checked_mul(source_stride))
                .ok_or(GpuCommandError::new(GpuCommandErrorCode::NumericOverflow))?;
            let source_end = source_start
                .checked_add(source_stride)
                .ok_or(GpuCommandError::new(GpuCommandErrorCode::NumericOverflow))?;
            let destination_y = destination
                .y
                .checked_add(row)
                .ok_or(GpuCommandError::new(GpuCommandErrorCode::NumericOverflow))?;
            let destination_start = usize::try_from(destination_y)
                .ok()
                .and_then(|value| value.checked_mul(destination_stride))
                .and_then(|value| {
                    usize::try_from(destination.x)
                        .ok()
                        .and_then(|x| x.checked_mul(4))
                        .and_then(|x| value.checked_add(x))
                })
                .ok_or(GpuCommandError::new(GpuCommandErrorCode::NumericOverflow))?;
            let destination_end = destination_start
                .checked_add(destination_row_bytes)
                .ok_or(GpuCommandError::new(GpuCommandErrorCode::NumericOverflow))?;
            match bitmap.format() {
                GlyphBitmapFormat::Alpha8 => {
                    let source = bitmap
                        .pixels()
                        .get(source_start..source_end)
                        .ok_or(GpuCommandError::new(GpuCommandErrorCode::InvalidResource))?;
                    let destination = self
                        .pixels
                        .get_mut(destination_start..destination_end)
                        .ok_or(GpuCommandError::new(GpuCommandErrorCode::InvalidResource))?;
                    for (alpha, pixel) in source.iter().zip(destination.chunks_exact_mut(4)) {
                        pixel.copy_from_slice(&[255, 255, 255, *alpha]);
                    }
                }
                GlyphBitmapFormat::Rgba8 => {
                    let source = bitmap
                        .pixels()
                        .get(source_start..source_end)
                        .ok_or(GpuCommandError::new(GpuCommandErrorCode::InvalidResource))?;
                    let destination = self
                        .pixels
                        .get_mut(destination_start..destination_end)
                        .ok_or(GpuCommandError::new(GpuCommandErrorCode::InvalidResource))?;
                    destination.copy_from_slice(source);
                }
            }
        }
        Ok(())
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
            max_save_depth,
            max_glyphs_per_batch: max_commands.saturating_mul(1_024),
        })
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

/// One backend-neutral GPU drawing command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GpuCommand {
    /// Clears the full render target, without inheriting prior state.
    Clear(Color),
    /// Fills one axis-aligned logical rectangle.
    FillRect {
        /// Logical rectangle to fill.
        rect: Rect,
        /// Immutable source paint.
        paint: Paint,
        /// Logical-to-target transform selected when the command was recorded.
        transform: Transform,
        /// Target-space scissor rectangle, if clipping is active.
        clip: Option<Rect>,
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
        /// Target-space scissor rectangle, if clipping is active.
        clip: Option<Rect>,
    },
    /// Draws one registered image into a logical rectangle.
    DrawImage {
        /// Image resource local to this command buffer.
        image: GpuImageId,
        /// Logical destination rectangle.
        destination: Rect,
        /// Additional straight-alpha opacity multiplier.
        opacity: u8,
        /// Compositing operation for the source image.
        blend_mode: BlendMode,
        /// Logical-to-target transform selected when the command was recorded.
        transform: Transform,
        /// Target-space scissor rectangle, if clipping is active.
        clip: Option<Rect>,
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
        /// Target-space scissor rectangle, if clipping is active.
        clip: Option<Rect>,
    },
}

/// Immutable, ordered GPU command buffer with locally owned resources.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GpuCommandBuffer {
    commands: Vec<GpuCommand>,
    paths: Vec<Path>,
    images: Vec<Image>,
    glyph_atlases: Vec<GpuGlyphAtlas>,
}

impl GpuCommandBuffer {
    /// Borrows commands in submission order.
    pub fn commands(&self) -> &[GpuCommand] {
        &self.commands
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
    paths: Vec<Path>,
    images: Vec<Image>,
    glyph_atlases: Vec<GpuGlyphAtlas>,
    limits: GpuCommandLimits,
    state: GpuState,
    saves: Vec<GpuState>,
}

#[derive(Clone, Copy, Debug)]
struct GpuState {
    transform: Transform,
    clip: ClipState,
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
            paths: Vec::new(),
            images: Vec::new(),
            glyph_atlases: Vec::new(),
            limits,
            state: GpuState {
                transform: Transform::IDENTITY,
                clip: ClipState::Unbounded,
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
        if self.saves.len() == self.limits.max_save_depth {
            return Err(GpuCommandError::new(GpuCommandErrorCode::ResourceLimit));
        }
        self.saves
            .try_reserve(1)
            .map_err(|_| GpuCommandError::new(GpuCommandErrorCode::AllocationFailed))?;
        self.saves.push(self.state);
        Ok(())
    }

    /// Restores the most recently saved transform and target-space scissor state.
    pub fn restore(&mut self) -> Result<(), GpuCommandError> {
        self.state = self
            .saves
            .pop()
            .ok_or(GpuCommandError::new(GpuCommandErrorCode::RestoreUnderflow))?;
        Ok(())
    }

    /// Intersects the active target-space scissor with one axis-aligned transformed rectangle.
    pub fn clip_rect(&mut self, rect: Rect) -> Result<(), GpuCommandError> {
        if !self.state.transform.is_axis_aligned() {
            return Err(GpuCommandError::new(
                GpuCommandErrorCode::UnsupportedTransform,
            ));
        }
        let clip = map_axis_aligned_rect(self.state.transform, rect)?;
        self.state.clip = match self.state.clip {
            ClipState::Unbounded => ClipState::Rect(clip),
            ClipState::Empty => ClipState::Empty,
            ClipState::Rect(current) => intersect_rect(current, clip)
                .map(ClipState::Rect)
                .unwrap_or(ClipState::Empty),
        };
        Ok(())
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
            clip: self.clip(),
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
            clip: self.clip(),
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
        if self.image(image).is_none() {
            return Err(GpuCommandError::new(GpuCommandErrorCode::InvalidResource));
        }
        self.push(GpuCommand::DrawImage {
            image,
            destination,
            opacity,
            blend_mode,
            transform: self.state.transform,
            clip: self.clip(),
        })
    }

    /// Records all raster glyphs in one text layout as one atlas-backed batch.
    ///
    /// Empty glyphs and glyphs absent from the supplied atlas are skipped.
    /// Text decorations remain ordinary rectangle commands so backends can
    /// batch them with other solid geometry independently.
    pub fn draw_text_layout_glyphs(
        &mut self,
        atlas: GpuGlyphAtlasId,
        layout: &TextLayout,
        origin: Point,
        paint: Paint,
    ) -> Result<(), GpuCommandError> {
        let atlas_resource = self
            .glyph_atlas(atlas)
            .ok_or(GpuCommandError::new(GpuCommandErrorCode::InvalidResource))?;
        let mut glyphs = Vec::new();
        for line in layout.lines() {
            let Some(paragraph) = line.paragraph() else {
                continue;
            };
            let line_x = origin
                .x()
                .bits()
                .checked_add(line.offset_x_bits())
                .ok_or(GpuCommandError::new(GpuCommandErrorCode::NumericOverflow))?;
            let baseline_y = origin
                .y()
                .bits()
                .checked_add(line.baseline_y_bits())
                .ok_or(GpuCommandError::new(GpuCommandErrorCode::NumericOverflow))?;
            for shaped in paragraph.runs() {
                let run = shaped.glyph_run();
                if shaped.glyph_offsets_x_bits().len() != run.glyphs().len() {
                    return Err(GpuCommandError::new(GpuCommandErrorCode::InvalidResource));
                }
                let run_x = line_x
                    .checked_add(shaped.origin_x_bits())
                    .ok_or(GpuCommandError::new(GpuCommandErrorCode::NumericOverflow))?;
                for (glyph, offset_x) in run.glyphs().iter().zip(shaped.glyph_offsets_x_bits()) {
                    let Some(entry) =
                        atlas_resource.glyph_entry(run.font(), glyph.glyph(), run.font_size_bits())
                    else {
                        continue;
                    };
                    if glyphs.len() == self.limits.max_glyphs_per_batch {
                        return Err(GpuCommandError::new(GpuCommandErrorCode::ResourceLimit));
                    }
                    let glyph_x = scaled_glyph_coordinate_bits(glyph.x().bits(), run)?;
                    let glyph_y = scaled_glyph_coordinate_bits(glyph.y().bits(), run)?;
                    let bitmap_left = pixel_bits(entry.left())?;
                    let bitmap_top = pixel_bits(entry.top())?;
                    let left = run_x
                        .checked_add(*offset_x)
                        .and_then(|value| value.checked_add(glyph_x))
                        .and_then(|value| value.checked_add(bitmap_left))
                        .ok_or(GpuCommandError::new(GpuCommandErrorCode::NumericOverflow))?;
                    let top = baseline_y
                        .checked_add(glyph_y)
                        .and_then(|value| value.checked_sub(bitmap_top))
                        .ok_or(GpuCommandError::new(GpuCommandErrorCode::NumericOverflow))?;
                    let right = left
                        .checked_add(pixel_bits_u32(entry.source().width())?)
                        .ok_or(GpuCommandError::new(GpuCommandErrorCode::NumericOverflow))?;
                    let bottom = top
                        .checked_add(pixel_bits_u32(entry.source().height())?)
                        .ok_or(GpuCommandError::new(GpuCommandErrorCode::NumericOverflow))?;
                    let destination = Rect::new(
                        Scalar::from_bits(left),
                        Scalar::from_bits(top),
                        Scalar::from_bits(right),
                        Scalar::from_bits(bottom),
                    )
                    .map_err(|_| GpuCommandError::new(GpuCommandErrorCode::NumericOverflow))?;
                    glyphs
                        .try_reserve(1)
                        .map_err(|_| GpuCommandError::new(GpuCommandErrorCode::AllocationFailed))?;
                    glyphs.push(GpuGlyphQuad {
                        source: entry.source(),
                        destination,
                        mask: entry.key().format() == GlyphBitmapFormat::Alpha8,
                    });
                }
            }
        }
        self.draw_glyph_batch(atlas, glyphs, paint)
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
            clip: self.clip(),
        })
    }

    /// Publishes commands and their owned resources for later submission.
    pub fn finish(self) -> GpuCommandBuffer {
        GpuCommandBuffer {
            commands: self.commands,
            paths: self.paths,
            images: self.images,
            glyph_atlases: self.glyph_atlases,
        }
    }

    fn push(&mut self, command: GpuCommand) -> Result<(), GpuCommandError> {
        if matches!(self.state.clip, ClipState::Empty) {
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

    fn clip(&self) -> Option<Rect> {
        match self.state.clip {
            ClipState::Unbounded => None,
            ClipState::Empty => None,
            ClipState::Rect(rect) => Some(rect),
        }
    }
}

fn resource_id(length: usize, max_resources: usize) -> Result<u32, GpuCommandError> {
    if length == max_resources {
        return Err(GpuCommandError::new(GpuCommandErrorCode::ResourceLimit));
    }
    u32::try_from(length).map_err(|_| GpuCommandError::new(GpuCommandErrorCode::ResourceLimit))
}

fn map_text_error(error: TextError) -> GpuCommandError {
    let code = match error.code() {
        TextErrorCode::AllocationFailed => GpuCommandErrorCode::AllocationFailed,
        TextErrorCode::NumericOverflow => GpuCommandErrorCode::NumericOverflow,
        TextErrorCode::ResourceLimit => GpuCommandErrorCode::ResourceLimit,
        _ => GpuCommandErrorCode::InvalidResource,
    };
    GpuCommandError::new(code)
}

fn scaled_glyph_coordinate_bits(design_bits: i32, run: &GlyphRun) -> Result<i32, GpuCommandError> {
    let numerator = i128::from(design_bits)
        .checked_mul(i128::from(run.font_size_bits()))
        .ok_or(GpuCommandError::new(GpuCommandErrorCode::NumericOverflow))?;
    let denominator = i128::from(64_i32)
        .checked_mul(i128::from(run.units_per_em()))
        .ok_or(GpuCommandError::new(GpuCommandErrorCode::NumericOverflow))?;
    let rounded = if numerator >= 0 {
        numerator
            .checked_add(denominator / 2)
            .ok_or(GpuCommandError::new(GpuCommandErrorCode::NumericOverflow))?
            / denominator
    } else {
        -((-numerator
            .checked_add(denominator / 2)
            .ok_or(GpuCommandError::new(GpuCommandErrorCode::NumericOverflow))?)
            / denominator)
    };
    i32::try_from(rounded).map_err(|_| GpuCommandError::new(GpuCommandErrorCode::NumericOverflow))
}

fn pixel_bits(value: i32) -> Result<i32, GpuCommandError> {
    value
        .checked_mul(1 << 16)
        .ok_or(GpuCommandError::new(GpuCommandErrorCode::NumericOverflow))
}

fn pixel_bits_u32(value: u32) -> Result<i32, GpuCommandError> {
    i32::try_from(value)
        .map_err(|_| GpuCommandError::new(GpuCommandErrorCode::NumericOverflow))
        .and_then(pixel_bits)
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
