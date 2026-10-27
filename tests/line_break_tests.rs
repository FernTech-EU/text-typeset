//! A line break inside a paragraph: U+2028 LINE SEPARATOR, the character a
//! word processor inserts for Shift+Enter.
//!
//! It ends a line, as `\n` always has here, and it is laid out as nothing:
//! no glyph and no width. A break at the paragraph's end starts an empty
//! line, a line it ends is not stretched by justification, and the offset
//! after it is the next line's start alone — for the caret, for a click past
//! the end of the line before, and for End.

mod helpers;
use helpers::{make_block, make_typesetter};

use text_typeset::CursorAffinity;
use text_typeset::LineEnd;
use text_typeset::TextFormat;
use text_typeset::font::resolve::resolve_font;
use text_typeset::layout::line::LayoutLine;
use text_typeset::layout::paragraph::{Alignment, RunOrder, break_into_lines};
use text_typeset::shaping::shaper::{font_metrics_px, shape_text};

const LS: char = '\u{2028}';

fn lines_of(text: &str, width: f32, alignment: Alignment) -> Vec<LayoutLine> {
    let ts = make_typesetter();
    let resolved =
        resolve_font(ts.font_registry(), None, None, None, None, None, 1.0, 1.0).unwrap();
    let run = shape_text(ts.font_registry(), &resolved, text, 0).unwrap();
    let metrics = font_metrics_px(ts.font_registry(), &resolved).unwrap();
    break_into_lines(
        vec![run],
        text,
        width,
        alignment,
        0.0,
        &metrics,
        None,
        RunOrder::AlreadyVisual,
    )
}

fn right_edge(line: &LayoutLine) -> f32 {
    line.runs
        .iter()
        .map(|run| run.x + run.shaped_run.advance_width)
        .fold(0.0, f32::max)
}

#[test]
fn a_line_separator_ends_the_line_and_is_laid_out_as_nothing() {
    let ts = make_typesetter();
    let resolved =
        resolve_font(ts.font_registry(), None, None, None, None, None, 1.0, 1.0).unwrap();
    let run = shape_text(ts.font_registry(), &resolved, &format!("a{LS}b"), 0).unwrap();
    let separator = run
        .glyphs
        .iter()
        .find(|glyph| glyph.cluster == 1)
        .expect("a glyph for the separator");
    assert_eq!(separator.glyph_id, 0, "never drawn");
    assert_eq!(separator.x_advance, 0.0, "no width");

    let lines = lines_of(
        &format!("Roses are red,{LS}violets are blue."),
        800.0,
        Alignment::Left,
    );
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].char_range, 0..15);
    assert_eq!(lines[0].hard_break_chars, 1);
    assert_eq!(lines[0].caret_end(), 14);
    assert_eq!(lines[1].char_range, 15..32);
    assert_eq!(lines[1].hard_break_chars, 0);
    let alone = lines_of("Roses are red,", 800.0, Alignment::Left);
    assert_eq!(
        lines[0].width, alone[0].width,
        "the separator adds no width"
    );
}

/// A right-aligned or centred line ending in a break sits where the same line
/// without it does: the separator's advance used to push it off its place.
#[test]
fn a_line_ending_in_a_break_keeps_its_alignment() {
    for alignment in [Alignment::Right, Alignment::Center] {
        let with = lines_of(&format!("Signed{LS}Anne"), 400.0, alignment);
        let without = lines_of("Signed", 400.0, alignment);
        assert_eq!(
            right_edge(&with[0]),
            right_edge(&without[0]),
            "{alignment:?}"
        );
    }
}

/// Shift+Enter at the end of a paragraph starts an empty line, where the caret
/// goes. Without it the caret stayed at the end of the line it had just left.
#[test]
fn a_break_at_the_end_starts_an_empty_line() {
    let lines = lines_of(&format!("abc{LS}"), 800.0, Alignment::Left);
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].char_range, 0..4);
    assert_eq!(lines[0].caret_end(), 3);
    assert_eq!(lines[1].char_range, 4..4);
    assert!(lines[1].runs.is_empty());

    let two = lines_of(&format!("abc{LS}{LS}"), 800.0, Alignment::Left);
    assert_eq!(two.len(), 3);
    let only = lines_of(&LS.to_string(), 800.0, Alignment::Left);
    assert_eq!(only.len(), 2);
}

