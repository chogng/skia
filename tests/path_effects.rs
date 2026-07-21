use skia::{
    ComposePathEffect, CornerPathEffect, DashPathEffect, DiscretePathEffect, PathBuilder,
    PathEffect, PathEffectLimits, PathVerb, Point, Rect, Scalar, SkiaErrorCode, SumPathEffect,
    Transform, TrimPathEffect, apply_path_effect, compose_path_effects, corner_path, dash_path,
    discrete_path, trim_path,
};

fn scalar(value: i32) -> Scalar {
    Scalar::from_i32(value).expect("scalar")
}

fn fraction(numerator: i64, denominator: i64) -> Scalar {
    Scalar::from_ratio(numerator, denominator).expect("fraction")
}

fn point(x: i32, y: i32) -> Point {
    Point::new(scalar(x), scalar(y))
}

fn bit_point(x: i32, y: i32) -> Point {
    Point::new(Scalar::from_bits(x), Scalar::from_bits(y))
}

fn line() -> skia::Path {
    let mut builder = PathBuilder::new(2).expect("builder");
    builder.move_to(point(0, 0)).expect("move");
    builder.line_to(point(8, 0)).expect("line");
    builder.finish().expect("path")
}

fn right_angle() -> skia::Path {
    let mut builder = PathBuilder::new(3).expect("builder");
    builder.move_to(point(0, 0)).expect("move");
    builder.line_to(point(4, 0)).expect("first line");
    builder.line_to(point(4, 4)).expect("second line");
    builder.finish().expect("path")
}

#[test]
fn trim_path_uses_normalized_arc_length_after_transform() {
    let effect = TrimPathEffect::new(fraction(1, 4), fraction(3, 4)).expect("effect");
    let result = trim_path(
        &line(),
        effect,
        Transform::translate(scalar(1), scalar(2)),
        PathEffectLimits::default(),
    )
    .expect("trim")
    .expect("non-empty");
    assert_eq!(
        result.verbs(),
        &[PathVerb::MoveTo(point(3, 2)), PathVerb::LineTo(point(7, 2))]
    );
}

#[test]
fn trim_path_wraps_open_and_closed_contours_without_closing_partials() {
    let wrap = TrimPathEffect::new(fraction(3, 4), fraction(1, 4)).expect("wrap");
    let open = trim_path(
        &line(),
        wrap,
        Transform::IDENTITY,
        PathEffectLimits::default(),
    )
    .expect("open trim")
    .expect("open result");
    assert_eq!(
        open.verbs(),
        &[
            PathVerb::MoveTo(point(6, 0)),
            PathVerb::LineTo(point(8, 0)),
            PathVerb::MoveTo(point(0, 0)),
            PathVerb::LineTo(point(2, 0)),
        ]
    );

    let mut rectangle = PathBuilder::new(5).expect("rectangle builder");
    rectangle
        .add_rect(Rect::new(scalar(0), scalar(0), scalar(10), scalar(10)).expect("rect"))
        .expect("add rect");
    let rectangle = rectangle.finish().expect("rectangle");
    let closed = trim_path(
        &rectangle,
        TrimPathEffect::new(fraction(3, 4), fraction(1, 4)).expect("closed wrap"),
        Transform::IDENTITY,
        PathEffectLimits::default(),
    )
    .expect("closed trim")
    .expect("closed result");
    assert_eq!(
        closed.verbs(),
        &[
            PathVerb::MoveTo(point(0, 10)),
            PathVerb::LineTo(point(0, 0)),
            PathVerb::LineTo(point(10, 0)),
        ]
    );
}

#[test]
fn trim_path_preserves_full_closure_and_bounds_output() {
    let mut rectangle = PathBuilder::new(5).expect("rectangle builder");
    rectangle
        .add_rect(Rect::new(scalar(0), scalar(0), scalar(4), scalar(4)).expect("rect"))
        .expect("add rect");
    let rectangle = rectangle.finish().expect("rectangle");
    let full = trim_path(
        &rectangle,
        TrimPathEffect::new(Scalar::ZERO, fraction(1, 1)).expect("full"),
        Transform::IDENTITY,
        PathEffectLimits::default(),
    )
    .expect("full trim")
    .expect("full result");
    assert!(matches!(full.verbs().last(), Some(PathVerb::Close)));

    assert!(
        trim_path(
            &rectangle,
            TrimPathEffect::new(fraction(1, 2), fraction(1, 2)).expect("empty"),
            Transform::IDENTITY,
            PathEffectLimits::default(),
        )
        .expect("empty trim")
        .is_none()
    );

    let error = trim_path(
        &rectangle,
        TrimPathEffect::new(Scalar::ZERO, fraction(1, 1)).expect("full"),
        Transform::IDENTITY,
        PathEffectLimits::new(8, 64, 3, 16).expect("limits"),
    )
    .expect_err("verb limit");
    assert_eq!(error.code(), SkiaErrorCode::ResourceLimit);
}

