mod helpers;
use helpers::Typesetter;

const NOTO_SANS: &[u8] = include_bytes!("../test-fonts/NotoSans-Variable.ttf");

#[test]
fn register_font_returns_valid_id() {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    // FontFaceId should be the first registered (index 0)
    assert_eq!(face, text_typeset::FontFaceId(0));
}

#[test]
fn register_multiple_fonts_returns_distinct_ids() {
    let mut ts = Typesetter::new();
    let face1 = ts.register_font(NOTO_SANS);
    let face2 = ts.register_font(NOTO_SANS); // same data, different registration
    assert_ne!(face1, face2);
}

#[test]
fn set_default_font_and_query() {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);
    // Default font should be queryable through the registry
    assert_eq!(ts.font_registry().default_font(), Some(face));
    assert!((ts.font_registry().default_size_px() - 16.0).abs() < f32::EPSILON);
}

#[test]
fn register_font_as_with_explicit_metadata() {
    let mut ts = Typesetter::new();
    let face = ts.register_font_as(NOTO_SANS, "CustomFamily", 400, false);
    // Should be queryable by the custom family name
    let found = ts.font_registry().query_font("CustomFamily", 400, false);
    assert_eq!(found, Some(face));
}

#[test]
fn query_by_family_name() {
    let mut ts = Typesetter::new();
    let _face = ts.register_font(NOTO_SANS);
    // Noto Sans variable font registers as "Noto Sans"
    let found = ts.font_registry().query_font("Noto Sans", 400, false);
    assert!(found.is_some());
}

#[test]
fn query_nonexistent_family_returns_none() {
    let mut ts = Typesetter::new();
    let _face = ts.register_font(NOTO_SANS);
    let found = ts
        .font_registry()
        .query_font("Nonexistent Font", 400, false);
    assert!(found.is_none());
}

#[test]
fn generic_family_mapping() {
    let mut ts = Typesetter::new();
    let _face = ts.register_font(NOTO_SANS);
    ts.set_generic_family("sans-serif", "Noto Sans");
    // Query via generic name should resolve through the mapping
    let found = ts.font_registry().query_font("sans-serif", 400, false);
    assert!(found.is_some());
}

#[test]
fn font_entry_has_valid_data() {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    let entry = ts.font_registry().get(face);
    assert!(entry.is_some());
    let entry = entry.unwrap();
    // Font data should be non-empty
    assert!(!entry.bytes().is_empty());
    // Should be parseable as a font
    let font_ref = swash::FontRef::from_index(entry.bytes(), entry.face_index as usize);
    assert!(font_ref.is_some());
}

#[test]
fn font_resolve_with_default_fallback() {
    use text_typeset::font::resolve::resolve_font;

    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 14.0);

    // Resolve with no family specified — should fall back to default
    let resolved = resolve_font(ts.font_registry(), None, None, None, None, None, 1.0, 1.0);
    assert!(resolved.is_some());
    let resolved = resolved.unwrap();
    assert_eq!(resolved.font_face_id, face);
    assert!((resolved.size_px - 14.0).abs() < f32::EPSILON);
}

#[test]
fn font_resolve_with_explicit_family() {
    use text_typeset::font::resolve::resolve_font;

    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);

    let resolved = resolve_font(
        ts.font_registry(),
        Some("Noto Sans"),
        None,
        None,
        None,
        Some(24),
        1.0,
        1.0,
    );
    assert!(resolved.is_some());
    let resolved = resolved.unwrap();
    assert!((resolved.size_px - 24.0).abs() < f32::EPSILON);
}

#[test]
fn font_resolve_bold_uses_weight_700() {
    use text_typeset::font::resolve::resolve_font;

    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);

    // font_bold=true should resolve (variable font supports weight axis)
    let resolved = resolve_font(
        ts.font_registry(),
        Some("Noto Sans"),
        None,
        Some(true),
        None,
        None,
        1.0,
        1.0,
    );
    assert!(resolved.is_some());
}

#[test]
fn glyph_coverage_check() {
    use text_typeset::font::resolve::font_has_glyph;

    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    // Noto Sans should have Latin characters
    assert!(font_has_glyph(ts.font_registry(), face, 'A'));
    assert!(font_has_glyph(ts.font_registry(), face, 'z'));
    // Unlikely to have CJK in the basic Noto Sans
    // (CJK is in separate Noto Sans CJK fonts)
}

#[test]
fn resolve_with_no_default_font_returns_none() {
    use text_typeset::font::resolve::resolve_font;

    let ts = Typesetter::new(); // no fonts registered, no default
    let resolved = resolve_font(ts.font_registry(), None, None, None, None, None, 1.0, 1.0);
    assert!(resolved.is_none());
}

