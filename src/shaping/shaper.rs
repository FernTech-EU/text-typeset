use harfrust::{Buffer, Direction, Feature, ShapeOptions, ShaperFont, Tag};

use crate::font::color::{GlyphPaint, glyph_paint};
use crate::font::registry::FontRegistry;
use crate::font::resolve::{
    FaceStyle, Presentation, ResolvedFont, face_covers, find_fallback_face,
};
use crate::font::script_fallback::{ScriptContext, preferred_families};
use crate::font::synthesis;
use crate::shaping::emoji;
use crate::shaping::run::{ShapedGlyph, ShapedRun};
use crate::types::FontFaceId;
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
    shape_text_in_paragraph(
        registry,
        resolved,
        text,
        text_offset,
        direction,
        features,
        &ScriptContext::new(text, None),
        0,
    )
}

/// [`shape_text_with_fallback`] for text that is part of a paragraph: the
/// paragraph and the language its block is in (`context`), and the byte
/// where `text` starts in the paragraph (`para_pos`), decide which faces
/// fallback prefers for Han (see [`crate::font::script_fallback`]).
#[allow(clippy::too_many_arguments)]
pub(crate) fn shape_text_in_paragraph(
    registry: &FontRegistry,
    resolved: &ResolvedFont,
    text: &str,
    text_offset: usize,
    direction: TextDirection,
    features: &[Feature],
    context: &ScriptContext<'_>,
    para_pos: usize,
) -> Option<ShapedRun> {
    let mut run = shape_text_directed(registry, resolved, text, text_offset, direction, features)?;
    blank_hard_breaks(text, &mut run);

    // Look for another face for every glyph the primary font lacks (a hard
    // break is glyph 0 on purpose and needs none) and every emoji it does not
    // draw in colour.
    let may_need = run.glyphs.iter().any(|g| g.glyph_id == 0) || emoji::may_contain_emoji(text);
    let needs = if may_need {
        fallback_needs(registry, text, &run)
    } else {
        Vec::new()
    };
    if needs.iter().any(|n| n.needed) {
        let style = FaceStyle::of_face(registry, resolved.font_face_id, resolved.weight);
        apply_glyph_fallback(
            registry,
            resolved,
            style,
            text,
            text_offset,
            features,
            &mut run,
            &needs,
            &[resolved.font_face_id],
            context,
            para_pos,
            0,
        );
        // A break beside a character the primary font lacks is re-shaped with
        // it, in a font that may well draw it.
        blank_hard_breaks(text, &mut run);
    }

    Some(run)
}

/// Whether `glyph` stands for a hard line break of `text`, the text its run was shaped
/// from.
fn is_hard_break_glyph(text: &str, glyph: &ShapedGlyph) -> bool {
    text.get(glyph.cluster as usize..)
        .and_then(|rest| rest.chars().next())
        .is_some_and(crate::types::is_hard_break)
}

/// Lay out every hard line break in `run` as nothing: glyph 0, which is never
/// drawn, with no advance and no offset (see [`crate::types::is_hard_break`]).
/// Returns how many there are.
///
/// A font draws a line separator as it likes (a box, a pilcrow) or not at all,
/// and gives it whatever advance it likes, which pushed a right-aligned or
/// centred line off its place by that much.
fn blank_hard_breaks(text: &str, run: &mut ShapedRun) -> usize {
    let mut count = 0;
    for glyph in &mut run.glyphs {
        if is_hard_break_glyph(text, glyph) {
            glyph.glyph_id = 0;
            glyph.x_advance = 0.0;
            glyph.x_offset = 0.0;
            glyph.y_offset = 0.0;
            count += 1;
        }
    }
    if count > 0 {
        run.advance_width = run.glyphs.iter().map(|g| g.x_advance).sum();
    }
    count
}

/// How deep fallback goes: the part of a span its fallback face does not
/// draw is handed to another face, and what that one cannot draw to another,
/// each time with every face tried so far left out. A span of one script
/// takes one face and a mixed one (Han then Hangul, say) a second; the cap
/// only bounds a pathological font set.
const MAX_FALLBACK_DEPTH: usize = 6;

/// What fallback makes of one glyph of a shaped run.
#[derive(Debug, Clone, Copy)]
struct FallbackNeed {
    /// Whether another face is looked for.
    needed: bool,
    /// Whether the glyph has no glyph at all (`.notdef`), as against one that
    /// is drawn but not as it should be (an emoji drawn in monochrome).
    missing: bool,
    /// Whether the glyph's cluster asks for emoji presentation.
    emoji: bool,
}