#[test]
fn corner_path_rounds_open_vertices_after_transform() {
    let rounded = corner_path(
        &right_angle(),
        CornerPathEffect::new(scalar(1)).expect("effect"),
        Transform::translate(scalar(2), scalar(3)),
        PathEffectLimits::default(),
    )
    .expect("corner path")
    .expect("non-empty");
    assert_eq!(
        rounded.verbs(),
        &[
            PathVerb::MoveTo(point(2, 3)),
            PathVerb::LineTo(point(5, 3)),
            PathVerb::QuadTo(point(6, 3), point(6, 4)),
            PathVerb::LineTo(point(6, 7)),
        ]
    );
}

#[test]
fn corner_path_clamps_large_radius_to_half_of_adjacent_edges() {
    let rounded = corner_path(
        &right_angle(),
        CornerPathEffect::new(scalar(10)).expect("effect"),
        Transform::IDENTITY,
        PathEffectLimits::default(),
    )
    .expect("corner path")
    .expect("non-empty");
    assert_eq!(
        rounded.verbs(),
        &[
            PathVerb::MoveTo(point(0, 0)),
            PathVerb::LineTo(point(2, 0)),
            PathVerb::QuadTo(point(4, 0), point(4, 2)),
            PathVerb::LineTo(point(4, 4)),
        ]
    );
}

#[test]
fn corner_path_preserves_closed_contours() {
    let mut rectangle = PathBuilder::new(5).expect("rectangle builder");
    rectangle
        .add_rect(Rect::new(scalar(0), scalar(0), scalar(4), scalar(4)).expect("rect"))
        .expect("add rect");
    let rounded = corner_path(
        &rectangle.finish().expect("rectangle"),
        CornerPathEffect::new(scalar(1)).expect("effect"),
        Transform::IDENTITY,
        PathEffectLimits::default(),
    )
    .expect("corner path")
    .expect("non-empty");
    assert_eq!(rounded.verbs().len(), 10);
    assert_eq!(
        rounded.verbs().first(),
        Some(&PathVerb::MoveTo(point(1, 0)))
    );
    assert!(matches!(rounded.verbs().last(), Some(PathVerb::Close)));
}

#[test]
fn corner_path_validates_radius_and_bounds_output() {
    let error = CornerPathEffect::new(Scalar::ZERO).expect_err("zero radius");
    assert_eq!(error.code(), SkiaErrorCode::InvalidGeometry);

    let error = corner_path(
        &right_angle(),
        CornerPathEffect::new(scalar(1)).expect("effect"),
        Transform::IDENTITY,
        PathEffectLimits::new(8, 64, 3, 16).expect("limits"),
    )
    .expect_err("verb limit");
    assert_eq!(error.code(), SkiaErrorCode::ResourceLimit);
}

#[test]
fn path_effects_compose_left_to_right_without_reapplying_transform() {
    let trim = TrimPathEffect::new(Scalar::ZERO, fraction(3, 4)).expect("trim");
    let corner = CornerPathEffect::new(scalar(1)).expect("corner");
    let effects: [&dyn PathEffect; 2] = [&trim, &corner];
    let result = compose_path_effects(
        &right_angle(),
        &effects,
        Transform::translate(scalar(2), scalar(3)),
        PathEffectLimits::default(),
    )
    .expect("compose")
    .expect("non-empty");
    assert_eq!(
        result.verbs(),
        &[
            PathVerb::MoveTo(point(2, 3)),
            PathVerb::LineTo(point(5, 3)),
            PathVerb::QuadTo(point(6, 3), point(6, 4)),
            PathVerb::LineTo(point(6, 5)),
        ]
    );
}

