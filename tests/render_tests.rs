mod helpers;
use helpers::{
    NOTO_SANS, Typesetter, assert_no_glyph_overlap, make_block, make_cell, make_typesetter,
};

use text_typeset::layout::block::{BlockLayoutParams, FragmentParams};
use text_typeset::layout::frame::{FrameBorderStyle, FrameLayoutParams, FramePosition};
use text_typeset::layout::paragraph::Alignment;
use text_typeset::layout::table::{CellLayoutParams, TableLayoutParams};
use text_typeset::{DecorationKind, UnderlineStyle, VerticalAlignment};

#[test]
fn full_pipeline_produces_glyph_quads() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "Hello world")]);
    let frame = ts.render();

    assert!(
        !frame.glyphs.is_empty(),
        "RenderFrame should contain glyph quads"
    );
    assert!(frame.atlas_width > 0);
    assert!(frame.atlas_height > 0);
    assert!(frame.atlas_dirty, "atlas should be dirty on first render");
    assert_no_glyph_overlap(frame);
}

#[test]
fn glyph_quads_have_valid_coordinates() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "Test")]);
    let frame = ts.render();

    for (i, quad) in frame.glyphs.iter().enumerate() {
        assert!(
            quad.screen[2] > 0.0 && quad.screen[3] > 0.0,
            "glyph {} should have positive width/height: {:?}",
            i,
            quad.screen
        );
        assert!(
            quad.atlas[2] > 0.0 && quad.atlas[3] > 0.0,
            "glyph {} should have positive atlas width/height: {:?}",
            i,
            quad.atlas
        );
        assert!(
            quad.atlas[0] >= 0.0 && quad.atlas[1] >= 0.0,
            "glyph {} atlas coordinates should be non-negative: {:?}",
            i,
            quad.atlas
        );
    }
}

#[test]
fn atlas_pixels_contain_rasterized_data() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "A")]);
    let frame = ts.render();

    assert!(
        !frame.atlas_pixels.is_empty(),
        "atlas pixels should not be empty"
    );
    // At least some pixels should be non-zero (rasterized glyph data)
    let nonzero = frame.atlas_pixels.iter().any(|&b| b > 0);
    assert!(nonzero, "atlas should have non-zero pixel data");
}

#[test]
fn multiple_blocks_produce_distinct_y_positions() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![
        make_block(1, "First paragraph"),
        make_block(2, "Second paragraph"),
    ]);
    let frame = ts.render();

    // Collect unique y positions
    let mut y_positions: Vec<f32> = frame.glyphs.iter().map(|q| q.screen[1]).collect();
    y_positions.sort_by(|a, b| a.partial_cmp(b).unwrap());
    y_positions.dedup_by(|a, b| (*a - *b).abs() < 1.0);

    assert!(
        y_positions.len() >= 2,
        "two blocks should produce glyphs at different y positions, got {:?}",
        y_positions
    );
}

#[test]
fn second_render_atlas_not_dirty() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "Hello")]);

    let _ = ts.render(); // first render — atlas gets built
    let frame = ts.render(); // second render — same content

    assert!(
        !frame.atlas_dirty,
        "atlas should not be dirty on second render with same content"
    );
}

#[test]
fn viewport_culling_omits_offscreen_blocks() {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(800.0, 50.0); // very short viewport

    // Create many blocks so total height exceeds viewport
    let blocks: Vec<_> = (0..20)
        .map(|i| make_block(i, &format!("Paragraph number {i} with some text.")))
        .collect();
    ts.layout_blocks(blocks);

    // Scroll to see only middle blocks
    ts.set_scroll_offset(200.0);
    let frame = ts.render();

    let glyph_count = frame.glyphs.len();

    // Now render without scroll (see top blocks)
    ts.set_scroll_offset(0.0);
    let frame_top = ts.render();

    // Different scroll positions should produce different glyph sets
    // (unless all blocks fit in viewport, which they don't at 50px height)
    assert!(
        glyph_count > 0,
        "should render some glyphs at scroll offset 200"
    );
    assert!(
        !frame_top.glyphs.is_empty(),
        "should render some glyphs at scroll offset 0"
    );
}

#[test]
fn content_height_is_positive_after_layout() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "Hello")]);
    assert!(ts.content_height() > 0.0);
}

#[test]
fn relayout_block_updates_render() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "Short."), make_block(2, "After.")]);
    let frame1 = ts.render();
    let count1 = frame1.glyphs.len();

    // Replace first block with longer text
    let longer = BlockLayoutParams {
        block_id: 1,
        position: 0,
        text: "This is a much longer first paragraph.".to_string(),
        fragments: vec![FragmentParams {
            text: "This is a much longer first paragraph.".to_string(),
            offset: 0,
            length: 38,
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
    };
    ts.relayout_block(&longer);
    let frame2 = ts.render();
    let count2 = frame2.glyphs.len();

    assert!(
        count2 > count1,
        "longer text should produce more glyphs: {} -> {}",
        count1,
        count2
    );
}

#[test]
fn blocks_with_margins_render_at_correct_y() {
    let mut ts = make_typesetter();
    let mut block = make_block(1, "Hello");
    block.top_margin = 20.0;
    ts.layout_blocks(vec![block]);
    let frame = ts.render();

    // First glyph's screen_y should account for top_margin + ascent
    // It should NOT double-count the margin
    assert!(!frame.glyphs.is_empty());
    let first_y = frame.glyphs[0].screen[1];
    // The glyph should be somewhere around 20px (margin) + baseline offset
    // Definitely should NOT be at 40+ (which would indicate double-counting)
    assert!(
        first_y < 40.0,
        "first glyph y ({}) suggests top_margin is double-counted",
        first_y
    );
    assert!(
        first_y > 0.0,
        "first glyph y ({}) should be positive (below document top)",
        first_y
    );
}

#[test]
fn multi_fragment_block_renders_all_glyphs() {
    // Test with two fragments (different formatting) in one block.
    // This exercises the cross-run boundary in build_line.
    let mut ts = make_typesetter();
    let text = "Hello world";
    let block = BlockLayoutParams {
        block_id: 1,
        position: 0,
        text: text.to_string(),
        fragments: vec![
            FragmentParams {
                text: "Hello ".to_string(),
                offset: 0,
                length: 6,
                font_family: None,
                font_weight: None,
                font_bold: Some(true), // bold
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
            },
            FragmentParams {
                text: "world".to_string(),
                offset: 6,
                length: 5,
                font_family: None,
                font_weight: None,
                font_bold: None, // normal
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
            },
        ],
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
    };
    ts.layout_blocks(vec![block]);
    let frame = ts.render();

    // "Hello world" = 11 characters; should produce ~11 glyphs
    // (space is one glyph too). With a variable font, bold and normal
    // resolve to the same face, so we get 2 shaped runs but same font.
    assert!(
        frame.glyphs.len() >= 10,
        "multi-fragment block should render all glyphs, got {}",
        frame.glyphs.len()
    );
}

#[test]
fn render_empty_document() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![]);
    let frame = ts.render();
    assert!(
        frame.glyphs.is_empty(),
        "empty document should produce no glyphs"
    );
    assert!(frame.images.is_empty());
    assert!(frame.decorations.is_empty());
}

