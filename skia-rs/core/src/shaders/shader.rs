use super::{Gradient, ImageShader, RuntimeShader};
use crate::paint::{BlendMode, Color};
use crate::sampling::SamplingOptions;
use skia_error::{SkiaError, SkiaErrorCode};
use skia_geometry::{Point, Transform};
use skia_image::Image;
use std::sync::Arc;

/// Backend-neutral source program currently implemented by paints.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum Shader {
    /// Returns one constant straight-alpha color at every coordinate.
    SolidColor(Color),
    /// Evaluates a bounded local-space gradient.
    Gradient(Gradient),
    /// Evaluates a bounded local-space runtime program on CPU and software GPU.
    Runtime(RuntimeShader),
    /// Samples an immutable image in local pixel-coordinate space.
    Image(ImageShader),
    /// Evaluates a child shader through an additional local-space transform.
    LocalMatrix(LocalMatrixShader),
    /// Composites two child shaders at the same local coordinate.
    Blend(BlendShader),
}

impl Shader {
    /// Maximum nesting depth accepted by a composed shader graph.
    pub const MAX_GRAPH_DEPTH: usize = 16;
    /// Maximum total node count accepted by a composed shader graph.
    pub const MAX_GRAPH_NODES: usize = 64;

    /// Creates a constant-color shader.
    pub const fn solid_color(color: Color) -> Self {
        Self::SolidColor(color)
    }

    /// Creates an image shader with explicit reconstruction and tiling policies.
    pub fn image(
        image: Image,
        sampling: SamplingOptions,
        x_tile_mode: super::TileMode,
        y_tile_mode: super::TileMode,
    ) -> Result<Self, SkiaError> {
        ImageShader::new(image, sampling, x_tile_mode, y_tile_mode).map(Self::Image)
    }

    /// Creates a local-matrix wrapper after validating its transform and graph limits.
    pub fn local_matrix(shader: ShaderHandle, local_matrix: Transform) -> Result<Self, SkiaError> {
        LocalMatrixShader::new(shader, local_matrix).map(Self::LocalMatrix)
    }

    /// Creates a bounded two-child blend shader.
    pub fn blend(
        source: ShaderHandle,
        destination: ShaderHandle,
        mode: BlendMode,
    ) -> Result<Self, SkiaError> {
        BlendShader::new(source, destination, mode).map(Self::Blend)
    }

    /// Returns the constant color representation when this shader has one.
    pub const fn solid(&self) -> Option<Color> {
        match self {
            Self::SolidColor(color) => Some(*color),
            Self::Gradient(_)
            | Self::Runtime(_)
            | Self::Image(_)
            | Self::LocalMatrix(_)
            | Self::Blend(_) => None,
        }
    }

    /// Returns the gradient representation when this shader has one.
    pub const fn gradient(&self) -> Option<Gradient> {
        match self {
            Self::Gradient(gradient) => Some(*gradient),
            Self::SolidColor(_)
            | Self::Runtime(_)
            | Self::Image(_)
            | Self::LocalMatrix(_)
            | Self::Blend(_) => None,
        }
    }

    /// Borrows the runtime representation when this shader has one.
    pub const fn runtime(&self) -> Option<&RuntimeShader> {
        match self {
            Self::Runtime(runtime) => Some(runtime),
            Self::SolidColor(_)
            | Self::Gradient(_)
            | Self::Image(_)
            | Self::LocalMatrix(_)
            | Self::Blend(_) => None,
        }
    }

    /// Borrows the image representation when this shader has one.
    pub const fn image_shader(&self) -> Option<&ImageShader> {
        match self {
            Self::Image(shader) => Some(shader),
            Self::SolidColor(_)
            | Self::Gradient(_)
            | Self::Runtime(_)
            | Self::LocalMatrix(_)
            | Self::Blend(_) => None,
        }
    }

    /// Borrows the local-matrix representation when this shader has one.
    pub const fn local_matrix_shader(&self) -> Option<&LocalMatrixShader> {
        match self {
            Self::LocalMatrix(shader) => Some(shader),
            Self::SolidColor(_)
            | Self::Gradient(_)
            | Self::Runtime(_)
            | Self::Image(_)
            | Self::Blend(_) => None,
        }
    }

    /// Borrows the blend representation when this shader has one.
    pub const fn blend_shader(&self) -> Option<&BlendShader> {
        match self {
            Self::Blend(shader) => Some(shader),
            Self::SolidColor(_)
            | Self::Gradient(_)
            | Self::Runtime(_)
            | Self::Image(_)
            | Self::LocalMatrix(_) => None,
        }
    }

    /// Evaluates this shader at one local-space point.
    pub fn sample(&self, point: Point) -> Result<Color, SkiaError> {
        match self {
            Self::SolidColor(color) => Ok(*color),
            Self::Gradient(gradient) => gradient.sample(point),
            Self::Runtime(runtime) => runtime.sample(point),
            Self::Image(shader) => shader.sample(point),
            Self::LocalMatrix(shader) => shader.sample(point),
            Self::Blend(shader) => shader.sample(point),
        }
    }

    fn graph_size(&self) -> (usize, usize) {
        match self {
            Self::SolidColor(_) | Self::Gradient(_) | Self::Runtime(_) | Self::Image(_) => (1, 1),
            Self::LocalMatrix(shader) => {
                let (depth, nodes) = shader.shader.as_shader().graph_size();
                (depth.saturating_add(1), nodes.saturating_add(1))
            }
            Self::Blend(shader) => {
                let (source_depth, source_nodes) = shader.source.as_shader().graph_size();
                let (destination_depth, destination_nodes) =
                    shader.destination.as_shader().graph_size();
                (
                    source_depth.max(destination_depth).saturating_add(1),
                    source_nodes
                        .saturating_add(destination_nodes)
                        .saturating_add(1),
                )
            }
        }
    }
}

