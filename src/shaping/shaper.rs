use harfrust::{Direction, Feature, FontRef, ShapeOptions, Tag, UnicodeBuffer};

use crate::font::registry::FontRegistry;
use crate::font::resolve::ResolvedFont;
use crate::shaping::run::{ShapedGlyph, ShapedRun};
use crate::types::FontFeature;

/// Convert public [`FontFeature`] toggles into harfrust [`Feature`]s,
/// applied across the whole shaped string (global range). Script-mandated
/// features apply regardless; these are the discretionary toggles.
pub fn to_harfrust_features(features: &[FontFeature]) -> Vec<Feature> {
    features
        .iter()
        .map(|f| Feature::new(Tag::new(&f.tag), f.value, ..))
        .collect()
}

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
    shape_text_with_fallback(
        registry,
        resolved,
        text,
        text_offset,
        TextDirection::Auto,
        &[],
    )
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
    features: &[Feature],
) -> Option<ShapedRun> {
    let mut run = shape_text_directed(registry, resolved, text, text_offset, direction, features)?;

    // Check for .notdef glyphs and attempt fallback
    if run.glyphs.iter().any(|g| g.glyph_id == 0) && !text.is_empty() {
        apply_glyph_fallback(registry, resolved, text, text_offset, features, &mut run);
    }

    Some(run)
}

/// Re-shape .notdef glyphs using fallback fonts.
///
/// Works on **spans** of consecutive .notdef glyphs, not on glyphs one at a
/// time: each span is mapped back to the character range that produced it and
/// that whole range is re-shaped in the fallback font, then spliced in.
///
/// Shaping a span as a unit is what makes complex scripts survive fallback.
/// Re-shaping character by character — which this used to do — cannot join
/// Arabic, because a letter shaped alone has no neighbours and can only take
/// its isolated form. It also kept only the *first* glyph of each result, so
/// any letter whose isolated form is a base plus a mark lost the mark. An
/// Arabic word set in a font without Arabic came out as disconnected,
/// dotless stumps.
///
/// A span is re-shaped with the run's own direction rather than `Auto`, so
/// the substituted glyphs come back in the same visual order as the ones
/// around them.
fn apply_glyph_fallback(
    registry: &FontRegistry,
    primary: &ResolvedFont,
    text: &str,
    text_offset: usize,
    features: &[Feature],
    run: &mut ShapedRun,
) {
    use crate::font::resolve::find_fallback_font;

    // Maximal spans of consecutive .notdef glyphs, as index ranges.
    let mut spans: Vec<std::ops::Range<usize>> = Vec::new();
    let mut i = 0;
    while i < run.glyphs.len() {
        if run.glyphs[i].glyph_id == 0 {
            let start = i;
            while i < run.glyphs.len() && run.glyphs[i].glyph_id == 0 {
                i += 1;
            }
            spans.push(start..i);
        } else {
            i += 1;
        }
    }
    if spans.is_empty() {
        return;
    }

    // Splice from the back so earlier ranges stay valid as lengths change.
    for span in spans.into_iter().rev() {
        let Some((byte_start, byte_end)) = notdef_char_range(&run.glyphs, &span, text) else {
            continue;
        };
        let Some(slice) = text.get(byte_start..byte_end) else {
            continue;
        };
        let Some(first_char) = slice.chars().next() else {
            continue;
        };

        let Some(fallback_id) = find_fallback_font(registry, first_char, primary.font_face_id)
        else {
            continue; // no fallback available — leave the span as .notdef
        };
        let Some(fallback_entry) = registry.get(fallback_id) else {
            continue;
        };

        let fallback_resolved = ResolvedFont {
            font_face_id: fallback_id,
            size_px: primary.size_px,
            face_index: fallback_entry.face_index,
            swash_cache_key: fallback_entry.swash_cache_key,
            scale_factor: primary.scale_factor,
            weight: primary.weight,
        };

        let Some(fallback_run) = shape_text_directed(
            registry,
            &fallback_resolved,
            slice,
            text_offset + byte_start,
            run.direction,
            features,
        ) else {
            continue;
        };
        if fallback_run.glyphs.is_empty() {
            continue;
        }

        // Clusters come back local to `slice`; lift them into `text` space,
        // which is what the rest of the pipeline expects of this run.
        let replacement: Vec<ShapedGlyph> = fallback_run
            .glyphs
            .into_iter()
            .map(|mut g| {
                g.cluster += byte_start as u32;
                g.font_face_id = fallback_id;
                g
            })
            .collect();

        run.glyphs.splice(span, replacement);
    }

    run.advance_width = run.glyphs.iter().map(|g| g.x_advance).sum();
}

