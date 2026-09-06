# Hidden-only-path salt disclosure: is it a residual linkability channel? (sq-93h)

**Status:** ASSESSMENT COMPLETE — **verdict: NO-BUILD.** The remedy sq-93h proposed
(move `m`-reconstruction to an in-circuit salt-commitment, mirroring the sq-ayv
index-commitment cross-binding, so the salt can be withheld) is **not recommended**:
it would hide a correlator that is *dominated* — under the audit-#9 issuance discipline
stated as (A1) in §3 — by one that stays public on the same manifest entry, so it buys
**zero** unlinkability for a new circuit member, a new anchored VK, and added proving
cost. The bead's *question* is answered here; the
bead's *proposed implementation* should not be built as specified.

**Priority:** P3, as filed. Nothing in this assessment raises it.

**Scope:** `sparq-zk-compose` (the hidden-only attestation path introduced by sq-xxg).
This record answers only sq-93h. The in-circuit salt *binding* residual is `sq-hyhj`
(`research/zk-in-circuit-salt-binding.md`) and is a different, still-open item.

## 1. The question

sq-xxg lets a scan commitment be attested by ONLY a hidden-issuer proof (no clear
`CommitmentAttestation`). To recompute the issuer-signed message
`m = commitment_message_with_status(C(G), salt, status_ref)` for such a commitment,
`HiddenIssuerAttestation` carries the per-graph RDFC10 salt
(`manifest.rs:2178`+, resolved by `verifier.rs::resolve_commitment_salt:4438`).

> Is disclosing that salt itself a residual **cross-presentation linkability channel**
> for the same committed graph?

## 2. What is actually disclosed today (verified against code)

| Value | Disclosed? | Where |
| --- | --- | --- |
| `C(G)` — the per-graph commitment | **ALWAYS, in the clear** | `ProofInputs::Scan { commitments }` (`manifest.rs:1028-1031`), and again as `HiddenIssuerAttestation::commitment` (`manifest.rs:2180`) |
| `C(G)` as a **bb public input** | **ALWAYS** | `reconstruct_public_inputs` pushes every `commitments[g]` word (`verifier.rs:7539-7552`) |
| per-graph `salt` — clear path | **ALWAYS, mandatorily** | a scan-covering attestation with `salt: None` is REJECTED `ScanCommitmentSaltMissing` (`verifier.rs:2952-2975`) |
| per-graph `salt` — hidden-only path | yes, on the hidden entry | `HiddenIssuerAttestation::salt` (`manifest.rs:2196-2205`) |

Two further facts drive the verdict:

- **`C(G)` is binding, NOT hiding.** `commit_canonical` is
  `poseidon2::hash(&leaves)` with **no blinder** (`crates/sparq-zk/src/commit.rs:250-259`).
- **The salt is a bnode-domain separator, not a commitment blinder.** IRI and literal
  encodings are salt-INDEPENDENT — `h2(TYPE, blake3(token))`; only the blank-node
  branch folds the salt, as `h2(BLANK_NODE, h2(salt, blake3(label)))`
  (`crates/sparq-zk/src/encode.rs:46-63`).

## 3. Verdict: salt disclosure is DOMINATED by commitment disclosure

The linkability question is *what partition of presentations can a coalition of
verifiers compute*. Compare the two disclosed values:

- `graph → C(G)` is deterministic and (by commitment binding) effectively injective.
  So "same disclosed `C`" ⟺ "same committed graph". This holds unconditionally from the
  code: the commitment is disclosed in the clear on every path (§2).
- `graph → salt` is a per-graph value chosen at issuance. Domination needs only ONE
  direction of it — that a salt is never reused for two DISTINCT graphs, so that
  "same disclosed salt" ⟹ "same committed graph".

**Assumption (A1) — salt injectivity across issuances.** No salt is ever issued for two
distinct graphs. This is the audit-#9 issuance discipline; it is **not** globally
machine-checked. What the verifier actually enforces is the *within-manifest* instance:
`SaltReused` (`verifier.rs:3062-3090`) rejects a manifest in which one salt maps to two
distinct scan/path-*referenced* commitments. It says nothing about salts across separate
presentations or re-issuances, so (A1) is an issuance-side assumption this record states
rather than a property the code establishes.

**Under (A1), the salt partition refines the `C(G)` partition.** The salt is therefore
not an *additional* correlator: any two presentations a coalition can link by salt, it
can already link by `C(G)` — which is not merely disclosed as JSON but byte-bound into
the bb public inputs of every scan sub-proof, so it cannot be withheld without
redesigning the scan member itself.

Note what is **not** claimed. The two partitions are not asserted to be *identical*:
that would additionally need salt *stability* — the same graph re-presented or re-issued
always carrying the same salt — which nothing here enforces either. It is also not
needed. If a re-issuance changes the salt, the salt links strictly *fewer* pairs than
`C(G)` does, which only strengthens domination. So the verdict rests on (A1) alone.

If (A1) is ever violated — a salt shared by two distinct graphs, across presentations
that `SaltReused` never sees together — the salt becomes a correlator `C(G)` does not
provide (it would link two *different* graphs, plausibly by common issuance origin).
That is a violation of the audit-#9 discipline in its own right and is the condition
under which this verdict, like premise (D1) in §5, must be revisited.

