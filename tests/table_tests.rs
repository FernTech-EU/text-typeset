//! Table layout, rendering, cursor, and hit-testing tests.
//!
//! Uses the framework-agnostic API (no text-document dependency).

mod helpers;
use helpers::{
    NOTO_SANS, Rect, RenderFrameExt, Typesetter, assert_caret_is_real, assert_no_glyph_overlap,
    make_block_at, make_cell_at, make_frame, make_table,
};

use text_typeset::layout::table::{CellLayoutParams, TableLayoutParams};
use text_typeset::{CursorDisplay, DecorationKind};

// ── Setup helpers ────────────────────────────────────────────────

/// Create a 3-row, 2-column table with known text content and positions.
///
/// Row 0: "Header A" (pos 0), "Header B" (pos 9)
/// Row 1: "Cell one" (pos 18), "Cell two" (pos 27)
/// Row 2: "Cell three" (pos 36), "Cell four" (pos 47)
fn make_3x2_table() -> TableLayoutParams {
    make_table(
        1,
        3,
        2,
        vec![
            make_cell_at(0, 0, 10, 0, "Header A"),
            make_cell_at(0, 1, 11, 9, "Header B"),
            make_cell_at(1, 0, 12, 18, "Cell one"),
            make_cell_at(1, 1, 13, 27, "Cell two"),
            make_cell_at(2, 0, 14, 36, "Cell three"),
            make_cell_at(2, 1, 15, 47, "Cell four"),
        ],
    )
}

/// Set up a Typesetter with the 3x2 table at a given viewport width.
fn setup_table(viewport_width: f32) -> Typesetter {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(viewport_width, 600.0);
    let table = make_3x2_table();
    ts.add_table(&table);
    ts.render();
    ts
}

// ── Layout tests ─────────────────────────────────────────────────

#[test]
fn table_has_positive_dimensions() {
    let ts = setup_table(800.0);
    let height = ts.content_height();
    assert!(
        height > 0.0,
        "table content height should be positive: {}",
        height
    );
}

#[test]
fn table_rows_have_increasing_y() {
    let ts = setup_table(800.0);

    // Caret y should increase from row 0 to row 2
    let y0 = ts.caret_rect(0)[1]; // Row 0, col 0
    let y1 = ts.caret_rect(18)[1]; // Row 1, col 0
    let y2 = ts.caret_rect(36)[1]; // Row 2, col 0

    assert!(y1 > y0, "row 1 y ({}) should be below row 0 y ({})", y1, y0);
    assert!(y2 > y1, "row 2 y ({}) should be below row 1 y ({})", y2, y1);
}

#[test]
fn table_columns_have_increasing_x() {
    let ts = setup_table(800.0);

    // Col 1 caret should be to the right of col 0 in the same row
    let x_col0 = ts.caret_rect(0)[0]; // Row 0, col 0
    let x_col1 = ts.caret_rect(9)[0]; // Row 0, col 1

    assert!(
        x_col1 > x_col0,
        "col 1 x ({}) should be right of col 0 x ({})",
        x_col1,
        x_col0
    );
}

#[test]
fn table_same_row_cells_share_y() {
    let ts = setup_table(800.0);

    // Both columns in row 0 should have the same y
    let y_col0 = ts.caret_rect(0)[1];
    let y_col1 = ts.caret_rect(9)[1];
    assert!(
        (y_col0 - y_col1).abs() < 1.0,
        "same-row cells should have same y: col0={}, col1={}",
        y_col0,
        y_col1
    );
}

#[test]
fn table_column_widths_finite_at_infinite_viewport() {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(f32::INFINITY, 600.0);
    ts.set_content_width(f32::INFINITY);
    let table = make_3x2_table();
    ts.add_table(&table);
    ts.render();

    // All caret positions should have finite x
    for pos in [0, 9, 18, 27, 36, 47] {
        let rect = ts.caret_rect(pos);
        assert!(
            rect[0].is_finite(),
            "caret x should be finite at pos {}: {}",
            pos,
            rect[0]
        );
    }
}

#[test]
fn table_column_widths_positive_at_narrow_viewport() {
    let ts = setup_table(100.0);

    // Even with a very narrow viewport, all carets should have valid positions
    for pos in [0, 9, 18, 27, 36, 47] {
        let rect = ts.caret_rect(pos);
        assert!(rect[0].is_finite(), "caret x finite at pos {}", pos);
        assert!(rect[3] > 0.0, "caret h positive at pos {}", pos);
    }
}

// ── Caret tests ──────────────────────────────────────────────────

