// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 FernTech

//! Streaming-append / virtualization timing harness.
//!
//! Answers one question with numbers: **what does it cost to append one line
//! to a document that already has N lines?**
//!
//! * `full_relayout` — the path a consumer is forced onto today. A block-count
//!   change invalidates the layout, so the whole document is re-shaped via
//!   [`FlowLayout::layout_blocks`] (which `clear()`s and re-adds every block).
//!   Expected O(N) per appended line.
//! * `incremental_append` — [`FlowLayout::add_block`] onto an already laid-out
//!   flow. Expected O(1): margin collapsing only inspects `flow_order.last()`,
//!   and the block/flow_order inserts are amortized O(1).
//!
//! The ratio between the two is the headroom a tail-append API unlocks, and
//! the `per-line` column is what a `tail -f` viewer actually pays per line.
//!
//! Deliberately harness-free (`harness = false`, plain `main`) rather than
//! criterion: criterion sizes its own iteration counts, which here would
//! either grow the flow under test by millions of blocks (OOM) or spend the
//! entire budget on untimed rebuilds. A fixed, declared workload is both
//! reproducible and honest.
//!
//! Run with `cargo bench --bench virtualization`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use text_typeset::layout::block::BlockLayoutParams;
use text_typeset::layout::flow::FlowLayout;

#[path = "../tests/helpers.rs"]
mod helpers;

use helpers::{make_block, make_typesetter};

/// Counting allocator, so the resident cost of a laid-out flow is a measured
/// number rather than a guess. A scrollback-capped viewer keeps every line in
/// its cap resident, so "bytes per resident line" decides whether a bounded
/// resident set is viable or whether viewport-windowed layout is required.
struct Counting;

static ALLOCATED: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATED.fetch_add(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        ALLOCATED.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

fn live_bytes() -> usize {
    ALLOCATED.load(Ordering::Relaxed)
}

/// Layout width. Wide enough that the fixture line never wraps, so one block
/// is exactly one visual line — the invariant the uniform-height fast path
/// relies on.
const WIDTH: f32 = 2000.0;

/// A representative log line.
const LINE: &str = "2026-07-16T12:00:00Z INFO  worker: processed batch id=1234 in 42ms";

/// Document sizes to probe. The scaling across these is the whole point: a
/// flat `incremental_append` column and a rising `full_relayout` column is
/// the O(1)-vs-O(N) result.
const SIZES: [usize; 3] = [1_000, 10_000, 100_000];

/// Appends timed per repetition. Large enough to dwarf clock granularity,
/// small enough to keep the flow's growth (and memory) bounded.
const APPEND_BATCH: usize = 1_000;

/// Repetitions per measurement; the median is reported.
const REPS: usize = 5;

fn build_blocks(n: usize) -> Vec<BlockLayoutParams> {
    (0..n).map(|i| make_block(i + 1, LINE)).collect()
}

fn median(mut xs: Vec<Duration>) -> Duration {
    xs.sort_unstable();
    xs[xs.len() / 2]
}

fn main() {
    let ts = make_typesetter();
    let registry = ts.font_registry();

    println!();
    println!("text-typeset — cost of appending one line to an N-line document");
    println!("{:-<78}", "");
    println!(
        "{:>9}  {:>16}  {:>16}  {:>14}",
        "N", "full_relayout", "incremental", "speedup"
    );
    println!(
        "{:>9}  {:>16}  {:>16}  {:>14}",
        "", "(per line)", "(per line)", ""
    );
    println!("{:-<78}", "");

    for n in SIZES {
        let blocks = build_blocks(n);

        // ── full_relayout: re-shape all N blocks (today's per-append cost) ──
        let full = median(
            (0..REPS)
                .map(|_| {
                    let params = blocks.clone();
                    let mut flow = FlowLayout::new();
                    let start = Instant::now();
                    flow.layout_blocks(registry, params, WIDTH);
                    let elapsed = start.elapsed();
                    std::hint::black_box(flow.content_height);
                    elapsed
                })
                .collect(),
        );

        // ── incremental_append: add_block onto an existing N-block flow ──
        // The N-block flow build is setup and is NOT timed; only the
        // APPEND_BATCH tail appends are.
        let extra: Vec<BlockLayoutParams> = (0..APPEND_BATCH)
            .map(|i| make_block(n + 1 + i, LINE))
            .collect();
        let batch = median(
            (0..REPS)
                .map(|_| {
                    let mut flow = FlowLayout::new();
                    flow.layout_blocks(registry, blocks.clone(), WIDTH);
                    let start = Instant::now();
                    for params in &extra {
                        // The real tail-append entry point, base-overlay
                        // capture included — not the raw `add_block`, whose
                        // bookkeeping a bulk layout would finish afterwards.
                        flow.append_block(registry, params, WIDTH);
                    }
                    let elapsed = start.elapsed();
                    std::hint::black_box(flow.content_height);
                    elapsed
                })
                .collect(),
        );
        let per_append = batch / APPEND_BATCH as u32;

        let speedup = full.as_secs_f64() / per_append.as_secs_f64().max(f64::MIN_POSITIVE);
        println!(
            "{:>9}  {:>16}  {:>16}  {:>13.0}x",
            n,
            format!("{:?}", full),
            format!("{:?}", per_append),
            speedup
        );
    }

    println!("{:-<78}", "");
    println!(
        "full_relayout = FlowLayout::layout_blocks(N)  |  incremental = FlowLayout::append_block(1)"
    );
    println!();

    bench_resident_cost();
}

/// Cost of keeping N lines *resident* (laid out): per-frame render and memory.
///
/// This is the question that decides whether a scrollback-capped viewer can
/// simply keep its whole cap laid out — relying on the render pass's existing
/// viewport culling — or whether layout itself must be windowed to the
/// viewport. `build_render_frame` culls to the viewport but does so with a
/// linear scan of `flow_order`, so render is O(N) in *comparisons* even
/// though it only emits glyphs for visible lines.
fn bench_resident_cost() {
    println!("Cost of keeping N lines resident (scrolled to the tail, 800x600 viewport)");
    println!("{:-<78}", "");
    println!(
        "{:>9}  {:>14}  {:>14}  {:>16}",
        "N", "render/frame", "memory", "bytes/line"
    );
    println!("{:-<78}", "");

    for n in SIZES {
        let before = live_bytes();
        let mut ts = make_typesetter();
        ts.set_viewport(800.0, 600.0);
        ts.layout_blocks(build_blocks(n));

        // Scroll to the tail — the log-viewer steady state, and the worst case
        // for a linear cull scan that starts at flow_order[0].
        let tail = (ts.content_height() - 600.0).max(0.0);
        ts.set_scroll_offset(tail);

        // Warm the glyph atlas/caches so we time the cull+emit, not first-run
        // rasterization.
        let _ = ts.render();

        let render = median(
            (0..REPS)
                .map(|_| {
                    let start = Instant::now();
                    let frame = ts.render();
                    let elapsed = start.elapsed();
                    std::hint::black_box(frame.glyphs.len());
                    elapsed
                })
                .collect(),
        );

        let resident = live_bytes().saturating_sub(before);
        println!(
            "{:>9}  {:>14}  {:>14}  {:>16}",
            n,
            format!("{:?}", render),
            format!("{:.1} MB", resident as f64 / (1024.0 * 1024.0)),
            format!("{} B", resident / n.max(1))
        );
    }

    println!("{:-<78}", "");
    println!("render = DocumentFlow::render() with viewport culling over all N flow items");
    println!();
}
