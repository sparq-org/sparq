//! [GPT-6 Astra] Local whole-query top-k diagnostic; never a canonical score.
//! Separate timing and counting builds avoid timing the instrumented allocator.

#[cfg(feature = "count-alloc")]
mod counting;

use oxrdf::{Literal, NamedNode, Term};
use sparq_core::Graph;
use sparq_engine::{query, QueryResult};
use std::fmt::Write;
use std::hint::black_box;
use std::time::Instant;

fn iri(value: &str) -> Term {
    NamedNode::new(value).unwrap().into()
}

fn priority(i: usize, n: usize, ties: bool) -> usize {
    if ties && i >= n - (n / 2 + 1) {
        n + 1
    } else {
        i
    }
}

fn pending(i: usize, n: usize, drained: bool) -> bool {
    !drained || i < n - (n / 2 - 1)
}

struct Fixture {
    graph: Graph,
    // Keep the original snapshot alive in the overlay case, as a live reader would.
    _snapshot: Option<Graph>,
    expected: Vec<usize>,
    seed_rows: usize,
    pending_rows: usize,
    tied_rows: usize,
    tombstones: usize,
}

fn fixture(case: &str, n: usize, k: usize) -> Fixture {
    assert!(matches!(
        case,
        "base" | "overlay" | "drained" | "drained-overlay" | "ties" | "ties-overlay"
            | "prefix128" | "prefix128-overlay" | "prefix800" | "prefix800-overlay"
            | "intermittent" | "intermittent-overlay"
    ));
    assert!(n >= 1024 && n <= 100_000 && k > 0 && k <= 1024);
    let overlay = case.contains("overlay");
    let drained = case.starts_with("drained");
    let ties = case.starts_with("ties");
    // [GPT-6 Astra] Predeclared miss fixtures use LIVE rank so the overlay
    // has exactly the same leading miss count after its every-fifth deletions.
    let prefix = if case.starts_with("prefix128") { 128 } else if case.starts_with("prefix800") { 800 } else { 0 };
    let intermittent = case.starts_with("intermittent");
    let mut ttl = String::new();
    let mut deletes = Vec::new();
    let mut expected = Vec::new();
    let (mut seed_rows, mut pending_rows, mut tied_rows) = (0, 0, 0);
    for i in 0..n {
        let p = priority(i, n, ties);
        let live_rank = n - 1 - i - if overlay { (n - 1) / 5 - i / 5 } else { 0 };
        let is_pending = if prefix > 0 { live_rank >= prefix } else if intermittent { live_rank % 8 != 0 } else { pending(i, n, drained) };
        let status = if is_pending {
            "pending"
        } else {
            "done"
        };
        writeln!(ttl, "<urn:s:{i}> <urn:peer> <urn:X> ; <urn:status> \"{status}\" ; <urn:priority> {p} ; <urn:seq> {i} ; <urn:a> {i} ; <urn:b> {i} ; <urn:c> {i} .").unwrap();
        if overlay && i % 5 == 0 {
            let s = iri(&format!("urn:s:{i}"));
            for (pred, object) in [
                ("peer", iri("urn:X")),
                ("status", Literal::new_simple_literal(status).into()),
                ("priority", Literal::from(p as i64).into()),
                ("seq", Literal::from(i as i64).into()),
                ("a", Literal::from(i as i64).into()),
                ("b", Literal::from(i as i64).into()),
                ("c", Literal::from(i as i64).into()),
            ] {
                deletes.push([s.clone(), iri(&format!("urn:{pred}")), object]);
            }
        } else {
            seed_rows += 1;
            tied_rows += usize::from(ties && p == n + 1);
            if is_pending {
                pending_rows += 1;
                expected.push((p, i));
            }
        }
    }
    expected.sort_unstable_by(|a, b| b.cmp(a));
    let expected = expected.into_iter().take(k).map(|(_, i)| i).collect();
    let base = Graph::load_str(&ttl, "turtle").unwrap();
    let tombstones = deletes.len();
    let (graph, snapshot) = if overlay {
        let mut fork = base.fork();
        fork.apply_delta(&[], &deletes).unwrap();
        assert_eq!(fork.pending_delta_len(), tombstones);
        (fork, Some(base))
    } else {
        (base, None)
    };
    if drained {
        assert!(pending_rows * 2 >= seed_rows);
        assert!(pending_rows * 2 - seed_rows <= 4);
    }
    if prefix > 0 {
        assert_eq!(seed_rows - pending_rows, prefix);
    }
    if intermittent {
        assert_eq!(seed_rows - pending_rows, seed_rows.div_ceil(8));
    }
    if ties {
        assert!(tied_rows > (seed_rows / 2).max(256).max(2 * k));
    }
    Fixture {
        graph,
        _snapshot: snapshot,
        expected,
        seed_rows,
        pending_rows,
        tied_rows,
        tombstones,
    }
}

