# sparq-solid vocabulary-gap design notes: `acl:accessToClass`, `acp:vc`, custom ACP mode IRIs, nested `acl:agentGroup` chains

Status: **design notes / research record** — *not* an implementation plan and **not a
cutover gate**. <!-- [OPUS-4.8] sq-3jtd.7 / gh-55 -->

This record is the per-gap design analysis the parent scope record
([`research/sparq-solid-scope.md`](./sparq-solid-scope.md) area 3, sequenced task sq-3jtd.7)
defers to: *"each needs its own design note before implementation; do not start blind."* It
covers the four still-open vocabulary gaps after the near-term ones landed
(`acp:CreatorAgent`/`OwnerAgent` — sq-3jtd.5; `acp:issuer` — sq-3jtd.6):

| Gap | Spec | Today | This note's verdict |
|---|---|---|---|
| `acl:accessToClass` | WAC (extension) | missing | **research-open — breaches §2.4** |
| `acp:vc` | ACP | **implemented** (sq-ysv3u — see §2 *Outcome*) | *(was: research-open — needs VC verification machinery)* — the plaintext/trust-the-issuer principal shipped once `sparq-vc` pinned the verifier interface; ZK still gated on `sq-qhy4` |
| custom ACP mode IRIs | ACP | partial (4 fixed modes) | **near-term feasible — bounded refactor of the auth-view predicate space** |
| nested `acl:agentGroup` / `vcard:hasMember` chains | WAC | partial (one hop) | **near-term feasible — but a deliberate spec-conformance decision precedes it** |

It does **not** restate the architecture; it builds on, and cross-references:

- the storage model, rule strata, NAF discipline, and the **§2.4 reasoner/content security
  boundary** of [`research/solid-access-control-design.md`](./solid-access-control-design.md);
- the feasibility framing and the "what stays in PSS" boundary of
  [`research/sparq-solid-scope.md`](./sparq-solid-scope.md) area 3;
- the single-prover ZK estate for `acp:vc` (skill `verifiable-credentials-zk`, the `sparq-zk*`
  crates) — see §2 below.

A shared constraint recurs in three of the four gaps and is worth stating once: the materializer
sees **only access-control inputs** — `.acl`/`.acr` graphs, `acl:agentGroup` group documents
(fragment stripped → graph name), and loader-synthesized structural facts. **Pod content graphs
are deliberately excluded** (design doc §2.4): otherwise any agent who can write a document could
embed `acl:`/`solidx:` triples and grant themselves access. Two of the four gaps
(`acl:accessToClass`, `acp:vc`) were research-open *precisely because a sound implementation has to
read something the boundary currently walls off* — typed pod content, or an externally-issued
credential — and doing that without re-opening the smuggling surface is a design problem, not a
vocab addition. `acp:vc` has since been implemented on exactly that basis: the credential is
verified OUTSIDE the boundary and enters as a caller-asserted trusted fact, never as content
(§2 *Outcome*).

---

## 1. `acl:accessToClass` — class-membership-gated authorization

### What the spec asks for

WAC's `acl:accessToClass` (an extension predicate, not in the WAC core editor's draft's
required set) lets an `acl:Authorization` apply to **every resource that is an instance of a
given RDFS/OWL class**, rather than to an enumerated `acl:accessTo <r>` resource. Concretely:

```turtle
<#classAuth> a acl:Authorization ;
    acl:accessToClass <http://schema.org/MedicalRecord> ;
    acl:agent <https://clinician.example/profile#me> ;
    acl:mode acl:Read .
```

grants the clinician `Read` on *every pod resource typed* `schema:MedicalRecord`.

### Why this is research-open: it breaches §2.4

The blocker is structural, not mechanical. To decide whether `<#classAuth>` applies to a
resource `<r>`, the materializer must evaluate a **class-membership join against the resource's
own content**: `<r> rdf:type schema:MedicalRecord` (transitively, under `rdfs:subClassOf`).
But:

1. **`rdf:type` triples live in pod *content* graphs, which §2.4 excludes from the reasoner.**
   This exclusion is load-bearing: today an agent with `Write` on `<r>` cannot influence who
   *else* may read `<r>`, because the authorizer never looks at `<r>`'s triples. The moment
   class membership gates access, **the writer of `<r>` controls its `rdf:type` and therefore
   controls the access decision** — they can type their resource `schema:MedicalRecord` to pull
   in the clinician grant, or *untype* it to evade an `accessToClass`-based denial. That is the
   exact privilege-escalation surface §2.4 exists to close.

