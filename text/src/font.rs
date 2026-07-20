use std::{fmt, sync::Arc};

use rustybuzz::ttf_parser::{self, OutlineBuilder};

use crate::{
    FontId, GlyphId, GlyphOutline, GlyphOutlineProvider, GlyphRun, OutlinePoint, OutlineSegment,
    PositionedGlyph, TextDirection, TextError, TextErrorCode, TextUnit,
};

/// Standard OpenType width class used during family matching.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum FontWidth {
    /// Width class 1.
    UltraCondensed,
    /// Width class 2.
    ExtraCondensed,
    /// Width class 3.
    Condensed,
    /// Width class 4.
    SemiCondensed,
    /// Width class 5.
    #[default]
    Normal,
    /// Width class 6.
    SemiExpanded,
    /// Width class 7.
    Expanded,
    /// Width class 8.
    ExtraExpanded,
    /// Width class 9.
    UltraExpanded,
}

impl FontWidth {
    /// Returns the OpenType width class in the inclusive range 1 through 9.
    pub const fn class(self) -> u16 {
        match self {
            Self::UltraCondensed => 1,
            Self::ExtraCondensed => 2,
            Self::Condensed => 3,
            Self::SemiCondensed => 4,
            Self::Normal => 5,
            Self::SemiExpanded => 6,
            Self::Expanded => 7,
            Self::ExtraExpanded => 8,
            Self::UltraExpanded => 9,
        }
    }
}

/// Upright or sloped face classification used during family matching.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum FontSlant {
    /// An upright face.
    #[default]
    Normal,
    /// A cursive italic face.
    Italic,
    /// A mechanically sloped oblique face.
    Oblique,
}

/// CSS-compatible weight, width, and slant request for one font face.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FontStyle {
    weight: u16,
    width: FontWidth,
    slant: FontSlant,
}

impl FontStyle {
    /// Regular weight, normal width, and upright slant.
    pub const NORMAL: Self = Self {
        weight: 400,
        width: FontWidth::Normal,
        slant: FontSlant::Normal,
    };

    /// Creates a style with a CSS weight in the inclusive range 1 through 1000.
    pub const fn new(weight: u16, width: FontWidth, slant: FontSlant) -> Result<Self, TextError> {
        if weight == 0 || weight > 1000 {
            return Err(TextError::new(TextErrorCode::InvalidFontStyle));
        }
        Ok(Self {
            weight,
            width,
            slant,
        })
    }

    /// Returns the CSS/OpenType weight value.
    pub const fn weight(self) -> u16 {
        self.weight
    }

    /// Returns the requested width class.
    pub const fn width(self) -> FontWidth {
        self.width
    }

    /// Returns the requested slant.
    pub const fn slant(self) -> FontSlant {
        self.slant
    }
}

impl Default for FontStyle {
    fn default() -> Self {
        Self::NORMAL
    }
}

/// Resource ceilings applied while loading and using one font face.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FontLimits {
    max_font_bytes: usize,
    max_text_bytes: usize,
    max_glyphs_per_run: usize,
    max_outline_segments: usize,
}

/// Scaled horizontal-font metrics in Q16.16 canvas units.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FontMetrics {
    ascent_bits: i32,
    descent_bits: i32,
    line_gap_bits: i32,
}

impl FontMetrics {
    pub(crate) const fn from_bits(ascent_bits: i32, descent_bits: i32, line_gap_bits: i32) -> Self {
        Self {
            ascent_bits,
            descent_bits,
            line_gap_bits,
        }
    }

    /// Returns the non-negative distance above the baseline.
    pub const fn ascent_bits(self) -> i32 {
        self.ascent_bits
    }

    /// Returns the non-negative distance below the baseline.
    pub const fn descent_bits(self) -> i32 {
        self.descent_bits
    }

    /// Returns the non-negative recommended inter-line gap.
    pub const fn line_gap_bits(self) -> i32 {
        self.line_gap_bits
    }

    /// Returns ascent plus descent plus line gap.
    pub fn line_height_bits(self) -> Result<i32, TextError> {
        self.ascent_bits
            .checked_add(self.descent_bits)
            .and_then(|value| value.checked_add(self.line_gap_bits))
            .ok_or(TextError::new(TextErrorCode::NumericOverflow))
    }
}

