//! Bidirectional layout in the **multi-line block** path.
//!
//! The single-line path has done UAX #9 for a long time via
//! `bidi_runs`; the block path — the one behind every multi-line rich
//! text editor — used to shape each formatting fragment as one
//! direction-agnostic unit and emit the results in logical order. A
//! paragraph mixing Arabic and Latin therefore rendered in typing order
//! rather than reading order.
//!
//! These tests pin the block path specifically: run reordering per line,
//! the paragraph base direction, and alignment derived from it.

mod helpers;

use helpers::{NOTO_ARABIC, NOTO_SANS, Typesetter, make_block};
use text_typeset::layout::block::layout_block;
use text_typeset::layout::line::LayoutLine;
use text_typeset::layout::paragraph::Alignment;
use text_typeset::shaping::shaper::TextDirection;

/// "كتب" (kataba) and "سلام" (salaam) — two Arabic words.
const KATABA: &str = "\u{0643}\u{062A}\u{0628}";
const SALAAM: &str = "\u{0633}\u{0644}\u{0627}\u{0645}";

/// A typesetter that can render both Latin and Arabic.
///
/// Arabic is the default face and Latin is registered alongside it, so
/// glyph fallback covers whichever the block's text actually uses.
fn bilingual() -> Typesetter {
    let mut ts = Typesetter::new();
    let arabic = ts.register_font(NOTO_ARABIC);
    ts.register_font(NOTO_SANS);
    ts.set_default_font(arabic, 16.0);
    ts.set_viewport(800.0, 600.0);
    ts
}

/// The lowest char offset each run covers, in the order the runs are
/// painted left to right.
///
/// Glyph clusters are char offsets in the block text by the time layout
/// is done, so the minimum cluster identifies which word a run holds
/// regardless of the direction it was shaped in.
fn logical_starts_in_visual_order(line: &LayoutLine) -> Vec<usize> {
    let mut runs: Vec<(f32, usize)> = line
        .runs
        .iter()
        .filter_map(|r| {
            let min = r.shaped_run.glyphs.iter().map(|g| g.cluster as usize).min()?;
            Some((r.x, min))
        })
        .collect();
    runs.sort_by(|a, b| a.0.total_cmp(&b.0));
    runs.into_iter().map(|(_, c)| c).collect()
}

fn single_line(ts: &Typesetter, params: &text_typeset::layout::block::BlockLayoutParams) -> LayoutLine {
    let layout = layout_block(ts.font_registry(), params, 600.0, 1.0, 1.0);
    assert_eq!(layout.lines.len(), 1, "test expects the text to fit one line");
    layout.lines.into_iter().next().unwrap()
}

#[test]
fn an_ltr_paragraph_keeps_its_runs_in_logical_order() {
    let ts = bilingual();
    let text = format!("abc {KATABA} def");
    let mut params = make_block(0, &text);
    params.base_direction = TextDirection::LeftToRight;

    let line = single_line(&ts, &params);
    let starts = logical_starts_in_visual_order(&line);

    // Latin paragraph: the embedded Arabic sits between its neighbours
    // and everything reads left to right, so visual order == logical.
    assert!(
        starts.windows(2).all(|w| w[0] < w[1]),
        "an LTR paragraph should paint runs in ascending logical order; got {starts:?}"
    );
}

#[test]
fn an_rtl_paragraph_paints_its_first_word_rightmost() {
    let ts = bilingual();
    // Logical order: KATABA, "hello", SALAAM. Read right-to-left, that
    // puts KATABA on the right and SALAAM on the left.
    let text = format!("{KATABA} hello {SALAAM}");
    let mut params = make_block(0, &text);
    params.base_direction = TextDirection::RightToLeft;

    let line = single_line(&ts, &params);
    let starts = logical_starts_in_visual_order(&line);

    assert!(
        starts.len() >= 3,
        "expected the paragraph to split into at least three directional runs, got {starts:?}"
    );
    assert!(
        starts.first() > starts.last(),
        "in an RTL paragraph the first logical word must paint rightmost \
         (so visual order runs from high char offsets to low); got {starts:?}"
    );

    // The Latin island keeps its own left-to-right order in the middle:
    // its run must sit between the two Arabic runs, not at either end.
    let hello_start = text.find("hello").map(|b| text[..b].chars().count()).unwrap();
    let hello_pos = starts.iter().position(|&s| s == hello_start);
    assert!(
        matches!(hello_pos, Some(p) if p > 0 && p < starts.len() - 1),
        "the Latin word should land between the two Arabic words; \
         starts {starts:?}, looking for {hello_start}"
    );
}

