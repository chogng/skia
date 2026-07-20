use skia_core::{GlyphBitmapFormat, GlyphRun, Point, Rect, Scalar, TextLayout};
use skia_gpu::GpuGlyphQuad;

use crate::{TextAtlas, TextGpuError, TextGpuErrorCode};

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
        for line in layout.lines() {
            let Some(paragraph) = line.paragraph() else {
                continue;
            };
            let line_x = origin
                .x()
                .bits()
                .checked_add(line.offset_x_bits())
                .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))?;
            let baseline_y = origin
                .y()
                .bits()
                .checked_add(line.baseline_y_bits())
                .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))?;
            for shaped in paragraph.runs() {
                let run = shaped.glyph_run();
                if shaped.glyph_offsets_x_bits().len() != run.glyphs().len() {
                    return Err(TextGpuError::new(TextGpuErrorCode::InvalidResource));
                }
                let run_x = line_x
                    .checked_add(shaped.origin_x_bits())
                    .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))?;
                for (glyph, offset_x) in run.glyphs().iter().zip(shaped.glyph_offsets_x_bits()) {
                    let Some(entry) =
                        self.glyph_entry(run.font(), glyph.glyph(), run.font_size_bits())
                    else {
                        continue;
                    };
                    let glyph_x = scaled_glyph_coordinate_bits(glyph.x().bits(), run)?;
                    let glyph_y = scaled_glyph_coordinate_bits(glyph.y().bits(), run)?;
                    let bitmap_left = pixel_bits(entry.left())?;
                    let bitmap_top = pixel_bits(entry.top())?;
                    let left = run_x
                        .checked_add(*offset_x)
                        .and_then(|value| value.checked_add(glyph_x))
                        .and_then(|value| value.checked_add(bitmap_left))
                        .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))?;
                    let top = baseline_y
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
            }
        }
        Ok(glyphs)
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
