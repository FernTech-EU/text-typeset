//! Writing-system (script) classification for installed fonts.
//!
//! Mirrors Qt's `QFontDatabase::WritingSystem`: a font is classified by
//! the scripts it can render. Detection reads the OS/2 table's
//! `ulUnicodeRange` (128 script-coverage bits) for the script-level
//! systems, its `ulCodePageRange` for the CJK-language + Vietnamese
//! distinction (Simplified vs Traditional Chinese share codepoints, so
//! they cannot be told apart from Unicode coverage alone — Qt uses the
//! same code-page heuristic), and a cmap sample-codepoint cross-check for
//! fonts whose OS/2 ranges are absent or wrong (common in subset/older
//! fonts).
//!
//! `ttf-parser` types `ulUnicodeRange` but not `ulCodePageRange`, so the
//! code-page word is read straight from the raw OS/2 table bytes. Every
//! function here takes owned font bytes, so it is `Send` and runs on the
//! background index thread (see [`super::writing_system_index`]), never on
//! the UI thread.

use ttf_parser::{Face, Tag};

/// A script / writing system a font can render.
///
/// The set mirrors Qt's `QFontDatabase::WritingSystem` (minus the
/// redundant `Other`, which Qt documents as an alias of [`Symbol`]).
///
/// [`Symbol`]: WritingSystem::Symbol
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
#[repr(u8)]
pub enum WritingSystem {
    Latin = 0,
    Greek,
    Cyrillic,
    Armenian,
    Hebrew,
    Arabic,
    Syriac,
    Thaana,
    Devanagari,
    Bengali,
    Gurmukhi,
    Gujarati,
    Oriya,
    Tamil,
    Telugu,
    Kannada,
    Malayalam,
    Sinhala,
    Thai,
    Lao,
    Tibetan,
    Myanmar,
    Georgian,
    Khmer,
    SimplifiedChinese,
    TraditionalChinese,
    Japanese,
    Korean,
    Vietnamese,
    Symbol,
    Ogham,
    Runic,
    Nko,
}

impl WritingSystem {
    /// Every writing system, in declaration order.
    pub const ALL: [WritingSystem; 33] = [
        WritingSystem::Latin,
        WritingSystem::Greek,
        WritingSystem::Cyrillic,
        WritingSystem::Armenian,
        WritingSystem::Hebrew,
        WritingSystem::Arabic,
        WritingSystem::Syriac,
        WritingSystem::Thaana,
        WritingSystem::Devanagari,
        WritingSystem::Bengali,
        WritingSystem::Gurmukhi,
        WritingSystem::Gujarati,
        WritingSystem::Oriya,
        WritingSystem::Tamil,
        WritingSystem::Telugu,
        WritingSystem::Kannada,
        WritingSystem::Malayalam,
        WritingSystem::Sinhala,
        WritingSystem::Thai,
        WritingSystem::Lao,
        WritingSystem::Tibetan,
        WritingSystem::Myanmar,
        WritingSystem::Georgian,
        WritingSystem::Khmer,
        WritingSystem::SimplifiedChinese,
        WritingSystem::TraditionalChinese,
        WritingSystem::Japanese,
        WritingSystem::Korean,
        WritingSystem::Vietnamese,
        WritingSystem::Symbol,
        WritingSystem::Ogham,
        WritingSystem::Runic,
        WritingSystem::Nko,
    ];

