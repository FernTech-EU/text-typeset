use std::collections::HashMap;

use crate::font::registry::FontRegistry;
use crate::layout::block::{
    BlockLayout, BlockLayoutParams, PaintSpan, apply_paint_spans, layout_block,
};
use crate::layout::frame::{FrameLayout, FrameLayoutParams, layout_frame};
use crate::layout::table::{TableLayout, TableLayoutParams, layout_table};

pub enum FlowItem {
    Block {
        block_id: usize,
        y: f32,
        height: f32,
    },
    Table {
        table_id: usize,
        y: f32,
        height: f32,
    },
    Frame {
        frame_id: usize,
        y: f32,
        height: f32,
    },
}

pub struct FlowLayout {
    pub blocks: HashMap<usize, BlockLayout>,
    pub tables: HashMap<usize, TableLayout>,
    pub frames: HashMap<usize, FrameLayout>,
    pub flow_order: Vec<FlowItem>,
    pub content_height: f32,
    pub viewport_width: f32,
    pub viewport_height: f32,
    pub cached_max_content_width: f32,
    /// Device pixel ratio passed to shapers and rasterizers.
    /// Layout is always stored in logical pixels; this only affects
    /// precision (physical ppem) and glyph bitmap resolution.
    pub scale_factor: f32,
    /// Per-document logical text-magnification factor (`1.0` = none). Set from
    /// `DocumentFlow::font_scale` at layout time and threaded into block layout
    /// alongside `scale_factor`; multiplies the resolved font size so all text
    /// grows logically (advances, line heights, content height) and reflows.
    pub font_scale: f32,
    /// Un-overlaid (shaped) copy of every laid-out block, keyed by block_id.
    /// The paint-overlay fast path re-derives the live blocks from these so
    /// repeated highlight changes never compound run splits. Populated by a
    /// full layout (`layout_blocks`) and refreshed per block on incremental
    /// relayout.
    base_blocks: HashMap<usize, BlockLayout>,
    /// Current paint-only highlight overlay per block_id. Empty for a block
    /// means "no overlay" (base colors). Kept so an incrementally relaid block
    /// re-applies its overlay.
    pending_paint_spans: HashMap<usize, Vec<PaintSpan>>,
}

impl Default for FlowLayout {
    fn default() -> Self {
        Self::new()
    }
}

impl FlowLayout {
    pub fn new() -> Self {
        Self {
            blocks: HashMap::new(),
            tables: HashMap::new(),
            frames: HashMap::new(),
            flow_order: Vec::new(),
            content_height: 0.0,
            viewport_width: 0.0,
            viewport_height: 0.0,
            cached_max_content_width: 0.0,
            scale_factor: 1.0,
            font_scale: 1.0,
            base_blocks: HashMap::new(),
            pending_paint_spans: HashMap::new(),
        }
    }

    /// Add a table to the flow at the current y position.
    pub fn add_table(
        &mut self,
        registry: &FontRegistry,
        params: &TableLayoutParams,
        available_width: f32,
    ) {
        let mut table = layout_table(
            registry,
            params,
            available_width,
            self.scale_factor,
            self.font_scale,
        );

        let mut y = self.content_height;
        table.y = y;
        y += table.total_height;

        self.flow_order.push(FlowItem::Table {
            table_id: table.table_id,
            y: table.y,
            height: table.total_height,
        });
        if table.total_width > self.cached_max_content_width {
            self.cached_max_content_width = table.total_width;
        }
        self.tables.insert(table.table_id, table);
        self.content_height = y;
    }

    /// Add a frame to the flow.
    ///
    /// - **Inline**: placed in normal flow, advances content_height.
    /// - **FloatLeft**: placed at current y, x=0. Does not advance content_height
    ///   (surrounding content wraps around it).
    /// - **FloatRight**: placed at current y, x=available_width - frame_width.
    /// - **Absolute**: placed at (margin_left, margin_top) from document origin.
    ///   Does not affect flow at all.
    pub fn add_frame(
        &mut self,
        registry: &FontRegistry,
        params: &FrameLayoutParams,
        available_width: f32,
    ) {
        use crate::layout::frame::FramePosition;

        let mut frame = layout_frame(
            registry,
            params,
            available_width,
            self.scale_factor,
            self.font_scale,
        );

        match params.position {
            FramePosition::Inline => {
                frame.y = self.content_height;
                frame.x = 0.0;
                self.content_height += frame.total_height;
            }
            FramePosition::FloatLeft => {
                frame.y = self.content_height;
                frame.x = 0.0;
                // Float doesn't advance content_height -content wraps beside it.
                // For simplicity, we still advance so subsequent blocks appear below.
                // True float wrapping would require a "float exclusion zone" tracked
                // during paragraph layout, which is significantly more complex.
                self.content_height += frame.total_height;
            }
            FramePosition::FloatRight => {
                frame.y = self.content_height;
                frame.x = (available_width - frame.total_width).max(0.0);
                self.content_height += frame.total_height;
            }
            FramePosition::Absolute => {
                // Absolute frames are positioned relative to the document origin
                // using their margin values as coordinates. They don't affect flow.
                frame.y = params.margin_top;
                frame.x = params.margin_left;
                // Don't advance content_height
            }
        }

        self.flow_order.push(FlowItem::Frame {
            frame_id: frame.frame_id,
            y: frame.y,
            height: frame.total_height,
        });
        if frame.total_width > self.cached_max_content_width {
            self.cached_max_content_width = frame.total_width;
        }
        self.frames.insert(frame.frame_id, frame);
    }

    /// Clear all layout state. Call before rebuilding from a new FlowSnapshot.
    pub fn clear(&mut self) {
        self.blocks.clear();
        self.tables.clear();
        self.frames.clear();
        self.flow_order.clear();
        self.content_height = 0.0;
        self.cached_max_content_width = 0.0;
        self.base_blocks.clear();
        self.pending_paint_spans.clear();
    }

    /// Re-capture every laid-out block (top-level, table cells, frames,
    /// recursively) as the paint-overlay base. Called after a full layout.
    pub(crate) fn refresh_base_blocks(&mut self) {
        self.base_blocks.clear();
        let mut collected: Vec<(usize, BlockLayout)> = Vec::new();
        for b in self.blocks.values() {
            collected.push((b.block_id, b.clone()));
        }
        for t in self.tables.values() {
            collect_table_base(t, &mut collected);
        }
        for f in self.frames.values() {
            collect_frame_base(f, &mut collected);
        }
        for (id, b) in collected {
            self.base_blocks.insert(id, b);
        }
    }

