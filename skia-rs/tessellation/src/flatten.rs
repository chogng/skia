use skia_geometry::{Point, Scalar, Transform};
use skia_path::{ConicWeight, Path, PathVerb};

use crate::{TessellationError, TessellationErrorCode};

/// Shared default number of line segments emitted for each curve verb.
pub const DEFAULT_CURVE_STEPS: u32 = 16;

/// Resource ceilings and curve resolution for one path-flattening operation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FlatteningLimits {
    max_contours: usize,
    max_points: usize,
    curve_steps: u32,
}

impl FlatteningLimits {
    /// Creates positive output ceilings and a positive fixed curve resolution.
    pub fn new(
        max_contours: usize,
        max_points: usize,
        curve_steps: u32,
    ) -> Result<Self, TessellationError> {
        if max_contours == 0 || max_points == 0 || curve_steps == 0 || curve_steps > i32::MAX as u32
        {
            return Err(error(TessellationErrorCode::InvalidLimits));
        }
        Ok(Self {
            max_contours,
            max_points,
            curve_steps,
        })
    }

    /// Derives exact worst-case output ceilings from one immutable path.
    pub fn for_path(path: &Path, curve_steps: u32) -> Result<Self, TessellationError> {
        if curve_steps == 0 || curve_steps > i32::MAX as u32 {
            return Err(error(TessellationErrorCode::InvalidLimits));
        }
        let curve_points = usize::try_from(curve_steps)
            .map_err(|_| error(TessellationErrorCode::ResourceLimit))?;
        let mut contours = 0_usize;
        let mut points = 0_usize;
        for verb in path.verbs() {
            let additional = match verb {
                PathVerb::MoveTo(_) => {
                    contours = contours
                        .checked_add(1)
                        .ok_or_else(|| error(TessellationErrorCode::ResourceLimit))?;
                    1
                }
                PathVerb::LineTo(_) => 1,
                PathVerb::QuadTo(..) | PathVerb::ConicTo(..) | PathVerb::CubicTo(..) => {
                    curve_points
                }
                PathVerb::Close => 0,
            };
            points = points
                .checked_add(additional)
                .ok_or_else(|| error(TessellationErrorCode::ResourceLimit))?;
        }
        Self::new(contours.max(1), points.max(1), curve_steps)
    }
}

/// One transformed polyline contour produced from a path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FlattenedContour {
    points: Vec<Point>,
    closed: bool,
}

impl FlattenedContour {
    /// Creates one transformed polyline contour for a backend-owned primitive.
    pub fn new(points: Vec<Point>, closed: bool) -> Self {
        Self { points, closed }
    }

    /// Borrows points in path traversal order without synthesizing a closing point.
    pub fn points(&self) -> &[Point] {
        &self.points
    }

    /// Returns whether the source contour ended with an explicit close verb.
    pub const fn is_closed(&self) -> bool {
        self.closed
    }

    /// Moves the points and explicit-close flag out of this contour.
    pub fn into_parts(self) -> (Vec<Point>, bool) {
        (self.points, self.closed)
    }
}

/// All transformed polyline contours produced from one path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FlattenedPath {
    contours: Vec<FlattenedContour>,
}

impl FlattenedPath {
    /// Borrows contours in source order.
    pub fn contours(&self) -> &[FlattenedContour] {
        &self.contours
    }

    /// Moves all contours out of this result.
    pub fn into_contours(self) -> Vec<FlattenedContour> {
        self.contours
    }
}

/// Deterministic fixed-step path flattener shared by drawing backends.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PathFlattener {
    limits: FlatteningLimits,
}

impl PathFlattener {
    /// Creates one flattener with explicit output ceilings and curve resolution.
    pub const fn new(limits: FlatteningLimits) -> Self {
        Self { limits }
    }