2. **The membership join is RDFS-recursive over `rdfs:subClassOf`**, so it would also need an
   ontology — itself pod content or an external vocabulary — fed to the reasoner, widening the
   trusted input set further.

### Design options (none cheap, all surveyed — do not start blind)

- **(A) Reject as out-of-scope, permanently.** The honest default. `acl:accessToClass` is an
  *extension*, not required for WAC conformance (design doc §3.6 already records it as out of
  scope), and the reference servers' support is uneven. The cost/risk of re-opening §2.4 is not
  justified by an extension predicate. **Recommended unless PSS presents a concrete need.**

- **(B) Trusted-caller class facts, mirroring `acp:CreatorAgent` (sq-3jtd.5).** PSS, which
  controls the storage layer, supplies a *trusted* `<r> solidx:ofClass <C>` fact through the
  same `AccessProvenance` channel that carries creator/owner facts — **never read from `<r>`'s
  content graph**, exactly as creator/owner are never read from the resource. The loader's
  existing `is_reserved_derivation_predicate` guard already hard-rejects a forged
  `solidx:ofClass` smuggled into an `.acl`/content graph (it is `solidx:`-space), so the
  smuggling surface stays closed. A new WAC rule grants on the join of `acl:accessToClass ?C`
  with the trusted `solidx:ofClass ?C` fact, optionally transitively closed over a
  *trusted-supplied* `rdfs:subClassOf` set. **This keeps §2.4 intact** because the class fact is
  storage metadata the trusted channel vouches for, not writer-controlled content. The open
  question is whether PSS actually *knows* resource class membership as trusted metadata (it
  knows creator/owner because it minted the resource; it does **not** obviously know
  `rdf:type` without reading content) — so (B) is only viable if PSS opts to compute and supply
  it as a first-class, trust-channel fact. **Feasible but predicated on a PSS product decision.**

- **(C) Read content under an explicit, audited trust escalation.** Feed `rdf:type` from content
  into a *separate, clearly-labelled* materialization stratum that only ever produces
  `accessToClass` grants, with the residual escalation risk documented. **Rejected** — it
  re-opens the very surface §2.4 closes and the "writer controls their own access" anomaly is
  not acceptable for a security oracle.

### Verdict

**Research-open, lowest priority.** Default to **(A) out-of-scope** (it is an extension). If PSS
ever needs it, **(B) trusted class facts** is the only §2.4-preserving path, and it is gated on
PSS being able to supply class membership as trusted metadata. **(C) is rejected.** No
implementation should start until PSS confirms a need *and* commits to the trusted-fact channel.

---

## 2. `acp:vc` — Verifiable-Credential-gated matcher

### What the spec asks for

ACP's `acp:vc` constrains a matcher on a **Verifiable Credential** the requesting agent
presents: the matcher is satisfied iff the agent holds a VC meeting the policy's stated
requirement (issuer, type, claim values). It is ACP's bridge from identity-based to
*attribute/claim-based* authorization.

### Why this is research-open: it needs verification machinery the authorizer does not have

The authorizer today answers a **pure graph question** over already-resolved
`(agent, client, issuer)` principals: it never validates a signature, checks a revocation
status, or reasons about a credential's claims. `acp:vc` needs *all three*:

1. **Cryptographic verification.** The presented VC's proof (a data-integrity proof / JWS /
   BBS+ selective disclosure) must verify against the issuer's key. That is exactly the
   machinery the sparq **ZK/VC estate** already reasons about — see the `verifiable-credentials-zk`
   skill (BBS+ vs SD-JWT-VC vs EdDSA trade-offs, Pedersen/Poseidon commitments, the
   credential→circuit encoding) and the `sparq-zk*` crates. `acp:vc` is therefore not a
   sparq-solid-local feature; it is a **composition point between sparq-solid and the VC estate**.

2. **Trust anchoring.** Which issuers are acceptable, and how their keys are resolved, is policy
   + deployment configuration — analogous to `acp:issuer` (the OIDC-issuer dimension already
   modelled, sq-3jtd.6) but for *credential* issuers, with a richer claim surface.

3. **Where verification runs vs where the graph decision runs.** The honest architecture split:
   the **VC must be verified *before* the principal reaches sparq-solid**, exactly as
   Solid-OIDC/DPoP authentication is done in PSS before a `Session` is handed in (scope doc
   "what stays in PSS"). sparq-solid should *not* verify signatures inline in the N3
   materializer — it has no business holding verification keys, and a per-request crypto check
   does not fit the materialize-once/cache model.

