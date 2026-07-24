use super::{
    BlendMode, Color, Gradient, GradientStop, Paint, Point, Scalar, Shader, ShaderHandle, TileMode,
    Transform,
};

fn scalar(value: i32) -> Scalar {
    Scalar::from_i32(value).expect("scalar")
}

fn point(x: i32, y: i32) -> Point {
    Point::new(scalar(x), scalar(y))
}

#[test]
fn shader_graph_evaluates_solid_local_matrix_and_blend_nodes() {
    let solid = Paint::new(Color::WHITE)
        .with_alpha(128)
        .with_shader(Color::RED.into());
    assert_eq!(
        solid.source_color(point(0, 0)).expect("solid sample"),
        Color::rgba(255, 0, 0, 128)
    );

    let gradient = Gradient::linear(
        point(0, 0),
        point(2, 0),
        &[
            GradientStop::new(Scalar::ZERO, Color::RED).expect("red stop"),
            GradientStop::new(scalar(1), Color::BLUE).expect("blue stop"),
        ],
        TileMode::Clamp,
    )
    .expect("gradient");
    let translated = ShaderHandle::from_gradient(gradient)
        .with_local_matrix(Transform::translate(scalar(1), Scalar::ZERO))
        .expect("local matrix");
    assert_eq!(
        translated
            .as_shader()
            .sample(point(2, 0))
            .expect("translated sample"),
        Color::rgb(128, 0, 128)
    );

    let blended = ShaderHandle::blend(
        ShaderHandle::from_color(Color::RED),
        ShaderHandle::from_color(Color::BLUE),
        BlendMode::SourceOver,
    )
    .expect("blend");
    assert_eq!(
        blended
            .as_shader()
            .sample(point(0, 0))
            .expect("blend sample"),
        Color::RED
    );
}

#[test]
fn shader_graph_rejects_singular_matrices_and_excessive_nesting() {
    assert!(
        ShaderHandle::from_color(Color::RED)
            .with_local_matrix(Transform::scale(Scalar::ZERO, scalar(1)))
            .is_err()
    );

    let mut shader = ShaderHandle::from_color(Color::RED);
    for _ in 1..Shader::MAX_GRAPH_DEPTH {
        shader = shader
            .with_local_matrix(Transform::IDENTITY)
            .expect("bounded nesting");
    }
    assert!(shader.with_local_matrix(Transform::IDENTITY).is_err());
}
