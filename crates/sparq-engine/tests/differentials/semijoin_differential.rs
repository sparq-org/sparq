//! (sq-gr8mb / survey §A3) Feature-on vs feature-off equivalence for the
//! opt-in `semijoin-bitmap` exact-bitmap semi-join reducer.
//!
//! The reducer (`semijoin::KeyFilter`, wired into `exec::eval_bgp_binary`) prefilters a
//! scan by a membership set built over the other join side's connecting-variable ids.
//! Its load-bearing CORRECTNESS CONTRACT is that the filter is membership-EXACT (no false
//! positives), so it removes ONLY rows that would fail the downstream join: the RESULT of
//! a query is IDENTICAL whether the feature is on or off — only fewer rows are scanned.
//!
//! This file is DELIBERATELY NOT feature-gated, so it runs in BOTH states: the default
//! `cargo test` (feature OFF) validates the answers via the existing executor, and the
//! `opt-in sparq-engine (semijoin-bitmap)` matrix leg (feature ON) re-runs the SAME
//! assertions against the prefiltered path.
//!
//! Each query's result is compared against an INDEPENDENT brute-force reference computed
//! here (a hand join over the triples), so a divergence on either build fails loudly —
//! that cross-build identity IS the feature-on-vs-off equivalence proof. The queries are
//! representative star / snowflake / chain shapes (the workloads §A3 targets), each with
//! enough patterns to drive the binary-join loop where the prefilter lives, and they
//! include cases where the prefilter would drop most rows (the win) and cases where it
//! drops none (no over-removal).

use sparq_core::Graph;
use sparq_engine::query;

const PFX: &str = "PREFIX ex: <http://ex/> PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";

/// Run `q` (with the shared prefixes) and return its solutions as a SORTED bag of
/// `(var, lexical)` rows — a representation that ignores row order (the join order is a
/// performance choice, never observable) but is exact on bound terms.
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
            // Sort cells WITHIN a row by variable name so the comparison is independent of
            // projection order (the brute-force reference is sorted the same way).
            cells.sort();
            cells
        })
        .collect();
    bag.sort();
    bag
}

