//! Text-to-GPU adapter for atlas-backed glyph rendering.
//!
//! `skia-gpu` deliberately owns only generic atlas images, positioned quads,
//! and backend-neutral draw commands. This crate owns the text-specific cache
//! keys, glyph rasterization, layout traversal, and registration metadata that
//! connect [`skia_core::TextLayout`] to those generic GPU primitives.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::{collections::HashMap, fmt};

use skia_core::{
    FontCollection, FontId, GlyphBitmap, GlyphBitmapFormat, GlyphId, GlyphRun, Paint, Point, Rect,
    Scalar, TextError, TextErrorCode, TextLayout,
};
use skia_gpu::{
    GpuAtlasRect, GpuCommandEncoder, GpuCommandError, GpuCommandErrorCode, GpuGlyphAtlas,
    GpuGlyphAtlasId, GpuGlyphQuad,
};
use skia_image::Image;

/// Stable machine-readable text-to-GPU adapter failure.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GpuTextErrorCode {
    /// Atlas dimensions or glyph ceilings are invalid.
    InvalidLimits,
    /// The requested encoder transform mode is unsupported.
    UnsupportedTransform,
    /// A coordinate or byte-size calculation overflowed.
    NumericOverflow,
    /// Atlas storage, glyph count, or command capacity was exhausted.
    ResourceLimit,
    /// A font, layout, atlas, or encoder resource was inconsistent.
    InvalidResource,
    /// A bounded allocation could not be reserved.
    AllocationFailed,
}

/// Source-redacted text-to-GPU adapter error.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GpuTextError {
    code: GpuTextErrorCode,
}

impl GpuTextError {
    const fn new(code: GpuTextErrorCode) -> Self {
        Self { code }
    }

    /// Returns the stable failure code.
    pub const fn code(self) -> GpuTextErrorCode {
        self.code
    }
}

impl fmt::Display for GpuTextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}", self.code)
    }
}

impl std::error::Error for GpuTextError {}

impl From<GpuCommandError> for GpuTextError {
    fn from(error: GpuCommandError) -> Self {
        let code = match error.code() {
            GpuCommandErrorCode::InvalidLimits => GpuTextErrorCode::InvalidLimits,
            GpuCommandErrorCode::UnsupportedTransform => GpuTextErrorCode::UnsupportedTransform,
            GpuCommandErrorCode::NumericOverflow => GpuTextErrorCode::NumericOverflow,
            GpuCommandErrorCode::ResourceLimit => GpuTextErrorCode::ResourceLimit,
            GpuCommandErrorCode::AllocationFailed => GpuTextErrorCode::AllocationFailed,
            GpuCommandErrorCode::InvalidSurface
            | GpuCommandErrorCode::RestoreUnderflow
            | GpuCommandErrorCode::InvalidResource => GpuTextErrorCode::InvalidResource,
        };
        Self::new(code)
    }
}

/// Stable identity of one rasterized glyph inside a text atlas.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GpuTextGlyphKey {
    font: FontId,
    glyph: GlyphId,
    font_size_bits: i32,
    format: GlyphBitmapFormat,
}

impl GpuTextGlyphKey {
    /// Creates a cache key from one validated glyph bitmap.
    pub const fn from_bitmap(bitmap: &GlyphBitmap) -> Self {
        Self {
            font: bitmap.font(),
            glyph: bitmap.glyph(),
            font_size_bits: bitmap.font_size_bits(),
            format: bitmap.format(),
        }
    }

    /// Returns the immutable font-instance identity.
    pub const fn font(self) -> FontId {
        self.font
    }

    /// Returns the font-local glyph identity.
    pub const fn glyph(self) -> GlyphId {
        self.glyph
    }

    /// Returns the Q16.16 raster size.
    pub const fn font_size_bits(self) -> i32 {
        self.font_size_bits
    }

    /// Returns the atlas sample interpretation.
    pub const fn format(self) -> GlyphBitmapFormat {
        self.format
    }
}

