//! Platform font-directory discovery for `skia-text`.
//!
//! This adapter owns filesystem and operating-system policy. The portable text
//! crate continues to own only immutable font bytes, shaping, and layout.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::{
    collections::{HashMap, HashSet, VecDeque},
    env, fmt, fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use skia_text::{
    FontCollection, FontCollectionLimits, FontFace, FontId, FontLimits, FontStyle, TextErrorCode,
};

/// Stable machine-readable system-font discovery failure.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SystemFontErrorCode {
    /// A configured discovery ceiling is zero or inconsistent.
    InvalidLimits,
    /// Directory enumeration or font-file reading failed.
    Io,
    /// Discovery or collection loading exceeded a resource ceiling.
    ResourceLimit,
    /// Loading a discovered face no longer produced valid font data.
    InvalidFontData,
    /// A collection rejected a duplicate stable font identifier.
    DuplicateFontId,
    /// Memory allocation failed.
    AllocationFailed,
}

/// Source-redacted system-font adapter error.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SystemFontError {
    code: SystemFontErrorCode,
}

impl SystemFontError {
    const fn new(code: SystemFontErrorCode) -> Self {
        Self { code }
    }

    /// Returns the stable failure code.
    pub const fn code(self) -> SystemFontErrorCode {
        self.code
    }
}

impl fmt::Display for SystemFontError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}", self.code)
    }
}

impl std::error::Error for SystemFontError {}

/// Directory, file, face, and byte ceilings for one discovery pass.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SystemFontDiscoveryLimits {
    max_directories: usize,
    max_files: usize,
    max_faces: usize,
    max_font_bytes: usize,
}

impl SystemFontDiscoveryLimits {
    /// Creates positive discovery ceilings.
    pub const fn new(
        max_directories: usize,
        max_files: usize,
        max_faces: usize,
        max_font_bytes: usize,
    ) -> Result<Self, SystemFontError> {
        if max_directories == 0 || max_files == 0 || max_faces == 0 || max_font_bytes == 0 {
            return Err(SystemFontError::new(SystemFontErrorCode::InvalidLimits));
        }
        Ok(Self {
            max_directories,
            max_files,
            max_faces,
            max_font_bytes,
        })
    }
}

impl Default for SystemFontDiscoveryLimits {
    fn default() -> Self {
        Self {
            max_directories: 4_096,
            max_files: 16_384,
            max_faces: 8_192,
            max_font_bytes: 64 * 1024 * 1024,
        }
    }
}

/// CSS-like generic family resolved by platform candidate lists.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GenericFontFamily {
    /// Proportional serif text.
    Serif,
    /// Proportional sans-serif text.
    SansSerif,
    /// Fixed-pitch text.
    Monospace,
    /// Platform user-interface text.
    SystemUi,
    /// Informal cursive text.
    Cursive,
    /// Decorative display text.
    Fantasy,
    /// Color or monochrome emoji glyphs.
    Emoji,
    /// Mathematical notation.
    Math,
}

/// One discovered face and its reloadable filesystem source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SystemFontRecord {
    id: FontId,
    path: PathBuf,
    face_index: u32,
    family_name: Option<String>,
    style: FontStyle,
}

impl SystemFontRecord {
    /// Returns the stable path-and-index-derived font identity.
    pub const fn id(&self) -> FontId {
        self.id
    }

    /// Borrows the source font file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the face index inside a collection font.
    pub const fn face_index(&self) -> u32 {
        self.face_index
    }

    /// Returns the preferred OpenType family name when present.
    pub fn family_name(&self) -> Option<&str> {
        self.family_name.as_deref()
    }

    /// Returns the parsed weight, width, and slant.
    pub const fn style(&self) -> FontStyle {
        self.style
    }

    /// Reloads this face using the supplied font-processing limits.
    pub fn load(&self, limits: FontLimits) -> Result<FontFace, SystemFontError> {
        let bytes = read_font(&self.path, limits.max_font_bytes())?;
        FontFace::from_shared_bytes_with_limits(self.id, bytes, self.face_index, limits)
            .map_err(map_text_error)
    }
}

/// Deterministically ordered snapshot of discovered system font faces.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SystemFontCatalog {
    records: Vec<SystemFontRecord>,
    scanned_files: usize,
    skipped_files: usize,
}

impl SystemFontCatalog {
    /// Borrows faces in platform-directory and lexical path order.
    pub fn records(&self) -> &[SystemFontRecord] {
        &self.records
    }

    /// Returns the number of font-extension files inspected.
    pub const fn scanned_files(&self) -> usize {
        self.scanned_files
    }

    /// Returns files skipped because they were oversized or not parseable fonts.
    pub const fn skipped_files(&self) -> usize {
        self.skipped_files
    }

