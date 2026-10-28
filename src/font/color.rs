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

/// The colour tables of one face, read once, and whether the rasterizer can
/// draw the face at all.
#[derive(Debug, Default)]
pub(crate) struct ColorTables {
    /// Whether the face has glyph data the rasterizer draws: `glyf`, `CFF` or
    /// `CFF2` outlines, or colour bitmaps or paints (which Apple Color Emoji,
    /// `sbix` only, relies on). A face with none of them, such as PingFang on
    /// macOS 27 whose outlines are in Apple's `hvgl` table, shapes and maps
    /// characters but draws nothing.
    drawable: bool,
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
        let outlines = has(b"glyf") || has(b"CFF ") || has(b"CFF2");
        Self {
            drawable: outlines || color_bitmaps || !colr_v0.is_empty() || !colr_v1.is_empty(),
            colr_v0,
            colr_v1,
            color_bitmaps,
        }
    }

    /// Whether the rasterizer can draw any glyph of the face. Glyph fallback
    /// never picks a face that cannot: its characters would come out blank.
    pub(crate) fn drawable(&self) -> bool {
        self.drawable
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

/// `bytes`, a single-face font, without the tables named in `drop` and with
/// the tables of `add` (empty ones will do for a test that reads only which
/// tables exist), as a test's stand-in for a face with other glyph data.
#[cfg(test)]
pub(crate) fn with_tables(bytes: &[u8], drop: &[&[u8; 4]], add: &[&[u8; 4]]) -> Vec<u8> {
    let count = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
    let mut tables: Vec<(Vec<u8>, Vec<u8>)> = (0..count)
        .map(|i| {
            let record = &bytes[12 + 16 * i..28 + 16 * i];
            let offset = u32::from_be_bytes(record[8..12].try_into().unwrap()) as usize;
            let length = u32::from_be_bytes(record[12..16].try_into().unwrap()) as usize;
            (
                record[..4].to_vec(),
                bytes[offset..offset + length].to_vec(),
            )
        })
        .filter(|(tag, _)| !drop.iter().any(|t| t.as_slice() == tag.as_slice()))
        .collect();
    tables.extend(add.iter().map(|tag| (tag.to_vec(), vec![0; 4])));
    // The directory is sorted by tag, which table lookup relies on.
    tables.sort();
    let mut out = bytes[..4].to_vec();
    out.extend((tables.len() as u16).to_be_bytes());
    out.extend([0; 6]);
    let mut offset = 12 + 16 * tables.len();
    for (tag, data) in &tables {
        out.extend_from_slice(tag);
        out.extend([0; 4]);
        out.extend((offset as u32).to_be_bytes());
        out.extend((data.len() as u32).to_be_bytes());
        offset += data.len().next_multiple_of(4);
    }
    for (_, data) in &tables {
        out.extend_from_slice(data);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    out
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

    #[test]
    fn a_face_is_drawable_by_its_outlines_or_its_colour_tables() {
        assert!(ColorTables::read(PLAIN, 0).drawable());
        assert!(ColorTables::read(COLRV1, 0).drawable());
        assert!(ColorTables::read(CBDT, 0).drawable());
        // Apple Color Emoji keeps its glyphs in sbix alone.
        let no_outlines = with_tables(PLAIN, &[b"glyf", b"loca"], &[]);
        assert!(!ColorTables::read(&no_outlines, 0).drawable());
        let sbix_only = with_tables(PLAIN, &[b"glyf", b"loca"], &[b"sbix"]);
        assert!(ColorTables::read(&sbix_only, 0).drawable());
        let cff2 = with_tables(PLAIN, &[b"glyf", b"loca"], &[b"CFF2"]);
        assert!(ColorTables::read(&cff2, 0).drawable());
    }
}
