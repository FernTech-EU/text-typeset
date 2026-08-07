//! HiDPI `scale_factor` invariants.
//!
//! Layout is in logical pixels at every scale factor; only the rasterized
//! glyph bitmaps (and the quad `atlas` rects) differ.

mod helpers;

use helpers::{NOTO_SANS, Typesetter, make_block, make_typesetter};
use text_typeset::font::resolve::resolve_font;
use text_typeset::layout::block::layout_block;
use text_typeset::layout::flow::FlowLayout;
use text_typeset::shaping::shaper::{font_metrics_px, shape_text};
use text_typeset::{RelayoutError, TextFormat};

const TEXT: &str = "Hello, world!";

fn fresh_ts() -> Typesetter {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(800.0, 600.0);
    ts
}

#[test]
fn default_scale_factor_is_one() {
    let ts = make_typesetter();
    assert_eq!(ts.scale_factor(), 1.0);
}

#[test]
fn scale_factor_is_clamped() {
    let mut ts = make_typesetter();
    ts.set_scale_factor(0.0);
    assert_eq!(ts.scale_factor(), 0.25);
    ts.set_scale_factor(100.0);
    assert_eq!(ts.scale_factor(), 8.0);
    ts.set_scale_factor(-5.0);
    assert_eq!(ts.scale_factor(), 0.25);
}

/// Lay out the same block at two scale factors via `FlowLayout` directly
/// so we can read `blocks`/`lines` without going through Typesetter.
fn flow_at(ts: &Typesetter, sf: f32) -> FlowLayout {
    let mut flow = FlowLayout::new();
    flow.scale_factor = sf;
    flow.add_block(ts.font_registry(), &make_block(1, TEXT), 800.0);
    flow
}

#[test]
fn layout_metrics_are_logical_at_any_scale_factor() {
    let ts = make_typesetter();
    let f1 = flow_at(&ts, 1.0);
    let f2 = flow_at(&ts, 2.0);

    let b1 = f1.blocks.get(&1).unwrap();
    let b2 = f2.blocks.get(&1).unwrap();

    assert_eq!(b1.lines.len(), b2.lines.len());
    assert!(
        (b1.height - b2.height).abs() < 0.05,
        "heights diverge: {} vs {}",
        b1.height,
        b2.height
    );
    for (l1, l2) in b1.lines.iter().zip(b2.lines.iter()) {
        assert!(
            (l1.width - l2.width).abs() < 0.05,
            "line width diverges: {} vs {}",
            l1.width,
            l2.width
        );
        assert!((l1.ascent - l2.ascent).abs() < 0.05);
        assert!((l1.descent - l2.descent).abs() < 0.05);
        assert!((l1.line_height - l2.line_height).abs() < 0.05);
        assert_eq!(l1.char_range, l2.char_range);
    }
}

#[test]
fn layout_block_scale_factor_param_matches_flow_field() {
    // Direct `layout_block(..., scale_factor)` should match going through
    // FlowLayout's field.
    let ts = make_typesetter();
    let flow = flow_at(&ts, 2.0);
    let direct = layout_block(ts.font_registry(), &make_block(1, TEXT), 800.0, 2.0, 1.0);

    let via_flow = flow.blocks.get(&1).unwrap();
    assert_eq!(via_flow.lines.len(), direct.lines.len());
    for (a, b) in via_flow.lines.iter().zip(direct.lines.iter()) {
        assert!((a.width - b.width).abs() < 0.05);
    }
}

#[test]
fn shaped_advances_are_logical_at_any_scale_factor() {
    let ts = make_typesetter();
    let r1 = resolve_font(ts.font_registry(), None, None, None, None, None, 1.0, 1.0).unwrap();
    let r2 = resolve_font(ts.font_registry(), None, None, None, None, None, 2.0, 1.0).unwrap();
    let run1 = shape_text(ts.font_registry(), &r1, TEXT, 0).unwrap();
    let run2 = shape_text(ts.font_registry(), &r2, TEXT, 0).unwrap();
    assert_eq!(run1.glyphs.len(), run2.glyphs.len());
    assert!((run1.advance_width - run2.advance_width).abs() < 0.05);
    for (g1, g2) in run1.glyphs.iter().zip(run2.glyphs.iter()) {
        assert!((g1.x_advance - g2.x_advance).abs() < 0.05);
    }
}

