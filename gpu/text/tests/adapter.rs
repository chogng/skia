use skia_core::{
    Color, FontCollection, FontCollectionLimits, FontFace, FontId, Paint, Point, Scalar,
    TextLayoutOptions,
};
use skia_gpu::{GpuBackend, GpuCommand, GpuCommandEncoder, GpuSurfaceDescriptor};
use skia_gpu_text::{GpuTextAtlasBuilder, GpuTextGlyphKey};

#[test]
fn text_adapter_shapes_packs_registers_and_replays_layout_glyphs() {
    let face = FontFace::from_bytes(FontId::new(91), toy_font('A')).expect("load toy font");
    let glyph = face
        .glyph_for_character('A')
        .expect("lookup glyph")
        .expect("covered glyph");
    let bitmap = face
        .rasterize_glyph(glyph, 12 << 16)
        .expect("rasterize")
        .expect("outline bitmap");
    let key = GpuTextGlyphKey::from_bitmap(&bitmap);
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    fonts.add_face(face).expect("register face");
    let layout = fonts
        .layout_text(
            "AA",
            12 << 16,
            TextLayoutOptions::new(32 << 16).expect("layout options"),
        )
        .expect("layout text");

    let mut builder = GpuTextAtlasBuilder::new(32, 32, 4).expect("atlas builder");
    builder
        .insert_layout(&layout, &fonts)
        .expect("pack layout glyphs");
    let atlas = builder.finish().expect("finish atlas");
    assert!(atlas.entry(key).is_some());

    let mut encoder = GpuCommandEncoder::new(2).expect("encoder");
    encoder.clear(Color::TRANSPARENT).expect("clear");
    let mut registration = atlas.register(&mut encoder).expect("register atlas");
    registration
        .draw_layout(
            &layout,
            Point::new(Scalar::ZERO, Scalar::ZERO),
            Paint::new(Color::rgba(20, 40, 200, 255)),
        )
        .expect("record text layout");
    drop(registration);
    let commands = encoder.finish();
    assert!(matches!(
        commands.commands()[1],
        GpuCommand::DrawGlyphs { ref glyphs, .. } if glyphs.len() == 2
    ));

    let mut backend = skia_gpu::software::SoftwareGpuBackend::default();
    let mut surface = backend
        .create_surface(GpuSurfaceDescriptor::new(32, 16).expect("surface descriptor"))
        .expect("surface");
    backend.submit(&mut surface, &commands).expect("replay");
    assert!(
        surface
            .pixels()
            .chunks_exact(4)
            .any(|pixel| pixel[2] > 0 && pixel[3] > 0)
    );
}

fn toy_font(character: char) -> Vec<u8> {
    build_font_from_tables(vec![
        (*b"cmap", cmap_table(character)),
        (*b"glyf", glyf_table()),
        (*b"head", head_table()),
        (*b"hhea", hhea_table()),
        (*b"hmtx", hmtx_table()),
        (*b"loca", loca_table()),
        (*b"maxp", maxp_table()),
    ])
}

fn build_font_from_tables(mut tables: Vec<([u8; 4], Vec<u8>)>) -> Vec<u8> {
    tables.sort_unstable_by_key(|(tag, _)| *tag);
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
        put_u32(&mut font, record + 8, offset as u32);
        put_u32(&mut font, record + 12, data.len() as u32);
        font.extend_from_slice(data);
        offset += data.len();
        while !offset.is_multiple_of(4) {
            font.push(0);
            offset += 1;
        }
    }
    font
}

fn cmap_table(character: char) -> Vec<u8> {
    let character = u16::try_from(u32::from(character)).expect("BMP character");
    let mut table = Vec::new();
    for value in [0, 1, 3, 1] {
        push_u16(&mut table, value);
    }
    push_u32(&mut table, 12);
    for value in [4, 32, 0, 4, 4, 1, 0, character, 0xffff, 0] {
        push_u16(&mut table, value);
    }
    for value in [character, 0xffff, 1_u16.wrapping_sub(character), 1, 0, 0] {
        push_u16(&mut table, value);
    }
    table
}

fn glyf_table() -> Vec<u8> {
    let mut table = Vec::new();
    for value in [1, 0, 0, 500, 700] {
        push_i16(&mut table, value);
    }
    push_u16(&mut table, 3);
    push_u16(&mut table, 0);
    table.extend([1, 1, 1, 1]);
    for delta in [0, 500, 0, -500, 0, 0, 700, 0] {
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
    for value in [600, 0, 600, 0] {
        push_u16(&mut table, value);
    }
    table
}

fn loca_table() -> Vec<u8> {
    let mut table = Vec::new();
    for value in [0, 0, 17] {
        push_u16(&mut table, value);
    }
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
