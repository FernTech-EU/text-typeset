use harfrust::{Direction, FontRef, ShaperData, UnicodeBuffer};

use crate::font::registry::FontRegistry;
use crate::font::resolve::ResolvedFont;
use crate::shaping::run::{ShapedGlyph, ShapedRun};

/// Read units-per-em for a font face.
///
/// `harfrust::FontRef` is a thin wrapper over read-fonts and exposes
/// the `head` table only through the `read_fonts::TableProvider` trait,
/// which harfrust doesn't re-export. Since we already depend on swash
/// for `font_metrics_px` further down, we reuse swash's `Metrics` to
/// pull UPEM — one less dependency surface to maintain.
fn units_per_em(bytes: &[u8], face_index: u32) -> Option<u16> {
    let font_ref = swash::FontRef::from_index(bytes, face_index as usize)?;
    let upem = font_ref.metrics(&[]).units_per_em;
    if upem == 0 { None } else { Some(upem) }
}

/// Text direction for shaping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextDirection {
    /// Auto-detect from text content (default).
    #[default]
    Auto,
    LeftToRight,
    RightToLeft,
}

/// Shape a text string with the given resolved font.
///
/// Returns a ShapedRun with glyph IDs and pixel-space positions.
/// The `text_offset` is the byte offset of this text within the block
/// (used for cluster mapping back to document positions).
/// Shape a text string with automatic glyph fallback.
///
/// After shaping with the primary font, any .notdef glyphs (glyph_id==0)
/// are detected and re-shaped with fallback fonts. If no fallback font
/// covers a character, it remains as .notdef (renders as blank space
/// with correct advance).
pub fn shape_text(
    registry: &FontRegistry,
    resolved: &ResolvedFont,
    text: &str,
    text_offset: usize,
) -> Option<ShapedRun> {
    shape_text_with_fallback(registry, resolved, text, text_offset, TextDirection::Auto)
}

/// Shape text with an explicit direction and glyph fallback.
///
/// Like `shape_text`, but caller supplies the direction instead of letting
/// rustybuzz guess. Used by the bidi-aware layout path, which splits text
/// into directional runs before shaping.
pub fn shape_text_with_fallback(
    registry: &FontRegistry,
    resolved: &ResolvedFont,
    text: &str,
    text_offset: usize,
    direction: TextDirection,
) -> Option<ShapedRun> {
    let mut run = shape_text_directed(registry, resolved, text, text_offset, direction)?;

    // Check for .notdef glyphs and attempt fallback
    if run.glyphs.iter().any(|g| g.glyph_id == 0) && !text.is_empty() {
        apply_glyph_fallback(registry, resolved, text, text_offset, &mut run);
    }

    Some(run)
}

/// Re-shape .notdef glyphs using fallback fonts.
///
/// For each .notdef glyph, finds the source character via the cluster value,
/// queries all registered fonts for coverage, and if one covers it,
/// shapes that single character with the fallback font and replaces
/// the .notdef glyph with the result.
fn apply_glyph_fallback(
    registry: &FontRegistry,
    primary: &ResolvedFont,
    text: &str,
    text_offset: usize,
    run: &mut ShapedRun,
) {
    use crate::font::resolve::find_fallback_font;

    for glyph in &mut run.glyphs {
        if glyph.glyph_id != 0 {
            continue;
        }

        // Find the character that produced this .notdef
        let byte_offset = glyph.cluster as usize;
        let ch = match text.get(byte_offset..).and_then(|s| s.chars().next()) {
            Some(c) => c,
            None => continue,
        };

        // Find a fallback font that has this character
        let fallback_id = match find_fallback_font(registry, ch, primary.font_face_id) {
            Some(id) => id,
            None => continue, // no fallback available -leave as .notdef
        };

        let fallback_entry = match registry.get(fallback_id) {
            Some(e) => e,
            None => continue,
        };

        // Shape just this character with the fallback font
        let fallback_resolved = ResolvedFont {
            font_face_id: fallback_id,
            size_px: primary.size_px,
            face_index: fallback_entry.face_index,
            swash_cache_key: fallback_entry.swash_cache_key,
            scale_factor: primary.scale_factor,
            weight: primary.weight,
        };

        let char_str = &text[byte_offset..byte_offset + ch.len_utf8()];
        if let Some(fallback_run) = shape_text_directed(
            registry,
            &fallback_resolved,
            char_str,
            text_offset + byte_offset,
            TextDirection::Auto,
        ) {
            // Replace the .notdef glyph with the fallback glyph(s)
            if let Some(fb_glyph) = fallback_run.glyphs.first() {
                glyph.glyph_id = fb_glyph.glyph_id;
                glyph.x_advance = fb_glyph.x_advance;
                glyph.y_advance = fb_glyph.y_advance;
                glyph.x_offset = fb_glyph.x_offset;
                glyph.y_offset = fb_glyph.y_offset;
                glyph.font_face_id = fallback_id;
            }
        }
    }

    // Recompute total advance
    run.advance_width = run.glyphs.iter().map(|g| g.x_advance).sum();
}