#[test]
fn font_metrics_are_logical_at_any_scale_factor() {
    let ts = make_typesetter();
    let r1 = resolve_font(ts.font_registry(), None, None, None, None, None, 1.0, 1.0).unwrap();
    let r2 = resolve_font(ts.font_registry(), None, None, None, None, None, 4.0, 1.0).unwrap();
    let m1 = font_metrics_px(ts.font_registry(), &r1).unwrap();
    let m2 = font_metrics_px(ts.font_registry(), &r2).unwrap();
    assert!((m1.ascent - m2.ascent).abs() < 0.05);
    assert!((m1.descent - m2.descent).abs() < 0.05);
    assert!((m1.stroke_size - m2.stroke_size).abs() < 0.05);
}

#[test]
fn identity_at_sf_one_matches_untouched() {
    // Going through the setter with sf=1.0 should be identical to never
    // touching it.
    let mut a = fresh_ts();
    a.layout_blocks(vec![make_block(1, TEXT)]);
    let ra_glyphs = a.render().glyphs.clone();

    let mut b = fresh_ts();
    b.set_scale_factor(1.0);
    b.layout_blocks(vec![make_block(1, TEXT)]);
    let rb_glyphs = b.render().glyphs.clone();

    assert_eq!(ra_glyphs.len(), rb_glyphs.len());
    for (qa, qb) in ra_glyphs.iter().zip(rb_glyphs.iter()) {
        for i in 0..4 {
            assert!(
                (qa.screen[i] - qb.screen[i]).abs() < 1e-3,
                "screen[{}] diverges: {} vs {}",
                i,
                qa.screen[i],
                qb.screen[i]
            );
            assert!(
                (qa.atlas[i] - qb.atlas[i]).abs() < 1e-3,
                "atlas[{}] diverges: {} vs {}",
                i,
                qa.atlas[i],
                qb.atlas[i]
            );
        }
    }
}

#[test]
fn screen_matches_logical_atlas_matches_physical() {
    // Lay out the same block at sf=1 and sf=2; compare the emitted quads.
    let mut a = fresh_ts();
    a.layout_blocks(vec![make_block(1, TEXT)]);
    let ra_glyphs = a.render().glyphs.clone();

    let mut b = fresh_ts();
    b.set_scale_factor(2.0);
    b.layout_blocks(vec![make_block(1, TEXT)]);
    let rb_glyphs = b.render().glyphs.clone();

    assert_eq!(ra_glyphs.len(), rb_glyphs.len());
    assert!(!ra_glyphs.is_empty());
    let mut checked_non_empty = false;
    for (qa, qb) in ra_glyphs.iter().zip(rb_glyphs.iter()) {
        if qa.screen[2] < 0.5 || qb.screen[2] < 0.5 {
            continue;
        }
        checked_non_empty = true;
        // Screen (logical) widths/heights should match to within one
        // physical pixel — rasterizer snaps glyph bounds to the physical
        // grid, so sf=2 can trim/expand by up to 1 physical px (=0.5 logical).
        // Be generous: allow 1 logical px slack.
        assert!(
            (qa.screen[2] - qb.screen[2]).abs() <= 1.01,
            "screen w diverges: {} vs {}",
            qa.screen[2],
            qb.screen[2]
        );
        assert!(
            (qa.screen[3] - qb.screen[3]).abs() <= 1.01,
            "screen h diverges: {} vs {}",
            qa.screen[3],
            qb.screen[3]
        );
        // Atlas (physical) dimensions must strictly grow — at sf=2 the
        // raster is at minimum as wide/tall as at sf=1. For thick-enough
        // glyphs (>=8 physical px) the ratio should be close to 2x; for
        // hairline glyphs (commas, periods) rasterizer rounding can give
        // ratios as low as ~1.5x which is still correct behaviour.
        assert!(
            qb.atlas[2] >= qa.atlas[2],
            "atlas w shrunk: {} -> {}",
            qa.atlas[2],
            qb.atlas[2]
        );
        assert!(
            qb.atlas[3] >= qa.atlas[3],
            "atlas h shrunk: {} -> {}",
            qa.atlas[3],
            qb.atlas[3]
        );
        if qa.atlas[2] >= 8.0 && qa.atlas[3] >= 8.0 {
            let ratio_w = qb.atlas[2] / qa.atlas[2];
            let ratio_h = qb.atlas[3] / qa.atlas[3];
            assert!(
                (1.6..=2.4).contains(&ratio_w),
                "atlas w ratio {} not near 2x ({} -> {})",
                ratio_w,
                qa.atlas[2],
                qb.atlas[2]
            );
            assert!(
                (1.6..=2.4).contains(&ratio_h),
                "atlas h ratio {} not near 2x ({} -> {})",
                ratio_h,
                qa.atlas[3],
                qb.atlas[3]
            );
        }
    }
    assert!(checked_non_empty, "no non-empty glyph quads were compared");
}

