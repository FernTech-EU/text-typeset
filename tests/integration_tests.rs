//! Functional tests using the real text-document API.
//! These tests verify the full pipeline: TextDocument -> FlowSnapshot -> Typesetter -> RenderFrame.

mod helpers;
use helpers::{
    NOTO_SANS, Typesetter, assert_caret_is_real, assert_no_glyph_overlap, make_typesetter,
};

use text_document::TextDocument;
#[test]
fn plain_text_document_renders_glyphs() {
    let doc = TextDocument::new();
    doc.set_plain_text("Hello, world!").unwrap();
    let flow = doc.snapshot_flow();

    let mut ts = make_typesetter();
    ts.layout_full(&flow);
    let frame = ts.render();

    assert!(
        !frame.glyphs.is_empty(),
        "plain text document should produce glyph quads"
    );
    assert!(frame.atlas_dirty);
    assert!(frame.atlas_width > 0);
    assert_no_glyph_overlap(frame);
}

#[test]
fn html_document_renders_glyphs() {
    let doc = TextDocument::new();
    let op = doc
        .set_html("<p>Bold <b>text</b> and <i>italic</i>.</p>")
        .unwrap();
    op.wait().unwrap();
    let flow = doc.snapshot_flow();

    let mut ts = make_typesetter();
    ts.layout_full(&flow);
    let frame = ts.render();

    assert!(
        !frame.glyphs.is_empty(),
        "HTML document should produce glyph quads"
    );
}

#[test]
fn markdown_document_renders() {
    let doc = TextDocument::new();
    let op = doc
        .set_markdown("# Heading\n\nParagraph text.\n\n- Item one\n- Item two")
        .unwrap();
    op.wait().unwrap();
    let flow = doc.snapshot_flow();

    let mut ts = make_typesetter();
    ts.layout_full(&flow);
    let frame = ts.render();

    assert!(
        !frame.glyphs.is_empty(),
        "Markdown document should produce glyph quads"
    );
    assert!(
        ts.content_height() > 0.0,
        "document should have positive content height"
    );
}

#[test]
fn multi_paragraph_document_has_increasing_y() {
    let doc = TextDocument::new();
    doc.set_plain_text("First paragraph.\n\nSecond paragraph.\n\nThird paragraph.")
        .unwrap();
    let flow = doc.snapshot_flow();

    let mut ts = make_typesetter();
    ts.layout_full(&flow);
    let frame = ts.render();

    // Collect unique y positions
    let mut ys: Vec<f32> = frame.glyphs.iter().map(|g| g.screen[1]).collect();
    ys.sort_by(|a, b| a.partial_cmp(b).unwrap());
    ys.dedup_by(|a, b| (*a - *b).abs() < 1.0);

    assert!(
        ys.len() >= 3,
        "3 paragraphs should produce glyphs at 3+ distinct y positions, got {}",
        ys.len()
    );
}

#[test]
fn hit_test_on_document_returns_valid_position() {
    let doc = TextDocument::new();
    doc.set_plain_text("Hello world").unwrap();
    let flow = doc.snapshot_flow();

    let mut ts = make_typesetter();
    ts.layout_full(&flow);
    let _ = ts.render();

    let result = ts.hit_test(40.0, 10.0);
    assert!(result.is_some(), "hit test should return a result");

    let result = result.unwrap();
    assert!(
        result.position <= 11, // "Hello world" = 11 chars
        "hit test position {} should be within document bounds",
        result.position
    );
}

#[test]
fn cursor_and_selection_on_document() {
    let doc = TextDocument::new();
    doc.set_plain_text("Select this text.").unwrap();
    let flow = doc.snapshot_flow();

    let mut ts = make_typesetter();
    ts.layout_full(&flow);

    // Set a selection spanning "this"
    ts.set_cursor(&text_typeset::CursorDisplay {
        position: 7,
        anchor: 11,
        affinity: text_typeset::CursorAffinity::Downstream,
        visible: true,
        selected_cells: vec![],
    });

    let frame = ts.render();

    let selections: Vec<_> = frame
        .decorations
        .iter()
        .filter(|d| d.kind == text_typeset::DecorationKind::Selection)
        .collect();
    assert!(
        !selections.is_empty(),
        "selection should produce highlight rects"
    );

    let carets: Vec<_> = frame
        .decorations
        .iter()
        .filter(|d| d.kind == text_typeset::DecorationKind::Cursor)
        .collect();
    assert_eq!(carets.len(), 1, "should have one caret");
}

#[test]
fn html_with_formatting_renders_decorations() {
    let doc = TextDocument::new();
    let op = doc
        .set_html("<p><u>Underlined</u> and <s>strikethrough</s> text.</p>")
        .unwrap();
    op.wait().unwrap();
    let flow = doc.snapshot_flow();

    let mut ts = make_typesetter();
    ts.layout_full(&flow);
    let frame = ts.render();

    let underlines: Vec<_> = frame
        .decorations
        .iter()
        .filter(|d| d.kind == text_typeset::DecorationKind::Underline)
        .collect();
    assert!(
        !underlines.is_empty(),
        "HTML <u> tag should produce Underline decorations"
    );
}

#[test]
fn document_with_table_renders() {
    let doc = TextDocument::new();
    let cursor = doc.cursor();
    cursor.insert_table(2, 2).unwrap();
    let flow = doc.snapshot_flow();

    let mut ts = make_typesetter();
    ts.layout_full(&flow);
    let frame = ts.render();

    let borders: Vec<_> = frame
        .decorations
        .iter()
        .filter(|d| d.kind == text_typeset::DecorationKind::TableBorder)
        .collect();
    assert!(
        !borders.is_empty(),
        "document with table should produce border decorations"
    );
}

#[test]
fn incremental_update_after_edit() {
    let doc = TextDocument::new();
    doc.set_plain_text("Short.").unwrap();
    let flow = doc.snapshot_flow();

    let mut ts = make_typesetter();
    ts.layout_full(&flow);
    let frame1 = ts.render();
    let count1 = frame1.glyphs.len();

    // Edit the document
    let cursor = doc.cursor();
    cursor.set_position(6, text_document::MoveMode::MoveAnchor);
    cursor.insert_text(" More text here.").unwrap();

    // Relayout with new snapshot
    let flow2 = doc.snapshot_flow();
    ts.layout_full(&flow2);
    let frame2 = ts.render();
    let count2 = frame2.glyphs.len();

    assert!(
        count2 > count1,
        "adding text should produce more glyphs: {} -> {}",
        count1,
        count2
    );
}

