use std::collections::BTreeMap;

use skia_core::{Color, DisplayList, DisplayListBuilder, Paint, Rect, Scalar};
use skia_image::Image;

use super::{
    RasterFallback, UnsupportedBehavior, XpsDocument, XpsErrorCode, XpsFormat, XpsOptions,
    XpsPageSize, XpsPageSpec,
};

fn scalar(value: i32) -> Scalar {
    Scalar::from_i32(value).expect("scalar")
}

fn rect(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
    Rect::new(scalar(left), scalar(top), scalar(right), scalar(bottom)).expect("rect")
}

fn page_spec() -> XpsPageSpec {
    XpsPageSpec::new(XpsPageSize::new(scalar(96), scalar(64)).expect("page size"))
}

fn vector_list() -> DisplayList {
    let mut builder = DisplayListBuilder::new(8).expect("builder");
    builder
        .fill_rect(rect(4, 5, 30, 20), Paint::new(Color::rgba(10, 20, 30, 128)))
        .expect("fill");
    builder.finish()
}

fn write_document(format: XpsFormat, list: &DisplayList) -> Vec<u8> {
    let options = XpsOptions {
        format,
        ..XpsOptions::default()
    };
    let mut document = XpsDocument::new(Vec::new(), options).expect("document");
    document
        .add_page(page_spec(), list)
        .expect("completed page");
    document.finish().expect("package")
}

#[test]
fn both_dialects_share_structure_and_switch_fixed_payload_namespaces() {
    let list = vector_list();
    let xps = zip_entries(&write_document(XpsFormat::Xps10, &list));
    let open_xps = zip_entries(&write_document(XpsFormat::OpenXps, &list));

    for entries in [&xps, &open_xps] {
        assert!(entries.contains_key("[Content_Types].xml"));
        assert!(entries.contains_key("_rels/.rels"));
        assert!(entries.contains_key("FixedDocumentSequence.fdseq"));
        assert!(entries.contains_key("Documents/1/FixedDocument.fdoc"));
        assert!(entries.contains_key("Documents/1/Pages/1.fpage"));
        let relationships = text(entries, "_rels/.rels");
        assert!(relationships.contains("Target=\"FixedDocumentSequence.fdseq\""));
        assert!(!relationships.contains("Target=\"/FixedDocumentSequence.fdseq\""));
    }

    assert!(
        text(&xps, "Documents/1/Pages/1.fpage")
            .contains("http://schemas.microsoft.com/xps/2005/06")
    );
    assert!(
        text(&open_xps, "Documents/1/Pages/1.fpage")
            .contains("http://schemas.openxps.org/oxps/v1.0")
    );
    assert!(
        text(&xps, "_rels/.rels")
            .contains("http://schemas.microsoft.com/xps/2005/06/fixedrepresentation")
    );
    assert!(
        text(&open_xps, "_rels/.rels")
            .contains("http://schemas.openxps.org/oxps/v1.0/fixedrepresentation")
    );
}

#[test]
fn native_vector_output_is_deterministic_and_keeps_fixed_page_geometry() {
    let list = vector_list();
    let first = write_document(XpsFormat::OpenXps, &list);
    let second = write_document(XpsFormat::OpenXps, &list);
    assert_eq!(first, second);
    let entries = zip_entries(&first);
    let page = text(&entries, "Documents/1/Pages/1.fpage");
    assert!(page.contains("Width=\"96\" Height=\"64\""));
    assert!(page.contains("Data=\"F1M4,5L30,5L30,20L4,20Z\""));
    assert!(page.contains("Fill=\"#800A141E\""));
}

#[test]
fn page_scoped_image_resources_have_unique_package_names() {
    let image = Image::from_rgba8(1, 1, vec![255, 0, 0, 255]).expect("one-pixel image resource");
    let mut builder = DisplayListBuilder::new(4).expect("builder");
    let image = builder.add_image(image).expect("image");
    builder
        .draw_image(image, rect(0, 0, 12, 12), u8::MAX, Paint::default())
        .expect("draw image");
    let list = builder.finish();
    let mut document = XpsDocument::new(Vec::new(), XpsOptions::default()).expect("document");
    document.add_page(page_spec(), &list).expect("first page");
    document.add_page(page_spec(), &list).expect("second page");
    let entries = zip_entries(&document.finish().expect("package"));
    assert!(entries.contains_key("Documents/1/Resources/Images/1-1.png"));
    assert!(entries.contains_key("Documents/1/Resources/Images/2-1.png"));
    assert!(text(&entries, "Documents/1/Pages/1.fpage").contains("../Resources/Images/1-1.png"));
    assert!(text(&entries, "Documents/1/Pages/2.fpage").contains("../Resources/Images/2-1.png"));
}

#[test]
fn unsupported_clip_can_use_bounded_whole_page_fallback() {
    let mut builder = DisplayListBuilder::new(4).expect("builder");
    builder.clip_rect(rect(0, 0, 20, 20)).expect("clip");
    builder
        .fill_rect(rect(0, 0, 40, 40), Paint::new(Color::RED))
        .expect("fill");
    let list = builder.finish();
    let spec = page_spec()
        .with_content_box(rect(2, 3, 80, 50))
        .expect("content box");

    let mut strict = XpsDocument::new(Vec::new(), XpsOptions::default()).expect("strict");
    assert_eq!(
        strict
            .add_page(spec, &list)
            .expect_err("clip needs fallback")
            .code(),
        XpsErrorCode::Unsupported
    );

    let options = XpsOptions {
        unsupported_behavior: UnsupportedBehavior::RasterizePage,
        raster_fallback: RasterFallback {
            dpi: 96,
            max_pixels: 96 * 64,
            max_bytes: 96 * 64 * 4,
        },
        ..XpsOptions::default()
    };
    let mut fallback = XpsDocument::new(Vec::new(), options).expect("fallback");
    fallback.add_page(spec, &list).expect("rasterized page");
    let entries = zip_entries(&fallback.finish().expect("package"));
    let image = entries
        .get("Documents/1/Resources/Images/1-1.png")
        .expect("fallback PNG");
    assert_eq!(&image[..8], b"\x89PNG\r\n\x1a\n");
    let page = text(&entries, "Documents/1/Pages/1.fpage");
    assert!(page.contains("<Canvas Clip=\"F1M2,3L80,3L80,50L2,50Z\">"));
    assert!(page.contains("<ImageBrush"));
}

fn text<'a>(entries: &'a BTreeMap<String, Vec<u8>>, name: &str) -> &'a str {
    std::str::from_utf8(entries.get(name).expect("package part")).expect("XML text")
}

fn zip_entries(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut entries = BTreeMap::new();
    let mut offset = 0_usize;
    while read_u32(bytes, offset) == 0x0403_4B50 {
        let size = read_u32(bytes, offset + 18) as usize;
        let name_length = read_u16(bytes, offset + 26) as usize;
        let extra_length = read_u16(bytes, offset + 28) as usize;
        let name_start = offset + 30;
        let data_start = name_start + name_length + extra_length;
        let data_end = data_start + size;
        let name = std::str::from_utf8(&bytes[name_start..name_start + name_length])
            .expect("ZIP part name")
            .to_owned();
        entries.insert(name, bytes[data_start..data_end].to_vec());
        offset = data_end;
    }
    entries
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().expect("u16"))
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("u32"))
}