impl FontLimits {
    /// Creates positive resource ceilings for font parsing and shaping.
    pub const fn new(
        max_font_bytes: usize,
        max_text_bytes: usize,
        max_glyphs_per_run: usize,
        max_outline_segments: usize,
    ) -> Result<Self, TextError> {
        if max_font_bytes == 0
            || max_text_bytes == 0
            || max_glyphs_per_run == 0
            || max_outline_segments == 0
        {
            return Err(TextError::new(TextErrorCode::InvalidLimits));
        }
        Ok(Self {
            max_font_bytes,
            max_text_bytes,
            max_glyphs_per_run,
            max_outline_segments,
        })
    }

    /// Returns the maximum accepted encoded font size.
    pub const fn max_font_bytes(self) -> usize {
        self.max_font_bytes
    }

    /// Returns the maximum accepted UTF-8 input size for one shaping call.
    pub const fn max_text_bytes(self) -> usize {
        self.max_text_bytes
    }

    /// Returns the maximum shaped glyph count for one run.
    pub const fn max_glyphs_per_run(self) -> usize {
        self.max_glyphs_per_run
    }

    /// Returns the maximum outline operation count for one glyph.
    pub const fn max_outline_segments(self) -> usize {
        self.max_outline_segments
    }
}

impl Default for FontLimits {
    fn default() -> Self {
        Self {
            max_font_bytes: 64 * 1024 * 1024,
            max_text_bytes: 4 * 1024 * 1024,
            max_glyphs_per_run: 1_000_000,
            max_outline_segments: 1_000_000,
        }
    }
}

/// Owned OpenType/TrueType face that shapes UTF-8 and resolves vector outlines.
///
/// The face owns immutable font bytes, so shaped runs and display lists never
/// contain platform font handles. It supports standalone fonts and indexed
/// faces in TrueType/OpenType collections.
#[derive(Clone)]
pub struct FontFace {
    id: FontId,
    bytes: Arc<[u8]>,
    face_index: u32,
    units_per_em: u16,
    glyph_count: u16,
    family_name: Option<String>,
    style: FontStyle,
    limits: FontLimits,
}

impl fmt::Debug for FontFace {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FontFace")
            .field("id", &self.id)
            .field("byte_len", &self.bytes.len())
            .field("face_index", &self.face_index)
            .field("units_per_em", &self.units_per_em)
            .field("glyph_count", &self.glyph_count)
            .field("family_name", &self.family_name)
            .field("style", &self.style)
            .field("limits", &self.limits)
            .finish()
    }
}

impl FontFace {
    /// Loads face zero with default resource ceilings.
    pub fn from_bytes(id: FontId, bytes: Vec<u8>) -> Result<Self, TextError> {
        Self::from_bytes_with_limits(id, bytes, 0, FontLimits::default())
    }

    /// Loads one indexed face with explicit resource ceilings.
    pub fn from_bytes_with_limits(
        id: FontId,
        bytes: Vec<u8>,
        face_index: u32,
        limits: FontLimits,
    ) -> Result<Self, TextError> {
        if bytes.len() > limits.max_font_bytes {
            return Err(TextError::new(TextErrorCode::ResourceLimit));
        }
        let face = ttf_parser::Face::parse(&bytes, face_index).map_err(|error| {
            let code = if error == ttf_parser::FaceParsingError::FaceIndexOutOfBounds {
                TextErrorCode::InvalidFaceIndex
            } else {
                TextErrorCode::InvalidFontData
            };
            TextError::new(code)
        })?;
        let units_per_em = face.units_per_em();
        if units_per_em == 0 {
            return Err(TextError::new(TextErrorCode::InvalidUnitsPerEm));
        }
        let glyph_count = face.number_of_glyphs();
        let family_name = preferred_family_name(&face);
        let weight = face.weight().to_number();
        let style = FontStyle::new(
            weight,
            font_width(face.width()),
            if face.is_oblique() {
                FontSlant::Oblique
            } else if face.is_italic() {
                FontSlant::Italic
            } else {
                FontSlant::Normal
            },
        )
        .map_err(|_| TextError::new(TextErrorCode::InvalidFontData))?;
        Ok(Self {
            id,
            bytes: bytes.into(),
            face_index,
            units_per_em,
            glyph_count,
            family_name,
            style,
            limits,
        })
    }

