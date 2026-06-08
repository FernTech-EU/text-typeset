//! # text-typeset
//!
//! Turns rich text documents into GPU-ready glyph quads.
//!
//! Typesetting crate for the `text-document` ecosystem. Takes a rich
//! text document model (styled paragraphs, tables, lists, frames)
//! and produces positioned glyph quads, decoration rectangles, and a
//! glyph atlas texture that any GPU framework can render in a few
//! draw calls.
//!
//! ```text
//! text-document (model) --> text-typeset (shaping + layout) --> framework adapter (rendering)
//! ```
//!
//! # Architecture: shared service, owned flows
//!
//! text-typeset is split along the axis of "what is shareable":
//!
//! - [`TextFontService`] owns the font registry, the glyph atlas,
//!   the glyph cache, the `swash` scale context, and the HiDPI
//!   scale factor. It is the expensive-to-build, expensive-to-share
//!   part. Construct one per process (or one per window) and share
//!   it by reference across every widget that emits text.
//!
//! - [`DocumentFlow`] owns the per-widget view state: viewport,
//!   zoom, scroll offset, content-width mode, flow layout, cursor,
//!   and default colors. Every widget that shows a document holds
//!   its own `DocumentFlow`. Layout and render methods take the
//!   service by reference, so many flows can render into one
//!   shared atlas — which means one GPU upload per frame, one
//!   shaped glyph rasterized at most once, and no cross-widget
//!   contamination of viewport / zoom / scroll state.
//!
//! # Quick start
//!
//! ```rust,no_run
//! use text_typeset::{DocumentFlow, TextFontService};
//!
//! let mut service = TextFontService::new();
//! let face = service.register_font(include_bytes!("../test-fonts/NotoSans-Variable.ttf"));
//! service.set_default_font(face, 16.0);
//!
//! let mut flow = DocumentFlow::new();
//! flow.set_viewport(800.0, 600.0);
//!
//! # #[cfg(feature = "text-document")]
//! # {
//! let doc = text_document::TextDocument::new();
//! doc.set_plain_text("Hello, world!").unwrap();
//! flow.layout_full(&service, &doc.snapshot_flow());
//! # }
//!
//! let frame = flow.render(&mut service);
//! // frame.glyphs       -> glyph quads (textured rects from the shared atlas)
//! // frame.atlas_pixels -> RGBA texture to upload (or skip via service.atlas_pixels())
//! // frame.decorations  -> cursor, selection, underlines, borders
//! ```
//!
//! # Sharing between widgets
//!
//! Put the service behind whatever smart pointer the host framework
//! uses — an `Rc<RefCell<TextFontService>>` for single-threaded UIs,
//! a plain `&mut` in render loops that already have exclusive
//! access. Every widget owns its own `DocumentFlow` and calls
//! `flow.render(&mut *service.borrow_mut())` when it paints.
//!
//! Because the service does not store any per-widget state,
//! "widget A rendered last" cannot break widget B. A changes to
//! `set_viewport`, `set_zoom`, `set_scroll_offset`, or `set_cursor`
//! live on A's flow and never touch B's.
//!
//! # Features
//!
//! - `text-document` (default): enables [`bridge`] module and
//!   [`DocumentFlow::layout_full`] for direct integration with
//!   text-document's `FlowSnapshot`.

mod types;

pub mod atlas;
pub mod font;
pub mod layout;
mod render;
pub mod shaping;

#[cfg(feature = "text-document")]
pub mod bridge;

mod document_flow;
mod font_service;

// Public API
pub use layout::block::{PaintSpan, apply_paint_spans};
pub use layout::inline_markup::{InlineAttrs, InlineMarkup, InlineSpan};
pub use types::{
    BlockVisualInfo, CharacterGeometry, CursorAffinity, CursorDisplay, DecorationKind,
    DecorationRect, FontFaceId, FontFeature, GlyphQuad, HitRegion, HitTestResult, Hyphenation,
    ImageQuad, LaidOutSpan, LaidOutSpanKind, ParagraphResult, RenderFrame, SingleLineResult,
    TextFormat, UnderlineStyle, VerticalAlignment,
};

pub use document_flow::{ContentWidthMode, DocumentFlow, RelayoutError};
pub use font::SharedFontData;
pub use font_service::{AtlasSnapshot, TextFontService};