#[test]
fn text_then_table_then_text_renders_all() {
    let doc = TextDocument::new();
    doc.set_plain_text("Before table.").unwrap();

    // Insert table after the text
    let cursor = doc.cursor();
    cursor.move_position(
        text_document::MoveOperation::End,
        text_document::MoveMode::MoveAnchor,
        1,
    );
    cursor.insert_block().unwrap();
    cursor.insert_table(1, 2).unwrap();
    cursor.move_position(
        text_document::MoveOperation::End,
        text_document::MoveMode::MoveAnchor,
        1,
    );
    cursor.insert_block().unwrap();
    cursor.insert_text("After table.").unwrap();

    let flow = doc.snapshot_flow();

    let mut ts = make_typesetter();
    ts.layout_full(&flow);
    let frame = ts.render();

    // Should have glyphs from both text blocks AND table borders
    assert!(
        frame.glyphs.len() >= 20,
        "mixed block+table+block should produce many glyphs, got {}",
        frame.glyphs.len()
    );

    let borders: Vec<_> = frame
        .decorations
        .iter()
        .filter(|d| d.kind == text_typeset::DecorationKind::TableBorder)
        .collect();
    assert!(
        !borders.is_empty(),
        "table between text blocks should still produce borders"
    );
}

#[test]
fn heading_renders_larger_than_body() {
    let doc = TextDocument::new();
    let op = doc
        .set_markdown("# Big Heading\n\nNormal paragraph.")
        .unwrap();
    op.wait().unwrap();
    let flow = doc.snapshot_flow();

    let mut ts = make_typesetter();
    ts.layout_full(&flow);
    let frame = ts.render();

    // Heading glyphs should be taller than body glyphs
    // The heading is on the first line (lowest y), body is below
    if frame.glyphs.len() >= 2 {
        // Find the max glyph height on the first line vs last line
        let mut ys_and_heights: Vec<(f32, f32)> = frame
            .glyphs
            .iter()
            .map(|g| (g.screen[1], g.screen[3]))
            .collect();
        ys_and_heights.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

        let first_line_height = ys_and_heights.first().map(|g| g.1).unwrap_or(0.0);
        let last_line_height = ys_and_heights.last().map(|g| g.1).unwrap_or(0.0);

        // Heading (first line) should have taller glyphs
        assert!(
            first_line_height > last_line_height * 1.2,
            "heading glyph height ({}) should be >1.2x body glyph height ({})",
            first_line_height,
            last_line_height
        );
    }
}

#[test]
fn empty_document_renders_without_panic() {
    let doc = TextDocument::new();
    let flow = doc.snapshot_flow();

    let mut ts = make_typesetter();
    ts.layout_full(&flow);
    let frame = ts.render();

    // Should not panic; may produce glyphs for the empty initial block
    assert!(frame.atlas_width > 0 || frame.glyphs.is_empty());
}

#[test]
fn content_height_grows_with_content() {
    let doc = TextDocument::new();
    doc.set_plain_text("One line").unwrap();
    let flow1 = doc.snapshot_flow();

    let mut ts = make_typesetter();
    ts.layout_full(&flow1);
    let h1 = ts.content_height();

    let doc2 = TextDocument::new();
    doc2.set_plain_text("Line one.\n\nLine two.\n\nLine three.\n\nLine four.")
        .unwrap();
    let flow2 = doc2.snapshot_flow();
    ts.layout_full(&flow2);
    let h2 = ts.content_height();

    assert!(
        h2 > h1,
        "more content should produce greater height: {} vs {}",
        h1,
        h2
    );
}

// ── Cursor movement across frame (blockquote) boundaries ───────

#[test]
fn hit_test_below_blockquote_lands_on_block_after() {
    let doc = TextDocument::new();
    let op = doc
        .set_markdown("Before\n\n> Quoted text\n\nAfter")
        .unwrap();
    op.wait().unwrap();
    let flow = doc.snapshot_flow();

    let mut ts = make_typesetter();
    ts.layout_full(&flow);
    ts.render();

    // Hit test near the bottom of the viewport - "After" should be there
    let content_h = ts.content_height();
    let result = ts.hit_test(10.0, content_h - 5.0);
    assert!(
        result.is_some(),
        "hit test near bottom should return a result"
    );
    let hit = result.unwrap();
    // The last block "After" should be hit, not the blockquote's block
    assert!(
        hit.position >= "Before\n\n> Quoted text\n\n".len() - 5,
        "hit test at bottom should be in the 'After' block, got position {}",
        hit.position
    );
}

#[test]
fn caret_rect_moves_through_blockquote() {
    let doc = TextDocument::new();
    let op = doc.set_markdown("AB\n\n> CD\n\nEF").unwrap();
    op.wait().unwrap();
    let flow = doc.snapshot_flow();

    let mut ts = make_typesetter();
    ts.layout_full(&flow);
    ts.render();

    // Caret at position 0 ("A") should be at the top
    let rect_before = ts.caret_rect(0);
    // Find a position inside the blockquote
    // The blockquote text "CD" is somewhere in the middle of the document
    let rect_quote = ts.caret_rect(4);
    // Find a position after the blockquote
    let rect_after = ts.caret_rect(8);

    // Each successive caret should be below the previous
    assert!(
        rect_quote[1] > rect_before[1],
        "caret in blockquote ({}) should be below caret before ({})",
        rect_quote[1],
        rect_before[1]
    );
    assert!(
        rect_after[1] > rect_quote[1],
        "caret after blockquote ({}) should be below caret in blockquote ({})",
        rect_after[1],
        rect_quote[1]
    );
    // All carets should have valid dimensions
    assert!(rect_before[3] > 0.0);
    assert!(rect_quote[3] > 0.0);
    assert!(rect_after[3] > 0.0);
}

#[test]
fn selection_spanning_blockquote_boundary() {
    let doc = TextDocument::new();
    let op = doc.set_markdown("AB\n\n> CD\n\nEF").unwrap();
    op.wait().unwrap();
    let flow = doc.snapshot_flow();

    let mut ts = make_typesetter();
    ts.layout_full(&flow);

    // Select from "AB" through the blockquote into "EF"
    ts.set_cursor(&text_typeset::CursorDisplay {
        position: 0,
        anchor: 10,
        affinity: text_typeset::CursorAffinity::Downstream,
        visible: true,
        selected_cells: vec![],
    });
    let frame = ts.render();

    let selections: Vec<_> = frame
        .decorations
        .iter()
        .filter(|d| d.kind == text_typeset::DecorationKind::Selection)
        .collect();
    assert!(
        selections.len() >= 2,
        "cross-blockquote selection should produce multiple rects, got {}",
        selections.len()
    );
}