    /// Returns the stable application-defined identity of this face.
    pub const fn id(&self) -> FontId {
        self.id
    }

    /// Returns the face index within its source font collection.
    pub const fn face_index(&self) -> u32 {
        self.face_index
    }

    /// Returns the face design-unit scale.
    pub const fn units_per_em(&self) -> u16 {
        self.units_per_em
    }

    /// Returns the number of glyphs addressable by this face.
    pub const fn glyph_count(&self) -> u16 {
        self.glyph_count
    }

    /// Returns the preferred typographic or legacy family name, when present.
    pub fn family_name(&self) -> Option<&str> {
        self.family_name.as_deref()
    }

    /// Returns the face's parsed weight, width, and slant.
    pub const fn style(&self) -> FontStyle {
        self.style
    }

    /// Resolves one Unicode scalar to its nominal font-local glyph.
    pub fn glyph_for_character(&self, character: char) -> Result<Option<GlyphId>, TextError> {
        let face = ttf_parser::Face::parse(&self.bytes, self.face_index)
            .map_err(|_| TextError::new(TextErrorCode::InvalidFontData))?;
        Ok(face
            .glyph_index(character)
            .map(|glyph| GlyphId::new(u32::from(glyph.0))))
    }

    /// Returns whether this face nominally covers one Unicode scalar.
    pub fn supports_character(&self, character: char) -> Result<bool, TextError> {
        self.glyph_for_character(character)
            .map(|glyph| glyph.is_some())
    }

    /// Returns baseline metrics scaled to one positive Q16.16 font size.
    pub fn metrics(&self, font_size_bits: i32) -> Result<FontMetrics, TextError> {
        if font_size_bits <= 0 {
            return Err(TextError::new(TextErrorCode::InvalidFontSize));
        }
        let face = ttf_parser::Face::parse(&self.bytes, self.face_index)
            .map_err(|_| TextError::new(TextErrorCode::InvalidFontData))?;
        let ascent_bits = scale_font_units_bits(
            i64::from(face.ascender()),
            font_size_bits,
            self.units_per_em,
        )?
        .max(0);
        let descent_bits = scale_font_units_bits(
            i64::from(face.descender())
                .checked_neg()
                .ok_or(TextError::new(TextErrorCode::NumericOverflow))?,
            font_size_bits,
            self.units_per_em,
        )?
        .max(0);
        let line_gap_bits = scale_font_units_bits(
            i64::from(face.line_gap()),
            font_size_bits,
            self.units_per_em,
        )?
        .max(0);
        Ok(FontMetrics {
            ascent_bits,
            descent_bits,
            line_gap_bits,
        })
    }

    /// Shapes one non-empty UTF-8 segment using automatic direction and script detection.
    ///
    /// The resulting clusters are UTF-8 byte offsets. Mixed-direction
    /// paragraphs should use [`crate::FontCollection::shape_paragraph`] instead.
    pub fn shape(&self, text: &str, font_size_bits: i32) -> Result<GlyphRun, TextError> {
        self.shape_segment(text, font_size_bits, None, 0)
    }

    /// Shapes one horizontal UTF-8 segment with an explicit direction.
    pub fn shape_with_direction(
        &self,
        text: &str,
        font_size_bits: i32,
        direction: TextDirection,
    ) -> Result<GlyphRun, TextError> {
        self.shape_segment(text, font_size_bits, Some(direction), 0)
    }

