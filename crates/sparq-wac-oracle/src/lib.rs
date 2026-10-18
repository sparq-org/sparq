//! sq-t999y — **server-independent WAC/ACP decision test-vectors** plus a
//! runner that asserts `sparq-solid`'s per-resource decision surface
//! ([`PodStore::decide`] / [`PodStore::decide_batch`]) matches every vector.
//!
//! # What this is
//!
//! A protocol-agnostic **corpus**: two embedded N-Quads fixture pods (a WAC `.acl`
//! variant and an ACP `.acr` variant, derived from the `sparq_solid::fixture` seed's
//! policy-shape subtrees) and, for each, a list of spec-declared decision rows
//! `(session, resource, mode) → expected {allow, granted_modes, scope, status}`.
//! A row's session spans all four dimensions — `agent`, `client`, `issuer` (ACP
//! `acp:issuer`) and `now` — and an ACP fixture additionally declares the TRUSTED
//! [`ProvenanceFact`] rows its `acp:CreatorAgent`/`acp:OwnerAgent` matchers resolve
//! against. The `now` dimension has its own corpus behind the opt-in `odrl-bridge`
//! feature (`window_corpus`), since only the ODRL bridge mints a live-clock window.
//! The vectors are **data** — any Solid-protocol implementation (the PSS-parity
//! suite, the future `solid-server` landing of epic `sq-gg0qq`) can assert against
//! the same rows. [`run_vectors`] is the sparq-native runner: it evaluates each row
//! through [`PodStore::decide`], cross-checks [`PodStore::decide_batch`] parity, and
//! returns a structured [`OracleReport`] instead of panicking.
//!
//! # Invariant (result-equivalence)
//!
//! For every fixture row, `PodStore::decide(...)` equals the spec-declared expected
//! decision (allow / granted modes / scope / status) — the oracle is the fixed point
//! a Solid resource server built on sparq must match.
//!
//! # Status
//!
//! **EXPERIMENTAL** dev-only corpus (`publish = false`). The row schema
//! ([`Vector`] / [`Expected`]) is stable-intent; rows and fixtures may grow. It makes
//! no authentication claim — a [`Session`] is a caller-asserted claim, exactly as in
//! `sparq-solid`.
//!
//! # Example
//!
//! ```
//! // Every embedded fixture row must hold on the real decision engine.
//! for fixture in sparq_wac_oracle::fixtures() {
//!     let report = sparq_wac_oracle::run_fixture(fixture)?;
//!     assert!(report.passed(), "{report}");
//! }
//! # Ok::<(), String>(())
//! ```

#![forbid(unsafe_code)]

// sq-39kps — POST-`Slug` `.acl` escalation corpus (a self-contained
// decision-layer adversarial acceptance bar for the future POST chokepoint guard,
// bead sq-gg0qq.5). Disjoint from the B1 vectors above.
pub mod escalation_corpus;

// sq-x1ayt — time-windowed conditional-grant rows (the `Session::now`
// dimension). Behind the opt-in `odrl-bridge` feature: the live-clock window is minted
// ONLY by sparq-solid's ODRL bridge (ACP has no time vocabulary), so the default build
// keeps zero ODRL code and deps.
#[cfg(feature = "odrl-bridge")]
pub mod window_corpus;

use sparq_core::Graph;
pub use sparq_solid::{
    AccessProvenance, AclScope, AclStatus, Mode, PodStore, Session, WacDecision,
};
use std::fmt;

/// Which access-control system a [`Fixture`]'s control documents use, i.e. which
/// `sparq-solid` materializer [`build_store`] runs (`materialize_wac` for `.acl`
/// corpora, `materialize_acp_with` — fed the fixture's [`Fixture::provenance`] — for
/// `.acr` corpora).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum System {
    /// Web Access Control (`.acl` documents, nearest-ACL inheritance).
    Wac,
    /// Access Control Policy (`.acr` documents, cumulative inheritance).
    Acp,
}

/// The spec-declared expected decision for one vector row — the four fields the
/// bead's result-equivalence invariant fixes: `allow`, `granted_modes`, `scope`,
/// `status` (see [`WacDecision`] for the field semantics).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Expected {
    /// The verdict for the requested mode.
    pub allow: bool,
    /// Every mode the session holds on the resource, in the engine's canonical
    /// `Read, Write, Append, Control` order (deduped).
    pub granted_modes: &'static [Mode],
    /// `AccessTo` (own control document) / `Default` (inherited) / `None` when no
    /// governing control document exists (`NoAcl`) or the request is malformed.
    pub scope: Option<AclScope>,
    /// The fail-closed load/error status (`Resolved` / `NoAcl` / `Transient`; the
    /// corpus never expects `Unloaded` — [`build_store`] always materializes).
    pub status: AclStatus,
}

