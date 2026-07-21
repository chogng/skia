use skia_error::{SkiaError, SkiaErrorCode};
use skia_geometry::{Point, Scalar};

const GRADIENT_SCALE: i128 = 1 << 16;

/// One straight-alpha sRGBA8 color.
///
/// The RGB channels are deliberately retained even when alpha is zero. This
/// makes `Color` suitable for image pixels as well as constant paint state;
/// compositing canonicalizes a fully transparent result to [`Color::TRANSPARENT`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Color {
    red: u8,
    green: u8,
    blue: u8,
    alpha: u8,
}

impl Color {
    /// Fully transparent black.
    pub const TRANSPARENT: Self = Self::rgba(0, 0, 0, 0);

    /// Opaque black.
    pub const BLACK: Self = Self::rgb(0, 0, 0);

    /// Opaque white.
    pub const WHITE: Self = Self::rgb(u8::MAX, u8::MAX, u8::MAX);

    /// Opaque red.
    pub const RED: Self = Self::rgb(u8::MAX, 0, 0);

    /// Opaque green.
    pub const GREEN: Self = Self::rgb(0, u8::MAX, 0);

    /// Opaque blue.
    pub const BLUE: Self = Self::rgb(0, 0, u8::MAX);

    /// Creates an opaque sRGBA8 color.
    pub const fn rgb(red: u8, green: u8, blue: u8) -> Self {
        Self::rgba(red, green, blue, u8::MAX)
    }

    /// Creates a straight-alpha sRGBA8 color.
    pub const fn rgba(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }

    /// Decodes an `0xAARRGGBB` color value.
    pub const fn from_argb(value: u32) -> Self {
        Self::rgba(
            (value >> 16) as u8,
            (value >> 8) as u8,
            value as u8,
            (value >> 24) as u8,
        )
    }

    /// Decodes an `0xRRGGBBAA` color value.
    pub const fn from_rgba_u32(value: u32) -> Self {
        Self::rgba(
            (value >> 24) as u8,
            (value >> 16) as u8,
            (value >> 8) as u8,
            value as u8,
        )
    }

    /// Returns the red channel.
    pub const fn red(self) -> u8 {
        self.red
    }

    /// Returns the green channel.
    pub const fn green(self) -> u8 {
        self.green
    }

    /// Returns the blue channel.
    pub const fn blue(self) -> u8 {
        self.blue
    }

    /// Returns the alpha channel.
    pub const fn alpha(self) -> u8 {
        self.alpha
    }

    /// Returns channels in top-level RGBA order.
    pub const fn channels(self) -> [u8; 4] {
        [self.red, self.green, self.blue, self.alpha]
    }

    /// Returns the color as `0xAARRGGBB`.
    pub const fn argb(self) -> u32 {
        ((self.alpha as u32) << 24)
            | ((self.red as u32) << 16)
            | ((self.green as u32) << 8)
            | self.blue as u32
    }

    /// Returns the color as `0xRRGGBBAA`.
    pub const fn rgba_u32(self) -> u32 {
        ((self.red as u32) << 24)
            | ((self.green as u32) << 16)
            | ((self.blue as u32) << 8)
            | self.alpha as u32
    }

    /// Replaces the alpha channel without changing the RGB channels.
    pub const fn with_alpha(self, alpha: u8) -> Self {
        Self::rgba(self.red, self.green, self.blue, alpha)
    }

    /// Multiplies the alpha channel by an 8-bit opacity factor.
    pub fn with_opacity(self, opacity: u8) -> Self {
        self.with_alpha(to_u8(mul_255(u32::from(self.alpha), u32::from(opacity))))
    }

    /// Returns whether alpha is fully opaque.
    pub const fn is_opaque(self) -> bool {
        self.alpha == u8::MAX
    }

    /// Returns whether alpha is fully transparent.
    pub const fn is_transparent(self) -> bool {
        self.alpha == 0
    }

    /// Composites `self` over `destination` using `blend_mode`.
    pub fn composite(self, destination: Self, blend_mode: BlendMode) -> Self {
        blend_mode.composite(self, destination)
    }
}

