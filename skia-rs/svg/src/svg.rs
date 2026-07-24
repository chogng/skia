use std::{
    collections::HashMap,
    fmt,
    io::{self, Write},
};

use skia_codec::{
    CodecErrorCode, EncodeFormat, EncodeLimits, EncodeOptions, ImageAsset, ImageCodec, PngOptions,
};
use skia_core::{
    BlendMode, ClipOp, Color, DisplayList, DrawCommand, FillRule, Gradient, GradientGeometry,
    ImageId, Paint, Path, PathVerb, Rect, SamplingFilter, Scalar, StrokeAlign, StrokeCap,
    StrokeJoin, StrokeOptions, TileMode, Transform,
};
use skia_image::{ColorSpace, Image};

const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";

/// Stable machine-readable SVG failure code.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SvgErrorCode {
    /// Canvas dimensions or its view box are invalid.
    InvalidCanvas,
    /// Save/restore commands are unbalanced.
    InvalidState,
    /// A configured resource ceiling is zero or otherwise invalid.
    InvalidLimits,
    /// A command, resource, path, embedded image, or output ceiling was exceeded.
    ResourceLimit,
    /// A display-list resource reference or path command sequence is invalid.
    InvalidResource,
    /// The SVG mapping cannot preserve a drawing semantic.
    Unsupported,
    /// Text requires an explicit outline or embedded-font policy.
    UnsupportedText,
    /// An image is outside the sRGB SVG output contract.
    UnsupportedColorProfile,
    /// Fixed-point or size arithmetic overflowed.
    NumericOverflow,
    /// Memory required for bounded serialization could not be allocated.
    AllocationFailed,
    /// The destination writer failed.
    Io,
}

/// Source-redacted SVG error with an optional I/O category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SvgError {
    code: SvgErrorCode,
    io_kind: Option<io::ErrorKind>,
}

impl SvgError {
    const fn new(code: SvgErrorCode) -> Self {
        Self {
            code,
            io_kind: None,
        }
    }

    fn io(error: &io::Error) -> Self {
        Self {
            code: SvgErrorCode::Io,
            io_kind: Some(error.kind()),
        }
    }

    /// Returns the stable error category.
    pub const fn code(self) -> SvgErrorCode {
        self.code
    }

    /// Returns the destination I/O category, when applicable.
    pub const fn io_kind(self) -> Option<io::ErrorKind> {
        self.io_kind
    }
}

impl fmt::Display for SvgError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.io_kind {
            Some(kind) => write!(formatter, "{:?} ({kind:?})", self.code),
            None => write!(formatter, "{:?}", self.code),
        }
    }
}

impl std::error::Error for SvgError {}

/// Positive single-canvas dimensions and its logical SVG view box.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SvgCanvasSpec {
    width: Scalar,
    height: Scalar,
    view_box: Rect,
}

impl SvgCanvasSpec {
    /// Creates a canvas whose view box starts at zero and matches its dimensions.
    pub fn new(width: Scalar, height: Scalar) -> Result<Self, SvgError> {
        if width.bits() <= 0 || height.bits() <= 0 {
            return Err(SvgError::new(SvgErrorCode::InvalidCanvas));
        }
        let view_box = Rect::new(Scalar::ZERO, Scalar::ZERO, width, height)
            .map_err(|_| SvgError::new(SvgErrorCode::InvalidCanvas))?;
        Ok(Self {
            width,
            height,
            view_box,
        })
    }

    /// Replaces the logical view box without changing the rendered dimensions.
    pub const fn with_view_box(mut self, view_box: Rect) -> Self {
        self.view_box = view_box;
        self
    }

    /// Returns the rendered canvas width.
    pub const fn width(self) -> Scalar {
        self.width
    }

    /// Returns the rendered canvas height.
    pub const fn height(self) -> Scalar {
        self.height
    }

    /// Returns the logical view box.
    pub const fn view_box(self) -> Rect {
        self.view_box
    }
}