#[test]
fn the_stored_direction_overrides_auto_detection() {
    let ts = bilingual();
    // Opens with a Latin acronym, so rules P2/P3 read the paragraph as
    // LTR even though it is Arabic prose.
    let text = format!("NASA {KATABA} {SALAAM}");

    let mut auto = make_block(0, &text);
    auto.base_direction = TextDirection::Auto;
    let auto_starts = logical_starts_in_visual_order(&single_line(&ts, &auto));

    let mut forced = make_block(0, &text);
    forced.base_direction = TextDirection::RightToLeft;
    let forced_starts = logical_starts_in_visual_order(&single_line(&ts, &forced));

    assert_eq!(
        auto_starts.first(),
        Some(&0),
        "auto-detection is expected to lead with the Latin acronym; got {auto_starts:?}"
    );
    assert_ne!(
        forced_starts.first(),
        Some(&0),
        "with the direction set to RTL the acronym must no longer paint \
         leftmost; got {forced_starts:?}"
    );
}

#[test]
fn an_rtl_paragraph_right_aligns_by_default() {
    let ts = bilingual();
    let text = format!("{KATABA} {SALAAM}");

    // `Alignment::Start` is the default — "whatever the paragraph's
    // direction says" — so the same block flips edge with its direction.
    let mut ltr = make_block(0, &text);
    ltr.alignment = Alignment::Start;
    ltr.base_direction = TextDirection::LeftToRight;

    let mut rtl = make_block(0, &text);
    rtl.alignment = Alignment::Start;
    rtl.base_direction = TextDirection::RightToLeft;

    let ltr_line = single_line(&ts, &ltr);
    let rtl_line = single_line(&ts, &rtl);

    let left_edge = |l: &LayoutLine| l.runs.iter().map(|r| r.x).fold(f32::INFINITY, f32::min);

    assert!(
        left_edge(&ltr_line) < 1.0,
        "an LTR paragraph should sit flush left; got {}",
        left_edge(&ltr_line)
    );
    assert!(
        left_edge(&rtl_line) > left_edge(&ltr_line) + 1.0,
        "an RTL paragraph should sit against the right edge instead; \
         LTR started at {}, RTL at {}",
        left_edge(&ltr_line),
        left_edge(&rtl_line)
    );
    // Flush right means the line ends at the content width.
    let right_edge = rtl_line
        .runs
        .iter()
        .map(|r| r.x + r.shaped_run.advance_width)
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(
        (right_edge - 600.0).abs() < 1.0,
        "the RTL line should end at the content width (600); got {right_edge}"
    );
}

#[test]
fn an_explicit_alignment_still_wins_over_direction() {
    let ts = bilingual();
    let text = format!("{KATABA} {SALAAM}");

    // A writer who deliberately chose flush-left keeps it, RTL or not:
    // only the *unset* Start/End variants follow the direction.
    let mut params = make_block(0, &text);
    params.alignment = Alignment::Left;
    params.base_direction = TextDirection::RightToLeft;

    let line = single_line(&ts, &params);
    let left_edge = line.runs.iter().map(|r| r.x).fold(f32::INFINITY, f32::min);
    assert!(
        left_edge < 1.0,
        "an explicit Left alignment must survive an RTL direction; got {left_edge}"
    );
}

