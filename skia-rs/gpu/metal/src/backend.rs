//! macOS Metal submission backend for `skia-gpu`.
//!
//! This adapter creates native Metal textures and command buffers, executes
//! clears, gradients, filters, isolated layers, paths, images, and atlas-backed
//! glyph batches through Metal shaders. Stable atlas keys enable bounded native
//! texture reuse across submissions, while destination snapshots provide the
//! complete backend-neutral blend-mode contract.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod clip;
mod clip_geometry;

use std::{collections::HashMap, fmt, mem::size_of};

use metal::{
    CommandQueue, Device, MTLClearColor, MTLCommandBufferStatus, MTLLoadAction, MTLOrigin,
    MTLPixelFormat, MTLPrimitiveType, MTLRegion, MTLResourceOptions, MTLScissorRect, MTLSize,
    MTLStorageMode, MTLStoreAction, MTLTextureUsage, RenderPassDescriptor,
    RenderPipelineDescriptor, RenderPipelineState, Texture, TextureDescriptor,
};
use skia_core::{
    BlendMode, Color, ColorFilter, GradientGeometry, ImageFilter, Paint, Point, Rect,
    SamplingFilter, SamplingOptions, SaveLayerOptions, Scalar, TileMode, Transform,
};
use skia_gpu::{
    GpuBackend, GpuCapabilities, GpuCommand, GpuCommandBuffer, GpuGlyphAtlas, GpuGlyphAtlasKey,
    GpuGlyphQuad, GpuSurfaceDescriptor,
};
use skia_image::Image;

use self::clip::ClipRenderer;

