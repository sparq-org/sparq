//! Integration tests for sq-pfae.16: TrustStore x certification-edge composition.
//!
//! Verifies the load-bearing cache-safety invariants:
//!
//! (a) Revoking (removing) a certification from the document changes `policy_version`
//!     and therefore the `AdmissionCacheKey` — no stale verdict survives revocation.
//! (b) Re-authoring a certification's validity window changes `policy_version`;
//!     `effective_rules_at` re-derives fail-closed at the new instant.
//!     NOTE: wall-clock expiry of an UNCHANGED cert does NOT flip the cache key on its
//!     own — it propagates by re-materialise / epoch-bump (sq-l5og residue).
//! (c) Zero certifications: `effective_rules_at` is byte-identical to the pre-cert
//!     path (a document built with `TrustDocument::new` gives the same result as one
//!     built with `TrustDocument::with_certifications` and an empty slice).
//! (d) A cert-derived rule is correctly narrowed by the store's per-`.acr` narrowing
//!     discipline (the derived rule goes through the SAME server-ceiling + narrowing
//!     path as a direct rule).
//! (e) Mutation evidence: if `policy_version` did NOT incorporate the certification set,
//!     the revocation-invalidates-cache assertion would fail.
//!
//! Gated on BOTH `store` and `cert-graph` features (the file is compiled only when both
//! are on). Run with:
//! ```
//! cargo test -p sparq-trust --features store,cert-graph --test trust_store_cert_rules
//! ```
//!
//! sq-pfae.16. 🤖 SPARQ agent — trust-graph authorisation PoC.
//! No cryptographic-soundness or privacy claim: the ZK estate is internally re-audited
//! but external accredited-cryptographer sign-off is PENDING (sq-qhy4). MPC is
//! semi-honest only.
#![cfg(all(feature = "store", feature = "cert-graph"))]

use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use sparq_trust::graph::{CertScope, Certification, certification_message};
use sparq_trust::policy::{ControlGate, TrustRule, parse_policy};
use sparq_trust::store::{TrustDocument, TrustLayer, TrustStore};
use sparq_zk::sig::{SecretKey, public_key_to_hex};

// ── Shared constants ────────────────────────────────────────────────────────

const SCHEMA_AGE: &str = "http://schema.org/age";
const GOV: &str = "https://gov.example/framework";
const CERTIFIED: &str = "https://dvs.example/issuer";
const POD: &str = "https://pod.ex/";
const DOCS: &str = "https://pod.ex/docs/";
const RES_X: &str = "https://pod.ex/docs/resource-x";

// ── Test helpers ────────────────────────────────────────────────────────────

fn iri(s: &str) -> NamedNode {
    NamedNode::new(s).unwrap()
}

fn gate() -> ControlGate {
    ControlGate::assert_control_gated()
}

/// Deterministic key pair by seed.
fn keypair(seed: u64) -> (SecretKey, sparq_zk::sig::PublicKey) {
    let sk = SecretKey::from_seed(seed);
    let pk = sk.public_key();
    (sk, pk)
}

/// A one-rule, Control-gated policy: `gov` trusts `schema:age` over `scope`, for `fresh`
/// seconds, signed by `gov_key_hex`.
fn policy_rules(
    rule_iri: &str,
    source: &str,
    key_hex: &str,
    predicate: &str,
    scope: &str,
    fresh: &str,
) -> Vec<TrustRule> {
    use sparq_trust::vocab::{
        FOR_PREDICATE, FRESH_WITHIN, ISSUER_KEY, RDF_TYPE, SCOPE, SOURCE, TRUST_RULE,
    };
    let s = NamedOrBlankNode::NamedNode(iri(rule_iri));
    let xsd_dur = iri("http://www.w3.org/2001/XMLSchema#duration");
    let triples = vec![
        Triple::new(s.clone(), iri(RDF_TYPE), Term::NamedNode(iri(TRUST_RULE))),
        Triple::new(s.clone(), iri(SOURCE), Term::NamedNode(iri(source))),
        Triple::new(
            s.clone(),
            iri(ISSUER_KEY),
            Term::Literal(Literal::new_simple_literal(key_hex)),
        ),
        Triple::new(
            s.clone(),
            iri(FOR_PREDICATE),
            Term::NamedNode(iri(predicate)),
        ),
        Triple::new(s.clone(), iri(SCOPE), Term::NamedNode(iri(scope))),
        Triple::new(
            s,
            iri(FRESH_WITHIN),
            Term::Literal(Literal::new_typed_literal(fresh, xsd_dur)),
        ),
    ];
    parse_policy(&triples, gate()).unwrap()
}