/// Decide, glyph by glyph, which glyphs of `run` (shaped from `text`) want
/// another face: those with no glyph, and those of an emoji cluster drawn
/// in anything but colour. A hard break needs none.
fn fallback_needs(registry: &FontRegistry, text: &str, run: &ShapedRun) -> Vec<FallbackNeed> {
    let emoji = emoji_clusters(text, &run.glyphs);
    run.glyphs
        .iter()
        .zip(emoji)
        .map(|(glyph, emoji)| {
            if is_hard_break_glyph(text, glyph) {
                return FallbackNeed {
                    needed: false,
                    missing: false,
                    emoji,
                };
            }
            let missing = glyph.glyph_id == 0;
            let not_in_color = emoji
                && !missing
                && registry
                    .get(glyph.font_face_id)
                    .is_some_and(|entry| glyph_paint(entry, glyph.glyph_id) != GlyphPaint::Color);
            FallbackNeed {
                needed: missing || not_in_color,
                missing,
                emoji,
            }
        })
        .collect()
}

/// For each glyph of `glyphs` (shaped from `text`), whether its cluster asks
/// for emoji presentation (see [`crate::shaping::emoji`]).
fn emoji_clusters(text: &str, glyphs: &[ShapedGlyph]) -> Vec<bool> {
    if !emoji::may_contain_emoji(text) {
        return vec![false; glyphs.len()];
    }
    // A cluster runs from its own start to the next start in the text, which
    // works for an RTL run too, whose glyphs carry descending clusters.
    let mut starts: Vec<usize> = glyphs.iter().map(|g| g.cluster as usize).collect();
    starts.sort_unstable();
    starts.dedup();
    glyphs
        .iter()
        .map(|g| {
            let start = g.cluster as usize;
            let end = match starts.binary_search(&start) {
                Ok(i) => starts.get(i + 1).copied().unwrap_or(text.len()),
                Err(_) => text.len(),
            };
            text.get(start..end).is_some_and(emoji::cluster_wants_emoji)
        })
        .collect()
}

