use crate::font::color::{GlyphPaint, glyph_paint};
use crate::font::registry::FontRegistry;
use crate::font::script_fallback::{ScriptContext, preferred_families};
use crate::types::FontFaceId;

/// A resolved font face with all parameters needed for shaping and rasterization.
#[non_exhaustive]
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
    /// Whether the text asks for italic. A face that has no italic draws it
    /// with a synthetic slant (see `crate::font::synthesis`); the glyph cache
    /// keys on it.
    pub italic: bool,
}

/// Resolve a font from text formatting parameters.
///
/// The resolution order is:
/// 1. If font_family is set, query by family name (with generic mapping)
/// 2. Apply font_weight (or font_bold as weight 700)
/// 3. Apply font_italic
/// 4. Fall back to the default font if no match
#[allow(clippy::too_many_arguments)]
pub fn resolve_font(
    registry: &FontRegistry,
    font_family: Option<&str>,
    font_weight: Option<u32>,
    font_bold: Option<bool>,
    font_italic: Option<bool>,
    font_point_size: Option<u32>,
    scale_factor: f32,
    font_scale: f32,
) -> Option<ResolvedFont> {
    let weight = resolve_weight(font_weight, font_bold);
    let italic = font_italic.unwrap_or(false);
    // `font_scale` is the per-document logical text-magnification factor
    // (accessibility "grow all text"), distinct from `scale_factor` (HiDPI
    // raster density). It multiplies the logical point size so advances, line
    // heights, and content height all grow and the text reflows correctly.
    let size_px = font_point_size
        .map(|s| s as f32)
        .unwrap_or(registry.default_size_px())
        * font_scale;

    // Try the specified family first.
    if let Some(family) = font_family {
        if let Some(face_id) = registry.query_font(family, weight, italic) {
            let entry = registry.get(face_id)?;
            return Some(ResolvedFont {
                font_face_id: face_id,
                size_px,
                face_index: entry.face_index,
                swash_cache_key: entry.swash_cache_key,
                scale_factor,
                weight,
                italic,
            });
        }
        // A family was explicitly requested but is not registered. Warn once
        // (per family, per process) so the silent fall-back to the default
        // font is visible — a theme that sets `family = "Roboto"` without
        // registering Roboto would otherwise just render the default with no
        // signal. This is the only place that warns; the lower-level
        // `query_font` is also used for glyph-fallback probing, which misses
        // by design and must stay quiet.
        warn_unknown_family(family);
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
            italic,
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
        italic,
    })
}

/// Warn once per process per family when a requested font family is not
/// registered, so the silent fall-back to the default font is visible.
fn warn_unknown_family(family: &str) {
    use std::collections::HashSet;
    use std::sync::{LazyLock, Mutex};
    static WARNED: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));
    if let Ok(mut warned) = WARNED.lock()
        && warned.insert(family.to_string())
    {
        eprintln!(
            "text-typeset: font family {family:?} is not registered; falling back \
             to the default font. Register it (e.g. via the host application's font \
             registrar) before shaping."
        );
    }
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
    let weight = registry
        .get(exclude)
        .and_then(|entry| registry.fontdb_weight(entry.fontdb_id))
        .unwrap_or(400);
    let style = FaceStyle::of_face(registry, exclude, weight);
    let mut text = [0; 4];
    let preferred = preferred_families(
        ch,
        &ScriptContext::new(ch.encode_utf8(&mut text), None),
        0,
        registry.default_language(),
        registry.fallback_platform(),
    );
    find_fallback_face(
        registry,
        ch,
        Presentation::Text,
        style,
        &[exclude],
        preferred,
    )
    .map(|(face, _)| face)
}

/// The weight, style and width a fallback face should match: those of the
/// text it stands in for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FaceStyle {
    pub(crate) weight: u16,
    pub(crate) style: fontdb::Style,
    pub(crate) stretch: fontdb::Stretch,
}

impl FaceStyle {
    /// The requested `weight`, with the style and width of `face_id`, the
    /// face the text was resolved to. The weight comes from the request,
    /// not the face: a variable face resolves every weight to one face.
    pub(crate) fn of_face(registry: &FontRegistry, face_id: FontFaceId, weight: u16) -> Self {
        let (style, stretch) = registry
            .face_style(face_id)
            .unwrap_or((fontdb::Style::Normal, fontdb::Stretch::Normal));
        Self {
            weight,
            style,
            stretch,
        }
    }
}

/// How a character is to be drawn, which decides the face fallback prefers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Presentation {
    /// As text: a face that draws it plain first, then one that draws it in
    /// colour. A symbol with no emoji presentation of its own (U+2764 with no
    /// U+FE0F after it) stays a text symbol when a text face has it.
    Text,
    /// As an emoji: a face that draws it in colour first (see
    /// [`crate::shaping::emoji`]), then one that draws it plain.
    Emoji,
}

impl Presentation {
    /// 0 for the glyph this presentation wants, 1 for the one it makes do
    /// with.
    fn rank(self, paint: GlyphPaint) -> u8 {
        match (self, paint) {
            (Presentation::Text, GlyphPaint::Plain) | (Presentation::Emoji, GlyphPaint::Color) => 0,
            (Presentation::Text, GlyphPaint::Color) | (Presentation::Emoji, GlyphPaint::Plain) => 1,
        }
    }
}

