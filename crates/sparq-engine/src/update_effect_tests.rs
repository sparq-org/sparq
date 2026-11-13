//! Differential oracle for ordered effect resolution and its deferred slot reads.

use super::*;
use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};

fn term(local: &str) -> Term {
    oxrdf::NamedNode::new(format!("http://example/{local}"))
        .unwrap()
        .into()
}

fn triple(id: u8) -> TripleTerms {
    [term(&format!("s{id}")), term("p"), term("o")]
}

fn slot(id: u8) -> GraphSlot {
    (id != 0).then(|| term(&format!("g{id}")))
}

fn target(id: u8) -> GraphTarget {
    match id {
        0 => GraphTarget::DefaultGraph,
        6 => GraphTarget::NamedGraphs,
        7 => GraphTarget::AllGraphs,
        _ => {
            GraphTarget::NamedNode(oxrdf::NamedNode::new(format!("http://example/g{id}")).unwrap())
        }
    }
}

fn delta(id: u8, inserts: &[u8], deletes: &[u8]) -> UpdateEffect {
    UpdateEffect::Delta {
        slot: slot(id),
        inserts: inserts.iter().copied().map(triple).collect(),
        deletes: deletes.iter().copied().map(triple).collect(),
    }
}

fn seed() -> Graph {
    let mut graph = Graph::new();
    for id in 0..4 {
        apply_slot_delta(&mut graph, &slot(id), &[triple(0), triple(1)], &[]).unwrap();
    }
    graph.ensure_named(&term("empty")).unwrap();
    graph
}

fn contents(graph: &Graph) -> (TripleSet, FxHashMap<Term, TripleSet>) {
    (
        decode_triples(graph),
        graph
            .named()
            .iter()
            .map(|(n, g)| (n.clone(), decode_triples(g)))
            .collect(),
    )
}

// Replay records in append order with the same set/existence rules as the journal.
fn replay(graph: &Graph, ops: &[TxnOp]) -> Graph {
    let mut out = graph.fork();
    for op in ops {
        match op {
            TxnOp::Insert { slot, triple } => {
                apply_slot_delta(&mut out, slot, std::slice::from_ref(triple), &[]).unwrap();
            }
            TxnOp::Delete { slot, triple } => {
                apply_slot_delta(&mut out, slot, &[], std::slice::from_ref(triple)).unwrap();
            }
            TxnOp::Create(name) => {
                out.ensure_named(name).unwrap();
            }
            TxnOp::Drop(name) => {
                out.drop_named_durable(name).unwrap();
            }
        }
    }
    out
}

#[test]
fn lazy_effect_resolution_matches_eager_sequences() {
    let strategy = prop::collection::vec(
        (
            0u8..5,
            0u8..8,
            prop::collection::vec(0u8..8, 0..6),
            prop::collection::vec(0u8..8, 0..6),
        ),
        0..64,
    );
    let mut runner = TestRunner::new_with_rng(
        Config {
            cases: 256,
            failure_persistence: None,
            ..Config::default()
        },
        TestRng::from_seed(RngAlgorithm::ChaCha, &[71; 32]),
    );
    runner
        .run(&strategy, |sequence| {
            let graph = seed();
            // Guarantee pending and materialised deltas, overlapping insert/delete bags,
            // pre-existing retractions, empty creation, and re-insertion after DROP.
            let mut effects = vec![
                delta(1, &[2, 2, 1], &[0, 1, 7]),
                UpdateEffect::Clear(target(1)),
                delta(1, &[3, 3], &[3]),
                UpdateEffect::Clear(target(1)),
                delta(1, &[4], &[]),
                UpdateEffect::Drop(target(1)),
                UpdateEffect::Create(term("g1")),
                delta(1, &[5], &[]),
                UpdateEffect::Create(term("g5")),
            ];
            for (op, id, inserts, deletes) in sequence {
                effects.push(match op {
                    0 | 4 => delta(id % 6, &inserts, &deletes),
                    1 => UpdateEffect::Clear(target(id)),
                    2 => UpdateEffect::Drop(target(id)),
                    _ => UpdateEffect::Create(term(&format!("g{}", id % 6))),
                });
            }
            let lazy = resolve_effect_ops(&graph, &effects);
            let eager = eager_effect_ops(&graph, &effects);
            prop_assert_eq!(
                &lazy,
                &eager,
                "resolved records must preserve order and multiplicity"
            );
            let mut applied = graph.fork();
            materialize_effects(&mut applied, &effects).unwrap();
            prop_assert_eq!(
                contents(&replay(&graph, &lazy)),
                contents(&replay(&graph, &eager))
            );
            prop_assert_eq!(
                contents(&replay(&graph, &lazy)),
                contents(&applied),
                "journal replay must match materialisation, including empty graph existence"
            );
            Ok(())
        })
        .unwrap();
}