    /// Replace the paint-only color overlay for the whole flow.
    ///
    /// `spans_by_block` maps block_id → its disjoint paint spans (from
    /// `text_document`'s `extract_paint_spans`). Every block is re-derived from
    /// its captured base, so colors/decorations change but glyph positions,
    /// advances, line breaks, and heights do NOT — no reshape, no reflow.
    /// Blocks absent from the map reset to base colors.
    pub fn apply_paint_spans_for(&mut self, spans_by_block: HashMap<usize, Vec<PaintSpan>>) {
        self.pending_paint_spans = spans_by_block;
        let base = &self.base_blocks;
        let pending = &self.pending_paint_spans;
        for b in self.blocks.values_mut() {
            overlay_block_in_place(b, base, pending);
        }
        for t in self.tables.values_mut() {
            for c in &mut t.cell_layouts {
                for b in &mut c.blocks {
                    overlay_block_in_place(b, base, pending);
                }
            }
        }
        for f in self.frames.values_mut() {
            overlay_frame_in_place(f, base, pending);
        }
    }

    /// Apply (or clear, when `spans` is empty) the paint overlay for a single
    /// block, re-derived from its base. Returns `false` if `block_id` has no
    /// captured base.
    pub fn apply_block_paint_spans(&mut self, block_id: usize, spans: &[PaintSpan]) -> bool {
        if !self.base_blocks.contains_key(&block_id) {
            return false;
        }
        if spans.is_empty() {
            self.pending_paint_spans.remove(&block_id);
        } else {
            self.pending_paint_spans.insert(block_id, spans.to_vec());
        }
        let base = &self.base_blocks;
        let pending = &self.pending_paint_spans;
        if let Some(b) = self.blocks.get_mut(&block_id) {
            overlay_block_in_place(b, base, pending);
            return true;
        }
        for t in self.tables.values_mut() {
            for c in &mut t.cell_layouts {
                for b in &mut c.blocks {
                    if b.block_id == block_id {
                        overlay_block_in_place(b, base, pending);
                        return true;
                    }
                }
            }
        }
        for f in self.frames.values_mut() {
            if overlay_one_in_frame(f, block_id, base, pending) {
                return true;
            }
        }
        true
    }

    /// After an incremental relayout of `block_id`, re-capture its (base-colored)
    /// shaped output as the new base and re-apply its pending overlay in place.
    ///
    /// The base re-capture happens unconditionally — even when no overlay is
    /// currently active. The fresh shaped output IS the new base, and a later
    /// `apply_block_paint_spans` (the engine re-applying syntax / search /
    /// spell highlights after an edit) overlays from `base_blocks`. If we
    /// skipped the re-capture when no spans were pending, that overlay would
    /// re-derive the block from the STALE pre-edit base and silently clobber
    /// the just-typed text (visible only in highlights-on views; a full
    /// re-layout from a resize would restore it).
    fn refresh_base_and_overlay_block(&mut self, block_id: usize) {
        let fresh = find_block_ref(self, block_id).cloned();
        if let Some(b) = fresh {
            self.base_blocks.insert(block_id, b);
        }
        // Nothing to overlay if no block carries pending paint spans — the
        // freshly-reshaped block already holds the correct base-colored output.
        if self.pending_paint_spans.is_empty() {
            return;
        }
        let base = &self.base_blocks;
        let pending = &self.pending_paint_spans;
        if let Some(b) = self.blocks.get_mut(&block_id) {
            overlay_block_in_place(b, base, pending);
            return;
        }
        for t in self.tables.values_mut() {
            for c in &mut t.cell_layouts {
                for b in &mut c.blocks {
                    if b.block_id == block_id {
                        overlay_block_in_place(b, base, pending);
                        return;
                    }
                }
            }
        }
        for f in self.frames.values_mut() {
            if overlay_one_in_frame(f, block_id, base, pending) {
                return;
            }
        }
    }

    /// Add a single block to the flow at the current y position.
    ///
    /// **Bulk-path primitive.** This deliberately does *not* capture the
    /// block's paint-overlay base; [`layout_blocks`](Self::layout_blocks) calls
    /// it in a loop and captures every base once at the end, which keeps the
    /// bulk path single-pass.
    ///
    /// Appending to an *existing* layout has no such follow-up pass, so use
    /// [`append_block`](Self::append_block) instead. Reaching for this one is
    /// silent when wrong: the block lays out and renders fine, but
    /// [`apply_block_paint_spans`](Self::apply_block_paint_spans) then
    /// early-returns `false` for it forever, so it never receives syntax,
    /// search, or spell highlighting and nothing reports a problem.
    pub fn add_block(
        &mut self,
        registry: &FontRegistry,
        params: &BlockLayoutParams,
        available_width: f32,
    ) {
        let mut block = layout_block(
            registry,
            params,
            available_width,
            self.scale_factor,
            self.font_scale,
        );

        // Margin collapsing with previous block
        let mut y = self.content_height;
        if let Some(FlowItem::Block {
            block_id: prev_id, ..
        }) = self.flow_order.last()
        {
            if let Some(prev_block) = self.blocks.get(prev_id) {
                let collapsed = prev_block.bottom_margin.max(block.top_margin);
                y -= prev_block.bottom_margin;
                y += collapsed;
            } else {
                y += block.top_margin;
            }
        } else {
            y += block.top_margin;
        }

        block.y = y;
        let block_content = block.height - block.top_margin - block.bottom_margin;
        y += block_content + block.bottom_margin;

        self.flow_order.push(FlowItem::Block {
            block_id: block.block_id,
            y: block.y,
            height: block.height,
        });
        self.update_max_width_for_block(&block);
        self.blocks.insert(block.block_id, block);
        self.content_height = y;
    }

    /// Append one block to the tail of an *existing* layout, in O(1).
    ///
    /// This is [`add_block`](Self::add_block) plus the paint-overlay
    /// bookkeeping that a bulk layout would otherwise do afterwards.
    /// [`layout_blocks`](Self::layout_blocks) calls `add_block` in a loop and
    /// then re-captures every block's base in one pass
    /// (`refresh_base_blocks`, O(N)); `add_block` on its own therefore leaves
    /// the appended block absent from `base_blocks`, and a later
    /// `apply_paint_spans_for` would re-derive it from a missing base.
    ///
    /// Reusing the bulk refresh for a tail append would re-clone the whole
    /// document once per appended line — precisely the O(N)-per-line cost this
    /// method exists to avoid — so it refreshes just the appended block
    /// (`refresh_base_and_overlay_block`, the same O(1) call
    /// [`relayout_block`](Self::relayout_block) already relies on).
    ///
    /// `add_block` itself is left untouched: `layout_blocks` depends on its
    /// current no-base-refresh behaviour to keep the bulk path single-pass.
    pub fn append_block(
        &mut self,
        registry: &FontRegistry,
        params: &BlockLayoutParams,
        available_width: f32,
    ) {
        self.add_block(registry, params, available_width);
        self.refresh_base_and_overlay_block(params.block_id);
    }

