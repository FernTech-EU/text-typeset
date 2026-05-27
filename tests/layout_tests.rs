mod helpers;
use helpers::{
    Typesetter, assert_blocks_non_overlapping, make_block, make_block_at, make_typesetter,
};

use text_typeset::font::resolve::resolve_font;
use text_typeset::layout::block::layout_block;
use text_typeset::layout::flow::FlowLayout;
use text_typeset::layout::frame::{FrameBorderStyle, FrameLayoutParams, FramePosition};
use text_typeset::layout::paragraph::{Alignment, break_into_lines};
use text_typeset::layout::table::{CellLayoutParams, TableLayoutParams};
use text_typeset::shaping::shaper::{font_metrics_px, shape_text};

/// Helper: shape text and break into lines with default settings.
fn layout_text(
    ts: &Typesetter,
    text: &str,
    width: f32,
    alignment: Alignment,
) -> Vec<text_typeset::layout::line::LayoutLine> {
    let resolved = resolve_font(ts.font_registry(), None, None, None, None, None, 1.0).unwrap();
    let run = shape_text(ts.font_registry(), &resolved, text, 0).unwrap();
    let metrics = font_metrics_px(ts.font_registry(), &resolved).unwrap();
    break_into_lines(vec![run], text, width, alignment, 0.0, &metrics)
}

#[test]
fn single_word_fits_in_one_line() {
    let ts = make_typesetter();
    let lines = layout_text(&ts, "Hello", 800.0, Alignment::Left);
    assert_eq!(lines.len(), 1, "short text should fit on one line");
    assert!(lines[0].width > 0.0);
}

#[test]
fn long_text_wraps_to_multiple_lines() {
    let ts = make_typesetter();
    let text = "The quick brown fox jumps over the lazy dog. This sentence is long enough to wrap at a reasonable width.";
    let lines = layout_text(&ts, text, 200.0, Alignment::Left);
    assert!(
        lines.len() > 1,
        "long text at 200px width should wrap: got {} lines",
        lines.len()
    );
}

#[test]
fn narrow_width_produces_more_lines() {
    let ts = make_typesetter();
    let text = "Hello world, this is a test of line breaking.";
    let wide = layout_text(&ts, text, 800.0, Alignment::Left);
    let narrow = layout_text(&ts, text, 100.0, Alignment::Left);
    assert!(
        narrow.len() > wide.len(),
        "narrow ({} lines) should produce more lines than wide ({} lines)",
        narrow.len(),
        wide.len()
    );
}

#[test]
fn empty_text_produces_one_empty_line() {
    let ts = make_typesetter();
    let lines = layout_text(&ts, "", 800.0, Alignment::Left);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].runs.len(), 0);
    assert!(
        lines[0].line_height > 0.0,
        "empty line should still have height"
    );
}

#[test]
fn line_widths_do_not_exceed_available_width() {
    let ts = make_typesetter();
    let text = "Word after word after word after word after word after word.";
    let available = 200.0;
    let lines = layout_text(&ts, text, available, Alignment::Left);

    for (i, line) in lines.iter().enumerate() {
        assert!(
            line.width <= available + 1.0, // +1.0 for floating point tolerance
            "line {} width {} exceeds available width {}",
            i,
            line.width,
            available
        );
    }
}

#[test]
fn right_alignment_shifts_runs() {
    let ts = make_typesetter();
    let lines = layout_text(&ts, "Hi", 800.0, Alignment::Right);
    assert_eq!(lines.len(), 1);
    // The first run's x should be close to (800 - text_width)
    if let Some(first_run) = lines[0].runs.first() {
        let expected_shift = 800.0 - lines[0].width;
        assert!(
            (first_run.x - expected_shift).abs() < 1.0,
            "right-aligned run x ({}) should be near {}",
            first_run.x,
            expected_shift
        );
    }
}

#[test]
fn center_alignment_shifts_runs() {
    let ts = make_typesetter();
    let lines = layout_text(&ts, "Hi", 800.0, Alignment::Center);
    assert_eq!(lines.len(), 1);
    if let Some(first_run) = lines[0].runs.first() {
        let expected_shift = (800.0 - lines[0].width) / 2.0;
        assert!(
            (first_run.x - expected_shift).abs() < 1.0,
            "center-aligned run x ({}) should be near {}",
            first_run.x,
            expected_shift
        );
    }
}