### Design sketch (the only §2.4- and architecture-respecting shape)

The clean decomposition mirrors how `acp:client`/`acp:issuer` already work — **lift the verified
claim into the principal**, do not verify in the reasoner:

- **PSS (or the VC estate) verifies the presented VC** and extracts the satisfied,
  verified claim set into the `Session` as a new principal dimension — e.g. a set of
  *verified credential predicates* `vc:type=<T>&issuer=<I>&claim…`, each minted into a
  reserved `urn:sparq:` principal exactly like the pair/triple principals (and subject to the
  same reserved-encoding injectivity guard, design doc §2.3).
- **The ACP rules add an `acp:vc` matcher dimension** that accepts a candidate iff the verified
  claim principal is in the session's set — a *graph* check over a *trusted, pre-verified* input,
  identical in shape to the existing `acp:client`/`acp:issuer` candidate enumeration (design doc
  §3.4). No crypto in the reasoner; the trust boundary is the `Session` ingestion point.
- **Selective disclosure / ZK** (BBS+ or a Noir circuit proving "holds a credential satisfying
  P" without revealing the credential) is the *aspirational* upgrade: the verified-claim
  principal would be the public output of a proof rather than a plaintext claim. This is where
  `acp:vc` "ties to the ZK/VC estate" — but it is **strictly downstream** of the plaintext-claim
  version above, and should not gate it.

### Open questions (must be answered before implementation)

1. **Claim-matching expressivity.** ACP `acp:vc` can require structured claim *values*
   (e.g. `age ≥ 18`), not just a credential *type*. The candidate-enumeration model is finite
   only over enumerable matcher values; range/threshold claims would need the same treatment as
   any value constraint and may not reduce to a finite candidate set. Scope to **exact-match
   claim predicates first**; range claims are a separate analysis (and a natural ZK-circuit
   target — `verifiable-credentials-zk` + `noir-circuit-patterns`).
2. **Combinatorial cost.** Adding a fourth principal dimension on top of `(agent, client,
   issuer)` extends the candidate product again. `acp:issuer` (sq-3jtd.6) already doubled the
   session lookups (≤6 → ≤12); a VC dimension multiplies by `{matcher VC values, top}`. Bounded,
   as the other dimensions are, but the session-expansion budget should be re-measured.
3. **Revocation/freshness.** A materialize-once auth view caches a grant; a revoked VC must not
   keep granting. Either the verified-claim principal carries an expiry the session honours, or
   re-verification happens per request *outside* the cached view. This is a PSS-side lifecycle
   concern but it constrains the principal design.

### Verdict

**Research-open, large, and cross-estate.** The sound architecture is **verify-in-PSS/VC-estate,
match-in-rules** — sparq-solid never verifies; it consumes a verified-claim principal exactly as
it consumes a verified `(agent, client, issuer)` triple today. The plaintext-claim version is a
bounded extension of the existing candidate model; the ZK/selective-disclosure version is a
genuine ZK-estate composition and strictly downstream. **Do not implement until (a) the VC-estate
verifier interface is pinned and (b) PSS confirms the claim-matching expressivity it needs.**

### Outcome — implemented [SONNET-4.6] sq-ysv3u (issue #2935)

Precondition (a) was met: `sparq-vc` (sq-ylbrq, issue #908) pinned the verifier interface —
W3C Data Integrity `eddsa-rdfc-2022` over RDFC-1.0 with `did:key`/`did:web` resolution. The
shipped implementation follows the sketch above with **one deliberate simplification**, and
one correction the analysis above did not anticipate:

- **The verified-claim principal is a trusted FACT, not a minted principal.** Rather than
  minting a fourth `urn:sparq:` principal dimension (which would multiply the candidate
  product, open question 2), the caller asserts `(agent WebID, requirement IRI)` holdings
  through a `VerifiedCredentials` map — the exact analogue of `AccessProvenance` — and the
  loader synthesizes `<agent> solidx:holdsVc <requirement>`. The rules then INTERSECT the
  matcher's `acp:agent` accept-set with the holders of its `acp:vc` requirements (the
  `rawAgentP`/`acceptsAgentP` split in `rules/acp-a.n3`). There is **no fourth principal
  dimension**: the candidate product stays `(agent, client, issuer)` and `Session` is
  unchanged, so the ≤12 per-session grant lookups open question 2 worried about do not grow
  at all. The residual cost is narrower and stays inside the agent dimension: each holder of
  a matcher's requirement becomes one additional candidate *agent* for that policy, exactly
  as a concrete `acp:agent` WebID already does. A policy with no `acp:vc` is unaffected.
- **Claim-matching expressivity (open question 1) is resolved by scoping, not by (b).** The
  `acp:vc` object is an opaque **requirement IRI** matched by exact equality; the rules never
  interpret claim values, so the range/threshold problem does not arise inside the reasoner.
  Deciding *whether a credential satisfies a requirement* is the verifier's job, where a
  `VcRequirement` states issuer + credential type + exact-match claims. PSS confirmation was
  therefore not a blocker for the graph half — a richer requirement language is an additive
  change to the verifier alone.
- **Revocation/freshness (open question 3) is NOT solved.** A holding lasts until the caller
  re-materializes. This is documented, not fixed.
- **The gap was not merely missing — it was FAIL-OPEN.** `acp:vc` was an *unrecognized*
  attribute, so a matcher carrying it satisfied the "no `acp:agent` ⇒ agent-unconstrained"
  rule and accepted `auth:Public`: a credential-gated policy granted **everyone, anonymous
  included**. The wrong-direction error for a security oracle. Correcting that is
  unconditional (no feature flag), and is the load-bearing half of the change.
- **ZK stays downstream, as this record says it should.** Only the trust-the-issuer backend
  is wired (opt-in `acp-vc` feature). A zero-knowledge / selective-disclosure backend remains
  gated on the ZK estate's external accredited-cryptographer audit (`sq-qhy4`).

---

## 3. Custom ACP mode IRIs

### What the spec asks for

ACP's `acp:allow`/`acp:deny` connect a policy to **mode IRIs**, and the ACP spec declares the
mode space **open** (design doc §1.4: *"mode IRIs are open"*) — a deployment may define and use
its own modes beyond the four conventional `acl:Read`/`Write`/`Append`/`Control`. The prototype
maps only those four (`authindex.rs` `Mode` enum + `from_mode_iri`; the rules emit a fixed
`auth:read`/`write`/`append`/`control` predicate set, design doc §3.6).

### Why this is the most tractable of the four

Unlike the other three, custom modes **do not touch §2.4 at all** — a mode IRI is policy data in
the `.acr`, already a trusted reasoner input. The only thing fixed is the **auth-view predicate
space**: the rules hardcode the four `auth:` mode predicates, the `Mode` enum hardcodes four
variants, and `Mode::from_pred`/`from_mode_iri` (`authindex.rs:140-163`) hardcode the IRI↔enum
map. There is no soundness barrier — only a closed enumeration to open up.

### Design: generalize the mode from a fixed enum to an IRI

The clean shape is to **stop materializing one auth predicate per mode** and instead carry the
**mode IRI as a term**:

- **Auth view.** Replace the four `auth:read`/… predicates with a single relation
  `?principal auth:granted ( ?resource ?modeIri )` (or, to keep it triple-shaped,
  `?principal auth:grantedMode ?gm . ?gm auth:onResource ?r . ?gm auth:modeIri ?m`). The grant
  rules become **mode-generic**: one rule that fires for *any* `acp:allow ?m` instead of four
  mode-specialized rules. This *shrinks* the rule set (the WAC/ACP grant matrices currently
  repeat the same body ×4 modes) while making the mode space open.
- **`Mode` type.** Replace the closed `enum Mode { Read, Write, Append, Control }` with an IRI
  newtype `Mode(IriString)` plus `const` constructors for the four standard IRIs (preserving the
  ergonomic `Mode::Read` etc. and the `Copy`-friendly cache key — though an interned IRI id, not
  a raw enum, becomes the cache key). `from_pred`/`from_mode_iri` become identity/parse over the
  IRI rather than a fixed match.
- **WAC interaction.** WAC's `Control` has *special semantics* (it grants read/write on the
  `.acl` document, not the resource — design doc §3.3, `authindex.rs:115`). That special-case
  rule must be preserved as a named rule keyed on the `acl:Control` IRI specifically; all *other*
  modes (standard or custom) flow through the generic grant. WAC itself does **not** have an open
  mode space (its modes are the four), so custom modes are an **ACP-only** generalization — the
  WAC materializer keeps its fixed four.

