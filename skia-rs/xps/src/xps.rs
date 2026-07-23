use std::{
    fmt,
    io::{self, Write},
};

use skia_codec::{
    CodecErrorCode, EncodeFormat, EncodeLimits, EncodeOptions, ImageAsset, ImageCodec, PngOptions,
};
use skia_core::{
    BlendMode, Color, DisplayList, DrawCommand, FillRule, Paint, Path, PathVerb, Rect, Scalar,
    SkiaError, SkiaErrorCode, StrokeAlign, StrokeCap, StrokeJoin, Transform,
};
use skia_cpu::{ClipRect, Surface, SurfaceLimits};
use skia_image::{ColorSpace, Image};

use crate::opc::{self, OpcError, Part};

const XPS_NAMESPACE: &str = "http://schemas.microsoft.com/xps/2005/06";
const OPEN_XPS_NAMESPACE: &str = "http://schemas.openxps.org/oxps/v1.0";
const CONTENT_TYPES_NAMESPACE: &str =
    "http://schemas.openxmlformats.org/package/2006/content-types";
const RELATIONSHIPS_NAMESPACE: &str =
    "http://schemas.openxmlformats.org/package/2006/relationships";

/// XPS package dialect selected for serialization.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum XpsFormat {
    /// Microsoft XML Paper Specification 1.0, conventionally stored as `.xps`.
    Xps10,
    /// ECMA-388 OpenXPS, conventionally stored as `.oxps`.
    #[default]
    OpenXps,
}

impl XpsFormat {
    fn namespace(self) -> &'static str {
        match self {
            Self::Xps10 => XPS_NAMESPACE,
            Self::OpenXps => OPEN_XPS_NAMESPACE,
        }
    }

    /// Returns the conventional filename extension without a leading dot.
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Xps10 => "xps",
            Self::OpenXps => "oxps",
        }
    }

    /// Returns the conventional Internet media type.
    pub const fn media_type(self) -> &'static str {
        match self {
            Self::Xps10 => "application/vnd.ms-xpsdocument",
            Self::OpenXps => "application/oxps",
        }
    }
}

/// Stable machine-readable XPS failure code.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum XpsErrorCode {
    /// A page operation was attempted in the wrong lifecycle state.
    InvalidState,
    /// A page size or content rectangle was invalid.
    InvalidPage,
    /// A configured resource ceiling was invalid.
    InvalidLimits,
    /// A page, command, resource, package part, raster, or byte limit was exceeded.
    ResourceLimit,
    /// A display-list or OPC resource reference was invalid.
    InvalidResource,
    /// The native fixed-page mapping cannot preserve a drawing semantic.
    Unsupported,
    /// Text requires a font embedding and glyph mapping policy not yet exposed.
    UnsupportedText,
    /// An image uses a color profile outside the fixed-page output contract.
    UnsupportedColorProfile,
    /// Fixed-point, package-offset, or size arithmetic overflowed.
    NumericOverflow,
    /// The destination writer failed.
    Io,
}

/// Source-redacted XPS error with an optional I/O category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct XpsError {
    code: XpsErrorCode,
    io_kind: Option<io::ErrorKind>,
}

impl XpsError {
    const fn new(code: XpsErrorCode) -> Self {
        Self {
            code,
            io_kind: None,
        }
    }

    fn io(error: &io::Error) -> Self {
        Self {
            code: XpsErrorCode::Io,
            io_kind: Some(error.kind()),
        }
    }

    /// Returns the stable error category.
    pub const fn code(self) -> XpsErrorCode {
        self.code
    }

    /// Returns the destination I/O category, when applicable.
    pub const fn io_kind(self) -> Option<io::ErrorKind> {
        self.io_kind
    }
}

impl fmt::Display for XpsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.io_kind {
            Some(kind) => write!(formatter, "{:?} ({kind:?})", self.code),
            None => write!(formatter, "{:?}", self.code),
        }
    }
}

impl std::error::Error for XpsError {}

/// Positive fixed-page dimensions in XPS logical units.
///
/// One XPS logical unit is 1/96 inch.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct XpsPageSize {
    width: Scalar,
    height: Scalar,
}

impl XpsPageSize {
    /// Creates a positive page size.
    pub fn new(width: Scalar, height: Scalar) -> Result<Self, XpsError> {
        if width.bits() <= 0 || height.bits() <= 0 {
            return Err(XpsError::new(XpsErrorCode::InvalidPage));
        }
        Ok(Self { width, height })
    }

    /// Returns the page width in 1/96-inch logical units.
    pub const fn width(self) -> Scalar {
        self.width
    }

    /// Returns the page height in 1/96-inch logical units.
    pub const fn height(self) -> Scalar {
        self.height
    }
}

