//! Cross-process `odrl:count` enforcement tests for [`FileCounterStore`] — the
//! distributed-atomicity follow-up (sq-5z1q) to the in-process `InMemoryCounterStore`.
//! [OPUS-4.8].
//!
//! Coverage:
//! - the file store driven through the **real** `evaluate_and_exercise` path (first N
//!   exercises grant, the (N+1)th denies), proving it is a drop-in for the trait seam;
//! - persistence across store *instances* over the same directory; and
//! - **genuinely cross-process** atomicity: this test binary re-spawns ITSELF as N worker
//!   subprocesses that hammer one shared count file, and asserts the total grants equal
//!   exactly the limit — never an over-grant. This is the claim the bead is about, tested
//!   across real OS processes (not just threads).
//!
//! Gated on `count-enforcement` (the whole feature).
#![cfg(feature = "count-enforcement")]

use sparq_policy::{
    evaluate_and_exercise, parse_policy_str, ConsumeResult, CountKey, FileCounterStore, Request,
    UsageCounterStore,
};
use std::path::PathBuf;

const ODRL: &str = "http://www.w3.org/ns/odrl/2/";

/// Env var the worker subprocess reads to know which shared count dir + limit to hammer.
const WORKER_DIR: &str = "SPARQ_FILECOUNT_WORKER_DIR";
const WORKER_LIMIT: &str = "SPARQ_FILECOUNT_WORKER_LIMIT";
const WORKER_ATTEMPTS: &str = "SPARQ_FILECOUNT_WORKER_ATTEMPTS";

fn read() -> String {
    format!("{ODRL}read")
}

fn policy_at_most(bound: &str) -> sparq_policy::ValidatedPolicy {
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<urn:pol/c> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:assignee <https://alice.ex/me> ;
    odrl:constraint [ odrl:leftOperand odrl:count ;
                      odrl:operator odrl:lteq ;
                      odrl:rightOperand "{bound}"^^xsd:integer ] ] .
"#
    );
    parse_policy_str(&ttl, "turtle").unwrap()
}

fn alice_reads() -> Request {
    Request::new(read())
        .on("urn:asset/x")
        .by("https://alice.ex/me")
}

