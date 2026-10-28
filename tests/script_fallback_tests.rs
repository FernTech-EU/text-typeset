//! The faces glyph fallback prefers for a script and language, through the
//! public API.
//!
//! One small Noto Sans CJK subset (中你好国語日本のはテキストカ한글국어) is
//! registered under the family names the fallback table prefers on each
//! system, so the test reads the same on all three: the family name alone
//! tells the faces apart. A Japanese face that covers everything comes first,
//! as BIZ UDGothic did on Windows, so the coverage search alone would take it.

mod helpers;

use helpers::{NOTO_SANS, make_block};
use text_typeset::layout::flow::FlowLayout;
use text_typeset::{DocumentFlow, FontFaceId, Hyphenation, TextFontService, TextFormat};

const CJK: &[u8] = include_bytes!("../test-fonts/NotoSansCJKkr-Mixed-subset.otf");

const JAPANESE: [&str; 3] = ["Noto Sans CJK JP", "Yu Gothic UI", "Hiragino Sans"];
const SIMPLIFIED: [&str; 3] = ["Noto Sans CJK SC", "Microsoft YaHei UI", "PingFang SC"];

fn service() -> TextFontService {
    let mut service = TextFontService::new_without_system_fonts();
    let latin = service.register_font(NOTO_SANS);
    service.set_default_font(latin, 16.0);
    service.register_font_as(CJK, "BIZ UDGothic", 400, false);
    for family in JAPANESE.iter().chain(&SIMPLIFIED) {
        service.register_font_as(CJK, family, 400, false);
    }
    service
}

/// The family of every fallback glyph of `text`, laid out as a label.
fn label_families(service: &mut TextFontService, text: &str) -> Vec<String> {
    let result =
        DocumentFlow::new().layout_single_line(service, text, &TextFormat::default(), None, 1.0);
    families(service, result.glyph_keys.iter().map(|k| k.font_face_id))
}

fn families(service: &TextFontService, faces: impl Iterator<Item = FontFaceId>) -> Vec<String> {
    let default = service.font_registry().default_font();
    faces
        .filter(|&face| Some(face) != default)
        .map(|face| service.font_registry().font_family_name(face).unwrap())
        .collect()
}

fn all_in(families: &[String], set: &[&str]) -> bool {
    !families.is_empty() && families.iter().all(|f| set.contains(&f.as_str()))
}

#[test]
fn chinese_takes_a_chinese_face_and_follows_the_default_language() {
    let mut service = service();
    let chinese = label_families(&mut service, "中你好");
    assert!(all_in(&chinese, &SIMPLIFIED), "{chinese:?}");

    service.set_default_language(Some("ja-JP"));
    let japanese = label_families(&mut service, "中你好");
    assert!(all_in(&japanese, &JAPANESE), "{japanese:?}");
}

/// The fallback faces of a block's first line.
fn block_families(
    service: &TextFontService,
    params: &text_typeset::layout::block::BlockLayoutParams,
) -> Vec<String> {
    let mut flow = FlowLayout::new();
    flow.scale_factor = 1.0;
    flow.add_block(service.font_registry(), params, 800.0);
    let block = flow.blocks.get(&params.block_id).unwrap();
    families(
        service,
        block.lines[0]
            .runs
            .iter()
            .flat_map(|run| run.shaped_run.glyphs.iter().map(|g| g.font_face_id)),
    )
}

#[test]
fn han_in_its_own_run_takes_the_kana_of_its_paragraph() {
    let service = service();
    // 日本語 is bold and のテキスト regular: two runs, shaped apart.
    let mut params = make_block(1, "日本語のテキスト");
    let split = "日本語".len();
    let mut kana = params.fragments[0].clone();
    params.fragments[0].text = "日本語".into();
    params.fragments[0].length = split;
    params.fragments[0].font_bold = Some(true);
    kana.text = "のテキスト".into();
    kana.offset = split;
    kana.length = kana.text.len();
    params.fragments.push(kana);

    let found = block_families(&service, &params);
    assert!(all_in(&found, &JAPANESE), "{found:?}");
}

#[test]
fn the_blocks_language_wins_over_the_paragraph() {
    let service = service();
    let mut params = make_block(1, "中のテキスト");
    params.hyphenation = Some(Hyphenation::new(*b"zh"));
    let found = block_families(&service, &params);
    assert!(SIMPLIFIED.contains(&found[0].as_str()), "{found:?}");
}
