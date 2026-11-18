//! Glyphs are rasterized at their fractional horizontal position, to the
//! nearest quarter of a pixel, and their quads start on a whole device pixel
//! (see `text_typeset::atlas::rasterizer::SUBPIXEL_BINS`).

mod helpers;
use helpers::{Typesetter, make_typesetter};

use text_typeset::TextFormat;
use text_typeset::atlas::rasterizer::{
    SUBPIXEL_BINS, rasterize_glyph_positioned, split_subpixel_x, subpixel_offset,
};
use text_typeset::font::resolve::resolve_font;
use text_typeset::shaping::shaper::shape_text;

/// Letters whose advances at 15 px leave a different fraction after each.
const TEXT: &str = "illi lil mimi sœurs étaient vêtues";

#[test]
fn a_position_splits_into_a_whole_pixel_and_the_nearest_quarter() {
    assert_eq!(SUBPIXEL_BINS, 4);
    assert_eq!(split_subpixel_x(3.0), (3.0, 0));
    assert_eq!(split_subpixel_x(3.1), (3.0, 0));
    assert_eq!(split_subpixel_x(3.13), (3.0, 1));
    assert_eq!(split_subpixel_x(3.5), (3.0, 2));
    assert_eq!(split_subpixel_x(3.74), (3.0, 3));
    assert_eq!(
        split_subpixel_x(3.9),
        (4.0, 0),
        "rounds up into the next pixel"
    );
    assert_eq!(split_subpixel_x(-0.3), (-1.0, 3));
    for bin in 0..SUBPIXEL_BINS {
        assert_eq!(subpixel_offset(bin), f32::from(bin) * 0.25);
    }
}

/// Where the ink of a glyph image sits, in device pixels from its origin,
/// weighting each pixel at its centre: not linear in a sub-pixel shift (it
/// jumps as an edge crosses a pixel boundary), but strictly increasing.
fn ink_centroid(left: i32, width: u32, data: &[u8]) -> f64 {
    let (mut sum, mut mass) = (0.0_f64, 0.0_f64);
    for (i, &a) in data.iter().enumerate() {
        let x = f64::from(i as u32 % width) + 0.5;
        sum += x * f64::from(a);
        mass += f64::from(a);
    }
    f64::from(left) + sum / mass
}

fn ink_total(data: &[u8]) -> f64 {
    data.iter().map(|&a| f64::from(a)).sum()
}

#[test]
fn each_bin_moves_the_same_ink_to_the_right() {
    let ts = make_typesetter();
    let resolved =
        resolve_font(ts.font_registry(), None, None, None, None, None, 1.0, 1.0).unwrap();
    let entry = ts.font_registry().get(resolved.font_face_id).unwrap();
    for letter in ["l", "o", "m", "e"] {
        let run = shape_text(ts.font_registry(), &resolved, letter, 0).unwrap();
        let glyph_id = run.glyphs[0].glyph_id;
        let mut scale_ctx = swash::scale::ScaleContext::new();
        let mut raster = |x_offset: f32| {
            rasterize_glyph_positioned(
                &mut scale_ctx,
                entry.bytes(),
                entry.face_index,
                entry.swash_cache_key,
                glyph_id,
                15.0,
                400,
                true,
                false,
                x_offset,
            )
            .unwrap()
        };
        let images: Vec<_> = (0..SUBPIXEL_BINS)
            .map(|bin| raster(subpixel_offset(bin)))
            .collect();
        let base = &images[0];
        let mass = ink_total(&base.data);
        let mut last = ink_centroid(base.placement_left, base.width, &base.data);
        for (bin, image) in images.iter().enumerate().skip(1) {
            let ink = ink_total(&image.data);
            assert!(
                (ink - mass).abs() / mass < 0.01,
                "'{letter}' bin {bin}: {ink} of ink where bin 0 has {mass}: a shift, \
                 not a different shape"
            );
            let centroid = ink_centroid(image.placement_left, image.width, &image.data);
            assert!(
                centroid > last,
                "'{letter}' bin {bin}: the ink did not move right ({last} -> {centroid})"
            );
            last = centroid;
        }
        // A whole pixel of offset is bin 0 one pixel to the right.
        let whole = raster(1.0);
        assert_eq!(whole.placement_left, base.placement_left + 1, "'{letter}'");
        assert_eq!((whole.width, whole.height), (base.width, base.height));
        assert_eq!(whole.data, base.data, "'{letter}'");
    }
}

fn quads_and_bins(ts: &mut Typesetter) -> (Vec<f32>, Vec<u8>) {
    let format = TextFormat {
        font_size: Some(15.0),
        ..Default::default()
    };
    let result = ts.layout_single_line(TEXT, &format, None);
    let xs = result.glyphs.iter().map(|q| q.screen[0]).collect();
    let bins = result.glyph_keys.iter().map(|k| k.subpixel_x).collect();
    (xs, bins)
}

#[test]
fn a_hinted_glyph_quad_starts_on_a_whole_device_pixel() {
    for scale in [1.0_f32, 1.25, 1.5, 2.0] {
        let mut ts = make_typesetter();
        ts.set_scale_factor(scale);
        let (xs, bins) = quads_and_bins(&mut ts);
        for x in &xs {
            let device = x * scale;
            assert!(
                (device - device.round()).abs() < 1e-3,
                "at scale {scale} a quad starts at device x {device}, between two pixels"
            );
        }
        let distinct: std::collections::BTreeSet<u8> = bins.iter().copied().collect();
        assert!(
            distinct.len() > 1,
            "at scale {scale} every glyph fell in one bin ({distinct:?}): the text no \
             longer exercises fractional positions"
        );
    }
}

#[test]
fn a_scaled_raster_keeps_its_fractional_origin() {
    // Zoomed content is drawn under a transform the renderer does not snap;
    // its glyphs are rasterized at the whole pixel, as before.
    let mut ts = make_typesetter();
    ts.set_raster_scale(1.25);
    let (_, bins) = quads_and_bins(&mut ts);
    assert!(bins.iter().all(|&b| b == 0), "{bins:?}");
}
