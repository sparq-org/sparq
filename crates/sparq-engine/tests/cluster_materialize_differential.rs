//! (sq-7d3dj.30.14) Differential + mutation tests for the membership-cluster
//! pre-materialisation planner path (`cluster-materialize` feature, SP2Bench q07).
//!
//! The optimisation is a pure JOIN-ORDER choice: when a BGP has the q07 shape (one
//! UNBOUND-predicate container-membership pattern `?bag ?member ?doc` + a small
//! BOUND-predicate anchor `?doc2 references ?bag`) the executor evaluates the
//! {anchor, membership} cluster standalone and natural-joins it to the rest, instead
//! of bind-joining the wide membership relation per driver binding. The result bag
//! MUST be identical to the naive greedy plan.
//!
//! Because the feature is a COMPILE-TIME flag, we cannot run the cluster path and the
//! naive path in the same binary. Instead we exploit a data invariant: on a graph
//! where the membership predicate is ALWAYS the single IRI `ex:member`, the
//! UNBOUND-predicate pattern `?bag ?member ?doc` (which TRIGGERS the cluster path when
//! the feature is on) and the BOUND-predicate pattern `?bag ex:member ?doc` (which
//! NEVER triggers it) return the IDENTICAL bag of rows. Comparing the two queries on
//! the same graph is therefore a true differential of the cluster path vs an
//! un-clustered plan — and it runs in BOTH feature states (feature-OFF is the naive
//! baseline for both queries, so the test still asserts a real equivalence).
//!
//! Randomised over graph shape + sizes; a mutation check confirms the assertion is not
//! vacuous. The file compiles clean in BOTH feature states.

use sparq_core::Graph;
use sparq_engine::query;

const PFX: &str = "PREFIX ex: <http://ex/> \
                   PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#> \
                   PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
                   PREFIX foaf: <http://xmlns.com/foaf/0.1/> \
                   PREFIX dc: <http://purl.org/dc/elements/1.1/> \
                   PREFIX dct: <http://purl.org/dc/terms/> ";

const UNBOUND: &str = "\0\u{1}unbound\u{1}\0";

type Row = Vec<(String, String)>;

/// Order-independent bag of `(variable, value)` rows. When the `cluster-materialize`
/// feature is compiled, `force_cluster` installs the permissive test thresholds so the
/// small-graph query actually TRIGGERS the cluster path (`eval_bgp_cluster`) — exercising
/// the REAL executor path, not just the pure `detect` shape test. Feature-off, the flag is
/// inert and both queries run the naive plan (still a valid equivalence check that the file
/// compiles + runs in BOTH states).
fn result_bag_opt(graph: &Graph, q: &str, force_cluster: bool) -> Vec<Row> {
    let full = format!("{}{}", PFX, q);
    let run = || query(graph, &full).unwrap();
    #[cfg(feature = "cluster-materialize")]
    let r = if force_cluster { sparq_engine::with_test_thresholds(run) } else { run() };
    #[cfg(not(feature = "cluster-materialize"))]
    let r = {
        let _ = force_cluster;
        run()
    };
    let vars: Vec<String> = r.vars.iter().map(|v| v.as_str().to_string()).collect();
    let mut bag: Vec<Row> = r
        .rows
        .iter()
        .map(|row| {
            let mut cells: Row = vars
                .iter()
                .zip(row.iter())
                .map(|(v, cell)| {
                    (
                        v.clone(),
                        cell.as_ref().map(|t| format!("{}", t)).unwrap_or_else(|| UNBOUND.to_string()),
                    )
                })
                .collect();
            cells.sort();
            cells
        })
        .collect();
    bag.sort();
    bag
}

/// Naive (un-forced) bag — used for the bound-predicate reference side.
fn result_bag(graph: &Graph, q: &str) -> Vec<Row> {
    result_bag_opt(graph, q, false)
}

/// Cluster-forced bag — used for the unbound-predicate side so the cluster path fires.
fn result_bag_clustered(graph: &Graph, q: &str) -> Vec<Row> {
    result_bag_opt(graph, q, true)
}

/// A cheap deterministic LCG so the "randomised" shapes are reproducible without a
/// dependency. Returns a value in `0..n`.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self, n: usize) -> usize {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % n.max(1)
    }
}

