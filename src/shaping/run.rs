use std::ops::Range;

use crate::shaping::shaper::TextDirection;
use crate::types::FontFaceId;

#[derive(Clone)]
pub struct ShapedGlyph {
    pub glyph_id: u16,
    pub cluster: u32,
    pub x_advance: f32,
    pub y_advance: f32,
    pub x_offset: f32,
    pub y_offset: f32,
    /// Font face for this specific glyph. May differ from the run's font_face_id
    /// when glyph fallback replaced a .notdef with a glyph from another font.
    pub font_face_id: FontFaceId,
}

#[derive(Clone)]
pub struct ShapedRun {
    pub font_face_id: FontFaceId,
    pub size_px: f32,
    /// Resolved font weight (CSS 100–900). Propagated to the glyph
    /// rasterizer so variable-font `wght` axis is set correctly and
    /// the glyph cache key distinguishes different weights.
    pub weight: u16,
    pub glyphs: Vec<ShapedGlyph>,
    pub advance_width: f32,
    pub text_range: Range<usize>,
    /// Resolved visual direction of this run. Glyphs are stored in visual
    /// (left-to-right) order, so for an RTL run their `cluster` byte
    /// offsets descend across the array. Hit-testing and caret placement
    /// need this to map between visual x and logical offset correctly.
    pub direction: TextDirection,
    /// Decoration flags from the source fragment's TextFormat.
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
    /// If Some, this run represents an inline image placeholder.
    pub image_name: Option<String>,
    /// Image height in pixels (used for line height expansion).
    pub image_height: f32,
}
