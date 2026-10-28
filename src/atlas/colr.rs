//! Painting COLRv1 colour glyphs.
//!
//! swash, which rasterizes every other glyph, paints COLRv0 layers and
//! colour bitmaps but not COLRv1, the format of Noto Color Emoji as Fedora
//! ships it. Its base glyphs have no outline, so they came out of the
//! rasterizer as nothing.
//!
//! skrifa walks the paint graph: it resolves layers, nested colour glyphs,
//! variations and transforms, guards against cycles, and hands each
//! gradient over already normalized (a linear gradient's third point folded
//! into two, colour stops brought into 0..1 with the geometry adjusted to
//! match). tiny-skia paints what it hands over: glyph outlines and clip
//! boxes as clip masks, solid fills and linear, two-point conical and sweep
//! gradients with pad, repeat and reflect extension, and layers composited
//! with the 28 COLRv1 composite modes, which are Skia's blend modes. Skia's
//! own Fontations backend splits the work the same way.
//!
//! skrifa states its gradients for font units with y pointing down, which
//! is why, as its documentation says, its sweep angles run clockwise. Each
//! shader therefore gets the glyph's transform with a y flip in front of
//! it, and its points with y negated.

use skrifa::color::{Brush, ColorGlyphFormat, ColorPainter, ColorStop, CompositeMode, Extend};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::raw::TableProvider;
use skrifa::raw::types::BoundingBox;
use skrifa::{FontRef, GlyphId, MetadataProvider};
use tiny_skia::{
    BlendMode, Color, FillRule, GradientStop, LinearGradient, Mask, Paint, Path, PathBuilder,
    Pixmap, PixmapPaint, Point, RadialGradient, Rect, Shader, SpreadMode, SweepGradient, Transform,
};

use crate::atlas::rasterizer::GlyphImage;

/// The largest side, in pixels, a painted glyph may have.
const MAX_SIDE: u32 = 4096;

/// The colour a palette entry 0xFFFF (the text colour) is painted in. The
/// glyph cache holds one bitmap for every text colour, as it does for
/// COLRv0 glyphs, so this is a fixed colour.
const FOREGROUND: [u8; 4] = [0, 0, 0, 255];

/// Whether glyph `glyph_id` of the face has a COLRv1 paint.
pub(crate) fn has_colr_v1(font_data: &[u8], face_index: u32, glyph_id: u16) -> bool {
    FontRef::from_index(font_data, face_index).is_ok_and(|font| {
        font.color_glyphs()
            .get_with_format(GlyphId::new(glyph_id.into()), ColorGlyphFormat::ColrV1)
            .is_some()
    })
}

