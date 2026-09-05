//! Per-line, per-character geometry capture.
//!
//! The layout engine already knows, for every laid-out line, where each
//! glyph cluster sits and which direction it reads in. Accessibility
//! needs the same information one *character* at a time: AccessKit's
//! `Role::TextRun` carries `character_positions` and `character_widths`
//! so a screen reader can route a braille cell to a word, a magnifier
//! can follow the review cursor, and `AXBoundsForRange` can answer.
//!
//! This module turns a [`LayoutLine`] into a [`LineGeometry`]. It reads
//! the same cluster data `caret_stops` reads, but reports *extents*
//! rather than caret positions: extents have no affinity to resolve, and
//! an RTL run's characters are measured from its right edge so positions
//! rise in reading order in both directions.

use crate::layout::line::{LayoutLine, PositionedRun};
use crate::shaping::shaper::TextDirection;
use crate::types::{CharacterGeometry, GeometryDirection, LineEnd, LineGeometry, LineSegment};

/// Byte offset of every character boundary in `text`, plus `text.len()`.
///
/// Built once per layout call and shared by every line, so mapping a
/// char offset to a byte offset is an index rather than a rescan.
pub(crate) fn char_byte_table(text: &str) -> Vec<usize> {
    let mut table = Vec::with_capacity(text.chars().count() + 1);
    table.extend(text.char_indices().map(|(i, _)| i));
    table.push(text.len());
    table
}

/// Byte offset of character `index`, clamped to the end of the text.
pub(crate) fn byte_at(table: &[usize], index: usize) -> usize {
    match table.get(index) {
        Some(b) => *b,
        None => table.last().copied().unwrap_or(0),
    }
}

/// A segment before its positions are rebased to the leading edge.
///
/// `chars` holds, per character in logical order, the **absolute** x of
/// its cluster's leading edge (its left edge in LTR, its right edge in
/// RTL) and its own advance. Keeping x absolute is what lets two
/// segments merge without re-deriving anything — including across the
/// gaps justification opens between runs.
pub(crate) struct RawSegment {
    pub char_range: std::ops::Range<usize>,
    pub direction: GeometryDirection,
    pub left: f32,
    pub width: f32,
    pub chars: Vec<(f32, f32)>,
}

impl RawSegment {
    /// Per-character geometry rebased to this segment's leading edge.
    pub(crate) fn characters(&self) -> Vec<CharacterGeometry> {
        let rtl = self.direction == GeometryDirection::RightToLeft;
        let right = self.left + self.width;
        let mut previous = 0.0f32;
        self.chars
            .iter()
            .map(|&(leading, advance)| {
                let raw = if rtl {
                    right - leading
                } else {
                    leading - self.left
                };
                // Positions are an exact running sum, so this only ever absorbs
                // float noise — but the contract (non-decreasing, never
                // negative) is one AccessKit consumers index with.
                let position = raw.max(previous).max(0.0);
                previous = position;
                CharacterGeometry {
                    position,
                    width: advance.max(0.0),
                }
            })
            .collect()
    }
}

