//! A footnote reference occupies one character and paints several glyphs.
//!
//! The document holds a single `U+FFFC` where a footnote is referenced; what a
//! reader sees there is a marker — "1", or "12", or whatever the host decided
//! the note is called. Those two facts pull in opposite directions, and every
//! test here pins one seam where they could come apart:
//!
//! * the marker must reserve the width its **own glyphs** advance to, not a
//!   guessed box (or the text after a footnote sits wrong);
//! * every one of those glyphs must map back to the **one** character the
//!   document actually has (or the caret stops between the `1` and the `2` of
//!   note twelve, at an offset no document position corresponds to);
//! * the marker must not inflate the line, because an inflated line is exempted
//!   from the interline multiplier — the bug `bd2cf71` fixed for images, which a
//!   footnote would otherwise reintroduce as "my leading changes when I add a
//!   note".

mod helpers;

use helpers::make_typesetter;
use text_typeset::layout::block::{BlockLayoutParams, FragmentParams};
use text_typeset::layout::paragraph::Alignment;
use text_typeset::{UnderlineStyle, VerticalAlignment};

/// `"Note"` + the sentinel + `"."`, with the middle fragment a footnote
/// reference carrying `marker`.
///
/// Byte offsets, because that is what `FragmentParams::offset` is: `"Note"`
/// spans 0..4, the sentinel is three bytes at 4..7, the full stop is at 7.
/// `length` is in characters, so the sentinel's is 1 however wide the marker
/// paints.
fn block_with_marker(id: usize, marker: Option<&str>) -> BlockLayoutParams {
    let text = "Note\u{FFFC}.";

    let frag = |text: &str, offset: usize, length: usize, marker: Option<&str>| FragmentParams {
        text: text.to_string(),
        offset,
        length,
        font_family: None,
        font_weight: None,
        font_bold: None,
        font_italic: None,
        font_point_size: None,
        underline_style: UnderlineStyle::None,
        overline: false,
        strikeout: false,
        is_link: false,
        letter_spacing: 0.0,
        word_spacing: 0.0,
        foreground_color: None,
        underline_color: None,
        background_color: None,
        anchor_href: None,
        tooltip: None,
        vertical_alignment: if marker.is_some() {
            VerticalAlignment::SuperScript
        } else {
            VerticalAlignment::Normal
        },
        image_name: None,
        image_width: 0.0,
        image_height: 0.0,
        footnote_marker: marker.map(str::to_string),
        features: Vec::new(),
    };

    BlockLayoutParams {
        base_direction: Default::default(),
        block_id: id,
        position: 0,
        text: text.to_string(),
        fragments: vec![
            frag("Note", 0, 4, None),
            frag("\u{FFFC}", 4, 1, marker),
            frag(".", 7, 1, None),
        ],
        alignment: Alignment::Left,
        top_margin: 0.0,
        bottom_margin: 0.0,
        left_margin: 0.0,
        right_margin: 0.0,
        text_indent: 0.0,
        list_marker: String::new(),
        list_indent: 0.0,
        tab_positions: vec![],
        line_height_multiplier: None,
        non_breakable_lines: false,
        hyphenation: None,
        checkbox: None,
        background_color: None,
    }
}

/// The sentinel is the fifth character of `"Note\u{FFFC}."`.
const SENTINEL_CHAR: usize = 4;

