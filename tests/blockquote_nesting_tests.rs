//! Tables and lists nested inside blockquotes — static geometry, render,
//! and incremental relayout.
//!
//! Regression coverage for two incremental-layout bugs:
//! - frame children were re-stacked grouped by kind (all blocks, then all
//!   tables, then nested frames) on relayout, scrambling interleaved
//!   content like [block, table, block];
//! - blocks inside cells of a table nested in a blockquote were invisible
//!   to `relayout_block` (silent no-op, stale display).

mod helpers;
use helpers::make_typesetter;

use text_document::{FlowElementSnapshot, TextDocument};
use text_typeset::DecorationKind;

const QUOTE_BLOCK_TABLE_BLOCK: &str =
    "> First paragraph\n>\n> | a | b |\n> |---|---|\n> | c | d |\n>\n> Last paragraph";

fn doc_with_markdown(md: &str) -> TextDocument {
    let doc = TextDocument::new();
    doc.set_markdown(md).unwrap().wait().unwrap();
    doc
}

/// The blockquote `FrameSnapshot` of a single-quote document, panicking
/// with context when the structure is not [Frame].
fn quote_frame(doc: &TextDocument) -> text_document::FrameSnapshot {
    let snap = doc.snapshot_flow();
    match snap.elements.first() {
        Some(FlowElementSnapshot::Frame(f)) => f.clone(),
        other => panic!("expected a blockquote frame as first element, got {other:?}"),
    }
}

/// Direct child blocks of a frame snapshot, in document order.
fn frame_blocks(frame: &text_document::FrameSnapshot) -> Vec<text_document::BlockSnapshot> {
    frame
        .elements
        .iter()
        .filter_map(|e| match e {
            FlowElementSnapshot::Block(b) => Some(b.clone()),
            _ => None,
        })
        .collect()
}

/// First cell block of the first nested table of a frame snapshot.
fn first_cell_block(frame: &text_document::FrameSnapshot) -> text_document::BlockSnapshot {
    frame
        .elements
        .iter()
        .find_map(|e| match e {
            FlowElementSnapshot::Table(t) => t.cells.first().and_then(|c| c.blocks.first()).cloned(),
            _ => None,
        })
        .expect("frame contains a table with a populated first cell")
}

/// Vertical extent `(top, bottom)` of all table-border decorations.
fn table_border_extent(frame: &text_typeset::RenderFrame) -> (f32, f32) {
    let mut top = f32::MAX;
    let mut bottom = f32::MIN;
    let mut found = false;
    for d in &frame.decorations {
        if d.kind == DecorationKind::TableBorder {
            found = true;
            top = top.min(d.rect[1]);
            bottom = bottom.max(d.rect[1] + d.rect[3]);
        }
    }
    assert!(found, "render frame contains TableBorder decorations");
    (top, bottom)
}

// ── T1: static geometry — table between sibling blocks inside the quote ──

#[test]
fn table_in_blockquote_sits_between_sibling_blocks() {
    let doc = doc_with_markdown(QUOTE_BLOCK_TABLE_BLOCK);
    let mut ts = make_typesetter();
    ts.layout_full(&doc.snapshot_flow());
    let frame = ts.render();
    let (table_top, table_bottom) = table_border_extent(frame);

    let bq = quote_frame(&doc);
    let blocks = frame_blocks(&bq);
    assert_eq!(blocks.len(), 2, "frame elements: {:?}", bq.elements);

    let first_rect = ts.caret_rect(blocks[0].position);
    let last_rect = ts.caret_rect(blocks[1].position);

    assert!(
        first_rect[1] + first_rect[3] <= table_top + 0.5,
        "first paragraph (bottom {}) must sit above the table (top {})",
        first_rect[1] + first_rect[3],
        table_top
    );
    assert!(
        last_rect[1] >= table_bottom - 0.5,
        "last paragraph (top {}) must sit below the table (bottom {})",
        last_rect[1],
        table_bottom
    );
    assert!(
        table_bottom <= ts.content_height() + 0.5,
        "table stays within the document content"
    );
}

// ── T2: list markers render inside the quote ──

