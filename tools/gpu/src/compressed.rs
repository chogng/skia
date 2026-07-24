use skia_core::Color;
use skia_image::Image;

use crate::TestGpuResourceError;

/// Compresses an RGBA8 fixture containing transparent black, opaque black,
/// and one opaque non-black color into BC1 blocks.
///
/// The result uses the transparent BC1 variant, so transparent source pixels
/// select color index three. This mirrors the narrow two-color compressor used
/// by upstream GPU tests without pretending to be a general image encoder.
pub fn two_color_bc1_compress(
    image: &Image,
    other: Color,
) -> Result<Vec<u8>, TestGpuResourceError> {
    if !other.is_opaque() || other == Color::BLACK {
        return Err(TestGpuResourceError::UnsupportedColor);
    }
    let blocks_x = image.width().div_ceil(4);
    let blocks_y = image.height().div_ceil(4);
    let block_count = usize::try_from(blocks_x)
        .ok()
        .and_then(|width| {
            usize::try_from(blocks_y)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .ok_or(TestGpuResourceError::DimensionsOverflow)?;
    let byte_count = block_count
        .checked_mul(8)
        .ok_or(TestGpuResourceError::DimensionsOverflow)?;
    let mut compressed = Vec::new();
    compressed
        .try_reserve_exact(byte_count)
        .map_err(|_| TestGpuResourceError::AllocationFailed)?;
    let color0 = rgb565(Color::BLACK);
    let color1 = rgb565(other);
    if color1 <= color0 {
        return Err(TestGpuResourceError::UnsupportedColor);
    }
    for block_y in 0..blocks_y {
        for block_x in 0..blocks_x {
            let mut indices = 0_u32;
            for y in 0..4_u32 {
                for x in 0..4_u32 {
                    let source_x = block_x * 4 + x;
                    let source_y = block_y * 4 + y;
                    if source_x >= image.width() || source_y >= image.height() {
                        continue;
                    }
                    let color = image
                        .pixel_at(source_x, source_y)
                        .map(|channels| {
                            Color::rgba(channels[0], channels[1], channels[2], channels[3])
                        })
                        .ok_or(TestGpuResourceError::DimensionsOverflow)?;
                    let index = if color == Color::TRANSPARENT {
                        3_u32
                    } else if color == Color::BLACK {
                        0_u32
                    } else if color == other {
                        1_u32
                    } else {
                        return Err(TestGpuResourceError::UnsupportedColor);
                    };
                    let shift = (y * 4 + x) * 2;
                    indices |= index << shift;
                }
            }
            compressed.extend_from_slice(&color0.to_le_bytes());
            compressed.extend_from_slice(&color1.to_le_bytes());
            compressed.extend_from_slice(&indices.to_le_bytes());
        }
    }
    Ok(compressed)
}

fn rgb565(color: Color) -> u16 {
    let red = (u16::from(color.red()) * 31 + 127) / 255;
    let green = (u16::from(color.green()) * 63 + 127) / 255;
    let blue = (u16::from(color.blue()) * 31 + 127) / 255;
    (red << 11) | (green << 5) | blue
}