#[test]
fn ensure_caret_visible_inside_blockquote() {
    let doc = TextDocument::new();
    // Create enough content to require scrolling
    let mut md = String::new();
    for i in 0..20 {
        md.push_str(&format!("Paragraph {}.\n\n", i));
    }
    md.push_str("> Deep blockquote text\n\n");
    md.push_str("Final paragraph.");
    let op = doc.set_markdown(&md).unwrap();
    op.wait().unwrap();
    let flow = doc.snapshot_flow();

    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(800.0, 50.0); // very short viewport

    ts.layout_full(&flow);
    ts.render();

    // Find a position deep in the document via hit_test
    let h = ts.content_height();
    let hit = ts.hit_test(10.0, h - 5.0);
    assert!(hit.is_some(), "should find content at bottom");
    let deep_pos = hit.unwrap().position;

    ts.set_cursor(&text_typeset::CursorDisplay {
        position: deep_pos,
        anchor: deep_pos,
        affinity: text_typeset::CursorAffinity::Downstream,
        visible: true,
        selected_cells: vec![],
    });

    let result = ts.ensure_caret_visible();
    assert!(
        result.is_some(),
        "should need to scroll to reveal caret near bottom of long document"
    );
    assert!(result.unwrap() > 0.0, "scroll offset should be positive");
}

#[test]
fn caret_rect_after_edit_inside_blockquote() {
    let doc = TextDocument::new();
    let op = doc.set_markdown("Before\n\n> Short\n\nAfter").unwrap();
    op.wait().unwrap();
    let flow = doc.snapshot_flow();

    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(200.0, 600.0); // narrow viewport to force wrapping

    ts.layout_full(&flow);
    ts.render();

    // Find the "After" block position via hit_test at the bottom of the content
    let h = ts.content_height();
    let after_hit = ts.hit_test(10.0, h - 5.0);
    assert!(after_hit.is_some(), "should find the After block");
    let after_pos = after_hit.unwrap().position;
    let rect_before_edit = ts.caret_rect(after_pos);
    assert_caret_is_real(rect_before_edit, "After block before edit");
    assert!(
        rect_before_edit[3] > 0.0,
        "caret for 'After' should have valid height before edit"
    );

    // Edit the blockquote to make it much longer
    let doc2 = TextDocument::new();
    let op = doc2
        .set_markdown("Before\n\n> This is a much longer blockquote that takes more vertical space than the short one did before\n\nAfter")
        .unwrap();
    op.wait().unwrap();
    let flow2 = doc2.snapshot_flow();
    ts.layout_full(&flow2);
    ts.render();

    // Find the "After" position again in the new layout
    let h2 = ts.content_height();
    let after_hit2 = ts.hit_test(10.0, h2 - 5.0);
    assert!(
        after_hit2.is_some(),
        "should find After block in new layout"
    );
    let after_pos2 = after_hit2.unwrap().position;
    let rect_after_edit = ts.caret_rect(after_pos2);
    assert_caret_is_real(rect_after_edit, "After block after edit");

    assert!(
        rect_after_edit[3] > 0.0,
        "caret after edit should have valid height"
    );
    // The caret for "After" should have moved down since the blockquote grew
    assert!(
        rect_after_edit[1] > rect_before_edit[1],
        "caret for 'After' should move down after blockquote grows: {} -> {}",
        rect_before_edit[1],
        rect_after_edit[1]
    );
}

#[test]
fn scroll_to_position_inside_blockquote() {
    let doc = TextDocument::new();
    let mut md = String::new();
    for i in 0..15 {
        md.push_str(&format!("Paragraph {}.\n\n", i));
    }
    md.push_str("> Blockquote deep in document\n\n");
    let op = doc.set_markdown(&md).unwrap();
    op.wait().unwrap();
    let flow = doc.snapshot_flow();

    let mut ts = make_typesetter();
    ts.layout_full(&flow);
    ts.render();

    // Use hit_test to find a position deep in the document
    let h = ts.content_height();
    let hit = ts.hit_test(10.0, h - 5.0);
    assert!(hit.is_some());
    let deep_pos = hit.unwrap().position;

    let offset = ts.scroll_to_position(deep_pos);
    assert!(
        offset > 0.0,
        "scroll_to_position inside blockquote should produce positive offset"
    );
}

// ── Nested blockquote (nested frame) integration tests ─────────

#[test]
fn caret_rect_inside_nested_blockquote() {
    let doc = TextDocument::new();
    let op = doc
        .set_markdown("Before\n\n> Outer\n>\n> > Inner nested\n\nAfter\n")
        .unwrap();
    op.wait().unwrap();
    let flow = doc.snapshot_flow();

    let mut ts = make_typesetter();
    ts.layout_full(&flow);
    ts.render();

    // Find a position inside "Inner nested" via hit_test at approximate y
    let h = ts.content_height();
    // Scan vertically to find the inner block
    let mut inner_pos = None;
    for y_probe in (0..(h as i32)).step_by(2) {
        if let Some(hit) = ts.hit_test(80.0, y_probe as f32) {
            let rect = ts.caret_rect(hit.position);
            // Inner nested content should be indented more than outer
            if rect[0] > 50.0 && rect[3] > 0.0 {
                inner_pos = Some(hit.position);
                break;
            }
        }
    }
    let pos = inner_pos.expect("should find a position inside nested blockquote");
    let rect = ts.caret_rect(pos);
    assert!(
        rect[3] > 0.0,
        "caret inside nested blockquote should have valid height"
    );
    assert!(
        rect[1] > 0.0,
        "caret y inside nested blockquote should be positive"
    );
}

#[test]
fn hit_test_inside_nested_blockquote_returns_inner_block() {
    let doc = TextDocument::new();
    let op = doc
        .set_markdown("Before\n\n> Outer\n>\n> > Inner nested\n\nAfter\n")
        .unwrap();
    op.wait().unwrap();
    let flow = doc.snapshot_flow();

    let mut ts = make_typesetter();
    ts.layout_full(&flow);
    ts.render();

    // Find all distinct block_ids by scanning
    let h = ts.content_height();
    let mut block_ids = std::collections::HashSet::new();
    for y_probe in (0..(h as i32)).step_by(3) {
        if let Some(hit) = ts.hit_test(80.0, y_probe as f32) {
            block_ids.insert(hit.block_id);
        }
    }
    // Should have at least 4 blocks: Before, Outer, Inner nested, After
    assert!(
        block_ids.len() >= 4,
        "should find at least 4 distinct blocks (got {}): {:?}",
        block_ids.len(),
        block_ids
    );
}

