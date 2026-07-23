#[path = "support/render_cases.rs"]
mod render_cases;

use std::{env, fs};

use render_cases::{
    CASES, golden_directory, render_cpu, render_software_gpu, sha256, write_goldens,
};
use skia_codec::ImageCodec;

#[test]
fn owned_scenes_match_cpu_and_software_gpu_pixels() {
    for case in CASES {
        let cpu = render_cpu(case);
        let software_gpu = render_software_gpu(case);
        assert_eq!(
            (software_gpu.width(), software_gpu.height()),
            (cpu.width(), cpu.height()),
            "{} dimensions",
            case.name
        );
        assert_eq!(software_gpu.pixels(), cpu.pixels(), "{} pixels", case.name);
    }
}

#[test]
#[ignore = "requires local generated golden data; run scripts/regenerate_goldens.sh first"]
fn local_goldens_match_software_replay() {
    let directory = golden_directory();
    let manifest = fs::read_to_string(directory.join("manifest.toml")).expect("golden manifest");
    for case in CASES {
        assert!(
            manifest.contains(&format!("name = \"{}\"", case.name)),
            "manifest entry for {}",
            case.name
        );
        let expected =
            fs::read(directory.join(format!("{}.rgba", case.name))).expect("raw RGBA golden");
        let actual = render_software_gpu(case);
        assert_eq!(expected, actual.pixels(), "{} golden pixels", case.name);
        assert!(
            manifest.contains(&format!("raw_rgba_sha256 = \"{}\"", sha256(&expected))),
            "raw hash for {}",
            case.name
        );
        let png = fs::read(directory.join(format!("{}.png", case.name))).expect("PNG golden");
        assert!(
            manifest.contains(&format!("png_sha256 = \"{}\"", sha256(&png))),
            "PNG hash for {}",
            case.name
        );
        let decoded = ImageCodec::decode(&png).expect("decode PNG golden");
        assert_eq!(
            decoded.image().pixels(),
            actual.pixels(),
            "{} PNG pixels",
            case.name
        );
    }
}

#[test]
#[ignore = "regenerates reviewed goldens; requires SKIA_UPDATE_GOLDENS=1"]
fn regenerate_owned_goldens() {
    assert_eq!(
        env::var("SKIA_UPDATE_GOLDENS").as_deref(),
        Ok("1"),
        "set SKIA_UPDATE_GOLDENS=1 to intentionally rewrite golden files"
    );
    write_goldens(&golden_directory()).expect("write goldens");
}
