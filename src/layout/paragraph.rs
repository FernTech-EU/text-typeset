use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::OnceLock;

use icu_segmenter::LineSegmenter;
use icu_segmenter::options::LineBreakOptions;

use crate::layout::line::{LayoutLine, PositionedRun, RunDecorations};
use crate::shaping::run::{ShapedGlyph, ShapedRun};
use crate::shaping::shaper::{FontMetricsPx, TextDirection};

/// How [`break_into_lines`] should treat the order of the runs it is given.
///
/// The two layout paths differ here and getting it wrong reverses text
/// twice. The single-line path runs the bidi algorithm itself and shapes
/// in display order, so its runs must be left alone; the block path hands
/// over logical-order runs tagged with embedding levels and needs each
/// line reordered once the breaks are known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOrder {
    /// Runs already arrive in visual order — do not reorder them.
    AlreadyVisual,
    /// Runs are in logical order; reorder each line per UAX #9 rule L2,
    /// and resolve `Start`/`End` alignment against this base direction.
    Logical(TextDirection),
}

/// Whether a break opportunity *must* be taken (LB4/LB5 hard line
/// break) or *may* be taken (regular UAX #14 break opportunity).
///
/// `icu_segmenter::LineSegmenter` doesn't distinguish the two — it just
/// emits byte offsets — so we classify each emitted offset ourselves by
/// looking at the line-break property of the preceding code point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BreakOpportunity {
    Allowed,
    Mandatory,
}

/// Shared compiled-data line segmenter. `LineSegmenter::new_auto`
/// returns a `LineSegmenterBorrowed<'static>` (Copy, statically-baked
/// CLDR data), but constructing it still touches the option-parsing
/// path — cache once to keep `break_into_lines` allocation-free.
fn line_segmenter() -> icu_segmenter::LineSegmenterBorrowed<'static> {
    static CELL: OnceLock<icu_segmenter::LineSegmenterBorrowed<'static>> = OnceLock::new();
    *CELL.get_or_init(|| LineSegmenter::new_auto(LineBreakOptions::default()))
}

/// UAX #14 LB4/LB5: a break at `byte_offset` is mandatory iff the
/// immediately-preceding code point is one of the hard line break
/// characters (LF, CR, NEL, VT, FF, LS, PS). The CR+LF sequence is
/// handled implicitly: the segmenter emits a single break after the
/// LF, and the char before that offset is `\n` — mandatory.
fn is_mandatory_break_at(text: &str, byte_offset: usize) -> bool {
    if byte_offset == 0 {
        return false;
    }
    let preceding = &text[..byte_offset];
    preceding
        .chars()
        .next_back()
        .is_some_and(crate::types::is_hard_break)
}

/// Enumerate UAX #14 break opportunities in `text` and classify each
/// as `Allowed` or `Mandatory`. Replaces the previous
/// `unicode_linebreak::linebreaks(text)` call site one-to-one.
fn enumerate_breaks(text: &str) -> Vec<(usize, BreakOpportunity)> {
    line_segmenter()
        .segment_str(text)
        .map(|byte_offset| {
            let kind = if is_mandatory_break_at(text, byte_offset) {
                BreakOpportunity::Mandatory
            } else {
                BreakOpportunity::Allowed
            };
            (byte_offset, kind)
        })
        .collect()
}