/// A triple `(s, p, o)` of N-Triples lexical forms.
type T = (&'static str, &'static str, &'static str);

/// Brute-force reference evaluator for a conjunctive (BGP) query over the given triples,
/// with all-variable subjects/predicates/objects. Each pattern element is either a
/// constant (the same lexical form that appears in the triple) or a `?var` placeholder.
/// Returns the SORTED bag of `(var, lexical)` rows the BGP must produce — computed with a
/// dead-simple nested-loop join so it shares NO code with the engine under test.
fn brute(triples: &[T], patterns: &[[&str; 3]]) -> Vec<Vec<(String, String)>> {
    // Each partial binding is a map var -> lexical. Start with the empty binding.
    let mut partials: Vec<Vec<(String, String)>> = vec![vec![]];
    for pat in patterns {
        let mut next: Vec<Vec<(String, String)>> = vec![];
        for b in &partials {
            for &(s, p, o) in triples {
                let cells = [s, p, o];
                let mut nb = b.clone();
                let mut ok = true;
                for (slot, &val) in pat.iter().zip(cells.iter()) {
                    if let Some(var) = slot.strip_prefix('?') {
                        // Consistency: a repeated var must bind to the same value.
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
    // Normalise each row to a sorted (var, lexical) vector and sort the bag.
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
    for &(s, p, o) in triples {
        nt.push_str(&format!("{s} {p} {o} .\n"));
    }
    Graph::load_str(&nt, "ntriples").unwrap()
}

/// A snowflake/star dataset: many `person`s each with a `name`, an `age`, and a `city`;
/// each `city` has a `country`. A star join on `?p` plus a chain `?p -> ?city -> ?country`.
fn social_dataset(n: usize) -> Vec<T> {
    // Leak small per-row strings so the &'static lifetime holds for the test's duration.
    let mut v: Vec<T> = vec![];
    for i in 0..n {
        let p: &'static str = Box::leak(format!("<http://ex/p{i}>").into_boxed_str());
        let name: &'static str = Box::leak(format!("\"name{i}\"").into_boxed_str());
        let age: &'static str =
            Box::leak(format!("\"{}\"^^<http://www.w3.org/2001/XMLSchema#integer>", i % 50).into_boxed_str());
        let city: &'static str = Box::leak(format!("<http://ex/city{}>", i % 7).into_boxed_str());
        v.push((p, "<http://ex/name>", name));
        v.push((p, "<http://ex/age>", age));
        v.push((p, "<http://ex/city>", city));
    }
    for c in 0..7 {
        let city: &'static str = Box::leak(format!("<http://ex/city{c}>", c = c).into_boxed_str());
        let country: &'static str = Box::leak(format!("<http://ex/country{}>", c % 3).into_boxed_str());
        v.push((city, "<http://ex/country>", country));
    }
    v
}

/// CORE: a star join (three patterns sharing `?p`) returns exactly the brute-force bag —
/// the prefilter (built off the first materialised pattern) drops only non-joining rows.
#[test]
fn star_join_matches_brute_force() {
    let triples = social_dataset(60);
    let g = load(&triples);

    let engine = result_bag(
        &g,
        "SELECT ?p ?name ?age WHERE { ?p ex:name ?name . ?p ex:age ?age . ?p ex:city ?c }",
    );
    let reference = brute(
        &triples,
        &[
            ["?p", "<http://ex/name>", "?name"],
            ["?p", "<http://ex/age>", "?age"],
            ["?p", "<http://ex/city>", "?c"],
        ],
    );
    // The engine omits `?c` from projection; the reference includes it. Compare on the
    // projected columns by re-projecting the reference to {p, name, age}.
    let reference = project(reference, &["p", "name", "age"]);
    assert_eq!(engine, reference, "star-join result must equal the brute-force bag");
    assert!(!engine.is_empty(), "non-vacuous");
}

/// A snowflake: star on `?p` joined to a chain `?p ex:city ?city . ?city ex:country ?co`.
/// Exercises a join where the connecting variable `?city` lives in BOTH a high-cardinality
/// (per-person) and low-cardinality (per-city) relation — the regime the reducer targets.
#[test]
fn snowflake_join_matches_brute_force() {
    let triples = social_dataset(80);
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

/// A highly-selective object-bound join where the prefilter should drop most scanned
/// rows of the wide `ex:age` relation (only persons in `city0`), but the answer is still
/// exactly the brute-force bag (no over-removal).
#[test]
fn selective_join_matches_brute_force() {
    let triples = social_dataset(100);
    let g = load(&triples);

    let engine = result_bag(
        &g,
        "SELECT ?p ?age WHERE { ?p ex:city <http://ex/city0> . ?p ex:age ?age }",
    );
    let reference = project(
        brute(
            &triples,
            &[
                ["?p", "<http://ex/city>", "<http://ex/city0>"],
                ["?p", "<http://ex/age>", "?age"],
            ],
        ),
        &["p", "age"],
    );
    assert_eq!(engine, reference, "selective join result must equal the brute-force bag");
    assert!(!engine.is_empty(), "non-vacuous");
}

/// A join whose connecting variable shares NO values across the two sides => EMPTY result.
/// The prefilter must not invent rows, and must not crash on an empty membership set.
#[test]
fn disjoint_join_is_empty_both_ways() {
    let triples: &[T] = &[
        ("<http://ex/a>", "<http://ex/r>", "<http://ex/x>"),
        ("<http://ex/b>", "<http://ex/s>", "<http://ex/y>"),
    ];
    let g = load(triples);
    // `?m` is bound by ex:r's object (x) and required to be ex:s's subject (b) — disjoint.
    let engine = result_bag(&g, "SELECT ?m WHERE { ?a ex:r ?m . ?m ex:s ?o }");
    assert!(engine.is_empty(), "disjoint join is empty");
}

/// A self-join with a repeated variable inside one pattern (`?x ex:eq ?x`) plus a chain,
/// so the prefilter's single-position membership check coexists with `build_row`'s
/// repeated-variable consistency check.
#[test]
fn repeated_var_pattern_matches_brute_force() {
    let triples: &[T] = &[
        ("<http://ex/a>", "<http://ex/eq>", "<http://ex/a>"), // a eq a (passes self-eq)
        ("<http://ex/b>", "<http://ex/eq>", "<http://ex/c>"), // b eq c (fails self-eq)
        ("<http://ex/a>", "<http://ex/p>", "<http://ex/v1>"),
        ("<http://ex/b>", "<http://ex/p>", "<http://ex/v2>"),
    ];
    let g = load(triples);
    let engine = result_bag(&g, "SELECT ?x ?v WHERE { ?x ex:eq ?x . ?x ex:p ?v }");
    let reference = project(
        brute(
            triples,
            &[["?x", "<http://ex/eq>", "?x"], ["?x", "<http://ex/p>", "?v"]],
        ),
        &["x", "v"],
    );
    assert_eq!(engine, reference, "repeated-var self-join must equal brute force");
    // Only `a` self-eq, so exactly one row.
    assert_eq!(engine.len(), 1, "exactly the self-equal subject survives");
}

/// Re-project a brute-force bag down to the named variable columns (and re-sort), so it
/// can be compared against an engine result that projects fewer variables.
fn project(bag: Vec<Vec<(String, String)>>, keep: &[&str]) -> Vec<Vec<(String, String)>> {
    let mut out: Vec<Vec<(String, String)>> = bag
        .into_iter()
        .map(|row| {
            let mut r: Vec<(String, String)> = row
                .into_iter()
                .filter(|(v, _)| keep.contains(&v.as_str()))
                .collect();
            r.sort();
            r
        })
        .collect();
    out.sort();
    out
}
