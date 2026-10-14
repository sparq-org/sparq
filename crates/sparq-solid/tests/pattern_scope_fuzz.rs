//! sq-8o74m — pattern-scope leakage-channel FUZZ battery over randomized masks.
//!
//! Hardens the fixed differential battery in `pattern_scope.rs`: for N seeded
//! pseudo-random (source-dataset, allow/deny scope map, query-battery) cases, the
//! scoped dataset must answer **row-identically** to an ORACLE `PodStore` loaded from
//! the source with the masked lines physically deleted — across every leakage channel
//! the design record (`research/odrl-pattern-scoped-targets-2026-07.md` §0) names:
//! SELECT rows, OPTIONAL, EXISTS, NOT EXISTS, MINUS, COUNT / GROUP BY aggregates, ASK,
//! `GRAPH ?g` enumeration, and property paths. Non-vacuity: whenever a case's mask is
//! non-trivial (≥ 1 triple masked), the scoped answers must ALSO differ from the
//! unmasked store — a no-op mask flips that differ-assertion red.
//!
//! Deterministic: a self-contained SplitMix64 generator seeded per case (NO new crate
//! dependency); every assertion message carries `case=<i> seed=<hex>` so a failure
//! reproduces exactly.

#![cfg(feature = "pattern-scope")]

use oxrdf::{Literal, NamedNode, Term};
use rustc_hash::FxHashMap;
use sparq_solid::{GraphScope, Mode, PodStore, ScopePattern, Session};

const NS: &str = "https://ex.dev/ns#";
const GRAPHS: [&str; 2] = ["https://pod.ex/g0", "https://pod.ex/g1"];
/// The predicate reserved for IRI→IRI edges (the property-path channel).
const PATH_PRED: usize = 2;
const N_SUBJECTS: u64 = 6;
const N_PREDS: u64 = 4;
const N_LITS: u64 = 6;
const CASES: u64 = 64;
const BASE_SEED: u64 = 0x5EED_CAFE_F00D_0001;

// ---------------------------------------------------------------------------
// Self-contained deterministic PRNG (SplitMix64 — public-domain algorithm).
// ---------------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n` (tiny `n`, modulo bias irrelevant for test-case shaping).
    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }

    /// `true` with probability `num/den`.
    fn chance(&mut self, num: u64, den: u64) -> bool {
        self.below(den) < num
    }
}

// ---------------------------------------------------------------------------
// Vocabulary — small on purpose so random patterns collide with random data.
// ---------------------------------------------------------------------------

fn subject(i: u64) -> Term {
    Term::NamedNode(NamedNode::new(format!("https://pod.ex/id#s{i}")).unwrap())
}

fn pred(i: u64) -> Term {
    Term::NamedNode(NamedNode::new(format!("{NS}p{i}")).unwrap())
}

fn lit(i: u64) -> Term {
    Term::Literal(Literal::new_simple_literal(format!("v{i}")))
}

fn iri(s: &str) -> Term {
    Term::NamedNode(NamedNode::new(s).unwrap())
}

/// A random object: IRI (from the subject vocab) or simple literal. The path
/// predicate always gets an IRI object so `<pN>+` chains stay well-formed.
fn object(rng: &mut Rng, p_idx: u64) -> Term {
    if p_idx == PATH_PRED as u64 || rng.chance(1, 3) {
        subject(rng.below(N_SUBJECTS))
    } else {
        lit(rng.below(N_LITS))
    }
}

// ---------------------------------------------------------------------------
// Case generation.
// ---------------------------------------------------------------------------

/// The pod's .acl: alice may Read everything under the pod root (subtree default) —
/// identical in the full and oracle stores, so it cancels in every differential.
fn acl_quads() -> String {
    let acl = "https://pod.ex/.acl";
    [
        (
            "http://www.w3.org/1999/02/22-rdf-syntax-ns#type",
            "<http://www.w3.org/ns/auth/acl#Authorization>",
        ),
        ("http://www.w3.org/ns/auth/acl#default", "<https://pod.ex/>"),
        (
            "http://www.w3.org/ns/auth/acl#agent",
            "<https://alice.ex/card#me>",
        ),
        (
            "http://www.w3.org/ns/auth/acl#mode",
            "<http://www.w3.org/ns/auth/acl#Read>",
        ),
    ]
    .iter()
    .map(|(p, o)| format!("<{acl}#r> <{p}> {o} <{acl}> .\n"))
    .collect()
}

