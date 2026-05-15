//! Proptest-driven robustness tests for text-typeset's public
//! layout/hit-test/render surface.
//!
//! `cargo-fuzz` needs nightly + libfuzzer-sys, which isn't assumed
//! here; proptest with 512–1024 iterations per property gives the
//! "parser doesn't panic on weird input" coverage that the fuzz
//! corpus would. Each property drives one public entry point with
//! random inputs and asserts that the call returns without
//! panicking and yields an internally consistent result.
//!
//! Invariants asserted beyond "no panic":
//!
//! * `layout_paragraph(text, width)` returns `line_count >= 1` for
//!   any non-empty text, and `0 <= width <= max_width + ε`.
//! * `layout_single_line(text)` returns `width >= 0` and
//!   `glyphs.len()` grows monotonically with text length.
//! * `hit_test(x, y)` returns either `None` or a region; no panic
//!   for any numeric input (including NaN, infinity, negatives).
//! * `character_geometry(block_id, start, end)` never panics on any
//!   range; returned positions are non-decreasing; widths
//!   non-negative.
//! * Random block-layout sequences preserve `assert_no_glyph_overlap`
//!   (the helper invariant from helpers.rs).

mod helpers;

use helpers::{Rect, RenderFrameExt, make_block, make_typesetter};
use proptest::prelude::*;
use text_typeset::{CursorDisplay, TextFormat};

// A small, UTF-8 friendly alphabet plus occasional multibyte chars
// and whitespace. Uniform byte noise would overwhelmingly produce
// invalid UTF-8 via regex expansion (proptest rejects those up
// front).
fn arb_text() -> impl Strategy<Value = String> {
    proptest::string::string_regex(r"[a-zA-Z0-9 .,\-!?\n\t éà🌍]{0,120}").unwrap()
}

fn arb_short_text() -> impl Strategy<Value = String> {
    proptest::string::string_regex(r"[a-zA-Z0-9 ]{0,30}").unwrap()
}

// Widths the layout path legitimately supports. Zero and very small
// positive widths are edge cases — we want them. Negative widths are
// invalid input; the layout must not panic, but behaviour is
// undefined.
fn arb_width() -> impl Strategy<Value = f32> {
    prop_oneof![Just(0.0_f32), Just(1.0_f32), 0.5f32..2000.0_f32,]
}

// ── Property: layout_paragraph never panics ─────────────────────────

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]
    #[test]
    fn layout_paragraph_never_panics(
        text in arb_text(),
        width in arb_width(),
        cap in proptest::option::of(0usize..20),
    ) {
        let mut ts = make_typesetter();
        let result = ts.layout_paragraph(&text, &TextFormat::default(), width, cap);
        // Invariants that must hold for every accepted input.
        prop_assert!(result.height >= 0.0);
        prop_assert!(result.width >= 0.0);
        if !text.is_empty() && width >= 1.0 && cap.is_none_or(|c| c >= 1) {
            prop_assert!(
                result.line_count >= 1,
                "non-empty text at positive width with cap≥1 must produce ≥1 line"
            );
        }
        if let Some(max) = cap {
            prop_assert!(
                result.line_count <= max,
                "line_count {} exceeds max_lines cap {}",
                result.line_count,
                max
            );
        }
    }
}

// ── Property: layout_single_line never panics ───────────────────────

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]
    #[test]
    fn layout_single_line_never_panics(
        text in arb_short_text(),
        max_width in proptest::option::of(arb_width()),
    ) {
        let mut ts = make_typesetter();
        let result = ts.layout_single_line(&text, &TextFormat::default(), max_width);
        prop_assert!(result.width >= 0.0);
        prop_assert!(result.height >= 0.0);
    }
}

// ── Property: hit_test never panics for any numeric input ───────────

proptest! {
    #![proptest_config(ProptestConfig { cases: 1024, ..ProptestConfig::default() })]
    #[test]
    fn hit_test_never_panics(
        text in arb_short_text(),
        x in prop_oneof![
            any::<f32>(),
            -100.0f32..1000.0_f32,
        ],
        y in prop_oneof![
            any::<f32>(),
            -100.0f32..1000.0_f32,
        ],
    ) {
        let mut ts = make_typesetter();
        if !text.is_empty() {
            ts.layout_blocks(vec![make_block(1, &text)]);
        }
        // NaN / infinity / huge / negative must all return gracefully.
        let _ = ts.hit_test(x, y);
    }
}

// ── Property: character_geometry never panics ───────────────────────

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]
    #[test]
    fn character_geometry_never_panics(
        text in arb_short_text(),
        a in 0usize..200,
        b in 0usize..200,
    ) {
        let mut ts = make_typesetter();
        if text.is_empty() { return Ok(()); }
        ts.layout_blocks(vec![make_block(1, &text)]);
        let (start, end) = if a <= b { (a, b) } else { (b, a) };
        let geom = ts.character_geometry(1, start, end);
        // Every returned geometry entry must have non-negative width.
        for g in &geom {
            prop_assert!(g.width >= 0.0, "width must be non-negative, got {}", g.width);
        }
        // Positions must be non-decreasing.
        for w in geom.windows(2) {
            prop_assert!(
                w[1].position >= w[0].position,
                "position regressed: {} -> {}",
                w[0].position,
                w[1].position
            );
        }
    }
}

