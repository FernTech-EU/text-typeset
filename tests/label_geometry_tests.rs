//! Per-line, per-character geometry, measured with real fonts.
//!
//! A screen reader does not consume glyph quads; it consumes extents.
//! Braille cell routing, the review cursor and macOS's
//! `AXBoundsForRange` all ask "where does character *n* sit, and how
//! wide is it" — and AccessKit answers from the `character_positions` /
//! `character_widths` arrays a `Role::TextRun` carries. The geometry API
//! is where those arrays come from, so what it promises has to hold
//! against real shaping rather than fixed-width mock metrics: cluster
//! merging, bidi reordering and the ellipsis path all change the answer.
//!
//! These tests lock the promises a consumer indexes with — that every
//! source character is accounted for exactly once, that line ranges
//! partition the source, that hard breaks are distinguishable from soft
//! wraps, and that an RTL run reports positive widths rising from its
//! own leading edge rather than the zeroes a left-to-right delta gave.

mod helpers;

use helpers::{NOTO_ARABIC, NOTO_HEBREW, NOTO_SANS, Typesetter, make_block, make_typesetter};
use text_typeset::{
    CharacterGeometry, GeometryDirection, InlineMarkup, LayoutGeometry, LineEnd, LineGeometry,
    TextFormat,
};

/// Wide enough that nothing wraps: only explicit breaks split lines.
const WIDE: f32 = 600.0;

/// A single sentence with no explicit break, long enough to wrap several
/// times at the narrow widths used below.
const PROSE: &str = "The quick brown fox jumps over the lazy dog near the riverbank at dawn.";

/// "shalom" — four Hebrew letters, logical order ש ל ו ם.
const SHALOM: &str = "\u{05E9}\u{05DC}\u{05D5}\u{05DD}";

/// "kataba" and "salaam" — two Arabic words to sit either side of a
/// Latin one.
const KATABA: &str = "\u{0643}\u{062A}\u{0628}";
const SALAAM: &str = "\u{0633}\u{0644}\u{0627}\u{0645}";

fn plain() -> TextFormat {
    TextFormat::default()
}

/// Hermetic Hebrew-only typesetter — the host's installed fonts must not
/// decide what these tests measure.
fn hebrew_typesetter() -> Typesetter {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_HEBREW);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(800.0, 600.0);
    ts
}

/// Arabic as the default face with Latin registered alongside, so
/// fallback covers a mixed-script line.
fn bilingual_typesetter() -> Typesetter {
    let mut ts = Typesetter::new();
    let arabic = ts.register_font(NOTO_ARABIC);
    ts.register_font(NOTO_SANS);
    ts.set_default_font(arabic, 16.0);
    ts.set_viewport(800.0, 600.0);
    ts
}

fn only_line(geometry: &LayoutGeometry) -> &LineGeometry {
    assert_eq!(
        geometry.lines.len(),
        1,
        "the single-line path emits exactly one line"
    );
    &geometry.lines[0]
}

/// The line's characters in logical order, walking its segments and
/// checking on the way that they tile `line.char_range` without a gap or
/// an overlap.
fn characters_in_logical_order(line: &LineGeometry) -> Vec<CharacterGeometry> {
    let mut next = line.char_range.start;
    let mut out = Vec::new();
    for (i, segment) in line.segments.iter().enumerate() {
        assert_eq!(
            segment.char_range.start, next,
            "segment {i} starts at {} but the previous one ended at {next}",
            segment.char_range.start
        );
        assert_eq!(
            segment.characters.len(),
            segment.char_range.len(),
            "segment {i} covers {:?} but reports {} characters",
            segment.char_range,
            segment.characters.len()
        );
        next = segment.char_range.end;
        out.extend(segment.characters.iter().copied());
    }
    out
}

