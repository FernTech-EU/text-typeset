//! System font enumeration (opt-in, default on) and the opt-out path.
//!
//! The opt-out test is deterministic. The default-on test depends on the
//! host having OS fonts installed, so it's `#[ignore]`d — run it locally
//! with `cargo test -- --ignored`.

mod helpers;
use helpers::NOTO_SANS;

use text_typeset::TextFontService;
use text_typeset::font::resolve::resolve_font;
use text_typeset::shaping::shaper::shape_text;

/// A CJK character NotoSans-Variable (Latin/Greek/Cyrillic) does not cover.
const CJK: &str = "\u{4E2D}"; // 中

#[test]
fn without_system_fonts_leaves_uncovered_chars_as_notdef() {
    let mut s = TextFontService::new_without_system_fonts();
    let face = s.register_font(NOTO_SANS);
    s.set_default_font(face, 16.0);

    // Only the registered Latin font exists, so there is nothing to fall
    // back to: the CJK char stays .notdef.
    assert_eq!(s.font_registry().all_entries().count(), 1);

    let resolved = resolve_font(s.font_registry(), None, None, None, None, None, 1.0, 1.0).unwrap();
    let run = shape_text(s.font_registry(), &resolved, CJK, 0).unwrap();
    assert!(
        run.glyphs.iter().any(|g| g.glyph_id == 0),
        "without system fonts an uncovered char should remain .notdef"
    );
}

#[test]
#[ignore = "depends on OS-installed fonts (non-deterministic in CI)"]
fn system_fonts_provide_fallback_for_uncovered_chars() {
    let mut s = TextFontService::new();
    let face = s.register_font(NOTO_SANS);
    s.set_default_font(face, 16.0);

    // load_system_fonts should have materialized many faces beyond the one
    // we registered.
    assert!(
        s.font_registry().all_entries().count() > 1,
        "new() should enumerate OS fonts"
    );

    let resolved = resolve_font(s.font_registry(), None, None, None, None, None, 1.0, 1.0).unwrap();
    let run = shape_text(s.font_registry(), &resolved, CJK, 0).unwrap();
    assert!(
        run.glyphs.iter().all(|g| g.glyph_id != 0),
        "system-font fallback should cover a CJK char (is a CJK font installed?)"
    );
}