    pub(crate) fn shape_segment(
        &self,
        text: &str,
        font_size_bits: i32,
        direction: Option<TextDirection>,
        cluster_offset: u32,
    ) -> Result<GlyphRun, TextError> {
        if font_size_bits <= 0 {
            return Err(TextError::new(TextErrorCode::InvalidFontSize));
        }
        if text.is_empty() {
            return Err(TextError::new(TextErrorCode::EmptyText));
        }
        if text.len() > self.limits.max_text_bytes {
            return Err(TextError::new(TextErrorCode::ResourceLimit));
        }

        let face = rustybuzz::Face::from_slice(&self.bytes, self.face_index)
            .ok_or(TextError::new(TextErrorCode::InvalidFontData))?;
        let mut buffer = rustybuzz::UnicodeBuffer::new();
        buffer.push_str(text);
        buffer.guess_segment_properties();
        if let Some(direction) = direction {
            buffer.set_direction(match direction {
                TextDirection::LeftToRight => rustybuzz::Direction::LeftToRight,
                TextDirection::RightToLeft => rustybuzz::Direction::RightToLeft,
            });
        }
        let output = rustybuzz::shape(&face, &[], buffer);
        if output.is_empty() {
            return Err(TextError::new(TextErrorCode::EmptyGlyphRun));
        }
        if output.len() > self.limits.max_glyphs_per_run {
            return Err(TextError::new(TextErrorCode::ResourceLimit));
        }

        let mut glyphs = Vec::new();
        glyphs
            .try_reserve_exact(output.len())
            .map_err(|_| TextError::new(TextErrorCode::AllocationFailed))?;
        let mut pen_x = 0_i64;
        let mut pen_y = 0_i64;
        for (info, position) in output.glyph_infos().iter().zip(output.glyph_positions()) {
            let x = pen_x
                .checked_add(i64::from(position.x_offset))
                .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
            let y = pen_y
                .checked_sub(i64::from(position.y_offset))
                .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
            let advance_x = i64::from(position.x_advance);
            let advance_y = i64::from(position.y_advance)
                .checked_neg()
                .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
            glyphs.push(PositionedGlyph::with_cluster(
                GlyphId::new(info.glyph_id),
                info.cluster
                    .checked_add(cluster_offset)
                    .ok_or(TextError::new(TextErrorCode::NumericOverflow))?,
                font_units(x)?,
                font_units(y)?,
                font_units(advance_x)?,
                font_units(advance_y)?,
            ));
            pen_x = pen_x
                .checked_add(i64::from(position.x_advance))
                .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
            pen_y = pen_y
                .checked_sub(i64::from(position.y_advance))
                .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
        }

        GlyphRun::new(self.id, font_size_bits, self.units_per_em, glyphs)
    }
}

fn preferred_family_name(face: &ttf_parser::Face<'_>) -> Option<String> {
    preferred_name(face, ttf_parser::name_id::TYPOGRAPHIC_FAMILY)
        .or_else(|| preferred_name(face, ttf_parser::name_id::FAMILY))
}

fn preferred_name(face: &ttf_parser::Face<'_>, name_id: u16) -> Option<String> {
    let mut first = None;
    for name in face.names() {
        if name.name_id != name_id {
            continue;
        }
        let Some(value) = name.to_string().filter(|value| !value.is_empty()) else {
            continue;
        };
        if name.language_id == 0x0409 {
            return Some(value);
        }
        if first.is_none() {
            first = Some(value);
        }
    }
    first
}

const fn font_width(width: ttf_parser::Width) -> FontWidth {
    match width {
        ttf_parser::Width::UltraCondensed => FontWidth::UltraCondensed,
        ttf_parser::Width::ExtraCondensed => FontWidth::ExtraCondensed,
        ttf_parser::Width::Condensed => FontWidth::Condensed,
        ttf_parser::Width::SemiCondensed => FontWidth::SemiCondensed,
        ttf_parser::Width::Normal => FontWidth::Normal,
        ttf_parser::Width::SemiExpanded => FontWidth::SemiExpanded,
        ttf_parser::Width::Expanded => FontWidth::Expanded,
        ttf_parser::Width::ExtraExpanded => FontWidth::ExtraExpanded,
        ttf_parser::Width::UltraExpanded => FontWidth::UltraExpanded,
    }
}

impl GlyphOutlineProvider for FontFace {
    fn glyph_outline(
        &self,
        font: FontId,
        glyph: GlyphId,
    ) -> Result<Option<GlyphOutline>, TextError> {
        if font != self.id {
            return Ok(None);
        }
        let Ok(glyph_index) = u16::try_from(glyph.value()) else {
            return Ok(None);
        };
        if glyph_index >= self.glyph_count {
            return Ok(None);
        }
        let face = ttf_parser::Face::parse(&self.bytes, self.face_index)
            .map_err(|_| TextError::new(TextErrorCode::InvalidFontData))?;
        let mut builder = PortableOutlineBuilder::new(self.limits.max_outline_segments);
        let outlined = face.outline_glyph(ttf_parser::GlyphId(glyph_index), &mut builder);
        if outlined.is_none() {
            return Ok(None);
        }
        let segments = builder.finish()?;
        GlyphOutline::new(self.id, glyph, segments).map(Some)
    }
}

