//! Hyphenation: dictionary breaks (hypher), soft-hyphen (U+00AD) breaks,
//! the rendered hyphen glyph at wrapped line ends, and the off-by-default
//! behavior.
//!
//! Tests drive `layout_block` directly so they can inspect glyph ids in
//! the resulting lines (the rendered hyphen is the `-` glyph appended to a
//! non-final line).

mod helpers;
use helpers::{NOTO_SANS, make_block};

use text_typeset::TextFontService;
use text_typeset::font::resolve::resolve_font;
use text_typeset::layout::block::{BlockLayout, layout_block};
use text_typeset::shaping::shaper::shape_text;

fn service() -> TextFontService {
    let mut s = TextFontService::new_without_system_fonts();
    let face = s.register_font(NOTO_SANS);
    s.set_default_font(face, 16.0);
    s
}

/// Glyph id of `-` in the default font (the rendered-hyphen glyph).
fn hyphen_gid(s: &TextFontService) -> u16 {
    let resolved = resolve_font(s.font_registry(), None, None, None, None, None, 1.0).unwrap();
    shape_text(s.font_registry(), &resolved, "-", 0)
        .unwrap()
        .glyphs[0]
        .glyph_id
}

/// True if any non-final line ends with the hyphen glyph.
fn has_trailing_hyphen(layout: &BlockLayout, hyphen: u16) -> bool {
    let n = layout.lines.len();
    layout.lines.iter().take(n.saturating_sub(1)).any(|line| {
        line.runs
            .last()
            .and_then(|r| r.shaped_run.glyphs.last())
            .is_some_and(|g| g.glyph_id == hyphen)
    })
}

const LONG_WORD: &str = "antidisestablishmentarianism";

#[test]
fn dictionary_hyphenation_breaks_a_long_word_with_a_hyphen() {
    let s = service();
    let hyphen = hyphen_gid(&s);

    let mut params = make_block(1, LONG_WORD);
    params.hyphenate = true;
    // Narrow column forces the single long word to wrap mid-word.
    let layout = layout_block(s.font_registry(), &params, 90.0, 1.0);

    assert!(
        layout.lines.len() >= 2,
        "narrow column should wrap the long word onto multiple lines, got {}",
        layout.lines.len()
    );
    assert!(
        has_trailing_hyphen(&layout, hyphen),
        "a dictionary-hyphenated line should end with the hyphen glyph"
    );
}

#[test]
fn hyphenation_off_inserts_no_hyphen() {
    let s = service();
    let hyphen = hyphen_gid(&s);

    let mut params = make_block(1, LONG_WORD);
    params.hyphenate = false; // explicit: the default
    let layout = layout_block(s.font_registry(), &params, 90.0, 1.0);

    assert!(
        !has_trailing_hyphen(&layout, hyphen),
        "with hyphenation off no hyphen glyph should be inserted at line ends"
    );
    // The word itself contains no '-', so the glyph must not appear at all.
    let any_hyphen = layout
        .lines
        .iter()
        .flat_map(|l| &l.runs)
        .flat_map(|r| &r.shaped_run.glyphs)
        .any(|g| g.glyph_id == hyphen);
    assert!(
        !any_hyphen,
        "no hyphen glyph should be present with hyphenation off"
    );
}

#[test]
fn soft_hyphen_breaks_and_renders_a_hyphen() {
    let s = service();
    let hyphen = hyphen_gid(&s);

    // A long word with a single soft hyphen in the middle. With a narrow
    // column the line should break at the SHY and render a hyphen there.
    let text = "verylongunbreak\u{00AD}ablecompoundword";
    let mut params = make_block(1, text);
    params.hyphenate = true;
    let layout = layout_block(s.font_registry(), &params, 110.0, 1.0);

    assert!(layout.lines.len() >= 2, "soft-hyphen word should wrap");
    assert!(
        has_trailing_hyphen(&layout, hyphen),
        "a line broken at a soft hyphen should render the hyphen glyph"
    );
}

#[test]
fn hyphenation_does_not_change_text_that_fits() {
    // When everything fits on one line, hyphenation must be a no-op.
    let s = service();
    let hyphen = hyphen_gid(&s);

    let mut on = make_block(1, "short words here");
    on.hyphenate = true;
    let layout_on = layout_block(s.font_registry(), &on, 1000.0, 1.0);

    let off = make_block(1, "short words here");
    let layout_off = layout_block(s.font_registry(), &off, 1000.0, 1.0);

    assert_eq!(layout_on.lines.len(), 1);
    assert_eq!(layout_off.lines.len(), 1);
    assert!(!has_trailing_hyphen(&layout_on, hyphen));
    // Same glyph count on the single line whether hyphenation is on or off.
    let count = |l: &BlockLayout| -> usize {
        l.lines
            .iter()
            .flat_map(|ln| &ln.runs)
            .map(|r| r.shaped_run.glyphs.len())
            .sum()
    };
    assert_eq!(count(&layout_on), count(&layout_off));
}
