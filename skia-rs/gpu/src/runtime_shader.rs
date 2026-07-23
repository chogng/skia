use skia_core::{Paint, RuntimeShaderInstruction, RuntimeShaderLimits};

/// Words retained for each portable runtime-shader instruction.
pub const RUNTIME_SHADER_INSTRUCTION_WORDS: usize = 6;
/// Maximum portable runtime-shader instruction count.
pub const RUNTIME_SHADER_MAX_INSTRUCTIONS: usize = RuntimeShaderLimits::MAX_INSTRUCTIONS as usize;
/// Maximum portable runtime-shader color-uniform count.
pub const RUNTIME_SHADER_MAX_UNIFORMS: usize = RuntimeShaderLimits::MAX_UNIFORMS as usize;

/// Backend-neutral fixed-size encoding of one validated runtime shader.
///
/// Both hardware backends upload this packet to their precompiled shader VM, so
/// program recording never needs a runtime source compiler or pipeline build.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RuntimeShaderPacket {
    instruction_count: u32,
    instructions: [[u32; RUNTIME_SHADER_INSTRUCTION_WORDS]; RUNTIME_SHADER_MAX_INSTRUCTIONS],
    uniforms: [u32; RUNTIME_SHADER_MAX_UNIFORMS],
}

impl RuntimeShaderPacket {
    /// Returns the number of active instructions.
    pub const fn instruction_count(&self) -> u32 {
        self.instruction_count
    }

    /// Borrows fixed-width encoded instructions.
    pub fn instructions(
        &self,
    ) -> &[[u32; RUNTIME_SHADER_INSTRUCTION_WORDS]; RUNTIME_SHADER_MAX_INSTRUCTIONS] {
        &self.instructions
    }

    /// Borrows packed straight-RGBA color uniforms.
    pub fn uniforms(&self) -> &[u32; RUNTIME_SHADER_MAX_UNIFORMS] {
        &self.uniforms
    }
}

/// Encodes the runtime shader carried by a paint for hardware submission.
pub fn runtime_shader_packet(paint: &Paint) -> Option<RuntimeShaderPacket> {
    let runtime = paint.shader_handle()?.as_shader().runtime()?;
    let mut packet = RuntimeShaderPacket {
        instruction_count: u32::try_from(runtime.program().instructions().len()).ok()?,
        instructions: [[0; RUNTIME_SHADER_INSTRUCTION_WORDS]; RUNTIME_SHADER_MAX_INSTRUCTIONS],
        uniforms: [0; RUNTIME_SHADER_MAX_UNIFORMS],
    };
    for (output, instruction) in packet
        .instructions
        .iter_mut()
        .zip(runtime.program().instructions())
    {
        *output = encode_instruction(*instruction);
    }
    for (output, color) in packet.uniforms.iter_mut().zip(runtime.uniforms()) {
        *output = u32::from_le_bytes(color.channels());
    }
    Some(packet)
}

fn encode_instruction(
    instruction: RuntimeShaderInstruction,
) -> [u32; RUNTIME_SHADER_INSTRUCTION_WORDS] {
    match instruction {
        RuntimeShaderInstruction::ConstantColor { destination, color } => [
            1,
            u32::from(destination),
            u32::from_le_bytes(color.channels()),
            0,
            0,
            0,
        ],
        RuntimeShaderInstruction::UniformColor {
            destination,
            uniform,
        } => [2, u32::from(destination), u32::from(uniform), 0, 0, 0],
        RuntimeShaderInstruction::LocalX {
            destination,
            start,
            end,
        } => [
            3,
            u32::from(destination),
            start.bits() as u32,
            end.bits() as u32,
            0,
            0,
        ],
        RuntimeShaderInstruction::LocalY {
            destination,
            start,
            end,
        } => [
            4,
            u32::from(destination),
            start.bits() as u32,
            end.bits() as u32,
            0,
            0,
        ],
        RuntimeShaderInstruction::Add {
            destination,
            first,
            second,
        } => [
            5,
            u32::from(destination),
            u32::from(first),
            u32::from(second),
            0,
            0,
        ],
        RuntimeShaderInstruction::Multiply {
            destination,
            first,
            second,
        } => [
            6,
            u32::from(destination),
            u32::from(first),
            u32::from(second),
            0,
            0,
        ],
        RuntimeShaderInstruction::Mix {
            destination,
            first,
            second,
            factor,
        } => [
            7,
            u32::from(destination),
            u32::from(first),
            u32::from(second),
            u32::from(factor),
            0,
        ],
        RuntimeShaderInstruction::Clamp {
            destination,
            source,
        } => [8, u32::from(destination), u32::from(source), 0, 0, 0],
        RuntimeShaderInstruction::Return { source } => [9, 0, u32::from(source), 0, 0, 0],
    }
}
