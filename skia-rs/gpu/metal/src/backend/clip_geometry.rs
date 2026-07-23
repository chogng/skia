use skia_core::{Path, Point, Scalar, Transform};
use skia_gpu::{GpuClipGeometry, GpuClipNode, GpuCommandBuffer};
use skia_tessellation::{
    DEFAULT_CURVE_STEPS, FlatteningLimits, PathFlattener, TessellationErrorCode,
};

use crate::{MetalError, MetalErrorCode};

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct ClipEdge {
    pub(crate) start: [f32; 2],
    pub(crate) end: [f32; 2],
}

pub(crate) fn clip_edges(
    commands: &GpuCommandBuffer,
    node: GpuClipNode,
) -> Result<Vec<ClipEdge>, MetalError> {
    let mut output = Vec::new();
    match node.geometry() {
        GpuClipGeometry::Rect(rect) => {
            let transform = node.transform();
            let points = [
                transform_point(transform, Point::new(rect.left(), rect.top()))?,
                transform_point(transform, Point::new(rect.right(), rect.top()))?,
                transform_point(transform, Point::new(rect.right(), rect.bottom()))?,
                transform_point(transform, Point::new(rect.left(), rect.bottom()))?,
            ];
            output
                .try_reserve_exact(4)
                .map_err(|_| submission_failed())?;
            for index in 0..points.len() {
                output.push(edge(points[index], points[(index + 1) % points.len()]));
            }
        }
        GpuClipGeometry::Path { path, .. } => {
            let path = commands.path(path).ok_or_else(unsupported_command)?;
            append_path_edges(path, node.transform(), &mut output)?;
        }
    }
    if output.is_empty() {
        return Err(unsupported_command());
    }
    Ok(output)
}

/// Flattens a fill path into target-space edges for one Metal mask pass.
pub(crate) fn path_edges(path: &Path, transform: Transform) -> Result<Vec<ClipEdge>, MetalError> {
    let mut output = Vec::new();
    append_path_edges(path, transform, &mut output)?;
    if output.is_empty() {
        return Err(unsupported_command());
    }
    Ok(output)
}

fn append_path_edges(
    path: &Path,
    transform: Transform,
    output: &mut Vec<ClipEdge>,
) -> Result<(), MetalError> {
    let limits =
        FlatteningLimits::for_path(path, DEFAULT_CURVE_STEPS).map_err(map_tessellation_error)?;
    let flattened = PathFlattener::new(limits)
        .flatten(path, transform)
        .map_err(map_tessellation_error)?;
    let edge_count = flattened
        .contours()
        .iter()
        .try_fold(0_usize, |count, contour| {
            count.checked_add(contour_edge_count(contour.points()))
        })
        .ok_or_else(submission_failed)?;
    output
        .try_reserve_exact(edge_count)
        .map_err(|_| submission_failed())?;
    for contour in flattened.contours() {
        let points = contour.points();
        if points.len() < 2 {
            continue;
        }
        for pair in points.windows(2) {
            output.push(edge(pair[0], pair[1]));
        }
        if points.last() != points.first() {
            output.push(edge(points[points.len() - 1], points[0]));
        }
    }
    Ok(())
}

fn contour_edge_count(points: &[Point]) -> usize {
    match points.len() {
        0 | 1 => 0,
        count if points.first() == points.last() => count - 1,
        count => count,
    }
}

fn edge(start: Point, end: Point) -> ClipEdge {
    ClipEdge {
        start: point_to_f32(start),
        end: point_to_f32(end),
    }
}

fn point_to_f32(point: Point) -> [f32; 2] {
    [scalar_to_f32(point.x()), scalar_to_f32(point.y())]
}

fn transform_point(transform: Transform, point: Point) -> Result<Point, MetalError> {
    transform.map_point(point).map_err(|_| submission_failed())
}

fn scalar_to_f32(value: Scalar) -> f32 {
    value.bits() as f32 / 65_536.0
}

fn map_tessellation_error(error: skia_tessellation::TessellationError) -> MetalError {
    match error.code() {
        TessellationErrorCode::InvalidPath | TessellationErrorCode::UnsupportedTopology => {
            unsupported_command()
        }
        TessellationErrorCode::InvalidLimits
        | TessellationErrorCode::NumericOverflow
        | TessellationErrorCode::ResourceLimit
        | TessellationErrorCode::AllocationFailed => submission_failed(),
    }
}

fn unsupported_command() -> MetalError {
    MetalError::new(MetalErrorCode::UnsupportedCommand)
}

fn submission_failed() -> MetalError {
    MetalError::new(MetalErrorCode::SubmissionFailed)
}

#[cfg(test)]
#[path = "clip_geometry_tests.rs"]
mod tests;