/// Fixed-page dimensions and an optional top-left content clip.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct XpsPageSpec {
    size: XpsPageSize,
    content_box: Option<Rect>,
}

impl XpsPageSpec {
    /// Creates a page without an additional content clip.
    pub const fn new(size: XpsPageSize) -> Self {
        Self {
            size,
            content_box: None,
        }
    }

    /// Adds a content box fully contained by the page.
    pub fn with_content_box(mut self, content_box: Rect) -> Result<Self, XpsError> {
        if content_box.left().bits() < 0
            || content_box.top().bits() < 0
            || content_box.right() > self.size.width
            || content_box.bottom() > self.size.height
        {
            return Err(XpsError::new(XpsErrorCode::InvalidPage));
        }
        self.content_box = Some(content_box);
        Ok(self)
    }

    /// Returns the page dimensions.
    pub const fn size(self) -> XpsPageSize {
        self.size
    }

    /// Returns the optional page-local content box.
    pub const fn content_box(self) -> Option<Rect> {
        self.content_box
    }
}

/// Hard ceilings for XPS construction and package serialization.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct XpsLimits {
    /// Maximum completed page count.
    pub max_pages: usize,
    /// Maximum OPC part count.
    pub max_parts: usize,
    /// Maximum image resource count on one page.
    pub max_resources_per_page: usize,
    /// Maximum display-list commands accepted by one page.
    pub max_commands_per_page: usize,
    /// Maximum serialized package bytes.
    pub max_output_bytes: u64,
}

impl XpsLimits {
    /// Validates that every ceiling is positive and representable by classic ZIP.
    pub fn validate(self) -> Result<Self, XpsError> {
        if self.max_pages == 0
            || self.max_parts == 0
            || self.max_resources_per_page == 0
            || self.max_commands_per_page == 0
            || self.max_output_bytes == 0
            || self.max_parts > usize::from(u16::MAX)
            || self.max_output_bytes > u64::from(u32::MAX)
        {
            return Err(XpsError::new(XpsErrorCode::InvalidLimits));
        }
        Ok(self)
    }
}

impl Default for XpsLimits {
    fn default() -> Self {
        Self {
            max_pages: 10_000,
            max_parts: 60_000,
            max_resources_per_page: 16_384,
            max_commands_per_page: 1_000_000,
            max_output_bytes: 512 * 1024 * 1024,
        }
    }
}

/// Whole-page CPU raster fallback configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RasterFallback {
    /// Raster resolution in pixels per inch.
    pub dpi: u16,
    /// Maximum raster pixel count.
    pub max_pixels: u64,
    /// Maximum RGBA working-memory bytes.
    pub max_bytes: u64,
}

impl RasterFallback {
    /// Validates positive bounded raster settings.
    pub fn validate(self) -> Result<Self, XpsError> {
        if self.dpi == 0 || self.dpi > 2400 || self.max_pixels == 0 || self.max_bytes == 0 {
            return Err(XpsError::new(XpsErrorCode::InvalidLimits));
        }
        Ok(self)
    }
}

impl Default for RasterFallback {
    fn default() -> Self {
        Self {
            dpi: 96,
            max_pixels: 67_108_864,
            max_bytes: 256 * 1024 * 1024,
        }
    }
}

/// Policy used when a display-list command has no faithful native mapping.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum UnsupportedBehavior {
    /// Reject the page explicitly.
    #[default]
    Error,
    /// Render the complete page through the deterministic CPU executor.
    RasterizePage,
}

/// XPS backend configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct XpsOptions {
    /// Package dialect to serialize.
    pub format: XpsFormat,
    /// Document and package resource ceilings.
    pub limits: XpsLimits,
    /// Unsupported drawing policy.
    pub unsupported_behavior: UnsupportedBehavior,
    /// Whole-page fallback limits.
    pub raster_fallback: RasterFallback,
}

impl Default for XpsOptions {
    fn default() -> Self {
        Self {
            format: XpsFormat::OpenXps,
            limits: XpsLimits::default(),
            unsupported_behavior: UnsupportedBehavior::Error,
            raster_fallback: RasterFallback::default(),
        }
    }
}

#[derive(Clone)]
struct ActivePage {
    spec: XpsPageSpec,
    lists: Vec<DisplayList>,
    command_count: usize,
}

#[derive(Clone)]
struct PageData {
    markup: Vec<u8>,
    images: Vec<Vec<u8>>,
}

/// Stateful, transactional XPS writer over an arbitrary `std::io::Write`.
///
/// No bytes reach the destination until [`finish`](Self::finish). Both
/// `finish` and [`abort`](Self::abort) consume the document.
pub struct XpsDocument<W: Write> {
    writer: W,
    options: XpsOptions,
    pages: Vec<PageData>,
    active: Option<ActivePage>,
}

