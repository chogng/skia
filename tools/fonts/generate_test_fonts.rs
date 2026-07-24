use std::{
    env, fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy)]
enum FontSlant {
    Normal,
    Italic,
    Oblique,
}

#[derive(Clone, Copy)]
struct FontStyle {
    weight: u16,
    width: u16,
    slant: FontSlant,
}

impl FontStyle {
    const NORMAL: Self = Self::new(400, 5, FontSlant::Normal);

    const fn new(weight: u16, width: u16, slant: FontSlant) -> Self {
        Self {
            weight,
            width,
            slant,
        }
    }
}

fn main() {
    let output = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("skia-rs/text/tests/fonts/synthetic"));
    fs::create_dir_all(&output).expect("create synthetic font directory");

    let basic_a = toy_font('A');
    let basic_b = toy_font('B');
    write_fixture(&output, "basic-a.ttf", basic_a.clone());
    write_fixture(&output, "basic-b.ttf", basic_b.clone());
    write_fixture(&output, "basic-c.ttf", toy_font('C'));
    write_fixture(&output, "basic-alef.ttf", toy_font('\u{05d0}'));
    write_fixture(&output, "basic-bet.ttf", toy_font('\u{05d1}'));
    write_fixture(&output, "basic-han-zhong.ttf", toy_font('\u{4e2d}'));
    write_fixture(&output, "basic-han-wen.ttf", toy_font('\u{6587}'));
    write_fixture(
        &output,
        "collection-ab.ttc",
        toy_font_collection(&[basic_a, basic_b]),
    );
    write_fixture(&output, "decorated-a.ttf", toy_font_with_decorations('A'));

    for (name, family, style) in [
        (
            "style-example-regular.ttf",
            "Example Sans",
            FontStyle::NORMAL,
        ),
        (
            "style-example-bold.ttf",
            "Example Sans",
            FontStyle::new(700, 5, FontSlant::Normal),
        ),
        (
            "style-example-bold-condensed.ttf",
            "Example Sans",
            FontStyle::new(700, 3, FontSlant::Normal),
        ),
        (
            "style-example-italic.ttf",
            "Example Sans",
            FontStyle::new(400, 5, FontSlant::Italic),
        ),
        (
            "style-example-medium.ttf",
            "Example Sans",
            FontStyle::new(500, 5, FontSlant::Normal),
        ),
        (
            "style-example-oblique.ttf",
            "Example Sans",
            FontStyle::new(400, 5, FontSlant::Oblique),
        ),
        ("style-other-regular.ttf", "Other Family", FontStyle::NORMAL),
    ] {
        write_fixture(&output, name, toy_styled_font(&['A'], family, style));
    }

    write_fixture(
        &output,
        "styled-small.ttf",
        toy_styled_font(
            &['-', 'A', 'a', 'e', 'h', 'i', 'n', 'o', 'p', 't', 'y'],
            "Small",
            FontStyle::NORMAL,
        ),
    );
    write_fixture(
        &output,
        "styled-large.ttf",
        toy_styled_font(
            &['-', 'A', 'a', 'e', 'h', 'i', 'n', 'o', 'p', 't', 'y'],
            "Large",
            FontStyle::NORMAL,
        ),
    );
    for (name, family, characters) in [
        ("styled-decorated.ttf", "Decorated", &[' ', 'A'][..]),
        (
            "styled-decoration-patterns.ttf",
            "Decoration Patterns",
            &['A'][..],
        ),
        ("styled-display-layout.ttf", "Display Layout", &['A'][..]),
        ("styled-span-styles.ttf", "Span Styles", &['A'][..]),
    ] {
        write_fixture(
            &output,
            name,
            toy_styled_font(characters, family, FontStyle::NORMAL),
        );
    }

    write_fixture(
        &output,
        "variable.ttf",
        toy_variable_font(&['A'], "Variable Sans"),
    );
    write_fixture(&output, "kerned-a.ttf", toy_kerned_font(&['A']));
    write_fixture(
        &output,
        "localized-layout.ttf",
        toy_localized_font(&['-', 'A', '\u{05d0}', '\u{2026}']),
    );
    write_fixture(&output, "localized-a.ttf", toy_localized_font(&['A']));
    write_fixture(
        &output,
        "ligature-carets.ttf",
        toy_ligature_font(Some(&[200, 450])),
    );
    write_fixture(&output, "ligature-atomic.ttf", toy_ligature_font(None));
    write_fixture(
        &output,
        "ligature-mismatched.ttf",
        toy_ligature_font(Some(&[300])),
    );

    for (name, characters) in [
        (
            "coverage-cpu-fallback.ttf",
            &['A', '\u{0301}', '\u{05d0}'][..],
        ),
        ("coverage-latin-space.ttf", &[' ', 'A'][..]),
        (
            "coverage-hyphenation.ttf",
            &['-', 'a', 'e', 'h', 'i', 'n', 'o', 'p', 't', 'y'][..],
        ),
        ("coverage-ellipsis.ttf", &['.', 'A', '\u{2026}'][..]),
        ("coverage-period-fallback.ttf", &['.', 'A'][..]),
        ("coverage-a-ellipsis.ttf", &['A', '\u{2026}'][..]),
        (
            "coverage-rtl-ellipsis.ttf",
            &['\u{05d0}', '\u{05d1}', '\u{05d2}', '\u{2026}'][..],
        ),
        (
            "coverage-dictionary.ttf",
            &[
                '-', 'A', 'B', 'a', 'b', '\u{0301}', '\u{05d0}', '\u{05d1}', '\u{05d2}', '\u{05d3}',
            ][..],
        ),
        ("coverage-mixed-space-alef.ttf", &[' ', 'A', '\u{05d0}'][..]),
        ("coverage-rtl-pair.ttf", &['\u{05d0}', '\u{05d1}'][..]),
        ("coverage-mixed-bidi.ttf", &['A', '\u{05d0}'][..]),
        (
            "coverage-selection.ttf",
            &['A', '\u{05d0}', '\u{05d1}', '\u{2026}', '\u{4e2d}'][..],
        ),
        (
            "coverage-spacing.ttf",
            &[' ', 'A', '\u{00a0}', '\u{0301}', '\u{2026}'][..],
        ),
        (
            "coverage-unicode-spaces.ttf",
            &['A', '\u{00a0}', '\u{2007}', '\u{202f}', '\u{3000}'][..],
        ),
        (
            "coverage-cjk.ttf",
            &[' ', '\u{0301}', '\u{3002}', '\u{4e2d}'][..],
        ),
        (
            "coverage-mixed-script.ttf",
            &['.', 'A', 'B', '\u{4e2d}'][..],
        ),
    ] {
        write_fixture(&output, name, toy_font_for(characters));
    }
}