#[test]
fn single_line_geometry_covers_every_char() {
    let mut ts = make_typesetter();
    let text = "Hello, geometry!";
    let (_, geometry) = ts.layout_single_line_with_geometry(text, &plain(), None);

    let line = only_line(&geometry);
    assert_eq!(line.char_range, 0..text.chars().count());

    // Walking the segments both checks the tiling and yields the
    // characters; the count is then the only thing left to confirm.
    let characters = characters_in_logical_order(line);
    assert_eq!(
        characters.len(),
        text.chars().count(),
        "every source character needs exactly one entry"
    );
}

#[test]
fn paragraph_line_byte_ranges_partition_the_source() {
    let mut ts = make_typesetter();
    let (_, geometry) = ts.layout_paragraph_with_geometry(PROSE, &plain(), 160.0, None);

    assert!(
        geometry.lines.len() > 1,
        "the wrap width must actually split this paragraph"
    );
    assert_eq!(geometry.source_len, PROSE.len());

    let mut next = 0usize;
    for line in &geometry.lines {
        assert_eq!(
            line.byte_range.start,
            next,
            "line {} starts at byte {} but line {} ended at {next}",
            line.index,
            line.byte_range.start,
            line.index.saturating_sub(1)
        );
        assert!(
            line.byte_range.end > line.byte_range.start,
            "line {} spans no bytes",
            line.index
        );
        next = line.byte_range.end;
    }
    assert_eq!(
        next, geometry.source_len,
        "the last line must reach the end of the source"
    );
}

#[test]
fn hard_break_is_reported_as_line_end() {
    let mut ts = make_typesetter();
    let (_, geometry) = ts.layout_paragraph_with_geometry("alpha\nbeta", &plain(), WIDE, None);

    assert_eq!(geometry.lines.len(), 2);
    assert_eq!(
        geometry.lines[0].end,
        LineEnd::HardBreak { chars: 1, bytes: 1 }
    );
    assert_eq!(geometry.lines[1].end, LineEnd::EndOfText);
}

#[test]
fn crlf_is_one_break_of_two_bytes() {
    let mut ts = make_typesetter();
    let (_, geometry) = ts.layout_paragraph_with_geometry("alpha\r\nbeta", &plain(), WIDE, None);

    assert_eq!(geometry.lines.len(), 2);
    // Two source characters and two source bytes: the caller's ranges
    // index the source, not AccessKit's one-character view of a CRLF.
    assert_eq!(
        geometry.lines[0].end,
        LineEnd::HardBreak { chars: 2, bytes: 2 }
    );
    assert_eq!(geometry.lines[1].end, LineEnd::EndOfText);
}

#[test]
fn soft_wrap_reports_softwrap() {
    let mut ts = make_typesetter();
    let (_, geometry) = ts.layout_paragraph_with_geometry(PROSE, &plain(), 140.0, None);

    assert!(
        geometry.lines.len() >= 3,
        "expected several wrapped lines, got {}",
        geometry.lines.len()
    );
    let (last, wrapped) = geometry
        .lines
        .split_last()
        .expect("the paragraph emitted at least one line");
    for line in wrapped {
        assert_eq!(
            line.end,
            LineEnd::SoftWrap,
            "line {} was broken to fit, not by the source",
            line.index
        );
    }
    assert_eq!(last.end, LineEnd::EndOfText);
}

#[test]
fn empty_line_reports_a_caret_x() {
    let mut ts = make_typesetter();
    let (_, geometry) = ts.layout_paragraph_with_geometry("a\n\nb", &plain(), WIDE, None);

    assert_eq!(geometry.lines.len(), 3);
    let blank = &geometry.lines[1];

    // The blank line carries its own hard break and nothing else, so its
    // range is one character wide rather than empty — `byte_range`
    // includes the trailing break by contract. What matters for a
    // consumer is that the line holds no text of its own.
    assert_eq!(
        blank.end,
        LineEnd::HardBreak { chars: 1, bytes: 1 },
        "the middle line is the break alone"
    );
    assert_eq!(blank.char_range, 2..3);
    assert!(
        blank.caret_x.is_finite(),
        "a line with nothing on it still has to say where a caret goes"
    );

    // The genuinely empty line — no text, no break — is the one the
    // "no segments, locate the caret with caret_x" contract is about, and
    // an empty block is where it arises.
    ts.layout_blocks(vec![make_block(1, "")]);
    ts.render();
    let lines = ts.block_line_geometry(1, "");
    assert_eq!(lines.len(), 1);
    assert!(lines[0].char_range.is_empty());
    assert!(
        lines[0].segments.is_empty(),
        "nothing to measure means no segments, not a zero-width one"
    );
    assert!(lines[0].caret_x.is_finite());
}