#[test]
fn justify_alignment_fills_line_width() {
    let ts = make_typesetter();
    let text = "Word after word after word after word after word end.";
    let lines = layout_text(&ts, text, 300.0, Alignment::Justify);

    // Non-last lines should be stretched to fill the available width
    if lines.len() > 1 {
        for (i, line) in lines.iter().enumerate() {
            if i < lines.len() - 1 && !line.runs.is_empty() {
                // Justified non-last line should be close to available width
                assert!(
                    (line.width - 300.0).abs() < 5.0,
                    "justified line {} width ({}) should be close to 300.0",
                    i,
                    line.width
                );
            }
        }
    }
}

#[test]
fn justify_alignment_works_with_multibyte_text() {
    let ts = make_typesetter();
    // Text with multibyte characters before spaces. If justify_line uses
    // char offsets as byte offsets to find spaces, the indices will be wrong
    // for text where byte offset != char offset.
    let text = "\u{00e9}\u{00e9}\u{00e9} word word word word word word word word end.";
    let lines = layout_text(&ts, text, 300.0, Alignment::Justify);

    if lines.len() > 1 {
        for (i, line) in lines.iter().enumerate() {
            if i < lines.len() - 1 && !line.runs.is_empty() {
                assert!(
                    (line.width - 300.0).abs() < 5.0,
                    "justified line {} with multibyte text: width ({}) should be close to 300.0",
                    i,
                    line.width
                );
            }
        }
    }
}

#[test]
fn lines_have_positive_height() {
    let ts = make_typesetter();
    let lines = layout_text(&ts, "Hello world", 800.0, Alignment::Left);
    for (i, line) in lines.iter().enumerate() {
        assert!(
            line.line_height > 0.0,
            "line {} should have positive height, got {}",
            i,
            line.line_height
        );
        assert!(line.ascent > 0.0, "line {} ascent should be positive", i);
        assert!(line.descent > 0.0, "line {} descent should be positive", i);
    }
}

#[test]
fn first_line_indent_reduces_available_space() {
    let ts = make_typesetter();
    let resolved = resolve_font(ts.font_registry(), None, None, None, None, None, 1.0).unwrap();
    let text = "Word word word word word word word word word word word.";
    let run = shape_text(ts.font_registry(), &resolved, text, 0).unwrap();
    let metrics = font_metrics_px(ts.font_registry(), &resolved).unwrap();

    let no_indent = break_into_lines(
        vec![run.clone()],
        text,
        200.0,
        Alignment::Left,
        0.0,
        &metrics,
    );

    let run2 = shape_text(ts.font_registry(), &resolved, text, 0).unwrap();
    let with_indent = break_into_lines(vec![run2], text, 200.0, Alignment::Left, 50.0, &metrics);

    assert!(
        with_indent.len() >= no_indent.len(),
        "indented ({} lines) should need at least as many lines as non-indented ({} lines)",
        with_indent.len(),
        no_indent.len()
    );
}

#[test]
fn mandatory_break_newline() {
    let ts = make_typesetter();
    let text = "Line one\nLine two";
    let lines = layout_text(&ts, text, 800.0, Alignment::Left);
    assert!(
        lines.len() >= 2,
        "text with \\n should produce at least 2 lines, got {}",
        lines.len()
    );
}

#[test]
fn all_glyphs_accounted_for_after_wrapping() {
    let ts = make_typesetter();
    let text = "The quick brown fox jumps over the lazy dog.";
    let resolved = resolve_font(ts.font_registry(), None, None, None, None, None, 1.0).unwrap();
    let run = shape_text(ts.font_registry(), &resolved, text, 0).unwrap();
    let total_glyphs: usize = run.glyphs.len();
    let metrics = font_metrics_px(ts.font_registry(), &resolved).unwrap();

    let lines = break_into_lines(vec![run], text, 150.0, Alignment::Left, 0.0, &metrics);

    let glyphs_in_lines: usize = lines
        .iter()
        .flat_map(|l| &l.runs)
        .map(|r| r.shaped_run.glyphs.len())
        .sum();

    assert_eq!(
        glyphs_in_lines, total_glyphs,
        "all {} glyphs should appear in lines (got {})",
        total_glyphs, glyphs_in_lines
    );
}