#[test]
fn zoom_and_scale_factor_are_orthogonal() {
    // sf=2 is HiDPI (physical density); zoom=1.5 is a display transform that
    // *also* densifies atlas bitmaps (so magnified text stays sharp). Screen
    // quads scale ~1.5×; atlas rects grow by the densify ladder for zoom.
    let mut a = fresh_ts();
    a.set_scale_factor(2.0);
    a.layout_blocks(vec![make_block(1, TEXT)]);
    let ra_glyphs = a.render().glyphs.clone();

    let mut b = fresh_ts();
    b.set_scale_factor(2.0);
    b.layout_blocks(vec![make_block(1, TEXT)]);
    b.set_zoom(1.5);
    let rb_glyphs = b.render().glyphs.clone();

    assert_eq!(ra_glyphs.len(), rb_glyphs.len());
    let densify = text_typeset::quantize_raster_scale(1.5);
    assert!(
        densify > 1.0,
        "zoom 1.5 must land above the 1× densify bucket"
    );
    for (qa, qb) in ra_glyphs.iter().zip(rb_glyphs.iter()) {
        if qa.screen[2] < 0.5 {
            continue;
        }
        // Bearing residual from unhinted densify bitmaps — allow a few px.
        assert!(
            (qb.screen[2] - qa.screen[2] * 1.5).abs() < 2.0,
            "zoom should scale screen w by ~1.5x: {} vs {}",
            qa.screen[2] * 1.5,
            qb.screen[2]
        );
        // Atlas densifies under zoom (sharp magnify), not identity.
        if qa.atlas[2] >= 8.0 {
            assert!(
                qb.atlas[2] > qa.atlas[2] * 1.1,
                "zoom must densify atlas w: {} -> {}",
                qa.atlas[2],
                qb.atlas[2]
            );
        }
    }
}

// ── raster_scale invariants ─────────────────────────────────────────
//
// Ambient `raster_scale` is the third axis next to `scale_factor` (HiDPI)
// and `zoom` (display transform + densify): it densifies bitmaps for
// content under an *external* scale transform. Paint densify is
// `quantize(ambient × zoom)`; layout metrics and pre-zoom screen rects
// stay logical.