/// Hard ceilings for one SVG serialization operation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SvgLimits {
    /// Maximum display-list command count.
    pub max_commands: usize,
    /// Maximum combined gradient, clip, and image resource count.
    pub max_resources: usize,
    /// Maximum verb count accepted from one path resource.
    pub max_path_verbs: usize,
    /// Maximum PNG bytes retained for one embedded image.
    pub max_embedded_image_bytes: usize,
    /// Maximum serialized SVG bytes.
    pub max_output_bytes: usize,
}

impl SvgLimits {
    /// Validates that every ceiling is positive.
    pub fn validate(self) -> Result<Self, SvgError> {
        if self.max_commands == 0
            || self.max_resources == 0
            || self.max_path_verbs == 0
            || self.max_embedded_image_bytes == 0
            || self.max_output_bytes == 0
        {
            return Err(SvgError::new(SvgErrorCode::InvalidLimits));
        }
        Ok(self)
    }
}

impl Default for SvgLimits {
    fn default() -> Self {
        Self {
            max_commands: 100_000,
            max_resources: 4_096,
            max_path_verbs: 1_000_000,
            max_embedded_image_bytes: 64 * 1024 * 1024,
            max_output_bytes: 256 * 1024 * 1024,
        }
    }
}

/// SVG serialization policy.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct SvgOptions {
    /// Resource ceilings for validation and serialization.
    pub limits: SvgLimits,
}

/// Stateless encoder for one portable display list and one SVG canvas.
pub struct SvgWriter;

impl SvgWriter {
    /// Compiles a complete deterministic UTF-8 SVG document in memory.
    ///
    /// Compilation is transactional: unsupported content and resource failures
    /// return before any destination writer is involved.
    pub fn encode(
        spec: SvgCanvasSpec,
        list: &DisplayList,
        options: SvgOptions,
    ) -> Result<Vec<u8>, SvgError> {
        let limits = options.limits.validate()?;
        Compiler::new(spec, list, limits)?.compile()
    }

    /// Compiles and writes a complete deterministic UTF-8 SVG document.
    ///
    /// No bytes are written when compilation fails. An I/O failure can still
    /// leave a prefix in a destination that does not provide atomic writes.
    pub fn write(
        writer: &mut (impl Write + ?Sized),
        spec: SvgCanvasSpec,
        list: &DisplayList,
        options: SvgOptions,
    ) -> Result<(), SvgError> {
        let bytes = Self::encode(spec, list, options)?;
        writer
            .write_all(&bytes)
            .map_err(|error| SvgError::io(&error))
    }
}

struct Compiler<'a> {
    spec: SvgCanvasSpec,
    list: &'a DisplayList,
    limits: SvgLimits,
    parts: OutputParts,
    transform: Transform,
    stack: Vec<StateFrame>,
    clip_depth: usize,
    resource_count: usize,
    gradient_ids: HashMap<(Gradient, u8), usize>,
    image_ids: HashMap<ImageId, usize>,
    next_gradient_id: usize,
    next_clip_id: usize,
    next_image_id: usize,
}

#[derive(Clone, Copy)]
struct StateFrame {
    transform: Transform,
    clip_depth: usize,
}

impl<'a> Compiler<'a> {
    fn new(
        spec: SvgCanvasSpec,
        list: &'a DisplayList,
        limits: SvgLimits,
    ) -> Result<Self, SvgError> {
        if list.commands().len() > limits.max_commands {
            return Err(SvgError::new(SvgErrorCode::ResourceLimit));
        }
        let mut stack = Vec::new();
        stack
            .try_reserve(list.commands().len().min(limits.max_commands))
            .map_err(|_| SvgError::new(SvgErrorCode::AllocationFailed))?;
        Ok(Self {
            spec,
            list,
            limits,
            parts: OutputParts::new(limits.max_output_bytes),
            transform: Transform::IDENTITY,
            stack,
            clip_depth: 0,
            resource_count: 0,
            gradient_ids: HashMap::new(),
            image_ids: HashMap::new(),
            next_gradient_id: 1,
            next_clip_id: 1,
            next_image_id: 1,
        })
    }

    fn compile(mut self) -> Result<Vec<u8>, SvgError> {
        for (index, command) in self.list.commands().iter().enumerate() {
            self.compile_command(index, command)?;
        }
        if !self.stack.is_empty() {
            return Err(SvgError::new(SvgErrorCode::InvalidState));
        }
        self.close_clips_to(0)?;
        self.parts.finish(self.spec)
    }

