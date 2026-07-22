use std::{collections::HashMap, mem::size_of};

use metal::{
    Device, LibraryRef, MTLLoadAction, MTLOrigin, MTLPixelFormat, MTLPrimitiveType, MTLRegion,
    MTLResourceOptions, MTLSize, MTLStorageMode, MTLStoreAction, MTLTextureUsage,
    RenderPassDescriptor, RenderPipelineDescriptor, RenderPipelineState, Texture,
    TextureDescriptor,
};
use skia_core::{ClipOp, FillRule, Path, StrokeOptions, Transform};
use skia_gpu::{GpuClipGeometry, GpuClipId, GpuCommandBuffer, GpuSurfaceDescriptor};
use skia_tessellation::{DEFAULT_CURVE_STEPS, FlatteningLimits, PathFlattener, stroke_mesh};

use super::clip_geometry::{ClipEdge, clip_edges, path_edges};
use crate::{MetalError, MetalErrorCode};

struct MaskInput<'a> {
    parent: &'a Texture,
    edges: &'a [ClipEdge],
    rule: FillRule,
    op: ClipOp,
    has_parent: bool,
}

pub(crate) struct ClipRenderer {
    device: Device,
    pipeline: RenderPipelineState,
    stroke_pipeline: RenderPipelineState,
    unclipped: Texture,
}

impl ClipRenderer {
    pub(crate) fn new(device: &Device, library: &LibraryRef) -> Result<Self, MetalError> {
        let vertex = library
            .get_function("skia_clip_vertex", None)
            .map_err(|_| MetalError::new(MetalErrorCode::ShaderLibraryFailed))?;
        let fragment = library
            .get_function("skia_clip_fragment", None)
            .map_err(|_| MetalError::new(MetalErrorCode::ShaderLibraryFailed))?;
        let descriptor = RenderPipelineDescriptor::new();
        descriptor.set_vertex_function(Some(&vertex));
        descriptor.set_fragment_function(Some(&fragment));
        let attachment = descriptor
            .color_attachments()
            .object_at(0)
            .ok_or(MetalError::new(MetalErrorCode::PipelineCreationFailed))?;
        attachment.set_pixel_format(MTLPixelFormat::R8Unorm);
        let pipeline = device
            .new_render_pipeline_state(&descriptor)
            .map_err(|_| MetalError::new(MetalErrorCode::PipelineCreationFailed))?;
        let stroke_vertex = library
            .get_function("skia_stroke_vertex", None)
            .map_err(|_| MetalError::new(MetalErrorCode::ShaderLibraryFailed))?;
        let stroke_fragment = library
            .get_function("skia_stroke_fragment", None)
            .map_err(|_| MetalError::new(MetalErrorCode::ShaderLibraryFailed))?;
        let stroke_descriptor = RenderPipelineDescriptor::new();
        stroke_descriptor.set_vertex_function(Some(&stroke_vertex));
        stroke_descriptor.set_fragment_function(Some(&stroke_fragment));
        let stroke_attachment = stroke_descriptor
            .color_attachments()
            .object_at(0)
            .ok_or(MetalError::new(MetalErrorCode::PipelineCreationFailed))?;
        stroke_attachment.set_pixel_format(MTLPixelFormat::R8Unorm);
        let stroke_pipeline = device
            .new_render_pipeline_state(&stroke_descriptor)
            .map_err(|_| MetalError::new(MetalErrorCode::PipelineCreationFailed))?;
        Ok(Self {
            device: device.clone(),
            pipeline,
            stroke_pipeline,
            unclipped: new_unclipped_texture(device),
        })
    }

    pub(crate) fn unclipped_texture(&self) -> &Texture {
        &self.unclipped
    }

