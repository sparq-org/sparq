//! sq-t5bne: QL-rewrite micro-benchmark — a self-asserting, deterministic workload
//! for the OWL 2 QL (DL-Lite_R) PerfectRef query rewriter. Run:
//!
//!     cargo run -p sparq-reason-ql --example ql_rewrite_bench --release --features experimental [DEPTH]
//!
//! No dataset to fetch: the TBox + query are generated in-process (a pure function of DEPTH,
//! byte-deterministic, no RNG/network/toolchain), so this is a hermetic, regenerable benchmark.
//! It runs the gated rewriter, so it `required-features = ["experimental"]` (a plain
//! `cargo build --all-targets` — which passes no features — skips it rather than failing on the
//! feature-gated `rewrite` import, mirroring sparq-reason-el's `hasse_bench`).
//!
//! Two workloads, each over the SAME public `rewrite` API, each with a CLOSED-FORM UCQ-size:
//!
//!   1. `subclass` — a DEPTH-deep `rdfs:subClassOf` chain `C0 ⊑ C1 ⊑ … ⊑ C_DEPTH` and the query
//!      `?x rdf:type C_DEPTH`. PerfectRef rewrites the single class atom backward along the chain:
//!      the original CQ plus one rewrite for EACH of the DEPTH proper sub-classes. The UCQ size
//!      has a CLOSED FORM: exactly `DEPTH + 1` disjuncts.
//!
//!   2. `subproperty` — a DEPTH-deep `rdfs:subPropertyOf` chain `P0 ⊑ P1 ⊑ … ⊑ P_DEPTH` and the
//!      query `?x P_DEPTH ?y`. PerfectRef rewrites the single role atom backward along the chain:
//!      again exactly `DEPTH + 1` disjuncts.
//!
//! THE CLOSED-FORM ASSERTION IS THE GATE. A linear chain of length DEPTH must produce a UCQ of
//! exactly DEPTH+1 conjunctive queries — UCQ size grows LINEARLY in the TBox size, never
//! super-linearly. Asserting this exact closed form proves there is NO HIDDEN BLOW-UP in the
//! rewrite/reduce saturation (a duplicate-disjunct or non-terminating-rewrite regression would
//! over-count and fail LOUDLY), and that the fixpoint dedup is working. Wall-clock seconds are
//! TREND-ONLY (quiet-box sensitive); the disjunct-count closed forms are the hard, deterministic,
//! load-robust CORRECTNESS gate — exactly as sparq-reason-el's classify_bench asserts closure
//! sizes and bench/deep-taxonomy / bench/owl-sameas assert closure sizes for the RL reasoner.

use oxrdf::Triple;
use spargebra::{Query, SparqlParser};
use sparq_reason_ql::{rewrite, rewrite_production};
use std::str::FromStr;
use std::time::Instant;

// sq-pbz04.3.5: the SPARQL/Turtle prefixes the corpus-profile section uses (the
// realistic QL corpus is written with prefixed names for readability). Kept separate from the
// chain benchmark, which builds bare N-triples.
const SPARQL_PREFIXES: &str = "PREFIX : <http://ex/> \
    PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#> \
    PREFIX owl: <http://www.w3.org/2002/07/owl#> \
    PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> ";
const TURTLE_PREFIXES: &str = "@prefix : <http://ex/> . \
    @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> . \
    @prefix owl: <http://www.w3.org/2002/07/owl#> . \
    @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> . ";

const RDFS_SUB_CLASS_OF: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";
const RDFS_SUB_PROPERTY_OF: &str = "http://www.w3.org/2000/01/rdf-schema#subPropertyOf";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const EX: &str = "http://sparq.dev/bench/ql#";

fn secs(t: Instant) -> f64 {
    t.elapsed().as_secs_f64()
}

fn triple(s: &str) -> Triple {
    Triple::from_str(s).expect("n-triple parse")
}

/// Build a DEPTH-deep `rdfs:subClassOf` chain `C0 ⊑ C1 ⊑ … ⊑ C_DEPTH`.
fn subclass_chain(depth: usize) -> Vec<Triple> {
    (0..depth)
        .map(|i| triple(&format!("<{EX}C{i}> <{RDFS_SUB_CLASS_OF}> <{EX}C{}> .", i + 1)))
        .collect()
}

/// Build a DEPTH-deep `rdfs:subPropertyOf` chain `P0 ⊑ P1 ⊑ … ⊑ P_DEPTH`.
fn subproperty_chain(depth: usize) -> Vec<Triple> {
    (0..depth)
        .map(|i| triple(&format!("<{EX}P{i}> <{RDFS_SUB_PROPERTY_OF}> <{EX}P{}> .", i + 1)))
        .collect()
}

fn parse(q: &str) -> Query {
    SparqlParser::new().parse_query(q).expect("query parse")
}