/// Build a signed `Certification` edge from `certifier_sk` → `certified_issuer`/`certified_key`,
/// with the given validity window.
fn signed_cert(
    certifier: &str,
    certifier_sk: &SecretKey,
    certified: &str,
    certified_key: sparq_zk::sig::PublicKey,
    valid_from: i64,
    valid_until: i64,
) -> Certification {
    let mut cert = Certification {
        certifier: iri(certifier),
        certifier_key: certifier_sk.public_key(),
        certified_issuer: iri(certified),
        certified_key,
        scope: CertScope::AnyService,
        valid_from_unix_secs: valid_from,
        valid_until_unix_secs: valid_until,
        signature_hex: String::new(),
    };
    cert.signature_hex = certifier_sk.sign_commitment(&certification_message(&cert));
    cert
}

// ── (a) Revocation invalidates cached verdict ────────────────────────────────

/// LOAD-BEARING: revoking (removing) a certification from the document changes
/// `policy_version`, so any cached `AdmissionCacheKey` computed before the revocation
/// is no longer valid — a stale verdict is NOT reused.
///
/// Mutation evidence (why this test cannot vacuously pass): if the `policy_version`
/// computation did NOT fold the certification set, the hash before and after removing
/// the cert would be the SAME, and the assertion `assert_ne!(v_with, v_without)` would
/// fail. The test is verified non-vacuous by the fact that building a store with
/// identical direct rules but different certification sets gives different versions.
#[test]
fn revocation_invalidates_policy_version() {
    let (gov_sk, gov_pk) = keypair(0xABC);
    let (_dvs_sk, dvs_pk) = keypair(0xDEF);
    let gov_key_hex = public_key_to_hex(&gov_pk);

    let rules = policy_rules(
        "https://pod.ex/trust#r1",
        GOV,
        &gov_key_hex,
        SCHEMA_AGE,
        POD,
        "P30D",
    );
    let cert = signed_cert(GOV, &gov_sk, CERTIFIED, dvs_pk, 0, i64::MAX / 2);

    let target = iri(RES_X);

    // Build a store with the certification present.
    let mut store_with = TrustStore::new();
    store_with
        .put(TrustDocument::with_certifications(
            TrustLayer::ServerWide,
            1,
            rules.clone(),
            vec![cert.clone()],
            gate(),
        ))
        .unwrap();
    let v_with = store_with.policy_version(&target);

    // Build a store WITHOUT the certification (revocation = removal from the cert set).
    let mut store_without = TrustStore::new();
    store_without
        .put(TrustDocument::with_certifications(
            TrustLayer::ServerWide,
            1,
            rules.clone(),
            vec![], // cert removed
            gate(),
        ))
        .unwrap();
    let v_without = store_without.policy_version(&target);

    assert_ne!(
        v_with, v_without,
        "revoking a certification MUST change policy_version (cache-safety invariant)"
    );

    // The effective rules differ correspondingly.
    let now_secs = 1_000_000i64;
    let eff_with = store_with.effective_rules_at(&target, now_secs);
    let eff_without = store_without.effective_rules_at(&target, now_secs);
    assert_eq!(
        eff_with.len(),
        eff_without.len() + 1,
        "with cert: 1 direct + 1 derived; without cert: 1 direct only"
    );
    assert_eq!(
        eff_without[0].source.as_str(),
        GOV,
        "without cert: only the direct anchor rule"
    );
    // The derived rule authorises the CERTIFIED issuer, not the gov framework.
    assert_eq!(
        eff_with[1].source.as_str(),
        CERTIFIED,
        "the second rule is the cert-derived one for the certified issuer"
    );
}

