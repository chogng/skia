use skia_core::{PathVerb, Point, Scalar, SkiaError, Transform};
use skia_gpu::{GpuClipGeometry, GpuClipNode, GpuCommandBuffer};

use crate::{MetalError, MetalErrorCode};

const CURVE_STEPS: i64 = 16;

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
            path_edges(path.verbs(), node.transform(), &mut output)?;
        }
    }
    if output.is_empty() {
        return Err(unsupported_command());
    }
    Ok(output)
}

fn path_edges(
    verbs: &[PathVerb],
    transform: Transform,
    output: &mut Vec<ClipEdge>,
) -> Result<(), MetalError> {
    let mut first = None;
    let mut current = None;
    for verb in verbs {
        match *verb {
            PathVerb::MoveTo(point) => {
                close_contour(output, current, first)?;
                let point = transform_point(transform, point)?;
                first = Some(point);
                current = Some(point);
            }
            PathVerb::LineTo(point) => {
                let start = current.ok_or_else(unsupported_command)?;
                let end = transform_point(transform, point)?;
                push_edge(output, start, end)?;
                current = Some(end);
            }
            PathVerb::QuadTo(control, end) => {
                let start = current.ok_or_else(unsupported_command)?;
                let control = transform_point(transform, control)?;
                let end = transform_point(transform, end)?;
                current = Some(flatten_quad(output, start, control, end)?);
            }
            PathVerb::ConicTo(control, end, weight) => {
                let start = current.ok_or_else(unsupported_command)?;
                let control = transform_point(transform, control)?;
                let end = transform_point(transform, end)?;
                current = Some(flatten_conic(output, start, control, end, weight.bits())?);
            }
            PathVerb::CubicTo(first_control, second_control, end) => {
                let start = current.ok_or_else(unsupported_command)?;
                let first_control = transform_point(transform, first_control)?;
                let second_control = transform_point(transform, second_control)?;
                let end = transform_point(transform, end)?;
                current = Some(flatten_cubic(
                    output,
                    start,
                    first_control,
                    second_control,
                    end,
                )?);
            }
            PathVerb::Close => {
                close_contour(output, current, first)?;
                first = None;
                current = None;
            }
        }
    }
    close_contour(output, current, first)
}

fn close_contour(
    output: &mut Vec<ClipEdge>,
    current: Option<Point>,
    first: Option<Point>,
) -> Result<(), MetalError> {
    if let (Some(current), Some(first)) = (current, first)
        && current != first
    {
        push_edge(output, current, first)?;
    }
    Ok(())
}

fn flatten_quad(
    output: &mut Vec<ClipEdge>,
    start: Point,
    control: Point,
    end: Point,
) -> Result<Point, MetalError> {
    let mut previous = start;
    for step in 1..=CURVE_STEPS {
        let next = Point::new(
            bezier2(start.x(), control.x(), end.x(), step)?,
            bezier2(start.y(), control.y(), end.y(), step)?,
        );
        push_edge(output, previous, next)?;
        previous = next;
    }
    Ok(previous)
}

fn flatten_conic(
    output: &mut Vec<ClipEdge>,
    start: Point,
    control: Point,
    end: Point,
    weight_bits: i32,
) -> Result<Point, MetalError> {
    let mut previous = start;
    for step in 1..=CURVE_STEPS {
        let next = Point::new(
            conic_coordinate(start.x(), control.x(), end.x(), weight_bits, step)?,
            conic_coordinate(start.y(), control.y(), end.y(), weight_bits, step)?,
        );
        push_edge(output, previous, next)?;
        previous = next;
    }
    Ok(previous)
}

fn flatten_cubic(
    output: &mut Vec<ClipEdge>,
    start: Point,
    first_control: Point,
    second_control: Point,
    end: Point,
) -> Result<Point, MetalError> {
    let mut previous = start;
    for step in 1..=CURVE_STEPS {
        let next = Point::new(
            bezier3(
                start.x(),
                first_control.x(),
                second_control.x(),
                end.x(),
                step,
            )?,
            bezier3(
                start.y(),
                first_control.y(),
                second_control.y(),
                end.y(),
                step,
            )?,
        );
        push_edge(output, previous, next)?;
        previous = next;
    }
    Ok(previous)
}

fn push_edge(output: &mut Vec<ClipEdge>, start: Point, end: Point) -> Result<(), MetalError> {
    output.try_reserve(1).map_err(|_| submission_failed())?;
    output.push(edge(start, end));
    Ok(())
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
    transform.map_point(point).map_err(map_geometry_error)
}

fn bezier2(start: Scalar, control: Scalar, end: Scalar, step: i64) -> Result<Scalar, MetalError> {
    let inverse = CURVE_STEPS - step;
    let value = i128::from(start.bits()) * i128::from(inverse * inverse)
        + i128::from(control.bits()) * i128::from(2 * inverse * step)
        + i128::from(end.bits()) * i128::from(step * step);
    rounded_scalar(value, i128::from(CURVE_STEPS * CURVE_STEPS))
}

fn conic_coordinate(
    start: Scalar,
    control: Scalar,
    end: Scalar,
    weight_bits: i32,
    step: i64,
) -> Result<Scalar, MetalError> {
    let inverse = CURVE_STEPS - step;
    let start_weight = i128::from(inverse * inverse) * i128::from(1_i64 << 16);
    let control_weight = i128::from(2 * inverse * step) * i128::from(weight_bits);
    let end_weight = i128::from(step * step) * i128::from(1_i64 << 16);
    let denominator = start_weight + control_weight + end_weight;
    let numerator = i128::from(start.bits()) * start_weight
        + i128::from(control.bits()) * control_weight
        + i128::from(end.bits()) * end_weight;
    rounded_scalar(numerator, denominator)
}

fn bezier3(
    start: Scalar,
    first_control: Scalar,
    second_control: Scalar,
    end: Scalar,
    step: i64,
) -> Result<Scalar, MetalError> {
    let inverse = CURVE_STEPS - step;
    let value = i128::from(start.bits()) * i128::from(inverse * inverse * inverse)
        + i128::from(first_control.bits()) * i128::from(3 * inverse * inverse * step)
        + i128::from(second_control.bits()) * i128::from(3 * inverse * step * step)
        + i128::from(end.bits()) * i128::from(step * step * step);
    rounded_scalar(value, i128::from(CURVE_STEPS * CURVE_STEPS * CURVE_STEPS))
}

fn rounded_scalar(value: i128, divisor: i128) -> Result<Scalar, MetalError> {
    let half = divisor / 2;
    let value = if value >= 0 {
        value.checked_add(half).ok_or_else(submission_failed)? / divisor
    } else {
        -(value
            .checked_neg()
            .and_then(|value| value.checked_add(half))
            .ok_or_else(submission_failed)?
            / divisor)
    };
    i32::try_from(value)
        .map(Scalar::from_bits)
        .map_err(|_| submission_failed())
}

fn scalar_to_f32(value: Scalar) -> f32 {
    value.bits() as f32 / 65_536.0
}

fn map_geometry_error(_: SkiaError) -> MetalError {
    submission_failed()
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

    use super::clip_edges;

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
}
