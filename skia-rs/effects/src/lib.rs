//! Built-in backend-neutral drawing effects.
//!
//! `skia-core` owns the stable effect value and extension contracts used by
//! paints and display lists. This crate supplies concrete path effects and
//! factory namespaces for the built-in shader and filter values. Execution
//! remains in tessellation and the CPU/GPU backends.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod factories;
mod path_effect;

pub use factories::{color_filters, image_filters, shaders};
pub use path_effect::{
    ComposePathEffect, CornerPathEffect, DashPathEffect, DiscretePathEffect, SumPathEffect,
    TrimPathEffect, corner_path, dash_path, discrete_path, trim_path,
};
pub use skia_core::{
    ColorFilter, ColorMatrix, Gradient, GradientGeometry, GradientStop, ImageFilter, PathEffect,
    PathEffectLimits, TileMode, apply_path_effect, compose_path_effects,
};