/// Shape text with an explicit direction.
pub fn shape_text_directed(
    registry: &FontRegistry,
    resolved: &ResolvedFont,
    text: &str,
    text_offset: usize,
    direction: TextDirection,
) -> Option<ShapedRun> {
    let entry = registry.get(resolved.font_face_id)?;
    let font = FontRef::from_index(entry.bytes(), entry.face_index).ok()?;

    let upem = units_per_em(entry.bytes(), entry.face_index).unwrap_or(0) as f32;
    if upem == 0.0 {
        return None;
    }
    // Shape at physical ppem, then divide results by scale_factor so
    // downstream layout stays in logical pixels. See ResolvedFont.
    let sf = resolved.scale_factor.max(f32::MIN_POSITIVE);
    let physical_size = resolved.size_px * sf;
    let physical_scale = physical_size / upem;
    let inv_sf = 1.0 / sf;

    let mut buffer = UnicodeBuffer::new();
    buffer.push_str(text);
    match direction {
        TextDirection::LeftToRight => buffer.set_direction(Direction::LeftToRight),
        TextDirection::RightToLeft => buffer.set_direction(Direction::RightToLeft),
        // harfrust panics if direction is left Invalid; explicitly
        // guess script + language + direction from the buffer content.
        TextDirection::Auto => buffer.guess_segment_properties(),
    }

    // ShaperData preprocesses font tables for shaping. For one-shot
    // shape calls we build it inline; a future optimisation can cache
    // it per `FontFaceId` if profiling shows it matters.
    let shaper_data = ShaperData::new(&font);
    let shaper = shaper_data.shaper(&font).build();
    let glyph_buffer = shaper.shape(buffer, &[]);

    let infos = glyph_buffer.glyph_infos();
    let positions = glyph_buffer.glyph_positions();

    let mut glyphs = Vec::with_capacity(infos.len());
    let mut total_advance = 0.0f32;

    for (info, pos) in infos.iter().zip(positions.iter()) {
        let x_advance = pos.x_advance as f32 * physical_scale * inv_sf;
        let y_advance = pos.y_advance as f32 * physical_scale * inv_sf;
        let x_offset = pos.x_offset as f32 * physical_scale * inv_sf;
        let y_offset = pos.y_offset as f32 * physical_scale * inv_sf;

        glyphs.push(ShapedGlyph {
            glyph_id: info.glyph_id as u16,
            cluster: info.cluster,
            x_advance,
            y_advance,
            x_offset,
            y_offset,
            font_face_id: resolved.font_face_id,
        });

        total_advance += x_advance;
    }

    Some(ShapedRun {
        font_face_id: resolved.font_face_id,
        size_px: resolved.size_px,
        weight: resolved.weight,
        glyphs,
        advance_width: total_advance,
        text_range: text_offset..text_offset + text.len(),
        underline_style: crate::types::UnderlineStyle::None,
        overline: false,
        strikeout: false,
        is_link: false,
        foreground_color: None,
        underline_color: None,
        background_color: None,
        anchor_href: None,
        tooltip: None,
        vertical_alignment: crate::types::VerticalAlignment::Normal,
        image_name: None,
        image_height: 0.0,
    })
}

