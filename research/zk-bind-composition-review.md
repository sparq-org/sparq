# ZK verifier `bind_*` composition review — search for a satisfying-but-false aggregate manifest

Bead: **sq-1s2.6** (epic sq-1s2). Reviewer: **Claude Opus 4.8** `[OPUS-4.8]` — dispatched as
Fable but downgraded to Opus 4.8 mid-run by the dual-use safety classifier (only ~2 of ~88 turns
ran on Fable before the downgrade on the adversarial-crypto content); see the Fable-safeguard note.
Date: 2026-07-01.

> **THIS REVIEW DOES NOT REPLACE THE EXTERNAL AUDIT (sq-qhy4).** It is a single-model
> adversarial *composition* read of the `bind_*` obligation set in
> `crates/sparq-zk-compose/src/verifier.rs`. All existing not-production-sound honesty
> labels stay in force verbatim: the whole ZK estate is remediated but **NOT** externally
> audited; no result here is a production-grade guarantee until an accredited cryptographer
> signs off (sq-qhy4, P0, OPEN). "BLOCKED-BY" verdicts below are *code-reading* verdicts, not
> proofs; several depend on `#[ignore]`d crypto-chain tests (nargo/bb) that were not executed.

## Scope and method

Prior review closed each obligation *individually*. This pass holds the whole set
simultaneously and asks: **can an adversary build an aggregate manifest that satisfies every
individual `bind_*` obligation yet binds to different data than claimed?** Method: (1) enumerate
every obligation with file:line and the exact fields it binds; (2) build the composition matrix
(for each shared field, is it bound by BOTH sides or only assumed by one?); (3) attempt
constructions — field-reordering, index aliasing, attribution-bit boundary, cross-manifest
splicing, stale-component replay, single-reference multi-credential; (4) verdict each attempt
BLOCKED-BY (obligation, file:line) or POTENTIAL-BREAK (exact construction).

## The obligation set (12 `bind_*` + 1 helper + orchestrator)

All in `crates/sparq-zk-compose/src/verifier.rs`. Orchestrator = `verify_manifest` (4578).
Structural prefilter = `prefilter_manifest_structure` (2003), which runs stages 1a/1b/2 then
calls `bind_query_correctness`, `bind_attributions`, `bind_issuer_attestations`,
`bind_revocation`, `bind_joins`. The bb (crypto) stage runs the per-sub-proof
`reconstruct_public_inputs` (4786) loop then `bind_hidden_revocation`,
`bind_hidden_issuer_attestations`, `bind_holder_pok`, `bind_holder_set`. `bind_holder_pop`
(→`bind_holder_binding`→`verify_holder_attestation_signature`) runs between the nonce burn and
the sub-proof loop. `bind_entailment` runs first (pure JSON).

