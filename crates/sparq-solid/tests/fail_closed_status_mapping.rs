//! [FABLE-5] sq-6tgxp — the FR-6 fail-closed load/error contract as a **table oracle**
//! over [`AclStatus`] + [`AclStatus::is_retryable`], locking the 403-vs-503 split a Solid
//! resource server (epic sq-gg0qq) must honour when mapping a [`WacDecision`] to an HTTP
//! status:
//!
//! | decision                                              | HTTP  |
//! |-------------------------------------------------------|-------|
//! | `allow == true` (necessarily `Resolved`)              | `200` |
//! | retryable deny (`Unloaded` / `Transient`)             | `503` |
//! | definitive deny (`Resolved` / `NoAcl`), ANY requester | `403` |
//!
//! sq-qonip: `http_status` below is a test-local **model** of that split, and it takes no
//! authentication state — because [`AclStatus`] carries none, which is what this suite can
//! honestly pin. `sparq-solid` is downstream of no HTTP crate, so nothing here observes the
//! shipped shell: `sparq-server`'s `solid_authz::deny_status_code` is pinned by its OWN unit
//! tests (`deny_status_code_maps_definitive_vs_retryable`,
//! `shipped_mapper_gives_anonymous_definitive_denies_the_same_403`), which is where a
//! server-side drift would be caught. Note the shipped 403-for-anonymous is a known
//! non-conformance on an LDP resource-serving path, documented on `AclStatus`; this suite
//! locks the *library* contract underneath it, not that HTTP choice.
//!
//! Invariants pinned here (all fail-closed, per `decide.rs` FR-6 / sq-snopa.2):
//! - `is_retryable()` holds **exactly** for `{Unloaded, Transient}` and NOT for
//!   `{Resolved, NoAcl}` — retryable statuses map to a distinct 503-class, never 403.
//! - A retryable status **never** surfaces as `allow == true`.
//! - `acl_link_header()` is `Some` **iff** `governing_acl` is `Some` — including
//!   `Unloaded`, where the ACL is discovered even though the view is unmaterialized.
//!
//! Exercises only the crate's always-on public decision surface (`PodStore::decide`),
//! deliberately in a fresh compilation unit disjoint from the `src/` unit tests and the
//! sibling integration suites.

use sparq_solid::{AclStatus, Mode, PodStore, Session, WacDecision};

/// A root `.acl` granting alice `acl:Read` (only) on everything under the pod root by
/// `acl:default`, plus one document under `/notes/` for decisions to target.
const POD_WITH_ROOT_ACL: &str = r#"
<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> "hello" <https://pod.ex/notes/n1> .
<https://pod.ex/.acl#owner> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .
<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> <https://pod.ex/.acl> .
<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#agent> <https://alice.ex/card#me> <https://pod.ex/.acl> .
<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .
"#;

/// A pod with data but NO access-control document anywhere up the container chain.
const POD_WITHOUT_ACL: &str =
    "<https://pod.ex/d#it> <https://ex.dev/ns#k> \"v\" <https://pod.ex/d> .\n";

const RESOURCE: &str = "https://pod.ex/notes/n1";

fn alice() -> Session<'static> {
    Session {
        agent: Some("https://alice.ex/card#me"),
        client: None,
        issuer: None,
        now: None,
    }
}

fn anonymous() -> Session<'static> {
    Session::default()
}

/// A [`PodStore`] over `nquads` with the WAC auth view materialized.
fn materialized_store(nquads: &str) -> PodStore {
    let mut store =
        PodStore::new(sparq_core::Graph::load_dataset(nquads, "nquads").expect("fixture loads"));
    store.materialize_wac().expect("materializes");
    store
}

/// A test-local **model** of the FR-6 status mapping a resource server applies to a
/// [`WacDecision`] — the 200/403/503 split this suite locks.
///
/// sq-qonip: it is NOT the shipped mapper and nothing here calls one; it takes no
/// authentication state because [`AclStatus`] supplies none, which is the invariant this
/// file actually pins. `sparq-server`'s `deny_status_code` is tested in `sparq-server`.
fn http_status(d: &WacDecision) -> u16 {
    if d.allow {
        200
    } else if d.status.is_retryable() {
        // Unloaded / Transient: operational, not a permission outcome — retryable class.
        503
    } else {
        // Resolved-deny / NoAcl: a definitive, authoritative forbidden, for every requester.
        403
    }
}

