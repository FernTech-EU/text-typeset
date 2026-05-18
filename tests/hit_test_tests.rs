mod helpers;
use helpers::{
    NOTO_SANS, Typesetter, assert_caret_is_real, make_block, make_block_at, make_cell_at,
    make_typesetter,
};

use text_typeset::layout::block::{BlockLayoutParams, FragmentParams};
use text_typeset::layout::frame::{FrameBorderStyle, FrameLayoutParams, FramePosition};
use text_typeset::layout::paragraph::Alignment;
use text_typeset::layout::table::TableLayoutParams;
use text_typeset::{DecorationKind, HitRegion, UnderlineStyle, VerticalAlignment};

#[test]
fn hit_test_on_text_returns_some() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "Hello world")]);
    ts.render(); // ensure layout is computed

    // Hit somewhere in the middle of the text
    let result = ts.hit_test(40.0, 10.0);
    assert!(result.is_some(), "hit test on text should return Some");
}

#[test]
fn hit_test_returns_correct_block_id() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "First"), make_block(2, "Second")]);

    // First block should be near y=0
    let r1 = ts.hit_test(10.0, 5.0);
    assert!(r1.is_some());
    assert_eq!(r1.unwrap().block_id, 1);

    // Second block should be below the first
    let height = ts.content_height();
    let r2 = ts.hit_test(10.0, height - 5.0);
    assert!(r2.is_some());
    assert_eq!(r2.unwrap().block_id, 2);
}

#[test]
fn hit_test_position_increases_left_to_right() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "ABCDEFGHIJ")]);

    let r_left = ts.hit_test(5.0, 10.0);
    let r_mid = ts.hit_test(60.0, 10.0);
    let r_right = ts.hit_test(120.0, 10.0);

    assert!(r_left.is_some());
    assert!(r_mid.is_some());
    assert!(r_right.is_some());

    let pos_left = r_left.unwrap().position;
    let pos_mid = r_mid.unwrap().position;
    let pos_right = r_right.unwrap().position;

    assert!(
        pos_left <= pos_mid && pos_mid <= pos_right,
        "positions should increase left-to-right: {} <= {} <= {}",
        pos_left,
        pos_mid,
        pos_right
    );
}

#[test]
fn hit_test_past_line_end() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "Hi")]);

    // Far to the right of a short line
    let result = ts.hit_test(700.0, 10.0);
    assert!(result.is_some());
    let result = result.unwrap();
    assert!(
        matches!(result.region, HitRegion::PastLineEnd),
        "far right should be PastLineEnd, got {:?}",
        std::mem::discriminant(&result.region)
    );
}

#[test]
fn hit_test_left_margin_region() {
    let mut ts = make_typesetter();
    let mut block = make_block(1, "Hello");
    block.left_margin = 50.0;
    ts.layout_blocks(vec![block]);

    // Click in the left margin area
    let result = ts.hit_test(10.0, 10.0);
    assert!(result.is_some());
    assert!(
        matches!(result.unwrap().region, HitRegion::LeftMargin),
        "click in left margin should return LeftMargin region"
    );
}

#[test]
fn caret_rect_at_position_zero() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "Hello")]);

    let rect = ts.caret_rect(0);
    assert_caret_is_real(rect, "position 0");
    // Caret at position 0 should be near x=0
    assert!(
        rect[0] < 10.0,
        "caret x at pos 0 should be near 0, got {}",
        rect[0]
    );
    assert!(rect[2] > 0.0, "caret should have positive width");
    assert!(rect[3] > 0.0, "caret should have positive height");
}

#[test]
fn caret_rect_moves_right_with_position() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "ABCDEF")]);

    let rect0 = ts.caret_rect(0);
    let rect3 = ts.caret_rect(3);
    let rect6 = ts.caret_rect(6);
    assert_caret_is_real(rect0, "position 0");
    assert_caret_is_real(rect3, "position 3");
    assert_caret_is_real(rect6, "position 6");

    assert!(
        rect3[0] > rect0[0],
        "caret at pos 3 ({}) should be right of pos 0 ({})",
        rect3[0],
        rect0[0]
    );
    assert!(
        rect6[0] > rect3[0],
        "caret at pos 6 ({}) should be right of pos 3 ({})",
        rect6[0],
        rect3[0]
    );
}

#[test]
fn caret_rect_height_matches_line_height() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "Hello")]);

    let rect = ts.caret_rect(0);
    // Caret height should be roughly one line height (ascent + descent + leading)
    // For 16px Noto Sans, line height is ~20px
    assert!(
        rect[3] > 10.0 && rect[3] < 40.0,
        "caret height {} should be reasonable for 16px font",
        rect[3]
    );
}

#[test]
fn hit_test_with_scroll_offset() {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(800.0, 600.0);

    let blocks: Vec<_> = (0..10)
        .map(|i| {
            let mut b = make_block(i, &format!("Paragraph {i} text."));
            b.position = i * 20; // give each block a different document position
            b
        })
        .collect();
    ts.layout_blocks(blocks);

    // Without scroll, hit at y=5 should be in block 0
    let r0 = ts.hit_test(10.0, 5.0);
    assert!(r0.is_some());
    assert_eq!(r0.unwrap().block_id, 0);

    // With scroll offset, the same screen y maps to a different document position
    ts.set_scroll_offset(100.0);
    let r_scrolled = ts.hit_test(10.0, 5.0);
    assert!(r_scrolled.is_some());
    // Should be a different block (further down in the document)
    assert_ne!(
        r_scrolled.unwrap().block_id,
        0,
        "scrolled hit test should be in a different block"
    );
}

