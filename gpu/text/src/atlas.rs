use std::collections::HashMap;

use skia_core::{FontCollection, FontId, GlyphBitmap, GlyphBitmapFormat, GlyphId, TextLayout};
use skia_gpu::{GpuAtlasRect, GpuGlyphAtlas, GpuGlyphAtlasKey};
use skia_image::Image;

use crate::{TextAtlasEntry, TextGlyphKey, TextGpuError, TextGpuErrorCode, error::map_text_error};

/// Packed text atlas and the index needed to position layout glyphs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextAtlas {
    atlas: GpuGlyphAtlas,
    entries: HashMap<TextGlyphKey, TextAtlasEntry>,
}

impl TextAtlas {
    /// Borrows the generic GPU atlas image.
    pub const fn gpu_atlas(&self) -> &GpuGlyphAtlas {
        &self.atlas
    }

    /// Moves the generic GPU atlas out for command-buffer registration.
    pub fn into_gpu_atlas(self) -> GpuGlyphAtlas {
        self.atlas
    }

    /// Resolves one exact text glyph cache key.
    pub fn entry(&self, key: TextGlyphKey) -> Option<TextAtlasEntry> {
        self.entries.get(&key).copied()
    }

    pub(crate) fn glyph_entry(
        &self,
        font: FontId,
        glyph: GlyphId,
        font_size_bits: i32,
    ) -> Option<TextAtlasEntry> {
        [GlyphBitmapFormat::Alpha8, GlyphBitmapFormat::Rgba8]
            .into_iter()
            .find_map(|format| self.entry(TextGlyphKey::new(font, glyph, font_size_bits, format)))
    }

    pub(crate) fn with_cache_key(mut self, cache_key: GpuGlyphAtlasKey) -> Self {
        self.atlas = self.atlas.with_cache_key(cache_key);
        self
    }
}

/// Bounded shelf packer for reusable text glyph atlases.
#[derive(Debug)]
pub struct TextAtlasBuilder {
    width: u32,
    height: u32,
    max_glyphs: usize,
    pixels: Vec<u8>,
    entries: HashMap<TextGlyphKey, TextAtlasEntry>,
    cursor_x: u32,
    cursor_y: u32,
    row_height: u32,
}

impl TextAtlasBuilder {
    /// Allocates one transparent, bounded RGBA8 text atlas.
    pub fn new(width: u32, height: u32, max_glyphs: usize) -> Result<Self, TextGpuError> {
        if width == 0 || height == 0 || max_glyphs == 0 {
            return Err(TextGpuError::new(TextGpuErrorCode::InvalidLimits));
        }
        let length = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|value| value.checked_mul(4))
            .and_then(|value| usize::try_from(value).ok())
            .ok_or(TextGpuError::new(TextGpuErrorCode::ResourceLimit))?;
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(length)
            .map_err(|_| TextGpuError::new(TextGpuErrorCode::AllocationFailed))?;
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
    pub fn insert(&mut self, bitmap: &GlyphBitmap) -> Result<TextAtlasEntry, TextGpuError> {
        let key = TextGlyphKey::from_bitmap(bitmap);
        if let Some(entry) = self.entries.get(&key) {
            return Ok(*entry);
        }
        if self.entries.len() == self.max_glyphs || bitmap.width() == 0 || bitmap.height() == 0 {
            return Err(TextGpuError::new(TextGpuErrorCode::ResourceLimit));
        }
        let padded_width = bitmap
            .width()
            .checked_add(2)
            .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))?;
        let padded_height = bitmap
            .height()
            .checked_add(2)
            .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))?;
        if padded_width > self.width || padded_height > self.height {
            return Err(TextGpuError::new(TextGpuErrorCode::ResourceLimit));
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
                .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))?;
            self.row_height = 0;
        }
        if self
            .cursor_y
            .checked_add(bitmap.height())
            .and_then(|value| value.checked_add(1))
            .is_none_or(|value| value > self.height)
        {
            return Err(TextGpuError::new(TextGpuErrorCode::ResourceLimit));
        }
        let source = GpuAtlasRect::new(
            self.cursor_x,
            self.cursor_y,
            bitmap.width(),
            bitmap.height(),
        )?;
        self.copy_bitmap(source, bitmap)?;
        let entry = TextAtlasEntry {
            key,
            source,
            left: bitmap.left(),
            top: bitmap.top(),
        };
        self.entries
            .try_reserve(1)
            .map_err(|_| TextGpuError::new(TextGpuErrorCode::AllocationFailed))?;
        self.entries.insert(key, entry);
        self.cursor_x = self
            .cursor_x
            .checked_add(bitmap.width())
            .and_then(|value| value.checked_add(1))
            .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))?;
        self.row_height = self.row_height.max(bitmap.height());
        Ok(entry)
    }

    /// Rasterizes and inserts every drawable glyph referenced by a text layout.
    pub fn insert_layout(
        &mut self,
        layout: &TextLayout,
        fonts: &FontCollection,
    ) -> Result<(), TextGpuError> {
        for line in layout.lines() {
            let Some(paragraph) = line.paragraph() else {
                continue;
            };
            for shaped in paragraph.runs() {
                let run = shaped.glyph_run();
                let face = fonts
                    .face(run.font())
                    .ok_or(TextGpuError::new(TextGpuErrorCode::InvalidResource))?;
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
    pub fn finish(self) -> Result<TextAtlas, TextGpuError> {
        let image = Image::from_rgba8(self.width, self.height, self.pixels)
            .map_err(|_| TextGpuError::new(TextGpuErrorCode::InvalidResource))?;
        Ok(TextAtlas {
            atlas: GpuGlyphAtlas::from_image(image),
            entries: self.entries,
        })
    }

    fn copy_bitmap(
        &mut self,
        destination: GpuAtlasRect,
        bitmap: &GlyphBitmap,
    ) -> Result<(), TextGpuError> {
        let source_stride = usize::try_from(bitmap.width())
            .ok()
            .and_then(|value| value.checked_mul(bitmap.format().bytes_per_pixel()))
            .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))?;
        let destination_stride = usize::try_from(self.width)
            .ok()
            .and_then(|value| value.checked_mul(4))
            .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))?;
        let destination_row_bytes = usize::try_from(bitmap.width())
            .ok()
            .and_then(|value| value.checked_mul(4))
            .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))?;
        for row in 0..bitmap.height() {
            let source_start = usize::try_from(row)
                .ok()
                .and_then(|value| value.checked_mul(source_stride))
                .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))?;
            let source_end = source_start
                .checked_add(source_stride)
                .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))?;
            let destination_y = destination
                .y()
                .checked_add(row)
                .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))?;
            let destination_start = usize::try_from(destination_y)
                .ok()
                .and_then(|value| value.checked_mul(destination_stride))
                .and_then(|value| {
                    usize::try_from(destination.x())
                        .ok()
                        .and_then(|x| x.checked_mul(4))
                        .and_then(|x| value.checked_add(x))
                })
                .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))?;
            let destination_end = destination_start
                .checked_add(destination_row_bytes)
                .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))?;
            let source = bitmap
                .pixels()
                .get(source_start..source_end)
                .ok_or(TextGpuError::new(TextGpuErrorCode::InvalidResource))?;
            let destination = self
                .pixels
                .get_mut(destination_start..destination_end)
                .ok_or(TextGpuError::new(TextGpuErrorCode::InvalidResource))?;
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
