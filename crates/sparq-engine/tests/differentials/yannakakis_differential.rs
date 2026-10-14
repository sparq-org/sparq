//! (sq-5zf8i / survey §A4) Feature-on vs feature-off equivalence for the opt-in
//! `yannakakis` bottom-up full-semijoin PREPASS for acyclic BGPs.
//!
//! The prepass (`exec::eval_bgp_yannakakis`, with the pure join-tree topology in
//! `semijoin::build_join_tree`) reduces every relation of an acyclic BGP against its
//! neighbours in the join tree BEFORE the main join — a bottom-up then top-down semijoin
//! sweep using the same exact `semijoin::KeyFilter` membership test the A3 reducer uses.
//!
//! Its load-bearing CORRECTNESS CONTRACT is that a semijoin is a pure FILTER: it removes
//! ONLY rows the final join would itself drop, so the RESULT of a query is IDENTICAL
//! whether the feature is on or off — only fewer rows are materialised by the join.
//!
//! This file is DELIBERATELY NOT feature-gated, so it runs in BOTH states: the default
//! `cargo test` (feature OFF) validates the answers via the binary executor, and the
//! `opt-in sparq-engine (yannakakis)` matrix leg (feature ON) re-runs the SAME assertions
//! against the prepass path. Each query's result is compared against an INDEPENDENT
//! brute-force reference computed here (a hand nested-loop join over the triples that
//! shares NO code with the engine), so a divergence on either build fails loudly — that
//! cross-build identity IS the on-vs-off equivalence proof.
//!
//! The shapes cover the §A4 targets: chain, star, snowflake, a CYCLIC triangle (which must
//! route to LFTJ, never the prepass), an EMPTY/disjoint join, a no-reduction case (the
//! prepass removes nothing), and both the SMALL regime (below the cost-gate, where the
//! prepass falls back to the binary plan) and a LARGE regime (above the gate, where the
//! reduction actually engages). Both regimes must give identical answers.

use sparq_core::Graph;
use sparq_engine::query;

const PFX: &str = "PREFIX ex: <http://ex/> PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";

/// A triple `(s, p, o)` of N-Triples lexical forms.
type T = (String, String, String);

/// Run `q` (with the shared prefixes) and return its solutions as a SORTED bag of
/// `(var, lexical)` rows — order-independent (the join order is never observable) but exact
/// on bound terms.
fn result_bag(graph: &Graph, q: &str) -> Vec<Vec<(String, String)>> {
    let r = query(graph, &format!("{PFX}{q}")).unwrap();
    let vars: Vec<String> = r.vars.iter().map(|v| v.as_str().to_string()).collect();
    let mut bag: Vec<Vec<(String, String)>> = r
        .rows
        .iter()
        .map(|row| {
            let mut cells: Vec<(String, String)> = vars
                .iter()
                .zip(row.iter())
                .map(|(v, cell)| {
                    let lex = cell.as_ref().map(|t| t.to_string()).unwrap_or_default();
                    (v.clone(), lex)
                })
                .collect();
            cells.sort();
            cells
        })
        .collect();
    bag.sort();
    bag
}

/// Brute-force reference evaluator for a conjunctive BGP over the given triples. Each
/// pattern element is either a constant (the exact lexical form in the triple) or a `?var`.
/// Returns the SORTED bag of `(var, lexical)` rows — a dead-simple nested-loop join sharing
/// NO code with the engine under test.
fn brute(triples: &[T], patterns: &[[&str; 3]]) -> Vec<Vec<(String, String)>> {
    let mut partials: Vec<Vec<(String, String)>> = vec![vec![]];
    for pat in patterns {
        let mut next: Vec<Vec<(String, String)>> = vec![];
        for b in &partials {
            for (s, p, o) in triples {
                let cells = [s.as_str(), p.as_str(), o.as_str()];
                let mut nb = b.clone();
                let mut ok = true;
                for (slot, &val) in pat.iter().zip(cells.iter()) {
                    if let Some(var) = slot.strip_prefix('?') {
                        if let Some((_, bound)) = nb.iter().find(|(v, _)| v == var) {
                            if bound != val {
                                ok = false;
                                break;
                            }
                        } else {
                            nb.push((var.to_string(), val.to_string()));
                        }
                    } else if *slot != val {
                        ok = false;
                        break;
                    }
                }
                if ok {
                    next.push(nb);
                }
            }
        }
        partials = next;
    }
    let mut bag: Vec<Vec<(String, String)>> = partials
        .into_iter()
        .map(|mut row| {
            row.sort();
            row
        })
        .collect();
    bag.sort();
    bag
}