#[test]
fn a_first_line_indent_insets_from_the_right_in_an_rtl_paragraph() {
    let ts = bilingual();
    let text = format!("{KATABA} {SALAAM}");
    const INDENT: f32 = 40.0;

    let mut params = make_block(0, &text);
    params.alignment = Alignment::Start;
    params.base_direction = TextDirection::RightToLeft;
    params.text_indent = INDENT;

    let line = single_line(&ts, &params);
    let right_edge = line
        .runs
        .iter()
        .map(|r| r.x + r.shaped_run.advance_width)
        .fold(f32::NEG_INFINITY, f32::max);

    // The indent is a *leading*-edge inset, and in an RTL paragraph the
    // leading edge is the right one.
    assert!(
        (right_edge - (600.0 - INDENT)).abs() < 1.0,
        "an RTL first line should be inset from the right by the indent: \
         expected right edge {}, got {right_edge}",
        600.0 - INDENT
    );
}

#[test]
fn arabic_in_a_block_is_shaped_with_joined_letterforms() {
    let ts = bilingual();
    let mut params = make_block(0, KATABA);
    params.base_direction = TextDirection::RightToLeft;

    let line = single_line(&ts, &params);
    let glyphs: Vec<u16> = line
        .runs
        .iter()
        .flat_map(|r| r.shaped_run.glyphs.iter().map(|g| g.glyph_id))
        .collect();

    assert!(
        !glyphs.is_empty() && glyphs.iter().all(|&g| g != 0),
        "Arabic in a block should shape without .notdef; got {glyphs:?}"
    );
    // Joined "كتب" is five glyphs; the disconnected isolated forms are
    // six. See complex_script_tests for the full oracle.
    assert_eq!(
        glyphs.len(),
        5,
        "expected the three letters to join into 5 glyphs, got {} — \
         the block path is shaping isolated forms",
        glyphs.len()
    );
}

/// Selection highlight rects produced for `anchor..position` over one block.
fn selection_rects_for(
    ts: &mut Typesetter,
    params: text_typeset::layout::block::BlockLayoutParams,
    anchor: usize,
    position: usize,
) -> Vec<[f32; 4]> {
    ts.layout_blocks(vec![params]);
    ts.set_cursor(&text_typeset::CursorDisplay {
        position,
        anchor,
        affinity: text_typeset::CursorAffinity::Downstream,
        visible: true,
        selected_cells: vec![],
    });
    ts.render()
        .decorations
        .iter()
        .filter(|d| d.kind == text_typeset::DecorationKind::Selection)
        .map(|d| d.rect)
        .collect()
}

#[test]
fn selecting_rtl_text_on_one_line_paints_a_visible_highlight() {
    let mut ts = bilingual();
    let mut params = make_block(1, KATABA);
    params.base_direction = TextDirection::RightToLeft;

    let rects = selection_rects_for(&mut ts, params, 0, KATABA.chars().count());

    // The failure this guards was not a wrong rect but *no* rect: the old
    // code took the x of each endpoint and subtracted, and on an RTL run
    // the start sits right of the end, so the negative width was dropped
    // by a `width > 0` guard. Cut/copy/delete kept working on the correct
    // range underneath, which made it read as a rendering glitch.
    assert!(
        !rects.is_empty(),
        "selecting right-to-left text must paint a highlight"
    );
    for r in &rects {
        assert!(
            r[2] > 0.0 && r[3] > 0.0,
            "selection rect should have positive size; got {r:?}"
        );
    }
}

#[test]
fn an_rtl_selection_covers_the_same_width_as_the_text() {
    let mut ts = bilingual();
    let mut params = make_block(1, KATABA);
    params.base_direction = TextDirection::RightToLeft;

    // Measure the text first, then check the highlight matches it.
    let line = single_line(&ts, &params);
    let text_width: f32 = line.runs.iter().map(|r| r.shaped_run.advance_width).sum();

    let rects = selection_rects_for(&mut ts, params, 0, KATABA.chars().count());
    let covered: f32 = rects.iter().map(|r| r[2]).sum();

    assert!(
        (covered - text_width).abs() < 1.0,
        "a full selection should cover the text's width: text {text_width}, \
         highlight {covered} (rects {rects:?})"
    );
}

