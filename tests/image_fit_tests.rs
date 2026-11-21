//! An inline image wider than its line's measure is laid out and painted at
//! that measure, its height scaled by the same factor.
//!
//! Uses the framework-agnostic API (no text-document dependency).

mod helpers;
use helpers::{NOTO_SANS, Typesetter, make_block, make_cell_at, make_table, make_typesetter};

use text_typeset::layout::block::{BlockLayoutParams, FragmentParams};
use text_typeset::layout::table::CellLayoutParams;
use text_typeset::{HitRegion, TextDirection};

const EPS: f32 = 0.5;

/// A fragment holding one image, at `offset` in its block's text.
fn image_fragment(offset: usize, name: &str, width: f32, height: f32) -> FragmentParams {
    let mut frag = make_block(0, "\u{FFFC}").fragments.remove(0);
    frag.offset = offset;
    frag.image_name = Some(name.to_string());
    frag.image_width = width;
    frag.image_height = height;
    frag
}

/// A block holding nothing but one image.
fn image_block(id: usize, width: f32, height: f32) -> BlockLayoutParams {
    let mut block = make_block(id, "\u{FFFC}");
    block.fragments = vec![image_fragment(0, "wide.png", width, height)];
    block
}

/// A block of `prefix` text followed by one image.
fn text_then_image(id: usize, prefix: &str, width: f32, height: f32) -> BlockLayoutParams {
    let text = format!("{prefix}\u{FFFC}");
    let mut block = make_block(id, &text);
    block.fragments[0].text = prefix.to_string();
    block.fragments[0].length = prefix.len();
    block
        .fragments
        .push(image_fragment(prefix.len(), "wide.png", width, height));
    block
}

fn typesetter_at(width: f32) -> Typesetter {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(width, 600.0);
    ts
}

/// The one image quad the frame painted: `[x, y, w, h]`.
fn only_image(ts: &mut Typesetter) -> [f32; 4] {
    let frame = ts.render();
    assert_eq!(frame.images.len(), 1, "expected exactly one image quad");
    frame.images[0].screen
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < EPS
}

#[test]
fn a_wide_image_is_capped_at_the_measure_with_its_proportions() {
    let mut ts = typesetter_at(400.0);
    ts.layout_blocks(vec![image_block(1, 2000.0, 1000.0)]);
    let [x, _, w, h] = only_image(&mut ts);
    assert!(close(x, 0.0), "starts at the left edge, got {x}");
    assert!(close(w, 400.0), "capped to the 400 px measure, got {w}");
    assert!(close(h, 200.0), "height scaled by the same 1/5, got {h}");
    assert!(
        ts.content_height() >= 200.0 && ts.content_height() < 260.0,
        "the line is as tall as the scaled image, not the stored one: {}",
        ts.content_height()
    );
    assert!(
        ts.max_content_width() <= 400.0 + EPS,
        "nothing reaches past the measure: {}",
        ts.max_content_width()
    );
}

#[test]
fn an_image_that_fits_keeps_its_own_size() {
    let mut ts = typesetter_at(400.0);
    ts.layout_blocks(vec![image_block(1, 300.0, 120.0)]);
    let [_, _, w, h] = only_image(&mut ts);
    assert!(
        close(w, 300.0) && close(h, 120.0),
        "never scaled up: {w}x{h}"
    );
}

#[test]
fn an_image_exactly_as_wide_as_the_measure_is_untouched() {
    let mut ts = typesetter_at(400.0);
    ts.layout_blocks(vec![image_block(1, 400.0, 100.0)]);
    let [_, _, w, h] = only_image(&mut ts);
    assert!(close(w, 400.0) && close(h, 100.0), "got {w}x{h}");
}

