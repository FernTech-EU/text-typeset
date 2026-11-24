//! `DocumentFlow::set_dim_outside`: everything outside one range of the text
//! is drawn at reduced opacity, the range itself at full strength.
//!
//! These pin the observable contract on the `RenderFrame`: which glyphs,
//! images and strokes lose alpha, which decorations never do, and that every
//! render path (full, block-only, cursor-only) shows the range currently set
//! rather than the one the cached quads were built with.

mod helpers;

use helpers::{
    NOTO_HEBREW, Typesetter, make_block, make_block_at, make_cell_at, make_table, make_typesetter,
};
use text_typeset::{DecorationKind, DimOutside, RenderFrame, UnderlineStyle};

const DIM: f32 = 0.4;

fn alphas(frame: &RenderFrame) -> Vec<f32> {
    frame.glyphs.iter().map(|g| g.color[3]).collect()
}

fn count(frame: &RenderFrame, alpha: f32) -> usize {
    alphas(frame)
        .iter()
        .filter(|a| (*a - alpha).abs() < 1e-4)
        .count()
}

/// Each glyph's screen rect and alpha, copied out so the frame's borrow of
/// the typesetter ends.
fn glyphs(frame: &RenderFrame) -> Vec<([f32; 4], f32)> {
    frame
        .glyphs
        .iter()
        .map(|g| (g.screen, g.color[3]))
        .collect()
}

/// The alphas of the glyphs drawn inside one block's vertical band.
fn alphas_in_block(ts: &Typesetter, glyphs: &[([f32; 4], f32)], block_id: usize) -> Vec<f32> {
    let info = ts.block_visual_info(block_id).expect("block laid out");
    glyphs
        .iter()
        .filter(|(screen, _)| {
            let mid = screen[1] + screen[3] * 0.5;
            mid >= info.y && mid <= info.y + info.height
        })
        .map(|(_, alpha)| *alpha)
        .collect()
}

fn all(values: &[f32], alpha: f32) -> bool {
    !values.is_empty() && values.iter().all(|a| (a - alpha).abs() < 1e-4)
}

/// Two paragraphs: "Hello" at 0..5, "World" at 6..11 (the separator is 5).
fn two_paragraphs() -> Typesetter {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![
        make_block_at(1, 0, "Hello"),
        make_block_at(2, 6, "World"),
    ]);
    ts
}

#[test]
fn without_a_range_nothing_is_dimmed() {
    let mut ts = two_paragraphs();
    let frame = ts.render();
    assert!(all(&alphas(frame), 1.0));
}

#[test]
fn the_paragraph_in_the_range_stays_bright_and_the_other_fades() {
    let mut ts = two_paragraphs();
    ts.flow.set_dim_outside(Some(DimOutside::new(6, 11, DIM)));
    let frame = glyphs(ts.render());
    let first = alphas_in_block(&ts, &frame, 1);
    let second = alphas_in_block(&ts, &frame, 2);
    assert!(all(&first, DIM), "outside the range: {first:?}");
    assert!(all(&second, 1.0), "inside the range: {second:?}");
}

/// A sentence boundary in the middle of a run: the glyphs on each side of it
/// get their own alpha. Spaces rasterize to nothing, so they are not counted.
#[test]
fn a_sentence_inside_a_paragraph_is_cut_at_its_characters() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "One two. Three four.")]);
    // "Three four." is 9..20.
    ts.flow.set_dim_outside(Some(DimOutside::new(9, 20, DIM)));
    let frame = ts.render();
    assert_eq!(count(frame, 1.0), "Threefour.".len());
    assert_eq!(count(frame, DIM), "Onetwo.".len());
}

/// Clusters are character offsets once a line is laid out; reading them as
/// bytes would shift every boundary after a multi-byte character.
#[test]
fn positions_are_characters_not_bytes() {
    let mut ts = make_typesetter();
    // "ééé." is four characters but seven bytes; "Abc." is 5..9.
    ts.layout_blocks(vec![make_block(1, "ééé. Abc.")]);
    ts.flow.set_dim_outside(Some(DimOutside::new(5, 9, DIM)));
    let frame = ts.render();
    assert_eq!(count(frame, 1.0), 4, "exactly \"Abc.\" is bright");
    assert_eq!(count(frame, DIM), 4, "exactly \"ééé.\" is dim");
}