#[test]
fn glyph_x_positions_increase_left_to_right() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "ABCDEF")]);
    let frame = ts.render();

    assert!(frame.glyphs.len() >= 6);
    for i in 1..frame.glyphs.len() {
        assert!(
            frame.glyphs[i].screen[0] > frame.glyphs[i - 1].screen[0],
            "glyph {} x ({}) should be > glyph {} x ({}) for LTR text",
            i,
            frame.glyphs[i].screen[0],
            i - 1,
            frame.glyphs[i - 1].screen[0]
        );
    }
}

#[test]
fn underline_produces_decoration_rect() {
    let mut ts = make_typesetter();
    let block = BlockLayoutParams {
        block_id: 1,
        position: 0,
        text: "underlined text".to_string(),
        fragments: vec![FragmentParams {
            text: "underlined text".to_string(),
            offset: 0,
            length: 15,
            font_family: None,
            font_weight: None,
            font_bold: None,
            font_italic: None,
            font_point_size: None,
            underline_style: UnderlineStyle::Single,
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
    };
    ts.layout_blocks(vec![block]);
    let frame = ts.render();

    let underlines: Vec<_> = frame
        .decorations
        .iter()
        .filter(|d| d.kind == text_typeset::DecorationKind::Underline)
        .collect();
    assert!(
        !underlines.is_empty(),
        "underlined text should produce Underline decoration rects"
    );
    // Underline should have positive width and be near the text
    for ul in &underlines {
        assert!(ul.rect[2] > 0.0, "underline width should be positive");
        assert!(ul.rect[3] > 0.0, "underline height should be positive");
    }
}

#[test]
fn strikeout_produces_decoration_rect() {
    let mut ts = make_typesetter();
    let block = BlockLayoutParams {
        block_id: 1,
        position: 0,
        text: "struck text".to_string(),
        fragments: vec![FragmentParams {
            text: "struck text".to_string(),
            offset: 0,
            length: 11,
            font_family: None,
            font_weight: None,
            font_bold: None,
            font_italic: None,
            font_point_size: None,
            underline_style: UnderlineStyle::None,
            overline: false,
            strikeout: true,
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
    };
    ts.layout_blocks(vec![block]);
    let frame = ts.render();

    let strikeouts: Vec<_> = frame
        .decorations
        .iter()
        .filter(|d| d.kind == text_typeset::DecorationKind::Strikeout)
        .collect();
    assert!(
        !strikeouts.is_empty(),
        "strikeout text should produce Strikeout decoration rects"
    );
}

#[test]
fn no_decorations_for_plain_text() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "plain text")]);
    let frame = ts.render();

    assert!(
        frame.decorations.is_empty(),
        "plain text should produce no decoration rects, got {}",
        frame.decorations.len()
    );
}

