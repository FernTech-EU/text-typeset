//! What a [`DocumentFlow`](crate::DocumentFlow) fills in for a block that
//! does not say: its language, its base direction, and the direction an
//! `Auto` paragraph with no strongly directional character takes.
//!
//! A block says nothing about one of them when it carries the unset value:
//! `language: None`, `base_direction: Auto`, `neutral_direction: Auto`. The
//! bridge converts a text-document block with no stored language and no
//! stored direction into exactly that, so a host can give a whole flow (a
//! text field, an editor) a language and a direction without writing either
//! into the document, where they would be content: undoable, saved, pasted.

use std::borrow::Cow;

use crate::layout::block::BlockLayoutParams;
use crate::layout::frame::FrameLayoutParams;
use crate::layout::table::TableLayoutParams;
use crate::shaping::shaper::TextDirection;

/// The flow's defaults. All unset by default, which changes nothing.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct BlockDefaults {
    pub(crate) language: Option<String>,
    pub(crate) direction: TextDirection,
    pub(crate) neutral_direction: TextDirection,
}

impl BlockDefaults {
    /// Whether any default is set, so that applying them could change a
    /// block.
    pub(crate) fn is_set(&self) -> bool {
        self.language.is_some()
            || self.direction != TextDirection::Auto
            || self.neutral_direction != TextDirection::Auto
    }

    /// Whether applying the defaults would change `params`.
    fn changes(&self, params: &BlockLayoutParams) -> bool {
        (params.language.is_none() && self.language.is_some())
            || (params.base_direction == TextDirection::Auto
                && self.direction != TextDirection::Auto)
            || (params.neutral_direction == TextDirection::Auto
                && self.neutral_direction != TextDirection::Auto)
    }

    /// Fill in what `params` leaves unset.
    pub(crate) fn apply_block(&self, params: &mut BlockLayoutParams) {
        if params.language.is_none() {
            params.language.clone_from(&self.language);
        }
        if params.base_direction == TextDirection::Auto {
            params.base_direction = self.direction;
        }
        if params.neutral_direction == TextDirection::Auto {
            params.neutral_direction = self.neutral_direction;
        }
    }

    /// Fill in every block of every cell of `table`.
    pub(crate) fn apply_table(&self, table: &mut TableLayoutParams) {
        for cell in &mut table.cells {
            for block in &mut cell.blocks {
                self.apply_block(block);
            }
        }
    }

    /// Fill in every block of `frame`, its tables and its nested frames.
    pub(crate) fn apply_frame(&self, frame: &mut FrameLayoutParams) {
        for (_, block) in &mut frame.blocks {
            self.apply_block(block);
        }
        for (_, table) in &mut frame.tables {
            self.apply_table(table);
        }
        for (_, nested) in &mut frame.frames {
            self.apply_frame(nested);
        }
    }