/// Paint glyph `glyph_id`'s COLRv1 paint at `size_px` (pixels per em), as
/// straight-alpha RGBA, the format the atlas holds colour glyphs in.
///
/// `None` when the glyph has no COLRv1 paint, when the paint cannot be
/// drawn (a cycle of colour glyphs, malformed data), or when it covers no
/// area.
pub(crate) fn paint_colr_v1(
    font_data: &[u8],
    face_index: u32,
    glyph_id: u16,
    size_px: f32,
) -> Option<GlyphImage> {
    let font = FontRef::from_index(font_data, face_index).ok()?;
    let glyph = font
        .color_glyphs()
        .get_with_format(GlyphId::new(glyph_id.into()), ColorGlyphFormat::ColrV1)?;
    let upem = f32::from(font.head().ok()?.units_per_em());
    if upem <= 0.0 || size_px <= 0.0 || !size_px.is_finite() {
        return None;
    }
    let scale = size_px / upem;

    // The glyph's extent in font units: its clip box, or, for a font with no
    // clip list, the outlines its paint fills.
    let bounds = match glyph.bounding_box(LocationRef::default(), Size::unscaled()) {
        Some(clip_box) => clip_box,
        None => {
            let mut measure = BoundsPainter::new(&font);
            glyph.paint(LocationRef::default(), &mut measure).ok()?;
            measure.bounds?
        }
    };
    let left = (bounds.x_min * scale).floor();
    let right = (bounds.x_max * scale).ceil();
    let bottom = (bounds.y_min * scale).floor();
    let top = (bounds.y_max * scale).ceil();
    let width = (right - left) as i64;
    let height = (top - bottom) as i64;
    if width <= 0 || height <= 0 || width > MAX_SIDE as i64 || height > MAX_SIDE as i64 {
        return None;
    }
    let (width, height) = (width as u32, height as u32);

    // Font units, y up, to pixmap pixels, y down, the box's top-left corner
    // at the origin.
    let base = Transform::from_row(scale, 0.0, 0.0, -scale, -left, top);
    let mut painter = PixmapPainter::new(&font, width, height, base)?;
    glyph.paint(LocationRef::default(), &mut painter).ok()?;
    let pixmap = painter.finish()?;

    let mut data = Vec::with_capacity(pixmap.data().len());
    for pixel in pixmap.pixels() {
        let c = pixel.demultiply();
        data.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    Some(GlyphImage {
        width,
        height,
        placement_left: left as i32,
        placement_top: top as i32,
        data,
        is_color: true,
    })
}

/// A glyph outline in font units.
fn glyph_path(font: &FontRef<'_>, glyph_id: GlyphId) -> Option<Path> {
    struct Pen(PathBuilder);
    impl OutlinePen for Pen {
        fn move_to(&mut self, x: f32, y: f32) {
            self.0.move_to(x, y);
        }
        fn line_to(&mut self, x: f32, y: f32) {
            self.0.line_to(x, y);
        }
        fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
            self.0.quad_to(cx0, cy0, x, y);
        }
        fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
            self.0.cubic_to(cx0, cy0, cx1, cy1, x, y);
        }
        fn close(&mut self) {
            self.0.close();
        }
    }
    let outline = font.outline_glyphs().get(glyph_id)?;
    let mut pen = Pen(PathBuilder::new());
    outline
        .draw(
            DrawSettings::unhinted(Size::unscaled(), LocationRef::default()),
            &mut pen,
        )
        .ok()?;
    pen.0.finish()
}

fn box_path(clip_box: BoundingBox<f32>) -> Option<Path> {
    Rect::from_ltrb(
        clip_box.x_min,
        clip_box.y_min,
        clip_box.x_max,
        clip_box.y_max,
    )
    .map(PathBuilder::from_rect)
}

fn to_transform(t: skrifa::color::Transform) -> Transform {
    Transform::from_row(t.xx, t.yx, t.xy, t.yy, t.dx, t.dy)
}

fn blend_mode(mode: CompositeMode) -> BlendMode {
    match mode {
        CompositeMode::Clear => BlendMode::Clear,
        CompositeMode::Src => BlendMode::Source,
        CompositeMode::Dest => BlendMode::Destination,
        CompositeMode::SrcOver => BlendMode::SourceOver,
        CompositeMode::DestOver => BlendMode::DestinationOver,
        CompositeMode::SrcIn => BlendMode::SourceIn,
        CompositeMode::DestIn => BlendMode::DestinationIn,
        CompositeMode::SrcOut => BlendMode::SourceOut,
        CompositeMode::DestOut => BlendMode::DestinationOut,
        CompositeMode::SrcAtop => BlendMode::SourceAtop,
        CompositeMode::DestAtop => BlendMode::DestinationAtop,
        CompositeMode::Xor => BlendMode::Xor,
        CompositeMode::Plus => BlendMode::Plus,
        CompositeMode::Screen => BlendMode::Screen,
        CompositeMode::Overlay => BlendMode::Overlay,
        CompositeMode::Darken => BlendMode::Darken,
        CompositeMode::Lighten => BlendMode::Lighten,
        CompositeMode::ColorDodge => BlendMode::ColorDodge,
        CompositeMode::ColorBurn => BlendMode::ColorBurn,
        CompositeMode::HardLight => BlendMode::HardLight,
        CompositeMode::SoftLight => BlendMode::SoftLight,
        CompositeMode::Difference => BlendMode::Difference,
        CompositeMode::Exclusion => BlendMode::Exclusion,
        CompositeMode::Multiply => BlendMode::Multiply,
        CompositeMode::HslHue => BlendMode::Hue,
        CompositeMode::HslSaturation => BlendMode::Saturation,
        CompositeMode::HslColor => BlendMode::Color,
        CompositeMode::HslLuminosity => BlendMode::Luminosity,
        // A value newer than this code, or malformed: draw it plainly.
        _ => BlendMode::SourceOver,
    }
}

