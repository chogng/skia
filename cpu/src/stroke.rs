use skia_core::{SkiaError, SkiaErrorCode, StrokeCap, StrokeJoin, StrokeOptions};

use crate::canvas::{Contour, DeviceRect, ceil_q16_i64, contour_bounds, floor_q16_i64};

pub(crate) fn stroke_bounds(
    contours: &[Contour],
    options: &StrokeOptions,
) -> Result<DeviceRect, SkiaError> {
    let bounds = contour_bounds(contours);
    let radius = i64::from(options.width().bits()).div_euclid(2);
    let mut extent = radius;
    if options.cap() == StrokeCap::Square {
        extent = extent
            .checked_mul(2)
            .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    }
    if options.join() == StrokeJoin::Miter {
        let miter_extent = i128::from(radius)
            .checked_mul(i128::from(options.miter_limit().bits()))
            .and_then(|value| value.checked_add((1 << 16) - 1))
            .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?
            / (1 << 16);
        extent = extent.max(
            i64::try_from(miter_extent)
                .map_err(|_| SkiaError::new(SkiaErrorCode::NumericOverflow))?,
        );
    }
    let left = bounds
        .left
        .checked_mul(1_i64 << 16)
        .and_then(|value| value.checked_sub(extent))
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    let top = bounds
        .top
        .checked_mul(1_i64 << 16)
        .and_then(|value| value.checked_sub(extent))
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    let right = bounds
        .right
        .checked_mul(1_i64 << 16)
        .and_then(|value| value.checked_add(extent))
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    let bottom = bounds
        .bottom
        .checked_mul(1_i64 << 16)
        .and_then(|value| value.checked_add(extent))
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    Ok(DeviceRect {
        left: floor_q16_i64(left),
        top: floor_q16_i64(top),
        right: ceil_q16_i64(right),
        bottom: ceil_q16_i64(bottom),
    })
}
