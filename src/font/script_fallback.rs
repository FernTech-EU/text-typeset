//! The faces glyph fallback prefers for a script, from a table per operating
//! system.
//!
//! Without it, fallback takes the first installed face that has the glyph,
//! whatever its script and language: on Windows, Chinese 中你好 came out in BIZ
//! UDGothic (a Japanese face, whose Han have Japanese forms), and 日本 next to
//! 한글 in Batang (a thin Korean serif) because it was the first face that had
//! both. A native text stack asks the system (CTFontCreateForString,
//! IDWriteFontFallback, fontconfig) which face draws a script for a language.
//! This crate asks a table of the faces each system ships for it, and falls
//! back to the coverage search when none of them is installed. A face the
//! host registered itself, rather than one the system scan found, still
//! comes first when it has the glyph: an app that ships its own font for a
//! script keeps it.
//!
//! The table covers the scripts whose fallback measured wrong: Japanese kana,
//! Korean hangul, and Han, whose one code point is drawn differently for
//! Japanese, Simplified Chinese, Traditional Chinese and Korean (Han
//! unification). Han is therefore drawn for a language, taken from, in this
//! order:
//!
//! 1. the language the host set on the block (today the language of its
//!    hyphenation, the only language a block carries);
//! 2. the kana or hangul nearest to it in the same paragraph: kana says
//!    Japanese, hangul Korean;
//! 3. the default language the host set on the service
//!    ([`TextFontService::set_default_language`]), its UI locale;
//! 4. Simplified Chinese.
//!
//! Every face of the table is sans-serif, the design the rest of a UI uses,
//! and is matched to the weight, style and width of the text it stands in
//! for, as the coverage search does: bold text gets the family's bold, or,
//! when the family has none, synthetic bold.
//!
//! Sources for what each system ships:
//!
//! - Windows: Microsoft, "Windows font list"
//!   (<https://learn.microsoft.com/typography/fonts/windows_11_font_list>,
//!   and the Windows 10 list beside it). Yu Gothic UI, Yu Gothic, Meiryo UI,
//!   Meiryo, Microsoft YaHei UI, Microsoft YaHei, Microsoft JhengHei UI,
//!   Microsoft JhengHei and Malgun Gothic are listed there as part of every
//!   install of Windows 10 and 11; BIZ UDGothic, Batang and the like come
//!   with optional features (the supplemental fonts a language pack adds),
//!   and the coverage search met them first.
//! - macOS: Apple, "Fonts included with macOS Sequoia" and the same page for
//!   earlier releases (support.apple.com). Hiragino Sans (since 10.11),
//!   Hiragino Kaku Gothic ProN, PingFang SC, TC and HK (since 10.11),
//!   Hiragino Sans GB and Apple SD Gothic Neo.
//! - Linux: the Noto CJK packages, Debian's `fonts-noto-cjk` and Fedora's
//!   `google-noto-sans-cjk-fonts` / `google-noto-sans-cjk-vf-fonts`
//!   ("Noto Sans CJK JP", "KR", "SC", "TC", "HK"), Google Fonts' per-language
//!   "Noto Sans JP" and the like, Adobe's Source Han Sans (whose
//!   language-specific builds are "Source Han Sans J", "K", "SC", "TC" and
//!   its region-specific ones "Source Han Sans JP", "KR", "CN", "TW"), and
//!   the older free faces: IPAexGothic and IPAGothic (`fonts-ipafont`,
//!   `ipa-gothic-fonts`), VL Gothic (`fonts-vlgothic`, `vlgothic-fonts`),
//!   WenQuanYi Zen Hei and Micro Hei (`fonts-wqy-*`, `wqy-*-fonts`) and
//!   NanumGothic (`fonts-nanum`, `naver-nanum-gothic-fonts`).
//!
//! Thai, Arabic, Hebrew, Devanagari and the other scripts are not in the
//! table: no wrong face was measured for them, and each has few faces on a
//! system, none of them for a language but its own. Emoji has its own pass
//! (see [`crate::font::resolve::find_fallback_face`]).
//!
//! [`TextFontService::set_default_language`]: crate::TextFontService::set_default_language

