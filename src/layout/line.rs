use std::ops::Range;

use crate::shaping::run::ShapedRun;
use crate::shaping::shaper::TextDirection;
use crate::types::CursorAffinity;

/// One place the caret can sit on a line: a logical offset, the x it
/// renders at, and which side of its run produced it.
///
/// `trailing` is what makes a direction boundary resolvable. A boundary
/// offset appears twice — once as the trailing edge of the run before it
/// and once as the leading edge of the run after — and the two sit at
/// different x. Recording which is which lets the caller pick by
/// affinity instead of by whichever happened to be leftmost.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CaretStop {
    pub offset: usize,
    pub x: f32,
    /// This stop is the *end* of its run's logical extent.
    pub trailing: bool,
}

/// Choose among the stops that share one offset.
///
/// With a single candidate there is nothing to disambiguate. With two —
/// a direction boundary — `Downstream` attaches the caret to the text
/// before the offset (the trailing stop) and `Upstream` to the text
/// after it (the leading stop). Falls back to the first candidate if the
/// expected side is absent, so a malformed line still yields a caret.
fn pick_by_affinity<'a>(stops: &[&'a CaretStop], affinity: CursorAffinity) -> Option<&'a CaretStop> {
    if stops.len() < 2 {
        return stops.first().copied();
    }
    let want_trailing = affinity == CursorAffinity::Downstream;
    stops
        .iter()
        .find(|s| s.trailing == want_trailing)
        .or_else(|| stops.first())
        .copied()
}

#[derive(Clone)]
pub struct LayoutLine {
    pub runs: Vec<PositionedRun>,
    /// Baseline y relative to block top (set by block layout).
    pub y: f32,
    pub ascent: f32,
    pub descent: f32,
    pub leading: f32,
    /// Total line height: ascent + descent + leading.
    pub line_height: f32,
    /// Actual content width (sum of run advances).
    pub width: f32,
    /// Character range in the block's text.
    pub char_range: Range<usize>,
}

impl LayoutLine {
    /// End (exclusive) of the cluster starting at char offset `cluster`:
    /// the smallest distinct glyph cluster in the line strictly greater
    /// than `cluster`, or the line's `char_range.end` if none is larger.
    ///
    /// Clusters across the whole line are the complete set of logical
    /// char-offset boundaries, so the next-larger one is exactly where
    /// `cluster`'s char span ends — true for LTR, RTL, and multi-char
    /// (ligature) clusters alike.
    pub(crate) fn cluster_end(&self, cluster: usize) -> usize {
        let mut best: Option<usize> = None;
        for run in &self.runs {
            for g in &run.shaped_run.glyphs {
                let c = g.cluster as usize;
                if c > cluster {
                    best = Some(best.map_or(c, |b| b.min(c)));
                }
            }
        }
        best.unwrap_or(self.char_range.end)
    }

    /// Find the x coordinate for a char offset within this line.
    ///
    /// Equivalent to [`Self::x_for_offset_with_affinity`] with the
    /// default (downstream) affinity — the caret attaches to the text
    /// *before* the offset.
    pub fn x_for_offset(&self, offset: usize) -> f32 {
        self.x_for_offset_with_affinity(offset, CursorAffinity::default())
    }

    /// Find the x coordinate for a char offset, disambiguating a
    /// direction boundary with `affinity`.
    ///
    /// Builds the line's caret stops in visual order — direction-aware,
    /// so an RTL run's caret for its lowest offset sits at its rightmost
    /// edge — then returns the x of the stop matching `offset`.
    ///
    /// One offset can produce **two** stops. Where an LTR run meets an
    /// RTL one, the boundary offset is both the trailing edge of the run
    /// before it and the leading edge of the run after it, and those sit
    /// at completely different x — often at opposite ends of the line.
    /// Neither is "the" answer: which one the writer means depends on
    /// which side they arrived from, so it is the caller's to say.
    /// Picking the leftmost, as this used to, put the caret at the far
    /// end of the line whenever the seam ran the other way.
    ///
    /// `Downstream` attaches the caret to the text before the offset (the
    /// trailing stop), `Upstream` to the text after it (the leading
    /// stop) — the same "before or after" question affinity already
    /// answers at a soft-wrap boundary.
    pub fn x_for_offset_with_affinity(&self, offset: usize, affinity: CursorAffinity) -> f32 {
        let stops = self.caret_stops();
        if stops.is_empty() {
            return 0.0;
        }

        let exact: Vec<&CaretStop> = stops.iter().filter(|s| s.offset == offset).collect();
        if let Some(stop) = pick_by_affinity(&exact, affinity) {
            return stop.x;
        }

        // No exact stop (e.g. offset inside a multi-char cluster): snap to
        // the nearest stop by logical distance.
        stops
            .iter()
            .min_by_key(|s| s.offset.abs_diff(offset))
            .map(|s| s.x)
            .unwrap_or(0.0)
    }