/// Byte offsets in `text` where a hyphenated line break may occur and a
/// hyphen glyph should be rendered: Knuth-Liang dictionary points inside
/// words (in `lang_code`, an ISO 639-1 code) plus soft-hyphen (U+00AD)
/// positions.
///
/// Returned offsets are "break before this byte" positions, matching the
/// UAX #14 offsets from [`enumerate_breaks`]. Soft hyphens are honored
/// regardless of language; dictionary breaks apply only when the
/// language's patterns are compiled in (`hypher::Lang::from_iso` resolves
/// it) — otherwise it gracefully degrades to soft-hyphen-only.
fn hyphenation_breaks(text: &str, lang_code: [u8; 2]) -> Vec<usize> {
    use hypher::hyphenate;

    let mut offsets = Vec::new();

    // Soft hyphens: break after the U+00AD so the hyphen renders at line end.
    for (idx, ch) in text.char_indices() {
        if ch == '\u{00AD}' {
            offsets.push(idx + ch.len_utf8());
        }
    }

    // Dictionary hyphenation, word by word — only if the language resolves.
    if let Some(lang) = hypher::Lang::from_iso(lang_code) {
        let mut word_start: Option<usize> = None;
        let flush = |start: usize, end: usize, offsets: &mut Vec<usize>| {
            let word = &text[start..end];
            // Skip trivially short words; `hyphenate` would yield no interior
            // breaks anyway, and this avoids the call overhead.
            if word.chars().count() < 5 {
                return;
            }
            let mut pos = start;
            let mut syllables = hyphenate(word, lang).peekable();
            while let Some(syl) = syllables.next() {
                pos += syl.len();
                // A break sits between syllables, not after the last one.
                if syllables.peek().is_some() {
                    offsets.push(pos);
                }
            }
        };
        for (idx, ch) in text.char_indices() {
            if ch.is_alphabetic() {
                word_start.get_or_insert(idx);
            } else if let Some(start) = word_start.take() {
                flush(start, idx, &mut offsets);
            }
        }
        if let Some(start) = word_start.take() {
            flush(start, text.len(), &mut offsets);
        }
    }

    offsets.sort_unstable();
    offsets.dedup();
    offsets
}

/// Map hyphenation byte offsets to glyph indices (first glyph whose
/// cluster is `>=` the offset), mirroring [`map_breaks_to_glyph_indices`].
fn map_offsets_to_glyph_indices(flat: &[FlatGlyph], offsets: &[usize]) -> HashSet<usize> {
    let mut set = HashSet::new();
    let mut cursor = 0usize;
    for &byte_offset in offsets {
        while cursor < flat.len() && (flat[cursor].cluster as usize) < byte_offset {
            cursor += 1;
        }
        set.insert(cursor.min(flat.len()));
    }
    set
}

/// Append a rendered hyphen glyph to the end of a line that broke at a
/// hyphenation point, accounting for its advance in the run and line
/// widths. The hyphen inherits the last glyph's cluster so caret/hit
/// math treats it as part of the final character.
fn append_hyphen(line: &mut LayoutLine, hyphen: &ShapedGlyph) {
    if let Some(run) = line.runs.last_mut() {
        let mut g = hyphen.clone();
        g.cluster = run
            .shaped_run
            .glyphs
            .last()
            .map(|gl| gl.cluster)
            .unwrap_or(0);
        run.shaped_run.glyphs.push(g);
        run.shaped_run.advance_width += hyphen.x_advance;
        line.width += hyphen.x_advance;
    }
}

/// Convert a byte offset within a UTF-8 string to a char offset.
///
/// Clamps to `text.len()` and rounds down to the nearest char boundary
/// if `byte_offset` lands inside a multi-byte character. HarfBuzz can
/// emit cluster values that don't coincide with UTF-8 char boundaries
/// (ligature splits, fallback shaping), so callers must never assume
/// cluster values are well-aligned.
fn byte_offset_to_char_offset(text: &str, byte_offset: usize) -> usize {
    let mut off = byte_offset.min(text.len());
    while off > 0 && !text.is_char_boundary(off) {
        off -= 1;
    }
    text[..off].chars().count()
}

/// Everything line wrapping needs to hyphenate: the pre-shaped hyphen
/// glyph to append at a break and the ISO 639-1 language for the
/// dictionary. Passed as `Some` only when hyphenation is enabled.
pub struct Hyphenator {
    /// Hyphen (`-`) glyph in the run's font, appended at hyphenated breaks.
    pub glyph: ShapedGlyph,
    /// ISO 639-1 language code for the Knuth-Liang dictionary.
    pub language: [u8; 2],
}

/// Text alignment within a line.
///
/// `Left`/`Right` are absolute; `Start`/`End` are relative to the
/// paragraph's base direction and are what an *unset* alignment should
/// use. Keeping both lets a writer who explicitly chose "flush left" keep
/// that in an RTL paragraph, while a paragraph nobody has aligned simply
/// follows its own direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Alignment {
    /// Leading edge: left in an LTR paragraph, right in an RTL one.
    #[default]
    Start,
    /// Trailing edge: right in an LTR paragraph, left in an RTL one.
    End,
    Left,
    Right,
    Center,
    Justify,
}

