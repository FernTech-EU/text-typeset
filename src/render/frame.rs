use crate::atlas::allocator::GlyphAtlas;
use crate::atlas::cache::{CachedGlyph, GlyphCache, GlyphCacheKey};
use crate::atlas::rasterizer::rasterize_glyph;
use crate::font::registry::FontRegistry;
use crate::layout::block::BlockLayout;
use crate::layout::flow::{FlowItem, FlowLayout};
use crate::layout::table::generate_table_decorations;
use crate::render::cursor::generate_cursor_decorations;
use crate::render::decoration::generate_block_decorations;
use crate::types::{CursorDisplay, GlyphQuad, ImageQuad, RenderFrame};

/// Build a RenderFrame from the current flow layout.
///
/// Iterates visible blocks (viewport culling), rasterizes uncached glyphs
/// into the atlas, and produces GlyphQuad entries for each visible glyph.
///
/// `raster_scale` densifies glyph bitmaps for content drawn under a scale
/// transform (`1.0` for unscaled UI): rasterization happens at
/// `size × scale_factor × raster_scale` physical pixels while glyph
/// `screen` rects stay in logical pixels — layout is identical at every
/// raster scale. Scaled rasters are unhinted.
#[allow(clippy::too_many_arguments)]
pub fn build_render_frame(
    flow: &FlowLayout,
    registry: &FontRegistry,
    atlas: &mut GlyphAtlas,
    cache: &mut GlyphCache,
    scale_context: &mut swash::scale::ScaleContext,
    scroll_offset: f32,
    viewport_width: f32,
    viewport_height: f32,
    cursors: &[CursorDisplay],
    cursor_color: [f32; 4],
    selection_color: [f32; 4],
    text_color: [f32; 4],
    raster_scale: f32,
    render_frame: &mut RenderFrame,
    eviction_epoch: &mut u64,
) {
    let scale_factor = flow.scale_factor;
    render_frame.glyphs.clear();
    render_frame.images.clear();
    render_frame.decorations.clear();
    render_frame.block_glyphs.clear();
    render_frame.block_decorations.clear();
    render_frame.block_images.clear();
    render_frame.block_heights.clear();
    render_frame.block_glyph_keys.clear();
    render_frame.glyph_keys.clear();

    // Advance generation and evict stale glyphs. The epoch bump is the
    // only externally visible trace of this eviction path — frameworks
    // retaining glyph quads across frames must watch
    // `TextFontService::eviction_epoch`, not just
    // `AtlasSnapshot::glyphs_evicted` (which reports only the
    // snapshot-driven evictions).
    cache.advance_generation();
    let evicted = cache.evict_unused();
    if !evicted.is_empty() {
        *eviction_epoch = eviction_epoch.wrapping_add(1);
    }
    for glyph in evicted {
        atlas.deallocate(glyph.alloc_id);
        #[cfg(debug_assertions)]
        atlas.debug_poison_rect(glyph.atlas_x, glyph.atlas_y, glyph.width, glyph.height);
    }

    let view_top = scroll_offset;
    let view_bottom = scroll_offset + viewport_height;

    for item in &flow.flow_order {
        let (block_id, item_y, item_height) = match item {
            FlowItem::Block {
                block_id,
                y,
                height,
            } => (*block_id, *y, *height),
            FlowItem::Table {
                table_id,
                y,
                height,
            } => {
                if *y + *height < view_top || *y > view_bottom {
                    continue;
                }
                if let Some(table) = flow.tables.get(table_id) {
                    // Capture table content glyphs/images so rebuild_flat_frame
                    // can reconstruct them (keyed by table_id).
                    let g_start = render_frame.glyphs.len();
                    let i_start = render_frame.images.len();
                    let mut key_buf: Vec<GlyphCacheKey> = Vec::new();
                    render_table_cells(
                        table,
                        0.0,
                        0.0,
                        registry,
                        atlas,
                        cache,
                        scale_context,
                        scroll_offset,
                        viewport_height,
                        text_color,
                        scale_factor,
                        raster_scale,
                        render_frame,
                        &mut key_buf,
                        eviction_epoch,
                    );
                    let table_g: Vec<GlyphQuad> = render_frame.glyphs[g_start..].to_vec();
                    let table_i: Vec<ImageQuad> = render_frame.images[i_start..].to_vec();
                    render_frame.glyph_keys.extend_from_slice(&key_buf);
                    render_frame.block_glyphs.push((*table_id, table_g));
                    render_frame.block_images.push((*table_id, table_i));
                    render_frame.block_glyph_keys.push((*table_id, key_buf));

                    let decos = generate_table_decorations(table, scroll_offset);
                    render_frame.decorations.extend(decos);
                }
                continue;
            }
            FlowItem::Frame {
                frame_id,
                y,
                height,
            } => {
                if *y + *height < view_top || *y > view_bottom {
                    continue;
                }
                if let Some(frame_layout) = flow.frames.get(frame_id) {
                    // Capture frame content glyphs/images so rebuild_flat_frame
                    // can reconstruct them (keyed by frame_id).
                    let g_start = render_frame.glyphs.len();
                    let i_start = render_frame.images.len();
                    let mut key_buf: Vec<GlyphCacheKey> = Vec::new();
                    render_frame_layout(
                        frame_layout,
                        registry,
                        atlas,
                        cache,
                        scale_context,
                        scroll_offset,
                        viewport_height,
                        text_color,
                        scale_factor,
                        raster_scale,
                        render_frame,
                        &mut key_buf,
                        eviction_epoch,
                    );
                    let frame_g: Vec<GlyphQuad> = render_frame.glyphs[g_start..].to_vec();
                    let frame_i: Vec<ImageQuad> = render_frame.images[i_start..].to_vec();
                    render_frame.glyph_keys.extend_from_slice(&key_buf);
                    render_frame.block_glyphs.push((*frame_id, frame_g));
                    render_frame.block_images.push((*frame_id, frame_i));
                    render_frame.block_glyph_keys.push((*frame_id, key_buf));
                }
                continue;
            }
        };

        // Viewport culling
        if item_y + item_height < view_top || item_y > view_bottom {
            continue;
        }

        if let Some(block) = flow.blocks.get(&block_id) {
            // Capture per-block glyphs and images
            let g_start = render_frame.glyphs.len();
            let i_start = render_frame.images.len();
            let mut key_buf: Vec<GlyphCacheKey> = Vec::new();
            render_block_at_offset(
                block,
                0.0,
                0.0,
                registry,
                atlas,
                cache,
                scale_context,
                scroll_offset,
                viewport_height,
                text_color,
                scale_factor,
                raster_scale,
                render_frame,
                &mut key_buf,
                eviction_epoch,
            );
            let block_g: Vec<GlyphQuad> = render_frame.glyphs[g_start..].to_vec();
            let block_i: Vec<ImageQuad> = render_frame.images[i_start..].to_vec();
            render_frame.glyph_keys.extend_from_slice(&key_buf);
            render_frame.block_glyphs.push((block_id, block_g));
            render_frame.block_images.push((block_id, block_i));
            render_frame.block_glyph_keys.push((block_id, key_buf));

            let decos = generate_block_decorations(
                block,
                registry,
                scroll_offset,
                viewport_height,
                0.0,
                0.0,
                viewport_width,
                text_color,
                scale_factor,
            );
            render_frame
                .block_decorations
                .push((block_id, decos.clone()));
            render_frame.decorations.extend(decos);

            // Snapshot block height for incremental render height-change detection
            render_frame.block_heights.insert(block_id, block.height);
        }
    }

    // Generate cursor and selection decorations
    let cursor_decos = generate_cursor_decorations(
        flow,
        cursors,
        scroll_offset,
        cursor_color,
        selection_color,
        viewport_width,
        viewport_height,
    );
    render_frame.decorations.extend(cursor_decos);

    // Update atlas metadata in the render frame
    render_frame.atlas_dirty = atlas.dirty;
    render_frame.atlas_width = atlas.width;
    render_frame.atlas_height = atlas.height;
    // Reuse existing allocation when capacity is sufficient
    if atlas.dirty {
        render_frame.atlas_pixels.clone_from(&atlas.pixels);
        atlas.dirty = false;
    }
    // Stamp the post-eviction epoch so subsequent cursor-only /
    // block-only paint paths can detect cross-frame staleness.
    render_frame.atlas_eviction_epoch = *eviction_epoch;
}

