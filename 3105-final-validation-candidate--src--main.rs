//! [GPT-6 Astra] Fixed local whole-query diagnostic for issue3105; not canonical.
#[cfg(feature = "count-alloc")]
mod counting;
use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use sparq_engine::{ask, query};
use std::{fmt::Write, hint::black_box, time::Instant};

fn iri(s: &str) -> Term {
    NamedNode::new(s).unwrap().into()
}
fn fixture(overlay: bool, n: usize) -> Graph {
    let mut ttl = String::new();
    for i in 0..70_000 {
        for j in 0..n { writeln!(ttl, "<urn:s:{i}> <urn:p{j}> {i} .").unwrap(); }
    }
    for i in 0..8192 {
        writeln!(ttl, "<urn:d:{i}> <urn:dead> <urn:o> .").unwrap();
    }
    let base = Graph::load_str(&ttl, "turtle").unwrap();
    if !overlay {
        return base;
    }
    let removed: Vec<[Term; 3]> = (0..4096)
        .map(|i| [iri(&format!("urn:d:{i}")), iri("urn:dead"), iri("urn:o")])
        .collect();
    let mut graph = base.fork();
    graph.apply_delta(&[], &removed).unwrap();
    assert_eq!(graph.store.overlay_len(), 4096);
    graph
}
fn run(graph: &Graph, q: &str, select: bool) -> usize {
    if select {
        black_box(query(black_box(graph), black_box(q)).unwrap())
            .rows
            .len()
    } else {
        usize::from(black_box(ask(black_box(graph), black_box(q)).unwrap()))
    }
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
    let args: Vec<String> = std::env::args().collect();
    let case = &args[1];
    let overlay = args[2] == "overlay";
    let n: usize = case.parse().unwrap();
    assert!(n == 5 || n == 8);
    let graph = fixture(overlay, n);
    let pid = graph.dict.lookup(&iri("urn:p0"));
    let scan = graph.store.scan_sorted(&[None, Some(pid), None], 0);
    assert_eq!(scan.rows.len(), 70_000);
    assert_eq!(scan.perm.order().into_iter().find(|&c| c != 1), Some(0));
    let position = if case == "second" { 2048 } else { 0 };
    let value = graph
        .dict
        .term(scan.to_spo(&scan.rows[position])[2])
        .to_string();
    let threshold = value.split('"').nth(1).unwrap().parse::<usize>().unwrap();
    drop(scan);
    let patterns: String = (0..n).map(|j| format!("?s <urn:p{j}> ?o . ")).collect();
    let filter = if case == "first" || case == "second" {
        format!("?o + 0 = {threshold}")
    } else if case == "late" {
        "?o + 0 >= 0".to_string()
    } else {
        "?o + 0 < 0".to_string()
    };
    let body = format!("{patterns} FILTER({filter})");
    let select = case == "late";
    let q = if select {
        format!("SELECT ?s ?o WHERE {{ {body} }} LIMIT 65537")
    } else {
        format!("ASK {{ {body} }}")
    };
    let expected = if select {
        65537
    } else {
        usize::from(case == "first" || case == "second")
    };
    // Full, uncapped oracle is outside all measurement windows.
    let oracle = query(&graph, &format!("SELECT ?s ?o WHERE {{ {body} }}")).unwrap();
    assert_eq!(oracle.rows.len(), if select { 70_000 } else { expected });
    for row in &oracle.rows {
        let s = row[0].as_ref().unwrap().to_string();
        let o = row[1].as_ref().unwrap().to_string();
        let i = o.split('"').nth(1).unwrap().parse::<usize>().unwrap();
        assert_eq!(s, format!("<urn:s:{i}>"));
    }
    if select {
        let checked = query(&graph, &q).unwrap();
        assert_eq!(checked.rows.len(), expected);
        for row in &checked.rows {
            let i = row[1]
                .as_ref()
                .unwrap()
                .to_string()
                .split('"')
                .nth(1)
                .unwrap()
                .parse::<usize>()
                .unwrap();
            assert!(i < 70_000);
            assert_eq!(row[0].as_ref().unwrap().to_string(), format!("<urn:s:{i}>"));
        }
    }
    drop(oracle);
    println!("{{\"kind\":\"fixture\",\"query\":\"{q}\",\"seed_position\":{position},\"threshold\":{threshold},\"triples\":{}}}",graph.store.len());
    let setup_rss = rss();
    let sample: usize = args[3].parse().unwrap();
    let reps = 1;
    let iterations = if cfg!(feature = "count-alloc") { 1 } else { 3 };
    for _ in 0..2 {
        assert_eq!(run(&graph, &q, select), expected);
    }
    #[cfg(feature = "count-alloc")]
    {
        // Complete worker startup after fixture/oracle and both query warmups.
        rayon::broadcast(|_| ());
        counting::calibrate();
    }
    for _ in 0..reps {
        let rep = sample;
        let before_rss = rss();
        #[cfg(feature = "count-alloc")]
        let baseline = counting::begin();
        let start = Instant::now();
        let mut rows = 0;
        for _ in 0..iterations {
            rows += run(&graph, &q, select);
        }
        let nanos = start.elapsed().as_nanos();
        #[cfg(feature = "count-alloc")]
        let (allocs, reallocs, bytes, peak, live) = counting::end(baseline);
        #[cfg(not(feature = "count-alloc"))]
        let (allocs, reallocs, bytes, peak, live) = (0u64, 0u64, 0u64, 0u64, 0u64);
        assert_eq!(rows, iterations * expected);
        println!("{{\"case\":\"{case}\",\"view\":\"{}\",\"counting\":{},\"rep\":{rep},\"iterations\":{iterations},\"nanos\":{nanos},\"rows\":{rows},\"allocs\":{allocs},\"reallocs\":{reallocs},\"requested_bytes\":{bytes},\"peak_live_growth\":{peak},\"live_after\":{live},\"setup_peak_rss\":{setup_rss},\"before_peak_rss\":{before_rss},\"after_peak_rss\":{}}}",if overlay {"overlay"} else {"base"},cfg!(feature="count-alloc"),rss());
    }
}
