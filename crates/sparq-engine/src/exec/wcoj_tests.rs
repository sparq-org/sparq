use super::*;
use spargebra::SparqlParser;

/// Extracts the BGP triple patterns from a simple `SELECT * WHERE { ... }`.
fn bgp(sparql: &str) -> Vec<TriplePattern> {
    let q = SparqlParser::new().parse_query(sparql).unwrap();
    let spargebra::Query::Select { pattern, .. } = q else { panic!() };
    let mut patterns = Vec::new();
    let mut filters = Vec::new();
    // unwrap Project -> inner
    fn inner_of(p: &GraphPattern) -> &GraphPattern {
        match p {
            GraphPattern::Project { inner, .. }
            | GraphPattern::Distinct { inner }
            | GraphPattern::Slice { inner, .. } => inner_of(inner),
            other => other,
        }
    }
    flatten_conjunction(inner_of(&pattern), &mut patterns, &mut filters);
    patterns
}

/// Sorted set of result rows from a Bindings, for order-independent equality.
fn rowset(b: &Bindings) -> Vec<Vec<(String, Id)>> {
    let mut rows: Vec<Vec<(String, Id)>> = b
        .rows
        .iter()
        .map(|r| {
            let mut kv: Vec<(String, Id)> =
                b.vars.iter().zip(r).map(|(v, &id)| (v.as_str().to_string(), id)).collect();
            kv.sort();
            kv
        })
        .collect();
    rows.sort();
    rows
}

fn random_graph(seed0: u64, n_nodes: u32, n_edges: usize) -> Graph {
    let mut seed = seed0;
    let mut next = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) as u32
    };
    let mut ttl = String::from("@prefix ex: <http://ex/> .\n");
    for _ in 0..n_edges {
        let a = next() % n_nodes;
        let b = next() % n_nodes;
        ttl.push_str(&format!("ex:n{a} ex:e ex:n{b} .\n"));
    }
    Graph::load_str(&ttl, "turtle").unwrap()
}

/// The binary and WCOJ BGP plans must return identical result sets.
fn assert_plans_agree(sparql: &str, graph: &Graph) {
    let patterns = bgp(sparql);
    let binary = eval_bgp_binary(graph, &patterns, &[]).unwrap();
    let wcoj = eval_bgp_wcoj(graph, &patterns).unwrap();
    assert_eq!(
        rowset(&binary),
        rowset(&wcoj),
        "binary vs WCOJ disagree for `{sparql}` (binary {} rows, wcoj {} rows)",
        binary.rows.len(),
        wcoj.rows.len()
    );
}

#[test]
fn cyclicity_classification() {
    assert!(!bgp_is_cyclic(&bgp("PREFIX ex: <http://ex/> SELECT * WHERE { ?a ex:e ?b . ?b ex:e ?c }")));
    assert!(bgp_is_cyclic(&bgp(
        "PREFIX ex: <http://ex/> SELECT * WHERE { ?a ex:e ?b . ?b ex:e ?c . ?c ex:e ?a }"
    )));
    assert!(bgp_is_cyclic(&bgp(
        "PREFIX ex: <http://ex/> SELECT * WHERE { ?a ex:e ?b . ?b ex:e ?c . ?c ex:e ?d . ?d ex:e ?a }"
    )));
    // star is acyclic
    assert!(!bgp_is_cyclic(&bgp(
        "PREFIX ex: <http://ex/> SELECT * WHERE { ?a ex:e ?b . ?a ex:e ?c . ?a ex:e ?d }"
    )));
}

#[test]
fn wcoj_matches_binary_over_random_graphs() {
    for seed in [0x1111u64, 0xACE1, 0xDEADBEEF, 0x5EED] {
        let g = random_graph(seed, 25, 120);
        // chain (acyclic)
        assert_plans_agree("PREFIX ex: <http://ex/> SELECT * WHERE { ?a ex:e ?b . ?b ex:e ?c }", &g);
        // triangle (cyclic) — the canonical WCOJ win
        assert_plans_agree(
            "PREFIX ex: <http://ex/> SELECT * WHERE { ?a ex:e ?b . ?b ex:e ?c . ?c ex:e ?a }",
            &g,
        );
        // 4-cycle
        assert_plans_agree(
            "PREFIX ex: <http://ex/> SELECT * WHERE { ?a ex:e ?b . ?b ex:e ?c . ?c ex:e ?d . ?d ex:e ?a }",
            &g,
        );
        // square with a diagonal (denser cycle)
        assert_plans_agree(
            "PREFIX ex: <http://ex/> SELECT * WHERE { ?a ex:e ?b . ?b ex:e ?c . ?c ex:e ?a . ?a ex:e ?d . ?d ex:e ?c }",
            &g,
        );
    }
}

