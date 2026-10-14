//! (sq-iywur) Result-equivalence for the opt-in `dp-planner` DPccp
//! join-order enumerator.
//!
//! The DP planner (`exec::eval_bgp_dp`, enumeration in `dp::plan`) picks a
//! `Cout`-optimal BUSHY join tree for a BGP instead of the greedy GOO left-deep order.
//! Its load-bearing CORRECTNESS CONTRACT is ORDER-ONLY: a BGP is a commutative and
//! associative natural join, so EVERY join tree over the same patterns yields the SAME
//! bindings — the planner changes join order, never the answer.
//!
//! This file is DELIBERATELY NOT feature-gated, so it runs in BOTH states:
//!  * default `cargo test` (feature OFF) validates every query's answer against an
//!    INDEPENDENT brute-force reference (a hand nested-loop join sharing no code with the
//!    engine) — and proves the test compiles clean without the feature; and
//!  * the `opt-in sparq-engine (dp-planner)` matrix leg (feature ON) additionally runs
//!    the SAME query under `with_dp_planner` at several budgets and asserts it is
//!    byte-for-byte equal to the greedy default (and hence to the brute-force reference).
//!
//! That cross-build identity (DP == greedy == brute-force) IS the on-vs-off equivalence
//! proof. Shapes cover: chain, star, snowflake, a disconnected cross-product BGP (the DP
//! declines and greedy handles it), a CYCLIC triangle (routed to LFTJ, never the DP), a
//! pushed-down numeric FILTER (exercises the DP leaf-scan filter), a repeated-variable
//! pattern, an OPTIONAL wrapping a BGP, and an empty result.

use sparq_core::Graph;
use sparq_engine::query;

const PFX: &str = "PREFIX ex: <http://ex/> PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";

type T = (String, String, String);

/// Run `q` and return its solutions as a SORTED bag of `(var, lexical)` rows —
/// order-independent (join order is never observable) but exact on bound terms.
fn result_bag(graph: &Graph, q: &str) -> Vec<Vec<(String, String)>> {
    // Unbound cells are recorded with an explicit NUL-delimited sentinel, NOT the empty
    // string: no `Term::to_string()` can ever produce this marker (every bound RDF term
    // serializes as `<iri>`, `"lex"…`, or `_:b` — even the empty literal renders WITH its
    // surrounding quotes), so an unbound slot can never collide with a legitimately bound
    // value. sq-iywur (addresses Copilot review re: empty-string ambiguity).
    const UNBOUND: &str = "\0\u{1}unbound\u{1}\0";
    let r = query(graph, &format!("{PFX}{q}")).unwrap();
    let vars: Vec<String> = r.vars.iter().map(|v| v.as_str().to_string()).collect();
    let mut bag: Vec<Vec<(String, String)>> = r
        .rows
        .iter()
        .map(|row| {
            let mut cells: Vec<(String, String)> = vars
                .iter()
                .zip(row.iter())
                .map(|(v, cell)| (v.clone(), cell.as_ref().map(|t| t.to_string()).unwrap_or_else(|| UNBOUND.to_string())))
                .collect();
            cells.sort();
            cells
        })
        .collect();
    bag.sort();
    bag
}

/// Runs `q` under the greedy default and — when the `dp-planner` feature is on — under the
/// DP planner at several budgets (0 = always greedy, 1 = tiny so multi-pattern BGPs fall
/// back, and a generous budget where the DP genuinely engages), asserting every run agrees.
/// Returns the (greedy) bag for comparison against the brute-force reference.
fn run_all(g: &Graph, q: &str) -> Vec<Vec<(String, String)>> {
    let greedy = result_bag(g, q);
    #[cfg(feature = "dp-planner")]
    {
        for &budget in &[0usize, 1, 4096, 1 << 16] {
            let dp = sparq_engine::with_dp_planner_budget(budget, || result_bag(g, q));
            assert_eq!(greedy, dp, "DP (budget={}) must match greedy for: {}", budget, q);
        }
        let dp = sparq_engine::with_dp_planner(|| result_bag(g, q));
        assert_eq!(greedy, dp, "DP (default budget) must match greedy for: {}", q);
    }
    greedy
}

/// Brute-force reference conjunctive-BGP evaluator: a dead-simple nested-loop join sharing
/// NO code with the engine. Each pattern slot is a constant (exact lexical) or a `?var`.
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

