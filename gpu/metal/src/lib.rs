//! macOS Metal submission backend for `skia-gpu`.
//!
//! This adapter creates native Metal textures and command buffers, executes
//! clears, and draws atlas-backed glyph batches through Metal shaders. Stable
//! atlas keys enable bounded native texture reuse across submissions. Other
//! drawing commands fail closed until their shader contracts are implemented.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::{collections::HashMap, fmt, mem::size_of};

use metal::{
    CommandQueue, Device, MTLBlendFactor, MTLClearColor, MTLCommandBufferStatus, MTLLoadAction,
    MTLOrigin, MTLPixelFormat, MTLPrimitiveType, MTLRegion, MTLResourceOptions, MTLScissorRect,
    MTLSize, MTLStorageMode, MTLStoreAction, MTLTextureUsage, RenderPassDescriptor,
    RenderPipelineDescriptor, RenderPipelineState, Texture, TextureDescriptor,
};
use skia_core::{BlendMode, Color, Point, Rect, Scalar, Transform};
use skia_gpu::{
    GpuBackend, GpuCommand, GpuCommandBuffer, GpuGlyphAtlas, GpuGlyphAtlasKey, GpuGlyphQuad,
    GpuSurfaceDescriptor,
};

const DEFAULT_ATLAS_CACHE_CAPACITY: usize = 8;
const DEFAULT_ATLAS_CACHE_BYTES: u64 = 64 * 1024 * 1024;

/// Stable machine-readable Metal backend failure.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MetalErrorCode {
    /// The machine did not expose a default Metal device.
    DeviceUnavailable,
    /// The adapter could not allocate a render-target texture.
    SurfaceAllocationFailed,
    /// The command buffer requested a not-yet-implemented GPU operation.
    UnsupportedCommand,
    /// Metal did not complete the submitted command buffer successfully.
    SubmissionFailed,
    /// The adapter could not allocate an RGBA8 readback buffer.
    ReadbackAllocationFailed,
    /// The compiled shared Metal shader library could not be loaded.
    ShaderLibraryFailed,
    /// The solid-rectangle render pipeline could not be created.
    PipelineCreationFailed,
}

/// Source-redacted Metal backend error.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MetalError {
    code: MetalErrorCode,
}

impl MetalError {
    const fn new(code: MetalErrorCode) -> Self {
        Self { code }
    }

    /// Returns the stable failure code.
    pub const fn code(self) -> MetalErrorCode {
        self.code
    }
}

impl fmt::Display for MetalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}", self.code)
    }
}

impl std::error::Error for MetalError {}

/// Native Metal render target owned by [`MetalBackend`].
pub struct MetalSurface {
    texture: Texture,
    descriptor: GpuSurfaceDescriptor,
}

impl MetalSurface {
    /// Returns the portable descriptor used to allocate this target.
    pub const fn descriptor(&self) -> GpuSurfaceDescriptor {
        self.descriptor
    }

    /// Reads the completed target as tightly packed row-major RGBA8 pixels.
    ///
    /// Call this only after a successful [`GpuBackend::submit`] operation.
    pub fn read_rgba8(&self) -> Result<Vec<u8>, MetalError> {
        let bytes = u64::from(self.descriptor.width())
            .checked_mul(u64::from(self.descriptor.height()))
            .and_then(|value| value.checked_mul(4))
            .ok_or(MetalError::new(MetalErrorCode::ReadbackAllocationFailed))?;
        let length = usize::try_from(bytes)
            .map_err(|_| MetalError::new(MetalErrorCode::ReadbackAllocationFailed))?;
        let mut output = Vec::new();
        output
            .try_reserve_exact(length)
            .map_err(|_| MetalError::new(MetalErrorCode::ReadbackAllocationFailed))?;
        output.resize(length, 0);
        self.texture.get_bytes(
            output.as_mut_ptr().cast(),
            u64::from(self.descriptor.width()) * 4,
            MTLRegion {
                origin: MTLOrigin { x: 0, y: 0, z: 0 },
                size: MTLSize {
                    width: u64::from(self.descriptor.width()),
                    height: u64::from(self.descriptor.height()),
                    depth: 1,
                },
            },
            0,
        );
        Ok(output)
    }
}

