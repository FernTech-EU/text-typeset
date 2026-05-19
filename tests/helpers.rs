#![allow(dead_code)]

use std::fmt;

use text_typeset::layout::block::{BlockLayoutParams, FragmentParams};
use text_typeset::layout::frame::{FrameBorderStyle, FrameLayoutParams, FramePosition};
use text_typeset::layout::paragraph::Alignment;
use text_typeset::layout::table::{CellLayoutParams, TableLayoutParams};
use text_typeset::{
    AtlasSnapshot, BlockVisualInfo, CharacterGeometry, ContentWidthMode, CursorDisplay,
    DecorationKind, DocumentFlow, FontFaceId, HitTestResult, InlineMarkup, ParagraphResult,
    RenderFrame, SingleLineResult, TextFontService, TextFormat, UnderlineStyle, VerticalAlignment,
};

pub const NOTO_SANS: &[u8] = include_bytes!("../test-fonts/NotoSans-Variable.ttf");

// ── Rect type ───────────────────────────────────────────────────

/// Thin wrapper over `[f32; 4]` giving named accessors and geometric tests.
#[derive(Clone, Copy, PartialEq)]
pub struct Rect(pub [f32; 4]);

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self([x, y, w, h])
    }

    pub fn x(&self) -> f32 {
        self.0[0]
    }
    pub fn y(&self) -> f32 {
        self.0[1]
    }
    pub fn w(&self) -> f32 {
        self.0[2]
    }
    pub fn h(&self) -> f32 {
        self.0[3]
    }
    pub fn right(&self) -> f32 {
        self.0[0] + self.0[2]
    }
    pub fn bottom(&self) -> f32 {
        self.0[1] + self.0[3]
    }

    /// Strict interior overlap - touching edges are NOT overlap.
    pub fn overlaps(&self, other: &Rect) -> bool {
        self.x() < other.right()
            && other.x() < self.right()
            && self.y() < other.bottom()
            && other.y() < self.bottom()
    }

    pub fn contains_point(&self, px: f32, py: f32) -> bool {
        px >= self.x() && px <= self.right() && py >= self.y() && py <= self.bottom()
    }

    pub fn contains(&self, other: &Rect) -> bool {
        other.x() >= self.x()
            && other.right() <= self.right()
            && other.y() >= self.y()
            && other.bottom() <= self.bottom()
    }
}

impl From<[f32; 4]> for Rect {
    fn from(a: [f32; 4]) -> Self {
        Self(a)
    }
}

impl From<Rect> for [f32; 4] {
    fn from(r: Rect) -> Self {
        r.0
    }
}

impl fmt::Display for Rect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[x={}, y={}, w={}, h={}]",
            self.0[0], self.0[1], self.0[2], self.0[3]
        )
    }
}

impl fmt::Debug for Rect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

// ── RenderFrameExt ──────────────────────────────────────────────

pub trait RenderFrameExt {
    fn glyph_rects(&self) -> Vec<Rect>;
    fn image_rects(&self) -> Vec<Rect>;
    fn decorations_of(&self, kind: DecorationKind) -> Vec<Rect>;
    fn cursor_rect(&self) -> Option<Rect>;
    fn selection_rects(&self) -> Vec<Rect>;
    fn glyph_count(&self) -> usize;
    fn decoration_count(&self, kind: DecorationKind) -> usize;
}

impl RenderFrameExt for RenderFrame {
    fn glyph_rects(&self) -> Vec<Rect> {
        self.glyphs.iter().map(|q| Rect::from(q.screen)).collect()
    }

    fn image_rects(&self) -> Vec<Rect> {
        self.images.iter().map(|q| Rect::from(q.screen)).collect()
    }

    fn decorations_of(&self, kind: DecorationKind) -> Vec<Rect> {
        self.decorations
            .iter()
            .filter(|d| d.kind == kind)
            .map(|d| Rect::from(d.rect))
            .collect()
    }

    fn cursor_rect(&self) -> Option<Rect> {
        self.decorations_of(DecorationKind::Cursor)
            .into_iter()
            .next()
    }

    fn selection_rects(&self) -> Vec<Rect> {
        self.decorations_of(DecorationKind::Selection)
    }

    fn glyph_count(&self) -> usize {
        self.glyphs.len()
    }

