use clipper2_rust::{ClipType, FillRule as ClipperFillRule, Paths64, Point64, boolean_op_64};
use skia_error::{SkiaError, SkiaErrorCode};
use skia_geometry::Transform;
use skia_path::{FillRule, Path, PathBuilder};

use crate::{
    DEFAULT_CURVE_STEPS, FlattenedPath, FlatteningLimits, PathFlattener, TessellationError,
    TessellationErrorCode,
};

/// Boolean operation applied between subject and clip paths.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PathBooleanOp {
    /// Keeps points covered by either input.
    Union,
    /// Keeps points covered by both inputs.
    Intersection,
    /// Keeps subject points not covered by the clip.
    Difference,
    /// Keeps points covered by exactly one input.
    Xor,
}

/// Resource ceilings for one path boolean operation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PathBooleanLimits {
    max_input_contours: usize,
    max_input_points: usize,
    max_output_contours: usize,
    max_output_points: usize,
    curve_steps: u32,
}

impl PathBooleanLimits {
    /// Creates positive input, output, and curve-resolution ceilings.
    pub fn new(
        max_input_contours: usize,
        max_input_points: usize,
        max_output_contours: usize,
        max_output_points: usize,
        curve_steps: u32,
    ) -> Result<Self, SkiaError> {
        if max_input_contours == 0
            || max_input_points == 0
            || max_output_contours == 0
            || max_output_points == 0
            || curve_steps == 0
        {
            return Err(SkiaError::new(SkiaErrorCode::InvalidLimits));
        }
        Ok(Self {
            max_input_contours,
            max_input_points,
            max_output_contours,
            max_output_points,
            curve_steps,
        })
    }
}

impl Default for PathBooleanLimits {
    fn default() -> Self {
        Self {
            max_input_contours: 4_096,
            max_input_points: 1_048_576,
            max_output_contours: 4_096,
            max_output_points: 1_048_576,
            curve_steps: DEFAULT_CURVE_STEPS,
        }
    }
}

/// Applies an integer-coordinate boolean operation to two transformed paths.
///
/// Curves are flattened with the configured fixed resolution. An empty result
/// is returned as `None`; a non-empty result is an ordinary path intended for
/// [`FillRule::NonZero`] filling.
pub fn path_boolean(
    subject: &Path,
    clip: &Path,
    operation: PathBooleanOp,
    input_rule: FillRule,
    transform: Transform,
    limits: PathBooleanLimits,
) -> Result<Option<Path>, SkiaError> {
    let subjects = flatten_paths(subject, transform, limits)?;
    let clips = flatten_paths(clip, transform, limits)?;
    let result = boolean_op_64(
        match operation {
            PathBooleanOp::Union => ClipType::Union,
            PathBooleanOp::Intersection => ClipType::Intersection,
            PathBooleanOp::Difference => ClipType::Difference,
            PathBooleanOp::Xor => ClipType::Xor,
        },
        match input_rule {
            FillRule::EvenOdd => ClipperFillRule::EvenOdd,
            FillRule::NonZero => ClipperFillRule::NonZero,
        },
        &subjects,
        &clips,
    );
    paths_to_path(result, limits)
}

fn flatten_paths(
    path: &Path,
    transform: Transform,
    limits: PathBooleanLimits,
) -> Result<Paths64, SkiaError> {
    let flattening = FlatteningLimits::new(
        limits.max_input_contours,
        limits.max_input_points,
        limits.curve_steps,
    )
    .map_err(map_tessellation_error)?;
    let flattened = PathFlattener::new(flattening)
        .flatten(path, transform)
        .map_err(map_tessellation_error)?;
    flattened_to_paths(flattened)
}

fn flattened_to_paths(flattened: FlattenedPath) -> Result<Paths64, SkiaError> {
    let mut paths = Paths64::new();
    paths
        .try_reserve_exact(flattened.contours().len())
        .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
    for contour in flattened.contours() {
        if contour.points().len() < 3 {
            continue;
        }
        let mut points = Vec::new();
        points
            .try_reserve_exact(contour.points().len())
            .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
        for point in contour.points() {
            points.push(Point64::new(
                i64::from(point.x().bits()),
                i64::from(point.y().bits()),
            ));
        }
        paths.push(points);
    }
    if paths.is_empty() {
        return Err(SkiaError::new(SkiaErrorCode::InvalidPath));
    }
    Ok(paths)
}

fn paths_to_path(mut paths: Paths64, limits: PathBooleanLimits) -> Result<Option<Path>, SkiaError> {
    paths.retain(|path| path.len() >= 3);
    if paths.is_empty() {
        return Ok(None);
    }
    if paths.len() > limits.max_output_contours {
        return Err(SkiaError::new(SkiaErrorCode::ResourceLimit));
    }
    let point_count = paths.iter().try_fold(0_usize, |count, path| {
        count
            .checked_add(path.len())
            .ok_or(SkiaError::new(SkiaErrorCode::ResourceLimit))
    })?;
    if point_count > limits.max_output_points {
        return Err(SkiaError::new(SkiaErrorCode::ResourceLimit));
    }
    let capacity = point_count
        .checked_add(paths.len())
        .ok_or(SkiaError::new(SkiaErrorCode::ResourceLimit))?;
    let mut builder = PathBuilder::new(capacity)?;
    for contour in paths {
        let first = to_point(contour[0])?;
        builder.move_to(first)?;
        for point in contour.into_iter().skip(1) {
            builder.line_to(to_point(point)?)?;
        }
        builder.close()?;
    }
    builder.finish().map(Some)
}

fn to_point(point: Point64) -> Result<skia_geometry::Point, SkiaError> {
    Ok(skia_geometry::Point::new(
        skia_geometry::Scalar::from_bits(
            i32::try_from(point.x).map_err(|_| SkiaError::new(SkiaErrorCode::NumericOverflow))?,
        ),
        skia_geometry::Scalar::from_bits(
            i32::try_from(point.y).map_err(|_| SkiaError::new(SkiaErrorCode::NumericOverflow))?,
        ),
    ))
}

fn map_tessellation_error(error: TessellationError) -> SkiaError {
    let code = match error.code() {
        TessellationErrorCode::InvalidLimits => SkiaErrorCode::InvalidLimits,
        TessellationErrorCode::NumericOverflow => SkiaErrorCode::NumericOverflow,
        TessellationErrorCode::InvalidPath | TessellationErrorCode::UnsupportedTopology => {
            SkiaErrorCode::InvalidPath
        }
        TessellationErrorCode::ResourceLimit => SkiaErrorCode::ResourceLimit,
        TessellationErrorCode::AllocationFailed => SkiaErrorCode::AllocationFailed,
    };
    SkiaError::new(code)
}
