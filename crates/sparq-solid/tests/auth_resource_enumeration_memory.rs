//! The resource ENUMERATION of an authorization materialization is bounded in MEMORY, not
//! only in its verdict.
//!
//! A resource of the auth view brings every container on its chain, and each container is an
//! owned prefix string, so one resource IRI of length L under D containers costs up to L x D
//! bytes. That cost is paid while the input is assembled — before any document is read and
//! before the assembled facts are counted — so the budgets checked after it bound the
//! verdict of a materialization over a long name while the enumeration has already done the
//! work. `AuthBudget::max_resource_iri_bytes` / `max_resource_path_depth` are charged
//! BEFORE the chain is walked, which makes the peak the budget rather than the name.
//!
//! That is a memory property, not an accept/reject one, so it is measured: a counting
//! global allocator records the peak this process holds while the refused materialization
//! runs. The name here is a single resource IRI ~32 KB long and ~16,000 containers deep,
//! whose chain is on the order of hundreds of megabytes, while the caps in force admit
//! 2 KiB and 64 containers.
//!
//! Which cap refuses this particular name is not what is measured (a chain of D containers
//! needs a name of at least 2D bytes, so a deep name is a long one too, and the aggregate
//! `max_input_bytes` charge would stop the walk even with both per-name caps gone): the
//! test asserts the refusal is a typed, fail-closed allowance refusal and that the peak is
//! the budget's order of magnitude rather than the name's. The kinds themselves are pinned
//! by `tests/auth_input_budget.rs`.
//!
//! This test lives in its own test binary so no other test's allocations land in the
//! measurement.
#![warn(clippy::undocumented_unsafe_blocks)]

use sparq_core::Graph;
use sparq_solid::{default_auth_budget, is_auth_allowance_refusal, PodStore};
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

/// Containers on the one resource IRI's chain.
const DEPTH: usize = 16_000;
/// What the refused materialization may hold on top of the dataset it was given. The chain
/// of this name is ~256 MB; the caps in force admit a 2 KiB name under 64 containers, whose
/// chain is at most 128 KiB, so this is a wide margin around the budget and not a tuned
/// number.
const HELD_CEILING: usize = 8 << 20;

#[test]
fn a_deep_resource_iri_is_refused_without_building_its_chain() {
    let resource = format!("https://pod.ex/{}doc", "a/".repeat(DEPTH));
    assert!(
        resource.len() > 4 * default_auth_budget().max_resource_iri_bytes,
        "the name is well past the caps in force"
    );
    let dataset = format!("<{resource}#it> <https://ex.dev/ns#k> \"v\" <{resource}> .\n");
    let mut store = PodStore::new(Graph::load_dataset(&dataset, "nquads").expect("parses"));

    let before = LIVE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let err = store
        .materialize_wac()
        .expect_err("a resource IRI past the caps must be refused");
    let held = PEAK.load(Ordering::Relaxed).saturating_sub(before);

    assert!(
        is_auth_allowance_refusal(&err),
        "the refusal is a typed budget refusal: {}",
        err
    );
    assert!(
        held < HELD_CEILING,
        "the refused materialization held {} bytes — the caps admit a 2 KiB name under {} \
         containers, so the container chain of this name was being built before it was \
         refused",
        held,
        default_auth_budget().max_resource_path_depth
    );
    // The dataset is still alive, so the measurement really did span the materialization.
    assert!(!dataset.is_empty());
}