/// Compositing operation for source and destination pixels.
///
/// Names use fully spelled-out source and destination terms.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BlendMode {
    /// Clears both source and destination.
    Clear,
    /// Replaces the destination with the source.
    Source,
    /// Preserves the destination.
    Destination,
    /// Standard source-over compositing.
    SourceOver,
    /// Destination-over compositing.
    DestinationOver,
    /// Keeps source covered by destination alpha.
    SourceIn,
    /// Keeps destination covered by source alpha.
    DestinationIn,
    /// Keeps source outside destination alpha.
    SourceOut,
    /// Keeps destination outside source alpha.
    DestinationOut,
    /// Keeps source atop destination.
    SourceAtop,
    /// Keeps destination atop source.
    DestinationAtop,
    /// Keeps pixels covered by exactly one input.
    Xor,
    /// Adds premultiplied components with saturation.
    Plus,
    /// Multiplies premultiplied source and destination components.
    Modulate,
    /// Multiplies source and destination colors.
    Multiply,
    /// Screens source and destination colors.
    Screen,
    /// Uses the destination to select multiply or screen.
    Overlay,
    /// Selects the darker source/destination color per channel.
    Darken,
    /// Selects the lighter source/destination color per channel.
    Lighten,
    /// Brightens the destination to reflect the source.
    ColorDodge,
    /// Darkens the destination to reflect the source.
    ColorBurn,
    /// Uses the source to select multiply or screen.
    HardLight,
    /// Applies the soft-light contrast curve.
    SoftLight,
    /// Uses the absolute channel difference.
    Difference,
    /// Uses a reduced channel difference.
    Exclusion,
    /// Takes hue from source and saturation/luminance from destination.
    Hue,
    /// Takes saturation from source and hue/luminance from destination.
    Saturation,
    /// Takes hue/saturation from source and luminance from destination.
    Color,
    /// Takes luminance from source and hue/saturation from destination.
    Luminosity,
}

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

/// Fixed-point 4×5 straight-RGBA color transform.
///
/// Coefficients and biases are signed Q16.16 values. Each output row multiplies
/// input channels in RGBA order and adds its bias in 8-bit channel units.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ColorMatrix {
    values: [i32; 20],
}

impl ColorMatrix {
    /// Identity color transform.
    pub const IDENTITY: Self = Self::new([
        1 << 16,
        0,
        0,
        0,
        0,
        0,
        1 << 16,
        0,
        0,
        0,
        0,
        0,
        1 << 16,
        0,
        0,
        0,
        0,
        0,
        1 << 16,
        0,
    ]);

    /// Creates a matrix from four consecutive five-value rows.
    pub const fn new(values: [i32; 20]) -> Self {
        Self { values }
    }

    /// Returns the exact row-major Q16.16 values.
    pub const fn values(self) -> [i32; 20] {
        self.values
    }

    /// Applies this matrix and clamps every output channel to `[0, 255]`.
    pub fn apply(self, color: Color) -> Color {
        let input = color.channels();
        let mut output = [0_u8; 4];
        for (row, output) in output.iter_mut().enumerate() {
            let values = &self.values[row * 5..row * 5 + 5];
            let total = values[..4].iter().zip(input).fold(
                i128::from(values[4]),
                |total, (coefficient, channel)| {
                    total + i128::from(*coefficient) * i128::from(channel)
                },
            );
            let rounded = rounded_shift_q16(total).clamp(0, 255);
            *output = u8::try_from(rounded).unwrap_or(0);
        }
        Color::rgba(output[0], output[1], output[2], output[3])
    }
}

/// Per-source color transformation applied before compositing.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ColorFilter {
    /// Applies a fixed-point 4×5 matrix.
    Matrix(ColorMatrix),
    /// Composites a constant filter color over the source color.
    Blend {
        /// Constant filter source.
        color: Color,
        /// Blend operation between the constant and original source.
        mode: BlendMode,
    },
}

impl ColorFilter {
    /// Applies the filter to one straight-alpha color.
    pub fn apply(self, source: Color) -> Color {
        match self {
            Self::Matrix(matrix) => matrix.apply(source),
            Self::Blend { color, mode } => color.composite(source, mode),
        }
    }
}

/// Whole-layer image processing performed before restore compositing.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ImageFilter {
    /// Applies one color filter independently to every layer pixel.
    Color(ColorFilter),
    /// Applies a separable transparent-edge box blur.
    BoxBlur {
        /// Positive integer kernel radius in device pixels.
        radius: u8,
    },
}

impl ImageFilter {
    /// Creates a positive box blur with a radius no larger than 64 pixels.
    pub const fn box_blur(radius: u8) -> Result<Self, SkiaError> {
        if radius == 0 || radius > 64 {
            return Err(SkiaError::new(SkiaErrorCode::InvalidGeometry));
        }
        Ok(Self::BoxBlur { radius })
    }
}

