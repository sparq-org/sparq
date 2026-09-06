<!-- [OPUS-5] E2EE program reconciliation (bead sq-1vbdy, issue #2677): converges the three parallel E2EE/CRDT research records — #2001 (GPT-5.6 NG design/spec), #2005 (Opus 4.8 Profile-BR privacy + threat model), #2002 (GPT-5.6 SPARQL-CRDT) — into ONE canonical Profile-BR set. Doc-only overlay: it decides which clause wins where the records disagree; it does not re-author them and makes NO cryptographic-soundness claim (sq-qhy4). -->

# E2EE program reconciliation — one canonical Profile-BR set

**Status:** reconciliation record (doc-only; **no implementation**, **no new design**).
Author: Claude Opus 5 `[OPUS-5]`. Date: 2026-07-26. Bead **sq-1vbdy** (issue #2677),
epic **sq-tag1q**.

Three records landed in parallel on the E2EE-queryable, NextGraph-shaped variant. They do
not textually conflict (separate files) but they **do** contradict each other on
mechanism, naming, and one data-model choice. This record is the **canonical overlay**:
for every disagreement it names the clause that wins and why, binds the RDF-CRDT to be
**one artifact** shared by both tracks, and wires the result to the implementation beads.
It **does not** re-author the three records — they stay as their authors wrote them; where
a clause is superseded, that is recorded here and flagged by a banner in the record.

> **Honesty banner (load-bearing).** Nothing here is a cryptographic-soundness or privacy
> claim. Every confidentiality, integrity, authorization, revocation, and convergence
> property named below is **designed/intended**, never proven; none of it has had external
> cryptographic review, and production use stays gated by **sq-qhy4** (P0, open). The
> survey's impossibility statement is unchanged and unchallenged by this reconciliation:
> **general server-side SPARQL evaluation over end-to-end-encrypted data without leakage
> does not exist**, and neither profile claims otherwise — query is always local, after
> decrypt, over materialized plaintext. Reconciling three designs does not make any of them
> sound; it only makes them *one* design instead of three.

---

## 0. Verdict in one page

1. **There is ONE profile, not two.** The canonical profile is **Profile BR
   (broker-relayed)**, framed by [#2005](./e2ee-queryable-nextgraph-variant-2026-07.md).
   The "NG profile" of [#2001](./e2ee-nextgraph-variant-gpt56-2026-07.md) is **not a rival
   profile** — it is Profile BR's **v0 wire realization** (the capability/envelope/epoch/
   broker-message layer). Read them as *framing* + *binding* of one thing. Canonical
   long-form name: **Profile BR**; canonical binding name: **the E2EE-NG binding of
   Profile BR** (the shipped crate `sparq-e2ee-ng` keeps its name).
2. **The tie-break rule** (used throughout §2, stated once): on **framing, threat model,
   adversary strength, and what MUST be disclosed**, #2005 wins — it is the privacy
   authority. On **mechanism**, the clause with the **smaller disclosed leakage surface**
   wins, unless #2005's framing forbids it. That rule is #2005's own §6 principle applied
   to itself, and it happens to select #2001's more conservative v0 mechanisms (randomized
   AEAD, random block IDs, opaque headers) over #2005's NextGraph-parity ones (convergent
   keys, content-addressed IDs, dedup). Where mechanism is already **shipped**, the shipped
   choice is cited as corroboration, never as authority.
3. **One CRDT, one crate — enforced, not aspirational.** The RDF CRDT of Profile BR and
   the SPARQL-CRDT of [#2002](./sparql-crdt-gpt56-2026-07.md) are the **same artifact**:
   the quad-set algebra frozen in `site/specs/sparql-crdt.typ` (sq-tag1q.4) and implemented
   in `crates/sparq-crdt`. Profile BR contributes **no CRDT definition of its own** — it
   contributes the encryption, capability, epoch, and relay envelope *around* it. §3 gives
   the concrete binding contract (identifiers, epoch unification, encoding boundary) that
   makes "same artifact" checkable rather than a slogan.
4. **The consolidated honesty position is unchanged and slightly stricter.** Profile BR
   leaks strictly more metadata than Profile CS (#2005 §6 stands verbatim as the canonical
   comparison), with **two v0 corrections** (§4): dedup/partial-fetch efficiency is *not*
   available in v0, and the persistent CRDT `ReplicaId` is a device-linkable pseudonym
   visible to every read-capability holder — a leak *inside* the trust boundary that
   neither record scored.
5. **Two questions stay open for the maintainer** (§6): the relay binding (Solid-pod vs
   dedicated broker) and whether a revocation default is ever declared. Everything else in
   #2005 §7's open-question list is resolved here on technical grounds.

---

## 1. Authority map — which record owns what

| Layer | Canonical source | Role |
|---|---|---|
| Option space, leakage tiers T0–T4, what is specifiable at all | [`e2ee-queryable-options.md`](./e2ee-queryable-options.md) (survey, sq-tag1q.3) | Defines Profile CS (mandatory core) + Profile SE; gates the spec bead |
| Profile framing, threat model, adversary strengths, leakage comparison, mandatory disclosure | [#2005](./e2ee-queryable-nextgraph-variant-2026-07.md) `[OPUS-4.8]` | **Privacy authority.** §5 threat model + §6 leakage table + BR-1…BR-9 are canonical |
| Wire realization: capability/envelope/epoch formats, broker messages, client API, disclosure ledger | [#2001](./e2ee-nextgraph-variant-gpt56-2026-07.md) `[GPT-5.6]` | **Binding authority.** §4, §5, §6, §8 are canonical *as the v0 binding of Profile BR* |
| RDF CRDT algebra, SPARQL-Update compilation, delta interchange | [#2002](./sparql-crdt-gpt56-2026-07.md) `[GPT-5.6]` → frozen in `site/specs/sparql-crdt.typ` (sq-tag1q.4) | **CRDT authority.** Where record and frozen spec differ, **the spec wins** |
| Shipped primitives | `crates/sparq-e2ee-ng` (sq-tag1q.9), `crates/sparq-crdt` (sq-tag1q.7.2) | Corroboration only — code is not authority for a design decision |

Nothing in this table promotes an unaudited artifact: every layer above is research-grade
and sq-qhy4-gated.

---

## 2. Contradiction ledger

Each row is a real disagreement between two of the three records. "Canonical" is the
clause that wins; the losing clause is **superseded on that point only** and its record is
otherwise intact.

| # | Point | #2005 (Profile BR) | #2001 (NG binding) | **Canonical resolution** |
|---|---|---|---|---|
| 1 | Profile name | "Profile BR", a second profile | "NG profile", a second profile | **Profile BR** is the profile; NG is its v0 binding. There is no separate NG profile. (§0.1) |
| 2 | Block/object key derivation | Store-scoped **convergent** keys → dedup; disclose equal-plaintext linkability (§4.2, BR-3) | **Randomized** AEAD; dedup deliberately sacrificed pending sq-qhy4 (§2) | **#2001.** Randomized in v0 — smaller leakage surface (no within-store plaintext-equality signal). Convergent keying is a **future opt-in, leakage-disclosed extension** requiring its own sq-qhy4 review and a BR-8 statement amendment. Corroborated by shipped `envelope::seal_block_random`. |
| 3 | Block identifiers | BLAKE3 digest of the encrypted Merkle root (content-addressed) | **Random** 256-bit, authenticated inside the encrypted parent; MUST NOT be a plaintext hash (§4.1) | **#2001**, for the same reason as row 2: content-addressed IDs re-introduce the equality signal that randomization removes. |
| 4 | Object structure | Merkle tree of blocks, internal nodes carry child block keys (NextGraph parity) | Chunked object; `(object_id, chunk_index, chunk_count)` bound in AEAD associated data (§8.3) | **#2001.** Visible Merkle child identifiers are a graph-shape signal; AAD-bound chunk positions give the same integrity property without it. |
| 5 | Read-capability granularity | Read-cap = `(objectId, objectKey)`; key travels in the reference; "read can be scoped by block or by branch" (§4.1) | Read-cap = `{RepoId, BranchId, Epoch, TopicId, K_read, locators, constraints}`; per-object keys **derived** from `K_read` by a domain-separated KDF (§4.2, §8.2) | **#2001.** Consequence, recorded plainly: **sharing granularity is per-branch-per-epoch, not per-object.** To share a subset of a dataset you create a separate branch. #2005's block-scoped sharing is superseded. |
| 6 | Capability authority levels | Two: read-cap / write-cap (§4.1) | Three: **read / publish / admin**, with `sk_publish` and `sk_admin` never combined (§4.2) | **#2001.** Strictly finer separation; #2005's write-cap conflates publish and admin. Corroborated by the shipped strict three-way separation. |
| 7 | Commit header visibility | Headers **SHOULD** be encrypted objects (§4.2, BR-2) | v0 default is **`opaque-header`**; clear routing headers are an explicit opt-in that reveals the commit DAG (§5) | **Both, tightened:** opaque-header is the **default and the conforming mode**; a clear-header deployment is opt-in and **MUST** amend its BR-8 leakage statement to disclose the exposed DAG. `SHOULD` in BR-2 reads as `MUST` for a default-conforming deployment. |
| 8 | Revocation option names | `BR-history` / `BR-reencrypt` (§4.4) | `forward-only` / `history-rekeyed` (§4.2) | **#2001's tokens** — they are the ones frozen in the shipped `HistoryPolicy` and carried in the signed epoch transition. #2005's BR-7 normative shape (MUST declare exactly one; MUST NOT call either "forward secrecy" or a post-compromise guarantee) is canonical **verbatim**. |
| 9 | CRDT element | Add-wins OR-Set of **triples** (§0, §4.3) | Observed-remove **quad** set, `or-set-quads-v0` (§4.1) | **Quads** (#2001, #2002, and the frozen spec agree). Named-graph membership is part of element identity. #2005's "triples" is superseded. This also resolves #2005 §7 Q5: a branch replicates a whole **dataset**, not one named graph. |
| 10 | CRDT identity token | (none) | `"or-set-quads-v0"` placeholder (§8.3) | **`sparq-crdt-delta/1`** — the format token frozen by `CRDT-WIRE-2`. A placeholder token in the E2EE binding would create a second CRDT identity, which is exactly the duplication this record forbids. |
| 11 | Crate names | `sparq-e2ee-sync` layered on a `sparq-e2ee` envelope crate (§0) | `sparq-e2ee-ng`, modules `capability/envelope/repo/crdt/sync/broker_protocol/materialize`; separate opt-in relay that MUST NOT link the query engine (§7) | **#2001.** `sparq-e2ee-ng` (shipped) is the client-side crate; the relay is a separate opt-in crate/binary. The names `sparq-e2ee` and `sparq-e2ee-sync` are retired. The `crdt` module of #2001 §7 is an **adapter over `sparq-crdt`**, not a CRDT implementation (§3). |
| 12 | Relay binding | Solid-pod-as-relay **first**, dedicated broker later (§4.5) | A concrete dedicated broker message set (§8.4) | **Layered, decision deferred (§6 Q1).** #2001 §8.4 is the normative **dedicated-relay binding**; a Solid-pod binding is a second binding of the *same abstract block+pub/sub contract*. Both records already agree the contract must be abstract; nothing shipped forecloses either. |
| 13 | Result freshness | Not required | Every result **MUST** be labelled `(repo_id, branch_id, epoch, frontier)`; "current" means current at that frontier (§6) | **#2001**, adopted as a Profile BR requirement. It is an honesty requirement, not a mechanism, so it strengthens #2005's framing rather than competing with it. |
| 14 | Identity anchor | WebID-anchored capability keys; explicitly **not** `did:ng` (§4.1) | Capabilities are bearer secrets, recipient-wrapped; no DID method adopted (§4.2) | **Both, layered:** capabilities are bearer secrets (#2001); the **recipient wrapping targets a WebID-associated key** (#2005). Neither record adopts `did:ng`. #2005 §7 Q3's sub-question — *where* the recipient key lives in the WebID profile — remains open and belongs to the Solid binding bead, not here. |
| 15 | Cipher / suite | ChaCha20 (reported NextGraph fact), "an AEAD from the registry" | Algorithm **agility** mandatory; v0 suite names are placeholders; a deployment MUST bind one reviewed suite and MUST NOT silently substitute (§8.1) | **#2001.** Suite selection is a sq-qhy4 deliverable. A NextGraph *fact* is not a sparq *choice*, and #2005 never presented it as one. |
| 16 | Threat-model pointer | — | §"Honesty and audit boundary" points at `e2ee-privacy-threat-model-2026-07.md` | **Broken link** — no such file was ever written. The privacy/threat-model record it defers to **is** #2005. Fixed in place by this bead. |
| 17 | Efficiency claim | §6 last row credits BR with dedup + partial fetch ("the efficiency *is* the leak") | v0 sacrifices dedup (§2) | **Corrected for v0** (§4): partial fetch and incremental commit sync remain; **dedup does not exist in v0**, so that half of the row describes a future extension, not the v0 profile. |
| 18 | CRDT scope | — | — | #2002 §5.2 maps `COPY`/`MOVE`/`ADD` to concrete deltas, but the frozen spec (`CRDT-SCOPE`) puts them **outside** the profile and requires rejection. **The frozen spec wins**; #2002 §5.2's three rows are superseded. |

Rows 2–4 have a shared consequence worth stating once: **v0 buys lower leakage by giving
up deduplication.** That is a deliberate, disclosed trade, not an oversight, and it is the
single largest divergence from NextGraph's documented design.

---

## 3. One CRDT, one artifact — the binding contract

Record #2005 flagged the requirement ("the two designs must share one CRDT"); this section
makes it checkable. Profile BR's commit bodies carry **SPARQL-CRDT deltas** exactly as frozen by
sq-tag1q.4. The binding is:

- **B1 — No second CRDT.** The E2EE-NG binding **MUST NOT** define, name, or version an
  RDF CRDT. The `crdt` module of #2001 §7 is an *adapter*: it hands `sparq-crdt` bytes and
  receives bytes back. Any add/remove/merge semantics in the E2EE layer is a defect.
- **B2 — Replication domain.** One Profile-BR **branch** is one CRDT **replication
  domain**: `DatasetId` ↔ `(RepoId, BranchId)`. A dataset (default graph + all named
  graphs) lives in one branch, because graph membership is part of quad identity (row 9).
- **B3 — One epoch, not two.** The E2EE `Epoch` (key/membership rotation, #2001 §4.1) and
  the CRDT `membership_epoch` (#2002 §4.3, `CRDT-WIRE-4`) are **the same integer** for that
  branch. This is forced, not cosmetic: `CRDT-WIRE-4` rejects an envelope whose epoch is
  not the receiver's current membership epoch, and a Profile-BR epoch transition *is* a
  membership change. An implementation that keeps two counters will silently reject valid
  deltas after any rotation.
- **B4 — Encoding boundary.** The CRDT wire form is canonical JSON (JCS, `CRDT-WIRE-2`);
  the E2EE binding's own structures are deterministic CBOR (#2001 §8.1). These do **not**
  conflict, because the delta is **opaque payload** to the E2EE layer: the exact JCS bytes
  are what gets padded, sealed, and bound in the AEAD associated data. The E2EE layer
  **MUST NOT** re-encode, re-order, or re-canonicalize a delta — #2002 §6.1's rule (never
  authenticate an ambiguous alternate encoding) and `CRDT-WIRE-3`'s reject-don't-normalize
  rule both depend on exactly one byte string per envelope identity.
- **B5 — Key material never enters the algebra.** `ReplicaId`, `Dot`, and `DatasetId`
  **MUST NOT** be derived from `K_read`, a publisher key, or an admin key (#2002 §9). Key
  rotation must not rewrite CRDT history; an epoch transition re-keys transport and
  storage, and leaves dots untouched.
- **B6 — Routing must not carry CRDT identifiers.** The CRDT `dataset` field is an
  IRI-shaped stable identifier that sits **inside** the ciphertext. It **MUST NOT** be used
  as, or derived into, a clear routing key: doing so would re-link epoch-rotated topics to
  one stable dataset and undo the unlinkability #2001 §5 claims for `RepoId`/`BranchId`.
- **B7 — Full-payload encryption.** The entire delta — adds, removes, causal context,
  origin id, sequence — is inside the AEAD envelope (#2002 §9). Only the routing fields the
  relay strictly needs stay clear, and those are enumerated by #2001 §5, not chosen ad hoc.
- **B8 — Receivers never re-evaluate.** #2002's evaluate-at-origin rule survives
  encryption unchanged: a decrypting replica applies concrete deltas and **never**
  re-evaluates a `WHERE` clause. Combined with B4 this is what makes convergence
  independent of the relay's delivery order.

Crate shape implied by B1–B8: `sparq-crdt` (algebra + codec + journal) ← `sparq-e2ee-ng`
(capability/envelope/epoch, and later sync/materialize) → `sparq-core` for materialization.
No cipher in `sparq-core`/`sparq-engine`/`sparq-substrate`; the relay links neither the
engine nor the CRDT algebra. Both records already required exactly this.

---

## 4. Consolidated honesty position

Record #2005 §6 (the CS-vs-BR leakage table) and #2005 BR-8 (the mandatory leakage statement)
stand as canonical. #2001 §5 is the **per-message ledger** from which a deployment's BR-8
statement is generated — the two are layered, not competing. Three amendments:

- **A1 — v0 has no deduplication** (ledger rows 2, 17). #2005 §6's "server-side
  selectivity / cost" row describes dedup + partial fetch; for v0, **only partial fetch and
  incremental commit sync apply**. The row's honest core — that partial-fetch selectivity
  is itself an access-pattern signal — is unaffected.
- **A2 — no within-store plaintext-equality signal in v0.** Randomized block encryption
  removes the equal-plaintext linkability that #2005 BR-3 required a deployment to
  disclose. BR-3's disclosure duty **re-attaches** if convergent keying is ever added.
- **A3 — new, unscored: `ReplicaId` is a persistent pseudonym inside the trust boundary.**
  The CRDT envelope carries a stable per-replica identifier and monotonic counters. These
  are hidden from the relay (B7), but they are visible to **every read-capability holder of
  the branch**, and because Profile BR provides **no forward secrecy by design**, that
  includes **future joiners reading history**. So a joiner can reconstruct per-device
  authorship and activity ordering over the whole pre-join history. Neither #2005 §6 (which
  scores the *broker*) nor #2001 §5 (which scores the *messages*) covers this, because it
  is a leak to an *authorized* party. It is a consequence of two accepted choices — no PFS,
  and dots-not-derived-from-keys — not a defect to fix here. Profile BR's BR-8 statement
  **MUST** name it. Rotating `ReplicaId` at an epoch transition is a plausible mitigation
  and is explicitly **not** specified here: it interacts with #2002 §4.3's causal-stability
  and garbage-collection rules and needs its own analysis.

Unchanged and restated, because reconciliation is exactly where overclaim creeps in: no
forward secrecy, no post-compromise security, no soundness claim, no server-side query over
ciphertext, no external audit — all of it sq-qhy4-gated, all of it designed-not-proven.

---

## 5. Wiring to the implementation beads

| Bead | Surface | What this reconciliation binds it to |
|---|---|---|
| **sq-tag1q.5** | `site/specs/e2ee-sparql.typ` — the E2EE-SPARQL spec draft (gated by the survey; **not yet written**) | Specify **one** optional profile, **Profile BR**, alongside the survey's mandatory Profile CS. Carry #2005 BR-1…BR-9 as the normative clauses, **amended by** ledger rows 2–8, 10, 13 and §4 A1–A3. Use #2001 §8 as the v0 binding annex. Keep the ZK/MPC composition informative-only with the sq-qhy4 caveat verbatim. Reference `sparq-crdt-delta/1` for the CRDT — do not restate the algebra. |
| **sq-tag1q.4** *(landed)* | `site/specs/sparql-crdt.typ` | Unchanged and authoritative for the CRDT. Ledger row 18 records that its `CRDT-SCOPE` supersedes #2002 §5.2 on `COPY`/`MOVE`/`ADD`. |
| **sq-tag1q.9** *(landed)* | `crates/sparq-e2ee-ng` | Corroborates rows 2, 3, 6, 8. No change required. When its sync layer lands it inherits B1–B8. |
| **sq-tag1q.7.x** *(partly landed)* | `crates/sparq-crdt` | The single CRDT artifact (B1). Its epoch field is the epoch of B3; its JCS bytes are the opaque payload of B4. |
| **sq-tag1q.14 / .16 / .17** | Not resolvable from this checkout — no title for these ids exists in the tree or in git history | Whichever of {sync + broker protocol, materialization adapter, capability/sharing UX} they cover, each inherits **B1–B8** and the ledger row for its surface, and **MUST NOT** introduce a second CRDT, a second epoch counter, a second delta encoding, or a convergent-keying default. Their titles must be confirmed against the live bead graph before work starts — this record deliberately does not guess them. |

Every bead above inherits the same hard boundary: opt-in crates, lean core, no cipher in
`sparq-core`/`sparq-engine`/`sparq-substrate`, and no claim that is not caveated as
research-grade and externally unaudited (sq-qhy4).

---

## 6. Still open for the maintainer

Resolved here on technical grounds, and therefore **closed**: #2005 §7 Q4 (shared CRDT —
§3), Q5 (named-graph mapping — ledger row 9 + B2), Q6 (BR × Profile SE composition — #2002
§9 shows SE needs a stable keyed term identity or a decrypt-and-rewrite migration before
any compatibility claim, so the composition stays **unspecified and discouraged**, exactly
as #2005 preferred), and the mechanism half of Q3 (identity — ledger row 14).

Genuinely open, needing a maintainer decision:

1. **Relay binding (ledger row 12).** Solid-pod-as-relay first (reuse, lean, pod host is
   the metadata observer) versus a dedicated relay implementing #2001 §8.4? #2005
   recommends Solid-first. Nothing shipped forecloses either, and the abstract contract is
   agreed either way.
2. **Revocation default (ledger row 8).** `forward-only` and `history-rekeyed` are both
   implemented as a declared, signed, per-transition field — i.e. the spec currently has
   **no default**, which is a defensible answer in itself. Confirm "no default, always
   declared", or pick one.
3. **`ReplicaId` rotation (§4 A3).** Accept the disclosed pseudonym-to-joiners leak, or
   open a bead to analyze rotation against causal stability and garbage collection?
4. **Bead titles for sq-tag1q.14 / .16 / .17** (§5) — confirm against the live bead graph.

---

## Cross-links

- Survey / option space: [`e2ee-queryable-options.md`](./e2ee-queryable-options.md) (sq-tag1q.3)
- Privacy authority (#2005): [`e2ee-queryable-nextgraph-variant-2026-07.md`](./e2ee-queryable-nextgraph-variant-2026-07.md)
- Binding authority (#2001): [`e2ee-nextgraph-variant-gpt56-2026-07.md`](./e2ee-nextgraph-variant-gpt56-2026-07.md)
- CRDT authority (#2002): [`sparql-crdt-gpt56-2026-07.md`](./sparql-crdt-gpt56-2026-07.md), frozen in `site/specs/sparql-crdt.typ`
- Shipped: `crates/sparq-e2ee-ng` + [`skills/e2ee-ng/SKILL.md`](../skills/e2ee-ng/SKILL.md), `crates/sparq-crdt`
- Adjacent estate: [`threat-model.md`](./threat-model.md), [`crypto-erase-at-rest.md`](./crypto-erase-at-rest.md), [`zk-audit-readiness-dossier.md`](./zk-audit-readiness-dossier.md), [`sparq-solid-scope.md`](./sparq-solid-scope.md)