    /// Maps a path through `transform` and replaces every curve with fixed steps.
    pub fn flatten(
        &self,
        path: &Path,
        transform: Transform,
    ) -> Result<FlattenedPath, TessellationError> {
        let mut contours = Vec::new();
        let mut current = Vec::new();
        let mut point_count = 0_usize;

        for verb in path.verbs() {
            match *verb {
                PathVerb::MoveTo(point) => {
                    if !current.is_empty() {
                        self.push_contour(&mut contours, current, false)?;
                        current = Vec::new();
                    }
                    self.push_point(&mut current, map_point(transform, point)?, &mut point_count)?;
                }
                PathVerb::LineTo(point) => {
                    require_active(&current)?;
                    self.push_point(&mut current, map_point(transform, point)?, &mut point_count)?;
                }
                PathVerb::QuadTo(control, end) => {
                    let start = require_active(&current)?;
                    self.flatten_quad(
                        &mut current,
                        start,
                        map_point(transform, control)?,
                        map_point(transform, end)?,
                        &mut point_count,
                    )?;
                }
                PathVerb::ConicTo(control, end, weight) => {
                    let start = require_active(&current)?;
                    self.flatten_conic(
                        &mut current,
                        start,
                        map_point(transform, control)?,
                        map_point(transform, end)?,
                        weight,
                        &mut point_count,
                    )?;
                }
                PathVerb::CubicTo(first_control, second_control, end) => {
                    let start = require_active(&current)?;
                    self.flatten_cubic(
                        &mut current,
                        start,
                        map_point(transform, first_control)?,
                        map_point(transform, second_control)?,
                        map_point(transform, end)?,
                        &mut point_count,
                    )?;
                }
                PathVerb::Close => {
                    require_active(&current)?;
                    self.push_contour(&mut contours, current, true)?;
                    current = Vec::new();
                }
            }
        }
        if !current.is_empty() {
            self.push_contour(&mut contours, current, false)?;
        }
        if contours.is_empty() {
            return Err(error(TessellationErrorCode::InvalidPath));
        }
        Ok(FlattenedPath { contours })
    }

    fn push_contour(
        &self,
        contours: &mut Vec<FlattenedContour>,
        points: Vec<Point>,
        closed: bool,
    ) -> Result<(), TessellationError> {
        if contours.len() == self.limits.max_contours {
            return Err(error(TessellationErrorCode::ResourceLimit));
        }
        contours
            .try_reserve(1)
            .map_err(|_| error(TessellationErrorCode::AllocationFailed))?;
        contours.push(FlattenedContour { points, closed });
        Ok(())
    }

    fn push_point(
        &self,
        points: &mut Vec<Point>,
        point: Point,
        point_count: &mut usize,
    ) -> Result<(), TessellationError> {
        if *point_count == self.limits.max_points {
            return Err(error(TessellationErrorCode::ResourceLimit));
        }
        points
            .try_reserve(1)
            .map_err(|_| error(TessellationErrorCode::AllocationFailed))?;
        points.push(point);
        *point_count += 1;
        Ok(())
    }

    fn flatten_quad(
        &self,
        output: &mut Vec<Point>,
        start: Point,
        control: Point,
        end: Point,
        point_count: &mut usize,
    ) -> Result<(), TessellationError> {
        let point_count_after_curve = self.reserve_curve(output, *point_count)?;
        for step in 1..=self.limits.curve_steps {
            output.push(Point::new(
                bezier2(
                    start.x(),
                    control.x(),
                    end.x(),
                    step,
                    self.limits.curve_steps,
                )?,
                bezier2(
                    start.y(),
                    control.y(),
                    end.y(),
                    step,
                    self.limits.curve_steps,
                )?,
            ));
        }
        *point_count = point_count_after_curve;
        Ok(())
    }

    fn flatten_conic(
        &self,
        output: &mut Vec<Point>,
        start: Point,
        control: Point,
        end: Point,
        weight: ConicWeight,
        point_count: &mut usize,
    ) -> Result<(), TessellationError> {
        let point_count_after_curve = self.reserve_curve(output, *point_count)?;
        for step in 1..=self.limits.curve_steps {
            output.push(Point::new(
                conic_coordinate(
                    start.x(),
                    control.x(),
                    end.x(),
                    weight,
                    step,
                    self.limits.curve_steps,
                )?,
                conic_coordinate(
                    start.y(),
                    control.y(),
                    end.y(),
                    weight,
                    step,
                    self.limits.curve_steps,
                )?,
            ));
        }
        *point_count = point_count_after_curve;
        Ok(())
    }

