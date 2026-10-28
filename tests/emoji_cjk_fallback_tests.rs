//! Glyph fallback across scripts and into colour emoji, with real faces.
//!
//! The faces are small subsets of Noto fonts, so each test controls exactly
//! which face covers which character, as an operating system's font set
//! does:
//!
//! - `NotoSansCJKjp-NoNi-subset.otf`: a Japanese face without 你 (U+4F60),
//!   standing for the Japanese or Korean system font that a fallback search
//!   meets first for 中 on Windows, and that has no 你.
//! - `NotoSansCJKsc-Ni-subset.otf`: a Simplified Chinese face with 你好中我
//!   only.
//! - `NotoSansCJKjp-Bold-NoNi-subset.otf`: the Bold of the Japanese face,
//!   with the same characters.
//! - `NotoColorEmoji-CBDT-subset.ttf`: colour bitmaps for a few emoji.
//! - `NotoColorEmoji-COLRv1-subset.ttf`: the same family in COLRv1, the
//!   format Fedora ships.
//! - `NotoSansSymbols2-Heart-subset.ttf`: a monochrome U+2764 heart.

mod helpers;

use helpers::NOTO_SANS;
use text_typeset::{DocumentFlow, FontFaceId, SingleLineResult, TextFontService, TextFormat};

const CJK_JP_NO_NI: &[u8] = include_bytes!("../test-fonts/NotoSansCJKjp-NoNi-subset.otf");
const CJK_SC_NI: &[u8] = include_bytes!("../test-fonts/NotoSansCJKsc-Ni-subset.otf");
const CJK_JP_BOLD_NO_NI: &[u8] = include_bytes!("../test-fonts/NotoSansCJKjp-Bold-NoNi-subset.otf");
const EMOJI_CBDT: &[u8] = include_bytes!("../test-fonts/NotoColorEmoji-CBDT-subset.ttf");
const EMOJI_COLRV1: &[u8] = include_bytes!("../test-fonts/NotoColorEmoji-COLRv1-subset.ttf");
const SYMBOLS2_HEART: &[u8] = include_bytes!("../test-fonts/NotoSansSymbols2-Heart-subset.ttf");

struct Faces {
    service: TextFontService,
    latin: FontFaceId,
    jp: FontFaceId,
    sc: FontFaceId,
    heart: FontFaceId,
    emoji: FontFaceId,
}

/// Noto Sans as the default face, then the fallback faces in the order an
/// unlucky system enumerates them: the CJK face without 你 before the one
/// with it, and the monochrome heart before the colour emoji.
fn faces_with(emoji_font: &[u8]) -> Faces {
    faces_in_order(emoji_font, false)
}

/// As [`faces_with`], with the emoji face before the monochrome heart when
/// `emoji_first`.
fn faces_in_order(emoji_font: &[u8], emoji_first: bool) -> Faces {
    let mut service = TextFontService::new_without_system_fonts();
    let latin = service.register_font(NOTO_SANS);
    service.set_default_font(latin, 16.0);
    let jp = service.register_font(CJK_JP_NO_NI);
    let sc = service.register_font(CJK_SC_NI);
    let (heart, emoji) = if emoji_first {
        let emoji = service.register_font(emoji_font);
        (service.register_font(SYMBOLS2_HEART), emoji)
    } else {
        let heart = service.register_font(SYMBOLS2_HEART);
        (heart, service.register_font(emoji_font))
    };
    Faces {
        service,
        latin,
        jp,
        sc,
        heart,
        emoji,
    }
}

fn layout(faces: &mut Faces, text: &str, size: f32) -> SingleLineResult {
    let format = TextFormat {
        font_size: Some(size),
        ..TextFormat::default()
    };
    DocumentFlow::new().layout_single_line(&mut faces.service, text, &format, None, 1.0)
}

/// The face that drew each glyph quad, in visual order.
fn quad_faces(result: &SingleLineResult) -> Vec<FontFaceId> {
    result.glyph_keys.iter().map(|k| k.font_face_id).collect()
}