    /// Selects the closest style in one named family.
    pub fn match_family(&self, family: &str, style: FontStyle) -> Option<&SystemFontRecord> {
        self.records
            .iter()
            .enumerate()
            .filter(|(_, record)| {
                record
                    .family_name()
                    .is_some_and(|name| name.eq_ignore_ascii_case(family))
            })
            .min_by_key(|(index, record)| (record.style().match_rank(style), *index))
            .map(|(_, record)| record)
    }

    /// Resolves one generic family through platform-preferred concrete names.
    pub fn match_generic(
        &self,
        family: GenericFontFamily,
        style: FontStyle,
    ) -> Option<&SystemFontRecord> {
        generic_candidates(family)
            .iter()
            .find_map(|candidate| self.match_family(candidate, style))
    }

    /// Resolves language-preferred families before falling back to a generic family.
    pub fn match_language(
        &self,
        language: &str,
        fallback: GenericFontFamily,
        style: FontStyle,
    ) -> Option<&SystemFontRecord> {
        language_candidates(language)
            .iter()
            .find_map(|candidate| self.match_family(candidate, style))
            .or_else(|| self.match_generic(fallback, style))
    }

    /// Loads every discovered record into a portable ordered collection.
    ///
    /// Font files are cached by path during this operation so indexed TTC/OTC
    /// faces share one immutable byte allocation.
    pub fn load_collection(
        &self,
        collection_limits: FontCollectionLimits,
        font_limits: FontLimits,
    ) -> Result<FontCollection, SystemFontError> {
        let mut collection = FontCollection::new(collection_limits);
        let mut files: HashMap<&Path, Arc<[u8]>> = HashMap::new();
        for record in &self.records {
            let bytes = if let Some(bytes) = files.get(record.path()) {
                Arc::clone(bytes)
            } else {
                let bytes = read_font(record.path(), font_limits.max_font_bytes())?;
                files
                    .try_reserve(1)
                    .map_err(|_| SystemFontError::new(SystemFontErrorCode::AllocationFailed))?;
                files.insert(record.path(), Arc::clone(&bytes));
                bytes
            };
            let face = FontFace::from_shared_bytes_with_limits(
                record.id,
                bytes,
                record.face_index,
                font_limits,
            )
            .map_err(map_text_error)?;
            collection.add_face(face).map_err(map_text_error)?;
        }
        Ok(collection)
    }
}

/// Discovers fonts in platform system/user directories plus explicit roots.
///
/// Explicit roots are appended after platform roots. Missing roots are ignored;
/// existing but unreadable directories or files return [`SystemFontErrorCode::Io`].
pub fn discover_system_fonts(
    additional_roots: &[PathBuf],
    limits: SystemFontDiscoveryLimits,
) -> Result<SystemFontCatalog, SystemFontError> {
    let mut roots = platform_font_roots();
    roots
        .try_reserve(additional_roots.len())
        .map_err(|_| SystemFontError::new(SystemFontErrorCode::AllocationFailed))?;
    roots.extend_from_slice(additional_roots);
    roots.dedup();
    discover_roots(&roots, limits)
}

fn discover_roots(
    roots: &[PathBuf],
    limits: SystemFontDiscoveryLimits,
) -> Result<SystemFontCatalog, SystemFontError> {
    let mut queue = VecDeque::new();
    for root in roots.iter().filter(|root| root.is_dir()) {
        queue.push_back(root.clone());
    }
    let mut seen_directories = HashSet::new();
    let mut seen_files = HashSet::new();
    let mut used_ids = HashSet::new();
    let mut catalog = SystemFontCatalog::default();
    while let Some(directory) = queue.pop_front() {
        if !seen_directories.insert(directory.clone()) {
            continue;
        }
        if seen_directories.len() > limits.max_directories {
            return Err(SystemFontError::new(SystemFontErrorCode::ResourceLimit));
        }
        let mut entries: Vec<_> = fs::read_dir(&directory)
            .map_err(|_| SystemFontError::new(SystemFontErrorCode::Io))?
            .collect::<Result<_, _>>()
            .map_err(|_| SystemFontError::new(SystemFontErrorCode::Io))?;
        entries.sort_unstable_by_key(|entry| entry.path());
        for entry in entries {
            let file_type = entry
                .file_type()
                .map_err(|_| SystemFontError::new(SystemFontErrorCode::Io))?;
            let path = entry.path();
            if file_type.is_dir() {
                queue.push_back(path);
            } else if file_type.is_file() && is_font_path(&path) && seen_files.insert(path.clone())
            {
                catalog.scanned_files = catalog
                    .scanned_files
                    .checked_add(1)
                    .ok_or(SystemFontError::new(SystemFontErrorCode::ResourceLimit))?;
                if catalog.scanned_files > limits.max_files {
                    return Err(SystemFontError::new(SystemFontErrorCode::ResourceLimit));
                }
                discover_file(&path, limits, &mut used_ids, &mut catalog)?;
            }
        }
    }
    Ok(catalog)
}

