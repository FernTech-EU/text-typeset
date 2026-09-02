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
    // With no syntax/search highlight on this block the span set is empty. This
    // used to reset the block to the STALE base ("Hello"), losing the edit.
    //
    // Nothing has overlaid this block, so no base was ever captured and there is
    // nothing to revert to: the call reports that it changed nothing.
    assert!(
        !ts.flow.apply_block_paint_spans(1, &[]),
        "a block that was never overlaid has nothing to clear"
    );

    assert_eq!(
        ts.render().glyphs.len(),
        after_edit,
        "empty overlay after an edit must preserve the edited text, not revert \
         to the pre-edit base"
    );
}

/// The same regression on the path that can actually still hit it: a block that
/// WAS overlaid holds a captured base, and an edit must replace it. If the
/// pre-edit copy survived the reshape, the empty overlay below would re-derive
/// the block from text the writer has already changed.
#[test]
fn an_edit_replaces_a_captured_base_rather_than_reverting_to_it() {
    let mut ts = laid_out("Hello");

    // Overlay first, which is what captures the base.
    ts.flow.apply_block_paint_spans(
        1,
        &[PaintSpan {
            char_start: 0,
            char_end: 5,
            foreground_color: Some(RED),
            ..Default::default()
        }],
    );
    assert_eq!(ts.flow.captured_paint_bases(), 1);
    let before = ts.render().glyphs.len();

    ts.relayout_block(&make_block(1, "Hello world"));
    let after_edit = ts.render().glyphs.len();
    assert!(after_edit > before, "the reshape must add glyphs");

    // The overlay is still pending, so the reshape re-captured a base from the
    // FRESH text and re-applied the colours to it. Clearing is therefore a real
    // change, and what it restores is the edited text.
    assert!(
        ts.flow.apply_block_paint_spans(1, &[]),
        "an overlay that is still active has something to clear"
    );
    assert_eq!(
        ts.render().glyphs.len(),
        after_edit,
        "the edited text must survive: the pre-edit base must not come back"
    );
    assert_eq!(
        ts.flow.captured_paint_bases(),
        0,
        "and the capture goes with the overlay it existed for"
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

// ── The overlay must not resurrect a stale block position ────────────────────

/// **A whole-flow recolor reverted every block after an edit to its pre-edit
/// document position.**
///
/// `relayout_block` does two things: reshape the edited block, and shift the
/// document-character `position` of every block after it by the edit's length.
/// It re-captured the *base* — the pristine shaped output the overlay
/// re-derives from — for the edited block only. The later blocks' bases kept
/// the pre-shift position.
///
/// `apply_paint_spans_for` then re-derives every block with
/// `*b = apply_paint_spans(base_b, spans)`, and `apply_paint_spans` opens with
/// `base.clone()` — the whole `BlockLayout`, `position` included. So the shift
/// was silently undone for every block after the edit.
///
/// `hit_test` and `caret_rect` both read `block.position` directly, so from
/// then on a click near a later paragraph's start resolved N characters early
/// and its selection highlights painted N characters off — N being however many
/// characters had been typed. Nothing recovered until a full relayout.
///
/// The overlay only runs when some highlighting feature is live (spell check, a
/// search session, a caret band), which is why this never showed up in a bare
/// editor.
#[test]
fn whole_flow_recolor_keeps_the_shifted_block_positions() {
    use std::collections::HashMap;

    let mut ts = make_typesetter();
    ts.layout_blocks(vec![
        helpers::make_block_at(1, 0, "para one"),
        helpers::make_block_at(2, 9, "para two"),
    ]);
    assert_eq!(ts.flow.block_position(2), Some(9), "as laid out");

    // Type four characters into block 1.
    ts.relayout_block(&helpers::make_block_at(1, 0, "para onetest"));
    assert_eq!(
        ts.flow.block_position(2),
        Some(13),
        "the relayout must shift the following block by the four characters typed"
    );

    // Any whole-document recolor — an empty span map is enough, and is exactly
    // what clearing a search does.
    ts.flow.apply_paint_spans_for(HashMap::new());
    assert_eq!(
        ts.flow.block_position(2),
        Some(13),
        "a recolor changes colours; it must not move the block back to where it \
         started before the edit"
    );
}

/// The same fault seen the way a writer sees it: through the hit-test.
#[test]
fn a_recolor_after_an_edit_leaves_later_blocks_clickable_at_the_right_offset() {
    use std::collections::HashMap;

    let mut ts = make_typesetter();
    ts.layout_blocks(vec![
        helpers::make_block_at(1, 0, "para one"),
        helpers::make_block_at(2, 9, "para two"),
    ]);
    ts.relayout_block(&helpers::make_block_at(1, 0, "para onetest"));

    let probe = |ts: &Typesetter| {
        let r = ts.caret_rect(13);
        ts.hit_test(0.5, r[1] + r[3] * 0.5).map(|h| h.position)
    };
    assert_eq!(probe(&ts), Some(13), "before the recolor");
    ts.flow.apply_paint_spans_for(HashMap::new());
    assert_eq!(
        probe(&ts),
        Some(13),
        "clicking the start of the second paragraph must still land on its first \
         character after a recolor"
    );
}

// ── The paint base is captured on write ──────────────────────────────────────
//
// A block's un-overlaid copy exists so an overlay can be re-derived without
// compounding run splits. It is only ever read for a block that carries an
// overlay, so it is only ever taken for one. Capturing every block at layout
// time instead cost a second deep copy of the whole shaped layout: on a
// book-length manuscript, 17 MB that a handful of blocks would ever have used.

#[test]
fn a_document_with_no_overlay_holds_no_base_copies() {
    let mut ts = laid_out("Nothing here is ever highlighted.");
    let _ = ts.render();
    assert_eq!(
        ts.flow.captured_paint_bases(),
        0,
        "laying out must not copy blocks nothing has asked to recolour"
    );
}

#[test]
fn only_the_overlaid_block_is_copied() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![
        make_block(1, "First paragraph"),
        make_block(2, "Second paragraph"),
        make_block(3, "Third paragraph"),
    ]);
    let _ = ts.render();
    assert_eq!(ts.flow.captured_paint_bases(), 0);

    ts.flow.apply_block_paint_spans(
        2,
        &[PaintSpan {
            char_start: 0,
            char_end: 6,
            foreground_color: Some(RED),
            ..Default::default()
        }],
    );

    assert_eq!(
        ts.flow.captured_paint_bases(),
        1,
        "only the block that was recoloured needs a base to be re-derived from"
    );
}

