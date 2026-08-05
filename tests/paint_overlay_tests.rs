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
    c.iter()
        .zip(target.iter())
        .all(|(a, b)| (a - b).abs() < 0.02)
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
    assert!(
        cs.iter().any(|c| !is(*c, RED)),
        "outside the span is not red"
    );
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
    assert!(
        !bg.is_empty(),
        "background overlay must emit a TextBackground rect"
    );
    assert!(
        bg.iter().all(|d| d.rect[2] > 0.0),
        "rects have positive width"
    );
    let _ = frame;
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

/// Regression: an incremental block edit followed by a paint-overlay pass must
/// render the EDITED text, not the pre-edit base. The editor (highlights on)
/// reshapes one block then calls `apply_block_paint_spans`; if the base block
/// is not refreshed by the relayout, the overlay re-derives the block from the
/// stale base and silently clobbers the just-typed characters. A highlights-off
/// view skips the overlay and was unaffected — which is why the bug presented
/// as "the editor pane drops edits while the read-only preview shows them".
#[test]
fn edit_then_empty_overlay_renders_new_text_not_stale_base() {
    let mut ts = laid_out("Hello");
    let before = ts.render().glyphs.len();
    assert!(before > 0);

    // Simulate a single-block edit: reshape block 1 with longer text.
    ts.relayout_block(&make_block(1, "Hello world"));
    let after_edit = ts.render().glyphs.len();
    assert!(
        after_edit > before,
        "the reshape itself must add glyphs for the new text"
    );

    // The editor's highlights-on path then re-applies the block's paint spans.
    // With no syntax/search highlight on this block the span set is empty —
    // this used to reset the block to the STALE base ("Hello"), losing the edit.
    let applied = ts.flow.apply_block_paint_spans(1, &[]);
    assert!(applied, "top-level block must be found for overlay");

    assert_eq!(
        ts.render().glyphs.len(),
        after_edit,
        "empty overlay after an edit must preserve the edited text, not revert \
         to the pre-edit base"
    );
}

/// Same regression, but with a NON-empty overlay (the char landed inside a
/// highlighted run). The recolor must apply on top of the FRESH geometry.
#[test]
fn edit_then_nonempty_overlay_keeps_edited_glyph_count() {
    let mut ts = laid_out("foo");
    let _ = ts.render();

    ts.relayout_block(&make_block(1, "foo bar baz"));
    let after_edit = ts.render().glyphs.len();

    ts.flow.apply_block_paint_spans(
        1,
        &[PaintSpan {
            char_start: 0,
            char_end: 3,
            foreground_color: Some(RED),
            ..Default::default()
        }],
    );

    assert_eq!(
        ts.render().glyphs.len(),
        after_edit,
        "recolor must overlay onto the edited geometry, not the stale base"
    );
    assert!(
        colors(&mut ts).iter().any(|c| is(*c, RED)),
        "the highlight still applies"
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
    assert!(
        cs.iter().any(|c| is(*c, BLUE)),
        "overlap resolves to last span (blue)"
    );
    assert!(
        cs.iter().any(|c| is(*c, RED)),
        "non-overlapping prefix stays red"
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Cross-run tiling
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// A block of two differently-formatted fragments, so one background span has to cover two
/// separate `PositionedRun`s.
fn bold_then_regular(text_a: &str, text_b: &str) -> text_typeset::layout::block::BlockLayoutParams {
    use text_typeset::layout::block::FragmentParams;
    use text_typeset::layout::paragraph::Alignment;
    use text_typeset::{UnderlineStyle, VerticalAlignment};
    let frag = |text: &str, offset: usize, bold: Option<bool>| FragmentParams {
        text: text.to_string(),
        offset,
        length: text.len(),
        font_family: None,
        font_weight: None,
        font_bold: bold,
        font_italic: None,
        font_point_size: None,
        underline_style: UnderlineStyle::None,
        overline: false,
        strikeout: false,
        is_link: false,
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
        footnote_marker: None,
        features: Vec::new(),
    };
    let whole = format!("{text_a}{text_b}");
    let mut b = make_block(1, &whole);
    b.fragments = vec![
        frag(text_a, 0, Some(true)),
        frag(text_b, text_a.len(), None),
    ];
    b.alignment = Alignment::Left;
    b
}

/// **The band must not show seams.** A highlight spanning a formatting boundary emits one
/// `TextBackground` rect per run, so a sentence that crosses a bold word is drawn as several
/// abutting rectangles. If they left a gap — or overlapped — the band would show a hairline
/// down the middle of a word.
///
/// This is the invariant that makes coalescing the rects unnecessary. Should it ever fail, that
/// is the signal to merge same-colour rects per line in `render/decoration.rs`, and not before.
#[test]
fn a_background_span_tiles_seamlessly_across_a_run_boundary() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![bold_then_regular("Hello ", "world")]);
    let base = positions(&mut ts);

    ts.flow.apply_block_paint_spans(
        1,
        &[PaintSpan {
            char_start: 0,
            char_end: 11,
            background_color: Some(GREEN),
            ..Default::default()
        }],
    );

    let frame = ts.render();
    let mut bands: Vec<[f32; 4]> = frame
        .decorations
        .iter()
        .filter(|d| d.kind == DecorationKind::TextBackground)
        .map(|d| d.rect)
        .collect();
    assert!(
        bands.len() >= 2,
        "the two fragments must produce separate rects for this test to mean anything; got {}",
        bands.len()
    );

    bands.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap());
    for pair in bands.windows(2) {
        let (left, right) = (pair[0], pair[1]);
        // Same visual line: identical top and height, or the band would step.
        assert!(
            (left[1] - right[1]).abs() < 1e-3 && (left[3] - right[3]).abs() < 1e-3,
            "rects on one line must share their vertical band: {left:?} vs {right:?}"
        );
        // No gap, no overlap.
        let seam = left[0] + left[2] - right[0];
        assert!(
            seam.abs() < 1e-3,
            "adjacent background rects must tile exactly; seam of {seam} between {left:?} and {right:?}"
        );
    }

    // …and covering two runs still reshapes nothing.
    assert_eq!(positions(&mut ts), base);
}