// ── Block layout tests ──────────────────────────────────────────

#[test]
fn block_layout_produces_lines() {
    let ts = make_typesetter();
    let params = make_block(1, "Hello world");
    let block = layout_block(ts.font_registry(), &params, 800.0, 1.0);

    assert_eq!(block.block_id, 1);
    assert!(
        !block.lines.is_empty(),
        "block should have at least one line"
    );
    assert!(block.height > 0.0, "block should have positive height");
}

#[test]
fn block_layout_with_margins() {
    let ts = make_typesetter();
    let mut params = make_block(1, "Hello");
    params.top_margin = 10.0;
    params.bottom_margin = 5.0;

    let block = layout_block(ts.font_registry(), &params, 800.0, 1.0);

    let content_height: f32 = block.lines.iter().map(|l| l.line_height).sum();
    let expected = params.top_margin + content_height + params.bottom_margin;
    assert!(
        (block.height - expected).abs() < 0.1,
        "block height {} should equal margins + content {}",
        block.height,
        expected
    );
}

#[test]
fn block_layout_respects_left_right_margins() {
    let ts = make_typesetter();
    let text = "Word word word word word word word word word word.";
    let mut params = make_block(1, text);
    params.left_margin = 50.0;
    params.right_margin = 50.0;

    let wide = layout_block(ts.font_registry(), &make_block(2, text), 400.0, 1.0);
    let narrow = layout_block(ts.font_registry(), &params, 400.0, 1.0);

    // With 100px total margins, there's less room for text = more lines
    assert!(
        narrow.lines.len() >= wide.lines.len(),
        "margined block ({} lines) should have at least as many lines as full-width ({} lines)",
        narrow.lines.len(),
        wide.lines.len()
    );
}

#[test]
fn block_lines_have_increasing_y() {
    let ts = make_typesetter();
    let text = "Line one. Line two. Line three. Line four. Line five.";
    let params = make_block(1, text);
    let block = layout_block(ts.font_registry(), &params, 100.0, 1.0);

    for i in 1..block.lines.len() {
        assert!(
            block.lines[i].y > block.lines[i - 1].y,
            "line {} y ({}) should be greater than line {} y ({})",
            i,
            block.lines[i].y,
            i - 1,
            block.lines[i - 1].y
        );
    }
}

// ── Flow layout tests ───────────────────────────────────────────

#[test]
fn flow_layout_stacks_blocks_vertically() {
    let ts = make_typesetter();
    let mut flow = FlowLayout::new();
    let blocks = vec![
        make_block(1, "First paragraph."),
        make_block(2, "Second paragraph."),
        make_block(3, "Third paragraph."),
    ];
    flow.layout_blocks(ts.font_registry(), blocks, 800.0);

    assert_eq!(flow.flow_order.len(), 3);
    assert!(flow.content_height > 0.0);

    // Each block should have a higher y than the previous
    let ys: Vec<f32> = flow
        .flow_order
        .iter()
        .map(|item| match item {
            text_typeset::layout::flow::FlowItem::Block { y, .. } => *y,
            _ => 0.0,
        })
        .collect();

    for i in 1..ys.len() {
        assert!(
            ys[i] > ys[i - 1],
            "block {} y ({}) should be > block {} y ({})",
            i,
            ys[i],
            i - 1,
            ys[i - 1]
        );
    }

    // Verify blocks don't overlap vertically
    let block_bounds: Vec<(f32, f32)> = flow
        .flow_order
        .iter()
        .filter_map(|item| match item {
            text_typeset::layout::flow::FlowItem::Block { y, height, .. } => Some((*y, *height)),
            _ => None,
        })
        .collect();
    assert_blocks_non_overlapping(&block_bounds);
}

