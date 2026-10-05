//! # sparq-trust — trust-graph authorisation admission gate (research PoC)
//!
//! The proof-of-concept for the trust-graph authorisation system (design record
//! `research/solid-trust-graph-authz-design.md` §6.0; issue #940, epic `sq-pfae`).
//! It adds the **admission stratum** ahead of the shipped `sparq-solid` WAC/ACP
//! derivation stratum: *"is this externally-attested fact from a source I trust for
//! this statement-type?"* — and, on success, injects the issuer-tagged fact into the
//! materialiser so the existing N3 reasoner can merge it with the `.acr` rules and
//! derive access.
//!
//! ```text
//!   attested statements (VC claims, signed graphs)
//!                 │
//!                 ▼
//!   ADMISSION stratum  (this crate, NEW, OPT-IN)
//!   gate: trust statement + verified issuer signature + freshness + holder binding
//!                 │  admitted facts (issuer-tagged, trust:admitted)
//!                 ▼
//!   DERIVATION stratum  (shipped sparq-solid WAC/ACP N3 rules + ABAC rule)
//!                 │  ".acr rules over the admitted facts ⇒ canAccess"
//!                 ▼
//!        <urn:sparq:auth>  (allow-list, fail-closed)
//! ```
//!
//! ## Modules
//!
//! - [`vocab`] — the `trust:` IRIs of the 10-term minimal core, plus the one
//!   `forPredicate → forShape` desugaring (sugar, not a primitive).
//! - [`policy`] — parse a Control-gated trust-policy graph into [`policy::TrustRule`]s
//!   (fail-closed: a policy not Control-gated admits nothing).
//! - [`mod@admit`] — the admission gate: canonicalise (RDFC-1.0), verify the issuer
//!   signature, enforce statement-type scoping (SHACL shape), freshness, and holder
//!   binding; output [`admit::AdmittedFact`]s. [`admit::admit_static`] runs only the
//!   session-independent (STATIC) class and defers holder/freshness to a per-request
//!   conditional-grant check — the `sq-xc4y` static/dynamic split.
//! - [`delegation`] — the **invocation-binding gate** (`sq-l5og`): verify a carried
//!   ZCAP/UCAN-style delegation chain, enforce monotone attenuation, and bind the
//!   **authenticated invoker == the chain's terminal delegate, key-proven per request**
//!   so an admitted delegation is **not replayable** under key substitution (the
//!   delegation analogue of §3.4 holder binding).
//! - [`wire`] — feed admitted facts into the N3 reasoner ahead of the materialiser;
//!   read off the derived `auth:*` grants ([`wire::derive_grants`]) OR the per-grant
//!   DYNAMIC conditions for a conditional grant ([`wire::derive_conditional_grants`]).
//! - `did` (OPT-IN, default-OFF `did` feature) — pluggable DID resolution
//!   (`did:key` offline self-cert + a pluggable `did:web` fetcher) that binds a
//!   `trust:issuerDid` to the issuer verifying key the gate already checks, narrowing
//!   the operator-asserted-key forgery vector D′ (`sq-pfae.3`). A plain code-span here
//!   (not an intra-doc link) so the default-feature doc build does not reference a
//!   feature-gated item.
//!
//! ## Opt-in by construction (strict additivity — design §2.2 G6)
//!
//! Nothing in the workspace's default build depends on this crate. `sparq-solid`
//! pulls it in ONLY behind its **default-OFF `trust-graph` cargo feature**; with the
//! feature off, `sparq-solid` behaves **exactly** as WAC/ACP do today — the only edge
//! it gains is one feature-gated call into this admission gate before the
//! materialiser. A pod with no trust graph is byte-identical to today's Solid.
//!
//! ## Honest scope — RESEARCH PROTOTYPE, NOT a security guarantee (read first)
//!
//! This is a research prototype to take to the LWS / Solid working groups, **not a
//! shipped feature and not a security guarantee**. It demonstrates exactly one
//! claim: *a single externally-attested fact is admitted through a
//! trusted-source-scoped gate, then merged by N3 reasoning with an `.acr` rule to
//! derive `canAccess`.* It proves nothing stronger. In particular:
//!
//! - **No privacy / unlinkability / anonymity.** The gate verifies a CHECKED issuer
//!   signature, but the credential is admitted **in the clear** — the verifier learns
//!   the exact value (`age 25`, not just "≥ 18"). It does **NOT** deliver ZKAPs-grade
//!   unlinkable/anonymous presentation. The ZK estate it composes with
//!   (`sparq-zk` / `sparq-zk-compose`) is research-grade and **externally UNAUDITED**
//!   — external accredited-cryptographer sign-off is **pending** (`sq-qhy4`), and
//!   `sparq-mpc` is honest-majority semi-honest only.
//! - **Holder binding is the clear-WebID, non-anonymous DEGRADED path.** §3.4's
//!   holder binding (`credentialSubject == Session.agent`) authenticates the
//!   requester's WebID **in the clear**. This is the documented degraded path
//!   (`sq-wvne` / `sq-xc4y`), **not** silently "solved": presentations stay linkable
//!   by requester identity. A ZKAPs-equivalent presentation would have to REPLACE
//!   clear-WebID holder binding with an in-ZK holder-PoP plus a nullifier — an
//!   architecture change, not a composition step.
//! - **Issuer keys are operator-asserted by default; DID-bindable opt-in (`sq-pfae.3`).**
//!   The default `trust:issuerKey → verifying-key` binding is supplied directly — the live
//!   forgery vector D′ (§3.3). The **opt-in `did` feature** lets a rule bind its key from a
//!   `trust:issuerDid` instead (the `did` module: `did:key` offline self-cert + a pluggable
//!   `did:web` fetcher), which **narrows** D′ but is no absolute anchor (`did:key` is
//!   self-certifying; `did:web` only as strong as host/TLS) — still not a fully end-to-end
//!   trust path.
//!
//! ## Open problems this PoC RESPECTS as documented limitations (never silently solved)
//!
//! - **`sq-xc4y`** — per-request holder-binding / freshness vs the
//!   session-independent materialise-once auth view: RESOLVED by the **static/dynamic
//!   admission split** (decision (a)). The static class (signature, statement-type
//!   scope, reserved-predicate guard, `trust:scope`) is session-independent and runs
//!   ONCE at materialise-time ([`admit::admit_static`]); the dynamic class (holder
//!   binding + freshness) is deferred to a **per-request conditional grant** re-checked
//!   at query time via the shipped sq-0q7n `auth:agent` / `auth:notAfter` path — so a
//!   stale or wrong-holder credential is denied per request WITHOUT a re-materialise
//!   and is never frozen into the view. (The combined [`admit::admit`] remains the
//!   single-request snapshot path.)
//! - **`sq-l5og`** — delegation invocation-binding: **SPECIFIED + enforced + tested**
//!   in [`delegation`]. The gate binds **authenticated invoker == the carried chain's
//!   terminal delegate, key-proven per request** (a fresh-challenge PoP with the
//!   delegate key bound into the signed `hop_message`), so an admitted delegation is
//!   **not replayable** under key substitution. It carries the chain WITH the invocation
//!   (object-capability discipline — resolves the §4.1↔§4.3 ambient-authority tension)
//!   and intersects against the **current** delegator grant (never a delegation-time
//!   snapshot). What stays open and is documented, not silently solved: deep-chain
//!   *incremental* revocation (full re-materialisation only — the §4.4 stale-authority
//!   window is bounded, not closed). DID-resolver key binding (`sq-pfae.3`) is now
//!   addressed by the opt-in `did` module (narrows, does not eliminate, forgery vector D′).
//! - **`sq-tu4e`** — no in-reasoner NAF over derived facts: `revoked` is an
//!   **input-only** per-request guard; there is **no deny-on-disagreement** rule.
//! - **`sq-wvne`** — ZKAPs-grade unlinkable presentation needs a 3-part ZK composite:
//!   **out of PoC scope** (no privacy/ZK).
//!
//! [OPUS-4.8] sq-pfae PoC (issue #940). Written while Fable unavailable — flag for
//! re-review when Fable returns. 🤖 SPARQ agent — trust-graph authorisation PoC.

