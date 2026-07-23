use skia_core::{
    GlyphBitmapFormat, GlyphRun, Point, Rect, Scalar, TextLayout, TextLayoutEvent, TextStyleId,
    text_layout_glyph_events,
};
use skia_gpu::GpuGlyphQuad;

use crate::{TextAtlas, TextGpuError, TextGpuErrorCode, error::map_skia_error};

/// One contiguous GPU glyph batch sharing a caller-defined text style.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextGlyphBatch {
    style_id: TextStyleId,
    glyphs: Vec<GpuGlyphQuad>,
}

impl TextGlyphBatch {
    /// Returns the style identity to resolve into a paint at submission time.
    pub const fn style_id(&self) -> TextStyleId {
        self.style_id
    }

    /// Borrows positioned glyph quads in visual drawing order.
    pub fn glyphs(&self) -> &[GpuGlyphQuad] {
        &self.glyphs
    }

    /// Moves the positioned glyph quads out of this batch.
    pub fn into_glyphs(self) -> Vec<GpuGlyphQuad> {
        self.glyphs
    }
}

impl TextAtlas {
    /// Converts one text layout into positioned quads referencing this atlas.
    ///
    /// The result is pure command data. Callers explicitly register
    /// [`Self::into_gpu_atlas`] with their encoder and submit these quads using
    /// [`skia_gpu::GpuCommandEncoder::draw_glyph_batch`]. Empty glyphs and
    /// glyphs absent from this atlas are skipped.
    pub fn layout_quads(
        &self,
        layout: &TextLayout,
        origin: Point,
    ) -> Result<Vec<GpuGlyphQuad>, TextGpuError> {
        let mut glyphs = Vec::new();
        for batch in self.layout_style_batches(layout, origin)? {
            glyphs
                .try_reserve(batch.glyphs.len())
                .map_err(|_| TextGpuError::new(TextGpuErrorCode::AllocationFailed))?;
            glyphs.extend(batch.glyphs);
        }
        Ok(glyphs)
    }

    /// Converts a text layout into contiguous per-style GPU glyph batches.
    ///
    /// Adjacent runs with the same [`TextStyleId`] are coalesced. Callers
    /// retain explicit paint resolution and encoder submission order.
    pub fn layout_style_batches(
        &self,
        layout: &TextLayout,
        origin: Point,
    ) -> Result<Vec<TextGlyphBatch>, TextGpuError> {
        let mut batches: Vec<TextGlyphBatch> = Vec::new();
        for event in text_layout_glyph_events(layout, origin).map_err(map_skia_error)? {
            let TextLayoutEvent::GlyphRun {
                style_id,
                run,
                origin,
                offsets_x_bits,
            } = event
            else {
                continue;
            };
            let mut glyphs = Vec::new();
            for (glyph, offset_x) in run.glyphs().iter().zip(offsets_x_bits) {
                let Some(entry) = self.glyph_entry(run.font(), glyph.glyph(), run.font_size_bits())
                else {
                    continue;
                };
                let glyph_x = scaled_glyph_coordinate_bits(glyph.x().bits(), run)?;
                let glyph_y = scaled_glyph_coordinate_bits(glyph.y().bits(), run)?;
                let bitmap_left = pixel_bits(entry.left())?;
                let bitmap_top = pixel_bits(entry.top())?;
                let left = origin
                    .x()
                    .bits()
                    .checked_add(*offset_x)
                    .and_then(|value| value.checked_add(glyph_x))
                    .and_then(|value| value.checked_add(bitmap_left))
                    .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))?;
                let top = origin
                    .y()
                    .bits()
                    .checked_add(glyph_y)
                    .and_then(|value| value.checked_sub(bitmap_top))
                    .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))?;
                let right = left
                    .checked_add(pixel_bits_u32(entry.source().width())?)
                    .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))?;
                let bottom = top
                    .checked_add(pixel_bits_u32(entry.source().height())?)
                    .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))?;
                let destination = Rect::new(
                    Scalar::from_bits(left),
                    Scalar::from_bits(top),
                    Scalar::from_bits(right),
                    Scalar::from_bits(bottom),
                )
                .map_err(|_| TextGpuError::new(TextGpuErrorCode::NumericOverflow))?;
                glyphs
                    .try_reserve(1)
                    .map_err(|_| TextGpuError::new(TextGpuErrorCode::AllocationFailed))?;
                glyphs.push(GpuGlyphQuad::new(
                    entry.source(),
                    destination,
                    entry.key().format() == GlyphBitmapFormat::Alpha8,
                ));
            }
            if glyphs.is_empty() {
                continue;
            }
            if let Some(batch) = batches.last_mut()
                && batch.style_id == style_id
            {
                batch
                    .glyphs
                    .try_reserve(glyphs.len())
                    .map_err(|_| TextGpuError::new(TextGpuErrorCode::AllocationFailed))?;
                batch.glyphs.extend(glyphs);
            } else {
                batches
                    .try_reserve(1)
                    .map_err(|_| TextGpuError::new(TextGpuErrorCode::AllocationFailed))?;
                batches.push(TextGlyphBatch { style_id, glyphs });
            }
        }
        Ok(batches)
    }
}

fn scaled_glyph_coordinate_bits(design_bits: i32, run: &GlyphRun) -> Result<i32, TextGpuError> {
    let numerator = i128::from(design_bits)
        .checked_mul(i128::from(run.font_size_bits()))
        .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))?;
    let denominator = i128::from(64_i32)
        .checked_mul(i128::from(run.units_per_em()))
        .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))?;
    let rounded = if numerator >= 0 {
        numerator
            .checked_add(denominator / 2)
            .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))?
            / denominator
    } else {
        -((-numerator
            .checked_add(denominator / 2)
            .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))?)
            / denominator)
    };
    i32::try_from(rounded).map_err(|_| TextGpuError::new(TextGpuErrorCode::NumericOverflow))
}

fn pixel_bits(value: i32) -> Result<i32, TextGpuError> {
    value
        .checked_mul(1 << 16)
        .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))
}

fn pixel_bits_u32(value: u32) -> Result<i32, TextGpuError> {
    i32::try_from(value)
        .map_err(|_| TextGpuError::new(TextGpuErrorCode::NumericOverflow))
        .and_then(pixel_bits)
}
