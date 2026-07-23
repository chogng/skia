use std::{fs, path::PathBuf};

use super::{
    GenericFontFamily, SystemFontDiscoveryLimits, SystemFontErrorCode, discover_roots,
    generic_candidates, is_font_path,
};

#[test]
fn recognizes_supported_font_extensions_case_insensitively() {
    assert!(is_font_path(&PathBuf::from("face.TTC")));
    assert!(is_font_path(&PathBuf::from("face.otf")));
    assert!(!is_font_path(&PathBuf::from("face.woff2")));
}

#[test]
fn generic_candidates_cover_every_public_family() {
    for family in [
        GenericFontFamily::Serif,
        GenericFontFamily::SansSerif,
        GenericFontFamily::Monospace,
        GenericFontFamily::SystemUi,
        GenericFontFamily::Cursive,
        GenericFontFamily::Fantasy,
        GenericFontFamily::Emoji,
        GenericFontFamily::Math,
    ] {
        assert!(!generic_candidates(family).is_empty(), "{family:?}");
    }
}

#[test]
fn discovery_skips_invalid_font_files_and_bounds_traversal() {
    let root = std::env::temp_dir().join(format!("skia-system-font-test-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("nested")).expect("directories");
    fs::write(root.join("invalid.ttf"), b"not a font").expect("invalid font");
    fs::write(root.join("nested/ignored.txt"), b"not scanned").expect("text file");
    let catalog = discover_roots(
        std::slice::from_ref(&root),
        SystemFontDiscoveryLimits::default(),
    )
    .expect("discover invalid font");
    assert!(catalog.records().is_empty());
    assert_eq!(catalog.scanned_files(), 1);
    assert_eq!(catalog.skipped_files(), 1);

    let error = discover_roots(
        std::slice::from_ref(&root),
        SystemFontDiscoveryLimits::new(1, 8, 8, 1024).expect("limits"),
    )
    .expect_err("directory ceiling");
    assert_eq!(error.code(), SystemFontErrorCode::ResourceLimit);
    fs::remove_dir_all(root).expect("cleanup");
}

#[cfg(target_os = "macos")]
#[test]
fn discovers_and_reloads_a_real_platform_face() {
    let catalog = super::discover_system_fonts(&[], SystemFontDiscoveryLimits::default())
        .expect("system font catalog");
    let record = catalog
        .match_generic(GenericFontFamily::SystemUi, skia_text::FontStyle::NORMAL)
        .or_else(|| catalog.records().first())
        .expect("at least one system font");
    let face = record
        .load(skia_text::FontLimits::default())
        .expect("reload discovered face");
    assert_eq!(face.id(), record.id());
    assert_eq!(face.face_index(), record.face_index());
}
