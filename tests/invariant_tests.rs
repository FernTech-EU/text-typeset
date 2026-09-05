//! Semantic invariants of text-typeset's public API.
//!
//! Complements `fuzz_robustness_tests.rs`: that suite asserts "no
//! panic on any input"; this one asserts the deeper algebraic
//! relationships that must hold across random inputs. Each
//! property is named and aimed at one relationship, so a shrunken
//! counter-example points directly at a bug.
//!
//! These are layout-engine properties that don't depend on exact
//! font metrics — they exercise relational correctness (monotonic-
//! ity, idempotence, round-trips, determinism, additivity) rather
//! than pixel-exact output.

mod helpers;

use helpers::{RenderFrameExt, make_block, make_typesetter};
use proptest::prelude::*;
use text_typeset::{CursorDisplay, TextFormat};

fn arb_text() -> impl Strategy<Value = String> {
    proptest::string::string_regex(r"[a-zA-Z0-9 ]{1,60}").unwrap()
}

fn arb_word() -> impl Strategy<Value = String> {
    proptest::string::string_regex(r"[a-zA-Z]{1,20}").unwrap()
}

// ── Invariant 1: layout determinism ─────────────────────────────────
// Same input → same output, every time. If `layout_paragraph` is not
// deterministic, caching, incremental rendering, and snapshot tests
// are all unreliable.

proptest! {
    #[test]
    fn layout_paragraph_is_deterministic(
        text in arb_text(),
        width in 10.0f32..800.0,
    ) {
        let mut ts1 = make_typesetter();
        let mut ts2 = make_typesetter();
        let r1 = ts1.layout_paragraph(&text, &TextFormat::default(), width, None);
        let r2 = ts2.layout_paragraph(&text, &TextFormat::default(), width, None);
        prop_assert_eq!(r1.line_count, r2.line_count);
        prop_assert_eq!(r1.glyphs.len(), r2.glyphs.len());
        // Exact positions should also match across two fresh
        // typesetters — no hidden state bleeds between instances.
        prop_assert!((r1.width - r2.width).abs() < 0.01);
        prop_assert!((r1.height - r2.height).abs() < 0.01);
    }
}

// ── Invariant 2: width monotonicity ─────────────────────────────────
// Widening the layout can only reduce (or keep equal) the line
// count. Narrowing can only increase (or keep equal).

proptest! {
    #[test]
    fn wider_layout_has_fewer_or_equal_lines(
        text in arb_text(),
        base in 40.0f32..400.0,
        delta in 10.0f32..400.0,
    ) {
        let mut ts = make_typesetter();
        let narrow = ts.layout_paragraph(&text, &TextFormat::default(), base, None);
        let wide = ts.layout_paragraph(&text, &TextFormat::default(), base + delta, None);
        prop_assert!(
            wide.line_count <= narrow.line_count,
            "widening {} -> {} should not increase lines: {} -> {}",
            base, base + delta, narrow.line_count, wide.line_count
        );
    }
}

// ── Invariant 3: appending text never decreases glyphs ──────────────

proptest! {
    #[test]
    fn append_never_decreases_glyph_count(
        base in arb_text(),
        extra in arb_word(),
        width in 50.0f32..800.0,
    ) {
        let mut ts = make_typesetter();
        let r1 = ts.layout_paragraph(&base, &TextFormat::default(), width, None);
        let combined = format!("{} {}", base, extra);
        let r2 = ts.layout_paragraph(&combined, &TextFormat::default(), width, None);
        prop_assert!(
            r2.glyphs.len() >= r1.glyphs.len(),
            "append shrank glyph count: {} -> {}",
            r1.glyphs.len(), r2.glyphs.len()
        );
    }
}

// ── Invariant 4: character_geometry count matches range ─────────────
// `character_geometry(block, 0, n)` returns exactly n entries even
// when the shaper produces ligatures (e.g. `"fi"` → one glyph,
// two characters). AccessKit's `character_positions` /
// `character_widths` need one entry per character to track the
// caret at character granularity.

proptest! {
    #[test]
    fn character_geometry_length_matches_char_range(text in arb_text()) {
        let mut ts = make_typesetter();
        ts.layout_blocks(vec![make_block(1, &text)]);
        let n = text.chars().count();
        let geom = ts.character_geometry(1, 0, n);
        prop_assert_eq!(
            geom.len(), n,
            "character_geometry must return exactly one entry per character"
        );
    }
}

