# Counting-window ownership: bounded read-only proposal

Confirmed: begin clears ACTIVE before rejection and leaves initialization unreserved; end releases ACTIVE before readout. Use a separate ownership AtomicBool: acquire by CAS before reset; keep existing ACTIVE boundaries; snapshot results before releasing ownership. Preserve the single-coordinator/quiescent-worker contract.

All current paths are single coordinator. concurrent-cold spawns and joins two readers inside one measure call; neither calls begin/end. Frozen data remain exact-old-head diagnostic evidence; no trigger for this ownership bug was found. Global allocator noise and prior limits remain.

A std-only actual-module rustc test is feasible. Keep clean calibrations separate from caught-panic/competing-begin state assertions, since panic/harness activity can allocate. The identical test against old source must compile and fail ACTIVE preservation. A serial active-contention test does not exercise the precise reset/readout race; review that ownership span explicitly. No existing detached-bench CI execution is claimed.

No edits, tests, builds, remeasurement or remote actions performed.

## Exact current review

````json
{
  "id": "PRR_kwDOSz3qKM8AAAABM1xJkQ",
  "state": "COMMENTED",
  "body": "### \ud83d\udd35 Needs a closer look\n\nA newly added bench-only allocator window (`bench/overlay-count/src/counting.rs`) has an incorrect `ACTIVE` state transition that can allow overlapping measurement windows.\n\n<details>\n<summary>Review details</summary>\n\n### Suppressed comments (1)\n\n**Previously missed (1)** \u2014 in code that hasn't changed since the last review.\n\n**bench/overlay-count/src/counting.rs:89**\n* `begin()` uses `ACTIVE.swap(false, \u2026)` which (a) clears the active flag before checking it (side effect on failure) and (b) doesn\u2019t actually prevent a second concurrent `begin()` from entering between the swap and the later `ACTIVE.store(true, \u2026)`. Even though this is a bench-only harness, it makes the allocator-window state machine incorrect and can undercount/overlap measurement windows.\n\n- **Files reviewed:** 14/15 changed files\n- **Comments generated:** 0 new\n- **Review effort level:** Lite\n</details>",
  "submittedAt": "2026-09-09T15:48:34Z",
  "commit": {
    "oid": "5757e70be09ed95bcbc2831cec7850fa29fdf620"
  },
  "author": {
    "login": "copilot-pull-request-reviewer"
  }
}
````

## Assessment and executable-test plan