    /// Drop the first `n` top-level blocks, returning how many were removed.
    ///
    /// The eviction half of a bounded streaming buffer (a log viewer's
    /// scrollback cap). Usually O(n) map removals plus one `Vec` memmove of the
    /// surviving `flow_order` entries — no reshaping. The return value is the
    /// count actually evicted, which is less than `n` when the run of leading
    /// blocks is shorter than `n` or a table/frame stops the walk.
    ///
    /// Surviving blocks deliberately **keep their absolute `y`**. The vacated
    /// band at the top simply becomes empty, and `content_height` is unchanged,
    /// so nothing below moves and no scroll position shifts under the user.
    /// Rewriting every survivor's `y` would make eviction O(remaining) and
    /// would yank the viewport; leaving them is both cheaper and correct.
    /// `flow_order`'s ascending-`y` ordering — which `hit_test`'s binary search
    /// relies on — is preserved, since removing a prefix of a sorted run leaves
    /// it sorted.
    ///
    /// Only top-level blocks are considered; a leading table or frame stops the
    /// eviction (a streaming buffer is a flat run of blocks by construction).
    ///
    /// If the widest block in the flow is among those evicted,
    /// `max_content_width` is recomputed from the survivors — O(remaining) for
    /// that call only. Evicting narrower blocks (the overwhelmingly common
    /// case) cannot lower the maximum and skips the recompute.
    pub fn remove_leading(&mut self, n: usize) -> usize {
        let mut removed = 0;
        let mut evicted_widest = false;

        for item in self.flow_order.iter().take(n) {
            let FlowItem::Block { block_id, .. } = item else {
                break;
            };
            if let Some(block) = self.blocks.remove(block_id) {
                // `cached_max_content_width` only ever grows, so it goes stale
                // exactly when the block that set it leaves. Comparing against
                // it here keeps the common case O(n).
                if block_max_width(&block) >= self.cached_max_content_width {
                    evicted_widest = true;
                }
            }
            self.base_blocks.remove(block_id);
            self.pending_paint_spans.remove(block_id);
            removed += 1;
        }

        self.flow_order.drain(..removed);
        if evicted_widest {
            self.recompute_max_content_width();
        }
        removed
    }

    /// Re-derive `cached_max_content_width` from everything currently laid out.
    ///
    /// The cache is a running maximum that only grows, which is fine while a
    /// flow only ever gains content. Anything that *removes* content has to
    /// re-derive it, or the horizontal scroll range keeps describing content
    /// that is no longer there.
    pub(crate) fn recompute_max_content_width(&mut self) {
        let mut max: f32 = 0.0;
        for block in self.blocks.values() {
            max = max.max(block_max_width(block));
        }
        for table in self.tables.values() {
            max = max.max(table.total_width);
        }
        for frame in self.frames.values() {
            max = max.max(frame.total_width);
        }
        self.cached_max_content_width = max;
    }

    /// Declare the total extent of a uniform-row-height document without
    /// shaping anything.
    ///
    /// In windowed mode `content_height` describes the whole document, not the
    /// shaped window, so it cannot come from the usual accumulator. Use this to
    /// keep the scrollbar honest when the row count changes outside the shaped
    /// window (a line appended while the user is scrolled away from the tail).
    ///
    /// Leaves the shaped window untouched.
    pub fn set_uniform_extent(&mut self, total_rows: usize, row_height: f32) {
        self.content_height = total_rows as f32 * row_height;
    }

    /// Shape only `window` — a slice of a much larger uniform-row-height
    /// document — placing each row at an arithmetic `y = index * row_height`.
    ///
    /// Shaping every line is what makes a large document expensive: a resident
    /// shaped line costs ~6.5 KB, so a 100 000-line buffer costs ~623 MB fully
    /// laid out, against ~1 MB for a viewport-sized window
    /// (`docs/streaming-baseline.md`). Rendering already culls to the viewport,
    /// so shaping the off-screen remainder buys nothing.
    ///
    /// `y` is arithmetic rather than accumulated precisely so that a row can be
    /// placed without having laid out — or even having in memory — any of the
    /// rows above it, which is what lets the window start at an arbitrary
    /// scroll position. `content_height` is likewise derived from
    /// `total_rows`, so the scrollbar reflects the whole document rather than
    /// the shaped window.
    ///
    /// # Invariants
    ///
    /// The arithmetic placement is only correct for a document whose rows are
    /// genuinely uniform: **one row = one visual line of exactly `row_height`**
    /// — no wrapping (`non_breakable_lines`), no embedded newlines, no
    /// per-row margins, one font size throughout. That suits log/console
    /// output and monospaced code; it does not suit prose. Callers with
    /// variable-height content must use [`layout_blocks`](Self::layout_blocks).
    /// `window` must be sorted ascending by index, which keeps `flow_order`
    /// ascending by `y` as `hit_test`'s binary search requires. Both are
    /// checked in debug builds.
    ///
    /// After this call, appending at the tail with
    /// [`append_block`](Self::append_block) stays correct: `content_height` is
    /// exactly `total_rows * row_height`, which is where row `total_rows`
    /// belongs. Trim the window's front with
    /// [`remove_leading`](Self::remove_leading).
    ///
    /// # Behaviour worth knowing
    ///
    /// Like [`layout_blocks`](Self::layout_blocks), this drops any paint
    /// overlay: re-apply spans after re-windowing, or the new rows render in
    /// base colours. Because this runs on every visible-range change, that
    /// re-apply is part of the scroll path, not a one-off.
    ///
    /// `max_content_width` accumulates across windows rather than being
    /// re-derived per window: it reports the widest row *seen so far*, not the
    /// widest row in the document (unknowable without shaping all of it) and
    /// not the widest row on screen (which would make the horizontal scrollbar
    /// jump on every vertical scroll). It therefore only grows during a
    /// windowed session.
    ///
    /// `y` is `index * row_height` in `f32`, which represents consecutive
    /// integers exactly only to 2^24: past ~840 000 rows at a 20 px row height,
    /// row positions begin quantizing and neighbouring rows drift out of
    /// alignment. Well beyond the sizes this targets, but not infinite.
    pub fn layout_window(
        &mut self,
        registry: &FontRegistry,
        window: &[(usize, BlockLayoutParams)],
        total_rows: usize,
        row_height: f32,
        available_width: f32,
    ) {
        debug_assert!(
            window.windows(2).all(|w| w[0].0 < w[1].0),
            "layout_window: window must be sorted ascending by row index, else \
             flow_order stops being ascending by y and hit_test's binary search \
             silently misreports rows"
        );
        debug_assert!(
            window.last().is_none_or(|(last, _)| *last < total_rows),
            "layout_window: window reaches row {:?} but the document declares \
             only {total_rows} rows — those rows would sit below content_height \
             and be unreachable by scrolling",
            window.last().map(|(i, _)| *i)
        );

        // The widest row seen so far must survive the rebuild: re-deriving it
        // from the window alone would make the horizontal scroll range track
        // whatever happens to be on screen, so it would jump on every vertical
        // scroll.
        let max_width_seen = self.cached_max_content_width;
        self.clear();
        self.cached_max_content_width = max_width_seen;

        for (index, params) in window {
            let mut block = layout_block(
                registry,
                params,
                available_width,
                self.scale_factor,
                self.font_scale,
            );
            debug_assert!(
                (block.height - row_height).abs() < 0.5,
                "layout_window: row {index} laid out {:.2}px tall against a \
                 declared row_height of {row_height:.2}px — the row is not a \
                 single unwrapped line, so arithmetic y placement would drift",
                block.height
            );

            let y = *index as f32 * row_height;
            block.y = y;
            self.flow_order.push(FlowItem::Block {
                block_id: block.block_id,
                y,
                height: block.height,
            });
            self.update_max_width_for_block(&block);
            self.blocks.insert(block.block_id, block);
        }

        // Describes the whole document, not the shaped window — so the
        // scrollbar spans everything even though almost none of it is shaped.
        self.set_uniform_extent(total_rows, row_height);
        // O(window), not O(document): the bulk path's cost is bounded by what
        // is actually resident.
        self.refresh_base_blocks();
    }

