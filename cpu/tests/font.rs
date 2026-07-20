use skia_core::{
    Color, FontFace, FontId, FontLimits, GlyphId, GlyphOutlineProvider, Paint, Scalar,
    TextErrorCode, Transform,
};
use skia_cpu::{Surface, SurfaceLimits};

#[test]
fn utf8_text_shapes_and_draws_through_the_cpu_pipeline() {
    let face = FontFace::from_bytes(FontId::new(7), toy_font()).expect("load toy font");
    let run = face.shape("AA", 10 << 16).expect("shape UTF-8 text");

    assert_eq!(run.glyphs().len(), 2);
    assert_eq!(run.glyphs()[0].cluster(), 0);
    assert_eq!(run.glyphs()[1].cluster(), 1);
    assert_eq!(run.glyphs()[0].glyph().value(), 1);
    assert_eq!(run.glyphs()[0].advance_x().bits(), 600 * 64);
    assert!(
        face.glyph_outline(face.id(), run.glyphs()[0].glyph())
            .expect("resolve outline")
            .is_some()
    );

    let mut surface = Surface::new(16, 12, SurfaceLimits::default()).expect("surface");
    let mut canvas = surface.canvas();
    canvas.set_transform(Transform::translate(scalar(2), scalar(9)));
    canvas
        .draw_glyph_run(&run, &face, Paint::new(Color::rgba(20, 40, 60, 255)))
        .expect("draw shaped text");
    drop(canvas);

    assert_eq!(pixel(&surface, 3, 4), [20, 40, 60, 255]);
    assert_eq!(pixel(&surface, 9, 4), [20, 40, 60, 255]);
    assert_eq!(pixel(&surface, 15, 11), [0, 0, 0, 0]);
}

#[test]
fn public_font_loader_rejects_malformed_data() {
    let error = FontFace::from_bytes(FontId::new(1), b"not a font".to_vec())
        .expect_err("malformed font must fail");
    assert_eq!(error.code(), TextErrorCode::InvalidFontData);
}

#[test]
fn font_limits_bound_shaping_and_outline_work() {
    let shaping_limits = FontLimits::new(1_024, 8, 1, 32).expect("valid limits");
    let face = FontFace::from_bytes_with_limits(FontId::new(2), toy_font(), 0, shaping_limits)
        .expect("load bounded font");
    assert_eq!(
        face.shape("AA", 10 << 16)
            .expect_err("two glyphs exceed the run limit")
            .code(),
        TextErrorCode::ResourceLimit
    );

    let outline_limits = FontLimits::new(1_024, 8, 8, 2).expect("valid limits");
    let face = FontFace::from_bytes_with_limits(FontId::new(3), toy_font(), 0, outline_limits)
        .expect("load bounded font");
    assert_eq!(
        face.glyph_outline(face.id(), GlyphId::new(1))
            .expect_err("square outline exceeds two segments")
            .code(),
        TextErrorCode::ResourceLimit
    );

    assert_eq!(
        FontFace::from_bytes_with_limits(FontId::new(4), toy_font(), 1, FontLimits::default())
            .expect_err("standalone font has no second face")
            .code(),
        TextErrorCode::InvalidFaceIndex
    );
}

fn scalar(value: i32) -> Scalar {
    Scalar::from_i32(value).expect("small scalar")
}

fn pixel(surface: &Surface, x: usize, y: usize) -> [u8; 4] {
    let offset = (y * surface.width() as usize + x) * 4;
    surface.pixels()[offset..offset + 4]
        .try_into()
        .expect("RGBA pixel")
}