/// Render a block's glyphs at the given offset.
///
/// Handles list markers and all lines. The offset is (0, 0) for top-level
/// blocks, and non-zero for blocks inside table cells or frames.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_block_at_offset(
    block: &BlockLayout,
    offset_x: f32,
    offset_y: f32,
    registry: &FontRegistry,
    atlas: &mut GlyphAtlas,
    cache: &mut GlyphCache,
    scale_context: &mut swash::scale::ScaleContext,
    scroll_offset: f32,
    viewport_height: f32,
    default_text_color: [f32; 4],
    scale_factor: f32,
    raster_scale: f32,
    render_frame: &mut RenderFrame,
    glyph_keys: &mut Vec<GlyphCacheKey>,
    eviction_epoch: &mut u64,
) {
    // Render list marker on the first line (if present)
    if let Some(marker) = &block.list_marker
        && let Some(first_line) = block.lines.first()
    {
        let baseline_y = offset_y + block.y + first_line.y;
        let screen_top = baseline_y - first_line.ascent - scroll_offset;
        if screen_top + first_line.line_height >= 0.0 && screen_top <= viewport_height {
            render_run_glyphs(
                &marker.run,
                offset_x + marker.x,
                baseline_y,
                registry,
                atlas,
                cache,
                scale_context,
                scroll_offset,
                default_text_color,
                scale_factor,
                raster_scale,
                render_frame,
                glyph_keys,
                eviction_epoch,
            );
        }
    }

    for line in &block.lines {
        let line_y = offset_y + block.y + line.y;
        // Line-level viewport culling
        let screen_top = line_y - line.ascent - scroll_offset;
        if screen_top + line.line_height < 0.0 {
            continue; // above viewport
        }
        if screen_top > viewport_height {
            break; // below viewport, and lines are ordered top-to-bottom
        }

        for positioned_run in &line.runs {
            let pen_x = offset_x + block.left_margin + positioned_run.x;

            // Inline image: emit ImageQuad instead of rasterizing glyphs
            if let Some(ref image_name) = positioned_run.shaped_run.image_name {
                let image_w = positioned_run.shaped_run.advance_width;
                let image_h = positioned_run.shaped_run.image_height;
                let image_y = line_y - image_h - scroll_offset;
                render_frame.images.push(crate::types::ImageQuad {
                    screen: [pen_x, image_y, image_w, image_h],
                    name: image_name.clone(),
                });
                continue;
            }

            // Adjust baseline for superscript/subscript
            let baseline_offset = match positioned_run.decorations.vertical_alignment {
                crate::types::VerticalAlignment::SuperScript => {
                    -(positioned_run.shaped_run.size_px * 0.35)
                }
                crate::types::VerticalAlignment::SubScript => {
                    positioned_run.shaped_run.size_px * 0.2
                }
                crate::types::VerticalAlignment::Normal => 0.0,
            };
            render_run_glyphs(
                &positioned_run.shaped_run,
                pen_x,
                line_y + baseline_offset,
                registry,
                atlas,
                cache,
                scale_context,
                scroll_offset,
                default_text_color,
                scale_factor,
                raster_scale,
                render_frame,
                glyph_keys,
                eviction_epoch,
            );
        }
    }
}