    /// Lay out a sequence of blocks vertically.
    pub fn layout_blocks(
        &mut self,
        registry: &FontRegistry,
        block_params: Vec<BlockLayoutParams>,
        available_width: f32,
    ) {
        self.clear();
        // Note: viewport_width is NOT set here. It's a display property
        // set by Typesetter::set_viewport(), not a layout property.
        // available_width is the layout width which may differ from viewport
        // when using ContentWidthMode::Fixed.
        for params in &block_params {
            self.add_block(registry, params, available_width);
        }
        // Capture the freshly-shaped blocks as the paint-overlay base. A full
        // layout clears any prior overlay (see `clear`), so the live blocks ARE
        // the base at this point; the engine applies paint spans afterward via
        // `apply_paint_spans_for`.
        self.refresh_base_blocks();
    }

    /// Update a single block's layout and shift subsequent items if height changed.
    ///
    /// Finds the block in top-level blocks, table cells, or frames, re-layouts
    /// it, and propagates any height delta to subsequent flow items.
    /// Relayout the one block `params` names, wherever it lives, and shift the
    /// document-character positions of everything after it.
    ///
    /// Returns `false` when the block is in none of the three places a block can
    /// be — top level, a table cell, a frame. The caller must treat that as a
    /// failed relayout and fall back to a full one: the shift is half of this
    /// method's job, so skipping it silently leaves every later block's
    /// `position` describing the document as it was before the edit.
    #[must_use]
    pub fn relayout_block(
        &mut self,
        registry: &FontRegistry,
        params: &BlockLayoutParams,
        available_width: f32,
    ) -> bool {
        let block_id = params.block_id;

        // Top-level block
        if self.blocks.contains_key(&block_id) {
            self.relayout_top_level_block(registry, params, available_width);
            self.refresh_base_and_overlay_block(block_id);
            return true;
        }

        // Table cell block: scan tables for the block_id
        let table_match = self.tables.iter().find_map(|(&tid, table)| {
            for cell in &table.cell_layouts {
                if cell.blocks.iter().any(|b| b.block_id == block_id) {
                    return Some((tid, cell.row, cell.column));
                }
            }
            None
        });
        if let Some((table_id, row, col)) = table_match {
            let old_char_len = block_char_len(find_block_ref(self, block_id));
            self.relayout_table_block(registry, params, table_id, row, col);
            let new_char_len = block_char_len(find_block_ref(self, block_id));
            let char_delta = new_char_len as isize - old_char_len as isize;
            self.shift_block_positions_after_table_block(table_id, block_id, char_delta);
            self.refresh_base_and_overlay_block(block_id);
            return true;
        }

        // Frame block: scan frames (including nested frames) for the block_id
        let frame_match = self.frames.iter().find_map(|(&fid, frame)| {
            if frame_contains_block(frame, block_id) {
                return Some(fid);
            }
            None
        });
        if let Some(frame_id) = frame_match {
            let old_char_len = block_char_len(find_block_ref(self, block_id));
            self.relayout_frame_block(registry, params, frame_id);
            let new_char_len = block_char_len(find_block_ref(self, block_id));
            let char_delta = new_char_len as isize - old_char_len as isize;
            self.shift_block_positions_after_frame_block(frame_id, block_id, char_delta);
            self.refresh_base_and_overlay_block(block_id);
            return true;
        }

        false
    }

    /// Relayout a top-level block (existing logic).
    fn relayout_top_level_block(
        &mut self,
        registry: &FontRegistry,
        params: &BlockLayoutParams,
        available_width: f32,
    ) {
        let block_id = params.block_id;
        let old_y = self.blocks.get(&block_id).map(|b| b.y).unwrap_or(0.0);
        let old_height = self.blocks.get(&block_id).map(|b| b.height).unwrap_or(0.0);
        let old_top_margin = self
            .blocks
            .get(&block_id)
            .map(|b| b.top_margin)
            .unwrap_or(0.0);
        let old_bottom_margin = self
            .blocks
            .get(&block_id)
            .map(|b| b.bottom_margin)
            .unwrap_or(0.0);
        let old_content = old_height - old_top_margin - old_bottom_margin;
        let old_end = old_y + old_content + old_bottom_margin;
        let old_char_len = block_char_len(self.blocks.get(&block_id));

        let mut block = layout_block(
            registry,
            params,
            available_width,
            self.scale_factor,
            self.font_scale,
        );
        block.y = old_y;

        if (block.top_margin - old_top_margin).abs() > 0.001 {
            let prev_bm = self.prev_block_bottom_margin(block_id).unwrap_or(0.0);
            let old_collapsed = prev_bm.max(old_top_margin);
            let new_collapsed = prev_bm.max(block.top_margin);
            block.y = old_y + (new_collapsed - old_collapsed);
        }

        let new_content = block.height - block.top_margin - block.bottom_margin;
        let new_end = block.y + new_content + block.bottom_margin;
        let delta = new_end - old_end;
        let new_char_len = block_char_len(Some(&block));
        let char_delta = new_char_len as isize - old_char_len as isize;

        let new_y = block.y;
        let new_height = block.height;
        self.update_max_width_for_block(&block);
        self.blocks.insert(block_id, block);

        // Update flow_order entry
        for item in &mut self.flow_order {
            if let FlowItem::Block {
                block_id: id,
                y,
                height,
            } = item
                && *id == block_id
            {
                *y = new_y;
                *height = new_height;
                break;
            }
        }

        self.shift_items_after_block(block_id, delta);
        self.shift_block_positions_after_block(block_id, char_delta);
    }