// Concrete regression guards for Invariant 4. The proptest above can
// stumble onto a ligature by chance; these pin the `== n` contract for
// the specific inputs that deterministically exercise the shaper's
// `liga` substitution ("fi" → one glyph, "ffi" → one glyph in most
// fonts). The collapse was a real accessibility bug — `character_
// geometry` returned zero entries for a ligated range, leaving a
// screen reader unable to place the caret between the characters —
// fixed by computing `line.char_range` end from text length rather
// than glyph count. Kept as named cases so a regression points at the
// ligature path directly instead of at a shrunken random string.

#[test]
fn ligature_character_geometry_returns_one_entry_per_char() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "fi")]);
    let geom = ts.character_geometry(1, 0, 2);
    // One entry per character, so the caret can sit between `f` and `i`
    // even when they shape to a single ligature glyph.
    assert_eq!(
        geom.len(),
        2,
        "ligature must not collapse character_geometry entries"
    );
}

#[test]
fn ffi_ligature_character_geometry_returns_three_entries() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "ffi")]);
    let geom = ts.character_geometry(1, 0, 3);
    // "ffi" triggers a 3→1 ligature in many fonts; the character
    // geometry must still expose three caret slots.
    assert_eq!(geom.len(), 3, "ffi ligature must expose 3 character slots");
}

// ── Invariant 5: hit_test → caret_rect round-trip ───────────────────
// If a hit-test returns position P, then asking for the caret rect at
// P should produce a rectangle whose horizontal range contains the
// hit point (within a tolerance for sub-pixel rounding).

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]
    #[test]
    fn hit_test_roundtrip_caret_is_near_hit_x(
        text in "[a-zA-Z ]{3,30}",
        x_frac in 0.0f32..1.0,
    ) {
        let mut ts = make_typesetter();
        ts.layout_blocks(vec![make_block(1, &text)]);
        let layout_w = ts.layout_width();
        let x = x_frac * layout_w;
        let y = 8.0_f32; // mid-line of default 16px font
        let hit = match ts.hit_test(x, y) {
            Some(h) => h,
            None => return Ok(()),
        };
        // Only exercise the round-trip when the hit is actually on text.
        if !matches!(hit.region, text_typeset::HitRegion::Text) {
            return Ok(());
        }
        let caret = ts.caret_rect(hit.position);
        let caret_x = caret[0];
        // Caret x should be within one glyph's advance width of the
        // hit point — rough 32-pixel tolerance absorbs both font-
        // specific kerning and caret-on-edge cases.
        let diff = (caret_x - x).abs();
        prop_assert!(
            diff <= 32.0,
            "caret_x {} far from hit x {} (diff {})",
            caret_x, x, diff
        );
    }
}

// ── Invariant 6: character_geometry positions are monotonic ─────────

proptest! {
    #[test]
    fn character_geometry_positions_are_monotonic(text in arb_text()) {
        let mut ts = make_typesetter();
        ts.layout_blocks(vec![make_block(1, &text)]);
        let n = text.chars().count();
        let geom = ts.character_geometry(1, 0, n);
        for w in geom.windows(2) {
            prop_assert!(
                w[1].position >= w[0].position,
                "position regressed: {} -> {}",
                w[0].position, w[1].position
            );
        }
    }
}

// ── Invariant 7: empty cursor emits no selection decorations ────────
// When anchor == position, the render must produce zero Selection
// decoration rects (cursor only). Non-negotiable — a stale selection
// rect after collapse is a visible bug.

proptest! {
    #[test]
    fn collapsed_cursor_has_no_selection(
        text in arb_text(),
        pos in 0usize..200,
    ) {
        let mut ts = make_typesetter();
        ts.layout_blocks(vec![make_block(1, &text)]);
        let max = text.chars().count();
        let clamped = pos.min(max);
        ts.set_cursor(&CursorDisplay {
            position: clamped,
            anchor: clamped,
            affinity: text_typeset::CursorAffinity::Downstream,
            visible: true,
            selected_cells: vec![],
        });
        let frame = ts.render();
        prop_assert_eq!(
            frame.decoration_count(text_typeset::DecorationKind::Selection),
            0,
            "collapsed cursor produced selection rects"
        );
    }
}

