use skia_core::{
    Color, DisplayListBuilder, DrawCommand, FillRule, FontCollection, FontCollectionLimits,
    FontFace, FontId, FontLimits, Paint, PathVerb, Rect, Scalar, StrokeCap, StrokeJoin,
};
use skia_image::Image;

use super::{
    SvgOptions, SvgReadErrorCode, SvgReadLimits, SvgReadOptions, SvgReader, SvgViewBoxAlignment,
    SvgViewBoxScale, SvgWriter,
};

fn decode(source: &str) -> super::SvgDocument {
    SvgReader::decode(source.as_bytes(), SvgReadOptions::default()).expect("decoded SVG")
}

fn scalar(value: i32) -> Scalar {
    Scalar::from_i32(value).expect("scalar")
}

fn test_fonts() -> FontCollection {
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    let bytes = include_bytes!("../../text/tests/fonts/skia/resources/fonts/test.ttc").to_vec();
    for index in 0..2 {
        let face = FontFace::from_bytes_with_limits(
            FontId::new(700 + index as u64),
            bytes.clone(),
            index,
            FontLimits::default(),
        )
        .expect("test font");
        fonts.add_face(face).expect("font collection");
    }
    fonts
}

#[test]
fn basic_shapes_lower_in_document_order_with_inherited_style() {
    let document = decode(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="120" height="80">
          <g fill="#123456" stroke="red" stroke-width="2">
            <rect x="1" y="2" width="10" height="20"/>
            <circle cx="30" cy="20" r="5" fill="none"/>
          </g>
        </svg>"##,
    );

    assert_eq!(document.canvas().width().bits(), 120 << 16);
    assert_eq!(document.canvas().height().bits(), 80 << 16);
    let commands = document.display_list().commands();
    assert_eq!(commands.len(), 3);
    assert!(matches!(
        &commands[0],
        DrawCommand::FillPath {
            rule: FillRule::NonZero,
            paint,
            ..
        } if paint.color() == Color::rgb(0x12, 0x34, 0x56)
    ));
    assert!(matches!(
        &commands[1],
        DrawCommand::StrokePath { options, paint, .. }
            if options.width().bits() == 2 << 16 && paint.color() == Color::RED
    ));
    assert!(matches!(
        &commands[2],
        DrawCommand::StrokePath { paint, .. } if paint.color() == Color::RED
    ));
}

#[test]
fn path_data_supports_relative_and_smooth_segments() {
    let document = decode(
        r#"<svg width="40" height="40"><path fill-rule="evenodd"
            d="M1 2 l3 0 h2 v4 q2 2 4 0 t4 0 c1 0 2 1 3 1 s2 1 3 0 z"/></svg>"#,
    );
    let DrawCommand::FillPath { path, rule, .. } = document.display_list().commands()[0] else {
        panic!("fill path");
    };
    assert_eq!(rule, FillRule::EvenOdd);
    let path = document.display_list().path(path).expect("path resource");
    assert!(matches!(path.verbs().first(), Some(PathVerb::MoveTo(_))));
    assert!(matches!(path.verbs().last(), Some(PathVerb::Close)));
    assert_eq!(path.verbs().len(), 9);
}

#[test]
fn arcs_rotation_skew_and_namespaces_lower_without_losing_geometry() {
    let document = decode(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="60">
          <g transform="rotate(30 20 20) skewX(10) skewY(-5)">
            <path d="M5 20 A15 10 25 1 1 40 30 a5 5 0 0 0 10 0"/>
          </g>
        </svg>"#,
    );
    assert!(matches!(
        document.display_list().commands()[0],
        DrawCommand::Save
    ));
    let DrawCommand::FillPath { path, .. } = document.display_list().commands()[2] else {
        panic!("fill path");
    };
    let path = document.display_list().path(path).expect("arc path");
    assert!(
        path.verbs()
            .iter()
            .filter(|verb| matches!(verb, PathVerb::CubicTo(..)))
            .count()
            >= 2
    );

    let error = SvgReader::decode(
        br#"<fake:svg xmlns:fake="urn:not-svg" width="1" height="1"/>"#,
        SvgReadOptions::default(),
    )
    .expect_err("foreign namespace");
    assert_eq!(error.code(), SvgReadErrorCode::InvalidDocument);
}