/// One random triple pattern: each component wildcard or a concrete vocab term.
/// Wildcard probabilities are tuned so patterns frequently hit generated data.
fn random_pattern(rng: &mut Rng) -> ScopePattern {
    let s = if rng.chance(1, 2) {
        None
    } else {
        Some(subject(rng.below(N_SUBJECTS)))
    };
    let p = if rng.chance(2, 5) {
        None
    } else {
        Some(pred(rng.below(N_PREDS)))
    };
    let o = if rng.chance(3, 5) {
        None
    } else if rng.chance(1, 2) {
        Some(subject(rng.below(N_SUBJECTS)))
    } else {
        Some(lit(rng.below(N_LITS)))
    };
    ScopePattern::new(s, p, o)
}

/// A random scope for one graph: `None` (graph contributes whole), an ODRL-permission
/// `allow_only`, or an ODRL-prohibition `deny_within` — pattern count 1..=3.
fn random_scope(rng: &mut Rng) -> Option<GraphScope> {
    let n = 1 + rng.below(3);
    match rng.below(3) {
        0 => None,
        1 => Some(GraphScope::allow_only(
            (0..n).map(|_| random_pattern(rng)).collect(),
        )),
        _ => Some(GraphScope::deny_within(
            (0..n).map(|_| random_pattern(rng)).collect(),
        )),
    }
}

struct Case {
    /// Full source dataset (data + acl), N-Quads.
    full_nq: String,
    /// Oracle dataset: `full_nq` with the masked lines physically deleted.
    oracle_nq: String,
    /// The scope map handed to `scoped_dataset`.
    scopes: FxHashMap<Term, GraphScope>,
    /// How many data triples the scope map masks (0 ⇒ trivial mask).
    masked: usize,
}

fn generate(rng: &mut Rng) -> Case {
    let mut full_nq = String::new();
    let mut oracle_nq = String::new();
    let mut scopes = FxHashMap::default();
    let mut masked = 0usize;

    for g in GRAPHS {
        // Deduplicated random triples (RDF graphs are sets; dedup keeps the
        // full-vs-oracle line diff exactly equal to the masked-triple set).
        let n = 6 + rng.below(12);
        let mut triples: Vec<[Term; 3]> = Vec::new();
        while (triples.len() as u64) < n {
            let p_idx = rng.below(N_PREDS);
            let t = [
                subject(rng.below(N_SUBJECTS)),
                pred(p_idx),
                object(rng, p_idx),
            ];
            if !triples.contains(&t) {
                triples.push(t);
            }
        }
        let scope = random_scope(rng);
        for t in &triples {
            // oxrdf `Term` Display is canonical N-Triples, so the store round-trips
            // to term-identical triples and the host-side `visible()` decision below
            // agrees exactly with the one `scoped_dataset` makes after decoding.
            let line = format!("{} {} {} <{g}> .\n", t[0], t[1], t[2]);
            full_nq += &line;
            let visible = scope.as_ref().is_none_or(|sc| sc.visible(t));
            if visible {
                oracle_nq += &line;
            } else {
                masked += 1;
            }
        }
        if let Some(sc) = scope {
            scopes.insert(iri(g), sc);
        }
    }
    full_nq += &acl_quads();
    oracle_nq += &acl_quads();
    Case {
        full_nq,
        oracle_nq,
        scopes,
        masked,
    }
}

fn store(nq: &str) -> PodStore {
    let mut s = PodStore::new(sparq_core::Graph::load_dataset(nq, "nquads").unwrap());
    s.materialize_wac().unwrap();
    s
}

fn alice() -> Session<'static> {
    Session {
        agent: Some("https://alice.ex/card#me"),
        client: None,
        issuer: None,
        now: None,
    }
}

