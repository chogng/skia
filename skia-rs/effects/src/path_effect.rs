use skia_core::{
    Path, PathBuilder, PathEffect, PathEffectLimits, Point, Scalar, SkiaError, SkiaErrorCode,
    StrokeOptions, Transform, apply_path_effect,
};
use skia_tessellation::{
    FlattenedContour, FlatteningLimits, PathFlattener, TessellationError, TessellationErrorCode,
    interpolate_stroke_segment, stroke_segment_length_bits,
};

const UNIT_BITS: i32 = 1 << 16;

/// Normalized trim interval applied independently to every path contour.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TrimPathEffect {
    start: Scalar,
    end: Scalar,
}

/// Maximum along-edge distance replaced by a quadratic corner curve.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CornerPathEffect {
    radius: Scalar,
}

/// Deterministic polyline resampling with seeded normal displacement.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DiscretePathEffect {
    segment_length: Scalar,
    deviation: Scalar,
    seed: u64,
}

/// Alternating visible and hidden intervals applied to a path centerline.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DashPathEffect {
    options: StrokeOptions,
}

/// Sequential composition of two borrowed path effects.
///
/// The inner effect runs first and consumes the supplied transform. The outer
/// effect then receives the inner output in target coordinates.
#[derive(Clone, Copy)]
pub struct ComposePathEffect<'a> {
    outer: &'a dyn PathEffect,
    inner: &'a dyn PathEffect,
}

/// Union-by-concatenation of two borrowed path-effect results.
///
/// Both effects receive the same source path and transform. Their output
/// contours are appended without a geometric boolean operation.
#[derive(Clone, Copy)]
pub struct SumPathEffect<'a> {
    first: &'a dyn PathEffect,
    second: &'a dyn PathEffect,
}

impl DashPathEffect {
    /// Creates a non-empty, even-length positive dash pattern.
    ///
    /// The phase is normalized into one complete pattern cycle.
    pub fn new(pattern: &[Scalar], phase: Scalar) -> Result<Self, SkiaError> {
        if pattern.is_empty() {
            return Err(SkiaError::new(SkiaErrorCode::InvalidGeometry));
        }
        let options =
            StrokeOptions::new(Scalar::from_bits(1))?.with_dash_pattern(pattern, phase)?;
        Ok(Self { options })
    }

    /// Borrows alternating visible and hidden lengths.
    pub fn pattern(&self) -> &[Scalar] {
        self.options.dash_pattern()
    }

    /// Returns the canonical non-negative phase within one cycle.
    pub const fn phase(&self) -> Scalar {
        self.options.dash_phase()
    }
}

impl<'a> ComposePathEffect<'a> {
    /// Creates an outer-after-inner composition.
    pub const fn new(outer: &'a dyn PathEffect, inner: &'a dyn PathEffect) -> Self {
        Self { outer, inner }
    }
}

impl<'a> SumPathEffect<'a> {
    /// Creates a concatenating pair of independently evaluated effects.
    pub const fn new(first: &'a dyn PathEffect, second: &'a dyn PathEffect) -> Self {
        Self { first, second }
    }
}

impl CornerPathEffect {
    /// Creates a positive corner distance.
    pub fn new(radius: Scalar) -> Result<Self, SkiaError> {
        if radius.bits() <= 0 {
            return Err(SkiaError::new(SkiaErrorCode::InvalidGeometry));
        }
        Ok(Self { radius })
    }

    /// Returns the maximum distance trimmed from each adjacent edge.
    pub const fn radius(self) -> Scalar {
        self.radius
    }
}

impl DiscretePathEffect {
    /// Creates an effect with positive maximum source interval and non-negative deviation.
    pub fn new(segment_length: Scalar, deviation: Scalar, seed: u64) -> Result<Self, SkiaError> {
        if segment_length.bits() <= 0 || deviation.bits() < 0 {
            return Err(SkiaError::new(SkiaErrorCode::InvalidGeometry));
        }
        Ok(Self {
            segment_length,
            deviation,
            seed,
        })
    }

    /// Returns the maximum source arc-length interval between samples.
    pub const fn segment_length(self) -> Scalar {
        self.segment_length
    }

    /// Returns the maximum absolute displacement along the local normal.
    pub const fn deviation(self) -> Scalar {
        self.deviation
    }

    /// Returns the deterministic displacement seed.
    pub const fn seed(self) -> u64 {
        self.seed
    }
}