/// Load N-Triples from `(s, p, o)` lexical triples.
fn load(triples: &[T]) -> Graph {
    let mut nt = String::new();
    for (s, p, o) in triples {
        nt.push_str(s);
        nt.push(' ');
        nt.push_str(p);
        nt.push(' ');
        nt.push_str(o);
        nt.push_str(" .\n");
    }
    Graph::load_str(&nt, "ntriples").unwrap()
}

/// Re-project a brute-force bag down to the named variable columns (and re-sort), so it can
/// be compared against an engine result that projects fewer variables.
fn project(bag: Vec<Vec<(String, String)>>, keep: &[&str]) -> Vec<Vec<(String, String)>> {
    let mut out: Vec<Vec<(String, String)>> = bag
        .into_iter()
        .map(|row| {
            let mut r: Vec<(String, String)> = row.into_iter().filter(|(v, _)| keep.contains(&v.as_str())).collect();
            r.sort();
            r
        })
        .collect();
    out.sort();
    out
}

/// A snowflake/star dataset: `n` persons each with a `name`, an `age`, and a `city`; each
/// `city` has a `country`. A star join on `?p` plus a chain `?p -> ?city -> ?country`.
/// Sized by `n`: pass an `n` >= 4096 to push the per-person relations over the prepass
/// cost-gate (≥4096 rows) so the reduction actually engages under the feature.
fn social_dataset(n: usize) -> Vec<T> {
    let mut v: Vec<T> = vec![];
    for i in 0..n {
        let p = format!("<http://ex/p{i}>");
        v.push((p.clone(), "<http://ex/name>".into(), format!("\"name{i}\"")));
        v.push((
            p.clone(),
            "<http://ex/age>".into(),
            format!("\"{}\"^^<http://www.w3.org/2001/XMLSchema#integer>", i % 50),
        ));
        v.push((p, "<http://ex/city>".into(), format!("<http://ex/city{}>", i % 7)));
    }
    for c in 0..7 {
        v.push((format!("<http://ex/city{c}>"), "<http://ex/country>".into(), format!("<http://ex/country{}>", c % 3)));
    }
    v
}

/// CHAIN: `?a -> ?b -> ?c -> ?d` over a long line graph. The classic Yannakakis case where
/// the largest binary intermediate dwarfs the final result. The prepass reduces each link
/// to the tuples that reach the end; the answer must equal the brute-force bag.
#[test]
fn chain_join_matches_brute_force() {
    // A line of `len` nodes plus a fan of dead-ends at each step (rows that join nowhere
    // downstream — exactly the dangling tuples the full reducer removes). `len*2` ex:next
    // triples must clear the prepass cost-gate (>=4096) so the reduction actually engages
    // under the feature (else this would silently exercise only the binary fallback).
    let len = 2200usize;
    let mut triples: Vec<T> = vec![];
    for i in 0..len {
        triples.push((format!("<http://ex/e/{i}>"), "<http://ex/next>".into(), format!("<http://ex/e/{}>", i + 1)));
        // A dead-end edge out of node i that has no outgoing `next` (drops in the join).
        triples.push((format!("<http://ex/e/{i}>"), "<http://ex/next>".into(), format!("<http://ex/dead/{i}>")));
    }
    let g = load(&triples);
    let engine = result_bag(
        &g,
        "SELECT ?a ?b ?c ?d WHERE { ?a ex:next ?b . ?b ex:next ?c . ?c ex:next ?d }",
    );
    let reference = brute(
        &triples,
        &[
            ["?a", "<http://ex/next>", "?b"],
            ["?b", "<http://ex/next>", "?c"],
            ["?c", "<http://ex/next>", "?d"],
        ],
    );
    assert_eq!(engine, reference, "chain result must equal the brute-force bag");
    assert!(!engine.is_empty(), "non-vacuous");
}

