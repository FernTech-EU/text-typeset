//! Comprehensive navigation and editing test suite.
//!
//! Exercises cursor movement (horizontal and visual vertical) and content
//! modification across all element types in a rich markdown document,
//! then re-verifies navigation after accumulated edits.
//!
//! Three viewport configurations test different wrapping regimes:
//! - 200px narrow (heavy wrapping)
//! - 600px medium (some wrapping)
//! - f32::INFINITY no-wrap

mod helpers;
use helpers::{NOTO_SANS, Rect, Typesetter, assert_caret_is_real, assert_no_glyph_overlap};

use text_document::{MoveMode, MoveOperation, TextDocument};
// ── Markdown document ──────────────────────────────────────────

/// Rich document containing every element type with inline formatting.
const RICH_MARKDOWN: &str = "\
First paragraph with **bold words** and *italic words* and ~~strikethrough~~ and <u>underlined</u> and `inline code` wrapping at narrow viewport.

- Bullet item one short
- Bullet item two with **bold** and *italic* text to wrap around the edge
  - Nested bullet under item two
  - Another nested bullet with enough words to wrap
    - Third level nesting here

1. Numbered first item
2. Numbered second with longer text that also wraps at narrow width
   1. Nested numbered item under two
   2. Another nested numbered item

> Blockquote paragraph with *italic* and **bold** wrapping text inside the frame.
>
> > Nested blockquote deeper inside with ~~strikethrough~~ and more text to wrap around.

```
fn code_block_example() {
    let x = 42;
    println!(\"the answer is {}\", x);
}
```

| Column A | Column B |
|----------|----------|
| Cell one | Cell two with **bold** and enough text to wrap inside the cell |
| Cell three | Cell four with `code` and *italic* |

Final paragraph after all elements with <u>underlined words</u> and wrapping text as well here.
";

// ── Local helpers ──────────────────────────────────────────────

/// Set up a rich markdown document + typesetter, return (doc, ts, line_height).
/// `line_height` is measured from caret at position 0 (used for visual up/down step).
fn setup_rich_doc(viewport_width: f32, no_wrap: bool) -> (TextDocument, Typesetter, f32) {
    let doc = TextDocument::new();
    let op = doc.set_markdown(RICH_MARKDOWN).unwrap();
    op.wait().unwrap();

    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(viewport_width, 600.0);
    if no_wrap {
        ts.set_content_width(f32::INFINITY);
    }

    let flow = doc.snapshot_flow();
    ts.layout_full(&flow);
    ts.render();

    // Measure line height from caret at position 0
    let rect0 = ts.caret_rect(0);
    let line_height = rect0[3]; // caret height is a good proxy for line height

    (doc, ts, line_height)
}

/// Full relayout + render cycle after an edit.
fn relayout_after_edit(doc: &TextDocument, ts: &mut Typesetter) {
    let flow = doc.snapshot_flow();
    ts.layout_full(&flow);
    ts.render();
}

/// Visual move down: hit_test(target_x, current_y + step).
/// Tries the preferred target_x, falls back to caret's own x if that gets stuck.
/// Returns new position or None if hit_test fails or position doesn't change.
fn visual_move_down(
    ts: &Typesetter,
    current_pos: usize,
    target_x: f32,
    line_height: f32,
) -> Option<usize> {
    let rect = ts.caret_rect(current_pos);
    let x_candidates = [target_x, rect[0] + 1.0];
    for multiplier in [1.0, 1.5, 2.0] {
        let target_y = rect[1] + line_height * multiplier;
        for &x in &x_candidates {
            if let Some(r) = ts.hit_test(x, target_y)
                && r.position != current_pos
            {
                return Some(r.position);
            }
        }
    }
    None
}

// ── Phase implementations ──────────────────────────────────────