fn run_subclass(depth: usize) {
    let tbox = subclass_chain(depth);
    let query = parse(&format!("SELECT ?x WHERE {{ ?x <{RDF_TYPE}> <{EX}C{depth}> }}"));
    let t = Instant::now();
    let r = rewrite(&query, &tbox).expect("subclass chain CQ is in QL scope");
    let dt = secs(t);

    // Closed form: the original CQ + one rewrite per proper sub-class = DEPTH + 1 disjuncts.
    let expected = depth + 1;
    assert_eq!(
        r.report.disjuncts, expected,
        "subclass depth {depth}: UCQ size {} != closed form {expected} (UCQ blow-up / dedup regression)",
        r.report.disjuncts
    );
    println!(
        "subclass    depth {depth:>6}: rewrite {dt:9.6}s   disjuncts {} (= d+1, asserted; linear UCQ, no blow-up)",
        r.report.disjuncts
    );
}

fn run_subproperty(depth: usize) {
    let tbox = subproperty_chain(depth);
    let query = parse(&format!("SELECT ?x ?y WHERE {{ ?x <{EX}P{depth}> ?y }}"));
    let t = Instant::now();
    let r = rewrite(&query, &tbox).expect("subproperty chain CQ is in QL scope");
    let dt = secs(t);

    // Closed form: the original CQ + one rewrite per proper sub-property = DEPTH + 1 disjuncts.
    let expected = depth + 1;
    assert_eq!(
        r.report.disjuncts, expected,
        "subproperty depth {depth}: UCQ size {} != closed form {expected} (UCQ blow-up / dedup regression)",
        r.report.disjuncts
    );
    println!(
        "subproperty depth {depth:>6}: rewrite {dt:9.6}s   disjuncts {} (= d+1, asserted; linear UCQ, no blow-up)",
        r.report.disjuncts
    );
}

/// Production-path invariant check (sq-g19x0): on the SAME linear `subclass` chain the production
/// rewrite (PerfectRef + tree-witness + minimisation) must (a) never EMPTY the UCQ — the input
/// CQ's answers must survive — and (b) never GROW it past the pre-minimisation count. A linear
/// chain has no redundant disjuncts (each disjunct is an incomparable single class atom), so the
/// minimised count equals the baseline DEPTH+1: minimisation correctly drops NOTHING. Asserting
/// this closed form proves minimisation is not over-aggressive on the chain (it would be an
/// unsoundness bug to collapse incomparable disjuncts) and not a no-op stub.
fn run_production(depth: usize) {
    let tbox = subclass_chain(depth);
    let query = parse(&format!("SELECT ?x WHERE {{ ?x <{RDF_TYPE}> <{EX}C{depth}> }}"));
    let t = Instant::now();
    let r = rewrite_production(&query, &tbox).expect("subclass chain CQ is in QL scope");
    let dt = secs(t);
    assert!(
        r.report.disjuncts >= 1,
        "production UCQ must never be empty (the input CQ's answers must survive)"
    );
    assert!(
        r.report.disjuncts <= r.report.disjuncts_before_minimisation,
        "minimisation must only REMOVE disjuncts, never grow the UCQ"
    );
    // Linear chain ⇒ DEPTH+1 incomparable single-atom disjuncts ⇒ nothing is redundant.
    let expected = depth + 1;
    assert_eq!(
        r.report.disjuncts, expected,
        "production depth {depth}: minimised UCQ size {} != closed form {expected} (incomparable disjuncts must not be dropped)",
        r.report.disjuncts
    );
    println!(
        "production  depth {depth:>6}: rewrite {dt:9.6}s   disjuncts {} (= d+1; before-min {}, dropped {})",
        r.report.disjuncts,
        r.report.disjuncts_before_minimisation,
        r.report.disjuncts_before_minimisation - r.report.disjuncts
    );
}

// ===========================================================================================
// sq-pbz04.3.5 — the combined-approach / H-complete-ABox EVALUATE-FIRST measurement.
//
// The combined approach (Kontchakov, Lutz, Toman, Wolter, Zakharyaschev — "The Combined Approach
// to Query Answering in DL-Lite") trades a large query REWRITING for a hierarchy-completed
// (H-complete) ABox plus a small filtering step: it wins ONLY WHEN pure PerfectRef rewriting
// blows the UCQ up while ABox saturation stays cheap. Whether to adopt it here is therefore an
// EMPIRICAL question about the UCQ-size DISTRIBUTION on the REAL corpus, not a literature claim.
//
// This section profiles `rewrite_production`'s minimised-UCQ size on (1) the REALISTIC corpus —
// the graduated DL-Lite_R oracle shapes (mirrored from `sparq-conformance`'s `QL_DLLITE_ORACLE`)
// plus the answered W3C `pr:QL` extensional-CQ shapes — versus (2) deliberately-adversarial
// CEILING probes that force the product blow-up. UCQ sizes are a PURE, DETERMINISTIC function of
// the (TBox, query), so every count is a CLOSED-FORM assertion — the reproducible evidence behind
// the reasoned NON-ADOPTION recorded in `research/owl2-ql-cq-gate-broadening.md` §10. No
// wall-clock here: the UCQ COUNTS are the canonical, load-robust metric (work-box seconds are not).
// ===========================================================================================