/// Restore-time compositing policy for one isolated layer.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SaveLayerOptions {
    bounds: Option<skia_geometry::Rect>,
    opacity: u8,
    blend_mode: BlendMode,
    filter: Option<ImageFilter>,
}

impl SaveLayerOptions {
    /// Creates a full-clip, opaque, source-over layer without a filter.
    pub const fn new() -> Self {
        Self {
            bounds: None,
            opacity: u8::MAX,
            blend_mode: BlendMode::SourceOver,
            filter: None,
        }
    }

    /// Restricts restore compositing to transformed logical bounds.
    pub const fn with_bounds(mut self, bounds: skia_geometry::Rect) -> Self {
        self.bounds = Some(bounds);
        self
    }

    /// Selects restore-time source opacity.
    pub const fn with_opacity(mut self, opacity: u8) -> Self {
        self.opacity = opacity;
        self
    }

    /// Selects restore-time source/destination compositing.
    pub const fn with_blend_mode(mut self, blend_mode: BlendMode) -> Self {
        self.blend_mode = blend_mode;
        self
    }

    /// Selects one restore-time image filter.
    pub const fn with_filter(mut self, filter: ImageFilter) -> Self {
        self.filter = Some(filter);
        self
    }

    /// Returns optional logical layer bounds.
    pub const fn bounds(self) -> Option<skia_geometry::Rect> {
        self.bounds
    }

    /// Returns restore-time source opacity.
    pub const fn opacity(self) -> u8 {
        self.opacity
    }

    /// Returns restore-time compositing mode.
    pub const fn blend_mode(self) -> BlendMode {
        self.blend_mode
    }

    /// Returns the optional restore-time filter.
    pub const fn filter(self) -> Option<ImageFilter> {
        self.filter
    }
}

impl Default for SaveLayerOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl BlendMode {
    /// Returns whether this is a Porter-Duff operation.
    pub const fn is_porter_duff(self) -> bool {
        matches!(
            self,
            Self::Clear
                | Self::Source
                | Self::Destination
                | Self::SourceOver
                | Self::DestinationOver
                | Self::SourceIn
                | Self::DestinationIn
                | Self::SourceOut
                | Self::DestinationOut
                | Self::SourceAtop
                | Self::DestinationAtop
                | Self::Xor
                | Self::Plus
                | Self::Modulate
        )
    }

    /// Composites `source` over `destination`.
    ///
    /// Color values use straight alpha at the API boundary. Calculations use
    /// rounded premultiplied 8-bit values, and transparent results are
    /// canonicalized to transparent black.
    pub fn composite(self, source: Color, destination: Color) -> Color {
        if matches!(self, Self::SourceOver) && destination.is_transparent() {
            return source.canonicalized();
        }
        if matches!(self, Self::SourceOver) && source.is_transparent() {
            return destination.canonicalized();
        }
        match self {
            Self::Clear => Color::TRANSPARENT,
            Self::Plus => plus(source, destination),
            Self::Modulate => modulate(source, destination),
            Self::Source => source.canonicalized(),
            Self::Destination => destination.canonicalized(),
            Self::SourceOver => {
                porter_duff(source, destination, 255, 255 - u32::from(source.alpha))
            }
            Self::DestinationOver => {
                porter_duff(source, destination, 255 - u32::from(destination.alpha), 255)
            }
            Self::SourceIn => porter_duff(source, destination, u32::from(destination.alpha), 0),
            Self::DestinationIn => porter_duff(source, destination, 0, u32::from(source.alpha)),
            Self::SourceOut => {
                porter_duff(source, destination, 255 - u32::from(destination.alpha), 0)
            }
            Self::DestinationOut => {
                porter_duff(source, destination, 0, 255 - u32::from(source.alpha))
            }
            Self::SourceAtop => porter_duff(
                source,
                destination,
                u32::from(destination.alpha),
                255 - u32::from(source.alpha),
            ),
            Self::DestinationAtop => porter_duff(
                source,
                destination,
                255 - u32::from(destination.alpha),
                u32::from(source.alpha),
            ),
            Self::Xor => porter_duff(
                source,
                destination,
                255 - u32::from(destination.alpha),
                255 - u32::from(source.alpha),
            ),
            Self::Multiply => separable(source, destination, multiply),
            Self::Screen => separable(source, destination, screen),
            Self::Overlay => separable(source, destination, overlay),
            Self::Darken => separable(source, destination, |source, destination| {
                source.min(destination)
            }),
            Self::Lighten => separable(source, destination, |source, destination| {
                source.max(destination)
            }),
            Self::ColorDodge => separable(source, destination, color_dodge),
            Self::ColorBurn => separable(source, destination, color_burn),
            Self::HardLight => separable(source, destination, |source, destination| {
                overlay(destination, source)
            }),
            Self::SoftLight => separable(source, destination, soft_light),
            Self::Difference => separable(source, destination, |source, destination| {
                source.abs_diff(destination)
            }),
            Self::Exclusion => separable(source, destination, exclusion),
            Self::Hue => non_separable(source, destination, hue),
            Self::Saturation => non_separable(source, destination, saturation),
            Self::Color => non_separable(source, destination, color),
            Self::Luminosity => non_separable(source, destination, luminosity),
        }
    }
}

