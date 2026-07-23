use skia_core::{
    BlendMode, Color, ColorFilter, ColorMatrix, DisplayListBuilder, FontCollection,
    FontCollectionLimits, Gradient, GradientStop, ImageFilter, Paint, PathEffectHandle,
    RuntimeShader, RuntimeShaderInstruction, RuntimeShaderLimits, RuntimeShaderProgram,
    SamplingOptions, SaveLayerOptions, ShaderHandle, StrokeCap, StrokeOptions, TileMode,
};
use skia_cpu::{ClipRect, Surface, SurfaceLimits};
use skia_effects::DashPathEffect;
use skia_error::SkiaErrorCode;
use skia_geometry::{Point, Rect, Scalar, Transform};
use skia_image::{Image, ImageErrorCode};
use skia_path::{ConicWeight, FillRule, PathBuilder};
use skia_tessellation::stroke_to_path;

fn scalar(value: i32) -> Scalar {
    Scalar::from_i32(value).unwrap()
}

fn point(x: i32, y: i32) -> Point {
    Point::new(scalar(x), scalar(y))
}

fn rect(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
    Rect::new(scalar(left), scalar(top), scalar(right), scalar(bottom)).unwrap()
}

fn pixel(surface: &Surface, x: usize, y: usize) -> [u8; 4] {
    let offset = (y * surface.width() as usize + x) * 4;
    surface.pixels()[offset..offset + 4].try_into().unwrap()
}

#[test]
fn clipped_source_over_rect_is_exact_and_save_restore_is_isolated() {
    let mut surface = Surface::new(4, 3, SurfaceLimits::default()).unwrap();
    let mut canvas = surface.canvas();
    canvas.clear(Color::WHITE);
    canvas.save().unwrap();
    canvas.clip_rect(ClipRect::new(rect(1, 1, 3, 3))).unwrap();
    canvas
        .fill_rect(rect(0, 0, 4, 3), Paint::new(Color::rgba(255, 0, 0, 128)))
        .unwrap();
    canvas.restore().unwrap();
    canvas
        .fill_rect(rect(0, 0, 1, 1), Paint::new(Color::BLACK))
        .unwrap();
    drop(canvas);

    assert_eq!(pixel(&surface, 0, 0), [0, 0, 0, 255]);
    assert_eq!(pixel(&surface, 1, 0), [255, 255, 255, 255]);
    assert_eq!(pixel(&surface, 1, 1), [255, 127, 127, 255]);
    assert_eq!(pixel(&surface, 2, 2), [255, 127, 127, 255]);
    assert_eq!(pixel(&surface, 3, 2), [255, 255, 255, 255]);
}

#[test]
fn transformed_gradient_and_color_filter_use_local_paint_coordinates() {
    let stops = [
        GradientStop::new(Scalar::ZERO, Color::RED).expect("red"),
        GradientStop::new(scalar(1), Color::BLUE).expect("blue"),
    ];
    let gradient =
        Gradient::linear(point(0, 0), point(4, 0), &stops, TileMode::Clamp).expect("gradient");
    let swap_red_blue = ColorMatrix::new([
        0,
        0,
        1 << 16,
        0,
        0,
        0,
        1 << 16,
        0,
        0,
        0,
        1 << 16,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        1 << 16,
        0,
    ]);
    let paint =
        Paint::from_gradient(gradient).with_color_filter(ColorFilter::Matrix(swap_red_blue));
    let mut surface = Surface::new(6, 1, SurfaceLimits::default()).expect("surface");
    let mut canvas = surface.canvas();
    canvas.set_transform(Transform::translate(scalar(1), Scalar::ZERO));
    canvas.fill_rect(rect(0, 0, 4, 1), paint).expect("fill");
    drop(canvas);

    assert_eq!(pixel(&surface, 0, 0), [0, 0, 0, 0]);
    assert_eq!(pixel(&surface, 1, 0), [32, 0, 223, 255]);
    assert_eq!(pixel(&surface, 2, 0), [96, 0, 159, 255]);
    assert_eq!(pixel(&surface, 3, 0), [159, 0, 96, 255]);
    assert_eq!(pixel(&surface, 4, 0), [223, 0, 32, 255]);
    assert_eq!(pixel(&surface, 5, 0), [0, 0, 0, 0]);
}

