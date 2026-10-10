//! Behavioural proof of the dropped-view guard: once [`PodStore::drop_auth_view`] has run,
//! no grant source other than a static WAC/ACP materialization changes the view. Every
//! public grant and refresh entry point is driven twice with the same inputs: on a live
//! store, where it must install (the positive control that makes the check meaningful),
//! and on a dropped store, where it must install nothing and grant no access.

use super::*;

const N1: &str = "https://pod.ex/notes/n1";
const ALICE: &str = "https://alice.ex/card#me";
const BOB: &str = "https://bob.ex/card#me";

/// One document, and an `.acl` granting Bob (not Alice) Read on the pod.
fn pod() -> PodStore {
    let nq = format!(
        "<{N1}#it> <https://ex.dev/ns#title> \"t\" <{N1}> .\n\
         <https://pod.ex/.acl#a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .\n\
         <https://pod.ex/.acl#a> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> <https://pod.ex/.acl> .\n\
         <https://pod.ex/.acl#a> <http://www.w3.org/ns/auth/acl#agent> <{BOB}> <https://pod.ex/.acl> .\n\
         <https://pod.ex/.acl#a> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .\n"
    );
    let mut store = PodStore::new(Graph::load_dataset(&nq, "nquads").expect("loads"));
    store.materialize_wac().expect("materializes");
    store
}

fn reads(store: &PodStore, agent: &str, doc: &str) -> bool {
    let s = Session { agent: Some(agent), client: None, issuer: None, now: None };
    store.accessible(&s, Mode::Read).iter().any(|g| g.as_str() == doc)
}

/// Drive `install` on a live pod (it must report an install, and when `grants`, Alice must
/// then read [`N1`]) and on a dropped one (it must report none, and nobody may read anything).
fn assert_guarded(name: &str, grants: bool, install: impl Fn(&mut PodStore) -> bool) {
    let mut live = pod();
    assert!(install(&mut live), "{name}: the positive control installs nothing");
    if grants {
        assert!(reads(&live, ALICE, N1), "{name}: the positive control grants nothing");
    }
    let mut dropped = dropped_pod();
    assert!(!install(&mut dropped), "{name}: installed onto a dropped view");
    for agent in [ALICE, BOB] {
        let s = Session { agent: Some(agent), client: None, issuer: None, now: None };
        assert!(dropped.accessible(&s, Mode::Read).is_empty(), "{name}: {agent} reads through a dropped view");
    }
    assert!(!dropped.may_install_grants(), "{name}: the drop was cleared");
}

fn dropped_pod() -> PodStore {
    let mut store = pod();
    store.drop_auth_view();
    store
}

#[cfg(feature = "odrl-bridge")]
mod odrl {
    use super::*;
    use sparq_policy::{parse_policy_str, Request, ValidatedPolicy};

    fn policy(rule: &str) -> ValidatedPolicy {
        parse_policy_str(
            &format!(
                "@prefix odrl: <http://www.w3.org/ns/odrl/2/> .\n\
                 <urn:pol/p> a odrl:Set ; odrl:{rule} [ odrl:action odrl:read ; \
                 odrl:target <{N1}> ; odrl:assignee <{ALICE}> ] ."
            ),
            "turtle",
        )
        .expect("parses")
    }

    fn request() -> Request {
        Request::new("http://www.w3.org/ns/odrl/2/read").on(N1).by(ALICE)
    }

    #[test]
    fn no_odrl_materializer_installs_onto_a_dropped_view() {
        let permit = policy("permission");
        let deny = policy("prohibition");
        let req = request();
        assert_guarded("materialize_odrl_permission", true, |s| {
            s.materialize_odrl_permission(&permit, &req).granted
        });
        assert_guarded("materialize_odrl_policy", true, |s| {
            s.materialize_odrl_policy(&permit, &req).granted
        });
        assert_guarded("materialize_odrl_policy_for_each", true, |s| {
            s.materialize_odrl_policy_for_each(&permit, std::slice::from_ref(&req))[0].granted
        });
        assert_guarded("materialize_odrl_permission_conditional", true, |s| {
            s.materialize_odrl_permission_conditional(&permit, &req).granted
        });
        assert_guarded("materialize_odrl_prohibition", false, |s| {
            s.materialize_odrl_prohibition(&deny, &req).prohibited
        });
        assert_guarded("materialize_odrl_prohibition_conditional", false, |s| {
            s.materialize_odrl_prohibition_conditional(&deny, &req).prohibited
        });
    }

    #[cfg(feature = "count-enforcement")]
    #[test]
    fn the_counted_materializer_does_not_install_onto_a_dropped_view() {
        let permit = policy("permission");
        let req = request();
        let counter: Arc<dyn sparq_policy::UsageCounterStore + Send + Sync> =
            Arc::new(sparq_policy::InMemoryCounterStore::new());
        assert_guarded("materialize_odrl_permission_counted", true, |s| {
            s.materialize_odrl_permission_counted(&permit, &req, &counter).granted
        });
    }

