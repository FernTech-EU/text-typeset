//! Crisp glyphs: outlines grid-fitted on both axes and advances in whole
//! device pixels (see [`GlyphRendering::Crisp`]).
//!
//! The autohinter does the grid-fitting, for every font: the TrueType
//! interpreter, as FreeType's v40 one it follows, ignores most horizontal
//! instructions when the output is antialiased, so a font's own bytecode
//! would leave stems where they fall. The hinted advance it reports is what
//! the shaper gives each glyph, so line breaking, caret positions and hit
//! testing all see the widths that are drawn.

use std::collections::HashMap;
use std::sync::Mutex;

use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{
    DrawSettings, Engine, HintingInstance, HintingOptions, OutlinePen, SmoothMode, Target,
};
use skrifa::{FontRef, GlyphId, MetadataProvider};

/// How glyphs are fitted to the pixel grid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GlyphRendering {
    /// Glyphs keep the advances the font designs and are drawn at their
    /// position to a quarter of a pixel, hinted vertically only: browsers
    /// and Core Text draw text this way. Spacing is exact; a stem that falls
    /// between two pixels is drawn across both.
    #[default]
    Natural,
    /// Glyphs are drawn on whole device pixels, grid-fitted horizontally as
    /// well as vertically, with advances rounded to whole device pixels and
    /// laid out with those widths: the look of Windows' GDI-compatible
    /// rendering and of FreeType's full hinting. Stems are sharp and as dark
    /// as the pixel allows; widths and spacing move by up to a pixel per
    /// glyph from the font's design. Text drawn under a scale (zoomed
    /// content) stays [`Natural`](Self::Natural): a raster that is not drawn
    /// pixel for pixel cannot be fitted to the grid.
    Crisp,
}

/// How many (size, weight) instances one face keeps. Building one runs the
/// autohinter's per-font analysis, so it is kept; a document uses a handful.
const MAX_INSTANCES: usize = 8;

/// One face's crisp hinting state, held by its
/// [`FontEntry`](crate::font::registry::FontEntry).
#[derive(Default)]
pub(crate) struct HintCache {
    inner: Mutex<Vec<HintedSize>>,
}

struct HintedSize {
    ppem_bits: u32,
    weight: u16,
    instance: HintingInstance,
    /// Hinted advance in device pixels per glyph (`None`: the glyph has no
    /// outline the hinter could fit).
    advances: HashMap<u16, Option<f32>>,
    /// Linear (design) advance in device pixels per glyph.
    linear: HashMap<u16, Option<f32>>,
}

fn hinting_options() -> HintingOptions {
    HintingOptions {
        engine: Engine::Auto(None),
        target: Target::Smooth {
            mode: SmoothMode::Normal,
            symmetric_rendering: true,
            preserve_linear_metrics: false,
        },
    }
}

fn location(font: &FontRef<'_>, weight: u16) -> skrifa::instance::Location {
    font.axes().location([("wght", f32::from(weight))])
}

impl HintCache {
    /// Run `f` on the instance for `ppem` device pixels at `weight`, building
    /// it on first use. `None` when the face has no outlines to hint.
    fn with_size<R>(
        &self,
        font: &FontRef<'_>,
        ppem: f32,
        weight: u16,
        f: impl FnOnce(&mut HintedSize) -> R,
    ) -> Option<R> {
        let mut sizes = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let ppem_bits = ppem.to_bits();
        let index = match sizes
            .iter()
            .position(|s| s.ppem_bits == ppem_bits && s.weight == weight)
        {
            Some(index) => index,
            None => {
                let outlines = font.outline_glyphs();
                let location = location(font, weight);
                let instance = HintingInstance::new(
                    &outlines,
                    Size::new(ppem),
                    LocationRef::from(&location),
                    hinting_options(),
                )
                .ok()?;
                if sizes.len() >= MAX_INSTANCES {
                    sizes.remove(0);
                }
                sizes.push(HintedSize {
                    ppem_bits,
                    weight,
                    instance,
                    advances: HashMap::new(),
                    linear: HashMap::new(),
                });
                sizes.len() - 1
            }
        };
        Some(f(&mut sizes[index]))
    }