fn font_units(value: i64) -> Result<TextUnit, TextError> {
    value
        .checked_mul(64)
        .and_then(|bits| i32::try_from(bits).ok())
        .map(TextUnit::from_bits)
        .ok_or(TextError::new(TextErrorCode::NumericOverflow))
}

pub(crate) fn scale_font_units_bits(
    design_units: i64,
    font_size_bits: i32,
    units_per_em: u16,
) -> Result<i32, TextError> {
    let numerator = i128::from(design_units)
        .checked_mul(i128::from(font_size_bits))
        .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
    let denominator = i128::from(units_per_em);
    let rounded = if numerator >= 0 {
        numerator
            .checked_add(denominator / 2)
            .ok_or(TextError::new(TextErrorCode::NumericOverflow))?
            / denominator
    } else {
        -((-numerator
            .checked_add(denominator / 2)
            .ok_or(TextError::new(TextErrorCode::NumericOverflow))?)
            / denominator)
    };
    i32::try_from(rounded).map_err(|_| TextError::new(TextErrorCode::NumericOverflow))
}

struct PortableOutlineBuilder {
    segments: Vec<OutlineSegment>,
    max_segments: usize,
    error: Option<TextError>,
}

impl PortableOutlineBuilder {
    fn new(max_segments: usize) -> Self {
        Self {
            segments: Vec::new(),
            max_segments,
            error: None,
        }
    }

    fn push(&mut self, segment: OutlineSegment) {
        if self.error.is_some() {
            return;
        }
        if self.segments.len() == self.max_segments {
            self.error = Some(TextError::new(TextErrorCode::ResourceLimit));
            return;
        }
        if self.segments.try_reserve(1).is_err() {
            self.error = Some(TextError::new(TextErrorCode::AllocationFailed));
            return;
        }
        self.segments.push(segment);
    }

    fn point(&mut self, x: f32, y: f32) -> Option<OutlinePoint> {
        let x = outline_unit(x);
        let y = outline_unit(-y);
        match (x, y) {
            (Ok(x), Ok(y)) => Some(OutlinePoint::new(x, y)),
            (Err(error), _) | (_, Err(error)) => {
                self.error = Some(error);
                None
            }
        }
    }

    fn finish(self) -> Result<Vec<OutlineSegment>, TextError> {
        self.error.map_or(Ok(self.segments), Err)
    }
}

impl OutlineBuilder for PortableOutlineBuilder {
    fn move_to(&mut self, x: f32, y: f32) {
        if let Some(point) = self.point(x, y) {
            self.push(OutlineSegment::MoveTo(point));
        }
    }

    fn line_to(&mut self, x: f32, y: f32) {
        if let Some(point) = self.point(x, y) {
            self.push(OutlineSegment::LineTo(point));
        }
    }

    fn quad_to(&mut self, control_x: f32, control_y: f32, x: f32, y: f32) {
        let control = self.point(control_x, control_y);
        let end = self.point(x, y);
        if let (Some(control), Some(end)) = (control, end) {
            self.push(OutlineSegment::QuadTo { control, end });
        }
    }

    fn curve_to(
        &mut self,
        first_x: f32,
        first_y: f32,
        second_x: f32,
        second_y: f32,
        x: f32,
        y: f32,
    ) {
        let first_control = self.point(first_x, first_y);
        let second_control = self.point(second_x, second_y);
        let end = self.point(x, y);
        if let (Some(first_control), Some(second_control), Some(end)) =
            (first_control, second_control, end)
        {
            self.push(OutlineSegment::CubicTo {
                first_control,
                second_control,
                end,
            });
        }
    }

    fn close(&mut self) {
        self.push(OutlineSegment::Close);
    }
}

fn outline_unit(value: f32) -> Result<TextUnit, TextError> {
    let scaled = f64::from(value) * 64.0;
    if !scaled.is_finite() || scaled < f64::from(i32::MIN) || scaled > f64::from(i32::MAX) {
        return Err(TextError::new(TextErrorCode::NumericOverflow));
    }
    Ok(TextUnit::from_bits(scaled.round() as i32))
}