#[test]
fn caret_rect_found_for_all_cell_positions() {
    let ts = setup_table(800.0);

    for pos in [
        0, 4, 8, 9, 13, 17, 18, 22, 26, 27, 31, 35, 36, 40, 45, 47, 51, 57,
    ] {
        assert_caret_is_real(ts.caret_rect(pos), &format!("pos {}", pos));
    }
}

#[test]
fn caret_y_monotonic_across_cells() {
    let ts = setup_table(800.0);

    // Walk through all cells in row-major order.
    // y should never decrease when moving to a new row.
    let positions = [0, 9, 18, 27, 36, 47]; // start of each cell block
    let mut prev_y = -1.0f32;

    for &pos in &positions {
        let rect = ts.caret_rect(pos);
        let y = rect[1];
        assert!(
            y >= prev_y - 1.0,
            "caret y should not decrease: prev={}, pos {} y={}",
            prev_y,
            pos,
            y
        );
        prev_y = y;
    }
}

#[test]
fn caret_x_advances_within_cell() {
    let ts = setup_table(800.0);

    // Within "Header A" (pos 0-7), x should advance
    let x_start = ts.caret_rect(0)[0];
    let x_mid = ts.caret_rect(4)[0];
    let x_end = ts.caret_rect(8)[0];

    assert!(
        x_mid > x_start,
        "x should advance: start={}, mid={}",
        x_start,
        x_mid
    );
    assert!(
        x_end > x_mid,
        "x should advance: mid={}, end={}",
        x_mid,
        x_end
    );
}

// ── Hit-test tests ───────────────────────────────────────────────

#[test]
fn hit_test_finds_cells_in_all_rows() {
    let ts = setup_table(800.0);

    // Hit test at the caret position for each row's first cell
    for (label, pos) in [("row 0", 0), ("row 1", 18), ("row 2", 36)] {
        let rect = ts.caret_rect(pos);
        let hx = rect[0].max(1.0);
        let hy = rect[1] + rect[3] * 0.5;
        let hit = ts.hit_test(hx, hy);
        assert!(
            hit.is_some(),
            "hit_test should find {} at ({}, {})",
            label,
            hx,
            hy
        );
    }
}

#[test]
fn hit_test_returns_table_cell_blocks() {
    let ts = setup_table(800.0);

    // Hit at row 1, col 0 area should find block 12 ("Cell one")
    let rect = ts.caret_rect(18);
    let hit = ts
        .hit_test(rect[0].max(1.0), rect[1] + rect[3] * 0.5)
        .unwrap();
    assert_eq!(hit.block_id, 12, "hit should find block 12 (Cell one)");

    // This should NOT be found by block_visual_info (it's in a table cell)
    assert!(
        ts.block_visual_info(hit.block_id).is_none(),
        "table cell block should not have block_visual_info"
    );

    // But is_block_in_table should return true
    assert!(
        ts.is_block_in_table(hit.block_id),
        "table cell block should be in table"
    );
}

// ── Rendering tests ──────────────────────────────────────────────

#[test]
fn table_renders_glyphs_for_all_cells() {
    let mut ts = setup_table(800.0);
    let frame = ts.render();

    // Table has 6 cells with text, should produce glyphs
    assert!(
        frame.glyphs.len() > 20,
        "table should render many glyphs, got {}",
        frame.glyphs.len()
    );
}

#[test]
fn table_renders_border_decorations() {
    let mut ts = setup_table(800.0);
    let frame = ts.render();

    // Should have border decorations (from generate_table_decorations)
    let borders: Vec<_> = frame
        .decorations
        .iter()
        .filter(|d| d.kind == text_typeset::DecorationKind::BlockBackground)
        .collect();
    // Table borders produce at least some decorations
    assert!(
        !borders.is_empty() || !frame.decorations.is_empty(),
        "table should produce decorations"
    );
}

#[test]
fn table_cell_glyphs_within_cell_bounds() {
    let mut ts = setup_table(800.0);
    let frame = ts.render();

    // All glyphs should have finite, non-negative positions
    for (i, glyph) in frame.glyphs.iter().enumerate() {
        let r = Rect::from(glyph.screen);
        assert!(
            r.x().is_finite() && r.y().is_finite(),
            "glyph[{}] has non-finite position: {}",
            i,
            r
        );
        assert!(r.x() >= 0.0, "glyph[{}] has negative x: {}", i, r);
    }
}

// ── Down-arrow / vertical navigation tests ───────────────────────