// ── Cursor and selection tests ──────────────────────────────────

#[test]
fn cursor_produces_caret_decoration() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "Hello world")]);
    ts.set_cursor(&text_typeset::CursorDisplay {
        position: 5,
        anchor: 5,
        visible: true,
        selected_cells: vec![],
    });
    let frame = ts.render();

    let carets: Vec<_> = frame
        .decorations
        .iter()
        .filter(|d| d.kind == text_typeset::DecorationKind::Cursor)
        .collect();
    assert_eq!(carets.len(), 1, "should have exactly one cursor caret");
    assert!(carets[0].rect[2] > 0.0, "caret should have positive width");
    assert!(carets[0].rect[3] > 0.0, "caret should have positive height");
}

#[test]
fn invisible_cursor_produces_no_caret() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "Hello")]);
    ts.set_cursor(&text_typeset::CursorDisplay {
        position: 3,
        anchor: 3,
        visible: false, // blink off
        selected_cells: vec![],
    });
    let frame = ts.render();

    let carets: Vec<_> = frame
        .decorations
        .iter()
        .filter(|d| d.kind == text_typeset::DecorationKind::Cursor)
        .collect();
    assert!(
        carets.is_empty(),
        "invisible cursor should produce no caret"
    );
}

#[test]
fn selection_produces_highlight_rects() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "Hello world")]);
    ts.set_cursor(&text_typeset::CursorDisplay {
        position: 0,
        anchor: 5, // select "Hello"
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
    // Selection should have positive width and height
    for sel in &selections {
        assert!(sel.rect[2] > 0.0, "selection width should be positive");
        assert!(sel.rect[3] > 0.0, "selection height should be positive");
    }
}

#[test]
fn no_selection_when_anchor_equals_position() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "Hello")]);
    ts.set_cursor(&text_typeset::CursorDisplay {
        position: 3,
        anchor: 3, // no selection
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
        selections.is_empty(),
        "no selection when anchor == position"
    );
}

#[test]
fn multi_line_selection_extends_to_viewport_width() {
    let mut ts = make_typesetter(); // 800x600
    ts.layout_blocks(vec![
        make_block(1, "Short line."),
        make_block(2, "Another line."),
    ]);
    // Select across both blocks (position 0 in block 1 to end of block 2)
    ts.set_cursor(&text_typeset::CursorDisplay {
        position: 0,
        anchor: 24, // past both blocks
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
        "multi-block selection should produce at least 2 selection rects, got {}",
        selections.len()
    );

    // The first selection rect (first line) should extend to viewport width (800)
    // because the selection continues to the next line
    let first_sel = &selections[0];
    let sel_right_edge = first_sel.rect[0] + first_sel.rect[2];
    assert!(
        sel_right_edge > 700.0,
        "first line selection should extend to near viewport width (800), got right edge at {}",
        sel_right_edge
    );
}

#[test]
fn single_line_selection_does_not_extend() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "Hello world, this is text.")]);
    // Select just "world" — single line, doesn't continue to next line
    ts.set_cursor(&text_typeset::CursorDisplay {
        position: 6,
        anchor: 11,
        visible: true,
        selected_cells: vec![],
    });
    let frame = ts.render();

    let selections: Vec<_> = frame
        .decorations
        .iter()
        .filter(|d| d.kind == text_typeset::DecorationKind::Selection)
        .collect();

    assert!(!selections.is_empty());
    let sel = &selections[0];
    let sel_width = sel.rect[2];
    // "world" is ~5 characters wide at 16px ≈ 40-50px, NOT 800px
    assert!(
        sel_width < 200.0,
        "single-line selection should NOT extend to viewport width, got width {}",
        sel_width
    );
}

#[test]
fn multiple_cursors() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "ABCDEFGHIJ")]);
    ts.set_cursors(&[
        text_typeset::CursorDisplay {
            position: 2,
            anchor: 2,
            visible: true,
            selected_cells: vec![],
        },
        text_typeset::CursorDisplay {
            position: 7,
            anchor: 7,
            visible: true,
            selected_cells: vec![],
        },
    ]);
    let frame = ts.render();

    let carets: Vec<_> = frame
        .decorations
        .iter()
        .filter(|d| d.kind == text_typeset::DecorationKind::Cursor)
        .collect();
    assert_eq!(carets.len(), 2, "should have two cursor carets");

    // Second caret should be to the right of the first
    assert!(
        carets[1].rect[0] > carets[0].rect[0],
        "second caret x ({}) should be > first caret x ({})",
        carets[1].rect[0],
        carets[0].rect[0]
    );
}

// ── Scrolling tests ─────────────────────────────────────────────

#[test]
fn block_visual_info_returns_data() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "Hello")]);

    let info = ts.block_visual_info(1);
    assert!(info.is_some());
    let info = info.unwrap();
    assert_eq!(info.block_id, 1);
    assert!(info.height > 0.0);
}