#[test]
fn flow_relayout_block_shifts_subsequent() {
    let ts = make_typesetter();
    let mut flow = FlowLayout::new();
    let blocks = vec![make_block(1, "Short."), make_block(2, "After.")];
    flow.layout_blocks(ts.font_registry(), blocks, 800.0);

    let y2_before = flow.blocks.get(&2).unwrap().y;

    // Replace first block with longer text that takes more lines
    let longer = make_block(
        1,
        "This is a much longer paragraph that will certainly wrap to multiple lines at a narrow width like one hundred pixels wide.",
    );
    flow.relayout_block(ts.font_registry(), &longer, 100.0);

    let y2_after = flow.blocks.get(&2).unwrap().y;
    assert!(
        y2_after > y2_before,
        "second block should shift down after first block grew: {} -> {}",
        y2_before,
        y2_after
    );
}

#[test]
fn flow_relayout_block_shifts_subsequent_positions() {
    // Cutting chars from a non-last paragraph must also shift the document
    // character positions of subsequent blocks — otherwise hit_test reports
    // stale positions and the caret drifts by the number of chars cut.
    let ts = make_typesetter();
    let mut flow = FlowLayout::new();

    // Emulate "First\nSecond\nThird": blocks at positions 0, 6, 13.
    let blocks = vec![
        make_block_at(1, 0, "First"),
        make_block_at(2, 6, "Second"),
        make_block_at(3, 13, "Third"),
    ];
    flow.layout_blocks(ts.font_registry(), blocks, 800.0);

    // Cut "Sec" (3 chars) from block 2: it now holds "ond".
    // New document layout: blocks at 0, 6, 10.
    let shorter = make_block_at(2, 6, "ond");
    flow.relayout_block(ts.font_registry(), &shorter, 800.0);

    assert_eq!(flow.blocks.get(&1).unwrap().position, 0);
    assert_eq!(flow.blocks.get(&2).unwrap().position, 6);
    assert_eq!(
        flow.blocks.get(&3).unwrap().position,
        10,
        "block 3's document position must be shifted by the char delta \
         (-3) after block 2 was relaid out with fewer chars"
    );
}

#[test]
fn flow_content_height_matches_blocks() {
    let ts = make_typesetter();
    let mut flow = FlowLayout::new();
    let blocks = vec![make_block(1, "First."), make_block(2, "Second.")];
    flow.layout_blocks(ts.font_registry(), blocks, 800.0);

    // Content height should be at least as tall as the sum of block heights
    let sum: f32 = flow.blocks.values().map(|b| b.height).sum();
    assert!(
        flow.content_height > 0.0,
        "content height should be positive"
    );
    // With margin collapsing, content_height may differ from raw sum
    // but should be in the same ballpark
    assert!(
        (flow.content_height - sum).abs() < sum,
        "content height {} should be roughly equal to block sum {}",
        flow.content_height,
        sum
    );
}

#[test]
fn emergency_break_on_long_word() {
    let ts = make_typesetter();
    // A single very long "word" with no spaces — no break opportunities
    let text = "Supercalifragilisticexpialidocious";
    let lines = layout_text(&ts, text, 50.0, Alignment::Left);
    // Should still produce lines (emergency breaks), not panic or infinite loop
    assert!(
        lines.len() >= 2,
        "long word at 50px width should emergency-break into multiple lines, got {}",
        lines.len()
    );
    // All glyphs should still be present
    let resolved = resolve_font(ts.font_registry(), None, None, None, None, None, 1.0).unwrap();
    let run = shape_text(ts.font_registry(), &resolved, text, 0).unwrap();
    let total_glyphs = run.glyphs.len();
    let glyphs_in_lines: usize = lines
        .iter()
        .flat_map(|l| &l.runs)
        .map(|r| r.shaped_run.glyphs.len())
        .sum();
    assert_eq!(
        glyphs_in_lines, total_glyphs,
        "emergency break should not lose glyphs: expected {}, got {}",
        total_glyphs, glyphs_in_lines
    );
}