    pub(crate) fn ensure_texture(
        &self,
        command_buffer: &metal::CommandBufferRef,
        surface: GpuSurfaceDescriptor,
        commands: &GpuCommandBuffer,
        clip: GpuClipId,
        textures: &mut HashMap<GpuClipId, Texture>,
    ) -> Result<(), MetalError> {
        let mut missing = Vec::new();
        let mut current = Some(clip);
        while let Some(id) = current {
            if textures.contains_key(&id) {
                break;
            }
            let node = commands
                .clip_node(id)
                .ok_or(MetalError::new(MetalErrorCode::UnsupportedCommand))?;
            missing.try_reserve(1).map_err(|_| submission_failed())?;
            missing.push((id, node));
            current = node.parent();
        }
        for (id, node) in missing.into_iter().rev() {
            let parent = node
                .parent()
                .and_then(|parent| textures.get(&parent))
                .unwrap_or(&self.unclipped);
            let edges = clip_edges(commands, node)?;
            let texture = self.new_texture(surface);
            let rule = match node.geometry() {
                GpuClipGeometry::Path { rule, .. } => rule,
                GpuClipGeometry::Rect(_) => FillRule::NonZero,
            };
            self.encode_mask(
                command_buffer,
                &texture,
                MaskInput {
                    parent,
                    edges: &edges,
                    rule,
                    op: node.op(),
                    has_parent: node.parent().is_some(),
                },
            )?;
            textures.insert(id, texture);
        }
        Ok(())
    }

    /// Renders a transformed fill path into a temporary mask, optionally
    /// intersected with an already-materialized complex clip mask.
    pub(crate) fn rasterize_path(
        &self,
        command_buffer: &metal::CommandBufferRef,
        surface: GpuSurfaceDescriptor,
        path: &Path,
        rule: FillRule,
        transform: Transform,
        parent: Option<&Texture>,
    ) -> Result<Texture, MetalError> {
        let edges = path_edges(path, transform)?;
        let texture = self.new_texture(surface);
        self.encode_mask(
            command_buffer,
            &texture,
            MaskInput {
                parent: parent.unwrap_or(&self.unclipped),
                edges: &edges,
                rule,
                op: ClipOp::Intersect,
                has_parent: parent.is_some(),
            },
        )?;
        Ok(texture)
    }

    /// Materializes deterministic shared stroke coverage as an R8 shader mask.
    pub(crate) fn rasterize_stroke(
        &self,
        command_buffer: &metal::CommandBufferRef,
        surface: GpuSurfaceDescriptor,
        path: &Path,
        options: &StrokeOptions,
        transform: Transform,
    ) -> Result<Texture, MetalError> {
        let limits = FlatteningLimits::for_path(path, DEFAULT_CURVE_STEPS)
            .map_err(|_| submission_failed())?;
        let contours = PathFlattener::new(limits)
            .flatten(path, transform)
            .map_err(|_| submission_failed())?
            .into_contours();
        let mesh = stroke_mesh(&contours, options).map_err(|_| submission_failed())?;
        if mesh.vertices().is_empty() {
            return Err(MetalError::new(MetalErrorCode::UnsupportedCommand));
        }
        let mut vertices = Vec::new();
        vertices
            .try_reserve_exact(mesh.vertices().len())
            .map_err(|_| submission_failed())?;
        for point in mesh.vertices() {
            vertices.push(StrokeVertex {
                position: [
                    point.x().bits() as f32 / 65_536.0,
                    point.y().bits() as f32 / 65_536.0,
                ],
            });
        }
        let byte_length = vertices
            .len()
            .checked_mul(size_of::<StrokeVertex>())
            .and_then(|value| u64::try_from(value).ok())
            .ok_or_else(submission_failed)?;
        let buffer = self.device.new_buffer_with_data(
            vertices.as_ptr().cast(),
            byte_length,
            MTLResourceOptions::CPUCacheModeDefaultCache,
        );
        let texture = self.new_texture(surface);
        let descriptor = stroke_render_pass(&texture)?;
        let encoder = command_buffer.new_render_command_encoder(descriptor);
        encoder.set_render_pipeline_state(&self.stroke_pipeline);
        encoder.set_vertex_buffer(0, Some(&buffer), 0);
        let viewport = [surface.width() as f32, surface.height() as f32];
        encoder.set_vertex_bytes(1, size_of::<[f32; 2]>() as u64, viewport.as_ptr().cast());
        encoder.draw_primitives(
            MTLPrimitiveType::Triangle,
            0,
            u64::try_from(vertices.len()).map_err(|_| submission_failed())?,
        );
        encoder.end_encoding();
        Ok(texture)
    }