impl Alignment {
    /// Resolve the direction-relative variants against a base direction.
    ///
    /// Always returns an absolute alignment, so line layout never has to
    /// think about direction again.
    pub fn resolve_for(self, base: TextDirection) -> Alignment {
        let rtl = base == TextDirection::RightToLeft;
        match self {
            Alignment::Start if rtl => Alignment::Right,
            Alignment::Start => Alignment::Left,
            Alignment::End if rtl => Alignment::Left,
            Alignment::End => Alignment::Right,
            absolute => absolute,
        }
    }
}

/// Put a line's runs into visual order per UAX #9 rule L2, then re-lay
/// their x positions left to right.
///
/// Line breaking is a logical operation and runs arrive in logical order,
/// so this is where a mixed-direction line finally becomes visual. It has
/// to run per line rather than per paragraph: reordering a paragraph
/// before breaking it would let a wrap point fall in the middle of an
/// already-reversed span and scramble every line after it.
///
/// The runs keep their glyphs untouched — harfrust already emitted those
/// in visual order within each run. Only the runs' relative order and
/// their x origins change.
fn reorder_line_visually(line: &mut LayoutLine) {
    if line.runs.len() < 2 {
        return;
    }
    // Cheap scan before any allocation: with nothing right-to-left on
    // the line there is nothing for rule L2 to reverse, and that is
    // every line of an ordinary Latin document. Building the level and
    // permutation vectors first would allocate twice per line on every
    // relayout just to conclude the same thing.
    if line.runs.iter().all(|r| r.shaped_run.bidi_level % 2 == 0) {
        return;
    }

    let levels: Vec<u8> = line.runs.iter().map(|r| r.shaped_run.bidi_level).collect();

    let order = crate::shaping::shaper::visual_order(&levels);
    if order.iter().copied().eq(0..order.len()) {
        return; // already visual — the common all-LTR case
    }

    // The line's left edge, which `build_line` set to the first-line
    // indent. Re-laying from here keeps the indent intact.
    let origin = line.runs.iter().map(|r| r.x).fold(f32::INFINITY, f32::min);

    let mut reordered: Vec<crate::layout::line::PositionedRun> = Vec::with_capacity(order.len());
    let mut taken: Vec<Option<crate::layout::line::PositionedRun>> =
        line.runs.drain(..).map(Some).collect();
    for logical_idx in order {
        if let Some(run) = taken[logical_idx].take() {
            reordered.push(run);
        }
    }

    let mut x = origin;
    for run in &mut reordered {
        run.x = x;
        x += run.shaped_run.advance_width;
    }
    line.runs = reordered;
}