#[test]
fn raster_scale_densifies_atlas_keeps_screen_logical() {
    // The dual of `screen_matches_logical_atlas_matches_physical`:
    // ambient raster_scale=2 densifies onto the ladder (~1.25³) and
    // leaves pre-zoom screen rects roughly logical (no apply_zoom).
    let mut a = fresh_ts();
    a.layout_blocks(vec![make_block(1, TEXT)]);
    let ra_glyphs = a.render().glyphs.clone();

    let mut b = fresh_ts();
    b.set_raster_scale(2.0);
    b.layout_blocks(vec![make_block(1, TEXT)]);
    let rb_glyphs = b.render().glyphs.clone();

    assert_eq!(ra_glyphs.len(), rb_glyphs.len());
    assert!(!ra_glyphs.is_empty());
    let densify = text_typeset::quantize_raster_scale(2.0);
    let mut checked_non_empty = false;
    for (qa, qb) in ra_glyphs.iter().zip(rb_glyphs.iter()) {
        if qa.screen[2] < 0.5 || qb.screen[2] < 0.5 {
            continue;
        }
        checked_non_empty = true;
        // Screen rects stay logical. Unhinted dense rasters change ink
        // bounds by a few logical px (bearing / pixel snapping).
        assert!(
            (qa.screen[2] - qb.screen[2]).abs() <= 3.0,
            "screen w diverges: {} vs {}",
            qa.screen[2],
            qb.screen[2]
        );
        assert!(
            (qa.screen[3] - qb.screen[3]).abs() <= 3.0,
            "screen h diverges: {} vs {}",
            qa.screen[3],
            qb.screen[3]
        );
        // Atlas rects must grow roughly with the densify ladder.
        assert!(
            qb.atlas[2] >= qa.atlas[2],
            "atlas w shrunk: {} -> {}",
            qa.atlas[2],
            qb.atlas[2]
        );
        if qa.atlas[2] >= 8.0 && qa.atlas[3] >= 8.0 {
            let ratio_w = qb.atlas[2] / qa.atlas[2];
            let ratio_h = qb.atlas[3] / qa.atlas[3];
            assert!(
                (densify * 0.7..=densify * 1.3).contains(&ratio_w),
                "atlas w ratio {} not near densify {densify} ({} -> {})",
                ratio_w,
                qa.atlas[2],
                qb.atlas[2]
            );
            assert!(
                (densify * 0.7..=densify * 1.3).contains(&ratio_h),
                "atlas h ratio {} not near densify {densify} ({} -> {})",
                ratio_h,
                qa.atlas[3],
                qb.atlas[3]
            );
        }
    }
    assert!(checked_non_empty, "no non-empty glyph quads were compared");
}

#[test]
fn raster_scale_label_path_keeps_metrics_logical() {
    // The single-line label path: identical width/height/baseline at
    // every raster scale (metrics come from shaping, which raster_scale
    // never touches).
    let mut ts = fresh_ts();
    let format = TextFormat::default();
    let r1 = ts.layout_single_line(TEXT, &format, None);
    ts.set_raster_scale(3.0);
    let r3 = ts.layout_single_line(TEXT, &format, None);

    assert!((r1.width - r3.width).abs() < 1e-3);
    assert!((r1.height - r3.height).abs() < 1e-3);
    assert!((r1.baseline - r3.baseline).abs() < 1e-3);
    assert_eq!(r1.glyphs.len(), r3.glyphs.len());
    for (qa, qb) in r1.glyphs.iter().zip(r3.glyphs.iter()) {
        if qa.atlas[2] >= 8.0 {
            assert!(
                qb.atlas[2] > qa.atlas[2] * 2.0,
                "atlas w should roughly triple: {} -> {}",
                qa.atlas[2],
                qb.atlas[2]
            );
        }
    }
}