/// Packed placement metadata for one text glyph bitmap.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GpuTextAtlasEntry {
    key: GpuTextGlyphKey,
    source: GpuAtlasRect,
    left: i32,
    top: i32,
}

impl GpuTextAtlasEntry {
    /// Returns the text glyph cache key.
    pub const fn key(self) -> GpuTextGlyphKey {
        self.key
    }

    /// Returns the atlas pixel rectangle.
    pub const fn source(self) -> GpuAtlasRect {
        self.source
    }

    /// Returns the raster placement offset right from the glyph origin.
    pub const fn left(self) -> i32 {
        self.left
    }

    /// Returns the raster placement offset above the glyph baseline.
    pub const fn top(self) -> i32 {
        self.top
    }
}

/// Packed text atlas before it is moved into one GPU command encoder.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GpuTextAtlas {
    atlas: GpuGlyphAtlas,
    entries: HashMap<GpuTextGlyphKey, GpuTextAtlasEntry>,
}

impl GpuTextAtlas {
    /// Borrows the upload-ready generic GPU atlas.
    pub const fn atlas(&self) -> &GpuGlyphAtlas {
        &self.atlas
    }

    /// Resolves one exact text glyph cache key.
    pub fn entry(&self, key: GpuTextGlyphKey) -> Option<GpuTextAtlasEntry> {
        self.entries.get(&key).copied()
    }

    /// Moves this atlas into an encoder and retains only its layout index.
    ///
    /// The returned registration binds the metadata to the exact command-
    /// buffer resource ID, preventing a layout from accidentally using the
    /// index of one atlas with the pixels of another.
    pub fn register<'encoder>(
        self,
        encoder: &'encoder mut GpuCommandEncoder,
    ) -> Result<GpuTextAtlasRegistration<'encoder>, GpuTextError> {
        let Self { atlas, entries } = self;
        let atlas = encoder.add_glyph_atlas(atlas)?;
        Ok(GpuTextAtlasRegistration {
            encoder,
            atlas,
            entries,
        })
    }
}

/// Text atlas metadata bound to one command-buffer-local GPU resource.
#[derive(Debug)]
pub struct GpuTextAtlasRegistration<'encoder> {
    encoder: &'encoder mut GpuCommandEncoder,
    atlas: GpuGlyphAtlasId,
    entries: HashMap<GpuTextGlyphKey, GpuTextAtlasEntry>,
}

