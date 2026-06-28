mod helpers;
use helpers::{NOTO_SANS, Typesetter, make_typesetter};

use text_typeset::font::resolve::resolve_font;
use text_typeset::shaping::shaper::{font_metrics_px, shape_text, shape_text_with_buffer};

#[test]
fn shape_hello_produces_glyphs() {
    let ts = make_typesetter();
    let resolved =
        resolve_font(ts.font_registry(), None, None, None, None, None, 1.0, 1.0).unwrap();
    let run = shape_text(ts.font_registry(), &resolved, "Hello", 0).unwrap();

    // "Hello" has 5 characters and should produce at least 5 glyphs
    // (could be fewer with ligatures, but Noto Sans doesn't ligate H-e-l-l-o)
    assert_eq!(run.glyphs.len(), 5);
    assert!(run.advance_width > 0.0);
}

#[test]
fn shape_empty_string_produces_no_glyphs() {
    let ts = make_typesetter();
    let resolved =
        resolve_font(ts.font_registry(), None, None, None, None, None, 1.0, 1.0).unwrap();
    let run = shape_text(ts.font_registry(), &resolved, "", 0).unwrap();
    assert_eq!(run.glyphs.len(), 0);
    assert!((run.advance_width - 0.0).abs() < f32::EPSILON);
}

#[test]
fn glyph_advances_are_positive() {
    let ts = make_typesetter();
    let resolved =
        resolve_font(ts.font_registry(), None, None, None, None, None, 1.0, 1.0).unwrap();
    let run = shape_text(ts.font_registry(), &resolved, "Test text", 0).unwrap();

    for glyph in &run.glyphs {
        // Space may have zero y_advance, but x_advance should be positive
        // (for LTR text, all glyphs advance right)
        assert!(
            glyph.x_advance >= 0.0,
            "glyph_id {} has negative x_advance: {}",
            glyph.glyph_id,
            glyph.x_advance
        );
    }
}

#[test]
fn cluster_values_map_to_byte_offsets() {
    let ts = make_typesetter();
    let resolved =
        resolve_font(ts.font_registry(), None, None, None, None, None, 1.0, 1.0).unwrap();
    let text = "AB";
    let run = shape_text(ts.font_registry(), &resolved, text, 0).unwrap();

    assert_eq!(run.glyphs.len(), 2);
    // First glyph's cluster should be byte offset 0 (for 'A')
    assert_eq!(run.glyphs[0].cluster, 0);
    // Second glyph's cluster should be byte offset 1 (for 'B')
    assert_eq!(run.glyphs[1].cluster, 1);
}

#[test]
fn multibyte_utf8_clusters_are_correct() {
    let ts = make_typesetter();
    let resolved =
        resolve_font(ts.font_registry(), None, None, None, None, None, 1.0, 1.0).unwrap();
    // 'é' is 2 bytes in UTF-8 (0xC3, 0xA9)
    let text = "Aé";
    let run = shape_text(ts.font_registry(), &resolved, text, 0).unwrap();

    assert!(run.glyphs.len() >= 2);
    assert_eq!(run.glyphs[0].cluster, 0); // 'A' at byte 0
    assert_eq!(run.glyphs[1].cluster, 1); // 'é' at byte 1
}

#[test]
fn text_offset_is_stored_in_run() {
    let ts = make_typesetter();
    let resolved =
        resolve_font(ts.font_registry(), None, None, None, None, None, 1.0, 1.0).unwrap();
    let run = shape_text(ts.font_registry(), &resolved, "Hi", 42).unwrap();
    assert_eq!(run.text_range, 42..44);
}

#[test]
fn larger_font_size_produces_larger_advances() {
    let ts = make_typesetter();
    let small = resolve_font(
        ts.font_registry(),
        None,
        None,
        None,
        None,
        Some(12),
        1.0,
        1.0,
    )
    .unwrap();
    let large = resolve_font(
        ts.font_registry(),
        None,
        None,
        None,
        None,
        Some(48),
        1.0,
        1.0,
    )
    .unwrap();

    let run_small = shape_text(ts.font_registry(), &small, "W", 0).unwrap();
    let run_large = shape_text(ts.font_registry(), &large, "W", 0).unwrap();

    assert!(
        run_large.advance_width > run_small.advance_width,
        "48px advance ({}) should be greater than 12px advance ({})",
        run_large.advance_width,
        run_small.advance_width
    );
}

#[test]
fn space_has_nonzero_advance() {
    let ts = make_typesetter();
    let resolved =
        resolve_font(ts.font_registry(), None, None, None, None, None, 1.0, 1.0).unwrap();
    let run = shape_text(ts.font_registry(), &resolved, " ", 0).unwrap();

    assert_eq!(run.glyphs.len(), 1);
    assert!(
        run.glyphs[0].x_advance > 0.0,
        "space advance should be positive"
    );
}