/// Turn one positioned run into a raw segment, or `None` when it has no
/// glyphs to measure.
fn raw_segment_of_run(line: &LayoutLine, run: &PositionedRun) -> Option<RawSegment> {
    let glyphs = &run.shaped_run.glyphs;
    if glyphs.is_empty() {
        return None;
    }
    let rtl = run.shaped_run.direction == TextDirection::RightToLeft;

    // Cluster groups in visual order: (cluster, visual left edge, advance).
    // Consecutive glyphs sharing a cluster are one group — a ligature, or a
    // base plus its combining marks.
    let mut groups: Vec<(usize, f32, f32)> = Vec::new();
    let mut pen = run.x;
    for glyph in glyphs {
        let cluster = glyph.cluster as usize;
        match groups.last_mut() {
            Some(last) if last.0 == cluster => last.2 += glyph.x_advance,
            _ => groups.push((cluster, pen, glyph.x_advance)),
        }
        pen += glyph.x_advance;
    }
    let left = run.x;
    let width = pen - run.x;

    // Logical order. An RTL run stores its glyphs visually, so its clusters
    // descend across the array; sorting is the direction-agnostic way to put
    // the groups back in reading order.
    groups.sort_by_key(|(cluster, _, _)| *cluster);

    let first_char = groups.first()?.0;
    let last_cluster = groups.last()?.0;
    // Where this run's logical extent ends: the next cluster anywhere on the
    // line, or the line's end. That is what makes a trailing ligature — or a
    // trailing newline, which belongs to the last run — cover its full span.
    let logical_end = line.cluster_end(last_cluster).max(last_cluster + 1);

    let mut chars: Vec<(f32, f32)> = Vec::with_capacity(logical_end.saturating_sub(first_char));
    for (i, &(cluster, visual_left, advance)) in groups.iter().enumerate() {
        let next = groups
            .get(i + 1)
            .map(|(c, _, _)| *c)
            .unwrap_or(logical_end)
            .max(cluster + 1);
        let leading = if rtl {
            visual_left + advance
        } else {
            visual_left
        };
        chars.push((leading, advance));
        // A cluster spanning several characters gives its whole advance to
        // the first of them; the rest sit at the same leading edge with no
        // advance of their own (a ligature interior, a combining mark).
        for _ in (cluster + 1)..next {
            chars.push((leading, 0.0));
        }
    }

    let char_range = first_char..first_char + chars.len();
    Some(RawSegment {
        char_range,
        direction: if rtl {
            GeometryDirection::RightToLeft
        } else {
            GeometryDirection::LeftToRight
        },
        left,
        width,
        chars,
    })
}

/// Every direction-uniform stretch of a line, in logical order.
///
/// Runs that read in the same direction and are contiguous in the text
/// are fused: a line split by font fallback, by a markup span boundary
/// or by a spell-check range is still one stretch of reading, and only a
/// direction change is a real segment boundary.
pub(crate) fn raw_segments(line: &LayoutLine) -> Vec<RawSegment> {
    let mut raws: Vec<RawSegment> = line
        .runs
        .iter()
        .filter_map(|r| raw_segment_of_run(line, r))
        .collect();
    raws.sort_by_key(|s| s.char_range.start);

    let mut merged: Vec<RawSegment> = Vec::with_capacity(raws.len());
    for seg in raws {
        if let Some(last) = merged.last_mut()
            && last.direction == seg.direction
            && last.char_range.end == seg.char_range.start
        {
            let left = last.left.min(seg.left);
            let right = (last.left + last.width).max(seg.left + seg.width);
            last.left = left;
            last.width = right - left;
            last.char_range.end = seg.char_range.end;
            last.chars.extend(seg.chars);
            continue;
        }
        merged.push(seg);
    }
    merged
}

/// How a line ends, read from the source text rather than from the
/// glyphs — a hard break may or may not have produced a glyph.
fn line_end(text: &str, table: &[usize], char_end: usize, total_chars: usize) -> LineEnd {
    let byte_end = byte_at(table, char_end);
    let upto = &text[..byte_end];
    if upto.ends_with("\r\n") {
        LineEnd::HardBreak { chars: 2, bytes: 2 }
    } else if upto.ends_with('\n') {
        LineEnd::HardBreak { chars: 1, bytes: 1 }
    } else if char_end >= total_chars {
        LineEnd::EndOfText
    } else {
        LineEnd::SoftWrap
    }
}