#[test]
fn wcoj_repeated_variable_pattern() {
    // self-loops: ?x ex:e ?x  — repeated-variable handling in build_trie.
    let g = Graph::load_str(
        "@prefix ex: <http://ex/> . ex:a ex:e ex:a . ex:a ex:e ex:b . ex:b ex:e ex:b .",
        "turtle",
    )
    .unwrap();
    let patterns = bgp("PREFIX ex: <http://ex/> SELECT * WHERE { ?x ex:e ?x }");
    let wcoj = eval_bgp_wcoj(&g, &patterns).unwrap();
    assert_eq!(wcoj.rows.len(), 2); // a and b
}

// ---- (sq-5zf8i / §A4) Yannakakis full-semijoin prepass ---------

/// A direct unit test for the ENGAGED prepass: a relation large enough to clear the
/// `YANNAKAKIS_MIN_REL` cost-gate, with most rows DANGLING (no join partner), so the
/// reduction actually fires. The prepass result must equal the binary plan's, row for
/// row — the load-bearing answer-equivalence invariant, tested against the in-crate
/// reference executor (not a brute force) so it covers the new public-on-this-feature
/// `eval_bgp_yannakakis` directly. Only compiles in the feature-on matrix leg.
#[cfg(feature = "yannakakis")]
#[test]
fn yannakakis_prepass_engages_and_matches_binary() {
    // 6000 subjects bound by ex:a (>4096 ⇒ above the gate); only the first 10 also have
    // an ex:b, so the join ?s ex:a ?x . ?s ex:b ?y returns 10 rows after dropping 5990
    // dangling ex:a tuples — exactly what the bottom-up semijoin removes.
    let mut ttl = String::from("@prefix ex: <http://ex/> .\n");
    for i in 0..6000u32 {
        ttl.push_str(&format!("ex:s{i} ex:a ex:x{i} .\n"));
        if i < 10 {
            ttl.push_str(&format!("ex:s{i} ex:b ex:y{i} .\n"));
        }
    }
    let g = Graph::load_str(&ttl, "turtle").unwrap();
    let patterns = bgp("PREFIX ex: <http://ex/> SELECT * WHERE { ?s ex:a ?x . ?s ex:b ?y }");

    // Sanity: the ex:a relation really is above the gate (so the prepass engages, not
    // the fallback) — otherwise this test would silently exercise the binary path.
    let prepared = prepare_bgp(&g, &patterns).unwrap();
    assert!(
        prepared.iter().map(|p| p.est).max().unwrap() >= YANNAKAKIS_MIN_REL,
        "test dataset must clear the cost-gate so the prepass engages"
    );

    let prepass = eval_bgp_yannakakis(&g, &patterns, &[]).unwrap();
    let binary = eval_bgp_binary(&g, &patterns, &[]).unwrap();
    assert_eq!(rowset(&prepass), rowset(&binary), "prepass must equal the binary plan");
    assert_eq!(prepass.rows.len(), 10, "exactly the ten joining subjects survive");
}

/// The cost-gate (pure-overhead guard): a tiny BGP below `YANNAKAKIS_MIN_REL` falls back
/// to the binary plan, and the answer is identical. Confirms the guard never changes
/// results. Only compiles in the feature-on matrix leg.
#[cfg(feature = "yannakakis")]
#[test]
fn yannakakis_cost_gate_falls_back_below_threshold() {
    let g = Graph::load_str(
        "@prefix ex: <http://ex/> . ex:s1 ex:a ex:x1 . ex:s1 ex:b ex:y1 . ex:s2 ex:a ex:x2 .",
        "turtle",
    )
    .unwrap();
    let patterns = bgp("PREFIX ex: <http://ex/> SELECT * WHERE { ?s ex:a ?x . ?s ex:b ?y }");
    let prepared = prepare_bgp(&g, &patterns).unwrap();
    assert!(prepared.iter().map(|p| p.est).max().unwrap() < YANNAKAKIS_MIN_REL);
    let prepass = eval_bgp_yannakakis(&g, &patterns, &[]).unwrap();
    let binary = eval_bgp_binary(&g, &patterns, &[]).unwrap();
    assert_eq!(rowset(&prepass), rowset(&binary), "fallback below the gate is result-identical");
    assert_eq!(prepass.rows.len(), 1); // only s1 has both a and b
}
