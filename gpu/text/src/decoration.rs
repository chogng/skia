use skia_core::{Point, Rect, Scalar, TextDecorationMetrics, TextLayout, TextStyleId};

use crate::{TextGpuError, TextGpuErrorCode};

/// One contiguous GPU text-decoration batch sharing a caller-defined style.
///
/// The rectangles are ordinary target-space geometry. Callers resolve
/// [`Self::style_id`] into a paint and record each rectangle with
/// [`skia_gpu::GpuCommandEncoder::fill_rect`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextDecorationBatch {
    style_id: TextStyleId,
    rects: Vec<Rect>,
}

impl TextDecorationBatch {
    /// Returns the style identity to resolve into a paint at submission time.
    pub const fn style_id(&self) -> TextStyleId {
        self.style_id
    }

    /// Borrows decoration rectangles in visual drawing order.
    pub fn rects(&self) -> &[Rect] {
        &self.rects
    }

    /// Moves the decoration rectangles out of this batch.
    pub fn into_rects(self) -> Vec<Rect> {
        self.rects
    }
}

/// Converts resolved underline and strike-through metrics into target-space rectangles.
///
/// Adjacent output using the same [`TextStyleId`] is coalesced into one batch.
/// This adapter remains backend-independent: callers retain paint resolution and
/// command ordering, and backends only receive generic rectangle commands.
pub fn layout_decoration_batches(
    layout: &TextLayout,
    origin: Point,
) -> Result<Vec<TextDecorationBatch>, TextGpuError> {
    let mut batches = Vec::new();
    for line in layout.lines() {
        if line.advance_x_bits() <= 0 {
            continue;
        }
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
        if line.decoration_segments().is_empty() {
            let right = line_x
                .checked_add(line.advance_x_bits())
                .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))?;
            for metrics in [line.underline_metrics(), line.strike_through_metrics()]
                .into_iter()
                .flatten()
            {
                push_decoration(
                    &mut batches,
                    TextStyleId::DEFAULT,
                    decoration_rect(line_x, right, baseline_y, metrics)?,
                )?;
            }
            continue;
        }
        for segment in line.decoration_segments() {
            let left = line_x
                .checked_add(segment.left_bits())
                .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))?;
            let right = line_x
                .checked_add(segment.right_bits())
                .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))?;
            for metrics in [
                segment.underline_metrics(),
                segment.strike_through_metrics(),
            ]
            .into_iter()
            .flatten()
            {
                push_decoration(
                    &mut batches,
                    segment.style_id(),
                    decoration_rect(left, right, baseline_y, metrics)?,
                )?;
            }
        }
    }
    Ok(batches)
}

fn decoration_rect(
    left_bits: i32,
    right_bits: i32,
    baseline_bits: i32,
    metrics: TextDecorationMetrics,
) -> Result<Rect, TextGpuError> {
    let center = baseline_bits
        .checked_add(metrics.offset_bits())
        .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))?;
    let top = center
        .checked_sub(metrics.thickness_bits() / 2)
        .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))?;
    let bottom = top
        .checked_add(metrics.thickness_bits())
        .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))?;
    Rect::new(
        Scalar::from_bits(left_bits),
        Scalar::from_bits(top),
        Scalar::from_bits(right_bits),
        Scalar::from_bits(bottom),
    )
    .map_err(|_| TextGpuError::new(TextGpuErrorCode::NumericOverflow))
}

fn push_decoration(
    batches: &mut Vec<TextDecorationBatch>,
    style_id: TextStyleId,
    rect: Rect,
) -> Result<(), TextGpuError> {
    if let Some(batch) = batches.last_mut()
        && batch.style_id == style_id
    {
        batch
            .rects
            .try_reserve(1)
            .map_err(|_| TextGpuError::new(TextGpuErrorCode::AllocationFailed))?;
        batch.rects.push(rect);
        return Ok(());
    }

    batches
        .try_reserve(1)
        .map_err(|_| TextGpuError::new(TextGpuErrorCode::AllocationFailed))?;
    let mut rects = Vec::new();
    rects
        .try_reserve(1)
        .map_err(|_| TextGpuError::new(TextGpuErrorCode::AllocationFailed))?;
    rects.push(rect);
    batches.push(TextDecorationBatch { style_id, rects });
    Ok(())
}