#[test]
fn save_layer_isolates_opacity_bounds_and_blur() {
    let mut surface = Surface::new(3, 1, SurfaceLimits::default()).expect("surface");
    let mut canvas = surface.canvas();
    canvas.clear(Color::BLUE);
    canvas
        .save_layer(
            SaveLayerOptions::new()
                .with_bounds(rect(1, 0, 3, 1))
                .with_opacity(128),
        )
        .expect("save layer");
    canvas
        .fill_rect(rect(0, 0, 3, 1), Paint::new(Color::RED))
        .expect("layer fill");
    canvas.restore().expect("restore layer");
    drop(canvas);
    assert_eq!(pixel(&surface, 0, 0), Color::BLUE.channels());
    assert_eq!(pixel(&surface, 1, 0), [128, 0, 127, 255]);
    assert_eq!(pixel(&surface, 2, 0), [128, 0, 127, 255]);

    let mut blurred = Surface::new(3, 3, SurfaceLimits::default()).expect("blur surface");
    let mut canvas = blurred.canvas();
    canvas
        .save_layer(
            SaveLayerOptions::new().with_filter(ImageFilter::box_blur(1).expect("blur filter")),
        )
        .expect("blur layer");
    canvas
        .fill_rect(rect(1, 1, 2, 2), Paint::new(Color::WHITE))
        .expect("center pixel");
    canvas.restore().expect("blur restore");
    drop(canvas);
    for y in 0..3 {
        for x in 0..3 {
            assert_eq!(pixel(&blurred, x, y), [255, 255, 255, 28]);
        }
    }
}

#[test]
fn save_layer_obeys_depth_and_memory_budgets() {
    let mut surface =
        Surface::new(2, 2, SurfaceLimits::new(4, 31, 2).expect("limits")).expect("surface");
    assert_eq!(
        surface
            .canvas()
            .save_layer(SaveLayerOptions::new())
            .expect_err("layer memory")
            .code(),
        SkiaErrorCode::ResourceLimit
    );
    assert!(ImageFilter::box_blur(0).is_err());
    assert!(ImageFilter::box_blur(65).is_err());
}

#[test]
fn even_odd_path_hole_and_translation_are_deterministic() {
    let mut path = PathBuilder::new(10).unwrap();
    path.move_to(point(0, 0)).unwrap();
    path.line_to(point(4, 0)).unwrap();
    path.line_to(point(4, 4)).unwrap();
    path.line_to(point(0, 4)).unwrap();
    path.close().unwrap();
    path.move_to(point(1, 1)).unwrap();
    path.line_to(point(3, 1)).unwrap();
    path.line_to(point(3, 3)).unwrap();
    path.line_to(point(1, 3)).unwrap();
    path.close().unwrap();
    let path = path.finish().unwrap();

    let mut surface = Surface::new(6, 5, SurfaceLimits::default()).unwrap();
    let mut canvas = surface.canvas();
    canvas.set_transform(Transform::translate(scalar(1), scalar(0)));
    canvas
        .fill_path(
            &path,
            FillRule::EvenOdd,
            Paint::new(Color::rgba(0, 0, 255, 255)),
        )
        .unwrap();
    drop(canvas);

    assert_eq!(pixel(&surface, 1, 0), [0, 0, 255, 255]);
    assert_eq!(pixel(&surface, 2, 1), [0, 0, 0, 0]);
    assert_eq!(pixel(&surface, 4, 3), [0, 0, 255, 255]);
    assert_eq!(pixel(&surface, 5, 4), [0, 0, 0, 0]);
}

#[test]
fn fixed_point_construction_and_surface_budgets_fail_closed() {
    assert!(Scalar::from_ratio(1, 0).is_err());
    assert!(Surface::new(3, 3, SurfaceLimits::new(8, 32, 1).unwrap()).is_err());
}

#[test]
fn bezier_fill_uses_a_deterministic_fixed_flattening() {
    let mut path = PathBuilder::new(4).unwrap();
    path.move_to(point(0, 4)).unwrap();
    path.quad_to(point(2, 0), point(4, 4)).unwrap();
    path.close().unwrap();
    let path = path.finish().unwrap();

    let mut surface = Surface::new(5, 5, SurfaceLimits::default()).unwrap();
    let mut canvas = surface.canvas();
    canvas
        .fill_path(
            &path,
            FillRule::NonZero,
            Paint::new(Color::rgba(0, 255, 0, 255)),
        )
        .unwrap();
    drop(canvas);

    assert_eq!(pixel(&surface, 2, 3), [0, 255, 0, 255]);
    assert_eq!(pixel(&surface, 2, 0), [0, 0, 0, 0]);
}