impl TrimPathEffect {
    /// Creates a normalized interval whose endpoints are in the inclusive range `[0, 1]`.
    ///
    /// `start > end` wraps through the contour seam. Equal endpoints produce
    /// an empty path rather than a full contour.
    pub fn new(start: Scalar, end: Scalar) -> Result<Self, SkiaError> {
        if !(0..=UNIT_BITS).contains(&start.bits()) || !(0..=UNIT_BITS).contains(&end.bits()) {
            return Err(SkiaError::new(SkiaErrorCode::InvalidGeometry));
        }
        Ok(Self { start, end })
    }

    /// Returns the normalized start fraction.
    pub const fn start(self) -> Scalar {
        self.start
    }

    /// Returns the normalized end fraction.
    pub const fn end(self) -> Scalar {
        self.end
    }
}

impl PathEffect for TrimPathEffect {
    fn apply(
        &self,
        path: &Path,
        transform: Transform,
        limits: PathEffectLimits,
    ) -> Result<Option<Path>, SkiaError> {
        trim_path(path, *self, transform, limits)
    }
}

impl PathEffect for CornerPathEffect {
    fn apply(
        &self,
        path: &Path,
        transform: Transform,
        limits: PathEffectLimits,
    ) -> Result<Option<Path>, SkiaError> {
        corner_path(path, *self, transform, limits)
    }
}

impl PathEffect for DiscretePathEffect {
    fn apply(
        &self,
        path: &Path,
        transform: Transform,
        limits: PathEffectLimits,
    ) -> Result<Option<Path>, SkiaError> {
        discrete_path(path, *self, transform, limits)
    }
}

impl PathEffect for DashPathEffect {
    fn apply(
        &self,
        path: &Path,
        transform: Transform,
        limits: PathEffectLimits,
    ) -> Result<Option<Path>, SkiaError> {
        dash_path(path, self, transform, limits)
    }
}

impl PathEffect for ComposePathEffect<'_> {
    fn apply(
        &self,
        path: &Path,
        transform: Transform,
        limits: PathEffectLimits,
    ) -> Result<Option<Path>, SkiaError> {
        let Some(inner) = apply_path_effect(path, self.inner, transform, limits)? else {
            return Ok(None);
        };
        apply_path_effect(&inner, self.outer, Transform::IDENTITY, limits)
    }
}

impl PathEffect for SumPathEffect<'_> {
    fn apply(
        &self,
        path: &Path,
        transform: Transform,
        limits: PathEffectLimits,
    ) -> Result<Option<Path>, SkiaError> {
        let first = apply_path_effect(path, self.first, transform, limits)?;
        let second = apply_path_effect(path, self.second, transform, limits)?;
        concatenate_effect_outputs(first.as_ref(), second.as_ref(), limits)
    }
}

/// Splits every transformed contour into visible dash centerline pieces.
///
/// Curves are flattened with the configured fixed resolution. Closed seams
/// remain joined when the visible interval crosses the contour origin.
pub fn dash_path(
    path: &Path,
    effect: &DashPathEffect,
    transform: Transform,
    limits: PathEffectLimits,
) -> Result<Option<Path>, SkiaError> {
    let flattening = FlatteningLimits::new(
        limits.max_contours(),
        limits.max_points(),
        limits.curve_steps(),
    )
    .map_err(map_tessellation_error)?;
    let contours = PathFlattener::new(flattening)
        .flatten(path, transform)
        .map_err(map_tessellation_error)?;
    let pieces = skia_tessellation::stroke_pieces(contours.contours(), &effect.options)?;
    let mut builder = PathBuilder::new(limits.max_output_verbs())?;
    let mut emitted = false;
    for piece in pieces {
        let Some(first) = piece.points().first().copied() else {
            continue;
        };
        builder.move_to(first)?;
        for point in piece.points().iter().copied().skip(1) {
            builder.line_to(point)?;
        }
        if piece.is_closed() {
            builder.close()?;
        }
        emitted = true;
    }
    if emitted {
        builder.finish().map(Some)
    } else {
        Ok(None)
    }
}

fn concatenate_effect_outputs(
    first: Option<&Path>,
    second: Option<&Path>,
    limits: PathEffectLimits,
) -> Result<Option<Path>, SkiaError> {
    if first.is_none() && second.is_none() {
        return Ok(None);
    }
    let mut builder = PathBuilder::new(limits.max_output_verbs())?;
    if let Some(path) = first {
        builder.append_path(path)?;
    }
    if let Some(path) = second {
        builder.append_path(path)?;
    }
    builder.finish().map(Some)
}

