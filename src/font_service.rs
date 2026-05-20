//! Shared font service: the part of text-typeset that can be shared
//! across many widgets viewing many documents.
//!
//! A [`TextFontService`] owns four things:
//!
//! - a font registry (parsed faces, family lookup, fallback chain),
//! - a GPU-bound glyph atlas (RGBA texture with bucketed allocations),
//! - a glyph cache keyed on `(face, glyph_id, physical_size_px)`,
//! - a `swash` scale context (reusable rasterizer workspace).
//!
//! None of these describe **what** a specific widget is showing — they
//! describe **how** glyphs are rasterized and cached. Two widgets
//! viewing the same or different documents in the same window should
//! share one `TextFontService` so that:
//!
//! 1. every glyph for a given `(face, size)` lives in one atlas
//!    and is uploaded to the GPU exactly once per frame;
//! 2. every shaped glyph rasterization happens at most once,
//!    amortized over every widget that ever renders it;
//! 3. font registration and fallback resolution are consistent
//!    across the whole UI.
//!
//! Per-widget state — viewport, zoom, scroll offset, flow layout,
//! cursor, colors — lives on a separate [`DocumentFlow`] that borrows
//! the service at layout and render time.
//!
//! [`DocumentFlow`]: crate::DocumentFlow
//!
//! # HiDPI invalidation
//!
//! [`set_scale_factor`](TextFontService::set_scale_factor) is the one
//! mutation that invalidates existing work: cached glyphs were
//! rasterized at the old physical ppem and are wrong at the new one,
//! and flow layouts stored shaped advances that depended on the
//! previous ppem rounding. The service clears its own glyph cache
//! and atlas on the spot, but it cannot reach into per-widget
//! [`DocumentFlow`] instances. Instead it bumps a monotonic
//! `scale_generation` counter every time the scale factor changes.
//! Each `DocumentFlow` remembers the generation it was last laid out
//! at. Call [`crate::DocumentFlow::layout_dirty_for_scale`] from the
//! framework side to ask "does this flow need a relayout?" and re-run
//! `layout_full` when the answer is yes.

use crate::atlas::allocator::GlyphAtlas;
use crate::atlas::cache::GlyphCache;
use crate::font::registry::FontRegistry;
use crate::font::resolve::resolve_font;
use crate::shaping::shaper::font_metrics_px;
use crate::types::{FontFaceId, TextFormat};

/// Outcome of a call to [`TextFontService::atlas_snapshot`].
///
/// Bundles the four signals a framework adapter needs to upload
/// (or skip uploading) the glyph atlas texture to the GPU:
///
/// - whether the atlas has pending pixel changes since the last
///   snapshot,
/// - its current pixel dimensions,
/// - a borrow of the raw RGBA pixel buffer, and
/// - whether any cached glyphs were evicted during this snapshot —
///   a signal that callers with paint caches holding old atlas
///   UVs must treat as an invalidation even if the atlas itself
///   reports clean afterwards (evicted slots may be reused by
///   future allocations, so any cached UV pointing into them is
///   stale).
///
/// Exposed as a struct rather than a tuple so every caller names
/// fields explicitly and can't swap positions silently.
#[derive(Debug)]
pub struct AtlasSnapshot<'a> {
    /// True if the atlas texture has pending pixel changes
    /// since it was last marked clean. The snapshot call clears
    /// this flag, so the caller must either upload `pixels` now
    /// or accept a one-frame delay.
    pub dirty: bool,
    /// Current atlas texture width in pixels.
    pub width: u32,
    /// Current atlas texture height in pixels.
    pub height: u32,
    /// Raw RGBA8 pixel buffer backing the atlas texture.
    pub pixels: &'a [u8],
    /// True if eviction freed at least one glyph slot during
    /// this snapshot. Callers that cache glyph positions (e.g.
    /// framework paint caches indexed by layout key) must
    /// invalidate when this is true — evicted slots may be
    /// reused by future allocations and old UVs would then
    /// point to the wrong glyph.
    pub glyphs_evicted: bool,
}

