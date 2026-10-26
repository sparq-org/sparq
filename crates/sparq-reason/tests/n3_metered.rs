//! Running-ledger caps compose with each closure's existing limits.

use sparq_core::dict::{Dict, Id};
use sparq_core::meter::{Dim, Ledger, Limits};
use sparq_reason::n3::compiled::{
    compile, eval_charged, eval_metered, intern_facts, CompiledRuleSet, MeteredError,
};
use sparq_reason::ClosureLimits;

fn fixture() -> (Dict, Vec<[Id; 3]>, CompiledRuleSet) {
    let mut d = Dict::new();
    let f = intern_facts(&mut d, "@prefix : <urn:> . :a :p :b . :b :p :c .").unwrap();
    let r =
        compile("@prefix : <urn:> . { ?x :p ?y } => { ?x :q ?y } . { ?x :q ?y } => { ?x :r ?y } .")
            .unwrap();
    (d, f, r)
}

fn usage(dim: Dim) -> usize {
    let (mut d, f, r) = fixture();
    let mut limits = ClosureLimits::default();
    limits.max_total_join_rows = usize::MAX - 1;
    let (_, c) = eval_charged(&mut d, &f, &r, limits).unwrap();
    match dim {
        Dim::JoinRows => c.join_rows.unwrap(),
        Dim::ClosureFacts => c.facts,
        _ => unreachable!(),
    }
}

fn ledger(dim: Dim, cap: usize) -> Ledger {
    let mut limits = Limits::default();
    match dim {
        Dim::JoinRows => limits.join_rows = cap,
        Dim::ClosureFacts => limits.closure_facts = cap,
        _ => unreachable!(),
    }
    Ledger::new(limits)
}

#[test]
fn metered_refuses_closure_facts_while_reading_input() {
    let (mut d, f, r) = fixture();
    let mut l = ledger(Dim::ClosureFacts, 4);
    l.charge(Dim::ClosureFacts, 3).unwrap();
    assert!(l.remaining(Dim::ClosureFacts) < f.len());
    let e = eval_metered(&mut d, &f, &r, ClosureLimits::default(), &mut l).unwrap_err();
    assert!(matches!(e, MeteredError::Ledger(e)
        if e.dim == Dim::ClosureFacts && e.limit == 4 && e.reached == 5));
    assert_eq!(l.spent(Dim::ClosureFacts), 3);
    assert_eq!(l.spent(Dim::JoinRows), 0);
}

#[test]
fn metered_refuses_at_remaining_cap_without_debit() {
    for dim in [Dim::JoinRows, Dim::ClosureFacts] {
        let needed = usage(dim);
        let (mut d, f, r) = fixture();
        let remaining = needed / 2;
        let mut l = ledger(dim, remaining + 3);
        l.charge(dim, 3).unwrap();
        let e = eval_metered(&mut d, &f, &r, ClosureLimits::default(), &mut l).unwrap_err();
        match e {
            MeteredError::Ledger(e) => {
                assert_eq!(e.dim, dim);
                assert_eq!(e.limit, remaining + 3);
                assert_eq!(e.reached, remaining + 4);
            }
            e => panic!("wrong refusal for {dim:?}: {e}"),
        }
        assert_eq!(l.spent(dim), 3);
        assert_eq!(l.remaining(dim), remaining);
        let other = if dim == Dim::JoinRows {
            Dim::ClosureFacts
        } else {
            Dim::JoinRows
        };
        assert_eq!(l.spent(other), 0);
    }
}

#[test]
fn metered_accepts_exact_cap_and_charges_actual_work() {
    for dim in [Dim::JoinRows, Dim::ClosureFacts] {
        let (mut d, f, r) = fixture();
        let mut l = ledger(dim, usage(dim) + 3);
        l.charge(dim, 3).unwrap();
        let (out, c) = r
            .bind(&mut d)
            .eval_metered(&mut d, &f, ClosureLimits::default(), &mut l)
            .unwrap();
        assert_eq!(out.len(), c.facts);
        assert_eq!(l.remaining(dim), 0);
        assert_eq!(
            l.spent(Dim::JoinRows),
            c.join_rows.unwrap() + if dim == Dim::JoinRows { 3 } else { 0 }
        );
        assert_eq!(
            l.spent(Dim::ClosureFacts),
            c.facts + if dim == Dim::ClosureFacts { 3 } else { 0 }
        );
    }
}