#[test]
fn rtl_widths_are_positive() {
    let mut ts = hebrew_typesetter();
    let (_, geometry) = ts.layout_single_line_with_geometry(SHALOM, &plain(), None);

    let line = only_line(&geometry);
    assert!(!line.segments.is_empty(), "Hebrew text must be measurable");
    for segment in &line.segments {
        assert_eq!(segment.direction, GeometryDirection::RightToLeft);
        for (i, character) in segment.characters.iter().enumerate() {
            // Deriving a width from the left-to-right delta between caret
            // stops collapsed every RTL character to zero or below.
            assert!(
                character.width > 0.0,
                "RTL character {i} reports width {}",
                character.width
            );
        }
    }
}

#[test]
fn rtl_positions_are_monotonic_from_the_leading_edge() {
    let mut ts = hebrew_typesetter();
    let (_, geometry) = ts.layout_single_line_with_geometry(SHALOM, &plain(), None);

    let line = only_line(&geometry);
    let characters = characters_in_logical_order(line);
    assert_eq!(characters.len(), SHALOM.chars().count());

    // The leading edge of an RTL segment is its right edge, so reading
    // order still runs from zero upwards.
    assert!(
        characters[0].position.abs() < 0.01,
        "the first character sits at the segment's leading edge, got {}",
        characters[0].position
    );
    for (i, pair) in characters.windows(2).enumerate() {
        assert!(
            pair[1].position >= pair[0].position,
            "position fell from {} to {} between characters {i} and {}",
            pair[0].position,
            pair[1].position,
            i + 1
        );
    }
}

#[test]
fn bidi_line_yields_one_segment_per_direction() {
    let mut ts = bilingual_typesetter();
    let text = format!("{KATABA} Rust {SALAAM}");
    let (_, geometry) = ts.layout_single_line_with_geometry(&text, &plain(), None);

    let line = only_line(&geometry);
    assert!(
        line.segments.len() > 1,
        "a line mixing Arabic and Latin reads in two directions"
    );
    for (i, pair) in line.segments.windows(2).enumerate() {
        assert_ne!(
            pair[0].direction,
            pair[1].direction,
            "segments {i} and {} read the same way and should have merged",
            i + 1
        );
    }

    // Contiguity in logical order, covering the whole line: the checker
    // walks the segments and asserts the tiling.
    let characters = characters_in_logical_order(line);
    assert_eq!(characters.len(), text.chars().count());
    assert_eq!(
        line.segments
            .last()
            .expect("segments were shown non-empty above")
            .char_range
            .end,
        line.char_range.end
    );
}

#[test]
fn trailing_ellipsis_reports_a_prefix_and_the_ellipsis_box() {
    let mut ts = make_typesetter();
    let text = "Hello, world! This is a rather long label.";

    let (full, _) = ts.layout_single_line_with_geometry(text, &plain(), None);
    // Derive the cut from the measured width rather than guessing a
    // pixel count the font might not agree with.
    let max_width = full.width / 4.0;
    let (_, geometry) = ts.layout_single_line_with_geometry(text, &plain(), Some(max_width));

    let line = only_line(&geometry);
    let truncation = line
        .truncation
        .expect("a label cut to a quarter of its width is truncated");
    assert!(
        truncation.ellipsis_x > 0.0,
        "at least one character fits before the ellipsis"
    );
    assert!(
        truncation.ellipsis_x <= max_width,
        "the ellipsis starts inside the budget, at {} of {max_width}",
        truncation.ellipsis_x
    );

    // The line stands for the whole source even though only a prefix was
    // drawn — the caller anchors the rest at the ellipsis.
    let total_chars = text.chars().count();
    assert_eq!(line.char_range, 0..total_chars);
    assert_eq!(line.byte_range, 0..text.len());

    let drawn = characters_in_logical_order(line);
    assert!(
        !drawn.is_empty() && drawn.len() < total_chars,
        "segments cover a strict prefix, got {} of {total_chars} characters",
        drawn.len()
    );
}

