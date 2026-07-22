//! Shaping correctness for complex scripts (Arabic joining, Devanagari
//! conjuncts, Hebrew RTL ordering).
//!
//! These assert that shaping *actually transforms* the input — not just
//! that some glyphs come back. They guard the i18n path that the rest of
//! the suite only exercises as "doesn't panic" seeds.

mod helpers;
use std::collections::HashSet;

use helpers::{NOTO_ARABIC, NOTO_DEVANAGARI, NOTO_HEBREW, NOTO_SANS, Typesetter};
use text_typeset::font::resolve::resolve_font;
use text_typeset::shaping::shaper::{
    TextDirection, shape_text, shape_text_directed, shape_text_with_fallback,
};

/// Build a typesetter whose default font is the given face.
fn typesetter_with(font: &[u8]) -> Typesetter {
    let mut ts = Typesetter::new();
    let face = ts.register_font(font);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(800.0, 600.0);
    ts
}

/// Every glyph id produced by shaping `text` on its own.
///
/// A single Arabic letter is not a single glyph: in Noto Sans Arabic each
/// of these decomposes into a base plus its dots (kaf `[357, 58]`, teh
/// `[286, 14]`, beh `[315, 14]`). An oracle that kept only the *first*
/// glyph would leave the dot components out of the "isolated" set, and
/// then any assertion of the form "some glyph is not isolated" passes on
/// the strength of a dot alone — whether or not joining happened. That is
/// exactly the false positive this helper replaces, so it deliberately
/// returns the whole sequence.
fn isolated_glyphs(ts: &Typesetter, text: &str) -> Vec<u16> {
    let resolved =
        resolve_font(ts.font_registry(), None, None, None, None, None, 1.0, 1.0).unwrap();
    let run = shape_text(ts.font_registry(), &resolved, text, 0).unwrap();
    run.glyphs.iter().map(|g| g.glyph_id).collect()
}

/// ك kaf, ت teh, ب beh — the letters of "كتب", all of which connect.
const KAF: &str = "\u{0643}";
const TEH: &str = "\u{062A}";
const BEH: &str = "\u{0628}";
const KATABA: &str = "\u{0643}\u{062A}\u{0628}";

/// The union of every glyph the three letters produce standing alone.
fn isolated_repertoire(ts: &Typesetter) -> HashSet<u16> {
    [KAF, TEH, BEH]
        .into_iter()
        .flat_map(|l| isolated_glyphs(ts, l))
        .collect()
}

#[test]
fn arabic_letters_join_into_contextual_forms() {
    let ts = typesetter_with(NOTO_ARABIC);
    let resolved =
        resolve_font(ts.font_registry(), None, None, None, None, None, 1.0, 1.0).unwrap();

    let isolated = isolated_repertoire(&ts);

    // The word "كتب" (kataba). Because the three letters connect, each
    // takes an initial/medial/final form — glyphs that appear in *none*
    // of the isolated renderings.
    let word = shape_text_directed(
        ts.font_registry(),
        &resolved,
        KATABA,
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
    let contextual: Vec<u16> = joined
        .iter()
        .copied()
        .filter(|g| !isolated.contains(g))
        .collect();

    // The real oracle: at least one glyph outside the isolated
    // repertoire. Merely *reordering* the isolated forms — which is what
    // the explicit-direction path did while `buffer.script` went unset —
    // yields a `joined` drawn entirely from `isolated`, and fails here.
    assert!(
        !contextual.is_empty(),
        "Arabic joining should produce contextual glyph forms outside the \
         isolated repertoire; got {joined:?}, all of which are isolated \
         forms drawn from {isolated:?} — the letters did not join"
    );
}

#[test]
fn arabic_joining_survives_an_explicit_direction() {
    let ts = typesetter_with(NOTO_ARABIC);
    let resolved =
        resolve_font(ts.font_registry(), None, None, None, None, None, 1.0, 1.0).unwrap();

    let shape = |dir| {
        let run = shape_text_directed(ts.font_registry(), &resolved, KATABA, 0, dir, &[]).unwrap();
        run.glyphs.iter().map(|g| g.glyph_id).collect::<Vec<u16>>()
    };

    // Pure-Arabic text auto-detects as RTL, so naming that direction
    // explicitly must not change a thing. It used to: an explicit
    // direction skipped `guess_segment_properties`, leaving the buffer's
    // script `None`, and harfrust fell back to DEFAULT_SHAPER — which
    // requests no init/medi/fina/isol and so never joins. The bidi-aware
    // layout path always shapes with an explicit direction, so that path
    // alone rendered Arabic disconnected.
    assert_eq!(
        shape(TextDirection::RightToLeft),
        shape(TextDirection::Auto),
        "explicitly naming the direction that auto-detection would have \
         picked must produce identical glyphs"
    );

    // And it is joined, not merely equal-and-broken.
    let isolated = isolated_repertoire(&ts);
    assert!(
        shape(TextDirection::RightToLeft)
            .iter()
            .any(|g| !isolated.contains(g)),
        "the explicit-direction path must produce joined forms"
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

#[test]
fn arabic_joins_when_it_arrives_through_glyph_fallback() {
    // The realistic case, and the one that was broken: the writing font is
    // a Latin serif with no Arabic at all, so every Arabic character
    // reaches the shaper as .notdef and is re-shaped in a fallback font.
    //
    // That path used to substitute one character at a time and keep only
    // the first glyph of each result. A letter shaped alone has no
    // neighbours and can only take its isolated form, and dropping the
    // second glyph dropped the dots — so Arabic set in a Latin font came
    // out as disconnected, dotless stumps. Shaping the whole .notdef span
    // in the fallback font instead makes it identical to shaping it in
    // that font directly.
    let mut fallback = Typesetter::new();
    let latin = fallback.register_font(NOTO_SANS);
    fallback.register_font(NOTO_ARABIC);
    fallback.set_default_font(latin, 16.0);

    let mut native = Typesetter::new();
    let arabic = native.register_font(NOTO_ARABIC);
    native.set_default_font(arabic, 16.0);

    let glyphs = |ts: &Typesetter| -> Vec<u16> {
        let resolved =
            resolve_font(ts.font_registry(), None, None, None, None, None, 1.0, 1.0).unwrap();
        shape_text_with_fallback(
            ts.font_registry(),
            &resolved,
            KATABA,
            0,
            TextDirection::RightToLeft,
            &[],
        )
        .unwrap()
        .glyphs
        .iter()
        .map(|g| g.glyph_id)
        .collect()
    };

    let via_fallback = glyphs(&fallback);
    let direct = glyphs(&native);

    assert!(
        via_fallback.iter().all(|&g| g != 0),
        "fallback should have resolved every glyph; got {via_fallback:?}"
    );
    assert_eq!(
        via_fallback,
        direct,
        "Arabic reached through glyph fallback must shape the same as Arabic \
         shaped natively — {} glyphs vs {}; per-character fallback yields the \
         isolated bases with the dots stripped",
        via_fallback.len(),
        direct.len()
    );
}