/// Break shaped runs into lines that fit within `available_width`.
///
/// Strategy: shape-first-then-break.
/// 1. The caller has already shaped the full paragraph into one or more ShapedRuns.
/// 2. We use unicode-linebreak to find break opportunities in the original text.
/// 3. We map break positions to glyph boundaries via cluster values.
/// 4. Greedy line wrapping: accumulate glyph advances, break at the last
///    allowed opportunity before exceeding the width.
/// 5. Apply alignment per line.
#[allow(clippy::too_many_arguments)]
pub fn break_into_lines(
    runs: Vec<ShapedRun>,
    text: &str,
    available_width: f32,
    alignment: Alignment,
    first_line_indent: f32,
    metrics: &FontMetricsPx,
    hyphenator: Option<Hyphenator>,
    run_order: RunOrder,
) -> Vec<LayoutLine> {
    // Where a glyph-less line puts the caret. The alignment pass below only
    // moves runs, so an empty line must carry this explicitly.
    let empty_caret_x = {
        let base = match run_order {
            RunOrder::Logical(base) => base,
            RunOrder::AlreadyVisual => TextDirection::LeftToRight,
        };
        empty_line_caret_x(alignment, base, first_line_indent, available_width)
    };

    if runs.is_empty() || text.is_empty() {
        // Empty paragraph: produce one empty line for the block to have height
        return vec![make_empty_line(metrics, 0..0, empty_caret_x)];
    }

    // Flatten all glyphs into a single sequence with their run association, in
    // logical order when the runs come in logical order (see `flatten_runs`).
    let flat = flatten_runs(&runs, matches!(run_order, RunOrder::Logical(_)));
    if flat.is_empty() {
        return vec![make_empty_line(metrics, 0..0, empty_caret_x)];
    }

    // Get UAX #14 break opportunities (byte offsets in text), each
    // classified as Allowed or Mandatory.
    let breaks: Vec<(usize, BreakOpportunity)> = enumerate_breaks(text);

    // Build sets of allowed and mandatory break positions (glyph indices)
    let (break_points, mandatory_breaks) = map_breaks_to_glyph_indices(&flat, &breaks);

    // Hyphenation break candidates (glyph indices). A break here needs a
    // trailing hyphen glyph and must reserve its advance in the fit check.
    let hyphen_points = if let Some(h) = &hyphenator {
        map_offsets_to_glyph_indices(&flat, &hyphenation_breaks(text, h.language))
    } else {
        HashSet::new()
    };
    let hyphen_adv = hyphenator
        .as_ref()
        .map(|h| h.glyph.x_advance)
        .unwrap_or(0.0);

    // Greedy line wrapping
    let mut lines = Vec::new();
    // Whether each line ends at a hard break, which justification leaves alone.
    let mut hard_ends: Vec<bool> = Vec::new();
    let mut line_start_glyph = 0usize;
    let mut line_width = 0.0f32;
    // Last break opportunity within the current line: (glyph index, whether
    // breaking there renders a hyphen).
    let mut last_break: Option<(usize, bool)> = None;
    // First line may be indented; subsequent lines use full width
    let mut effective_width = available_width - first_line_indent;

    for i in 0..flat.len() {
        let glyph_advance = flat[i].x_advance;
        line_width += glyph_advance;

        // Check for mandatory break — O(1) lookup — and where it falls in the text.
        let mandatory_at = mandatory_breaks.get(&(i + 1)).copied();
        let is_mandatory = mandatory_at.is_some();

        let exceeds_width = line_width > effective_width && line_start_glyph < i;

        if is_mandatory || exceeds_width {
            let (break_at, needs_hyphen) = if is_mandatory {
                (i + 1, false)
            } else if let Some((bp, hy)) = last_break {
                if bp > line_start_glyph {
                    (bp, hy)
                } else {
                    (i + 1, false) // emergency break -no opportunity found
                }
            } else {
                (i + 1, false) // emergency break -no break opportunities at all
            };

            let indent = if lines.is_empty() {
                first_line_indent
            } else {
                0.0
            };
            let mut line = build_line(
                &runs,
                &flat,
                line_start_glyph,
                break_at,
                metrics,
                indent,
                text,
                if is_mandatory { mandatory_at } else { None },
            );
            if needs_hyphen && let Some(h) = &hyphenator {
                append_hyphen(&mut line, &h.glyph);
            }
            lines.push(line);
            hard_ends.push(is_mandatory);

            line_start_glyph = break_at;
            // Subsequent lines use full available width
            effective_width = available_width;
            // Re-accumulate width for glyphs already scanned past the break
            line_width = 0.0;
            for j in break_at..=i {
                if j < flat.len() {
                    line_width += flat[j].x_advance;
                }
            }
            last_break = None;
        }

        // Update the break opportunity AFTER the width check so that a break
        // discovered at this glyph does not clobber the previous one when the
        // width is already exceeded. Hyphenation points are checked first so
        // a soft hyphen (which is also a UAX #14 opportunity) renders its
        // hyphen; they only count when the hyphen itself still fits.
        let at = i + 1;
        if hyphen_points.contains(&at) && line_width + hyphen_adv <= effective_width {
            last_break = Some((at, true));
        } else if break_points.contains(&at) {
            last_break = Some((at, false));
        }
    }

    // Remaining glyphs form the last line
    if line_start_glyph < flat.len() {
        let line = build_line(
            &runs,
            &flat,
            line_start_glyph,
            flat.len(),
            metrics,
            if lines.is_empty() {
                first_line_indent
            } else {
                0.0
            },
            text,
            None,
        );
        lines.push(line);
        hard_ends.push(false);
    }

    // A hard break at the very end starts a line of its own, empty, where the
    // caret goes after Shift+Enter at the end of a paragraph: without it the
    // caret stayed at the end of the line it had just left, and the paragraph
    // a line short.
    if text
        .chars()
        .next_back()
        .is_some_and(crate::types::is_hard_break)
    {
        let end = byte_offset_to_char_offset(text, text.len());
        let base = match run_order {
            RunOrder::Logical(base) => base,
            RunOrder::AlreadyVisual => TextDirection::LeftToRight,
        };
        let caret_x = empty_line_caret_x(alignment, base, 0.0, available_width);
        lines.push(make_empty_line(metrics, end..end, caret_x));
        hard_ends.push(false);
    }

    // Put each line's runs into visual order before anything reads their
    // x positions. Alignment shifts every run by the same amount so it
    // does not care about order, but `justify_line` re-lays runs
    // sequentially from the vector and would otherwise justify a
    // mixed-direction line in logical order.
    let base_direction = match run_order {
        RunOrder::Logical(base) => {
            for line in &mut lines {
                reorder_line_visually(line);
            }
            base
        }
        // Already visual: reordering here would reverse the caller's work.
        RunOrder::AlreadyVisual => TextDirection::LeftToRight,
    };

    // Apply alignment. An unset alignment follows the paragraph's base
    // direction, so an RTL paragraph right-aligns without the host having
    // to translate direction into alignment itself.
    let alignment = alignment.resolve_for(base_direction);
    let rtl_paragraph = base_direction == TextDirection::RightToLeft;
    let effective_width = available_width;
    let last_idx = lines.len().saturating_sub(1);
    for (i, line) in lines.iter_mut().enumerate() {
        let indent = if i == 0 { first_line_indent } else { 0.0 };
        // A first-line indent insets from the paragraph's *leading* edge,
        // which in an RTL paragraph is the right one. `build_line` always
        // insets from the left, so undo that here and let the narrowed
        // `line_avail` below carry the inset over to the right instead.
        if rtl_paragraph && indent != 0.0 {
            for run in &mut line.runs {
                run.x -= indent;
            }
        }
        let line_avail = effective_width - indent;
        match alignment {
            // `resolve_for` maps these onto Left/Right, so they cannot
            // reach here — but fail soft rather than panicking in a
            // layout pass that runs on every keystroke.
            Alignment::Start | Alignment::End | Alignment::Left => {
                // Runs already sit at the indent, except in an RTL
                // paragraph where the block above moved them to 0 so the
                // inset could go to the trailing edge. An explicit Left
                // means the writer wants flush-left text, and the indent
                // still belongs on the paragraph's leading edge, so put
                // it back.
                if rtl_paragraph && indent != 0.0 {
                    for run in &mut line.runs {
                        run.x += indent;
                    }
                }
            }
            Alignment::Right => {
                let shift = (line_avail - line.width).max(0.0);
                for run in &mut line.runs {
                    run.x += shift;
                }
            }
            Alignment::Center => {
                let shift = ((line_avail - line.width) / 2.0).max(0.0);
                for run in &mut line.runs {
                    run.x += shift;
                }
            }
            Alignment::Justify => {
                // Don't justify the last line, nor one a hard break ends: a
                // line of verse or of a letter set apart by Shift+Enter keeps
                // its words where they are, as a browser and Typst set it.
                if i < last_idx && !hard_ends[i] && line.width > 0.0 {
                    justify_line(line, line_avail, text);
                }
            }
        }
    }

    if lines.is_empty() {
        lines.push(make_empty_line(metrics, 0..0, empty_caret_x));
    }

    // Convert glyph cluster values from byte offsets to char offsets.
    // This must happen AFTER alignment because justify_line needs byte
    // offsets to find space characters in the original text.
    for line in &mut lines {
        for run in &mut line.runs {
            for glyph in &mut run.shaped_run.glyphs {
                glyph.cluster = byte_offset_to_char_offset(text, glyph.cluster as usize) as u32;
            }
        }
    }

    lines
}