    /// Relayout a block inside a table cell. Recomputes the row height
    /// and propagates any table height delta to subsequent flow items.
    fn relayout_table_block(
        &mut self,
        registry: &FontRegistry,
        params: &BlockLayoutParams,
        table_id: usize,
        row: usize,
        col: usize,
    ) {
        let table = match self.tables.get_mut(&table_id) {
            Some(t) => t,
            None => return,
        };

        let old_table_height = table.total_height;
        recompute_table_cell(
            table,
            registry,
            params,
            row,
            col,
            self.scale_factor,
            self.font_scale,
        );
        let delta = table.total_height - old_table_height;

        // Update flow_order entry for this table
        for item in &mut self.flow_order {
            if let FlowItem::Table {
                table_id: id,
                height,
                ..
            } = item
                && *id == table_id
            {
                *height = table.total_height;
                break;
            }
        }

        self.shift_items_after_table(table_id, delta);
    }

    /// Relayout a block inside a frame. Handles direct blocks, blocks in
    /// cells of frame-nested tables, and blocks in nested frames (any
    /// depth). Recomputes frame content height and propagates any height
    /// delta to subsequent flow items.
    fn relayout_frame_block(
        &mut self,
        registry: &FontRegistry,
        params: &BlockLayoutParams,
        frame_id: usize,
    ) {
        let old_total_height = match self.frames.get(&frame_id) {
            Some(f) => f.total_height,
            None => return,
        };

        {
            let frame = self.frames.get_mut(&frame_id).unwrap();
            relayout_block_deep_in_frame(
                frame,
                registry,
                params,
                self.scale_factor,
                self.font_scale,
            );
        }

        let new_total_height = self.frames[&frame_id].total_height;
        let delta = new_total_height - old_total_height;

        for item in &mut self.flow_order {
            if let FlowItem::Frame {
                frame_id: id,
                height,
                ..
            } = item
                && *id == frame_id
            {
                *height = new_total_height;
                break;
            }
        }

        self.shift_items_after_frame(frame_id, delta);
    }

    /// Shift the document-character `position` of every block that appears
    /// after the given target block in flow order by `char_delta` characters.
    ///
    /// `shift_items_after_block` only propagates the vertical pixel delta.
    /// This method propagates the character delta so hit_test and caret_rect
    /// keep returning correct document positions after an incremental
    /// relayout that changed the target block's char length (e.g. a cut or
    /// paste inside a non-last paragraph).
    fn shift_block_positions_after_block(&mut self, block_id: usize, char_delta: isize) {
        self.shift_block_positions_after_flow_item(FlowItemRef::Block(block_id), char_delta);
    }

    /// Shift the document-character `position` of every block belonging to
    /// a flow item that appears after `target` in flow order.
    fn shift_block_positions_after_flow_item(&mut self, target: FlowItemRef, char_delta: isize) {
        if char_delta == 0 {
            return;
        }
        // Snapshot the order of items so we can mutate the containing
        // HashMaps inside the loop.
        let refs: Vec<FlowItemRef> = self
            .flow_order
            .iter()
            .map(|item| match item {
                FlowItem::Block { block_id, .. } => FlowItemRef::Block(*block_id),
                FlowItem::Table { table_id, .. } => FlowItemRef::Table(*table_id),
                FlowItem::Frame { frame_id, .. } => FlowItemRef::Frame(*frame_id),
            })
            .collect();
        let mut found = false;
        for r in refs {
            if found {
                match r {
                    FlowItemRef::Block(id) => {
                        if let Some(b) = self.blocks.get_mut(&id) {
                            b.position = apply_char_delta(b.position, char_delta);
                        }
                    }
                    FlowItemRef::Table(id) => {
                        if let Some(t) = self.tables.get_mut(&id) {
                            shift_block_positions_in_table(t, char_delta);
                        }
                    }
                    FlowItemRef::Frame(id) => {
                        if let Some(f) = self.frames.get_mut(&id) {
                            shift_block_positions_in_frame(f, char_delta);
                        }
                    }
                }
            } else if r == target {
                found = true;
            }
        }
    }

    /// Shift document positions after an edit inside a top-level table's
    /// cell: cell blocks after the edited block within the table, then
    /// every flow item after the table.
    fn shift_block_positions_after_table_block(
        &mut self,
        table_id: usize,
        block_id: usize,
        char_delta: isize,
    ) {
        if char_delta == 0 {
            return;
        }
        if let Some(table) = self.tables.get_mut(&table_id) {
            shift_table_positions_after_block(table, block_id, char_delta);
        }
        self.shift_block_positions_after_flow_item(FlowItemRef::Table(table_id), char_delta);
    }

    /// Shift document positions after an edit inside a frame (any depth):
    /// frame content after the edited block, then every flow item after
    /// the frame.
    fn shift_block_positions_after_frame_block(
        &mut self,
        frame_id: usize,
        block_id: usize,
        char_delta: isize,
    ) {
        if char_delta == 0 {
            return;
        }
        if let Some(frame) = self.frames.get_mut(&frame_id) {
            shift_frame_positions_after_block(frame, block_id, char_delta);
        }
        self.shift_block_positions_after_flow_item(FlowItemRef::Frame(frame_id), char_delta);
    }

    /// Shift all flow items after the given block by `delta` pixels.
    fn shift_items_after_block(&mut self, block_id: usize, delta: f32) {
        if delta.abs() <= 0.001 {
            return;
        }
        let mut found = false;
        for item in &mut self.flow_order {
            match item {
                FlowItem::Block {
                    block_id: id, y, ..
                } => {
                    if found {
                        *y += delta;
                        if let Some(b) = self.blocks.get_mut(id) {
                            b.y += delta;
                        }
                    }
                    if *id == block_id {
                        found = true;
                    }
                }
                FlowItem::Table {
                    table_id: id, y, ..
                } => {
                    if found {
                        *y += delta;
                        if let Some(t) = self.tables.get_mut(id) {
                            t.y += delta;
                        }
                    }
                }
                FlowItem::Frame {
                    frame_id: id, y, ..
                } => {
                    if found {
                        *y += delta;
                        if let Some(f) = self.frames.get_mut(id) {
                            f.y += delta;
                        }
                    }
                }
            }
        }
        self.content_height += delta;
    }