/// A line of verse set apart by Shift+Enter in a justified paragraph keeps its
/// words where they are, as a browser and Typst set it. The paragraph's lines
/// that wrap are still justified.
#[test]
fn justification_leaves_a_line_ending_in_a_break_alone() {
    let text =
        format!("a b c{LS}The quick brown fox jumps over the lazy dog near the riverbank at dawn.");
    let justified = lines_of(&text, 200.0, Alignment::Justify);
    let left = lines_of(&text, 200.0, Alignment::Left);
    assert!(justified.len() > 3);
    assert_eq!(
        right_edge(&justified[0]),
        right_edge(&left[0]),
        "verse line"
    );
    assert!(
        right_edge(&justified[1]) > right_edge(&left[1]) + 1.0,
        "a wrapped line is still justified"
    );
}

#[test]
fn geometry_reports_the_separator_as_a_hard_break() {
    let mut ts = make_typesetter();
    let (_, geometry) = ts.layout_paragraph_with_geometry(
        &format!("alpha{LS}beta"),
        &TextFormat::default(),
        600.0,
        None,
    );
    assert_eq!(geometry.lines.len(), 2);
    assert_eq!(
        geometry.lines[0].end,
        LineEnd::HardBreak { chars: 1, bytes: 3 }
    );
    assert_eq!(geometry.lines[1].end, LineEnd::EndOfText);

    // The empty line after a break at the end holds no break of its own.
    let (_, geometry) = ts.layout_paragraph_with_geometry(
        &format!("alpha{LS}"),
        &TextFormat::default(),
        600.0,
        None,
    );
    assert_eq!(geometry.lines.len(), 2);
    assert_eq!(
        geometry.lines[0].end,
        LineEnd::HardBreak { chars: 1, bytes: 3 }
    );
    assert_eq!(geometry.lines[1].end, LineEnd::EndOfText);
}

/// The offset after a break is drawn at the start of the next line whatever
/// the affinity: drawn at the end of the line before, a caret there typed onto
/// the next line.
#[test]
fn the_caret_after_a_break_is_on_the_next_line() {
    let mut ts = make_typesetter();
    ts.set_viewport(800.0, 600.0);
    let text = format!("abc{LS}def");
    ts.layout_blocks(vec![make_block(1, &text)]);
    ts.render();

    let end_of_first = ts.caret_rect_with_affinity(3, CursorAffinity::Downstream);
    let start = ts.caret_rect_with_affinity(0, CursorAffinity::Downstream);
    for affinity in [CursorAffinity::Downstream, CursorAffinity::Upstream] {
        let after = ts.caret_rect_with_affinity(4, affinity);
        assert!(
            after[1] > end_of_first[1] + 1.0,
            "{affinity:?}: on the next line"
        );
        assert_eq!(after[0], start[0], "{affinity:?}: at its start");
    }
    assert_eq!(
        end_of_first[1], start[1],
        "the end of the first line is on it"
    );
}

/// A click past the end of a line ending in a break puts the caret before the
/// break, at the end of that line, not at the start of the next.
#[test]
fn a_click_past_the_end_of_the_line_stops_before_the_break() {
    let mut ts = make_typesetter();
    ts.set_viewport(800.0, 600.0);
    let text = format!("abc{LS}def");
    ts.layout_blocks(vec![make_block(1, &text)]);
    ts.render();
    let first = ts.caret_rect_with_affinity(0, CursorAffinity::Downstream);
    let hit = ts
        .hit_test(700.0, first[1] + first[3] / 2.0)
        .expect("a hit on the first line");
    assert_eq!(hit.position, 3);
    assert_eq!(hit.affinity, CursorAffinity::Downstream);

    // At the start of the next line the click is not on a wrap's upstream
    // side: there is no wrap there.
    let second = ts.caret_rect_with_affinity(4, CursorAffinity::Downstream);
    let hit = ts
        .hit_test(-5.0, second[1] + second[3] / 2.0)
        .expect("a hit on the second line");
    assert_eq!(hit.position, 4);
    assert_eq!(hit.affinity, CursorAffinity::Downstream);
}

/// Home and End stay on the line, and End stops before its break.
#[test]
fn end_stops_before_the_break() {
    let mut ts = make_typesetter();
    ts.set_viewport(800.0, 600.0);
    let text = format!("abc{LS}def{LS}");
    ts.layout_blocks(vec![make_block(1, &text)]);
    ts.render();
    for affinity in [CursorAffinity::Downstream, CursorAffinity::Upstream] {
        assert_eq!(ts.visual_line_range_at(1, affinity), Some((0, 3)));
        assert_eq!(ts.visual_line_range_at(3, affinity), Some((0, 3)));
        assert_eq!(ts.visual_line_range_at(4, affinity), Some((4, 7)));
        assert_eq!(ts.visual_line_range_at(8, affinity), Some((8, 8)));
    }
}
