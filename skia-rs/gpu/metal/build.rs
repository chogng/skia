use std::path::PathBuf;
use std::process::Command;
use std::{env, fmt::Write, fs};

const RUNTIME_SHADER_INSTRUCTION_COUNT: usize = 64;
const RUNTIME_SHADER_INSTRUCTION_WORDS: usize = 6;
const RUNTIME_SHADER_SPECIALIZATION_MARKER: &str = "// RUNTIME_SHADER_SPECIALIZATION";

fn main() {
    println!("cargo:rerun-if-changed=../../shaders/solid_rect.metal");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }

    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let source = manifest.join("../../shaders/solid_rect.metal");
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("build output directory"));
    let generated_source = output.join("solid_rect.generated.metal");
    let air = output.join("solid_rect.air");
    let library = output.join("solid_rect.metallib");
    fs::write(
        &generated_source,
        specialize_runtime_shader_source(fs::read_to_string(&source).expect("read Metal shader")),
    )
    .expect("write specialized Metal shader");

    if let Err(error) = run(
        "metal",
        [
            "-c",
            generated_source.to_str().expect("UTF-8 shader path"),
            "-o",
            air.to_str().expect("UTF-8 AIR path"),
        ],
    ) {
        println!("cargo:warning={error}; Metal shader compilation skipped");
        return;
    }
    if let Err(error) = run(
        "metallib",
        [
            air.to_str().expect("UTF-8 AIR path"),
            "-o",
            library.to_str().expect("UTF-8 Metal library path"),
        ],
    ) {
        println!("cargo:warning={error}; Metal shader library generation skipped");
        return;
    }
    println!("cargo:rustc-env=SKIA_METAL_LIBRARY={}", library.display());
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
    writeln!(
        output,
        "constant bool runtime_pipeline_specialized [[function_constant(0)]];"
    )
    .expect("write specialization mode");
    writeln!(
        output,
        "constant uint runtime_specialized_instruction_count [[function_constant(1)]];"
    )
    .expect("write instruction count specialization");
    for index in 0..RUNTIME_SHADER_INSTRUCTION_COUNT * RUNTIME_SHADER_INSTRUCTION_WORDS {
        writeln!(
            output,
            "constant uint runtime_specialized_word_{index} [[function_constant({})]];",
            index + 2
        )
        .expect("write instruction specialization");
    }
    writeln!(
        output,
        "inline uint specialized_runtime_instruction_word(uint instruction, uint word) {{\n    switch (instruction) {{"
    )
    .expect("write specialization function");
    for instruction in 0..RUNTIME_SHADER_INSTRUCTION_COUNT {
        writeln!(
            output,
            "        case {instruction}: {{\n            switch (word) {{"
        )
        .expect("write instruction switch");
        for word in 0..RUNTIME_SHADER_INSTRUCTION_WORDS {
            let index = instruction * RUNTIME_SHADER_INSTRUCTION_WORDS + word;
            writeln!(
                output,
                "                case {word}: return runtime_specialized_word_{index};"
            )
            .expect("write word switch");
        }
        writeln!(
            output,
            "                default: return 0;\n            }}\n        }}"
        )
        .expect("write instruction switch end");
    }
    writeln!(output, "        default: return 0;\n    }}\n}}")
        .expect("write specialization function end");
    output
}

fn run<'a>(tool: &str, arguments: impl IntoIterator<Item = &'a str>) -> Result<(), String> {
    let status = Command::new("xcrun")
        .args(["--sdk", "macosx", tool])
        .args(arguments)
        .status()
        .map_err(|error| format!("failed to launch {tool}: {error}"))?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| format!("{tool} failed with {status}"))
}
