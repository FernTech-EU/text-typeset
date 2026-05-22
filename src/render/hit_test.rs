use crate::layout::flow::{FlowItem, FlowLayout};
use crate::types::{CursorAffinity, HitRegion, HitTestResult};

/// Map a screen-space point to a document position.
///
/// The coordinates are relative to the widget's top-left corner,
/// with scroll already accounted for (the caller passes screen coords,
/// and this function adds `scroll_offset` to convert to document space).
pub fn hit_test(flow: &FlowLayout, scroll_offset: f32, x: f32, y: f32) -> Option<HitTestResult> {
    let doc_y = y + scroll_offset;

    // Try frames first (clicks inside frame content areas take priority)
    if let Some(result) = hit_test_in_frame(flow, doc_y, x) {
        return Some(result);
    }

    // Try tables
    if let Some(result) = hit_test_in_table(flow, doc_y, x) {
        return Some(result);
    }

    // Fall back to top-level blocks
    let (block_id, block) = find_block_at_y(flow, doc_y)?;
    hit_test_block(block_id, block, doc_y, x, 0.0, 0.0)
}

/// Get the screen-space caret rectangle at a document position with
/// the given affinity. At soft-wrap boundaries the same position has
/// two visual placements; `affinity` picks between them. At every
/// other position affinity is irrelevant and the result is identical
/// to `CursorAffinity::Downstream`.
pub fn caret_rect(
    flow: &FlowLayout,
    scroll_offset: f32,
    position: usize,
    affinity: CursorAffinity,
) -> [f32; 4] {
    // Search frames first (matching hit_test priority so that after incremental
    // relayout of a frame block, overlapping stale positions in subsequent
    // top-level blocks don't steal the caret).
    for frame in flow.frames.values() {
        if let Some(rect) = caret_rect_in_frame(frame, position, affinity, scroll_offset, 0.0, 0.0)
        {
            return rect;
        }
    }

    // Search table cell blocks (before top-level blocks, matching hit_test priority
    // so that table cell positions are not stolen by an overlapping top-level block).
    for table in flow.tables.values() {
        for cell in &table.cell_layouts {
            if cell.row >= table.row_ys.len() || cell.column >= table.column_xs.len() {
                continue;
            }
            let offset_x = table.column_xs[cell.column];
            let offset_y = table.y + table.row_ys[cell.row];
            for block in &cell.blocks {
                if let Some(rect) = caret_rect_in_block(
                    block,
                    position,
                    affinity,
                    scroll_offset,
                    offset_x,
                    offset_y,
                ) {
                    return rect;
                }
            }
        }
    }

    // Search top-level blocks
    for item in &flow.flow_order {
        let bid = match item {
            FlowItem::Block { block_id, .. } => *block_id,
            _ => continue,
        };
        let block = match flow.blocks.get(&bid) {
            Some(b) => b,
            None => continue,
        };

        if let Some(rect) = caret_rect_in_block(block, position, affinity, scroll_offset, 0.0, 0.0)
        {
            return rect;
        }
    }

    // Fallback: the position is not covered by any block — this happens for
    // positions that sit on a non-text anchor in the document's char space,
    // most notably the 1-char U+FFFC table-anchor sentinel (which lives
    // between the block before a table and the table's first cell). Map it to
    // the caret at the END of the nearest preceding block so the caret lands
    // at a sensible location (just before the table) with a monotonic y,
    // rather than jumping to the document's top-left corner.
    if let Some(rect) = nearest_preceding_caret(flow, scroll_offset, position) {
        return rect;
    }

    // Last resort (position precedes all content): top-left.
    [0.0, -scroll_offset, 2.0, 16.0]
}

