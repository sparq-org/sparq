use oxrdf::{BlankNode, NamedNode, NamedOrBlankNode, Term};
use sparq_core::Graph;
use sparq_engine::{
    apply_effects, parse_update_rec2013, update_graph_scope, update_in_place_algebra_capturing,
    update_in_place_atomic_with_budget, QueryBudget, UpdateEffect,
};
use std::collections::HashMap;

fn iri(name: &str) -> Term {
    NamedNode::new(format!("urn:{name}")).unwrap().into()
}

fn dump(graph: &Graph) -> Vec<(Option<Term>, Vec<[Term; 3]>)> {
    fn triples(graph: &Graph) -> Vec<[Term; 3]> {
        let mut rows: Vec<_> = graph
            .iter_ids()
            .map(|row| row.map(|id| graph.dict().term(id)))
            .collect();
        rows.sort_by_cached_key(|row| format!("{row:?}"));
        rows
    }
    let mut rows = vec![(None, triples(graph))];
    rows.extend(
        graph
            .named()
            .iter()
            .map(|(name, graph)| (Some(name.clone()), triples(graph))),
    );
    rows.sort_by_cached_key(|(name, _)| format!("{name:?}"));
    rows
}

fn fixture(extra: usize) -> Graph {
    let mut data = String::from(
        "<urn:s> <urn:p> <urn:default> .\n\
         <urn:s> <urn:p> <urn:o> <urn:a> .\n\
         _:seed <urn:p> <urn:o> <urn:a> .\n\
         <urn:s> <urn:p> <urn:b> <urn:b> .\n",
    );
    for i in 0..extra {
        data.push_str(&format!(
            "<urn:unrelated> <urn:p> \"{i}\" <urn:extra{i}> .\n"
        ));
    }
    let mut graph = Graph::load_dataset(&data, "nquads").unwrap();
    graph.ensure_named(&iri("empty")).unwrap();
    graph
}

fn assert_effects_equivalent(base: &Graph, text: &str) -> Vec<UpdateEffect> {
    let initial = dump(base);
    let update = parse_update_rec2013(text).unwrap();
    let names = update_graph_scope(&update).unwrap_or_else(|| panic!("unbounded: {text}"));
    let mut scoped = base.fork_scoped(&names);
    let mut full = base.fork();
    let budget = QueryBudget::unlimited();
    let scoped_effects =
        update_in_place_algebra_capturing(&mut scoped, &update, None, None, &budget).unwrap();
    let full_effects =
        update_in_place_algebra_capturing(&mut full, &update, None, None, &budget).unwrap();
    // Vec equality checks every effect, slot, triple and their order, without sorting the logs.
    assert_eq!(scoped_effects, full_effects, "{text}");
    let mut replay = base.fork();
    apply_effects(&mut replay, &scoped_effects).unwrap();
    assert_eq!(dump(&replay), dump(&full), "{text}");
    assert_eq!(dump(&replay.fork_scoped(&names)), dump(&scoped), "{text}");
    assert_eq!(dump(base), initial, "base changed: {text}");
    scoped_effects
}

fn assert_scope(text: &str, expected: &[&str]) {
    let update = parse_update_rec2013(text).unwrap();
    let mut names = update_graph_scope(&update).unwrap();
    names.sort_by_cached_key(Term::to_string);
    let mut expected: Vec<_> = expected.iter().map(|name| iri(name)).collect();
    expected.sort_by_cached_key(Term::to_string);
    assert_eq!(names, expected, "{text}");
}