#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]

pub mod admit;
pub mod delegation;
/// PROV-O delegation audit for human + AI agents (the P5 audit half, `sq-pfae.6`): render
/// an invocation-bound delegation chain + effective capability as a minimal W3C PROV-O
/// graph (`prov:actedOnBehalfOf` lineage + the effective grant as `auth:*` RDF), with the
/// human/AI principal classification the design §4.2 names as an attested attribute on the
/// delegate. Behind the default-OFF `delegation-prov` feature: nothing in the lean default
/// build depends on it (no new dependency — pure `oxrdf`, no `sparq-prov` edge).
#[cfg(feature = "delegation-prov")]
#[cfg_attr(docsrs, doc(cfg(feature = "delegation-prov")))]
pub mod delegation_prov;
#[cfg(feature = "did")]
#[cfg_attr(docsrs, doc(cfg(feature = "did")))]
pub mod did;
pub mod policy;
/// The N3 proof-admissibility ruleset over `sparq-reason` — the §4.3 ODRL →
/// admissible-proof-set reduction (`strongerThan`-closure / `atLeast` /
/// `overDimension` / `satisfies`) run as a runnable Notation3 ruleset, with the
/// Rust default-deny universal (`sq-ufsi9`, Phase 2). Behind the default-OFF
/// `secprop-admissibility` feature (which enables `secprop-vocab`): nothing in
/// the lean default build depends on it.
#[cfg(feature = "secprop-admissibility")]
#[cfg_attr(docsrs, doc(cfg(feature = "secprop-admissibility")))]
pub mod admissibility;
#[cfg(feature = "secprop-vocab")]
#[cfg_attr(docsrs, doc(cfg(feature = "secprop-vocab")))]
pub mod secprop;
/// The `trustx:` certification-scope vocabulary (`framework_vocab` module +
/// `trust-framework.ttl`) — the trust-expression program's layer for expressing
/// framework-certified-issuer trust and issuer certification scope over the
/// verifier→holder trust requirements (issue #1592; design record
/// `research/trust-expression-spec.md` §3.4 / D4). It EXTENDS the `trust:` vocabulary
/// (shared base IRI) and REFERENCES the vendored `sec-req:` eIDAS/UK-DVS individuals via
/// `rdfs:seeAlso` — it does not fork or duplicate them (the design's D5). Behind the
/// default-OFF `framework-vocab` feature: `const &str` data + a TTL drift test, no new
/// dependency, strictly additive.
#[cfg(feature = "framework-vocab")]
#[cfg_attr(docsrs, doc(cfg(feature = "framework-vocab")))]
pub mod framework_vocab;
/// Holder-side trust-expression contract evaluation, CLEAR path (`expression` module,
/// `sq-6syab.4`; issue #1592, design `research/trust-expression-spec.md` §3.1–3.5): parse
/// the verifier→holder request (SPARQL query + trust-requirements graph + nonce),
/// generate the §3.1 normative reference rewrite Q→Q' (conjoining issuer-membership /
/// positive-status-attestation-validity-at-t / certification-scope-conformance patterns),
/// evaluate Q' via `sparq-engine` over the holder's attested named-graph dataset, and
/// assemble the provenance-encoded response (RDF 1.2 reifier normative form + the
/// named-graph/PROV-O runnable-today mapping). Fail-closed: no admissible derivation ⇒ no
/// binding, and the response provenance suffices for an independent verifier re-check
/// (Q' over R — `verify_response` in plain code-span; feature-gated item). Behind the
/// default-OFF `expression` feature (which enables `framework-vocab` + `status-list` +
/// `did` + `secprop-admissibility` for the surfaces it REUSES, and pulls `sparq-engine` +
/// the vendored `spargebra` parser): nothing in the lean default build depends on it.
#[cfg(feature = "expression")]
#[cfg_attr(docsrs, doc(cfg(feature = "expression")))]
pub mod expression;
/// The depth-bounded certification-edge trust-graph closure (`graph` module): a pure,
/// **fail-closed, attenuation-only** pre-processing step that derives additional
/// `TrustRule`s from signed `trustx:Certification` edges AHEAD of — and consumed verbatim
/// by — the UNCHANGED admission gate. Every derived rule is `⊆ (anchor ∩ cert scope ∩
/// validity window)`: a certification can only NARROW, never widen, the certifier's own
/// authority, and any unverifiable / expired / cyclic / over-depth / broadening edge
/// contributes NOTHING; zero certifications ⇒ output is the direct rules byte-identical.
/// Behind the default-OFF `cert-graph` feature (which enables `framework-vocab` for the
/// vocabulary): pure Rust over the RDFC-1.0 signature primitive this crate already carries
/// — NO new dependency, strictly additive (`sq-pfae.15`). A plain code-span (not an
/// intra-doc link) for `TrustRule` so the default-feature doc build does not reference a
/// feature-gated item.
#[cfg(feature = "cert-graph")]
#[cfg_attr(docsrs, doc(cfg(feature = "cert-graph")))]
pub mod graph;
/// The trust-document storage / authoring model — server-wide vs per-`.acr` documents,
/// versioning, revocation, and the admission cache key that composes with the
/// sparq-solid epoch cache (the P4 model, `sq-pfae.5`). Behind the default-OFF `store`
/// feature: nothing in the lean default build depends on it.
#[cfg(feature = "store")]
#[cfg_attr(docsrs, doc(cfg(feature = "store")))]
pub mod store;
/// Live credential status / revocation over a W3C Bitstring Status List + a minimal PROV-O
/// denial justification (the P6 status stratum, `sq-pfae.7`): fetch (pluggable resolver) +
/// decode (multibase + a pluggable GZIP seam) a status list, gate derivations **fail-closed
/// on set/unknown/stale** status, and render a minimal `prov:`/`trust:` justification per
/// allow OR deny. Behind the default-OFF `status-list` feature (dependency-free; the nested
/// `status-list-flate2` feature adds the built-in `flate2` GZIP decoder): nothing in the lean
/// default build depends on it.
#[cfg(feature = "status-list")]
#[cfg_attr(docsrs, doc(cfg(feature = "status-list")))]
pub mod status_list;
pub mod vocab;
pub mod wire;