    fn decoration_count(&self, kind: DecorationKind) -> usize {
        self.decorations.iter().filter(|d| d.kind == kind).count()
    }
}

// ── Invariant assertions ────────────────────────────────────────

/// No two glyph quads overlap significantly (> 50% of smaller area).
/// Minor overlap from kerning/bearing is normal; this catches doubled glyphs
/// from a buggy incremental render.
pub fn assert_no_glyph_overlap(frame: &RenderFrame) {
    let rects: Vec<Rect> = frame.glyph_rects();
    for i in 0..rects.len() {
        for j in (i + 1)..rects.len() {
            if !rects[i].overlaps(&rects[j]) {
                continue;
            }
            // Compute overlap area
            let ox =
                (rects[i].right().min(rects[j].right()) - rects[i].x().max(rects[j].x())).max(0.0);
            let oy = (rects[i].bottom().min(rects[j].bottom()) - rects[i].y().max(rects[j].y()))
                .max(0.0);
            let overlap_area = ox * oy;
            let area_i = rects[i].w() * rects[i].h();
            let area_j = rects[j].w() * rects[j].h();
            let smaller = area_i.min(area_j);
            if smaller > 0.0 {
                let ratio = overlap_area / smaller;
                assert!(
                    ratio < 0.5,
                    "glyph[{}] {} significantly overlaps glyph[{}] {} (overlap ratio {:.2})",
                    i,
                    rects[i],
                    j,
                    rects[j],
                    ratio
                );
            }
        }
    }
}

/// `after` has at least as many glyphs as `before`.
pub fn assert_glyph_count_preserved(before: &RenderFrame, after: &RenderFrame) {
    assert!(
        after.glyphs.len() >= before.glyphs.len(),
        "glyph count decreased: {} -> {}",
        before.glyphs.len(),
        after.glyphs.len()
    );
}

/// `after` has at least as many decorations of `kind` as `before`.
pub fn assert_decoration_count_preserved(
    before: &RenderFrame,
    after: &RenderFrame,
    kind: DecorationKind,
) {
    let before_count = before.decoration_count(kind);
    let after_count = after.decoration_count(kind);
    assert!(
        after_count >= before_count,
        "{:?} decoration count decreased: {} -> {}",
        kind,
        before_count,
        after_count
    );
}

/// Sorted (y, height) pairs do not overlap vertically.
pub fn assert_blocks_non_overlapping(blocks: &[(f32, f32)]) {
    let mut sorted: Vec<(f32, f32)> = blocks.to_vec();
    sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    for i in 0..sorted.len().saturating_sub(1) {
        let bottom = sorted[i].0 + sorted[i].1;
        let next_y = sorted[i + 1].0;
        assert!(
            bottom <= next_y + 0.01,
            "block[{}] y={} h={} bottom={} overlaps block[{}] y={}",
            i,
            sorted[i].0,
            sorted[i].1,
            bottom,
            i + 1,
            next_y
        );
    }
}

/// Caret rect has h > 0 and y >= 0. Catches the fallback sentinel
/// `[0.0, -scroll_offset, 2.0, 16.0]` emitted when caret_rect() cannot
/// find the position.
pub fn assert_caret_is_real(rect: [f32; 4], label: &str) {
    assert!(
        rect[3] > 0.0,
        "caret height is zero for {}: {:?}",
        label,
        rect
    );
    assert!(
        rect[1] >= 0.0,
        "caret y is negative for {} (sentinel?): {:?}",
        label,
        rect
    );
}

// ── Setup helpers ───────────────────────────────────────────────

// ── Test-only Typesetter façade ─────────────────────────────────
//
// text-typeset's public API is deliberately split into a shared
// `TextFontService` (fonts + glyph atlas + shaper cache) and a
// per-widget `DocumentFlow` (viewport, layout, cursor, render
// state). The test suite exercises that split end-to-end by
// building a fresh service and a fresh flow for every test. This
// façade exists ONLY in the test module: it holds a service and a
// flow side by side and forwards every method the old monolithic
// `Typesetter` used to expose, so the existing tests keep reading
// naturally without repeating `(service, flow)` boilerplate on
// every line.
//
// **Not public API.** If you're reading this while porting a
// downstream crate, use `TextFontService` + `DocumentFlow`
// directly — see the library docs.