/// In a right-to-left paragraph the first sentence is on the right. The cut
/// follows the characters, whatever side of the screen they land on.
#[test]
fn right_to_left_text_is_cut_by_its_characters() {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_HEBREW);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(800.0, 600.0);
    // "שלום. עולם." — the second sentence, "עולם.", is 6..11.
    let text = "\u{05E9}\u{05DC}\u{05D5}\u{05DD}. \u{05E2}\u{05D5}\u{05DC}\u{05DD}.";
    ts.layout_blocks(vec![make_block(1, text)]);
    ts.flow.set_dim_outside(Some(DimOutside::new(6, 11, DIM)));
    ts.render();
    let start_of_second = ts.caret_rect(6)[0];
    let frame = ts.render();
    let mut bright_xs = Vec::new();
    let mut dim_xs = Vec::new();
    for g in &frame.glyphs {
        if (g.color[3] - 1.0).abs() < 1e-4 {
            bright_xs.push(g.screen[0]);
        } else {
            dim_xs.push(g.screen[0]);
        }
    }
    assert_eq!(bright_xs.len(), 5);
    assert_eq!(dim_xs.len(), 5);
    // The bright sentence reads second, so it sits to the LEFT of where it
    // starts; the dim one sits to the right of that point.
    assert!(bright_xs.iter().all(|x| *x < start_of_second));
    assert!(dim_xs.iter().all(|x| *x > start_of_second - 1.0));
}

/// An empty range keeps nothing bright: the caret in an empty paragraph.
#[test]
fn an_empty_range_dims_everything() {
    let mut ts = two_paragraphs();
    ts.flow.set_dim_outside(Some(DimOutside::new(5, 5, DIM)));
    let frame = ts.render();
    assert!(all(&alphas(frame), DIM));
}

#[test]
fn an_out_of_range_opacity_is_clamped() {
    let mut ts = two_paragraphs();
    ts.flow.set_dim_outside(Some(DimOutside::new(6, 11, -3.0)));
    let frame = ts.render();
    assert_eq!(count(frame, 0.0), "Hello".len());
    ts.flow
        .set_dim_outside(Some(DimOutside::new(6, 11, f32::NAN)));
    let frame = ts.render();
    assert!(all(&alphas(frame), 1.0), "a NaN opacity dims nothing");
}

/// An opacity with no effect (NaN) and a real one differ on every faded
/// glyph, so moving between them redraws them all on the incremental paths
/// too. Compared as raw numbers, NaN looked like no change of opacity, and a
/// caret move inside one block rebuilt that block alone: the other paragraph
/// kept the old opacity until a full render.
#[test]
fn moving_between_a_nan_and_a_real_opacity_redraws_every_block() {
    for (before, after) in [(DIM, f32::NAN), (f32::NAN, DIM)] {
        let mut ts = two_paragraphs();
        ts.flow.set_dim_outside(Some(DimOutside::new(0, 5, before)));
        ts.render();
        ts.flow.set_dim_outside(Some(DimOutside::new(0, 5, after)));
        let incremental = alphas(ts.render_cursor_only());
        let full = alphas(ts.render());
        assert_eq!(incremental, full, "{before} then {after}");
    }
}

/// The writer's own colour survives: only the alpha changes.
#[test]
fn a_coloured_run_keeps_its_colour() {
    let mut ts = make_typesetter();
    let mut block = make_block(1, "Red");
    block.fragments[0].foreground_color = Some([1.0, 0.0, 0.0, 1.0]);
    ts.layout_blocks(vec![block]);
    ts.flow.set_dim_outside(Some(DimOutside::new(10, 10, DIM)));
    let frame = ts.render();
    for g in &frame.glyphs {
        assert_eq!(g.color, [1.0, 0.0, 0.0, DIM]);
    }
}

fn image_block(id: usize, position: usize) -> text_typeset::layout::block::BlockLayoutParams {
    let mut block = make_block_at(id, position, "\u{FFFC}");
    let fragment = &mut block.fragments[0];
    fragment.image_name = Some("picture.png".to_string());
    fragment.image_width = 40.0;
    fragment.image_height = 30.0;
    fragment.length = "\u{FFFC}".len();
    block
}

