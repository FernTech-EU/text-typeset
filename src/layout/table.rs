use crate::font::registry::FontRegistry;
use crate::layout::block::{BlockLayout, BlockLayoutParams, layout_block};
use crate::types::{DecorationKind, DecorationRect};

/// Parameters for a table, extracted from text-document's TableSnapshot.
pub struct TableLayoutParams {
    pub table_id: usize,
    pub rows: usize,
    pub columns: usize,
    /// Relative column widths (0.0-1.0). If empty, columns are distributed evenly.
    pub column_widths: Vec<f32>,
    pub border_width: f32,
    pub cell_spacing: f32,
    pub cell_padding: f32,
    /// One entry per cell, in row-major order (row 0 col 0, row 0 col 1, ...).
    pub cells: Vec<CellLayoutParams>,
}

/// Parameters for a single table cell.
pub struct CellLayoutParams {
    pub row: usize,
    pub column: usize,
    pub blocks: Vec<BlockLayoutParams>,
    pub background_color: Option<[f32; 4]>,
}

/// Computed layout for an entire table.
pub struct TableLayout {
    pub table_id: usize,
    /// Y position relative to document start (set by flow).
    pub y: f32,
    /// Total height of the table including borders.
    pub total_height: f32,
    /// Total width of the table.
    pub total_width: f32,
    /// Computed column x positions (left edge of each column's content area).
    pub column_xs: Vec<f32>,
    /// Computed column widths (content area, excluding padding).
    pub column_content_widths: Vec<f32>,
    /// Computed row y positions (top edge of each row's content area, relative to table top).
    pub row_ys: Vec<f32>,
    /// Computed row heights (content area).
    pub row_heights: Vec<f32>,
    /// Laid out cells. Each cell contains its block layouts.
    pub cell_layouts: Vec<CellLayout>,
    pub border_width: f32,
    pub cell_padding: f32,
}

pub struct CellLayout {
    pub row: usize,
    pub column: usize,
    pub blocks: Vec<BlockLayout>,
    pub background_color: Option<[f32; 4]>,
}

/// Lay out a table: distribute column widths, lay out cell contents, compute row heights.
pub fn layout_table(
    registry: &FontRegistry,
    params: &TableLayoutParams,
    available_width: f32,
    scale_factor: f32,
) -> TableLayout {
    let cols = params.columns.max(1);
    let rows = params.rows.max(1);
    let border = params.border_width;
    let padding = params.cell_padding;
    let spacing = params.cell_spacing;

    // Total overhead: borders + spacing + padding for all columns
    let total_overhead =
        border * 2.0 + spacing * (cols as f32 - 1.0).max(0.0) + padding * 2.0 * cols as f32;
    let content_area = (available_width - total_overhead).max(0.0);

    // Compute column widths
    let column_content_widths =
        compute_column_widths(&params.column_widths, cols, content_area, padding);

    // Compute column x positions
    let mut column_xs = Vec::with_capacity(cols);
    let mut x = border;
    for (c, &col_w) in column_content_widths.iter().enumerate() {
        column_xs.push(x + padding);
        x += padding * 2.0 + col_w;
        if c < cols - 1 {
            x += spacing;
        }
    }
    let total_width = x + border;

    // Lay out each cell's blocks
    let mut cell_layouts = Vec::new();
    // Track row heights (max cell content height per row)
    let mut row_heights = vec![0.0f32; rows];

    for cell_params in &params.cells {
        let r = cell_params.row;
        let c = cell_params.column;
        if c >= cols || r >= rows {
            continue;
        }

        let cell_width = column_content_widths[c];
        let mut cell_blocks = Vec::new();
        let mut cell_height = 0.0f32;

        for block_params in &cell_params.blocks {
            let block = layout_block(registry, block_params, cell_width, scale_factor);
            cell_height += block.height;
            cell_blocks.push(block);
        }

        // Position blocks within the cell vertically
        let mut block_y = 0.0f32;
        for block in &mut cell_blocks {
            block.y = block_y;
            block_y += block.height;
        }

        row_heights[r] = row_heights[r].max(cell_height);

        cell_layouts.push(CellLayout {
            row: r,
            column: c,
            blocks: cell_blocks,
            background_color: cell_params.background_color,
        });
    }

    // Compute row y positions
    let mut row_ys = Vec::with_capacity(rows);
    let mut y = border;
    for (r, &row_h) in row_heights.iter().enumerate() {
        row_ys.push(y + padding);
        y += padding * 2.0 + row_h;
        if r < rows - 1 {
            y += spacing;
        }
    }
    let total_height = y + border;

    TableLayout {
        table_id: params.table_id,
        y: 0.0, // set by flow
        total_height,
        total_width,
        column_xs,
        column_content_widths,
        row_ys,
        row_heights,
        cell_layouts,
        border_width: border,
        cell_padding: padding,
    }
}