/// One decision test-vector: a `(session, resource, mode)` request plus its
/// [`Expected`] outcome. `agent`/`client`/`issuer`/`now` are the caller-asserted
/// session claim; [`Vector::session`] assembles the [`Session`].
#[derive(Debug, Clone, Copy)]
pub struct Vector {
    /// Short unique label, surfaced in [`Failure`] reports.
    pub name: &'static str,
    /// The session's WebID claim; `None` = anonymous.
    pub agent: Option<&'static str>,
    /// The session's client-identifier claim (WAC `acl:origin` / ACP `acp:client`).
    pub client: Option<&'static str>,
    /// The session's OIDC-issuer claim (ACP `acp:issuer`); `None` = no issuer asserted,
    /// which an issuer-constrained grant treats as fail-closed. Set it with
    /// [`Vector::via_issuer`].
    pub issuer: Option<&'static str>,
    /// The request instant as an `xsd:dateTime` lexical string; `None` = no clock, which
    /// a time-windowed conditional grant treats as fail-closed. Set it with
    /// [`Vector::at`].
    pub now: Option<&'static str>,
    /// The resource (named-graph IRI) being decided on.
    pub resource: &'static str,
    /// The requested access mode.
    pub mode: Mode,
    /// The spec-declared expected decision.
    pub expect: Expected,
}

impl Vector {
    /// The [`Session`] this vector's request is made as.
    pub fn session(&self) -> Session<'static> {
        Session {
            agent: self.agent,
            client: self.client,
            issuer: self.issuer,
            now: self.now,
        }
    }

    /// This vector with the session's OIDC-issuer claim bound (ACP `acp:issuer`).
    pub const fn via_issuer(self, issuer: &'static str) -> Vector {
        Vector {
            issuer: Some(issuer),
            ..self
        }
    }

    /// This vector with the request clock bound (mirrors [`Session::at`]), for a
    /// time-windowed conditional grant.
    pub const fn at(self, now: &'static str) -> Vector {
        Vector {
            now: Some(now),
            ..self
        }
    }
}

/// One TRUSTED per-resource creator/owner fact, the corpus's declarative form of an
/// [`AccessProvenance`] entry — the channel `acp:CreatorAgent` / `acp:OwnerAgent`
/// matchers resolve against. Never read from pod content (see [`AccessProvenance`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProvenanceFact {
    /// The resource (named-graph IRI) the fact is about.
    pub resource: &'static str,
    /// Its creator's WebID; `None` = unknown ⇒ no `CreatorAgent` grant can fire.
    pub creator: Option<&'static str>,
    /// Its owner's WebID; `None` = unknown ⇒ no `OwnerAgent` grant can fire.
    pub owner: Option<&'static str>,
}

/// One embedded fixture: a pod dataset (N-Quads) + the decision vectors that hold
/// on it under `system`'s semantics.
#[derive(Debug, Clone, Copy)]
pub struct Fixture {
    /// Corpus label (`"wac"` / `"acp"`).
    pub name: &'static str,
    /// Which materializer the pod's control documents require.
    pub system: System,
    /// The pod dataset — content graphs + `.acl`/`.acr` control-document graphs.
    pub nquads: &'static str,
    /// The TRUSTED creator/owner facts the pod's `acp:CreatorAgent`/`acp:OwnerAgent`
    /// matchers resolve against ([`build_store`] feeds them to
    /// [`PodStore::materialize_acp_with`]). Empty for a corpus with no provenance
    /// matchers; a WAC fixture must leave it empty (WAC has no creator/owner vocabulary).
    pub provenance: &'static [ProvenanceFact],
    /// The spec-declared decision rows.
    pub vectors: &'static [Vector],
}

/// One vector the store's decision did not match (or a `decide`-vs-`decide_batch`
/// divergence), with a human-readable field-level diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// The [`Vector::name`] of the failing row.
    pub vector: &'static str,
    /// What mismatched (expected-vs-got, per field).
    pub detail: String,
}

/// The outcome of [`run_vectors`] over one vector list: total rows checked and
/// every [`Failure`]. Empty `failures` ⇔ the store matches the corpus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OracleReport {
    /// Number of vectors evaluated.
    pub total: usize,
    /// Every mismatch found (empty = pass).
    pub failures: Vec<Failure>,
}

impl OracleReport {
    /// `true` iff every vector matched (no failures).
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }
}

impl fmt::Display for OracleReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.passed() {
            return write!(f, "oracle: {} vectors, all passed", self.total);
        }
        writeln!(
            f,
            "oracle: {}/{} vectors FAILED:",
            self.failures.len(),
            self.total
        )?;
        for fail in &self.failures {
            writeln!(f, "  [{}] {}", fail.vector, fail.detail)?;
        }
        Ok(())
    }
}

// ─── the embedded corpus ────────────────────────────────────────────────────────────

/// Principals shared by both fixtures (the `sparq_solid::fixture` seed cast).
pub const ALICE: &str = "https://alice.ex/card#me";
/// See [`ALICE`].
pub const BOB: &str = "https://bob.ex/card#me";
/// See [`ALICE`].
pub const CAROL: &str = "https://carol.ex/card#me";
/// See [`ALICE`].
pub const DAVE: &str = "https://dave.ex/card#me";
/// The client identifier used by the pair-principal grants.
pub const APP: &str = "https://app.ex";
/// The OIDC issuer the ACP `iss6` shape's `acp:issuer` matcher trusts.
pub const GOOD_IDP: &str = "https://good-idp.ex";
/// An untrusted OIDC issuer — every `iss6` row presenting it is denied.
pub const EVIL_IDP: &str = "https://evil-idp.ex";

