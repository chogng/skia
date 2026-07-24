use std::{collections::HashMap, fmt};

use skia_codec::{CodecErrorCode, CodecLimits, ImageCodec};
use skia_core::{
    Angle, BlendMode, ClipOp, Color, ColorFilter, ColorMatrix, DisplayList, DisplayListBuilder,
    FillRule, FontCollection, FontSlant, FontStyle, FontWidth, Gradient, GradientStop, ImageFilter,
    Paint, Path, PathBuilder, PathVerb, Point, Rect, SamplingOptions, SaveLayerOptions, Scalar,
    ShaderHandle, StrokeCap, StrokeJoin, StrokeOptions, TextDirection, TextError, TextErrorCode,
    TextStyleSpan, TileMode, Transform,
};
use skia_xml::{XmlDocument, XmlElement, XmlError, XmlErrorCode, XmlLimits, XmlNode};

use crate::css::{CascadedStyle, CssError, Stylesheet};
use crate::{SvgCanvasSpec, SvgPreserveAspectRatio, SvgViewBoxAlignment, SvgViewBoxScale};

const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";
const XLINK_NAMESPACE: &str = "http://www.w3.org/1999/xlink";

/// Stable machine-readable SVG input failure code.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SvgReadErrorCode {
    /// The XML layer rejected the document.
    InvalidXml,
    /// The root element or SVG structure is invalid.
    InvalidDocument,
    /// A number, length, color, style, transform, or path is malformed.
    InvalidValue,
    /// Geometry cannot be represented by the fixed-point drawing contracts.
    InvalidGeometry,
    /// A configured XML, command, or path ceiling was exceeded.
    ResourceLimit,
    /// A valid SVG feature is outside this reader's current profile.
    Unsupported,
    /// Text was present but the caller did not provide an ordered font collection.
    MissingFontContext,
    /// Memory allocation failed.
    AllocationFailed,
}

/// Source-redacted SVG input error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SvgReadError {
    code: SvgReadErrorCode,
    xml_offset: Option<usize>,
}

impl SvgReadError {
    const fn new(code: SvgReadErrorCode) -> Self {
        Self {
            code,
            xml_offset: None,
        }
    }

    fn xml(error: XmlError) -> Self {
        let code = match error.code() {
            XmlErrorCode::ResourceLimit | XmlErrorCode::InvalidLimits => {
                SvgReadErrorCode::ResourceLimit
            }
            XmlErrorCode::AllocationFailed => SvgReadErrorCode::AllocationFailed,
            _ => SvgReadErrorCode::InvalidXml,
        };
        Self {
            code,
            xml_offset: Some(error.offset()),
        }
    }

    /// Returns the stable failure category.
    pub const fn code(self) -> SvgReadErrorCode {
        self.code
    }

    /// Returns the XML byte offset when the XML layer reported the failure.
    pub const fn xml_offset(self) -> Option<usize> {
        self.xml_offset
    }
}

impl fmt::Display for SvgReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.xml_offset {
            Some(offset) => write!(formatter, "{:?} at XML byte {offset}", self.code),
            None => write!(formatter, "{:?}", self.code),
        }
    }
}

impl std::error::Error for SvgReadError {}

/// Hard ceilings for one SVG input operation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SvgReadLimits {
    /// XML parser ceilings.
    pub xml: XmlLimits,
    /// Maximum commands and per-kind resources in the resulting display list.
    pub max_display_list_items: usize,
    /// Maximum verbs accepted in any one generated path.
    pub max_path_verbs: usize,
    /// Maximum nested local resource-reference depth.
    pub max_reference_depth: usize,
    /// Maximum encoded input and decoded RGBA8 bytes for one embedded image.
    pub max_embedded_image_bytes: usize,
}

impl Default for SvgReadLimits {
    fn default() -> Self {
        let xml = XmlLimits {
            max_attribute_value_bytes: 12 * 1024 * 1024,
            ..XmlLimits::default()
        };
        Self {
            xml,
            max_display_list_items: 100_000,
            max_path_verbs: 1_000_000,
            max_reference_depth: 64,
            max_embedded_image_bytes: 8 * 1024 * 1024,
        }
    }
}

/// SVG input policy.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct SvgReadOptions {
    /// Resource ceilings for parsing and lowering.
    pub limits: SvgReadLimits,
}

/// One decoded SVG canvas and its backend-neutral drawing commands.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SvgDocument {
    canvas: SvgCanvasSpec,
    display_list: DisplayList,
}

impl SvgDocument {
    /// Returns the SVG viewport and view box.
    pub const fn canvas(&self) -> SvgCanvasSpec {
        self.canvas
    }

    /// Borrows the decoded drawing commands.
    pub const fn display_list(&self) -> &DisplayList {
        &self.display_list
    }

    /// Consumes the document into its canvas and display list.
    pub fn into_parts(self) -> (SvgCanvasSpec, DisplayList) {
        (self.canvas, self.display_list)
    }
}

/// Stateless bounded SVG-to-display-list decoder.
pub struct SvgReader;

impl SvgReader {
    /// Parses and transactionally lowers one UTF-8 SVG document.
    pub fn decode(input: &[u8], options: SvgReadOptions) -> Result<SvgDocument, SvgReadError> {
        Self::decode_impl(input, options, None)
    }

    /// Parses and lowers one SVG using caller-owned portable fonts for text.
    ///
    /// Font family, weight, width, and slant matching is deterministic within
    /// the supplied collection. The reader never consults platform fonts.
    pub fn decode_with_fonts(
        input: &[u8],
        options: SvgReadOptions,
        fonts: &FontCollection,
    ) -> Result<SvgDocument, SvgReadError> {
        Self::decode_impl(input, options, Some(fonts))
    }

    fn decode_impl(
        input: &[u8],
        options: SvgReadOptions,
        fonts: Option<&FontCollection>,
    ) -> Result<SvgDocument, SvgReadError> {
        let limits = options.limits;
        if limits.max_display_list_items == 0
            || limits.max_path_verbs == 0
            || limits.max_reference_depth == 0
            || limits.max_embedded_image_bytes == 0
        {
            return Err(SvgReadError::new(SvgReadErrorCode::ResourceLimit));
        }
        let xml = XmlDocument::parse(input, limits.xml).map_err(SvgReadError::xml)?;
        Compiler::new(xml.root(), limits, fonts)?.compile()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TextAnchor {
    Start,
    Middle,
    End,
}

struct TextCursor {
    x: Scalar,
    y: Scalar,
    emitted: bool,
    pending_space: bool,
}

#[derive(Clone, Copy)]
enum MarkerPosition {
    Start,
    Middle,
    End,
}

#[derive(Clone, Copy)]
struct MarkerVertex {
    point: Point,
    angle_degrees: f64,
    position: MarkerPosition,
}

#[derive(Clone)]
struct Style {
    fill: Option<PaintSource>,
    stroke: Option<PaintSource>,
    fill_rule: FillRule,
    stroke_width: Scalar,
    line_cap: StrokeCap,
    line_join: StrokeJoin,
    miter_limit: Scalar,
    dash_pattern: Vec<Scalar>,
    dash_offset: Scalar,
    fill_opacity: u8,
    stroke_opacity: u8,
    visible: bool,
    font_size: Scalar,
    font_families: Vec<String>,
    font_style: FontStyle,
    text_anchor: TextAnchor,
    text_direction: Option<TextDirection>,
    marker_start: Option<String>,
    marker_mid: Option<String>,
    marker_end: Option<String>,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            fill: Some(PaintSource::Color(Color::BLACK)),
            stroke: None,
            fill_rule: FillRule::NonZero,
            stroke_width: Scalar::from_bits(1 << 16),
            line_cap: StrokeCap::Butt,
            line_join: StrokeJoin::Miter,
            miter_limit: Scalar::from_bits(4 << 16),
            dash_pattern: Vec::new(),
            dash_offset: Scalar::ZERO,
            fill_opacity: u8::MAX,
            stroke_opacity: u8::MAX,
            visible: true,
            font_size: Scalar::from_bits(16 << 16),
            font_families: vec!["sans-serif".to_owned()],
            font_style: FontStyle::NORMAL,
            text_anchor: TextAnchor::Start,
            text_direction: None,
            marker_start: None,
            marker_mid: None,
            marker_end: None,
        }
    }
}

struct Compiler<'a, 'fonts> {
    root: &'a XmlElement,
    limits: SvgReadLimits,
    builder: DisplayListBuilder,
    resources: HashMap<String, &'a XmlElement>,
    reference_stack: Vec<String>,
    canvas: Option<SvgCanvasSpec>,
    viewport_stack: Vec<Rect>,
    stylesheet: Stylesheet,
    ancestors: Vec<&'a XmlElement>,
    fonts: Option<&'fonts FontCollection>,
}

#[derive(Clone)]
enum PaintSource {
    Color(Color),
    Reference(String),
}

fn collect_resources<'a>(
    element: &'a XmlElement,
    resources: &mut HashMap<String, &'a XmlElement>,
    maximum: usize,
) -> Result<(), SvgReadError> {
    if element
        .namespace_uri()
        .is_none_or(|uri| uri == SVG_NAMESPACE)
        && matches!(
            element.local_name(),
            "script" | "animate" | "animateColor" | "animateMotion" | "animateTransform" | "set"
        )
    {
        return Err(SvgReadError::new(SvgReadErrorCode::Unsupported));
    }
    if let Some(id) = element.attribute_ns(None, "id") {
        if id.is_empty() || id.chars().any(char::is_whitespace) {
            return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
        }
        if resources.len() >= maximum {
            return Err(SvgReadError::new(SvgReadErrorCode::ResourceLimit));
        }
        resources
            .try_reserve(1)
            .map_err(|_| SvgReadError::new(SvgReadErrorCode::AllocationFailed))?;
        if resources.insert(id.to_owned(), element).is_some() {
            return Err(SvgReadError::new(SvgReadErrorCode::InvalidDocument));
        }
    }
    for child in element.children() {
        if let XmlNode::Element(child) = child {
            collect_resources(child, resources, maximum)?;
        }
    }
    Ok(())
}

impl<'a, 'fonts> Compiler<'a, 'fonts> {
    fn new(
        root: &'a XmlElement,
        limits: SvgReadLimits,
        fonts: Option<&'fonts FontCollection>,
    ) -> Result<Self, SvgReadError> {
        let builder =
            DisplayListBuilder::new(limits.max_display_list_items).map_err(map_core_error)?;
        let mut resources = HashMap::new();
        collect_resources(root, &mut resources, limits.max_display_list_items)?;
        let stylesheet =
            Stylesheet::parse(root, limits.max_display_list_items).map_err(map_css_error)?;
        Ok(Self {
            root,
            limits,
            builder,
            resources,
            reference_stack: Vec::new(),
            canvas: None,
            viewport_stack: Vec::new(),
            stylesheet,
            ancestors: Vec::new(),
            fonts,
        })
    }

