use std::collections::HashMap;

use skia_core::{
    Paint, RuntimeShader, RuntimeShaderInstruction, RuntimeShaderLimits, RuntimeShaderProgram,
};

/// Words retained for each portable runtime-shader instruction.
pub const RUNTIME_SHADER_INSTRUCTION_WORDS: usize = 6;
/// Maximum portable runtime-shader instruction count.
pub const RUNTIME_SHADER_MAX_INSTRUCTIONS: usize = RuntimeShaderLimits::MAX_INSTRUCTIONS as usize;
/// Maximum portable runtime-shader color-uniform count.
pub const RUNTIME_SHADER_MAX_UNIFORMS: usize = RuntimeShaderLimits::MAX_UNIFORMS as usize;

const DEFAULT_RUNTIME_SHADER_PACKET_CACHE_CAPACITY: usize = 64;

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

#[derive(Clone, Debug)]
struct RuntimeShaderProgramPacket {
    instruction_count: u32,
    instructions: [[u32; RUNTIME_SHADER_INSTRUCTION_WORDS]; RUNTIME_SHADER_MAX_INSTRUCTIONS],
}

#[derive(Clone, Debug)]
struct CachedRuntimeShaderProgram {
    program: RuntimeShaderProgram,
    packet: RuntimeShaderProgramPacket,
    last_used: u64,
}

/// Observable statistics for a [`RuntimeShaderPacketCache`].
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct RuntimeShaderPacketCacheStats {
    hits: u64,
    misses: u64,
    evictions: u64,
    entries: usize,
}

impl RuntimeShaderPacketCacheStats {
    /// Returns the number of program encodings reused without re-encoding instructions.
    pub const fn hits(self) -> u64 {
        self.hits
    }

    /// Returns the number of newly encoded runtime shader programs.
    pub const fn misses(self) -> u64 {
        self.misses
    }

    /// Returns the number of least-recently-used program encodings evicted.
    pub const fn evictions(self) -> u64 {
        self.evictions
    }

    /// Returns the number of retained program encodings.
    pub const fn entries(self) -> usize {
        self.entries
    }
}

/// Bounded program-hash cache for reusable runtime shader instruction packets.
///
/// The existing Metal and Vulkan paint pipelines interpret all programs through
/// one precompiled VM, so this cache stores encoded instruction streams rather
/// than redundant per-program native pipelines. Uniform values are bound fresh
/// for every returned [`RuntimeShaderPacket`].
#[derive(Debug)]
pub struct RuntimeShaderPacketCache {
    capacity: usize,
    clock: u64,
    entries: HashMap<u64, Vec<CachedRuntimeShaderProgram>>,
    entry_count: usize,
    hits: u64,
    misses: u64,
    evictions: u64,
}

impl RuntimeShaderPacketCache {
    /// Creates a cache that retains up to `capacity` distinct source programs.
    ///
    /// A zero capacity retains one program so every runtime shader remains
    /// encodable while cache storage stays bounded.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            clock: 0,
            entries: HashMap::new(),
            entry_count: 0,
            hits: 0,
            misses: 0,
            evictions: 0,
        }
    }

    /// Encodes a paint's runtime shader, reusing its cached source program when available.
    pub fn packet(&mut self, paint: &Paint) -> Option<RuntimeShaderPacket> {
        let runtime = paint.shader_handle()?.as_shader().runtime()?;
        self.clock = self.clock.wrapping_add(1);
        let last_used = self.clock;
        let fingerprint = runtime_shader_program_fingerprint(runtime.program());
        if let Some(entries) = self.entries.get_mut(&fingerprint)
            && let Some(entry) = entries
                .iter_mut()
                .find(|entry| entry.program == *runtime.program())
        {
            self.hits = self.hits.saturating_add(1);
            entry.last_used = last_used;
            return Some(bind_runtime_shader_packet(&entry.packet, runtime));
        }

        self.misses = self.misses.saturating_add(1);
        if self.entry_count >= self.capacity {
            self.evict_least_recently_used();
        }
        let packet = encode_runtime_shader_program(runtime.program())?;
        self.entries
            .entry(fingerprint)
            .or_default()
            .push(CachedRuntimeShaderProgram {
                program: runtime.program().clone(),
                packet: packet.clone(),
                last_used,
            });
        self.entry_count = self.entry_count.saturating_add(1);
        Some(bind_runtime_shader_packet(&packet, runtime))
    }

    /// Returns the current program-packet cache counters.
    pub const fn stats(&self) -> RuntimeShaderPacketCacheStats {
        RuntimeShaderPacketCacheStats {
            hits: self.hits,
            misses: self.misses,
            evictions: self.evictions,
            entries: self.entry_count,
        }
    }

    fn evict_least_recently_used(&mut self) {
        let candidate = self
            .entries
            .iter()
            .flat_map(|(fingerprint, entries)| {
                entries
                    .iter()
                    .enumerate()
                    .map(move |(index, entry)| (*fingerprint, index, entry.last_used))
            })
            .min_by_key(|(_, _, last_used)| *last_used)
            .map(|(fingerprint, index, _)| (fingerprint, index));
        let Some((fingerprint, index)) = candidate else {
            return;
        };
        let empty = {
            let entries = self
                .entries
                .get_mut(&fingerprint)
                .expect("cache candidate remains present");
            entries.swap_remove(index);
            entries.is_empty()
        };
        if empty {
            self.entries.remove(&fingerprint);
        }
        self.entry_count = self.entry_count.saturating_sub(1);
        self.evictions = self.evictions.saturating_add(1);
    }
}