    fn new_texture(&self, surface: GpuSurfaceDescriptor) -> Texture {
        let descriptor = TextureDescriptor::new();
        descriptor.set_width(u64::from(surface.width()));
        descriptor.set_height(u64::from(surface.height()));
        descriptor.set_pixel_format(MTLPixelFormat::R8Unorm);
        descriptor.set_usage(MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead);
        descriptor.set_storage_mode(MTLStorageMode::Private);
        self.device.new_texture(&descriptor)
    }

    fn encode_mask(
        &self,
        command_buffer: &metal::CommandBufferRef,
        output: &Texture,
        input: MaskInput<'_>,
    ) -> Result<(), MetalError> {
        let byte_length = input
            .edges
            .len()
            .checked_mul(size_of::<ClipEdge>())
            .and_then(|length| u64::try_from(length).ok())
            .ok_or_else(submission_failed)?;
        let edge_buffer = self.device.new_buffer_with_data(
            input.edges.as_ptr().cast(),
            byte_length,
            MTLResourceOptions::CPUCacheModeDefaultCache,
        );
        let descriptor = texture_render_pass(output)?;
        let encoder = command_buffer.new_render_command_encoder(descriptor);
        encoder.set_render_pipeline_state(&self.pipeline);
        encoder.set_fragment_buffer(0, Some(&edge_buffer), 0);
        let uniforms = [
            u32::try_from(input.edges.len()).map_err(|_| submission_failed())?,
            u32::from(input.rule == FillRule::EvenOdd),
            u32::from(input.op == ClipOp::Difference),
            u32::from(input.has_parent),
        ];
        encoder.set_fragment_bytes(1, size_of::<[u32; 4]>() as u64, uniforms.as_ptr().cast());
        encoder.set_fragment_texture(0, Some(input.parent));
        encoder.draw_primitives(MTLPrimitiveType::Triangle, 0, 3);
        encoder.end_encoding();
        Ok(())
    }
}

fn texture_render_pass<'a>(
    texture: &Texture,
) -> Result<&'a metal::RenderPassDescriptorRef, MetalError> {
    let descriptor = RenderPassDescriptor::new();
    let attachment = descriptor
        .color_attachments()
        .object_at(0)
        .ok_or(MetalError::new(MetalErrorCode::SurfaceAllocationFailed))?;
    attachment.set_texture(Some(texture));
    attachment.set_load_action(MTLLoadAction::DontCare);
    attachment.set_store_action(MTLStoreAction::Store);
    Ok(descriptor)
}

fn stroke_render_pass<'a>(
    texture: &Texture,
) -> Result<&'a metal::RenderPassDescriptorRef, MetalError> {
    let descriptor = RenderPassDescriptor::new();
    let attachment = descriptor
        .color_attachments()
        .object_at(0)
        .ok_or(MetalError::new(MetalErrorCode::SurfaceAllocationFailed))?;
    attachment.set_texture(Some(texture));
    attachment.set_load_action(MTLLoadAction::Clear);
    attachment.set_store_action(MTLStoreAction::Store);
    attachment.set_clear_color(metal::MTLClearColor::new(0.0, 0.0, 0.0, 0.0));
    Ok(descriptor)
}

fn new_unclipped_texture(device: &Device) -> Texture {
    let descriptor = TextureDescriptor::new();
    descriptor.set_width(1);
    descriptor.set_height(1);
    descriptor.set_pixel_format(MTLPixelFormat::R8Unorm);
    descriptor.set_usage(MTLTextureUsage::ShaderRead);
    descriptor.set_storage_mode(MTLStorageMode::Managed);
    let texture = device.new_texture(&descriptor);
    let value = u8::MAX;
    texture.replace_region(
        MTLRegion {
            origin: MTLOrigin { x: 0, y: 0, z: 0 },
            size: MTLSize {
                width: 1,
                height: 1,
                depth: 1,
            },
        },
        0,
        (&value as *const u8).cast(),
        1,
    );
    texture
}

fn submission_failed() -> MetalError {
    MetalError::new(MetalErrorCode::SubmissionFailed)
}

#[repr(C)]
struct StrokeVertex {
    position: [f32; 2],
}
