//! Immutable paths, fixed-point path construction, and stroke geometry.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod path;

pub use path::{
    Angle, ArcDirection, ArcStart, ConicWeight, FillRule, Path, PathBounds, PathBuilder, PathVerb,
    StrokeAlign, StrokeCap, StrokeJoin, StrokeOptions,
};