````json
{
  "head": "5757e70be09ed95bcbc2831cec7850fa29fdf620",
  "assessment": "Confirmed bench-only window ownership defect. Minimal separate ownership guard is warranted; no evidence that the existing single-coordinator measurement paths trigger it. Read-only assessment only: no source edit, compile, test, benchmark, remote action or model call.",
  "mechanism": [
    "begin ACTIVE.swap(false) first changes state, then asserts on the old value. A nested/competing begin against an active window clears counting before panic. The failed attempt need not have touched counters directly to invalidate subsequent recording.",
    "Two callers can both observe false while either resets counters before ACTIVE.store(true). A mere ACTIVE compare_exchange(false,true) closes duplicate admission but activates counting before resets, changing the measured boundary.",
    "end currently clears ACTIVE before loading counters. Treating that as the admission lock lets a new begin reset counters during the preceding readout. Ownership must extend through the local snapshot of all five returned values."
  ],
  "caller_inventory": [
    {
      "where": "main.rs:101\u2013106 -> counting.rs:103\u2013118",
      "path": "main creates the global one-thread Rayon pool, then calls calibrate once before fixture/argument dispatch. calibrate has one balanced begin/end and direct128/64/256byte System-wrapper calibration; it is not called by worker closures."
    },
    {
      "where": "main.rs:118\u2013165, especially148\u2013153",
      "path": "The baseline count curve nests ordinary sequential loops over delta/workload/phase/repetition. Each sample creates/prewarms its graph and obtains heap/RSS before one begin, calls synchronous run, then end before result assertion/output. No nested begin and no thread calls to begin/end."
    },
    {
      "where": "lifecycle.rs:23\u201345; actual measure calls at145,269",
      "path": "measure is the only lifecycle window wrapper: begin -> synchronous FnOnce -> elapsed -> end -> RSS/record construction. Its two call sites are run_all and run_reads; no nested measure."
    },
    {
      "where": "lifecycle.rs:130\u2013210",
      "path": "run_all invokes measure serially for snapshot/fork-insert/fork-tombstone/inplace-insert/multi-cold/multi-warm/concurrent-cold. Ordinary cases synchronously apply/read/retain; emitting and retained-content verification happen outside every window."
    },
    {
      "where": "lifecycle.rs:150\u2013164",
      "path": "concurrent-cold creates two readers within one measure closure. Barrier(2) aligns their query starts; neither reader calls begin/end. Both JoinHandles are joined and scoped threads complete before the closure returns to end. Thread launch/join overhead is intentionally inside that one window."
    },
    {
      "where": "lifecycle.rs:229\u2013315",
      "path": "run_reads loops R={1,2,4,8,16}, repetitions and four retained generations serially; one measure per generation. It has no worker launch or window calls in run/query helpers."
    }
  ],
  "frozen_measurement_impact": {
    "conclusion": "The confirmed bad admission/readout interleavings require overlapping window coordinators or a nested begin. No such path is present in these callers, including concurrent-cold. Preserve all frozen results as exact-old-head diagnostic evidence; do not retroactively relabel them as measured with the repaired guard or claim the protocol was concurrency-safe.",
    "limits": "Source call-graph evidence is not a trace of every historical allocation or background scheduling event. The process-global wrapper can count unrelated in-process allocations; worker quiescence, successful calibration and no unexpected panic remain measurement preconditions. No raw matrix re-audit, rebuild or remeasurement was performed. This finding does not justify blanket invalidation or renewed timing sweeps.",
    "source_versions": {
      "acfa31cf52ec0d2641dd4925d5b4f094a0a531fe": {
        "counting_sha256": "25b3712e37c4142edac164b32ce47308123bac8201f8a3ff2e3078cd4b5416f2",
        "same_as_current": true
      },
      "cb638a42a54fc7c9e11e9101587910e668a5f92a": {
        "counting_sha256": "25b3712e37c4142edac164b32ce47308123bac8201f8a3ff2e3078cd4b5416f2",
        "same_as_current": true
      },
      "b86b5d5ad84bce900762defa094630eb358317a6": {
        "counting_sha256": "25b3712e37c4142edac164b32ce47308123bac8201f8a3ff2e3078cd4b5416f2",
        "same_as_current": true
      },
      "5757e70be09ed95bcbc2831cec7850fa29fdf620": {
        "counting_sha256": "25b3712e37c4142edac164b32ce47308123bac8201f8a3ff2e3078cd4b5416f2",
        "same_as_current": true
      }
    }
  },
  "minimal_fix_proposal": {
    "scope": "counting.rs ownership transition and one actual-module regression; optionally README command documentation. No production core change, new storage API, allocator hot-path change, Cargo dependency or gate adjustment.",
    "begin": "Introduce a separate static ownership AtomicBool, initiallyfalse. First compare_exchange(false,true,Acquire,Relaxed); failed acquisition panics outside the allocator without mutating ACTIVE/counters. Only after acquisition perform existing baseline/reset operations, then the existing ACTIVE.store(true,Relaxed). Remove ACTIVE.swap(false) from admission.",
    "end": "Keep ACTIVE.store(false,Relaxed) at the current boundary. Load the entire result tuple into a local value while ownership remains held, then ownership.store(false,Release), then return that local tuple. The acquire/release pair orders readout-before-next-reset ownership handoff; active allocation counters stay atomic and existing hot-path loads unchanged.",
    "preconditions": "Single successful coordinator, balanced begin/end by that coordinator, no worker allocation in reset/readout phases; work/threads must finish before end. This is not an owner-token API or a stop-the-world snapshot. An erroneous non-owner end or arbitrary allocator calls racing the boundary are not made sound by the flag. No lock or panic added inside GlobalAlloc methods.",
    "denied_begin_caveat": "Even with side-effect-free failed acquisition, assert/catch_unwind and panic payload handling may allocate while the legitimate window is active. Thus preserve logical ACTIVE/ownership state, not exact totals through an invalid begin; the invalid-call scenario is not a valid measurement."
  },
  "one_focused_regression_proposal": {
    "actual_module": "Embed a cfg(test) serial regression using the actual counting.rs implementation, then compile that exact file as a standalone std-only crate with pinned rustc --edition=2021 --test. It needs no sparq, Rayon, libc, Cargo resolution or optimized build. Alternatively a #[path] module import preserves its inner module docs and global allocator; avoid a copied state-machine implementation.",
    "command_template": "<installed-pinned-rustc> --edition=2021 --test bench/overlay-count/src/counting.rs -o <task-private>/counting-window-tests; <binary> --exact <single_test_name> --test-threads=1 --nocapture",
    "scenarios": [
      "Call existing calibrate for clean sequential windows, twice, with no diagnostic formatting inside the windows. Its exact expectation remains(2,1,448,320,baseline); do not replace it with a tolerance or implementation-mirroring count assertion.",
      "Set a no-allocation panic hook outside a window. Begin a legitimate window, catch a duplicate begin, record ACTIVE without formatting/asserting, then end and assert the recorded flag remained true. Drop panic payloads/restore hooks outside clean calibration. Do not compare the contaminated window totals.",
      "For competing admission, precreate a worker and synchronization before opening the owner window; release it only after the owner begin has returned. Worker attempts begin and catches rejection, then joins before end. Record ACTIVE and whether the worker was denied, close the owner window, assert outside it. This deterministically exercises competing ACTIVE-window admission; it is not a deterministic reset/readout interleaving test.",
      "Finish with another clean calibration to verify recovery and successive reset behavior. One serial test avoids parallel tests sharing the process-global counters/hook."
    ],
    "old_source_control": "Apply the identical test suffix to the exact old counting.rs source at5757 in a task-private copy. Tests must reference existing begin/end/calibrate/ACTIVE, not the new ownership variable, so this control compiles. The old source must fail the denied-begin ACTIVE preservation assertion; a compile failure or zero tests is not a killed control. Preserve exact old/fixed source, rustc argv/binary hashes, logs and exit/test counts.",
    "coverage_limits": "This minimal serial/active-contention control does not deterministically schedule a competing begin during initialization or readout. The owner flag spanning both regions must be explicitly reviewed from source. Avoid claiming those two interleavings were executed; test hooks or a wider concurrency harness would be separate scope.",
    "allocator_noise": "rustc --test uses libtest, whose own threads/capture can allocate globally; --test-threads=1 and --nocapture reduce interference but do not prove absence of harness allocations. All setup/barriers/thread creation/panic-hook changes/output/assertion failures must be outside clean exact calibration windows. If exact calibration is contaminated, report it rather than retrying until green or subtracting guessed overhead. A tiny std-only harnessless executable including the exact module is the cleaner alternative for strict allocation counts, but should be separately scoped if needed.",
    "ci_wiring": "Detached bench/overlay-count has its own [workspace] and is not automatically exercised by root cargo test --workspace. No existing dedicated bench.yml command was established to run these new tests. Document the exact standalone command; do not claim full CI reaches it or silently expand CI."
  },
  "references": [
    {
      "url": "https://doc.rust-lang.org/nomicon/atomics.html",
      "provenance": "Root supplied current primary-source verification: atomic RMW is indivisible even atRelaxed; acquire/release is needed for ownership handoff ordering."
    }
  ],
  "source_inventory": {
    "bench/overlay-count/src/counting.rs": {
      "sha256": "25b3712e37c4142edac164b32ce47308123bac8201f8a3ff2e3078cd4b5416f2",
      "bytes": 4318
    },
    "bench/overlay-count/src/main.rs": {
      "sha256": "946188525809298d068450911e93b3d0c3bed2e9a7c6cbcd91b713a71e6c5e48",
      "bytes": 6085
    },
    "bench/overlay-count/src/lifecycle.rs": {
      "sha256": "d02c77e89674cf79f351f419540f1a69d192adae16bdbbc23a8fbce97c587e2b",
      "bytes": 13346
    },
    "bench/overlay-count/Cargo.toml": {
      "sha256": "f0cbcc2c7f6ea5e1e576254dc9b3904540833e03ac796505d31bf926f8b1c040",
      "bytes": 562
    },
    "bench/overlay-count/README.md": {
      "sha256": "73d93c0b757e2186a094768f82cdff9006209598e3442e070f831ebb83c90f4d",
      "bytes": 5014
    }
  },
  "status": "proposal_ready_for_root_review; source5757 remains clean; build hold preserved"
}
````

