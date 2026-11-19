//! `GlyphRendering::Crisp`: glyphs fitted to the pixel grid on both axes,
//! on whole device pixels, laid out with advances in whole device pixels.

mod helpers;
use helpers::{Typesetter, make_typesetter};

use text_typeset::atlas::rasterizer::{rasterize_glyph_crisp, rasterize_glyph_positioned};
use text_typeset::font::resolve::resolve_font;
use text_typeset::shaping::shaper::shape_text;
use text_typeset::{GlyphRendering, TextFormat};

const TEXT: &str = "Livre · Synopsis · Ouverts Résolus illi mimi sœurs étaient";

fn crisp_typesetter(scale: f32) -> Typesetter {
    let mut ts = make_typesetter();
    ts.set_scale_factor(scale);
    ts.service.set_glyph_rendering(GlyphRendering::Crisp);
    ts
}

fn format(size: f32) -> TextFormat {
    TextFormat {
        font_size: Some(size),
        ..Default::default()
    }
}

#[test]
fn natural_is_the_default_and_a_change_asks_for_a_relayout() {
    let mut ts = make_typesetter();
    assert_eq!(ts.service.glyph_rendering(), GlyphRendering::Natural);
    let generation = ts.service.scale_generation();
    ts.service.set_glyph_rendering(GlyphRendering::Natural);
    assert_eq!(
        ts.service.scale_generation(),
        generation,
        "no change, no relayout"
    );
    ts.service.set_glyph_rendering(GlyphRendering::Crisp);
    assert_eq!(ts.service.glyph_rendering(), GlyphRendering::Crisp);
    assert_ne!(
        ts.service.scale_generation(),
        generation,
        "the advances changed: every flow must be laid out again"
    );
}

#[test]
fn a_crisp_glyph_advances_by_whole_device_pixels() {
    for scale in [1.0_f32, 1.25, 1.5, 2.0] {
        let ts = crisp_typesetter(scale);
        let resolved = resolve_font(
            ts.font_registry(),
            None,
            None,
            None,
            None,
            Some(13),
            scale,
            1.0,
        )
        .unwrap();
        let run = shape_text(ts.font_registry(), &resolved, TEXT, 0).unwrap();
        for glyph in &run.glyphs {
            let device = glyph.x_advance * scale;
            assert!(
                (device - device.round()).abs() < 1e-3,
                "at scale {scale} glyph {} advances {device} device px",
                glyph.glyph_id
            );
        }
        let sum: f32 = run.glyphs.iter().map(|g| g.x_advance).sum();
        assert!((run.advance_width - sum).abs() < 1e-3);
    }
}

#[test]
fn a_natural_glyph_keeps_its_design_advance() {
    // The control: without the mode, advances are fractional.
    let ts = make_typesetter();
    let resolved = resolve_font(
        ts.font_registry(),
        None,
        None,
        None,
        None,
        Some(13),
        1.0,
        1.0,
    )
    .unwrap();
    let run = shape_text(ts.font_registry(), &resolved, TEXT, 0).unwrap();
    assert!(
        run.glyphs
            .iter()
            .any(|g| (g.x_advance - g.x_advance.round()).abs() > 0.01),
        "the text no longer has a fractional advance to fit"
    );
}

#[test]
fn a_crisp_quad_starts_on_a_whole_device_pixel_with_a_crisp_bitmap() {
    for scale in [1.0_f32, 1.5, 2.0] {
        let mut ts = crisp_typesetter(scale);
        let result = ts.layout_single_line(TEXT, &format(13.0), None);
        assert!(!result.glyphs.is_empty());
        for (quad, key) in result.glyphs.iter().zip(&result.glyph_keys) {
            let device = quad.screen[0] * scale;
            assert!(
                (device - device.round()).abs() < 1e-3,
                "at scale {scale} a quad starts at device x {device}"
            );
            assert!(
                key.crisp,
                "glyph {} drawn from a natural bitmap",
                key.glyph_id
            );
            assert_eq!(key.subpixel_x, 0);
        }
        let width_device = result.width * scale;
        assert!(
            (width_device - width_device.round()).abs() < 1e-3,
            "the line measures {width_device} device px"
        );
    }
}

#[test]
fn zoomed_text_stays_natural() {
    let mut ts = crisp_typesetter(1.0);
    ts.set_raster_scale(1.25);
    let result = ts.layout_single_line(TEXT, &format(13.0), None);
    assert!(result.glyph_keys.iter().all(|k| !k.crisp));
}