/// The byte range of `text` that a span of consecutive .notdef glyphs covers.
///
/// Glyphs sit in visual order, so clusters ascend across an LTR run and
/// descend across an RTL one. Taking the min and max over the span rather
/// than its first and last glyph keeps this direction-agnostic.
///
/// The span's end is the nearest cluster *after* it among the glyphs outside
/// it — the start of whatever the primary font did manage to shape — or the
/// end of the text when the span runs to the edge.
fn notdef_char_range(
    glyphs: &[ShapedGlyph],
    span: &std::ops::Range<usize>,
    text: &str,
) -> Option<(usize, usize)> {
    let inside = glyphs.get(span.clone())?;
    let start = inside.iter().map(|g| g.cluster as usize).min()?;
    let last = inside.iter().map(|g| g.cluster as usize).max()?;

    let end = glyphs
        .iter()
        .enumerate()
        .filter(|(i, _)| !span.contains(i))
        .map(|(_, g)| g.cluster as usize)
        .filter(|&c| c > last)
        .min()
        .unwrap_or(text.len());

    if start >= end || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
        return None;
    }
    Some((start, end))
}

/// Shape text with an explicit direction.
pub fn shape_text_directed(
    registry: &FontRegistry,
    resolved: &ResolvedFont,
    text: &str,
    text_offset: usize,
    direction: TextDirection,
    features: &[Feature],
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
        TextDirection::Auto => {}
    }
    // Always guess, including after an explicit set_direction. The guess
    // only fills in what is still unset — it assigns `script` when it is
    // `None` and `direction` when it is `Invalid` — so the caller's
    // direction survives untouched and the script gets populated.
    //
    // That script is what picks the shaper: with `script: None` harfrust
    // falls back to DEFAULT_SHAPER, which never requests init/medi/fina/
    // isol, and Arabic comes out in disconnected isolated forms. Setting
    // it selects ARABIC_SHAPER and the letters join. It also keeps
    // harfrust from panicking on a still-Invalid direction in the Auto
    // case, which is why this call used to live in that branch alone.
    buffer.guess_segment_properties();

    // Resolve the concrete direction (Auto is now decided by the buffer).
    // Stored on the run so hit-testing knows RTL glyph order.
    let resolved_direction = if buffer.direction() == Direction::RightToLeft {
        TextDirection::RightToLeft
    } else {
        TextDirection::LeftToRight
    };

    // ShaperData preprocesses font tables for shaping. It's built once
    // per face and cached on the FontEntry, so repeated shape calls
    // (every relayout/keystroke) reuse the same preprocessed tables.
    let shaper_data = entry.shaper_data(&font);
    let shaper = shaper_data.shaper(&font).build();
    let glyph_buffer = shaper.shape(buffer, ShapeOptions::new().features(features));

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
        direction: resolved_direction,
        bidi_level: if resolved_direction == TextDirection::RightToLeft {
            1
        } else {
            0
        },
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
    features: &[Feature],
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

    let resolved_direction = if buffer.direction() == Direction::RightToLeft {
        TextDirection::RightToLeft
    } else {
        TextDirection::LeftToRight
    };

    let shaper_data = entry.shaper_data(&font);
    let shaper = shaper_data.shaper(&font).build();
    let glyph_buffer = shaper.shape(buffer, ShapeOptions::new().features(features));

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
        direction: resolved_direction,
        bidi_level: if resolved_direction == TextDirection::RightToLeft {
            1
        } else {
            0
        },
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
    /// UAX #9 embedding level: even is LTR, odd is RTL.
    ///
    /// Rule L2 reorders by level, not by direction, and the two are not
    /// interchangeable: a Latin phrase quoted inside Arabic inside an
    /// English paragraph sits at level 2, and reordering it as though it
    /// were level 0 puts it on the wrong side of the Arabic. `direction`
    /// cannot tell those apart — `level` can.
    pub level: u8,
}

