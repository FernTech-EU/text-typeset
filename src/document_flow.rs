//! Per-widget document flow state.
//!
//! A [`DocumentFlow`] is everything that describes **what a specific
//! widget is showing** — viewport, zoom, scroll offset, wrap mode,
//! the laid-out flow (blocks / tables / frames), the rendered frame
//! cache, the cursor(s), and the selection / caret / text colors.
//!
//! Flows do not own font data. Every layout and render call takes a
//! [`TextFontService`] by reference and reads the font registry,
//! glyph atlas, and glyph cache through it. This split lets many
//! widgets in the same window share one atlas (and one GPU upload
//! per frame) while each owns an independent view onto its own
//! document.
//!
//! # Lifecycle
//!
//! ```rust,no_run
//! use text_typeset::{DocumentFlow, TextFontService};
//!
//! let mut service = TextFontService::new();
//! let face = service.register_font(include_bytes!("../test-fonts/NotoSans-Variable.ttf"));
//! service.set_default_font(face, 16.0);
//!
//! let mut flow = DocumentFlow::new();
//! flow.set_viewport(800.0, 600.0);
//!
//! # #[cfg(feature = "text-document")]
//! # {
//! let doc = text_document::TextDocument::new();
//! doc.set_plain_text("Hello, world!").unwrap();
//! flow.layout_full(&service, &doc.snapshot_flow());
//! # }
//!
//! let frame = flow.render(&mut service);
//! // frame.glyphs     -> glyph quads (textured rects from the shared atlas)
//! // frame.decorations -> cursor, selection, underlines, borders
//! ```
//!
//! The caller's pattern for a multi-widget UI is the same, plus one
//! rule: each widget owns its own `DocumentFlow` and must re-push
//! its view state (viewport, zoom, scroll, cursor, colors) before
//! its own `layout_*` / `render` call, because those fields live on
//! the flow itself, not on the shared service.

use crate::TextFontService;
use crate::font::resolve::resolve_font;
use crate::layout::block::BlockLayoutParams;
use crate::layout::flow::{FlowItem, FlowLayout};
use crate::layout::frame::FrameLayoutParams;
use crate::layout::inline_markup::{InlineAttrs, InlineMarkup};
use crate::layout::paragraph::{Alignment, Hyphenator, RunOrder, break_into_lines};
use crate::layout::table::TableLayoutParams;
use crate::shaping::run::{ShapedGlyph, ShapedRun};
use crate::shaping::shaper::{
    TextDirection, bidi_runs, font_metrics_px, shape_text, shape_text_with_fallback,
    to_harfrust_features,
};
use crate::types::{
    BlockVisualInfo, CharacterGeometry, CursorDisplay, DecorationKind, DecorationRect, GlyphQuad,
    HitTestResult, LaidOutSpan, LaidOutSpanKind, ParagraphResult, RenderFrame, SingleLineResult,
    TextFormat,
};

/// Reasons [`DocumentFlow::relayout_block`] may refuse an
/// incremental update.
///
/// Both variants describe invariant violations the caller can
/// detect structurally ahead of time by asking
/// [`DocumentFlow::has_layout`] and
/// [`DocumentFlow::layout_dirty_for_scale`]. Returned as a
/// `Result` rather than panicking so a misbehaving caller
/// produces a recoverable error at the exact call site instead
/// of corrupting the flow with a partial relayout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayoutError {
    /// No `layout_*` method has been called on this flow yet.
    /// The caller must run [`DocumentFlow::layout_full`] or
    /// [`DocumentFlow::layout_blocks`] first to establish a
    /// baseline layout before incremental updates make sense.
    NoLayout,
    /// The backing [`TextFontService`] has had its HiDPI scale
    /// factor changed since this flow was last laid out, so the
    /// existing block layouts hold advances at the old ppem.
    /// Re-shaping a single block would leave it at the new ppem
    /// while neighbors stay at the old, producing an inconsistent
    /// flow. The caller must re-run `layout_full` /
    /// `layout_blocks` to rebuild everything at the new scale.
    ScaleDirty,
}

impl std::fmt::Display for RelayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RelayoutError::NoLayout => {
                f.write_str("relayout_block called before any layout_* method")
            }
            RelayoutError::ScaleDirty => f.write_str(
                "relayout_block called after a scale-factor change without a fresh layout_*",
            ),
        }
    }
}

impl std::error::Error for RelayoutError {}

/// How the content (layout) width is determined.
///
/// Controls whether text reflows when the viewport resizes (web or
/// editor style) or wraps at a fixed width (page / WYSIWYG style).
#[derive(Debug, Clone, Copy, Default)]
pub enum ContentWidthMode {
    /// Content width equals viewport width (divided by zoom). Text
    /// reflows on window resize — the default, typical for editors
    /// and web layout.
    #[default]
    Auto,
    /// Content width is fixed at a specific value, independent of
    /// the viewport. Useful for page-like WYSIWYG layout, print
    /// preview, or side panels with their own column width.
    Fixed(f32),
}

/// Per-widget document flow state.
///
/// See the module-level docs for the shape of the split and for
/// lifecycle examples. Every layout/render method here takes a
/// [`TextFontService`] reference so flows can share one atlas across
/// an entire window.
/// Whether the render window changed enough since the last full render that the
/// incremental paths — which reuse the culled cache — must fall back to a full
/// re-render. A `None`↔`Some` transition always drifts; two `Some`s drift on a
/// >0.001px change in either endpoint (so a scroll re-renders, a still view does not).
fn render_window_drifted(now: Option<(f32, f32)>, then: Option<(f32, f32)>) -> bool {
    match (now, then) {
        (None, None) => false,
        (Some((t0, h0)), Some((t1, h1))) => (t0 - t1).abs() > 0.001 || (h0 - h1).abs() > 0.001,
        _ => true,
    }
}

pub struct DocumentFlow {
    flow_layout: FlowLayout,
    render_frame: RenderFrame,
    scroll_offset: f32,
    rendered_scroll_offset: f32,
    /// When `Some((top, height))`, render culling uses this content-space band
    /// instead of `[scroll_offset, scroll_offset + viewport_height]`. Positioning
    /// (glyph screen y, hit-testing) is unaffected. See
    /// [`DocumentFlow::set_render_window`].
    render_window: Option<(f32, f32)>,
    /// The `render_window` in effect at the last full `render()`, so the
    /// incremental paths can fall back when the visible band scrolls.
    rendered_window: Option<(f32, f32)>,
    viewport_width: f32,
    viewport_height: f32,
    content_width_mode: ContentWidthMode,
    selection_color: [f32; 4],
    cursor_color: [f32; 4],
    text_color: [f32; 4],
    /// Background used by the text-document bridge when a code block
    /// carries no explicit `background_color`. Overrides the bridge's
    /// historical light-grey default. Threaded into every
    /// `convert_flow_with` / `convert_block_with` call kicked off
    /// from `layout_full`. See [`Self::set_code_block_background`].
    code_block_background: [f32; 4],
    /// Foreground used by the text-document bridge for monospaced runs
    /// (markdown inline `code`, fenced code blocks) that carry no
    /// explicit `foreground_color`. `None` keeps the engine's default
    /// `text_color`. See [`Self::set_code_block_foreground`].
    code_block_foreground: Option<[f32; 4]>,
    /// Foreground used by the text-document bridge for runs carrying a
    /// hyperlink that set no explicit `foreground_color`. `None` keeps
    /// the engine's default `text_color`, which leaves links looking
    /// exactly like prose. See [`Self::set_link_foreground`].
    link_foreground: Option<[f32; 4]>,
    /// Echo / masking character for secure (password) fields. When
    /// `Some(c)`, every character laid out by `layout_full` is replaced
    /// with `c` before shaping, so the real text never reaches the
    /// shaper or the glyph atlas. `None` (default) lays text out
    /// verbatim. Threaded into the bridge via [`crate::bridge::BridgeOptions::echo_char`]
    /// from `layout_full`. See [`set_echo_char`](Self::set_echo_char).
    echo_char: Option<char>,
    /// Auto-hyphenate justified blocks that don't set `hyphenate`
    /// explicitly. Threaded into the bridge via
    /// [`crate::bridge::BridgeOptions::hyphenate_justified`] from
    /// `layout_full`. Enable on prose surfaces only. `false` by default.
    /// See [`set_hyphenate_justified`](Self::set_hyphenate_justified).
    hyphenate_justified: bool,
    cursors: Vec<CursorDisplay>,
    zoom: f32,
    rendered_zoom: f32,
    /// Per-document logical text-magnification factor (`1.0` = none). Unlike
    /// `zoom` (a post-layout *display* transform that leaves font metrics
    /// untouched) and `raster_scale`/`scale_factor` (raster density only),
    /// `font_scale` multiplies the resolved logical font size *before* shaping,
    /// so glyph advances, line heights, and `content_height` all grow and text
    /// re-wraps. This is the accessibility "grow all text" knob. Set via
    /// [`set_font_scale`](Self::set_font_scale); pushed into `flow_layout` at
    /// every `layout_*` call alongside `scale_factor`.
    font_scale: f32,
    /// Raster densification for content drawn under a scale transform.
    /// `1.0` = unscaled UI. Rasterization happens at
    /// `size × scale_factor × raster_scale` physical pixels while layout
    /// and glyph `screen` rects stay in logical pixels. Unlike `zoom`
    /// (a post-layout coordinate transform) this changes which bitmaps
    /// the quads sample, so the incremental `render_block_only` path
    /// falls back to a full render when it changed. See
    /// [`set_raster_scale`](Self::set_raster_scale).
    raster_scale: f32,
    rendered_raster_scale: f32,
    /// `TextFontService::scale_generation` at the time of the last
    /// `layout_*` call. Used by
    /// [`layout_dirty_for_scale`](DocumentFlow::layout_dirty_for_scale)
    /// so the framework can detect HiDPI transitions and re-run
    /// layout without having to track them itself.
    layout_scale_generation: u64,
    /// Whether any `layout_*` call has been made at least once.
    has_layout: bool,
}

impl DocumentFlow {
    /// Create an empty flow with no content.
    ///
    /// After construction the caller typically calls
    /// [`set_viewport`](Self::set_viewport) and one of the
    /// `layout_*` methods before the first render.
    pub fn new() -> Self {
        Self {
            flow_layout: FlowLayout::new(),
            render_frame: RenderFrame::new(),
            scroll_offset: 0.0,
            rendered_scroll_offset: f32::NAN,
            render_window: None,
            rendered_window: None,
            viewport_width: 0.0,
            viewport_height: 0.0,
            content_width_mode: ContentWidthMode::Auto,
            selection_color: [0.26, 0.52, 0.96, 0.3],
            cursor_color: [0.0, 0.0, 0.0, 1.0],
            text_color: [0.0, 0.0, 0.0, 1.0],
            code_block_background: [0.95, 0.95, 0.95, 1.0],
            code_block_foreground: None,
            link_foreground: None,
            echo_char: None,
            hyphenate_justified: false,
            cursors: Vec::new(),
            zoom: 1.0,
            rendered_zoom: f32::NAN,
            font_scale: 1.0,
            raster_scale: 1.0,
            rendered_raster_scale: f32::NAN,
            layout_scale_generation: 0,
            has_layout: false,
        }
    }

    // ── Viewport & content width ───────────────────────────────

    /// Set the visible area dimensions in logical pixels.
    ///
    /// The viewport controls:
    ///
    /// - **Culling**: only blocks within the viewport are rendered.
    /// - **Selection highlight**: multi-line selection extends to
    ///   the viewport width.
    /// - **Layout width** (in [`ContentWidthMode::Auto`]): text
    ///   wraps at `viewport_width / zoom`.
    ///
    /// Call this when the widget's container resizes. A resize by
    /// itself does not relayout — re-run `layout_full` /
    /// `layout_blocks` if the wrap width changed.
    pub fn set_viewport(&mut self, width: f32, height: f32) {
        self.viewport_width = width;
        self.viewport_height = height;
        self.flow_layout.viewport_width = width;
        self.flow_layout.viewport_height = height;
    }

    /// Current viewport width in logical pixels.
    pub fn viewport_width(&self) -> f32 {
        self.viewport_width
    }