/// The atlas pixels of one quad: (visible pixels, of which coloured).
fn quad_pixels(service: &TextFontService, result: &SingleLineResult, i: usize) -> (usize, usize) {
    let [x, y, w, h] = result.glyphs[i].atlas;
    let width = service.atlas_width() as usize;
    let pixels = service.atlas_pixels();
    let (mut visible, mut colored) = (0, 0);
    for row in y as usize..(y + h) as usize {
        for col in x as usize..(x + w) as usize {
            let p = &pixels[(row * width + col) * 4..][..4];
            if p[3] > 32 {
                visible += 1;
                let spread = p[..3].iter().max().unwrap() - p[..3].iter().min().unwrap();
                if spread > 48 {
                    colored += 1;
                }
            }
        }
    }
    (visible, colored)
}

#[test]
fn an_emoji_after_cjk_text_is_drawn_in_colour() {
    // The Windows report: "Showcase日本にほん😁😁" drew no emoji. The span of
    // characters Noto Sans lacks ran from 日 to the last emoji and was shaped
    // in the face found for 日, which has no emoji.
    let mut f = faces_with(EMOJI_CBDT);
    let text = "Showcase\u{65E5}\u{672C}\u{306B}\u{307B}\u{3093}\u{1F601}\u{1F601}";
    let result = layout(&mut f, text, 16.0);

    let faces = quad_faces(&result);
    assert_eq!(faces.len(), 15, "one quad per character: {faces:?}");
    assert!(faces[..8].iter().all(|&id| id == f.latin));
    assert!(faces[8..13].iter().all(|&id| id == f.jp));
    assert_eq!(&faces[13..], &[f.emoji, f.emoji]);
    for i in 13..15 {
        assert!(result.glyphs[i].is_color, "the emoji quad is a colour quad");
        let (visible, colored) = quad_pixels(&f.service, &result, i);
        assert!(
            colored > 50,
            "the emoji has coloured pixels: {visible} visible, {colored} coloured"
        );
    }
}

#[test]
fn a_flag_before_cjk_text_leaves_the_cjk_drawn() {
    // The macOS report: after 🇯🇵 the CJK text drew as blank space. The flag's
    // face took the whole span, CJK included.
    let mut f = faces_with(EMOJI_CBDT);
    let result = layout(&mut f, "\u{1F1EF}\u{1F1F5}\u{6559}\u{4E2D}\u{6587}", 16.0);
    let faces = quad_faces(&result);
    assert_eq!(faces.first(), Some(&f.emoji), "{faces:?}");
    assert_eq!(&faces[faces.len() - 3..], &[f.jp, f.jp, f.jp]);
}

#[test]
fn a_cjk_word_is_set_in_the_face_that_draws_all_of_it() {
    // The Windows report: 你 went missing in "中你好" and "我你好你好". The
    // span went to the first face with 中, which has no 你. The face with
    // 你 has the whole word, so the whole word goes to it, not one character.
    let mut f = faces_with(EMOJI_CBDT);
    for text in [
        "\u{4E2D}\u{4F60}\u{597D}",
        "\u{6211}\u{4F60}\u{597D}\u{4F60}\u{597D}",
    ] {
        let result = layout(&mut f, text, 32.0);
        let faces = quad_faces(&result);
        assert_eq!(faces.len(), text.chars().count(), "{text}: {faces:?}");
        assert!(faces.iter().all(|&id| id == f.sc), "{text}: {faces:?}");
        for i in 0..faces.len() {
            assert!(
                quad_pixels(&f.service, &result, i).0 > 20,
                "{text}: glyph {i} drew"
            );
        }
    }
}

#[test]
fn a_fallback_face_has_the_weight_of_its_text() {
    // The Windows report: regular 中你好 came out in BIZ UDGothic Bold, whose
    // file sorts before the Regular one. Here too the Bold is registered
    // first.
    let mut service = TextFontService::new_without_system_fonts();
    let latin = service.register_font(NOTO_SANS);
    service.set_default_font(latin, 16.0);
    let bold = service.register_font(CJK_JP_BOLD_NO_NI);
    let regular = service.register_font(CJK_JP_NO_NI);
    let mut layout_weighted = |text: &str, bold_text: bool| {
        let format = TextFormat {
            font_size: Some(16.0),
            font_bold: Some(bold_text),
            ..TextFormat::default()
        };
        let result = DocumentFlow::new().layout_single_line(&mut service, text, &format, None, 1.0);
        quad_faces(&result)
    };
    assert_eq!(
        layout_weighted("a\u{65E5}\u{672C}", false),
        vec![latin, regular, regular]
    );
    assert_eq!(
        layout_weighted("a\u{65E5}\u{672C}", true),
        vec![latin, bold, bold]
    );
}