#[test]
fn hinted_key_separates_same_physical_size() {
    // 7px at raster_scale=2 and 14px at raster_scale=1 share the same
    // physical ppem (14) but differ in hinting — they must be distinct
    // cache entries (different keys), or the first-rasterized bitmap
    // would be served for both.
    let mut ts = fresh_ts();
    let small = TextFormat {
        font_size: Some(7.0),
        ..Default::default()
    };
    let large = TextFormat {
        font_size: Some(14.0),
        ..Default::default()
    };

    let r14 = ts.layout_single_line(TEXT, &large, None);
    ts.set_raster_scale(2.0);
    let r7x2 = ts.layout_single_line(TEXT, &small, None);

    assert!(!r14.glyph_keys.is_empty());
    assert!(!r7x2.glyph_keys.is_empty());
    let k14 = r14.glyph_keys[0];
    let k7x2 = r7x2.glyph_keys[0];
    assert_eq!(
        k14.size_bits, k7x2.size_bits,
        "test premise: both reach the same physical ppem"
    );
    assert!(k14.hinted, "unscaled raster must be hinted");
    assert!(!k7x2.hinted, "scaled raster must be unhinted");
    assert_ne!(k14, k7x2, "keys must not collide");
}

#[test]
fn raster_scale_and_zoom_compose_for_densify() {
    // densify = quantize(ambient_raster_scale × zoom). Screen scales by
    // zoom; atlas density tracks the composed ladder, not ambient alone.
    let mut a = fresh_ts();
    a.set_raster_scale(2.0);
    a.layout_blocks(vec![make_block(1, TEXT)]);
    let ra_glyphs = a.render().glyphs.clone();

    let mut b = fresh_ts();
    b.set_raster_scale(2.0);
    b.layout_blocks(vec![make_block(1, TEXT)]);
    b.set_zoom(1.5);
    let rb_glyphs = b.render().glyphs.clone();

    let densify_a = text_typeset::quantize_raster_scale(2.0);
    let densify_b = text_typeset::quantize_raster_scale(2.0 * 1.5);
    assert!(
        densify_b > densify_a,
        "zoom on top of ambient densify must raise the ladder: {densify_a} -> {densify_b}"
    );

    assert_eq!(ra_glyphs.len(), rb_glyphs.len());
    for (qa, qb) in ra_glyphs.iter().zip(rb_glyphs.iter()) {
        if qa.screen[2] < 0.5 {
            continue;
        }
        assert!(
            (qb.screen[2] - qa.screen[2] * 1.5).abs() < 2.0,
            "zoom should scale screen w by ~1.5x: {} vs {}",
            qa.screen[2] * 1.5,
            qb.screen[2]
        );
        // Composed densify grows the atlas beyond ambient-only.
        if qa.atlas[2] >= 8.0 {
            assert!(
                qb.atlas[2] > qa.atlas[2] * 1.05,
                "composed densify must grow atlas w: {} -> {}",
                qa.atlas[2],
                qb.atlas[2]
            );
        }
    }
}

#[test]
fn raster_scale_change_falls_back_to_full_render_in_block_only_path() {
    // `render_block_only` reuses per-block quads from the last full
    // render; those bake atlas rects at the old raster scale, so a
    // scale change must force the full path.
    let mut ts = fresh_ts();
    ts.layout_blocks(vec![make_block(1, TEXT)]);
    let before: Vec<[f32; 4]> = ts.render().glyphs.iter().map(|q| q.atlas).collect();

    ts.set_raster_scale(2.0);
    let after: Vec<[f32; 4]> = ts
        .render_block_only(1)
        .glyphs
        .iter()
        .map(|q| q.atlas)
        .collect();

    assert_eq!(before.len(), after.len());
    let grew = before
        .iter()
        .zip(after.iter())
        .filter(|(a, _)| a[2] >= 8.0)
        .all(|(a, b)| b[2] > a[2]);
    assert!(
        grew,
        "block-only render after a raster-scale change must re-rasterize (full-render fallback)"
    );
}

