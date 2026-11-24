/// Opaque handle to a registered font face.
///
/// Obtained from [`crate::TextFontService::register_font`] or [`crate::TextFontService::register_font_as`].
/// Pass to [`crate::TextFontService::set_default_font`] to make it the default.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct FontFaceId(pub u32);

// ── Render output ───────────────────────────────────────────────

/// Everything needed to draw one frame.
///
/// Produced by [`crate::DocumentFlow::render`]. Contains glyph quads (textured rectangles
/// from the atlas), inline image placeholders, and decoration rectangles
/// (selections, cursor, underlines, table borders, etc.).
///
/// The adapter draws the frame in three passes:
/// 1. Upload `atlas_pixels` as a GPU texture (only when `atlas_dirty` is true).
/// 2. Draw each [`GlyphQuad`] as a textured rectangle from the atlas.
/// 3. Draw each [`DecorationRect`] as a colored rectangle.
pub struct RenderFrame {
    /// True if the atlas texture changed since the last frame (needs re-upload).
    pub atlas_dirty: bool,
    /// Atlas texture width in pixels.
    pub atlas_width: u32,
    /// Atlas texture height in pixels.
    pub atlas_height: u32,
    /// RGBA pixel data, row-major. Length = `atlas_width * atlas_height * 4`.
    pub atlas_pixels: Vec<u8>,
    /// One textured rectangle per visible glyph.
    pub glyphs: Vec<GlyphQuad>,
    /// Inline image placeholders. The adapter loads the actual image data
    /// (e.g., via `TextDocument::resource(name)`) and draws it at the given
    /// screen position.
    pub images: Vec<ImageQuad>,
    /// Decoration rectangles: selections, cursor, underlines, strikeouts,
    /// overlines, backgrounds, table borders, and cell backgrounds.
    pub decorations: Vec<DecorationRect>,
    /// Per-item glyph data for incremental updates: one entry per top-level
    /// block, table and frame drawn, in flow order.
    pub(crate) block_glyphs: Vec<(FlowItemKey, Vec<GlyphQuad>)>,
    /// Per-item text decorations (underlines, highlights, squiggles; NOT the
    /// cursor or selection, and NOT the table and top-level frame borders,
    /// which `rebuild_flat_frame` regenerates). A table's and a frame's carry
    /// the decorations of every block inside them.
    pub(crate) block_decorations: Vec<(FlowItemKey, Vec<DecorationRect>)>,
    /// Per-item image data for incremental updates.
    pub(crate) block_images: Vec<(FlowItemKey, Vec<ImageQuad>)>,
    /// Per-block height snapshot for detecting height changes in incremental render.
    pub(crate) block_heights: std::collections::HashMap<usize, f32>,
    /// Per-block glyph cache keys, parallel to [`Self::block_glyphs`]. Used by
    /// [`crate::DocumentFlow::render_cursor_only`] and
    /// [`crate::DocumentFlow::render_block_only`] to mark every cached
    /// glyph as still-in-use in the shared `GlyphCache` — otherwise
    /// glyphs reused via paint-cache hits (which never re-enter the
    /// `cache.get()` path that refreshes timestamps) would age out and
    /// their atlas slots could be reallocated for unrelated glyphs,
    /// silently corrupting the cached `GlyphQuad`s' atlas references.
    pub(crate) block_glyph_keys: Vec<(FlowItemKey, Vec<crate::atlas::cache::GlyphCacheKey>)>,
    /// Flat glyph cache keys, parallel to [`Self::glyphs`]. Rebuilt from
    /// [`Self::block_glyph_keys`] by `rebuild_flat_frame`; passed to
    /// [`crate::TextFontService::touch_glyphs`] on every cursor-only /
    /// block-only paint so the shared atlas keeps visible glyphs alive.
    pub(crate) glyph_keys: Vec<crate::atlas::cache::GlyphCacheKey>,
    /// Snapshot of [`crate::TextFontService::eviction_epoch`] at the
    /// moment this frame's atlas references were baked. Cursor-only
    /// and block-only paths compare against the service's current
    /// epoch and fall back to a full re-render if eviction has
    /// happened since — defensive safety net behind the `touch_glyphs`
    /// keep-alive mechanism.
    pub(crate) atlas_eviction_epoch: u64,
}

/// Which flow item a cached slice of a [`RenderFrame`] belongs to.
///
/// Blocks, tables and frames are numbered separately by the document (each
/// kind of entity has its own counter), so a bare id is ambiguous: a block and
/// a blockquote frame can both be number 2. Keyed by id alone, re-rendering
/// block 2 replaced the frame's cached glyphs with the block's, so the quote
/// vanished and the paragraph was drawn twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum FlowItemKey {
    Block(usize),
    Table(usize),
    Frame(usize),
}