/// A language Han characters are drawn for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CjkLanguage {
    Japanese,
    SimplifiedChinese,
    TraditionalChinese,
    Korean,
}

impl CjkLanguage {
    /// The CJK language a BCP 47 tag (`ja`, `zh-Hant-TW`, `zh_CN`) names, or
    /// `None` for a language that is not Chinese, Japanese or Korean.
    pub(crate) fn from_tag(tag: &str) -> Option<Self> {
        let lower = tag.trim().to_ascii_lowercase();
        let mut subtags = lower.split(['-', '_']);
        match subtags.next()? {
            "ja" => Some(CjkLanguage::Japanese),
            "ko" => Some(CjkLanguage::Korean),
            "yue" => Some(CjkLanguage::TraditionalChinese),
            "zh" => {
                let traditional = subtags.any(|s| matches!(s, "hant" | "tw" | "hk" | "mo"));
                Some(if traditional {
                    CjkLanguage::TraditionalChinese
                } else {
                    CjkLanguage::SimplifiedChinese
                })
            }
            _ => None,
        }
    }
}

/// The operating system whose faces the table names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Platform {
    Windows,
    MacOs,
    /// Linux and the other Unix systems, which share the free fonts.
    Unix,
}

impl Platform {
    /// The system this crate was built for.
    pub(crate) const CURRENT: Platform = if cfg!(target_os = "windows") {
        Platform::Windows
    } else if cfg!(target_os = "macos") {
        Platform::MacOs
    } else {
        Platform::Unix
    };
}

/// The families that draw `language`, in the order they are preferred.
fn families(platform: Platform, language: CjkLanguage) -> &'static [&'static str] {
    use CjkLanguage::*;
    match (platform, language) {
        (Platform::Windows, Japanese) => &["Yu Gothic UI", "Yu Gothic", "Meiryo UI", "Meiryo"],
        (Platform::Windows, SimplifiedChinese) => &["Microsoft YaHei UI", "Microsoft YaHei"],
        (Platform::Windows, TraditionalChinese) => &["Microsoft JhengHei UI", "Microsoft JhengHei"],
        (Platform::Windows, Korean) => &["Malgun Gothic"],
        (Platform::MacOs, Japanese) => &["Hiragino Sans", "Hiragino Kaku Gothic ProN"],
        (Platform::MacOs, SimplifiedChinese) => &["PingFang SC", "Hiragino Sans GB"],
        (Platform::MacOs, TraditionalChinese) => &["PingFang TC", "PingFang HK"],
        (Platform::MacOs, Korean) => &["Apple SD Gothic Neo"],
        (Platform::Unix, Japanese) => &[
            "Noto Sans CJK JP",
            "Noto Sans JP",
            "Source Han Sans JP",
            "Source Han Sans J",
            "IPAexGothic",
            "IPAGothic",
            "VL Gothic",
        ],
        (Platform::Unix, SimplifiedChinese) => &[
            "Noto Sans CJK SC",
            "Noto Sans SC",
            "Source Han Sans SC",
            "Source Han Sans CN",
            "WenQuanYi Zen Hei",
            "WenQuanYi Micro Hei",
        ],
        (Platform::Unix, TraditionalChinese) => &[
            "Noto Sans CJK TC",
            "Noto Sans TC",
            "Source Han Sans TC",
            "Source Han Sans TW",
            "Noto Sans CJK HK",
        ],
        (Platform::Unix, Korean) => &[
            "Noto Sans CJK KR",
            "Noto Sans KR",
            "Source Han Sans KR",
            "Source Han Sans K",
            "NanumGothic",
            "Nanum Gothic",
        ],
    }
}

/// What a character is to the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TableScript {
    /// Hiragana and katakana: Japanese.
    Kana,
    /// Hangul: Korean.
    Hangul,
    /// Bopomofo: Traditional Chinese (it is Taiwan's phonetic script).
    Bopomofo,
    /// Han, and the punctuation and symbols CJK text shares, whose forms
    /// differ by language as Han's do: drawn for a language.
    Han,
}