fn discover_file(
    path: &Path,
    limits: SystemFontDiscoveryLimits,
    used_ids: &mut HashSet<FontId>,
    catalog: &mut SystemFontCatalog,
) -> Result<(), SystemFontError> {
    let metadata = fs::metadata(path).map_err(|_| SystemFontError::new(SystemFontErrorCode::Io))?;
    if usize::try_from(metadata.len()).map_or(true, |length| length > limits.max_font_bytes) {
        catalog.skipped_files += 1;
        return Ok(());
    }
    let bytes = read_font(path, limits.max_font_bytes)?;
    let mut face_index = 0_u32;
    let mut added = false;
    loop {
        let id = unique_font_id(path, face_index, used_ids)?;
        match FontFace::from_shared_bytes_with_limits(
            id,
            Arc::clone(&bytes),
            face_index,
            FontLimits::default(),
        ) {
            Ok(face) => {
                if catalog.records.len() == limits.max_faces {
                    return Err(SystemFontError::new(SystemFontErrorCode::ResourceLimit));
                }
                catalog
                    .records
                    .try_reserve(1)
                    .map_err(|_| SystemFontError::new(SystemFontErrorCode::AllocationFailed))?;
                catalog.records.push(SystemFontRecord {
                    id,
                    path: path.to_path_buf(),
                    face_index,
                    family_name: face.family_name().map(str::to_owned),
                    style: face.style(),
                });
                used_ids.insert(id);
                added = true;
            }
            Err(error) if error.code() == TextErrorCode::InvalidFaceIndex && face_index > 0 => {
                break;
            }
            Err(_) if face_index == 0 => break,
            Err(_) => break,
        }
        face_index = face_index
            .checked_add(1)
            .ok_or(SystemFontError::new(SystemFontErrorCode::ResourceLimit))?;
    }
    if !added {
        catalog.skipped_files += 1;
    }
    Ok(())
}

fn read_font(path: &Path, max_bytes: usize) -> Result<Arc<[u8]>, SystemFontError> {
    let bytes = fs::read(path).map_err(|_| SystemFontError::new(SystemFontErrorCode::Io))?;
    if bytes.len() > max_bytes {
        return Err(SystemFontError::new(SystemFontErrorCode::ResourceLimit));
    }
    Ok(bytes.into())
}

fn unique_font_id(
    path: &Path,
    face_index: u32,
    used: &HashSet<FontId>,
) -> Result<FontId, SystemFontError> {
    let mut value = 0xcbf2_9ce4_8422_2325_u64;
    for byte in path.to_string_lossy().as_bytes() {
        value ^= u64::from(*byte);
        value = value.wrapping_mul(0x0000_0100_0000_01b3);
    }
    for byte in face_index.to_le_bytes() {
        value ^= u64::from(byte);
        value = value.wrapping_mul(0x0000_0100_0000_01b3);
    }
    for attempt in 0_u64..=u16::MAX.into() {
        let id = FontId::new(value.wrapping_add(attempt));
        if !used.contains(&id) {
            return Ok(id);
        }
    }
    Err(SystemFontError::new(SystemFontErrorCode::ResourceLimit))
}

fn is_font_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "ttf" | "otf" | "ttc" | "otc"
            )
        })
}

fn map_text_error(error: skia_text::TextError) -> SystemFontError {
    let code = match error.code() {
        TextErrorCode::ResourceLimit => SystemFontErrorCode::ResourceLimit,
        TextErrorCode::DuplicateFontId => SystemFontErrorCode::DuplicateFontId,
        TextErrorCode::AllocationFailed => SystemFontErrorCode::AllocationFailed,
        _ => SystemFontErrorCode::InvalidFontData,
    };
    SystemFontError::new(code)
}

