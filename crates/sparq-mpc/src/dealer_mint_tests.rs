//! sq-yyro / issue #6117 — how many dealers ONE protocol run mints.
//!
//! Every public hidden-regime entry point documents that **one** fresh dealer
//! covers the whole run: the operator's own stage AND the oblivious output stage
//! draw from it in sequence. Since #6117 routed the output stages through the
//! caller's source, a regression that re-mints a second dealer for the output
//! stage is invisible to the differential and transcript suites whenever the
//! entry point canonicalises its output (the two `PartialResult`-returning
//! twins sort the disclosed rows, so a changed shuffle order does not move any
//! assertion). The [`crate::shamir::dealer_mints`] tally closes that hole
//! directly: it counts `ShamirBackend::dealer()` calls on this thread.
//!
//! The mirror assertion is that every `*_through` entry mints NOTHING — it must
//! consume the caller's source and never reach for the backend's own dealer.

use oxrdf::{NamedNode, Term, Variable};

use crate::batched::RowBinding;
use crate::field::Fp;
use crate::hidden_distinct::{
    distinct_hidden_pairs, distinct_hidden_pairs_oblivious, distinct_hidden_pairs_slots,
    distinct_hidden_pairs_slots_through, SecretEndpointPair,
};
use crate::hidden_path::{
    eval_bounded_path_hidden, eval_bounded_path_hidden_slots,
    eval_bounded_path_hidden_slots_through, eval_exact_k_chain_hidden,
    eval_exact_k_chain_hidden_slots, eval_exact_k_chain_hidden_slots_through, HiddenBoundedPath,
    HiddenEdge, HiddenEdges, HiddenNode, PredicatedEdge, PredicatedEdges,
};
use crate::join::{BatchedHiddenInput, HiddenValueJoin};
use crate::oblivious_join::{
    oblivious_set_output_hidden_keys, oblivious_set_output_hidden_keys_through,
};
use crate::partial::HolderId;
use crate::shamir::dealer_mints;

const P: &str = "http://ex/p";

fn iri(s: &str) -> Term {
    Term::NamedNode(NamedNode::new(s).unwrap())
}

fn node(key: u64, s: &str) -> HiddenNode {
    HiddenNode {
        key: Fp::new(key),
        term: iri(s),
    }
}

/// The two-hop chain `a → b → c` (keys 1, 2, 3), so `k = 2` has one true match.
fn chain_edges() -> HiddenEdges {
    HiddenEdges::new(vec![
        HiddenEdge {
            subject: node(1, "http://ex/a"),
            object: node(2, "http://ex/b"),
        },
        HiddenEdge {
            subject: node(2, "http://ex/b"),
            object: node(3, "http://ex/c"),
        },
    ])
}

/// The same chain, labelled with the single predicate `P`.
fn predicated_chain_edges() -> PredicatedEdges {
    PredicatedEdges::new(vec![
        PredicatedEdge {
            predicate: P.to_string(),
            subject: node(1, "http://ex/a"),
            object: node(2, "http://ex/b"),
        },
        PredicatedEdge {
            predicate: P.to_string(),
            subject: node(2, "http://ex/b"),
            object: node(3, "http://ex/c"),
        },
    ])
}

/// Three endpoint pairs with one duplicate, so the DISTINCT stage really dedups.
fn distinct_pairs() -> Vec<SecretEndpointPair> {
    let mk = |a: u64, b: u64, at: &str, bt: &str| SecretEndpointPair {
        a_key: Fp::new(a),
        b_key: Fp::new(b),
        a_term: iri(at),
        b_term: iri(bt),
    };
    vec![
        mk(1, 2, "http://ex/a", "http://ex/b"),
        mk(5, 6, "http://ex/e", "http://ex/f"),
        mk(1, 2, "http://ex/a", "http://ex/b"),
    ]
}

fn batch(holder: &str, var: &str, rows: &[(u64, &str)]) -> BatchedHiddenInput {
    BatchedHiddenInput::new(
        HolderId::new(holder),
        vec![Variable::new_unchecked(var)],
        rows.iter()
            .map(|(k, p)| {
                (
                    Fp::new(*k),
                    vec![Some(Term::Literal(oxrdf::Literal::new_simple_literal(*p)))],
                )
            })
            .collect(),
        RowBinding::Positional,
    )
    .unwrap()
}

/// `reset()` the tally, run `f`, and return the mints it caused. The backend and
/// every fixture are built BEFORE the reset, so only `f`'s own mints are counted.
fn mints_during(f: impl FnOnce()) -> usize {
    dealer_mints::reset();
    f();
    dealer_mints::count()
}