impl<W: Write> XpsDocument<W> {
    /// Creates an empty document with explicit output options.
    pub fn new(writer: W, options: XpsOptions) -> Result<Self, XpsError> {
        options.limits.validate()?;
        options.raster_fallback.validate()?;
        Ok(Self {
            writer,
            options,
            pages: Vec::new(),
            active: None,
        })
    }

    /// Starts one fixed page. Nested pages are rejected.
    pub fn begin_page(&mut self, spec: XpsPageSpec) -> Result<(), XpsError> {
        if self.active.is_some() {
            return Err(XpsError::new(XpsErrorCode::InvalidState));
        }
        if self.pages.len() >= self.options.limits.max_pages {
            return Err(XpsError::new(XpsErrorCode::ResourceLimit));
        }
        self.active = Some(ActivePage {
            spec,
            lists: Vec::new(),
            command_count: 0,
        });
        Ok(())
    }

    /// Appends an immutable display list to the active page.
    pub fn add_display_list(&mut self, list: &DisplayList) -> Result<(), XpsError> {
        let active = self
            .active
            .as_mut()
            .ok_or(XpsError::new(XpsErrorCode::InvalidState))?;
        let count = active
            .command_count
            .checked_add(list.commands().len())
            .ok_or(XpsError::new(XpsErrorCode::ResourceLimit))?;
        if count > self.options.limits.max_commands_per_page {
            return Err(XpsError::new(XpsErrorCode::ResourceLimit));
        }
        active.command_count = count;
        active.lists.push(list.clone());
        Ok(())
    }

    /// Completes the active page using native markup or configured fallback.
    pub fn end_page(&mut self) -> Result<(), XpsError> {
        let active = self
            .active
            .as_ref()
            .ok_or(XpsError::new(XpsErrorCode::InvalidState))?;
        let page = match compile_native_page(active, self.options.format, self.options.limits) {
            Ok(page) => page,
            Err(error)
                if error.code() == XpsErrorCode::Unsupported
                    && self.options.unsupported_behavior == UnsupportedBehavior::RasterizePage =>
            {
                compile_raster_page(
                    active,
                    self.options.format,
                    self.options.raster_fallback,
                    self.options.limits,
                )?
            }
            Err(error) => return Err(error),
        };
        self.pages.push(page);
        self.active = None;
        Ok(())
    }