#[test]
fn scoped_effects_and_replay_match_full_forks() {
    let updates = [
        "INSERT DATA { <urn:s> <urn:p> 1 . GRAPH <urn:a> { <urn:s> <urn:q> 2 } GRAPH <urn:new> { <urn:s> <urn:q> 3 } }",
        "DELETE DATA { GRAPH <urn:a> { <urn:s> <urn:p> <urn:o> } GRAPH <urn:missing> { <urn:s> <urn:p> <urn:o> } }",
        "DELETE WHERE { GRAPH <urn:a> { ?s ?p ?o } }",
        "DELETE { GRAPH <urn:a> { ?s <urn:p> ?o } } INSERT { GRAPH <urn:b> { ?s <urn:q> ?o } } WHERE { GRAPH <urn:a> { ?s <urn:p> ?o } }",
        "WITH <urn:a> DELETE { ?s <urn:p> ?o } INSERT { ?s <urn:q> ?o } WHERE { ?s <urn:p> ?o }",
        "WITH <urn:new> INSERT { ?s <urn:q> ?o } USING <urn:a> USING <urn:b> WHERE { ?s <urn:p> ?o }",
        "INSERT { GRAPH <urn:new> { ?s <urn:q> ?o } } USING NAMED <urn:a> USING NAMED <urn:absent> WHERE { GRAPH <urn:a> { ?s <urn:p> ?o } }",
        "INSERT { <urn:s> <urn:q> 1 } WHERE { FILTER EXISTS { GRAPH <urn:empty> {} } FILTER NOT EXISTS { GRAPH <urn:absent> {} } }",
        "INSERT { <urn:s> <urn:q> ?n } WHERE { { SELECT (COUNT(?s) AS ?n) WHERE { GRAPH <urn:a> { ?s ?p ?o } } } }",
        "INSERT { <urn:s> <urn:q> ?o } WHERE { GRAPH <urn:a> { ?s <urn:p>/<urn:p>? ?o } OPTIONAL { GRAPH <urn:b> { ?s ?p ?x } FILTER EXISTS { GRAPH <urn:empty> {} } } MINUS { GRAPH <urn:missing> { ?s ?p ?o } } }",
        "INSERT { <urn:s> <urn:q> ?o } WHERE { { GRAPH <urn:a> { <urn:s> <urn:p> ?o } } UNION { GRAPH <urn:b> { <urn:s> <urn:p> ?o } } }",
        "CREATE GRAPH <urn:new>; CREATE GRAPH <urn:a>; CREATE GRAPH <urn:empty>",
        "DROP GRAPH <urn:a>; DROP GRAPH <urn:missing>; CREATE GRAPH <urn:a>",
        "CLEAR GRAPH <urn:a>; CLEAR GRAPH <urn:missing>; CLEAR DEFAULT",
        "DROP DEFAULT; DROP GRAPH <urn:empty>; INSERT DATA { GRAPH <urn:empty> { <urn:s> <urn:p> 1 } }",
        "ADD GRAPH <urn:a> TO GRAPH <urn:new>",
        "MOVE GRAPH <urn:a> TO GRAPH <urn:b>",
        "COPY DEFAULT TO GRAPH <urn:new>",
        "COPY GRAPH <urn:a> TO DEFAULT",
        "COPY GRAPH <urn:missing> TO GRAPH <urn:b>",
        "LOAD SILENT <file:///nonexistent-scoped-fork-fixture> INTO GRAPH <urn:loaded>",
        "INSERT DATA { GRAPH <urn:new> { <urn:s> <urn:p> 1 } }; ADD GRAPH <urn:new> TO GRAPH <urn:a>; DROP GRAPH <urn:new>",
    ];
    for extra in [0, 3, 17] {
        let base = fixture(extra);
        for text in updates {
            assert_effects_equivalent(&base, text);
        }
    }
}

#[test]
fn scope_includes_concrete_reads_and_writes_and_deduplicates() {
    let update = parse_update_rec2013("WITH <urn:with> DELETE { ?s ?p ?o } INSERT { GRAPH <urn:insert> { ?s ?p ?o } } USING <urn:using> USING NAMED <urn:named> WHERE { GRAPH <urn:read> { ?s ?p ?o } FILTER EXISTS { GRAPH <urn:exists> {} } }; LOAD SILENT <file:///missing> INTO GRAPH <urn:load>; CREATE GRAPH <urn:create>; CLEAR GRAPH <urn:clear>; DROP GRAPH <urn:drop>; INSERT DATA { GRAPH <urn:read> { <urn:s> <urn:p> 1 } }").unwrap();
    let names = update_graph_scope(&update).unwrap();
    assert_eq!(
        names,
        ["with", "insert", "using", "named", "exists", "read", "load", "create", "clear", "drop"]
            .map(iri)
    );
    assert_eq!(
        update_graph_scope(&parse_update_rec2013("CLEAR DEFAULT").unwrap()),
        Some(vec![])
    );
}