/// Trims every transformed contour by the same normalized arc-length interval.
///
/// Curves are flattened with the configured fixed resolution. Partial results
/// are open contours; selecting `[0, 1]` preserves explicitly closed contours.
/// Equal endpoints return `None`.
pub fn trim_path(
    path: &Path,
    effect: TrimPathEffect,
    transform: Transform,
    limits: PathEffectLimits,
) -> Result<Option<Path>, SkiaError> {
    if effect.start == effect.end {
        return Ok(None);
    }
    let flattening = FlatteningLimits::new(
        limits.max_contours(),
        limits.max_points(),
        limits.curve_steps(),
    )
    .map_err(map_tessellation_error)?;
    let contours = PathFlattener::new(flattening)
        .flatten(path, transform)
        .map_err(map_tessellation_error)?;
    let mut pieces = Vec::new();
    pieces
        .try_reserve_exact(contours.contours().len())
        .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
    for contour in contours.contours() {
        append_trimmed_contour(&mut pieces, contour, effect)?;
    }
    if pieces.is_empty() {
        return Ok(None);
    }
    let verb_count = pieces.iter().try_fold(0_usize, |count, piece| {
        count
            .checked_add(piece.points.len())
            .and_then(|value| value.checked_add(usize::from(piece.closed)))
            .ok_or(SkiaError::new(SkiaErrorCode::ResourceLimit))
    })?;
    if verb_count == 0 || verb_count > limits.max_output_verbs() {
        return Err(SkiaError::new(SkiaErrorCode::ResourceLimit));
    }
    let mut builder = PathBuilder::new(verb_count)?;
    for piece in pieces {
        builder.move_to(piece.points[0])?;
        for point in piece.points.into_iter().skip(1) {
            builder.line_to(point)?;
        }
        if piece.closed {
            builder.close()?;
        }
    }
    builder.finish().map(Some)
}

/// Replaces transformed polyline corners with deterministic quadratic curves.
///
/// Each corner trims at most `effect.radius()` from both incident edges and at
/// most half of either edge. Open endpoints remain unchanged; explicitly
/// closed contours remain closed. Contours with fewer than two usable points
/// are omitted, and an entirely empty result is returned as `None`.
pub fn corner_path(
    path: &Path,
    effect: CornerPathEffect,
    transform: Transform,
    limits: PathEffectLimits,
) -> Result<Option<Path>, SkiaError> {
    let flattening = FlatteningLimits::new(
        limits.max_contours(),
        limits.max_points(),
        limits.curve_steps(),
    )
    .map_err(map_tessellation_error)?;
    let contours = PathFlattener::new(flattening)
        .flatten(path, transform)
        .map_err(map_tessellation_error)?;
    let mut builder = PathBuilder::new(limits.max_output_verbs())?;
    let mut emitted = false;
    for contour in contours.contours() {
        let points = normalized_points(contour.points(), contour.is_closed())?;
        if points.len() < 2 {
            continue;
        }
        if contour.is_closed() {
            if points.len() >= 3 {
                append_closed_corners(&mut builder, &points, effect)?;
            } else {
                builder.move_to(points[0])?;
                builder.line_to(points[1])?;
                builder.close()?;
            }
        } else {
            append_open_corners(&mut builder, &points, effect)?;
        }
        emitted = true;
    }
    if emitted {
        builder.finish().map(Some)
    } else {
        Ok(None)
    }
}