fn compute_column_widths(
    specified: &[f32],
    cols: usize,
    content_area: f32,
    _padding: f32,
) -> Vec<f32> {
    // When no explicit widths, distribute content_area evenly.
    // Clamp to a reasonable range: at least 1px (zero-width guard) and
    // at most a sensible default when the viewport is unbounded.
    let default_col_width = if content_area.is_finite() {
        (content_area / cols as f32).max(1.0)
    } else {
        200.0
    };

    if specified.is_empty() || specified.len() != cols {
        return vec![default_col_width; cols];
    }

    // Use specified proportions
    let total: f32 = specified.iter().sum();
    if total <= 0.0 {
        return vec![default_col_width; cols];
    }

    specified
        .iter()
        .map(|&s| (s / total) * content_area)
        .collect()
}

/// Generate border and background decoration rects for a top-level table.
pub fn generate_table_decorations(table: &TableLayout, scroll_offset: f32) -> Vec<DecorationRect> {
    generate_table_decorations_at(table, 0.0, 0.0, scroll_offset)
}

/// Generate border and background decoration rects for a table whose
/// origin is shifted by `(offset_x, offset_y)` — a table nested inside a
/// frame (e.g. a blockquote), where the offsets are the frame's content
/// origin.
pub fn generate_table_decorations_at(
    table: &TableLayout,
    offset_x: f32,
    offset_y: f32,
    scroll_offset: f32,
) -> Vec<DecorationRect> {
    let mut decorations = Vec::new();
    let table_y = offset_y + table.y - scroll_offset;

    // Outer table border
    if table.border_width > 0.0 {
        let bw = table.border_width;
        let color = [0.6, 0.6, 0.6, 1.0]; // gray border
        // Top
        decorations.push(DecorationRect {
            rect: [offset_x, table_y, table.total_width, bw],
            color,
            kind: DecorationKind::TableBorder,
        });
        // Bottom
        decorations.push(DecorationRect {
            rect: [
                offset_x,
                table_y + table.total_height - bw,
                table.total_width,
                bw,
            ],
            color,
            kind: DecorationKind::TableBorder,
        });
        // Left
        decorations.push(DecorationRect {
            rect: [offset_x, table_y, bw, table.total_height],
            color,
            kind: DecorationKind::TableBorder,
        });
        // Right
        decorations.push(DecorationRect {
            rect: [
                offset_x + table.total_width - bw,
                table_y,
                bw,
                table.total_height,
            ],
            color,
            kind: DecorationKind::TableBorder,
        });

        // Row borders
        for r in 1..table.row_ys.len() {
            let row_y = table.row_ys[r] - table.cell_padding;
            decorations.push(DecorationRect {
                rect: [offset_x, table_y + row_y - bw / 2.0, table.total_width, bw],
                color,
                kind: DecorationKind::TableBorder,
            });
        }

        // Column borders
        for c in 1..table.column_xs.len() {
            let col_x = table.column_xs[c] - table.cell_padding;
            decorations.push(DecorationRect {
                rect: [offset_x + col_x - bw / 2.0, table_y, bw, table.total_height],
                color,
                kind: DecorationKind::TableBorder,
            });
        }
    }

    // Cell backgrounds
    for cell in &table.cell_layouts {
        if let Some(bg_color) = cell.background_color
            && cell.row < table.row_ys.len()
            && cell.column < table.column_xs.len()
        {
            let cx = table.column_xs[cell.column] - table.cell_padding;
            let cy = table.row_ys[cell.row] - table.cell_padding;
            let cw = table.column_content_widths[cell.column] + table.cell_padding * 2.0;
            let ch = table.row_heights[cell.row] + table.cell_padding * 2.0;
            decorations.push(DecorationRect {
                rect: [offset_x + cx, table_y + cy, cw, ch],
                color: bg_color,
                kind: DecorationKind::TableCellBackground,
            });
        }
    }

    decorations
}