#[test]
fn unbounded_constructs_require_full_forks_even_when_nested() {
    for text in [
        "INSERT { GRAPH <urn:x> { ?s ?p ?o } } WHERE { GRAPH ?g { ?s ?p ?o } }",
        "INSERT { GRAPH ?g { <urn:s> <urn:p> 1 } } WHERE { VALUES ?g { <urn:a> } }",
        "DELETE { GRAPH ?g { ?s ?p ?o } } WHERE { VALUES ?g { <urn:a> } GRAPH <urn:a> { ?s ?p ?o } }",
        "CLEAR NAMED", "CLEAR ALL", "DROP ALL", "DROP NAMED",
        "COPY DEFAULT TO <urn:x>; INSERT { <urn:s> <urn:p> 1 } WHERE { GRAPH ?g {} }",
        "INSERT { <urn:s> <urn:p> 1 } WHERE { FILTER NOT EXISTS { GRAPH ?g {} } }",
        "INSERT { <urn:s> <urn:p> ?x } WHERE { BIND(IF(EXISTS { GRAPH ?g {} }, 1, 0) AS ?x) }",
        "INSERT { <urn:s> <urn:p> ?x } WHERE { { SELECT (SUM(IF(EXISTS { GRAPH ?g {} }, 1, 0)) AS ?x) WHERE {} } }",
        "INSERT { <urn:s> <urn:p> 1 } WHERE { { SELECT ?s WHERE { GRAPH <urn:a> { ?s ?p ?o } } ORDER BY EXISTS { GRAPH ?g {} } LIMIT 1 } }",
        "INSERT { <urn:s> <urn:p> 1 } WHERE { OPTIONAL { GRAPH <urn:a> {} FILTER EXISTS { GRAPH ?g {} } } }",
        "INSERT { <urn:s> <urn:p> 1 } WHERE { SERVICE <urn:remote> { GRAPH <urn:a> {} } }",
        "WITH <urn:a> INSERT { <urn:s> <urn:p> 1 } WHERE { GRAPH ?g {} }",
        "INSERT { <urn:s> <urn:p> 1 } USING NAMED <urn:a> WHERE { GRAPH ?g {} }",
    ] {
        assert_eq!(update_graph_scope(&parse_update_rec2013(text).unwrap()), None, "{text}");
    }
}

// Independent executions mint different fresh blank nodes. Compare the complete
// ordered logs modulo fresh-node labels, retaining existing nodes and triple terms.
fn normalize(effects: &mut [UpdateEffect]) {
    fn term(value: &mut Term, labels: &mut HashMap<BlankNode, BlankNode>) {
        match value {
            Term::BlankNode(node) if node.as_str() != "seed" => {
                let next = BlankNode::new(format!("fresh{}", labels.len())).unwrap();
                *node = labels.entry(node.clone()).or_insert(next).clone();
            }
            Term::Triple(triple) => {
                let mut subject: Term = triple.subject.clone().into();
                term(&mut subject, labels);
                triple.subject = match subject {
                    Term::NamedNode(node) => NamedOrBlankNode::NamedNode(node),
                    Term::BlankNode(node) => NamedOrBlankNode::BlankNode(node),
                    _ => unreachable!(),
                };
                term(&mut triple.object, labels);
            }
            _ => (),
        }
    }
    let mut labels = HashMap::new();
    for effect in effects {
        if let UpdateEffect::Delta {
            inserts, deletes, ..
        } = effect
        {
            for triple in deletes.iter_mut().chain(inserts) {
                for value in triple {
                    term(value, &mut labels);
                }
            }
        }
    }
}