    /// Current viewport height in logical pixels.
    pub fn viewport_height(&self) -> f32 {
        self.viewport_height
    }

    /// Pin content width at a fixed value, independent of viewport.
    ///
    /// Text wraps at this width regardless of how wide the viewport
    /// is. Use for page-like (WYSIWYG) layout or documents with an
    /// explicit column width. Pass `f32::INFINITY` for no-wrap mode.
    pub fn set_content_width(&mut self, width: f32) {
        self.content_width_mode = ContentWidthMode::Fixed(width);
    }

    /// Reflow content width to follow the viewport (the default).
    ///
    /// Text re-wraps on every viewport resize. Standard editor and
    /// web-style layout.
    pub fn set_content_width_auto(&mut self) {
        self.content_width_mode = ContentWidthMode::Auto;
    }

    /// The effective width used for text layout (line wrapping,
    /// table columns, etc.).
    ///
    /// In [`ContentWidthMode::Auto`], equals `viewport_width / zoom`
    /// so that text reflows to fit the zoomed viewport. In
    /// [`ContentWidthMode::Fixed`], equals the set value (zoom only
    /// magnifies the rendered output).
    pub fn layout_width(&self) -> f32 {
        match self.content_width_mode {
            ContentWidthMode::Auto => self.viewport_width / self.zoom,
            ContentWidthMode::Fixed(w) => w,
        }
    }

    /// The currently configured content-width mode.
    pub fn content_width_mode(&self) -> ContentWidthMode {
        self.content_width_mode
    }

    /// Set the vertical scroll offset in logical pixels from the
    /// top of the document. Affects culling and screen-space `y`
    /// coordinates in the rendered frame.
    pub fn set_scroll_offset(&mut self, offset: f32) {
        self.scroll_offset = offset;
    }

    /// Current vertical scroll offset.
    pub fn scroll_offset(&self) -> f32 {
        self.scroll_offset
    }

    /// Restrict render **culling** to the content-space band `[top, top + height]`
    /// instead of the default `[scroll_offset, scroll_offset + viewport_height]`.
    ///
    /// This affects *only* which blocks / lines / decorations are emitted into the
    /// frame — glyph screen positions, hit-testing and caret geometry all still key
    /// off `scroll_offset` and are unchanged. It exists for an editor laid out at its
    /// full document height inside an outer `ScrollArea` ("bastard mode"): its own
    /// viewport spans the whole document (so the viewport-derived window culls
    /// nothing) and `scroll_offset` stays `0` (the ancestor scrolls it by
    /// translation), so the true visible band must be supplied from the ancestor
    /// clip. Pass `None` (the default) to restore the viewport-derived window.
    pub fn set_render_window(&mut self, window: Option<(f32, f32)>) {
        self.render_window = window;
    }

    /// The active render window, if any. See [`set_render_window`](Self::set_render_window).
    pub fn render_window(&self) -> Option<(f32, f32)> {
        self.render_window
    }

    /// Total content height after layout, in logical pixels.
    pub fn content_height(&self) -> f32 {
        self.flow_layout.content_height
    }

    /// Maximum content width across all laid-out lines, in logical
    /// pixels. Used for horizontal scrollbar range when wrapping
    /// is disabled.
    pub fn max_content_width(&self) -> f32 {
        self.flow_layout.cached_max_content_width
    }

    // ── Zoom ────────────────────────────────────────────────────

    /// Set the display zoom level (`1.0` = 100 %).
    ///
    /// Zoom scales screen-space output (glyph quads, decorations, caret
    /// rects) after layout. Font metrics at layout stay at base size;
    /// hit-test inputs are inversely scaled.
    ///
    /// **Wrap / reflow.** In [`ContentWidthMode::Auto`] (the editor
    /// default) layout width is `viewport_width / zoom`, so text
    /// re-wraps when zoom changes — browser-style zoom. In
    /// [`ContentWidthMode::Fixed`], wrap width is independent of zoom
    /// (page magnify without reflow).
    ///
    /// **Sharpness.** Glyph bitmaps densify under zoom: the next
    /// [`render`](Self::render) rasterizes at
    /// `ambient_raster_scale × quantize(zoom)` physical density so
    /// magnified text stays crisp instead of stretching a 1× atlas
    /// entry. Zoom-out (`< 1`) keeps density ≥ 1 and relies on linear
    /// minification. Continuous zoom is quantized onto a short ladder
    /// (same contract as scene transform densification) so the atlas
    /// does not grow a new size per frame.
    ///
    /// Clamped to `0.1..=10.0`. Default is `1.0`.
    pub fn set_zoom(&mut self, zoom: f32) {
        self.zoom = zoom.clamp(0.1, 10.0);
    }

    /// Current display zoom level.
    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    /// Raster densification used on the next paint: ambient
    /// [`raster_scale`](Self::raster_scale) × zoom, quantized onto the
    /// densify ladder (see [`quantize_raster_scale`]). Layout and
    /// pre-zoom `screen` rects stay logical; only the atlas bitmap
    /// density changes.
    fn densify_raster_scale(&self) -> f32 {
        quantize_raster_scale(self.raster_scale * self.zoom)
    }

    // ── Font scale (logical text magnification) ──────────────────

    /// Set the per-document logical font-scale factor (`1.0` = none).
    ///
    /// Unlike [`set_zoom`](Self::set_zoom) (a post-layout display transform
    /// that does **not** change font metrics), `font_scale` multiplies the
    /// resolved logical font size *before* shaping. Glyph advances, line
    /// heights, and `content_height` all grow, and text re-wraps — true text
    /// magnification, the mechanism behind an app-wide "grow all text"
    /// accessibility setting. Takes effect on the next `layout_*` call.
    /// Clamped to `0.1..=10.0`.
    pub fn set_font_scale(&mut self, font_scale: f32) {
        self.font_scale = font_scale.clamp(0.1, 10.0);
    }

    /// Current logical font-scale factor.
    pub fn font_scale(&self) -> f32 {
        self.font_scale
    }

    /// Set the ambient raster densification scale for content drawn
    /// under an *external* scale transform (a zoomed scene viewport).
    ///
    /// Combined with [`set_zoom`](Self::set_zoom) at paint time: glyphs
    /// densify at `quantize(raster_scale × zoom)`. Layout, metrics, and
    /// pre-zoom `screen` rects stay logical, so no relayout is needed
    /// after changing ambient densification alone. The next
    /// [`render`](Self::render) rasterizes missing glyphs at the new
    /// density; old-density entries age out of the atlas via the normal
    /// LRU. Scaled rasters (`!= 1.0`) are unhinted.
    ///
    /// Clamped to `0.1..=16.0`. Default is `1.0`.
    pub fn set_raster_scale(&mut self, raster_scale: f32) {
        self.raster_scale = raster_scale.clamp(0.1, 16.0);
    }

    /// Current raster densification scale.
    pub fn raster_scale(&self) -> f32 {
        self.raster_scale
    }

    // ── Scale factor sync ───────────────────────────────────────

    /// Whether any `layout_*` method has run on this flow at least
    /// once. Callers that need to distinguish "never laid out"
    /// from "laid out against a stale scale factor" read this
    /// alongside [`layout_dirty_for_scale`](Self::layout_dirty_for_scale).
    pub fn has_layout(&self) -> bool {
        self.has_layout
    }

    /// Returns `true` when the backing [`TextFontService`] has had
    /// its HiDPI scale factor changed since this flow was last laid
    /// out, meaning stored shaped advances and cached ppem values
    /// are stale.
    ///
    /// Call after every `service.set_scale_factor(...)` to decide
    /// whether to re-run `layout_full` / `layout_blocks` before the
    /// next render. Returns `false` for flows that have never been
    /// laid out at all (nothing to invalidate).
    pub fn layout_dirty_for_scale(&self, service: &TextFontService) -> bool {
        self.has_layout && self.layout_scale_generation != service.scale_generation()
    }

    // ── Layout ──────────────────────────────────────────────────

    /// Full layout from a text-document `FlowSnapshot`.
    ///
    /// Clears any existing flow state and lays out every element
    /// (blocks, tables, frames) from the snapshot in flow order.
    /// Call on document load or `DocumentReset`. For single-block
    /// edits prefer [`relayout_block`](Self::relayout_block).
    #[cfg(feature = "text-document")]
    pub fn layout_full(&mut self, service: &TextFontService, flow: &text_document::FlowSnapshot) {
        use crate::bridge::{BridgeOptions, convert_flow_with};

        let opts = BridgeOptions {
            code_block_background: self.code_block_background,
            code_block_foreground: self.code_block_foreground,
            link_foreground: self.link_foreground,
            echo_char: self.echo_char,
            hyphenate_justified: self.hyphenate_justified,
        };
        let converted = convert_flow_with(flow, &opts);

        // Merge all elements by flow index and process in order.
        let mut all_items: Vec<(usize, FlowItemKind)> = Vec::new();
        for (idx, params) in converted.blocks {
            all_items.push((idx, FlowItemKind::Block(params)));
        }
        for (idx, params) in converted.tables {
            all_items.push((idx, FlowItemKind::Table(params)));
        }
        for (idx, params) in converted.frames {
            all_items.push((idx, FlowItemKind::Frame(params)));
        }
        all_items.sort_by_key(|(idx, _)| *idx);

        let lw = self.layout_width();
        self.flow_layout.clear();
        self.flow_layout.viewport_width = self.viewport_width;
        self.flow_layout.viewport_height = self.viewport_height;
        self.flow_layout.scale_factor = service.scale_factor;
        self.flow_layout.font_scale = self.font_scale;

        for (_idx, kind) in all_items {
            match kind {
                FlowItemKind::Block(params) => {
                    self.flow_layout
                        .add_block(&service.font_registry, &params, lw);
                }
                FlowItemKind::Table(params) => {
                    self.flow_layout
                        .add_table(&service.font_registry, &params, lw);
                }
                FlowItemKind::Frame(params) => {
                    self.flow_layout
                        .add_frame(&service.font_registry, &params, lw);
                }
            }
        }

        // Capture the freshly-shaped blocks as the paint-overlay base. The
        // engine applies paint spans afterward (recolor without reshape).
        self.flow_layout.refresh_base_blocks();

        self.note_layout_done(service);
    }

    /// Lay out a list of blocks from scratch.
    ///
    /// Framework-agnostic entry point — the caller assembles
    /// [`BlockLayoutParams`] directly without going through
    /// text-document. Replaces any existing flow state.
    pub fn layout_blocks(
        &mut self,
        service: &TextFontService,
        block_params: Vec<BlockLayoutParams>,
    ) {
        self.flow_layout.scale_factor = service.scale_factor;
        self.flow_layout.font_scale = self.font_scale;
        self.flow_layout
            .layout_blocks(&service.font_registry, block_params, self.layout_width());
        self.note_layout_done(service);
    }

    /// Append a block to the current flow, in O(1).
    ///
    /// The block counterpart of [`add_frame`](Self::add_frame) /
    /// [`add_table`](Self::add_table), and the incremental alternative to
    /// re-running [`layout_blocks`](Self::layout_blocks) after content grows.
    ///
    /// Streaming consumers (a log/console view tailing output) need this: a
    /// full re-layout is O(N) in the whole document, so appending one line to
    /// a 100 000-line buffer costs over a second, while this stays flat at the
    /// cost of shaping the one new line, whatever the buffer already holds.
    /// See `docs/streaming-baseline.md` for the measurements.
    ///
    /// Appends at the tail: the new block takes the current `content_height`
    /// as its `y` (margin-collapsed against the previous block, exactly as a
    /// bulk layout would place it), so an append-only sequence produces a flow
    /// identical to laying the same blocks out in one call.
    ///
    /// # Invariants
    ///
    /// Like [`relayout_block`](Self::relayout_block), this is an incremental
    /// operation, so it must not run against a layout shaped at a different
    /// HiDPI scale: appending at the current scale while every existing block
    /// sits at the old one would leave the flow permanently mixed-scale — and
    /// worse, stamping the flow as freshly laid out would clear the very
    /// staleness flag ([`layout_dirty_for_scale`](Self::layout_dirty_for_scale))
    /// the caller relies on to know it must re-layout. Returns
    /// [`RelayoutError::ScaleDirty`] instead; the caller re-runs
    /// [`layout_full`](Self::layout_full) / [`layout_blocks`](Self::layout_blocks).
    ///
    /// Unlike `relayout_block` there is no `NoLayout` error: appending to an
    /// empty flow is how an append-only buffer legitimately starts.
    pub fn add_block(
        &mut self,
        service: &TextFontService,
        params: &BlockLayoutParams,
    ) -> Result<(), RelayoutError> {
        // Only meaningful once a layout exists; an empty flow has no
        // established scale to conflict with.
        if self.has_layout && self.layout_scale_generation != service.scale_generation() {
            return Err(RelayoutError::ScaleDirty);
        }
        self.flow_layout.scale_factor = service.scale_factor;
        self.flow_layout.font_scale = self.font_scale;
        self.flow_layout
            .append_block(&service.font_registry, params, self.layout_width());
        self.note_layout_done(service);
        Ok(())
    }