| # | Obligation | file:line | Binds (fields) | Trust anchor |
|---|---|---|---|---|
| 1 | `bind_issuer_attestations` | 2242 | each scan `commitments[g]` → issuer Schnorr sig over `commitment_message_with_status/holder(C,salt,status_ref[,holder_digest])`; salt-uniqueness over referenced set | external `trusted_key_set` K; `manifest.revocation` (single) |
| 2 | `bind_revocation` | 2722 | `manifest.revocation` (single) liveness: freshness + `authoritative.bit(index)==0` OR requires hidden proof | RP `RevocationPolicy` authoritative snapshot |
| 3 | `bind_hidden_revocation` | 2871 | `hidden_revocation` proof: public `root`==RP authoritative Merkle root, public `index_commitment`==issuer-signed `rev.index_commitment`, in-circuit `bit==0` | RP snapshot root; issuer-signed index commitment |
| 4 | `bind_hidden_issuer_attestations` | 3034 | each entry: public `m`==recomputed `commitment_message_with_status(C,salt,status_ref)`, public `key_set_root`==RP KeySet root | RP `KeySet` root; `manifest.revocation` (single) for `status_ref` |
| 5 | `bind_holder_pok` | 3190 | in-circuit PoK: public `holder_pk_digest`==issuer-attested digest (via `verify_holder_attestation_signature`), nonce | external K; covering attestation; nonce |
| 6 | `bind_holder_set` | 3377 | set-membership: public `holder_set_root`==RP HolderRegistry root; commitment referenced by a scan | RP `HolderRegistry` root |
| 7 | `bind_query_correctness` | 3633 | query BGP consts → scan `pattern_const_enc`; query FILTER (op,bound,slot,verdict) → `filter_int` edge | `manifest.query` (RP-read) |
| 8 | `bind_joins` | 3790 | each `JoinEdge`: `join_eq` public `commit_a/b`==scan `commitments[graph_*]`; `slot_a/b`==query-derived shared-var slots; N-way `join_commitment` chain equality | referenced scan commitments; query slots |
| 9 | `bind_attributions` | 3981 | `manifest.attributions[pi]` ⊇ answering scan's proof-bound `attribution[g]` bits | scan proof-bound `attribution` (per-scan-local g) |
| 10 | `bind_holder_pop` | 4082 | Schnorr PoP over `holder_pop_message(challenge)=Poseidon2([ZKSIG_HP,challenge])`; holder∈registry; → `bind_holder_binding` | RP `HolderRegistry`; nonce |
| 11 | `bind_holder_binding` | 4189 | presented `pk` digest == covering attestation's issuer-signed `holder_pk_digest` | external K; covering attestation |
| 12 | `bind_entailment` | 4433 | `entailment_regime` accepted by policy; each derivation step well-formed + antecedents grounded to disclosed base / earlier step | RP `EntailmentPolicy`; disclosed scan rows |
| — | `verify_holder_attestation_signature` | 4330 | (helper) issuer sig over `commitment_message_with_holder` verifies under K; recomputes `status_ref` from `manifest.revocation` | external K; `manifest.revocation` (single) |