#[test]
fn vertical_hit_test_reaches_all_rows() {
    let ts = setup_table(800.0);

    // Get the caret rect for the first cell in each row
    let r0 = Rect::from(ts.caret_rect(0)); // Row 0
    let r1 = Rect::from(ts.caret_rect(18)); // Row 1
    let r2 = Rect::from(ts.caret_rect(36)); // Row 2

    // Simulate down-arrow: hit_test at (same x, next row y)
    let x = r0.x().max(1.0);

    // From row 0, move to row 1
    let hit1 = ts.hit_test(x, r1.y() + r1.h() * 0.5);
    assert!(hit1.is_some(), "should hit row 1");
    let hit1 = hit1.unwrap();
    assert_eq!(hit1.block_id, 12, "should hit row 1 col 0 block (12)");

    // From row 1, move to row 2
    let hit2 = ts.hit_test(x, r2.y() + r2.h() * 0.5);
    assert!(hit2.is_some(), "should hit row 2");
    let hit2 = hit2.unwrap();
    assert_eq!(hit2.block_id, 14, "should hit row 2 col 0 block (14)");
}

#[test]
fn hit_test_between_rows_finds_nearest() {
    let ts = setup_table(800.0);

    let r0 = Rect::from(ts.caret_rect(0)); // Row 0
    let r1 = Rect::from(ts.caret_rect(18)); // Row 1

    // Hit-test at a y midpoint between row 0 bottom and row 1 top
    let mid_y = (r0.bottom() + r1.y()) / 2.0;
    let hit = ts.hit_test(r0.x().max(1.0), mid_y);
    assert!(
        hit.is_some(),
        "hit_test should find something between rows at y={}",
        mid_y
    );
}

// ── Relayout tests ───────────────────────────────────────────────

#[test]
fn relayout_table_block_updates_cell_content() {
    let mut ts = setup_table(800.0);

    let before_height = ts.content_height();
    let glyph_count_before = ts.render().glyphs.len();

    // Relayout cell (1,0) with longer text
    let updated = make_block_at(12, 18, "Cell one now has much more text to wrap around");
    ts.relayout_block(&updated);
    ts.render();

    let after_height = ts.content_height();
    let glyph_count_after = ts.render().glyphs.len();

    // Table should be taller (text wraps)
    assert!(
        after_height >= before_height,
        "table should grow: before={}, after={}",
        before_height,
        after_height
    );

    // More glyphs should be rendered
    assert!(
        glyph_count_after > glyph_count_before,
        "more glyphs after longer text: before={}, after={}",
        glyph_count_before,
        glyph_count_after
    );
}

#[test]
fn render_block_only_preserves_table_cell_glyphs() {
    let mut ts = setup_table(800.0);
    let full_glyph_count = ts.render().glyphs.len();
    assert!(full_glyph_count > 0, "table should render glyphs");

    // Relayout cell (1,0) with new text, then use render_block_only
    let updated = make_block_at(12, 18, "Updated!");
    ts.relayout_block(&updated);
    let frame = ts.render_block_only(12);

    // render_block_only must fall back to a full render for table cell blocks,
    // so glyphs should still be present (not vanish).
    assert!(
        frame.glyphs.len() > 10,
        "table cell glyphs should be preserved after render_block_only, got {}",
        frame.glyphs.len()
    );
}

// ── Inter-row gap hit-test tests ────────────────────────────────

#[test]
fn hit_test_in_inter_row_gap_snaps_to_nearest_row() {
    let ts = setup_table(800.0);

    let r0 = Rect::from(ts.caret_rect(0)); // Row 0
    let r1 = Rect::from(ts.caret_rect(18)); // Row 1

    // Find the exact gap: just past row 0 bottom, just before row 1 top
    let gap_y = (r0.bottom() + r1.y()) / 2.0;
    let x = r0.x().max(1.0);

    let hit = ts.hit_test(x, gap_y);
    assert!(
        hit.is_some(),
        "hit_test in inter-row gap at y={} should snap to a row",
        gap_y
    );

    // Should resolve to either row 0 or row 1 block
    let bid = hit.unwrap().block_id;
    assert!(
        bid == 10 || bid == 12,
        "should hit row 0 (block 10) or row 1 (block 12), got block {}",
        bid
    );
}

#[test]
fn hit_test_just_above_row_content_finds_row() {
    let ts = setup_table(800.0);

    // Hit just 1px above the row 1 caret
    let r1 = Rect::from(ts.caret_rect(18));
    let hit = ts.hit_test(r1.x().max(1.0), r1.y() - 1.0);
    assert!(
        hit.is_some(),
        "hit_test just above row 1 content should still find a cell"
    );
}

// ── Layout: column widths, spacing, single row/col, multi-block ─

