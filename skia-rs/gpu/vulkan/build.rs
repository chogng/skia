use std::{env, fs, path::PathBuf};

fn main() {
    let shader_path = PathBuf::from("shaders/renderer.wgsl");
    println!("cargo:rerun-if-changed={}", shader_path.display());
    let source = fs::read_to_string(&shader_path).expect("read Vulkan WGSL shader");
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