/// Resamples transformed contours and displaces samples along local normals.
///
/// The source arc-length interval is uniform per contour and never exceeds
/// `effect.segment_length()`. Displacement may increase the final straight-line
/// distance between samples. Open endpoints remain fixed. Closed contours use
/// one displaced seam sample and remain explicitly closed. The same seed and
/// input always produce identical Q16.16 output.
pub fn discrete_path(
    path: &Path,
    effect: DiscretePathEffect,
    transform: Transform,
    limits: PathEffectLimits,
) -> Result<Option<Path>, SkiaError> {
    let flattening = FlatteningLimits::new(
        limits.max_contours(),
        limits.max_points(),
        limits.curve_steps(),
    )
    .map_err(map_tessellation_error)?;
    let contours = PathFlattener::new(flattening)
        .flatten(path, transform)
        .map_err(map_tessellation_error)?;
    let mut builder = PathBuilder::new(limits.max_output_verbs())?;
    let mut emitted_verbs = 0_usize;
    let mut emitted = false;
    for (contour_index, contour) in contours.contours().iter().enumerate() {
        let segments = contour_segments(contour)?;
        let total = segments.last().map_or(0, |segment| segment.end_distance);
        if total == 0 {
            continue;
        }
        let mut segment_count = total / i64::from(effect.segment_length().bits());
        if total % i64::from(effect.segment_length().bits()) != 0 {
            segment_count = segment_count
                .checked_add(1)
                .ok_or(SkiaError::new(SkiaErrorCode::ResourceLimit))?;
        }
        if contour.is_closed() {
            segment_count = segment_count.max(3);
        } else {
            segment_count = segment_count.max(1);
        }
        let segment_count = usize::try_from(segment_count)
            .map_err(|_| SkiaError::new(SkiaErrorCode::ResourceLimit))?;
        let required_verbs = segment_count
            .checked_add(1)
            .ok_or(SkiaError::new(SkiaErrorCode::ResourceLimit))?;
        emitted_verbs = emitted_verbs
            .checked_add(required_verbs)
            .ok_or(SkiaError::new(SkiaErrorCode::ResourceLimit))?;
        if emitted_verbs > limits.max_output_verbs() {
            return Err(SkiaError::new(SkiaErrorCode::ResourceLimit));
        }

        let sample_end = if contour.is_closed() {
            segment_count
        } else {
            required_verbs
        };
        for sample_index in 0..sample_end {
            let distance = uniform_distance(total, sample_index, segment_count)?;
            let (mut point, tangent) = point_and_tangent_at(&segments, distance)?;
            let displace =
                contour.is_closed() || (sample_index != 0 && sample_index != segment_count);
            if displace && effect.deviation().bits() != 0 {
                point = displace_point(
                    point,
                    tangent,
                    random_offset_bits(effect, contour_index, sample_index)?,
                )?;
            }
            if sample_index == 0 {
                builder.move_to(point)?;
            } else {
                builder.line_to(point)?;
            }
        }
        if contour.is_closed() {
            builder.close()?;
        }
        emitted = true;
    }
    if emitted {
        builder.finish().map(Some)
    } else {
        Ok(None)
    }
}

fn uniform_distance(total: i64, index: usize, count: usize) -> Result<i64, SkiaError> {
    let numerator = i128::from(total)
        .checked_mul(
            i128::try_from(index).map_err(|_| SkiaError::new(SkiaErrorCode::ResourceLimit))?,
        )
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    let denominator =
        i128::try_from(count).map_err(|_| SkiaError::new(SkiaErrorCode::ResourceLimit))?;
    let rounded = numerator
        .checked_add(denominator / 2)
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?
        / denominator;
    i64::try_from(rounded).map_err(|_| SkiaError::new(SkiaErrorCode::NumericOverflow))
}

fn point_and_tangent_at(
    segments: &[MeasuredSegment],
    distance: i64,
) -> Result<(Point, MeasuredSegment), SkiaError> {
    let segment = segments
        .iter()
        .find(|segment| distance < segment.end_distance)
        .or_else(|| segments.last())
        .copied()
        .ok_or(SkiaError::new(SkiaErrorCode::InvalidGeometry))?;
    let length = segment.end_distance - segment.start_distance;
    let along = (distance - segment.start_distance).clamp(0, length);
    Ok((
        interpolate_stroke_segment(segment.start, segment.end, along, length)?,
        segment,
    ))
}

fn random_offset_bits(
    effect: DiscretePathEffect,
    contour_index: usize,
    sample_index: usize,
) -> Result<i64, SkiaError> {
    let contour =
        u64::try_from(contour_index).map_err(|_| SkiaError::new(SkiaErrorCode::ResourceLimit))?;
    let sample =
        u64::try_from(sample_index).map_err(|_| SkiaError::new(SkiaErrorCode::ResourceLimit))?;
    let mut value = effect.seed()
        ^ contour.wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ sample.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^= value >> 31;
    let random = i128::from((value >> 32) as u32);
    let deviation = i128::from(effect.deviation().bits());
    let offset = random
        .checked_mul(deviation * 2)
        .and_then(|value| value.checked_add(i128::from(u32::MAX) / 2))
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?
        / i128::from(u32::MAX)
        - deviation;
    i64::try_from(offset).map_err(|_| SkiaError::new(SkiaErrorCode::NumericOverflow))
}