fn language_candidates(language: &str) -> &'static [&'static str] {
    let normalized = language.to_ascii_lowercase();
    if normalized.starts_with("zh-hant") || normalized.starts_with("zh-tw") {
        &[
            "PingFang TC",
            "Heiti TC",
            "Microsoft JhengHei",
            "Noto Sans CJK TC",
        ]
    } else if normalized.starts_with("zh") {
        &[
            "PingFang SC",
            "Heiti SC",
            "Microsoft YaHei",
            "Noto Sans CJK SC",
        ]
    } else if normalized.starts_with("ja") {
        &["Hiragino Sans", "Yu Gothic", "Noto Sans CJK JP"]
    } else if normalized.starts_with("ko") {
        &["Apple SD Gothic Neo", "Malgun Gothic", "Noto Sans CJK KR"]
    } else if normalized.starts_with("ar") {
        &["Geeza Pro", "Segoe UI", "Noto Sans Arabic"]
    } else if normalized.starts_with("he") {
        &["Arial Hebrew", "Segoe UI", "Noto Sans Hebrew"]
    } else if normalized.starts_with("hi") {
        &["Kohinoor Devanagari", "Nirmala UI", "Noto Sans Devanagari"]
    } else if normalized.starts_with("th") {
        &["Thonburi", "Leelawadee UI", "Noto Sans Thai"]
    } else {
        &[]
    }
}

#[cfg(target_os = "macos")]
fn generic_candidates(family: GenericFontFamily) -> &'static [&'static str] {
    match family {
        GenericFontFamily::Serif => &["New York", "Times New Roman", "Times"],
        GenericFontFamily::SansSerif => &["Helvetica Neue", "Arial"],
        GenericFontFamily::Monospace => &["SF Mono", "Menlo", "Monaco", "Courier"],
        GenericFontFamily::SystemUi => &["SF Pro Display", "SF Pro Text", "Helvetica Neue"],
        GenericFontFamily::Cursive => &["Apple Chancery", "Snell Roundhand"],
        GenericFontFamily::Fantasy => &["Papyrus", "Copperplate"],
        GenericFontFamily::Emoji => &["Apple Color Emoji"],
        GenericFontFamily::Math => &["STIX Two Math", "STIXGeneral", "Times New Roman"],
    }
}

#[cfg(target_os = "windows")]
fn generic_candidates(family: GenericFontFamily) -> &'static [&'static str] {
    match family {
        GenericFontFamily::Serif => &["Times New Roman", "Cambria"],
        GenericFontFamily::SansSerif => &["Arial", "Segoe UI"],
        GenericFontFamily::Monospace => &["Cascadia Mono", "Consolas", "Courier New"],
        GenericFontFamily::SystemUi => &["Segoe UI"],
        GenericFontFamily::Cursive => &["Comic Sans MS", "Segoe Script"],
        GenericFontFamily::Fantasy => &["Impact"],
        GenericFontFamily::Emoji => &["Segoe UI Emoji"],
        GenericFontFamily::Math => &["Cambria Math"],
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn generic_candidates(family: GenericFontFamily) -> &'static [&'static str] {
    match family {
        GenericFontFamily::Serif => &["Noto Serif", "DejaVu Serif", "Liberation Serif"],
        GenericFontFamily::SansSerif => &["Noto Sans", "DejaVu Sans", "Liberation Sans"],
        GenericFontFamily::Monospace => &["Noto Sans Mono", "DejaVu Sans Mono", "Liberation Mono"],
        GenericFontFamily::SystemUi => &["Noto Sans", "DejaVu Sans"],
        GenericFontFamily::Cursive => &["URW Chancery L"],
        GenericFontFamily::Fantasy => &["Impact"],
        GenericFontFamily::Emoji => &["Noto Color Emoji"],
        GenericFontFamily::Math => &["STIX Two Math", "Noto Sans Math", "DejaVu Math TeX Gyre"],
    }
}

#[cfg(target_os = "macos")]
fn platform_font_roots() -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from("/System/Library/Fonts"),
        PathBuf::from("/System/Library/Fonts/Supplemental"),
        PathBuf::from("/Library/Fonts"),
    ];
    if let Some(home) = env::var_os("HOME") {
        roots.push(PathBuf::from(home).join("Library/Fonts"));
    }
    roots
}

#[cfg(target_os = "windows")]
fn platform_font_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(windows) = env::var_os("WINDIR") {
        roots.push(PathBuf::from(windows).join("Fonts"));
    }
    if let Some(local) = env::var_os("LOCALAPPDATA") {
        roots.push(PathBuf::from(local).join("Microsoft/Windows/Fonts"));
    }
    roots
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn platform_font_roots() -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from("/usr/share/fonts"),
        PathBuf::from("/usr/local/share/fonts"),
    ];
    if let Some(data) = env::var_os("XDG_DATA_HOME") {
        roots.push(PathBuf::from(data).join("fonts"));
    } else if let Some(home) = env::var_os("HOME") {
        roots.push(PathBuf::from(&home).join(".local/share/fonts"));
        roots.push(PathBuf::from(home).join(".fonts"));
    }
    roots
}

#[cfg(test)]
mod tests {
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
        let root =
            std::env::temp_dir().join(format!("skia-system-font-test-{}", std::process::id()));
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
}