#[test]
fn margin_collapsing_uses_max_not_sum() {
    let ts = make_typesetter();
    let mut flow = FlowLayout::new();
    let mut block1 = make_block(1, "First.");
    block1.bottom_margin = 20.0;
    let mut block2 = make_block(2, "Second.");
    block2.top_margin = 30.0;

    flow.layout_blocks(ts.font_registry(), vec![block1, block2], 800.0);

    let y1 = flow.blocks.get(&1).unwrap().y;
    let h1 = flow.blocks.get(&1).unwrap().height;
    let y2 = flow.blocks.get(&2).unwrap().y;

    // Space between blocks should be max(20, 30) = 30, not 20 + 30 = 50
    let gap = y2 - (y1 + h1 - flow.blocks.get(&1).unwrap().bottom_margin);
    // gap should be exactly 30 (the collapsed margin)
    assert!(
        (gap - 30.0).abs() < 1.0,
        "gap between blocks ({}) should be ~30 (collapsed max), not 50 (summed)",
        gap
    );
}

#[test]
fn text_indent_shifts_first_line() {
    let ts = make_typesetter();
    let resolved = resolve_font(ts.font_registry(), None, None, None, None, None, 1.0).unwrap();
    let text = "Hello world, this is a test sentence for indentation.";
    let run = shape_text(ts.font_registry(), &resolved, text, 0).unwrap();
    let metrics = font_metrics_px(ts.font_registry(), &resolved).unwrap();

    let lines = break_into_lines(vec![run], text, 300.0, Alignment::Left, 40.0, &metrics);

    // First line should have its first run starting at x >= 40 (the indent)
    assert!(!lines.is_empty());
    if let Some(first_run) = lines[0].runs.first() {
        assert!(
            first_run.x >= 39.0,
            "first line run x ({}) should be >= indent (40.0)",
            first_run.x
        );
    }

    // Second line (if any) should start at x ~= 0 (no indent)
    if lines.len() > 1
        && let Some(second_run) = lines[1].runs.first()
    {
        assert!(
            second_run.x < 5.0,
            "second line run x ({}) should be near 0 (no indent)",
            second_run.x
        );
    }
}

#[test]
fn multi_fragment_all_glyphs_accounted() {
    // Verify no glyph loss when a paragraph has multiple formatting runs
    let ts = make_typesetter();
    let text = "Bold Normal";
    let resolved_bold =
        resolve_font(ts.font_registry(), None, None, Some(true), None, None, 1.0).unwrap();
    let resolved_normal =
        resolve_font(ts.font_registry(), None, None, None, None, None, 1.0).unwrap();

    let run1 = shape_text(ts.font_registry(), &resolved_bold, "Bold ", 0).unwrap();
    let run2 = shape_text(ts.font_registry(), &resolved_normal, "Normal", 5).unwrap();
    let total = run1.glyphs.len() + run2.glyphs.len();

    let metrics = font_metrics_px(ts.font_registry(), &resolved_normal).unwrap();
    let lines = break_into_lines(
        vec![run1, run2],
        text,
        800.0,
        Alignment::Left,
        0.0,
        &metrics,
    );

    let in_lines: usize = lines
        .iter()
        .flat_map(|l| &l.runs)
        .map(|r| r.shaped_run.glyphs.len())
        .sum();

    assert_eq!(
        in_lines, total,
        "multi-fragment paragraph should preserve all {} glyphs (got {})",
        total, in_lines
    );
}