    /// The advance of `glyph_id` at `ppem` device pixels, hinted (whole
    /// pixels) and linear, both in device pixels.
    pub(crate) fn advances(
        &self,
        font_data: &[u8],
        face_index: u32,
        glyph_id: u16,
        ppem: f32,
        weight: u16,
    ) -> Option<(f32, f32)> {
        let font = FontRef::from_index(font_data, face_index).ok()?;
        self.with_size(&font, ppem, weight, |size| {
            let hinted = *size.advances.entry(glyph_id).or_insert_with(|| {
                let outlines = font.outline_glyphs();
                let glyph = outlines.get(GlyphId::new(u32::from(glyph_id)))?;
                let metrics = glyph
                    .draw(
                        DrawSettings::hinted(&size.instance, false),
                        &mut skrifa::outline::pen::NullPen,
                    )
                    .ok()?;
                metrics.advance_width
            });
            let linear = *size.linear.entry(glyph_id).or_insert_with(|| {
                let location = location(&font, weight);
                font.glyph_metrics(Size::new(ppem), LocationRef::from(&location))
                    .advance_width(GlyphId::new(u32::from(glyph_id)))
            });
            Some((hinted?, linear?))
        })
        .flatten()
    }

    /// Draw `glyph_id`'s hinted outline at `ppem` device pixels into `pen`
    /// (y up, origin on the baseline).
    pub(crate) fn draw(
        &self,
        font_data: &[u8],
        face_index: u32,
        glyph_id: u16,
        ppem: f32,
        weight: u16,
        pen: &mut impl OutlinePen,
    ) -> bool {
        let Ok(font) = FontRef::from_index(font_data, face_index) else {
            return false;
        };
        self.with_size(&font, ppem, weight, |size| {
            let outlines = font.outline_glyphs();
            outlines
                .get(GlyphId::new(u32::from(glyph_id)))
                .is_some_and(|glyph| {
                    glyph
                        .draw(DrawSettings::hinted(&size.instance, false), pen)
                        .is_ok()
                })
        })
        .unwrap_or(false)
    }
}

/// Fit a shaped glyph's advance and offset to whole device pixels: the
/// hinted advance plus whatever the shaper added to the design advance
/// (kerning, synthetic bold), rounded. `scale_factor` converts between the
/// shaper's logical pixels and device pixels.
pub(crate) fn crisp_advance(
    shaped: f32,
    hinted_device: f32,
    linear_device: f32,
    scale_factor: f32,
) -> f32 {
    let shaped_device = shaped * scale_factor;
    let adjustment = (shaped_device - linear_device).round();
    (hinted_device + adjustment).max(0.0) / scale_factor
}

/// Round a logical offset to whole device pixels.
pub(crate) fn crisp_offset(offset: f32, scale_factor: f32) -> f32 {
    (offset * scale_factor).round() / scale_factor
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_advance_keeps_its_kerning_rounded() {
        // Design 7.3 px, hinted 7, kerned by -0.6 px: 7 - 1 = 6.
        assert_eq!(crisp_advance(6.7, 7.0, 7.3, 1.0), 6.0);
        // No kerning: the hinted advance.
        assert_eq!(crisp_advance(7.3, 7.0, 7.3, 1.0), 7.0);
        // At 1.5x, logical 4.8 is 7.2 device px; hinted 7 → 7 / 1.5.
        assert!((crisp_advance(4.8, 7.0, 7.2, 1.5) - 7.0 / 1.5).abs() < 1e-6);
        // A zero-width mark stays zero.
        assert_eq!(crisp_advance(0.0, 0.0, 0.0, 1.0), 0.0);
    }

    #[test]
    fn an_offset_lands_on_a_device_pixel() {
        assert_eq!(crisp_offset(1.4, 1.0), 1.0);
        assert!((crisp_offset(1.4, 1.5) * 1.5 - 2.0).abs() < 1e-6);
    }
}
