//! `urn:concept:` verifier suite — the opt-in `concept` feature.
//!
//! The point of a second implementation is that it does not inherit the
//! producer's blind spots, so the positive vectors below are **known answers
//! derived outside this crate** (Python `hashlib` + a hand-written multibase
//! encoder over the canonical N-Quads document this crate emits), not values
//! read back out of the code under test. Flip a byte in either the digest or
//! the record and the suite goes red — asserted directly in
//! `mutating_the_expected_digest_makes_verification_fail`.
//!
//! Scope note: this suite exercises the **envelope + guard**. It asserts
//! nothing about which quads constitute a concept record — that rule belongs to
//! the concept-hash definition (#1683) and its profile freeze (#1746) and is
//! not vendored here; see `research/genai-urn-concept-verifier-design.md`.
#![cfg(feature = "concept")]

use oxrdf::{GraphName, Literal, NamedNode, NamedOrBlankNode, Quad, Term};
use sparq_canon::concept::{
    concept_multihash, concept_urn, verify_concept, verify_concept_urn, ConceptError, ConceptHash,
    ConceptUrn, Multibase,
};
use sparq_canon::{parse_nquads, CanonError};

/// A two-blank-node record whose canonical form is
/// ```text
/// _:c14n0 <http://ex/label> "H²O"@en <http://ex/g> .
/// _:c14n1 <http://ex/label> "water" .
/// _:c14n1 <http://ex/prime> _:c14n0 .
/// ```
/// — 124 bytes including the final newline. Chosen to exercise blank-node
/// relabelling, a named graph, a language-tagged literal and a non-ASCII
/// character all at once.
const RECORD_NQ: &str = concat!(
    "_:b0 <http://ex/label> \"water\" .\n",
    "_:b0 <http://ex/prime> _:b1 .\n",
    "_:b1 <http://ex/label> \"H\u{b2}O\"@en <http://ex/g> .\n",
);

/// SHA-256 of that canonical document, as multibase-multihash, computed
/// independently of this crate.
const URN_SHA256_BASE16: &str =
    "urn:concept:f12206ff69f76d733563c99f833c20c396155b91599bdcca39afcd05ed59d5e121d5f";
const URN_SHA256_BASE32: &str =
    "urn:concept:bciqg75u7o3ltgvr4th4dhqqmhfqvloivtg64zi427tif5vm5lyjb2xy";
const URN_SHA256_BASE58: &str = "urn:concept:zQmVsi5WTUK8PmdA9WEn1XRDPq2EhM4oQe3pU2rYMMfNiqL";
const URN_SHA512_BASE32: &str = concat!(
    "urn:concept:bcnapexzxt637c5frwgkpf4lqawte32fn54wrrpm7o7uytl5chpeo4bge2dp5s",
    "ib23gzdt2i6huod7k6nqutiqevf2kjazn6iehwo4jb2ce"
);

fn record() -> Vec<Quad> {
    parse_nquads(RECORD_NQ).unwrap()
}

/// The same record with its blank nodes relabelled and its quads reordered —
/// RDF-isomorphic, so it must carry the same concept name.
fn isomorphic_record() -> Vec<Quad> {
    parse_nquads(concat!(
        "_:zzz <http://ex/label> \"H\u{b2}O\"@en <http://ex/g> .\n",
        "_:aaa <http://ex/prime> _:zzz .\n",
        "_:aaa <http://ex/label> \"water\" .\n",
    ))
    .unwrap()
}

fn named(iri: &str) -> NamedNode {
    NamedNode::new(iri).unwrap()
}

#[test]
fn minted_urns_match_the_independently_computed_known_answers() {
    let r = record();
    assert_eq!(
        concept_urn(&r, ConceptHash::Sha256, Multibase::Base16Lower).unwrap(),
        URN_SHA256_BASE16
    );
    assert_eq!(
        concept_urn(&r, ConceptHash::Sha256, Multibase::Base32Lower).unwrap(),
        URN_SHA256_BASE32
    );
    assert_eq!(
        concept_urn(&r, ConceptHash::Sha256, Multibase::Base58Btc).unwrap(),
        URN_SHA256_BASE58
    );
    assert_eq!(
        concept_urn(&r, ConceptHash::Sha512, Multibase::Base32Lower).unwrap(),
        URN_SHA512_BASE32
    );
}

#[test]
fn every_alphabet_of_one_name_verifies_the_same_record() {
    let r = record();
    for urn in [URN_SHA256_BASE16, URN_SHA256_BASE32, URN_SHA256_BASE58] {
        verify_concept_urn(urn, &r).unwrap_or_else(|e| panic!("{} rejected: {}", urn, e));
        // The alphabets are three spellings of one multihash.
        assert_eq!(
            ConceptUrn::parse(urn).unwrap().multihash(),
            ConceptUrn::parse(URN_SHA256_BASE16).unwrap().multihash()
        );
    }
    verify_concept_urn(URN_SHA512_BASE32, &r).unwrap();
}