    /// Drop the first `n` blocks of the flow, returning how many were removed.
    ///
    /// The eviction half of a bounded streaming buffer: pair it with
    /// [`add_block`](Self::add_block) to hold a scrollback cap. Usually O(n)
    /// plus one `Vec` memmove of the survivors — nothing is reshaped. The
    /// return value is the count actually evicted, which is less than `n` when
    /// the flow holds fewer leading blocks than that, or a table/frame stops
    /// the walk.
    ///
    /// Survivors keep their absolute `y`, so the vacated band at the top
    /// becomes empty and `content_height` does not change: content below never
    /// moves, and the viewport stays where the user put it. Callers that want
    /// the freed space reclaimed re-run a full [`layout_blocks`](Self::layout_blocks).
    ///
    /// Only leading top-level blocks are evicted; a leading table or frame
    /// stops the walk. Evicting the widest block re-derives
    /// [`max_content_width`](Self::max_content_width) from the survivors, so
    /// the horizontal scroll range stops describing content that is gone.
    pub fn remove_leading(&mut self, n: usize) -> usize {
        self.flow_layout.remove_leading(n)
    }

    /// Shape only `window` — a slice of a much larger uniform-row-height
    /// document — placing each row at `y = index * row_height`.
    ///
    /// The memory counterpart of [`add_block`](Self::add_block): `add_block`
    /// makes *growing* a buffer cheap, this makes *holding* a large one cheap.
    /// A resident shaped line costs ~6.5 KB, so a fully laid-out 100 000-line
    /// buffer costs ~623 MB, against ~1 MB for a viewport-sized window; render
    /// already culls to the viewport, so shaping the rest buys nothing. See
    /// `docs/streaming-baseline.md`.
    ///
    /// `content_height` is derived from `total_rows`, so the scrollbar spans
    /// the whole document even though almost none of it is shaped. Re-call this
    /// when the visible range moves; append at the tail with
    /// [`add_block`](Self::add_block) and trim the front with
    /// [`remove_leading`](Self::remove_leading) while following output, which
    /// avoids re-shaping the window on every line.
    ///
    /// # Invariants
    ///
    /// Correct only for genuinely uniform rows: **one row = one visual line of
    /// exactly `row_height`** — no wrapping, no embedded newlines, no per-row
    /// margins, one font size throughout (log/console output, monospaced
    /// code). Variable-height or wrapped content must use
    /// [`layout_blocks`](Self::layout_blocks) / [`layout_full`](Self::layout_full).
    /// `window` must be sorted ascending by index. Both are checked in debug
    /// builds.
    ///
    /// Rows outside the window are not laid out, so
    /// [`block_visual_info`](Self::block_visual_info) and hit-testing answer
    /// only for resident rows; derive off-window geometry arithmetically from
    /// `row_height`.
    ///
    /// # Behaviour worth knowing
    ///
    /// Like [`layout_blocks`](Self::layout_blocks), this drops any paint
    /// overlay — re-apply spans after re-windowing or the rows render in base
    /// colours. Since re-windowing happens on every visible-range change, that
    /// re-apply belongs on the scroll path, not in one-off setup.
    ///
    /// [`max_content_width`](Self::max_content_width) reports the widest row
    /// *seen so far* in this session: not the document's widest (unknowable
    /// without shaping all of it), and deliberately not the window's widest,
    /// which would make the horizontal scrollbar jump on every vertical scroll.
    ///
    /// `f32` places rows exactly only to 2^24, so past ~840 000 rows at a 20 px
    /// row height positions begin quantizing — far beyond the target sizes, but
    /// not unbounded.
    pub fn layout_window(
        &mut self,
        service: &TextFontService,
        window: &[(usize, BlockLayoutParams)],
        total_rows: usize,
        row_height: f32,
    ) {
        self.flow_layout.scale_factor = service.scale_factor;
        self.flow_layout.font_scale = self.font_scale;
        self.flow_layout.layout_window(
            &service.font_registry,
            window,
            total_rows,
            row_height,
            self.layout_width(),
        );
        self.note_layout_done(service);
    }

    /// Declare the total extent of a uniform-row-height document without
    /// shaping anything.
    ///
    /// Keeps the scrollbar honest when the row count changes outside the shaped
    /// window — a line appended while the user is scrolled away from the tail,
    /// where [`add_block`](Self::add_block) would wrongly shape a row nowhere
    /// near the window. Leaves the shaped window untouched.
    ///
    /// Only meaningful for a flow driven by [`layout_window`](Self::layout_window).
    /// On a normally laid-out flow this overwrites the accumulated
    /// `content_height` with a fabricated `total_rows * row_height` that bears
    /// no relation to the real content, so the scroll range goes wrong; nothing
    /// in the type distinguishes the two, so this is the caller's contract.
    pub fn set_uniform_extent(&mut self, total_rows: usize, row_height: f32) {
        self.flow_layout.set_uniform_extent(total_rows, row_height);
    }

    /// Convert one document block snapshot into layout params using this flow's
    /// own bridge options — the per-block half of [`layout_full`](Self::layout_full)'s
    /// conversion, exposed for the windowed streaming path.
    ///
    /// [`layout_window`](Self::layout_window) takes already-built
    /// [`BlockLayoutParams`], but only
    /// this flow knows the code-block colours, echo char, and
    /// justified-hyphenation policy that `layout_full` folds in through
    /// [`BridgeOptions`](crate::bridge::BridgeOptions). A streaming consumer
    /// building a window of rows from document snapshots calls this per row, so
    /// the windowed and full paths shape a given block identically. The result
    /// is a plain value the caller may tint (set a fragment's
    /// `foreground_color`) before handing the window to `layout_window`.
    pub fn block_params_for(
        &self,
        block: &text_document::BlockSnapshot,
    ) -> crate::layout::block::BlockLayoutParams {
        let opts = crate::bridge::BridgeOptions {
            code_block_background: self.code_block_background,
            code_block_foreground: self.code_block_foreground,
            link_foreground: self.link_foreground,
            echo_char: self.echo_char,
            hyphenate_justified: self.hyphenate_justified,
        };
        crate::bridge::convert_block_with(block, &opts)
    }

    /// Append a frame to the current flow. The frame's position
    /// (inline, float, absolute) is carried in `params`.
    pub fn add_frame(&mut self, service: &TextFontService, params: &FrameLayoutParams) {
        self.flow_layout.scale_factor = service.scale_factor;
        self.flow_layout.font_scale = self.font_scale;
        self.flow_layout
            .add_frame(&service.font_registry, params, self.layout_width());
        self.note_layout_done(service);
    }

    /// Append a table to the current flow.
    pub fn add_table(&mut self, service: &TextFontService, params: &TableLayoutParams) {
        self.flow_layout.scale_factor = service.scale_factor;
        self.flow_layout.font_scale = self.font_scale;
        self.flow_layout
            .add_table(&service.font_registry, params, self.layout_width());
        self.note_layout_done(service);
    }

    /// Relayout a single block after its content or formatting
    /// changed.
    ///
    /// Re-shapes and re-wraps just that block, then shifts
    /// subsequent items if the height changed. Much cheaper than a
    /// full layout for single-block edits (typing, format toggles).
    /// If the block lives inside a table cell, the row height is
    /// re-measured and content below the table shifts.
    ///
    /// # Invariants
    ///
    /// This is an incremental operation and only makes sense when
    /// a valid layout is already installed on this flow, laid out
    /// against the same HiDPI scale factor the service currently
    /// reports. Violations produce a [`RelayoutError`]:
    ///
    /// - [`RelayoutError::NoLayout`] if no `layout_*` method has
    ///   run on this flow yet — there is nothing to update.
    /// - [`RelayoutError::ScaleDirty`] if the service's scale
    ///   factor has changed since the last layout — reshaping a
    ///   single block would leave neighbors at the old ppem and
    ///   produce an inconsistent flow. The caller must re-run
    ///   [`layout_full`](Self::layout_full) / [`layout_blocks`](Self::layout_blocks)
    ///   first.
    ///
    /// Both conditions are detected structurally from
    /// [`has_layout`](Self::has_layout) and
    /// [`layout_dirty_for_scale`](Self::layout_dirty_for_scale),
    /// so callers that already guard those don't need to handle
    /// the error.
    pub fn relayout_block(
        &mut self,
        service: &TextFontService,
        params: &BlockLayoutParams,
    ) -> Result<(), RelayoutError> {
        if !self.has_layout {
            return Err(RelayoutError::NoLayout);
        }
        if self.layout_scale_generation != service.scale_generation() {
            return Err(RelayoutError::ScaleDirty);
        }
        self.flow_layout.scale_factor = service.scale_factor;
        self.flow_layout.font_scale = self.font_scale;
        self.flow_layout
            .relayout_block(&service.font_registry, params, self.layout_width());
        self.note_layout_done(service);
        Ok(())
    }

    /// Replace the paint-only color overlay for the whole flow, re-derived from
    /// the captured base layout. Recolors without reshaping or reflowing — the
    /// fast path for search / spell / paint-only syntax highlights. Call
    /// `render` afterward to refresh the GPU frame.
    pub fn apply_paint_spans_for(
        &mut self,
        spans_by_block: std::collections::HashMap<usize, Vec<crate::layout::block::PaintSpan>>,
    ) {
        self.flow_layout.apply_paint_spans_for(spans_by_block);
    }

    /// Apply (or clear) the paint overlay for a single block. Returns `false`
    /// if the block has no captured base (no full layout yet).
    pub fn apply_block_paint_spans(
        &mut self,
        block_id: usize,
        spans: &[crate::layout::block::PaintSpan],
    ) -> bool {
        self.flow_layout.apply_block_paint_spans(block_id, spans)
    }

    fn note_layout_done(&mut self, service: &TextFontService) {
        self.has_layout = true;
        self.layout_scale_generation = service.scale_generation();
    }

    // ── Rendering ──────────────────────────────────────────────

    /// Render the visible viewport and return the produced frame.
    ///
    /// Performs viewport culling, rasterizes any glyphs missing
    /// from the atlas into it, and emits glyph quads, image quads,
    /// and decoration rectangles. The returned reference borrows
    /// both `self` and `service`; drop it before the next mutation.
    ///
    /// On every call, stale glyphs (unused for ~120 frames) are
    /// evicted from the atlas to reclaim slot space.
    pub fn render(&mut self, service: &mut TextFontService) -> &RenderFrame {
        let effective_vw = self.viewport_width / self.zoom;
        let effective_vh = self.viewport_height / self.zoom;
        let densify = self.densify_raster_scale();
        crate::render::frame::build_render_frame(
            &self.flow_layout,
            &service.font_registry,
            &mut service.atlas,
            &mut service.glyph_cache,
            &mut service.scale_context,
            self.scroll_offset,
            effective_vw,
            effective_vh,
            self.render_window,
            &self.cursors,
            self.cursor_color,
            self.selection_color,
            self.text_color,
            densify,
            &mut self.render_frame,
            &mut service.eviction_epoch,
        );
        self.rendered_scroll_offset = self.scroll_offset;
        self.rendered_window = self.render_window;
        self.rendered_zoom = self.zoom;
        self.rendered_raster_scale = self.raster_scale;
        apply_zoom(&mut self.render_frame, self.zoom);
        &self.render_frame
    }