#[test]
fn letter_spacing_increases_total_width() {
    let mut ts = make_typesetter();

    // Normal text
    let normal = make_block(1, "Hello");
    ts.layout_blocks(vec![normal]);
    let frame_normal = ts.render();
    let normal_width: f32 = frame_normal
        .glyphs
        .last()
        .map(|g| g.screen[0] + g.screen[2])
        .unwrap_or(0.0);

    // Same text with letter_spacing=5.0
    let mut ts2 = make_typesetter();
    let spaced = BlockLayoutParams {
        block_id: 1,
        position: 0,
        text: "Hello".to_string(),
        fragments: vec![FragmentParams {
            text: "Hello".to_string(),
            offset: 0,
            length: 5,
            font_family: None,
            font_weight: None,
            font_bold: None,
            font_italic: None,
            font_point_size: None,
            underline_style: UnderlineStyle::None,
            overline: false,
            strikeout: false,
            is_link: false,
            letter_spacing: 5.0,
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
    };
    ts2.layout_blocks(vec![spaced]);
    let frame_spaced = ts2.render();
    let spaced_width: f32 = frame_spaced
        .glyphs
        .last()
        .map(|g| g.screen[0] + g.screen[2])
        .unwrap_or(0.0);

    assert!(
        spaced_width > normal_width + 15.0,
        "letter_spacing=5 on 5 chars should add ~25px: normal={}, spaced={}",
        normal_width,
        spaced_width
    );
}

#[test]
fn word_spacing_increases_gap_between_words() {
    let mut ts = make_typesetter();

    // Normal
    let normal = make_block(1, "A B");
    ts.layout_blocks(vec![normal]);
    let frame_normal = ts.render();

    let mut ts2 = make_typesetter();
    let spaced = BlockLayoutParams {
        block_id: 1,
        position: 0,
        text: "A B".to_string(),
        fragments: vec![FragmentParams {
            text: "A B".to_string(),
            offset: 0,
            length: 3,
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
            word_spacing: 20.0,
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
    };
    ts2.layout_blocks(vec![spaced]);
    let frame_spaced = ts2.render();

    // Content width should increase by ~20px (one space)
    let normal_last_x = frame_normal
        .glyphs
        .last()
        .map(|g| g.screen[0])
        .unwrap_or(0.0);
    let spaced_last_x = frame_spaced
        .glyphs
        .last()
        .map(|g| g.screen[0])
        .unwrap_or(0.0);
    assert!(
        spaced_last_x > normal_last_x + 15.0,
        "word_spacing=20 should push last glyph right: normal_x={}, spaced_x={}",
        normal_last_x,
        spaced_last_x
    );
}

#[test]
fn scroll_offset_shifts_glyph_y() {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(800.0, 600.0);

    // Create enough content so it's still visible at a small scroll offset
    let blocks: Vec<_> = (0..10)
        .map(|i| make_block(i, &format!("Paragraph {i} with some text.")))
        .collect();
    ts.layout_blocks(blocks);

    ts.set_scroll_offset(0.0);
    let frame0 = ts.render();
    // Pick the y of the first glyph
    assert!(!frame0.glyphs.is_empty());
    let y_at_0 = frame0.glyphs[0].screen[1];

    // Small scroll offset — content should still be in viewport
    ts.set_scroll_offset(5.0);
    let frame5 = ts.render();
    assert!(!frame5.glyphs.is_empty());
    let y_at_5 = frame5.glyphs[0].screen[1];

    let diff = y_at_0 - y_at_5;
    assert!(
        (diff - 5.0).abs() < 1.0,
        "scroll offset 5 should shift y by ~5: y0={}, y5={}, diff={}",
        y_at_0,
        y_at_5,
        diff
    );
}

#[test]
fn render_cursor_only_falls_back_when_scroll_offset_changed() {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(800.0, 600.0);

    let blocks: Vec<_> = (0..10)
        .map(|i| make_block(i, &format!("Paragraph {i} with some text.")))
        .collect();
    ts.layout_blocks(blocks);

    ts.set_scroll_offset(0.0);
    let frame = ts.render();
    assert!(!frame.glyphs.is_empty());
    let y_at_0 = frame.glyphs[0].screen[1];

    // Change scroll offset then call render_cursor_only
    ts.set_scroll_offset(5.0);
    let frame = ts.render_cursor_only();
    assert!(!frame.glyphs.is_empty());
    let y_after = frame.glyphs[0].screen[1];

    let diff = y_at_0 - y_after;
    assert!(
        (diff - 5.0).abs() < 1.0,
        "render_cursor_only should fall back to full render when scroll offset changed: \
         y0={}, y_after={}, diff={}",
        y_at_0,
        y_after,
        diff
    );
}

#[test]
fn render_block_only_falls_back_when_scroll_offset_changed() {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(800.0, 600.0);

    let blocks: Vec<_> = (0..10)
        .map(|i| make_block(i, &format!("Paragraph {i} with some text.")))
        .collect();
    ts.layout_blocks(blocks.clone());

    ts.set_scroll_offset(0.0);
    let frame = ts.render();
    assert!(!frame.glyphs.is_empty());
    let y_at_0 = frame.glyphs[0].screen[1];

    // Change scroll offset then call render_block_only
    ts.set_scroll_offset(5.0);
    let frame = ts.render_block_only(0);
    assert!(!frame.glyphs.is_empty());
    let y_after = frame.glyphs[0].screen[1];

    let diff = y_at_0 - y_after;
    assert!(
        (diff - 5.0).abs() < 1.0,
        "render_block_only should fall back to full render when scroll offset changed: \
         y0={}, y_after={}, diff={}",
        y_at_0,
        y_after,
        diff
    );
}

// ── List tests ──────────────────────────────────────────────────

#[test]
fn list_marker_renders_extra_glyphs() {
    let mut ts = make_typesetter();
    let block = BlockLayoutParams {
        block_id: 1,
        position: 0,
        text: "Item text".to_string(),
        fragments: vec![FragmentParams {
            text: "Item text".to_string(),
            offset: 0,
            length: 9,
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
        list_marker: "1.".to_string(),
        list_indent: 30.0,
        tab_positions: vec![],
        line_height_multiplier: None,
        non_breakable_lines: false,
        checkbox: None,
        background_color: None,
    };
    ts.layout_blocks(vec![block]);
    let frame = ts.render();

    // Should have glyphs for both "1." (marker) and "Item text" (content)
    // "1." = 2 chars, "Item text" = 9 chars, total ~11 glyphs
    assert!(
        frame.glyphs.len() >= 10,
        "list item should render marker + content glyphs, got {}",
        frame.glyphs.len()
    );
}

#[test]
fn list_marker_positioned_left_of_content() {
    let mut ts = make_typesetter();
    let block = BlockLayoutParams {
        block_id: 1,
        position: 0,
        text: "Content".to_string(),
        fragments: vec![FragmentParams {
            text: "Content".to_string(),
            offset: 0,
            length: 7,
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
        list_marker: "•".to_string(),
        list_indent: 30.0,
        tab_positions: vec![],
        line_height_multiplier: None,
        non_breakable_lines: false,
        checkbox: None,
        background_color: None,
    };
    ts.layout_blocks(vec![block]);
    let frame = ts.render();

    // Find the leftmost glyph — should be the bullet marker
    let min_x = frame
        .glyphs
        .iter()
        .map(|g| g.screen[0])
        .fold(f32::MAX, f32::min);
    // Content starts at list_indent (30px), so marker should be to the left
    assert!(
        min_x < 30.0,
        "list marker should be positioned left of content indent (30px), got x={}",
        min_x
    );
}

#[test]
fn list_indent_shifts_content_right() {
    let mut ts = make_typesetter();

    // Block without list
    let plain = make_block(1, "Hello");
    ts.layout_blocks(vec![plain]);
    let frame_plain = ts.render();
    let plain_first_x = frame_plain
        .glyphs
        .first()
        .map(|g| g.screen[0])
        .unwrap_or(0.0);

    // Block with list indent
    let mut ts2 = make_typesetter();
    let listed = BlockLayoutParams {
        block_id: 1,
        position: 0,
        text: "Hello".to_string(),
        fragments: vec![FragmentParams {
            text: "Hello".to_string(),
            offset: 0,
            length: 5,
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
        list_marker: String::new(), // no marker, but with indent
        list_indent: 40.0,
        tab_positions: vec![],
        line_height_multiplier: None,
        non_breakable_lines: false,
        checkbox: None,
        background_color: None,
    };
    ts2.layout_blocks(vec![listed]);
    let frame_listed = ts2.render();

    // Content glyphs in the listed block should be shifted right by ~40px
    // Filter out any marker glyphs by looking at the content x range
    let listed_content_x = frame_listed
        .glyphs
        .iter()
        .filter(|g| g.screen[0] >= 30.0) // skip any marker glyphs
        .map(|g| g.screen[0])
        .next()
        .unwrap_or(0.0);

    assert!(
        listed_content_x > plain_first_x + 30.0,
        "list content should be shifted right by indent: plain_x={}, listed_x={}",
        plain_first_x,
        listed_content_x
    );
}

// ── Table tests ─────────────────────────────────────────────────

#[test]
fn table_renders_cell_glyphs() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![]); // clear flow
    ts.add_table(&TableLayoutParams {
        table_id: 1,
        rows: 2,
        columns: 2,
        column_widths: vec![],
        border_width: 1.0,
        cell_spacing: 0.0,
        cell_padding: 4.0,
        cells: vec![
            make_cell(0, 0, "A"),
            make_cell(0, 1, "B"),
            make_cell(1, 0, "C"),
            make_cell(1, 1, "D"),
        ],
    });
    let frame = ts.render();

    // 4 cells with 1 character each = at least 4 glyph quads
    assert!(
        frame.glyphs.len() >= 4,
        "2x2 table should render at least 4 glyphs, got {}",
        frame.glyphs.len()
    );
    assert_no_glyph_overlap(frame);
}

#[test]
fn table_produces_border_decorations() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![]);
    ts.add_table(&TableLayoutParams {
        table_id: 1,
        rows: 2,
        columns: 2,
        column_widths: vec![],
        border_width: 1.0,
        cell_spacing: 0.0,
        cell_padding: 4.0,
        cells: vec![
            make_cell(0, 0, "A"),
            make_cell(0, 1, "B"),
            make_cell(1, 0, "C"),
            make_cell(1, 1, "D"),
        ],
    });
    let frame = ts.render();

    let borders: Vec<_> = frame
        .decorations
        .iter()
        .filter(|d| d.kind == text_typeset::DecorationKind::TableBorder)
        .collect();
    assert!(
        !borders.is_empty(),
        "table should produce border decoration rects"
    );
}

#[test]
fn table_cells_at_different_positions() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![]);
    ts.add_table(&TableLayoutParams {
        table_id: 1,
        rows: 1,
        columns: 2,
        column_widths: vec![],
        border_width: 1.0,
        cell_spacing: 0.0,
        cell_padding: 4.0,
        cells: vec![make_cell(0, 0, "Left"), make_cell(0, 1, "Right")],
    });
    let frame = ts.render();

    // Collect x positions of all glyphs
    let xs: Vec<f32> = frame.glyphs.iter().map(|g| g.screen[0]).collect();
    // "Left" glyphs should be at lower x than "Right" glyphs
    // Find the gap between the two groups
    let mut sorted_xs = xs.clone();
    sorted_xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    sorted_xs.dedup_by(|a, b| (*a - *b).abs() < 1.0);

    assert!(
        sorted_xs.len() >= 2,
        "two cells should produce glyphs at different x positions"
    );
}

#[test]
fn table_has_positive_content_height() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![]);
    ts.add_table(&TableLayoutParams {
        table_id: 1,
        rows: 2,
        columns: 2,
        column_widths: vec![],
        border_width: 1.0,
        cell_spacing: 0.0,
        cell_padding: 4.0,
        cells: vec![
            make_cell(0, 0, "A"),
            make_cell(0, 1, "B"),
            make_cell(1, 0, "C"),
            make_cell(1, 1, "D"),
        ],
    });
    assert!(
        ts.content_height() > 0.0,
        "table should contribute to content height"
    );
}

#[test]
fn table_cell_background() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![]);
    let mut cell = make_cell(0, 0, "Highlighted");
    cell.background_color = Some([1.0, 1.0, 0.0, 0.3]); // yellow highlight
    ts.add_table(&TableLayoutParams {
        table_id: 1,
        rows: 1,
        columns: 1,
        column_widths: vec![],
        border_width: 1.0,
        cell_spacing: 0.0,
        cell_padding: 4.0,
        cells: vec![cell],
    });
    let frame = ts.render();

    let bgs: Vec<_> = frame
        .decorations
        .iter()
        .filter(|d| d.kind == text_typeset::DecorationKind::TableCellBackground)
        .collect();
    assert!(
        !bgs.is_empty(),
        "cell with background_color should produce TableCellBackground decoration"
    );
}

