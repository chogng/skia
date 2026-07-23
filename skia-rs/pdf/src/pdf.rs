use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    io::{self, Write},
};

use flate2::{Compression, write::ZlibEncoder};
use sha2::{Digest, Sha256};
use skia_core::{
    BlendMode, ClipOp, DisplayList, DrawCommand, FillRule, FontFace, FontId, GlyphId, GlyphOutline,
    GlyphOutlineProvider, GlyphRun, Gradient, GradientGeometry, Paint, Path, PathVerb, Point, Rect,
    Scalar, SkiaError, StrokeAlign, StrokeCap, StrokeJoin, TextError, TextUnit, TileMode,
    Transform, glyph_outline_path,
};
use skia_cpu::{ClipRect, Surface, SurfaceLimits};
use skia_image::{ColorSpace, Image};

/// Stable machine-readable PDF failure code.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PdfErrorCode {
    /// A page operation was attempted in the wrong lifecycle state.
    InvalidState,
    /// A page size or content rectangle was invalid.
    InvalidPage,
    /// A configured limit was zero or internally inconsistent.
    InvalidLimits,
    /// A page, command, object, resource, raster, or byte limit was exceeded.
    ResourceLimit,
    /// A display-list resource reference was invalid.
    InvalidResource,
    /// Document metadata or a requested conformance profile was invalid.
    InvalidMetadata,
    /// A link or named destination was invalid or unresolved.
    InvalidDestination,
    /// The selected PDF native mapping cannot preserve a drawing semantic.
    Unsupported,
    /// Text requires a glyph-outline resolver not present in the first PDF API.
    UnsupportedText,
    /// The image uses a color profile that is not explicitly supported.
    UnsupportedColorProfile,
    /// Fixed-point or PDF offset arithmetic overflowed.
    NumericOverflow,
    /// The destination writer failed.
    Io,
}

/// Source-redacted PDF error with an optional I/O kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdfError {
    code: PdfErrorCode,
    io_kind: Option<io::ErrorKind>,
}

impl PdfError {
    const fn new(code: PdfErrorCode) -> Self {
        Self {
            code,
            io_kind: None,
        }
    }

    fn io(error: &io::Error) -> Self {
        Self {
            code: PdfErrorCode::Io,
            io_kind: Some(error.kind()),
        }
    }

    /// Returns the stable failure code.
    pub const fn code(self) -> PdfErrorCode {
        self.code
    }

    /// Returns the standard I/O category when the destination failed.
    pub const fn io_kind(self) -> Option<io::ErrorKind> {
        self.io_kind
    }
}

impl fmt::Display for PdfError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.io_kind {
            Some(kind) => write!(formatter, "{:?} ({kind:?})", self.code),
            None => write!(formatter, "{:?}", self.code),
        }
    }
}

impl std::error::Error for PdfError {}

type DocumentError = PdfError;
type DocumentErrorCode = PdfErrorCode;

/// Positive PDF page dimensions measured in points (1/72 inch).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PageSize {
    width: Scalar,
    height: Scalar,
}

impl PageSize {
    /// Creates a positive page size.
    pub fn new(width: Scalar, height: Scalar) -> Result<Self, DocumentError> {
        if width.bits() <= 0 || height.bits() <= 0 {
            return Err(DocumentError::new(DocumentErrorCode::InvalidPage));
        }
        Ok(Self { width, height })
    }

    /// Returns the page width in points.
    pub const fn width(self) -> Scalar {
        self.width
    }

    /// Returns the page height in points.
    pub const fn height(self) -> Scalar {
        self.height
    }
}

/// Page geometry, including an optional top-left content clipping rectangle.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PageSpec {
    size: PageSize,
    content_box: Option<Rect>,
}

impl PageSpec {
    /// Creates an unclipped page.
    pub const fn new(size: PageSize) -> Self {
        Self {
            size,
            content_box: None,
        }
    }

    /// Restricts page drawing to a rectangle fully contained by the page.
    pub fn with_content_box(mut self, content_box: Rect) -> Result<Self, DocumentError> {
        if content_box.left().bits() < 0
            || content_box.top().bits() < 0
            || content_box.right() > self.size.width
            || content_box.bottom() > self.size.height
        {
            return Err(DocumentError::new(DocumentErrorCode::InvalidPage));
        }
        self.content_box = Some(content_box);
        Ok(self)
    }

    /// Returns the physical page size.
    pub const fn size(self) -> PageSize {
        self.size
    }

    /// Returns the optional top-left content box.
    pub const fn content_box(self) -> Option<Rect> {
        self.content_box
    }
}

fn rect_within_page(rect: Rect, size: PageSize) -> bool {
    rect.left().bits() >= 0
        && rect.top().bits() >= 0
        && rect.right() <= size.width
        && rect.bottom() <= size.height
}

fn point_within_page(point: Point, size: PageSize) -> bool {
    point.x().bits() >= 0
        && point.y().bits() >= 0
        && point.x() <= size.width
        && point.y() <= size.height
}

fn link_target_is_valid(target: &PdfLinkTarget) -> bool {
    match target {
        PdfLinkTarget::Uri(uri) | PdfLinkTarget::NamedDestination(uri) => !uri.is_empty(),
    }
}

/// Reproducible PDF information dictionary fields.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PdfMetadata {
    /// Human-readable document title.
    pub title: Option<String>,
    /// Human-readable author.
    pub author: Option<String>,
    /// Human-readable subject.
    pub subject: Option<String>,
    /// Search keywords.
    pub keywords: Option<String>,
    /// Application that created the logical document.
    pub creator: Option<String>,
    /// PDF producer. Defaults to a stable `skia-pdf` value when absent.
    pub producer: Option<String>,
    /// Explicit UTC creation time. Omit it to preserve byte-for-byte
    /// reproducibility without wall-clock metadata.
    pub creation: Option<PdfDateTime>,
    /// Explicit UTC modification time. Omit it to preserve byte-for-byte
    /// reproducibility without wall-clock metadata.
    pub modified: Option<PdfDateTime>,
}

type DocumentMetadata = PdfMetadata;

/// A validated UTC timestamp suitable for PDF and XMP metadata.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PdfDateTime {
    year: u16,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
}

impl PdfDateTime {
    /// Creates one validated UTC timestamp.
    pub fn new(
        year: u16,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
    ) -> Result<Self, DocumentError> {
        let days = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if year.is_multiple_of(4)
                && (!year.is_multiple_of(100) || year.is_multiple_of(400)) =>
            {
                29
            }
            2 => 28,
            _ => return Err(DocumentError::new(DocumentErrorCode::InvalidMetadata)),
        };
        if year == 0
            || year > 9_999
            || day == 0
            || day > days
            || hour > 23
            || minute > 59
            || second > 59
        {
            return Err(DocumentError::new(DocumentErrorCode::InvalidMetadata));
        }
        Ok(Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
        })
    }
}

/// PDF conformance profile selected for serialized output.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PdfConformance {
    /// Ordinary PDF 1.7 output with deterministic metadata omitted by default.
    #[default]
    Standard,
    /// PDF/A-2b document metadata, sRGB output intent, and deterministic file
    /// identifier. Explicit creation and modification timestamps are required.
    PdfA2b,
}

/// Destination selected when a PDF link annotation is activated.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfLinkTarget {
    /// Opens a non-empty URI.
    Uri(String),
    /// Jumps to a non-empty document-global named destination.
    NamedDestination(String),
}

/// Semantic role assigned to one complete display list in a tagged PDF page.
///
/// The document writer creates a standards-defined marked-content sequence for
/// the list and a matching flat structure-tree element.  It deliberately does
/// not infer semantics from drawing commands.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PdfStructureTag {
    /// A paragraph of prose.
    Paragraph,
    /// A first-level heading.
    Heading1,
    /// A second-level heading.
    Heading2,
    /// A generic inline span.
    Span,
    /// A figure or illustration.
    Figure,
    /// A list container.
    List,
    /// One list item.
    ListItem,
    /// A table container.
    Table,
    /// One table row.
    TableRow,
    /// One table header cell.
    TableHeader,
    /// One table data cell.
    TableData,
}

/// One embedded TrueType font program selected for searchable PDF text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfEmbeddedFont {
    font: FontId,
    program: Vec<u8>,
}

impl PdfEmbeddedFont {
    /// Creates an embeddable single-face TrueType program for `font`.
    ///
    /// Collection fonts and CFF/OpenType programs require a proper subsetter
    /// and are rejected rather than being mislabeled as `FontFile2` data.
    pub fn new(font: FontId, program: Vec<u8>) -> Result<Self, DocumentError> {
        const MAX_PROGRAM_BYTES: usize = 64 * 1024 * 1024;
        let supported = matches!(program.get(..4), Some(b"\0\x01\0\0" | b"true" | b"typ1"));
        if program.is_empty() || program.len() > MAX_PROGRAM_BYTES || !supported {
            return Err(DocumentError::new(DocumentErrorCode::UnsupportedText));
        }
        Ok(Self { font, program })
    }

    /// Copies a single-face TrueType program from a portable [`FontFace`].
    pub fn from_font_face(face: &FontFace) -> Result<Self, DocumentError> {
        if face.face_index() != 0 {
            return Err(DocumentError::new(DocumentErrorCode::UnsupportedText));
        }
        Self::new(face.id(), face.encoded_bytes().to_vec())
    }

    /// Returns the stable font identity selected by this program.
    pub const fn font(&self) -> FontId {
        self.font
    }
}

/// Supplies an embedded TrueType program and source text for PDF glyph runs.
///
/// The exact source text becomes an `ActualText` replacement, preserving
/// search and copy semantics without guessing Unicode values from glyph IDs.
pub trait PdfTextProvider {
    /// Returns the embeddable program selected by a glyph run's font ID.
    fn embedded_font(&self, font: FontId) -> Option<PdfEmbeddedFont>;

    /// Returns the original non-empty Unicode source text for one glyph run.
    fn source_text(&self, run: &GlyphRun) -> Option<String>;
}

impl PdfStructureTag {
    fn pdf_name(self) -> &'static str {
        match self {
            Self::Paragraph => "P",
            Self::Heading1 => "H1",
            Self::Heading2 => "H2",
            Self::Span => "Span",
            Self::Figure => "Figure",
            Self::List => "L",
            Self::ListItem => "LI",
            Self::Table => "Table",
            Self::TableRow => "TR",
            Self::TableHeader => "TH",
            Self::TableData => "TD",
        }
    }
}

/// Hard ceilings for PDF construction and serialized output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdfLimits {
    /// Maximum completed page count.
    pub max_pages: usize,
    /// Maximum indirect object count.
    pub max_objects: usize,
    /// Maximum globally deduplicated PDF resources.
    pub max_resources: usize,
    /// Maximum display-list commands accepted by one page.
    pub max_commands_per_page: usize,
    /// Maximum link annotations accepted by one page.
    pub max_annotations_per_page: usize,
    /// Maximum document-global named destinations.
    pub max_named_destinations: usize,
    /// Maximum document-global bookmark entries.
    pub max_bookmarks: usize,
    /// Maximum tagged display lists accepted by one page.
    pub max_structure_elements_per_page: usize,
    /// Maximum serialized PDF bytes.
    pub max_output_bytes: u64,
}

impl PdfLimits {
    /// Validates that every ceiling is positive.
    pub fn validate(self) -> Result<Self, DocumentError> {
        if self.max_pages == 0
            || self.max_objects == 0
            || self.max_resources == 0
            || self.max_commands_per_page == 0
            || self.max_annotations_per_page == 0
            || self.max_named_destinations == 0
            || self.max_bookmarks == 0
            || self.max_structure_elements_per_page == 0
            || self.max_output_bytes == 0
        {
            return Err(DocumentError::new(DocumentErrorCode::InvalidLimits));
        }
        Ok(self)
    }
}

impl Default for PdfLimits {
    fn default() -> Self {
        Self {
            max_pages: 10_000,
            max_objects: 100_000,
            max_resources: 16_384,
            max_commands_per_page: 1_000_000,
            max_annotations_per_page: 16_384,
            max_named_destinations: 16_384,
            max_bookmarks: 16_384,
            max_structure_elements_per_page: 16_384,
            max_output_bytes: 512 * 1024 * 1024,
        }
    }
}

type DocumentLimits = PdfLimits;

