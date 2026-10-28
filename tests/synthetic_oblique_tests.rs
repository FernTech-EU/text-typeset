//! Synthetic oblique for italic text on a face that has no italic.
//!
//! `NotoSansCJKjp-NoNi-subset.otf` is a one-style CJK face: no CJK font has
//! an italic. `NotoSans-Italic-subset.ttf` (a, b, c) is a real italic.

mod helpers;

use helpers::NOTO_SANS;
use text_typeset::atlas::rasterizer::{GlyphImage, rasterize_glyph_styled};
use text_typeset::{DocumentFlow, SingleLineResult, TextFontService, TextFormat};

const UPRIGHT_CJK: &[u8] = include_bytes!("../test-fonts/NotoSansCJKjp-NoNi-subset.otf");
const ITALIC_LATIN: &[u8] = include_bytes!("../test-fonts/NotoSans-Italic-subset.ttf");

fn raster(font: &[u8], ch: char, italic: bool) -> GlyphImage {
    let glyph = swash::FontRef::from_index(font, 0)
        .unwrap()
        .charmap()
        .map(ch);
    let mut context = swash::scale::ScaleContext::new();
    rasterize_glyph_styled(
        &mut context,
        font,
        0,
        swash::CacheKey::new(),
        glyph,
        96.0,
        400,
        false,
        italic,
    )
    .unwrap()
}

/// The ink-weighted mean x of the rows `rows`, in the image's own pixels
/// measured from its placement origin.
fn mean_x(image: &GlyphImage, rows: std::ops::Range<u32>) -> f32 {
    let (mut sum, mut weight) = (0.0, 0.0);
    for row in rows {
        for col in 0..image.width {
            let a = f32::from(image.data[(row * image.width + col) as usize]);
            sum += a * (col as f32 + image.placement_left as f32);
            weight += a;
        }
    }
    sum / weight
}

#[test]
fn an_upright_face_asked_for_italic_is_slanted_about_the_baseline() {
    let upright = raster(UPRIGHT_CJK, '\u{65E5}', false);
    let slanted = raster(UPRIGHT_CJK, '\u{65E5}', true);
    assert_eq!(slanted.height, upright.height, "a slant moves no row");
    assert_eq!(slanted.placement_top, upright.placement_top);

    // The top quarter leans right of the bottom quarter by the shear times
    // the distance between them, which upright it does not.
    let quarter = upright.height / 4;
    let lean = |image: &GlyphImage| {
        mean_x(image, 0..quarter) - mean_x(image, image.height - quarter..image.height)
    };
    let rise = (upright.height - quarter) as f32;
    let added = lean(&slanted) - lean(&upright);
    assert!(
        (added - 0.2 * rise).abs() < 2.0,
        "the top leans {added} px further over a {rise} px rise"
    );
}

#[test]
fn an_italic_face_is_not_slanted_again() {
    let a = raster(ITALIC_LATIN, 'a', false);
    let b = raster(ITALIC_LATIN, 'a', true);
    assert_eq!(
        (a.width, a.height, a.placement_left),
        (b.width, b.height, b.placement_left)
    );
    assert_eq!(a.data, b.data);
}

fn layout(service: &mut TextFontService, text: &str, italic: bool) -> SingleLineResult {
    let format = TextFormat {
        font_size: Some(24.0),
        font_italic: Some(italic),
        ..TextFormat::default()
    };
    DocumentFlow::new().layout_single_line(service, text, &format, None, 1.0)
}

#[test]
fn italic_text_on_an_upright_face_keeps_its_advances_and_gets_its_own_bitmaps() {
    let mut service = TextFontService::new_without_system_fonts();
    let latin = service.register_font(NOTO_SANS);
    service.set_default_font(latin, 16.0);
    let cjk = service.register_font(UPRIGHT_CJK);
    let text = "\u{65E5}\u{672C}\u{6559}";

    let upright = layout(&mut service, text, false);
    let italic = layout(&mut service, text, true);

    let faces = |r: &SingleLineResult| {
        r.glyph_keys
            .iter()
            .map(|k| k.font_face_id)
            .collect::<Vec<_>>()
    };
    assert_eq!(faces(&italic), vec![cjk; 3], "the one face, for italic too");
    assert!(italic.glyph_keys.iter().all(|k| k.italic));
    assert!(upright.glyph_keys.iter().all(|k| !k.italic));

    // A slant moves no advance: the line, and every caret stop on it, are
    // where they are upright.
    assert_eq!(italic.width, upright.width);
    // The slanted bitmaps are their own: wider, and elsewhere in the atlas.
    for (u, i) in upright.glyphs.iter().zip(&italic.glyphs) {
        assert!(
            i.screen[2] > u.screen[2],
            "{:?} vs {:?}",
            i.screen,
            u.screen
        );
        assert_ne!(i.atlas, u.atlas);
    }
}
