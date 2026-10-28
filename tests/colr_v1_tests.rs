//! COLRv1 colour glyphs, painted by the rasterizer.
//!
//! Three faces:
//!
//! - `NotoColorEmoji-COLRv1-subset.ttf`: 😀 ❤ 👓 🌅 from Noto Color Emoji as
//!   Fedora ships it (radial gradients, transforms, a SrcIn composite). Its
//!   base glyphs have no outline: before the painter they rasterized to
//!   nothing.
//! - `COLRv1-test-glyphs.ttf`: Google's COLRv1 test font
//!   (googlefonts/color-fonts, Apache-2.0, see its LICENSE file), one glyph
//!   per paint feature. The colours checked below are the ones its glyphs
//!   are defined with.
//! - `COLRv1-Cycle-test.ttf`: generated with fontTools for this test. Its
//!   one colour glyph (U+E000) is two PaintColrGlyph pointing at each other,
//!   with no outline: a glyph the painter cannot draw.

use text_typeset::atlas::rasterizer::{GlyphImage, rasterize_glyph};

const NOTO_COLRV1: &[u8] = include_bytes!("../test-fonts/NotoColorEmoji-COLRv1-subset.ttf");
const TEST_GLYPHS: &[u8] = include_bytes!("../test-fonts/COLRv1-test-glyphs.ttf");
const CYCLE: &[u8] = include_bytes!("../test-fonts/COLRv1-Cycle-test.ttf");

/// Rasterize the glyph `ch` maps to at `size` pixels per em.
fn raster(font: &[u8], ch: char, size: f32) -> GlyphImage {
    let glyph = swash::FontRef::from_index(font, 0)
        .unwrap()
        .charmap()
        .map(ch);
    assert_ne!(glyph, 0, "U+{:04X} is in the face", ch as u32);
    let mut context = swash::scale::ScaleContext::new();
    rasterize_glyph(
        &mut context,
        font,
        0,
        swash::CacheKey::new(),
        glyph,
        size,
        400,
        true,
    )
    .expect("the glyph rasterizes")
}

/// The test font's glyphs are drawn on a 1000-unit em; rasterized at 100
/// pixels per em, a font unit is a tenth of a pixel.
const SIZE: f32 = 100.0;

/// The straight-alpha RGBA pixel under the font-unit point (`x`, `y`) of a
/// glyph rasterized at [`SIZE`] from the 1000-unit test font.
fn pixel(image: &GlyphImage, x: f32, y: f32) -> [u8; 4] {
    let scale = SIZE / 1000.0;
    let col = (x * scale).floor() as i32 - image.placement_left;
    let row = image.placement_top - (y * scale).ceil() as i32;
    assert!(
        (0..image.width as i32).contains(&col) && (0..image.height as i32).contains(&row),
        "({x}, {y}) is inside the {}x{} image",
        image.width,
        image.height
    );
    let at = (row as usize * image.width as usize + col as usize) * 4;
    image.data[at..at + 4].try_into().unwrap()
}

fn assert_rgba(image: &GlyphImage, x: f32, y: f32, want: [u8; 4], what: &str) {
    let got = pixel(image, x, y);
    let close = got.iter().zip(want).all(|(&g, w)| g.abs_diff(w) <= 24);
    assert!(close, "{what}: at ({x}, {y}) want {want:?}, got {got:?}");
}

/// A point at `degrees` counter-clockwise from the positive x axis (the
/// font's own orientation, y up), `radius` units from (`cx`, `cy`).
fn polar(cx: f32, cy: f32, radius: f32, degrees: f32) -> (f32, f32) {
    let r = degrees.to_radians();
    (cx + radius * r.cos(), cy + radius * r.sin())
}

#[test]
fn noto_colrv1_emoji_are_painted_in_colour() {
    for (ch, what) in [
        ('\u{1F600}', "grinning face"),
        ('\u{2764}', "heart"),
        ('\u{1F453}', "glasses, a SrcIn composite"),
        ('\u{1F305}', "sunrise, radial gradients"),
    ] {
        let image = raster(NOTO_COLRV1, ch, 64.0);
        assert!(image.is_color, "{what} is a colour image");
        let coloured = image
            .data
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| {
                p[3] > 200 && p[..3].iter().max().unwrap() - p[..3].iter().min().unwrap() > 60
            })
            .count();
        assert!(
            coloured * 4 > (image.width * image.height) as usize / 4,
            "{what}: {coloured} coloured pixels in {}x{}",
            image.width,
            image.height
        );
    }

    // The face is yellow and the heart red.
    let dominant = |ch: char, wanted: fn(&[u8]) -> bool| {
        let image = raster(NOTO_COLRV1, ch, 64.0);
        let opaque: Vec<&[u8; 4]> = image
            .data
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[3] > 200)
            .collect();
        opaque.iter().filter(|p| wanted(&p[..])).count() * 2 > opaque.len()
    };
    assert!(dominant('\u{1F600}', |p| p[0] > 200
        && p[1] > 120
        && p[2] < 90));
    assert!(dominant('\u{2764}', |p| p[0] > 180
        && p[1] < 90
        && p[2] < 90));
}

