use super::*;

// Chain a -p-> b -p-> c -p-> d, plus a -q-> x.
fn g() -> Graph {
    Graph::load_str(
        "@prefix : <http://ex/> .\n:a :p :b . :b :p :c . :c :p :d . :a :q :x .\n",
        "turtle",
    )
    .unwrap()
}
fn n(sparql: &str) -> usize {
    crate::query(&g(), sparql).unwrap().len()
}
const PFX: &str = "PREFIX : <http://ex/> ";

#[test]
fn property_paths() {
    let n = |q: &str| n(&format!("{PFX}{q}"));
    // OneOrMore (transitive): all reachable pairs over the chain = C(4,2) = 6.
    assert_eq!(n("SELECT ?x ?y WHERE { ?x :p+ ?y }"), 6);
    assert_eq!(n("SELECT ?y WHERE { :a :p+ ?y }"), 3); // b,c,d
    // ZeroOrMore adds reflexive self-match: a,b,c,d.
    assert_eq!(n("SELECT ?y WHERE { :a :p* ?y }"), 4);
    // Sequence / Alternative / ZeroOrOne.
    assert_eq!(n("SELECT ?y WHERE { :a :p/:p ?y }"), 1); // c
    assert_eq!(n("SELECT ?y WHERE { :a :p|:q ?y }"), 2); // b, x
    assert_eq!(n("SELECT ?y WHERE { :a :p? ?y }"), 2); // a (zero), b (one)
    // Reverse (inverse), incl. inverse-transitive.
    assert_eq!(n("SELECT ?x WHERE { :d ^:p ?x }"), 1); // c
    assert_eq!(n("SELECT ?x WHERE { :d ^:p+ ?x }"), 3); // c,b,a
    // NegatedPropertySet: edges not via :p  ->  only a -q-> x.
    assert_eq!(n("SELECT ?x ?y WHERE { ?x !:p ?y }"), 1);
}

#[test]
fn named_graphs() {
    // One default-graph triple + two named graphs (g1 has 2, g2 has 1).
    let nq = "<http://ex/a> <http://ex/p> <http://ex/x> .\n\
                  <http://ex/b> <http://ex/p> <http://ex/y> <http://ex/g1> .\n\
                  <http://ex/c> <http://ex/p> <http://ex/z> <http://ex/g1> .\n\
                  <http://ex/d> <http://ex/p> <http://ex/w> <http://ex/g2> .\n";
    let g = Graph::load_dataset(nq, "nquads").unwrap();
    let n = |q: &str| crate::query(&g, q).unwrap().len();
    // The default graph holds ONLY default triples (named graphs are not folded in).
    assert_eq!(n("SELECT * WHERE { ?s ?p ?o }"), 1);
    assert_eq!(n("SELECT * WHERE { GRAPH <http://ex/g1> { ?s ?p ?o } }"), 2);
    assert_eq!(n("SELECT * WHERE { GRAPH <http://ex/g2> { ?s ?p ?o } }"), 1);
    // GRAPH ?g ranges over both named graphs (3 triples), binding ?g; an absent graph -> 0.
    assert_eq!(n("SELECT ?g ?s WHERE { GRAPH ?g { ?s ?p ?o } }"), 3);
    assert_eq!(n("SELECT * WHERE { GRAPH <http://ex/absent> { ?s ?p ?o } }"), 0);
    // Result ids are translated to the outer dict, so a join across GRAPH works.
    assert_eq!(n("SELECT ?o WHERE { GRAPH ?g { <http://ex/b> <http://ex/p> ?o } }"), 1);
}

/// (sq-zz8z, gh-51) The graph-IRI prefix range-scan index must return EXACTLY the
/// graphs a full scan + `STRSTARTS(STR(?g), prefix)` keeps. We assert this two ways: (1) the
/// indexed query result equals a hand-computed oracle over the known graph set, and (2) it
/// equals the core range-scan API — across the edge cases the bead calls out (empty prefix,
/// no-match, exact-match, prefix-is-a-substring of another graph's IRI, percent-encoding).
mod graph_prefix_index {
    use super::*;
    use std::collections::BTreeSet;