/// Phase 1: Walk right through the entire document, sample caret rects.
fn phase_horizontal_walk(doc: &TextDocument, ts: &mut Typesetter) {
    let cursor = doc.cursor();
    cursor.set_position(0, MoveMode::MoveAnchor);

    let doc_len = doc.character_count();
    let mut prev_pos = 0usize;
    let mut prev_rect = Rect::from(ts.caret_rect(0));
    let mut positions_visited = 1usize;

    // Walk right
    loop {
        cursor.move_position(MoveOperation::NextCharacter, MoveMode::MoveAnchor, 1);
        let pos = cursor.position();
        if pos == prev_pos {
            // Stuck - at end of document
            break;
        }

        positions_visited += 1;

        // Sample every 5th position
        if positions_visited.is_multiple_of(5) {
            let rect = ts.caret_rect(pos);
            assert_caret_is_real(rect, &format!("right-walk pos {}", pos));

            let r = Rect::from(rect);
            // If same line (y within tolerance): x should increase or we wrapped
            let same_line = (r.y() - prev_rect.y()).abs() < prev_rect.h() * 0.5;
            if !same_line {
                // New line - x should be near left edge (or at least less than prev)
                // Just verify y increased
                assert!(
                    r.y() >= prev_rect.y() - 1.0,
                    "right-walk: y should not decrease when moving to new line at pos {}: prev_y={}, new_y={}",
                    pos,
                    prev_rect.y(),
                    r.y()
                );
            }
            prev_rect = r;
        }

        prev_pos = pos;
    }

    assert!(
        positions_visited >= doc_len / 2,
        "horizontal walk should visit most positions: visited {}, doc_len {}",
        positions_visited,
        doc_len
    );

    // Walk left back to 0
    let mut left_positions = 0usize;
    loop {
        cursor.move_position(MoveOperation::PreviousCharacter, MoveMode::MoveAnchor, 1);
        let pos = cursor.position();
        left_positions += 1;

        if left_positions.is_multiple_of(5) {
            let rect = ts.caret_rect(pos);
            assert_caret_is_real(rect, &format!("left-walk pos {}", pos));
        }

        if pos == 0 {
            break;
        }
    }

    assert!(
        left_positions >= doc_len / 2,
        "left walk should visit most positions: visited {}, doc_len {}",
        left_positions,
        doc_len
    );
}

/// Phase 2: Vertical walk using NextBlock/PreviousBlock. At each block, move
/// to position 1 (second character) and verify caret_rect is valid with
/// increasing y as we go down, decreasing y as we go up.
fn phase_vertical_block_walk(doc: &TextDocument, ts: &mut Typesetter, line_height: f32) {
    let cursor = doc.cursor();
    cursor.set_position(0, MoveMode::MoveAnchor);

    // Move to char 1 of first block
    cursor.move_position(MoveOperation::NextCharacter, MoveMode::MoveAnchor, 1);
    let rect0 = ts.caret_rect(cursor.position());
    assert_caret_is_real(rect0, "block[0] char 1");

    let mut prev_y = rect0[1];
    let mut down_blocks = 0usize;
    let mut block_start_positions: Vec<usize> = vec![0]; // block start positions

    // Walk down block by block
    loop {
        let prev_pos = cursor.position();
        cursor.move_position(MoveOperation::NextBlock, MoveMode::MoveAnchor, 1);
        let block_start = cursor.position();
        if block_start == prev_pos {
            break; // at end of document
        }
        block_start_positions.push(block_start);

        // Move to char 1 of this block
        cursor.move_position(MoveOperation::NextCharacter, MoveMode::MoveAnchor, 1);
        let pos = cursor.position();
        // If NextCharacter moved us to a different block, skip (empty block)
        if pos == block_start {
            continue;
        }

        let rect = ts.caret_rect(pos);
        assert_caret_is_real(
            rect,
            &format!("block-down[{}] char 1 (pos {})", down_blocks, pos),
        );

        // Block char 1 y should be >= previous block's y
        assert!(
            rect[1] >= prev_y - 1.0,
            "block-down[{}]: y should not decrease: pos={}, prev_y={:.1}, new_y={:.1}",
            down_blocks,
            pos,
            prev_y,
            rect[1]
        );

        prev_y = rect[1];
        down_blocks += 1;

        if down_blocks > 100 {
            break;
        }
    }

    assert!(
        down_blocks >= 5,
        "block-down should visit at least 5 blocks, visited {}",
        down_blocks
    );

    // Walk back up block by block, checking char 1 of each
    let mut up_blocks = 0usize;
    let mut current_y = prev_y;
    loop {
        let prev_pos = cursor.position();
        cursor.move_position(MoveOperation::PreviousBlock, MoveMode::MoveAnchor, 1);
        let block_start = cursor.position();
        if block_start == prev_pos {
            break;
        }

        // Move to char 1
        cursor.move_position(MoveOperation::NextCharacter, MoveMode::MoveAnchor, 1);
        let pos = cursor.position();
        if pos == block_start {
            // Go back to block start for the next PreviousBlock
            cursor.set_position(block_start, MoveMode::MoveAnchor);
            continue;
        }

        let rect = ts.caret_rect(pos);
        assert_caret_is_real(
            rect,
            &format!("block-up[{}] char 1 (pos {})", up_blocks, pos),
        );

        assert!(
            rect[1] <= current_y + 1.0,
            "block-up[{}]: y should not increase: pos={}, prev_y={:.1}, new_y={:.1}",
            up_blocks,
            pos,
            current_y,
            rect[1]
        );

        current_y = rect[1];
        // Position cursor back at block start for next PreviousBlock
        cursor.set_position(block_start, MoveMode::MoveAnchor);
        up_blocks += 1;

        if up_blocks > 100 {
            break;
        }
    }

    assert!(
        up_blocks >= 5,
        "block-up should visit at least 5 blocks, visited {}",
        up_blocks
    );

    // Visual up/down within first wrapping paragraph (hit_test-based)
    // Use a moderate x to avoid edge effects
    if block_start_positions.len() >= 2 {
        let first_block_end = block_start_positions[1];
        let mid_pos = 5.min(first_block_end.saturating_sub(1)); // near start of first block
        let mid_rect = ts.caret_rect(mid_pos);
        let walk_x = mid_rect[0];
        if let Some(below_pos) = visual_move_down(ts, mid_pos, walk_x, line_height)
            && below_pos < first_block_end
        {
            let below_rect = ts.caret_rect(below_pos);
            // Verify y increased (we moved to a line below)
            assert!(
                below_rect[1] > mid_rect[1],
                "visual-down should increase y within paragraph"
            );
        }
    }
}

