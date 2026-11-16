//! A flow's default language and directions, and the direction a paragraph
//! with no strongly directional character takes.
//!
//! A host gives a whole flow (a text field, an editor) a language and a
//! direction through `DocumentFlow::set_default_language`,
//! `set_default_direction` and `set_neutral_direction`, without writing them
//! into the document. These tests pin that the defaults reach exactly the
//! blocks that say nothing, through every layout path, and that the neutral
//! direction decides only where the text cannot.

mod helpers;

use helpers::{NOTO_ARABIC, NOTO_SANS, Typesetter, make_block, make_block_at};
use text_typeset::layout::paragraph::Alignment;
use text_typeset::{TextDirection, resolve_paragraph_direction};

/// "سلام" (salaam).
const SALAAM: &str = "\u{0633}\u{0644}\u{0627}\u{0645}";

/// Latin and Arabic, a 600 px wide flow.
fn bilingual() -> Typesetter {
    let mut ts = Typesetter::new();
    let latin = ts.register_font(NOTO_SANS);
    ts.register_font(NOTO_ARABIC);
    ts.set_default_font(latin, 16.0);
    ts.set_viewport(600.0, 400.0);
    ts
}

fn start_aligned(
    id: usize,
    position: usize,
    text: &str,
) -> text_typeset::layout::block::BlockLayoutParams {
    let mut block = make_block_at(id, position, text);
    block.alignment = Alignment::Start;
    block
}

/// Where the caret stands at `position`, in x.
fn caret_x(ts: &Typesetter, position: usize) -> f32 {
    ts.caret_rect(position)[0]
}

#[test]
fn auto_follows_the_first_strong_character() {
    let rtl = TextDirection::RightToLeft;
    let ltr = TextDirection::LeftToRight;
    assert_eq!(
        resolve_paragraph_direction(SALAAM, TextDirection::Auto, ltr),
        rtl
    );
    assert_eq!(
        resolve_paragraph_direction("12 abc", TextDirection::Auto, rtl),
        ltr
    );
    let mixed = format!("42, {SALAAM} abc");
    assert_eq!(
        resolve_paragraph_direction(&mixed, TextDirection::Auto, ltr),
        rtl
    );
}

#[test]
fn text_with_no_strong_character_takes_the_neutral_direction() {
    let rtl = TextDirection::RightToLeft;
    for text in ["", "123", "(42) !", " "] {
        assert_eq!(
            resolve_paragraph_direction(text, TextDirection::Auto, rtl),
            rtl,
            "{text:?}"
        );
        assert_eq!(
            resolve_paragraph_direction(text, TextDirection::Auto, TextDirection::Auto),
            TextDirection::LeftToRight,
            "{text:?}: UAX #9's own answer"
        );
    }
}

#[test]
fn a_strong_character_inside_an_isolate_does_not_decide() {
    // RLI abc PDI: the Latin is inside an isolate, so the paragraph has no
    // strong character of its own (UAX #9 P2).
    let text = "\u{2067}abc\u{2069}";
    assert_eq!(
        resolve_paragraph_direction(text, TextDirection::Auto, TextDirection::RightToLeft),
        TextDirection::RightToLeft
    );
}

#[test]
fn an_explicit_direction_ignores_the_text_and_the_neutral() {
    assert_eq!(
        resolve_paragraph_direction(
            SALAAM,
            TextDirection::LeftToRight,
            TextDirection::RightToLeft
        ),
        TextDirection::LeftToRight
    );
    assert_eq!(
        resolve_paragraph_direction("abc", TextDirection::RightToLeft, TextDirection::Auto),
        TextDirection::RightToLeft
    );
}