    // Graphs across three tenants + a couple of adversarial IRIs:
    //   tenantA/g/0, tenantA/g/1, tenantA/g/10   (note: "tenantA/g/1" is a PREFIX of ".../10")
    //   tenantB/g/0
    //   tenantAB/g/0                              ("tenantA" is a PREFIX of "tenantAB")
    //   has%20space/g/0                           (percent-encoding edge case)
    const ALL: [&str; 6] = [
        "http://ex/tenantA/g/0",
        "http://ex/tenantA/g/1",
        "http://ex/tenantA/g/10",
        "http://ex/tenantB/g/0",
        "http://ex/tenantAB/g/0",
        "http://ex/has%20space/g/0",
    ];

    fn ds() -> Graph {
        let mut nq = String::new();
        for (i, g) in ALL.iter().enumerate() {
            // each graph carries one :size triple so SUM(?size)/COUNT are non-trivial
            nq.push_str(&format!(
                "<http://ex/s{i}> <http://ex/size> \"{}\"^^<http://www.w3.org/2001/XMLSchema#integer> <{g}> .\n",
                (i + 1) * 10
            ));
        }
        Graph::load_dataset(&nq, "nquads").unwrap()
    }

    // The set of graph IRIs the indexed `GRAPH ?g … FILTER(STRSTARTS(STR(?g),prefix))` returns.
    fn indexed_graphs(g: &Graph, prefix: &str) -> BTreeSet<String> {
        let q = format!(
            "SELECT ?g WHERE {{ GRAPH ?g {{ ?s ?p ?o }} FILTER(STRSTARTS(STR(?g), \"{prefix}\")) }}"
        );
        let r = crate::query(g, &q).unwrap();
        let mut s = BTreeSet::new();
        for row in &r.rows {
            if let Some(Term::NamedNode(n)) = row.first().and_then(|c| c.as_ref()) {
                s.insert(n.as_str().to_string());
            }
        }
        s
    }

    // Oracle: STRSTARTS over the literal IRI set, computed in plain Rust.
    fn oracle(prefix: &str) -> BTreeSet<String> {
        ALL.iter().filter(|g| g.starts_with(prefix)).map(|s| s.to_string()).collect()
    }

    // Core API range scan, directly: the set the index yields for a prefix.
    fn core_index_graphs(g: &Graph, prefix: &str) -> BTreeSet<String> {
        let mut s = BTreeSet::new();
        g.for_named_graphs_with_prefix(prefix, |name, _| {
            if let Term::NamedNode(n) = name {
                s.insert(n.as_str().to_string());
            }
        });
        s
    }

    #[test]
    fn indexed_matches_oracle_across_edge_cases() {
        let g = ds();
        let prefixes = [
            "",                          // empty: matches every graph
            "http://ex/tenantA",         // matches tenantA/* AND tenantAB/* (prefix of prefix)
            "http://ex/tenantA/",        // matches only tenantA/* (the slash excludes tenantAB)
            "http://ex/tenantA/g/1",     // exact-prefix that is itself a substring of .../10
            "http://ex/tenantA/g/10",    // exact match of one graph IRI
            "http://ex/tenantB/",        // single tenant
            "http://ex/tenantC/",        // NO match
            "http://ex/has%20space/",    // percent-encoded IRI
            "zzz-nonexistent",           // no match, sorts after everything
            "http://ex/",                // common ancestor: all six
        ];
        for p in prefixes {
            let idx = indexed_graphs(&g, p);
            let orc = oracle(p);
            assert_eq!(idx, orc, "indexed result != oracle for prefix {p:?}");
            // The core range-scan API agrees with the oracle too (the index itself, not just
            // the engine wiring).
            assert_eq!(core_index_graphs(&g, p), orc, "core index != oracle for prefix {p:?}");
        }
    }