### Risks / costs (bounded, all engineering)

1. **Public API churn.** `Mode` is a public type (`authindex.rs`, re-exported, with a doctest and
   the `Need`/`accessible`/`update_as` surface keyed on it). Changing `enum Mode` → an IRI
   newtype is a **breaking public-API change**, so it triggers the **G2 public-api→`SKILL.md`
   rule** and a README/`cargo doc` update. The migration should keep `Mode::Read` etc. as
   associated constants so existing callers (PSS via the four standard modes) compile unchanged.
2. **Cache-key cost.** The per-session cache is keyed on `(principal, Mode)`. An IRI-keyed mode
   means interning custom mode IRIs into the dict and keying on the id — a small, measured change,
   not a redesign.
3. **PSS need is unproven.** PSS today speaks the four WAC modes. Custom ACP modes are a
   *capability* the design always claimed ("the design supports any mode IRI") but which no known
   consumer exercises. Implementing it is low-risk but should be **demand-driven** — it is the
   one gap here that is "feasible but currently speculative" rather than "blocked."

### Verdict

**Near-term feasible — the only one of the four with no soundness barrier.** It is a bounded
refactor (generic grant rules + IRI-valued `Mode`) that *simplifies* the rule set, gated only on
(a) a real consumer need and (b) the public-API/`SKILL.md` migration discipline. Recommended
sequencing: do this **only when a deployment actually defines a custom mode**, because it is pure
generalization with a public-API cost and no current user.