#[test]
fn dash_path_splits_transformed_centerlines_and_normalizes_phase() {
    let effect = DashPathEffect::new(&[scalar(2), scalar(2)], scalar(5)).expect("dash");
    assert_eq!(effect.pattern(), &[scalar(2), scalar(2)]);
    assert_eq!(effect.phase(), scalar(1));
    let dashed = dash_path(
        &line(),
        &effect,
        Transform::translate(scalar(1), scalar(2)),
        PathEffectLimits::default(),
    )
    .expect("dash path")
    .expect("visible intervals");
    assert_eq!(
        dashed.verbs(),
        &[
            PathVerb::MoveTo(point(1, 2)),
            PathVerb::LineTo(point(2, 2)),
            PathVerb::MoveTo(point(4, 2)),
            PathVerb::LineTo(point(6, 2)),
            PathVerb::MoveTo(point(8, 2)),
            PathVerb::LineTo(point(9, 2)),
        ]
    );
}

#[test]
fn dash_path_validates_pattern_and_output_limit() {
    assert_eq!(
        DashPathEffect::new(&[], Scalar::ZERO)
            .expect_err("empty pattern")
            .code(),
        SkiaErrorCode::InvalidGeometry
    );
    assert_eq!(
        DashPathEffect::new(&[scalar(1)], Scalar::ZERO)
            .expect_err("odd pattern")
            .code(),
        SkiaErrorCode::InvalidGeometry
    );
    let error = dash_path(
        &line(),
        &DashPathEffect::new(&[scalar(1), scalar(1)], Scalar::ZERO).expect("dash"),
        Transform::IDENTITY,
        PathEffectLimits::new(8, 64, 4, 16).expect("limits"),
    )
    .expect_err("verb limit");
    assert_eq!(error.code(), SkiaErrorCode::ResourceLimit);
}

#[test]
fn object_composition_and_sum_obey_transform_and_limits() {
    let inner = TrimPathEffect::new(Scalar::ZERO, fraction(3, 4)).expect("inner");
    let outer = CornerPathEffect::new(scalar(1)).expect("outer");
    let composed = ComposePathEffect::new(&outer, &inner);
    let result = apply_path_effect(
        &right_angle(),
        &composed,
        Transform::translate(scalar(2), scalar(3)),
        PathEffectLimits::default(),
    )
    .expect("compose")
    .expect("path");
    assert_eq!(
        result.verbs(),
        &[
            PathVerb::MoveTo(point(2, 3)),
            PathVerb::LineTo(point(5, 3)),
            PathVerb::QuadTo(point(6, 3), point(6, 4)),
            PathVerb::LineTo(point(6, 5)),
        ]
    );

    let first = TrimPathEffect::new(Scalar::ZERO, fraction(1, 2)).expect("first");
    let second = TrimPathEffect::new(fraction(1, 2), fraction(1, 1)).expect("second");
    let sum = SumPathEffect::new(&first, &second);
    let result = apply_path_effect(
        &line(),
        &sum,
        Transform::translate(scalar(1), Scalar::ZERO),
        PathEffectLimits::default(),
    )
    .expect("sum")
    .expect("path");
    assert_eq!(
        result.verbs(),
        &[
            PathVerb::MoveTo(point(1, 0)),
            PathVerb::LineTo(point(5, 0)),
            PathVerb::MoveTo(point(5, 0)),
            PathVerb::LineTo(point(9, 0)),
        ]
    );
    let error = apply_path_effect(
        &line(),
        &sum,
        Transform::IDENTITY,
        PathEffectLimits::new(8, 64, 3, 16).expect("limits"),
    )
    .expect_err("sum limit");
    assert_eq!(error.code(), SkiaErrorCode::ResourceLimit);
}

#[test]
fn empty_path_effect_pipeline_is_a_bounded_transform() {
    let transformed = compose_path_effects(
        &line(),
        &[],
        Transform::translate(scalar(1), scalar(2)),
        PathEffectLimits::default(),
    )
    .expect("identity pipeline")
    .expect("path");
    assert_eq!(
        transformed.verbs(),
        &[PathVerb::MoveTo(point(1, 2)), PathVerb::LineTo(point(9, 2))]
    );

    let error = compose_path_effects(
        &line(),
        &[],
        Transform::IDENTITY,
        PathEffectLimits::new(8, 64, 1, 16).expect("limits"),
    )
    .expect_err("verb limit");
    assert_eq!(error.code(), SkiaErrorCode::ResourceLimit);
}

