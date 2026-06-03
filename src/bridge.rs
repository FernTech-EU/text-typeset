//! Bridge between text-document snapshot types and text-typeset layout params.
//!
//! Converts `FlowSnapshot`, `BlockSnapshot`, `TextFormat`, etc. into
//! `BlockLayoutParams`, `FragmentParams`, `TableLayoutParams`, etc.

use text_document::{
    BlockSnapshot, CellSnapshot, FlowElementSnapshot, FlowSnapshot, FragmentContent, FrameSnapshot,
    TableSnapshot,
};

use crate::layout::block::{BlockLayoutParams, FragmentParams, PaintSpan};
use crate::layout::frame::{FrameLayoutParams, FramePosition};
use crate::layout::paragraph::Alignment;
use crate::layout::table::{CellLayoutParams, TableLayoutParams};

const DEFAULT_LIST_INDENT: f32 = 24.0;
const INDENT_PER_LEVEL: f32 = 24.0;

/// Parse a 2-letter ISO 639-1 code (e.g. "en", "fr") into a lowercased
/// byte pair for `hypher::Lang::from_iso`. Returns `None` for anything
/// that isn't two ASCII letters (incl. longer tags like "en-US" — only
/// the primary subtag matters, so callers may pass that and we take the
/// first two letters).
fn iso639_1(code: &str) -> Option<[u8; 2]> {
    let b = code.trim().as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1].is_ascii_alphabetic() {
        Some([b[0].to_ascii_lowercase(), b[1].to_ascii_lowercase()])
    } else {
        None
    }
}

/// Per-call knobs threaded through the conversion functions so that a
/// host widget can override defaults driven by its active theme.
///
/// Default values reproduce the historical pre-themed behaviour: a
/// light-grey card behind fenced code blocks and no foreground
/// override for monospaced runs.
#[derive(Clone, Copy)]
pub struct BridgeOptions {
    /// Background painted behind blocks where `BlockFormat.is_code_block
    /// == Some(true)` AND the block carries no explicit
    /// `background_color`. Has no effect on prose blocks or blocks that
    /// set their own background.
    pub code_block_background: [f32; 4],
    /// Foreground used for character runs whose font family resolves
    /// to `monospace` (set by the markdown importer for inline `code`
    /// spans and by `is_code_block` blocks). `None` keeps the
    /// engine-level default text colour. Only applied when the run
    /// carries no explicit `foreground_color`.
    pub code_block_foreground: Option<[f32; 4]>,
    /// When `Some(c)`, every character of every block laid out with
    /// these options is replaced with `c` — one echo char per source
    /// `char` — before shaping. This is the password / secure-field
    /// masking path: the real text never reaches the shaper or the
    /// glyph atlas, only the echo character does. Emitting one echo per
    /// source `char` (not per grapheme) preserves char counts, so the
    /// engine's char-indexed caret / selection / hit-test stay aligned
    /// with the host document's positions. `None` (default) lays text
    /// out verbatim.
    pub echo_char: Option<char>,
    /// When true, blocks that are **justified** and don't set the
    /// `hyphenate` flag explicitly are hyphenated automatically (in the
    /// block's `language`, defaulting to English). This pairs hyphenation
    /// with justification — its primary use case — without requiring a
    /// per-block flag. An explicit `BlockFormat.hyphenate` (true or false)
    /// always wins. Hosts should enable this only for prose/rich-text
    /// surfaces, not single-line/label widgets. `false` by default.
    pub hyphenate_justified: bool,
}

impl Default for BridgeOptions {
    fn default() -> Self {
        Self {
            code_block_background: [0.95, 0.95, 0.95, 1.0],
            code_block_foreground: None,
            echo_char: None,
            hyphenate_justified: false,
        }
    }
}

/// Convert a FlowSnapshot into layout params that can be fed to a [`DocumentFlow`].
///
/// [`DocumentFlow`]: crate::DocumentFlow
pub fn convert_flow(flow: &FlowSnapshot) -> FlowElements {
    convert_flow_with(flow, &BridgeOptions::default())
}