    fn compile(mut self) -> Result<SvgDocument, SvgReadError> {
        if self.root.local_name() != "svg"
            || self
                .root
                .namespace_uri()
                .is_some_and(|namespace| namespace != SVG_NAMESPACE)
        {
            return Err(SvgReadError::new(SvgReadErrorCode::InvalidDocument));
        }
        let preserve_aspect_ratio = self
            .root
            .attribute_ns(None, "preserveAspectRatio")
            .map(parse_preserve_aspect_ratio)
            .transpose()?
            .unwrap_or(SvgPreserveAspectRatio::DEFAULT);
        let view_box = self
            .root
            .attribute("viewBox")
            .map(parse_view_box)
            .transpose()?;
        let width = match self.root.attribute("width") {
            Some(width) => parse_positive_length(width)?,
            None => Scalar::from_i32(300).map_err(map_core_error)?,
        };
        let height = match self.root.attribute("height") {
            Some(height) => parse_positive_length(height)?,
            None => Scalar::from_i32(150).map_err(map_core_error)?,
        };
        let mut canvas = SvgCanvasSpec::new(width, height)
            .map_err(|_| SvgReadError::new(SvgReadErrorCode::InvalidGeometry))?
            .with_preserve_aspect_ratio(preserve_aspect_ratio);
        if let Some(view_box) = view_box {
            canvas = canvas.with_view_box(view_box);
        }
        self.canvas = Some(canvas);
        self.viewport_stack.push(canvas.view_box());

        let cascade = self
            .stylesheet
            .cascade(self.root, &[])
            .map_err(map_css_error)?;
        if cascade.property("display") == Some("none") {
            return Ok(SvgDocument {
                canvas,
                display_list: self.builder.finish(),
            });
        }
        let style = parse_style(&Style::default(), &cascade)?;
        let transform = cascade
            .property("transform")
            .map(parse_transform)
            .transpose()?;
        let opacity = cascade
            .property("opacity")
            .map(parse_opacity)
            .transpose()?
            .unwrap_or(u8::MAX);
        let clip = cascade
            .property("clip-path")
            .filter(|value| *value != "none")
            .map(|value| {
                local_url_reference(value)
                    .map(str::to_owned)
                    .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::Unsupported))
            })
            .transpose()?;
        let uses_state = transform.is_some() || opacity != u8::MAX || clip.is_some();
        if uses_state {
            if opacity == u8::MAX {
                self.builder.save().map_err(map_core_error)?;
            } else {
                self.builder
                    .save_layer(SaveLayerOptions::new().with_opacity(opacity))
                    .map_err(map_core_error)?;
            }
            if let Some(transform) = transform {
                self.builder
                    .concat_transform(transform)
                    .map_err(map_core_error)?;
            }
            if let Some(clip) = clip.as_deref() {
                self.apply_clip_path(clip)?;
            }
        }
        self.ancestors.push(self.root);
        let result = self.lower_children(self.root, &style);
        self.ancestors.pop();
        result?;
        self.viewport_stack.pop();
        if uses_state {
            self.builder.restore().map_err(map_core_error)?;
        }
        Ok(SvgDocument {
            canvas,
            display_list: self.builder.finish(),
        })
    }

    fn lower_children(
        &mut self,
        parent: &'a XmlElement,
        inherited: &Style,
    ) -> Result<(), SvgReadError> {
        for child in parent.children() {
            match child {
                XmlNode::Element(element) => self.lower_element(element, inherited)?,
                XmlNode::Text(text) if !text.trim().is_empty() => {
                    return Err(SvgReadError::new(SvgReadErrorCode::Unsupported));
                }
                XmlNode::Text(_) => {}
            }
        }
        Ok(())
    }

    fn lower_element(
        &mut self,
        element: &'a XmlElement,
        inherited: &Style,
    ) -> Result<(), SvgReadError> {
        if element
            .namespace_uri()
            .is_some_and(|namespace| namespace != SVG_NAMESPACE)
        {
            return Err(SvgReadError::new(SvgReadErrorCode::Unsupported));
        }
        let name = element.local_name();
        if matches!(
            name,
            "title"
                | "desc"
                | "metadata"
                | "defs"
                | "style"
                | "linearGradient"
                | "radialGradient"
                | "pattern"
                | "filter"
                | "feColorMatrix"
                | "marker"
                | "clipPath"
                | "mask"
                | "symbol"
        ) {
            return Ok(());
        }
        let cascade = self
            .stylesheet
            .cascade(element, &self.ancestors)
            .map_err(map_css_error)?;
        if cascade.property("display") == Some("none") {
            return Ok(());
        }
        let style = parse_style(inherited, &cascade)?;
        let transform = cascade
            .property("transform")
            .map(parse_transform)
            .transpose()?;
        let opacity = cascade
            .property("opacity")
            .map(parse_opacity)
            .transpose()?
            .unwrap_or(u8::MAX);
        let clip = cascade
            .property("clip-path")
            .filter(|value| *value != "none")
            .map(|value| {
                local_url_reference(value)
                    .map(str::to_owned)
                    .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::Unsupported))
            })
            .transpose()?;
        let mask = cascade
            .property("mask")
            .filter(|value| *value != "none")
            .map(|value| {
                local_url_reference(value)
                    .map(str::to_owned)
                    .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::Unsupported))
            })
            .transpose()?;
        let filter = cascade
            .property("filter")
            .filter(|value| *value != "none")
            .map(|value| {
                local_url_reference(value)
                    .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::Unsupported))
                    .and_then(|id| self.resolve_color_matrix_filter(id))
            })
            .transpose()?;

        let uses_state =
            transform.is_some() || opacity != u8::MAX || clip.is_some() || filter.is_some();
        if uses_state {
            if opacity == u8::MAX && filter.is_none() {
                self.builder.save().map_err(map_core_error)?;
            } else {
                let mut options = SaveLayerOptions::new().with_opacity(opacity);
                if let Some(filter) = filter {
                    options = options.with_filter(filter);
                }
                self.builder.save_layer(options).map_err(map_core_error)?;
            }
            if let Some(transform) = transform {
                self.builder
                    .concat_transform(transform)
                    .map_err(map_core_error)?;
            }
            if let Some(clip) = clip.as_deref() {
                self.apply_clip_path(clip)?;
            }
        }

        if mask.is_some() {
            self.builder
                .save_layer(SaveLayerOptions::new())
                .map_err(map_core_error)?;
        }
        self.ancestors.push(element);
        let mut result = match name {
            "g" => self.lower_children(element, &style),
            "svg" => self.lower_nested_svg(element, &style, &cascade),
            "rect" => self
                .rect_path(element)
                .and_then(|path| self.draw_optional_path(path, &style)),
            "circle" => self
                .circle_path(element)
                .and_then(|path| self.draw_optional_path(path, &style)),
            "ellipse" => self
                .ellipse_path(element)
                .and_then(|path| self.draw_optional_path(path, &style)),
            "line" => self
                .line_path(element)
                .and_then(|path| self.draw_path(path, &style)),
            "polyline" => self
                .polygon_path(element, false)
                .and_then(|path| self.draw_optional_path(path, &style)),
            "polygon" => self
                .polygon_path(element, true)
                .and_then(|path| self.draw_optional_path(path, &style)),
            "path" => self
                .path(element)
                .and_then(|path| self.draw_optional_path(path, &style)),
            "image" => self.draw_image_element(element, &style, &cascade),
            "text" => self.lower_text(element, &style, &cascade),
            "use" => self.lower_use(element, &style),
            _ => Err(SvgReadError::new(SvgReadErrorCode::Unsupported)),
        };
        if result.is_ok()
            && let Some(mask) = mask.as_deref()
        {
            result = self.apply_alpha_mask(mask);
        }
        self.ancestors.pop();
        if mask.is_some() {
            self.builder.restore().map_err(map_core_error)?;
        }

        if uses_state {
            self.builder.restore().map_err(map_core_error)?;
        }
        result
    }

    fn draw_path(&mut self, path: Path, style: &Style) -> Result<(), SvgReadError> {
        if !style.visible {
            return Ok(());
        }
        let marker_vertices = if style.marker_start.is_some()
            || style.marker_mid.is_some()
            || style.marker_end.is_some()
        {
            path_marker_vertices(&path)?
        } else {
            Vec::new()
        };
        let fill_pattern = style
            .fill
            .as_ref()
            .and_then(|source| self.paint_server_of_kind(source, "pattern"));
        let fill_paint = if fill_pattern.is_some() {
            None
        } else {
            style
                .fill
                .as_ref()
                .map(|source| self.resolve_paint(source, &path, style.fill_opacity))
                .transpose()?
        };
        if style
            .stroke
            .as_ref()
            .and_then(|source| self.paint_server_of_kind(source, "pattern"))
            .is_some()
        {
            return Err(SvgReadError::new(SvgReadErrorCode::Unsupported));
        }
        let stroke_paint = style
            .stroke
            .as_ref()
            .map(|source| self.resolve_paint(source, &path, style.stroke_opacity))
            .transpose()?;
        let pattern_bounds = fill_pattern
            .map(|_| {
                let bounds = path
                    .tight_bounds()
                    .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidGeometry))?;
                Rect::new(bounds.left(), bounds.top(), bounds.right(), bounds.bottom())
                    .map_err(map_core_error)
            })
            .transpose()?;
        let id = self.builder.add_path(path).map_err(map_core_error)?;
        if let Some(pattern) = fill_pattern {
            self.lower_pattern_fill(
                pattern,
                id,
                style.fill_rule,
                pattern_bounds
                    .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidGeometry))?,
                style.fill_opacity,
            )?;
        } else if let Some(paint) = fill_paint {
            self.builder
                .fill_path(id, style.fill_rule, paint)
                .map_err(map_core_error)?;
        }
        if let Some(paint) = stroke_paint
            && style.stroke_width.bits() > 0
        {
            let options = StrokeOptions::new(style.stroke_width)
                .and_then(|options| options.with_miter_limit(style.miter_limit))
                .and_then(|options| {
                    options.with_dash_pattern(&style.dash_pattern, style.dash_offset)
                })
                .map(|options| options.with_cap(style.line_cap).with_join(style.line_join))
                .map_err(map_core_error)?;
            self.builder
                .stroke_path_with_options(id, options, paint)
                .map_err(map_core_error)?;
        }
        for vertex in marker_vertices {
            let reference = match vertex.position {
                MarkerPosition::Start => style.marker_start.as_deref(),
                MarkerPosition::Middle => style.marker_mid.as_deref(),
                MarkerPosition::End => style.marker_end.as_deref(),
            };
            if let Some(reference) = reference {
                self.lower_marker(reference, vertex, style)?;
            }
        }
        Ok(())
    }

    fn paint_server_of_kind(&self, source: &PaintSource, kind: &str) -> Option<&'a XmlElement> {
        let PaintSource::Reference(id) = source else {
            return None;
        };
        self.resources.get(id).copied().filter(|element| {
            element
                .namespace_uri()
                .is_none_or(|uri| uri == SVG_NAMESPACE)
                && element.local_name() == kind
        })
    }

    fn lower_pattern_fill(
        &mut self,
        pattern: &'a XmlElement,
        target: skia_core::PathId,
        rule: FillRule,
        bounds: Rect,
        opacity: u8,
    ) -> Result<(), SvgReadError> {
        let id = pattern
            .attribute_ns(None, "id")
            .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidValue))?;
        if self.reference_stack.len() >= self.limits.max_reference_depth
            || self.reference_stack.iter().any(|active| active == id)
        {
            return Err(SvgReadError::new(SvgReadErrorCode::ResourceLimit));
        }
        let chain = self.resource_chain(pattern, "pattern")?;
        if effective_attribute(&chain, "patternTransform").is_some() {
            return Err(SvgReadError::new(SvgReadErrorCode::Unsupported));
        }
        let object_bounding_box =
            match effective_attribute(&chain, "patternUnits").unwrap_or("objectBoundingBox") {
                "objectBoundingBox" => true,
                "userSpaceOnUse" => false,
                _ => return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue)),
            };
        let bounds_width = subtract(bounds.right(), bounds.left())?;
        let bounds_height = subtract(bounds.bottom(), bounds.top())?;
        if bounds_width.bits() <= 0 || bounds_height.bits() <= 0 {
            return Err(SvgReadError::new(SvgReadErrorCode::InvalidGeometry));
        }
        let (base_x, base_y, tile_width, tile_height) = if object_bounding_box {
            (
                object_box_coordinate(
                    effective_attribute(&chain, "x"),
                    bounds.left(),
                    bounds_width,
                    Scalar::ZERO,
                )?,
                object_box_coordinate(
                    effective_attribute(&chain, "y"),
                    bounds.top(),
                    bounds_height,
                    Scalar::ZERO,
                )?,
                object_box_extent(effective_attribute(&chain, "width"), bounds_width)?,
                object_box_extent(effective_attribute(&chain, "height"), bounds_height)?,
            )
        } else {
            (
                effective_attribute(&chain, "x")
                    .map(parse_length)
                    .transpose()?
                    .unwrap_or(Scalar::ZERO),
                effective_attribute(&chain, "y")
                    .map(parse_length)
                    .transpose()?
                    .unwrap_or(Scalar::ZERO),
                effective_attribute(&chain, "width")
                    .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidValue))
                    .and_then(parse_positive_length)?,
                effective_attribute(&chain, "height")
                    .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidValue))
                    .and_then(parse_positive_length)?,
            )
        };
        if tile_width.bits() <= 0 || tile_height.bits() <= 0 {
            return Err(SvgReadError::new(SvgReadErrorCode::InvalidGeometry));
        }
        let columns = tile_range(bounds.left(), bounds.right(), base_x, tile_width)?;
        let rows = tile_range(bounds.top(), bounds.bottom(), base_y, tile_height)?;
        let tile_count = columns
            .len()
            .checked_mul(rows.len())
            .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::ResourceLimit))?;
        if tile_count == 0 || tile_count > self.limits.max_display_list_items {
            return Err(SvgReadError::new(SvgReadErrorCode::ResourceLimit));
        }
        let owner = chain
            .iter()
            .rev()
            .find(|element| {
                element
                    .children()
                    .iter()
                    .any(|child| child.as_element().is_some())
            })
            .copied()
            .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidValue))?;
        let cascade = self
            .stylesheet
            .cascade(owner, &self.ancestors)
            .map_err(map_css_error)?;
        let pattern_style = parse_style(&Style::default(), &cascade)?;
        let view_box = effective_attribute(&chain, "viewBox")
            .map(parse_view_box)
            .transpose()?;
        let content_object_bounding_box =
            match effective_attribute(&chain, "patternContentUnits").unwrap_or("userSpaceOnUse") {
                "objectBoundingBox" if view_box.is_none() => true,
                "userSpaceOnUse" | "objectBoundingBox" if view_box.is_some() => false,
                "userSpaceOnUse" => false,
                _ => return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue)),
            };

        self.reference_stack.push(id.to_owned());
        self.builder.save().map_err(map_core_error)?;
        self.builder
            .clip_path(target, rule, ClipOp::Intersect)
            .map_err(map_core_error)?;
        if opacity != u8::MAX {
            self.builder
                .save_layer(SaveLayerOptions::new().with_opacity(opacity))
                .map_err(map_core_error)?;
        }
        self.ancestors.push(owner);
        let mut result = Ok(());
        'tiles: for row in rows {
            for &column in &columns {
                let tile_x = tile_position(base_x, tile_width, column)?;
                let tile_y = tile_position(base_y, tile_height, row)?;
                let tile = Rect::new(
                    tile_x,
                    tile_y,
                    add(tile_x, tile_width)?,
                    add(tile_y, tile_height)?,
                )
                .map_err(map_core_error)?;
                self.builder.save().map_err(map_core_error)?;
                self.builder.clip_rect(tile).map_err(map_core_error)?;
                let mapping = if let Some(view_box) = view_box {
                    viewport_mapping(
                        tile,
                        view_box,
                        effective_attribute(&chain, "preserveAspectRatio")
                            .unwrap_or("xMidYMid meet"),
                    )?
                } else {
                    let offset_x = subtract(tile_x, base_x)?;
                    let offset_y = subtract(tile_y, base_y)?;
                    let repeat = Transform::translate(offset_x, offset_y);
                    if content_object_bounding_box {
                        repeat
                            .concat(Transform::new(
                                bounds_width,
                                Scalar::ZERO,
                                Scalar::ZERO,
                                bounds_height,
                                bounds.left(),
                                bounds.top(),
                            ))
                            .map_err(map_core_error)?
                    } else {
                        repeat
                    }
                };
                self.builder
                    .concat_transform(mapping)
                    .map_err(map_core_error)?;
                result = self.lower_children(owner, &pattern_style);
                let restore = self.builder.restore().map_err(map_core_error);
                if result.is_err() {
                    break 'tiles;
                }
                restore?;
            }
        }
        self.ancestors.pop();
        if opacity != u8::MAX {
            self.builder.restore().map_err(map_core_error)?;
        }
        self.builder.restore().map_err(map_core_error)?;
        self.reference_stack.pop();
        result
    }

    fn lower_marker(
        &mut self,
        reference: &str,
        vertex: MarkerVertex,
        inherited: &Style,
    ) -> Result<(), SvgReadError> {
        if self.reference_stack.len() >= self.limits.max_reference_depth
            || self
                .reference_stack
                .iter()
                .any(|active| active == reference)
        {
            return Err(SvgReadError::new(SvgReadErrorCode::ResourceLimit));
        }
        let marker = *self
            .resources
            .get(reference)
            .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidValue))?;
        if marker
            .namespace_uri()
            .is_some_and(|uri| uri != SVG_NAMESPACE)
            || marker.local_name() != "marker"
        {
            return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
        }
        let width = marker
            .attribute_ns(None, "markerWidth")
            .map(parse_positive_length)
            .transpose()?
            .unwrap_or(Scalar::from_bits(3 << 16));
        let height = marker
            .attribute_ns(None, "markerHeight")
            .map(parse_positive_length)
            .transpose()?
            .unwrap_or(Scalar::from_bits(3 << 16));
        let viewport =
            Rect::new(Scalar::ZERO, Scalar::ZERO, width, height).map_err(map_core_error)?;
        let view_box = marker
            .attribute_ns(None, "viewBox")
            .map(parse_view_box)
            .transpose()?;
        let content_mapping = view_box
            .map(|view_box| {
                viewport_mapping(
                    viewport,
                    view_box,
                    marker
                        .attribute_ns(None, "preserveAspectRatio")
                        .unwrap_or("xMidYMid meet"),
                )
            })
            .transpose()?
            .unwrap_or(Transform::IDENTITY);
        let reference_point = Point::new(
            marker
                .attribute_ns(None, "refX")
                .map(parse_length)
                .transpose()?
                .unwrap_or(Scalar::ZERO),
            marker
                .attribute_ns(None, "refY")
                .map(parse_length)
                .transpose()?
                .unwrap_or(Scalar::ZERO),
        );
        let mapped_reference = content_mapping
            .map_point(reference_point)
            .map_err(map_core_error)?;
        let unit_scale = match marker
            .attribute_ns(None, "markerUnits")
            .unwrap_or("strokeWidth")
        {
            "strokeWidth" => Transform::scale(inherited.stroke_width, inherited.stroke_width),
            "userSpaceOnUse" => Transform::IDENTITY,
            _ => return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue)),
        };
        let angle = match marker.attribute_ns(None, "orient").unwrap_or("0") {
            "auto" => vertex.angle_degrees,
            "auto-start-reverse" => {
                vertex.angle_degrees
                    + if matches!(vertex.position, MarkerPosition::Start) {
                        180.0
                    } else {
                        0.0
                    }
            }
            value => parse_angle_degrees(value)?,
        };
        let placement = Transform::translate(
            subtract(Scalar::ZERO, mapped_reference.x())?,
            subtract(Scalar::ZERO, mapped_reference.y())?,
        );
        let placement = placement.concat(unit_scale).map_err(map_core_error)?;
        let rotation = rotation_transform(scalar_from_f64(angle)?, Scalar::ZERO, Scalar::ZERO)?;
        let placement = placement.concat(rotation).map_err(map_core_error)?;
        let placement = placement
            .concat(Transform::translate(vertex.point.x(), vertex.point.y()))
            .map_err(map_core_error)?;
        let final_transform = content_mapping.concat(placement).map_err(map_core_error)?;
        let cascade = self
            .stylesheet
            .cascade(marker, &self.ancestors)
            .map_err(map_css_error)?;
        if cascade.property("display") == Some("none") {
            return Ok(());
        }
        let marker_style = parse_style(&Style::default(), &cascade)?;
        let opacity = cascade
            .property("opacity")
            .map(parse_opacity)
            .transpose()?
            .unwrap_or(u8::MAX);

        self.reference_stack.push(reference.to_owned());
        if opacity == u8::MAX {
            self.builder.save().map_err(map_core_error)?;
        } else {
            self.builder
                .save_layer(SaveLayerOptions::new().with_opacity(opacity))
                .map_err(map_core_error)?;
        }
        if !matches!(cascade.property("overflow"), Some("visible")) {
            let mut clip_builder = self.path_builder()?;
            clip_builder
                .add_round_rect(viewport, Scalar::ZERO, Scalar::ZERO)
                .map_err(map_core_error)?;
            let clip = clip_builder
                .finish()
                .and_then(|path| path.transformed(placement))
                .map_err(map_core_error)?;
            let clip = self.builder.add_path(clip).map_err(map_core_error)?;
            self.builder
                .clip_path(clip, FillRule::NonZero, ClipOp::Intersect)
                .map_err(map_core_error)?;
        }
        self.builder
            .concat_transform(final_transform)
            .map_err(map_core_error)?;
        self.ancestors.push(marker);
        let result = self.lower_children(marker, &marker_style);
        self.ancestors.pop();
        let restore = self.builder.restore().map_err(map_core_error);
        self.reference_stack.pop();
        result.and(restore)
    }

    fn draw_optional_path(
        &mut self,
        path: Option<Path>,
        style: &Style,
    ) -> Result<(), SvgReadError> {
        match path {
            Some(path) => self.draw_path(path, style),
            None => Ok(()),
        }
    }

    fn resolve_paint(
        &mut self,
        source: &PaintSource,
        path: &Path,
        opacity: u8,
    ) -> Result<Paint, SvgReadError> {
        match source {
            PaintSource::Color(color) => Ok(Paint::new(color.with_opacity(opacity))),
            PaintSource::Reference(id) => {
                if self.reference_stack.len() >= self.limits.max_reference_depth
                    || self.reference_stack.iter().any(|active| active == id)
                {
                    return Err(SvgReadError::new(SvgReadErrorCode::ResourceLimit));
                }
                let element = *self
                    .resources
                    .get(id)
                    .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidValue))?;
                self.reference_stack.push(id.clone());
                let result = self.resolve_gradient(element, path, opacity);
                self.reference_stack.pop();
                result
            }
        }
    }

    fn resolve_gradient(
        &self,
        element: &'a XmlElement,
        path: &Path,
        opacity: u8,
    ) -> Result<Paint, SvgReadError> {
        if element
            .namespace_uri()
            .is_some_and(|uri| uri != SVG_NAMESPACE)
            || !matches!(element.local_name(), "linearGradient" | "radialGradient")
        {
            return Err(SvgReadError::new(SvgReadErrorCode::Unsupported));
        }
        let chain = self.gradient_chain(element)?;
        let units = effective_attribute(&chain, "gradientUnits").unwrap_or("objectBoundingBox");
        let object_bounding_box = match units {
            "objectBoundingBox" => true,
            "userSpaceOnUse" => false,
            _ => return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue)),
        };
        let bounding_transform = if object_bounding_box {
            let bounds = path
                .tight_bounds()
                .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidGeometry))?;
            let width = subtract(bounds.right(), bounds.left())?;
            let height = subtract(bounds.bottom(), bounds.top())?;
            if width.bits() <= 0 || height.bits() <= 0 {
                return Err(SvgReadError::new(SvgReadErrorCode::InvalidGeometry));
            }
            Some(Transform::new(
                width,
                Scalar::ZERO,
                Scalar::ZERO,
                height,
                bounds.left(),
                bounds.top(),
            ))
        } else {
            None
        };
        let spread = match effective_attribute(&chain, "spreadMethod").unwrap_or("pad") {
            "pad" => TileMode::Clamp,
            "repeat" => TileMode::Repeat,
            "reflect" => TileMode::Mirror,
            _ => return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue)),
        };
        let stops = gradient_stops(&chain)?;
        let gradient = if element.local_name() == "linearGradient" {
            let start = Point::new(
                self.gradient_coordinate(
                    effective_attribute(&chain, "x1"),
                    Scalar::ZERO,
                    object_bounding_box,
                    Axis::Horizontal,
                )?,
                self.gradient_coordinate(
                    effective_attribute(&chain, "y1"),
                    Scalar::ZERO,
                    object_bounding_box,
                    Axis::Vertical,
                )?,
            );
            let end = Point::new(
                self.gradient_coordinate(
                    effective_attribute(&chain, "x2"),
                    Scalar::from_bits(1 << 16),
                    object_bounding_box,
                    Axis::Horizontal,
                )?,
                self.gradient_coordinate(
                    effective_attribute(&chain, "y2"),
                    Scalar::ZERO,
                    object_bounding_box,
                    Axis::Vertical,
                )?,
            );
            Gradient::linear(start, end, &stops, spread).map_err(map_core_error)?
        } else {
            let center_x = self.gradient_coordinate(
                effective_attribute(&chain, "cx"),
                Scalar::from_bits(1 << 15),
                object_bounding_box,
                Axis::Horizontal,
            )?;
            let center_y = self.gradient_coordinate(
                effective_attribute(&chain, "cy"),
                Scalar::from_bits(1 << 15),
                object_bounding_box,
                Axis::Vertical,
            )?;
            let radius = self.gradient_coordinate(
                effective_attribute(&chain, "r"),
                Scalar::from_bits(1 << 15),
                object_bounding_box,
                Axis::Radius,
            )?;
            if radius.bits() <= 0 {
                return Err(SvgReadError::new(SvgReadErrorCode::InvalidGeometry));
            }
            for (name, center) in [("fx", center_x), ("fy", center_y)] {
                if let Some(value) = effective_attribute(&chain, name)
                    && self.gradient_coordinate(
                        Some(value),
                        center,
                        object_bounding_box,
                        if name == "fx" {
                            Axis::Horizontal
                        } else {
                            Axis::Vertical
                        },
                    )? != center
                {
                    return Err(SvgReadError::new(SvgReadErrorCode::Unsupported));
                }
            }
            if let Some(value) = effective_attribute(&chain, "fr")
                && parse_length_or_percentage(value)?.bits() != 0
            {
                return Err(SvgReadError::new(SvgReadErrorCode::Unsupported));
            }
            Gradient::radial(Point::new(center_x, center_y), radius, &stops, spread)
                .map_err(map_core_error)?
        };

        let gradient_transform = effective_attribute(&chain, "gradientTransform")
            .map(parse_transform)
            .transpose()?;
        let local_matrix = match (gradient_transform, bounding_transform) {
            (Some(gradient), Some(bounds)) => {
                Some(gradient.concat(bounds).map_err(map_core_error)?)
            }
            (Some(gradient), None) => Some(gradient),
            (None, bounds) => bounds,
        };
        let color = Color::WHITE.with_opacity(opacity);
        match local_matrix {
            Some(matrix) => {
                let shader = ShaderHandle::from_gradient(gradient)
                    .with_local_matrix(matrix)
                    .map_err(map_core_error)?;
                Ok(Paint::new(color).with_shader(shader))
            }
            None => Ok(Paint::from_gradient(gradient).with_color(color)),
        }
    }

    fn gradient_chain(&self, element: &'a XmlElement) -> Result<Vec<&'a XmlElement>, SvgReadError> {
        self.resource_chain(element, element.local_name())
    }

    fn resource_chain(
        &self,
        element: &'a XmlElement,
        kind: &str,
    ) -> Result<Vec<&'a XmlElement>, SvgReadError> {
        let mut chain = vec![element];
        let mut current = element;
        while let Some(reference) = href(current) {
            let id = reference
                .strip_prefix('#')
                .filter(|id| !id.is_empty())
                .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::Unsupported))?;
            if chain.len() >= self.limits.max_reference_depth {
                return Err(SvgReadError::new(SvgReadErrorCode::ResourceLimit));
            }
            let base = *self
                .resources
                .get(id)
                .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidValue))?;
            if base.local_name() != kind
                || base.namespace_uri().is_some_and(|uri| uri != SVG_NAMESPACE)
                || chain.iter().any(|element| std::ptr::eq(*element, base))
            {
                return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
            }
            chain.push(base);
            current = base;
        }
        chain.reverse();
        Ok(chain)
    }

    fn gradient_coordinate(
        &self,
        value: Option<&str>,
        default: Scalar,
        object_bounding_box: bool,
        axis: Axis,
    ) -> Result<Scalar, SvgReadError> {
        let Some(value) = value else {
            return Ok(default);
        };
        let (coordinate, percentage) = parse_length_or_percentage_with_kind(value)?;
        if object_bounding_box {
            return Ok(if percentage {
                coordinate
            } else {
                parse_scalar(value)?
            });
        }
        if !percentage {
            return parse_length(value);
        }
        let view_box = self
            .viewport_stack
            .last()
            .copied()
            .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidDocument))?;
        let extent = match axis {
            Axis::Horizontal => subtract(view_box.right(), view_box.left())?,
            Axis::Vertical => subtract(view_box.bottom(), view_box.top())?,
            Axis::Radius => {
                let width = scalar_to_f64(subtract(view_box.right(), view_box.left())?);
                let height = scalar_to_f64(subtract(view_box.bottom(), view_box.top())?);
                scalar_from_f64(((width * width + height * height) / 2.0).sqrt())?
            }
        };
        let origin = match axis {
            Axis::Horizontal => view_box.left(),
            Axis::Vertical => view_box.top(),
            Axis::Radius => Scalar::ZERO,
        };
        add(origin, multiply_scalar(extent, coordinate)?)
    }

    fn lower_use(&mut self, element: &XmlElement, inherited: &Style) -> Result<(), SvgReadError> {
        let reference = href(element)
            .and_then(|value| value.strip_prefix('#'))
            .filter(|id| !id.is_empty())
            .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::Unsupported))?;
        if self.reference_stack.len() >= self.limits.max_reference_depth
            || self
                .reference_stack
                .iter()
                .any(|active| active == reference)
        {
            return Err(SvgReadError::new(SvgReadErrorCode::ResourceLimit));
        }
        let target = *self
            .resources
            .get(reference)
            .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidValue))?;
        if target
            .namespace_uri()
            .is_some_and(|uri| uri != SVG_NAMESPACE)
        {
            return Err(SvgReadError::new(SvgReadErrorCode::Unsupported));
        }
        if target.local_name() == "symbol" {
            self.reference_stack.push(reference.to_owned());
            let result = self.lower_symbol_instance(element, target, inherited);
            self.reference_stack.pop();
            return result;
        }
        let x = optional_length(element, "x")?;
        let y = optional_length(element, "y")?;
        let translated = x != Scalar::ZERO || y != Scalar::ZERO;
        if translated {
            self.builder.save().map_err(map_core_error)?;
            self.builder
                .concat_transform(Transform::translate(x, y))
                .map_err(map_core_error)?;
        }
        self.reference_stack.push(reference.to_owned());
        let result = self.lower_element(target, inherited);
        self.reference_stack.pop();
        if translated {
            self.builder.restore().map_err(map_core_error)?;
        }
        result
    }

    fn lower_symbol_instance(
        &mut self,
        use_element: &XmlElement,
        symbol: &'a XmlElement,
        inherited: &Style,
    ) -> Result<(), SvgReadError> {
        let parent = self
            .viewport_stack
            .last()
            .copied()
            .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidDocument))?;
        let parent_width = subtract(parent.right(), parent.left())?;
        let parent_height = subtract(parent.bottom(), parent.top())?;
        let x = nested_length(
            use_element.attribute_ns(None, "x"),
            parent_width,
            Scalar::ZERO,
        )?;
        let y = nested_length(
            use_element.attribute_ns(None, "y"),
            parent_height,
            Scalar::ZERO,
        )?;
        let width = nested_length(
            use_element.attribute_ns(None, "width"),
            parent_width,
            parent_width,
        )?;
        let height = nested_length(
            use_element.attribute_ns(None, "height"),
            parent_height,
            parent_height,
        )?;
        if width.bits() < 0 || height.bits() < 0 {
            return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
        }
        if width == Scalar::ZERO || height == Scalar::ZERO {
            return Ok(());
        }
        let viewport = Rect::new(x, y, add(x, width)?, add(y, height)?).map_err(map_core_error)?;
        let view_box = symbol
            .attribute_ns(None, "viewBox")
            .map(parse_view_box)
            .transpose()?;
        let cascade = self
            .stylesheet
            .cascade(symbol, &self.ancestors)
            .map_err(map_css_error)?;
        if cascade.property("display") == Some("none") {
            return Ok(());
        }
        let style = parse_style(inherited, &cascade)?;
        let transform = cascade
            .property("transform")
            .map(parse_transform)
            .transpose()?;
        let opacity = cascade
            .property("opacity")
            .map(parse_opacity)
            .transpose()?
            .unwrap_or(u8::MAX);
        if opacity == u8::MAX {
            self.builder.save().map_err(map_core_error)?;
        } else {
            self.builder
                .save_layer(SaveLayerOptions::new().with_opacity(opacity))
                .map_err(map_core_error)?;
        }
        self.builder.clip_rect(viewport).map_err(map_core_error)?;
        if let Some(transform) = transform {
            self.builder
                .concat_transform(transform)
                .map_err(map_core_error)?;
        }
        let (child_viewport, mapping) = if let Some(view_box) = view_box {
            (
                view_box,
                viewport_mapping(
                    viewport,
                    view_box,
                    symbol
                        .attribute_ns(None, "preserveAspectRatio")
                        .unwrap_or("xMidYMid meet"),
                )?,
            )
        } else {
            (
                Rect::new(Scalar::ZERO, Scalar::ZERO, width, height).map_err(map_core_error)?,
                Transform::translate(x, y),
            )
        };
        self.builder
            .concat_transform(mapping)
            .map_err(map_core_error)?;
        self.viewport_stack.push(child_viewport);
        self.ancestors.push(symbol);
        let result = self.lower_children(symbol, &style);
        self.ancestors.pop();
        self.viewport_stack.pop();
        self.builder.restore().map_err(map_core_error)?;
        result
    }

    fn lower_nested_svg(
        &mut self,
        element: &'a XmlElement,
        inherited: &Style,
        cascade: &CascadedStyle,
    ) -> Result<(), SvgReadError> {
        let parent = self
            .viewport_stack
            .last()
            .copied()
            .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidDocument))?;
        let parent_width = subtract(parent.right(), parent.left())?;
        let parent_height = subtract(parent.bottom(), parent.top())?;
        let x = nested_length(element.attribute_ns(None, "x"), parent_width, Scalar::ZERO)?;
        let y = nested_length(element.attribute_ns(None, "y"), parent_height, Scalar::ZERO)?;
        let width = nested_length(
            element.attribute_ns(None, "width"),
            parent_width,
            parent_width,
        )?;
        let height = nested_length(
            element.attribute_ns(None, "height"),
            parent_height,
            parent_height,
        )?;
        if width.bits() < 0 || height.bits() < 0 {
            return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
        }
        if width == Scalar::ZERO || height == Scalar::ZERO {
            return Ok(());
        }
        let viewport = Rect::new(x, y, add(x, width)?, add(y, height)?).map_err(map_core_error)?;
        let view_box = element
            .attribute_ns(None, "viewBox")
            .map(parse_view_box)
            .transpose()?;
        let overflow_hidden = !matches!(cascade.property("overflow"), Some("visible"));
        if !matches!(
            cascade.property("overflow"),
            None | Some("hidden" | "scroll" | "auto" | "visible")
        ) {
            return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
        }
        self.builder.save().map_err(map_core_error)?;
        if overflow_hidden {
            self.builder.clip_rect(viewport).map_err(map_core_error)?;
        }
        let (child_viewport, transform) = if let Some(view_box) = view_box {
            (
                view_box,
                viewport_mapping(
                    viewport,
                    view_box,
                    element
                        .attribute_ns(None, "preserveAspectRatio")
                        .unwrap_or("xMidYMid meet"),
                )?,
            )
        } else {
            (
                Rect::new(Scalar::ZERO, Scalar::ZERO, width, height).map_err(map_core_error)?,
                Transform::translate(x, y),
            )
        };
        self.builder
            .concat_transform(transform)
            .map_err(map_core_error)?;
        self.viewport_stack.push(child_viewport);
        let result = self.lower_children(element, inherited);
        self.viewport_stack.pop();
        self.builder.restore().map_err(map_core_error)?;
        result
    }

    fn draw_image_element(
        &mut self,
        element: &XmlElement,
        style: &Style,
        cascade: &CascadedStyle,
    ) -> Result<(), SvgReadError> {
        if !style.visible {
            return Ok(());
        }
        let x = optional_length(element, "x")?;
        let y = optional_length(element, "y")?;
        let width = required_nonnegative_length(element, "width")?;
        let height = required_nonnegative_length(element, "height")?;
        if width == Scalar::ZERO || height == Scalar::ZERO {
            return Ok(());
        }
        let source =
            href(element).ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidValue))?;
        let encoded = decode_image_data_uri(source, self.limits.max_embedded_image_bytes)?;
        let decoded_limit = u64::try_from(self.limits.max_embedded_image_bytes)
            .map_err(|_| SvgReadError::new(SvgReadErrorCode::ResourceLimit))?;
        let codec_limits = CodecLimits::new(
            self.limits.max_embedded_image_bytes,
            (decoded_limit / 4).max(1),
            decoded_limit,
        )
        .map_err(map_codec_error)?;
        let asset =
            ImageCodec::decode_with_limits(&encoded, codec_limits).map_err(map_codec_error)?;
        let image = asset.image();
        let destination = image_destination(
            x,
            y,
            width,
            height,
            image.width(),
            image.height(),
            element
                .attribute_ns(None, "preserveAspectRatio")
                .unwrap_or("xMidYMid meet"),
        )?;
        let image = self
            .builder
            .add_image(image.clone())
            .map_err(map_core_error)?;
        let sampling = match cascade.property("image-rendering").unwrap_or("auto") {
            "auto" | "smooth" | "optimizeQuality" => SamplingOptions::LINEAR,
            "pixelated" | "crisp-edges" | "optimizeSpeed" => SamplingOptions::NEAREST,
            _ => return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue)),
        };
        self.builder
            .draw_image_with_sampling(
                image,
                destination,
                u8::MAX,
                Paint::new(Color::WHITE),
                sampling,
            )
            .map_err(map_core_error)
    }

    fn lower_text(
        &mut self,
        element: &'a XmlElement,
        style: &Style,
        cascade: &CascadedStyle,
    ) -> Result<(), SvgReadError> {
        let mut cursor = TextCursor {
            x: cascade
                .property("x")
                .map(first_length)
                .transpose()?
                .unwrap_or(Scalar::ZERO),
            y: cascade
                .property("y")
                .map(first_length)
                .transpose()?
                .unwrap_or(Scalar::ZERO),
            emitted: false,
            pending_space: false,
        };
        if let Some(dx) = cascade.property("dx") {
            cursor.x = add(cursor.x, first_length(dx)?)?;
        }
        if let Some(dy) = cascade.property("dy") {
            cursor.y = add(cursor.y, first_length(dy)?)?;
        }
        self.lower_text_children(element, style, &mut cursor)
    }

    fn lower_text_children(
        &mut self,
        parent: &'a XmlElement,
        inherited: &Style,
        cursor: &mut TextCursor,
    ) -> Result<(), SvgReadError> {
        for child in parent.children() {
            match child {
                XmlNode::Text(text) => {
                    let normalized = normalize_text(text, cursor);
                    if !normalized.is_empty() {
                        self.draw_text_segment(&normalized, inherited, cursor)?;
                    }
                }
                XmlNode::Element(element)
                    if element
                        .namespace_uri()
                        .is_none_or(|uri| uri == SVG_NAMESPACE)
                        && element.local_name() == "tspan" =>
                {
                    let cascade = self
                        .stylesheet
                        .cascade(element, &self.ancestors)
                        .map_err(map_css_error)?;
                    if cascade.property("display") == Some("none") {
                        continue;
                    }
                    let style = parse_style(inherited, &cascade)?;
                    if let Some(x) = cascade.property("x") {
                        cursor.x = first_length(x)?;
                        cursor.emitted = false;
                        cursor.pending_space = false;
                    }
                    if let Some(y) = cascade.property("y") {
                        cursor.y = first_length(y)?;
                    }
                    if let Some(dx) = cascade.property("dx") {
                        cursor.x = add(cursor.x, first_length(dx)?)?;
                    }
                    if let Some(dy) = cascade.property("dy") {
                        cursor.y = add(cursor.y, first_length(dy)?)?;
                    }
                    self.ancestors.push(element);
                    let result = self.lower_text_children(element, &style, cursor);
                    self.ancestors.pop();
                    result?;
                }
                XmlNode::Element(element)
                    if element
                        .namespace_uri()
                        .is_none_or(|uri| uri == SVG_NAMESPACE)
                        && matches!(element.local_name(), "title" | "desc") => {}
                XmlNode::Element(_) => {
                    return Err(SvgReadError::new(SvgReadErrorCode::Unsupported));
                }
            }
        }
        Ok(())
    }

    fn draw_text_segment(
        &mut self,
        text: &str,
        style: &Style,
        cursor: &mut TextCursor,
    ) -> Result<(), SvgReadError> {
        let fonts = self
            .fonts
            .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::MissingFontContext))?;
        let families = style
            .font_families
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        let typeface = fonts
            .match_typeface_for_families(&families, style.font_style)
            .or_else(|| fonts.typefaces().first())
            .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::MissingFontContext))?;
        let source_end = u32::try_from(text.len())
            .map_err(|_| SvgReadError::new(SvgReadErrorCode::ResourceLimit))?;
        let span = TextStyleSpan::new(0, source_end, typeface.id(), style.font_size.bits())
            .map_err(map_text_error)?;
        let paragraph = match style.text_direction {
            Some(direction) => fonts
                .shape_styled_paragraph_with_direction(text, &[span], direction)
                .map_err(map_text_error)?,
            None => fonts
                .shape_styled_paragraph(text, &[span])
                .map_err(map_text_error)?,
        };
        let advance = Scalar::from_bits(paragraph.advance_x_bits());
        let origin_x = match style.text_anchor {
            TextAnchor::Start => cursor.x,
            TextAnchor::Middle => {
                subtract(cursor.x, Scalar::from_bits(paragraph.advance_x_bits() / 2))?
            }
            TextAnchor::End => subtract(cursor.x, advance)?,
        };
        if style.visible {
            if style.stroke.is_some() {
                return Err(SvgReadError::new(SvgReadErrorCode::Unsupported));
            }
            if let Some(fill) = &style.fill {
                let PaintSource::Color(color) = fill else {
                    return Err(SvgReadError::new(SvgReadErrorCode::Unsupported));
                };
                self.builder
                    .draw_shaped_paragraph(
                        &paragraph,
                        Point::new(origin_x, cursor.y),
                        Paint::new(color.with_opacity(style.fill_opacity)),
                    )
                    .map_err(map_core_error)?;
            }
        }
        cursor.x = add(cursor.x, advance)?;
        cursor.emitted = true;
        Ok(())
    }

    fn apply_clip_path(&mut self, id: &str) -> Result<(), SvgReadError> {
        if self.reference_stack.len() >= self.limits.max_reference_depth
            || self.reference_stack.iter().any(|active| active == id)
        {
            return Err(SvgReadError::new(SvgReadErrorCode::ResourceLimit));
        }
        let clip = *self
            .resources
            .get(id)
            .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidValue))?;
        if clip.namespace_uri().is_some_and(|uri| uri != SVG_NAMESPACE)
            || clip.local_name() != "clipPath"
        {
            return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
        }
        if !matches!(
            clip.attribute_ns(None, "clipPathUnits"),
            None | Some("userSpaceOnUse")
        ) {
            return Err(SvgReadError::new(SvgReadErrorCode::Unsupported));
        }
        let mut shapes = Vec::new();
        for child in clip.children() {
            match child {
                XmlNode::Text(text) if text.trim().is_empty() => {}
                XmlNode::Text(_) => {
                    return Err(SvgReadError::new(SvgReadErrorCode::Unsupported));
                }
                XmlNode::Element(element) => {
                    if let Some(path) = self.clip_geometry(element)? {
                        shapes
                            .try_reserve(1)
                            .map_err(|_| SvgReadError::new(SvgReadErrorCode::AllocationFailed))?;
                        shapes.push(path);
                    }
                }
            }
        }
        if shapes.is_empty() {
            return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
        }
        let mut path = combine_paths(&shapes, self.limits.max_path_verbs)?;
        if let Some(transform) = clip.attribute_ns(None, "transform") {
            path = path
                .transformed(parse_transform(transform)?)
                .map_err(map_core_error)?;
        }
        let rule = match style_property(clip, "clip-rule").unwrap_or("nonzero") {
            "nonzero" => FillRule::NonZero,
            "evenodd" => FillRule::EvenOdd,
            _ => return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue)),
        };
        let path = self.builder.add_path(path).map_err(map_core_error)?;
        self.builder
            .clip_path(path, rule, ClipOp::Intersect)
            .map_err(map_core_error)
    }

    fn apply_alpha_mask(&mut self, id: &str) -> Result<(), SvgReadError> {
        if self.reference_stack.len() >= self.limits.max_reference_depth
            || self.reference_stack.iter().any(|active| active == id)
        {
            return Err(SvgReadError::new(SvgReadErrorCode::ResourceLimit));
        }
        let mask = *self
            .resources
            .get(id)
            .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidValue))?;
        if mask.namespace_uri().is_some_and(|uri| uri != SVG_NAMESPACE)
            || mask.local_name() != "mask"
        {
            return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
        }
        if !matches!(mask.attribute_ns(None, "maskUnits"), Some("userSpaceOnUse"))
            || !matches!(
                mask.attribute_ns(None, "maskContentUnits"),
                None | Some("userSpaceOnUse")
            )
        {
            return Err(SvgReadError::new(SvgReadErrorCode::Unsupported));
        }
        if !matches!(style_property(mask, "mask-type"), Some("alpha")) {
            return Err(SvgReadError::new(SvgReadErrorCode::Unsupported));
        }
        let cascade = self
            .stylesheet
            .cascade(mask, &self.ancestors)
            .map_err(map_css_error)?;
        let style = parse_style(&Style::default(), &cascade)?;
        self.reference_stack.push(id.to_owned());
        self.builder
            .save_layer(SaveLayerOptions::new().with_blend_mode(BlendMode::DestinationIn))
            .map_err(map_core_error)?;
        let has_region = ["x", "y", "width", "height"]
            .iter()
            .any(|name| mask.attribute_ns(None, name).is_some());
        let region_result = if has_region {
            (|| {
                let x = optional_length(mask, "x")?;
                let y = optional_length(mask, "y")?;
                let width = required_nonnegative_length(mask, "width")?;
                let height = required_nonnegative_length(mask, "height")?;
                if width == Scalar::ZERO || height == Scalar::ZERO {
                    return Err(SvgReadError::new(SvgReadErrorCode::InvalidGeometry));
                }
                let region =
                    Rect::new(x, y, add(x, width)?, add(y, height)?).map_err(map_core_error)?;
                self.builder.clip_rect(region).map_err(map_core_error)
            })()
        } else {
            Ok(())
        };
        let result = region_result.and_then(|()| {
            self.ancestors.push(mask);
            let result = self.lower_children(mask, &style);
            self.ancestors.pop();
            result
        });
        let restore_result = self.builder.restore().map_err(map_core_error);
        self.reference_stack.pop();
        result.and(restore_result)
    }

    fn resolve_color_matrix_filter(&self, id: &str) -> Result<ImageFilter, SvgReadError> {
        let filter = *self
            .resources
            .get(id)
            .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidValue))?;
        if filter
            .namespace_uri()
            .is_some_and(|uri| uri != SVG_NAMESPACE)
            || filter.local_name() != "filter"
        {
            return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
        }
        if filter.attributes().iter().any(|attribute| {
            attribute.namespace_uri().is_none()
                && matches!(
                    attribute.local_name(),
                    "x" | "y" | "width" | "height" | "filterUnits" | "primitiveUnits"
                )
        }) {
            return Err(SvgReadError::new(SvgReadErrorCode::Unsupported));
        }
        let primitives = filter
            .children()
            .iter()
            .filter_map(XmlNode::as_element)
            .collect::<Vec<_>>();
        if primitives.len() != 1 {
            return Err(SvgReadError::new(SvgReadErrorCode::Unsupported));
        }
        let primitive = primitives[0];
        if primitive
            .namespace_uri()
            .is_some_and(|uri| uri != SVG_NAMESPACE)
            || primitive.local_name() != "feColorMatrix"
            || !matches!(primitive.attribute_ns(None, "type"), None | Some("matrix"))
            || !matches!(
                primitive.attribute_ns(None, "in"),
                None | Some("SourceGraphic")
            )
            || primitive.attribute_ns(None, "result").is_some()
        {
            return Err(SvgReadError::new(SvgReadErrorCode::Unsupported));
        }
        if primitive
            .children()
            .iter()
            .any(|child| child.as_text().is_none_or(|text| !text.trim().is_empty()))
        {
            return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
        }
        let values = match primitive.attribute_ns(None, "values") {
            Some(values) => NumberList::parse_all(values)?,
            None => vec![
                Scalar::from_bits(1 << 16),
                Scalar::ZERO,
                Scalar::ZERO,
                Scalar::ZERO,
                Scalar::ZERO,
                Scalar::ZERO,
                Scalar::from_bits(1 << 16),
                Scalar::ZERO,
                Scalar::ZERO,
                Scalar::ZERO,
                Scalar::ZERO,
                Scalar::ZERO,
                Scalar::from_bits(1 << 16),
                Scalar::ZERO,
                Scalar::ZERO,
                Scalar::ZERO,
                Scalar::ZERO,
                Scalar::ZERO,
                Scalar::from_bits(1 << 16),
                Scalar::ZERO,
            ],
        };
        if values.len() != 20 {
            return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
        }
        let mut matrix = [0_i32; 20];
        for (index, value) in values.into_iter().enumerate() {
            matrix[index] = if index % 5 == 4 {
                multiply_scalar(value, Scalar::from_bits(255 << 16))?.bits()
            } else {
                value.bits()
            };
        }
        Ok(ImageFilter::Color(ColorFilter::Matrix(ColorMatrix::new(
            matrix,
        ))))
    }

    fn clip_geometry(&self, element: &XmlElement) -> Result<Option<Path>, SvgReadError> {
        if element
            .namespace_uri()
            .is_some_and(|namespace| namespace != SVG_NAMESPACE)
        {
            return Err(SvgReadError::new(SvgReadErrorCode::Unsupported));
        }
        let mut path = match element.local_name() {
            "rect" => self.rect_path(element)?,
            "circle" => self.circle_path(element)?,
            "ellipse" => self.ellipse_path(element)?,
            "polygon" => self.polygon_path(element, true)?,
            "path" => self.path(element)?,
            _ => return Err(SvgReadError::new(SvgReadErrorCode::Unsupported)),
        };
        if let Some(transform) = element.attribute_ns(None, "transform")
            && let Some(value) = path.take()
        {
            path = Some(
                value
                    .transformed(parse_transform(transform)?)
                    .map_err(map_core_error)?,
            );
        }
        Ok(path)
    }

    fn rect_path(&self, element: &XmlElement) -> Result<Option<Path>, SvgReadError> {
        let x = optional_length(element, "x")?;
        let y = optional_length(element, "y")?;
        let width = required_nonnegative_length(element, "width")?;
        let height = required_nonnegative_length(element, "height")?;
        if width == Scalar::ZERO || height == Scalar::ZERO {
            return Ok(None);
        }
        let rect = Rect::new(x, y, add(x, width)?, add(y, height)?).map_err(map_core_error)?;
        let radius_x = element
            .attribute("rx")
            .map(parse_nonnegative_length)
            .transpose()?;
        let radius_y = element
            .attribute("ry")
            .map(parse_nonnegative_length)
            .transpose()?;
        let (radius_x, radius_y) = match (radius_x, radius_y) {
            (None, None) => (Scalar::ZERO, Scalar::ZERO),
            (Some(value), None) | (None, Some(value)) => (value, value),
            (Some(x), Some(y)) => (x, y),
        };
        let mut builder = self.path_builder()?;
        builder
            .add_round_rect(rect, radius_x, radius_y)
            .map_err(map_core_error)?;
        builder.finish().map(Some).map_err(map_core_error)
    }

    fn circle_path(&self, element: &XmlElement) -> Result<Option<Path>, SvgReadError> {
        let center = Point::new(
            optional_length(element, "cx")?,
            optional_length(element, "cy")?,
        );
        let radius = required_nonnegative_length(element, "r")?;
        if radius == Scalar::ZERO {
            return Ok(None);
        }
        let mut builder = self.path_builder()?;
        builder.add_circle(center, radius).map_err(map_core_error)?;
        builder.finish().map(Some).map_err(map_core_error)
    }

    fn ellipse_path(&self, element: &XmlElement) -> Result<Option<Path>, SvgReadError> {
        let cx = optional_length(element, "cx")?;
        let cy = optional_length(element, "cy")?;
        let rx = required_nonnegative_length(element, "rx")?;
        let ry = required_nonnegative_length(element, "ry")?;
        if rx == Scalar::ZERO || ry == Scalar::ZERO {
            return Ok(None);
        }
        let bounds = Rect::new(
            subtract(cx, rx)?,
            subtract(cy, ry)?,
            add(cx, rx)?,
            add(cy, ry)?,
        )
        .map_err(map_core_error)?;
        let mut builder = self.path_builder()?;
        builder.add_oval(bounds).map_err(map_core_error)?;
        builder.finish().map(Some).map_err(map_core_error)
    }

    fn line_path(&self, element: &XmlElement) -> Result<Path, SvgReadError> {
        let mut builder = self.path_builder()?;
        builder
            .move_to(Point::new(
                optional_length(element, "x1")?,
                optional_length(element, "y1")?,
            ))
            .map_err(map_core_error)?;
        builder
            .line_to(Point::new(
                optional_length(element, "x2")?,
                optional_length(element, "y2")?,
            ))
            .map_err(map_core_error)?;
        builder.finish().map_err(map_core_error)
    }

    fn polygon_path(
        &self,
        element: &XmlElement,
        close: bool,
    ) -> Result<Option<Path>, SvgReadError> {
        let source = element
            .attribute("points")
            .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidValue))?;
        let values = NumberList::parse_all(source)?;
        if values.len() % 2 != 0 {
            return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
        }
        let points = values
            .chunks_exact(2)
            .map(|pair| Point::new(pair[0], pair[1]))
            .collect::<Vec<_>>();
        if points.len() < if close { 3 } else { 2 } {
            return Ok(None);
        }
        let mut builder = self.path_builder()?;
        builder
            .add_polygon(&points, close)
            .map_err(map_core_error)?;
        builder.finish().map(Some).map_err(map_core_error)
    }

    fn path(&self, element: &XmlElement) -> Result<Option<Path>, SvgReadError> {
        let data = element
            .attribute("d")
            .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidValue))?;
        if data.trim().is_empty() {
            return Ok(None);
        }
        PathDataParser::new(data, self.limits.max_path_verbs)?
            .parse()
            .map(Some)
    }

    fn path_builder(&self) -> Result<PathBuilder, SvgReadError> {
        PathBuilder::new(self.limits.max_path_verbs).map_err(map_core_error)
    }
}