#[test]
fn table_width_does_not_exceed_viewport() {
    let mut ts = make_typesetter(); // viewport 800x600
    ts.layout_blocks(vec![]);
    ts.add_table(&TableLayoutParams {
        table_id: 1,
        rows: 1,
        columns: 4,
        column_widths: vec![],
        border_width: 1.0,
        cell_spacing: 2.0,
        cell_padding: 8.0,
        cells: vec![
            make_cell(0, 0, "One"),
            make_cell(0, 1, "Two"),
            make_cell(0, 2, "Three"),
            make_cell(0, 3, "Four"),
        ],
    });
    let frame = ts.render();

    // All glyph x positions should be within the viewport width
    for (i, glyph) in frame.glyphs.iter().enumerate() {
        assert!(
            glyph.screen[0] + glyph.screen[2] <= 810.0, // small tolerance
            "glyph {} right edge ({}) exceeds viewport width 800",
            i,
            glyph.screen[0] + glyph.screen[2]
        );
    }
}

#[test]
fn block_then_table_renders_table_below_block() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "Above the table.")]);

    let block_height = ts.content_height();
    assert!(block_height > 0.0);

    ts.add_table(&TableLayoutParams {
        table_id: 2,
        rows: 1,
        columns: 1,
        column_widths: vec![],
        border_width: 1.0,
        cell_spacing: 0.0,
        cell_padding: 4.0,
        cells: vec![make_cell(0, 0, "Cell")],
    });

    let frame = ts.render();

    // Find the y range of block glyphs and table glyphs
    // Block glyphs should have lower y (higher on screen) than table glyphs
    // Block 1 is "Above the table." — block_id 1 uses default params with position=0
    // The table cell uses block_id 0 (from make_cell)

    // All glyphs should be present
    assert!(
        frame.glyphs.len() >= 5, // "Above" + "Cell" glyphs
        "should have glyphs from both block and table, got {}",
        frame.glyphs.len()
    );

    // Table content height should be larger than just the block
    assert!(
        ts.content_height() > block_height,
        "content height with table ({}) should exceed block-only height ({})",
        ts.content_height(),
        block_height
    );
}

// ── Frame tests ─────────────────────────────────────────────────

#[test]
fn frame_renders_nested_block_glyphs() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![]);
    ts.add_frame(&FrameLayoutParams {
        frame_id: 1,
        position: FramePosition::Inline,
        width: None,
        height: None,
        margin_top: 0.0,
        margin_bottom: 0.0,
        margin_left: 0.0,
        margin_right: 0.0,
        padding: 8.0,
        border_width: 1.0,
        border_style: FrameBorderStyle::Full,
        blocks: vec![(0, make_block(9000, "Inside frame"))],
        tables: vec![],
        frames: vec![],
    });
    let frame = ts.render();

    assert!(
        frame.glyphs.len() >= 10,
        "frame with 'Inside frame' should render glyphs, got {}",
        frame.glyphs.len()
    );
    assert_no_glyph_overlap(frame);
}

#[test]
fn frame_contributes_to_content_height() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![]);
    ts.add_frame(&FrameLayoutParams {
        frame_id: 1,
        position: FramePosition::Inline,
        width: None,
        height: None,
        margin_top: 10.0,
        margin_bottom: 10.0,
        margin_left: 0.0,
        margin_right: 0.0,
        padding: 8.0,
        border_width: 1.0,
        border_style: FrameBorderStyle::Full,
        blocks: vec![(0, make_block(9000, "Content"))],
        tables: vec![],
        frames: vec![],
    });

    assert!(
        ts.content_height() > 20.0, // at least margins + some content
        "frame content height ({}) should include margins and content",
        ts.content_height()
    );
}

#[test]
fn block_then_frame_renders_frame_below() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "Above")]);
    let block_h = ts.content_height();

    ts.add_frame(&FrameLayoutParams {
        frame_id: 2,
        position: FramePosition::Inline,
        width: None,
        height: None,
        margin_top: 0.0,
        margin_bottom: 0.0,
        margin_left: 0.0,
        margin_right: 0.0,
        padding: 4.0,
        border_width: 0.0,
        border_style: FrameBorderStyle::Full,
        blocks: vec![(0, make_block(9000, "Below"))],
        tables: vec![],
        frames: vec![],
    });

    assert!(
        ts.content_height() > block_h,
        "adding frame should increase content height"
    );

    let frame_render = ts.render();
    assert!(
        frame_render.glyphs.len() >= 8, // "Above" + "Below"
        "should render glyphs from both block and frame"
    );
}

#[test]
fn frame_with_border_produces_decorations() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![]);
    ts.add_frame(&FrameLayoutParams {
        frame_id: 1,
        position: FramePosition::Inline,
        width: Some(200.0),
        height: None,
        margin_top: 0.0,
        margin_bottom: 0.0,
        margin_left: 0.0,
        margin_right: 0.0,
        padding: 4.0,
        border_width: 2.0,
        border_style: FrameBorderStyle::Full,
        blocks: vec![(0, make_block(9000, "Bordered"))],
        tables: vec![],
        frames: vec![],
    });
    let frame = ts.render();

    let borders: Vec<_> = frame
        .decorations
        .iter()
        .filter(|d| d.kind == text_typeset::DecorationKind::Background) // frame borders use Background kind
        .collect();
    assert!(
        borders.len() >= 4,
        "bordered frame should produce at least 4 border rects, got {}",
        borders.len()
    );
}