/// Same as [`convert_flow`] but accepts host-supplied [`BridgeOptions`]
/// for theme-driven colour overrides.
pub fn convert_flow_with(flow: &FlowSnapshot, opts: &BridgeOptions) -> FlowElements {
    let mut blocks = Vec::new();
    let mut tables = Vec::new();
    let mut frames = Vec::new();

    for (i, element) in flow.elements.iter().enumerate() {
        match element {
            FlowElementSnapshot::Block(block) => {
                blocks.push((i, convert_block_with(block, opts)));
            }
            FlowElementSnapshot::Table(table) => {
                tables.push((i, convert_table_with(table, opts)));
            }
            FlowElementSnapshot::Frame(frame) => {
                frames.push((i, convert_frame_with(frame, opts)));
            }
        }
    }

    FlowElements {
        blocks,
        tables,
        frames,
    }
}

/// Converted flow elements, ordered by their position in the flow.
pub struct FlowElements {
    /// (flow_index, params)
    pub blocks: Vec<(usize, BlockLayoutParams)>,
    pub tables: Vec<(usize, TableLayoutParams)>,
    pub frames: Vec<(usize, FrameLayoutParams)>,
}

pub fn convert_block(block: &BlockSnapshot) -> BlockLayoutParams {
    convert_block_with(block, &BridgeOptions::default())
}

/// Same as [`convert_block`] but with theme-driven [`BridgeOptions`]
/// for code-block colour overrides.
pub fn convert_block_with(block: &BlockSnapshot, opts: &BridgeOptions) -> BlockLayoutParams {
    let alignment = block
        .block_format
        .alignment
        .as_ref()
        .map(convert_alignment)
        .unwrap_or_default();

    let heading_scale = match block.block_format.heading_level {
        Some(1) => 2.0,
        Some(2) => 1.5,
        Some(3) => 1.25,
        Some(4) => 1.1,
        _ => 1.0,
    };

    // text-document's `FragmentContent::{Text, Image}.offset` is the
    // **character** offset of the fragment within the block. text-typeset
    // downstream (block.rs:143 / paragraph.rs:216) treats
    // `FragmentParams.offset` as the fragment's **byte** start in
    // `block.text`, then adds it to glyph clusters (also bytes) to
    // lift them into block-text byte space. The two units must
    // agree, or any block whose first fragment carries a multi-byte
    // character causes every subsequent fragment's glyphs to land at
    // the wrong byte position — observed as hit-tests + formatting
    // landing a character or two past the user's selection around
    // em-dashes, curly quotes, accented characters, emoji, etc.
    //
    // Build a single char → byte index once over `block.text` (O(N)),
    // then look each fragment's char offset up in O(1) and pass the
    // byte offset into `convert_fragment`. The fragment stream covers
    // the whole block text in char order, so the lookup is in range
    // for every fragment we see.
    let char_to_byte: Vec<usize> = block
        .text
        .char_indices()
        .map(|(b, _)| b)
        .chain(std::iter::once(block.text.len()))
        .collect();
    let fragments: Vec<FragmentParams> = block
        .fragments
        .iter()
        .map(|f| {
            let char_offset = match f {
                FragmentContent::Text { offset, .. } => *offset,
                FragmentContent::Image { offset, .. } => *offset,
            };
            let byte_offset = char_to_byte
                .get(char_offset)
                .copied()
                .unwrap_or(block.text.len());
            convert_fragment(f, heading_scale, opts, byte_offset)
        })
        .collect();

    let indent_level = block.block_format.indent.unwrap_or(0) as f32;

    let (list_marker, list_indent) = if let Some(ref info) = block.list_info {
        let list_indent_level = info.indent as f32;
        (
            info.marker.clone(),
            DEFAULT_LIST_INDENT + list_indent_level * INDENT_PER_LEVEL,
        )
    } else {
        (String::new(), indent_level * INDENT_PER_LEVEL)
    };

    let checkbox = match block.block_format.marker {
        Some(text_document::MarkerType::Checked) => Some(true),
        Some(text_document::MarkerType::Unchecked) => Some(false),
        _ => None,
    };

    let mut params = BlockLayoutParams {
        block_id: block.block_id,
        position: block.position,
        text: block.text.clone(),
        fragments,
        alignment,
        top_margin: block.block_format.top_margin.unwrap_or(0) as f32,
        bottom_margin: block.block_format.bottom_margin.unwrap_or(0) as f32,
        left_margin: block.block_format.left_margin.unwrap_or(0) as f32,
        right_margin: block.block_format.right_margin.unwrap_or(0) as f32,
        text_indent: block.block_format.text_indent.unwrap_or(0) as f32,
        list_marker,
        list_indent,
        tab_positions: block
            .block_format
            .tab_positions
            .iter()
            .map(|&t| t as f32)
            .collect(),
        line_height_multiplier: block.block_format.line_height,
        non_breakable_lines: block.block_format.non_breakable_lines.unwrap_or(false)
            || block.block_format.is_code_block == Some(true),
        // Map the document's per-block hyphenation flag + language to the
        // engine's Hyphenation config. An explicit `hyphenate` flag always
        // wins; when it's unset, `hyphenate_justified` opts justified
        // blocks in (hyphenation's main use case). Language defaults to
        // English when unset/unparseable; unsupported languages degrade to
        // soft-hyphen-only at wrap time.
        hyphenation: {
            let enabled = match block.block_format.hyphenate {
                Some(v) => v,
                None => opts.hyphenate_justified && alignment == Alignment::Justify,
            };
            enabled.then(|| crate::types::Hyphenation {
                language: block
                    .block_format
                    .language
                    .as_deref()
                    .and_then(iso639_1)
                    .unwrap_or(*b"en"),
            })
        },
        checkbox,
        background_color: block
            .block_format
            .background_color
            .as_ref()
            .and_then(|s| parse_css_color(s))
            .or_else(|| {
                if block.block_format.is_code_block == Some(true) {
                    Some(opts.code_block_background)
                } else {
                    None
                }
            }),
    };

    if let Some(echo) = opts.echo_char {
        mask_block_params(&mut params, echo);
    }

    params
}