/// Caret at the end of the block with the greatest end-position `<= position`,
/// searching frames, table cells, and top-level blocks (the same sources as
/// [`caret_rect`]). Used as a fallback when `position` falls in a gap not
/// covered by any block (e.g. a table-anchor sentinel). Returns `None` when no
/// block ends at or before `position`.
fn nearest_preceding_caret(
    flow: &FlowLayout,
    scroll_offset: f32,
    position: usize,
) -> Option<[f32; 4]> {
    let mut best_end: Option<usize> = None;
    let mut best_rect: Option<[f32; 4]> = None;

    let mut consider = |block: &BlockLayout, offset_x: f32, offset_y: f32| {
        let block_end =
            block.position + block.lines.last().map(|l| l.char_range.end).unwrap_or(0);
        if block_end > position {
            return;
        }
        if best_end.is_some_and(|be| block_end < be) {
            return;
        }
        if let Some(rect) = caret_rect_in_block(
            block,
            block_end,
            CursorAffinity::Downstream,
            scroll_offset,
            offset_x,
            offset_y,
        ) {
            best_end = Some(block_end);
            best_rect = Some(rect);
        }
    };

    // Top-level blocks.
    for item in &flow.flow_order {
        if let FlowItem::Block { block_id, .. } = item
            && let Some(block) = flow.blocks.get(block_id)
        {
            consider(block, 0.0, 0.0);
        }
    }
    // Top-level table cells.
    for table in flow.tables.values() {
        for cell in &table.cell_layouts {
            if cell.row >= table.row_ys.len() || cell.column >= table.column_xs.len() {
                continue;
            }
            let offset_x = table.column_xs[cell.column];
            let offset_y = table.y + table.row_ys[cell.row];
            for block in &cell.blocks {
                consider(block, offset_x, offset_y);
            }
        }
    }
    // Frames (recursively): their blocks and table cells.
    for frame in flow.frames.values() {
        consider_frame_blocks(frame, 0.0, 0.0, &mut consider);
    }

    best_rect
}

/// Visit every block inside a frame (its own blocks, its tables' cells, and
/// nested frames) with the absolute (offset_x, offset_y) of its container,
/// invoking `f` for each. Mirrors `caret_rect_in_frame`'s traversal.
fn consider_frame_blocks(
    frame: &crate::layout::frame::FrameLayout,
    base_x: f32,
    base_y: f32,
    f: &mut impl FnMut(&BlockLayout, f32, f32),
) {
    let fx = base_x + frame.x + frame.content_x;
    let fy = base_y + frame.y + frame.content_y;
    for table in &frame.tables {
        for cell in &table.cell_layouts {
            if cell.row >= table.row_ys.len() || cell.column >= table.column_xs.len() {
                continue;
            }
            let offset_x = fx + table.column_xs[cell.column];
            let offset_y = fy + table.y + table.row_ys[cell.row];
            for block in &cell.blocks {
                f(block, offset_x, offset_y);
            }
        }
    }
    for block in &frame.blocks {
        f(block, fx, fy);
    }
    for nested in &frame.frames {
        consider_frame_blocks(nested, fx, fy, f);
    }
}

// ── Internal helpers ────────────────────────────────────────────

use crate::layout::block::BlockLayout;
use crate::layout::line::LayoutLine;