---

## 4. Nested `acl:agentGroup` / `vcard:hasMember` chains

### What the spec asks for, and what we do today

WAC `acl:agentGroup <G>` grants to the members of a `vcard:Group` `<G>`, where membership is
`<G> vcard:hasMember <agent>`. The shipped rule resolves **exactly one hop** (`wac.n3:53`):

```n3
{ ?auth a acl:Authorization . ?auth acl:agentGroup ?g . ?g vcard:hasMember ?a . }
=> { ?auth solidx:grantsAgent ?a . } .
```

A *nested* group — where `<G> vcard:hasMember <H>` and `<H>` is itself a group with its own
`vcard:hasMember` agents — is **not** closed over: `<H>`'s members are not pulled in.

### Is nesting even WAC-conformant? (the decision that precedes implementation)

This is the load-bearing question and the reason this is a *design note*, not a one-line rule
change. **`vcard:hasMember` does not carry transitive semantics**: vCard's `hasMember` relates a
group to *an entity in the group*, with no spec statement that a member-that-is-a-group's members
are transitively members of the outer group. The WAC editor's draft describes `acl:agentGroup`
as pointing at a group document listing members via `vcard:hasMember`; it does **not** mandate
recursive expansion. So:

- **Recursive group expansion is a *semantic choice*, not obviously a conformance requirement.**
  Implementing it could make sparq-solid grant access the reference servers (CSS/ESS) do **not**
  grant — which for a **security oracle is the wrong-direction error** (granting more than the
  reference). The whole point of the conformance harnesses (sq-3jtd.8/.9) and the aspirational
  CSS differential oracle is to *not* drift from the reference evaluator's decisions.
- Therefore the design note's first deliverable is a **decision, validated against the reference
  evaluators**: does CSS/ESS recursively expand nested `vcard:hasMember`? Until that is checked,
  closing the chain risks *over-granting*. The conservative, sound default — and the current
  behaviour — is **one hop only**.

### If (and only if) nesting is confirmed in-scope: the implementation is trivial

The mechanism is already present — group documents are loaded as graphs and fed to the reasoner
(design doc §2.4), and the rule set is N3 with native fixpoint recursion. Closing the chain is a
two-rule transitive closure over an *intermediate* membership predicate, mirroring the existing
`solidx:ancestor` closure (design doc §3.2):

```n3
# base: a direct member is an effective member
{ ?g vcard:hasMember ?m . } => { ?g solidx:effMember ?m . } .
# step: a member that is itself a (loaded) group contributes its effective members
{ ?g solidx:effMember ?h . ?h solidx:isLoadedGroupDoc true . ?h vcard:hasMember ?m . }
=> { ?g solidx:effMember ?m . } .
# grant over the transitive closure
{ ?auth acl:agentGroup ?g . ?g solidx:effMember ?a . } => { ?auth solidx:grantsAgent ?a . } .
```

### The two real risks (both must be handled, both inform the rule above)

1. **Cycles.** `<G> hasMember <H>`, `<H> hasMember <G>` must terminate. N3's no-retraction
   fixpoint (design doc §1.4) *does* terminate on a monotone closure — `solidx:effMember` only
   grows and the value space is finite (the loaded group members) — so a cycle reaches fixpoint
   without infinite regress. This is the same reasoning that makes the `solidx:ancestor` closure
   safe. **Worth an explicit cyclic-group regression test.**