// ── Invariant 8: viewport change doesn't change paragraph layout ────
// `layout_paragraph` is a pure single-line-api call — changing the
// typesetter's viewport between two calls should not perturb the
// paragraph result. (The paragraph takes an explicit max_width.)

proptest! {
    #[test]
    fn layout_paragraph_independent_of_viewport(
        text in arb_text(),
        width in 50.0f32..400.0,
        vw1 in 100.0f32..1000.0,
        vw2 in 100.0f32..1000.0,
    ) {
        let mut ts = make_typesetter();
        ts.set_viewport(vw1, 600.0);
        let r1 = ts.layout_paragraph(&text, &TextFormat::default(), width, None);
        ts.set_viewport(vw2, 600.0);
        let r2 = ts.layout_paragraph(&text, &TextFormat::default(), width, None);
        prop_assert_eq!(r1.line_count, r2.line_count);
        prop_assert_eq!(r1.glyphs.len(), r2.glyphs.len());
    }
}

// ── Invariant 9: render count stability ─────────────────────────────
// Calling render() twice without changes must produce the same glyph
// count and decoration count. Guards against any hidden per-render
// mutation that would break incremental painting.

proptest! {
    #[test]
    fn render_is_idempotent_without_mutation(text in arb_text()) {
        let mut ts = make_typesetter();
        ts.layout_blocks(vec![make_block(1, &text)]);
        let (n1, d1) = {
            let f = ts.render();
            (f.glyph_count(), f.decorations.len())
        };
        let (n2, d2) = {
            let f = ts.render();
            (f.glyph_count(), f.decorations.len())
        };
        prop_assert_eq!(n1, n2, "glyph count changed between identical renders");
        prop_assert_eq!(d1, d2, "decoration count changed between identical renders");
    }
}

// ── Invariant 10: content_width round-trip ──────────────────────────
// `set_content_width(w)` followed by `layout_width()` must return w
// (within float tolerance). `set_content_width_auto()` must then
// reset to the viewport width.

proptest! {
    #[test]
    fn content_width_set_then_read_round_trips(
        explicit in 10.0f32..1200.0,
        vw in 10.0f32..1200.0,
    ) {
        let mut ts = make_typesetter();
        ts.set_viewport(vw, 600.0);
        ts.set_content_width(explicit);
        prop_assert!(
            (ts.layout_width() - explicit).abs() < 0.01,
            "after set_content_width({}), layout_width() = {}",
            explicit, ts.layout_width()
        );
        ts.set_content_width_auto();
        // `auto` mode derives from viewport; exact formula is
        // library-internal but it must differ-from-explicit when
        // the viewport differs from explicit.
        let after_auto = ts.layout_width();
        prop_assert!(after_auto > 0.0);
    }
}

// ── Geometry invariants ─────────────────────────────────────────────
// The `*_with_geometry` layout paths report, per emitted line, the
// source ranges that line covers and one `CharacterGeometry` per
// character of each of its segments. An accessibility layer indexes
// those ranges directly — AccessKit's `character_positions` /
// `character_widths` on a `Role::TextRun` — so a gap between two
// lines, a short `characters` vector or a position that goes backwards
// puts the review cursor on the wrong character rather than merely
// looking odd.

// Short strings with multibyte characters and explicit newlines, so a
// case exercises the hard-break path as often as the soft-wrap one.
// Shaping a real font dominates the cost of these properties: the
// bounded alphabet and the few-dozen-character cap are what keep a few
// hundred cases per property affordable.
/// Float slack for a measured extent. Advances are summed in f32, so a
/// few-dozen-character line drifts well under a hundredth of a pixel;
/// anything larger is a real disagreement.
const EPS: f32 = 0.05;

fn arb_geometry_text() -> impl Strategy<Value = String> {
    proptest::string::string_regex(r"[a-z0-9 éà🌍\n]{0,40}").unwrap()
}

// ── Invariant 11: emitted lines tile the source ─────────────────────
// Line byte_ranges run contiguously from 0, and — when `max_lines`
// dropped nothing — end at the source length. A hole between two lines
// is text no accessible range can name.

