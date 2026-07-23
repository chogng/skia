use skia_geometry::{Point, Scalar};

use super::{BlendMode, Color, ColorFilter, ColorMatrix, Gradient, GradientStop, Paint, TileMode};

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
    assert_eq!(
        Color::rgb(200, 100, 40).composite(Color::rgb(30, 80, 220), BlendMode::SoftLight),
        Color::rgb(61, 68, 199)
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
    let gradient = Gradient::radial(center, Scalar::from_bits(10 << 16), &stops, TileMode::Clamp)
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