fn write_fixture(output: &Path, name: &str, bytes: Vec<u8>) {
    fs::write(output.join(name), bytes).expect("write synthetic font fixture");
}

fn toy_font(character: char) -> Vec<u8> {
    toy_font_for(&[character])
}

fn toy_font_collection(faces: &[Vec<u8>]) -> Vec<u8> {
    assert!(!faces.is_empty());
    let directory_len = 12 + faces.len() * 4;
    let mut collection = vec![0; directory_len];
    collection[..4].copy_from_slice(b"ttcf");
    put_u32(&mut collection, 4, 0x0001_0000);
    put_u32(
        &mut collection,
        8,
        u32::try_from(faces.len()).expect("small collection"),
    );
    for (index, face) in faces.iter().enumerate() {
        while !collection.len().is_multiple_of(4) {
            collection.push(0);
        }
        let face_offset = collection.len();
        let mut face = face.clone();
        let table_count = usize::from(read_u16(&face, 4));
        for table_index in 0..table_count {
            let record = 12 + table_index * 16;
            let table_offset =
                usize::try_from(read_u32(&face, record + 8)).expect("small table offset");
            put_u32(
                &mut face,
                record + 8,
                u32::try_from(face_offset + table_offset).expect("small collection"),
            );
        }
        put_u32(
            &mut collection,
            12 + index * 4,
            u32::try_from(face_offset).expect("small collection"),
        );
        collection.extend_from_slice(&face);
    }
    collection
}

fn toy_font_with_decorations(character: char) -> Vec<u8> {
    build_font_from_tables(vec![
        (*b"cmap", cmap_table(&[character])),
        (*b"glyf", glyf_table()),
        (*b"head", head_table()),
        (*b"hhea", hhea_table()),
        (*b"hmtx", hmtx_table()),
        (*b"loca", loca_table()),
        (*b"maxp", maxp_table()),
        (*b"OS/2", os2_table(FontStyle::NORMAL)),
        (*b"post", post_table()),
    ])
}

fn toy_font_for(characters: &[char]) -> Vec<u8> {
    build_toy_font(characters, None, false, false)
}

fn toy_styled_font(characters: &[char], family: &str, style: FontStyle) -> Vec<u8> {
    build_toy_font(characters, Some((family, style)), false, false)
}