#[test]
fn font_metrics_are_reasonable() {
    let ts = make_typesetter();
    let resolved =
        resolve_font(ts.font_registry(), None, None, None, None, None, 1.0, 1.0).unwrap();
    let metrics = font_metrics_px(ts.font_registry(), &resolved).unwrap();

    // Ascent should be positive (above baseline)
    assert!(metrics.ascent > 0.0, "ascent should be positive");
    // Descent should be positive in swash (distance below baseline)
    assert!(metrics.descent > 0.0, "descent should be positive");
    // Line height (ascent + descent + leading) should be reasonable for 16px
    let line_height = metrics.ascent + metrics.descent + metrics.leading;
    assert!(
        line_height > 10.0 && line_height < 40.0,
        "line height {} is out of reasonable range for 16px",
        line_height
    );
}

#[test]
fn total_advance_matches_sum_of_glyphs() {
    let ts = make_typesetter();
    let resolved =
        resolve_font(ts.font_registry(), None, None, None, None, None, 1.0, 1.0).unwrap();
    let run = shape_text(ts.font_registry(), &resolved, "Hello world", 0).unwrap();

    let sum: f32 = run.glyphs.iter().map(|g| g.x_advance).sum();
    assert!(
        (run.advance_width - sum).abs() < 0.01,
        "total advance {} should match glyph sum {}",
        run.advance_width,
        sum
    );
}

#[test]
fn no_notdef_glyphs_for_basic_latin() {
    let ts = make_typesetter();
    let resolved =
        resolve_font(ts.font_registry(), None, None, None, None, None, 1.0, 1.0).unwrap();
    let run = shape_text(
        ts.font_registry(),
        &resolved,
        "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789",
        0,
    )
    .unwrap();

    for glyph in &run.glyphs {
        assert_ne!(
            glyph.glyph_id, 0,
            ".notdef glyph found — font missing a basic Latin character"
        );
    }
}

#[test]
fn shape_with_buffer_recycling() {
    let ts = make_typesetter();
    let resolved =
        resolve_font(ts.font_registry(), None, None, None, None, None, 1.0, 1.0).unwrap();

    let buffer = harfrust::UnicodeBuffer::new();
    let (run1, buffer) =
        shape_text_with_buffer(ts.font_registry(), &resolved, "Hello", 0, buffer, &[]).unwrap();
    assert_eq!(run1.glyphs.len(), 5);

    // Reuse the recycled buffer for a second shaping call
    let (run2, _buffer) =
        shape_text_with_buffer(ts.font_registry(), &resolved, "World", 0, buffer, &[]).unwrap();
    assert_eq!(run2.glyphs.len(), 5);

    // Both runs should have identical advance structure (same font, same length)
    assert!(run1.advance_width > 0.0);
    assert!(run2.advance_width > 0.0);
}

// ── BiDi tests ──────────────────────────────────────────────────

#[test]
fn bidi_pure_ltr_produces_single_run() {
    use text_typeset::shaping::shaper::bidi_runs;

    let runs = bidi_runs("Hello world");
    assert_eq!(runs.len(), 1);
    assert_eq!(
        runs[0].direction,
        text_typeset::shaping::shaper::TextDirection::LeftToRight
    );
}

#[test]
fn bidi_empty_text_produces_no_runs() {
    use text_typeset::shaping::shaper::bidi_runs;

    // Empty input has nothing to shape; the layout path early-exits on
    // `text.is_empty()`, so an empty slice of runs is the honest answer.
    let runs = bidi_runs("");
    assert!(runs.is_empty());
}

#[test]
fn bidi_mixed_produces_multiple_runs() {
    use text_typeset::shaping::shaper::bidi_runs;

    // Hebrew text mixed with English
    let runs = bidi_runs("Hello שלום world");
    assert!(
        runs.len() >= 2,
        "mixed LTR+RTL text should produce at least 2 bidi runs, got {}",
        runs.len()
    );

    // At least one run should be RTL
    let has_rtl = runs
        .iter()
        .any(|r| r.direction == text_typeset::shaping::shaper::TextDirection::RightToLeft);
    assert!(has_rtl, "mixed text should contain an RTL run");
}

#[test]
fn shape_rtl_text_produces_glyphs() {
    use text_typeset::shaping::shaper::{TextDirection, shape_text_directed};

    let ts = make_typesetter();
    let resolved =
        resolve_font(ts.font_registry(), None, None, None, None, None, 1.0, 1.0).unwrap();

    let run = shape_text_directed(
        ts.font_registry(),
        &resolved,
        "שלום",
        0,
        TextDirection::RightToLeft,
        &[],
    );
    assert!(run.is_some(), "shaping RTL text should succeed");
    let run = run.unwrap();
    assert!(!run.glyphs.is_empty(), "RTL text should produce glyphs");
    assert!(run.advance_width > 0.0);
}