impl GpuTextAtlasRegistration<'_> {
    /// Returns the command-buffer-local generic atlas identifier.
    pub const fn atlas_id(&self) -> GpuGlyphAtlasId {
        self.atlas
    }

    /// Borrows the exact encoder that owns this registered atlas.
    ///
    /// Use this for state or non-text commands that must be interleaved between
    /// text batches. The registration retains the exclusive borrow until it is
    /// dropped, so its text index cannot be submitted to another encoder.
    pub fn encoder(&mut self) -> &mut GpuCommandEncoder {
        self.encoder
    }

    /// Records all raster glyphs in one text layout as one atlas-backed batch.
    ///
    /// Empty glyphs and glyphs absent from the registered atlas are skipped.
    /// Text decorations remain ordinary geometry commands owned by the upper
    /// renderer rather than this glyph adapter.
    pub fn draw_layout(
        &mut self,
        layout: &TextLayout,
        origin: Point,
        paint: Paint,
    ) -> Result<(), GpuTextError> {
        let mut glyphs = Vec::new();
        for line in layout.lines() {
            let Some(paragraph) = line.paragraph() else {
                continue;
            };
            let line_x = origin
                .x()
                .bits()
                .checked_add(line.offset_x_bits())
                .ok_or(GpuTextError::new(GpuTextErrorCode::NumericOverflow))?;
            let baseline_y = origin
                .y()
                .bits()
                .checked_add(line.baseline_y_bits())
                .ok_or(GpuTextError::new(GpuTextErrorCode::NumericOverflow))?;
            for shaped in paragraph.runs() {
                let run = shaped.glyph_run();
                if shaped.glyph_offsets_x_bits().len() != run.glyphs().len() {
                    return Err(GpuTextError::new(GpuTextErrorCode::InvalidResource));
                }
                let run_x = line_x
                    .checked_add(shaped.origin_x_bits())
                    .ok_or(GpuTextError::new(GpuTextErrorCode::NumericOverflow))?;
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
                        .ok_or(GpuTextError::new(GpuTextErrorCode::NumericOverflow))?;
                    let top = baseline_y
                        .checked_add(glyph_y)
                        .and_then(|value| value.checked_sub(bitmap_top))
                        .ok_or(GpuTextError::new(GpuTextErrorCode::NumericOverflow))?;
                    let right = left
                        .checked_add(pixel_bits_u32(entry.source().width())?)
                        .ok_or(GpuTextError::new(GpuTextErrorCode::NumericOverflow))?;
                    let bottom = top
                        .checked_add(pixel_bits_u32(entry.source().height())?)
                        .ok_or(GpuTextError::new(GpuTextErrorCode::NumericOverflow))?;
                    let destination = Rect::new(
                        Scalar::from_bits(left),
                        Scalar::from_bits(top),
                        Scalar::from_bits(right),
                        Scalar::from_bits(bottom),
                    )
                    .map_err(|_| GpuTextError::new(GpuTextErrorCode::NumericOverflow))?;
                    glyphs
                        .try_reserve(1)
                        .map_err(|_| GpuTextError::new(GpuTextErrorCode::AllocationFailed))?;
                    glyphs.push(GpuGlyphQuad::new(
                        entry.source(),
                        destination,
                        entry.key().format() == GlyphBitmapFormat::Alpha8,
                    ));
                }
            }
        }
        self.encoder
            .draw_glyph_batch(self.atlas, glyphs, paint)
            .map_err(Into::into)
    }

    fn glyph_entry(
        &self,
        font: FontId,
        glyph: GlyphId,
        font_size_bits: i32,
    ) -> Option<GpuTextAtlasEntry> {
        [GlyphBitmapFormat::Alpha8, GlyphBitmapFormat::Rgba8]
            .into_iter()
            .find_map(|format| {
                self.entries
                    .get(&GpuTextGlyphKey {
                        font,
                        glyph,
                        font_size_bits,
                        format,
                    })
                    .copied()
            })
    }
}

/// Bounded shelf packer for reusable text glyph atlases.
#[derive(Debug)]
pub struct GpuTextAtlasBuilder {
    width: u32,
    height: u32,
    max_glyphs: usize,
    pixels: Vec<u8>,
    entries: HashMap<GpuTextGlyphKey, GpuTextAtlasEntry>,
    cursor_x: u32,
    cursor_y: u32,
    row_height: u32,
}