const R: Mode = Mode::Read;
const W: Mode = Mode::Write;
const RWC: &[Mode] = &[Mode::Read, Mode::Write, Mode::Control];
const RW: &[Mode] = &[Mode::Read, Mode::Write];
const READ: &[Mode] = &[Mode::Read];
const WRITE: &[Mode] = &[Mode::Write];
const NONE: &[Mode] = &[];

/// A `Resolved` expectation with `Default` (inherited) discovery scope.
const fn resolved_default(allow: bool, granted: &'static [Mode]) -> Expected {
    Expected {
        allow,
        granted_modes: granted,
        scope: Some(AclScope::Default),
        status: AclStatus::Resolved,
    }
}
/// A `Resolved` expectation with `AccessTo` (own control document) discovery scope.
const fn resolved_access_to(allow: bool, granted: &'static [Mode]) -> Expected {
    Expected {
        allow,
        granted_modes: granted,
        scope: Some(AclScope::AccessTo),
        status: AclStatus::Resolved,
    }
}

/// Shorthand vector constructor (const-friendly).
const fn v(
    name: &'static str,
    agent: Option<&'static str>,
    client: Option<&'static str>,
    resource: &'static str,
    mode: Mode,
    expect: Expected,
) -> Vector {
    Vector {
        name,
        agent,
        client,
        issuer: None,
        now: None,
        resource,
        mode,
        expect,
    }
}

const FAIL_CLOSED_NO_ACL: Expected = Expected {
    allow: false,
    granted_modes: NONE,
    scope: None,
    status: AclStatus::NoAcl,
};
const FAIL_CLOSED_TRANSIENT: Expected = Expected {
    allow: false,
    granted_modes: NONE,
    scope: None,
    status: AclStatus::Transient,
};

/// WAC decision rows over [`WAC_NQUADS`] — nearest-ACL inheritance/shadowing,
/// `accessTo` vs `default` scope, group / public / authenticated / origin-pair
/// subjects, and the fail-closed rows.
static WAC_VECTORS: [Vector; 27] = [
    // priv0: owner-only via the root .acl's acl:default (inherited, Default scope).
    v(
        "wac-priv0-alice-read",
        Some(ALICE),
        None,
        "https://pod.ex/priv0/d0.ttl",
        R,
        resolved_default(true, RWC),
    ),
    v(
        "wac-priv0-alice-append-denied",
        Some(ALICE),
        None,
        "https://pod.ex/priv0/d0.ttl",
        Mode::Append,
        resolved_default(false, RWC),
    ),
    v(
        "wac-priv0-bob-read-denied",
        Some(BOB),
        None,
        "https://pod.ex/priv0/d0.ttl",
        R,
        resolved_default(false, NONE),
    ),
    v(
        "wac-priv0-anon-read-denied",
        None,
        None,
        "https://pod.ex/priv0/d0.ttl",
        R,
        resolved_default(false, NONE),
    ),
    // pub1: public read (foaf:Agent) + owner re-grant.
    v(
        "wac-pub1-anon-read",
        None,
        None,
        "https://pod.ex/pub1/d0.ttl",
        R,
        resolved_default(true, READ),
    ),
    v(
        "wac-pub1-dave-read",
        Some(DAVE),
        None,
        "https://pod.ex/pub1/d0.ttl",
        R,
        resolved_default(true, READ),
    ),
    v(
        "wac-pub1-alice-write",
        Some(ALICE),
        None,
        "https://pod.ex/pub1/d0.ttl",
        W,
        resolved_default(true, RWC),
    ),
    v(
        "wac-pub1-bob-write-denied",
        Some(BOB),
        None,
        "https://pod.ex/pub1/d0.ttl",
        W,
        resolved_default(false, READ),
    ),
    // team2: vcard group read+write; NO owner re-grant — nearest-ACL SHADOWS alice.
    v(
        "wac-team2-bob-write",
        Some(BOB),
        None,
        "https://pod.ex/team2/d0.ttl",
        W,
        resolved_default(true, RW),
    ),
    v(
        "wac-team2-carol-read",
        Some(CAROL),
        None,
        "https://pod.ex/team2/d0.ttl",
        R,
        resolved_default(true, RW),
    ),
    v(
        "wac-team2-alice-shadowed",
        Some(ALICE),
        None,
        "https://pod.ex/team2/d0.ttl",
        R,
        resolved_default(false, NONE),
    ),
    v(
        "wac-team2-dave-denied",
        Some(DAVE),
        None,
        "https://pod.ex/team2/d0.ttl",
        R,
        resolved_default(false, NONE),
    ),
    // friends3: bob acl:default-only (members, not the container); alice accessTo-only.
    v(
        "wac-friends3-bob-member-read",
        Some(BOB),
        None,
        "https://pod.ex/friends3/d0.ttl",
        R,
        resolved_default(true, READ),
    ),
    v(
        "wac-friends3-alice-member-denied",
        Some(ALICE),
        None,
        "https://pod.ex/friends3/d0.ttl",
        R,
        resolved_default(false, NONE),
    ),
    v(
        "wac-friends3-alice-container-read",
        Some(ALICE),
        None,
        "https://pod.ex/friends3/",
        R,
        resolved_access_to(true, RWC),
    ),
    v(
        "wac-friends3-bob-container-denied",
        Some(BOB),
        None,
        "https://pod.ex/friends3/",
        R,
        resolved_access_to(false, NONE),
    ),
    // mixed4: any AUTHENTICATED agent reads members; anonymous does not.
    v(
        "wac-mixed4-dave-authenticated-read",
        Some(DAVE),
        None,
        "https://pod.ex/mixed4/d0.ttl",
        R,
        resolved_default(true, READ),
    ),
    v(
        "wac-mixed4-anon-denied",
        None,
        None,
        "https://pod.ex/mixed4/d0.ttl",
        R,
        resolved_default(false, NONE),
    ),
    v(
        "wac-mixed4-alice-owner-read",
        Some(ALICE),
        None,
        "https://pod.ex/mixed4/d0.ttl",
        R,
        resolved_default(true, RWC),
    ),
    // mixed4/own.ttl: resource-specific OWN .acl — bob only, AccessTo scope, shadows all.
    v(
        "wac-own-bob-read",
        Some(BOB),
        None,
        "https://pod.ex/mixed4/own.ttl",
        R,
        resolved_access_to(true, READ),
    ),
    v(
        "wac-own-alice-shadowed",
        Some(ALICE),
        None,
        "https://pod.ex/mixed4/own.ttl",
        R,
        resolved_access_to(false, NONE),
    ),
    // origin5: bob ONLY through the app.ex client (acl:origin pair); alice unconstrained.
    v(
        "wac-origin5-bob-app-read",
        Some(BOB),
        Some(APP),
        "https://pod.ex/origin5/d0.ttl",
        R,
        resolved_default(true, READ),
    ),
    v(
        "wac-origin5-bob-no-client-denied",
        Some(BOB),
        None,
        "https://pod.ex/origin5/d0.ttl",
        R,
        resolved_default(false, NONE),
    ),
    v(
        "wac-origin5-bob-evil-client-denied",
        Some(BOB),
        Some("https://evil.ex"),
        "https://pod.ex/origin5/d0.ttl",
        R,
        resolved_default(false, NONE),
    ),
    v(
        "wac-origin5-carol-app-denied",
        Some(CAROL),
        Some(APP),
        "https://pod.ex/origin5/d0.ttl",
        R,
        resolved_default(false, NONE),
    ),
    // fail-closed: no ACL anywhere up the chain / malformed resource IRI.
    v(
        "wac-foreign-resource-noacl",
        Some(ALICE),
        None,
        "https://other.ex/x",
        R,
        FAIL_CLOSED_NO_ACL,
    ),
    v(
        "wac-malformed-iri-transient",
        Some(ALICE),
        None,
        "not a valid iri",
        R,
        FAIL_CLOSED_TRANSIENT,
    ),
];