impl Default for RuntimeShaderPacketCache {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_RUNTIME_SHADER_PACKET_CACHE_CAPACITY)
    }
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
    Some(bind_runtime_shader_packet(
        &encode_runtime_shader_program(runtime.program())?,
        runtime,
    ))
}

fn encode_runtime_shader_program(
    program: &RuntimeShaderProgram,
) -> Option<RuntimeShaderProgramPacket> {
    let mut packet = RuntimeShaderProgramPacket {
        instruction_count: u32::try_from(program.instructions().len()).ok()?,
        instructions: [[0; RUNTIME_SHADER_INSTRUCTION_WORDS]; RUNTIME_SHADER_MAX_INSTRUCTIONS],
    };
    for (output, instruction) in packet.instructions.iter_mut().zip(program.instructions()) {
        *output = encode_instruction(*instruction);
    }
    Some(packet)
}

fn bind_runtime_shader_packet(
    program: &RuntimeShaderProgramPacket,
    runtime: &RuntimeShader,
) -> RuntimeShaderPacket {
    let mut packet = RuntimeShaderPacket {
        instruction_count: program.instruction_count,
        instructions: program.instructions,
        uniforms: [0; RUNTIME_SHADER_MAX_UNIFORMS],
    };
    for (output, color) in packet.uniforms.iter_mut().zip(runtime.uniforms()) {
        *output = u32::from_le_bytes(color.channels());
    }
    packet
}

fn runtime_shader_program_fingerprint(program: &RuntimeShaderProgram) -> u64 {
    let mut state = 14_695_981_039_346_656_037_u64;
    fingerprint_word(&mut state, u32::from(program.uniform_count()));
    for instruction in program.instructions() {
        match *instruction {
            RuntimeShaderInstruction::ConstantColor { destination, color } => {
                fingerprint_words(
                    &mut state,
                    &[
                        1,
                        u32::from(destination),
                        u32::from_le_bytes(color.channels()),
                    ],
                );
            }
            RuntimeShaderInstruction::UniformColor {
                destination,
                uniform,
            } => fingerprint_words(&mut state, &[2, u32::from(destination), u32::from(uniform)]),
            RuntimeShaderInstruction::LocalX {
                destination,
                start,
                end,
            } => fingerprint_words(
                &mut state,
                &[
                    3,
                    u32::from(destination),
                    start.bits() as u32,
                    end.bits() as u32,
                ],
            ),
            RuntimeShaderInstruction::LocalY {
                destination,
                start,
                end,
            } => fingerprint_words(
                &mut state,
                &[
                    4,
                    u32::from(destination),
                    start.bits() as u32,
                    end.bits() as u32,
                ],
            ),
            RuntimeShaderInstruction::Add {
                destination,
                first,
                second,
            } => fingerprint_words(
                &mut state,
                &[
                    5,
                    u32::from(destination),
                    u32::from(first),
                    u32::from(second),
                ],
            ),
            RuntimeShaderInstruction::Multiply {
                destination,
                first,
                second,
            } => fingerprint_words(
                &mut state,
                &[
                    6,
                    u32::from(destination),
                    u32::from(first),
                    u32::from(second),
                ],
            ),
            RuntimeShaderInstruction::Mix {
                destination,
                first,
                second,
                factor,
            } => fingerprint_words(
                &mut state,
                &[
                    7,
                    u32::from(destination),
                    u32::from(first),
                    u32::from(second),
                    u32::from(factor),
                ],
            ),
            RuntimeShaderInstruction::Clamp {
                destination,
                source,
            } => fingerprint_words(&mut state, &[8, u32::from(destination), u32::from(source)]),
            RuntimeShaderInstruction::Return { source } => {
                fingerprint_words(&mut state, &[9, u32::from(source)]);
            }
        }
    }
    state
}

fn fingerprint_words(state: &mut u64, words: &[u32]) {
    for word in words {
        fingerprint_word(state, *word);
    }
}

fn fingerprint_word(state: &mut u64, word: u32) {
    *state ^= u64::from(word);
    *state = state.wrapping_mul(1_099_511_628_211);
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
