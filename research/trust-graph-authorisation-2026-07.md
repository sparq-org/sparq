# Trust-graph authorisation for Solid/LWS — 2026-07 estate audit + remaining-scope decomposition

Status: **design-for-review decomposition record** (epic `sq-pfae`, issue
[#940](https://github.com/sparq-org/sparq/issues/940)). This record does NOT restate the model
design — that is [`research/solid-trust-graph-authz-design.md`](solid-trust-graph-authz-design.md)
(merged, maintainer-reviewed, `sq-pfae.1`). Its job is three things: (1) an honest audit of
what the epic has ALREADY shipped versus what the brief's "trust-graph authorisation" still
lacks; (2) the design of the one genuinely net-new increment — **evaluating certification
edges (who vouches for whom, with what constraints) into the admission decision**, so
authorisation is driven by a trust *graph* rather than a flat trust-rule list; (3) the
disjoint child-bead plan for the fleet.

<!-- [FABLE-5] Fable-tier FRONT decomposition record for the remaining sq-pfae scope.
🤖 SPARQ agent. Companion records: solid-trust-graph-authz-design.md (the model),
trust-expression-spec.md (the sq-6syab verifier↔holder contract — a DIFFERENT surface,
boundary stated in §5), solid-access-control-design.md (the WAC/ACP substrate). -->

> **HONESTY FRAMING (load-bearing).** Everything here that touches the ZK/MPC estate
> inherits the standing discipline: the sparq ZK verifier is remediated and internally
> re-audited but has **no external accredited-cryptographer sign-off** (open gate
> `sq-qhy4`), and `sparq-mpc` is honest-majority **semi-honest only**. Nothing in this
> record, and no child bead below, asserts a settled cryptographic soundness, privacy, or
> unlinkability guarantee. The trust-graph layer designed here is a **clear-path**
> authorisation mechanism: it bottoms out in operator-anchored keys and signed
> attestations, not in cryptographic anonymity. Terminology: this program uses **"trust
> requirements"** for the verifier-side contract object (maintainer directive on #1592);
> no other term is used for it. No performance numbers appear here; work-box timings are
> non-canonical.

## 0. Corrected premise — this epic is 9/14 built, not greenfield

An architect brief re-stating the epic could read as if the trust-graph layer were still
to be designed. It is not. Verified against the code on `main` (2026-07-10), the estate is:

| Surface | State | Evidence |
|---|---|---|
| Two-stratum model (admission ahead of the WAC/ACP derivation), static/dynamic split | **Merged** | `crates/sparq-trust/src/admit.rs` (`admit`, `admit_static`), design §3.3 (`sq-xc4y` resolution) |
| Ten-term `trust:` vocabulary + `forPredicate` sugar, byte-pinned TTL | **Merged** | `crates/sparq-trust/src/vocab.rs`, `ontologies/trust/trust.ttl` (`sq-pfae.2`) |
| `trustx:` certification-scope vocabulary (`Framework`, `Certification`, `certifies`, `underFramework`, `scope`, `validFrom`/`validUntil`, `StatusAttestation`, eIDAS 2.0 / DIATF individuals) | **Merged (vocabulary ONLY — no pod-side evaluation)** | `crates/sparq-trust/src/framework_vocab.rs`, `ontologies/trust/trust-framework.ttl` (`sq-6syab.2`) |
| Control-gated trust-policy parsing (reified + relational forms), fail-closed | **Merged** | `crates/sparq-trust/src/policy.rs` (`ControlGate`, `parse_policy`) (`sq-pfae.4`) |
| DID resolution (`did:key` offline, `did:web` pluggable fetcher) | **Merged** | `crates/sparq-trust/src/did.rs` (`sq-pfae.3`) |
| Capability-delegation chains: carried-with-invocation, per-hop checked signatures, key-substitution defence, monotone attenuation, invoker binding + fresh-challenge PoP, PROV-O audit | **Merged** | `crates/sparq-trust/src/delegation.rs`, `delegation_prov.rs`, `tests/delegation_replay.rs` (`sq-pfae.6`, `sq-l5og` fix) |
| Status-list revocation, fail-closed on set/unknown/stale, signed-list verification, PROV-O justification | **Merged** | `crates/sparq-trust/src/status_list.rs` (`sq-pfae.7`, `sq-pfae.13`) |
| Trust-document storage: server ceiling + per-`.acr` narrowing, monotone versions, `AdmissionCacheKey` | **Merged (standalone; solid wiring is open bead `sq-pfae.12`)** | `crates/sparq-trust/src/store.rs` (`sq-pfae.5`) |
| sparq-solid wiring seam (default-OFF `trust-graph` feature; additive injection into `<urn:sparq:auth>`; conditional grants re-checked per request) | **Merged** | `crates/sparq-solid/src/trust_wire.rs`, `tests/trust_graph.rs` (`sq-pfae.10`) |
| WAC/ACP decision API FR-1..FR-5 (`/authz/decide`, `/authz/wac-allow`, `/authz/query`; stateless; fail-closed status mapping) | **Merged (NO trust-graph exposure)** | `crates/sparq-server/src/solid_authz.rs` (`sq-snopa`) |
| ODRL → auth-view bridge (permissions, prohibitions, conditional twins, ledger retraction), deny-overrides via the existing `∪ allow ∖ ∪ deny` | **Merged (composition with trust-graph UNTESTED)** | `crates/sparq-solid/src/odrl_bridge.rs` |
| ODRL-driven proof-method admissibility + secprop pre-check | **Merged** | `crates/sparq-trust/src/admissibility.rs`, `admit_with_precheck` (`sq-0dksu`) |

Open children carried forward (not re-cut here): `sq-pfae.8` (ZK admission feasibility —
**hard-gated on `sq-qhy4`**, stays designed/research, never a v1 guarantee), `sq-pfae.9`
(cost/decidability spike), `sq-pfae.11` (PoC hardenings: precise `xsd:duration`, real
Control-gate wiring, in-`.acr` ABAC-rule discovery), `sq-pfae.12` (wire `TrustStore` into
`sparq-solid`), `sq-pfae.14` (incremental status revocation).

## 1. The gap — the graph exists as vocabulary, but the decision is still flat

The brief's framing — "authorisation decisions driven by a trust graph (who
delegates/vouches for whom, with what constraints) rather than just per-resource ACLs" —
is HALF built:

- **Capability delegation** ("who delegates to whom") IS graph-shaped and IS evaluated:
  `delegation.rs` walks a carried chain per invocation with attenuation and invoker
  binding. Nothing to re-design.
- **Issuer vouching** ("who vouches for whom as a *source of statements*, with what
  constraints") exists ONLY as vocabulary. `trustx:Certification` can *say* "framework F
  certifies issuer I for scope S, valid over [t₀,t₁]" — eIDAS-trusted-list-shaped edges —
  but **no code path derives admission authority from such an edge**. The admission gate
  (`admit.rs`) and the store (`store.rs`) consume a flat `Vec<TrustRule>` in which every
  trusted source is *directly enumerated with its key*. A pod cannot yet say "admit age
  statements from any issuer the eIDAS trusted list certifies for age attestation" — the
  exact mode-2 relation the merged vocabulary was minted for, and the RT linked-role
  expressivity the model record concedes it lacks (design record §5.2).
- The **decision API** (`/authz/*`) and the **ODRL composition law** have no trust-graph
  surface at all: `/authz/decide` cannot carry presented credentials or trust policy, and
  no test pins that an ODRL prohibition still overrides a trust-admitted grant.

So the remaining epic scope is an **evaluation + composition** increment over merged
vocabulary and merged gates — not a new model. That is what this record decomposes.

## 2. Design: certification-edge closure into effective trust rules

### 2.1 What a trust edge MEANS (normative, fail-closed)

A trust edge is a `trustx:Certification`: a **signed, time-windowed, status-checked,
scope-attenuating** attestation by an *anchor authority* that an issuer may act as a
source for statements **within the intersection** of the anchor's own authority and the
certification's `trustx:certificationScope`. An edge is **never blanket trust**, and it confers
nothing by itself — authority flows only from a **local, Control-gated anchor rule**:

```turtle
# Pod policy (Control-gated channel, same as .acl — parse rejects ungated input):
[] a trust:TrustRule ;
   trustx:trustsFramework  <https://eidas.example/trusted-list> ;  # the anchor edge-source
   trust:issuerKey         <did:web:eidas.example#tl-key> ;        # key the LIST is signed with
   trust:forShape          [ ... age-attestation shape ... ] ;     # ceiling: max shape any derived rule may admit
   trust:scope             <resourceX> ;
   trust:freshWithin       "P30D"^^xsd:duration .

# Presented certification bundle (a SIGNED graph, verified like any credential):
[] a trustx:Certification ;
   trustx:certifies      <https://gov.example/issuer> ;
   trustx:underFramework <https://eidas.example/trusted-list> ;
   trustx:certificationScope [ ... shape/attribute-list the issuer is certified for ... ] ;
   trustx:validFrom "..." ; trustx:validUntil "..." .
```

This reuses the merged `trust:` + `trustx:` terms **unchanged** — no new vocabulary is
minted for the one-hop case. (The framework individuals, `sec-req:` regulatory instances,
and the `StatusAttestation`/`coveredBy` status terms are all already vendored/merged.)

### 2.2 The decision algorithm (the ONLY new evaluation logic)

Effective rules are derived by a bounded closure, then the **unchanged** admission gate
runs; nothing downstream of `Vec<TrustRule>` changes:

```text
derive_effective_rules(direct_rules, certifications, now, depth_bound) -> rules:
  rules := direct_rules                              # flat rules pass through untouched
  frontier := anchor rules of form trustsFramework   # graph mode starts ONLY at local anchors
  for depth in 1..=depth_bound:                      # v1 prototype: depth_bound = 1
    for (anchor, cert) in frontier × certifications:
      if cert.under_framework != anchor.source:            continue   # edge must attach to the anchor
      if not verify_sig(canon(cert_graph), anchor.key):    continue   # CHECKED signature (RDFC-1.0 commit), never self-asserted
      if now outside [validFrom, validUntil]:              continue   # time window
      if status_attestation_missing_or_set_or_stale(cert): continue   # positive-attestation discipline (OWA), fail-closed
      derived_shape := intersect(anchor.forShape, cert.scope)         # ATTENUATION-ONLY
      if derived_shape is empty or not provably ⊆ anchor.forShape:    continue
      emit TrustRule { source: cert.certifies, key: key_of(cert.certifies),
                       shape: derived_shape, scope: anchor.scope,
                       fresh_within: min(anchor.fresh_within, cert window remainder) }
  return rules            # then the UNCHANGED admit()/admit_static() gate runs over them
```

Load-bearing properties (each is a test in the child beads, not prose):

1. **Attenuation-only constraint propagation.** A derived rule's shape/scope/freshness is
   the intersection with the anchor's — an edge can only *narrow* what its anchor already
   grants. A certification whose scope is not provably within the anchor ceiling
   contributes **nothing** (fail-closed on shape-containment uncertainty — containment
   that cannot be decided cheaply is treated as *not contained*).
2. **No ambient edges.** Only certifications reachable from a local Control-gated anchor
   rule confer anything. An attacker-supplied "certification" with no matching anchor is
   inert, exactly as an untrusted credential is today (design record §3.3-D4).
3. **Meta-scope non-escalation.** An issuer certified *for attributes* cannot certify
   other issuers: acting as an edge-*source* at depth *k+1* requires the entity's own
   incoming certification scope to cover certification-issuing itself (a scope over
   `trustx:Certification` statements), which an attribute-scope never does. In the v1
   prototype this is moot (depth 1: only the anchored framework is an edge-source), but
   the invariant is stated now so the depth>1 extension cannot silently drop it.
4. **Cycles and depth.** The closure is depth-bounded with a visited set; a cycle or an
   over-depth path contributes nothing (deny-by-absence, never divergence). This is a
   Rust-side pure function — **not** an in-reasoner recursion — so the `sq-tu4e`
   stratification and one-side-bound seeding disciplines are untouched.
5. **Strict additivity, twice.** Zero certifications ⇒ derived set is exactly
   `direct_rules` ⇒ behaviour byte-identical to today's flat path. Feature OFF ⇒ the
   module is not compiled; `sparq-solid`/`sparq-server` defaults unchanged (the G6
   property the whole epic maintains).

### 2.3 Revocation of an edge

Certifications are status-checked like credentials (the merged `status_list.rs` gate:
fail-closed on set/unknown/stale, positive-attestation discipline per #1592). Revoking a
certification must invalidate cached admission verdicts: `TrustStore::policy_version`
must **cover the certification set**, so the `(epoch, AdmissionCacheKey)` composition in
`sq-pfae.12` picks up edge revocation exactly as it picks up rule changes. Honest
residue (unchanged from the model record): revocation after materialise propagates by
re-materialisation — the stale-authority window is *bounded* by freshness/max-age, not
closed; the incremental path is `sq-pfae.14`, which now also covers certification-status
deltas.

### 2.4 Composition with the WAC/ACP decision API and ODRL

- **Decision API.** `/authz/decide` gains an optional, feature-gated (default-OFF,
  double-opt-in like `solid-authz`) trust block: presented credential graphs, the trust
  policy, certification bundles. The handler runs the existing
  `PodStore::admit_trust_credential_*` seam over the derived effective rules, then the
  unchanged `decide()`. The response carries a **minimal admission justification**
  (which rule / which certification edge admitted the deciding fact — the
  `justify_status_decision` PROV-O idiom), so the LWS/Solid-WG-facing surface exposes
  *why* a graph-driven decision held. Stateless like the rest of `sq-snopa` (dataset in
  the request); the stateful server-pod variant stays a deferred follow-up. Fail-closed:
  any malformed, unverifiable, stale, revoked, or unbound trust input ⇒ deny with
  justification; never a grant, and never an error path that grants.
- **ODRL.** Both bridges are additive injectors into `<urn:sparq:auth>`; enforcement is
  the single existing `∪ allow ∖ ∪ deny` walk, so the intended law is already structural:
  **an ODRL prohibition (deny triple) overrides a trust-graph-admitted allow for the same
  (principal, mode, target)**. What is missing is the *pinned test* — today no test runs
  `trust-graph` and `odrl-bridge` together, so the law is design intent, not verified
  property. A conformance bead closes that (and pins byte-identical decisions with
  trust-graph OFF).

### 2.5 Explicitly deferred (designed-only; recorded so nobody "helpfully" builds them)

- **Multi-hop endorsement (depth > 1)** — the closure and the meta-scope invariant are
  specified above, but the prototype ships depth 1 (the eIDAS/DIATF trusted-list shape).
  Depth > 1 needs the certification-authority scope shape + the `sq-pfae.9` cost bound
  first.
- **Threshold / k-of-n corroboration** (RT^T-style "admit only if ≥k independent trusted
  issuers attest"). Monotone and implementable as a Rust-side admission count over
  *distinct verified issuer keys* — but requested nowhere in #940/#1592, and
  deny-on-disagreement (its dual) is likely unreachable under input-only NAF (model
  record §3.5). Stays designed-only; no bead.
- **Anything ZK/private.** Unlinkable or hidden-issuer admission of certification edges
  is `sq-pfae.8` + `sq-6syab.5` territory, hard-gated on `sq-qhy4` (external
  accredited-cryptographer sign-off pending; MPC semi-honest only). The clear-path layer
  here makes **no** anonymity, unlinkability, or ZK-soundness claim.

## 3. Soundness posture (summary of the fail-closed obligations)

Deny on: no matching anchor rule; unverifiable/absent signature (rule key, credential,
certification, or status list); expired or not-yet-valid window; missing/set/stale status
attestation; scope or shape not provably attenuating; cycle or depth exhaustion; holder
not bound; Control-gate absent. Every one of these is an *absence-of-grant*, not an
exception path — the gates return empty sets, and the decision layer's single `deny()`
constructor keeps uncertainty ⇒ deny (FR-6). None of this is a cryptographic guarantee:
signature verification rides the internally re-audited but **externally unaudited**
`sparq-zk` estate (`sq-qhy4` pending), and the whole layer is a research prototype behind
default-OFF features, matching the epic's honest constraints.

## 4. Child-bead plan (disjoint; created under `sq-pfae`)

New beads (this record). File-sets are pairwise disjoint; same-crate beads are
dep-sequenced so at most one is in flight per crate (`sparq-server` `http.rs` gets
exactly one in-flight toucher, per the conflict-partition):

| Bead | Crate | Tier | Files | One-line scope |
|---|---|---|---|---|
| `sq-pfae.15` — certification-edge closure | `sparq-trust` | opus | `src/graph.rs` (new), `src/lib.rs` (mod line), `Cargo.toml` (feature `cert-graph`), `tests/certification_graph_e2e.rs` (new), `ontologies/trust/SEMANTICS.md` | `derive_effective_rules` per §2.2 + adversarial edge-forgery matrix |
| `sq-pfae.16` — store composition | `sparq-trust` | sonnet | `src/store.rs`, `tests/trust_store_cert_rules.rs` (new) | certification-derived rules through `effective_rules` narrowing; `policy_version` covers the certification set |
| `sq-pfae.17` — decision-API extension | `sparq-server` | sonnet | `src/solid_authz.rs`, `Cargo.toml`, `src/http.rs` (route guard only), `tests/solid_authz_trust.rs` (new) | stateless trust block on `/authz/decide` + minimal admission justification, double-opt-in, fail-closed |
| `sq-pfae.18` — ODRL composition conformance | `sparq-solid` | sonnet | `tests/trust_odrl_compose.rs` (new only) | pin deny-overrides across trust-graph × ODRL + feature-OFF byte-identity |

Dependency edges (real ordering only): `sq-pfae.15` → `sq-pfae.16` (consumes the derivation API);
`sq-pfae.15` → `sq-pfae.17` (the request schema carries certifications once the `.15` types exist);
`sq-pfae.15` → `sq-pfae.9` (the cost spike must bound the closure path too); `sq-pfae.12` →
`sq-pfae.18` (the `.18` fixtures target the post-wiring `PodStore` surface — writing them
against the flat-rule seam would churn). `sq-pfae.11`/`sq-pfae.14` remain independent
(file-disjoint from `sq-pfae.15`: `policy.rs`/`status_list.rs` vs the new `graph.rs`) but
share the crate — the scheduler should not run them concurrently with `sq-pfae.15`/`.16`.

Phasing: **wave 1** `sq-pfae.15` ∥ `sq-pfae.12` ∥ `sq-pfae.11`-or-`.14` (disjoint files; one
per crate at a time) → **wave 2** `sq-pfae.16` ∥ `sq-pfae.17` ∥ `sq-pfae.18` → **tail**
`sq-pfae.9`; `sq-pfae.8` stays hard-gated on `sq-qhy4` and is untouched by this plan.

Review discipline: `sq-pfae.15` is authorisation-derivation logic (a broadening bug is privilege
escalation) — it goes through escalated adversarial review before arming, never
mechanical-verify alone. All beads keep the epic's standing constraints: opt-in
default-OFF features, both-feature-state gates green, no unqualified ZK/privacy claim.

## 5. Boundaries with the neighbouring programs (to prevent duplicate work)

- **`sq-6syab` (trust-expression, #1592)** owns the *verifier↔holder* contract: the
  trust-requirements document a verifier sends, the holder-side evaluation
  (`sq-6syab.4`), the conformance suite and paper. THIS epic owns the *pod-side
  admission* consumption of the same `trustx:` vocabulary. Shared vocabulary, disjoint
  evaluation surfaces and disjoint files; `sq-pfae.15` must not touch `framework_vocab.rs` (read
  the constants, add none — any new term goes through a `sq-6syab.2`-style vocabulary
  bead with TTL pinning).
- **`sq-rsd3v` (ZK inference + credentials)** owns everything zero-knowledge about this
  layer; it is dep-blocked on this epic and remains so. No bead here adds circuit or
  proof surface.

<!-- markdownlint retained; no vendored content modified. [FABLE-5] -->
