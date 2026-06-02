use crate::font::registry::FontRegistry;
use crate::font::resolve::{ResolvedFont, resolve_font};
use crate::layout::line::LayoutLine;
use crate::layout::paragraph::{Alignment, break_into_lines};
use crate::shaping::run::ShapedRun;
use crate::shaping::shaper::{
    FontMetricsPx, TextDirection, font_metrics_px, shape_text, shape_text_with_fallback,
    to_harfrust_features,
};

/// Computed layout for a single block (paragraph).
#[derive(Clone)]
pub struct BlockLayout {
    pub block_id: usize,
    /// Document character position of the block start.
    pub position: usize,
    /// Laid out lines within the block.
    pub lines: Vec<LayoutLine>,
    /// Top edge relative to document start (set by flow layout).
    pub y: f32,
    /// Total height: top_margin + sum(line heights) + bottom_margin.
    pub height: f32,
    pub top_margin: f32,
    pub bottom_margin: f32,
    pub left_margin: f32,
    pub right_margin: f32,
    /// Shaped list marker (positioned to the left of the content area).
    /// None if the block is not a list item.
    pub list_marker: Option<ShapedListMarker>,
    /// Block background color (RGBA). None means transparent.
    pub background_color: Option<[f32; 4]>,
}

/// A shaped list marker ready for rendering.
#[derive(Clone)]
pub struct ShapedListMarker {
    pub run: ShapedRun,
    /// X position of the marker (relative to block left edge, before content indent).
    pub x: f32,
}

/// Parameters extracted from text-document's BlockFormat / TextFormat.
/// This is a plain struct so block layout doesn't depend on text-document types.
#[derive(Clone)]
pub struct BlockLayoutParams {
    pub block_id: usize,
    pub position: usize,
    pub text: String,
    pub fragments: Vec<FragmentParams>,
    pub alignment: Alignment,
    pub top_margin: f32,
    pub bottom_margin: f32,
    pub left_margin: f32,
    pub right_margin: f32,
    pub text_indent: f32,
    /// List marker text (e.g., "1.", "•", "a)"). Empty if not a list item.
    pub list_marker: String,
    /// Additional left indent for list items (in pixels).
    pub list_indent: f32,
    /// Tab stop positions in pixels from the left margin.
    pub tab_positions: Vec<f32>,
    /// Line height multiplier. 1.0 = normal (from font metrics), 1.5 = 150%, 2.0 = double.
    /// None means use font metrics (ascent + descent + leading).
    pub line_height_multiplier: Option<f32>,
    /// If true, prevent line wrapping. The entire block is one long line.
    pub non_breakable_lines: bool,
    /// Checkbox marker: None = no checkbox, Some(false) = unchecked, Some(true) = checked.
    pub checkbox: Option<bool>,
    /// Block background color (RGBA). None means transparent.
    pub background_color: Option<[f32; 4]>,
}

/// A text fragment with its formatting parameters.
#[derive(Clone)]
pub struct FragmentParams {
    pub text: String,
    /// **Byte** offset of this fragment's first character inside the
    /// owning block's text. Lifted into glyph clusters by
    /// `paragraph::flatten_runs` so glyph clusters
    /// can be compared directly against `unicode-linebreak` break
    /// positions (also bytes) and against the block-level text used
    /// for `byte_offset_to_char_offset` conversion. Hosts threading
    /// text-document `FragmentContent` through the bridge must
    /// translate the char-based `FragmentContent::offset` into bytes
    /// before assigning here.
    pub offset: usize,
    pub length: usize,
    pub font_family: Option<String>,
    pub font_weight: Option<u32>,
    pub font_bold: Option<bool>,
    pub font_italic: Option<bool>,
    pub font_point_size: Option<u32>,
    pub underline_style: crate::types::UnderlineStyle,
    pub overline: bool,
    pub strikeout: bool,
    pub is_link: bool,
    /// Extra space added after each glyph (in pixels). From TextFormat::letter_spacing.
    pub letter_spacing: f32,
    /// Extra space added after space glyphs (in pixels). From TextFormat::word_spacing.
    pub word_spacing: f32,
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
    /// If Some, this fragment represents an inline image placeholder.
    pub image_name: Option<String>,
    /// Image width in pixels. Only meaningful when image_name is Some.
    pub image_width: f32,
    /// Image height in pixels. Only meaningful when image_name is Some.
    pub image_height: f32,
    /// Discretionary OpenType features to toggle during shaping. Empty =
    /// font defaults. See [`crate::types::FontFeature`].
    pub features: Vec<crate::types::FontFeature>,
}

