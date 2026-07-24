/// Built-in color-filter factories.
pub mod color_filters {
    use skia_core::{BlendMode, Color, ColorFilter, ColorFilterHandle, ColorMatrix};

    /// Creates a fixed-point 4×5 straight-RGBA matrix filter.
    pub const fn matrix(matrix: ColorMatrix) -> ColorFilter {
        ColorFilter::Matrix(matrix)
    }

    /// Creates a constant-color blend filter.
    pub const fn blend(color: Color, mode: BlendMode) -> ColorFilter {
        ColorFilter::Blend { color, mode }
    }

    /// Wraps a built-in filter for shared paint or display-list ownership.
    pub fn handle(filter: ColorFilter) -> ColorFilterHandle {
        ColorFilterHandle::new(filter)
    }
}

/// Built-in whole-layer image-filter factories.
pub mod image_filters {
    use skia_core::{ColorFilter, ImageFilter, ImageFilterHandle, SkiaError};

    /// Creates a per-pixel color-filter image effect.
    pub const fn color(filter: ColorFilter) -> ImageFilter {
        ImageFilter::Color(filter)
    }

    /// Creates a separable transparent-edge box blur.
    pub const fn box_blur(radius: u8) -> Result<ImageFilter, SkiaError> {
        ImageFilter::box_blur(radius)
    }

    /// Wraps a built-in filter for shared layer or display-list ownership.
    pub fn handle(filter: ImageFilter) -> ImageFilterHandle {
        ImageFilterHandle::new(filter)
    }
}

/// Built-in paint-source shader factories.
pub mod shaders {
    use skia_core::{
        BlendMode, Color, Gradient, GradientStop, Point, SamplingOptions, Scalar, ShaderHandle,
        SkiaError, TileMode, Transform,
    };
    use skia_image::Image;

    /// Creates a constant-color source shader.
    pub fn solid_color(color: Color) -> ShaderHandle {
        ShaderHandle::from_color(color)
    }

    /// Creates a tiled image shader in image pixel-coordinate space.
    pub fn image(
        image: Image,
        sampling: SamplingOptions,
        x_tile_mode: TileMode,
        y_tile_mode: TileMode,
    ) -> Result<ShaderHandle, SkiaError> {
        ShaderHandle::from_image(image, sampling, x_tile_mode, y_tile_mode)
    }

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

    /// Wraps a gradient for shared paint or display-list ownership.
    pub fn gradient_handle(gradient: Gradient) -> ShaderHandle {
        ShaderHandle::from_gradient(gradient)
    }

    /// Applies an additional child-to-parent local transform to a shader.
    pub fn local_matrix(
        shader: ShaderHandle,
        transform: Transform,
    ) -> Result<ShaderHandle, SkiaError> {
        shader.with_local_matrix(transform)
    }

    /// Composites two source shaders with one blend mode.
    pub fn blend(
        source: ShaderHandle,
        destination: ShaderHandle,
        mode: BlendMode,
    ) -> Result<ShaderHandle, SkiaError> {
        ShaderHandle::blend(source, destination, mode)
    }
}

/// Factories for bounded runtime color-expression shaders.
pub mod runtime_shaders {
    use skia_core::{
        Color, RuntimeShader, RuntimeShaderInstruction, RuntimeShaderLimits, RuntimeShaderProgram,
        ShaderHandle, SkiaError,
    };

    /// Validates one bounded local-space runtime shader program.
    pub fn program(
        instructions: &[RuntimeShaderInstruction],
        uniform_count: u8,
        limits: RuntimeShaderLimits,
    ) -> Result<RuntimeShaderProgram, SkiaError> {
        RuntimeShaderProgram::new(instructions, uniform_count, limits)
    }

    /// Binds immutable color uniforms to a validated runtime shader program.
    pub fn bind(
        program: RuntimeShaderProgram,
        uniforms: &[Color],
    ) -> Result<RuntimeShader, SkiaError> {
        RuntimeShader::new(program, uniforms)
    }

    /// Binds a runtime program and wraps it for shared paint ownership.
    pub fn handle(
        program: RuntimeShaderProgram,
        uniforms: &[Color],
    ) -> Result<ShaderHandle, SkiaError> {
        bind(program, uniforms).map(ShaderHandle::from_runtime)
    }
}
