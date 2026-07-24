use crate::{
    FontCollection, FontCollectionLimits, FontId, FontMetrics, FontSlant, FontStyle, FontWidth,
    GlyphBitmap, GlyphId, GlyphOutline, GlyphOutlineProvider, GlyphRun, GlyphRunSource,
    OutlinePoint, OutlineSegment, PositionedGlyph, TextDecorationMetrics, TextDirection, TextError,
    TextErrorCode, TextUnit, Typeface,
};

use super::TypefaceBackend;

#[path = "../../../tools/fonts/test_typeface.rs"]
mod test_data;

const TEST_UNITS_PER_EM: u16 = 2048;

#[derive(Clone, Copy, Debug)]
struct TestTypefaceBackend {
    id: FontId,
    data: &'static test_data::TestFontData,
}

impl TestTypefaceBackend {
    fn new(id: FontId, data: &'static test_data::TestFontData) -> Result<Self, TextError> {
        data.validate()
            .map_err(|_| TextError::new(TextErrorCode::InvalidFontData))?;
        Ok(Self { id, data })
    }

    fn typeface(self) -> Typeface {
        Typeface::from_backend(self)
    }

    fn decoration_metrics(
        self,
        font_size_bits: i32,
        offset: f32,
        thickness: f32,
    ) -> Result<Option<TextDecorationMetrics>, TextError> {
        if font_size_bits <= 0 {
            return Err(TextError::new(TextErrorCode::InvalidFontSize));
        }
        if !offset.is_finite() || !thickness.is_finite() || thickness <= 0.0 {
            return Ok(None);
        }
        Ok(Some(TextDecorationMetrics::from_bits_for_test(
            scaled_em_bits(offset, font_size_bits)?,
            scaled_em_bits(thickness, font_size_bits)?.max(1),
        )))
    }
}

impl TypefaceBackend for TestTypefaceBackend {
    fn id(&self) -> FontId {
        self.id
    }

    fn family_name(&self) -> Option<&str> {
        Some(self.data.toy_family)
    }

    fn matches_family(&self, family: &str) -> bool {
        self.data.toy_family.eq_ignore_ascii_case(family)
            || self.data.generic_family.eq_ignore_ascii_case(family)
    }

    fn style(&self) -> FontStyle {
        data_style(self.data.style).expect("validated upstream style")
    }

    fn glyph_for_character(&self, character: char) -> Result<Option<GlyphId>, TextError> {
        Ok(self
            .data
            .glyph_index(character as u32)
            .and_then(|index| u32::try_from(index).ok())
            .map(GlyphId::new))
    }

    fn rasterize_glyph(
        &self,
        glyph: GlyphId,
        font_size_bits: i32,
    ) -> Result<Option<GlyphBitmap>, TextError> {
        if font_size_bits <= 0 {
            return Err(TextError::new(TextErrorCode::InvalidFontSize));
        }
        if usize::try_from(glyph.value())
            .ok()
            .is_none_or(|index| index >= self.data.char_codes.len())
        {
            return Ok(None);
        }
        Ok(None)
    }

    fn metrics(&self, font_size_bits: i32) -> Result<FontMetrics, TextError> {
        if font_size_bits <= 0 {
            return Err(TextError::new(TextErrorCode::InvalidFontSize));
        }
        Ok(FontMetrics::from_bits(
            scaled_em_bits(-self.data.metrics.ascent, font_size_bits)?.max(0),
            scaled_em_bits(self.data.metrics.descent, font_size_bits)?.max(0),
            scaled_em_bits(self.data.metrics.leading, font_size_bits)?.max(0),
        ))
    }

    fn underline_metrics(
        &self,
        font_size_bits: i32,
    ) -> Result<Option<TextDecorationMetrics>, TextError> {
        self.decoration_metrics(
            font_size_bits,
            self.data.metrics.underline_position,
            self.data.metrics.underline_thickness,
        )
    }

    fn strike_through_metrics(
        &self,
        font_size_bits: i32,
    ) -> Result<Option<TextDecorationMetrics>, TextError> {
        self.decoration_metrics(
            font_size_bits,
            self.data.metrics.strikeout_position,
            self.data.metrics.strikeout_thickness,
        )
    }