/// Render all cells in a table at the given offset.
///
/// The offset is (0, 0) for top-level tables, and non-zero for tables
/// inside frames.
#[allow(clippy::too_many_arguments)]
fn render_table_cells(
    table: &crate::layout::table::TableLayout,
    offset_x: f32,
    offset_y: f32,
    registry: &FontRegistry,
    atlas: &mut GlyphAtlas,
    cache: &mut GlyphCache,
    scale_context: &mut swash::scale::ScaleContext,
    scroll_offset: f32,
    viewport_height: f32,
    default_text_color: [f32; 4],
    scale_factor: f32,
    raster_scale: f32,
    render_frame: &mut RenderFrame,
    glyph_keys: &mut Vec<GlyphCacheKey>,
    eviction_epoch: &mut u64,
) {
    for cell in &table.cell_layouts {
        if cell.row >= table.row_ys.len() || cell.column >= table.column_xs.len() {
            continue;
        }
        let cell_x = offset_x + table.column_xs[cell.column];
        let cell_y = offset_y + table.y + table.row_ys[cell.row];

        for block in &cell.blocks {
            render_block_at_offset(
                block,
                cell_x,
                cell_y,
                registry,
                atlas,
                cache,
                scale_context,
                scroll_offset,
                viewport_height,
                default_text_color,
                scale_factor,
                raster_scale,
                render_frame,
                glyph_keys,
                eviction_epoch,
            );
            let cell_w = table
                .column_content_widths
                .get(cell.column)
                .copied()
                .unwrap_or(200.0);
            let decos = generate_block_decorations(
                block,
                registry,
                scroll_offset,
                viewport_height,
                cell_x,
                cell_y,
                cell_w,
                default_text_color,
                scale_factor,
            );
            render_frame.decorations.extend(decos);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_frame_layout(
    frame: &crate::layout::frame::FrameLayout,
    registry: &FontRegistry,
    atlas: &mut GlyphAtlas,
    cache: &mut GlyphCache,
    scale_context: &mut swash::scale::ScaleContext,
    scroll_offset: f32,
    viewport_height: f32,
    default_text_color: [f32; 4],
    scale_factor: f32,
    raster_scale: f32,
    render_frame: &mut RenderFrame,
    glyph_keys: &mut Vec<GlyphCacheKey>,
    eviction_epoch: &mut u64,
) {
    let offset_x = frame.x + frame.content_x;
    let offset_y = frame.y + frame.content_y;

    // Render nested blocks
    for block in &frame.blocks {
        render_block_at_offset(
            block,
            offset_x,
            offset_y,
            registry,
            atlas,
            cache,
            scale_context,
            scroll_offset,
            viewport_height,
            default_text_color,
            scale_factor,
            raster_scale,
            render_frame,
            glyph_keys,
            eviction_epoch,
        );
        let decos = generate_block_decorations(
            block,
            registry,
            scroll_offset,
            viewport_height,
            offset_x,
            offset_y,
            frame.content_width,
            default_text_color,
            scale_factor,
        );
        render_frame.decorations.extend(decos);
    }

    // Render nested tables
    for table in &frame.tables {
        render_table_cells(
            table,
            offset_x,
            offset_y,
            registry,
            atlas,
            cache,
            scale_context,
            scroll_offset,
            viewport_height,
            default_text_color,
            scale_factor,
            raster_scale,
            render_frame,
            glyph_keys,
            eviction_epoch,
        );
    }

    // Render nested frames (recursive).
    // Nested frames store positions relative to the parent frame's content area,
    // so we render them with the parent's content offset already applied.
    for nested in &frame.frames {
        render_nested_frame(
            nested,
            offset_x,
            offset_y,
            registry,
            atlas,
            cache,
            scale_context,
            scroll_offset,
            viewport_height,
            default_text_color,
            scale_factor,
            raster_scale,
            render_frame,
            glyph_keys,
            eviction_epoch,
        );
    }

    // Frame border decorations
    append_frame_border_decorations(frame, scroll_offset, &mut render_frame.decorations);
}

/// Append frame border decoration rects to the given vec.
///
/// Extracted so that `rebuild_flat_frame` can regenerate frame decorations
/// without a full render pass.
pub fn append_frame_border_decorations(
    frame: &crate::layout::frame::FrameLayout,
    scroll_offset: f32,
    decorations: &mut Vec<crate::types::DecorationRect>,
) {
    if frame.border_width > 0.0
        && frame.border_style != crate::layout::frame::FrameBorderStyle::None
    {
        let bw = frame.border_width;
        let fx = frame.x;
        let fy = frame.y - scroll_offset;
        let fw = frame.total_width;
        let fh = frame.total_height;
        let color = match frame.border_style {
            crate::layout::frame::FrameBorderStyle::LeftOnly => [0.5, 0.5, 0.6, 1.0],
            _ => [0.6, 0.6, 0.6, 1.0],
        };

        match frame.border_style {
            crate::layout::frame::FrameBorderStyle::LeftOnly => {
                // Blockquote style: left border only
                decorations.push(crate::types::DecorationRect {
                    rect: [fx, fy, bw, fh],
                    color,
                    kind: crate::types::DecorationKind::Background,
                });
            }
            crate::layout::frame::FrameBorderStyle::Full => {
                // Top
                decorations.push(crate::types::DecorationRect {
                    rect: [fx, fy, fw, bw],
                    color,
                    kind: crate::types::DecorationKind::Background,
                });
                // Bottom
                decorations.push(crate::types::DecorationRect {
                    rect: [fx, fy + fh - bw, fw, bw],
                    color,
                    kind: crate::types::DecorationKind::Background,
                });
                // Left
                decorations.push(crate::types::DecorationRect {
                    rect: [fx, fy, bw, fh],
                    color,
                    kind: crate::types::DecorationKind::Background,
                });
                // Right
                decorations.push(crate::types::DecorationRect {
                    rect: [fx + fw - bw, fy, bw, fh],
                    color,
                    kind: crate::types::DecorationKind::Background,
                });
            }
            crate::layout::frame::FrameBorderStyle::None => {}
        }
    }
}

/// Render a nested frame with parent content offsets applied.
#[allow(clippy::too_many_arguments)]
fn render_nested_frame(
    nested: &crate::layout::frame::FrameLayout,
    parent_x: f32,
    parent_y: f32,
    registry: &FontRegistry,
    atlas: &mut GlyphAtlas,
    cache: &mut GlyphCache,
    scale_context: &mut swash::scale::ScaleContext,
    scroll_offset: f32,
    viewport_height: f32,
    default_text_color: [f32; 4],
    scale_factor: f32,
    raster_scale: f32,
    render_frame: &mut RenderFrame,
    glyph_keys: &mut Vec<GlyphCacheKey>,
    eviction_epoch: &mut u64,
) {
    let frame_x = parent_x + nested.x;
    let frame_y = parent_y + nested.y;
    let content_x = frame_x + nested.content_x;
    let content_y = frame_y + nested.content_y;

    // Render nested blocks
    for block in &nested.blocks {
        render_block_at_offset(
            block,
            content_x,
            content_y,
            registry,
            atlas,
            cache,
            scale_context,
            scroll_offset,
            viewport_height,
            default_text_color,
            scale_factor,
            raster_scale,
            render_frame,
            glyph_keys,
            eviction_epoch,
        );
        let decos = generate_block_decorations(
            block,
            registry,
            scroll_offset,
            viewport_height,
            content_x,
            content_y,
            nested.content_width,
            default_text_color,
            scale_factor,
        );
        render_frame.decorations.extend(decos);
    }

    // Render nested tables
    for table in &nested.tables {
        render_table_cells(
            table,
            content_x,
            content_y,
            registry,
            atlas,
            cache,
            scale_context,
            scroll_offset,
            viewport_height,
            default_text_color,
            scale_factor,
            raster_scale,
            render_frame,
            glyph_keys,
            eviction_epoch,
        );
    }

    // Recurse into further nested frames
    for inner in &nested.frames {
        render_nested_frame(
            inner,
            content_x,
            content_y,
            registry,
            atlas,
            cache,
            scale_context,
            scroll_offset,
            viewport_height,
            default_text_color,
            scale_factor,
            raster_scale,
            render_frame,
            glyph_keys,
            eviction_epoch,
        );
    }

    // Border decorations for this nested frame
    // Need to temporarily adjust x/y for the border drawing
    let border_frame = crate::layout::frame::FrameLayout {
        frame_id: nested.frame_id,
        x: frame_x,
        y: frame_y,
        total_width: nested.total_width,
        total_height: nested.total_height,
        content_x: nested.content_x,
        content_y: nested.content_y,
        content_width: nested.content_width,
        content_height: nested.content_height,
        blocks: Vec::new(),
        tables: Vec::new(),
        frames: Vec::new(),
        border_width: nested.border_width,
        border_style: nested.border_style,
    };
    append_frame_border_decorations(&border_frame, scroll_offset, &mut render_frame.decorations);
}

/// Render all glyphs in a shaped run at the given position.
///
/// Uses each glyph's `font_face_id` (which may differ from the run's
/// if glyph fallback replaced a .notdef with a glyph from another font).
#[allow(clippy::too_many_arguments)]
fn render_run_glyphs(
    run: &crate::shaping::run::ShapedRun,
    start_x: f32,
    baseline_y: f32,
    registry: &FontRegistry,
    atlas: &mut GlyphAtlas,
    cache: &mut GlyphCache,
    scale_context: &mut swash::scale::ScaleContext,
    scroll_offset: f32,
    default_text_color: [f32; 4],
    scale_factor: f32,
    raster_scale: f32,
    render_frame: &mut RenderFrame,
    glyph_keys: &mut Vec<GlyphCacheKey>,
    eviction_epoch: &mut u64,
) {
    // Rasterize at physical ppem; the cache key uses the physical size so
    // different scale_factors never collide. `raster_scale` densifies the
    // bitmap only — positions come from shaping at the logical ppem, so
    // scaled rasters are unhinted (the `hinted` key bit keeps them from
    // colliding with a hinted glyph of the same physical size).
    let raster_scale = if raster_scale > 0.0 {
        raster_scale
    } else {
        1.0
    };
    let hinted = raster_scale == 1.0;
    let sf = scale_factor.max(f32::MIN_POSITIVE);
    let inv_total = 1.0 / (sf * raster_scale);
    let physical_size_px = run.size_px * sf * raster_scale;

    let mut pen_x = start_x;
    for glyph in &run.glyphs {
        if glyph.glyph_id == 0 {
            pen_x += glyph.x_advance;
            continue;
        }

        // Use the glyph's own font_face_id (may be a fallback font)
        let entry = match registry.get(glyph.font_face_id) {
            Some(e) => e,
            None => {
                pen_x += glyph.x_advance;
                continue;
            }
        };

        let cache_key = GlyphCacheKey::with_weight(
            glyph.font_face_id,
            glyph.glyph_id,
            physical_size_px,
            run.weight as u32,
            hinted,
        );
        ensure_glyph_cached(
            &cache_key,
            cache,
            atlas,
            scale_context,
            entry.bytes(),
            entry.face_index,
            entry.swash_cache_key,
            physical_size_px,
            run.weight as u32,
            eviction_epoch,
        );
        if let Some(cached) = cache.get(&cache_key) {
            glyph_keys.push(cache_key);
            // CachedGlyph stores physical dims; convert to logical for the
            // quad's screen rect. The atlas rect stays physical (it's the
            // actual texture sub-region).
            let logical_w = cached.width as f32 * inv_total;
            let logical_h = cached.height as f32 * inv_total;
            let logical_left = cached.placement_left as f32 * inv_total;
            let logical_top = cached.placement_top as f32 * inv_total;
            let screen_x = pen_x + glyph.x_offset + logical_left;
            let screen_y = baseline_y - glyph.y_offset - logical_top - scroll_offset;
            let color = if cached.is_color {
                [1.0, 1.0, 1.0, 1.0]
            } else {
                run.foreground_color.unwrap_or(default_text_color)
            };
            render_frame.glyphs.push(GlyphQuad {
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
        }
        pen_x += glyph.x_advance;
    }
}

/// Ensure a glyph is in the cache and atlas. Rasterizes on cache miss.
///
/// The raster honours the key's `hinted` bit, and a full atlas triggers
/// emergency eviction of not-this-generation glyphs (bumping
/// `eviction_epoch`) before the glyph is given up on.
#[allow(clippy::too_many_arguments)]
fn ensure_glyph_cached(
    key: &GlyphCacheKey,
    cache: &mut GlyphCache,
    atlas: &mut GlyphAtlas,
    scale_context: &mut swash::scale::ScaleContext,
    font_data: &[u8],
    face_index: u32,
    swash_cache_key: swash::CacheKey,
    size_px: f32,
    font_weight: u32,
    eviction_epoch: &mut u64,
) {
    if cache.peek(key).is_some() {
        return;
    }

    let image = match rasterize_glyph(
        scale_context,
        font_data,
        face_index,
        swash_cache_key,
        key.glyph_id,
        size_px,
        font_weight,
        key.hinted,
    ) {
        Some(img) => img,
        None => return,
    };

    if image.width == 0 || image.height == 0 {
        return;
    }

    let (alloc, evicted) = crate::atlas::allocate_or_evict(atlas, cache, image.width, image.height);
    if evicted {
        *eviction_epoch = eviction_epoch.wrapping_add(1);
    }
    let alloc = match alloc {
        Some(a) => a,
        None => return,
    };
    let rect = alloc.rectangle;
    let atlas_x = rect.min.x as u32;
    let atlas_y = rect.min.y as u32;

    if image.is_color {
        atlas.blit_rgba(atlas_x, atlas_y, image.width, image.height, &image.data);
    } else {
        atlas.blit_mask(atlas_x, atlas_y, image.width, image.height, &image.data);
    }

    cache.insert(
        *key,
        CachedGlyph {
            alloc_id: alloc.id,
            atlas_x,
            atlas_y,
            width: image.width,
            height: image.height,
            placement_left: image.placement_left,
            placement_top: image.placement_top,
            is_color: image.is_color,
            last_used: 0, // will be set by insert()
        },
    );
}