    /// Incremental render that only re-renders one block's glyphs.
    ///
    /// Reuses cached glyph / decoration data for all other blocks
    /// from the last full `render()`. Call after
    /// [`relayout_block`](Self::relayout_block) when only one block's
    /// text changed.
    ///
    /// Falls back to a full [`render`](Self::render) if the block's
    /// height changed (subsequent glyph positions would be stale),
    /// if scroll offset or zoom changed since the last full render,
    /// or if the block lives inside a table / frame (those are
    /// cached with a different key).
    pub fn render_block_only(
        &mut self,
        service: &mut TextFontService,
        block_id: usize,
    ) -> &RenderFrame {
        if (self.scroll_offset - self.rendered_scroll_offset).abs() > 0.001
            || render_window_drifted(self.render_window, self.rendered_window)
            || (self.zoom - self.rendered_zoom).abs() > 0.001
            || (self.raster_scale - self.rendered_raster_scale).abs() > 0.001
        {
            return self.render(service);
        }

        // Defensive: if the atlas has dropped any entry since the last
        // full render, our cached per-block glyph quads may now point
        // at slots owned by unrelated glyphs. Fall back to a full
        // re-render — `touch_glyphs` in `rebuild_flat_frame` is the
        // primary keep-alive mechanism, this is the safety net.
        if service.eviction_epoch != self.render_frame.atlas_eviction_epoch {
            return self.render(service);
        }

        if !self.flow_layout.blocks.contains_key(&block_id) {
            let in_table = self.flow_layout.tables.values().any(|table| {
                table
                    .cell_layouts
                    .iter()
                    .any(|c| c.blocks.iter().any(|b| b.block_id == block_id))
            });
            if in_table {
                return self.render(service);
            }
            let in_frame = self
                .flow_layout
                .frames
                .values()
                .any(|frame| crate::layout::flow::frame_contains_block(frame, block_id));
            if in_frame {
                return self.render(service);
            }
        }

        if let Some(block) = self.flow_layout.blocks.get(&block_id) {
            let old_height = self
                .render_frame
                .block_heights
                .get(&block_id)
                .copied()
                .unwrap_or(block.height);
            if (block.height - old_height).abs() > 0.001 {
                return self.render(service);
            }
        }

        let effective_vw = self.viewport_width / self.zoom;
        let effective_vh = self.viewport_height / self.zoom;
        let densify = self.densify_raster_scale();
        let scale_factor = service.scale_factor;
        let mut new_glyphs = Vec::new();
        let mut new_images = Vec::new();
        let mut new_keys: Vec<crate::atlas::cache::GlyphCacheKey> = Vec::new();
        if let Some(block) = self.flow_layout.blocks.get(&block_id) {
            let mut tmp = RenderFrame::new();
            crate::render::frame::render_block_at_offset(
                block,
                0.0,
                0.0,
                &service.font_registry,
                &mut service.atlas,
                &mut service.glyph_cache,
                &mut service.scale_context,
                self.scroll_offset,
                effective_vh,
                self.render_window,
                self.text_color,
                scale_factor,
                densify,
                &mut tmp,
                &mut new_keys,
                &mut service.eviction_epoch,
            );
            new_glyphs = tmp.glyphs;
            new_images = tmp.images;
        }

        let new_decos = if let Some(block) = self.flow_layout.blocks.get(&block_id) {
            crate::render::decoration::generate_block_decorations(
                block,
                &service.font_registry,
                self.scroll_offset,
                effective_vh,
                self.render_window,
                0.0,
                0.0,
                effective_vw,
                self.text_color,
                scale_factor,
            )
        } else {
            Vec::new()
        };

        if let Some(entry) = self
            .render_frame
            .block_glyphs
            .iter_mut()
            .find(|(id, _)| *id == block_id)
        {
            entry.1 = new_glyphs;
        }
        if let Some(entry) = self
            .render_frame
            .block_images
            .iter_mut()
            .find(|(id, _)| *id == block_id)
        {
            entry.1 = new_images;
        }
        if let Some(entry) = self
            .render_frame
            .block_decorations
            .iter_mut()
            .find(|(id, _)| *id == block_id)
        {
            entry.1 = new_decos;
        }
        if let Some(entry) = self
            .render_frame
            .block_glyph_keys
            .iter_mut()
            .find(|(id, _)| *id == block_id)
        {
            entry.1 = new_keys;
        }

        self.rebuild_flat_frame(service);
        apply_zoom(&mut self.render_frame, self.zoom);
        &self.render_frame
    }

    /// Lightweight render that only updates cursor/selection
    /// decorations.
    ///
    /// Reuses the existing glyph quads and images from the last
    /// full `render()`. Use when only the cursor blinked or the
    /// selection changed. Falls back to a full [`render`](Self::render)
    /// if the scroll offset or zoom changed in the meantime.
    pub fn render_cursor_only(&mut self, service: &mut TextFontService) -> &RenderFrame {
        if (self.scroll_offset - self.rendered_scroll_offset).abs() > 0.001
            || render_window_drifted(self.render_window, self.rendered_window)
            || (self.zoom - self.rendered_zoom).abs() > 0.001
        {
            return self.render(service);
        }

        // Defensive: if any atlas eviction has happened since the last
        // full render, our cached glyph quads' atlas coordinates may
        // refer to slots reallocated for unrelated glyphs. Fall back
        // to a fresh render rather than painting garbled text.
        if service.eviction_epoch != self.render_frame.atlas_eviction_epoch {
            return self.render(service);
        }

        // Keep cached glyphs alive in the shared atlas. Cursor blinks
        // and selection-only changes paint the cached `render_frame.glyphs`
        // without ever calling `cache.get`, so the LRU sees those
        // glyphs as idle and ages them out under sustained activity
        // in *other* widgets sharing the atlas. Touching here closes
        // that gap so the eviction fallback above stays a safety net
        // rather than a hot path.
        service.touch_glyphs(&self.render_frame.glyph_keys);

        self.render_frame.decorations.retain(|d| {
            !matches!(
                d.kind,
                DecorationKind::Cursor | DecorationKind::Selection | DecorationKind::CellSelection
            )
        });

        let effective_vw = self.viewport_width / self.zoom;
        let effective_vh = self.viewport_height / self.zoom;
        let mut cursor_decos = crate::render::cursor::generate_cursor_decorations(
            &self.flow_layout,
            &self.cursors,
            self.scroll_offset,
            self.cursor_color,
            self.selection_color,
            effective_vw,
            effective_vh,
        );
        apply_zoom_decorations(&mut cursor_decos, self.zoom);
        self.render_frame.decorations.extend(cursor_decos);

        &self.render_frame
    }

    fn rebuild_flat_frame(&mut self, service: &mut TextFontService) {
        self.render_frame.glyphs.clear();
        self.render_frame.images.clear();
        self.render_frame.decorations.clear();
        self.render_frame.glyph_keys.clear();
        for (_, glyphs) in &self.render_frame.block_glyphs {
            self.render_frame.glyphs.extend_from_slice(glyphs);
        }
        for (_, images) in &self.render_frame.block_images {
            self.render_frame.images.extend_from_slice(images);
        }
        for (_, decos) in &self.render_frame.block_decorations {
            self.render_frame.decorations.extend_from_slice(decos);
        }
        for (_, keys) in &self.render_frame.block_glyph_keys {
            self.render_frame.glyph_keys.extend_from_slice(keys);
        }
        // Keep the cached glyphs alive in the shared atlas. Without
        // this, blocks that are still visible through cached quads
        // but never re-rasterized this frame would age out under the
        // 120-generation LRU and have their atlas slots reallocated
        // to unrelated glyphs — corrupting every paint that reuses
        // these quads (the editor-and-viewer-mangled-together bug).
        service.touch_glyphs(&self.render_frame.glyph_keys);

        for item in &self.flow_layout.flow_order {
            match item {
                FlowItem::Table { table_id, .. } => {
                    if let Some(table) = self.flow_layout.tables.get(table_id) {
                        let decos = crate::layout::table::generate_table_decorations(
                            table,
                            self.scroll_offset,
                        );
                        self.render_frame.decorations.extend(decos);
                    }
                }
                FlowItem::Frame { frame_id, .. } => {
                    if let Some(frame) = self.flow_layout.frames.get(frame_id) {
                        crate::render::frame::append_frame_table_decorations(
                            frame,
                            0.0,
                            0.0,
                            self.scroll_offset,
                            &mut self.render_frame.decorations,
                        );
                        crate::render::frame::append_frame_border_decorations(
                            frame,
                            self.scroll_offset,
                            &mut self.render_frame.decorations,
                        );
                    }
                }
                FlowItem::Block { .. } => {}
            }
        }

        let effective_vw = self.viewport_width / self.zoom;
        let effective_vh = self.viewport_height / self.zoom;
        let cursor_decos = crate::render::cursor::generate_cursor_decorations(
            &self.flow_layout,
            &self.cursors,
            self.scroll_offset,
            self.cursor_color,
            self.selection_color,
            effective_vw,
            effective_vh,
        );
        self.render_frame.decorations.extend(cursor_decos);

        self.render_frame.atlas_dirty = service.atlas.dirty;
        self.render_frame.atlas_width = service.atlas.width;
        self.render_frame.atlas_height = service.atlas.height;
        if service.atlas.dirty {
            let pixels = &service.atlas.pixels;
            let needed = (service.atlas.width * service.atlas.height * 4) as usize;
            self.render_frame.atlas_pixels.resize(needed, 0);
            let copy_len = needed.min(pixels.len());
            self.render_frame.atlas_pixels[..copy_len].copy_from_slice(&pixels[..copy_len]);
            service.atlas.dirty = false;
        }
    }

    // ── Single-line layout ──────────────────────────────────────

    /// Lay out a single line of text and return GPU-ready glyph
    /// quads. Fast path for labels, tooltips, overlays — anything
    /// that doesn't need the full document pipeline.
    ///
    /// If `max_width` is set and the shaped text exceeds it, the
    /// output is truncated with an ellipsis character. Glyph quads
    /// are positioned with the top-left at `(0, 0)`.
    ///
    /// `raster_scale` densifies glyph bitmaps for content drawn under
    /// a scale transform (pass `1.0` for unscaled UI): rasterization
    /// happens at `size × scale_factor × raster_scale` physical pixels
    /// while every returned metric and `screen` rect stays in logical
    /// pixels — layout is identical at every raster scale.
    pub fn layout_single_line(
        &mut self,
        service: &mut TextFontService,
        text: &str,
        format: &TextFormat,
        max_width: Option<f32>,
        raster_scale: f32,
    ) -> SingleLineResult {
        let empty = SingleLineResult {
            width: 0.0,
            height: 0.0,
            baseline: 0.0,
            underline_offset: 0.0,
            underline_thickness: 0.0,
            glyphs: Vec::new(),
            glyph_keys: Vec::new(),
            spans: Vec::new(),
        };

        if text.is_empty() {
            return empty;
        }

        let font_point_size = format.font_size.map(|s| s as u32);
        let resolved = match resolve_font(
            &service.font_registry,
            format.font_family.as_deref(),
            format.font_weight,
            format.font_bold,
            format.font_italic,
            font_point_size,
            service.scale_factor,
            1.0, // standalone shaper: caller's explicit size is already theme-scaled
        ) {
            Some(r) => r,
            None => return empty,
        };

        let metrics = match font_metrics_px(&service.font_registry, &resolved) {
            Some(m) => m,
            None => return empty,
        };
        let line_height = metrics.ascent + metrics.descent + metrics.leading;
        let baseline = metrics.ascent;

        let features = to_harfrust_features(&format.features);
        let runs: Vec<_> = bidi_runs(text)
            .into_iter()
            .filter_map(|br| {
                let slice = text.get(br.byte_range.clone())?;
                shape_text_with_fallback(
                    &service.font_registry,
                    &resolved,
                    slice,
                    br.byte_range.start,
                    br.direction,
                    &features,
                )
            })
            .collect();

        if runs.is_empty() {
            return empty;
        }

        let total_advance: f32 = runs.iter().map(|r| r.advance_width).sum();

        let (truncate_at_visual_index, final_width, ellipsis_run) = if let Some(max_w) = max_width
            && total_advance > max_w
        {
            let ellipsis_run = shape_text(&service.font_registry, &resolved, "\u{2026}", 0);
            let ellipsis_width = ellipsis_run
                .as_ref()
                .map(|r| r.advance_width)
                .unwrap_or(0.0);
            let budget = (max_w - ellipsis_width).max(0.0);

            let mut used = 0.0f32;
            let mut count = 0usize;
            'outer: for run in &runs {
                for g in &run.glyphs {
                    if used + g.x_advance > budget {
                        break 'outer;
                    }
                    used += g.x_advance;
                    count += 1;
                }
            }

            (Some(count), used + ellipsis_width, ellipsis_run)
        } else {
            (None, total_advance, None)
        };