#[test]
fn list_in_blockquote_renders_markers_left_of_text() {
    let doc = doc_with_markdown("> - item1\n> - item2");
    let mut ts = make_typesetter();
    ts.layout_full(&doc.snapshot_flow());
    let frame = ts.render();
    assert!(!frame.glyphs.is_empty());

    let bq = quote_frame(&doc);
    let items = frame_blocks(&bq);
    assert_eq!(items.len(), 2);
    for item in &items {
        assert!(
            item.list_info.is_some(),
            "quoted list item carries list_info"
        );
    }

    // The marker glyph is drawn left of the item text on the same line.
    let glyphs = frame.glyphs.clone();
    for item in &items {
        let text_rect = ts.caret_rect(item.position);
        let line_top = text_rect[1];
        let line_bottom = text_rect[1] + text_rect[3];
        let has_marker_glyph = glyphs.iter().any(|g| {
            let gx = g.screen[0];
            let gy = g.screen[1];
            gx < text_rect[0] && gy >= line_top - 2.0 && gy <= line_bottom + 2.0
        });
        assert!(
            has_marker_glyph,
            "expected a marker glyph left of x={} around y={}..{}",
            text_rect[0], line_top, line_bottom
        );
    }
}

// ── T3: incremental relayout preserves interleaved order ──

#[test]
fn relayout_first_block_keeps_table_between_blocks() {
    let doc = doc_with_markdown(QUOTE_BLOCK_TABLE_BLOCK);
    let mut ts = make_typesetter();
    ts.layout_full(&doc.snapshot_flow());
    let (table_top_before, _) = {
        let frame = ts.render();
        table_border_extent(frame)
    };

    // Grow the first quoted paragraph enough to add lines.
    let bq = quote_frame(&doc);
    let first_block = &frame_blocks(&bq)[0];
    let cursor = doc.cursor();
    cursor.set_position(first_block.position, text_document::MoveMode::MoveAnchor);
    cursor
        .insert_text(&"growing text that wraps onto several lines once it is long enough ".repeat(4))
        .unwrap();

    let block_snapshot = doc
        .snapshot_block_at_position(first_block.position)
        .expect("block at cursor");
    ts.relayout_block(&text_typeset::bridge::convert_block(&block_snapshot));

    let frame = ts.render();
    let (table_top_after, table_bottom_after) = table_border_extent(frame);

    assert!(
        table_top_after > table_top_before,
        "table must move down when the paragraph above it grows: {} -> {}",
        table_top_before,
        table_top_after
    );

    // Re-read positions (the edit shifted them) and check the order is intact.
    let bq = quote_frame(&doc);
    let blocks = frame_blocks(&bq);
    let first_rect = ts.caret_rect(blocks[0].position);
    let last_rect = ts.caret_rect(blocks[1].position);
    assert!(
        first_rect[1] + first_rect[3] <= table_top_after + 0.5,
        "first paragraph still above the table after relayout"
    );
    assert!(
        last_rect[1] >= table_bottom_after - 0.5,
        "last paragraph still below the table after relayout \
         (table must NOT be re-stacked after the blocks): last top {}, table bottom {}",
        last_rect[1],
        table_bottom_after
    );
}

// ── T4: editing a cell of a table-in-blockquote is not a silent no-op ──

#[test]
fn relayout_cell_block_in_quoted_table_grows_content() {
    let doc = doc_with_markdown("> | a | b |\n> |---|---|\n> | c | d |");
    let mut ts = make_typesetter();
    ts.layout_full(&doc.snapshot_flow());
    ts.render();
    let height_before = ts.content_height();

    let bq = quote_frame(&doc);
    let cell_block = first_cell_block(&bq);
    let cursor = doc.cursor();
    cursor.set_position(cell_block.position, text_document::MoveMode::MoveAnchor);
    cursor
        .insert_text("a much longer cell content that has to wrap onto multiple lines in the narrow column ")
        .unwrap();

    let block_snapshot = doc
        .snapshot_block_at_position(cell_block.position)
        .expect("cell block at cursor");
    ts.relayout_block(&text_typeset::bridge::convert_block(&block_snapshot));

    let height_after = ts.content_height();
    assert!(
        height_after > height_before,
        "growing a cell of a table inside a blockquote must grow the layout \
         (silent no-op regression): {} -> {}",
        height_before,
        height_after
    );

    // The glyphs for the new text actually render.
    let frame = ts.render();
    assert!(!frame.glyphs.is_empty());
}