    /// Stable machine identifier (settings keys, i18n label lookup, debug).
    /// Never localized and never changes across releases.
    pub const fn id(self) -> &'static str {
        match self {
            WritingSystem::Latin => "latin",
            WritingSystem::Greek => "greek",
            WritingSystem::Cyrillic => "cyrillic",
            WritingSystem::Armenian => "armenian",
            WritingSystem::Hebrew => "hebrew",
            WritingSystem::Arabic => "arabic",
            WritingSystem::Syriac => "syriac",
            WritingSystem::Thaana => "thaana",
            WritingSystem::Devanagari => "devanagari",
            WritingSystem::Bengali => "bengali",
            WritingSystem::Gurmukhi => "gurmukhi",
            WritingSystem::Gujarati => "gujarati",
            WritingSystem::Oriya => "oriya",
            WritingSystem::Tamil => "tamil",
            WritingSystem::Telugu => "telugu",
            WritingSystem::Kannada => "kannada",
            WritingSystem::Malayalam => "malayalam",
            WritingSystem::Sinhala => "sinhala",
            WritingSystem::Thai => "thai",
            WritingSystem::Lao => "lao",
            WritingSystem::Tibetan => "tibetan",
            WritingSystem::Myanmar => "myanmar",
            WritingSystem::Georgian => "georgian",
            WritingSystem::Khmer => "khmer",
            WritingSystem::SimplifiedChinese => "simplified-chinese",
            WritingSystem::TraditionalChinese => "traditional-chinese",
            WritingSystem::Japanese => "japanese",
            WritingSystem::Korean => "korean",
            WritingSystem::Vietnamese => "vietnamese",
            WritingSystem::Symbol => "symbol",
            WritingSystem::Ogham => "ogham",
            WritingSystem::Runic => "runic",
            WritingSystem::Nko => "nko",
        }
    }

    /// An untranslated English display name — a convenience for hosts that
    /// don't localize the writing-system list. The GUI layer normally maps
    /// [`id`](Self::id) to a translated label instead.
    pub const fn english_name(self) -> &'static str {
        match self {
            WritingSystem::Latin => "Latin",
            WritingSystem::Greek => "Greek",
            WritingSystem::Cyrillic => "Cyrillic",
            WritingSystem::Armenian => "Armenian",
            WritingSystem::Hebrew => "Hebrew",
            WritingSystem::Arabic => "Arabic",
            WritingSystem::Syriac => "Syriac",
            WritingSystem::Thaana => "Thaana",
            WritingSystem::Devanagari => "Devanagari",
            WritingSystem::Bengali => "Bengali",
            WritingSystem::Gurmukhi => "Gurmukhi",
            WritingSystem::Gujarati => "Gujarati",
            WritingSystem::Oriya => "Oriya",
            WritingSystem::Tamil => "Tamil",
            WritingSystem::Telugu => "Telugu",
            WritingSystem::Kannada => "Kannada",
            WritingSystem::Malayalam => "Malayalam",
            WritingSystem::Sinhala => "Sinhala",
            WritingSystem::Thai => "Thai",
            WritingSystem::Lao => "Lao",
            WritingSystem::Tibetan => "Tibetan",
            WritingSystem::Myanmar => "Myanmar",
            WritingSystem::Georgian => "Georgian",
            WritingSystem::Khmer => "Khmer",
            WritingSystem::SimplifiedChinese => "Simplified Chinese",
            WritingSystem::TraditionalChinese => "Traditional Chinese",
            WritingSystem::Japanese => "Japanese",
            WritingSystem::Korean => "Korean",
            WritingSystem::Vietnamese => "Vietnamese",
            WritingSystem::Symbol => "Symbol",
            WritingSystem::Ogham => "Ogham",
            WritingSystem::Runic => "Runic",
            WritingSystem::Nko => "N'Ko",
        }
    }

    /// A short sample string in this writing system, for previewing a font
    /// in a picker. Mirrors Qt's `QFontDatabase::writingSystemSample`.
    pub const fn sample_text(self) -> &'static str {
        match self {
            WritingSystem::Latin => "Aa Bb Yy Zz",
            WritingSystem::Greek => "Αα Ββ Γγ",
            WritingSystem::Cyrillic => "Аа Бб Вв",
            WritingSystem::Armenian => "Աա Բբ Գգ",
            WritingSystem::Hebrew => "אבּג",
            WritingSystem::Arabic => "أبجد",
            WritingSystem::Syriac => "ܐܒܓ",
            WritingSystem::Thaana => "ހށނ",
            WritingSystem::Devanagari => "अआइ",
            WritingSystem::Bengali => "অআই",
            WritingSystem::Gurmukhi => "ਅਆਇ",
            WritingSystem::Gujarati => "અઆઇ",
            WritingSystem::Oriya => "ଅଆଇ",
            WritingSystem::Tamil => "அஆஇ",
            WritingSystem::Telugu => "అఆఇ",
            WritingSystem::Kannada => "ಅಆಇ",
            WritingSystem::Malayalam => "അആഇ",
            WritingSystem::Sinhala => "අආඇ",
            WritingSystem::Thai => "กขค",
            WritingSystem::Lao => "ກຂຄ",
            WritingSystem::Tibetan => "ཀཁག",
            WritingSystem::Myanmar => "ကခဂ",
            WritingSystem::Georgian => "აბგ",
            WritingSystem::Khmer => "កខគ",
            WritingSystem::SimplifiedChinese => "汉字样本",
            WritingSystem::TraditionalChinese => "漢字樣本",
            WritingSystem::Japanese => "あア漢字",
            WritingSystem::Korean => "한국어",
            WritingSystem::Vietnamese => "Tiếng Việt",
            WritingSystem::Symbol => "★ ♦ ♣ ♠",
            WritingSystem::Ogham => "ᚁᚂᚃ",
            WritingSystem::Runic => "ᚠᚢᚦ",
            WritingSystem::Nko => "ߊߋߌ",
        }
    }
}