/// Hit-test a single block, with optional coordinate offsets for frame-nested blocks.
fn hit_test_block(
    block_id: usize,
    block: &BlockLayout,
    doc_y: f32,
    x: f32,
    offset_x: f32,
    offset_y: f32,
) -> Option<HitTestResult> {
    let content_top = offset_y + block.y;
    let local_x = x - offset_x;

    // Find which line within the block FIRST. We need the matched
    // line (and its char_range.start) before deciding what the
    // left-margin shortcut returns — for wrapped blocks, the start
    // of line K (K > 0) is NOT the block's start, and Home pressed
    // from line K should go to LINE K's start, not the block start.
    // (Pre-affinity, the shortcut returned `block.position` here,
    // which silently broke `move_cursor_to_line_edge` for any line
    // past the first wrap line — the bug went unnoticed because
    // Downstream-default caret_rect always picked line 1's rect at
    // a wrap boundary, so the Home probe always landed on line 1.)
    let local_y = doc_y - content_top;
    let (line_idx, line) = match find_line_index_at_y(&block.lines, local_y) {
        Some((i, l)) => (i, l),
        None => {
            // Above all lines: return start of first line
            if let Some(first_line) = block.lines.first()
                && local_y < first_line.y - first_line.ascent
            {
                return Some(HitTestResult {
                    position: block.position + first_line.char_range.start,
                    affinity: CursorAffinity::Downstream,
                    block_id,
                    offset_in_block: first_line.char_range.start,
                    region: HitRegion::BelowContent,
                    tooltip: None,
                    table_id: None,
                });
            }
            // Below all lines: return end of last line
            if let Some(last_line) = block.lines.last() {
                return Some(HitTestResult {
                    position: block.position + last_line.char_range.end,
                    affinity: CursorAffinity::Downstream,
                    block_id,
                    offset_in_block: last_line.char_range.end,
                    region: HitRegion::BelowContent,
                    tooltip: None,
                    table_id: None,
                });
            }
            return Some(HitTestResult {
                position: block.position,
                affinity: CursorAffinity::Downstream,
                block_id,
                offset_in_block: 0,
                region: HitRegion::BelowContent,
                tooltip: None,
                table_id: None,
            });
        }
    };

    // Left margin shortcut: if local_x is to the LEFT of the block's
    // text content area, return the start of the matched line. For
    // wrapped blocks line K (K > 0) starts at its own
    // `char_range.start`, not at the block's `position`. Compute
    // affinity normally so a left-margin click on line K+1 still
    // gets Upstream when line K ends at the same position.
    if local_x < block.left_margin {
        let offset_in_block = line.char_range.start;
        let affinity = if line_idx > 0
            && block
                .lines
                .get(line_idx - 1)
                .is_some_and(|prev| prev.char_range.end == offset_in_block)
        {
            CursorAffinity::Upstream
        } else {
            CursorAffinity::Downstream
        };
        return Some(HitTestResult {
            position: block.position + offset_in_block,
            affinity,
            block_id,
            offset_in_block,
            region: HitRegion::LeftMargin,
            tooltip: None,
            table_id: None,
        });
    }

    // Find which glyph within the line
    let glyph_x = local_x - block.left_margin;
    let (offset_in_block, region, tooltip) = find_position_in_line(line, glyph_x);

    // Affinity rule at soft-wrap boundaries: when the click landed in
    // line K+1's Y range AND the resolved offset equals K+1's
    // char_range.start AND the previous line K ends at the same offset
    // (i.e. K and K+1 share a wrap boundary), the click is on the
    // upstream side and the caret must render at the START of line
    // K+1, not at the END of line K. Otherwise the default Downstream
    // applies (everything else, including end-of-line clicks and
    // non-wrap positions).
    let affinity = if offset_in_block == line.char_range.start
        && line_idx > 0
        && block
            .lines
            .get(line_idx - 1)
            .is_some_and(|prev| prev.char_range.end == offset_in_block)
    {
        CursorAffinity::Upstream
    } else {
        CursorAffinity::Downstream
    };

    Some(HitTestResult {
        position: block.position + offset_in_block,
        affinity,
        block_id,
        offset_in_block,
        region,
        tooltip,
        table_id: None,
    })
}