/// macOS Metal device and queue implementing the portable GPU contract.
pub struct MetalBackend {
    device: Device,
    queue: CommandQueue,
    solid_rect_pipeline: RenderPipelineState,
    glyph_pipeline: RenderPipelineState,
    atlas_cache_capacity: usize,
    atlas_cache_max_bytes: u64,
    atlas_cache_bytes: u64,
    atlas_cache: Vec<CachedAtlasTexture>,
    atlas_cache_clock: u64,
    atlas_cache_hits: u64,
    atlas_uploads: u64,
    atlas_evictions: u64,
}

/// Observable native glyph-atlas texture cache counters.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct MetalAtlasCacheStats {
    hits: u64,
    uploads: u64,
    evictions: u64,
    entries: usize,
    retained_bytes: u64,
}

impl MetalAtlasCacheStats {
    /// Returns cross-submission native texture reuse count.
    pub const fn hits(self) -> u64 {
        self.hits
    }

    /// Returns the number of atlas images uploaded to native textures.
    pub const fn uploads(self) -> u64 {
        self.uploads
    }

    /// Returns the number of cached native textures evicted or replaced.
    pub const fn evictions(self) -> u64 {
        self.evictions
    }

    /// Returns the current number of retained native atlas textures.
    pub const fn entries(self) -> usize {
        self.entries
    }

    /// Returns the RGBA8 byte size represented by retained native textures.
    pub const fn retained_bytes(self) -> u64 {
        self.retained_bytes
    }
}

impl MetalBackend {
    /// Opens the default system Metal device and one persistent command queue.
    pub fn new() -> Result<Self, MetalError> {
        let device =
            Device::system_default().ok_or(MetalError::new(MetalErrorCode::DeviceUnavailable))?;
        let queue = device.new_command_queue();
        let library = device
            .new_library_with_file(env!("SKIA_METAL_LIBRARY"))
            .map_err(|_| MetalError::new(MetalErrorCode::ShaderLibraryFailed))?;
        let vertex = library
            .get_function("skia_solid_rect_vertex", None)
            .map_err(|_| MetalError::new(MetalErrorCode::ShaderLibraryFailed))?;
        let fragment = library
            .get_function("skia_solid_rect_fragment", None)
            .map_err(|_| MetalError::new(MetalErrorCode::ShaderLibraryFailed))?;
        let descriptor = RenderPipelineDescriptor::new();
        descriptor.set_vertex_function(Some(&vertex));
        descriptor.set_fragment_function(Some(&fragment));
        let solid_attachment = descriptor
            .color_attachments()
            .object_at(0)
            .ok_or(MetalError::new(MetalErrorCode::PipelineCreationFailed))?;
        solid_attachment.set_pixel_format(MTLPixelFormat::RGBA8Unorm);
        solid_attachment.set_blending_enabled(true);
        solid_attachment.set_source_rgb_blend_factor(MTLBlendFactor::SourceAlpha);
        solid_attachment.set_destination_rgb_blend_factor(MTLBlendFactor::OneMinusSourceAlpha);
        solid_attachment.set_source_alpha_blend_factor(MTLBlendFactor::One);
        solid_attachment.set_destination_alpha_blend_factor(MTLBlendFactor::OneMinusSourceAlpha);
        let solid_rect_pipeline = device
            .new_render_pipeline_state(&descriptor)
            .map_err(|_| MetalError::new(MetalErrorCode::PipelineCreationFailed))?;
        let glyph_vertex = library
            .get_function("skia_glyph_vertex", None)
            .map_err(|_| MetalError::new(MetalErrorCode::ShaderLibraryFailed))?;
        let glyph_fragment = library
            .get_function("skia_glyph_fragment", None)
            .map_err(|_| MetalError::new(MetalErrorCode::ShaderLibraryFailed))?;
        let glyph_descriptor = RenderPipelineDescriptor::new();
        glyph_descriptor.set_vertex_function(Some(&glyph_vertex));
        glyph_descriptor.set_fragment_function(Some(&glyph_fragment));
        let glyph_attachment = glyph_descriptor
            .color_attachments()
            .object_at(0)
            .ok_or(MetalError::new(MetalErrorCode::PipelineCreationFailed))?;
        glyph_attachment.set_pixel_format(MTLPixelFormat::RGBA8Unorm);
        glyph_attachment.set_blending_enabled(true);
        glyph_attachment.set_source_rgb_blend_factor(MTLBlendFactor::SourceAlpha);
        glyph_attachment.set_destination_rgb_blend_factor(MTLBlendFactor::OneMinusSourceAlpha);
        glyph_attachment.set_source_alpha_blend_factor(MTLBlendFactor::One);
        glyph_attachment.set_destination_alpha_blend_factor(MTLBlendFactor::OneMinusSourceAlpha);
        let glyph_pipeline = device
            .new_render_pipeline_state(&glyph_descriptor)
            .map_err(|_| MetalError::new(MetalErrorCode::PipelineCreationFailed))?;
        Ok(Self {
            device,
            queue,
            solid_rect_pipeline,
            glyph_pipeline,
            atlas_cache_capacity: DEFAULT_ATLAS_CACHE_CAPACITY,
            atlas_cache_max_bytes: DEFAULT_ATLAS_CACHE_BYTES,
            atlas_cache_bytes: 0,
            atlas_cache: Vec::new(),
            atlas_cache_clock: 0,
            atlas_cache_hits: 0,
            atlas_uploads: 0,
            atlas_evictions: 0,
        })
    }