## Complete counting.rs

````rust
//! [GPT-6 Astra] Bench-only System wrapper, following bench/alloc-track.
//! Requested live bytes exclude allocator metadata and transient realloc internals.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};

struct Counting;
static ACTIVE: AtomicBool = AtomicBool::new(false);
static LIVE: AtomicU64 = AtomicU64::new(0);
static PEAK: AtomicU64 = AtomicU64::new(0);
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static REALLOCS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);

fn added(bytes: usize) {
    let live = LIVE.fetch_add(bytes as u64, Relaxed) + bytes as u64;
    if ACTIVE.load(Relaxed) {
        PEAK.fetch_max(live, Relaxed);
    }
}

// SAFETY: All pointer/layout operations are forwarded unchanged to System.
// Atomics allocate nothing, never dereference pointers and never unwind.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The GlobalAlloc caller supplies a valid layout.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            added(layout.size());
            if ACTIVE.load(Relaxed) {
                ALLOCS.fetch_add(1, Relaxed);
                BYTES.fetch_add(layout.size() as u64, Relaxed);
            }
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The GlobalAlloc caller supplies a valid layout.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            added(layout.size());
            if ACTIVE.load(Relaxed) {
                ALLOCS.fetch_add(1, Relaxed);
                BYTES.fetch_add(layout.size() as u64, Relaxed);
            }
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: The caller supplies this allocation's original pointer/layout.
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size() as u64, Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: The caller supplies a live allocation and valid nonzero new size.
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            if new_size >= layout.size() {
                added(new_size - layout.size());
            } else {
                LIVE.fetch_sub((layout.size() - new_size) as u64, Relaxed);
            }
            if ACTIVE.load(Relaxed) {
                REALLOCS.fetch_add(1, Relaxed);
                // Full new request size, not merely the growth in live bytes.
                BYTES.fetch_add(new_size as u64, Relaxed);
            }
        }
        new_ptr
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Begin a window while the benchmark and its initialized Rayon pool are idle.
pub fn begin() -> u64 {
    assert!(!ACTIVE.swap(false, Relaxed));
    let baseline = LIVE.load(Relaxed);
    PEAK.store(baseline, Relaxed);
    ALLOCS.store(0, Relaxed);
    REALLOCS.store(0, Relaxed);
    BYTES.store(0, Relaxed);
    ACTIVE.store(true, Relaxed);
    baseline
}