#[test]
fn an_image_outside_the_range_carries_the_opacity() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![image_block(1, 0), make_block_at(2, 2, "Text")]);
    let frame = ts.render();
    assert_eq!(frame.images[0].opacity, 1.0, "no range: fully opaque");

    ts.flow.set_dim_outside(Some(DimOutside::new(2, 6, DIM)));
    let frame = ts.render();
    assert_eq!(frame.images[0].opacity, DIM);

    ts.flow.set_dim_outside(Some(DimOutside::new(0, 1, DIM)));
    let frame = ts.render();
    assert_eq!(
        frame.images[0].opacity, 1.0,
        "the image is the bright range"
    );
}

fn underlined(text: &str, style: UnderlineStyle) -> Typesetter {
    let mut ts = make_typesetter();
    let mut block = make_block(1, text);
    block.fragments[0].underline_style = style;
    ts.layout_blocks(vec![block]);
    ts
}

/// A single underline under both sentences is cut where the glyphs change,
/// and the two pieces still cover the run with no gap and no overlap.
#[test]
fn an_underline_is_cut_at_the_sentence_boundary() {
    let mut ts = underlined("Aaaa. Bbbb.", UnderlineStyle::Single);
    let whole: Vec<[f32; 4]> = ts
        .render()
        .decorations
        .iter()
        .filter(|d| d.kind == DecorationKind::Underline)
        .map(|d| d.rect)
        .collect();
    assert_eq!(whole.len(), 1);
    let [x, _, w, _] = whole[0];

    // "Bbbb." is 6..11.
    ts.flow.set_dim_outside(Some(DimOutside::new(6, 11, DIM)));
    let boundary = ts.caret_rect(6)[0];
    let frame = ts.render();
    let mut pieces: Vec<([f32; 4], f32)> = frame
        .decorations
        .iter()
        .filter(|d| d.kind == DecorationKind::Underline)
        .map(|d| (d.rect, d.color[3]))
        .collect();
    pieces.sort_by(|a, b| a.0[0].total_cmp(&b.0[0]));
    assert_eq!(pieces.len(), 2, "one dim piece, one bright piece");
    let (left, right) = (pieces[0], pieces[1]);
    assert!((left.1 - DIM).abs() < 1e-4 && (right.1 - 1.0).abs() < 1e-4);
    assert!((left.0[0] - x).abs() < 0.01);
    assert!((left.0[0] + left.0[2] - right.0[0]).abs() < 0.01, "no gap");
    assert!((right.0[0] + right.0[2] - (x + w)).abs() < 0.01);
    // The cut lands on the space before "Bbbb.", which belongs to the dim
    // side, so it is no further right than where "B" starts.
    assert!(right.0[0] <= boundary + 0.5);
}

/// The spell-check squiggle is ink too, and stays one connected wave: its
/// bands are cut, never re-phased.
#[test]
fn a_squiggle_fades_with_its_word() {
    let mut ts = underlined("Wrongg word.", UnderlineStyle::SpellCheck);
    let before: Vec<[f32; 4]> = ts
        .render()
        .decorations
        .iter()
        .filter(|d| d.kind == DecorationKind::Underline)
        .map(|d| d.rect)
        .collect();
    ts.flow.set_dim_outside(Some(DimOutside::new(20, 20, DIM)));
    let after: Vec<([f32; 4], f32)> = ts
        .render()
        .decorations
        .iter()
        .filter(|d| d.kind == DecorationKind::Underline)
        .map(|d| (d.rect, d.color[3]))
        .collect();
    assert_eq!(after.len(), before.len(), "nothing to cut, nothing added");
    for ((rect, alpha), original) in after.iter().zip(&before) {
        assert_eq!(rect, original, "same wave");
        assert!((alpha - DIM).abs() < 1e-4);
    }
}

/// Backgrounds are not ink. A highlight behind the text, the caret band
/// among them, keeps its strength.
#[test]
fn a_text_background_is_never_dimmed() {
    let mut ts = make_typesetter();
    let mut block = make_block(1, "Shaded");
    block.fragments[0].background_color = Some([1.0, 1.0, 0.0, 0.5]);
    block.background_color = Some([0.0, 0.0, 1.0, 1.0]);
    ts.layout_blocks(vec![block]);
    ts.flow.set_dim_outside(Some(DimOutside::new(50, 50, DIM)));
    let frame = ts.render();
    let text_bg = frame
        .decorations
        .iter()
        .find(|d| d.kind == DecorationKind::TextBackground)
        .expect("text background");
    assert_eq!(text_bg.color, [1.0, 1.0, 0.0, 0.5]);
    let block_bg = frame
        .decorations
        .iter()
        .find(|d| d.kind == DecorationKind::BlockBackground)
        .expect("block background");
    assert_eq!(block_bg.color, [0.0, 0.0, 1.0, 1.0]);
}

