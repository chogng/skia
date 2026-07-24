use super::TileMode;
use crate::paint::Color;
use crate::sampling::{SamplingFilter, SamplingOptions};
use skia_error::{SkiaError, SkiaErrorCode};
use skia_geometry::Point;
use skia_image::{Image, ImageErrorCode};

/// Immutable bitmap source sampled in its own pixel-coordinate space.
///
/// The image occupies the local rectangle from `(0, 0)` to `(width, height)`.
/// Texel centers are at half-integer coordinates, so a local matrix can place
/// or scale this shader without changing its sampling convention.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ImageShader {
    image: Image,
    sampling: SamplingOptions,
    x_tile_mode: TileMode,
    y_tile_mode: TileMode,
}

impl ImageShader {
    /// Converts an immutable source image to rendering sRGB RGBA8 and retains it.
    pub fn new(
        image: Image,
        sampling: SamplingOptions,
        x_tile_mode: TileMode,
        y_tile_mode: TileMode,
    ) -> Result<Self, SkiaError> {
        let image = image.to_rendering_image().map_err(map_image_error)?;
        Ok(Self {
            image,
            sampling,
            x_tile_mode,
            y_tile_mode,
        })
    }

    /// Borrows the normalized rendering image.
    pub const fn image(&self) -> &Image {
        &self.image
    }

    /// Returns the reconstruction filter.
    pub const fn sampling(&self) -> SamplingOptions {
        self.sampling
    }

    /// Returns the horizontal out-of-range coordinate policy.
    pub const fn x_tile_mode(&self) -> TileMode {
        self.x_tile_mode
    }

    /// Returns the vertical out-of-range coordinate policy.
    pub const fn y_tile_mode(&self) -> TileMode {
        self.y_tile_mode
    }

    /// Samples one image-space coordinate with the selected filtering and tiling.
    pub fn sample(&self, point: Point) -> Result<Color, SkiaError> {
        let pixels = match self.sampling.filter() {
            SamplingFilter::Nearest => self.sample_nearest(point)?,
            SamplingFilter::Linear => self.sample_linear(point)?,
        };
        Ok(Color::rgba(pixels[0], pixels[1], pixels[2], pixels[3]))
    }

    fn sample_nearest(&self, point: Point) -> Result<[u8; 4], SkiaError> {
        let x = tile_index(
            floor_q16(i64::from(point.x().bits())),
            self.image.width(),
            self.x_tile_mode,
        )?;
        let y = tile_index(
            floor_q16(i64::from(point.y().bits())),
            self.image.height(),
            self.y_tile_mode,
        )?;
        self.image
            .pixel_at(x, y)
            .ok_or(SkiaError::new(SkiaErrorCode::InvalidResource))
    }

    fn sample_linear(&self, point: Point) -> Result<[u8; 4], SkiaError> {
        let horizontal = linear_axis(point.x().bits());
        let vertical = linear_axis(point.y().bits());
        let top_left = self.pixel(horizontal.first, vertical.first)?;
        let top_right = self.pixel(horizontal.second, vertical.first)?;
        let bottom_left = self.pixel(horizontal.first, vertical.second)?;
        let bottom_right = self.pixel(horizontal.second, vertical.second)?;
        let first_x_weight = Q16_ONE - horizontal.second_weight;
        let first_y_weight = Q16_ONE - vertical.second_weight;
        let denominator = i128::from(Q16_ONE) * i128::from(Q16_ONE);
        let mut output = [0_u8; 4];
        for channel in 0..4 {
            let top = i128::from(top_left[channel]) * i128::from(first_x_weight)
                + i128::from(top_right[channel]) * i128::from(horizontal.second_weight);
            let bottom = i128::from(bottom_left[channel]) * i128::from(first_x_weight)
                + i128::from(bottom_right[channel]) * i128::from(horizontal.second_weight);
            let value = top
                .checked_mul(i128::from(first_y_weight))
                .and_then(|value| {
                    bottom
                        .checked_mul(i128::from(vertical.second_weight))
                        .and_then(|bottom| value.checked_add(bottom))
                })
                .and_then(|value| value.checked_add(denominator / 2))
                .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?
                / denominator;
            output[channel] =
                u8::try_from(value).map_err(|_| SkiaError::new(SkiaErrorCode::NumericOverflow))?;
        }
        Ok(output)
    }

    fn pixel(&self, x: i64, y: i64) -> Result<[u8; 4], SkiaError> {
        let x = tile_index(x, self.image.width(), self.x_tile_mode)?;
        let y = tile_index(y, self.image.height(), self.y_tile_mode)?;
        self.image
            .pixel_at(x, y)
            .ok_or(SkiaError::new(SkiaErrorCode::InvalidResource))
    }
}

const Q16_ONE: i64 = 1 << 16;

#[derive(Clone, Copy)]
struct LinearAxis {
    first: i64,
    second: i64,
    second_weight: i64,
}

fn linear_axis(point_bits: i32) -> LinearAxis {
    let shifted = i64::from(point_bits) - (Q16_ONE / 2);
    LinearAxis {
        first: floor_q16(shifted),
        second: floor_q16(shifted).saturating_add(1),
        second_weight: shifted.rem_euclid(Q16_ONE),
    }
}

fn tile_index(index: i64, extent: u32, mode: TileMode) -> Result<u32, SkiaError> {
    let extent = i64::from(extent);
    if extent <= 0 {
        return Err(SkiaError::new(SkiaErrorCode::InvalidImage));
    }
    let tiled = match mode {
        TileMode::Clamp => index.clamp(0, extent - 1),
        TileMode::Repeat => index.rem_euclid(extent),
        TileMode::Mirror => {
            let period = extent
                .checked_mul(2)
                .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
            let offset = index.rem_euclid(period);
            if offset < extent {
                offset
            } else {
                period - offset - 1
            }
        }
    };
    u32::try_from(tiled).map_err(|_| SkiaError::new(SkiaErrorCode::NumericOverflow))
}

fn floor_q16(value: i64) -> i64 {
    if value >= 0 {
        value >> 16
    } else {
        -((-value + 65_535) >> 16)
    }
}

fn map_image_error(error: skia_image::ImageError) -> SkiaError {
    match error.code() {
        ImageErrorCode::AllocationFailed => SkiaError::new(SkiaErrorCode::AllocationFailed),
        _ => SkiaError::new(SkiaErrorCode::InvalidImage),
    }
}