fn table_script(ch: char) -> Option<TableScript> {
    let script = match ch as u32 {
        0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF65..=0xFF9F | 0x1AFF0..=0x1B16F => {
            TableScript::Kana
        }
        0x1100..=0x11FF | 0x3130..=0x318F | 0xA960..=0xA97F | 0xAC00..=0xD7FF | 0xFFA0..=0xFFDC => {
            TableScript::Hangul
        }
        0x3100..=0x312F | 0x31A0..=0x31BF => TableScript::Bopomofo,
        0x2E80..=0x2FFF
        | 0x3000..=0x303F
        | 0x3190..=0x319F
        | 0x31C0..=0x31EF
        | 0x3200..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F
        | 0xFF00..=0xFF64
        | 0xFFE0..=0xFFEF
        | 0x20000..=0x323AF => TableScript::Han,
        _ => return None,
    };
    Some(script)
}

/// The paragraph the text being shaped belongs to, and the language its
/// block says it is in: what decides the language of its Han.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ScriptContext<'a> {
    paragraph: &'a str,
    language: Option<CjkLanguage>,
}

impl<'a> ScriptContext<'a> {
    /// `paragraph` is the whole text of the block (or label) being laid out,
    /// `language` the language the host set on it.
    pub(crate) fn new(paragraph: &'a str, language: Option<CjkLanguage>) -> Self {
        Self {
            paragraph,
            language,
        }
    }

    /// The language Han at byte `pos` of the paragraph is drawn for, when
    /// the host or the paragraph says (steps 1 and 2 of the module doc).
    fn language_at(&self, pos: usize) -> Option<CjkLanguage> {
        self.language
            .or_else(|| nearest_kana_or_hangul(self.paragraph, pos))
    }
}

/// Japanese or Korean, from the kana or hangul nearest to byte `pos` of
/// `paragraph`, counted in characters; kana on a tie.
fn nearest_kana_or_hangul(paragraph: &str, pos: usize) -> Option<CjkLanguage> {
    let pos = pos.min(paragraph.len());
    let (before, after) = (paragraph.get(..pos)?, paragraph.get(pos..)?);
    let language = |ch: char| match table_script(ch) {
        Some(TableScript::Kana) => Some(CjkLanguage::Japanese),
        Some(TableScript::Hangul) => Some(CjkLanguage::Korean),
        _ => None,
    };
    let ahead = after
        .chars()
        .enumerate()
        .find_map(|(n, ch)| language(ch).map(|l| (n, l)));
    let behind = before
        .chars()
        .rev()
        .enumerate()
        .find_map(|(n, ch)| language(ch).map(|l| (n + 1, l)));
    match (behind, ahead) {
        (Some((b, lb)), Some((a, la))) => {
            if b < a || (b == a && lb == CjkLanguage::Japanese) {
                Some(lb)
            } else {
                Some(la)
            }
        }
        (one, other) => one.or(other).map(|(_, l)| l),
    }
}

