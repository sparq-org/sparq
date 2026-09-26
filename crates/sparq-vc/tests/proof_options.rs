//! Public-API tests for the typed proof-option validation seam (zkp-14.3).
//!
//! [OPUS-5.5] Every invalid option must be rejected identically by
//! [`ProofConfig::validate`], [`sign`], [`sign_graph`], [`verify`] and
//! [`verify_graph`], before canonicalization and before the `verificationMethod`
//! resolver is consulted. Valid options must round-trip verbatim. The compact
//! `proofPurpose` terms are checked against the VC v2 `@context` `@id`s written
//! out by hand below, not against the production lookup.
//!
//! This is lexical validation only; nothing here checks key authorization,
//! freshness, audience, or credential status.

use std::cell::Cell;
use std::collections::HashSet;

use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use proptest::prelude::*;
use sparq_core::Graph;
use sparq_vc::did::{Did, DidError, DidKeyResolver, DidResolver};
use sparq_vc::{
    DataIntegrityProof, ProofConfig, ProofOptionError, SUPPORTED_PURPOSE_TERMS, SigningKey,
    VcError, VerifiedProof, VerifyingKey, sign, sign_graph, verify, verify_graph,
};

/// XSD 1.1 `xsd:dateTime` values at the edges of the lexical space.
const VALID_CREATED: &[&str] = &[
    "2023-02-24T23:36:38Z",
    "0000-01-01T00:00:00Z",
    "0000-02-29T12:00:00",
    "-0000-06-15T00:00:00Z",
    "-0001-12-31T23:59:59Z",
    "-0004-02-29T00:00:00+14:00",
    "-0400-02-29T00:00:00Z",
    "2000-02-29T00:00:00Z",
    "2024-02-29T00:00:00-14:00",
    "12345-01-01T00:00:00Z",
    "2023-12-31T24:00:00Z",
    "2023-12-31T24:00:00.000Z",
    "2023-01-01T00:00:00.123456789012345678901234567890Z",
    "2023-01-01T00:00:00",
    "2023-01-01T00:00:00+13:59",
    "2023-01-01T00:00:00-00:00",
];

/// Values outside the XSD 1.1 `xsd:dateTime` lexical space.
const INVALID_CREATED: &[&str] = &[
    "",
    "2023-02-29T00:00:00Z",
    "1900-02-29T00:00:00Z",
    "-0001-02-29T00:00:00Z",
    "-0100-02-29T00:00:00Z",
    "2023-04-31T00:00:00Z",
    "2023-13-01T00:00:00Z",
    "023-01-01T00:00:00Z",
    "02023-01-01T00:00:00Z",
    "+2023-01-01T00:00:00Z",
    "2023-01-01",
    "2023-01-01T24:00:01Z",
    "2023-01-01T24:00:00.5Z",
    "2023-01-01T00:00:60Z",
    "2023-01-01T00:00:00.Z",
    "2023-01-01T00:00:00+14:01",
    "2023-01-01T00:00:00+1400",
    " 2023-01-01T00:00:00Z",
    "2023-01-01T00:00:00Z ",
    "\u{ff12}\u{ff10}\u{ff12}\u{ff13}-01-01T00:00:00Z",
];

/// `verificationMethod` values that are not absolute IRIs.
const INVALID_VERIFICATION_METHODS: &[&str] = &[
    "",
    "#key",
    "relative/key",
    "z6MkrJVnaZkeFzdQyMZu1cgjg7k1pZZ6pvBQ7XJPt4swbTQ2",
    "did key",
    "https://exa mple.test/k",
];

/// `proofPurpose` values that are neither a supported compact term nor an
/// absolute IRI.
const INVALID_PURPOSES: &[&str] = &[
    "",
    "foo",
    "assertionmethod",
    "AssertionMethod",
    "assertionMethod ",
    " assertionMethod",
    // An expanded `@id`'s local name is not itself a compact term.
    "authenticationMethod",
    "https://exa mple.test/p",
];

/// The `proofPurpose` scoped-context `@id`s of <https://www.w3.org/ns/credentials/v2>,
/// written out by hand as an oracle independent of the production lookup.
const W3C_V2_PURPOSE_IDS: [(&str, &str); 5] = [
    (
        "assertionMethod",
        "https://w3id.org/security#assertionMethod",
    ),
    (
        "authentication",
        "https://w3id.org/security#authenticationMethod",
    ),
    (
        "capabilityDelegation",
        "https://w3id.org/security#capabilityDelegationMethod",
    ),
    (
        "capabilityInvocation",
        "https://w3id.org/security#capabilityInvocationMethod",
    ),
    (
        "keyAgreement",
        "https://w3id.org/security#keyAgreementMethod",
    ),
];