/// Capture one laid-out line.
///
/// `y_top` is the top of the line box in the coordinate space the caller
/// reports its glyph quads in; `text` and `table` describe the text the
/// line's char offsets index.
pub(crate) fn line_geometry(
    line: &LayoutLine,
    index: usize,
    y_top: f32,
    text: &str,
    table: &[usize],
    total_chars: usize,
) -> LineGeometry {
    let merged = raw_segments(line);
    let line_height = line.line_height;
    let segments: Vec<LineSegment> = merged
        .iter()
        .map(|seg| LineSegment {
            byte_range: byte_at(table, seg.char_range.start)..byte_at(table, seg.char_range.end),
            char_range: seg.char_range.clone(),
            direction: seg.direction,
            rect: [seg.left, y_top, seg.width, line_height],
            characters: seg.characters(),
        })
        .collect();

    let left = merged.iter().map(|s| s.left).fold(f32::INFINITY, f32::min);
    let right = merged
        .iter()
        .map(|s| s.left + s.width)
        .fold(f32::NEG_INFINITY, f32::max);
    let (x, width) = if left.is_finite() && right.is_finite() {
        (left, right - left)
    } else {
        (line.empty_caret_x, 0.0)
    };

    let char_range = line.char_range.start.min(total_chars)..line.char_range.end.min(total_chars);
    LineGeometry {
        index,
        byte_range: byte_at(table, char_range.start)..byte_at(table, char_range.end),
        end: line_end(text, table, char_range.end, total_chars),
        char_range,
        rect: [x, y_top, width, line_height],
        baseline: y_top + line.ascent,
        caret_x: line.empty_caret_x,
        segments,
        truncation: None,
    }
}

/// Character index of the character containing byte offset `byte`.
pub(crate) fn char_at(table: &[usize], byte: usize) -> usize {
    match table.binary_search(&byte) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    }
}

/// Geometry for one shaped line laid out at the origin — the
/// single-line layout path, which never builds a [`LayoutLine`] of its
/// own.
///
/// `emit_limit` is the number of glyphs the caller actually drew, when
/// it truncated; `truncation` locates the ellipsis it drew instead. The
/// reported line always covers the **whole** source: when the drawn
/// glyphs form a logical prefix the segments cover that prefix and the
/// caller can anchor the rest at the ellipsis, and when they do not — a
/// bidirectional line, where a visual prefix is not a logical one —
/// there are no segments at all rather than misleading ones.
pub(crate) fn single_line_geometry(
    text: &str,
    runs: &[crate::shaping::run::ShapedRun],
    emit_limit: Option<usize>,
    truncation: Option<crate::types::LineTruncation>,
    ascent: f32,
    descent: f32,
    leading: f32,
) -> crate::types::LayoutGeometry {
    use crate::layout::line::RunDecorations;

    let table = char_byte_table(text);
    let total_chars = table.len().saturating_sub(1);

    let mut positioned: Vec<PositionedRun> = Vec::with_capacity(runs.len());
    let mut dropped_min: Option<usize> = None;
    let mut emitted_max: Option<usize> = None;
    let mut pen_x = 0.0f32;
    let mut emitted = 0usize;

    for run in runs {
        let run_x = pen_x;
        let mut glyphs = Vec::with_capacity(run.glyphs.len());
        for glyph in &run.glyphs {
            // Shaping reports clusters as byte offsets into the run's own
            // slice; the line's coordinates are char offsets into the whole
            // text.
            let cluster = char_at(&table, run.text_range.start + glyph.cluster as usize);
            if emit_limit.is_some_and(|limit| emitted >= limit) {
                dropped_min = Some(dropped_min.map_or(cluster, |m: usize| m.min(cluster)));
                continue;
            }
            let mut g = glyph.clone();
            g.cluster = cluster as u32;
            pen_x += g.x_advance;
            emitted += 1;
            emitted_max = Some(emitted_max.map_or(cluster, |m: usize| m.max(cluster)));
            glyphs.push(g);
        }
        if glyphs.is_empty() {
            continue;
        }
        let advance: f32 = glyphs.iter().map(|g| g.x_advance).sum();
        let mut shaped = run.clone();
        shaped.glyphs = glyphs;
        shaped.advance_width = advance;
        positioned.push(PositionedRun {
            shaped_run: shaped,
            x: run_x,
            decorations: RunDecorations::default(),
        });
    }

    let drawn_is_logical_prefix = match (emitted_max, dropped_min) {
        (_, None) => true,
        (Some(last_drawn), Some(first_dropped)) => last_drawn < first_dropped,
        (None, Some(_)) => true,
    };
    let covered = dropped_min.unwrap_or(total_chars);
    let line_height = ascent + descent + leading;
    let width = pen_x;

    let line = LayoutLine {
        runs: if drawn_is_logical_prefix {
            positioned
        } else {
            Vec::new()
        },
        y: 0.0,
        ascent,
        descent,
        leading,
        line_height,
        width,
        char_range: 0..covered,
        empty_caret_x: 0.0,
    };

    let mut geometry = line_geometry(&line, 0, 0.0, text, &table, total_chars);
    // The line stands for the whole source even when only part of it was
    // drawn — the caller anchors the undrawn tail at the ellipsis.
    geometry.char_range = 0..total_chars;
    geometry.byte_range = 0..text.len();
    geometry.end = line_end(text, &table, total_chars, total_chars);
    geometry.truncation = truncation;
    if geometry.segments.is_empty() {
        geometry.rect = [0.0, 0.0, width, line_height];
    }

    crate::types::LayoutGeometry {
        lines: vec![geometry],
        dropped_lines: 0,
        source_len: text.len(),
        rendered_text: None,
        links: Vec::new(),
    }
}