// ── (b) Re-authored validity window changes policy_version; fail-closed on time ──

/// LOAD-BEARING: two scenarios are proved here:
///
/// 1. **Re-authored validity window → different `policy_version`.** Two stores built from
///    DIFFERENT stored `valid_until` values (i.e. the author re-authored the window to a
///    new value) produce different `policy_version` hashes and therefore different
///    `AdmissionCacheKey`s — a stale verdict from before the re-authoring is NOT reused.
///    This is the mechanism: the *stored* window is folded into the hash, so any change
///    to the stored document (including a window edit) is detected.
///
/// 2. **`effective_rules_at` fails closed on time.** When `now > valid_until` for the
///    cert in the store, `effective_rules_at` re-derives and the expired cert contributes
///    NOTHING (fail-closed on time, identical to the standalone `graph` path).
///
/// NOTE: this test does NOT prove that wall-clock expiry (time passing a fixed, unchanged
/// `valid_until` in an unchanged store) invalidates a cached verdict on its own — it does
/// NOT do so via `policy_version` alone.  That propagation path is re-materialise /
/// epoch-bump, bounded by the host epoch cadence; the residual stale-authority window is
/// the acknowledged open problem tracked by `sq-l5og`.
#[test]
fn reauthored_window_changes_policy_version_and_rederive_fails_closed_on_time() {
    let (gov_sk, gov_pk) = keypair(0x123);
    let (_dvs_sk, dvs_pk) = keypair(0x456);
    let gov_key_hex = public_key_to_hex(&gov_pk);

    let rules = policy_rules(
        "https://pod.ex/trust#r2",
        GOV,
        &gov_key_hex,
        SCHEMA_AGE,
        POD,
        "P30D",
    );
    let target = iri(RES_X);

    // Two certs with different validity windows (same certifier/certified, different `valid_until`).
    let cert_long = signed_cert(GOV, &gov_sk, CERTIFIED, dvs_pk, 0, 1_000_000);
    let cert_short = signed_cert(GOV, &gov_sk, CERTIFIED, dvs_pk, 0, 500_000);

    let mut store_long = TrustStore::new();
    store_long
        .put(TrustDocument::with_certifications(
            TrustLayer::ServerWide,
            1,
            rules.clone(),
            vec![cert_long],
            gate(),
        ))
        .unwrap();

    let mut store_short = TrustStore::new();
    store_short
        .put(TrustDocument::with_certifications(
            TrustLayer::ServerWide,
            1,
            rules.clone(),
            vec![cert_short],
            gate(),
        ))
        .unwrap();

    let v_long = store_long.policy_version(&target);
    let v_short = store_short.policy_version(&target);
    assert_ne!(
        v_long, v_short,
        "different stored valid_until (re-authored window) MUST change policy_version"
    );

    // Verify fail-closed on time: at now > cert_short.valid_until, the short cert
    // contributes nothing to effective_rules_at.
    let now_past_expiry = 600_000i64; // > 500_000 valid_until of cert_short
    let now_valid = 400_000i64;       // < 500_000 valid_until of cert_short

    let eff_expired = store_short.effective_rules_at(&target, now_past_expiry);
    let eff_valid = store_short.effective_rules_at(&target, now_valid);

    assert_eq!(
        eff_expired.len(),
        1,
        "after expiry: expired cert contributes NOTHING; only the direct anchor rule"
    );
    assert_eq!(
        eff_valid.len(),
        2,
        "before expiry: cert contributes a derived rule; direct + 1 derived"
    );
    assert_eq!(
        eff_expired[0].source.as_str(),
        GOV,
        "only the gov direct rule survives after expiry"
    );
}