/// ACP decision rows over [`ACP_NQUADS`] — CUMULATIVE inheritance (the key WAC
/// difference), allOf agent+client pairs, deny-overrides, noneOf conditional
/// grants, the `acp:issuer` dimension, the `acp:CreatorAgent`/`acp:OwnerAgent`
/// provenance matchers (resolved through [`ACP_PROVENANCE`]), and the fail-closed rows.
static ACP_VECTORS: [Vector; 35] = [
    // priv0: owner-only via the root .acr's memberAccessControl (Default discovery scope).
    v(
        "acp-priv0-alice-read",
        Some(ALICE),
        None,
        "https://pod.ex/priv0/d0.ttl",
        R,
        resolved_default(true, RWC),
    ),
    v(
        "acp-priv0-bob-denied",
        Some(BOB),
        None,
        "https://pod.ex/priv0/d0.ttl",
        R,
        resolved_default(false, NONE),
    ),
    // the root container itself: own .acr, accessControl policy → AccessTo scope.
    v(
        "acp-root-alice-read",
        Some(ALICE),
        None,
        "https://pod.ex/",
        R,
        resolved_access_to(true, RWC),
    ),
    v(
        "acp-root-anon-denied",
        None,
        None,
        "https://pod.ex/",
        R,
        resolved_access_to(false, NONE),
    ),
    // pub1: public read for members; alice KEEPS owner access (cumulative — no shadowing).
    v(
        "acp-pub1-anon-read",
        None,
        None,
        "https://pod.ex/pub1/d0.ttl",
        R,
        resolved_default(true, READ),
    ),
    v(
        "acp-pub1-dave-read",
        Some(DAVE),
        None,
        "https://pod.ex/pub1/d0.ttl",
        R,
        resolved_default(true, READ),
    ),
    v(
        "acp-pub1-alice-cumulative",
        Some(ALICE),
        None,
        "https://pod.ex/pub1/d0.ttl",
        W,
        resolved_default(true, RWC),
    ),
    // team2: anyOf enumerated agents read+write; alice still cumulative.
    v(
        "acp-team2-bob-write",
        Some(BOB),
        None,
        "https://pod.ex/team2/d0.ttl",
        W,
        resolved_default(true, RW),
    ),
    v(
        "acp-team2-carol-read",
        Some(CAROL),
        None,
        "https://pod.ex/team2/d0.ttl",
        R,
        resolved_default(true, RW),
    ),
    v(
        "acp-team2-alice-cumulative",
        Some(ALICE),
        None,
        "https://pod.ex/team2/d0.ttl",
        R,
        resolved_default(true, RWC),
    ),
    v(
        "acp-team2-dave-denied",
        Some(DAVE),
        None,
        "https://pod.ex/team2/d0.ttl",
        R,
        resolved_default(false, NONE),
    ),
    // friends3: allOf {agent bob} {client app.ex} — exactly the pair, nothing else.
    v(
        "acp-friends3-bob-app-read",
        Some(BOB),
        Some(APP),
        "https://pod.ex/friends3/d0.ttl",
        R,
        resolved_default(true, READ),
    ),
    v(
        "acp-friends3-bob-no-client-denied",
        Some(BOB),
        None,
        "https://pod.ex/friends3/d0.ttl",
        R,
        resolved_default(false, NONE),
    ),
    v(
        "acp-friends3-carol-app-denied",
        Some(CAROL),
        Some(APP),
        "https://pod.ex/friends3/d0.ttl",
        R,
        resolved_default(false, NONE),
    ),
    // mixed4: public read but DENY dave (deny-overrides).
    v(
        "acp-mixed4-dave-deny-overrides",
        Some(DAVE),
        None,
        "https://pod.ex/mixed4/d0.ttl",
        R,
        resolved_default(false, NONE),
    ),
    v(
        "acp-mixed4-anon-read",
        None,
        None,
        "https://pod.ex/mixed4/d0.ttl",
        R,
        resolved_default(true, READ),
    ),
    // origin5: public read EXCEPT carol (noneOf → per-session conditional grant).
    v(
        "acp-origin5-carol-noneof-denied",
        Some(CAROL),
        None,
        "https://pod.ex/origin5/d0.ttl",
        R,
        resolved_default(false, NONE),
    ),
    v(
        "acp-origin5-anon-read",
        None,
        None,
        "https://pod.ex/origin5/d0.ttl",
        R,
        resolved_default(true, READ),
    ),
    v(
        "acp-origin5-dave-read",
        Some(DAVE),
        None,
        "https://pod.ex/origin5/d0.ttl",
        R,
        resolved_default(true, READ),
    ),
    // iss6: allOf {agent bob} {issuer good-idp} — the acp:issuer dimension. The grant
    // rides a minted (agent, client, issuer) triple principal, so it fires ONLY for the
    // exact issuer; an absent issuer claim is fail-closed, never issuer-agnostic.
    v(
        "acp-iss6-bob-good-idp-read",
        Some(BOB),
        None,
        "https://pod.ex/iss6/d0.ttl",
        R,
        resolved_default(true, READ),
    )
    .via_issuer(GOOD_IDP),
    v(
        "acp-iss6-bob-evil-idp-denied",
        Some(BOB),
        None,
        "https://pod.ex/iss6/d0.ttl",
        R,
        resolved_default(false, NONE),
    )
    .via_issuer(EVIL_IDP),
    v(
        "acp-iss6-bob-no-issuer-denied",
        Some(BOB),
        None,
        "https://pod.ex/iss6/d0.ttl",
        R,
        resolved_default(false, NONE),
    ),
    v(
        "acp-iss6-carol-good-idp-denied",
        Some(CAROL),
        None,
        "https://pod.ex/iss6/d0.ttl",
        R,
        resolved_default(false, NONE),
    )
    .via_issuer(GOOD_IDP),
    // an issuer-UNCONSTRAINED grant (alice's, from the root) ignores the dimension:
    // presenting an untrusted issuer neither widens nor narrows it.
    v(
        "acp-iss6-alice-issuer-agnostic",
        Some(ALICE),
        None,
        "https://pod.ex/iss6/d0.ttl",
        R,
        resolved_default(true, RWC),
    )
    .via_issuer(EVIL_IDP),
    // prov7: acp:CreatorAgent grants Read, acp:OwnerAgent grants Write, both resolved
    // from the TRUSTED ACP_PROVENANCE channel. d0: creator bob, owner carol. d1: creator
    // carol, owner UNKNOWN.
    v(
        "acp-prov7-bob-creator-read-d0",
        Some(BOB),
        None,
        "https://pod.ex/prov7/d0.ttl",
        R,
        resolved_default(true, READ),
    ),
    v(
        "acp-prov7-bob-creator-write-d0-denied",
        Some(BOB),
        None,
        "https://pod.ex/prov7/d0.ttl",
        W,
        resolved_default(false, READ),
    ),
    // resource-SCOPED: bob created d0, so he is granted nothing on d1.
    v(
        "acp-prov7-bob-read-d1-denied",
        Some(BOB),
        None,
        "https://pod.ex/prov7/d1.ttl",
        R,
        resolved_default(false, NONE),
    ),
    v(
        "acp-prov7-carol-owner-write-d0",
        Some(CAROL),
        None,
        "https://pod.ex/prov7/d0.ttl",
        W,
        resolved_default(true, WRITE),
    ),
    // owner ≠ creator: carol owns d0 but did not create it, so she has no Read there.
    v(
        "acp-prov7-carol-read-d0-denied",
        Some(CAROL),
        None,
        "https://pod.ex/prov7/d0.ttl",
        R,
        resolved_default(false, WRITE),
    ),
    v(
        "acp-prov7-carol-creator-read-d1",
        Some(CAROL),
        None,
        "https://pod.ex/prov7/d1.ttl",
        R,
        resolved_default(true, READ),
    ),
    // fail-closed: d1 has NO owner fact, so the OwnerAgent matcher never fires there.
    v(
        "acp-prov7-carol-write-d1-denied",
        Some(CAROL),
        None,
        "https://pod.ex/prov7/d1.ttl",
        W,
        resolved_default(false, READ),
    ),
    v(
        "acp-prov7-dave-read-d0-denied",
        Some(DAVE),
        None,
        "https://pod.ex/prov7/d0.ttl",
        R,
        resolved_default(false, NONE),
    ),
    // anonymous has no WebID, so it can never be a creator/owner.
    v(
        "acp-prov7-anon-read-d0-denied",
        None,
        None,
        "https://pod.ex/prov7/d0.ttl",
        R,
        resolved_default(false, NONE),
    ),
    // alice is neither creator nor owner, but keeps her cumulative root grant.
    v(
        "acp-prov7-alice-cumulative",
        Some(ALICE),
        None,
        "https://pod.ex/prov7/d0.ttl",
        R,
        resolved_default(true, RWC),
    ),
    // fail-closed: no control document anywhere up the chain.
    v(
        "acp-foreign-resource-noacl",
        Some(BOB),
        None,
        "https://other.ex/x",
        R,
        FAIL_CLOSED_NO_ACL,
    ),
];