    /// Whether `offset` sits on a direction boundary within this line —
    /// i.e. whether [`Self::x_for_offset_with_affinity`] would return
    /// different x for the two affinities.
    ///
    /// The widget layer needs this to know when moving the caret across
    /// a seam has to flip affinity rather than leave it alone.
    pub fn is_direction_boundary(&self, offset: usize) -> bool {
        let stops = self.caret_stops();
        let mut xs = stops.iter().filter(|s| s.offset == offset).map(|s| s.x);
        let Some(first) = xs.next() else {
            return false;
        };
        xs.any(|x| x != first)
    }

    /// Caret stops across the line in visual (left-to-right) order. For
    /// an LTR run each glyph contributes a stop at its left edge (the
    /// glyph's own cluster) plus a trailing stop at the run's right edge;
    /// for an RTL run the leftmost edge is the trailing (highest) offset
    /// and each glyph's right edge is its own (leading) offset.
    pub(crate) fn caret_stops(&self) -> Vec<CaretStop> {
        let mut stops: Vec<CaretStop> = Vec::new();
        for run in &self.runs {
            let glyphs = &run.shaped_run.glyphs;
            if glyphs.is_empty() {
                continue;
            }
            let mut gx = run.x;
            if run.shaped_run.direction == TextDirection::RightToLeft {
                // Leftmost edge: caret after the last logical char in the run.
                stops.push(CaretStop {
                    offset: self.cluster_end(glyphs[0].cluster as usize),
                    x: gx,
                    trailing: true,
                });
                for g in glyphs {
                    gx += g.x_advance;
                    stops.push(CaretStop {
                        offset: g.cluster as usize,
                        x: gx,
                        trailing: false,
                    });
                }
            } else {
                for g in glyphs {
                    stops.push(CaretStop {
                        offset: g.cluster as usize,
                        x: gx,
                        trailing: false,
                    });
                    gx += g.x_advance;
                }
                // Rightmost edge: caret after the last logical char in the run.
                let last = glyphs.last().map(|g| g.cluster as usize).unwrap_or(0);
                stops.push(CaretStop {
                    offset: self.cluster_end(last),
                    x: gx,
                    trailing: true,
                });
            }
        }
        stops
    }
}

#[derive(Clone)]
pub struct PositionedRun {
    pub shaped_run: ShapedRun,
    /// X offset from the left edge of the content area.
    pub x: f32,
    /// Decoration flags for this run.
    pub decorations: RunDecorations,
}

/// Text decoration flags and metadata carried from the source TextFormat.
#[derive(Clone, Debug, Default)]
pub struct RunDecorations {
    pub underline_style: crate::types::UnderlineStyle,
    pub overline: bool,
    pub strikeout: bool,
    pub is_link: bool,
    /// Text foreground color (RGBA). None means default (black).
    pub foreground_color: Option<[f32; 4]>,
    /// Underline color (RGBA). None means use foreground_color.
    pub underline_color: Option<[f32; 4]>,
    /// Text-level background highlight color (RGBA). None means transparent.
    pub background_color: Option<[f32; 4]>,
    /// Hyperlink destination URL.
    pub anchor_href: Option<String>,
    /// Tooltip text.
    pub tooltip: Option<String>,
    /// Vertical alignment (normal, superscript, subscript).
    pub vertical_alignment: crate::types::VerticalAlignment,
}