#[test]
fn block_visual_info_nonexistent_returns_none() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "Hello")]);
    assert!(ts.block_visual_info(999).is_none());
}

#[test]
fn ensure_caret_visible_when_already_visible() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "Hello")]);
    ts.set_cursor(&text_typeset::CursorDisplay {
        position: 0,
        anchor: 0,
        visible: true,
        selected_cells: vec![],
    });

    // Content fits in viewport — caret is already visible
    let result = ts.ensure_caret_visible();
    assert!(
        result.is_none(),
        "caret should already be visible in a large viewport"
    );
}

#[test]
fn ensure_caret_visible_scrolls_down_when_needed() {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(800.0, 50.0); // very short viewport

    // Create many blocks
    let blocks: Vec<_> = (0..20)
        .map(|i| {
            let mut b = make_block(i, &format!("Paragraph {i}."));
            b.position = i * 20;
            b
        })
        .collect();
    ts.layout_blocks(blocks);

    // Put cursor at a position that's below the viewport
    ts.set_cursor(&text_typeset::CursorDisplay {
        position: 300, // deep in the document
        anchor: 300,
        visible: true,
        selected_cells: vec![],
    });

    let result = ts.ensure_caret_visible();
    assert!(
        result.is_some(),
        "should need to scroll to make caret at position 300 visible"
    );
    assert!(
        result.unwrap() > 0.0,
        "scroll offset should be positive to reveal the caret"
    );
}

#[test]
fn scroll_to_position_changes_offset() {
    let mut ts = make_typesetter();
    let blocks: Vec<_> = (0..10)
        .map(|i| {
            let mut b = make_block(i, &format!("Paragraph {i}."));
            b.position = i * 20;
            b
        })
        .collect();
    ts.layout_blocks(blocks);

    let offset = ts.scroll_to_position(100);
    assert!(
        offset >= 0.0,
        "scroll_to_position should return non-negative offset"
    );
}

// ── Coverage: hit_test edge cases ───────────────────────────────

#[test]
fn hit_test_below_all_content() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "Short")]);
    // Click far below the single-line content
    let result = ts.hit_test(10.0, 500.0);
    assert!(
        result.is_some(),
        "below-content hit test should still return a result"
    );
    let result = result.unwrap();
    assert!(
        matches!(result.region, HitRegion::BelowContent),
        "far below content should be BelowContent, got {:?}",
        result.region
    );
}

#[test]
fn hit_test_above_all_content_returns_first_block() {
    let mut ts = make_typesetter();
    let mut b1 = make_block(1, "First");
    b1.position = 0;
    let mut b2 = make_block(2, "Last");
    b2.position = 10;
    ts.layout_blocks(vec![b1, b2]);

    // Simulate page-up from near the top: screen y is negative enough
    // that doc_y = y + scroll_offset < 0
    let result = ts.hit_test(10.0, -500.0);
    assert!(
        result.is_some(),
        "above-content hit test should return a result"
    );
    let hit = result.unwrap();
    assert_eq!(
        hit.block_id, 1,
        "hit test above all content should return the first block, not the last (got block {})",
        hit.block_id
    );
    assert_eq!(
        hit.position, 0,
        "hit test above all content should return start of first block (pos 0), got {}",
        hit.position
    );
}

#[test]
fn caret_rect_at_end_of_document() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "Hello")]);
    // Position past the last character
    let rect = ts.caret_rect(5);
    assert!(
        rect[2] > 0.0 && rect[3] > 0.0,
        "caret at end should have size"
    );
}

#[test]
fn caret_rect_with_no_layout() {
    let ts = make_typesetter();
    // No layout done — should return fallback
    let rect = ts.caret_rect(0);
    assert!(rect[3] > 0.0, "fallback caret should have positive height");
}

#[test]
fn hit_test_between_blocks_below_content() {
    let mut ts = make_typesetter();
    let mut block = make_block(1, "Short");
    block.top_margin = 0.0;
    block.bottom_margin = 100.0; // large gap after block
    ts.layout_blocks(vec![block]);
    // Click in the gap below the block's content but within its margin
    let result = ts.hit_test(10.0, 50.0);
    assert!(result.is_some());
    let hit = result.unwrap();
    assert_eq!(hit.block_id, 1, "should return the only block");
}

#[test]
fn hit_test_link_region_detected() {
    let mut ts = make_typesetter();
    // Create a block where the text is marked as a link
    let block = BlockLayoutParams {
        block_id: 1,
        position: 0,
        text: "Click here".to_string(),
        fragments: vec![FragmentParams {
            text: "Click here".to_string(),
            offset: 0,
            length: 10,
            font_family: None,
            font_weight: None,
            font_bold: None,
            font_italic: None,
            font_point_size: None,
            underline_style: UnderlineStyle::Single,
            overline: false,
            strikeout: false,
            is_link: true,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            foreground_color: None,
            underline_color: None,
            background_color: None,
            anchor_href: None,
            tooltip: None,
            vertical_alignment: VerticalAlignment::Normal,
            image_name: None,
            image_width: 0.0,
            image_height: 0.0,
        }],
        alignment: Alignment::Left,
        top_margin: 0.0,
        bottom_margin: 0.0,
        left_margin: 0.0,
        right_margin: 0.0,
        text_indent: 0.0,
        list_marker: String::new(),
        list_indent: 0.0,
        tab_positions: vec![],
        line_height_multiplier: None,
        non_breakable_lines: false,
        checkbox: None,
        background_color: None,
    };
    ts.layout_blocks(vec![block]);

    // Hit test on the link text
    let result = ts.hit_test(30.0, 10.0);
    assert!(result.is_some());
    match result.unwrap().region {
        HitRegion::Link { .. } => {} // expected
        other => panic!(
            "expected Link region, got {:?}",
            std::mem::discriminant(&other)
        ),
    }
}

