// written while Fable 5 unavailable — re-review when Fable returns.
//! Commutativity-batch correctness (Wave A2 §6.5; litreview A §A.2(c)).
//!
//! `CommitGranularity::CommuteGroup` splits a window's batch into maximal runs of
//! provably-commuting pure-data updates and publishes one generation per run; a
//! barrier (WHERE-driven / graph-management / unparseable update, or a same-graph
//! opposite-polarity data update) starts a new generation. Application order is
//! FIFO in BOTH modes, so the committed dataset must be byte-identical to serial
//! execution — these tests assert that DIFFERENTIALLY: the same update stream run
//! under `Window` and under `CommuteGroup` (and against a hand-applied serial
//! reference) yields the same final graph.
//!
//! They also assert the *grouping* itself: how many generations each granularity
//! produces, which is the observable consequence of the conservative graph-level
//! conflict rule.

use std::sync::Arc;
use std::time::Duration;

use sparq_core::Graph;
use sparq_serve::{CommitGranularity, GenerationRing, GraphApplier, PodId, Writer, WriterConfig};

fn empty_graph() -> Graph {
    Graph::load_str("", "ntriples").expect("empty graph")
}

/// Total quad count across the default graph AND every named graph.
/// `Graph::len()` counts the default graph only; commutativity batching here
/// targets NAMED graphs (the Solid pod-per-named-graph conflict unit, §6.5), so
/// the differential reference must span all graphs.
fn total_quads(g: &Graph) -> usize {
    sparq_engine::count(
        g,
        "SELECT * WHERE { { ?s ?p ?o } UNION { GRAPH ?g { ?s ?p ?o } } }",
    )
    .expect("count query")
}

/// Drains a fixed update stream through a writer at the given granularity and
/// returns (final triple count, final generation number). Sync submission with a
/// long window + small enough max_batch behaviour: we submit detached then close
/// by dropping, so the WHOLE stream lands in one window — exactly the case where
/// commute-grouping vs window-grouping diverges in generation count.
fn run_stream(granularity: CommitGranularity, updates: &[(&str, &str)]) -> (usize, u64) {
    let ring = Arc::new(GenerationRing::new(empty_graph()));
    let writer = Writer::spawn(
        ring.clone(),
        GraphApplier::new(),
        // Wide window + large max_batch so every update joins ONE window; the
        // granularity then decides how many generations that window publishes.
        WriterConfig {
            window: Duration::from_millis(120),
            max_batch: 4096,
            granularity,
            ..WriterConfig::default()
        },
    );
    // Submit all but the last detached, then a sync submit to force the window to
    // have opened and collected everyone before we read.
    for (upd, pod) in &updates[..updates.len() - 1] {
        writer
            .submit_detached((*upd).to_string(), [PodId::from(*pod)])
            .unwrap();
    }
    let (last_upd, last_pod) = updates[updates.len() - 1];
    writer
        .submit(last_upd.to_string(), [PodId::from(last_pod)])
        .unwrap();
    drop(writer); // drain any straggler
    let current = ring.current();
    (total_quads(current.snapshot()), current.number())
}

/// A serial reference: apply the same updates one at a time to a fresh graph.
fn serial_result(updates: &[(&str, &str)]) -> usize {
    use sparq_serve::ApplyUpdates;
    let mut applier = GraphApplier::new();
    let mut g = empty_graph();
    for (upd, _) in updates {
        let mut w = applier.fork(&g).unwrap();
        applier.apply(&mut w, &(*upd).to_string()).unwrap();
        g = applier.seal(w).unwrap(); // (sq-vpx4) seal is now fallible (in-memory: never Err)
    }
    total_quads(&g)
}

const INS_A0: &str = "INSERT DATA { GRAPH <http://ex/a> { <http://ex/s0> <http://ex/p> \"v\" } }";
const INS_A1: &str = "INSERT DATA { GRAPH <http://ex/a> { <http://ex/s1> <http://ex/p> \"v\" } }";
const INS_B0: &str = "INSERT DATA { GRAPH <http://ex/b> { <http://ex/s0> <http://ex/p> \"v\" } }";
const INS_B1: &str = "INSERT DATA { GRAPH <http://ex/b> { <http://ex/s1> <http://ex/p> \"v\" } }";
const DEL_A0: &str = "DELETE DATA { GRAPH <http://ex/a> { <http://ex/s0> <http://ex/p> \"v\" } }";