#[test]
fn an_isomorphic_record_verifies_against_the_same_name() {
    // Relabelled blank nodes and reordered quads: the same concept.
    verify_concept_urn(URN_SHA256_BASE58, &isomorphic_record()).unwrap();
    assert_eq!(
        concept_urn(&isomorphic_record(), ConceptHash::Sha256, Multibase::Base58Btc).unwrap(),
        URN_SHA256_BASE58
    );
}

/// The mutation check: a one-byte edit to the expected digest must turn the
/// positive vector red. A verifier whose tests pass against a broken comparison
/// is worse than none.
#[test]
fn mutating_the_expected_digest_makes_verification_fail() {
    let mut mh = concept_multihash(&record(), ConceptHash::Sha256).unwrap();
    assert_eq!(mh.len(), 34);
    for i in 0..mh.len() {
        let original = mh[i];
        mh[i] ^= 0x01;
        let urn = format!("urn:concept:{}", hex_multibase(&mh));
        assert!(
            verify_concept_urn(&urn, &record()).is_err(),
            "flipping byte {} of the multihash still verified",
            i
        );
        mh[i] = original;
    }
    // …and unflipped it still passes, so the loop above is not vacuously red.
    let urn = format!("urn:concept:{}", hex_multibase(&mh));
    verify_concept_urn(&urn, &record()).unwrap();
    assert_eq!(urn, URN_SHA256_BASE16);
}