    fn flatten_cubic(
        &self,
        output: &mut Vec<Point>,
        start: Point,
        first_control: Point,
        second_control: Point,
        end: Point,
        point_count: &mut usize,
    ) -> Result<(), TessellationError> {
        let point_count_after_curve = self.reserve_curve(output, *point_count)?;
        for step in 1..=self.limits.curve_steps {
            output.push(Point::new(
                bezier3(
                    start.x(),
                    first_control.x(),
                    second_control.x(),
                    end.x(),
                    step,
                    self.limits.curve_steps,
                )?,
                bezier3(
                    start.y(),
                    first_control.y(),
                    second_control.y(),
                    end.y(),
                    step,
                    self.limits.curve_steps,
                )?,
            ));
        }
        *point_count = point_count_after_curve;
        Ok(())
    }

    fn reserve_curve(
        &self,
        output: &mut Vec<Point>,
        point_count: usize,
    ) -> Result<usize, TessellationError> {
        let curve_points = usize::try_from(self.limits.curve_steps)
            .map_err(|_| error(TessellationErrorCode::ResourceLimit))?;
        let required = point_count
            .checked_add(curve_points)
            .ok_or_else(|| error(TessellationErrorCode::ResourceLimit))?;
        if required > self.limits.max_points {
            return Err(error(TessellationErrorCode::ResourceLimit));
        }
        output
            .try_reserve(curve_points)
            .map_err(|_| error(TessellationErrorCode::AllocationFailed))?;
        Ok(required)
    }
}

fn require_active(points: &[Point]) -> Result<Point, TessellationError> {
    points
        .last()
        .copied()
        .ok_or_else(|| error(TessellationErrorCode::InvalidPath))
}

fn map_point(transform: Transform, point: Point) -> Result<Point, TessellationError> {
    transform
        .map_point(point)
        .map_err(|_| error(TessellationErrorCode::NumericOverflow))
}

fn bezier2(
    start: Scalar,
    control: Scalar,
    end: Scalar,
    step: u32,
    steps: u32,
) -> Result<Scalar, TessellationError> {
    let step = i128::from(step);
    let steps = i128::from(steps);
    let inverse = steps - step;
    let value = i128::from(start.bits()) * inverse * inverse
        + i128::from(control.bits()) * 2 * inverse * step
        + i128::from(end.bits()) * step * step;
    rounded_scalar(value, steps * steps)
}

fn conic_coordinate(
    start: Scalar,
    control: Scalar,
    end: Scalar,
    weight: ConicWeight,
    step: u32,
    steps: u32,
) -> Result<Scalar, TessellationError> {
    let step = i128::from(step);
    let steps = i128::from(steps);
    let inverse = steps - step;
    let start_weight = inverse * inverse * i128::from(1_i64 << 16);
    let control_weight = 2 * inverse * step * i128::from(weight.bits());
    let end_weight = step * step * i128::from(1_i64 << 16);
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
    step: u32,
    steps: u32,
) -> Result<Scalar, TessellationError> {
    let step = i128::from(step);
    let steps = i128::from(steps);
    let inverse = steps - step;
    let value = i128::from(start.bits()) * inverse * inverse * inverse
        + i128::from(first_control.bits()) * 3 * inverse * inverse * step
        + i128::from(second_control.bits()) * 3 * inverse * step * step
        + i128::from(end.bits()) * step * step * step;
    rounded_scalar(value, steps * steps * steps)
}

fn rounded_scalar(value: i128, divisor: i128) -> Result<Scalar, TessellationError> {
    let half = divisor / 2;
    let value = if value >= 0 {
        value
            .checked_add(half)
            .ok_or_else(|| error(TessellationErrorCode::NumericOverflow))?
            / divisor
    } else {
        -(value
            .checked_neg()
            .and_then(|value| value.checked_add(half))
            .ok_or_else(|| error(TessellationErrorCode::NumericOverflow))?
            / divisor)
    };
    i32::try_from(value)
        .map(Scalar::from_bits)
        .map_err(|_| error(TessellationErrorCode::NumericOverflow))
}

const fn error(code: TessellationErrorCode) -> TessellationError {
    TessellationError::new(code)
}

#[cfg(test)]
#[path = "flatten_tests.rs"]
mod tests;