#[test]
fn caret_rect_on_second_line() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(
        1,
        "First line that wraps to a second line at narrow width.",
    )]);
    ts.set_viewport(150.0, 600.0);
    ts.layout_blocks(vec![make_block(
        1,
        "First line that wraps to a second line at narrow width.",
    )]);

    // Position somewhere on the second line
    let rect_line1 = ts.caret_rect(0);
    let rect_line2 = ts.caret_rect(30); // should be on a different line
    // Line 2 caret should be below line 1 caret
    assert!(
        rect_line2[1] > rect_line1[1],
        "caret on second line ({}) should be below first line ({})",
        rect_line2[1],
        rect_line1[1]
    );
}

// ── Selection in tables/frames ─────────────────────────────────

#[test]
fn selection_highlights_text_inside_table_cell() {
    let mut ts = make_typesetter();
    // Block 1: "AB" at position 0 (length 2, +1 separator = position 3 for table)
    ts.layout_blocks(vec![make_block_at(1, 0, "AB")]);
    // Table with cell text "Hello" starting at document position 3
    ts.add_table(&TableLayoutParams {
        table_id: 10,
        rows: 1,
        columns: 1,
        column_widths: vec![],
        border_width: 1.0,
        cell_spacing: 0.0,
        cell_padding: 4.0,
        cells: vec![make_cell_at(0, 0, 100, 3, "Hello")],
    });
    // Select "Hello" (positions 3..8)
    ts.set_cursor(&text_typeset::CursorDisplay {
        position: 8,
        anchor: 3,
        visible: true,
        selected_cells: vec![],
    });
    let block1_height = ts.block_visual_info(1).unwrap().height;
    let frame = ts.render();

    let sel_rects: Vec<_> = frame
        .decorations
        .iter()
        .filter(|d| d.kind == DecorationKind::Selection)
        .collect();
    assert!(
        !sel_rects.is_empty(),
        "selection inside a table cell should produce selection rects"
    );
    // The selection rect y should be below the top-level block
    for r in &sel_rects {
        assert!(
            r.rect[1] >= block1_height - 5.0,
            "table selection rect y ({}) should be below block 1 (height {})",
            r.rect[1],
            block1_height
        );
    }
}

#[test]
fn selection_highlights_text_inside_frame() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block_at(1, 0, "AB")]);
    ts.add_frame(&FrameLayoutParams {
        frame_id: 20,
        position: FramePosition::Inline,
        width: None,
        height: None,
        margin_top: 0.0,
        margin_bottom: 0.0,
        margin_left: 0.0,
        margin_right: 0.0,
        padding: 4.0,
        border_width: 1.0,
        border_style: FrameBorderStyle::Full,
        blocks: vec![make_block_at(200, 3, "World")],
        tables: vec![],
        frames: vec![],
    });
    // Select "World" (positions 3..8)
    ts.set_cursor(&text_typeset::CursorDisplay {
        position: 8,
        anchor: 3,
        visible: true,
        selected_cells: vec![],
    });
    let frame = ts.render();

    let sel_rects: Vec<[f32; 4]> = frame
        .decorations
        .iter()
        .filter(|d| d.kind == DecorationKind::Selection)
        .map(|d| d.rect)
        .collect();
    assert!(
        !sel_rects.is_empty(),
        "selection inside a frame should produce selection rects"
    );
    // The selection rect y should be below the top-level block
    let block1_height = ts.block_visual_info(1).unwrap().height;
    for r in &sel_rects {
        assert!(
            r[1] >= block1_height - 5.0,
            "frame selection rect y ({}) should be below block 1 (height {})",
            r[1],
            block1_height
        );
    }
}

// ── Table hit-testing ──────────────────────────────────────────

#[test]
fn hit_test_inside_table_cell_returns_correct_block() {
    let mut ts = make_typesetter();
    // Block at position 0, text "AB" (2 chars + separator = next pos 3)
    ts.layout_blocks(vec![make_block_at(1, 0, "AB")]);
    ts.add_table(&TableLayoutParams {
        table_id: 10,
        rows: 1,
        columns: 1,
        column_widths: vec![],
        border_width: 1.0,
        cell_spacing: 0.0,
        cell_padding: 4.0,
        cells: vec![make_cell_at(0, 0, 100, 3, "Hello")],
    });
    ts.render();

    // The table starts below block 1. Hit-test in the table area.
    let block1_info = ts.block_visual_info(1).unwrap();
    let table_y = block1_info.y + block1_info.height;
    // Hit at screen coords: x in the middle of the cell, y inside the table
    let result = ts.hit_test(50.0, table_y + 10.0);
    assert!(
        result.is_some(),
        "hit test inside table cell should return a result"
    );
    let result = result.unwrap();
    assert_eq!(
        result.block_id, 100,
        "hit test should return the table cell block id"
    );
    assert!(
        result.position >= 3,
        "position should be >= 3 (start of cell text)"
    );
}

