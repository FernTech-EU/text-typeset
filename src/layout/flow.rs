use std::collections::HashMap;

use crate::font::registry::FontRegistry;
use crate::layout::block::{BlockLayout, BlockLayoutParams, PaintSpan, apply_paint_spans, layout_block};
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
        let mut table = layout_table(registry, params, available_width, self.scale_factor);

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

        let mut frame = layout_frame(registry, params, available_width, self.scale_factor);

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
    pub fn add_block(
        &mut self,
        registry: &FontRegistry,
        params: &BlockLayoutParams,
        available_width: f32,
    ) {
        let mut block = layout_block(registry, params, available_width, self.scale_factor);

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
    pub fn relayout_block(
        &mut self,
        registry: &FontRegistry,
        params: &BlockLayoutParams,
        available_width: f32,
    ) {
        let block_id = params.block_id;

        // Top-level block
        if self.blocks.contains_key(&block_id) {
            self.relayout_top_level_block(registry, params, available_width);
            self.refresh_base_and_overlay_block(block_id);
            return;
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
            self.relayout_table_block(registry, params, table_id, row, col);
            self.refresh_base_and_overlay_block(block_id);
            return;
        }

        // Frame block: scan frames (including nested frames) for the block_id
        let frame_match = self.frames.iter().find_map(|(&fid, frame)| {
            if frame_contains_block(frame, block_id) {
                return Some(fid);
            }
            None
        });
        if let Some(frame_id) = frame_match {
            self.relayout_frame_block(registry, params, frame_id);
            self.refresh_base_and_overlay_block(block_id);
        }
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

        let mut block = layout_block(registry, params, available_width, self.scale_factor);
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

        let cell_width = table
            .column_content_widths
            .get(col)
            .copied()
            .unwrap_or(200.0);
        let old_table_height = table.total_height;

        // Find the cell and replace the block
        let cell = match table
            .cell_layouts
            .iter_mut()
            .find(|c| c.row == row && c.column == col)
        {
            Some(c) => c,
            None => return,
        };

        let new_block = layout_block(registry, params, cell_width, self.scale_factor);
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

    /// Relayout a block inside a frame. Recomputes frame content height
    /// and propagates any height delta to subsequent flow items.
    fn relayout_frame_block(
        &mut self,
        registry: &FontRegistry,
        params: &BlockLayoutParams,
        frame_id: usize,
    ) {
        let frame = match self.frames.get_mut(&frame_id) {
            Some(f) => f,
            None => return,
        };

        let old_total_height = frame.total_height;
        let new_block = layout_block(registry, params, frame.content_width, self.scale_factor);

        relayout_block_in_frame(frame, params.block_id, new_block);

        let delta = frame.total_height - old_total_height;

        for item in &mut self.flow_order {
            if let FlowItem::Frame {
                frame_id: id,
                height,
                ..
            } = item
                && *id == frame_id
            {
                *height = frame.total_height;
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
            match r {
                FlowItemRef::Block(id) => {
                    if found && let Some(b) = self.blocks.get_mut(&id) {
                        b.position = apply_char_delta(b.position, char_delta);
                    }
                    if id == block_id {
                        found = true;
                    }
                }
                FlowItemRef::Table(id) => {
                    if found && let Some(t) = self.tables.get_mut(&id) {
                        shift_block_positions_in_table(t, char_delta);
                    }
                }
                FlowItemRef::Frame(id) => {
                    if found && let Some(f) = self.frames.get_mut(&id) {
                        shift_block_positions_in_frame(f, char_delta);
                    }
                }
            }
        }
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
        for line in &block.lines {
            let w = line.width + block.left_margin + block.right_margin;
            if w > self.cached_max_content_width {
                self.cached_max_content_width = w;
            }
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

// ── Paint-overlay helpers (recolor without reshape/reflow) ──────────────────

/// Re-derive `b` in place from its base + pending overlay. No-op if no base
/// was captured for it.
fn overlay_block_in_place(
    b: &mut BlockLayout,
    base: &HashMap<usize, BlockLayout>,
    pending: &HashMap<usize, Vec<PaintSpan>>,
) {
    if let Some(base_b) = base.get(&b.block_id) {
        let empty: Vec<PaintSpan> = Vec::new();
        let spans = pending.get(&b.block_id).unwrap_or(&empty);
        *b = apply_paint_spans(base_b, spans);
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
fn find_block_ref(flow: &FlowLayout, block_id: usize) -> Option<&BlockLayout> {
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

/// Check whether a frame (or any of its nested frames) contains a block with the given id.
pub(crate) fn frame_contains_block(frame: &FrameLayout, block_id: usize) -> bool {
    if frame.blocks.iter().any(|b| b.block_id == block_id) {
        return true;
    }
    frame
        .frames
        .iter()
        .any(|nested| frame_contains_block(nested, block_id))
}

/// Replace a block inside a frame (searching nested frames recursively)
/// and recompute content/total heights up the tree.
fn relayout_block_in_frame(frame: &mut FrameLayout, block_id: usize, new_block: BlockLayout) {
    let old_content_height = frame.content_height;

    // Try direct blocks first
    if let Some(old) = frame.blocks.iter_mut().find(|b| b.block_id == block_id) {
        *old = new_block;
    } else {
        // Recurse into nested frames
        for nested in &mut frame.frames {
            if frame_contains_block(nested, block_id) {
                relayout_block_in_frame(nested, block_id, new_block);
                break;
            }
        }
    }

    // Reposition all direct content (blocks, tables, nested frames) vertically
    let mut content_y = 0.0f32;
    for block in &mut frame.blocks {
        block.y = content_y + block.top_margin;
        let block_content = block.height - block.top_margin - block.bottom_margin;
        content_y = block.y + block_content + block.bottom_margin;
    }
    for table in &mut frame.tables {
        table.y = content_y;
        content_y += table.total_height;
    }
    for nested in &mut frame.frames {
        nested.y = content_y;
        content_y += nested.total_height;
    }

    frame.content_height = content_y;
    frame.total_height += content_y - old_content_height;
}
