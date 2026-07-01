//! Font enumeration + writing-system (script) classification.
//!
//! These run against the bundled `test-fonts/` (Latin/Greek/Cyrillic,
//! Arabic, Hebrew, Devanagari) so they are deterministic and CI-safe. A
//! CJK-coverage test would need a bundled CJK font (none is shipped — a
//! size/license cost), so that one is `#[ignore]`d and depends on OS fonts,
//! matching the `system_font_tests.rs` convention.

mod helpers;
use helpers::{NOTO_ARABIC, NOTO_DEVANAGARI, NOTO_HEBREW, NOTO_SANS};

use text_typeset::TextFontService;
use text_typeset::WritingSystem;
use text_typeset::font::writing_system::writing_systems_for_face;

// ── Family enumeration ──────────────────────────────────────────

#[test]
fn families_are_sorted_deduped_and_named() {
    let mut s = TextFontService::new_without_system_fonts();
    s.register_font(NOTO_SANS);
    s.register_font(NOTO_ARABIC);
    s.register_font(NOTO_HEBREW);

    let families = s.families();
    assert!(!families.is_empty(), "registered fonts should enumerate");

    // Sorted case-insensitively.
    let names: Vec<String> = families.iter().map(|f| f.name.clone()).collect();
    let mut sorted = names.clone();
    sorted.sort_by_key(|n| n.to_lowercase());
    assert_eq!(
        names, sorted,
        "families() must be sorted case-insensitively"
    );

    // Deduplicated (no case-insensitive duplicates).
    let mut lowered: Vec<String> = names.iter().map(|n| n.to_lowercase()).collect();
    let n = lowered.len();
    lowered.sort();
    lowered.dedup();
    assert_eq!(lowered.len(), n, "families() must be deduplicated");

    // The registered fonts show up.
    assert!(
        names.iter().any(|n| n.contains("Noto Sans")),
        "expected a Noto Sans family, got {names:?}"
    );

    // family_names() is the same list, projected.
    let just_names = s.family_names();
    assert_eq!(just_names, names);
}

#[test]
fn known_fonts_are_not_monospaced() {
    let mut s = TextFontService::new_without_system_fonts();
    s.register_font(NOTO_SANS);
    // Noto Sans is proportional.
    assert!(!s.family_is_monospaced("Noto Sans"));
    // Unknown family is trivially not monospaced.
    assert!(!s.family_is_monospaced("No Such Font"));
}

// ── Writing-system classification (pure function, bundled fonts) ─

#[test]
fn latin_font_reports_latin() {
    let ws = writing_systems_for_face(NOTO_SANS, 0);
    assert!(ws.contains(WritingSystem::Latin), "Noto Sans covers Latin");
    assert!(
        !ws.contains(WritingSystem::Arabic),
        "Noto Sans (Latin/Greek/Cyrillic) must not report Arabic"
    );
}

#[test]
fn arabic_font_reports_arabic() {
    let ws = writing_systems_for_face(NOTO_ARABIC, 0);
    assert!(ws.contains(WritingSystem::Arabic));
}

#[test]
fn hebrew_font_reports_hebrew() {
    let ws = writing_systems_for_face(NOTO_HEBREW, 0);
    assert!(ws.contains(WritingSystem::Hebrew));
}

#[test]
fn devanagari_font_reports_devanagari() {
    let ws = writing_systems_for_face(NOTO_DEVANAGARI, 0);
    assert!(ws.contains(WritingSystem::Devanagari));
}

#[test]
fn garbage_bytes_yield_empty_set() {
    let ws = writing_systems_for_face(b"not a font", 0);
    assert!(ws.is_empty());
}

// ── Background index builder ─────────────────────────────────────

#[test]
fn index_builder_covers_registered_families() {
    let mut s = TextFontService::new_without_system_fonts();
    s.register_font(NOTO_SANS);
    s.register_font(NOTO_ARABIC);

    let builder = s.writing_system_index_builder();
    assert!(builder.family_count() >= 2);

    // build() is what the widget runs off-thread. Keys are lowercased so
    // they align with `families()`' case-insensitive dedup.
    let map = builder.build();
    let arabic_family = map
        .iter()
        .find(|(name, _)| name.contains("arabic"))
        .map(|(_, set)| *set)
        .expect("Arabic family should be indexed");
    assert!(arabic_family.contains(WritingSystem::Arabic));

    let latin_family = map
        .iter()
        .find(|(name, set)| {
            name.contains("noto sans") && !name.contains("arabic") && !set.is_empty()
        })
        .map(|(_, set)| *set)
        .expect("Latin family should be indexed");
    assert!(latin_family.contains(WritingSystem::Latin));
}

#[test]
#[ignore = "depends on OS-installed fonts (non-deterministic in CI)"]
fn system_enumeration_lists_many_families() {
    let s = TextFontService::new();
    let families = s.families();
    assert!(
        families.len() > 10,
        "a desktop OS should expose many font families"
    );
    // At least one monospaced family is typically installed.
    assert!(
        families.iter().any(|f| f.monospaced),
        "expected at least one monospaced system font"
    );
}