    fn compile_command(
        &mut self,
        command_index: usize,
        command: &DrawCommand,
    ) -> Result<(), SvgError> {
        match command {
            DrawCommand::Clear(color) => self.emit_clear(command_index, *color),
            DrawCommand::Save => self.save(None),
            DrawCommand::SaveLayer(options) => {
                if options.bounds().is_some()
                    || options.filter_handle().is_some()
                    || options.blend_mode() != BlendMode::SourceOver
                {
                    return Err(SvgError::new(SvgErrorCode::Unsupported));
                }
                self.save(Some(options.opacity()))
            }
            DrawCommand::Restore => self.restore(),
            DrawCommand::ClipRect { rect, op } => self.clip_rect(*rect, *op),
            DrawCommand::ClipPath { path, rule, op } => self.clip_path(*path, *rule, *op),
            DrawCommand::SetTransform(transform) => {
                self.transform = *transform;
                Ok(())
            }
            DrawCommand::ConcatTransform(transform) => {
                self.transform = self.transform.concat(*transform).map_err(map_skia_error)?;
                Ok(())
            }
            DrawCommand::FillRect { rect, paint } => self.fill_rect(*rect, paint),
            DrawCommand::FillPath { path, rule, paint } => self.fill_path(*path, *rule, paint),
            DrawCommand::StrokePath {
                path,
                options,
                paint,
            } => self.stroke_path(*path, options, paint),
            DrawCommand::DrawImage {
                image,
                destination,
                opacity,
                sampling,
                paint,
            } => self.draw_image(*image, *destination, *opacity, sampling.filter(), paint),
            DrawCommand::DrawGlyphRun { .. } | DrawCommand::DrawPositionedGlyphRun { .. } => {
                Err(SvgError::new(SvgErrorCode::UnsupportedText))
            }
        }
    }

    fn emit_clear(&mut self, command_index: usize, color: Color) -> Result<(), SvgError> {
        if command_index != 0 {
            return Err(SvgError::new(SvgErrorCode::Unsupported));
        }
        if color.is_transparent() {
            return Ok(());
        }
        let view_box = self.spec.view_box;
        let mut element = String::from("<rect");
        push_rect_geometry(&mut element, view_box);
        push_solid_paint(&mut element, "fill", color);
        element.push_str("/>");
        self.parts.body(&element)
    }

    fn save(&mut self, opacity: Option<u8>) -> Result<(), SvgError> {
        let frame = StateFrame {
            transform: self.transform,
            clip_depth: self.clip_depth,
        };
        self.stack.push(frame);
        match opacity {
            Some(value) if value != u8::MAX => self
                .parts
                .body(&format!("<g opacity=\"{}\">", unit_channel(value))),
            _ => self.parts.body("<g>"),
        }
    }

    fn restore(&mut self) -> Result<(), SvgError> {
        let frame = self
            .stack
            .pop()
            .ok_or(SvgError::new(SvgErrorCode::InvalidState))?;
        self.close_clips_to(frame.clip_depth)?;
        self.parts.body("</g>")?;
        self.transform = frame.transform;
        Ok(())
    }

    fn clip_rect(&mut self, rect: Rect, op: ClipOp) -> Result<(), SvgError> {
        if op != ClipOp::Intersect {
            return Err(SvgError::new(SvgErrorCode::Unsupported));
        }
        let id = self.next_clip_resource()?;
        let mut definition =
            format!("<clipPath id=\"clip{id}\" clipPathUnits=\"userSpaceOnUse\"><rect");
        push_rect_geometry(&mut definition, rect);
        push_transform_attribute(&mut definition, self.transform);
        definition.push_str("/></clipPath>");
        self.parts.defs(&definition)?;
        self.open_clip(id)
    }

