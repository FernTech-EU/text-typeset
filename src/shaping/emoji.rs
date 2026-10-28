//! Which shaped clusters ask to be drawn as colour emoji.
//!
//! A character is drawn as an emoji or as text by its *presentation*
//! (UTS #51, section 4): a few hundred pictographs default to emoji
//! presentation (the `Emoji_Presentation` property), others default to
//! text and switch to emoji when followed by the emoji variation selector
//! U+FE0F, and U+FE0E asks for text whichever the default is.
//!
//! Glyph fallback only used to look for a font when the primary font had
//! no glyph at all. That leaves two cases drawn wrong: a pictograph the
//! primary font draws in outline (Inter has a U+2764 heart, and a glyph for
//! U+FE0F too, so the red heart U+2764 U+FE0F never reached fallback), and
//! one whose first fallback face is a monochrome font that happens to cover
//! it. The shaper asks [`cluster_wants_emoji`] for every cluster and treats
//! one that is not drawn in colour as needing fallback.

/// U+FE0E VARIATION SELECTOR-15: text presentation.
const TEXT_PRESENTATION_SELECTOR: char = '\u{FE0E}';
/// U+FE0F VARIATION SELECTOR-16: emoji presentation.
const EMOJI_PRESENTATION_SELECTOR: char = '\u{FE0F}';
/// U+20E3 COMBINING ENCLOSING KEYCAP, the end of a keycap sequence (a digit,
/// U+FE0F, U+20E3).
const COMBINING_ENCLOSING_KEYCAP: char = '\u{20E3}';

/// The lowest code point with `Emoji_Presentation`. A character below it
/// (a keycap base, a text-default symbol such as U+00A9) only asks for emoji
/// through U+FE0F or U+20E3 after it.
const FIRST_CANDIDATE: char = '\u{231A}';

/// `Emoji_Presentation=Yes`, from Unicode 16.0's `emoji-data.txt`, as
/// inclusive ranges in ascending order.
///
/// It covers the regional indicators (flags), the skin-tone modifiers and
/// the black flag that starts a tag sequence, so a flag, a modifier
/// sequence and a tag sequence are all recognised from their first
/// character or their modifier.
const EMOJI_PRESENTATION: &[(char, char)] = &[
    ('\u{231A}', '\u{231B}'),
    ('\u{23E9}', '\u{23EC}'),
    ('\u{23F0}', '\u{23F0}'),
    ('\u{23F3}', '\u{23F3}'),
    ('\u{25FD}', '\u{25FE}'),
    ('\u{2614}', '\u{2615}'),
    ('\u{2648}', '\u{2653}'),
    ('\u{267F}', '\u{267F}'),
    ('\u{2693}', '\u{2693}'),
    ('\u{26A1}', '\u{26A1}'),
    ('\u{26AA}', '\u{26AB}'),
    ('\u{26BD}', '\u{26BE}'),
    ('\u{26C4}', '\u{26C5}'),
    ('\u{26CE}', '\u{26CE}'),
    ('\u{26D4}', '\u{26D4}'),
    ('\u{26EA}', '\u{26EA}'),
    ('\u{26F2}', '\u{26F3}'),
    ('\u{26F5}', '\u{26F5}'),
    ('\u{26FA}', '\u{26FA}'),
    ('\u{26FD}', '\u{26FD}'),
    ('\u{2705}', '\u{2705}'),
    ('\u{270A}', '\u{270B}'),
    ('\u{2728}', '\u{2728}'),
    ('\u{274C}', '\u{274C}'),
    ('\u{274E}', '\u{274E}'),
    ('\u{2753}', '\u{2755}'),
    ('\u{2757}', '\u{2757}'),
    ('\u{2795}', '\u{2797}'),
    ('\u{27B0}', '\u{27B0}'),
    ('\u{27BF}', '\u{27BF}'),
    ('\u{2B1B}', '\u{2B1C}'),
    ('\u{2B50}', '\u{2B50}'),
    ('\u{2B55}', '\u{2B55}'),
    ('\u{1F004}', '\u{1F004}'),
    ('\u{1F0CF}', '\u{1F0CF}'),
    ('\u{1F18E}', '\u{1F18E}'),
    ('\u{1F191}', '\u{1F19A}'),
    ('\u{1F1E6}', '\u{1F1FF}'),
    ('\u{1F201}', '\u{1F201}'),
    ('\u{1F21A}', '\u{1F21A}'),
    ('\u{1F22F}', '\u{1F22F}'),
    ('\u{1F232}', '\u{1F236}'),
    ('\u{1F238}', '\u{1F23A}'),
    ('\u{1F250}', '\u{1F251}'),
    ('\u{1F300}', '\u{1F320}'),
    ('\u{1F32D}', '\u{1F335}'),
    ('\u{1F337}', '\u{1F37C}'),
    ('\u{1F37E}', '\u{1F393}'),
    ('\u{1F3A0}', '\u{1F3CA}'),
    ('\u{1F3CF}', '\u{1F3D3}'),
    ('\u{1F3E0}', '\u{1F3F0}'),
    ('\u{1F3F4}', '\u{1F3F4}'),
    ('\u{1F3F8}', '\u{1F43E}'),
    ('\u{1F440}', '\u{1F440}'),
    ('\u{1F442}', '\u{1F4FC}'),
    ('\u{1F4FF}', '\u{1F53D}'),
    ('\u{1F54B}', '\u{1F54E}'),
    ('\u{1F550}', '\u{1F567}'),
    ('\u{1F57A}', '\u{1F57A}'),
    ('\u{1F595}', '\u{1F596}'),
    ('\u{1F5A4}', '\u{1F5A4}'),
    ('\u{1F5FB}', '\u{1F64F}'),
    ('\u{1F680}', '\u{1F6C5}'),
    ('\u{1F6CC}', '\u{1F6CC}'),
    ('\u{1F6D0}', '\u{1F6D2}'),
    ('\u{1F6D5}', '\u{1F6D7}'),
    ('\u{1F6DC}', '\u{1F6DF}'),
    ('\u{1F6EB}', '\u{1F6EC}'),
    ('\u{1F6F4}', '\u{1F6FC}'),
    ('\u{1F7E0}', '\u{1F7EB}'),
    ('\u{1F7F0}', '\u{1F7F0}'),
    ('\u{1F90C}', '\u{1F93A}'),
    ('\u{1F93C}', '\u{1F945}'),
    ('\u{1F947}', '\u{1F9FF}'),
    ('\u{1FA70}', '\u{1FA7C}'),
    ('\u{1FA80}', '\u{1FA89}'),
    ('\u{1FA8F}', '\u{1FAC6}'),
    ('\u{1FACE}', '\u{1FADC}'),
    ('\u{1FADF}', '\u{1FAE9}'),
    ('\u{1FAF0}', '\u{1FAF8}'),
];

