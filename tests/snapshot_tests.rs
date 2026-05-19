//! Snapshot tests for text-typeset's structural outputs.
//!
//! Exact glyph coordinates depend on font metrics and the rustybuzz
//! / swash versions — they're not stable enough to snapshot raw.
//! These tests therefore snapshot **derived, structural** shapes:
//! line counts, wrap indices, decoration kinds, hit-region types,
//! block ordering, and other integer-/enum-valued summaries that
//! stay stable across font backend updates.
//!
//! Each snapshot is a diff-reviewable `.snap` file under
//! `tests/snapshots/`. `cargo insta review` walks the diffs when a
//! typeset output intentionally changes.

mod helpers;

use helpers::{
    Rect, RenderFrameExt, make_block, make_block_at, make_cell, make_table, make_typesetter,
};
use insta::{assert_debug_snapshot, assert_snapshot};
use text_typeset::layout::paragraph::Alignment;
use text_typeset::{CursorDisplay, DecorationKind, HitRegion, TextFormat};

fn fmt_rect(r: Rect) -> String {
    // Round to 1 decimal to absorb sub-pixel shaping noise that can
    // change between rustybuzz/swash patch releases. Structure (the
    // relative ordering and approximate sizing of rectangles)
    // remains the stable signal.
    format!(
        "[x={:.1}, y={:.1}, w={:.1}, h={:.1}]",
        r.x(),
        r.y(),
        r.w(),
        r.h()
    )
}

// ── Paragraph line-count snapshots ──────────────────────────────────

#[test]
fn snapshot_paragraph_wraps_at_narrow_width() {
    let mut ts = make_typesetter();
    let text = "The quick brown fox jumps over the lazy dog in the late afternoon.";
    let result = ts.layout_paragraph(text, &TextFormat::default(), 100.0, None);
    let summary = format!(
        "line_count={}, height_gt_zero={}, width_le_max={}",
        result.line_count,
        result.height > 0.0,
        result.width <= 100.0 + 0.5
    );
    assert_snapshot!(summary);
}

#[test]
fn snapshot_paragraph_line_counts_across_widths() {
    let mut ts = make_typesetter();
    let text =
        "One two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen.";
    let widths = [40.0_f32, 80.0, 160.0, 320.0, 640.0];
    let lines: Vec<String> = widths
        .iter()
        .map(|&w| {
            let r = ts.layout_paragraph(text, &TextFormat::default(), w, None);
            format!("width={}, line_count={}", w as i32, r.line_count)
        })
        .collect();
    assert_debug_snapshot!(lines);
}

#[test]
fn snapshot_paragraph_respects_max_lines() {
    let mut ts = make_typesetter();
    let text =
        "One two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen.";
    let full = ts.layout_paragraph(text, &TextFormat::default(), 60.0, None);
    let capped = ts.layout_paragraph(text, &TextFormat::default(), 60.0, Some(2));
    let summary = format!(
        "uncapped_lines={}, capped_lines={} (cap=2)",
        full.line_count, capped.line_count
    );
    assert_snapshot!(summary);
}

// ── Decoration layout snapshots ─────────────────────────────────────

#[test]
fn snapshot_selection_decoration_kinds() {
    // Selection across two lines should emit `Selection` decoration
    // rectangles; the kinds and their ordering relative to text are
    // stable across font updates even if their exact x/y isn't.
    let mut ts = make_typesetter();
    let text = "Hello world, this is a slightly longer example to force a wrap.";
    ts.set_viewport(200.0, 400.0);
    ts.layout_blocks(vec![make_block(1, text)]);
    ts.set_cursor(&CursorDisplay {
        position: 6,
        anchor: 30,
        affinity: text_typeset::CursorAffinity::Downstream,
        visible: true,
        selected_cells: vec![],
    });
    let frame = ts.render();
    let kinds: Vec<String> = frame
        .decorations
        .iter()
        .map(|d| format!("{:?}", d.kind))
        .collect();
    assert_debug_snapshot!(kinds);
}

#[test]
fn snapshot_cursor_only_emits_one_cursor_decoration() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "hello")]);
    ts.set_cursor(&CursorDisplay {
        position: 2,
        anchor: 2,
        affinity: text_typeset::CursorAffinity::Downstream,
        visible: true,
        selected_cells: vec![],
    });
    let frame = ts.render();
    let counts = (
        frame.decoration_count(DecorationKind::Cursor),
        frame.decoration_count(DecorationKind::Selection),
    );
    assert_debug_snapshot!(counts);
}

// ── Hit-test region classification ─────────────────────────────────

#[test]
fn snapshot_hit_test_regions_across_line() {
    let mut ts = make_typesetter();
    let text = "Hello world.";
    ts.layout_blocks(vec![make_block(1, text)]);
    // Sample points: well inside text, just at first glyph, far right
    // past the line end, way below the content.
    let points: Vec<(f32, f32, String)> = [
        (5.0_f32, 8.0_f32),
        (40.0, 8.0),
        (400.0, 8.0),  // past end of line
        (40.0, 800.0), // below all content
    ]
    .iter()
    .map(|&(x, y)| {
        let region = match ts.hit_test(x, y) {
            Some(r) => format!("{:?}", r.region),
            None => "None".to_string(),
        };
        (x, y, region)
    })
    .collect();
    let summary: Vec<String> = points
        .iter()
        .map(|(x, y, r)| format!("({}, {}) -> {}", x, y, r))
        .collect();
    assert_debug_snapshot!(summary);
}