/// The same single triple as [`triples`], as N-Triples for the store path.
const NT: &str = "<https://example.test/s> <https://example.test/p> \"o\" .\n";

fn key() -> SigningKey {
    SigningKey::from_seed(&[31_u8; 32])
}

fn vm_for(key: &SigningKey) -> String {
    let did = key.did_key();
    format!("{did}#{}", did.strip_prefix("did:key:").unwrap())
}

fn triples() -> Vec<Triple> {
    vec![Triple::new(
        NamedOrBlankNode::NamedNode(NamedNode::new_unchecked("https://example.test/s")),
        NamedNode::new_unchecked("https://example.test/p"),
        Term::Literal(Literal::new_simple_literal("o")),
    )]
}

/// A document RDFC-1.0 rejects (an RDF 1.2 triple term), so reaching the
/// canonicalizer is observable as `VcError::Canon`.
fn uncanonicalizable() -> Vec<Triple> {
    let inner = triples().remove(0);
    vec![Triple::new(
        NamedOrBlankNode::NamedNode(NamedNode::new_unchecked("https://example.test/r")),
        NamedNode::new_unchecked("https://example.test/about"),
        Term::Triple(Box::new(inner)),
    )]
}

/// A signer, a non-empty store graph, and a proof whose options are valid.
struct Fixture {
    key: SigningKey,
    graph: Graph,
    good: DataIntegrityProof,
}

fn fixture() -> Fixture {
    let key = key();
    let graph = Graph::load_str(NT, "ntriples").expect("load N-Triples");
    let good = sign(&triples(), &key, &ProofConfig::new(vm_for(&key))).expect("valid options sign");
    Fixture { key, graph, good }
}

/// A `did:key` resolver that counts how often it is consulted.
///
/// `verify` calls `resolve_str`, which for a non-DID IRI fails before `resolve`,
/// so both entry points count.
#[derive(Default)]
struct CountingResolver(Cell<usize>);

impl CountingResolver {
    fn calls(&self) -> usize {
        self.0.get()
    }
}

impl DidResolver for CountingResolver {
    fn resolve(&self, did: &Did) -> Result<VerifyingKey, DidError> {
        self.0.set(self.0.get() + 1);
        DidKeyResolver.resolve(did)
    }

    fn resolve_str(&self, s: &str) -> Result<VerifyingKey, DidError> {
        self.0.set(self.0.get() + 1);
        DidKeyResolver.resolve_str(s)
    }
}

fn invalid_option<T: std::fmt::Debug>(result: Result<T, VcError>) -> ProofOptionError {
    match result {
        Err(VcError::InvalidProofOption(e)) => e,
        other => panic!("expected InvalidProofOption, got {other:?}"),
    }
}

fn did_error<T: std::fmt::Debug>(result: Result<T, VcError>) -> DidError {
    match result {
        Err(VcError::Did(e)) => e,
        other => panic!("expected VcError::Did, got {other:?}"),
    }
}

fn verified(result: Result<VerifiedProof, VcError>) -> VerifiedProof {
    result.expect("proof verifies")
}

fn with_purpose(purpose: &str, base: &ProofConfig) -> ProofConfig {
    ProofConfig {
        proof_purpose: purpose.to_string(),
        ..base.clone()
    }
}

fn with_method(verification_method: &str, base: &ProofConfig) -> ProofConfig {
    ProofConfig {
        verification_method: verification_method.to_string(),
        ..base.clone()
    }
}