/// Shared font resources for a text-typeset session.
///
/// Owns the font registry, the glyph atlas, the glyph cache, and the
/// `swash` scale context. Construct one per process (or one per window
/// if you really need isolated atlases) and share it by `Rc<RefCell<_>>`
/// across every [`DocumentFlow`] that renders into the same atlas.
///
/// [`DocumentFlow`]: crate::DocumentFlow
pub struct TextFontService {
    pub(crate) font_registry: FontRegistry,
    pub(crate) atlas: GlyphAtlas,
    pub(crate) glyph_cache: GlyphCache,
    pub(crate) scale_context: swash::scale::ScaleContext,
    pub(crate) scale_factor: f32,
    /// Bumps every time [`set_scale_factor`](Self::set_scale_factor)
    /// actually changes the value. `DocumentFlow` snapshots this on
    /// layout and exposes a dirty check for callers.
    pub(crate) scale_generation: u64,
}

impl TextFontService {
    /// Create an empty service with no fonts registered.
    ///
    /// Call [`register_font`](Self::register_font) and
    /// [`set_default_font`](Self::set_default_font) before any
    /// [`DocumentFlow`] lays out content against this service.
    ///
    /// [`DocumentFlow`]: crate::DocumentFlow
    pub fn new() -> Self {
        Self {
            font_registry: FontRegistry::new(),
            atlas: GlyphAtlas::new(),
            glyph_cache: GlyphCache::new(),
            scale_context: swash::scale::ScaleContext::new(),
            scale_factor: 1.0,
            scale_generation: 0,
        }
    }

    // ── Font registration ───────────────────────────────────────

    /// Register a font face from raw TTF/OTF/WOFF bytes.
    ///
    /// Parses the font's name table to extract family, weight, and
    /// style, then indexes it via `fontdb` for CSS-spec font matching.
    /// Returns the first face ID — font collections (`.ttc`) may
    /// contain multiple faces.
    ///
    /// # Panics
    ///
    /// Panics if the font data contains no parseable faces.
    pub fn register_font(&mut self, data: &[u8]) -> FontFaceId {
        let ids = self.font_registry.register_font(data);
        ids.into_iter()
            .next()
            .expect("font data contained no faces")
    }

    /// Register a font from a pre-built shared byte container,
    /// avoiding the copy that [`register_font`](Self::register_font)
    /// would perform.
    ///
    /// Pass an `Arc<Mmap>` (or any `Arc<dyn AsRef<[u8]> + Sync + Send>`)
    /// when the caller already holds the data in a shareable form —
    /// useful for large system fonts (color emoji) where copying to an
    /// owned `Vec<u8>` would double the resident memory cost.
    ///
    /// # Panics
    ///
    /// Panics if the font data contains no parseable faces.
    pub fn register_font_shared(&mut self, data: crate::font::SharedFontData) -> FontFaceId {
        let ids = self.font_registry.register_font_shared(data);
        ids.into_iter()
            .next()
            .expect("font data contained no faces")
    }

    /// Register a font with explicit metadata, overriding the font's
    /// name table. Use when the font's internal metadata is unreliable
    /// or when aliasing a font to a different family name.
    ///
    /// # Panics
    ///
    /// Panics if the font data contains no parseable faces.
    pub fn register_font_as(
        &mut self,
        data: &[u8],
        family: &str,
        weight: u16,
        italic: bool,
    ) -> FontFaceId {
        let ids = self
            .font_registry
            .register_font_as(data, family, weight, italic);
        ids.into_iter()
            .next()
            .expect("font data contained no faces")
    }

    /// Like [`register_font_as`](Self::register_font_as) but takes a
    /// pre-built shared byte container, avoiding the copy.
    ///
    /// # Panics
    ///
    /// Panics if the font data contains no parseable faces.
    pub fn register_font_shared_as(
        &mut self,
        data: crate::font::SharedFontData,
        family: &str,
        weight: u16,
        italic: bool,
    ) -> FontFaceId {
        let ids = self
            .font_registry
            .register_font_shared_as(data, family, weight, italic);
        ids.into_iter()
            .next()
            .expect("font data contained no faces")
    }

    /// Set which face to use as the document default, plus its base
    /// size in logical pixels. This is the fallback font when a
    /// fragment's `TextFormat` doesn't specify a family or when the
    /// requested family isn't found.
    pub fn set_default_font(&mut self, face: FontFaceId, size_px: f32) {
        self.font_registry.set_default_font(face, size_px);
    }

