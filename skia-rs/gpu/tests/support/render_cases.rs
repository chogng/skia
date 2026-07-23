// Shared owned render scenes for the pixel-oracle integration test.
use std::{
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use skia_codec::{EncodeFormat, EncodeOptions, ImageAsset, ImageCodec, PngOptions};
use skia_core::{
    Color, FillRule, Gradient, GradientStop, Paint, PathBuilder, Point, Rect, SaveLayerOptions,
    Scalar, TileMode, Transform,
};
use skia_cpu::{ClipRect, Surface, SurfaceLimits};
use skia_gpu::{GpuBackend, GpuCommandEncoder, GpuSurfaceDescriptor, software::SoftwareGpuBackend};
use skia_image::Image;

pub const WIDTH: u32 = 8;
pub const HEIGHT: u32 = 8;

#[derive(Clone, Copy, Debug)]
pub struct RenderCase {
    pub name: &'static str,
}

pub const CASES: [RenderCase; 4] = [
    RenderCase { name: "clip_alpha" },
    RenderCase {
        name: "even_odd_path",
    },
    RenderCase {
        name: "gradient_layer",
    },
    RenderCase {
        name: "linear_image",
    },
];

pub fn render_cpu(case: RenderCase) -> Surface {
    let mut surface = Surface::new(WIDTH, HEIGHT, SurfaceLimits::default()).expect("surface");
    let mut canvas = surface.canvas();
    match case.name {
        "clip_alpha" => {
            canvas.clear(Color::WHITE);
            canvas.save().expect("save");
            canvas
                .clip_rect(ClipRect::new(rect(1, 1, 7, 7)))
                .expect("clip");
            canvas
                .fill_rect(rect(0, 0, 8, 8), Paint::new(Color::rgba(255, 0, 0, 128)))
                .expect("fill");
            canvas.restore().expect("restore");
        }
        "even_odd_path" => {
            let path = donut_path();
            canvas.clear(Color::WHITE);
            canvas.set_transform(Transform::translate(scalar(1), scalar(1)));
            canvas
                .fill_path(
                    &path,
                    FillRule::EvenOdd,
                    Paint::new(Color::rgba(0, 0, 255, 255)),
                )
                .expect("fill path");
        }
        "gradient_layer" => {
            canvas.clear(Color::BLUE);
            canvas
                .save_layer(
                    SaveLayerOptions::new()
                        .with_bounds(rect(1, 1, 7, 7))
                        .with_opacity(192),
                )
                .expect("save layer");
            canvas
                .fill_rect(rect(0, 0, 8, 8), gradient_paint())
                .expect("fill gradient");
            canvas.restore().expect("restore layer");
        }
        "linear_image" => {
            canvas.clear(Color::WHITE);
            canvas
                .draw_image_with_sampling(
                    &source_image(),
                    rect(1, 1, 7, 7),
                    u8::MAX,
                    skia_core::BlendMode::SourceOver,
                    skia_core::SamplingOptions::LINEAR,
                )
                .expect("draw image");
        }
        name => panic!("unknown render case {name}"),
    }
    drop(canvas);
    surface
}

pub fn render_software_gpu(case: RenderCase) -> Surface {
    let mut encoder = GpuCommandEncoder::new(16).expect("encoder");
    match case.name {
        "clip_alpha" => {
            encoder.clear(Color::WHITE).expect("clear");
            encoder.save().expect("save");
            encoder.clip_rect(rect(1, 1, 7, 7)).expect("clip");
            encoder
                .fill_rect(rect(0, 0, 8, 8), Paint::new(Color::rgba(255, 0, 0, 128)))
                .expect("fill");
            encoder.restore().expect("restore");
        }
        "even_odd_path" => {
            let path = encoder.add_path(donut_path()).expect("path");
            encoder.clear(Color::WHITE).expect("clear");
            encoder.set_transform(Transform::translate(scalar(1), scalar(1)));
            encoder
                .fill_path(
                    path,
                    FillRule::EvenOdd,
                    Paint::new(Color::rgba(0, 0, 255, 255)),
                )
                .expect("fill path");
        }
        "gradient_layer" => {
            encoder.clear(Color::BLUE).expect("clear");
            encoder
                .save_layer(
                    SaveLayerOptions::new()
                        .with_bounds(rect(1, 1, 7, 7))
                        .with_opacity(192),
                )
                .expect("save layer");
            encoder
                .fill_rect(rect(0, 0, 8, 8), gradient_paint())
                .expect("fill gradient");
            encoder.restore().expect("restore layer");
        }
        "linear_image" => {
            let image = encoder.add_image(source_image()).expect("image");
            encoder.clear(Color::WHITE).expect("clear");
            encoder
                .draw_image_with_sampling(
                    image,
                    rect(1, 1, 7, 7),
                    u8::MAX,
                    skia_core::BlendMode::SourceOver,
                    skia_core::SamplingOptions::LINEAR,
                )
                .expect("draw image");
        }
        name => panic!("unknown render case {name}"),
    }
    let commands = encoder.finish();
    let mut backend = SoftwareGpuBackend::default();
    let mut surface = backend
        .create_surface(GpuSurfaceDescriptor::new(WIDTH, HEIGHT).expect("descriptor"))
        .expect("software surface");
    backend
        .submit(&mut surface, &commands)
        .expect("software replay");
    surface
}

pub fn golden_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

pub fn write_goldens(directory: &Path) -> Result<(), String> {
    fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let mut manifest = String::from(
        "version = 1\nrenderer = \"skia-gpu software replay\"\ncolor_space = \"sRGB\"\n\n",
    );
    for (index, case) in CASES.into_iter().enumerate() {
        if index != 0 {
            writeln!(manifest).expect("string write");
        }
        let surface = render_software_gpu(case);
        let raw_path = directory.join(format!("{}.rgba", case.name));
        let png_path = directory.join(format!("{}.png", case.name));
        fs::write(&raw_path, surface.pixels()).map_err(|error| error.to_string())?;
        let image = Image::from_rgba8(WIDTH, HEIGHT, surface.pixels().to_vec())
            .map_err(|error| error.to_string())?;
        let encoded = ImageCodec::encode(
            &ImageAsset::new(image),
            &EncodeOptions::new(EncodeFormat::Png(PngOptions::balanced_v1())),
        )
        .map_err(|error| error.to_string())?;
        fs::write(&png_path, encoded.bytes()).map_err(|error| error.to_string())?;
        writeln!(manifest, "[[golden]]").expect("string write");
        writeln!(manifest, "name = \"{}\"", case.name).expect("string write");
        writeln!(manifest, "width = {WIDTH}").expect("string write");
        writeln!(manifest, "height = {HEIGHT}").expect("string write");
        writeln!(
            manifest,
            "raw_rgba_sha256 = \"{}\"",
            sha256(surface.pixels())
        )
        .expect("string write");
        writeln!(manifest, "png_sha256 = \"{}\"", sha256(encoded.bytes())).expect("string write");
    }
    fs::write(directory.join("manifest.toml"), manifest).map_err(|error| error.to_string())
}

fn scalar(value: i32) -> Scalar {
    Scalar::from_i32(value).expect("fixed-point scalar")
}

fn point(x: i32, y: i32) -> Point {
    Point::new(scalar(x), scalar(y))
}

fn rect(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
    Rect::new(scalar(left), scalar(top), scalar(right), scalar(bottom)).expect("rectangle")
}

fn donut_path() -> skia_core::Path {
    let mut builder = PathBuilder::new(10).expect("path builder");
    builder.add_rect(rect(0, 0, 6, 6)).expect("outer rect");
    builder.add_rect(rect(2, 2, 4, 4)).expect("inner rect");
    builder.finish().expect("path")
}

fn gradient_paint() -> Paint {
    let stops = [
        GradientStop::new(Scalar::ZERO, Color::RED).expect("red stop"),
        GradientStop::new(scalar(1), Color::GREEN).expect("green stop"),
    ];
    Paint::from_gradient(
        Gradient::linear(point(0, 0), point(8, 8), &stops, TileMode::Clamp).expect("gradient"),
    )
}

fn source_image() -> Image {
    Image::from_rgba8(
        2,
        2,
        vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ],
    )
    .expect("source image")
}

pub fn sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(output, "{byte:02x}").expect("string write");
    }
    output
}