#[test]
fn selection_spanning_nested_blockquote() {
    let doc = TextDocument::new();
    let op = doc
        .set_markdown("Before\n\n> Outer quote\n>\n> > Inner nested\n\nAfter\n")
        .unwrap();
    op.wait().unwrap();
    let flow = doc.snapshot_flow();

    let mut ts = make_typesetter();
    ts.layout_full(&flow);
    ts.render();

    // Find positions for "Outer" and "Inner" via hit test
    let h = ts.content_height();
    let mut positions = Vec::new();
    for y_probe in (0..(h as i32)).step_by(3) {
        if let Some(hit) = ts.hit_test(80.0, y_probe as f32)
            && positions
                .last()
                .is_none_or(|&(_, last_bid)| last_bid != hit.block_id)
        {
            positions.push((hit.position, hit.block_id));
        }
    }
    assert!(
        positions.len() >= 3,
        "should find at least 3 distinct block positions (got {})",
        positions.len()
    );

    let start = positions[0].0;
    let end = positions.last().unwrap().0 + 3;
    ts.set_cursor(&text_typeset::CursorDisplay {
        position: start,
        anchor: end,
        affinity: text_typeset::CursorAffinity::Downstream,
        visible: true,
        selected_cells: vec![],
    });
    let frame = ts.render();

    let sel_rects: Vec<_> = frame
        .decorations
        .iter()
        .filter(|d| d.kind == text_typeset::DecorationKind::Selection)
        .collect();
    assert!(
        sel_rects.len() >= 2,
        "selection spanning nested blockquote should produce multiple rects (got {})",
        sel_rects.len()
    );
}

#[test]
fn caret_rect_after_edit_inside_nested_blockquote() {
    let doc = TextDocument::new();
    let op = doc
        .set_markdown("Before\n\n> Outer\n>\n> > Short\n\nAfter\n")
        .unwrap();
    op.wait().unwrap();

    let mut ts = make_typesetter();
    ts.set_viewport(200.0, 600.0);

    let flow = doc.snapshot_flow();
    ts.layout_full(&flow);
    ts.render();

    // Find "After" position
    let h = ts.content_height();
    let after_hit = ts.hit_test(10.0, h - 5.0).expect("should find After block");
    let rect_before = ts.caret_rect(after_hit.position);
    assert_caret_is_real(rect_before, "After block before nested bq edit");

    // Edit the nested blockquote to have longer text
    let long_text = "Before\n\n> Outer\n>\n> > This is a much longer piece of text that should wrap and push the After block down significantly\n\nAfter\n";
    let op = doc.set_markdown(long_text).unwrap();
    op.wait().unwrap();
    let flow2 = doc.snapshot_flow();
    ts.layout_full(&flow2);
    ts.render();

    let h2 = ts.content_height();
    let after_hit2 = ts
        .hit_test(10.0, h2 - 5.0)
        .expect("should find After block after edit");
    let rect_after = ts.caret_rect(after_hit2.position);
    assert_caret_is_real(rect_after, "After block after nested bq edit");

    assert!(
        rect_after[1] > rect_before[1],
        "caret for After should move down after nested blockquote grows: {} -> {}",
        rect_before[1],
        rect_after[1]
    );
}

// ── Incremental relayout inside frame (typing simulation) ──────

#[test]
fn incremental_relayout_blockquote_shows_new_glyph() {
    let doc = TextDocument::new();
    let op = doc.set_markdown("Before\n\n> Hello\n\nAfter\n").unwrap();
    op.wait().unwrap();

    let mut ts = make_typesetter();
    let flow = doc.snapshot_flow();
    ts.layout_full(&flow);
    let frame1 = ts.render();
    let glyph_count_before = frame1.glyphs.len();

    // Find position inside the blockquote text "Hello"
    let h = ts.content_height();
    let mut bq_pos = None;
    for y in (0..(h as i32)).step_by(2) {
        if let Some(hit) = ts.hit_test(60.0, y as f32) {
            let rect = ts.caret_rect(hit.position);
            // Look for indented content (blockquote)
            if rect[0] > 20.0 {
                bq_pos = Some(hit.position);
                break;
            }
        }
    }
    let pos = bq_pos.expect("should find a position inside the blockquote");

    // Insert a character at that position using TextCursor
    let cursor = doc.cursor();
    cursor.set_position(pos, text_document::MoveMode::MoveAnchor);
    cursor.insert_text("X").unwrap();

    // Get the modified block snapshot and convert
    let block_snapshot = doc
        .snapshot_block_at_position(pos)
        .expect("should find block at cursor position");
    let block_params = text_typeset::bridge::convert_block(&block_snapshot);

    // Incremental relayout (NOT layout_full)
    ts.relayout_block(&block_params);
    let frame2 = ts.render();
    let glyph_count_after = frame2.glyphs.len();

    assert!(
        glyph_count_after > glyph_count_before,
        "after inserting a character in blockquote via relayout_block, \
         glyph count should increase: {} -> {}",
        glyph_count_before,
        glyph_count_after
    );
}

#[test]
fn render_block_only_for_frame_block_shows_new_glyph() {
    let doc = TextDocument::new();
    let op = doc.set_markdown("Before\n\n> Hello\n\nAfter\n").unwrap();
    op.wait().unwrap();

    let mut ts = make_typesetter();
    let flow = doc.snapshot_flow();
    ts.layout_full(&flow);
    let frame1 = ts.render();
    let glyph_count_before = frame1.glyphs.len();

    // Find a position inside the blockquote by scanning y and filtering
    // for blocks NOT in flow.blocks (i.e. frame-internal blocks)
    let h = ts.content_height();
    let mut bq_pos = None;
    let mut bq_block_id = None;
    for y in (0..(h as i32)).step_by(2) {
        if let Some(hit) = ts.hit_test(60.0, y as f32) {
            // block_visual_info returns None for frame-internal blocks
            if ts.block_visual_info(hit.block_id).is_none() {
                bq_pos = Some(hit.position);
                bq_block_id = Some(hit.block_id);
                break;
            }
        }
    }
    let pos = bq_pos.expect("should find a position inside the blockquote frame");
    let found_block_id = bq_block_id.unwrap();

    // Insert a character at that position using TextCursor
    let cursor = doc.cursor();
    cursor.set_position(pos, text_document::MoveMode::MoveAnchor);
    cursor.insert_text("X").unwrap();

    // Get the modified block snapshot and convert
    let block_snapshot = doc
        .snapshot_block_at_position(pos)
        .expect("should find block at cursor position");
    let block_params = text_typeset::bridge::convert_block(&block_snapshot);
    let block_id = block_params.block_id;
    assert_eq!(
        block_id, found_block_id,
        "block_id from snapshot should match the one found by hit_test"
    );

    ts.relayout_block(&block_params);

    // Use render_block_only (the app's fast path for typing)
    let frame2 = ts.render_block_only(block_id);
    let glyph_count_after = frame2.glyphs.len();

    assert!(
        glyph_count_after > glyph_count_before,
        "render_block_only for a frame block should show the new character: {} -> {}",
        glyph_count_before,
        glyph_count_after
    );
}