/// Coverage of the fullest column of a glyph image.
fn fullest_column(width: u32, data: &[u8]) -> u32 {
    (0..width)
        .map(|x| {
            data.iter()
                .skip(x as usize)
                .step_by(width as usize)
                .map(|&a| u32::from(a))
                .max()
                .unwrap_or(0)
        })
        .max()
        .unwrap_or(0)
}

#[test]
fn a_crisp_stem_fills_its_pixels() {
    let ts = make_typesetter();
    let resolved = resolve_font(
        ts.font_registry(),
        None,
        None,
        None,
        None,
        Some(13),
        1.0,
        1.0,
    )
    .unwrap();
    let entry = ts.font_registry().get(resolved.font_face_id).unwrap();
    let mut scale_ctx = swash::scale::ScaleContext::new();
    for letter in ["l", "i", "I", "H", "n"] {
        let run = shape_text(ts.font_registry(), &resolved, letter, 0).unwrap();
        let glyph_id = run.glyphs[0].glyph_id;
        let crisp = rasterize_glyph_crisp(entry, glyph_id, 13.0, 400, false)
            .expect("a plain outline glyph is drawn crisp");
        assert!(
            fullest_column(crisp.width, &crisp.data) >= 250,
            "'{letter}': the crisp stem does not fill a column"
        );
        // The control, for the letters whose stem is under two pixels wide
        // at this size: the same glyph half a pixel off the grid, as natural
        // positioning can draw it, has no full column.
        if !matches!(letter, "l" | "i") {
            continue;
        }
        let natural = rasterize_glyph_positioned(
            &mut scale_ctx,
            entry.bytes(),
            entry.face_index,
            entry.swash_cache_key,
            glyph_id,
            13.0,
            400,
            true,
            false,
            0.5,
        )
        .unwrap();
        assert!(
            fullest_column(natural.width, &natural.data) < 250,
            "'{letter}': the control has a full column too, so this proves nothing"
        );
    }
}

#[test]
fn a_space_has_no_crisp_bitmap() {
    let ts = make_typesetter();
    let resolved = resolve_font(
        ts.font_registry(),
        None,
        None,
        None,
        None,
        Some(13),
        1.0,
        1.0,
    )
    .unwrap();
    let entry = ts.font_registry().get(resolved.font_face_id).unwrap();
    let run = shape_text(ts.font_registry(), &resolved, " ", 0).unwrap();
    assert!(rasterize_glyph_crisp(entry, run.glyphs[0].glyph_id, 13.0, 400, false).is_none());
}

#[test]
fn a_crisp_glyph_sits_where_the_natural_one_does() {
    // Grid fitting moves an edge by a pixel at most: a crisp glyph's top and
    // bottom stay within a pixel of the natural glyph's, for letters with
    // and without ascenders and descenders.
    let ts = make_typesetter();
    let resolved = resolve_font(
        ts.font_registry(),
        None,
        None,
        None,
        None,
        Some(13),
        1.0,
        1.0,
    )
    .unwrap();
    let entry = ts.font_registry().get(resolved.font_face_id).unwrap();
    let mut scale_ctx = swash::scale::ScaleContext::new();
    for letter in ["o", "x", "l", "p", "g", "C", "É", "t", ","] {
        let run = shape_text(ts.font_registry(), &resolved, letter, 0).unwrap();
        let glyph_id = run.glyphs[0].glyph_id;
        let crisp = rasterize_glyph_crisp(entry, glyph_id, 13.0, 400, false).unwrap();
        let natural = rasterize_glyph_positioned(
            &mut scale_ctx,
            entry.bytes(),
            entry.face_index,
            entry.swash_cache_key,
            glyph_id,
            13.0,
            400,
            true,
            false,
            0.0,
        )
        .unwrap();
        let bottom = |top: i32, height: u32| top - height as i32;
        assert!(
            (crisp.placement_top - natural.placement_top).abs() <= 1,
            "'{letter}': crisp top {} vs natural {}",
            crisp.placement_top,
            natural.placement_top
        );
        assert!(
            (bottom(crisp.placement_top, crisp.height)
                - bottom(natural.placement_top, natural.height))
            .abs()
                <= 1,
            "'{letter}': the bottoms differ"
        );
        assert!(
            (crisp.placement_left - natural.placement_left).abs() <= 1,
            "'{letter}'"
        );
    }
}
