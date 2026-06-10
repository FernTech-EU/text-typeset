mod helpers;
use helpers::make_typesetter;

use text_typeset::font::resolve::resolve_font;
use text_typeset::shaping::shaper::shape_text;

// ── Atlas allocator tests ───────────────────────────────────────

mod allocator {
    use text_typeset::atlas::allocator::GlyphAtlas;

    #[test]
    fn new_atlas_has_correct_dimensions() {
        let atlas = GlyphAtlas::new();
        assert_eq!(atlas.width, 512);
        assert_eq!(atlas.height, 512);
        assert_eq!(atlas.pixels.len(), 512 * 512 * 4);
        assert!(!atlas.dirty);
    }

    #[test]
    fn allocate_small_rect_succeeds() {
        let mut atlas = GlyphAtlas::new();
        let alloc = atlas.allocate(16, 20);
        assert!(alloc.is_some());
        let alloc = alloc.unwrap();
        let rect = alloc.rectangle;
        assert!(rect.min.x >= 0);
        assert!(rect.min.y >= 0);
        // BucketedAtlasAllocator may round up to bucket sizes
        assert!((rect.max.x - rect.min.x) as u32 >= 16);
        assert!((rect.max.y - rect.min.y) as u32 >= 20);
    }

    #[test]
    fn allocate_multiple_rects_dont_overlap() {
        let mut atlas = GlyphAtlas::new();
        let a1 = atlas.allocate(32, 32).unwrap();
        let a2 = atlas.allocate(32, 32).unwrap();

        let r1 = a1.rectangle;
        let r2 = a2.rectangle;

        // Rectangles should not overlap
        let overlap_x = r1.min.x < r2.max.x && r2.min.x < r1.max.x;
        let overlap_y = r1.min.y < r2.max.y && r2.min.y < r1.max.y;
        assert!(
            !(overlap_x && overlap_y),
            "allocations overlap: {:?} and {:?}",
            r1,
            r2
        );
    }

    #[test]
    fn blit_mask_writes_rgba_pixels() {
        let mut atlas = GlyphAtlas::new();
        let alloc = atlas.allocate(2, 2).unwrap();
        let x = alloc.rectangle.min.x as u32;
        let y = alloc.rectangle.min.y as u32;

        // Blit a 2x2 alpha mask: [128, 255, 0, 64]
        atlas.blit_mask(x, y, 2, 2, &[128, 255, 0, 64]);

        // Check first pixel: should be [255, 255, 255, 128]
        let offset = ((y * atlas.width + x) * 4) as usize;
        assert_eq!(atlas.pixels[offset], 255); // R
        assert_eq!(atlas.pixels[offset + 1], 255); // G
        assert_eq!(atlas.pixels[offset + 2], 255); // B
        assert_eq!(atlas.pixels[offset + 3], 128); // A
        assert!(atlas.dirty);
    }

    #[test]
    fn blit_rgba_writes_color_pixels() {
        let mut atlas = GlyphAtlas::new();
        let alloc = atlas.allocate(1, 1).unwrap();
        let x = alloc.rectangle.min.x as u32;
        let y = alloc.rectangle.min.y as u32;

        atlas.blit_rgba(x, y, 1, 1, &[10, 20, 30, 40]);

        let offset = ((y * atlas.width + x) * 4) as usize;
        assert_eq!(atlas.pixels[offset], 10);
        assert_eq!(atlas.pixels[offset + 1], 20);
        assert_eq!(atlas.pixels[offset + 2], 30);
        assert_eq!(atlas.pixels[offset + 3], 40);
    }

    #[test]
    fn allocate_triggers_grow_when_full() {
        let mut atlas = GlyphAtlas::new(); // 512x512

        // Fill the atlas with large blocks until allocation would fail without growing
        let mut count = 0;
        loop {
            if atlas.allocate(128, 128).is_none() {
                break;
            }
            count += 1;
            if count > 200 {
                // Safety limit — should never need this many 128x128 in 512x512
                break;
            }
        }
        // After growing, the atlas should be larger
        assert!(
            atlas.width > 512 || atlas.height > 512,
            "atlas should have grown: {}x{}",
            atlas.width,
            atlas.height
        );
    }

    #[test]
    fn deallocate_frees_space() {
        let mut atlas = GlyphAtlas::new();
        let alloc = atlas.allocate(256, 256).unwrap();
        let id = alloc.id;
        let space_before = atlas.allocator.free_space();
        atlas.deallocate(id);
        let space_after = atlas.allocator.free_space();
        assert!(
            space_after >= space_before,
            "free space should not decrease after deallocation"
        );
    }