#[test]
fn cursor_reaches_all_positions_in_frame_block_after_insert() {
    let doc = TextDocument::new();
    let op = doc.set_markdown("Before\n\n> Hello\n\nAfter\n").unwrap();
    op.wait().unwrap();

    let mut ts = make_typesetter();
    let flow = doc.snapshot_flow();
    ts.layout_full(&flow);
    ts.render();

    // Find a frame-internal block
    let h = ts.content_height();
    let mut bq_pos = None;
    let mut bq_block_id = None;
    for y in (0..(h as i32)).step_by(2) {
        if let Some(hit) = ts.hit_test(60.0, y as f32)
            && ts.block_visual_info(hit.block_id).is_none()
        {
            bq_pos = Some(hit.position);
            bq_block_id = Some(hit.block_id);
            break;
        }
    }
    let pos = bq_pos.expect("should find a position inside the blockquote frame");
    let block_id = bq_block_id.unwrap();

    // Record the block position range before insert
    // "Hello" = 5 chars, so block should have positions block_pos..block_pos+5
    let rect_before_end = ts.caret_rect(pos + 5);
    assert!(
        rect_before_end[2] > 0.0,
        "caret should be visible at end of 'Hello'"
    );

    // Insert "X" at position `pos` (start of block text) making "XHello"
    let cursor = doc.cursor();
    cursor.set_position(pos, text_document::MoveMode::MoveAnchor);
    cursor.insert_text("X").unwrap();

    let block_snapshot = doc
        .snapshot_block_at_position(pos)
        .expect("should find block");
    let block_params = text_typeset::bridge::convert_block(&block_snapshot);
    assert_eq!(block_params.block_id, block_id);

    ts.relayout_block(&block_params);
    ts.render();

    // After insert, block text is "XHello" (6 chars)
    // Every position from block_pos to block_pos+6 should have a valid caret
    let block_position = block_params.position;
    let text_len = block_params.text.chars().count();
    for offset in 0..=text_len {
        let abs_pos = block_position + offset;
        let rect = ts.caret_rect(abs_pos);
        assert!(
            rect[2] > 0.0 && rect[3] > 0.0,
            "caret_rect at offset {} (abs pos {}) should be valid, got {:?}",
            offset,
            abs_pos,
            rect
        );
    }

    // Also verify text-document's cursor can reach every position
    // via set_position (simulating what the app does on arrow keys)
    let cursor2 = doc.cursor();
    for offset in 0..=text_len {
        let abs_pos = block_position + offset;
        cursor2.set_position(abs_pos, text_document::MoveMode::MoveAnchor);
        let actual = cursor2.position();
        assert_eq!(
            actual, abs_pos,
            "text-document cursor should reach position {} but got {}",
            abs_pos, actual
        );
    }

    // And verify MoveRight can step through all positions in the block
    cursor2.set_position(block_position, text_document::MoveMode::MoveAnchor);
    for offset in 1..=text_len {
        cursor2.move_position(
            text_document::MoveOperation::Right,
            text_document::MoveMode::MoveAnchor,
            1,
        );
        let expected = block_position + offset;
        let actual = cursor2.position();
        assert_eq!(
            actual, expected,
            "MoveRight step {} should reach position {} but cursor is at {}",
            offset, expected, actual
        );
    }

    // Verify that the document end is also reachable (position == doc length)
    let doc_len = doc.character_count();
    cursor2.set_position(doc_len, text_document::MoveMode::MoveAnchor);
    assert_eq!(
        cursor2.position(),
        doc_len,
        "cursor should reach document end after insert"
    );
    // And length - 1
    cursor2.set_position(doc_len - 1, text_document::MoveMode::MoveAnchor);
    assert_eq!(
        cursor2.position(),
        doc_len - 1,
        "cursor should reach document length - 1 after insert"
    );
    // Caret should be valid at doc_len - 1
    let rect = ts.caret_rect(doc_len - 1);
    assert!(
        rect[3] > 0.0,
        "caret at doc_len-1 ({}) should be valid, got {:?}",
        doc_len - 1,
        rect
    );
}

#[test]
fn frame_block_wrapping_after_insert_grows_frame() {
    // Use a narrow viewport to make wrapping happen quickly
    let doc = TextDocument::new();
    let op = doc.set_markdown("Before\n\n> Hello\n\nAfter\n").unwrap();
    op.wait().unwrap();

    let mut ts = make_typesetter();
    ts.set_viewport(200.0, 600.0); // narrow: makes wrapping likely
    let flow = doc.snapshot_flow();
    ts.layout_full(&flow);
    ts.render();

    // Find the frame-internal block
    let h = ts.content_height();
    let mut bq_pos = None;
    let mut bq_block_id = None;
    for y in (0..(h as i32)).step_by(2) {
        if let Some(hit) = ts.hit_test(60.0, y as f32)
            && ts.block_visual_info(hit.block_id).is_none()
        {
            bq_pos = Some(hit.position);
            bq_block_id = Some(hit.block_id);
            break;
        }
    }
    let pos = bq_pos.expect("should find a position inside the blockquote frame");
    let block_id = bq_block_id.unwrap();

    let height_before = ts.content_height();

    // Insert enough text to force wrapping (30 wide chars)
    let cursor = doc.cursor();
    cursor.set_position(pos, text_document::MoveMode::MoveAnchor);
    cursor
        .insert_text("ABCDEFGHIJKLMNOPQRSTUVWXYZ1234")
        .unwrap();

    let block_snapshot = doc
        .snapshot_block_at_position(pos)
        .expect("should find block");
    let block_params = text_typeset::bridge::convert_block(&block_snapshot);
    assert_eq!(block_params.block_id, block_id);

    ts.relayout_block(&block_params);
    ts.render();

    let height_after = ts.content_height();

    // The text should now wrap to multiple lines, making the frame taller
    assert!(
        height_after > height_before,
        "content height should increase after inserting enough text to wrap: {} -> {}",
        height_before,
        height_after
    );

    // Every position in the block should have a valid caret
    let text_len = block_params.text.chars().count();
    let block_position = block_params.position;
    for offset in 0..=text_len {
        let abs_pos = block_position + offset;
        let rect = ts.caret_rect(abs_pos);
        assert!(
            rect[3] > 0.0,
            "caret at offset {} (abs pos {}) should be valid after wrapping, got {:?}",
            offset,
            abs_pos,
            rect
        );
    }
}