#[test]
fn local_resources_gradients_use_and_clip_paths_round_trip() {
    let document = decode(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="30">
          <defs>
            <linearGradient id="paint">
              <stop offset="0%" stop-color="#ff0000"/>
              <stop offset="100%" stop-color="#0000ff" stop-opacity=".5"/>
            </linearGradient>
            <path id="shape" d="M2 2 H30 V20 H2 Z"/>
            <clipPath id="clip">
              <circle cx="16" cy="11" r="9"/>
              <rect x="2" y="2" width="5" height="5"/>
            </clipPath>
          </defs>
          <use href="#shape" x="2" fill="url(#paint)" clip-path="url(#clip)"/>
        </svg>"##,
    );
    assert!(
        document
            .display_list()
            .commands()
            .iter()
            .any(|command| matches!(command, DrawCommand::ClipPath { .. }))
    );
    assert!(document
        .display_list()
        .commands()
        .iter()
        .any(|command| matches!(command, DrawCommand::FillPath { paint, .. } if paint.shader_handle().is_some())));

    let output = String::from_utf8(
        SvgWriter::encode(
            document.canvas(),
            document.display_list(),
            SvgOptions::default(),
        )
        .expect("round-trip SVG"),
    )
    .expect("UTF-8");
    assert!(output.contains("<linearGradient"));
    assert!(output.contains("gradientTransform=\"matrix("));
    assert!(output.contains("<clipPath"));
}

#[test]
fn root_aspect_ratio_policy_and_reference_cycles_are_explicit() {
    let document = decode(
        r#"<svg width="40" height="20" viewBox="0 0 10 10"
             preserveAspectRatio="xMaxYMin slice"><rect width="10" height="10"/></svg>"#,
    );
    assert_eq!(
        document.canvas().preserve_aspect_ratio().alignment(),
        SvgViewBoxAlignment::XMaxYMin
    );
    assert_eq!(
        document.canvas().preserve_aspect_ratio().scale(),
        SvgViewBoxScale::Slice
    );
    let output = String::from_utf8(
        SvgWriter::encode(
            document.canvas(),
            document.display_list(),
            SvgOptions::default(),
        )
        .expect("preserved aspect ratio"),
    )
    .expect("UTF-8");
    assert!(output.contains(r#"preserveAspectRatio="xMaxYMin slice""#));

    let error = SvgReader::decode(
        br##"<svg width="10" height="10"><defs><g id="cycle"><use href="#cycle"/></g></defs><use href="#cycle"/></svg>"##,
        SvgReadOptions::default(),
    )
    .expect_err("cyclic use");
    assert_eq!(error.code(), SvgReadErrorCode::ResourceLimit);
}

#[test]
fn embedded_data_uri_images_decode_and_preserve_aspect_ratio() {
    let image = Image::from_rgba8(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]).expect("image");
    let mut builder = DisplayListBuilder::new(2).expect("display list");
    let image = builder.add_image(image).expect("image id");
    builder
        .draw_image(
            image,
            Rect::new(scalar(0), scalar(0), scalar(2), scalar(1)).expect("rect"),
            u8::MAX,
            Paint::default(),
        )
        .expect("draw image");
    let encoded = String::from_utf8(
        SvgWriter::encode(
            super::SvgCanvasSpec::new(scalar(2), scalar(1)).expect("canvas"),
            &builder.finish(),
            SvgOptions::default(),
        )
        .expect("encoded image SVG"),
    )
    .expect("UTF-8");
    let start = encoded.find("data:image/png;base64,").expect("data URI");
    let end = encoded[start..].find('"').expect("URI end") + start;
    let data_uri = &encoded[start..end];
    let source = format!(
        r#"<svg width="20" height="20"><image x="0" y="0" width="20" height="20" href="{data_uri}"/></svg>"#
    );
    let document = decode(&source);
    let DrawCommand::DrawImage { destination, .. } = document.display_list().commands()[0] else {
        panic!("image command");
    };
    assert_eq!(destination.left().bits(), 0);
    assert_eq!(destination.top().bits(), 5 << 16);
    assert_eq!(destination.right().bits(), 20 << 16);
    assert_eq!(destination.bottom().bits(), 15 << 16);
}

#[test]
fn nested_viewports_clip_and_map_their_view_box() {
    let document = decode(
        r#"<svg width="100" height="80">
          <svg x="10" y="20" width="40" height="20" viewBox="0 0 10 10"
               preserveAspectRatio="xMaxYMid meet">
            <rect width="10" height="10"/>
          </svg>
        </svg>"#,
    );
    let commands = document.display_list().commands();
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, DrawCommand::ClipRect { .. }))
    );
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, DrawCommand::ConcatTransform(_)))
    );
}