    fn clip_path(
        &mut self,
        path_id: skia_core::PathId,
        rule: FillRule,
        op: ClipOp,
    ) -> Result<(), SvgError> {
        if op != ClipOp::Intersect {
            return Err(SvgError::new(SvgErrorCode::Unsupported));
        }
        let path = self
            .list
            .path(path_id)
            .ok_or(SvgError::new(SvgErrorCode::InvalidResource))?;
        if path.verbs().is_empty() {
            return self.open_empty_clip();
        }
        let data = path_data(path, self.limits.max_path_verbs)?;
        let id = self.next_clip_resource()?;
        let mut definition = format!(
            "<clipPath id=\"clip{id}\" clipPathUnits=\"userSpaceOnUse\"><path d=\"{data}\""
        );
        if rule == FillRule::EvenOdd {
            definition.push_str(" clip-rule=\"evenodd\"");
        }
        push_transform_attribute(&mut definition, self.transform);
        definition.push_str("/></clipPath>");
        self.parts.defs(&definition)?;
        self.open_clip(id)
    }

    fn open_empty_clip(&mut self) -> Result<(), SvgError> {
        let id = self.next_clip_resource()?;
        self.parts.defs(&format!(
            "<clipPath id=\"clip{id}\" clipPathUnits=\"userSpaceOnUse\"/>"
        ))?;
        self.open_clip(id)
    }

    fn open_clip(&mut self, id: usize) -> Result<(), SvgError> {
        self.parts
            .body(&format!("<g clip-path=\"url(#clip{id})\">"))?;
        self.clip_depth = self
            .clip_depth
            .checked_add(1)
            .ok_or(SvgError::new(SvgErrorCode::NumericOverflow))?;
        Ok(())
    }

    fn close_clips_to(&mut self, target: usize) -> Result<(), SvgError> {
        if target > self.clip_depth {
            return Err(SvgError::new(SvgErrorCode::InvalidState));
        }
        while self.clip_depth > target {
            self.parts.body("</g>")?;
            self.clip_depth -= 1;
        }
        Ok(())
    }

    fn fill_rect(&mut self, rect: Rect, paint: &Paint) -> Result<(), SvgError> {
        let attributes = self.paint_attributes(paint, "fill")?;
        let mut element = String::from("<rect");
        push_rect_geometry(&mut element, rect);
        element.push_str(&attributes);
        push_transform_attribute(&mut element, self.transform);
        element.push_str("/>");
        self.parts.body(&element)
    }

    fn fill_path(
        &mut self,
        path_id: skia_core::PathId,
        rule: FillRule,
        paint: &Paint,
    ) -> Result<(), SvgError> {
        let path = self
            .list
            .path(path_id)
            .ok_or(SvgError::new(SvgErrorCode::InvalidResource))?;
        if path.verbs().is_empty() {
            return Ok(());
        }
        let data = path_data(path, self.limits.max_path_verbs)?;
        let attributes = self.paint_attributes(paint, "fill")?;
        let mut element = format!("<path d=\"{data}\"");
        if rule == FillRule::EvenOdd {
            element.push_str(" fill-rule=\"evenodd\"");
        }
        element.push_str(&attributes);
        push_transform_attribute(&mut element, self.transform);
        element.push_str("/>");
        self.parts.body(&element)
    }

    fn stroke_path(
        &mut self,
        path_id: skia_core::PathId,
        options: &StrokeOptions,
        paint: &Paint,
    ) -> Result<(), SvgError> {
        if options.align() != StrokeAlign::Center || paint.path_effect().is_some() {
            return Err(SvgError::new(SvgErrorCode::Unsupported));
        }
        let path = self
            .list
            .path(path_id)
            .ok_or(SvgError::new(SvgErrorCode::InvalidResource))?;
        if path.verbs().is_empty() {
            return Ok(());
        }
        let data = path_data(path, self.limits.max_path_verbs)?;
        let attributes = self.paint_attributes_without_path_effect(paint, "stroke")?;
        let mut element = format!("<path d=\"{data}\" fill=\"none\"");
        element.push_str(&attributes);
        element.push_str(" stroke-width=\"");
        element.push_str(&svg_scalar(options.width()));
        element.push_str("\" stroke-linecap=\"");
        element.push_str(line_cap(options.cap()));
        element.push_str("\" stroke-linejoin=\"");
        element.push_str(line_join(options.join()));
        element.push_str("\" stroke-miterlimit=\"");
        element.push_str(&svg_scalar(options.miter_limit()));
        element.push('"');
        if !options.dash_pattern().is_empty() {
            element.push_str(" stroke-dasharray=\"");
            for (index, value) in options.dash_pattern().iter().enumerate() {
                if index != 0 {
                    element.push(' ');
                }
                element.push_str(&svg_scalar(*value));
            }
            element.push_str("\" stroke-dashoffset=\"");
            element.push_str(&svg_scalar(options.dash_phase()));
            element.push('"');
        }
        push_transform_attribute(&mut element, self.transform);
        element.push_str("/>");
        self.parts.body(&element)
    }