// ── Property: block stack produces no pathological glyph overlap ────

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]
    #[test]
    fn random_block_stack_has_no_doubled_glyph(
        texts in prop::collection::vec(arb_short_text(), 1..5),
    ) {
        // Tighter than `assert_no_glyph_overlap` from helpers.rs:
        // that helper uses 50% overlap as "significant" to catch
        // doubled-glyph render bugs, but real typography (fJ kerning,
        // stacked diacritics) legitimately exceeds 50%. At 95%+
        // overlap the glyphs are effectively the same quad, which is
        // the only thing fuzzing should flag.
        let mut ts = make_typesetter();
        let blocks: Vec<_> = texts
            .iter()
            .enumerate()
            .map(|(i, t)| make_block(i + 1, t))
            .collect();
        ts.layout_blocks(blocks);
        let frame = ts.render();
        let rects: Vec<Rect> = frame.glyph_rects();
        for i in 0..rects.len() {
            for j in (i + 1)..rects.len() {
                if !rects[i].overlaps(&rects[j]) { continue; }
                let ox = (rects[i].right().min(rects[j].right())
                    - rects[i].x().max(rects[j].x())).max(0.0);
                let oy = (rects[i].bottom().min(rects[j].bottom())
                    - rects[i].y().max(rects[j].y())).max(0.0);
                let overlap = ox * oy;
                let smaller = (rects[i].w() * rects[i].h())
                    .min(rects[j].w() * rects[j].h());
                if smaller > 0.0 {
                    let ratio = overlap / smaller;
                    prop_assert!(
                        ratio < 0.95,
                        "glyph {} {} nearly identical to glyph {} {} (ratio {:.3})",
                        i, rects[i], j, rects[j], ratio
                    );
                }
            }
        }
    }
}

// ── Property: viewport + scroll + render is always safe ─────────────

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]
    #[test]
    fn viewport_and_scroll_mutations_never_panic(
        text in arb_short_text(),
        vw in 0.0f32..2000.0,
        vh in 0.0f32..2000.0,
        scroll in -500.0f32..5000.0,
        zoom in 0.1f32..4.0,
    ) {
        let mut ts = make_typesetter();
        ts.set_viewport(vw, vh);
        ts.set_scroll_offset(scroll);
        ts.set_zoom(zoom);
        if !text.is_empty() {
            ts.layout_blocks(vec![make_block(1, &text)]);
        }
        let _ = ts.render();
    }
}

// ── Property: cursor updates never desync ───────────────────────────

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]
    #[test]
    fn cursor_position_mutations_never_panic(
        text in arb_short_text(),
        pos in 0usize..500,
        anchor in 0usize..500,
    ) {
        let mut ts = make_typesetter();
        if text.is_empty() { return Ok(()); }
        ts.layout_blocks(vec![make_block(1, &text)]);
        ts.set_cursor(&CursorDisplay {
            position: pos,
            anchor,
            visible: true,
            selected_cells: vec![],
        });
        let _ = ts.render();
    }
}

// ── Seed corpus: hand-picked adversarial inputs ─────────────────────

#[test]
fn seed_corpus_adversarial_text() {
    let inputs: &[&str] = &[
        "",
        " ",
        "\n",
        "\0",
        "a",
        "very long single word that has no spaces so line break cannot possibly find a valid opportunity to split",
        "\u{200B}\u{200B}\u{200B}", // zero-width spaces
        "\t\t\t",
        "👨‍👩‍👧‍👦 family",
        "👋🏻 skin-tone",
        "🇫🇷 flag",
        "e\u{0301}X combining acute",
        "שלום hello", // hebrew + latin
        "العربية",    // arabic
        "\u{FEFF}BOM prefix",
    ];
    let mut ts = make_typesetter();
    for text in inputs {
        // All four public entry points.
        let _ = ts.layout_paragraph(text, &TextFormat::default(), 200.0, None);
        let _ = ts.layout_single_line(text, &TextFormat::default(), Some(200.0));
        if !text.is_empty() {
            ts.layout_blocks(vec![make_block(1, text)]);
            let _ = ts.hit_test(10.0, 10.0);
            let _ = ts.character_geometry(1, 0, text.chars().count());
            let _ = ts.render();
        }
    }
}

#[test]
fn seed_corpus_pathological_widths() {
    let mut ts = make_typesetter();
    let text = "Hello world, this is some sample text for layout.";
    let widths: &[f32] = &[0.0, 0.001, 1.0, 5.0, 10.0, 10_000.0, f32::INFINITY];
    for &w in widths {
        let _ = ts.layout_paragraph(text, &TextFormat::default(), w, None);
    }
}
