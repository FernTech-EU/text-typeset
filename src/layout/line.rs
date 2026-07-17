use std::ops::Range;

use crate::shaping::run::ShapedRun;
use crate::shaping::shaper::TextDirection;

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
    /// Builds the line's caret stops `(logical_offset, x)` in visual
    /// order — direction-aware, so an RTL run's caret for its lowest
    /// offset sits at its rightmost edge — then returns the x of the stop
    /// matching `offset` (nearest by offset if there's no exact match).
    pub fn x_for_offset(&self, offset: usize) -> f32 {
        let stops = self.caret_stops();
        if stops.is_empty() {
            return 0.0;
        }
        // Exact match first (earliest/leftmost stop wins on ties).
        if let Some((_, x)) = stops.iter().find(|(o, _)| *o == offset) {
            return *x;
        }
        // No exact stop (e.g. offset inside a multi-char cluster): snap to
        // the nearest stop by logical distance.
        stops
            .iter()
            .min_by_key(|(o, _)| o.abs_diff(offset))
            .map(|(_, x)| *x)
            .unwrap_or(0.0)
    }

    /// Caret stops `(logical char offset, x)` across the line in visual
    /// (left-to-right) order. For an LTR run each glyph contributes a stop
    /// at its left edge (the glyph's own cluster) plus a trailing stop at
    /// the run's right edge; for an RTL run the leftmost edge is the
    /// trailing (highest) offset and each glyph's right edge is its own
    /// (leading) offset.
    pub(crate) fn caret_stops(&self) -> Vec<(usize, f32)> {
        let mut stops: Vec<(usize, f32)> = Vec::new();
        for run in &self.runs {
            let glyphs = &run.shaped_run.glyphs;
            if glyphs.is_empty() {
                continue;
            }
            let mut gx = run.x;
            if run.shaped_run.direction == TextDirection::RightToLeft {
                // Leftmost edge: caret after the last logical char in the run.
                stops.push((self.cluster_end(glyphs[0].cluster as usize), gx));
                for g in glyphs {
                    gx += g.x_advance;
                    stops.push((g.cluster as usize, gx));
                }
            } else {
                for g in glyphs {
                    stops.push((g.cluster as usize, gx));
                    gx += g.x_advance;
                }
                // Rightmost edge: caret after the last logical char in the run.
                let last = glyphs.last().map(|g| g.cluster as usize).unwrap_or(0);
                stops.push((self.cluster_end(last), gx));
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