fn spread_mode(extend: Extend) -> SpreadMode {
    match extend {
        Extend::Repeat => SpreadMode::Repeat,
        Extend::Reflect => SpreadMode::Reflect,
        _ => SpreadMode::Pad,
    }
}

/// The painter that draws into a pixmap.
struct PixmapPainter<'a> {
    font: FontRef<'a>,
    palette: Vec<[u8; 4]>,
    width: u32,
    height: u32,
    /// Font units to pixmap pixels, innermost transform last.
    transforms: Vec<Transform>,
    /// The clip in force, innermost last. `None` at the bottom: no clip.
    clips: Vec<Option<Mask>>,
    /// Layers, the glyph's own canvas first; each pushed one with the mode
    /// it is composited down with.
    layers: Vec<(Pixmap, BlendMode)>,
    /// Set when a layer could not be made.
    failed: bool,
}

impl<'a> PixmapPainter<'a> {
    fn new(font: &FontRef<'a>, width: u32, height: u32, base: Transform) -> Option<Self> {
        let palette = font
            .color_palettes()
            .get(0)
            .map(|p| {
                p.colors()
                    .iter()
                    .map(|c| [c.red, c.green, c.blue, c.alpha])
                    .collect()
            })
            .unwrap_or_default();
        Some(Self {
            font: font.clone(),
            palette,
            width,
            height,
            transforms: vec![base],
            clips: vec![None],
            layers: vec![(Pixmap::new(width, height)?, BlendMode::SourceOver)],
            failed: false,
        })
    }

    fn finish(mut self) -> Option<Pixmap> {
        if self.failed {
            return None;
        }
        // A paint graph leaves its layers balanced; this only guards a
        // malformed one.
        while self.layers.len() > 1 {
            self.pop_layer();
        }
        self.layers.pop().map(|(pixmap, _)| pixmap)
    }

    fn transform(&self) -> Transform {
        self.transforms.last().copied().unwrap_or_default()
    }

    fn clip(&self) -> Option<&Mask> {
        self.clips.last().and_then(Option::as_ref)
    }

    fn color(&self, palette_index: u16, alpha: f32) -> Color {
        let [r, g, b, a] = if palette_index == 0xFFFF {
            FOREGROUND
        } else {
            self.palette
                .get(usize::from(palette_index))
                .copied()
                .unwrap_or(FOREGROUND)
        };
        let mut color = Color::from_rgba8(r, g, b, a);
        color.apply_opacity(alpha.clamp(0.0, 1.0));
        color
    }

    fn stops(&self, stops: &[ColorStop]) -> Vec<(f32, Color)> {
        stops
            .iter()
            .map(|s| (s.offset, self.color(s.palette_index, s.alpha)))
            .collect()
    }

