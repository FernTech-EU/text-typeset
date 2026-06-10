use std::collections::HashMap;

use etagere::AllocId;

use crate::types::FontFaceId;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct GlyphCacheKey {
    pub font_face_id: FontFaceId,
    pub glyph_id: u16,
    pub size_bits: u32,
    /// Font weight (variation axis) so that e.g. Inter Regular and Inter
    /// Bold produce separate cache entries even though they share the
    /// same `font_face_id` and `glyph_id` in a variable font.
    pub weight: u32,
    /// Whether the bitmap was rasterized with hinting. Rasters produced
    /// under a raster scale (zoomed content) are unhinted, and the same
    /// *physical* size can be reached both ways — 7px at raster_scale 2
    /// and 14px at raster_scale 1 share `size_bits` but must not share
    /// a bitmap.
    pub hinted: bool,
}

impl GlyphCacheKey {
    pub fn new(font_face_id: FontFaceId, glyph_id: u16, size_px: f32) -> Self {
        Self {
            font_face_id,
            glyph_id,
            size_bits: size_px.to_bits(),
            weight: 400,
            hinted: true,
        }
    }

    pub fn with_weight(
        font_face_id: FontFaceId,
        glyph_id: u16,
        size_px: f32,
        weight: u32,
        hinted: bool,
    ) -> Self {
        Self {
            font_face_id,
            glyph_id,
            size_bits: size_px.to_bits(),
            weight,
            hinted,
        }
    }
}

pub struct CachedGlyph {
    pub alloc_id: AllocId,
    pub atlas_x: u32,
    pub atlas_y: u32,
    pub width: u32,
    pub height: u32,
    pub placement_left: i32,
    pub placement_top: i32,
    pub is_color: bool,
    /// Frame generation when this glyph was last used.
    pub last_used: u64,
}

/// A glyph removed by [`GlyphCache::evict_unused`]: the allocator id to
/// deallocate plus the atlas rectangle the glyph occupied. Consumers that
/// retain glyph quads across frames (paint caches) key their invalidation
/// off the eviction; the rect lets debug builds poison-fill freed pixels
/// so stale-UV sampling is visually unmistakable.
#[derive(Clone, Copy, Debug)]
pub struct EvictedGlyph {
    pub alloc_id: AllocId,
    pub atlas_x: u32,
    pub atlas_y: u32,
    pub width: u32,
    pub height: u32,
}

/// Glyph cache with LRU eviction.
///
/// Tracks a frame generation counter. Each `get` marks the glyph as used
/// in the current generation. `evict_unused` removes glyphs not used
/// for `max_idle_frames` generations and deallocates their atlas space.
pub struct GlyphCache {
    pub(crate) entries: HashMap<GlyphCacheKey, CachedGlyph>,
    generation: u64,
    last_eviction_generation: u64,
}

/// Number of frames a glyph can go unused before being evicted.
const MAX_IDLE_FRAMES: u64 = 120; // ~2 seconds at 60fps

impl Default for GlyphCache {
    fn default() -> Self {
        Self::new()
    }
}

impl GlyphCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            generation: 0,
            last_eviction_generation: 0,
        }
    }

    /// Advance the frame generation counter. Call once per render frame.
    pub fn advance_generation(&mut self) {
        self.generation += 1;
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Look up a cached glyph, marking it as used in the current generation.
    pub fn get(&mut self, key: &GlyphCacheKey) -> Option<&CachedGlyph> {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.last_used = self.generation;
            Some(entry)
        } else {
            None
        }
    }

    /// Look up without marking as used (for read-only queries).
    pub fn peek(&self, key: &GlyphCacheKey) -> Option<&CachedGlyph> {
        self.entries.get(key)
    }

    pub fn insert(&mut self, key: GlyphCacheKey, mut glyph: CachedGlyph) {
        glyph.last_used = self.generation;
        self.entries.insert(key, glyph);
    }

    /// Evict glyphs unused for MAX_IDLE_FRAMES generations.
    /// Returns the evicted entries — each carries the `AllocId` to
    /// deallocate from the atlas plus the atlas rectangle it occupied
    /// (the bucketed allocator cannot resolve a rectangle from an
    /// `AllocId` after the fact, so the rect is captured here, before
    /// the entry is dropped; debug builds use it to poison-fill the
    /// freed pixels).
    /// Only runs the actual eviction scan every 60 calls (~1 second at 60fps)
    /// to avoid iterating the entire cache on every render.
    pub fn evict_unused(&mut self) -> Vec<EvictedGlyph> {
        // Only scan every 60 generations (~1 second at 60fps)
        if self.generation - self.last_eviction_generation < 60 {
            return Vec::new();
        }
        self.last_eviction_generation = self.generation;

        let threshold = self.generation.saturating_sub(MAX_IDLE_FRAMES);
        let mut evicted = Vec::new();

        self.entries.retain(|_key, glyph| {
            if glyph.last_used < threshold {
                evicted.push(EvictedGlyph {
                    alloc_id: glyph.alloc_id,
                    atlas_x: glyph.atlas_x,
                    atlas_y: glyph.atlas_y,
                    width: glyph.width,
                    height: glyph.height,
                });
                false
            } else {
                true
            }
        });

        evicted
    }

    /// Emergency eviction when the atlas is full: drop every glyph not
    /// used in the *current* generation, regardless of the idle window.
    ///
    /// Unlike [`evict_unused`](Self::evict_unused) this runs
    /// unconditionally (no every-60-generations gate) — it is only
    /// called when an allocation has already failed at the atlas size
    /// cap, where the alternative is silently dropping the new glyph.
    /// Glyphs touched this generation are still referenced by quads in
    /// the frame being built and must survive.
    pub fn evict_for_pressure(&mut self) -> Vec<EvictedGlyph> {
        let current = self.generation;
        let mut evicted = Vec::new();

        self.entries.retain(|_key, glyph| {
            if glyph.last_used < current {
                evicted.push(EvictedGlyph {
                    alloc_id: glyph.alloc_id,
                    atlas_x: glyph.atlas_x,
                    atlas_y: glyph.atlas_y,
                    width: glyph.width,
                    height: glyph.height,
                });
                false
            } else {
                true
            }
        });

        evicted
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Mark multiple glyphs as used in the current generation without
    /// returning their data. Used by callers that cache glyph output
    /// externally (e.g. per-widget paint caches) and need to keep the
    /// glyphs alive in the atlas even though they don't re-measure them
    /// every frame.
    pub fn touch(&mut self, keys: &[GlyphCacheKey]) {
        let current = self.generation;
        for key in keys {
            if let Some(entry) = self.entries.get_mut(key) {
                entry.last_used = current;
            }
        }
    }
}