#[test]
fn fresh_blank_nodes_preserve_log_structure_and_replay() {
    let base = fixture(11);
    for text in [
        "INSERT DATA { GRAPH <urn:a> { _:b <urn:p> _:b . <urn:s> <urn:q> <<( _:b <urn:p> _:b )>> } }",
        "INSERT { GRAPH <urn:b> { _:b <urn:p> ?s . _:b <urn:q> _:b } } WHERE { GRAPH <urn:a> { ?s <urn:p> ?o } }",
    ] {
        let update = parse_update_rec2013(text).unwrap();
        let mut scoped = base.fork_scoped(&update_graph_scope(&update).unwrap());
        let mut full = base.fork();
        let budget = QueryBudget::unlimited();
        let mut a = update_in_place_algebra_capturing(&mut scoped, &update, None, None, &budget).unwrap();
        let mut b = update_in_place_algebra_capturing(&mut full, &update, None, None, &budget).unwrap();
        let mut replay_a = base.fork();
        let mut replay_b = base.fork();
        apply_effects(&mut replay_a, &a).unwrap();
        apply_effects(&mut replay_b, &b).unwrap();
        let names = update_graph_scope(&update).unwrap();
        assert_eq!(dump(&replay_a.fork_scoped(&names)), dump(&scoped));
        assert_eq!(dump(&replay_b), dump(&full));
        normalize(&mut a);
        normalize(&mut b);
        assert_eq!(a, b, "{text}");
    }
}

#[test]
fn atomic_swap_preserves_unmentioned_graphs() {
    let mut graph = fixture(7);
    let before = dump(&graph);
    update_in_place_atomic_with_budget(
        &mut graph,
        "INSERT DATA { <urn:new> <urn:p> 1 }",
        &QueryBudget::unlimited(),
    )
    .unwrap();
    assert_eq!(graph.named().len(), before.len() - 1);
    for (name, triples) in before.into_iter().filter(|(name, _)| name.is_some()) {
        let current = dump(graph.named_graph(&name.unwrap()).unwrap());
        assert_eq!(current, vec![(None, triples)]);
    }
}