fn toy_variable_font(characters: &[char], family: &str) -> Vec<u8> {
    build_toy_font(characters, Some((family, FontStyle::NORMAL)), true, false)
}

fn toy_kerned_font(characters: &[char]) -> Vec<u8> {
    build_toy_font(characters, None, false, true)
}

fn toy_localized_font(characters: &[char]) -> Vec<u8> {
    let outline = glyf_table();
    let mut glyf = outline.clone();
    glyf.extend_from_slice(&outline);
    let mut hhea = hhea_table();
    put_u16(&mut hhea, 34, 3);
    let mut hmtx = hmtx_table();
    push_u16(&mut hmtx, 600);
    push_i16(&mut hmtx, 0);
    let mut loca = loca_table();
    push_u16(&mut loca, 34);
    let mut maxp = maxp_table();
    put_u16(&mut maxp, 4, 3);
    build_font_from_tables(vec![
        (*b"cmap", cmap_table(characters)),
        (*b"glyf", glyf),
        (*b"GSUB", localized_gsub_table()),
        (*b"head", head_table()),
        (*b"hhea", hhea),
        (*b"hmtx", hmtx),
        (*b"loca", loca),
        (*b"maxp", maxp),
    ])
}

fn toy_ligature_font(caret_coordinates: Option<&[i16]>) -> Vec<u8> {
    let outline = glyf_table();
    let mut glyf = outline.clone();
    glyf.extend_from_slice(&outline);
    let mut hhea = hhea_table();
    put_u16(&mut hhea, 34, 3);
    let mut hmtx = hmtx_table();
    push_u16(&mut hmtx, 600);
    push_i16(&mut hmtx, 0);
    let mut loca = loca_table();
    push_u16(&mut loca, 34);
    let mut maxp = maxp_table();
    put_u16(&mut maxp, 4, 3);
    let mut tables = vec![
        (*b"cmap", cmap_table(&['f', 'i', 'ا', 'ب', 'ج'])),
        (*b"glyf", glyf),
        (*b"GSUB", ligature_gsub_table()),
        (*b"head", head_table()),
        (*b"hhea", hhea),
        (*b"hmtx", hmtx),
        (*b"loca", loca),
        (*b"maxp", maxp),
    ];
    if let Some(coordinates) = caret_coordinates {
        tables.push((*b"GDEF", ligature_gdef_table(coordinates)));
    }
    build_font_from_tables(tables)
}

