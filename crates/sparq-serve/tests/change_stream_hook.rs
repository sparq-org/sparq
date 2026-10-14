//! (sq-bdaw5) `ChangeLog::into_commit_hook` × `Writer::spawn_with_commit_hook`
//! against the PRODUCTION types (`Graph` snapshots, `GraphApplier`, SPARQL Updates):
//! commits made through the sequenced writer are recorded into the durable CDC log ON
//! THE WRITER THREAD — no caller-side lock, one record per published generation — and a
//! consumer replays them from disk exactly. This is the bead's end-to-end invariant:
//! recording no longer serialises submitters, yet the stream stays gapless, monotonic,
//! and durable-before-ack.

#![cfg(feature = "change-stream")]

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use sparq_core::Graph;
use sparq_serve::{
    ChangeLog, ChangeOp, GenerationRing, GraphApplier, PodId, Writer, WriterConfig,
};

fn scratch(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "sparq-cdc-hook-{}-{}-{:?}",
        tag,
        std::process::id(),
        std::thread::current().id()
    ))
}

fn empty_graph() -> Graph {
    Graph::load_str("", "ntriples").expect("empty graph")
}

/// Sorted (op, quad) pairs — the equality oracle for "same change set".
fn change_pairs(rec: &sparq_serve::ChangeRecord) -> Vec<(ChangeOp, String)> {
    let mut v: Vec<(ChangeOp, String)> =
        rec.changes.iter().map(|c| (c.op, c.quad.clone())).collect();
    v.sort();
    v
}

/// THE load-bearing invariant: submit SPARQL Updates through a hooked writer, and the
/// durable log holds ONE exact record per published generation — appended before the
/// submitter's ack (a separate reader handle polls it from disk the moment `submit`
/// returns), replayable after the writer is gone. (Mutation witness: drop the hook —
/// i.e. spawn with plain `Writer::spawn` — and every poll below comes back empty.)
#[test]
fn hooked_writer_records_every_commit_durably_before_ack() {
    let tmp = scratch("e2e");
    let _ = fs::remove_dir_all(&tmp);

    let ring = Arc::new(GenerationRing::<Graph>::new(empty_graph()));
    let log = ChangeLog::open(&tmp).expect("open log");
    let writer = Writer::spawn_with_commit_hook(
        ring.clone(),
        GraphApplier::new(),
        WriterConfig {
            adaptive_commit: true, // serial submits: one generation (= one record) each
            ..WriterConfig::default()
        },
        log.into_commit_hook(|e| panic!("record append failed: {e}")),
    );

    let pod = PodId::new("http://ex/g");
    let g1 = writer
        .submit(
            "INSERT DATA { <http://ex/s1> <http://ex/p> <http://ex/o1> }".to_string(),
            [pod.clone()],
        )
        .expect("commit 1");
    // Durable-before-ack: a SEPARATE reader handle already sees this commit's record.
    let seen = ChangeLog::open(&tmp).expect("reader open").poll(0).expect("poll");
    assert_eq!(seen.len(), 1, "record durably appended before the ack");
    assert_eq!(seen[0].generation, g1);

    let g2 = writer
        .submit(
            "DELETE DATA { <http://ex/s1> <http://ex/p> <http://ex/o1> } ; \
             INSERT DATA { <http://ex/s2> <http://ex/p> \"v\" }"
                .to_string(),
            [pod.clone()],
        )
        .expect("commit 2");
    drop(writer); // drain + join; the log handle lives on the writer thread and drops with it

    // Replay from disk: exactly one record per commit, gapless, quad-exact.
    let replayed = ChangeLog::open(&tmp).expect("reopen").poll(0).expect("poll all");
    assert_eq!(replayed.len(), 2);
    assert_eq!((replayed[0].seq, replayed[0].generation), (0, g1));
    assert_eq!((replayed[1].seq, replayed[1].generation), (1, g2));
    assert_eq!(
        change_pairs(&replayed[0]),
        vec![(
            ChangeOp::Insert,
            "<http://ex/s1> <http://ex/p> <http://ex/o1> .".to_string()
        )]
    );
    assert_eq!(
        change_pairs(&replayed[1]),
        vec![
            (
                ChangeOp::Insert,
                "<http://ex/s2> <http://ex/p> \"v\" .".to_string()
            ),
            (
                ChangeOp::Delete,
                "<http://ex/s1> <http://ex/p> <http://ex/o1> .".to_string()
            ),
        ]
    );

    let _ = fs::remove_dir_all(&tmp);
}

/// DIRECT unit coverage of `into_commit_hook`, including the error policy: a failing
/// append (here: a non-forward pair, which `record_commit` rejects fail-closed) is
/// DROPPED and reported to `on_error` — the hook itself never panics or blocks — while
/// a valid pair still records normally afterwards.
#[test]
fn into_commit_hook_reports_append_failure_to_on_error_and_keeps_going() {
    let tmp = scratch("onerr");
    let _ = fs::remove_dir_all(&tmp);

    let ring: GenerationRing<Graph> = GenerationRing::new(empty_graph());
    let g0 = ring.current();
    let g1 = ring.publish(
        Graph::load_str("<http://ex/s> <http://ex/p> <http://ex/o> .\n", "ntriples")
            .expect("load g1"),
        [PodId::new("http://ex/g")],
    );

    let errors = std::sync::Mutex::new(Vec::<String>::new());
    let log = ChangeLog::open(&tmp).expect("open log");
    let mut hook = log.into_commit_hook(|e| errors.lock().unwrap().push(e.to_string()));

    // Non-forward pair: the append is rejected fail-closed → dropped + reported.
    hook(g1.as_ref(), g0.as_ref());
    assert_eq!(errors.lock().unwrap().len(), 1, "failure surfaced to on_error");

    // A valid pair still records (the hook survives a dropped record).
    hook(g0.as_ref(), g1.as_ref());
    assert_eq!(errors.lock().unwrap().len(), 1, "no spurious extra error");

    let recs = ChangeLog::open(&tmp).expect("reopen").poll(0).expect("poll");
    assert_eq!(recs.len(), 1, "exactly the valid pair was recorded");
    assert_eq!(recs[0].generation, g1.number());
    assert_eq!(recs[0].insert_count(), 1);

    let _ = fs::remove_dir_all(&tmp);
}