/// Replace every text fragment's content with `echo` repeated once per
/// source `char`, rewriting the block text and fragment byte offsets to
/// match. Image-placeholder fragments pass through unchanged (only their
/// byte offset shifts). Used for password / secure-field masking: the
/// plaintext is substituted here, before shaping, so it never reaches
/// the shaper or the glyph atlas. Char counts are preserved per
/// fragment, keeping the engine's char-indexed caret / selection /
/// hit-test aligned with the host's real document positions.
fn mask_block_params(params: &mut BlockLayoutParams, echo: char) {
    if params.fragments.is_empty() {
        params.text = echo.to_string().repeat(params.text.chars().count());
        return;
    }
    let mut masked_block = String::new();
    let mut byte_cursor = 0usize;
    for frag in params.fragments.iter_mut() {
        frag.offset = byte_cursor;
        if frag.image_name.is_some() {
            // Inline image placeholder — keep the object-replacement
            // character intact; only its byte offset shifts.
            masked_block.push_str(&frag.text);
            byte_cursor += frag.text.len();
            continue;
        }
        let masked = echo.to_string().repeat(frag.text.chars().count());
        byte_cursor += masked.len();
        masked_block.push_str(&masked);
        frag.text = masked;
    }
    params.text = masked_block;
}

fn convert_fragment(
    frag: &FragmentContent,
    heading_scale: f32,
    opts: &BridgeOptions,
    byte_offset: usize,
) -> FragmentParams {
    match frag {
        FragmentContent::Text {
            text,
            format,
            length,
            ..
        } => {
            // Monospaced runs without an explicit foreground pick up the
            // host theme's code_block_foreground so `inline code` and
            // fenced code blocks read as their own register against
            // prose. Authors that pinned a colour explicitly always win.
            let is_monospace = format
                .font_family
                .as_deref()
                .map(|f| f.eq_ignore_ascii_case("monospace"))
                .unwrap_or(false);
            let foreground_color =
                format
                    .foreground_color
                    .as_ref()
                    .map(convert_color)
                    .or(if is_monospace {
                        opts.code_block_foreground
                    } else {
                        None
                    });
            FragmentParams {
                text: text.clone(),
                offset: byte_offset,
                length: *length,
                font_family: format.font_family.clone(),
                font_weight: format.font_weight,
                font_bold: format.font_bold,
                font_italic: format.font_italic,
                font_point_size: if heading_scale != 1.0 {
                    // Apply heading scale; use 16 as default if no explicit size
                    Some((format.font_point_size.unwrap_or(16) as f32 * heading_scale) as u32)
                } else {
                    format.font_point_size
                },
                underline_style: convert_underline_style(format),
                overline: format.font_overline.unwrap_or(false),
                strikeout: format.font_strikeout.unwrap_or(false),
                is_link: format.is_anchor.unwrap_or(false),
                letter_spacing: format.letter_spacing.unwrap_or(0) as f32,
                word_spacing: format.word_spacing.unwrap_or(0) as f32,
                foreground_color,
                underline_color: format.underline_color.as_ref().map(convert_color),
                background_color: format.background_color.as_ref().map(convert_color),
                anchor_href: format.anchor_href.clone(),
                tooltip: format.tooltip.clone(),
                vertical_alignment: convert_vertical_alignment(format),
                image_name: None,
                image_width: 0.0,
                image_height: 0.0,
                features: Vec::new(),
            }
        }
        FragmentContent::Image {
            name,
            width,
            height,
            quality: _,
            format,
            ..
        } => FragmentParams {
            text: "\u{FFFC}".to_string(),
            offset: byte_offset,
            length: 1,
            font_family: None,
            font_weight: None,
            font_bold: None,
            font_italic: None,
            font_point_size: None,
            underline_style: crate::types::UnderlineStyle::None,
            overline: false,
            strikeout: false,
            is_link: format.is_anchor.unwrap_or(false),
            letter_spacing: 0.0,
            word_spacing: 0.0,
            foreground_color: None,
            underline_color: None,
            background_color: None,
            anchor_href: format.anchor_href.clone(),
            tooltip: format.tooltip.clone(),
            vertical_alignment: crate::types::VerticalAlignment::Normal,
            image_name: Some(name.clone()),
            image_width: *width as f32,
            image_height: *height as f32,
            features: Vec::new(),
        },
    }
}

