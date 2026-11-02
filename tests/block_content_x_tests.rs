//! Where a block's text starts: `BlockVisualInfo::content_x` and the marker box.
//!
//! The line rectangles `block_geometry` reports are relative to the start of
//! the block's text, which sits after the block's left margin and its list or
//! paragraph indent, while `BlockVisualInfo::x` is the column edge. A caller
//! placing text rectangles from `x` alone puts every indented block left of
//! where it is painted. These tests use the render frame as the oracle: the
//! geometry must agree with the glyph quads the renderer emits.

mod helpers;

use helpers::{
    NOTO_ARABIC, NOTO_SANS, Typesetter, make_block, make_cell_at, make_frame, make_table,
};
use text_typeset::layout::paragraph::Alignment;
use text_typeset::shaping::shaper::TextDirection;
use text_typeset::{BlockVisualInfo, LineGeometry};

/// A glyph's left edge is its pen position plus a side bearing, a few pixels
/// at most for the 16 px test face. Margins and indents below are far larger,
/// so agreement within this tolerance cannot be a coincidence.
const BEARING: f32 = 3.0;

/// Text starting with a letter that has a small left bearing.
const TEXT: &str = "Hello world";

fn typesetter() -> Typesetter {
    let mut ts = make_typesetter_with_width();
    ts.render();
    ts
}

fn make_typesetter_with_width() -> Typesetter {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(800.0, 600.0);
    ts
}

/// The x extent `(min_left, max_right)` of the rendered glyphs of the first
/// line, restricted to `x_range`, at scroll 0 and zoom 1.
fn glyph_extent(
    ts: &mut Typesetter,
    info: &BlockVisualInfo,
    line: &LineGeometry,
    x_range: (f32, f32),
) -> (f32, f32) {
    let top = info.y + line.rect[1];
    let bottom = top + line.rect[3];
    let frame = ts.render();
    let mut left = f32::INFINITY;
    let mut right = f32::NEG_INFINITY;
    for g in &frame.glyphs {
        let cy = g.screen[1] + g.screen[3] / 2.0;
        if cy < top - 1.0 || cy > bottom + 1.0 {
            continue;
        }
        if g.screen[0] < x_range.0 || g.screen[0] >= x_range.1 {
            continue;
        }
        left = left.min(g.screen[0]);
        right = right.max(g.screen[0] + g.screen[2]);
    }
    assert!(
        left.is_finite(),
        "no glyph painted on the line, in {x_range:?}"
    );
    (left, right)
}

/// The text of `info`'s first line starts at `content_x + line.rect[0]`.
fn assert_text_starts_at_content_x(ts: &mut Typesetter, block_id: usize, text: &str) {
    let (info, lines) = ts.block_geometry(block_id, text).expect("laid out");
    let line = &lines[0];
    let (left, _) = glyph_extent(ts, &info, line, (info.content_x - BEARING, f32::INFINITY));
    let expected = info.content_x + line.rect[0];
    assert!(
        (left - expected).abs() <= BEARING,
        "first glyph painted at {left}, geometry says the text starts at {expected} \
         (x = {}, content_x = {})",
        info.x,
        info.content_x
    );
}

#[test]
fn a_plain_paragraph_starts_its_text_at_the_column_edge() {
    let mut ts = typesetter();
    ts.layout_blocks(vec![make_block(1, TEXT)]);
    let (info, _) = ts.block_geometry(1, TEXT).unwrap();
    assert_eq!(info.content_x, info.x);
    assert_eq!(info.marker_x, None);
    assert_eq!(info.marker_width, 0.0);
    assert_text_starts_at_content_x(&mut ts, 1, TEXT);
}