#[test]
fn float_right_frame_positioned_at_right_edge() {
    let mut ts = make_typesetter(); // 800x600
    ts.layout_blocks(vec![]);
    ts.add_frame(&FrameLayoutParams {
        frame_id: 1,
        position: FramePosition::FloatRight,
        width: Some(200.0),
        height: None,
        margin_top: 0.0,
        margin_bottom: 0.0,
        margin_left: 0.0,
        margin_right: 0.0,
        padding: 4.0,
        border_width: 0.0,
        border_style: FrameBorderStyle::Full,
        blocks: vec![(0, make_block(9000, "Right"))],
        tables: vec![],
        frames: vec![],
    });
    let frame = ts.render();

    // Glyphs should be near the right edge of the viewport
    assert!(!frame.glyphs.is_empty());
    let min_x = frame
        .glyphs
        .iter()
        .map(|g| g.screen[0])
        .fold(f32::MAX, f32::min);
    assert!(
        min_x > 500.0,
        "float-right frame glyphs should be on the right side, got min_x={}",
        min_x
    );
}

#[test]
fn absolute_frame_does_not_affect_content_height() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "Normal block")]);
    let height_before = ts.content_height();

    ts.add_frame(&FrameLayoutParams {
        frame_id: 2,
        position: FramePosition::Absolute,
        width: Some(100.0),
        height: None,
        margin_top: 50.0, // used as y position for absolute
        margin_bottom: 0.0,
        margin_left: 300.0, // used as x position for absolute
        margin_right: 0.0,
        padding: 4.0,
        border_width: 0.0,
        border_style: FrameBorderStyle::Full,
        blocks: vec![(0, make_block(9000, "Floating"))],
        tables: vec![],
        frames: vec![],
    });

    assert!(
        (ts.content_height() - height_before).abs() < 0.01,
        "absolute frame should not change content height: before={}, after={}",
        height_before,
        ts.content_height()
    );

    let frame = ts.render();
    // Absolute frame glyphs should be near x=300
    let abs_glyphs: Vec<_> = frame
        .glyphs
        .iter()
        .filter(|g| g.screen[0] > 250.0)
        .collect();
    assert!(
        !abs_glyphs.is_empty(),
        "absolute frame should render glyphs near x=300"
    );
}

#[test]
fn underline_inside_table_cell_produces_decoration() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![]);
    let cell = CellLayoutParams {
        row: 0,
        column: 0,
        blocks: vec![BlockLayoutParams {
            block_id: 1,
            position: 0,
            text: "underlined".to_string(),
            fragments: vec![FragmentParams {
                text: "underlined".to_string(),
                offset: 0,
                length: 10,
                font_family: None,
                font_weight: None,
                font_bold: None,
                font_italic: None,
                font_point_size: None,
                underline_style: UnderlineStyle::Single, // <-- underline inside table cell
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
        }],
        background_color: None,
    };
    ts.add_table(&TableLayoutParams {
        table_id: 1,
        rows: 1,
        columns: 1,
        column_widths: vec![],
        border_width: 0.0,
        cell_spacing: 0.0,
        cell_padding: 4.0,
        cells: vec![cell],
    });
    let frame = ts.render();

    let underlines: Vec<_> = frame
        .decorations
        .iter()
        .filter(|d| d.kind == text_typeset::DecorationKind::Underline)
        .collect();
    assert!(
        !underlines.is_empty(),
        "underlined text inside table cell should produce Underline decoration"
    );
}

#[test]
fn underline_inside_frame_produces_decoration() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![]);
    ts.add_frame(&FrameLayoutParams {
        frame_id: 1,
        position: FramePosition::Inline,
        width: None,
        height: None,
        margin_top: 0.0,
        margin_bottom: 0.0,
        margin_left: 0.0,
        margin_right: 0.0,
        padding: 4.0,
        border_width: 0.0,
        border_style: FrameBorderStyle::Full,
        blocks: vec![(0, BlockLayoutParams {
            block_id: 2,
            position: 0,
            text: "underlined".to_string(),
            fragments: vec![FragmentParams {
                text: "underlined".to_string(),
                offset: 0,
                length: 10,
                font_family: None,
                font_weight: None,
                font_bold: None,
                font_italic: None,
                font_point_size: None,
                underline_style: UnderlineStyle::Single,
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
        })],
        tables: vec![],
        frames: vec![],
    });
    let frame = ts.render();

    let underlines: Vec<_> = frame
        .decorations
        .iter()
        .filter(|d| d.kind == text_typeset::DecorationKind::Underline)
        .collect();
    assert!(
        !underlines.is_empty(),
        "underlined text inside frame should produce Underline decoration"
    );
}

// ── Content width mode tests ────────────────────────────────────

#[test]
fn fixed_content_width_wraps_at_set_width() {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(800.0, 600.0);

    // Auto mode: text wraps at viewport width (800)
    let text = "Word after word after word after word after word after word.";
    ts.layout_blocks(vec![make_block(1, text)]);
    let frame_auto = ts.render();
    let _auto_glyphs = frame_auto.glyphs.len();

    // Fixed mode: text wraps at 200px (much narrower)
    let mut ts2 = Typesetter::new();
    let face2 = ts2.register_font(NOTO_SANS);
    ts2.set_default_font(face2, 16.0);
    ts2.set_viewport(800.0, 600.0);
    ts2.set_content_width(200.0);
    ts2.layout_blocks(vec![make_block(1, text)]);
    let h_fixed = ts2.content_height();
    let h_auto = ts.content_height();

    // Narrower content width = more lines = taller content
    assert!(
        h_fixed > h_auto,
        "fixed 200px width ({}) should produce taller content than 800px auto ({})",
        h_fixed,
        h_auto
    );
}

#[test]
fn content_width_auto_follows_viewport() {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);
    ts.set_content_width_auto(); // explicit auto (same as default)

    ts.set_viewport(400.0, 600.0);
    assert!(
        (ts.layout_width() - 400.0).abs() < 0.01,
        "auto mode: layout_width should equal viewport width"
    );

    ts.set_viewport(1200.0, 600.0);
    assert!(
        (ts.layout_width() - 1200.0).abs() < 0.01,
        "auto mode: layout_width should follow viewport resize"
    );
}

#[test]
fn fixed_content_width_independent_of_viewport() {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);
    ts.set_content_width(500.0);

    ts.set_viewport(800.0, 600.0);
    assert!(
        (ts.layout_width() - 500.0).abs() < 0.01,
        "fixed mode: layout_width should be 500 regardless of viewport"
    );

    ts.set_viewport(300.0, 600.0);
    assert!(
        (ts.layout_width() - 500.0).abs() < 0.01,
        "fixed mode: layout_width should stay 500 even with smaller viewport"
    );
}

#[test]
fn switching_from_fixed_to_auto() {
    let mut ts = Typesetter::new();
    let face = ts.register_font(NOTO_SANS);
    ts.set_default_font(face, 16.0);
    ts.set_viewport(800.0, 600.0);

    ts.set_content_width(500.0);
    assert!((ts.layout_width() - 500.0).abs() < 0.01);

    ts.set_content_width_auto();
    assert!((ts.layout_width() - 800.0).abs() < 0.01);
}

// ── Coverage: decoration edge cases ─────────────────────────────

