use std::sync::Arc;

use skia_core::{ClipOp, FillRule, SkiaError, SkiaErrorCode};

use crate::canvas::{Contour, DeviceRect, contains, pixel_center};

pub(crate) fn apply_clip(
    width: u32,
    height: u32,
    scissor: DeviceRect,
    current: Option<&[u8]>,
    contours: &[Contour],
    rule: FillRule,
    op: ClipOp,
) -> Result<Arc<[u8]>, SkiaError> {
    let length = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|value| usize::try_from(value).ok())
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(length)
        .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
    output.resize(length, 0_u8);

    let surface = DeviceRect {
        left: 0,
        top: 0,
        right: i64::from(width),
        bottom: i64::from(height),
    };
    let bounds = scissor.intersection(surface);
    for y in bounds.top..bounds.bottom {
        for x in bounds.left..bounds.right {
            let index = mask_index(width, x, y)?;
            let was_visible = current.is_none_or(|mask| mask[index] != 0);
            if !was_visible {
                continue;
            }
            let inside = contains(contours, pixel_center(x, y)?, rule)?;
            let visible = match op {
                ClipOp::Intersect => inside,
                ClipOp::Difference => !inside,
            };
            if visible {
                output[index] = u8::MAX;
            }
        }
    }
    Ok(output.into())
}

pub(crate) fn mask_index(width: u32, x: i64, y: i64) -> Result<usize, SkiaError> {
    y.checked_mul(i64::from(width))
        .and_then(|value| value.checked_add(x))
        .and_then(|value| usize::try_from(value).ok())
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))
}
