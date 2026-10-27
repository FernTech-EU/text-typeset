//! A line break inside a paragraph: U+2028 LINE SEPARATOR, the character a
//! word processor inserts for Shift+Enter.
//!
//! It ends a line, as `\n` always has here, and it is laid out as nothing:
//! no glyph and no width. A break at the paragraph's end starts an empty
//! line, a line it ends is not stretched by justification, and the offset
//! after it is the next line's start alone: for the caret, for a click past
//! the end of the line before, and for End.

mod helpers;
use helpers::{NOTO_ARABIC, NOTO_SANS, Typesetter, make_block, make_typesetter};

use text_typeset::CursorAffinity;
use text_typeset::LineEnd;
use text_typeset::TextFormat;
use text_typeset::font::resolve::resolve_font;
use text_typeset::layout::block::layout_block;
use text_typeset::layout::line::LayoutLine;
use text_typeset::layout::paragraph::{Alignment, RunOrder, break_into_lines};
use text_typeset::shaping::shaper::{
    TextDirection, font_metrics_px, shape_text, shape_text_with_buffer,
};

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

fn left_edge(line: &LayoutLine) -> f32 {
    line.runs
        .iter()
        .map(|run| run.x)
        .fold(f32::INFINITY, f32::min)
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
        // The left edge: a right-aligned line's right edge is the width's whatever the
        // line holds.
        assert_eq!(left_edge(&with[0]), left_edge(&without[0]), "{alignment:?}");
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

// ── What the reviews found ──────────────────────────────────────

const KATABA: &str = "\u{0643}\u{062A}\u{0628}";
const SALAAM: &str = "\u{0633}\u{0644}\u{0627}\u{0645}";

/// Arabic first, Latin beside it, so fallback covers both.
fn bilingual() -> Typesetter {
    let mut ts = Typesetter::new();
    let arabic = ts.register_font(NOTO_ARABIC);
    ts.register_font(NOTO_SANS);
    ts.set_default_font(arabic, 16.0);
    ts.set_viewport(800.0, 600.0);
    ts
}

/// Latin first, Arabic as its fallback.
fn bilingual_latin_first() -> Typesetter {
    let mut ts = Typesetter::new();
    let latin = ts.register_font(NOTO_SANS);
    ts.register_font(NOTO_ARABIC);
    ts.set_default_font(latin, 16.0);
    ts
}

/// A line separator stays inside the right-to-left run it sits in (it is bidi class
/// WS), and the run's glyphs are stored in visual order: walked that way, the break
/// mapped to the run's first glyph and was never taken. Shift+Enter did nothing in
/// Arabic or Hebrew.
#[test]
fn a_line_separator_breaks_a_right_to_left_paragraph() {
    let ts = bilingual();
    let mut params = make_block(0, &format!("{KATABA}{LS}{SALAAM}"));
    params.base_direction = TextDirection::RightToLeft;
    let layout = layout_block(ts.font_registry(), &params, 600.0, 1.0, 1.0);
    let ranges: Vec<_> = layout.lines.iter().map(|l| l.char_range.clone()).collect();
    assert_eq!(ranges, [0..4, 4..8]);
    assert_eq!(layout.lines[0].hard_break_chars, 1);
}

/// A wrapped right-to-left paragraph sets its words down in reading order. Its first
/// line held its last words, and its last line claimed the whole paragraph.
#[test]
fn a_wrapped_right_to_left_paragraph_keeps_its_reading_order() {
    let ts = bilingual();
    let text = vec![SALAAM; 30].join(" ");
    let mut params = make_block(0, &text);
    params.base_direction = TextDirection::RightToLeft;
    let layout = layout_block(ts.font_registry(), &params, 140.0, 1.0, 1.0);
    assert!(layout.lines.len() > 2);
    assert_eq!(layout.lines[0].char_range.start, 0);
    for pair in layout.lines.windows(2) {
        assert_eq!(pair[0].char_range.end, pair[1].char_range.start);
    }
    assert_eq!(
        layout.lines.last().map(|l| l.char_range.end),
        Some(text.chars().count())
    );
}

/// A line ending in a break before a right-to-left line: the next line's first glyph
/// carries its last character, which the line before took its end from.
#[test]
fn a_break_before_a_right_to_left_line_ends_its_line() {
    let ts = bilingual();
    let mut params = make_block(0, &format!("abc{LS}{KATABA}"));
    params.base_direction = TextDirection::LeftToRight;
    let layout = layout_block(ts.font_registry(), &params, 600.0, 1.0, 1.0);
    assert_eq!(layout.lines[0].char_range, 0..4);
    assert_eq!(layout.lines[0].hard_break_chars, 1);
}

/// A click left of a right-to-left line ending in a break: the break is that line's
/// leftmost glyph, with no width, and the click took its far edge, the next line's start.
#[test]
fn a_click_left_of_a_right_to_left_line_stays_on_it() {
    let mut ts = bilingual();
    let mut params = make_block(1, &format!("{KATABA}{LS}{SALAAM}"));
    params.base_direction = TextDirection::RightToLeft;
    ts.layout_blocks(vec![params]);
    ts.render();
    let first = ts.caret_rect_with_affinity(0, CursorAffinity::Downstream);
    let hit = ts
        .hit_test(5.0, first[1] + first[3] / 2.0)
        .expect("a hit on the first line");
    assert_eq!(hit.position, 3);
}

/// The offset inside a `\r\n` belongs to the line the break ends, and has a caret
/// there: it belonged to no line, and its caret fell back to another block's.
#[test]
fn the_offset_inside_a_crlf_has_a_caret_on_its_line() {
    let mut ts = make_typesetter();
    ts.set_viewport(800.0, 600.0);
    ts.layout_blocks(vec![make_block(1, "abc\r\ndef")]);
    ts.render();
    let end = ts.caret_rect_with_affinity(3, CursorAffinity::Downstream);
    let inside = ts.caret_rect_with_affinity(4, CursorAffinity::Downstream);
    let next = ts.caret_rect_with_affinity(5, CursorAffinity::Downstream);
    assert_eq!(inside[1], end[1]);
    assert!(next[1] > end[1] + 1.0);
    assert_eq!(
        ts.visual_line_range_at(4, CursorAffinity::Downstream),
        Some((0, 3))
    );
}

/// Letter spacing leaves a break at no width: spaced, it pushed a right-aligned line
/// ending in one off its place.
#[test]
fn letter_spacing_gives_a_break_no_width() {
    let ts = make_typesetter();
    let width = |text: &str| {
        let mut params = make_block(0, text);
        for fragment in &mut params.fragments {
            fragment.letter_spacing = 5.0;
        }
        layout_block(ts.font_registry(), &params, 600.0, 1.0, 1.0).lines[0].width
    };
    assert_eq!(width(&format!("Signed{LS}Anne")), width("Signed"));
}

/// A break beside a character the primary font lacks leaves that character its own
/// fallback font: in one span with the break, the span took the break's font.
#[test]
fn a_break_does_not_take_its_neighbours_fallback_font() {
    let ts = bilingual_latin_first();
    let resolved =
        resolve_font(ts.font_registry(), None, None, None, None, None, 1.0, 1.0).unwrap();
    let text = format!("a{LS}{SALAAM}");
    let run = shape_text(ts.font_registry(), &resolved, &text, 0).unwrap();
    let arabic: Vec<_> = run.glyphs.iter().filter(|g| g.cluster >= 4).collect();
    assert!(!arabic.is_empty());
    let ids: Vec<u16> = run.glyphs.iter().map(|g| g.glyph_id).collect();
    assert!(arabic.iter().all(|g| g.glyph_id != 0), "{ids:?}");
}

/// Every shaping entry point lays a break out as nothing.
#[test]
fn shaping_with_a_recycled_buffer_blanks_a_break_too() {
    let ts = make_typesetter();
    let resolved =
        resolve_font(ts.font_registry(), None, None, None, None, None, 1.0, 1.0).unwrap();
    let (run, _) = shape_text_with_buffer(
        ts.font_registry(),
        &resolved,
        &format!("a{LS}b"),
        0,
        harfrust::Buffer::new(),
        &[],
    )
    .unwrap();
    let separator = run.glyphs.iter().find(|g| g.cluster == 1).unwrap();
    assert_eq!((separator.glyph_id, separator.x_advance), (0, 0.0));
}