fn toy_font() -> Vec<u8> {
    let tables = [
        (*b"cmap", cmap_table()),
        (*b"glyf", glyf_table()),
        (*b"head", head_table()),
        (*b"hhea", hhea_table()),
        (*b"hmtx", hmtx_table()),
        (*b"loca", loca_table()),
        (*b"maxp", maxp_table()),
    ];
    let table_count = u16::try_from(tables.len()).expect("small table count");
    let directory_len = 12 + tables.len() * 16;
    let mut font = vec![0; directory_len];
    put_u32(&mut font, 0, 0x0001_0000);
    put_u16(&mut font, 4, table_count);
    put_u16(&mut font, 6, 64);
    put_u16(&mut font, 8, 2);
    put_u16(&mut font, 10, 48);

    let mut offset = directory_len;
    for (index, (tag, data)) in tables.iter().enumerate() {
        let record = 12 + index * 16;
        font[record..record + 4].copy_from_slice(tag);
        put_u32(
            &mut font,
            record + 8,
            u32::try_from(offset).expect("small font"),
        );
        put_u32(
            &mut font,
            record + 12,
            u32::try_from(data.len()).expect("small table"),
        );
        font.extend_from_slice(data);
        offset += data.len();
        while !offset.is_multiple_of(4) {
            font.push(0);
            offset += 1;
        }
    }
    font
}

fn cmap_table() -> Vec<u8> {
    let mut table = Vec::new();
    push_u16(&mut table, 0);
    push_u16(&mut table, 1);
    push_u16(&mut table, 3);
    push_u16(&mut table, 1);
    push_u32(&mut table, 12);
    push_u16(&mut table, 4);
    push_u16(&mut table, 32);
    push_u16(&mut table, 0);
    push_u16(&mut table, 4);
    push_u16(&mut table, 4);
    push_u16(&mut table, 1);
    push_u16(&mut table, 0);
    push_u16(&mut table, 65);
    push_u16(&mut table, 0xffff);
    push_u16(&mut table, 0);
    push_u16(&mut table, 65);
    push_u16(&mut table, 0xffff);
    push_i16(&mut table, -64);
    push_i16(&mut table, 1);
    push_u16(&mut table, 0);
    push_u16(&mut table, 0);
    table
}

fn glyf_table() -> Vec<u8> {
    let mut table = Vec::new();
    push_i16(&mut table, 1);
    push_i16(&mut table, 0);
    push_i16(&mut table, 0);
    push_i16(&mut table, 500);
    push_i16(&mut table, 700);
    push_u16(&mut table, 3);
    push_u16(&mut table, 0);
    table.extend([1, 1, 1, 1]);
    for delta in [0, 500, 0, -500] {
        push_i16(&mut table, delta);
    }
    for delta in [0, 0, 700, 0] {
        push_i16(&mut table, delta);
    }
    table
}

fn head_table() -> Vec<u8> {
    let mut table = vec![0; 54];
    put_u32(&mut table, 0, 0x0001_0000);
    put_u32(&mut table, 12, 0x5f0f_3cf5);
    put_u16(&mut table, 18, 1_000);
    put_u16(&mut table, 46, 8);
    put_u16(&mut table, 50, 0);
    table
}

fn hhea_table() -> Vec<u8> {
    let mut table = vec![0; 36];
    put_u32(&mut table, 0, 0x0001_0000);
    put_i16(&mut table, 4, 800);
    put_i16(&mut table, 6, -200);
    put_u16(&mut table, 10, 600);
    put_i16(&mut table, 18, 1);
    put_u16(&mut table, 34, 2);
    table
}

fn hmtx_table() -> Vec<u8> {
    let mut table = Vec::new();
    push_u16(&mut table, 600);
    push_i16(&mut table, 0);
    push_u16(&mut table, 600);
    push_i16(&mut table, 0);
    table
}

fn loca_table() -> Vec<u8> {
    let mut table = Vec::new();
    push_u16(&mut table, 0);
    push_u16(&mut table, 0);
    push_u16(&mut table, 17);
    table
}

fn maxp_table() -> Vec<u8> {
    let mut table = vec![0; 32];
    put_u32(&mut table, 0, 0x0001_0000);
    put_u16(&mut table, 4, 2);
    table
}

fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend(value.to_be_bytes());
}

fn push_i16(bytes: &mut Vec<u8>, value: i16) {
    bytes.extend(value.to_be_bytes());
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend(value.to_be_bytes());
}

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn put_i16(bytes: &mut [u8], offset: usize, value: i16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}