    /// Map a generic family name (e.g. `"serif"`, `"monospace"`) to a
    /// concrete registered family. When text-document emits a fragment
    /// whose `font_family` matches a generic, the font resolver looks
    /// it up through this table before querying fontdb.
    pub fn set_generic_family(&mut self, generic: &str, family: &str) {
        self.font_registry.set_generic_family(generic, family);
    }

    /// Look up the family name of a registered face by id.
    pub fn font_family_name(&self, face_id: FontFaceId) -> Option<String> {
        self.font_registry.font_family_name(face_id)
    }

    /// Borrow the font registry directly — needed by callers that
    /// want to inspect or extend it beyond the helpers exposed here.
    pub fn font_registry(&self) -> &FontRegistry {
        &self.font_registry
    }

    // ── Font metrics ────────────────────────────────────────────

    /// Line height (in logical pixels) for the registry's default
    /// font + size, computed as `ascent + descent + leading`.
    ///
    /// Useful for callers that need to size a widget against the
    /// intrinsic line height *before* any content has been laid out
    /// (an empty editor that wants to report a `min_lines`-tall
    /// intrinsic size, for example). Returns `0.0` when no default
    /// font is registered or the face cannot be opened.
    ///
    /// Does not apply any per-block `line_height_multiplier` —
    /// multipliers live on `BlockFormat`, not on the font, and have
    /// no meaning when there's no block to multiply against. Use
    /// [`measure_line_height`](Self::measure_line_height) when you
    /// already have a [`TextFormat`].
    pub fn default_line_height(&self) -> f32 {
        self.measure_line_height(&TextFormat::default())
    }

    /// Line height (in logical pixels) for an explicit
    /// [`TextFormat`], computed as `ascent + descent + leading` of
    /// the resolved font at the resolved size. Fields left as `None`
    /// fall back to the registry's defaults — same resolution path
    /// as live inline runs (see `font::resolve::resolve_font`).
    ///
    /// Returns `0.0` when the format cannot be resolved (no default
    /// font registered, requested family missing and no fallback,
    /// face fails to open).
    pub fn measure_line_height(&self, format: &TextFormat) -> f32 {
        let font_point_size = format.font_size.map(|s| s as u32);
        let resolved = match resolve_font(
            &self.font_registry,
            format.font_family.as_deref(),
            format.font_weight,
            format.font_bold,
            format.font_italic,
            font_point_size,
            self.scale_factor,
        ) {
            Some(r) => r,
            None => return 0.0,
        };
        match font_metrics_px(&self.font_registry, &resolved) {
            Some(m) => m.ascent + m.descent + m.leading,
            None => 0.0,
        }
    }

    // ── HiDPI scale factor ──────────────────────────────────────

    /// Set the device pixel ratio for HiDPI rasterization.
    ///
    /// Layout stays in logical pixels; glyphs are shaped and
    /// rasterized at `size_px * scale_factor` so text is crisp on
    /// HiDPI displays. Orthogonal to [`DocumentFlow::set_zoom`],
    /// which is a post-layout display transform.
    ///
    /// Changing this value invalidates the glyph cache and the
    /// atlas (both are cleared here) and marks every
    /// [`DocumentFlow`] that was laid out against this service as
    /// stale via the `scale_generation` counter. The caller must
    /// then re-run `layout_full` / `layout_blocks` on every flow
    /// before the next render — existing shaped advances depended
    /// on the old ppem rounding.
    ///
    /// Clamped to `0.25..=8.0`. Default is `1.0`.
    ///
    /// [`DocumentFlow`]: crate::DocumentFlow
    /// [`DocumentFlow::set_zoom`]: crate::DocumentFlow::set_zoom
    pub fn set_scale_factor(&mut self, scale_factor: f32) {
        let sf = scale_factor.clamp(0.25, 8.0);
        if (self.scale_factor - sf).abs() <= f32::EPSILON {
            return;
        }
        self.scale_factor = sf;
        // Glyph raster cells were produced at the old physical size
        // and would be wrong at the new one. Drop the cache outright.
        self.glyph_cache.entries.clear();
        // The bucketed allocator still holds the rectangles for those
        // evicted glyphs; start from a fresh allocator so the space
        // is actually reclaimed rather than fragmented.
        self.atlas = GlyphAtlas::new();
        // Bump the generation so per-widget flows can detect the
        // invalidation and re-run their layouts.
        self.scale_generation = self.scale_generation.wrapping_add(1);
    }

