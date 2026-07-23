use super::{
    Color, Paint, Point, RuntimeShader, RuntimeShaderInstruction, RuntimeShaderLimits,
    RuntimeShaderProgram, Scalar, ShaderHandle,
};

fn scalar(value: i32) -> Scalar {
    Scalar::from_i32(value).expect("scalar")
}

#[test]
fn runtime_shader_evaluates_a_local_coordinate_color_mix() {
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
                end: scalar(2),
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

    assert_eq!(
        paint
            .source_color(Point::new(scalar(1), Scalar::ZERO))
            .expect("sample"),
        Color::rgb(128, 0, 128)
    );
}

#[test]
fn runtime_shader_rejects_invalid_register_types_and_resource_limits() {
    let invalid = RuntimeShaderProgram::new(
        &[
            RuntimeShaderInstruction::LocalX {
                destination: 0,
                start: scalar(0),
                end: scalar(1),
            },
            RuntimeShaderInstruction::Return { source: 0 },
        ],
        0,
        RuntimeShaderLimits::default(),
    );
    assert!(invalid.is_err());

    let limited = RuntimeShaderProgram::new(
        &[RuntimeShaderInstruction::Return { source: 0 }],
        0,
        RuntimeShaderLimits::new(1, 1, 1).expect("limits"),
    );
    assert!(limited.is_err());
}