/// Immutable constant-color paint selected for one draw operation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Paint {
    color: Color,
    blend_mode: BlendMode,
    gradient: Option<Gradient>,
    color_filter: Option<ColorFilter>,
}

impl Paint {
    /// Creates one source-over paint.
    pub const fn new(color: Color) -> Self {
        Self {
            color,
            blend_mode: BlendMode::SourceOver,
            gradient: None,
            color_filter: None,
        }
    }

    /// Creates a source-over gradient paint with full opacity.
    pub const fn from_gradient(gradient: Gradient) -> Self {
        Self {
            color: Color::WHITE,
            blend_mode: BlendMode::SourceOver,
            gradient: Some(gradient),
            color_filter: None,
        }
    }

    /// Selects the source color.
    pub const fn with_color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    /// Replaces the source alpha while retaining its RGB channels.
    pub const fn with_alpha(mut self, alpha: u8) -> Self {
        self.color = self.color.with_alpha(alpha);
        self
    }

    /// Multiplies the source alpha by one 8-bit opacity factor.
    pub fn with_opacity(mut self, opacity: u8) -> Self {
        self.color = self.color.with_opacity(opacity);
        self
    }

    /// Selects a compositing operation.
    pub const fn with_blend_mode(mut self, blend_mode: BlendMode) -> Self {
        self.blend_mode = blend_mode;
        self
    }

    /// Selects a local-space gradient; the current color alpha modulates it.
    pub const fn with_gradient(mut self, gradient: Gradient) -> Self {
        self.gradient = Some(gradient);
        self
    }

    /// Restores constant-color source evaluation.
    pub const fn without_gradient(mut self) -> Self {
        self.gradient = None;
        self
    }

    /// Selects a pre-compositing color filter.
    pub const fn with_color_filter(mut self, color_filter: ColorFilter) -> Self {
        self.color_filter = Some(color_filter);
        self
    }

    /// Removes the pre-compositing color filter.
    pub const fn without_color_filter(mut self) -> Self {
        self.color_filter = None;
        self
    }

    /// Returns the straight source color.
    pub const fn color(self) -> Color {
        self.color
    }

    /// Returns the compositing operation.
    pub const fn blend_mode(self) -> BlendMode {
        self.blend_mode
    }

    /// Returns the optional local-space gradient.
    pub const fn gradient(self) -> Option<Gradient> {
        self.gradient
    }

    /// Returns the optional pre-compositing color filter.
    pub const fn color_filter(self) -> Option<ColorFilter> {
        self.color_filter
    }

    /// Evaluates this paint's source at one local-space point.
    pub fn source_color(self, point: Point) -> Result<Color, SkiaError> {
        let source = if let Some(gradient) = self.gradient {
            gradient.sample(point)?.with_opacity(self.color.alpha())
        } else {
            self.color
        };
        Ok(self.filter_color(source))
    }