#[test]
fn cubic_curves_and_sheared_rectangles_use_the_general_path_rasterizer() {
    let mut curve = PathBuilder::new(4).unwrap();
    curve.move_to(point(0, 4)).unwrap();
    curve
        .cubic_to(point(0, 0), point(4, 0), point(4, 4))
        .unwrap();
    curve.close().unwrap();
    let curve = curve.finish().unwrap();

    let mut surface = Surface::new(6, 5, SurfaceLimits::default()).unwrap();
    let mut canvas = surface.canvas();
    canvas
        .fill_path(
            &curve,
            FillRule::NonZero,
            Paint::new(Color::rgba(0, 0, 255, 255)),
        )
        .unwrap();
    canvas.set_transform(Transform::new(
        scalar(1),
        scalar(0),
        scalar(1),
        scalar(1),
        scalar(0),
        scalar(0),
    ));
    canvas
        .fill_rect(rect(0, 0, 2, 2), Paint::new(Color::rgba(255, 0, 0, 255)))
        .unwrap();
    drop(canvas);

    assert_eq!(pixel(&surface, 2, 3), [0, 0, 255, 255]);
    assert_eq!(pixel(&surface, 0, 0), [255, 0, 0, 255]);
}

#[test]
fn oval_and_round_rect_conveniences_reach_the_cpu_rasterizer() {
    let mut oval_builder = PathBuilder::new(6).unwrap();
    oval_builder.add_oval(rect(1, 1, 7, 5)).unwrap();
    let oval = oval_builder.finish().unwrap();

    let mut round_rect_builder = PathBuilder::new(10).unwrap();
    round_rect_builder
        .add_round_rect(rect(0, 0, 6, 6), scalar(2), scalar(2))
        .unwrap();
    let round_rect = round_rect_builder.finish().unwrap();

    let mut surface = Surface::new(8, 6, SurfaceLimits::default()).unwrap();
    let mut canvas = surface.canvas();
    canvas
        .fill_path(
            &oval,
            FillRule::NonZero,
            Paint::new(Color::rgba(0, 255, 0, 255)),
        )
        .unwrap();
    canvas
        .fill_path(
            &round_rect,
            FillRule::NonZero,
            Paint::new(Color::rgba(255, 0, 0, 255)),
        )
        .unwrap();
    drop(canvas);

    assert_eq!(pixel(&surface, 0, 0), [0, 0, 0, 0]);
    assert_eq!(pixel(&surface, 3, 0), [255, 0, 0, 255]);
    assert_eq!(pixel(&surface, 7, 3), [0, 0, 0, 0]);
    assert_eq!(pixel(&surface, 6, 3), [0, 255, 0, 255]);
}

#[test]
fn conics_and_reversed_contours_preserve_fill_semantics() {
    let mut quadratic_builder = PathBuilder::new(3).unwrap();
    quadratic_builder.move_to(point(0, 4)).unwrap();
    quadratic_builder.quad_to(point(4, 0), point(8, 4)).unwrap();
    quadratic_builder.close().unwrap();
    let quadratic = quadratic_builder.finish().unwrap();

    let mut conic_builder = PathBuilder::new(3).unwrap();
    conic_builder.move_to(point(0, 4)).unwrap();
    conic_builder
        .conic_to(point(4, 0), point(8, 4), ConicWeight::ONE)
        .unwrap();
    conic_builder.close().unwrap();
    let conic = conic_builder.finish().unwrap();

    let mut quadratic_surface = Surface::new(9, 5, SurfaceLimits::default()).unwrap();
    quadratic_surface
        .canvas()
        .fill_path(
            &quadratic,
            FillRule::NonZero,
            Paint::new(Color::rgba(0, 255, 0, 255)),
        )
        .unwrap();
    let mut conic_surface = Surface::new(9, 5, SurfaceLimits::default()).unwrap();
    conic_surface
        .canvas()
        .fill_path(
            &conic,
            FillRule::NonZero,
            Paint::new(Color::rgba(0, 255, 0, 255)),
        )
        .unwrap();
    assert_eq!(quadratic_surface.pixels(), conic_surface.pixels());

    let mut outer_builder = PathBuilder::new(5).unwrap();
    outer_builder.add_rect(rect(0, 0, 8, 8)).unwrap();
    let outer = outer_builder.finish().unwrap();
    let mut inner_builder = PathBuilder::new(5).unwrap();
    inner_builder.add_rect(rect(2, 2, 6, 6)).unwrap();
    let inner = inner_builder.finish().unwrap().reversed().unwrap();
    let mut compound_builder = PathBuilder::new(11).unwrap();
    compound_builder.append_path(&outer).unwrap();
    compound_builder.append_path(&inner).unwrap();
    let compound = compound_builder.finish().unwrap();

    let mut surface = Surface::new(8, 8, SurfaceLimits::default()).unwrap();
    surface
        .canvas()
        .fill_path(
            &compound,
            FillRule::NonZero,
            Paint::new(Color::rgba(255, 0, 0, 255)),
        )
        .unwrap();
    assert_eq!(pixel(&surface, 1, 1), [255, 0, 0, 255]);
    assert_eq!(pixel(&surface, 3, 3), [0, 0, 0, 0]);
}