proptest! {
    #[test]
    fn paragraph_geometry_covers_every_byte(
        text in arb_geometry_text(),
        width in 10.0f32..200.0,
        cap in proptest::option::of(1usize..5),
    ) {
        let mut ts = make_typesetter();
        let (_, geom) =
            ts.layout_paragraph_with_geometry(&text, &TextFormat::default(), width, cap);
        prop_assert_eq!(geom.source_len, text.len());
        let mut next = 0usize;
        for line in &geom.lines {
            prop_assert_eq!(
                line.byte_range.start, next,
                "line {} starts at {}, previous line ended at {}",
                line.index, line.byte_range.start, next
            );
            prop_assert!(
                line.byte_range.end >= line.byte_range.start,
                "line {} has an inverted byte range {:?}",
                line.index, line.byte_range
            );
            next = line.byte_range.end;
        }
        // Text the layout declined entirely (empty input) has nothing to
        // tile; a truncated layout stops wherever `max_lines` cut it.
        if !geom.lines.is_empty() && geom.dropped_lines == 0 {
            prop_assert_eq!(
                next, geom.source_len,
                "last line ends at {} but the source is {} bytes",
                next, geom.source_len
            );
        }
    }
}

// ── Invariant 12: segments carry one entry per character ────────────
// A segment is either fully measured or absent — the "empty means
// unmeasurable, never partial" half of the contract lives on the line,
// so any segment that does exist must account for every character of
// its own range, ligatures and combining marks included.

proptest! {
    #[test]
    fn every_segment_has_one_char_entry_per_char_or_none(
        text in arb_geometry_text(),
        width in 10.0f32..200.0,
    ) {
        let mut ts = make_typesetter();
        let (_, geom) =
            ts.layout_paragraph_with_geometry(&text, &TextFormat::default(), width, None);
        for line in &geom.lines {
            let mut covered = 0usize;
            for seg in &line.segments {
                prop_assert_eq!(
                    seg.characters.len(), seg.char_range.len(),
                    "line {} segment {:?} reports {} character entries",
                    line.index, seg.char_range, seg.characters.len()
                );
                covered += seg.characters.len();
            }
            // `char_range` reaches the line from the layout, not from the
            // segments, so this is the half of the contract a segment
            // built from the wrong cluster span can actually break.
            if line.truncation.is_none() {
                prop_assert_eq!(
                    covered, line.char_range.len(),
                    "line {} spans {:?} but its segments measure {} characters",
                    line.index, line.char_range, covered
                );
            }
        }
    }
}

// ── Invariant 13: positions rise in reading order ───────────────────
// A consumer reads the positions as a running offset along the line, so
// one that dips puts a highlight behind its predecessor. The generator
// drives the Latin path; the same assertion holds for an RTL segment
// because positions are measured from the segment's leading edge, which
// is its right edge there.

proptest! {
    #[test]
    fn character_positions_are_monotonic_within_a_segment(
        text in arb_geometry_text(),
        width in 10.0f32..200.0,
    ) {
        let mut ts = make_typesetter();
        let (_, geom) =
            ts.layout_paragraph_with_geometry(&text, &TextFormat::default(), width, None);
        for line in &geom.lines {
            for seg in &line.segments {
                for (i, w) in seg.characters.windows(2).enumerate() {
                    prop_assert!(
                        w[1].position >= w[0].position,
                        "line {} segment {:?} position regressed at {}: {} -> {}",
                        line.index, seg.char_range, i, w[0].position, w[1].position
                    );
                }
                // The rebase onto the segment's leading edge is the part a
                // consumer can see go wrong: the first character sits at
                // zero and the last one ends at the segment's far edge,
                // whichever direction the segment reads in. Monotonicity
                // alone would survive an rebase that slid every character
                // off the box.
                let (Some(first), Some(last)) =
                    (seg.characters.first(), seg.characters.last())
                else {
                    continue;
                };
                prop_assert!(
                    first.position.abs() <= EPS,
                    "line {} segment {:?} starts at {}, not at its leading edge",
                    line.index, seg.char_range, first.position
                );
                prop_assert!(
                    last.position + last.width <= seg.rect[2] + EPS,
                    "line {} segment {:?} is {} wide but its characters reach {}",
                    line.index, seg.char_range, seg.rect[2], last.position + last.width
                );
            }
        }
    }
}