#[test]
fn hit_test_returns_text_region_for_table_cell_content() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block_at(1, 0, "AB")]);
    ts.add_table(&TableLayoutParams {
        table_id: 10,
        rows: 1,
        columns: 1,
        column_widths: vec![],
        border_width: 1.0,
        cell_spacing: 0.0,
        cell_padding: 4.0,
        cells: vec![make_cell_at(0, 0, 100, 3, "Hello")],
    });
    ts.render();

    let block1_info = ts.block_visual_info(1).unwrap();
    let table_y = block1_info.y + block1_info.height;
    let result = ts.hit_test(50.0, table_y + 10.0).unwrap();
    assert!(
        matches!(result.region, HitRegion::Text | HitRegion::PastLineEnd),
        "hit test on table cell text should return Text or PastLineEnd region, got {:?}",
        result.region
    );
}

// ── Caret rect in tables ───────────────────────────────────────

#[test]
fn caret_rect_inside_table_cell_has_valid_position() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block_at(1, 0, "AB")]);
    ts.add_table(&TableLayoutParams {
        table_id: 10,
        rows: 1,
        columns: 1,
        column_widths: vec![],
        border_width: 1.0,
        cell_spacing: 0.0,
        cell_padding: 4.0,
        cells: vec![make_cell_at(0, 0, 100, 3, "Hello")],
    });
    ts.render();

    // Caret at position 3 (start of "Hello" in cell)
    let rect = ts.caret_rect(3);
    assert_caret_is_real(rect, "position 3 inside table cell");
    let block1_info = ts.block_visual_info(1).unwrap();
    let table_top = block1_info.y + block1_info.height;

    // Caret y should be in the table area (below block 1)
    assert!(
        rect[1] >= table_top - 5.0,
        "caret rect y ({}) should be at or below the table top ({})",
        rect[1],
        table_top
    );
    assert!(rect[3] > 0.0, "caret height should be positive");
}

#[test]
fn caret_rect_inside_table_advances_with_position() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block_at(1, 0, "AB")]);
    ts.add_table(&TableLayoutParams {
        table_id: 10,
        rows: 1,
        columns: 1,
        column_widths: vec![],
        border_width: 1.0,
        cell_spacing: 0.0,
        cell_padding: 4.0,
        cells: vec![make_cell_at(0, 0, 100, 3, "Hello")],
    });
    ts.render();

    let rect_start = ts.caret_rect(3); // start of "Hello"
    let rect_mid = ts.caret_rect(5); // middle of "Hello"
    assert!(
        rect_mid[0] > rect_start[0],
        "caret at position 5 ({}) should be right of position 3 ({})",
        rect_mid[0],
        rect_start[0]
    );
}

// ── Cursor movement across frame boundaries ────────────────────

#[test]
fn hit_test_below_frame_content_does_not_stick_to_frame() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block_at(1, 0, "AB")]);
    ts.add_frame(&FrameLayoutParams {
        frame_id: 20,
        position: FramePosition::Inline,
        width: None,
        height: None,
        margin_top: 4.0,
        margin_bottom: 4.0,
        margin_left: 16.0,
        margin_right: 0.0,
        padding: 8.0,
        border_width: 3.0,
        border_style: FrameBorderStyle::LeftOnly,
        blocks: vec![make_block_at(100, 3, "CD")],
        tables: vec![],
        frames: vec![],
    });

    // Caret at end of "CD" inside frame (position 5)
    let caret = ts.caret_rect(5);
    let line_height = caret[3];
    // Target y is one line below the caret (below the frame content)
    let target_y = caret[1] + line_height;

    // hit_test below the frame content should NOT return the frame's block.
    // It should fall through to the block before the frame or return BelowContent.
    let result = ts.hit_test(50.0, target_y);
    if let Some(hit) = &result {
        assert_ne!(
            hit.block_id, 100,
            "hit_test below frame content should not return the frame's block"
        );
    }
}

// ── Hit-test inside frame content ──────────────────────────────

#[test]
fn hit_test_inside_frame_returns_frame_block() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block_at(1, 0, "AB")]);
    ts.add_frame(&FrameLayoutParams {
        frame_id: 20,
        position: FramePosition::Inline,
        width: None,
        height: None,
        margin_top: 4.0,
        margin_bottom: 4.0,
        margin_left: 16.0,
        margin_right: 0.0,
        padding: 8.0,
        border_width: 3.0,
        border_style: FrameBorderStyle::LeftOnly,
        blocks: vec![make_block_at(100, 3, "Hello")],
        tables: vec![],
        frames: vec![],
    });
    ts.render();

    // Get the frame content area: below block 1
    let block1_info = ts.block_visual_info(1).unwrap();
    let frame_content_y = block1_info.y + block1_info.height + 4.0 + 3.0 + 8.0 + 5.0;
    // Hit inside the frame content
    let result = ts.hit_test(50.0, frame_content_y);
    assert!(
        result.is_some(),
        "hit test inside frame should return a result"
    );
    let hit = result.unwrap();
    assert_eq!(
        hit.block_id, 100,
        "hit test inside frame should return the frame's block id"
    );
    assert!(
        hit.position >= 3,
        "position should be >= 3 (start of frame block text)"
    );
}

