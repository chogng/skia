use crate::{TextDecorationMetrics, TextError, TextErrorCode};

const MIN_PATTERN_UNIT_BITS: i32 = 1 << 16;
const MAX_DECORATION_RECTS: usize = 1_000_000;

/// Visual pattern used to draw an underline or strike-through.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum TextDecorationStyle {
    /// Draw one uninterrupted line.
    #[default]
    Solid,
    /// Draw repeating long marks separated by gaps.
    Dashed,
    /// Draw repeating square dots separated by equal gaps.
    Dotted,
    /// Draw a connected stepped wave around the font-provided line center.
    Wavy,
}

/// One axis-aligned Q16.16 rectangle in resolved text-decoration geometry.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TextDecorationRect {
    left_bits: i32,
    top_bits: i32,
    right_bits: i32,
    bottom_bits: i32,
}

impl TextDecorationRect {
    /// Returns the inclusive left coordinate.
    pub const fn left_bits(self) -> i32 {
        self.left_bits
    }

    /// Returns the inclusive top coordinate.
    pub const fn top_bits(self) -> i32 {
        self.top_bits
    }

    /// Returns the exclusive right coordinate.
    pub const fn right_bits(self) -> i32 {
        self.right_bits
    }

    /// Returns the exclusive bottom coordinate.
    pub const fn bottom_bits(self) -> i32 {
        self.bottom_bits
    }
}

/// Expands one resolved decoration line into backend-neutral rectangle geometry.
///
/// Pattern phase starts at `left_bits`, so adjacent style ranges remain
/// deterministic without depending on a renderer. Dashed and dotted spacing,
/// plus wavy amplitude, scale from the font-provided thickness with a one-pixel
/// minimum pattern unit.
pub fn text_decoration_rects(
    left_bits: i32,
    right_bits: i32,
    baseline_bits: i32,
    metrics: TextDecorationMetrics,
    style: TextDecorationStyle,
) -> Result<Vec<TextDecorationRect>, TextError> {
    if right_bits <= left_bits || metrics.thickness_bits() <= 0 {
        return Err(TextError::new(TextErrorCode::InvalidLayout));
    }
    let thickness = metrics.thickness_bits();
    let center = baseline_bits
        .checked_add(metrics.offset_bits())
        .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
    let top = center
        .checked_sub(thickness / 2)
        .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
    let bottom = top
        .checked_add(thickness)
        .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
    let unit = thickness.max(MIN_PATTERN_UNIT_BITS);
    let mut rects = Vec::new();

    match style {
        TextDecorationStyle::Solid => {
            push_rect(&mut rects, left_bits, top, right_bits, bottom)?;
        }
        TextDecorationStyle::Dashed => {
            let dash = unit
                .checked_mul(3)
                .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
            let stride = unit
                .checked_mul(5)
                .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
            let mut left = left_bits;
            while left < right_bits {
                let right = left.saturating_add(dash).min(right_bits);
                push_rect(&mut rects, left, top, right, bottom)?;
                if right == right_bits {
                    break;
                }
                left = left
                    .checked_add(stride)
                    .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
            }
        }
        TextDecorationStyle::Dotted => {
            let stride = unit
                .checked_mul(2)
                .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
            let mut left = left_bits;
            while left < right_bits {
                let right = left.saturating_add(thickness).min(right_bits);
                push_rect(&mut rects, left, top, right, bottom)?;
                if right == right_bits {
                    break;
                }
                left = left
                    .checked_add(stride)
                    .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
            }
        }
        TextDecorationStyle::Wavy => {
            let offsets = [0, -unit, 0, unit];
            let mut left = left_bits;
            let mut phase = 0_usize;
            while left < right_bits {
                let right = left.saturating_add(unit).min(right_bits);
                let first = offsets[phase];
                let second = offsets[(phase + 1) % offsets.len()];
                let wave_top = center
                    .checked_add(first.min(second))
                    .and_then(|value| value.checked_sub(thickness / 2))
                    .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
                let wave_bottom = center
                    .checked_add(first.max(second))
                    .and_then(|value| value.checked_add(thickness - thickness / 2))
                    .ok_or(TextError::new(TextErrorCode::NumericOverflow))?;
                push_rect(&mut rects, left, wave_top, right, wave_bottom)?;
                left = right;
                phase = (phase + 1) % offsets.len();
            }
        }
    }
    Ok(rects)
}

fn push_rect(
    rects: &mut Vec<TextDecorationRect>,
    left_bits: i32,
    top_bits: i32,
    right_bits: i32,
    bottom_bits: i32,
) -> Result<(), TextError> {
    if rects.len() == MAX_DECORATION_RECTS {
        return Err(TextError::new(TextErrorCode::ResourceLimit));
    }
    rects
        .try_reserve(1)
        .map_err(|_| TextError::new(TextErrorCode::AllocationFailed))?;
    rects.push(TextDecorationRect {
        left_bits,
        top_bits,
        right_bits,
        bottom_bits,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics() -> TextDecorationMetrics {
        TextDecorationMetrics::from_bits_for_test(1 << 16, 2 << 16)
    }

    #[test]
    fn patterns_expand_with_deterministic_phase() {
        let dashed = text_decoration_rects(
            0,
            20 << 16,
            10 << 16,
            metrics(),
            TextDecorationStyle::Dashed,
        )
        .expect("dashes");
        assert_eq!(dashed.len(), 2);
        assert_eq!((dashed[0].left_bits, dashed[0].right_bits), (0, 6 << 16));
        assert_eq!(
            (dashed[1].left_bits, dashed[1].right_bits),
            (10 << 16, 16 << 16)
        );

        let dotted = text_decoration_rects(
            0,
            10 << 16,
            10 << 16,
            metrics(),
            TextDecorationStyle::Dotted,
        )
        .expect("dots");
        assert_eq!(dotted.len(), 3);
        assert_eq!(
            (dotted[1].left_bits, dotted[1].right_bits),
            (4 << 16, 6 << 16)
        );

        let wavy =
            text_decoration_rects(0, 8 << 16, 10 << 16, metrics(), TextDecorationStyle::Wavy)
                .expect("wave");
        assert_eq!(wavy.len(), 4);
        assert!(wavy[0].top_bits < wavy[2].top_bits);
        assert!(wavy[0].bottom_bits < wavy[2].bottom_bits);
    }
}