    #[test]
    fn poison_fill_marks_atlas_dirty_and_writes_magenta() {
        let mut atlas = GlyphAtlas::new();
        atlas.dirty = false;

        atlas.debug_poison_rect(2, 3, 4, 4);

        assert!(atlas.dirty, "poison fill must mark the atlas dirty");
        // Inside the rect: solid magenta.
        for (x, y) in [(2u32, 3u32), (5, 3), (2, 6), (5, 6)] {
            let i = ((y * atlas.width + x) * 4) as usize;
            assert_eq!(
                &atlas.pixels[i..i + 4],
                &[255, 0, 255, 255],
                "pixel ({x},{y}) inside the poisoned rect must be magenta"
            );
        }
        // Outside the rect: untouched (zeroed).
        for (x, y) in [(1u32, 3u32), (6, 3), (2, 2), (2, 7)] {
            let i = ((y * atlas.width + x) * 4) as usize;
            assert_eq!(
                &atlas.pixels[i..i + 4],
                &[0, 0, 0, 0],
                "pixel ({x},{y}) outside the poisoned rect must be untouched"
            );
        }
    }

    #[test]
    fn poison_fill_clamps_to_atlas_bounds() {
        let mut atlas = GlyphAtlas::new();
        let (w, h) = (atlas.width, atlas.height);
        // Rect extending past both edges must not panic.
        atlas.debug_poison_rect(w - 2, h - 2, 10, 10);
        let i = (((h - 1) * w + (w - 1)) * 4) as usize;
        assert_eq!(&atlas.pixels[i..i + 4], &[255, 0, 255, 255]);
        // Fully out-of-bounds rect is a no-op.
        let mut atlas2 = GlyphAtlas::new();
        atlas2.dirty = false;
        atlas2.debug_poison_rect(w + 5, h + 5, 4, 4);
        assert!(!atlas2.dirty);
    }
}

// ── Rasterizer tests ────────────────────────────────────────────

mod rasterizer {
    use super::*;
    use text_typeset::atlas::rasterizer::rasterize_glyph;

    #[test]
    fn rasterize_letter_a_produces_image() {
        let ts = make_typesetter();
        let resolved = resolve_font(ts.font_registry(), None, None, None, None, None, 1.0).unwrap();
        let entry = ts.font_registry().get(resolved.font_face_id).unwrap();

        // Shape 'A' to get its glyph ID
        let run = shape_text(ts.font_registry(), &resolved, "A", 0).unwrap();
        let glyph_id = run.glyphs[0].glyph_id;

        let mut scale_ctx = swash::scale::ScaleContext::new();
        let image = rasterize_glyph(
            &mut scale_ctx,
            entry.bytes(),
            entry.face_index,
            entry.swash_cache_key,
            glyph_id,
            16.0,
            400,
            true,
        );

        assert!(image.is_some(), "rasterization should succeed for 'A'");
        let image = image.unwrap();
        assert!(image.width > 0, "rasterized glyph should have width > 0");
        assert!(image.height > 0, "rasterized glyph should have height > 0");
        assert!(!image.data.is_empty(), "pixel data should not be empty");
    }

    #[test]
    fn rasterized_glyph_has_nonzero_pixels() {
        let ts = make_typesetter();
        let resolved = resolve_font(ts.font_registry(), None, None, None, None, None, 1.0).unwrap();
        let entry = ts.font_registry().get(resolved.font_face_id).unwrap();

        let run = shape_text(ts.font_registry(), &resolved, "A", 0).unwrap();
        let glyph_id = run.glyphs[0].glyph_id;

        let mut scale_ctx = swash::scale::ScaleContext::new();
        let image = rasterize_glyph(
            &mut scale_ctx,
            entry.bytes(),
            entry.face_index,
            entry.swash_cache_key,
            glyph_id,
            24.0,
            400,
            true,
        )
        .unwrap();

        // At least some pixels should be non-zero (the glyph is not blank)
        let has_nonzero = image.data.iter().any(|&b| b > 0);
        assert!(
            has_nonzero,
            "rasterized 'A' should have non-zero pixel data"
        );
    }

    #[test]
    fn larger_size_produces_larger_glyph() {
        let ts = make_typesetter();
        let resolved_small =
            resolve_font(ts.font_registry(), None, None, None, None, Some(12), 1.0).unwrap();
        let _resolved_large =
            resolve_font(ts.font_registry(), None, None, None, None, Some(48), 1.0).unwrap();
        let entry = ts.font_registry().get(resolved_small.font_face_id).unwrap();

        let run = shape_text(ts.font_registry(), &resolved_small, "M", 0).unwrap();
        let glyph_id = run.glyphs[0].glyph_id;

        let mut scale_ctx = swash::scale::ScaleContext::new();
        let small = rasterize_glyph(
            &mut scale_ctx,
            entry.bytes(),
            entry.face_index,
            entry.swash_cache_key,
            glyph_id,
            12.0,
            400,
            true,
        )
        .unwrap();
        let large = rasterize_glyph(
            &mut scale_ctx,
            entry.bytes(),
            entry.face_index,
            entry.swash_cache_key,
            glyph_id,
            48.0,
            400,
            true,
        )
        .unwrap();

        assert!(
            large.width > small.width && large.height > small.height,
            "48px glyph ({}x{}) should be larger than 12px glyph ({}x{})",
            large.width,
            large.height,
            small.width,
            small.height
        );
    }