    fn shape_segment(
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
        if language.is_some_and(|language| !crate::valid_language_tag(language)) {
            return Err(TextError::new(TextErrorCode::InvalidLanguage));
        }

        let mut characters: Vec<(usize, char)> = text.char_indices().collect();
        if direction == Some(TextDirection::RightToLeft) {
            characters.reverse();
        }
        let mut glyphs = Vec::new();
        glyphs
            .try_reserve_exact(characters.len())
            .map_err(|_| TextError::new(TextErrorCode::AllocationFailed))?;
        let mut pen_x_bits = 0_i32;
        for (relative_cluster, character) in characters {
            let glyph_index = self
                .data
                .glyph_index(character as u32)
                .ok_or(TextError::new(TextErrorCode::MissingGlyph))?;
            let glyph = GlyphId::new(
                u32::try_from(glyph_index)
                    .map_err(|_| TextError::new(TextErrorCode::InvalidFontData))?,
            );
            let advance_bits = fixed_advance_bits(
                self.data
                    .advance_fixed(glyph_index)
                    .ok_or(TextError::new(TextErrorCode::InvalidFontData))?,
            )?;
            let cluster = u32::try_from(relative_cluster)
                .ok()
                .and_then(|cluster| cluster.checked_add(cluster_offset))
                .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
            glyphs.push(PositionedGlyph::with_cluster(
                glyph,
                cluster,
                TextUnit::from_bits(pen_x_bits),
                TextUnit::ZERO,
                TextUnit::from_bits(advance_bits),
                TextUnit::ZERO,
            ));
            pen_x_bits = pen_x_bits
                .checked_add(advance_bits)
                .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
        }
        GlyphRun::new_with_source(
            self.id,
            font_size_bits,
            TEST_UNITS_PER_EM,
            glyphs,
            GlyphRunSource::new(text.to_owned(), cluster_offset)?,
        )
    }

    fn glyph_outline(&self, glyph: GlyphId) -> Result<Option<GlyphOutline>, TextError> {
        let Ok(glyph_index) = usize::try_from(glyph.value()) else {
            return Ok(None);
        };
        let Some(outline) = self.data.outline(glyph_index) else {
            return Ok(None);
        };
        let mut segments = Vec::new();
        segments
            .try_reserve_exact(outline.verbs.len())
            .map_err(|_| TextError::new(TextErrorCode::AllocationFailed))?;
        let mut point_offset = 0_usize;
        for &verb in outline.verbs {
            let mut next_point = || {
                let coordinates = outline
                    .points
                    .get(point_offset..point_offset + 2)
                    .ok_or(TextError::new(TextErrorCode::InvalidFontData))?;
                point_offset += 2;
                Ok(OutlinePoint::new(
                    design_unit(coordinates[0])?,
                    design_unit(coordinates[1])?,
                ))
            };
            segments.push(match verb {
                test_data::MOVE_VERB => OutlineSegment::MoveTo(next_point()?),
                test_data::LINE_VERB => OutlineSegment::LineTo(next_point()?),
                test_data::QUAD_VERB => OutlineSegment::QuadTo {
                    control: next_point()?,
                    end: next_point()?,
                },
                test_data::CUBIC_VERB => OutlineSegment::CubicTo {
                    first_control: next_point()?,
                    second_control: next_point()?,
                    end: next_point()?,
                },
                test_data::CLOSE_VERB => OutlineSegment::Close,
                _ => return Err(TextError::new(TextErrorCode::InvalidFontData)),
            });
        }
        if point_offset != outline.points.len() {
            return Err(TextError::new(TextErrorCode::InvalidFontData));
        }
        GlyphOutline::new(self.id, glyph, segments).map(Some)
    }
}

#[test]
fn upstream_test_typeface_tables_are_self_consistent() {
    for face in test_data::FACES {
        face.validate()
            .unwrap_or_else(|reason| panic!("{}: {reason}", face.source_file));
        assert_eq!(
            test_data::resolve_face(face.generic_family, face.style)
                .expect("generic family")
                .source_file,
            face.source_file
        );
        assert_eq!(
            test_data::resolve_face(face.toy_family, face.style)
                .expect("toy family")
                .source_file,
            face.source_file
        );
        for (glyph_index, &character) in face.char_codes.iter().enumerate() {
            assert_eq!(face.glyph_index(character), Some(glyph_index));
            assert!(face.advance_fixed(glyph_index).is_some());
            assert!(face.resolve_glyph(character).is_some());
        }
    }
}