fn displace_point(point: Point, tangent: MeasuredSegment, offset: i64) -> Result<Point, SkiaError> {
    let dx = i64::from(tangent.end.x().bits()) - i64::from(tangent.start.x().bits());
    let dy = i64::from(tangent.end.y().bits()) - i64::from(tangent.start.y().bits());
    let length = tangent.end_distance - tangent.start_distance;
    let x_delta = rounded_signed_ratio(-i128::from(dy) * i128::from(offset), length)?;
    let y_delta = rounded_signed_ratio(i128::from(dx) * i128::from(offset), length)?;
    let x = i64::from(point.x().bits())
        .checked_add(x_delta)
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    let y = i64::from(point.y().bits())
        .checked_add(y_delta)
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    Ok(Point::new(
        Scalar::from_bits(
            i32::try_from(x).map_err(|_| SkiaError::new(SkiaErrorCode::NumericOverflow))?,
        ),
        Scalar::from_bits(
            i32::try_from(y).map_err(|_| SkiaError::new(SkiaErrorCode::NumericOverflow))?,
        ),
    ))
}

fn rounded_signed_ratio(numerator: i128, denominator: i64) -> Result<i64, SkiaError> {
    let denominator = i128::from(denominator);
    let rounded = if numerator >= 0 {
        numerator
            .checked_add(denominator / 2)
            .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?
            / denominator
    } else {
        numerator
            .checked_sub(denominator / 2)
            .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?
            / denominator
    };
    i64::try_from(rounded).map_err(|_| SkiaError::new(SkiaErrorCode::NumericOverflow))
}

fn append_open_corners(
    builder: &mut PathBuilder,
    points: &[Point],
    effect: CornerPathEffect,
) -> Result<(), SkiaError> {
    builder.move_to(points[0])?;
    for index in 1..points.len() - 1 {
        let (entry, exit) =
            corner_points(points[index - 1], points[index], points[index + 1], effect)?;
        builder.line_to(entry)?;
        builder.quad_to(points[index], exit)?;
    }
    builder.line_to(points[points.len() - 1])
}

fn append_closed_corners(
    builder: &mut PathBuilder,
    points: &[Point],
    effect: CornerPathEffect,
) -> Result<(), SkiaError> {
    let first = corner_points(points[points.len() - 1], points[0], points[1], effect)?;
    builder.move_to(first.1)?;
    for index in 1..points.len() {
        let (entry, exit) = corner_points(
            points[index - 1],
            points[index],
            points[(index + 1) % points.len()],
            effect,
        )?;
        builder.line_to(entry)?;
        builder.quad_to(points[index], exit)?;
    }
    builder.line_to(first.0)?;
    builder.quad_to(points[0], first.1)?;
    builder.close()
}

fn corner_points(
    previous: Point,
    vertex: Point,
    next: Point,
    effect: CornerPathEffect,
) -> Result<(Point, Point), SkiaError> {
    let incoming = stroke_segment_length_bits(previous, vertex)?;
    let outgoing = stroke_segment_length_bits(vertex, next)?;
    if incoming == 0 || outgoing == 0 {
        return Ok((vertex, vertex));
    }
    let distance = i64::from(effect.radius().bits())
        .min(incoming / 2)
        .min(outgoing / 2);
    Ok((
        interpolate_stroke_segment(previous, vertex, incoming - distance, incoming)?,
        interpolate_stroke_segment(vertex, next, distance, outgoing)?,
    ))
}

fn normalized_points(points: &[Point], closed: bool) -> Result<Vec<Point>, SkiaError> {
    let mut normalized = Vec::new();
    normalized
        .try_reserve_exact(points.len())
        .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
    for point in points {
        if normalized.last() != Some(point) {
            normalized.push(*point);
        }
    }
    if closed && normalized.len() > 1 && normalized.first() == normalized.last() {
        normalized.pop();
    }
    Ok(normalized)
}

#[derive(Debug)]
struct TrimmedPiece {
    points: Vec<Point>,
    closed: bool,
}