    /// Opens the default device with an explicit native atlas cache capacity.
    ///
    /// A zero capacity disables cross-submission texture retention.
    pub fn with_atlas_cache_capacity(capacity: usize) -> Result<Self, MetalError> {
        let mut backend = Self::new()?;
        backend.atlas_cache_capacity = capacity;
        Ok(backend)
    }

    /// Replaces the native atlas cache capacity and immediately evicts excess entries.
    pub fn set_atlas_cache_capacity(&mut self, capacity: usize) {
        self.atlas_cache_capacity = capacity;
        while self.atlas_cache.len() > capacity {
            self.evict_lru();
        }
    }

    /// Replaces the native atlas byte budget and immediately evicts excess entries.
    ///
    /// A zero budget disables cross-submission texture retention.
    pub fn set_atlas_cache_byte_limit(&mut self, max_bytes: u64) {
        self.atlas_cache_max_bytes = max_bytes;
        while self.atlas_cache_bytes > max_bytes {
            if !self.evict_lru() {
                break;
            }
        }
    }

    /// Returns native atlas texture reuse, upload, and eviction counters.
    pub const fn atlas_cache_stats(&self) -> MetalAtlasCacheStats {
        MetalAtlasCacheStats {
            hits: self.atlas_cache_hits,
            uploads: self.atlas_uploads,
            evictions: self.atlas_evictions,
            entries: self.atlas_cache.len(),
            retained_bytes: self.atlas_cache_bytes,
        }
    }
}

impl GpuBackend for MetalBackend {
    type Surface = MetalSurface;
    type Error = MetalError;

    fn create_surface(
        &mut self,
        descriptor: GpuSurfaceDescriptor,
    ) -> Result<Self::Surface, Self::Error> {
        let texture_descriptor = TextureDescriptor::new();
        texture_descriptor.set_width(u64::from(descriptor.width()));
        texture_descriptor.set_height(u64::from(descriptor.height()));
        texture_descriptor.set_pixel_format(MTLPixelFormat::RGBA8Unorm);
        texture_descriptor.set_usage(MTLTextureUsage::RenderTarget);
        texture_descriptor.set_storage_mode(MTLStorageMode::Managed);
        let texture = self.device.new_texture(&texture_descriptor);
        Ok(MetalSurface {
            texture,
            descriptor,
        })
    }

