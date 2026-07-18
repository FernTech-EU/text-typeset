//! `DocumentFlow::set_render_window` — the paint-only cull override used by an
//! editor laid out at full document height inside an outer ScrollArea.
//!
//! The window must (a) cull to the requested content-space band, and (b) leave
//! glyph positions untouched — culling only, never a shift. The second property
//! is the correctness lynchpin: positioning and hit-testing still key off
//! `scroll_offset` (which stays 0 in bastard mode), so a windowed glyph lands at
//! exactly the same screen coordinate it would without the window.

mod helpers;
use helpers::{make_block, make_typesetter};

/// A tall document: 40 one-line blocks. Returns (typesetter, line_height).
fn tall_doc() -> (helpers::Typesetter, f32) {
    let mut ts = make_typesetter();
    let blocks: Vec<_> = (1..=40)
        .map(|i| make_block(i, "The quick brown fox"))
        .collect();
    ts.layout_blocks(blocks);
    let line_h = ts.content_height() / 40.0;
    // A viewport tall enough to hold the whole document (bastard mode).
    ts.set_viewport(400.0, ts.content_height() + line_h);
    (ts, line_h)
}

/// Every (x, y) in `subset` must appear in `superset` — i.e. windowing removed
/// glyphs but never moved the ones it kept.
fn positions_are_a_subset(subset: &[[f32; 4]], superset: &[[f32; 4]]) {
    for q in subset {
        let found = superset
            .iter()
            .any(|s| (s[0] - q[0]).abs() < 0.01 && (s[1] - q[1]).abs() < 0.01);
        assert!(
            found,
            "windowed glyph at ({:.2}, {:.2}) is missing (or moved) in the full frame — \
             the window must cull, never shift",
            q[0], q[1]
        );
    }
}

#[test]
fn a_window_culls_to_its_band_without_moving_glyphs() {
    let (mut ts, line_h) = tall_doc();

    // Baseline: no window → the whole document renders.
    let full: Vec<[f32; 4]> = ts.render().glyphs.iter().map(|g| g.screen).collect();
    assert!(!full.is_empty());

    // A window over the middle of the document (rows ~10..15).
    let top = 10.0 * line_h;
    let height = 5.0 * line_h;
    ts.set_render_window(Some((top, height)));
    let windowed: Vec<[f32; 4]> = ts.render().glyphs.iter().map(|g| g.screen).collect();

    assert!(
        windowed.len() < full.len(),
        "a 5-line window over a 40-line document must cull most glyphs (got {} of {})",
        windowed.len(),
        full.len()
    );
    assert!(!windowed.is_empty(), "the visible band must still render");

    // Positioning is unchanged: every kept glyph is at its full-frame position.
    positions_are_a_subset(&windowed, &full);

    // And the kept glyphs really are the middle band, not the top: their y sits
    // near the window, well below the first rows.
    let min_y = windowed.iter().map(|g| g[1]).fold(f32::INFINITY, f32::min);
    assert!(
        min_y > top - 2.0 * line_h,
        "windowed glyphs (min y = {min_y:.1}) should be near the band top {top:.1}, not the \
         document top"
    );
}

#[test]
fn none_restores_the_whole_document() {
    let (mut ts, line_h) = tall_doc();
    let full_count = ts.render().glyphs.len();

    ts.set_render_window(Some((0.0, 3.0 * line_h)));
    let windowed_count = ts.render().glyphs.len();
    assert!(windowed_count < full_count);

    ts.set_render_window(None);
    let restored_count = ts.render().glyphs.len();
    assert_eq!(
        restored_count, full_count,
        "clearing the window must render the whole document again"
    );
}

#[test]
fn a_top_window_and_a_middle_window_render_different_bands() {
    let (mut ts, line_h) = tall_doc();

    ts.set_render_window(Some((0.0, 4.0 * line_h)));
    let top_band: Vec<f32> = ts.render().glyphs.iter().map(|g| g.screen[1]).collect();

    ts.set_render_window(Some((20.0 * line_h, 4.0 * line_h)));
    let mid_band: Vec<f32> = ts.render().glyphs.iter().map(|g| g.screen[1]).collect();

    let top_max = top_band.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let mid_min = mid_band.iter().cloned().fold(f32::INFINITY, f32::min);
    assert!(
        mid_min > top_max,
        "the middle band (min y {mid_min:.1}) must sit entirely below the top band \
         (max y {top_max:.1}) — the two windows select disjoint content"
    );
}