/// Build a q07-shaped N-Triples graph: `n_classes` Document subclasses, `n_docs`
/// typed+titled documents, and a references→bag→member membership structure where
/// EVERY membership triple uses the single predicate `ex:member` (so bound and unbound
/// predicate patterns coincide). `seed` randomises which docs are typed, which bags
/// reference which docs, and bag membership fan-out (incl. some bags with several
/// members and some docs in several bags — non-contiguous, multiplicity-exercising).
fn build_graph(n_classes: usize, n_docs: usize, n_bags: usize, seed: u64) -> Graph {
    let mut rng = Lcg(seed);
    let mut nt = String::new();
    for c in 0..n_classes {
        nt.push_str(&format!(
            "<http://ex/Class{}> <http://www.w3.org/2000/01/rdf-schema#subClassOf> <http://xmlns.com/foaf/0.1/Document> .\n",
            c
        ));
    }
    for d in 0..n_docs {
        let c = rng.next(n_classes);
        nt.push_str(&format!(
            "<http://ex/d{}> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Class{}> .\n",
            d, c
        ));
        // ~80% of docs get a title.
        if rng.next(10) < 8 {
            nt.push_str(&format!("<http://ex/d{}> <http://purl.org/dc/elements/1.1/title> \"T{}\" .\n", d, d));
        }
    }
    for b in 0..n_bags {
        // Each bag is referenced by 0..2 documents (0 exercises the empty-reference case).
        let refs = rng.next(3);
        for _ in 0..refs {
            let rd = rng.next(n_docs.max(1));
            nt.push_str(&format!("<http://ex/d{}> <http://purl.org/dc/terms/references> <http://ex/bag{}> .\n", rd, b));
        }
        // Each bag has 0..3 members; some docs appear in several bags.
        let members = rng.next(4);
        for _ in 0..members {
            let md = rng.next(n_docs.max(1));
            nt.push_str(&format!("<http://ex/bag{}> <http://ex/member> <http://ex/d{}> .\n", b, md));
        }
    }
    Graph::load_reader(nt.as_bytes(), "ntriples").unwrap()
}

/// The q07 OUTER BGP shape with an UNBOUND membership predicate (TRIGGERS the cluster
/// path). `?title_or_star` lets us also test the DISTINCT-title projection.
fn q_unbound() -> &'static str {
    "SELECT ?doc ?title WHERE { \
       ?class rdfs:subClassOf foaf:Document . \
       ?doc rdf:type ?class . \
       ?doc dc:title ?title . \
       ?bag ?member ?doc . \
       ?doc2 dct:references ?bag }"
}

/// Same query with a BOUND membership predicate — NEVER triggers the cluster path.
/// Identical rows on a graph whose only membership predicate is `ex:member`.
fn q_bound() -> &'static str {
    "SELECT ?doc ?title WHERE { \
       ?class rdfs:subClassOf foaf:Document . \
       ?doc rdf:type ?class . \
       ?doc dc:title ?title . \
       ?bag ex:member ?doc . \
       ?doc2 dct:references ?bag }"
}

#[test]
fn cluster_path_matches_unclustered_across_random_shapes() {
    for seed in 0..40u64 {
        let n_classes = 1 + (seed as usize % 4);
        let n_docs = 8 + (seed as usize % 40);
        let n_bags = 4 + (seed as usize % 30);
        let g = build_graph(n_classes, n_docs, n_bags, seed.wrapping_mul(2654435761).wrapping_add(1));
        let unbound = result_bag_clustered(&g, q_unbound());
        let bound = result_bag(&g, q_bound());
        assert_eq!(
            unbound, bound,
            "cluster (unbound-predicate) path diverged from the un-clustered (bound-predicate) plan at seed {}",
            seed
        );
    }
}