    /// The shader for `brush`. `transform` maps font units (y up) to the
    /// space of the draw call that uses it: a shader is in the draw call's
    /// own space, and the call's transform applies to it as to the shape.
    fn shader(&self, brush: &Brush<'_>, transform: Transform) -> Option<Shader<'static>> {
        let transform = transform.pre_scale(1.0, -1.0);
        let flip = |p: skrifa::raw::types::Point<f32>| Point::from_xy(p.x, -p.y);
        match *brush {
            Brush::Solid {
                palette_index,
                alpha,
            } => Some(Shader::SolidColor(self.color(palette_index, alpha))),
            Brush::LinearGradient {
                p0,
                p1,
                color_stops,
                extend,
            } => match color_stops {
                [] => None,
                [only] => Some(Shader::SolidColor(
                    self.color(only.palette_index, only.alpha),
                )),
                _ => LinearGradient::new(
                    flip(p0),
                    flip(p1),
                    gradient_stops(self.stops(color_stops)),
                    spread_mode(extend),
                    transform,
                ),
            },
            Brush::RadialGradient {
                c0,
                r0,
                c1,
                r1,
                color_stops,
                extend,
            } => {
                let (c0, r0, c1, r1, stops) =
                    truncate_negative_radius(c0, r0, c1, r1, self.stops(color_stops))?;
                match stops.as_slice() {
                    [] => None,
                    [(_, only)] => Some(Shader::SolidColor(*only)),
                    _ => RadialGradient::new(
                        flip(c0),
                        r0,
                        flip(c1),
                        r1,
                        gradient_stops(stops),
                        spread_mode(extend),
                        transform,
                    ),
                }
            }
            Brush::SweepGradient {
                c0,
                start_angle,
                end_angle,
                color_stops,
                extend,
            } => match color_stops {
                [] => None,
                [only] => Some(Shader::SolidColor(
                    self.color(only.palette_index, only.alpha),
                )),
                _ => {
                    // tiny-skia applies the angular range only when its
                    // start is not 0 (`scale != 1 && bias != 0` where Skia
                    // has `||`), so a range starting at 0 would be drawn as
                    // though it ran to 360. A thousandth of a degree is no
                    // visible change and keeps the range.
                    let start_angle = if start_angle == 0.0 && end_angle < 360.0 {
                        1e-3
                    } else {
                        start_angle
                    };
                    SweepGradient::new(
                        flip(c0),
                        start_angle,
                        end_angle,
                        gradient_stops(self.stops(color_stops)),
                        spread_mode(extend),
                        transform,
                    )
                }
            },
        }
    }

    /// Fill `path` drawn under `transform` (font units to pixels), or with no
    /// path the whole layer (in pixels, `transform` then unused), inside the
    /// clip in force.
    fn fill_path(&mut self, path: Option<&Path>, transform: Transform, shader: Shader<'static>) {
        let paint = Paint {
            shader,
            blend_mode: BlendMode::SourceOver,
            anti_alias: true,
            ..Paint::default()
        };
        let clip = self.clips.last().cloned().flatten();
        let Some((layer, _)) = self.layers.last_mut() else {
            return;
        };
        match path {
            Some(path) => {
                layer.fill_path(path, &paint, FillRule::Winding, transform, clip.as_ref())
            }
            None => {
                if let Some(rect) = Rect::from_xywh(0.0, 0.0, self.width as f32, self.height as f32)
                {
                    layer.fill_rect(rect, &paint, transform, clip.as_ref());
                }
            }
        }
    }

    fn push_clip_path(&mut self, path: Option<Path>) {
        let transform = self.transform();
        let mask = match (self.clip().cloned(), path) {
            (_, None) => Mask::new(self.width, self.height),
            (None, Some(path)) => Mask::new(self.width, self.height).map(|mut mask| {
                mask.fill_path(&path, FillRule::Winding, true, transform);
                mask
            }),
            (Some(mut mask), Some(path)) => {
                mask.intersect_path(&path, FillRule::Winding, true, transform);
                Some(mask)
            }
        };
        self.clips.push(mask);
    }
}

fn gradient_stops(stops: Vec<(f32, Color)>) -> Vec<GradientStop> {
    stops
        .into_iter()
        .map(|(offset, color)| GradientStop::new(offset, color))
        .collect()
}

type Center = skrifa::raw::types::Point<f32>;

/// A radial gradient with its colour line cut where a radius would go
/// negative.
///
/// skrifa brings the colour stops into 0..1 and moves the circles to match,
/// which can leave a circle with a negative radius; it leaves the cut to the
/// client, which Skia makes this way: the circle moves to where the radius
/// is 0, the stops beyond that point are dropped, and the colour there is
/// interpolated. Both radii negative draws nothing.
#[allow(clippy::type_complexity)]
fn truncate_negative_radius(
    c0: Center,
    r0: f32,
    c1: Center,
    r1: f32,
    stops: Vec<(f32, Color)>,
) -> Option<(Center, f32, Center, f32, Vec<(f32, Color)>)> {
    if r0 >= 0.0 && r1 >= 0.0 {
        return Some((c0, r0, c1, r1, stops));
    }
    if r0 < 0.0 && r1 < 0.0 {
        return None;
    }
    // Where the radius is 0 along the colour line, strictly inside 0..1.
    let t = r0 / (r0 - r1);
    let at = c0 + (c1 - c0) * t;
    let zero = (t, color_at(&stops, t));
    if r0 < 0.0 {
        // Keep t..1, brought back to 0..1.
        let kept = std::iter::once(zero)
            .chain(stops.into_iter().filter(|(o, _)| *o > t))
            .map(|(o, c)| ((o - t) / (1.0 - t), c))
            .collect();
        Some((at, 0.0, c1, r1, kept))
    } else {
        // Keep 0..t, brought back to 0..1.
        let kept = stops
            .into_iter()
            .filter(|(o, _)| *o < t)
            .chain(std::iter::once(zero))
            .map(|(o, c)| (o / t, c))
            .collect();
        Some((c0, r0, at, 0.0, kept))
    }
}

/// The colour of a colour line at `t`, interpolated between the stops
/// around it (straight alpha, as tiny-skia interpolates).
fn color_at(stops: &[(f32, Color)], t: f32) -> Color {
    let Some(&(first_offset, first)) = stops.first() else {
        return Color::TRANSPARENT;
    };
    if t <= first_offset {
        return first;
    }
    for pair in stops.windows(2) {
        let ((o0, c0), (o1, c1)) = (pair[0], pair[1]);
        if t <= o1 {
            let f = if o1 > o0 { (t - o0) / (o1 - o0) } else { 1.0 };
            let mix = |a: f32, b: f32| a + (b - a) * f;
            return Color::from_rgba(
                mix(c0.red(), c1.red()),
                mix(c0.green(), c1.green()),
                mix(c0.blue(), c1.blue()),
                mix(c0.alpha(), c1.alpha()),
            )
            .unwrap_or(c1);
        }
    }
    stops.last().map_or(first, |&(_, c)| c)
}

impl ColorPainter for PixmapPainter<'_> {
    fn push_transform(&mut self, transform: skrifa::color::Transform) {
        let next = self.transform().pre_concat(to_transform(transform));
        self.transforms.push(next);
    }

