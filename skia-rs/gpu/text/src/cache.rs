use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use skia_core::{FontCollection, FontId, GlyphId, TextLayout};
use skia_gpu::GpuGlyphAtlasKey;

use crate::{TextAtlas, TextAtlasBuilder, TextGpuError, TextGpuErrorCode};

static NEXT_ATLAS_KEY: AtomicU64 = AtomicU64::new(1);

/// Dimensions and entry ceilings for one reusable text-atlas cache.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TextAtlasCacheLimits {
    atlas_width: u32,
    atlas_height: u32,
    max_glyphs_per_atlas: usize,
    max_atlases: usize,
}

impl TextAtlasCacheLimits {
    /// Creates positive per-atlas dimensions and cache entry ceilings.
    pub const fn new(
        atlas_width: u32,
        atlas_height: u32,
        max_glyphs_per_atlas: usize,
        max_atlases: usize,
    ) -> Result<Self, TextGpuError> {
        if atlas_width == 0 || atlas_height == 0 || max_glyphs_per_atlas == 0 || max_atlases == 0 {
            return Err(TextGpuError::new(TextGpuErrorCode::InvalidLimits));
        }
        Ok(Self {
            atlas_width,
            atlas_height,
            max_glyphs_per_atlas,
            max_atlases,
        })
    }

    /// Returns each atlas width in physical pixels.
    pub const fn atlas_width(self) -> u32 {
        self.atlas_width
    }

    /// Returns each atlas height in physical pixels.
    pub const fn atlas_height(self) -> u32 {
        self.atlas_height
    }

    /// Returns the maximum unique glyph requests packed into one atlas.
    pub const fn max_glyphs_per_atlas(self) -> usize {
        self.max_glyphs_per_atlas
    }

    /// Returns the maximum number of retained immutable atlases.
    pub const fn max_atlases(self) -> usize {
        self.max_atlases
    }
}

impl Default for TextAtlasCacheLimits {
    fn default() -> Self {
        Self {
            atlas_width: 1_024,
            atlas_height: 1_024,
            max_glyphs_per_atlas: 4_096,
            max_atlases: 4,
        }
    }
}

/// Observable counters for one text-atlas cache.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct TextAtlasCacheStats {
    hits: u64,
    misses: u64,
    evictions: u64,
    entries: usize,
}

impl TextAtlasCacheStats {
    /// Returns the number of layout lookups served by a retained atlas.
    pub const fn hits(self) -> u64 {
        self.hits
    }

    /// Returns the number of layout lookups that required a new atlas.
    pub const fn misses(self) -> u64 {
        self.misses
    }

    /// Returns the number of least-recently-used atlases removed.
    pub const fn evictions(self) -> u64 {
        self.evictions
    }

    /// Returns the current number of retained atlases.
    pub const fn entries(self) -> usize {
        self.entries
    }
}

/// Bounded least-recently-used cache of immutable packed text atlases.
///
/// A hit may reuse an atlas containing a superset of the requested glyphs.
/// Stable [`FontId`] values must continue to identify immutable font instances.
#[derive(Debug)]
pub struct TextAtlasCache {
    limits: TextAtlasCacheLimits,
    entries: Vec<CachedAtlas>,
    clock: u64,
    hits: u64,
    misses: u64,
    evictions: u64,
}

impl TextAtlasCache {
    /// Creates an empty cache with explicit resource ceilings.
    pub const fn new(limits: TextAtlasCacheLimits) -> Self {
        Self {
            limits,
            entries: Vec::new(),
            clock: 0,
            hits: 0,
            misses: 0,
            evictions: 0,
        }
    }

    /// Returns a reusable atlas covering every glyph requested by `layout`.
    pub fn get_or_insert_layout(
        &mut self,
        layout: &TextLayout,
        fonts: &FontCollection,
    ) -> Result<Arc<TextAtlas>, TextGpuError> {
        let requests = collect_requests(layout, self.limits.max_glyphs_per_atlas)?;
        let now = self.next_tick()?;
        if let Some(index) = self.entries.iter().position(|entry| {
            requests
                .iter()
                .all(|request| entry.requests.contains(request))
        }) {
            self.hits = self
                .hits
                .checked_add(1)
                .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))?;
            self.entries[index].last_used = now;
            return Ok(Arc::clone(&self.entries[index].atlas));
        }

        self.misses = self
            .misses
            .checked_add(1)
            .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))?;
        let mut builder = TextAtlasBuilder::new(
            self.limits.atlas_width,
            self.limits.atlas_height,
            self.limits.max_glyphs_per_atlas,
        )?;
        builder.insert_layout(layout, fonts)?;
        let cache_key = next_atlas_key()?;
        let atlas = Arc::new(builder.finish()?.with_cache_key(cache_key));

        if self.entries.len() == self.limits.max_atlases {
            let index = self
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(index, _)| index)
                .ok_or(TextGpuError::new(TextGpuErrorCode::InvalidResource))?;
            self.entries.remove(index);
            self.evictions = self
                .evictions
                .checked_add(1)
                .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))?;
        }
        self.entries
            .try_reserve(1)
            .map_err(|_| TextGpuError::new(TextGpuErrorCode::AllocationFailed))?;
        self.entries.push(CachedAtlas {
            requests,
            atlas: Arc::clone(&atlas),
            last_used: now,
        });
        Ok(atlas)
    }

    /// Returns hit, miss, eviction, and current-entry counters.
    pub const fn stats(&self) -> TextAtlasCacheStats {
        TextAtlasCacheStats {
            hits: self.hits,
            misses: self.misses,
            evictions: self.evictions,
            entries: self.entries.len(),
        }
    }

    /// Drops every retained atlas without resetting lifetime counters.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    fn next_tick(&mut self) -> Result<u64, TextGpuError> {
        self.clock = self
            .clock
            .checked_add(1)
            .ok_or(TextGpuError::new(TextGpuErrorCode::NumericOverflow))?;
        Ok(self.clock)
    }
}

#[derive(Debug)]
struct CachedAtlas {
    requests: HashSet<TextGlyphRequest>,
    atlas: Arc<TextAtlas>,
    last_used: u64,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct TextGlyphRequest {
    font: FontId,
    glyph: GlyphId,
    font_size_bits: i32,
}

fn collect_requests(
    layout: &TextLayout,
    max_glyphs: usize,
) -> Result<HashSet<TextGlyphRequest>, TextGpuError> {
    let mut requests = HashSet::new();
    for line in layout.lines() {
        let Some(paragraph) = line.paragraph() else {
            continue;
        };
        for shaped in paragraph.runs() {
            let run = shaped.glyph_run();
            for glyph in run.glyphs() {
                if requests.len() == max_glyphs
                    && !requests.contains(&TextGlyphRequest {
                        font: run.font(),
                        glyph: glyph.glyph(),
                        font_size_bits: run.font_size_bits(),
                    })
                {
                    return Err(TextGpuError::new(TextGpuErrorCode::ResourceLimit));
                }
                requests
                    .try_reserve(1)
                    .map_err(|_| TextGpuError::new(TextGpuErrorCode::AllocationFailed))?;
                requests.insert(TextGlyphRequest {
                    font: run.font(),
                    glyph: glyph.glyph(),
                    font_size_bits: run.font_size_bits(),
                });
            }
        }
    }
    Ok(requests)
}

fn next_atlas_key() -> Result<GpuGlyphAtlasKey, TextGpuError> {
    NEXT_ATLAS_KEY
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .map(GpuGlyphAtlasKey::new)
        .map_err(|_| TextGpuError::new(TextGpuErrorCode::NumericOverflow))
}