/// The per-case leakage-channel battery. Channel predicates are drawn per case
/// (randomized queries), the path channel always probes the IRI-edge predicate.
/// Every SELECT carries a total ORDER BY so JSON comparison is deterministic.
fn battery(rng: &mut Rng) -> Vec<(&'static str, String)> {
    let a = format!("{NS}p{}", rng.below(N_PREDS));
    let b = format!("{NS}p{}", rng.below(N_PREDS));
    let path = format!("{NS}p{PATH_PRED}");
    vec![
        ("select-all", "SELECT ?s ?p ?o WHERE { GRAPH ?g { ?s ?p ?o } } ORDER BY ?s ?p ?o".to_owned()),
        (
            "optional",
            format!(
                "SELECT ?s ?x ?y WHERE {{ GRAPH ?g {{ ?s <{a}> ?x OPTIONAL {{ ?s <{b}> ?y }} }} }} \
                 ORDER BY ?s ?x ?y"
            ),
        ),
        (
            "exists",
            format!(
                "SELECT ?s ?x WHERE {{ GRAPH ?g {{ ?s <{a}> ?x \
                 FILTER EXISTS {{ ?s <{b}> ?z }} }} }} ORDER BY ?s ?x"
            ),
        ),
        (
            "not-exists",
            format!(
                "SELECT ?s ?x WHERE {{ GRAPH ?g {{ ?s <{a}> ?x \
                 FILTER NOT EXISTS {{ ?s <{b}> ?z }} }} }} ORDER BY ?s ?x"
            ),
        ),
        (
            "minus",
            format!(
                "SELECT ?s WHERE {{ GRAPH ?g {{ {{ ?s <{a}> ?x }} MINUS {{ ?s <{b}> ?y }} }} }} \
                 ORDER BY ?s"
            ),
        ),
        ("count", "SELECT (COUNT(*) AS ?c) WHERE { GRAPH ?g { ?s ?p ?o } }".to_owned()),
        (
            "group-count",
            "SELECT ?s (COUNT(?p) AS ?c) WHERE { GRAPH ?g { ?s ?p ?o } } GROUP BY ?s ORDER BY ?s ?c"
                .to_owned(),
        ),
        ("graphs", "SELECT DISTINCT ?g WHERE { GRAPH ?g { ?s ?p ?o } } ORDER BY ?g".to_owned()),
        ("path", format!("SELECT ?x ?y WHERE {{ GRAPH ?g {{ ?x <{path}>+ ?y }} }} ORDER BY ?x ?y")),
    ]
}

/// Two per-case ASK probes: satisfiability of a random predicate and a random
/// transitive path between concrete endpoints.
fn asks(rng: &mut Rng) -> Vec<(&'static str, String)> {
    let p = format!("{NS}p{}", rng.below(N_PREDS));
    let from = subject(rng.below(N_SUBJECTS));
    let to = subject(rng.below(N_SUBJECTS));
    vec![
        ("ask-pred", format!("ASK {{ GRAPH ?g {{ ?s <{p}> ?o }} }}")),
        (
            "ask-path",
            format!("ASK {{ GRAPH ?g {{ {from} <{NS}p{PATH_PRED}>+ {to} }} }}"),
        ),
    ]
}

/// THE fuzz battery: oracle-equivalence under every leakage channel for every seeded
/// case, plus the per-case non-vacuity differ-assertion on non-trivial masks.
#[test]
fn fuzz_masked_equals_oracle_across_all_leakage_channels() {
    let session = alice();
    let mut nontrivial = 0u64;

    for case_idx in 0..CASES {
        let seed = BASE_SEED ^ case_idx.wrapping_mul(0xA24B_AED4_963E_E407);
        let mut rng = Rng::new(seed);
        let case = generate(&mut rng);
        let ctx = format!("case={case_idx} seed={seed:#x}");

        let full = store(&case.full_nq);
        let oracle = store(&case.oracle_nq);
        let scoped = full.scoped_dataset(&session, Mode::Read, &case.scopes);

        for (label, q) in battery(&mut rng) {
            let got = scoped.query_json(&q).unwrap();
            let want = oracle.query_json_as(&session, Mode::Read, &q).unwrap();
            assert_eq!(
                got, want,
                "leak via {label} ({ctx}): scoped != oracle\nquery: {q}"
            );
        }
        for (label, q) in asks(&mut rng) {
            let got = scoped.ask(&q).unwrap();
            let want = oracle.ask_as(&session, Mode::Read, &q).unwrap();
            assert_eq!(
                got, want,
                "leak via {label} ({ctx}): scoped != oracle\nquery: {q}"
            );
        }

        // Non-vacuity: a non-trivial mask MUST change the observable dataset — with a
        // no-op mask (scopes ignored / mask knocked out) this assertion goes red.
        if case.masked > 0 {
            nontrivial += 1;
            let q = "SELECT ?s ?p ?o WHERE { GRAPH ?g { ?s ?p ?o } } ORDER BY ?s ?p ?o";
            let scoped_json = scoped.query_json(q).unwrap();
            let unmasked = full.query_json_as(&session, Mode::Read, q).unwrap();
            assert_ne!(
                scoped_json, unmasked,
                "vacuous mask ({ctx}): masked {} triple(s) yet select-all unchanged",
                case.masked
            );
        }
    }

    // The battery itself must be non-trivial: a healthy majority of the seeded cases
    // must actually mask something (guards against a generator drift that would turn
    // the whole fuzz run into no-op-mask comparisons).
    assert!(
        nontrivial >= CASES / 2,
        "generator drift: only {nontrivial}/{CASES} cases had a non-trivial mask"
    );
}
