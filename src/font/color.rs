//! Whether the rasterizer draws a glyph in colour or plain.
//!
//! The rasterizer paints COLRv0 layers, CBDT/sbix bitmaps and COLRv1 paints
//! in colour (the last in [`crate::atlas::colr`]) and everything else as a
//! monochrome outline. Glyph fallback reads this to prefer a face that draws
//! an emoji in colour.

use ttf_parser::{Face, Tag};

use crate::font::registry::FontEntry;

/// How a face's glyph comes out of the rasterizer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GlyphPaint {
    /// In colour: a COLRv0 layer list, a colour bitmap or a COLRv1 paint.
    Color,
    /// As the glyph's outline, in the text colour.
    Plain,
}

/// The colour tables of one face, read once.
#[derive(Debug, Default)]
pub(crate) struct ColorTables {
    /// Glyphs with a COLRv0 layer list, sorted. Empty unless the face also
    /// has the CPAL palette swash needs to paint them.
    colr_v0: Vec<u16>,
    /// Glyphs with a COLRv1 paint, sorted.
    colr_v1: Vec<u16>,
    /// Whether the face has colour bitmaps (CBDT with CBLC, or sbix).
    color_bitmaps: bool,
}

impl ColorTables {
    pub(crate) fn read(bytes: &[u8], face_index: u32) -> Self {
        let Ok(face) = Face::parse(bytes, face_index) else {
            return Self::default();
        };
        let raw = face.raw_face();
        let has = |tag: &[u8; 4]| raw.table(Tag::from_bytes(tag)).is_some();
        let color_bitmaps = (has(b"CBDT") && has(b"CBLC")) || has(b"sbix");
        let (mut colr_v0, colr_v1) = raw
            .table(Tag::from_bytes(b"COLR"))
            .map(read_colr_base_glyphs)
            .unwrap_or_default();
        if !has(b"CPAL") {
            colr_v0.clear();
        }
        Self {
            colr_v0,
            colr_v1,
            color_bitmaps,
        }
    }
}

/// How `entry`'s glyph `glyph_id` comes out of the rasterizer.
pub(crate) fn glyph_paint(entry: &FontEntry, glyph_id: u16) -> GlyphPaint {
    let tables = entry.color_tables();
    if tables.colr_v0.binary_search(&glyph_id).is_ok() {
        return GlyphPaint::Color;
    }
    if tables.color_bitmaps && has_color_bitmap(entry, glyph_id) {
        return GlyphPaint::Color;
    }
    if tables.colr_v1.binary_search(&glyph_id).is_ok() {
        return GlyphPaint::Color;
    }
    GlyphPaint::Plain
}

fn has_color_bitmap(entry: &FontEntry, glyph_id: u16) -> bool {
    swash::FontRef::from_index(entry.bytes(), entry.face_index as usize)
        .is_some_and(|font| font.color_strikes().any(|strike| strike.contains(glyph_id)))
}

/// The base glyphs of a COLR table: those with a version 0 layer list, and
/// those with a version 1 paint. Both lists are sorted by glyph id in the
/// table, as the format requires.
fn read_colr_base_glyphs(colr: &[u8]) -> (Vec<u16>, Vec<u16>) {
    let u16_at = |at: usize| {
        colr.get(at..at + 2)
            .map(|b| u16::from_be_bytes([b[0], b[1]]))
    };
    let u32_at = |at: usize| {
        colr.get(at..at + 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize)
    };

    let mut v0 = Vec::new();
    let mut v1 = Vec::new();
    let Some(version) = u16_at(0) else {
        return (v0, v1);
    };

    // Version 0 header: numBaseGlyphRecords, baseGlyphRecordsOffset. Each
    // BaseGlyph record is 6 bytes and starts with its glyph id.
    if let (Some(count), Some(offset)) = (u16_at(2), u32_at(4)) {
        for i in 0..count as usize {
            match u16_at(offset + i * 6) {
                Some(glyph) => v0.push(glyph),
                None => break,
            }
        }
    }

    // Version 1 adds baseGlyphListOffset at byte 14: a u32 count, then
    // 6-byte BaseGlyphPaint records, each starting with its glyph id.
    if version >= 1
        && let Some(list) = u32_at(14).filter(|&offset| offset != 0)
        && let Some(count) = u32_at(list)
    {
        for i in 0..count {
            match u16_at(list + 4 + i * 6) {
                Some(glyph) => v1.push(glyph),
                None => break,
            }
        }
    }

    (v0, v1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const COLRV1: &[u8] = include_bytes!("../../test-fonts/NotoColorEmoji-COLRv1-subset.ttf");
    const CBDT: &[u8] = include_bytes!("../../test-fonts/NotoColorEmoji-CBDT-subset.ttf");
    const PLAIN: &[u8] = include_bytes!("../../test-fonts/NotoSansSymbols2-Heart-subset.ttf");

    fn glyph(bytes: &[u8], ch: char) -> u16 {
        swash::FontRef::from_index(bytes, 0)
            .unwrap()
            .charmap()
            .map(ch)
    }

    #[test]
    fn colr_v1_base_glyphs_are_read() {
        let tables = ColorTables::read(COLRV1, 0);
        let grin = glyph(COLRV1, '\u{1F600}');
        assert_ne!(grin, 0);
        assert!(
            tables.colr_v0.is_empty(),
            "Noto's COLRv1 build has no v0 layers"
        );
        assert!(tables.colr_v1.binary_search(&grin).is_ok());
        assert!(!tables.color_bitmaps);
    }

    #[test]
    fn colour_bitmaps_and_plain_faces_are_told_apart() {
        assert!(ColorTables::read(CBDT, 0).color_bitmaps);
        let plain = ColorTables::read(PLAIN, 0);
        assert!(!plain.color_bitmaps && plain.colr_v0.is_empty() && plain.colr_v1.is_empty());
    }
}