/// Re-shape the glyphs of `run` that need another face in fallback fonts.
///
/// Works on **spans** of consecutive glyphs that need one, not on glyphs one
/// at a time: each span is mapped back to the character range that produced
/// it and that whole range is re-shaped in the fallback font, then spliced
/// in.
///
/// Shaping a span as a unit is what makes complex scripts survive fallback.
/// Re-shaping character by character, which this used to do, cannot join
/// Arabic, because a letter shaped alone has no neighbours and can only take
/// its isolated form. It also kept only the *first* glyph of each result, so
/// any letter whose isolated form is a base plus a mark lost the mark. An
/// Arabic word set in a font without Arabic came out as disconnected,
/// dotless stumps.
///
/// A span is cut where emoji presentation starts or stops, so that an emoji
/// after CJK text gets a face of its own rather than the CJK font's
/// `.notdef`. The span is re-shaped in the face found for its first
/// character, and whatever that face does not draw goes through the same
/// steps again, with every face tried so far left out: before, a span was
/// shaped in one face and what that face lacked stayed blank. When the face
/// found for the first character a span's face lacks draws the whole span,
/// the span is set in that face instead, so that a word is not split
/// between two CJK fonts of different designs.
///
/// Every face is chosen to match `style`, the weight, style and width of the
/// text (see [`find_fallback_face`]), and for kana, hangul and Han among the
/// faces the system ships for their language first, which for Han is found
/// in `context`, the paragraph `text` starts at byte `para_pos` of (see
/// [`crate::font::script_fallback`]).
///
/// A span is re-shaped with the run's own direction rather than `Auto`, so
/// the substituted glyphs come back in the same visual order as the ones
/// around them.
#[allow(clippy::too_many_arguments)]
fn apply_glyph_fallback(
    registry: &FontRegistry,
    primary: &ResolvedFont,
    style: FaceStyle,
    text: &str,
    text_offset: usize,
    features: &[Feature],
    run: &mut ShapedRun,
    needs: &[FallbackNeed],
    tried: &[FontFaceId],
    context: &ScriptContext<'_>,
    para_pos: usize,
    depth: usize,
) {
    // The faces the fallback table prefers for the character at byte `at` of
    // `text`.
    let preferred = |ch: char, at: usize| {
        preferred_families(
            ch,
            context,
            para_pos + at,
            registry.default_language(),
            registry.fallback_platform(),
        )
    };
    // Maximal spans of consecutive glyphs that need a face, cut where emoji
    // presentation changes. A hard break needs none and so ends a span:
    // inside one it would choose the span's font, since a span is re-shaped
    // in the font that draws its first character, and a character beside it
    // would lose its own.
    let mut spans: Vec<(std::ops::Range<usize>, bool)> = Vec::new();
    let mut i = 0;
    while i < needs.len() {
        if !needs[i].needed {
            i += 1;
            continue;
        }
        let start = i;
        let emoji = needs[i].emoji;
        while i < needs.len() && needs[i].needed && needs[i].emoji == emoji {
            i += 1;
        }
        spans.push((start..i, emoji));
    }
    if spans.is_empty() {
        return;
    }

    // Splice from the back so earlier ranges stay valid as lengths change.
    for (span, emoji) in spans.into_iter().rev() {
        let any_missing = needs[span.clone()].iter().any(|n| n.missing);
        let Some((byte_start, byte_end)) = span_char_range(&run.glyphs, &span, text) else {
            continue;
        };
        let Some(slice) = text.get(byte_start..byte_end) else {
            continue;
        };
        let Some(first_char) = slice.chars().next() else {
            continue;
        };

        // An emoji drawn in monochrome is left out with the faces tried
        // already, so the search is for a face that draws it otherwise.
        let mut excluded: Vec<FontFaceId> = tried.to_vec();
        for glyph in &run.glyphs[span.clone()] {
            if !excluded.contains(&glyph.font_face_id) {
                excluded.push(glyph.font_face_id);
            }
        }
        let presentation = if emoji {
            Presentation::Emoji
        } else {
            Presentation::Text
        };
        let Some((mut fallback_id, paint)) = find_fallback_face(
            registry,
            first_char,
            presentation,
            style,
            &excluded,
            preferred(first_char, byte_start),
        ) else {
            continue; // no fallback available: leave the span as it is
        };
        // An emoji already drawn, if in monochrome, is only worth replacing
        // by one drawn in colour.
        if emoji && !any_missing && paint != GlyphPaint::Color {
            continue;
        }

        let reshape = |face: FontFaceId| -> Option<(ResolvedFont, ShapedRun)> {
            let entry = registry.get(face)?;
            let resolved = ResolvedFont {
                font_face_id: face,
                size_px: primary.size_px,
                face_index: entry.face_index,
                swash_cache_key: entry.swash_cache_key,
                scale_factor: primary.scale_factor,
                weight: primary.weight,
                italic: primary.italic,
            };
            // Clusters come back local to `slice`.
            let run = shape_text_directed(
                registry,
                &resolved,
                slice,
                text_offset + byte_start,
                run.direction,
                features,
            )?;
            (!run.glyphs.is_empty()).then_some((resolved, run))
        };
        let Some((mut fallback_resolved, mut fallback_run)) = reshape(fallback_id) else {
            continue;
        };
        blank_hard_breaks(slice, &mut fallback_run);

        excluded.push(fallback_id);
        let mut still = fallback_needs(registry, slice, &fallback_run);
        if !emoji
            && let Some((at, lacking)) = fallback_run
                .glyphs
                .iter()
                .zip(&still)
                .find(|(_, n)| n.missing)
                .and_then(|(g, _)| {
                    let at = g.cluster as usize;
                    Some((at, slice.get(at..)?.chars().next()?))
                })
            && let Some((whole_id, _)) = find_fallback_face(
                registry,
                lacking,
                Presentation::Text,
                style,
                &excluded,
                preferred(lacking, byte_start + at),
            )
            && face_covers(registry, whole_id, slice)
            && let Some((resolved, mut whole_run)) = reshape(whole_id)
        {
            blank_hard_breaks(slice, &mut whole_run);
            fallback_id = whole_id;
            fallback_resolved = resolved;
            fallback_run = whole_run;
            excluded.push(fallback_id);
            still = fallback_needs(registry, slice, &fallback_run);
        }

        if depth < MAX_FALLBACK_DEPTH && still.iter().any(|n| n.needed) {
            apply_glyph_fallback(
                registry,
                &fallback_resolved,
                style,
                slice,
                text_offset + byte_start,
                features,
                &mut fallback_run,
                &still,
                &excluded,
                context,
                para_pos + byte_start,
                depth + 1,
            );
        }

        // Lift the clusters from `slice` into `text` space, which is what the
        // rest of the pipeline expects of this run. Each glyph keeps the face
        // it was shaped in, which deeper fallback may have changed.
        let replacement: Vec<ShapedGlyph> = fallback_run
            .glyphs
            .into_iter()
            .map(|mut g| {
                g.cluster += byte_start as u32;
                g
            })
            .collect();

        run.glyphs.splice(span, replacement);
    }

    run.advance_width = run.glyphs.iter().map(|g| g.x_advance).sum();
}