/// Try to hit-test within frame blocks and tables inside frames.
fn hit_test_in_frame(flow: &FlowLayout, doc_y: f32, x: f32) -> Option<HitTestResult> {
    for item in &flow.flow_order {
        let frame_id = match item {
            FlowItem::Frame {
                frame_id,
                y,
                height,
            } if doc_y >= *y && doc_y < *y + *height => *frame_id,
            _ => continue,
        };
        let frame = flow.frames.get(&frame_id)?;
        let offset_x = frame.x + frame.content_x;
        let offset_y = frame.y + frame.content_y;
        let local_y = doc_y - offset_y;

        // Only claim the hit if the point is within the content area.
        // Points in the frame's chrome (margin/border/padding) should fall
        // through to subsequent flow items so cursor Up/Down can cross
        // frame boundaries.
        if local_y < 0.0 || local_y >= frame.content_height {
            continue;
        }

        // Try tables inside the frame first
        for table in &frame.tables {
            if local_y >= table.y
                && local_y < table.y + table.total_height
                && let Some(result) = hit_test_table_content(table, doc_y, x, offset_x, offset_y)
            {
                return Some(result);
            }
        }

        // Try nested frames
        for nested in &frame.frames {
            let nested_content_y = offset_y + nested.y + nested.content_y;
            let nested_local_y = doc_y - nested_content_y;
            if nested_local_y >= 0.0
                && nested_local_y < nested.content_height
                && let Some(result) = hit_test_frame_content(nested, doc_y, x, offset_x, offset_y)
            {
                return Some(result);
            }
        }

        // Find block at local_y within frame
        for block in &frame.blocks {
            let block_bottom = block.y + block.height;
            if local_y >= block.y && local_y < block_bottom {
                return hit_test_block(block.block_id, block, doc_y, x, offset_x, offset_y);
            }
        }

        // Fallback: last block in frame (point is within content area
        // but between blocks, e.g. in margin collapsing space)
        if let Some(block) = frame.blocks.last() {
            return hit_test_block(block.block_id, block, doc_y, x, offset_x, offset_y);
        }
    }
    None
}

/// Hit-test within a single frame's content (blocks, tables, nested frames).
/// `base_x`/`base_y` are the parent coordinate offsets (before this frame's own offsets).
fn hit_test_frame_content(
    frame: &crate::layout::frame::FrameLayout,
    doc_y: f32,
    x: f32,
    base_x: f32,
    base_y: f32,
) -> Option<HitTestResult> {
    let offset_x = base_x + frame.x + frame.content_x;
    let offset_y = base_y + frame.y + frame.content_y;
    let local_y = doc_y - offset_y;

    // Try tables
    for table in &frame.tables {
        if local_y >= table.y
            && local_y < table.y + table.total_height
            && let Some(result) = hit_test_table_content(table, doc_y, x, offset_x, offset_y)
        {
            return Some(result);
        }
    }

    // Try nested frames (recursive)
    for nested in &frame.frames {
        let nested_content_y = offset_y + nested.y + nested.content_y;
        let nested_local_y = doc_y - nested_content_y;
        if nested_local_y >= 0.0
            && nested_local_y < nested.content_height
            && let Some(result) = hit_test_frame_content(nested, doc_y, x, offset_x, offset_y)
        {
            return Some(result);
        }
    }

    // Try blocks
    for block in &frame.blocks {
        let block_bottom = block.y + block.height;
        if local_y >= block.y && local_y < block_bottom {
            return hit_test_block(block.block_id, block, doc_y, x, offset_x, offset_y);
        }
    }

    // Fallback: last block
    if let Some(block) = frame.blocks.last() {
        return hit_test_block(block.block_id, block, doc_y, x, offset_x, offset_y);
    }

    None
}

/// Try to hit-test within top-level table cells.
fn hit_test_in_table(flow: &FlowLayout, doc_y: f32, x: f32) -> Option<HitTestResult> {
    for item in &flow.flow_order {
        let table_id = match item {
            FlowItem::Table {
                table_id,
                y,
                height,
            } if doc_y >= *y && doc_y < *y + *height => *table_id,
            _ => continue,
        };
        let table = flow.tables.get(&table_id)?;
        return hit_test_table_content(table, doc_y, x, 0.0, 0.0);
    }
    None
}