    #[test]
    fn space_glyph_rasterizes_to_empty_or_none() {
        let ts = make_typesetter();
        let resolved = resolve_font(ts.font_registry(), None, None, None, None, None, 1.0).unwrap();
        let entry = ts.font_registry().get(resolved.font_face_id).unwrap();

        let run = shape_text(ts.font_registry(), &resolved, " ", 0).unwrap();
        let glyph_id = run.glyphs[0].glyph_id;

        let mut scale_ctx = swash::scale::ScaleContext::new();
        let image = rasterize_glyph(
            &mut scale_ctx,
            entry.bytes(),
            entry.face_index,
            entry.swash_cache_key,
            glyph_id,
            16.0,
            400,
            true,
        );

        // Space may rasterize to None (no outline) or to an empty image
        if let Some(img) = image {
            // If it does rasterize, it should have zero or very few pixels
            assert!(
                img.width * img.height <= 4,
                "space glyph should be tiny or empty, got {}x{}",
                img.width,
                img.height
            );
        }
        // None is also acceptable — space has no visible outline
    }
}

// ── Glyph cache tests ──────────────────────────────────────────

mod cache {
    use text_typeset::FontFaceId;
    use text_typeset::atlas::cache::{CachedGlyph, GlyphCache, GlyphCacheKey};

    #[test]
    fn cache_miss_returns_none() {
        let mut cache = GlyphCache::new();
        let key = GlyphCacheKey::new(FontFaceId(0), 42, 16.0);
        assert!(cache.get(&key).is_none());
    }

    #[test]
    fn cache_insert_then_get() {
        let mut cache = GlyphCache::new();
        let key = GlyphCacheKey::new(FontFaceId(0), 42, 16.0);
        cache.insert(
            key,
            CachedGlyph {
                alloc_id: etagere::AllocId::deserialize(1),
                atlas_x: 10,
                atlas_y: 20,
                width: 8,
                height: 12,
                placement_left: 1,
                placement_top: 10,
                is_color: false,
                last_used: 0,
            },
        );

        let entry = cache.get(&key);
        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert_eq!(entry.atlas_x, 10);
        assert_eq!(entry.atlas_y, 20);
        assert_eq!(entry.width, 8);
        assert_eq!(entry.height, 12);
    }

    #[test]
    fn different_sizes_are_different_keys() {
        let mut cache = GlyphCache::new();
        let key_16 = GlyphCacheKey::new(FontFaceId(0), 42, 16.0);
        let key_24 = GlyphCacheKey::new(FontFaceId(0), 42, 24.0);

        cache.insert(
            key_16,
            CachedGlyph {
                alloc_id: etagere::AllocId::deserialize(1),
                atlas_x: 0,
                atlas_y: 0,
                width: 8,
                height: 12,
                placement_left: 0,
                placement_top: 0,
                is_color: false,
                last_used: 0,
            },
        );

        assert!(cache.get(&key_16).is_some());
        assert!(cache.get(&key_24).is_none());
    }

    #[test]
    fn pressure_eviction_recovers_space_for_new_allocations() {
        use text_typeset::atlas::allocate_or_evict;
        use text_typeset::atlas::allocator::GlyphAtlas;

        // Fill the atlas to its size cap with 500x500 slots (the atlas
        // grows to 4096² on demand, then allocation fails).
        let mut atlas = GlyphAtlas::new();
        let mut cache = GlyphCache::new();
        let mut glyph_id: u16 = 0;
        let mut keys = Vec::new();
        while let Some(alloc) = atlas.allocate(500, 500) {
            let key = GlyphCacheKey::new(FontFaceId(0), glyph_id, 16.0);
            let rect = alloc.rectangle;
            cache.insert(
                key,
                CachedGlyph {
                    alloc_id: alloc.id,
                    atlas_x: rect.min.x as u32,
                    atlas_y: rect.min.y as u32,
                    width: 500,
                    height: 500,
                    placement_left: 0,
                    placement_top: 0,
                    is_color: false,
                    last_used: 0,
                },
            );
            keys.push(key);
            glyph_id += 1;
        }
        assert!(keys.len() > 8, "expected the atlas to hold many slots");
        assert!(atlas.allocate(500, 500).is_none(), "atlas must be full");

        // New frame: only the first two glyphs are still in use.
        cache.advance_generation();
        cache.touch(&keys[..2]);

        let (alloc, evicted) = allocate_or_evict(&mut atlas, &mut cache, 500, 500);
        assert!(evicted, "pressure eviction must fire on a full atlas");
        assert!(
            alloc.is_some(),
            "the freed space must satisfy the failed allocation"
        );
        assert_eq!(
            cache.len(),
            2,
            "glyphs touched this generation must survive pressure eviction"
        );
        assert!(cache.peek(&keys[0]).is_some());
        assert!(cache.peek(&keys[1]).is_some());
        assert!(cache.peek(&keys[2]).is_none());
    }

