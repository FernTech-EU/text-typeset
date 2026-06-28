//! Shaping correctness for complex scripts (Arabic joining, Devanagari
//! conjuncts, Hebrew RTL ordering).
//!
//! These assert that shaping *actually transforms* the input — not just
//! that some glyphs come back. They guard the i18n path that the rest of
//! the suite only exercises as "doesn't panic" seeds.

mod helpers;
use std::collections::HashSet;

use helpers::{NOTO_ARABIC, NOTO_DEVANAGARI, NOTO_HEBREW, Typesetter};
use text_typeset::font::resolve::resolve_font;
use text_typeset::shaping::shaper::{TextDirection, shape_text, shape_text_directed};

/// Build a typesetter whose default font is the given face.
fn typesetter_with(font: &[u8]) -> Typesetter {
    let mut ts = Typesetter::new();
    let face = ts.register_font(font);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(800.0, 600.0);
    ts
}

/// Glyph id of the single glyph produced by shaping `text` in isolation.
fn isolated_glyph(ts: &Typesetter, text: &str) -> u16 {
    let resolved =
        resolve_font(ts.font_registry(), None, None, None, None, None, 1.0, 1.0).unwrap();
    let run = shape_text(ts.font_registry(), &resolved, text, 0).unwrap();
    run.glyphs.first().map(|g| g.glyph_id).unwrap_or(0)
}

#[test]
fn arabic_letters_join_into_contextual_forms() {
    let ts = typesetter_with(NOTO_ARABIC);
    let resolved =
        resolve_font(ts.font_registry(), None, None, None, None, None, 1.0, 1.0).unwrap();

    // Isolated forms of kaf, teh, beh.
    let isolated: HashSet<u16> = [
        isolated_glyph(&ts, "\u{0643}"), // ك kaf
        isolated_glyph(&ts, "\u{062A}"), // ت teh
        isolated_glyph(&ts, "\u{0628}"), // ب beh
    ]
    .into_iter()
    .collect();

    // The word "كتب" (kataba) — these three letters connect, so each
    // takes an initial/medial/final form distinct from its isolated form.
    let word = shape_text_directed(
        ts.font_registry(),
        &resolved,
        "\u{0643}\u{062A}\u{0628}",
        0,
        TextDirection::RightToLeft,
        &[],
    )
    .unwrap();

    assert!(
        word.glyphs.iter().all(|g| g.glyph_id != 0),
        "Arabic word should have no .notdef glyphs (wrong font?)"
    );

    let joined: Vec<u16> = word.glyphs.iter().map(|g| g.glyph_id).collect();
    assert!(
        joined.iter().any(|g| !isolated.contains(g)),
        "Arabic joining should produce contextual glyph forms distinct \
         from the isolated letters; got {joined:?}, isolated {isolated:?}"
    );
}

#[test]
fn devanagari_forms_a_conjunct() {
    let ts = typesetter_with(NOTO_DEVANAGARI);
    let resolved =
        resolve_font(ts.font_registry(), None, None, None, None, None, 1.0, 1.0).unwrap();

    // क + ् (virama) + ष  →  क्ष  : the virama ligates ka+ssa into a
    // single conjunct cluster, so 3 codepoints shape to fewer glyphs.
    let text = "\u{0915}\u{094D}\u{0937}";
    assert_eq!(text.chars().count(), 3);

    let run = shape_text(ts.font_registry(), &resolved, text, 0).unwrap();

    assert!(
        run.glyphs.iter().all(|g| g.glyph_id != 0),
        "Devanagari conjunct should have no .notdef glyphs (wrong font?)"
    );
    assert!(
        run.glyphs.len() < text.chars().count(),
        "Devanagari conjunct क्ष should shape to fewer glyphs than its \
         {} codepoints (GSUB conjunct formation); got {} glyphs",
        text.chars().count(),
        run.glyphs.len()
    );
}

#[test]
fn hebrew_rtl_glyphs_are_in_visual_order() {
    let ts = typesetter_with(NOTO_HEBREW);
    let resolved =
        resolve_font(ts.font_registry(), None, None, None, None, None, 1.0, 1.0).unwrap();

    // "שלום" (shalom). Shaped RTL, harfrust returns glyphs in visual
    // (left-to-right) order, so cluster byte offsets are non-increasing.
    let run = shape_text_directed(
        ts.font_registry(),
        &resolved,
        "\u{05E9}\u{05DC}\u{05D5}\u{05DD}",
        0,
        TextDirection::RightToLeft,
        &[],
    )
    .unwrap();

    assert!(!run.glyphs.is_empty());
    assert!(
        run.glyphs.iter().all(|g| g.glyph_id != 0),
        "Hebrew text should have no .notdef glyphs (wrong font?)"
    );

    let clusters: Vec<u32> = run.glyphs.iter().map(|g| g.cluster).collect();
    assert!(
        clusters.windows(2).all(|w| w[0] >= w[1]),
        "RTL shaping should return glyphs in visual order (non-increasing \
         clusters); got {clusters:?}"
    );
    // Sanity: it really did decrease somewhere (not a single-glyph run).
    assert!(
        clusters.first() > clusters.last(),
        "expected descending clusters across the RTL run; got {clusters:?}"
    );
}