#[test]
fn stroke_has_round_caps_and_joins_without_external_dependencies() {
    let mut path = PathBuilder::new(3).unwrap();
    path.move_to(point(1, 2)).unwrap();
    path.line_to(point(5, 2)).unwrap();
    let path = path.finish().unwrap();

    let mut surface = Surface::new(7, 4, SurfaceLimits::default()).unwrap();
    let mut canvas = surface.canvas();
    canvas
        .stroke_path(&path, scalar(2), Paint::new(Color::rgba(255, 0, 0, 255)))
        .unwrap();
    drop(canvas);

    assert_eq!(pixel(&surface, 0, 2), [255, 0, 0, 255]);
    assert_eq!(pixel(&surface, 5, 1), [255, 0, 0, 255]);
    assert_eq!(pixel(&surface, 3, 0), [0, 0, 0, 0]);
}

#[test]
fn display_list_replays_a_path_effect_held_by_paint() {
    let mut path = PathBuilder::new(2).expect("path builder");
    path.move_to(point(0, 2)).expect("move");
    path.line_to(point(8, 2)).expect("line");
    let effect = DashPathEffect::new(&[scalar(2), scalar(2)], Scalar::ZERO).expect("dash");
    let paint = Paint::new(Color::WHITE).with_path_effect(PathEffectHandle::new(effect));
    let options = StrokeOptions::new(scalar(2))
        .expect("stroke")
        .with_cap(StrokeCap::Butt);
    let mut builder = DisplayListBuilder::new(4).expect("display list");
    let path = builder
        .add_path(path.finish().expect("path"))
        .expect("path resource");
    builder
        .stroke_path_with_options(path, options, paint)
        .expect("stroke command");
    let list = builder.finish();
    let mut surface = Surface::new(8, 4, SurfaceLimits::default()).expect("surface");
    surface
        .execute_display_list(&list, &FontCollection::new(FontCollectionLimits::default()))
        .expect("replay");

    for x in [0, 1, 4, 5] {
        assert_eq!(pixel(&surface, x, 2), Color::WHITE.channels());
    }
    for x in [2, 3, 6, 7] {
        assert_eq!(pixel(&surface, x, 2), Color::TRANSPARENT.channels());
    }
}

#[test]
fn display_list_replays_a_runtime_shader_held_by_paint() {
    let program = RuntimeShaderProgram::new(
        &[
            RuntimeShaderInstruction::UniformColor {
                destination: 0,
                uniform: 0,
            },
            RuntimeShaderInstruction::UniformColor {
                destination: 1,
                uniform: 1,
            },
            RuntimeShaderInstruction::LocalX {
                destination: 2,
                start: scalar(0),
                end: scalar(4),
            },
            RuntimeShaderInstruction::Mix {
                destination: 3,
                first: 0,
                second: 1,
                factor: 2,
            },
            RuntimeShaderInstruction::Return { source: 3 },
        ],
        2,
        RuntimeShaderLimits::default(),
    )
    .expect("program");
    let runtime = RuntimeShader::new(program, &[Color::RED, Color::BLUE]).expect("runtime");
    let paint = Paint::new(Color::WHITE).with_shader(ShaderHandle::from_runtime(runtime));
    let mut builder = DisplayListBuilder::new(1).expect("display list");
    builder
        .fill_rect(rect(0, 0, 4, 1), paint)
        .expect("fill command");
    let mut surface = Surface::new(4, 1, SurfaceLimits::default()).expect("surface");
    surface
        .execute_display_list(
            &builder.finish(),
            &FontCollection::new(FontCollectionLimits::default()),
        )
        .expect("replay");

    assert_eq!(pixel(&surface, 0, 0), [223, 0, 32, 255]);
    assert_eq!(pixel(&surface, 3, 0), [32, 0, 223, 255]);
}

#[test]
fn concatenated_transforms_and_curve_order_fail_closed() {
    let transform = Transform::translate(scalar(1), scalar(1))
        .concat(Transform::scale(scalar(2), scalar(3)))
        .unwrap();
    assert_eq!(transform.map_point(point(1, 1)).unwrap(), point(4, 6));

    let mut path = PathBuilder::new(1).unwrap();
    assert_eq!(
        path.cubic_to(point(0, 0), point(1, 1), point(2, 2))
            .unwrap_err()
            .code(),
        SkiaErrorCode::InvalidPath
    );
}

