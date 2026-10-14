//! End-to-end test of the trust-document storage/authoring model (`sq-pfae.5`, P4):
//! the admission cache key composes with the sparq-solid epoch cache, and the
//! per-`.acr` NARROWING composition is monotone. Drives the REAL [`TrustStore`] +
//! [`admit`] path (no mock): a verdict cached under `(epoch, AdmissionCacheKey)` is
//! served only while BOTH the epoch AND the trust policy are unchanged, and the
//! narrowing actually changes which credentials admit.
//!
//! Gated on the `store` feature (the whole file). Executed by a dedicated
//! `feature-matrix.yml` leg (`sparq-trust (store)`) — see that workflow.
//!
//! sq-pfae.5. 🤖 SPARQ agent — trust-graph authorisation PoC.
#![cfg(feature = "store")]

use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use sparq_trust::admit::{admit, PresentedCredential, Session};
use sparq_trust::policy::{parse_policy, ControlGate, TrustRule};
use sparq_trust::store::{AdmissionCacheKey, TrustDocument, TrustLayer, TrustStore};
use sparq_zk::sig::{public_key_to_hex, SecretKey};
use std::collections::HashMap;

const SCHEMA_AGE: &str = "http://schema.org/age";
const GOV: &str = "https://gov.example/issuer";

fn iri(s: &str) -> NamedNode {
    NamedNode::new(s).unwrap()
}

/// A Control-gated single-rule policy (gov trusts `schema:age` over `scope`, within
/// `fresh`).
fn policy(rule_iri: &str, key_hex: &str, scope: &str, fresh: &str) -> Vec<TrustRule> {
    let s = NamedOrBlankNode::NamedNode(iri(rule_iri));
    let t = "https://sparq.dev/ns/trust#";
    let triples = vec![
        Triple::new(
            s.clone(),
            iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type"),
            Term::NamedNode(iri(&format!("{t}TrustRule"))),
        ),
        Triple::new(s.clone(), iri(&format!("{t}source")), Term::NamedNode(iri(GOV))),
        Triple::new(
            s.clone(),
            iri(&format!("{t}issuerKey")),
            Term::Literal(Literal::new_simple_literal(key_hex)),
        ),
        Triple::new(
            s.clone(),
            iri(&format!("{t}forPredicate")),
            Term::NamedNode(iri(SCHEMA_AGE)),
        ),
        Triple::new(s.clone(), iri(&format!("{t}scope")), Term::NamedNode(iri(scope))),
        Triple::new(
            s,
            iri(&format!("{t}freshWithin")),
            Term::Literal(Literal::new_typed_literal(
                fresh,
                iri("http://www.w3.org/2001/XMLSchema#duration"),
            )),
        ),
    ];
    parse_policy(&triples, ControlGate::assert_control_gated()).unwrap()
}

/// A gov-signed `<holder> schema:age <age>` credential, issued at `issued_at`.
fn credential(holder: &str, age: i64, issued_at: i64) -> PresentedCredential {
    let sk = SecretKey::from_seed(0xC0FFEE);
    let salt = [3u8; 32];
    let graph = vec![Triple::new(
        NamedOrBlankNode::NamedNode(iri(holder)),
        iri(SCHEMA_AGE),
        Term::Literal(Literal::new_typed_literal(
            age.to_string(),
            iri("http://www.w3.org/2001/XMLSchema#integer"),
        )),
    )];
    // Sign over the RDFC-1.0 commitment, exactly as the issuance side does
    // (`sign_commitment` returns the hex-encoded signature directly).
    let salt_fr = sparq_zk::encode::salt_from_bytes(&salt);
    let commitment = sparq_zk::commit::commit_triples(&graph, salt_fr).unwrap();
    let issuer_signature_hex = sk.sign_commitment(&commitment.commitment);
    PresentedCredential {
        graph,
        issuer_signature_hex,
        salt,
        issued_at_unix_secs: issued_at,
        revoked: false,
    }
}

fn gov_key_hex() -> String {
    public_key_to_hex(&SecretKey::from_seed(0xC0FFEE).public_key())
}

/// A tiny admission cache keyed by `(epoch, AdmissionCacheKey)`, mirroring how
/// sparq-solid would cache an admission verdict on top of its epoch cache. The verdict
/// is the count of admitted facts.
#[derive(Default)]
struct AdmissionCache {
    inner: HashMap<(u64, AdmissionCacheKey), usize>,
    misses: usize,
}

impl AdmissionCache {
    /// Look up or compute-and-store the admission verdict. Counts a miss whenever the
    /// real gate had to run (so a test can assert a stale verdict was NOT served).
    fn verdict(
        &mut self,
        epoch: u64,
        store: &TrustStore,
        cred: &PresentedCredential,
        session: &Session,
        target: &NamedNode,
    ) -> usize {
        let key = store.admission_cache_key(cred, &session.agent, target);
        if let Some(v) = self.inner.get(&(epoch, key)) {
            return *v;
        }
        self.misses += 1;
        // The REAL admission gate over the effective (narrowed) rules.
        let rules = store.effective_rules(target);
        let admitted = admit(cred, &rules, session, target).len();
        self.inner.insert((epoch, key), admitted);
        admitted
    }
}

