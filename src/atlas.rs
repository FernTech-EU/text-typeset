pub mod allocator;
pub mod cache;
pub mod rasterizer;

use allocator::GlyphAtlas;
use cache::GlyphCache;
use etagere::Allocation;

/// Allocate an atlas slot, falling back to emergency eviction when the
/// atlas is full at its size cap.
///
/// The normal [`GlyphAtlas::allocate`] path grows the atlas up to its
/// maximum and then fails — historically the glyph was silently dropped.
/// Under a raster scale (zoomed content rasterizing at up to 4× ppem)
/// that failure mode becomes reachable in practice, so on failure this
/// evicts every cached glyph not used in the current generation
/// ([`GlyphCache::evict_for_pressure`]), deallocates their slots
/// (poisoning them in debug builds, same as idle eviction), and retries
/// once.
///
/// Returns the allocation (still `None` if even the freed space can't
/// fit the glyph) and whether any eviction happened — the caller must
/// bump the service's `eviction_epoch` when it did, so externally
/// retained quads (paint caches) invalidate.
pub fn allocate_or_evict(
    atlas: &mut GlyphAtlas,
    cache: &mut GlyphCache,
    width: u32,
    height: u32,
) -> (Option<Allocation>, bool) {
    if let Some(alloc) = atlas.allocate(width, height) {
        return (Some(alloc), false);
    }

    let evicted = cache.evict_for_pressure();
    if evicted.is_empty() {
        return (None, false);
    }
    for glyph in &evicted {
        atlas.deallocate(glyph.alloc_id);
        #[cfg(debug_assertions)]
        atlas.debug_poison_rect(glyph.atlas_x, glyph.atlas_y, glyph.width, glyph.height);
    }

    (atlas.allocate(width, height), true)
}