// ── T5: nested blockquote with table ──

#[test]
fn table_in_nested_blockquote_lays_out_and_relayouts() {
    let doc = doc_with_markdown("> Outer\n> > | X | Y |\n> > |---|---|\n> > | a | b |");
    let mut ts = make_typesetter();
    ts.layout_full(&doc.snapshot_flow());
    let frame = ts.render();
    let (table_top, table_bottom) = table_border_extent(frame);
    assert!(table_bottom > table_top);

    // Find the inner quote's table cell block through the snapshots.
    let outer = quote_frame(&doc);
    let inner = outer
        .elements
        .iter()
        .find_map(|e| match e {
            FlowElementSnapshot::Frame(f) => Some(f.clone()),
            _ => None,
        })
        .expect("outer quote contains the inner quote");
    assert_eq!(inner.format.is_blockquote, Some(true));
    let cell_block = first_cell_block(&inner);

    let height_before = ts.content_height();
    let cursor = doc.cursor();
    cursor.set_position(cell_block.position, text_document::MoveMode::MoveAnchor);
    cursor
        .insert_text("longer nested cell content that needs to wrap across lines in this column ")
        .unwrap();

    let block_snapshot = doc
        .snapshot_block_at_position(cell_block.position)
        .expect("nested cell block at cursor");
    ts.relayout_block(&text_typeset::bridge::convert_block(&block_snapshot));

    assert!(
        ts.content_height() > height_before,
        "editing a cell two frames deep must propagate heights: {} -> {}",
        height_before,
        ts.content_height()
    );
}

// ── Quote followed by a top-level table keeps its own left bar ──
// (regression: the widget-catalog editor sample — a one-paragraph quote,
// then a sibling table — must render BOTH the quote bar and the table)

#[test]
fn quote_before_top_level_table_keeps_left_bar() {
    let doc = doc_with_markdown(
        "Type here\n\n- editable bullet\n- second bullet\n\n> A blockquote you can edit.\n\n| Knob | Value |\n| --- | --- |\n| min_lines | 3 |",
    );
    let mut ts = make_typesetter();
    ts.layout_full(&doc.snapshot_flow());
    let frame = ts.render();
    let (table_top, _) = table_border_extent(frame);

    let bar = frame
        .decorations
        .iter()
        .filter(|d| d.kind == DecorationKind::Background)
        .filter(|d| d.rect[2] < 10.0 && d.rect[3] > d.rect[2])
        .max_by(|a, b| a.rect[3].total_cmp(&b.rect[3]))
        .expect("blockquote left bar must render when a table follows the quote");

    assert!(
        bar.rect[1] + bar.rect[3] <= table_top + 0.5,
        "quote (bar y {}..{}) sits above the sibling table (top {}), not around it",
        bar.rect[1],
        bar.rect[1] + bar.rect[3],
        table_top
    );
}

// ── T6: render smoke — quote bar spans the table ──

#[test]
fn blockquote_left_bar_covers_nested_table() {
    let doc = doc_with_markdown(QUOTE_BLOCK_TABLE_BLOCK);
    let mut ts = make_typesetter();
    ts.layout_full(&doc.snapshot_flow());
    let frame = ts.render();
    let (table_top, table_bottom) = table_border_extent(frame);

    // The blockquote's LeftOnly border is a thin, tall Background rect.
    let bar = frame
        .decorations
        .iter()
        .filter(|d| d.kind == DecorationKind::Background)
        .filter(|d| d.rect[2] < 10.0 && d.rect[3] > d.rect[2])
        .max_by(|a, b| a.rect[3].total_cmp(&b.rect[3]))
        .expect("blockquote left bar decoration present");

    assert!(
        bar.rect[1] <= table_top + 0.5 && bar.rect[1] + bar.rect[3] >= table_bottom - 0.5,
        "quote bar (y {}..{}) must span the table (y {}..{})",
        bar.rect[1],
        bar.rect[1] + bar.rect[3],
        table_top,
        table_bottom
    );

    // Cell text renders inside the quote: at least one glyph between the
    // table borders.
    assert!(
        frame
            .glyphs
            .iter()
            .any(|g| g.screen[1] >= table_top && g.screen[1] <= table_bottom),
        "cell glyphs render within the table extent"
    );
}
