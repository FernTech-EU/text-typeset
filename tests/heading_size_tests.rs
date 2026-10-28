//! A heading's size is a ratio of the body size the host sets, not of a fixed
//! 16 px: with a 13 px or a 20 px body the headings keep the proportion they
//! have at 16 px, and at 16 px they are the sizes they always were.

mod helpers;

use helpers::{NOTO_SANS, Typesetter};
use text_document::TextDocument;
use text_typeset::bridge::{convert_block, convert_flow};
use text_typeset::layout::flow::FlowLayout;

/// One block per heading level, then a body paragraph.
const MARKDOWN: &str = "# One\n\n## Two\n\n### Three\n\n#### Four\n\nBody text.";

/// The size every block of `MARKDOWN` is laid out at, in block order, with
/// `body_px` as the default font size.
fn block_sizes(body_px: f32) -> Vec<f32> {
    let doc = TextDocument::new();
    doc.set_markdown(MARKDOWN).unwrap().wait().unwrap();
    let elements = convert_flow(&doc.snapshot_flow());

    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, body_px);

    let mut flow = FlowLayout::new();
    flow.scale_factor = 1.0;
    let mut sizes = Vec::new();
    for (_, params) in &elements.blocks {
        flow.add_block(ts.font_registry(), params, 800.0);
        let block = flow.blocks.get(&params.block_id).unwrap();
        sizes.push(block.lines[0].runs[0].shaped_run.size_px);
    }
    sizes
}

#[test]
fn headings_at_a_16_px_body_keep_their_sizes() {
    assert_eq!(block_sizes(16.0), vec![32.0, 24.0, 20.0, 17.0, 16.0]);
}

#[test]
fn headings_follow_a_13_px_body() {
    assert_eq!(block_sizes(13.0), vec![26.0, 19.0, 16.0, 14.0, 13.0]);
}

#[test]
fn headings_follow_a_20_px_body() {
    assert_eq!(block_sizes(20.0), vec![40.0, 30.0, 25.0, 22.0, 20.0]);
}

/// A heading whose run sets its own size is scaled from that size, whatever
/// the body size.
#[test]
fn a_heading_run_with_its_own_size_scales_from_it() {
    let doc = TextDocument::new();
    doc.set_markdown("# Small").unwrap().wait().unwrap();
    let snapshot = doc.snapshot_flow();
    let text_document::FlowElementSnapshot::Block(block) = &snapshot.elements[0] else {
        panic!("expected a block");
    };
    let mut params = convert_block(block);
    params.fragments[0].font_point_size = Some(10);

    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 13.0);
    let mut flow = FlowLayout::new();
    flow.scale_factor = 1.0;
    flow.add_block(ts.font_registry(), &params, 800.0);
    let size = flow.blocks.get(&params.block_id).unwrap().lines[0].runs[0]
        .shaped_run
        .size_px;
    assert_eq!(size, 20.0);
}