/// Hit-test within a table's cells. `base_x`/`base_y` are added when
/// the table lives inside a frame.
fn hit_test_table_content(
    table: &crate::layout::table::TableLayout,
    doc_y: f32,
    x: f32,
    base_x: f32,
    base_y: f32,
) -> Option<HitTestResult> {
    let local_y = doc_y - base_y - table.y;
    let local_x = x - base_x;

    // Find row
    let row = find_table_row(table, local_y)?;
    // Find column
    let col = find_table_column(table, local_x)?;

    // Find the cell at (row, col)
    let cell = table
        .cell_layouts
        .iter()
        .find(|c| c.row == row && c.column == col)?;

    let cell_x = base_x + table.column_xs[col];
    let cell_y = base_y + table.y + table.row_ys[row];

    // Find block within the cell
    let tid = table.table_id;
    let cell_local_y = doc_y - cell_y;
    for block in &cell.blocks {
        let block_bottom = block.y + block.height;
        if cell_local_y >= block.y && cell_local_y < block_bottom {
            return hit_test_block(block.block_id, block, doc_y, x, cell_x, cell_y).map(|r| {
                HitTestResult {
                    table_id: Some(tid),
                    ..r
                }
            });
        }
    }

    // Fallback: last block in cell
    if let Some(block) = cell.blocks.last() {
        return hit_test_block(block.block_id, block, doc_y, x, cell_x, cell_y).map(|r| {
            HitTestResult {
                table_id: Some(tid),
                ..r
            }
        });
    }

    None
}

/// Find which row a local y coordinate falls into.
///
/// If the coordinate lands in the gap between two rows (border/spacing area),
/// snaps to the nearest row so that vertical navigation (down-arrow) can
/// cross row boundaries without falling into a dead zone.
fn find_table_row(table: &crate::layout::table::TableLayout, local_y: f32) -> Option<usize> {
    if table.row_ys.is_empty() {
        return None;
    }

    for (r, &row_y) in table.row_ys.iter().enumerate() {
        let row_top = row_y - table.cell_padding;
        let row_bottom =
            row_y + table.row_heights.get(r).copied().unwrap_or(0.0) + table.cell_padding;
        if local_y >= row_top && local_y < row_bottom {
            return Some(r);
        }
    }

    // Point is inside the table but in the gap between rows (or just outside
    // the first/last row padding). Snap to the nearest row.
    let mut best_row = 0;
    let mut best_dist = f32::MAX;
    for (r, &row_y) in table.row_ys.iter().enumerate() {
        let row_top = row_y - table.cell_padding;
        let row_bottom =
            row_y + table.row_heights.get(r).copied().unwrap_or(0.0) + table.cell_padding;
        let row_mid = (row_top + row_bottom) / 2.0;
        let dist = (local_y - row_mid).abs();
        if dist < best_dist {
            best_dist = dist;
            best_row = r;
        }
    }
    Some(best_row)
}

/// Find which column a local x coordinate falls into.
///
/// If the coordinate lands in the gap between columns, snaps to the nearest
/// column (same rationale as `find_table_row`).
fn find_table_column(table: &crate::layout::table::TableLayout, local_x: f32) -> Option<usize> {
    if table.column_xs.is_empty() {
        return None;
    }

    for (c, &col_x) in table.column_xs.iter().enumerate() {
        let col_left = col_x - table.cell_padding;
        let col_right =
            col_x + table.column_content_widths.get(c).copied().unwrap_or(0.0) + table.cell_padding;
        if local_x >= col_left && local_x < col_right {
            return Some(c);
        }
    }

    // Snap to nearest column.
    let mut best_col = 0;
    let mut best_dist = f32::MAX;
    for (c, &col_x) in table.column_xs.iter().enumerate() {
        let col_left = col_x - table.cell_padding;
        let col_right =
            col_x + table.column_content_widths.get(c).copied().unwrap_or(0.0) + table.cell_padding;
        let col_mid = (col_left + col_right) / 2.0;
        let dist = (local_x - col_mid).abs();
        if dist < best_dist {
            best_dist = dist;
            best_col = c;
        }
    }
    Some(best_col)
}