    fn draw_image(
        &mut self,
        image_id: ImageId,
        destination: Rect,
        opacity: u8,
        sampling: SamplingFilter,
        paint: &Paint,
    ) -> Result<(), SvgError> {
        if paint.blend_mode() != BlendMode::SourceOver
            || paint.shader_handle().is_some()
            || paint.color_filter_handle().is_some()
            || paint.path_effect().is_some()
        {
            return Err(SvgError::new(SvgErrorCode::Unsupported));
        }
        let image = self
            .list
            .image(image_id)
            .cloned()
            .ok_or(SvgError::new(SvgErrorCode::InvalidResource))?;
        if !matches!(image.color_space(), ColorSpace::Srgb) {
            return Err(SvgError::new(SvgErrorCode::UnsupportedColorProfile));
        }
        let resource_id = self.intern_image(image_id, &image)?;
        let mut element = format!("<use href=\"#image{resource_id}\"");
        push_rect_geometry(&mut element, destination);
        let alpha = multiply_alpha(opacity, paint.color().alpha());
        if alpha != u8::MAX {
            element.push_str(" opacity=\"");
            element.push_str(&unit_channel(alpha));
            element.push('"');
        }
        if sampling == SamplingFilter::Nearest {
            element.push_str(" image-rendering=\"pixelated\"");
        }
        push_transform_attribute(&mut element, self.transform);
        element.push_str("/>");
        self.parts.body(&element)
    }

    fn paint_attributes(&mut self, paint: &Paint, property: &str) -> Result<String, SvgError> {
        if paint.path_effect().is_some() {
            return Err(SvgError::new(SvgErrorCode::Unsupported));
        }
        self.paint_attributes_without_path_effect(paint, property)
    }

    fn paint_attributes_without_path_effect(
        &mut self,
        paint: &Paint,
        property: &str,
    ) -> Result<String, SvgError> {
        if paint.blend_mode() != BlendMode::SourceOver || paint.color_filter_handle().is_some() {
            return Err(SvgError::new(SvgErrorCode::Unsupported));
        }
        let mut output = String::new();
        match paint.shader() {
            None => push_solid_paint(&mut output, property, paint.color()),
            Some(shader) => {
                let gradient = shader
                    .gradient()
                    .ok_or(SvgError::new(SvgErrorCode::Unsupported))?;
                let id = self.intern_gradient(gradient, paint.color().alpha())?;
                output.push(' ');
                output.push_str(property);
                output.push_str("=\"url(#gradient");
                output.push_str(&id.to_string());
                output.push_str(")\"");
            }
        }
        Ok(output)
    }