/// The TRUSTED creator/owner facts the [`ACP_NQUADS`] `prov7` shape's
/// `acp:CreatorAgent`/`acp:OwnerAgent` matchers resolve against. `prov7/d1.ttl`
/// deliberately carries NO owner: the fail-closed row asserts the `OwnerAgent` matcher
/// never fires without one.
pub static ACP_PROVENANCE: [ProvenanceFact; 2] = [
    ProvenanceFact {
        resource: "https://pod.ex/prov7/d0.ttl",
        creator: Some(BOB),
        owner: Some(CAROL),
    },
    ProvenanceFact {
        resource: "https://pod.ex/prov7/d1.ttl",
        creator: Some(CAROL),
        owner: None,
    },
];

/// The embedded WAC pod (N-Quads).
pub const WAC_NQUADS: &str = include_str!("../fixtures/wac.nq");
/// The embedded ACP pod (N-Quads).
pub const ACP_NQUADS: &str = include_str!("../fixtures/acp.nq");

static FIXTURES: [Fixture; 2] = [
    Fixture {
        name: "wac",
        system: System::Wac,
        nquads: WAC_NQUADS,
        provenance: &[],
        vectors: &WAC_VECTORS,
    },
    Fixture {
        name: "acp",
        system: System::Acp,
        nquads: ACP_NQUADS,
        provenance: &ACP_PROVENANCE,
        vectors: &ACP_VECTORS,
    },
];

