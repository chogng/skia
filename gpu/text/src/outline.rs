use skia_core::{
    GlyphOutlineProvider, Path, Point, Scalar, TextLayout, TextStyleId, Transform,
    glyph_outline_path,
};

use crate::{TextGpuError, TextGpuErrorCode, error::map_text_error};

/// One contiguous batch of target-space glyph outline paths sharing a style.
///
/// Callers resolve [`Self::style_id`] to a paint, register every returned path
/// with [`skia_gpu::GpuCommandEncoder::add_path`], and record ordinary
/// [`skia_gpu::GpuCommandEncoder::fill_path`] commands using
/// [`skia_core::FillRule::NonZero`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextOutlineBatch {
    style_id: TextStyleId,
    paths: Vec<Path>,
}

impl TextOutlineBatch {
    /// Returns the caller-defined paint/style identity for this batch.
    pub const fn style_id(&self) -> TextStyleId {
        self.style_id
    }

    /// Borrows target-space glyph paths in visual drawing order.
    pub fn paths(&self) -> &[Path] {
        &self.paths
    }

    /// Moves target-space glyph paths out of this batch.
    pub fn into_paths(self) -> Vec<Path> {
        self.paths
    }
}

/// Converts a laid-out text block into per-style target-space vector paths.
///
/// Empty or missing outlines are skipped, matching CPU glyph drawing. Layout
/// line offsets, run origins, justification offsets, glyph positions, and the
/// caller's top-left `origin` are all baked into each path. The adapter remains
/// independent from command encoders and hardware backends.
pub fn layout_outline_batches(
    layout: &TextLayout,
    provider: &impl GlyphOutlineProvider,
    origin: Point,
) -> Result<Vec<TextOutlineBatch>, TextGpuError> {
    let mut batches: Vec<TextOutlineBatch> = Vec::new();
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
            let mut paths = Vec::new();
            for (glyph, offset_x) in run.glyphs().iter().zip(shaped.glyph_offsets_x_bits()) {
                let Some(outline) = provider
                    .glyph_outline(run.font(), glyph.glyph())
                    .map_err(map_text_error)?
                else {
                    continue;
                };
                if outline.font() != run.font() || outline.glyph() != glyph.glyph() {
                    return Err(TextGpuError::new(TextGpuErrorCode::InvalidResource));
                }
                let Some(path) = glyph_outline_path(run, *glyph, &outline)? else {
                    continue;
                };
                let x = run_x
                    .checked_add(*offset_x)
                    .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))?;
                let path = path.transformed(Transform::translate(
                    Scalar::from_bits(x),
                    Scalar::from_bits(baseline_y),
                ))?;
                paths
                    .try_reserve(1)
                    .map_err(|_| TextGpuError::new(TextGpuErrorCode::AllocationFailed))?;
                paths.push(path);
            }
            if paths.is_empty() {
                continue;
            }
            if let Some(batch) = batches.last_mut()
                && batch.style_id == shaped.style_id()
            {
                batch
                    .paths
                    .try_reserve(paths.len())
                    .map_err(|_| TextGpuError::new(TextGpuErrorCode::AllocationFailed))?;
                batch.paths.extend(paths);
            } else {
                batches
                    .try_reserve(1)
                    .map_err(|_| TextGpuError::new(TextGpuErrorCode::AllocationFailed))?;
                batches.push(TextOutlineBatch {
                    style_id: shaped.style_id(),
                    paths,
                });
            }
        }
    }
    Ok(batches)
}