    fn intern_gradient(&mut self, gradient: Gradient, opacity: u8) -> Result<usize, SvgError> {
        let key = (gradient, opacity);
        if let Some(id) = self.gradient_ids.get(&key) {
            return Ok(*id);
        }
        let id = self.next_gradient_id;
        let mut definition = String::new();
        match gradient.geometry() {
            GradientGeometry::Linear { start, end } => {
                definition.push_str("<linearGradient id=\"gradient");
                definition.push_str(&id.to_string());
                definition.push_str("\" gradientUnits=\"userSpaceOnUse\" x1=\"");
                definition.push_str(&svg_scalar(start.x()));
                definition.push_str("\" y1=\"");
                definition.push_str(&svg_scalar(start.y()));
                definition.push_str("\" x2=\"");
                definition.push_str(&svg_scalar(end.x()));
                definition.push_str("\" y2=\"");
                definition.push_str(&svg_scalar(end.y()));
                definition.push('"');
            }
            GradientGeometry::Radial { center, radius } => {
                definition.push_str("<radialGradient id=\"gradient");
                definition.push_str(&id.to_string());
                definition.push_str("\" gradientUnits=\"userSpaceOnUse\" cx=\"");
                definition.push_str(&svg_scalar(center.x()));
                definition.push_str("\" cy=\"");
                definition.push_str(&svg_scalar(center.y()));
                definition.push_str("\" r=\"");
                definition.push_str(&svg_scalar(radius));
                definition.push('"');
            }
        }
        definition.push_str(" spreadMethod=\"");
        definition.push_str(match gradient.tile_mode() {
            TileMode::Clamp => "pad",
            TileMode::Repeat => "repeat",
            TileMode::Mirror => "reflect",
        });
        definition.push_str("\">");
        for stop in gradient.stops() {
            let color = stop.color().with_opacity(opacity);
            definition.push_str("<stop offset=\"");
            definition.push_str(&svg_scalar(stop.offset()));
            definition.push_str("\" stop-color=\"");
            definition.push_str(&svg_color(color));
            definition.push('"');
            if color.alpha() != u8::MAX {
                definition.push_str(" stop-opacity=\"");
                definition.push_str(&unit_channel(color.alpha()));
                definition.push('"');
            }
            definition.push_str("/>");
        }
        definition.push_str(match gradient.geometry() {
            GradientGeometry::Linear { .. } => "</linearGradient>",
            GradientGeometry::Radial { .. } => "</radialGradient>",
        });
        self.reserve_resource()?;
        self.parts.defs(&definition)?;
        self.gradient_ids
            .try_reserve(1)
            .map_err(|_| SvgError::new(SvgErrorCode::AllocationFailed))?;
        self.gradient_ids.insert(key, id);
        self.next_gradient_id = self
            .next_gradient_id
            .checked_add(1)
            .ok_or(SvgError::new(SvgErrorCode::NumericOverflow))?;
        Ok(id)
    }

    fn intern_image(&mut self, image_id: ImageId, image: &Image) -> Result<usize, SvgError> {
        if let Some(id) = self.image_ids.get(&image_id) {
            return Ok(*id);
        }
        let png = encode_png(image, self.limits.max_embedded_image_bytes)?;
        let data = base64_encode(&png)?;
        let id = self.next_image_id;
        let definition = format!(
            "<symbol id=\"image{id}\" viewBox=\"0 0 {} {}\" preserveAspectRatio=\"none\">\
             <image width=\"{}\" height=\"{}\" href=\"data:image/png;base64,{data}\"/>\
             </symbol>",
            image.width(),
            image.height(),
            image.width(),
            image.height()
        );
        self.reserve_resource()?;
        self.parts.defs(&definition)?;
        self.image_ids
            .try_reserve(1)
            .map_err(|_| SvgError::new(SvgErrorCode::AllocationFailed))?;
        self.image_ids.insert(image_id, id);
        self.next_image_id = self
            .next_image_id
            .checked_add(1)
            .ok_or(SvgError::new(SvgErrorCode::NumericOverflow))?;
        Ok(id)
    }

    fn next_clip_resource(&mut self) -> Result<usize, SvgError> {
        self.reserve_resource()?;
        let id = self.next_clip_id;
        self.next_clip_id = self
            .next_clip_id
            .checked_add(1)
            .ok_or(SvgError::new(SvgErrorCode::NumericOverflow))?;
        Ok(id)
    }

    fn reserve_resource(&mut self) -> Result<(), SvgError> {
        if self.resource_count >= self.limits.max_resources {
            return Err(SvgError::new(SvgErrorCode::ResourceLimit));
        }
        self.resource_count += 1;
        Ok(())
    }
}

struct OutputParts {
    defs: String,
    body: String,
    used: usize,
    maximum: usize,
}

impl OutputParts {
    fn new(maximum: usize) -> Self {
        Self {
            defs: String::new(),
            body: String::new(),
            used: 0,
            maximum,
        }
    }

    fn defs(&mut self, value: &str) -> Result<(), SvgError> {
        Self::append(&mut self.defs, &mut self.used, self.maximum, value)
    }