/// Lay out a single block: resolve fonts, shape fragments, break into lines.
///
/// `scale_factor` is the device pixel ratio. Layout output is always in
/// logical pixels; the scale factor affects shaping/rasterization precision.
pub fn layout_block(
    registry: &FontRegistry,
    params: &BlockLayoutParams,
    available_width: f32,
    scale_factor: f32,
) -> BlockLayout {
    let effective_left_margin = params.left_margin + params.list_indent;
    let content_width = (available_width - effective_left_margin - params.right_margin).max(0.0);

    // Resolve fonts and shape each fragment
    let mut shaped_runs = Vec::new();
    let mut default_metrics: Option<FontMetricsPx> = None;

    for frag in &params.fragments {
        // Inline image: create a synthetic run with one placeholder glyph
        if let Some(ref image_name) = frag.image_name {
            use crate::shaping::run::{ShapedGlyph, ShapedRun};
            let image_glyph = ShapedGlyph {
                glyph_id: 0,
                cluster: 0,
                x_advance: frag.image_width,
                y_advance: 0.0,
                x_offset: 0.0,
                y_offset: 0.0,
                font_face_id: crate::types::FontFaceId(0),
            };
            let run = ShapedRun {
                font_face_id: crate::types::FontFaceId(0),
                size_px: 0.0,
                weight: 400,
                glyphs: vec![image_glyph],
                advance_width: frag.image_width,
                text_range: frag.offset..frag.offset + frag.text.len(),
                underline_style: frag.underline_style,
                overline: false,
                strikeout: false,
                is_link: frag.is_link,
                foreground_color: None,
                underline_color: None,
                background_color: None,
                anchor_href: frag.anchor_href.clone(),
                tooltip: frag.tooltip.clone(),
                vertical_alignment: crate::types::VerticalAlignment::Normal,
                image_name: Some(image_name.clone()),
                image_height: frag.image_height,
            };
            shaped_runs.push(run);
            continue;
        }

        // Scale font size for superscript/subscript
        let font_point_size = match frag.vertical_alignment {
            crate::types::VerticalAlignment::SuperScript
            | crate::types::VerticalAlignment::SubScript => frag
                .font_point_size
                .map(|s| ((s as f32 * 0.65) as u32).max(1)),
            crate::types::VerticalAlignment::Normal => frag.font_point_size,
        };

        let resolved = resolve_font(
            registry,
            frag.font_family.as_deref(),
            frag.font_weight,
            frag.font_bold,
            frag.font_italic,
            font_point_size,
            scale_factor,
        );

        if let Some(resolved) = resolved {
            // Capture default metrics from the first resolved font
            if default_metrics.is_none() {
                default_metrics = font_metrics_px(registry, &resolved);
            }

            let features = to_harfrust_features(&frag.features);
            if let Some(mut run) = shape_text_with_fallback(
                registry,
                &resolved,
                &frag.text,
                frag.offset,
                TextDirection::Auto,
                &features,
            ) {
                run.underline_style = frag.underline_style;
                run.overline = frag.overline;
                run.strikeout = frag.strikeout;
                run.is_link = frag.is_link;
                run.foreground_color = frag.foreground_color;
                run.underline_color = frag.underline_color;
                run.background_color = frag.background_color;
                run.anchor_href = frag.anchor_href.clone();
                run.tooltip = frag.tooltip.clone();
                run.vertical_alignment = frag.vertical_alignment;

                // Apply letter_spacing and word_spacing post-shaping
                if frag.letter_spacing != 0.0 || frag.word_spacing != 0.0 {
                    apply_spacing(&mut run, &frag.text, frag.letter_spacing, frag.word_spacing);
                }

                // Apply tab stops
                if !params.tab_positions.is_empty() {
                    apply_tab_stops(&mut run, &frag.text, &params.tab_positions);
                }

                shaped_runs.push(run);
            }
        }
    }

    // Fallback metrics if no fragments resolved
    let metrics = default_metrics.unwrap_or_else(|| get_default_metrics(registry, scale_factor));

    // Non-breakable lines: use infinite width to prevent wrapping
    let wrap_width = if params.non_breakable_lines {
        f32::INFINITY
    } else {
        content_width
    };

    // Break shaped runs into lines
    let mut lines = break_into_lines(
        shaped_runs,
        &params.text,
        wrap_width,
        params.alignment,
        params.text_indent,
        &metrics,
    );

    // Apply line height multiplier
    let line_height_mul = params.line_height_multiplier.unwrap_or(1.0).max(0.1);

    // Compute y positions for each line (relative to block content top)
    let mut y = 0.0f32;
    for line in &mut lines {
        if line_height_mul != 1.0 {
            line.line_height *= line_height_mul;
        }
        line.y = y + line.ascent; // y is the baseline position
        y += line.line_height;
    }

    let content_height = y;
    let total_height = params.top_margin + content_height + params.bottom_margin;

    // Shape list marker or checkbox marker
    let list_marker = if params.checkbox.is_some() {
        shape_checkbox_marker(registry, &metrics, params, scale_factor)
    } else if !params.list_marker.is_empty() {
        shape_list_marker(registry, &metrics, params, scale_factor)
    } else {
        None
    };

    BlockLayout {
        block_id: params.block_id,
        position: params.position,
        lines,
        y: 0.0, // set by flow layout
        height: total_height,
        top_margin: params.top_margin,
        bottom_margin: params.bottom_margin,
        left_margin: effective_left_margin,
        right_margin: params.right_margin,
        list_marker,
        background_color: params.background_color,
    }
}

