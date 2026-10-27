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
//! The same bound is measured for the store-side walk over the same names: the lifecycle
//! REPLAY behind `PodStore::rematerialize_after_edit` announces every resource an edit brings
//! into being, and the caller owns each of those names, so one ADDED name of length L under D
//! containers costs up to L x D bytes there before the rebuild's own input assembly runs.
//!
//! These tests share one test binary (a process has one global allocator) and hold `SERIAL`
//! while they measure, so no other test's allocations land in a measurement.
#![warn(clippy::undocumented_unsafe_blocks)]

use sparq_core::Graph;
use sparq_solid::{default_auth_budget, is_auth_allowance_refusal, PodStore};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// Held by each test while it measures, so the measurements never overlap.
static SERIAL: Mutex<()> = Mutex::new(());

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
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
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
    eprintln!("resource enumeration allocation peak: {held}");

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

#[test]
fn a_deep_added_name_is_refused_without_replaying_its_chain() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
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
        .ensure_named(&term)
        .expect("store the graph");

    let start = LIVE.load(Ordering::Relaxed);
    PEAK.store(start, Ordering::Relaxed);
    let err = store
        .rematerialize_after_edit(&before, None, &[name.as_str()], false)
        .expect_err("a name past the caps must refuse the rebuild");
    let held = PEAK.load(Ordering::Relaxed).saturating_sub(start);
    eprintln!("resource replay allocation peak: {held}");

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

fn measured_refusal(mut store: PodStore, kind: sparq_solid::AuthAllowance, ceiling: usize) {
    let before = LIVE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let error = store.materialize_wac().expect_err("input must be refused");
    let peak = PEAK.load(Ordering::Relaxed).saturating_sub(before);
    eprintln!("{kind:?} allocation peak: {peak}");
    assert_eq!(
        sparq_solid::auth_allowance_refusal(&error),
        Some(kind),
        "{error}"
    );
    assert!(
        peak < ceiling,
        "refusal allocated {peak} bytes, ceiling {ceiling}"
    );
}

fn document_store(document: Graph, group: bool) -> PodStore {
    use oxrdf::{NamedNode, Term};
    let name = if group {
        "https://pod.ex/group"
    } else {
        "https://pod.ex/doc.acl"
    };
    let mut graph = Graph::new();
    graph
        .add_named(Term::NamedNode(NamedNode::new(name).unwrap()), document)
        .unwrap();
    if group {
        let acl = Graph::load_str(
            "@prefix acl: <http://www.w3.org/ns/auth/acl#> . <https://pod.ex/doc.acl#a> acl:accessTo <https://pod.ex/doc>; acl:mode acl:Read; acl:agentGroup <https://pod.ex/group#team> .",
            "turtle",
        ).unwrap();
        graph
            .add_named(
                NamedNode::new("https://pod.ex/doc.acl").unwrap().into(),
                acl,
            )
            .unwrap();
    }
    PodStore::new(graph)
}

fn stored_literal_case(group: bool) {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    use oxrdf::{Literal, NamedNode};
    {
        let mut dict = sparq_core::dict::Dict::new();
        let subject = if group {
            "https://pod.ex/group#team"
        } else {
            "https://pod.ex/doc.acl#a"
        };
        let s = dict.intern(&NamedNode::new(subject).unwrap().into());
        let p = dict.intern(&NamedNode::new("https://ex.dev/p").unwrap().into());
        let o = dict.intern(&Literal::new_simple_literal("x".repeat(64 << 20)).into());
        let document = Graph::from_parts(dict, vec![[s, p, o]]);
        measured_refusal(
            document_store(document, group),
            sparq_solid::AuthAllowance::InputTermBytes,
            2 << 20,
        );
    }
}

fn document_row_case(group: bool) {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    use oxrdf::NamedNode;
    use sparq_core::dict::{inline_id_of_int, Dict};
    {
        let mut dict = Dict::new();
        let subject = if group {
            "https://pod.ex/group#team"
        } else {
            "https://pod.ex/doc.acl#a"
        };
        let s = dict.intern(&NamedNode::new(subject).unwrap().into());
        let p = dict.intern(&NamedNode::new("https://ex.dev/p").unwrap().into());
        let rows = (0..5_000_000)
            .map(|n| [s, p, inline_id_of_int(n).unwrap()])
            .collect();
        let document = Graph::from_parts(dict, rows);
        measured_refusal(
            document_store(document, group),
            sparq_solid::AuthAllowance::InputBytes,
            8 << 20,
        );
    }
}

#[test]
fn skolem_expansion_is_charged_before_its_strings_are_allocated() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    use oxrdf::{BlankNode, Literal, NamedNode};
    let doc = format!("https://pod.ex/{}.acl", "a".repeat(2 << 10));
    let mut dict = sparq_core::dict::Dict::new();
    let p = dict.intern(&NamedNode::new("https://ex.dev/p").unwrap().into());
    let o = dict.intern(&Literal::new_simple_literal("x").into());
    let rows = (0..10_000)
        .map(|n| {
            [
                dict.intern(&BlankNode::new(format!("b{n}")).unwrap().into()),
                p,
                o,
            ]
        })
        .collect();
    let mut graph = Graph::new();
    graph
        .add_named(
            NamedNode::new(doc).unwrap().into(),
            Graph::from_parts(dict, rows),
        )
        .unwrap();
    let mut store = PodStore::new(graph);
    let mut budget = default_auth_budget();
    // Admit all stored terms; the refusal must account for the generated strings.
    budget.max_document_bytes = usize::MAX;
    budget.max_input_bytes = 4 << 20;
    budget.max_generated_bytes = 4 << 20;
    store.set_auth_budget(budget);
    measured_refusal(store, sparq_solid::AuthAllowance::GeneratedBytes, 8 << 20);
}

#[test]
fn stored_literal_is_refused_before_control_document_reconstruction() {
    stored_literal_case(false);
}
#[test]
fn stored_literal_is_refused_before_group_document_reconstruction() {
    stored_literal_case(true);
}
#[test]
fn row_cap_precedes_control_document_reservation() {
    document_row_case(false);
}
#[test]
fn row_cap_precedes_group_document_reservation() {
    document_row_case(true);
}
