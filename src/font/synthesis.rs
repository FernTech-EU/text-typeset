//! Synthetic bold and oblique, for a face without the weight or the slant
//! asked of it.
//!
//! Bold text can land on a face that has no bold: glyph fallback finds the
//! face of a family that matches the weight (see
//! [`FontRegistry::family_face_matching`](crate::font::registry::FontRegistry::family_face_matching)),
//! and when the family has one face, Arial Unicode MS on the Mac for every
//! CJK character, that face is regular. Native text stacks embolden such a
//! face, and so does this crate, the way FreeType's `FT_GlyphSlot_Embolden`
//! does it: the outline grows by a twenty-fourth of the em, and so does the
//! glyph's advance, so that the layout, the caret and hit testing measure the
//! glyph as it is drawn.
//!
//! The shaper widens the advances and the rasterizer emboldens the outline,
//! both asking [`FaceSynthesis::synthesizes_bold`] of the face (the shaper of
//! a copy cached on its font entry), so the two cannot disagree.
//!
//! Italic text likewise lands on a face with no italic: no CJK font has one.
//! The rasterizer then slants the outline, by fontconfig's synthetic oblique
//! shear, about the baseline. A slant moves no advance, so the shaper does
//! nothing for it and the caret and hit testing are as for upright text.

use ttf_parser::{Face, Tag};

/// The weight from which text is bold: CSS `bolder` and Blink's
/// `font-synthesis` threshold.
const BOLD_THRESHOLD: u16 = 600;

/// The shear of synthetic oblique: x moves by a fifth of y, about 11.3
/// degrees, fontconfig's default matrix for it.
pub(crate) const OBLIQUE_SHEAR: f32 = 0.2;

/// What decides synthetic bold and oblique of a face: its weight and how far
/// a weight axis takes it, and whether it is italic or can slant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FaceSynthesis {
    /// The OS/2 weight class.
    weight: u16,
    /// The top of the face's `wght` axis, if it has one.
    axis_max: Option<f32>,
    /// Whether the face is italic or oblique (its OS/2 selection, its slanted
    /// `post` angle).
    slanted: bool,
    /// Whether the face has a `slnt` or `ital` axis.
    slant_axis: bool,
}

impl FaceSynthesis {
    pub(crate) fn read(font_data: &[u8], face_index: u32) -> Self {
        match Face::parse(font_data, face_index) {
            Ok(face) => {
                let axis = |tag: &[u8; 4]| {
                    face.variation_axes()
                        .into_iter()
                        .find(|axis| axis.tag == Tag::from_bytes(tag))
                };
                Self {
                    weight: face.weight().to_number(),
                    axis_max: axis(b"wght").map(|axis| axis.max_value),
                    slanted: face.is_italic()
                        || face.is_oblique()
                        || face.italic_angle().abs() > f32::EPSILON,
                    slant_axis: axis(b"slnt").is_some() || axis(b"ital").is_some(),
                }
            }
            // Unreadable: treated as a face that has the weight and the slant.
            Err(_) => Self {
                weight: 1000,
                axis_max: None,
                slanted: true,
                slant_axis: false,
            },
        }
    }

    /// Whether `requested_weight` on this face is drawn with synthetic bold:
    /// the weight asks for bold, the face is lighter than bold, and it has no
    /// weight axis that reaches bold (a variable face draws the weight for
    /// real).
    pub(crate) fn synthesizes_bold(self, requested_weight: u16) -> bool {
        requested_weight >= BOLD_THRESHOLD
            && self.weight < BOLD_THRESHOLD
            && self
                .axis_max
                .is_none_or(|max| max < f32::from(BOLD_THRESHOLD))
    }

    /// Whether italic text on this face is drawn with synthetic oblique: the
    /// face is upright and has no axis to slant it.
    pub(crate) fn synthesizes_oblique(self, italic_requested: bool) -> bool {
        italic_requested && !self.slanted && !self.slant_axis
    }
}

/// How much synthetic bold widens a glyph's outline and advance, in the
/// units of `size_px` (pixels per em): a twenty-fourth of the em, as
/// FreeType.
pub(crate) fn embolden_strength(size_px: f32) -> f32 {
    size_px / 24.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const REGULAR_CJK: &[u8] = include_bytes!("../../test-fonts/NotoSansCJKjp-NoNi-subset.otf");
    const BOLD_CJK: &[u8] = include_bytes!("../../test-fonts/NotoSansCJKjp-Bold-NoNi-subset.otf");
    const VARIABLE: &[u8] = include_bytes!("../../test-fonts/NotoSans-Variable.ttf");
    const ITALIC: &[u8] = include_bytes!("../../test-fonts/NotoSans-Italic-subset.ttf");

    fn of(font: &[u8]) -> FaceSynthesis {
        FaceSynthesis::read(font, 0)
    }

    #[test]
    fn a_regular_face_asked_for_bold_is_emboldened() {
        assert!(of(REGULAR_CJK).synthesizes_bold(700));
        assert!(of(REGULAR_CJK).synthesizes_bold(600));
        assert!(!of(REGULAR_CJK).synthesizes_bold(400));
        assert!(!of(REGULAR_CJK).synthesizes_bold(500));
    }

    #[test]
    fn a_bold_face_or_a_weight_axis_draws_bold_for_real() {
        assert!(!of(BOLD_CJK).synthesizes_bold(700));
        assert!(!of(VARIABLE).synthesizes_bold(700));
    }

    #[test]
    fn an_upright_face_asked_for_italic_is_slanted() {
        assert!(of(REGULAR_CJK).synthesizes_oblique(true));
        assert!(!of(REGULAR_CJK).synthesizes_oblique(false));
    }

    #[test]
    fn an_italic_face_is_not_slanted_again() {
        assert!(!of(ITALIC).synthesizes_oblique(true));
    }
}