/// A compact set of [`WritingSystem`]s, backed by a `u128` bitset (33
/// variants ≪ 128 bits). Hand-rolled to avoid a `bitflags` dependency
/// (the workspace uses none).
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash)]
pub struct WritingSystemSet(u128);

impl WritingSystemSet {
    /// The empty set.
    pub const fn new() -> Self {
        Self(0)
    }

    /// Add a writing system.
    pub fn insert(&mut self, ws: WritingSystem) {
        self.0 |= 1u128 << (ws as u8);
    }

    /// True if `ws` is present.
    pub const fn contains(self, ws: WritingSystem) -> bool {
        self.0 & (1u128 << (ws as u8)) != 0
    }

    /// True if no writing system is present.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Number of writing systems present.
    pub const fn len(self) -> u32 {
        self.0.count_ones()
    }

    /// The union of two sets.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Iterate the present writing systems in [`WritingSystem::ALL`] order.
    pub fn iter(self) -> impl Iterator<Item = WritingSystem> {
        WritingSystem::ALL
            .into_iter()
            .filter(move |&ws| self.contains(ws))
    }
}

// OS/2 `ulUnicodeRange` bit → script-level writing system. Bit numbers per
// the Microsoft OS/2 spec (matches Qt's `requiredUnicodeBits`). CJK
// ideographs (bit 59) are handled specially below — the language cannot be
// read from the Unicode range alone.
const UNICODE_RANGE_BITS: &[(u8, WritingSystem)] = &[
    (0, WritingSystem::Latin),       // Basic Latin
    (7, WritingSystem::Greek),       // Greek and Coptic
    (9, WritingSystem::Cyrillic),    // Cyrillic
    (10, WritingSystem::Armenian),   // Armenian
    (11, WritingSystem::Hebrew),     // Hebrew
    (13, WritingSystem::Arabic),     // Arabic
    (14, WritingSystem::Nko),        // NKo
    (15, WritingSystem::Devanagari), // Devanagari
    (16, WritingSystem::Bengali),    // Bengali
    (17, WritingSystem::Gurmukhi),   // Gurmukhi
    (18, WritingSystem::Gujarati),   // Gujarati
    (19, WritingSystem::Oriya),      // Oriya
    (20, WritingSystem::Tamil),      // Tamil
    (21, WritingSystem::Telugu),     // Telugu
    (22, WritingSystem::Kannada),    // Kannada
    (23, WritingSystem::Malayalam),  // Malayalam
    (24, WritingSystem::Thai),       // Thai
    (25, WritingSystem::Lao),        // Lao
    (26, WritingSystem::Georgian),   // Georgian
    (28, WritingSystem::Korean),     // Hangul Jamo
    (49, WritingSystem::Japanese),   // Hiragana
    (50, WritingSystem::Japanese),   // Katakana
    (52, WritingSystem::Korean),     // Hangul Compatibility Jamo
    (56, WritingSystem::Korean),     // Hangul Syllables
    (70, WritingSystem::Tibetan),    // Tibetan
    (71, WritingSystem::Syriac),     // Syriac
    (72, WritingSystem::Thaana),     // Thaana
    (73, WritingSystem::Sinhala),    // Sinhala
    (74, WritingSystem::Myanmar),    // Myanmar
    (78, WritingSystem::Ogham),      // Ogham
    (79, WritingSystem::Runic),      // Runic
    (80, WritingSystem::Khmer),      // Khmer
];