/// Whole-page CPU raster fallback configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RasterFallback {
    /// Raster resolution. Page coordinates remain PDF points.
    pub dpi: u16,
    /// Maximum pixels in one fallback page.
    pub max_pixels: u64,
    /// Maximum RGBA working bytes in one fallback page.
    pub max_bytes: u64,
}

impl RasterFallback {
    /// Validates positive raster ceilings and a bounded DPI.
    pub fn validate(self) -> Result<Self, DocumentError> {
        if self.dpi == 0 || self.dpi > 2_400 || self.max_pixels == 0 || self.max_bytes == 0 {
            return Err(DocumentError::new(DocumentErrorCode::InvalidLimits));
        }
        Ok(self)
    }
}

impl Default for RasterFallback {
    fn default() -> Self {
        Self {
            dpi: 144,
            max_pixels: 64 * 1024 * 1024,
            max_bytes: 256 * 1024 * 1024,
        }
    }
}

/// Policy for semantics that lack an exact first-tier PDF mapping.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnsupportedBehavior {
    /// Return a stable unsupported error without writing the document.
    Error,
    /// Render the entire affected page through the CPU executor.
    RasterizePage,
}

/// Color-compositing policy for PDF output.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PdfColorPolicy {
    /// Preserve PDF-native vector painting, matching the conventional SkPDF
    /// strategy. PDF viewers choose the blend color space for transparency.
    #[default]
    NativePdf,
    /// Require CPU raster fallback when a page needs source/destination color
    /// compositing, preserving this crate's linear-light rendering contract.
    /// Opaque source-over vector commands remain native.
    LinearMatch,
}

/// PDF backend configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfOptions {
    /// Reproducible Info dictionary fields.
    pub metadata: DocumentMetadata,
    /// Requested document-level PDF conformance profile.
    pub conformance: PdfConformance,
    /// Construction and output ceilings.
    pub limits: DocumentLimits,
    /// Unsupported drawing policy.
    pub unsupported_behavior: UnsupportedBehavior,
    /// Color-compositing policy for native PDF content.
    pub color_policy: PdfColorPolicy,
    /// Whole-page fallback policy used when enabled.
    pub raster_fallback: RasterFallback,
}

impl Default for PdfOptions {
    fn default() -> Self {
        Self {
            metadata: DocumentMetadata::default(),
            conformance: PdfConformance::Standard,
            limits: DocumentLimits::default(),
            unsupported_behavior: UnsupportedBehavior::Error,
            color_policy: PdfColorPolicy::NativePdf,
            raster_fallback: RasterFallback::default(),
        }
    }
}

#[derive(Clone)]
struct ActivePage {
    spec: PageSpec,
    lists: Vec<ActiveList>,
    command_count: usize,
    annotations: Vec<LinkAnnotation>,
    destinations: Vec<NamedDestination>,
}

#[derive(Clone)]
struct ActiveList {
    list: DisplayList,
    structure_tag: Option<PdfStructureTag>,
}

#[derive(Clone)]
struct PageData {
    spec: PageSpec,
    content: Vec<u8>,
    ext_gstates: Vec<usize>,
    images: Vec<usize>,
    gradients: Vec<usize>,
    forms: Vec<usize>,
    fonts: Vec<usize>,
    annotations: Vec<LinkAnnotation>,
    destinations: Vec<NamedDestination>,
    structure: Vec<PdfStructureEntry>,
}

#[derive(Clone)]
struct LinkAnnotation {
    rect: Rect,
    target: PdfLinkTarget,
}

#[derive(Clone)]
struct NamedDestination {
    name: String,
    point: Point,
}

#[derive(Clone)]
struct PdfBookmark {
    title: String,
    destination: String,
}

#[derive(Clone, Copy)]
struct PdfStructureEntry {
    tag: PdfStructureTag,
    mcid: usize,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ExtGState {
    alpha: u8,
    blend_mode: BlendMode,
}

#[derive(Clone, Default)]
struct Resources {
    ext_gstates: Vec<ExtGState>,
    images: Vec<PdfImage>,
    gradients: Vec<Gradient>,
    forms: Vec<PdfForm>,
    fonts: Vec<PdfFont>,
}

#[derive(Clone, Eq, PartialEq)]
struct PdfImage {
    image: Image,
    interpolate: bool,
}

#[derive(Clone)]
struct PdfForm {
    size: PageSize,
    content: Vec<u8>,
    ext_gstates: Vec<usize>,
    images: Vec<usize>,
    gradients: Vec<usize>,
    forms: Vec<usize>,
    fonts: Vec<usize>,
}

#[derive(Clone, Eq, PartialEq)]
struct PdfFont {
    font: FontId,
    program: Vec<u8>,
}

/// Stateful PDF 1.7 document writer over an arbitrary `std::io::Write`.
///
/// Construction is transactional: no bytes are sent to the destination until
/// [`finish`](Self::finish). Consuming `finish` and `abort` prevent repeated
/// closure at the type level.
pub struct PdfDocument<W: Write> {
    writer: W,
    options: PdfOptions,
    pages: Vec<PageData>,
    active: Option<ActivePage>,
    resources: Resources,
    bookmarks: Vec<PdfBookmark>,
}

impl<W: Write> PdfDocument<W> {
    /// Creates an empty deterministic PDF document.
    pub fn new(writer: W, options: PdfOptions) -> Result<Self, DocumentError> {
        options.limits.validate()?;
        options.raster_fallback.validate()?;
        if options.conformance == PdfConformance::PdfA2b
            && (options.metadata.creation.is_none() || options.metadata.modified.is_none())
        {
            return Err(DocumentError::new(DocumentErrorCode::InvalidMetadata));
        }
        Ok(Self {
            writer,
            options,
            pages: Vec::new(),
            active: None,
            resources: Resources::default(),
            bookmarks: Vec::new(),
        })
    }

    /// Starts a page. Nested pages are rejected.
    pub fn begin_page(&mut self, spec: PageSpec) -> Result<(), DocumentError> {
        if self.active.is_some() {
            return Err(DocumentError::new(DocumentErrorCode::InvalidState));
        }
        if self.pages.len() >= self.options.limits.max_pages {
            return Err(DocumentError::new(DocumentErrorCode::ResourceLimit));
        }
        self.active = Some(ActivePage {
            spec,
            lists: Vec::new(),
            command_count: 0,
            annotations: Vec::new(),
            destinations: Vec::new(),
        });
        Ok(())
    }

    /// Appends one immutable display list to the active page.
    pub fn add_display_list(&mut self, list: &DisplayList) -> Result<(), DocumentError> {
        let active = self
            .active
            .as_mut()
            .ok_or(DocumentError::new(DocumentErrorCode::InvalidState))?;
        let count = active
            .command_count
            .checked_add(list.commands().len())
            .ok_or(DocumentError::new(DocumentErrorCode::ResourceLimit))?;
        if count > self.options.limits.max_commands_per_page {
            return Err(DocumentError::new(DocumentErrorCode::ResourceLimit));
        }
        active.command_count = count;
        active.lists.push(ActiveList {
            list: list.clone(),
            structure_tag: None,
        });
        Ok(())
    }

    /// Appends one semantically tagged immutable display list to the active
    /// page. Tagged output requires native PDF compilation; it is not silently
    /// replaced with a page bitmap.
    pub fn add_tagged_display_list(
        &mut self,
        tag: PdfStructureTag,
        list: &DisplayList,
    ) -> Result<(), DocumentError> {
        let active = self
            .active
            .as_mut()
            .ok_or(DocumentError::new(DocumentErrorCode::InvalidState))?;
        let count = active
            .command_count
            .checked_add(list.commands().len())
            .ok_or(DocumentError::new(DocumentErrorCode::ResourceLimit))?;
        if count > self.options.limits.max_commands_per_page
            || active
                .lists
                .iter()
                .filter(|entry| entry.structure_tag.is_some())
                .count()
                >= self.options.limits.max_structure_elements_per_page
        {
            return Err(DocumentError::new(DocumentErrorCode::ResourceLimit));
        }
        active.command_count = count;
        active.lists.push(ActiveList {
            list: list.clone(),
            structure_tag: Some(tag),
        });
        Ok(())
    }

    /// Adds one link annotation in the active page's top-left coordinate space.
    pub fn add_link(&mut self, rect: Rect, target: PdfLinkTarget) -> Result<(), DocumentError> {
        let active = self
            .active
            .as_mut()
            .ok_or(DocumentError::new(DocumentErrorCode::InvalidState))?;
        if active.annotations.len() >= self.options.limits.max_annotations_per_page
            || !rect_within_page(rect, active.spec.size)
            || !link_target_is_valid(&target)
        {
            return Err(DocumentError::new(DocumentErrorCode::InvalidDestination));
        }
        active.annotations.push(LinkAnnotation { rect, target });
        Ok(())
    }

    /// Defines one document-global destination on the active page.
    pub fn add_named_destination(
        &mut self,
        name: String,
        point: Point,
    ) -> Result<(), DocumentError> {
        let completed_count = self
            .pages
            .iter()
            .map(|page| page.destinations.len())
            .sum::<usize>();
        let active = self
            .active
            .as_mut()
            .ok_or(DocumentError::new(DocumentErrorCode::InvalidState))?;
        if name.is_empty()
            || completed_count
                .checked_add(active.destinations.len())
                .is_none_or(|count| count >= self.options.limits.max_named_destinations)
            || !point_within_page(point, active.spec.size)
        {
            return Err(DocumentError::new(DocumentErrorCode::InvalidDestination));
        }
        active.destinations.push(NamedDestination { name, point });
        Ok(())
    }

    /// Adds a document-outline bookmark that jumps to an existing or later
    /// named destination.
    pub fn add_bookmark(
        &mut self,
        title: String,
        destination: String,
    ) -> Result<(), DocumentError> {
        if title.is_empty()
            || destination.is_empty()
            || self.bookmarks.len() >= self.options.limits.max_bookmarks
        {
            return Err(DocumentError::new(DocumentErrorCode::InvalidDestination));
        }
        self.bookmarks.push(PdfBookmark { title, destination });
        Ok(())
    }

    /// Completes the active page and resolves its native or fallback content.
    pub fn end_page(&mut self) -> Result<(), DocumentError> {
        self.end_page_inner(None, None)
    }

    /// Completes the active page using portable glyph outlines when it contains
    /// text commands. Text is emitted as vector paths, not searchable PDF text.
    pub fn end_page_with_glyph_outlines(
        &mut self,
        glyphs: &impl GlyphOutlineProvider,
    ) -> Result<(), DocumentError> {
        self.end_page_inner(Some(glyphs), None)
    }

    /// Completes the active page using embedded TrueType glyph text and the
    /// provider's exact source string for searchable PDF output.
    pub fn end_page_with_embedded_text(
        &mut self,
        text: &impl PdfTextProvider,
    ) -> Result<(), DocumentError> {
        self.end_page_inner(None, Some(text))
    }

    fn end_page_inner(
        &mut self,
        glyphs: Option<&dyn GlyphOutlineProvider>,
        text: Option<&dyn PdfTextProvider>,
    ) -> Result<(), DocumentError> {
        let active = self
            .active
            .as_ref()
            .ok_or(DocumentError::new(DocumentErrorCode::InvalidState))?;
        let mut resources = self.resources.clone();
        let native = if self.options.color_policy == PdfColorPolicy::LinearMatch
            && page_requires_linear_match_fallback(active)?
        {
            Err(DocumentError::new(DocumentErrorCode::Unsupported))
        } else {
            compile_native_page(active, glyphs, text, &mut resources, self.options.limits)
        };
        let page = match native {
            Ok(page) => page,
            Err(error)
                if error.code() == DocumentErrorCode::Unsupported
                    && self.options.unsupported_behavior == UnsupportedBehavior::RasterizePage =>
            {
                compile_raster_page(
                    active,
                    glyphs,
                    &mut resources,
                    self.options.raster_fallback,
                    self.options.limits,
                )?
            }
            Err(error) => return Err(error),
        };
        self.resources = resources;
        self.pages.push(page);
        self.active = None;
        Ok(())
    }

    /// Adds and completes one page in a single call.
    pub fn add_page(&mut self, spec: PageSpec, list: &DisplayList) -> Result<(), DocumentError> {
        self.begin_page(spec)?;
        if let Err(error) = self.add_display_list(list).and_then(|()| self.end_page()) {
            self.active = None;
            return Err(error);
        }
        Ok(())
    }