#[test]
fn the_caret_is_never_dimmed() {
    let mut ts = two_paragraphs();
    ts.set_cursor(&text_typeset::CursorDisplay {
        position: 2,
        anchor: 2,
        affinity: text_typeset::CursorAffinity::Downstream,
        visible: true,
        selected_cells: Vec::new(),
    });
    ts.flow.set_dim_outside(Some(DimOutside::new(6, 11, DIM)));
    let frame = ts.render();
    let caret = frame
        .decorations
        .iter()
        .find(|d| d.kind == DecorationKind::Cursor)
        .expect("caret");
    assert_eq!(caret.color[3], 1.0);
}

fn list_item(
    id: usize,
    position: usize,
    text: &str,
) -> text_typeset::layout::block::BlockLayoutParams {
    let mut block = make_block_at(id, position, text);
    block.list_marker = "1.".to_string();
    block.list_indent = 30.0;
    block
}

/// A list marker belongs to its paragraph, not to a character: it is bright
/// whenever the bright range touches the item, even when that range is the
/// item's second sentence or the empty range of an empty item.
#[test]
fn a_list_marker_follows_its_paragraph() {
    let marker_alpha = |ts: &mut Typesetter| -> Vec<f32> {
        let item = ts.block_visual_info(1).expect("laid out");
        ts.render()
            .glyphs
            .iter()
            .filter(|g| {
                let mid_y = g.screen[1] + g.screen[3] * 0.5;
                g.screen[0] < item.content_x && mid_y >= item.y && mid_y <= item.y + item.height
            })
            .map(|g| g.color[3])
            .collect()
    };

    let mut ts = make_typesetter();
    ts.layout_blocks(vec![
        list_item(1, 0, "One. Two."),
        make_block_at(2, 10, "Else"),
    ]);
    // The item's second sentence.
    ts.flow.set_dim_outside(Some(DimOutside::new(5, 9, DIM)));
    let alphas = marker_alpha(&mut ts);
    assert!(all(&alphas, 1.0), "{alphas:?}");
    // The other paragraph.
    ts.flow.set_dim_outside(Some(DimOutside::new(10, 14, DIM)));
    let alphas = marker_alpha(&mut ts);
    assert!(all(&alphas, DIM), "{alphas:?}");

    let mut empty = make_typesetter();
    empty.layout_blocks(vec![list_item(1, 0, ""), make_block_at(2, 1, "Else")]);
    empty.flow.set_dim_outside(Some(DimOutside::new(0, 0, DIM)));
    let alphas = marker_alpha(&mut empty);
    assert!(all(&alphas, 1.0), "the caret's empty item: {alphas:?}");
}

/// Inside a table only the cell holding the range stays bright.
#[test]
fn in_a_table_only_the_cell_with_the_range_stays_bright() {
    let mut ts = make_typesetter();
    ts.add_table(&make_table(
        10,
        1,
        2,
        vec![
            make_cell_at(0, 0, 11, 0, "Left"),
            make_cell_at(0, 1, 12, 5, "Right"),
        ],
    ));
    ts.flow.set_dim_outside(Some(DimOutside::new(5, 10, DIM)));
    let frame = ts.render();
    assert_eq!(count(frame, 1.0), "Right".len());
    assert_eq!(count(frame, DIM), "Left".len());
}

// ── Incremental paths ───────────────────────────────────────────

/// Moving the caret repaints through `render_cursor_only`, which reuses the
/// cached quads. A range that moved to another paragraph must still show.
#[test]
fn cursor_only_render_follows_a_range_that_moved_to_another_block() {
    let mut ts = two_paragraphs();
    ts.flow.set_dim_outside(Some(DimOutside::new(0, 5, DIM)));
    ts.render();

    ts.flow.set_dim_outside(Some(DimOutside::new(6, 11, DIM)));
    let frame = glyphs(ts.render_cursor_only());
    assert!(all(&alphas_in_block(&ts, &frame, 1), DIM));
    assert!(all(&alphas_in_block(&ts, &frame, 2), 1.0));
}

