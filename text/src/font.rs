use std::{fmt, sync::Arc};

use rustybuzz::ttf_parser::{self, OutlineBuilder};
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    FontId, GlyphId, GlyphOutline, GlyphOutlineProvider, GlyphRun, LigatureCaret, OutlinePoint,
    OutlineSegment, PositionedGlyph, TextDirection, TextError, TextErrorCode, TextUnit,
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

/// Four-byte OpenType table, feature, or variation-axis tag.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FontTag([u8; 4]);

impl FontTag {
    /// Creates a tag from its exact four bytes.
    pub const fn new(bytes: [u8; 4]) -> Self {
        Self(bytes)
    }

    /// Returns the exact four tag bytes.
    pub const fn bytes(self) -> [u8; 4] {
        self.0
    }

    const fn parser_tag(self) -> ttf_parser::Tag {
        ttf_parser::Tag::from_bytes(&self.0)
    }
}

/// One axis declared by a variable OpenType font.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FontVariationAxis {
    tag: FontTag,
    min_value_bits: i32,
    default_value_bits: i32,
    max_value_bits: i32,
    hidden: bool,
}

impl FontVariationAxis {
    /// Returns the four-byte axis tag, such as `wght`.
    pub const fn tag(self) -> FontTag {
        self.tag
    }

    /// Returns the minimum axis value in signed Q16.16 units.
    pub const fn min_value_bits(self) -> i32 {
        self.min_value_bits
    }

    /// Returns the default axis value in signed Q16.16 units.
    pub const fn default_value_bits(self) -> i32 {
        self.default_value_bits
    }

    /// Returns the maximum axis value in signed Q16.16 units.
    pub const fn max_value_bits(self) -> i32 {
        self.max_value_bits
    }

    /// Returns whether the font marks this axis as hidden from user interfaces.
    pub const fn hidden(self) -> bool {
        self.hidden
    }
}

/// One requested variable-font coordinate in signed Q16.16 axis units.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FontVariation {
    tag: FontTag,
    value_bits: i32,
}

impl FontVariation {
    /// Creates one coordinate for an OpenType variation axis.
    pub const fn new(tag: FontTag, value_bits: i32) -> Self {
        Self { tag, value_bits }
    }

    /// Returns the target axis tag.
    pub const fn tag(self) -> FontTag {
        self.tag
    }

    /// Returns the requested signed Q16.16 axis value.
    pub const fn value_bits(self) -> i32 {
        self.value_bits
    }
}

/// One global OpenType shaping feature value.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FontFeature {
    tag: FontTag,
    value: u32,
}

impl FontFeature {
    /// Creates one global shaping feature, such as `kern=0` or `liga=1`.
    pub const fn new(tag: FontTag, value: u32) -> Self {
        Self { tag, value }
    }

    /// Returns the four-byte OpenType feature tag.
    pub const fn tag(self) -> FontTag {
        self.tag
    }

    /// Returns the feature value forwarded to the shaping engine.
    pub const fn value(self) -> u32 {
        self.value
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

/// One scaled horizontal text-decoration line in Q16.16 canvas units.
///
/// The offset is measured from the baseline to the line center. Positive
/// offsets move down in canvas coordinates; thickness is always positive.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TextDecorationMetrics {
    offset_bits: i32,
    thickness_bits: i32,
}

impl TextDecorationMetrics {
    /// Returns the signed baseline-to-center offset in canvas coordinates.
    pub const fn offset_bits(self) -> i32 {
        self.offset_bits
    }

