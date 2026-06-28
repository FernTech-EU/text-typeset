//! Logical `font_scale` (accessibility text magnification) invariants.
//!
//! Unlike `scale_factor` (HiDPI raster density — logical metrics are invariant),
//! `font_scale` multiplies the *logical* font size: advances, line heights, and
//! content height all grow, and text re-wraps. `1.0` is the identity.

mod helpers;

use helpers::{Typesetter, make_block, make_typesetter};
use text_typeset::layout::flow::FlowLayout;

const TEXT: &str = "Hello, world!";
const TEXT_LONG: &str =
    "The quick brown fox jumps over the lazy dog and keeps on running far away.";

/// Lay out a block through `FlowLayout` at a given `font_scale`.
fn flow_at_font_scale(ts: &Typesetter, fs: f32) -> FlowLayout {
    let mut flow = FlowLayout::new();
    flow.scale_factor = 1.0;
    flow.font_scale = fs;
    flow.add_block(ts.font_registry(), &make_block(1, TEXT), 800.0);
    flow
}

#[test]
fn font_scale_default_is_one() {
    let flow = FlowLayout::new();
    assert_eq!(flow.font_scale, 1.0);
}

#[test]
fn font_scale_one_is_identity() {
    let ts = make_typesetter();
    let a = flow_at_font_scale(&ts, 1.0);
    // A second flow at the same scale must match exactly.
    let b = flow_at_font_scale(&ts, 1.0);
    let ba = a.blocks.get(&1).unwrap();
    let bb = b.blocks.get(&1).unwrap();
    assert!((ba.height - bb.height).abs() < 0.001);
}

#[test]
fn font_scale_grows_logical_metrics() {
    let ts = make_typesetter();
    let f1 = flow_at_font_scale(&ts, 1.0);
    let f2 = flow_at_font_scale(&ts, 2.0);

    let b1 = f1.blocks.get(&1).unwrap();
    let b2 = f2.blocks.get(&1).unwrap();

    // Same single line of text, but every logical dimension ~doubles.
    assert_eq!(b1.lines.len(), b2.lines.len());
    let l1 = &b1.lines[0];
    let l2 = &b2.lines[0];
    assert!(
        (l2.width - l1.width * 2.0).abs() < l1.width * 0.05,
        "line width should ~double: {} vs {}",
        l1.width,
        l2.width
    );
    assert!(
        (l2.line_height - l1.line_height * 2.0).abs() < l1.line_height * 0.05,
        "line height should ~double: {} vs {}",
        l1.line_height,
        l2.line_height
    );
    assert!(
        (b2.height - b1.height * 2.0).abs() < b1.height * 0.05,
        "block height should ~double: {} vs {}",
        b1.height,
        b2.height
    );
}

#[test]
fn font_scale_reflows_wrapped_text() {
    // Larger text in the same width wraps onto more lines.
    let ts = make_typesetter();
    let mut narrow_1x = FlowLayout::new();
    narrow_1x.font_scale = 1.0;
    narrow_1x.add_block(ts.font_registry(), &make_block(1, TEXT_LONG), 300.0);

    let mut narrow_2x = FlowLayout::new();
    narrow_2x.font_scale = 2.0;
    narrow_2x.add_block(ts.font_registry(), &make_block(1, TEXT_LONG), 300.0);

    let lines_1x = narrow_1x.blocks.get(&1).unwrap().lines.len();
    let lines_2x = narrow_2x.blocks.get(&1).unwrap().lines.len();
    assert!(
        lines_2x > lines_1x,
        "2x text in the same width must wrap onto more lines: {lines_1x} vs {lines_2x}"
    );
}