#[test]
fn the_two_accessors_agree() {
    let mut ts = typesetter();
    let mut b = make_block(1, TEXT);
    b.left_margin = 10.0;
    b.list_indent = 24.0;
    b.list_marker = "\u{2022}".into();
    ts.layout_blocks(vec![b]);
    let a = ts.block_visual_info(1).unwrap();
    let (b, _) = ts.block_geometry(1, TEXT).unwrap();
    assert_eq!(a.x, b.x);
    assert_eq!(a.content_x, b.content_x);
    assert_eq!(a.marker_x, b.marker_x);
    assert_eq!(a.marker_width, b.marker_width);
    assert_eq!(a.y, b.y);
    assert_eq!(a.height, b.height);
}

#[test]
fn a_list_item_starts_its_text_after_the_indent_and_draws_its_marker_before() {
    let mut ts = typesetter();
    let mut b = make_block(1, TEXT);
    b.list_marker = "\u{2022}".into();
    b.list_indent = 24.0;
    ts.layout_blocks(vec![b]);
    let (info, lines) = ts.block_geometry(1, TEXT).unwrap();
    assert_eq!(info.x, 0.0);
    assert_eq!(info.content_x, 24.0, "the list indent is counted once");
    assert_text_starts_at_content_x(&mut ts, 1, TEXT);

    // The marker box is in the indent area, left of the text.
    let marker_x = info.marker_x.expect("a bullet has a marker box");
    assert!(info.marker_width > 0.0);
    assert!(marker_x >= info.x);
    assert!(marker_x + info.marker_width <= info.content_x);
    let (left, right) = glyph_extent(&mut ts, &info, &lines[0], (-1000.0, info.content_x - 0.5));
    assert!((left - marker_x).abs() <= BEARING, "{left} vs {marker_x}");
    assert!(right <= marker_x + info.marker_width + BEARING);
}

#[test]
fn a_numbered_item_with_a_left_margin_adds_margin_and_indent_once() {
    let mut ts = typesetter();
    let mut b = make_block(1, TEXT);
    b.left_margin = 10.0;
    b.list_marker = "12.".into();
    b.list_indent = 30.0;
    ts.layout_blocks(vec![b]);
    let (info, lines) = ts.block_geometry(1, TEXT).unwrap();
    assert_eq!(info.content_x, 40.0);
    assert_text_starts_at_content_x(&mut ts, 1, TEXT);
    let marker_x = info.marker_x.unwrap();
    // The marker sits inside the area between the margin and the text.
    assert!(marker_x >= info.x + 10.0 - f32::EPSILON);
    let (left, _) = glyph_extent(&mut ts, &info, &lines[0], (-1000.0, info.content_x - 0.5));
    assert!((left - marker_x).abs() <= BEARING);
}

#[test]
fn a_checkbox_item_reports_its_marker_box() {
    for checked in [false, true] {
        let mut ts = typesetter();
        let mut b = make_block(1, TEXT);
        b.checkbox = Some(checked);
        b.list_indent = 28.0;
        ts.layout_blocks(vec![b]);
        let (info, lines) = ts.block_geometry(1, TEXT).unwrap();
        assert_eq!(info.content_x, 28.0);
        assert_text_starts_at_content_x(&mut ts, 1, TEXT);
        let marker_x = info.marker_x.expect("a checkbox has a marker box");
        assert!(info.marker_width > 0.0);
        let (left, _) = glyph_extent(&mut ts, &info, &lines[0], (-1000.0, info.content_x - 0.5));
        assert!(
            (left - marker_x).abs() <= BEARING,
            "checked={checked}: {left} vs {marker_x}"
        );
    }
}

