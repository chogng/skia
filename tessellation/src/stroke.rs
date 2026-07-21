use skia_error::{SkiaError, SkiaErrorCode};
use skia_geometry::{Point, Scalar};
use skia_path::{Path, PathBuilder, StrokeCap, StrokeJoin, StrokeOptions};

use crate::{
    DEFAULT_CURVE_STEPS, FlattenedContour, FlatteningLimits, PathFlattener, TessellationError,
    TessellationErrorCode,
};

/// One continuous portion of a stroked contour after dash processing.
#[derive(Debug)]
pub struct StrokePiece {
    points: Vec<Point>,
    closed: bool,
}

/// Triangle-list geometry for one fully expanded stroke.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StrokeMesh {
    vertices: Vec<Point>,
}

impl StrokeMesh {
    /// Borrows triangle-list vertices; every consecutive three form one triangle.
    pub fn vertices(&self) -> &[Point] {
        &self.vertices
    }

    /// Converts this triangle list to a non-zero fill path.
    pub fn to_path(&self) -> Result<Path, SkiaError> {
        let triangle_count = self.vertices.len() / 3;
        let capacity = triangle_count
            .checked_mul(4)
            .ok_or(SkiaError::new(SkiaErrorCode::AllocationFailed))?;
        let mut builder = PathBuilder::new(capacity)?;
        for triangle in self.vertices.chunks_exact(3) {
            builder.move_to(triangle[0])?;
            builder.line_to(triangle[1])?;
            builder.line_to(triangle[2])?;
            builder.close()?;
        }
        builder.finish()
    }
}

impl StrokePiece {
    /// Borrows this piece's points in traversal order.
    pub fn points(&self) -> &[Point] {
        &self.points
    }

    /// Returns whether this piece joins its final point back to its first point.
    pub const fn is_closed(&self) -> bool {
        self.closed
    }
}

/// Normalizes contours and applies the configured dash pattern.
pub fn stroke_pieces(
    contours: &[FlattenedContour],
    options: &StrokeOptions,
) -> Result<Vec<StrokePiece>, SkiaError> {
    let mut pieces = Vec::new();
    for contour in contours {
        let points = normalized_stroke_points(contour)?;
        if points.len() < 2 {
            continue;
        }
        if options.dash_pattern().is_empty() {
            pieces
                .try_reserve(1)
                .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
            pieces.push(StrokePiece {
                points,
                closed: contour.is_closed(),
            });
        } else {
            let dashed = dash_contour(&points, contour.is_closed(), options)?;
            pieces
                .try_reserve(dashed.len())
                .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
            pieces.extend(dashed);
        }
    }
    Ok(pieces)
}

/// Expands a dashed, capped, and joined stroke into backend-neutral triangles.
///
/// The mesh uses a fixed round-primitive resolution so GPU backends can rasterize
/// it without repeating backend-specific dash or join logic.
pub fn stroke_mesh(
    contours: &[FlattenedContour],
    options: &StrokeOptions,
) -> Result<StrokeMesh, SkiaError> {
    const ROUND_STEPS: usize = 24;
    let pieces = stroke_pieces(contours, options)?;
    let radius = f64::from(options.width().bits()) / 131_072.0;
    let mut vertices = Vec::new();
    for piece in &pieces {
        let points = piece.points();
        if points.len() < 2 {
            continue;
        }
        let segment_count = if piece.is_closed() {
            points.len()
        } else {
            points.len() - 1
        };
        for index in 0..segment_count {
            let start = point_to_f64(points[index]);
            let end = point_to_f64(points[(index + 1) % points.len()]);
            let Some((direction, normal)) = direction_and_normal(start, end) else {
                continue;
            };
            let extend_start =
                !piece.is_closed() && index == 0 && options.cap() == StrokeCap::Square;
            let extend_end = !piece.is_closed()
                && index + 1 == segment_count
                && options.cap() == StrokeCap::Square;
            let start = if extend_start {
                subtract(start, scale(direction, radius))
            } else {
                start
            };
            let end = if extend_end {
                add(end, scale(direction, radius))
            } else {
                end
            };
            append_quad(
                &mut vertices,
                add(start, scale(normal, radius)),
                add(end, scale(normal, radius)),
                subtract(start, scale(normal, radius)),
                subtract(end, scale(normal, radius)),
            )?;
        }
        if !piece.is_closed() && options.cap() == StrokeCap::Round {
            append_circle(&mut vertices, point_to_f64(points[0]), radius, ROUND_STEPS)?;
            append_circle(
                &mut vertices,
                point_to_f64(points[points.len() - 1]),
                radius,
                ROUND_STEPS,
            )?;
        }
        let join_start = if piece.is_closed() { 0 } else { 1 };
        let join_end = if piece.is_closed() {
            points.len()
        } else {
            points.len() - 1
        };
        for index in join_start..join_end {
            let previous = point_to_f64(points[(index + points.len() - 1) % points.len()]);
            let vertex = point_to_f64(points[index]);
            let next = point_to_f64(points[(index + 1) % points.len()]);
            append_join(
                &mut vertices,
                previous,
                vertex,
                next,
                radius,
                options,
                ROUND_STEPS,
            )?;
        }
    }
    Ok(StrokeMesh { vertices })
}