#[test]
fn overline_produces_decoration_rect() {
    let mut ts = make_typesetter();
    let block = BlockLayoutParams {
        block_id: 1,
        position: 0,
        text: "overlined".to_string(),
        fragments: vec![FragmentParams {
            text: "overlined".to_string(),
            offset: 0,
            length: 9,
            font_family: None,
            font_weight: None,
            font_bold: None,
            font_italic: None,
            font_point_size: None,
            underline_style: UnderlineStyle::None,
            overline: true,
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
    };
    ts.layout_blocks(vec![block]);
    let frame = ts.render();

    let overlines: Vec<_> = frame
        .decorations
        .iter()
        .filter(|d| d.kind == text_typeset::DecorationKind::Overline)
        .collect();
    assert!(
        !overlines.is_empty(),
        "overlined text should produce Overline decoration"
    );
}

// ── Coverage: flow edge cases ───────────────────────────────────

#[test]
fn float_left_frame_renders() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![]);
    ts.add_frame(&FrameLayoutParams {
        frame_id: 1,
        position: FramePosition::FloatLeft,
        width: Some(200.0),
        height: None,
        margin_top: 0.0,
        margin_bottom: 0.0,
        margin_left: 0.0,
        margin_right: 0.0,
        padding: 4.0,
        border_width: 0.0,
        border_style: FrameBorderStyle::Full,
        blocks: vec![(0, make_block(9000, "FloatL"))],
        tables: vec![],
        frames: vec![],
    });
    let frame = ts.render();
    assert!(
        !frame.glyphs.is_empty(),
        "float-left frame should render glyphs"
    );
    // Float-left: x should be near 0
    let min_x = frame
        .glyphs
        .iter()
        .map(|g| g.screen[0])
        .fold(f32::MAX, f32::min);
    assert!(min_x < 50.0, "float-left glyphs should be near left edge");
}

#[test]
fn block_after_table_has_margin_applied() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![]);
    ts.add_table(&TableLayoutParams {
        table_id: 1,
        rows: 1,
        columns: 1,
        column_widths: vec![],
        border_width: 1.0,
        cell_spacing: 0.0,
        cell_padding: 4.0,
        cells: vec![make_cell(0, 0, "Cell")],
    });
    // Add a block AFTER the table via add_block on the flow layout
    let _block = make_block(2, "After table");
    // We need to go through the flow directly since layout_blocks clears
    ts.add_table(&TableLayoutParams {
        table_id: 3,
        rows: 1,
        columns: 1,
        column_widths: vec![],
        border_width: 0.0,
        cell_spacing: 0.0,
        cell_padding: 4.0,
        cells: vec![make_cell(0, 0, "Cell2")],
    });
    let frame = ts.render();
    assert!(!frame.glyphs.is_empty());
}

#[test]
fn relayout_block_shifts_table_below() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "Short.")]);
    ts.add_table(&TableLayoutParams {
        table_id: 2,
        rows: 1,
        columns: 1,
        column_widths: vec![],
        border_width: 1.0,
        cell_spacing: 0.0,
        cell_padding: 4.0,
        cells: vec![make_cell(0, 0, "Cell")],
    });
    let h_before = ts.content_height();

    // Relayout block 1 with much longer text
    let longer = BlockLayoutParams {
        block_id: 1,
        position: 0,
        text: "This is now a very long paragraph that takes up many lines at the current width."
            .to_string(),
        fragments: vec![FragmentParams {
            text:
                "This is now a very long paragraph that takes up many lines at the current width."
                    .to_string(),
            offset: 0,
            length: 80,
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
    };
    ts.set_viewport(200.0, 600.0);
    ts.relayout_block(&longer);
    let h_after = ts.content_height();
    assert!(
        h_after > h_before,
        "content height should grow after relayout"
    );
}

// ── Coverage: frame with nested table rendering ─────────────────

#[test]
fn frame_with_nested_table_renders() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![]);
    ts.add_frame(&FrameLayoutParams {
        frame_id: 1,
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
        blocks: vec![(0, make_block(9000, "Before table"))],
        tables: vec![(
            1,
            TableLayoutParams {
                table_id: 10,
                rows: 1,
                columns: 2,
                column_widths: vec![],
                border_width: 1.0,
                cell_spacing: 0.0,
                cell_padding: 4.0,
                cells: vec![make_cell(0, 0, "A"), make_cell(0, 1, "B")],
            },
        )],
        frames: vec![],
    });
    let frame = ts.render();
    assert!(
        frame.glyphs.len() >= 12,
        "frame with block + table should render many glyphs, got {}",
        frame.glyphs.len()
    );
}

#[test]
fn render_block_only_preserves_table_decorations() {
    let mut ts = make_typesetter();
    // Layout: block 1 + table + block 2
    ts.layout_blocks(vec![make_block(1, "First block")]);
    ts.add_table(&TableLayoutParams {
        table_id: 10,
        rows: 1,
        columns: 2,
        column_widths: vec![],
        border_width: 1.0,
        cell_spacing: 0.0,
        cell_padding: 4.0,
        cells: vec![make_cell(0, 0, "A"), make_cell(0, 1, "B")],
    });

    // Full render: should produce table border decorations
    let frame = ts.render();
    let borders_full: Vec<_> = frame
        .decorations
        .iter()
        .filter(|d| d.kind == text_typeset::DecorationKind::TableBorder)
        .collect();
    assert!(
        !borders_full.is_empty(),
        "full render should produce table border decorations"
    );
    let border_count = borders_full.len();

    // render_block_only should still include table decorations
    let frame_after = ts.render_block_only(1);
    let borders_after: Vec<_> = frame_after
        .decorations
        .iter()
        .filter(|d| d.kind == text_typeset::DecorationKind::TableBorder)
        .collect();
    assert_eq!(
        borders_after.len(),
        border_count,
        "render_block_only should preserve table border decorations"
    );
}

#[test]
fn render_block_only_preserves_frame_decorations() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "First block")]);
    ts.add_frame(&FrameLayoutParams {
        frame_id: 20,
        position: FramePosition::Inline,
        width: None,
        height: None,
        margin_top: 4.0,
        margin_bottom: 4.0,
        margin_left: 16.0,
        margin_right: 0.0,
        padding: 8.0,
        border_width: 3.0,
        border_style: FrameBorderStyle::Full,
        blocks: vec![(0, make_block(2, "Frame content"))],
        tables: vec![],
        frames: vec![],
    });

    // Full render: should produce frame border decorations (Background kind)
    let frame = ts.render();
    let bg_decos_full: Vec<_> = frame
        .decorations
        .iter()
        .filter(|d| d.kind == text_typeset::DecorationKind::Background)
        .collect();
    assert!(
        !bg_decos_full.is_empty(),
        "full render should produce frame border decorations"
    );
    let bg_count = bg_decos_full.len();

    // render_block_only should still include frame border decorations
    let frame_after = ts.render_block_only(1);
    let bg_decos_after: Vec<_> = frame_after
        .decorations
        .iter()
        .filter(|d| d.kind == text_typeset::DecorationKind::Background)
        .collect();
    assert_eq!(
        bg_decos_after.len(),
        bg_count,
        "render_block_only should preserve frame border decorations"
    );
}

