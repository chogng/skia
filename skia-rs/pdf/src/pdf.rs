use std::{
    fmt,
    io::{self, Write},
};

use flate2::{Compression, write::ZlibEncoder};
use skia_core::{
    BlendMode, ClipOp, DisplayList, DrawCommand, FillRule, Paint, Path, PathVerb, Rect, Scalar,
    SkiaError, StrokeAlign, StrokeCap, StrokeJoin, Transform,
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
}

type DocumentMetadata = PdfMetadata;

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

/// PDF backend configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfOptions {
    /// Reproducible Info dictionary fields.
    pub metadata: DocumentMetadata,
    /// Construction and output ceilings.
    pub limits: DocumentLimits,
    /// Unsupported drawing policy.
    pub unsupported_behavior: UnsupportedBehavior,
    /// Whole-page fallback policy used when enabled.
    pub raster_fallback: RasterFallback,
}

impl Default for PdfOptions {
    fn default() -> Self {
        Self {
            metadata: DocumentMetadata::default(),
            limits: DocumentLimits::default(),
            unsupported_behavior: UnsupportedBehavior::Error,
            raster_fallback: RasterFallback::default(),
        }
    }
}

#[derive(Clone)]
struct ActivePage {
    spec: PageSpec,
    lists: Vec<DisplayList>,
    command_count: usize,
}

#[derive(Clone)]
struct PageData {
    spec: PageSpec,
    content: Vec<u8>,
    ext_gstates: Vec<usize>,
    images: Vec<usize>,
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
}

#[derive(Clone, Eq, PartialEq)]
struct PdfImage {
    image: Image,
    interpolate: bool,
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
}