fn map_core_error(error: skia_core::SkiaError) -> SvgReadError {
    use skia_core::SkiaErrorCode;
    let code = match error.code() {
        SkiaErrorCode::ResourceLimit | SkiaErrorCode::InvalidLimits => {
            SvgReadErrorCode::ResourceLimit
        }
        SkiaErrorCode::AllocationFailed => SvgReadErrorCode::AllocationFailed,
        _ => SvgReadErrorCode::InvalidGeometry,
    };
    SvgReadError::new(code)
}

fn map_codec_error(error: skia_codec::CodecError) -> SvgReadError {
    let code = match error.code() {
        CodecErrorCode::InputTooLarge | CodecErrorCode::ImageTooLarge => {
            SvgReadErrorCode::ResourceLimit
        }
        _ => SvgReadErrorCode::InvalidValue,
    };
    SvgReadError::new(code)
}

fn map_css_error(error: CssError) -> SvgReadError {
    let code = match error {
        CssError::Invalid => SvgReadErrorCode::InvalidValue,
        CssError::ResourceLimit => SvgReadErrorCode::ResourceLimit,
        CssError::AllocationFailed => SvgReadErrorCode::AllocationFailed,
    };
    SvgReadError::new(code)
}

fn map_text_error(error: TextError) -> SvgReadError {
    let code = match error.code() {
        TextErrorCode::ResourceLimit => SvgReadErrorCode::ResourceLimit,
        TextErrorCode::AllocationFailed => SvgReadErrorCode::AllocationFailed,
        TextErrorCode::EmptyFontCollection => SvgReadErrorCode::MissingFontContext,
        _ => SvgReadErrorCode::InvalidValue,
    };
    SvgReadError::new(code)
}