/// Every embedded fixture (currently the WAC and ACP pods).
pub fn fixtures() -> &'static [Fixture] {
    &FIXTURES
}

// ─── the runner ─────────────────────────────────────────────────────────────────────

/// Load a fixture's pod into a [`PodStore`] and materialize its auth view with the
/// matching `sparq-solid` materializer — [`PodStore::materialize_acp_with`] for an ACP
/// corpus, so `acp:CreatorAgent`/`acp:OwnerAgent` matchers resolve against the fixture's
/// TRUSTED [`Fixture::provenance`] facts (an empty list is exactly `materialize_acp` on a
/// fresh store, i.e. fully fail-closed).
///
/// # Errors
///
/// If the fixture's N-Quads fail to load, if materialization fails, or if a **WAC**
/// fixture declares provenance facts — WAC has no creator/owner vocabulary, so the
/// materializer would silently ignore them and the rows would assert nothing.
pub fn build_store(fixture: &Fixture) -> Result<PodStore, String> {
    let graph = Graph::load_dataset(fixture.nquads, "nquads")?;
    let mut store = PodStore::new(graph);
    match fixture.system {
        System::Wac if !fixture.provenance.is_empty() => {
            return Err(format!(
                "fixture {}: WAC has no creator/owner vocabulary, so provenance facts \
                 would be silently ignored",
                fixture.name
            ));
        }
        System::Wac => store.materialize_wac()?,
        System::Acp => store.materialize_acp_with(&access_provenance(fixture.provenance))?,
    };
    Ok(store)
}

/// The corpus's declarative [`ProvenanceFact`] rows as the `sparq-solid`
/// [`AccessProvenance`] map [`build_store`] materializes with.
pub fn access_provenance(facts: &[ProvenanceFact]) -> AccessProvenance {
    let mut prov = AccessProvenance::new();
    for fact in facts {
        if let Some(creator) = fact.creator {
            prov.set_creator(fact.resource, creator);
        }
        if let Some(owner) = fact.owner {
            prov.set_owner(fact.resource, owner);
        }
    }
    prov
}

