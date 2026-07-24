use crate::paint::Color;
use crate::shaders::gradient::rounded_ratio;
use skia_error::{SkiaError, SkiaErrorCode};
use skia_geometry::{Point, Scalar};
use std::sync::Arc;

const RUNTIME_SHADER_MAX_REGISTERS: usize = 16;
const RUNTIME_SHADER_ONE: i32 = 1 << 16;
const RUNTIME_SHADER_CHANNEL_MAX: i32 = 255 << 16;

/// Resource ceilings for one [`RuntimeShaderProgram`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RuntimeShaderLimits {
    max_instructions: u8,
    max_uniforms: u8,
    max_registers: u8,
}

impl RuntimeShaderLimits {
    /// Maximum instruction count supported by the portable shader packet.
    pub const MAX_INSTRUCTIONS: u8 = 64;
    /// Maximum color-uniform count supported by the portable shader packet.
    pub const MAX_UNIFORMS: u8 = 16;
    /// Maximum typed register count supported by the portable shader packet.
    pub const MAX_REGISTERS: u8 = RUNTIME_SHADER_MAX_REGISTERS as u8;

    /// Creates instruction, uniform, and register ceilings for one program.
    pub fn new(
        max_instructions: u8,
        max_uniforms: u8,
        max_registers: u8,
    ) -> Result<Self, SkiaError> {
        if max_instructions == 0
            || max_instructions > Self::MAX_INSTRUCTIONS
            || max_uniforms > Self::MAX_UNIFORMS
            || max_registers == 0
            || max_registers > Self::MAX_REGISTERS
        {
            return Err(SkiaError::new(SkiaErrorCode::InvalidLimits));
        }
        Ok(Self {
            max_instructions,
            max_uniforms,
            max_registers,
        })
    }

    /// Returns the maximum instruction count.
    pub const fn max_instructions(self) -> u8 {
        self.max_instructions
    }

    /// Returns the maximum color-uniform count.
    pub const fn max_uniforms(self) -> u8 {
        self.max_uniforms
    }

    /// Returns the maximum typed register count.
    pub const fn max_registers(self) -> u8 {
        self.max_registers
    }
}

impl Default for RuntimeShaderLimits {
    fn default() -> Self {
        Self {
            max_instructions: Self::MAX_INSTRUCTIONS,
            max_uniforms: Self::MAX_UNIFORMS,
            max_registers: Self::MAX_REGISTERS,
        }
    }
}

/// One instruction in a bounded, local-space runtime shader.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RuntimeShaderInstruction {
    /// Writes a constant straight-alpha color.
    ConstantColor {
        /// Destination color register.
        destination: u8,
        /// Constant value.
        color: Color,
    },
    /// Writes a caller-provided color uniform.
    UniformColor {
        /// Destination color register.
        destination: u8,
        /// Index in the runtime shader's color-uniform array.
        uniform: u8,
    },
    /// Writes a clamped Q16.16 local X parameter over one non-degenerate interval.
    LocalX {
        /// Destination scalar register.
        destination: u8,
        /// Coordinate mapped to zero.
        start: Scalar,
        /// Coordinate mapped to one.
        end: Scalar,
    },
    /// Writes a clamped Q16.16 local Y parameter over one non-degenerate interval.
    LocalY {
        /// Destination scalar register.
        destination: u8,
        /// Coordinate mapped to zero.
        start: Scalar,
        /// Coordinate mapped to one.
        end: Scalar,
    },
    /// Adds two color registers with saturating 8-bit-channel output.
    Add {
        /// Destination color register.
        destination: u8,
        /// First color register.
        first: u8,
        /// Second color register.
        second: u8,
    },
    /// Multiplies two color registers channel by channel.
    Multiply {
        /// Destination color register.
        destination: u8,
        /// First color register.
        first: u8,
        /// Second color register.
        second: u8,
    },
    /// Interpolates between two colors with a scalar register.
    Mix {
        /// Destination color register.
        destination: u8,
        /// Color selected at factor zero.
        first: u8,
        /// Color selected at factor one.
        second: u8,
        /// Q16.16 interpolation factor register.
        factor: u8,
    },
    /// Clamps all channels of one color register to the 8-bit range.
    Clamp {
        /// Destination color register.
        destination: u8,
        /// Source color register.
        source: u8,
    },
    /// Returns the final color register; it must be the last instruction.
    Return {
        /// Source color register.
        source: u8,
    },
}