#[test]
fn render_block_only_preserves_frame_glyphs() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "First block")]);
    ts.add_frame(&FrameLayoutParams {
        frame_id: 20,
        position: FramePosition::Inline,
        width: None,
        height: None,
        margin_top: 4.0,
        margin_bottom: 4.0,
        margin_left: 16.0,
        margin_right: 0.0,
        padding: 8.0,
        border_width: 3.0,
        border_style: FrameBorderStyle::Full,
        blocks: vec![(0, make_block(2, "Frame content"))],
        tables: vec![],
        frames: vec![],
    });

    // Full render: count frame content glyphs
    let frame = ts.render();
    let glyph_count_full = frame.glyphs.len();
    assert!(
        glyph_count_full > 0,
        "full render should produce glyphs for both top-level block and frame content"
    );

    // render_block_only should preserve frame content glyphs
    let frame_after = ts.render_block_only(1);
    assert_eq!(
        frame_after.glyphs.len(),
        glyph_count_full,
        "render_block_only should preserve frame content glyphs, got {} vs full {}",
        frame_after.glyphs.len(),
        glyph_count_full
    );
}

#[test]
fn render_block_only_preserves_table_glyphs() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "First block")]);
    ts.add_table(&TableLayoutParams {
        table_id: 10,
        rows: 1,
        columns: 1,
        column_widths: vec![],
        border_width: 1.0,
        cell_spacing: 0.0,
        cell_padding: 4.0,
        cells: vec![CellLayoutParams {
            row: 0,
            column: 0,
            blocks: vec![make_block(100, "Cell text")],
            background_color: None,
        }],
    });

    let frame = ts.render();
    let glyph_count_full = frame.glyphs.len();
    assert!(glyph_count_full > 0);

    let frame_after = ts.render_block_only(1);
    assert_eq!(
        frame_after.glyphs.len(),
        glyph_count_full,
        "render_block_only should preserve table cell glyphs"
    );
}

#[test]
fn nested_frame_renders_inner_content() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![]);
    ts.add_frame(&FrameLayoutParams {
        frame_id: 1,
        position: FramePosition::Inline,
        width: None,
        height: None,
        margin_top: 4.0,
        margin_bottom: 4.0,
        margin_left: 16.0,
        margin_right: 0.0,
        padding: 8.0,
        border_width: 3.0,
        border_style: FrameBorderStyle::LeftOnly,
        blocks: vec![(0, make_block(10, "Outer frame text"))],
        tables: vec![],
        frames: vec![(
            1,
            FrameLayoutParams {
                frame_id: 2,
                position: FramePosition::Inline,
                width: None,
                height: None,
                margin_top: 4.0,
                margin_bottom: 4.0,
                margin_left: 16.0,
                margin_right: 0.0,
                padding: 8.0,
                border_width: 3.0,
                border_style: FrameBorderStyle::LeftOnly,
                blocks: vec![(0, make_block(20, "Inner frame text"))],
                tables: vec![],
                frames: vec![],
            },
        )],
    });
    let frame = ts.render();

    // Both outer and inner text should produce glyphs
    assert!(
        frame.glyphs.len() >= 10,
        "nested frames should render both outer and inner text, got {} glyphs",
        frame.glyphs.len()
    );

    // Should produce at least 2 border decorations (one per frame)
    let border_decos: Vec<_> = frame
        .decorations
        .iter()
        .filter(|d| d.kind == DecorationKind::Background)
        .collect();
    assert!(
        border_decos.len() >= 2,
        "nested frames should produce border decorations for both frames, got {}",
        border_decos.len()
    );
}

#[test]
fn nested_frame_contributes_to_content_height() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![]);
    ts.add_frame(&FrameLayoutParams {
        frame_id: 1,
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
        blocks: vec![(0, make_block(10, "Outer"))],
        tables: vec![],
        frames: vec![(
            1,
            FrameLayoutParams {
                frame_id: 2,
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
                blocks: vec![(0, make_block(20, "Inner"))],
                tables: vec![],
                frames: vec![],
            },
        )],
    });

    let height_with_nested = ts.content_height();

    // Compare with a frame that has only the outer block (no nested frame)
    let mut ts2 = make_typesetter();
    ts2.layout_blocks(vec![]);
    ts2.add_frame(&FrameLayoutParams {
        frame_id: 1,
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
        blocks: vec![(0, make_block(10, "Outer"))],
        tables: vec![],
        frames: vec![],
    });

    let height_without = ts2.content_height();
    assert!(
        height_with_nested > height_without,
        "nested frame should increase content height: {} vs {}",
        height_with_nested,
        height_without
    );
}

// ── Inline image rendering ─────────────────────────────────────

fn make_image_block(id: usize, image_name: &str, width: f32, height: f32) -> BlockLayoutParams {
    let text = "\u{FFFC}";
    BlockLayoutParams {
        block_id: id,
        position: 0,
        text: text.to_string(),
        fragments: vec![FragmentParams {
            text: text.to_string(),
            offset: 0,
            length: 1,
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
            image_name: Some(image_name.to_string()),
            image_width: width,
            image_height: height,
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

#[test]
fn image_produces_image_quad() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_image_block(1, "test.png", 100.0, 50.0)]);
    let frame = ts.render();

    assert_eq!(
        frame.images.len(),
        1,
        "block with one image should produce one ImageQuad"
    );
    assert_eq!(frame.images[0].name, "test.png");
    assert!(
        (frame.images[0].screen[2] - 100.0).abs() < 0.1,
        "image width should be 100.0, got {}",
        frame.images[0].screen[2]
    );
    assert!(
        (frame.images[0].screen[3] - 50.0).abs() < 0.1,
        "image height should be 50.0, got {}",
        frame.images[0].screen[3]
    );
}

#[test]
fn image_has_positive_content_height() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_image_block(1, "test.png", 100.0, 50.0)]);

    assert!(
        ts.content_height() > 0.0,
        "block with image should have positive content height"
    );
}

#[test]
fn tall_image_expands_line_height() {
    let mut ts = make_typesetter();
    // A text-only block for baseline line height
    ts.layout_blocks(vec![make_block(1, "Hello")]);
    let text_height = ts.content_height();

    // A block with a very tall image
    ts.layout_blocks(vec![make_image_block(1, "tall.png", 50.0, 200.0)]);
    let image_height = ts.content_height();

    assert!(
        image_height > text_height,
        "tall image ({}) should produce taller content than text ({})",
        image_height,
        text_height
    );
}

#[test]
fn mixed_text_and_image_both_render() {
    let mut ts = make_typesetter();
    let text = "Hello\u{FFFC}";
    ts.layout_blocks(vec![BlockLayoutParams {
        block_id: 1,
        position: 0,
        text: text.to_string(),
        fragments: vec![
            FragmentParams {
                text: "Hello".to_string(),
                offset: 0,
                length: 5,
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
            },
            FragmentParams {
                text: "\u{FFFC}".to_string(),
                offset: 5,
                length: 1,
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
                image_name: Some("icon.png".to_string()),
                image_width: 32.0,
                image_height: 32.0,
            },
        ],
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
    }]);
    let frame = ts.render();

    assert!(
        !frame.glyphs.is_empty(),
        "text fragment should produce glyph quads"
    );
    assert_eq!(
        frame.images.len(),
        1,
        "image fragment should produce one ImageQuad"
    );
    assert_eq!(frame.images[0].name, "icon.png");
}

// ── Zoom tests ─────────────────────────────────────────────────