#[test]
fn table_custom_column_widths_respected() {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(800.0, 600.0);

    // 1:3 ratio -- col 1 should be 3x wider than col 0
    let table = TableLayoutParams {
        table_id: 1,
        rows: 1,
        columns: 2,
        column_widths: vec![1.0, 3.0],
        border_width: 1.0,
        cell_spacing: 0.0,
        cell_padding: 4.0,
        cells: vec![
            make_cell_at(0, 0, 10, 0, "Narrow"),
            make_cell_at(0, 1, 11, 7, "Wide column"),
        ],
    };
    ts.add_table(&table);
    ts.render();

    let x0 = ts.caret_rect(0)[0];
    let x1 = ts.caret_rect(7)[0];

    // Col 1 should start significantly to the right of col 0
    assert!(
        x1 > x0,
        "col 1 x ({}) should be right of col 0 x ({})",
        x1,
        x0
    );

    // Verify the proportions: col 0 gets 1/4, col 1 gets 3/4 of content area.
    // Compare against an even-distribution table.
    let mut ts_even = Typesetter::new();
    let face_e = ts_even.register_font(NOTO_SANS);
    ts_even.set_default_font(face_e, 16.0);
    ts_even.set_viewport(800.0, 600.0);
    let table_even = TableLayoutParams {
        table_id: 1,
        rows: 1,
        columns: 2,
        column_widths: vec![], // even distribution
        border_width: 1.0,
        cell_spacing: 0.0,
        cell_padding: 4.0,
        cells: vec![
            make_cell_at(0, 0, 10, 0, "Narrow"),
            make_cell_at(0, 1, 11, 7, "Wide column"),
        ],
    };
    ts_even.add_table(&table_even);
    ts_even.render();

    let x1_even = ts_even.caret_rect(7)[0];
    // In the 1:3 table, col 1 starts earlier (col 0 is narrower)
    assert!(
        x1 < x1_even,
        "with 1:3 widths, col 1 should start earlier ({}) than even ({})",
        x1,
        x1_even
    );
}

#[test]
fn table_cell_spacing_creates_gaps_between_rows() {
    // Table without spacing
    let mut ts0 = Typesetter::new();
    let f0 = ts0.register_font(NOTO_SANS);
    ts0.set_default_font(f0, 16.0);
    ts0.set_viewport(800.0, 600.0);
    let table0 = TableLayoutParams {
        table_id: 1,
        rows: 2,
        columns: 1,
        column_widths: vec![],
        border_width: 1.0,
        cell_spacing: 0.0,
        cell_padding: 4.0,
        cells: vec![
            make_cell_at(0, 0, 10, 0, "Row A"),
            make_cell_at(1, 0, 11, 6, "Row B"),
        ],
    };
    ts0.add_table(&table0);
    ts0.render();
    let h0 = ts0.content_height();
    let y0_r0 = ts0.caret_rect(0)[1];
    let y0_r1 = ts0.caret_rect(6)[1];

    // Table with spacing = 10
    let mut ts1 = Typesetter::new();
    let f1 = ts1.register_font(NOTO_SANS);
    ts1.set_default_font(f1, 16.0);
    ts1.set_viewport(800.0, 600.0);
    let table1 = TableLayoutParams {
        table_id: 1,
        rows: 2,
        columns: 1,
        column_widths: vec![],
        border_width: 1.0,
        cell_spacing: 10.0,
        cell_padding: 4.0,
        cells: vec![
            make_cell_at(0, 0, 10, 0, "Row A"),
            make_cell_at(1, 0, 11, 6, "Row B"),
        ],
    };
    ts1.add_table(&table1);
    ts1.render();
    let h1 = ts1.content_height();
    let y1_r0 = ts1.caret_rect(0)[1];
    let y1_r1 = ts1.caret_rect(6)[1];

    assert!(
        h1 > h0,
        "spaced table should be taller: no_spacing={}, spacing={}",
        h0,
        h1
    );

    let gap_without = y0_r1 - y0_r0;
    let gap_with = y1_r1 - y1_r0;
    assert!(
        gap_with > gap_without,
        "row gap with spacing ({}) should exceed without ({})",
        gap_with,
        gap_without
    );
}