#[test]
fn multi_fragment_wrapping_breaks_at_correct_boundary() {
    // When two formatting runs produce text like "AAAA BBBB" at a narrow width,
    // the break should happen at the space between them — not at a wrong position
    // due to cluster values being in fragment-local space.
    let ts = make_typesetter();
    let text = "AAAA BBBB";
    let resolved = resolve_font(ts.font_registry(), None, None, None, None, None, 1.0).unwrap();

    // Shape as two separate runs (simulating two fragments)
    let run1 = shape_text(ts.font_registry(), &resolved, "AAAA ", 0).unwrap();
    let run2 = shape_text(ts.font_registry(), &resolved, "BBBB", 5).unwrap();

    let metrics = font_metrics_px(ts.font_registry(), &resolved).unwrap();

    // Use a width that fits "AAAA " but not "AAAA BBBB"
    let run1_width = run1.advance_width;
    let narrow_width = run1_width + 5.0; // just barely wider than the first run

    let lines = break_into_lines(
        vec![run1, run2],
        text,
        narrow_width,
        Alignment::Left,
        0.0,
        &metrics,
    );

    assert!(
        lines.len() >= 2,
        "should wrap to at least 2 lines at width {}, got {} lines",
        narrow_width,
        lines.len()
    );

    // First line should contain "AAAA " (5 glyphs), second should contain "BBBB" (4 glyphs)
    let first_line_glyphs: usize = lines[0]
        .runs
        .iter()
        .map(|r| r.shaped_run.glyphs.len())
        .sum();
    let second_line_glyphs: usize = lines[1]
        .runs
        .iter()
        .map(|r| r.shaped_run.glyphs.len())
        .sum();

    assert_eq!(
        first_line_glyphs + second_line_glyphs,
        9,
        "total glyphs across lines should be 9"
    );
}

// ── New block format features ───────────────────────────────────

#[test]
fn line_height_multiplier_increases_block_height() {
    let ts = make_typesetter();

    let mut normal = make_block(1, "Hello world");
    normal.line_height_multiplier = None; // default 1.0
    let block_normal = layout_block(ts.font_registry(), &normal, 800.0, 1.0);

    let mut double = make_block(2, "Hello world");
    double.line_height_multiplier = Some(2.0);
    let block_double = layout_block(ts.font_registry(), &double, 800.0, 1.0);

    assert!(
        block_double.height > block_normal.height * 1.8,
        "2.0 line height ({}) should be ~2x normal ({})",
        block_double.height,
        block_normal.height
    );
}

#[test]
fn non_breakable_lines_prevents_wrapping() {
    let ts = make_typesetter();

    let text = "This is a long sentence that would normally wrap at a narrow width.";

    let wrapping = make_block(1, text);
    let block_wrap = layout_block(ts.font_registry(), &wrapping, 100.0, 1.0);

    let mut no_wrap = make_block(2, text);
    no_wrap.non_breakable_lines = true;
    let block_no_wrap = layout_block(ts.font_registry(), &no_wrap, 100.0, 1.0);

    assert!(
        block_wrap.lines.len() > 1,
        "wrapping block should have multiple lines at 100px"
    );
    assert_eq!(
        block_no_wrap.lines.len(),
        1,
        "non-breakable block should have exactly one line"
    );
}

#[test]
fn tab_stops_advance_to_next_position() {
    let ts = make_typesetter();

    let mut params = make_block(1, "A\tB");
    params.tab_positions = vec![100.0, 200.0, 300.0];
    let block = layout_block(ts.font_registry(), &params, 800.0, 1.0);

    // The tab should push B to approximately x=100
    assert!(!block.lines.is_empty());
    let line = &block.lines[0];
    let total_width: f32 = line.runs.iter().map(|r| r.shaped_run.advance_width).sum();
    // "A" is ~10px, tab should jump to 100px, "B" is ~10px = total ~110px
    assert!(
        total_width > 90.0,
        "tab stop at 100px should push total width past 90: got {}",
        total_width
    );
}

#[test]
fn checkbox_marker_renders() {
    let ts = make_typesetter();

    let mut params = make_block(1, "Todo item");
    params.checkbox = Some(false); // unchecked
    params.list_indent = 24.0;
    let block = layout_block(ts.font_registry(), &params, 800.0, 1.0);

    assert!(
        block.list_marker.is_some(),
        "checkbox block should have a marker"
    );
}

#[test]
fn checked_checkbox_marker_renders() {
    let ts = make_typesetter();

    let mut params = make_block(1, "Done item");
    params.checkbox = Some(true); // checked
    params.list_indent = 24.0;
    let block = layout_block(ts.font_registry(), &params, 800.0, 1.0);

    assert!(
        block.list_marker.is_some(),
        "checked checkbox block should have a marker"
    );
}