    fn body(&mut self, value: &str) -> Result<(), SvgError> {
        Self::append(&mut self.body, &mut self.used, self.maximum, value)
    }

    fn append(
        destination: &mut String,
        used: &mut usize,
        maximum: usize,
        value: &str,
    ) -> Result<(), SvgError> {
        let next = used
            .checked_add(value.len())
            .ok_or(SvgError::new(SvgErrorCode::NumericOverflow))?;
        if next > maximum {
            return Err(SvgError::new(SvgErrorCode::ResourceLimit));
        }
        destination
            .try_reserve(value.len())
            .map_err(|_| SvgError::new(SvgErrorCode::AllocationFailed))?;
        destination.push_str(value);
        *used = next;
        Ok(())
    }

    fn finish(self, spec: SvgCanvasSpec) -> Result<Vec<u8>, SvgError> {
        let view_box = spec.view_box;
        let header = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
             <svg xmlns=\"{SVG_NAMESPACE}\" width=\"{}\" height=\"{}\" viewBox=\"{} {} {} {}\">",
            svg_scalar(spec.width),
            svg_scalar(spec.height),
            svg_scalar(view_box.left()),
            svg_scalar(view_box.top()),
            svg_rect_width(view_box),
            svg_rect_height(view_box)
        );
        let defs_overhead = if self.defs.is_empty() {
            0
        } else {
            "<defs></defs>".len()
        };
        let total = header
            .len()
            .checked_add(defs_overhead)
            .and_then(|value| value.checked_add(self.used))
            .and_then(|value| value.checked_add("</svg>".len()))
            .ok_or(SvgError::new(SvgErrorCode::NumericOverflow))?;
        if total > self.maximum {
            return Err(SvgError::new(SvgErrorCode::ResourceLimit));
        }
        let mut output = String::new();
        output
            .try_reserve_exact(total)
            .map_err(|_| SvgError::new(SvgErrorCode::AllocationFailed))?;
        output.push_str(&header);
        if !self.defs.is_empty() {
            output.push_str("<defs>");
            output.push_str(&self.defs);
            output.push_str("</defs>");
        }
        output.push_str(&self.body);
        output.push_str("</svg>");
        Ok(output.into_bytes())
    }
}

fn path_data(path: &Path, maximum_verbs: usize) -> Result<String, SvgError> {
    if path.verbs().len() > maximum_verbs {
        return Err(SvgError::new(SvgErrorCode::ResourceLimit));
    }
    let mut output = String::new();
    output
        .try_reserve(path.verbs().len().saturating_mul(20))
        .map_err(|_| SvgError::new(SvgErrorCode::AllocationFailed))?;
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
                return Err(SvgError::new(SvgErrorCode::Unsupported));
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

fn require_current(point: Option<skia_core::Point>) -> Result<skia_core::Point, SvgError> {
    point.ok_or(SvgError::new(SvgErrorCode::InvalidResource))
}

fn push_point(output: &mut String, x: Scalar, y: Scalar) {
    output.push_str(&svg_scalar(x));
    output.push(' ');
    output.push_str(&svg_scalar(y));
}

fn push_rect_geometry(output: &mut String, rect: Rect) {
    output.push_str(" x=\"");
    output.push_str(&svg_scalar(rect.left()));
    output.push_str("\" y=\"");
    output.push_str(&svg_scalar(rect.top()));
    output.push_str("\" width=\"");
    output.push_str(&svg_rect_width(rect));
    output.push_str("\" height=\"");
    output.push_str(&svg_rect_height(rect));
    output.push('"');
}

fn push_transform_attribute(output: &mut String, transform: Transform) {
    if transform == Transform::IDENTITY {
        return;
    }
    output.push_str(" transform=\"matrix(");
    for (index, coefficient) in transform.coefficients().iter().enumerate() {
        if index != 0 {
            output.push(' ');
        }
        output.push_str(&svg_scalar(*coefficient));
    }
    output.push_str(")\"");
}

fn push_solid_paint(output: &mut String, property: &str, color: Color) {
    output.push(' ');
    output.push_str(property);
    output.push_str("=\"");
    output.push_str(&svg_color(color));
    output.push('"');
    if color.alpha() != u8::MAX {
        output.push(' ');
        output.push_str(property);
        output.push_str("-opacity=\"");
        output.push_str(&unit_channel(color.alpha()));
        output.push('"');
    }
}

fn svg_color(color: Color) -> String {
    format!(
        "#{:02X}{:02X}{:02X}",
        color.red(),
        color.green(),
        color.blue()
    )
}

fn svg_scalar(value: Scalar) -> String {
    fixed_bits(i64::from(value.bits()))
}

fn svg_rect_width(rect: Rect) -> String {
    fixed_bits(i64::from(rect.right().bits()) - i64::from(rect.left().bits()))
}

fn svg_rect_height(rect: Rect) -> String {
    fixed_bits(i64::from(rect.bottom().bits()) - i64::from(rect.top().bits()))
}

fn fixed_bits(bits: i64) -> String {
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

fn line_cap(cap: StrokeCap) -> &'static str {
    match cap {
        StrokeCap::Butt => "butt",
        StrokeCap::Round => "round",
        StrokeCap::Square => "square",
    }
}

fn line_join(join: StrokeJoin) -> &'static str {
    match join {
        StrokeJoin::Miter => "miter",
        StrokeJoin::Round => "round",
        StrokeJoin::Bevel => "bevel",
    }
}