    /// `params` with the defaults filled in, copied only when they change it.
    pub(crate) fn block<'p>(&self, params: &'p BlockLayoutParams) -> Cow<'p, BlockLayoutParams> {
        if self.changes(params) {
            let mut owned = params.clone();
            self.apply_block(&mut owned);
            Cow::Owned(owned)
        } else {
            Cow::Borrowed(params)
        }
    }

    /// `table` with the defaults filled in, copied only when one is set.
    pub(crate) fn table<'p>(&self, table: &'p TableLayoutParams) -> Cow<'p, TableLayoutParams> {
        if self.is_set() {
            let mut owned = table.clone();
            self.apply_table(&mut owned);
            Cow::Owned(owned)
        } else {
            Cow::Borrowed(table)
        }
    }

    /// `frame` with the defaults filled in, copied only when one is set.
    pub(crate) fn frame<'p>(&self, frame: &'p FrameLayoutParams) -> Cow<'p, FrameLayoutParams> {
        if self.is_set() {
            let mut owned = frame.clone();
            self.apply_frame(&mut owned);
            Cow::Owned(owned)
        } else {
            Cow::Borrowed(frame)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::frame::{FrameBorderStyle, FramePosition};
    use crate::layout::table::CellLayoutParams;

    fn block(language: Option<&str>, direction: TextDirection) -> BlockLayoutParams {
        BlockLayoutParams {
            block_id: 0,
            position: 0,
            text: String::new(),
            fragments: Vec::new(),
            alignment: crate::layout::paragraph::Alignment::Start,
            top_margin: 0.0,
            bottom_margin: 0.0,
            left_margin: 0.0,
            right_margin: 0.0,
            text_indent: 0.0,
            list_marker: String::new(),
            list_indent: 0.0,
            tab_positions: Vec::new(),
            base_direction: direction,
            neutral_direction: TextDirection::Auto,
            language: language.map(str::to_owned),
            line_height_multiplier: None,
            non_breakable_lines: false,
            hyphenation: None,
            checkbox: None,
            background_color: None,
        }
    }

    fn defaults() -> BlockDefaults {
        BlockDefaults {
            language: Some("ar".to_owned()),
            direction: TextDirection::RightToLeft,
            neutral_direction: TextDirection::RightToLeft,
        }
    }

    #[test]
    fn unset_defaults_change_nothing_and_copy_nothing() {
        let unset = BlockDefaults::default();
        assert!(!unset.is_set());
        let params = block(None, TextDirection::Auto);
        assert!(matches!(unset.block(&params), Cow::Borrowed(_)));
    }

    #[test]
    fn only_what_a_block_leaves_unset_is_filled() {
        let mut open = block(None, TextDirection::Auto);
        defaults().apply_block(&mut open);
        assert_eq!(open.language.as_deref(), Some("ar"));
        assert_eq!(open.base_direction, TextDirection::RightToLeft);
        assert_eq!(open.neutral_direction, TextDirection::RightToLeft);

        let mut stated = block(Some("ja"), TextDirection::LeftToRight);
        stated.neutral_direction = TextDirection::LeftToRight;
        assert!(matches!(defaults().block(&stated), Cow::Borrowed(_)));
        defaults().apply_block(&mut stated);
        assert_eq!(stated.language.as_deref(), Some("ja"));
        assert_eq!(stated.base_direction, TextDirection::LeftToRight);
        assert_eq!(stated.neutral_direction, TextDirection::LeftToRight);
    }

    #[test]
    fn tables_and_nested_frames_are_filled_to_the_bottom() {
        let table = TableLayoutParams {
            table_id: 1,
            rows: 1,
            columns: 1,
            column_widths: Vec::new(),
            border_width: 0.0,
            cell_spacing: 0.0,
            cell_padding: 0.0,
            cells: vec![CellLayoutParams {
                row: 0,
                column: 0,
                blocks: vec![block(None, TextDirection::Auto)],
                background_color: None,
            }],
        };
        let frame = |blocks, tables, frames| FrameLayoutParams {
            frame_id: 2,
            position: FramePosition::default(),
            width: None,
            height: None,
            margin_top: 0.0,
            margin_bottom: 0.0,
            margin_left: 0.0,
            margin_right: 0.0,
            padding: 0.0,
            border_width: 0.0,
            border_style: FrameBorderStyle::default(),
            blocks,
            tables,
            frames,
        };
        let inner = frame(vec![(0, block(None, TextDirection::Auto))], vec![], vec![]);
        let outer = frame(vec![], vec![(0, table)], vec![(1, inner)]);
        let filled = defaults().frame(&outer).into_owned();
        let cell_block = &filled.tables[0].1.cells[0].blocks[0];
        assert_eq!(cell_block.language.as_deref(), Some("ar"));
        let nested_block = &filled.frames[0].1.blocks[0].1;
        assert_eq!(nested_block.base_direction, TextDirection::RightToLeft);
    }
}
