use super::*;
use proptest::prelude::*;
use proptest::test_runner::RngSeed;

// An ordered, independent model of the original Vec operations, including duplicate names.
type Model = Vec<(Term, Vec<[Term; 3]>)>;

fn name(i: u8) -> Term {
    match i % 3 {
        0 => NamedNode::new(format!("urn:nidx:{}", i / 3))
            .unwrap()
            .into(),
        1 => oxrdf::BlankNode::new(format!("nidx{}", i / 3))
            .unwrap()
            .into(),
        _ => Literal::new_simple_literal(format!("nidx{}", i / 3)).into(),
    }
}

fn payload(i: u8) -> Vec<[Term; 3]> {
    vec![[
        name(0),
        name(3),
        Literal::new_simple_literal(i.to_string()).into(),
    ]]
}

fn subgraph(i: u8) -> Graph {
    let mut graph = Graph::new();
    graph.apply_delta(&payload(i), &[]).unwrap();
    graph
}

fn assert_state(graph: &Graph, expected: &Model) {
    let actual: Model = graph
        .named()
        .iter()
        .map(|(n, sub)| {
            (
                n.clone(),
                sub.iter_ids()
                    .map(|ids| ids.map(|id| sub.dict().term(id)))
                    .collect(),
            )
        })
        .collect();
    assert_eq!(
        &actual, expected,
        "membership, payload and Vec iteration order"
    );
    for n in (0..16)
        .map(name)
        .chain(expected.iter().map(|(n, _)| n.clone()))
    {
        let oracle = graph
            .named()
            .iter()
            .position(|(candidate, _)| *candidate == n);
        assert_eq!(
            graph.named_graph_index(&n),
            oracle,
            "indexed position for {n}"
        );
        assert_eq!(
            graph.named_graph(&n).map(|sub| sub as *const Graph),
            oracle.map(|i| &graph.named()[i].1 as *const Graph),
            "first exact graph for {n}"
        );
    }
    for prefix in ["urn:", "nidx", "absent"] {
        let mut actual = Vec::new();
        graph.for_named_graphs_with_prefix(prefix, |n, _| actual.push(n.clone()));
        let mut oracle: Vec<_> = expected
            .iter()
            .filter(|(n, _)| Graph::graph_name_str(n).starts_with(prefix))
            .map(|(n, _)| n.clone())
            .collect();
        actual.sort_by_key(ToString::to_string);
        oracle.sort_by_key(ToString::to_string);
        assert_eq!(
            actual, oracle,
            "prefix index remains independent and current"
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 24, rng_seed: RngSeed::Fixed(0x71887370), .. ProptestConfig::default()
    })]
    #[test]
    fn named_index_matches_vec_operations(actions in prop::collection::vec((0u8..16, 0u8..12, any::<u8>()), 80..120)) {
        let mut graph = Graph::new();
        let mut model = Model::new();
        for (kind, i, value) in actions {
            let n = name(i);
            match kind {
                0 => { graph.add_named(n.clone(), subgraph(value)).unwrap(); model.push((n, payload(value))); }
                1 => {
                    let position = model.iter().position(|(m, _)| *m == n).unwrap_or_else(|| {
                        model.push((n.clone(), vec![])); model.len() - 1
                    });
                    assert_eq!(graph.ensure_named(&n).unwrap(), position);
                }
                2 => {
                    graph.set_named(n.clone(), subgraph(value)).unwrap();
                    match model.iter_mut().find(|(m, _)| *m == n) {
                        Some((_, triples)) => *triples = payload(value),
                        None => model.push((n, payload(value))),
                    }
                }
                3 | 4 => {
                    if kind == 3 { graph.remove_named(&n).unwrap(); }
                    else { graph.drop_named_durable(&n).unwrap(); }
                    if let Some(i) = model.iter().position(|(m, _)| *m == n) { model.remove(i); }
                }
                5 => {
                    graph.retain_named(|m, _| *m != n).unwrap();
                    model.retain(|(m, _)| *m != n);
                }
                6 => {
                    let members = graph.take_named().unwrap();
                    assert_state(&graph, &vec![]);
                    graph.extend_named(members).unwrap();
                }
                7 => { graph.drop_all_named_durable().unwrap(); model.clear(); }
                8 => {
                    graph.clear_named_durable(&n).unwrap();
                    if let Some((_, triples)) = model.iter_mut().find(|(m, _)| *m == n) { triples.clear(); }
                }
                9 => { graph = graph.fork(); }
                10 => {
                    let names = [name(11), n.clone(), n];
                    graph = graph.fork_scoped(&names);
                    model.retain(|(m, _)| names.contains(m));
                }
                11 => { graph = std::sync::Arc::new(graph.snapshot()).clone().fork(); }
                12 => { graph = graph.into_compressed(); }
                13 => { graph.replace_default(Graph::new()).unwrap(); }
                14 => { graph.rebuild_default(|_, _| ()).unwrap(); }
                _ => { graph.vacuum().unwrap(); }
            }
            assert_state(&graph, &model);
        }
    }
}

