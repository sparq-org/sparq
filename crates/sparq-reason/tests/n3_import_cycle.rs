//! N3 `log:semantics` recursive-import CYCLE-SAFETY (bead sq-0fj4p, survey §A10).
//!
//! 🤖 SPARQ agent — sq-0fj4p N3 import-cycle detection.
//!
//! N3 is Turing-complete: a `log:semantics` document whose own closure re-imports a document
//! active up the resolution stack — directly (A->A), indirectly (A->B->A), or via a re-used
//! node in a diamond — drives the `formula_closure` -> nested-closure recursion forever under
//! a LIVE resolver. Without the guard this aborts the process with a stack overflow (a
//! denial-of-service on any production caller that wires a network/filesystem resolver). The
//! engine now breaks such a cyclic re-import (returns the in-progress formula unclosed,
//! cwm-style: a document already being loaded is not re-loaded), so reasoning always
//! TERMINATES. Valid acyclic imports — including a diamond that legitimately re-uses a shared
//! document across SIBLING branches — are unaffected.
//!
//! Every cyclic case runs on a watchdog thread: a regression (the guard removed) would hang or
//! crash the whole suite, so the watchdog turns non-termination into an explicit, bounded
//! FAILURE rather than a hang. The watchdog is EVENT-DRIVEN — a worker that finishes, or that
//! panics, is observed as soon as it does so (its result arrives, or the channel closes) and
//! the thread is joined — so the timeout below is a last-resort hang guard, never a time budget
//! a slow runner can trip. Documents use the `doc:`/`ex:` IRI spaces; the resolver maps a
//! `doc:` IRI to that document's N3 text (the production analogue of a deref).

use sparq_reason::n3::{reason_n3_terms_with_resolver, Resolver, Term};
use std::sync::mpsc;
use std::time::Duration;

const DOC: &str = "http://doc/";
const EX: &str = "http://ex/";

/// Last-resort hang guard for the watchdog worker, in seconds.
///
/// NOT a performance budget and NOT a measurement: a healthy run — including one that fails by
/// panicking — never waits on this value, because `with_watchdog` observes the worker's result
/// or its panic as it happens and joins the thread. The guard exists only so a genuine
/// NON-TERMINATION (the import-cycle guard removed) fails the suite instead of hanging CI
/// forever, and is deliberately far above any plausible honest runtime so a slow, heavily
/// loaded runner cannot trip it.
const HANG_GUARD_SECS: u64 = 300;

/// Run `f` on a worker thread and return its value, joining the thread.
///
/// A worker panic is re-raised on the test thread as soon as it happens (the panicking worker
/// drops the sender, which closes the channel), so a failing assertion inside `f` reports its
/// own message rather than a timeout. Only an actual hang reaches [`HANG_GUARD_SECS`].
fn with_watchdog<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    match rx.recv_timeout(Duration::from_secs(HANG_GUARD_SECS)) {
        // Event-driven: the worker has produced its result, so this join returns at once.
        Ok(value) => {
            if let Err(payload) = worker.join() {
                std::panic::resume_unwind(payload);
            }
            value
        }
        // The sender was dropped without a result: the worker panicked. Join to re-raise that
        // panic here, so the real failure surfaces instead of a timeout that never happened.
        Err(mpsc::RecvTimeoutError::Disconnected) => match worker.join() {
            Err(payload) => std::panic::resume_unwind(payload),
            Ok(()) => unreachable!("watchdog worker closed the channel without sending a result"),
        },
        Err(mpsc::RecvTimeoutError::Timeout) => panic!(
            "reasoning did not terminate within the {HANG_GUARD_SECS}s hang guard (import cycle \
             not broken). This guard bounds a HANG; it does not measure performance."
        ),
    }
}

#[test]
fn watchdog_reraises_worker_panic_promptly() {
    // The event-driven contract. `HANG_GUARD_SECS` is minutes long, so a watchdog that only
    // waited on the timeout — or that reported a timeout instead of the worker's own panic —
    // fails one of these two assertions.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {})); // the panic below is expected; keep the log clean
    let started = std::time::Instant::now();
    let caught = std::panic::catch_unwind(|| with_watchdog(|| panic!("worker exploded")));
    std::panic::set_hook(hook);

    let payload = caught.expect_err("a panicking worker must fail the test");
    let msg = payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_default();
    assert!(msg.contains("worker exploded"), "the worker's own panic must surface, got {msg:?}");
    assert!(
        started.elapsed() < Duration::from_secs(HANG_GUARD_SECS),
        "a panicking worker must be reported promptly, not after the hang guard"
    );
}

fn has(facts: &[[Term; 3]], s: &str, p: &str, o: &str) -> bool {
    let iri = |ns: &str, l: &str| Term::Iri(format!("{ns}{l}"));
    facts.iter().any(|t| *t == [iri(EX, s), iri(EX, p), iri(EX, o)])
}

const PRE: &str = "@prefix : <http://ex/> .\n\
                   @prefix doc: <http://doc/> .\n\
                   @prefix log: <http://www.w3.org/2000/10/swap/log#> .\n";