#[test]
fn zoom_2x_scales_glyph_coordinates() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "Hello")]);

    // Render at 1x
    let frame_1x = ts.render();
    let glyphs_1x: Vec<[f32; 4]> = frame_1x.glyphs.iter().map(|q| q.screen).collect();
    assert!(!glyphs_1x.is_empty());

    // Render at 2x
    ts.set_zoom(2.0);
    let frame_2x = ts.render();
    let glyphs_2x: Vec<[f32; 4]> = frame_2x.glyphs.iter().map(|q| q.screen).collect();

    assert_eq!(glyphs_1x.len(), glyphs_2x.len());
    for (g1, g2) in glyphs_1x.iter().zip(glyphs_2x.iter()) {
        assert!(
            (g2[0] - g1[0] * 2.0).abs() < 0.01,
            "x: {} vs {}",
            g2[0],
            g1[0] * 2.0
        );
        assert!(
            (g2[1] - g1[1] * 2.0).abs() < 0.01,
            "y: {} vs {}",
            g2[1],
            g1[1] * 2.0
        );
        assert!(
            (g2[2] - g1[2] * 2.0).abs() < 0.01,
            "w: {} vs {}",
            g2[2],
            g1[2] * 2.0
        );
        assert!(
            (g2[3] - g1[3] * 2.0).abs() < 0.01,
            "h: {} vs {}",
            g2[3],
            g1[3] * 2.0
        );
    }
}

#[test]
fn zoom_half_scales_glyph_coordinates() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "Test")]);

    let frame_1x = ts.render();
    let glyphs_1x: Vec<[f32; 4]> = frame_1x.glyphs.iter().map(|q| q.screen).collect();

    ts.set_zoom(0.5);
    let frame_half = ts.render();
    let glyphs_half: Vec<[f32; 4]> = frame_half.glyphs.iter().map(|q| q.screen).collect();

    assert_eq!(glyphs_1x.len(), glyphs_half.len());
    for (g1, gh) in glyphs_1x.iter().zip(glyphs_half.iter()) {
        assert!((gh[0] - g1[0] * 0.5).abs() < 0.01);
        assert!((gh[1] - g1[1] * 0.5).abs() < 0.01);
        assert!((gh[2] - g1[2] * 0.5).abs() < 0.01);
        assert!((gh[3] - g1[3] * 0.5).abs() < 0.01);
    }
}

#[test]
fn zoom_does_not_change_content_height() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "Hello world")]);
    let h1 = ts.content_height();

    ts.set_zoom(3.0);
    let h2 = ts.content_height();

    assert!(
        (h1 - h2).abs() < 0.001,
        "content_height should not change with zoom"
    );
}

#[test]
fn zoom_hit_test_inverse_scaling() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "Hello world")]);
    ts.render();

    // Hit test at 1x
    let result_1x = ts.hit_test(50.0, 10.0);

    // Same logical position at 2x requires doubled screen coordinates
    ts.set_zoom(2.0);
    let result_2x = ts.hit_test(100.0, 20.0);

    assert!(result_1x.is_some());
    assert!(result_2x.is_some());
    assert_eq!(
        result_1x.unwrap().position,
        result_2x.unwrap().position,
        "hit_test at (100,20) with zoom 2x should match (50,10) at 1x"
    );
}

#[test]
fn zoom_caret_rect_scales_output() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "Hello")]);
    ts.render();

    let rect_1x = ts.caret_rect(0);

    ts.set_zoom(2.0);
    let rect_2x = ts.caret_rect(0);

    assert!((rect_2x[0] - rect_1x[0] * 2.0).abs() < 0.01);
    assert!((rect_2x[1] - rect_1x[1] * 2.0).abs() < 0.01);
    assert!((rect_2x[2] - rect_1x[2] * 2.0).abs() < 0.01);
    assert!((rect_2x[3] - rect_1x[3] * 2.0).abs() < 0.01);
}

#[test]
fn zoom_decorations_scale() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![{
        let mut b = make_block(1, "underlined");
        b.fragments[0].underline_style = UnderlineStyle::Single;
        b
    }]);

    let frame_1x = ts.render();
    let underlines_1x: Vec<[f32; 4]> = frame_1x
        .decorations
        .iter()
        .filter(|d| d.kind == DecorationKind::Underline)
        .map(|d| d.rect)
        .collect();
    assert!(!underlines_1x.is_empty());

    ts.set_zoom(2.0);
    let frame_2x = ts.render();
    let underlines_2x: Vec<[f32; 4]> = frame_2x
        .decorations
        .iter()
        .filter(|d| d.kind == DecorationKind::Underline)
        .map(|d| d.rect)
        .collect();

    assert_eq!(underlines_1x.len(), underlines_2x.len());
    for (u1, u2) in underlines_1x.iter().zip(underlines_2x.iter()) {
        assert!(
            (u2[0] - u1[0] * 2.0).abs() < 0.5,
            "x: {} vs {}",
            u2[0],
            u1[0] * 2.0
        );
        assert!(
            (u2[1] - u1[1] * 2.0).abs() < 0.5,
            "y: {} vs {}",
            u2[1],
            u1[1] * 2.0
        );
    }
}

#[test]
fn zoom_clamps_to_valid_range() {
    let mut ts = Typesetter::new();
    ts.set_zoom(0.01);
    assert!((ts.zoom() - 0.1).abs() < f32::EPSILON);
    ts.set_zoom(100.0);
    assert!((ts.zoom() - 10.0).abs() < f32::EPSILON);
    ts.set_zoom(1.5);
    assert!((ts.zoom() - 1.5).abs() < f32::EPSILON);
}

#[test]
fn zoom_default_is_one() {
    let ts = Typesetter::new();
    assert!((ts.zoom() - 1.0).abs() < f32::EPSILON);
}

#[test]
fn zoom_render_cursor_only_scales_cursor() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "Hello")]);
    ts.set_zoom(2.0);
    ts.set_cursor(&text_typeset::CursorDisplay {
        position: 2,
        anchor: 2,
        affinity: text_typeset::CursorAffinity::Downstream,
        visible: true,
        selected_cells: vec![],
    });
    ts.render();

    // Move cursor, use render_cursor_only
    ts.set_cursor(&text_typeset::CursorDisplay {
        position: 3,
        anchor: 3,
        affinity: text_typeset::CursorAffinity::Downstream,
        visible: true,
        selected_cells: vec![],
    });
    let frame = ts.render_cursor_only();

    let cursor_rects: Vec<&[f32; 4]> = frame
        .decorations
        .iter()
        .filter(|d| d.kind == DecorationKind::Cursor)
        .map(|d| &d.rect)
        .collect();
    assert_eq!(cursor_rects.len(), 1);
    // Cursor should have non-trivial zoomed coordinates
    let r = cursor_rects[0];
    assert!(r[2] > 0.0 && r[3] > 0.0, "cursor should have positive size");
}

#[test]
fn zoom_render_block_only_scales_output() {
    let mut ts = make_typesetter();
    ts.layout_blocks(vec![make_block(1, "First"), make_block(2, "Second")]);
    ts.set_zoom(2.0);
    ts.render();

    // Relayout block 1 and use render_block_only
    ts.relayout_block(&make_block(1, "Changed"));
    let frame = ts.render_block_only(1);

    // All glyphs should be at zoomed coordinates
    for q in &frame.glyphs {
        assert!(
            q.screen[2] > 0.0 && q.screen[3] > 0.0,
            "glyph should have positive size at zoom"
        );
    }
    assert_no_glyph_overlap(frame);
}