fn combine_paths(paths: &[Path], maximum: usize) -> Result<Path, SvgReadError> {
    let mut builder = PathBuilder::new(maximum).map_err(map_core_error)?;
    for path in paths {
        for verb in path.verbs() {
            match *verb {
                skia_core::PathVerb::MoveTo(point) => {
                    builder.move_to(point).map_err(map_core_error)?
                }
                skia_core::PathVerb::LineTo(point) => {
                    builder.line_to(point).map_err(map_core_error)?
                }
                skia_core::PathVerb::QuadTo(control, end) => {
                    builder.quad_to(control, end).map_err(map_core_error)?
                }
                skia_core::PathVerb::ConicTo(control, end, weight) => builder
                    .conic_to(control, end, weight)
                    .map_err(map_core_error)?,
                skia_core::PathVerb::CubicTo(first, second, end) => builder
                    .cubic_to(first, second, end)
                    .map_err(map_core_error)?,
                skia_core::PathVerb::Close => builder.close().map_err(map_core_error)?,
            }
        }
    }
    builder.finish().map_err(map_core_error)
}

#[derive(Clone, Copy)]
struct MarkerSegment {
    start: Point,
    end: Point,
    start_tangent: (f64, f64),
    end_tangent: (f64, f64),
}

fn path_marker_vertices(path: &Path) -> Result<Vec<MarkerVertex>, SvgReadError> {
    let mut output = Vec::new();
    output
        .try_reserve(path.verbs().len().saturating_mul(2))
        .map_err(|_| SvgReadError::new(SvgReadErrorCode::AllocationFailed))?;
    let mut segments = Vec::new();
    segments
        .try_reserve(path.verbs().len())
        .map_err(|_| SvgReadError::new(SvgReadErrorCode::AllocationFailed))?;
    let mut current = None;
    let mut contour_start = None;
    for verb in path.verbs() {
        match *verb {
            PathVerb::MoveTo(point) => {
                append_marker_contour(&segments, &mut output);
                segments.clear();
                current = Some(point);
                contour_start = Some(point);
            }
            PathVerb::LineTo(end) => {
                let start =
                    current.ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidGeometry))?;
                let tangent = point_delta(start, end);
                segments.push(MarkerSegment {
                    start,
                    end,
                    start_tangent: tangent,
                    end_tangent: tangent,
                });
                current = Some(end);
            }
            PathVerb::QuadTo(control, end) | PathVerb::ConicTo(control, end, _) => {
                let start =
                    current.ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidGeometry))?;
                segments.push(MarkerSegment {
                    start,
                    end,
                    start_tangent: first_nonzero_delta(&[start, control, end]),
                    end_tangent: last_nonzero_delta(&[start, control, end]),
                });
                current = Some(end);
            }
            PathVerb::CubicTo(first, second, end) => {
                let start =
                    current.ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidGeometry))?;
                segments.push(MarkerSegment {
                    start,
                    end,
                    start_tangent: first_nonzero_delta(&[start, first, second, end]),
                    end_tangent: last_nonzero_delta(&[start, first, second, end]),
                });
                current = Some(end);
            }
            PathVerb::Close => {
                let start =
                    current.ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidGeometry))?;
                let end = contour_start
                    .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidGeometry))?;
                if start != end {
                    let tangent = point_delta(start, end);
                    segments.push(MarkerSegment {
                        start,
                        end,
                        start_tangent: tangent,
                        end_tangent: tangent,
                    });
                }
                append_marker_contour(&segments, &mut output);
                segments.clear();
                current = Some(end);
            }
        }
    }
    append_marker_contour(&segments, &mut output);
    Ok(output)
}