/// Asserts `validate` and all four entry points report exactly `expected` for
/// `cfg`, without consulting the resolver.
fn assert_rejected_everywhere(fx: &Fixture, cfg: &ProofConfig, expected: &ProofOptionError) {
    assert_eq!(cfg.validate(), Err(expected.clone()), "validate: {cfg:?}");
    assert_eq!(
        &invalid_option(sign(&triples(), &fx.key, cfg)),
        expected,
        "sign: {cfg:?}"
    );
    assert_eq!(
        &invalid_option(sign_graph(&fx.graph, &fx.key, cfg)),
        expected,
        "sign_graph: {cfg:?}"
    );

    let mut proof = fx.good.clone();
    proof.config = cfg.clone();
    let resolver = CountingResolver::default();
    assert_eq!(
        &invalid_option(verify(&triples(), &proof, &resolver)),
        expected,
        "verify: {cfg:?}"
    );
    assert_eq!(
        &invalid_option(verify_graph(&fx.graph, &proof, &resolver)),
        expected,
        "verify_graph: {cfg:?}"
    );
    assert_eq!(resolver.calls(), 0, "resolver consulted for {cfg:?}");
}

/// Asserts `cfg` validates, signs through both sign entry points, and verifies
/// through both verify entry points with the config echoed verbatim.
fn assert_accepted_everywhere(fx: &Fixture, cfg: &ProofConfig) {
    assert_eq!(cfg.validate(), Ok(()), "validate: {cfg:?}");
    let proof = sign(&triples(), &fx.key, cfg).expect("valid options sign");
    let graph_proof = sign_graph(&fx.graph, &fx.key, cfg).expect("valid options sign a graph");
    assert_eq!(&proof.config, cfg);
    assert_eq!(&graph_proof.config, cfg);

    let resolver = CountingResolver::default();
    let v = verified(verify(&triples(), &proof, &resolver));
    let vg = verified(verify_graph(&fx.graph, &graph_proof, &resolver));
    assert_eq!(&v.config, cfg);
    assert_eq!(&vg.config, cfg);
    assert_eq!(resolver.calls(), 2, "{cfg:?}");
}

#[test]
fn boundary_created_values_round_trip_verbatim() {
    let fx = fixture();
    for &created in VALID_CREATED {
        let cfg = ProofConfig::new(vm_for(&fx.key)).with_created(created);
        assert_accepted_everywhere(&fx, &cfg);
        let proof = sign(&triples(), &fx.key, &cfg).unwrap();
        assert_eq!(proof.config.created.as_deref(), Some(created));
        let v = verified(verify(&triples(), &proof, &DidKeyResolver));
        assert_eq!(v.config.created.as_deref(), Some(created));
    }
}

/// The literal is signed as written: equal-valued lexical forms are
/// different signed statements.
#[test]
fn created_is_not_normalized() {
    let key = key();
    let signed = ProofConfig::new(vm_for(&key)).with_created("2023-12-31T24:00:00Z");
    let mut proof = sign(&triples(), &key, &signed).unwrap();
    verified(verify(&triples(), &proof, &DidKeyResolver));
    for equal_value in [
        "2024-01-01T00:00:00Z",
        "2023-12-31T24:00:00.0Z",
        "2023-12-31T24:00:00+00:00",
    ] {
        proof.config.created = Some(equal_value.to_string());
        assert_eq!(proof.config.validate(), Ok(()), "{equal_value:?}");
        assert!(
            matches!(
                verify(&triples(), &proof, &DidKeyResolver),
                Err(VcError::SignatureInvalid)
            ),
            "{equal_value:?}"
        );
    }
}

#[test]
fn invalid_created_is_rejected_by_every_entry_point() {
    let fx = fixture();
    for &created in INVALID_CREATED {
        let cfg = ProofConfig::new(vm_for(&fx.key)).with_created(created);
        let expected = cfg.validate().expect_err(created);
        assert!(
            matches!(&expected, ProofOptionError::Created { value, .. } if value == created),
            "{expected:?}"
        );
        assert_rejected_everywhere(&fx, &cfg, &expected);
    }
}

#[test]
fn verification_method_must_be_an_absolute_iri() {
    let fx = fixture();
    let base = ProofConfig::new(vm_for(&fx.key));
    for &vm in INVALID_VERIFICATION_METHODS {
        let cfg = with_method(vm, &base);
        let expected = cfg.validate().expect_err(vm);
        assert!(
            matches!(&expected, ProofOptionError::VerificationMethod { value, .. } if value == vm),
            "{vm:?}: {expected:?}"
        );
        assert_rejected_everywhere(&fx, &cfg, &expected);
    }
}

