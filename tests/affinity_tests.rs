//! Tests for [`text_typeset::CursorAffinity`] at soft-wrap boundaries.
//!
//! Background: a long paragraph that wraps across lines K and K+1 has
//! one character position N that sits at BOTH the end of line K and
//! the start of line K+1. `CursorAffinity` disambiguates the two
//! placements (Downstream = end of K, Upstream = start of K+1). At
//! every non-boundary position affinity is a no-op.

mod helpers;
use helpers::{make_block, make_typesetter};
use text_typeset::CursorAffinity;

/// Lay out a single long paragraph at a narrow viewport so it
/// definitely wraps. Returns the typesetter, the wrap-boundary
/// position (= char_range.end of the first line = char_range.start of
/// the second), and the y-coordinates of the two lines' carets.
struct WrappedSetup {
    ts: helpers::Typesetter,
    boundary_pos: usize,
    line_1_y: f32,
    line_2_y: f32,
}

fn setup_wrapped_paragraph() -> WrappedSetup {
    let mut ts = make_typesetter();
    // Narrow viewport so a normal sentence definitely wraps. NotoSans
    // at 16px renders this content over at least 2 visual lines.
    ts.set_viewport(120.0, 600.0);
    let text = "The quick brown fox jumps over the lazy dog.";
    ts.layout_blocks(vec![make_block(1, text)]);
    ts.render();

    // Probe across positions; find the one whose Downstream caret_rect
    // lands on line 1 (smaller y) and whose Upstream lands on line 2
    // (larger y) — that is exactly the wrap boundary by construction.
    // Walk positions 1..text.len() looking for the first pos with a
    // strictly different y between the two affinities.
    let mut boundary: Option<(usize, f32, f32)> = None;
    for pos in 1..text.len() {
        let d = ts.caret_rect_with_affinity(pos, CursorAffinity::Downstream);
        let u = ts.caret_rect_with_affinity(pos, CursorAffinity::Upstream);
        // Two distinct y values → wrap boundary.
        if (u[1] - d[1]).abs() > 1.0 {
            assert!(
                u[1] > d[1],
                "Upstream y ({}) must be below Downstream y ({}) at wrap boundary pos {}",
                u[1],
                d[1],
                pos
            );
            boundary = Some((pos, d[1], u[1]));
            break;
        }
    }
    let (boundary_pos, line_1_y, line_2_y) = boundary
        .expect("at least one wrap boundary should exist; widen test text or narrow viewport");

    WrappedSetup {
        ts,
        boundary_pos,
        line_1_y,
        line_2_y,
    }
}

// ── caret_rect: the core invariant ─────────────────────────────────

#[test]
fn caret_rect_downstream_at_wrap_boundary_lands_on_previous_line() {
    let setup = setup_wrapped_paragraph();
    let rect = setup
        .ts
        .caret_rect_with_affinity(setup.boundary_pos, CursorAffinity::Downstream);
    assert!(
        (rect[1] - setup.line_1_y).abs() < 0.5,
        "Downstream caret at wrap boundary should sit on line 1 (y={}), got y={}",
        setup.line_1_y,
        rect[1]
    );
    // x should be on the RIGHT side of line 1 (past last glyph). A
    // narrow viewport at 120 px means line 1's right edge is near the
    // viewport width; the caret x should be well past zero.
    assert!(
        rect[0] > 20.0,
        "Downstream caret x should be past the line start, got {}",
        rect[0]
    );
}

#[test]
fn caret_rect_upstream_at_wrap_boundary_lands_on_next_line() {
    let setup = setup_wrapped_paragraph();
    let rect = setup
        .ts
        .caret_rect_with_affinity(setup.boundary_pos, CursorAffinity::Upstream);
    assert!(
        (rect[1] - setup.line_2_y).abs() < 0.5,
        "Upstream caret at wrap boundary should sit on line 2 (y={}), got y={}",
        setup.line_2_y,
        rect[1]
    );
    // x should be at the LEFT edge of line 2 (line start).
    assert!(
        rect[0] < 8.0,
        "Upstream caret x at line start should be near 0, got {}",
        rect[0]
    );
}

#[test]
fn caret_rect_default_affinity_is_downstream() {
    // Tests rely on this in many places: the no-affinity-arg legacy
    // helper [`Typesetter::caret_rect`] returns the same rect as
    // explicit Downstream. Codifies the default.
    let setup = setup_wrapped_paragraph();
    let default = setup.ts.caret_rect(setup.boundary_pos);
    let downstream = setup
        .ts
        .caret_rect_with_affinity(setup.boundary_pos, CursorAffinity::Downstream);
    assert_eq!(default, downstream);
}