    /// Adds and completes one page in a single operation.
    pub fn add_page(&mut self, spec: XpsPageSpec, list: &DisplayList) -> Result<(), XpsError> {
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

    /// Serializes the selected deterministic XPS package and returns the writer.
    pub fn finish(mut self) -> Result<W, XpsError> {
        if self.active.is_some() || self.pages.is_empty() {
            return Err(XpsError::new(XpsErrorCode::InvalidState));
        }
        let bytes = serialize_package(&self.pages, self.options.format, self.options.limits)?;
        self.writer
            .write_all(&bytes)
            .map_err(|error| XpsError::io(&error))?;
        Ok(self.writer)
    }

    /// Aborts construction without writing and returns the destination.
    pub fn abort(self) -> W {
        self.writer
    }
}

fn compile_native_page(
    active: &ActivePage,
    format: XpsFormat,
    limits: XpsLimits,
) -> Result<PageData, XpsError> {
    let mut body = String::new();
    let mut images = Vec::new();
    if let Some(content_box) = active.spec.content_box {
        body.push_str("<Canvas Clip=\"");
        body.push_str(&rect_data(content_box, FillRule::NonZero));
        body.push_str("\">");
    }
    for (list_index, list) in active.lists.iter().enumerate() {
        compile_list(
            list,
            list_index == 0,
            active.spec,
            &mut body,
            &mut images,
            limits,
        )?;
    }
    if active.spec.content_box.is_some() {
        body.push_str("</Canvas>");
    }
    Ok(PageData {
        markup: fixed_page_markup(active.spec.size, format, &body),
        images,
    })
}

fn compile_list(
    list: &DisplayList,
    first_list: bool,
    spec: XpsPageSpec,
    output: &mut String,
    images: &mut Vec<Vec<u8>>,
    limits: XpsLimits,
) -> Result<(), XpsError> {
    let mut transform = Transform::IDENTITY;
    let mut transforms = Vec::new();
    for (command_index, command) in list.commands().iter().enumerate() {
        match command {
            DrawCommand::Clear(color) => {
                if !first_list || command_index != 0 {
                    return Err(XpsError::new(XpsErrorCode::Unsupported));
                }
                emit_filled_path(
                    output,
                    &page_rect_data(spec.size, FillRule::NonZero),
                    &Paint::new(*color),
                    Transform::IDENTITY,
                )?;
            }
            DrawCommand::Save => transforms.push(transform),
            DrawCommand::SaveLayer(_) => {
                return Err(XpsError::new(XpsErrorCode::Unsupported));
            }
            DrawCommand::Restore => {
                transform = transforms
                    .pop()
                    .ok_or(XpsError::new(XpsErrorCode::InvalidState))?;
            }
            DrawCommand::ClipRect { .. } | DrawCommand::ClipPath { .. } => {
                return Err(XpsError::new(XpsErrorCode::Unsupported));
            }
            DrawCommand::SetTransform(next) => transform = *next,
            DrawCommand::ConcatTransform(next) => {
                transform = transform.concat(*next).map_err(map_skia_error)?;
            }
            DrawCommand::FillRect { rect, paint } => {
                emit_filled_path(
                    output,
                    &rect_data(*rect, FillRule::NonZero),
                    paint,
                    transform,
                )?;
            }
            DrawCommand::FillPath { path, rule, paint } => {
                let path = list
                    .path(*path)
                    .ok_or(XpsError::new(XpsErrorCode::InvalidResource))?;
                if path.verbs().is_empty() {
                    continue;
                }
                let data = path_data(path, *rule)?;
                emit_filled_path(output, &data, paint, transform)?;
            }
            DrawCommand::StrokePath {
                path,
                options,
                paint,
            } => {
                if options.align() != StrokeAlign::Center
                    || !options.dash_pattern().is_empty()
                    || paint.path_effect().is_some()
                {
                    return Err(XpsError::new(XpsErrorCode::Unsupported));
                }
                validate_native_paint(paint, true)?;
                let path = list
                    .path(*path)
                    .ok_or(XpsError::new(XpsErrorCode::InvalidResource))?;
                if path.verbs().is_empty() {
                    continue;
                }
                let data = path_data(path, FillRule::NonZero)?;
                output.push_str("<Path Data=\"");
                output.push_str(&data);
                output.push_str("\" Stroke=\"");
                output.push_str(&xps_color(paint.color()));
                output.push_str("\" StrokeThickness=\"");
                output.push_str(&xps_scalar(options.width()));
                output.push_str("\" StrokeStartLineCap=\"");
                output.push_str(line_cap(options.cap()));
                output.push_str("\" StrokeEndLineCap=\"");
                output.push_str(line_cap(options.cap()));
                output.push_str("\" StrokeDashCap=\"");
                output.push_str(line_cap(options.cap()));
                output.push_str("\" StrokeLineJoin=\"");
                output.push_str(line_join(options.join()));
                output.push_str("\" StrokeMiterLimit=\"");
                output.push_str(&xps_scalar(options.miter_limit()));
                output.push('"');
                push_transform_attribute(output, transform);
                output.push_str("/>");
            }
            DrawCommand::DrawImage {
                image,
                destination,
                opacity,
                paint,
                ..
            } => {
                validate_native_image_paint(paint)?;
                let image = list
                    .image(*image)
                    .ok_or(XpsError::new(XpsErrorCode::InvalidResource))?;
                if !matches!(image.color_space(), ColorSpace::Srgb) {
                    return Err(XpsError::new(XpsErrorCode::UnsupportedColorProfile));
                }
                if images.len() >= limits.max_resources_per_page {
                    return Err(XpsError::new(XpsErrorCode::ResourceLimit));
                }
                let encoded = encode_png(image, limits.max_output_bytes)?;
                images.push(encoded);
                let image_number = images.len();
                emit_image(
                    output,
                    image,
                    *destination,
                    multiply_alpha(*opacity, paint.color().alpha()),
                    transform,
                    image_number,
                );
            }
            DrawCommand::DrawGlyphRun { .. } | DrawCommand::DrawPositionedGlyphRun { .. } => {
                return Err(XpsError::new(XpsErrorCode::UnsupportedText));
            }
        }
    }
    if !transforms.is_empty() {
        return Err(XpsError::new(XpsErrorCode::InvalidState));
    }
    Ok(())
}

fn emit_filled_path(
    output: &mut String,
    data: &str,
    paint: &Paint,
    transform: Transform,
) -> Result<(), XpsError> {
    validate_native_paint(paint, false)?;
    output.push_str("<Path Data=\"");
    output.push_str(data);
    output.push_str("\" Fill=\"");
    output.push_str(&xps_color(paint.color()));
    output.push('"');
    push_transform_attribute(output, transform);
    output.push_str("/>");
    Ok(())
}

fn emit_image(
    output: &mut String,
    image: &Image,
    destination: Rect,
    opacity: u8,
    transform: Transform,
    image_number: usize,
) {
    output.push_str("<Path Data=\"");
    output.push_str(&rect_data(destination, FillRule::NonZero));
    output.push('"');
    if opacity != u8::MAX {
        output.push_str(" Opacity=\"");
        output.push_str(&unit_channel(opacity));
        output.push('"');
    }
    push_transform_attribute(output, transform);
    output.push_str("><Path.Fill><ImageBrush ImageSource=\"../Resources/Images/__SKIA_PAGE__-");
    output.push_str(&image_number.to_string());
    output.push_str(".png\" Viewbox=\"0,0,");
    output.push_str(&image.width().to_string());
    output.push(',');
    output.push_str(&image.height().to_string());
    output.push_str("\" Viewport=\"");
    output.push_str(&xps_scalar(destination.left()));
    output.push(',');
    output.push_str(&xps_scalar(destination.top()));
    output.push(',');
    output.push_str(&xps_scalar(Scalar::from_bits(
        destination.right().bits() - destination.left().bits(),
    )));
    output.push(',');
    output.push_str(&xps_scalar(Scalar::from_bits(
        destination.bottom().bits() - destination.top().bits(),
    )));
    output.push_str(
        "\" ViewboxUnits=\"Absolute\" ViewportUnits=\"Absolute\" TileMode=\"None\"/>\
         </Path.Fill></Path>",
    );
}

fn validate_native_paint(paint: &Paint, stroke: bool) -> Result<(), XpsError> {
    if paint.blend_mode() != BlendMode::SourceOver
        || paint.shader_handle().is_some()
        || paint.color_filter_handle().is_some()
        || (!stroke && paint.path_effect().is_some())
    {
        return Err(XpsError::new(XpsErrorCode::Unsupported));
    }
    Ok(())
}

fn validate_native_image_paint(paint: &Paint) -> Result<(), XpsError> {
    if paint.blend_mode() != BlendMode::SourceOver
        || paint.shader_handle().is_some()
        || paint.color_filter_handle().is_some()
        || paint.path_effect().is_some()
    {
        return Err(XpsError::new(XpsErrorCode::Unsupported));
    }
    Ok(())
}

fn encode_png(image: &Image, maximum: u64) -> Result<Vec<u8>, XpsError> {
    let maximum =
        usize::try_from(maximum).map_err(|_| XpsError::new(XpsErrorCode::ResourceLimit))?;
    let options = EncodeOptions::new(EncodeFormat::Png(PngOptions::balanced_v1())).with_limits(
        EncodeLimits::new(maximum).map_err(|_| XpsError::new(XpsErrorCode::InvalidLimits))?,
    );
    ImageCodec::encode(&ImageAsset::new(image.clone()), &options)
        .map(|encoded| encoded.into_bytes())
        .map_err(|error| match error.code() {
            CodecErrorCode::OutputTooLarge => XpsError::new(XpsErrorCode::ResourceLimit),
            CodecErrorCode::UnsupportedColorProfile => {
                XpsError::new(XpsErrorCode::UnsupportedColorProfile)
            }
            _ => XpsError::new(XpsErrorCode::InvalidResource),
        })
}

fn fixed_page_markup(size: XpsPageSize, format: XpsFormat, body: &str) -> Vec<u8> {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
         <FixedPage xmlns=\"{}\" Width=\"{}\" Height=\"{}\" xml:lang=\"und\">\
         {body}</FixedPage>",
        format.namespace(),
        xps_scalar(size.width),
        xps_scalar(size.height)
    )
    .into_bytes()
}