/// Stop the window before formatting output or checking returned query results.
pub fn end(baseline: u64) -> (u64, u64, u64, u64, u64) {
    ACTIVE.store(false, Relaxed);
    (
        ALLOCS.load(Relaxed),
        REALLOCS.load(Relaxed),
        BYTES.load(Relaxed),
        PEAK.load(Relaxed).saturating_sub(baseline),
        LIVE.load(Relaxed),
    )
}

pub fn calibrate() {
    let baseline = begin();
    // SAFETY: Each successful allocation is used only with its matching layout;
    // realloc transfers ownership on success. No allocated byte is dereferenced.
    unsafe {
        let old = Layout::from_size_align(128, 8).unwrap();
        let zero = Layout::from_size_align(64, 8).unwrap();
        let p = ALLOCATOR.alloc(old);
        let q = ALLOCATOR.alloc_zeroed(zero);
        assert!(!p.is_null() && !q.is_null());
        let p = ALLOCATOR.realloc(p, old, 256);
        assert!(!p.is_null());
        ALLOCATOR.dealloc(p, Layout::from_size_align(256, 8).unwrap());
        ALLOCATOR.dealloc(q, zero);
    }
    assert_eq!(end(baseline), (2, 1, 448, 320, baseline));
}
````

## Complete main caller

````rust
//! [GPT-6 Astra] Local overlay-count diagnostic; no canonical performance claim.

#[cfg(feature = "count-alloc")]
mod counting;
mod lifecycle;

use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use sparq_engine::query;
use std::{fmt::Write, hint::black_box, time::Instant};

const SUBJECTS: usize = 50_000;
const PREDICATES: usize = 4;
const DELETIONS: [usize; 5] = [0, 16, 1024, 8192, 32768];
const QUERY: &str = "SELECT ?o WHERE { <urn:s:49999> <urn:p:0> ?o }";

fn iri(value: &str) -> Term {
    NamedNode::new(value).unwrap().into()
}

fn base() -> Graph {
    let mut ttl = String::new();
    for s in 0..SUBJECTS {
        for p in 0..PREDICATES {
            writeln!(ttl, "<urn:s:{s}> <urn:p:{p}> <urn:o:{s}> .").unwrap();
        }
    }
    let graph = Graph::load_str(&ttl, "turtle").unwrap();
    assert_eq!(graph.store.len(), SUBJECTS * PREDICATES);
    // Freeze the dictionary outside every measured window, including no-delta cases.
    drop(graph.fork());
    graph
}

fn delta(n: usize, added: bool) -> Vec<[Term; 3]> {
    (0..n)
        .map(|i| {
            let s = i / PREDICATES + if added { SUBJECTS } else { 0 };
            [
                iri(&format!("urn:s:{s}")),
                iri(&format!("urn:p:{}", i % PREDICATES)),
                iri(&format!("urn:o:{s}")),
            ]
        })
        .collect()
}

fn fork(base: &Graph, delta: &[[Term; 3]], added: bool) -> Graph {
    let mut graph = base.fork();
    if added {
        graph.apply_delta(delta, &[]).unwrap();
    } else {
        graph.apply_delta(&[], delta).unwrap();
    }
    assert_eq!(graph.store.overlay_len(), delta.len());
    graph
}

fn check_query(graph: &Graph) {
    let b = query(graph, QUERY).unwrap();
    assert_eq!(b.rows.len(), 1);
    assert_eq!(b.rows[0][0], Some(iri("urn:o:49999")));
}