#[test]
fn a_character_no_face_of_the_span_has_gets_a_face_of_its_own() {
    // No face has both 日 and 你: each keeps its own.
    let mut f = faces_with(EMOJI_CBDT);
    let result = layout(&mut f, "a\u{65E5}\u{4F60}\u{65E5}", 16.0);
    assert_eq!(quad_faces(&result), vec![f.latin, f.jp, f.sc, f.jp]);
}

#[test]
fn the_emoji_variation_selector_asks_for_the_colour_heart() {
    // The Windows report: ❤️ (U+2764 U+FE0F) was drawn as a black text heart.
    // A monochrome heart sits before the colour one in the fallback order.
    let mut f = faces_with(EMOJI_CBDT);
    let result = layout(&mut f, "a\u{2764}\u{FE0F}b", 16.0);
    let faces = quad_faces(&result);
    assert_eq!(faces, vec![f.latin, f.emoji, f.latin]);
    assert!(result.glyphs[1].is_color);

    // Without the selector U+2764 defaults to text: the plain heart is right,
    // even when the colour face comes first.
    for emoji_first in [false, true] {
        let mut f = faces_in_order(EMOJI_CBDT, emoji_first);
        let result = layout(&mut f, "a\u{2764}b", 16.0);
        assert_eq!(quad_faces(&result), vec![f.latin, f.heart, f.latin]);
        assert!(!result.glyphs[1].is_color);
    }
}

#[test]
fn a_face_that_draws_the_heart_in_outline_gives_way_to_colour() {
    // Inter, Teksilo's default face, has a U+2764 of its own and maps U+FE0F,
    // so the red heart never reached fallback. Here the monochrome heart is
    // the default face.
    let mut f = faces_with(EMOJI_CBDT);
    f.service.set_default_font(f.heart, 16.0);
    let result = layout(&mut f, "\u{2764}\u{FE0F}", 16.0);
    assert_eq!(quad_faces(&result), vec![f.emoji]);
    assert!(result.glyphs[0].is_color);

    let result = layout(&mut f, "\u{2764}", 16.0);
    assert_eq!(quad_faces(&result), vec![f.heart]);
}

#[test]
fn an_emoji_is_drawn_at_the_size_of_its_text() {
    // The emoji of a heading is as large, against the heading's letters, as
    // the emoji of body text is against body text.
    let mut f = faces_with(EMOJI_CBDT);
    let mut ratios = Vec::new();
    for size in [14.0, 21.0, 28.0, 32.0] {
        let result = layout(&mut f, "H\u{1F62C}", size);
        assert_eq!(quad_faces(&result), vec![f.latin, f.emoji], "at {size}px");
        let letter = result.glyphs[0].screen[3];
        let emoji = result.glyphs[1].screen[3];
        assert!(
            (emoji / size - 1.17).abs() < 0.1,
            "at {size}px the emoji is {emoji}px tall"
        );
        ratios.push(emoji / letter);
    }
    let (min, max) = ratios
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), &r| (lo.min(r), hi.max(r)));
    assert!(max - min < 0.1, "emoji to letter height ratios: {ratios:?}");
}

#[test]
fn a_colrv1_emoji_is_drawn_in_colour() {
    // Fedora's Noto Color Emoji is COLRv1, which the rasterizer did not
    // paint, and its base glyphs have no outline: an emoji set in it drew
    // as blank space. With the colour face before the plain heart, it is
    // the one fallback takes, and its glyphs come out in colour.
    let mut f = faces_in_order(EMOJI_COLRV1, true);
    for text in ["a\u{2764}\u{FE0F}b", "a\u{1F600}b"] {
        let result = layout(&mut f, text, 16.0);
        assert_eq!(
            quad_faces(&result),
            vec![f.latin, f.emoji, f.latin],
            "{text}"
        );
        assert!(result.glyphs[1].is_color, "{text}");
        let (visible, colored) = quad_pixels(&f.service, &result, 1);
        assert!(
            colored > 30,
            "{text}: {visible} visible, {colored} coloured"
        );
    }
}