    /// The current scale factor (default `1.0`).
    pub fn scale_factor(&self) -> f32 {
        self.scale_factor
    }

    /// Monotonic counter bumped by every successful
    /// [`set_scale_factor`](Self::set_scale_factor) call.
    ///
    /// `DocumentFlow` snapshots this during layout so the framework
    /// can ask whether a flow needs to be re-laid out after a HiDPI
    /// change without having to track the transition itself.
    pub fn scale_generation(&self) -> u64 {
        self.scale_generation
    }

    // ── Atlas ───────────────────────────────────────────────────

    /// Read the glyph atlas state without triggering a render.
    ///
    /// Optionally advances the cache generation and runs eviction.
    /// Returns an [`AtlasSnapshot`] the caller can pattern-match
    /// by field. The atlas's internal dirty flag is cleared here,
    /// so the caller must either upload `pixels` during the
    /// returned borrow or accept a one-frame delay.
    ///
    /// When `snapshot.glyphs_evicted` is true, callers that cache
    /// glyph positions (e.g. paint caches) must invalidate —
    /// evicted atlas slots may be reused by subsequent allocations
    /// and old UVs would now point to the wrong glyph.
    ///
    /// Only advance the generation on frames where actual text
    /// work happened; skipping eviction on idle frames prevents
    /// aging out glyphs that are still visible but not re-measured
    /// this tick.
    pub fn atlas_snapshot(&mut self, advance_generation: bool) -> AtlasSnapshot<'_> {
        let mut glyphs_evicted = false;
        if advance_generation {
            self.glyph_cache.advance_generation();
            let evicted = self.glyph_cache.evict_unused();
            glyphs_evicted = !evicted.is_empty();
            for alloc_id in evicted {
                self.atlas.deallocate(alloc_id);
            }
        }

        let dirty = self.atlas.dirty;
        let width = self.atlas.width;
        let height = self.atlas.height;
        if dirty {
            self.atlas.dirty = false;
        }
        AtlasSnapshot {
            dirty,
            width,
            height,
            pixels: &self.atlas.pixels[..],
            glyphs_evicted,
        }
    }

    /// Mark the given glyph cache keys as used in the current
    /// generation, preventing them from being evicted. Use this when
    /// glyph quads are cached externally (e.g. per-widget paint
    /// caches) and the normal `rasterize_glyph_quad` → `get()` path
    /// is skipped.
    pub fn touch_glyphs(&mut self, keys: &[crate::atlas::cache::GlyphCacheKey]) {
        self.glyph_cache.touch(keys);
    }

    /// True if the atlas has pending pixel changes since the last
    /// upload. The atlas is marked clean after every `render()` that
    /// copies pixels into its `RenderFrame`; this accessor exposes
    /// the flag for framework paint-cache invalidation decisions.
    pub fn atlas_dirty(&self) -> bool {
        self.atlas.dirty
    }

    /// Current atlas texture width in pixels.
    pub fn atlas_width(&self) -> u32 {
        self.atlas.width
    }

    /// Current atlas texture height in pixels.
    pub fn atlas_height(&self) -> u32 {
        self.atlas.height
    }

    /// Raw atlas pixel buffer (RGBA8).
    pub fn atlas_pixels(&self) -> &[u8] {
        &self.atlas.pixels
    }

    /// Mark the atlas clean after the caller has uploaded its
    /// contents to the GPU. Paired with `atlas_dirty` + `atlas_pixels`
    /// for framework adapters that upload directly from the service
    /// instead of consuming `RenderFrame::atlas_pixels`.
    pub fn mark_atlas_clean(&mut self) {
        self.atlas.dirty = false;
    }
}

impl Default for TextFontService {
    fn default() -> Self {
        Self::new()
    }
}