/// The families the table prefers for `ch`, at byte `pos` of the context's
/// paragraph: empty for a character the table has nothing to say about.
pub(crate) fn preferred_families(
    ch: char,
    context: &ScriptContext<'_>,
    pos: usize,
    default_language: Option<CjkLanguage>,
    platform: Platform,
) -> &'static [&'static str] {
    let language = match table_script(ch) {
        None => return &[],
        Some(TableScript::Kana) => CjkLanguage::Japanese,
        Some(TableScript::Hangul) => CjkLanguage::Korean,
        Some(TableScript::Bopomofo) => CjkLanguage::TraditionalChinese,
        Some(TableScript::Han) => context
            .language_at(pos)
            .or(default_language)
            .unwrap_or(CjkLanguage::SimplifiedChinese),
    };
    families(platform, language)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_name_their_cjk_language() {
        use CjkLanguage::*;
        for (tag, language) in [
            ("ja", Some(Japanese)),
            ("ja-JP", Some(Japanese)),
            ("ko_KR", Some(Korean)),
            ("zh", Some(SimplifiedChinese)),
            ("zh-CN", Some(SimplifiedChinese)),
            ("zh-Hans-SG", Some(SimplifiedChinese)),
            ("zh-TW", Some(TraditionalChinese)),
            ("zh-Hant", Some(TraditionalChinese)),
            ("zh_HK", Some(TraditionalChinese)),
            ("yue", Some(TraditionalChinese)),
            ("fr-FR", None),
            ("", None),
        ] {
            assert_eq!(CjkLanguage::from_tag(tag), language, "{tag:?}");
        }
    }

    #[test]
    fn han_takes_the_language_of_the_nearest_kana_or_hangul() {
        let text = "日本語のテキスト、한국어 漢字";
        let at = |s: &str| text.find(s).unwrap();
        assert_eq!(
            nearest_kana_or_hangul(text, at("日")),
            Some(CjkLanguage::Japanese)
        );
        assert_eq!(
            nearest_kana_or_hangul(text, at("漢")),
            Some(CjkLanguage::Korean)
        );
        assert_eq!(nearest_kana_or_hangul("中文", 0), None);
    }

    #[test]
    fn the_hosts_language_wins_over_the_paragraph() {
        let context = ScriptContext::new("日本のテキスト", Some(CjkLanguage::SimplifiedChinese));
        assert_eq!(
            preferred_families('日', &context, 0, None, Platform::Unix)[0],
            "Noto Sans CJK SC"
        );
    }

    #[test]
    fn han_alone_follows_the_default_language_then_simplified_chinese() {
        let context = ScriptContext::new("中文", None);
        assert_eq!(
            preferred_families(
                '中',
                &context,
                0,
                Some(CjkLanguage::Japanese),
                Platform::Windows
            )[0],
            "Yu Gothic UI"
        );
        assert_eq!(
            preferred_families('中', &context, 0, None, Platform::MacOs)[0],
            "PingFang SC"
        );
    }

    #[test]
    fn latin_has_no_table_entry() {
        let context = ScriptContext::new("abc", None);
        assert!(preferred_families('a', &context, 0, None, Platform::Unix).is_empty());
    }

    /// Glyph fallback against a registry that "has" chosen faces: the same
    /// small Noto Sans CJK subset (中你好国語日本のはテキストカ한글국어)
    /// registered under the family names of the faces a system ships, so the
    /// family name is all that tells them apart. The faces the coverage
    /// search would meet first, a Japanese face and a Korean serif that both
    /// cover everything, come first.
    mod fallback {
        use super::super::*;
        use crate::font::registry::FontRegistry;
        use crate::font::resolve::resolve_font;
        use crate::shaping::shaper::{TextDirection, shape_text_in_paragraph};
        use crate::types::FontFaceId;

        const LATIN: &[u8] = include_bytes!("../../test-fonts/NotoSans-Variable.ttf");
        const CJK: &[u8] = include_bytes!("../../test-fonts/NotoSansCJKkr-Mixed-subset.otf");

        struct Faces {
            registry: FontRegistry,
            jp_bold: Option<FontFaceId>,
        }

        /// The Latin default face, registered by the host, then `families`
        /// (name, weight) in order, standing for installed system fonts, with
        /// the Linux table.
        fn registry_with(families: &[(&str, u16)]) -> Faces {
            let mut registry = FontRegistry::new_without_system_fonts();
            let latin = registry.register_font(LATIN)[0];
            registry.set_default_font(latin, 16.0);
            let mut jp_bold = None;
            for &(family, weight) in families {
                let face = registry.register_font_as(CJK, family, weight, false)[0];
                registry.treat_as_system(face);
                if family == "Noto Sans CJK JP" && weight == 700 {
                    jp_bold = Some(face);
                }
            }
            registry.set_fallback_platform(Platform::Unix);
            Faces { registry, jp_bold }
        }

        fn system() -> Faces {
            registry_with(&[
                ("BIZ UDGothic", 400),
                ("Noto Serif CJK KR", 400),
                ("Noto Sans CJK JP", 400),
                ("Noto Sans CJK JP", 700),
                ("Noto Sans CJK SC", 400),
                ("Noto Sans CJK TC", 400),
                ("Noto Sans CJK KR", 400),
            ])
        }

        /// The faces the glyphs of `text` are drawn in, shaped at `weight` as
        /// the part of `paragraph` that starts at byte `at`.
        fn faces_of(
            faces: &Faces,
            text: &str,
            paragraph: &str,
            at: usize,
            language: Option<CjkLanguage>,
            weight: u32,
        ) -> Vec<FontFaceId> {
            let resolved = resolve_font(
                &faces.registry,
                None,
                Some(weight),
                None,
                None,
                None,
                1.0,
                1.0,
            )
            .unwrap();
            let run = shape_text_in_paragraph(
                &faces.registry,
                &resolved,
                text,
                0,
                TextDirection::Auto,
                &[],
                &ScriptContext::new(paragraph, language),
                at,
            )
            .unwrap();
            run.glyphs
                .iter()
                .filter(|g| g.glyph_id != 0 && g.font_face_id != resolved.font_face_id)
                .map(|g| g.font_face_id)
                .collect()
        }

        fn families_of(faces: &Faces, ids: &[FontFaceId]) -> Vec<String> {
            ids.iter()
                .map(|&id| faces.registry.font_family_name(id).unwrap())
                .collect()
        }

        /// The one family every glyph of `text`, alone in its paragraph, is
        /// drawn in.
        fn family(faces: &Faces, text: &str) -> String {
            let families = families_of(faces, &faces_of(faces, text, text, 0, None, 400));
            assert!(!families.is_empty(), "{text:?} fell back to nothing");
            assert!(
                families.iter().all(|f| *f == families[0]),
                "{text:?} is drawn in {families:?}"
            );
            families[0].clone()
        }

        #[test]
        fn chinese_text_picks_the_sc_face_over_a_jp_face_that_covers_it() {
            assert_eq!(family(&system(), "中你好"), "Noto Sans CJK SC");
        }

        #[test]
        fn a_japanese_paragraph_with_kana_picks_jp_for_its_han() {
            let faces = system();
            assert_eq!(family(&faces, "日本語のテキスト"), "Noto Sans CJK JP");
            // Han alone in its run takes the kana of the paragraph around it.
            let paragraph = "日本語のテキスト";
            let ids = faces_of(&faces, "日本語", paragraph, 0, None, 400);
            assert_eq!(families_of(&faces, &ids), vec!["Noto Sans CJK JP"; 3]);
        }

        #[test]
        fn korean_picks_kr_never_a_serif() {
            let faces = system();
            assert_eq!(family(&faces, "한국어"), "Noto Sans CJK KR");
            // 日本 next to 한글: the Han takes the hangul's language, and no
            // character lands in the serif that covers both.
            let ids = faces_of(&faces, "日本 한글", "日本 한글", 0, None, 400);
            assert_eq!(families_of(&faces, &ids), vec!["Noto Sans CJK KR"; 4]);
        }

        #[test]
        fn the_blocks_language_and_the_default_language_decide_lone_han() {
            let mut faces = system();
            let ids = faces_of(
                &faces,
                "中你好",
                "中你好",
                0,
                Some(CjkLanguage::Japanese),
                400,
            );
            assert_eq!(families_of(&faces, &ids), vec!["Noto Sans CJK JP"; 3]);

            faces.registry.set_default_language(Some("zh-TW"));
            assert_eq!(family(&faces, "中你好"), "Noto Sans CJK TC");
            faces.registry.set_default_language(Some("ko"));
            assert_eq!(family(&faces, "中你好"), "Noto Sans CJK KR");
            // The paragraph's kana wins over the default language.
            assert_eq!(family(&faces, "日本のテキスト"), "Noto Sans CJK JP");
        }

        #[test]
        fn bold_text_picks_the_table_faces_bold() {
            let faces = system();
            let ids = faces_of(&faces, "日本の", "日本の", 0, None, 700);
            assert_eq!(ids, vec![faces.jp_bold.unwrap(); 3]);
        }

        #[test]
        fn with_no_table_face_installed_the_coverage_search_decides() {
            let faces = registry_with(&[("BIZ UDGothic", 400), ("Noto Serif CJK KR", 400)]);
            assert_eq!(family(&faces, "中你好"), "BIZ UDGothic");
            assert_eq!(family(&faces, "한국어"), "BIZ UDGothic");
        }

        #[test]
        fn a_face_the_host_registered_wins_over_the_table() {
            let mut faces = system();
            // An app that ships its own CJK font, registered after the
            // system's: it keeps it for every script the table covers.
            faces
                .registry
                .register_font_as(CJK, "App Sans CJK", 400, false);
            for text in ["中你好", "日本のテキスト", "한국어"] {
                assert_eq!(family(&faces, text), "App Sans CJK", "{text:?}");
            }
        }

        /// A block of `fragments` (text, bold), laid out; the family of every
        /// glyph of its first line the default face does not draw.
        fn block_families(
            faces: &Faces,
            fragments: &[(&str, bool)],
            hyphenation: Option<crate::types::Hyphenation>,
        ) -> Vec<String> {
            use crate::layout::block::{BlockLayoutParams, FragmentParams, layout_block};
            let mut text = String::new();
            let mut params_fragments = Vec::new();
            for &(piece, bold) in fragments {
                params_fragments.push(FragmentParams {
                    text: piece.to_string(),
                    offset: text.len(),
                    length: piece.len(),
                    font_family: None,
                    font_weight: None,
                    font_bold: Some(bold),
                    font_italic: None,
                    font_point_size: None,
                    size_scale: 1.0,
                    underline_style: crate::types::UnderlineStyle::None,
                    overline: false,
                    strikeout: false,
                    is_link: false,
                    letter_spacing: 0.0,
                    word_spacing: 0.0,
                    foreground_color: None,
                    underline_color: None,
                    background_color: None,
                    anchor_href: None,
                    tooltip: None,
                    vertical_alignment: crate::types::VerticalAlignment::Normal,
                    image_name: None,
                    image_width: 0.0,
                    image_height: 0.0,
                    footnote_marker: None,
                    features: Vec::new(),
                });
                text.push_str(piece);
            }
            let params = BlockLayoutParams {
                block_id: 1,
                position: 0,
                text,
                fragments: params_fragments,
                alignment: crate::layout::paragraph::Alignment::Left,
                top_margin: 0.0,
                bottom_margin: 0.0,
                left_margin: 0.0,
                right_margin: 0.0,
                text_indent: 0.0,
                list_marker: String::new(),
                list_indent: 0.0,
                tab_positions: Vec::new(),
                base_direction: TextDirection::Auto,
                line_height_multiplier: None,
                non_breakable_lines: false,
                hyphenation,
                checkbox: None,
                background_color: None,
            };
            let block = layout_block(&faces.registry, &params, 800.0, 1.0, 1.0);
            let default = faces.registry.default_font();
            let ids: Vec<FontFaceId> = block.lines[0]
                .runs
                .iter()
                .flat_map(|run| run.shaped_run.glyphs.iter().map(|g| g.font_face_id))
                .filter(|&id| Some(id) != default)
                .collect();
            families_of(faces, &ids)
        }

        #[test]
        fn han_in_a_run_of_its_own_takes_the_kana_of_its_block() {
            // 日本語 bold and のテキスト regular: two runs, shaped apart.
            let faces = system();
            let found = block_families(&faces, &[("日本語", true), ("のテキスト", false)], None);
            assert_eq!(found.len(), 8, "{found:?}");
            assert!(found.iter().all(|f| f == "Noto Sans CJK JP"), "{found:?}");
        }

        #[test]
        fn the_blocks_language_wins_over_its_kana() {
            let faces = system();
            let found = block_families(
                &faces,
                &[("中のテキスト", false)],
                Some(crate::types::Hyphenation::new(*b"zh")),
            );
            assert_eq!(found[0], "Noto Sans CJK SC", "{found:?}");
        }

        #[test]
        fn a_label_follows_the_services_default_language() {
            use crate::{DocumentFlow, TextFontService, TextFormat};
            let mut service = TextFontService::new_without_system_fonts();
            service.font_registry = system().registry;
            let label = |service: &mut TextFontService| {
                let result = DocumentFlow::new().layout_single_line(
                    service,
                    "中你好",
                    &TextFormat::default(),
                    None,
                    1.0,
                );
                let ids: Vec<FontFaceId> =
                    result.glyph_keys.iter().map(|k| k.font_face_id).collect();
                ids.iter()
                    .map(|&id| service.font_registry.font_family_name(id).unwrap())
                    .collect::<Vec<_>>()
            };
            assert_eq!(label(&mut service), vec!["Noto Sans CJK SC"; 3]);
            service.set_default_language(Some("ja-JP"));
            assert_eq!(label(&mut service), vec!["Noto Sans CJK JP"; 3]);
        }
    }
}