/// STAR (large): three patterns sharing `?p`, over enough persons to clear the cost-gate so
/// the prepass engages. Answer equals the brute-force bag (only non-joining rows dropped).
#[test]
fn star_join_large_matches_brute_force() {
    // n persons => n triples per per-person predicate; >=4096 clears the cost-gate so the
    // prepass genuinely engages (not the binary fallback) under the feature-on leg.
    let triples = social_dataset(4200);
    let g = load(&triples);
    let engine = result_bag(
        &g,
        "SELECT ?p ?name ?age WHERE { ?p ex:name ?name . ?p ex:age ?age . ?p ex:city ?c }",
    );
    let reference = project(
        brute(
            &triples,
            &[
                ["?p", "<http://ex/name>", "?name"],
                ["?p", "<http://ex/age>", "?age"],
                ["?p", "<http://ex/city>", "?c"],
            ],
        ),
        &["p", "name", "age"],
    );
    assert_eq!(engine, reference, "large star result must equal the brute-force bag");
    assert!(!engine.is_empty(), "non-vacuous");
}

/// STAR (small): same shape but below the cost-gate, so the feature-ON build falls back to
/// the binary plan. Still must equal the brute-force bag — the fallback is result-identical.
#[test]
fn star_join_small_matches_brute_force() {
    let triples = social_dataset(40);
    let g = load(&triples);
    let engine = result_bag(
        &g,
        "SELECT ?p ?name ?age WHERE { ?p ex:name ?name . ?p ex:age ?age . ?p ex:city ?c }",
    );
    let reference = project(
        brute(
            &triples,
            &[
                ["?p", "<http://ex/name>", "?name"],
                ["?p", "<http://ex/age>", "?age"],
                ["?p", "<http://ex/city>", "?c"],
            ],
        ),
        &["p", "name", "age"],
    );
    assert_eq!(engine, reference, "small star result must equal the brute-force bag");
    assert!(!engine.is_empty(), "non-vacuous");
}

/// SNOWFLAKE (large): star on `?p` joined to a chain `?p ex:city ?city . ?city ex:country
/// ?co`. `?city` connects a high-cardinality (per-person) and a low-cardinality (per-city)
/// relation — the regime the reducer targets. Above the cost-gate so the prepass engages.
#[test]
fn snowflake_join_matches_brute_force() {
    // >=4096 persons clears the cost-gate so the prepass engages on this snowflake shape.
    let triples = social_dataset(4200);
    let g = load(&triples);
    let engine = result_bag(
        &g,
        "SELECT ?p ?co WHERE { ?p ex:name ?n . ?p ex:city ?city . ?city ex:country ?co }",
    );
    let reference = project(
        brute(
            &triples,
            &[
                ["?p", "<http://ex/name>", "?n"],
                ["?p", "<http://ex/city>", "?city"],
                ["?city", "<http://ex/country>", "?co"],
            ],
        ),
        &["p", "co"],
    );
    assert_eq!(engine, reference, "snowflake result must equal the brute-force bag");
    assert!(!engine.is_empty(), "non-vacuous");
}

/// NO-REDUCTION: every tuple already joins (a complete star where each person has all three
/// attributes). The prepass removes nothing; the answer must be unchanged and complete.
#[test]
fn no_reduction_case_matches_brute_force() {
    // >=4096 persons clears the cost-gate so the prepass engages even when it removes nothing.
    let triples = social_dataset(4200);
    let g = load(&triples);
    let engine = result_bag(
        &g,
        "SELECT ?p ?name ?age ?c WHERE { ?p ex:name ?name . ?p ex:age ?age . ?p ex:city ?c }",
    );
    let reference = brute(
        &triples,
        &[
            ["?p", "<http://ex/name>", "?name"],
            ["?p", "<http://ex/age>", "?age"],
            ["?p", "<http://ex/city>", "?c"],
        ],
    );
    assert_eq!(engine, reference, "complete star: prepass removes nothing");
    assert_eq!(engine.len(), 4200, "every person contributes exactly one row");
}