/// A paragraph's bidi structure in **logical** order.
///
/// [`bidi_runs`] returns runs already reordered for display, which suits
/// a single-line label that is shaped and painted in one go. The
/// multi-line editor cannot use that: line breaking is a logical
/// operation, so runs have to stay in logical order until the breaker has
/// decided where the lines fall, and only then get reordered *per line*.
/// Reordering the paragraph up front and breaking afterwards would
/// scramble any paragraph that wraps.
pub struct BidiParagraph {
    /// Runs of uniform embedding level, in logical order.
    pub runs: Vec<BidiRun>,
    /// The paragraph embedding level actually used — the explicit base
    /// direction when one was given, else the rule P2/P3 auto-detection.
    pub para_level: u8,
}

impl BidiParagraph {
    /// The paragraph's base direction, as resolved.
    pub fn base_direction(&self) -> TextDirection {
        if self.para_level % 2 == 1 {
            TextDirection::RightToLeft
        } else {
            TextDirection::LeftToRight
        }
    }
}

/// The unicode-bidi paragraph level that expresses `base`.
///
/// `None` asks unicode-bidi to auto-detect (rules P2/P3: the first strong
/// character wins, defaulting to LTR when the text has none). An explicit
/// level overrides that — which is the point of honouring a stored
/// paragraph direction, since P2/P3 mis-detects any RTL paragraph that
/// happens to open with a digit, a Latin acronym or an opening quote.
fn base_para_level(base: TextDirection) -> Option<unicode_bidi::Level> {
    match base {
        TextDirection::Auto => None,
        TextDirection::LeftToRight => Some(unicode_bidi::Level::ltr()),
        TextDirection::RightToLeft => Some(unicode_bidi::Level::rtl()),
    }
}

/// Resolve `text`'s bidi structure under an explicit or auto base direction.
///
/// Runs come back in logical order, each tagged with its embedding level.
/// Callers shape each run with its own direction and later reorder the
/// runs of each line with [`visual_order`].
pub fn analyze_paragraph(text: &str, base: TextDirection) -> BidiParagraph {
    use unicode_bidi::BidiInfo;

    if text.is_empty() {
        return BidiParagraph {
            runs: Vec::new(),
            para_level: base_para_level(base).map_or(0, |l| l.number()),
        };
    }

    let bidi_info = BidiInfo::new(text, base_para_level(base));

    // A block is one paragraph as far as layout is concerned; if the text
    // somehow carries a hard break, the first paragraph's level is the
    // one that governs alignment.
    let para_level = bidi_info
        .paragraphs
        .first()
        .map(|p| p.level.number())
        .or_else(|| base_para_level(base).map(|l| l.number()))
        .unwrap_or(0);

    // Group consecutive characters of equal level. Iterating by
    // `char_indices` rather than by byte keeps a run boundary from ever
    // landing inside a multi-byte character.
    let mut starts: Vec<(usize, u8)> = Vec::new();
    for (idx, _) in text.char_indices() {
        let level = bidi_info.levels[idx].number();
        if starts.last().map(|&(_, l)| l) != Some(level) {
            starts.push((idx, level));
        }
    }

    // Each run ends where the next begins; the last ends with the text.
    let ends = starts
        .iter()
        .skip(1)
        .map(|&(start, _)| start)
        .chain(std::iter::once(text.len()));

    let mut runs: Vec<BidiRun> = starts
        .iter()
        .zip(ends)
        .map(|(&(start, level), end)| BidiRun {
            byte_range: start..end,
            direction: if level % 2 == 1 {
                TextDirection::RightToLeft
            } else {
                TextDirection::LeftToRight
            },
            // Always 0 here: these runs are in *logical* order and the
            // block path reorders per line after breaking, so a
            // paragraph-wide visual index would be both unused and
            // misleading. `bidi_runs` fills it in for the single-line
            // path, which does lay a paragraph out in one go.
            visual_order: 0,
            level,
        })
        .collect();

    BidiParagraph { runs, para_level }
}