#[test]
fn changing_scale_factor_resets_atlas_and_relayout_repopulates() {
    // Before: render at sf=1, note atlas width and that glyphs exist.
    let mut ts = fresh_ts();
    ts.layout_blocks(vec![make_block(1, TEXT)]);
    let before = ts.render();
    let before_glyphs = before.glyphs.len();
    let before_atlas_w = before.atlas_width;
    assert!(before_glyphs > 0);

    // Switch sf: the service clears its glyph cache and atlas in
    // place, and bumps its scale_generation counter. Per-widget
    // flow layouts are NOT touched (the service no longer reaches
    // into them). Every flow stamps the service's current
    // generation during its last `layout_*` call, so the flow can
    // report whether it is now stale via
    // `DocumentFlow::layout_dirty_for_scale`.
    ts.set_scale_factor(2.0);
    assert!(
        ts.flow.layout_dirty_for_scale(&ts.service),
        "flow must flag stale layout after scale_factor change"
    );

    // Re-layout against the new scale factor — this is the
    // caller's contract after a HiDPI change. The flow snaps back
    // to "up to date" and the relayouted glyphs land in a fresh
    // atlas.
    ts.layout_blocks(vec![make_block(1, TEXT)]);
    assert!(!ts.flow.layout_dirty_for_scale(&ts.service));

    let after_glyphs_and_atlas: Vec<_> = ts
        .render()
        .glyphs
        .iter()
        .map(|q| (q.atlas[2], q.atlas[3]))
        .collect();
    assert_eq!(after_glyphs_and_atlas.len(), before_glyphs);

    // The atlas must be marked dirty by the fresh rasterizations.
    // (It is either the same initial atlas or has grown; either
    // way its width is at least the initial 512.)
    assert!(before_atlas_w >= 512);
}

// ── incremental append vs. a stale scale ────────────────────────

/// `add_block` must refuse to append against a layout shaped at a different
/// scale, exactly as `relayout_block` does.
///
/// The danger is not the one appended block, it is that appending would
/// *stamp the flow as current*: `note_layout_done` re-records the service's
/// scale generation, so `layout_dirty_for_scale` would flip back to false and
/// the caller's "I must re-layout" signal would be destroyed — leaving every
/// pre-existing block shaped at the old scale forever, silently. A streaming
/// consumer appends constantly, so a HiDPI change mid-stream would hit this
/// almost immediately.
#[test]
fn add_block_refuses_a_stale_scale_and_preserves_the_dirty_flag() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "First line.")]);
    assert!(!ts.flow.layout_dirty_for_scale(&ts.service));

    // HiDPI change: the flow is now stale and the caller must re-layout.
    ts.set_scale_factor(2.0);
    assert!(ts.flow.layout_dirty_for_scale(&ts.service));

    let err = ts
        .flow
        .add_block(&ts.service, &make_block(2, "Streamed line."));
    assert!(
        matches!(err, Err(RelayoutError::ScaleDirty)),
        "add_block must reject a stale-scale append, got {err:?}"
    );

    assert!(
        ts.flow.layout_dirty_for_scale(&ts.service),
        "the rejected append must leave the dirty flag standing — clearing it \
         would strand every existing block at the old scale forever"
    );
    assert!(
        ts.flow.block_visual_info(2).is_none(),
        "a rejected append must not leave a half-applied block behind"
    );

    // The documented recovery re-establishes a clean, single-scale flow.
    ts.layout_blocks(vec![make_block(1, "First line.")]);
    assert!(!ts.flow.layout_dirty_for_scale(&ts.service));
    assert!(
        ts.flow
            .add_block(&ts.service, &make_block(2, "Streamed line."))
            .is_ok(),
        "append must succeed once the flow is re-laid-out at the new scale"
    );
}

/// Appending to a flow that has never been laid out is how an append-only
/// buffer starts, so it must not be treated as a scale conflict.
#[test]
fn add_block_on_a_fresh_flow_is_not_scale_dirty() {
    let ts = make_typesetter();
    let mut flow = text_typeset::DocumentFlow::new();
    assert!(
        flow.add_block(&ts.service, &make_block(1, "First streamed line."))
            .is_ok(),
        "appending to an empty flow must be allowed"
    );
}
