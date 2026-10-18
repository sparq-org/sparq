//! `ClosureLimits::max_facts` bounds what a REFUSED closure HOLDS, not only what an
//! accepted one holds.
//!
//! The compiled evaluator inserted every fact the caller handed over into its store, and
//! copied the whole store into the round-0 delta, BEFORE the first round checked the fact
//! allowance. The verdict was the same either way — the round check refused the run — but
//! the peak the refused run held was set by the CALLER's input length, not by the
//! allowance: a caller could make a closure hold (and copy) an unbounded fact set under
//! any allowance at all. The allowance is now charged while the input is read, so the peak
//! is the allowance.
//!
//! That is a memory property, not an accept/reject one, so it is measured: a counting
//! global allocator records the peak this process holds while the refused run executes.
//! The input here is two million facts (~24 MB of ids, which the test itself allocates
//! before the call); holding them in the store's hash set, its list, its per-predicate
//! index and the round-0 copy costs on the order of a hundred megabytes, while the
//! allowance in force is a thousand facts.
//!
//! This test lives in its own test binary so no other test's allocations land in the
//! measurement.
#![warn(clippy::undocumented_unsafe_blocks)]

use sparq_core::dict::Id;
use sparq_reason::n3::compiled::{compile, eval_with_limits};
use sparq_reason::ClosureLimits;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Bytes this process currently holds through the global allocator.
static LIVE: AtomicUsize = AtomicUsize::new(0);
/// The high-water mark of `LIVE` since it was last reset.
static PEAK: AtomicUsize = AtomicUsize::new(0);

struct Counting;

// SAFETY: every method forwards to `System` with the caller's arguments unchanged, so
// `System` upholds the `GlobalAlloc` contract; the wrapper only updates byte counters.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the layout is the caller's, forwarded unchanged to the system allocator.
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            let live = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(live, Ordering::Relaxed);
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        // SAFETY: `ptr` came from this allocator with this layout.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: forwarded unchanged; `ptr`/`layout` are the caller's valid pair.
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
            let live = LIVE.fetch_add(new_size, Ordering::Relaxed) + new_size;
            PEAK.fetch_max(live, Ordering::Relaxed);
        }
        p
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

/// Facts the allowance bounds the refused run to.
const MAX_FACTS: usize = 1_000;
/// Facts the caller hands over.
const INPUT_FACTS: usize = 2_000_000;
/// What the refused run may hold on top of its input. Holding the input in the store (hash
/// set + list + per-predicate index) and copying it into the round-0 delta costs tens of
/// megabytes per copy at this input size, so this is a wide margin around the allowance's
/// own few tens of kilobytes and not a tuned number.
const HELD_CEILING: usize = 8 << 20;

#[test]
fn a_refused_input_is_not_held_whole() {
    // A rule set with no builtins and nothing to derive: the ids below need not name any
    // term in a dictionary, and the run's only work is reading the input.
    let rules = compile("@prefix : <http://ex/> . { ?x :never ?y } => { ?x :nothing ?y } .")
        .expect("compile");
    let mut dict = sparq_core::dict::Dict::new();

    // Distinct facts, built directly as ids so the measurement is about the evaluator's
    // own structures and not about interning two million IRIs.
    let facts: Vec<[Id; 3]> = (0..INPUT_FACTS as Id).map(|i| [i, 0, 0]).collect();

    let mut limits = ClosureLimits::default();
    limits.max_facts = MAX_FACTS;

    let before = LIVE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let err = eval_with_limits(&mut dict, &facts, &rules, limits)
        .expect_err("2,000,000 input facts under a 1,000-fact allowance must be refused");
    let held = PEAK.load(Ordering::Relaxed).saturating_sub(before);

    assert!(
        err.contains("forward closure limit exceeded"),
        "the refusal is the closure cut: {}",
        err
    );
    assert!(
        held < HELD_CEILING,
        "a refused run held {} bytes on top of its input — the allowance was {} facts, so \
         the input was being held whole rather than charged as it was read",
        held,
        MAX_FACTS
    );
    // The input itself is still alive, so the measurement really did span the run.
    assert_eq!(facts.len(), INPUT_FACTS);
}