/// A resolved paint-only color overlay span for one character range of a block.
///
/// `char_start`/`char_end` are **block-relative character offsets** — the same
/// space as the post-layout `ShapedGlyph::cluster` values (see
/// `break_into_lines`, which converts clusters to char offsets). Each field is
/// `None` when the overlay does not override it (the base run's value is kept).
/// Applying paint spans never changes glyph geometry, advances, or line breaks
/// — only color / decoration attributes — so the layout does not reflow.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PaintSpan {
    pub char_start: usize,
    pub char_end: usize,
    pub foreground_color: Option<[f32; 4]>,
    pub underline_color: Option<[f32; 4]>,
    pub background_color: Option<[f32; 4]>,
    pub underline_style: Option<crate::types::UnderlineStyle>,
    pub overline: Option<bool>,
    pub strikeout: Option<bool>,
}

/// The effective set of overrides for one glyph, used to group consecutive
/// glyphs that share the same paint result into a single output run.
#[derive(Clone, Default, PartialEq)]
struct PaintOverride {
    foreground_color: Option<[f32; 4]>,
    underline_color: Option<[f32; 4]>,
    background_color: Option<[f32; 4]>,
    underline_style: Option<crate::types::UnderlineStyle>,
    overline: Option<bool>,
    strikeout: Option<bool>,
}

impl PaintOverride {
    fn is_noop(&self) -> bool {
        *self == PaintOverride::default()
    }

    /// Merge the overlapping spans covering `char_off` (last span wins per
    /// field). Overlay spans from `extract_paint_spans` are already disjoint,
    /// but last-wins keeps this correct for arbitrary inputs.
    fn for_char(char_off: usize, spans: &[PaintSpan]) -> Self {
        let mut o = PaintOverride::default();
        for s in spans {
            if s.char_start <= char_off && char_off < s.char_end {
                if s.foreground_color.is_some() {
                    o.foreground_color = s.foreground_color;
                }
                if s.underline_color.is_some() {
                    o.underline_color = s.underline_color;
                }
                if s.background_color.is_some() {
                    o.background_color = s.background_color;
                }
                if s.underline_style.is_some() {
                    o.underline_style = s.underline_style;
                }
                if s.overline.is_some() {
                    o.overline = s.overline;
                }
                if s.strikeout.is_some() {
                    o.strikeout = s.strikeout;
                }
            }
        }
        o
    }

    /// Apply this override onto a positioned run segment, writing color /
    /// decoration fields on BOTH the shaped run and its duplicated
    /// `RunDecorations` (the renderer reads glyph color from the former and
    /// decoration rects from the latter). `None` fields keep the base value.
    fn apply(&self, run: &mut crate::layout::line::PositionedRun) {
        if let Some(c) = self.foreground_color {
            run.shaped_run.foreground_color = Some(c);
            run.decorations.foreground_color = Some(c);
        }
        if let Some(c) = self.underline_color {
            run.shaped_run.underline_color = Some(c);
            run.decorations.underline_color = Some(c);
        }
        if let Some(c) = self.background_color {
            run.shaped_run.background_color = Some(c);
            run.decorations.background_color = Some(c);
        }
        if let Some(s) = self.underline_style {
            run.shaped_run.underline_style = s;
            run.decorations.underline_style = s;
        }
        if let Some(b) = self.overline {
            run.shaped_run.overline = b;
            run.decorations.overline = b;
        }
        if let Some(b) = self.strikeout {
            run.shaped_run.strikeout = b;
            run.decorations.strikeout = b;
        }
    }
}