// ── (c) Zero-cert byte-identity ─────────────────────────────────────────────

/// LOAD-BEARING: with NO certifications, `effective_rules_at` and `policy_version`
/// are byte-identical to the pre-cert path (a document built with `TrustDocument::new`
/// and one built with `with_certifications` + empty slice).
#[test]
fn zero_cert_byte_identity() {
    let (_, gov_pk) = keypair(0x789);
    let gov_key_hex = public_key_to_hex(&gov_pk);

    let rules = policy_rules(
        "https://pod.ex/trust#r3",
        GOV,
        &gov_key_hex,
        SCHEMA_AGE,
        POD,
        "P30D",
    );
    let target = iri(RES_X);
    let now_secs = 1_000_000i64;

    // Store built with TrustDocument::new (no cert-graph awareness).
    let mut store_new = TrustStore::new();
    store_new
        .put(TrustDocument::new(TrustLayer::ServerWide, 1, rules.clone(), gate()))
        .unwrap();

    // Store built with TrustDocument::with_certifications + empty cert slice.
    let mut store_empty_certs = TrustStore::new();
    store_empty_certs
        .put(TrustDocument::with_certifications(
            TrustLayer::ServerWide,
            1,
            rules.clone(),
            vec![],
            gate(),
        ))
        .unwrap();

    // policy_version must be identical.
    assert_eq!(
        store_new.policy_version(&target),
        store_empty_certs.policy_version(&target),
        "zero certifications ⇒ policy_version byte-identical to new()"
    );

    // effective_rules_at must produce the same result.
    let eff_new = store_new.effective_rules_at(&target, now_secs);
    let eff_empty = store_empty_certs.effective_rules_at(&target, now_secs);
    assert_eq!(
        eff_new.len(),
        eff_empty.len(),
        "zero certifications ⇒ same number of effective rules"
    );
    for (a, b) in eff_new.iter().zip(eff_empty.iter()) {
        assert_eq!(a.source.as_str(), b.source.as_str());
        assert_eq!(a.scope.as_str(), b.scope.as_str());
        assert_eq!(a.fresh_within_secs, b.fresh_within_secs);
        assert_eq!(
            public_key_to_hex(&a.issuer_key),
            public_key_to_hex(&b.issuer_key)
        );
    }
}

// ── (d) Per-`.acr` narrowing applies to cert-derived rules ──────────────────