#[test]
fn metered_sequential_closures_share_only_what_is_left() {
    for dim in [Dim::JoinRows, Dim::ClosureFacts] {
        let needed = usage(dim);
        let (mut d, f, r) = fixture();
        let mut l = ledger(dim, 2 * needed - 1);
        eval_metered(&mut d, &f, &r, ClosureLimits::default(), &mut l).unwrap();
        assert_eq!(l.remaining(dim), needed - 1);
        let e = eval_metered(&mut d, &f, &r, ClosureLimits::default(), &mut l).unwrap_err();
        assert!(matches!(e, MeteredError::Ledger(e) if e.dim == dim));
        assert_eq!(l.spent(dim), needed);
        // Enough for both: both charges are real and add up exactly.
        let mut l = ledger(dim, 2 * needed);
        for _ in 0..2 {
            eval_metered(&mut d, &f, &r, ClosureLimits::default(), &mut l).unwrap();
        }
        assert_eq!(l.remaining(dim), 0);
    }
}

#[test]
fn metered_caller_caps_preserve_exact_existing_strings_including_ties() {
    for dim in [Dim::JoinRows, Dim::ClosureFacts] {
        for equal in [false, true] {
            let needed = usage(dim);
            let (mut d, f, r) = fixture();
            let mut limits = ClosureLimits::default();
            match dim {
                Dim::JoinRows => limits.max_total_join_rows = needed - 1,
                Dim::ClosureFacts => limits.max_facts = needed - 1,
                _ => unreachable!(),
            }
            let exact = eval_charged(&mut d, &f, &r, limits).unwrap_err();
            let mut l = ledger(dim, if equal { needed - 1 } else { needed + 1 });
            let e = eval_metered(&mut d, &f, &r, limits, &mut l).unwrap_err();
            assert!(matches!(e, MeteredError::Closure(e) if e == exact));
            assert_eq!(l.spent(Dim::JoinRows), 0);
            assert_eq!(l.spent(Dim::ClosureFacts), 0);
        }
    }
}

#[test]
fn metered_unbounded_counts_and_matches_default_closure() {
    let (mut d, f, r) = fixture();
    let (mut expected, uncounted) = eval_charged(&mut d, &f, &r, ClosureLimits::default()).unwrap();
    assert_eq!(uncounted.join_rows, None);
    let mut l = Ledger::new(Limits::default());
    let (mut out, c) = eval_metered(&mut d, &f, &r, ClosureLimits::default(), &mut l).unwrap();
    out.sort_unstable();
    expected.sort_unstable();
    assert_eq!(out, expected);
    assert!(c.join_rows.is_some_and(|n| n > 0));
    assert_eq!(l.spent(Dim::JoinRows), usage(Dim::JoinRows));
    assert_eq!(l.spent(Dim::ClosureFacts), usage(Dim::ClosureFacts));
}

#[test]
fn metered_other_cuts_are_closure_errors() {
    let (mut d, f, r) = fixture();
    for limits in [
        {
            let mut l = ClosureLimits::default();
            l.max_rounds = 0;
            l
        },
        {
            let mut l = ClosureLimits::default();
            l.max_join_rows = 0;
            l
        },
    ] {
        let exact = eval_charged(&mut d, &f, &r, limits).unwrap_err();
        let e =
            eval_metered(&mut d, &f, &r, limits, &mut Ledger::new(Limits::default())).unwrap_err();
        assert!(matches!(e, MeteredError::Closure(e) if e == exact));
    }
    // A caller's round refusal includes both limits in its message. A ledger may
    // narrow facts without being the cut, so the caller's original values must render.
    let mut limits = ClosureLimits::default();
    limits.max_rounds = 0;
    let exact = eval_charged(&mut d, &f, &r, limits).unwrap_err();
    let mut l = ledger(Dim::ClosureFacts, f.len());
    let e = eval_metered(&mut d, &f, &r, limits, &mut l).unwrap_err();
    assert!(matches!(e, MeteredError::Closure(e) if e == exact));
    let r = compile("@prefix : <urn:> . @prefix string: <http://www.w3.org/2000/10/swap/string#> . { ?x :p ?y . (\"hello\" \"world\") string:concatenation ?s } => { ?x :out ?s } .").unwrap();
    let mut limits = ClosureLimits::default();
    limits.max_term_bytes = 1;
    let exact = eval_charged(&mut d, &f, &r, limits).unwrap_err();
    let e = eval_metered(&mut d, &f, &r, limits, &mut Ledger::new(Limits::default())).unwrap_err();
    assert!(matches!(e, MeteredError::Closure(e) if e == exact));
}
