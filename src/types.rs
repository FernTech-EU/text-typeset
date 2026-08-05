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
    /// Per-block glyph data for incremental updates. Keyed by block_id.
    pub(crate) block_glyphs: Vec<(usize, Vec<GlyphQuad>)>,
    /// Per-block decoration data (underlines, etc. — NOT cursor/selection).
    pub(crate) block_decorations: Vec<(usize, Vec<DecorationRect>)>,
    /// Per-block image data for incremental updates.
    pub(crate) block_images: Vec<(usize, Vec<ImageQuad>)>,
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
    pub(crate) block_glyph_keys: Vec<(usize, Vec<crate::atlas::cache::GlyphCacheKey>)>,
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

/// A positioned glyph to draw as a textured quad from the atlas.
///
/// The adapter draws the rectangle at `screen` position, sampling from
/// the `atlas` rectangle in the atlas texture, tinted with `color`.
#[derive(Clone)]
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
pub struct ImageQuad {
    /// Screen position and size: `[x, y, width, height]` in pixels.
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
}

/// A colored rectangle for decorations (underlines, selections, borders, etc.).
#[derive(Clone)]
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
pub struct HitTestResult {
    /// Absolute character position in the document.
    pub position: usize,
    /// Which side of a soft-wrap boundary the click landed on. When
    /// the matched line's Y range contained the click and `position`
    /// equals that line's `char_range.start` AND a preceding line in
    /// the same block ends at the same position, the click is on the
    /// upstream side of the boundary → `Upstream`. Otherwise
    /// `Downstream`. At non-wrap positions the value is `Downstream`
    /// (default) and does not affect anything.
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

/// Visual position and size of a laid-out block.
///
/// Returned by [`crate::DocumentFlow::block_visual_info`].
pub struct BlockVisualInfo {
    /// Block ID (matches `BlockSnapshot::block_id`).
    pub block_id: usize,
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
pub struct CharacterGeometry {
    pub position: f32,
    pub width: f32,
}