/// A positioned glyph to draw as a textured quad from the atlas.
///
/// The adapter draws the rectangle at `screen` position, sampling from
/// the `atlas` rectangle in the atlas texture, tinted with `color`.
#[derive(Clone)]
#[non_exhaustive]
pub struct GlyphQuad {
    /// Screen position and size: `[x, y, width, height]` in pixels.
    pub screen: [f32; 4],
    /// Atlas source rectangle: `[x, y, width, height]` in atlas pixel coordinates.
    pub atlas: [f32; 4],
    /// Glyph color: `[r, g, b, a]`, 0.0-1.0.
    /// For normal text glyphs, this is the text color (default black).
    /// For color emoji, this is `[1, 1, 1, 1]` (color is baked into the atlas).
    pub color: [f32; 4],
    /// `true` if the atlas region for this glyph holds a pre-multiplied
    /// RGBA color bitmap (color emoji via COLR/CBDT/sbix). The renderer
    /// must sample `texture.rgb` directly instead of using the texture
    /// as an alpha mask tinted by [`color`](Self::color).
    pub is_color: bool,
}

/// An inline image placeholder.
///
/// text-typeset computes the position and size but does NOT load or rasterize
/// the image. The adapter retrieves the image data (e.g., from
/// `TextDocument::resource(name)`) and draws it as a separate texture.
#[derive(Clone)]
#[non_exhaustive]
pub struct ImageQuad {
    /// Screen position and size: `[x, y, width, height]` in pixels.
    ///
    /// The size is the laid-out one: an image wider than its line's measure
    /// is capped to that measure, its height scaled by the same factor, so
    /// this can be smaller than the size the document stores. A host that
    /// draws a resize grip or reports the size the writer sees takes it from
    /// here, not from the document.
    pub screen: [f32; 4],
    /// Image resource name (matches `FragmentContent::Image::name` from text-document).
    pub name: String,
    /// Document-absolute character offset of this image's single `U+FFFC`.
    ///
    /// The name alone cannot say *which* placement this is — a document may
    /// hold one picture in three places — so anything that has to answer "is
    /// THIS image inside the selection" needs the offset. Same value, derived
    /// the same way, as the offset `hit_test` reports for a click on it.
    pub char_offset: usize,
    /// How opaque to draw the picture, `0.0`–`1.0`. `1.0` unless the image
    /// lies outside a [`DimOutside`] range, where it is that range's
    /// `opacity`. A glyph carries the same information in its colour's
    /// alpha; an image has no colour, so it needs its own field.
    pub opacity: f32,
}

impl ImageQuad {
    /// An image placement: `screen` as `[x, y, width, height]`, the resource
    /// `name`, and the `char_offset` of its `U+FFFC`, drawn fully opaque.
    /// For a host that stands in for the typesetter, a test for instance.
    pub fn new(screen: [f32; 4], name: impl Into<String>, char_offset: usize) -> Self {
        Self {
            screen,
            name: name.into(),
            char_offset,
            opacity: 1.0,
        }
    }
}

/// Draw everything outside one range of the text faded.
///
/// The writing-comfort "focus" effect: the sentence or paragraph being
/// written stays at full strength and the rest of the text fades back. Set
/// per view with [`DocumentFlow::set_dim_outside`](crate::DocumentFlow::set_dim_outside).
///
/// It is applied while the frame is built, from each glyph's position in the
/// text, so it is exact for ligatures and right-to-left text and costs no
/// reshaping. What it dims is the **ink**: glyphs, inline images, list
/// markers, underlines, strikeouts, overlines and spell-check squiggles.
/// Backgrounds (a block's, a run's highlight), table and frame borders, the
/// caret and the selection keep their full strength: a background behind
/// the text is not the text, and a dimmed caret or selection would hide the
/// very thing the writer is acting on.
///
/// ## How the ink fades
///
/// With a page colour ([`toward`](Self::toward)), faded ink keeps its alpha
/// and its colour is mixed toward the page in sRGB: `opacity` of the ink's
/// own colour, the rest the page's. Without one, the ink's alpha is scaled
/// by `opacity` instead.
///
/// The two are not the same, and the page colour is what makes a strength
/// mean one thing on every theme. A renderer that blends in linear light (as
/// GPU pipelines over an sRGB surface do) turns an alpha of `0.35` into near
/// invisibility for black text on white and into a mild fade for light text
/// on a dark page. Mixing the colours in sRGB fades both by the same
/// perceived amount. Colour glyphs (emoji) and images have no single colour
/// to mix, so they always fade by alpha.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DimOutside {
    /// Document-absolute character offset where the bright range starts.
    pub start: usize,
    /// Document-absolute character offset where it ends (exclusive).
    ///
    /// `start == end` is a valid, empty range: everything is dimmed. That is
    /// what an empty paragraph asks for: there is nothing to keep bright, but
    /// the rest of the text must still fade.
    pub end: usize,
    /// How much of the ink outside the range remains, `0.0`–`1.0`. Clamped
    /// when used.
    pub opacity: f32,
    /// The colour the faded ink is drawn on, `[r, g, b, a]`, sRGB, 0.0–1.0.
    /// `None` fades by alpha. See the type's documentation.
    pub toward: Option<[f32; 4]>,
}

impl DimOutside {
    /// Keep `[start, end)` bright and fade the rest to `opacity`, by alpha
    /// until a page colour is given with [`toward`](Self::toward).
    pub fn new(start: usize, end: usize, opacity: f32) -> Self {
        Self {
            start,
            end,
            opacity,
            toward: None,
        }
    }

