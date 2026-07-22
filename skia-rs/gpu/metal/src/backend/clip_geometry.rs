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
mod tests {
    use skia_core::{
        ClipOp, Color, ConicWeight, FillRule, Paint, PathBuilder, Point, Rect, Scalar,
    };
    use skia_gpu::{GpuCommand, GpuCommandEncoder};

    use super::{clip_edges, contour_edge_count};

    fn point(x: i32, y: i32) -> Point {
        Point::new(
            Scalar::from_i32(x).expect("x"),
            Scalar::from_i32(y).expect("y"),
        )
    }

    #[test]
    fn clip_edges_flatten_every_curve_family_with_fixed_steps() {
        let mut path = PathBuilder::new(5).expect("path limits");
        path.move_to(point(0, 0)).expect("move");
        path.quad_to(point(1, 2), point(2, 0)).expect("quad");
        path.conic_to(point(3, 2), point(4, 0), ConicWeight::ONE)
            .expect("conic");
        path.cubic_to(point(5, 2), point(6, 2), point(7, 0))
            .expect("cubic");
        path.close().expect("close");
        let mut encoder = GpuCommandEncoder::new(1).expect("encoder");
        let path = encoder
            .add_path(path.finish().expect("path"))
            .expect("path");
        encoder
            .clip_path(path, FillRule::NonZero, ClipOp::Intersect)
            .expect("clip");
        encoder
            .fill_rect(
                Rect::new(
                    Scalar::ZERO,
                    Scalar::ZERO,
                    Scalar::from_i32(1).expect("right"),
                    Scalar::from_i32(1).expect("bottom"),
                )
                .expect("rect"),
                Paint::new(Color::WHITE),
            )
            .expect("draw");
        let commands = encoder.finish();
        let GpuCommand::FillRect {
            clip: Some(clip), ..
        } = commands.commands()[0]
        else {
            panic!("expected complex clip");
        };
        let edges = clip_edges(&commands, commands.clip_node(clip).expect("clip node"))
            .expect("flattened edges");

        assert_eq!(edges.len(), 49);
    }

    #[test]
    fn explicit_return_to_start_does_not_add_a_duplicate_closing_edge() {
        assert_eq!(
            contour_edge_count(&[point(0, 0), point(2, 0), point(0, 0)]),
            2
        );
    }
}