const DEFAULT_ATLAS_CACHE_CAPACITY: usize = 8;
const DEFAULT_ATLAS_CACHE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TEXTURE_DIMENSION_2D: u32 = 16_384;

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
#[derive(Clone)]
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
    image_pipeline: RenderPipelineState,
    box_blur_pipeline: RenderPipelineState,
    clip_renderer: ClipRenderer,
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
    fn allocate_surface(
        &self,
        descriptor: GpuSurfaceDescriptor,
    ) -> Result<MetalSurface, MetalError> {
        if !self.capabilities().supports_surface(descriptor) {
            return Err(MetalError::new(MetalErrorCode::SurfaceAllocationFailed));
        }
        let texture_descriptor = TextureDescriptor::new();
        texture_descriptor.set_width(u64::from(descriptor.width()));
        texture_descriptor.set_height(u64::from(descriptor.height()));
        texture_descriptor.set_pixel_format(MTLPixelFormat::RGBA8Unorm);
        texture_descriptor.set_usage(MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead);
        texture_descriptor.set_storage_mode(MTLStorageMode::Managed);
        let texture = self.device.new_texture(&texture_descriptor);
        Ok(MetalSurface {
            texture,
            descriptor,
        })
    }

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
        let glyph_pipeline = device
            .new_render_pipeline_state(&glyph_descriptor)
            .map_err(|_| MetalError::new(MetalErrorCode::PipelineCreationFailed))?;
        let image_vertex = library
            .get_function("skia_image_vertex", None)
            .map_err(|_| MetalError::new(MetalErrorCode::ShaderLibraryFailed))?;
        let image_fragment = library
            .get_function("skia_image_fragment", None)
            .map_err(|_| MetalError::new(MetalErrorCode::ShaderLibraryFailed))?;
        let image_descriptor = RenderPipelineDescriptor::new();
        image_descriptor.set_vertex_function(Some(&image_vertex));
        image_descriptor.set_fragment_function(Some(&image_fragment));
        let image_attachment = image_descriptor
            .color_attachments()
            .object_at(0)
            .ok_or(MetalError::new(MetalErrorCode::PipelineCreationFailed))?;
        image_attachment.set_pixel_format(MTLPixelFormat::RGBA8Unorm);
        let image_pipeline = device
            .new_render_pipeline_state(&image_descriptor)
            .map_err(|_| MetalError::new(MetalErrorCode::PipelineCreationFailed))?;
        let blur_vertex = library
            .get_function("skia_filter_vertex", None)
            .map_err(|_| MetalError::new(MetalErrorCode::ShaderLibraryFailed))?;
        let blur_fragment = library
            .get_function("skia_box_blur_fragment", None)
            .map_err(|_| MetalError::new(MetalErrorCode::ShaderLibraryFailed))?;
        let blur_descriptor = RenderPipelineDescriptor::new();
        blur_descriptor.set_vertex_function(Some(&blur_vertex));
        blur_descriptor.set_fragment_function(Some(&blur_fragment));
        let blur_attachment = blur_descriptor
            .color_attachments()
            .object_at(0)
            .ok_or(MetalError::new(MetalErrorCode::PipelineCreationFailed))?;
        blur_attachment.set_pixel_format(MTLPixelFormat::RGBA8Unorm);
        let box_blur_pipeline = device
            .new_render_pipeline_state(&blur_descriptor)
            .map_err(|_| MetalError::new(MetalErrorCode::PipelineCreationFailed))?;
        let clip_renderer = ClipRenderer::new(&device, &library)?;
        Ok(Self {
            device,
            queue,
            solid_rect_pipeline,
            glyph_pipeline,
            image_pipeline,
            box_blur_pipeline,
            clip_renderer,
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

    fn capabilities(&self) -> GpuCapabilities {
        let max_bytes = u64::from(MAX_TEXTURE_DIMENSION_2D)
            .saturating_mul(u64::from(MAX_TEXTURE_DIMENSION_2D))
            .saturating_mul(4);
        GpuCapabilities::new(
            MAX_TEXTURE_DIMENSION_2D,
            MAX_TEXTURE_DIMENSION_2D,
            max_bytes,
        )
        .expect("Metal reports a positive 2D texture limit")
    }

    fn create_surface(
        &mut self,
        descriptor: GpuSurfaceDescriptor,
    ) -> Result<Self::Surface, Self::Error> {
        self.allocate_surface(descriptor)
    }

    fn submit(
        &mut self,
        surface: &mut Self::Surface,
        commands: &GpuCommandBuffer,
    ) -> Result<(), Self::Error> {
        for command in commands.commands() {
            match command {
                GpuCommand::Clear(_) => {}
                GpuCommand::SaveLayer { clip, .. } => {
                    if clip.is_some_and(|clip| commands.clip_node(clip).is_none()) {
                        return Err(MetalError::new(MetalErrorCode::UnsupportedCommand));
                    }
                }
                GpuCommand::RestoreLayer | GpuCommand::FillRect { .. } => {}
                GpuCommand::FillPath { path, .. } | GpuCommand::StrokePath { path, .. } => {
                    if commands.path(*path).is_none() {
                        return Err(MetalError::new(MetalErrorCode::UnsupportedCommand));
                    }
                }
                GpuCommand::DrawImage { image, .. } => {
                    if commands.image(*image).is_none() {
                        return Err(MetalError::new(MetalErrorCode::UnsupportedCommand));
                    }
                }
                GpuCommand::DrawGlyphs { atlas, .. } => {
                    if commands.glyph_atlas(*atlas).is_none() {
                        return Err(MetalError::new(MetalErrorCode::UnsupportedCommand));
                    }
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
        let mut clip_textures = HashMap::new();
        let mut layers = Vec::<MetalLayer>::new();
        for command in commands.commands() {
            match command {
                GpuCommand::Clear(color) => {
                    let target = current_target(surface, &layers);
                    encode_clear(command_buffer, &target, *color)?;
                }
                GpuCommand::SaveLayer {
                    options,
                    transform,
                    scissor,
                    clip,
                } => {
                    let layer_surface = self.allocate_surface(surface.descriptor)?;
                    encode_clear(command_buffer, &layer_surface, Color::TRANSPARENT)?;
                    layers.push(MetalLayer {
                        surface: layer_surface,
                        options: *options,
                        transform: *transform,
                        scissor: *scissor,
                        clip: *clip,
                    });
                }
                GpuCommand::RestoreLayer => {
                    let layer = layers
                        .pop()
                        .ok_or(MetalError::new(MetalErrorCode::UnsupportedCommand))?;
                    if let Some(clip) = layer.clip {
                        self.clip_renderer.ensure_texture(
                            command_buffer,
                            surface.descriptor,
                            commands,
                            clip,
                            &mut clip_textures,
                        )?;
                    }
                    let parent = current_target(surface, &layers);
                    let clip_texture = layer
                        .clip
                        .and_then(|id| clip_textures.get(&id))
                        .unwrap_or(self.clip_renderer.unclipped_texture());
                    self.encode_layer_restore(command_buffer, &parent, layer, clip_texture)?;
                }
                GpuCommand::FillRect {
                    rect,
                    paint,
                    transform,
                    scissor,
                    clip,
                } => {
                    let target = current_target(surface, &layers);
                    if let Some(clip) = clip {
                        self.clip_renderer.ensure_texture(
                            command_buffer,
                            surface.descriptor,
                            commands,
                            *clip,
                            &mut clip_textures,
                        )?;
                    }
                    let clip_texture = clip
                        .and_then(|id| clip_textures.get(&id))
                        .unwrap_or(self.clip_renderer.unclipped_texture());
                    self.encode_solid_rect(
                        command_buffer,
                        &target,
                        *rect,
                        *paint,
                        *transform,
                        *scissor,
                        clip_texture,
                        clip.is_some(),
                    )?;
                }
                GpuCommand::FillPath {
                    path,
                    rule,
                    paint,
                    transform,
                    scissor,
                    clip,
                } => {
                    let target = current_target(surface, &layers);
                    if let Some(clip) = clip {
                        self.clip_renderer.ensure_texture(
                            command_buffer,
                            surface.descriptor,
                            commands,
                            *clip,
                            &mut clip_textures,
                        )?;
                    }
                    let path = commands
                        .path(*path)
                        .ok_or(MetalError::new(MetalErrorCode::UnsupportedCommand))?;
                    let parent = clip.and_then(|id| clip_textures.get(&id));
                    let fill_mask = self.clip_renderer.rasterize_path(
                        command_buffer,
                        surface.descriptor,
                        path,
                        *rule,
                        *transform,
                        parent,
                    )?;
                    self.encode_solid_surface(
                        command_buffer,
                        &target,
                        *paint,
                        *transform,
                        *scissor,
                        &fill_mask,
                    )?;
                }
                GpuCommand::DrawImage {
                    image,
                    destination,
                    opacity,
                    sampling,
                    paint,
                    transform,
                    scissor,
                    clip,
                    ..
                } => {
                    let target = current_target(surface, &layers);
                    if let Some(clip) = clip {
                        self.clip_renderer.ensure_texture(
                            command_buffer,
                            surface.descriptor,
                            commands,
                            *clip,
                            &mut clip_textures,
                        )?;
                    }
                    let image = commands
                        .image(*image)
                        .ok_or(MetalError::new(MetalErrorCode::UnsupportedCommand))?;
                    let texture = self.upload_image(image);
                    let clip_texture = clip
                        .and_then(|id| clip_textures.get(&id))
                        .unwrap_or(self.clip_renderer.unclipped_texture());
                    self.encode_image(
                        command_buffer,
                        &target,
                        &texture,
                        *destination,
                        *opacity,
                        *paint,
                        *sampling,
                        *transform,
                        *scissor,
                        clip_texture,
                        clip.is_some(),
                    )?;
                }
                GpuCommand::DrawGlyphs {
                    atlas,
                    glyphs,
                    paint,
                    transform,
                    scissor,
                    clip,
                } => {
                    let target = current_target(surface, &layers);
                    if let Some(clip) = clip {
                        self.clip_renderer.ensure_texture(
                            command_buffer,
                            surface.descriptor,
                            commands,
                            *clip,
                            &mut clip_textures,
                        )?;
                    }
                    let texture = atlas_textures
                        .get(atlas)
                        .ok_or(MetalError::new(MetalErrorCode::SubmissionFailed))?;
                    let clip_texture = clip
                        .and_then(|id| clip_textures.get(&id))
                        .unwrap_or(self.clip_renderer.unclipped_texture());
                    self.encode_glyphs(
                        command_buffer,
                        &target,
                        texture,
                        glyphs,
                        *paint,
                        *transform,
                        *scissor,
                        clip_texture,
                        clip.is_some(),
                    )?;
                }
                GpuCommand::StrokePath {
                    path,
                    options,
                    paint,
                    transform,
                    scissor,
                    clip,
                } => {
                    let target = current_target(surface, &layers);
                    if let Some(clip) = clip {
                        self.clip_renderer.ensure_texture(
                            command_buffer,
                            surface.descriptor,
                            commands,
                            *clip,
                            &mut clip_textures,
                        )?;
                    }
                    let path = commands
                        .path(*path)
                        .ok_or(MetalError::new(MetalErrorCode::UnsupportedCommand))?;
                    let stroke_mask = self.clip_renderer.rasterize_stroke(
                        command_buffer,
                        surface.descriptor,
                        path,
                        options,
                        *transform,
                    )?;
                    let clip_texture = clip
                        .and_then(|id| clip_textures.get(&id))
                        .unwrap_or(self.clip_renderer.unclipped_texture());
                    self.encode_solid_surface_with_masks(
                        command_buffer,
                        &target,
                        *paint,
                        *transform,
                        *scissor,
                        clip_texture,
                        clip.is_some(),
                        &stroke_mask,
                    )?;
                }
            }
        }
        if !layers.is_empty() {
            return Err(MetalError::new(MetalErrorCode::UnsupportedCommand));
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
        scissor: Option<Rect>,
        clip_texture: &Texture,
        has_clip: bool,
    ) -> Result<(), MetalError> {
        let Some(scissor) = scissor_rect(scissor, surface.descriptor) else {
            return Ok(());
        };
        let vertices = solid_rect_vertices(rect, transform)?;
        self.encode_solid_vertices(
            command_buffer,
            surface,
            &vertices,
            paint,
            scissor,
            clip_texture,
            has_clip,
            self.clip_renderer.unclipped_texture(),
            false,
        )
    }

    fn encode_solid_surface(
        &self,
        command_buffer: &metal::CommandBufferRef,
        surface: &MetalSurface,
        paint: skia_core::Paint,
        transform: Transform,
        scissor: Option<Rect>,
        clip_texture: &Texture,
    ) -> Result<(), MetalError> {
        let Some(scissor) = scissor_rect(scissor, surface.descriptor) else {
            return Ok(());
        };
        let vertices = solid_surface_vertices(surface.descriptor, transform)?;
        self.encode_solid_vertices(
            command_buffer,
            surface,
            &vertices,
            paint,
            scissor,
            clip_texture,
            true,
            self.clip_renderer.unclipped_texture(),
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_solid_surface_with_masks(
        &self,
        command_buffer: &metal::CommandBufferRef,
        surface: &MetalSurface,
        paint: skia_core::Paint,
        transform: Transform,
        scissor: Option<Rect>,
        clip_texture: &Texture,
        has_clip: bool,
        shape_mask: &Texture,
    ) -> Result<(), MetalError> {
        let Some(scissor) = scissor_rect(scissor, surface.descriptor) else {
            return Ok(());
        };
        self.encode_solid_vertices(
            command_buffer,
            surface,
            &solid_surface_vertices(surface.descriptor, transform)?,
            paint,
            scissor,
            clip_texture,
            has_clip,
            shape_mask,
            true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_solid_vertices(
        &self,
        command_buffer: &metal::CommandBufferRef,
        surface: &MetalSurface,
        vertices: &[SolidVertex],
        paint: skia_core::Paint,
        scissor: MTLScissorRect,
        clip_texture: &Texture,
        has_clip: bool,
        shape_texture: &Texture,
        has_shape: bool,
    ) -> Result<(), MetalError> {
        let byte_length = u64::try_from(size_of_val(vertices))
            .map_err(|_| MetalError::new(MetalErrorCode::SubmissionFailed))?;
        let vertex_buffer = self.device.new_buffer_with_data(
            vertices.as_ptr().cast(),
            byte_length,
            MTLResourceOptions::CPUCacheModeDefaultCache,
        );
        let descriptor = render_pass(surface, MTLLoadAction::Load)?;
        let destination = self.snapshot_texture(command_buffer, surface)?;
        let encoder = command_buffer.new_render_command_encoder(descriptor);
        encoder.set_render_pipeline_state(&self.solid_rect_pipeline);
        encoder.set_scissor_rect(scissor);
        encoder.set_vertex_buffer(0, Some(&vertex_buffer), 0);
        let viewport = viewport_size(surface.descriptor);
        encoder.set_vertex_bytes(1, size_of_val(&viewport) as u64, viewport.as_ptr().cast());
        let paint = paint_uniforms(paint);
        encoder.set_fragment_bytes(
            0,
            size_of_val(&paint) as u64,
            (&paint as *const PaintUniforms).cast(),
        );
        let has_clip = u32::from(has_clip);
        encoder.set_fragment_bytes(
            1,
            size_of_val(&has_clip) as u64,
            (&has_clip as *const u32).cast(),
        );
        encoder.set_fragment_texture(0, Some(clip_texture));
        let has_shape = u32::from(has_shape);
        encoder.set_fragment_bytes(
            2,
            size_of_val(&has_shape) as u64,
            (&has_shape as *const u32).cast(),
        );
        encoder.set_fragment_texture(1, Some(shape_texture));
        encoder.set_fragment_texture(2, Some(&destination));
        encoder.draw_primitives(MTLPrimitiveType::Triangle, 0, vertices.len() as u64);
        encoder.end_encoding();
        Ok(())
    }

    fn snapshot_texture(
        &self,
        command_buffer: &metal::CommandBufferRef,
        surface: &MetalSurface,
    ) -> Result<Texture, MetalError> {
        let descriptor = TextureDescriptor::new();
        descriptor.set_width(u64::from(surface.descriptor.width()));
        descriptor.set_height(u64::from(surface.descriptor.height()));
        descriptor.set_pixel_format(MTLPixelFormat::RGBA8Unorm);
        descriptor.set_usage(MTLTextureUsage::ShaderRead);
        descriptor.set_storage_mode(MTLStorageMode::Private);
        let snapshot = self.device.new_texture(&descriptor);
        let blit = command_buffer.new_blit_command_encoder();
        blit.copy_from_texture(
            &surface.texture,
            0,
            0,
            MTLOrigin { x: 0, y: 0, z: 0 },
            MTLSize {
                width: u64::from(surface.descriptor.width()),
                height: u64::from(surface.descriptor.height()),
                depth: 1,
            },
            &snapshot,
            0,
            0,
            MTLOrigin { x: 0, y: 0, z: 0 },
        );
        blit.end_encoding();
        Ok(snapshot)
    }

    fn encode_layer_restore(
        &self,
        command_buffer: &metal::CommandBufferRef,
        parent: &MetalSurface,
        layer: MetalLayer,
        clip_texture: &Texture,
    ) -> Result<(), MetalError> {
        let source = match layer.options.filter() {
            None => layer.surface,
            Some(ImageFilter::Color(filter)) => {
                self.color_filtered_surface(command_buffer, &layer.surface, filter)?
            }
            Some(ImageFilter::BoxBlur { radius }) => {
                self.box_blurred_surface(command_buffer, &layer.surface, radius)?
            }
        };
        let Some(scissor) = layer_restore_scissor(
            layer.options.bounds(),
            layer.transform,
            layer.scissor,
            parent.descriptor,
        )?
        else {
            return Ok(());
        };
        let destination = surface_rect(parent.descriptor)?;
        let vertices = image_vertices(
            destination,
            source.texture.width() as f32,
            source.texture.height() as f32,
            Transform::IDENTITY,
        )?;
        let paint = Paint::new(Color::WHITE)
            .with_alpha(layer.options.opacity())
            .with_blend_mode(layer.options.blend_mode());
        self.encode_image_vertices(
            command_buffer,
            parent,
            &source.texture,
            &vertices,
            paint,
            SamplingOptions::NEAREST,
            scissor,
            clip_texture,
            layer.clip.is_some(),
        )
    }

    fn color_filtered_surface(
        &self,
        command_buffer: &metal::CommandBufferRef,
        source: &MetalSurface,
        filter: ColorFilter,
    ) -> Result<MetalSurface, MetalError> {
        let output = self.allocate_surface(source.descriptor)?;
        encode_clear(command_buffer, &output, Color::TRANSPARENT)?;
        self.encode_image(
            command_buffer,
            &output,
            &source.texture,
            surface_rect(source.descriptor)?,
            u8::MAX,
            Paint::new(Color::WHITE)
                .with_color_filter(filter)
                .with_blend_mode(BlendMode::Source),
            SamplingOptions::NEAREST,
            Transform::IDENTITY,
            None,
            self.clip_renderer.unclipped_texture(),
            false,
        )?;
        Ok(output)
    }

    fn box_blurred_surface(
        &self,
        command_buffer: &metal::CommandBufferRef,
        source: &MetalSurface,
        radius: u8,
    ) -> Result<MetalSurface, MetalError> {
        let horizontal = self.allocate_surface(source.descriptor)?;
        self.encode_box_blur_pass(command_buffer, source, &horizontal, radius, [1, 0])?;
        let vertical = self.allocate_surface(source.descriptor)?;
        self.encode_box_blur_pass(command_buffer, &horizontal, &vertical, radius, [0, 1])?;
        Ok(vertical)
    }

    fn encode_box_blur_pass(
        &self,
        command_buffer: &metal::CommandBufferRef,
        source: &MetalSurface,
        output: &MetalSurface,
        radius: u8,
        direction: [i32; 2],
    ) -> Result<(), MetalError> {
        let descriptor = render_pass(output, MTLLoadAction::DontCare)?;
        let encoder = command_buffer.new_render_command_encoder(descriptor);
        encoder.set_render_pipeline_state(&self.box_blur_pipeline);
        encoder.set_fragment_texture(0, Some(&source.texture));
        encoder.set_fragment_bytes(0, size_of_val(&direction) as u64, direction.as_ptr().cast());
        let radius = u32::from(radius);
        encoder.set_fragment_bytes(
            1,
            size_of_val(&radius) as u64,
            (&radius as *const u32).cast(),
        );
        encoder.draw_primitives(MTLPrimitiveType::Triangle, 0, 3);
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
        self.upload_image(atlas.image())
    }

    fn upload_image(&self, image: &Image) -> Texture {
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
        scissor: Option<Rect>,
        clip_texture: &Texture,
        has_clip: bool,
    ) -> Result<(), MetalError> {
        let Some(scissor) = scissor_rect(scissor, surface.descriptor) else {
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
        let destination = self.snapshot_texture(command_buffer, surface)?;
        let encoder = command_buffer.new_render_command_encoder(descriptor);
        encoder.set_render_pipeline_state(&self.glyph_pipeline);
        encoder.set_scissor_rect(scissor);
        encoder.set_vertex_buffer(0, Some(&vertex_buffer), 0);
        let viewport = viewport_size(surface.descriptor);
        encoder.set_vertex_bytes(1, size_of_val(&viewport) as u64, viewport.as_ptr().cast());
        let paint = paint_uniforms(paint);
        encoder.set_fragment_bytes(
            0,
            size_of_val(&paint) as u64,
            (&paint as *const PaintUniforms).cast(),
        );
        let has_clip = u32::from(has_clip);
        encoder.set_fragment_bytes(
            1,
            size_of_val(&has_clip) as u64,
            (&has_clip as *const u32).cast(),
        );
        encoder.set_fragment_texture(0, Some(atlas));
        encoder.set_fragment_texture(1, Some(clip_texture));
        encoder.set_fragment_texture(2, Some(&destination));
        encoder.draw_primitives(MTLPrimitiveType::Triangle, 0, vertices.len() as u64);
        encoder.end_encoding();
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_image(
        &self,
        command_buffer: &metal::CommandBufferRef,
        surface: &MetalSurface,
        image: &Texture,
        destination: Rect,
        opacity: u8,
        paint: Paint,
        sampling: SamplingOptions,
        transform: Transform,
        scissor: Option<Rect>,
        clip_texture: &Texture,
        has_clip: bool,
    ) -> Result<(), MetalError> {
        let Some(scissor) = scissor_rect(scissor, surface.descriptor) else {
            return Ok(());
        };
        let vertices = image_vertices(
            destination,
            image.width() as f32,
            image.height() as f32,
            transform,
        )?;
        let paint = paint.with_opacity(opacity);
        self.encode_image_vertices(
            command_buffer,
            surface,
            image,
            &vertices,
            paint,
            sampling,
            scissor,
            clip_texture,
            has_clip,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_image_vertices(
        &self,
        command_buffer: &metal::CommandBufferRef,
        surface: &MetalSurface,
        image: &Texture,
        vertices: &[ImageVertex],
        paint: Paint,
        sampling: SamplingOptions,
        scissor: MTLScissorRect,
        clip_texture: &Texture,
        has_clip: bool,
    ) -> Result<(), MetalError> {
        let bytes = vertices
            .len()
            .checked_mul(size_of::<ImageVertex>())
            .and_then(|n| u64::try_from(n).ok())
            .ok_or(MetalError::new(MetalErrorCode::SubmissionFailed))?;
        let buffer = self.device.new_buffer_with_data(
            vertices.as_ptr().cast(),
            bytes,
            MTLResourceOptions::CPUCacheModeDefaultCache,
        );
        let descriptor = render_pass(surface, MTLLoadAction::Load)?;
        let target_snapshot = self.snapshot_texture(command_buffer, surface)?;
        let encoder = command_buffer.new_render_command_encoder(descriptor);
        encoder.set_render_pipeline_state(&self.image_pipeline);
        encoder.set_scissor_rect(scissor);
        encoder.set_vertex_buffer(0, Some(&buffer), 0);
        let viewport = viewport_size(surface.descriptor);
        encoder.set_vertex_bytes(1, size_of_val(&viewport) as u64, viewport.as_ptr().cast());
        let paint = image_paint_uniforms(paint);
        encoder.set_fragment_bytes(
            0,
            size_of_val(&paint) as u64,
            (&paint as *const PaintUniforms).cast(),
        );
        let has_clip = u32::from(has_clip);
        encoder.set_fragment_bytes(
            1,
            size_of_val(&has_clip) as u64,
            (&has_clip as *const u32).cast(),
        );
        let sampling_filter = match sampling.filter() {
            SamplingFilter::Nearest => 0_u32,
            SamplingFilter::Linear => 1_u32,
        };
        encoder.set_fragment_bytes(
            2,
            size_of_val(&sampling_filter) as u64,
            (&sampling_filter as *const u32).cast(),
        );
        encoder.set_fragment_texture(0, Some(image));
        encoder.set_fragment_texture(1, Some(clip_texture));
        encoder.set_fragment_texture(2, Some(&target_snapshot));
        encoder.draw_primitives(MTLPrimitiveType::Triangle, 0, vertices.len() as u64);
        encoder.end_encoding();
        Ok(())
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SolidVertex {
    position: [f32; 2],
    local_position: [f32; 2],
}

struct MetalLayer {
    surface: MetalSurface,
    options: SaveLayerOptions,
    transform: Transform,
    scissor: Option<Rect>,
    clip: Option<skia_gpu::GpuClipId>,
}

fn current_target(surface: &MetalSurface, layers: &[MetalLayer]) -> MetalSurface {
    layers
        .last()
        .map_or_else(|| surface.clone(), |layer| layer.surface.clone())
}

fn surface_rect(descriptor: GpuSurfaceDescriptor) -> Result<Rect, MetalError> {
    let width = i32::try_from(descriptor.width())
        .ok()
        .and_then(|width| Scalar::from_i32(width).ok())
        .ok_or(MetalError::new(MetalErrorCode::SubmissionFailed))?;
    let height = i32::try_from(descriptor.height())
        .ok()
        .and_then(|height| Scalar::from_i32(height).ok())
        .ok_or(MetalError::new(MetalErrorCode::SubmissionFailed))?;
    Rect::new(Scalar::ZERO, Scalar::ZERO, width, height)
        .map_err(|_| MetalError::new(MetalErrorCode::SubmissionFailed))
}

fn layer_restore_scissor(
    bounds: Option<Rect>,
    transform: Transform,
    scissor: Option<Rect>,
    descriptor: GpuSurfaceDescriptor,
) -> Result<Option<MTLScissorRect>, MetalError> {
    let Some(mut result) = scissor_rect(scissor, descriptor) else {
        return Ok(None);
    };
    if let Some(bounds) = bounds {
        let corners = [
            Point::new(bounds.left(), bounds.top()),
            Point::new(bounds.right(), bounds.top()),
            Point::new(bounds.right(), bounds.bottom()),
            Point::new(bounds.left(), bounds.bottom()),
        ];
        let mut left = i64::MAX;
        let mut top = i64::MAX;
        let mut right = i64::MIN;
        let mut bottom = i64::MIN;
        for corner in corners {
            let point = transform
                .map_point(corner)
                .map_err(|_| MetalError::new(MetalErrorCode::SubmissionFailed))?;
            left = left.min(scalar_floor(point.x()));
            top = top.min(scalar_floor(point.y()));
            right = right.max(scalar_ceil(point.x()));
            bottom = bottom.max(scalar_ceil(point.y()));
        }
        let width = i64::from(descriptor.width());
        let height = i64::from(descriptor.height());
        left = left.clamp(0, width);
        top = top.clamp(0, height);
        right = right.clamp(0, width);
        bottom = bottom.clamp(0, height);
        if left >= right || top >= bottom {
            return Ok(None);
        }
        let bounds = MTLScissorRect {
            x: left as u64,
            y: top as u64,
            width: (right - left) as u64,
            height: (bottom - top) as u64,
        };
        let result_right = result.x.saturating_add(result.width);
        let result_bottom = result.y.saturating_add(result.height);
        let bounds_right = bounds.x.saturating_add(bounds.width);
        let bounds_bottom = bounds.y.saturating_add(bounds.height);
        let intersect_left = result.x.max(bounds.x);
        let intersect_top = result.y.max(bounds.y);
        let intersect_right = result_right.min(bounds_right);
        let intersect_bottom = result_bottom.min(bounds_bottom);
        if intersect_left >= intersect_right || intersect_top >= intersect_bottom {
            return Ok(None);
        }
        result = MTLScissorRect {
            x: intersect_left,
            y: intersect_top,
            width: intersect_right - intersect_left,
            height: intersect_bottom - intersect_top,
        };
    }
    Ok(Some(result))
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PaintUniforms {
    color: [f32; 4],
    gradient_colors: [[f32; 4]; 8],
    gradient_offsets: [[f32; 4]; 2],
    gradient_geometry: [f32; 4],
    matrix: [[f32; 4]; 4],
    matrix_bias: [f32; 4],
    filter_color: [f32; 4],
    modes: [u32; 4],
    extra: [u32; 4],
}

const _: [u8; 320] = [0; size_of::<PaintUniforms>()];

struct CachedAtlasTexture {
    key: GpuGlyphAtlasKey,
    atlas: GpuGlyphAtlas,
    texture: Texture,
    last_used: u64,
    bytes: u64,
}

#[repr(C, align(8))]
#[derive(Clone, Copy)]
struct GlyphVertex {
    position: [f32; 2],
    local_position: [f32; 2],
    atlas_position: [f32; 2],
    mask: u32,
}

const _: [u8; 32] = [0; size_of::<GlyphVertex>()];

#[repr(C)]
#[derive(Clone, Copy)]
struct ImageVertex {
    position: [f32; 2],
    image_position: [f32; 2],
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
        local_position: [
            scalar_to_f32(logical[index].x()),
            scalar_to_f32(logical[index].y()),
        ],
    }))
}

fn solid_surface_vertices(
    surface: GpuSurfaceDescriptor,
    transform: Transform,
) -> Result<[SolidVertex; 6], MetalError> {
    let width = surface.width() as f32;
    let height = surface.height() as f32;
    let position = [[0.0, 0.0], [width, 0.0], [0.0, height], [width, height]];
    let inverse = transform
        .inverse()
        .map_err(|_| MetalError::new(MetalErrorCode::SubmissionFailed))?;
    let bounds = surface_rect(surface)?;
    let device = [
        Point::new(bounds.left(), bounds.top()),
        Point::new(bounds.right(), bounds.top()),
        Point::new(bounds.left(), bounds.bottom()),
        Point::new(bounds.right(), bounds.bottom()),
    ];
    let mut local = [[0.0; 2]; 4];
    for (index, point) in device.into_iter().enumerate() {
        let point = inverse
            .map_point(point)
            .map_err(|_| MetalError::new(MetalErrorCode::SubmissionFailed))?;
        local[index] = [scalar_to_f32(point.x()), scalar_to_f32(point.y())];
    }
    Ok([0, 1, 2, 1, 3, 2].map(|index| SolidVertex {
        position: position[index],
        local_position: local[index],
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

fn image_paint_uniforms(paint: Paint) -> PaintUniforms {
    let mut uniforms = paint_uniforms(paint);
    uniforms.modes[0] = 0;
    uniforms
}

fn paint_uniforms(paint: Paint) -> PaintUniforms {
    let mut uniforms = PaintUniforms {
        color: paint_color(paint.color()),
        gradient_colors: [[0.0; 4]; 8],
        gradient_offsets: [[0.0; 4]; 2],
        gradient_geometry: [0.0; 4],
        matrix: [[0.0; 4]; 4],
        matrix_bias: [0.0; 4],
        filter_color: [0.0; 4],
        modes: [0; 4],
        extra: [blend_mode_id(paint.blend_mode()), 0, 0, 0],
    };
    if let Some(gradient) = paint.gradient() {
        uniforms.modes[0] = match gradient.geometry() {
            GradientGeometry::Linear { start, end } => {
                uniforms.gradient_geometry = [
                    scalar_to_f32(start.x()),
                    scalar_to_f32(start.y()),
                    scalar_to_f32(end.x()),
                    scalar_to_f32(end.y()),
                ];
                1
            }
            GradientGeometry::Radial { center, radius } => {
                uniforms.gradient_geometry = [
                    scalar_to_f32(center.x()),
                    scalar_to_f32(center.y()),
                    scalar_to_f32(radius),
                    0.0,
                ];
                2
            }
        };
        uniforms.modes[1] = gradient.stops().len() as u32;
        uniforms.modes[2] = match gradient.tile_mode() {
            TileMode::Clamp => 0,
            TileMode::Repeat => 1,
            TileMode::Mirror => 2,
        };
        for (index, stop) in gradient.stops().iter().enumerate() {
            uniforms.gradient_colors[index] = paint_color(stop.color());
            uniforms.gradient_offsets[index / 4][index % 4] = scalar_to_f32(stop.offset());
        }
    }
    if let Some(filter) = paint.color_filter() {
        match filter {
            ColorFilter::Matrix(matrix) => {
                uniforms.modes[3] = 1;
                let values = matrix.values();
                for row in 0..4 {
                    for column in 0..4 {
                        uniforms.matrix[row][column] = values[row * 5 + column] as f32 / 65_536.0;
                    }
                    uniforms.matrix_bias[row] = values[row * 5 + 4] as f32 / 65_536.0 / 255.0;
                }
            }
            ColorFilter::Blend { color, mode } => {
                uniforms.modes[3] = 2;
                uniforms.filter_color = paint_color(color);
                uniforms.extra[1] = blend_mode_id(mode);
            }
        }
    }
    uniforms
}

fn blend_mode_id(mode: BlendMode) -> u32 {
    match mode {
        BlendMode::Clear => 0,
        BlendMode::Source => 1,
        BlendMode::Destination => 2,
        BlendMode::SourceOver => 3,
        BlendMode::DestinationOver => 4,
        BlendMode::SourceIn => 5,
        BlendMode::DestinationIn => 6,
        BlendMode::SourceOut => 7,
        BlendMode::DestinationOut => 8,
        BlendMode::SourceAtop => 9,
        BlendMode::DestinationAtop => 10,
        BlendMode::Xor => 11,
        BlendMode::Plus => 12,
        BlendMode::Modulate => 13,
        BlendMode::Multiply => 14,
        BlendMode::Screen => 15,
        BlendMode::Overlay => 16,
        BlendMode::Darken => 17,
        BlendMode::Lighten => 18,
        BlendMode::ColorDodge => 19,
        BlendMode::ColorBurn => 20,
        BlendMode::HardLight => 21,
        BlendMode::SoftLight => 22,
        BlendMode::Difference => 23,
        BlendMode::Exclusion => 24,
        BlendMode::Hue => 25,
        BlendMode::Saturation => 26,
        BlendMode::Color => 27,
        BlendMode::Luminosity => 28,
    }
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
                local_position: [
                    scalar_to_f32(logical[index].x()),
                    scalar_to_f32(logical[index].y()),
                ],
                atlas_position: atlas_position[index],
                mask,
            });
        }
    }
    Ok(vertices)
}

fn image_vertices(
    destination: Rect,
    width: f32,
    height: f32,
    transform: Transform,
) -> Result<[ImageVertex; 6], MetalError> {
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
    let image_position = [[0.0, 0.0], [width, 0.0], [0.0, height], [width, height]];
    Ok([0, 1, 2, 1, 3, 2].map(|index| ImageVertex {
        position: position[index],
        image_position: image_position[index],
    }))
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