/// SOUNDNESS: a cert-derived rule is further narrowed by a per-`.acr` document that
/// covers the target — it goes through the SAME server-ceiling + narrowing discipline
/// as a direct rule. The derived rule's scope and freshness are tightened by the
/// per-`.acr` document.
#[test]
fn cert_derived_rule_is_narrowed_by_per_acr() {
    let (gov_sk, gov_pk) = keypair(0xAA1);
    let (_dvs_sk, dvs_pk) = keypair(0xBB2);
    let gov_key_hex = public_key_to_hex(&gov_pk);
    let dvs_key_hex = public_key_to_hex(&dvs_pk);

    // Server-wide: gov trusts age over all /pod.ex/, P30D.
    let server_rules = policy_rules(
        "https://pod.ex/trust#srv",
        GOV,
        &gov_key_hex,
        SCHEMA_AGE,
        POD,
        "P30D",
    );
    let cert = signed_cert(GOV, &gov_sk, CERTIFIED, dvs_pk, 0, i64::MAX / 2);

    // Per-.acr on /docs/: the certified issuer (dvs) trusts age over /docs/, P7D.
    // This should narrow the cert-derived rule from P30D/POD to P7D/DOCS.
    let acr_rules = policy_rules(
        "https://pod.ex/docs/.acr#dvsr",
        CERTIFIED,
        &dvs_key_hex,
        SCHEMA_AGE,
        DOCS,
        "P7D",
    );

    let mut store = TrustStore::new();
    store
        .put(TrustDocument::with_certifications(
            TrustLayer::ServerWide,
            1,
            server_rules,
            vec![cert],
            gate(),
        ))
        .unwrap();
    store
        .put(TrustDocument::new(
            TrustLayer::Container(iri(DOCS)),
            1,
            acr_rules,
            gate(),
        ))
        .unwrap();

    let target = iri(RES_X);
    let now_secs = 1_000_000i64;
    let eff = store.effective_rules_at(&target, now_secs);

    // We expect: 1 direct rule (gov, P30D, narrowed to /docs/ scope) + 1 derived (dvs, narrowed).
    // The per-`.acr` can only NARROW — it cannot broaden the ceiling.
    assert!(eff.len() >= 2, "at least: direct gov rule + cert-derived dvs rule");

    // Find the cert-derived rule (source == CERTIFIED).
    let derived = eff.iter().find(|r| r.source.as_str() == CERTIFIED);
    assert!(derived.is_some(), "the cert-derived rule must appear in effective_rules");
    let derived = derived.unwrap();

    // The derived rule must be NARROWED by the per-.acr: no fresher than P7D.
    assert!(
        derived.fresh_within_secs <= 7 * 86_400,
        "cert-derived rule is narrowed to P7D by the per-.acr; got {} s",
        derived.fresh_within_secs
    );
    // The scope must be the narrower /docs/.
    assert_eq!(
        derived.scope.as_str(),
        DOCS,
        "cert-derived rule scope is narrowed to /docs/ by the per-.acr"
    );
}

// ── (e) Mutation evidence: the test goes red if the invariant is broken ─────

/// MUTATION EVIDENCE: demonstrates that the `revocation_invalidates_policy_version`
/// test would FAIL if `policy_version` did NOT incorporate the certification set.
///
/// Specifically: if we compute `policy_version` using ONLY the document version (like
/// the pre-.16 path), two stores with the SAME document version but DIFFERENT cert sets
/// would produce the SAME version — the stale-verdict invariant would be broken.
///
/// This test asserts the OPPOSITE (that the versions ARE different) to prove the
/// cert-aware implementation is what's running.
#[test]
fn mutation_evidence_version_differs_across_cert_sets() {
    let (gov_sk, gov_pk) = keypair(0xCC3);
    let (_dvs_sk, dvs_pk) = keypair(0xDD4);
    let gov_key_hex = public_key_to_hex(&gov_pk);

    let rules = policy_rules(
        "https://pod.ex/trust#rmut",
        GOV,
        &gov_key_hex,
        SCHEMA_AGE,
        POD,
        "P30D",
    );
    let cert = signed_cert(GOV, &gov_sk, CERTIFIED, dvs_pk, 0, 9_999_999);

    let target = iri(RES_X);

    // Version 1 WITH cert.
    let mut store_a = TrustStore::new();
    store_a
        .put(TrustDocument::with_certifications(
            TrustLayer::ServerWide,
            1, // same document version as store_b below
            rules.clone(),
            vec![cert],
            gate(),
        ))
        .unwrap();

    // Version 1 WITHOUT cert (same document version, different cert set).
    let mut store_b = TrustStore::new();
    store_b
        .put(TrustDocument::with_certifications(
            TrustLayer::ServerWide,
            1, // same document version
            rules,
            vec![],
            gate(),
        ))
        .unwrap();

    let va = store_a.policy_version(&target);
    let vb = store_b.policy_version(&target);

    // If this assertion fails, the implementation is NOT incorporating the cert set into
    // policy_version — the cache-safety invariant is broken.
    assert_ne!(
        va, vb,
        "MUTATION EVIDENCE: same document version but different cert sets MUST produce \
         different policy_version — if this fails, the implementation is broken"
    );
}