impl GpuTextAtlasBuilder {
    /// Allocates one transparent, bounded RGBA8 text atlas.
    pub fn new(width: u32, height: u32, max_glyphs: usize) -> Result<Self, GpuTextError> {
        if width == 0 || height == 0 || max_glyphs == 0 {
            return Err(GpuTextError::new(GpuTextErrorCode::InvalidLimits));
        }
        let length = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|value| value.checked_mul(4))
            .and_then(|value| usize::try_from(value).ok())
            .ok_or(GpuTextError::new(GpuTextErrorCode::ResourceLimit))?;
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(length)
            .map_err(|_| GpuTextError::new(GpuTextErrorCode::AllocationFailed))?;
        pixels.resize(length, 0);
        Ok(Self {
            width,
            height,
            max_glyphs,
            pixels,
            entries: HashMap::new(),
            cursor_x: 1,
            cursor_y: 1,
            row_height: 0,
        })
    }

    /// Inserts or reuses one exact glyph bitmap.
    pub fn insert(&mut self, bitmap: &GlyphBitmap) -> Result<GpuTextAtlasEntry, GpuTextError> {
        let key = GpuTextGlyphKey::from_bitmap(bitmap);
        if let Some(entry) = self.entries.get(&key) {
            return Ok(*entry);
        }
        if self.entries.len() == self.max_glyphs || bitmap.width() == 0 || bitmap.height() == 0 {
            return Err(GpuTextError::new(GpuTextErrorCode::ResourceLimit));
        }
        let padded_width = bitmap
            .width()
            .checked_add(2)
            .ok_or(GpuTextError::new(GpuTextErrorCode::NumericOverflow))?;
        let padded_height = bitmap
            .height()
            .checked_add(2)
            .ok_or(GpuTextError::new(GpuTextErrorCode::NumericOverflow))?;
        if padded_width > self.width || padded_height > self.height {
            return Err(GpuTextError::new(GpuTextErrorCode::ResourceLimit));
        }
        if self
            .cursor_x
            .checked_add(bitmap.width())
            .and_then(|value| value.checked_add(1))
            .is_none_or(|value| value > self.width)
        {
            self.cursor_x = 1;
            self.cursor_y = self
                .cursor_y
                .checked_add(self.row_height)
                .and_then(|value| value.checked_add(1))
                .ok_or(GpuTextError::new(GpuTextErrorCode::NumericOverflow))?;
            self.row_height = 0;
        }
        if self
            .cursor_y
            .checked_add(bitmap.height())
            .and_then(|value| value.checked_add(1))
            .is_none_or(|value| value > self.height)
        {
            return Err(GpuTextError::new(GpuTextErrorCode::ResourceLimit));
        }
        let source = GpuAtlasRect::new(
            self.cursor_x,
            self.cursor_y,
            bitmap.width(),
            bitmap.height(),
        )?;
        self.copy_bitmap(source, bitmap)?;
        let entry = GpuTextAtlasEntry {
            key,
            source,
            left: bitmap.left(),
            top: bitmap.top(),
        };
        self.entries
            .try_reserve(1)
            .map_err(|_| GpuTextError::new(GpuTextErrorCode::AllocationFailed))?;
        self.entries.insert(key, entry);
        self.cursor_x = self
            .cursor_x
            .checked_add(bitmap.width())
            .and_then(|value| value.checked_add(1))
            .ok_or(GpuTextError::new(GpuTextErrorCode::NumericOverflow))?;
        self.row_height = self.row_height.max(bitmap.height());
        Ok(entry)
    }

    /// Rasterizes and inserts every drawable glyph referenced by a text layout.
    pub fn insert_layout(
        &mut self,
        layout: &TextLayout,
        fonts: &FontCollection,
    ) -> Result<(), GpuTextError> {
        for line in layout.lines() {
            let Some(paragraph) = line.paragraph() else {
                continue;
            };
            for shaped in paragraph.runs() {
                let run = shaped.glyph_run();
                let face = fonts
                    .face(run.font())
                    .ok_or(GpuTextError::new(GpuTextErrorCode::InvalidResource))?;
                for glyph in run.glyphs() {
                    if let Some(bitmap) = face
                        .rasterize_glyph(glyph.glyph(), run.font_size_bits())
                        .map_err(map_text_error)?
                    {
                        self.insert(&bitmap)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Publishes the immutable generic atlas and text glyph index.
    pub fn finish(self) -> Result<GpuTextAtlas, GpuTextError> {
        let image = Image::from_rgba8(self.width, self.height, self.pixels)
            .map_err(|_| GpuTextError::new(GpuTextErrorCode::InvalidResource))?;
        Ok(GpuTextAtlas {
            atlas: GpuGlyphAtlas::from_image(image),
            entries: self.entries,
        })
    }

    fn copy_bitmap(
        &mut self,
        destination: GpuAtlasRect,
        bitmap: &GlyphBitmap,
    ) -> Result<(), GpuTextError> {
        let source_stride = usize::try_from(bitmap.width())
            .ok()
            .and_then(|value| value.checked_mul(bitmap.format().bytes_per_pixel()))
            .ok_or(GpuTextError::new(GpuTextErrorCode::NumericOverflow))?;
        let destination_stride = usize::try_from(self.width)
            .ok()
            .and_then(|value| value.checked_mul(4))
            .ok_or(GpuTextError::new(GpuTextErrorCode::NumericOverflow))?;
        let destination_row_bytes = usize::try_from(bitmap.width())
            .ok()
            .and_then(|value| value.checked_mul(4))
            .ok_or(GpuTextError::new(GpuTextErrorCode::NumericOverflow))?;
        for row in 0..bitmap.height() {
            let source_start = usize::try_from(row)
                .ok()
                .and_then(|value| value.checked_mul(source_stride))
                .ok_or(GpuTextError::new(GpuTextErrorCode::NumericOverflow))?;
            let source_end = source_start
                .checked_add(source_stride)
                .ok_or(GpuTextError::new(GpuTextErrorCode::NumericOverflow))?;
            let destination_y = destination
                .y()
                .checked_add(row)
                .ok_or(GpuTextError::new(GpuTextErrorCode::NumericOverflow))?;
            let destination_start = usize::try_from(destination_y)
                .ok()
                .and_then(|value| value.checked_mul(destination_stride))
                .and_then(|value| {
                    usize::try_from(destination.x())
                        .ok()
                        .and_then(|x| x.checked_mul(4))
                        .and_then(|x| value.checked_add(x))
                })
                .ok_or(GpuTextError::new(GpuTextErrorCode::NumericOverflow))?;
            let destination_end = destination_start
                .checked_add(destination_row_bytes)
                .ok_or(GpuTextError::new(GpuTextErrorCode::NumericOverflow))?;
            let source = bitmap
                .pixels()
                .get(source_start..source_end)
                .ok_or(GpuTextError::new(GpuTextErrorCode::InvalidResource))?;
            let destination = self
                .pixels
                .get_mut(destination_start..destination_end)
                .ok_or(GpuTextError::new(GpuTextErrorCode::InvalidResource))?;
            match bitmap.format() {
                GlyphBitmapFormat::Alpha8 => {
                    for (alpha, pixel) in source.iter().zip(destination.chunks_exact_mut(4)) {
                        pixel.copy_from_slice(&[255, 255, 255, *alpha]);
                    }
                }
                GlyphBitmapFormat::Rgba8 => destination.copy_from_slice(source),
            }
        }
        Ok(())
    }
}

fn map_text_error(error: TextError) -> GpuTextError {
    let code = match error.code() {
        TextErrorCode::AllocationFailed => GpuTextErrorCode::AllocationFailed,
        TextErrorCode::NumericOverflow => GpuTextErrorCode::NumericOverflow,
        TextErrorCode::ResourceLimit => GpuTextErrorCode::ResourceLimit,
        _ => GpuTextErrorCode::InvalidResource,
    };
    GpuTextError::new(code)
}

fn scaled_glyph_coordinate_bits(design_bits: i32, run: &GlyphRun) -> Result<i32, GpuTextError> {
    let numerator = i128::from(design_bits)
        .checked_mul(i128::from(run.font_size_bits()))
        .ok_or(GpuTextError::new(GpuTextErrorCode::NumericOverflow))?;
    let denominator = i128::from(64_i32)
        .checked_mul(i128::from(run.units_per_em()))
        .ok_or(GpuTextError::new(GpuTextErrorCode::NumericOverflow))?;
    let rounded = if numerator >= 0 {
        numerator
            .checked_add(denominator / 2)
            .ok_or(GpuTextError::new(GpuTextErrorCode::NumericOverflow))?
            / denominator
    } else {
        -((-numerator
            .checked_add(denominator / 2)
            .ok_or(GpuTextError::new(GpuTextErrorCode::NumericOverflow))?)
            / denominator)
    };
    i32::try_from(rounded).map_err(|_| GpuTextError::new(GpuTextErrorCode::NumericOverflow))
}

fn pixel_bits(value: i32) -> Result<i32, GpuTextError> {
    value
        .checked_mul(1 << 16)
        .ok_or(GpuTextError::new(GpuTextErrorCode::NumericOverflow))
}

fn pixel_bits_u32(value: u32) -> Result<i32, GpuTextError> {
    i32::try_from(value)
        .map_err(|_| GpuTextError::new(GpuTextErrorCode::NumericOverflow))
        .and_then(pixel_bits)
}