/// (a) The marker reserves what its own glyphs advance to.
///
/// Not a fixed box and not a guess: note 12 is wider than note 1, because two
/// digits are wider than one. A placeholder advance would pass every other test
/// in this file and still push the rest of the sentence to the wrong place.
#[test]
fn a_wider_marker_reserves_more_room() {
    let mut ts = make_typesetter();

    ts.layout_blocks(vec![block_with_marker(1, Some("1"))]);
    let one = ts.character_geometry(1, SENTINEL_CHAR, SENTINEL_CHAR + 1)[0].width;
    let one_block = ts.max_content_width();

    ts.layout_blocks(vec![block_with_marker(1, Some("144"))]);
    let many = ts.character_geometry(1, SENTINEL_CHAR, SENTINEL_CHAR + 1)[0].width;
    let many_block = ts.max_content_width();

    assert!(one > 0.0, "a marker must reserve real width, got {one}");
    assert!(
        many > one,
        "a three-digit marker ({many}) must reserve more room than a one-digit \
         marker ({one}) — equal widths mean the advance is a placeholder, not the \
         marker's own glyphs"
    );
    // The reserved width must also be the width the *line* grew by. These came
    // apart once: the block widened correctly while the caret stop after the
    // marker moved by a single glyph, because the collapsed clusters had been
    // pinned to the absolute offset and `flatten_runs` then lifted them a second
    // time. Measuring only one of the two would not have caught it.
    assert!(
        ((many_block - one_block) - (many - one)).abs() < 0.01,
        "the line grew by {} but the reference's own box grew by {} — the marker's \
         advance and its caret stops disagree",
        many_block - one_block,
        many - one
    );
}

/// (b) Every glyph of the marker maps back to the one character behind it.
///
/// This is the property that makes the reference atomic. Clicking the second
/// digit of "12" must land where clicking the first does — on the sentinel —
/// because there is no document position between them to land on. Without the
/// cluster collapse the shaper's own clusters run 0,1 within the marker and get
/// lifted into block-byte space, so the second digit resolves a character late.
#[test]
fn a_click_anywhere_in_the_marker_lands_on_the_sentinel() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![block_with_marker(1, Some("12"))]);

    // The marker spans the caret stop before the sentinel to the one after it.
    let before = ts.caret_rect(SENTINEL_CHAR);
    let after = ts.caret_rect(SENTINEL_CHAR + 1);
    let (left, right) = (before[0], after[0]);
    assert!(
        right > left,
        "the reference must occupy horizontal room: {left} .. {right}"
    );

    // Sample across the marker's whole advance, edges included.
    let y = before[1] + 1.0;
    let samples = 9;
    for i in 0..=samples {
        let x = left + (right - left) * (i as f32 / samples as f32);
        let Some(hit) = ts.hit_test(x, y) else {
            continue;
        };
        assert!(
            hit.offset_in_block == SENTINEL_CHAR || hit.offset_in_block == SENTINEL_CHAR + 1,
            "a click at x={x} inside the marker resolved to offset {} — the only \
             answers a one-character reference can give are {SENTINEL_CHAR} (before \
             it) and {} (after it)",
            hit.offset_in_block,
            SENTINEL_CHAR + 1
        );
    }
}

/// (c) The marker paints glyphs, and is not an image.
///
/// An image reserves a box the host measured and is drawn by the image layer; a
/// marker is text and goes through the ordinary glyph path. Producing an
/// `ImageQuad` here would mean the branch was written by copying the image one
/// too faithfully — and nothing would draw, because no host registers bytes for
/// a footnote.
#[test]
fn the_marker_paints_as_text_not_as_an_image() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![block_with_marker(1, Some("144"))]);

    let with_marker = {
        let frame = ts.render();
        assert!(
            frame.images.is_empty(),
            "a footnote marker must not produce an ImageQuad — got {}",
            frame.images.len()
        );
        frame.glyphs.len()
    };

    ts.layout_blocks(vec![block_with_marker(1, None)]);
    let without_marker = ts.render().glyphs.len();

    assert!(
        with_marker > without_marker,
        "the marker must contribute glyphs of its own: {with_marker} with it, \
         {without_marker} without"
    );
}

