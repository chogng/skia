use super::{
    BlendMode, Color, ColorFilter, ColorFilterHandle, Gradient, GradientStop, ImageFilter,
    ImageFilterHandle, Paint, Point, SaveLayerOptions, Scalar, ShaderHandle, TileMode,
};

#[test]
fn paint_and_layers_retain_shared_value_effect_handles() {
    let color_filter = ColorFilterHandle::new(ColorFilter::Blend {
        color: Color::RED,
        mode: BlendMode::SourceOver,
    });
    let paint = Paint::new(Color::BLUE).with_color_filter_handle(color_filter.clone());
    assert_eq!(paint.color_filter_handle(), Some(&color_filter));
    assert_eq!(paint.filter_color(Color::BLUE), Color::RED);

    let image_filter = ImageFilterHandle::new(ImageFilter::box_blur(1).expect("blur"));
    let options = SaveLayerOptions::new().with_filter_handle(image_filter.clone());
    assert_eq!(options.filter_handle(), Some(&image_filter));
    assert_eq!(
        options.filter(),
        Some(ImageFilter::box_blur(1).expect("blur"))
    );
}

#[test]
fn paint_retains_a_shared_gradient_shader_handle() {
    let start = Point::new(Scalar::ZERO, Scalar::ZERO);
    let end = Point::new(Scalar::from_i32(2).expect("end"), Scalar::ZERO);
    let stops = [
        GradientStop::new(Scalar::ZERO, Color::RED).expect("start"),
        GradientStop::new(Scalar::from_i32(1).expect("end"), Color::BLUE).expect("end"),
    ];
    let gradient = Gradient::linear(start, end, &stops, TileMode::Clamp).expect("gradient");
    let shader = ShaderHandle::from_gradient(gradient);
    let paint = Paint::new(Color::WHITE).with_shader(shader.clone());

    assert_eq!(paint.shader_handle(), Some(&shader));
    assert_eq!(paint.gradient(), Some(gradient));
    assert_eq!(paint.source_color(start).expect("sample"), Color::RED);
}
