---
name: trust-graph
description: "Pod-side certification-edge trust-graph admission with the opt-in cert-graph (sparq-trust) + trust-graph (sparq-solid) cargo features, both default-OFF: anchor trust in a certifier (a framework operator / Trusted-List authority) via a Control-gated trust:TrustRule, then let the depth-bounded, attenuation-only, fail-closed closure (derive_effective_rules) turn that certifier's signed trustx:Certification edges into derived rules the UNCHANGED admission gate consumes — so a pod admits facts from issuers the certifier vouches for, never wider than the certifier's own authority. Use when access should follow certified-issuer attestations rather than a controller-enumerated agent list; covers the anchor-rule TTL shape, the certification-bundle shape, the six EdgeRejection fail-closed reject reasons, and the PodStore wiring. Also covers the default-OFF expression feature (sparq-trust): the verifier→holder trust-expression contract, clear path — parse the query+TR+nonce request, generate the §3.1 reference rewrite Q→Q', evaluate over the holder's attested named-graph dataset, and independently re-check the provenance-encoded response. RESEARCH prototype — no privacy/unlinkability claim, ZK estate externally unaudited (sq-qhy4)."
license: MIT
metadata:
  version: "0.1.0"
  homepage: https://github.com/sparq-org/sparq
---

# sparq trust-graph — certification-edge admission (pod-side closure)

The certification-edge layer answers: *"my pod anchors trust in a certifier (a government
framework operator, a Trusted-List registrar) — may it therefore admit facts from an issuer
that certifier has **certified**, without me enumerating that issuer myself?"*

`sparq_trust::graph::derive_effective_rules` (opt-in **`cert-graph`** feature on
`sparq-trust`, default-OFF) runs a **depth-bounded** (v1: depth-1), **attenuation-only**,
**fail-closed** closure over signed `trustx:Certification` edges. It is pure
pre-processing: it produces the `Vec<TrustRule>` that the **UNCHANGED** admission gate
(`sparq_trust::admit`) then consumes exactly as it consumes a hand-authored policy.

```text
  direct_rules (Control-gated trust:TrustRule anchors)   signed trustx:Certification edges
                                  │                            │
                                  ▼                            ▼
  ┌────────────────────────────────────────────────────────────────────┐
  │  derive_effective_rules   (opt-in `cert-graph`, default-OFF)       │
  │  depth-1 · attenuation-only · fail-closed                          │
  │  every derived rule ⊆ (anchor ∩ cert scope ∩ validity window)      │
  └────────────────────────────────────────────────────────────────────┘
                                  │  effective_rules = direct_rules ++ derived
                                  ▼
  ADMISSION gate (sparq_trust::admit, UNCHANGED) ─→ auth grants in <urn:sparq:auth>
                                  (pod wiring: sparq-solid `trust-graph`, default-OFF)
```

## What this is — and is NOT (honest scope, read first)

- **RESEARCH prototype, default-OFF.** Every flag involved is opt-in and **default-OFF**
  (`cert-graph` on `sparq-trust`, `trust-graph` on `sparq-solid`, `solid-authz-trust` on
  `sparq-server`). With the features off, the crates build **byte-identically** to plain
  WAC/ACP today — strict additivity is a design property (G6,
  `research/solid-trust-graph-authz-design.md` §2.2).
- **NO privacy / unlinkability / anonymity claim.** Admission is the **clear path**: the
  credential is admitted in the clear and the holder binding authenticates a clear WebID.
  The ZK estate this layer shares a signature primitive with has **no external
  accredited-cryptographer sign-off** (open audit gate **sq-qhy4**), and `sparq-mpc` is
  honest-majority **semi-honest only**. Nothing in this skill is a production security
  guarantee.
- **Framework trust is ANCHORED, not proven.** A `trustx:Certification` bottoms out in a
  trust anchor you chose, not cryptography (design §7.2). The certifier's *own* key is
  still operator-/DID-asserted — the live upstream forgery vector (D′, `sq-pfae.3`). The
  closure defeats *broadening* and *forged-edge* escalation **given honest anchor keys**;
  it does not close the key-trust gap above it.
- **Attenuation-only is the HARD invariant.** A derived rule that grants an issuer
  authority its certifier does not hold is a **privilege escalation**, not a bug to paper
  over. Where narrowing cannot be *proven* (undecidable SHACL-shape containment), the edge
  contributes **nothing**.