/// Base16-lower multibase, written here rather than via `Multibase::encode` so
/// the mutation loop does not depend on the encoder it is testing through.
fn hex_multibase(bytes: &[u8]) -> String {
    let mut s = String::from("f");
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

#[test]
fn a_substituted_record_is_rejected() {
    // Same shape, one literal changed.
    let tampered = parse_nquads(concat!(
        "_:b0 <http://ex/label> \"water\" .\n",
        "_:b0 <http://ex/prime> _:b1 .\n",
        "_:b1 <http://ex/label> \"D\u{b2}O\"@en <http://ex/g> .\n",
    ))
    .unwrap();
    assert!(matches!(
        verify_concept_urn(URN_SHA256_BASE58, &tampered),
        Err(ConceptError::Mismatch)
    ));

    // A record that differs only by the graph name — canonical N-Quads carries
    // the graph term, so this is a different concept.
    let regraphed = parse_nquads(concat!(
        "_:b0 <http://ex/label> \"water\" .\n",
        "_:b0 <http://ex/prime> _:b1 .\n",
        "_:b1 <http://ex/label> \"H\u{b2}O\"@en <http://ex/other> .\n",
    ))
    .unwrap();
    assert!(matches!(
        verify_concept_urn(URN_SHA256_BASE58, &regraphed),
        Err(ConceptError::Mismatch)
    ));

    // A superset of the record (the classic "index more than was named" bug).
    let mut extra = record();
    extra.push(Quad::new(
        NamedOrBlankNode::NamedNode(named("http://ex/s")),
        named("http://ex/p"),
        Term::Literal(Literal::new_simple_literal("smuggled")),
        GraphName::DefaultGraph,
    ));
    assert!(matches!(
        verify_concept_urn(URN_SHA256_BASE58, &extra),
        Err(ConceptError::Mismatch)
    ));
}

#[test]
fn a_language_tag_differing_only_in_case_yields_the_same_name() {
    // BCP 47 tags are case-insensitive, and oxrdf normalizes them to lower case
    // when a literal is constructed — so `@EN` and `@en` are the same term and
    // the same concept. Pinned because the alternative (a byte-wise split on
    // tag case) would silently fork one concept into two names.
    let upper_tag = parse_nquads(concat!(
        "_:b0 <http://ex/label> \"water\" .\n",
        "_:b0 <http://ex/prime> _:b1 .\n",
        "_:b1 <http://ex/label> \"H\u{b2}O\"@EN <http://ex/g> .\n",
    ))
    .unwrap();
    assert_eq!(
        concept_urn(&upper_tag, ConceptHash::Sha256, Multibase::Base58Btc).unwrap(),
        URN_SHA256_BASE58
    );
    verify_concept_urn(URN_SHA256_BASE58, &upper_tag).unwrap();
}

#[test]
fn every_envelope_malformation_fails_closed() {
    let r = record();
    let cases: [(&str, &str); 8] = [
        ("http://ex/not-a-urn", "wrong scheme"),
        ("urn:other:z1220ff", "wrong namespace identifier"),
        ("urn:concept:", "empty name-specific string"),
        (
            "urn:concept:Q12206ff69f76d733563c99f833c20c396155b91599bdcca39afcd05ed59d5e121d5f",
            "unknown multibase prefix",
        ),
        ("urn:concept:f1220zz", "character outside the alphabet"),
        ("urn:concept:f1220ff", "declared length exceeds the digest present"),
        (
            "urn:concept:f12206ff69f76d733563c99f833c20c396155b91599bdcca39afcd05ed59d5e121d5fff",
            "trailing bytes after the digest",
        ),
        ("urn:concept:f1200", "zero-length digest"),
    ];
    for (urn, why) in cases {
        assert!(
            verify_concept_urn(urn, &r).is_err(),
            "{} was accepted ({})",
            urn,
            why
        );
        assert!(ConceptUrn::parse(urn).is_err(), "{} parsed ({})", urn, why);
    }
}

/// An oversized name-specific string is refused on its length, before the
/// alphabet is decoded at all. base58btc decoding is quadratic in the body
/// length, so a guard that only rejected the eventual malformed multihash would
/// already have done the work an ingestion-time attacker was buying. The
/// assertion is on the length rejection specifically: an oversized body errors
/// either way, and only the *reason* distinguishes early refusal from late.
#[test]
fn an_oversized_name_is_refused_on_length_not_after_decoding() {
    let r = record();
    // Every character is valid in the declared alphabet, so nothing but the
    // length bound can stop either of these.
    let hostile = [
        format!("urn:concept:z{}", "Q".repeat(50_000)),
        format!("urn:concept:f{}", "ab".repeat(50_000)),
    ];
    for urn in &hostile {
        let err = ConceptUrn::parse(urn).unwrap_err();
        assert!(
            matches!(&err, ConceptError::Multibase(m) if m.contains("rejected before decoding")),
            "oversized name was not refused on length: {:?}",
            err
        );
        assert!(verify_concept_urn(urn, &r).is_err());
    }
    // The longest legitimate name — a SHA-512 multihash written in base16, the
    // widest digest in the least dense alphabet — is unaffected.
    let widest = format!("urn:concept:f1340{}", "ab".repeat(64));
    assert!(!matches!(
        ConceptUrn::parse(&widest),
        Err(ConceptError::Multibase(_))
    ));
}

#[test]
fn an_unsupported_hash_code_is_a_rejection_not_a_pass() {
    // sha1 (0x11), structurally well-formed at its declared 20 bytes.
    let urn = format!("urn:concept:f1114{}", "07".repeat(20));
    let urn = urn.as_str();
    let parsed = ConceptUrn::parse(urn).unwrap();
    assert_eq!(parsed.hash_code(), 0x11);
    assert!(matches!(
        verify_concept(&parsed, &record()),
        Err(ConceptError::UnsupportedHash(0x11))
    ));
    assert!(matches!(
        verify_concept_urn(urn, &record()),
        Err(ConceptError::UnsupportedHash(0x11))
    ));
}

#[test]
fn a_hash_code_can_be_pinned_before_verifying() {
    // A caller that will only accept SHA-256 checks the declared code first;
    // `hash_code()` is public precisely so the policy stays with the caller.
    let parsed = ConceptUrn::parse(URN_SHA512_BASE32).unwrap();
    assert_ne!(parsed.hash_code(), ConceptHash::Sha256.code());
    assert!(verify_concept(&parsed, &record()).is_ok());
}

#[test]
fn an_empty_record_is_rejected_rather_than_hashed() {
    assert!(matches!(
        verify_concept_urn(URN_SHA256_BASE58, &[]),
        Err(ConceptError::EmptyRecord)
    ));
    assert!(matches!(
        concept_urn(&[], ConceptHash::Sha256, Multibase::Base58Btc),
        Err(ConceptError::EmptyRecord)
    ));
}

#[test]
fn rdf12_triple_terms_in_a_record_fail_closed() {
    let inner = oxrdf::Triple::new(
        named("http://ex/s"),
        named("http://ex/p"),
        Term::Literal(Literal::new_simple_literal("v")),
    );
    let with_tt = vec![Quad::new(
        NamedOrBlankNode::NamedNode(named("http://ex/c")),
        named("http://ex/says"),
        Term::Triple(Box::new(inner)),
        GraphName::DefaultGraph,
    )];
    assert!(matches!(
        concept_urn(&with_tt, ConceptHash::Sha256, Multibase::Base58Btc),
        Err(ConceptError::Canon(CanonError::TripleTerm))
    ));
    assert!(matches!(
        verify_concept_urn(URN_SHA256_BASE58, &with_tt),
        Err(ConceptError::Canon(CanonError::TripleTerm))
    ));
}

#[test]
fn parsed_names_round_trip_through_every_alphabet() {
    let mh = ConceptUrn::parse(URN_SHA256_BASE58).unwrap();
    for base in [
        Multibase::Base16Lower,
        Multibase::Base16Upper,
        Multibase::Base32Lower,
        Multibase::Base32Upper,
        Multibase::Base58Btc,
        Multibase::Base64Url,
    ] {
        let urn = concept_urn(&record(), ConceptHash::Sha256, base).unwrap();
        let parsed = ConceptUrn::parse(&urn).unwrap();
        assert_eq!(parsed.multibase(), base);
        assert_eq!(parsed.multihash(), mh.multihash());
        assert_eq!(parsed.to_urn(), urn);
        verify_concept(&parsed, &record()).unwrap();
    }
}