Consequence for the proposed remedy: replacing the disclosed salt with an in-circuit
salt-commitment leaves `C(G)` public on the *same* manifest entry. The presentation
stays exactly as linkable as before. The sq-ayv analogy does not carry: there, the
clear `index` was the strongest remaining handle on the credential's status slot and
committing it genuinely removed a distinguisher; here the salt is not the strongest
handle — `C(G)` is, and under (A1) it links at least every pair the salt does.

**Therefore: do not build the salt-commitment circuit member for sq-93h.** The work that
would actually buy cross-presentation unlinkability is hiding or re-randomising `C(G)`
itself, which is a much larger, separate epic and is not what this bead scopes.

## 4. What salt disclosure DOES cost (honest residual — confidentiality, not linkability)

Not the channel sq-93h asks about, but it should be on the record rather than lost:

Because `C(G)` has no blinder (§2), an adversary who **guesses** a graph's contents can
recompute `C(G)` and confirm the guess. For a graph with **no blank nodes** this holds
regardless of the salt — the encodings are salt-independent, so `C(G)` is already an
unblinded hash of guessable content. For a graph **with** blank nodes, the salt is the
only unknown in `h2(salt, blake3("c14n0"))` (RDFC10 canonical labels are a tiny public
dictionary: `c14n0`, `c14n1`, …), so disclosing it makes such a graph confirmable too.

Three reasons this is not sq-93h and not a reason to build the salt-commitment:

1. It is a **guess-confirmation** (confidentiality) exposure, not a linkability one.
2. It is **not specific to the hidden-only path** — the clear path discloses the salt
   *mandatorily* (`verifier.rs:2952-2975`), so the hidden-only path is strictly no
   worse than the status quo the bead itself notes.
3. Its actual fix is a **hiding commitment** (fold a per-graph blinder into
   `commit_canonical`, with the matching in-circuit change), not a salt-commitment.
   Withholding the salt while `C(G)` stays unblinded still leaves every bnode-free
   graph confirmable, so a salt-commitment would not close it either.

Captured as a separate follow-up; it is not in this bead's scope and must not be
claimed as fixed by anything here.

## 5. The precondition, and the trip-wire that guards it

Beyond the issuance-side assumption (A1) of §3, the verdict rests on ONE premise about
the code:

> **(D1)** `C(G)` is disclosed in the clear on every path, including hidden-only.

If a future tier ever hides or re-randomises the commitment, (D1) fails and the
disclosed salt immediately becomes the *finest remaining* correlator — at which point
sq-93h must be RE-OPENED and the in-circuit salt-commitment reconsidered on its merits.

(D1) — and only (D1) — is pinned by
`crates/sparq-zk-compose/src/verifier.rs::hidden_only_salt_disclosure_is_dominated_by_the_clear_commitment`,
which asserts, on the real paths rather than on any test-local notion of "disclosed":

1. the hidden-only salt fallback resolves (`resolve_commitment_salt`);
2. `reconstruct_public_inputs` — whose output stage 3a byte-compares against the
   prover's `public_inputs` — emits `C(G)` as scan public-input **word 1**, byte for
   byte, with and without the salt; and
3. on the serialized wire form of the salt-**withheld** manifest, the salt is gone while
   `C(G)` survives, and the round-tripped manifest reconstructs the same public inputs.

A commitment-hiding change stops emitting that cleartext word and turns (2)/(3) red,
which is exactly the intended signal. (A1) is deliberately **not** claimed to be tested:
it is a cross-presentation issuance property, and the verifier's `SaltReused` check —
the only machine-checked fragment of it — is scoped to a single manifest, so no test in
this crate can establish it.

## 6. Honesty / privacy-claims-gate note

Nothing here asserts any ZK/privacy property as *achieved*. The composition verifier
remains **NOT-yet-sound** (`sq-qhy4` / `sq-9hrn`, epic `sq-1s2`) with **no external
accredited-cryptographer sign-off**; this record is a scoping/priority analysis of one
documented privacy deferral, research-grade, and closes no soundness gap. The register
line for CR-G6 should keep listing per-graph salt disclosure as a known, accepted
disclosure — the correction this record makes is only that it is **dominated by the
commitment disclosure** under §3's stated issuance assumption (A1), so hiding the salt
alone is not the remedy.

## 7. Cross-references

- `research/zk-in-circuit-salt-binding.md` (`sq-hyhj`) — §1 delegates the hidden-path
  salt-disclosure residual to this bead; the in-circuit *binding* half is NOT closed
  by this record and remains open.
- `research/zk-statuslist-hide-iri-version.md` (`sq-6qe`) — the revocation IRI/version
  half of the CR-G6 residual set.
- `research/zk-soundness-audit.md` #9 — the per-graph-salt / cross-graph
  bnode-correlation control this disclosure participates in.
- `research/zk-audit-readiness-dossier.md` CR-G6 and
  `compliance/cryptoreview/gap-register.md` CR-G6 — the residual register rows naming
  `sq-93h`; syncing them to this verdict is a doc-only follow-up.