## When to reach for trust-graph vs plain WAC/ACP

- **Plain WAC/ACP** (`skills/access-control/SKILL.md`) — the controller can enumerate the
  agents/clients/issuers directly (`acl:agent`, ACP matchers). No `sparq-trust` needed.
- **Trust-graph admission, direct anchors only** — access should follow an
  **externally-attested fact** ("age over 18, said by an issuer I trust for `age`
  statements"), and you can enumerate each trusted issuer yourself as a
  `trust:TrustRule`. That path is documented in `skills/access-control/SKILL.md`
  (the `trust-graph` feature's admission stratum) — this skill does not repeat it.
- **The certification-edge closure (this skill)** — you anchor a **certifier** (framework
  operator / Trusted-List) rather than each issuer, and want issuers *it certifies* to be
  admitted transitively (depth-1), automatically narrowed to what the certifier itself may
  confer. The `trustx:trustsFramework` mode of the trust-requirements vocabulary
  ("admit issuers certified under this framework, subject to scope conformance") is what
  this closure operationalises pod-side.

## Feature flags (all default-OFF, opt-in — the core stays lean)

```toml
# The closure itself (sparq_trust::graph): derive_effective_rules, explain_edge,
# Certification, CertScope, EdgeRejection. Implies the trustx: framework-vocab module.
sparq-trust = { path = "crates/sparq-trust", features = ["cert-graph"] }

# Pod-side wiring: PodStore admission-install methods over <urn:sparq:auth>.
sparq-solid = { path = "crates/sparq-solid", features = ["trust-graph"] }
# + DID-resolved issuer keys (trust:issuerDid) instead of operator-asserted hex:
sparq-solid = { path = "crates/sparq-solid", features = ["trust-graph-did"] }
```

The HTTP surface (`sparq-server`) exposes the same closure behind its own opt-in
`solid-authz-trust` feature: the request's JSON trust block may carry a
`"certifications"` array (wire schema in `crates/sparq-server/src/solid_authz.rs`), and
the handler runs `derive_effective_rules(..., depth_bound = 1)` ahead of admission,
reporting `certGraphDerived` in the decision JSON. Both `CertScope` kinds are on the wire:
`"scopeKind": "anyService"`, and `"scopeKind": "shape"` + `"scopeShapePredicateIri"` for a
statement-type-scoped edge (`sq-sllu4`). A shape scope travels as a `trust:forPredicate`
IRI, not an arbitrary shape closure, because the certifier's signature covers the scope
triples — the server rebuilds ONE canonical desugaring (fixed blank-node labels, fixed
triple order; see `cert_scope_predicate_shape`) so the certifier's own reconstruction hashes
to the same signed preimage. Anything else fails the signature gate.

## The anchor rule — `trust:TrustRule` is the ceiling

Anchors are the trust ROOT: only a certifier that already appears in the Control-gated
policy (as a rule `trust:source` with the matching key) can confer anything. The policy
graph is parsed by `sparq_trust::parse_policy` (possession of a `ControlGate` is the
authoring channel — same discipline as `.acl`/`.acr`). Reified authoring form
(namespace `trust:` = `https://sparq.dev/ns/trust#`):

```turtle
@prefix trust: <https://sparq.dev/ns/trust#> .
@prefix xsd:   <http://www.w3.org/2001/XMLSchema#> .

[] a trust:TrustRule ;
  trust:source      <https://gov.example/framework> ;   # the certifier's identity IRI
  trust:issuerKey   "a1b2…" ;                           # its verification key, hex (or trust:issuerDid)
  trust:forPredicate <https://schema.org/age> ;         # statement-type CEILING (desugars to
                                                        # a single-predicate trust:forShape)
  trust:scope       <https://pod.example/medical/> ;    # resource/container ceiling
  trust:freshWithin "P30D"^^xsd:duration .              # staleness ceiling
```

Field-by-field, each is a **ceiling** the closure can only narrow, never widen:

- `trust:source` + `trust:issuerKey` — a certification is anchored only if its certifier
  IRI **and** key both match a rule here (`trust:issuerDid` works too, behind the `did` /
  `trust-graph-did` features).
- `trust:forShape` / `trust:forPredicate` — the statement-type ceiling: a SHACL node-shape
  (a `forPredicate` is desugared into a single-predicate shape). The derived rule's shape
  is `anchor.shape ∩ cert.scope`.
- `trust:scope` — the resource ceiling. A certification narrows WHO and WHAT-TYPE, never
  widens WHERE: the derived rule inherits the anchor's resource scope **unchanged**.
- `trust:freshWithin` (`xsd:duration`) — the staleness ceiling: the derived rule's
  freshness is `min(anchor.fresh_within, certification window remaining)`. The lexical
  value must be an **exact** day/time duration — `PnDTnHnMnS` (`P30D`, `PT12H`,
  `P1DT2H3M4S`). The nominal `P1Y` / `P6M` designators have no exact seconds value
  without an anchor instant, so they are **rejected fail-closed** (the whole policy is
  refused) rather than approximated; write `P365D` if that is what you mean.

The equivalent claim-level relational form (`trust:trustsSourceFor`) desugars to the same
`Vec<TrustRule>` — see `skills/access-control/SKILL.md` for both authoring forms.

## The certification bundle — `trustx:Certification`

A certification is a **signed edge**: an authority attests that an issuer is certified —
under a framework, over a scope, within a validity window. A Trusted-List entry or a
DIATF-register entry IS one such edge. The vocabulary (namespace `trustx:` — the same
`https://sparq.dev/ns/trust#` namespace, machine-readable form in
`crates/sparq-trust/ontologies/trust/trust-framework.ttl`):

```turtle
@prefix trustx: <https://sparq.dev/ns/trust#> .
@prefix xsd:    <http://www.w3.org/2001/XMLSchema#> .

<urn:example:cert-1> a trustx:Certification ;
  trustx:certifies      <https://issuer.example/dvs> ;      # the vouched-for issuer
  trustx:underFramework <https://gov.example/framework> ;   # the trustx:Framework
  trustx:certificationScope trustx:AnyServiceScope ;        # or a SHACL shape / rulebook IRI
  trustx:validFrom      "2026-01-01T00:00:00Z"^^xsd:dateTime ;
  trustx:validUntil     "2027-01-01T00:00:00Z"^^xsd:dateTime .
```

What the closure actually consumes is the `sparq_trust::graph::Certification` struct —
the edge plus the two keys and the certifier's signature:

- `certifier` / `certifier_key` — must match an anchor rule's `source` + `issuer_key`.
- `certified_issuer` / `certified_key` — the derived rule's issuer identity + key. The
  key is **bound into the signed message**, so a substituted key breaks the signature.
- `scope` — `CertScope::AnyService` (the coarsest, service-level scope — the honest DIATF
  granularity; imposes no statement-type narrowing of its own, the anchor shape stays the
  ceiling) or `CertScope::Shape(…)` (the issuer is certified only for statements matching
  that SHACL node-shape).
- `valid_from_unix_secs` / `valid_until_unix_secs` — the inclusive window.
- `signature_hex` — the certifier's Schnorr signature over the **domain-separated**
  `certification_message` (domain tag `"TRUSTCRT"`, distinct from delegation-hop /
  commitment / holder-PoP tags). A graph merely *claiming* `trustx:certifies` proves
  nothing: the signature is CHECKED, never taken on assertion, and it covers the scope +
  window, so a narrow certification cannot be re-presented broadened.

## The closure — `derive_effective_rules` → the unchanged `admit`

```rust
use sparq_trust::graph::{derive_effective_rules, Certification};
use sparq_trust::policy::TrustRule;

// direct_rules: parse_policy(...) output (the Control-gated anchors).
// certifications: the framework's published register entries, as signed edges.
let effective: Vec<TrustRule> =
    derive_effective_rules(&direct_rules, &certifications, now_unix_secs, 1);
// `effective` = direct_rules (cloned verbatim, in order) ++ surviving derived rules.
// Feed it to sparq_trust::admit exactly like a hand-authored policy — no gate change.
```

Load-bearing properties (each adversarially tested in
`crates/sparq-trust/tests/certification_graph_e2e.rs`):

- **Strict additivity** — zero surviving certifications ⇒ the output is `direct_rules`
  **byte-identical**. The closure can only append.
- **Depth-bounded** — v1 is depth-1: derived rules are never re-used as anchors for a
  further round; `depth_bound = 0` short-circuits to `direct_rules` verbatim.
- **Attenuation-only** — every derived rule satisfies
  `derived ⊆ (anchor ∩ cert scope ∩ validity window)`. Shape containment is
  **target-set-contravariant, conformance-covariant**: a cert shape narrows the anchor
  only when its SHACL *selection/target* predicates are a **subset** of the anchor's
  (targets may only shrink — an extra `sh:targetSubjectsOf` WIDENS) and its *conformance*
  constraints are a **superset** (constraints may only grow). Anything unprovable fails
  closed.

## Fail-closed reject reasons — `EdgeRejection`

`derive_effective_rules` silently drops a rejected edge; `explain_edge` runs the same
gates in the same order and returns *why* an edge contributed nothing (the adversarial
matrix asserts against these). The variants, in gate order:

| Variant | The edge is dropped because … |
| --- | --- |
| `Cyclic` | `certifier == certified_issuer` (self-certification), or the certified issuer already anchors the certifier — a cycle could launder a broadening, and adds no authority. Checked FIRST, before the signature. |
| `NoAnchor` | no `direct_rules` anchor has this certifier's `source` **and** matching key — the pod does not anchor this certifier, so it can confer nothing. |
| `SignatureInvalid` | the signature is absent, unparseable, or does not verify under the certifier's key over the domain-separated `certification_message` (a self-asserted / forged edge). |
| `OutOfWindow` | the window is ill-formed (`valid_until < valid_from`) or does not cover `now` — expired / not-yet-valid; fail-closed on time. |
| `Broadening` | the certification scope is NOT provably contained in the certifier's anchor shape — a broadening attempt **or** an undecidable containment (undecidable ⇒ not contained ⇒ contributes nothing). |
| `OverDepth` | `depth_bound` was `0`, or the edge would only be reachable beyond the bound (v1 depth-1: derived rules are never anchors). |

## Pod-side wiring — `sparq-solid` (`trust-graph` feature)

With `trust-graph` ON, `PodStore` gains the admission-install methods that put derived
grants into `<urn:sparq:auth>` **on top of** the unchanged WAC/ACP view (grants only union
allows; a trust-graph grant can never drop a WAC/ACP grant). Two install paths — choosing
the wrong one is a soundness bug (the `sq-xc4y` static/dynamic split):

- `PodStore::admit_trust_credential_static` — the **materialise-time** path: runs only the
  session-independent class (`admit_static`) and installs `auth:ConditionalGrant`s whose
  holder + freshness are re-checked **per request**. Use this for a long-lived
  materialise-once view.
- `PodStore::admit_trust_credential_and_materialize` (and `_with_rule`) — the
  **single-request snapshot** path: runs the combined gate against one live `Session` and
  installs an unconditional grant valid for *that* request only. Never use it to populate
  a long-lived view.

Feed `effective_rules` from the closure to either path as the `rules` argument; admission
itself never errors — a credential that fails any gate simply admits nothing
(fail-closed, default-deny).

## Trust-expression surfaces — verifier↔holder contract (sq-6syab, issue #1592)

The **trust-expression layer** (`sq-6syab` epic) defines a *verifier-to-holder contract*
for framework-anchored attestation queries — answering: *"emit the results of this query
over attributes from parties **[X, Y and Z] OR from certified issuers within eIDAS/DIATF**,
but ONLY where each issuer has issued only what they are certified to issue"* (the
**certification-scope** constraint).

### The vocabulary layer — `trustx:` certification terms (`sq-6syab.2`, `framework_vocab`)

Behind the default-OFF **`framework-vocab`** feature. The `trustx:` namespace (sharing the
`trust:` base IRI, extending it, not forking) adds:

- **Trust requirements** (`trustx:TrustRequirements`) — the contract carrier: ONE small RDF
  graph binding a SPARQL query to its trust conditions. The trust conditions live HERE, never
  in the query (no new query syntax — design §3.1). Properties: `trustx:question` (the
  query IRI), `trustx:trustsIssuer` (enumerated issuers), `trustx:trustsFramework`
  (framework-certified issuers), `trustx:requiresScopeConformance` (issuer certification
  scope check), `trustx:requiresValidStatusAt` (positive status window instant),
  `trustx:methodPolicy` (OPTIONAL ODRL reference).
- **Certification and scope** (`trustx:Certification`, `trustx:certificationScope`) — an issuer being
  *certified under* a framework (`trustx:underFramework`), for a *scope* ranging from
  service-level (`trustx:AnyServiceScope`, the honest DIATF granularity) down to a
  predicate set / SHACL shape (reusing `trust:forShape`). Validity window via
  `trustx:validFrom`/`trustx:validUntil`.
- **Status attestation** (`trustx:StatusAttestation`, `trustx:coveredBy`) — positive,
  time-windowed attestation that an issuer / credential was valid in a window (OWA/monotone:
  non-revocation = existence of covering attestation, never absence).
- **Framework individuals** (`trustx:eIDAS2`, `trustx:DIATF`) — thin instances that
  `rdfs:seeAlso` the vendored `sec-req:` eIDAS 2.0 / UK DVS regulatory individuals (no
  fork, no duplication).

Machine-readable form: `crates/sparq-trust/ontologies/trust/trust-framework.ttl`. All terms
are **NON-STANDARD** (a WG would rehome them). **Anchored, not proven** — framework
membership bottoms out in a trust anchor (the operator's signed Trusted List / register),
not cryptography. See `crates/sparq-trust/README.md` for the full honesty frame.

### Holder-side evaluation — the `expression` feature (`sq-6syab.4`)

The same `trustx:` vocabulary also drives the **verifier→holder trust-expression
contract** (issue #1592, `research/trust-expression-spec.md` §3.1–3.5): a verifier sends a
SPARQL query `Q`, a `trustx:TrustRequirements` graph `TR`, and a nonce; the holder answers
`Q` **only from statements admissible under `TR`** and returns a provenance-encoded
response the verifier can independently re-check. Opt-in module
`sparq_trust::expression`, behind the default-OFF **`expression`** feature (enables
`framework-vocab` + `status-list` + `did` + `secprop-admissibility` — the surfaces it
reuses — and pulls `sparq-engine` + the vendored `spargebra` parser; the lean default
build is byte-identical with it OFF):

```toml
sparq-trust = { path = "crates/sparq-trust", features = ["expression"] }
```

The public surface, in contract order — the challenge-nonce type first, then the five
contract calls:

0. **`ChallengeNonce::generate()` / `ChallengeNonce::from_wire(&str)`** — the nonce is a
   type, not a `&str`, so the freshness obligation is visible at the construction site
   (issue #4621). `generate()` is the **verifier-side** path: 32 bytes from the OS CSPRNG.
   `from_wire()` adopts a value that legitimately came from outside (the echoed challenge,
   a decoded response, a session nonce minted a layer above) and **promises nothing about
   freshness** — a call site wrapping a literal with it is visibly opting out. Empty /
   all-whitespace is refused (`ExpressionError::EmptyNonce`); there is deliberately no
   length or entropy heuristic, which would reject `"n"` while accepting a 32-byte
   constant. Hardening only: nothing here can *detect* a reused nonce, and the ZK/trust
   estate remains externally unaudited (`sq-qhy4`).
1. **`parse_request(query, tr_triples, &nonce)` → `ContractRequest`** — fail-closed on a
   missing/duplicated requirements node, a missing `trustx:question` /
   `trustx:requiresValidStatusAt`, a non-UTC `xsd:dateTime`, a malformed `did:` issuer,
   or a `TR` naming **no trust mode** (neither `trustx:trustsIssuer` nor
   `trustx:trustsFramework` — such a document admits nothing and is refused up front,
   never evaluated vacuously). `trustx:question` is an **opaque label** at this layer:
   it is parsed for presence and IRI-ness but never resolved or compared against the
   query, so the question↔query association is a caller-owned trust boundary (request
   authentication / trusted question resolution — see the module's honest-scope docs).
2. **`rewrite_query(&request)`** — the §3.1 normative reference rewrite `Q → Q'`: each
   of `Q`'s triple patterns is wrapped in a `GRAPH ?g { … }` over the holder's
   attestation bundles and conjoined with issuer-membership (mode 1), positive
   status-attestation validity at *t* (the existence of a covering window — never
   evidence-of-absence), and certification-window + scope-conformance (mode 2) patterns;
   the two modes compose by plain `UNION`.
3. **`evaluate_contract(&request, &holder, precheck)` → `ContractOutcome`** — runs the
   optional `trustx:methodPolicy` ODRL pre-check (the existing
   `sparq_trust::admissibility::admissible` reduction), then `Q'` via `sparq-engine`
   over the holder's attested dataset — one named graph per attestation bundle;
   attribution (`prov:wasAttributedTo`), status attestations, and certifications in the
   default graph (build it with `Graph::load_dataset` from TriG) — then assembles the
   response.
4. **`verify_response(&request, &response)`** — the INDEPENDENT verifier re-check:
   re-derives `Q'` from the request alone and evaluates it over the response's
   named-graph (TriG) form. A wrong nonce is refused outright; a stripped- or
   tampered-provenance response simply yields no admissible derivation.
5. **`mint_status_attestation(…)`** — the status-list bridge: only a verified
   `status_list::LiveStatus::Live` check can mint the positive, time-windowed
   `trustx:StatusAttestation` triples the rewrite consumes, so a revoked credential can
   never acquire a covering window.

The response carries both design-§4 encodings: the RDF 1.2 **reifier normative form**
(`rdf:reifies` + a triple term, PROV-O qualification on the reifier) and the mechanically
lossless **named-graph + PROV-O TriG mapping** — the latter is what `verify_response`
re-checks, runnable on any SPARQL 1.1 engine today.

**Limitations (v1 — each a fail-closed refusal, never a partial evaluation):** `Q` must
be an ASK or SELECT (optionally DISTINCT) over ONE basic graph pattern — no property
paths, FILTER, OPTIONAL, UNION, dataset clauses, blank-node patterns, or RDF 1.2
triple-term patterns (the engine cannot *match* triple terms yet, design §7.5) — and no
variables in the reserved `?__tx_*` namespace the rewrite mints.

**Fail-closed trust boundaries (load-bearing):**

- **No admissible derivation ⇒ no binding.** An untrusted issuer, a stale or missing
  status window, a scope violation, an expired or status-uncovered certification — each
  yields `false` / zero rows AND a response with zero bundles. Every check is the
  monotone existence of a positive attestation (OWA); "reject" is never a derived denial.
- **The clear path re-checks admissibility, not cryptography.** The verifier must trust
  the underlying attestations' signatures and the completeness of what the holder
  disclosed (design §7.3); framework trust is anchored, not proven (§7.2).
- **The method-policy pre-check is IRI-bound but caller-resolved.** A `MethodPrecheck`
  resolving a different policy IRI than `TR` names is refused
  (`ExpressionError::MethodPolicyMismatch`), so the named policy can never be silently
  substituted with a weaker one — but resolving that IRI into the policy's N3
  constraints remains the caller's trust boundary; nothing authenticates the resolution
  itself.
- **No ZK / privacy / unlinkability claim.** This is the CLEAR path; the ZK realisation
  is a separate bead (`sq-6syab.5`), the sparq ZK estate is internally re-audited with
  external accredited-cryptographer sign-off PENDING (`sq-qhy4`), and `sparq-mpc` is
  honest-majority semi-honest only.

## Cross-references

- `skills/access-control/SKILL.md` — the WAC/ACP layer, the admission stratum, both
  anchor-authoring forms, the `TrustStore` narrowing model (this skill deliberately does
  not duplicate it).
- `skills/verifiable-credentials/SKILL.md` — the standards-interop signed-graph
  complement; `skills/zk-query-proofs/SKILL.md` — the (externally unaudited, sq-qhy4)
  ZK estate.
- `crates/sparq-trust/README.md` — the crate surface overview, module list, and the
  full honesty frame. `crates/sparq-trust/src/framework_vocab.rs` + rustdoc (`cargo doc
  -p sparq-trust --all-features`) — the vocabulary IRIs as Rust constants.
- `crates/sparq-trust/tests/certification_graph_e2e.rs` — the pod-side adversarial
  edge-forgery matrix (this section § *Pod-side wiring*).
- `research/trust-expression-spec.md` (epic `sq-6syab`, issue #1592; design § 3.1 contract,
  § 3.4 certification-scope, § 4 response provenance encoding, § 7 honesty / trust anchors)
  and `research/solid-trust-graph-authz-design.md` § 6.0 (pod-side epic `sq-pfae`,
  issue #940) — the design records.