#[test]
fn margins_and_a_list_indent_narrow_the_measure() {
    let mut ts = typesetter_at(400.0);
    let mut block = image_block(1, 2000.0, 1000.0);
    block.left_margin = 40.0;
    block.right_margin = 20.0;
    block.list_indent = 40.0;
    block.list_marker = "1.".to_string();
    ts.layout_blocks(vec![block]);
    let [x, _, w, h] = only_image(&mut ts);
    assert!(close(w, 300.0), "400 - 40 - 20 - 40 = 300, got {w}");
    assert!(close(h, 150.0), "height follows the width, got {h}");
    assert!(
        close(x, 80.0),
        "set after the margin and list indent, got {x}"
    );
    assert!(x + w <= 400.0 - 20.0 + EPS, "stays inside the right margin");
}

#[test]
fn a_first_line_indent_is_room_the_image_does_not_take() {
    let mut ts = typesetter_at(400.0);
    let mut block = image_block(1, 2000.0, 1000.0);
    block.text_indent = 50.0;
    ts.layout_blocks(vec![block]);
    let [x, _, w, _] = only_image(&mut ts);
    assert!(
        close(w, 350.0),
        "the indented first line is 350 px, got {w}"
    );
    assert!(
        x + w <= 400.0 + EPS,
        "and the image ends inside it: {x} + {w}"
    );
}

#[test]
fn an_image_in_a_table_cell_is_capped_at_the_cell() {
    let mut ts = typesetter_at(400.0);
    let mut image_cell = CellLayoutParams {
        row: 0,
        column: 1,
        blocks: vec![image_block(11, 2000.0, 1000.0)],
        background_color: None,
    };
    image_cell.blocks[0].position = 2;
    let mut table = make_table(1, 1, 2, vec![make_cell_at(0, 0, 10, 0, "A"), image_cell]);
    table.column_widths = vec![200.0, 200.0];
    ts.add_table(&table);
    let [x, _, w, h] = only_image(&mut ts);
    assert!(w < 200.0, "narrower than its 200 px column, got {w}");
    assert!(close(h, w / 2.0), "proportions kept: {w}x{h}");
    assert!(x + w <= 400.0 + EPS, "inside the table: {x} + {w}");
}

#[test]
fn the_painted_quad_the_line_box_and_the_hit_test_agree() {
    let mut ts = typesetter_at(400.0);
    ts.layout_blocks(vec![image_block(1, 2000.0, 1000.0)]);
    let [x, y, w, h] = only_image(&mut ts);
    assert!(close(w, 400.0), "drawn at the measure, got {w}");
    let mid_y = y + h / 2.0;

    let inside = ts.hit_test(x + w - 1.0, mid_y).expect("a hit inside");
    assert!(
        matches!(inside.region, HitRegion::Image { .. }),
        "the last pixel of the drawn image is the image: {:?}",
        inside.region
    );
    // Past the drawn edge, where the stored 2000 px used to reach.
    let past = ts.hit_test(x + w + 1.0, mid_y);
    assert!(
        !matches!(past.map(|r| r.region), Some(HitRegion::Image { .. })),
        "a pixel past the drawn edge is not the image"
    );

    // Caret after the image sits at its drawn right edge.
    let caret = ts.caret_rect(1);
    assert!(
        close(caret[0], x + w),
        "caret after the image at {}",
        caret[0]
    );
}

#[test]
fn text_before_a_wide_image_sends_it_to_a_full_line_of_its_own() {
    let mut ts = typesetter_at(400.0);
    ts.layout_blocks(vec![text_then_image(1, "Look: ", 2000.0, 1000.0)]);
    let [x, _, w, h] = only_image(&mut ts);
    assert!(close(x, 0.0), "the image starts its own line, got x = {x}");
    assert!(close(w, 400.0), "capped to the whole measure, got {w}");
    assert!(
        close(h, 200.0),
        "not squashed by the text before it, got {h}"
    );
}