#[test]
fn hit_test_inside_frame_returns_text_region() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block_at(1, 0, "AB")]);
    ts.add_frame(&FrameLayoutParams {
        frame_id: 20,
        position: FramePosition::Inline,
        width: None,
        height: None,
        margin_top: 0.0,
        margin_bottom: 0.0,
        margin_left: 0.0,
        margin_right: 0.0,
        padding: 4.0,
        border_width: 0.0,
        border_style: FrameBorderStyle::None,
        blocks: vec![make_block_at(100, 3, "Hello world")],
        tables: vec![],
        frames: vec![],
    });
    ts.render();

    let block1_info = ts.block_visual_info(1).unwrap();
    let frame_content_y = block1_info.y + block1_info.height + 4.0 + 5.0;
    let result = ts.hit_test(50.0, frame_content_y).unwrap();
    assert!(
        matches!(result.region, HitRegion::Text | HitRegion::PastLineEnd),
        "hit test on frame text should return Text or PastLineEnd, got {:?}",
        result.region
    );
}

// ── Caret rect inside frames ───────────────────────────────────

#[test]
fn caret_rect_inside_frame_has_valid_position() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block_at(1, 0, "AB")]);
    ts.add_frame(&FrameLayoutParams {
        frame_id: 20,
        position: FramePosition::Inline,
        width: None,
        height: None,
        margin_top: 4.0,
        margin_bottom: 4.0,
        margin_left: 16.0,
        margin_right: 0.0,
        padding: 8.0,
        border_width: 3.0,
        border_style: FrameBorderStyle::LeftOnly,
        blocks: vec![make_block_at(100, 3, "Hello")],
        tables: vec![],
        frames: vec![],
    });
    ts.render();

    // Caret at position 3 (start of "Hello" in frame)
    let rect = ts.caret_rect(3);
    assert_caret_is_real(rect, "position 3 inside frame");
    let block1_info = ts.block_visual_info(1).unwrap();
    let frame_top = block1_info.y + block1_info.height;

    // Caret y should be in the frame area (below block 1)
    assert!(
        rect[1] >= frame_top - 5.0,
        "caret rect y ({}) should be at or below the frame top ({})",
        rect[1],
        frame_top
    );
    assert!(rect[3] > 0.0, "caret height should be positive");
}

#[test]
fn caret_rect_inside_frame_advances_with_position() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block_at(1, 0, "AB")]);
    ts.add_frame(&FrameLayoutParams {
        frame_id: 20,
        position: FramePosition::Inline,
        width: None,
        height: None,
        margin_top: 0.0,
        margin_bottom: 0.0,
        margin_left: 0.0,
        margin_right: 0.0,
        padding: 4.0,
        border_width: 0.0,
        border_style: FrameBorderStyle::None,
        blocks: vec![make_block_at(100, 3, "Hello")],
        tables: vec![],
        frames: vec![],
    });
    ts.render();

    let rect_start = ts.caret_rect(3); // start of "Hello"
    let rect_mid = ts.caret_rect(5); // middle of "Hello"
    assert!(
        rect_mid[0] > rect_start[0],
        "caret at position 5 ({}) should be right of position 3 ({})",
        rect_mid[0],
        rect_start[0]
    );
}

// ── Relayout after cut from a non-last paragraph ───────────────

#[test]
fn hit_test_on_last_paragraph_after_cut_from_middle() {
    // Mirrors the user-reported UI bug: after cutting chars from a non-last
    // paragraph, clicking at the very end of the last paragraph used to land
    // N chars short of the actual last character, because relayout_block
    // shifted Y positions but left BlockLayout.position stale.
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![
        make_block_at(1, 0, "First"),
        make_block_at(2, 6, "Second"),
        make_block_at(3, 13, "Third"),
    ]);
    ts.render();

    // Capture where the very last caret position renders BEFORE the cut
    // (position 18 = end of "Third" in the initial 16-char, 3-block doc).
    let last_caret_before = ts.caret_rect(18);

    // Cut "Sec" from block 2: text goes from "Second" (6 chars) to "ond" (3).
    // New document layout: blocks at 0, 6, 10; max cursor position = 15.
    ts.relayout_block(&make_block_at(2, 6, "ond"));
    ts.render();

    // Click just inside the right edge of the last paragraph. In the pre-fix
    // world, the layout thought block 3 was at position 13, so this click
    // returned position 18 — a position that no longer exists in the doc.
    // With the fix, block 3's position is 10, so this click returns 15 (the
    // real end of the shortened doc).
    let hit = ts
        .hit_test(200.0, last_caret_before[1] + last_caret_before[3] / 2.0)
        .expect("click inside last paragraph should hit");
    assert_eq!(
        hit.position, 15,
        "clicking at the visual end of the last paragraph after cut must map to \
         the new max cursor position (15), not the stale pre-cut value"
    );
}

// ── Relayout frame block (simulating typing) ───────────────────