/// `is_retryable()` holds for EXACTLY `{Unloaded, Transient}` — exhaustive over the enum,
/// asserted against an independent `matches!` oracle so adding a variant or editing the
/// method body must reconcile with this table.
#[test]
fn is_retryable_exactly_unloaded_and_transient() {
    let all = [
        AclStatus::Resolved,
        AclStatus::NoAcl,
        AclStatus::Unloaded,
        AclStatus::Transient,
    ];
    for status in all {
        let expected = matches!(status, AclStatus::Unloaded | AclStatus::Transient);
        assert_eq!(
            status.is_retryable(),
            expected,
            "is_retryable({status:?}) must be {expected} — retryable is exactly {{Unloaded, Transient}}"
        );
    }
    // Belt-and-braces: the definitive statuses spelled out.
    assert!(!AclStatus::Resolved.is_retryable());
    assert!(!AclStatus::NoAcl.is_retryable());
    assert!(AclStatus::Unloaded.is_retryable());
    assert!(AclStatus::Transient.is_retryable());
}

/// The full end-to-end table: real `PodStore::decide` calls in every reachable status,
/// checked against the expected `(allow, status, http)` row.
#[test]
fn status_to_http_table() {
    let materialized = materialized_store(POD_WITH_ROOT_ACL);
    let no_acl_store = materialized_store(POD_WITHOUT_ACL);
    // A governing ACL exists but the auth view was never materialized → Unloaded.
    let unloaded_store = PodStore::new(
        sparq_core::Graph::load_dataset(POD_WITH_ROOT_ACL, "nquads").expect("fixture loads"),
    );

    struct Row {
        name: &'static str,
        decision: WacDecision,
        expect_allow: bool,
        expect_status: AclStatus,
        expect_http: u16,
    }

    let rows = [
        Row {
            name: "authoritative allow (alice holds Read)",
            decision: materialized.decide(&alice(), RESOURCE, Mode::Read),
            expect_allow: true,
            expect_status: AclStatus::Resolved,
            expect_http: 200,
        },
        Row {
            name: "authoritative deny: authenticated principal lacks the mode",
            decision: materialized.decide(&alice(), RESOURCE, Mode::Write),
            expect_allow: false,
            expect_status: AclStatus::Resolved,
            expect_http: 403,
        },
        Row {
            name: "anonymous over a resource requiring auth: definitive deny",
            decision: materialized.decide(&anonymous(), RESOURCE, Mode::Read),
            expect_allow: false,
            expect_status: AclStatus::Resolved,
            expect_http: 403,
        },
        Row {
            name: "no discoverable ACL, authenticated: definitive deny",
            decision: no_acl_store.decide(&alice(), "https://pod.ex/d", Mode::Read),
            expect_allow: false,
            expect_status: AclStatus::NoAcl,
            expect_http: 403,
        },
        Row {
            name: "no discoverable ACL, anonymous: definitive deny",
            decision: no_acl_store.decide(&anonymous(), "https://pod.ex/d", Mode::Read),
            expect_allow: false,
            expect_status: AclStatus::NoAcl,
            expect_http: 403,
        },
        Row {
            name: "ACL discovered but view unmaterialized: retryable",
            decision: unloaded_store.decide(&alice(), RESOURCE, Mode::Read),
            expect_allow: false,
            expect_status: AclStatus::Unloaded,
            expect_http: 503,
        },
        Row {
            name: "malformed resource IRI: typed transient error, retryable",
            decision: materialized.decide(&alice(), "not a valid iri", Mode::Read),
            expect_allow: false,
            expect_status: AclStatus::Transient,
            expect_http: 503,
        },
    ];

    for row in &rows {
        let d = &row.decision;
        assert_eq!(d.allow, row.expect_allow, "[{}] allow", row.name);
        assert_eq!(d.status, row.expect_status, "[{}] status", row.name);
        assert_eq!(
            http_status(d),
            row.expect_http,
            "[{}] HTTP mapping",
            row.name
        );

        // INVARIANT (fail-closed): a retryable status NEVER surfaces as allow — the
        // 503 class and the allow verdict are mutually exclusive by construction.
        assert!(
            !(d.status.is_retryable() && d.allow),
            "[{}] retryable status must never allow",
            row.name
        );
        // INVARIANT: allow only ever rides an authoritative Resolved decision.
        if d.allow {
            assert_eq!(
                d.status,
                AclStatus::Resolved,
                "[{}] allow ⇒ Resolved",
                row.name
            );
        }
        // INVARIANT (FR-5): the Link: rel="acl" surface exists iff discovery found a
        // governing ACL — checked across EVERY row, in every status.
        assert_eq!(
            d.acl_link_header().is_some(),
            d.governing_acl.is_some(),
            "[{}] acl_link_header() Some iff governing_acl Some",
            row.name
        );
    }
}