        let text_color = format.color.unwrap_or(self.text_color);
        let glyph_capacity: usize = runs.iter().map(|r| r.glyphs.len()).sum();
        let mut quads = Vec::with_capacity(glyph_capacity + 1);
        let mut keys = Vec::with_capacity(glyph_capacity + 1);
        let mut pen_x = 0.0f32;
        let mut emitted = 0usize;

        'emit: for run in &runs {
            for glyph in &run.glyphs {
                if let Some(limit) = truncate_at_visual_index
                    && emitted >= limit
                {
                    break 'emit;
                }
                rasterize_glyph_quad(
                    service,
                    glyph,
                    run,
                    pen_x,
                    baseline,
                    text_color,
                    raster_scale,
                    &mut quads,
                    &mut keys,
                );
                pen_x += glyph.x_advance;
                emitted += 1;
            }
        }

        if let Some(ref e_run) = ellipsis_run {
            for glyph in &e_run.glyphs {
                rasterize_glyph_quad(
                    service,
                    glyph,
                    e_run,
                    pen_x,
                    baseline,
                    text_color,
                    raster_scale,
                    &mut quads,
                    &mut keys,
                );
                pen_x += glyph.x_advance;
            }
        }

        SingleLineResult {
            width: final_width,
            height: line_height,
            baseline,
            underline_offset: metrics.underline_offset,
            underline_thickness: metrics.stroke_size,
            glyphs: quads,
            glyph_keys: keys,
            spans: Vec::new(),
        }
    }

    /// Lay out a multi-line paragraph by wrapping text at `max_width`.
    ///
    /// Multi-line counterpart to
    /// [`layout_single_line`](Self::layout_single_line). Shapes the
    /// input, breaks it at Unicode line-break opportunities
    /// (greedy, left-aligned), and rasterizes each line's glyphs
    /// into paragraph-local coordinates starting at `(0, 0)`.
    ///
    /// If `max_lines` is `Some(n)`, at most `n` lines are emitted
    /// and any remainder is silently dropped.
    ///
    /// See [`layout_single_line`](Self::layout_single_line) for the
    /// `raster_scale` contract (pass `1.0` for unscaled UI).
    pub fn layout_paragraph(
        &mut self,
        service: &mut TextFontService,
        text: &str,
        format: &TextFormat,
        max_width: f32,
        max_lines: Option<usize>,
        raster_scale: f32,
    ) -> ParagraphResult {
        let empty = ParagraphResult {
            width: 0.0,
            height: 0.0,
            baseline_first: 0.0,
            line_count: 0,
            line_height: 0.0,
            underline_offset: 0.0,
            underline_thickness: 0.0,
            glyphs: Vec::new(),
            glyph_keys: Vec::new(),
            spans: Vec::new(),
        };

        if text.is_empty() || max_width <= 0.0 {
            return empty;
        }

        let font_point_size = format.font_size.map(|s| s as u32);
        let resolved = match resolve_font(
            &service.font_registry,
            format.font_family.as_deref(),
            format.font_weight,
            format.font_bold,
            format.font_italic,
            font_point_size,
            service.scale_factor,
            1.0, // standalone shaper: caller's explicit size is already theme-scaled
        ) {
            Some(r) => r,
            None => return empty,
        };

        let metrics = match font_metrics_px(&service.font_registry, &resolved) {
            Some(m) => m,
            None => return empty,
        };

        let features = to_harfrust_features(&format.features);
        let runs: Vec<_> = bidi_runs(text)
            .into_iter()
            .filter_map(|br| {
                let slice = text.get(br.byte_range.clone())?;
                shape_text_with_fallback(
                    &service.font_registry,
                    &resolved,
                    slice,
                    br.byte_range.start,
                    br.direction,
                    &features,
                )
            })
            .collect();

        if runs.is_empty() {
            return empty;
        }

        let hyphenator = format.hyphenation.and_then(|h| {
            shape_text(&service.font_registry, &resolved, "-", 0)
                .and_then(|r| r.glyphs.into_iter().next())
                .map(|glyph| Hyphenator {
                    glyph,
                    language: h.language,
                })
        });
        let lines = break_into_lines(
            runs,
            text,
            max_width,
            Alignment::Left,
            0.0,
            &metrics,
            hyphenator,
            // This path ran the bidi algorithm itself and shaped in
            // display order, so the runs must not be reordered again.
            RunOrder::AlreadyVisual,
        );

        let line_count = match max_lines {
            Some(n) => lines.len().min(n),
            None => lines.len(),
        };

        let text_color = format.color.unwrap_or(self.text_color);
        let mut quads: Vec<GlyphQuad> = Vec::new();
        let mut keys: Vec<crate::atlas::cache::GlyphCacheKey> = Vec::new();
        let mut y_top = 0.0f32;
        let mut max_line_width = 0.0f32;
        let baseline_first = metrics.ascent;

        for line in lines.iter().take(line_count) {
            if line.width > max_line_width {
                max_line_width = line.width;
            }
            let baseline_y = y_top + metrics.ascent;
            for run in &line.runs {
                let mut pen_x = run.x;
                let run_copy = run.shaped_run.clone();
                for glyph in &run_copy.glyphs {
                    rasterize_glyph_quad(
                        service,
                        glyph,
                        &run_copy,
                        pen_x,
                        baseline_y,
                        text_color,
                        raster_scale,
                        &mut quads,
                        &mut keys,
                    );
                    pen_x += glyph.x_advance;
                }
            }
            y_top += metrics.ascent + metrics.descent + metrics.leading;
        }

        let line_height = metrics.ascent + metrics.descent + metrics.leading;
        ParagraphResult {
            width: max_line_width,
            height: y_top,
            baseline_first,
            line_count,
            line_height,
            underline_offset: metrics.underline_offset,
            underline_thickness: metrics.stroke_size,
            glyphs: quads,
            glyph_keys: keys,
            spans: Vec::new(),
        }
    }

    /// Single-line layout with inline markup. See
    /// [`layout_single_line`](Self::layout_single_line) for the plain
    /// variant. Accepts parsed `[label](url)`, `*italic*`, and
    /// `**bold**` spans and annotates the output with per-span
    /// bounding rectangles for hit-testing.
    pub fn layout_single_line_markup(
        &mut self,
        service: &mut TextFontService,
        markup: &InlineMarkup,
        format: &TextFormat,
        max_width: Option<f32>,
        raster_scale: f32,
    ) -> SingleLineResult {
        if markup.spans.is_empty() {
            return SingleLineResult {
                width: 0.0,
                height: 0.0,
                baseline: 0.0,
                underline_offset: 0.0,
                underline_thickness: 0.0,
                glyphs: Vec::new(),
                glyph_keys: Vec::new(),
                spans: Vec::new(),
            };
        }

        let per_span: Vec<(SingleLineResult, &crate::layout::inline_markup::InlineSpan)> = markup
            .spans
            .iter()
            .map(|sp| {
                let fmt = merge_format(format, sp.attrs);
                let r = if sp.text.is_empty() {
                    SingleLineResult {
                        width: 0.0,
                        height: 0.0,
                        baseline: 0.0,
                        underline_offset: 0.0,
                        underline_thickness: 0.0,
                        glyphs: Vec::new(),
                        glyph_keys: Vec::new(),
                        spans: Vec::new(),
                    }
                } else {
                    self.layout_single_line(service, &sp.text, &fmt, None, raster_scale)
                };
                (r, sp)
            })
            .collect();

        let total_width: f32 = per_span.iter().map(|(r, _)| r.width).sum();
        let line_height = per_span
            .iter()
            .map(|(r, _)| r.height)
            .fold(0.0f32, f32::max);
        let baseline = per_span
            .iter()
            .map(|(r, _)| r.baseline)
            .fold(0.0f32, f32::max);
        // Carry underline metrics from the first non-empty span. Spans may
        // use different fonts but a single line only has one underline, so
        // the first span wins.
        let (underline_offset, underline_thickness) = per_span
            .iter()
            .map(|(r, _)| (r.underline_offset, r.underline_thickness))
            .find(|(_, t)| *t > 0.0)
            .unwrap_or((0.0, 0.0));

        let truncate = match max_width {
            Some(mw) if total_width > mw => Some(mw),
            _ => None,
        };

        let mut glyphs: Vec<GlyphQuad> = Vec::new();
        let mut all_keys: Vec<crate::atlas::cache::GlyphCacheKey> = Vec::new();
        let mut spans_out: Vec<LaidOutSpan> = Vec::new();
        let mut pen_x: f32 = 0.0;
        let effective_width = truncate.unwrap_or(total_width);

        for (r, sp) in &per_span {
            let remaining = (effective_width - pen_x).max(0.0);
            let span_visible_width = r.width.min(remaining);
            if span_visible_width <= 0.0 && r.width > 0.0 {
                spans_out.push(LaidOutSpan {
                    kind: if let Some(url) = sp.link_url.clone() {
                        LaidOutSpanKind::Link { url }
                    } else {
                        LaidOutSpanKind::Text
                    },
                    line_index: 0,
                    rect: [pen_x, 0.0, 0.0, line_height],
                    byte_range: sp.byte_range.clone(),
                });
                continue;
            }

            for (gi, g) in r.glyphs.iter().enumerate() {
                let g_right = pen_x + g.screen[0] + g.screen[2];
                if g_right > effective_width + 0.5 {
                    break;
                }
                let mut gq = g.clone();
                gq.screen[0] += pen_x;
                glyphs.push(gq);
                if let Some(k) = r.glyph_keys.get(gi) {
                    all_keys.push(*k);
                }
            }

            spans_out.push(LaidOutSpan {
                kind: if let Some(url) = sp.link_url.clone() {
                    LaidOutSpanKind::Link { url }
                } else {
                    LaidOutSpanKind::Text
                },
                line_index: 0,
                rect: [pen_x, 0.0, span_visible_width, line_height],
                byte_range: sp.byte_range.clone(),
            });

            pen_x += r.width;
            if truncate.is_some() && pen_x >= effective_width {
                break;
            }
        }

        SingleLineResult {
            width: effective_width,
            height: line_height,
            baseline,
            underline_offset,
            underline_thickness,
            glyphs,
            glyph_keys: all_keys,
            spans: spans_out,
        }
    }

    /// Paragraph layout with inline markup. Multi-line counterpart
    /// to [`layout_single_line_markup`](Self::layout_single_line_markup).
    /// Emits a [`LaidOutSpan`] for every link segment so the caller
    /// can hit-test against wrapped links.
    pub fn layout_paragraph_markup(
        &mut self,
        service: &mut TextFontService,
        markup: &InlineMarkup,
        format: &TextFormat,
        max_width: f32,
        max_lines: Option<usize>,
        raster_scale: f32,
    ) -> ParagraphResult {
        let empty = ParagraphResult {
            width: 0.0,
            height: 0.0,
            baseline_first: 0.0,
            line_count: 0,
            line_height: 0.0,
            underline_offset: 0.0,
            underline_thickness: 0.0,
            glyphs: Vec::new(),
            glyph_keys: Vec::new(),
            spans: Vec::new(),
        };

        if markup.spans.is_empty() || max_width <= 0.0 {
            return empty;
        }

        let mut flat = String::new();
        let mut span_flat_offsets: Vec<usize> = Vec::with_capacity(markup.spans.len());
        for sp in &markup.spans {
            span_flat_offsets.push(flat.len());
            flat.push_str(&sp.text);
        }
        if flat.is_empty() {
            return empty;
        }

        let base_point_size = format.font_size.map(|s| s as u32);
        let base_resolved = match resolve_font(
            &service.font_registry,
            format.font_family.as_deref(),
            format.font_weight,
            format.font_bold,
            format.font_italic,
            base_point_size,
            service.scale_factor,
            1.0, // standalone shaper: caller's explicit size is already theme-scaled
        ) {
            Some(r) => r,
            None => return empty,
        };
        let metrics = match font_metrics_px(&service.font_registry, &base_resolved) {
            Some(m) => m,
            None => return empty,
        };

        let mut all_runs: Vec<ShapedRun> = Vec::new();
        for (span_idx, sp) in markup.spans.iter().enumerate() {
            if sp.text.is_empty() {
                continue;
            }
            let fmt = merge_format(format, sp.attrs);
            let span_point_size = fmt.font_size.map(|s| s as u32);
            let Some(resolved) = resolve_font(
                &service.font_registry,
                fmt.font_family.as_deref(),
                fmt.font_weight,
                fmt.font_bold,
                fmt.font_italic,
                span_point_size,
                service.scale_factor,
                1.0, // standalone shaper: caller's explicit size is already theme-scaled
            ) else {
                continue;
            };

            let flat_start = span_flat_offsets[span_idx];
            let features = to_harfrust_features(&fmt.features);
            for br in bidi_runs(&sp.text) {
                let slice = match sp.text.get(br.byte_range.clone()) {
                    Some(s) => s,
                    None => continue,
                };
                let Some(mut run) = shape_text_with_fallback(
                    &service.font_registry,
                    &resolved,
                    slice,
                    flat_start + br.byte_range.start,
                    br.direction,
                    &features,
                ) else {
                    continue;
                };
                if let Some(url) = sp.link_url.as_ref() {
                    run.is_link = true;
                    run.anchor_href = Some(url.clone());
                }
                all_runs.push(run);
            }
        }

        if all_runs.is_empty() {
            return empty;
        }

        let hyphenator = format.hyphenation.and_then(|h| {
            shape_text(&service.font_registry, &base_resolved, "-", 0)
                .and_then(|r| r.glyphs.into_iter().next())
                .map(|glyph| Hyphenator {
                    glyph,
                    language: h.language,
                })
        });
        let lines = break_into_lines(
            all_runs,
            &flat,
            max_width,
            Alignment::Left,
            0.0,
            &metrics,
            hyphenator,
            // This path ran the bidi algorithm itself and shaped in
            // display order, so the runs must not be reordered again.
            RunOrder::AlreadyVisual,
        );

        let line_count = match max_lines {
            Some(n) => lines.len().min(n),
            None => lines.len(),
        };

        let text_color = format.color.unwrap_or(self.text_color);
        let mut glyphs_out: Vec<GlyphQuad> = Vec::new();
        let mut keys_out: Vec<crate::atlas::cache::GlyphCacheKey> = Vec::new();
        let mut spans_out: Vec<LaidOutSpan> = Vec::new();
        let line_height = metrics.ascent + metrics.descent + metrics.leading;
        let mut y_top: f32 = 0.0;
        let mut max_line_width: f32 = 0.0;
        let baseline_first = metrics.ascent;

        for (line_idx, line) in lines.iter().take(line_count).enumerate() {
            if line.width > max_line_width {
                max_line_width = line.width;
            }
            let baseline_y = y_top + metrics.ascent;

            for pr in &line.runs {
                let run_copy = pr.shaped_run.clone();
                let mut pen_x = pr.x;
                for glyph in &run_copy.glyphs {
                    rasterize_glyph_quad(
                        service,
                        glyph,
                        &run_copy,
                        pen_x,
                        baseline_y,
                        text_color,
                        raster_scale,
                        &mut glyphs_out,
                        &mut keys_out,
                    );
                    pen_x += glyph.x_advance;
                }

                if pr.decorations.is_link
                    && let Some(url) = pr.decorations.anchor_href.clone()
                {
                    let width = pr.shaped_run.advance_width;
                    spans_out.push(LaidOutSpan {
                        kind: LaidOutSpanKind::Link { url },
                        line_index: line_idx,
                        rect: [pr.x, y_top, width, line_height],
                        byte_range: pr.shaped_run.text_range.clone(),
                    });
                }
            }

            y_top += line_height;
        }

        ParagraphResult {
            width: max_line_width,
            height: y_top,
            baseline_first,
            line_count,
            line_height,
            underline_offset: metrics.underline_offset,
            underline_thickness: metrics.stroke_size,
            glyphs: glyphs_out,
            glyph_keys: keys_out,
            spans: spans_out,
        }
    }

    // ── Hit testing & character geometry ───────────────────────

    /// Map a screen-space point to a document position. Coordinates
    /// are relative to the widget's top-left corner; the scroll
    /// offset is applied internally. Returns `None` when the flow
    /// has no content.
    pub fn hit_test(&self, x: f32, y: f32) -> Option<HitTestResult> {
        crate::render::hit_test::hit_test(
            &self.flow_layout,
            self.scroll_offset,
            x / self.zoom,
            y / self.zoom,
        )
    }

    /// Per-character advance geometry within a laid-out block.
    ///
    /// Used by accessibility layers that need to expose character
    /// positions to screen readers (AccessKit's `character_positions`
    /// / `character_widths` on `Role::TextRun`). `char_start` and
    /// `char_end` are block-relative character offsets. Returns one
    /// entry per character in the range, with `position` measured
    /// in run-local coordinates (the first character sits at `0`).
    pub fn character_geometry(
        &self,
        block_id: usize,
        char_start: usize,
        char_end: usize,
    ) -> Vec<CharacterGeometry> {
        // x for `offset` against a sorted, offset-deduped stop list: exact match,
        // else the nearer of the two bracketing stops (lower offset wins a tie) —
        // the same rule `LayoutLine::x_for_offset` applies, but O(log n) against a
        // shared build instead of an O(n) rebuild-and-scan per character.
        fn x_in_sorted_stops(stops: &[(usize, f32)], offset: usize) -> f32 {
            if stops.is_empty() {
                return 0.0;
            }
            match stops.binary_search_by_key(&offset, |(o, _)| *o) {
                Ok(i) => stops[i].1,
                Err(i) => {
                    let left = i.checked_sub(1).map(|j| stops[j]);
                    let right = stops.get(i).copied();
                    match (left, right) {
                        (Some((lo, lx)), Some((ro, rx))) => {
                            if offset.abs_diff(lo) <= ro.abs_diff(offset) {
                                lx
                            } else {
                                rx
                            }
                        }
                        (Some((_, lx)), None) => lx,
                        (None, Some((_, rx))) => rx,
                        (None, None) => 0.0,
                    }
                }
            }
        }

        if char_start >= char_end {
            return Vec::new();
        }
        let block = match self.flow_layout.blocks.get(&block_id) {
            Some(b) => b,
            None => return Vec::new(),
        };

        let mut absolute: Vec<(usize, f32)> = Vec::with_capacity(char_end - char_start);
        for line in &block.lines {
            if line.char_range.end <= char_start || line.char_range.start >= char_end {
                continue;
            }
            let local_start = char_start.max(line.char_range.start);
            let local_end = char_end.min(line.char_range.end);
            // Build the line's caret stops ONCE and index into them, rather than
            // calling `x_for_offset` per character — which rebuilt the stop list
            // (O(runs+glyphs), one allocation) every call. The paint pass splits
            // a spell-checked line into a run per range, so `x_for_offset`-per-char
            // is O(chars × runs) per line and turns quadratic on a dense document
            // (this dominated the accessibility rebuild on a Lorem scene with tens
            // of thousands of ranges). Stable-sort by offset then dedup keeps
            // the first (leftmost) x per offset.
            //
            // At a direction boundary an offset has two stops at different x,
            // and this deliberately keeps the leftmost rather than following
            // affinity the way `x_for_offset` now does: these are character
            // *extents* for a screen reader, which has no caret and so no
            // affinity to consult. A stable choice matters more than which
            // side it lands on.
            let mut stops: Vec<(usize, f32)> =
                line.caret_stops().iter().map(|s| (s.offset, s.x)).collect();
            stops.sort_by_key(|(o, _)| *o);
            stops.dedup_by_key(|(o, _)| *o);
            for c in local_start..local_end {
                absolute.push((c, x_in_sorted_stops(&stops, c)));
            }
            if local_end == char_end {
                absolute.push((local_end, x_in_sorted_stops(&stops, local_end)));
            }
        }

        if absolute.is_empty() {
            return Vec::new();
        }

        absolute.sort_by_key(|(c, _)| *c);

        let base_x = absolute.first().map(|(_, x)| *x).unwrap_or(0.0);
        let mut out: Vec<CharacterGeometry> = Vec::with_capacity(absolute.len());
        for window in absolute.windows(2) {
            let (c, x) = window[0];
            let (_, x_next) = window[1];
            if c >= char_end {
                break;
            }
            out.push(CharacterGeometry {
                position: x - base_x,
                width: (x_next - x).max(0.0),
            });
        }
        out
    }

    /// Screen-space caret rectangle at a document position with the
    /// given affinity, as `[x, y, width, height]`. Feed this to the
    /// platform IME for composition window placement. For drawing the
    /// caret itself, use the `DecorationKind::Cursor` entry in
    /// [`RenderFrame::decorations`] instead.
    ///
    /// Affinity only changes the result at soft-wrap boundaries; at
    /// every other position the two affinities return the same rect.
    /// `CursorAffinity::Downstream` matches the pre-affinity behavior.
    pub fn caret_rect(&self, position: usize, affinity: crate::types::CursorAffinity) -> [f32; 4] {
        let mut rect = crate::render::hit_test::caret_rect(
            &self.flow_layout,
            self.scroll_offset,
            position,
            affinity,
        );
        rect[0] *= self.zoom;
        rect[1] *= self.zoom;
        rect[2] *= self.zoom;
        rect[3] *= self.zoom;
        rect
    }

    // ── Cursor & colors ────────────────────────────────────────

    /// Replace the cursor display with a single cursor.
    pub fn set_cursor(&mut self, cursor: &CursorDisplay) {
        self.cursors = vec![CursorDisplay {
            position: cursor.position,
            anchor: cursor.anchor,
            affinity: cursor.affinity,
            visible: cursor.visible,
            selected_cells: cursor.selected_cells.clone(),
        }];
    }

    /// Replace the cursor display with multiple cursors (multi-caret
    /// editing). Each cursor independently generates a caret and
    /// optional selection highlight.
    pub fn set_cursors(&mut self, cursors: &[CursorDisplay]) {
        self.cursors = cursors
            .iter()
            .map(|c| CursorDisplay {
                position: c.position,
                anchor: c.anchor,
                affinity: c.affinity,
                visible: c.visible,
                selected_cells: c.selected_cells.clone(),
            })
            .collect();
    }

    /// Set the selection highlight color `[r, g, b, a]` in 0..=1
    /// space. Default: `[0.26, 0.52, 0.96, 0.3]` (translucent blue).
    pub fn set_selection_color(&mut self, color: [f32; 4]) {
        self.selection_color = color;
    }

    /// Set the caret color `[r, g, b, a]`. Default: black.
    pub fn set_cursor_color(&mut self, color: [f32; 4]) {
        self.cursor_color = color;
    }

    /// Set the default text color `[r, g, b, a]`, used when a
    /// fragment has no explicit `foreground_color`. Default: black.
    pub fn set_text_color(&mut self, color: [f32; 4]) {
        self.text_color = color;
    }

    /// Current default text color.
    pub fn text_color(&self) -> [f32; 4] {
        self.text_color
    }

    /// Set the background painted behind fenced code blocks when the
    /// block carries no explicit `background_color`. Hosts wire this
    /// from the active theme so dark / light swaps reach the cards.
    /// Default `[0.95, 0.95, 0.95, 1.0]` (light grey). Affects future
    /// `layout_full` / `relayout_block` calls; existing layouts keep
    /// their already-converted background until they next re-shape.
    pub fn set_code_block_background(&mut self, color: [f32; 4]) {
        self.code_block_background = color;
    }

    /// Current code-block background default.
    pub fn code_block_background(&self) -> [f32; 4] {
        self.code_block_background
    }

    /// Auto-hyphenate justified blocks (that don't set `hyphenate`
    /// explicitly) on future `layout_full` / `relayout_block` calls.
    /// Enable on prose/rich-text surfaces; leave off for single-line or
    /// label widgets. Default `false`.
    pub fn set_hyphenate_justified(&mut self, enabled: bool) {
        self.hyphenate_justified = enabled;
    }

    /// Whether justified blocks are auto-hyphenated.
    pub fn hyphenate_justified(&self) -> bool {
        self.hyphenate_justified
    }

    /// Set the foreground used for monospaced runs (inline `code`,
    /// fenced code blocks) that carry no explicit `foreground_color`.
    /// `None` (default) keeps the engine's `text_color`. Hosts wire
    /// this from the active theme alongside `set_code_block_background`.
    pub fn set_code_block_foreground(&mut self, color: Option<[f32; 4]>) {
        self.code_block_foreground = color;
    }

    /// Current code-block foreground override.
    pub fn code_block_foreground(&self) -> Option<[f32; 4]> {
        self.code_block_foreground
    }

    /// Set the foreground used for hyperlink runs that carry no explicit
    /// `foreground_color`. `None` (default) keeps the engine's
    /// `text_color` — i.e. links that look like prose. Hosts wire this
    /// from the active theme, alongside `set_code_block_foreground`.
    ///
    /// Changing it needs a full relayout: the colour is baked into the
    /// shaped runs at layout time, not resolved at paint.
    pub fn set_link_foreground(&mut self, color: Option<[f32; 4]>) {
        self.link_foreground = color;
    }

    /// Current link foreground override.
    pub fn link_foreground(&self) -> Option<[f32; 4]> {
        self.link_foreground
    }

    /// Set the echo / masking character for secure (password) fields.
    ///
    /// When `Some(c)`, every character laid out by future `layout_full`
    /// calls is replaced with `c` before shaping, so the real text never
    /// reaches the shaper or the glyph atlas. `None` (default) lays text
    /// out verbatim. One echo char is emitted per source `char`,
    /// preserving char counts so caret / selection / hit-test (all
    /// char-indexed) stay aligned with the host document's positions.
    ///
    /// Affects future `layout_full` calls; existing layouts keep their
    /// already-converted glyphs until they next re-shape. The incremental
    /// `relayout_block` path takes pre-converted [`BlockLayoutParams`], so
    /// hosts driving that path must thread the same echo char through
    /// their own [`crate::bridge::BridgeOptions`].
    pub fn set_echo_char(&mut self, echo: Option<char>) {
        self.echo_char = echo;
    }

    /// Current echo / masking character, if any.
    pub fn echo_char(&self) -> Option<char> {
        self.echo_char
    }

    // ── Scrolling helpers ──────────────────────────────────────

    /// Visual position and height of a laid-out block. Returns
    /// `None` if `block_id` is not in the current layout.
    pub fn block_visual_info(&self, block_id: usize) -> Option<BlockVisualInfo> {
        let block = self.flow_layout.blocks.get(&block_id)?;
        Some(BlockVisualInfo {
            block_id,
            y: block.y,
            height: block.height,
        })
    }

    /// The reading direction of the text *at* `position`.
    ///
    /// This is the direction of the bidi run the caret sits in, not the
    /// paragraph's — inside an English quotation in an Arabic paragraph
    /// it reports left-to-right. That is what an arrow key needs: which
    /// way the caret travels visually when it steps one character
    /// forward logically.
    ///
    /// Falls back to the paragraph direction at a position no run
    /// covers (an empty block, or the very end of the text), and to
    /// `LeftToRight` when there is no layout at all.
    pub fn direction_at(&self, position: usize) -> TextDirection {
        let Some(block) = self.block_containing(position) else {
            return TextDirection::LeftToRight;
        };
        let offset = position.saturating_sub(block.position);

        for line in &block.lines {
            if offset < line.char_range.start || offset > line.char_range.end {
                continue;
            }
            // Compare against each run's own cluster span rather than
            // asking `cluster_end` per glyph: that scans every glyph on
            // the line, which made this quadratic in line length on a
            // path every arrow keypress runs.
            for run in &line.runs {
                let mut lo = usize::MAX;
                let mut hi = 0usize;
                for g in &run.shaped_run.glyphs {
                    let c = g.cluster as usize;
                    lo = lo.min(c);
                    hi = hi.max(c);
                }
                if lo == usize::MAX {
                    continue;
                }
                // `hi` is the last cluster's *start*; the run reaches at
                // least one character past it.
                if offset >= lo && offset <= hi.max(lo) {
                    return run.shaped_run.direction;
                }
            }
        }
        block.base_direction
    }

    /// The base direction of the paragraph containing `position`.
    ///
    /// Home and End want this one rather than [`Self::direction_at`]: they move
    /// to the logical ends of the line, and which visual edge those land
    /// on is a property of the paragraph, not of whatever run the caret
    /// happens to be sitting in.
    pub fn paragraph_direction_at(&self, position: usize) -> TextDirection {
        self.block_containing(position)
            .map(|b| b.base_direction)
            .unwrap_or(TextDirection::LeftToRight)
    }

    /// The document positions of the start and end of the *visual* line
    /// containing `position` — i.e. what Home and End should move to.
    ///
    /// These are logical ends: the start is the lowest character offset
    /// on the line whichever screen edge that sits on. Asking the
    /// question this way rather than hit-testing a far-off-screen x
    /// keeps Home and End correct in right-to-left paragraphs, where the
    /// logical start is drawn on the right, and avoids depending on how
    /// a hit-test clamps coordinates outside the text.
    ///
    /// `affinity` picks the line at a soft-wrap boundary, where one
    /// position belongs to both the end of one line and the start of the
    /// next. Returns `None` if there is no layout for `position`.
    pub fn visual_line_range_at(
        &self,
        position: usize,
        affinity: crate::types::CursorAffinity,
    ) -> Option<(usize, usize)> {
        let block = self.block_containing(position)?;
        let offset = position.saturating_sub(block.position);

        let mut candidates = block
            .lines
            .iter()
            .filter(|l| offset >= l.char_range.start && offset <= l.char_range.end);
        let first = candidates.next()?;

        // Two lines can claim a boundary offset. Per `CursorAffinity`:
        // Downstream renders at the END of the previous wrap line,
        // Upstream at the START of the next one.
        let line = match candidates.next() {
            Some(second) if affinity == crate::types::CursorAffinity::Upstream => second,
            Some(_) => first,
            None => first,
        };

        Some((
            block.position + line.char_range.start,
            block.position + line.char_range.end,
        ))
    }

    /// The laid-out block whose character range covers `position`.
    ///
    /// Searches top-level blocks, table cells and frames, so a caret
    /// inside a table or a blockquote resolves like any other.
    fn block_containing(&self, position: usize) -> Option<&crate::layout::block::BlockLayout> {
        // `end` is inclusive so a caret at the very end of a block still
        // resolves, but that makes a block boundary match *two* blocks.
        // `blocks` is a HashMap, so picking whichever `find` reached
        // first made the answer depend on hash order — Home/End and the
        // arrow keys behaved differently from run to run at every
        // paragraph start. Prefer a block that strictly contains the
        // position, and fall back to a boundary match only if none does.
        let strictly_inside = |b: &crate::layout::block::BlockLayout| {
            let end = block_end(b);
            position >= b.position && position < end
        };
        let covers = |b: &crate::layout::block::BlockLayout| {
            position >= b.position && position <= block_end(b)
        };

        fn block_end(b: &crate::layout::block::BlockLayout) -> usize {
            b.lines
                .last()
                .map(|l| b.position + l.char_range.end)
                .unwrap_or(b.position)
        }

        // Deterministic tie-break among boundary matches: the latest
        // block that starts at or before the position.
        fn best<'b>(
            acc: Option<&'b crate::layout::block::BlockLayout>,
            b: &'b crate::layout::block::BlockLayout,
        ) -> Option<&'b crate::layout::block::BlockLayout> {
            match acc {
                Some(prev) if prev.position >= b.position => Some(prev),
                _ => Some(b),
            }
        }

        if let Some(b) = self
            .flow_layout
            .blocks
            .values()
            .filter(|b| strictly_inside(b))
            .fold(None, best)
        {
            return Some(b);
        }
        if let Some(b) = self
            .flow_layout
            .blocks
            .values()
            .filter(|b| covers(b))
            .fold(None, best)
        {
            return Some(b);
        }
        for table in self.flow_layout.tables.values() {
            for cell in &table.cell_layouts {
                if let Some(b) = cell.blocks.iter().find(|b| covers(b)) {
                    return Some(b);
                }
            }
        }
        for frame in self.flow_layout.frames.values() {
            if let Some(b) = frame.blocks.iter().find(|b| covers(b)) {
                return Some(b);
            }
        }
        None
    }

    /// Whether `position` sits on a direction boundary — a place where
    /// an LTR run meets an RTL one and the caret has two possible x on
    /// the same line.
    ///
    /// The widget layer uses this to decide whether moving the caret
    /// here has to choose a side: at an ordinary position affinity makes
    /// no difference and can be left alone, but at a seam the caret
    /// jumps across the line if it carries the wrong one.
    pub fn is_direction_boundary_at(&self, position: usize) -> bool {
        let Some(block) = self.block_containing(position) else {
            return false;
        };
        let offset = position.saturating_sub(block.position);
        block
            .lines
            .iter()
            .filter(|l| offset >= l.char_range.start && offset <= l.char_range.end)
            .any(|l| l.is_direction_boundary(offset))
    }

    /// Whether a block lives inside any table cell.
    pub fn is_block_in_table(&self, block_id: usize) -> bool {
        self.flow_layout.tables.values().any(|table| {
            table
                .cell_layouts
                .iter()
                .any(|cell| cell.blocks.iter().any(|b| b.block_id == block_id))
        })
    }

    /// Scroll so that `position` is visible, placing it roughly one
    /// third from the top of the viewport. Returns the new offset.
    /// Affinity defaults to `Downstream` since scroll targeting picks
    /// any acceptable line for the position.
    pub fn scroll_to_position(&mut self, position: usize) -> f32 {
        let rect = crate::render::hit_test::caret_rect(
            &self.flow_layout,
            self.scroll_offset,
            position,
            crate::types::CursorAffinity::Downstream,
        );
        let target_y = rect[1] + self.scroll_offset - self.viewport_height / (3.0 * self.zoom);
        self.scroll_offset = target_y.max(0.0);
        self.scroll_offset
    }

    /// Scroll the minimum amount needed to make the current caret
    /// visible. Call after arrow-key / click / typing. Returns
    /// `Some(new_offset)` if the scroll moved, `None` otherwise.
    ///
    /// The caret this reads is the one last handed to [`set_cursor`](Self::set_cursor),
    /// which an editor typically refreshes once per frame — so calling this
    /// from inside a key handler corrects for wherever the caret was on the
    /// *previous* frame, not where the keystroke just put it. Editors that move
    /// a cursor and reveal it in the same breath want
    /// [`ensure_position_visible`](Self::ensure_position_visible), which takes
    /// the position outright.
    pub fn ensure_caret_visible(&mut self) -> Option<f32> {
        if self.cursors.is_empty() {
            return None;
        }
        let pos = self.cursors[0].position;
        let affinity = self.cursors[0].affinity;
        self.ensure_position_visible(pos, affinity)
    }

    /// Scroll the minimum amount needed to make `position` visible. Returns
    /// `Some(new_offset)` if the scroll moved, `None` otherwise.
    ///
    /// The explicit-position form of [`ensure_caret_visible`](Self::ensure_caret_visible),
    /// for the common editor shape where a key handler moves its own cursor and
    /// then reveals it: the flow's cached cursor is a frame behind at that
    /// moment, and correcting against it leaves the caret the keystroke just
    /// moved sitting outside the viewport until the *next* keystroke.
    pub fn ensure_position_visible(
        &mut self,
        position: usize,
        affinity: crate::types::CursorAffinity,
    ) -> Option<f32> {
        let rect = crate::render::hit_test::caret_rect(
            &self.flow_layout,
            self.scroll_offset,
            position,
            affinity,
        );
        let caret_screen_y = rect[1];
        let caret_screen_bottom = caret_screen_y + rect[3];
        let effective_vh = self.viewport_height / self.zoom;
        let margin = 10.0 / self.zoom;
        let old_offset = self.scroll_offset;

        if caret_screen_y < 0.0 {
            self.scroll_offset += caret_screen_y - margin;
            self.scroll_offset = self.scroll_offset.max(0.0);
        } else if caret_screen_bottom > effective_vh {
            self.scroll_offset += caret_screen_bottom - effective_vh + margin;
        }

        if (self.scroll_offset - old_offset).abs() > 0.001 {
            Some(self.scroll_offset)
        } else {
            None
        }
    }
}