fn build_toy_font(
    characters: &[char],
    metadata: Option<(&str, FontStyle)>,
    variable: bool,
    kerned: bool,
) -> Vec<u8> {
    let mut tables = vec![
        (*b"cmap", cmap_table(characters)),
        (*b"glyf", glyf_table()),
        (*b"head", head_table()),
        (*b"hhea", hhea_table()),
        (*b"hmtx", hmtx_table()),
        (*b"loca", loca_table()),
        (*b"maxp", maxp_table()),
    ];
    if let Some((family, style)) = metadata {
        tables.push((*b"name", name_table(family)));
        tables.push((*b"OS/2", os2_table(style)));
        tables.push((*b"post", post_table()));
    }
    if variable {
        tables.push((*b"fvar", fvar_table()));
    }
    if kerned {
        tables.push((*b"kern", kern_table()));
    }
    build_font_from_tables(tables)
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

fn localized_gsub_table() -> Vec<u8> {
    let mut script_list = Vec::new();
    push_u16(&mut script_list, 1);
    script_list.extend_from_slice(b"latn");
    push_u16(&mut script_list, 8);
    push_u16(&mut script_list, 0);
    push_u16(&mut script_list, 1);
    script_list.extend_from_slice(b"SRB ");
    push_u16(&mut script_list, 10);
    push_u16(&mut script_list, 0);
    push_u16(&mut script_list, 0xffff);
    push_u16(&mut script_list, 1);
    push_u16(&mut script_list, 0);

    let mut feature_list = Vec::new();
    push_u16(&mut feature_list, 1);
    feature_list.extend_from_slice(b"locl");
    push_u16(&mut feature_list, 8);
    push_u16(&mut feature_list, 0);
    push_u16(&mut feature_list, 1);
    push_u16(&mut feature_list, 0);

    let mut lookup_list = Vec::new();
    push_u16(&mut lookup_list, 1);
    push_u16(&mut lookup_list, 4);
    push_u16(&mut lookup_list, 1);
    push_u16(&mut lookup_list, 0);
    push_u16(&mut lookup_list, 1);
    push_u16(&mut lookup_list, 8);
    push_u16(&mut lookup_list, 1);
    push_u16(&mut lookup_list, 6);
    push_i16(&mut lookup_list, 1);
    push_u16(&mut lookup_list, 1);
    push_u16(&mut lookup_list, 1);
    push_u16(&mut lookup_list, 1);

    let script_offset = 10_u16;
    let feature_offset =
        script_offset + u16::try_from(script_list.len()).expect("small script list");
    let lookup_offset =
        feature_offset + u16::try_from(feature_list.len()).expect("small feature list");
    let mut table = Vec::new();
    push_u32(&mut table, 0x0001_0000);
    push_u16(&mut table, script_offset);
    push_u16(&mut table, feature_offset);
    push_u16(&mut table, lookup_offset);
    table.extend(script_list);
    table.extend(feature_list);
    table.extend(lookup_list);
    table
}

fn ligature_gsub_table() -> Vec<u8> {
    let mut script_list = Vec::new();
    push_u16(&mut script_list, 2);
    script_list.extend_from_slice(b"arab");
    push_u16(&mut script_list, 14);
    script_list.extend_from_slice(b"latn");
    push_u16(&mut script_list, 26);
    for _ in 0..2 {
        push_u16(&mut script_list, 4);
        push_u16(&mut script_list, 0);
        push_u16(&mut script_list, 0);
        push_u16(&mut script_list, 0xffff);
        push_u16(&mut script_list, 1);
        push_u16(&mut script_list, 0);
    }

    let mut feature_list = Vec::new();
    push_u16(&mut feature_list, 1);
    feature_list.extend_from_slice(b"liga");
    push_u16(&mut feature_list, 8);
    push_u16(&mut feature_list, 0);
    push_u16(&mut feature_list, 1);
    push_u16(&mut feature_list, 0);

    let mut substitution = Vec::new();
    push_u16(&mut substitution, 1);
    push_u16(&mut substitution, 8);
    push_u16(&mut substitution, 1);
    push_u16(&mut substitution, 14);
    push_u16(&mut substitution, 1);
    push_u16(&mut substitution, 1);
    push_u16(&mut substitution, 1);
    push_u16(&mut substitution, 1);
    push_u16(&mut substitution, 4);
    push_u16(&mut substitution, 2);
    push_u16(&mut substitution, 3);
    push_u16(&mut substitution, 1);
    push_u16(&mut substitution, 1);

    let mut lookup_list = Vec::new();
    push_u16(&mut lookup_list, 1);
    push_u16(&mut lookup_list, 4);
    push_u16(&mut lookup_list, 4);
    push_u16(&mut lookup_list, 0);
    push_u16(&mut lookup_list, 1);
    push_u16(&mut lookup_list, 8);
    lookup_list.extend(substitution);

    let script_offset = 10_u16;
    let feature_offset =
        script_offset + u16::try_from(script_list.len()).expect("small script list");
    let lookup_offset =
        feature_offset + u16::try_from(feature_list.len()).expect("small feature list");
    let mut table = Vec::new();
    push_u32(&mut table, 0x0001_0000);
    push_u16(&mut table, script_offset);
    push_u16(&mut table, feature_offset);
    push_u16(&mut table, lookup_offset);
    table.extend(script_list);
    table.extend(feature_list);
    table.extend(lookup_list);
    table
}

fn ligature_gdef_table(caret_coordinates: &[i16]) -> Vec<u8> {
    assert!(!caret_coordinates.is_empty());
    let caret_count = u16::try_from(caret_coordinates.len()).expect("small caret count");
    let mut table = Vec::new();
    push_u32(&mut table, 0x0001_0000);
    push_u16(&mut table, 0);
    push_u16(&mut table, 0);
    push_u16(&mut table, 12);
    push_u16(&mut table, 0);

    push_u16(&mut table, 6);
    push_u16(&mut table, 1);
    push_u16(&mut table, 12);
    push_u16(&mut table, 1);
    push_u16(&mut table, 1);
    push_u16(&mut table, 2);

    push_u16(&mut table, caret_count);
    let caret_values_offset = 2 + caret_count * 2;
    for index in 0..caret_count {
        push_u16(&mut table, caret_values_offset + index * 4);
    }
    for coordinate in caret_coordinates {
        push_u16(&mut table, 1);
        push_i16(&mut table, *coordinate);
    }
    table
}

fn name_table(family: &str) -> Vec<u8> {
    let encoded: Vec<u8> = family.encode_utf16().flat_map(u16::to_be_bytes).collect();
    let mut table = vec![0; 18];
    put_u16(&mut table, 0, 0);
    put_u16(&mut table, 2, 1);
    put_u16(&mut table, 4, 18);
    put_u16(&mut table, 6, 3);
    put_u16(&mut table, 8, 1);
    put_u16(&mut table, 10, 0x0409);
    put_u16(&mut table, 12, 16);
    put_u16(
        &mut table,
        14,
        u16::try_from(encoded.len()).expect("short family"),
    );
    put_u16(&mut table, 16, 0);
    table.extend(encoded);
    table
}

fn os2_table(style: FontStyle) -> Vec<u8> {
    let mut table = vec![0; 96];
    put_u16(&mut table, 0, 4);
    put_u16(&mut table, 4, style.weight);
    put_u16(&mut table, 6, style.width);
    put_i16(&mut table, 26, 100);
    put_i16(&mut table, 28, 300);
    let selection = match style.slant {
        FontSlant::Normal => 0,
        FontSlant::Italic => 1,
        FontSlant::Oblique => 1 << 9,
    };
    put_u16(&mut table, 62, selection);
    table
}

fn post_table() -> Vec<u8> {
    let mut table = vec![0; 32];
    put_u32(&mut table, 0, 0x0003_0000);
    put_i16(&mut table, 8, -100);
    put_i16(&mut table, 10, 100);
    table
}

fn fvar_table() -> Vec<u8> {
    let mut table = vec![0; 36];
    put_u16(&mut table, 0, 1);
    put_u16(&mut table, 2, 0);
    put_u16(&mut table, 4, 16);
    put_u16(&mut table, 8, 1);
    put_u16(&mut table, 10, 20);
    put_u16(&mut table, 12, 0);
    put_u16(&mut table, 14, 8);
    table[16..20].copy_from_slice(b"wght");
    put_u32(&mut table, 20, 100 << 16);
    put_u32(&mut table, 24, 400 << 16);
    put_u32(&mut table, 28, 900 << 16);
    put_u16(&mut table, 34, 256);
    table
}

fn kern_table() -> Vec<u8> {
    let mut table = vec![0; 24];
    put_u16(&mut table, 2, 1);
    put_u16(&mut table, 6, 20);
    put_u16(&mut table, 8, 1);
    put_u16(&mut table, 10, 1);
    put_u16(&mut table, 12, 6);
    put_u16(&mut table, 18, 1);
    put_u16(&mut table, 20, 1);
    put_i16(&mut table, 22, -100);
    table
}

fn cmap_table(characters: &[char]) -> Vec<u8> {
    let mut characters: Vec<u16> = characters
        .iter()
        .copied()
        .map(|character| {
            u16::try_from(u32::from(character)).expect("toy font supports BMP characters")
        })
        .collect();
    characters.sort_unstable();
    characters.dedup();
    assert!(!characters.is_empty());
    assert!(!characters.contains(&0xffff));
    let segment_count = u16::try_from(characters.len() + 1).expect("small segment count");
    let power = 1_u16 << segment_count.ilog2();
    let search_range = power * 2;
    let entry_selector = u16::try_from(power.ilog2()).expect("small entry selector");
    let segment_count_x2 = segment_count * 2;
    let range_shift = segment_count_x2 - search_range;
    let length = 16 + usize::from(segment_count) * 8;
    let mut table = Vec::new();
    push_u16(&mut table, 0);
    push_u16(&mut table, 1);
    push_u16(&mut table, 3);
    push_u16(&mut table, 1);
    push_u32(&mut table, 12);
    push_u16(&mut table, 4);
    push_u16(&mut table, u16::try_from(length).expect("small cmap"));
    push_u16(&mut table, 0);
    push_u16(&mut table, segment_count_x2);
    push_u16(&mut table, search_range);
    push_u16(&mut table, entry_selector);
    push_u16(&mut table, range_shift);
    for character in &characters {
        push_u16(&mut table, *character);
    }
    push_u16(&mut table, 0xffff);
    push_u16(&mut table, 0);
    for character in &characters {
        push_u16(&mut table, *character);
    }
    push_u16(&mut table, 0xffff);
    for character in &characters {
        push_u16(&mut table, 1_u16.wrapping_sub(*character));
    }
    push_i16(&mut table, 1);
    for _ in 0..segment_count {
        push_u16(&mut table, 0);
    }
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

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}