#[test]
fn an_indented_paragraph_shifts_its_text_by_exactly_the_indent() {
    // A paragraph indent (BlockFormat.indent) arrives as `list_indent`, with no marker.
    let mut flat = typesetter();
    flat.layout_blocks(vec![make_block(1, TEXT)]);
    let (flat_info, flat_lines) = flat.block_geometry(1, TEXT).unwrap();
    let (flat_left, _) = glyph_extent(
        &mut flat,
        &flat_info,
        &flat_lines[0],
        (-1000.0, f32::INFINITY),
    );

    let mut ts = typesetter();
    let mut b = make_block(1, TEXT);
    b.list_indent = 36.0;
    b.left_margin = 4.0;
    ts.layout_blocks(vec![b]);
    let (info, lines) = ts.block_geometry(1, TEXT).unwrap();
    assert_eq!(info.x, 0.0);
    assert_eq!(info.content_x, 40.0);
    assert_eq!(info.marker_x, None);
    // The line rect is relative to content_x: identical to the flat one.
    assert_eq!(lines[0].rect[0], flat_lines[0].rect[0]);
    let (left, _) = glyph_extent(&mut ts, &info, &lines[0], (-1000.0, f32::INFINITY));
    assert!(
        (left - flat_left - 40.0).abs() < 0.01,
        "painted glyphs moved by {} not 40",
        left - flat_left
    );
}

#[test]
fn a_table_cell_block_has_the_cell_origin_plus_its_indent() {
    let mut ts = typesetter();
    let mut indented = make_block(101, TEXT);
    indented.list_marker = "1.".into();
    indented.list_indent = 20.0;
    let cell0 = make_cell_at(0, 0, 100, 0, "Left");
    let mut cell1 = make_cell_at(0, 1, 101, 5, TEXT);
    cell1.blocks = vec![indented];
    ts.add_table(&make_table(1, 1, 2, vec![cell0, cell1]));
    ts.render();
    let (plain, _) = ts.block_geometry(100, "Left").unwrap();
    let (info, lines) = ts.block_geometry(101, TEXT).unwrap();
    assert!(info.x > plain.x, "column 1 starts right of column 0");
    assert_eq!(plain.content_x, plain.x);
    assert!((info.content_x - (info.x + 20.0)).abs() < 0.001);
    assert_text_starts_at_content_x(&mut ts, 101, TEXT);
    let marker_x = info.marker_x.unwrap();
    assert!(marker_x >= info.x && marker_x + info.marker_width <= info.content_x);
    let (left, _) = glyph_extent(
        &mut ts,
        &info,
        &lines[0],
        (info.x - 1.0, info.content_x - 0.5),
    );
    assert!((left - marker_x).abs() <= BEARING);
}

#[test]
fn a_frame_block_has_the_frame_origin_plus_its_indent() {
    let mut ts = typesetter();
    let mut inner = make_block(2, TEXT);
    inner.left_margin = 6.0;
    inner.list_marker = "\u{2022}".into();
    inner.list_indent = 18.0;
    let mut frame = make_frame(1, vec![inner]);
    frame.margin_left = 30.0;
    ts.layout_blocks(vec![make_block(1, "Before")]);
    ts.add_frame(&frame);
    ts.render();
    let (info, lines) = ts.block_geometry(2, TEXT).unwrap();
    assert!(
        info.x > 30.0,
        "the frame has its own left origin: {}",
        info.x
    );
    assert!((info.content_x - (info.x + 24.0)).abs() < 0.001);
    assert_text_starts_at_content_x(&mut ts, 2, TEXT);
    let marker_x = info.marker_x.unwrap();
    let (left, _) = glyph_extent(
        &mut ts,
        &info,
        &lines[0],
        (info.x - 1.0, info.content_x - 0.5),
    );
    assert!((left - marker_x).abs() <= BEARING);
}

fn arabic_typesetter() -> Typesetter {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_ARABIC);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(800.0, 600.0);
    ts
}

/// "kataba salaam": two Arabic words.
const ARABIC: &str = "\u{0643}\u{062A}\u{0628} \u{0633}\u{0644}\u{0627}\u{0645}";