    fn submit(
        &mut self,
        surface: &mut Self::Surface,
        commands: &GpuCommandBuffer,
    ) -> Result<(), Self::Error> {
        for command in commands.commands() {
            match command {
                GpuCommand::Clear(_) => {}
                GpuCommand::FillRect { paint, .. }
                    if paint.blend_mode() == BlendMode::SourceOver => {}
                GpuCommand::DrawGlyphs { atlas, paint, .. } => {
                    if paint.blend_mode() != BlendMode::SourceOver
                        || commands.glyph_atlas(*atlas).is_none()
                    {
                        return Err(MetalError::new(MetalErrorCode::UnsupportedCommand));
                    }
                }
                GpuCommand::FillRect { .. }
                | GpuCommand::FillPath { .. }
                | GpuCommand::DrawImage { .. } => {
                    return Err(MetalError::new(MetalErrorCode::UnsupportedCommand));
                }
            }
        }
        if commands.commands().is_empty() {
            return Ok(());
        }

        let mut atlas_textures = HashMap::new();
        for command in commands.commands() {
            if let GpuCommand::DrawGlyphs { atlas, .. } = command
                && !atlas_textures.contains_key(atlas)
            {
                let atlas_resource = commands
                    .glyph_atlas(*atlas)
                    .ok_or(MetalError::new(MetalErrorCode::UnsupportedCommand))?;
                atlas_textures.insert(*atlas, self.atlas_texture(atlas_resource)?);
            }
        }
        let command_buffer = self.queue.new_command_buffer();
        for command in commands.commands() {
            match command {
                GpuCommand::Clear(color) => {
                    encode_clear(command_buffer, surface, *color)?;
                }
                GpuCommand::FillRect {
                    rect,
                    paint,
                    transform,
                    clip,
                } => {
                    self.encode_solid_rect(
                        command_buffer,
                        surface,
                        *rect,
                        *paint,
                        *transform,
                        *clip,
                    )?;
                }
                GpuCommand::DrawGlyphs {
                    atlas,
                    glyphs,
                    paint,
                    transform,
                    clip,
                } => {
                    let texture = atlas_textures
                        .get(atlas)
                        .ok_or(MetalError::new(MetalErrorCode::SubmissionFailed))?;
                    self.encode_glyphs(
                        command_buffer,
                        surface,
                        texture,
                        glyphs,
                        *paint,
                        *transform,
                        *clip,
                    )?;
                }
                GpuCommand::FillPath { .. } | GpuCommand::DrawImage { .. } => {
                    unreachable!("commands were prevalidated")
                }
            }
        }
        let blit = command_buffer.new_blit_command_encoder();
        blit.synchronize_resource(&surface.texture);
        blit.end_encoding();
        command_buffer.commit();
        command_buffer.wait_until_completed();
        if command_buffer.status() != MTLCommandBufferStatus::Completed {
            return Err(MetalError::new(MetalErrorCode::SubmissionFailed));
        }
        Ok(())
    }
}

impl MetalBackend {
    #[allow(clippy::too_many_arguments)]
    fn encode_solid_rect(
        &self,
        command_buffer: &metal::CommandBufferRef,
        surface: &MetalSurface,
        rect: Rect,
        paint: skia_core::Paint,
        transform: Transform,
        clip: Option<Rect>,
    ) -> Result<(), MetalError> {
        let Some(scissor) = scissor_rect(clip, surface.descriptor) else {
            return Ok(());
        };
        let vertices = solid_rect_vertices(rect, transform)?;
        let byte_length = u64::try_from(size_of_val(&vertices))
            .map_err(|_| MetalError::new(MetalErrorCode::SubmissionFailed))?;
        let vertex_buffer = self.device.new_buffer_with_data(
            vertices.as_ptr().cast(),
            byte_length,
            MTLResourceOptions::CPUCacheModeDefaultCache,
        );
        let descriptor = render_pass(surface, MTLLoadAction::Load)?;
        let encoder = command_buffer.new_render_command_encoder(descriptor);
        encoder.set_render_pipeline_state(&self.solid_rect_pipeline);
        encoder.set_scissor_rect(scissor);
        encoder.set_vertex_buffer(0, Some(&vertex_buffer), 0);
        let viewport = viewport_size(surface.descriptor);
        encoder.set_vertex_bytes(1, size_of_val(&viewport) as u64, viewport.as_ptr().cast());
        let color = paint_color(paint.color());
        encoder.set_fragment_bytes(0, size_of_val(&color) as u64, color.as_ptr().cast());
        encoder.draw_primitives(MTLPrimitiveType::Triangle, 0, vertices.len() as u64);
        encoder.end_encoding();
        Ok(())
    }