// OS/2 `ulCodePageRange1` bit → writing system. The code-page word is the
// only reliable signal for the CJK-language + Vietnamese distinction.
const CODEPAGE_RANGE1_BITS: &[(u8, WritingSystem)] = &[
    (8, WritingSystem::Vietnamese),          // 1258 Vietnamese
    (16, WritingSystem::Thai),               // 874  Thai
    (17, WritingSystem::Japanese),           // 932  JIS/Japan
    (18, WritingSystem::SimplifiedChinese),  // 936  Chinese Simplified
    (19, WritingSystem::Korean),             // 949  Korean Wansung
    (20, WritingSystem::TraditionalChinese), // 950  Chinese Traditional
    (21, WritingSystem::Korean),             // 1361 Korean Johab
];

// cmap sample codepoints per writing system — the cross-check for fonts
// whose OS/2 ranges are absent or wrong. Every listed codepoint must be
// covered for the system to be asserted. Simplified/Traditional Chinese are
// intentionally absent (undecidable from codepoints) and handled via Han
// below.
const CMAP_PROBES: &[(WritingSystem, &[char])] = &[
    (WritingSystem::Latin, &['A', 'z']),
    (WritingSystem::Greek, &['Α', 'ω']),
    (WritingSystem::Cyrillic, &['А', 'я']),
    (WritingSystem::Armenian, &['Ա', 'ֆ']),
    (WritingSystem::Hebrew, &['א', 'ת']),
    (WritingSystem::Arabic, &['ا', 'ي']),
    (WritingSystem::Syriac, &['ܐ']),
    (WritingSystem::Thaana, &['ހ']),
    (WritingSystem::Devanagari, &['अ', 'ह']),
    (WritingSystem::Bengali, &['অ']),
    (WritingSystem::Gurmukhi, &['ਅ']),
    (WritingSystem::Gujarati, &['અ']),
    (WritingSystem::Oriya, &['ଅ']),
    (WritingSystem::Tamil, &['அ']),
    (WritingSystem::Telugu, &['అ']),
    (WritingSystem::Kannada, &['ಅ']),
    (WritingSystem::Malayalam, &['അ']),
    (WritingSystem::Sinhala, &['අ']),
    (WritingSystem::Thai, &['ก', 'ฮ']),
    (WritingSystem::Lao, &['ກ']),
    (WritingSystem::Tibetan, &['ཀ']),
    (WritingSystem::Myanmar, &['က']),
    (WritingSystem::Georgian, &['ა', 'ჰ']),
    (WritingSystem::Khmer, &['ក']),
    (WritingSystem::Japanese, &['あ', 'ア']),
    (WritingSystem::Korean, &['가', '한']),
    (WritingSystem::Vietnamese, &['ế', 'ệ']),
    (WritingSystem::Ogham, &['ᚁ']),
    (WritingSystem::Runic, &['ᚠ']),
    (WritingSystem::Nko, &['ߊ']),
];

/// OS/2 unicode-range bit 59 = CJK Unified Ideographs (Han).
const OS2_HAN_BIT: u8 = 59;