fn append_marker_contour(segments: &[MarkerSegment], output: &mut Vec<MarkerVertex>) {
    let Some(first) = segments.first() else {
        return;
    };
    output.push(MarkerVertex {
        point: first.start,
        angle_degrees: vector_angle(first.start_tangent),
        position: MarkerPosition::Start,
    });
    for pair in segments.windows(2) {
        output.push(MarkerVertex {
            point: pair[0].end,
            angle_degrees: bisected_angle(pair[0].end_tangent, pair[1].start_tangent),
            position: MarkerPosition::Middle,
        });
    }
    if let Some(last) = segments.last() {
        output.push(MarkerVertex {
            point: last.end,
            angle_degrees: vector_angle(last.end_tangent),
            position: MarkerPosition::End,
        });
    }
}

fn point_delta(start: Point, end: Point) -> (f64, f64) {
    (
        scalar_to_f64(end.x()) - scalar_to_f64(start.x()),
        scalar_to_f64(end.y()) - scalar_to_f64(start.y()),
    )
}

fn first_nonzero_delta(points: &[Point]) -> (f64, f64) {
    points
        .windows(2)
        .map(|pair| point_delta(pair[0], pair[1]))
        .find(|(x, y)| *x != 0.0 || *y != 0.0)
        .unwrap_or((1.0, 0.0))
}

fn last_nonzero_delta(points: &[Point]) -> (f64, f64) {
    points
        .windows(2)
        .rev()
        .map(|pair| point_delta(pair[0], pair[1]))
        .find(|(x, y)| *x != 0.0 || *y != 0.0)
        .unwrap_or((1.0, 0.0))
}

fn vector_angle(vector: (f64, f64)) -> f64 {
    vector.1.atan2(vector.0).to_degrees()
}

fn bisected_angle(incoming: (f64, f64), outgoing: (f64, f64)) -> f64 {
    let normalize = |(x, y): (f64, f64)| {
        let length = x.hypot(y);
        if length == 0.0 {
            (0.0, 0.0)
        } else {
            (x / length, y / length)
        }
    };
    let incoming = normalize(incoming);
    let outgoing = normalize(outgoing);
    let sum = (incoming.0 + outgoing.0, incoming.1 + outgoing.1);
    if sum.0.abs() < f64::EPSILON && sum.1.abs() < f64::EPSILON {
        vector_angle(outgoing)
    } else {
        vector_angle(sum)
    }
}

#[derive(Clone, Copy)]
enum Axis {
    Horizontal,
    Vertical,
    Radius,
}

fn href(element: &XmlElement) -> Option<&str> {
    element
        .attribute_ns(None, "href")
        .or_else(|| element.attribute_ns(Some(XLINK_NAMESPACE), "href"))
}

fn effective_attribute<'a>(chain: &[&'a XmlElement], name: &str) -> Option<&'a str> {
    chain
        .iter()
        .rev()
        .find_map(|element| element.attribute_ns(None, name))
}

fn gradient_stops(chain: &[&XmlElement]) -> Result<Vec<GradientStop>, SvgReadError> {
    let owner = chain
        .iter()
        .rev()
        .find(|element| {
            element.children().iter().any(|child| {
                child
                    .as_element()
                    .is_some_and(|element| element.local_name() == "stop")
            })
        })
        .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidValue))?;
    let mut stops = Vec::new();
    let mut previous = Scalar::ZERO;
    for child in owner.children() {
        let XmlNode::Element(stop) = child else {
            if child.as_text().is_some_and(|text| !text.trim().is_empty()) {
                return Err(SvgReadError::new(SvgReadErrorCode::Unsupported));
            }
            continue;
        };
        if stop.namespace_uri().is_some_and(|uri| uri != SVG_NAMESPACE)
            || stop.local_name() != "stop"
        {
            return Err(SvgReadError::new(SvgReadErrorCode::Unsupported));
        }
        if stops.len() == Gradient::MAX_STOPS {
            return Err(SvgReadError::new(SvgReadErrorCode::ResourceLimit));
        }
        let offset = stop
            .attribute_ns(None, "offset")
            .map(parse_length_or_percentage)
            .transpose()?
            .unwrap_or(Scalar::ZERO);
        let offset = Scalar::from_bits(offset.bits().clamp(previous.bits(), 1 << 16));
        previous = offset;
        let color_value = style_property(stop, "stop-color").unwrap_or("black");
        let PaintSource::Color(color) = parse_paint(color_value)?
            .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidValue))?
        else {
            return Err(SvgReadError::new(SvgReadErrorCode::Unsupported));
        };
        let opacity = style_property(stop, "stop-opacity")
            .map(parse_opacity)
            .transpose()?
            .unwrap_or(u8::MAX);
        stops.push(GradientStop::new(offset, color.with_opacity(opacity)).map_err(map_core_error)?);
    }
    if stops.len() < 2 {
        return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
    }
    Ok(stops)
}