    #[test]
    fn pressure_eviction_spares_full_atlas_of_current_glyphs() {
        use text_typeset::atlas::allocate_or_evict;
        use text_typeset::atlas::allocator::GlyphAtlas;

        // Same fill, but every glyph is in use this generation — nothing
        // may be evicted and the allocation still fails (caller falls
        // back to dropping the glyph, as before).
        let mut atlas = GlyphAtlas::new();
        let mut cache = GlyphCache::new();
        let mut glyph_id: u16 = 0;
        let mut keys = Vec::new();
        while let Some(alloc) = atlas.allocate(500, 500) {
            let key = GlyphCacheKey::new(FontFaceId(0), glyph_id, 16.0);
            let rect = alloc.rectangle;
            cache.insert(
                key,
                CachedGlyph {
                    alloc_id: alloc.id,
                    atlas_x: rect.min.x as u32,
                    atlas_y: rect.min.y as u32,
                    width: 500,
                    height: 500,
                    placement_left: 0,
                    placement_top: 0,
                    is_color: false,
                    last_used: 0,
                },
            );
            keys.push(key);
            glyph_id += 1;
        }
        let total = keys.len();
        cache.touch(&keys); // everything used in the current generation

        let (alloc, evicted) = allocate_or_evict(&mut atlas, &mut cache, 500, 500);
        assert!(!evicted, "in-use glyphs must never be pressure-evicted");
        assert!(alloc.is_none());
        assert_eq!(cache.len(), total);
    }

    #[test]
    fn evict_unused_removes_stale_glyphs() {
        let mut cache = GlyphCache::new();
        let key = GlyphCacheKey::new(FontFaceId(0), 42, 16.0);
        cache.insert(
            key,
            CachedGlyph {
                alloc_id: etagere::AllocId::deserialize(1),
                atlas_x: 0,
                atlas_y: 0,
                width: 8,
                height: 12,
                placement_left: 0,
                placement_top: 0,
                is_color: false,
                last_used: 0,
            },
        );
        assert_eq!(cache.len(), 1);

        // Advance 200 generations without accessing the glyph
        for _ in 0..200 {
            cache.advance_generation();
        }

        let evicted = cache.evict_unused();
        assert_eq!(evicted.len(), 1, "should evict one stale glyph");
        assert_eq!(cache.len(), 0, "cache should be empty after eviction");
    }

    #[test]
    fn evict_unused_returns_evicted_rects() {
        let mut cache = GlyphCache::new();
        let key = GlyphCacheKey::new(FontFaceId(0), 7, 16.0);
        cache.insert(
            key,
            CachedGlyph {
                alloc_id: etagere::AllocId::deserialize(3),
                atlas_x: 10,
                atlas_y: 20,
                width: 8,
                height: 12,
                placement_left: 0,
                placement_top: 0,
                is_color: false,
                last_used: 0,
            },
        );

        for _ in 0..200 {
            cache.advance_generation();
        }

        let evicted = cache.evict_unused();
        assert_eq!(evicted.len(), 1);
        let glyph = &evicted[0];
        assert_eq!(glyph.alloc_id, etagere::AllocId::deserialize(3));
        assert_eq!(
            (glyph.atlas_x, glyph.atlas_y, glyph.width, glyph.height),
            (10, 20, 8, 12),
            "evicted entry must carry the atlas rect it occupied (for debug poison fill)"
        );
    }

    #[test]
    fn recently_used_glyphs_not_evicted() {
        let mut cache = GlyphCache::new();
        let key = GlyphCacheKey::new(FontFaceId(0), 42, 16.0);
        cache.insert(
            key,
            CachedGlyph {
                alloc_id: etagere::AllocId::deserialize(1),
                atlas_x: 0,
                atlas_y: 0,
                width: 8,
                height: 12,
                placement_left: 0,
                placement_top: 0,
                is_color: false,
                last_used: 0,
            },
        );

        // Advance 50 generations but keep using the glyph
        for _ in 0..50 {
            cache.advance_generation();
            let _ = cache.get(&key); // marks as used
        }

        let evicted = cache.evict_unused();
        assert!(
            evicted.is_empty(),
            "recently used glyphs should not be evicted"
        );
        assert_eq!(cache.len(), 1);
    }
}
