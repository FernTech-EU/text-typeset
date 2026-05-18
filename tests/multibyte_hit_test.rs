//! Regression tests for hit-testing and caret geometry on blocks
//! containing multi-byte characters (em-dash, curly quotes, accented
//! characters).
//!
//! Pre-fix bug: `FragmentContent::*.offset` is a **character** offset
//! within the block (documented as such), but the
//! text-document → text-typeset bridge passed it straight to
//! `FragmentParams.offset`, which is treated downstream as a **byte**
//! offset. For blocks whose first fragment contained any multi-byte
//! character (em-dash `—` = 3 bytes, é = 2 bytes, etc.), every
//! subsequent fragment's glyph clusters lifted into block-text byte
//! space landed at the wrong byte position. After the
//! `byte_offset_to_char_offset` round-trip the cluster ended up at a
//! char index off by `(bytes − chars)` of the multi-byte chars
//! upstream — visible to the user as Ctrl+B / clicks landing
//! a character past where they pointed around em-dashes.
//!
//! The bug only manifests when the block has multiple fragments
//! (e.g. a bold span next to plain prose). A single-fragment block
//! happens to land at byte_offset == char_offset == 0, masking the
//! confusion.

mod helpers;
use helpers::{NOTO_SANS, Typesetter, make_typesetter};

use text_document::TextDocument;

/// Build a typesetter laid out on `md` at a generous viewport.
fn layout_md(md: &str) -> Typesetter {
    let doc = TextDocument::new();
    let op = doc.set_markdown(md).expect("markdown parses");
    op.wait().expect("markdown op completes");
    let flow = doc.snapshot_flow();
    let mut ts = make_typesetter();
    ts.set_viewport(800.0, 600.0);
    ts.layout_full(&flow);
    ts
}

/// Walk every char position in `text` and assert that the typesetter
/// caret moves strictly left-to-right (no two distinct positions share
/// the same x). This catches the bridge unit-confusion symptom where
/// glyph clusters collapsed multiple chars to the same x past an
/// em-dash.
fn assert_caret_x_strictly_increases(ts: &Typesetter, text: &str, label: &str) {
    let mut prev_x = f32::NEG_INFINITY;
    let char_count = text.chars().count();
    for char_pos in 0..=char_count {
        let rect = ts.caret_rect(char_pos);
        let x = rect[0];
        assert!(
            x > prev_x - 0.01,
            "[{label}] caret x went BACKWARDS at char {char_pos}: \
             prev_x={prev_x:.2}, x={x:.2}; em-dash bridge unit confusion?"
        );
        prev_x = x;
    }
}

/// Walk every char position and round-trip caret_rect → hit_test.
/// Probe just past the caret's x (inside glyph N, before its
/// midpoint) so hit_test should report position N. `find_position_in_line`
/// returns the cluster of the first glyph whose midpoint sits past
/// the probe x, so probing at `rect[0] + 0.5` lands inside glyph N
/// (a tiny ε past its left edge) and the loop returns N.
fn assert_hit_test_round_trips(ts: &Typesetter, text: &str, label: &str) {
    let baseline_y = ts.caret_rect(0)[1] + ts.caret_rect(0)[3] / 2.0;
    let char_count = text.chars().count();
    // Skip the last position — past the last glyph there's no glyph
    // to land in, hit_test reports PastLineEnd at char_count regardless.
    for char_pos in 0..char_count {
        let here = ts.caret_rect(char_pos);
        let probe_x = here[0] + 0.5;
        let hit = ts
            .hit_test(probe_x, baseline_y)
            .unwrap_or_else(|| panic!("[{label}] hit_test returned None at char {char_pos}"));
        assert_eq!(
            hit.position, char_pos,
            "[{label}] hit_test probe inside glyph {char_pos} returned {} \
             — em-dash byte/char confusion in the bridge?",
            hit.position
        );
    }
}

#[test]
fn em_dash_in_single_fragment_block_round_trips() {
    // Single-fragment block — the bug is MASKED here because
    // text_range.start = 0 for the only fragment, so char-vs-byte
    // confusion has nothing to bite on. Pre-fix this already passed;
    // keep it as a positive control so a regression in shaping or
    // hit-test wouldn't be misattributed to the multi-fragment case.
    let text = "abc — xyz";
    let ts = layout_md(text);
    assert_caret_x_strictly_increases(&ts, text, "single-fragment em-dash");
    assert_hit_test_round_trips(&ts, text, "single-fragment em-dash");
}

#[test]
fn em_dash_inside_bold_span_round_trips() {
    // Bold span containing an em-dash forces three fragments
    // (default + bold + default). The bold fragment's
    // FragmentContent.offset is in chars; pre-fix the bridge passed
    // that char offset as bytes to text-typeset, so glyph clusters
    // for the bold fragment and everything after it landed at the
    // wrong byte position in the block text.
    let rendered = "abc — bold — xyz";
    let ts = layout_md("abc **— bold —** xyz");
    assert_caret_x_strictly_increases(&ts, rendered, "bold em-dash");
    assert_hit_test_round_trips(&ts, rendered, "bold em-dash");
}

#[test]
fn em_dash_after_bold_prefix_round_trips() {
    // Bold prefix (`**a**`) makes the next fragment land at
    // char_offset == 1 / byte_offset == 1 in the block. The em-dash
    // appears in the second (plain) fragment so the bug shifted
    // clusters from there onward.
    let rendered = "a bcd — efg";
    let ts = layout_md("**a** bcd — efg");
    assert_caret_x_strictly_increases(&ts, rendered, "em-dash after bold");
    assert_hit_test_round_trips(&ts, rendered, "em-dash after bold");
}

#[test]
fn curly_quote_and_accent_round_trip() {
    // Curly apostrophe (’ = 3 bytes, 1 char), é (2 bytes, 1 char).
    // Same bridge bug, different codepoints — ensures the fix is
    // general, not em-dash-specific.
    let rendered = "It’s café x";
    let ts = layout_md("**It’s** café x");
    assert_caret_x_strictly_increases(&ts, rendered, "curly-quote / accent");
    assert_hit_test_round_trips(&ts, rendered, "curly-quote / accent");
}

#[allow(dead_code)]
fn _silence_unused() {
    let _ = NOTO_SANS;
}