    /// Adds and completes one page using portable glyph outlines for text.
    pub fn add_page_with_glyph_outlines(
        &mut self,
        spec: PageSpec,
        list: &DisplayList,
        glyphs: &impl GlyphOutlineProvider,
    ) -> Result<(), DocumentError> {
        self.begin_page(spec)?;
        if let Err(error) = self
            .add_display_list(list)
            .and_then(|()| self.end_page_with_glyph_outlines(glyphs))
        {
            self.active = None;
            return Err(error);
        }
        Ok(())
    }

    /// Adds and completes one page using embedded TrueType glyph text.
    pub fn add_page_with_embedded_text(
        &mut self,
        spec: PageSpec,
        list: &DisplayList,
        text: &impl PdfTextProvider,
    ) -> Result<(), DocumentError> {
        self.begin_page(spec)?;
        if let Err(error) = self
            .add_display_list(list)
            .and_then(|()| self.end_page_with_embedded_text(text))
        {
            self.active = None;
            return Err(error);
        }
        Ok(())
    }

    /// Returns the number of completed pages.
    pub const fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Returns whether a page is currently open.
    pub const fn is_page_open(&self) -> bool {
        self.active.is_some()
    }

    /// Serializes a deterministic PDF 1.7 and returns the underlying writer.
    pub fn finish(self) -> Result<W, DocumentError> {
        if self.active.is_some() {
            return Err(DocumentError::new(DocumentErrorCode::InvalidState));
        }
        serialize_pdf(
            self.writer,
            &self.pages,
            &self.resources,
            &self.options.metadata,
            self.options.conformance,
            self.options.limits,
            &self.bookmarks,
        )
    }

    /// Aborts construction without writing bytes and returns the destination.
    pub fn abort(self) -> W {
        self.writer
    }
}

struct OutlineProviderRef<'a>(&'a dyn GlyphOutlineProvider);

impl GlyphOutlineProvider for OutlineProviderRef<'_> {
    fn glyph_outline(
        &self,
        font: FontId,
        glyph: GlyphId,
    ) -> Result<Option<GlyphOutline>, TextError> {
        self.0.glyph_outline(font, glyph)
    }
}

fn page_requires_linear_match_fallback(active: &ActivePage) -> Result<bool, DocumentError> {
    for entry in &active.lists {
        let list = &entry.list;
        for command in list.commands() {
            match command {
                DrawCommand::Clear(color) if color.alpha() != u8::MAX => return Ok(true),
                DrawCommand::SaveLayer(_) => return Ok(true),
                DrawCommand::FillRect { paint, .. }
                | DrawCommand::FillPath { paint, .. }
                | DrawCommand::StrokePath { paint, .. }
                | DrawCommand::DrawGlyphRun { paint, .. }
                | DrawCommand::DrawPositionedGlyphRun { paint, .. }
                    if paint_requires_linear_match_fallback(paint) =>
                {
                    return Ok(true);
                }
                DrawCommand::DrawImage {
                    image,
                    opacity,
                    paint,
                    ..
                } => {
                    let image = list
                        .image(*image)
                        .ok_or(DocumentError::new(DocumentErrorCode::InvalidResource))?;
                    if multiply_alpha(*opacity, paint.color().alpha()) != u8::MAX
                        || paint.blend_mode() != BlendMode::SourceOver
                        || image_has_transparency(image)
                    {
                        return Ok(true);
                    }
                }
                _ => {}
            }
        }
    }
    Ok(false)
}

fn paint_requires_linear_match_fallback(paint: &Paint) -> bool {
    paint.color().alpha() != u8::MAX || paint.blend_mode() != BlendMode::SourceOver
}

fn image_has_transparency(image: &Image) -> bool {
    for y in 0..image.height() {
        for x in 0..image.width() {
            if image.pixel_at(x, y).is_none_or(|pixel| pixel[3] != u8::MAX) {
                return true;
            }
        }
    }
    false
}

fn compile_native_page(
    active: &ActivePage,
    glyphs: Option<&dyn GlyphOutlineProvider>,
    text: Option<&dyn PdfTextProvider>,
    resources: &mut Resources,
    limits: DocumentLimits,
) -> Result<PageData, DocumentError> {
    let mut content = Vec::new();
    let mut used_gstates = Vec::new();
    let mut used_images = Vec::new();
    let mut used_gradients = Vec::new();
    let mut used_forms = Vec::new();
    let mut used_fonts = Vec::new();
    let mut structure = Vec::new();
    push_text(&mut content, "q\n");
    push_text(
        &mut content,
        &format!("1 0 0 -1 0 {} cm\n", pdf_scalar(active.spec.size.height)),
    );
    if let Some(rect) = active.spec.content_box {
        emit_rect(&mut content, rect);
        push_text(&mut content, "W n\n");
    }
    for (list_index, entry) in active.lists.iter().enumerate() {
        let list = &entry.list;
        push_text(&mut content, "q\n");
        if let Some(tag) = entry.structure_tag {
            let mcid = structure.len();
            push_text(
                &mut content,
                &format!("/{} << /MCID {mcid} >> BDC\n", tag.pdf_name()),
            );
            structure.push(PdfStructureEntry { tag, mcid });
        }
        compile_list(
            list,
            list_index == 0,
            active.spec,
            glyphs,
            text,
            &mut content,
            resources,
            &mut used_gstates,
            &mut used_images,
            &mut used_gradients,
            &mut used_forms,
            &mut used_fonts,
            limits,
        )?;
        if entry.structure_tag.is_some() {
            push_text(&mut content, "EMC\n");
        }
        push_text(&mut content, "Q\n");
    }
    push_text(&mut content, "Q\n");
    Ok(PageData {
        spec: active.spec,
        content,
        ext_gstates: used_gstates,
        images: used_images,
        gradients: used_gradients,
        forms: used_forms,
        fonts: used_fonts,
        annotations: active.annotations.clone(),
        destinations: active.destinations.clone(),
        structure,
    })
}