fn parse_length_or_percentage(value: &str) -> Result<Scalar, SvgReadError> {
    parse_length_or_percentage_with_kind(value).map(|(value, _)| value)
}

fn parse_length_or_percentage_with_kind(value: &str) -> Result<(Scalar, bool), SvgReadError> {
    let value = value.trim();
    if let Some(percentage) = value.strip_suffix('%') {
        let percentage = parse_scalar(percentage)?;
        return Ok((scalar_from_f64(scalar_to_f64(percentage) / 100.0)?, true));
    }
    Ok((parse_length(value)?, false))
}

fn multiply_scalar(left: Scalar, right: Scalar) -> Result<Scalar, SvgReadError> {
    let product = i64::from(left.bits())
        .checked_mul(i64::from(right.bits()))
        .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidGeometry))?;
    let rounded = if product >= 0 {
        (product + (1 << 15)) >> 16
    } else {
        -((-product + (1 << 15)) >> 16)
    };
    i32::try_from(rounded)
        .map(Scalar::from_bits)
        .map_err(|_| SvgReadError::new(SvgReadErrorCode::InvalidGeometry))
}

fn object_box_fraction(value: &str) -> Result<Scalar, SvgReadError> {
    let value = value.trim();
    if let Some(percentage) = value.strip_suffix('%') {
        return scalar_from_f64(scalar_to_f64(parse_scalar(percentage)?) / 100.0);
    }
    if value
        .bytes()
        .any(|byte| byte.is_ascii_alphabetic() || byte == b'%')
    {
        return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
    }
    parse_scalar(value)
}

fn object_box_coordinate(
    value: Option<&str>,
    origin: Scalar,
    extent: Scalar,
    default: Scalar,
) -> Result<Scalar, SvgReadError> {
    let fraction = value
        .map(object_box_fraction)
        .transpose()?
        .unwrap_or(default);
    add(origin, multiply_scalar(extent, fraction)?)
}

fn object_box_extent(value: Option<&str>, extent: Scalar) -> Result<Scalar, SvgReadError> {
    let fraction = value
        .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidValue))
        .and_then(object_box_fraction)?;
    let result = multiply_scalar(extent, fraction)?;
    if result.bits() <= 0 {
        return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
    }
    Ok(result)
}

fn tile_range(
    minimum: Scalar,
    maximum: Scalar,
    base: Scalar,
    extent: Scalar,
) -> Result<Vec<i32>, SvgReadError> {
    if maximum <= minimum || extent.bits() <= 0 {
        return Err(SvgReadError::new(SvgReadErrorCode::InvalidGeometry));
    }
    let extent_f64 = scalar_to_f64(extent);
    let start = ((scalar_to_f64(minimum) - scalar_to_f64(base)) / extent_f64).floor();
    let end = ((scalar_to_f64(maximum) - scalar_to_f64(base)) / extent_f64).ceil() - 1.0;
    if !start.is_finite()
        || !end.is_finite()
        || start < f64::from(i32::MIN)
        || end > f64::from(i32::MAX)
        || end < start
    {
        return Err(SvgReadError::new(SvgReadErrorCode::ResourceLimit));
    }
    let start = start as i32;
    let end = end as i32;
    let length = i64::from(end)
        .checked_sub(i64::from(start))
        .and_then(|value| value.checked_add(1))
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::ResourceLimit))?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(length)
        .map_err(|_| SvgReadError::new(SvgReadErrorCode::AllocationFailed))?;
    output.extend(start..=end);
    Ok(output)
}

fn tile_position(base: Scalar, extent: Scalar, index: i32) -> Result<Scalar, SvgReadError> {
    let offset = i64::from(extent.bits())
        .checked_mul(i64::from(index))
        .and_then(|value| value.checked_add(i64::from(base.bits())))
        .and_then(|value| i32::try_from(value).ok())
        .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidGeometry))?;
    Ok(Scalar::from_bits(offset))
}

fn decode_image_data_uri(value: &str, maximum: usize) -> Result<Vec<u8>, SvgReadError> {
    let (metadata, payload) = value
        .strip_prefix("data:")
        .and_then(|value| value.split_once(','))
        .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::Unsupported))?;
    let media_type = metadata
        .strip_suffix(";base64")
        .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::Unsupported))?;
    if !matches!(media_type, "image/png" | "image/jpeg" | "image/webp") {
        return Err(SvgReadError::new(SvgReadErrorCode::Unsupported));
    }
    decode_base64(payload, maximum)
}

fn decode_base64(source: &str, maximum: usize) -> Result<Vec<u8>, SvgReadError> {
    let mut output = Vec::new();
    output
        .try_reserve((source.len() / 4).saturating_mul(3).min(maximum))
        .map_err(|_| SvgReadError::new(SvgReadErrorCode::AllocationFailed))?;
    let mut quartet = [0_u8; 4];
    let mut count = 0;
    let mut finished = false;
    for byte in source.bytes().filter(|byte| !byte.is_ascii_whitespace()) {
        if finished {
            return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
        }
        quartet[count] = byte;
        count += 1;
        if count != 4 {
            continue;
        }
        let first = base64_value(quartet[0])?;
        let second = base64_value(quartet[1])?;
        let third_padding = quartet[2] == b'=';
        let fourth_padding = quartet[3] == b'=';
        if third_padding && !fourth_padding {
            return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
        }
        let third = if third_padding {
            0
        } else {
            base64_value(quartet[2])?
        };
        let fourth = if fourth_padding {
            0
        } else {
            base64_value(quartet[3])?
        };
        if third_padding && second & 0x0F != 0
            || fourth_padding && !third_padding && third & 0x03 != 0
        {
            return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
        }
        push_limited(&mut output, (first << 2) | (second >> 4), maximum)?;
        if !third_padding {
            push_limited(&mut output, (second << 4) | (third >> 2), maximum)?;
        }
        if !fourth_padding {
            push_limited(&mut output, (third << 6) | fourth, maximum)?;
        }
        finished = third_padding || fourth_padding;
        count = 0;
    }
    if count != 0 || output.is_empty() {
        return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
    }
    Ok(output)
}

fn base64_value(byte: u8) -> Result<u8, SvgReadError> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err(SvgReadError::new(SvgReadErrorCode::InvalidValue)),
    }
}

fn push_limited(output: &mut Vec<u8>, byte: u8, maximum: usize) -> Result<(), SvgReadError> {
    if output.len() >= maximum {
        return Err(SvgReadError::new(SvgReadErrorCode::ResourceLimit));
    }
    output
        .try_reserve(1)
        .map_err(|_| SvgReadError::new(SvgReadErrorCode::AllocationFailed))?;
    output.push(byte);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn image_destination(
    x: Scalar,
    y: Scalar,
    width: Scalar,
    height: Scalar,
    image_width: u32,
    image_height: u32,
    preserve_aspect_ratio: &str,
) -> Result<Rect, SvgReadError> {
    if preserve_aspect_ratio.trim() == "none" {
        return Rect::new(x, y, add(x, width)?, add(y, height)?).map_err(map_core_error);
    }
    if !matches!(preserve_aspect_ratio.trim(), "xMidYMid" | "xMidYMid meet") {
        return Err(SvgReadError::new(SvgReadErrorCode::Unsupported));
    }
    let width_f64 = scalar_to_f64(width);
    let height_f64 = scalar_to_f64(height);
    let scale = (width_f64 / f64::from(image_width)).min(height_f64 / f64::from(image_height));
    let drawn_width = f64::from(image_width) * scale;
    let drawn_height = f64::from(image_height) * scale;
    let left = scalar_from_f64(scalar_to_f64(x) + (width_f64 - drawn_width) / 2.0)?;
    let top = scalar_from_f64(scalar_to_f64(y) + (height_f64 - drawn_height) / 2.0)?;
    Rect::new(
        left,
        top,
        add(left, scalar_from_f64(drawn_width)?)?,
        add(top, scalar_from_f64(drawn_height)?)?,
    )
    .map_err(map_core_error)
}

fn nested_length(
    value: Option<&str>,
    reference: Scalar,
    default: Scalar,
) -> Result<Scalar, SvgReadError> {
    let Some(value) = value else {
        return Ok(default);
    };
    let (value, percentage) = parse_length_or_percentage_with_kind(value)?;
    if percentage {
        multiply_scalar(reference, value)
    } else {
        Ok(value)
    }
}

fn viewport_mapping(
    viewport: Rect,
    view_box: Rect,
    preserve_aspect_ratio: &str,
) -> Result<Transform, SvgReadError> {
    let viewport_width = scalar_to_f64(subtract(viewport.right(), viewport.left())?);
    let viewport_height = scalar_to_f64(subtract(viewport.bottom(), viewport.top())?);
    let view_width = scalar_to_f64(subtract(view_box.right(), view_box.left())?);
    let view_height = scalar_to_f64(subtract(view_box.bottom(), view_box.top())?);
    let mut tokens = preserve_aspect_ratio.split_ascii_whitespace();
    let first = tokens.next().unwrap_or("xMidYMid");
    let align = if first == "defer" {
        tokens.next().unwrap_or("xMidYMid")
    } else {
        first
    };
    if align == "none" {
        if tokens.next().is_some() {
            return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
        }
        let scale_x = viewport_width / view_width;
        let scale_y = viewport_height / view_height;
        return Ok(Transform::new(
            scalar_from_f64(scale_x)?,
            Scalar::ZERO,
            Scalar::ZERO,
            scalar_from_f64(scale_y)?,
            scalar_from_f64(
                scalar_to_f64(viewport.left()) - scalar_to_f64(view_box.left()) * scale_x,
            )?,
            scalar_from_f64(
                scalar_to_f64(viewport.top()) - scalar_to_f64(view_box.top()) * scale_y,
            )?,
        ));
    }
    let mode = tokens.next().unwrap_or("meet");
    if tokens.next().is_some() || !matches!(mode, "meet" | "slice") {
        return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
    }
    let (horizontal, vertical) = match align {
        "xMinYMin" => (0.0, 0.0),
        "xMidYMin" => (0.5, 0.0),
        "xMaxYMin" => (1.0, 0.0),
        "xMinYMid" => (0.0, 0.5),
        "xMidYMid" => (0.5, 0.5),
        "xMaxYMid" => (1.0, 0.5),
        "xMinYMax" => (0.0, 1.0),
        "xMidYMax" => (0.5, 1.0),
        "xMaxYMax" => (1.0, 1.0),
        _ => return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue)),
    };
    let scale = if mode == "meet" {
        (viewport_width / view_width).min(viewport_height / view_height)
    } else {
        (viewport_width / view_width).max(viewport_height / view_height)
    };
    let offset_x = (viewport_width - view_width * scale) * horizontal;
    let offset_y = (viewport_height - view_height * scale) * vertical;
    Ok(Transform::new(
        scalar_from_f64(scale)?,
        Scalar::ZERO,
        Scalar::ZERO,
        scalar_from_f64(scale)?,
        scalar_from_f64(
            scalar_to_f64(viewport.left()) + offset_x - scalar_to_f64(view_box.left()) * scale,
        )?,
        scalar_from_f64(
            scalar_to_f64(viewport.top()) + offset_y - scalar_to_f64(view_box.top()) * scale,
        )?,
    ))
}

fn parse_preserve_aspect_ratio(value: &str) -> Result<SvgPreserveAspectRatio, SvgReadError> {
    let mut tokens = value.split_ascii_whitespace();
    let first = tokens.next().unwrap_or("xMidYMid");
    let alignment = if first == "defer" {
        tokens.next().unwrap_or("xMidYMid")
    } else {
        first
    };
    let alignment = match alignment {
        "none" => SvgViewBoxAlignment::None,
        "xMinYMin" => SvgViewBoxAlignment::XMinYMin,
        "xMidYMin" => SvgViewBoxAlignment::XMidYMin,
        "xMaxYMin" => SvgViewBoxAlignment::XMaxYMin,
        "xMinYMid" => SvgViewBoxAlignment::XMinYMid,
        "xMidYMid" => SvgViewBoxAlignment::XMidYMid,
        "xMaxYMid" => SvgViewBoxAlignment::XMaxYMid,
        "xMinYMax" => SvgViewBoxAlignment::XMinYMax,
        "xMidYMax" => SvgViewBoxAlignment::XMidYMax,
        "xMaxYMax" => SvgViewBoxAlignment::XMaxYMax,
        _ => return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue)),
    };
    let scale = match tokens.next() {
        None | Some("meet") => SvgViewBoxScale::Meet,
        Some("slice") => SvgViewBoxScale::Slice,
        Some(_) => return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue)),
    };
    if tokens.next().is_some()
        || alignment == SvgViewBoxAlignment::None && scale != SvgViewBoxScale::Meet
    {
        return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
    }
    Ok(SvgPreserveAspectRatio::new(alignment, scale))
}

fn parse_style(inherited: &Style, cascade: &CascadedStyle) -> Result<Style, SvgReadError> {
    let mut style = inherited.clone();
    for name in [
        "fill",
        "stroke",
        "fill-rule",
        "stroke-width",
        "stroke-linecap",
        "stroke-linejoin",
        "stroke-miterlimit",
        "stroke-dasharray",
        "stroke-dashoffset",
        "fill-opacity",
        "stroke-opacity",
        "visibility",
        "font-size",
        "font-family",
        "font-style",
        "font-weight",
        "font-stretch",
        "text-anchor",
        "direction",
        "marker",
        "marker-start",
        "marker-mid",
        "marker-end",
    ] {
        if let Some(value) = cascade.property(name) {
            apply_style(&mut style, name, value)?;
        }
    }
    Ok(style)
}

fn style_property<'a>(element: &'a XmlElement, requested: &str) -> Option<&'a str> {
    let mut result = element.attribute(requested);
    if let Some(declarations) = element.attribute("style") {
        for declaration in declarations.split(';') {
            let Some((name, value)) = declaration.split_once(':') else {
                continue;
            };
            if name.trim() == requested {
                result = Some(value.trim());
            }
        }
    }
    result
}