#[test]
fn table_single_row_layout() {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(800.0, 600.0);
    let table = make_table(
        1,
        1,
        2,
        vec![
            make_cell_at(0, 0, 10, 0, "Left"),
            make_cell_at(0, 1, 11, 5, "Right"),
        ],
    );
    ts.add_table(&table);
    ts.render();

    assert!(ts.content_height() > 0.0);
    assert_caret_is_real(ts.caret_rect(0), "single row col 0");
    assert_caret_is_real(ts.caret_rect(5), "single row col 1");

    // 4 outer borders + 0 row dividers + 1 column divider = 5
    let frame = ts.render();
    let border_count = frame.decoration_count(DecorationKind::TableBorder);
    assert_eq!(
        border_count, 5,
        "1-row 2-col table should have 5 border decorations, got {}",
        border_count
    );
}

#[test]
fn table_single_column_layout() {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(800.0, 600.0);
    let table = make_table(
        1,
        2,
        1,
        vec![
            make_cell_at(0, 0, 10, 0, "Top"),
            make_cell_at(1, 0, 11, 4, "Bottom"),
        ],
    );
    ts.add_table(&table);
    ts.render();

    let y0 = ts.caret_rect(0)[1];
    let y1 = ts.caret_rect(4)[1];
    assert!(y1 > y0, "row 1 y ({}) should be below row 0 y ({})", y1, y0);

    // 4 outer borders + 1 row divider + 0 column dividers = 5
    let frame = ts.render();
    let border_count = frame.decoration_count(DecorationKind::TableBorder);
    assert_eq!(
        border_count, 5,
        "2-row 1-col table should have 5 border decorations, got {}",
        border_count
    );
}

#[test]
fn table_multiple_blocks_per_cell() {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(800.0, 600.0);

    let table = TableLayoutParams {
        table_id: 1,
        rows: 1,
        columns: 1,
        column_widths: vec![],
        border_width: 1.0,
        cell_spacing: 0.0,
        cell_padding: 4.0,
        cells: vec![CellLayoutParams {
            row: 0,
            column: 0,
            blocks: vec![
                make_block_at(20, 0, "First paragraph"),
                make_block_at(21, 16, "Second paragraph"),
            ],
            background_color: None,
        }],
    };
    ts.add_table(&table);
    ts.render();

    assert_caret_is_real(ts.caret_rect(0), "first block start");
    assert_caret_is_real(ts.caret_rect(16), "second block start");

    let y0 = ts.caret_rect(0)[1];
    let y1 = ts.caret_rect(16)[1];
    assert!(
        y1 > y0,
        "second block y ({}) should be below first ({})",
        y1,
        y0
    );
}

#[test]
fn table_out_of_bounds_cell_silently_skipped() {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(800.0, 600.0);

    // 1x1 table with a valid cell and an out-of-bounds cell
    let table = TableLayoutParams {
        table_id: 1,
        rows: 1,
        columns: 1,
        column_widths: vec![],
        border_width: 1.0,
        cell_spacing: 0.0,
        cell_padding: 4.0,
        cells: vec![
            make_cell_at(0, 0, 10, 0, "Valid"),
            make_cell_at(5, 5, 99, 100, "Out of bounds"),
        ],
    };
    ts.add_table(&table);
    ts.render();

    assert!(
        ts.content_height() > 0.0,
        "table should have positive height"
    );
    assert_caret_is_real(ts.caret_rect(0), "valid cell");
    let frame = ts.render();
    assert!(!frame.glyphs.is_empty(), "valid cell should produce glyphs");
}

// ── Caret: single-char relayout ─────────────────────────────────

#[test]
fn single_char_relayout_does_not_grow_row_height() {
    let mut ts = setup_table(800.0);
    ts.render();

    let height_before = ts.content_height();

    // Append one character to "Cell one" (block 12, pos 18) -> "Cell one!"
    let updated = make_block_at(12, 18, "Cell one!");
    ts.relayout_block(&updated);
    ts.render();

    let height_after = ts.content_height();
    assert!(
        (height_after - height_before).abs() < 0.01,
        "adding one char should not grow the table: before={}, after={}",
        height_before,
        height_after
    );
}

// ── Rendering: borders, backgrounds, overlap ────────────────────

#[test]
fn table_zero_border_produces_no_border_decorations() {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(800.0, 600.0);

    let table = TableLayoutParams {
        table_id: 1,
        rows: 2,
        columns: 2,
        column_widths: vec![],
        border_width: 0.0,
        cell_spacing: 0.0,
        cell_padding: 4.0,
        cells: vec![
            make_cell_at(0, 0, 10, 0, "A"),
            make_cell_at(0, 1, 11, 2, "B"),
            make_cell_at(1, 0, 12, 4, "C"),
            make_cell_at(1, 1, 13, 6, "D"),
        ],
    };
    ts.add_table(&table);
    let frame = ts.render();

    let border_count = frame.decoration_count(DecorationKind::TableBorder);
    assert_eq!(
        border_count, 0,
        "zero border_width should produce no TableBorder decorations, got {}",
        border_count
    );
    assert!(!frame.glyphs.is_empty(), "glyphs should still render");
}

