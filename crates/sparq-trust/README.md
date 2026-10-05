<!-- [OPUS-4.8] sq-pfae PoC (issue #940). 🤖 SPARQ agent — trust-graph authorisation PoC. -->
# sparq-trust

<p>
  <a href="../../LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT"></a>
</p>

The **proof-of-concept** for trust-graph authorisation over [sparq-solid](../sparq-solid/README.md)
(design record §6.0; [issue #940](https://github.com/sparq-org/sparq/issues/940); epic `sq-pfae`). It
adds the **admission stratum** ahead of the shipped WAC/ACP derivation stratum: *"is this
externally-attested fact from a source I trust for this statement-type?"* — and on success injects
the issuer-tagged fact so the existing N3 reasoner merges it with the `.acr` rules to derive access.

> **RESEARCH PROTOTYPE — NOT a shipped feature and NOT a security guarantee.** It demonstrates
> exactly one claim: a single externally-attested fact is admitted through a
> trusted-source-scoped gate, then merged by N3 reasoning with an `.acr` rule to derive
> `canAccess`. It does **not** provide privacy, unlinkability, or anonymity (see *Honest scope*).
> The ZK estate it composes with is externally **unaudited** (`sq-qhy4`), pending
> accredited-cryptographer sign-off.

<!-- separate blockquote (MD028): the model-provenance note is a distinct callout. [OPUS-4.8] -->

> Model: Opus 4.8 (Fable unavailable — flag for re-review when Fable returns).

## 🚀 Quickstart

`sparq-trust` is **opt-in**: nothing in the default build depends on it; the gate is wired into `sparq-solid` behind its default-OFF `trust-graph` cargo feature.

```rust
use sparq_trust::admit::{admit, PresentedCredential, Session};
use sparq_trust::policy::{parse_policy, ControlGate};
use oxrdf::NamedNode;

# fn demo(cred: &PresentedCredential, policy: &[oxrdf::Triple]) {
// Parse a Control-gated trust policy → rules (fail-closed: an un-gated policy admits nothing).
let rules = parse_policy(policy, ControlGate::assert_control_gated()).unwrap();

// The admission gate: RDFC-1.0 canonicalise → CHECKED issuer signature → SHACL statement-type
// scoping → freshness → clear-WebID holder binding. Output: the issuer-tagged admitted facts.
let session = Session { agent: NamedNode::new("https://jesse.ex/card#me").unwrap(), now_unix_secs: 1_700_000_000 };
let target = NamedNode::new("https://pod.ex/resourceX").unwrap();
let admitted = admit(cred, &rules, &session, &target);
# let _ = admitted;
# }
```

From `sparq-solid` (`--features trust-graph`), `PodStore::admit_trust_credential_with_rule` runs the
gate and installs the derived `auth:*` grants into `<urn:sparq:auth>` on top of the unchanged WAC/ACP
view; `--features trust-graph-did` forwards the DID issuer-key binding (`sq-pfae.3`).

## ✨ Features

- **The minimal `trust:` ontology** (`vocab`) — the 10-term core (`TrustPolicy`, `TrustRule`,
  `trustsSourceFor`, `source`, `forShape`, `Source`, `issuerKey`, `scope`, `freshWithin`,
  `admitted`) + `issuerDid` + the `forPredicate → forShape` desugaring. Published as Turtle
  ([`trust.ttl`](ontologies/trust/trust.ttl), [`SEMANTICS.md`](ontologies/trust/SEMANTICS.md)), pinned to the
  Rust constants by a sync test. **All `trust:` IRIs are NON-STANDARD** — a WG would rehome them.
- **Fail-closed policy parsing** (`policy`) — a trust policy not presented through the
  Control-gated channel admits nothing (a type-level `ControlGate`). Accepts the reified
  `trust:TrustRule` form AND the claim-level `trust:trustsSourceFor` relational form (per-(source,
  statement-type) trust — the compact replacement for ACP's type-only `acp:vc`, `sq-pfae.4`).
- **The admission gate** (`admit`) — canonicalise (`sparq-canon` RDFC-1.0), verify the **checked** issuer
  signature over the commitment (`sparq-zk`, never a self-asserted triple), enforce statement-type scoping
  via a real SHACL shape (`sparq-shacl`), freshness, and the clear-WebID holder binding. Default-deny.
- **N3 merge** (`wire`) — feed admitted facts into `sparq-reason`; the `.acr` ABAC rule derives the grant (age>18 e2e).
- **Storage / authoring model** (`store`, **opt-in `store` feature**, `sq-pfae.5`) — a server-wide default + per-`.acr` documents that **NARROW, never broaden**
  the ceiling (`effective_rules`); monotone versioning (stale rejected) + revocation; `AdmissionCacheKey` composes with the sparq-solid epoch cache. No new dep.
- **Static / dynamic admission split** (`admit_static` + `derive_conditional_grants`, `sq-xc4y`) — decides the
  **session-independent** class once at materialise-time and defers the **per-request** class (holder + freshness)
  to an `auth:ConditionalGrant` re-checked per request.
- **The invocation-binding gate** (`delegation`, `sq-l5og`) — verify a carried ZCAP/UCAN-style delegation chain (each hop a
  CHECKED delegator signature over delegator/delegate **keys** + capability + expiry), enforce monotone attenuation
  (`child ⊆ parent`), and bind **invoker == terminal delegate, key-proven per request** via a fresh-challenge PoP.
- **Delegation PROV-O audit** (`delegation_prov`, **opt-in `delegation-prov`**, `sq-pfae.6`) — render an invocation-bound
  chain as a minimal W3C PROV-O graph (`prov:actedOnBehalfOf` per hop, human/AI principal, effective grant as `auth:*` RDF).
- **DID issuer-key binding** (`did`, **opt-in `did` feature**, `sq-pfae.3`) — a rule may name its issuer by
  `trust:issuerDid` instead of `trust:issuerKey` hex (`DidKeyResolver` decodes `did:key` offline; `DidWebResolver`
  reads `did:web` via a **pluggable** fetcher). **Narrows** the forgery vector D′ (no absolute anchor).
- **Security properties** (`secprop` / `admissibility`, **opt-in `secprop-vocab` / `secprop-admissibility`**, `sq-5oru9` / `sq-ufsi9`) — the sparq **`sec-prop:` extension**
  ([`secprop-ext.ttl`](../sparq-secprop-vocab/ontologies/secprop-ext.ttl), owned by the dependency-free [`sparq-secprop-vocab`](../sparq-secprop-vocab) leaf since [#3705](https://github.com/sparq-org/sparq/issues/3705); proof-system dimensions + the **assurance / audit-status axis** the vendored ontology lacks) and the §4.3 ODRL → admissible-proof-set reduction as a RUNNABLE N3 ruleset on `sparq-reason` (Rust **default-deny**). Reasons over ANNOTATIONS, not crypto (`sq-qhy4`). Three VENDORED dimension IRIs used as `secx:property` values (`SEC_PROP_POST_QUANTUM_FORGERY` / `_SNOOPING` / `SEC_PROP_SIGNATURE_TYPE_LEAKAGE`) are re-asserted verbatim as `sec-prop:SecurityProperty` subjects and listed in `ALL_SECPROP_IRIS` ([#3441](https://github.com/sparq-org/sparq/issues/3441)).
- **`trustx:` certification-scope vocabulary** (`framework_vocab`, **opt-in `framework-vocab`**, `sq-6syab.2` / [#1592](https://github.com/sparq-org/sparq/issues/1592)) —
  the trust-expression layer for **framework-certified-issuer** trust: a verifier→holder trust-requirements graph, two modes (enumerated `trustsIssuer` OR framework-certified), positive status attestation. Turtle ([`trust-framework.ttl`](ontologies/trust/trust-framework.ttl)) extends `trust:` + references vendored `sec-req:` eIDAS/UK-DVS individuals (no fork). **Anchored, not proven** (`sq-qhy4`).
- **Holder-side trust-expression evaluation + conformance suite** (`expression`, **opt-in `expression`**, `sq-6syab.4`/`.3`/`.6` / [#1592](https://github.com/sparq-org/sparq/issues/1592)) —
  the CLEAR-path verifier→holder contract: `parse_request` (query `Q` + trust-requirements graph `TR` + nonce), the §3.1 reference rewrite `Q → Q'` (issuer membership, positive status-attestation validity at *t*, certification-scope conformance; the two modes compose by `UNION`), evaluation via `sparq-engine`, and a provenance-encoded response in BOTH design-§4 encodings (RDF 1.2 reifier + the runnable-today named-graph/PROV-O mapping `verify_response` re-checks). **Fail-closed:** no admissible derivation ⇒ no binding AND zero disclosed bundles. The W3C-manifest [conformance suite](tests/trust-expression/manifest.ttl) drives all ten cases of design §6 through that API, with zero known-failing entries. Spec-conformance only — **not** a soundness or privacy claim (`sq-qhy4`).
- **Certification-edge trust-graph closure + store composition** (`graph` + `store`, **opt-in `cert-graph`**, `sq-pfae.15`/`sq-pfae.16`) — `derive_effective_rules`: depth-bounded (v1 depth-1), **attenuation-ONLY, fail-closed** closure from signed `trustx:Certification` edges AHEAD of the UNCHANGED admit gate. `TrustDocument::with_certifications` attaches edges to the document; `TrustStore::effective_rules_at` pipes them through the SAME server-ceiling + per-`.acr` narrowing path. **Cache-safety:** `policy_version` folds certifier IRI + certified-issuer IRI + validity window per edge — revoking a cert OR re-authoring its validity window changes `AdmissionCacheKey`. Wall-clock expiry of an unchanged cert propagates by re-materialise / epoch-bump, bounded by the host epoch cadence (residue tracked by `sq-l5og`). Zero certs ⇒ byte-identical to `TrustDocument::new`.
- **Property-admissibility pre-check** (`admit_with_precheck`, **opt-in `secprop-precheck`**, `sq-dt5hv` Ph 5 / `sq-nrwqs` / `sq-ddbm8`) — an OPTIONAL pre-admission check: the caller
  passes ONLY the requester's ODRL preference + the method IRI; the gate resolves the method's posture from the **bundled** `secprop-methods.ttl` (tamper-resistant; unknown method / malformed operand fail closed) and **fails closed** *before* the sig/holder checks. NO preference ⇒ **byte-identical** to `admit`.
- **Live status / revocation** (`status_list`, **opt-in `status-list`**, `sq-pfae.7`) — gate derivations on a **live W3C Bitstring Status List** instead of `revoked: bool`: fetch
  (pluggable `StatusListResolver`) + decode (multibase over a pluggable `GzipDecoder`; built-in `Flate2GzipDecoder` behind `status-list-flate2`); `admit_with_status` admits ONLY a `LiveStatus::Live` credential — **fail-closed on set/unknown/stale** — with a minimal `prov:`/`trust:` justification. On an epoch bump `StatusDelta::between` (`sq-pfae.14`) names the changed slots so the caller re-runs the UNCHANGED gate over only the affected grants — a *selection* over two input snapshots, never an in-reasoner retraction. Skip only under `valid_at(now, max_age) && !affects(entry)`; a not-newer / coverage-changed / over-limit delta demands a full re-check, and `affects` does NOT see staleness (the §4.4 window stays bounded by `max_age_secs`, not closed).
- **Verified status-list issuer signature** (`VerifyingLiveStatusCheck`, **`status-list`**, `sq-pfae.13`) — verify the status-list VC's OWN issuer
  signature (the SAME `sparq-zk` Schnorr-over-RDFC-1.0 path the admit gate uses) against a trusted status-authority key (or a `did:key`/`did:web` issuer
  via `with_did_issuer`, `did` feature) **before** trusting its bits. **Fail-closed**: an unsigned / bad-sig / wrong-key / unresolvable-issuer list VC is `Unknown` (deny).
- **Public-key re-exports** (`public_key_from_hex`, `PublicKey`, `sq-0hu2w`) — downstream crates reach the issuer-key helpers via `sparq-trust` directly, avoiding a separate `sparq-zk` dependency.

## Honest scope — what this does and does NOT do

- **No privacy / unlinkability / anonymity.** The credential is admitted **in the clear** — the verifier learns the
  exact value (`age 25`, not "≥ 18"); this does **not** match ZKAPs-grade unlinkable presentation.
- **Holder binding + delegation invocation are the clear-WebID, non-anonymous paths** (`sq-wvne` / `sq-l5og`):
  `credentialSubject == Session.agent` (and invoker == terminal delegate) authenticate the WebID in the clear — not
  silently "solved"; presentations stay linkable by requester identity. The `delegate_key` binding defeats the
  key-substitution stolen-chain replay but not full non-replayability. PROV-O records add NO security property.
- **Issuer keys: operator-asserted by default; DID-bindable (opt-in, `sq-pfae.3`).** The default `trust:issuerKey` hex
  binding is the live forgery vector D′ (§3.3); the `did` feature binds from a `trust:issuerDid` — **narrows**, not anchors.
- **Open problems respected:** `sq-wvne` (ZK privacy) is **out of PoC scope**; `sq-xc4y` RESOLVED; `sq-l5og`
  **specified + enforced + tested**; `sq-tu4e` (live status) is now the opt-in `status-list` gate (`sq-pfae.7`),
  with the list VC's own issuer signature verifiable (`sq-pfae.13`).
- **Strict additivity (G6):** with `sparq-solid`'s `trust-graph` feature OFF the crate is not compiled — `sparq-solid` behaves exactly as WAC/ACP do today (byte-identical).

## 📚 Learn more

- Machine-readable [`trust.ttl`](ontologies/trust/trust.ttl) + [`SEMANTICS.md`](ontologies/trust/SEMANTICS.md)
  (`sq-pfae.2`) + [`secprop-ext.ttl`](../sparq-secprop-vocab/ontologies/secprop-ext.ttl); design record
  `research/solid-trust-graph-authz-design.md` (§3.2 storage; §4 delegation; §6.0 PoC) —
  [#940](https://github.com/sparq-org/sparq/issues/940). `cargo doc -p sparq-trust --all-features`.

## License

MIT — see [LICENSE](../../LICENSE).