fn apply_style(style: &mut Style, name: &str, value: &str) -> Result<(), SvgReadError> {
    if value.trim() == "inherit"
        && matches!(
            name,
            "fill"
                | "stroke"
                | "fill-rule"
                | "stroke-width"
                | "stroke-linecap"
                | "stroke-linejoin"
                | "stroke-miterlimit"
                | "stroke-dasharray"
                | "stroke-dashoffset"
                | "fill-opacity"
                | "stroke-opacity"
                | "visibility"
                | "font-size"
                | "font-family"
                | "font-style"
                | "font-weight"
                | "font-stretch"
                | "text-anchor"
                | "direction"
                | "marker"
                | "marker-start"
                | "marker-mid"
                | "marker-end"
        )
    {
        return Ok(());
    }
    match name {
        "fill" => style.fill = parse_paint(value)?,
        "stroke" => style.stroke = parse_paint(value)?,
        "fill-rule" => {
            style.fill_rule = match value.trim() {
                "nonzero" => FillRule::NonZero,
                "evenodd" => FillRule::EvenOdd,
                _ => return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue)),
            }
        }
        "stroke-width" => style.stroke_width = parse_nonnegative_length(value)?,
        "stroke-linecap" => {
            style.line_cap = match value.trim() {
                "butt" => StrokeCap::Butt,
                "round" => StrokeCap::Round,
                "square" => StrokeCap::Square,
                _ => return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue)),
            }
        }
        "stroke-linejoin" => {
            style.line_join = match value.trim() {
                "miter" => StrokeJoin::Miter,
                "round" => StrokeJoin::Round,
                "bevel" => StrokeJoin::Bevel,
                _ => return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue)),
            }
        }
        "stroke-miterlimit" => {
            let value = parse_scalar(value)?;
            if value.bits() < 1 << 16 {
                return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
            }
            style.miter_limit = value;
        }
        "stroke-dasharray" => {
            if value.trim() == "none" {
                style.dash_pattern.clear();
            } else {
                let mut pattern = NumberList::parse_all(value)?;
                if pattern.is_empty() || pattern.iter().any(|value| value.bits() <= 0) {
                    return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
                }
                if pattern.len() % 2 != 0 {
                    let repeated = pattern.clone();
                    pattern.extend(repeated);
                }
                style.dash_pattern = pattern;
            }
        }
        "stroke-dashoffset" => style.dash_offset = parse_length(value)?,
        "fill-opacity" => style.fill_opacity = parse_opacity(value)?,
        "stroke-opacity" => style.stroke_opacity = parse_opacity(value)?,
        "visibility" => {
            style.visible = match value.trim() {
                "visible" => true,
                "hidden" | "collapse" => false,
                _ => return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue)),
            }
        }
        "font-size" => style.font_size = parse_positive_length(value)?,
        "font-family" => style.font_families = parse_font_families(value)?,
        "font-style" => {
            let slant = match value.trim() {
                "normal" => FontSlant::Normal,
                "italic" => FontSlant::Italic,
                value if value == "oblique" || value.starts_with("oblique ") => FontSlant::Oblique,
                _ => return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue)),
            };
            style.font_style =
                FontStyle::new(style.font_style.weight(), style.font_style.width(), slant)
                    .map_err(map_text_error)?;
        }
        "font-weight" => {
            let weight = match value.trim() {
                "normal" => 400,
                "bold" => 700,
                "bolder" => style.font_style.weight().saturating_add(300).min(1000),
                "lighter" => style.font_style.weight().saturating_sub(300).max(1),
                value => value
                    .parse::<u16>()
                    .ok()
                    .filter(|value| (1..=1000).contains(value))
                    .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidValue))?,
            };
            style.font_style =
                FontStyle::new(weight, style.font_style.width(), style.font_style.slant())
                    .map_err(map_text_error)?;
        }
        "font-stretch" => {
            let width = match value.trim() {
                "ultra-condensed" => FontWidth::UltraCondensed,
                "extra-condensed" => FontWidth::ExtraCondensed,
                "condensed" => FontWidth::Condensed,
                "semi-condensed" => FontWidth::SemiCondensed,
                "normal" => FontWidth::Normal,
                "semi-expanded" => FontWidth::SemiExpanded,
                "expanded" => FontWidth::Expanded,
                "extra-expanded" => FontWidth::ExtraExpanded,
                "ultra-expanded" => FontWidth::UltraExpanded,
                _ => return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue)),
            };
            style.font_style =
                FontStyle::new(style.font_style.weight(), width, style.font_style.slant())
                    .map_err(map_text_error)?;
        }
        "text-anchor" => {
            style.text_anchor = match value.trim() {
                "start" => TextAnchor::Start,
                "middle" => TextAnchor::Middle,
                "end" => TextAnchor::End,
                _ => return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue)),
            }
        }
        "direction" => {
            style.text_direction = match value.trim() {
                "ltr" => Some(TextDirection::LeftToRight),
                "rtl" => Some(TextDirection::RightToLeft),
                _ => return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue)),
            }
        }
        "marker" => {
            let reference = parse_local_resource_property(value)?;
            style.marker_start.clone_from(&reference);
            style.marker_mid.clone_from(&reference);
            style.marker_end = reference;
        }
        "marker-start" => style.marker_start = parse_local_resource_property(value)?,
        "marker-mid" => style.marker_mid = parse_local_resource_property(value)?,
        "marker-end" => style.marker_end = parse_local_resource_property(value)?,
        "style"
        | "opacity"
        | "transform"
        | "display"
        | "id"
        | "class"
        | "xmlns"
        | "viewBox"
        | "preserveAspectRatio"
        | "version"
        | "href"
        | "xlink:href"
        | "image-rendering"
        | "overflow"
        | "gradientUnits"
        | "gradientTransform"
        | "spreadMethod"
        | "offset"
        | "stop-color"
        | "stop-opacity"
        | "clip-path"
        | "clip-rule"
        | "clipPathUnits"
        | "dx"
        | "dy"
        | "fx"
        | "fy"
        | "fr"
        | "width"
        | "height"
        | "x"
        | "y"
        | "x1"
        | "y1"
        | "x2"
        | "y2"
        | "cx"
        | "cy"
        | "r"
        | "rx"
        | "ry"
        | "points"
        | "d" => {}
        "mask" | "filter" | "paint-order" | "vector-effect" => {
            return Err(SvgReadError::new(SvgReadErrorCode::Unsupported));
        }
        name if name.starts_with("xmlns:")
            || name.starts_with("xml:")
            || name.starts_with("data-")
            || name.starts_with("aria-") => {}
        _ => return Err(SvgReadError::new(SvgReadErrorCode::Unsupported)),
    }
    Ok(())
}

fn parse_font_families(value: &str) -> Result<Vec<String>, SvgReadError> {
    let mut families = Vec::new();
    for family in value.split(',') {
        let family = family.trim();
        let family = if family.len() >= 2
            && ((family.starts_with('"') && family.ends_with('"'))
                || (family.starts_with('\'') && family.ends_with('\'')))
        {
            &family[1..family.len() - 1]
        } else {
            family
        };
        if family.is_empty()
            || family
                .chars()
                .any(|ch| ch.is_control() || matches!(ch, '"' | '\''))
        {
            return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
        }
        families
            .try_reserve(1)
            .map_err(|_| SvgReadError::new(SvgReadErrorCode::AllocationFailed))?;
        families.push(family.to_owned());
    }
    if families.is_empty() {
        return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
    }
    Ok(families)
}

fn normalize_text(source: &str, cursor: &mut TextCursor) -> String {
    let starts_with_space = source.chars().next().is_some_and(char::is_whitespace);
    let ends_with_space = source.chars().next_back().is_some_and(char::is_whitespace);
    let words = source.split_whitespace().collect::<Vec<_>>();
    if words.is_empty() {
        cursor.pending_space |= !source.is_empty();
        return String::new();
    }
    let mut output = String::new();
    if cursor.emitted && (cursor.pending_space || starts_with_space) {
        output.push(' ');
    }
    output.push_str(&words.join(" "));
    cursor.pending_space = ends_with_space;
    output
}

fn first_length(value: &str) -> Result<Scalar, SvgReadError> {
    let first = value
        .split(|ch: char| ch.is_ascii_whitespace() || ch == ',')
        .find(|value| !value.is_empty())
        .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidValue))?;
    parse_length(first)
}

fn parse_paint(value: &str) -> Result<Option<PaintSource>, SvgReadError> {
    let value = value.trim();
    if value == "none" {
        return Ok(None);
    }
    if let Some(id) = local_url_reference(value) {
        return Ok(Some(PaintSource::Reference(id.to_owned())));
    }
    if value.starts_with("url(") || value == "currentColor" {
        return Err(SvgReadError::new(SvgReadErrorCode::Unsupported));
    }
    let color = match value {
        "black" => Color::BLACK,
        "white" => Color::WHITE,
        "red" => Color::RED,
        "green" => Color::rgb(0, 128, 0),
        "blue" => Color::BLUE,
        "yellow" => Color::rgb(255, 255, 0),
        "cyan" | "aqua" => Color::rgb(0, 255, 255),
        "magenta" | "fuchsia" => Color::rgb(255, 0, 255),
        "gray" | "grey" => Color::rgb(128, 128, 128),
        "transparent" => Color::TRANSPARENT,
        _ if value.starts_with('#') => parse_hex_color(value)?,
        _ => return Err(SvgReadError::new(SvgReadErrorCode::Unsupported)),
    };
    Ok(Some(PaintSource::Color(color)))
}

fn local_url_reference(value: &str) -> Option<&str> {
    value
        .strip_prefix("url(#")
        .and_then(|value| value.strip_suffix(')'))
        .filter(|id| !id.is_empty())
}

fn parse_local_resource_property(value: &str) -> Result<Option<String>, SvgReadError> {
    let value = value.trim();
    if value == "none" {
        return Ok(None);
    }
    local_url_reference(value)
        .map(|id| Some(id.to_owned()))
        .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::Unsupported))
}

fn parse_hex_color(value: &str) -> Result<Color, SvgReadError> {
    let digits = &value[1..];
    let byte = |pair: &str| {
        u8::from_str_radix(pair, 16).map_err(|_| SvgReadError::new(SvgReadErrorCode::InvalidValue))
    };
    match digits.len() {
        3 => {
            let mut chars = digits.chars();
            let expand = |ch: char| {
                ch.to_digit(16)
                    .map(|value| (value as u8) * 17)
                    .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidValue))
            };
            Ok(Color::rgb(
                expand(chars.next().unwrap_or_default())?,
                expand(chars.next().unwrap_or_default())?,
                expand(chars.next().unwrap_or_default())?,
            ))
        }
        4 => {
            let mut chars = digits.chars();
            let expand = |ch: char| {
                ch.to_digit(16)
                    .map(|value| (value as u8) * 17)
                    .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidValue))
            };
            Ok(Color::rgba(
                expand(chars.next().unwrap_or_default())?,
                expand(chars.next().unwrap_or_default())?,
                expand(chars.next().unwrap_or_default())?,
                expand(chars.next().unwrap_or_default())?,
            ))
        }
        6 => Ok(Color::rgb(
            byte(&digits[0..2])?,
            byte(&digits[2..4])?,
            byte(&digits[4..6])?,
        )),
        8 => Ok(Color::rgba(
            byte(&digits[0..2])?,
            byte(&digits[2..4])?,
            byte(&digits[4..6])?,
            byte(&digits[6..8])?,
        )),
        _ => Err(SvgReadError::new(SvgReadErrorCode::InvalidValue)),
    }
}

fn parse_view_box(value: &str) -> Result<Rect, SvgReadError> {
    let values = NumberList::parse_all(value)?;
    if values.len() != 4 || values[2].bits() <= 0 || values[3].bits() <= 0 {
        return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
    }
    Rect::new(
        values[0],
        values[1],
        add(values[0], values[2])?,
        add(values[1], values[3])?,
    )
    .map_err(map_core_error)
}

fn optional_length(element: &XmlElement, name: &str) -> Result<Scalar, SvgReadError> {
    element
        .attribute(name)
        .map(parse_length)
        .transpose()
        .map(|value| value.unwrap_or(Scalar::ZERO))
}

fn required_nonnegative_length(element: &XmlElement, name: &str) -> Result<Scalar, SvgReadError> {
    element
        .attribute(name)
        .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidValue))
        .and_then(parse_nonnegative_length)
}

fn parse_length(value: &str) -> Result<Scalar, SvgReadError> {
    let value = value.trim();
    let number = value.strip_suffix("px").unwrap_or(value);
    parse_scalar(number)
}

fn parse_positive_length(value: &str) -> Result<Scalar, SvgReadError> {
    let value = parse_length(value)?;
    if value.bits() <= 0 {
        return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
    }
    Ok(value)
}

fn parse_nonnegative_length(value: &str) -> Result<Scalar, SvgReadError> {
    let value = parse_length(value)?;
    if value.bits() < 0 {
        return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
    }
    Ok(value)
}

fn parse_opacity(value: &str) -> Result<u8, SvgReadError> {
    let scalar = parse_scalar(value)?;
    let bits = scalar.bits().clamp(0, 1 << 16);
    Ok(((i64::from(bits) * 255 + (1 << 15)) >> 16) as u8)
}

fn parse_transform(value: &str) -> Result<Transform, SvgReadError> {
    let mut source = value.trim();
    let mut result = Transform::IDENTITY;
    while !source.is_empty() {
        let open = source
            .find('(')
            .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidValue))?;
        let name = source[..open].trim();
        let close = source[open + 1..]
            .find(')')
            .map(|offset| open + 1 + offset)
            .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidValue))?;
        let values = NumberList::parse_all(&source[open + 1..close])?;
        let transform = match (name, values.as_slice()) {
            ("matrix", [a, b, c, d, e, f]) => Transform::new(*a, *b, *c, *d, *e, *f),
            ("translate", [x]) => Transform::translate(*x, Scalar::ZERO),
            ("translate", [x, y]) => Transform::translate(*x, *y),
            ("scale", [x]) => Transform::scale(*x, *x),
            ("scale", [x, y]) => Transform::scale(*x, *y),
            ("rotate", [angle]) => rotation_transform(*angle, Scalar::ZERO, Scalar::ZERO)?,
            ("rotate", [angle, center_x, center_y]) => {
                rotation_transform(*angle, *center_x, *center_y)?
            }
            ("skewX", [angle]) => skew_transform(*angle, true)?,
            ("skewY", [angle]) => skew_transform(*angle, false)?,
            _ => return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue)),
        };
        result = result.concat(transform).map_err(map_core_error)?;
        source = source[close + 1..].trim_start_matches(|ch: char| ch.is_whitespace() || ch == ',');
    }
    Ok(result)
}

fn parse_angle_degrees(value: &str) -> Result<f64, SvgReadError> {
    let value = value.trim();
    let (number, factor) = if let Some(number) = value.strip_suffix("deg") {
        (number, 1.0)
    } else if let Some(number) = value.strip_suffix("grad") {
        (number, 0.9)
    } else if let Some(number) = value.strip_suffix("rad") {
        (number, 180.0 / std::f64::consts::PI)
    } else if let Some(number) = value.strip_suffix("turn") {
        (number, 360.0)
    } else {
        (value, 1.0)
    };
    let degrees = scalar_to_f64(parse_scalar(number)?) * factor;
    if !degrees.is_finite() {
        return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
    }
    Ok(degrees)
}

fn rotation_transform(
    angle: Scalar,
    center_x: Scalar,
    center_y: Scalar,
) -> Result<Transform, SvgReadError> {
    let radians = scalar_to_f64(angle).to_radians();
    let cosine = radians.cos();
    let sine = radians.sin();
    let center_x_f64 = scalar_to_f64(center_x);
    let center_y_f64 = scalar_to_f64(center_y);
    Ok(Transform::new(
        scalar_from_f64(cosine)?,
        scalar_from_f64(sine)?,
        scalar_from_f64(-sine)?,
        scalar_from_f64(cosine)?,
        scalar_from_f64(center_x_f64 - cosine * center_x_f64 + sine * center_y_f64)?,
        scalar_from_f64(center_y_f64 - sine * center_x_f64 - cosine * center_y_f64)?,
    ))
}