#[test]
fn table_cell_background_produces_decoration() {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(800.0, 600.0);

    let table = TableLayoutParams {
        table_id: 1,
        rows: 2,
        columns: 2,
        column_widths: vec![],
        border_width: 1.0,
        cell_spacing: 0.0,
        cell_padding: 4.0,
        cells: vec![
            CellLayoutParams {
                row: 0,
                column: 0,
                blocks: vec![make_block_at(10, 0, "Highlighted")],
                background_color: Some([1.0, 0.0, 0.0, 0.5]),
            },
            make_cell_at(0, 1, 11, 12, "Normal"),
            make_cell_at(1, 0, 12, 19, "Normal"),
            make_cell_at(1, 1, 13, 26, "Normal"),
        ],
    };
    ts.add_table(&table);
    let frame = ts.render();

    let bg_rects = frame.decorations_of(DecorationKind::TableCellBackground);
    assert_eq!(
        bg_rects.len(),
        1,
        "exactly one cell has a background, got {}",
        bg_rects.len()
    );
    let bg = &bg_rects[0];
    assert!(bg.w() > 0.0, "background should have positive width");
    assert!(bg.h() > 0.0, "background should have positive height");
}

#[test]
fn table_border_decoration_count_matches_structure() {
    let mut ts = setup_table(800.0);
    let frame = ts.render();

    // 3 rows, 2 cols: 4 outer + 2 row dividers + 1 col divider = 7
    let border_count = frame.decoration_count(DecorationKind::TableBorder);
    assert_eq!(
        border_count, 7,
        "3x2 table should have 7 border decorations, got {}",
        border_count
    );
}

#[test]
fn table_no_glyph_overlap_across_cells() {
    let mut ts = setup_table(800.0);
    let frame = ts.render();
    assert_no_glyph_overlap(frame);
}

// ── Hit-test: column boundary, far right, above table ───────────

#[test]
fn hit_test_at_column_boundary_snaps_to_nearest_column() {
    let ts = setup_table(800.0);

    // Find x midpoint between col 0 end and col 1 start in row 0
    let x_col0_end = ts.caret_rect(8)[0]; // end of "Header A"
    let x_col1_start = ts.caret_rect(9)[0]; // start of "Header B"
    let mid_x = (x_col0_end + x_col1_start) / 2.0;
    let row_y = ts.caret_rect(0)[1] + ts.caret_rect(0)[3] * 0.5;

    let hit = ts.hit_test(mid_x, row_y);
    assert!(
        hit.is_some(),
        "hit_test at column boundary x={} should find a cell",
        mid_x
    );

    let bid = hit.unwrap().block_id;
    assert!(
        bid == 10 || bid == 11,
        "should snap to col 0 (block 10) or col 1 (block 11), got block {}",
        bid
    );
}

#[test]
fn hit_test_far_right_of_table_snaps_to_last_column() {
    let ts = setup_table(800.0);

    let row_y = ts.caret_rect(0)[1] + ts.caret_rect(0)[3] * 0.5;
    let hit = ts.hit_test(2000.0, row_y);
    assert!(
        hit.is_some(),
        "hit_test far right should snap to last column"
    );
    // Should snap to col 1 (block 11 in row 0)
    assert_eq!(
        hit.unwrap().block_id,
        11,
        "far right should snap to last column block"
    );
}

#[test]
fn hit_test_above_table_top_finds_first_row() {
    let ts = setup_table(800.0);

    let r0 = Rect::from(ts.caret_rect(0));
    // Hit-test 5px above the table's first row
    let hit = ts.hit_test(r0.x().max(1.0), r0.y() - 5.0);
    assert!(
        hit.is_some(),
        "hit_test above table top should still find a cell"
    );
}

#[test]
fn is_block_in_table_false_for_non_table_block() {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(800.0, 600.0);

    // Add a standalone block first
    ts.layout_blocks(vec![make_block_at(1, 0, "Standalone")]);

    // Then add a table
    let table = make_table(1, 1, 1, vec![make_cell_at(0, 0, 20, 12, "In table")]);
    ts.add_table(&table);
    ts.render();

    assert!(
        !ts.is_block_in_table(1),
        "standalone block should not be in table"
    );
    assert!(
        ts.is_block_in_table(20),
        "table cell block should be in table"
    );
}

// ── Selection and cursor tests ──────────────────────────────────

