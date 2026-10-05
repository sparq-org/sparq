//! Differential tests for the bound-endpoint pushdown: for every operator ×
//! every binding shape (subject bound / object bound / both / neither /
//! same-variable), the pushed-down evaluation must agree with the
//! full-relation evaluation filtered after the fact — over random graphs
//! containing cycles, diamonds, self-loops and disconnected components.
use super::*;

/// Random multigraph over predicates `:e` / `:f`, plus deterministic
/// adversarial shapes: a diamond (d0→{d1,d2}→d3), a directed 3-cycle
/// (c0→c1→c2→c0), a self-loop (s0→s0) and a disconnected island (i0→i1).
fn random_graph(seed0: u64, n_nodes: u32, n_edges: usize) -> Graph {
    let mut seed = seed0;
    let mut next = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) as u32
    };
    let mut ttl = String::from("@prefix : <http://ex/> .\n");
    for _ in 0..n_edges {
        let (a, b) = (next() % n_nodes, next() % n_nodes);
        let p = if next() % 3 == 0 { "f" } else { "e" };
        ttl.push_str(&format!(":n{a} :{p} :n{b} .\n"));
    }
    ttl.push_str(":d0 :e :d1 . :d0 :e :d2 . :d1 :e :d3 . :d2 :e :d3 .\n");
    ttl.push_str(":c0 :e :c1 . :c1 :e :c2 . :c2 :e :c0 . :s0 :e :s0 .\n");
    ttl.push_str(":i0 :f :i1 .\n");
    Graph::load_str(&ttl, "turtle").unwrap()
}

/// Sorted row-strings of a result (paths have set semantics, order-free).
fn rowset(r: &crate::QueryResult) -> Vec<String> {
    let mut v: Vec<String> = r
        .rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|t| t.as_ref().map(|t| t.to_string()).unwrap_or_default())
                .collect::<Vec<_>>()
                .join("\t")
        })
        .collect();
    v.sort();
    v.dedup();
    v
}

const PFX: &str = "PREFIX : <http://ex/> ";

/// Every path operator and the compositions whose pushdown rules differ
/// (inverse-of-recursive, recursive inside sequence on either side,
/// alternative-of-recursive, negated sets, zero-length operators).
const PATHS: &[&str] = &[
    ":e",
    "^:e",
    ":e+",
    ":e*",
    ":e?",
    "^:e+",
    "(^:e)+",
    ":e/:f",
    ":e/:f+",
    ":e+/:f",
    ":e+/:f+",
    ":e|:f",
    "(:e|:f)+",
    "(:e|^:f)*",
    "!:e",
    "!(:e|:f)",
    "^(:e/:f)",
    ":e*/:f?",
];

/// Endpoints sampled from every adversarial shape (all exist in each graph).
const NODES: &[&str] = &["n0", "n1", "n2", "d0", "d3", "c0", "c1", "s0", "i0", "i1"];

#[test]
fn pushdown_matches_full_closure_for_all_operators_and_binding_shapes() {
    for seed in [0x1111u64, 0xACE1, 0xDEADBEEF] {
        let g = random_graph(seed, 12, 40);
        for path in PATHS {
            // Reference: the full (both-unbound) relation, post-filtered here.
            let full = rowset(&crate::query(&g, &format!("{PFX}SELECT ?x ?y WHERE {{ ?x {path} ?y }}")).unwrap());
            let pairs: Vec<(String, String)> = full
                .iter()
                .map(|r| {
                    let (x, y) = r.split_once('\t').unwrap();
                    (x.to_string(), y.to_string())
                })
                .collect();
            // Same variable at both ends: the diagonal.
            let diag = rowset(&crate::query(&g, &format!("{PFX}SELECT ?x WHERE {{ ?x {path} ?x }}")).unwrap());
            let mut want_diag: Vec<String> =
                pairs.iter().filter(|(x, y)| x == y).map(|(x, _)| x.clone()).collect();
            want_diag.sort();
            want_diag.dedup();
            assert_eq!(diag, want_diag, "same-var disagrees for `{path}` (seed {seed:#x})");
            for node in NODES {
                let iri = format!("<http://ex/{node}>");
                // Subject bound.
                let got =
                    rowset(&crate::query(&g, &format!("{PFX}SELECT ?y WHERE {{ :{node} {path} ?y }}")).unwrap());
                let mut want: Vec<String> =
                    pairs.iter().filter(|(x, _)| *x == iri).map(|(_, y)| y.clone()).collect();
                want.sort();
                want.dedup();
                assert_eq!(got, want, "bound-subject :{node} disagrees for `{path}` (seed {seed:#x})");
                // Object bound.
                let got =
                    rowset(&crate::query(&g, &format!("{PFX}SELECT ?x WHERE {{ ?x {path} :{node} }}")).unwrap());
                let mut want: Vec<String> =
                    pairs.iter().filter(|(_, y)| *y == iri).map(|(x, _)| x.clone()).collect();
                want.sort();
                want.dedup();
                assert_eq!(got, want, "bound-object :{node} disagrees for `{path}` (seed {seed:#x})");
            }
            // Both bound: membership must match the full relation exactly.
            for (a, b) in
                [("n0", "n1"), ("d0", "d3"), ("c0", "c0"), ("c0", "c2"), ("s0", "s0"), ("i0", "i1"), ("n0", "i0")]
            {
                let row = format!("<http://ex/{a}>\t<http://ex/{b}>");
                let got = crate::query(&g, &format!("{PFX}SELECT * WHERE {{ :{a} {path} :{b} }}")).unwrap();
                assert_eq!(
                    !got.rows.is_empty(),
                    full.contains(&row),
                    "both-bound :{a}/:{b} disagrees for `{path}` (seed {seed:#x})"
                );
            }
        }
    }
}