/// A flattened glyph with enough info to map back to runs.
struct FlatGlyph {
    x_advance: f32,
    cluster: u32,
    run_index: usize,
    glyph_index_in_run: usize,
}

/// The glyphs of `runs` as one sequence, each with its run and its index in it.
///
/// With `logical`, the runs arrive in logical order and so does the sequence: a
/// right-to-left run's glyphs, which shaping stores in visual order (its last
/// character first), are walked from its end. Line breaking reads the sequence as the
/// text runs, so walked visually such a run was broken from its logical end: a wrapped
/// right-to-left paragraph set its last words on its first line, and a break inside
/// one (a line separator, which stays in the run it sits in) mapped to its first
/// glyph and was never taken. A line still takes a contiguous slice of each run's
/// glyphs, since the logical stretch of a run is a contiguous stretch of it visually.
fn flatten_runs(runs: &[ShapedRun], logical: bool) -> Vec<FlatGlyph> {
    let mut flat = Vec::new();
    for (run_idx, run) in runs.iter().enumerate() {
        // Offset cluster values from fragment-text space to block-text space.
        // rustybuzz assigns clusters as byte offsets within the fragment text (0-based),
        // but unicode-linebreak returns byte offsets in the full block text.
        let cluster_offset = run.text_range.start as u32;
        let push = |flat: &mut Vec<FlatGlyph>, glyph_idx: usize| {
            flat.push(FlatGlyph {
                x_advance: run.glyphs[glyph_idx].x_advance,
                cluster: run.glyphs[glyph_idx].cluster + cluster_offset,
                run_index: run_idx,
                glyph_index_in_run: glyph_idx,
            });
        };
        if logical && run.direction == TextDirection::RightToLeft {
            for glyph_idx in (0..run.glyphs.len()).rev() {
                push(&mut flat, glyph_idx);
            }
        } else {
            for glyph_idx in 0..run.glyphs.len() {
                push(&mut flat, glyph_idx);
            }
        }
    }
    flat
}