/// The q07 query also nests OPTIONALs with the same shape; check the FULL nested query
/// (with `!bound` double negation) is result-stable between the two predicate forms.
#[test]
fn cluster_path_matches_in_nested_optional_shape() {
    let g = build_graph(3, 30, 20, 99);
    let unbound = result_bag_clustered(
        &g,
        "SELECT DISTINCT ?title WHERE { \
           ?class rdfs:subClassOf foaf:Document . \
           ?doc rdf:type ?class . \
           ?doc dc:title ?title . \
           ?bag2 ?member2 ?doc . \
           ?doc2 dct:references ?bag2 \
           OPTIONAL { \
             ?class3 rdfs:subClassOf foaf:Document . ?doc3 rdf:type ?class3 . \
             ?doc3 dct:references ?bag3 . ?bag3 ?member3 ?doc \
           } FILTER (!bound(?doc3)) }",
    );
    let bound = result_bag(
        &g,
        "SELECT DISTINCT ?title WHERE { \
           ?class rdfs:subClassOf foaf:Document . \
           ?doc rdf:type ?class . \
           ?doc dc:title ?title . \
           ?bag2 ex:member ?doc . \
           ?doc2 dct:references ?bag2 \
           OPTIONAL { \
             ?class3 rdfs:subClassOf foaf:Document . ?doc3 rdf:type ?class3 . \
             ?doc3 dct:references ?bag3 . ?bag3 ex:member ?doc \
           } FILTER (!bound(?doc3)) }",
    );
    assert_eq!(unbound, bound, "nested-OPTIONAL q07 shape diverged between cluster and un-clustered plans");
}

/// Multiplicity: a NON-distinct query over a graph with fan-out (a doc in several bags,
/// a bag with several members) must emit each row the same number of times on both
/// plans — the cluster materialisation must not dedup.
#[test]
fn cluster_path_preserves_multiplicity() {
    // Hand-built: bag1 has doc-a as member twice via two referrers; the join fans out.
    let nt = r#"<http://ex/C> <http://www.w3.org/2000/01/rdf-schema#subClassOf> <http://xmlns.com/foaf/0.1/Document> .
<http://ex/a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/C> .
<http://ex/a> <http://purl.org/dc/elements/1.1/title> "TA" .
<http://ex/bag1> <http://ex/member> <http://ex/a> .
<http://ex/r1> <http://purl.org/dc/terms/references> <http://ex/bag1> .
<http://ex/r2> <http://purl.org/dc/terms/references> <http://ex/bag1> .
<http://ex/bag2> <http://ex/member> <http://ex/a> .
<http://ex/r3> <http://purl.org/dc/terms/references> <http://ex/bag2> .
"#;
    let g = Graph::load_reader(nt.as_bytes(), "ntriples").unwrap();
    let unbound = result_bag_clustered(&g, q_unbound());
    let bound = result_bag(&g, q_bound());
    // Expect 3 rows (bag1 referenced twice → 2, bag2 once → 1) all binding ?doc=a.
    assert_eq!(unbound.len(), 3, "multiplicity: expected 3 rows, got {}", unbound.len());
    assert_eq!(unbound, bound, "multiplicity differed between cluster and un-clustered plans");
}

/// Mutation check: the differential assertion is NON-vacuous. If we compare against a
/// query with a DIFFERENT (genuinely distinct) membership predicate, the bags must
/// DIFFER — proving the equality assertions above test real, matching data.
#[test]
fn differential_is_non_vacuous() {
    let nt = r#"<http://ex/C> <http://www.w3.org/2000/01/rdf-schema#subClassOf> <http://xmlns.com/foaf/0.1/Document> .
<http://ex/a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/C> .
<http://ex/a> <http://purl.org/dc/elements/1.1/title> "TA" .
<http://ex/bag1> <http://ex/member> <http://ex/a> .
<http://ex/r1> <http://purl.org/dc/terms/references> <http://ex/bag1> .
"#;
    let g = Graph::load_reader(nt.as_bytes(), "ntriples").unwrap();
    let with_member = result_bag(&g, q_bound()); // uses ex:member — 1 row
    let with_other = result_bag(
        &g,
        "SELECT ?doc ?title WHERE { \
           ?class rdfs:subClassOf foaf:Document . ?doc rdf:type ?class . ?doc dc:title ?title . \
           ?bag ex:other ?doc . ?doc2 dct:references ?bag }",
    ); // uses ex:other — 0 rows (predicate absent)
    assert!(!with_member.is_empty(), "sanity: ex:member query should return rows");
    assert!(with_other.is_empty(), "sanity: ex:other query should return no rows");
    assert_ne!(with_member, with_other, "mutation check: differential must be able to detect a difference");
}