/// Reorder items from logical into visual order per UAX #9 rule L2.
///
/// Returns indices into `levels`: `result[0]` is the item to paint
/// leftmost. L2 reads "from the highest level down to the lowest odd
/// level, reverse any contiguous sequence of items at that level or
/// higher", which is what the loop below does literally.
pub fn visual_order(levels: &[u8]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..levels.len()).collect();
    let Some(&max) = levels.iter().max() else {
        return order;
    };
    // With no odd level nothing is RTL, so logical order is already visual.
    let Some(min_odd) = levels.iter().copied().filter(|l| l % 2 == 1).min() else {
        return order;
    };

    let mut level = max;
    while level >= min_odd {
        let mut i = 0;
        while i < order.len() {
            if levels[order[i]] >= level {
                let start = i;
                while i < order.len() && levels[order[i]] >= level {
                    i += 1;
                }
                order[start..i].reverse();
            } else {
                i += 1;
            }
        }
        // min_odd is odd, hence >= 1, so this cannot wrap past zero.
        level -= 1;
    }
    order
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
                level: level.number(),
            });
        }
    }

    if runs.is_empty() {
        runs.push(BidiRun {
            byte_range: 0..text.len(),
            direction: TextDirection::LeftToRight,
            visual_order: 0,
            level: 0,
        });
    }

    runs
}

#[cfg(test)]
mod bidi_tests {
    use super::*;

    const ARABIC: &str = "\u{0643}\u{062A}\u{0628}"; // كتب
    const HEBREW: &str = "\u{05E9}\u{05DC}\u{05D5}\u{05DD}"; // שלום

    #[test]
    fn rule_l2_leaves_all_ltr_text_alone() {
        assert_eq!(visual_order(&[0, 0, 0]), vec![0, 1, 2]);
        assert_eq!(visual_order(&[]), Vec::<usize>::new());
    }

    #[test]
    fn rule_l2_reverses_a_run_of_rtl() {
        // level 0 "a", level 1 RTL, level 0 "b" -> the RTL span reverses
        // in place but stays between its neighbours.
        assert_eq!(visual_order(&[0, 1, 1, 0]), vec![0, 2, 1, 3]);
    }

    #[test]
    fn rule_l2_nests_an_ltr_island_inside_rtl() {
        // An English phrase (level 2) quoted inside Arabic (level 1)
        // inside an English paragraph (level 0). The level-2 island must
        // keep its own left-to-right order while the level-1 material
        // around it reverses — the case `direction` alone cannot express.
        let levels = [0, 1, 2, 2, 1, 0];
        assert_eq!(visual_order(&levels), vec![0, 4, 2, 3, 1, 5]);
    }

    #[test]
    fn rule_l2_reverses_the_whole_line_in_an_rtl_paragraph() {
        // Level 1 throughout with a level-2 Latin word: the paragraph
        // reads right-to-left, so logical item 0 paints rightmost.
        assert_eq!(visual_order(&[1, 2, 1]), vec![2, 1, 0]);
    }

    #[test]
    fn a_leading_digit_does_not_fool_auto_detection() {
        // Worth pinning, because it is easy to assume otherwise: digits
        // are type EN, not strong, so rule P2 skips them and finds the
        // Arabic. An Arabic paragraph opening with a number needs no
        // explicit direction.
        let text = "123 \u{0643}\u{062A}\u{0628}";
        assert_eq!(analyze_paragraph(text, TextDirection::Auto).para_level, 1);
    }

