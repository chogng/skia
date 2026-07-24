use crate::paint::Color;
use skia_error::{SkiaError, SkiaErrorCode};
use skia_geometry::{Point, Scalar};

const GRADIENT_SCALE: i128 = 1 << 16;

/// Gradient coordinate behavior outside the first and last stop.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TileMode {
    /// Extends the edge stop colors.
    Clamp,
    /// Repeats every unit interval.
    Repeat,
    /// Alternates forward and reversed unit intervals.
    Mirror,
}

/// One color and normalized Q16.16 position in a gradient.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GradientStop {
    offset: Scalar,
    color: Color,
}

impl GradientStop {
    const EMPTY: Self = Self {
        offset: Scalar::ZERO,
        color: Color::TRANSPARENT,
    };

    /// Creates a stop whose offset is in the inclusive range `[0, 1]`.
    pub const fn new(offset: Scalar, color: Color) -> Result<Self, SkiaError> {
        if offset.bits() < 0 || offset.bits() > 1 << 16 {
            return Err(SkiaError::new(SkiaErrorCode::InvalidGeometry));
        }
        Ok(Self { offset, color })
    }

    /// Returns the normalized position.
    pub const fn offset(self) -> Scalar {
        self.offset
    }

    /// Returns the straight-alpha stop color.
    pub const fn color(self) -> Color {
        self.color
    }
}

/// Geometric shape used to evaluate a gradient in local canvas coordinates.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GradientGeometry {
    /// Projection along a non-degenerate line from start to end.
    Linear {
        /// Unit-interval origin.
        start: Point,
        /// Unit-interval endpoint.
        end: Point,
    },
    /// Distance from a center divided by a positive radius.
    Radial {
        /// Circle center.
        center: Point,
        /// Unit-interval radius.
        radius: Scalar,
    },
}

/// Immutable bounded linear or radial gradient.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Gradient {
    geometry: GradientGeometry,
    stops: [GradientStop; Self::MAX_STOPS],
    stop_count: u8,
    tile_mode: TileMode,
}

impl Gradient {
    /// Maximum stop count retained inline by one paint.
    pub const MAX_STOPS: usize = 8;

    /// Creates a local-space linear gradient.
    pub fn linear(
        start: Point,
        end: Point,
        stops: &[GradientStop],
        tile_mode: TileMode,
    ) -> Result<Self, SkiaError> {
        if start == end {
            return Err(SkiaError::new(SkiaErrorCode::InvalidGeometry));
        }
        Self::new(GradientGeometry::Linear { start, end }, stops, tile_mode)
    }

    /// Creates a local-space radial gradient.
    pub fn radial(
        center: Point,
        radius: Scalar,
        stops: &[GradientStop],
        tile_mode: TileMode,
    ) -> Result<Self, SkiaError> {
        if radius.bits() <= 0 {
            return Err(SkiaError::new(SkiaErrorCode::InvalidGeometry));
        }
        Self::new(
            GradientGeometry::Radial { center, radius },
            stops,
            tile_mode,
        )
    }

    fn new(
        geometry: GradientGeometry,
        stops: &[GradientStop],
        tile_mode: TileMode,
    ) -> Result<Self, SkiaError> {
        if stops.len() < 2
            || stops.len() > Self::MAX_STOPS
            || stops.windows(2).any(|pair| pair[0].offset > pair[1].offset)
        {
            return Err(SkiaError::new(SkiaErrorCode::InvalidGeometry));
        }
        let mut retained = [GradientStop::EMPTY; Self::MAX_STOPS];
        retained[..stops.len()].copy_from_slice(stops);
        Ok(Self {
            geometry,
            stops: retained,
            stop_count: u8::try_from(stops.len())
                .map_err(|_| SkiaError::new(SkiaErrorCode::ResourceLimit))?,
            tile_mode,
        })
    }

    /// Returns the local-space geometry.
    pub const fn geometry(self) -> GradientGeometry {
        self.geometry
    }

    /// Borrows ordered retained stops.
    pub fn stops(&self) -> &[GradientStop] {
        &self.stops[..usize::from(self.stop_count)]
    }

    /// Returns the out-of-range coordinate policy.
    pub const fn tile_mode(self) -> TileMode {
        self.tile_mode
    }

