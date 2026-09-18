mod helpers;
use helpers::{NOTO_SANS, Typesetter};

use text_document::{MoveMode, MoveOperation, TextDocument};
const RICH_MARKDOWN: &str = "First paragraph with **bold words** and *italic words* and ~~strikethrough~~ and <u>underlined</u> and `inline code` wrapping at narrow viewport.

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

fn is_cursor_in_frame(ts: &mut Typesetter, cursor_pos: usize) -> bool {
    let rect = ts.caret_rect(cursor_pos);
    if rect[3] <= 0.0 {
        return false;
    }
    let hx = rect[0].max(1.0);
    let hy = rect[1] + rect[3] * 0.5;
    if let Some(hit) = ts.hit_test(hx, hy) {
        !ts.is_top_level_block(hit.block_id)
    } else {
        false
    }
}

fn relayout_after_edit(doc: &TextDocument, ts: &mut Typesetter) {
    let flow = doc.snapshot_flow();
    ts.layout_full(&flow);
    ts.render();
}

/// Simulate Phase 4 edits, then check frame_block collection
#[test]
fn debug_phase4_frame_blocks() {
    let doc = TextDocument::new();
    let op = doc.set_markdown(RICH_MARKDOWN).unwrap();
    op.wait().unwrap();

    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(f32::INFINITY, 600.0);
    ts.set_content_width(f32::INFINITY);

    let flow = doc.snapshot_flow();
    ts.layout_full(&flow);
    ts.render();

    // Simulate Phase 4 edits (simplified: just insert "X" at a few positions
    // and insert_block at one) to get the document into the right state
    let content_h = ts.content_height();
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
    let positions: Vec<usize> = container_positions
        .iter()
        .step_by(container_positions.len().max(1) / 6)
        .copied()
        .take(6)
        .collect();

    let cursor = doc.cursor();

    // Insert "X" and " word" at each container position (same as Phase 4)
    for &pos in &positions {
        cursor.set_position(pos, MoveMode::MoveAnchor);
        cursor.insert_text("X").unwrap();
        relayout_after_edit(&doc, &mut ts);
        cursor.insert_text(" word").unwrap();
        relayout_after_edit(&doc, &mut ts);
    }

    // Bulk insert
    if let Some(&first_pos) = positions.first() {
        cursor.set_position(first_pos, MoveMode::MoveAnchor);
        cursor
            .insert_text(" alpha bravo charlie delta echo foxtrot golf hotel india juliet")
            .unwrap();
        relayout_after_edit(&doc, &mut ts);
    }

    // Enter key at second position
    if positions.len() >= 2 {
        let approx_pos = positions[1] + 30;
        let clamped = approx_pos.min(doc.character_count());
        cursor.set_position(clamped, MoveMode::MoveAnchor);
        cursor.insert_block().unwrap();
        relayout_after_edit(&doc, &mut ts);
        cursor.insert_text("New line content here").unwrap();
        relayout_after_edit(&doc, &mut ts);
    }

    // Now do the frame block scan (same as phase_repeated_enter_in_frames)
    relayout_after_edit(&doc, &mut ts);
    let content_h = ts.content_height();

    eprintln!("\n=== Frame blocks after Phase 4 edits ===");
    let mut frame_positions: Vec<usize> = Vec::new();
    let mut seen_blocks: Vec<usize> = Vec::new();

    for y_step in (0..(content_h as i32)).step_by(4) {
        if let Some(hit) = ts.hit_test(60.0, y_step as f32)
            && !ts.is_top_level_block(hit.block_id)
            && !seen_blocks.contains(&hit.block_id)
        {
            seen_blocks.push(hit.block_id);
            frame_positions.push(hit.position);
            eprintln!(
                "  frame_block[{}]: block_id={}, pos={}",
                frame_positions.len() - 1,
                hit.block_id,
                hit.position
            );
        }
    }

    // For frame_block[0], walk to end and try Enter
    if let Some(&frame_pos) = frame_positions.first() {
        eprintln!("\nTesting frame_block[0] at pos={}", frame_pos);
        cursor.set_position(frame_pos, MoveMode::MoveAnchor);
        cursor.move_position(MoveOperation::End, MoveMode::MoveAnchor, 1);

        if !is_cursor_in_frame(&mut ts, cursor.position()) {
            cursor.set_position(frame_pos, MoveMode::MoveAnchor);
            loop {
                let prev = cursor.position();
                cursor.move_position(MoveOperation::NextCharacter, MoveMode::MoveAnchor, 1);
                if cursor.position() == prev {
                    break;
                }
                if !is_cursor_in_frame(&mut ts, cursor.position()) {
                    cursor.set_position(prev, MoveMode::MoveAnchor);
                    break;
                }
            }
        }

        let end_pos = cursor.position();
        eprintln!(
            "  Walked to end: pos={}, in_frame={}",
            end_pos,
            is_cursor_in_frame(&mut ts, end_pos)
        );

        // Check block_at for this position
        let bi = doc.block_at(end_pos);
        eprintln!(
            "  block_at({}): {:?}",
            end_pos,
            bi.map(|i| (i.block_id, i.start, i.length))
        );

        // Press Enter
        cursor.insert_block().unwrap();
        relayout_after_edit(&doc, &mut ts);

        let pos_after = cursor.position();
        let in_frame = is_cursor_in_frame(&mut ts, pos_after);
        eprintln!("  After Enter: pos={}, in_frame={}", pos_after, in_frame);

        let bi2 = doc.block_at(pos_after);
        eprintln!(
            "  block_at({}): {:?}",
            pos_after,
            bi2.map(|i| (i.block_id, i.start, i.length))
        );

        // Check what hit_test finds
        let rect = ts.caret_rect(pos_after);
        let hx = rect[0].max(1.0);
        let hy = rect[1] + rect[3] * 0.5;
        if let Some(hit) = ts.hit_test(hx, hy) {
            eprintln!(
                "  hit_test at caret: block_id={}, vi_is_some={}",
                hit.block_id,
                ts.block_visual_info(hit.block_id).is_some()
            );
        }
    }
}