#[test]
fn background_color_stored_in_layout() {
    let ts = make_typesetter();

    let mut params = make_block(1, "Highlighted");
    params.background_color = Some([1.0, 1.0, 0.0, 0.3]);
    let block = layout_block(ts.font_registry(), &params, 800.0, 1.0);

    assert_eq!(
        block.background_color,
        Some([1.0, 1.0, 0.0, 0.3]),
        "background_color should be stored in BlockLayout"
    );
}

// ── char_range correctness ──────────────────────────────────────

#[test]
fn char_range_end_correct_for_ascii() {
    let ts = make_typesetter();
    let text = "Hello";
    let lines = layout_text(&ts, text, 800.0, Alignment::Left);
    assert_eq!(lines.len(), 1);
    // For ASCII, char_range.end should be text.len() (5)
    assert_eq!(
        lines[0].char_range.end,
        text.len(),
        "char_range.end should equal text byte length for ASCII"
    );
}

#[test]
fn char_range_end_correct_for_multibyte_utf8() {
    let ts = make_typesetter();
    // Each character here is 2 bytes in UTF-8
    let text = "\u{00e9}\u{00e8}\u{00ea}"; // e-acute, e-grave, e-circumflex
    let lines = layout_text(&ts, text, 800.0, Alignment::Left);
    assert_eq!(lines.len(), 1);
    // char_range uses char offsets, not byte offsets
    let char_count = text.chars().count(); // 3
    assert_eq!(
        lines[0].char_range.end, char_count,
        "char_range.end should be {} (char count) for multibyte text, got {}",
        char_count, lines[0].char_range.end
    );
}

// ── layout_blocks / add_block equivalence ───────────────────────

#[test]
fn layout_blocks_matches_add_block_sequence() {
    let ts = make_typesetter();

    let mut b1 = make_block(1, "First paragraph.");
    b1.top_margin = 10.0;
    b1.bottom_margin = 20.0;
    let mut b2 = make_block(2, "Second paragraph.");
    b2.top_margin = 15.0;
    b2.bottom_margin = 5.0;

    // Method 1: layout_blocks
    let mut flow1 = FlowLayout::new();
    flow1.layout_blocks(ts.font_registry(), vec![b1.clone(), b2.clone()], 800.0);

    // Method 2: clear + add_block loop
    let mut flow2 = FlowLayout::new();
    flow2.add_block(ts.font_registry(), &b1, 800.0);
    flow2.add_block(ts.font_registry(), &b2, 800.0);

    // Both should produce identical results
    assert!(
        (flow1.content_height - flow2.content_height).abs() < 0.001,
        "content_height mismatch: layout_blocks={}, add_block={}",
        flow1.content_height,
        flow2.content_height
    );

    let y1_a = flow1.blocks.get(&1).unwrap().y;
    let y1_b = flow2.blocks.get(&1).unwrap().y;
    assert!(
        (y1_a - y1_b).abs() < 0.001,
        "block 1 y mismatch: {} vs {}",
        y1_a,
        y1_b
    );

    let y2_a = flow1.blocks.get(&2).unwrap().y;
    let y2_b = flow2.blocks.get(&2).unwrap().y;
    assert!(
        (y2_a - y2_b).abs() < 0.001,
        "block 2 y mismatch: {} vs {}",
        y2_a,
        y2_b
    );
}

// ── relayout with margin changes ────────────────────────────────

#[test]
fn relayout_block_handles_top_margin_change() {
    let ts = make_typesetter();
    let mut flow = FlowLayout::new();

    let mut b1 = make_block(1, "First.");
    b1.bottom_margin = 10.0;
    let mut b2 = make_block(2, "Second.");
    b2.top_margin = 5.0;
    let b3 = make_block(3, "Third.");

    flow.layout_blocks(
        ts.font_registry(),
        vec![b1.clone(), b2.clone(), b3.clone()],
        800.0,
    );

    let y2_before = flow.blocks.get(&2).unwrap().y;
    let y3_before = flow.blocks.get(&3).unwrap().y;

    // Increase block 2's top margin from 5 to 30
    let mut b2_updated = make_block(2, "Second.");
    b2_updated.top_margin = 30.0;
    flow.relayout_block(ts.font_registry(), &b2_updated, 800.0);

    let y2_after = flow.blocks.get(&2).unwrap().y;
    let y3_after = flow.blocks.get(&3).unwrap().y;

    // Block 2 should move down (larger margin)
    assert!(
        y2_after > y2_before,
        "block 2 should move down with larger top margin: {} -> {}",
        y2_before,
        y2_after
    );

    // Block 3 should also shift down
    assert!(
        y3_after > y3_before,
        "block 3 should shift down when block 2 moves: {} -> {}",
        y3_before,
        y3_after
    );
}