/// Test façade that bundles a [`TextFontService`] and a
/// [`DocumentFlow`] into one struct with the pre-split
/// `Typesetter` method surface. Fully owned by the test module.
pub struct Typesetter {
    pub service: TextFontService,
    pub flow: DocumentFlow,
}

impl Typesetter {
    pub fn new() -> Self {
        Self {
            service: TextFontService::new(),
            flow: DocumentFlow::new(),
        }
    }

    // ── Font registration (service-side) ──
    pub fn register_font(&mut self, data: &[u8]) -> FontFaceId {
        self.service.register_font(data)
    }
    pub fn register_font_as(
        &mut self,
        data: &[u8],
        family: &str,
        weight: u16,
        italic: bool,
    ) -> FontFaceId {
        self.service.register_font_as(data, family, weight, italic)
    }
    pub fn set_default_font(&mut self, face: FontFaceId, size_px: f32) {
        self.service.set_default_font(face, size_px);
    }
    pub fn set_generic_family(&mut self, generic: &str, family: &str) {
        self.service.set_generic_family(generic, family);
    }
    pub fn font_family_name(&self, face_id: FontFaceId) -> Option<String> {
        self.service.font_family_name(face_id)
    }
    pub fn font_registry(&self) -> &text_typeset::font::registry::FontRegistry {
        self.service.font_registry()
    }
    pub fn set_scale_factor(&mut self, scale_factor: f32) {
        self.service.set_scale_factor(scale_factor);
        // The pre-split Typesetter cleared the flow layout on
        // scale change. After the split the service can no longer
        // reach into per-widget flows, so tests that poke the
        // scale factor and then read the flow expect the dirty
        // check to be applied. Tests that do care re-run their
        // `layout_*` calls explicitly; tests that don't care
        // just keep the old (now empty) layout.
        //
        // We don't pre-clear here either: the generation counter
        // on the service already marks the flow as dirty, and
        // `layout_dirty_for_scale` plus the next layout call
        // handle the cleanup exactly like production does.
    }
    pub fn scale_factor(&self) -> f32 {
        self.service.scale_factor()
    }
    pub fn atlas_snapshot(&mut self, advance_generation: bool) -> AtlasSnapshot<'_> {
        self.service.atlas_snapshot(advance_generation)
    }

    // ── Viewport / scroll / zoom (flow-side) ──
    pub fn set_viewport(&mut self, width: f32, height: f32) {
        self.flow.set_viewport(width, height);
    }
    pub fn set_content_width(&mut self, width: f32) {
        self.flow.set_content_width(width);
    }
    pub fn set_content_width_auto(&mut self) {
        self.flow.set_content_width_auto();
    }
    pub fn layout_width(&self) -> f32 {
        self.flow.layout_width()
    }
    pub fn set_scroll_offset(&mut self, offset: f32) {
        self.flow.set_scroll_offset(offset);
    }
    pub fn content_height(&self) -> f32 {
        self.flow.content_height()
    }
    pub fn max_content_width(&self) -> f32 {
        self.flow.max_content_width()
    }
    pub fn set_zoom(&mut self, zoom: f32) {
        self.flow.set_zoom(zoom);
    }
    pub fn zoom(&self) -> f32 {
        self.flow.zoom()
    }

    // ── Layout ──
    #[cfg(feature = "text-document")]
    pub fn layout_full(&mut self, flow: &text_document::FlowSnapshot) {
        self.flow.layout_full(&self.service, flow);
    }
    pub fn layout_blocks(&mut self, block_params: Vec<BlockLayoutParams>) {
        self.flow.layout_blocks(&self.service, block_params);
    }
    pub fn add_frame(&mut self, params: &FrameLayoutParams) {
        self.flow.add_frame(&self.service, params);
    }
    pub fn add_table(&mut self, params: &TableLayoutParams) {
        self.flow.add_table(&self.service, params);
    }
    pub fn relayout_block(&mut self, params: &BlockLayoutParams) {
        // Tests that exercise the pre-split Typesetter API
        // always call this after a prior layout, never across a
        // scale-factor change. Anything else is a test bug and
        // should fail loudly rather than silently corrupt the
        // flow.
        self.flow
            .relayout_block(&self.service, params)
            .expect("relayout_block invariant violated in test");
    }

    // ── Rendering ──
    pub fn render(&mut self) -> &RenderFrame {
        self.flow.render(&mut self.service)
    }
    pub fn render_block_only(&mut self, block_id: usize) -> &RenderFrame {
        self.flow.render_block_only(&mut self.service, block_id)
    }
    pub fn render_cursor_only(&mut self) -> &RenderFrame {
        self.flow.render_cursor_only(&mut self.service)
    }

    // ── Single-line layout ──
    pub fn layout_single_line(
        &mut self,
        text: &str,
        format: &TextFormat,
        max_width: Option<f32>,
    ) -> SingleLineResult {
        self.flow
            .layout_single_line(&mut self.service, text, format, max_width)
    }
    pub fn layout_paragraph(
        &mut self,
        text: &str,
        format: &TextFormat,
        max_width: f32,
        max_lines: Option<usize>,
    ) -> ParagraphResult {
        self.flow
            .layout_paragraph(&mut self.service, text, format, max_width, max_lines)
    }
    pub fn layout_single_line_markup(
        &mut self,
        markup: &InlineMarkup,
        format: &TextFormat,
        max_width: Option<f32>,
    ) -> SingleLineResult {
        self.flow
            .layout_single_line_markup(&mut self.service, markup, format, max_width)
    }
    pub fn layout_paragraph_markup(
        &mut self,
        markup: &InlineMarkup,
        format: &TextFormat,
        max_width: f32,
        max_lines: Option<usize>,
    ) -> ParagraphResult {
        self.flow
            .layout_paragraph_markup(&mut self.service, markup, format, max_width, max_lines)
    }

    // ── Hit testing & geometry ──
    pub fn hit_test(&self, x: f32, y: f32) -> Option<HitTestResult> {
        self.flow.hit_test(x, y)
    }
    pub fn character_geometry(
        &self,
        block_id: usize,
        char_start: usize,
        char_end: usize,
    ) -> Vec<CharacterGeometry> {
        self.flow.character_geometry(block_id, char_start, char_end)
    }
    /// Caret rect at `position` with the default `Downstream`
    /// affinity. Existing tests use this; new affinity-aware tests
    /// use [`Self::caret_rect_with_affinity`].
    pub fn caret_rect(&self, position: usize) -> [f32; 4] {
        self.flow
            .caret_rect(position, text_typeset::CursorAffinity::Downstream)
    }

    /// Caret rect at `position` with a specific affinity. At soft-wrap
    /// boundaries the two affinities return different rects (one per
    /// display line); at every other position the result is the same
    /// as `caret_rect(position)`.
    pub fn caret_rect_with_affinity(
        &self,
        position: usize,
        affinity: text_typeset::CursorAffinity,
    ) -> [f32; 4] {
        self.flow.caret_rect(position, affinity)
    }

    // ── Cursor & colors ──
    pub fn set_cursor(&mut self, cursor: &CursorDisplay) {
        self.flow.set_cursor(cursor);
    }
    pub fn set_cursors(&mut self, cursors: &[CursorDisplay]) {
        self.flow.set_cursors(cursors);
    }
    pub fn set_selection_color(&mut self, color: [f32; 4]) {
        self.flow.set_selection_color(color);
    }
    pub fn set_cursor_color(&mut self, color: [f32; 4]) {
        self.flow.set_cursor_color(color);
    }
    pub fn set_text_color(&mut self, color: [f32; 4]) {
        self.flow.set_text_color(color);
    }

    // ── Scrolling helpers ──
    pub fn block_visual_info(&self, block_id: usize) -> Option<BlockVisualInfo> {
        self.flow.block_visual_info(block_id)
    }
    pub fn is_block_in_table(&self, block_id: usize) -> bool {
        self.flow.is_block_in_table(block_id)
    }
    pub fn scroll_to_position(&mut self, position: usize) -> f32 {
        self.flow.scroll_to_position(position)
    }
    pub fn ensure_caret_visible(&mut self) -> Option<f32> {
        self.flow.ensure_caret_visible()
    }

    pub fn content_width_mode(&self) -> ContentWidthMode {
        self.flow.content_width_mode()
    }
}

