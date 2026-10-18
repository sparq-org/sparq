//! `ClosureLimits::max_facts` over the GIVEN facts, and the `ClosureCharge` a closure
//! reports.
//!
//! The VERDICTS here are the allowance's long-standing contract and this file is their
//! regression guard: an input fact set past `max_facts` is refused, the boundary is exact,
//! and duplicates are charged once. Charging the input as it is READ (rather than after
//! the whole set is inserted and copied into the round-0 delta) does not change any of
//! them — it changes what a REFUSED run HOLDS, which is measured in
//! `n3_input_fact_memory.rs`, not here.
//!
//! `ClosureCharge` is new: it is what a caller running SEVERAL closures under one
//! aggregate cap of its own adds up.

use sparq_reason::n3::compiled::{compile, eval_charged, eval_with_limits, intern_facts};
use sparq_reason::ClosureLimits;

/// A rule set that derives nothing, so every fact in the closure is an input fact and the
/// count is unambiguous.
const INERT: &str = "@prefix : <http://ex/> . { ?x :never ?y } => { ?x :nothing ?y } .";

/// `n` distinct input facts.
fn facts(n: usize) -> String {
    let mut src = String::from("@prefix : <http://ex/> .\n");
    for i in 0..n {
        src.push_str(&format!(":s{} :p :o .\n", i));
    }
    src
}

fn limits(max_facts: usize) -> ClosureLimits {
    let mut l = ClosureLimits::default();
    l.max_facts = max_facts;
    l
}

/// An input fact set past `max_facts` is refused, with the forward-closure cut the text
/// engine uses.
#[test]
fn an_input_fact_set_past_the_fact_allowance_is_refused() {
    let rules = compile(INERT).expect("compile");
    let mut dict = sparq_core::dict::Dict::new();
    let input = intern_facts(&mut dict, &facts(500)).expect("intern");
    let err = eval_with_limits(&mut dict, &input, &rules, limits(100))
        .expect_err("500 input facts must not be held under a 100-fact allowance");
    assert!(
        err.contains("forward closure limit exceeded"),
        "the refusal must be the closure cut: {}",
        err
    );
    assert!(
        err.contains("100"),
        "the refusal names the allowance: {}",
        err
    );
}

/// The boundary: an input fact set exactly at the allowance is accepted, one fact more is
/// refused — the same either side of the input-side charge, which is why the memory test
/// is the one that guards it.
#[test]
fn the_input_fact_allowance_is_exact_at_its_boundary() {
    let rules = compile(INERT).expect("compile");
    // The rule document itself asserts no facts, so the store holds exactly the input.
    for (n, accepted) in [(64usize, true), (65, false)] {
        let mut dict = sparq_core::dict::Dict::new();
        let input = intern_facts(&mut dict, &facts(n)).expect("intern");
        assert_eq!(input.len(), n);
        let got = eval_with_limits(&mut dict, &input, &rules, limits(64));
        assert_eq!(
            got.is_ok(),
            accepted,
            "{} input facts under a 64-fact allowance: {:?}",
            n,
            got.err()
        );
    }
}

/// Duplicate input facts are charged once: the allowance bounds what the closure HOLDS,
/// so handing the same fact over repeatedly does not refuse a run that fits.
#[test]
fn duplicate_input_facts_are_charged_once() {
    let rules = compile(INERT).expect("compile");
    let mut dict = sparq_core::dict::Dict::new();
    let one = intern_facts(&mut dict, &facts(4)).expect("intern");
    let mut repeated = Vec::new();
    for _ in 0..50 {
        repeated.extend_from_slice(&one);
    }
    assert_eq!(repeated.len(), 200);
    let closure = eval_with_limits(&mut dict, &repeated, &rules, limits(4))
        .expect("200 copies of 4 distinct facts hold 4 facts");
    assert_eq!(closure.len(), 4);
}

/// `eval_charged` reports the facts the fixpoint held and the rows it charged, so a caller
/// running several closures can cap their aggregate itself.
#[test]
fn a_closure_reports_what_it_charged() {
    let rules =
        compile("@prefix : <http://ex/> . { ?x a :Man } => { ?x a :Mortal } .").expect("compile");
    let mut dict = sparq_core::dict::Dict::new();
    let input = intern_facts(
        &mut dict,
        "@prefix : <http://ex/> . :a a :Man . :b a :Man .",
    )
    .expect("intern");
    let mut l = ClosureLimits::default();
    l.max_total_join_rows = 10_000;
    let (closure, charge) = eval_charged(&mut dict, &input, &rules, l).expect("eval");
    assert_eq!(closure.len(), 4, "2 inputs + 2 derivations");
    assert_eq!(charge.facts, 4);
    let rows = charge.join_rows.expect("a bounded total is counted");
    assert!(
        rows >= 4,
        "at least a row and a firing per input fact: {}",
        rows
    );
    assert!(rows <= 10_000);

    // Unbounded: nothing is counted at all, so there is no total to report.
    let mut dict2 = sparq_core::dict::Dict::new();
    let input2 = intern_facts(&mut dict2, "@prefix : <http://ex/> . :a a :Man .").expect("intern");
    let (_, unbounded) =
        eval_charged(&mut dict2, &input2, &rules, ClosureLimits::default()).expect("eval");
    assert_eq!(unbounded.join_rows, None);
    assert_eq!(unbounded.facts, 2);
}