/// Apply paint-only color spans to a base [`BlockLayout`], returning a recolored
/// clone. The base is left untouched.
///
/// The result has byte-identical glyph positions, advances, line breaks, line
/// widths, and block height to `base` — only color / decoration attributes
/// differ. This is the "recolor without reshape/reflow" fast path: a run is
/// split into segments at paint-span boundaries (snapped to glyph/cluster
/// boundaries, never mid-cluster) and each segment's color fields are set.
/// Splitting a run never alters any glyph advance, so line widths are preserved.
///
/// Empty `spans` returns an exact (color-preserving) clone of `base`.
pub fn apply_paint_spans(base: &BlockLayout, spans: &[PaintSpan]) -> BlockLayout {
    let mut out = base.clone();
    if spans.is_empty() {
        return out;
    }
    for line in &mut out.lines {
        let mut new_runs: Vec<crate::layout::line::PositionedRun> =
            Vec::with_capacity(line.runs.len());
        for run in line.runs.drain(..) {
            recolor_run_into(run, spans, &mut new_runs);
        }
        line.runs = new_runs;
    }
    out
}

/// Split `run` at paint-span boundaries and push the recolored segment(s) onto
/// `out`. Image / glyph-less runs are passed through unchanged (paint overlays
/// never recolor images).
fn recolor_run_into(
    run: crate::layout::line::PositionedRun,
    spans: &[PaintSpan],
    out: &mut Vec<crate::layout::line::PositionedRun>,
) {
    if run.shaped_run.glyphs.is_empty() || run.shaped_run.image_name.is_some() {
        out.push(run);
        return;
    }

    // Per-glyph effective override, in glyph (visual) order. Works for LTR and
    // RTL alike: we group by adjacency in the glyph array, not by char order.
    let overrides: Vec<PaintOverride> = run
        .shaped_run
        .glyphs
        .iter()
        .map(|g| PaintOverride::for_char(g.cluster as usize, spans))
        .collect();

    // Fast path: the whole run shares one override (the common case, and the
    // only case when `spans` doesn't touch this run — then it's a no-op). Keep
    // the base `advance_width` exactly so a cleared/uncovered run is identical.
    if overrides.iter().all(|o| *o == overrides[0]) {
        let mut seg = run;
        overrides[0].apply(&mut seg);
        out.push(seg);
        return;
    }

    // Split into maximal runs of equal override.
    let glyphs = run.shaped_run.glyphs.clone();
    let mut seg_x = run.x;
    let mut start = 0usize;
    while start < glyphs.len() {
        let ov = &overrides[start];
        let mut end = start + 1;
        while end < glyphs.len() && overrides[end] == *ov {
            end += 1;
        }
        let seg_glyphs: Vec<crate::shaping::run::ShapedGlyph> = glyphs[start..end].to_vec();
        let seg_advance: f32 = seg_glyphs.iter().map(|g| g.x_advance).sum();
        let mut shaped = run.shaped_run.clone();
        shaped.glyphs = seg_glyphs;
        shaped.advance_width = seg_advance;
        let mut seg = crate::layout::line::PositionedRun {
            shaped_run: shaped,
            x: seg_x,
            decorations: run.decorations.clone(),
        };
        if !ov.is_noop() {
            ov.apply(&mut seg);
        }
        out.push(seg);
        seg_x += seg_advance;
        start = end;
    }
}

/// Add letter_spacing (to all glyphs) and word_spacing (to space glyphs).
fn apply_spacing(run: &mut ShapedRun, text: &str, letter_spacing: f32, word_spacing: f32) {
    let mut extra_advance = 0.0f32;
    for glyph in &mut run.glyphs {
        glyph.x_advance += letter_spacing;
        extra_advance += letter_spacing;

        // Add word_spacing to space characters.
        // Detect spaces by mapping cluster back to the text.
        if word_spacing != 0.0 {
            let byte_offset = glyph.cluster as usize;
            if let Some(ch) = text.get(byte_offset..).and_then(|s| s.chars().next())
                && ch == ' '
            {
                glyph.x_advance += word_spacing;
                extra_advance += word_spacing;
            }
        }
    }
    run.advance_width += extra_advance;
}