impl Default for DocumentFlow {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "text-document")]
enum FlowItemKind {
    Block(BlockLayoutParams),
    Table(TableLayoutParams),
    Frame(FrameLayoutParams),
}

/// Rasterize a single glyph into the service's atlas and append a
/// `GlyphQuad` to the output vec. Shared between
/// [`DocumentFlow::layout_single_line`] and
/// [`DocumentFlow::layout_paragraph`] (plus the markup variants).
///
/// `raster_scale` densifies the bitmap without touching layout: the
/// glyph is rasterized at `size × scale_factor × raster_scale`
/// physical pixels while the emitted `screen` rect stays in logical
/// pixels (divided by the *total* scale), so content drawn under a
/// scale transform (scene zoom) samples a matching-resolution bitmap
/// instead of stretching a 1× raster. Scaled rasters are unhinted —
/// glyph positions come from shaping at the logical ppem.
#[allow(clippy::too_many_arguments)]
fn rasterize_glyph_quad(
    service: &mut TextFontService,
    glyph: &ShapedGlyph,
    run: &ShapedRun,
    pen_x: f32,
    baseline: f32,
    text_color: [f32; 4],
    raster_scale: f32,
    quads: &mut Vec<GlyphQuad>,
    glyph_keys: &mut Vec<crate::atlas::cache::GlyphCacheKey>,
) {
    use crate::atlas::cache::GlyphCacheKey;
    use crate::atlas::rasterizer::rasterize_glyph;

    if glyph.glyph_id == 0 {
        return;
    }

    let entry = match service.font_registry.get(glyph.font_face_id) {
        Some(e) => e,
        None => return,
    };

    let raster_scale = if raster_scale > 0.0 {
        raster_scale
    } else {
        1.0
    };
    let hinted = raster_scale == 1.0;
    let sf = service.scale_factor.max(f32::MIN_POSITIVE);
    let inv_total = 1.0 / (sf * raster_scale);
    let physical_size_px = run.size_px * sf * raster_scale;
    let cache_key = GlyphCacheKey::with_weight(
        glyph.font_face_id,
        glyph.glyph_id,
        physical_size_px,
        run.weight as u32,
        hinted,
    );

    if service.glyph_cache.peek(&cache_key).is_none()
        && let Some(image) = rasterize_glyph(
            &mut service.scale_context,
            entry.bytes(),
            entry.face_index,
            entry.swash_cache_key,
            glyph.glyph_id,
            physical_size_px,
            run.weight as u32,
            hinted,
        )
        && image.width > 0
        && image.height > 0
    {
        let (alloc, evicted) = crate::atlas::allocate_or_evict(
            &mut service.atlas,
            &mut service.glyph_cache,
            image.width,
            image.height,
        );
        if evicted {
            service.eviction_epoch = service.eviction_epoch.wrapping_add(1);
        }
        if let Some(alloc) = alloc {
            let rect = alloc.rectangle;
            let atlas_x = rect.min.x as u32;
            let atlas_y = rect.min.y as u32;
            if image.is_color {
                service
                    .atlas
                    .blit_rgba(atlas_x, atlas_y, image.width, image.height, &image.data);
            } else {
                service
                    .atlas
                    .blit_mask(atlas_x, atlas_y, image.width, image.height, &image.data);
            }
            service.glyph_cache.insert(
                cache_key,
                crate::atlas::cache::CachedGlyph {
                    alloc_id: alloc.id,
                    atlas_x,
                    atlas_y,
                    width: image.width,
                    height: image.height,
                    placement_left: image.placement_left,
                    placement_top: image.placement_top,
                    is_color: image.is_color,
                    last_used: 0,
                },
            );
        }
    }

    if let Some(cached) = service.glyph_cache.get(&cache_key) {
        let logical_w = cached.width as f32 * inv_total;
        let logical_h = cached.height as f32 * inv_total;
        let logical_left = cached.placement_left as f32 * inv_total;
        let logical_top = cached.placement_top as f32 * inv_total;
        let screen_x = pen_x + glyph.x_offset + logical_left;
        let screen_y = baseline - glyph.y_offset - logical_top;
        let color = if cached.is_color {
            [1.0, 1.0, 1.0, 1.0]
        } else {
            text_color
        };
        quads.push(GlyphQuad {
            screen: [screen_x, screen_y, logical_w, logical_h],
            atlas: [
                cached.atlas_x as f32,
                cached.atlas_y as f32,
                cached.width as f32,
                cached.height as f32,
            ],
            color,
            is_color: cached.is_color,
        });
        glyph_keys.push(cache_key);
    }
}