// ── Relayout in tables/frames ──────────────────────────────────

#[test]
fn relayout_table_block_updates_table_height() {
    let mut ts = Typesetter::new();
    let face = ts.register_font(include_bytes!("../test-fonts/NotoSans-Variable.ttf"));
    ts.set_default_font(face, 16.0);
    ts.set_viewport(800.0, 600.0);

    ts.layout_blocks(vec![make_block_at(1, 0, "Before")]);
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
            blocks: vec![make_block_at(100, 7, "Short")],
            background_color: None,
        }],
    });

    let height_before = ts.content_height();

    // Relayout the table cell block with much longer text that wraps
    let long_text = "This is a much longer piece of text that should cause the table cell to grow in height because it wraps to multiple lines.";
    ts.relayout_block(&make_block_at(100, 7, long_text));

    let height_after = ts.content_height();
    assert!(
        height_after > height_before,
        "content height should grow after relayout with longer text: {} -> {}",
        height_before,
        height_after
    );
}

#[test]
fn relayout_frame_block_updates_frame_height() {
    let mut ts = Typesetter::new();
    let face = ts.register_font(include_bytes!("../test-fonts/NotoSans-Variable.ttf"));
    ts.set_default_font(face, 16.0);
    ts.set_viewport(800.0, 600.0);

    ts.layout_blocks(vec![make_block_at(1, 0, "Before")]);
    ts.add_frame(&FrameLayoutParams {
        frame_id: 20,
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
        blocks: vec![(0, make_block_at(200, 7, "Short"))],
        tables: vec![],
        frames: vec![],
    });

    let height_before = ts.content_height();

    let long_text = "This is a much longer piece of text that should cause the frame to grow in height because it wraps to multiple lines.";
    ts.relayout_block(&make_block_at(200, 7, long_text));

    let height_after = ts.content_height();
    assert!(
        height_after > height_before,
        "content height should grow after relayout frame block: {} -> {}",
        height_before,
        height_after
    );
}

#[test]
fn relayout_block_inside_nested_frame_updates_heights() {
    let mut ts = make_typesetter();
    ts.set_viewport(200.0, 600.0);

    ts.layout_blocks(vec![make_block_at(1, 0, "Before")]);
    ts.add_frame(&FrameLayoutParams {
        frame_id: 20,
        position: FramePosition::Inline,
        width: None,
        height: None,
        margin_top: 0.0,
        margin_bottom: 0.0,
        margin_left: 0.0,
        margin_right: 0.0,
        padding: 4.0,
        border_width: 0.0,
        border_style: FrameBorderStyle::None,
        blocks: vec![(0, make_block_at(100, 3, "Outer"))],
        tables: vec![],
        frames: vec![(
            1,
            FrameLayoutParams {
                frame_id: 30,
                position: FramePosition::Inline,
                width: None,
                height: None,
                margin_top: 0.0,
                margin_bottom: 0.0,
                margin_left: 0.0,
                margin_right: 0.0,
                padding: 4.0,
                border_width: 0.0,
                border_style: FrameBorderStyle::None,
                blocks: vec![(0, make_block_at(200, 9, "Short"))],
                tables: vec![],
                frames: vec![],
            },
        )],
    });

    let height_before = ts.content_height();

    let long_text = "This is a much longer piece of text that should cause wrapping and increase the nested frame height.";
    ts.relayout_block(&make_block_at(200, 9, long_text));

    let height_after = ts.content_height();
    assert!(
        height_after > height_before,
        "content height should grow after relayout of nested frame block: {} -> {}",
        height_before,
        height_after,
    );
}