#[test]
fn successful_load_effects_match_full_forks() {
    let dir = std::env::temp_dir().join(format!("scoped-load-{}", BlankNode::default().as_str()));
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("input.ttl"), "<urn:loaded> <urn:p> <urn:o> .").unwrap();
    sparq_engine::with_load_base(&dir, || {
        for destination in ["", " INTO GRAPH <urn:a>", " INTO GRAPH <urn:absent>"] {
            let text = format!("LOAD <file://input.ttl>{destination}");
            assert!(!assert_effects_equivalent(&fixture(5), &text).is_empty());
        }
        let text = "LOAD <file://input.ttl> INTO GRAPH <urn:new>; INSERT { GRAPH <urn:later> { ?s ?p ?o } } WHERE { GRAPH <urn:new> { ?s ?p ?o } }";
        assert_scope(text, &["new", "later"]);
        let effects = assert_effects_equivalent(&fixture(5), text);
        assert!(effects.iter().any(|effect| matches!(effect, UpdateEffect::Delta { slot: Some(name), inserts, .. } if name == &iri("later") && !inserts.is_empty())));
    });
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn dataset_clauses_and_nested_concrete_graphs_preserve_effects() {
    let cases: &[(&str, &[&str], bool)] = &[
        ("INSERT { GRAPH <urn:out> { ?s <urn:q> ?o } } USING NAMED <urn:a> WHERE { GRAPH <urn:a> { ?s <urn:p> ?o } }", &["out", "a"], true),
        // A concrete graph outside USING NAMED is absent from the active dataset,
        // even though that graph exists in the store and is included in the scope.
        ("INSERT { GRAPH <urn:out> { ?s <urn:q> ?o } } USING NAMED <urn:a> WHERE { GRAPH <urn:b> { ?s <urn:p> ?o } }", &["out", "a", "b"], false),
        ("WITH <urn:a> INSERT { GRAPH <urn:out> { ?s <urn:q> ?o } } WHERE { GRAPH <urn:b> { ?s <urn:p> ?o } }", &["out", "a", "b"], true),
        ("WITH <urn:a> DELETE { ?s <urn:p> ?o } INSERT { GRAPH <urn:b> { ?s <urn:q> ?o } } WHERE { GRAPH <urn:a> { ?s <urn:p> ?o } }", &["a", "b"], true),
        // USING overrides WITH for the WHERE; all templates here have explicit graph names,
        // so the parser eliminates the otherwise unused WITH name from its algebra.
        ("WITH <urn:a> INSERT { GRAPH <urn:out> { ?s <urn:q> ?o } } USING <urn:b> WHERE { ?s <urn:p> ?o }", &["out", "b"], true),
        ("WITH <urn:empty> INSERT { ?s <urn:q> ?o } USING NAMED <urn:a> WHERE { GRAPH <urn:a> { ?s <urn:p> ?o } }", &["empty", "a"], true),
        ("INSERT { GRAPH <urn:out> { ?s <urn:q> ?o } } WHERE { GRAPH <urn:a> { ?s <urn:p> ?o OPTIONAL { GRAPH <urn:b> { ?s <urn:p> ?other } } } }", &["out", "a", "b"], true),
        ("INSERT { GRAPH <urn:out> { ?s <urn:q> ?o } } WHERE { GRAPH <urn:a> { ?s <urn:p> ?o } MINUS { GRAPH <urn:b> { ?s <urn:p> ?o } } }", &["out", "a", "b"], true),
        ("INSERT { GRAPH <urn:out> { ?s <urn:q> ?o } } WHERE { { GRAPH <urn:a> { ?s <urn:p> ?o } } UNION { GRAPH <urn:b> { ?s <urn:p> ?o } } }", &["out", "a", "b"], true),
        ("INSERT { GRAPH <urn:out> { ?s <urn:q> ?o } } WHERE { { SELECT DISTINCT ?s ?o WHERE { GRAPH <urn:a> { ?s <urn:p> ?o } FILTER EXISTS { GRAPH <urn:empty> {} } FILTER NOT EXISTS { GRAPH <urn:absent> {} } } ORDER BY ?s LIMIT 2 } }", &["out", "a", "empty", "absent"], true),
        ("INSERT { GRAPH <urn:out> { ?s <urn:q> ?o } } WHERE { GRAPH <urn:a> { ?s <urn:p>+ ?o } }", &["out", "a"], true),
    ];
    for shift in [0, 1, 7] {
        let mut base = fixture(11);
        let mut members = base.take_named().unwrap();
        members.rotate_left(shift);
        if shift == 7 {
            members.reverse();
        }
        base.extend_named(members).unwrap();
        for (text, names, has_effects) in cases {
            assert_scope(text, names);
            let effects = assert_effects_equivalent(&base, text);
            assert_eq!(!effects.is_empty(), *has_effects, "{text}");
        }
    }
}

#[test]
fn transfer_matrix_and_later_reads_include_source_and_destination_slots() {
    let base = fixture(9);
    let endpoints = [
        "DEFAULT",
        "GRAPH <urn:a>",
        "GRAPH <urn:b>",
        "GRAPH <urn:empty>",
        "GRAPH <urn:absent>",
    ];
    for operation in ["ADD", "MOVE", "COPY"] {
        for from in endpoints {
            for to in endpoints {
                for silent in ["", "SILENT "] {
                    let text = format!("{operation} {silent}{from} TO {to}");
                    let expected: Vec<_> = if from == to {
                        vec![] // The parser lowers identity transfers to no operations.
                    } else {
                        let mut names: Vec<_> = [from, to]
                            .into_iter()
                            .filter_map(|endpoint| {
                                endpoint
                                    .strip_prefix("GRAPH <urn:")
                                    .and_then(|s| s.strip_suffix('>'))
                            })
                            .collect();
                        names.sort_unstable();
                        names.dedup();
                        names
                    };
                    assert_scope(&text, &expected);
                    assert_effects_equivalent(&base, &text);
                }
            }
        }
    }
    for text in [
        "CREATE SILENT GRAPH <urn:new>; INSERT DATA { GRAPH <urn:new> { <urn:s> <urn:p> <urn:o> } }; INSERT { GRAPH <urn:later> { ?s ?p ?o } } WHERE { GRAPH <urn:new> { ?s ?p ?o } }",
        "MOVE GRAPH <urn:a> TO GRAPH <urn:new>; INSERT { GRAPH <urn:later> { ?s ?p ?o } } WHERE { GRAPH <urn:new> { ?s ?p ?o } }",
        "MOVE DEFAULT TO GRAPH <urn:new>; ADD GRAPH <urn:new> TO DEFAULT; INSERT { GRAPH <urn:later> { ?s ?p ?o } } WHERE { ?s ?p ?o }",
        "COPY GRAPH <urn:a> TO GRAPH <urn:new>; DROP SILENT GRAPH <urn:a>; WITH <urn:new> INSERT { GRAPH <urn:later> { ?s ?p ?o } } WHERE { ?s ?p ?o }",
        "DROP SILENT GRAPH <urn:absent>; CREATE SILENT GRAPH <urn:absent>; INSERT { GRAPH <urn:later> { <urn:s> <urn:p> <urn:exists> } } WHERE { GRAPH <urn:absent> {} }",
    ] {
        let effects = assert_effects_equivalent(&base, text);
        assert!(effects.iter().any(|effect| matches!(effect, UpdateEffect::Delta { slot: Some(name), inserts, .. } if name == &iri("later") && !inserts.is_empty())), "later read was vacuous: {text}");
    }
}