    #[test]
    fn aggregate_sum_count_is_prefix_scoped() {
        // The PSS `usage(prefix)` shape: SUM(?size) + COUNT(DISTINCT ?g), prefix-scoped.
        let g = ds();
        let usage = |prefix: &str| -> (i64, usize) {
            let q = format!(
                "SELECT (SUM(?size) AS ?bytes) (COUNT(DISTINCT ?g) AS ?n) WHERE {{ \
                     GRAPH ?g {{ ?s <http://ex/size> ?size }} FILTER(STRSTARTS(STR(?g), \"{prefix}\")) }}"
            );
            let r = crate::query(&g, &q).unwrap();
            let row = &r.rows[0];
            let bytes = match row[0].as_ref() {
                Some(Term::Literal(l)) => l.value().parse::<i64>().unwrap(),
                _ => 0,
            };
            let n = match row[1].as_ref() {
                Some(Term::Literal(l)) => l.value().parse::<usize>().unwrap(),
                _ => 0,
            };
            (bytes, n)
        };
        // tenantA/* = graphs at indices 0,1,2 with sizes 10,20,30 = 60, count 3.
        assert_eq!(usage("http://ex/tenantA/"), (60, 3));
        // tenantA (no slash) also catches tenantAB (index 4, size 50) -> 110, count 4.
        assert_eq!(usage("http://ex/tenantA"), (110, 4));
        // tenantB/* = index 3, size 40 -> (40, 1).
        assert_eq!(usage("http://ex/tenantB/"), (40, 1));
        // no match -> COUNT of empty group is 0.
        let r = crate::query(
            &g,
            "SELECT (COUNT(DISTINCT ?g) AS ?n) WHERE { GRAPH ?g { ?s ?p ?o } FILTER(STRSTARTS(STR(?g), \"http://ex/tenantC/\")) }",
        )
        .unwrap();
        let n = match r.rows[0][0].as_ref() {
            Some(Term::Literal(l)) => l.value().parse::<usize>().unwrap(),
            _ => 0,
        };
        assert_eq!(n, 0);
    }