/// Compare initial frame block layout with incremental relayout.
/// Checks that the line breaking width (and hence line count) matches.
#[test]
fn frame_block_relayout_preserves_line_structure() {
    let doc = TextDocument::new();
    let long_text = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let md = format!("Before\n\n> {}\n\nAfter\n", long_text);
    let op = doc.set_markdown(&md).unwrap();
    op.wait().unwrap();

    let mut ts = make_typesetter();
    ts.set_viewport(200.0, 600.0);
    let flow = doc.snapshot_flow();
    ts.layout_full(&flow);
    ts.render();

    // Find the frame-internal block and its line count from initial layout
    let h = ts.content_height();
    let mut bq_block_id = None;
    for y in (0..(h as i32)).step_by(2) {
        if let Some(hit) = ts.hit_test(60.0, y as f32)
            && ts.block_visual_info(hit.block_id).is_none()
        {
            bq_block_id = Some(hit.block_id);
            break;
        }
    }
    let _block_id = bq_block_id.expect("should find blockquote block");

    // Snapshot the block from text-document (without editing) and relayout
    // This tests whether relayout with the SAME text produces the SAME layout
    let block_snapshot = doc
        .snapshot_block_at_position(7) // position 7 is inside the blockquote
        .expect("should find block");
    let block_params = text_typeset::bridge::convert_block(&block_snapshot);

    // Get the caret positions at start and end BEFORE relayout
    let text_len_chars = block_params.text.chars().count();
    let caret_before_start = ts.caret_rect(block_params.position);
    let caret_before_end = ts.caret_rect(block_params.position + text_len_chars);

    // Now relayout with same text
    ts.relayout_block(&block_params);
    ts.render();

    let caret_after_start = ts.caret_rect(block_params.position);
    let caret_after_end = ts.caret_rect(block_params.position + text_len_chars);

    // Start caret should be at the same position
    assert!(
        (caret_before_start[0] - caret_after_start[0]).abs() < 1.0
            && (caret_before_start[1] - caret_after_start[1]).abs() < 1.0,
        "start caret should not move: {:?} -> {:?}",
        caret_before_start,
        caret_after_start
    );

    // End caret should be at the same position (same line structure)
    assert!(
        (caret_before_end[0] - caret_after_end[0]).abs() < 1.0
            && (caret_before_end[1] - caret_after_end[1]).abs() < 1.0,
        "end caret should not move after relayout with same text: {:?} -> {:?}",
        caret_before_end,
        caret_after_end
    );
}

/// Simulate the app's actual editing loop: char-by-char insert with
/// render_block_only, verifying the frame grows and glyphs appear.
#[test]
fn render_block_only_frame_grows_on_wrap() {
    let doc = TextDocument::new();
    let op = doc.set_markdown("X\n\n> Hello\n\nY\n").unwrap();
    op.wait().unwrap();

    let mut ts = make_typesetter();
    ts.set_viewport(200.0, 600.0);
    let flow = doc.snapshot_flow();
    ts.layout_full(&flow);
    ts.render();

    // Find the frame-internal block
    let h = ts.content_height();
    let mut bq_pos = None;
    for y in (0..(h as i32)).step_by(2) {
        if let Some(hit) = ts.hit_test(60.0, y as f32)
            && ts.block_visual_info(hit.block_id).is_none()
        {
            bq_pos = Some(hit.position);
            break;
        }
    }
    let pos = bq_pos.expect("should find frame block");

    let height_before = ts.content_height();
    let glyphs_before = ts.render().glyphs.len();

    // Insert 30 chars one at a time (like real typing), using render_block_only
    let cursor = doc.cursor();
    for i in 0..30 {
        let insert_pos = pos + i;
        cursor.set_position(insert_pos, text_document::MoveMode::MoveAnchor);
        cursor.insert_text("W").unwrap();

        let snap = doc
            .snapshot_block_at_position(insert_pos)
            .expect("block snap");
        let params = text_typeset::bridge::convert_block(&snap);
        ts.relayout_block(&params);
        ts.render_block_only(params.block_id);
    }

    let height_after = ts.content_height();

    // Full render to get clean glyph count
    let glyphs_final = ts.render().glyphs.len();

    assert!(
        height_after > height_before,
        "frame should have grown: {} -> {}",
        height_before,
        height_after
    );
    assert!(
        glyphs_final > glyphs_before,
        "should have more glyphs: {} -> {}",
        glyphs_before,
        glyphs_final
    );
}

/// After inserting text in a frame block, caret_rect for positions inside the
/// frame should point inside the frame - not jump to a subsequent top-level block.
#[test]
fn caret_rect_stays_in_frame_after_insert() {
    let doc = TextDocument::new();
    // "X" (block 0), frame with "Hello" (block inside frame), "Y" (block after frame)
    let op = doc.set_markdown("X\n\n> Hello\n\nY\n").unwrap();
    op.wait().unwrap();

    let mut ts = make_typesetter();
    let flow = doc.snapshot_flow();
    ts.layout_full(&flow);
    ts.render();

    // Find the frame-internal block
    let h = ts.content_height();
    let mut frame_block_pos = None;
    for y in (0..(h as i32)).step_by(2) {
        if let Some(hit) = ts.hit_test(60.0, y as f32)
            && ts.block_visual_info(hit.block_id).is_none()
        {
            frame_block_pos = Some((hit.position, hit.block_id));
            break;
        }
    }
    let (bq_pos, _) = frame_block_pos.expect("should find blockquote block");

    // Get the "Y" block position by scanning below the frame
    let mut y_block_pos = None;
    for y in (0..(h as i32)).step_by(2) {
        if let Some(hit) = ts.hit_test(60.0, y as f32) {
            // Top-level block below the frame block
            if hit.position > bq_pos && ts.block_visual_info(hit.block_id).is_some() {
                y_block_pos = Some(hit.position);
                break;
            }
        }
    }
    let y_pos = y_block_pos.expect("should find Y block");
    let y_block_caret_before = ts.caret_rect(y_pos);

    // Insert "Z" at the start of "Hello" (bq_pos, same as the passing tests)
    let cursor = doc.cursor();
    cursor.set_position(bq_pos, text_document::MoveMode::MoveAnchor);
    cursor.insert_text("Z").unwrap();

    let snap = doc
        .snapshot_block_at_position(bq_pos)
        .expect("should find frame block");
    assert!(
        snap.text.contains("Z"),
        "frame block text should contain 'Z', got {:?}",
        snap.text
    );

    let params = text_typeset::bridge::convert_block(&snap);
    ts.relayout_block(&params);
    ts.render();

    // The caret at end of frame block should be above the "Y" block
    let frame_text_len = params.text.chars().count();
    let end_of_frame = params.position + frame_text_len;
    let frame_end_caret = ts.caret_rect(end_of_frame);

    assert!(
        frame_end_caret[1] < y_block_caret_before[1],
        "caret at end of frame block (pos {}) should be above 'Y' block: \
         frame_caret_y={} should be < y_block_y={}",
        end_of_frame,
        frame_end_caret[1],
        y_block_caret_before[1]
    );
}