impl Default for Typesetter {
    fn default() -> Self {
        Self::new()
    }
}

/// Pre-split façade with NotoSans at 16px default and an 800×600
/// viewport. Uses the test-only `Typesetter` wrapper.
pub fn make_typesetter() -> Typesetter {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(800.0, 600.0);
    ts
}

/// Minimal BlockLayoutParams: single fragment, all formatting fields at defaults.
pub fn make_block(id: usize, text: &str) -> BlockLayoutParams {
    make_block_at(id, 0, text)
}

/// Same as make_block but with a non-zero document position.
pub fn make_block_at(id: usize, position: usize, text: &str) -> BlockLayoutParams {
    BlockLayoutParams {
        block_id: id,
        position,
        text: text.to_string(),
        fragments: vec![FragmentParams {
            text: text.to_string(),
            offset: 0,
            length: text.len(),
            font_family: None,
            font_weight: None,
            font_bold: None,
            font_italic: None,
            font_point_size: None,
            underline_style: UnderlineStyle::None,
            overline: false,
            strikeout: false,
            is_link: false,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            foreground_color: None,
            underline_color: None,
            background_color: None,
            anchor_href: None,
            tooltip: None,
            vertical_alignment: VerticalAlignment::Normal,
            image_name: None,
            image_width: 0.0,
            image_height: 0.0,
        }],
        alignment: Alignment::Left,
        top_margin: 0.0,
        bottom_margin: 0.0,
        left_margin: 0.0,
        right_margin: 0.0,
        text_indent: 0.0,
        list_marker: String::new(),
        list_indent: 0.0,
        tab_positions: vec![],
        line_height_multiplier: None,
        non_breakable_lines: false,
        checkbox: None,
        background_color: None,
    }
}