#[test]
fn a_selection_across_a_direction_boundary_paints_every_piece() {
    let mut ts = bilingual();
    let text = format!("{KATABA} hello {SALAAM}");
    let mut params = make_block(1, &text);
    params.base_direction = TextDirection::RightToLeft;

    let chars = text.chars().count();
    let rects = selection_rects_for(&mut ts, params, 0, chars);

    let covered: f32 = rects.iter().map(|r| r[2]).sum();
    assert!(
        !rects.is_empty() && covered > 0.0,
        "a mixed-direction selection must paint something; got {rects:?}"
    );

    // Selecting everything covers everything, however many pieces the
    // reordering split the range into.
    let mut ts2 = bilingual();
    let mut measure = make_block(1, &text);
    measure.base_direction = TextDirection::RightToLeft;
    let line = single_line(&ts2, &measure);
    let text_width: f32 = line.runs.iter().map(|r| r.shaped_run.advance_width).sum();
    assert!(
        (covered - text_width).abs() < 2.0,
        "selecting the whole line should cover its whole width: text \
         {text_width}, highlight {covered} across {} rect(s)",
        rects.len()
    );
    let _ = &mut ts2;
}

#[test]
fn a_wrapped_rtl_paragraph_reorders_each_line_independently() {
    let ts = bilingual();
    // Alternate Arabic and Latin so that *every* line carries runs of
    // more than one embedding level. An all-Arabic paragraph collapses
    // to a single run per line, which would make the ordering assertion
    // below vacuously true and prove nothing.
    let words: Vec<String> = (0..10)
        .map(|i| match i % 3 {
            0 => KATABA.to_string(),
            1 => format!("word{i}"),
            _ => SALAAM.to_string(),
        })
        .collect();
    let text = words.join(" ");

    let mut params = make_block(0, &text);
    params.base_direction = TextDirection::RightToLeft;

    let layout = layout_block(ts.font_registry(), &params, 140.0, 1.0, 1.0);
    assert!(
        layout.lines.len() > 1,
        "test needs the paragraph to wrap; it produced {} line(s)",
        layout.lines.len()
    );

    // Every line independently reads right to left. Reordering the
    // paragraph before breaking (rather than per line) would leave the
    // later lines scrambled, so checking all of them is the point.
    let mut lines_with_several_runs = 0;
    for (i, line) in layout.lines.iter().enumerate() {
        let starts = logical_starts_in_visual_order(line);
        if starts.len() > 1 {
            lines_with_several_runs += 1;
        }
        assert!(
            starts.first() >= starts.last(),
            "line {i} of an RTL paragraph should paint its earliest text \
             rightmost; got {starts:?}"
        );
    }
    assert!(
        lines_with_several_runs >= 2,
        "the fixture must produce multi-run lines or this test asserts \
         nothing; only {lines_with_several_runs} line(s) had several runs"
    );
}

#[test]
fn a_blank_paragraph_inside_a_selection_stays_finite() {
    // Regression: the trailing-edge extension folded over an *empty*
    // span list, so `rightmost` started at NEG_INFINITY and the rect
    // came out [-inf, y, inf, h]. A blank line between two prose
    // paragraphs is the most ordinary structure there is, so nearly
    // every multi-paragraph selection hit it.
    let mut ts = bilingual();
    ts.layout_blocks(vec![
        helpers::make_block_at(0, 0, "Hello"),
        helpers::make_block_at(1, 6, ""),
        helpers::make_block_at(2, 7, "World"),
    ]);
    ts.set_cursor(&text_typeset::CursorDisplay {
        position: 12,
        anchor: 0,
        affinity: text_typeset::CursorAffinity::Downstream,
        visible: true,
        selected_cells: vec![],
    });

    let rects: Vec<[f32; 4]> = ts
        .render()
        .decorations
        .iter()
        .filter(|d| d.kind == text_typeset::DecorationKind::Selection)
        .map(|d| d.rect)
        .collect();

    assert!(!rects.is_empty(), "the selection should paint something");
    for r in &rects {
        assert!(
            r.iter().all(|v| v.is_finite()),
            "selection rect must be finite; got {r:?}"
        );
        assert!(
            r[2] >= 0.0 && r[2] <= 1000.0,
            "selection rect width should be sane; got {r:?}"
        );
    }
}