#[test]
fn affine_transforms_round_trip_points_and_reject_singular_matrices() {
    let transform = Transform::new(
        scalar(1),
        scalar(1),
        Scalar::ZERO,
        scalar(1),
        scalar(2),
        scalar(3),
    );
    let mapped = transform.map_point(point(4, 5)).expect("map point");
    assert_eq!(mapped, point(6, 12));
    assert_eq!(
        transform.inverse().unwrap().map_point(mapped).unwrap(),
        point(4, 5)
    );

    let singular = Transform::new(
        scalar(1),
        scalar(2),
        scalar(2),
        scalar(4),
        Scalar::ZERO,
        Scalar::ZERO,
    );
    assert_eq!(
        singular.inverse().unwrap_err().code(),
        SkiaErrorCode::InvalidGeometry
    );
}

#[test]
fn rgba_images_scale_nearest_neighbor_and_keep_source_color_under_opacity() {
    let image = Image::from_rgba8(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]).unwrap();
    let mut surface = Surface::new(4, 2, SurfaceLimits::default()).unwrap();
    let mut canvas = surface.canvas();
    canvas
        .draw_image(&image, rect(0, 0, 4, 2), 128, BlendMode::SourceOver)
        .unwrap();
    drop(canvas);

    assert_eq!(pixel(&surface, 0, 0), [255, 0, 0, 128]);
    assert_eq!(pixel(&surface, 1, 1), [255, 0, 0, 128]);
    assert_eq!(pixel(&surface, 2, 0), [0, 0, 255, 128]);
    assert_eq!(pixel(&surface, 3, 1), [0, 0, 255, 128]);
    assert_eq!(
        Image::from_rgba8(2, 2, vec![0; 3]).unwrap_err().code(),
        ImageErrorCode::InvalidPixels
    );
}

#[test]
fn rgba_images_support_texel_center_linear_sampling() {
    let image = Image::from_rgba8(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]).unwrap();
    let mut surface = Surface::new(4, 1, SurfaceLimits::default()).unwrap();
    surface
        .canvas()
        .draw_image_with_sampling(
            &image,
            rect(0, 0, 4, 1),
            255,
            BlendMode::SourceOver,
            SamplingOptions::LINEAR,
        )
        .unwrap();

    assert_eq!(pixel(&surface, 0, 0), [255, 0, 0, 255]);
    assert_eq!(pixel(&surface, 1, 0), [191, 0, 64, 255]);
    assert_eq!(pixel(&surface, 2, 0), [64, 0, 191, 255]);
    assert_eq!(pixel(&surface, 3, 0), [0, 0, 255, 255]);
}

#[test]
fn rgba_images_support_rotated_inverse_mapped_sampling() {
    let image = Image::from_rgba8(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]).unwrap();
    let mut surface = Surface::new(3, 3, SurfaceLimits::default()).unwrap();
    let mut canvas = surface.canvas();
    canvas.set_transform(Transform::new(
        Scalar::ZERO,
        scalar(1),
        scalar(-1),
        Scalar::ZERO,
        scalar(2),
        Scalar::ZERO,
    ));
    canvas
        .draw_image(&image, rect(0, 0, 2, 1), 255, BlendMode::SourceOver)
        .unwrap();
    drop(canvas);

    assert_eq!(pixel(&surface, 1, 0), [255, 0, 0, 255]);
    assert_eq!(pixel(&surface, 1, 1), [0, 0, 255, 255]);
    assert_eq!(pixel(&surface, 0, 0), [0, 0, 0, 0]);
    assert_eq!(pixel(&surface, 2, 1), [0, 0, 0, 0]);
}

#[test]
fn stroke_to_path_expands_a_transformed_stroke_for_normal_fill() {
    let mut builder = PathBuilder::new(2).unwrap();
    builder.move_to(point(2, 3)).unwrap();
    builder.line_to(point(8, 3)).unwrap();
    let options = StrokeOptions::new(scalar(2))
        .unwrap()
        .with_cap(StrokeCap::Square);
    let expanded =
        stroke_to_path(&builder.finish().unwrap(), &options, Transform::IDENTITY).unwrap();
    let mut surface = Surface::new(10, 6, SurfaceLimits::default()).unwrap();
    surface
        .canvas()
        .fill_path(&expanded, FillRule::NonZero, Paint::new(Color::WHITE))
        .unwrap();

    assert_eq!(pixel(&surface, 1, 3), Color::WHITE.channels());
    assert_eq!(pixel(&surface, 8, 3), Color::WHITE.channels());
    assert_eq!(pixel(&surface, 0, 3), [0, 0, 0, 0]);
}
