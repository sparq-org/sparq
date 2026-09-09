# Trust-graph authorisation for Solid/LWS: trusted-source-scoped statements + rules

Status: **design-for-review** (research record). This is a *proposal* to be taken to the
LWS and Solid working groups, not a shipped feature and not a security guarantee. It builds
on, and composes with, the *shipped* Solid access-control substrate in `crates/sparq-solid`
([`research/solid-access-control-design.md`](solid-access-control-design.md)) and the
research-stage ZK/credential estate in `crates/sparq-zk` / `crates/sparq-zk-compose`. The
privacy/unlinkability half of this design depends on that ZK estate, whose external
accredited-cryptographer sign-off is **pending** (bead `sq-qhy4`) and whose MPC layer is
honest-majority semi-honest only — see [§7 Honest limitations](#7-honest-limitations--comparison).
No performance numbers are quoted here; any timing taken on the work box is non-canonical.

<!-- [OPUS-4.8] lead-designer trust-graph authz design record; companion to the six in-flight
prior-art surveys (PRs #942/#943/#945/#946/#947/#949). §§2.2/5/7 amended after a four-lens
adversarial review (gh-940): corrected the hidden-issuer "not reached" factual error,
disentangled ZKAPs from IETF Privacy Pass, conceded the per-(source,type) novelty is
integration not trust-model, and recorded the admission-vs-materialize / delegation-invocation
/ conflict-semantics / 3-part-ZK-composite open problems (sq-xc4y / sq-l5og / sq-tu4e /
sq-wvne). [OPUS-4.8] revision: §2.3 rewritten as the MINIMAL ORTHOGONAL ONTOLOGY (ten-term
irreducible core + irreducibility argument + forPredicate→forShape desugaring as sugar, not a
primitive); new §5.4 EXPRESSIVE-COMPLETENESS coverage tables (every ZKAPs + every RFC
9576/9577/9578 concept → ontology / zk-comp / gap, honest per the audit); new §6.0 PoC SPEC
(age>18 end-to-end admission+N3-merge over the shipped sparq-solid/-reason/-canon/-zk/-shacl
estate + new opt-in `sparq-trust` crate); new §7.6 load-bearing-claims-a-skeptic-should-attack
(de-dup-lossless / coverage-complete / soundness). [OPUS-4.8] FINAL revision (two-lens
adversarial fold-in, gh-940): RECLASSIFIED every *presentation*-unlinkability coverage row
(§5.4.1 Origin-client/Attester-origin, §5.4.2 Presentation/Redemption-context/Cross-origin/
Attestation-reveals-no-identifying-info) from `zk-comp` to `gap` — they are blocked by the §3.4
clear-WebID holder binding (an ARCHITECTURE change, not a ZK-composition step), per the
privacy-layer-deliverability lens; added the §5.4.3 item-5 architecture-change gap + the
privacy-layer "promissory/blocked/aspirational, NOT achievable under current design" footnote;
clarified the §5.4.1 Issuer-client row (in-the-clear current / undisclosed-key `sq-z9l` built-
but-not-yet-sound); strengthened §7.1-B to an architecture-change blocker; reframed §7.6 to flag
the doc as SELF-ADVERSARIAL (the primary load-bearing claims to attack are the §7.1–7.4
concessions audited for adequacy — sq-xc4y/l5og/tu4e/wvne — NOT invented losslessness/complete-
ness claims); sharpened §6.0 into an exact build brief (sparq-trust module surface + admission
evaluation algorithm pseudocode + open-problem hooks the PoC must respect, not silently solve).
The de-dup/coverage claims are kept bounded (predicate-only direction; structural-layer only),
NOT overclaimed. -->

GitHub issue: [#940](https://github.com/sparq-org/sparq/issues/940).

## 0. The request (verbatim)

> There is an authorisation system design that I would like to prototype as we do this to
> propose to the LWS and Solid working groups. The idea is that often we want to provide
> RBAC and ABAC on the solid ecosystem; and the only way of coming close to that is using
> the acp:vc matchers which are very hacky because the matcher is "do you have a verifiable
> credential of this exact access grant shape." Specifically, the way that the access control
> system would work is by having a "trust graph" which is the set of statements and/or rules
> that are used to determine the sources that a given storage server or resource will trust
> for making statements about access control. These rules can be specific enough to say that
> only some types of statements are trusted from particular sources. In Solid today the way
> that this effectively works is that the .acl and the .acl of parent resources is the one
> and only trusted source of information. If we consider the case of, e.g., wanting to enable
> people over the age of 18 to access a resource, then we need to think about this a little
> differently. In this case the trust graph may trust the .acl document and statements about
> age issued by trusted governments. This then lets someone present a verifiable credential
> where the age statement `<Jesse> <age> 25 .` can be merged with the .acl rule saying
> `{ ?x <age> ?y . FILTER(?y > 18) } => { ?x <canAccess> <resourceX> }` to get to the fact
> that I can access the resource. ... support capability delegation for (human and AI)
> agents. Show that with the right ontology terminology, this addresses a superset of the
> features of technologies such as ZKaps.

## 1. Motivation — why `acp:vc` is a hack, and what RBAC+ABAC on Solid needs

### 1.1 The deployed substrate, verified

Two things are true in Solid today and in `sparq-solid` specifically; both were checked
against the code, not assumed:

- **`.acl`/`.acr` is the one and only trusted source.** `sparq-solid` materialises a
  `<urn:sparq:auth>` allow-list by running WAC + ACP as N3 rules
  (`crates/sparq-solid/rules/*.n3`) over **only** the `.acl`/`.acr`/group graphs plus
  loader-synthesised structural facts; pod *content* is deliberately excluded from the
  reasoner (`research/solid-access-control-design.md` §2.4). The author of the control
  document is the sole trust root.
- **`acp:vc` matches a credential's *type IRI*, not its *claims*.** ACP's `acp:vc` matcher
  (per the ACP spec) is satisfied when a presented Verifiable Credential *is of a stated
  type*; it does **not** read the credential's contents. You can require "a VC of type
  `:AgeCredential`"; you cannot say "…whose `age` is greater than 18". That is the
  maintainer's "exact-credential-shape hack": to approximate ABAC you must mint a credential
  whose very *shape* encodes the grant, and the issuer effectively makes the access decision
  for you. **`acp:vc` is not implemented in `sparq-solid` at all** — `grep -rc "acp:vc"
  crates/sparq-solid/` returns zero — so claim-level credential reasoning is genuinely new
  here, not an extension of existing code (correction **C2**, §1.3).

### 1.2 What RBAC and ABAC actually require

- **RBAC** (ANSI/INCITS 359): access is set-membership in named role↔permission relations.
  On Solid you can fake a role with an `acl:agentGroup`, but the group membership has no
  *provenance* — you cannot say "trust HR for the *manager* role but trust a government for
  *over-18*". A role should be a *derived view* of source-attested attributes, not a flat,
  single-authority list.
- **ABAC** (NIST SP 800-162): access is a boolean function of subject/object/environment
  *attributes*. The standard names "trusted attribute sources / Attribute Authorities" as a
  first-class operational concern — but leaves *which authority is trusted for which
  attribute* **out of band**, a deployment decision, not a machine-checkable statement.
  XACML 3.0 approximates this only at *policy* granularity (its `PolicyIssuer` +
  reduction); VC trust registries do it only as a *flat issuer list* per credential *type*.
  The trust-management literature **does** express per-(source, attribute-type) trust — RT and
  PERMIS do, as Datalog (§2.2, §9.1) — but **not as RDF a *deployed Solid pod* reasoner merges
  with the local `.acl` rules and binds to signed-graph admission**. That integration is the
  gap, not the relation itself.

That object — *"resource R trusts source S for statements of type T"*, rendered as RDF the
**shipped** Solid reasoner consumes and merges with `.acl` — is what this design adds (an
integration / standards-fit contribution, not a new trust-model primitive; §2.2). ABAC then
falls out: the age rule
`{ ?x age ?y . FILTER(?y > 18) } => { ?x canAccess R }` fires over attested attributes; and
RBAC falls out as the special case where T is a role-assignment predicate.

### 1.3 Corrections to the brief's premise (be honest up front)

- **C1.** sparq-solid's `acp:issuer` dimension is **not** per-statement trust. It is the OIDC
  issuer that vouched for the requester's WebID (`Session.issuer`, matched as an IRI in
  `authindex.rs`; `ANY_ISSUER` is the top). It is an *identity* dimension, not a
  *which-source-for-which-claim* dimension. The trust graph generalises it; it does not reuse
  its semantics.
- **C2.** `acp:vc` is *type-only* and *unimplemented* in this repo. Claim-level reasoning over
  credential contents — the heart of this design — does not exist today.
- **C3 (the "blank slate" correction).** The design is **not** greenfield. sparq already ships
  *single-axis instances* of the trust primitive that this design unifies: the `acp:issuer`
  identity dimension; the `AccessProvenance` trusted-fact channel with its `solidx:`
  reserved-predicate anti-forgery guard (§2.4 of the AC design); the PROV-O lineage in
  `crates/sparq-prov`; and the `crates/sparq-zk` issuer-signed-commitment registry
  (`zk:issuerKey`, `zk:statusList`). The novelty is the *unification* of these into one
  general, queryable, per-(source, statement-type, constraint) trust layer — and the strings
  "trust graph" / "ZKaps" appear nowhere in the repo, so the *framing and vocabulary* are new
  even where a mechanism is partially present.

## 2. The trust-graph model

### 2.1 Definition

A **trust graph** is an RDF graph of **trust statements** plus the local **`.acl`/`.acr`
rules**, that together determine which externally-attested statements a resource (or server)
will *admit* into its access-control derivation, scoped per *source* and per *statement-type*.

Formally, the access decision is a two-stage reasoning pipeline:

```text
   attested statements (VC claims, signed graphs)
                 │
                 ▼
   ┌──────────────────────────────────┐
   │  ADMISSION stratum  (NEW)         │   gate: trust statement + verified issuer
   │  "is this fact from a source I    │          signature + Rust-side freshness
   │   trust for this statement-type?" │          + input-stratified revocation
   └──────────────────────────────────┘
                 │  admitted facts (issuer-tagged)
                 ▼
   ┌──────────────────────────────────┐
   │  DERIVATION stratum  (EXISTING)   │   the shipped sparq-solid WAC/ACP N3 rules
   │  ".acl rules over the admitted    │   + any ABAC rule (age>18 ⇒ canAccess)
   │   facts ⇒ canAccess"              │
   └──────────────────────────────────┘
                 │
                 ▼
        <urn:sparq:auth>  (allow-list, fail-closed)
```

The admission stratum is the genuinely new contribution; the derivation stratum is the
existing `sparq-solid` materialiser, which already turns a merged assertion graph into a
materialised `<urn:sparq:auth>` view.

### 2.2 Trusted-source-scoped trust (the integration primitive — NOT a new *trust-model* primitive)

The core relation is **per-(source, statement-type)**. How it sits relative to surveyed prior
art (stated precisely — the prior draft's absolute "strictly finer than EVERY surveyed prior
art / None expresses it" was **wrong**, and contradicted this design's own §9.1):

- Named Graphs / VC Data Integrity attest **per graph** (issuer signs a whole graph).
- Annotated-RDF / tSPARQL attach a trust value **per triple**, but *type-agnostic*.
- VC trust registries (DIF Trust Establishment, EUDI Trusted Lists) trust an issuer **per
  credential type**, as a flat list — no reasoning, no merge with local rules.
- ZCAP-LD/UCAN trust **per resource-capability**.
- **RT (Li/Mitchell/Winsborough, IEEE S&P 2002)** — the doc's own cited formal antecedent
  (§9.1, §9.4) — and **PERMIS** **already express issuer-scoped-per-attribute/role-type trust
  as Datalog**: RT credentials are typed/parameterised role definitions (`A.r ← B.r1`) with
  authority over each role localised to its issuer, i.e. *exactly* "trust issuer B for
  attribute-type `r`"; PERMIS gates `roleAssign :- attrCert, issuedBy, trustedIssuer, …`. So
  the **per-(source, statement-type) relation is NOT a new trust-MODEL primitive** — RT and
  PERMIS have it. (§9.1 already says this design *generalises* PERMIS's global `trustedIssuer`
  to per-type `trustsSourceFor` and treats RT as the antecedent; the earlier "None expresses
  it" in this subsection was a self-contradiction, now removed.)

**The honest novelty is a SYSTEMS / INTEGRATION + standards-fit contribution, not a
trust-model one:** (a) rendering RT/PERMIS-style typed-issuer trust as **RDF a *shipped* Solid
pod stratified-NAF reasoner merges with the local `.acl`/`.acr` rules**; (b) binding admission
to **VC Data Integrity / RDFC-1.0 signed-graph verification on the existing `sparq-zk`
estate**; and (c) recovering **WAC / ACP / Solid-OIDC as degenerate cases**. What RT and
PERMIS *lack* — and what is the real delta — is the RDF/merge-with-local-rules rendering, the
Solid binding, and signed-graph admission. See the novelty-honesty concession in §7 (item J).

The **degenerate case** recovers today's Solid exactly: a trust graph whose only statement is
"trust the `.acl`/`.acr` document for all access-control predicates" reproduces WAC/ACP
unchanged. This is the strict-additivity property (G6, §7): **a pod with no trust graph
behaves exactly as WAC/ACP do now.** Solid-OIDC is the *n = 1* case (one issuer, one
statement-type: the identity assertion).

### 2.3 The minimal orthogonal ontology

All IRIs below under `https://sparq.dev/ns/trust#` (prefix `trust:`) are **NON-STANDARD,
invented for this proposal**; a WG would rename/rehome them. They are placeholders to make
the semantics concrete, not a claim of standardisation.

This subsection presents a **deliberately minimal** vocabulary. Before pinning the terms we
ran a de-duplication pass that asks of every candidate term: *can this be expressed via a
more-general primitive already in the set?* If yes, it is **not** a primitive — it is
**syntactic sugar that desugars** to the primitive at load time, and is documented as such
(§2.3.3), not added to the normative core. The discipline matters because every extra
primitive is one more thing a WG must standardise, one more admission-rule shape the engine
must evaluate soundly, and one more surface an adversary can probe. The minimal set is **ten
terms**.

#### 2.3.1 The minimal term set (irreducible core)

| Term (proposed, non-standard) | Domain → Range | Meaning |
|---|---|---|
| `trust:TrustPolicy` | class | The policy container: a set of admission rules scoping a resource/server. |
| `trust:TrustRule` | class | One reified admission rule, grouping a source/type/scope/freshness condition. |
| `trust:trustsSourceFor` | `trust:Source` → shape | **The core relation:** admit statements of this type (shape) from this source. |
| `trust:source` | rule → `trust:Source` | The attesting source a trust rule names. |
| `trust:forShape` | rule → `sh:NodeShape` | Statement-type as a SHACL shape (the *one* statement-type primitive; uses shipped `sparq-shacl`). |
| `trust:Source` | class | An attesting authority, identified by an issuer key / DID. |
| `trust:issuerKey` | `trust:Source` → key/DID | Verification key the source signs with (aligns with `zk:issuerKey`). |
| `trust:scope` | rule → resource/container | Where the trust rule applies (server-wide vs per-`.acr`). |
| `trust:freshWithin` | rule → `xsd:duration` | Maximum staleness admitted (consulted against `Session.now`). |
| `trust:admitted` | (internal) | Marks a fact that passed admission; analogous to `solidx:` internal vocab. |

#### 2.3.2 Why no term is redundant (the irreducibility argument)

Each term occupies a **distinct, non-overlapping role**; none can be derived from a
combination of the others, so the set is irreducible. Stated per term:

1. **`trust:TrustPolicy`** — the policy *container*. There is no other way to group/scope a
   set of rules as one administrable unit; collapsing it would scatter rules with no binding
   entity.
2. **`trust:TrustRule`** — the reified rule entity that *collects* one admission condition
   (`source` + `forShape` + `scope` + `freshWithin`) under a single node. It could in
   principle be inlined as flat properties on the policy, but then a policy with two rules
   over the same source could not keep their type/scope/freshness conditions apart. The
   reification is what makes a rule the unit of conflict, override, and audit.
3. **`trust:trustsSourceFor`** — the **foundational primitive**: *"trust source S for
   statement-type T."* It is the only way to express per-(source, statement-type) scoping;
   every other term either *names a participant in* this relation or *constrains* it. It
   cannot be synthesised from any combination of the rest.
4. **`trust:source`** — connects a rule to a **named** `trust:Source` so one source can be
   referenced across many rules. Inlining the issuer key directly onto each rule (eliminating
   this term) would lose source identity-reuse and force key duplication.
5. **`trust:forShape`** — the **sole** statement-type primitive. A SHACL shape is the most
   general type constraint in the set: predicate-only, cardinality, value-range, and
   conjunctive constraints are all shapes. Nothing else expresses fine-grained statement
   typing, and (critically) `forShape` cannot be reduced *backward* to anything coarser
   without losing expressivity. This is why the predicate-IRI convenience desugars *into*
   `forShape` and not the reverse (§2.3.3).
6. **`trust:Source`** — the named attesting-authority entity. It must be a distinct node
   because it is simultaneously the **object** of `trust:source` and the **subject** of
   `trust:issuerKey`; eliminating it would force inlining keys into rules and lose the
   ability to refer to a source by name.
7. **`trust:issuerKey`** — the **cryptographic binding** *"source S's verification key is
   K."* It is the load-bearing gate for signature verification at admission (§3.3); there is
   no alternative mechanism to bind a source to its key.
8. **`trust:scope`** — the authorisation *boundary* (*"this rule applies to resource R /
   container C / server-wide"*). It is **orthogonal** to source and type — the same
   source/type pair may be trusted for resource A but not B — so it cannot be derived from
   them.
9. **`trust:freshWithin`** — the **temporal** gate (*"admit facts issued within duration
   D"*). It is orthogonal to source/type/scope and, per §3.3, must be a per-request Rust
   side-condition, **not** an in-reasoner predicate; it therefore cannot be synthesised from
   the other (reasoner-level) terms.
10. **`trust:admitted`** — the **stratum-boundary marker** that tags a fact which passed
    admission, enabling the stratified separation of admission from derivation (§2.1). Remove
    it and the two-stratum architecture collapses: admission gates become indistinguishable
    from derivation rules and the soundness argument of §3.3 has nothing to hang on.

**No further collapse is sound.** Two near-misses were considered and rejected:
`trust:TrustRule` could be inlined into `trust:TrustPolicy` as flat triples — rejected because
reification is what lets one policy hold multiple independently-overridable, independently-
auditable rules; and `trust:source` could be folded into `trust:trustsSourceFor` via
`rdf:subject`/`rdf:object` reification — rejected because it sacrifices named-source reuse for
no expressivity gain. Every remaining term purchases distinct expressivity or a distinct
soundness hook. The **only** purely-syntactic redundancy the de-dup found is
`trust:forPredicate` → `trust:forShape`, handled next as sugar.

#### 2.3.3 Conveniences that DESUGAR to primitives (sugar, not primitives)

The de-dup pass found exactly **one** term in the prior draft that is *not* a primitive but a
shorthand. It is kept for ergonomics, but **defined by its desugaring**, so a WG standardises
only the primitive:

- **`trust:forPredicate P` is SUGAR for a single-predicate `trust:forShape`.** A rule that
  trusts a source for *all* triples on predicate `P` (the cheap, decidable, v1-default case)
  is exactly the special case of a shape that targets the subjects of `P` and requires `P` to
  be present. The load-time desugaring is:

  ```n3
  # surface syntax (convenience):
  [] a trust:TrustRule ; trust:forPredicate schema:age .

  # desugars to (normative primitive) — a single-predicate SHACL node-shape:
  [] a trust:TrustRule ; trust:forShape [
       a sh:NodeShape ;
       sh:targetSubjectsOf schema:age ;            # target = subjects asserting the predicate
       sh:property [ sh:path schema:age ; sh:minCount 1 ]
     ] .
  ```

  The desugaring is **lossless in the predicate-only direction** (every `forPredicate P`
  assertion maps to exactly this shape) and runs the shipped, terminating `sparq-shacl`
  validator as the admission side-condition. The ergonomic reason to keep the sugar is that
  forcing every predicate-granularity rule to carry an inline or external shape document is
  boilerplate the common case does not need; the *semantic* reason it is **not** a primitive
  is that it adds no expressivity `forShape` lacks. v1 ships `forPredicate` as the default
  surface syntax and `forShape` as the expressive upgrade (the **statement-type granularity**
  question, §8), but the **normative vocabulary is `forShape` alone** — `forPredicate` is
  defined wholly by the rewrite above.

  (Note: `sh:targetSubjectsOf` + an `sh:minCount 1` property shape is the real, shipped SHACL
  idiom for "constrain to a single predicate"; there is no `sh:targetPredicate` term in SHACL,
  so the desugaring uses the genuine target/path mechanism `sparq-shacl` already evaluates.)

So the reader sees three things, by design: the **minimal vocabulary** (§2.3.1, ten
primitives), **why no primitive is redundant** (§2.3.2), and **the desugaring of the one
convenience** (§2.3.3). The skeptic's attack to mount is *"the de-dup is lossy"* — i.e. some
`forPredicate` rule does **not** round-trip through the shape rewrite, or some "primitive" is
secretly derivable; both are addressed above and listed as load-bearing claims in §7.6.

### 2.4 Relation to named-graph/quad trust and VC issuer trust

The *unit of attestation* is a Carroll-Bizer named graph made issuer-bound via VC Data
Integrity / RDFC-1.0 — which is **byte-aligned with sparq's existing crypto estate**:
`sparq-zk` already commits per-named-graph over RDFC-1.0-canonicalised leaves
(`crates/sparq-zk/src/{canon,commit,sig}.rs`), and `crates/sparq-canon` already wraps
RDFC-1.0 as a public API. So a trust graph's *input* needs no new canonicalisation
infrastructure: it consumes the same signed-graph unit the ZK estate already produces.

The trust statement itself is the **machine-readable, reasoning-driven form** of the VCDM 2.0
trust model — the spec already says "verifiers trust certain issuers for certain claims" but
*deliberately leaves that decision out-of-band and non-machine-readable*. The trust graph
reifies exactly that out-of-band decision as in-graph triples, per source **and** per
statement-type, then lets the reasoner derive access.

## 3. Evaluation and construction

### 3.1 Worked example: the age-over-18 case

**Inputs.**

```n3
# (a) The .acl rule (ABAC), authored by the resource controller — TRUSTED root:
{ ?x schema:age ?y . ?y math:greaterThan 18 } => { ?x auth:read <resourceX> } .

# (b) The trust statement, in the resource's trust policy — also controller-authored:
[] a trust:TrustRule ;
   trust:source     <https://gov.example/issuer> ;
   trust:issuerKey  <did:web:gov.example#key-1> ;
   trust:forPredicate schema:age ;
   trust:scope      <resourceX> ;
   trust:freshWithin "P30D"^^xsd:duration .

# (c) The presented credential graph G, signed by the gov issuer over its RDFC-1.0 commitment:
#   <Jesse> schema:age 25 .            (claim)
#   plus a VC Data Integrity proof binding G to did:web:gov.example#key-1
```

**Admission stratum.** A fact `<Jesse> schema:age 25` is admitted **iff** all hold:

1. there is a trust rule whose `trust:forPredicate` is `schema:age` and whose `trust:scope`
   covers `<resourceX>`;
2. the credential graph G carrying that triple is verified to be **signed by the key**
   `trust:issuerKey` names — a *checked* signature over G's RDFC-1.0 commitment, **not** a
   self-asserted "I am signed" triple (this is the load-bearing soundness condition, §3.3);
3. the credential is fresh (within `trust:freshWithin` of `Session.now`) and not revoked
   (status-list check);
4. the credential subject `<Jesse>` **binds to the authenticated requester** (the holder
   binding / subject-to-WebID join, §3.4).

Only then does `<Jesse> schema:age 25` enter the derivation stratum as a `trust:admitted`
fact tagged with its issuer.

**Derivation stratum.** The shipped sparq-solid materialiser runs rule (a) over the admitted
fact; `math:greaterThan` fires; `<Jesse> auth:read <resourceX>` lands in `<urn:sparq:auth>`.
A subsequent query is rewritten to that allow-list exactly as today.

### 3.2 Construction (how to build both graphs)

- The **trust graph is authored in the same trusted channel as `.acl`/`.acr`** — i.e. behind
  `acl:Control` (WAC) / ACR-write (ACP). Whoever may write the policy may write the trust
  rules; nothing else may. This is the NGAC self-administration discipline (administration is
  the same graph, governed by the same decision function) and it *avoids XACML's documented
  out-of-band-root gap* (NIST SP 800-178: trusted policies are "assumed valid, and their
  origin is established outside the delegation model").
- **Scope** is a v1 open question (§8): server-wide trust policy, per-`.acr` trust rules, or
  both. The design recommends *both*, with per-`.acr` rules narrowing (never broadening) a
  server default — monotone like ACP inheritance.

### 3.3 Soundness — the boundary this re-opens, and how it stays closed

This is the single most important correctness claim and the place an adversarial reviewer
should look first.

sparq-solid **deliberately closed** the content/reasoner boundary (§2.4 of the AC design):
pod *content* is never fed to the reasoner, because a writer who could inject `acl:`/`solidx:`
triples could self-grant. The trust graph **re-opens that boundary by policy** — it *admits*
external content into the derivation — which re-creates the escalation risk unless admission
is gated correctly. The design's safety argument:

1. **Admission requires a *verified* issuer signature, never a self-asserted one.** A graph
   claiming `trust:issuerKey ...` proves nothing; the signature over its RDFC-1.0 commitment
   must be *checked* against the key the trust rule names, exactly as the loader today checks
   `validate_principal_iri` and *rejects* forged `solidx:` predicates
   (`is_reserved_derivation_predicate`). An untrusted source's statement is **not usable**:
   no matching `trust:trustsSourceFor` ⇒ not admitted ⇒ invisible to the derivation
   (fail-closed, D4).
2. **Statement-type scoping is enforced at admission, not derivation.** A source trusted for
   `schema:age` cannot launder an `acl:agent` or `solidx:creator` triple in: those predicates
   are *not* in its `trust:forPredicate`/`trust:forShape` set, so the admission rule never
   fires for them. The reserved-derivation-predicate guard remains in force *under* this — a
   trust rule cannot grant a source the right to assert `solidx:` internal vocabulary.
3. **The trust graph itself is Control-gated.** Because trust rules live in the same channel
   as `.acl`, an attacker cannot insert a self-favouring trust rule without already holding
   write access to the policy — at which point they could edit `.acl` directly. No new escalation
   surface is opened *by the trust graph as data*; the new surface is purely the *admitted
   external facts*, which (1)+(2) gate.

**Monotonicity / non-monotonicity.** The shipped reasoner is **no-retraction, stratified
negation-as-failure** (sparq-reason; the AC design's stratification discipline). Several
consequences the design must honour — and one architectural gap the prior draft glossed:

- **ADMISSION-VS-MATERIALIZE-ONCE GAP — RESOLVED: decision (a), the static/dynamic split
  (`sq-xc4y`, shipped in `sparq-trust` + `sparq-solid` `trust_wire`).** The prior draft
  presented admission as a clean stratum *ahead of* derivation. But the shipped `sparq-solid`
  auth view is materialised **once, session-independently** (`PodStore`, `lib.rs`) and then
  queried **per-session** by principal expansion — whereas admission gates on
  `credentialSubject == Session.agent` (holder binding, §3.4) and `trust:freshWithin` vs
  `Session.now`, which are **per-request** facts. A per-request, identity-bound admission
  decision **cannot** simply sit ahead of derivation inside a session-independent
  materialise-once view: either admission re-runs per request (negating the P4 epoch-cache
  composition) **or** holder binding degrades to a query-time principal match. **The decision
  taken is (a):** SPLIT admission into a **STATIC** class (issuer signature, statement-type
  scope, reserved-predicate guard, `trust:scope`) decided **once at materialise-time**
  (`sparq_trust::admit_static`), and a **DYNAMIC** class (holder binding + freshness) **deferred
  to a per-request conditional grant**. The static decision emits an `auth:ConditionalGrant`
  whose `auth:agent` is the credential subject and whose `auth:notAfter` is
  `issued_at + freshWithin`; both are re-checked **per request** by the shipped sq-0q7n
  `AuthIndex::cond_applies` path (`auth:agent` against `Session.agent`, `auth:notAfter` against
  `Session.now`). So holder binding does NOT degrade to a static principal match — it is a live
  per-request check — and freshness lapses at query time **without** a re-materialise; the
  dynamic verdict is never frozen into the view, while the static stratum composes with the
  epoch cache. (This generalises the sq-0q7n precedent — which already does this for time
  windows — to the identity dimension. Option (b) per-request re-materialise was rejected: it
  negates the epoch cache and the existing conditional-grant machinery already gives the
  per-request semantics for free.) The load-bearing soundness test
  (`sparq-solid/tests/trust_graph.rs::static_admission_defers_holder_and_freshness_to_query_time`)
  drives this through the REAL `PodStore`: a stale `now` and a wrong-holder request are each
  denied at query time. **Honest residue:** revocation that occurs AFTER materialise is still a
  re-materialise event (epoch bump, G5/§8) — only holder + freshness are deferred, because they
  are pure functions of the per-request `Session`; revocation is an external-state change, not a
  request fact.
- Admission rules must be **stratified ahead of** derivation: all *static* admission decisions
  for a predicate complete before any derivation rule reads it, so scoped-NAF stays sound.
- **Freshness is not in the reasoner; revocation may only enter as an input-stratified
  guard; the §2.1 diagram is reworded accordingly.**
  Time is a **per-request Rust check** (`authindex.rs`), not an in-reasoner predicate, and the
  incremental counting profile permits negation-as-failure **only over input-only predicates**
  and **rejects NAF over *derived* predicates** (`incremental.rs`, `n3_compile`). The full
  evaluator can negate a derived predicate only through the explicit
  `reason_n3_stratified` contract: the predicate must reach its complete fixpoint in an
  earlier stratum. So a v1 `not-revoked` guard must be **input-stratified** (NAF over an
  *input-only* `revoked` predicate seeded before admission); treating a revocation predicate
  that admission rules can also derive as input-only is **unsound**. The §2.1 admission box is
  therefore a **Rust-side per-request freshness side-condition + an input-stratified
  revocation guard**, not same-stratum negation over derived facts (`sq-tu4e`).
- **Seeding-caveat citation, corrected.** The "two-unbound-atom seeding blow-up" war story
  belongs to the **incremental counting path** (`sparq-reason` incremental seeding), **not**
  the full evaluator. **solid uses the full evaluator through both `reason_n3` and
  `reason_n3_stratified`**; the latter already drives its three-stratum ACP materialisation,
  and the N3 module supports `math:greaterThan`. So the real, *unanalysed* termination risk
  for the paths that actually run is
  **recursive / unbound-join admission rules over external-graph extents in the full
  evaluator** — P8 must bound *that* path, not the incremental one (`sq-tu4e`).
- Revocation/expiry is *non-monotone* (a previously-admitted fact is withdrawn). Because the
  materialiser is full-re-run-on-change (epoch-bump-on-rematerialise, no incremental
  maintenance yet), revocation propagates by re-materialisation, *not* by retraction inside a
  single run. v1 must therefore re-evaluate on credential expiry/revocation and on trust-graph
  change; incremental admission maintenance is a named follow-up, not shipped (G5/§8).

**Entailment laundering (G3).** If a source is trusted for `schema:age` but the reasoner can
*derive* age from an untrusted `schema:birthDate`, does the derived age inherit trust?
**v1 admits only directly-attested facts of a trusted type** — no laundering of derived facts
through the trust boundary. Trust-annotation *propagation* through the closure (the
annotated-RDF/semiring extension to sparq-reason) is a research alternative, explicitly out
of v1 scope.

**Termination.** Admission rules are guarded conjunctions over signed-graph facts + trust
facts; with predicate-IRI typing and one-side-bound seeding they terminate. SHACL-shape typing
(`trust:forShape`) runs the shipped, terminating `sparq-shacl` validator as a side condition,
not as recursive rule expansion.

**Issuer-key distribution — a LIVE forgery vector, not a footnote.** A trust rule names
`trust:issuerKey` as a DID/verification-method IRI. **sparq has no DID resolver today** —
`did:example:...` appears only as opaque example IRIs in the zk registry. So in v1 the binding
`trust:issuerKey → verifying-key` is **unverified** — an attacker who controls what a DID IRI
*resolves to* (or who can substitute the key material) can present a credential "signed by"
the trusted issuer, i.e. **silent privilege escalation**. This is an **active attack surface**
that is **gated only by P2** (a pluggable `did:key`/`did:web` resolver feeding the existing
`sparq-zk` signature check), alongside the `sig.rs` issuer-disclosure caveat (§5.3). Until P2
ships, keys must be supplied **directly** (the `zk:issuerPublicKey` material) so the binding is
operator-asserted rather than resolver-verified — honest, but **not** an end-to-end trust path.
Tracked in `sq-tu4e`; P2 is `sq-pfae.3`.

### 3.4 The subject-to-requester join (holder binding)

The credential says `<Jesse> schema:age 25`; the requester authenticated as some WebID. The
admission rule must **bind the credential subject to the authenticated principal** — otherwise
anyone could present Jesse's age credential. This is a hard join and is its own correctness
obligation: v1 requires the credential's `credentialSubject` (or a holder-binding proof, e.g.
SD-JWT-VC `cnf`) to equal `Session.agent`. Presenting a third party's credential without
holder binding must **not** admit the fact.

### 3.5 Conflict resolution

Two trusted issuers may attest contradictory values (`age 25` vs `age 17`). The derivation
stratum inherits ACP's normative **deny-overrides** for the *access* decision, but the
*admission* of contradictory *facts* needs its own rule. v1 admits both issuer-tagged facts.

**Expressibility boundary (resolved).** Conservative **deny when a disagreement exists is
expressible today without NAF or an engine extension**: a positive rule joins two admitted,
issuer-tagged facts for the same subject and predicate, checks distinct values via
`log:notEqualTo` (exact term comparison, which can over-report value-equal but term-distinct
literals such as `"25"^^xsd:integer` and `"25.0"^^xsd:decimal`, in the fail-closed direction)
or `math:notEqualTo` when the attested value is known to be numeric, and, where the policy
requires it, checks distinct issuers. It then derives a positive conflict witness. A second
positive rule must join that witness against the policy structure and derive the prohibition
for each specific `(principal, target, mode)` grant that consumed the disputed attribute.
Deny-overrides is Rust-side post-materialisation set subtraction over those access tuples
(`authindex.rs`), not subtraction over attribute-level conflict facts. It therefore defeats
the routed grant derived from `age 25` when `age 17` also produces that conflict witness. This
rule is monotone: adding an attestation can add a conflict, never retract one.

A superficially similar encoding — **grant only if no derived conflict exists** — does require
NAF over a derived predicate. It is not accepted by the incremental counting profile
(`incremental.rs` rejects a guard predicate derived by any rule), but it is expressible by
the full evaluator's explicit `reason_n3_stratified` API when conflict reaches a fixpoint in
an earlier stratum and the later stratum negates it. It is **not sound in one `reason_n3`
fixpoint**, whose no-retraction semantics cannot withdraw a grant after a conflict appears.
The solid ACP materialiser already uses a three-stratum `reason_n3_stratified` pipeline
(`crates/sparq-solid/src/materialize.rs`), so such a conflict stratum could be added to that
existing path rather than requiring a new driver.
v1 therefore uses the positive-conflict + deny-overrides form; an engine extension is needed
only for dynamic/general conflict policies that cannot be compiled to positive witnesses and
the existing fixed strata (for example preference order or threshold/k-of-n policy
evaluation). Those general semantics remain designed-only (§7). [SONNET-4.6] `sq-tu4e`.

## 4. Capability delegation for human AND AI agents

> **Status of this section (read first — it is the most aspirational in the doc).** §4 is
> **DESIGN-ONLY**. There is **ZERO delegation substrate in any crate today** (verified by
> grep across `crates/sparq-solid` and `crates/sparq-server`: no `dpop` / `gnap` /
> `key-proof` / `proof_of_possession` / `zcap` / `ucan` / `actedOnBehalfOf`). The shipped
> `Session.client` is a **bare IRI** (WAC `acl:origin` / ACP `acp:client`) with **no
> proof-of-possession**, and `crates/sparq-prov` records `prov:wasAssociatedWith` a **single
> agent** per reasoner-derivation activity — it has **no `prov:actedOnBehalfOf` /
> delegation-chain modelling**. So where this section names "key-proofing", "PROV-O records
> the chain", or "build on existing seams", read it as **proposed**, not shipped. The whole
> AI-agent delegation story is a **research hypothesis under live caveats** — the same posture
> §5.3 takes for the ZK/privacy half — not a near-term, substrate-backed deliverable. The two
> open security problems this section leaves are tracked as `sq-l5og` (invocation binding) and
> are enumerated in §7.4 (items K, K′, M, N, O).

### 4.1 Two trust roots, one derivation

The surveyed prior art has **two unreconciled trust roots**: VC/issuer-rooted *attribute*
trust (which the §3 admission stratum handles) and ZCAP/UCAN *resource*-rooted *capability*
trust (delegation chains). A delegation is *also* a trusted-source-attested statement, so it
rides the same admission rail: a ZCAP-LD/UCAN delegation document is a signed graph; admitting
it means *verifying the chain to a root the trust graph anchors* and treating its caveats as
trust-graph conditions.

**The invocation-binding hole (the core soundness gap of §4).** UCAN/ZCAP deliberately
separate **delegation** (the signed chain document — *who may act*) from **invocation**
(exercising it on *this* request, key-bound to the caller). The "rides the admission rail"
model above captures the chain's **existence** but **not** the per-request *invoker binding*.
§3.4 gives a holder-binding rule for *attribute* credentials (`credentialSubject ==
Session.agent`); this section, in v1, **does not yet state the analogous rule for
delegations** — namely **authenticated invoker == the chain's terminal delegate, key-proven
per request**. Without that gate, an admitted delegation becomes a **standing graph fact any
session reaching that graph could ride** — the exact replay / re-delegation vector — which is
a privilege-escalation of the **same severity class as the §2.4 boundary re-opening** and
needs the same kind of adversarial forgery test (a delegation-replay analogue of
`acp_forged_*_in_acr_document_does_not_grant`). This was an **open problem**, `sq-l5og`.

> **[OPUS-4.8] sq-l5og — RESOLVED in the PoC** (`crates/sparq-trust/src/delegation.rs`,
> tests `crates/sparq-trust/tests/delegation_replay.rs`). The missing rule is now **specified,
> enforced, and tested**: `invoke()` gates the carried chain on (1) trust-anchored root,
> (2) a CHECKED delegator signature per hop over a domain-separated `hop_message` that binds the
> hop's delegator/delegate **keys** alongside the capability + expiry, (3) monotone attenuation
> (`child ⊆ parent` actions + expiry), (4) terminal-hop expiry vs `now`, (5) scope, and (6) the
> **invocation binding** — *authenticated invoker == terminal delegate* **AND** a per-request
> fresh-challenge proof-of-possession under the terminal key (DPoP/GNAP-style, modelled on the
> shipped `sparq-zk` `holder_pop_message` / `sign_holder_pop`).
>
> **Soundness fix (adversarial review, [OPUS-4.8]).** An earlier revision excluded `delegate_key`
> from `hop_message`, so the terminal hop's `delegate_key` was attested by NO signature: an
> attacker could capture the chain, substitute its OWN key as the terminal `delegate_key` (the
> genuine WebID + genuine delegator signature still verified, and the PoP verified under the
> attacker key), and ride the chain — a confirmed key-substitution replay BYPASS. Folding each
> hop's `delegate_key` (and `delegator_key`) into the signed preimage closes it: a substituted
> terminal key breaks the delegator's signature ⇒ rejected at step 2. The matrix now includes the
> key-substitution negative test (`stolen_chain_with_substituted_terminal_key_is_denied`, plus
> the single-hop variant). The delegation-replay forgery matrix (third-party replay,
> stolen-chain-without-key, **key-substitution**, replayed-PoP-over-old-challenge, forged/lifted
> hop signature, escalating attenuation, broken link, expired, out-of-scope) all DENY.
>
> **Honest scope — do NOT overclaim full non-replayability.** With the key bound, a chain captured
> off the wire cannot have its terminal key swapped, so it no longer yields a usable PoP under an
> attacker key. But the property is only as sound as the trust in the keys themselves: the delegate
> key is attested by the delegator's signature, and the delegator's OWN key is still
> operator-/chain-asserted — there is no DID resolver binding a WebID to a key yet (`sq-pfae.3`,
> the live forgery vector D′). So the gate defeats stolen-chain **key-substitution** replay; it does
> NOT close the upstream key-trust gap. Still open and documented (not solved): deep-chain
> *incremental* revocation (full re-materialisation only — §4.4 stale-window bounded, not closed)
> and DID-resolver delegate-key binding (`sq-pfae.3`). This is a research PoC, **not** a shipped
> security guarantee.

**Ambient-authority tension — RESOLVED in the PoC ([OPUS-4.8] item M).** This §4.1 "admitted as
a graph fact" storage model **is** an ambient lookup, which is precisely what §4.3(a)'s
object-capability discipline ("carry the chain *with* the invocation, do not look it up
ambiently") warns against. The PoC takes the **obj-cap side explicitly**: `invoke()` gates on a
chain **carried with the invocation** plus the live PoP, never an ambient graph lookup; storing
a delegation as a graph fact (the §4.1 model) is demoted to an OPTIONAL audit record, never the
authority source. This is the design choice the PoC makes for `sq-l5og` (flagged to the
maintainer to steer — see the gh issue opened on PR-open).

### 4.2 Attenuation and on-behalf-of

- **Attenuation (monotone narrowing).** Each delegation hop may only *restrict* (ZCAP-LD: a
  child capability's `allowedAction ⊆ parent`, `expires ≤ parent`, target only
  suffix-narrowed; UCAN: each hop restates or attenuates; SPKI tag-intersection). The trust
  graph models a delegated capability as RDF whose caveats are *conditions in the admission
  rule* — and the engine must *enforce* monotonicity, never derive a broader grant than the
  chain permits.
- **On-behalf-of / AI agents.** An AI agent is a distinct principal (its own DID/key) holding
  an **attenuated child** of its human principal's authority. The proposed correctness
  invariant is the **intersection rule**: an agent's effective permissions =
  `delegator-permissions ∩ agent-allowed-scope`, enforced per request. "Human vs AI" is just
  an attested attribute / caveat on the delegate. This maps to OAuth Token Exchange (RFC 8693)
  on-behalf-of and the Stanford "Authenticated Delegation and Authorized AI Agents" model
  (arXiv 2501.09674); the trust-graph value-add is rendering the delegation as RDF that
  reasons with the *same* `.acl` rules. **Two honest qualifications:** (i) the intersection
  rule is an **UNENFORCED design assertion** in v1 — no engine mechanism is shown that
  *forces* the derived agent grant ⊆ delegator grant; "enforce monotonicity" is a
  *requirement*, not an implemented guarantee. (ii) The intersection must bind to the
  **CURRENT** delegator grant — re-derived on **every** re-materialisation — **never** the
  grant snapshotted at delegation time; because `delegator-permissions` is itself a *derived,
  time-varying* view, computing the intersection against a stale snapshot lets an AI agent
  **retain escalated authority** after the delegator's grant is revoked. The doc does not yet
  pin *which* snapshot the intersection binds (§7 item N).

### 4.3 Confused-deputy resistance

Object-capability discipline (designation = authority carried *with* the request, no ambient
authority) is what resists the confused-deputy problem. The trust graph must therefore (a)
carry the delegation chain *with the invocation* (not look it up ambiently), (b) bind the
invocation to the holder's key (GNAP-style key-proofing / DPoP), and (c) keep the agent's
authority provably `⊆` the delegator's via the intersection rule.

**Reclassify (a)–(c) and the audit claim from implied-shipped to PROPOSED.** None of this
composes with an existing seam: **there is no DPoP / GNAP / key-proof / proof-of-possession
code in any crate** (verified by grep), and `Session.client` carries no key binding — so the
key-proofing in (b), which is the *named* confused-deputy defence, is **entirely unbuilt**.
The "PROV-O lineage (`sparq-prov`) records the chain for audit" claim is likewise **aspirational**:
`sparq-prov` records `prov:wasAssociatedWith` a **single** agent per reasoner-derivation
activity (`crates/sparq-prov/src/reason.rs`) and has **no `prov:actedOnBehalfOf` /
delegation-chain modelling** — it captures *reasoner-derivation* provenance, not an
*authority-delegation* chain. **v1 therefore does NOT prevent confused-deputy in the obj-cap
sense** — the per-request key binding is a **prerequisite, not a property** (P5 non-goal, §7
item O).

### 4.4 Revocation under delegation

Revoking a delegated capability is harder than revoking a credential (the chain may be deep).
v1 consults a status list per link and re-materialises on change; deep-chain incremental
revocation is a named follow-up. **Two honest qualifications.** (i) The cited
`crates/sparq-zk-compose/src/revocation.rs` is a **Merkle / status-list ZK primitive** — it
has **no chain-revocation** modelling; it is a building block for the *per-link* status check,
**not** a delegation-chain revocation mechanism. (ii) Because retraction is **full
re-materialisation** (epoch bump, no incremental maintenance yet), a revoked mid-chain link
**stays live until the next epoch** — an **unbounded stale-authority window** sized only by
the re-materialisation cadence, which matters most for **long-lived (AI-agent) sessions**. The
doc does not size this window (§7 item N).

## 5. The superset-of-ZKaps argument (honest and precise)

The brief asks to show the model expresses a **superset of ZKaps' features**. The honest,
defensible version splits the claim into two layers — one provable from the authorisation
model, one that requires composition with the (un-audited) ZK estate.

### 5.1 Two distinct things called "ZKaps" (and two *more* distinct artifacts inside the first)

- **ZKAPs (Least Authority product):** single-use, *unlinkable*, *anonymous* authorisation
  tokens (Least Authority, *Zero-Knowledge Access Passes* whitepaper, July 2021). A ZKAP
  attests **anonymous proof-of-payment** ("this redemption corresponds to one paid token"),
  with strong unlinkability (issuance↔redemption and redemption↔redemption mutually
  unlinkable). It carries **no attributes, no predicates, and is deliberately non-delegable.**
  Crucially, **disentangle two things that are *not* the same artifact** (a defect the prior
  draft conflated): Least Authority's ZKAPs are built on the **original, pre-RFC Privacy Pass
  VOPRF/DLEQ construction**; they do **not** implement the **IETF Privacy Pass RFCs 9576 /
  9577 / 9578 (June 2024)**, which standardise a *different* issuance protocol targeting
  proof-of-humanness/attestation. So "ZKAPs" (Least Authority, proof-of-payment, original
  Privacy Pass crypto) and "IETF Privacy Pass" (RFC 9576-9578, attestation) are distinct and
  must not be cited as one. The prior draft's gloss "attests one bit ('the holder passed
  attestation')" mischaracterised ZKAPs specifically — their bit is *anonymous
  proof-of-payment*, not proof-of-attestation.
- **"ZKaps" as the maintainer's term** for a *capability-VC* / ZK-attested access grant
  (predicate-proof over a hidden attribute, e.g. "prove age ≥ 18 without revealing 25").

The argument must be precise about which is meant, because they pull in opposite directions:
ZKAPs are *more private* and *less expressive*; capability-VCs are *more expressive* and
*conditionally private*.

### 5.2 What the trust graph subsumes (authorisation-model layer — defensible)

On the **policy/derivation axis**, the trust graph is more expressive **than a one-bit
token / fixed-predicate capability check** — the relativisation matters, see the caveat at
the end of this subsection:

- A ZKAP-gated or capability-token-gated access is the special case where the trust rule is
  "admit a one-bit attestation from issuer S and grant" — an N3 rule is a strict superset of a
  fixed-predicate token check (an arbitrary `{…}=>{…}` rule subsumes Biscuit-Datalog, which
  subsumes a single predicate).
- The trust graph *also* admits arbitrary attribute statements that **merge with `.acl`
  rules** (the age>18 derivation), which a pure capability/one-bit token **cannot** express.
- Delegation (ZCAP/UCAN attenuated chains) is *also* subsumed, as a species of
  trusted-source-attested statement (§4) — **but only at the chain-existence level; the
  per-request invocation binding that makes a delegation *non-replayable* is unspecified in
  v1, see §4 and §7(K).**

So **at the authorisation-model level, the trust graph expresses a superset of the *features*
of ZKAPs and of capability-VCs** — this is the claim that is defensible from the model.

**Scope the expressivity claim honestly (not against the whole field).** "More expressive"
here is relative to ZKAPs / capability-VCs / a fixed-predicate token check, **not** against
the trust-management literature. In particular the v1 *predicate-IRI* admission rule is **not**
more expressive than RT's full Datalog: RT (Li/Mitchell/Winsborough 2002 — the doc's own
cited antecedent) has linked, manifold, and threshold roles (`RT_1`, `RT_2`, `RT^T`, `RT^D`)
that the v1 admission rule does not yet match. The expressivity delta is over the *token /
one-bit* baseline, exactly as §5.1 scopes it — see the novelty-honesty concession in
§7 (item J).

### 5.3 What it does NOT get for free (privacy/unlinkability layer — caveated)

A **plain VC-presentation** model does **not** preserve ZKAPs' *unlinkability/anonymity*. When
a credential is admitted in the clear, the verifier learns the exact value (`age 25`, not just
"≥ 18") and can correlate presentations. To **match** ZKAPs' privacy property, the trust-graph
*derivation* must be discharged by a zero-knowledge proof — "prove `canAccess` was derived
without revealing the underlying facts" — composed with sparq's ZK estate
(`sparq-zk`/`sparq-zk-compose`). That estate is **research-stage and NOT externally audited**:
the v1 verifier is remediated and internally re-audited but external accredited-cryptographer
sign-off is **pending** (`sq-qhy4`). `sparq-mpc` is honest-majority semi-honest only.

**Hiding the issuer is necessary but NOT sufficient for ZKAPs-grade unlinkability.** ZKAPs /
Privacy Pass give *presentation* unlinkability: redemptions are mutually unlinkable **and**
unlinkable to issuance, enforced by single-use / rate-limit. Matching that needs **three**
pieces, and the honest status of each (verified in code, correcting the prior draft's
single-axis "the hidden-issuer upgrade is not yet reached", which was **factually wrong**):

1. **Hidden-issuer set-membership — BUILT, but NOT-yet-sound / externally unaudited.** The
   *in-the-clear* issuer-key check in `crates/sparq-zk/src/sig.rs` (lines ~36-39) **discloses
   which issuer signed** each graph (it checks `pk_i` in the clear against the *disclosed*
   key-set `K`), and its own comment says "the in-circuit undisclosed-key upgrade removes that
   leak." That upgrade **is implemented** (bead `sq-z9l`): the host-side helpers
   `key_set_root` / `key_membership_witness` / `hidden_issuer_prover_toml` ship in
   `crates/sparq-zk-compose/src/issuer.rs`, the in-circuit relation is
   `zk/compose/compose_core/src/issuer.nr`, and a compiled member exists at
   `zk/compose/hidden_issuer_d4`. So the correct status is **built-but-not-yet-sound**
   (`sparq-zk-compose/src/lib.rs` flags it NOT-yet-sound, gated on `sq-qhy4`) — *not*
   "not reached". The v1 clear-issuer check in `sig.rs` is the **in-the-clear interim**; the
   in-circuit undisclosed-key path is its **sound-once-audited successor**.
2. **Hidden-holder zero-knowledge proof-of-possession — BUILT + WIRED, but NOT-yet-sound.**
   The holder PoK member (`holder.nr`, bead `sq-xqfg`) proves possession of a holder key
   without disclosing it, and the verifier binding gate `bind_holder_pok` (T6, bead `sq-i1dt`)
   **is implemented** (`crates/sparq-zk-compose/src/verifier.rs` ~line 3158, tested in
   `tests/holder_pok_binding.rs`; `sq-i1dt` is CLOSED "already implemented on main"). But it
   is **NOT-yet-sound** (`sq-qhy4`): a passing PoK is *not*, under an adversarial prover, a
   guarantee — so it does **not** soundly anonymise the requester today. (A reviewer who said
   `bind_holder_pok` is "not implemented" was mistaken on the *build* status; the real
   limitation is *soundness/audit*, not absence.)
3. **A single-use nullifier / rate-limiting / double-spend primitive — ABSENT.** A grep across
   `sparq-zk` and `sparq-zk-compose` finds **no** nullifier, rate-limit, or double-spend
   primitive. Without one there is no enforcement that a presentation is single-use, which is
   load-bearing for ZKAPs' anti-replay/anti-Sybil property.

**The deeper tension the prior draft missed:** even with a hidden issuer *and* a selective
disclosure predicate proof, **§3.4's holder binding (`credentialSubject == Session.agent`)
authenticates the requester's WebID in the clear** at the derivation stratum — so the verifier
still learns *which authenticated WebID* is requesting. Presentations are therefore **trivially
linkable by requester identity**, and the access is **not anonymous**. §3.4 holder binding is
in **direct tension** with ZKAPs anonymity, and a ZKAPs-equivalent presentation must **replace
clear-WebID holder binding** with an **in-ZK holder-PoP plus nullifier** (bind to a key, prove
membership/possession in zero knowledge, enforce single-use). This composite is
**designed / partially-prototyped, NOT shipped or sound** — tracked as open problem `sq-wvne`.
Because the blocker is an **architectural property of the request path** (a clear WebID), and
not merely a missing ZK gadget, the *presentation*-unlinkability rows of the §5.4 coverage
tables are classified `gap` (architecture change required), **not** `zk-comp`: composing the
existing ZK estate does **not** close them.

**Therefore the honest framing for the WGs is:**

> The trust graph expresses a superset of ZKAPs' features **at the authorisation/derivation
> layer** (attribute-conditioned grants, arbitrary-predicate rules, delegation — none of which
> a one-bit token expresses). It does **not** provide ZKAPs' unlinkable/anonymous *presentation*
> property. Matching that needs a **three-part ZK composite — hidden issuer + ZK holder-PoP +
> nullifier** — of which only the first two are **built but not-yet-sound / externally
> unaudited**, and the third (nullifier / single-use) is **absent**; and §3.4 clear-WebID
> holder binding must be replaced by an in-ZK holder-PoP. The privacy half is a research
> hypothesis under live caveats, **not** a settled result, and is hard-gated on external
> sign-off `sq-qhy4`.

This is a *strictly weaker and true* claim than "superset of ZKaps", and is the one to take to
the working groups. See §7 for the adversarial-review limitations.

### 5.4 Expressive completeness vs ZKAPs and IETF Privacy Pass

This subsection makes the coverage claim **checkable**: it maps **every** ZKAPs (Least
Authority 2021) concept and **every** IETF Privacy Pass concept (RFC 9576 architecture / 9577
HTTP authentication scheme / 9578 issuance protocols) to the trust-graph ontology term or
structure that expresses it. The verdict per row is one of three, and the column is the
**whole honesty of the argument**:

- **ontology** — expressed *by the ontology / authorisation model itself*: the
  structural/relationship concept maps to a term or admitted-fact structure, no ZK required.
- **zk-comp** — expressible **only by composing the (unaudited) ZK estate**
  (`sparq-zk`/`sparq-zk-compose`), captured by the ontology as a **named obligation** (the
  three-part composite of §5.3), **not delivered by the ontology**. Hard-gated on external
  sign-off `sq-qhy4`. A `zk-comp` row is *honest about being a promissory note.*
- **gap** — **not** expressible today by either the ontology or the (built) ZK estate; the
  minimal term(s)/primitive to close it are named in the row.

#### 5.4.1 ZKAPs (Least Authority, 2021) — concept coverage

| ZKAPs concept | Trust-graph term / structure | Verdict |
|---|---|---|
| Issuer role (mints tokens) | `trust:Source` + `trust:issuerKey` (issuer-signing via `zk:issuerKey`) | ontology |
| Redeemer / verifier role | admission-stratum verifier checks issuer sig over RDFC-1.0 commitment (§3.1, §3.3) | ontology |
| Client / holder role | `Session.agent` + `credentialSubject` holder binding (§3.4) | ontology |
| Token issuance (as authorisation grant) | `trust:trustsSourceFor` + `trust:source` + VC Data Integrity / RDFC-1.0 signed-graph binding (§2, §3.1) | ontology |
| Redemption (presentation verification) | `trust:admitted` facts after admission sig-verification (§3.3) | ontology |
| Payment/grant binding (token tied to issuance event) | VC bound to issuer + `trust:freshWithin` (proof-of-payment captured as a VC attribute claim) | ontology |
| Attestation / proof-of-property binding | `credentialSubject` claim admitted per `trust:forShape`/sugar (§2.3, §3.1) | ontology |
| Authorisation decision / access control | admission → derivation strata; admitted facts merge with `.acl` via N3 (§2.1–2.2, §3.1) | ontology |
| Non-delegability (deliberately non-transferable) | holder binding `credentialSubject == Session.agent` (§3.4) enforces single-subject use | ontology |
| Issuer role (authority minting) | `trust:Source` + `trust:issuerKey` (§2.3) | ontology |
| Issuer-client unlinkability (issuance↔redemption) | hidden-issuer set-membership (in-the-clear issuer check current `sig.rs`; undisclosed-key variant `sq-z9l` **built but not-yet-sound**, `sq-qhy4`) — §5.3(1) | zk-comp |
| Origin-client unlinkability (presentation privacy) | **NOT achievable under the current design**: §3.4 authenticates the WebID in the clear, so presentations stay linkable by requester identity even with hidden-holder ZK PoP + nullifier. Requires the **architecture change** of §5.3 (replace §3.4 clear-WebID holder binding with an in-ZK holder-PoP); nullifier also **absent** — §5.3(2) | **gap** (architecture change required + absent primitive — `sq-wvne`) |
| Attester-origin / cross-site unlinkability | 3-part composite over the §3.4 architecture change (hidden-issuer + in-ZK holder-PoP + nullifier); blocked by the same clear-WebID binding (§5.3) | **gap** (same §3.4 architecture change + absent nullifier — `sq-wvne`) |
| One-more-forgery security | in-circuit issuer-sig gadget (`sq-z9l`) not-yet-sound; audit `sq-qhy4` pending | zk-comp |
| Concurrent / multi-session security | ZK estate multi-session security (in `sq-qhy4` audit scope) | zk-comp |
| Redemption-context unlinkability (same holder, many redemptions) | nullifier + rate-limit **without** deanonymisation | **gap** (no nullifier/rate-limit primitive — §5.3(3)) |
| Double-spend prevention / single-use enforcement | nullifier / single-use marker | **gap** (ABSENT in `sparq-zk`/`sparq-zk-compose`) |
| Rate-limiting / bounded-use-per-epoch without deanon | ARC / rate-limited-token primitive | **gap** (Privacy-Pass ARC designed but absent) |
| VOPRF / DLEQ blind-signature construction | (crypto protocol primitive) | **gap** (composition dependency, not an ontology concept) |
| Unconditional input secrecy / blindness | (crypto protocol primitive: blind issuance) | **gap** (VOPRF/DLEQ-blind, external to the ontology) |

#### 5.4.2 IETF Privacy Pass (RFC 9576 / 9577 / 9578) — concept coverage

The four-party Privacy Pass trust architecture maps **precisely** onto the trust-graph roles:
**Client ↔ `credentialSubject` (`Session.agent`)**, **Issuer ↔ `trust:Source` + `trust:issuerKey`**,
**Attester ↔ `trust:Source`**, **Origin ↔ `trust:scope`**, **Property ↔ `trust:forShape`/sugar**.
Both Privacy Pass and the trust graph are instances of the *same* foundational
per-(source, type) trust pattern (RT/PERMIS; §2.2). Concept-by-concept:

| Privacy Pass concept (RFC) | Trust-graph term / structure | Verdict |
|---|---|---|
| Client (9576 §3) | `credentialSubject` / `Session.agent`; authenticated requester | ontology |
| Origin (9576 §3) | `trust:scope` resource; admission gate applies at the resource | ontology |
| Attester (9576 §3) | `trust:Source` (entity generating attestation) | ontology |
| Issuer (9576 §3) | `trust:Source` + `trust:issuerKey` | ontology |
| Attestation property (age, humanness, …) (9576 §5) | `trust:forShape` (or `forPredicate` sugar) statement-type scoping | ontology |
| Token = cryptographic proof of attestation (9576 §4.2, 9578 §3) | admitted fact after sig-verification; credential graph signed by `trust:source` | ontology |
| Origin trusts Issuer for property-type (9576 §4.1) | `trust:trustsSourceFor` (per-source, per-statement-type) | ontology |
| Issuer trusts Attester for attestation (9576 §4.1) | `trust:trustsSourceFor` at the issuer layer | ontology |
| Non-colluding parties cannot share info (9576 §4.1) | source-scoped admission isolates different sources' facts | ontology |
| TokenChallenge components (9577 §2) | admission-gate preconditions (token_type/issuer_name/redemption_context/origin_info → rule conditions) | ontology |
| challenge_digest binding (replay prevention) (9577 §2) | digest verified inside the admission signature check | ontology |
| redemption_context (origin-specific binding) (9577 §2) | `trust:scope` (resource-specific constraint) | ontology |
| Context-bound tokens prevent cross-origin replay (9577 §2) | challenge_digest + redemption_context (`trust:scope`) verified at admission | ontology |
| VOPRF tokens — Type 0x0001, private-key verify (9578 §4) | authenticator admitted after sig-verification (ontology admits *output*, not mechanism) | ontology |
| Blind-RSA tokens — Type 0x0002, public-key verify (9578 §5) | authenticator admitted after public-key verification | ontology |
| Key identification token_key_id (9578 §6.1) | `trust:issuerKey` (resolved during admission sig check) | ontology |
| Key rotation / not-before (9578 §6.2) | `trust:issuerKey` with temporal metadata; `trust:freshWithin` | ontology |
| Deployment models (Shared/Joint/Split, 9576 §5) | different `trust:trustsSourceFor` configurations | ontology |
| Holder binding (credentialSubject↔presenter, 9576 §3) | `credentialSubject == Session.agent`, verified at admission (§3.4) | ontology |
| Freshness check (9576 §4.2) | `trust:freshWithin`; per-request Rust check (`authindex.rs`) | ontology |
| Revocation check (9576 §4.2) | W3C Bitstring Status List; input-stratified / per-request gate (§3.3) | ontology |
| Issuer-scoped vs distributed verification (9578 §4/§5) | trust rule encodes issuer-only vs public-key trust | ontology |
| Per-(Source, Statement-Type, Constraint) scoping (9576 §3–4) | `trust:trustsSourceFor` + `trust:freshWithin` | ontology |
| Issuance unlinkability (issuer↔redemption, 9576 §4.1, 9578 §3) | hidden-issuer set-membership (`sq-z9l`, built/not-yet-sound) | zk-comp |
| Presentation unlinkability (origins can't re-identify client, 9576 §4.1) | **NOT achievable under the current design**: even with hidden-issuer + ZK holder-PoP + nullifier, §3.4 authenticates the requester's WebID *in the clear*, so presentations stay trivially linkable by requester identity. Requires an **architecture change** — replace §3.4 clear-WebID holder binding with an **in-ZK holder-PoP**; the nullifier primitive is also **absent** | **gap** (architecture change required + absent primitive — `sq-wvne`) |
| Redemption-context unlinkability (9576 §4.1) | single-use + ZK composite (presupposes presentation unlinkability above — same §3.4 blocker) | **gap** (nullifier absent + §3.4 architecture change — `sq-wvne`) |
| Cross-origin unlinkability (9576 §4.1) | presentation anonymity (ZK composite); plain trust graph leaks WebID via §3.4 clear binding | **gap** (same §3.4 architecture change — `sq-wvne`) |
| Attestation reveals no identifying info (9576 §5) | ZK composite; plain credential disclosure is linkable and §3.4 discloses the WebID | **gap** (same §3.4 architecture change — `sq-wvne`) |
| Single-use / non-replayable token (9576 §5.1, 9578 §3) | nullifier / single-use marker | **gap** (no nullifier primitive — `sq-wvne`) |
| Nullifier / double-spend prevention (9576 §5.1) | nullifier primitive | **gap** (absent, verified by grep) |
| Issuer directory discovery (`.well-known/private-token-issuer-directory`, 9578 §7) | dynamic issuer-registry lookup | **gap** (static `trust:issuerKey`; needs P2 resolver `sq-pfae.3`) |
| Issuance-endpoint discovery (9578 §7) | issuer-registry model | **gap** (no issuer registry; P2) |
| Greasing / reserved token types (9577 §2) | statement-type versioning | **gap** (minor, design-only; not security-critical) |

#### 5.4.3 Verdict, and the minimal terms to close the gaps

**The honest thesis (scoped to one layer only):** the trust-graph ontology **completely covers
the structural / relationship concepts** of both ZKAPs and IETF Privacy Pass — every *role*,
every *trust relation*, every *attestation/property/token/challenge/scope/freshness* concept
maps to a term or admitted-fact structure (the `ontology` rows, which are the large majority of
both tables). That is the **only** completeness this document claims. The
**unlinkable-anonymous *presentation* property** is **not delivered by the ontology** and is
**not** completable under the current design: it is captured **as a named obligation** that is
partly a ZK-composition note (the `zk-comp` rows of §5.4.1 — the one-more-forgery /
multi-session / issuer-side hidden-issuer pieces, gated on `sq-qhy4`) and partly a **hard `gap`
requiring an architecture change** (see below). This is exactly the "superset of policy
expressivity, *composes with* — not supersedes — unlinkability" framing of §5.3, now made
row-by-row checkable.

**Coverage is therefore NOT unconditionally complete — there are genuine `gap` rows, and they
are stated plainly rather than papered over.** They cluster into **five** items, none of which
is a *trust-relation* concept (the ontology's core), all of which are *presentation-mechanics*
the ontology was never meant to be:

1. **Single-use / nullifier / double-spend** — the load-bearing anti-replay primitive ZKAPs and
   Privacy Pass both require, **absent** from the ZK estate. *Minimal term to close:* a
   reserved internal marker `trust:nullifier` (a per-presentation unique tag the verifier
   records and refuses to admit twice) plus the in-circuit nullifier-derivation gadget the ZK
   estate must add. This is an **ontology + crypto** addition, not pure ontology. Tracked
   `sq-wvne`.
2. **Rate-limiting / bounded-use-per-epoch without deanonymisation** — the Privacy-Pass ARC
   family. *Minimal term:* `trust:rateLimit` (max uses per epoch) on a `trust:TrustRule`,
   enforced via the same nullifier ledger. Crypto primitive absent.
3. **Issuer directory discovery** — dynamic resolution of `trust:issuerKey`. *No new term* — it
   is closed by the **P2 DID/issuer resolver** (`sq-pfae.3`) feeding the existing signature
   check; until then the binding is operator-asserted (§3.3, the live forgery vector D′).
4. **Greasing / statement-type versioning** — a minor, non-security-critical gap. *Minimal
   term:* a `trust:reservedType` registration on the policy; design-only.
5. **Presentation / origin-client / cross-origin unlinkability — an ARCHITECTURE-CHANGE gap, not
   a ZK-composition gap.** This is the item the prior draft mis-marked `zk-comp`. Even with the
   full three-part ZK composite (hidden issuer + ZK holder-PoP + nullifier — all unaudited),
   §3.4's holder binding authenticates the requester's **WebID in the clear** at the derivation
   stratum, so presentations remain **trivially linkable by requester identity**. Closing it
   requires **replacing §3.4 clear-WebID holder binding with an in-ZK holder-PoP + nullifier**
   (the nullifier of item 1 is itself absent) — a *design change to the admission/holder-binding
   architecture*, not merely "compose the existing ZK estate". Tracked `sq-wvne`. The §5.4.1
   `Origin-client`/`Attester-origin` and §5.4.2 `Presentation`/`Redemption-context`/
   `Cross-origin`/`Attestation-reveals-no-identifying-info` rows are now marked `gap` for this
   reason (they were `zk-comp` in the prior draft, which overstated deliverability).

> **Footnote on the privacy layer (do not skip).** The `zk-comp` rows assume **both** an
> external audit (`sq-qhy4`) **and** — for any *presentation*-unlinkability row — the
> architectural replacement of §3.4 holder binding (item 5 above), neither of which is shipped.
> The hidden-issuer variant (`sq-z9l`) and the holder-PoK member (`sq-xqfg`) are **built but
> not-yet-sound**; the nullifier is **absent**. **Completeness for the privacy layer is
> therefore promissory, blocked, and partly aspirational — it is NOT achievable under the
> current design** and is **not claimed** here. Only the structural/relationship-layer
> completeness (the `ontology` rows, CLAIM 2 in §7.6) is asserted.

So the de-dup'd ten-term core (§2.3) + `forPredicate` sugar covers **all** the *trust-model and
attestation structure* of both technologies (the `ontology` rows), and the coverage table names
the missing **presentation-mechanics** items — the crypto-backed obligations
(`trust:nullifier`, `trust:rateLimit`, the issuer-resolver) **and** the §3.4 holder-binding
architecture change that no amount of ZK composition fixes. A skeptic should attack the claim
*"coverage is complete"* precisely here: the `ontology` rows are defensible from the model, the
`zk-comp` rows are **promissory** (unaudited, `sq-qhy4`), and the `gap` rows — including every
*presentation*-unlinkability row — are **conceded, not hidden**. Completeness holds **only for
the structural/relationship layer**, and is **explicitly not claimed** for the
unlinkable-presentation layer (§7.6, CLAIM 2).

## 6. Prototype plan (decomposed; each phase a future bead)

### 6.0 The next PoC to build — trust-graph EVALUATION, age>18 end-to-end

This is the **single, concrete proof-of-concept to build next** on sparq's existing estate
(it instantiates phases **P1+P3** of the decomposition below; P2/P4–P8 follow). It is a
**research prototype**, not a shipped feature and not a security guarantee; its **privacy half
is out of scope and its ZK estate is unaudited** (`sq-qhy4`). What it demonstrates: a single
externally-attested fact is **admitted** through a trusted-source-scoped gate, then **merged
by N3 reasoning** with an `.acl` rule to **derive `canAccess`** — the §3.1 worked example,
running end-to-end.

**Goal (the one demonstrable claim).** Given (a) the controller-authored `.acl` ABAC rule
`{ ?x schema:age ?y . ?y math:greaterThan 18 } => { ?x auth:read <resourceX> }`, (b) a
`trust:TrustRule` trusting `<https://gov.example/issuer>` for `schema:age` on `<resourceX>`,
and (c) a VC-attested graph `<Jesse> schema:age 25` **signed by the gov issuer over its
RDFC-1.0 commitment**, the PoC **admits** the age fact **iff** the issuer signature verifies,
the statement-type is in scope, the credential is fresh, and the credential subject binds to
the authenticated requester — then **derives** `<Jesse> auth:read <resourceX>` into
`<urn:sparq:auth>`. The **negative** cases must *not* grant: a forged/absent signature, an
out-of-scope predicate (e.g. an `acl:agent` triple smuggled in the same graph), a stale
credential, and a third-party credential with no holder binding.

**End-to-end pipeline (the §3.1 worked example, concretely).**

1. **Parse + canonicalise** the presented credential graph G — `sparq-canon` (RDFC-1.0), the
   *same* canonical unit the ZK estate already commits over.
2. **Verify the issuer signature** over G's RDFC-1.0 commitment against the key the matching
   `trust:TrustRule`'s `trust:issuerKey` names — `sparq-zk` `sig`/`commit` (the **checked**
   signature, never a self-asserted "I am signed" triple; the load-bearing soundness
   condition, §3.3). v1 supplies the verifying key **directly** (operator-asserted) because
   there is **no DID resolver yet** (P2/`sq-pfae.3`); this is the honest, not-end-to-end
   binding called out as the live forgery vector D′ (§3.3, §7.3).
3. **Admission gate** — N3 admission rules + a Rust side-condition that together check: a
   matching `trust:trustsSourceFor` (`trust:source` + `trust:forShape`/`forPredicate`-sugar)
   whose `trust:scope` covers `<resourceX>`; freshness vs `Session.now` within
   `trust:freshWithin` (a per-request Rust check, **not** an in-reasoner predicate); the
   reserved-predicate guard (`solidx:`/`urn:sparq:`) stays in force so a source trusted for
   `schema:age` cannot launder an `acl:`/`solidx:` triple; and the holder binding
   `credentialSubject == Session.agent` (§3.4). On success the fact is injected **issuer-tagged
   as `trust:admitted`** ahead of the materialiser.
4. **Derivation** — the **shipped** `sparq-solid` materialiser runs the `.acl` rule via
   `sparq-reason` (full `reason_n3`, which supports `math:greaterThan`) over the admitted fact;
   `<Jesse> auth:read <resourceX>` lands in `<urn:sparq:auth>` exactly as today. The query is
   rewritten to that allow-list with no engine change.

**Crates it composes (all shipped) + the one new opt-in crate.**

| Role in the PoC | Crate | Status |
|---|---|---|
| `.acl`/`.acr` ACP/WAC issuer rules + materialised `<urn:sparq:auth>` + reserved-predicate guard | `sparq-solid` | shipped |
| N3 reasoning (admission + derivation; full `reason_n3`) | `sparq-reason` | shipped |
| RDFC-1.0 canonicalisation of the credential graph | `sparq-canon` | shipped |
| Issuer signature over the RDFC-1.0 commitment | `sparq-zk` (`sig`/`commit`) | shipped, **ZK estate unaudited** (`sq-qhy4`) |
| Statement-type shape check (the `forShape` primitive; `forPredicate` desugars to it) | `sparq-shacl` | shipped |
| **NEW: the admission stratum itself** — `trust:` vocab loader, the admission N3 rules, the Rust side-conditions (freshness, holder-binding, signature-call), `trust:admitted` injection ahead of the materialiser | **`sparq-trust`** (new, **opt-in cargo feature**, default-OFF) | to build |

The new crate **`sparq-trust`** is the only new code; it is **opt-in (default-OFF cargo
feature)** so the core (`sparq-solid`/`sparq-reason`) stays lean and a pod with no trust graph
behaves exactly as WAC/ACP do now (the strict-additivity property, G6/§2.2). It adds **no new
engine** — it wires the admission gate onto the shipped reasoner/canon/zk/shacl estate and
hands admitted facts to the existing materialiser.

**`sparq-trust` crate surface (the build brief — implement exactly this).** New crate
`crates/sparq-trust`, behind a `sparq-solid` cargo feature `trust-graph` (default-OFF; the only
edge `sparq-solid` gains is one feature-gated call into the admission gate before it
materialises). Modules:

- **`vocab.rs`** — the `trust:` IRIs of §2.3.1 as constants; the **one** desugaring
  `forPredicate P → forShape (sh:targetSubjectsOf P + sh:path P ; sh:minCount 1)` (§2.3.3),
  applied at load.
- **`policy.rs`** — parse a trust policy graph (`.acr`-channel) into `TrustRule { source,
  issuer_key, shape, scope, fresh_within }`; reject a policy that is not Control-gated (§3.2).
- **`admit.rs`** — the admission gate (the algorithm below). Input: presented credential graph
  `G`, the parsed `Vec<TrustRule>`, and the live `Session { agent, now }`. Output:
  `Vec<AdmittedFact>` (issuer-tagged, `trust:admitted`).
- **`wire.rs`** — feed `AdmittedFact`s into the existing `sparq-solid` assertion graph **ahead
  of** the materialiser; everything downstream is unchanged shipped code.

**Admission evaluation algorithm (`admit.rs` — the load-bearing logic).** For each credential
graph `G` and each candidate `TrustRule r`, admit a triple `t = (s,p,o) ∈ G` **iff all** hold
(short-circuit on first failure; default-deny):

```text
admit(G, rules, session) -> admitted:
  cG   := canonicalise(G)                              # sparq-canon RDFC-1.0
  for r in rules:
    if not scope_covers(r.scope, target_resource):     continue            # §3.2 scope
    if not verify_sig(cG.commitment, r.issuer_key):     continue            # §3.3 (1) CHECKED sig
    if session.now - issued_at(G) > r.fresh_within:     continue            # §3.3 (B′) per-request Rust
    if revoked(G):                                      continue            # input-stratified guard
    for t=(s,p,o) in G:
      if is_reserved(p):                                continue            # solidx:/urn:sparq: guard stays in force
      if not shape_admits(r.shape, t, G):               continue            # §2.3.2 forShape / forPredicate-sugar (sparq-shacl)
      if subject_of(t) != session.agent (no holder PoP):continue            # §3.4 holder binding
      emit AdmittedFact{ t, issuer: r.source, mark: trust:admitted }
```

`verify_sig`, `revoked`, freshness, and holder-binding are **Rust side-conditions** (per-request,
not in-reasoner — §3.3 B′); `shape_admits` runs the shipped, terminating `sparq-shacl` validator;
`scope_covers` is a containment check. The emitted facts then enter the **unchanged** `sparq-solid`
materialiser, which runs the `.acl` rule via `sparq-reason` `reason_n3`. **Open-problem hooks the
PoC must respect** (do not silently "solve" them — wire them as the documented degraded path):
`sq-xc4y` (holder-binding/freshness are per-request → **RESOLVED** by the static/dynamic split:
`admit_static` decides the session-independent class once and defers holder/freshness to a
per-request conditional grant, §3.3 A′ — never frozen into the materialise-once view);
`sq-tu4e` (no in-reasoner NAF over
derived facts → `revoked` is an **input-only** seeded predicate; deny-on-disagreement →
**RESOLVED** in §3.5 as a positive conflict witness routed to an access-tuple prohibition plus
deny-overrides — implement it, do not omit it);
`sq-l5og` / `sq-wvne` are **out of PoC scope** (no delegation, no ZK/privacy).

**Acceptance (the adversarial forgery tests, mirroring `acp_forged_*_in_acr_document_does_not_grant`).**
Positive: the age-25 credential grants `auth:read`. Negative (each must **deny**): (i) a graph
with a tampered/absent signature; (ii) a source trusted for `schema:age` presenting an
out-of-scope `acl:agent`/`solidx:creator` triple; (iii) a stale credential past
`trust:freshWithin`; (iv) a third-party credential whose `credentialSubject != Session.agent`;
(v) two trusted issuers attesting contradictory values for an attribute consumed by a grant.
These are the §7.3-E tests that **do not yet exist** and whose absence currently leaves
statement-type-scoping / no-laundering / key-binding as *design intent, not verified property*
(`sq-pfae.4`).

**Explicit non-goals (honest scope).** No privacy / unlinkability (the §5.3 three-part ZK
composite, hard-gated on `sq-qhy4`); no delegation (§4 is design-only, zero substrate); no DID
resolver (P2; keys supplied directly); no incremental admission maintenance (re-materialise on
change). The PoC proves **trusted-source-scoped admission + N3 merge → derived grant** and
nothing stronger.

### 6.1 Full phase decomposition (each phase a future bead)

Build on sparq's existing estate; do not add a new engine. Sequencing: **P1→P2→P3→P4** is the
core spine; **P5, P6, P7** depend on P3/P4; **P7 (privacy) is hard-gated on `sq-qhy4`**.

1. **P1 — Trust-graph vocabulary + semantics note (design-review).** Pin the `trust:` terms
   (§2.3), the admission/derivation two-stratum semantics, the degenerate-`.acl` equivalence,
   and the strict-additivity property. Doc + vocab only; no code. *Blocks all prototype
   phases.*
2. **P2 — DID resolution + issuer-key binding (prototype).** Pluggable `did:key`/`did:web`
   resolver feeding issuer keys into the existing `sparq-zk` signature check. Closes the
   "no DID resolver" gap. *Blocked-on: P1.*
3. **P3 — Claim-level admission stratum (prototype, the core deliverable, closes the `acp:vc`
   gap / G1).** N3 admission rules + Rust wiring that verify an issuer signature over a
   credential's RDFC-1.0 commitment, check `trust:trustsSourceFor`, freshness and the
   subject-to-requester holder binding, and inject admitted facts (issuer-tagged) ahead of the
   sparq-solid materialiser. Reuse `sparq-canon`, `sparq-zk/sig`, the `solidx:`/`urn:sparq:`
   reserved-predicate guards. *Blocked-on: P1, P2.*
4. **P4 — Trust-document storage/authoring model (prototype, G2).** Where trust rules live
   (server vs per-`.acr`), how they are Control-gated, versioned, and revoked; the admission
   cache key (evidence hash + policy version) composed with the sparq-solid epoch cache.
   *Blocked-on: P1.*
5. **P5 — Delegation stratum for human + AI agents (prototype, G3).** ZCAP-LD/UCAN chain
   verification rooted at the resource, monotone attenuation, the agent⊆delegator intersection
   invariant **enforced against the *current* delegator grant**, and — the prerequisite the
   prior draft omitted — the **invocation-binding gate** (authenticated invoker == chain
   terminal delegate, **key-proven per request** via a DPoP/GNAP layer that does **not exist
   today**) so an admitted delegation is **not replayable under key substitution** (each hop's
   `delegate_key` bound into the delegator-signed preimage — the `sq-l5og` soundness fix; this is
   NOT full non-replayability, since the delegator's key is still operator-asserted, `sq-pfae.3`).
   **NOTE: there is ZERO delegation /
   key-proof substrate in the repo today** (no zcap/ucan/dpop/gnap/`actedOnBehalfOf`); this
   phase is design-only at the start, not "build on existing seams". Add delegation-replay
   adversarial forgery tests. `revocation.rs` is a status-list primitive, **not**
   chain-revocation. *Blocked-on: P3. Open problem: `sq-l5og`.*
6. **P6 — Live status/revocation + minimal justification (prototype, G5/G6).** Fetch+decode a
   W3C Bitstring Status List; gate derivations on validity; emit a *minimal* PROV-O
   justification subset for each grant (sparq has PROV-O lineage but not minimal proof trees).
   *Blocked-on: P3.* **PROTOTYPED** (`sq-pfae.7`, opt-in `status-list` feature): the
   `sparq_trust::status_list` module decodes a Bitstring Status List (pluggable resolver +
   GZIP seam, MSB-first clear-index — distinct from the LSB-first ZK `StatusListSnapshot`
   mirror), `admit_with_status` gates the REAL admission path **fail-closed on
   set/unknown/stale**, and `justify_status_decision` renders the minimal PROV-O allow/deny
   justification. The status-list VC's OWN issuer signature is verified by the opt-in
   `VerifyingLiveStatusCheck` (`sq-pfae.13`, fail-closed on unsigned/bad-sig/wrong-key).
   **Incremental revocation is bounded, not delivered (`sq-pfae.14`).** `StatusDelta::between`
   diffs two snapshots of one list across an **epoch bump** and names the changed slots, so the
   caller re-runs the UNCHANGED gate over only the affected grants. It is a *selection* over two
   **input** snapshots — no verdict, no derived-fact read, no in-reasoner retraction — so the
   input-stratified / one-side-bound seeding discipline (`sq-tu4e`) is untouched; a not-newer,
   coverage-changed, or over-limit delta falls back to the full re-check (fail-closed). The skip
   contract needs BOTH halves — `valid_at(now, max_age)` (a whole-list freshness precondition,
   since staleness is a property of the snapshot, not of a slot) AND `!affects(entry)` — under
   which a skipped entry satisfies `verdict(prev) admits ⇒ verdict(next) admits`. **Open
   residue:** this does NOT close the §4.4 stale-authority window — an unchanged bit still ages
   into `Stale` and `affects` will not flag it, which is why `valid_at` is required; a delta
   binds exactly one status list; and deep-chain delegation revocation (§4.4) remains full
   re-materialisation. [OPUS-4.8] [OPUS-5]
7. **P7 — Privacy/ZK admission feasibility (design + caveated prototype, G4).** Show the
   trust-graph derivation can run under sparq's ZK estate to give ZKAP-equivalent unlinkable
   predicate access. ZKAPs-grade unlinkability needs the **three-part composite** (§5.3): the
   hidden-issuer set-membership (**built, unaudited**) + hidden-holder ZK PoP (**built+wired,
   unaudited**) + a **single-use nullifier / rate-limit primitive that is ABSENT** and must be
   built, **plus** replacing §3.4 clear-WebID holder binding with an in-ZK holder-PoP. Selective
   disclosure via BBS+ as a parallel cryptosuite once in-circuit cost is measured. **Hard-gated
   on external sign-off `sq-qhy4`; stays designed/research, never a v1 guarantee.** *Blocked-on:
   P3 + `sq-qhy4`. Open problem: `sq-wvne`.*
8. **P8 — Cost/decidability spike (prototype).** Bound admission-rule evaluation cost and
   confirm one-side-bound seeding everywhere; work-box timings are non-canonical. *Blocked-on:
   P1–P4.* **ANALYSED** (`sq-pfae.9`, [`research/trust-admission-cost-decidability.md`](trust-admission-cost-decidability.md)):
   the bound is **argued, not machine-checked**, for a stated six-condition fragment (safety
   + no head existentials + ground IRI predicates in premise *and* conclusion + no
   term-minting builtin on a recursive cycle + no scope re-entry + no compound term
   constructed in a conclusion), under which data complexity is PTIME; outside it the path is
   **undecidable** — `reason_n3` carries no budget and `wire::derive_grants` validates
   nothing. The last two conditions are load-bearing, not tidying: a variable conclusion
   predicate is range-restricted yet breaks the `|P|·|A|²` bound, and a recursive
   list-/quoted-triple-valued head breaks termination outright with no blank node and no
   builtin. One-side-bound seeding is
   **confirmed for the v1 admission path and refuted as a blanket claim** (two shipped rules
   violate it, both bounded and polynomial). **Enforcement is NOT shipped**: the fragment is
   one the path happens to stay inside, not one it is held inside.

### 6.2 LWS/Solid-WG proposal framing

Take to the WGs the **weaker true claim**: the trust graph is the *formal semantics ACP's
unimplemented `acp:vc` always needed* — the missing "which issuer is trusted for which claim"
predicate — positioned as **strictly additive** to WAC/ACP (no trust graph ⇒ unchanged
behaviour) and **below** ODRL usage control (`crates/sparq-policy`). Present per-(source,
statement-type) trust **honestly as an RT/PERMIS-equivalent relation rendered as RDF a
*shipped Solid reasoner* merges with local `.acl` rules** — a *systems-integration and
standards-fit* contribution, **not** a new trust-model primitive (§2.2, §7 item J); present
claim-level admission as the unifier of ACP-VC / ZCAP / ABAC; present ZK-private admission and
AI-agent delegation as the *least-mature* parts (delegation has **no substrate today**),
explicitly behind the privacy and delegation caveats. Whether to land it as new vocab, an ACP
profile, or both is an open WG question (§8).

## 7. Honest limitations & comparison

This section is **expanded after an adversarial review** (four lenses: ZKAPs/privacy,
evaluation-soundness, delegation, and overclaim/novelty/citation). Each concession below
states plainly what is *implemented-and-verified* vs *designed-only* vs *proposed* vs
*not-yet-sound*, and names the bead that tracks the open work. Read it as the **honest
counterweight** to §§2–6: the *split* claim is sound to take to the WGs, but several pieces the
prior draft phrased as settled are open problems.

### 7.1 Privacy / unlinkability (the ZKAPs half)

- **A — Privacy is not free, not proven, and needs a THREE-part composite (not a single
  axis).** §5.3: a plain VC-presentation discloses the exact attribute value and is linkable.
  Matching ZKAPs' *presentation* unlinkability needs **(i) hidden-issuer set-membership —
  BUILT but NOT-yet-sound / externally unaudited** (`sq-z9l`: `sparq-zk-compose/src/issuer.rs`
  + `zk/compose/compose_core/src/issuer.nr` + the compiled `hidden_issuer_d4` member; gated on
  `sq-qhy4`); **(ii) hidden-holder ZK proof-of-possession — BUILT + WIRED but NOT-yet-sound**
  (`holder.nr` `sq-xqfg`; verifier binding gate `bind_holder_pok` T6/`sq-i1dt` is implemented
  and tested per its CLOSED bead, but is NOT-yet-sound under `sq-qhy4`, so it does **not**
  soundly anonymise the requester today); and **(iii) a single-use nullifier / rate-limiting
  primitive — ABSENT** (no such primitive in `sparq-zk` or `sparq-zk-compose`). The prior draft
  **wrongly** said "the hidden-issuer set-membership upgrade is not yet reached" — corrected
  here: it IS built, the limitation is *soundness/audit*, not *absence*. MPC is semi-honest
  only. No line asserts a settled ZK/MPC privacy or soundness property. Tracked: `sq-wvne`.
- **B — Hiding the issuer is necessary but NOT sufficient; §3.4 holder binding is an
  ARCHITECTURE-CHANGE blocker, not merely a ZK-composition gap.** Even with a hidden issuer +
  selective-disclosure predicate proof + the full ZK composite, §3.4's holder binding
  (`credentialSubject == Session.agent`) authenticates the requester's WebID **in the clear** at
  the derivation stratum, so presentations are **trivially linkable by requester identity** and
  access is **not anonymous** — no amount of ZK composition over the *credential* fixes a *clear
  WebID* on the *request*. A ZKAPs-equivalent presentation must therefore **replace clear-WebID
  holder binding** with an **in-ZK holder-PoP + nullifier** (bind to a key, prove
  membership/possession in zero knowledge, enforce single-use): a **change to the
  admission/holder-binding architecture**, not a compose-the-estate step. Accordingly the
  *presentation*-unlinkability rows of §5.4 are now classified `gap` (architecture change
  required), **not** `zk-comp`. This composite is **designed / partially-prototyped, NOT shipped
  or sound**, and the nullifier is **absent** (`sq-wvne`).
- **C — The superset claim is authorisation-model-level only.** It is *not* a standalone
  cryptographic superset; the unlinkable-presentation half is composition with the un-audited
  ZK layer (§5).

### 7.2 Novelty / citation honesty

- **J — Per-(source, statement-type) trust is NOT a new trust-MODEL primitive.** The **RT
  framework (Li/Mitchell/Winsborough 2002)** and **PERMIS** already express typed/role-scoped
  issuer trust as Datalog, with authority over each attribute-type localised to its issuer. The
  contribution here is **NOT inventing that relation** but (a) rendering it as **RDF a *shipped*
  Solid stratified-NAF reasoner merges with local `.acl`/`.acr` rules**, (b) binding admission
  to **VC Data Integrity / RDFC-1.0 signed-graph verification** on the existing `sparq-zk`
  estate, and (c) recovering **WAC/ACP/Solid-OIDC as degenerate cases** — a **systems-integration
  and standards-fit** contribution, not a trust-model one. The prior draft's absolute "strictly
  finer than EVERY surveyed prior art / None expresses it" (§2.2) **contradicted this doc's own
  §9.1** and is removed; RT and PERMIS are now listed in §2.2 as the prior arts that *do* scope
  issuer-trust by attribute-type, with what they *lack* (RDF/merge-with-local-rules, Solid
  binding, signed-graph admission) stated as the real delta. The §5.2 "more expressive" claim is
  relativised to the *token / one-bit / fixed-predicate* baseline — it is **not** more expressive
  than RT's full Datalog (RT has linked/manifold/threshold roles the v1 predicate-IRI rule does
  not match).
- **ZKAPs ≠ IETF Privacy Pass (citation disentangled).** §5.1 now separates **Least Authority
  ZKAPs** (whitepaper 2021; VOPRF/DLEQ over the *original* pre-RFC Privacy Pass construction;
  *anonymous proof-of-payment*) from the **IETF Privacy Pass RFCs 9576/9577/9578** (June 2024;
  a different issuance protocol targeting attestation/proof-of-humanness). ZKAPs do **not**
  implement those RFCs; the prior "passed attestation" gloss is corrected to "anonymous
  proof-of-payment" for the Least Authority sense.

### 7.3 Evaluation soundness (the trust-scoping / reasoner half)

- **A′ — ADMISSION-VS-MATERIALIZE-ONCE GAP — RESOLVED (decision (a); `sq-xc4y`, shipped).**
  Holder binding + freshness are **per-request**, but the shipped auth view is materialised
  **once, session-independently** (`PodStore`, `lib.rs`) and queried per-session. The decision:
  **split static admission (signature / type-scope / scope, materialise-time) from dynamic
  admission (holder-binding / freshness, query-time)**, with the dynamic class deferred to a
  **conditional grant** re-checked per request by the shipped sq-0q7n `cond_applies` path
  (`auth:agent`↔`Session.agent`, `auth:notAfter`↔`Session.now`). Holder binding does NOT degrade
  to a frozen static match — it is a live per-request check — and freshness lapses at query time
  without a re-materialise. This generalises the sq-0q7n time-window precedent to the identity
  dimension; option (b) per-request re-materialise was rejected (negates P4, redundant with the
  conditional-grant machinery). See §3.3 for the full rationale and the soundness test. Residue:
  post-materialise *revocation* is still a re-materialise event (external-state change, not a
  request fact) — only the two pure-`Session` conditions are deferred.
- **B′ — Freshness is not in the reasoner; revocation is input-stratified.** Time is a
  per-request Rust check (`authindex.rs`). The incremental counting profile permits NAF only
  over input-only predicates; the full evaluator permits derived-predicate NAF only through
  `reason_n3_stratified`, after the predicate is complete in an earlier stratum. The v1
  `not-revoked` guard is therefore NAF over an input-only `revoked` predicate seeded before
  admission; same-stratum derived revocation would be **unsound** (`sq-tu4e`).
- **Re-opening the §2.4 boundary is the principal soundness risk.** §3.3 holds *only* if
  admission verifies **real signatures** (never self-asserted trust triples) and enforces
  statement-type scoping; a bug there is privilege-escalation, not a cosmetic defect.
- **E — Statement-type scoping / no-laundering / key-binding are DESIGNED, not VERIFIED.** The
  adversarial forgery tests that would establish them — mirroring the existing
  `acp_forged_*_in_acr_document_does_not_grant` suite (`crates/sparq-solid/tests/acp.rs`) for
  *out-of-scope predicate*, *replayed/stale credential*, *third-party credential without holder
  binding*, *reserved-`solidx:` smuggled through an admitted graph* — **DO NOT YET EXIST**.
  Until they do, scoping/no-laundering/key-binding are **design intent**, not established
  properties (`sq-pfae.4` carries the test obligation).
- **D′ — Issuer-key binding is a LIVE forgery vector, not a footnote.** With **no DID resolver**
  (P2/`sq-pfae.3` unbuilt), `trust:issuerKey → verifying-key` is **unverified** = **silent
  privilege escalation**, gated only by P2 — stated as an **active surface** alongside the
  `sig.rs` disclosure caveat (`sq-tu4e`).
- **C′ — Seeding-caveat citation corrected; the real termination risk is unanalysed.** The
  "two-unbound-atom seeding blow-up" belongs to the **incremental counting path**, not the path
  solid runs: **solid uses the full evaluator through both `reason_n3` and
  `reason_n3_stratified`** (the latter already drives its three-stratum ACP materialisation).
  The real,
  *unanalysed* termination risk is **recursive / unbound-join admission rules over external-graph
  extents in the full evaluator** — P8 (`sq-pfae` P8) must bound *that* path (`sq-tu4e`). No
  formal complexity bound is proven here.
  **Superseded in two places by the P8 analysis** (`sq-pfae.9`,
  [`research/trust-admission-cost-decidability.md`](trust-admission-cost-decidability.md) §0):
  (a) the seeding blow-up is **not** confined to the incremental path — its canonical recorded
  instance is a *full-evaluator* rule in `crates/sparq-solid/rules/common.n3`, hand-split for
  exactly this reason; (b) since `sq-zgbso.4` the production materialiser runs the **compiled**
  id-level evaluator, not the text engine, which now survives there only as the differential
  test oracle. The formal bound is in that record; termination and seeding cost are shown there
  to be **independent** properties, which this paragraph conflates.
- **F — Conflicting-fact deny-on-disagreement is reachable.** A positive join over two
  disagreeing issuer-tagged facts derives a conflict/prohibition witness, and existing
  deny-overrides defeats the competing grant without NAF. Only the alternative “grant iff no
  derived conflict exists” encoding needs derived-predicate NAF: it requires the shipped
  `reason_n3_stratified` driver and is unsound in a single no-retraction fixpoint. v1 uses the
  positive-witness form; conflict resolution beyond
  deny-overrides + conservative-admission is designed-only.
- **Revocation/freshness is full-re-run, not incremental.** Stale-grant ("new enemy") risk is
  bounded by re-materialisation, not by retraction; incremental/temporal maintenance is not
  shipped.
- **Entailment laundering is excluded by fiat in v1** (directly-attested facts only); the
  trust-propagation alternative is unbuilt.

### 7.4 Delegation (the capability / AI-agent half)

- **K — ZERO delegation substrate exists today; §4 is DESIGN-ONLY.** There is **no
  ZCAP/UCAN/on-behalf-of/holder-binding/key-proof code in any crate** (verified by grep across
  `sparq-solid` and `sparq-server`). The cited `sparq-zk-compose/src/revocation.rs` is a
  Merkle/status-list ZK primitive with **no chain-revocation**, and `sparq-prov` records
  **single-agent** `prov:wasAssociatedWith`, **not** an `actedOnBehalfOf` delegation chain. §4.3's
  "PROV-O records the chain" and "key-proofing" are reclassified from **implied-shipped to
  proposed**.
- **K′ — Invocation is distinct from delegation; the invocation-binding gate — `sq-l5og` —
  is now SPECIFIED, ENFORCED, and TESTED in the PoC ([OPUS-4.8]).** The rule
  **authenticated invoker == the carried chain's terminal delegate, key-proven per request**
  (the delegation analogue of §3.4 holder binding) is implemented in
  `crates/sparq-trust/src/delegation.rs` (`invoke()`), with the delegation-replay forgery
  matrix — the analogue of `acp_forged_*_in_acr_document_does_not_grant` — in
  `crates/sparq-trust/tests/delegation_replay.rs`. The per-request fresh-challenge
  proof-of-possession (DPoP/GNAP-style, on the shipped `sparq-zk` PoP primitive) — together with
  binding each hop's `delegate_key` into the delegator-signed `hop_message` (the soundness fix an
  adversarial review forced, after a confirmed key-substitution BYPASS where the terminal
  `delegate_key` was attested by no signature) — defeats stolen-chain **key-substitution** replay.
  Do **not** overclaim full non-replayability: the delegate key is only as trustworthy as the
  delegator's key that attests it, and that key is still operator-asserted (`sq-pfae.3`).
  **Still open** (documented, not solved): deep-chain *incremental* revocation (full
  re-materialisation only) and DID-resolver delegate-key binding (`sq-pfae.3`, the live forgery
  vector D′). It is a research PoC, **not** a shipped security guarantee.
- **M — Ambient-authority self-contradiction.** §4.1's "admitted-as-graph-fact" model **is** the
  ambient lookup §4.3 warns against; storing delegations as ambient graph facts re-introduces
  ambient authority **unless** the invocation binding (K′) gates **every** read. v1 does not yet
  resolve which model it takes (`sq-l5og`).
- **N — Intersection invariant is UNENFORCED and must bind the CURRENT delegator grant.** The
  `effective = delegator-permissions ∩ agent-allowed-scope` rule is a **design assertion** — no
  engine mechanism is shown that forces it; "enforce monotonicity" is a *requirement*, not a
  guarantee. It must recompute against the **current** delegator grant on every
  re-materialisation, **never** the delegation-time snapshot; until incremental revocation
  ships there is an **unbounded stale-authority window** for long-lived (AI-agent) sessions,
  bounded only by epoch re-materialisation cadence (`sq-l5og`).
- **O — P5 explicit non-goal: v1 does NOT prevent confused-deputy in the obj-cap sense.** The
  key-proof / DPoP layer is **absent**, so per-request key binding is a **prerequisite, not a
  property**. The whole AI-agent delegation story is a **research hypothesis under live
  caveats** — the same posture §5.3 takes for the ZK/privacy half — not a near-term,
  substrate-backed deliverable. (Citation anchors are accurate: Stanford/MIT "Authenticated
  Delegation and Authorized AI Agents", arXiv 2501.09674; UCAN attenuation/revocation; RFC 8693
  Token Exchange — the defect is the gap between those standards and sparq's *substrate*, not
  the citations.) **AI-agent delegation standards are early/moving** (draft-klrc-aiagent-auth,
  OAuth on-behalf-of drafts, GNAP); the intersection invariant and default-tight attenuation are
  sparq-local design choices, not adopted standards.

### 7.5 Open-problem beads created by this review

The genuine **design gaps** (not mere caveats) surfaced above are tracked as beads under
`gh-940`, to be sequenced by the orchestrator:

- `sq-xc4y` — per-request holder-binding/freshness admission vs session-independent
  materialise-once auth view: **RESOLVED** — decision (a), the static/dynamic split, shipped in
  `sparq-trust` (`admit_static` / `derive_conditional_grants`) + `sparq-solid`
  (`admit_trust_credential_static`, conditional-grant install on the sq-0q7n path); §3.3 A′.
- `sq-l5og` — delegation invocation-binding gate (invoker == terminal delegate, key-proven).
  **[OPUS-4.8] RESOLVED in the PoC** (`crates/sparq-trust/src/delegation.rs` + the
  `delegation_replay` forgery tests): the gate defeats stolen-chain **key-substitution** replay
  (each hop's `delegate_key` is bound into the delegator-signed `hop_message` — the soundness fix
  after a confirmed bypass), takes the obj-cap side of the ambient-authority tension (item M), and
  binds the intersection to the *current* delegator grant (item N). It does NOT claim full
  non-replayability — the delegator's key is still operator-asserted. Residual open: deep-chain
  incremental revocation + DID-resolver key binding (`sq-pfae.3`).
- `sq-tu4e` — **PARTIALLY RESOLVED (conflict reachability only):**
  conflicting-issuer-fact deny-on-disagreement is a monotone positive conflict witness routed
  to an access-tuple prohibition and followed by deny-overrides; only the alternative
  no-derived-conflict grant needs the full evaluator's explicit stratified driver. **Still
  open under this bead:** issuer-key binding (the LIVE forgery vector gated on
  P2/`sq-pfae.3`) and the unanalysed recursive-admission termination risk (§7.1 C′/D′).
  Freshness stays Rust-side, revocation is input-stratified, and the seeding citation is
  corrected.
- `sq-wvne` — ZKAPs-grade unlinkable presentation needs a 3-part ZK composite (hidden-issuer +
  ZK holder-PoP + nullifier); clear-WebID holder binding is in tension with anonymity.

### 7.6 Load-bearing claims a skeptic should attack first

**What this document does and does NOT claim (read before attacking).** This record is
**self-adversarial**: it opens in design-for-review status and spends all of §7 dismantling its
own prior draft's overclaims. The *primary* load-bearing claims a reviewer should attack are the
**concessions of §7.1–7.4**, audited for *adequacy* — i.e. is each gap conceded honestly and
fully, or does residual overclaim survive? Those concessions are specific and falsifiable, and
are framed as **open problems, not settled claims** (with the exception of
admission-vs-materialise-once `sq-xc4y`, now RESOLVED by the static/dynamic split, §3.3 A′, and
conflict / deny-on-disagreement reachability `sq-tu4e`, resolved in §3.5):
delegation invocation-binding (`sq-l5og`), the remaining freshness / issuer-key limits from
`sq-tu4e`, and the ZK-presentation composite +
§3.4-holder-binding architecture blocker (`sq-wvne`). The document makes **no claim** to
cryptographic losslessness, to a *proven* minimal set, or to ZKAPs-grade unlinkability; the only
two *constructive* completeness claims it stakes are the two below (de-dup losslessness scoped to
the predicate-only direction, and coverage scoped to the structural/relationship layer). Both are
stated as **falsifiable** targets, with the counter-evidence that would sink each:

- **CLAIM 1 — "the de-dup is lossless."** Every term dropped from the prior draft is either a
  primitive that survives in the ten-term core (§2.3.1) or a *convenience that desugars without
  loss* into a primitive (§2.3.3). The only collapse is `trust:forPredicate` → `trust:forShape`.
  **How to falsify:** exhibit a `trust:forPredicate P` rule whose §2.3.3 shape rewrite
  (`sh:targetSubjectsOf P` + `sh:path P ; sh:minCount 1`) admits a **different** fact set than
  the predicate rule would — i.e. the rewrite is *not* extensionally equal on some graph; **or**
  show that one of the ten "primitives" is in fact derivable from the others (the §2.3.2
  irreducibility argument is wrong for some term). *Known boundary (conceded):* the desugaring
  is lossless **only in the predicate-only direction** — `forShape` is strictly more expressive
  and does **not** reduce back to `forPredicate`; the claim is that `forPredicate` adds no
  expressivity, not that the two are interchangeable.
- **CLAIM 2 — "coverage is complete."** It is **deliberately bounded**: complete for the
  **structural/relationship layer** of ZKAPs and IETF Privacy Pass (the `ontology` rows of
  §5.4), and **explicitly NOT claimed** for the unlinkable-presentation layer (the `zk-comp`
  rows are promissory, gated on `sq-qhy4`; the `gap` rows — *including every presentation-
  unlinkability row* — are conceded). The privacy-layer completeness is **not achievable under
  the current design**: every *presentation*-unlinkability row of §5.4 is now a `gap`, not a
  `zk-comp` row, because §3.4 clear-WebID holder binding leaks the requester identity regardless
  of the ZK composite (the §5.4.3 item-5 architecture-change gap). **How to falsify:** name a
  ZKAPs or RFC 9576/9577/9578 *structural/trust-relation/attestation* concept that has **no**
  `ontology` row — that would break the bounded completeness claim; **or** show that a row marked
  `ontology` actually requires ZK composition (mis-classified as deliverable when it is
  promissory or blocked), **or** that a `zk-comp` row is in fact a hard `gap` (the
  built-but-unsound status of `sq-z9l`/`sq-xqfg` is overstated). The claim is **not** "the trust
  graph delivers ZKAPs' privacy" — that is the `zk-comp`/`gap` half, and asserting it would be
  the dishonest overclaim §5–§7 exist to prevent.
- **CLAIM 3 (the original soundness claim, restated for completeness).** Admission re-opens the
  §2.4 content/reasoner boundary safely **only if** it verifies real issuer signatures (never
  self-asserted trust triples) and enforces statement-type scoping; **and** the adversarial
  forgery tests that would establish this **do not yet exist** (§7.3-E, `sq-pfae.4`), so
  scoping/no-laundering/key-binding are **design intent, not verified properties** — the PoC
  (§6.0) exists to convert them into tested ones.

## 8. Open questions for the maintainer

1. **Trust-document scope:** server-wide, per-`.acr`, or both (recommendation: both,
   per-`.acr` narrowing)?
2. **Statement-type granularity:** predicate-IRI (v1, decidable) vs SHACL shape (expressive)
   vs graph pattern?
3. **Superset claim strength for the WG:** the weaker true one (recommended) or a stronger
   framing the maintainer wants to defend?
4. **AI-agent delegation:** a new principal *kind* or just a capability chain with a
   human/AI attribute (recommendation: the latter — fewer new primitives)?
5. **Privacy posture for v1:** in-the-clear admission now with ZK later (recommended,
   honest), or ZK from the start (blocks on `sq-qhy4`)?
6. **Obligations:** reuse the ODRL bridge (`sparq-policy`) for duties/usage-control above the
   access decision, or keep them separate?
7. **Standards-track shape:** new `trust:` vocabulary, an ACP profile that seeds matchers from
   `acp:vc`/`acp:issuer`, or both?

## 9. Prior art and sources

### 9.1 jeswr's own lws-acp design (build on, credit, and diverge)

This design **builds on jeswr's `lws-acp/docs`** (<https://github.com/jeswr/lws-acp/tree/main/docs>:
`datalog-core.md`, `layering.md`, `expressivity-matrix.md`, `model-encodings.md`,
`layering-lws-context.md`), digested in the in-flight companion record
`research/trust-graph-prior-art-lws-acp.md` (PR #943). He noted some of it "could be crap";
crediting the good and dropping the bad with a one-line reason:

**KEEP (sound, reused):**

- The **Layer-0 / Layer-4 split** — truth-condition rules (Layer 0) separated from
  evidence/credential *admission* (Layer 4). This design's two strata (derivation / admission)
  *are* that split, made concrete on the shipped reasoner.
- **Stratified Datalog** as the technology-neutral core; every AC paradigm is a predicate
  profile. Matches the shipped stratified-NAF reasoner.
- The **trusted-issuer-guarded admission rule** pattern (PERMIS-style `roleAssign :- attrCert,
  issuedBy, trustedIssuer, fresh, notRevoked`) — generalised here from *global* `trustedIssuer`
  to *per-statement-type* `trustsSourceFor`.
- The **per-model encodings** ("Datalog Rosetta stone") as the evidence that one core subsumes
  ACL/RBAC/ReBAC/Zanzibar/Capability — the backbone of the superset argument's
  authorisation-model half.
- The **admission cache key** = evidence hash + policy version (composes with the sparq-solid
  epoch cache).

**REWORK:**

- `trustedIssuer` (global) → `trustsSourceFor` (per-statement-type). *Reason: global issuer
  trust cannot say "gov for age, not for role" — the whole point.*
- 15 layers → **2 normative surfaces** (admission + derivation). *Reason: the 15-layer stack
  re-specifies off-the-shelf protocols (OIDC/UMA/GNAP/TLS/ZCAP-LD) the WG should reference, not
  re-standardise.*
- `lws:allOf`/`lws:not` combinators → reuse ACP `acp:allOf`/`acp:noneOf`. *Reason: they collide
  with the ACP vocab already shipped in `sparq-solid`; reuse, don't fork.*
- Closed-world deny → **split into open-world fact admission + closed-world access derivation**.
  *Reason: admitting attested facts is open-world; the access decision stays closed-world
  fail-closed (D4).*

**DROP (with one-line reason):**

- **OSI-style layer numbering as normative** — *presentational scaffolding, not semantics; a WG
  will not standardise a 15-layer OSI analogy.*
- **Re-specifying OIDC/UMA/GNAP/TLS/ZCAP-LD** — *off-the-shelf; reference them, do not
  re-standardise them.*
- **The unqualified "superset-of-ZKaps" claim** — *over-stated; the honest split (§5) replaces
  it. The expressivity-matrix doc's own finding is "no single most-expressive model and no
  supremacy claim", which undercuts any unqualified superset.*
- **`layering-lws-context.md`'s deferral of the LWS binding and delegation** — *it admits it
  does not map layers to Solid concepts and defers delegation; this design supersedes it by
  binding to the shipped sparq-solid substrate and specifying delegation (§4). Kept idea: the
  admission cache key.*

### 9.2 In-flight companion prior-art surveys (this design synthesises them)

Six prior-art surveys were produced for this design and are **open PRs against `main`** at
time of writing (not yet merged), cited by path + PR:

- `research/trust-graph-prior-art-lws-acp.md` — PR #943 (jeswr's lws-acp digest).
- `research/trust-graph-prior-art-odrl.md` — PR #942 (ODRL evaluation frame; the missing
  admission theory).
- `research/trust-graph-prior-art-*` (RBAC/ABAC/NGAC/ReBAC + trust-management) — PR #945.
- `research/trust-graph-rdf-integrity-prior-art.md` — PR #946 (named-graph/quad trust,
  RDFC-1.0, N3 scoped reasoning).
- `research/trust-graph-prior-art-semweb-ac.md` — PR #947 (WAC/ACP/`acp:vc`, AIR,
  Protune/PeerTrust, Shi3ld, Kirrane survey).
- `research/trust-graph-capabilities-delegation-zkaps.md` — PR #949 (UCAN/ZCAP/Macaroons/
  Biscuit/SPKI/Privacy-Pass/anon-creds).

### 9.3 In-repo substrate (verified for this design)

- `crates/sparq-solid/` — WAC/ACP as N3 rules (`rules/*.n3`), `acp:issuer` dimension
  (`src/authindex.rs`), `AccessProvenance` + `solidx:` reserved-predicate guard
  (`src/loader.rs`, `src/provenance.rs`), materialised `<urn:sparq:auth>` view
  (`src/materialize.rs`). `acp:vc` has **zero** occurrences.
- `crates/sparq-reason/` — stratified-NAF N3 reasoner (the derivation engine).
- `crates/sparq-canon/` — RDFC-1.0 canonicalisation (W3C Rec) as a public API.
- `crates/sparq-zk/`, `crates/sparq-zk-compose/` — per-named-graph RDFC-1.0 commitment +
  issuer signature (`src/{canon,commit,sig}.rs`), `zk:issuerKey`/`zk:statusList` registry
  (`src/registry.rs`), revocation (`src/revocation.rs`). **Not externally audited (`sq-qhy4`);
  the in-the-clear issuer check discloses which issuer signed.**
- `crates/sparq-policy/` — ODRL 2.2 subset + fail-closed evaluator (the usage-control layer
  above access control).
- `crates/sparq-prov/` — PROV-O lineage for derived data.
- `research/solid-access-control-design.md` (§2.4 boundary, the shipped AC substrate);
  `research/feature-research-odrl-policy.md`; the `zk-*` and `mpc-*` design records.

### 9.4 External standards and literature

- Solid WAC <https://solidproject.org/TR/wac>; ACP <https://solidproject.org/TR/acp>;
  Solid-OIDC <https://solid.github.io/solid-oidc/>.
- W3C VC Data Model 2.0 <https://www.w3.org/TR/vc-data-model-2.0/>; VC Data Integrity +
  RDFC-1.0 <https://www.w3.org/TR/rdf-canon/>; VC-DI-BBS <https://www.w3.org/TR/vc-di-bbs/>;
  Bitstring Status List <https://www.w3.org/TR/vc-bitstring-status-list/>; DID Core
  <https://www.w3.org/TR/did-1.0/>.
- W3C-CCG ZCAP-LD <https://w3c-ccg.github.io/zcap-spec/>; UCAN
  <https://ucan.xyz/specification/> (and UCAN revocation <https://ucan.xyz/>); Macaroons
  (Google, NDSS 2014); Biscuit <https://github.com/eclipse-biscuit/biscuit>; SPKI/SDSI
  RFC 2693; GNAP RFC 9635.
- **IETF Privacy Pass RFC 9576 / 9577 / 9578 (June 2024)** — the standardised attestation
  issuance/redemption protocol. **Distinct from** Least Authority's ZKAPs (below): the RFCs
  target proof-of-humanness/attestation, ZKAPs target anonymous proof-of-payment over the
  *original* pre-RFC Privacy Pass VOPRF/DLEQ construction. Do not cite them as one (§5.1).
- NIST SP 800-162 (ABAC); ANSI/INCITS 359 (RBAC); INCITS 565-2020 (NGAC); NIST SP 800-178
  (XACML vs NGAC); OASIS XACML 3.0 + Administration & Delegation Profile.
- Li, Mitchell, Winsborough, *Design of a Role-Based Trust-Management Framework* (IEEE S&P
  2002) — the formal antecedent (attested statements → Datalog, cross-domain delegation).
- Carroll, Bizer, Hayes, Stickler, *Named Graphs, Provenance and Trust* (WWW 2005); Zanzibar
  (USENIX ATC '19); Kirrane, Mileo, Decker, *Access control and the RDF: a survey* (Semantic
  Web 8(2), 2017).
- More, Ramacher, Alber, Herzl, *Extending Expressive Access Policies with Privacy Features*
  (2022) — closest published prior art to the privacy half (predicate proofs over private
  attributes).
- OAuth Token Exchange RFC 8693; draft-klrc-aiagent-auth; Stanford *Authenticated Delegation
  and Authorized AI Agents* (arXiv 2501.09674).
- **ZKAPs (Least Authority)** <https://leastauthority.com/product-development/zkaps/> — *Zero-
  Knowledge Access Passes* whitepaper (July 2021); anonymous **proof-of-payment** tokens over
  the original pre-RFC Privacy Pass VOPRF/DLEQ construction. **Not** an implementation of the
  IETF Privacy Pass RFCs (above); see the §5.1 disentanglement and §7 item J.