    fn atlas_texture(&mut self, atlas: &GpuGlyphAtlas) -> Result<Texture, MetalError> {
        let now = self
            .atlas_cache_clock
            .checked_add(1)
            .ok_or(MetalError::new(MetalErrorCode::SubmissionFailed))?;
        self.atlas_cache_clock = now;
        let Some(cache_key) = atlas.cache_key() else {
            self.atlas_uploads = self
                .atlas_uploads
                .checked_add(1)
                .ok_or(MetalError::new(MetalErrorCode::SubmissionFailed))?;
            return Ok(self.upload_atlas(atlas));
        };
        let atlas_bytes = u64::try_from(atlas.image().pixels().len())
            .map_err(|_| MetalError::new(MetalErrorCode::SubmissionFailed))?;
        if self.atlas_cache_capacity == 0
            || self.atlas_cache_max_bytes == 0
            || atlas_bytes > self.atlas_cache_max_bytes
        {
            self.atlas_uploads = self
                .atlas_uploads
                .checked_add(1)
                .ok_or(MetalError::new(MetalErrorCode::SubmissionFailed))?;
            return Ok(self.upload_atlas(atlas));
        }

        if let Some(index) = self
            .atlas_cache
            .iter()
            .position(|entry| entry.key == cache_key && entry.atlas.image() == atlas.image())
        {
            self.atlas_cache_hits = self
                .atlas_cache_hits
                .checked_add(1)
                .ok_or(MetalError::new(MetalErrorCode::SubmissionFailed))?;
            self.atlas_cache[index].last_used = now;
            return Ok(self.atlas_cache[index].texture.clone());
        }

        if let Some(index) = self
            .atlas_cache
            .iter()
            .position(|entry| entry.key == cache_key)
        {
            let removed = self.atlas_cache.remove(index);
            self.atlas_cache_bytes = self.atlas_cache_bytes.saturating_sub(removed.bytes);
            self.atlas_evictions = self
                .atlas_evictions
                .checked_add(1)
                .ok_or(MetalError::new(MetalErrorCode::SubmissionFailed))?;
        }
        while self.atlas_cache.len() == self.atlas_cache_capacity
            || self
                .atlas_cache_bytes
                .checked_add(atlas_bytes)
                .is_none_or(|bytes| bytes > self.atlas_cache_max_bytes)
        {
            if !self.evict_lru() {
                break;
            }
        }
        let texture = self.upload_atlas(atlas);
        self.atlas_uploads = self
            .atlas_uploads
            .checked_add(1)
            .ok_or(MetalError::new(MetalErrorCode::SubmissionFailed))?;
        self.atlas_cache
            .try_reserve(1)
            .map_err(|_| MetalError::new(MetalErrorCode::SubmissionFailed))?;
        self.atlas_cache.push(CachedAtlasTexture {
            key: cache_key,
            atlas: atlas.clone(),
            texture: texture.clone(),
            last_used: now,
            bytes: atlas_bytes,
        });
        self.atlas_cache_bytes = self
            .atlas_cache_bytes
            .checked_add(atlas_bytes)
            .ok_or(MetalError::new(MetalErrorCode::SubmissionFailed))?;
        Ok(texture)
    }

    fn evict_lru(&mut self) -> bool {
        let Some(index) = self
            .atlas_cache
            .iter()
            .enumerate()
            .min_by_key(|(_, entry)| entry.last_used)
            .map(|(index, _)| index)
        else {
            return false;
        };
        let removed = self.atlas_cache.remove(index);
        self.atlas_cache_bytes = self.atlas_cache_bytes.saturating_sub(removed.bytes);
        self.atlas_evictions = self.atlas_evictions.saturating_add(1);
        true
    }