    /// Fade by mixing toward `page`, the colour the text is drawn on, rather
    /// than by alpha. A translucent `page` cannot be mixed toward (what shows
    /// through it is unknown), so it is ignored and the fade stays by alpha.
    pub fn toward(mut self, page: [f32; 4]) -> Self {
        self.toward = (page[3] >= 1.0).then_some(page);
        self
    }

    /// The factor for ink at document position `position`: `1.0` inside the
    /// range, the clamped opacity outside it.
    pub(crate) fn factor_at(&self, position: usize) -> f32 {
        if position >= self.start && position < self.end {
            1.0
        } else {
            self.dim_factor()
        }
    }

    /// The factor for ink that belongs to the whole block spanning
    /// `[block_start, block_end]` rather than to one character: its list or
    /// checkbox marker. The marker follows its paragraph: it is bright when
    /// the bright range touches the block, including an empty range sitting
    /// in it (the caret in an empty list item).
    pub(crate) fn factor_for_block(&self, block_start: usize, block_end: usize) -> f32 {
        let touches = if self.start == self.end {
            self.start >= block_start && self.start <= block_end
        } else {
            self.start <= block_end && self.end > block_start
        };
        if touches { 1.0 } else { self.dim_factor() }
    }

    fn dim_factor(&self) -> f32 {
        if self.opacity.is_finite() {
            self.opacity.clamp(0.0, 1.0)
        } else {
            1.0
        }
    }

    /// `color` faded by `factor` (one of the two above). `mixable` is false
    /// for ink with no colour of its own to mix (a colour glyph, whose quad
    /// colour is only a tint), which fades by alpha whatever the page.
    pub(crate) fn fade(&self, color: [f32; 4], factor: f32, mixable: bool) -> [f32; 4] {
        if factor >= 1.0 {
            return color;
        }
        match self.toward {
            Some(page) if mixable => {
                let mix = |ink: f32, paper: f32| ink * factor + paper * (1.0 - factor);
                [
                    mix(color[0], page[0]),
                    mix(color[1], page[1]),
                    mix(color[2], page[2]),
                    color[3],
                ]
            }
            _ => [color[0], color[1], color[2], color[3] * factor],
        }
    }

    /// The smallest range of positions whose look differs between `self` and
    /// `other`, as `[lo, hi)`. `None` when every position is drawn the same
    /// under both, and the whole space when the change is not confined to
    /// positions at all (a different opacity or page colour changes every
    /// faded glyph).
    pub(crate) fn changed_hull(&self, other: &DimOutside) -> Option<(usize, usize)> {
        if self == other {
            return None;
        }
        // The effective factors, not the raw opacities: NaN, which fades nothing, compares
        // unequal to everything without differing from it by more than any epsilon.
        if self.dim_factor() != other.dim_factor() || self.toward != other.toward {
            return Some((0, usize::MAX));
        }
        Some((self.start.min(other.start), self.end.max(other.end)))
    }
}

#[cfg(test)]
mod dim_outside_tests {
    use super::DimOutside;

    const INK: [f32; 4] = [0.1, 0.2, 0.3, 1.0];
    const PAGE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

    #[test]
    fn without_a_page_the_alpha_fades() {
        let dim = DimOutside::new(0, 0, 0.4);
        assert_eq!(dim.fade(INK, 0.4, true), [0.1, 0.2, 0.3, 0.4]);
    }

    #[test]
    fn with_a_page_the_colour_is_mixed_and_the_alpha_kept() {
        let dim = DimOutside::new(0, 0, 0.4).toward(PAGE);
        let [r, g, b, a] = dim.fade(INK, 0.4, true);
        assert!((r - (0.1 * 0.4 + 0.6)).abs() < 1e-6);
        assert!((g - (0.2 * 0.4 + 0.6)).abs() < 1e-6);
        assert!((b - (0.3 * 0.4 + 0.6)).abs() < 1e-6);
        assert_eq!(a, 1.0, "coverage is the glyph's, not the fade's");
    }

    #[test]
    fn ink_with_no_colour_of_its_own_fades_by_alpha_even_with_a_page() {
        let dim = DimOutside::new(0, 0, 0.4).toward(PAGE);
        assert_eq!(
            dim.fade([1.0, 1.0, 1.0, 1.0], 0.4, false),
            [1.0, 1.0, 1.0, 0.4]
        );
    }

    #[test]
    fn bright_ink_is_untouched() {
        let dim = DimOutside::new(0, 0, 0.4).toward(PAGE);
        assert_eq!(dim.fade(INK, 1.0, true), INK);
    }

    #[test]
    fn a_translucent_page_is_not_mixed_toward() {
        let dim = DimOutside::new(0, 0, 0.4).toward([1.0, 1.0, 1.0, 0.5]);
        assert_eq!(dim.toward, None);
    }

    #[test]
    fn a_new_page_colour_changes_everything() {
        let light = DimOutside::new(0, 5, 0.4).toward(PAGE);
        let dark = DimOutside::new(0, 5, 0.4).toward([0.1, 0.1, 0.1, 1.0]);
        assert_eq!(light.changed_hull(&dark), Some((0, usize::MAX)));
        assert_eq!(light.changed_hull(&light), None);
    }
}