fn path_data(path: &Path, rule: FillRule) -> Result<String, XpsError> {
    let mut output = String::from(match rule {
        FillRule::EvenOdd => "F0",
        FillRule::NonZero => "F1",
    });
    let mut current = None;
    let mut contour_start = None;
    for verb in path.verbs() {
        match *verb {
            PathVerb::MoveTo(point) => {
                output.push('M');
                push_point(&mut output, point.x(), point.y());
                current = Some(point);
                contour_start = Some(point);
            }
            PathVerb::LineTo(point) => {
                require_current(current)?;
                output.push('L');
                push_point(&mut output, point.x(), point.y());
                current = Some(point);
            }
            PathVerb::QuadTo(control, end) => {
                require_current(current)?;
                output.push('Q');
                push_point(&mut output, control.x(), control.y());
                output.push(' ');
                push_point(&mut output, end.x(), end.y());
                current = Some(end);
            }
            PathVerb::ConicTo(_, _, _) => {
                return Err(XpsError::new(XpsErrorCode::Unsupported));
            }
            PathVerb::CubicTo(first, second, end) => {
                require_current(current)?;
                output.push('C');
                push_point(&mut output, first.x(), first.y());
                output.push(' ');
                push_point(&mut output, second.x(), second.y());
                output.push(' ');
                push_point(&mut output, end.x(), end.y());
                current = Some(end);
            }
            PathVerb::Close => {
                require_current(current)?;
                output.push('Z');
                current = contour_start;
            }
        }
    }
    Ok(output)
}