    #[test]
    fn index_stays_correct_after_mutation_changes_graph_set() {
        // `update` returns a NEW graph (fresh prefix-index cache); the mutated set must scan
        // correctly even though the count changed twice. (Same-object cache coherence is
        // covered by the sparq-core test `graph_prefix_range_scan_and_cache_coherence`.)
        let g = ds();
        assert_eq!(indexed_graphs(&g, "http://ex/tenantA/").len(), 3);
        // Drop tenantA/g/10, add tenantA/g/2 -> still a tenantA/* set, but a different one.
        let g = crate::update::update(&g, "DROP GRAPH <http://ex/tenantA/g/10>").unwrap();
        let g = crate::update::update(
            &g,
            "INSERT DATA { GRAPH <http://ex/tenantA/g/2> { <http://ex/x> <http://ex/p> <http://ex/y> } }",
        )
        .unwrap();
        let got = indexed_graphs(&g, "http://ex/tenantA/");
        let want: BTreeSet<String> = ["http://ex/tenantA/g/0", "http://ex/tenantA/g/1", "http://ex/tenantA/g/2"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(got, want);
    }
}

/// sq-wij: GRAPH-scoped zero-length property paths (`*` / `?`) must
/// bind each node to ITSELF over the NAMED graph's node domain — and the domain
/// must be the named graph alone, never leaking nodes from the default graph or a
/// sibling named graph. A zero-length path inside `GRAPH <g> { … }` evaluates
/// against `g`'s self-contained sub-`Graph`, so `graph_nodes` already sees only
/// `g`; these tests lock that scoping in (and would catch a regression that read
/// the default/union node set instead).
#[test]
fn graph_scoped_zero_length_paths() {
    use std::collections::BTreeSet;
    // g1 nodes (as subject/object): a, b, c, d  (a-p->b, c-p->d).
    // g2 nodes:                      e, f        (e-p->f).
    // default-graph nodes:           m, n        (m-p->n)  — must NOT leak into a GRAPH.
    let nq = "<http://ex/m> <http://ex/p> <http://ex/n> .\n\
                  <http://ex/a> <http://ex/p> <http://ex/b> <http://ex/g1> .\n\
                  <http://ex/c> <http://ex/p> <http://ex/d> <http://ex/g1> .\n\
                  <http://ex/e> <http://ex/p> <http://ex/f> <http://ex/g2> .\n";
    let g = Graph::load_dataset(nq, "nquads").unwrap();

    // Collect the set of local-names a query binds to ?x (subject position),
    // asserting the path solution per row is reflexive (?x == ?y) when both are
    // selected. Returns the sorted set of subject local-names.
    let diag = |q: &str| -> BTreeSet<String> {
        let r = crate::query(&g, q).unwrap();
        let xi = r.vars.iter().position(|v| v.as_str() == "x").unwrap();
        let yi = r.vars.iter().position(|v| v.as_str() == "y");
        let mut out = BTreeSet::new();
        for row in &r.rows {
            let x = row[xi].as_ref().unwrap();
            if let Some(yi) = yi {
                // Zero-length diagonal: the two ends are the SAME term.
                assert_eq!(&row[yi], &Some(x.clone()), "expected reflexive bind, row {row:?}");
            }
            let s = match x {
                Term::NamedNode(n) => n.as_str().rsplit('/').next().unwrap().to_string(),
                other => panic!("unexpected term {other:?}"),
            };
            out.insert(s);
        }
        out
    };
    let set = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>();
    const P: &str = "PREFIX : <http://ex/> ";

    // --- `:p*` with a VARIABLE start: reflexive over the graph domain UNION the
    //     forward closure. Inside GRAPH <g1> the domain is exactly {a,b,c,d}; the
    //     default-graph nodes m,n and g2's e,f must NOT appear. -----------------
    // `?x :p* ?x` (same var) is purely the diagonal = the node domain of g1.
    assert_eq!(
        diag(&format!("{P}SELECT ?x ?y WHERE {{ GRAPH :g1 {{ ?x :p* ?y FILTER(?x = ?y) }} }}")),
        set(&["a", "b", "c", "d"]),
        "p* diagonal inside GRAPH <g1> must be g1's nodes only"
    );
    // g2 has a disjoint domain {e,f}: the wrong (union/default) scope would leak.
    assert_eq!(
        diag(&format!("{P}SELECT ?x ?y WHERE {{ GRAPH :g2 {{ ?x :p* ?y FILTER(?x = ?y) }} }}")),
        set(&["e", "f"]),
        "p* diagonal inside GRAPH <g2> must be g2's nodes only"
    );

    // --- `:p?` (ZeroOrOne) reflexive bindings inside GRAPH: a bound start yields
    //     its own self-pair plus the one-step neighbour, scoped to the graph. ---
    // `<a> :p? ?y` in g1 -> {a (zero), b (one)}.
    let p_opt_g1: BTreeSet<String> = {
        let r = crate::query(&g, &format!("{P}SELECT ?y WHERE {{ GRAPH :g1 {{ :a :p? ?y }} }}")).unwrap();
        r.rows.iter().map(|row| match row[0].as_ref().unwrap() {
            Term::NamedNode(n) => n.as_str().rsplit('/').next().unwrap().to_string(),
            other => panic!("{other:?}"),
        }).collect()
    };
    assert_eq!(p_opt_g1, set(&["a", "b"]), ":p? in GRAPH <g1> = self + one hop");
    // `?x :p? ?x` (same var) inside GRAPH <g1>: the diagonal = g1's node domain.
    assert_eq!(
        diag(&format!("{P}SELECT ?x ?y WHERE {{ GRAPH :g1 {{ ?x :p? ?y FILTER(?x = ?y) }} }}")),
        set(&["a", "b", "c", "d"]),
        ":p? diagonal inside GRAPH <g1> must be g1's nodes only"
    );

    // --- The default graph's own zero-length diagonal is {m,n} — confirming the
    //     GRAPH scopes above genuinely excluded these nodes. -------------------
    assert_eq!(
        diag(&format!("{P}SELECT ?x ?y WHERE {{ ?x :p* ?y FILTER(?x = ?y) }}")),
        set(&["m", "n"]),
        "default-graph p* diagonal must be the default graph's nodes only"
    );

    // --- `GRAPH ?g { ?x :p* ?y FILTER(?x=?y) }`: the diagonal per named graph,
    //     unioned, binds ?g to the SOURCE graph. The full set of subjects is the
    //     union of each named graph's domain (g1 ∪ g2), never the default graph. -
    let r = crate::query(
        &g,
        &format!("{P}SELECT ?g ?x ?y WHERE {{ GRAPH ?g {{ ?x :p* ?y FILTER(?x = ?y) }} }}"),
    )
    .unwrap();
    let (gi, xi, yi) = (
        r.vars.iter().position(|v| v.as_str() == "g").unwrap(),
        r.vars.iter().position(|v| v.as_str() == "x").unwrap(),
        r.vars.iter().position(|v| v.as_str() == "y").unwrap(),
    );
    let local = |t: &Term| match t {
        Term::NamedNode(n) => n.as_str().rsplit('/').next().unwrap().to_string(),
        other => panic!("{other:?}"),
    };
    let mut pairs: BTreeSet<(String, String)> = BTreeSet::new();
    for row in &r.rows {
        assert_eq!(&row[yi], &row[xi], "GRAPH ?g p* diagonal must be reflexive");
        pairs.insert((local(row[gi].as_ref().unwrap()), local(row[xi].as_ref().unwrap())));
    }
    let expected: BTreeSet<(String, String)> = [
        ("g1", "a"), ("g1", "b"), ("g1", "c"), ("g1", "d"), ("g2", "e"), ("g2", "f"),
    ]
    .iter()
    .map(|(a, b)| (a.to_string(), b.to_string()))
    .collect();
    assert_eq!(pairs, expected, "GRAPH ?g zero-length nodes must be scoped per named graph");
}

#[test]
fn rdf_star_concrete_triple_terms() {
    // RDF 1.2 triple terms load and CONCRETE `<< … >>` patterns match via the
    // STRUCTURAL dictionary encoding (component ids). Variable-inside patterns are
    // handled by BGP decomposition (sq-kbs / T6) — see the dedicated tests below.
    let g = Graph::load_str(
        "PREFIX : <http://ex/>\n<< :alice :age 30 >> :certainty 0.9 .\n<< :bob :age 25 >> :certainty 0.5 .",
        "turtle",
    )
    .unwrap();
    let n = |q: &str| crate::query(&g, q).unwrap().len();
    assert_eq!(n("PREFIX : <http://ex/> SELECT ?c WHERE { << :alice :age 30 >> :certainty ?c }"), 1);
    assert_eq!(n("PREFIX : <http://ex/> SELECT ?c WHERE { << :carol :age 99 >> :certainty ?c }"), 0);
}

#[test]
fn rdf_star_structural_roundtrip_output() {
    // Loading RDF 1.2 triple-term Turtle and selecting the triple term materialises a structural
    // `Term::Triple` (oxrdf formats it as `<<( … )>>`), NOT the old canonical-string
    // literal stopgap.
    let g = Graph::load_str("PREFIX : <http://ex/>\n<< :alice :age 30 >> :certainty 0.9 .", "turtle").unwrap();
    let r = crate::query(
        &g,
        "SELECT ?t WHERE { ?r <http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies> ?t }",
    )
    .unwrap();
    assert_eq!(r.rows.len(), 1);
    let expected = Term::Triple(Box::new(oxrdf::Triple::new(
        oxrdf::NamedNode::new_unchecked("http://ex/alice"),
        oxrdf::NamedNode::new_unchecked("http://ex/age"),
        Term::Literal(Literal::new_typed_literal("30", xsd::INTEGER)),
    )));
    assert_eq!(r.rows[0][0], Some(expected));

    // SPARQL 1.2 JSON results encoding: {"type":"triple","value":{subject/predicate/object}}.
    let json = crate::query_json(
        &g,
        "SELECT ?t WHERE { ?r <http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies> ?t }",
    )
    .unwrap();
    assert!(
        json.contains(
            "{\"type\":\"triple\",\"value\":{\"subject\":{\"type\":\"uri\",\"value\":\"http://ex/alice\"},\
                 \"predicate\":{\"type\":\"uri\",\"value\":\"http://ex/age\"},\"object\":{\"type\":\"literal\",\
                 \"value\":\"30\",\"datatype\":\"http://www.w3.org/2001/XMLSchema#integer\"}}}"
        ),
        "got: {json}"
    );
}

#[test]
fn rdf_star_nested_triple_terms() {
    // A triple term nests through the OBJECT position; it round-trips structurally.
    let g = Graph::load_str("PREFIX : <http://ex/>\n:x :p <<( :a :b <<( :c :d :e )>> )>> .", "turtle").unwrap();
    let r = crate::query(&g, "PREFIX : <http://ex/> SELECT ?o WHERE { :x :p ?o }").unwrap();
    assert_eq!(r.rows.len(), 1);
    let nn = oxrdf::NamedNode::new_unchecked;
    let inner = Term::Triple(Box::new(oxrdf::Triple::new(nn("http://ex/c"), nn("http://ex/d"), Term::NamedNode(nn("http://ex/e")))));
    let outer = Term::Triple(Box::new(oxrdf::Triple::new(nn("http://ex/a"), nn("http://ex/b"), inner)));
    assert_eq!(r.rows[0][0], Some(outer.clone()));
    // The concrete nested pattern matches (1) and a non-matching one misses (0).
    let n = |q: &str| crate::query(&g, q).unwrap().len();
    assert_eq!(n("PREFIX : <http://ex/> SELECT ?s WHERE { ?s :p <<( :a :b <<( :c :d :e )>> )>> }"), 1);
    assert_eq!(n("PREFIX : <http://ex/> SELECT ?s WHERE { ?s :p <<( :a :b <<( :c :d :x )>> )>> }"), 0);
}

#[test]
fn rdf_star_values_ground_triple_term() {
    // A ground triple term in VALUES binds and joins against stored triple terms.
    let g = Graph::load_str("PREFIX : <http://ex/>\n<< :alice :age 30 >> :certainty 0.9 .", "turtle").unwrap();
    let n = |q: &str| crate::query(&g, q).unwrap().len();
    assert_eq!(
        n("PREFIX : <http://ex/> SELECT ?c WHERE { VALUES ?t { <<( :alice :age 30 )>> } \
               ?r <http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies> ?t . ?r :certainty ?c }"),
        1
    );
    assert_eq!(
        n("PREFIX : <http://ex/> SELECT ?c WHERE { VALUES ?t { <<( :alice :age 31 )>> } \
               ?r <http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies> ?t . ?r :certainty ?c }"),
        0
    );
}

#[test]
fn rdf_star_variables_inside_quoted_patterns() {
    // F14: variables inside triple-term patterns MATCH structurally against the
    // stored triple terms, binding the inner variables.
    let g = Graph::load_str(
        "PREFIX : <http://ex/>\n<< :alice :age 30 >> :certainty 0.9 .\n<< :bob :age 25 >> :certainty 0.4 .\n:alice :name \"Alice\" .",
        "turtle",
    )
    .unwrap();
    let q = |s: &str| crate::query(&g, s).unwrap();
    // Var in the quoted subject slot.
    let r = q("PREFIX : <http://ex/> SELECT ?w ?c WHERE { << ?w :age 30 >> :certainty ?c }");
    assert_eq!(r.rows.len(), 1);
    assert!(r.rows[0][0].as_ref().unwrap().to_string().contains("alice"));
    // All slots variable: both reified statements match.
    assert_eq!(q("PREFIX : <http://ex/> SELECT * WHERE { << ?s ?p ?o >> :certainty ?c }").rows.len(), 2);
    // Inner variable JOINS with an outer pattern (alice has a name, bob does not).
    assert_eq!(
        q("PREFIX : <http://ex/> SELECT ?n WHERE { << ?w :age ?a >> :certainty ?c . ?w :name ?n }").rows.len(),
        1
    );
    // A ground component inside the quoted pattern constrains the match.
    assert_eq!(q("PREFIX : <http://ex/> SELECT ?c WHERE { << :bob :age ?a >> :certainty ?c }").rows.len(), 1);
    // No match: nothing reifies :alice :age 31.
    assert_eq!(q("PREFIX : <http://ex/> SELECT ?c WHERE { << :alice :age 31 >> :certainty ?c }").rows.len(), 0);
}

#[test]
fn rdf_star_quoted_pattern_var_positions_bind() {
    // sq-kbs (T6): a variable in EACH position of a triple-term pattern binds to
    // the matching component of the stored triple term, and the bound VALUES are
    // exactly the subject/predicate/object of the reified statement.
    let g = Graph::load_str(
        "PREFIX : <http://ex/>\n\
             << :alice :age 30 >> :certainty 0.9 .\n\
             << :bob :likes :tea >> :certainty 0.4 .",
        "turtle",
    )
    .unwrap();
    let sols = |s: &str| {
        let r = crate::query(&g, s).unwrap();
        let mut out: Vec<String> = r
            .rows
            .iter()
            .map(|row| row.iter().map(|c| c.as_ref().map(|t| t.to_string()).unwrap_or_default()).collect::<Vec<_>>().join("|"))
            .collect();
        out.sort();
        out
    };
    // Variable in SUBJECT slot — binds to the reified subject.
    assert_eq!(sols("PREFIX : <http://ex/> SELECT ?s WHERE { << ?s :age 30 >> :certainty ?c }"), vec!["<http://ex/alice>"]);
    // Variable in PREDICATE slot — binds to the reified predicate.
    assert_eq!(sols("PREFIX : <http://ex/> SELECT ?p WHERE { << :alice ?p 30 >> :certainty ?c }"), vec!["<http://ex/age>"]);
    // Variable in OBJECT slot — binds to the reified object (literal preserved).
    assert_eq!(
        sols("PREFIX : <http://ex/> SELECT ?o WHERE { << :alice :age ?o >> :certainty ?c }"),
        vec!["\"30\"^^<http://www.w3.org/2001/XMLSchema#integer>"]
    );
}

#[test]
fn rdf_star_quoted_pattern_ground_var_mix_filters() {
    // sq-kbs (T6): ground positions FILTER, variable positions BIND, within one
    // quoted pattern. Only the statement whose ground slots match contributes a row,
    // and the variable slot is bound to that statement's value.
    let g = Graph::load_str(
        "PREFIX : <http://ex/>\n\
             << :alice :age 30 >> :certainty 0.9 .\n\
             << :alice :age 40 >> :certainty 0.1 .\n\
             << :bob :age 30 >> :certainty 0.5 .",
        "turtle",
    )
    .unwrap();
    let q = |s: &str| crate::query(&g, s).unwrap();
    // Ground subject + ground predicate, variable object: two alice-age statements.
    let r = q("PREFIX : <http://ex/> SELECT ?o ?c WHERE { << :alice :age ?o >> :certainty ?c }");
    assert_eq!(r.rows.len(), 2);
    // Ground subject + ground object, variable predicate: exactly the :alice :age 30 row.
    let r = q("PREFIX : <http://ex/> SELECT ?c WHERE { << :alice ?p 30 >> :certainty ?c }");
    assert_eq!(r.rows.len(), 1);
    assert_eq!(r.rows[0][0].as_ref().unwrap().to_string(), "\"0.9\"^^<http://www.w3.org/2001/XMLSchema#decimal>");
    // No ground match: nothing reifies :carol :age 30.
    assert_eq!(q("PREFIX : <http://ex/> SELECT ?c WHERE { << :carol :age 30 >> :certainty ?c }").rows.len(), 0);
}

#[test]
fn rdf_star_quoted_pattern_all_vars_enumerate() {
    // sq-kbs (T6): `<< ?s ?p ?o >>` enumerates EVERY stored triple term, binding all
    // three slots, and the count equals the number of distinct reified statements.
    let g = Graph::load_str(
        "PREFIX : <http://ex/>\n\
             << :alice :age 30 >> :certainty 0.9 .\n\
             << :bob :likes :tea >> :certainty 0.4 .\n\
             << :carol :age 22 >> :certainty 0.7 .\n\
             :alice :name \"Alice\" .",
        "turtle",
    )
    .unwrap();
    let r = crate::query(&g, "PREFIX : <http://ex/> SELECT ?s ?p ?o ?c WHERE { << ?s ?p ?o >> :certainty ?c }").unwrap();
    assert_eq!(r.rows.len(), 3);
    // Every row binds all four variables (no NULLs), and the subjects cover all three.
    let mut subjects: Vec<String> = r.rows.iter().map(|row| row[0].as_ref().unwrap().to_string()).collect();
    subjects.sort();
    assert_eq!(subjects, vec!["<http://ex/alice>", "<http://ex/bob>", "<http://ex/carol>"]);
    assert!(r.rows.iter().all(|row| row.iter().take(4).all(|c| c.is_some())));
}

#[test]
fn rdf_star_quoted_pattern_nested_one_level() {
    // sq-kbs (T6): a triple term NESTED (one level) inside a triple-term pattern
    // unifies recursively — the inner variables bind to the inner statement's parts.
    let g = Graph::load_str(
        "PREFIX : <http://ex/>\n<< << :alice :age 30 >> :statedBy :bob >> :certainty 0.8 .",
        "turtle",
    )
    .unwrap();
    let r = crate::query(
        &g,
        "PREFIX : <http://ex/> SELECT ?who ?src WHERE { << << ?who :age 30 >> :statedBy ?src >> :certainty ?c }",
    )
    .unwrap();
    assert_eq!(r.rows.len(), 1);
    assert_eq!(r.rows[0][0].as_ref().unwrap().to_string(), "<http://ex/alice>");
    assert_eq!(r.rows[0][1].as_ref().unwrap().to_string(), "<http://ex/bob>");
}

#[test]
fn rdf_star_triple_builtins() {
    // F15: TRIPLE / isTRIPLE / SUBJECT / PREDICATE / OBJECT.
    let g = Graph::load_str("PREFIX : <http://ex/>\n<< :alice :age 30 >> :certainty 0.9 .", "turtle").unwrap();
    let one = |s: &str| {
        let r = crate::query(&g, s).unwrap();
        r.rows[0][0].as_ref().map(|t| t.to_string())
    };
    assert_eq!(
        one("PREFIX : <http://ex/> SELECT (TRIPLE(:a, :b, 1) AS ?t) {}").unwrap(),
        "<<( <http://ex/a> <http://ex/b> \"1\"^^<http://www.w3.org/2001/XMLSchema#integer> )>>"
    );
    assert_eq!(one("PREFIX : <http://ex/> SELECT (isTRIPLE(TRIPLE(:a, :b, :c)) AS ?x) {}").unwrap(), "\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>");
    assert_eq!(one("PREFIX : <http://ex/> SELECT (SUBJECT(TRIPLE(:a, :b, :c)) AS ?x) {}").unwrap(), "<http://ex/a>");
    assert_eq!(one("PREFIX : <http://ex/> SELECT (PREDICATE(TRIPLE(:a, :b, :c)) AS ?x) {}").unwrap(), "<http://ex/b>");
    assert_eq!(one("PREFIX : <http://ex/> SELECT (OBJECT(TRIPLE(:a, :b, :c)) AS ?x) {}").unwrap(), "<http://ex/c>");
    // A literal subject is a type error -> unbound.
    assert_eq!(one("PREFIX : <http://ex/> SELECT (TRIPLE(1, :b, :c) AS ?t) {}"), None);
}