#[test]
fn a_partly_selected_cluster_still_paints() {
    // A glyph covering several characters — a ligature, an Indic
    // conjunct — must highlight when the selection cuts into it. Testing
    // only whether the cluster *starts* inside the range dropped it.
    let n = KATABA.chars().count();
    let mut ts = bilingual();
    let mut params = make_block(1, KATABA);
    params.base_direction = TextDirection::RightToLeft;

    // Every single-character selection must paint, including any that
    // land inside a cluster covering more than one char.
    for c in 0..n {
        let mut p = make_block(1, KATABA);
        p.base_direction = TextDirection::RightToLeft;
        let rects = selection_rects_for(&mut ts, p, c, c + 1);
        assert!(
            !rects.is_empty(),
            "selecting character {c} of {n} must paint something"
        );
    }
    let _ = &mut params;
}

// ── Stage 7: the caret at a direction seam ─────────────────────

/// Caret x at `offset` under each affinity, for a one-line block.
fn caret_x_both_ways(ts: &Typesetter, params: &text_typeset::layout::block::BlockLayoutParams,
                     offset: usize) -> (f32, f32) {
    let line = single_line(ts, params);
    (
        line.x_for_offset_with_affinity(offset, text_typeset::CursorAffinity::Downstream),
        line.x_for_offset_with_affinity(offset, text_typeset::CursorAffinity::Upstream),
    )
}

#[test]
fn a_direction_seam_gives_the_caret_two_places_to_be() {
    let ts = bilingual();
    // "abc<arabic>" — the offset between the Latin and the Arabic is
    // both the trailing edge of "abc" and the leading edge of the
    // Arabic word, and those are at different x.
    let text = format!("abc{KATABA}");
    let mut params = make_block(0, &text);
    params.base_direction = TextDirection::LeftToRight;

    let seam = 3; // just after "abc"
    let line = single_line(&ts, &params);
    assert!(
        line.is_direction_boundary(seam),
        "offset {seam} should be a direction boundary in {text:?}"
    );

    let (downstream, upstream) = caret_x_both_ways(&ts, &params, seam);
    assert_ne!(
        downstream, upstream,
        "the two affinities must resolve to different x at a seam — that \
         ambiguity is the whole reason the axis exists"
    );

    // Downstream attaches to the text *before* the offset, so it sits at
    // the right edge of "abc"; upstream attaches to the Arabic, whose
    // leading edge is its own right-hand side, further right.
    assert!(
        upstream > downstream,
        "upstream should sit at the RTL run's leading (right) edge, \
         downstream at the end of the Latin; got down={downstream}, up={upstream}"
    );
}

#[test]
fn affinity_does_nothing_away_from_a_seam() {
    let ts = bilingual();
    let mut params = make_block(0, "abcdef");
    params.base_direction = TextDirection::LeftToRight;

    let line = single_line(&ts, &params);
    for offset in 0..=6 {
        assert!(
            !line.is_direction_boundary(offset),
            "uniform Latin text has no direction boundary at {offset}"
        );
        let (d, u) = caret_x_both_ways(&ts, &params, offset);
        assert_eq!(
            d, u,
            "affinity must be a no-op at offset {offset} of uniform text"
        );
    }
}

#[test]
fn the_caret_rect_follows_the_chosen_side_of_a_seam() {
    let mut ts = bilingual();
    let text = format!("abc{KATABA}");
    let mut params = make_block(1, &text);
    params.base_direction = TextDirection::LeftToRight;
    ts.layout_blocks(vec![params]);

    let seam = 3;
    let downstream = ts.caret_rect_with_affinity(seam, text_typeset::CursorAffinity::Downstream);
    let upstream = ts.caret_rect_with_affinity(seam, text_typeset::CursorAffinity::Upstream);

    // The rect is what actually gets painted, so the axis has to survive
    // the whole way out — not just the line-level helper.
    assert_ne!(
        downstream[0], upstream[0],
        "caret_rect must honour the bidi axis; got {downstream:?} and {upstream:?}"
    );
    assert!(downstream.iter().all(|v| v.is_finite()));
    assert!(upstream.iter().all(|v| v.is_finite()));
}