/// The same, when the range moves between sentences of one paragraph: only
/// that block is redrawn, and the other keeps its (correct) cached quads.
#[test]
fn cursor_only_render_follows_a_range_that_moved_inside_a_block() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![
        make_block_at(1, 0, "One two. Three four."),
        make_block_at(2, 21, "Other"),
    ]);
    ts.flow.set_dim_outside(Some(DimOutside::new(0, 8, DIM)));
    let first = count(ts.render(), 1.0);
    assert_eq!(first, "Onetwo.".len());

    ts.flow.set_dim_outside(Some(DimOutside::new(9, 20, DIM)));
    let frame = ts.render_cursor_only();
    assert_eq!(count(frame, 1.0), "Threefour.".len());
    let frame = glyphs(frame);
    assert!(all(&alphas_in_block(&ts, &frame, 2), DIM));
}

#[test]
fn cursor_only_render_follows_turning_it_on_and_off_and_a_new_opacity() {
    let mut ts = two_paragraphs();
    ts.render();
    ts.flow.set_dim_outside(Some(DimOutside::new(6, 11, DIM)));
    assert_eq!(count(ts.render_cursor_only(), DIM), "Hello".len());

    ts.flow.set_dim_outside(Some(DimOutside::new(6, 11, 0.2)));
    assert_eq!(count(ts.render_cursor_only(), 0.2), "Hello".len());

    ts.flow.set_dim_outside(None);
    assert!(all(&alphas(ts.render_cursor_only()), 1.0));
}

/// Typing at the end of the bright paragraph: relayout one block, extend the
/// range by one character, render that block only. The other paragraph keeps
/// its cached dim quads, and the new character is bright.
#[test]
fn block_only_render_after_typing_in_the_bright_paragraph() {
    let mut ts = two_paragraphs();
    ts.flow.set_dim_outside(Some(DimOutside::new(0, 5, DIM)));
    ts.render();

    ts.relayout_block(&make_block_at(1, 0, "Hellos"));
    ts.flow.set_dim_outside(Some(DimOutside::new(0, 6, DIM)));
    let frame = ts.render_block_only(1);
    assert_eq!(count(frame, 1.0), "Hellos".len());
    assert_eq!(count(frame, DIM), "World".len());
}

/// Backspace: the old range ends one past the paragraph's new end, on its
/// separator. Still a one-block change, and still right.
#[test]
fn block_only_render_after_deleting_in_the_bright_paragraph() {
    let mut ts = two_paragraphs();
    ts.flow.set_dim_outside(Some(DimOutside::new(0, 5, DIM)));
    ts.render();

    ts.relayout_block(&make_block_at(1, 0, "Hell"));
    ts.flow.set_dim_outside(Some(DimOutside::new(0, 4, DIM)));
    let frame = ts.render_block_only(1);
    assert_eq!(count(frame, 1.0), "Hell".len());
    assert_eq!(count(frame, DIM), "World".len());
}

/// A block-only render whose range moved somewhere else must not leave the
/// other block's stale quads on screen.
#[test]
fn block_only_render_redraws_everything_when_the_range_left_the_block() {
    let mut ts = two_paragraphs();
    ts.flow.set_dim_outside(Some(DimOutside::new(0, 5, DIM)));
    ts.render();

    ts.flow.set_dim_outside(Some(DimOutside::new(6, 11, DIM)));
    let frame = glyphs(ts.render_block_only(1));
    assert!(all(&alphas_in_block(&ts, &frame, 1), DIM));
    assert!(all(&alphas_in_block(&ts, &frame, 2), 1.0));
}

/// The incremental paths must produce exactly what a full render would.
#[test]
fn incremental_and_full_renders_agree() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![
        make_block_at(1, 0, "One. Two."),
        make_block_at(2, 10, "Three. Four."),
    ]);
    let ranges = [
        Some(DimOutside::new(0, 4, DIM)),
        Some(DimOutside::new(5, 9, DIM)),
        Some(DimOutside::new(17, 22, DIM)),
        Some(DimOutside::new(17, 22, 0.1)),
        None,
        Some(DimOutside::new(9, 9, DIM)),
    ];
    ts.render();
    for range in ranges {
        ts.flow.set_dim_outside(range);
        let incremental = alphas(ts.render_cursor_only());
        let full = alphas(ts.render());
        assert_eq!(incremental, full, "range {range:?}");
    }
}