/// Lexical validity is not resolution: a well-formed IRI that is not a
/// resolvable `did:key` passes validation, then fails in the resolver.
#[test]
fn lexically_valid_method_still_goes_through_resolution() {
    let fx = fixture();
    for (vm, expected) in [
        (
            "https://issuer.example/keys/1",
            DidError::Malformed("https://issuer.example/keys/1".to_string()),
        ),
        (
            "did:web:issuer.example#key-1",
            DidError::UnsupportedMethod("web".to_string()),
        ),
    ] {
        let cfg = ProofConfig::new(vm);
        assert_eq!(cfg.validate(), Ok(()), "{vm:?}");
        let proof = sign(&triples(), &fx.key, &cfg).expect("well-formed options sign");
        let graph_proof =
            sign_graph(&fx.graph, &fx.key, &cfg).expect("well-formed options sign a graph");

        let resolver = CountingResolver::default();
        assert_eq!(did_error(verify(&triples(), &proof, &resolver)), expected);
        assert_eq!(
            did_error(verify_graph(&fx.graph, &graph_proof, &resolver)),
            expected
        );
        assert_eq!(resolver.calls(), 2, "{vm:?}");
    }
}

#[test]
fn purpose_accepts_supported_terms_and_absolute_iris_only() {
    let fx = fixture();
    let base = ProofConfig::new(vm_for(&fx.key));
    for term in SUPPORTED_PURPOSE_TERMS {
        assert_accepted_everywhere(&fx, &with_purpose(term, &base));
    }
    assert_accepted_everywhere(
        &fx,
        &with_purpose("https://example.test/purposes#audit", &base),
    );

    for &purpose in INVALID_PURPOSES {
        let cfg = with_purpose(purpose, &base);
        let expected = cfg.validate().expect_err(purpose);
        assert!(
            matches!(&expected, ProofOptionError::ProofPurpose { value, .. } if value == purpose),
            "{purpose:?}: {expected:?}"
        );
        assert_rejected_everywhere(&fx, &cfg, &expected);
    }
}

/// Each compact term and its VC v2 `@context` `@id` are the same RDF, so they
/// hash (and deterministically sign) identically and verify interchangeably.
/// The pre-zkp-14.3 `sec:<term>` IRI is a different statement for every term
/// except `assertionMethod`.
#[test]
fn compact_and_expanded_purpose_sign_identically() {
    let fx = fixture();
    let base = ProofConfig::new(vm_for(&fx.key));

    let mut oracle_terms: Vec<&str> = W3C_V2_PURPOSE_IDS.iter().map(|(t, _)| *t).collect();
    let mut supported = SUPPORTED_PURPOSE_TERMS.to_vec();
    oracle_terms.sort_unstable();
    supported.sort_unstable();
    assert_eq!(supported, oracle_terms);

    let mut compact_values = HashSet::new();
    for (term, id) in W3C_V2_PURPOSE_IDS {
        let compact = with_purpose(term, &base);
        let expanded = with_purpose(id, &base);

        let a = sign(&triples(), &fx.key, &compact).unwrap();
        let b = sign(&triples(), &fx.key, &expanded).unwrap();
        assert_eq!(a.proof_value, b.proof_value, "{term}");
        let ga = sign_graph(&fx.graph, &fx.key, &compact).unwrap();
        let gb = sign_graph(&fx.graph, &fx.key, &expanded).unwrap();
        assert_eq!(ga.proof_value, gb.proof_value, "{term}");

        // A proof signed under the compact term verifies with the config
        // rewritten to the expanded `@id`, and the config is echoed verbatim.
        let mut swapped = a.clone();
        swapped.config = expanded.clone();
        let v = verified(verify(&triples(), &swapped, &DidKeyResolver));
        assert_eq!(v.config.proof_purpose, id);
        let mut graph_swapped = ga;
        graph_swapped.config = expanded;
        verified(verify_graph(&fx.graph, &graph_swapped, &DidKeyResolver));

        let legacy = format!("https://w3id.org/security#{term}");
        if legacy != id {
            let old = sign(&triples(), &fx.key, &with_purpose(&legacy, &base)).unwrap();
            assert_ne!(old.proof_value, a.proof_value, "{term}");
            let mut legacy_swapped = a.clone();
            legacy_swapped.config = with_purpose(&legacy, &base);
            assert!(
                matches!(
                    verify(&triples(), &legacy_swapped, &DidKeyResolver),
                    Err(VcError::SignatureInvalid)
                ),
                "{term}"
            );
        } else {
            assert_eq!(term, "assertionMethod");
        }

        assert!(compact_values.insert(a.proof_value), "{term} collides");
    }

    // Compact IRIs are not expanded: `sec:assertionMethod` is a different IRI.
    let unexpanded = sign(
        &triples(),
        &fx.key,
        &with_purpose("sec:assertionMethod", &base),
    )
    .unwrap();
    assert!(!compact_values.contains(&unexpanded.proof_value));
    let custom = sign(
        &triples(),
        &fx.key,
        &with_purpose("https://example.test/purposes#audit", &base),
    )
    .unwrap();
    assert!(!compact_values.contains(&custom.proof_value));
}