/// (e) A marker leaves the line alone.
///
/// `break_into_lines` inflates a line to fit anything taller than the font's
/// ascent, and `bd2cf71` then exempts an inflated line from the paragraph's
/// interline multiplier. A marker that inflated its line would therefore change
/// the leading of the paragraph holding it — a footnote silently reflowing the
/// page it sits on. A superscript is smaller than the text it rides on, so the
/// correct height is exactly the height without it.
#[test]
fn a_marker_does_not_change_the_line_height() {
    let mut ts = make_typesetter();

    ts.layout_blocks(vec![block_with_marker(1, None)]);
    let plain = ts.content_height();

    ts.layout_blocks(vec![block_with_marker(1, Some("144"))]);
    let marked = ts.content_height();

    assert!(
        (marked - plain).abs() < 0.01,
        "a footnote marker changed the block's height from {plain} to {marked} — \
         an inflated line is exempted from the interline multiplier, so this \
         would move every line of the paragraph"
    );
}

/// (e), under a line-height multiplier — the shape the image bug actually took.
///
/// The regression was only visible with a multiplier set: the inflated height
/// got multiplied and the difference was dumped below the line. A marker must
/// be invisible to that arithmetic, at 1.5× and at 0.8× alike (sub-1.0 line
/// heights arrive unclamped from imported HTML).
#[test]
fn a_marker_is_invisible_to_the_interline_multiplier() {
    for multiplier in [0.8_f32, 1.0, 1.5, 2.0] {
        let mut ts = make_typesetter();

        let mut plain_block = block_with_marker(1, None);
        plain_block.line_height_multiplier = Some(multiplier);
        ts.layout_blocks(vec![plain_block]);
        let plain = ts.content_height();

        let mut marked_block = block_with_marker(1, Some("12"));
        marked_block.line_height_multiplier = Some(multiplier);
        ts.layout_blocks(vec![marked_block]);
        let marked = ts.content_height();

        assert!(
            (marked - plain).abs() < 0.01,
            "at a {multiplier}× line height a marker changed the block from \
             {plain} to {marked}"
        );
    }
}

/// (d) The marker takes the direction of the text around it.
///
/// A reference is a neutral character, so in Arabic prose it belongs to the RTL
/// run and must not be dragged to paragraph level — the same rule the image
/// branch learned, where hardcoding level 0 left a picture on the wrong side of
/// the words it belonged to. The digits themselves shape LTR inside that run,
/// which is correct and is the bidi algorithm's job, not ours.
#[test]
fn a_marker_in_rtl_prose_stays_with_its_text() {
    let mut ts = make_typesetter();

    let mut block = block_with_marker(1, Some("12"));
    block.text = "مرحبا\u{FFFC}.".to_string();
    // "مرحبا" is ten bytes; the sentinel follows at 10..13, the stop at 13.
    block.fragments[0].text = "مرحبا".to_string();
    block.fragments[0].length = 5;
    block.fragments[1].offset = 10;
    block.fragments[2].offset = 13;
    block.base_direction = text_typeset::shaping::shaper::TextDirection::RightToLeft;

    ts.layout_blocks(vec![block]);

    // Measured from the caret stops, not `character_geometry`: that reports a
    // zero width for *every* character of an RTL block — plain Arabic prose
    // included, with no footnote anywhere near it — so it cannot tell a working
    // marker from a broken one here. (A real gap, since it is what feeds
    // per-character geometry to AccessKit, but a pre-existing one and not this
    // feature's to fix.)
    //
    // In RTL the caret before the sentinel sits to its *right*, so the reference
    // spans `caret(6) .. caret(5)`.
    let right = ts.caret_rect(5)[0];
    let left = ts.caret_rect(6)[0];
    let width = right - left;

    assert!(
        width > 0.0,
        "the marker must still reserve width in RTL prose, got {width}"
    );
    // Two digits, so twice a digit's advance — proof the marker shaped as itself
    // rather than falling back to the sentinel's own tofu box, which is 16.0.
    assert!(
        (width - 18.304).abs() < 0.01,
        "expected two digits' advance (18.304) in RTL prose, got {width} — 16.0 \
         would mean the sentinel was drawn instead of the marker"
    );
    assert!(
        ts.content_height() > 0.0,
        "an RTL block carrying a marker must still lay out"
    );
}