#[test]
fn effect_order_and_absent_slot_operations_match_exactly() {
    use spargebra::algebra::GraphTarget;
    let text = "CREATE SILENT GRAPH <urn:new>; INSERT DATA { GRAPH <urn:b> { <urn:ordered> <urn:p> <urn:one> } <urn:ordered> <urn:p> <urn:default-new> . GRAPH <urn:a> { <urn:ordered> <urn:p> <urn:one> } GRAPH <urn:b> { <urn:ordered> <urn:p> <urn:two> } }; DELETE DATA { GRAPH <urn:absent> { <urn:ordered> <urn:p> <urn:absent-object> } }; CLEAR SILENT GRAPH <urn:absent>; DROP SILENT GRAPH <urn:new>; DROP SILENT GRAPH <urn:absent>; CREATE SILENT GRAPH <urn:empty>; CREATE SILENT GRAPH <urn:new>";
    let triple = |object| [iri("ordered"), iri("p"), iri(object)];
    let target = |name| GraphTarget::NamedNode(NamedNode::new(format!("urn:{name}")).unwrap());
    assert_eq!(
        assert_effects_equivalent(&fixture(5), text),
        vec![
            UpdateEffect::Create(iri("new")),
            UpdateEffect::Delta {
                slot: Some(iri("b")),
                inserts: vec![triple("one"), triple("two")],
                deletes: vec![]
            },
            UpdateEffect::Delta {
                slot: None,
                inserts: vec![triple("default-new")],
                deletes: vec![]
            },
            UpdateEffect::Delta {
                slot: Some(iri("a")),
                inserts: vec![triple("one")],
                deletes: vec![]
            },
            UpdateEffect::Delta {
                slot: Some(iri("absent")),
                inserts: vec![],
                deletes: vec![triple("absent-object")]
            },
            UpdateEffect::Clear(target("absent")),
            UpdateEffect::Drop(target("new")),
            UpdateEffect::Drop(target("absent")),
            UpdateEffect::Create(iri("empty")),
            UpdateEffect::Create(iri("new")),
        ]
    );
}