/// The negated-property-set block-skip scan must agree with an INDEPENDENT
/// evaluation path: a plain scan + FILTER on the predicate.
#[test]
fn negated_property_set_matches_filter_oracle() {
    for seed in [0x5EEDu64, 0xBEEF] {
        let g = random_graph(seed, 10, 60);
        for (path, cond) in [("!:e", "?p != :e"), ("!(:e|:f)", "?p != :e && ?p != :f")] {
            let got = rowset(&crate::query(&g, &format!("{PFX}SELECT ?x ?y WHERE {{ ?x {path} ?y }}")).unwrap());
            let oracle = rowset(
                &crate::query(
                    &g,
                    &format!("{PFX}SELECT DISTINCT ?x ?y WHERE {{ ?x ?p ?y FILTER({cond}) }}"),
                )
                .unwrap(),
            );
            assert_eq!(got, oracle, "`{path}` disagrees with filter oracle (seed {seed:#x})");
        }
    }
}

/// (sq-5kr) Pin the bound-endpoint shortcuts that AVOID the
/// full-store scans: `p*`/`p?` over a bound endpoint must emit ITS reflexive
/// pair (not the whole `graph_nodes()` domain), and a bound-endpoint `!(...)`
/// must return only that node's non-excluded edges from the narrowed scan.
/// The load-bearing case is a CONSTANT endpoint absent from the data
/// (`:z`, which occurs in no triple): the zero-length rules still bind
/// `<z> p* <z>` / `<z> p? <z>`, whereas a reflexive pass over `graph_nodes()`
/// would never see `:z` and would WRONGLY drop the solution — so this guards
/// against a regression to the old domain-scan reflexive logic.
#[test]
fn bound_endpoint_zero_length_and_negated_avoid_full_store_scan() {
    // :a -e-> :b ; :a -f-> :c .  :z occurs in NO triple.
    let g = Graph::load_str("@prefix : <http://ex/> .\n:a :e :b . :a :f :c .\n", "turtle").unwrap();
    let rows = |sparql: &str| rowset(&crate::query(&g, &format!("{PFX}{sparql}")).unwrap());
    let ask = |sparql: &str| !crate::query(&g, &format!("{PFX}{sparql}")).unwrap().rows.is_empty();
    let want = |items: &[&str]| {
        let mut v: Vec<String> = items.iter().map(|s| s.to_string()).collect();
        v.sort();
        v.dedup();
        v
    };

    // Bound-subject `p*`: reflexive self + the one `:e` hop (NOT the node domain).
    assert_eq!(rows("SELECT ?y WHERE { :a :e* ?y }"), want(&["<http://ex/a>", "<http://ex/b>"]));
    // Bound-subject `p?`: reflexive self + the one `:e` hop.
    assert_eq!(rows("SELECT ?y WHERE { :a :e? ?y }"), want(&["<http://ex/a>", "<http://ex/b>"]));
    // Constant endpoint ABSENT from the data still has its zero-length self-pair.
    assert_eq!(rows("SELECT ?y WHERE { :z :e* ?y }"), want(&["<http://ex/z>"]));
    assert!(ask("ASK WHERE { :z :e? :z }"), "absent-term :z must satisfy its own :e? self-pair");
    // Bound-subject negated set: only :a's non-:e edges (the :f hop), narrowed scan.
    assert_eq!(rows("SELECT ?y WHERE { :a !:e ?y }"), want(&["<http://ex/c>"]));
    // Bound-OBJECT negated set: subjects reaching :c via a non-:e predicate (:a -f-> :c).
    assert_eq!(rows("SELECT ?x WHERE { ?x !:e :c }"), want(&["<http://ex/a>"]));
}

