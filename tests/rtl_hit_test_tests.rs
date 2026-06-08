//! Hit-testing and caret placement for right-to-left text.
//!
//! Glyphs are stored in visual (left-to-right) order, so an RTL run's
//! clusters descend across the array. Before the direction-aware fix,
//! `find_position_in_line` / `x_for_offset` scanned as if every run were
//! LTR, so caret placement and click mapping inside Hebrew/Arabic words
//! were wrong. A pure-RTL block exercises both functions end-to-end:
//! `layout_blocks` resolves the run direction to RTL (harfrust guesses it
//! from the Hebrew script), and `caret_rect` / `hit_test` consume it.

mod helpers;
use helpers::{NOTO_HEBREW, Typesetter, make_block};

/// "shalom" — 4 Hebrew letters, logical order ש(0) ל(1) ו(2) ם(3).
const SHALOM: &str = "\u{05E9}\u{05DC}\u{05D5}\u{05DD}";

fn hebrew_typesetter() -> Typesetter {
    let mut ts = Typesetter::new(); // hermetic: no system fonts
    let face = ts.register_font(NOTO_HEBREW);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(800.0, 600.0);
    ts
}

#[test]
fn rtl_caret_moves_leftward_as_offset_grows() {
    let mut ts = hebrew_typesetter();
    ts.layout_blocks(vec![make_block(1, SHALOM)]);
    ts.render();

    let n = SHALOM.chars().count();
    // Logical offset 0 is the start of reading → rightmost on screen;
    // the end is leftmost. (Old LTR-only code put offset 0 at the left.)
    let x0 = ts.caret_rect(0)[0];
    let xn = ts.caret_rect(n)[0];
    assert!(
        x0 > xn,
        "RTL: caret for offset 0 ({x0:.1}) should be right of the end caret ({xn:.1})"
    );

    let mut prev = f32::INFINITY;
    for i in 0..=n {
        let x = ts.caret_rect(i)[0];
        assert!(
            x < prev + 0.01,
            "RTL caret should move leftward as the logical offset grows: \
             offset {i} x={x:.1} not <= prev {prev:.1}"
        );
        prev = x;
    }
}

#[test]
fn rtl_hit_test_round_trips_to_same_visual_position() {
    let mut ts = hebrew_typesetter();
    ts.layout_blocks(vec![make_block(1, SHALOM)]);
    ts.render();

    let n = SHALOM.chars().count();
    let r0 = ts.caret_rect(0);
    let baseline_y = r0[1] + r0[3] / 2.0;

    // For every caret position, clicking at its x must map back to a
    // logical offset whose own caret sits at the same x. This is robust
    // to which side of a shared boundary the click resolves to.
    for i in 0..=n {
        let x = ts.caret_rect(i)[0];
        let hit = ts
            .hit_test(x, baseline_y)
            .unwrap_or_else(|| panic!("hit_test returned None at offset {i} (x={x:.1})"));
        let x_back = ts.caret_rect(hit.position)[0];
        assert!(
            (x - x_back).abs() < 1.0,
            "RTL round-trip mismatch at offset {i}: caret x={x:.1}, \
             hit gave offset {} with caret x={x_back:.1}",
            hit.position
        );
    }
}

#[test]
fn rtl_click_inside_word_lands_in_logical_range() {
    let mut ts = hebrew_typesetter();
    ts.layout_blocks(vec![make_block(1, SHALOM)]);
    ts.render();

    let n = SHALOM.chars().count();
    let r0 = ts.caret_rect(0);
    let baseline_y = r0[1] + r0[3] / 2.0;

    // Click the visual middle of the word; the resolved logical offset
    // must be a valid interior position (not clamped to an end).
    let left = ts.caret_rect(n)[0];
    let right = ts.caret_rect(0)[0];
    let mid_x = (left + right) / 2.0;
    let hit = ts.hit_test(mid_x, baseline_y).expect("hit in word");
    assert!(
        hit.position <= n,
        "resolved offset {} out of range 0..={n}",
        hit.position
    );
    assert!(
        hit.position > 0 && hit.position < n,
        "clicking the visual middle of an RTL word should land on an \
         interior offset, got {}",
        hit.position
    );
}
