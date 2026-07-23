use super::{
    Color, Paint, Path, PathBuilder, PathEffect, PathEffectHandle, PathEffectLimits, Point, Scalar,
    SkiaError, Transform,
};

#[derive(Debug)]
struct TranslationEffect;

impl PathEffect for TranslationEffect {
    fn apply(
        &self,
        path: &Path,
        transform: Transform,
        _limits: PathEffectLimits,
    ) -> Result<Option<Path>, SkiaError> {
        path.transformed(transform).map(Some)
    }
}

#[test]
fn paint_retains_a_cloneable_path_effect_handle() {
    let mut builder = PathBuilder::new(2).expect("path builder");
    builder
        .move_to(Point::new(Scalar::ZERO, Scalar::ZERO))
        .expect("move");
    builder
        .line_to(Point::new(
            Scalar::from_i32(1).expect("scalar"),
            Scalar::ZERO,
        ))
        .expect("line");
    let path = builder.finish().expect("path");
    let handle = PathEffectHandle::new(TranslationEffect);
    let paint = Paint::new(Color::WHITE).with_path_effect(handle.clone());

    assert_eq!(paint.path_effect(), Some(&handle));
    let transformed = handle
        .apply(
            &path,
            Transform::translate(Scalar::from_i32(2).expect("scalar"), Scalar::ZERO),
        )
        .expect("effect")
        .expect("geometry");
    assert_eq!(
        transformed,
        path.transformed(Transform::translate(
            Scalar::from_i32(2).expect("scalar"),
            Scalar::ZERO,
        ))
        .expect("transform")
    );
}