/// Quantize an accumulated densification scale onto a geometric ladder of
/// 1.25ⁿ steps, `n ∈ [0, 6]` (so the value lands in `[1.0, ~3.81]`), for
/// glyph raster densification under zoom / external scale transforms.
///
/// The ladder bounds the number of distinct atlas entries a continuous
/// zoom gesture can create (7 buckets). The bucket value is derived from
/// an integer index, so the same input always yields the bit-identical
/// f32 — cache keys stay stable across frames — and the function is
/// idempotent (a bucket value maps to itself). Between buckets the
/// residual GPU scaling is at most ~12%, invisible under the glyph
/// atlas's linear filtering. Scales below 1 clamp to 1: zoomed-out text
/// relies on linear minification rather than rasterizing below logical
/// size.
///
/// Kept in lockstep with `teksilo_canvas::quantize_raster_scale` (scene
/// transform densification uses the same ladder).
pub fn quantize_raster_scale(scale: f32) -> f32 {
    if !scale.is_finite() || scale <= 1.0 {
        return 1.0;
    }
    const STEP: f32 = 1.25;
    /// 1.25⁶ ≈ 3.81 — the densest raster bucket. Deep zoom beyond it
    /// rides linear magnification; an unbounded ladder would explode
    /// atlas area quadratically.
    const MAX_BUCKET: i32 = 6;
    let bucket = ((scale.ln() / STEP.ln()).round() as i32).clamp(0, MAX_BUCKET);
    STEP.powi(bucket)
}

