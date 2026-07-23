use skia_core::{
    Color, Paint, RuntimeShader, RuntimeShaderInstruction, RuntimeShaderLimits,
    RuntimeShaderProgram, ShaderHandle,
};

use super::RuntimeShaderPacketCache;

#[test]
fn runtime_shader_packet_cache_reuses_program_encoding_and_rebinds_uniforms() {
    let program = RuntimeShaderProgram::new(
        &[
            RuntimeShaderInstruction::UniformColor {
                destination: 0,
                uniform: 0,
            },
            RuntimeShaderInstruction::Return { source: 0 },
        ],
        1,
        RuntimeShaderLimits::default(),
    )
    .expect("program");
    let red = Paint::new(Color::WHITE).with_shader(ShaderHandle::from_runtime(
        RuntimeShader::new(program.clone(), &[Color::RED]).expect("red runtime"),
    ));
    let blue = Paint::new(Color::WHITE).with_shader(ShaderHandle::from_runtime(
        RuntimeShader::new(program, &[Color::BLUE]).expect("blue runtime"),
    ));
    let mut cache = RuntimeShaderPacketCache::with_capacity(1);

    let first = cache.packet(&red).expect("first packet");
    let second = cache.packet(&blue).expect("second packet");

    assert_eq!(first.instructions(), second.instructions());
    assert_eq!(
        first.uniforms()[0],
        u32::from_le_bytes(Color::RED.channels())
    );
    assert_eq!(
        second.uniforms()[0],
        u32::from_le_bytes(Color::BLUE.channels())
    );
    assert_eq!(cache.stats().hits(), 1);
    assert_eq!(cache.stats().misses(), 1);
    assert_eq!(cache.stats().entries(), 1);
}
