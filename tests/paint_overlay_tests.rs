//! Paint-only overlay: recolor a laid-out block WITHOUT reshaping or
//! reflowing. These assert the observable contract — glyph screen positions
//! are byte-identical before/after a recolor (proving no reflow), only colors
//! and decorations change.

mod helpers;

use helpers::{Typesetter, make_block, make_typesetter};
use text_typeset::{DecorationKind, PaintSpan};

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];

fn positions(ts: &mut Typesetter) -> Vec<[f32; 4]> {
    ts.render().glyphs.iter().map(|g| g.screen).collect()
}
fn colors(ts: &mut Typesetter) -> Vec<[f32; 4]> {
    ts.render().glyphs.iter().map(|g| g.color).collect()
}
fn is(c: [f32; 4], target: [f32; 4]) -> bool {
    c.iter().zip(target.iter()).all(|(a, b)| (a - b).abs() < 0.02)
}

fn laid_out(text: &str) -> Typesetter {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, text)]);
    ts
}

#[test]
fn full_block_recolor_preserves_positions() {
    let mut ts = laid_out("Hello world");
    let base = positions(&mut ts);
    assert!(!base.is_empty());

    let applied = ts.flow.apply_block_paint_spans(
        1,
        &[PaintSpan {
            char_start: 0,
            char_end: 11,
            foreground_color: Some(RED),
            ..Default::default()
        }],
    );
    assert!(applied, "top-level block must be found");

    // Glyph positions are byte-identical: no reshape, no reflow.
    assert_eq!(positions(&mut ts), base);
    // Every glyph is now red.
    assert!(colors(&mut ts).iter().all(|c| is(*c, RED)));
}

#[test]
fn partial_span_splits_run_but_preserves_positions() {
    let mut ts = laid_out("Hello world");
    let base = positions(&mut ts);

    // Recolor only chars [2, 7) — splits the single run into 3 segments.
    ts.flow.apply_block_paint_spans(
        1,
        &[PaintSpan {
            char_start: 2,
            char_end: 7,
            foreground_color: Some(RED),
            ..Default::default()
        }],
    );

    // Splitting a run must not move any glyph.
    assert_eq!(positions(&mut ts), base);
    let cs = colors(&mut ts);
    assert!(cs.iter().any(|c| is(*c, RED)), "the span range is red");
    assert!(cs.iter().any(|c| !is(*c, RED)), "outside the span is not red");
}

#[test]
fn background_span_emits_text_background_decoration() {
    let mut ts = laid_out("Highlight me");
    let base = positions(&mut ts);

    ts.flow.apply_block_paint_spans(
        1,
        &[PaintSpan {
            char_start: 0,
            char_end: 12,
            background_color: Some([1.0, 1.0, 0.0, 0.5]),
            ..Default::default()
        }],
    );

    let frame = ts.render();
    let bg: Vec<_> = frame
        .decorations
        .iter()
        .filter(|d| d.kind == DecorationKind::TextBackground)
        .collect();
    assert!(!bg.is_empty(), "background overlay must emit a TextBackground rect");
    assert!(bg.iter().all(|d| d.rect[2] > 0.0), "rects have positive width");
    drop(frame);
    // And it did not reflow.
    assert_eq!(positions(&mut ts), base);
}

#[test]
fn clearing_overlay_restores_base_colors() {
    let mut ts = laid_out("Clear me");
    let base_colors = colors(&mut ts);

    ts.flow.apply_block_paint_spans(
        1,
        &[PaintSpan {
            char_start: 0,
            char_end: 8,
            foreground_color: Some(GREEN),
            ..Default::default()
        }],
    );
    let _ = ts.render();

    // Empty spans clears the overlay back to base.
    ts.flow.apply_block_paint_spans(1, &[]);
    assert_eq!(colors(&mut ts), base_colors);
}

#[test]
fn reapplying_overlay_does_not_compound() {
    let mut ts = laid_out("ABCDEFGH");
    let base = positions(&mut ts);

    // S1: red on [0,4)
    ts.flow.apply_block_paint_spans(
        1,
        &[PaintSpan {
            char_start: 0,
            char_end: 4,
            foreground_color: Some(RED),
            ..Default::default()
        }],
    );
    let _ = ts.render();

    // S2: blue on [4,8) — re-derived from base, so S1's red must be gone.
    ts.flow.apply_block_paint_spans(
        1,
        &[PaintSpan {
            char_start: 4,
            char_end: 8,
            foreground_color: Some(BLUE),
            ..Default::default()
        }],
    );

    assert_eq!(positions(&mut ts), base, "still no reflow after re-apply");
    let cs = colors(&mut ts);
    assert!(cs.iter().any(|c| is(*c, BLUE)), "S2 applied");
    assert!(
        !cs.iter().any(|c| is(*c, RED)),
        "S1 must not persist (re-derived from base, no compounding)"
    );
}

#[test]
fn overlapping_spans_last_wins() {
    let mut ts = laid_out("ABCD");
    ts.flow.apply_block_paint_spans(
        1,
        &[
            PaintSpan {
                char_start: 0,
                char_end: 4,
                foreground_color: Some(RED),
                ..Default::default()
            },
            PaintSpan {
                char_start: 2,
                char_end: 4,
                foreground_color: Some(BLUE),
                ..Default::default()
            },
        ],
    );
    let cs = colors(&mut ts);
    assert!(cs.iter().any(|c| is(*c, BLUE)), "overlap resolves to last span (blue)");
    assert!(cs.iter().any(|c| is(*c, RED)), "non-overlapping prefix stays red");
}