fn convert_vertical_alignment(
    format: &text_document::TextFormat,
) -> crate::types::VerticalAlignment {
    use crate::types::VerticalAlignment;
    match format.vertical_alignment {
        Some(text_document::CharVerticalAlignment::SuperScript) => VerticalAlignment::SuperScript,
        Some(text_document::CharVerticalAlignment::SubScript) => VerticalAlignment::SubScript,
        _ => VerticalAlignment::Normal,
    }
}

fn convert_underline_style(format: &text_document::TextFormat) -> crate::types::UnderlineStyle {
    use crate::types::UnderlineStyle;
    match &format.underline_style {
        Some(s) => convert_underline_style_value(s),
        None => {
            if format.font_underline.unwrap_or(false) {
                UnderlineStyle::Single
            } else {
                UnderlineStyle::None
            }
        }
    }
}

/// Map a raw `text_document::UnderlineStyle` to the typesetter enum.
fn convert_underline_style_value(
    s: &text_document::UnderlineStyle,
) -> crate::types::UnderlineStyle {
    use crate::types::UnderlineStyle;
    match s {
        text_document::UnderlineStyle::SingleUnderline => UnderlineStyle::Single,
        text_document::UnderlineStyle::DashUnderline => UnderlineStyle::Dash,
        text_document::UnderlineStyle::DotLine => UnderlineStyle::Dot,
        text_document::UnderlineStyle::DashDotLine => UnderlineStyle::DashDot,
        text_document::UnderlineStyle::DashDotDotLine => UnderlineStyle::DashDotDot,
        text_document::UnderlineStyle::WaveUnderline => UnderlineStyle::Wave,
        text_document::UnderlineStyle::SpellCheckUnderline => UnderlineStyle::SpellCheck,
        text_document::UnderlineStyle::NoUnderline => UnderlineStyle::None,
    }
}