/// Search a frame (and its nested frames, recursively) for a caret position.
fn caret_rect_in_frame(
    frame: &crate::layout::frame::FrameLayout,
    position: usize,
    affinity: CursorAffinity,
    scroll_offset: f32,
    base_x: f32,
    base_y: f32,
) -> Option<[f32; 4]> {
    let fx = base_x + frame.x + frame.content_x;
    let fy = base_y + frame.y + frame.content_y;
    // Search tables before blocks (matching hit_test_in_frame priority).
    for table in &frame.tables {
        for cell in &table.cell_layouts {
            if cell.row >= table.row_ys.len() || cell.column >= table.column_xs.len() {
                continue;
            }
            let offset_x = fx + table.column_xs[cell.column];
            let offset_y = fy + table.y + table.row_ys[cell.row];
            for block in &cell.blocks {
                if let Some(rect) = caret_rect_in_block(
                    block,
                    position,
                    affinity,
                    scroll_offset,
                    offset_x,
                    offset_y,
                ) {
                    return Some(rect);
                }
            }
        }
    }
    for block in &frame.blocks {
        if let Some(rect) = caret_rect_in_block(block, position, affinity, scroll_offset, fx, fy) {
            return Some(rect);
        }
    }
    for nested in &frame.frames {
        if let Some(rect) = caret_rect_in_frame(nested, position, affinity, scroll_offset, fx, fy) {
            return Some(rect);
        }
    }
    None
}

/// Compute the caret rect for a position within a single block.
/// Returns None if the position is not within this block.
///
/// Affinity selection: at a soft-wrap boundary the position equals
/// both line K's `char_range.end` and line K+1's `char_range.start`.
/// The default Downstream picks line K (the earlier match — current
/// pre-affinity behavior). Upstream picks line K+1 (the later match).
/// At every non-boundary position there is only one matching line and
/// affinity is irrelevant.
fn caret_rect_in_block(
    block: &BlockLayout,
    position: usize,
    affinity: CursorAffinity,
    scroll_offset: f32,
    offset_x: f32,
    offset_y: f32,
) -> Option<[f32; 4]> {
    let block_end = block.position + block.lines.last().map(|l| l.char_range.end).unwrap_or(0);

    if position < block.position || position > block_end {
        return None;
    }

    let offset_in_block = position - block.position;

    // Find every line whose char_range contains offset_in_block. At a
    // wrap boundary two consecutive lines match; pick the LAST when
    // affinity is Upstream, the FIRST otherwise.
    let mut matched: Option<&LayoutLine> = None;
    for line in &block.lines {
        if offset_in_block < line.char_range.start {
            continue;
        }
        if offset_in_block > line.char_range.end {
            continue;
        }
        match affinity {
            CursorAffinity::Downstream => {
                matched = Some(line);
                break;
            }
            CursorAffinity::Upstream => {
                // Keep walking — last match wins.
                matched = Some(line);
            }
        }
    }

    let line = matched?;
    let caret_x = line.x_for_offset(offset_in_block) + block.left_margin + offset_x;
    let caret_y = offset_y + block.y + line.y - line.ascent - scroll_offset;
    let caret_height = line.line_height;
    Some([caret_x, caret_y, 2.0, caret_height])
}