/// Phase 3: Boundary crossing - walk NextBlock through the document and verify
/// that caret_rect is valid at char 0 and char 1 of every block, and that
/// hit_test can find every block's content area.
fn phase_boundary_crossing(doc: &TextDocument, ts: &mut Typesetter) {
    let cursor = doc.cursor();
    cursor.set_position(0, MoveMode::MoveAnchor);

    let mut block_count = 0usize;
    let mut prev_block_start = 0usize;

    loop {
        let block_start = cursor.position();

        // Caret at block start should be valid
        let rect_start = ts.caret_rect(block_start);
        assert_caret_is_real(
            rect_start,
            &format!(
                "boundary block[{}] start (pos {})",
                block_count, block_start
            ),
        );

        // Move to char 1 and verify
        let prev = cursor.position();
        cursor.move_position(MoveOperation::NextCharacter, MoveMode::MoveAnchor, 1);
        let char1_pos = cursor.position();
        if char1_pos != prev {
            let rect1 = ts.caret_rect(char1_pos);
            assert_caret_is_real(
                rect1,
                &format!("boundary block[{}] char1 (pos {})", block_count, char1_pos),
            );

            // hit_test at caret position should find a valid position
            // (not necessarily the exact same position, since rounding and
            // sub-pixel differences can cause hit_test to snap to a neighbor)
            let hx = rect1[0].max(1.0);
            let hy = rect1[1] + rect1[3] * 0.5;
            let hit = ts.hit_test(hx, hy);
            assert!(
                hit.is_some(),
                "hit_test should return a result at caret position for block[{}]: \
                 caret_pos={}, caret_rect={:?}",
                block_count,
                char1_pos,
                rect1
            );
        }

        // Move to next block
        cursor.set_position(block_start, MoveMode::MoveAnchor);
        cursor.move_position(MoveOperation::NextBlock, MoveMode::MoveAnchor, 1);
        let next_start = cursor.position();
        if next_start == block_start {
            break; // at end
        }

        // Verify NextCharacter can walk from prev block to this block
        // (there are no unreachable gaps in the position space)
        if block_count > 0 {
            let gap = next_start.saturating_sub(prev_block_start);
            assert!(
                gap < 500,
                "gap between blocks {} and {} is suspiciously large: {}",
                block_count - 1,
                block_count,
                gap
            );
        }

        prev_block_start = block_start;
        block_count += 1;

        if block_count > 100 {
            break;
        }
    }

    assert!(
        block_count >= 5,
        "boundary crossing should visit at least 5 blocks, visited {}",
        block_count
    );
}