/// One child shader evaluated through the inverse of an additional local matrix.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct LocalMatrixShader {
    shader: ShaderHandle,
    local_matrix: Transform,
    inverse: Transform,
}

impl LocalMatrixShader {
    /// Validates a non-singular matrix and the resulting graph resource limits.
    pub fn new(shader: ShaderHandle, local_matrix: Transform) -> Result<Self, SkiaError> {
        let inverse = local_matrix.inverse()?;
        let (depth, nodes) = shader.as_shader().graph_size();
        validate_graph_size(depth.saturating_add(1), nodes.saturating_add(1))?;
        Ok(Self {
            shader,
            local_matrix,
            inverse,
        })
    }

    /// Borrows the wrapped child shader.
    pub const fn shader(&self) -> &ShaderHandle {
        &self.shader
    }

    /// Returns the child-to-parent local transform.
    pub const fn local_matrix(&self) -> Transform {
        self.local_matrix
    }

    /// Evaluates the child after mapping the parent coordinate into child space.
    pub fn sample(&self, point: Point) -> Result<Color, SkiaError> {
        self.shader
            .as_shader()
            .sample(self.inverse.map_point(point)?)
    }
}

/// Two child shaders composited with one backend-neutral blend mode.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct BlendShader {
    source: ShaderHandle,
    destination: ShaderHandle,
    mode: BlendMode,
}

impl BlendShader {
    /// Validates the combined graph and retains both child shaders.
    pub fn new(
        source: ShaderHandle,
        destination: ShaderHandle,
        mode: BlendMode,
    ) -> Result<Self, SkiaError> {
        let (source_depth, source_nodes) = source.as_shader().graph_size();
        let (destination_depth, destination_nodes) = destination.as_shader().graph_size();
        let depth = source_depth.max(destination_depth).saturating_add(1);
        let nodes = source_nodes
            .checked_add(destination_nodes)
            .and_then(|nodes| nodes.checked_add(1))
            .ok_or(SkiaError::new(SkiaErrorCode::InvalidLimits))?;
        validate_graph_size(depth, nodes)?;
        Ok(Self {
            source,
            destination,
            mode,
        })
    }

    /// Borrows the source child.
    pub const fn source(&self) -> &ShaderHandle {
        &self.source
    }

    /// Borrows the destination child.
    pub const fn destination(&self) -> &ShaderHandle {
        &self.destination
    }

    /// Returns the child compositing mode.
    pub const fn mode(&self) -> BlendMode {
        self.mode
    }

    /// Evaluates and composites both children at one coordinate.
    pub fn sample(&self, point: Point) -> Result<Color, SkiaError> {
        let source = self.source.as_shader().sample(point)?;
        let destination = self.destination.as_shader().sample(point)?;
        Ok(source.composite(destination, self.mode))
    }
}

/// Shared, immutable ownership of one backend-neutral [`Shader`].
///
/// A handle lets paints and display lists reuse the same source program while
/// keeping source evaluation extensible beyond inline gradient values.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ShaderHandle {
    shader: Arc<Shader>,
}

impl ShaderHandle {
    /// Wraps one shader in shared ownership.
    pub fn new(shader: Shader) -> Self {
        Self {
            shader: Arc::new(shader),
        }
    }

    /// Wraps one constant-color shader in shared ownership.
    pub fn from_color(color: Color) -> Self {
        Self::new(Shader::solid_color(color))
    }

    /// Wraps one gradient shader in shared ownership.
    pub fn from_gradient(gradient: Gradient) -> Self {
        Self::new(Shader::Gradient(gradient))
    }

    /// Wraps one bounded runtime shader in shared ownership.
    pub fn from_runtime(runtime: RuntimeShader) -> Self {
        Self::new(Shader::Runtime(runtime))
    }

    /// Creates an image shader with explicit reconstruction and tiling policies.
    pub fn from_image(
        image: Image,
        sampling: SamplingOptions,
        x_tile_mode: super::TileMode,
        y_tile_mode: super::TileMode,
    ) -> Result<Self, SkiaError> {
        Shader::image(image, sampling, x_tile_mode, y_tile_mode).map(Self::new)
    }

    /// Wraps this shader in an additional local-space transform.
    pub fn with_local_matrix(self, local_matrix: Transform) -> Result<Self, SkiaError> {
        Shader::local_matrix(self, local_matrix).map(Self::new)
    }

    /// Composites two retained shaders with one blend mode.
    pub fn blend(source: Self, destination: Self, mode: BlendMode) -> Result<Self, SkiaError> {
        Shader::blend(source, destination, mode).map(Self::new)
    }

    /// Returns the backend-neutral shader value.
    pub fn shader(&self) -> Shader {
        self.shader.as_ref().clone()
    }

    /// Borrows the backend-neutral shader value.
    pub fn as_shader(&self) -> &Shader {
        &self.shader
    }
}

impl From<Gradient> for ShaderHandle {
    fn from(gradient: Gradient) -> Self {
        Self::from_gradient(gradient)
    }
}

impl From<RuntimeShader> for ShaderHandle {
    fn from(runtime: RuntimeShader) -> Self {
        Self::from_runtime(runtime)
    }
}

impl From<Color> for ShaderHandle {
    fn from(color: Color) -> Self {
        Self::from_color(color)
    }
}

fn validate_graph_size(depth: usize, nodes: usize) -> Result<(), SkiaError> {
    if depth > Shader::MAX_GRAPH_DEPTH || nodes > Shader::MAX_GRAPH_NODES {
        return Err(SkiaError::new(SkiaErrorCode::InvalidLimits));
    }
    Ok(())
}