/// The load-bearing invariant: a cached admission verdict is reused while epoch AND
/// policy version are unchanged, but a trust-document edit/revoke bumps the policy
/// version → the cache key changes → the gate re-runs (no stale verdict).
#[test]
fn cache_key_invalidates_on_policy_change_but_reuses_when_unchanged() {
    let key = gov_key_hex();
    let target = iri("https://pod.ex/docs/x");
    let session = Session {
        agent: iri("https://jesse.ex/card#me"),
        now_unix_secs: 1_000_000,
    };
    let cred = credential("https://jesse.ex/card#me", 25, 1_000_000 - 86_400); // 1 day old

    let mut store = TrustStore::new();
    store
        .put(TrustDocument::new(
            TrustLayer::ServerWide,
            1,
            policy("https://pod.ex/trust#r", &key, "https://pod.ex/", "P30D"),
            ControlGate::assert_control_gated(),
        ))
        .unwrap();

    let mut cache = AdmissionCache::default();
    let epoch = 7;

    // First call: a miss, admits the age fact.
    assert_eq!(cache.verdict(epoch, &store, &cred, &session, &target), 1);
    assert_eq!(cache.misses, 1);

    // Same epoch + same policy + same credential: served from cache, NO new miss.
    assert_eq!(cache.verdict(epoch, &store, &cred, &session, &target), 1);
    assert_eq!(cache.misses, 1, "unchanged inputs reuse the cached verdict");

    // Edit the server-wide policy (a version bump). The policy version changes, so the
    // cache key changes and the gate re-runs (a new miss) — the stale verdict is NOT
    // served.
    store
        .put(TrustDocument::new(
            TrustLayer::ServerWide,
            2,
            policy("https://pod.ex/trust#r", &key, "https://pod.ex/", "P30D"),
            ControlGate::assert_control_gated(),
        ))
        .unwrap();
    assert_eq!(cache.verdict(epoch, &store, &cred, &session, &target), 1);
    assert_eq!(cache.misses, 2, "a policy edit invalidates the cached verdict");
}

/// The narrowing composition is REAL: a per-`.acr` document tightening freshness to P1D
/// turns a 5-day-old credential that the server-wide P30D would admit into a denial
/// inside that container — and the same credential still admits OUTSIDE the container.
#[test]
fn per_acr_freshness_narrowing_changes_admission() {
    let key = gov_key_hex();
    let session = Session {
        agent: iri("https://jesse.ex/card#me"),
        now_unix_secs: 1_000_000,
    };
    // 5-day-old credential.
    let cred = credential("https://jesse.ex/card#me", 25, 1_000_000 - 5 * 86_400);

    let mut store = TrustStore::new();
    store
        .put(TrustDocument::new(
            TrustLayer::ServerWide,
            1,
            policy("https://pod.ex/trust#r", &key, "https://pod.ex/", "P30D"),
            ControlGate::assert_control_gated(),
        ))
        .unwrap();
    // Per-.acr on /private/ narrows freshness to P1D.
    store
        .put(TrustDocument::new(
            TrustLayer::Container(iri("https://pod.ex/private/")),
            1,
            policy(
                "https://pod.ex/private/.acr#r",
                &key,
                "https://pod.ex/private/",
                "P1D",
            ),
            ControlGate::assert_control_gated(),
        ))
        .unwrap();

    // Inside /private/: the narrowed P1D denies the 5-day-old credential.
    let inside = iri("https://pod.ex/private/secret");
    let rules_inside = store.effective_rules(&inside);
    assert_eq!(
        admit(&cred, &rules_inside, &session, &inside).len(),
        0,
        "P1D narrowing denies the stale credential inside the container"
    );

    // Outside /private/ (e.g. /docs/x): the server-wide P30D still admits it.
    let outside = iri("https://pod.ex/docs/x");
    let rules_outside = store.effective_rules(&outside);
    assert_eq!(
        admit(&cred, &rules_outside, &session, &outside).len(),
        1,
        "the server-wide P30D admits it outside the narrowed container"
    );
}

/// A different credential (different holder) gets a different evidence hash → a
/// different cache key → its own verdict (no cross-credential cache collision).
#[test]
fn distinct_credentials_have_distinct_cache_keys() {
    let key = gov_key_hex();
    let target = iri("https://pod.ex/docs/x");
    let mut store = TrustStore::new();
    store
        .put(TrustDocument::new(
            TrustLayer::ServerWide,
            1,
            policy("https://pod.ex/trust#r", &key, "https://pod.ex/", "P30D"),
            ControlGate::assert_control_gated(),
        ))
        .unwrap();

    let jesse = iri("https://jesse.ex/card#me");
    let alice = iri("https://alice.ex/card#me");
    let cred_j = credential(jesse.as_str(), 25, 990_000);
    let cred_a = credential(alice.as_str(), 25, 990_000);

    let k_j = store.admission_cache_key(&cred_j, &jesse, &target);
    let k_a = store.admission_cache_key(&cred_a, &alice, &target);
    assert_ne!(
        k_j, k_a,
        "different holders/credentials must not share a cache key"
    );
}