#[test]
fn clearing_an_overlay_releases_its_base_copy() {
    let mut ts = laid_out("Clear me twice");
    let base_colors = colors(&mut ts);

    ts.flow.apply_block_paint_spans(
        1,
        &[PaintSpan {
            char_start: 0,
            char_end: 5,
            foreground_color: Some(BLUE),
            ..Default::default()
        }],
    );
    assert_eq!(ts.flow.captured_paint_bases(), 1);

    ts.flow.apply_block_paint_spans(1, &[]);
    assert_eq!(
        colors(&mut ts),
        base_colors,
        "clearing must restore the base colours"
    );
    assert_eq!(
        ts.flow.captured_paint_bases(),
        0,
        "and must not go on holding a copy of a block nothing overlays"
    );
}

#[test]
fn the_whole_flow_overlay_copies_only_the_blocks_it_names() {
    use std::collections::HashMap;

    let mut ts = make_typesetter();
    ts.layout_blocks(vec![
        make_block(1, "First paragraph"),
        make_block(2, "Second paragraph"),
        make_block(3, "Third paragraph"),
    ]);
    let _ = ts.render();

    let mut spans: HashMap<usize, Vec<PaintSpan>> = HashMap::new();
    spans.insert(
        3,
        vec![PaintSpan {
            char_start: 0,
            char_end: 5,
            foreground_color: Some(GREEN),
            ..Default::default()
        }],
    );
    ts.flow.apply_paint_spans_for(spans);
    assert_eq!(
        ts.flow.captured_paint_bases(),
        1,
        "a wash that names one block must not copy the other two"
    );

    // A later wash naming nothing clears the overlay and the copy with it.
    ts.flow.apply_paint_spans_for(HashMap::new());
    assert_eq!(ts.flow.captured_paint_bases(), 0);
}

#[test]
fn clearing_a_block_that_was_never_overlaid_reports_nothing_to_repaint() {
    let mut ts = laid_out("Untouched");
    assert!(
        !ts.flow.apply_block_paint_spans(1, &[]),
        "a block already showing its base colours has nothing to change"
    );
    assert!(
        !ts.flow.apply_block_paint_spans(404, &[]),
        "and a block that is not in this layout at all has nothing either"
    );
}