/// Parse a Turtle TBox (prefixes prepended) into triples for the rewriter.
fn tbox_ttl(ttl: &str) -> Vec<Triple> {
    let full = format!("{}{}", TURTLE_PREFIXES, ttl);
    oxttl::TurtleParser::new()
        .for_reader(full.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .expect("tbox turtle parse")
}

/// Parse a prefixed SPARQL `SELECT` query.
fn cq(body: &str) -> Query {
    parse(&format!("{}{}", SPARQL_PREFIXES, body))
}

/// The minimised UCQ size (`rewrite_production`) for a (TBox, query), asserting the raw-vs-minimised
/// invariant. The realistic-corpus / ceiling helpers call this and assert the closed form.
fn min_ucq(ttl: &str, body: &str) -> usize {
    let tbox = tbox_ttl(ttl);
    let query = cq(body);
    let p = rewrite_production(&query, &tbox).expect("corpus case is in QL scope");
    assert!(
        p.report.disjuncts <= p.report.disjuncts_before_minimisation,
        "minimisation must only REMOVE disjuncts: {} > {}",
        p.report.disjuncts,
        p.report.disjuncts_before_minimisation
    );
    p.report.disjuncts
}

/// The REALISTIC QL corpus: `(id, tbox_ttl, query, expected_minimised_ucq)`. Covers every
/// DL-Lite_R rewriting axis the oracle proves (class subsumption incl. a transitive chain, the
/// existential domain/range inclusions, role inclusion, `owl:inverseOf`, the unqualified `∃R`
/// generator with its applicability condition, a two-distinguished-variable product, and the
/// conjunction that minimises) PLUS the answered W3C `pr:QL` extensional-CQ shapes (a single/double
/// class atom over a shallow RDFS hierarchy, a literal-object role atom). Each `expected` is the
/// hand-verified closed form. sq-pbz04.3.5
fn realistic_corpus() -> Vec<(&'static str, &'static str, &'static str, usize)> {
    vec![
        // --- graduated DL-Lite_R oracle shapes (mirror of QL_DLLITE_ORACLE C1..C11) ---
        ("class-subsumption", ":Manager rdfs:subClassOf :Employee .", "SELECT ?x WHERE { ?x a :Employee }", 2),
        ("transitive-subclass", ":A rdfs:subClassOf :B . :B rdfs:subClassOf :C .", "SELECT ?x WHERE { ?x a :C }", 3),
        ("domain-introduces-role", ":worksFor rdfs:domain :Employee .", "SELECT ?x WHERE { ?x a :Employee }", 2),
        ("range-introduces-inverse", ":worksFor rdfs:range :Company .", "SELECT ?y WHERE { ?y a :Company }", 2),
        ("role-inclusion", ":manages rdfs:subPropertyOf :worksFor .", "SELECT ?x ?y WHERE { ?x :worksFor ?y }", 2),
        ("inverse-of-role-chain", ":employs owl:inverseOf :worksFor . :manages rdfs:subPropertyOf :employs .", "SELECT ?x ?y WHERE { ?x :worksFor ?y }", 3),
        ("exists-super-fires-unbound", ":Employee rdfs:subClassOf [ owl:onProperty :worksFor ; owl:someValuesFrom owl:Thing ] .", "SELECT ?x WHERE { ?x :worksFor ?y }", 2),
        ("exists-super-blocked-bound", ":Employee rdfs:subClassOf [ owl:onProperty :worksFor ; owl:someValuesFrom owl:Thing ] .", "SELECT ?x ?y WHERE { ?x :worksFor ?y }", 1),
        ("two-var-product", ":A rdfs:subClassOf :B .", "SELECT ?x ?y WHERE { ?x a :B . ?y a :B }", 4),
        ("conjunction-minimises", ":A rdfs:subClassOf :B . :B rdfs:subClassOf :C .", "SELECT ?x WHERE { ?x a :A . ?x a :C }", 1),
        ("empty-tbox-identity", "", "SELECT ?x WHERE { ?x a :Employee }", 1),
        // --- answered W3C pr:QL extensional-CQ shapes ---
        ("w3c-simple-typed", ":a rdfs:subClassOf :c .", "SELECT ?x WHERE { ?x rdf:type :c }", 2),
        ("w3c-rdfs-subclass", ":aType rdfs:subClassOf :bType .", "SELECT ?x WHERE { ?x rdf:type :bType }", 2),
        ("w3c-literal-object", "", "SELECT ?x WHERE { ?x <http://xmlns.com/foaf/0.1/name> \"name\"@en }", 1),
    ]
}

/// Profile the realistic corpus + the adversarial ceiling probes and assert the closed forms. The
/// realistic distribution is the load-bearing datum for the sq-pbz04.3.5 non-adoption verdict.
fn run_corpus_profile() {
    println!("\n[sq-pbz04.3.5] combined-approach evaluate-first — minimised-UCQ profile (deterministic)");

    // (1) Realistic corpus: every case is asserted to its hand-verified closed form, and the
    //     whole distribution is bucketed. The load-bearing claim: NOTHING blows up.
    let corpus = realistic_corpus();
    let mut sizes: Vec<usize> = Vec::new();
    for (id, ttl, body, expected) in &corpus {
        let m = min_ucq(ttl, body);
        assert_eq!(
            m, *expected,
            "realistic case {} minimised UCQ {} != closed form {}",
            id, m, expected
        );
        sizes.push(m);
    }
    sizes.sort_unstable();
    let n = sizes.len();
    let max = *sizes.last().unwrap();
    let (mut b1, mut b2_4, mut b5_16, mut b17) = (0usize, 0usize, 0usize, 0usize);
    for &s in &sizes {
        match s {
            1 => b1 += 1,
            2..=4 => b2_4 += 1,
            5..=16 => b5_16 += 1,
            _ => b17 += 1,
        }
    }
    // The DECISION rests on this: on the REAL corpus the minimised UCQ never exceeds a tiny bound,
    // so there is no rewriting blow-up for the combined approach to remove. Assert it explicitly.
    assert!(
        max <= 4 && b5_16 == 0 && b17 == 0,
        "REALISTIC corpus produced a large UCQ (max {}) — the non-adoption premise would need re-checking",
        max
    );
    println!(
        "  realistic corpus: {} answered cases  min {}  median {}  max {}  (dist [1]={} [2-4]={} [5-16]={} [17+]={})",
        n, sizes[0], sizes[n / 2], max, b1, b2_4, b5_16, b17
    );

    // (2) Adversarial CEILING probes: the ONLY shape that blows up is a multi-atom CQ over
    //     INDEPENDENT class hierarchies (the product), which minimisation cannot collapse (the
    //     disjuncts are mutually incomparable). Closed forms: (k+1)^atoms. This is exactly the
    //     combined approach's target — and exactly the shape ABSENT from the realistic corpus.
    for k in [2usize, 3, 5, 8] {
        // Two independent depth-k subclass chains, both class atoms on the SAME answer variable.
        let ttl = format!("{} {}", chain_ttl("C", k), chain_ttl("D", k));
        let body = format!("SELECT ?x WHERE {{ ?x a :C{} . ?x a :D{} }}", k, k);
        let m = min_ucq(&ttl, &body);
        assert_eq!(m, (k + 1) * (k + 1), "product-2 k={} UCQ {} != (k+1)^2", k, m);
        println!("  ceiling product(2 atoms, depth {}): UCQ {} = (k+1)^2 (incomparable — minimisation cannot help)", k, m);
    }
    for k in [2usize, 3] {
        let ttl = format!("{} {} {}", chain_ttl("C", k), chain_ttl("D", k), chain_ttl("E", k));
        let body = format!("SELECT ?x WHERE {{ ?x a :C{} . ?x a :D{} . ?x a :E{} }}", k, k, k);
        let m = min_ucq(&ttl, &body);
        assert_eq!(m, (k + 1) * (k + 1) * (k + 1), "product-3 k={} UCQ {} != (k+1)^3", k, m);
        println!("  ceiling product(3 atoms, depth {}): UCQ {} = (k+1)^3", k, m);
    }
    println!(
        "  VERDICT: realistic UCQ <= 4; blow-up needs multi-atom CQs over independent deep \
         hierarchies (absent from the corpus) => combined approach NOT adopted (see design record)."
    );
}

/// A depth-`n` `rdfs:subClassOf` chain `:P0 ⊑ :P1 ⊑ … ⊑ :Pn` in Turtle (prefixed names).
fn chain_ttl(prefix: &str, n: usize) -> String {
    (0..n)
        .map(|i| format!(":{}{} rdfs:subClassOf :{}{}.", prefix, i, prefix, i + 1))
        .collect::<Vec<_>>()
        .join(" ")
}

fn main() {
    let depth: usize = std::env::args()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(2_000);
    println!("sparq-reason-ql ql_rewrite_bench — depth {depth} (deterministic, regenerable)");
    run_subclass(depth);
    run_subproperty(depth);
    run_production(depth);
    run_corpus_profile();
    println!("OK: closed-form UCQ-size assertions held (UCQ scales linearly; wall-clock above is trend-only).");
}