/// Differential: disjoint-graph inserts commute. Window and CommuteGroup must
/// produce the SAME final graph as serial application. Because all four inserts
/// target distinct graph slots with no opposite polarity, CommuteGroup forms a
/// single commuting run → still ONE generation (same as Window).
#[test]
fn disjoint_inserts_commute_identical_result_one_group() {
    let stream = [(INS_A0, "a"), (INS_B0, "b"), (INS_A1, "a"), (INS_B1, "b")];
    let serial = serial_result(&stream);
    assert_eq!(serial, 4);

    let (win_len, win_gen) = run_stream(CommitGranularity::Window, &stream);
    let (cg_len, cg_gen) = run_stream(CommitGranularity::CommuteGroup, &stream);

    assert_eq!(win_len, serial, "Window result == serial");
    assert_eq!(cg_len, serial, "CommuteGroup result == serial");
    assert_eq!(win_gen, 1, "Window: one window, one generation");
    assert_eq!(
        cg_gen, 1,
        "CommuteGroup: all four are same/disjoint-graph same-polarity inserts → one commuting run"
    );
}

/// Differential: an opposite-polarity pair on the SAME graph is a barrier between
/// groups. Insert-a, delete-a (same graph, opposite polarity) cannot commute, so
/// CommuteGroup splits into two generations; the FINAL result is still identical
/// to serial (FIFO order is preserved across groups).
#[test]
fn same_graph_opposite_polarity_splits_but_result_identical() {
    // INS_A0 then DEL_A0 (deletes what was inserted) then INS_A1.
    let stream = [(INS_A0, "a"), (DEL_A0, "a"), (INS_A1, "a")];
    let serial = serial_result(&stream);
    assert_eq!(
        serial, 1,
        "insert s0, delete s0, insert s1 → only s1 remains"
    );

    let (win_len, win_gen) = run_stream(CommitGranularity::Window, &stream);
    let (cg_len, cg_gen) = run_stream(CommitGranularity::CommuteGroup, &stream);

    assert_eq!(win_len, serial);
    assert_eq!(
        cg_len, serial,
        "CommuteGroup result == serial despite group splits"
    );
    assert_eq!(
        win_gen, 1,
        "Window collapses the whole window into one generation"
    );
    assert!(
        cg_gen > 1,
        "CommuteGroup splits at the insert/delete polarity conflict (got {cg_gen} generations)"
    );
}

/// A WHERE-driven update is a barrier: it commutes with nothing, forcing a
/// singleton generation, but the final result is still serial-identical.
#[test]
fn where_update_is_a_barrier_result_identical() {
    let where_upd = "INSERT { GRAPH <http://ex/a> { ?s <http://ex/q> \"copy\" } } \
         WHERE { GRAPH <http://ex/a> { ?s <http://ex/p> \"v\" } }";
    let stream = [(INS_A0, "a"), (where_upd, "a"), (INS_B0, "b")];
    let serial = serial_result(&stream);
    // s0 inserted into a; WHERE copies it adding (s0, q, copy) in a; then b gets s0.
    assert_eq!(serial, 3);

    let (cg_len, cg_gen) = run_stream(CommitGranularity::CommuteGroup, &stream);
    assert_eq!(
        cg_len, serial,
        "barrier update still produces the serial result"
    );
    assert!(
        cg_gen >= 2,
        "the WHERE barrier forces at least one extra generation (got {cg_gen})"
    );
}

/// Pseudo-random differential fuzz: many mixed inserts/deletes across a small set
/// of graphs, run under both granularities and serial — all three must agree.
#[test]
fn fuzz_differential_window_vs_commute_vs_serial() {
    // A deterministic LCG so the stream is reproducible.
    let mut state: u64 = 0x9E3779B97F4A7C15;
    let mut next = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        state >> 33
    };
    let graphs = ["http://ex/g0", "http://ex/g1", "http://ex/g2"];
    let mut owned: Vec<(String, &str)> = Vec::new();
    for _ in 0..200 {
        let g = graphs[(next() % 3) as usize];
        let subj = next() % 20;
        let del = next() % 2 == 0;
        let op = if del { "DELETE" } else { "INSERT" };
        let upd =
            format!("{op} DATA {{ GRAPH <{g}> {{ <http://ex/s{subj}> <http://ex/p> \"v\" }} }}");
        let pod = g.rsplit('/').next().unwrap();
        owned.push((upd, pod));
    }
    let stream: Vec<(&str, &str)> = owned.iter().map(|(u, p)| (u.as_str(), *p)).collect();

    let serial = serial_result(&stream);
    let (win_len, _) = run_stream(CommitGranularity::Window, &stream);
    let (cg_len, _) = run_stream(CommitGranularity::CommuteGroup, &stream);

    assert_eq!(win_len, serial, "Window == serial over 200 mixed ops");
    assert_eq!(cg_len, serial, "CommuteGroup == serial over 200 mixed ops");
}