2. **§2.4 boundary — which `vcard:hasMember` edges are trusted.** A one-hop rule reads
   `?g vcard:hasMember ?a` only from the **loaded group document** `<g>` (referenced by
   `acl:agentGroup`, fragment stripped → graph name — design doc §2.4 explicitly allows group
   docs as reasoner input). A *transitive* rule must not blindly follow `vcard:hasMember` edges
   into **arbitrary pod content**: if `<H>` is just some resource a writer controls, its
   `vcard:hasMember` triples are writer-controlled and following them re-opens the §2.4
   escalation surface. The rule must therefore only expand a nested group `<H>` when `<H>` is
   **itself an `acl:agentGroup`-referenced, loaded group document** — that is the
   `?h solidx:isLoadedGroupDoc true` guard in the step rule above, a fact the loader synthesizes
   for exactly the graphs it loaded as group documents (the trusted set), never for content
   graphs. **This is the subtle part and the reason this needs a design note, not a one-liner.**

### Verdict

**Near-term feasible *as a closure*, but gated on a conformance decision first.** The N3
mechanism is trivial and terminates safely; the two real tasks are (1) **confirm with CSS/ESS
that recursive expansion is the reference behaviour** before changing decisions in the
over-granting direction, and (2) **bound the transitive expansion to loaded group documents**
(a loader-synthesized `solidx:isLoadedGroupDoc` guard) so it cannot follow writer-controlled
`vcard:hasMember` edges in pod content. Until (1) is answered, the sound default stays **one hop**.

---

## Summary of verdicts and prerequisites

| Gap | Verdict | Hard prerequisite before any implementation |
|---|---|---|
| `acl:accessToClass` | research-open, lowest priority | a PSS need **and** a commitment to supply class membership as a *trusted* `solidx:ofClass` fact (option B); else stay out-of-scope (A). Reading content (C) is **rejected**. |
| `acp:vc` | **DONE** ([SONNET-4.6] sq-ysv3u) — was: research-open, large, cross-estate | prerequisites met: `sparq-vc` pinned the verifier interface, and the verify-outside / match-in-rules split shipped as a trusted `VerifiedCredentials` channel + exact-IRI requirement matching. ZK selective-disclosure remains downstream (`sq-qhy4`). See §2 *Outcome*. |
| custom ACP mode IRIs | near-term feasible (no soundness barrier) | a real deployment defining a custom mode + the G2 public-API/`SKILL.md` migration (`Mode` enum → IRI newtype, generic grant rules) |
| nested `acl:agentGroup` chains | near-term feasible as a closure | (1) confirm recursive expansion is CSS/ESS reference behaviour (don't over-grant); (2) bound expansion to loaded group docs (`solidx:isLoadedGroupDoc` guard) |

**Cross-cutting principle (restated):** three of the four gaps are blocked or constrained by the
**§2.4 reasoner/content boundary**. The pattern that has worked twice already (creator/owner facts
in sq-3jtd.5, the `(agent, client, issuer)` principal in sq-3jtd.6) is the template for the
tractable cases: **lift a trusted, externally-established fact into a reserved-namespace principal
or `solidx:` fact at the trusted ingestion point, then match it in the rules with a pure graph
check** — never verify, type-check, or read content inside the materializer. `accessToClass`
option (B) and `acp:vc`'s verified-claim principal both follow this template; custom modes and
nested groups stay entirely within the existing trusted-input set.

## Cross-references (do not duplicate)

- Parent scope record: [`research/sparq-solid-scope.md`](./sparq-solid-scope.md) area 3
  (sequenced task **sq-3jtd.7** — this note is its deliverable).
- Architecture / §2.4 boundary / rule strata / NAF discipline:
  [`research/solid-access-control-design.md`](./solid-access-control-design.md) §§1.4, 2.4, 3.x, 7.4.
- Landed siblings whose pattern this note generalizes: **sq-3jtd.5** (`acp:CreatorAgent`/
  `OwnerAgent` via trusted `AccessProvenance` facts), **sq-3jtd.6** (`acp:issuer` pair→triple
  principal).
- `acp:vc` ZK/VC estate: skill `verifiable-credentials-zk`, `noir-circuit-patterns`, the
  `sparq-zk*` crates (single-prover ZK over committed RDF/credentials).
- Conformance / no-over-grant discipline for the nested-group decision: **sq-3jtd.8** (WAC
  decision-parity harness), **sq-3jtd.9** (ACP harness + aspirational CSS differential oracle).