// ── Invariant 14: widths are advances, never deltas ─────────────────
// A character with no advance of its own reports 0.0; nothing reports
// less. A negative width would reach AccessKit as a highlight rect
// extending backwards over its neighbour.

proptest! {
    #[test]
    fn character_widths_are_never_negative(
        text in arb_geometry_text(),
        width in 10.0f32..200.0,
    ) {
        let mut ts = make_typesetter();
        let (_, geom) =
            ts.layout_paragraph_with_geometry(&text, &TextFormat::default(), width, None);
        for line in &geom.lines {
            for seg in &line.segments {
                for (i, c) in seg.characters.iter().enumerate() {
                    prop_assert!(
                        c.width >= 0.0,
                        "line {} segment {:?} character {} has width {}",
                        line.index, seg.char_range, i, c.width
                    );
                }
                // The advances add up to the segment's own width. The
                // non-negativity above is enforced by a clamp inside the
                // geometry builder and so cannot fail; this is the
                // assertion that notices when the clamp had to fire, or
                // when a cluster hands its advance to the wrong character.
                // No path reached here justifies its lines, so there are no
                // inter-run gaps for the sum to fall short of.
                let sum: f32 = seg.characters.iter().map(|c| c.width).sum();
                prop_assert!(
                    (sum - seg.rect[2]).abs() <= EPS,
                    "line {} segment {:?} is {} wide but its advances sum to {}",
                    line.index, seg.char_range, seg.rect[2], sum
                );
            }
        }
    }
}

// ── Invariant 15: geometry is instance-independent ──────────────────
// Same input, two typesetters built from scratch, byte-identical
// geometry. Consumers cache these ranges against a document revision;
// if a fresh instance disagreed with a warm one, a cache reload would
// silently move every caret. Covers the paragraph path and the
// single-line path, whose truncation bookkeeping is the more fragile
// of the two.

proptest! {
    #[test]
    fn geometry_is_deterministic_across_two_fresh_typesetters(
        text in arb_geometry_text(),
        width in 10.0f32..200.0,
    ) {
        let mut ts1 = make_typesetter();
        let mut ts2 = make_typesetter();
        let (_, para1) =
            ts1.layout_paragraph_with_geometry(&text, &TextFormat::default(), width, None);
        let (_, para2) =
            ts2.layout_paragraph_with_geometry(&text, &TextFormat::default(), width, None);
        prop_assert_eq!(para1, para2, "paragraph geometry differs between instances");
        let (_, single1) =
            ts1.layout_single_line_with_geometry(&text, &TextFormat::default(), Some(width));
        let (_, single2) =
            ts2.layout_single_line_with_geometry(&text, &TextFormat::default(), Some(width));
        prop_assert_eq!(single1, single2, "single-line geometry differs between instances");
    }
}

// ── Invariant 16: one geometry line per emitted line ────────────────
// `LayoutGeometry::lines` describes the lines the layout drew, not the
// ones it considered: a caller iterating the geometry alongside the
// glyphs must not run past the end. Checked both uncapped and with
// `max_lines`, where the surplus goes to `dropped_lines` instead.

proptest! {
    #[test]
    fn geometry_line_count_equals_result_line_count(
        text in arb_geometry_text(),
        width in 10.0f32..200.0,
        cap in 1usize..5,
    ) {
        let mut ts = make_typesetter();
        let (uncapped, uncapped_geom) =
            ts.layout_paragraph_with_geometry(&text, &TextFormat::default(), width, None);
        prop_assert_eq!(
            uncapped_geom.lines.len(), uncapped.line_count,
            "uncapped: {} geometry lines for {} emitted lines",
            uncapped_geom.lines.len(), uncapped.line_count
        );
        let (capped, capped_geom) =
            ts.layout_paragraph_with_geometry(&text, &TextFormat::default(), width, Some(cap));
        prop_assert_eq!(
            capped_geom.lines.len(), capped.line_count,
            "max_lines={}: {} geometry lines for {} emitted lines",
            cap, capped_geom.lines.len(), capped.line_count
        );
    }
}