/// CYCLIC triangle: `?x->?y, ?y->?z, ?z->?x`. A cyclic BGP must route to LFTJ, NOT the
/// prepass (the prepass only runs on the acyclic branch). The answer must equal brute force
/// either way — this guards the routing invariant.
#[test]
fn cyclic_triangle_matches_brute_force() {
    // A small graph with exactly one directed triangle a->b->c->a plus distractor edges.
    let triples: Vec<T> = vec![
        ("<http://ex/a>".into(), "<http://ex/e>".into(), "<http://ex/b>".into()),
        ("<http://ex/b>".into(), "<http://ex/e>".into(), "<http://ex/c>".into()),
        ("<http://ex/c>".into(), "<http://ex/e>".into(), "<http://ex/a>".into()),
        ("<http://ex/a>".into(), "<http://ex/e>".into(), "<http://ex/d>".into()), // dangling
        ("<http://ex/d>".into(), "<http://ex/e>".into(), "<http://ex/a>".into()),
    ];
    let g = load(&triples);
    let engine = result_bag(&g, "SELECT ?x ?y ?z WHERE { ?x ex:e ?y . ?y ex:e ?z . ?z ex:e ?x }");
    let reference = brute(
        &triples,
        &[
            ["?x", "<http://ex/e>", "?y"],
            ["?y", "<http://ex/e>", "?z"],
            ["?z", "<http://ex/e>", "?x"],
        ],
    );
    assert_eq!(engine, reference, "cyclic triangle (LFTJ path) must equal brute force");
    assert!(!engine.is_empty(), "the triangle exists");
}

/// EMPTY/disjoint: a connecting variable shares NO values across the two sides => empty
/// result. The prepass must not invent rows, and must not crash on an empty membership set.
#[test]
fn disjoint_join_is_empty_both_ways() {
    let triples: Vec<T> = vec![
        ("<http://ex/a>".into(), "<http://ex/r>".into(), "<http://ex/x>".into()),
        ("<http://ex/b>".into(), "<http://ex/s>".into(), "<http://ex/y>".into()),
    ];
    let g = load(&triples);
    let engine = result_bag(&g, "SELECT ?m WHERE { ?a ex:r ?m . ?m ex:s ?o }");
    assert!(engine.is_empty(), "disjoint join is empty");
}

/// A FILTERed chain: a pushed-down sargable numeric FILTER plus a multi-pattern join. The
/// prepass threads the same pushed-down filter through its materialising scans, so the
/// answer must equal the brute-force bag (filter applied in the reference too).
#[test]
fn filtered_join_matches_brute_force() {
    // >=4096 persons: the unfiltered ex:age/ex:name relations clear the cost-gate so the
    // prepass engages and threads the pushed-down sargable FILTER through its scans.
    let triples = social_dataset(4200);
    let g = load(&triples);
    // ages are i % 50; keep only ages < 10 (a selective sargable filter), joined to name.
    let engine = result_bag(
        &g,
        "SELECT ?p ?age ?name WHERE { ?p ex:age ?age . ?p ex:name ?name . FILTER(?age < 10) }",
    );
    // Reference: full join, then filter on the integer value of ?age < 10.
    let joined = brute(
        &triples,
        &[["?p", "<http://ex/age>", "?age"], ["?p", "<http://ex/name>", "?name"]],
    );
    let filtered: Vec<Vec<(String, String)>> = joined
        .into_iter()
        .filter(|row| {
            let age_lex = &row.iter().find(|(v, _)| v == "age").unwrap().1;
            // Lexical form is `"NN"^^<...integer>`; parse the leading quoted integer.
            let n: i64 = age_lex.trim_start_matches('"').split('"').next().unwrap().parse().unwrap();
            n < 10
        })
        .collect();
    let reference = project(filtered, &["p", "age", "name"]);
    assert_eq!(engine, reference, "filtered join must equal the brute-force bag");
    assert!(!engine.is_empty(), "non-vacuous");
}