/// sq-qonip — the **library** invariant underneath the HTTP split: a definitive deny to an
/// anonymous requester is indistinguishable, in everything [`AclStatus`] exposes, from the
/// authenticated one. Same `status`, same retryability, so any mapper reading only a
/// [`WacDecision`] necessarily emits the same code for both.
///
/// That is what makes the shipped shell's uniform 403 a *consequence* rather than a choice —
/// and it is why an LDP resource server owing an anonymous requester Solid's 401 challenge
/// must source the authentication state itself (a known non-conformance if it does not; see
/// the `AclStatus` docs). This test does not observe any HTTP mapper: `http_status` is the
/// local model, and `sparq-server`'s real one is pinned by its own unit tests.
#[test]
fn anonymous_definitive_deny_is_indistinguishable_from_authenticated() {
    let materialized = materialized_store(POD_WITH_ROOT_ACL);
    let no_acl_store = materialized_store(POD_WITHOUT_ACL);

    // The two definitive-deny statuses, reached by an ANONYMOUS requester.
    let anon_resolved = materialized.decide(&anonymous(), RESOURCE, Mode::Read);
    let anon_no_acl = no_acl_store.decide(&anonymous(), "https://pod.ex/d", Mode::Read);
    // Their AUTHENTICATED counterparts: alice holds Read but not Write, and the ACL-less
    // pod denies her too.
    let auth_resolved = materialized.decide(&alice(), RESOURCE, Mode::Write);
    let auth_no_acl = no_acl_store.decide(&alice(), "https://pod.ex/d", Mode::Read);

    for (name, d) in [
        ("Resolved, anonymous", &anon_resolved),
        ("NoAcl, anonymous", &anon_no_acl),
    ] {
        assert!(!d.allow, "[{}] fail-closed", name);
        assert!(
            !d.status.is_retryable(),
            "[{}] a definitive deny, not the retryable 503 class",
            name
        );
        assert_eq!(
            http_status(d),
            403,
            "[{}] a definitive deny under the local model of the mapping",
            name
        );
    }

    // The authenticated counterparts are IDENTICAL in everything the decision exposes, so
    // no mapper reading only a `WacDecision` can tell the two apart.
    assert_eq!(anon_resolved.status, auth_resolved.status);
    assert_eq!(
        http_status(&anon_resolved),
        http_status(&auth_resolved),
        "Resolved deny: anonymous and authenticated share one code"
    );
    assert_eq!(anon_no_acl.status, auth_no_acl.status);
    assert_eq!(
        http_status(&anon_no_acl),
        http_status(&auth_no_acl),
        "NoAcl deny: anonymous and authenticated share one code"
    );
}

/// `Unloaded` still discovers + advertises the governing ACL (decide.rs FR-5/FR-6): the
/// server can tell the client WHERE the ACL is even on a retryable 503; and the
/// undiscovered statuses (`NoAcl`, `Transient`) advertise nothing.
#[test]
fn acl_link_header_presence_follows_discovery() {
    // Unloaded: governing ACL discovered although the view is unmaterialized.
    let unloaded_store = PodStore::new(
        sparq_core::Graph::load_dataset(POD_WITH_ROOT_ACL, "nquads").expect("fixture loads"),
    );
    let unloaded = unloaded_store.decide(&alice(), RESOURCE, Mode::Read);
    assert_eq!(unloaded.status, AclStatus::Unloaded);
    assert!(
        unloaded.governing_acl.is_some(),
        "Unloaded still discovers the ACL"
    );
    assert_eq!(
        unloaded.acl_link_header().as_deref(),
        Some(r#"<https://pod.ex/.acl>; rel="acl""#),
        "the link value is the discovered governing ACL as an RFC 8288 link-value"
    );

    // Resolved (allow AND deny): same governing ACL, same link.
    let materialized = materialized_store(POD_WITH_ROOT_ACL);
    for mode in [Mode::Read, Mode::Write] {
        let d = materialized.decide(&alice(), RESOURCE, mode);
        assert_eq!(d.status, AclStatus::Resolved);
        assert_eq!(
            d.acl_link_header().as_deref(),
            Some(r#"<https://pod.ex/.acl>; rel="acl""#)
        );
    }

    // NoAcl / Transient: no governing ACL was discovered ⇒ nothing to advertise.
    let no_acl_store = materialized_store(POD_WITHOUT_ACL);
    let no_acl = no_acl_store.decide(&alice(), "https://pod.ex/d", Mode::Read);
    assert_eq!(no_acl.status, AclStatus::NoAcl);
    assert!(no_acl.governing_acl.is_none());
    assert!(no_acl.acl_link_header().is_none());

    let transient = no_acl_store.decide(&alice(), "not a valid iri", Mode::Read);
    assert_eq!(transient.status, AclStatus::Transient);
    assert!(transient.governing_acl.is_none());
    assert!(transient.acl_link_header().is_none());
}
