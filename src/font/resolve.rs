use crate::font::registry::FontRegistry;
use crate::types::FontFaceId;

/// A resolved font face with all parameters needed for shaping and rasterization.
pub struct ResolvedFont {
    pub font_face_id: FontFaceId,
    pub size_px: f32,
    pub face_index: u32,
    pub swash_cache_key: swash::CacheKey,
    /// Device pixel ratio applied during shaping and rasterization.
    /// Shaping happens at `size_px * scale_factor` and results are
    /// divided by `scale_factor` to produce logical-pixel metrics,
    /// so downstream layout stays in logical space.
    pub scale_factor: f32,
    /// Resolved font weight (CSS-style 100–900). Used as the `wght`
    /// variation axis when rasterizing variable fonts and as part of
    /// the glyph cache key so different weights produce separate
    /// rasterized bitmaps.
    pub weight: u16,
}

/// Resolve a font from text formatting parameters.
///
/// The resolution order is:
/// 1. If font_family is set, query by family name (with generic mapping)
/// 2. Apply font_weight (or font_bold as weight 700)
/// 3. Apply font_italic
/// 4. Fall back to the default font if no match
pub fn resolve_font(
    registry: &FontRegistry,
    font_family: Option<&str>,
    font_weight: Option<u32>,
    font_bold: Option<bool>,
    font_italic: Option<bool>,
    font_point_size: Option<u32>,
    scale_factor: f32,
) -> Option<ResolvedFont> {
    let weight = resolve_weight(font_weight, font_bold);
    let italic = font_italic.unwrap_or(false);
    let size_px = font_point_size
        .map(|s| s as f32)
        .unwrap_or(registry.default_size_px());

    // Try the specified family first
    if let Some(family) = font_family
        && let Some(face_id) = registry.query_font(family, weight, italic)
    {
        let entry = registry.get(face_id)?;
        return Some(ResolvedFont {
            font_face_id: face_id,
            size_px,
            face_index: entry.face_index,
            swash_cache_key: entry.swash_cache_key,
            scale_factor,
            weight,
        });
    }

    // Fall back to default font, but still try to match weight/italic
    // by querying for a variant of the default font's family.
    let default_id = registry.default_font()?;
    if (weight != 400 || italic)
        && let Some(variant_id) = registry.query_variant(default_id, weight, italic)
    {
        let variant_entry = registry.get(variant_id)?;
        return Some(ResolvedFont {
            font_face_id: variant_id,
            size_px,
            face_index: variant_entry.face_index,
            swash_cache_key: variant_entry.swash_cache_key,
            scale_factor,
            weight,
        });
    }
    let entry = registry.get(default_id)?;
    Some(ResolvedFont {
        font_face_id: default_id,
        size_px,
        face_index: entry.face_index,
        swash_cache_key: entry.swash_cache_key,
        scale_factor,
        weight,
    })
}

/// Check if a font has a glyph for the given character.
/// Used for glyph fallback -trying other registered fonts when
/// the primary font doesn't cover a character.
pub fn font_has_glyph(registry: &FontRegistry, face_id: FontFaceId, ch: char) -> bool {
    let entry = match registry.get(face_id) {
        Some(e) => e,
        None => return false,
    };
    let font_ref = match swash::FontRef::from_index(entry.bytes(), entry.face_index as usize) {
        Some(f) => f,
        None => return false,
    };
    font_ref.charmap().map(ch) != 0
}

/// Find a fallback font that has the given character.
///
/// Explicitly-registered faces are tried before OS-discovered ones, so a
/// host's bundled fallback fonts win over arbitrary system fonts; system
/// fonts (whose bytes load lazily on this scan) are the last resort.
pub fn find_fallback_font(
    registry: &FontRegistry,
    ch: char,
    exclude: FontFaceId,
) -> Option<FontFaceId> {
    let covers = |entry: &crate::font::registry::FontEntry| {
        swash::FontRef::from_index(entry.bytes(), entry.face_index as usize)
            .is_some_and(|font_ref| font_ref.charmap().map(ch) != 0)
    };

    // Pass 1: explicitly-registered faces. Pass 2: system faces.
    for system_pass in [false, true] {
        for (face_id, entry) in registry.all_entries() {
            if face_id == exclude || entry.is_system != system_pass {
                continue;
            }
            if covers(entry) {
                return Some(face_id);
            }
        }
    }
    None
}

/// Convert TextFormat weight fields to a u16 weight value for fontdb.
fn resolve_weight(font_weight: Option<u32>, font_bold: Option<bool>) -> u16 {
    if let Some(w) = font_weight {
        return w.min(1000) as u16;
    }
    if font_bold == Some(true) {
        return 700;
    }
    400 // Normal weight
}