/// Re-project a brute-force bag down to named variable columns (and re-sort).
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

/// A social dataset: `n` persons each with name/age/city, each city with a country.
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

#[test]
fn chain_join_matches_brute_force() {
    // A 4-pattern chain ?a->?b->?c->?d with dead-end tuples that join nowhere — the DP is
    // free to reorder; every order (and greedy) must yield the same bag.
    let len = 300usize;
    let mut triples: Vec<T> = vec![];
    for i in 0..len {
        triples.push((format!("<http://ex/e/{i}>"), "<http://ex/next>".into(), format!("<http://ex/e/{}>", i + 1)));
        triples.push((format!("<http://ex/e/{i}>"), "<http://ex/next>".into(), format!("<http://ex/dead/{i}>")));
    }
    let g = load(&triples);
    let engine = run_all(&g, "SELECT ?a ?b ?c ?d WHERE { ?a ex:next ?b . ?b ex:next ?c . ?c ex:next ?d }");
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

#[test]
fn star_join_matches_brute_force() {
    let triples = social_dataset(120);
    let g = load(&triples);
    let engine = run_all(&g, "SELECT ?p ?name ?age WHERE { ?p ex:name ?name . ?p ex:age ?age . ?p ex:city ?c }");
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
    assert_eq!(engine, reference);
    assert!(!engine.is_empty());
}

#[test]
fn snowflake_matches_brute_force() {
    // Star on ?p (name/age/city) + chain ?p -> ?city -> ?country: a 4-pattern connected
    // BGP mixing a star and a chain, the shape where a bushy DP tree differs most from
    // greedy left-deep.
    let triples = social_dataset(90);
    let g = load(&triples);
    let engine = run_all(
        &g,
        "SELECT ?p ?age ?country WHERE { ?p ex:name ?name . ?p ex:age ?age . ?p ex:city ?city . ?city ex:country ?country }",
    );
    let reference = project(
        brute(
            &triples,
            &[
                ["?p", "<http://ex/name>", "?name"],
                ["?p", "<http://ex/age>", "?age"],
                ["?p", "<http://ex/city>", "?city"],
                ["?city", "<http://ex/country>", "?country"],
            ],
        ),
        &["p", "age", "country"],
    );
    assert_eq!(engine, reference);
    assert!(!engine.is_empty());
}

#[test]
fn disconnected_cross_product_matches_brute_force() {
    // Two patterns sharing NO variable ⇒ a disconnected BGP (cartesian product). The DP
    // declines (returns None) and greedy performs the cross product; result must match.
    let triples: Vec<T> = vec![
        ("<http://ex/a1>".into(), "<http://ex/p>".into(), "\"x\"".into()),
        ("<http://ex/a2>".into(), "<http://ex/p>".into(), "\"y\"".into()),
        ("<http://ex/b1>".into(), "<http://ex/q>".into(), "\"m\"".into()),
        ("<http://ex/b2>".into(), "<http://ex/q>".into(), "\"n\"".into()),
    ];
    let g = load(&triples);
    let engine = run_all(&g, "SELECT ?s ?o ?t ?u WHERE { ?s ex:p ?o . ?t ex:q ?u }");
    let reference = brute(&triples, &[["?s", "<http://ex/p>", "?o"], ["?t", "<http://ex/q>", "?u"]]);
    assert_eq!(engine, reference);
    assert_eq!(engine.len(), 4, "2x2 cross product");
}

#[test]
fn cyclic_triangle_matches_brute_force() {
    // A cyclic BGP (?a->?b->?c->?a) routes to LFTJ (never the binary DP path), but with the
    // planner installed the answer must still be correct.
    let triples: Vec<T> = vec![
        ("<http://ex/a>".into(), "<http://ex/e>".into(), "<http://ex/b>".into()),
        ("<http://ex/b>".into(), "<http://ex/e>".into(), "<http://ex/c>".into()),
        ("<http://ex/c>".into(), "<http://ex/e>".into(), "<http://ex/a>".into()),
        ("<http://ex/a>".into(), "<http://ex/e>".into(), "<http://ex/x>".into()),
    ];
    let g = load(&triples);
    let engine = run_all(&g, "SELECT ?a ?b ?c WHERE { ?a ex:e ?b . ?b ex:e ?c . ?c ex:e ?a }");
    let reference = brute(
        &triples,
        &[["?a", "<http://ex/e>", "?b"], ["?b", "<http://ex/e>", "?c"], ["?c", "<http://ex/e>", "?a"]],
    );
    assert_eq!(engine, reference);
    assert_eq!(engine.len(), 3, "the single triangle, once per rotation");
}

#[test]
fn pushed_filter_in_chain_matches_brute_force() {
    // A pushed-down numeric FILTER on a chain: exercises the DP leaf-scan filter (a Leaf must
    // apply its `pat_filters` entry exactly like the greedy seed scan).
    let mut triples: Vec<T> = vec![];
    for i in 0..40u32 {
        triples.push((
            format!("<http://ex/p{i}>"),
            "<http://ex/age>".into(),
            format!("\"{}\"^^<http://www.w3.org/2001/XMLSchema#integer>", i),
        ));
        triples.push((format!("<http://ex/p{i}>"), "<http://ex/name>".into(), format!("\"n{i}\"")));
    }
    let g = load(&triples);
    let engine = run_all(
        &g,
        "SELECT ?p ?age ?name WHERE { ?p ex:age ?age . ?p ex:name ?name . FILTER(?age > 30) }",
    );
    // Brute-force the two-pattern join, then filter numerically in the reference.
    let joined = brute(&triples, &[["?p", "<http://ex/age>", "?age"], ["?p", "<http://ex/name>", "?name"]]);
    let mut reference: Vec<Vec<(String, String)>> = joined
        .into_iter()
        .filter(|row| {
            row.iter()
                .find(|(v, _)| v == "age")
                .and_then(|(_, lex)| lex.split('"').nth(1))
                .and_then(|n| n.parse::<i64>().ok())
                .is_some_and(|n| n > 30)
        })
        .collect();
    reference.sort();
    assert_eq!(engine, reference);
    assert!(!engine.is_empty());
}

#[test]
fn repeated_variable_pattern_matches_brute_force() {
    // A pattern with a repeated variable (?x ex:knows ?x — self-edges) plus a join: the DP
    // leaf scan must honour the same repeated-variable consistency check as greedy.
    let triples: Vec<T> = vec![
        ("<http://ex/a>".into(), "<http://ex/knows>".into(), "<http://ex/a>".into()),
        ("<http://ex/b>".into(), "<http://ex/knows>".into(), "<http://ex/c>".into()),
        ("<http://ex/a>".into(), "<http://ex/name>".into(), "\"A\"".into()),
        ("<http://ex/b>".into(), "<http://ex/name>".into(), "\"B\"".into()),
    ];
    let g = load(&triples);
    let engine = run_all(&g, "SELECT ?x ?n WHERE { ?x ex:knows ?x . ?x ex:name ?n }");
    let reference = brute(&triples, &[["?x", "<http://ex/knows>", "?x"], ["?x", "<http://ex/name>", "?n"]]);
    assert_eq!(engine, reference);
    assert_eq!(engine.len(), 1, "only the self-loop node a joins");
}

#[test]
fn optional_over_bgp_matches_brute_force() {
    // OPTIONAL wrapping a BGP: the inner required + optional BGPs each flow through the
    // binary executor, so the planner install must not perturb OPTIONAL semantics.
    let triples = social_dataset(30);
    let g = load(&triples);
    let engine = run_all(
        &g,
        "SELECT ?p ?name ?country WHERE { ?p ex:name ?name . ?p ex:city ?city . OPTIONAL { ?city ex:country ?country } }",
    );
    // Every person has a city, every city a country in this dataset, so OPTIONAL always binds.
    let reference = project(
        brute(
            &triples,
            &[
                ["?p", "<http://ex/name>", "?name"],
                ["?p", "<http://ex/city>", "?city"],
                ["?city", "<http://ex/country>", "?country"],
            ],
        ),
        &["p", "name", "country"],
    );
    assert_eq!(engine, reference);
    assert!(!engine.is_empty());
}

#[test]
fn empty_result_matches_brute_force() {
    // A connected BGP whose join is empty (no person has the sought predicate value).
    let triples = social_dataset(20);
    let g = load(&triples);
    let engine = run_all(
        &g,
        "SELECT ?p WHERE { ?p ex:name ?name . ?p ex:age ?age . ?p ex:nonexistent ?z }",
    );
    assert!(engine.is_empty(), "no such predicate ⇒ empty under every plan");
}
