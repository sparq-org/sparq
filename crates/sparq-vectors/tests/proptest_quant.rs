//! Property tests for the categorical codebook's lossless member algebra.

use std::collections::BTreeSet;

use oxrdf::{Literal, NamedNode, Term};
use proptest::collection::btree_set;
use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};
use sparq_vectors::{Codebook, INVALID_SLOT};

// sq-lyf31: retain the term kind in the generated identity so the
// corpus exercises both RDF IRIs and literals while remaining distinct.
fn term(kind: bool, id: u16) -> Term {
    if kind {
        Term::NamedNode(NamedNode::new_unchecked(format!(
            "https://example.test/member/{id}"
        )))
    } else {
        Term::Literal(Literal::new_simple_literal(format!("member-{id}")))
    }
}

fn member_sets() -> impl Strategy<Value = (BTreeSet<u16>, BTreeSet<u16>)> {
    (
        btree_set(any::<u16>(), 1..12),
        btree_set(any::<u16>(), 1..12),
    )
}

#[test]
fn codebook_member_algebra_holds_for_distinct_rdf_terms() {
    let config = Config {
        cases: 256,
        ..Config::default()
    };
    let rng = TestRng::from_seed(RngAlgorithm::ChaCha, &[0x56; 32]);
    let mut runner = TestRunner::new_with_rng(config, rng);

    runner
        .run(&member_sets(), |(iri_ids, literal_ids)| {
            let members: Vec<Term> = iri_ids
                .into_iter()
                .map(|id| term(true, id))
                .chain(literal_ids.into_iter().map(|id| term(false, id)))
                .collect();
            let codebook = Codebook::new(members.clone());

            prop_assert_eq!(codebook.member_count(), members.len());
            prop_assert_eq!(codebook.dim(), members.len() + 1);

            let mut slots = BTreeSet::new();
            for member in &members {
                prop_assert!(codebook.is_member(member));
                let slot = codebook.slot(member);
                prop_assert!((1..codebook.dim()).contains(&slot));
                prop_assert!(slots.insert(slot), "distinct members shared slot {slot}");
                prop_assert_eq!(codebook.slot(member), slot, "slot was not stable");

                let encoded = codebook.encode(member);
                prop_assert_eq!(encoded.len(), codebook.dim());
                prop_assert_eq!(codebook.decode(&encoded), Some(member.clone()));

                let mut into = vec![f32::NAN; codebook.dim()];
                codebook.encode_into(member, &mut into).unwrap();
                prop_assert_eq!(
                    into.iter().map(|value| value.to_bits()).collect::<Vec<_>>(),
                    encoded
                        .iter()
                        .map(|value| value.to_bits())
                        .collect::<Vec<_>>()
                );
            }
            prop_assert_eq!(slots.len(), codebook.member_count());

            let outsider = Term::NamedNode(NamedNode::new_unchecked(
                "https://example.test/definitely-not-a-member",
            ));
            prop_assert!(!codebook.is_member(&outsider));
            prop_assert_eq!(codebook.slot(&outsider), INVALID_SLOT);
            let invalid_block = codebook.encode(&outsider);
            prop_assert!(members
                .iter()
                .all(|member| codebook.encode(member) != invalid_block));
            prop_assert_eq!(codebook.decode(&invalid_block), None);
            prop_assert_eq!(codebook.decode(&vec![0.0; codebook.dim()]), None);
            Ok(())
        })
        .expect("deterministically generated codebook must satisfy its algebra");
}

#[test]
fn deterministic_generator_is_non_vacuous() {
    let rng = TestRng::from_seed(RngAlgorithm::ChaCha, &[0x56; 32]);
    let mut runner = TestRunner::new_with_rng(Config::default(), rng);
    let (iri_ids, literal_ids) = member_sets().new_tree(&mut runner).unwrap().current();

    assert!(!iri_ids.is_empty(), "IRI member path must be exercised");
    assert!(
        !literal_ids.is_empty(),
        "literal member path must be exercised"
    );
    assert!(
        iri_ids.len() + literal_ids.len() >= 2,
        "slot injectivity needs two members"
    );
}