#[test]
fn font_weight_takes_priority_over_font_bold() {
    use text_typeset::font::resolve::resolve_font;

    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);

    // font_weight=300 should override font_bold=true (which implies 700)
    let resolved = resolve_font(
        ts.font_registry(),
        Some("Noto Sans"),
        Some(300),
        Some(true),
        None,
        None,
        1.0,
        1.0,
    );
    assert!(resolved.is_some());
    // Can't check the weight directly (variable font returns same face),
    // but this exercises the priority logic in resolve_weight
}

#[test]
fn register_font_as_overrides_family_name() {
    let mut ts = Typesetter::new();
    let _face = ts.register_font_as(NOTO_SANS, "MyCustomName", 400, false);

    // Should NOT be findable under original family name
    let original = ts.font_registry().query_font("Noto Sans", 400, false);
    assert!(
        original.is_none(),
        "original family name should not match after override"
    );

    // Should be findable under custom name
    let custom = ts.font_registry().query_font("MyCustomName", 400, false);
    assert!(custom.is_some());
}

// ── Coverage: resolve edge cases ────────────────────────────────

#[test]
fn find_fallback_font_returns_none_with_single_font() {
    use text_typeset::font::resolve::find_fallback_font;

    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);

    // Only one font registered — fallback excludes it, so returns None
    let fallback = find_fallback_font(ts.font_registry(), '?', face);
    assert!(
        fallback.is_none(),
        "no fallback when only one font and it's excluded"
    );
}

#[test]
fn find_fallback_font_finds_second_font() {
    use text_typeset::font::resolve::find_fallback_font;

    let mut ts = Typesetter::new();
    let face1 = ts.register_font(NOTO_SANS);
    let face2 = ts.register_font(NOTO_SANS); // same data, different registration

    // Excluding face1, should find face2 as fallback for 'A'
    let fallback = find_fallback_font(ts.font_registry(), 'A', face1);
    assert_eq!(fallback, Some(face2));
}

#[test]
fn font_has_glyph_with_invalid_face_id() {
    use text_typeset::font::resolve::font_has_glyph;

    let ts = Typesetter::new();
    // Invalid face ID — no fonts registered
    assert!(!font_has_glyph(
        ts.font_registry(),
        text_typeset::FontFaceId(999),
        'A'
    ));
}

#[test]
fn resolve_font_with_nonexistent_family_falls_back() {
    use text_typeset::font::resolve::resolve_font;

    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);

    // Request a family that doesn't exist — should fall back to default
    let resolved = resolve_font(
        ts.font_registry(),
        Some("NonExistentFont"),
        None,
        None,
        None,
        Some(20),
        1.0,
        1.0,
    );
    assert!(resolved.is_some());
    let resolved = resolved.unwrap();
    assert_eq!(
        resolved.font_face_id, face,
        "should fall back to default font"
    );
    assert!(
        (resolved.size_px - 20.0).abs() < 0.01,
        "should respect requested size"
    );
}

// ── Line-height queries ─────────────────────────────────────────

#[test]
fn default_line_height_zero_when_no_font_registered() {
    let ts = Typesetter::new();
    assert_eq!(ts.service.default_line_height(), 0.0);
}

#[test]
fn default_line_height_matches_manual_metrics() {
    use text_typeset::font::resolve::resolve_font;
    use text_typeset::shaping::shaper::font_metrics_px;

    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 14.0);

    let resolved =
        resolve_font(ts.font_registry(), None, None, None, None, None, 1.0, 1.0).unwrap();
    let metrics = font_metrics_px(ts.font_registry(), &resolved).unwrap();
    let expected = metrics.ascent + metrics.descent + metrics.leading;

    let actual = ts.service.default_line_height();
    assert!(
        (actual - expected).abs() < 0.001,
        "expected {}, got {}",
        expected,
        actual
    );
    assert!(actual > 0.0);
}

#[test]
fn measure_line_height_scales_with_font_size() {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);

    let small = ts.service.measure_line_height(&text_typeset::TextFormat {
        font_size: Some(12.0),
        ..Default::default()
    });
    let large = ts.service.measure_line_height(&text_typeset::TextFormat {
        font_size: Some(48.0),
        ..Default::default()
    });

    assert!(small > 0.0);
    assert!(
        large > small * 3.5,
        "48px should be roughly 4× 12px line-height"
    );
}

#[test]
fn measure_line_height_default_format_matches_default_line_height() {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 18.0);

    let via_measure = ts
        .service
        .measure_line_height(&text_typeset::TextFormat::default());
    let via_default = ts.service.default_line_height();
    assert!((via_measure - via_default).abs() < 0.001);
}