fn skew_transform(angle: Scalar, horizontal: bool) -> Result<Transform, SvgReadError> {
    let tangent = scalar_to_f64(angle).to_radians().tan();
    let tangent = scalar_from_f64(tangent)?;
    let one = Scalar::from_bits(1 << 16);
    Ok(if horizontal {
        Transform::new(one, Scalar::ZERO, tangent, one, Scalar::ZERO, Scalar::ZERO)
    } else {
        Transform::new(one, tangent, Scalar::ZERO, one, Scalar::ZERO, Scalar::ZERO)
    })
}

fn scalar_to_f64(value: Scalar) -> f64 {
    f64::from(value.bits()) / 65_536.0
}

fn scalar_from_f64(value: f64) -> Result<Scalar, SvgReadError> {
    let bits = value * 65_536.0;
    if !bits.is_finite() || bits < f64::from(i32::MIN) || bits > f64::from(i32::MAX) {
        return Err(SvgReadError::new(SvgReadErrorCode::InvalidGeometry));
    }
    Ok(Scalar::from_bits(bits.round() as i32))
}

fn add(left: Scalar, right: Scalar) -> Result<Scalar, SvgReadError> {
    left.bits()
        .checked_add(right.bits())
        .map(Scalar::from_bits)
        .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidGeometry))
}

fn subtract(left: Scalar, right: Scalar) -> Result<Scalar, SvgReadError> {
    left.bits()
        .checked_sub(right.bits())
        .map(Scalar::from_bits)
        .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidGeometry))
}

fn reflect(control: Point, around: Point) -> Result<Point, SvgReadError> {
    Ok(Point::new(
        subtract(add(around.x(), around.x())?, control.x())?,
        subtract(add(around.y(), around.y())?, control.y())?,
    ))
}

struct NumberList<'a> {
    source: &'a str,
    offset: usize,
}

impl<'a> NumberList<'a> {
    fn new(source: &'a str) -> Self {
        Self { source, offset: 0 }
    }

    fn parse_all(source: &'a str) -> Result<Vec<Scalar>, SvgReadError> {
        let mut parser = Self::new(source);
        let mut values = Vec::new();
        while parser.skip_separators() {
            values.push(parser.number()?);
        }
        Ok(values)
    }

    fn skip_separators(&mut self) -> bool {
        while matches!(self.peek(), Some(ch) if ch.is_ascii_whitespace() || ch == ',') {
            self.offset += 1;
        }
        self.offset < self.source.len()
    }

    fn peek(&self) -> Option<char> {
        self.source
            .as_bytes()
            .get(self.offset)
            .copied()
            .map(char::from)
    }

    fn number(&mut self) -> Result<Scalar, SvgReadError> {
        self.skip_separators();
        let start = self.offset;
        if matches!(self.peek(), Some('+' | '-')) {
            self.offset += 1;
        }
        let mut digits = 0;
        while matches!(self.peek(), Some(ch) if ch.is_ascii_digit()) {
            self.offset += 1;
            digits += 1;
        }
        if self.peek() == Some('.') {
            self.offset += 1;
            while matches!(self.peek(), Some(ch) if ch.is_ascii_digit()) {
                self.offset += 1;
                digits += 1;
            }
        }
        if digits == 0 {
            return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            self.offset += 1;
            if matches!(self.peek(), Some('+' | '-')) {
                self.offset += 1;
            }
            let exponent_start = self.offset;
            while matches!(self.peek(), Some(ch) if ch.is_ascii_digit()) {
                self.offset += 1;
            }
            if self.offset == exponent_start {
                return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
            }
        }
        parse_scalar(&self.source[start..self.offset])
    }
}

fn parse_scalar(value: &str) -> Result<Scalar, SvgReadError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
    }
    let (negative, value) = match value.as_bytes()[0] {
        b'-' => (true, &value[1..]),
        b'+' => (false, &value[1..]),
        _ => (false, value),
    };
    let (mantissa, exponent) = value.find(['e', 'E']).map_or((value, 0_i32), |position| {
        let exponent = value[position + 1..].parse::<i32>().unwrap_or(i32::MAX);
        (&value[..position], exponent)
    });
    let fraction_digits = mantissa
        .split_once('.')
        .map_or(0, |(_, fraction)| fraction.len());
    let mut digits = mantissa.replace('.', "");
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
    }
    while digits.starts_with('0') && digits.len() > 1 {
        digits.remove(0);
    }
    let mut numerator = digits
        .parse::<i128>()
        .map_err(|_| SvgReadError::new(SvgReadErrorCode::InvalidValue))?;
    if negative {
        numerator = -numerator;
    }
    let power = exponent
        .checked_sub(i32::try_from(fraction_digits).unwrap_or(i32::MAX))
        .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidValue))?;
    if power.unsigned_abs() > 18 {
        return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue));
    }
    let scale = 10_i128
        .checked_pow(power.unsigned_abs())
        .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidValue))?;
    let (numerator, denominator) = if power >= 0 {
        (
            numerator
                .checked_mul(scale)
                .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidValue))?,
            1_i128,
        )
    } else {
        (numerator, scale)
    };
    Scalar::from_ratio(
        i64::try_from(numerator).map_err(|_| SvgReadError::new(SvgReadErrorCode::InvalidValue))?,
        i64::try_from(denominator)
            .map_err(|_| SvgReadError::new(SvgReadErrorCode::InvalidValue))?,
    )
    .map_err(map_core_error)
}

struct PathDataParser<'a> {
    numbers: NumberList<'a>,
    builder: PathBuilder,
    command: Option<char>,
    current: Point,
    contour_start: Point,
    last_cubic_control: Option<Point>,
    last_quad_control: Option<Point>,
}

impl<'a> PathDataParser<'a> {
    fn new(source: &'a str, max_verbs: usize) -> Result<Self, SvgReadError> {
        Ok(Self {
            numbers: NumberList::new(source),
            builder: PathBuilder::new(max_verbs).map_err(map_core_error)?,
            command: None,
            current: Point::new(Scalar::ZERO, Scalar::ZERO),
            contour_start: Point::new(Scalar::ZERO, Scalar::ZERO),
            last_cubic_control: None,
            last_quad_control: None,
        })
    }

    fn parse(mut self) -> Result<Path, SvgReadError> {
        while self.numbers.skip_separators() {
            if matches!(self.numbers.peek(), Some(ch) if ch.is_ascii_alphabetic()) {
                let command = self.numbers.peek().unwrap_or_default();
                self.numbers.offset += 1;
                self.command = Some(command);
                if matches!(command, 'Z' | 'z') {
                    self.builder.close().map_err(map_core_error)?;
                    self.current = self.contour_start;
                    self.reset_controls();
                    self.command = None;
                    continue;
                }
            }
            let command = self
                .command
                .ok_or_else(|| SvgReadError::new(SvgReadErrorCode::InvalidValue))?;
            self.segment(command)?;
        }
        self.builder.finish().map_err(map_core_error)
    }

    fn segment(&mut self, command: char) -> Result<(), SvgReadError> {
        let relative = command.is_ascii_lowercase();
        match command.to_ascii_uppercase() {
            'M' => {
                let point = self.point(relative)?;
                self.builder.move_to(point).map_err(map_core_error)?;
                self.current = point;
                self.contour_start = point;
                self.command = Some(if relative { 'l' } else { 'L' });
                self.reset_controls();
            }
            'L' => {
                let point = self.point(relative)?;
                self.builder.line_to(point).map_err(map_core_error)?;
                self.current = point;
                self.reset_controls();
            }
            'H' => {
                let x = self.coordinate(relative, self.current.x())?;
                let point = Point::new(x, self.current.y());
                self.builder.line_to(point).map_err(map_core_error)?;
                self.current = point;
                self.reset_controls();
            }
            'V' => {
                let y = self.coordinate(relative, self.current.y())?;
                let point = Point::new(self.current.x(), y);
                self.builder.line_to(point).map_err(map_core_error)?;
                self.current = point;
                self.reset_controls();
            }
            'C' => {
                let first = self.point(relative)?;
                let second = self.point(relative)?;
                let end = self.point(relative)?;
                self.builder
                    .cubic_to(first, second, end)
                    .map_err(map_core_error)?;
                self.current = end;
                self.last_cubic_control = Some(second);
                self.last_quad_control = None;
            }
            'S' => {
                let first = self
                    .last_cubic_control
                    .map(|control| reflect(control, self.current))
                    .transpose()?
                    .unwrap_or(self.current);
                let second = self.point(relative)?;
                let end = self.point(relative)?;
                self.builder
                    .cubic_to(first, second, end)
                    .map_err(map_core_error)?;
                self.current = end;
                self.last_cubic_control = Some(second);
                self.last_quad_control = None;
            }
            'Q' => {
                let control = self.point(relative)?;
                let end = self.point(relative)?;
                self.builder.quad_to(control, end).map_err(map_core_error)?;
                self.current = end;
                self.last_quad_control = Some(control);
                self.last_cubic_control = None;
            }
            'T' => {
                let control = self
                    .last_quad_control
                    .map(|control| reflect(control, self.current))
                    .transpose()?
                    .unwrap_or(self.current);
                let end = self.point(relative)?;
                self.builder.quad_to(control, end).map_err(map_core_error)?;
                self.current = end;
                self.last_quad_control = Some(control);
                self.last_cubic_control = None;
            }
            'A' => {
                let radius_x = self.numbers.number()?;
                let radius_y = self.numbers.number()?;
                let rotation = self.numbers.number()?;
                let large_arc = self.flag()?;
                let sweep = self.flag()?;
                let end = self.point(relative)?;
                append_svg_arc(
                    &mut self.builder,
                    self.current,
                    end,
                    radius_x,
                    radius_y,
                    rotation,
                    large_arc,
                    sweep,
                )?;
                self.current = end;
                self.reset_controls();
            }
            _ => return Err(SvgReadError::new(SvgReadErrorCode::InvalidValue)),
        }
        Ok(())
    }

    fn point(&mut self, relative: bool) -> Result<Point, SvgReadError> {
        let x = self.coordinate(relative, self.current.x())?;
        let y = self.coordinate(relative, self.current.y())?;
        Ok(Point::new(x, y))
    }

    fn coordinate(&mut self, relative: bool, base: Scalar) -> Result<Scalar, SvgReadError> {
        let value = self.numbers.number()?;
        if relative {
            add(base, value)
        } else {
            Ok(value)
        }
    }

    fn flag(&mut self) -> Result<bool, SvgReadError> {
        match self.numbers.number()?.bits() {
            0 => Ok(false),
            65_536 => Ok(true),
            _ => Err(SvgReadError::new(SvgReadErrorCode::InvalidValue)),
        }
    }

    fn reset_controls(&mut self) {
        self.last_cubic_control = None;
        self.last_quad_control = None;
    }
}

#[allow(clippy::too_many_arguments)]
fn append_svg_arc(
    builder: &mut PathBuilder,
    start: Point,
    end: Point,
    radius_x: Scalar,
    radius_y: Scalar,
    rotation: Scalar,
    large_arc: bool,
    sweep: bool,
) -> Result<(), SvgReadError> {
    if start == end {
        return Ok(());
    }
    let mut radius_x = scalar_to_f64(radius_x).abs();
    let mut radius_y = scalar_to_f64(radius_y).abs();
    if radius_x == 0.0 || radius_y == 0.0 {
        return builder.line_to(end).map_err(map_core_error);
    }

    let start_x = scalar_to_f64(start.x());
    let start_y = scalar_to_f64(start.y());
    let end_x = scalar_to_f64(end.x());
    let end_y = scalar_to_f64(end.y());
    let rotation_degrees = scalar_to_f64(rotation).rem_euclid(360.0);
    let radians = rotation_degrees.to_radians();
    let cosine = radians.cos();
    let sine = radians.sin();
    let half_x = (start_x - end_x) / 2.0;
    let half_y = (start_y - end_y) / 2.0;
    let transformed_x = cosine * half_x + sine * half_y;
    let transformed_y = -sine * half_x + cosine * half_y;

    let scale = transformed_x.powi(2) / radius_x.powi(2) + transformed_y.powi(2) / radius_y.powi(2);
    if scale > 1.0 {
        let scale = scale.sqrt();
        radius_x *= scale;
        radius_y *= scale;
    }

    let radius_x_squared = radius_x.powi(2);
    let radius_y_squared = radius_y.powi(2);
    let transformed_x_squared = transformed_x.powi(2);
    let transformed_y_squared = transformed_y.powi(2);
    let numerator = radius_x_squared * radius_y_squared
        - radius_x_squared * transformed_y_squared
        - radius_y_squared * transformed_x_squared;
    let denominator =
        radius_x_squared * transformed_y_squared + radius_y_squared * transformed_x_squared;
    if denominator == 0.0 {
        return builder.line_to(end).map_err(map_core_error);
    }
    let sign = if large_arc == sweep { -1.0 } else { 1.0 };
    let coefficient = sign * (numerator.max(0.0) / denominator).sqrt();
    let center_x_transformed = coefficient * radius_x * transformed_y / radius_y;
    let center_y_transformed = -coefficient * radius_y * transformed_x / radius_x;
    let center_x =
        cosine * center_x_transformed - sine * center_y_transformed + (start_x + end_x) / 2.0;
    let center_y =
        sine * center_x_transformed + cosine * center_y_transformed + (start_y + end_y) / 2.0;

    let unit_start_x = (transformed_x - center_x_transformed) / radius_x;
    let unit_start_y = (transformed_y - center_y_transformed) / radius_y;
    let unit_end_x = (-transformed_x - center_x_transformed) / radius_x;
    let unit_end_y = (-transformed_y - center_y_transformed) / radius_y;
    let start_angle = unit_start_y.atan2(unit_start_x);
    let mut sweep_angle = (unit_start_x * unit_end_y - unit_start_y * unit_end_x)
        .atan2(unit_start_x * unit_end_x + unit_start_y * unit_end_y);
    if !sweep && sweep_angle > 0.0 {
        sweep_angle -= std::f64::consts::TAU;
    } else if sweep && sweep_angle < 0.0 {
        sweep_angle += std::f64::consts::TAU;
    }
    if sweep_angle == 0.0 {
        return builder.line_to(end).map_err(map_core_error);
    }

    let bounds = Rect::new(
        scalar_from_f64(center_x - radius_x)?,
        scalar_from_f64(center_y - radius_y)?,
        scalar_from_f64(center_x + radius_x)?,
        scalar_from_f64(center_y + radius_y)?,
    )
    .map_err(map_core_error)?;
    builder
        .arc_to_rotated(
            bounds,
            angle_from_f64(rotation_degrees)?,
            angle_from_f64(start_angle.to_degrees())?,
            angle_from_f64(sweep_angle.to_degrees())?,
        )
        .map_err(map_core_error)
}

fn angle_from_f64(degrees: f64) -> Result<Angle, SvgReadError> {
    if !degrees.is_finite() {
        return Err(SvgReadError::new(SvgReadErrorCode::InvalidGeometry));
    }
    let scaled = degrees * 1_000_000.0;
    if scaled < i64::MIN as f64 || scaled > i64::MAX as f64 {
        return Err(SvgReadError::new(SvgReadErrorCode::InvalidGeometry));
    }
    Angle::from_degrees_ratio(scaled.round() as i64, 1_000_000).map_err(map_core_error)
}