#[test]
fn max_lines_reports_dropped_lines() {
    let mut ts = make_typesetter();
    let (_, uncapped) = ts.layout_paragraph_with_geometry(PROSE, &plain(), 120.0, None);
    assert!(
        uncapped.lines.len() > 2,
        "the paragraph must want more lines than the cap allows"
    );

    let (_, capped) = ts.layout_paragraph_with_geometry(PROSE, &plain(), 120.0, Some(2));
    assert_eq!(capped.lines.len(), 2);
    assert!(capped.dropped_lines > 0);
    assert_eq!(capped.dropped_lines, uncapped.lines.len() - 2);
}

#[test]
fn markup_geometry_indexes_the_rendered_text_and_reports_links() {
    let markup = InlineMarkup::parse("See [docs](http://x) now");
    let mut ts = make_typesetter();

    let (_, single) = ts.layout_single_line_markup_with_geometry(&markup, &plain(), None);
    let (_, paragraph) = ts.layout_paragraph_markup_with_geometry(&markup, &plain(), WIDE, None);

    for (path, geometry) in [("single-line", &single), ("paragraph", &paragraph)] {
        let rendered = geometry
            .rendered_text
            .as_deref()
            .unwrap_or_else(|| panic!("{path} markup path must report the text its ranges index"));
        assert_eq!(
            rendered, "See docs now",
            "{path}: the markup syntax is stripped from the indexed text"
        );
        assert_eq!(geometry.source_len, rendered.len());

        for line in &geometry.lines {
            assert!(
                rendered.get(line.byte_range.clone()).is_some(),
                "{path}: line {} range {:?} does not slice the rendered text",
                line.index,
                line.byte_range
            );
            for segment in &line.segments {
                assert!(
                    rendered.get(segment.byte_range.clone()).is_some(),
                    "{path}: segment range {:?} does not slice the rendered text",
                    segment.byte_range
                );
            }
        }

        assert_eq!(geometry.links.len(), 1, "{path}: one link in the markup");
        let link = &geometry.links[0];
        assert_eq!(
            rendered
                .get(link.rendered_byte_range.clone())
                .unwrap_or_else(|| panic!("{path}: link range escapes the rendered text")),
            "docs",
            "{path}: the link range covers the label, not the syntax"
        );
        assert_eq!(link.url, "http://x", "{path}");
    }
}

#[test]
fn character_geometry_matches_segment_geometry_for_ltr() {
    let mut ts = make_typesetter();
    let text = "Hello world";
    ts.layout_blocks(vec![make_block(1, text)]);
    ts.render();

    let lines = ts.block_line_geometry(1, text);
    assert_eq!(lines.len(), 1, "the text must fit one line for this test");
    let from_segments = characters_in_logical_order(&lines[0]);

    let total_chars = text.chars().count();
    let from_range = ts.character_geometry(1, 0, total_chars);
    assert_eq!(from_range.len(), total_chars);
    assert_eq!(from_segments.len(), total_chars);

    // Both are measured from the same origin here — the range starts at
    // the line's only segment — so the two must agree outright.
    for (i, (segment_char, range_char)) in from_segments.iter().zip(&from_range).enumerate() {
        assert!(
            (segment_char.position - range_char.position).abs() < 0.01,
            "character {i}: segment says position {}, range says {}",
            segment_char.position,
            range_char.position
        );
        assert!(
            (segment_char.width - range_char.width).abs() < 0.01,
            "character {i}: segment says width {}, range says {}",
            segment_char.width,
            range_char.width
        );
    }
}