/// The row budget must fire INSIDE a single-source traversal (not only
/// between traversal roots): one long chain, one start node, tiny budget.
#[test]
fn budget_fires_inside_directed_traversal() {
    let mut ttl = String::from("@prefix : <http://ex/> .\n");
    for k in 0..20_000u32 {
        ttl.push_str(&format!(":m{k} :e :m{} .\n", k + 1));
    }
    let g = Graph::load_str(&ttl, "turtle").unwrap();
    let budget = crate::QueryBudget { max_rows: Some(64), ..crate::QueryBudget::unlimited() };
    let err = crate::query_with_budget(&g, "PREFIX : <http://ex/> SELECT ?y WHERE { :m0 :e+ ?y }", &budget)
        .unwrap_err();
    assert!(err.contains("budget"), "expected a budget error, got: {err}");
}

/// PAIRED micro-benchmark (same process, same graph) for bound-subject
/// `knows+` at ~1M edges: the single-source pushdown vs the previous
/// full-closure-then-filter algorithm. Clustered social graph — 100k
/// people in 1000 clusters of 100, ten random in-cluster `knows` edges
/// each — so the full closure is feasible to measure at all (~10M pairs;
/// an unclustered 1M-edge graph would make "before" run for hours, which
/// is the pathology being fixed). Run manually:
///   cargo test -p sparq-engine --release bench_bound_endpoint -- --ignored --nocapture
/// Absolute times on a shared machine are contended; the RATIO is the result.
#[test]
#[ignore]
fn bench_bound_endpoint_pushdown_1m_edges() {
    use std::time::Instant;
    const CLUSTERS: u32 = 1_000;
    const SIZE: u32 = 100;
    const OUT_DEG: u32 = 10;
    let mut seed = 0x5EEDu64;
    let mut next = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) as u32
    };
    let mut ttl = String::with_capacity(48 << 20);
    ttl.push_str("@prefix : <http://ex/> .\n");
    for c in 0..CLUSTERS {
        for m in 0..SIZE {
            let a = c * SIZE + m;
            for _ in 0..OUT_DEG {
                let b = c * SIZE + next() % SIZE;
                ttl.push_str(&format!(":p{a} :knows :p{b} .\n"));
            }
        }
    }
    let t = Instant::now();
    let g = Graph::load_str(&ttl, "turtle").unwrap();
    eprintln!("loaded {} triples in {:.2?}", g.store.len(), t.elapsed());

    let knows = PropertyPathExpression::NamedNode(oxrdf::NamedNode::new("http://ex/knows").unwrap());
    let plus = PropertyPathExpression::OneOrMore(Box::new(knows.clone()));
    let id = |k: u32| g.id_of(&Term::NamedNode(oxrdf::NamedNode::new(format!("http://ex/p{k}")).unwrap())).unwrap();

    // AFTER: bound-subject pushdown, median over many random starts.
    let starts: Vec<Id> = (0..200).map(|_| id(next() % (CLUSTERS * SIZE))).collect();
    let mut times: Vec<f64> = Vec::new();
    let mut rows = 0usize;
    for &s in &starts {
        let t = Instant::now();
        let r = path_pairs(&g, &plus, PathEnds { s: Some(s), o: None }).unwrap();
        times.push(t.elapsed().as_secs_f64());
        rows += r.len();
    }
    times.sort_by(f64::total_cmp);
    let after = times[times.len() / 2];
    eprintln!("bound-subject pushdown: median {:.3?} over {} starts ({} total rows)",
        std::time::Duration::from_secs_f64(after), starts.len(), rows);

    // BEFORE: the previous algorithm — full all-pairs closure, then filter.
    let t = Instant::now();
    let full = transitive_closure_pairs(path_pairs(&g, &knows, PathEnds::NONE).unwrap());
    let before = t.elapsed().as_secs_f64();
    let s0 = starts[0];
    let filtered = full.iter().filter(|&&(x, _)| x == s0).count();
    eprintln!("full closure (old algorithm): {:.3?} ({} pairs; {} for one start)",
        std::time::Duration::from_secs_f64(before), full.len(), filtered);
    eprintln!("PAIRED RATIO full-closure / single-source = {:.0}x", before / after);

    // Both-bound reachability with early exit (hit and miss), and bound object.
    let (a, hit, miss) = (id(0), id(SIZE - 1), id(SIZE)); // same cluster / next cluster
    let t = Instant::now();
    for _ in 0..100 {
        let _ = path_pairs(&g, &plus, PathEnds { s: Some(a), o: Some(hit) }).unwrap();
    }
    eprintln!("both-bound (hit, early exit): {:.3?}/iter", t.elapsed() / 100);
    let t = Instant::now();
    for _ in 0..100 {
        let r = path_pairs(&g, &plus, PathEnds { s: Some(a), o: Some(miss) }).unwrap();
        assert!(!r.iter().any(|&(_, y)| y == miss));
    }
    eprintln!("both-bound (miss, cluster exhausted): {:.3?}/iter", t.elapsed() / 100);
    let t = Instant::now();
    for _ in 0..100 {
        let _ = path_pairs(&g, &plus, PathEnds { s: None, o: Some(a) }).unwrap();
    }
    eprintln!("bound-object reverse traversal: {:.3?}/iter", t.elapsed() / 100);
}