/// Immutable, validated program for a restricted runtime shader.
///
/// Programs contain no loops, host callbacks, source text, or texture access.
/// Their compact typed register IR is portable to a future CPU interpreter and
/// Metal/Vulkan's precompiled shader VM.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct RuntimeShaderProgram {
    instructions: Arc<[RuntimeShaderInstruction]>,
    uniform_count: u8,
    register_count: u8,
}

impl RuntimeShaderProgram {
    /// Validates and owns one bounded runtime shader program.
    pub fn new(
        instructions: &[RuntimeShaderInstruction],
        uniform_count: u8,
        limits: RuntimeShaderLimits,
    ) -> Result<Self, SkiaError> {
        let register_count = validate_runtime_shader_program(instructions, uniform_count, limits)?;
        Ok(Self {
            instructions: Arc::from(instructions),
            uniform_count,
            register_count,
        })
    }

    /// Borrows the validated instruction stream in execution order.
    pub fn instructions(&self) -> &[RuntimeShaderInstruction] {
        &self.instructions
    }

    /// Returns the required number of color uniforms.
    pub const fn uniform_count(&self) -> u8 {
        self.uniform_count
    }

    /// Returns the number of typed registers referenced by this program.
    pub const fn register_count(&self) -> u8 {
        self.register_count
    }

    fn sample(&self, uniforms: &[Color], point: Point) -> Result<Color, SkiaError> {
        if uniforms.len() != usize::from(self.uniform_count) {
            return Err(SkiaError::new(SkiaErrorCode::InvalidResource));
        }
        let mut registers = [None; RUNTIME_SHADER_MAX_REGISTERS];
        for instruction in self.instructions() {
            match *instruction {
                RuntimeShaderInstruction::ConstantColor { destination, color } => {
                    write_runtime_register(
                        &mut registers,
                        destination,
                        RuntimeShaderValue::Color(runtime_color(color)),
                    )?;
                }
                RuntimeShaderInstruction::UniformColor {
                    destination,
                    uniform,
                } => {
                    let color = uniforms
                        .get(usize::from(uniform))
                        .copied()
                        .ok_or(SkiaError::new(SkiaErrorCode::InvalidResource))?;
                    write_runtime_register(
                        &mut registers,
                        destination,
                        RuntimeShaderValue::Color(runtime_color(color)),
                    )?;
                }
                RuntimeShaderInstruction::LocalX {
                    destination,
                    start,
                    end,
                } => {
                    write_runtime_register(
                        &mut registers,
                        destination,
                        RuntimeShaderValue::Scalar(runtime_coordinate(point.x(), start, end)?),
                    )?;
                }
                RuntimeShaderInstruction::LocalY {
                    destination,
                    start,
                    end,
                } => {
                    write_runtime_register(
                        &mut registers,
                        destination,
                        RuntimeShaderValue::Scalar(runtime_coordinate(point.y(), start, end)?),
                    )?;
                }
                RuntimeShaderInstruction::Add {
                    destination,
                    first,
                    second,
                } => {
                    let color = runtime_add(
                        runtime_color_register(&registers, first)?,
                        runtime_color_register(&registers, second)?,
                    );
                    write_runtime_register(
                        &mut registers,
                        destination,
                        RuntimeShaderValue::Color(color),
                    )?;
                }
                RuntimeShaderInstruction::Multiply {
                    destination,
                    first,
                    second,
                } => {
                    let color = runtime_multiply(
                        runtime_color_register(&registers, first)?,
                        runtime_color_register(&registers, second)?,
                    );
                    write_runtime_register(
                        &mut registers,
                        destination,
                        RuntimeShaderValue::Color(color),
                    )?;
                }
                RuntimeShaderInstruction::Mix {
                    destination,
                    first,
                    second,
                    factor,
                } => {
                    let color = runtime_mix(
                        runtime_color_register(&registers, first)?,
                        runtime_color_register(&registers, second)?,
                        runtime_scalar_register(&registers, factor)?,
                    );
                    write_runtime_register(
                        &mut registers,
                        destination,
                        RuntimeShaderValue::Color(color),
                    )?;
                }
                RuntimeShaderInstruction::Clamp {
                    destination,
                    source,
                } => {
                    let color = runtime_clamp(runtime_color_register(&registers, source)?);
                    write_runtime_register(
                        &mut registers,
                        destination,
                        RuntimeShaderValue::Color(color),
                    )?;
                }
                RuntimeShaderInstruction::Return { source } => {
                    return Ok(runtime_color_from_value(runtime_color_register(
                        &registers, source,
                    )?));
                }
            }
        }
        Err(SkiaError::new(SkiaErrorCode::InvalidResource))
    }
}