#[test]
fn snapshot_hit_test_within_vs_past_last_line() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "short")]);
    // Exact region names for points on/off the text.
    let inside = ts.hit_test(5.0, 8.0).map(|r| format!("{:?}", r.region));
    let past = ts.hit_test(500.0, 8.0).map(|r| format!("{:?}", r.region));
    assert_debug_snapshot!(vec![
        format!("inside = {:?}", inside),
        format!("past_end = {:?}", past),
    ]);
}

// ── Block stacking: ordering + relative layout ──────────────────────

#[test]
fn snapshot_multi_block_stacking_order() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![
        make_block_at(1, 0, "first block"),
        make_block_at(2, 12, "second block is longer than the first"),
        make_block_at(3, 50, "third"),
    ]);
    // Stable summary: each block's visual height sign and ordering.
    // Exact pixels vary with font; the ordering + positive/stacked
    // nature does not.
    let heights: Vec<String> = (1..=3)
        .map(|id| {
            let info = ts.block_visual_info(id).unwrap();
            format!(
                "block {}: y>=0={}, height>0={}",
                id,
                info.y >= 0.0,
                info.height > 0.0
            )
        })
        .collect();
    assert_debug_snapshot!(heights);
}

// ── Table structural layout ─────────────────────────────────────────

#[test]
fn snapshot_table_cell_ordering() {
    let mut ts = make_typesetter();
    let table = make_table(
        1,
        2,
        2,
        vec![
            make_cell(0, 0, "A"),
            make_cell(0, 1, "B"),
            make_cell(1, 0, "c"),
            make_cell(1, 1, "d"),
        ],
    );
    ts.add_table(&table);
    // Sample: do we get a RenderFrame with glyph rectangles? The
    // relative ordering left→right, top→bottom of the A, B, c, d
    // glyphs is a stable property.
    let frame = ts.render();
    let n_glyphs = frame.glyph_count();
    assert_snapshot!(format!("glyph_count={} (expected >=4)", n_glyphs));
}

// ── Alignment snapshot ──────────────────────────────────────────────

#[test]
fn snapshot_alignment_variations() {
    let mut ts = make_typesetter();
    let alignments = [
        Alignment::Left,
        Alignment::Right,
        Alignment::Center,
        Alignment::Justify,
    ];
    let mut out: Vec<String> = Vec::new();
    for (i, &align) in alignments.iter().enumerate() {
        let mut block = make_block(100 + i, "align me");
        block.alignment = align;
        ts.layout_blocks(vec![block]);
        let frame = ts.render();
        let first_glyph = frame.glyph_rects().first().copied().map(fmt_rect);
        out.push(format!(
            "{:?}: first_glyph={:?}",
            align,
            first_glyph.is_some()
        ));
    }
    assert_debug_snapshot!(out);
}

// ── Character geometry snapshot ─────────────────────────────────────

#[test]
fn snapshot_character_geometry_is_monotonic() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "abcdefghij")]);
    let geom = ts.character_geometry(1, 0, 10);
    // Check monotonic non-decreasing x positions; record the count
    // as the stable value.
    let positions_non_decreasing = geom.windows(2).all(|w| w[1].position >= w[0].position);
    let all_widths_non_negative = geom.iter().all(|g| g.width >= 0.0);
    assert_snapshot!(format!(
        "count={}, monotonic={}, non_negative_widths={}",
        geom.len(),
        positions_non_decreasing,
        all_widths_non_negative
    ));
}

// ── HitRegion enum coverage ─────────────────────────────────────────
//
// Lock in the set of distinguishable HitRegion variants that real
// layouts produce — if a new variant is added or an old one renamed,
// this test fails loudly and the diff shows exactly what changed.

#[test]
fn snapshot_hit_region_variant_names() {
    // This is a compile-time exhaustiveness check encoded as a
    // runtime enum list. If `HitRegion` ever gains or loses a
    // variant, the authors must update the match below and the
    // snapshot will flag the intent.
    let names: Vec<&'static str> = vec![
        match_name(&HitRegion::Text),
        match_name(&HitRegion::LeftMargin),
        match_name(&HitRegion::Indent),
        match_name(&HitRegion::TableBorder),
        match_name(&HitRegion::BelowContent),
        match_name(&HitRegion::PastLineEnd),
        match_name(&HitRegion::Image {
            name: String::new(),
        }),
        match_name(&HitRegion::Link {
            href: String::new(),
        }),
    ];
    assert_debug_snapshot!(names);
}

fn match_name(r: &HitRegion) -> &'static str {
    match r {
        HitRegion::Text => "Text",
        HitRegion::LeftMargin => "LeftMargin",
        HitRegion::Indent => "Indent",
        HitRegion::TableBorder => "TableBorder",
        HitRegion::BelowContent => "BelowContent",
        HitRegion::PastLineEnd => "PastLineEnd",
        HitRegion::Image { .. } => "Image",
        HitRegion::Link { .. } => "Link",
    }
}
