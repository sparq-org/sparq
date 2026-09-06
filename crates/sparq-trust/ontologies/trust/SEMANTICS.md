<!-- [OPUS-4.8] sq-pfae.2 (issue #940 / gh-940). 🤖 SPARQ agent — trust-graph
authorisation. Written while Fable unavailable; flag for re-review when Fable returns. -->
# Trust-graph two-stratum semantics note (normative reference)

> **Status — DESIGN-FOR-REVIEW, NOT a shipped guarantee.** This is the *normative
> semantics reference* for the `trust:` vocabulary (`trust.ttl` in this directory). It
> pins the two-stratum (admission / derivation) semantics, the degenerate-`.acl`
> equivalence, and the strict-additivity property so the prototype phases (epic
> `sq-pfae`) have one place to point at. It is a **proposal** to take to the LWS and
> Solid working groups, not a security guarantee. The full motivation, soundness
> argument, and honest limitations live in `research/solid-trust-graph-authz-design.md`
> (this note is a focused, normatively-worded extract of §2.1, §2.3, §3, §5.2 and §7
> G6 — the design record remains authoritative where they differ).
>
> Model: Opus 4.8 (Fable unavailable — flag for re-review when Fable returns).

## 0. Scope and references

This note defines the **meaning** of a trust graph: what it admits, what it derives,
and the two structural properties a reviewer should be able to check by reading the
definitions alone (degenerate-`.acl` equivalence and strict additivity). It is
deliberately **doc + vocab only** (bead `sq-pfae.2`) — no new runtime capability. The
admission *gate* that realises stratum 1 is the opt-in `sparq-trust` crate
(`src/admit.rs`, PoC bead `sq-pfae.10`); the derivation *materialiser* that realises
stratum 2 is the shipped `sparq-solid` WAC/ACP reasoner.

| Concept here | Vocabulary term (`trust.ttl`) | Realised by |
|---|---|---|
| Admission gate | `trust:trustsSourceFor`, `trust:source`, `trust:forShape`, `trust:issuerKey`, `trust:scope`, `trust:freshWithin` | `sparq-trust::admit` (opt-in) |
| Stratum-boundary marker | `trust:admitted` | the gate tags admitted facts |
| Policy container / rule | `trust:TrustPolicy`, `trust:TrustRule`, `trust:Source` | `sparq-trust::policy` |
| Derivation | the local `.acl`/`.acr` + ABAC N3 rules | shipped `sparq-solid` materialiser |

## 1. The two strata

The access decision is a **two-stage, stratified** reasoning pipeline. The strata are
ordered: every relevant admission decision completes before any derivation rule reads
its output. The boundary between them is the marker `trust:admitted`.

```text
   attested statements (VC claims, signed graphs)
                 │
                 ▼
   ┌───────────────────────────────────────────────┐
   │  STRATUM 1 — ADMISSION  (NEW, opt-in)          │
   │  "is this fact from a source I trust for this  │
   │   statement-type?"                             │
   │  gate = matching trust rule (source × shape ×  │
   │  scope) ∧ CHECKED issuer signature ∧ freshness │
   │  ∧ not-revoked ∧ holder binding                │
   └───────────────────────────────────────────────┘
                 │  trust:admitted facts (issuer-tagged)
                 ▼
   ┌───────────────────────────────────────────────┐
   │  STRATUM 2 — DERIVATION  (EXISTING, shipped)   │
   │  the sparq-solid WAC/ACP N3 rules + any ABAC   │
   │  rule (age>18 ⇒ canAccess) over admitted facts │
   └───────────────────────────────────────────────┘
                 │
                 ▼
        <urn:sparq:auth>  (allow-list, fail-closed)
```

### 1.1 Stratum 1 — Admission (the new contribution)

A fact `S P O` carried by a presented, signed credential graph *G* is **admitted** —
i.e. enters stratum 2 tagged `trust:admitted` and bound to its issuer — **iff ALL** of
the following hold. (This is the conjunction worked through in design §3.1; the
soundness of each conjunct is §3.3.)

1. **A matching trust rule exists.** There is a `trust:TrustRule` whose `trust:source`
   names a `trust:Source` *S′*, whose statement-type (`trust:forShape`, or the
   `trust:forPredicate` sugar desugared to a single-predicate shape) the fact
   *satisfies* under the shipped `sparq-shacl` validator, and whose `trust:scope`
   covers the target resource.
2. **The issuer signature is CHECKED, never self-asserted.** *G* is verified to be
   signed by the key `trust:issuerKey` names for *S′*, over *G*'s RDFC-1.0 commitment.
   A graph merely *claiming* `trust:issuerKey …` proves nothing. *(Caveat: the
   `issuerKey → verifying-key` binding is operator-asserted by default — the live forgery
   vector D′, design §3.3. The opt-in `did` resolver (`sq-pfae.3`) binds it from a
   `trust:issuerDid` instead, narrowing — not eliminating — D′.)*
3. **Freshness and revocation pass.** The credential is within `trust:freshWithin` of
   `Session.now` and is not revoked. Both are **per-request side-conditions** —
   freshness is a Rust check (time is not a reasoner fact); a `not-revoked` guard, if
   used, must be NAF over an **input-only** `revoked` predicate (the shipped reasoner
   rejects NAF over *derived* predicates). See §3 and `sq-tu4e`.
4. **Holder binding.** The credential subject binds to the authenticated requester
   (v1: `credentialSubject == Session.agent`, the clear-WebID degraded path — `sq-wvne`
   / `sq-xc4y`). Presenting a third party's credential without holder binding does
   **not** admit the fact.

**Statement-type scoping is enforced HERE, at admission, not at derivation.** A source
trusted for `schema:age` cannot launder an `acl:agent` or `solidx:` triple in: those
predicates are not in its shape, so the admission test fails for them, and the
reserved-derivation-predicate guard additionally forbids any source from asserting
`solidx:`/`trust:admitted` internal vocabulary (design §3.3 item 2).

**v1 admits only DIRECTLY-attested facts of a trusted type** — no entailment
laundering (deriving `age` from an untrusted `birthDate` and inheriting trust). Trust
propagation through the closure is explicitly out of v1 scope (design §3.3, G3).

### 1.2 Stratum 2 — Derivation (the shipped substrate, unchanged)

The shipped `sparq-solid` materialiser runs the local `.acl`/`.acr` WAC/ACP N3 rules —
plus any ABAC rule such as `{ ?x schema:age ?y . ?y math:greaterThan 18 } => { ?x
auth:read R }` — over **the union of the existing trusted inputs and the
`trust:admitted` facts**, and materialises `<urn:sparq:auth>` exactly as today. Nothing
in this stratum changes: it consumes the admitted facts as if they had always been
trusted inputs, because stratum 1 has already vouched for them.

### 1.3 Why the ordering is load-bearing

The shipped reasoner is **no-retraction, stratified negation-as-failure**. Admission
must be stratified strictly ahead of derivation so scoped-NAF stays sound: a derivation
rule must never observe a fact mid-admission. **Open soundness question (`sq-xc4y`):**
the shipped auth view is materialised once, session-independently, whereas holder
binding and freshness are per-request — so a per-request, identity-bound admission
decision cannot simply sit frozen ahead of a materialise-once view. v1 must either
re-run admission per request for credential-gated resources, or split static admission
(signature / type-scope, materialise-time) from dynamic admission (holder / freshness,
query-time). This note records the constraint; it does **not** declare it solved.

## 2. The degenerate-`.acl` equivalence

> **Property (degenerate case).** A trust graph whose *only* statement trusts the
> `.acl`/`.acr` document for all access-control predicates **reproduces WAC/ACP exactly**
> (design §2.2, §5.2).

Concretely: let the trust policy contain a single `trust:TrustRule` whose `trust:source`
is the resource's own control document, whose statement-type covers the access-control
predicates, and whose `trust:scope` is the resource/container. Then stratum 1 admits
**exactly** the `.acl`/`.acr` rules the shipped loader already trusts (the control
document is, by construction, signed/authoritative for its own resource), and stratum 2
runs the identical WAC/ACP derivation over the identical input set. No external fact is
admitted (there is no other trust rule), so the materialised `<urn:sparq:auth>` is
identical to today's.

Two named special cases fall out:

- **WAC/ACP** is the degenerate case above: `.acl`/`.acr` as the one trusted source.
- **Solid-OIDC** is the *n = 1* case: one issuer, one statement-type (the identity
  assertion that the requester controls a WebID).

This is the precise sense in which the trust graph is a **conservative
generalisation**: today's single-trusted-source model is the one-rule instance of the
many-rule model. RBAC/ABAC then appear by *adding* rules (a role-assignment predicate
from HR; `schema:age` from a government) — never by changing how `.acl` is evaluated.

## 3. The strict-additivity property (G6)

> **Property (strict additivity, G6).** A pod with **no** trust graph behaves
> **exactly** as WAC/ACP do today (design §2.2 G6, §7).

This is stronger than the degenerate-`.acl` equivalence and is what makes the whole
proposal safe to adopt incrementally. It holds at **two** levels:

1. **Semantic additivity.** With an empty trust policy, stratum 1 admits **nothing**
   (no trust rule ⇒ no external fact passes the gate ⇒ fail-closed). Stratum 2 then
   runs over exactly the inputs it has today, so the access decision is **byte-identical**
   to current WAC/ACP. Adding a trust rule can only **admit more** facts for the reasoner
   to consider; it can never suppress an existing `.acl`/`.acr` grant or deny path — the
   admission stratum is **monotone-add** with respect to the derivation inputs. (Conflict
   between two *trusted* external attestations is a separate question with its own honest
   caveats — design §3.5, `sq-tu4e` — and does not affect additivity over the *empty*
   trust graph.)

2. **Build / dependency additivity (opt-in by construction).** Nothing in the
   workspace's default build depends on `sparq-trust`. `sparq-solid` pulls it in **only**
   behind its default-OFF `trust-graph` cargo feature, so the lean core
   (`sparq-core` / `sparq-engine` / `sparq-reason`) and the default `sparq-solid` build
   are byte-identical with or without the trust-graph work. A pod that never enables the
   feature pays nothing — no code, no dependency, no behaviour change. This is the
   feature-gating discipline of `AGENTS.md` (new capability ⇒ opt-in crate/feature; core
   stays lean) made concrete for this surface.

**Why a reviewer can trust this from the definitions alone:** strict additivity is not
an empirical claim about a running system; it is a structural consequence of (a)
fail-closed admission over an empty rule set and (b) the derivation stratum being the
*unmodified* shipped materialiser. Both are checkable by reading §1 above and the
`sparq-trust` Cargo manifest (`default = []`, the feature wiring in `sparq-solid`).

## 4. Honest limitations (do not over-read this note)

This note pins **semantics and additivity**, nothing about privacy. In particular:

- **No privacy / unlinkability / anonymity.** Admission verifies a CHECKED signature but
  the credential is admitted **in the clear** — the verifier learns the exact value
  (`age 25`, not "≥ 18"). This is **not** ZKAPs-grade unlinkable presentation.
- **Holder binding is the clear-WebID, non-anonymous degraded path** (`sq-wvne` /
  `sq-xc4y`).
- **Issuer keys are operator-asserted by default** — the live forgery vector D′. The
  opt-in `did` resolver (`sq-pfae.3`) binds the key from a `trust:issuerDid`
  (`did:key`/`did:web`), narrowing — not eliminating — D′.
- The ZK estate this design *could* compose with (`sparq-zk` / `sparq-zk-compose`) is
  research-grade and **externally unaudited** — external accredited-cryptographer
  sign-off is **pending** (`sq-qhy4`); `sparq-mpc` is honest-majority semi-honest only.
- Open problems respected as documented limitations, never silently solved: `sq-xc4y`
  (admission vs materialise-once), `sq-tu4e` (no in-reasoner NAF over derived facts;
  no deny-on-disagreement), `sq-l5og` (delegation invocation binding), `sq-wvne` (ZK
  privacy).

## 5. The certification-edge trust-graph closure (opt-in `cert-graph`, `sq-pfae.15`)

The `trustx:Certification` vocabulary (`framework_vocab`, design record
`research/trust-expression-spec.md` §3.4 / D4) says who vouches for whom, over what
scope, in what window — but has **no pod-side evaluation**: admission consumes a flat
`Vec<TrustRule>`. The `graph` module (behind the default-OFF **`cert-graph`** feature)
adds `derive_effective_rules`: a **depth-bounded (v1: depth-1), attenuation-only,
fail-closed** certification-edge closure that runs **AHEAD of** — and produces the rule
set consumed **verbatim by** — the UNCHANGED admission gate (stratum 1 above). It changes
**no** gate; it is a pure pre-processing step.

### 5.1 The certification edge

A `trustx:Certification` is modelled as a signed **edge** (`graph::Certification`): an
authority (the **certifier** — a framework operator / a higher issuer) attests that a
**certified issuer** is certified — `trustx:underFramework`, over `trustx:certificationScope`, within
`trustx:validFrom`/`validUntil` — to issue statements of that scope. A Trusted-List /
DIATF-register entry **is** one such edge. The edge is admitted into the closure ONLY if
a signature over its domain-separated `certification_message` (which binds the certifier,
the certified issuer **and its key**, the scope, and the window) **verifies under the
certifier's key** — a CHECKED signature, never a self-asserted `trustx:certifies` triple.

### 5.2 The HARD invariant — attenuation-ONLY (a broadening bug is privilege escalation)

Every derived rule satisfies `derived ⊆ (anchor ∩ cert scope ∩ validity window)`: a
certification can only **NARROW**, never **WIDEN**, the certifier's own authority. Concretely:

- **Anchor.** The certifier MUST be a **direct** anchor rule (`source` + matching key) in
  the input `direct_rules` — only authority the pod already holds can be conferred. (This
  is the depth bound made concrete: a *derived* rule is never re-used as an anchor, so the
  closure is a single pass and cannot transitively chain.)
- **Statement-type narrowing — target-set-CONTRAVARIANT, conformance-COVARIANT.** A SHACL
  shape selects focus nodes by the **UNION** of its target predicates
  (`sh:targetSubjectsOf` / `sh:targetObjectsOf` / `sh:targetNode` / `sh:targetClass` /
  `sh:targetWhere`, and the implicit-class typing) and then requires each to satisfy every
  **conformance** constraint (`sh:property` / `sh:minCount` / `sh:datatype` / …). These move
  in OPPOSITE directions: adding a conformance constraint **narrows** the admitted set (fewer
  nodes conform), but adding a target predicate **widens** it (more nodes are selected). So a
  cert shape narrows the anchor **only** when — proven by a **root-anchored, injective
  structural match** — its **selection/target set is a SUBSET** of the anchor's (targets may
  only shrink) **AND** its **conformance-constraint set is a SUPERSET** of the anchor's
  (constraints may only grow). Any **un-modelled selection predicate ⇒ fail-closed**
  (contributes nothing). `AnyServiceScope` imposes no narrowing of its own and inherits the
  anchor shape unchanged (never broadening it). Where narrowing cannot be *proven*, the edge
  contributes **NOTHING** (design §3.4: undecidable containment ⇒ NOT contained ⇒ contributes
  nothing — the fail-closed side).

  > A prior version treated **any** injective structural superset as a narrowing ("⊇ as a
  > constraint set ⇒ ⊆ as an admitted-node set"). That equivalence holds only for
  > *conformance* constraints; it is **UNSOUND for target predicates**, where an extra
  > `sh:targetSubjectsOf` triple is a **broadening**, not a narrowing. An escalated adversarial
  > review (`sq-pfae.15`) found the bypass: a cert = anchor shape **+ an extra
  > `sh:targetSubjectsOf schema:email`** was admitted as a "narrowing" and granted the
  > certified issuer authority over `schema:email` the certifier never held. The containment
  > check is now split into a contravariant target-set check and a covariant conformance
  > check, and the additive-target case is denied `EdgeRejection::Broadening`.
- **Resource-scope + freshness narrowing.** The derived rule keeps the anchor's resource
  `scope` (a cert narrows *who* and *what-type*, never widens *where*) and freshness
  `min(anchor.fresh_within, window)`.
- **Time.** An expired / not-yet-valid / inverted-window (missing positive status) edge
  ⇒ NOTHING (positive, existence-of-a-covering-window semantics; OWA/monotonicity, §3.3
  D3).
- **No cycle amplification.** A self-certifying edge, or one whose certified issuer already
  anchors the certifier, is dropped — it can add no new authority and is the shape a cycle
  would use to launder a broadening.
- **Strict additivity.** Zero certifications (or none surviving the gate) ⇒ output is
  `direct_rules` **byte-identical**; a `depth_bound` of `0` short-circuits identically.

### 5.3 Honest scope

`derive_effective_rules` makes **no** cryptographic-soundness, anonymity, or
unlinkability claim. It is a **fail-closed attenuation** layer — a CHECKED certifier
signature over a canonical message plus scope/time narrowing (framework trust is
**anchored, not proven**, §7.2). It defeats a *broadening* or *forged-edge* escalation
**given honest anchor keys**; the certifier's own key is still operator-/DID-asserted (the
live forgery vector D′, `sq-pfae.3`), and the shared ZK signature primitive is externally
**unaudited** (`sq-qhy4`; `sparq-mpc` semi-honest only). The load-bearing evidence is the
adversarial edge-forgery matrix in
`crates/sparq-trust/tests/certification_graph_e2e.rs` (forged / broadening / expired /
revoked / cyclic / over-depth / meta-scope-escalation edges ALL deny + a positive
end-to-end case).

See `research/solid-trust-graph-authz-design.md` §7 and
`research/trust-expression-spec.md` §7 for the full, audited limitations lists, and
`crates/sparq-trust/README.md` for the PoC's honest scope.
