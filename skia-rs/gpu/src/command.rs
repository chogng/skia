use skia_core::{
    Color, FillRule, Paint, Path, Rect, SamplingOptions, SaveLayerOptions, Shader, StrokeOptions,
    Transform,
};
use skia_image::Image;

use crate::{
    GpuClipId, GpuClipNode, GpuGlyphAtlas, GpuGlyphAtlasId, GpuGlyphQuad, GpuImageId, GpuPathId,
};

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

impl GpuCommand {
    /// Returns whether this command needs hardware lowering for a runtime shader.
    ///
    /// Image draws deliberately return `false`: their paint uses alpha, color
    /// filtering, and blending, but does not sample the paint source shader.
    pub fn requires_runtime_shader_lowering(&self) -> bool {
        let paint = match self {
            Self::FillRect { paint, .. }
            | Self::FillPath { paint, .. }
            | Self::StrokePath { paint, .. }
            | Self::DrawGlyphs { paint, .. } => Some(paint),
            Self::Clear(_)
            | Self::SaveLayer { .. }
            | Self::RestoreLayer
            | Self::DrawImage { .. } => None,
        };
        paint.is_some_and(|paint| {
            paint
                .shader_handle()
                .is_some_and(|shader| shader.as_shader().runtime().is_some())
        })
    }

    /// Returns whether this command carries a shader graph not yet lowered by native backends.
    ///
    /// Native backends lower a direct [`Shader::Image`] paint for geometric
    /// draws. Composition nodes, and image-shader glyph paints, still require
    /// graph lowering. Image draws ignore the paint source shader entirely.
    pub fn requires_shader_graph_lowering(&self) -> bool {
        let paint = match self {
            Self::FillRect { paint, .. }
            | Self::FillPath { paint, .. }
            | Self::StrokePath { paint, .. }
            | Self::DrawGlyphs { paint, .. } => Some(paint),
            Self::Clear(_)
            | Self::SaveLayer { .. }
            | Self::RestoreLayer
            | Self::DrawImage { .. } => None,
        };
        paint.is_some_and(|paint| {
            paint
                .shader_handle()
                .is_some_and(|shader| match shader.as_shader() {
                    Shader::Image(_) => matches!(self, Self::DrawGlyphs { .. }),
                    Shader::SolidColor(_) | Shader::LocalMatrix(_) | Shader::Blend(_) => true,
                    Shader::Gradient(_) | Shader::Runtime(_) => false,
                })
        })
    }
}

/// Immutable, ordered GPU command buffer with locally owned resources.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GpuCommandBuffer {
    pub(crate) commands: Vec<GpuCommand>,
    pub(crate) clips: Vec<GpuClipNode>,
    pub(crate) paths: Vec<Path>,
    pub(crate) images: Vec<Image>,
    pub(crate) glyph_atlases: Vec<GpuGlyphAtlas>,
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

    /// Resolves a glyph atlas referenced by a command.
    pub fn glyph_atlas(&self, id: GpuGlyphAtlasId) -> Option<&GpuGlyphAtlas> {
        self.glyph_atlases.get(usize::try_from(id.0).ok()?)
    }
}