/// Convert a block snapshot's paint-only highlight overlay into the typesetter's
/// [`PaintSpan`]s. Char offsets pass through unchanged (both sides are
/// block-relative char offsets — the space post-layout glyph clusters live in).
/// Underline is expressed through `underline_style`: an explicit
/// `underline_style` wins, else `font_underline` maps to Single / None.
pub fn convert_paint_spans(block: &BlockSnapshot) -> Vec<PaintSpan> {
    block
        .paint_highlights
        .iter()
        .map(|h| {
            let underline_style = match &h.underline_style {
                Some(s) => Some(convert_underline_style_value(s)),
                None => match h.font_underline {
                    Some(true) => Some(crate::types::UnderlineStyle::Single),
                    Some(false) => Some(crate::types::UnderlineStyle::None),
                    None => None,
                },
            };
            PaintSpan {
                char_start: h.start,
                char_end: h.start + h.length,
                foreground_color: h.foreground_color.as_ref().map(convert_color),
                underline_color: h.underline_color.as_ref().map(convert_color),
                background_color: h.background_color.as_ref().map(convert_color),
                underline_style,
                overline: h.font_overline,
                strikeout: h.font_strikeout,
            }
        })
        .collect()
}

/// Walk a whole [`FlowSnapshot`] (top-level blocks, table cells, and frames
/// recursively) and collect the paint-only overlay for every block that has
/// one, keyed by block_id. Blocks without paint highlights are omitted (the
/// engine resets those to their base colors).
pub fn collect_paint_spans(
    flow: &FlowSnapshot,
) -> std::collections::HashMap<usize, Vec<PaintSpan>> {
    let mut out = std::collections::HashMap::new();
    for el in &flow.elements {
        collect_paint_spans_element(el, &mut out);
    }
    out
}

fn collect_paint_spans_element(
    el: &FlowElementSnapshot,
    out: &mut std::collections::HashMap<usize, Vec<PaintSpan>>,
) {
    match el {
        FlowElementSnapshot::Block(b) => {
            if !b.paint_highlights.is_empty() {
                out.insert(b.block_id, convert_paint_spans(b));
            }
        }
        FlowElementSnapshot::Table(t) => {
            for c in &t.cells {
                for b in &c.blocks {
                    if !b.paint_highlights.is_empty() {
                        out.insert(b.block_id, convert_paint_spans(b));
                    }
                }
            }
        }
        FlowElementSnapshot::Frame(f) => {
            for e in &f.elements {
                collect_paint_spans_element(e, out);
            }
        }
    }
}

fn convert_color(c: &text_document::Color) -> [f32; 4] {
    [
        c.red as f32 / 255.0,
        c.green as f32 / 255.0,
        c.blue as f32 / 255.0,
        c.alpha as f32 / 255.0,
    ]
}