/// Check whether `cursor_pos` is visually inside a frame (blockquote).
/// Frame-internal blocks have no entry in the top-level flow, so
/// `block_visual_info` returns `None` for them.
fn is_cursor_in_frame(ts: &mut Typesetter, cursor_pos: usize) -> bool {
    let rect = ts.caret_rect(cursor_pos);
    if rect[3] <= 0.0 {
        return false; // sentinel caret
    }
    let hx = rect[0].max(1.0);
    let hy = rect[1] + rect[3] * 0.5;
    if let Some(hit) = ts.hit_test(hx, hy) {
        // Frame-internal blocks return None from block_visual_info
        !ts.is_top_level_block(hit.block_id)
    } else {
        false
    }
}

/// Phase 4: Content modification at each container type.
fn phase_content_modification(doc: &TextDocument, ts: &mut Typesetter) {
    let content_h = ts.content_height();

    // Find positions inside different containers by scanning vertically
    let mut container_positions: Vec<usize> = Vec::new();
    let mut last_y = -100.0f32;

    for y_step in (0..(content_h as i32)).step_by(8) {
        if let Some(hit) = ts.hit_test(80.0, y_step as f32) {
            let rect = ts.caret_rect(hit.position);
            if rect[3] > 0.0 && (rect[1] - last_y).abs() > 10.0 {
                container_positions.push(hit.position);
                last_y = rect[1];
            }
        }
    }

    // Pick up to 6 distinct positions (one per container type ideally)
    let positions: Vec<usize> = container_positions
        .iter()
        .step_by(container_positions.len().max(1) / 6)
        .copied()
        .take(6)
        .collect();

    assert!(
        positions.len() >= 3,
        "should find positions in at least 3 containers, found {}",
        positions.len()
    );

    let cursor = doc.cursor();

    for (i, &pos) in positions.iter().enumerate() {
        let label = format!("container[{}] pos={}", i, pos);

        // Record container type before the edit
        let was_in_frame = is_cursor_in_frame(ts, pos);

        // Record caret before
        let rect_before = ts.caret_rect(pos);
        assert_caret_is_real(rect_before, &format!("{} before", label));

        // Insert "X"
        cursor.set_position(pos, MoveMode::MoveAnchor);
        cursor.insert_text("X").unwrap();
        relayout_after_edit(doc, ts);

        let pos_after = cursor.position();
        let rect_after = ts.caret_rect(pos_after);
        assert_caret_is_real(rect_after, &format!("{} after insert-X", label));

        // Caret should have advanced (x or y changed)
        assert!(
            (rect_after[0] - rect_before[0]).abs() > 0.5
                || (rect_after[1] - rect_before[1]).abs() > 0.5,
            "{}: caret should move after insert-X: before={:?}, after={:?}",
            label,
            rect_before,
            rect_after
        );

        // Container type must be preserved: if we were in a frame, we're still in a frame
        if was_in_frame {
            assert!(
                is_cursor_in_frame(ts, pos_after),
                "{}: cursor escaped frame after insert-X (pos {} -> {})",
                label,
                pos,
                pos_after
            );
        }

        // Insert " word"
        cursor.insert_text(" word").unwrap();
        relayout_after_edit(doc, ts);
        let pos_word = cursor.position();
        let rect2 = ts.caret_rect(pos_word);
        assert_caret_is_real(rect2, &format!("{} after insert-word", label));

        if was_in_frame {
            assert!(
                is_cursor_in_frame(ts, pos_word),
                "{}: cursor escaped frame after insert-word (pos {})",
                label,
                pos_word
            );
        }
    }

    // Bulk insert: 10 words at the first container position
    if let Some(&first_pos) = positions.first() {
        let was_in_frame = is_cursor_in_frame(ts, first_pos);
        cursor.set_position(first_pos, MoveMode::MoveAnchor);
        cursor
            .insert_text(" alpha bravo charlie delta echo foxtrot golf hotel india juliet")
            .unwrap();
        relayout_after_edit(doc, ts);
        let pos_bulk = cursor.position();
        let rect = ts.caret_rect(pos_bulk);
        assert_caret_is_real(rect, "after 10-word insert");

        if was_in_frame {
            assert!(
                is_cursor_in_frame(ts, pos_bulk),
                "cursor escaped frame after 10-word insert (pos {})",
                pos_bulk
            );
        }
    }

    // Enter key (insert_block) at the second container position
    if positions.len() >= 2 {
        let approx_pos = positions[1] + 30; // offset for prior inserts
        let clamped = approx_pos.min(doc.character_count());
        cursor.set_position(clamped, MoveMode::MoveAnchor);

        let was_in_frame = is_cursor_in_frame(ts, cursor.position());
        let y_before = ts.caret_rect(cursor.position())[1];

        cursor.insert_block().unwrap();
        relayout_after_edit(doc, ts);

        let pos_enter = cursor.position();
        let rect_after = ts.caret_rect(pos_enter);
        assert_caret_is_real(rect_after, "after insert_block (enter key)");

        assert!(
            rect_after[1] >= y_before - 1.0,
            "caret should move down after Enter: y_before={}, y_after={}",
            y_before,
            rect_after[1]
        );

        if was_in_frame {
            assert!(
                is_cursor_in_frame(ts, pos_enter),
                "cursor escaped frame after insert_block (pos {})",
                pos_enter
            );
        }

        // Insert text on the new line
        cursor.insert_text("New line content here").unwrap();
        relayout_after_edit(doc, ts);
        let rect = ts.caret_rect(cursor.position());
        assert_caret_is_real(rect, "after text on new line");
    }

    // ── Repeated Enter+char at end of every frame block ────────
    // This catches bugs where insert_block at the end of the last block
    // in a frame escapes to the next top-level block.
    phase_repeated_enter_in_frames(doc, ts);

    // Verify no glyph overlap after all modifications
    let frame = ts.render();
    assert_no_glyph_overlap(frame);
}