fn check(result: &QueryResult, expected: &[usize]) {
    let ids: Vec<_> = result
        .rows
        .iter()
        .map(|row| match &row[0] {
            Some(Term::NamedNode(n)) => n
                .as_str()
                .strip_prefix("urn:s:")
                .unwrap()
                .parse::<usize>()
                .unwrap(),
            other => panic!("unexpected binding: {other:?}"),
        })
        .collect();
    assert_eq!(ids, expected, "independent total-order oracle");
}

/// Cumulative process RSS high-water mark; it cannot be reset at a query boundary.
fn max_rss() -> u64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // SAFETY: getrusage initializes the pointed-to rusage on success; RUSAGE_SELF
    // requests this process only. Assume initialization only after return code zero.
    assert_eq!(
        unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) },
        0
    );
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
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).is_some_and(|x| x == "selftest") {
        #[cfg(feature = "count-alloc")]
        counting::calibrate();
        println!(
            "kind=selftest\tcounting={}\tstatus=pass",
            cfg!(feature = "count-alloc")
        );
        return;
    }
    assert_eq!(args.len(), 8, "MODE CASE N K WARMUP REPS EXPECTED_PATH");
    let (mode, case) = (args[1].as_str(), args[2].as_str());
    let n = args[3].parse().unwrap();
    let k = args[4].parse().unwrap();
    let warmup: usize = args[5].parse().unwrap();
    let reps: usize = args[6].parse().unwrap();
    assert!(matches!(mode, "verify" | "measure") && reps > 0);
    #[cfg(feature = "count-alloc")]
    let setup_baseline = counting::begin();
    let f = fixture(case, n, k);
    let text = format!("SELECT ?s WHERE {{ ?s <urn:peer> <urn:X> ; <urn:status> \"pending\" ; <urn:priority> ?p ; <urn:seq> ?seq ; <urn:a> ?a ; <urn:b> ?b ; <urn:c> ?c }} ORDER BY DESC(?p) DESC(?seq) LIMIT {k}");
    #[cfg(feature = "count-alloc")]
    let setup_counts = counting::end(setup_baseline);
    let setup_rss = max_rss();
    println!("kind=fixture\tcase={case}\tn={n}\tk={k}\tseed_rows={}\tpending_rows={}\ttied_rows={}\tprobe_predicates=6\ttombstones={}\tsetup_max_rss={setup_rss}\tcounting={}", f.seed_rows, f.pending_rows, f.tied_rows, f.tombstones, cfg!(feature="count-alloc"));
    #[cfg(feature = "count-alloc")]
    println!("kind=setup_alloc\tallocs={}\treallocs={}\trequested_bytes={}\tpeak_live_delta={}\tending_live={}", setup_counts.0, setup_counts.1, setup_counts.2, setup_counts.3, setup_counts.4);
    if mode == "verify" {
        let result = query(&f.graph, &text).unwrap();
        check(&result, &f.expected);
        let trace = sparq_engine::explain_analyze(&f.graph, &text).unwrap();
        let fallback = trace.contains("BGP [binary GOO]");
        assert_eq!(fallback, args[7] == "fallback", "{trace}");
        println!(
            "kind=verify\tpath={}\trows={}\toracle=pass",
            if fallback {
                "fallback"
            } else {
                "indexed-inferred"
            },
            result.rows.len()
        );
        println!("{trace}");
        return;
    }
    for _ in 0..warmup {
        check(&query(&f.graph, &text).unwrap(), &f.expected);
    }
    let pre_query_rss = max_rss();
    for rep in 0..reps {
        #[cfg(feature = "count-alloc")]
        let baseline = counting::begin();
        let start = Instant::now();
        let result = black_box(query(black_box(&f.graph), black_box(&text)).unwrap());
        let nanos = start.elapsed().as_nanos();
        #[cfg(feature = "count-alloc")]
        let counts = counting::end(baseline);
        let rss = max_rss();
        // Validation and output formatting are outside the measured call/window.
        check(&result, &f.expected);
        #[cfg(feature = "count-alloc")]
        println!("kind=sample\trep={rep}\tallocs={}\treallocs={}\trequested_bytes={}\tpeak_live_delta={}\tending_live={}\tpre_query_max_rss={pre_query_rss}\tpost_query_max_rss={rss}\trows={}\toracle=pass", counts.0, counts.1, counts.2, counts.3, counts.4, result.rows.len());
        #[cfg(not(feature = "count-alloc"))]
        println!("kind=sample\trep={rep}\twhole_query_ns={nanos}\tpre_query_max_rss={pre_query_rss}\tpost_query_max_rss={rss}\trows={}\toracle=pass", result.rows.len());
        #[cfg(feature = "count-alloc")]
        let _ = nanos; // Instrumented timing is deliberately not reported.
    }
}