    fn upload_atlas(&self, atlas: &GpuGlyphAtlas) -> Texture {
        let image = atlas.image();
        let descriptor = TextureDescriptor::new();
        descriptor.set_width(u64::from(image.width()));
        descriptor.set_height(u64::from(image.height()));
        descriptor.set_pixel_format(MTLPixelFormat::RGBA8Unorm);
        descriptor.set_usage(MTLTextureUsage::ShaderRead);
        descriptor.set_storage_mode(MTLStorageMode::Managed);
        let texture = self.device.new_texture(&descriptor);
        texture.replace_region(
            MTLRegion {
                origin: MTLOrigin { x: 0, y: 0, z: 0 },
                size: MTLSize {
                    width: u64::from(image.width()),
                    height: u64::from(image.height()),
                    depth: 1,
                },
            },
            0,
            image.pixels().as_ptr().cast(),
            u64::from(image.width()) * 4,
        );
        texture
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_glyphs(
        &self,
        command_buffer: &metal::CommandBufferRef,
        surface: &MetalSurface,
        atlas: &Texture,
        glyphs: &[GpuGlyphQuad],
        paint: skia_core::Paint,
        transform: Transform,
        clip: Option<Rect>,
    ) -> Result<(), MetalError> {
        let Some(scissor) = scissor_rect(clip, surface.descriptor) else {
            return Ok(());
        };
        let vertices = glyph_vertices(glyphs, transform)?;
        let byte_length = vertices
            .len()
            .checked_mul(size_of::<GlyphVertex>())
            .and_then(|length| u64::try_from(length).ok())
            .ok_or(MetalError::new(MetalErrorCode::SubmissionFailed))?;
        let vertex_buffer = self.device.new_buffer_with_data(
            vertices.as_ptr().cast(),
            byte_length,
            MTLResourceOptions::CPUCacheModeDefaultCache,
        );
        let descriptor = render_pass(surface, MTLLoadAction::Load)?;
        let encoder = command_buffer.new_render_command_encoder(descriptor);
        encoder.set_render_pipeline_state(&self.glyph_pipeline);
        encoder.set_scissor_rect(scissor);
        encoder.set_vertex_buffer(0, Some(&vertex_buffer), 0);
        let viewport = viewport_size(surface.descriptor);
        encoder.set_vertex_bytes(1, size_of_val(&viewport) as u64, viewport.as_ptr().cast());
        let paint = paint_color(paint.color());
        encoder.set_fragment_bytes(0, size_of_val(&paint) as u64, paint.as_ptr().cast());
        encoder.set_fragment_texture(0, Some(atlas));
        encoder.draw_primitives(MTLPrimitiveType::Triangle, 0, vertices.len() as u64);
        encoder.end_encoding();
        Ok(())
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SolidVertex {
    position: [f32; 2],
}

struct CachedAtlasTexture {
    key: GpuGlyphAtlasKey,
    atlas: GpuGlyphAtlas,
    texture: Texture,
    last_used: u64,
    bytes: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct GlyphVertex {
    position: [f32; 2],
    atlas_position: [f32; 2],
    mask: u32,
}

fn solid_rect_vertices(rect: Rect, transform: Transform) -> Result<[SolidVertex; 6], MetalError> {
    let logical = [
        Point::new(rect.left(), rect.top()),
        Point::new(rect.right(), rect.top()),
        Point::new(rect.left(), rect.bottom()),
        Point::new(rect.right(), rect.bottom()),
    ];
    let mut position = [[0.0; 2]; 4];
    for (index, point) in logical.into_iter().enumerate() {
        let mapped = transform
            .map_point(point)
            .map_err(|_| MetalError::new(MetalErrorCode::SubmissionFailed))?;
        position[index] = [scalar_to_f32(mapped.x()), scalar_to_f32(mapped.y())];
    }
    Ok([0, 1, 2, 1, 3, 2].map(|index| SolidVertex {
        position: position[index],
    }))
}

fn viewport_size(descriptor: GpuSurfaceDescriptor) -> [f32; 2] {
    [descriptor.width() as f32, descriptor.height() as f32]
}

fn paint_color(color: Color) -> [f32; 4] {
    let [red, green, blue, alpha] = color.channels();
    let scale = f32::from(u8::MAX);
    [
        f32::from(red) / scale,
        f32::from(green) / scale,
        f32::from(blue) / scale,
        f32::from(alpha) / scale,
    ]
}

fn glyph_vertices(
    glyphs: &[GpuGlyphQuad],
    transform: Transform,
) -> Result<Vec<GlyphVertex>, MetalError> {
    let capacity = glyphs
        .len()
        .checked_mul(6)
        .ok_or(MetalError::new(MetalErrorCode::SubmissionFailed))?;
    let mut vertices = Vec::new();
    vertices
        .try_reserve_exact(capacity)
        .map_err(|_| MetalError::new(MetalErrorCode::SubmissionFailed))?;
    for glyph in glyphs {
        let destination = glyph.destination();
        let source = glyph.source();
        let logical = [
            Point::new(destination.left(), destination.top()),
            Point::new(destination.right(), destination.top()),
            Point::new(destination.left(), destination.bottom()),
            Point::new(destination.right(), destination.bottom()),
        ];
        let mut position = [[0.0; 2]; 4];
        for (index, point) in logical.into_iter().enumerate() {
            let mapped = transform
                .map_point(point)
                .map_err(|_| MetalError::new(MetalErrorCode::SubmissionFailed))?;
            position[index] = [scalar_to_f32(mapped.x()), scalar_to_f32(mapped.y())];
        }
        let left = source.x() as f32;
        let top = source.y() as f32;
        let right = source.x().saturating_add(source.width()) as f32;
        let bottom = source.y().saturating_add(source.height()) as f32;
        let atlas_position = [[left, top], [right, top], [left, bottom], [right, bottom]];
        let mask = u32::from(glyph.is_mask());
        for index in [0, 1, 2, 1, 3, 2] {
            vertices.push(GlyphVertex {
                position: position[index],
                atlas_position: atlas_position[index],
                mask,
            });
        }
    }
    Ok(vertices)
}

fn encode_clear(
    command_buffer: &metal::CommandBufferRef,
    surface: &MetalSurface,
    color: Color,
) -> Result<(), MetalError> {
    let descriptor = render_pass(surface, MTLLoadAction::Clear)?;
    let attachment = descriptor
        .color_attachments()
        .object_at(0)
        .ok_or(MetalError::new(MetalErrorCode::SurfaceAllocationFailed))?;
    attachment.set_clear_color(clear_color(color));
    let encoder = command_buffer.new_render_command_encoder(descriptor);
    encoder.end_encoding();
    Ok(())
}

fn render_pass<'a>(
    surface: &MetalSurface,
    load_action: MTLLoadAction,
) -> Result<&'a metal::RenderPassDescriptorRef, MetalError> {
    let descriptor = RenderPassDescriptor::new();
    let attachment = descriptor
        .color_attachments()
        .object_at(0)
        .ok_or(MetalError::new(MetalErrorCode::SurfaceAllocationFailed))?;
    attachment.set_texture(Some(&surface.texture));
    attachment.set_load_action(load_action);
    attachment.set_store_action(MTLStoreAction::Store);
    Ok(descriptor)
}

fn scissor_rect(clip: Option<Rect>, surface: GpuSurfaceDescriptor) -> Option<MTLScissorRect> {
    let width = u64::from(surface.width());
    let height = u64::from(surface.height());
    let Some(clip) = clip else {
        return Some(MTLScissorRect {
            x: 0,
            y: 0,
            width,
            height,
        });
    };
    let left = scalar_floor(clip.left()).clamp(0, i64::from(surface.width()));
    let top = scalar_floor(clip.top()).clamp(0, i64::from(surface.height()));
    let right = scalar_ceil(clip.right()).clamp(0, i64::from(surface.width()));
    let bottom = scalar_ceil(clip.bottom()).clamp(0, i64::from(surface.height()));
    if left >= right || top >= bottom {
        return None;
    }
    Some(MTLScissorRect {
        x: left as u64,
        y: top as u64,
        width: (right - left) as u64,
        height: (bottom - top) as u64,
    })
}

fn scalar_to_f32(value: Scalar) -> f32 {
    value.bits() as f32 / 65_536.0
}

fn scalar_floor(value: Scalar) -> i64 {
    i64::from(value.bits()).div_euclid(65_536)
}

fn scalar_ceil(value: Scalar) -> i64 {
    -i64::from(value.bits()).div_euclid(-65_536)
}

fn clear_color(color: Color) -> MTLClearColor {
    let [red, green, blue, alpha] = color.channels();
    let scale = f64::from(u8::MAX);
    MTLClearColor::new(
        f64::from(red) / scale,
        f64::from(green) / scale,
        f64::from(blue) / scale,
        f64::from(alpha) / scale,
    )
}
