//! Deterministic path processing and triangle conversion for drawing backends.
//!
//! Curve flattening is shared by CPU and hardware backends. The triangle-mesh
//! implementation currently accepts one closed, convex, line-only contour;
//! unsupported topology fails closed instead of silently producing a different
//! fill. Future releases extend that mesh contract with holes, non-convex
//! contours, and stroke meshes.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod boolean;
mod flatten;
mod path_effect;
mod stroke;
mod tessellator;

pub use boolean::{PathBooleanLimits, PathBooleanOp, path_boolean};
pub use flatten::{
    DEFAULT_CURVE_STEPS, FlattenedContour, FlattenedPath, FlatteningLimits, PathFlattener,
};
pub use path_effect::{
    ComposePathEffect, CornerPathEffect, DashPathEffect, DiscretePathEffect, PathEffect,
    PathEffectLimits, SumPathEffect, TrimPathEffect, apply_path_effect, compose_path_effects,
    corner_path, dash_path, discrete_path, trim_path,
};
pub use stroke::{
    StrokeMesh, StrokePiece, interpolate_stroke_segment, stroke_contains, stroke_contours_to_path,
    stroke_mesh, stroke_pieces, stroke_segment_length_bits, stroke_to_path,
};

pub use tessellator::{
    TessellationError, TessellationErrorCode, TessellationLimits, Tessellator, TriangleMesh,
};