#[test]
fn exact_lookup_work_is_bounded() {
    use named_index::COMPARISONS;
    for count in [256, 4096, 16384] {
        let mut graph = Graph::new();
        for i in 0..count {
            graph
                .add_named(
                    NamedNode::new(format!("urn:graph:{i}")).unwrap().into(),
                    Graph::new(),
                )
                .unwrap();
        }
        let n = graph.named()[count - 1].0.clone();
        COMPARISONS.with(|c| c.set(0));
        assert!(graph.named_graph(&n).is_some());
        assert!(
            (1..=64).contains(&COMPARISONS.with(|c| c.get())),
            "exact lookup must not scan named graphs"
        );
        COMPARISONS.with(|c| c.set(0));
        assert_eq!(graph.ensure_named(&n).unwrap(), count - 1);
        assert!(
            (1..=64).contains(&COMPARISONS.with(|c| c.get())),
            "ensure must use exact lookup"
        );
        COMPARISONS.with(|c| c.set(0));
        let scoped = graph.fork_scoped(&[n.clone(), n]);
        assert_eq!(scoped.named().len(), 1);
        assert!(
            (2..=128).contains(&COMPARISONS.with(|c| c.get())),
            "scoped selection must not scan named graphs"
        );
        COMPARISONS.with(|c| c.set(0));
        assert!(graph
            .named_graph(&NamedNode::new("urn:missing").unwrap().into())
            .is_none());
        assert!(
            COMPARISONS.with(|c| c.get()) <= 64,
            "missing lookup must not scan named graphs"
        );
        let before = graph.heap_bytes();
        assert_eq!(
            before - Graph::new().heap_bytes(),
            graph.named_index.heap_bytes(),
            "account for the exact index allocation"
        );
        assert!(before > Graph::new().heap_bytes());
    }
}

#[cfg(feature = "mmap")]
mod durable {
    use super::*;

    struct TempDir(std::path::PathBuf);
    impl Drop for TempDir {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
    fn disk_name(i: u8) -> Term {
        NamedNode::new(format!("urn:disk:{i}")).unwrap().into()
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 8, rng_seed: RngSeed::Fixed(0x73707188), .. ProptestConfig::default()
        })]
        #[test]
        fn named_index_survives_save_replay_and_rewrites(actions in prop::collection::vec((0u8..9, 0u8..6), 20..40)) {
            let tmp = TempDir(std::env::temp_dir().join(format!("sparq-nidx-{}", random_id())));
            Graph::new().save(&tmp.0).unwrap();
            let mut graph = Graph::open(&tmp.0).unwrap();
            let mut model = Model::new();
            for (kind, i) in actions {
                let n = disk_name(i);
                match kind {
                    0 | 1 => {
                        let op = if kind == 0 { TxnOp::Create(n.clone()) } else { TxnOp::Drop(n.clone()) };
                        graph.commit_txn_ops(&[op]).unwrap();
                        if kind == 0 {
                            if !model.iter().any(|(m, _)| *m == n) { model.push((n.clone(), vec![])); }
                        } else { model.retain(|(m, _)| *m != n); }
                        drop(graph); // Leave a committed, unmaterialized journal for replay.
                        let reader = Graph::open_read_only(&tmp.0).unwrap();
                        assert_state(&reader, &model);
                        drop(reader);
                        graph = Graph::open(&tmp.0).unwrap();
                    }
                    2 | 3 => {
                        let old_slots: Vec<_> = graph.named.iter().map(|(n, sub)|
                            (n.clone(), sub.named_slot_in(&tmp.0.join(NAMED_SUBDIR)).unwrap())).collect();
                        if kind == 2 {
                            graph.ensure_named(&n).unwrap();
                            if !model.iter().any(|(m, _)| *m == n) { model.push((n.clone(), vec![])); }
                        } else {
                            graph.drop_named_durable(&n).unwrap(); model.retain(|(m, _)| *m != n);
                        }
                        for (n, slot) in old_slots {
                            if let Some(sub) = graph.named_graph(&n) {
                                assert_eq!(sub.named_slot_in(&tmp.0.join(NAMED_SUBDIR)), Some(slot), "survivors keep declared slot ids");
                            }
                        }
                    }
                    4 => { graph.compact().unwrap(); }
                    5 => { graph.vacuum().unwrap(); }
                    6 => {
                        let i = graph.ensure_named(&n).unwrap();
                        graph.apply_delta_to_named(i, &payload(7), &[]).unwrap();
                        match model.iter_mut().find(|(m, _)| *m == n) {
                            Some((_, triples)) => *triples = payload(7),
                            None => model.push((n.clone(), payload(7))),
                        }
                        drop(graph);
                        graph = Graph::open(&tmp.0).unwrap(); // Per-slot WAL replay.
                    }
                    7 => {
                        graph.clear_named_durable(&n).unwrap();
                        if let Some((_, triples)) = model.iter_mut().find(|(m, _)| *m == n) { triples.clear(); }
                    }
                    _ => {
                        let copy = tmp.0.with_extension("copy");
                        graph.save(&copy).unwrap();
                        let opened = Graph::open(&copy).unwrap();
                        assert_state(&opened, &model);
                        drop(opened);
                        std::fs::remove_dir_all(copy).unwrap();
                    }
                }
                assert_state(&graph, &model);
                assert_state(&std::sync::Arc::new(graph.snapshot()).clone(), &model);
                let selected = graph.fork_scoped(&[n.clone(), n.clone()]);
                let expected = model.iter().filter(|(m, _)| *m == n).cloned().collect();
                assert_state(&selected, &expected);
            }
            drop(graph);
        }
    }
}