    fn pop_transform(&mut self) {
        if self.transforms.len() > 1 {
            self.transforms.pop();
        }
    }

    fn push_clip_glyph(&mut self, glyph_id: GlyphId) {
        let path = glyph_path(&self.font, glyph_id);
        self.push_clip_path(path);
    }

    fn push_clip_box(&mut self, clip_box: BoundingBox<f32>) {
        self.push_clip_path(box_path(clip_box));
    }

    fn pop_clip(&mut self) {
        if self.clips.len() > 1 {
            self.clips.pop();
        }
    }

    fn fill(&mut self, brush: Brush<'_>) {
        // Fills the whole layer, in pixels: the shader takes the transform.
        let transform = self.transform();
        if let Some(shader) = self.shader(&brush, transform) {
            self.fill_path(None, Transform::identity(), shader);
        }
    }

    fn fill_glyph(
        &mut self,
        glyph_id: GlyphId,
        brush_transform: Option<skrifa::color::Transform>,
        brush: Brush<'_>,
    ) {
        // The outline is filled directly, anti-aliased, inside the clip in
        // force, under the glyph's transform, which tiny-skia applies to the
        // shader too: the shader takes only `brush_transform`.
        let transform = self.transform();
        let brush_space = brush_transform.map_or(Transform::identity(), to_transform);
        let Some(path) = glyph_path(&self.font, glyph_id) else {
            return;
        };
        if let Some(shader) = self.shader(&brush, brush_space) {
            self.fill_path(Some(&path), transform, shader);
        }
    }