// ── Fading toward the page ──────────────────────────────────────

const PAGE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const INK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

fn mixed(factor: f32) -> [f32; 3] {
    let m = |ink: f32, page: f32| ink * factor + page * (1.0 - factor);
    [m(INK[0], PAGE[0]), m(INK[1], PAGE[1]), m(INK[2], PAGE[2])]
}

fn is_rgb(color: [f32; 4], rgb: [f32; 3]) -> bool {
    color[..3]
        .iter()
        .zip(rgb.iter())
        .all(|(a, b)| (a - b).abs() < 1e-4)
}

/// With a page colour the faded glyphs are mixed toward it and keep their
/// alpha: the same perceived fade whatever the renderer's blending space.
#[test]
fn toward_a_page_the_faded_text_is_mixed_and_keeps_its_alpha() {
    let mut ts = two_paragraphs();
    ts.set_text_color(INK);
    ts.flow
        .set_dim_outside(Some(DimOutside::new(6, 11, DIM).toward(PAGE)));
    let frame = ts.render();
    assert!(all(&alphas(frame), 1.0), "no glyph lost alpha");
    let faded = frame
        .glyphs
        .iter()
        .filter(|g| is_rgb(g.color, mixed(DIM)))
        .count();
    let bright = frame
        .glyphs
        .iter()
        .filter(|g| is_rgb(g.color, [0.0, 0.0, 0.0]))
        .count();
    assert_eq!(faded, "Hello".len());
    assert_eq!(bright, "World".len());
}

/// Strokes fade the same way as their glyphs.
#[test]
fn toward_a_page_an_underline_is_mixed_too() {
    let mut ts = underlined("Aaaa. Bbbb.", UnderlineStyle::Single);
    ts.set_text_color(INK);
    ts.flow
        .set_dim_outside(Some(DimOutside::new(6, 11, DIM).toward(PAGE)));
    let frame = ts.render();
    let pieces: Vec<[f32; 4]> = frame
        .decorations
        .iter()
        .filter(|d| d.kind == DecorationKind::Underline)
        .map(|d| d.color)
        .collect();
    assert_eq!(pieces.len(), 2);
    assert!(pieces.iter().all(|c| (c[3] - 1.0).abs() < 1e-4));
    assert!(pieces.iter().any(|c| is_rgb(*c, mixed(DIM))));
    assert!(pieces.iter().any(|c| is_rgb(*c, [0.0, 0.0, 0.0])));
}

/// An image cannot be mixed: it fades by its own opacity, page or not.
#[test]
fn toward_a_page_an_image_still_fades_by_opacity() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![image_block(1, 0), make_block_at(2, 2, "Text")]);
    ts.flow
        .set_dim_outside(Some(DimOutside::new(2, 6, DIM).toward(PAGE)));
    let frame = ts.render();
    assert_eq!(frame.images[0].opacity, DIM);
}

/// A theme switch moves the page colour and nothing else. The cursor-only
/// path must notice and redraw every faded glyph in the new mix.
#[test]
fn cursor_only_render_follows_a_new_page_colour() {
    let mut ts = two_paragraphs();
    ts.set_text_color(INK);
    ts.flow
        .set_dim_outside(Some(DimOutside::new(6, 11, DIM).toward(PAGE)));
    ts.render();

    let sepia = [0.9, 0.8, 0.6, 1.0];
    ts.flow
        .set_dim_outside(Some(DimOutside::new(6, 11, DIM).toward(sepia)));
    let want = [
        sepia[0] * (1.0 - DIM),
        sepia[1] * (1.0 - DIM),
        sepia[2] * (1.0 - DIM),
    ];
    let frame = ts.render_cursor_only();
    let faded = frame
        .glyphs
        .iter()
        .filter(|g| is_rgb(g.color, want))
        .count();
    assert_eq!(faded, "Hello".len(), "the faded text took the new page");
}

// ── Review regressions ──────────────────────────────────────────