#[test]
fn test_typefaces_match_through_the_production_collection_api() {
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    for (index, face) in test_data::FACES.iter().enumerate() {
        fonts
            .add_typeface(
                TestTypefaceBackend::new(FontId::new(index as u64 + 1), face)
                    .expect("valid test face")
                    .typeface(),
            )
            .expect("add test typeface");
    }

    assert!(fonts.faces().is_empty());
    assert_eq!(fonts.typefaces().len(), test_data::FACES.len());
    for (index, face) in test_data::FACES.iter().enumerate() {
        let style = data_style(face.style).expect("valid style");
        let expected = FontId::new(index as u64 + 1);
        assert_eq!(
            fonts
                .match_typeface(face.toy_family, style)
                .expect("toy family")
                .id(),
            expected
        );
        assert_eq!(
            fonts
                .match_typeface(face.generic_family, style)
                .expect("generic family")
                .id(),
            expected
        );
    }
}

#[test]
fn test_typeface_shapes_and_resolves_outlines_through_font_collection() {
    let data = test_data::default_face();
    let id = FontId::new(1000);
    let typeface = TestTypefaceBackend::new(id, data)
        .expect("valid test face")
        .typeface();
    let mut fonts = FontCollection::new(FontCollectionLimits::default());
    fonts.add_typeface(typeface).expect("add test typeface");

    let text = "Rust";
    let paragraph = fonts
        .shape_paragraph(text, 16 << 16)
        .expect("shape with test typeface");
    let run = paragraph.runs()[0].glyph_run();
    assert_eq!(run.font(), id);
    assert_eq!(run.glyphs().len(), text.chars().count());
    for (glyph, character) in run.glyphs().iter().zip(text.chars()) {
        assert_eq!(
            usize::try_from(glyph.glyph().value()).expect("glyph index"),
            data.glyph_index(character as u32)
                .expect("covered character")
        );
    }

    let drawable = run
        .glyphs()
        .iter()
        .find(|glyph| {
            data.outline(glyph.glyph().value() as usize)
                .is_some_and(|outline| !outline.verbs.is_empty())
        })
        .expect("drawable glyph");
    let outline = fonts
        .glyph_outline(id, drawable.glyph())
        .expect("outline lookup")
        .expect("test outline");
    assert!(!outline.segments().is_empty());
    assert!(paragraph.metrics().line_height_bits().is_ok());
}

fn data_style(style: test_data::FontStyle) -> Result<FontStyle, TextError> {
    let width = match style.width {
        1 => FontWidth::UltraCondensed,
        2 => FontWidth::ExtraCondensed,
        3 => FontWidth::Condensed,
        4 => FontWidth::SemiCondensed,
        5 => FontWidth::Normal,
        6 => FontWidth::SemiExpanded,
        7 => FontWidth::Expanded,
        8 => FontWidth::ExtraExpanded,
        9 => FontWidth::UltraExpanded,
        _ => return Err(TextError::new(TextErrorCode::InvalidFontData)),
    };
    let slant = match style.slant {
        test_data::Slant::Upright => FontSlant::Normal,
        test_data::Slant::Italic => FontSlant::Italic,
    };
    FontStyle::new(style.weight, width, slant)
}

fn fixed_advance_bits(advance: u32) -> Result<i32, TextError> {
    let bits = u64::from(advance)
        .checked_mul(u64::from(TEST_UNITS_PER_EM))
        .and_then(|value| value.checked_mul(64))
        .and_then(|value| value.checked_add(1 << 15))
        .map(|value| value >> 16)
        .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
    i32::try_from(bits).map_err(|_| TextError::new(TextErrorCode::NumericOverflow))
}

fn design_unit(value: f32) -> Result<TextUnit, TextError> {
    let bits = f64::from(value) * f64::from(TEST_UNITS_PER_EM) * 64.0;
    if !bits.is_finite() || bits < f64::from(i32::MIN) || bits > f64::from(i32::MAX) {
        return Err(TextError::new(TextErrorCode::InvalidFontData));
    }
    Ok(TextUnit::from_bits(bits.round() as i32))
}

fn scaled_em_bits(value: f32, font_size_bits: i32) -> Result<i32, TextError> {
    let bits = f64::from(value) * f64::from(font_size_bits);
    if !bits.is_finite() || bits < f64::from(i32::MIN) || bits > f64::from(i32::MAX) {
        return Err(TextError::new(TextErrorCode::NumericOverflow));
    }
    Ok(bits.round() as i32)
}
