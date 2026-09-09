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