/// Map unicode-linebreak byte offsets to glyph indices using a merged walk.
/// Both `flat` (by cluster) and `breaks` (by byte offset) are sorted,
/// so a single O(b + m) pass replaces the previous O(b × m) approach.
///
/// Returns (break_points: HashSet<glyph_idx>, mandatory_breaks: glyph_idx → the byte
/// offset of the break in the text).
fn map_breaks_to_glyph_indices(
    flat: &[FlatGlyph],
    breaks: &[(usize, BreakOpportunity)],
) -> (HashSet<usize>, HashMap<usize, usize>) {
    let mut break_points = HashSet::new();
    let mut mandatory_breaks = HashMap::new();
    let mut glyph_cursor = 0usize;

    for &(byte_offset, opportunity) in breaks {
        // Advance glyph cursor to the first glyph whose cluster >= byte_offset
        while glyph_cursor < flat.len() && (flat[glyph_cursor].cluster as usize) < byte_offset {
            glyph_cursor += 1;
        }
        let glyph_idx = if glyph_cursor < flat.len() {
            glyph_cursor
        } else {
            flat.len()
        };
        break_points.insert(glyph_idx);
        if opportunity == BreakOpportunity::Mandatory {
            mandatory_breaks.insert(glyph_idx, byte_offset);
        }
    }

    (break_points, mandatory_breaks)
}

