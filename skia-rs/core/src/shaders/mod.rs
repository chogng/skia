//! Backend-neutral paint-source shader semantics.
//!
//! Shader values live in `skia-core` so CPU and GPU executors consume the same
//! bounded composition graph, validated gradients and runtime programs,
//! coordinate rules, and ownership model.

mod gradient;
mod runtime;
mod shader;

pub(crate) use gradient::rounded_shift_q16;
pub use gradient::{Gradient, GradientGeometry, GradientStop, TileMode};
pub use runtime::{
    RuntimeShader, RuntimeShaderInstruction, RuntimeShaderLimits, RuntimeShaderProgram,
};
pub use shader::{BlendShader, LocalMatrixShader, Shader, ShaderHandle};
