use super::{
    BlendMode, Color, ColorFilter, ColorFilterHandle, ImageFilter, ImageFilterHandle, Paint,
    SaveLayerOptions,
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