/// Build a LayoutLine from a glyph range within the flat sequence.
///
/// `hard_end` is where a hard break the line ends at falls in the text, the byte
/// after the break: the line's text ends there. Read from the next glyph instead, it
/// overshot into the next line when that one opens with a right-to-left run, whose
/// first glyph carries its *last* cluster, and the break was lost.
#[allow(clippy::too_many_arguments)]
fn build_line(
    runs: &[ShapedRun],
    flat: &[FlatGlyph],
    start: usize,
    end: usize,
    metrics: &FontMetricsPx,
    indent: f32,
    text: &str,
    hard_end: Option<usize>,
) -> LayoutLine {
    // Group consecutive glyphs by run_index to reconstruct PositionedRuns. A group is
    // a contiguous slice of its run's glyphs, walked forwards, or backwards for a
    // right-to-left run laid out in logical order (see `flatten_runs`), so its slice is
    // the span of the indices it holds.
    let mut positioned_runs = Vec::new();
    let mut x = indent;
    let mut i = start;
    while i < end {
        let run_idx = flat[i].run_index;
        let (mut lo, mut hi) = (flat[i].glyph_index_in_run, flat[i].glyph_index_in_run);
        let mut j = i + 1;
        while j < end && flat[j].run_index == run_idx {
            lo = lo.min(flat[j].glyph_index_in_run);
            hi = hi.max(flat[j].glyph_index_in_run);
            j += 1;
        }
        if let Some((pr, advance)) = extract_sub_run(runs, run_idx, lo, hi + 1) {
            positioned_runs.push(PositionedRun {
                decorations: RunDecorations {
                    underline_style: pr.underline_style,
                    overline: pr.overline,
                    strikeout: pr.strikeout,
                    is_link: pr.is_link,
                    foreground_color: pr.foreground_color,
                    underline_color: pr.underline_color,
                    background_color: pr.background_color,
                    anchor_href: pr.anchor_href.clone(),
                    tooltip: pr.tooltip.clone(),
                    vertical_alignment: pr.vertical_alignment,
                },
                shaped_run: pr,
                x,
            });
            x += advance;
        }
        i = j;
    }

    let width = x - indent;

    // Compute char range from cluster values.
    // Clusters from rustybuzz are byte offsets — convert to char offsets
    // so that positions match text-document's character-based coordinates.
    //
    // Glyphs are in *visual* order, so for an RTL run `flat[start]` holds
    // the largest cluster and `flat[end-1]` the smallest. Take the min/max
    // over the line's glyphs instead of trusting the array ends, so the
    // logical range is correct for both directions.
    let byte_start = flat[start..end.min(flat.len())]
        .iter()
        .map(|g| g.cluster as usize)
        .min()
        .unwrap_or(0);
    let byte_end = if let Some(hard_end) = hard_end {
        hard_end
    } else if end >= flat.len() {
        // Last glyph reaches the end of the input text. Always snap to the
        // full length: a trailing ligature glyph may cover several source
        // chars, so `max_cluster + 1` would be wrong.
        text.len()
    } else {
        // Next visual glyph's cluster bounds an LTR line exactly; for a
        // wrapped RTL line take whichever is larger so the logical end
        // still covers the line's highest cluster.
        let line_max = flat[start..end]
            .iter()
            .map(|g| g.cluster as usize)
            .max()
            .unwrap_or(0);
        (flat[end].cluster as usize).max(line_max)
    };
    let char_start = byte_offset_to_char_offset(text, byte_start);
    let char_end = byte_offset_to_char_offset(text, byte_end);
    let hard_break_chars = match text
        .get(byte_start..byte_end)
        .and_then(crate::types::LineEnd::hard_break_ending)
    {
        Some(crate::types::LineEnd::HardBreak { chars, .. }) => chars as usize,
        _ => 0,
    };

    // Expand line height for inline images taller than the font ascent
    let mut ascent = metrics.ascent;
    for run in &positioned_runs {
        if run.shaped_run.image_name.is_some() && run.shaped_run.image_height > ascent {
            ascent = run.shaped_run.image_height;
        }
    }
    let line_height = ascent + metrics.descent + metrics.leading;

    LayoutLine {
        runs: positioned_runs,
        y: 0.0, // will be set by the caller (block layout)
        ascent,
        descent: metrics.descent,
        leading: metrics.leading,
        width,
        char_range: char_start..char_end,
        line_height,
        // Degenerate fallback only: a built line normally derives caret
        // positions from its runs, but if every run came out glyph-less
        // the caret still has the line's start to sit at.
        empty_caret_x: indent,
        hard_break_chars,
    }
}

/// Extract a sub-run (slice of glyphs) from a ShapedRun.
/// Cluster values are offset to block-text space (adding text_range.start).
fn extract_sub_run(
    runs: &[ShapedRun],
    run_index: usize,
    glyph_start: usize,
    glyph_end: usize,
) -> Option<(ShapedRun, f32)> {
    let run = &runs[run_index];
    let end = glyph_end.min(run.glyphs.len());
    if glyph_start >= end {
        return None;
    }
    let cluster_offset = run.text_range.start as u32;
    let mut sub_glyphs = run.glyphs[glyph_start..end].to_vec();
    // Offset cluster values from fragment-local to block-text space
    for g in &mut sub_glyphs {
        g.cluster += cluster_offset;
    }
    let advance: f32 = sub_glyphs.iter().map(|g| g.x_advance).sum();

    let sub_run = ShapedRun {
        font_face_id: run.font_face_id,
        size_px: run.size_px,
        weight: run.weight,
        italic: run.italic,
        glyphs: sub_glyphs,
        advance_width: advance,
        text_range: run.text_range.clone(),
        direction: run.direction,
        bidi_level: run.bidi_level,
        underline_style: run.underline_style,
        overline: run.overline,
        strikeout: run.strikeout,
        is_link: run.is_link,
        foreground_color: run.foreground_color,
        underline_color: run.underline_color,
        background_color: run.background_color,
        anchor_href: run.anchor_href.clone(),
        tooltip: run.tooltip.clone(),
        vertical_alignment: run.vertical_alignment,
        image_name: run.image_name.clone(),
        image_height: run.image_height,
    };
    Some((sub_run, advance))
}