#[test]
fn a_colrv1_glyph_scales_with_its_size() {
    let small = raster(NOTO_COLRV1, '\u{1F600}', 32.0);
    let large = raster(NOTO_COLRV1, '\u{1F600}', 64.0);
    assert!(small.width.abs_diff(large.width / 2) <= 1);
    assert!(small.height.abs_diff(large.height / 2) <= 1);
}

#[test]
fn a_sweep_gradient_runs_counter_clockwise_from_its_start_angle() {
    // sweep_0_90_pad_narrow: centre (500, 600), 0 to 90 degrees, stops at
    // 0.25 linen, 0.42 blue, 0.58 red, 0.75 dark slate grey, padded.
    let image = raster(TEST_GLYPHS, '\u{F0202}', SIZE);
    let at = |degrees| polar(500.0, 600.0, 300.0, degrees);
    let (x, y) = at(10.0);
    assert_rgba(
        &image,
        x,
        y,
        [250, 240, 230, 255],
        "10 degrees, before the first stop",
    );
    let (x, y) = at(45.0);
    assert_rgba(
        &image,
        x,
        y,
        [128, 0, 128, 255],
        "45 degrees, between blue and red",
    );
    let (x, y) = at(180.0);
    assert_rgba(
        &image,
        x,
        y,
        [47, 79, 79, 255],
        "180 degrees, past the range",
    );
    // Clockwise from the start would put the colours below the axis.
    let (x, y) = at(-10.0);
    assert_rgba(
        &image,
        x,
        y,
        [47, 79, 79, 255],
        "350 degrees, past the range",
    );
}

#[test]
fn a_linear_gradient_pads_repeats_and_reflects() {
    // linear_gradient_extend_mode_*: green at x 0, white at 153.5, red at
    // 307, along x; the third point sets the direction.
    let pad = raster(TEST_GLYPHS, '\u{F0500}', SIZE);
    let repeat = raster(TEST_GLYPHS, '\u{F0501}', SIZE);
    let reflect = raster(TEST_GLYPHS, '\u{F0502}', SIZE);
    let (green, white, red) = ([0, 128, 0, 255], [255, 255, 255, 255], [255, 0, 0, 255]);
    for image in [&pad, &repeat, &reflect] {
        assert_rgba(image, 5.0, 500.0, green, "at the first stop");
        assert_rgba(image, 154.0, 500.0, white, "at the middle stop");
    }
    assert_rgba(&pad, 700.0, 500.0, red, "padded with the last stop");
    // x 461 is one length on: the middle stop again, either way.
    assert_rgba(&repeat, 461.0, 500.0, white, "repeated");
    assert_rgba(&reflect, 461.0, 500.0, white, "reflected");
    // x 595 is 1.94 lengths on: near red repeated, near green reflected.
    let near_red = [255, 32, 32, 255];
    let near_green = [32, 144, 32, 255];
    assert_rgba(&repeat, 595.0, 500.0, near_red, "repeated, near the end");
    assert_rgba(
        &reflect,
        595.0,
        500.0,
        near_green,
        "reflected, near the start",
    );
}

#[test]
fn a_radial_gradient_pads_and_reflects() {
    // radial_contained_gradient_extend_mode_*: centre (166, 768), radius 0
    // to 256, green, white at half, red.
    let pad = raster(TEST_GLYPHS, '\u{F0503}', SIZE);
    let reflect = raster(TEST_GLYPHS, '\u{F0505}', SIZE);
    assert_rgba(
        &pad,
        166.0 + 128.0,
        768.0,
        [255, 255, 255, 255],
        "at half the radius",
    );
    assert_rgba(
        &pad,
        166.0 + 384.0,
        768.0,
        [255, 0, 0, 255],
        "padded beyond it",
    );
    assert_rgba(
        &reflect,
        166.0 + 384.0,
        768.0,
        [255, 255, 255, 255],
        "reflected to half",
    );
    assert_rgba(
        &reflect,
        166.0 + 500.0,
        768.0,
        [24, 140, 24, 255],
        "reflected to the centre",
    );
}