/// A validated runtime shader and its immutable color uniforms.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct RuntimeShader {
    program: Arc<RuntimeShaderProgram>,
    uniforms: Arc<[Color]>,
}

impl RuntimeShader {
    /// Binds color uniforms to one owned program.
    pub fn new(program: RuntimeShaderProgram, uniforms: &[Color]) -> Result<Self, SkiaError> {
        Self::from_program(Arc::new(program), uniforms)
    }

    /// Binds color uniforms to a shared program.
    pub fn from_program(
        program: Arc<RuntimeShaderProgram>,
        uniforms: &[Color],
    ) -> Result<Self, SkiaError> {
        if uniforms.len() != usize::from(program.uniform_count()) {
            return Err(SkiaError::new(SkiaErrorCode::InvalidResource));
        }
        Ok(Self {
            program,
            uniforms: Arc::from(uniforms),
        })
    }

    /// Borrows the shared validated program.
    pub fn program(&self) -> &RuntimeShaderProgram {
        &self.program
    }

    /// Borrows the immutable bound color uniforms.
    pub fn uniforms(&self) -> &[Color] {
        &self.uniforms
    }

    /// Evaluates this shader at one local-space point.
    pub fn sample(&self, point: Point) -> Result<Color, SkiaError> {
        self.program.sample(&self.uniforms, point)
    }
}