/// **Undo moved the caret into an earlier paragraph and grew it.** The old
/// bright range's offsets now fall inside that paragraph, although they
/// belonged to the next one. Judged by offsets alone the change looked
/// confined to the grown paragraph, a block-only render redrew just it, and
/// the paragraph the range had left stayed bright.
#[test]
fn a_range_that_moved_into_a_grown_paragraph_redraws_the_one_it_left() {
    let mut ts = make_typesetter();
    // "The fox jumps." is 0..14, "Yes." is 15..19.
    ts.layout_blocks(vec![
        make_block_at(1, 0, "The fox jumps."),
        make_block_at(2, 15, "Yes."),
    ]);
    ts.flow.set_dim_outside(Some(DimOutside::new(15, 19, DIM)));
    ts.render();

    // Undo puts "quick " back into the first paragraph and the caret with it:
    // that paragraph is now 0..20, and the old range 15..19 sits inside it.
    ts.relayout_block(&make_block_at(1, 0, "The quick fox jumps."));
    ts.flow.set_dim_outside(Some(DimOutside::new(0, 20, DIM)));
    let frame = glyphs(ts.render_block_only(1));
    assert!(
        all(&alphas_in_block(&ts, &frame, 2), DIM),
        "the paragraph the range left must fade"
    );
    assert!(all(&alphas_in_block(&ts, &frame, 1), 1.0));
}

fn underlined_block(
    id: usize,
    position: usize,
    text: &str,
) -> text_typeset::layout::block::BlockLayoutParams {
    let mut block = make_block_at(id, position, text);
    block.fragments[0].underline_style = UnderlineStyle::Single;
    block
}

fn underline_count(frame: &RenderFrame) -> usize {
    frame
        .decorations
        .iter()
        .filter(|d| d.kind == DecorationKind::Underline)
        .count()
}

/// A caret move between sentences redraws one block and rebuilds the frame
/// from what was cached. The text decorations inside a blockquote frame were
/// never cached, so every underline, squiggle and highlight in a quote
/// vanished on the first sentence move.
#[test]
fn a_sentence_move_keeps_the_decorations_inside_a_frame() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block_at(1, 0, "One two. Three four.")]);
    ts.add_frame(&helpers::make_frame(
        50,
        vec![underlined_block(60, 21, "Quoted words")],
    ));
    ts.flow.set_dim_outside(Some(DimOutside::new(0, 8, DIM)));
    let full = underline_count(ts.render());
    assert!(full > 0, "the quote is underlined to begin with");

    ts.flow.set_dim_outside(Some(DimOutside::new(9, 20, DIM)));
    assert_eq!(
        underline_count(ts.render_cursor_only()),
        full,
        "the quote's underline is still drawn"
    );
}

/// The same, for the text decorations inside a table's cells.
#[test]
fn a_sentence_move_keeps_the_decorations_inside_a_table() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block_at(1, 0, "One two. Three four.")]);
    let mut cell = make_cell_at(0, 0, 61, 21, "Cell");
    cell.blocks[0].fragments[0].underline_style = UnderlineStyle::Single;
    ts.add_table(&make_table(70, 1, 1, vec![cell]));
    ts.flow.set_dim_outside(Some(DimOutside::new(0, 8, DIM)));
    let full = underline_count(ts.render());
    assert!(full > 0, "the cell is underlined to begin with");

    ts.flow.set_dim_outside(Some(DimOutside::new(9, 20, DIM)));
    assert_eq!(underline_count(ts.render_cursor_only()), full);
}

/// Blocks, tables and frames are numbered separately, so a paragraph and a
/// quote before it can share a number. A block-only render of the paragraph
/// must replace the paragraph's cached glyphs, not the quote's.
#[test]
fn a_block_only_render_never_replaces_a_frame_that_shares_its_number() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![]);
    ts.add_frame(&helpers::make_frame(2, vec![make_block_at(10, 0, "Quote")]));
    ts.flow
        .add_block(&ts.service, &make_block_at(2, 6, "Text"))
        .expect("appended after the quote");
    let before = ts.render().glyphs.len();
    assert_eq!(before, "Quote".len() + "Text".len());

    ts.relayout_block(&make_block_at(2, 6, "Texts"));
    let after = ts.render_block_only(2).glyphs.len();
    assert_eq!(
        after,
        "Quote".len() + "Texts".len(),
        "the quote is drawn once and the paragraph once, in its new form"
    );
}