/// Reason `src` with a `(iri, text)` resolver, on a watchdog. Returns the closure facts.
fn reason_with_docs(src: &'static str, docs: Vec<(String, String)>) -> Vec<[Term; 3]> {
    with_watchdog(move || {
        let resolver =
            move |iri: &str| docs.iter().find(|(d, _)| d == iri).map(|(_, t)| t.clone());
        let r = &resolver;
        reason_n3_terms_with_resolver(src, None, Some(r as &Resolver)).expect("reasoning").facts
    })
}

// A document that imports `doc:<d>`'s semantics and closes it (log:supports). The
// supports-close is what re-enters the resolver and drives the recursion.
fn importer(d: &str) -> String {
    format!(
        "{PRE}{{ doc:{d} log:semantics ?g . ?g log:supports {{:x :y :z}} }} => {{ :top :saw :{d} }} ."
    )
}

#[test]
fn direct_self_import_terminates() {
    // Doc A imports its OWN semantics (A->A) and closes it. Without the guard: stack overflow.
    let src = "@prefix : <http://ex/> .\n\
               @prefix doc: <http://doc/> .\n\
               @prefix log: <http://www.w3.org/2000/10/swap/log#> .\n\
               { doc:a log:semantics ?g . ?g log:supports {:x :y :z} } => { :top :saw :a } .";
    let f = reason_with_docs(src, vec![(format!("{DOC}a"), importer("a"))]);
    // Load-bearing assertion: the run TERMINATED (the watchdog did not fire and the process
    // did not abort). The cyclic re-import contributes no extra fact, but reasoning completes.
    assert!(f.iter().all(|t| t.len() == 3), "terminated; facts: {:?}", f);
}

#[test]
fn indirect_import_cycle_terminates() {
    // A imports B, B imports A (A->B->A). Closing A's semantics re-imports A through B.
    let src = "@prefix : <http://ex/> .\n\
               @prefix doc: <http://doc/> .\n\
               @prefix log: <http://www.w3.org/2000/10/swap/log#> .\n\
               { doc:a log:semantics ?g . ?g log:supports {:x :y :z} } => { :top :saw :a } .";
    let f = reason_with_docs(
        src,
        vec![(format!("{DOC}a"), importer("b")), (format!("{DOC}b"), importer("a"))],
    );
    assert!(f.iter().all(|t| t.len() == 3), "terminated; facts: {:?}", f);
}

#[test]
fn diamond_reuse_still_works() {
    // doc:b and doc:c each import the SHARED node doc:d (a diamond top->{b,c}->d). D is acyclic
    // and re-used across two SIBLING branches — NOT a cycle. Each branch must successfully
    // import D and the run must terminate, so we positively observe BOTH sibling imports of the
    // shared node succeeding (D carries a marker its closure surfaces via log:includes).
    let d = format!("{PRE}:d-loaded :is :true .");
    let leaf = |branch: &str| {
        format!(
            "{PRE}{{ doc:d log:semantics ?g . ?g log:includes {{:d-loaded :is :true}} }} \
             => {{ :from-{branch} :d :ok }} ."
        )
    };
    let src = "@prefix : <http://ex/> .\n\
               @prefix doc: <http://doc/> .\n\
               @prefix log: <http://www.w3.org/2000/10/swap/log#> .\n\
               { doc:b log:semantics ?gb . ?gb log:supports {:from-b :d :ok} } => { :top :b :ok } .\n\
               { doc:c log:semantics ?gc . ?gc log:supports {:from-c :d :ok} } => { :top :c :ok } .";
    let f = reason_with_docs(
        src,
        vec![
            (format!("{DOC}b"), leaf("b")),
            (format!("{DOC}c"), leaf("c")),
            (format!("{DOC}d"), d),
        ],
    );
    assert!(
        has(&f, "top", "b", "ok"),
        "branch B re-importing the shared diamond node D must succeed; facts: {:?}",
        f
    );
    assert!(
        has(&f, "top", "c", "ok"),
        "sibling branch C re-importing the SAME node D must ALSO succeed (re-use is not a cycle); facts: {:?}",
        f
    );
}

#[test]
fn valid_acyclic_import_unchanged() {
    // The contract: a valid acyclic import behaves exactly as before the cycle guard — the
    // resolved document's semantics is inspectable and its supports-close fires the rule.
    let a = format!("{PRE}:x :y :z .\n{{:x :y :z}} => {{:derived :ok :yes}} .");
    let src = "@prefix : <http://ex/> .\n\
               @prefix doc: <http://doc/> .\n\
               @prefix log: <http://www.w3.org/2000/10/swap/log#> .\n\
               { doc:a log:semantics ?g . ?g log:supports {:derived :ok :yes} } \
               => { :top :closed :a } .";
    let f = reason_with_docs(src, vec![(format!("{DOC}a"), a)]);
    assert!(
        has(&f, "top", "closed", "a"),
        "an acyclic log:semantics import must close and fire exactly as before; facts: {:?}",
        f
    );
}
