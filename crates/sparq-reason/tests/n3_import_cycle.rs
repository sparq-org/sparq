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
//! FAILURE rather than a hang. Documents use the `doc:`/`ex:` IRI spaces; the resolver maps a
//! `doc:` IRI to that document's N3 text (the production analogue of a deref).

use sparq_reason::n3::{reason_n3_terms_with_resolver, Resolver, Term};
use std::sync::mpsc;
use std::time::Duration;

const DOC: &str = "http://doc/";
const EX: &str = "http://ex/";

/// Run `f` on a worker thread, failing the test if it does not finish within `secs`. A
/// non-terminating reasoner (the bug this guards) would otherwise hang CI indefinitely.
fn with_watchdog<T: Send + 'static>(secs: u64, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(Duration::from_secs(secs))
        .expect("reasoning did not terminate within the watchdog window (import cycle not broken)")
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
    with_watchdog(30, move || {
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