#[test]
fn an_rtl_list_item_ends_its_text_at_the_line_rect_and_keeps_the_marker_at_the_left() {
    let mut ts = arabic_typesetter();
    ts.set_content_width(400.0);
    let mut b = make_block(1, ARABIC);
    b.base_direction = TextDirection::RightToLeft;
    b.alignment = Alignment::Right;
    b.list_marker = "\u{2022}".into();
    b.list_indent = 24.0;
    ts.layout_blocks(vec![b]);
    ts.render();
    let (info, lines) = ts.block_geometry(1, ARABIC).unwrap();
    assert_eq!(info.content_x, 24.0);
    let line = &lines[0];

    // The text: rightmost glyph edge equals the line rect's right edge.
    let (left, right) = glyph_extent(
        &mut ts,
        &info,
        line,
        (info.content_x - BEARING, f32::INFINITY),
    );
    let rect_left = info.content_x + line.rect[0];
    let rect_right = rect_left + line.rect[2];
    assert!(
        (right - rect_right).abs() <= BEARING,
        "text ends at {right}, line rect ends at {rect_right}"
    );
    assert!(left >= rect_left - BEARING, "{left} vs {rect_left}");

    // The marker is in the indent area at the left, as in a left-to-right item.
    let marker_x = info.marker_x.unwrap();
    assert!(marker_x + info.marker_width <= info.content_x);
    let (mleft, _) = glyph_extent(&mut ts, &info, line, (-1000.0, info.content_x - 0.5));
    assert!((mleft - marker_x).abs() <= BEARING, "{mleft} vs {marker_x}");
}

#[test]
fn a_marker_wider_than_the_indent_starts_at_the_left_margin_and_overlaps_the_text() {
    let mut ts = typesetter();
    let mut b = make_block(1, TEXT);
    b.left_margin = 10.0;
    b.list_marker = "\u{2022}".into();
    b.list_indent = 0.0;
    ts.layout_blocks(vec![b]);
    let (info, _) = ts.block_geometry(1, TEXT).unwrap();
    assert_eq!(info.content_x, info.x + 10.0);
    assert_eq!(
        info.marker_x,
        Some(info.x + 10.0),
        "the marker never starts left of the left margin"
    );
    assert!(info.marker_width > 0.0);
    assert!(
        info.marker_x.unwrap() + info.marker_width > info.content_x,
        "with no indent the marker box runs into the text"
    );
}

/// The bridge turns a paragraph indent into `list_indent` and a list item's
/// level into its own indent; hand-built params do not exercise that.
#[test]
fn a_real_document_reports_content_x_for_an_indented_paragraph_and_a_nested_item() {
    use text_document::{BlockFormat, TextDocument};

    let doc = TextDocument::new();
    doc.set_markdown("Plain\n\n- one\n  - two\n")
        .unwrap()
        .wait()
        .unwrap();
    let cursor = doc.cursor();
    cursor
        .set_block_format(&BlockFormat {
            indent: Some(2),
            ..Default::default()
        })
        .unwrap();

    let mut ts = typesetter();
    ts.layout_full(&doc.snapshot_flow());
    ts.render();

    let mut by_text = std::collections::HashMap::new();
    let mut pos = 0;
    for _ in 0..3 {
        let block = doc.block_at_position(pos).expect("block");
        by_text.insert(block.text(), block.id());
        pos = block.position() + block.text().chars().count() + 1;
    }
    let info_of = |ts: &Typesetter, t: &str| ts.block_geometry(by_text[t], t).unwrap().0;

    let plain = info_of(&ts, "Plain");
    let one = info_of(&ts, "one");
    let two = info_of(&ts, "two");
    assert_eq!(
        plain.content_x - plain.x,
        48.0,
        "indent 2 is two levels of 24"
    );
    assert_eq!(plain.marker_x, None);
    assert!(one.marker_x.is_some() && two.marker_x.is_some());
    assert!(one.content_x - one.x > 0.0);
    assert!(
        two.content_x > one.content_x,
        "the nested item's text starts right of its parent's: {} vs {}",
        two.content_x,
        one.content_x
    );
    for t in ["Plain", "one", "two"] {
        assert_text_starts_at_content_x(&mut ts, by_text[t], t);
    }
}