    /// Shift all flow items after the given table by `delta` pixels.
    fn shift_items_after_table(&mut self, table_id: usize, delta: f32) {
        if delta.abs() <= 0.001 {
            return;
        }
        let mut found = false;
        for item in &mut self.flow_order {
            match item {
                FlowItem::Table {
                    table_id: id, y, ..
                } => {
                    if *id == table_id {
                        found = true;
                        continue;
                    }
                    if found {
                        *y += delta;
                        if let Some(t) = self.tables.get_mut(id) {
                            t.y += delta;
                        }
                    }
                }
                FlowItem::Block {
                    block_id: id, y, ..
                } => {
                    if found {
                        *y += delta;
                        if let Some(b) = self.blocks.get_mut(id) {
                            b.y += delta;
                        }
                    }
                }
                FlowItem::Frame {
                    frame_id: id, y, ..
                } => {
                    if found {
                        *y += delta;
                        if let Some(f) = self.frames.get_mut(id) {
                            f.y += delta;
                        }
                    }
                }
            }
        }
        self.content_height += delta;
    }

    /// Shift all flow items after the given frame by `delta` pixels.
    fn shift_items_after_frame(&mut self, frame_id: usize, delta: f32) {
        if delta.abs() <= 0.001 {
            return;
        }
        let mut found = false;
        for item in &mut self.flow_order {
            match item {
                FlowItem::Frame {
                    frame_id: id, y, ..
                } => {
                    if *id == frame_id {
                        found = true;
                        continue;
                    }
                    if found {
                        *y += delta;
                        if let Some(f) = self.frames.get_mut(id) {
                            f.y += delta;
                        }
                    }
                }
                FlowItem::Block {
                    block_id: id, y, ..
                } => {
                    if found {
                        *y += delta;
                        if let Some(b) = self.blocks.get_mut(id) {
                            b.y += delta;
                        }
                    }
                }
                FlowItem::Table {
                    table_id: id, y, ..
                } => {
                    if found {
                        *y += delta;
                        if let Some(t) = self.tables.get_mut(id) {
                            t.y += delta;
                        }
                    }
                }
            }
        }
        self.content_height += delta;
    }

    /// Update the cached max content width considering a single block's lines.
    fn update_max_width_for_block(&mut self, block: &BlockLayout) {
        let w = block_max_width(block);
        if w > self.cached_max_content_width {
            self.cached_max_content_width = w;
        }
    }