#[test]
fn an_empty_block_starts_on_the_right_when_the_neutral_direction_is_rtl() {
    let mut ts = bilingual();
    ts.layout_blocks(vec![start_aligned(0, 0, "")]);
    let left = caret_x(&ts, 0);
    assert!(
        left < 1.0,
        "an empty Auto block starts on the left; at {left}"
    );

    ts.flow.set_neutral_direction(TextDirection::RightToLeft);
    ts.layout_blocks(vec![start_aligned(0, 0, "")]);
    let right = caret_x(&ts, 0);
    assert!(
        right > 590.0,
        "with a right-to-left neutral direction it starts on the right; at {right}"
    );
    assert_eq!(ts.flow.direction_at(0), TextDirection::RightToLeft);
}

#[test]
fn the_neutral_direction_never_turns_text_that_has_a_direction() {
    let mut ts = bilingual();
    ts.flow.set_neutral_direction(TextDirection::RightToLeft);
    ts.layout_blocks(vec![start_aligned(0, 0, "abc")]);
    assert!(caret_x(&ts, 0) < 1.0, "Latin text stays on the left");
    assert_eq!(ts.flow.direction_at(1), TextDirection::LeftToRight);
}

#[test]
fn a_blocks_own_neutral_direction_wins_over_the_flows() {
    let mut ts = bilingual();
    ts.flow.set_neutral_direction(TextDirection::RightToLeft);
    let mut block = start_aligned(0, 0, "");
    block.neutral_direction = TextDirection::LeftToRight;
    ts.layout_blocks(vec![block]);
    assert!(caret_x(&ts, 0) < 1.0);
}

#[test]
fn the_default_direction_reaches_only_blocks_with_none() {
    let mut ts = bilingual();
    ts.flow.set_default_direction(TextDirection::RightToLeft);
    let unset = start_aligned(0, 0, "abc");
    let mut stated = start_aligned(1, 4, "abc");
    stated.base_direction = TextDirection::LeftToRight;
    ts.layout_blocks(vec![unset, stated]);

    // A right-to-left paragraph of Latin text sits against the right edge,
    // so its first character is far from the left.
    assert!(
        caret_x(&ts, 0) > 500.0,
        "the block with no direction takes the flow's; at {}",
        caret_x(&ts, 0)
    );
    assert!(
        caret_x(&ts, 4) < 1.0,
        "the block that states left to right keeps it; at {}",
        caret_x(&ts, 4)
    );
}

#[test]
fn the_defaults_reach_a_relaid_out_block_and_an_appended_one() {
    let mut ts = bilingual();
    ts.flow.set_neutral_direction(TextDirection::RightToLeft);
    ts.layout_blocks(vec![start_aligned(0, 0, "")]);
    ts.relayout_block(&start_aligned(0, 0, ""));
    assert!(
        caret_x(&ts, 0) > 590.0,
        "relayout_block: at {}",
        caret_x(&ts, 0)
    );

    ts.flow
        .add_block(&ts.service, &start_aligned(1, 1, ""))
        .expect("an append at the layout's scale");
    assert!(caret_x(&ts, 1) > 590.0, "add_block: at {}", caret_x(&ts, 1));
}

#[test]
fn the_defaults_reach_a_document_laid_out_in_full() {
    let doc = text_document::TextDocument::new();
    doc.set_plain_text("").unwrap();
    let mut ts = bilingual();
    ts.flow.set_neutral_direction(TextDirection::RightToLeft);
    ts.layout_full(&doc.snapshot_flow());
    assert!(caret_x(&ts, 0) > 590.0, "at {}", caret_x(&ts, 0));
}

#[test]
fn the_default_language_is_trimmed_and_an_empty_one_is_none() {
    let mut ts = Typesetter::new();
    ts.flow.set_default_language(Some("  ja-JP "));
    assert_eq!(ts.flow.default_language(), Some("ja-JP"));
    ts.flow.set_default_language(Some(""));
    assert_eq!(ts.flow.default_language(), None);
    // The bridge gives a block with no stored language `None`, and one with
    // a language its own.
    assert_eq!(make_block(0, "x").language, None);
}