/// Geometry for a run of laid-out lines that stack from `origin_y`.
///
/// Shared by the paragraph layout paths (`origin_y = 0.0`, lines stacked
/// by the caller) and by the block path, where each line already carries
/// its own baseline.
pub(crate) fn stacked_lines_geometry(
    lines: &[LayoutLine],
    text: &str,
    tops: impl Fn(usize, &LayoutLine) -> f32,
) -> Vec<LineGeometry> {
    let table = char_byte_table(text);
    let total_chars = table.len().saturating_sub(1);
    lines
        .iter()
        .enumerate()
        .map(|(i, line)| line_geometry(line, i, tops(i, line), text, &table, total_chars))
        .collect()
}

/// Per-character geometry across a block's lines, for a character range.
///
/// One entry per character in `char_start..char_end`, with `position`
/// measured from the first character of the range and `width` the
/// character's own advance — taken from the segment it belongs to, so an
/// RTL character reports a positive width rather than the zero a
/// left-to-right delta between caret stops would give.
pub(crate) fn character_geometry_over_lines(
    lines: &[LayoutLine],
    char_start: usize,
    char_end: usize,
) -> Vec<CharacterGeometry> {
    if char_start >= char_end {
        return Vec::new();
    }
    // char offset -> (leading edge x, own advance), gathered across every
    // line the range touches.
    let mut map: std::collections::BTreeMap<usize, (f32, f32)> = std::collections::BTreeMap::new();
    for line in lines {
        if line.char_range.end <= char_start || line.char_range.start >= char_end {
            continue;
        }
        for seg in raw_segments(line) {
            for (i, &(leading, advance)) in seg.chars.iter().enumerate() {
                let c = seg.char_range.start + i;
                if c >= char_start && c < char_end {
                    map.entry(c).or_insert((leading, advance));
                }
            }
        }
    }

    let base = map.values().next().map(|(x, _)| *x).unwrap_or(0.0);
    let mut out = Vec::with_capacity(char_end - char_start);
    let mut carry = base;
    for c in char_start..char_end {
        match map.get(&c) {
            Some(&(leading, advance)) => {
                carry = leading + advance;
                out.push(CharacterGeometry {
                    position: leading - base,
                    width: advance.max(0.0),
                });
            }
            // A character no glyph covers — a hard break, a zero-width
            // control — sits where the previous one ended and takes no
            // space of its own.
            None => out.push(CharacterGeometry {
                position: carry - base,
                width: 0.0,
            }),
        }
    }
    out
}

/// Geometry for a wrapped paragraph whose lines stack from the origin at
/// a constant `line_height` — the shape both paragraph layout paths
/// produce.
pub(crate) fn paragraph_geometry(
    lines: &[LayoutLine],
    text: &str,
    line_height: f32,
    dropped_lines: usize,
    rendered_text: Option<String>,
    links: Vec<crate::types::LinkGeometry>,
) -> crate::types::LayoutGeometry {
    crate::types::LayoutGeometry {
        lines: stacked_lines_geometry(lines, text, |i, _| i as f32 * line_height),
        dropped_lines,
        source_len: text.len(),
        rendered_text,
        links,
    }
}
