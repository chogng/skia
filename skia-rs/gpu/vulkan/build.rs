use std::{env, fmt::Write, fs, path::PathBuf};

const RUNTIME_SHADER_INSTRUCTION_COUNT: usize = 64;
const RUNTIME_SHADER_INSTRUCTION_WORDS: usize = 6;
const RUNTIME_SHADER_SPECIALIZATION_MARKER: &str = "// RUNTIME_SHADER_SPECIALIZATION";

fn main() {
    let shader_path = PathBuf::from("shaders/renderer.wgsl");
    println!("cargo:rerun-if-changed={}", shader_path.display());
    let source = specialize_runtime_shader_source(
        fs::read_to_string(&shader_path).expect("read Vulkan WGSL shader"),
    );
    let module = naga::front::wgsl::parse_str(&source).expect("parse Vulkan WGSL shader");
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .expect("validate Vulkan WGSL shader");
    let options = naga::back::spv::Options::default();
    let pipeline = naga::back::spv::PipelineOptions {
        shader_stage: naga::ShaderStage::Compute,
        entry_point: "main".to_owned(),
    };
    let words = naga::back::spv::write_vec(&module, &info, &options, Some(&pipeline))
        .expect("compile Vulkan WGSL shader to SPIR-V");
    let bytes = words
        .iter()
        .flat_map(|word| word.to_le_bytes())
        .collect::<Vec<_>>();
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR")).join("renderer.spv");
    fs::write(output, bytes).expect("write Vulkan SPIR-V shader");
}

fn specialize_runtime_shader_source(source: String) -> String {
    let generated = runtime_shader_specialization_source();
    assert!(
        source.contains(RUNTIME_SHADER_SPECIALIZATION_MARKER),
        "runtime shader specialization marker"
    );
    source.replace(RUNTIME_SHADER_SPECIALIZATION_MARKER, &generated)
}

fn runtime_shader_specialization_source() -> String {
    let mut output = String::new();
    writeln!(output, "const runtime_pipeline_specialized: bool = false;")
        .expect("write specialization mode");
    writeln!(
        output,
        "const runtime_specialized_instruction_count: u32 = 0u;"
    )
    .expect("write instruction count specialization");
    for index in 0..RUNTIME_SHADER_INSTRUCTION_COUNT * RUNTIME_SHADER_INSTRUCTION_WORDS {
        writeln!(output, "const runtime_specialized_word_{index}: u32 = 0u;")
            .expect("write instruction specialization");
    }
    writeln!(
        output,
        "fn specialized_runtime_instruction_word(instruction: u32, word: u32) -> u32 {{\n    switch (instruction) {{"
    )
    .expect("write specialization function");
    for instruction in 0..RUNTIME_SHADER_INSTRUCTION_COUNT {
        writeln!(
            output,
            "        case {instruction}u: {{\n            switch (word) {{"
        )
        .expect("write instruction switch");
        for word in 0..RUNTIME_SHADER_INSTRUCTION_WORDS {
            let index = instruction * RUNTIME_SHADER_INSTRUCTION_WORDS + word;
            writeln!(
                output,
                "                case {word}u: {{ return runtime_specialized_word_{index}; }}"
            )
            .expect("write word switch");
        }
        writeln!(
            output,
            "                default: {{ return 0u; }}\n            }}\n        }}"
        )
        .expect("write instruction switch end");
    }
    writeln!(output, "        default: {{ return 0u; }}\n    }}\n}}")
        .expect("write specialization function end");
    output
}