#[test]
fn composite_modes_combine_the_two_layers() {
    // composite_*: an orange square (backdrop, 166..666 by 333..833) and a
    // light blue one (source, 333..833 by 166..666), overlapping on 333..666
    // both ways, over a black cross through (500, 500).
    let orange = [255, 220, 1, 255];
    let light_blue = [104, 199, 232, 255];
    let backdrop_alone = (250.0, 750.0);
    let source_alone = (750.0, 250.0);
    let alpha = |image: &GlyphImage, (x, y): (f32, f32)| pixel(image, x, y)[3];

    let src_in = raster(TEST_GLYPHS, '\u{F0A05}', SIZE);
    assert_rgba(&src_in, 500.0, 500.0, light_blue, "SrcIn, the overlap");
    assert_eq!(alpha(&src_in, source_alone), 0, "SrcIn, the source alone");
    assert_eq!(
        alpha(&src_in, backdrop_alone),
        0,
        "SrcIn, the backdrop alone"
    );

    let multiply = raster(TEST_GLYPHS, '\u{F0A17}', SIZE);
    let (bx, by) = backdrop_alone;
    let (sx, sy) = source_alone;
    assert_rgba(
        &multiply,
        500.0,
        500.0,
        [104, 172, 1, 255],
        "Multiply, the overlap",
    );
    assert_rgba(&multiply, bx, by, orange, "Multiply, the backdrop alone");
    assert_rgba(&multiply, sx, sy, light_blue, "Multiply, the source alone");

    let xor = raster(TEST_GLYPHS, '\u{F0A0B}', SIZE);
    assert_rgba(&xor, bx, by, orange, "Xor, the backdrop alone");
    assert_rgba(&xor, sx, sy, light_blue, "Xor, the source alone");
    assert_eq!(
        alpha(&xor, (620.0, 380.0)),
        0,
        "Xor, the overlap, off the cross"
    );
}

#[test]
fn a_transform_moves_the_paint_under_it() {
    // translate_200_200: a half-transparent blue cross, and the same cross
    // in 70% orange moved by (200, 200) under it (DestOver).
    let image = raster(TEST_GLYPHS, '\u{F0905}', SIZE);
    assert_rgba(
        &image,
        700.0,
        700.0,
        [255, 165, 0, 178],
        "the moved cross, alone",
    );
    assert_rgba(
        &image,
        300.0,
        500.0,
        [0, 0, 255, 128],
        "the cross in place, alone",
    );
    let both = pixel(&image, 700.0, 500.0);
    assert!(
        both[2] > both[0],
        "where both are, the backdrop is on top: {both:?}"
    );

    // Neither has a clip box: their extent is measured from the outlines
    // they fill. translate_0_0 is two crosses on 250..750;
    // scale_1.5_1.5_center_500.0_500.0 scales one of them to 125..875,
    // 12.5 to 87.5 pixels, which rounds out to 76.
    let upright = raster(TEST_GLYPHS, '\u{F0900}', SIZE);
    let scaled = raster(TEST_GLYPHS, '\u{F0301}', SIZE);
    assert_eq!((upright.width, upright.height), (50, 50));
    assert_eq!((scaled.width, scaled.height), (76, 76));
}

#[test]
fn a_clip_box_bounds_the_image() {
    // clip_box_center: clipped to 250..750 in both directions.
    let image = raster(TEST_GLYPHS, '\u{F0C04}', SIZE);
    assert_eq!((image.width, image.height), (50, 50));
    assert_eq!((image.placement_left, image.placement_top), (25, 75));
}

#[test]
fn the_text_colour_entry_is_painted_black() {
    // foreground_color_solid_alpha_1: palette entry 0xFFFF.
    let image = raster(TEST_GLYPHS, '\u{F0B06}', SIZE);
    let opaque: Vec<&[u8; 4]> = image
        .data
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[3] > 200)
        .collect();
    assert!(!opaque.is_empty());
    assert!(opaque.iter().all(|p| p[..3].iter().all(|&c| c < 24)));
}

#[test]
fn a_colrv1_glyph_the_painter_cannot_draw_is_a_missing_glyph_box() {
    let image = raster(CYCLE, '\u{E000}', SIZE);
    assert!(!image.is_color);
    let inked = image.data.iter().filter(|&&a| a > 128).count();
    assert!(inked > 50, "the box drew: {inked} pixels");
}