fn rect_data(rect: Rect, rule: FillRule) -> String {
    let prefix = if rule == FillRule::EvenOdd {
        "F0"
    } else {
        "F1"
    };
    format!(
        "{prefix}M{},{}L{},{}L{},{}L{},{}Z",
        xps_scalar(rect.left()),
        xps_scalar(rect.top()),
        xps_scalar(rect.right()),
        xps_scalar(rect.top()),
        xps_scalar(rect.right()),
        xps_scalar(rect.bottom()),
        xps_scalar(rect.left()),
        xps_scalar(rect.bottom())
    )
}

fn page_rect_data(size: XpsPageSize, rule: FillRule) -> String {
    let prefix = if rule == FillRule::EvenOdd {
        "F0"
    } else {
        "F1"
    };
    format!(
        "{prefix}M0,0L{},0L{},{}L0,{}Z",
        xps_scalar(size.width),
        xps_scalar(size.width),
        xps_scalar(size.height),
        xps_scalar(size.height)
    )
}

fn push_point(output: &mut String, x: Scalar, y: Scalar) {
    output.push_str(&xps_scalar(x));
    output.push(',');
    output.push_str(&xps_scalar(y));
}

fn require_current<T>(current: Option<T>) -> Result<T, XpsError> {
    current.ok_or(XpsError::new(XpsErrorCode::InvalidResource))
}

fn push_transform_attribute(output: &mut String, transform: Transform) {
    if transform == Transform::IDENTITY {
        return;
    }
    let coefficients = transform.coefficients();
    output.push_str(" RenderTransform=\"");
    for (index, coefficient) in coefficients.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        output.push_str(&xps_scalar(*coefficient));
    }
    output.push('"');
}

fn line_cap(cap: StrokeCap) -> &'static str {
    match cap {
        StrokeCap::Butt => "Flat",
        StrokeCap::Round => "Round",
        StrokeCap::Square => "Square",
    }
}

fn line_join(join: StrokeJoin) -> &'static str {
    match join {
        StrokeJoin::Miter => "Miter",
        StrokeJoin::Round => "Round",
        StrokeJoin::Bevel => "Bevel",
    }
}

fn xps_color(color: Color) -> String {
    format!(
        "#{:02X}{:02X}{:02X}{:02X}",
        color.alpha(),
        color.red(),
        color.green(),
        color.blue()
    )
}

fn unit_channel(value: u8) -> String {
    if value == 0 {
        "0".to_owned()
    } else if value == u8::MAX {
        "1".to_owned()
    } else {
        let scaled = (u32::from(value) * 1_000_000 + 127) / 255;
        format!("0.{scaled:06}").trim_end_matches('0').to_owned()
    }
}