/// CellLayoutParams with a single block. Auto block_id = row * 100 + col.
pub fn make_cell(row: usize, col: usize, text: &str) -> CellLayoutParams {
    CellLayoutParams {
        row,
        column: col,
        blocks: vec![make_block(row * 100 + col, text)],
        background_color: None,
    }
}

/// CellLayoutParams with explicit block_id and position.
pub fn make_cell_at(
    row: usize,
    col: usize,
    block_id: usize,
    position: usize,
    text: &str,
) -> CellLayoutParams {
    CellLayoutParams {
        row,
        column: col,
        blocks: vec![make_block_at(block_id, position, text)],
        background_color: None,
    }
}

/// TableLayoutParams with common defaults.
pub fn make_table(
    id: usize,
    rows: usize,
    cols: usize,
    cells: Vec<CellLayoutParams>,
) -> TableLayoutParams {
    TableLayoutParams {
        table_id: id,
        rows,
        columns: cols,
        column_widths: vec![],
        border_width: 1.0,
        cell_spacing: 0.0,
        cell_padding: 4.0,
        cells,
    }
}

/// FrameLayoutParams with common defaults: Inline, no width/height,
/// zero margins, padding=4.0, border=1.0, Full border style.
pub fn make_frame(id: usize, blocks: Vec<BlockLayoutParams>) -> FrameLayoutParams {
    FrameLayoutParams {
        frame_id: id,
        position: FramePosition::Inline,
        width: None,
        height: None,
        margin_top: 0.0,
        margin_bottom: 0.0,
        margin_left: 0.0,
        margin_right: 0.0,
        padding: 4.0,
        border_width: 1.0,
        border_style: FrameBorderStyle::Full,
        blocks,
        tables: vec![],
        frames: vec![],
    }
}

// ── Debug helper ────────────────────────────────────────────────

/// Print all glyph positions and decoration kinds/positions to stderr.
#[allow(dead_code)]
pub fn dump_frame(frame: &RenderFrame) {
    eprintln!("=== RenderFrame dump ===");
    eprintln!("glyphs ({}):", frame.glyphs.len());
    for (i, q) in frame.glyphs.iter().enumerate() {
        eprintln!("  [{}] {}", i, Rect::from(q.screen));
    }
    eprintln!("images ({}):", frame.images.len());
    for (i, q) in frame.images.iter().enumerate() {
        eprintln!("  [{}] {} name={:?}", i, Rect::from(q.screen), q.name);
    }
    eprintln!("decorations ({}):", frame.decorations.len());
    for (i, d) in frame.decorations.iter().enumerate() {
        eprintln!("  [{}] {:?}\t{}", i, d.kind, Rect::from(d.rect));
    }
    eprintln!(
        "atlas: {}x{}, dirty={}",
        frame.atlas_width, frame.atlas_height, frame.atlas_dirty
    );
}