/// Whether `c` has the `Emoji_Presentation` property: drawn as an emoji
/// even with no variation selector after it.
pub(crate) fn has_emoji_presentation(c: char) -> bool {
    if c < FIRST_CANDIDATE {
        return false;
    }
    EMOJI_PRESENTATION
        .binary_search_by(|&(lo, hi)| {
            if hi < c {
                std::cmp::Ordering::Less
            } else if lo > c {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// Whether `text` could hold an emoji at all. A cheap test run before
/// shaped clusters are classified one by one, so that the Latin, CJK or
/// Arabic text that makes up nearly every run pays one scan and no lookup.
pub(crate) fn may_contain_emoji(text: &str) -> bool {
    text.chars()
        .any(|c| c >= FIRST_CANDIDATE || c == COMBINING_ENCLOSING_KEYCAP)
}

/// Whether the text of one shaped cluster asks for emoji presentation.
///
/// U+FE0E anywhere in it asks for text, and wins. Otherwise U+FE0F or a
/// keycap asks for emoji, and so does any character that defaults to it.
/// A cluster is a base character with whatever the shaper merged into it
/// (variation selectors, skin-tone modifiers, zero-width joiners and the
/// characters they join, keycaps, tags), so a whole sequence is classified
/// at once.
pub(crate) fn cluster_wants_emoji(cluster: &str) -> bool {
    let mut wants = false;
    for c in cluster.chars() {
        if c == TEXT_PRESENTATION_SELECTOR {
            return false;
        }
        if c == EMOJI_PRESENTATION_SELECTOR
            || c == COMBINING_ENCLOSING_KEYCAP
            || has_emoji_presentation(c)
        {
            wants = true;
        }
    }
    wants
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_sorted_and_disjoint() {
        for pair in EMOJI_PRESENTATION.windows(2) {
            assert!(pair[0].0 <= pair[0].1, "{:?} is not a range", pair[0]);
            assert!(
                pair[0].1 < pair[1].0,
                "{:?} and {:?} overlap",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn default_emoji_presentation_is_recognised() {
        for c in [
            '\u{1F600}',
            '\u{1F601}',
            '\u{1F62C}',
            '\u{231A}',
            '\u{1F1EF}',
            '\u{1F3FD}',
        ] {
            assert!(has_emoji_presentation(c), "U+{:04X}", c as u32);
        }
        for c in ['a', '\u{4E2D}', '\u{2764}', '\u{263A}', '1', '#'] {
            assert!(!has_emoji_presentation(c), "U+{:04X}", c as u32);
        }
    }

    #[test]
    fn a_selector_decides_presentation() {
        // A text-default heart: text alone, emoji with VS16.
        assert!(!cluster_wants_emoji("\u{2764}"));
        assert!(cluster_wants_emoji("\u{2764}\u{FE0F}"));
        // An emoji-default face: emoji alone, text with VS15.
        assert!(cluster_wants_emoji("\u{1F600}"));
        assert!(!cluster_wants_emoji("\u{1F600}\u{FE0E}"));
    }

    #[test]
    fn sequences_are_classified_whole() {
        // Keycap, with and without VS16.
        assert!(cluster_wants_emoji("1\u{FE0F}\u{20E3}"));
        assert!(cluster_wants_emoji("1\u{20E3}"));
        // Flag: two regional indicators.
        assert!(cluster_wants_emoji("\u{1F1EF}\u{1F1F5}"));
        // Skin-tone modifier sequence on a text-default base.
        assert!(cluster_wants_emoji("\u{270B}\u{1F3FD}"));
        // ZWJ sequence starting with a text-default heart.
        assert!(cluster_wants_emoji("\u{2764}\u{FE0F}\u{200D}\u{1F525}"));
    }

    #[test]
    fn plain_text_is_not_a_candidate() {
        assert!(!may_contain_emoji("Showcase"));
        assert!(!may_contain_emoji("\u{0643}\u{062A}\u{0628}"));
        assert!(may_contain_emoji("abc\u{1F600}"));
        assert!(may_contain_emoji("1\u{20E3}"));
        // CJK passes the cheap test, and the per-cluster test says no.
        assert!(may_contain_emoji("\u{4E2D}"));
        assert!(!cluster_wants_emoji("\u{4E2D}"));
    }
}