/// Shape the list marker text and position it in the indent area.
fn shape_list_marker(
    registry: &FontRegistry,
    _metrics: &FontMetricsPx,
    params: &BlockLayoutParams,
    scale_factor: f32,
) -> Option<ShapedListMarker> {
    // Use the default font for the marker
    let resolved = resolve_font(registry, None, None, None, None, None, scale_factor)?;
    let run = shape_text(registry, &resolved, &params.list_marker, 0)?;

    // Position the marker: right-aligned within the indent area, with a small gap
    let gap = 4.0; // pixels between marker and content
    let marker_x = params.left_margin + params.list_indent - run.advance_width - gap;
    let marker_x = marker_x.max(params.left_margin);

    Some(ShapedListMarker { run, x: marker_x })
}

/// Expand tab character advances to reach the next tab stop position.
fn apply_tab_stops(run: &mut ShapedRun, text: &str, tab_positions: &[f32]) {
    let default_tab = 48.0; // default tab width if no stops defined
    let mut pen_x = 0.0f32;

    for glyph in &mut run.glyphs {
        let byte_offset = glyph.cluster as usize;
        if let Some(ch) = text.get(byte_offset..).and_then(|s| s.chars().next())
            && ch == '\t'
        {
            // Find the next tab stop after the current pen position
            let next_stop = tab_positions
                .iter()
                .find(|&&stop| stop > pen_x + 1.0)
                .copied()
                .unwrap_or_else(|| {
                    // Past all defined stops: use default tab increments
                    let last = tab_positions.last().copied().unwrap_or(0.0);
                    let increment = if tab_positions.len() >= 2 {
                        tab_positions[1] - tab_positions[0]
                    } else {
                        default_tab
                    };
                    let mut stop = last + increment;
                    while stop <= pen_x + 1.0 {
                        stop += increment;
                    }
                    stop
                });

            let tab_advance = next_stop - pen_x;
            let delta = tab_advance - glyph.x_advance;
            glyph.x_advance = tab_advance;
            run.advance_width += delta;
        }
        pen_x += glyph.x_advance;
    }
}

/// Shape a checkbox marker (unchecked or checked) for rendering in the margin.
fn shape_checkbox_marker(
    registry: &FontRegistry,
    _metrics: &FontMetricsPx,
    params: &BlockLayoutParams,
    scale_factor: f32,
) -> Option<ShapedListMarker> {
    let checked = params.checkbox?;
    let marker_text = if checked { "\u{2611}" } else { "\u{2610}" }; // ballot box with/without check

    let resolved = resolve_font(registry, None, None, None, None, None, scale_factor)?;
    let run = shape_text(registry, &resolved, marker_text, 0)?;

    // If the font doesn't have the ballot box characters, use ASCII fallback
    let run = if run.glyphs.iter().any(|g| g.glyph_id == 0) {
        let fallback_text = if checked { "[x]" } else { "[ ]" };
        shape_text(registry, &resolved, fallback_text, 0)?
    } else {
        run
    };

    let gap = 4.0;
    let marker_x = params.left_margin + params.list_indent - run.advance_width - gap;
    let marker_x = marker_x.max(params.left_margin);

    Some(ShapedListMarker { run, x: marker_x })
}

fn get_default_metrics(registry: &FontRegistry, scale_factor: f32) -> FontMetricsPx {
    if let Some(default_id) = registry.default_font() {
        let resolved = ResolvedFont {
            font_face_id: default_id,
            size_px: registry.default_size_px(),
            face_index: registry.get(default_id).map(|e| e.face_index).unwrap_or(0),
            swash_cache_key: registry
                .get(default_id)
                .map(|e| e.swash_cache_key)
                .unwrap_or_default(),
            scale_factor,
            weight: 400,
        };
        if let Some(m) = font_metrics_px(registry, &resolved) {
            return m;
        }
    }
    // Absolute fallback: synthetic metrics for 16px
    FontMetricsPx {
        ascent: 14.0,
        descent: 4.0,
        leading: 0.0,
        underline_offset: -2.0,
        strikeout_offset: 5.0,
        stroke_size: 1.0,
    }
}