    /// Evaluates one local-space point with deterministic fixed-point interpolation.
    pub fn sample(self, point: Point) -> Result<Color, SkiaError> {
        let parameter = match self.geometry {
            GradientGeometry::Linear { start, end } => linear_parameter(start, end, point)?,
            GradientGeometry::Radial { center, radius } => radial_parameter(center, radius, point)?,
        };
        Ok(sample_stops(
            self.stops(),
            tile_parameter(parameter, self.tile_mode),
        ))
    }
}

fn linear_parameter(start: Point, end: Point, point: Point) -> Result<i128, SkiaError> {
    let dx = i128::from(end.x().bits()) - i128::from(start.x().bits());
    let dy = i128::from(end.y().bits()) - i128::from(start.y().bits());
    let px = i128::from(point.x().bits()) - i128::from(start.x().bits());
    let py = i128::from(point.y().bits()) - i128::from(start.y().bits());
    let numerator = px
        .checked_mul(dx)
        .and_then(|value| {
            py.checked_mul(dy)
                .and_then(|other| value.checked_add(other))
        })
        .and_then(|value| value.checked_mul(GRADIENT_SCALE))
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    let denominator = dx
        .checked_mul(dx)
        .and_then(|value| {
            dy.checked_mul(dy)
                .and_then(|other| value.checked_add(other))
        })
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    if denominator == 0 {
        return Err(SkiaError::new(SkiaErrorCode::InvalidGeometry));
    }
    Ok(rounded_ratio(numerator, denominator))
}

fn radial_parameter(center: Point, radius: Scalar, point: Point) -> Result<i128, SkiaError> {
    let dx = i128::from(point.x().bits()) - i128::from(center.x().bits());
    let dy = i128::from(point.y().bits()) - i128::from(center.y().bits());
    let squared = dx
        .checked_mul(dx)
        .and_then(|value| {
            dy.checked_mul(dy)
                .and_then(|other| value.checked_add(other))
        })
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    let distance = squared.unsigned_abs().isqrt();
    let numerator = i128::try_from(distance)
        .ok()
        .and_then(|distance| distance.checked_mul(GRADIENT_SCALE))
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    Ok(rounded_ratio(numerator, i128::from(radius.bits())))
}

fn tile_parameter(parameter: i128, tile_mode: TileMode) -> i32 {
    let tiled = match tile_mode {
        TileMode::Clamp => parameter.clamp(0, GRADIENT_SCALE),
        TileMode::Repeat => parameter.rem_euclid(GRADIENT_SCALE),
        TileMode::Mirror => {
            let period = GRADIENT_SCALE * 2;
            let value = parameter.rem_euclid(period);
            if value > GRADIENT_SCALE {
                period - value
            } else {
                value
            }
        }
    };
    i32::try_from(tiled).unwrap_or_default()
}

fn sample_stops(stops: &[GradientStop], parameter: i32) -> Color {
    let first = stops[0];
    if parameter <= first.offset.bits() {
        return first.color;
    }
    for pair in stops.windows(2) {
        let start = pair[0];
        let end = pair[1];
        if parameter <= end.offset.bits() {
            let span = i128::from(end.offset.bits() - start.offset.bits());
            if span == 0 {
                return end.color;
            }
            let offset = i128::from(parameter - start.offset.bits());
            return interpolate_color(start.color, end.color, offset, span);
        }
    }
    stops[stops.len() - 1].color
}

fn interpolate_color(start: Color, end: Color, offset: i128, span: i128) -> Color {
    let start = start.channels();
    let end = end.channels();
    let mut output = [0_u8; 4];
    for index in 0..4 {
        let value = i128::from(start[index]) * (span - offset) + i128::from(end[index]) * offset;
        output[index] = u8::try_from((value + span / 2) / span).unwrap_or_default();
    }
    Color::rgba(output[0], output[1], output[2], output[3])
}

pub(crate) fn rounded_ratio(numerator: i128, denominator: i128) -> i128 {
    let rounded =
        (numerator.unsigned_abs() + denominator.unsigned_abs() / 2) / denominator.unsigned_abs();
    let rounded = i128::try_from(rounded).unwrap_or(i128::MAX);
    if (numerator < 0) == (denominator < 0) {
        rounded
    } else {
        -rounded
    }
}

pub(crate) fn rounded_shift_q16(value: i128) -> i128 {
    if value >= 0 {
        (value + (1 << 15)) >> 16
    } else {
        -((-value + (1 << 15)) >> 16)
    }
}
