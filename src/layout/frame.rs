use crate::font::registry::FontRegistry;
use crate::layout::block::layout_block;
use crate::layout::block::{BlockLayout, BlockLayoutParams};
use crate::layout::table::{TableLayout, TableLayoutParams, layout_table};

/// Frame position type (from text-document's FramePosition).
///
/// **Note on floats**: `FloatLeft` and `FloatRight` currently place the frame
/// at the correct x position but do not implement content wrapping (text does
/// not flow beside the float). They advance `content_height` like `Inline`,
/// so subsequent content appears below rather than beside the float.
/// True float exclusion zones would require tracking available width per
/// y-range during paragraph layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FramePosition {
    /// Inline: rendered in normal flow order, advances content_height.
    #[default]
    Inline,
    /// Float left: placed at x=0, advances content_height.
    /// Content wrapping is not yet implemented.
    FloatLeft,
    /// Float right: placed at x=available_width - frame_width, advances content_height.
    /// Content wrapping is not yet implemented.
    FloatRight,
    /// Absolute: placed at (margin_left, margin_top) from document origin.
    /// Does not affect flow or content_height.
    Absolute,
}

/// How the frame border is drawn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FrameBorderStyle {
    /// Draw border on all four sides (default).
    #[default]
    Full,
    /// Draw only the left border (blockquote style).
    LeftOnly,
    /// No border.
    None,
}

/// Parameters for a frame, extracted from text-document's FrameSnapshot.
pub struct FrameLayoutParams {
    pub frame_id: usize,
    pub position: FramePosition,
    /// Frame width constraint (None = use available width).
    pub width: Option<f32>,
    /// Frame height constraint (None = auto from content).
    pub height: Option<f32>,
    pub margin_top: f32,
    pub margin_bottom: f32,
    pub margin_left: f32,
    pub margin_right: f32,
    pub padding: f32,
    pub border_width: f32,
    pub border_style: FrameBorderStyle,
    /// Nested flow elements: blocks, tables, and frames within the frame.
    /// Each element carries its `flow_index` (its position in the source
    /// `FrameSnapshot.elements` vec) so `layout_frame` can interleave
    /// blocks / tables / frames in document order. Without the index,
    /// nested-frame content (e.g. a deeper blockquote between two
    /// sibling blocks) would render in the wrong visual order.
    pub blocks: Vec<(usize, BlockLayoutParams)>,
    pub tables: Vec<(usize, TableLayoutParams)>,
    pub frames: Vec<(usize, FrameLayoutParams)>,
}

/// Computed layout for a frame.
pub struct FrameLayout {
    pub frame_id: usize,
    pub y: f32,
    pub x: f32,
    pub total_width: f32,
    pub total_height: f32,
    pub content_x: f32,
    pub content_y: f32,
    pub content_width: f32,
    pub content_height: f32,
    pub blocks: Vec<BlockLayout>,
    pub tables: Vec<TableLayout>,
    pub frames: Vec<FrameLayout>,
    pub border_width: f32,
    pub border_style: FrameBorderStyle,
}

/// Lay out a frame: compute dimensions, lay out nested content.
pub fn layout_frame(
    registry: &FontRegistry,
    params: &FrameLayoutParams,
    available_width: f32,
    scale_factor: f32,
) -> FrameLayout {
    let border = params.border_width;
    let pad = params.padding;
    let frame_width = params.width.unwrap_or(available_width);
    let content_width =
        (frame_width - border * 2.0 - pad * 2.0 - params.margin_left - params.margin_right)
            .max(0.0);

    // Lay out nested elements (blocks, tables, frames) in document
    // (flow) order — interleaved by `flow_index`. Y-coordinates and
    // visual placement follow flow order; the output Vecs (blocks /
    // tables / frames) are pushed in that same order, which is also
    // the natural per-kind iteration order since `flow_index` is
    // monotonic across the input streams.
    enum FlowItem<'a> {
        Block(&'a BlockLayoutParams),
        Table(&'a TableLayoutParams),
        Frame(&'a FrameLayoutParams),
    }
    let mut ordered: Vec<(usize, FlowItem)> =
        Vec::with_capacity(params.blocks.len() + params.tables.len() + params.frames.len());
    for (idx, b) in &params.blocks {
        ordered.push((*idx, FlowItem::Block(b)));
    }
    for (idx, t) in &params.tables {
        ordered.push((*idx, FlowItem::Table(t)));
    }
    for (idx, f) in &params.frames {
        ordered.push((*idx, FlowItem::Frame(f)));
    }
    ordered.sort_by_key(|(idx, _)| *idx);

    let mut blocks: Vec<BlockLayout> = Vec::with_capacity(params.blocks.len());
    let mut tables: Vec<TableLayout> = Vec::with_capacity(params.tables.len());
    let mut nested_frames: Vec<FrameLayout> = Vec::with_capacity(params.frames.len());
    let mut content_y = 0.0f32;

    for (_flow_idx, item) in &ordered {
        match item {
            FlowItem::Block(block_params) => {
                let mut block = layout_block(registry, block_params, content_width, scale_factor);
                block.y = content_y + block.top_margin;
                let block_content = block.height - block.top_margin - block.bottom_margin;
                content_y = block.y + block_content + block.bottom_margin;
                blocks.push(block);
            }
            FlowItem::Table(table_params) => {
                let mut table = layout_table(registry, table_params, content_width, scale_factor);
                table.y = content_y;
                content_y += table.total_height;
                tables.push(table);
            }
            FlowItem::Frame(frame_params) => {
                let mut nested = layout_frame(registry, frame_params, content_width, scale_factor);
                nested.y = content_y;
                nested.x = 0.0;
                content_y += nested.total_height;
                nested_frames.push(nested);
            }
        }
    }

    let auto_content_height = content_y;
    let content_height = params
        .height
        .map(|h| (h - border * 2.0 - pad * 2.0).max(0.0))
        .unwrap_or(auto_content_height);

    let total_height =
        params.margin_top + border + pad + content_height + pad + border + params.margin_bottom;
    let total_width =
        params.margin_left + border + pad + content_width + pad + border + params.margin_right;

    let content_x = params.margin_left + border + pad;
    let content_y_offset = params.margin_top + border + pad;

    FrameLayout {
        frame_id: params.frame_id,
        y: 0.0, // set by flow
        x: 0.0,
        total_width,
        total_height,
        content_x,
        content_y: content_y_offset,
        content_width,
        content_height,
        blocks,
        tables,
        frames: nested_frames,
        border_width: border,
        border_style: params.border_style,
    }
}