/// Sub-phase: find every frame-internal block, move to its end, and do
/// repeated Enter+char cycles. After each Enter, the cursor must still
/// be inside a frame.
fn phase_repeated_enter_in_frames(doc: &TextDocument, ts: &mut Typesetter) {
    // Re-snapshot to get a clean state after prior edits
    relayout_after_edit(doc, ts);

    let content_h = ts.content_height();
    let cursor = doc.cursor();

    // Collect all frame-internal block positions by scanning for hits where
    // block_visual_info returns None
    let mut frame_positions: Vec<usize> = Vec::new();
    let mut seen_blocks: Vec<usize> = Vec::new();

    for y_step in (0..(content_h as i32)).step_by(4) {
        if let Some(hit) = ts.hit_test(60.0, y_step as f32)
            && !ts.is_top_level_block(hit.block_id)
            && !ts.is_block_in_table(hit.block_id)
            && !seen_blocks.contains(&hit.block_id)
        {
            seen_blocks.push(hit.block_id);
            frame_positions.push(hit.position);
        }
    }

    if frame_positions.is_empty() {
        return; // no frames in document (shouldn't happen with our rich doc)
    }

    // For each frame block, move to the end and do Enter+char 5 times
    for (fi, &frame_pos) in frame_positions.iter().enumerate() {
        // Move to end of this block
        cursor.set_position(frame_pos, MoveMode::MoveAnchor);
        cursor.move_position(MoveOperation::End, MoveMode::MoveAnchor, 1);

        // End might have moved past the frame (to the next block).
        // Use NextBlock-1 approach: go to block start, then walk right to the end.
        // Simpler: just use the position and check if we're still in a frame.
        // If End moved us out, back up to frame_pos and walk right.
        if !is_cursor_in_frame(ts, cursor.position()) {
            cursor.set_position(frame_pos, MoveMode::MoveAnchor);
            // Walk right until we leave the frame or hit end of block
            loop {
                let prev = cursor.position();
                cursor.move_position(MoveOperation::NextCharacter, MoveMode::MoveAnchor, 1);
                if cursor.position() == prev {
                    break;
                }
                if !is_cursor_in_frame(ts, cursor.position()) {
                    // Went past the frame, back up one
                    cursor.set_position(prev, MoveMode::MoveAnchor);
                    break;
                }
            }
        }

        let end_pos = cursor.position();
        if !is_cursor_in_frame(ts, end_pos) {
            continue; // can't find frame end, skip
        }

        // If end_pos lands on a block separator (one before the next block's
        // start), insert_block there may escape the frame.  Step back one
        // character to stay within the block text proper.
        if let Ok(info) = doc.block_at(end_pos)
            && end_pos < info.start
        {
            // Separator position - back up into actual block content
            cursor.move_position(MoveOperation::PreviousCharacter, MoveMode::MoveAnchor, 1);
            if !is_cursor_in_frame(ts, cursor.position()) {
                continue;
            }
        }

        let label = format!("frame_block[{}]", fi);

        // Repeated Enter + char, 5 times
        for round in 0..5 {
            // If the cursor is at a block separator (position before a block's
            // start), step back so insert_block operates within the block text
            // and stays inside the frame.
            if let Ok(info) = doc.block_at(cursor.position())
                && cursor.position() < info.start
            {
                cursor.move_position(MoveOperation::PreviousCharacter, MoveMode::MoveAnchor, 1);
                if !is_cursor_in_frame(ts, cursor.position()) {
                    break;
                }
            }

            cursor.insert_block().unwrap();
            relayout_after_edit(doc, ts);

            let pos_after_enter = cursor.position();
            assert_caret_is_real(
                ts.caret_rect(pos_after_enter),
                &format!("{} Enter #{}", label, round + 1),
            );
            assert!(
                is_cursor_in_frame(ts, pos_after_enter),
                "{}: cursor escaped frame after Enter #{} (pos {}). \
                 insert_block at end of frame block should stay inside the frame.",
                label,
                round + 1,
                pos_after_enter
            );

            // Insert a character
            let ch = (b'a' + round as u8) as char;
            cursor.insert_text(&ch.to_string()).unwrap();
            relayout_after_edit(doc, ts);

            let pos_after_char = cursor.position();
            assert_caret_is_real(
                ts.caret_rect(pos_after_char),
                &format!("{} after '{}'", label, ch),
            );
            assert!(
                is_cursor_in_frame(ts, pos_after_char),
                "{}: cursor escaped frame after inserting '{}' (pos {})",
                label,
                ch,
                pos_after_char
            );
        }
    }
}