/// Evaluate every vector against `store` and return a structured [`OracleReport`].
///
/// Two checks per row: (1) [`PodStore::decide`] must equal the [`Expected`] fields
/// (`allow`, `granted_modes`, `scope`, `status`); (2) [`PodStore::decide_batch`]
/// (grouped per session, in row order) must return element-for-element the same
/// [`WacDecision`]s as the singleton calls — batch/singleton parity is part of the
/// oracle. The report collects ALL failures; it never panics, so a consumer can
/// print the full field-level diff.
pub fn run_vectors(store: &PodStore, vectors: &[Vector]) -> OracleReport {
    let mut failures = Vec::new();

    // 1) per-row decide() vs the expected fields.
    let singles: Vec<WacDecision> = vectors
        .iter()
        .map(|vector| store.decide(&vector.session(), vector.resource, vector.mode))
        .collect();
    for (vector, got) in vectors.iter().zip(&singles) {
        if let Some(detail) = diff(vector, got) {
            failures.push(Failure {
                vector: vector.name,
                detail,
            });
        }
    }

    // 2) decide_batch parity: group rows by session (order-preserving), one batch per
    //    session, each element must equal the singleton decision. The key spans ALL FOUR
    //    session dimensions — two rows sharing an agent/client but differing in issuer or
    //    clock are DIFFERENT sessions, and batching them together would silently decide
    //    them under the wrong one.
    type SessionKey = (
        Option<&'static str>,
        Option<&'static str>,
        Option<&'static str>,
        Option<&'static str>,
    );
    let mut groups: Vec<(SessionKey, Vec<usize>)> = Vec::new();
    for (i, vector) in vectors.iter().enumerate() {
        let key = (vector.agent, vector.client, vector.issuer, vector.now);
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, idxs)) => idxs.push(i),
            None => groups.push((key, vec![i])),
        }
    }
    for (_, idxs) in &groups {
        let session = vectors[idxs[0]].session();
        let requests: Vec<(&str, Mode)> = idxs
            .iter()
            .map(|&i| (vectors[i].resource, vectors[i].mode))
            .collect();
        let batch = store.decide_batch(&session, &requests);
        for (&i, batched) in idxs.iter().zip(&batch) {
            if *batched != singles[i] {
                failures.push(Failure {
                    vector: vectors[i].name,
                    detail: format!(
                        "decide_batch diverges from decide: batch={batched:?} single={:?}",
                        singles[i]
                    ),
                });
            }
        }
    }

    OracleReport {
        total: vectors.len(),
        failures,
    }
}

/// [`build_store`] + [`run_vectors`] over one fixture.
pub fn run_fixture(fixture: &Fixture) -> Result<OracleReport, String> {
    let store = build_store(fixture)?;
    Ok(run_vectors(&store, fixture.vectors))
}

/// Run every embedded fixture; `Err` on a load/materialize failure, otherwise one
/// `(fixture-name, report)` per fixture (a report may still contain failures —
/// check [`OracleReport::passed`]).
pub fn run_all() -> Result<Vec<(&'static str, OracleReport)>, String> {
    fixtures()
        .iter()
        .map(|f| Ok((f.name, run_fixture(f)?)))
        .collect()
}