#[test]
fn path_effect_boundary_enforces_limits_ignored_by_an_implementation() {
    struct UnboundedEffect;

    impl PathEffect for UnboundedEffect {
        fn apply(
            &self,
            path: &skia::Path,
            transform: Transform,
            _limits: PathEffectLimits,
        ) -> Result<Option<skia::Path>, skia::SkiaError> {
            path.transformed(transform).map(Some)
        }
    }

    let effect = UnboundedEffect;
    let effects: [&dyn PathEffect; 1] = [&effect];
    let error = compose_path_effects(
        &line(),
        &effects,
        Transform::IDENTITY,
        PathEffectLimits::new(8, 64, 1, 16).expect("limits"),
    )
    .expect_err("boundary verb limit");
    assert_eq!(error.code(), SkiaErrorCode::ResourceLimit);
}

#[test]
fn discrete_path_has_a_stable_seeded_fixed_point_result() {
    let effect = DiscretePathEffect::new(scalar(2), scalar(1), 7).expect("effect");
    let first = discrete_path(
        &line(),
        effect,
        Transform::IDENTITY,
        PathEffectLimits::default(),
    )
    .expect("discrete path")
    .expect("non-empty");
    let second = discrete_path(
        &line(),
        effect,
        Transform::IDENTITY,
        PathEffectLimits::default(),
    )
    .expect("repeat")
    .expect("non-empty");
    assert_eq!(first, second);
    assert_eq!(
        first.verbs(),
        &[
            PathVerb::MoveTo(point(0, 0)),
            PathVerb::LineTo(bit_point(2 << 16, -12_895)),
            PathVerb::LineTo(bit_point(4 << 16, -64_184)),
            PathVerb::LineTo(bit_point(6 << 16, -58_194)),
            PathVerb::LineTo(point(8, 0)),
        ]
    );

    let different = discrete_path(
        &line(),
        DiscretePathEffect::new(scalar(2), scalar(1), 8).expect("different seed"),
        Transform::IDENTITY,
        PathEffectLimits::default(),
    )
    .expect("different")
    .expect("non-empty");
    assert_ne!(first, different);
}

#[test]
fn discrete_path_keeps_open_endpoints_and_closed_seams() {
    let straight = discrete_path(
        &line(),
        DiscretePathEffect::new(scalar(2), Scalar::ZERO, 99).expect("effect"),
        Transform::translate(scalar(1), scalar(2)),
        PathEffectLimits::default(),
    )
    .expect("straight")
    .expect("non-empty");
    assert_eq!(
        straight.verbs(),
        &[
            PathVerb::MoveTo(point(1, 2)),
            PathVerb::LineTo(point(3, 2)),
            PathVerb::LineTo(point(5, 2)),
            PathVerb::LineTo(point(7, 2)),
            PathVerb::LineTo(point(9, 2)),
        ]
    );

    let mut rectangle = PathBuilder::new(5).expect("rectangle builder");
    rectangle
        .add_rect(Rect::new(scalar(0), scalar(0), scalar(4), scalar(4)).expect("rect"))
        .expect("add rect");
    let closed = discrete_path(
        &rectangle.finish().expect("rectangle"),
        DiscretePathEffect::new(scalar(6), scalar(1), 9).expect("effect"),
        Transform::IDENTITY,
        PathEffectLimits::default(),
    )
    .expect("closed")
    .expect("non-empty");
    assert_eq!(closed.verbs().len(), 4);
    assert!(matches!(closed.verbs().last(), Some(PathVerb::Close)));
}

#[test]
fn discrete_path_validates_geometry_and_preflights_output() {
    let error = DiscretePathEffect::new(Scalar::ZERO, scalar(1), 0).expect_err("zero segment");
    assert_eq!(error.code(), SkiaErrorCode::InvalidGeometry);
    let error = DiscretePathEffect::new(scalar(1), Scalar::from_bits(-1), 0)
        .expect_err("negative deviation");
    assert_eq!(error.code(), SkiaErrorCode::InvalidGeometry);

    let error = discrete_path(
        &line(),
        DiscretePathEffect::new(scalar(1), scalar(1), 0).expect("effect"),
        Transform::IDENTITY,
        PathEffectLimits::new(8, 64, 8, 16).expect("limits"),
    )
    .expect_err("verb limit");
    assert_eq!(error.code(), SkiaErrorCode::ResourceLimit);
}