#[test]
fn transforms_group_opacity_and_stroke_geometry_survive_writer_round_trip() {
    let document = decode(
        r##"<svg width="20" height="10" viewBox="0 0 20 10">
          <g style="opacity:.5; stroke-dasharray:3 2 1" transform="translate(2,3) scale(2)"
             fill="none" stroke="#00ff00" stroke-linecap="square"
             stroke-linejoin="bevel" stroke-miterlimit="3">
            <line x1="0" y1="0" x2="4" y2="0"/>
          </g>
        </svg>"##,
    );
    let commands = document.display_list().commands();
    assert!(matches!(commands[0], DrawCommand::SaveLayer(_)));
    assert!(matches!(commands[1], DrawCommand::ConcatTransform(_)));
    assert!(matches!(
        &commands[2],
        DrawCommand::StrokePath { options, .. }
            if options.cap() == StrokeCap::Square
                && options.join() == StrokeJoin::Bevel
                && options.miter_limit().bits() == 3 << 16
    ));
    assert!(matches!(commands[3], DrawCommand::Restore));

    let bytes = SvgWriter::encode(
        document.canvas(),
        document.display_list(),
        SvgOptions::default(),
    )
    .expect("encoded SVG");
    let output = String::from_utf8(bytes).expect("UTF-8");
    assert!(output.contains("opacity=\"0.501961\""));
    assert!(output.contains("stroke-linecap=\"square\""));
    assert!(output.contains("stroke-dasharray=\"3 2 1 3 2 1\""));
}

#[test]
fn stylesheets_cascade_and_symbol_instances_establish_viewports() {
    let document = decode(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="60">
          <style>
            .themed { fill: red; stroke: black }
            symbol > .themed[data-state="active"] { fill: #2468ac }
            #instance { stroke: blue !important }
          </style>
          <defs>
            <symbol id="badge" viewBox="0 0 10 10" preserveAspectRatio="none">
              <rect class="themed" data-state="active" width="10" height="10"/>
            </symbol>
          </defs>
          <use id="instance" href="#badge" x="20" y="10" width="40" height="20"
               style="stroke: yellow"/>
        </svg>"##,
    );
    let commands = document.display_list().commands();
    assert!(commands.iter().any(
        |command| matches!(command, DrawCommand::ClipRect { rect, .. }
            if rect.left().bits() == 20 << 16
                && rect.top().bits() == 10 << 16
                && rect.right().bits() == 60 << 16
                && rect.bottom().bits() == 30 << 16)
    ));
    assert!(commands.iter().any(|command| matches!(
        command,
        DrawCommand::FillPath { paint, .. } if paint.color() == Color::rgb(0x24, 0x68, 0xac)
    )));
}

#[test]
fn text_uses_explicit_fonts_and_lowers_positioned_glyph_runs() {
    let fonts = test_fonts();
    let document = SvgReader::decode_with_fonts(
        br##"<svg width="100" height="30">
          <style>text { fill: #13579b; font-size: 14px }</style>
          <text x="10" y="20" text-anchor="middle">A1<tspan font-weight="700">2</tspan></text>
        </svg>"##,
        SvgReadOptions::default(),
        &fonts,
    )
    .expect("text SVG");
    assert!(
        document
            .display_list()
            .commands()
            .iter()
            .any(|command| matches!(
                command,
                DrawCommand::DrawPositionedGlyphRun { origin, paint, .. }
                    if origin.y().bits() == 20 << 16
                        && paint.color() == Color::rgb(0x13, 0x57, 0x9b)
            ))
    );
}

#[test]
fn alpha_masks_lower_as_destination_in_layers() {
    let document = decode(
        r##"<svg width="30" height="20">
          <defs>
            <mask id="fade" mask-type="alpha" maskUnits="userSpaceOnUse"
                  x="0" y="0" width="30" height="20">
              <rect width="15" height="20" fill="#ffffff80"/>
            </mask>
          </defs>
          <rect width="30" height="20" fill="red" mask="url(#fade)"/>
        </svg>"##,
    );
    assert!(
        document
            .display_list()
            .commands()
            .iter()
            .any(|command| matches!(
                command,
                DrawCommand::SaveLayer(options)
                    if options.blend_mode() == skia_core::BlendMode::DestinationIn
            ))
    );
}

#[test]
fn vector_patterns_tile_inside_the_target_path() {
    let document = decode(
        r##"<svg width="40" height="20">
          <defs>
            <pattern id="grid" width="25%" height="50%" viewBox="0 0 10 10"
                     preserveAspectRatio="none">
              <rect width="5" height="10" fill="#112233"/>
            </pattern>
          </defs>
          <rect x="0" y="0" width="40" height="20" fill="url(#grid)"/>
        </svg>"##,
    );
    let commands = document.display_list().commands();
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, DrawCommand::ClipPath { .. }))
    );
    assert_eq!(
        commands
            .iter()
            .filter(
                |command| matches!(command, DrawCommand::FillPath { paint, .. }
                if paint.color() == Color::rgb(0x11, 0x22, 0x33))
            )
            .count(),
        8
    );
}