/// Classify the writing systems a single font face can render.
///
/// `bytes` are the whole font file; `index` selects the face within a font
/// collection (`.ttc`). Returns an empty set for unparsable data; returns
/// `{Symbol}` for a font that parses but covers none of the known scripts
/// (dingbat / icon fonts). Pure and `Send` — safe to call off-thread.
pub fn writing_systems_for_face(bytes: &[u8], index: u32) -> WritingSystemSet {
    let mut set = WritingSystemSet::new();
    let Ok(face) = Face::parse(bytes, index) else {
        return set;
    };

    // (a) OS/2 ulUnicodeRange → script-level systems.
    let unicode_bits = face.tables().os2.map(|t| t.unicode_ranges().0).unwrap_or(0);
    for &(bit, ws) in UNICODE_RANGE_BITS {
        if unicode_bits & (1u128 << bit) != 0 {
            set.insert(ws);
        }
    }
    let has_han = unicode_bits & (1u128 << OS2_HAN_BIT) != 0;

    // (b) OS/2 ulCodePageRange1 → CJK-language + Vietnamese distinction.
    // ttf-parser doesn't type the code-page word, so read it raw.
    if let Some(cp1) = raw_code_page_range1(&face) {
        for &(bit, ws) in CODEPAGE_RANGE1_BITS {
            if cp1 & (1u32 << bit) != 0 {
                set.insert(ws);
            }
        }
    }

    // Han coverage with no code-page disambiguation: Simplified and
    // Traditional share codepoints, so mark both (Qt behaves the same when
    // the code-page bits are silent).
    if has_han && !has_any_cjk_language(set) {
        set.insert(WritingSystem::SimplifiedChinese);
        set.insert(WritingSystem::TraditionalChinese);
    }

    // (c) cmap cross-check for scripts the OS/2 tables didn't assert.
    for &(ws, probes) in CMAP_PROBES {
        if !set.contains(ws) && probes.iter().all(|&c| face.glyph_index(c).is_some()) {
            set.insert(ws);
        }
    }
    // cmap fallback for Han when OS/2 was silent about it entirely.
    if !has_any_cjk_language(set) && ['中', '文'].iter().all(|&c| face.glyph_index(c).is_some()) {
        set.insert(WritingSystem::SimplifiedChinese);
        set.insert(WritingSystem::TraditionalChinese);
    }

    // A font that parses but covers none of the known scripts is a
    // symbol/dingbat font (Wingdings, icon fonts, …).
    if set.is_empty() {
        set.insert(WritingSystem::Symbol);
    }

    set
}

/// True if any CJK-language system (which the code-page bits distinguish)
/// is already present.
const fn has_any_cjk_language(set: WritingSystemSet) -> bool {
    set.contains(WritingSystem::SimplifiedChinese)
        || set.contains(WritingSystem::TraditionalChinese)
        || set.contains(WritingSystem::Japanese)
        || set.contains(WritingSystem::Korean)
}

/// Read OS/2 `ulCodePageRange1` (32 bits) from the raw table bytes.
///
/// The code-page ranges were added in OS/2 version 1 and sit at byte
/// offset 78. `ttf-parser` exposes the typed unicode ranges but not the
/// code-page word, so read it directly. Returns `None` when the table is
/// absent, too short, or version 0 (no code-page fields).
fn raw_code_page_range1(face: &Face) -> Option<u32> {
    let data = face.raw_face().table(Tag::from_bytes(b"OS/2"))?;
    // Need the version (offset 0) and the u32 at offset 78..82.
    if data.len() < 82 {
        return None;
    }
    let version = u16::from_be_bytes([data[0], data[1]]);
    if version < 1 {
        return None; // code-page ranges only exist from OS/2 v1.
    }
    Some(u32::from_be_bytes([data[78], data[79], data[80], data[81]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_insert_contains_iter() {
        let mut set = WritingSystemSet::new();
        assert!(set.is_empty());
        set.insert(WritingSystem::Latin);
        set.insert(WritingSystem::Arabic);
        assert!(set.contains(WritingSystem::Latin));
        assert!(set.contains(WritingSystem::Arabic));
        assert!(!set.contains(WritingSystem::Greek));
        assert_eq!(set.len(), 2);
        let collected: Vec<_> = set.iter().collect();
        assert_eq!(collected, vec![WritingSystem::Latin, WritingSystem::Arabic]);
    }

    #[test]
    fn all_ids_unique_and_stable() {
        let mut ids: Vec<&str> = WritingSystem::ALL.iter().map(|w| w.id()).collect();
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n, "writing-system ids must be unique");
        assert_eq!(n, 33);
    }

    #[test]
    fn every_system_has_a_sample() {
        for ws in WritingSystem::ALL {
            assert!(!ws.sample_text().is_empty(), "{ws:?} has no sample text");
            assert!(!ws.english_name().is_empty());
        }
    }
}
