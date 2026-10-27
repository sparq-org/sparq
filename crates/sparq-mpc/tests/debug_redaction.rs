//! A secret must not reach a `Debug` rendering, from any container that owns one.
//!
//! The types in this crate are nested: a `Share` sits inside a `Message`, a
//! `MatchBit`, a `Candidate`; a private join key sits inside `HiddenNode`,
//! `BatchedHiddenInput`, `SecretEndpointPair`. A derived `Debug` on any one of them
//! prints the secret from every container above it, so one stray `{:?}` in a log
//! line, a panic message or an `assert_eq!` failure carries share values and join
//! keys with it. The owning types therefore hand-write `Debug`.
//!
//! This suite puts a SENTINEL secret into each type, formats it with both `{:?}`
//! and `{:#?}`, directly and inside every container that holds it, and asserts the
//! sentinel never appears. It uses only the public API and draws no randomness, so
//! it runs in BOTH feature states. The dealer-built authenticated sharings are
//! covered in-crate, where a `MacSession` is reachable.

use oxrdf::{Literal, NamedNode, Term, Variable};
use sparq_mpc::field::Fp;
use sparq_mpc::hidden_distinct::SecretEndpointPair;
use sparq_mpc::hidden_path::{
    HiddenEdge, HiddenEdges, HiddenNode, PredicatedEdge, PredicatedEdges,
};
use sparq_mpc::join::BatchedHiddenInput;
use sparq_mpc::oblivious_join::{Candidate, MatchBit};
use sparq_mpc::partial::HolderId;
use sparq_mpc::shamir::Share;
use sparq_mpc::term_encode::KeyEncoder;
use sparq_mpc::transport::Message;
use sparq_mpc::{BatchedShares, RowBinding};

/// A field-element sentinel: large and arbitrary, so it cannot be confused with a
/// length, a party index, a column count or any other public quantity a rendering
/// legitimately carries.
const SECRET_VALUE: u64 = 987_654_321_987;

/// A term sentinel, for the plaintext payload / endpoint-term fields.
const SECRET_TEXT: &str = "SENTINEL-PAYLOAD-24681357";

fn secret_fp() -> Fp {
    Fp::new(SECRET_VALUE)
}

fn secret_term() -> Term {
    Term::Literal(Literal::new_simple_literal(SECRET_TEXT))
}

fn secret_iri() -> Term {
    Term::NamedNode(NamedNode::new(format!("http://ex/{}", SECRET_TEXT)).unwrap())
}

fn secret_share() -> Share {
    Share {
        x: 1,
        y: secret_fp(),
    }
}

fn secret_sharing() -> Vec<Share> {
    (1..=3u64).map(|x| Share { x, y: secret_fp() }).collect()
}

fn secret_node() -> HiddenNode {
    HiddenNode {
        key: secret_fp(),
        term: secret_term(),
    }
}

/// Assert that neither sentinel survives either `Debug` format of `value`.
///
/// Both formats are checked because `{:#?}` is a different code path through
/// `Formatter`: a hand-written impl that forgets `alternate` handling, or a field
/// that only the pretty printer expands, would show up here and not in `{:?}`.
fn assert_redacted<T: std::fmt::Debug>(what: &str, value: &T) {
    for (how, rendered) in [
        ("{:?}", format!("{:?}", value)),
        ("{:#?}", format!("{:#?}", value)),
    ] {
        assert!(
            !rendered.contains(&SECRET_VALUE.to_string()),
            "{} printed the field-element sentinel through {}: {}",
            what,
            how,
            rendered
        );
        assert!(
            !rendered.contains(SECRET_TEXT),
            "{} printed the term sentinel through {}: {}",
            what,
            how,
            rendered
        );
    }
}

#[test]
fn a_share_redacts_its_value_directly_and_in_every_container() {
    // The owning type.
    assert_redacted("Share", &secret_share());
    assert_redacted("Vec<Share>", &secret_sharing());

    // The public shape is still there — the redaction is not a blanket refusal to
    // print, which would make the impl useless for diagnostics.
    let rendered = format!("{:?}", secret_share());
    assert!(
        rendered.contains("Share") && rendered.contains("x: 1"),
        "the evaluation point is public and must still print: {}",
        rendered
    );

    // Every container the brief lists that holds a share.
    assert_redacted("Message::Shares", &Message::Shares(secret_sharing()));
    assert_redacted("Message::Open", &Message::Open(secret_sharing()));
    assert_redacted(
        "MatchBit::SecretShared",
        &MatchBit::SecretShared(secret_sharing()),
    );
    assert_redacted(
        "Candidate",
        &Candidate {
            payload: vec![Some(secret_term())],
            matched: MatchBit::SecretShared(secret_sharing()),
        },
    );
    assert_redacted(
        "Vec<Candidate>",
        &vec![Candidate {
            payload: vec![Some(secret_term()), None],
            matched: MatchBit::SecretShared(secret_sharing()),
        }],
    );
    assert_redacted(
        "BatchedShares",
        &BatchedShares::new(
            vec![secret_sharing(), secret_sharing()],
            RowBinding::Positional,
        )
        .unwrap(),
    );
}