fn run(graph: &Graph, workload: &str, iterations: usize) -> usize {
    let pattern = [
        Some(graph.dict.lookup(&iri("urn:s:49999"))),
        Some(graph.dict.lookup(&iri("urn:p:0"))),
        None,
    ];
    let mut count = 0;
    for _ in 0..iterations {
        if workload == "scan" {
            count += black_box(graph.store.scan(black_box(&pattern)).rows.len());
        } else {
            let result = query(black_box(graph), black_box(QUERY)).unwrap();
            let b = black_box(result);
            count += b.rows.len();
        }
    }
    black_box(count)
}

fn rss() -> u64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // SAFETY: getrusage initializes the valid, writable output on success.
    assert_eq!(
        unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) },
        0
    );
    // SAFETY: the successful call above initialized the complete structure.
    let bytes = unsafe { usage.assume_init() }.ru_maxrss as u64;
    if cfg!(target_os = "macos") {
        bytes
    } else {
        bytes * 1024
    }
}

fn main() {
    rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build_global()
        .unwrap();
    #[cfg(feature = "count-alloc")]
    counting::calibrate();
    if std::env::args().nth(1).as_deref() == Some("reads-per-generation") {
        lifecycle::run_reads();
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("lifecycle") {
        lifecycle::run_all();
        return;
    }
    let graph = base();
    println!("{{\"kind\":\"fixture\",\"subjects\":{SUBJECTS},\"predicates\":{PREDICATES},\"base_triples\":{},\"setup_process_peak_rss_bytes\":{},\"rayon_threads\":1,\"counting\":{}}}", graph.store.len(), rss(), cfg!(feature = "count-alloc"));
    for (n, added) in DELETIONS
        .into_iter()
        .map(|n| (n, false))
        .chain([(8192, true)])
    {
        let changes = delta(n, added);
        // Independent oracle fork: verification cannot initialize a sample's caches.
        let oracle = fork(&graph, &changes, added);
        check_query(&oracle);
        assert_eq!(run(&oracle, "scan", 1), 1);
        drop(oracle);
        for workload in ["scan", "query"] {
            for phase in ["cold", "warm"] {
                let iterations = if phase == "cold" {
                    1
                } else if workload == "scan" {
                    10_000
                } else {
                    100
                };
                let reps = if cfg!(feature = "count-alloc") { 3 } else { 7 };
                // Two unrecorded samples warm code/pages, each on an independent fork.
                for rep in 0..reps + 2 {
                    let sample = fork(&graph, &changes, added);
                    let heap_cold = sample.store.heap_bytes();
                    if phase == "warm" {
                        assert_eq!(run(&sample, workload, 1), 1);
                    }
                    let heap_before = sample.store.heap_bytes();
                    let rss_before = rss();
                    #[cfg(feature = "count-alloc")]
                    let baseline = counting::begin();
                    let start = Instant::now();
                    let rows = run(&sample, workload, iterations);
                    let elapsed = start.elapsed().as_nanos();
                    #[cfg(feature = "count-alloc")]
                    let counts = counting::end(baseline);
                    #[cfg(not(feature = "count-alloc"))]
                    let counts = (0_u64, 0_u64, 0_u64, 0_u64, 0_u64);
                    assert_eq!(rows, iterations);
                    if rep >= 2 {
                        println!("{{\"kind\":\"sample\",\"delta\":{n},\"added\":{added},\"workload\":\"{workload}\",\"phase\":\"{phase}\",\"rep\":{},\"iterations\":{iterations},\"elapsed_ns\":{elapsed},\"allocs\":{},\"reallocs\":{},\"requested_bytes\":{},\"peak_live_delta_bytes\":{},\"store_heap_cold\":{heap_cold},\"store_heap_before\":{heap_before},\"store_heap_after\":{},\"process_peak_rss_before\":{rss_before},\"process_peak_rss_after\":{}}}", rep - 2, counts.0, counts.1, counts.2, counts.3, sample.store.heap_bytes(), rss());
                    }
                }
            }
        }
    }
}
````

## Complete lifecycle callers

````rust
//! [GPT-6 Astra] Fixed lifecycle/admission cases requested by the #4246 review.

use super::*;
use sparq_core::{store::Perm, GraphSnapshot};
use std::sync::Barrier;

const D: usize = 32_768;
const GENERATIONS: usize = 4;
const MULTI: &str = "SELECT ?o ?s ?p WHERE { <urn:s:49999> <urn:p:0> ?o . ?s <urn:p:1> <urn:o:49999> . ?s ?p <urn:o:49999> }";

#[derive(Clone, Copy, Default)]
struct Sample {
    elapsed: u128,
    phases: [u128; 3],
    counts: (u64, u64, u64, u64, u64),
    live_before: u64,
    rss_before: u64,
    rss_after: u64,
    retained_heap: usize,
    initial_heap: usize,
}

fn measure(f: impl FnOnce() -> [u128; 3]) -> Sample {
    let rss_before = rss();
    #[cfg(feature = "count-alloc")]
    let live_before = counting::begin();
    #[cfg(not(feature = "count-alloc"))]
    let live_before = 0;
    let start = Instant::now();
    let phases = f();
    let elapsed = start.elapsed().as_nanos();
    #[cfg(feature = "count-alloc")]
    let counts = counting::end(live_before);
    #[cfg(not(feature = "count-alloc"))]
    let counts = (0, 0, 0, 0, 0);
    Sample {
        elapsed,
        phases,
        counts,
        live_before,
        rss_before,
        rss_after: rss(),
        ..Sample::default()
    }
}

fn emit(case: &str, warm_perms: usize, rep: usize, generation: usize, row: Sample) {
    println!("{{\"kind\":\"lifecycle\",\"case\":\"{case}\",\"warm_perms\":{warm_perms},\"rep\":{rep},\"generation\":{generation},\"elapsed_ns\":{},\"phase_ns\":{:?},\"allocs\":{},\"reallocs\":{},\"requested_bytes\":{},\"peak_live_delta_bytes\":{},\"live_before\":{},\"live_after\":{},\"rss_before\":{},\"rss_after\":{},\"retained_overlay_reported_heap\":{},\"initial_overlay_reported_heap\":{}}}", row.elapsed,row.phases,row.counts.0,row.counts.1,row.counts.2,row.counts.3,row.live_before,row.counts.4,row.rss_before,row.rss_after,row.retained_heap,row.initial_heap);
}

fn prime(graph: &Graph, permutations: usize) {
    let pattern = [
        Some(graph.dict.lookup(&iri("urn:s:49999"))),
        Some(graph.dict.lookup(&iri("urn:p:0"))),
        Some(graph.dict.lookup(&iri("urn:o:49999"))),
    ];
    for &perm in &Perm::ALL[..permutations] {
        assert_eq!(graph.store.scan_perm(&pattern, perm).unwrap().rows.len(), 1);
    }
}

fn multi(graph: &Graph) -> usize {
    let result = query(black_box(graph), black_box(MULTI)).unwrap();
    black_box(result.rows.len())
}

fn check_multi(graph: &Graph) {
    let result = query(graph, MULTI).unwrap();
    assert_eq!(result.rows.len(), 4);
    let mut actual: Vec<_> = result
        .rows
        .iter()
        .map(|r| {
            assert_eq!(r[0], Some(iri("urn:o:49999")));
            assert_eq!(r[1], Some(iri("urn:s:49999")));
            r[2].as_ref().unwrap().to_string()
        })
        .collect();
    actual.sort_unstable();
    let expected: Vec<_> = (0..4)
        .map(|p| iri(&format!("urn:p:{p}")).to_string())
        .collect();
    assert_eq!(actual, expected);
}

pub(super) fn run_all() {
    let pristine = base();
    let base_heap = pristine.store.heap_bytes();
    let deletions = delta(D, false);
    let inserts: Vec<_> = (0..GENERATIONS)
        .map(|i| {
            [
                iri(&format!("urn:new:{i}")),
                iri("urn:p:0"),
                iri("urn:new-object"),
            ]
        })
        .collect();
    let tombstones: Vec<_> = (0..GENERATIONS)
        .map(|i| {
            [
                iri(&format!("urn:s:{}", 20_000 + i)),
                iri("urn:p:0"),
                iri(&format!("urn:o:{}", 20_000 + i)),
            ]
        })
        .collect();
    let oracle = fork(&pristine, &deletions, false);
    let before = oracle.store.heap_bytes();
    check_multi(&oracle);
    // This records actual multi-query projection engagement outside measured forks.
    println!("{{\"kind\":\"lifecycle_fixture\",\"subjects\":{SUBJECTS},\"predicates\":{PREDICATES},\"deletions\":{D},\"generations\":{GENERATIONS},\"multi_query_heap_delta\":{},\"counting\":{},\"setup_process_peak_rss_bytes\":{}}}", oracle.store.heap_bytes()-before,cfg!(feature="count-alloc"),rss());
    drop(oracle);
    let cases = [
        ("snapshot", 1),
        ("snapshot", 6),
        ("fork-insert", 1),
        ("fork-insert", 6),
        ("fork-tombstone", 1),
        ("fork-tombstone", 6),
        ("inplace-insert", 6),
        ("multi-cold", 0),
        ("multi-warm", 3),
        ("concurrent-cold", 0),
    ];
    for (case, warm_perms) in cases {
        let reps = if cfg!(feature = "count-alloc") { 3 } else { 7 };
        for rep in 0..reps + 2 {
            let mut initial = fork(&pristine, &deletions, false);
            if case == "multi-warm" {
                check_multi(&initial); // SPO, POS, OSP via three actual BGP estimates
            } else if warm_perms > 0 {
                prime(&initial, warm_perms);
            }
            let initial_heap = initial.store.heap_bytes() - base_heap;
            let mut records = [Sample::default(); GENERATIONS];
            let count = if matches!(case, "multi-cold" | "multi-warm" | "concurrent-cold") {
                1
            } else {
                GENERATIONS
            };
            let mut generations: Vec<Graph> = Vec::with_capacity(GENERATIONS);
            let mut snapshots: Vec<GraphSnapshot> = Vec::with_capacity(GENERATIONS);
            for generation in 0..count {
                let mut sample = measure(|| {
                    if case.starts_with("multi-") {
                        assert_eq!(multi(&initial), 4);
                        return [0; 3];
                    }
                    if case == "concurrent-cold" {
                        let barrier = Barrier::new(2);
                        // Includes thread launch/join overhead; reader durations are
                        // individual observations, not tail-percentile estimates.
                        return std::thread::scope(|scope| {
                            let read = || {
                                barrier.wait();
                                let start = Instant::now();
                                assert_eq!(run(&initial, "query", 1), 1);
                                start.elapsed().as_nanos()
                            };
                            let a = scope.spawn(read);
                            let b = scope.spawn(read);
                            [a.join().unwrap(), b.join().unwrap(), 0]
                        });
                    }
                    let start = Instant::now();
                    if case == "snapshot" {
                        let snapshot = initial.snapshot();
                        let cloned = start.elapsed().as_nanos();
                        let read = Instant::now();
                        assert_eq!(run(&snapshot, "query", 1), 1);
                        let read = read.elapsed().as_nanos();
                        snapshots.push(snapshot); // retain publication target
                        return [cloned, 0, read];
                    }
                    if case == "inplace-insert" {
                        initial
                            .apply_delta(std::slice::from_ref(&inserts[generation]), &[])
                            .unwrap();
                        let delta = start.elapsed().as_nanos();
                        let read = Instant::now();
                        assert_eq!(run(&initial, "query", 1), 1);
                        return [0, delta, read.elapsed().as_nanos()];
                    }
                    let parent = generations.last().unwrap_or(&initial);
                    let mut next = parent.fork();
                    let cloned = start.elapsed().as_nanos();
                    let delta = Instant::now();
                    if case == "fork-insert" {
                        next.apply_delta(std::slice::from_ref(&inserts[generation]), &[])
                            .unwrap();
                    } else {
                        next.apply_delta(&[], std::slice::from_ref(&tombstones[generation]))
                            .unwrap();
                    }
                    let delta = delta.elapsed().as_nanos();
                    let read = Instant::now();
                    assert_eq!(run(&next, "query", 1), 1);
                    let read = read.elapsed().as_nanos();
                    generations.push(next); // local ownership publication, no service I/O
                    [cloned, delta, read]
                });
                sample.initial_heap = initial_heap;
                sample.retained_heap = initial.store.heap_bytes() - base_heap
                    + generations
                        .iter()
                        .map(|g| g.store.heap_bytes() - base_heap)
                        .sum::<usize>()
                    + snapshots
                        .iter()
                        .map(|g| g.store.heap_bytes() - base_heap)
                        .sum::<usize>();
                records[generation] = sample;
            }
            // Verify the retained generation contents outside every measured window.
            for (i, graph) in generations.iter().enumerate() {
                check_query(graph);
                assert_eq!(
                    graph.store.len(),
                    SUBJECTS * PREDICATES - D + if case == "fork-insert" { i + 1 } else { 0 }
                        - if case == "fork-tombstone" { i + 1 } else { 0 }
                );
            }
            for graph in &snapshots {
                check_query(graph);
                assert_eq!(graph.store.len(), SUBJECTS * PREDICATES - D);
            }
            if rep >= 2 {
                for (generation, &record) in records[..count].iter().enumerate() {
                    emit(case, warm_perms, rep - 2, generation + 1, record);
                }
            }
        }
    }
}

/// [GPT-6 Astra] The sole additional measurement authorized after the opt-in
/// decision: fixed deletion lineage, varying only reads per retained generation.
pub(super) fn run_reads() {
    let pristine = base();
    let base_heap = pristine.store.heap_bytes();
    let deletions = delta(D, false);
    let tombstones: Vec<_> = (0..GENERATIONS)
        .map(|i| {
            [
                iri(&format!("urn:s:{}", 20_000 + i)),
                iri("urn:p:0"),
                iri(&format!("urn:o:{}", 20_000 + i)),
            ]
        })
        .collect();
    check_query(&pristine);
    println!("{{\"kind\":\"reads_fixture\",\"subjects\":{SUBJECTS},\"predicates\":{PREDICATES},\"deletions\":{D},\"generations\":{GENERATIONS},\"initial_warm_perms\":6,\"rayon_threads\":1,\"counting\":{},\"setup_process_peak_rss_bytes\":{}}}",cfg!(feature="count-alloc"),rss());
    for (reads, name) in [
        (1, "fork-tombstone-R1"),
        (2, "fork-tombstone-R2"),
        (4, "fork-tombstone-R4"),
        (8, "fork-tombstone-R8"),
        (16, "fork-tombstone-R16"),
    ] {
        let reps = if cfg!(feature = "count-alloc") { 3 } else { 7 };
        for rep in 0..reps + 2 {
            let initial = fork(&pristine, &deletions, false);
            prime(&initial, 6);
            let initial_heap = initial.store.heap_bytes() - base_heap;
            let mut generations: Vec<Graph> = Vec::with_capacity(GENERATIONS);
            let mut records = [Sample::default(); GENERATIONS];
            for generation in 0..GENERATIONS {
                let mut sample = measure(|| {
                    let start = Instant::now();
                    let mut next = generations.last().unwrap_or(&initial).fork();
                    let cloned = start.elapsed().as_nanos();
                    let delta = Instant::now();
                    next.apply_delta(&[], std::slice::from_ref(&tombstones[generation]))
                        .unwrap();
                    let delta = delta.elapsed().as_nanos();
                    let read = Instant::now();
                    assert_eq!(run(&next, "query", reads), reads);
                    let read = read.elapsed().as_nanos();
                    generations.push(next);
                    [cloned, delta, read]
                });
                sample.initial_heap = initial_heap;
                sample.retained_heap = initial_heap
                    + generations
                        .iter()
                        .map(|g| g.store.heap_bytes() - base_heap)
                        .sum::<usize>();
                records[generation] = sample;
            }
            for (generation, graph) in generations.iter().enumerate() {
                check_query(graph);
                assert_eq!(
                    graph.store.len(),
                    SUBJECTS * PREDICATES - D - generation - 1
                );
                for (i, tombstone) in tombstones.iter().enumerate() {
                    let pattern = tombstone.clone().map(|t| Some(graph.dict.lookup(&t)));
                    assert_eq!(
                        graph.store.scan(&pattern).rows.len(),
                        usize::from(i > generation)
                    );
                }
            }
            if rep >= 2 {
                for (generation, &sample) in records.iter().enumerate() {
                    emit(name, 6, rep - 2, generation + 1, sample);
                }
            }
        }
    }
}
````

## Actual source search

````text
bench/overlay-count/src/counting.rs:80:pub fn begin() -> u64 {
bench/overlay-count/src/counting.rs:92:pub fn end(baseline: u64) -> (u64, u64, u64, u64, u64) {
bench/overlay-count/src/counting.rs:103:pub fn calibrate() {
bench/overlay-count/src/counting.rs:104:    let baseline = begin();
bench/overlay-count/src/counting.rs:118:    assert_eq!(end(baseline), (2, 1, 448, 320, baseline));
bench/overlay-count/src/lifecycle.rs:23:fn measure(f: impl FnOnce() -> [u128; 3]) -> Sample {
bench/overlay-count/src/lifecycle.rs:26:    let live_before = counting::begin();
bench/overlay-count/src/lifecycle.rs:33:    let counts = counting::end(live_before);
bench/overlay-count/src/lifecycle.rs:145:                let mut sample = measure(|| {
bench/overlay-count/src/lifecycle.rs:161:                            let a = scope.spawn(read);
bench/overlay-count/src/lifecycle.rs:162:                            let b = scope.spawn(read);
bench/overlay-count/src/lifecycle.rs:163:                            [a.join().unwrap(), b.join().unwrap(), 0]
bench/overlay-count/src/lifecycle.rs:269:                let mut sample = measure(|| {
bench/overlay-count/src/main.rs:102:        .num_threads(1)
bench/overlay-count/src/main.rs:103:        .build_global()
bench/overlay-count/src/main.rs:106:    counting::calibrate();
bench/overlay-count/src/main.rs:148:                    let baseline = counting::begin();
bench/overlay-count/src/main.rs:153:                    let counts = counting::end(baseline);
````

## Detached benchmark manifest

````toml
# [GPT-6 Astra] Local diagnostic only; no production feature or dependency changes.
[package]
name = "overlay-count-diagnostic"
version = "0.0.0"
edition = "2021"
publish = false

[workspace]

[patch.crates-io]
spargebra = { path = "../../vendor/spargebra" }

[features]
count-alloc = []

[dependencies]
sparq-core = { path = "../../crates/sparq-core" }
sparq-engine = { path = "../../crates/sparq-engine" }
rayon = "1"
oxrdf = { version = "0.3", features = ["rdf-12"] }
libc = "0.2"

[profile.release]
opt-level = 3
debug = false
lto = false
codegen-units = 16
````