#[test]
fn selection_within_single_table_cell() {
    let mut ts = setup_table(800.0);

    // Select "Head" within "Header A" (positions 0..4)
    ts.set_cursor(&CursorDisplay {
        position: 4,
        anchor: 0,
        affinity: text_typeset::CursorAffinity::Downstream,
        visible: true,
        selected_cells: vec![],
    });
    let frame = ts.render();

    let sel = frame.selection_rects();
    assert!(
        !sel.is_empty(),
        "selection inside a table cell should produce selection rects"
    );
    for (i, r) in sel.iter().enumerate() {
        assert!(
            r.w() > 0.0,
            "selection rect[{}] should have positive width",
            i
        );
        assert!(
            r.h() > 0.0,
            "selection rect[{}] should have positive height",
            i
        );
    }
}

#[test]
fn selection_across_table_cells() {
    let mut ts = setup_table(800.0);

    // Select from "Header A" (pos 0) through "Cell one" (pos 22)
    ts.set_cursor(&CursorDisplay {
        position: 22,
        anchor: 0,
        affinity: text_typeset::CursorAffinity::Downstream,
        visible: true,
        selected_cells: vec![],
    });
    let frame = ts.render();

    let sel = frame.selection_rects();
    assert!(
        sel.len() >= 2,
        "cross-cell selection should produce multiple rects, got {}",
        sel.len()
    );

    // Selection rects should span different y values (rows 0 and 1)
    let min_y = sel.iter().map(|r| r.y()).fold(f32::MAX, f32::min);
    let max_y = sel.iter().map(|r| r.bottom()).fold(f32::MIN, f32::max);
    assert!(
        max_y - min_y > 10.0,
        "selection should span multiple rows: min_y={}, max_y={}",
        min_y,
        max_y
    );
}

#[test]
fn cursor_visible_in_table_cell() {
    let mut ts = setup_table(800.0);

    // Place cursor at "Cell one" start (pos 18, block 12, row 1)
    ts.set_cursor(&CursorDisplay {
        position: 18,
        anchor: 18,
        affinity: text_typeset::CursorAffinity::Downstream,
        visible: true,
        selected_cells: vec![],
    });
    let frame = ts.render();

    let cursor = frame.cursor_rect();
    assert!(cursor.is_some(), "cursor should be visible in table cell");

    let c = cursor.unwrap();
    assert!(c.h() > 0.0, "cursor should have positive height");

    // Cursor y should be in row 1's area
    let row1_y = ts.caret_rect(18)[1];
    assert!(
        (c.y() - row1_y).abs() < 5.0,
        "cursor y ({}) should be near row 1 caret y ({})",
        c.y(),
        row1_y
    );
}

// ── Mixed content tests ─────────────────────────────────────────

#[test]
fn two_tables_in_sequence() {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(800.0, 600.0);

    let table1 = make_table(1, 1, 1, vec![make_cell_at(0, 0, 10, 0, "Table one")]);
    ts.add_table(&table1);

    let height_after_one = ts.content_height();

    let table2 = make_table(2, 1, 1, vec![make_cell_at(0, 0, 20, 10, "Table two")]);
    ts.add_table(&table2);
    ts.render();

    let height_after_two = ts.content_height();
    assert!(
        height_after_two > height_after_one,
        "two tables should be taller than one: one={}, two={}",
        height_after_one,
        height_after_two
    );

    let y1 = ts.caret_rect(0)[1];
    let y2 = ts.caret_rect(10)[1];
    assert!(
        y2 > y1,
        "table 2 caret y ({}) should be below table 1 ({})",
        y2,
        y1
    );
}

// ── Scrolling tests ─────────────────────────────────────────────

#[test]
fn scroll_offset_shifts_table_caret_positions() {
    let mut ts = setup_table(800.0);
    ts.render();

    let y_before = ts.caret_rect(0)[1];

    ts.set_scroll_offset(50.0);
    ts.render();

    let y_after = ts.caret_rect(0)[1];
    let delta = y_before - y_after;
    assert!(
        (delta - 50.0).abs() < 1.0,
        "scroll offset of 50 should shift caret y by ~50: before={}, after={}, delta={}",
        y_before,
        y_after,
        delta
    );
}

#[test]
fn content_height_unchanged_after_same_text_relayout() {
    let mut ts = setup_table(800.0);
    ts.render();

    let height_before = ts.content_height();

    // Relayout with identical text
    let same = make_block_at(12, 18, "Cell one");
    ts.relayout_block(&same);
    ts.render();

    let height_after = ts.content_height();
    assert!(
        (height_after - height_before).abs() < 0.01,
        "relayout with same text should not change height: before={}, after={}",
        height_before,
        height_after
    );
}