#[allow(clippy::too_many_arguments)]
fn compile_list(
    list: &DisplayList,
    first_list: bool,
    spec: PageSpec,
    glyphs: Option<&dyn GlyphOutlineProvider>,
    text: Option<&dyn PdfTextProvider>,
    output: &mut Vec<u8>,
    resources: &mut Resources,
    used_gstates: &mut Vec<usize>,
    used_images: &mut Vec<usize>,
    used_gradients: &mut Vec<usize>,
    used_forms: &mut Vec<usize>,
    used_fonts: &mut Vec<usize>,
    limits: DocumentLimits,
) -> Result<(), DocumentError> {
    let mut cursor = 0;
    compile_commands(
        list,
        &mut cursor,
        first_list,
        false,
        spec,
        glyphs,
        text,
        output,
        resources,
        used_gstates,
        used_images,
        used_gradients,
        used_forms,
        used_fonts,
        limits,
        Transform::IDENTITY,
    )?;
    if cursor != list.commands().len() {
        return Err(DocumentError::new(DocumentErrorCode::InvalidState));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn compile_commands(
    list: &DisplayList,
    cursor: &mut usize,
    allow_initial_clear: bool,
    stop_at_restore: bool,
    spec: PageSpec,
    glyphs: Option<&dyn GlyphOutlineProvider>,
    text: Option<&dyn PdfTextProvider>,
    output: &mut Vec<u8>,
    resources: &mut Resources,
    used_gstates: &mut Vec<usize>,
    used_images: &mut Vec<usize>,
    used_gradients: &mut Vec<usize>,
    used_forms: &mut Vec<usize>,
    used_fonts: &mut Vec<usize>,
    limits: DocumentLimits,
    mut transform: Transform,
) -> Result<Transform, DocumentError> {
    let mut transforms = Vec::new();
    while let Some(command) = list.commands().get(*cursor) {
        let command_index = *cursor;
        *cursor = cursor
            .checked_add(1)
            .ok_or(DocumentError::new(DocumentErrorCode::NumericOverflow))?;
        match command {
            DrawCommand::Clear(color) => {
                if !allow_initial_clear || command_index != 0 {
                    return Err(DocumentError::new(DocumentErrorCode::Unsupported));
                }
                emit_paint(
                    output,
                    &Paint::new(*color),
                    false,
                    resources,
                    used_gstates,
                    limits,
                )?;
                emit_page_rect(output, spec.size);
                push_text(output, "f\n");
            }
            DrawCommand::Save => {
                push_text(output, "q\n");
                transforms.push(transform);
            }
            DrawCommand::SaveLayer(options) => {
                let form = compile_layer(
                    list,
                    cursor,
                    spec,
                    options.clone(),
                    glyphs,
                    text,
                    resources,
                    limits,
                    transform,
                )?;
                let gstate = intern_gstate(
                    resources,
                    ExtGState {
                        alpha: options.opacity(),
                        blend_mode: options.blend_mode(),
                    },
                    limits,
                )?;
                push_unique(used_gstates, gstate);
                push_unique(used_forms, form);
                push_text(output, &format!("q\n/GS{gstate} gs\n/Fm{form} Do\nQ\n"));
            }
            DrawCommand::Restore => {
                if let Some(saved) = transforms.pop() {
                    transform = saved;
                    push_text(output, "Q\n");
                } else if stop_at_restore {
                    return Ok(transform);
                } else {
                    return Err(DocumentError::new(DocumentErrorCode::InvalidState));
                }
            }
            DrawCommand::ClipRect { rect, op } => {
                if *op != ClipOp::Intersect {
                    return Err(DocumentError::new(DocumentErrorCode::Unsupported));
                }
                emit_rect(output, *rect);
                push_text(output, "W n\n");
            }
            DrawCommand::ClipPath { path, rule, op } => {
                if *op != ClipOp::Intersect {
                    return Err(DocumentError::new(DocumentErrorCode::Unsupported));
                }
                emit_path(
                    output,
                    list.path(*path)
                        .ok_or(DocumentError::new(DocumentErrorCode::InvalidResource))?,
                )?;
                push_text(
                    output,
                    if *rule == FillRule::EvenOdd {
                        "W* n\n"
                    } else {
                        "W n\n"
                    },
                );
            }
            DrawCommand::SetTransform(next) => {
                let reset = transform
                    .inverse()
                    .map_err(map_skia_error)?
                    .concat(*next)
                    .map_err(map_skia_error)?;
                emit_transform(output, reset);
                transform = *next;
            }
            DrawCommand::ConcatTransform(next) => {
                emit_transform(output, *next);
                transform = transform.concat(*next).map_err(map_skia_error)?;
            }
            DrawCommand::FillRect { rect, paint } => {
                if let Some(gradient) = paint.gradient() {
                    emit_gradient_rect(
                        output,
                        *rect,
                        gradient,
                        paint,
                        resources,
                        used_gradients,
                        limits,
                    )?;
                } else {
                    emit_paint(output, paint, false, resources, used_gstates, limits)?;
                    emit_rect(output, *rect);
                    push_text(output, "f\n");
                }
            }
            DrawCommand::FillPath { path, rule, paint } => {
                let path = list
                    .path(*path)
                    .ok_or(DocumentError::new(DocumentErrorCode::InvalidResource))?;
                if let Some(gradient) = paint.gradient() {
                    emit_gradient_path(
                        output,
                        path,
                        *rule,
                        gradient,
                        paint,
                        resources,
                        used_gradients,
                        limits,
                    )?;
                } else {
                    emit_paint(output, paint, false, resources, used_gstates, limits)?;
                    emit_path(output, path)?;
                    push_text(
                        output,
                        if *rule == FillRule::EvenOdd {
                            "f*\n"
                        } else {
                            "f\n"
                        },
                    );
                }
            }
            DrawCommand::StrokePath {
                path,
                options,
                paint,
            } => {
                if options.align() != StrokeAlign::Center || paint.path_effect().is_some() {
                    return Err(DocumentError::new(DocumentErrorCode::Unsupported));
                }
                emit_paint(output, paint, true, resources, used_gstates, limits)?;
                push_text(
                    output,
                    &format!(
                        "{} w\n{} J\n{} j\n{} M\n",
                        pdf_scalar(options.width()),
                        cap_number(options.cap()),
                        join_number(options.join()),
                        pdf_scalar(options.miter_limit())
                    ),
                );
                if options.dash_pattern().is_empty() {
                    push_text(output, "[] 0 d\n");
                } else {
                    let pattern = options
                        .dash_pattern()
                        .iter()
                        .map(|value| pdf_scalar(*value))
                        .collect::<Vec<_>>()
                        .join(" ");
                    push_text(
                        output,
                        &format!("[{pattern}] {} d\n", pdf_scalar(options.dash_phase())),
                    );
                }
                emit_path(
                    output,
                    list.path(*path)
                        .ok_or(DocumentError::new(DocumentErrorCode::InvalidResource))?,
                )?;
                push_text(output, "S\n");
            }
            DrawCommand::DrawImage {
                image,
                destination,
                opacity,
                sampling,
                paint,
            } => {
                if paint.shader_handle().is_some()
                    || paint.color_filter_handle().is_some()
                    || paint.path_effect().is_some()
                {
                    return Err(DocumentError::new(DocumentErrorCode::Unsupported));
                }
                let image = list
                    .image(*image)
                    .ok_or(DocumentError::new(DocumentErrorCode::InvalidResource))?;
                if !matches!(image.color_space(), ColorSpace::Srgb) {
                    return Err(DocumentError::new(
                        DocumentErrorCode::UnsupportedColorProfile,
                    ));
                }
                let interpolate = *sampling == skia_core::SamplingOptions::LINEAR;
                let image_index = intern_image(resources, image, interpolate, limits)?;
                push_unique(used_images, image_index);
                let alpha = multiply_alpha(*opacity, paint.color().alpha());
                let gs = intern_gstate(
                    resources,
                    ExtGState {
                        alpha,
                        blend_mode: paint.blend_mode(),
                    },
                    limits,
                )?;
                push_unique(used_gstates, gs);
                push_text(
                    output,
                    &format!(
                        "q\n/GS{gs} gs\n{} 0 0 {} {} {} cm\n/Im{image_index} Do\nQ\n",
                        pdf_scalar(Scalar::from_bits(
                            destination.right().bits() - destination.left().bits()
                        )),
                        pdf_scalar(Scalar::from_bits(
                            destination.bottom().bits() - destination.top().bits()
                        )),
                        pdf_scalar(destination.left()),
                        pdf_scalar(destination.top()),
                    ),
                );
            }
            DrawCommand::DrawGlyphRun { run, paint } => {
                let run = list
                    .glyph_run(*run)
                    .ok_or(DocumentError::new(DocumentErrorCode::InvalidResource))?;
                if let Some(text) = text {
                    emit_embedded_glyph_run(
                        output,
                        run,
                        Point::new(Scalar::ZERO, Scalar::ZERO),
                        None,
                        text,
                        paint,
                        resources,
                        used_gstates,
                        used_fonts,
                        limits,
                    )?;
                } else {
                    let glyphs =
                        glyphs.ok_or(DocumentError::new(DocumentErrorCode::UnsupportedText))?;
                    emit_glyph_run(output, run, glyphs, paint, resources, used_gstates, limits)?;
                }
            }
            DrawCommand::DrawPositionedGlyphRun {
                run,
                origin,
                offsets_x_bits,
                paint,
            } => {
                let run = list
                    .glyph_run(*run)
                    .ok_or(DocumentError::new(DocumentErrorCode::InvalidResource))?;
                if let Some(text) = text {
                    emit_embedded_glyph_run(
                        output,
                        run,
                        *origin,
                        Some(offsets_x_bits),
                        text,
                        paint,
                        resources,
                        used_gstates,
                        used_fonts,
                        limits,
                    )?;
                } else {
                    let glyphs =
                        glyphs.ok_or(DocumentError::new(DocumentErrorCode::UnsupportedText))?;
                    emit_positioned_glyph_run(
                        output,
                        run,
                        *origin,
                        offsets_x_bits,
                        glyphs,
                        paint,
                        resources,
                        used_gstates,
                        limits,
                    )?;
                }
            }
        }
    }
    if stop_at_restore || !transforms.is_empty() {
        return Err(DocumentError::new(DocumentErrorCode::InvalidState));
    }
    Ok(transform)
}

#[allow(clippy::too_many_arguments)]
fn compile_layer(
    list: &DisplayList,
    cursor: &mut usize,
    spec: PageSpec,
    options: skia_core::SaveLayerOptions,
    glyphs: Option<&dyn GlyphOutlineProvider>,
    text: Option<&dyn PdfTextProvider>,
    resources: &mut Resources,
    limits: DocumentLimits,
    transform: Transform,
) -> Result<usize, DocumentError> {
    if options.filter_handle().is_some() || pdf_blend_name(options.blend_mode()).is_none() {
        return Err(DocumentError::new(DocumentErrorCode::Unsupported));
    }
    let mut content = Vec::new();
    let mut used_gstates = Vec::new();
    let mut used_images = Vec::new();
    let mut used_gradients = Vec::new();
    let mut used_forms = Vec::new();
    let mut used_fonts = Vec::new();
    push_text(&mut content, "q\n");
    if let Some(bounds) = options.bounds() {
        emit_rect(&mut content, bounds);
        push_text(&mut content, "W n\n");
    }
    compile_commands(
        list,
        cursor,
        false,
        true,
        spec,
        glyphs,
        text,
        &mut content,
        resources,
        &mut used_gstates,
        &mut used_images,
        &mut used_gradients,
        &mut used_forms,
        &mut used_fonts,
        limits,
        transform,
    )?;
    push_text(&mut content, "Q\n");
    intern_form(
        resources,
        PdfForm {
            size: spec.size,
            content,
            ext_gstates: used_gstates,
            images: used_images,
            gradients: used_gradients,
            forms: used_forms,
            fonts: used_fonts,
        },
        limits,
    )
}

fn emit_paint(
    output: &mut Vec<u8>,
    paint: &Paint,
    stroke: bool,
    resources: &mut Resources,
    used_gstates: &mut Vec<usize>,
    limits: DocumentLimits,
) -> Result<(), DocumentError> {
    if paint.shader_handle().is_some()
        || paint.color_filter_handle().is_some()
        || (!stroke && paint.path_effect().is_some())
    {
        return Err(DocumentError::new(DocumentErrorCode::Unsupported));
    }
    if pdf_blend_name(paint.blend_mode()).is_none() {
        return Err(DocumentError::new(DocumentErrorCode::Unsupported));
    }
    let color = paint.color();
    let operator = if stroke { "RG" } else { "rg" };
    push_text(
        output,
        &format!(
            "{} {} {} {operator}\n",
            pdf_channel(color.red()),
            pdf_channel(color.green()),
            pdf_channel(color.blue())
        ),
    );
    let gs = intern_gstate(
        resources,
        ExtGState {
            alpha: color.alpha(),
            blend_mode: paint.blend_mode(),
        },
        limits,
    )?;
    push_unique(used_gstates, gs);
    push_text(output, &format!("/GS{gs} gs\n"));
    Ok(())
}

fn emit_gradient_rect(
    output: &mut Vec<u8>,
    rect: Rect,
    gradient: Gradient,
    paint: &Paint,
    resources: &mut Resources,
    used_gradients: &mut Vec<usize>,
    limits: DocumentLimits,
) -> Result<(), DocumentError> {
    let gradient = intern_gradient(resources, gradient, paint, limits)?;
    push_unique(used_gradients, gradient);
    push_text(output, "q\n");
    emit_rect(output, rect);
    push_text(output, "W n\n");
    push_text(output, &format!("/Sh{gradient} sh\nQ\n"));
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn emit_gradient_path(
    output: &mut Vec<u8>,
    path: &Path,
    rule: FillRule,
    gradient: Gradient,
    paint: &Paint,
    resources: &mut Resources,
    used_gradients: &mut Vec<usize>,
    limits: DocumentLimits,
) -> Result<(), DocumentError> {
    let gradient = intern_gradient(resources, gradient, paint, limits)?;
    push_unique(used_gradients, gradient);
    push_text(output, "q\n");
    emit_path(output, path)?;
    push_text(
        output,
        if rule == FillRule::EvenOdd {
            "W* n\n"
        } else {
            "W n\n"
        },
    );
    push_text(output, &format!("/Sh{gradient} sh\nQ\n"));
    Ok(())
}

fn intern_gradient(
    resources: &mut Resources,
    gradient: Gradient,
    paint: &Paint,
    limits: DocumentLimits,
) -> Result<usize, DocumentError> {
    if paint.color().alpha() != u8::MAX
        || paint.blend_mode() != BlendMode::SourceOver
        || paint.color_filter_handle().is_some()
        || paint.path_effect().is_some()
        || gradient.tile_mode() != TileMode::Clamp
        || gradient
            .stops()
            .iter()
            .any(|stop| stop.color().alpha() != u8::MAX)
        || gradient
            .stops()
            .windows(2)
            .any(|stops| stops[0].offset() >= stops[1].offset())
    {
        return Err(DocumentError::new(DocumentErrorCode::Unsupported));
    }
    if let Some(index) = resources
        .gradients
        .iter()
        .position(|candidate| *candidate == gradient)
    {
        return Ok(index);
    }
    ensure_resource_capacity(resources, limits)?;
    resources.gradients.push(gradient);
    Ok(resources.gradients.len() - 1)
}

#[allow(clippy::too_many_arguments)]
fn emit_glyph_run(
    output: &mut Vec<u8>,
    run: &GlyphRun,
    glyphs: &dyn GlyphOutlineProvider,
    paint: &Paint,
    resources: &mut Resources,
    used_gstates: &mut Vec<usize>,
    limits: DocumentLimits,
) -> Result<(), DocumentError> {
    emit_paint(output, paint, false, resources, used_gstates, limits)?;
    for glyph in run.glyphs() {
        emit_glyph_path(output, run, *glyph, glyphs)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn emit_embedded_glyph_run(
    output: &mut Vec<u8>,
    run: &GlyphRun,
    origin: Point,
    offsets_x_bits: Option<&[i32]>,
    text: &dyn PdfTextProvider,
    paint: &Paint,
    resources: &mut Resources,
    used_gstates: &mut Vec<usize>,
    used_fonts: &mut Vec<usize>,
    limits: DocumentLimits,
) -> Result<(), DocumentError> {
    let embedded = text
        .embedded_font(run.font())
        .ok_or(DocumentError::new(DocumentErrorCode::UnsupportedText))?;
    if embedded.font() != run.font() {
        return Err(DocumentError::new(DocumentErrorCode::InvalidResource));
    }
    let source = text
        .source_text(run)
        .filter(|source| !source.is_empty())
        .ok_or(DocumentError::new(DocumentErrorCode::UnsupportedText))?;
    if let Some(offsets) = offsets_x_bits
        && offsets.len() != run.glyphs().len()
    {
        return Err(DocumentError::new(DocumentErrorCode::InvalidResource));
    }
    let font = intern_font(resources, embedded, limits)?;
    push_unique(used_fonts, font);
    emit_paint(output, paint, false, resources, used_gstates, limits)?;
    push_text(output, "q\n");
    push_text(
        output,
        &format!(
            "/Span << /ActualText {} >> BDC\nBT\n/F{font} {} Tf\n",
            pdf_string(&source),
            pdf_scalar(Scalar::from_bits(run.font_size_bits()))
        ),
    );
    for (index, glyph) in run.glyphs().iter().enumerate() {
        let glyph_id = u16::try_from(glyph.glyph().value())
            .map_err(|_| DocumentError::new(DocumentErrorCode::UnsupportedText))?;
        let mut x = scaled_text_unit(glyph.x(), run)?;
        let y = scaled_text_unit(glyph.y(), run)?;
        x = scalar_sum(x, origin.x())?;
        if let Some(offsets) = offsets_x_bits {
            x = scalar_sum(x, Scalar::from_bits(offsets[index]))?;
        }
        let y = scalar_sum(y, origin.y())?;
        push_text(
            output,
            &format!(
                "1 0 0 -1 {} {} Tm\n<{glyph_id:04X}> Tj\n",
                pdf_scalar(x),
                pdf_scalar(y),
            ),
        );
    }
    push_text(output, "ET\nEMC\nQ\n");
    Ok(())
}

fn scaled_text_unit(unit: TextUnit, run: &GlyphRun) -> Result<Scalar, DocumentError> {
    let numerator = i128::from(unit.bits())
        .checked_mul(i128::from(run.font_size_bits()))
        .ok_or(DocumentError::new(DocumentErrorCode::NumericOverflow))?;
    let denominator = i128::from(64_i32)
        .checked_mul(i128::from(run.units_per_em()))
        .ok_or(DocumentError::new(DocumentErrorCode::NumericOverflow))?;
    let bits = if numerator >= 0 {
        numerator
            .checked_add(denominator / 2)
            .ok_or(DocumentError::new(DocumentErrorCode::NumericOverflow))?
            / denominator
    } else {
        -((-numerator
            .checked_add(denominator / 2)
            .ok_or(DocumentError::new(DocumentErrorCode::NumericOverflow))?)
            / denominator)
    };
    i32::try_from(bits)
        .map(Scalar::from_bits)
        .map_err(|_| DocumentError::new(DocumentErrorCode::NumericOverflow))
}

fn scalar_sum(first: Scalar, second: Scalar) -> Result<Scalar, DocumentError> {
    first
        .bits()
        .checked_add(second.bits())
        .map(Scalar::from_bits)
        .ok_or(DocumentError::new(DocumentErrorCode::NumericOverflow))
}

#[allow(clippy::too_many_arguments)]
fn emit_positioned_glyph_run(
    output: &mut Vec<u8>,
    run: &GlyphRun,
    origin: Point,
    offsets_x_bits: &[i32],
    glyphs: &dyn GlyphOutlineProvider,
    paint: &Paint,
    resources: &mut Resources,
    used_gstates: &mut Vec<usize>,
    limits: DocumentLimits,
) -> Result<(), DocumentError> {
    if offsets_x_bits.len() != run.glyphs().len() {
        return Err(DocumentError::new(DocumentErrorCode::InvalidResource));
    }
    push_text(output, "q\n");
    emit_transform(output, Transform::translate(origin.x(), origin.y()));
    emit_paint(output, paint, false, resources, used_gstates, limits)?;
    let mut applied_offset_bits = 0_i32;
    for (glyph, offset_bits) in run.glyphs().iter().zip(offsets_x_bits) {
        let delta_bits = offset_bits
            .checked_sub(applied_offset_bits)
            .ok_or(DocumentError::new(DocumentErrorCode::NumericOverflow))?;
        if delta_bits != 0 {
            emit_transform(
                output,
                Transform::translate(Scalar::from_bits(delta_bits), Scalar::ZERO),
            );
            applied_offset_bits = *offset_bits;
        }
        emit_glyph_path(output, run, *glyph, glyphs)?;
    }
    push_text(output, "Q\n");
    Ok(())
}

fn emit_glyph_path(
    output: &mut Vec<u8>,
    run: &GlyphRun,
    glyph: skia_core::PositionedGlyph,
    glyphs: &dyn GlyphOutlineProvider,
) -> Result<(), DocumentError> {
    let outline = glyphs
        .glyph_outline(run.font(), glyph.glyph())
        .map_err(|_| DocumentError::new(DocumentErrorCode::UnsupportedText))?;
    let Some(outline) = outline else {
        return Ok(());
    };
    if outline.font() != run.font() || outline.glyph() != glyph.glyph() {
        return Err(DocumentError::new(DocumentErrorCode::InvalidResource));
    }
    let Some(path) = glyph_outline_path(run, glyph, &outline).map_err(map_skia_error)? else {
        return Ok(());
    };
    emit_path(output, &path)?;
    push_text(output, "f\n");
    Ok(())
}

fn emit_transform(output: &mut Vec<u8>, transform: Transform) {
    let c = transform.coefficients();
    push_text(
        output,
        &format!(
            "{} {} {} {} {} {} cm\n",
            pdf_scalar(c[0]),
            pdf_scalar(c[1]),
            pdf_scalar(c[2]),
            pdf_scalar(c[3]),
            pdf_scalar(c[4]),
            pdf_scalar(c[5])
        ),
    );
}

fn emit_rect(output: &mut Vec<u8>, rect: Rect) {
    push_text(
        output,
        &format!(
            "{} {} {} {} re\n",
            pdf_scalar(rect.left()),
            pdf_scalar(rect.top()),
            pdf_scalar(Scalar::from_bits(rect.right().bits() - rect.left().bits())),
            pdf_scalar(Scalar::from_bits(rect.bottom().bits() - rect.top().bits()))
        ),
    );
}

fn emit_page_rect(output: &mut Vec<u8>, size: PageSize) {
    push_text(
        output,
        &format!(
            "0 0 {} {} re\n",
            pdf_scalar(size.width),
            pdf_scalar(size.height)
        ),
    );
}

fn emit_path(output: &mut Vec<u8>, path: &Path) -> Result<(), DocumentError> {
    let mut current = None;
    let mut contour_start = None;
    for verb in path.verbs() {
        match *verb {
            PathVerb::MoveTo(point) => {
                push_text(
                    output,
                    &format!("{} {} m\n", pdf_scalar(point.x()), pdf_scalar(point.y())),
                );
                current = Some(point);
                contour_start = Some(point);
            }
            PathVerb::LineTo(point) => {
                require_current(current)?;
                push_text(
                    output,
                    &format!("{} {} l\n", pdf_scalar(point.x()), pdf_scalar(point.y())),
                );
                current = Some(point);
            }
            PathVerb::QuadTo(control, end) => {
                let start = require_current(current)?;
                let first = quadratic_control(start, control)?;
                let second = quadratic_control(end, control)?;
                push_text(
                    output,
                    &format!(
                        "{} {} {} {} {} {} c\n",
                        pdf_scalar(first.0),
                        pdf_scalar(first.1),
                        pdf_scalar(second.0),
                        pdf_scalar(second.1),
                        pdf_scalar(end.x()),
                        pdf_scalar(end.y())
                    ),
                );
                current = Some(end);
            }
            PathVerb::ConicTo(_, _, _) => {
                return Err(DocumentError::new(DocumentErrorCode::Unsupported));
            }
            PathVerb::CubicTo(first, second, end) => {
                require_current(current)?;
                push_text(
                    output,
                    &format!(
                        "{} {} {} {} {} {} c\n",
                        pdf_scalar(first.x()),
                        pdf_scalar(first.y()),
                        pdf_scalar(second.x()),
                        pdf_scalar(second.y()),
                        pdf_scalar(end.x()),
                        pdf_scalar(end.y())
                    ),
                );
                current = Some(end);
            }
            PathVerb::Close => {
                require_current(current)?;
                push_text(output, "h\n");
                current = contour_start;
            }
        }
    }
    Ok(())
}

fn require_current(point: Option<skia_core::Point>) -> Result<skia_core::Point, DocumentError> {
    point.ok_or(DocumentError::new(DocumentErrorCode::InvalidResource))
}

fn quadratic_control(
    endpoint: skia_core::Point,
    control: skia_core::Point,
) -> Result<(Scalar, Scalar), DocumentError> {
    Ok((
        two_thirds(endpoint.x(), control.x())?,
        two_thirds(endpoint.y(), control.y())?,
    ))
}

fn two_thirds(endpoint: Scalar, control: Scalar) -> Result<Scalar, DocumentError> {
    let value = i64::from(endpoint.bits())
        .checked_add(
            i64::from(control.bits() - endpoint.bits())
                .checked_mul(2)
                .ok_or(DocumentError::new(DocumentErrorCode::NumericOverflow))?
                / 3,
        )
        .ok_or(DocumentError::new(DocumentErrorCode::NumericOverflow))?;
    i32::try_from(value)
        .map(Scalar::from_bits)
        .map_err(|_| DocumentError::new(DocumentErrorCode::NumericOverflow))
}

fn cap_number(cap: StrokeCap) -> u8 {
    match cap {
        StrokeCap::Butt => 0,
        StrokeCap::Round => 1,
        StrokeCap::Square => 2,
    }
}

fn join_number(join: StrokeJoin) -> u8 {
    match join {
        StrokeJoin::Miter => 0,
        StrokeJoin::Round => 1,
        StrokeJoin::Bevel => 2,
    }
}

fn intern_gstate(
    resources: &mut Resources,
    value: ExtGState,
    limits: DocumentLimits,
) -> Result<usize, DocumentError> {
    if let Some(index) = resources.ext_gstates.iter().position(|item| *item == value) {
        return Ok(index);
    }
    ensure_resource_capacity(resources, limits)?;
    resources.ext_gstates.push(value);
    Ok(resources.ext_gstates.len() - 1)
}

fn intern_image(
    resources: &mut Resources,
    image: &Image,
    interpolate: bool,
    limits: DocumentLimits,
) -> Result<usize, DocumentError> {
    if let Some(index) = resources
        .images
        .iter()
        .position(|item| item.image == *image && item.interpolate == interpolate)
    {
        return Ok(index);
    }
    ensure_resource_capacity(resources, limits)?;
    resources.images.push(PdfImage {
        image: image.clone(),
        interpolate,
    });
    Ok(resources.images.len() - 1)
}

fn ensure_resource_capacity(
    resources: &Resources,
    limits: DocumentLimits,
) -> Result<(), DocumentError> {
    if resources.ext_gstates.len()
        + resources.images.len()
        + resources.gradients.len()
        + resources.forms.len()
        + resources.fonts.len()
        >= limits.max_resources
    {
        return Err(DocumentError::new(DocumentErrorCode::ResourceLimit));
    }
    Ok(())
}

fn intern_form(
    resources: &mut Resources,
    form: PdfForm,
    limits: DocumentLimits,
) -> Result<usize, DocumentError> {
    ensure_resource_capacity(resources, limits)?;
    resources.forms.push(form);
    Ok(resources.forms.len() - 1)
}

fn intern_font(
    resources: &mut Resources,
    font: PdfEmbeddedFont,
    limits: DocumentLimits,
) -> Result<usize, DocumentError> {
    if let Some(index) = resources
        .fonts
        .iter()
        .position(|candidate| candidate.font == font.font)
    {
        if resources.fonts[index].program != font.program {
            return Err(DocumentError::new(DocumentErrorCode::InvalidResource));
        }
        return Ok(index);
    }
    ensure_resource_capacity(resources, limits)?;
    resources.fonts.push(PdfFont {
        font: font.font,
        program: font.program,
    });
    Ok(resources.fonts.len() - 1)
}

fn push_unique(values: &mut Vec<usize>, value: usize) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn compile_raster_page(
    active: &ActivePage,
    glyphs: Option<&dyn GlyphOutlineProvider>,
    resources: &mut Resources,
    fallback: RasterFallback,
    limits: DocumentLimits,
) -> Result<PageData, DocumentError> {
    if active
        .lists
        .iter()
        .any(|entry| entry.structure_tag.is_some())
    {
        return Err(DocumentError::new(DocumentErrorCode::Unsupported));
    }
    for entry in &active.lists {
        let list = &entry.list;
        for command in list.commands() {
            if matches!(
                command,
                DrawCommand::DrawGlyphRun { .. } | DrawCommand::DrawPositionedGlyphRun { .. }
            ) && glyphs.is_none()
            {
                return Err(DocumentError::new(DocumentErrorCode::UnsupportedText));
            }
            if let DrawCommand::DrawImage { image, .. } = command {
                let image = list
                    .image(*image)
                    .ok_or(DocumentError::new(DocumentErrorCode::InvalidResource))?;
                if !matches!(image.color_space(), ColorSpace::Srgb) {
                    return Err(DocumentError::new(
                        DocumentErrorCode::UnsupportedColorProfile,
                    ));
                }
            }
        }
    }
    let width = raster_dimension(active.spec.size.width, fallback.dpi)?;
    let height = raster_dimension(active.spec.size.height, fallback.dpi)?;
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or(DocumentError::new(DocumentErrorCode::NumericOverflow))?;
    let bytes = pixels
        .checked_mul(4)
        .ok_or(DocumentError::new(DocumentErrorCode::NumericOverflow))?;
    if pixels > fallback.max_pixels || bytes > fallback.max_bytes {
        return Err(DocumentError::new(DocumentErrorCode::ResourceLimit));
    }
    let surface_limits =
        SurfaceLimits::new(fallback.max_pixels, fallback.max_bytes, 256).map_err(map_skia_error)?;
    let mut surface = Surface::new(width, height, surface_limits).map_err(map_skia_error)?;
    let scale = Scalar::from_ratio(i64::from(fallback.dpi), 72).map_err(map_skia_error)?;
    replay_scaled(&mut surface, active, Transform::scale(scale, scale), glyphs)?;
    let image = Image::from_rgba8(width, height, surface.pixels().to_vec())
        .map_err(|_| DocumentError::new(DocumentErrorCode::InvalidResource))?;
    let image_index = intern_image(resources, &image, true, limits)?;
    let gstate = intern_gstate(
        resources,
        ExtGState {
            alpha: u8::MAX,
            blend_mode: BlendMode::SourceOver,
        },
        limits,
    )?;
    let content = format!(
        "q\n1 0 0 -1 0 {} cm\nq\n/GS{gstate} gs\n{} 0 0 {} 0 0 cm\n/Im{image_index} Do\nQ\nQ\n",
        pdf_scalar(active.spec.size.height),
        pdf_scalar(active.spec.size.width),
        pdf_scalar(active.spec.size.height)
    )
    .into_bytes();
    Ok(PageData {
        spec: active.spec,
        content,
        ext_gstates: vec![gstate],
        images: vec![image_index],
        gradients: Vec::new(),
        forms: Vec::new(),
        fonts: Vec::new(),
        annotations: active.annotations.clone(),
        destinations: active.destinations.clone(),
        structure: Vec::new(),
    })
}

fn raster_dimension(points: Scalar, dpi: u16) -> Result<u32, DocumentError> {
    let numerator = i64::from(points.bits())
        .checked_mul(i64::from(dpi))
        .ok_or(DocumentError::new(DocumentErrorCode::NumericOverflow))?;
    let denominator = 72_i64 << 16;
    let pixels = numerator
        .checked_add(denominator - 1)
        .ok_or(DocumentError::new(DocumentErrorCode::NumericOverflow))?
        / denominator;
    u32::try_from(pixels.max(1)).map_err(|_| DocumentError::new(DocumentErrorCode::ResourceLimit))
}

fn replay_scaled(
    surface: &mut Surface,
    active: &ActivePage,
    device_scale: Transform,
    glyphs: Option<&dyn GlyphOutlineProvider>,
) -> Result<(), DocumentError> {
    let mut canvas = surface.canvas();
    if let Some(rect) = active.spec.content_box {
        canvas.set_transform(device_scale);
        canvas
            .clip_rect(ClipRect::new(rect))
            .map_err(map_skia_error)?;
    }
    for entry in &active.lists {
        let list = &entry.list;
        let mut logical = Transform::IDENTITY;
        let mut stack = Vec::new();
        canvas.set_transform(logical.concat(device_scale).map_err(map_skia_error)?);
        for command in list.commands() {
            match command {
                DrawCommand::Clear(color) => canvas.clear(*color),
                DrawCommand::Save => {
                    canvas.save().map_err(map_skia_error)?;
                    stack.push(logical);
                }
                DrawCommand::SaveLayer(options) => {
                    canvas.save_layer(options.clone()).map_err(map_skia_error)?;
                    stack.push(logical);
                }
                DrawCommand::Restore => {
                    canvas.restore().map_err(map_skia_error)?;
                    logical = stack
                        .pop()
                        .ok_or(DocumentError::new(DocumentErrorCode::InvalidState))?;
                }
                DrawCommand::ClipRect { rect, op } => canvas
                    .clip_rect_with_op(ClipRect::new(*rect), *op)
                    .map_err(map_skia_error)?,
                DrawCommand::ClipPath { path, rule, op } => canvas
                    .clip_path(
                        list.path(*path)
                            .ok_or(DocumentError::new(DocumentErrorCode::InvalidResource))?,
                        *rule,
                        *op,
                    )
                    .map_err(map_skia_error)?,
                DrawCommand::SetTransform(transform) => {
                    logical = *transform;
                    canvas.set_transform(logical.concat(device_scale).map_err(map_skia_error)?);
                }
                DrawCommand::ConcatTransform(transform) => {
                    logical = logical.concat(*transform).map_err(map_skia_error)?;
                    canvas.set_transform(logical.concat(device_scale).map_err(map_skia_error)?);
                }
                DrawCommand::FillRect { rect, paint } => canvas
                    .fill_rect(*rect, paint.clone())
                    .map_err(map_skia_error)?,
                DrawCommand::FillPath { path, rule, paint } => canvas
                    .fill_path(
                        list.path(*path)
                            .ok_or(DocumentError::new(DocumentErrorCode::InvalidResource))?,
                        *rule,
                        paint.clone(),
                    )
                    .map_err(map_skia_error)?,
                DrawCommand::StrokePath {
                    path,
                    options,
                    paint,
                } => canvas
                    .stroke_path_with_options(
                        list.path(*path)
                            .ok_or(DocumentError::new(DocumentErrorCode::InvalidResource))?,
                        options,
                        paint.clone(),
                    )
                    .map_err(map_skia_error)?,
                DrawCommand::DrawImage {
                    image,
                    destination,
                    opacity,
                    sampling,
                    paint,
                } => canvas
                    .draw_image_with_paint(
                        list.image(*image)
                            .ok_or(DocumentError::new(DocumentErrorCode::InvalidResource))?,
                        *destination,
                        *opacity,
                        paint.clone(),
                        *sampling,
                    )
                    .map_err(map_skia_error)?,
                DrawCommand::DrawGlyphRun { run, paint } => {
                    let glyphs =
                        glyphs.ok_or(DocumentError::new(DocumentErrorCode::UnsupportedText))?;
                    let glyphs = OutlineProviderRef(glyphs);
                    let run = list
                        .glyph_run(*run)
                        .ok_or(DocumentError::new(DocumentErrorCode::InvalidResource))?;
                    canvas
                        .draw_glyph_run(run, &glyphs, paint.clone())
                        .map_err(map_skia_error)?;
                }
                DrawCommand::DrawPositionedGlyphRun {
                    run,
                    origin,
                    offsets_x_bits,
                    paint,
                } => {
                    let glyphs =
                        glyphs.ok_or(DocumentError::new(DocumentErrorCode::UnsupportedText))?;
                    let glyphs = OutlineProviderRef(glyphs);
                    let run = list
                        .glyph_run(*run)
                        .ok_or(DocumentError::new(DocumentErrorCode::InvalidResource))?;
                    canvas
                        .draw_positioned_glyph_run(
                            run,
                            offsets_x_bits,
                            *origin,
                            &glyphs,
                            paint.clone(),
                        )
                        .map_err(map_skia_error)?;
                }
            }
        }
        if !stack.is_empty() {
            return Err(DocumentError::new(DocumentErrorCode::InvalidState));
        }
    }
    Ok(())
}

fn collect_destinations(
    pages: &[PageData],
    bookmarks: &[PdfBookmark],
) -> Result<BTreeMap<String, (usize, Point)>, DocumentError> {
    let mut destinations = BTreeMap::new();
    for (page_index, page) in pages.iter().enumerate() {
        for destination in &page.destinations {
            if destinations
                .insert(destination.name.clone(), (page_index, destination.point))
                .is_some()
            {
                return Err(DocumentError::new(DocumentErrorCode::InvalidDestination));
            }
        }
    }
    let names = destinations.keys().cloned().collect::<BTreeSet<_>>();
    for page in pages {
        for annotation in &page.annotations {
            if let PdfLinkTarget::NamedDestination(name) = &annotation.target
                && !names.contains(name)
            {
                return Err(DocumentError::new(DocumentErrorCode::InvalidDestination));
            }
        }
    }
    if bookmarks
        .iter()
        .any(|bookmark| !names.contains(&bookmark.destination))
    {
        return Err(DocumentError::new(DocumentErrorCode::InvalidDestination));
    }
    Ok(destinations)
}

fn annotation_dictionary(spec: PageSpec, annotation: &LinkAnnotation) -> String {
    let height = spec.size.height;
    let rect = annotation.rect;
    let action = match &annotation.target {
        PdfLinkTarget::Uri(uri) => format!("/A << /S /URI /URI {} >>", pdf_string(uri)),
        PdfLinkTarget::NamedDestination(name) => format!("/Dest {}", pdf_string(name)),
    };
    format!(
        "<< /Type /Annot /Subtype /Link /Rect [{} {} {} {}] /Border [0 0 0] /F 4 {action} >>",
        pdf_scalar(rect.left()),
        pdf_scalar(Scalar::from_bits(height.bits() - rect.bottom().bits())),
        pdf_scalar(rect.right()),
        pdf_scalar(Scalar::from_bits(height.bits() - rect.top().bits())),
    )
}

fn serialize_pdf<W: Write>(
    writer: W,
    pages: &[PageData],
    resources: &Resources,
    metadata: &DocumentMetadata,
    conformance: PdfConformance,
    limits: DocumentLimits,
    bookmarks: &[PdfBookmark],
) -> Result<W, DocumentError> {
    let mut next_object = 4_usize;
    let conformance_objects = if conformance == PdfConformance::PdfA2b {
        let xmp = next_object;
        let output_intent = xmp + 1;
        let icc = xmp + 2;
        next_object = icc + 1;
        Some((xmp, output_intent, icc))
    } else {
        None
    };
    let page_start = next_object;
    let content_start = page_start
        .checked_add(pages.len())
        .ok_or(DocumentError::new(DocumentErrorCode::NumericOverflow))?;
    let gstate_start = content_start
        .checked_add(pages.len())
        .ok_or(DocumentError::new(DocumentErrorCode::NumericOverflow))?;
    let gradient_start = gstate_start
        .checked_add(resources.ext_gstates.len())
        .ok_or(DocumentError::new(DocumentErrorCode::NumericOverflow))?;
    let form_start = gradient_start
        .checked_add(resources.gradients.len())
        .ok_or(DocumentError::new(DocumentErrorCode::NumericOverflow))?;
    let font_start = form_start
        .checked_add(resources.forms.len())
        .ok_or(DocumentError::new(DocumentErrorCode::NumericOverflow))?;
    let font_object_count = resources
        .fonts
        .len()
        .checked_mul(4)
        .ok_or(DocumentError::new(DocumentErrorCode::NumericOverflow))?;
    let image_start = font_start
        .checked_add(font_object_count)
        .ok_or(DocumentError::new(DocumentErrorCode::NumericOverflow))?;
    let mut font_objects = Vec::with_capacity(resources.fonts.len());
    for index in 0..resources.fonts.len() {
        let type0 = font_start
            .checked_add(index * 4)
            .ok_or(DocumentError::new(DocumentErrorCode::NumericOverflow))?;
        let cid = type0
            .checked_add(1)
            .ok_or(DocumentError::new(DocumentErrorCode::NumericOverflow))?;
        let descriptor = cid
            .checked_add(1)
            .ok_or(DocumentError::new(DocumentErrorCode::NumericOverflow))?;
        let program = descriptor
            .checked_add(1)
            .ok_or(DocumentError::new(DocumentErrorCode::NumericOverflow))?;
        font_objects.push((type0, cid, descriptor, program));
    }
    let mut image_objects = Vec::with_capacity(resources.images.len());
    next_object = image_start;
    for resource in &resources.images {
        let image = &resource.image;
        let rgb = next_object;
        next_object += 1;
        let mask = (!image
            .pixels()
            .chunks_exact(4)
            .all(|pixel| pixel[3] == u8::MAX))
        .then(|| {
            let object = next_object;
            next_object += 1;
            object
        });
        image_objects.push((rgb, mask));
    }
    let destinations = collect_destinations(pages, bookmarks)?;
    let mut annotation_objects = Vec::with_capacity(pages.len());
    for page in pages {
        let mut objects = Vec::with_capacity(page.annotations.len());
        for _ in &page.annotations {
            objects.push(next_object);
            next_object = next_object
                .checked_add(1)
                .ok_or(DocumentError::new(DocumentErrorCode::NumericOverflow))?;
        }
        annotation_objects.push(objects);
    }
    let outline_objects = if bookmarks.is_empty() {
        None
    } else {
        let root = next_object;
        next_object = next_object
            .checked_add(1)
            .ok_or(DocumentError::new(DocumentErrorCode::NumericOverflow))?;
        let mut items = Vec::with_capacity(bookmarks.len());
        for _ in bookmarks {
            items.push(next_object);
            next_object = next_object
                .checked_add(1)
                .ok_or(DocumentError::new(DocumentErrorCode::NumericOverflow))?;
        }
        Some((root, items))
    };
    let structure_objects = if pages.iter().all(|page| page.structure.is_empty()) {
        None
    } else {
        let root = next_object;
        let parent_tree = root
            .checked_add(1)
            .ok_or(DocumentError::new(DocumentErrorCode::NumericOverflow))?;
        next_object = parent_tree
            .checked_add(1)
            .ok_or(DocumentError::new(DocumentErrorCode::NumericOverflow))?;
        let mut entries = Vec::with_capacity(pages.len());
        for page in pages {
            let mut page_entries = Vec::with_capacity(page.structure.len());
            for _ in &page.structure {
                page_entries.push(next_object);
                next_object = next_object
                    .checked_add(1)
                    .ok_or(DocumentError::new(DocumentErrorCode::NumericOverflow))?;
            }
            entries.push(page_entries);
        }
        Some((root, parent_tree, entries))
    };
    let object_count = next_object - 1;
    if object_count > limits.max_objects {
        return Err(DocumentError::new(DocumentErrorCode::ResourceLimit));
    }

    let mut bodies = vec![Vec::new(); object_count + 1];
    let mut catalog = String::from("<< /Type /Catalog /Pages 2 0 R");
    if let Some((xmp, output_intent, _)) = conformance_objects {
        catalog.push_str(&format!(
            " /Metadata {xmp} 0 R /OutputIntents [{output_intent} 0 R]"
        ));
    }
    if let Some((outline_root, _)) = &outline_objects {
        catalog.push_str(&format!(
            " /Outlines {outline_root} 0 R /PageMode /UseOutlines"
        ));
    }
    if let Some((structure_root, _, _)) = &structure_objects {
        catalog.push_str(&format!(
            " /MarkInfo << /Marked true >> /StructTreeRoot {structure_root} 0 R"
        ));
    }
    if !destinations.is_empty() {
        catalog.push_str(" /Names << /Dests << /Names [");
        for (name, (page, point)) in &destinations {
            let height = pages[*page].spec.size.height;
            catalog.push_str(&format!(
                " {} [{} 0 R /XYZ {} {} null]",
                pdf_string(name),
                page_start + page,
                pdf_scalar(point.x()),
                pdf_scalar(Scalar::from_bits(height.bits() - point.y().bits())),
            ));
        }
        catalog.push_str(" ] >> >>");
    }
    catalog.push_str(" >>");
    bodies[1] = catalog.into_bytes();
    let kids = (0..pages.len())
        .map(|index| format!("{} 0 R", page_start + index))
        .collect::<Vec<_>>()
        .join(" ");
    bodies[2] = format!("<< /Type /Pages /Count {} /Kids [{kids}] >>", pages.len()).into_bytes();
    bodies[3] = info_dictionary(metadata).into_bytes();
    if let Some((xmp_object, output_intent_object, icc_object)) = conformance_objects {
        let xmp = xmp_metadata(metadata)?;
        bodies[xmp_object] = stream_object("/Type /Metadata /Subtype /XML", xmp.as_bytes());
        let icc = ColorSpace::srgb_icc_profile()
            .map_err(|_| DocumentError::new(DocumentErrorCode::UnsupportedColorProfile))?;
        let icc = zlib_compress(&icc)?;
        bodies[icc_object] = stream_object("/N 3 /Alternate /DeviceRGB /Filter /FlateDecode", &icc);
        bodies[output_intent_object] = format!(
            "<< /Type /OutputIntent /S /GTS_PDFA1 /OutputConditionIdentifier {} /Info {} /RegistryName {} /DestOutputProfile {icc_object} 0 R >>",
            pdf_string("sRGB IEC61966-2.1"),
            pdf_string("sRGB IEC61966-2.1"),
            pdf_string("http://www.color.org"),
        )
        .into_bytes();
    }

    for (index, page) in pages.iter().enumerate() {
        let page_object = page_start + index;
        let content_object = content_start + index;
        let resource_dictionary = resource_dictionary(
            &page.ext_gstates,
            &page.images,
            &page.gradients,
            &page.forms,
            &page.fonts,
            gstate_start,
            gradient_start,
            form_start,
            &font_objects,
            &image_objects,
        );
        let annotations = if annotation_objects[index].is_empty() {
            String::new()
        } else {
            let values = annotation_objects[index]
                .iter()
                .map(|object| format!("{object} 0 R"))
                .collect::<Vec<_>>()
                .join(" ");
            format!(" /Annots [{values}]")
        };
        let struct_parents = if !page.structure.is_empty() {
            format!(" /StructParents {index}")
        } else {
            String::new()
        };
        bodies[page_object] = format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources {resource_dictionary} /Contents {content_object} 0 R{annotations}{struct_parents} >>",
            pdf_scalar(page.spec.size.width),
            pdf_scalar(page.spec.size.height)
        )
        .into_bytes();
        bodies[content_object] = stream_object("", &page.content);
    }
    for (index, gstate) in resources.ext_gstates.iter().enumerate() {
        let blend = pdf_blend_name(gstate.blend_mode)
            .ok_or(DocumentError::new(DocumentErrorCode::Unsupported))?;
        bodies[gstate_start + index] = format!(
            "<< /Type /ExtGState /ca {} /CA {} /BM /{blend} >>",
            pdf_channel(gstate.alpha),
            pdf_channel(gstate.alpha)
        )
        .into_bytes();
    }
    for (index, gradient) in resources.gradients.iter().enumerate() {
        bodies[gradient_start + index] = pdf_gradient_dictionary(*gradient)?.into_bytes();
    }
    for (index, form) in resources.forms.iter().enumerate() {
        let resource_dictionary = resource_dictionary(
            &form.ext_gstates,
            &form.images,
            &form.gradients,
            &form.forms,
            &form.fonts,
            gstate_start,
            gradient_start,
            form_start,
            &font_objects,
            &image_objects,
        );
        let dictionary = format!(
            "/Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 {} {}] /Group << /S /Transparency /I true /K false >> /Resources {resource_dictionary}",
            pdf_scalar(form.size.width),
            pdf_scalar(form.size.height),
        );
        bodies[form_start + index] = stream_object(&dictionary, &form.content);
    }
    for (index, font) in resources.fonts.iter().enumerate() {
        let (type0, cid, descriptor, program) = font_objects[index];
        let name = format!("SkiaFont{}", font.font.value());
        bodies[type0] = format!(
            "<< /Type /Font /Subtype /Type0 /BaseFont /{name} /Encoding /Identity-H /DescendantFonts [{cid} 0 R] >>"
        )
        .into_bytes();
        bodies[cid] = format!(
            "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /{name} /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor {descriptor} 0 R /CIDToGIDMap /Identity /DW 1000 >>"
        )
        .into_bytes();
        bodies[descriptor] = format!(
            "<< /Type /FontDescriptor /FontName /{name} /Flags 4 /FontBBox [0 -200 1000 1000] /ItalicAngle 0 /Ascent 800 /Descent -200 /CapHeight 700 /StemV 80 /FontFile2 {program} 0 R >>"
        )
        .into_bytes();
        let compressed = zlib_compress(&font.program)?;
        bodies[program] = stream_object(
            &format!("/Length1 {} /Filter /FlateDecode", font.program.len()),
            &compressed,
        );
    }
    for (index, resource) in resources.images.iter().enumerate() {
        let image = &resource.image;
        let (rgb_object, mask_object) = image_objects[index];
        let mut rgb = Vec::with_capacity(image.pixels().len() / 4 * 3);
        let mut alpha = Vec::with_capacity(image.pixels().len() / 4);
        for pixel in image.pixels().chunks_exact(4) {
            rgb.extend_from_slice(&pixel[..3]);
            alpha.push(pixel[3]);
        }
        let rgb = zlib_compress(&rgb)?;
        let mask_entry =
            mask_object.map_or(String::new(), |object| format!(" /SMask {object} 0 R"));
        let dictionary = format!(
            "/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Interpolate {}{mask_entry}",
            image.width(),
            image.height(),
            resource.interpolate
        );
        bodies[rgb_object] = stream_object(&dictionary, &rgb);
        if let Some(mask_object) = mask_object {
            let alpha = zlib_compress(&alpha)?;
            let dictionary = format!(
                "/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode",
                image.width(),
                image.height()
            );
            bodies[mask_object] = stream_object(&dictionary, &alpha);
        }
    }
    for (page_index, page) in pages.iter().enumerate() {
        for (annotation, object) in page.annotations.iter().zip(&annotation_objects[page_index]) {
            bodies[*object] = annotation_dictionary(page.spec, annotation).into_bytes();
        }
    }
    if let Some((outline_root, outline_items)) = outline_objects {
        let first = outline_items[0];
        let last = outline_items[outline_items.len() - 1];
        bodies[outline_root] = format!(
            "<< /Type /Outlines /First {first} 0 R /Last {last} 0 R /Count {} >>",
            outline_items.len()
        )
        .into_bytes();
        for (index, (bookmark, object)) in bookmarks.iter().zip(outline_items).enumerate() {
            let previous = (index > 0).then(|| format!(" /Prev {} 0 R", object - 1));
            let next = (index + 1 < bookmarks.len()).then(|| format!(" /Next {} 0 R", object + 1));
            bodies[object] = format!(
                "<< /Title {} /Parent {outline_root} 0 R /Dest {}{}{} >>",
                pdf_string(&bookmark.title),
                pdf_string(&bookmark.destination),
                previous.unwrap_or_default(),
                next.unwrap_or_default(),
            )
            .into_bytes();
        }
    }
    if let Some((structure_root, parent_tree, structure_entries)) = structure_objects {
        let children = structure_entries
            .iter()
            .flatten()
            .map(|object| format!("{object} 0 R"))
            .collect::<Vec<_>>()
            .join(" ");
        bodies[structure_root] =
            format!("<< /Type /StructTreeRoot /K [{children}] /ParentTree {parent_tree} 0 R >>")
                .into_bytes();
        let mut parent_tree_numbers = String::new();
        for (page_index, entries) in structure_entries.iter().enumerate() {
            if entries.is_empty() {
                continue;
            }
            let values = entries
                .iter()
                .map(|object| format!("{object} 0 R"))
                .collect::<Vec<_>>()
                .join(" ");
            parent_tree_numbers.push_str(&format!(" {page_index} [{values}]"));
        }
        bodies[parent_tree] = format!("<< /Nums [{parent_tree_numbers} ] >>").into_bytes();
        for (page_index, (page, entries)) in pages.iter().zip(structure_entries).enumerate() {
            for (entry, object) in page.structure.iter().zip(entries) {
                bodies[object] = format!(
                    "<< /Type /StructElem /S /{} /P {structure_root} 0 R /Pg {} 0 R /K {} >>",
                    entry.tag.pdf_name(),
                    page_start + page_index,
                    entry.mcid,
                )
                .into_bytes();
            }
        }
    }

    let document_id = (conformance == PdfConformance::PdfA2b).then(|| {
        let mut digest = Sha256::new();
        for body in bodies.iter().skip(1) {
            digest.update(body);
        }
        format!("{:x}", digest.finalize())
    });
    let mut sink = LimitedWriter::new(writer, limits.max_output_bytes);
    sink.write_bytes(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n")?;
    let mut offsets = vec![0_u64; object_count + 1];
    for object in 1..=object_count {
        offsets[object] = sink.position;
        sink.write_bytes(format!("{object} 0 obj\n").as_bytes())?;
        sink.write_bytes(&bodies[object])?;
        sink.write_bytes(b"\nendobj\n")?;
    }
    let xref = sink.position;
    sink.write_bytes(format!("xref\n0 {}\n", object_count + 1).as_bytes())?;
    sink.write_bytes(b"0000000000 65535 f \n")?;
    for offset in offsets.iter().skip(1) {
        if *offset > 9_999_999_999 {
            return Err(DocumentError::new(DocumentErrorCode::ResourceLimit));
        }
        sink.write_bytes(format!("{offset:010} 00000 n \n").as_bytes())?;
    }
    let id = document_id.map_or(String::new(), |id| format!(" /ID [<{id}> <{id}>]"));
    sink.write_bytes(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R /Info 3 0 R{id} >>\nstartxref\n{xref}\n%%EOF\n",
            object_count + 1
        )
        .as_bytes(),
    )?;
    Ok(sink.writer)
}

#[allow(clippy::too_many_arguments)]
fn resource_dictionary(
    ext_gstates: &[usize],
    images: &[usize],
    gradients: &[usize],
    forms: &[usize],
    fonts: &[usize],
    gstate_start: usize,
    gradient_start: usize,
    form_start: usize,
    font_objects: &[(usize, usize, usize, usize)],
    image_objects: &[(usize, Option<usize>)],
) -> String {
    let mut dictionary = String::from("<<");
    if !ext_gstates.is_empty() {
        dictionary.push_str(" /ExtGState <<");
        for value in ext_gstates {
            dictionary.push_str(&format!(" /GS{value} {} 0 R", gstate_start + value));
        }
        dictionary.push_str(" >>");
    }
    if !images.is_empty() || !forms.is_empty() {
        dictionary.push_str(" /XObject <<");
        for value in images {
            dictionary.push_str(&format!(" /Im{value} {} 0 R", image_objects[*value].0));
        }
        for value in forms {
            dictionary.push_str(&format!(" /Fm{value} {} 0 R", form_start + value));
        }
        dictionary.push_str(" >>");
    }
    if !fonts.is_empty() {
        dictionary.push_str(" /Font <<");
        for value in fonts {
            dictionary.push_str(&format!(" /F{value} {} 0 R", font_objects[*value].0));
        }
        dictionary.push_str(" >>");
    }
    if !gradients.is_empty() {
        dictionary.push_str(" /Shading <<");
        for value in gradients {
            dictionary.push_str(&format!(" /Sh{value} {} 0 R", gradient_start + value));
        }
        dictionary.push_str(" >>");
    }
    dictionary.push_str(" >>");
    dictionary
}

fn info_dictionary(metadata: &DocumentMetadata) -> String {
    let mut info = String::from("<<");
    append_info(&mut info, "Title", metadata.title.as_deref());
    append_info(&mut info, "Author", metadata.author.as_deref());
    append_info(&mut info, "Subject", metadata.subject.as_deref());
    append_info(&mut info, "Keywords", metadata.keywords.as_deref());
    append_info(&mut info, "Creator", metadata.creator.as_deref());
    append_info(&mut info, "Producer", Some(pdf_producer(metadata)));
    append_info_date(&mut info, "CreationDate", metadata.creation);
    append_info_date(&mut info, "ModDate", metadata.modified);
    info.push_str(" >>");
    info
}

fn pdf_producer(metadata: &DocumentMetadata) -> &str {
    metadata.producer.as_deref().unwrap_or("skia-pdf 0.1")
}

fn append_info_date(output: &mut String, key: &str, value: Option<PdfDateTime>) {
    if let Some(value) = value {
        output.push_str(&format!(" /{key} {}", pdf_string(&value.pdf_date())));
    }
}

impl PdfDateTime {
    fn pdf_date(self) -> String {
        format!(
            "D:{:04}{:02}{:02}{:02}{:02}{:02}Z",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }

    fn xmp_date(self) -> String {
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

fn xmp_metadata(metadata: &DocumentMetadata) -> Result<String, DocumentError> {
    let creation = metadata
        .creation
        .ok_or(DocumentError::new(DocumentErrorCode::InvalidMetadata))?;
    let modified = metadata
        .modified
        .ok_or(DocumentError::new(DocumentErrorCode::InvalidMetadata))?;
    let title = metadata.title.as_deref().map(xml_text).unwrap_or_default();
    let author = metadata.author.as_deref().map(xml_text).unwrap_or_default();
    let subject = metadata
        .subject
        .as_deref()
        .map(xml_text)
        .unwrap_or_default();
    let keywords = metadata
        .keywords
        .as_deref()
        .map(xml_text)
        .unwrap_or_default();
    let creator = metadata
        .creator
        .as_deref()
        .map(xml_text)
        .unwrap_or_default();
    let producer = xml_text(pdf_producer(metadata));
    Ok(format!(
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n<rdf:Description rdf:about=\"\" xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\" pdfaid:part=\"2\" pdfaid:conformance=\"B\"/>\n<rdf:Description rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\" xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\">\n<dc:format>application/pdf</dc:format><dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">{title}</rdf:li></rdf:Alt></dc:title><dc:creator><rdf:Seq><rdf:li>{author}</rdf:li></rdf:Seq></dc:creator><dc:description><rdf:Alt><rdf:li xml:lang=\"x-default\">{subject}</rdf:li></rdf:Alt></dc:description><pdf:Keywords>{keywords}</pdf:Keywords><pdf:Producer>{producer}</pdf:Producer><xmp:CreatorTool>{creator}</xmp:CreatorTool><xmp:CreateDate>{}</xmp:CreateDate><xmp:ModifyDate>{}</xmp:ModifyDate><xmp:MetadataDate>{}</xmp:MetadataDate>\n</rdf:Description>\n</rdf:RDF>\n</x:xmpmeta>\n<?xpacket end=\"w\"?>",
        creation.xmp_date(),
        modified.xmp_date(),
        modified.xmp_date(),
    ))
}

fn xml_text(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '\"' => output.push_str("&quot;"),
            '\'' => output.push_str("&apos;"),
            _ => output.push(character),
        }
    }
    output
}

fn append_info(output: &mut String, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        output.push_str(&format!(" /{key} {}", pdf_string(value)));
    }
}

fn pdf_string(value: &str) -> String {
    if value.is_ascii() {
        let mut escaped = String::with_capacity(value.len());
        for byte in value.bytes() {
            match byte {
                b'\\' | b'(' | b')' => {
                    escaped.push('\\');
                    escaped.push(char::from(byte));
                }
                b'\n' => escaped.push_str("\\n"),
                b'\r' => escaped.push_str("\\r"),
                b'\t' => escaped.push_str("\\t"),
                8 => escaped.push_str("\\b"),
                12 => escaped.push_str("\\f"),
                0x20..=0x7E => escaped.push(char::from(byte)),
                _ => escaped.push_str(&format!("\\{byte:03o}")),
            }
        }
        format!("({escaped})")
    } else {
        let mut hex = String::from("<FEFF");
        for unit in value.encode_utf16() {
            hex.push_str(&format!("{unit:04X}"));
        }
        hex.push('>');
        hex
    }
}

fn stream_object(dictionary: &str, data: &[u8]) -> Vec<u8> {
    let mut output = format!("<< {dictionary} /Length {} >>\nstream\n", data.len()).into_bytes();
    output.extend_from_slice(data);
    output.extend_from_slice(b"\nendstream");
    output
}

fn zlib_compress(data: &[u8]) -> Result<Vec<u8>, DocumentError> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(6));
    encoder
        .write_all(data)
        .map_err(|error| DocumentError::io(&error))?;
    encoder.finish().map_err(|error| DocumentError::io(&error))
}

struct LimitedWriter<W> {
    writer: W,
    position: u64,
    limit: u64,
}

impl<W: Write> LimitedWriter<W> {
    const fn new(writer: W, limit: u64) -> Self {
        Self {
            writer,
            position: 0,
            limit,
        }
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), DocumentError> {
        let length = u64::try_from(bytes.len())
            .map_err(|_| DocumentError::new(DocumentErrorCode::ResourceLimit))?;
        let end = self
            .position
            .checked_add(length)
            .ok_or(DocumentError::new(DocumentErrorCode::ResourceLimit))?;
        if end > self.limit {
            return Err(DocumentError::new(DocumentErrorCode::ResourceLimit));
        }
        self.writer
            .write_all(bytes)
            .map_err(|error| DocumentError::io(&error))?;
        self.position = end;
        Ok(())
    }
}

fn pdf_scalar(value: Scalar) -> String {
    let bits = i64::from(value.bits());
    let negative = bits < 0;
    let magnitude = bits.unsigned_abs();
    let whole = magnitude >> 16;
    let fraction = magnitude & 0xFFFF;
    if fraction == 0 {
        return format!("{}{whole}", if negative { "-" } else { "" });
    }
    let decimal = fraction * 152_587_890_625_u64;
    let trimmed = format!("{decimal:016}").trim_end_matches('0').to_owned();
    format!("{}{whole}.{trimmed}", if negative { "-" } else { "" })
}

fn pdf_channel(value: u8) -> String {
    if value == 0 {
        "0".to_owned()
    } else if value == u8::MAX {
        "1".to_owned()
    } else {
        let scaled = (u32::from(value) * 1_000_000 + 127) / 255;
        format!("0.{scaled:06}").trim_end_matches('0').to_owned()
    }
}

fn pdf_gradient_dictionary(gradient: Gradient) -> Result<String, DocumentError> {
    let (shading_type, coordinates) = match gradient.geometry() {
        GradientGeometry::Linear { start, end } => (
            2_u8,
            format!(
                "{} {} {} {}",
                pdf_scalar(start.x()),
                pdf_scalar(start.y()),
                pdf_scalar(end.x()),
                pdf_scalar(end.y())
            ),
        ),
        GradientGeometry::Radial { center, radius } => (
            3_u8,
            format!(
                "{} {} 0 {} {} {}",
                pdf_scalar(center.x()),
                pdf_scalar(center.y()),
                pdf_scalar(center.x()),
                pdf_scalar(center.y()),
                pdf_scalar(radius)
            ),
        ),
    };
    Ok(format!(
        "<< /ShadingType {shading_type} /ColorSpace /DeviceRGB /Coords [{coordinates}] /Function {} /Extend [true true] >>",
        pdf_gradient_function(gradient)?
    ))
}

fn pdf_gradient_function(gradient: Gradient) -> Result<String, DocumentError> {
    let stops = gradient.stops();
    if stops.len() < 2
        || stops
            .windows(2)
            .any(|pair| pair[0].offset() >= pair[1].offset())
        || stops.iter().any(|stop| stop.color().alpha() != u8::MAX)
    {
        return Err(DocumentError::new(DocumentErrorCode::Unsupported));
    }
    let functions = stops
        .windows(2)
        .map(|pair| {
            let start = pair[0].color();
            let end = pair[1].color();
            format!(
                "<< /FunctionType 2 /Domain [0 1] /C0 [{} {} {}] /C1 [{} {} {}] /N 1 >>",
                pdf_channel(start.red()),
                pdf_channel(start.green()),
                pdf_channel(start.blue()),
                pdf_channel(end.red()),
                pdf_channel(end.green()),
                pdf_channel(end.blue()),
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    let bounds = stops[1..stops.len() - 1]
        .iter()
        .map(|stop| pdf_scalar(stop.offset()))
        .collect::<Vec<_>>()
        .join(" ");
    let encode = std::iter::repeat_n("0 1", stops.len() - 1)
        .collect::<Vec<_>>()
        .join(" ");
    Ok(format!(
        "<< /FunctionType 3 /Domain [0 1] /Functions [{functions}] /Bounds [{bounds}] /Encode [{encode}] >>"
    ))
}

fn pdf_blend_name(mode: BlendMode) -> Option<&'static str> {
    Some(match mode {
        BlendMode::SourceOver => "Normal",
        BlendMode::Multiply => "Multiply",
        BlendMode::Screen => "Screen",
        BlendMode::Overlay => "Overlay",
        BlendMode::Darken => "Darken",
        BlendMode::Lighten => "Lighten",
        BlendMode::ColorDodge => "ColorDodge",
        BlendMode::ColorBurn => "ColorBurn",
        BlendMode::HardLight => "HardLight",
        BlendMode::SoftLight => "SoftLight",
        BlendMode::Difference => "Difference",
        BlendMode::Exclusion => "Exclusion",
        BlendMode::Hue => "Hue",
        BlendMode::Saturation => "Saturation",
        BlendMode::Color => "Color",
        BlendMode::Luminosity => "Luminosity",
        _ => return None,
    })
}

fn multiply_alpha(first: u8, second: u8) -> u8 {
    ((u16::from(first) * u16::from(second) + 127) / 255) as u8
}

fn push_text(output: &mut Vec<u8>, text: &str) {
    output.extend_from_slice(text.as_bytes());
}

fn map_skia_error(error: SkiaError) -> DocumentError {
    use skia_core::SkiaErrorCode;
    let code = match error.code() {
        SkiaErrorCode::ResourceLimit | SkiaErrorCode::AllocationFailed => {
            DocumentErrorCode::ResourceLimit
        }
        SkiaErrorCode::NumericOverflow => DocumentErrorCode::NumericOverflow,
        SkiaErrorCode::InvalidResource => DocumentErrorCode::InvalidResource,
        SkiaErrorCode::InvalidLimits => DocumentErrorCode::InvalidLimits,
        _ => DocumentErrorCode::Unsupported,
    };
    DocumentError::new(code)
}