#[test]
fn caret_rect_affinity_is_noop_at_non_wrap_positions() {
    let setup = setup_wrapped_paragraph();
    // Test a position safely inside the first line — well before the
    // wrap boundary. The two affinities must produce identical rects.
    let probe_pos = setup.boundary_pos.saturating_sub(2);
    assert!(probe_pos > 0, "boundary must be at least 2 from start");
    let d = setup
        .ts
        .caret_rect_with_affinity(probe_pos, CursorAffinity::Downstream);
    let u = setup
        .ts
        .caret_rect_with_affinity(probe_pos, CursorAffinity::Upstream);
    assert_eq!(
        d, u,
        "affinity must be a no-op at non-boundary positions; got down={d:?}, up={u:?}"
    );
}

#[test]
fn caret_rect_affinity_unaffected_for_single_line_block() {
    // Wide viewport — single line, no wraps. Both affinities behave
    // identically at every position. This is the contract we promise
    // single-line widgets (TextInput) — they can pass Downstream
    // without thinking.
    let mut ts = make_typesetter();
    ts.set_viewport(800.0, 200.0);
    ts.layout_blocks(vec![make_block(1, "Hello, world!")]);
    ts.render();
    for pos in 0..=13 {
        let d = ts.caret_rect_with_affinity(pos, CursorAffinity::Downstream);
        let u = ts.caret_rect_with_affinity(pos, CursorAffinity::Upstream);
        assert_eq!(
            d, u,
            "single-line block: affinity must not affect caret rect at pos {pos}, got d={d:?} u={u:?}"
        );
    }
}

// ── hit_test: produces Upstream when the click lands on line K+1's start ──

#[test]
fn hit_test_at_start_of_wrapped_line_returns_upstream() {
    let setup = setup_wrapped_paragraph();
    // Click on the FAR LEFT of line 2. The hit should return position
    // == boundary_pos AND affinity == Upstream.
    let hit = setup
        .ts
        .hit_test(0.5, setup.line_2_y + 2.0)
        .expect("hit-test in second wrapped line should return Some");
    assert_eq!(
        hit.position, setup.boundary_pos,
        "click at start of line 2 should resolve to the wrap-boundary position"
    );
    assert_eq!(
        hit.affinity,
        CursorAffinity::Upstream,
        "click at line-2 start should produce Upstream affinity (so caret renders at line 2's start, not line 1's end)"
    );
}

#[test]
fn hit_test_at_end_of_wrapped_line_returns_downstream() {
    let setup = setup_wrapped_paragraph();
    // Click far to the right on LINE 1. Should land at the wrap
    // boundary with Downstream affinity (we're hovering line 1's end).
    let hit = setup
        .ts
        .hit_test(500.0, setup.line_1_y + 2.0)
        .expect("hit-test past end of line 1 should return Some");
    // The result might be exactly boundary_pos (if line 1's char_range.end
    // == boundary_pos, which is what defines the wrap boundary).
    assert_eq!(hit.position, setup.boundary_pos);
    assert_eq!(
        hit.affinity,
        CursorAffinity::Downstream,
        "click at line-1 end should produce Downstream affinity"
    );
}

#[test]
fn hit_test_at_non_boundary_returns_downstream() {
    let setup = setup_wrapped_paragraph();
    // Click in the middle of line 1, well before the wrap point.
    let hit = setup
        .ts
        .hit_test(40.0, setup.line_1_y + 2.0)
        .expect("hit-test middle of line 1 should return Some");
    assert!(hit.position < setup.boundary_pos);
    assert_eq!(
        hit.affinity,
        CursorAffinity::Downstream,
        "non-boundary positions always report Downstream (the default)"
    );
}

#[test]
fn hit_test_at_block_first_line_start_is_downstream_not_upstream() {
    // Pre-affinity behavior: the first wrap line of a block is NOT a
    // wrap-continuation — its char_range.start has no preceding line
    // in the same block that ends there. So clicking at the very
    // start (position 0 of block, line 0) must remain Downstream.
    let setup = setup_wrapped_paragraph();
    let hit = setup
        .ts
        .hit_test(0.5, setup.line_1_y + 2.0)
        .expect("hit-test at line-1 start should return Some");
    assert_eq!(hit.position, 0);
    assert_eq!(
        hit.affinity,
        CursorAffinity::Downstream,
        "first line's start is not a wrap continuation; affinity must stay Downstream"
    );
}

// ── caret_rect: the two affinities really render at different lines ──

#[test]
fn caret_rect_y_differs_between_affinities_at_wrap_boundary() {
    let setup = setup_wrapped_paragraph();
    let d_y = setup
        .ts
        .caret_rect_with_affinity(setup.boundary_pos, CursorAffinity::Downstream)[1];
    let u_y = setup
        .ts
        .caret_rect_with_affinity(setup.boundary_pos, CursorAffinity::Upstream)[1];
    let line_height = setup
        .ts
        .caret_rect_with_affinity(setup.boundary_pos, CursorAffinity::Downstream)[3];
    let dy = u_y - d_y;
    // Upstream caret should sit a full line below Downstream caret —
    // not zero (would mean same line), not multiple lines.
    assert!(
        dy >= line_height * 0.8 && dy <= line_height * 1.5,
        "Upstream-Downstream y delta ({}) should be approximately one line height ({})",
        dy,
        line_height
    );
}