/// A colored rectangle for decorations (underlines, selections, borders, etc.).
#[derive(Clone)]
#[non_exhaustive]
pub struct DecorationRect {
    /// Screen position and size: `[x, y, width, height]` in pixels.
    pub rect: [f32; 4],
    /// Color: `[r, g, b, a]`, 0.0-1.0.
    pub color: [f32; 4],
    /// What kind of decoration this rectangle represents.
    pub kind: DecorationKind,
}

/// The type of a [`DecorationRect`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecorationKind {
    /// Selection highlight (translucent background behind selected text).
    Selection,
    /// Cursor caret (thin vertical line at the insertion point).
    Cursor,
    /// Underline (below baseline, from font metrics).
    Underline,
    /// Strikethrough (at x-height, from font metrics).
    Strikeout,
    /// Overline (at ascent line).
    Overline,
    /// Generic background (e.g., frame borders).
    Background,
    /// Block-level background color.
    BlockBackground,
    /// Table border line.
    TableBorder,
    /// Table cell background color.
    TableCellBackground,
    /// Text-level background highlight (behind individual text runs).
    /// Adapters should draw these before glyph quads so text appears on top.
    TextBackground,
    /// Cell-level selection highlight (entire cell background when cells are
    /// selected as a rectangular region, as opposed to text within cells).
    CellSelection,
}

/// Underline style for text decorations.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum UnderlineStyle {
    /// No underline.
    #[default]
    None,
    /// Solid single underline.
    Single,
    /// Dashed underline.
    Dash,
    /// Dotted underline.
    Dot,
    /// Alternating dash-dot pattern.
    DashDot,
    /// Alternating dash-dot-dot pattern.
    DashDotDot,
    /// Wavy underline.
    Wave,
    /// Spell-check underline (wavy, typically red).
    SpellCheck,
}

/// Vertical alignment for characters (superscript, subscript, etc.).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum VerticalAlignment {
    /// Normal baseline alignment.
    #[default]
    Normal,
    /// Superscript: smaller size, shifted up.
    SuperScript,
    /// Subscript: smaller size, shifted down.
    SubScript,
}

// ── Hit testing ─────────────────────────────────────────────────

/// Disambiguates the two visual placements a single character position
/// can have at a soft-wrap boundary. A long paragraph that wraps across
/// lines K and K+1 has one character position N that sits at both the
/// END of line K and the START of line K+1; affinity picks which one
/// the caret renders at and which line `Home`/`End`-style navigation
/// considers "current".
///
/// Affinity is a display concern: it makes no sense without a layout
/// engine and a wrap width. It is never persisted with the text model
/// (cf. Cocoa `NSSelectionAffinity` on `NSTextView`, not on
/// `NSTextStorage`; Chromium `PositionWithAffinity` at the editing
/// layer, not on `Position`; same in Qt and CodeMirror).
///
/// At positions that are NOT wrap boundaries — the interior of a line,
/// the start of the first wrap-line, the end of the last wrap-line of
/// a paragraph — affinity is a no-op and the rendering is identical.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum CursorAffinity {
    /// Place the caret at the END of the previous wrap line. This is
    /// the visual "trailing" placement and the default for any
    /// position not produced by an upstream-side interaction.
    #[default]
    Downstream,
    /// Place the caret at the START of the next wrap line.
    Upstream,
}

/// Result of [`crate::DocumentFlow::hit_test`] - maps a screen-space point to a
/// document position.
#[non_exhaustive]
pub struct HitTestResult {
    /// Absolute character position in the document.
    pub position: usize,
    /// Which side of a soft-wrap boundary the click landed on. When
    /// the matched line's Y range contained the click and `position`
    /// equals that line's `char_range.start` AND the preceding line in
    /// the same block wraps at the same position, the click is on the
    /// upstream side of the boundary → `Upstream`. Otherwise
    /// `Downstream`. At non-wrap positions, the start of a line after a
    /// hard break included, the value is `Downstream` (default) and does
    /// not affect anything.
    pub affinity: CursorAffinity,
    /// Which block (paragraph) was hit, identified by stable block ID.
    pub block_id: usize,
    /// Character offset within the block (0 = start of block).
    pub offset_in_block: usize,
    /// What region of the layout was hit.
    pub region: HitRegion,
    /// Tooltip text if the hit position has a tooltip. None otherwise.
    pub tooltip: Option<String>,
    /// When non-None, the hit position is inside a table cell.
    /// Identifies the table by its stable table ID.
    /// None for hits on top-level blocks, frame blocks, or outside any table.
    pub table_id: Option<usize>,
}

/// What region of the layout a hit test landed in.
#[derive(Debug)]
pub enum HitRegion {
    /// Inside a text run (normal text content).
    Text,
    /// In the block's left margin area (before any text content).
    LeftMargin,
    /// In the block's indent area.
    Indent,
    /// On a table border line.
    TableBorder,
    /// Below all content in the document.
    BelowContent,
    /// Past the end of a line (to the right of the last character).
    PastLineEnd,
    /// On an inline image.
    Image { name: String },
    /// On a hyperlink.
    Link { href: String },
}

// ── Cursor display ──────────────────────────────────────────────