fn unique_dir(tag: &str) -> PathBuf {
    let mut d = std::env::temp_dir();
    d.push(format!("sparq-filecount-it-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

// ---------------------------------------------------------------------------
// 1. The file store drives the REAL evaluate_and_exercise path: first N grant,
//    (N+1)th denies, and the count persists to disk.
// ---------------------------------------------------------------------------
#[test]
fn file_store_through_real_evaluator() {
    let dir = unique_dir("real-eval");
    let p = policy_at_most("3");
    let store = FileCounterStore::new(&dir).unwrap();
    let req = alice_reads();

    for expected in 1..=3 {
        let d = evaluate_and_exercise(&p, &req, &store);
        assert!(d.allow, "exercise {expected} should grant: {d:?}");
        assert_eq!(d.consumed, Some(expected));
    }
    let d = evaluate_and_exercise(&p, &req, &store);
    assert!(!d.allow, "4th exercise must deny (limit reached)");
    assert_eq!(d.consumed, None);

    // The count is on disk: a brand-new store over the SAME dir sees 3 consumed and still
    // denies — exactly the cross-process sharing the in-memory store cannot give.
    let key = CountKey {
        rule_id: p.permissions[0].id.clone(),
        party: "https://alice.ex/me".into(),
        target: "urn:asset/x".into(),
    };
    let reopened = FileCounterStore::new(&dir).unwrap();
    assert_eq!(reopened.consumed(&key), Some(3));
    assert!(
        !evaluate_and_exercise(&p, &req, &reopened).allow,
        "reopened store still denies"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 2. A base DENY (wrong party) burns no budget — through the file store.
// ---------------------------------------------------------------------------
#[test]
fn file_store_base_deny_consumes_nothing() {
    let dir = unique_dir("base-deny");
    let p = policy_at_most("2");
    let store = FileCounterStore::new(&dir).unwrap();
    let mallory = Request::new(read())
        .on("urn:asset/x")
        .by("https://mallory.ex/me");
    assert!(!evaluate_and_exercise(&p, &mallory, &store).allow);
    // Alice still has her full budget.
    assert!(evaluate_and_exercise(&p, &alice_reads(), &store).allow);
    assert!(evaluate_and_exercise(&p, &alice_reads(), &store).allow);
    assert!(!evaluate_and_exercise(&p, &alice_reads(), &store).allow);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 3. CROSS-PROCESS ATOMICITY: re-spawn this test binary as N worker subprocesses, all
//    hammering ONE shared count file; assert the grand total of grants equals EXACTLY the
//    limit — never an over-grant. This is the bead's core claim, tested across real OS
//    processes (the in-memory store would over-grant N×limit here).
// ---------------------------------------------------------------------------
#[test]
fn cross_process_never_overgrants() {
    // If we are a spawned worker, do the work and exit (handled before the harness even
    // gets here via the module-level entry below). The parent path continues:
    const LIMIT: u64 = 60;
    const WORKERS: usize = 6;
    const ATTEMPTS_PER_WORKER: usize = 40; // 240 attempts >> 60 budget

    let dir = unique_dir("xproc");
    std::fs::create_dir_all(&dir).unwrap();
    let exe = std::env::current_exe().expect("test binary path");

    let mut children = Vec::new();
    for _ in 0..WORKERS {
        let child = std::process::Command::new(&exe)
            // Run ONLY the worker entry test in the child, quietly.
            .args(["--exact", "cross_process_worker_entry", "--nocapture"])
            .env(WORKER_DIR, &dir)
            .env(WORKER_LIMIT, LIMIT.to_string())
            .env(WORKER_ATTEMPTS, ATTEMPTS_PER_WORKER.to_string())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn worker");
        children.push(child);
    }

    // Each worker prints `GRANTED <n>` on its stdout; sum them.
    let mut total_granted: u64 = 0;
    for child in children {
        let out = child.wait_with_output().expect("worker finished");
        assert!(out.status.success(), "worker exited non-zero");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let n: u64 = stdout
            .lines()
            .find_map(|l| l.strip_prefix("GRANTED "))
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or_else(|| panic!("worker did not report GRANTED; stdout was:\n{stdout}"));
        total_granted += n;
    }

    // EXACTLY the limit was granted across ALL processes — the file lock closed the
    // cross-process TOCTOU window. The in-memory store would have granted up to
    // WORKERS*LIMIT here.
    assert_eq!(
        total_granted, LIMIT,
        "cross-process grants must equal exactly the limit, never over-grant"
    );

    // And the persisted counter agrees.
    let store = FileCounterStore::new(&dir).unwrap();
    let key = CountKey {
        rule_id: "r".into(),
        party: "alice".into(),
        target: "x".into(),
    };
    assert_eq!(store.consumed(&key), Some(LIMIT));

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// The worker entry point: a real #[test] the parent re-invokes by name in a child
// process. It hammers the shared count file and prints `GRANTED <n>`. When NOT run as a
// worker (no env var) it is a no-op, so it does nothing in a normal `cargo test` run.
// ---------------------------------------------------------------------------
#[test]
fn cross_process_worker_entry() {
    let Ok(dir) = std::env::var(WORKER_DIR) else {
        return; // not a worker invocation → no-op
    };
    let limit: u64 = std::env::var(WORKER_LIMIT).unwrap().parse().unwrap();
    let attempts: usize = std::env::var(WORKER_ATTEMPTS).unwrap().parse().unwrap();

    let store = FileCounterStore::new(&dir).unwrap();
    let key = CountKey {
        rule_id: "r".into(),
        party: "alice".into(),
        target: "x".into(),
    };
    let mut granted = 0u64;
    for _ in 0..attempts {
        if matches!(store.try_consume(&key, limit), ConsumeResult::Consumed(_)) {
            granted += 1;
        }
    }
    println!("GRANTED {granted}");
}