    /// The ledger refresh rebuilds from the baseline captured before the drop, so a refresh
    /// that ran on a dropped view would grant again what the drop withdrew.
    #[test]
    fn no_ledger_refresh_rebuilds_a_dropped_view() {
        let permit = policy("permission");
        let req = request();
        let bridged = || {
            let mut store = pod();
            assert!(store.materialize_odrl_permission(&permit, &req).granted);
            store
        };
        let refreshes: [(&str, &dyn Fn(&mut PodStore)); 2] = [
            ("refresh_odrl_grants", &|s| {
                s.refresh_odrl_grants();
            }),
            ("refresh_odrl_grant", &|s| {
                s.refresh_odrl_grant(&permit, &req, odrl_bridge::BridgeKind::Permission);
            }),
        ];
        for (name, refresh) in refreshes {
            // Positive control: on a live view the refresh keeps the bridged grant.
            let mut live = bridged();
            refresh(&mut live);
            assert!(reads(&live, ALICE, N1), "{name}: the positive control lost the grant");
            let mut dropped = bridged();
            dropped.drop_auth_view();
            refresh(&mut dropped);
            for agent in [ALICE, BOB] {
                assert!(!reads(&dropped, agent, N1), "{name}: {agent} reads through a dropped view");
            }
            assert!(!dropped.may_install_grants(), "{name}: the drop was cleared");
        }
    }
}

#[cfg(feature = "trust-graph")]
mod trust {
    use super::*;
    use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
    use sparq_trust::admit::{PresentedCredential, Session as TrustSession};
    use sparq_trust::policy::{parse_policy, ControlGate, TrustRule};
    use sparq_trust::vocab;
    use sparq_zk::commit::commit_triples;
    use sparq_zk::encode::salt_from_bytes;
    use sparq_zk::sig::{public_key_to_hex, SecretKey};

    const JESSE: &str = "https://jesse.ex/card#me";
    const ISSUER: &str = "https://gov.example/issuer";
    const AGE: &str = "http://schema.org/age";
    const NOW: i64 = 1_700_000_000;
    const SALT: [u8; 32] = [9u8; 32];
    const RULE: &str = "@prefix schema: <http://schema.org/> .\n\
        @prefix math: <http://www.w3.org/2000/10/swap/math#> .\n\
        @prefix auth: <https://sparq.dev/ns/auth#> .\n\
        { ?x schema:age ?y . ?y math:greaterThan 18 } => { ?x auth:read <https://pod.ex/notes/n1> } .\n";

    fn iri(s: &str) -> NamedNode {
        NamedNode::new(s).expect("iri")
    }

    fn triple(s: &NamedNode, p: &str, o: Term) -> Triple {
        Triple::new(NamedOrBlankNode::NamedNode(s.clone()), iri(p), o)
    }

    /// A government-signed `<Jesse> schema:age 25`, and a rule trusting that issuer for it.
    fn credential_and_rules() -> (PresentedCredential, Vec<TrustRule>) {
        let sk = SecretKey::from_seed(0xABCDEF);
        let graph = vec![triple(
            &iri(JESSE),
            AGE,
            Term::Literal(Literal::new_typed_literal("25", iri("http://www.w3.org/2001/XMLSchema#integer"))),
        )];
        let commitment = commit_triples(&graph, salt_from_bytes(&SALT)).expect("commits");
        let cred = PresentedCredential {
            graph,
            issuer_signature_hex: sk.sign_commitment(&commitment.commitment),
            salt: SALT,
            issued_at_unix_secs: NOW - 86_400,
            revoked: false,
        };
        let rule = iri("https://pod.ex/.acr#trustrule");
        let policy = vec![
            triple(&rule, vocab::RDF_TYPE, Term::NamedNode(iri(vocab::TRUST_RULE))),
            triple(&rule, vocab::SOURCE, Term::NamedNode(iri(ISSUER))),
            triple(&rule, vocab::ISSUER_KEY, Term::Literal(Literal::new_simple_literal(public_key_to_hex(&sk.public_key())))),
            triple(&rule, vocab::FOR_PREDICATE, Term::NamedNode(iri(AGE))),
            triple(&rule, vocab::SCOPE, Term::NamedNode(iri(N1))),
            triple(&rule, vocab::FRESH_WITHIN, Term::Literal(Literal::new_simple_literal("P30D"))),
        ];
        (cred, parse_policy(&policy, ControlGate::assert_control_gated()).expect("rules parse"))
    }

    #[test]
    fn no_trust_installer_installs_onto_a_dropped_view() {
        let (cred, rules) = credential_and_rules();
        let session = TrustSession { agent: iri(JESSE), now_unix_secs: NOW };
        let target = iri(N1);
        // The grant goes to Jesse, so the read check is Jesse's.
        let jesse_reads = |name: &str, store: &PodStore, want: bool| {
            assert_eq!(reads(store, JESSE, N1), want, "{name}");
        };
        let mut live = pod();
        let out = live
            .admit_trust_credential_with_rule(&cred, &rules, &session, &target, RULE)
            .expect("admits");
        assert_eq!(out.installed_grants.len(), 1, "the positive control installs nothing");
        jesse_reads("admit_trust_credential_with_rule: positive control", &live, true);
        let mut dropped = dropped_pod();
        let out = dropped
            .admit_trust_credential_with_rule(&cred, &rules, &session, &target, RULE)
            .expect("admits");
        assert!(out.installed_grants.is_empty(), "admit_trust_credential_with_rule installed onto a dropped view");
        jesse_reads("admit_trust_credential_with_rule: dropped", &dropped, false);

        let mut live = pod();
        let out = live.admit_trust_credential_static(&cred, &rules, &target, RULE).expect("admits");
        assert_eq!(out.installed_count, 1, "the positive control installs nothing");
        let mut dropped = dropped_pod();
        let out = dropped.admit_trust_credential_static(&cred, &rules, &target, RULE).expect("admits");
        assert_eq!(out.installed_count, 0, "admit_trust_credential_static installed onto a dropped view");

        // `admit_trust_credential_and_materialize` is `admit_trust_credential_with_rule`
        // with the store's own rule, so the check above covers it.
    }
}