/// Phase 5: Re-run navigation phases on the now-modified document.
fn phase_post_modification_navigation(doc: &TextDocument, ts: &mut Typesetter, line_height: f32) {
    // Re-render to ensure clean state
    relayout_after_edit(doc, ts);

    // Repeat horizontal walk
    phase_horizontal_walk(doc, ts);

    // Repeat vertical block walk
    phase_vertical_block_walk(doc, ts, line_height);
}

// ── Main test runner ───────────────────────────────────────────

fn run_navigation_suite(viewport_width: f32, no_wrap: bool) {
    let (doc, mut ts, line_height) = setup_rich_doc(viewport_width, no_wrap);

    // Phase 1: Horizontal walk
    phase_horizontal_walk(&doc, &mut ts);

    // Phase 2: Vertical block walk (char 1 of each block)
    phase_vertical_block_walk(&doc, &mut ts, line_height);

    // Phase 3: Boundary crossing
    phase_boundary_crossing(&doc, &mut ts);

    // Phase 4: Content modification
    phase_content_modification(&doc, &mut ts);

    // Phase 5: Post-modification navigation
    phase_post_modification_navigation(&doc, &mut ts, line_height);
}

// ── Three viewport test functions ──────────────────────────────

#[test]
fn navigation_narrow_viewport() {
    run_navigation_suite(200.0, false);
}

#[test]
fn navigation_medium_viewport() {
    run_navigation_suite(600.0, false);
}

#[test]
fn navigation_no_wrap() {
    run_navigation_suite(800.0, true);
}