/// Parse a CSS color string into RGBA floats (0.0-1.0).
///
/// Supports: `#RGB`, `#RRGGBB`, `#RRGGBBAA`, `rgb(r,g,b)`, `rgba(r,g,b,a)`,
/// and common named colors.
fn parse_css_color(s: &str) -> Option<[f32; 4]> {
    let s = s.trim();

    // Named colors
    match s.to_ascii_lowercase().as_str() {
        "transparent" => return Some([0.0, 0.0, 0.0, 0.0]),
        "black" => return Some([0.0, 0.0, 0.0, 1.0]),
        "white" => return Some([1.0, 1.0, 1.0, 1.0]),
        "red" => return Some([1.0, 0.0, 0.0, 1.0]),
        "green" => return Some([0.0, 128.0 / 255.0, 0.0, 1.0]),
        "blue" => return Some([0.0, 0.0, 1.0, 1.0]),
        "yellow" => return Some([1.0, 1.0, 0.0, 1.0]),
        "cyan" | "aqua" => return Some([0.0, 1.0, 1.0, 1.0]),
        "magenta" | "fuchsia" => return Some([1.0, 0.0, 1.0, 1.0]),
        "gray" | "grey" => return Some([128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, 1.0]),
        _ => {}
    }

    // Hex formats
    if let Some(hex) = s.strip_prefix('#') {
        let hex = hex.trim();
        return match hex.len() {
            3 => {
                // #RGB
                let r = u8::from_str_radix(&hex[0..1], 16).ok()?;
                let g = u8::from_str_radix(&hex[1..2], 16).ok()?;
                let b = u8::from_str_radix(&hex[2..3], 16).ok()?;
                Some([
                    (r * 17) as f32 / 255.0,
                    (g * 17) as f32 / 255.0,
                    (b * 17) as f32 / 255.0,
                    1.0,
                ])
            }
            4 => {
                // #RGBA
                let r = u8::from_str_radix(&hex[0..1], 16).ok()?;
                let g = u8::from_str_radix(&hex[1..2], 16).ok()?;
                let b = u8::from_str_radix(&hex[2..3], 16).ok()?;
                let a = u8::from_str_radix(&hex[3..4], 16).ok()?;
                Some([
                    (r * 17) as f32 / 255.0,
                    (g * 17) as f32 / 255.0,
                    (b * 17) as f32 / 255.0,
                    (a * 17) as f32 / 255.0,
                ])
            }
            6 => {
                // #RRGGBB
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                Some([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0])
            }
            8 => {
                // #RRGGBBAA
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
                Some([
                    r as f32 / 255.0,
                    g as f32 / 255.0,
                    b as f32 / 255.0,
                    a as f32 / 255.0,
                ])
            }
            _ => None,
        };
    }

    // rgb(r, g, b) and rgba(r, g, b, a)
    let inner = s
        .strip_prefix("rgba(")
        .and_then(|s| s.strip_suffix(')'))
        .or_else(|| s.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')))?;

    let parts: Vec<&str> = inner.split(',').collect();
    match parts.len() {
        3 => {
            let r: u8 = parts[0].trim().parse().ok()?;
            let g: u8 = parts[1].trim().parse().ok()?;
            let b: u8 = parts[2].trim().parse().ok()?;
            Some([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0])
        }
        4 => {
            let r: u8 = parts[0].trim().parse().ok()?;
            let g: u8 = parts[1].trim().parse().ok()?;
            let b: u8 = parts[2].trim().parse().ok()?;
            let a: f32 = parts[3].trim().parse().ok()?;
            Some([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a])
        }
        _ => None,
    }
}

fn convert_alignment(a: &text_document::Alignment) -> Alignment {
    match a {
        text_document::Alignment::Left => Alignment::Left,
        text_document::Alignment::Right => Alignment::Right,
        text_document::Alignment::Center => Alignment::Center,
        text_document::Alignment::Justify => Alignment::Justify,
    }
}

pub fn convert_table(table: &TableSnapshot) -> TableLayoutParams {
    convert_table_with(table, &BridgeOptions::default())
}

pub fn convert_table_with(table: &TableSnapshot, opts: &BridgeOptions) -> TableLayoutParams {
    let column_widths: Vec<f32> = table.column_widths.iter().map(|&w| w as f32).collect();

    let cells: Vec<CellLayoutParams> = table.cells.iter().map(|c| convert_cell(c, opts)).collect();

    TableLayoutParams {
        table_id: table.table_id,
        rows: table.rows,
        columns: table.columns,
        column_widths,
        border_width: table.format.border.unwrap_or(1) as f32,
        cell_spacing: table.format.cell_spacing.unwrap_or(0) as f32,
        cell_padding: table.format.cell_padding.unwrap_or(4) as f32,
        cells,
    }
}

fn convert_cell(cell: &CellSnapshot, opts: &BridgeOptions) -> CellLayoutParams {
    let blocks: Vec<BlockLayoutParams> = cell
        .blocks
        .iter()
        .map(|b| convert_block_with(b, opts))
        .collect();

    let background_color = cell
        .format
        .background_color
        .as_ref()
        .and_then(|s| parse_css_color(s));

    CellLayoutParams {
        row: cell.row,
        column: cell.column,
        blocks,
        background_color,
    }
}

pub fn convert_frame(frame: &FrameSnapshot) -> FrameLayoutParams {
    convert_frame_with(frame, &BridgeOptions::default())
}

pub fn convert_frame_with(frame: &FrameSnapshot, opts: &BridgeOptions) -> FrameLayoutParams {
    let mut blocks = Vec::new();
    let mut tables = Vec::new();
    let mut frames = Vec::new();

    for (i, element) in frame.elements.iter().enumerate() {
        match element {
            FlowElementSnapshot::Block(block) => {
                // Carry the flow index so `layout_frame` can interleave
                // blocks with sibling tables/frames in document order.
                // Dropping the index here is the bug that caused
                // nested-frame content (e.g. a depth-3 blockquote
                // sitting between two depth-2 blocks) to render in the
                // wrong visual order.
                blocks.push((i, convert_block_with(block, opts)));
            }
            FlowElementSnapshot::Table(table) => {
                tables.push((i, convert_table_with(table, opts)));
            }
            FlowElementSnapshot::Frame(inner_frame) => {
                frames.push((i, convert_frame_with(inner_frame, opts)));
            }
        }
    }

    let position = match &frame.format.position {
        Some(text_document::FramePosition::InFlow) | None => FramePosition::Inline,
        Some(text_document::FramePosition::FloatLeft) => FramePosition::FloatLeft,
        Some(text_document::FramePosition::FloatRight) => FramePosition::FloatRight,
    };

    let is_blockquote = frame.format.is_blockquote == Some(true);

    FrameLayoutParams {
        frame_id: frame.frame_id,
        position,
        width: frame.format.width.map(|w| w as f32),
        height: frame.format.height.map(|h| h as f32),
        margin_top: frame
            .format
            .top_margin
            .unwrap_or(if is_blockquote { 4 } else { 0 }) as f32,
        margin_bottom: frame
            .format
            .bottom_margin
            .unwrap_or(if is_blockquote { 4 } else { 0 }) as f32,
        margin_left: frame
            .format
            .left_margin
            .unwrap_or(if is_blockquote { 16 } else { 0 }) as f32,
        margin_right: frame.format.right_margin.unwrap_or(0) as f32,
        padding: frame
            .format
            .padding
            .unwrap_or(if is_blockquote { 8 } else { 0 }) as f32,
        border_width: frame
            .format
            .border
            .unwrap_or(if is_blockquote { 3 } else { 0 }) as f32,
        border_style: if is_blockquote {
            crate::layout::frame::FrameBorderStyle::LeftOnly
        } else {
            crate::layout::frame::FrameBorderStyle::Full
        },
        blocks,
        tables,
        frames,
    }
}

#[cfg(test)]
mod tests {
    use super::iso639_1;

    #[test]
    fn iso639_1_parses_two_letter_codes_case_insensitively() {
        assert_eq!(iso639_1("en"), Some(*b"en"));
        assert_eq!(iso639_1("FR"), Some(*b"fr"));
        assert_eq!(iso639_1("De"), Some(*b"de"));
        // Region subtags are ignored — only the primary subtag matters.
        assert_eq!(iso639_1("en-US"), Some(*b"en"));
        assert_eq!(iso639_1("  fr  "), Some(*b"fr"));
    }

    #[test]
    fn iso639_1_rejects_non_letter_codes() {
        assert_eq!(iso639_1(""), None);
        assert_eq!(iso639_1("x"), None);
        assert_eq!(iso639_1("12"), None);
    }
}