impl<W: Write> PdfDocument<W> {
    /// Creates an empty deterministic PDF document.
    pub fn new(writer: W, options: PdfOptions) -> Result<Self, DocumentError> {
        options.limits.validate()?;
        options.raster_fallback.validate()?;
        Ok(Self {
            writer,
            options,
            pages: Vec::new(),
            active: None,
            resources: Resources::default(),
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
        active.lists.push(list.clone());
        Ok(())
    }

    /// Completes the active page and resolves its native or fallback content.
    pub fn end_page(&mut self) -> Result<(), DocumentError> {
        let active = self
            .active
            .as_ref()
            .ok_or(DocumentError::new(DocumentErrorCode::InvalidState))?;
        let mut resources = self.resources.clone();
        let native = compile_native_page(active, &mut resources, self.options.limits);
        let page = match native {
            Ok(page) => page,
            Err(error)
                if error.code() == DocumentErrorCode::Unsupported
                    && self.options.unsupported_behavior == UnsupportedBehavior::RasterizePage =>
            {
                compile_raster_page(
                    active,
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
            self.options.limits,
        )
    }

    /// Aborts construction without writing bytes and returns the destination.
    pub fn abort(self) -> W {
        self.writer
    }
}

fn compile_native_page(
    active: &ActivePage,
    resources: &mut Resources,
    limits: DocumentLimits,
) -> Result<PageData, DocumentError> {
    let mut content = Vec::new();
    let mut used_gstates = Vec::new();
    let mut used_images = Vec::new();
    push_text(&mut content, "q\n");
    push_text(
        &mut content,
        &format!("1 0 0 -1 0 {} cm\n", pdf_scalar(active.spec.size.height)),
    );
    if let Some(rect) = active.spec.content_box {
        emit_rect(&mut content, rect);
        push_text(&mut content, "W n\n");
    }
    for (list_index, list) in active.lists.iter().enumerate() {
        push_text(&mut content, "q\n");
        compile_list(
            list,
            list_index == 0,
            active.spec,
            &mut content,
            resources,
            &mut used_gstates,
            &mut used_images,
            limits,
        )?;
        push_text(&mut content, "Q\n");
    }
    push_text(&mut content, "Q\n");
    Ok(PageData {
        spec: active.spec,
        content,
        ext_gstates: used_gstates,
        images: used_images,
    })
}

#[allow(clippy::too_many_arguments)]
fn compile_list(
    list: &DisplayList,
    first_list: bool,
    spec: PageSpec,
    output: &mut Vec<u8>,
    resources: &mut Resources,
    used_gstates: &mut Vec<usize>,
    used_images: &mut Vec<usize>,
    limits: DocumentLimits,
) -> Result<(), DocumentError> {
    let mut transform = Transform::IDENTITY;
    let mut transforms = Vec::new();
    for (command_index, command) in list.commands().iter().enumerate() {
        match command {
            DrawCommand::Clear(color) => {
                if !first_list || command_index != 0 {
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
            DrawCommand::SaveLayer(_) => {
                return Err(DocumentError::new(DocumentErrorCode::Unsupported));
            }
            DrawCommand::Restore => {
                transform = transforms
                    .pop()
                    .ok_or(DocumentError::new(DocumentErrorCode::InvalidState))?;
                push_text(output, "Q\n");
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
                emit_paint(output, paint, false, resources, used_gstates, limits)?;
                emit_rect(output, *rect);
                push_text(output, "f\n");
            }
            DrawCommand::FillPath { path, rule, paint } => {
                emit_paint(output, paint, false, resources, used_gstates, limits)?;
                emit_path(
                    output,
                    list.path(*path)
                        .ok_or(DocumentError::new(DocumentErrorCode::InvalidResource))?,
                )?;
                push_text(
                    output,
                    if *rule == FillRule::EvenOdd {
                        "f*\n"
                    } else {
                        "f\n"
                    },
                );
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
            DrawCommand::DrawGlyphRun { .. } | DrawCommand::DrawPositionedGlyphRun { .. } => {
                return Err(DocumentError::new(DocumentErrorCode::UnsupportedText));
            }
        }
    }
    if !transforms.is_empty() {
        return Err(DocumentError::new(DocumentErrorCode::InvalidState));
    }
    Ok(())
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
    if resources.ext_gstates.len() + resources.images.len() >= limits.max_resources {
        return Err(DocumentError::new(DocumentErrorCode::ResourceLimit));
    }
    Ok(())
}

fn push_unique(values: &mut Vec<usize>, value: usize) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn compile_raster_page(
    active: &ActivePage,
    resources: &mut Resources,
    fallback: RasterFallback,
    limits: DocumentLimits,
) -> Result<PageData, DocumentError> {
    for list in &active.lists {
        for command in list.commands() {
            if matches!(
                command,
                DrawCommand::DrawGlyphRun { .. } | DrawCommand::DrawPositionedGlyphRun { .. }
            ) {
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
    replay_scaled(&mut surface, active, Transform::scale(scale, scale))?;
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
) -> Result<(), DocumentError> {
    let mut canvas = surface.canvas();
    if let Some(rect) = active.spec.content_box {
        canvas.set_transform(device_scale);
        canvas
            .clip_rect(ClipRect::new(rect))
            .map_err(map_skia_error)?;
    }
    for list in &active.lists {
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
                DrawCommand::DrawGlyphRun { .. } | DrawCommand::DrawPositionedGlyphRun { .. } => {
                    return Err(DocumentError::new(DocumentErrorCode::UnsupportedText));
                }
            }
        }
        if !stack.is_empty() {
            return Err(DocumentError::new(DocumentErrorCode::InvalidState));
        }
    }
    Ok(())
}

fn serialize_pdf<W: Write>(
    writer: W,
    pages: &[PageData],
    resources: &Resources,
    metadata: &DocumentMetadata,
    limits: DocumentLimits,
) -> Result<W, DocumentError> {
    let page_start = 4_usize;
    let content_start = page_start
        .checked_add(pages.len())
        .ok_or(DocumentError::new(DocumentErrorCode::NumericOverflow))?;
    let gstate_start = content_start
        .checked_add(pages.len())
        .ok_or(DocumentError::new(DocumentErrorCode::NumericOverflow))?;
    let image_start = gstate_start
        .checked_add(resources.ext_gstates.len())
        .ok_or(DocumentError::new(DocumentErrorCode::NumericOverflow))?;
    let mut image_objects = Vec::with_capacity(resources.images.len());
    let mut next_object = image_start;
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
    let object_count = next_object - 1;
    if object_count > limits.max_objects {
        return Err(DocumentError::new(DocumentErrorCode::ResourceLimit));
    }

    let mut bodies = vec![Vec::new(); object_count + 1];
    bodies[1] = b"<< /Type /Catalog /Pages 2 0 R >>".to_vec();
    let kids = (0..pages.len())
        .map(|index| format!("{} 0 R", page_start + index))
        .collect::<Vec<_>>()
        .join(" ");
    bodies[2] = format!("<< /Type /Pages /Count {} /Kids [{kids}] >>", pages.len()).into_bytes();
    bodies[3] = info_dictionary(metadata).into_bytes();

    for (index, page) in pages.iter().enumerate() {
        let page_object = page_start + index;
        let content_object = content_start + index;
        let mut resource_dictionary = String::from("<<");
        if !page.ext_gstates.is_empty() {
            resource_dictionary.push_str(" /ExtGState <<");
            for value in &page.ext_gstates {
                resource_dictionary.push_str(&format!(" /GS{value} {} 0 R", gstate_start + value));
            }
            resource_dictionary.push_str(" >>");
        }
        if !page.images.is_empty() {
            resource_dictionary.push_str(" /XObject <<");
            for value in &page.images {
                resource_dictionary
                    .push_str(&format!(" /Im{value} {} 0 R", image_objects[*value].0));
            }
            resource_dictionary.push_str(" >>");
        }
        resource_dictionary.push_str(" >>");
        bodies[page_object] = format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources {resource_dictionary} /Contents {content_object} 0 R >>",
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
    sink.write_bytes(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R /Info 3 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            object_count + 1
        )
        .as_bytes(),
    )?;
    Ok(sink.writer)
}

fn info_dictionary(metadata: &DocumentMetadata) -> String {
    let mut info = String::from("<<");
    append_info(&mut info, "Title", metadata.title.as_deref());
    append_info(&mut info, "Author", metadata.author.as_deref());
    append_info(&mut info, "Subject", metadata.subject.as_deref());
    append_info(&mut info, "Keywords", metadata.keywords.as_deref());
    append_info(&mut info, "Creator", metadata.creator.as_deref());
    append_info(
        &mut info,
        "Producer",
        Some(metadata.producer.as_deref().unwrap_or("skia-pdf 0.1")),
    );
    info.push_str(" >>");
    info
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
