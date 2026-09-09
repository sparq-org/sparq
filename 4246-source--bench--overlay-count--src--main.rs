//! [GPT-6 Astra] Local overlay-count diagnostic; no canonical performance claim.

#[cfg(feature = "count-alloc")]
mod counting;

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