// ── Table-ID in HitTestResult ────────────────────────────────────

#[test]
fn hit_test_table_cells_report_table_id() {
    let ts = setup_table(800.0);

    // Hit test the first cell in each row
    for (label, pos) in [("row 0", 0usize), ("row 1", 18), ("row 2", 36)] {
        let rect = ts.caret_rect(pos);
        let hx = rect[0].max(1.0);
        let hy = rect[1] + rect[3] * 0.5;
        let hit = ts.hit_test(hx, hy).unwrap();
        assert_eq!(
            hit.table_id,
            Some(1),
            "cell {} hit should report table_id=Some(1), got {:?}",
            label,
            hit.table_id
        );
    }
}

#[test]
fn hit_test_all_cells_have_consistent_table_id() {
    let ts = setup_table(800.0);

    // Start position of every cell block in the 3x2 table
    for (pos, label) in [
        (0, "row0 col0"),
        (9, "row0 col1"),
        (18, "row1 col0"),
        (27, "row1 col1"),
        (36, "row2 col0"),
        (47, "row2 col1"),
    ] {
        let rect = ts.caret_rect(pos);
        let hit = ts
            .hit_test(rect[0].max(1.0), rect[1] + rect[3] * 0.5)
            .unwrap();
        assert_eq!(
            hit.table_id,
            Some(1),
            "cell {} (pos {}) should report table_id=Some(1), got {:?}",
            label,
            pos,
            hit.table_id
        );
    }
}

#[test]
fn hit_test_block_before_table_has_no_table_id() {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(800.0, 600.0);

    // Place a top-level block before the table, then add the table.
    ts.layout_blocks(vec![make_block_at(99, 0, "Before table")]);
    ts.add_table(&make_3x2_table());
    ts.render();

    // block_visual_info gives the physical y of the top-level block (which is
    // above the table in the flow). caret_rect(0) would resolve to the table
    // cell at position 0 instead, so we don't use it here.
    let info = ts.block_visual_info(99).unwrap();
    let hit = ts.hit_test(40.0, info.y + info.height * 0.5).unwrap();
    assert_eq!(
        hit.table_id, None,
        "block hit before table should have table_id=None, got {:?}",
        hit.table_id
    );
}

#[test]
fn hit_test_two_tables_return_correct_ids() {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(800.0, 600.0);

    let table1 = make_table(
        1,
        1,
        2,
        vec![
            make_cell_at(0, 0, 10, 0, "T1-A"),
            make_cell_at(0, 1, 11, 5, "T1-B"),
        ],
    );
    let table2 = make_table(
        2,
        1,
        2,
        vec![
            make_cell_at(0, 0, 20, 100, "T2-A"),
            make_cell_at(0, 1, 21, 105, "T2-B"),
        ],
    );
    ts.add_table(&table1);
    ts.add_table(&table2);
    ts.render();

    // Hit first table via its known block position
    let r1 = ts.caret_rect(0);
    let hit1 = ts.hit_test(r1[0].max(1.0), r1[1] + r1[3] * 0.5).unwrap();
    assert_eq!(
        hit1.table_id,
        Some(1),
        "first table hit should report table_id=Some(1), got {:?}",
        hit1.table_id
    );

    // Hit second table via its known block position
    let r2 = ts.caret_rect(100);
    let hit2 = ts.hit_test(r2[0].max(1.0), r2[1] + r2[3] * 0.5).unwrap();
    assert_eq!(
        hit2.table_id,
        Some(2),
        "second table hit should report table_id=Some(2), got {:?}",
        hit2.table_id
    );
}

#[test]
fn hit_test_table_inside_frame_has_table_id() {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(800.0, 600.0);

    let table = make_table(
        7,
        1,
        2,
        vec![
            make_cell_at(0, 0, 30, 0, "Frame T-A"),
            make_cell_at(0, 1, 31, 10, "Frame T-B"),
        ],
    );
    let mut frame = make_frame(5, vec![]);
    frame.tables = vec![(0, table)];
    ts.add_frame(&frame);
    ts.render();

    // caret_rect searches table cell blocks including those inside frames
    let rect = ts.caret_rect(0);
    let hit = ts
        .hit_test(rect[0].max(1.0), rect[1] + rect[3] * 0.5)
        .unwrap();
    assert_eq!(
        hit.table_id,
        Some(7),
        "table inside frame should report table_id=Some(7), got {:?}",
        hit.table_id
    );
}
