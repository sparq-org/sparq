//! The STORE-SIDE resource enumeration is bounded in MEMORY, not only in its verdict.
//!
//! The twin of `auth_resource_enumeration_memory.rs`, for the other walk over the same
//! names: the lifecycle REPLAY behind `PodStore::rematerialize_after_edit`. The replay
//! announces every resource an edit brings into being — the added graph and each container
//! above it — and the caller OWNS each of those names, so one added name of length L under D
//! containers costs up to L x D bytes there, paid before the rebuild's own input assembly
//! runs and therefore before the budgets checked inside it. The name is being ADDED, so this
//! is not bounded by what the store already holds.
//!
//! `AuthBudget::max_resource_iri_bytes` / `max_resource_path_depth` are charged BEFORE the
//! chain is walked and every announced name is charged against the aggregate
//! `max_input_bytes`, which makes the peak the budget rather than the name. That is a memory
//! property, not an accept/reject one, so it is measured: a counting global allocator records
//! the peak this process holds while the refused rebuild runs.
//!
//! This test lives in its own test binary so no other test's allocations land in the
//! measurement.

use sparq_core::Graph;
use sparq_solid::{default_auth_budget, is_auth_allowance_refusal, PodStore};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Bytes this process currently holds through the global allocator.
static LIVE: AtomicUsize = AtomicUsize::new(0);
/// The high-water mark of `LIVE` since it was last reset.
static PEAK: AtomicUsize = AtomicUsize::new(0);

struct Counting;

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

/// Containers on the one added resource IRI's chain.
const DEPTH: usize = 16_000;
/// What the refused rebuild may hold on top of the store it was given. The chain of this
/// name is ~256 MB; the caps in force admit a 2 KiB name under 64 containers, whose chain is
/// at most 128 KiB, so this is a wide margin around the budget and not a tuned number.
const HELD_CEILING: usize = 8 << 20;

#[test]
fn a_deep_added_name_is_refused_without_replaying_its_chain() {
    let mut store = PodStore::new(Graph::default());
    store.materialize_wac().expect("an empty pod materializes");
    let before = store.stored_graphs();

    let name = format!("https://pod.ex/{}doc", "a/".repeat(DEPTH));
    assert!(
        name.len() > 4 * default_auth_budget().max_resource_iri_bytes,
        "the name is well past the caps in force"
    );
    let term = oxrdf::Term::NamedNode(oxrdf::NamedNode::new(&name).expect("an IRI"));
    store
        .graph_unchecked_mut()
        .named
        .push((term, Graph::default()));

    let start = LIVE.load(Ordering::Relaxed);
    PEAK.store(start, Ordering::Relaxed);
    let err = store
        .rematerialize_after_edit(&before, None, &[name.as_str()], false)
        .expect_err("a name past the caps must refuse the rebuild");
    let held = PEAK.load(Ordering::Relaxed).saturating_sub(start);

    assert!(
        is_auth_allowance_refusal(&err),
        "the refusal is a typed budget refusal: {}",
        err
    );
    assert!(
        held < HELD_CEILING,
        "the refused rebuild held {} bytes — the caps admit a 2 KiB name under {} containers, \
         so this name's container chain was being replayed before it was refused",
        held,
        default_auth_budget().max_resource_path_depth
    );
    // The name is still alive, so the measurement really did span the rebuild.
    assert!(!name.is_empty());
}