fn xps_scalar(value: Scalar) -> String {
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

fn multiply_alpha(left: u8, right: u8) -> u8 {
    ((u16::from(left) * u16::from(right) + 127) / 255) as u8
}

fn compile_raster_page(
    active: &ActivePage,
    format: XpsFormat,
    fallback: RasterFallback,
    limits: XpsLimits,
) -> Result<PageData, XpsError> {
    for list in &active.lists {
        for command in list.commands() {
            if matches!(
                command,
                DrawCommand::DrawGlyphRun { .. } | DrawCommand::DrawPositionedGlyphRun { .. }
            ) {
                return Err(XpsError::new(XpsErrorCode::UnsupportedText));
            }
            if let DrawCommand::DrawImage { image, .. } = command {
                let image = list
                    .image(*image)
                    .ok_or(XpsError::new(XpsErrorCode::InvalidResource))?;
                if !matches!(image.color_space(), ColorSpace::Srgb) {
                    return Err(XpsError::new(XpsErrorCode::UnsupportedColorProfile));
                }
            }
        }
    }
    let width = raster_dimension(active.spec.size.width, fallback.dpi)?;
    let height = raster_dimension(active.spec.size.height, fallback.dpi)?;
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or(XpsError::new(XpsErrorCode::NumericOverflow))?;
    let bytes = pixels
        .checked_mul(4)
        .ok_or(XpsError::new(XpsErrorCode::NumericOverflow))?;
    if pixels > fallback.max_pixels || bytes > fallback.max_bytes {
        return Err(XpsError::new(XpsErrorCode::ResourceLimit));
    }
    let surface_limits =
        SurfaceLimits::new(fallback.max_pixels, fallback.max_bytes, 256).map_err(map_skia_error)?;
    let mut surface = Surface::new(width, height, surface_limits).map_err(map_skia_error)?;
    let scale = Scalar::from_ratio(i64::from(fallback.dpi), 96).map_err(map_skia_error)?;
    replay_scaled(&mut surface, active, Transform::scale(scale, scale))?;
    let image = Image::from_rgba8(width, height, surface.pixels().to_vec())
        .map_err(|_| XpsError::new(XpsErrorCode::InvalidResource))?;
    let png = encode_png(&image, limits.max_output_bytes)?;
    let destination = Rect::new(
        Scalar::ZERO,
        Scalar::ZERO,
        active.spec.size.width,
        active.spec.size.height,
    )
    .map_err(map_skia_error)?;
    let mut body = String::new();
    emit_image(
        &mut body,
        &image,
        destination,
        u8::MAX,
        Transform::IDENTITY,
        1,
    );
    if let Some(content_box) = active.spec.content_box {
        body = format!(
            "<Canvas Clip=\"{}\">{body}</Canvas>",
            rect_data(content_box, FillRule::NonZero)
        );
    }
    Ok(PageData {
        markup: fixed_page_markup(active.spec.size, format, &body),
        images: vec![png],
    })
}

fn raster_dimension(units: Scalar, dpi: u16) -> Result<u32, XpsError> {
    let numerator = i64::from(units.bits())
        .checked_mul(i64::from(dpi))
        .ok_or(XpsError::new(XpsErrorCode::NumericOverflow))?;
    let denominator = 96_i64 << 16;
    let pixels = numerator
        .checked_add(denominator - 1)
        .ok_or(XpsError::new(XpsErrorCode::NumericOverflow))?
        / denominator;
    u32::try_from(pixels.max(1)).map_err(|_| XpsError::new(XpsErrorCode::ResourceLimit))
}

fn replay_scaled(
    surface: &mut Surface,
    active: &ActivePage,
    device_scale: Transform,
) -> Result<(), XpsError> {
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
                        .ok_or(XpsError::new(XpsErrorCode::InvalidState))?;
                }
                DrawCommand::ClipRect { rect, op } => canvas
                    .clip_rect_with_op(ClipRect::new(*rect), *op)
                    .map_err(map_skia_error)?,
                DrawCommand::ClipPath { path, rule, op } => canvas
                    .clip_path(
                        list.path(*path)
                            .ok_or(XpsError::new(XpsErrorCode::InvalidResource))?,
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
                            .ok_or(XpsError::new(XpsErrorCode::InvalidResource))?,
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
                            .ok_or(XpsError::new(XpsErrorCode::InvalidResource))?,
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
                            .ok_or(XpsError::new(XpsErrorCode::InvalidResource))?,
                        *destination,
                        *opacity,
                        paint.clone(),
                        *sampling,
                    )
                    .map_err(map_skia_error)?,
                DrawCommand::DrawGlyphRun { .. } | DrawCommand::DrawPositionedGlyphRun { .. } => {
                    return Err(XpsError::new(XpsErrorCode::UnsupportedText));
                }
            }
        }
        if !stack.is_empty() {
            return Err(XpsError::new(XpsErrorCode::InvalidState));
        }
    }
    Ok(())
}

fn serialize_package(
    pages: &[PageData],
    format: XpsFormat,
    limits: XpsLimits,
) -> Result<Vec<u8>, XpsError> {
    let image_count = pages.iter().try_fold(0_usize, |total, page| {
        total
            .checked_add(page.images.len())
            .ok_or(XpsError::new(XpsErrorCode::NumericOverflow))
    })?;
    let relationship_part_count = pages.iter().filter(|page| !page.images.is_empty()).count();
    let part_count = 4_usize
        .checked_add(pages.len())
        .and_then(|value| value.checked_add(relationship_part_count))
        .and_then(|value| value.checked_add(image_count))
        .ok_or(XpsError::new(XpsErrorCode::NumericOverflow))?;
    if part_count > limits.max_parts {
        return Err(XpsError::new(XpsErrorCode::ResourceLimit));
    }
    let mut parts = Vec::new();
    parts
        .try_reserve_exact(part_count)
        .map_err(|_| XpsError::new(XpsErrorCode::ResourceLimit))?;
    parts.push(Part {
        name: "[Content_Types].xml".to_owned(),
        bytes: content_types(image_count != 0).into_bytes(),
    });
    parts.push(Part {
        name: "_rels/.rels".to_owned(),
        bytes: package_relationships(format).into_bytes(),
    });
    parts.push(Part {
        name: "FixedDocumentSequence.fdseq".to_owned(),
        bytes: fixed_document_sequence(format).into_bytes(),
    });
    parts.push(Part {
        name: "Documents/1/FixedDocument.fdoc".to_owned(),
        bytes: fixed_document(pages.len(), format).into_bytes(),
    });
    for (page_index, page) in pages.iter().enumerate() {
        let page_number = page_index + 1;
        let markup = String::from_utf8(page.markup.clone())
            .map_err(|_| XpsError::new(XpsErrorCode::InvalidResource))?
            .replace("__SKIA_PAGE__", &page_number.to_string())
            .into_bytes();
        parts.push(Part {
            name: format!("Documents/1/Pages/{page_number}.fpage"),
            bytes: markup,
        });
        if !page.images.is_empty() {
            parts.push(Part {
                name: format!("Documents/1/Pages/_rels/{page_number}.fpage.rels"),
                bytes: page_relationships(page.images.len(), page_number, format).into_bytes(),
            });
            for (image_index, image) in page.images.iter().enumerate() {
                parts.push(Part {
                    name: format!(
                        "Documents/1/Resources/Images/{page_number}-{}.png",
                        image_index + 1
                    ),
                    bytes: image.clone(),
                });
            }
        }
    }
    opc::serialize(parts, limits.max_output_bytes).map_err(map_opc_error)
}