#[test]
fn a_candidate_redacts_its_payload_but_keeps_the_public_arity() {
    let candidate = Candidate {
        payload: vec![Some(secret_term()), None, Some(secret_iri())],
        matched: MatchBit::Public(true),
    };
    assert_redacted("Candidate", &candidate);

    let rendered = format!("{:?}", candidate);
    assert!(
        rendered.contains("columns: 3"),
        "the payload arity is public and must still print: {}",
        rendered
    );
}

#[test]
fn a_private_join_key_is_redacted_directly_and_in_every_container() {
    assert_redacted("HiddenNode", &secret_node());
    assert_redacted(
        "HiddenEdge",
        &HiddenEdge {
            subject: secret_node(),
            object: secret_node(),
        },
    );
    assert_redacted(
        "HiddenEdges",
        &HiddenEdges::new(vec![HiddenEdge {
            subject: secret_node(),
            object: secret_node(),
        }]),
    );
    assert_redacted(
        "PredicatedEdge",
        &PredicatedEdge {
            predicate: "http://ex/p".to_string(),
            subject: secret_node(),
            object: secret_node(),
        },
    );
    assert_redacted(
        "PredicatedEdges",
        &PredicatedEdges::new(vec![PredicatedEdge {
            predicate: "http://ex/p".to_string(),
            subject: secret_node(),
            object: secret_node(),
        }]),
    );
    // A DECLARED node domain holds endpoint terms directly, in a field of its own
    // — including the terms of intermediate nodes, which `HiddenNode`'s `Debug`
    // redacts. A derived `Debug` on `PredicatedEdges` would print exactly what
    // that redaction withholds, so the declared domain has to be redacted too.
    let declared = PredicatedEdges::new(vec![PredicatedEdge {
        predicate: "http://ex/p".to_string(),
        subject: secret_node(),
        object: secret_node(),
    }])
    .with_node_domain(vec![secret_term(), secret_iri()])
    .expect("the declared domain covers the one edge's endpoints");
    assert_redacted("PredicatedEdges (declared node domain)", &declared);
    assert_redacted(
        "SecretEndpointPair",
        &SecretEndpointPair {
            a_key: secret_fp(),
            b_key: secret_fp(),
            a_term: secret_term(),
            b_term: secret_iri(),
        },
    );

    // The predicate is PUBLIC (it is named in the public path expression), so the
    // redaction must not have swallowed it.
    let rendered = format!(
        "{:?}",
        PredicatedEdge {
            predicate: "http://ex/p".to_string(),
            subject: secret_node(),
            object: secret_node(),
        }
    );
    assert!(
        rendered.contains("http://ex/p"),
        "the public predicate must still print: {}",
        rendered
    );

    // `PredicatedEdges`' redaction keeps the two PUBLIC sizes — the edge count and
    // the declared domain's size, which already enters the public candidate bound —
    // and says which of "declared" / "none declared" applies. Without that the
    // rendering would be useless for the thing a `Debug` is for.
    let shown = format!("{:?}", declared);
    assert!(
        shown.contains("edges: 1") && shown.contains("2 term(s)"),
        "the public sizes must still print: {}",
        shown
    );
    let bare = format!(
        "{:?}",
        PredicatedEdges::new(vec![PredicatedEdge {
            predicate: "http://ex/p".to_string(),
            subject: secret_node(),
            object: secret_node(),
        }])
    );
    assert!(
        bare.contains("none declared"),
        "an undeclared domain must say so rather than print a size: {}",
        bare
    );
}

#[test]
fn the_keyed_row_containers_redact_their_keys_and_payloads() {
    let rows = vec![(secret_fp(), vec![Some(secret_term())])];
    let batched = BatchedHiddenInput {
        holder: HolderId::new("h"),
        payload_vars: vec![Variable::new("name").unwrap()],
        rows,
        // A keyed binding carries DISCLOSED row keys, so this one is expected to
        // print — the assertion below pins that, so the redaction cannot quietly
        // grow to cover public data.
        binding: RowBinding::Keyed(vec!["disclosed-row-key".to_string()]),
    };
    assert_redacted("BatchedHiddenInput", &batched);
    let rendered = format!("{:?}", batched);
    assert!(
        rendered.contains('h') && rendered.contains("name") && rendered.contains("row_count: 1"),
        "the holder, the disclosed column names and the row count are public: {}",
        rendered
    );
    assert!(
        format!("{:?}", batched).contains("disclosed-row-key"),
        "a Keyed binding's row keys are disclosed and must still print"
    );
}

#[test]
fn the_key_encoder_redacts_its_whole_key_domain() {
    let mut enc = KeyEncoder::new();
    enc.encode(&secret_term()).expect("no collision");
    enc.encode(&secret_iri()).expect("no collision");
    assert_redacted("KeyEncoder", &enc);

    // Not even the entry count: the map collapses duplicate terms, so the count is
    // the number of DISTINCT keys rather than the row count the join discloses.
    let one = {
        let mut e = KeyEncoder::new();
        e.encode(&secret_term()).unwrap();
        format!("{:?}", e)
    };
    assert_eq!(
        one,
        format!("{:?}", enc),
        "the rendering must not vary with the number of entries"
    );
}