/// Shape a text string, reusing a UnicodeBuffer to avoid allocations.
pub fn shape_text_with_buffer(
    registry: &FontRegistry,
    resolved: &ResolvedFont,
    text: &str,
    text_offset: usize,
    buffer: UnicodeBuffer,
) -> Option<(ShapedRun, UnicodeBuffer)> {
    let entry = registry.get(resolved.font_face_id)?;
    let font = FontRef::from_index(entry.bytes(), entry.face_index).ok()?;

    let upem = units_per_em(entry.bytes(), entry.face_index).unwrap_or(0) as f32;
    if upem == 0.0 {
        return None;
    }
    let sf = resolved.scale_factor.max(f32::MIN_POSITIVE);
    let physical_size = resolved.size_px * sf;
    let physical_scale = physical_size / upem;
    let inv_sf = 1.0 / sf;

    let mut buffer = buffer;
    buffer.push_str(text);
    // Recycled buffers come back without segment properties; explicitly
    // guess them so harfrust doesn't panic on Direction::Invalid.
    buffer.guess_segment_properties();

    let shaper_data = ShaperData::new(&font);
    let shaper = shaper_data.shaper(&font).build();
    let glyph_buffer = shaper.shape(buffer, &[]);

    let infos = glyph_buffer.glyph_infos();
    let positions = glyph_buffer.glyph_positions();

    let mut glyphs = Vec::with_capacity(infos.len());
    let mut total_advance = 0.0f32;

    for (info, pos) in infos.iter().zip(positions.iter()) {
        let x_advance = pos.x_advance as f32 * physical_scale * inv_sf;
        let y_advance = pos.y_advance as f32 * physical_scale * inv_sf;
        let x_offset = pos.x_offset as f32 * physical_scale * inv_sf;
        let y_offset = pos.y_offset as f32 * physical_scale * inv_sf;

        glyphs.push(ShapedGlyph {
            glyph_id: info.glyph_id as u16,
            cluster: info.cluster,
            x_advance,
            y_advance,
            x_offset,
            y_offset,
            font_face_id: resolved.font_face_id,
        });

        total_advance += x_advance;
    }

    let run = ShapedRun {
        font_face_id: resolved.font_face_id,
        size_px: resolved.size_px,
        weight: resolved.weight,
        glyphs,
        advance_width: total_advance,
        text_range: text_offset..text_offset + text.len(),
        underline_style: crate::types::UnderlineStyle::None,
        overline: false,
        strikeout: false,
        is_link: false,
        foreground_color: None,
        underline_color: None,
        background_color: None,
        anchor_href: None,
        tooltip: None,
        vertical_alignment: crate::types::VerticalAlignment::Normal,
        image_name: None,
        image_height: 0.0,
    };

    // Reclaim the buffer for reuse
    let recycled = glyph_buffer.clear();
    Some((run, recycled))
}

/// Get font metrics (ascent, descent, leading) scaled to logical pixels.
///
/// Scales at `size_px * scale_factor` (physical) and divides by
/// `scale_factor`, so callers always see logical-pixel metrics.
pub fn font_metrics_px(registry: &FontRegistry, resolved: &ResolvedFont) -> Option<FontMetricsPx> {
    let entry = registry.get(resolved.font_face_id)?;
    let font_ref = swash::FontRef::from_index(entry.bytes(), entry.face_index as usize)?;
    let sf = resolved.scale_factor.max(f32::MIN_POSITIVE);
    let physical_size = resolved.size_px * sf;
    let metrics = font_ref.metrics(&[]).scale(physical_size);
    let inv_sf = 1.0 / sf;

    Some(FontMetricsPx {
        ascent: metrics.ascent * inv_sf,
        descent: metrics.descent * inv_sf,
        leading: metrics.leading * inv_sf,
        underline_offset: metrics.underline_offset * inv_sf,
        strikeout_offset: metrics.strikeout_offset * inv_sf,
        stroke_size: metrics.stroke_size * inv_sf,
    })
}

/// A bidi run: a contiguous range of text with the same direction.
pub struct BidiRun {
    pub byte_range: std::ops::Range<usize>,
    pub direction: TextDirection,
    /// Visual order index (for reordering after line breaking).
    pub visual_order: usize,
}

/// Analyze text for bidirectional content and return directional runs
/// in **visual order** per UAX #9 (Unicode Bidirectional Algorithm, rule L2).
///
/// The returned runs can be shaped independently and concatenated left-to-right
/// to produce correctly-ordered mixed-script text (e.g. Latin embedded in
/// Arabic). For pure-LTR text, returns a single LTR run. For pure-RTL text,
/// returns a single RTL run.
pub fn bidi_runs(text: &str) -> Vec<BidiRun> {
    use unicode_bidi::BidiInfo;

    if text.is_empty() {
        return Vec::new();
    }

    let bidi_info = BidiInfo::new(text, None);
    let mut runs = Vec::new();

    for para in &bidi_info.paragraphs {
        let (levels, level_runs) = bidi_info.visual_runs(para, para.range.clone());
        for level_run in level_runs {
            if level_run.is_empty() {
                continue;
            }
            let level = levels[level_run.start];
            let direction = if level.is_rtl() {
                TextDirection::RightToLeft
            } else {
                TextDirection::LeftToRight
            };
            let visual_order = runs.len();
            runs.push(BidiRun {
                byte_range: level_run,
                direction,
                visual_order,
            });
        }
    }

    if runs.is_empty() {
        runs.push(BidiRun {
            byte_range: 0..text.len(),
            direction: TextDirection::LeftToRight,
            visual_order: 0,
        });
    }

    runs
}

pub struct FontMetricsPx {
    pub ascent: f32,
    pub descent: f32,
    pub leading: f32,
    pub underline_offset: f32,
    pub strikeout_offset: f32,
    pub stroke_size: f32,
}