#[test]
fn nested_unbounded_graphs_and_variable_templates_always_fall_back() {
    for pattern in [
        "OPTIONAL { GRAPH ?g { ?s ?p ?o } }",
        "GRAPH <urn:a> {} MINUS { GRAPH ?g { ?s ?p ?o } }",
        "{ GRAPH <urn:a> {} } UNION { GRAPH ?g { ?s ?p ?o } }",
        "GRAPH <urn:a> { GRAPH ?g { ?s ?p ?o } }",
        "{ SELECT ?s WHERE { GRAPH ?g { ?s ?p ?o } } }",
        "FILTER EXISTS { GRAPH ?g {} }",
        "FILTER NOT EXISTS { GRAPH ?g {} }",
        "GRAPH <urn:a> {} LATERAL { GRAPH ?g { ?s ?p ?o } }",
        "SERVICE SILENT <urn:remote> { GRAPH <urn:a> {} }",
        "SERVICE ?endpoint { GRAPH <urn:a> {} }",
    ] {
        let text = format!("INSERT {{ <urn:log> <urn:p> <urn:o> }} WHERE {{ {pattern} }}");
        assert_eq!(
            update_graph_scope(&parse_update_rec2013(&text).unwrap()),
            None,
            "{text}"
        );
    }
    for text in [
        "INSERT { GRAPH <urn:out> { ?s ?p ?o } } USING NAMED <urn:a> USING NAMED <urn:b> WHERE { GRAPH ?g { ?s ?p ?o } }",
        "WITH <urn:a> INSERT { GRAPH <urn:out> { ?s ?p ?o } } WHERE { GRAPH ?g { ?s ?p ?o } }",
        "WITH <urn:a> INSERT { GRAPH ?g { ?s ?p ?o } } USING NAMED <urn:b> WHERE { VALUES ?g { <urn:out> } GRAPH <urn:b> { ?s ?p ?o } }",
        "WITH <urn:a> DELETE { GRAPH ?g { ?s ?p ?o } } WHERE { VALUES ?g { <urn:b> } GRAPH <urn:b> { ?s ?p ?o } }",
    ] {
        assert_eq!(update_graph_scope(&parse_update_rec2013(text).unwrap()), None, "{text}");
    }
}

#[test]
fn blank_node_graph_names_are_rejected_or_observed_only_through_full_fallback() {
    for text in [
        "INSERT DATA { GRAPH _:g { <urn:s> <urn:p> <urn:o> } }",
        "DELETE DATA { GRAPH _:g { <urn:s> <urn:p> <urn:o> } }",
        "INSERT { GRAPH _:g { <urn:s> <urn:p> <urn:o> } } WHERE {}",
        "INSERT { <urn:s> <urn:p> <urn:o> } WHERE { GRAPH _:g {} }",
        "WITH _:g INSERT { <urn:s> <urn:p> <urn:o> } WHERE {}",
        "INSERT { <urn:s> <urn:p> <urn:o> } USING NAMED _:g WHERE {}",
        "CREATE SILENT GRAPH _:g",
        "DROP SILENT GRAPH _:g",
        "LOAD SILENT <file:///missing> INTO GRAPH _:g",
        "ADD DEFAULT TO GRAPH _:g",
        "MOVE GRAPH _:g TO DEFAULT",
        "COPY GRAPH _:g TO DEFAULT",
    ] {
        assert!(
            parse_update_rec2013(text).is_err(),
            "parser accepted a blank node graph name: {text}"
        );
    }
    let mut base = fixture(3);
    let name: Term = BlankNode::new("graph-slot").unwrap().into();
    base.add_named(
        name.clone(),
        Graph::load_str("<urn:s> <urn:p> <urn:blank-graph-object> .", "ntriples").unwrap(),
    )
    .unwrap();
    // A bounded update must leave an unrelated blank-node-named member intact.
    assert_effects_equivalent(
        &base,
        "INSERT DATA { GRAPH <urn:a> { <urn:s> <urn:q> <urn:o> } }",
    );
    let text = "DELETE WHERE { GRAPH ?g { ?s ?p ?o } }";
    let update = parse_update_rec2013(text).unwrap();
    assert_eq!(update_graph_scope(&update), None);
    let mut full = base.fork();
    let effects = update_in_place_algebra_capturing(
        &mut full,
        &update,
        None,
        None,
        &QueryBudget::unlimited(),
    )
    .unwrap();
    assert!(effects.iter().any(|effect| matches!(effect, UpdateEffect::Delta { slot: Some(slot), deletes, .. } if slot == &name && !deletes.is_empty())));
    let mut replay = base.fork();
    apply_effects(&mut replay, &effects).unwrap();
    assert_eq!(dump(&replay), dump(&full));
}