The single load-bearing seam every obligation ultimately rides is
`reconstruct_public_inputs` (audit #1: byte-equality of the DECLARED `ProofInputs` against the
proof's `public_inputs`, with the **verifier's own nonce** as public-input field 0) + the
canonical vk recompute (audit #2). Every "bound" field above becomes a lie iff it is not
byte-included in that reconstruction; the re-audit (`research/zk-verifier-reaudit.md` §1)
confirmed no omitted/extra/reordered `pub`. This composition review takes that per-circuit
field-order result as given and looks at *cross-obligation* consistency instead.

## Composition matrix — shared fields and who binds them

| Shared field | Obligations that read it | Bound by BOTH sides? |
|---|---|---|
| verifier nonce (challenge, PI field 0) | 1–12, every sub-proof, PoP | YES — `reconstruct_public_inputs` feeds the RP nonce into every sub-proof; PoP message binds it; single-use `seen` store. Cross-manifest/stale splice closed. |
| scan `commitments[g]` | 1,4,5,6,8,9,11 | YES — byte-bound by reconstruction; all consumers compare as field elements (`to_field()`), 0x-padding-insensitive. |
| `commitment_attestations[*]` ↔ commitment | 1,5,11 (covering lookup) | YES — identical `a.commitment.to_field()==c_field` lookup in all three. |
| issuer-signed `status_ref` | 1,2,3,4,5,11 | **PARTIAL** — derived from the **single** `manifest.revocation`; see §Finding B. |
| `attribution[g]` (per-graph bit) | 9 (superset), reconstruction (bind) | **PARTIAL** — bound to proof as scan-**local** index; Q6 consumer treats it as **global**; see §Finding A. |
| `holder_pk_digest` | 1,5,11, `verify_holder_attestation_signature` | YES — anchored in issuer sig (ZKSIG_C4) under K in every path; presented `pk` reused between PoP verify and digest check. |
| `join_commitment` / slot_a/b | 8 | YES — byte-bound; slots made public; N-way chain equality enforced. |

## Attack attempts and verdicts

### 1. Field / parameter reordering (public-input layout)
Reorder a scan's `attribution` before `row_count`, or swap `commit_a`/`commit_b`.
**BLOCKED-BY** `reconstruct_public_inputs` (verifier.rs:4786) — the reconstruction emits fields
in each member's fixed `main` declaration order and byte-compares; any reorder yields a vector
that cannot byte-match the proof (re-audit §1 confirmed the order per circuit). Not re-litigated.

### 2. Attribution-bit boundary / off-by-one (the bead's flagged verifier.rs:3981)
Under-length or omit `attribution` to make `bind_attributions` inspect fewer bits.
**BLOCKED-BY** stage-1b `prefilter_manifest_structure` (verifier.rs:2052): rejects
`attribution.len() != CircuitId.k` (`AttributionMalformed`) with no `serde(default)` short/pad —
so `bind_attributions` (4004) always iterates exactly k bits and `reconstruct_public_inputs`
(4876) binds exactly k bits. The `unwrap_or(false)` at 4876 is unreachable (belt-and-braces).
No off-by-one: both loops are 0-based over the same k. **BLOCKED.**

### 3. Attribution graph-index ALIASING across scans — **POTENTIAL-BREAK-CANDIDATE, BLOCKED-BY salt-separation** (Finding A)
See §Finding A. Verdict: **no false-accept** because audit #9 salt-uniqueness independently
blocks the only exploitable payload (cross-graph bnode correlation). But the Q6 non-bnode
obligation gate is **inert** for cross-scan joins — a real composition weakness. → **P2 bead sq-en5dx**.

### 4. Cross-manifest splicing / stale-component replay
Splice a scan sub-proof (or a `join_eq`, `hidden_issuer`, `holder_pok` proof) proved in an
earlier session into a fresh manifest.
**BLOCKED-BY** the nonce discipline (`verify_manifest` 4634–4685): every sub-proof's
public-input field 0 is reconstructed with the **current** verifier nonce; a stale component
committed a different nonce → byte-compare fails (`PublicInputMismatch`). The nonce is also
recorded single-use *before* the crypto gate (burn-on-mismatch, sq-3v2), so re-presenting the
whole manifest under its original nonce is `NonceReplay`. The hidden gates (3,4,5,6) each feed
`&challenge` (the nonce) into their own reconstruction. **BLOCKED.**

### 5. Single-reference multi-credential smuggle (revoked credential via a join) — **BLOCKED, fail-closed** (Finding B)
Present credential A (live, index 5) and credential B (REVOKED, index 9) both scan-covering,
joined, hoping B's liveness goes unchecked because `manifest.revocation` is a single field.
**BLOCKED-BY** `resolve_status_ref` (verifier.rs:2563) inside `bind_issuer_attestations`: EVERY
scan-covering commitment's issuer-signed `att_status` must byte-match the ONE
`manifest.revocation` (index+version), so A and B cannot both be attested (A forces rev.index=5,
B forces rev.index=9). Setting rev to B's index makes `bind_revocation` read
`authoritative.bit(9)==SET` → `CredentialRevoked`. No false-accept. **BLOCKED** — but it exposes
a functional/latent-soundness concern; see §Finding B. → **P3 bead sq-cuvmj**.

### 6. Holder-PoP field-ordering / A-presents-B
Holder A (trusted key) presents B's credential; or reorder the PoP message fields.
**BLOCKED-BY** the composition of `bind_holder_pop` (4082, PoP over
`Poseidon2([ZKSIG_HP,challenge])` under presented `pk`) and `bind_holder_binding` (4189):
the **same** `pk` verified in the PoP is required to satisfy
`holder_key_digest(pk)==issuer-attested digest`, and that digest is anchored in the issuer's
EUF-CMA signature over `commitment_message_with_holder` under external K
(`verify_holder_attestation_signature` 4330). A's digest ≠ B's attested digest →
`HolderKeyMismatch`. The covering-attestation scoping (4227) prevents satisfying the check with
an unrelated sibling attestation. **BLOCKED.**

### 7. Hidden-revocation index re-point
Prove `bit==0` against a different (unrevoked) index than the credential's.
**BLOCKED-BY** `bind_hidden_revocation` (2871): the proof's public `index_commitment` must
byte-equal the **issuer-signed** `rev.index_commitment` (2937–2948), and the public `root` must
equal the RP's authoritative root (2914–2921); the in-circuit cross-binding ties the proven-unset
index to that commitment. Re-pointing needs a fresh issuer signature. **BLOCKED.**

### 8. Hidden-issuer message / key-set forge
Prove membership in a prover-forged key set, or bind `m` to a commitment no scan uses.
**BLOCKED-BY** `bind_hidden_issuer_attestations` (3034): public `key_set_root` must equal the
RP-derived root; `m` is recomputed by the verifier (never `hi.message`) via
`scan_referenced_messages` and must match a **scan-referenced** commitment's message
(`HiddenIssuerUnreferencedCommitment`/`HiddenIssuerMessageMismatch`). NOTE: the M-1
challenge-reduction wrap defect (`research/zk-membership-pok-reaudit.md` §3) is an
**in-circuit** hazard, out of scope of this host-composition read; recorded as already fixed
(`reduction_range_bind`) but unverified here. **BLOCKED (host composition).**

### 9. Join slot / multi-scan mis-binding
Order `sub_proofs` so a `join_eq`'s slot binding validates against a first-match scan ≠ the one
the edge references; or chain N-way joins with divergent `join_commitment`s.
**BLOCKED-BY** `bind_joins` (3790): `pattern_answered_by_scan` binds against the SPECIFIC
`edge.scan_a`/`edge.scan_b` (membership, not first-match — sq-sfsi comment 3811); `commit_a/b`
byte-match those scans' commitments; N-way chain requires byte-equal `join_commitment` per
shared variable (3922). **BLOCKED.** (join_eq soundness itself is opt-in / not-yet-sound.)

### 10. Entailment ungrounded-antecedent / regime swap
Claim `Rdfs` derivations whose antecedents aren't in the disclosed base.
**BLOCKED-BY** `bind_entailment` (4433): regime must be policy-accepted; every step well-formed +
regime-admitted; every antecedent grounded to a disclosed scan row or a strictly-earlier derived
triple (forward-chain only). Honest scope: it does NOT prove ZK closure over undisclosed
antecedents — those are rejected, not assumed (documented). **BLOCKED (over the disclosed base).**

## Finding A — attribution graph-index aliasing (POTENTIAL-BREAK-CANDIDATE, BLOCKED-BY salt-separation)

**The seam.** `attribution[g]` in a scan sub-proof is an index into **that scan's own**
`commitments` vector (build.rs:313–326 sweeps `enc_fr` — the per-scan graph list — pushing one
bit per local graph). `bind_attributions` (verifier.rs:4004) and `reconstruct_public_inputs`
(4876) both use this **scan-local** g. But the Q6 consumer,
`sparq_zk::verify::cross_graph_join_obligations` (crates/sparq-zk/src/verify.rs:486–515), treats
`manifest.attributions[pi]` as a set of **globally-distinct graph identities**:
`cross_possible = attributions[i].union(&attributions[j]).count() > 1` — two patterns are
"same graph, bnode-join OK, no obligation" iff their index sets collapse to one element.

**The construction.** Present two DISTINCT credentials as two separate single-commitment
(`k=1`) scans:
- Scan A: `commitments=[C_X]`, `attribution=[true]` (local graph 0 = graph X), answers pattern 0.
- Scan B: `commitments=[C_Y]`, `attribution=[true]` (local graph 0 = graph Y), answers pattern 1.

The minimal attribution declaration that satisfies `bind_attributions` is
`attributions = [[0],[0]]` (each `{0}` ⊇ its scan's `{0}`). `cross_graph_join_obligations` then
computes `{0} ∪ {0}`, count 1 → **NOT cross_possible** → drops the non-bnode obligation for a
join on a variable shared between pattern 0 (graph X) and pattern 1 (graph Y). `bind_attributions`
cannot catch this: `{0} ⊇ {0}` holds; the under-declaration guard only bites WITHIN one scan (a
`k=2` scan declaring `[[0],[0]]` while `attribution=[true,true]`). So **the Q6 non-bnode
obligation gate is inert for cross-scan (separate-`k=1`-scan) joins.**

**Why it is nonetheless not a false-accept (BLOCKED-BY).** The only payload the dropped
obligation would enable is a *cross-graph bnode correlation* (claiming graph X's bnode == graph
Y's bnode to satisfy a join the two credentials can't legitimately support). Two independent
gates block the payload:
1. **Salt-uniqueness (audit #9):** `bind_issuer_attestations` (verifier.rs:2466–2492,
   `SaltReused`) rejects two distinct scan-referenced commitments sharing an issuer-attested
   salt. X and Y being distinct commitments ⇒ distinct salts.
2. **Proof-bound per-graph encoding:** a bnode encodes as `f(label, salt)` (build.rs:283–285),
   and each scan's disclosed rows are proof-bound to *its* graph's salt. So the same bnode label
   in X and Y discloses **different** field encodings; the cross-graph join value-equality (which
   the relying party computes from the disclosed encodings) is empty. Non-bnode (IRI/literal)
   values are salt-independent, so their cross-graph joins are legitimate and value-checkable.

So the aliasing drops a gate that turns out to be **redundant with salt-separation**; the live
backstop is audit #9, not Q6. Verdict: **no exploitable break at present.**

**Why it still matters (fragility).** The `bind_attributions`/reconstruct layer and the Q6 gate
disagree on the graph-index namespace (scan-local vs global). The Q6 machinery *appears* to
defend cross-scan bnode joins but does not — it is dead code for that case. Any future change that
(a) introduces a shared-salt optimization, (b) relaxes `SaltReused` scoping, or (c) removes/weakens
salt-separation believing Q6 covers it, silently re-opens the audit-#8/#9 cross-graph
bnode-correlation class. The `attributions[pi]` field doc (manifest.rs:1157) says "committed graph
indices" as if a global namespace exists — there is none; each scan owns its `commitments`.

**Recommended hardening (P2 bead sq-en5dx).** Either (i) key the Q6 gate on
globally-distinct graph identities — the canonical commitment hex of `commitments[g]` for the
answering scan — instead of the scan-local integer, so two distinct graphs never collapse; or
(ii) explicitly document salt-separation as the *primary* Q6 defense and the non-bnode obligation
as advisory-only, and add a cross-scan aliasing regression test asserting the current
salt-separation backstop. Option (i) restores the obligation's intended teeth and removes the
namespace ambiguity.

> **RESOLVED (sq-en5dx, Option (i)) `[OPUS-4.8]`.** `prefilter_manifest_structure` now feeds the
> Q6 gate a GLOBAL-namespace attribution vector via `global_attributions` (verifier.rs), which
> keys each pattern's attribution set on the answering scan's committed-graph IDENTITY
> (`field_to_be_bytes_32(commitments[g])`) rather than the scan-local integer. Two distinct `k=1`
> scans no longer collapse: `|A_0 ∪ A_1| = 2`, so the cross-scan non-bnode obligation is REQUIRED
> and a manifest omitting it (and any hidden `join_edge`) is rejected by the Q6 obligation itself
> (`recheck` → `MissingObligation`), independent of salt-separation. Same-graph multi-scan still
> collapses correctly (no spurious obligation). Negative regression:
> `join_gates::finding_a_cross_scan_alias_forge_rejected_by_q6` (fails if the namespace fix is
> reverted — verified). This does **NOT** lift any not-production-sound label (sq-qhy4): it removes
> a fragility (Q6 was dead code for cross-scan; salt-separation was the sole live backstop), giving
> defense-in-depth, not a new soundness guarantee.

## Finding B — single-reference revocation binding (fail-closed over-restriction; latent risk)

`manifest.revocation: Option<RevocationStatus>` (manifest.rs:1183) and
`manifest.hidden_revocation: Option<HiddenIndexRevocation>` (1220) are **scalar** — one status
reference and one hidden-index proof per presentation. Every scan-covering commitment's
issuer-signed `att_status` is cross-checked against that single reference (`resolve_status_ref`,
verifier.rs:2563), and `scan_referenced_messages`/`verify_holder_attestation_signature` recompute
every commitment's `status_ref` from the same single `manifest.revocation`.

**Consequence (fail-closed; structural rejection, no false-accept in attempt 5).** A presentation carrying two credentials with **distinct**
(list, index, version) references is structurally rejected: they cannot both match the one
`manifest.revocation`. No false-accept (attempt 5). But this means the headline **cross-credential
JOIN** use case of `bind_joins` — joining two genuinely different credentials — only passes when
both credentials share an **identical** issuer-signed status slot (essentially only intra-credential
multi-graph joins). This is a real **functional limitation** hiding behind an over-restrictive
fail-closed gate, and a **latent soundness risk**: if `revocation`/`hidden_revocation` are ever
promoted to `Vec` to support multi-credential presentations, the single-check assumptions in
`bind_revocation`, `bind_hidden_revocation`, `scan_referenced_messages`, and
`verify_holder_attestation_signature` must ALL be re-derived per-commitment, or a second
credential's liveness would go unchecked. → **P3 bead sq-cuvmj** to (a) document the constraint on
`bind_joins`, and (b) pre-register the per-commitment obligations any future `Vec` migration owes.

> **DOCUMENTED (sq-cuvmj) `[OPUS-5]`.** Both deliverables landed; the limitation itself is
> UNCHANGED (this is documentation + a tripwire, not a fix, and it lifts no soundness label).
> (a) `bind_joins` carries a `# Cross-credential scope constraint` section stating that the gate
> validates hidden joins across the graphs of ONE credential (or credentials sharing a status
> slot), NOT arbitrary multi-credential joins, and that the blocking rejection happens upstream in
> `resolve_status_ref`/`bind_issuer_attestations` — fail-closed, with attempt 5's no-false-accept
> argument recorded inline. (b) `ProofManifest::revocation` carries the canonical pre-registration:
> the scalar-by-design rationale plus the four per-commitment obligations a `Vec` migration owes
> (`bind_issuer_attestations`/`resolve_status_ref`; `bind_revocation`; `bind_hidden_revocation` +
> `bind_fully_hidden_revocation`; `scan_referenced_messages` + `verify_holder_attestation_signature`),
> with `hidden_revocation`/`fully_hidden_revocation` cross-referencing it. The invariant is pinned by
> `verifier::tests::two_credentials_with_distinct_status_refs_are_rejected` — two credentials on
> distinct status slots, both attestations independently VALID, refused at the reference gate, with
> a same-slot control proving the fixture is otherwise accepted. That test is deliberately a
> TRIPWIRE: a `Vec` migration turns it red and must discharge the pre-registered obligations before
> flipping its expectation.

## Overall composition verdict

**NO EXPLOITABLE COMPOSITION BREAK FOUND.** Every attempted construction (field-reorder, attribution
boundary/off-by-one, cross-scan index aliasing, cross-manifest splice, stale-component replay,
single-reference multi-credential smuggle, holder-PoP A-presents-B, hidden-revocation re-point,
hidden-issuer forge, join slot mis-binding, entailment ungrounding) is BLOCKED, either by the
nonce-bound `reconstruct_public_inputs`/canonical-vk seam, by fail-closed structural gates, or by
audit-#9 salt-separation. No composition break was found for the single-credential (and
intra-credential multi-graph) presentations the obligation set structurally admits — i.e. no
false-accept in this single-model code-reading review; this is not a proof of soundness (see the
header disclaimer and sq-qhy4).

Two composition weaknesses are documented, neither an exploitable break at present:
- **Finding A (P2) — RESOLVED (sq-en5dx):** the Q6 non-bnode-obligation gate WAS *inert* for
  cross-scan joins due to a scan-local vs global graph-index namespace mismatch (salt-separation the
  sole live backstop). Fixed by Option (i): `global_attributions` keys the union on committed-graph
  identity, so cross-scan joins over distinct graphs now require the obligation (see the RESOLVED
  note under Finding A). Does not lift any not-production-sound label (sq-qhy4).
- **Finding B (P3) — DOCUMENTED (sq-cuvmj):** scalar `revocation`/`hidden_revocation` fail-closes
  multi-credential presentations (restricting `bind_joins`) and is a latent soundness pitfall for
  any future `Vec` migration. The limitation REMAINS; what landed is the `bind_joins` scope
  constraint, the per-commitment `Vec`-migration obligations pre-registered on
  `ProofManifest::revocation`, and a regression tripwire pinning the single-reference invariant
  (see the DOCUMENTED note under Finding B).

**This does not lift any not-production-sound label. sq-qhy4 (external accredited-cryptographer
audit, P0) remains the gating soundness authority; this review is advisory input to it, not a
substitute.**

---

## Provenance / Fable-safeguard note `[OPUS-4.8]`

This review was **dispatched as Claude Fable 5** but **ran ~97% on `claude-opus-4-8`**: Fable's
dual-use safety classifier downgraded the run to Opus 4.8 mid-session on the adversarial-crypto
content, so only ~2 of ~88 turns executed on Fable. The honest authorship stamp is therefore
**Opus 4.8**, not Fable — the header stamp and the commit trailer were corrected accordingly
(`docs(zk): correct provenance stamp to Opus 4.8 (Fable safeguard downgrade)`). The review
*content* is unchanged; only the provenance attribution was fixed.