#[derive(Clone, Copy)]
enum RuntimeShaderValue {
    Scalar(i32),
    Color([i32; 4]),
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum RuntimeShaderRegisterKind {
    Scalar,
    Color,
}

fn validate_runtime_shader_program(
    instructions: &[RuntimeShaderInstruction],
    uniform_count: u8,
    limits: RuntimeShaderLimits,
) -> Result<u8, SkiaError> {
    if instructions.is_empty() {
        return Err(SkiaError::new(SkiaErrorCode::InvalidResource));
    }
    if instructions.len() > usize::from(limits.max_instructions())
        || uniform_count > limits.max_uniforms()
    {
        return Err(SkiaError::new(SkiaErrorCode::ResourceLimit));
    }

    let mut registers = [None; RUNTIME_SHADER_MAX_REGISTERS];
    let mut register_count = 0_u8;
    for (index, instruction) in instructions.iter().enumerate() {
        let final_instruction = index + 1 == instructions.len();
        match *instruction {
            RuntimeShaderInstruction::ConstantColor { destination, .. } => {
                write_runtime_register_kind(
                    &mut registers,
                    destination,
                    RuntimeShaderRegisterKind::Color,
                    limits,
                    &mut register_count,
                )?
            }
            RuntimeShaderInstruction::UniformColor {
                destination,
                uniform,
            } => {
                if uniform >= uniform_count {
                    return Err(SkiaError::new(SkiaErrorCode::InvalidResource));
                }
                write_runtime_register_kind(
                    &mut registers,
                    destination,
                    RuntimeShaderRegisterKind::Color,
                    limits,
                    &mut register_count,
                )?;
            }
            RuntimeShaderInstruction::LocalX {
                destination,
                start,
                end,
            }
            | RuntimeShaderInstruction::LocalY {
                destination,
                start,
                end,
            } => {
                if start == end {
                    return Err(SkiaError::new(SkiaErrorCode::InvalidGeometry));
                }
                write_runtime_register_kind(
                    &mut registers,
                    destination,
                    RuntimeShaderRegisterKind::Scalar,
                    limits,
                    &mut register_count,
                )?;
            }
            RuntimeShaderInstruction::Add {
                destination,
                first,
                second,
            }
            | RuntimeShaderInstruction::Multiply {
                destination,
                first,
                second,
            } => {
                expect_runtime_register_kind(&registers, first, RuntimeShaderRegisterKind::Color)?;
                expect_runtime_register_kind(&registers, second, RuntimeShaderRegisterKind::Color)?;
                write_runtime_register_kind(
                    &mut registers,
                    destination,
                    RuntimeShaderRegisterKind::Color,
                    limits,
                    &mut register_count,
                )?;
            }
            RuntimeShaderInstruction::Mix {
                destination,
                first,
                second,
                factor,
            } => {
                expect_runtime_register_kind(&registers, first, RuntimeShaderRegisterKind::Color)?;
                expect_runtime_register_kind(&registers, second, RuntimeShaderRegisterKind::Color)?;
                expect_runtime_register_kind(
                    &registers,
                    factor,
                    RuntimeShaderRegisterKind::Scalar,
                )?;
                write_runtime_register_kind(
                    &mut registers,
                    destination,
                    RuntimeShaderRegisterKind::Color,
                    limits,
                    &mut register_count,
                )?;
            }
            RuntimeShaderInstruction::Clamp {
                destination,
                source,
            } => {
                expect_runtime_register_kind(&registers, source, RuntimeShaderRegisterKind::Color)?;
                write_runtime_register_kind(
                    &mut registers,
                    destination,
                    RuntimeShaderRegisterKind::Color,
                    limits,
                    &mut register_count,
                )?;
            }
            RuntimeShaderInstruction::Return { source } => {
                if !final_instruction {
                    return Err(SkiaError::new(SkiaErrorCode::InvalidResource));
                }
                expect_runtime_register_kind(&registers, source, RuntimeShaderRegisterKind::Color)?;
            }
        }
    }
    if !matches!(
        instructions.last(),
        Some(RuntimeShaderInstruction::Return { .. })
    ) {
        return Err(SkiaError::new(SkiaErrorCode::InvalidResource));
    }
    Ok(register_count)
}

fn write_runtime_register_kind(
    registers: &mut [Option<RuntimeShaderRegisterKind>; RUNTIME_SHADER_MAX_REGISTERS],
    register: u8,
    kind: RuntimeShaderRegisterKind,
    limits: RuntimeShaderLimits,
    register_count: &mut u8,
) -> Result<(), SkiaError> {
    let index = usize::from(register);
    if index >= usize::from(limits.max_registers()) {
        return Err(SkiaError::new(SkiaErrorCode::ResourceLimit));
    }
    registers[index] = Some(kind);
    *register_count = (*register_count).max(register.saturating_add(1));
    Ok(())
}

fn expect_runtime_register_kind(
    registers: &[Option<RuntimeShaderRegisterKind>; RUNTIME_SHADER_MAX_REGISTERS],
    register: u8,
    expected: RuntimeShaderRegisterKind,
) -> Result<(), SkiaError> {
    if registers.get(usize::from(register)).copied().flatten() == Some(expected) {
        Ok(())
    } else {
        Err(SkiaError::new(SkiaErrorCode::InvalidResource))
    }
}

fn write_runtime_register(
    registers: &mut [Option<RuntimeShaderValue>; RUNTIME_SHADER_MAX_REGISTERS],
    register: u8,
    value: RuntimeShaderValue,
) -> Result<(), SkiaError> {
    let target = registers
        .get_mut(usize::from(register))
        .ok_or(SkiaError::new(SkiaErrorCode::InvalidResource))?;
    *target = Some(value);
    Ok(())
}

fn runtime_color_register(
    registers: &[Option<RuntimeShaderValue>; RUNTIME_SHADER_MAX_REGISTERS],
    register: u8,
) -> Result<[i32; 4], SkiaError> {
    match registers.get(usize::from(register)).copied().flatten() {
        Some(RuntimeShaderValue::Color(color)) => Ok(color),
        _ => Err(SkiaError::new(SkiaErrorCode::InvalidResource)),
    }
}

fn runtime_scalar_register(
    registers: &[Option<RuntimeShaderValue>; RUNTIME_SHADER_MAX_REGISTERS],
    register: u8,
) -> Result<i32, SkiaError> {
    match registers.get(usize::from(register)).copied().flatten() {
        Some(RuntimeShaderValue::Scalar(value)) => Ok(value),
        _ => Err(SkiaError::new(SkiaErrorCode::InvalidResource)),
    }
}

fn runtime_color(color: Color) -> [i32; 4] {
    color.channels().map(|channel| i32::from(channel) << 16)
}

fn runtime_color_from_value(color: [i32; 4]) -> Color {
    let channels = color.map(|channel| {
        let rounded = (channel.clamp(0, RUNTIME_SHADER_CHANNEL_MAX) + (1 << 15)) >> 16;
        u8::try_from(rounded).unwrap_or(u8::MAX)
    });
    Color::rgba(channels[0], channels[1], channels[2], channels[3])
}

fn runtime_coordinate(value: Scalar, start: Scalar, end: Scalar) -> Result<i32, SkiaError> {
    let numerator = i128::from(value.bits()) - i128::from(start.bits());
    let denominator = i128::from(end.bits()) - i128::from(start.bits());
    if denominator == 0 {
        return Err(SkiaError::new(SkiaErrorCode::InvalidGeometry));
    }
    let scaled = numerator
        .checked_mul(i128::from(RUNTIME_SHADER_ONE))
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    let value = rounded_ratio(scaled, denominator).clamp(0, i128::from(RUNTIME_SHADER_ONE));
    i32::try_from(value).map_err(|_| SkiaError::new(SkiaErrorCode::NumericOverflow))
}

fn runtime_add(first: [i32; 4], second: [i32; 4]) -> [i32; 4] {
    std::array::from_fn(|index| {
        first[index]
            .saturating_add(second[index])
            .clamp(0, RUNTIME_SHADER_CHANNEL_MAX)
    })
}

fn runtime_multiply(first: [i32; 4], second: [i32; 4]) -> [i32; 4] {
    std::array::from_fn(|index| {
        let product = i64::from(first[index]) * i64::from(second[index]);
        let rounded = product + i64::from(RUNTIME_SHADER_CHANNEL_MAX) / 2;
        i32::try_from(rounded / i64::from(RUNTIME_SHADER_CHANNEL_MAX))
            .unwrap_or(RUNTIME_SHADER_CHANNEL_MAX)
            .clamp(0, RUNTIME_SHADER_CHANNEL_MAX)
    })
}

fn runtime_mix(first: [i32; 4], second: [i32; 4], factor: i32) -> [i32; 4] {
    let factor = factor.clamp(0, RUNTIME_SHADER_ONE);
    std::array::from_fn(|index| {
        let mixed = i64::from(first[index]) * i64::from(RUNTIME_SHADER_ONE - factor)
            + i64::from(second[index]) * i64::from(factor);
        i32::try_from((mixed + i64::from(RUNTIME_SHADER_ONE) / 2) / i64::from(RUNTIME_SHADER_ONE))
            .unwrap_or(RUNTIME_SHADER_CHANNEL_MAX)
            .clamp(0, RUNTIME_SHADER_CHANNEL_MAX)
    })
}

fn runtime_clamp(color: [i32; 4]) -> [i32; 4] {
    color.map(|channel| channel.clamp(0, RUNTIME_SHADER_CHANNEL_MAX))
}
