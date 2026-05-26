use swash::scale::{Render, ScaleContext, Source, StrikeWith};
use swash::zeno::Format;
use swash::{CacheKey, FontRef};

pub struct GlyphImage {
    pub width: u32,
    pub height: u32,
    pub placement_left: i32,
    pub placement_top: i32,
    pub data: Vec<u8>,
    pub is_color: bool,
}

pub fn rasterize_glyph(
    scale_context: &mut ScaleContext,
    font_data: &[u8],
    face_index: u32,
    cache_key: CacheKey,
    glyph_id: u16,
    size_px: f32,
    font_weight: u32,
) -> Option<GlyphImage> {
    let base = FontRef::from_index(font_data, face_index as usize)?;
    let font_ref = FontRef {
        data: base.data,
        offset: base.offset,
        key: cache_key,
    };

    // Always set the wght variation axis explicitly so the scaler
    // gets deterministic state regardless of ScaleContext reuse
    // history.  For non-variable fonts this is a harmless no-op.
    let mut scaler = scale_context
        .builder(font_ref)
        .size(size_px)
        .hint(true)
        .variations(&[("wght", font_weight as f32)])
        .build();

    let image = Render::new(&[
        Source::ColorOutline(0),
        Source::ColorBitmap(StrikeWith::BestFit),
        Source::Outline,
    ])
    .format(Format::Alpha)
    .render(&mut scaler, glyph_id)?;

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
    let expected_len =
        (image.placement.width * image.placement.height) as usize * bytes_per_pixel;
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