#[test]
fn source_graphic_color_matrix_filters_lower_without_raster_fallback() {
    let document = decode(
        r##"<svg width="20" height="10">
          <defs>
            <filter id="tint">
              <feColorMatrix type="matrix"
                values="0 0 0 0 1
                        0 1 0 0 0
                        0 0 1 0 0
                        0 0 0 .5 0"/>
            </filter>
          </defs>
          <rect width="20" height="10" fill="white" filter="url(#tint)"/>
        </svg>"##,
    );
    assert!(
        document
            .display_list()
            .commands()
            .iter()
            .any(|command| matches!(
                command,
                DrawCommand::SaveLayer(options)
                    if matches!(
                        options.filter(),
                        Some(skia_core::ImageFilter::Color(skia_core::ColorFilter::Matrix(_)))
                    )
            ))
    );
}

#[test]
fn malformed_xml_and_unsupported_svg_are_distinct() {
    let malformed = SvgReader::decode(
        br#"<svg width="1" height="1"><path></svg>"#,
        SvgReadOptions::default(),
    )
    .expect_err("mismatched XML");
    assert_eq!(malformed.code(), SvgReadErrorCode::InvalidXml);
    assert!(malformed.xml_offset().is_some());

    let unsupported = SvgReader::decode(
        br#"<svg width="1" height="1"><text>hello</text></svg>"#,
        SvgReadOptions::default(),
    )
    .expect_err("missing font context");
    assert_eq!(unsupported.code(), SvgReadErrorCode::MissingFontContext);
}

#[test]
fn visibility_can_be_overridden_and_zero_geometry_is_not_recorded() {
    let document = decode(
        r#"<svg width="10" height="10" visibility="hidden">
          <rect width="0" height="4"/>
          <g visibility="visible"><rect width="2" height="3"/></g>
          <circle r="2"/>
        </svg>"#,
    );
    assert_eq!(document.display_list().commands().len(), 1);
    assert!(matches!(
        document.display_list().commands()[0],
        DrawCommand::FillPath { .. }
    ));
}

#[test]
fn display_list_and_path_limits_are_enforced() {
    let options = SvgReadOptions {
        limits: SvgReadLimits {
            max_display_list_items: 1,
            max_path_verbs: 2,
            ..SvgReadLimits::default()
        },
    };
    let error = SvgReader::decode(
        br#"<svg width="10" height="10"><path d="M0 0 L1 1 L2 2"/></svg>"#,
        options,
    )
    .expect_err("path ceiling");
    assert_eq!(error.code(), SvgReadErrorCode::ResourceLimit);
}