#[test]
fn a_narrower_then_wider_measure_does_not_compound() {
    let mut ts = typesetter_at(800.0);
    ts.layout_blocks(vec![image_block(1, 2000.0, 1000.0)]);
    let wide_first = only_image(&mut ts);

    ts.set_viewport(300.0, 600.0);
    ts.layout_blocks(vec![image_block(1, 2000.0, 1000.0)]);
    let narrow = only_image(&mut ts);
    assert!(close(narrow[2], 300.0) && close(narrow[3], 150.0));

    ts.set_viewport(800.0, 600.0);
    ts.layout_blocks(vec![image_block(1, 2000.0, 1000.0)]);
    let wide_again = only_image(&mut ts);
    assert!(
        close(wide_again[2], wide_first[2]) && close(wide_again[3], wide_first[3]),
        "back at 800 px it is {:?}, as at first {:?}",
        wide_again,
        wide_first
    );
    assert!(close(wide_again[2], 800.0) && close(wide_again[3], 400.0));
}

#[test]
fn a_relaid_block_is_capped_like_a_fresh_one() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![image_block(1, 2000.0, 1000.0)]);
    ts.render();
    ts.relayout_block(&image_block(1, 3000.0, 600.0));
    let [_, _, w, h] = only_image(&mut ts);
    assert!(close(w, 800.0) && close(h, 160.0), "got {w}x{h}");
}

#[test]
fn a_right_to_left_paragraph_caps_the_same_and_keeps_it_inside() {
    let mut ts = typesetter_at(400.0);
    let mut block = text_then_image(1, "שלום ", 2000.0, 1000.0);
    block.base_direction = TextDirection::RightToLeft;
    ts.layout_blocks(vec![block]);
    let [x, _, w, h] = only_image(&mut ts);
    assert!(
        close(w, 400.0) && close(h, 200.0),
        "same cap as LTR: {w}x{h}"
    );
    assert!(
        x >= -EPS && x + w <= 400.0 + EPS,
        "inside the frame: {x} + {w}"
    );
}

#[test]
fn an_unbounded_line_leaves_images_alone() {
    let mut ts = typesetter_at(400.0);
    let mut block = image_block(1, 2000.0, 1000.0);
    block.non_breakable_lines = true;
    ts.layout_blocks(vec![block]);
    let [_, _, w, h] = only_image(&mut ts);
    assert!(
        close(w, 2000.0) && close(h, 1000.0),
        "a line that never wraps has no measure to cap to: {w}x{h}"
    );
}

#[test]
fn a_degenerate_image_never_goes_below_one_pixel() {
    let mut ts = typesetter_at(400.0);
    ts.layout_blocks(vec![image_block(1, 100_000.0, 2.0)]);
    let [_, _, w, h] = only_image(&mut ts);
    assert!(close(w, 400.0), "capped, got {w}");
    assert!(h >= 1.0, "a hairline image keeps one pixel, got {h}");

    ts.layout_blocks(vec![image_block(1, 2000.0, 0.0)]);
    let [_, _, w, h] = only_image(&mut ts);
    assert!(
        close(w, 400.0),
        "a zero height does not stop the cap, got {w}"
    );
    assert!(
        h.is_finite() && h >= 0.0,
        "and stays a real number, got {h}"
    );
}

#[test]
fn several_images_on_one_line_are_each_capped_on_their_own() {
    let mut ts = typesetter_at(400.0);
    let text = "\u{FFFC}\u{FFFC}";
    let mut block = make_block(1, text);
    let obj = "\u{FFFC}".len();
    block.fragments = vec![
        image_fragment(0, "a.png", 100.0, 50.0),
        image_fragment(obj, "b.png", 1600.0, 800.0),
    ];
    ts.layout_blocks(vec![block]);
    let frame = ts.render();
    assert_eq!(frame.images.len(), 2);
    let small = frame.images[0].screen;
    let big = frame.images[1].screen;
    assert!(
        close(small[2], 100.0),
        "the small one is untouched: {small:?}"
    );
    assert!(close(big[2], 400.0) && close(big[3], 200.0), "got {big:?}");
    assert!(
        big[1] > small[1],
        "the capped one wraps below the small one: {small:?} {big:?}"
    );
}