/// Cursor display state for rendering.
///
/// The adapter reads cursor position from text-document's `TextCursor`
/// and creates this struct to feed to [`crate::DocumentFlow::set_cursor`].
/// text-typeset uses it to generate caret and selection decorations
/// in the next [`crate::DocumentFlow::render`] call.
pub struct CursorDisplay {
    /// Cursor position (character offset in the document).
    pub position: usize,
    /// Selection anchor. Equals `position` when there is no selection.
    /// When different from `position`, the range `[min(anchor, position), max(anchor, position))`
    /// is highlighted as a selection.
    pub anchor: usize,
    /// Which side of a soft-wrap boundary the caret renders on (see
    /// [`CursorAffinity`]). At non-boundary positions this is a
    /// no-op; default `Downstream` (current behavior before affinity
    /// was introduced).
    pub affinity: CursorAffinity,
    /// Whether the caret is visible (false during the blink-off phase).
    /// The adapter manages the blink timer; text-typeset just respects this flag.
    pub visible: bool,
    /// When non-empty, render cell-level selection highlights instead of
    /// text-level selection. Each tuple is `(table_id, row, col)` identifying
    /// a selected cell. The adapter fills this from `TextCursor::selected_cells()`.
    pub selected_cells: Vec<(usize, usize, usize)>,
}

// ── Scrolling ───────────────────────────────────────────────────

/// Visual position and size of a laid-out table, and of its rows and
/// columns.
///
/// Returned by [`crate::DocumentFlow::table_visual_info`]. `x` / `y` are the
/// document-space top-left of the whole table box; `column_xs` and `row_ys`
/// are measured from that origin, so cell `(r, c)` occupies
/// `(x + column_xs[c], y + row_ys[r], column_content_widths[c], row_heights[r])`.
#[non_exhaustive]
pub struct TableVisualInfo {
    /// Table ID (matches `TableSnapshot::table_id`).
    pub table_id: usize,
    /// Document-space x of the table box's left edge, in pixels.
    pub x: f32,
    /// Document-space y of the table box's top edge, in pixels.
    pub y: f32,
    /// Total width of the table, in pixels.
    pub width: f32,
    /// Total height of the table including borders, in pixels.
    pub height: f32,
    /// Left edge of each column's content area, relative to `x`.
    pub column_xs: Vec<f32>,
    /// Width of each column's content area, in pixels.
    pub column_content_widths: Vec<f32>,
    /// Top edge of each row's content area, relative to `y`.
    pub row_ys: Vec<f32>,
    /// Height of each row's content area, in pixels.
    pub row_heights: Vec<f32>,
}

/// Visual position and size of a laid-out block.
///
/// Returned by [`crate::DocumentFlow::block_visual_info`] and, with the
/// block's lines, by [`crate::DocumentFlow::block_geometry`].
///
/// Three x positions matter, and they are not the same:
///
/// - [`x`](Self::x), the left edge of the column the block sits in;
/// - [`content_x`](Self::content_x), where the block's text starts, after its
///   left margin and any list or paragraph indent;
/// - [`marker_x`](Self::marker_x), where its list or checkbox marker is drawn,
///   in the indent area before the text.
///
/// Line rectangles returned by [`crate::DocumentFlow::block_geometry`] and
/// [`crate::DocumentFlow::block_line_geometry`] are relative to
/// [`content_x`](Self::content_x), not to [`x`](Self::x).
#[non_exhaustive]
pub struct BlockVisualInfo {
    /// Block ID (matches `BlockSnapshot::block_id`).
    pub block_id: usize,
    /// X position of the left edge of the block's column, relative to the
    /// document start, in pixels. It does **not** include the block's own left
    /// margin or indent: a rectangle around the block's text starts at
    /// [`content_x`](Self::content_x), and one that also covers the marker
    /// starts here.
    ///
    /// Zero for a block in the document's own column. A block inside a table
    /// cell or a frame has its own left origin: column 1 of a table does not
    /// start at the document's left edge.
    pub x: f32,
    /// X position where the block's text starts, relative to the document
    /// start, in pixels: [`x`](Self::x) plus the block's left margin and its
    /// list or paragraph indent. This is the origin of the `x` of every line
    /// rectangle and segment the line geometry reports for the block, and the
    /// origin the renderer paints glyphs from.
    ///
    /// Equal to [`x`](Self::x) for a block with no margin and no indent.
    pub content_x: f32,
    /// X position of the block's list or checkbox marker box, relative to the
    /// document start, in pixels, or `None` when the block has no marker.
    ///
    /// The marker is drawn in the indent area just before the text,
    /// right-aligned with a small gap, and never left of the block's left
    /// margin. When the indent is narrower than the marker, the marker starts
    /// at the left margin (`x` plus the margin) and its box overlaps the text.
    /// In a right-to-left block it is currently still drawn at the left.
    pub marker_x: Option<f32>,
    /// Advance width of the marker, in pixels; `0.0` when the block has no
    /// marker. The marker box is `marker_x` to `marker_x + marker_width`,
    /// on the block's first line.
    pub marker_width: f32,
    /// Y position of the block's top edge relative to the document start, in pixels.
    pub y: f32,
    /// Total height of the block including margins, in pixels.
    pub height: f32,
}

// ── OpenType features ───────────────────────────────────────────

