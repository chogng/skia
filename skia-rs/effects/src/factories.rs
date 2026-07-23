/// Built-in color-filter factories.
pub mod color_filters {
    use skia_core::{BlendMode, Color, ColorFilter, ColorMatrix};

    /// Creates a fixed-point 4×5 straight-RGBA matrix filter.
    pub const fn matrix(matrix: ColorMatrix) -> ColorFilter {
        ColorFilter::Matrix(matrix)
    }

    /// Creates a constant-color blend filter.
    pub const fn blend(color: Color, mode: BlendMode) -> ColorFilter {
        ColorFilter::Blend { color, mode }
    }
}

/// Built-in whole-layer image-filter factories.
pub mod image_filters {
    use skia_core::{ColorFilter, ImageFilter, SkiaError};

    /// Creates a per-pixel color-filter image effect.
    pub const fn color(filter: ColorFilter) -> ImageFilter {
        ImageFilter::Color(filter)
    }

    /// Creates a separable transparent-edge box blur.
    pub const fn box_blur(radius: u8) -> Result<ImageFilter, SkiaError> {
        ImageFilter::box_blur(radius)
    }
}

/// Built-in paint-source shader factories.
pub mod shaders {
    use skia_core::{Gradient, GradientStop, Point, Scalar, SkiaError, TileMode};

    /// Creates a bounded local-space linear gradient shader.
    pub fn linear_gradient(
        start: Point,
        end: Point,
        stops: &[GradientStop],
        tile_mode: TileMode,
    ) -> Result<Gradient, SkiaError> {
        Gradient::linear(start, end, stops, tile_mode)
    }

    /// Creates a bounded local-space radial gradient shader.
    pub fn radial_gradient(
        center: Point,
        radius: Scalar,
        stops: &[GradientStop],
        tile_mode: TileMode,
    ) -> Result<Gradient, SkiaError> {
        Gradient::radial(center, radius, stops, tile_mode)
    }
}