#[cfg(feature = "secprop-admissibility")]
pub use admissibility::{admissible, ruleset, Admissibility};
pub use admit::{
    admit, admit_static, AdmittedFact, PresentedCredential, Session, StaticAdmittedFact,
};
#[cfg(feature = "status-list")]
pub use admit::admit_with_status;
#[cfg(feature = "secprop-precheck")]
pub use admit::{
    admit_with_precheck, AdmissibilityConstraint, AdmissibilityPreference, PrecheckOutcome,
};
pub use delegation::{
    effective_against_current, hop_message, invoke, Capability, DelegationChain, DelegationHop,
    EffectiveCapability, Invocation, InvocationDenied,
};
#[cfg(feature = "delegation-prov")]
pub use delegation_prov::{
    audit_invocation, is_auth_action, AuditConfig, DelegationAudit, PrincipalClassification,
    PrincipalKind,
};
#[cfg(feature = "did")]
pub use did::{
    did_key_for, Did, DidDocumentFetcher, DidError, DidKeyResolver, DidResolver, DidWebResolver,
};
#[cfg(feature = "expression")]
pub use expression::{
    evaluate_contract, mint_status_attestation, parse_request, rewrite_query, verify_response,
    ChallengeNonce, ContractAnswer, ContractOutcome, ContractRequest, ExpressionError,
    MethodPrecheck, ProvenanceResponse, TrustRequirements,
};
#[cfg(feature = "cert-graph")]
pub use graph::{
    certification_message, derive_effective_rules, explain_edge, Certification, CertScope,
    EdgeRejection,
};
pub use policy::{parse_policy, ControlGate, PolicyError, ShapeRef, TrustRule};
#[cfg(feature = "did")]
pub use policy::{resolve_rule_keys, IssuerBinding};
// [SONNET-4.6] sq-0hu2w: re-export the sig primitives so downstream crates that need to
// parse/hold a `PublicKey` (e.g. callers constructing a `TrustRule.issuer_key` or calling
// the admission gate) do NOT need a direct `sparq-zk` dependency — `sparq-trust` is the
// single dependency chokepoint.
pub use sparq_zk::sig::{public_key_from_hex, PublicKey};
#[cfg(feature = "store")]
pub use store::{
    AdmissionCacheKey, PolicyVersion, StoreError, TrustDocument, TrustLayer, TrustStore,
};
#[cfg(feature = "status-list")]
pub use status_list::{
    decode_encoded_list, justify_status_decision, DeltaUnbounded, GzipDecoder, IdentityGzipDecoder,
    LiveStatus, LiveStatusCheck, SignedStatusList, StatusBitstring, StatusDelta,
    StatusJustification, StatusListEntry, StatusListResolver, StatusPurpose,
    VerifiedStatusListResolver, VerifyingLiveStatusCheck, DEFAULT_MAX_CHANGED_INDICES,
};
pub use wire::{derive_conditional_grants, derive_grants, ConditionalGrant};