/// An OpenType feature toggle applied during shaping.
///
/// `tag` is the 4-byte feature tag (e.g. `*b"liga"`, `*b"smcp"`,
/// `*b"tnum"`, `*b"ss01"`); `value` is the feature value — `0` disables
/// it, `1` enables it, and some features (e.g. `aalt`) take an index.
///
/// Script-mandated features (Arabic joining, Indic reordering, etc.)
/// always apply regardless of this list; these toggles control the
/// *discretionary* typographic features a caller wants on or off.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FontFeature {
    /// The 4-byte OpenType feature tag.
    pub tag: [u8; 4],
    /// Feature value: `0` = off, `1` = on, or a feature-specific index.
    pub value: u32,
}

impl FontFeature {
    /// A feature tag turned on (`value = 1`).
    pub const fn on(tag: [u8; 4]) -> Self {
        Self { tag, value: 1 }
    }

    /// A feature tag turned off (`value = 0`).
    pub const fn off(tag: [u8; 4]) -> Self {
        Self { tag, value: 0 }
    }

    /// A feature tag with an explicit value.
    pub const fn new(tag: [u8; 4], value: u32) -> Self {
        Self { tag, value }
    }
}

/// Hyphenation settings for line wrapping.
///
/// Presence (`Some`) enables hyphenation; the `language` selects the
/// Knuth-Liang dictionary. Soft hyphens (U+00AD) always break and render a
/// hyphen when enabled, regardless of language; dictionary hyphenation
/// applies only when the language's patterns are compiled in (otherwise it
/// silently falls back to soft-hyphen-only).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hyphenation {
    /// ISO 639-1 language code, e.g. `*b"en"`, `*b"fr"`, `*b"de"`.
    pub language: [u8; 2],
}

impl Hyphenation {
    /// Hyphenation in the given ISO 639-1 language.
    pub const fn new(language: [u8; 2]) -> Self {
        Self { language }
    }

    /// English hyphenation (`en`).
    pub const ENGLISH: Self = Self { language: *b"en" };
}

impl Default for Hyphenation {
    fn default() -> Self {
        Self::ENGLISH
    }
}

// ── Single-line API ────────────────────────────────────────────

/// Text formatting parameters for the single-line layout API.
///
/// Controls font selection, size, and text color. All fields are optional
/// and fall back to the typesetter's defaults (default font, default size,
/// default text color).
#[derive(Clone, Debug, Default)]
pub struct TextFormat {
    /// Font family name (e.g., "Noto Sans", "monospace").
    /// None means use the default font.
    pub font_family: Option<String>,
    /// Font weight (100-900). Overrides `font_bold`.
    pub font_weight: Option<u32>,
    /// Shorthand for weight 700. Ignored if `font_weight` is set.
    pub font_bold: Option<bool>,
    /// Italic style.
    pub font_italic: Option<bool>,
    /// Font size in pixels. None means use the default size.
    pub font_size: Option<f32>,
    /// Text color (RGBA, 0.0-1.0). None means use the typesetter's text color.
    pub color: Option<[f32; 4]>,
    /// Discretionary OpenType features to toggle during shaping (ligatures,
    /// small caps, tabular numerals, stylistic sets, …). Empty = font defaults.
    pub features: Vec<FontFeature>,
    /// Hyphenation (Knuth-Liang dictionary + soft-hyphen breaks) for line
    /// wrapping. `None` = disabled (default); most useful for justified
    /// prose. See [`Hyphenation`].
    pub hyphenation: Option<Hyphenation>,
}

/// Result of [`crate::DocumentFlow::layout_single_line`].
///
/// Contains the measured dimensions and GPU-ready glyph quads for a
/// single line of text. No flow layout, line breaking, or bidi analysis
/// is performed.
#[non_exhaustive]
pub struct SingleLineResult {
    /// Total advance width of the shaped text, in pixels.
    pub width: f32,
    /// Line height (ascent + descent + leading), in pixels.
    pub height: f32,
    /// Distance from the top of the line to the baseline, in pixels.
    pub baseline: f32,
    /// Distance from baseline to the top of the underline, in logical
    /// pixels. Positive = below the baseline. Sourced from the primary
    /// font's `post` table.
    pub underline_offset: f32,
    /// Underline line thickness in logical pixels. Sourced from the
    /// primary font's stroke size.
    pub underline_thickness: f32,
    /// GPU-ready glyph quads, positioned at y=0 (no scroll offset).
    pub glyphs: Vec<GlyphQuad>,
    /// Per-glyph cache keys, parallel to `glyphs`. Callers that cache
    /// glyph output externally should pass these back to
    /// [`crate::TextFontService::touch_glyphs`] each frame to prevent the
    /// atlas from evicting still-visible glyphs.
    pub glyph_keys: Vec<crate::atlas::cache::GlyphCacheKey>,
    /// Per-span bounding rectangles for markup-aware layout
    /// ([`crate::DocumentFlow::layout_single_line_markup`]). Empty for
    /// the plain-text layout path.
    pub spans: Vec<LaidOutSpan>,
}

/// A single laid-out span produced by the markup-aware layout path.
///
/// When a link wraps across two paragraph lines, it produces two
/// `LaidOutSpan` entries sharing the same URL and byte_range but with
/// distinct `line_index` / `rect`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct LaidOutSpan {
    pub kind: LaidOutSpanKind,
    /// Which wrapped line this span piece lives on (0 for single-line).
    pub line_index: usize,
    /// Local-space rect: `[x, y, width, height]`, same space as glyph quads.
    pub rect: [f32; 4],
    /// Byte range into the original markup source.
    pub byte_range: std::ops::Range<usize>,
}