fn find_block_at_y(flow: &FlowLayout, doc_y: f32) -> Option<(usize, &BlockLayout)> {
    if flow.flow_order.is_empty() {
        return None;
    }

    // Binary search: find the last item whose y <= doc_y
    let idx = flow.flow_order.partition_point(|item| {
        let y = match item {
            FlowItem::Block { y, .. } | FlowItem::Table { y, .. } | FlowItem::Frame { y, .. } => *y,
        };
        y <= doc_y
    });

    // Check the item at idx-1 (the last one with y <= doc_y) and nearby items
    let start = idx.saturating_sub(1);
    let end = (idx + 1).min(flow.flow_order.len());
    for i in start..end {
        if let FlowItem::Block {
            block_id,
            y,
            height,
        } = &flow.flow_order[i]
            && doc_y >= *y
            && doc_y < *y + *height
            && let Some(block) = flow.blocks.get(block_id)
        {
            return Some((*block_id, block));
        }
    }

    // Determine whether doc_y is above or below all content by comparing
    // against the first item's y. idx==0 means no item has y <= doc_y.
    if idx == 0 {
        // Above all blocks: return the first one
        for item in &flow.flow_order {
            if let FlowItem::Block { block_id, .. } = item
                && let Some(block) = flow.blocks.get(block_id)
            {
                return Some((*block_id, block));
            }
        }
    }

    // doc_y sits in the inter-block gap created by top/bottom margins:
    // no block contains it, but it's not below the document either.
    // Snap to the nearest Block walking BACKWARDS from idx — i.e., the
    // block immediately above the gap. Without this the function fell
    // through to "return the last block of the document", which made
    // mouse-drag selections jump from the anchor to the document end
    // every time the pointer crossed an inter-block margin.
    if idx < flow.flow_order.len() {
        for i in (0..idx).rev() {
            if let FlowItem::Block { block_id, .. } = &flow.flow_order[i]
                && let Some(block) = flow.blocks.get(block_id)
            {
                return Some((*block_id, block));
            }
        }
    }

    // Below all blocks: return the last one
    for item in flow.flow_order.iter().rev() {
        if let FlowItem::Block { block_id, .. } = item
            && let Some(block) = flow.blocks.get(block_id)
        {
            return Some((*block_id, block));
        }
    }
    None
}

/// Find the line at the given local-y, returning its index in `lines`
/// alongside the line reference. The index lets the caller inspect
/// the previous line (line K-1) to detect soft-wrap boundaries and
/// compute the click's [`CursorAffinity`].
fn find_line_index_at_y(lines: &[LayoutLine], local_y: f32) -> Option<(usize, &LayoutLine)> {
    // line.y is the baseline; the line occupies from (y - ascent) to (y - ascent + line_height)
    for (i, line) in lines.iter().enumerate() {
        let line_top = line.y - line.ascent;
        let line_bottom = line_top + line.line_height;
        if local_y >= line_top && local_y < line_bottom {
            return Some((i, line));
        }
    }
    None
}

fn find_position_in_line(line: &LayoutLine, local_x: f32) -> (usize, HitRegion, Option<String>) {
    for run in &line.runs {
        // Inline image hit detection
        if let Some(ref name) = run.shaped_run.image_name {
            let img_end = run.x + run.shaped_run.advance_width;
            if local_x < img_end {
                let offset = run
                    .shaped_run
                    .glyphs
                    .first()
                    .map(|g| g.cluster as usize)
                    .unwrap_or(line.char_range.start);
                return (
                    offset,
                    HitRegion::Image { name: name.clone() },
                    run.decorations.tooltip.clone(),
                );
            }
            continue;
        }

        let mut glyph_x = run.x;

        for glyph in &run.shaped_run.glyphs {
            let glyph_mid = glyph_x + glyph.x_advance / 2.0;

            if local_x < glyph_mid {
                let offset = glyph.cluster as usize;
                let tooltip = run.decorations.tooltip.clone();
                if run.decorations.is_link {
                    return (
                        offset,
                        HitRegion::Link {
                            href: run.decorations.anchor_href.clone().unwrap_or_default(),
                        },
                        tooltip,
                    );
                }
                return (offset, HitRegion::Text, tooltip);
            }

            glyph_x += glyph.x_advance;
        }
    }

    // Past end of line
    (line.char_range.end, HitRegion::PastLineEnd, None)
}
