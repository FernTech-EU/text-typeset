# Streaming / virtualization — measured baseline

Captured **before** any change, on `feat/virtualized-layout` off `main` (`73154a7`), via
`cargo bench --bench virtualization`. These numbers justify every subsequent change; re-run the
same bench after each one and compare.

Fixture: a 65-char log line, laid out no-wrap (one block = one visual line), NotoSans 16px.

## 1. Cost of appending ONE line to an N-line document

| N | `full_relayout` (per line) | `incremental` (per line) | speedup |
|---:|---:|---:|---:|
| 1 000 | 10.9 ms | 10.4 µs | 1 046× |
| 10 000 | 113.0 ms | 9.7 µs | 11 655× |
| 100 000 | **1.167 s** | 10.0 µs | **117 007×** |

- `full_relayout` = `FlowLayout::layout_blocks(N)` — the path a consumer is forced onto today,
  because a block-count change invalidates the layout and there is no tail-append entry point.
  Clearly **O(N)**: ~10× cost per 10× lines. At 100 k lines, appending a single line costs
  **over a second**.
- `incremental` = `FlowLayout::add_block(1)` onto an already laid-out flow. **Flat ~10 µs
  regardless of N** — already O(1); that 10 µs is just the irreducible cost of shaping one line.
  Margin collapsing only inspects `flow_order.last()`, and the map/vec inserts are amortized O(1).

**Conclusion:** the O(1) primitive already exists and is fast. What is missing is only the
`DocumentFlow`-level entry point to reach it, plus a consumer that doesn't force a full relayout.

## 2. Cost of keeping N lines resident (scrolled to the tail, 800×600 viewport)

| N | render/frame | memory | bytes/line |
|---:|---:|---:|---:|
| 1 000 | 154.1 µs | 10.8 MB | 11 374 B |
| 10 000 | 162.6 µs | 68.1 MB | 7 139 B |
| 100 000 | 219.5 µs | **622.9 MB** | 6 531 B |

(`bytes/line` at 1 k is inflated by fixed overhead — font data and the glyph atlas. The *marginal*
cost is ≈ 6.5 KB per resident shaped line.)

- **Render is a non-issue.** 154 µs → 219 µs while N grows 100×: nearly flat. `build_render_frame`
  scans all of `flow_order` linearly, but a range comparison per item is trivial next to emitting
  the ~30 visible lines' glyphs. At 100 k that is ~1.3 % of a 16.7 ms frame budget.
  **`render/frame.rs` therefore needs no change.**
- **Memory is the binding constraint.** ≈ 6.5 KB per resident shaped line means a fully-resident
  100 k-line document costs **623 MB**. A bounded resident set is viable only to ~10 k lines
  (68 MB); it does **not** reach the 100 k+ target.

**Conclusion:** appending fast is necessary but not sufficient. Keeping every line *shaped* is what
explodes; only laying out a viewport-sized window (memory O(window), not O(N)) reaches the target.
Raw text is cheap by comparison — 65 B/line in the rope, ~6.5 MB at 100 k.

## What this implies for the design

1. `DocumentFlow::append_block` — a thin wrapper over the already-O(1) `FlowLayout::add_block`.
   Kills the 1.17 s/line cliff. Low risk.
2. `DocumentFlow::layout_window` (uniform row height, arithmetic `y = idx * row_height`) — required
   by the **memory** result, not by render. Keeps only a window shaped. This is the one genuinely
   new code path; it is additive and opt-in, and existing `layout_full`/`layout_blocks`/
   `relayout_block` are untouched, so `RichTextEditor` keeps its current behaviour exactly.
3. `DocumentFlow::remove_leading` — trims the resident window's front. Absolute `y` values of
   survivors are left untouched (the region above simply becomes empty and is scrolled past), so
   this is O(n_removed) plus a `Vec` memmove — no reshape, no y rewrite.