/// Kind discriminator for [`LaidOutSpan`].
#[derive(Debug, Clone)]
pub enum LaidOutSpanKind {
    Text,
    Link { url: String },
}

/// Result of [`crate::DocumentFlow::layout_paragraph`].
///
/// Contains the measured dimensions and GPU-ready glyph quads for a
/// multi-line paragraph wrapped at a fixed width. Glyphs are positioned
/// in paragraph-local coordinates: `x = 0` is the left edge of the
/// paragraph, `y = 0` is the top of the first line's line box. The
/// adapter should offset all glyph quads by the paragraph's screen
/// position before drawing.
#[non_exhaustive]
pub struct ParagraphResult {
    /// Width of the widest laid-out line, in pixels. May be less than the
    /// `max_width` passed to `layout_paragraph` if the content is narrower.
    pub width: f32,
    /// Total stacked paragraph height in pixels — sum of line heights for
    /// all emitted lines.
    pub height: f32,
    /// Distance from `y = 0` to the baseline of the first line, in pixels.
    pub baseline_first: f32,
    /// Number of lines actually emitted (respects `max_lines` when set).
    pub line_count: usize,
    /// Line height (single line's ascent + descent + leading), in pixels.
    /// Useful for callers that need to reason about per-line geometry.
    pub line_height: f32,
    /// Distance from baseline to the top of the underline, in logical
    /// pixels. Positive = below the baseline. Sourced from the primary
    /// font's `post` table.
    pub underline_offset: f32,
    /// Underline line thickness in logical pixels. Sourced from the
    /// primary font's stroke size.
    pub underline_thickness: f32,
    /// GPU-ready glyph quads in paragraph-local coordinates.
    pub glyphs: Vec<GlyphQuad>,
    /// Per-glyph cache keys, parallel to `glyphs`. See
    /// [`SingleLineResult::glyph_keys`].
    pub glyph_keys: Vec<crate::atlas::cache::GlyphCacheKey>,
    /// Per-span bounding rectangles for markup-aware layout
    /// ([`crate::DocumentFlow::layout_paragraph_markup`]). Empty for
    /// the plain-text layout path.
    pub spans: Vec<LaidOutSpan>,
}

impl RenderFrame {
    pub(crate) fn new() -> Self {
        Self {
            atlas_dirty: false,
            atlas_width: 0,
            atlas_height: 0,
            atlas_pixels: Vec::new(),
            glyphs: Vec::new(),
            images: Vec::new(),
            decorations: Vec::new(),
            block_glyphs: Vec::new(),
            block_decorations: Vec::new(),
            block_images: Vec::new(),
            block_heights: std::collections::HashMap::new(),
            block_glyph_keys: Vec::new(),
            glyph_keys: Vec::new(),
            atlas_eviction_epoch: 0,
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// CharacterGeometry — accessibility per-character advance data
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Per-character advance geometry for a laid-out text run.
///
/// Consumed by accessibility layers that need to populate AccessKit's
/// `character_positions` and `character_widths` on a `Role::TextRun`
/// node so screen reader highlight cursors and screen magnifiers can
/// track the caret at character granularity.
///
/// `position` is measured in run-local coordinates: the first
/// character of the requested range sits at `position == 0.0`, and
/// subsequent characters accumulate their advance widths. `width` is
/// the advance width of each character, in the same units.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct CharacterGeometry {
    pub position: f32,
    pub width: f32,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Line geometry — per-line, per-character layout output
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Reading direction of a [`LineSegment`], as the accessibility layer
/// needs it.
///
/// Distinct from [`crate::TextDirection`], which carries an `Auto`
/// variant meaning "not yet resolved". Geometry is produced *after*
/// shaping, so every segment's direction is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeometryDirection {
    LeftToRight,
    RightToLeft,
}

/// How a laid-out line ends.
///
/// A screen reader distinguishes a paragraph break (which it announces
/// and which occupies a character in the accessible text) from a soft
/// wrap (which occupies nothing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnd {
    /// The line was broken to fit the wrap width. Nothing in the source
    /// separates it from the next line.
    SoftWrap,
    /// The line ends with an explicit break in the source: a `\n`, a `\r\n`,
    /// or any other hard line break of UAX #14 (a carriage return, next
    /// line, vertical tab, form feed, paragraph separator, or the line
    /// separator U+2028 a word processor inserts for Shift+Enter).
    ///
    /// `chars` is how many source *characters* the break occupies (1, or 2
    /// for `\r\n`); `bytes` is how many source *bytes* it occupies (1 for
    /// `\n`, 2 for `\r\n`, 3 for U+2028). AccessKit counts a `\r\n` as a
    /// single character whose `character_lengths` entry is 2 — that is
    /// `bytes`.
    HardBreak { chars: u8, bytes: u8 },
    /// The line ends because the source does.
    EndOfText,
}

impl LineEnd {
    /// The explicit break `text` ends with, if it ends with one.
    pub(crate) fn hard_break_ending(text: &str) -> Option<Self> {
        if text.ends_with("\r\n") {
            return Some(LineEnd::HardBreak { chars: 2, bytes: 2 });
        }
        let last = text.chars().next_back()?;
        is_hard_break(last).then(|| LineEnd::HardBreak {
            chars: 1,
            bytes: last.len_utf8() as u8,
        })
    }
}

/// Whether `c` is a hard line break, after which a line always ends (UAX #14
/// classes BK, CR, LF and NL): line feed, carriage return, next line,
/// vertical tab, form feed, line separator (U+2028) and paragraph separator
/// (U+2029).
///
/// It is laid out as nothing: no glyph and no advance. A font that draws one
/// (some draw U+2028 as a box) would otherwise put it at the end of the line,
/// and its advance would push a right-aligned or centred line off its place.
pub(crate) fn is_hard_break(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r' | '\u{0085}' | '\u{000B}' | '\u{000C}' | '\u{2028}' | '\u{2029}'
    )
}