/// Converts pre-flattened stroke contours into an equivalent non-zero fill path.
pub fn stroke_contours_to_path(
    contours: &[FlattenedContour],
    options: &StrokeOptions,
) -> Result<Path, SkiaError> {
    stroke_mesh(contours, options)?.to_path()
}

/// Converts a transformed stroke into an equivalent non-zero fill path.
pub fn stroke_to_path(
    path: &Path,
    options: &StrokeOptions,
    transform: skia_geometry::Transform,
) -> Result<Path, SkiaError> {
    let limits =
        FlatteningLimits::for_path(path, DEFAULT_CURVE_STEPS).map_err(map_tessellation_error)?;
    let contours = PathFlattener::new(limits)
        .flatten(path, transform)
        .map_err(map_tessellation_error)?;
    stroke_contours_to_path(contours.contours(), options)
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

fn append_join(
    output: &mut Vec<Point>,
    previous: (f64, f64),
    vertex: (f64, f64),
    next: (f64, f64),
    radius: f64,
    options: &StrokeOptions,
    round_steps: usize,
) -> Result<(), SkiaError> {
    let Some((incoming, incoming_normal)) = direction_and_normal(previous, vertex) else {
        return Ok(());
    };
    let Some((outgoing, outgoing_normal)) = direction_and_normal(vertex, next) else {
        return Ok(());
    };
    let turn = cross(incoming, outgoing);
    if turn.abs() < f64::EPSILON {
        return Ok(());
    }
    if options.join() == StrokeJoin::Round {
        return append_circle(output, vertex, radius, round_steps);
    }
    let side = if turn > 0.0 { -1.0 } else { 1.0 };
    let outer_incoming = add(vertex, scale(incoming_normal, radius * side));
    let outer_outgoing = add(vertex, scale(outgoing_normal, radius * side));
    if options.join() == StrokeJoin::Bevel {
        return append_triangle(output, vertex, outer_incoming, outer_outgoing);
    }
    let denominator = cross(incoming, outgoing);
    if denominator.abs() < f64::EPSILON {
        return Ok(());
    }
    let delta = subtract(outer_outgoing, outer_incoming);
    let t = cross(delta, outgoing) / denominator;
    let miter = add(outer_incoming, scale(incoming, t));
    let limit = radius * f64::from(options.miter_limit().bits()) / 65_536.0;
    if distance_squared(vertex, miter) > limit * limit {
        return append_triangle(output, vertex, outer_incoming, outer_outgoing);
    }
    append_triangle(output, outer_incoming, miter, vertex)?;
    append_triangle(output, vertex, miter, outer_outgoing)
}

fn append_quad(
    output: &mut Vec<Point>,
    first: (f64, f64),
    second: (f64, f64),
    third: (f64, f64),
    fourth: (f64, f64),
) -> Result<(), SkiaError> {
    append_triangle(output, first, second, third)?;
    append_triangle(output, second, fourth, third)
}

fn append_circle(
    output: &mut Vec<Point>,
    center: (f64, f64),
    radius: f64,
    steps: usize,
) -> Result<(), SkiaError> {
    for index in 0..steps {
        let angle = std::f64::consts::TAU * index as f64 / steps as f64;
        let next = std::f64::consts::TAU * (index + 1) as f64 / steps as f64;
        append_triangle(
            output,
            center,
            (
                center.0 + radius * angle.cos(),
                center.1 + radius * angle.sin(),
            ),
            (
                center.0 + radius * next.cos(),
                center.1 + radius * next.sin(),
            ),
        )?;
    }
    Ok(())
}

fn append_triangle(
    output: &mut Vec<Point>,
    first: (f64, f64),
    second: (f64, f64),
    third: (f64, f64),
) -> Result<(), SkiaError> {
    let area = cross(subtract(second, first), subtract(third, first));
    if area.abs() < f64::EPSILON {
        return Ok(());
    }
    output
        .try_reserve(3)
        .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
    output.push(point_from_f64(first)?);
    if area < 0.0 {
        output.push(point_from_f64(second)?);
        output.push(point_from_f64(third)?);
    } else {
        output.push(point_from_f64(third)?);
        output.push(point_from_f64(second)?);
    }
    Ok(())
}

fn point_to_f64(point: Point) -> (f64, f64) {
    (
        f64::from(point.x().bits()) / 65_536.0,
        f64::from(point.y().bits()) / 65_536.0,
    )
}

fn point_from_f64(point: (f64, f64)) -> Result<Point, SkiaError> {
    let scalar = |value: f64| {
        if !value.is_finite() {
            return Err(SkiaError::new(SkiaErrorCode::NumericOverflow));
        }
        let scaled = (value * 65_536.0).round();
        if scaled < f64::from(i32::MIN) || scaled > f64::from(i32::MAX) {
            return Err(SkiaError::new(SkiaErrorCode::NumericOverflow));
        }
        Ok(Scalar::from_bits(scaled as i32))
    };
    Ok(Point::new(scalar(point.0)?, scalar(point.1)?))
}

fn direction_and_normal(start: (f64, f64), end: (f64, f64)) -> Option<((f64, f64), (f64, f64))> {
    let delta = subtract(end, start);
    let length = (delta.0 * delta.0 + delta.1 * delta.1).sqrt();
    (length > 0.0).then(|| {
        let direction = (delta.0 / length, delta.1 / length);
        (direction, (-direction.1, direction.0))
    })
}

fn add(first: (f64, f64), second: (f64, f64)) -> (f64, f64) {
    (first.0 + second.0, first.1 + second.1)
}

fn subtract(first: (f64, f64), second: (f64, f64)) -> (f64, f64) {
    (first.0 - second.0, first.1 - second.1)
}

fn scale(value: (f64, f64), factor: f64) -> (f64, f64) {
    (value.0 * factor, value.1 * factor)
}

fn cross(first: (f64, f64), second: (f64, f64)) -> f64 {
    first.0 * second.1 - first.1 * second.0
}

fn distance_squared(first: (f64, f64), second: (f64, f64)) -> f64 {
    let delta = subtract(first, second);
    delta.0 * delta.0 + delta.1 * delta.1
}

fn normalized_stroke_points(contour: &FlattenedContour) -> Result<Vec<Point>, SkiaError> {
    let mut points = Vec::new();
    points
        .try_reserve_exact(contour.points().len())
        .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
    for point in contour.points() {
        if points.last() != Some(point) {
            points.push(*point);
        }
    }
    if contour.is_closed() && points.len() > 1 && points.first() == points.last() {
        points.pop();
    }
    Ok(points)
}

fn dash_contour(
    points: &[Point],
    closed: bool,
    options: &StrokeOptions,
) -> Result<Vec<StrokePiece>, SkiaError> {
    let pattern = options.dash_pattern();
    let mut pattern_index = 0_usize;
    let mut phase = i64::from(options.dash_phase().bits());
    while phase >= i64::from(pattern[pattern_index].bits()) {
        phase -= i64::from(pattern[pattern_index].bits());
        pattern_index = (pattern_index + 1) % pattern.len();
    }
    let starts_on = pattern_index.is_multiple_of(2);
    let mut pattern_remaining = i64::from(pattern[pattern_index].bits()) - phase;
    let segment_count = if closed {
        points.len()
    } else {
        points.len() - 1
    };
    let contour_start = points[0];
    let mut current = Vec::new();
    let mut pieces = Vec::new();

    for index in 0..segment_count {
        let start = points[index];
        let end = points[(index + 1) % points.len()];
        let length = stroke_segment_length_bits(start, end)?;
        if length == 0 {
            continue;
        }
        let mut offset = 0_i64;
        while offset < length {
            let step = pattern_remaining.min(length - offset);
            if pattern_index.is_multiple_of(2) {
                push_unique_point(
                    &mut current,
                    interpolate_stroke_segment(start, end, offset, length)?,
                )?;
                push_unique_point(
                    &mut current,
                    interpolate_stroke_segment(start, end, offset + step, length)?,
                )?;
            }
            offset += step;
            pattern_remaining -= step;
            if pattern_remaining == 0 {
                if pattern_index.is_multiple_of(2) {
                    finish_stroke_piece(&mut pieces, &mut current)?;
                }
                pattern_index = (pattern_index + 1) % pattern.len();
                pattern_remaining = i64::from(pattern[pattern_index].bits());
            }
        }
    }

    let unfinished_on_at_end = !current.is_empty();
    finish_stroke_piece(&mut pieces, &mut current)?;
    if closed && starts_on && unfinished_on_at_end && pieces.len() > 1 {
        let first_starts_at_seam = pieces
            .first()
            .and_then(|piece| piece.points.first())
            .is_some_and(|point| *point == contour_start);
        let last_ends_at_seam = pieces
            .last()
            .and_then(|piece| piece.points.last())
            .is_some_and(|point| *point == contour_start);
        if first_starts_at_seam && last_ends_at_seam {
            let first = pieces.remove(0);
            let mut last = pieces
                .pop()
                .ok_or(SkiaError::new(SkiaErrorCode::InvalidPath))?;
            last.points
                .try_reserve(first.points.len().saturating_sub(1))
                .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
            for point in first.points.into_iter().skip(1) {
                push_unique_point(&mut last.points, point)?;
            }
            pieces.insert(0, last);
        }
    }
    if closed && pieces.len() == 1 {
        let piece = &mut pieces[0];
        if piece.points.len() > 2 && piece.points.first() == piece.points.last() {
            piece.points.pop();
            piece.closed = true;
        }
    }
    Ok(pieces)
}

fn finish_stroke_piece(
    pieces: &mut Vec<StrokePiece>,
    current: &mut Vec<Point>,
) -> Result<(), SkiaError> {
    if current.len() >= 2 {
        pieces
            .try_reserve(1)
            .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
        pieces.push(StrokePiece {
            points: std::mem::take(current),
            closed: false,
        });
    } else {
        current.clear();
    }
    Ok(())
}

fn push_unique_point(points: &mut Vec<Point>, point: Point) -> Result<(), SkiaError> {
    if points.last() == Some(&point) {
        return Ok(());
    }
    points
        .try_reserve(1)
        .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
    points.push(point);
    Ok(())
}

/// Returns the deterministic Q16.16 length of one line segment.
pub fn stroke_segment_length_bits(start: Point, end: Point) -> Result<i64, SkiaError> {
    let dx = i128::from(end.x().bits()) - i128::from(start.x().bits());
    let dy = i128::from(end.y().bits()) - i128::from(start.y().bits());
    let squared = dx
        .checked_mul(dx)
        .and_then(|value| {
            dy.checked_mul(dy)
                .and_then(|other| value.checked_add(other))
        })
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    i64::try_from(
        u128::try_from(squared)
            .map_err(|_| SkiaError::new(SkiaErrorCode::NumericOverflow))?
            .isqrt(),
    )
    .map_err(|_| SkiaError::new(SkiaErrorCode::NumericOverflow))
}

/// Interpolates one point at a Q16.16 distance along a known-length segment.
pub fn interpolate_stroke_segment(
    start: Point,
    end: Point,
    distance: i64,
    length: i64,
) -> Result<Point, SkiaError> {
    if length == 0 {
        return Err(SkiaError::new(SkiaErrorCode::InvalidGeometry));
    }
    if distance == 0 {
        return Ok(start);
    }
    if distance == length {
        return Ok(end);
    }
    let coordinate = |start: Scalar, end: Scalar| -> Result<Scalar, SkiaError> {
        let delta = i128::from(end.bits()) - i128::from(start.bits());
        let numerator = delta
            .checked_mul(i128::from(distance))
            .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
        let denominator = u128::from(length.unsigned_abs());
        let half = denominator / 2;
        let magnitude = numerator
            .unsigned_abs()
            .checked_add(half)
            .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?
            / denominator;
        let offset = i128::try_from(magnitude)
            .map_err(|_| SkiaError::new(SkiaErrorCode::NumericOverflow))?;
        let offset = if (numerator < 0) == (length < 0) {
            offset
        } else {
            -offset
        };
        i32::try_from(
            i128::from(start.bits())
                .checked_add(offset)
                .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?,
        )
        .map(Scalar::from_bits)
        .map_err(|_| SkiaError::new(SkiaErrorCode::NumericOverflow))
    };
    Ok(Point::new(
        coordinate(start.x(), end.x())?,
        coordinate(start.y(), end.y())?,
    ))
}

/// Tests whether a point lies within the exact coverage geometry of a stroke.
pub fn stroke_contains(
    pieces: &[StrokePiece],
    sample: Point,
    options: &StrokeOptions,
) -> Result<bool, SkiaError> {
    let radius = i128::from(options.width().bits()).div_euclid(2);
    for piece in pieces {
        let points = piece.points();
        if points.len() < 2 {
            continue;
        }
        let segment_count = if piece.is_closed() {
            points.len()
        } else {
            points.len() - 1
        };
        for index in 0..segment_count {
            let start = points[index];
            let end = points[(index + 1) % points.len()];
            let square_start =
                !piece.is_closed() && index == 0 && options.cap() == StrokeCap::Square;
            let square_end = !piece.is_closed()
                && index + 1 == segment_count
                && options.cap() == StrokeCap::Square;
            if point_in_segment_strip(sample, start, end, radius, square_start, square_end)? {
                return Ok(true);
            }
        }
        if !piece.is_closed()
            && options.cap() == StrokeCap::Round
            && (point_near_point(sample, points[0], options.width())?
                || point_near_point(sample, points[points.len() - 1], options.width())?)
        {
            return Ok(true);
        }
        if piece.is_closed() {
            for index in 0..points.len() {
                let previous = points[(index + points.len() - 1) % points.len()];
                let vertex = points[index];
                let next = points[(index + 1) % points.len()];
                if stroke_join_contains(sample, previous, vertex, next, options)? {
                    return Ok(true);
                }
            }
        } else {
            for index in 1..points.len() - 1 {
                if stroke_join_contains(
                    sample,
                    points[index - 1],
                    points[index],
                    points[index + 1],
                    options,
                )? {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

fn point_in_segment_strip(
    sample: Point,
    start: Point,
    end: Point,
    radius: i128,
    extend_start: bool,
    extend_end: bool,
) -> Result<bool, SkiaError> {
    let start_x = i128::from(start.x().bits());
    let start_y = i128::from(start.y().bits());
    let delta_x = i128::from(end.x().bits()) - start_x;
    let delta_y = i128::from(end.y().bits()) - start_y;
    let length_squared = delta_x
        .checked_mul(delta_x)
        .and_then(|value| {
            delta_y
                .checked_mul(delta_y)
                .and_then(|other| value.checked_add(other))
        })
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    if length_squared == 0 {
        return Ok(false);
    }
    let sample_x = i128::from(sample.x().bits());
    let sample_y = i128::from(sample.y().bits());
    let relative_x = sample_x - start_x;
    let relative_y = sample_y - start_y;
    let projection = relative_x
        .checked_mul(delta_x)
        .and_then(|value| {
            relative_y
                .checked_mul(delta_y)
                .and_then(|other| value.checked_add(other))
        })
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    let length = i128::from(stroke_segment_length_bits(start, end)?);
    let extension = radius
        .checked_mul(length)
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    let lower = if extend_start { -extension } else { 0 };
    let upper = length_squared
        .checked_add(if extend_end { extension } else { 0 })
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    if projection < lower || projection > upper {
        return Ok(false);
    }
    let cross = relative_x
        .checked_mul(delta_y)
        .and_then(|value| {
            relative_y
                .checked_mul(delta_x)
                .and_then(|other| value.checked_sub(other))
        })
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    let distance = cross
        .checked_mul(cross)
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    let limit = radius
        .checked_mul(radius)
        .and_then(|value| value.checked_mul(length_squared))
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    Ok(distance <= limit)
}

fn stroke_join_contains(
    sample: Point,
    previous: Point,
    vertex: Point,
    next: Point,
    options: &StrokeOptions,
) -> Result<bool, SkiaError> {
    if options.join() == StrokeJoin::Round {
        return point_near_point(sample, vertex, options.width());
    }
    let incoming = (
        i128::from(vertex.x().bits()) - i128::from(previous.x().bits()),
        i128::from(vertex.y().bits()) - i128::from(previous.y().bits()),
    );
    let outgoing = (
        i128::from(next.x().bits()) - i128::from(vertex.x().bits()),
        i128::from(next.y().bits()) - i128::from(vertex.y().bits()),
    );
    let turn = cross_coordinates(incoming, outgoing)?;
    if turn == 0 {
        return Ok(false);
    }
    let side = if turn > 0 { -1_i128 } else { 1_i128 };
    let radius = i128::from(options.width().bits()).div_euclid(2);
    if radius == 0 {
        return Ok(false);
    }
    let center = (i128::from(vertex.x().bits()), i128::from(vertex.y().bits()));
    let outer_incoming = offset_coordinate(center, incoming, radius, side)?;
    let outer_outgoing = offset_coordinate(center, outgoing, radius, side)?;
    let sample = (i128::from(sample.x().bits()), i128::from(sample.y().bits()));
    if options.join() == StrokeJoin::Bevel {
        return point_in_triangle(sample, center, outer_incoming, outer_outgoing);
    }
    let delta = (
        outer_outgoing.0 - outer_incoming.0,
        outer_outgoing.1 - outer_incoming.1,
    );
    let numerator = cross_coordinates(delta, outgoing)?;
    let miter = (
        outer_incoming
            .0
            .checked_add(rounded_div_signed(
                incoming
                    .0
                    .checked_mul(numerator)
                    .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?,
                turn,
            )?)
            .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?,
        outer_incoming
            .1
            .checked_add(rounded_div_signed(
                incoming
                    .1
                    .checked_mul(numerator)
                    .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?,
                turn,
            )?)
            .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?,
    );
    let miter_delta = (miter.0 - center.0, miter.1 - center.1);
    let miter_length_squared = miter_delta
        .0
        .checked_mul(miter_delta.0)
        .and_then(|value| {
            miter_delta
                .1
                .checked_mul(miter_delta.1)
                .and_then(|other| value.checked_add(other))
        })
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    let limit = radius
        .checked_mul(i128::from(options.miter_limit().bits()))
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?
        / (1 << 16);
    if miter_length_squared
        > limit
            .checked_mul(limit)
            .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?
    {
        return point_in_triangle(sample, center, outer_incoming, outer_outgoing);
    }
    Ok(point_in_triangle(sample, center, outer_incoming, miter)?
        || point_in_triangle(sample, center, miter, outer_outgoing)?)
}

fn offset_coordinate(
    center: (i128, i128),
    direction: (i128, i128),
    radius: i128,
    side: i128,
) -> Result<(i128, i128), SkiaError> {
    let squared = direction
        .0
        .checked_mul(direction.0)
        .and_then(|value| {
            direction
                .1
                .checked_mul(direction.1)
                .and_then(|other| value.checked_add(other))
        })
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    let length = i128::try_from(
        u128::try_from(squared)
            .map_err(|_| SkiaError::new(SkiaErrorCode::NumericOverflow))?
            .isqrt(),
    )
    .map_err(|_| SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    if length == 0 {
        return Ok(center);
    }
    let scale = radius
        .checked_mul(side)
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    let x = rounded_div_signed(
        (-direction.1)
            .checked_mul(scale)
            .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?,
        length,
    )?;
    let y = rounded_div_signed(
        direction
            .0
            .checked_mul(scale)
            .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?,
        length,
    )?;
    Ok((
        center
            .0
            .checked_add(x)
            .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?,
        center
            .1
            .checked_add(y)
            .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?,
    ))
}

fn cross_coordinates(first: (i128, i128), second: (i128, i128)) -> Result<i128, SkiaError> {
    first
        .0
        .checked_mul(second.1)
        .and_then(|value| {
            first
                .1
                .checked_mul(second.0)
                .and_then(|other| value.checked_sub(other))
        })
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))
}

fn point_in_triangle(
    point: (i128, i128),
    first: (i128, i128),
    second: (i128, i128),
    third: (i128, i128),
) -> Result<bool, SkiaError> {
    if cross_coordinates(
        (second.0 - first.0, second.1 - first.1),
        (third.0 - first.0, third.1 - first.1),
    )? == 0
    {
        return Ok(false);
    }
    let edge = |start: (i128, i128), end: (i128, i128)| {
        cross_coordinates(
            (end.0 - start.0, end.1 - start.1),
            (point.0 - start.0, point.1 - start.1),
        )
    };
    let first_edge = edge(first, second)?;
    let second_edge = edge(second, third)?;
    let third_edge = edge(third, first)?;
    Ok(!((first_edge < 0 || second_edge < 0 || third_edge < 0)
        && (first_edge > 0 || second_edge > 0 || third_edge > 0)))
}

fn point_near_point(sample: Point, point: Point, width: Scalar) -> Result<bool, SkiaError> {
    point_near_coordinates(
        i128::from(sample.x().bits()),
        i128::from(sample.y().bits()),
        i128::from(point.x().bits()),
        i128::from(point.y().bits()),
        width,
    )
}

fn point_near_coordinates(
    sample_x: i128,
    sample_y: i128,
    point_x: i128,
    point_y: i128,
    width: Scalar,
) -> Result<bool, SkiaError> {
    let dx = sample_x - point_x;
    let dy = sample_y - point_y;
    let distance_squared = dx
        .checked_mul(dx)
        .and_then(|value| {
            dy.checked_mul(dy)
                .and_then(|other| value.checked_add(other))
        })
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    let radius = i128::from(width.bits()).div_euclid(2);
    let radius_squared = radius
        .checked_mul(radius)
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    Ok(distance_squared <= radius_squared)
}

fn rounded_div_signed(numerator: i128, denominator: i128) -> Result<i128, SkiaError> {
    let half = denominator.unsigned_abs() / 2;
    let magnitude = numerator
        .unsigned_abs()
        .checked_add(half)
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?
        / denominator.unsigned_abs();
    let magnitude =
        i128::try_from(magnitude).map_err(|_| SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    Ok(if (numerator < 0) == (denominator < 0) {
        magnitude
    } else {
        -magnitude
    })
}