fn make_empty_line(
    metrics: &FontMetricsPx,
    char_range: Range<usize>,
    empty_caret_x: f32,
) -> LayoutLine {
    LayoutLine {
        runs: Vec::new(),
        y: 0.0,
        ascent: metrics.ascent,
        descent: metrics.descent,
        leading: metrics.leading,
        width: 0.0,
        char_range,
        line_height: metrics.ascent + metrics.descent + metrics.leading,
        empty_caret_x,
        hard_break_chars: 0,
    }
}

/// The x where the caret sits on a line with no glyphs — i.e. where the
/// first typed character's glyph would land. Mirrors the run-shifting
/// arithmetic of `break_into_lines`' alignment pass with a line width of
/// zero, so an empty paragraph's caret cannot disagree with where the
/// first keystroke lays text out (the "caret jumps on the first
/// character" bug).
fn empty_line_caret_x(
    alignment: Alignment,
    base_direction: TextDirection,
    first_line_indent: f32,
    available_width: f32,
) -> f32 {
    let rtl = base_direction == TextDirection::RightToLeft;
    let indent = first_line_indent;
    let line_avail = (available_width - indent).max(0.0);
    match alignment.resolve_for(base_direction) {
        // In an RTL paragraph the alignment pass first moves runs to the
        // left edge so the indent insets from the *right*; Right then
        // shifts by the remaining width. Width zero leaves the caret at
        // `line_avail` (RTL) or the full width (LTR).
        Alignment::Right => {
            if rtl {
                line_avail
            } else {
                available_width
            }
        }
        Alignment::Center => {
            let shift = line_avail / 2.0;
            if rtl { shift } else { indent + shift }
        }
        // Justify never justifies the last line, and a lone line is the
        // last line, so runs keep their unshifted positions: the indent
        // for LTR; the left edge for RTL, where the alignment pass moved
        // them and Justify does not put the indent back.
        Alignment::Justify => {
            if rtl {
                0.0
            } else {
                indent
            }
        }
        // `resolve_for` maps Start/End onto Left/Right, so what remains is
        // Left — which keeps runs at the indent in both directions.
        _ => indent,
    }
}

/// Distribute extra space among word gaps for justification.
///
/// Finds space glyphs (cluster mapping to ' ') across all runs and
/// increases their x_advance proportionally. Then recomputes run x positions.
fn justify_line(line: &mut LayoutLine, target_width: f32, text: &str) {
    let extra = target_width - line.width;
    if extra <= 0.0 {
        return;
    }

    // Count space glyphs across all runs
    let mut space_count = 0usize;
    for run in &line.runs {
        for glyph in &run.shaped_run.glyphs {
            let byte_offset = glyph.cluster as usize;
            if let Some(ch) = text.get(byte_offset..).and_then(|s| s.chars().next())
                && ch == ' '
            {
                space_count += 1;
            }
        }
    }

    if space_count == 0 {
        return;
    }

    let extra_per_space = extra / space_count as f32;

    // Increase x_advance of space glyphs
    for run in &mut line.runs {
        for glyph in &mut run.shaped_run.glyphs {
            let byte_offset = glyph.cluster as usize;
            if let Some(ch) = text.get(byte_offset..).and_then(|s| s.chars().next())
                && ch == ' '
            {
                glyph.x_advance += extra_per_space;
            }
        }
        // Recompute run advance width
        run.shaped_run.advance_width = run.shaped_run.glyphs.iter().map(|g| g.x_advance).sum();
    }

    // Recompute run x positions (runs follow each other)
    let first_x = line.runs.first().map(|r| r.x).unwrap_or(0.0);
    let mut x = first_x;
    for run in &mut line.runs {
        run.x = x;
        x += run.shaped_run.advance_width;
    }

    line.width = target_width;
}
