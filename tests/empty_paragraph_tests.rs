//! The caret and metrics of an **empty paragraph**.
//!
//! An empty block produces one glyph-less line, and that line used to
//! answer every caret query with x = 0 and measure itself with the
//! registry's default face. Both answers disagree with what the first
//! keystroke produces — a glyph at the first-line indent (or wherever
//! alignment puts it), measured in the block's own font — so the caret
//! visibly jumped and resized on the first typed character, shifting
//! every block below. These tests pin the fixed behaviour: an empty
//! line's caret sits where the first character will land, and its
//! metrics come from the block's fragment formats when there are any.

mod helpers;

use helpers::{NOTO_SANS, Typesetter, make_block};
use text_typeset::CursorAffinity;
use text_typeset::layout::block::{BlockLayoutParams, layout_block};
use text_typeset::layout::paragraph::Alignment;
use text_typeset::shaping::shaper::TextDirection;

const AVAIL: f32 = 600.0;
const INDENT: f32 = 40.0;

fn latin() -> Typesetter {
    let mut ts = Typesetter::new();
    let sans = ts.register_font(NOTO_SANS);
    ts.set_default_font(sans, 16.0);
    ts.set_viewport(800.0, 600.0);
    ts
}

/// Lay the block out and return the caret x for offset 0 on its
/// (single) line.
fn caret_x(ts: &Typesetter, params: &BlockLayoutParams) -> f32 {
    let layout = layout_block(ts.font_registry(), params, AVAIL, 1.0, 1.0);
    assert_eq!(layout.lines.len(), 1, "expected exactly one line");
    layout.lines[0].x_for_offset_with_affinity(0, CursorAffinity::Downstream)
}

#[test]
fn an_empty_paragraph_puts_the_caret_at_the_first_line_indent() {
    let ts = latin();
    let mut params = make_block(0, "");
    params.text_indent = INDENT;

    assert_eq!(
        caret_x(&ts, &params),
        INDENT,
        "the caret on an empty indented paragraph must sit at the indent, \
         where the first typed character will land"
    );
}

#[test]
fn an_empty_unindented_paragraph_keeps_the_caret_at_zero() {
    let ts = latin();
    let params = make_block(0, "");

    assert_eq!(caret_x(&ts, &params), 0.0);
}

#[test]
fn the_first_keystroke_does_not_move_the_caret() {
    let ts = latin();

    let mut empty = make_block(0, "");
    empty.text_indent = INDENT;
    let before = caret_x(&ts, &empty);

    // The same paragraph one keystroke later: the caret that sat at
    // offset 0 now sits at the leading edge of the typed glyph.
    let mut typed = make_block(0, "a");
    typed.text_indent = INDENT;
    let after = caret_x(&ts, &typed);

    assert_eq!(
        before, after,
        "typing the first character must not move the caret's anchor"
    );
}

#[test]
fn an_empty_centered_paragraph_puts_the_caret_at_the_center() {
    let ts = latin();
    let mut params = make_block(0, "");
    params.alignment = Alignment::Center;

    assert_eq!(caret_x(&ts, &params), AVAIL / 2.0);
}

#[test]
fn an_empty_right_aligned_paragraph_puts_the_caret_at_the_right_edge() {
    let ts = latin();
    let mut params = make_block(0, "");
    params.alignment = Alignment::Right;

    assert_eq!(caret_x(&ts, &params), AVAIL);
}

#[test]
fn an_empty_rtl_paragraph_puts_the_caret_at_the_trailing_edge_minus_indent() {
    let ts = latin();
    let mut params = make_block(0, "");
    params.base_direction = TextDirection::RightToLeft;
    // `make_block` pins the explicit `Left`; RTL paragraphs normally carry
    // the direction-following `Start`, which resolves to `Right`.
    params.alignment = Alignment::Start;
    params.text_indent = INDENT;

    // A first-line indent insets from an RTL paragraph's *leading* edge —
    // the right one.
    assert_eq!(caret_x(&ts, &params), AVAIL - INDENT);
}

#[test]
fn an_empty_paragraph_measures_in_its_own_fragment_font() {
    let ts = latin();

    // An empty fragment still carries the block's character format — the
    // shape text-document / the typography-defaults fill hand over for an
    // empty paragraph. Its (here, doubled) size must drive the line box.
    let mut sized = make_block(0, "");
    sized.fragments[0].font_point_size = Some(32);
    let sized_layout = layout_block(ts.font_registry(), &sized, AVAIL, 1.0, 1.0);

    // The same block with no fragments at all: only the registry default
    // (16 px) is left to measure with.
    let mut bare = make_block(0, "");
    bare.fragments.clear();
    let bare_layout = layout_block(ts.font_registry(), &bare, AVAIL, 1.0, 1.0);

    assert!(
        sized_layout.lines[0].line_height > bare_layout.lines[0].line_height + 1.0,
        "an empty fragment's font size must reach the empty line's metrics \
         (sized {} vs bare {})",
        sized_layout.lines[0].line_height,
        bare_layout.lines[0].line_height,
    );

    // And the empty line's box equals the box its first character will
    // have, so nothing shifts on the first keystroke.
    let mut typed = make_block(0, "a");
    typed.fragments[0].font_point_size = Some(32);
    let typed_layout = layout_block(ts.font_registry(), &typed, AVAIL, 1.0, 1.0);
    assert_eq!(
        sized_layout.lines[0].line_height, typed_layout.lines[0].line_height,
        "the empty line and its one-character successor must share a line box"
    );
}