#[test]
fn relayout_frame_block_renders_new_glyphs() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block_at(1, 0, "AB")]);
    ts.add_frame(&FrameLayoutParams {
        frame_id: 20,
        position: FramePosition::Inline,
        width: None,
        height: None,
        margin_top: 0.0,
        margin_bottom: 0.0,
        margin_left: 0.0,
        margin_right: 0.0,
        padding: 4.0,
        border_width: 0.0,
        border_style: FrameBorderStyle::None,
        blocks: vec![make_block_at(100, 3, "Hi")],
        tables: vec![],
        frames: vec![],
    });

    let frame1 = ts.render();
    let glyph_count_before = frame1.glyphs.len();
    let caret_before = ts.caret_rect(4); // after 'H' in "Hi"

    // Simulate typing: "Hi" -> "Hxi" (insert 'x' at position 4)
    ts.relayout_block(&make_block_at(100, 3, "Hxi"));
    let frame2 = ts.render();
    let glyph_count_after = frame2.glyphs.len();

    assert!(
        glyph_count_after > glyph_count_before,
        "after relayout with longer text, glyph count should increase: {} -> {}",
        glyph_count_before,
        glyph_count_after
    );

    // Caret at position 5 (after the inserted 'x') should be right of where 4 was
    let caret_after = ts.caret_rect(5);
    assert!(
        caret_after[0] > caret_before[0],
        "caret after typing should be right of previous caret: {} -> {}",
        caret_before[0],
        caret_after[0]
    );
}

#[test]
fn relayout_frame_block_caret_advances_correctly() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block_at(1, 0, "AB")]);
    ts.add_frame(&FrameLayoutParams {
        frame_id: 20,
        position: FramePosition::Inline,
        width: None,
        height: None,
        margin_top: 0.0,
        margin_bottom: 0.0,
        margin_left: 0.0,
        margin_right: 0.0,
        padding: 4.0,
        border_width: 0.0,
        border_style: FrameBorderStyle::None,
        blocks: vec![make_block_at(100, 3, "Hello")],
        tables: vec![],
        frames: vec![],
    });
    ts.render();

    // Record caret positions before relayout
    let caret_h = ts.caret_rect(3); // start of "Hello"
    let caret_e = ts.caret_rect(4); // after 'H'

    // Simulate typing 'X' at position 4: "Hello" -> "HXello"
    ts.relayout_block(&make_block_at(100, 3, "HXello"));
    ts.render();

    // After relayout:
    // pos 3 = start of "HXello" (same as before)
    // pos 4 = after 'H' (should be same x as before since 'H' didn't change)
    // pos 5 = after 'X' (the new char - cursor should be here)
    let caret_h_after = ts.caret_rect(3);
    let caret_x_after = ts.caret_rect(5); // after the inserted 'X'

    assert!(
        (caret_h_after[0] - caret_h[0]).abs() < 1.0,
        "start of block caret should stay at same x: {} vs {}",
        caret_h[0],
        caret_h_after[0]
    );
    assert!(
        caret_x_after[0] > caret_e[0],
        "caret after inserted 'X' at pos 5 ({}) should be right of old pos 4 ({})",
        caret_x_after[0],
        caret_e[0]
    );
}

// ── Nested frame (frame inside frame) tests ────────────────────

#[test]
fn caret_rect_inside_nested_frame() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block_at(1, 0, "AB")]);
    ts.add_frame(&FrameLayoutParams {
        frame_id: 20,
        position: FramePosition::Inline,
        width: None,
        height: None,
        margin_top: 0.0,
        margin_bottom: 0.0,
        margin_left: 0.0,
        margin_right: 0.0,
        padding: 4.0,
        border_width: 0.0,
        border_style: FrameBorderStyle::None,
        blocks: vec![make_block_at(100, 3, "Outer")],
        tables: vec![],
        frames: vec![(
            1,
            FrameLayoutParams {
                frame_id: 30,
                position: FramePosition::Inline,
                width: None,
                height: None,
                margin_top: 0.0,
                margin_bottom: 0.0,
                margin_left: 0.0,
                margin_right: 0.0,
                padding: 4.0,
                border_width: 0.0,
                border_style: FrameBorderStyle::None,
                blocks: vec![make_block_at(200, 9, "Inner")],
                tables: vec![],
                frames: vec![],
            },
        )],
    });
    ts.render();

    // Caret at position 9 (start of "Inner" in nested frame)
    let rect = ts.caret_rect(9);
    assert_caret_is_real(rect, "position 9 inside nested frame");
    assert!(
        rect[3] > 0.0,
        "caret inside nested frame should have valid height (not fallback)"
    );
    // Should NOT be the fallback [0, -scroll, 2, 16]
    assert!(
        rect[1] > 0.0,
        "caret y inside nested frame ({}) should be positive (below top-level content)",
        rect[1]
    );
}