#[test]
fn cached_shaper_data_yields_identical_glyphs() {
    // ShaperData is built once per face and cached on the FontEntry.
    // Re-shaping the same face must produce byte-identical glyph output
    // (the cache is a pure speedup, not a behavior change).
    let ts = make_typesetter();
    let resolved =
        resolve_font(ts.font_registry(), None, None, None, None, None, 1.0, 1.0).unwrap();

    let first = shape_text(ts.font_registry(), &resolved, "Cached shaping", 0).unwrap();
    // Second call hits the cached ShaperData on the same FontEntry.
    let second = shape_text(ts.font_registry(), &resolved, "Cached shaping", 0).unwrap();

    assert_eq!(first.glyphs.len(), second.glyphs.len());
    for (a, b) in first.glyphs.iter().zip(second.glyphs.iter()) {
        assert_eq!(a.glyph_id, b.glyph_id);
        assert_eq!(a.cluster, b.cluster);
        assert!((a.x_advance - b.x_advance).abs() < f32::EPSILON);
    }
    assert!((first.advance_width - second.advance_width).abs() < f32::EPSILON);
}

// ── OpenType feature tests ──────────────────────────────────────

#[test]
fn opentype_features_reach_the_shaper() {
    use text_typeset::FontFeature;
    use text_typeset::shaping::shaper::{TextDirection, shape_text_directed, to_harfrust_features};

    let ts = make_typesetter();
    let resolved =
        resolve_font(ts.font_registry(), None, None, None, None, None, 1.0, 1.0).unwrap();

    // Standard ligatures off vs on. NotoSans ligates "fi"; with `liga`
    // disabled it stays two glyphs, with it enabled it collapses to one.
    let off = to_harfrust_features(&[FontFeature::off(*b"liga")]);
    let on = to_harfrust_features(&[FontFeature::on(*b"liga")]);

    let run_off = shape_text_directed(
        ts.font_registry(),
        &resolved,
        "fi",
        0,
        TextDirection::LeftToRight,
        &off,
    )
    .unwrap();
    let run_on = shape_text_directed(
        ts.font_registry(),
        &resolved,
        "fi",
        0,
        TextDirection::LeftToRight,
        &on,
    )
    .unwrap();

    assert_eq!(
        run_off.glyphs.len(),
        2,
        "liga=0 should keep 'fi' as two glyphs"
    );
    assert_eq!(
        run_on.glyphs.len(),
        1,
        "liga=1 should ligate 'fi' into one glyph"
    );
}

// ── Glyph fallback tests ────────────────────────────────────────

#[test]
fn shape_text_with_fallback_no_notdef_for_basic_latin() {
    // With one font that covers Latin, shape_text should produce no .notdef
    let ts = make_typesetter();
    let resolved =
        resolve_font(ts.font_registry(), None, None, None, None, None, 1.0, 1.0).unwrap();
    let run = shape_text(ts.font_registry(), &resolved, "Hello", 0).unwrap();
    assert!(
        run.glyphs.iter().all(|g| g.glyph_id != 0),
        "Latin text with Noto Sans should have no .notdef glyphs"
    );
}

#[test]
fn shape_text_fallback_is_attempted_for_missing_glyphs() {
    // Register two copies of the same font. Shape a character that exists
    // in the font — should work without fallback. The test verifies the
    // fallback code path doesn't break normal operation.
    let mut ts = Typesetter::new();
    let face1 = ts.register_font(NOTO_SANS);
    let _face2 = ts.register_font(NOTO_SANS); // second registration for fallback pool
    ts.set_default_font(face1, 16.0);

    let resolved =
        resolve_font(ts.font_registry(), None, None, None, None, None, 1.0, 1.0).unwrap();
    let run = shape_text(ts.font_registry(), &resolved, "Test", 0).unwrap();

    assert_eq!(run.glyphs.len(), 4);
    assert!(run.advance_width > 0.0);
    // All glyphs should be resolved (no .notdef)
    assert!(
        run.glyphs.iter().all(|g| g.glyph_id != 0),
        "all glyphs should be resolved"
    );
}

#[test]
fn shape_text_advance_recomputed_after_fallback() {
    // Even if fallback is attempted (even if no .notdef present),
    // advance_width should match the sum of glyph advances.
    let ts = make_typesetter();
    let resolved =
        resolve_font(ts.font_registry(), None, None, None, None, None, 1.0, 1.0).unwrap();
    let run = shape_text(ts.font_registry(), &resolved, "Hello world", 0).unwrap();

    let sum: f32 = run.glyphs.iter().map(|g| g.x_advance).sum();
    assert!(
        (run.advance_width - sum).abs() < 0.01,
        "advance_width ({}) should match glyph sum ({})",
        run.advance_width,
        sum
    );
}
