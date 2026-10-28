use swash::scale::{Render, ScaleContext, Source, StrikeWith};
use swash::zeno::{Format, Transform};
use swash::{CacheKey, FontRef};

use crate::font::synthesis::{self, FaceSynthesis};

/// swash's `embolden` strength for an outline that should end up
/// [`synthesis::embolden_strength`] wider. swash moves every edge out by the
/// whole strength, where FreeType, whose algorithm it ports, halves it first,
/// so the glyph grows by twice what it is given (pinned by
/// `a_synthetic_bold_glyph_is_as_much_wider_as_its_advance`).
const EMBOLDEN_TO_SWASH: f32 = 0.5;

#[non_exhaustive]
pub struct GlyphImage {
    pub width: u32,
    pub height: u32,
    pub placement_left: i32,
    pub placement_top: i32,
    pub data: Vec<u8>,
    pub is_color: bool,
}

/// Rasterize upright text; see [`rasterize_glyph_styled`].
#[allow(clippy::too_many_arguments)]
pub fn rasterize_glyph(
    scale_context: &mut ScaleContext,
    font_data: &[u8],
    face_index: u32,
    cache_key: CacheKey,
    glyph_id: u16,
    size_px: f32,
    font_weight: u32,
    hinted: bool,
) -> Option<GlyphImage> {
    rasterize_glyph_styled(
        scale_context,
        font_data,
        face_index,
        cache_key,
        glyph_id,
        size_px,
        font_weight,
        hinted,
        false,
    )
}

/// Rasterize glyph `glyph_id` of the face at `size_px` pixels per em, for
/// text of `font_weight` that asks for italic when `italic`. A face lighter
/// than a bold weight is emboldened, and an upright face asked for italic is
/// slanted (see `crate::font::synthesis`).
#[allow(clippy::too_many_arguments)]
pub fn rasterize_glyph_styled(
    scale_context: &mut ScaleContext,
    font_data: &[u8],
    face_index: u32,
    cache_key: CacheKey,
    glyph_id: u16,
    size_px: f32,
    font_weight: u32,
    hinted: bool,
    italic: bool,
) -> Option<GlyphImage> {
    // swash paints COLRv0 layers and colour bitmaps, not COLRv1, which is
    // painted here (see `crate::atlas::colr`). A face can carry both for one
    // glyph, the COLRv0 layers for renderers without COLRv1: the COLRv1
    // paint is the glyph as designed.
    if let Some(image) = crate::atlas::colr::paint_colr_v1(font_data, face_index, glyph_id, size_px)
    {
        return Some(image);
    }

    let base = FontRef::from_index(font_data, face_index as usize)?;
    let font_ref = FontRef {
        data: base.data,
        offset: base.offset,
        key: cache_key,
    };

    // Always set the wght variation axis explicitly so the scaler
    // gets deterministic state regardless of ScaleContext reuse
    // history.  For non-variable fonts this is a harmless no-op.
    //
    // `hinted` is false when rasterizing under a raster scale (zoomed
    // content): glyph positions come from shaping at the *logical*
    // ppem, and hinting at a different physical ppem would grid-snap
    // stems inconsistently with those positions.
    let mut scaler = scale_context
        .builder(font_ref)
        .size(size_px)
        .hint(hinted)
        .variations(&[("wght", font_weight as f32)])
        .build();

    // A face lighter than the weight asked of it is emboldened, by as much as
    // the shaper widened its advances, and an upright face asked for italic
    // is slanted about the baseline, which moves no advance (see
    // `crate::font::synthesis`). swash emboldens outline glyphs only, and
    // slants outline and COLRv0 glyphs; colour bitmaps are drawn as they are.
    let synthesis = FaceSynthesis::read(font_data, face_index);
    let embolden = if synthesis.synthesizes_bold(font_weight.min(1000) as u16) {
        synthesis::embolden_strength(size_px) * EMBOLDEN_TO_SWASH
    } else {
        0.0
    };
    // swash outlines are y up, so a positive shear leans the tops forward.
    let slant = synthesis
        .synthesizes_oblique(italic)
        .then(|| Transform::new(1.0, 0.0, synthesis::OBLIQUE_SHEAR, 1.0, 0.0, 0.0));
    let mut image = Render::new(&[
        Source::ColorOutline(0),
        Source::ColorBitmap(StrikeWith::BestFit),
        Source::Outline,
    ])
    .format(Format::Alpha)
    .embolden(embolden)
    .transform(slant)
    .render(&mut scaler, glyph_id);

    // A COLRv1 glyph the painter could not draw (a cycle of colour glyphs,
    // malformed data) has, in the fonts that use COLRv1 (Noto Color Emoji as
    // Fedora ships it), an empty outline beside its paint, so it would come
    // out as nothing, which reads as a lost character. Draw the face's
    // missing-glyph box in its place.
    let drew_nothing = image
        .as_ref()
        .is_none_or(|img| img.placement.width == 0 || img.placement.height == 0);
    if drew_nothing && crate::atlas::colr::has_colr_v1(font_data, face_index, glyph_id) {
        image = Render::new(&[Source::Outline])
            .format(Format::Alpha)
            .render(&mut scaler, 0);
    }
    let image = image?;

    let is_color = matches!(image.content, swash::scale::image::Content::Color);

    // Swash sets `image.content` independently of `Format::Alpha`: color
    // sources always produce 4-bpp RGBA data even when the format hint
    // is Alpha, and outline sources with Format::Alpha produce 1-bpp.
    // Downstream blits dispatch on `is_color` (→ blit_rgba, 4 bpp) vs
    // the alpha path (→ blit_mask, 1 bpp). If a future swash upgrade or
    // an unexpected `Content::SubpixelMask` (3 bpp) ever broke that
    // invariant, the blit would silently read past the data buffer and
    // splatter neighbor glyph pixels across the atlas — refuse to
    // rasterize rather than corrupting the atlas. The missing glyph
    // renders as `.notdef`, which is *visibly* wrong instead of
    // silently scrambling neighboring glyphs everywhere they're sampled.
    let bytes_per_pixel = if is_color { 4 } else { 1 };
    let expected_len = (image.placement.width * image.placement.height) as usize * bytes_per_pixel;
    if image.data.len() != expected_len {
        debug_assert!(
            false,
            "swash image data length {} disagrees with Content {:?} for {}x{} glyph \
             (expected {} bytes); blit_{} would corrupt the atlas — returning None",
            image.data.len(),
            image.content,
            image.placement.width,
            image.placement.height,
            expected_len,
            if is_color { "rgba" } else { "mask" },
        );
        return None;
    }

    Some(GlyphImage {
        width: image.placement.width,
        height: image.placement.height,
        placement_left: image.placement.left,
        placement_top: image.placement.top,
        data: image.data,
        is_color,
    })
}
