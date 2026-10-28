//! Synthetic bold for a face lighter than the weight asked of it.
//!
//! `NotoSansCJKjp-NoNi-subset.otf` is a one-weight CJK face, as Arial
//! Unicode MS is on the Mac, where bold CJK stayed regular (Mac round 10).

mod helpers;

use helpers::NOTO_SANS;
use text_typeset::atlas::rasterizer::{GlyphImage, rasterize_glyph};
use text_typeset::{DocumentFlow, SingleLineResult, TextFontService, TextFormat};

const REGULAR_CJK: &[u8] = include_bytes!("../test-fonts/NotoSansCJKjp-NoNi-subset.otf");
const BOLD_CJK: &[u8] = include_bytes!("../test-fonts/NotoSansCJKjp-Bold-NoNi-subset.otf");

fn raster(font: &[u8], ch: char, size: f32, weight: u32) -> GlyphImage {
    let glyph = swash::FontRef::from_index(font, 0)
        .unwrap()
        .charmap()
        .map(ch);
    let mut context = swash::scale::ScaleContext::new();
    rasterize_glyph(
        &mut context,
        font,
        0,
        swash::CacheKey::new(),
        glyph,
        size,
        weight,
        false,
    )
    .unwrap()
}

fn ink(image: &GlyphImage) -> u32 {
    image.data.iter().map(|&a| u32::from(a)).sum()
}

#[test]
fn a_synthetic_bold_glyph_is_as_much_wider_as_its_advance() {
    // 96 px per em: the outline grows by 4 px, a twenty-fourth of the em.
    let regular = raster(REGULAR_CJK, '\u{65E5}', 96.0, 400);
    let bold = raster(REGULAR_CJK, '\u{65E5}', 96.0, 700);
    let wider = bold.width as f32 - regular.width as f32;
    assert!((wider - 96.0 / 24.0).abs() <= 1.5, "wider by {wider}");
    assert!(ink(&bold) as f32 > ink(&regular) as f32 * 1.15);
}

fn layout(service: &mut TextFontService, text: &str, bold: bool) -> SingleLineResult {
    let format = TextFormat {
        font_size: Some(24.0),
        font_bold: Some(bold),
        ..TextFormat::default()
    };
    DocumentFlow::new().layout_single_line(service, text, &format, None, 1.0)
}

fn quads_ink(service: &TextFontService, result: &SingleLineResult) -> u32 {
    let width = service.atlas_width() as usize;
    let pixels = service.atlas_pixels();
    let mut total = 0;
    for q in &result.glyphs {
        let [x, y, w, h] = q.atlas;
        for row in y as usize..(y + h) as usize {
            for col in x as usize..(x + w) as usize {
                total += u32::from(pixels[(row * width + col) * 4 + 3]);
            }
        }
    }
    total
}

#[test]
fn bold_text_in_a_one_weight_face_is_emboldened_and_advances_with_it() {
    let mut service = TextFontService::new_without_system_fonts();
    let latin = service.register_font(NOTO_SANS);
    service.set_default_font(latin, 16.0);
    let cjk = service.register_font(REGULAR_CJK);
    let text = "\u{65E5}\u{672C}\u{6559}";

    let regular = layout(&mut service, text, false);
    let regular_ink = quads_ink(&service, &regular);
    let bold = layout(&mut service, text, true);
    let bold_ink = quads_ink(&service, &bold);

    let faces = |r: &SingleLineResult| {
        r.glyph_keys
            .iter()
            .map(|k| k.font_face_id)
            .collect::<Vec<_>>()
    };
    assert_eq!(faces(&regular), vec![cjk; 3]);
    assert_eq!(faces(&bold), vec![cjk; 3], "the one face, for bold too");

    // Each advance grows by a twenty-fourth of the em, as the outline does.
    let extra = 24.0 / 24.0;
    assert!(
        (bold.width - regular.width - 3.0 * extra).abs() < 0.01,
        "{} vs {}",
        bold.width,
        regular.width
    );
    let step = |r: &SingleLineResult| r.glyphs[1].screen[0] - r.glyphs[0].screen[0];
    // The second glyph sits one widened advance on, its ink grown to the left
    // by half the extra as the first glyph's is.
    assert!(
        (step(&bold) - step(&regular) - extra).abs() < 1.01,
        "{} vs {}",
        step(&bold),
        step(&regular)
    );
    assert!(
        bold_ink as f32 > regular_ink as f32 * 1.1,
        "ink {bold_ink} vs {regular_ink}"
    );
}

#[test]
fn a_face_that_has_the_weight_is_not_emboldened() {
    // The Bold face, asked for bold, is drawn and measured as it is.
    let regular_weight = raster(BOLD_CJK, '\u{65E5}', 96.0, 400);
    let bold_weight = raster(BOLD_CJK, '\u{65E5}', 96.0, 700);
    assert_eq!(
        (regular_weight.width, regular_weight.height),
        (bold_weight.width, bold_weight.height)
    );
    assert_eq!(ink(&regular_weight), ink(&bold_weight));

    // And a regular face asked for regular is not touched either.
    let a = raster(REGULAR_CJK, '\u{65E5}', 96.0, 400);
    let b = raster(REGULAR_CJK, '\u{65E5}', 96.0, 500);
    assert_eq!(ink(&a), ink(&b));
}
