<!-- [OPUS-5] sq-lhcot.6 (issue #2787) — design record for the opt-in independent
`urn:concept` verifier over sparq-canon. DESIGN ONLY: the deliverable is gated on the
Kern/PSS #1746 profile + fixture freeze and is deliberately NOT implemented here. -->

# Design record — the opt-in independent `urn:concept` verifier over `sparq-canon`

**Bead:** `sq-lhcot.6` (issue #2787). **Parent:** `sq-lhcot` / issue #2581.
**Routed surface:** `crates/sparq-canon`.
**Coordination:** `defer_to_kern` — GitHub #1683 (definition) and #1746 (profile/fixture freeze).
**Status:** **DESIGN ONLY — NOT IMPLEMENTED, AND DELIBERATELY SO.** The gate this work is
conditioned on has not been observed to close (§1). Nothing in `crates/` changes with this record.

> **AMENDMENT — [SONNET-4.6], issue #1746.** The *profile-independent* part of phase 3 has since
> landed as `crates/sparq-canon`'s opt-in, off-by-default **`concept`** feature: the
> multibase/multihash envelope (both published multiformats specs, not Kern's to freeze) and the
> fail-closed, constant-time, full-multihash-prefix recompute-and-byte-compare of §5.2, over a
> **caller-supplied** quad set. The gate of §1 is NOT treated as closed: the scope-extraction rule
> (§3, §5.1) is still Kern's, is still not vendored, and is deliberately absent from the
> implementation — which is why the caller passes the quads. **Still blocked:** phase 1 (vendor the
> frozen profile + authoritative corpus), phase 2 (a scope-canonicalization primitive, which
> presupposes a scope rule), and the phase-4 wiring of the guard into an actual ingestion path. The
> §2 independence caveat and the §3 not-whole-graph-RDFC-1.0 statement are carried verbatim into
> the module rustdoc, the crate README, and `skills/rdf-canon/SKILL.md` (phase 5, partial).
> Open questions **Q1, Q2 and Q4-for-the-scope-primitive remain open**; **Q3 is answered only for
> this implementation's own conventions** (trailing newline hashed; empty scope rejected rather
> than hashed; permitted codes = sha2-256/384/512 at full length), stated as conventions so a
> disagreement with the frozen profile shows up as a visible difference, not a silent one.

## 1. Gate status: the freeze has not landed in this repository

The bead is explicit: *"gated on the #1746 profile/fixture freeze — do NOT implement ahead of the
frozen boundary. Kern/PSS OWN the definition algorithm, corpus, and authoritative fixtures."*
Before designing anything, that gate was checked against the tree rather than assumed.

What the repository actually says today:

| evidence | reading |
|---|---|
| `crates/sparq-vectors/src/spqv_provenance.rs:36-47` — *"The reserved extension area (KERN BOUNDARY — do not implement ahead of the profile freeze) … reserved pending a cross-implementation profile (#1746)"* | the boundary is declared **open** |
| `skills/vector-search/SKILL.md:1364-1375` — *"**No** fields are defined over it … The extensible-provenance profile itself (the `#1746` reserved-area semantics) is NOT frozen"* | same, in the user-facing surface |
| `crates/sparq-vectors/README.md:81` — *"profile (#1746) — no encoder privileged"* | same |
| no `urn:concept` string anywhere in the tree | no record format has been received |
| no `multihash` string anywhere in the tree | no digest envelope, no dependency, no fixtures |
| no Kern/PSS concept fixtures under `crates/*/tests/` | the authoritative corpus has not been vendored |

So there is no frozen record serialization to parse and no authoritative fixture corpus to validate
against. **Implementing the verifier now would mean inventing the very artifact Kern owns** — which
is the precise failure mode the existing KERN-boundary convention was written to prevent, and it
would privilege sparq's guess as the de facto profile. The correct output of this bead, at this
moment, is this record: the design pinned down and the primitives inventoried, so that when the
freeze lands the implementation is mechanical rather than exploratory.

*Verification limit (honest):* the live state of GitHub #1683/#1746 was **not** queried — this task
runs under an orchestration contract that forbids GitHub API calls. The conclusion above is drawn
from in-repo evidence only. If the freeze has in fact closed upstream and the profile + fixtures are
simply not yet vendored here, the correct next step is still **vendor the frozen artifacts first**
(phase 1 below), not implement against a remembered shape.

## 2. A correction to the brief's premise: what "independent" can and cannot mean

The bead title says *"independent … verifier over sparq-canon"*. This needs pinning down, because
the obvious reading is not true and would become a false assurance claim if it reached a README.

**`sparq-canon` is not an independent implementation of RDFC-1.0.** It delegates the algorithm to
the third-party `rdf-canon` crate; the crate is the *public surface* plus the single oxrdf-0.3↔0.2
bridge, not a second implementation. This is stated in `crates/sparq-canon/Cargo.toml`
(*"The RDFC-1.0 ALGORITHM itself is the maintained zkp-ld `rdf-canon` crate … This crate is the
clean PUBLIC SURFACE"*) and again in `research/gap-canon-native-2026-07.md` §2
(*"NOT an independent-implementation comparison"*).

Therefore the independence this bead can honestly deliver is:

- **Independent of Kern's concept implementation** — sparq re-derives the declared digest from the
  received record with its own code path, so a bug or a compromise in the Kern-side producer is
  caught at sparq's ingestion boundary. **This is the real deliverable.**
- **NOT independent of the RDFC-1.0 implementation** — if the concept definition reuses RDFC-1.0
  primitives, both sides may end up on the same `rdf-canon` lineage, and an algorithm-level bug
  there would be reproduced identically by both and cancel out. Any independence claim must carry
  this caveat. (The genuinely cross-implementation check in this repo is the JS `rdf-canonize`
  column of `bench/canon/run.sh`; if cross-implementation assurance on the canonicalization step is
  wanted, that is the lever, and it is a separate piece of work.)

Wording that must **not** be used: "double-implementation verified", "independently proven". Wording
that is accurate: *"the declared digest is independently recomputed at ingestion by a second
implementation of the concept-definition algorithm; the underlying canonicalization primitive is
shared, so this detects producer-side defects, not defects in RDFC-1.0 itself."*

## 3. Node-level concept hashing is **NOT** whole-graph RDFC-1.0 — demonstrated

The bead requires this to be recorded explicitly, per the posted #1746 comment. It is recorded here
with counterexamples produced by running this repository's own `sparq-canon`, not by reading the
specification.

RDFC-1.0's canonical blank-node labels are a function of the **entire input dataset**, by two
independent mechanisms: a blank node's first-degree hash is computed over *every* quad mentioning it
in the dataset, and the `c14nN` identifiers are then issued by a single counter walking the sorted
hash list for the *whole* dataset. Restricting to a node/SCC scope perturbs both. Concretely, with
`sparq_canon::issue_quads`:

**Counterexample A — an out-of-scope quad swaps the in-scope labels.** Scope `S = {_:a, _:b}`, both
carrying only `<http://ex/p> "v"`, so they are indistinguishable *within* `S`. The whole graph `G`
adds one quad `_:a <http://ex/outside> "w"` that is **not** part of the scope:

| | `_:a` | `_:b` |
|---|---|---|
| `issue_quads(S)` — scope canonicalized alone | `c14n0` | `c14n1` |
| `issue_quads(G)` — whole graph | `c14n1` | `c14n0` |

**Counterexample B — unrelated components shift the counter.** Scope `S = {_:a}`; `G` adds three
wholly disconnected blank nodes sharing no term with `S`. `issue_quads(S)` gives `_:a ↦ c14n0`;
`issue_quads(G)` gives `_:a ↦ c14n2`.

The consequence is sharp:

> `issue_quads(G)` restricted to `S` **≠** `issue_quads(S)`, in general.

So a per-node/SCC concept hash is **not** an RDFC-1.0 property of `G`. It is a different function,
parameterized by a scope-extraction rule, that may **only reuse lower-level canonicalization
primitives** — the N-Quads term serialization and escaping, the term collation order, and the hash
profile — never the whole-graph result. Any documentation, README, or `SKILL.md` line describing
this capability must say so; describing it as "RDFC-1.0 canonicalization of the concept" would be
false.

### 3.1 The fork this creates — a real question for Kern, not a detail

The two label sources are not merely different, they have **incompatible properties**, and the
profile must pick one deliberately:

- **Scope-alone labels** (canonicalize the extracted subgraph in isolation) are **local**: the
  concept hash depends only on the concept, so it is stable as the surrounding graph grows, and it
  is reproducible by a consumer holding only the concept. But it is *not* RDFC-1.0 over `G`, and two
  scopes that are isomorphic in isolation collide even when the wider graph distinguishes them.
- **Whole-graph labels** (canonicalize `G`, then project onto `S`) agree with RDFC-1.0 over `G`, but
  make the concept hash **non-local**: counterexamples A and B show that adding triples *elsewhere
  in the graph* silently changes a concept's digest. That is unusable as a content identifier, and
  it is an availability/grief vector — any writer who can add unrelated triples can invalidate every
  concept digest in the store.

The design below assumes **scope-alone**, because non-locality disqualifies the alternative for an
identifier — but this is sparq's *reading*, not an agreed decision, and it is open question **Q1**.

### 3.2 The boundary-blank-node hazard

Counterexample A is also the general adversarial-fixture class. If the scope is permitted to contain
a blank node that also occurs in quads outside the scope, that blank node has **no stable identity**:
its identity inside the scope is decided by the extraction rule, and its identity in `G` is decided
by RDFC-1.0, and §3 shows these disagree. Robust profiles avoid this by requiring the scope to be
**blank-node-closed** (if a blank node is in the scope, every quad mentioning it is in the scope) —
which is exactly why an SCC-based scope is a plausible definition. Whether the frozen profile
mandates closure, and what a verifier does with a non-closed record, is open question **Q2**.

## 4. What `sparq-canon` already provides (verified against the source)

The primitives the verifier will build on exist today and are stable public API in
`crates/sparq-canon/src/lib.rs`:

| primitive | signature | role in the verifier |
|---|---|---|
| `parse_nquads` | `(&str) -> Result<Vec<Quad>, CanonError>` | read the record's serialized scope |
| `canonicalize_quads` | `(&[Quad]) -> Result<String, CanonError>` | the canonical byte image of a scope, SHA-256 profile |
| `canonicalize_quads_with::<D>` | `(&[Quad]) -> Result<String, CanonError>` | as above under a non-default hash profile (e.g. SHA-384) |
| `digest_quads_with::<D>` | `(&[Quad]) -> Result<Vec<u8>, CanonError>` | digest of the exact canonical document, including the trailing newline |
| `issue_quads` / `issue_quads_with::<D>` | `-> Result<HashMap<String, String>, CanonError>` | the issued-identifier map — the primitive §3 is stated in terms of |
| `canonicalize_triples` | `(&[Triple]) -> Result<CanonicalGraph, CanonError>` | single-graph form; `CanonicalGraph.lines[i]` is a stable per-triple image |
| `CanonError::TripleTerm` | — | RDF-1.2 triple terms are rejected on the standard paths (fail-closed) |

Two properties matter for a byte-comparing verifier and are worth naming, because a re-derivation
that disagrees on either produces a spurious rejection:

- `digest_quads_with` hashes **every canonical byte, including the final trailing newline** on a
  non-empty dataset (documented at `lib.rs:315-320`). Whether the frozen profile hashes the same
  byte string — trailing newline included, and whether an empty scope hashes the empty string or is
  an error — is open question **Q3**.
- `D` in `digest_quads_with::<D>` selects **only the final digest**; canonicalization itself retains
  the RDFC-1.0 default hash profile. A profile that varies the *canonicalization* hash needs
  `canonicalize_quads_with::<D>` plus an explicit outer digest, not `digest_quads_with`.

**Gap:** there is no scope-extraction primitive. `sparq-canon` canonicalizes what it is handed; it
has no notion of a node neighbourhood or an SCC. That extraction rule is Kern's to define (§5).

## 5. The design, conditional on the freeze

### 5.1 Ownership split — what sparq builds and what it must not

| owned by Kern/PSS (do **not** implement) | owned by sparq (this bead) |
|---|---|
| the concept-definition algorithm | a second, independent implementation of it |
| the scope-extraction rule (node/SCC boundary) | executing that rule as specified |
| the record serialization + multihash envelope | a parser + digest re-derivation for it |
| the authoritative positive/adversarial corpus | running against it, plus sparq-authored edge cases |

### 5.2 The ingestion guard

The capability is a **guard**, not a transform. The order is load-bearing:

```text
receive concept record
  → parse the record (reject malformed; no partial acceptance)
  → extract the declared node/SCC scope exactly as the profile specifies
  → canonicalize the scope with sparq-canon primitives (§4)
  → recompute the multihash under the declared code + length
  → BYTE-COMPARE against the digest declared in the record
      ↳ mismatch → Err, and the record is NOT indexed
      ↳ match    → proceed to indexing
```

Non-negotiable properties, each mirroring an existing convention in this repo:

- **Fail-closed, and before indexing.** A mismatch, an unparseable record, an unknown multihash code,
  a declared digest length that disagrees with the code, or a scope the extraction rule cannot
  resolve is an `Err` with **nothing indexed** — never a warning over a completed ingest. This is the
  same discipline as `EmbeddingProvenance`'s `LegacyMode::Reject` and the recall gate in
  `sparq-vectors::dedup`, both of which refuse to emit rather than degrade.
- **Unknown-is-rejection, not acceptance.** Precedent: `Metric::from_tag` returns `None` for an
  unknown tag and is treated as "cannot verify" (`spqv_provenance.rs:85-95`).
- **Opt-in and off by default.** A feature-gated surface, so the default build and the wasm artifact
  are byte-identical without it — the standing rule for `sparq-canon` and `sparq-zk`.
- **Byte comparison, not structural comparison.** Compare the recomputed digest bytes to the declared
  bytes. Never "re-serialize both sides and compare the parse", which would launder a serialization
  disagreement into a false pass. Use a constant-time comparison: the digest is an integrity claim,
  and a length-or-prefix-leaking compare invites an oracle if a record is ever attacker-supplied.
- **The multihash prefix is part of the comparison.** Comparing only the raw digest tail would let a
  record declare a weaker hash code than the one actually used.

### 5.3 Placement — and why probably not inside `sparq-canon`

The routed surface is `sparq-canon`, but the honest recommendation is a **split**, and the maintainer
should rule on it (open question **Q4**):

- **Into `sparq-canon`:** a generic, feature-gated *scope-canonicalization* primitive — canonicalize a
  caller-supplied subset of quads and return its canonical byte image — carrying the §3 contract in
  its own rustdoc: *this is not whole-graph RDFC-1.0; the labels are scope-local*. That is squarely
  canonicalization work and belongs with the other primitives.
- **Out of `sparq-canon`:** the concept-record parser, the multihash envelope, and the ingestion
  guard. These are a partner-specific wire format. Putting them in `sparq-canon` couples a crate
  whose stated purpose is *"RDFC-1.0 … as a small, opt-in public API"* to an external record schema
  and drags in a multihash dependency, and the crate's own Cargo.toml comment defends its leanness
  explicitly. A separate opt-in surface keeps `sparq-canon` a canonicalization crate.

### 5.4 Validation obligations

- The **Kern/PSS authoritative fixtures**, positive and adversarial, vendored and run as a
  conformance suite — the same shape as the vendored W3C suite under
  `crates/sparq-canon/tests/rdf-canon-testdata/`.
- **Independently authored edge cases** (sparq's own — the point of a second implementation is that
  it does not inherit the producer's blind spots). At minimum: an empty scope; a scope that is a
  single isolated node; boundary blank nodes per §3.2; a scope whose blank nodes are isomorphic to
  another scope's; unicode and escaping edge cases in IRIs and literals; language-tagged and
  datatyped literals differing only in case of the language tag; a record whose declared multihash
  code is unknown; a record whose declared length disagrees with its code; a truncated digest; a
  digest that is correct for a *different* scope of the same graph (the substitution attack); and
  RDF-1.2 triple terms in scope, which the standard paths reject via `CanonError::TripleTerm`.
- **A mutation check.** Flip one byte of the expected digest in a positive fixture and confirm the
  test goes red — a verifier whose tests pass against a broken comparison is worse than none.
- **No performance claims** in any accompanying documentation.

## 6. Phased plan (each phase a future bead — none launchable yet)

0. **BLOCKED — the gate.** Confirm on #1746 that the profile and fixture corpus are frozen, and on
   #1683 that the definition algorithm is settled. **Phases 1-5 must not start before this closes.**
1. **Vendor the frozen artifacts.** Bring the profile document and the authoritative positive +
   adversarial fixture corpus into the tree, pinned and attributed, with no interpretation applied.
2. **Scope-canonicalization primitive in `sparq-canon`** (feature-gated, default-off), carrying the
   §3 "not whole-graph RDFC-1.0" contract in its rustdoc and a test that reproduces counterexamples
   A and B so the contract cannot silently rot.
3. **Record parser + multihash re-derivation** on the surface chosen in §5.3, fail-closed on every
   unknown, with the constant-time byte comparison of §5.2.
4. **The ingestion guard** — wire the check *before* indexing, with a mismatch aborting the ingest,
   plus the sparq-authored edge cases and the mutation check of §5.4.
5. **Documentation + honest framing** — the §2 independence caveat and the §3 non-RDFC-1.0 statement
   in the crate README and the relevant `SKILL.md`, and the KERN-boundary notes updated from
   "reserved pending the profile" to the frozen reality.

Phases 2 and 3 are disjoint by crate and can run in parallel once phase 1 lands. Phase 4 depends on
both.

## 7. Open questions for the maintainer / for #1746

- **Q1 (§3.1) — scope-alone or whole-graph labels?** These have incompatible properties; whole-graph
  labels make a concept digest change when unrelated triples are added elsewhere. sparq's reading is
  scope-alone. **This must be explicit in the frozen profile, not left to implementations.**
- **Q2 (§3.2) — is the scope required to be blank-node-closed?** If not, what must a verifier do with
  a boundary blank node? Fail-closed is sparq's default reading.
- **Q3 (§4) — exact hashed byte string.** Trailing newline included? Empty scope: empty-string digest
  or an error? Which multihash codes are permitted?
- **Q4 (§5.3) — placement.** Does the maintainer accept the split (primitive in `sparq-canon`,
  record/guard elsewhere), or should the whole capability live in `sparq-canon`?
- **Q5 (§2) — independence claim wording.** Confirm the caveated phrasing before it reaches any
  README, so the shared-`rdf-canon` limitation is never dropped in summary.

## 8. Related, distinct

`sq-lhcot.2` (external-key `.spqv` profile) is a **different** seam on the same KERN boundary and is
not addressed here. The `EmbeddingProvenance::reserved` area remains opaque with no fields defined;
nothing in this record extends it.

## 9. Estate-fit verdict from the earlier KB evaluation

<!-- [GPT-5.6] sq-y64lh / issue #3124 — this section records the broader
canonicalisation + embedding + ZK + grounding verdict without changing the
freeze-gated implementation plan above. -->

The earlier `sq-y64lh` research issue asked whether sparq should consume this mechanism natively
across the KB estate. The answer is **conditionally yes at the ingestion and identity boundary,
but no as a shared primitive across every content-addressed subsystem**:

- **Canonicalisation:** share the existing `sparq-canon` serialization and scope-canonicalisation
  primitives where the frozen profile permits, but implement node/SCC extraction and cycle handling
  as a distinct algorithm. Section 3 demonstrates why whole-dataset RDFC-1.0 output cannot serve as
  the concept identifier. SCC construction also requires retaining the extracted dependency
  subgraph and canonicalising each component as a unit; blank-node closure must therefore be a
  profile invariant, not an implementation convenience. No credible cycle or blank-node cost can
  be stated until the authoritative corpus fixes the extraction rule and supplies representative
  cyclic fixtures.
- **PKG/vector reuse:** a concept digest is a good *content revision key*, not an embedding identity.
  The reusable key must be at least `(concept multihash, model id, model version, content version,
  verbalisation regime, metric, normalization, dimension)`. Those latter axes already exist in
  `EmbeddingProvenance`; omitting them would let vectors from incompatible coordinate spaces collide
  under the same concept hash. A frozen concept profile could therefore populate a future versioned
  extension of the reserved provenance area, but it must not replace provenance or the `.spqv`
  graph-generation/staleness contract. This seam is worthwhile only after cross-system fixtures
  demonstrate that independent producers derive the same concept digest and embedding input text.
- **ZK commitments:** reuse is limited to canonical RDF observations and algorithm identifiers.
  `sparq-zk` commits an ordered canonical graph representation into domain-specific BN254/Poseidon2
  field elements; a `urn:concept` multihash is a wire identity over a scoped definition. Treating
  either digest as the other would change the committed statement and omit the ZK scheme's
  domain/leaf encoding. A later circuit may bind a concept multihash as an explicitly
  domain-separated public value, but the concept digest must not replace `C(G)` or its registry
  scheme. The ZK estate remains research-grade and lacks the pending external audit.
- **Human labels and NSM grounding:** neither is an identity-layer invariant. A deterministic label
  can be derived only after freezing a lossy presentation policy (language preference, predicate
  priority, lexicalisation, tie-breaking, and version); structural isomorphism alone does not yield
  a uniquely meaningful human label. Likewise, treating a fixed Natural Semantic Metalanguage
  prime set as the sole labelled foundation is a linguistic hypothesis, not evidence supplied by
  content addressing. sparq should permit such annotations as versioned, attributed presentation
  or grounding layers and evaluate them empirically; they must not enter the concept digest or be
  described as canonical semantics without an independently reviewed specification and corpus.

Consequently, early alignment should freeze **one concept-record format and one multihash profile**
with Kern/PSS, then let sparq verify it independently before indexing. Alignment does **not** mean
reusing that digest as an embedding-space identifier or a ZK graph commitment. Until the profile
and fixtures named in §1 land, sparq consumes no native `urn:concept` wire format and reserves no
semantics in `EmbeddingProvenance`.