    #[test]
    fn an_explicit_base_direction_overrides_first_strong_detection() {
        // A Latin acronym *is* strong (type L), so rule P2 stops at the
        // "NASA" and calls this Arabic paragraph left-to-right — the
        // real mis-detection, and the reason a stored paragraph
        // direction has to win over guessing.
        let text = "NASA \u{0623}\u{0639}\u{0644}\u{0646}\u{062A}";
        assert_eq!(
            analyze_paragraph(text, TextDirection::Auto).para_level,
            0,
            "auto-detection is expected to get this one wrong"
        );

        let forced = analyze_paragraph(text, TextDirection::RightToLeft);
        assert_eq!(forced.para_level, 1);
        assert_eq!(forced.base_direction(), TextDirection::RightToLeft);

        // And the override genuinely changes the layout, not just the
        // recorded level: the Arabic now leads visually. Runs come back
        // in logical order, so ask rule L2 which one paints leftmost —
        // the same thing a caller laying the paragraph out would do.
        let auto = analyze_paragraph(text, TextDirection::Auto);
        let first_visual = |p: &BidiParagraph| {
            let levels: Vec<u8> = p.runs.iter().map(|r| r.level).collect();
            visual_order(&levels).first().map(|&i| p.runs[i].direction)
        };
        assert_eq!(first_visual(&auto), Some(TextDirection::LeftToRight));
        assert_eq!(first_visual(&forced), Some(TextDirection::RightToLeft));
    }

    #[test]
    fn pure_arabic_auto_detects_as_rtl() {
        let para = analyze_paragraph(ARABIC, TextDirection::Auto);
        assert_eq!(para.base_direction(), TextDirection::RightToLeft);
        assert_eq!(para.runs.len(), 1);
        assert_eq!(para.runs[0].direction, TextDirection::RightToLeft);
        assert_eq!(para.runs[0].byte_range, 0..ARABIC.len());
    }

    #[test]
    fn runs_come_back_in_logical_order_and_cover_the_text() {
        // "hello <hebrew> world" — three runs, contiguous, logical order.
        let text = format!("hello {HEBREW} world");
        let para = analyze_paragraph(&text, TextDirection::Auto);

        assert!(para.runs.len() >= 2, "expected a directional split");
        assert_eq!(para.runs[0].byte_range.start, 0);
        assert_eq!(para.runs.last().unwrap().byte_range.end, text.len());
        for pair in para.runs.windows(2) {
            assert_eq!(
                pair[0].byte_range.end, pair[1].byte_range.start,
                "runs must tile the text with no gap or overlap"
            );
            assert!(
                pair[0].byte_range.start < pair[1].byte_range.start,
                "runs must be in logical order"
            );
        }
        assert!(
            para.runs
                .iter()
                .any(|r| r.direction == TextDirection::RightToLeft),
            "the Hebrew span should have produced an RTL run"
        );
    }

    #[test]
    fn run_boundaries_never_split_a_multibyte_character() {
        let text = format!("a{ARABIC}b{HEBREW}c");
        for run in analyze_paragraph(&text, TextDirection::Auto).runs {
            assert!(
                text.is_char_boundary(run.byte_range.start)
                    && text.is_char_boundary(run.byte_range.end),
                "run {:?} splits a character in {text:?}",
                run.byte_range
            );
        }
    }

    #[test]
    fn rule_l2_over_a_real_paragraph_is_a_permutation() {
        let text = format!("hello {HEBREW} world {ARABIC} end");
        let para = analyze_paragraph(&text, TextDirection::Auto);
        let levels: Vec<u8> = para.runs.iter().map(|r| r.level).collect();

        let mut seen = visual_order(&levels);
        seen.sort_unstable();
        assert_eq!(
            seen,
            (0..para.runs.len()).collect::<Vec<_>>(),
            "every run must appear exactly once in the visual order"
        );
    }

    #[test]
    fn empty_text_analyzes_without_panicking() {
        let para = analyze_paragraph("", TextDirection::RightToLeft);
        assert!(para.runs.is_empty());
        assert_eq!(para.para_level, 1);
    }
}

pub struct FontMetricsPx {
    pub ascent: f32,
    pub descent: f32,
    pub leading: f32,
    pub underline_offset: f32,
    pub strikeout_offset: f32,
    pub stroke_size: f32,
}