fn multiply_alpha(left: u8, right: u8) -> u8 {
    ((u16::from(left) * u16::from(right) + 127) / 255) as u8
}

fn encode_png(image: &Image, maximum: usize) -> Result<Vec<u8>, SvgError> {
    let limits =
        EncodeLimits::new(maximum).map_err(|_| SvgError::new(SvgErrorCode::InvalidLimits))?;
    let options =
        EncodeOptions::new(EncodeFormat::Png(PngOptions::balanced_v1())).with_limits(limits);
    ImageCodec::encode(&ImageAsset::new(image.clone()), &options)
        .map(|encoded| encoded.into_bytes())
        .map_err(|error| match error.code() {
            CodecErrorCode::OutputTooLarge => SvgError::new(SvgErrorCode::ResourceLimit),
            CodecErrorCode::UnsupportedColorProfile => {
                SvgError::new(SvgErrorCode::UnsupportedColorProfile)
            }
            _ => SvgError::new(SvgErrorCode::InvalidResource),
        })
}

fn base64_encode(bytes: &[u8]) -> Result<String, SvgError> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let groups = bytes
        .len()
        .checked_add(2)
        .ok_or(SvgError::new(SvgErrorCode::NumericOverflow))?
        / 3;
    let output_length = groups
        .checked_mul(4)
        .ok_or(SvgError::new(SvgErrorCode::NumericOverflow))?;
    let mut output = String::new();
    output
        .try_reserve_exact(output_length)
        .map_err(|_| SvgError::new(SvgErrorCode::AllocationFailed))?;
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        output.push(char::from(ALPHABET[usize::from(first >> 2)]));
        output.push(char::from(
            ALPHABET[usize::from(((first & 0x03) << 4) | (second >> 4))],
        ));
        if chunk.len() > 1 {
            output.push(char::from(
                ALPHABET[usize::from(((second & 0x0F) << 2) | (third >> 6))],
            ));
        } else {
            output.push('=');
        }
        if chunk.len() > 2 {
            output.push(char::from(ALPHABET[usize::from(third & 0x3F)]));
        } else {
            output.push('=');
        }
    }
    debug_assert_eq!(output.len(), output_length);
    Ok(output)
}

fn map_skia_error(error: skia_core::SkiaError) -> SvgError {
    use skia_core::SkiaErrorCode;
    match error.code() {
        SkiaErrorCode::AllocationFailed => SvgError::new(SvgErrorCode::AllocationFailed),
        SkiaErrorCode::NumericOverflow => SvgError::new(SvgErrorCode::NumericOverflow),
        SkiaErrorCode::ResourceLimit => SvgError::new(SvgErrorCode::ResourceLimit),
        _ => SvgError::new(SvgErrorCode::InvalidResource),
    }
}