    /// Applies only this paint's color filter to an externally supplied source.
    pub fn filter_color(self, source: Color) -> Color {
        self.color_filter
            .map_or(source, |filter| filter.apply(source))
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

fn rounded_ratio(numerator: i128, denominator: i128) -> i128 {
    let rounded =
        (numerator.unsigned_abs() + denominator.unsigned_abs() / 2) / denominator.unsigned_abs();
    let rounded = i128::try_from(rounded).unwrap_or(i128::MAX);
    if (numerator < 0) == (denominator < 0) {
        rounded
    } else {
        -rounded
    }
}

fn rounded_shift_q16(value: i128) -> i128 {
    if value >= 0 {
        (value + (1 << 15)) >> 16
    } else {
        -((-value + (1 << 15)) >> 16)
    }
}

impl Default for Paint {
    fn default() -> Self {
        Self::new(Color::BLACK)
    }
}

impl Color {
    fn canonicalized(self) -> Self {
        if self.alpha == 0 {
            Self::TRANSPARENT
        } else {
            self
        }
    }
}

fn porter_duff(
    source: Color,
    destination: Color,
    source_factor: u32,
    destination_factor: u32,
) -> Color {
    let source = Premul::from(source);
    let destination = Premul::from(destination);
    Premul {
        red: add_sat(
            mul_255(source.red, source_factor),
            mul_255(destination.red, destination_factor),
        ),
        green: add_sat(
            mul_255(source.green, source_factor),
            mul_255(destination.green, destination_factor),
        ),
        blue: add_sat(
            mul_255(source.blue, source_factor),
            mul_255(destination.blue, destination_factor),
        ),
        alpha: add_sat(
            mul_255(source.alpha, source_factor),
            mul_255(destination.alpha, destination_factor),
        ),
    }
    .into()
}

fn plus(source: Color, destination: Color) -> Color {
    let source = Premul::from(source);
    let destination = Premul::from(destination);
    Premul {
        red: add_sat(source.red, destination.red),
        green: add_sat(source.green, destination.green),
        blue: add_sat(source.blue, destination.blue),
        alpha: add_sat(source.alpha, destination.alpha),
    }
    .into()
}

fn modulate(source: Color, destination: Color) -> Color {
    let source = Premul::from(source);
    let destination = Premul::from(destination);
    Premul {
        red: mul_255(source.red, destination.red),
        green: mul_255(source.green, destination.green),
        blue: mul_255(source.blue, destination.blue),
        alpha: mul_255(source.alpha, destination.alpha),
    }
    .into()
}

fn separable(source: Color, destination: Color, blend: impl Fn(u32, u32) -> u32) -> Color {
    let source_premul = Premul::from(source);
    let destination_premul = Premul::from(destination);
    let alpha = add_sat(
        source_premul.alpha,
        mul_255(destination_premul.alpha, 255 - source_premul.alpha),
    );
    let channel = |source: u8, destination: u8, source_channel: u32, destination_channel: u32| {
        let outside_source = mul_255(source_channel, 255 - destination_premul.alpha);
        let outside_destination = mul_255(destination_channel, 255 - source_premul.alpha);
        let overlap = mul_255(
            mul_255(
                blend(u32::from(source), u32::from(destination)),
                source_premul.alpha,
            ),
            destination_premul.alpha,
        );
        add_sat(add_sat(outside_source, outside_destination), overlap)
    };
    Premul {
        red: channel(
            source.red,
            destination.red,
            source_premul.red,
            destination_premul.red,
        ),
        green: channel(
            source.green,
            destination.green,
            source_premul.green,
            destination_premul.green,
        ),
        blue: channel(
            source.blue,
            destination.blue,
            source_premul.blue,
            destination_premul.blue,
        ),
        alpha,
    }
    .into()
}

fn non_separable(
    source: Color,
    destination: Color,
    blend: impl Fn([i32; 3], [i32; 3]) -> [i32; 3],
) -> Color {
    let source_premul = Premul::from(source);
    let destination_premul = Premul::from(destination);
    let alpha = add_sat(
        source_premul.alpha,
        mul_255(destination_premul.alpha, 255 - source_premul.alpha),
    );
    let blended = blend(
        [
            i32::from(source.red),
            i32::from(source.green),
            i32::from(source.blue),
        ],
        [
            i32::from(destination.red),
            i32::from(destination.green),
            i32::from(destination.blue),
        ],
    );
    let channel = |source: u32, destination: u32, blended: i32| {
        add_sat(
            add_sat(
                mul_255(source, 255 - destination_premul.alpha),
                mul_255(destination, 255 - source_premul.alpha),
            ),
            mul_255(
                mul_255(blended.clamp(0, 255) as u32, source_premul.alpha),
                destination_premul.alpha,
            ),
        )
    };
    Premul {
        red: channel(source_premul.red, destination_premul.red, blended[0]),
        green: channel(source_premul.green, destination_premul.green, blended[1]),
        blue: channel(source_premul.blue, destination_premul.blue, blended[2]),
        alpha,
    }
    .into()
}

fn multiply(source: u32, destination: u32) -> u32 {
    mul_255(source, destination)
}
fn screen(source: u32, destination: u32) -> u32 {
    source + destination - mul_255(source, destination)
}
fn overlay(source: u32, destination: u32) -> u32 {
    if destination <= 127 {
        mul_255(2 * source, destination)
    } else {
        255 - mul_255(2 * (255 - source), 255 - destination)
    }
}
fn color_dodge(source: u32, destination: u32) -> u32 {
    if source == 255 {
        255
    } else {
        (destination * 255 / (255 - source)).min(255)
    }
}
fn color_burn(source: u32, destination: u32) -> u32 {
    ((255 - destination) * 255)
        .checked_div(source)
        .map_or(0, |value| 255 - value.min(255))
}
fn soft_light(source: u32, destination: u32) -> u32 {
    if source <= 127 {
        destination - mul_255(mul_255(255 - 2 * source, destination), 255 - destination)
    } else {
        let dark = if destination <= 63 {
            ((16 * destination - 12 * 255) * destination + 4 * 255 * 255) * destination
                / (255 * 255)
        } else {
            integer_sqrt(destination * 255)
        };
        destination + mul_255(2 * source - 255, dark - destination)
    }
}
fn exclusion(source: u32, destination: u32) -> u32 {
    source + destination - 2 * mul_255(source, destination)
}

fn hue(source: [i32; 3], destination: [i32; 3]) -> [i32; 3] {
    set_lum(set_sat(source, sat(destination)), lum(destination))
}
fn saturation(source: [i32; 3], destination: [i32; 3]) -> [i32; 3] {
    set_lum(set_sat(destination, sat(source)), lum(destination))
}
fn color(source: [i32; 3], destination: [i32; 3]) -> [i32; 3] {
    set_lum(source, lum(destination))
}
fn luminosity(source: [i32; 3], destination: [i32; 3]) -> [i32; 3] {
    set_lum(destination, lum(source))
}

fn lum(color: [i32; 3]) -> i32 {
    (77 * color[0] + 150 * color[1] + 29 * color[2] + 128) / 256
}
fn sat(color: [i32; 3]) -> i32 {
    color.into_iter().max().unwrap_or(0) - color.into_iter().min().unwrap_or(0)
}
fn set_lum(mut color: [i32; 3], target: i32) -> [i32; 3] {
    let delta = target - lum(color);
    for channel in &mut color {
        *channel += delta;
    }
    clip_color(color)
}
fn clip_color(mut color: [i32; 3]) -> [i32; 3] {
    let luminance = lum(color);
    let minimum = color.into_iter().min().unwrap_or(0);
    let maximum = color.into_iter().max().unwrap_or(0);
    if minimum < 0 {
        for channel in &mut color {
            *channel = luminance + (*channel - luminance) * luminance / (luminance - minimum);
        }
    }
    if maximum > 255 {
        for channel in &mut color {
            *channel =
                luminance + (*channel - luminance) * (255 - luminance) / (maximum - luminance);
        }
    }
    color.map(|channel| channel.clamp(0, 255))
}
fn set_sat(color: [i32; 3], target: i32) -> [i32; 3] {
    let mut order = [0_usize, 1, 2];
    order.sort_by_key(|&index| color[index]);
    let mut result = [0; 3];
    let minimum = color[order[0]];
    let maximum = color[order[2]];
    if maximum > minimum {
        result[order[1]] = (color[order[1]] - minimum) * target / (maximum - minimum);
        result[order[2]] = target;
    }
    result
}

#[derive(Clone, Copy)]
struct Premul {
    red: u32,
    green: u32,
    blue: u32,
    alpha: u32,
}
impl From<Color> for Premul {
    fn from(color: Color) -> Self {
        Self {
            red: mul_255(u32::from(color.red), u32::from(color.alpha)),
            green: mul_255(u32::from(color.green), u32::from(color.alpha)),
            blue: mul_255(u32::from(color.blue), u32::from(color.alpha)),
            alpha: u32::from(color.alpha),
        }
    }
}
impl From<Premul> for Color {
    fn from(value: Premul) -> Self {
        if value.alpha == 0 {
            return Self::TRANSPARENT;
        }
        Self::rgba(
            to_u8(round_div(value.red * 255, value.alpha)),
            to_u8(round_div(value.green * 255, value.alpha)),
            to_u8(round_div(value.blue * 255, value.alpha)),
            to_u8(value.alpha),
        )
    }
}

fn mul_255(left: u32, right: u32) -> u32 {
    round_div(left * right, 255)
}
fn round_div(numerator: u32, denominator: u32) -> u32 {
    (numerator + denominator / 2) / denominator
}
fn add_sat(left: u32, right: u32) -> u32 {
    left.saturating_add(right).min(255)
}
fn to_u8(value: u32) -> u8 {
    u8::try_from(value.min(255)).unwrap_or(u8::MAX)
}
fn integer_sqrt(value: u32) -> u32 {
    let mut low = 0_u32;
    let mut high = 256_u32;
    while low < high {
        let middle = (low + high).div_ceil(2);
        if middle * middle <= value {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    low
}

#[cfg(test)]
mod tests {
    use skia_geometry::{Point, Scalar};

    use super::{
        BlendMode, Color, ColorFilter, ColorMatrix, Gradient, GradientStop, Paint, TileMode,
    };

    #[test]
    fn color_constructors_pack_channels_and_apply_opacity() {
        let color = Color::rgba(1, 2, 3, 128);
        assert_eq!(color.channels(), [1, 2, 3, 128]);
        assert_eq!(color.argb(), 0x8001_0203);
        assert_eq!(color.rgba_u32(), 0x0102_0380);
        assert_eq!(Color::from_argb(0x8001_0203), color);
        assert_eq!(Color::from_rgba_u32(0x0102_0380), color);
        assert_eq!(color.with_alpha(64).alpha(), 64);
        assert_eq!(color.with_opacity(128).alpha(), 64);
        assert!(Color::BLACK.is_opaque());
        assert!(Color::TRANSPARENT.is_transparent());
    }

    #[test]
    fn paint_builders_are_immutable_and_default_to_black_source_over() {
        let paint = Paint::default()
            .with_color(Color::RED)
            .with_alpha(64)
            .with_opacity(128)
            .with_blend_mode(BlendMode::Screen);
        assert_eq!(Paint::default(), Paint::new(Color::BLACK));
        assert_eq!(paint.color(), Color::rgba(255, 0, 0, 32));
        assert_eq!(paint.blend_mode(), BlendMode::Screen);
    }

    #[test]
    fn porter_duff_modes_cover_source_destination_and_alpha_edges() {
        let source = Color::rgba(255, 0, 0, 128);
        let destination = Color::rgba(0, 0, 255, 255);
        assert_eq!(
            source.composite(destination, BlendMode::SourceOver),
            Color::rgba(128, 0, 127, 255)
        );
        assert_eq!(
            source.composite(destination, BlendMode::DestinationOver),
            destination
        );
        assert_eq!(source.composite(destination, BlendMode::SourceIn), source);
        assert_eq!(
            source.composite(destination, BlendMode::DestinationOut),
            Color::rgba(0, 0, 255, 127)
        );
        assert_eq!(
            source.composite(Color::TRANSPARENT, BlendMode::SourceOver),
            source
        );
    }

    #[test]
    fn advanced_modes_are_defined_for_every_8_bit_input() {
        let source = Color::rgba(100, 200, 50, 192);
        let destination = Color::rgba(200, 100, 250, 128);
        let modes = [
            BlendMode::Clear,
            BlendMode::Source,
            BlendMode::Destination,
            BlendMode::SourceOver,
            BlendMode::DestinationOver,
            BlendMode::SourceIn,
            BlendMode::DestinationIn,
            BlendMode::SourceOut,
            BlendMode::DestinationOut,
            BlendMode::SourceAtop,
            BlendMode::DestinationAtop,
            BlendMode::Xor,
            BlendMode::Plus,
            BlendMode::Modulate,
            BlendMode::Multiply,
            BlendMode::Screen,
            BlendMode::Overlay,
            BlendMode::Darken,
            BlendMode::Lighten,
            BlendMode::ColorDodge,
            BlendMode::ColorBurn,
            BlendMode::HardLight,
            BlendMode::SoftLight,
            BlendMode::Difference,
            BlendMode::Exclusion,
            BlendMode::Hue,
            BlendMode::Saturation,
            BlendMode::Color,
            BlendMode::Luminosity,
        ];
        for mode in modes {
            let _color = source.composite(destination, mode);
        }
        assert_eq!(
            Color::rgb(100, 200, 50).composite(Color::rgb(200, 100, 250), BlendMode::Multiply),
            Color::rgb(78, 78, 49)
        );
        assert_eq!(
            Color::rgb(100, 200, 50).composite(Color::rgb(200, 100, 250), BlendMode::Screen),
            Color::rgb(222, 222, 251)
        );
    }

    #[test]
    fn linear_gradient_interpolates_tiles_and_modulates_alpha() {
        let point = |x| Point::new(Scalar::from_bits(x), Scalar::ZERO);
        let stops = [
            GradientStop::new(Scalar::ZERO, Color::RED).expect("red"),
            GradientStop::new(Scalar::from_bits(1 << 16), Color::BLUE).expect("blue"),
        ];
        let gradient =
            Gradient::linear(point(0), point(4 << 16), &stops, TileMode::Clamp).expect("linear");
        assert_eq!(gradient.sample(point(0)).expect("start"), Color::RED);
        assert_eq!(
            gradient.sample(point(2 << 16)).expect("middle"),
            Color::rgb(128, 0, 128)
        );
        assert_eq!(gradient.sample(point(8 << 16)).expect("clamp"), Color::BLUE);
        assert_eq!(
            Paint::from_gradient(gradient)
                .with_alpha(128)
                .source_color(point(2 << 16))
                .expect("paint"),
            Color::rgba(128, 0, 128, 128)
        );

        let repeated =
            Gradient::linear(point(0), point(4 << 16), &stops, TileMode::Repeat).expect("repeat");
        assert_eq!(
            repeated.sample(point(5 << 16)).expect("repeat sample"),
            Color::rgb(191, 0, 64)
        );
        let mirrored =
            Gradient::linear(point(0), point(4 << 16), &stops, TileMode::Mirror).expect("mirror");
        assert_eq!(
            mirrored.sample(point(5 << 16)).expect("mirror sample"),
            Color::rgb(64, 0, 191)
        );
    }

    #[test]
    fn radial_gradient_and_color_filters_are_deterministic() {
        let center = Point::new(Scalar::ZERO, Scalar::ZERO);
        let stops = [
            GradientStop::new(Scalar::ZERO, Color::WHITE).expect("white"),
            GradientStop::new(Scalar::from_bits(1 << 16), Color::BLACK).expect("black"),
        ];
        let gradient =
            Gradient::radial(center, Scalar::from_bits(10 << 16), &stops, TileMode::Clamp)
                .expect("radial");
        assert_eq!(
            gradient
                .sample(Point::new(
                    Scalar::from_bits(6 << 16),
                    Scalar::from_bits(8 << 16)
                ))
                .expect("edge"),
            Color::BLACK
        );

        let grayscale = ColorMatrix::new([
            21_845,
            21_845,
            21_845,
            0,
            0,
            21_845,
            21_845,
            21_845,
            0,
            0,
            21_845,
            21_845,
            21_845,
            0,
            0,
            0,
            0,
            0,
            1 << 16,
            0,
        ]);
        assert_eq!(
            grayscale.apply(Color::rgba(30, 60, 90, 128)),
            Color::rgba(60, 60, 60, 128)
        );
        let filtered = Paint::new(Color::BLUE).with_color_filter(ColorFilter::Blend {
            color: Color::rgba(255, 0, 0, 128),
            mode: BlendMode::SourceOver,
        });
        assert_eq!(
            filtered.source_color(center).expect("filter"),
            Color::rgba(128, 0, 127, 255)
        );
    }

    #[test]
    fn gradients_validate_geometry_stop_order_and_capacity() {
        let point = Point::new(Scalar::ZERO, Scalar::ZERO);
        let stop = GradientStop::new(Scalar::ZERO, Color::BLACK).expect("stop");
        assert!(Gradient::linear(point, point, &[stop, stop], TileMode::Clamp).is_err());
        assert!(Gradient::radial(point, Scalar::ZERO, &[stop, stop], TileMode::Clamp).is_err());
        assert!(
            Gradient::linear(
                point,
                Point::new(Scalar::from_bits(1 << 16), Scalar::ZERO),
                &[stop],
                TileMode::Clamp,
            )
            .is_err()
        );
        assert!(GradientStop::new(Scalar::from_bits((1 << 16) + 1), Color::BLACK).is_err());
    }
}