/// Field-level expected-vs-got diff for one row (`None` = match).
fn diff(vector: &Vector, got: &WacDecision) -> Option<String> {
    let expect = &vector.expect;
    let mut parts = Vec::new();
    if got.allow != expect.allow {
        parts.push(format!(
            "allow: expected {}, got {}",
            expect.allow, got.allow
        ));
    }
    if got.granted_modes != expect.granted_modes {
        parts.push(format!(
            "granted_modes: expected {:?}, got {:?}",
            expect.granted_modes, got.granted_modes
        ));
    }
    if got.scope != expect.scope {
        parts.push(format!(
            "scope: expected {:?}, got {:?}",
            expect.scope, got.scope
        ));
    }
    if got.status != expect.status {
        parts.push(format!(
            "status: expected {:?}, got {:?}",
            expect.status, got.status
        ));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── the acceptance run: every embedded fixture row holds on the real engine ───

    #[test]
    fn wac_fixture_all_vectors_pass() {
        let fixture = &fixtures()[0];
        assert_eq!(fixture.system, System::Wac);
        let report = run_fixture(fixture).expect("wac fixture builds");
        assert!(report.passed(), "{report}");
        assert_eq!(report.total, WAC_VECTORS.len());
    }

    #[test]
    fn acp_fixture_all_vectors_pass() {
        let fixture = &fixtures()[1];
        assert_eq!(fixture.system, System::Acp);
        let report = run_fixture(fixture).expect("acp fixture builds");
        assert!(report.passed(), "{report}");
        assert_eq!(report.total, ACP_VECTORS.len());
    }

    #[test]
    fn run_all_covers_every_fixture() {
        let reports = run_all().expect("all fixtures build");
        assert_eq!(reports.len(), fixtures().len());
        for (name, report) in &reports {
            assert!(report.passed(), "fixture {name}: {report}");
        }
    }

    // ─── non-vacuity: the runner DETECTS mismatches (mutation witnesses) ───────────

    #[test]
    fn runner_flags_wrong_expected_allow() {
        // Flip one expected verdict → run_vectors must report exactly that row.
        let store = build_store(&fixtures()[0]).expect("builds");
        let mut wrong = WAC_VECTORS[0];
        assert!(wrong.expect.allow, "precondition: row expects allow");
        wrong.expect.allow = false;
        let report = run_vectors(&store, &[wrong]);
        assert!(!report.passed(), "a flipped expectation must fail");
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].vector, wrong.name);
        assert!(
            report.failures[0].detail.contains("allow"),
            "{}",
            report.failures[0].detail
        );
    }

    #[test]
    fn runner_flags_unmaterialized_store() {
        // A store that never materialized decides Unloaded — every Resolved row must
        // fail (fail-closed visibility: the oracle cannot silently "pass" on an
        // unloaded auth view).
        let fixture = &fixtures()[0];
        let graph = Graph::load_dataset(fixture.nquads, "nquads").expect("loads");
        let store = PodStore::new(graph); // NOTE: no materialize
        let report = run_vectors(&store, fixture.vectors);
        assert!(!report.passed());
        // Every row expecting Resolved must be flagged (only NoAcl/Transient rows
        // are status-independent of materialization).
        let resolved_rows = fixture
            .vectors
            .iter()
            .filter(|v| v.expect.status == AclStatus::Resolved)
            .count();
        assert!(
            report.failures.len() >= resolved_rows,
            "expected >= {resolved_rows} failures, got {}",
            report.failures.len()
        );
    }

    // ─── direct unit tests per public fn/type (coverage ratchet) ───────────────────

    #[test]
    fn fixtures_exposes_wac_and_acp() {
        let all = fixtures();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].name, "wac");
        assert_eq!(all[1].name, "acp");
        assert!(all[0].nquads.contains(".acl") && !all[0].nquads.contains(".acr"));
        assert!(all[1].nquads.contains(".acr") && !all[1].nquads.contains(".acl"));
        assert!(!all[0].vectors.is_empty() && !all[1].vectors.is_empty());
    }

    #[test]
    fn vector_session_assembles_claims() {
        let vector = v(
            "s",
            Some(ALICE),
            Some(APP),
            "https://pod.ex/x",
            Mode::Read,
            FAIL_CLOSED_NO_ACL,
        );
        let session = vector.session();
        assert_eq!(session.agent, Some(ALICE));
        assert_eq!(session.client, Some(APP));
        // `v` leaves the issuer/clock dimensions unset — an issuer-constrained or
        // time-windowed grant treats both as fail-closed.
        assert_eq!(session.issuer, None);
        assert_eq!(session.now, None);
    }

    #[test]
    fn via_issuer_and_at_bind_the_remaining_session_dimensions() {
        let base = v(
            "s",
            Some(ALICE),
            Some(APP),
            "https://pod.ex/x",
            Mode::Read,
            FAIL_CLOSED_NO_ACL,
        );
        let bound = base.via_issuer(GOOD_IDP).at("2026-06-17T09:00:00Z");
        let session = bound.session();
        assert_eq!(session.issuer, Some(GOOD_IDP));
        assert_eq!(session.now, Some("2026-06-17T09:00:00Z"));
        // the other fields are carried through untouched
        assert_eq!(session.agent, Some(ALICE));
        assert_eq!(session.client, Some(APP));
        assert_eq!(bound.name, base.name);
        assert_eq!(bound.resource, base.resource);
        // …and the base row is unchanged (both builders are by-value).
        assert_eq!(base.issuer, None);
        assert_eq!(base.now, None);
    }

    #[test]
    fn access_provenance_maps_only_the_supplied_facts() {
        let prov = access_provenance(&ACP_PROVENANCE);
        assert_eq!(prov.creator("https://pod.ex/prov7/d0.ttl"), Some(BOB));
        assert_eq!(prov.owner("https://pod.ex/prov7/d0.ttl"), Some(CAROL));
        assert_eq!(prov.creator("https://pod.ex/prov7/d1.ttl"), Some(CAROL));
        // d1 declares no owner, so the OwnerAgent matcher stays fail-closed there…
        assert_eq!(prov.owner("https://pod.ex/prov7/d1.ttl"), None);
        // …and an undeclared resource knows neither.
        assert_eq!(prov.creator("https://pod.ex/prov7/d2.ttl"), None);
        assert_eq!(prov.owner("https://pod.ex/prov7/d2.ttl"), None);
        // an empty list is exactly the fully fail-closed map.
        assert!(access_provenance(&[]).is_empty());
    }

    #[test]
    fn build_store_rejects_provenance_on_a_wac_fixture() {
        // WAC has no creator/owner vocabulary, so the materializer would silently ignore
        // the facts — fail loudly instead of shipping rows that assert nothing.
        let bad = Fixture {
            name: "wac-with-provenance",
            system: System::Wac,
            nquads: WAC_NQUADS,
            provenance: &ACP_PROVENANCE,
            vectors: &[],
        };
        let Err(err) = build_store(&bad) else {
            panic!("WAC + provenance must be rejected");
        };
        assert!(err.contains("wac-with-provenance"), "{err}");
        // the same fixture without provenance builds fine.
        assert!(build_store(&Fixture {
            provenance: &[],
            ..bad
        })
        .is_ok());
    }

    #[test]
    fn build_store_rejects_garbage_nquads() {
        let bad = Fixture {
            name: "bad",
            system: System::Wac,
            nquads: "this is not nquads",
            provenance: &[],
            vectors: &[],
        };
        assert!(build_store(&bad).is_err());
    }

    #[test]
    fn report_display_lists_failures() {
        let ok = OracleReport {
            total: 3,
            failures: vec![],
        };
        assert!(ok.passed());
        assert!(ok.to_string().contains("all passed"));
        let bad = OracleReport {
            total: 3,
            failures: vec![Failure {
                vector: "row-x",
                detail: "allow: expected true, got false".into(),
            }],
        };
        assert!(!bad.passed());
        let text = bad.to_string();
        assert!(text.contains("row-x") && text.contains("allow"), "{text}");
    }
}