fn content_types(has_png: bool) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
         <Types xmlns=\"{CONTENT_TYPES_NAMESPACE}\">\
         <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
         <Default Extension=\"fdseq\" ContentType=\"application/vnd.ms-package.xps-fixeddocumentsequence+xml\"/>\
         <Default Extension=\"fdoc\" ContentType=\"application/vnd.ms-package.xps-fixeddocument+xml\"/>\
         <Default Extension=\"fpage\" ContentType=\"application/vnd.ms-package.xps-fixedpage+xml\"/>\
         {}\
         </Types>",
        if has_png {
            "<Default Extension=\"png\" ContentType=\"image/png\"/>"
        } else {
            ""
        }
    )
}

fn package_relationships(format: XpsFormat) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
         <Relationships xmlns=\"{RELATIONSHIPS_NAMESPACE}\">\
         <Relationship Id=\"R1\" Type=\"{}/fixedrepresentation\" \
         Target=\"FixedDocumentSequence.fdseq\"/>\
         </Relationships>",
        format.namespace()
    )
}

fn fixed_document_sequence(format: XpsFormat) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
         <FixedDocumentSequence xmlns=\"{}\">\
         <DocumentReference Source=\"Documents/1/FixedDocument.fdoc\"/>\
         </FixedDocumentSequence>",
        format.namespace()
    )
}

fn fixed_document(page_count: usize, format: XpsFormat) -> String {
    let mut output = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
         <FixedDocument xmlns=\"{}\">",
        format.namespace()
    );
    for page_number in 1..=page_count {
        output.push_str("<PageContent Source=\"Pages/");
        output.push_str(&page_number.to_string());
        output.push_str(".fpage\"/>");
    }
    output.push_str("</FixedDocument>");
    output
}

fn page_relationships(image_count: usize, page_number: usize, format: XpsFormat) -> String {
    let mut output = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
         <Relationships xmlns=\"{RELATIONSHIPS_NAMESPACE}\">"
    );
    for image_number in 1..=image_count {
        output.push_str("<Relationship Id=\"R");
        output.push_str(&image_number.to_string());
        output.push_str("\" Type=\"");
        output.push_str(format.namespace());
        output.push_str("/required-resource\" Target=\"../Resources/Images/");
        output.push_str(&page_number.to_string());
        output.push('-');
        output.push_str(&image_number.to_string());
        output.push_str(".png\"/>");
    }
    output.push_str("</Relationships>");
    output
}

fn map_skia_error(error: SkiaError) -> XpsError {
    match error.code() {
        SkiaErrorCode::ResourceLimit | SkiaErrorCode::AllocationFailed => {
            XpsError::new(XpsErrorCode::ResourceLimit)
        }
        SkiaErrorCode::NumericOverflow => XpsError::new(XpsErrorCode::NumericOverflow),
        SkiaErrorCode::InvalidResource | SkiaErrorCode::InvalidImage => {
            XpsError::new(XpsErrorCode::InvalidResource)
        }
        _ => XpsError::new(XpsErrorCode::Unsupported),
    }
}

fn map_opc_error(error: OpcError) -> XpsError {
    match error {
        OpcError::InvalidPart => XpsError::new(XpsErrorCode::InvalidResource),
        OpcError::ResourceLimit => XpsError::new(XpsErrorCode::ResourceLimit),
        OpcError::NumericOverflow => XpsError::new(XpsErrorCode::NumericOverflow),
    }
}