/// Invalid options are reported before the canonicalizer or resolver runs;
/// valid ones reach them.
#[test]
fn invalid_options_fail_before_canonicalization_and_resolution() {
    let key = key();
    let doc = uncanonicalizable();
    let valid = ProofConfig::new(vm_for(&key));
    assert!(matches!(
        sign(&doc, &key, &valid),
        Err(VcError::Canon(sparq_canon::CanonError::TripleTerm))
    ));

    let good = sign(&triples(), &key, &valid).unwrap();
    let resolver = CountingResolver::default();
    assert!(matches!(
        verify(&doc, &good, &resolver),
        Err(VcError::Canon(sparq_canon::CanonError::TripleTerm))
    ));
    assert_eq!(resolver.calls(), 1);

    for invalid in [
        with_method("relative/key", &valid),
        with_purpose("unsupported", &valid),
        valid.clone().with_created("2023-02-30T00:00:00Z"),
    ] {
        let expected = invalid.validate().unwrap_err();
        assert_eq!(invalid_option(sign(&doc, &key, &invalid)), expected);

        let mut proof = good.clone();
        proof.config = invalid;
        assert_eq!(invalid_option(verify(&doc, &proof, &resolver)), expected);
        assert_eq!(
            resolver.calls(),
            1,
            "resolver consulted for an invalid config: {expected:?}"
        );
    }
}

#[test]
fn error_display_names_the_option_without_echoing_the_value() {
    let cases = [
        (
            ProofConfig::new("did:key:z6Mk").with_created("2023-02-30T00:00:00Z"),
            "invalid proof option `created`: ",
            Some("2023-02-30"),
        ),
        (
            with_purpose("noSuchPurpose", &ProofConfig::new("did:key:z6Mk")),
            "invalid proof option `proofPurpose`: ",
            Some("noSuchPurpose"),
        ),
        // The IRI parser's own reason is not pinned here, so only the prefix is.
        (
            ProofConfig::new("relative/key"),
            "invalid proof option `verificationMethod`: not an absolute IRI: ",
            None,
        ),
    ];
    for (cfg, prefix, hidden) in cases {
        let message = VcError::InvalidProofOption(cfg.validate().unwrap_err()).to_string();
        assert!(message.starts_with(prefix), "{message}");
        if let Some(hidden) = hidden {
            assert!(!message.contains(hidden), "{message}");
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Any `created` string either signs and verifies with the literal intact,
    /// or is rejected as an invalid `created` option. Half the inputs are
    /// dateTime-shaped (some valid, some not) so both outcomes are exercised.
    #[test]
    fn arbitrary_created_signs_verbatim_or_is_rejected(
        created in prop_oneof![
            any::<String>(),
            "-?[0-9]{4}-[01][0-9]-[0-3][0-9]T[0-2][0-9]:[0-5][0-9]:[0-6][0-9](\\.[0-9]{1,9})?(Z|[+-]1[0-5]:[0-5][0-9])?",
        ],
    ) {
        let key = key();
        let cfg = ProofConfig::new(vm_for(&key)).with_created(created.clone());
        match sign(&triples(), &key, &cfg) {
            Ok(proof) => {
                prop_assert_eq!(cfg.validate(), Ok(()));
                prop_assert_eq!(proof.config.created.as_deref(), Some(created.as_str()));
                prop_assert!(verify(&triples(), &proof, &DidKeyResolver).is_ok());
            }
            Err(VcError::InvalidProofOption(err)) => {
                prop_assert_eq!(cfg.validate(), Err(err.clone()));
                match err {
                    ProofOptionError::Created { value, .. } => prop_assert_eq!(value, created),
                    other => prop_assert!(false, "unexpected option error {:?}", other),
                }
            }
            Err(other) => prop_assert!(false, "unexpected error {:?}", other),
        }
    }
}