/// Scale all screen-space coordinates in a RenderFrame by `zoom`.
fn apply_zoom(frame: &mut RenderFrame, zoom: f32) {
    if (zoom - 1.0).abs() <= f32::EPSILON {
        return;
    }
    for q in &mut frame.glyphs {
        q.screen[0] *= zoom;
        q.screen[1] *= zoom;
        q.screen[2] *= zoom;
        q.screen[3] *= zoom;
    }
    for q in &mut frame.images {
        q.screen[0] *= zoom;
        q.screen[1] *= zoom;
        q.screen[2] *= zoom;
        q.screen[3] *= zoom;
    }
    apply_zoom_decorations(&mut frame.decorations, zoom);
}

/// Scale all screen-space coordinates in decoration rects by `zoom`.
fn apply_zoom_decorations(decorations: &mut [DecorationRect], zoom: f32) {
    if (zoom - 1.0).abs() <= f32::EPSILON {
        return;
    }
    for d in decorations.iter_mut() {
        d.rect[0] *= zoom;
        d.rect[1] *= zoom;
        d.rect[2] *= zoom;
        d.rect[3] *= zoom;
    }
}

/// Derive a per-span [`TextFormat`] from a base format and inline
/// markup attributes (bold / italic).
fn merge_format(base: &TextFormat, attrs: InlineAttrs) -> TextFormat {
    let mut fmt = base.clone();
    if attrs.is_bold() {
        fmt.font_bold = Some(true);
        if let Some(w) = fmt.font_weight
            && w < 600
        {
            fmt.font_weight = Some(700);
        } else if fmt.font_weight.is_none() {
            fmt.font_weight = Some(700);
        }
    }
    if attrs.is_italic() {
        fmt.font_italic = Some(true);
    }
    fmt
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::block::{BlockLayoutParams, FragmentParams};
    use crate::layout::paragraph::Alignment;
    use crate::types::{UnderlineStyle, VerticalAlignment};

    const NOTO_SANS: &[u8] = include_bytes!("../test-fonts/NotoSans-Variable.ttf");

    fn service() -> TextFontService {
        // Hermetic: don't pull in the host machine's fonts.
        let mut s = TextFontService::new_without_system_fonts();
        let face = s.register_font(NOTO_SANS);
        s.set_default_font(face, 16.0);
        s
    }

    fn block(id: usize, text: &str) -> BlockLayoutParams {
        BlockLayoutParams {
            base_direction: Default::default(),
            block_id: id,
            position: 0,
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
                footnote_marker: None,
                features: Vec::new(),
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
            hyphenation: None,
            checkbox: None,
            background_color: None,
        }
    }

    #[test]
    fn relayout_block_returns_no_layout_when_never_laid_out() {
        let svc = service();
        let mut flow = DocumentFlow::new();
        flow.set_viewport(400.0, 200.0);
        let err = flow.relayout_block(&svc, &block(1, "Hello")).unwrap_err();
        assert_eq!(err, RelayoutError::NoLayout);
    }

    #[test]
    fn relayout_block_returns_scale_dirty_after_scale_factor_change() {
        let mut svc = service();
        let mut flow = DocumentFlow::new();
        flow.set_viewport(400.0, 200.0);
        flow.layout_blocks(&svc, vec![block(1, "Hello")]);
        assert!(flow.has_layout());

        // Simulate a HiDPI transition on the shared service.
        svc.set_scale_factor(2.0);
        assert!(flow.layout_dirty_for_scale(&svc));

        let err = flow
            .relayout_block(&svc, &block(1, "Hello world"))
            .unwrap_err();
        assert_eq!(err, RelayoutError::ScaleDirty);
    }

    #[test]
    fn relayout_block_succeeds_after_fresh_layout_post_scale_change() {
        let mut svc = service();
        let mut flow = DocumentFlow::new();
        flow.set_viewport(400.0, 200.0);
        flow.layout_blocks(&svc, vec![block(1, "Hello")]);

        svc.set_scale_factor(2.0);
        // Caller is expected to re-run a full layout at the new
        // scale before issuing incremental updates.
        flow.layout_blocks(&svc, vec![block(1, "Hello")]);
        assert!(!flow.layout_dirty_for_scale(&svc));

        // Now the incremental path succeeds.
        flow.relayout_block(&svc, &block(1, "Hello world"))
            .expect("relayout_block must succeed after a fresh post-scale layout");
    }

    /// `block_params_for` converts a document block snapshot into layout params
    /// — the per-block seam the windowed streaming path is built on. The text
    /// must round-trip so the shaped row matches the document.
    #[test]
    fn block_params_for_converts_a_document_snapshot() {
        let flow = DocumentFlow::new();
        let doc = text_document::TextDocument::new();
        doc.set_plain_text("alpha\nbeta").unwrap();

        // Second block ("beta") starts after "alpha\n" — position 6.
        let snap = doc.snapshot_block_at_position(6).expect("block snapshot");
        let params = flow.block_params_for(&snap);

        assert_eq!(params.text, "beta", "the block text must round-trip");
        assert!(
            !params.fragments.is_empty(),
            "a non-empty block must convert to at least one fragment"
        );
    }

    /// It must use *this flow's* bridge options, not defaults — otherwise the
    /// windowed path would shape a block differently from `layout_full`. The
    /// echo char is the cheapest observable: with it set, the conversion masks
    /// the text.
    #[test]
    fn block_params_for_honours_the_flow_echo_char() {
        let mut flow = DocumentFlow::new();
        flow.set_echo_char(Some('•'));
        let doc = text_document::TextDocument::new();
        doc.set_plain_text("secret").unwrap();

        let snap = doc.snapshot_block_at_position(0).expect("block snapshot");
        let params = flow.block_params_for(&snap);

        assert!(
            params.fragments.iter().all(|f| !f.text.contains("secret")),
            "the flow's echo char must mask the plaintext, proving its own \
             bridge options are used"
        );
    }
}