#[test]
fn character_geometry_multi_line_behaviour_is_unchanged() {
    let mut ts = make_typesetter();
    ts.set_content_width(160.0);
    ts.layout_blocks(vec![make_block(1, PROSE)]);
    ts.render();

    let lines = ts.block_line_geometry(1, PROSE);
    assert!(
        lines.len() > 1,
        "the content width must wrap this block for the test to mean anything"
    );

    let total_chars = PROSE.chars().count();
    let geometry = ts.character_geometry(1, 0, total_chars);
    assert_eq!(geometry.len(), total_chars);
    assert!(
        geometry[0].position.abs() < 0.01,
        "the range's first character sits at zero, got {}",
        geometry[0].position
    );

    // Positions have always been line-local on this path: each new line
    // restarts near its own left edge rather than accumulating down the
    // block. One drop per line boundary is what that looks like.
    let drops = geometry
        .windows(2)
        .filter(|pair| pair[1].position < pair[0].position)
        .count();
    assert_eq!(
        drops,
        lines.len() - 1,
        "expected one position drop per line boundary across {} lines",
        lines.len()
    );

    // Widths are each character's own advance now, not a delta between
    // caret stops. `>= 0.0` would not test that: the builder clamps at
    // zero, so it cannot fail. Every character of this prose is a
    // visible Latin glyph or a space, so each has an advance of its own
    // — including the last character of each wrapped line, which is
    // where a delta between caret stops has no successor to subtract and
    // collapses.
    for (i, character) in geometry.iter().enumerate() {
        assert!(
            character.width > 0.0,
            "character {i} ({:?}) reports width {}",
            PROSE.chars().nth(i),
            character.width
        );
    }
}

#[test]
fn block_line_geometry_reports_positive_widths_for_an_rtl_block() {
    // The block path and the paragraph path go through the same capture,
    // but only the block path is what the rich-text and code editors
    // consume — and it is the one `character_geometry` used to answer
    // from caret-stop deltas, which read right-to-left advances as
    // negative and clamped them to zero.
    let mut ts = hebrew_typesetter();
    ts.layout_blocks(vec![make_block(1, SHALOM)]);
    ts.render();

    let lines = ts.block_line_geometry(1, SHALOM);
    assert_eq!(lines.len(), 1, "four letters fit one line");
    let segment = lines[0]
        .segments
        .first()
        .expect("a shaped Hebrew word has one right-to-left segment");
    assert_eq!(segment.direction, GeometryDirection::RightToLeft);
    assert_eq!(segment.characters.len(), SHALOM.chars().count());
    for (i, ch) in segment.characters.iter().enumerate() {
        assert!(
            ch.width > 0.0,
            "letter {i} reports width {} — a right-to-left advance must be positive",
            ch.width
        );
    }

    let advances: f32 = segment.characters.iter().map(|c| c.width).sum();
    assert!(
        (advances - segment.rect[2]).abs() < 0.05,
        "the letters' advances ({advances}) must tile the segment box ({})",
        segment.rect[2]
    );
}

#[test]
fn block_line_geometry_stacks_lines_from_the_block_top() {
    // Line boxes are relative to the block's top edge, not to the
    // document: a consumer offsets them by the block's own origin.
    let mut ts = make_typesetter();
    ts.set_content_width(160.0);
    ts.layout_blocks(vec![make_block(1, PROSE)]);
    ts.render();

    let lines = ts.block_line_geometry(1, PROSE);
    assert!(lines.len() > 1, "the block must wrap for this test");
    assert!(
        lines[0].rect[1].abs() < 0.01,
        "the first line's box starts at the block top, not at {}",
        lines[0].rect[1]
    );
    for pair in lines.windows(2) {
        assert!(
            pair[1].rect[1] > pair[0].rect[1],
            "line {} sits at {} but line {} sits at {}",
            pair[0].index,
            pair[0].rect[1],
            pair[1].index,
            pair[1].rect[1]
        );
        assert!(
            pair[1].baseline > pair[0].baseline,
            "baselines must descend with the lines"
        );
    }
}