fn append_trimmed_contour(
    output: &mut Vec<TrimmedPiece>,
    contour: &FlattenedContour,
    effect: TrimPathEffect,
) -> Result<(), SkiaError> {
    let segments = contour_segments(contour)?;
    let total = segments.last().map_or(0, |segment| segment.end_distance);
    if total == 0 {
        return Ok(());
    }
    if effect.start.bits() == 0 && effect.end.bits() == UNIT_BITS {
        let mut points = Vec::new();
        points
            .try_reserve_exact(contour.points().len())
            .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
        points.extend_from_slice(contour.points());
        if points.len() >= 2 {
            output
                .try_reserve(1)
                .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
            output.push(TrimmedPiece {
                points: std::mem::take(&mut points),
                closed: contour.is_closed(),
            });
        }
        return Ok(());
    }
    let start = normalized_distance(total, effect.start)?;
    let end = normalized_distance(total, effect.end)?;
    if start < end {
        append_interval(output, &segments, start, end)?;
    } else if contour.is_closed() {
        let mut tail = interval_points(&segments, start, total)?;
        let head = interval_points(&segments, 0, end)?;
        if !tail.is_empty() && !head.is_empty() {
            tail.try_reserve(head.len().saturating_sub(1))
                .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
            for point in head.into_iter().skip(1) {
                push_unique(&mut tail, point)?;
            }
            push_piece(output, tail)?;
        } else {
            push_piece(output, tail)?;
            push_piece(output, head)?;
        }
    } else {
        append_interval(output, &segments, start, total)?;
        append_interval(output, &segments, 0, end)?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
struct MeasuredSegment {
    start: Point,
    end: Point,
    start_distance: i64,
    end_distance: i64,
}

fn contour_segments(contour: &FlattenedContour) -> Result<Vec<MeasuredSegment>, SkiaError> {
    let points = contour.points();
    let count = if contour.is_closed() {
        points.len()
    } else {
        points.len().saturating_sub(1)
    };
    let mut segments = Vec::new();
    segments
        .try_reserve_exact(count)
        .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
    let mut distance = 0_i64;
    for index in 0..count {
        let start = points[index];
        let end = points[(index + 1) % points.len()];
        let length = stroke_segment_length_bits(start, end)?;
        if length == 0 {
            continue;
        }
        let end_distance = distance
            .checked_add(length)
            .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
        segments.push(MeasuredSegment {
            start,
            end,
            start_distance: distance,
            end_distance,
        });
        distance = end_distance;
    }
    Ok(segments)
}

fn normalized_distance(total: i64, fraction: Scalar) -> Result<i64, SkiaError> {
    let value = i128::from(total)
        .checked_mul(i128::from(fraction.bits()))
        .and_then(|value| value.checked_add(1 << 15))
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?
        / (1 << 16);
    i64::try_from(value).map_err(|_| SkiaError::new(SkiaErrorCode::NumericOverflow))
}

fn append_interval(
    output: &mut Vec<TrimmedPiece>,
    segments: &[MeasuredSegment],
    start: i64,
    end: i64,
) -> Result<(), SkiaError> {
    push_piece(output, interval_points(segments, start, end)?)
}

fn interval_points(
    segments: &[MeasuredSegment],
    start: i64,
    end: i64,
) -> Result<Vec<Point>, SkiaError> {
    let mut points = Vec::new();
    if start >= end {
        return Ok(points);
    }
    for segment in segments {
        let overlap_start = start.max(segment.start_distance);
        let overlap_end = end.min(segment.end_distance);
        if overlap_start >= overlap_end {
            continue;
        }
        let length = segment.end_distance - segment.start_distance;
        let first = interpolate_stroke_segment(
            segment.start,
            segment.end,
            overlap_start - segment.start_distance,
            length,
        )?;
        let last = interpolate_stroke_segment(
            segment.start,
            segment.end,
            overlap_end - segment.start_distance,
            length,
        )?;
        push_unique(&mut points, first)?;
        push_unique(&mut points, last)?;
    }
    Ok(points)
}

fn push_piece(output: &mut Vec<TrimmedPiece>, points: Vec<Point>) -> Result<(), SkiaError> {
    if points.len() < 2 {
        return Ok(());
    }
    output
        .try_reserve(1)
        .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
    output.push(TrimmedPiece {
        points,
        closed: false,
    });
    Ok(())
}

fn push_unique(points: &mut Vec<Point>, point: Point) -> Result<(), SkiaError> {
    if points.last() == Some(&point) {
        return Ok(());
    }
    points
        .try_reserve(1)
        .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
    points.push(point);
    Ok(())
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