#[test]
fn hit_test_inside_nested_frame_returns_inner_block() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block_at(1, 0, "AB")]);
    ts.add_frame(&FrameLayoutParams {
        frame_id: 20,
        position: FramePosition::Inline,
        width: None,
        height: None,
        margin_top: 0.0,
        margin_bottom: 0.0,
        margin_left: 0.0,
        margin_right: 0.0,
        padding: 4.0,
        border_width: 0.0,
        border_style: FrameBorderStyle::None,
        blocks: vec![make_block_at(100, 3, "Outer")],
        tables: vec![],
        frames: vec![(
            1,
            FrameLayoutParams {
                frame_id: 30,
                position: FramePosition::Inline,
                width: None,
                height: None,
                margin_top: 0.0,
                margin_bottom: 0.0,
                margin_left: 0.0,
                margin_right: 0.0,
                padding: 4.0,
                border_width: 0.0,
                border_style: FrameBorderStyle::None,
                blocks: vec![make_block_at(200, 9, "Inner")],
                tables: vec![],
                frames: vec![],
            },
        )],
    });
    ts.render();

    // Use caret_rect to find where the nested frame content is
    let caret = ts.caret_rect(9);
    assert!(
        caret[1] > 0.0,
        "caret_rect should return valid position for nested frame content (got y={})",
        caret[1]
    );
    let result = ts.hit_test(caret[0] + 10.0, caret[1] + caret[3] / 2.0);
    assert!(
        result.is_some(),
        "hit test inside nested frame should return a result"
    );
    let hit = result.unwrap();
    assert_eq!(
        hit.block_id, 200,
        "hit test inside nested frame should return the inner block id"
    );
}

#[test]
fn selection_inside_nested_frame() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block_at(1, 0, "AB")]);
    ts.add_frame(&FrameLayoutParams {
        frame_id: 20,
        position: FramePosition::Inline,
        width: None,
        height: None,
        margin_top: 0.0,
        margin_bottom: 0.0,
        margin_left: 0.0,
        margin_right: 0.0,
        padding: 4.0,
        border_width: 0.0,
        border_style: FrameBorderStyle::None,
        blocks: vec![make_block_at(100, 3, "Outer")],
        tables: vec![],
        frames: vec![(
            1,
            FrameLayoutParams {
                frame_id: 30,
                position: FramePosition::Inline,
                width: None,
                height: None,
                margin_top: 0.0,
                margin_bottom: 0.0,
                margin_left: 0.0,
                margin_right: 0.0,
                padding: 4.0,
                border_width: 0.0,
                border_style: FrameBorderStyle::None,
                blocks: vec![make_block_at(200, 9, "Inner")],
                tables: vec![],
                frames: vec![],
            },
        )],
    });
    // Select "Inner" (positions 9..14)
    ts.set_cursor(&text_typeset::CursorDisplay {
        position: 9,
        anchor: 14,
        visible: true,
        selected_cells: vec![],
    });
    let frame = ts.render();

    let sel_rects: Vec<_> = frame
        .decorations
        .iter()
        .filter(|d| d.kind == DecorationKind::Selection)
        .collect();
    assert!(
        !sel_rects.is_empty(),
        "selection inside nested frame should produce selection rects"
    );
}

// ── Table-ID in HitTestResult ────────────────────────────────────

#[test]
fn hit_test_plain_block_has_no_table_id() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "Hello world")]);
    ts.render();

    let result = ts.hit_test(40.0, 10.0).unwrap();
    assert_eq!(
        result.table_id, None,
        "block-only hit should have table_id=None, got {:?}",
        result.table_id
    );
}

// ── Inter-block margin gap ───────────────────────────────────────

#[test]
fn hit_test_in_inter_block_gap_snaps_to_block_above() {
    // Two blocks at different document_positions with a generous
    // gap between them (block 2 has top_margin=40). A click in the
    // gap used to fall through `find_block_at_y`'s "no containing
    // block" branch and return the LAST block in the document,
    // which made mouse-drag selections jump from the anchor to the
    // document end every time the pointer crossed the gap.
    //
    // The fix: snap to the block immediately above the gap so the
    // selection extends naturally to end-of-block-N until the
    // pointer crosses into block N+1's painted area.
    let mut ts = make_typesetter();
    let block_a = make_block_at(1, 0, "first block");
    let mut block_b = make_block_at(2, 12, "second block");
    block_b.top_margin = 40.0;
    let mut block_c = make_block_at(3, 25, "third block");
    block_c.top_margin = 40.0;
    ts.layout_blocks(vec![block_a, block_b, block_c]);
    ts.render();

    // Find a y that falls inside the gap between block 1 and block 2:
    // block 1's bottom edge + half the 40-px gap.
    let line_h = 16.0_f32; // default font line height in the test rig
    let gap_y = line_h + 20.0; // ~5 px into the gap above block 2

    let hit = ts
        .hit_test(10.0, gap_y)
        .expect("hit_test returns Some in inter-block gap");
    assert_eq!(
        hit.block_id, 1,
        "click in gap between blocks 1 and 2 should snap to block 1 \
         (got block {}); pre-fix this returned the LAST block in the \
         document and selections jumped to document end on every drag",
        hit.block_id
    );

    // Same check for the gap between block 2 and block 3 — confirms
    // the fix walks BACK from idx, not just to the document's first
    // block.
    let gap2_y = ts.content_height() - line_h - 20.0;
    let hit2 = ts
        .hit_test(10.0, gap2_y)
        .expect("hit_test returns Some in second gap");
    assert_eq!(
        hit2.block_id, 2,
        "click in gap between blocks 2 and 3 should snap to block 2 \
         (got block {})",
        hit2.block_id
    );
}