/// Reproduce: repeated Enter + char at the end of a frame block eventually
/// causes the cursor to escape the frame.
///
/// Steps:
/// 1. Position cursor at end of the last block in a blockquote frame
/// 2. Enter -> "a" -> Enter -> "b" -> Enter -> "c" -> Enter
/// 3. BUG: after the 4th Enter, cursor jumps outside the frame to the
///    start of the next top-level block.
#[test]
fn repeated_enter_at_end_of_frame_stays_inside() {
    let doc = TextDocument::new();
    let op = doc.set_markdown("Before\n\n> Hello\n\nAfter\n").unwrap();
    op.wait().unwrap();

    let mut ts = make_typesetter();
    let flow = doc.snapshot_flow();
    ts.layout_full(&flow);
    ts.render();

    // Find the frame-internal block (blockquote "Hello")
    let h = ts.content_height();
    let mut bq_pos = None;
    for y in (0..(h as i32)).step_by(2) {
        if let Some(hit) = ts.hit_test(60.0, y as f32)
            && ts.block_visual_info(hit.block_id).is_none()
        {
            bq_pos = Some(hit.position);
            break;
        }
    }
    let bq_start = bq_pos.expect("should find position inside blockquote frame");

    // Move cursor to end of "Hello"
    let bq_snap = doc
        .snapshot_block_at_position(bq_start)
        .expect("should find blockquote block");
    let bq_text_len = bq_snap.text.chars().count();
    let end_pos = bq_snap.position + bq_text_len;

    let cursor = doc.cursor();
    cursor.set_position(end_pos, text_document::MoveMode::MoveAnchor);

    /// Check that the cursor is inside the frame by doing a hit_test at the
    /// caret position and verifying the hit block has no block_visual_info
    /// (frame-internal blocks are not in the top-level flow).
    fn assert_cursor_in_frame(ts: &mut Typesetter, cursor_pos: usize, label: &str) {
        let rect = ts.caret_rect(cursor_pos);
        assert_caret_is_real(rect, label);

        // hit_test at the caret should return a frame-internal block
        let hx = rect[0].max(1.0);
        let hy = rect[1] + rect[3] * 0.5;
        if let Some(hit) = ts.hit_test(hx, hy) {
            assert!(
                ts.block_visual_info(hit.block_id).is_none(),
                "{}: cursor at pos {} landed on top-level block {} instead of frame block. \
                 caret_rect={:?}",
                label,
                cursor_pos,
                hit.block_id,
                rect
            );
        }
    }

    // Verify we start inside the frame
    assert_cursor_in_frame(&mut ts, end_pos, "initial: end of Hello");

    // Repeatedly: Enter + char, checking caret stays inside the frame each time
    let chars = ['a', 'b', 'c'];
    for (i, ch) in chars.iter().enumerate() {
        // Enter
        cursor.insert_block().unwrap();
        let flow = doc.snapshot_flow();
        ts.layout_full(&flow);
        ts.render();

        let pos_after_enter = cursor.position();
        assert_cursor_in_frame(
            &mut ts,
            pos_after_enter,
            &format!("after Enter #{} (before '{}')", i + 1, ch),
        );

        // Insert char
        cursor.insert_text(&ch.to_string()).unwrap();
        let flow = doc.snapshot_flow();
        ts.layout_full(&flow);
        ts.render();

        let pos_after_char = cursor.position();
        assert_cursor_in_frame(
            &mut ts,
            pos_after_char,
            &format!("after inserting '{}'", ch),
        );
    }

    // The 4th Enter (the one that triggers the bug)
    cursor.insert_block().unwrap();
    let flow = doc.snapshot_flow();

    ts.layout_full(&flow);
    ts.render();

    let pos_final = cursor.position();

    // Verify the new block was created inside the frame, not as a top-level block.
    // The flow snapshot should have the new empty block inside the frame.
    // Verify the cursor block is inside the blockquote frame, not a top-level block.
    //
    // The flow snapshot after 4th Enter shows:
    //   [0] Block "Before"
    //   [1] Frame (blockquote)
    //     [0] Block "Hello"
    //     [1] Block "a"
    //     [2] Block "b"
    //     [3] Block "c"
    //   [2] Block "Af"    <-- cursor lands here (BUG: should be inside frame)
    //   [3] Block "ter"
    //
    // text-document's insert_block() at the end of "c" splits the "After"
    // block instead of creating a new empty block inside the frame.

    // Check the flow snapshot: the cursor's block should be inside the frame
    let flow_final = doc.snapshot_flow();
    let cursor_block_in_frame = flow_final.elements.iter().any(|elem| {
        if let text_document::FlowElementSnapshot::Frame(f) = elem {
            f.elements.iter().any(|inner| {
                if let text_document::FlowElementSnapshot::Block(b) = inner {
                    b.position <= pos_final && pos_final <= b.position + b.text.chars().count()
                } else {
                    false
                }
            })
        } else {
            false
        }
    });

    assert!(
        cursor_block_in_frame,
        "BUG (text-document): after 4th Enter at end of blockquote frame, \
         cursor position {} is not inside the frame in the flow snapshot. \
         insert_block() at the end of the last block in a frame should create \
         a new block inside the frame, not split the next top-level block.",
        pos_final
    );
}

#[test]
fn markdown_table_cells_render_at_distinct_positions() {
    let doc = TextDocument::new();
    let md = "\
| Column A | Column B |
|----------|----------|
| Cell one | Cell two with **bold** |
| Cell three | Cell four with `code` and *italic* |

Final paragraph after all elements.";
    let op = doc.set_markdown(md).unwrap();
    op.wait().unwrap();
    let flow = doc.snapshot_flow();

    // The flow must contain a Table element (not a single concatenated block)
    let table_count = flow
        .elements
        .iter()
        .filter(|e| matches!(e, text_document::FlowElementSnapshot::Table(_)))
        .count();
    assert!(
        table_count >= 1,
        "markdown table should produce a Table flow element, got {} tables out of {} elements",
        table_count,
        flow.elements.len()
    );

    let mut ts = make_typesetter();
    ts.layout_full(&flow);
    let frame = ts.render();

    // The table should produce border decorations
    let table_borders = frame
        .decorations
        .iter()
        .filter(|d| d.kind == text_typeset::DecorationKind::TableBorder)
        .count();
    assert!(
        table_borders > 0,
        "markdown table should produce table border decorations"
    );

    // Collect unique y positions - table rows + final paragraph need 4+ distinct y lines
    let mut ys: Vec<f32> = frame.glyphs.iter().map(|g| g.screen[1]).collect();
    ys.sort_by(|a, b| a.partial_cmp(b).unwrap());
    ys.dedup_by(|a, b| (*a - *b).abs() < 1.0);
    assert!(
        ys.len() >= 4,
        "table rows + paragraph should produce 4+ distinct y lines, got {}",
        ys.len()
    );
}