    /// Find the bottom margin of the block immediately before `block_id` in flow order.
    fn prev_block_bottom_margin(&self, block_id: usize) -> Option<f32> {
        let mut prev_bm = None;
        for item in &self.flow_order {
            match item {
                FlowItem::Block { block_id: id, .. } => {
                    if *id == block_id {
                        return prev_bm;
                    }
                    if let Some(b) = self.blocks.get(id) {
                        prev_bm = Some(b.bottom_margin);
                    }
                }
                _ => {
                    // Non-block items reset margin collapsing
                    prev_bm = None;
                }
            }
        }
        None
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FlowItemRef {
    Block(usize),
    Table(usize),
    Frame(usize),
}

fn block_char_len(block: Option<&BlockLayout>) -> usize {
    block
        .and_then(|b| b.lines.last().map(|l| l.char_range.end))
        .unwrap_or(0)
}

fn apply_char_delta(position: usize, delta: isize) -> usize {
    if delta >= 0 {
        position + delta as usize
    } else {
        position.saturating_sub((-delta) as usize)
    }
}

fn shift_block_positions_in_slice(blocks: &mut [BlockLayout], delta: isize) {
    for block in blocks {
        block.position = apply_char_delta(block.position, delta);
    }
}

fn shift_block_positions_in_table(table: &mut TableLayout, delta: isize) {
    for cell in &mut table.cell_layouts {
        shift_block_positions_in_slice(&mut cell.blocks, delta);
    }
}

fn shift_block_positions_in_frame(frame: &mut FrameLayout, delta: isize) {
    shift_block_positions_in_slice(&mut frame.blocks, delta);
    for table in &mut frame.tables {
        shift_block_positions_in_table(table, delta);
    }
    for nested in &mut frame.frames {
        shift_block_positions_in_frame(nested, delta);
    }
}

/// Shift the `position` of every cell block that comes after `block_id`
/// within `table` (cells iterate in document/row-major order). Returns
/// `true` when the edited block was found in this table.
fn shift_table_positions_after_block(
    table: &mut TableLayout,
    block_id: usize,
    delta: isize,
) -> bool {
    let mut found = false;
    for cell in &mut table.cell_layouts {
        for b in &mut cell.blocks {
            if found {
                b.position = apply_char_delta(b.position, delta);
            } else if b.block_id == block_id {
                found = true;
            }
        }
    }
    found
}

/// Shift the `position` of every block that comes after `block_id` in
/// `frame`'s document order — walking `flow_order` so interleaved blocks,
/// tables, and nested frames shift correctly. Returns `true` when the
/// edited block was found in this frame tree.
fn shift_frame_positions_after_block(
    frame: &mut FrameLayout,
    block_id: usize,
    delta: isize,
) -> bool {
    let order = frame.flow_order.clone();
    let mut found = false;
    for child in &order {
        match child {
            crate::layout::frame::FrameChildRef::Block(bid) => {
                if found {
                    if let Some(b) = frame.blocks.iter_mut().find(|b| b.block_id == *bid) {
                        b.position = apply_char_delta(b.position, delta);
                    }
                } else if *bid == block_id {
                    found = true;
                }
            }
            crate::layout::frame::FrameChildRef::Table(tid) => {
                if let Some(t) = frame.tables.iter_mut().find(|t| t.table_id == *tid) {
                    if found {
                        shift_block_positions_in_table(t, delta);
                    } else {
                        found = shift_table_positions_after_block(t, block_id, delta);
                    }
                }
            }
            crate::layout::frame::FrameChildRef::Frame(fid) => {
                if let Some(nested) = frame.frames.iter_mut().find(|f| f.frame_id == *fid) {
                    if found {
                        shift_block_positions_in_frame(nested, delta);
                    } else {
                        found = shift_frame_positions_after_block(nested, block_id, delta);
                    }
                }
            }
        }
    }
    found
}

// ── Paint-overlay helpers (recolor without reshape/reflow) ──────────────────

/// Re-derive `b` in place from its base + pending overlay. No-op if no base
/// was captured for it.
///
/// The base supplies the *shaped* output — runs, glyphs, line breaks — which is
/// what a recolor re-derives. It does **not** supply where the block sits.
///
/// Those are two different lifetimes. A base is captured when its own block is
/// laid out, but a block's placement changes without it being laid out at all:
/// an edit anywhere above shifts every later block's document-character
/// `position` and its `y`, live, through `shift_block_positions_after_block` and
/// `shift_items_after_block` — neither of which re-captures the bases they
/// invalidate. `apply_paint_spans` opens with `base.clone()`, so overlaying
/// without care puts a block back where it was before an edit it was never part
/// of. `hit_test` and `caret_rect` read `position` straight, so from then on a
/// click near that block landed N characters early and its selection highlights
/// painted N characters off — N being however many characters had been typed
/// above it. Only a full relayout recovered.
///
/// Carrying the live placement across is the whole fix: it is exactly the two
/// fields the shift functions own, and the live block is never the staler of the
/// two — a relayout of this block refreshes its base from it.
fn overlay_block_in_place(
    b: &mut BlockLayout,
    base: &HashMap<usize, BlockLayout>,
    pending: &HashMap<usize, Vec<PaintSpan>>,
) {
    if let Some(base_b) = base.get(&b.block_id) {
        let empty: Vec<PaintSpan> = Vec::new();
        let spans = pending.get(&b.block_id).unwrap_or(&empty);
        let (position, y) = (b.position, b.y);
        *b = apply_paint_spans(base_b, spans);
        b.position = position;
        b.y = y;
    }
}

/// Re-derive every block in a frame (recursively) from base + pending.
fn overlay_frame_in_place(
    frame: &mut FrameLayout,
    base: &HashMap<usize, BlockLayout>,
    pending: &HashMap<usize, Vec<PaintSpan>>,
) {
    for b in &mut frame.blocks {
        overlay_block_in_place(b, base, pending);
    }
    for t in &mut frame.tables {
        for c in &mut t.cell_layouts {
            for b in &mut c.blocks {
                overlay_block_in_place(b, base, pending);
            }
        }
    }
    for nested in &mut frame.frames {
        overlay_frame_in_place(nested, base, pending);
    }
}

/// Re-derive a single block (by id) inside a frame (recursively). Returns true
/// if found.
fn overlay_one_in_frame(
    frame: &mut FrameLayout,
    block_id: usize,
    base: &HashMap<usize, BlockLayout>,
    pending: &HashMap<usize, Vec<PaintSpan>>,
) -> bool {
    for b in &mut frame.blocks {
        if b.block_id == block_id {
            overlay_block_in_place(b, base, pending);
            return true;
        }
    }
    for t in &mut frame.tables {
        for c in &mut t.cell_layouts {
            for b in &mut c.blocks {
                if b.block_id == block_id {
                    overlay_block_in_place(b, base, pending);
                    return true;
                }
            }
        }
    }
    for nested in &mut frame.frames {
        if overlay_one_in_frame(nested, block_id, base, pending) {
            return true;
        }
    }
    false
}

fn collect_table_base(t: &TableLayout, out: &mut Vec<(usize, BlockLayout)>) {
    for c in &t.cell_layouts {
        for b in &c.blocks {
            out.push((b.block_id, b.clone()));
        }
    }
}

fn collect_frame_base(f: &FrameLayout, out: &mut Vec<(usize, BlockLayout)>) {
    for b in &f.blocks {
        out.push((b.block_id, b.clone()));
    }
    for t in &f.tables {
        collect_table_base(t, out);
    }
    for nested in &f.frames {
        collect_frame_base(nested, out);
    }
}

/// Find a block by id across top-level / table cells / frames.
/// Widest laid-out line of `block`, margins included.
///
/// The single definition of "how wide is this block", shared by the running
/// maximum (`update_max_width_for_block`) and the re-derivation eviction needs
/// (`recompute_max_content_width`), so the two cannot drift apart and disagree
/// about the horizontal scroll range.
fn block_max_width(block: &BlockLayout) -> f32 {
    block
        .lines
        .iter()
        .map(|line| line.width + block.left_margin + block.right_margin)
        .fold(0.0_f32, f32::max)
}

pub(crate) fn find_block_ref(flow: &FlowLayout, block_id: usize) -> Option<&BlockLayout> {
    if let Some(b) = flow.blocks.get(&block_id) {
        return Some(b);
    }
    for t in flow.tables.values() {
        for c in &t.cell_layouts {
            for b in &c.blocks {
                if b.block_id == block_id {
                    return Some(b);
                }
            }
        }
    }
    for f in flow.frames.values() {
        if let Some(b) = find_block_in_frame(f, block_id) {
            return Some(b);
        }
    }
    None
}

fn find_block_in_frame(frame: &FrameLayout, block_id: usize) -> Option<&BlockLayout> {
    for b in &frame.blocks {
        if b.block_id == block_id {
            return Some(b);
        }
    }
    for t in &frame.tables {
        for c in &t.cell_layouts {
            for b in &c.blocks {
                if b.block_id == block_id {
                    return Some(b);
                }
            }
        }
    }
    for nested in &frame.frames {
        if let Some(b) = find_block_in_frame(nested, block_id) {
            return Some(b);
        }
    }
    None
}

/// Check whether a frame (or any of its nested frames) contains a block
/// with the given id — including blocks inside cells of tables that are
/// direct children of the frame.
pub(crate) fn frame_contains_block(frame: &FrameLayout, block_id: usize) -> bool {
    if frame.blocks.iter().any(|b| b.block_id == block_id) {
        return true;
    }
    for table in &frame.tables {
        for cell in &table.cell_layouts {
            if cell.blocks.iter().any(|b| b.block_id == block_id) {
                return true;
            }
        }
    }
    frame
        .frames
        .iter()
        .any(|nested| frame_contains_block(nested, block_id))
}

/// Where inside a frame a given block lives (one level deep — nested
/// frames are reported as `NestedFrame` and must be descended into).
pub(crate) enum FrameBlockLocation {
    /// A direct child block of the frame.
    DirectBlock,
    /// A block inside a cell of a table that is a direct child of the
    /// frame. Carries `(table_id, row, column)`.
    TableCell(usize, usize, usize),
    /// A block somewhere inside a nested frame (carries the nested
    /// frame's id).
    NestedFrame(usize),
}

/// Locate `block_id` among the direct children of `frame`.
pub(crate) fn find_block_location_in_frame(
    frame: &FrameLayout,
    block_id: usize,
) -> Option<FrameBlockLocation> {
    if frame.blocks.iter().any(|b| b.block_id == block_id) {
        return Some(FrameBlockLocation::DirectBlock);
    }
    for table in &frame.tables {
        for cell in &table.cell_layouts {
            if cell.blocks.iter().any(|b| b.block_id == block_id) {
                return Some(FrameBlockLocation::TableCell(
                    table.table_id,
                    cell.row,
                    cell.column,
                ));
            }
        }
    }
    for nested in &frame.frames {
        if frame_contains_block(nested, block_id) {
            return Some(FrameBlockLocation::NestedFrame(nested.frame_id));
        }
    }
    None
}

/// Replace a direct child block of `frame` and re-stack the frame's
/// children. The caller is responsible for routing blocks that live in
/// nested frames or table cells (see `relayout_block_deep_in_frame`) —
/// the block must have been laid out at this frame's `content_width`.
fn relayout_block_in_frame(frame: &mut FrameLayout, block_id: usize, new_block: BlockLayout) {
    if let Some(old) = frame.blocks.iter_mut().find(|b| b.block_id == block_id) {
        *old = new_block;
    }
    reposition_frame_children(frame);
}

/// Relayout the block identified by `params.block_id` wherever it lives
/// inside `frame`: as a direct block, inside a cell of a frame-nested
/// table, or anywhere within a nested frame (recursing to any depth).
/// Each level re-stacks its children in flow order on the way back up,
/// so height changes propagate to the outermost frame's `total_height`.
fn relayout_block_deep_in_frame(
    frame: &mut FrameLayout,
    registry: &FontRegistry,
    params: &BlockLayoutParams,
    scale_factor: f32,
    font_scale: f32,
) {
    match find_block_location_in_frame(frame, params.block_id) {
        None => {}
        Some(FrameBlockLocation::DirectBlock) => {
            let new_block = layout_block(
                registry,
                params,
                frame.content_width,
                scale_factor,
                font_scale,
            );
            relayout_block_in_frame(frame, params.block_id, new_block);
        }
        Some(FrameBlockLocation::TableCell(table_id, row, col)) => {
            if let Some(table) = frame.tables.iter_mut().find(|t| t.table_id == table_id) {
                recompute_table_cell(table, registry, params, row, col, scale_factor, font_scale);
            }
            reposition_frame_children(frame);
        }
        Some(FrameBlockLocation::NestedFrame(nested_frame_id)) => {
            if let Some(nested) = frame
                .frames
                .iter_mut()
                .find(|f| f.frame_id == nested_frame_id)
            {
                relayout_block_deep_in_frame(nested, registry, params, scale_factor, font_scale);
            }
            reposition_frame_children(frame);
        }
    }
}

/// Re-layout the block identified by `params.block_id` inside the cell at
/// `(row, col)`, then recompute the row height, row y positions, and the
/// table's `total_height`. Shared by the top-level table relayout path and
/// the frame-nested one; the caller propagates the height delta into its
/// own container.
fn recompute_table_cell(
    table: &mut crate::layout::table::TableLayout,
    registry: &FontRegistry,
    params: &BlockLayoutParams,
    row: usize,
    col: usize,
    scale_factor: f32,
    font_scale: f32,
) {
    let cell_width = table
        .column_content_widths
        .get(col)
        .copied()
        .unwrap_or(200.0);

    // Find the cell and replace the block
    let cell = match table
        .cell_layouts
        .iter_mut()
        .find(|c| c.row == row && c.column == col)
    {
        Some(c) => c,
        None => return,
    };

    let new_block = layout_block(registry, params, cell_width, scale_factor, font_scale);
    if let Some(old) = cell
        .blocks
        .iter_mut()
        .find(|b| b.block_id == params.block_id)
    {
        *old = new_block;
    }

    // Reposition blocks within the cell and recompute cell height
    let mut block_y = 0.0f32;
    for block in &mut cell.blocks {
        block.y = block_y;
        block_y += block.height;
    }
    let cell_height = block_y;

    // Recompute row height by scanning all cells in this row
    if row < table.row_heights.len() {
        let mut max_h = 0.0f32;
        for c in &table.cell_layouts {
            if c.row == row {
                let h: f32 = c.blocks.iter().map(|b| b.height).sum();
                max_h = max_h.max(h);
            }
        }
        // Also consider the cell we just updated
        max_h = max_h.max(cell_height);
        table.row_heights[row] = max_h;
    }

    // Recompute row y positions and total height
    let border = table.border_width;
    let padding = table.cell_padding;
    let spacing = if table.row_ys.len() > 1 {
        // Infer spacing from existing layout
        if table.row_ys.len() >= 2 && !table.row_heights.is_empty() {
            let expected = table.row_ys[0] + padding + table.row_heights[0] + padding;
            (table.row_ys.get(1).copied().unwrap_or(expected) - expected).max(0.0)
        } else {
            0.0
        }
    } else {
        0.0
    };
    let mut y = border;
    for (r, &row_h) in table.row_heights.iter().enumerate() {
        if r < table.row_ys.len() {
            table.row_ys[r] = y + padding;
        }
        y += padding * 2.0 + row_h;
        if r < table.row_heights.len() - 1 {
            y += spacing;
        }
    }
    table.total_height = y + border;
}

/// Recompute the y position of every direct child of `frame` in document
/// (flow) order, then update `content_height` / `total_height`.
///
/// Walks `frame.flow_order` so interleaved children keep their document
/// order — stacking the per-kind vecs one after another would visually
/// reorder a frame containing e.g. [block, table, block]. Placement
/// mirrors `layout_frame` exactly: blocks at `content_y + top_margin`
/// (no collapsing between frame children), tables and nested frames at
/// `content_y` advancing by `total_height`.
pub(crate) fn reposition_frame_children(frame: &mut FrameLayout) {
    let old_content_height = frame.content_height;
    let order = frame.flow_order.clone();
    let mut content_y = 0.0f32;

    for child in &order {
        match child {
            crate::layout::frame::FrameChildRef::Block(bid) => {
                if let Some(block) = frame.blocks.iter_mut().find(|b| b.block_id == *bid) {
                    block.y = content_y + block.top_margin;
                    let block_content = block.height - block.top_margin - block.bottom_margin;
                    content_y = block.y + block_content + block.bottom_margin;
                }
            }
            crate::layout::frame::FrameChildRef::Table(tid) => {
                if let Some(table) = frame.tables.iter_mut().find(|t| t.table_id == *tid) {
                    table.y = content_y;
                    content_y += table.total_height;
                }
            }
            crate::layout::frame::FrameChildRef::Frame(fid) => {
                if let Some(nested) = frame.frames.iter_mut().find(|f| f.frame_id == *fid) {
                    nested.y = content_y;
                    content_y += nested.total_height;
                }
            }
        }
    }

    frame.content_height = content_y;
    frame.total_height += content_y - old_content_height;
}