#[test]
fn delta_only_parent_effects_do_not_decode_slots() {
    for size in [1, 10_000] {
        let mut graph = Graph::new();
        let triples: Vec<_> = (0..size)
            .map(|i| [term("parent"), term("contains"), term(&format!("child{i}"))])
            .collect();
        apply_slot_delta(&mut graph, &slot(1), &triples, &[]).unwrap();
        let effects = vec![
            delta(1, &[1, 1], &[2]),
            delta(1, &[3], &[1]),
            delta(0, &[4], &[]),
        ];
        crate::dataset::TRIPLE_DECODES.with(|count| count.set(0));
        let ops = resolve_effect_ops(&graph, &effects);
        let decodes = crate::dataset::TRIPLE_DECODES.with(|count| count.get());
        assert_eq!(
            decodes, 0,
            "delta-only resolution must not decode any slot (parent size {size})"
        );
        assert_eq!(ops, eager_effect_ops(&graph, &effects));
        assert_eq!(ops.len(), 6, "duplicate delta records must be retained");
        // Check the probe is live on the content-reading path as well.
        crate::dataset::TRIPLE_DECODES.with(|count| count.set(0));
        let mut clearing = effects;
        clearing.push(UpdateEffect::Clear(target(1)));
        resolve_effect_ops(&graph, &clearing);
        assert_eq!(
            crate::dataset::TRIPLE_DECODES.with(|count| count.get()),
            1,
            "a later CLEAR must decode its slot exactly once"
        );
    }
}

// Previous eager-on-first-touch resolver, deliberately independent of the lazy state.
fn eager_effect_ops(graph: &Graph, effects: &[UpdateEffect]) -> Vec<TxnOp> {
    let mut records: Vec<TxnOp> = Vec::new();

    let mut running: Vec<(GraphSlot, TripleSet)> = Vec::new();

    fn slot_mut<'a>(
        graph: &Graph,
        running: &'a mut Vec<(GraphSlot, TripleSet)>,
        slot: &GraphSlot,
    ) -> &'a mut TripleSet {
        match running.iter().position(|(s, _)| s == slot) {
            Some(i) => &mut running[i].1,
            None => {
                let seed = match slot {
                    None => decode_triples(graph),
                    Some(name) => graph
                        .named_graph(name)
                        .map(decode_triples)
                        .unwrap_or_default(),
                };
                running.push((slot.clone(), seed));
                let i = running.len() - 1;
                &mut running[i].1
            }
        }
    }

    fn clear_slot(
        graph: &Graph,
        records: &mut Vec<TxnOp>,
        running: &mut Vec<(GraphSlot, TripleSet)>,
        slot: &GraphSlot,
    ) {
        let view = slot_mut(graph, running, slot);
        for t in view.drain() {
            records.push(TxnOp::Delete {
                slot: slot.clone(),
                triple: t,
            });
        }
    }

    fn drop_slot(
        graph: &Graph,
        records: &mut Vec<TxnOp>,
        running: &mut Vec<(GraphSlot, TripleSet)>,
        slot: &GraphSlot,
    ) {
        clear_slot(graph, records, running, slot);
        if let Some(name) = slot {
            records.push(TxnOp::Drop(name.clone()));
        }
    }

    fn named_slots(graph: &Graph, running: &[(GraphSlot, TripleSet)]) -> Vec<GraphSlot> {
        let mut names: Vec<GraphSlot> =
            graph.named().iter().map(|(n, _)| Some(n.clone())).collect();
        for (slot, _) in running {
            if slot.is_some() && !names.contains(slot) {
                names.push(slot.clone());
            }
        }
        names
    }

    for effect in effects {
        match effect {
            UpdateEffect::Delta {
                slot,
                inserts,
                deletes,
            } => {
                let view = slot_mut(graph, &mut running, slot);
                for t in deletes {
                    records.push(TxnOp::Delete {
                        slot: slot.clone(),
                        triple: t.clone(),
                    });
                    view.remove(t);
                }
                for t in inserts {
                    records.push(TxnOp::Insert {
                        slot: slot.clone(),
                        triple: t.clone(),
                    });
                    view.insert(t.clone());
                }
            }
            UpdateEffect::Clear(target) => match target {
                GraphTarget::DefaultGraph => clear_slot(graph, &mut records, &mut running, &None),
                GraphTarget::NamedNode(n) => clear_slot(
                    graph,
                    &mut records,
                    &mut running,
                    &Some(Term::NamedNode(n.clone())),
                ),
                GraphTarget::NamedGraphs => {
                    for name in named_slots(graph, &running) {
                        clear_slot(graph, &mut records, &mut running, &name);
                    }
                }
                GraphTarget::AllGraphs => {
                    clear_slot(graph, &mut records, &mut running, &None);
                    for name in named_slots(graph, &running) {
                        clear_slot(graph, &mut records, &mut running, &name);
                    }
                }
            },
            UpdateEffect::Drop(target) => match target {
                GraphTarget::DefaultGraph => clear_slot(graph, &mut records, &mut running, &None),
                GraphTarget::NamedNode(n) => drop_slot(
                    graph,
                    &mut records,
                    &mut running,
                    &Some(Term::NamedNode(n.clone())),
                ),
                GraphTarget::NamedGraphs => {
                    for name in named_slots(graph, &running) {
                        drop_slot(graph, &mut records, &mut running, &name);
                    }
                }
                GraphTarget::AllGraphs => {
                    clear_slot(graph, &mut records, &mut running, &None);
                    for name in named_slots(graph, &running) {
                        drop_slot(graph, &mut records, &mut running, &name);
                    }
                }
            },
            UpdateEffect::Create(name) => {
                slot_mut(graph, &mut running, &Some(name.clone()));
                records.push(TxnOp::Create(name.clone()));
            }
        }
    }
    records
}