/// The byte range of `text` that a span of consecutive glyphs covers.
///
/// Glyphs sit in visual order, so clusters ascend across an LTR run and
/// descend across an RTL one. Taking the min and max over the span rather
/// than its first and last glyph keeps this direction-agnostic.
///
/// The span's end is the nearest cluster *after* it among the glyphs outside
/// it (the start of whatever follows it in the text), or the end of the text
/// when the span runs to the edge.
fn span_char_range(
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
    let font = entry.shaping_font()?;

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

    let mut buffer = Buffer::new();
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

    // The font is opened once per face and cached on the FontEntry, and
    // it keeps its preprocessed tables, so repeated shape calls (every
    // relayout/keystroke) reuse them; the ShaperFont over it is cheap.
    harfrust::shape(
        &ShaperFont::new(font),
        &mut buffer,
        ShapeOptions::new().features(features),
    )
    .ok()?;

    let infos = buffer.glyph_infos();
    let positions = buffer.glyph_positions();

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
    if entry.synthesis().synthesizes_bold(resolved.weight) {
        total_advance = widen_for_synthetic_bold(&mut glyphs, resolved.size_px);
    }

    Some(ShapedRun {
        font_face_id: resolved.font_face_id,
        size_px: resolved.size_px,
        weight: resolved.weight,
        italic: resolved.italic,
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

/// Shape a text string, reusing a harfrust `Buffer` to avoid allocations.
pub fn shape_text_with_buffer(
    registry: &FontRegistry,
    resolved: &ResolvedFont,
    text: &str,
    text_offset: usize,
    buffer: Buffer,
    features: &[Feature],
) -> Option<(ShapedRun, Buffer)> {
    let entry = registry.get(resolved.font_face_id)?;
    let font = entry.shaping_font()?;

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

    harfrust::shape(
        &ShaperFont::new(font),
        &mut buffer,
        ShapeOptions::new().features(features),
    )
    .ok()?;

    let infos = buffer.glyph_infos();
    let positions = buffer.glyph_positions();

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
    if entry.synthesis().synthesizes_bold(resolved.weight) {
        total_advance = widen_for_synthetic_bold(&mut glyphs, resolved.size_px);
    }

    let run = ShapedRun {
        font_face_id: resolved.font_face_id,
        size_px: resolved.size_px,
        weight: resolved.weight,
        italic: resolved.italic,
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
    let mut run = run;
    blank_hard_breaks(text, &mut run);

    // Reclaim the buffer for reuse: clearing keeps the allocation and
    // drops the contents and segment properties.
    buffer.clear();
    Some((run, buffer))
}

/// Widen every advancing glyph by what synthetic bold adds to its outline
/// (see [`crate::font::synthesis`]), so the layout, the caret and hit testing
/// measure the glyph as the rasterizer draws it. Returns the run's new
/// advance.
fn widen_for_synthetic_bold(glyphs: &mut [ShapedGlyph], size_px: f32) -> f32 {
    let extra = synthesis::embolden_strength(size_px);
    for glyph in glyphs.iter_mut().filter(|g| g.x_advance > 0.0) {
        glyph.x_advance += extra;
    }
    glyphs.iter().map(|g| g.x_advance).sum()
}

#[non_exhaustive]
pub struct FontMetricsPx {
    pub ascent: f32,
    pub descent: f32,
    pub leading: f32,
    pub underline_offset: f32,
    pub strikeout_offset: f32,
    pub stroke_size: f32,
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
#[non_exhaustive]
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
#[non_exhaustive]
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
/// paragraph direction, since P2/P3 incorrectly classifies any RTL
/// paragraph that happens to open with a digit, a Latin acronym or an
/// opening quote.
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

    let runs: Vec<BidiRun> = starts
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
        // real false detection, and the reason a stored paragraph
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