    fn push_layer(&mut self, composite_mode: CompositeMode) {
        // A layer the size of the base one cannot fail to be made; if it did,
        // the layers would no longer pair up, and the glyph is not drawn.
        match Pixmap::new(self.width, self.height) {
            Some(pixmap) => self.layers.push((pixmap, blend_mode(composite_mode))),
            None => self.failed = true,
        }
    }

    fn pop_layer(&mut self) {
        if self.failed || self.layers.len() < 2 {
            return;
        }
        let clip = self.clips.last().cloned().flatten();
        let Some((top, mode)) = self.layers.pop() else {
            return;
        };
        if let Some((below, _)) = self.layers.last_mut() {
            below.draw_pixmap(
                0,
                0,
                top.as_ref(),
                &PixmapPaint {
                    blend_mode: mode,
                    ..PixmapPaint::default()
                },
                Transform::identity(),
                clip.as_ref(),
            );
        }
    }
}

/// A painter that only measures: the union of what the paint fills, for a
/// glyph with no clip box.
struct BoundsPainter<'a> {
    font: FontRef<'a>,
    transforms: Vec<Transform>,
    /// The bounds of the clip in force, in font units; `None` at the bottom
    /// for no clip.
    clips: Vec<Option<Rect>>,
    bounds: Option<BoundingBox<f32>>,
}

impl<'a> BoundsPainter<'a> {
    fn new(font: &FontRef<'a>) -> Self {
        Self {
            font: font.clone(),
            transforms: vec![Transform::identity()],
            clips: vec![None],
            bounds: None,
        }
    }

    fn transform(&self) -> Transform {
        self.transforms.last().copied().unwrap_or_default()
    }

    fn push_clip_bounds(&mut self, path: Option<Path>) {
        let transformed = path
            .and_then(|p| p.transform(self.transform()))
            .map(|p| p.bounds());
        let next = match (self.clips.last().copied().flatten(), transformed) {
            (_, None) => Rect::from_xywh(0.0, 0.0, 0.0, 0.0),
            (None, Some(rect)) => Some(rect),
            (Some(clip), Some(rect)) => clip.intersect(&rect),
        };
        self.clips
            .push(next.or_else(|| Rect::from_xywh(0.0, 0.0, 0.0, 0.0)));
    }

    fn add(&mut self, rect: Rect) {
        let b = BoundingBox {
            x_min: rect.left(),
            y_min: rect.top(),
            x_max: rect.right(),
            y_max: rect.bottom(),
        };
        self.bounds = Some(match self.bounds {
            None => b,
            Some(a) => BoundingBox {
                x_min: a.x_min.min(b.x_min),
                y_min: a.y_min.min(b.y_min),
                x_max: a.x_max.max(b.x_max),
                y_max: a.y_max.max(b.y_max),
            },
        });
    }
}

impl ColorPainter for BoundsPainter<'_> {
    fn push_transform(&mut self, transform: skrifa::color::Transform) {
        let next = self.transform().pre_concat(to_transform(transform));
        self.transforms.push(next);
    }

    fn pop_transform(&mut self) {
        if self.transforms.len() > 1 {
            self.transforms.pop();
        }
    }

    fn push_clip_glyph(&mut self, glyph_id: GlyphId) {
        let path = glyph_path(&self.font, glyph_id);
        self.push_clip_bounds(path);
    }

    fn push_clip_box(&mut self, clip_box: BoundingBox<f32>) {
        self.push_clip_bounds(box_path(clip_box));
    }

    fn pop_clip(&mut self) {
        if self.clips.len() > 1 {
            self.clips.pop();
        }
    }

    fn fill(&mut self, _brush: Brush<'_>) {
        // A fill covers the clip in force; with none it has no extent to
        // measure, and only a clip box would bound it.
        if let Some(rect) = self.clips.last().copied().flatten()
            && rect.width() > 0.0
            && rect.height() > 0.0
        {
            self.add(rect);
        }
    }

    fn push_layer(&mut self, _composite_mode: CompositeMode) {}
}