// ── Typing through table cells ──────────────────────────────────

#[test]
fn typing_in_all_table_cells_keeps_caret_inside_cell() {
    let doc = TextDocument::new();
    let md = "\
| Alpha | Beta |
|-------|------|
| Gamma | Delta |";
    let op = doc.set_markdown(md).unwrap();
    op.wait().unwrap();

    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);
    // Narrow viewport so text wraps inside cells
    ts.set_viewport(200.0, 600.0);

    let flow = doc.snapshot_flow();
    ts.layout_full(&flow);

    // Collect cell block_ids in row-major order from the flow snapshot.
    let mut cell_block_ids: Vec<usize> = Vec::new();
    for element in &flow.elements {
        if let text_document::FlowElementSnapshot::Table(table) = element {
            for cell in &table.cells {
                for block in &cell.blocks {
                    cell_block_ids.push(block.block_id);
                }
            }
        }
    }
    assert_eq!(
        cell_block_ids.len(),
        4,
        "2x2 table should have 4 cell blocks, got {}",
        cell_block_ids.len()
    );

    // Determine the right boundary for each column.
    // col 0 right edge = col 1 left edge (the next column's caret start x).
    // col 1 right edge = viewport width (it's the last column).
    let col1_start_x = ts.caret_rect(find_block_text_start(&flow, cell_block_ids[1]))[0];

    let cursor = doc.cursor();

    // Type sequences of "char space" -- single characters separated by spaces.
    // This pattern creates frequent break opportunities at each space,
    // so the caret should wrap to the next line before overflowing the cell.
    // 40 "words" for cell 0, 10 for cells 1-3.
    let chars_40 = "a b c d e f g h i j k l m n o p q r s t \
                    u v w x y z A B C D E F G H I J K L M N";
    let chars_10 = "a b c d e f g h i j";

    for (cell_idx, &block_id) in cell_block_ids.iter().enumerate() {
        // Re-snapshot to get current positions (earlier typing shifted them).
        let flow = doc.snapshot_flow();
        ts.layout_full(&flow);

        // Move cursor to the end of this cell's current text.
        let end_pos = find_block_text_end(&flow, block_id);
        cursor.set_position(end_pos, text_document::MoveMode::MoveAnchor);

        // Discover the cell's column boundaries.
        let cell_left_x = ts.caret_rect(find_block_text_start(&flow, block_id))[0];
        // For column 0 cells, right boundary = col 1 start x.
        // For column 1 cells, right boundary = viewport width.
        let is_col0 = cell_idx == 0 || cell_idx == 2;
        let cell_right_x = if is_col0 { col1_start_x } else { 200.0 };

        // Type the text character by character, prepending a space separator.
        let text = if cell_idx == 0 { chars_40 } else { chars_10 };

        for (char_idx, ch) in std::iter::once(' ').chain(text.chars()).enumerate() {
            cursor.insert_text(&ch.to_string()).unwrap();

            let flow = doc.snapshot_flow();
            ts.layout_full(&flow);

            let pos = cursor.position();
            let rect = ts.caret_rect(pos);

            // 1. Caret must be real (not the fallback sentinel).
            //    The sentinel has x=0 and y=-scroll_offset; catch it explicitly.
            let is_sentinel = rect[0] == 0.0 && rect[2] == 2.0 && rect[3] == 16.0;
            assert!(
                !is_sentinel,
                "cell[{}] char {} '{}': caret fell back to sentinel at pos {}. caret={:?}",
                cell_idx, char_idx, ch, pos, rect
            );
            assert_caret_is_real(
                rect,
                &format!("cell[{}] char {} '{}'", cell_idx, char_idx, ch),
            );

            // 2. Caret x must stay within the cell's column boundaries.
            //    Left: must not jump before the cell's left edge.
            assert!(
                rect[0] >= cell_left_x - 2.0,
                "cell[{}] char {} '{}': caret x ({}) jumped left of cell ({}). caret={:?}",
                cell_idx,
                char_idx,
                ch,
                rect[0],
                cell_left_x,
                rect
            );
            //    Right: must not overflow into the adjacent cell.
            assert!(
                rect[0] < cell_right_x,
                "cell[{}] char {} '{}': caret x ({}) overflowed past cell right edge ({}). \
                 caret={:?}",
                cell_idx,
                char_idx,
                ch,
                rect[0],
                cell_right_x,
                rect
            );

            // 3. Caret y must be at or below the cell's row top.
            let row_top_y = ts.caret_rect(find_block_text_start(&flow, block_id))[1];
            assert!(
                rect[1] >= row_top_y - 1.0,
                "cell[{}] char {} '{}': caret y ({}) is above row top ({}). caret={:?}",
                cell_idx,
                char_idx,
                ch,
                rect[1],
                row_top_y,
                rect
            );

            // 4. Hit-test at the caret must resolve to a table cell (not escape).
            let hx = rect[0].max(1.0);
            let hy = rect[1] + rect[3] * 0.5;
            if let Some(hit) = ts.hit_test(hx, hy) {
                assert!(
                    ts.is_block_in_table(hit.block_id),
                    "cell[{}] char {} '{}': caret escaped the table! \
                     hit block {} is not in table. caret={:?}",
                    cell_idx,
                    char_idx,
                    ch,
                    hit.block_id,
                    rect
                );
            }
        }
    }

    fn find_block_text_start(flow: &text_document::FlowSnapshot, block_id: usize) -> usize {
        for element in &flow.elements {
            if let text_document::FlowElementSnapshot::Table(table) = element {
                for cell in &table.cells {
                    for block in &cell.blocks {
                        if block.block_id == block_id {
                            return block.position;
                        }
                    }
                }
            }
        }
        panic!("block_id {} not found in flow snapshot", block_id);
    }

    fn find_block_text_end(flow: &text_document::FlowSnapshot, block_id: usize) -> usize {
        for element in &flow.elements {
            if let text_document::FlowElementSnapshot::Table(table) = element {
                for cell in &table.cells {
                    for block in &cell.blocks {
                        if block.block_id == block_id {
                            return block.position + block.text.len();
                        }
                    }
                }
            }
        }
        panic!("block_id {} not found in flow snapshot", block_id);
    }
}