/// Find a face other than those in `exclude` that has a glyph for `ch`,
/// and say how that glyph is drawn.
///
/// Faces are visited explicitly-registered first, then system faces, and for
/// [`Presentation::Emoji`] the faces whose family name says "emoji" before
/// both: every colour emoji font in use (Apple Color Emoji, Segoe UI Emoji,
/// Noto Color Emoji, Twemoji) says so in its name, and asking the metadata
/// first finds it without opening, one by one, the system faces that sit
/// before it and happen to have a monochrome glyph.
///
/// The first face whose glyph is drawn as the presentation wants (see
/// [`Presentation`]) is taken. Failing that, the first face that draws the
/// glyph some other way, so that the system faces need not all be opened,
/// which loads every one of them, to find that none draws it as wanted.
///
/// The face taken is then swapped for the face of its family that matches
/// `style` (see [`FontRegistry::family_face_matching`]), when that face has
/// the glyph too and draws it as well: the first face found is whichever
/// file the system lists first, Bold as readily as Regular.
///
/// For text, between the explicitly-registered faces and the system ones,
/// the families of `preferred` are asked in turn, each for its face matching
/// `style`, and the first that draws the glyph plain is taken: the faces the
/// system ships for the character's script and language (see
/// [`crate::font::script_fallback`]), which the first system face to cover
/// it often is not. A face the host registered still wins over them.
pub(crate) fn find_fallback_face(
    registry: &FontRegistry,
    ch: char,
    presentation: Presentation,
    style: FaceStyle,
    exclude: &[FontFaceId],
    preferred: &[&str],
) -> Option<(FontFaceId, GlyphPaint)> {
    #[derive(Clone, Copy, PartialEq)]
    enum Pass {
        EmojiNamed,
        Explicit,
        System,
    }
    let passes: &[Pass] = match presentation {
        Presentation::Text => &[Pass::Explicit, Pass::System],
        Presentation::Emoji => &[Pass::EmojiNamed, Pass::Explicit, Pass::System],
    };

    let glyph_paint_of = |face_id: FontFaceId| {
        let entry = registry.get(face_id)?;
        // A face the rasterizer cannot draw would put blanks where the text is.
        if !entry.color_tables().drawable() {
            return None;
        }
        let glyph = swash::FontRef::from_index(entry.bytes(), entry.face_index as usize)?
            .charmap()
            .map(ch);
        (glyph != 0).then(|| glyph_paint(entry, glyph))
    };
    let matching = |(face_id, paint): (FontFaceId, GlyphPaint)| {
        registry
            .family_face_matching(face_id, style.weight, style.style, style.stretch)
            .filter(|&better| better != face_id && !exclude.contains(&better))
            .and_then(|better| {
                let better_paint = glyph_paint_of(better)?;
                (presentation.rank(better_paint) <= presentation.rank(paint))
                    .then_some((better, better_paint))
            })
            .unwrap_or((face_id, paint))
    };

    // The first face seen that draws the glyph, but not as wanted.
    let mut second: Option<(FontFaceId, GlyphPaint)> = None;
    for &pass in passes {
        // The table comes after the faces the host registered, so an app that
        // ships its own font for a script keeps it, and before the system's.
        if pass == Pass::System
            && presentation == Presentation::Text
            && let Some(found) = preferred.iter().find_map(|family| {
                let face_id =
                    registry.family_face(family, style.weight, style.style, style.stretch)?;
                (!exclude.contains(&face_id) && glyph_paint_of(face_id) == Some(GlyphPaint::Plain))
                    .then_some((face_id, GlyphPaint::Plain))
            })
        {
            return Some(found);
        }
        for (face_id, entry) in registry.all_entries() {
            if exclude.contains(&face_id) {
                continue;
            }
            let visits = match pass {
                Pass::EmojiNamed => registry.is_emoji_family(face_id),
                Pass::Explicit => !entry.is_system,
                Pass::System => entry.is_system,
            };
            if !visits || !entry.color_tables().drawable() {
                continue;
            }
            let Some(glyph) = swash::FontRef::from_index(entry.bytes(), entry.face_index as usize)
                .map(|font_ref| font_ref.charmap().map(ch))
                .filter(|&glyph| glyph != 0)
            else {
                continue;
            };
            let paint = glyph_paint(entry, glyph);
            match presentation.rank(paint) {
                0 => return Some(matching((face_id, paint))),
                1 if pass == Pass::System => {
                    return Some(matching(second.unwrap_or((face_id, paint))));
                }
                _ => {
                    second.get_or_insert((face_id, paint));
                }
            }
        }
    }
    second.map(matching)
}

/// Whether `face_id` has a glyph for every character of `text` that needs
/// one: whitespace, controls and default-ignorable characters (variation
/// selectors, zero-width joiners) are skipped.
pub(crate) fn face_covers(registry: &FontRegistry, face_id: FontFaceId, text: &str) -> bool {
    let Some(entry) = registry.get(face_id) else {
        return false;
    };
    let Some(font_ref) = swash::FontRef::from_index(entry.bytes(), entry.face_index as usize)
    else {
        return false;
    };
    let charmap = font_ref.charmap();
    text.chars()
        .filter(|&c| !c.is_whitespace() && !c.is_control() && !is_default_ignorable(c))
        .all(|c| charmap.map(c) != 0)
}

/// The default-ignorable characters a font need not map: they shape to
/// nothing whether it does or not.
fn is_default_ignorable(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{180B}'..='\u{180F}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FEFF}'
            | '\u{E0000}'..='\u{E0FFF}'
    )
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
