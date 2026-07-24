use super::{Gradient, RuntimeShader};
use crate::paint::Color;
use skia_error::SkiaError;
use skia_geometry::Point;
use std::sync::Arc;

/// Backend-neutral source program currently implemented by paints.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum Shader {
    /// Evaluates a bounded local-space gradient.
    Gradient(Gradient),
    /// Evaluates a bounded local-space runtime program on CPU and software GPU.
    Runtime(RuntimeShader),
}

impl Shader {
    /// Returns the gradient representation when this shader has one.
    pub const fn gradient(&self) -> Option<Gradient> {
        match self {
            Self::Gradient(gradient) => Some(*gradient),
            Self::Runtime(_) => None,
        }
    }

    /// Borrows the runtime representation when this shader has one.
    pub const fn runtime(&self) -> Option<&RuntimeShader> {
        match self {
            Self::Gradient(_) => None,
            Self::Runtime(runtime) => Some(runtime),
        }
    }

    /// Evaluates this shader at one local-space point.
    pub fn sample(&self, point: Point) -> Result<Color, SkiaError> {
        match self {
            Self::Gradient(gradient) => gradient.sample(point),
            Self::Runtime(runtime) => runtime.sample(point),
        }
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

    /// Wraps one gradient shader in shared ownership.
    pub fn from_gradient(gradient: Gradient) -> Self {
        Self::new(Shader::Gradient(gradient))
    }

    /// Wraps one bounded runtime shader in shared ownership.
    pub fn from_runtime(runtime: RuntimeShader) -> Self {
        Self::new(Shader::Runtime(runtime))
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