/// Where a line's trailing ellipsis was drawn, for a line the layout
/// truncated.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct LineTruncation {
    /// Leading edge of the ellipsis glyph, in layout-local coordinates.
    pub ellipsis_x: f32,
    /// Advance width of the ellipsis glyph.
    pub ellipsis_width: f32,
}

/// One direction-uniform stretch of a laid-out line.
///
/// A pure LTR or pure RTL line has exactly one segment; a bidirectional
/// line has one per direction change, in *logical* order.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct LineSegment {
    /// Byte range of this segment in the source text (or, on the markup
    /// paths, in [`LayoutGeometry::rendered_text`]).
    pub byte_range: std::ops::Range<usize>,
    /// Character range of this segment, in the same text.
    pub char_range: std::ops::Range<usize>,
    /// Reading direction of every character in this segment.
    pub direction: GeometryDirection,
    /// Bounding box `[x, y, width, height]` in layout-local coordinates, with
    /// the same origin as [`LineGeometry::rect`].
    pub rect: [f32; 4],
    /// One entry per character of `char_range`, in **logical** order.
    ///
    /// `position` is measured from the segment's *leading* edge — its
    /// left edge when [`direction`](Self::direction) is
    /// [`GeometryDirection::LeftToRight`], its right edge when it is
    /// [`GeometryDirection::RightToLeft`] — so positions are
    /// non-decreasing and widths are never negative in both cases.
    ///
    /// A character with no advance of its own (a combining mark, the
    /// interior of a ligature) reports its cluster's leading position and
    /// a width of `0.0`.
    pub characters: Vec<CharacterGeometry>,
}

/// One laid-out visual line.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct LineGeometry {
    /// Zero-based index of this line within the layout.
    pub index: usize,
    /// Byte range of the line in the source text, including any trailing
    /// hard break (see [`LineEnd`]).
    pub byte_range: std::ops::Range<usize>,
    /// Character range of the line, in the same text.
    pub char_range: std::ops::Range<usize>,
    /// Bounding box `[x, y, width, height]` in layout-local coordinates.
    /// `y` is the top of the line box, `height` its full line height.
    ///
    /// For a line of a laid-out document block, `x` is measured from
    /// [`BlockVisualInfo::content_x`], the start of the block's text, and `y`
    /// from the block's top edge.
    pub rect: [f32; 4],
    /// Baseline y in layout-local coordinates.
    pub baseline: f32,
    /// Where a caret sits on a line with no glyphs of its own — the x the
    /// first typed character would take, honouring indent, alignment and
    /// base direction. Like `rect`, it is measured from
    /// [`BlockVisualInfo::content_x`] for a laid-out document block.
    pub caret_x: f32,
    /// Direction-uniform stretches in logical order.
    ///
    /// **Empty means unmeasurable, never partial**: a consumer that finds
    /// no segments must fall back to degenerate geometry for the whole
    /// line rather than for part of it.
    pub segments: Vec<LineSegment>,
    /// How the line ends.
    pub end: LineEnd,
    /// Set when the line was cut short and an ellipsis drawn.
    pub truncation: Option<LineTruncation>,
}

/// One hyperlink in a markup layout, reported against the rendered text.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct LinkGeometry {
    /// Byte range of the link's label in
    /// [`LayoutGeometry::rendered_text`] — *not* in the markup source.
    pub rendered_byte_range: std::ops::Range<usize>,
    /// The link target.
    pub url: String,
}

/// Per-line, per-character geometry for one layout call.
///
/// Produced by the `*_with_geometry` layout methods on
/// [`crate::DocumentFlow`]. Every range in it indexes
/// [`rendered_text`](Self::rendered_text) when that is `Some` (the markup
/// paths) and the caller's source text otherwise.
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct LayoutGeometry {
    /// One entry per emitted line, in visual top-to-bottom order.
    pub lines: Vec<LineGeometry>,
    /// How many further lines the layout produced but did not emit,
    /// because `max_lines` cut them.
    pub dropped_lines: usize,
    /// Byte length of the text every range indexes.
    pub source_len: usize,
    /// The text the ranges index, when it differs from the caller's input
    /// — i.e. on the markup paths, where the syntax has been stripped.
    /// `None` on the plain-text paths, where the input *is* that text.
    pub rendered_text: Option<String>,
    /// Hyperlinks found in the markup, against the rendered text.
    pub links: Vec<LinkGeometry>,
}