/// Each public entry point mints EXACTLY one dealer per protocol run: one
/// continuous source covering the operator's stage and the oblivious output
/// stage. Two mints means the output stage restarted on a second keystream.
#[test]
fn every_public_entry_mints_exactly_one_dealer_per_run() {
    let mut wrong: Vec<String> = Vec::new();
    let mut check = |name: &str, f: &mut dyn FnMut()| {
        let got = mints_during(&mut *f);
        if got != 1 {
            wrong.push(format!("{}: {} mints", name, got));
        }
    };

    let b5 = crate::shamir::ShamirBackend::new_seeded(5, 0xBA7C1).unwrap();
    let join = HiddenValueJoin::new(b5.clone());
    let left = batch("L", "l", &[(5, "l0"), (7, "l1"), (9, "l2")]);
    let right = batch("R", "r", &[(5, "r0"), (8, "r1"), (9, "r2")]);
    check("batched_join", &mut || {
        join.batched_join(&left, &right, 3).unwrap();
    });

    let b3 = crate::shamir::ShamirBackend::new_seeded(3, 0x9A7C).unwrap();
    let edges = chain_edges();
    check("eval_exact_k_chain_hidden", &mut || {
        eval_exact_k_chain_hidden(&b3, &edges, 2, 4).unwrap();
    });
    check("eval_exact_k_chain_hidden_slots", &mut || {
        eval_exact_k_chain_hidden_slots(&b3, &edges, 2, 4).unwrap();
    });

    let pedges = predicated_chain_edges();
    let form = HiddenBoundedPath::range(P, 1, 2);
    check("eval_bounded_path_hidden", &mut || {
        eval_bounded_path_hidden(&b3, &pedges, &form, 8).unwrap();
    });
    check("eval_bounded_path_hidden_slots", &mut || {
        eval_bounded_path_hidden_slots(&b3, &pedges, &form, 8).unwrap();
    });

    let pairs = distinct_pairs();
    check("distinct_hidden_pairs", &mut || {
        distinct_hidden_pairs(&b3, &pairs, 4).unwrap();
    });
    check("distinct_hidden_pairs_slots", &mut || {
        distinct_hidden_pairs_slots(&b3, &pairs, 4).unwrap();
    });
    check("distinct_hidden_pairs_oblivious", &mut || {
        distinct_hidden_pairs_oblivious(&b3, &pairs, 4).unwrap();
    });

    let lk = [Fp::new(1), Fp::new(2)];
    let rk = [Fp::new(2), Fp::new(3)];
    check("oblivious_set_output_hidden_keys", &mut || {
        oblivious_set_output_hidden_keys(&b3, &lk, &rk, 4).unwrap();
    });

    assert!(wrong.is_empty(), "not one dealer per run: {:?}", wrong);
}

/// The mirror: a `*_through` entry mints NOTHING. It must consume the caller's
/// source end to end — if any stage reaches for `backend.dealer()` instead, the
/// caller's keystream is no longer the one the protocol ran on.
#[test]
fn every_through_entry_mints_no_dealer() {
    let mut wrong: Vec<String> = Vec::new();

    let b5 = crate::shamir::ShamirBackend::new_seeded(5, 0xBA7C1).unwrap();
    let join = HiddenValueJoin::new(b5.clone());
    let left = batch("L", "l", &[(5, "l0"), (7, "l1"), (9, "l2")]);
    let right = batch("R", "r", &[(5, "r0"), (8, "r1"), (9, "r2")]);
    let mut src = b5.dealer();
    let got = mints_during(|| {
        join.batched_join_through(&mut src, &left, &right, 3)
            .unwrap();
    });
    if got != 0 {
        wrong.push(format!("batched_join_through: {} mints", got));
    }

    let b3 = crate::shamir::ShamirBackend::new_seeded(3, 0x9A7C).unwrap();
    let edges = chain_edges();
    let mut src = b3.dealer();
    let got = mints_during(|| {
        eval_exact_k_chain_hidden_slots_through(
            &mut src,
            &b3,
            &edges,
            2,
            crate::oblivious_join::PayloadWidths::derived(2),
            4,
        )
        .unwrap();
    });
    if got != 0 {
        wrong.push(format!(
            "eval_exact_k_chain_hidden_slots_through: {} mints",
            got
        ));
    }

    let pedges = predicated_chain_edges();
    let form = HiddenBoundedPath::range(P, 1, 2);
    let mut src = b3.dealer();
    let got = mints_during(|| {
        eval_bounded_path_hidden_slots_through(
            &mut src,
            &b3,
            &pedges,
            &form,
            crate::oblivious_join::PayloadWidths::derived(2),
            8,
        )
        .unwrap();
    });
    if got != 0 {
        wrong.push(format!(
            "eval_bounded_path_hidden_slots_through: {} mints",
            got
        ));
    }

    let pairs = distinct_pairs();
    let mut src = b3.dealer();
    let got = mints_during(|| {
        distinct_hidden_pairs_slots_through(
            &mut src,
            &b3,
            &pairs,
            crate::oblivious_join::PayloadWidths::derived(2),
            4,
        )
        .unwrap();
    });
    if got != 0 {
        wrong.push(format!(
            "distinct_hidden_pairs_slots_through: {} mints",
            got
        ));
    }

    let lk = [Fp::new(1), Fp::new(2)];
    let rk = [Fp::new(2), Fp::new(3)];
    let mut src = b3.dealer();
    let got = mints_during(|| {
        oblivious_set_output_hidden_keys_through(&mut src, &b3, &lk, &rk, 4).unwrap();
    });
    if got != 0 {
        wrong.push(format!(
            "oblivious_set_output_hidden_keys_through: {} mints",
            got
        ));
    }

    assert!(wrong.is_empty(), "a _through entry minted: {:?}", wrong);
}

/// The tally itself: it counts every mint on this thread, and nothing else.
/// Without this the two tests above could both pass on a counter stuck at its
/// expected value.
#[test]
fn the_tally_counts_exactly_the_mints_on_this_thread() {
    let b = crate::shamir::ShamirBackend::new_seeded(3, 1).unwrap();
    dealer_mints::reset();
    assert_eq!(dealer_mints::count(), 0);
    // Building a backend is not a mint.
    let _other = crate::shamir::ShamirBackend::new_seeded(5, 2).unwrap();
    assert_eq!(dealer_mints::count(), 0);
    let _d1 = b.dealer();
    assert_eq!(dealer_mints::count(), 1);
    let _d2 = b.dealer();
    let _d3 = _other.dealer();
    assert_eq!(dealer_mints::count(), 3);
    dealer_mints::reset();
    assert_eq!(dealer_mints::count(), 0);
}
