[![crates.io](https://img.shields.io/crates/v/text-typeset?style=flat-square&logo=rust)](https://crates.io/crates/text-typeset)
[![API](https://docs.rs/text-typeset/badge.svg)](https://docs.rs/text-typeset)
![quality](https://img.shields.io/github/actions/workflow/status/jacquetc/text-typeset/ci.yml)
[![codecov](https://codecov.io/gh/jacquetc/text-typeset/branch/main/graph/badge.svg?token=AONY49DQM0)](https://codecov.io/gh/jacquetc/text-typeset)
[![license](https://img.shields.io/badge/license-MPL--2.0-blue?style=flat-square)](#license)

# text-typeset

Turns rich text documents into GPU-ready glyph quads.

Typesetting crate for the [text-document](https://github.com/jacquetc/text-document) ecosystem. Takes a rich text document model (styled paragraphs, tables, lists, frames) and produces positioned glyph quads, decoration rectangles, and a glyph atlas texture that any GPU framework can render in a few draw calls.

```text
text-document (model) --> text-typeset (shaping + layout) --> framework adapter (rendering)
```

## Architecture: shared service, owned flows

text-typeset is split along the axis of "what is shareable":

| Type | Lifetime | Owns | Shared across |
| --- | --- | --- | --- |
| [`TextFontService`] | one per process / window | font registry, glyph atlas, glyph cache, `swash` scale context, HiDPI scale factor | every widget that emits text |
| [`DocumentFlow`] | one per widget | viewport, zoom, scroll, wrap mode, flow layout, cursor, colors, rendered frame | nothing — strictly per-widget |

A `DocumentFlow` does not own font data. Every `layout_*` and `render` method takes a `TextFontService` reference and reads the font registry and glyph atlas through it. This makes it cheap to run many widgets against a single shared atlas — one GPU upload per frame, one shaped glyph rasterized at most once, and no cross-widget contamination of viewport / zoom / scroll state.

Put the service behind whatever smart pointer your host framework uses (an `Rc<RefCell<TextFontService>>` for single-threaded UIs, a plain `&mut` inside a render loop that already has exclusive access). Every widget owns its own `DocumentFlow` and calls `flow.render(&mut *service.borrow_mut())` when it paints. Because the service carries no per-widget state, changes that widget A makes to `set_viewport`, `set_zoom`, `set_scroll_offset`, or `set_cursor` live on A's flow and never touch B's.

## Features

- **Text shaping** via [rustybuzz](https://crates.io/crates/rustybuzz) (Rust port of HarfBuzz) with OpenType feature support
- **Font management** via [fontdb](https://crates.io/crates/fontdb) with CSS-spec font matching, generic family mapping, and glyph fallback
- **Glyph rasterization** via [swash](https://crates.io/crates/swash) with color emoji support (COLR/CBDT)
- **Glyph atlas** backed by [etagere](https://crates.io/crates/etagere) shelf packing with auto-grow and LRU eviction — shared across every widget through the `TextFontService`
- **Paragraph layout** with line breaking ([unicode-linebreak](https://crates.io/crates/unicode-linebreak), UAX #14), four alignment modes (left, right, center, justify), and first-line indent
- **BiDi text** analysis via [unicode-bidi](https://crates.io/crates/unicode-bidi) with per-run directional shaping
- **Tables** with column width distribution, cell layout, borders, and cell backgrounds
- **Lists** with marker rendering (bullet, decimal, alpha, roman) at configurable indent levels
- **Frames** with inline, float-left, float-right, and absolute positioning
- **Text decorations**: underline, strikeout, overline (from font metrics)
- **Letter spacing** and **word spacing**
- **Hit testing**: screen coordinates to document position with region detection (text, margin, link, image)
- **Cursor display** with caret rendering, blink support, multi-cursor, and selection painting with full-width line highlighting
- **Zoom**: display zoom (0.1x to 10x) without text reflow, scales all output coordinates — owned per-widget
- **Cell-level selection**: table cell highlighting via `selected_cells` in cursor display
- **Scrolling**: `ensure_caret_visible`, `scroll_to_position` (automatic 1/3-viewport placement), viewport culling
- **Incremental relayout**: update a single block without re-laying-out the entire document
- **Content width modes**: auto (follows viewport) or fixed (for page-like WYSIWYG layout)
- **HiDPI**: the service carries a single scale factor and bumps a `scale_generation` counter on every change; each flow stamps the counter during layout and exposes `layout_dirty_for_scale` so the framework can detect and act on HiDPI transitions without tracking them itself

## Framework-agnostic output

text-typeset produces a `RenderFrame` containing:

- **Glyph quads** (`[screen_rect, atlas_rect, color]`) drawn as textured rectangles from the atlas
- **Image quads** (position + resource name) for inline images loaded by the adapter
- **Decoration rects** (underline, strikeout, selection, cursor, table borders, backgrounds)
- **Atlas texture** (RGBA, updated incrementally)

The rendering contract is thin: "draw N sub-rects from a texture + M colored rects." Any framework that supports textured quads can serve as a backend: Godot (`draw_texture_rect_region`), Qt (`QPainter::drawImage`), wgpu, egui, iced.

## Quick start — single widget

```rust
use text_typeset::{DocumentFlow, TextFontService};

// 1. Build the shared service once and register fonts on it.
let mut service = TextFontService::new();
let font = service.register_font(include_bytes!("test-fonts/NotoSans-Variable.ttf"));
service.set_default_font(font, 16.0);

// 2. Build a per-widget flow and give it a viewport.
let mut flow = DocumentFlow::new();
flow.set_viewport(800.0, 600.0);

// 3. Lay out + render. Every layout / render method takes the
//    service by reference so the glyph atlas stays shared.
let doc = text_document::TextDocument::new();
doc.set_plain_text("Hello, world!").unwrap();
flow.layout_full(&service, &doc.snapshot_flow());

let frame = flow.render(&mut service);
// frame.glyphs       -> glyph quads (textured rects from the shared atlas)
// frame.atlas_pixels -> RGBA texture to upload (or read via service.atlas_pixels())
// frame.decorations  -> cursor, selection, underlines, borders
```

## Quick start — multiple widgets on one document

Two widgets — say, a live editor and a read-only preview — share a `TextDocument`, a `TextFontService`, and produce glyphs into the same atlas. Each owns its own `DocumentFlow` with its own viewport / zoom / scroll:

```rust
use std::cell::RefCell;
use std::rc::Rc;
use text_typeset::{DocumentFlow, TextFontService};

let service: Rc<RefCell<TextFontService>> = {
    let mut svc = TextFontService::new();
    let face = svc.register_font(include_bytes!("test-fonts/NotoSans-Variable.ttf"));
    svc.set_default_font(face, 16.0);
    Rc::new(RefCell::new(svc))
};

let mut editor_flow = DocumentFlow::new();
editor_flow.set_viewport(600.0, 400.0);

let mut preview_flow = DocumentFlow::new();
preview_flow.set_viewport(500.0, 400.0);
preview_flow.set_zoom(0.8); // zoom lives on the flow — only affects this widget

let doc = text_document::TextDocument::new();
doc.set_plain_text("Hello, world!").unwrap();

editor_flow.layout_full(&service.borrow(), &doc.snapshot_flow());
preview_flow.layout_full(&service.borrow(), &doc.snapshot_flow());

let _editor_frame = editor_flow.render(&mut service.borrow_mut());
let _preview_frame = preview_flow.render(&mut service.borrow_mut());
```

Neither `render` call mutates state that belongs to the other flow. The atlas, shaper cache, and font registry are shared; the viewport, scroll, zoom, cursor, and flow layout are not.

## Content width

By default, text wraps at the viewport width (web/editor style). For page-like layout:

```rust
// Fixed width: text wraps at 600px regardless of viewport size
flow.set_content_width(600.0);

// Back to auto: text reflows when viewport resizes
flow.set_content_width_auto();
```

## HiDPI

`set_scale_factor` lives on the service because it drives the physical ppem of every rasterized glyph — one atlas, one scale:

```rust
service.set_scale_factor(2.0); // clears the atlas + glyph cache, bumps scale_generation

// Every flow stamps the service's scale_generation at layout time.
// After a change, ask whether a flow is stale and relayout if so:
if editor_flow.layout_dirty_for_scale(&service.borrow()) {
    editor_flow.layout_full(&service.borrow(), &doc.snapshot_flow());
}
```

The service cannot reach into per-widget flows, so it cannot clear their layouts for you. The generation counter makes that invalidation observable; the caller decides what to do with it.

## Dependencies

| Crate | Version | Purpose |
| --- | --- | --- |
| rustybuzz | 0.20 | OpenType shaping |
| swash | 0.2 | Font metrics and glyph rasterization |
| fontdb | 0.23 | Font discovery and CSS-spec matching |
| etagere | 0.3 | Glyph atlas allocation (shelf packing) |
| unicode-linebreak | 0.1 | Line break opportunities (UAX #14) |
| unicode-bidi | 0.3 | Bidirectional text (UAX #9) |
| text-document | -- | Document model (optional, default feature) |

## License

Licensed under the Mozilla Public License, Version 2.0 ([LICENSE](LICENSE)).