    /// Returns the positive line thickness.
    pub const fn thickness_bits(self) -> i32 {
        self.thickness_bits
    }
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
    variation_axes: Vec<FontVariationAxis>,
    variations: Vec<FontVariation>,
    features: Vec<FontFeature>,
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
            .field("variation_axes", &self.variation_axes)
            .field("variations", &self.variations)
            .field("features", &self.features)
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
        let mut variation_axes = Vec::new();
        variation_axes
            .try_reserve_exact(usize::from(face.variation_axes().len()))
            .map_err(|_| TextError::new(TextErrorCode::AllocationFailed))?;
        for axis in face.variation_axes() {
            variation_axes.push(FontVariationAxis {
                tag: FontTag::new(axis.tag.to_bytes()),
                min_value_bits: fixed_axis_bits(axis.min_value)?,
                default_value_bits: fixed_axis_bits(axis.def_value)?,
                max_value_bits: fixed_axis_bits(axis.max_value)?,
                hidden: axis.hidden,
            });
        }
        Ok(Self {
            id,
            bytes: bytes.into(),
            face_index,
            units_per_em,
            glyph_count,
            family_name,
            style,
            variation_axes,
            variations: Vec::new(),
            features: Vec::new(),
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

    /// Borrows axes declared by this face's OpenType `fvar` table.
    pub fn variation_axes(&self) -> &[FontVariationAxis] {
        &self.variation_axes
    }

    /// Borrows non-default coordinates applied to this immutable face instance.
    pub fn variations(&self) -> &[FontVariation] {
        &self.variations
    }

    /// Borrows global OpenType features applied by this immutable face instance.
    pub fn features(&self) -> &[FontFeature] {
        &self.features
    }

    /// Creates an immutable variable-font instance with a new stable identity.
    ///
    /// Every coordinate must name a declared axis, occur at most once, and
    /// remain inside that axis's inclusive range. Unspecified axes keep their
    /// font-defined defaults.
    pub fn instantiate_variations(
        &self,
        id: FontId,
        variations: &[FontVariation],
    ) -> Result<Self, TextError> {
        if id == self.id
            || self.variation_axes.is_empty()
            || variations.len() > self.variation_axes.len()
            || variations.len() > 64
        {
            return Err(TextError::new(TextErrorCode::InvalidFontVariation));
        }
        for (index, variation) in variations.iter().copied().enumerate() {
            if variations[..index]
                .iter()
                .any(|existing| existing.tag == variation.tag)
            {
                return Err(TextError::new(TextErrorCode::InvalidFontVariation));
            }
            let axis = self
                .variation_axes
                .iter()
                .find(|axis| axis.tag == variation.tag)
                .ok_or(TextError::new(TextErrorCode::InvalidFontVariation))?;
            if variation.value_bits < axis.min_value_bits
                || variation.value_bits > axis.max_value_bits
            {
                return Err(TextError::new(TextErrorCode::InvalidFontVariation));
            }
        }
        let mut instance_variations = Vec::new();
        instance_variations
            .try_reserve_exact(variations.len())
            .map_err(|_| TextError::new(TextErrorCode::AllocationFailed))?;
        for axis in &self.variation_axes {
            if let Some(variation) = variations
                .iter()
                .copied()
                .find(|variation| variation.tag == axis.tag)
                && variation.value_bits != axis.default_value_bits
            {
                instance_variations.push(variation);
            }
        }
        let mut instance = self.clone();
        instance.id = id;
        instance.variations = instance_variations;
        Ok(instance)
    }

    /// Creates an immutable shaping-feature instance with a new stable identity.
    ///
    /// Tags must be unique. Unsupported tags remain valid and are ignored by
    /// fonts that do not provide the corresponding OpenType feature.
    pub fn instantiate_features(
        &self,
        id: FontId,
        features: &[FontFeature],
    ) -> Result<Self, TextError> {
        const MAX_FEATURES: usize = 256;
        if id == self.id {
            return Err(TextError::new(TextErrorCode::InvalidFontFeature));
        }
        if features.len() > MAX_FEATURES {
            return Err(TextError::new(TextErrorCode::ResourceLimit));
        }
        for (index, feature) in features.iter().enumerate() {
            if features[..index]
                .iter()
                .any(|existing| existing.tag == feature.tag)
            {
                return Err(TextError::new(TextErrorCode::InvalidFontFeature));
            }
        }
        let mut instance_features = Vec::new();
        instance_features
            .try_reserve_exact(features.len())
            .map_err(|_| TextError::new(TextErrorCode::AllocationFailed))?;
        instance_features.extend_from_slice(features);
        instance_features.sort_unstable_by_key(|feature| feature.tag);
        let mut instance = self.clone();
        instance.id = id;
        instance.features = instance_features;
        Ok(instance)
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
        let face = self.parser_face()?;
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

    /// Returns scaled underline metrics, or `None` when the font has no `post` table.
    pub fn underline_metrics(
        &self,
        font_size_bits: i32,
    ) -> Result<Option<TextDecorationMetrics>, TextError> {
        self.decoration_metrics(font_size_bits, |face| face.underline_metrics())
    }

    /// Returns scaled strike-through metrics, or `None` when the font has no `OS/2` table.
    pub fn strike_through_metrics(
        &self,
        font_size_bits: i32,
    ) -> Result<Option<TextDecorationMetrics>, TextError> {
        self.decoration_metrics(font_size_bits, |face| face.strikeout_metrics())
    }

    fn decoration_metrics(
        &self,
        font_size_bits: i32,
        select: impl FnOnce(&ttf_parser::Face<'_>) -> Option<ttf_parser::LineMetrics>,
    ) -> Result<Option<TextDecorationMetrics>, TextError> {
        if font_size_bits <= 0 {
            return Err(TextError::new(TextErrorCode::InvalidFontSize));
        }
        let face = self.parser_face()?;
        let Some(metrics) = select(&face) else {
            return Ok(None);
        };
        if metrics.thickness <= 0 {
            return Err(TextError::new(TextErrorCode::InvalidFontData));
        }
        Ok(Some(TextDecorationMetrics {
            offset_bits: scale_font_units_bits(
                i64::from(metrics.position)
                    .checked_neg()
                    .ok_or(TextError::new(TextErrorCode::NumericOverflow))?,
                font_size_bits,
                self.units_per_em,
            )?,
            thickness_bits: scale_font_units_bits(
                i64::from(metrics.thickness),
                font_size_bits,
                self.units_per_em,
            )?
            .max(1),
        }))
    }

    /// Shapes one non-empty UTF-8 segment using automatic direction and script detection.
    ///
    /// The resulting clusters are UTF-8 byte offsets. Mixed-direction
    /// paragraphs should use [`crate::FontCollection::shape_paragraph`] instead.
    pub fn shape(&self, text: &str, font_size_bits: i32) -> Result<GlyphRun, TextError> {
        self.shape_segment(text, font_size_bits, None, 0, None)
    }

    /// Shapes one segment with a BCP 47-style language for OpenType `locl`.
    pub fn shape_with_language(
        &self,
        text: &str,
        font_size_bits: i32,
        language: &str,
    ) -> Result<GlyphRun, TextError> {
        self.shape_segment(text, font_size_bits, None, 0, Some(language))
    }

    /// Shapes one horizontal UTF-8 segment with an explicit direction.
    pub fn shape_with_direction(
        &self,
        text: &str,
        font_size_bits: i32,
        direction: TextDirection,
    ) -> Result<GlyphRun, TextError> {
        self.shape_segment(text, font_size_bits, Some(direction), 0, None)
    }

    /// Shapes one segment with explicit direction and shaping language.
    pub fn shape_with_direction_and_language(
        &self,
        text: &str,
        font_size_bits: i32,
        direction: TextDirection,
        language: &str,
    ) -> Result<GlyphRun, TextError> {
        self.shape_segment(text, font_size_bits, Some(direction), 0, Some(language))
    }

    pub(crate) fn shape_segment(
        &self,
        text: &str,
        font_size_bits: i32,
        direction: Option<TextDirection>,
        cluster_offset: u32,
        language: Option<&str>,
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
        if language.is_some_and(|language| !crate::valid_language_tag(language)) {
            return Err(TextError::new(TextErrorCode::InvalidLanguage));
        }

        let mut face = rustybuzz::Face::from_slice(&self.bytes, self.face_index)
            .ok_or(TextError::new(TextErrorCode::InvalidFontData))?;
        for variation in &self.variations {
            face.as_mut()
                .set_variation(
                    variation.tag.parser_tag(),
                    variation.value_bits as f32 / 65_536.0,
                )
                .ok_or(TextError::new(TextErrorCode::InvalidFontVariation))?;
        }
        let mut buffer = rustybuzz::UnicodeBuffer::new();
        buffer.push_str(text);
        buffer.guess_segment_properties();
        if let Some(direction) = direction {
            buffer.set_direction(match direction {
                TextDirection::LeftToRight => rustybuzz::Direction::LeftToRight,
                TextDirection::RightToLeft => rustybuzz::Direction::RightToLeft,
            });
        }
        if let Some(language) = language {
            buffer.set_language(
                language
                    .parse()
                    .map_err(|_| TextError::new(TextErrorCode::InvalidLanguage))?,
            );
        }
        let shaping_direction = match buffer.direction() {
            rustybuzz::Direction::RightToLeft => TextDirection::RightToLeft,
            _ => TextDirection::LeftToRight,
        };
        let mut features = Vec::new();
        features
            .try_reserve_exact(self.features.len())
            .map_err(|_| TextError::new(TextErrorCode::AllocationFailed))?;
        for feature in &self.features {
            features.push(rustybuzz::Feature::new(
                feature.tag.parser_tag(),
                feature.value,
                ..,
            ));
        }
        let output = rustybuzz::shape(&face, &features, buffer);
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

        let ligature_carets = collect_ligature_carets(
            face.as_ref(),
            text,
            cluster_offset,
            shaping_direction,
            &glyphs,
        )?;
        GlyphRun::with_ligature_carets(
            self.id,
            font_size_bits,
            self.units_per_em,
            glyphs,
            ligature_carets,
        )
    }

    fn parser_face(&self) -> Result<ttf_parser::Face<'_>, TextError> {
        let mut face = ttf_parser::Face::parse(&self.bytes, self.face_index)
            .map_err(|_| TextError::new(TextErrorCode::InvalidFontData))?;
        for variation in &self.variations {
            face.set_variation(
                variation.tag.parser_tag(),
                variation.value_bits as f32 / 65_536.0,
            )
            .ok_or(TextError::new(TextErrorCode::InvalidFontVariation))?;
        }
        Ok(face)
    }
}

fn collect_ligature_carets(
    face: &ttf_parser::Face<'_>,
    text: &str,
    cluster_offset: u32,
    direction: TextDirection,
    glyphs: &[PositionedGlyph],
) -> Result<Vec<LigatureCaret>, TextError> {
    let source_end = u32::try_from(text.len())
        .ok()
        .and_then(|length| cluster_offset.checked_add(length))
        .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
    let mut cluster_boundaries = Vec::new();
    cluster_boundaries
        .try_reserve(glyphs.len().saturating_add(1))
        .map_err(|_| TextError::new(TextErrorCode::AllocationFailed))?;
    cluster_boundaries.push(source_end);
    cluster_boundaries.extend(glyphs.iter().map(|glyph| glyph.cluster()));
    cluster_boundaries.sort_unstable();
    cluster_boundaries.dedup();

    let mut carets = Vec::new();
    let mut first = 0_usize;
    while first < glyphs.len() {
        let cluster = glyphs[first].cluster();
        let mut end = first + 1;
        while end < glyphs.len() && glyphs[end].cluster() == cluster {
            end += 1;
        }
        let cluster_index = cluster_boundaries
            .binary_search(&cluster)
            .map_err(|_| TextError::new(TextErrorCode::InvalidLayout))?;
        let cluster_end = *cluster_boundaries
            .get(cluster_index + 1)
            .ok_or(TextError::new(TextErrorCode::InvalidLayout))?;
        let local_start = usize::try_from(
            cluster
                .checked_sub(cluster_offset)
                .ok_or(TextError::new(TextErrorCode::InvalidLayout))?,
        )
        .map_err(|_| TextError::new(TextErrorCode::NumericOverflow))?;
        let local_end = usize::try_from(
            cluster_end
                .checked_sub(cluster_offset)
                .ok_or(TextError::new(TextErrorCode::InvalidLayout))?,
        )
        .map_err(|_| TextError::new(TextErrorCode::NumericOverflow))?;
        if local_start > local_end
            || local_end > text.len()
            || !text.is_char_boundary(local_start)
            || !text.is_char_boundary(local_end)
        {
            return Err(TextError::new(TextErrorCode::InvalidLayout));
        }
        let mut source_boundaries: Vec<u32> = text[local_start..local_end]
            .grapheme_indices(true)
            .skip(1)
            .map(|(relative, _)| {
                u32::try_from(local_start + relative)
                    .ok()
                    .and_then(|offset| cluster_offset.checked_add(offset))
                    .ok_or(TextError::new(TextErrorCode::NumericOverflow))
            })
            .collect::<Result<_, _>>()?;
        if !source_boundaries.is_empty() {
            let mut matching = None;
            for (relative_index, glyph) in glyphs[first..end].iter().enumerate() {
                let glyph_id = u16::try_from(glyph.glyph().value())
                    .map_err(|_| TextError::new(TextErrorCode::InvalidFontData))?;
                let coordinates =
                    gdef_ligature_caret_coordinates(face, glyph_id, source_boundaries.len())?;
                if coordinates.len() == source_boundaries.len() {
                    if matching.is_some() {
                        matching = None;
                        break;
                    }
                    matching = Some((first + relative_index, coordinates));
                }
            }
            if let Some((glyph_index, mut coordinates)) = matching {
                coordinates.sort_unstable();
                coordinates.dedup();
                let advance_bits = glyphs[glyph_index].advance_x().bits();
                let minimum = advance_bits.min(0);
                let maximum = advance_bits.max(0);
                let coordinates_are_internal = coordinates.iter().all(|coordinate| {
                    i32::try_from(i64::from(*coordinate) * 64)
                        .is_ok_and(|coordinate| coordinate > minimum && coordinate < maximum)
                });
                if coordinates.len() == source_boundaries.len() && coordinates_are_internal {
                    if direction == TextDirection::RightToLeft {
                        coordinates.reverse();
                    }
                    carets
                        .try_reserve(source_boundaries.len())
                        .map_err(|_| TextError::new(TextErrorCode::AllocationFailed))?;
                    for (source_offset, coordinate) in source_boundaries.drain(..).zip(coordinates)
                    {
                        carets.push(LigatureCaret {
                            glyph_index,
                            source_offset,
                            x: TextUnit::from_i32(coordinate)?,
                        });
                    }
                }
            }
        }
        first = end;
    }
    Ok(carets)
}

fn gdef_ligature_caret_coordinates(
    face: &ttf_parser::Face<'_>,
    glyph_id: u16,
    expected_count: usize,
) -> Result<Vec<i32>, TextError> {
    let Some(data) = face.raw_face().table(ttf_parser::Tag::from_bytes(b"GDEF")) else {
        return Ok(Vec::new());
    };
    let version = gdef_u32(data, 0).ok_or(TextError::new(TextErrorCode::InvalidFontData))?;
    if !matches!(version, 0x0001_0000 | 0x0001_0002 | 0x0001_0003) {
        return Err(TextError::new(TextErrorCode::InvalidFontData));
    }
    let list_offset =
        usize::from(gdef_u16(data, 8).ok_or(TextError::new(TextErrorCode::InvalidFontData))?);
    if list_offset == 0 {
        return Ok(Vec::new());
    }
    let coverage_offset = usize::from(
        gdef_u16(data, list_offset).ok_or(TextError::new(TextErrorCode::InvalidFontData))?,
    );
    let glyph_count = usize::from(
        gdef_u16(data, list_offset + 2).ok_or(TextError::new(TextErrorCode::InvalidFontData))?,
    );
    let coverage = list_offset
        .checked_add(coverage_offset)
        .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
    let Some(coverage_index) = gdef_coverage_index(data, coverage, glyph_id)? else {
        return Ok(Vec::new());
    };
    if coverage_index >= glyph_count {
        return Err(TextError::new(TextErrorCode::InvalidFontData));
    }
    let offset_position = list_offset
        .checked_add(4)
        .and_then(|offset| offset.checked_add(coverage_index.checked_mul(2)?))
        .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
    let ligature_offset = usize::from(
        gdef_u16(data, offset_position).ok_or(TextError::new(TextErrorCode::InvalidFontData))?,
    );
    let ligature = list_offset
        .checked_add(ligature_offset)
        .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
    let caret_count = usize::from(
        gdef_u16(data, ligature).ok_or(TextError::new(TextErrorCode::InvalidFontData))?,
    );
    if caret_count != expected_count {
        return Ok(Vec::new());
    }
    let mut coordinates = Vec::new();
    coordinates
        .try_reserve_exact(caret_count)
        .map_err(|_| TextError::new(TextErrorCode::AllocationFailed))?;
    for index in 0..caret_count {
        let offset_position = ligature
            .checked_add(2)
            .and_then(|offset| offset.checked_add(index.checked_mul(2)?))
            .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
        let caret_offset = usize::from(
            gdef_u16(data, offset_position)
                .ok_or(TextError::new(TextErrorCode::InvalidFontData))?,
        );
        let caret = ligature
            .checked_add(caret_offset)
            .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
        let format = gdef_u16(data, caret).ok_or(TextError::new(TextErrorCode::InvalidFontData))?;
        match format {
            1 => coordinates.push(i32::from(
                gdef_i16(data, caret + 2).ok_or(TextError::new(TextErrorCode::InvalidFontData))?,
            )),
            3 => {
                let base = i32::from(
                    gdef_i16(data, caret + 2)
                        .ok_or(TextError::new(TextErrorCode::InvalidFontData))?,
                );
                let device_offset = usize::from(
                    gdef_u16(data, caret + 4)
                        .ok_or(TextError::new(TextErrorCode::InvalidFontData))?,
                );
                let variation_delta = if device_offset == 0 {
                    0
                } else {
                    let device = caret
                        .checked_add(device_offset)
                        .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
                    let outer_index = gdef_u16(data, device)
                        .ok_or(TextError::new(TextErrorCode::InvalidFontData))?;
                    let inner_index = gdef_u16(data, device + 2)
                        .ok_or(TextError::new(TextErrorCode::InvalidFontData))?;
                    let delta_format = gdef_u16(data, device + 4)
                        .ok_or(TextError::new(TextErrorCode::InvalidFontData))?;
                    if delta_format == 0x8000 {
                        face.tables()
                            .gdef
                            .and_then(|gdef| {
                                gdef.glyph_variation_delta(
                                    outer_index,
                                    inner_index,
                                    face.variation_coordinates(),
                                )
                            })
                            .map_or(0, |delta| delta.round() as i32)
                    } else {
                        0
                    }
                };
                coordinates.push(
                    base.checked_add(variation_delta)
                        .ok_or(TextError::new(TextErrorCode::NumericOverflow))?,
                );
            }
            2 => return Ok(Vec::new()),
            _ => return Err(TextError::new(TextErrorCode::InvalidFontData)),
        }
    }
    Ok(coordinates)
}

fn gdef_coverage_index(
    data: &[u8],
    offset: usize,
    glyph_id: u16,
) -> Result<Option<usize>, TextError> {
    let format = gdef_u16(data, offset).ok_or(TextError::new(TextErrorCode::InvalidFontData))?;
    let count = usize::from(
        gdef_u16(data, offset + 2).ok_or(TextError::new(TextErrorCode::InvalidFontData))?,
    );
    match format {
        1 => {
            for index in 0..count {
                let position = offset
                    .checked_add(4)
                    .and_then(|value| value.checked_add(index.checked_mul(2)?))
                    .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
                let covered = gdef_u16(data, position)
                    .ok_or(TextError::new(TextErrorCode::InvalidFontData))?;
                if covered == glyph_id {
                    return Ok(Some(index));
                }
            }
        }
        2 => {
            for index in 0..count {
                let position = offset
                    .checked_add(4)
                    .and_then(|value| value.checked_add(index.checked_mul(6)?))
                    .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
                let start = gdef_u16(data, position)
                    .ok_or(TextError::new(TextErrorCode::InvalidFontData))?;
                let end = gdef_u16(data, position + 2)
                    .ok_or(TextError::new(TextErrorCode::InvalidFontData))?;
                let start_index = usize::from(
                    gdef_u16(data, position + 4)
                        .ok_or(TextError::new(TextErrorCode::InvalidFontData))?,
                );
                if glyph_id >= start && glyph_id <= end {
                    return start_index
                        .checked_add(usize::from(glyph_id - start))
                        .map(Some)
                        .ok_or(TextError::new(TextErrorCode::NumericOverflow));
                }
            }
        }
        _ => return Err(TextError::new(TextErrorCode::InvalidFontData)),
    }
    Ok(None)
}

fn gdef_u16(data: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_be_bytes(
        data.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn gdef_i16(data: &[u8], offset: usize) -> Option<i16> {
    Some(i16::from_be_bytes(
        data.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn gdef_u32(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        data.get(offset..offset + 4)?.try_into().ok()?,
    ))
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
        let face = self.parser_face()?;
        let mut builder = PortableOutlineBuilder::new(self.limits.max_outline_segments);
        let outlined = face.outline_glyph(ttf_parser::GlyphId(glyph_index), &mut builder);
        if outlined.is_none() {
            return Ok(None);
        }
        let segments = builder.finish()?;
        GlyphOutline::new(self.id, glyph, segments).map(Some)
    }
}

fn fixed_axis_bits(value: f32) -> Result<i32, TextError> {
    let bits = f64::from(value) * 65_536.0;
    if !bits.is_finite() || bits < f64::from(i32::MIN) || bits > f64::from(i32::MAX) {
        return Err(TextError::new(TextErrorCode::InvalidFontData));
    }
    Ok(bits.round() as i32)
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
        -(numerator
            .checked_neg()
            .ok_or(TextError::new(TextErrorCode::NumericOverflow))?
            .checked_add(denominator / 2)
            .ok_or(TextError::new(TextErrorCode::NumericOverflow))?
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
