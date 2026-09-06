<!-- [OPUS-4.8] Design-for-review authored by Opus 4.8 (Fable 5 unavailable) — re-review when Fable returns. -->
# In-circuit hidden cross-credential JOIN (shared join key never disclosed)

Design record for bead **sq-bwwl** (`area:sparq-zk-compose`, `security`, `zk`):
prove that two scan sub-proofs share a value at a chosen pair of slots —
`row_a[slot_i] == row_b[slot_j]` — **without disclosing the joined term
encoding**, bound to the two scans' public-input commitments so the prover cannot
swap rows. This is the **single-prover** analogue of the deferred MPC PSI join
(`sq-t21`): the same goal, a *distinct* trust model (one prover holds both
credentials). NO production code here; this is a design-for-review record matching
the rigour of `research/zk-soundness-audit.md`, `research/zk-verifier-reaudit.md`,
and `research/zk-holder-pop-design.md`.

Parent context: epic `sq-1s2` ("ZK query-proof build-out + in-circuit privacy
upgrades"). Siblings this design is sized to fit: `sq-z9l` (in-circuit
issuer-signature gadget — the **direct gadget-reuse precedent**, landed),
`sq-3e5`/`sq-ayv` (hidden-index revocation + committed-index cross-binding — the
**commit-and-cross-bind precedent**, landed), `sq-c2ql` (credential-bound
HolderPoP — the **same `verify_manifest` integration shape**). The MPC sibling is
`sq-t21` (§6).

---

## 0. Where the join leaks today (cite file:line)

### 0.1 What a cross-credential join is, in this estate

A multi-pattern BGP that shares a variable across two credentials —
`{ ?p :worksFor :ACME } { ?p :hasSalary ?s }` where the two triple patterns are
answered by **two different committed graphs** (two issuer credentials) — is
proved today by **two separate `scan_k…` sub-proofs**, one per pattern, plus the
verifier-side `recheck` join machinery. The shared variable `?p` is the join key.

The scan circuit's `rows` parameter is **PUBLIC** and carries the full term
encodings of every disclosed match row:

- `zk/compose/scan_k2_n16_r8/src/main.nr:14` —
  `rows: pub [[Field; 3]; 8],   // disclosed matched rows (term encodings)`
- `crates/sparq-zk-compose/src/manifest.rs:438-439` — the host mirror:
  `/// Disclosed matched rows (length r, padded with zero rows). rows: Vec<[FieldHex; 3]>`

So scan-A's rows disclose `Enc(?p)` in (say) slot 0, and scan-B's rows disclose
the **same** `Enc(?p)` in (say) slot 0. **The join key's term encoding is in the
clear in both proofs' public inputs.** The relying party reads it directly.

### 0.2 The two ways the join is "checked", and where each leaks

1. **The fundamental leak — the join is implicit in disclosed rows.** The shared
   variable's encoding appears in the PUBLIC `rows` of *both* scans. The
   verifier's `recheck` (`verify.rs`, driven from
   `prefilter_manifest_structure`, `verifier.rs:1548-1567`) and the eventual
   plaintext re-join over disclosed rows operate over those public encodings. **No
   binding edge is even required for the leak:** the join key is disclosed by the
   scan proofs themselves. This is the headline privacy gap — the
   *privacy-sensitive joined entity* (`?p`, a person/account/DID) is revealed
   whenever two credentials are joined.

2. **The binding-edge equality — same disclosure class, scan→filter only.** The
   one place a *cross-proof slot equality* is currently enforced is the
   binding-consistency loop:

   - `crates/sparq-zk-compose/src/verifier.rs:1630-1659` — for each
     `BindingEdge`, the verifier reads the scanned slot from the PUBLIC rows
     (`verifier.rs:1640-1644`, `rows.get(edge.from_row).get(edge.from_slot)`) and
     compares it as a **plaintext `FieldHex` string equality** to the consuming
     proof's `operand_enc` (`verifier.rs:1656-1658`, `if scanned != operand { …
     BindingInconsistent }`).

   The `BindingEdge` type (`manifest.rs:513-523`) only ever connects a **scan →
   filter** (`from_proof = scan`, `to_proof = filter`); the `to` side must be a
   `FilterInt`/`FilterF64` or the verifier returns `EdgeKindMismatch`
   (`verifier.rs:1645` for the scanned-slot arm, `verifier.rs:1654` for the
   operand arm). **There is no scan↔scan join edge, and no
   `join_eq` circuit member** — only `scan` / `filter_int` / `filter_f64` /
   `revoke_unset` / `hidden_issuer` members exist (`manifest.rs:366-405`,
   `CircuitId`). So even the equality the estate *does* enforce is (a) over
   disclosed plaintext and (b) not available for cross-credential joins.

**The leak, precisely (the one to cite):**
`crates/sparq-zk-compose/src/verifier.rs:1640-1658` reads the join value from the
PUBLIC scan `rows` (`manifest.rs:438-439`, `scan_k2_n16_r8/src/main.nr:14`) and
checks equality over the **disclosed** term encoding. The joined entity's encoding
is a public input of the scan proofs; it is never hidden. There is no in-circuit
join relation in `zk/compose/compose_core/` (`scan.nr`, `filter_int.nr`,
`filter_float.nr`, `revoke.nr`, `issuer.nr` — no `join.nr`).

---

## 1. Threat model

### 1.1 Setting and parties

The **single-prover** trust model — the load-bearing distinction from `sq-t21`:

- **One prover (holder)** legitimately holds **both** credentials (e.g. an
  employment VC from issuer X and a payroll VC from issuer Y), each a committed
  named graph with an issuer Schnorr attestation over its commitment
  (`bind_issuer_attestations`, `verifier.rs:1763`). The prover sees both
  plaintexts; there is **no secret to hide *from the prover*** — only from the
  **verifier**. (Contrast `sq-t21`, where two *mutually distrusting* holders each
  hide their side from the *other*; that requires MPC, not single-prover ZK.)
- **Verifier (relying party)** anchors trusted issuer keys (`KeySet K`), issues a
  fresh `challenge` per presentation, and runs `verify_manifest`. It must learn
  *that* a join holds (and possibly its cardinality) but **not the joined term**.

### 1.2 Adversary and goal

A **malicious prover** controlling its own side fully (writes the manifest JSON,
chooses public inputs, runs the prover). Its goal is to get `verify_manifest`
=> `Ok(())` for a join that **does not actually hold** — i.e. to claim
`?p` is shared across the two credentials when the two credentials bind `?p` to
*different* entities — **while** the verifier (and we, the designers) want the
joined value hidden. The two failure classes the design must defeat:

- **(A1) Forge a join across non-equal terms.** Prove `row_a[i] == row_b[j]` when
  `row_a[i] ≠ row_b[j]` (the equality is a lie).
- **(A2) Bind to the wrong / unbound rows.** Prove a true equality but over rows
  that are **not** the rows of the two scan proofs the manifest presents (swap in
  rows from some other graph, or rows the scan proof never disclosed), so the
  "join" attaches to a statement the credentials do not support.

The issuer is honest; the verifier is honest and fail-closed; the audit-#1/#2
public-input-reconstruction + canonical-vk gate and the audit-#3/#9/#12 issuer
gates are in force (post-`sq-1s2`). The residual privacy hole this design closes
is the **disclosure of the join key** (§0); the residual *soundness* obligations
it must not reopen are A1/A2.

### 1.3 What must stay hidden

The **join-key term encoding** `Enc(?p)` — the privacy-sensitive joined entity.
After this design, neither `row_a[i]` nor `row_b[j]` (nor anything from which the
verifier can recover `Enc(?p)` cheaply) appears in any public input.

### 1.4 What the verifier still learns — residual leakage (stated plainly)

A hidden-key join is **not** a zero-knowledge oracle. Honestly:

- **(R1) That a join exists.** The verifier learns there is *some* shared value at
  `(slot_i, slot_j)` across the two credentials — that is the whole point of
  presenting the proof. This is intrinsic and not removable while still answering
  the query.
- **(R2) Cardinality / multiplicity.** The number of joined rows the proof
  attests (how many `(a,b)` pairs match) is visible from the manifest structure
  (one `join_eq` instance per matched pair, or a multiplicity public input). A
  hidden-key join with N matched pairs reveals N. Hiding cardinality requires
  padding to a fixed bound (a fixed-R join member) — a cost/leakage trade-off
  noted in §5 and left as an optional tier.
- **(R3) Everything the scan proofs *otherwise* disclose.** This member hides only
  the *join column*. Any OTHER disclosed slot in the same rows (e.g. a disclosed
  `:worksFor :ACME` constant, or a disclosed salary) stays disclosed. If `?p` is
  the *only* sensitive term, the row may otherwise be all-constant/variable; the
  privacy win is exactly the join key.
- **(R4) Dictionary / brute-force on the encoding.** The hidden value is a
  Poseidon2 term encoding over a (possibly small) domain. If the verifier can
  *enumerate candidate `?p` values* and recompute encodings, a hidden equality
  over an unblinded encoding is brute-forceable offline. **This is a real residual
  and it drives the construction choice** (§2.4: per-presentation blinding /
  salting of the committed join value, so the public artefact is hiding, not a
  bare deterministic hash the verifier can dictionary-attack). Even blinded, R1/R2
  remain.

> **Honest bottom line.** This member converts "the joined entity is disclosed" to
> "the verifier learns a join of cardinality N exists, but not *which* entity",
> provided the public join artefact is a *hiding* commitment (§2.4). It does not,
> and cannot, hide *that* a join was performed.

---

## 2. Construction

### 2.1 The trivial in-circuit core

Equality of two hidden field elements is the cheapest possible constraint:
`assert(a == b)` (one `Field` subtraction-to-zero). The entire difficulty is
**not** the equality — it is **binding** `a` and `b` to *the right rows of the
right scan proofs* while keeping them private, and making the public artefact
*hiding* so R4 does not reopen the disclosure.

The scan circuit already commits every slot value: each row's term encodings are
absorbed into the per-graph Poseidon2 commitment (`scan.nr:96-108`,
`commit_fold` over `h3(enc[g][i][0..2])`), and the disclosed `rows` are
constrained to be present in some active slot (`scan.nr:126-150`). So a slot value
is *already cryptographically committed* — but as a **public** row element today.
The construction makes the join value **private** and re-binds it to the same
commitment.

### 2.2 RECOMMENDED construction — `join_eq` over per-scan committed slot values

The recommended single-prover construction is a **new `join_eq` circuit member**
that takes the **two graph commitments and the two join-slot indices as public
inputs** (the `commitments[g]` values the two scan proofs already expose and that
the audit-#1 reconstruction byte-binds, plus `slot_a`/`slot_b` — which the query
text already reveals, see §4.4), re-derives the two joined slot **values**
**privately in-circuit from the witnessed graph contents**, asserts they are equal,
and exposes a **hiding commitment to the join value** as the only public join
artefact. **The join VALUE is hidden; the join SLOTS are public** — this is the
soundness choice §4.4 concludes (the slot the shared variable occupies is not a
secret, only the joined term is), and the public-input sketch below is consistent
with it.

**Public inputs of `join_eq` (declaration order = the audit-#1 reconstruction
order):**

```text
challenge:        pub Field         // verifier nonce (replay binding S2.5) — field 0, as every member
commit_a:         pub Field         // graph-A commitment  C(G_a)  == scan-A.commitments[g_a]
commit_b:         pub Field         // graph-B commitment  C(G_b)  == scan-B.commitments[g_b]
join_commitment:  pub Field         // HIDING commitment to the join value (see §2.4)
slot_a:           pub u32           // graph-A join slot in {0,1,2} (== query-derived s_a, §4.4)
slot_b:           pub u32           // graph-B join slot in {0,1,2} (== query-derived s_b, §4.4)
```

**Private witnesses:**

```text
enc_a:    [[Field;3]; N_a]   counts_a: u32   // graph-A contents + size (same shape as scan-A)
enc_b:    [[Field;3]; N_b]   counts_b: u32   // graph-B contents + size
row_a:    [Field; 3]                         // the joined row of A
row_b:    [Field; 3]                         // the joined row of B
blinding: Field                              // per-presentation blinder for join_commitment
```

(The join slots are **not** private witnesses — they are the PUBLIC `slot_a` /
`slot_b` above, byte-bound to the query-derived positions, §4.4.)

**The relation (sketch — mirrors `scan.nr`'s present-in-graph discipline):**

```text
// 1. Re-commit both graphs to their PUBLIC commitments (binds the witnessed
//    contents to the SAME C(G) the scan proofs expose — §2.3 anti-row-swap).
assert_eq(commit_fold(map(h3, enc_a), counts_a), commit_a);   // == scan.nr:96-108
assert_eq(commit_fold(map(h3, enc_b), counts_b), commit_b);

// 2. row_a / row_b are genuinely rows of their committed graphs (active slot).
assert(row_present_in(enc_a, counts_a, row_a));               // == scan.nr:139-149
assert(row_present_in(enc_b, counts_b, row_b));

// 3. Select the join slots at the PUBLIC, query-bound positions slot_a / slot_b
//    (in range {0,1,2}); the verifier equates these to the query-derived s_a/s_b
//    (§4.4 slot binding), so the prover cannot pick a convenient column.
let a_val = select_slot(row_a, slot_a);    // slot_a in {0,1,2}, public
let b_val = select_slot(row_b, slot_b);    // slot_b in {0,1,2}, public

// 4. THE JOIN — trivial equality over the two hidden values.
assert(a_val == b_val, "join slots are not equal");

// 5. Hiding public artefact: bind the (single) join value under a blinder.
assert(join_commitment == h3(JOIN_DOMAIN, a_val, blinding));  // §2.4
```

Step 1 is **the load-bearing binding**: because the prover must reproduce the
*exact* graph commitments the two scan proofs published (and those are byte-bound
into the scan proofs' public inputs and ultimately into the issuer signatures over
them — `verifier.rs:3409-3410`, `bind_issuer_attestations`), the prover cannot
feed `join_eq` a graph other than the one the scan attests. Step 2 forces `row_a`
to be a real row of that graph. The equality (step 4) is then over values that
provably come from the two attested credentials.

### 2.3 How `join_eq` binds to the scan proofs (no row swap, A2)

The composition is **commitment equality**, not row equality. The verifier
(§3) checks `join_eq.commit_a` byte-equals `scan_a.commitments[g_a]` and
`join_eq.commit_b` byte-equals `scan_b.commitments[g_b]` — both are public inputs,
both already audit-#1 byte-bound into their proofs. So:

- The prover cannot point `join_eq` at a graph the scan proofs do not attest (the
  commitment would not match a scan's bound `commitments[g]`).
- The prover cannot present a `row_a` that is not in `G_a` (step 2 re-checks
  presence against the re-committed contents — the same `scan.nr:139-149`
  constraint, so the re-commit in step 1 is what makes presence meaningful).
- Because both scans already prove **completeness** (every matching active slot is
  disclosed, `scan.nr:152-176`) and the issuer signed `C(G)` (audit #3), the join
  is over genuine, attested, complete credential content.

> **Design note — why re-commit instead of "reuse the scan's `rows`".** The
> alternative is to make the scan member itself expose a *per-row hiding
> commitment* (replacing the cleartext `rows` for the join column) and have a tiny
> `join_eq` member prove `open(c_a) == open(c_b)`. That is cleaner (no
> graph-re-commit) but requires **changing the scan circuit's public-input
> layout** — which re-touches the audit-#1 reconstruction, the empirical bb
> anchors, and every scan member. The recommended construction keeps the scan
> members **untouched** and adds a self-contained `join_eq` that re-derives from
> the already-public `commitments[g]`. See §2.6 for the alternative weighed
> against this.

### 2.4 Hiding the public artefact (defeating R4)

If `join_eq` exposed nothing about the value, the verifier could not even confirm
the same value joins across a *chain* of more than two patterns, and (more
importantly) we want a stable public handle to (a) cross-bind a 3-way join and (b)
let the FILTER/other members reference the joined column without disclosing it. The
**hiding commitment** `join_commitment = h3(JOIN_DOMAIN, a_val, blinding)` (step 5)
solves R4:

- `blinding` is a **per-presentation** random field element (drawn like the
  ingest salt, `ingest.rs:136-149` `SaltMint` precedent). Two presentations of the
  same join value produce **unlinkable** `join_commitment`s.
- Without the blinder, `join_commitment` would be a deterministic Poseidon2 hash
  of a (possibly low-entropy) term — **dictionary-attackable** by a verifier who
  enumerates candidate `?p` (R4). The blinder makes it a *hiding* commitment
  (Poseidon2 is the commitment primitive the estate already uses for the
  index-hiding revocation commitment, `revoke.nr:145`
  `h3(SIG_DOMAIN_STATUS_IDX_COMMIT, index, blinding)` — **exact reuse precedent**).
- The domain tag `JOIN_DOMAIN` (`"ZKSIG_JN"`, a fresh constant mirroring
  `SIG_DOMAIN_*` in `sig.rs`) prevents cross-substituting a join commitment for a
  status/index/holder commitment — the audit's domain-separation discipline.

For a **multi-way join** (the join value shared across patterns 1,2,3), each
pairwise `join_eq` can take the **same `join_commitment`** as a public input (the
prover uses one blinder), and the verifier checks the join_commitments are equal
across the chain — composing pairwise equalities into an N-way join without ever
disclosing the value. (Equality of `a_val` across the chain follows transitively
from each pairwise `assert(a_val == b_val)` plus the shared-commitment check.)

### 2.5 Multiplicity / duplicate handling, ordering, canonicalisation

- **Multiplicity (R2).** SPARQL bag semantics: a join can produce N>1 rows. Each
  matched `(row_a, row_b)` pair is one `join_eq` instance (one sub-proof) OR a
  fixed-R `join_eq_rN` member proving R pairs at once (cheaper amortised, hides
  exact N up to R by zero-padding). The **recommended default is one instance per
  pair** (simplest, composes with the existing per-sub-proof verifier loop);
  the fixed-R member is the cardinality-hiding tier (§5).
- **Duplicate join values.** Two different `?p` values colliding to the *same*
  field encoding is ruled out **only up to the standard collision-resistance
  assumption**, not as a mathematical guarantee: `encode_term` is a hash-based
  encoding (Blake3 + Poseidon2 over the type-code-prefixed value, `encode.rs:46-59`;
  bnodes salted, IRIs/literals salt-independent), so distinct terms map to distinct
  field elements with all but negligible probability. Under that assumption
  `a_val == b_val` ⟺ the terms are equal (a collision would let a prover join two
  *different* entities — the same negligible-probability caveat the rest of the
  estate's encoding soundness rests on). [OPUS-4.8] Within ONE credential, a bnode
  is salted per-graph (`encode.rs:56-59`)
  — a **bnode is never a valid cross-credential join key** (its encoding differs
  across graphs by salt, by design — the audit-#9 separation), so the verifier
  must REJECT a `join_eq` whose join slot is provably a bnode-typed term across
  graphs. (In practice the join key is a global IRI; this is the same
  "global-IRI-as-join-key" convention the MPC side documents, `join.rs:14-23`.)
- **Ordering / canonicalisation.** The equality is symmetric, so `(commit_a,
  commit_b)` ordering is free — but to keep the manifest canonical and the
  verifier check deterministic, the manifest **sorts the edge by
  `(from_proof, from_slot)`** (the existing `BindingEdge` ordering convention) and
  the verifier treats `join_eq` as unordered. Term encodings are already canonical
  (Poseidon2 over canonical RDF terms, `encode.rs`; literal canonicalisation per
  `filter_int.nr` tokenisation), so there is no in-circuit canonicalisation to do
  beyond what the scan member already enforces.

### 2.6 Alternative constructions (noted, not recommended)

- **(Alt-A) Per-row hiding commitments in the scan member.** Replace the cleartext
  join column in `scan.rows` with `h3(JOIN_DOMAIN, enc, row_blinding)` and have a
  micro `join_eq` prove `c_a == c_b` (commitment equality) + open both to the same
  value. *Pro:* no graph re-commit in `join_eq` (cheapest join member). *Con:*
  changes the scan public-input layout → re-touches the audit-#1 reconstruction,
  the empirical bb anchors (`verifier.rs:3582-3614`), and every `scan_k…` member;
  higher blast radius, higher re-audit cost. **UNDECIDED — measure first** (§5,
  `sq-uii0`): whether removing the re-commit pays for a breaking scan
  public-input-layout change is a `bb gates` question, and no in-tree,
  regression-gated measurement exists yet, so this record takes no position either
  way. It is decided when `sq-uii0` lands the probe methodology plus a
  `_comment_sq_uii0` snapshot entry. Whatever that says, Alt-A is a re-audit of the
  most soundness-critical surface in the estate — never a drive-by.
- **(Alt-B) Set-membership / circuit-PSI inside one prover.** Prove the join value
  is in the *intersection* of the two graphs' value sets via a Merkle/lookup
  argument (the `key_set_membership` gadget, `issuer.nr:221-240`). *Pro:* natural
  for many-to-many joins, hides cardinality better. *Con:* heavier (Merkle per
  side) and **unnecessary in the single-prover model** — the prover knows both
  sides, so it can witness the matched pair directly; PSI machinery only pays off
  when the two sides are *mutually private* (that is `sq-t21`, §6). Rejected for
  the single-prover member on empirical-honesty grounds: it buys nothing the
  trivial equality does not, at strictly higher cost.
- **(Alt-C) Pure verifier-side equality over hidden per-row commitments, no
  circuit.** If scans exposed per-row hiding commitments (Alt-A), the *equality*
  could be a verifier-side `commit_a == commit_b` byte-compare with **no `join_eq`
  circuit at all**. *Con:* equality of two *hiding* commitments with **different
  blinders** is not a value equality (it's a coincidence check); you would need a
  shared blinder, which re-links presentations (reopens R4). The circuit is what
  lets the blinders differ while still proving value equality. So Alt-C is unsound
  for the hiding requirement — **recorded as the trap to avoid.**

---

## 3. Fit to the existing compose pipeline

### 3.1 New circuit-family member + `CircuitId`

- **New circuit member** `zk/compose/join_eq/src/main.nr` (and, for the
  cardinality-hiding tier, `join_eq_rN`), with the relation in a new
  `zk/compose/compose_core/src/join.nr` (sibling of `scan.nr`/`issuer.nr`),
  reusing `commit_fold`/`h3`/`h2` from `hashes.nr` **verbatim** (the re-commit in
  §2.2 step 1 is literally `scan.nr:96-108`). Parameter order in `main` IS the
  public-input layout the verifier reconstructs (audit-#1 discipline,
  `scan_k2_n16_r8/src/main.nr:4-5` "Do not reorder `main`'s parameters").
- **New `CircuitId::JoinEq`** in `manifest.rs:366-405`, e.g.
  `JoinEq { n_a: u32, n_b: u32 }` (the two graph-size buckets; `package()` =>
  `join_eq_na{n_a}_nb{n_b}`), derived from the manifest shape exactly like
  `Scan { k, n, r }` so a proof verifies only against the member its public inputs
  fit.

### 3.2 New `ProofInputs` + manifest edge

- **New `ProofInputs::JoinEq`** (`manifest.rs:427-483`) carrying
  `{ id, commit_a, commit_b, join_commitment }` (all `FieldHex`) plus the PUBLIC
  join slots `{ slot_a, slot_b }` (`u32`) — exactly the member's `pub` params after
  `challenge` (§2.2 / §4.4). `ProofInputs::circuit_id()` (`manifest.rs:485-493`)
  gains the arm.
- **New `JoinEdge`-style manifest edge** OR (cleaner) a dedicated
  `join_edges: Vec<JoinEdge>` on `ProofManifest` (`manifest.rs:604-607`, alongside
  `binding_edges`):

  ```text
  pub struct JoinEdge {
      pub scan_a: usize,  pub graph_a: usize,   // which scan sub-proof + which commitments[g]
      pub scan_b: usize,  pub graph_b: usize,
      pub join_proof: usize,                    // index of the join_eq sub-proof
  }
  ```

  This is the *hidden-key* analogue of `BindingEdge` (`manifest.rs:513-523`): where
  `BindingEdge` ties a **disclosed** scan slot to a filter operand, `JoinEdge` ties
  two scans' **commitments** to a `join_eq` proof — disclosing the *graph linkage*
  but **not the joined value**.

### 3.3 New verifier gate (in `verify_manifest`'s staged flow)

A new **`bind_joins`** stage, slotted in `verify_manifest` after the
issuer-attestation gate and before final accept (the same slot
`research/zk-holder-pop-design.md` §3.3 uses for `bind_holder_pok`), gated by a
relying-party `JoinPolicy` (mirroring `RevocationPolicy`/`HolderBindingPolicy`) so
a deployment not using hidden joins stays on the disclosed-row path. For each
`JoinEdge`:

1. Resolve `scan_a`/`scan_b` sub-proofs and read their public `commitments[graph_a]`
   / `commitments[graph_b]` (byte-bound by audit-#1, `verifier.rs:3409-3410`).
2. Read the `join_eq` sub-proof's public `commit_a`/`commit_b` and require
   **byte-equality** with the scans' commitments (`JoinCommitmentMismatch` if not).
   *This is the anti-A2 check — the join is bound to the right credentials.*
3. Require both `graph_a`/`graph_b` commitments carry a valid in-`K` issuer
   attestation (already enforced by `bind_issuer_attestations`,
   `verifier.rs:1763`) — i.e. the joined credentials are attested.
4. The `join_eq` proof itself is verified in the bb stage exactly like every other
   sub-proof: `reconstruct_public_inputs` rebuilds `[challenge, commit_a, commit_b,
   join_commitment, slot_a, slot_b]` (verifier nonce as field 0), `canonical_vk` by
   re-derived `CircuitId::JoinEq`, `bb verify` (`verifier.rs:3280-3312` loop).
   *This is the anti-A1 check — the equality is cryptographically proved, not
   JSON-asserted.*
5. **Query binding (audit-#5/#10 discipline).** `bind_query_correctness`
   (`verifier.rs:2781`) gains: a query whose BGP shares a variable across two
   patterns answered by two *different* scans REQUIRES a `JoinEdge` whose
   `(scan_a, graph_a, slot)`/`(scan_b, graph_b, slot)` map to that shared variable's
   slots (`variable_slots`, `verify.rs`), else `UnboundJoin` (fail-closed — a join
   the query demands but the manifest does not prove is rejected, exactly as
   `UnboundFilter`/`UnboundPattern` are). The slot indices are the PUBLIC
   `slot_a`/`slot_b` inputs (§2.2, §4.4 — the slot is not secret, only the value
   is), so the verifier binds them with a plain public-input equality: it requires
   `slot_a`/`slot_b` to equal the *slot the variable occupies* from the query parse,
   and the circuit's `select_slot(row, slot_a)`/`select_slot(row, slot_b)` is taken
   at exactly that public position — see §4.4 (the slot-binding obligation, the one
   genuinely new soundness subtlety, discharged by making the slots public).

### 3.4 Composition with filter / revoke / issuer / HolderPoP

- **With FILTER.** A `join_eq` exposes `join_commitment`, not the value; a FILTER
  over the *joined* column would need the value. Two honest options: (i) the
  FILTER's `operand_enc` is the **disclosed** non-join slot (joins and filters on
  *different* columns compose trivially — the common case), or (ii) a future
  `filter_committed` member proves `op(open(join_commitment), bound)` reusing the
  blinder opening (a v2, noted, not in scope). For v1, **join hides the join
  column; filters operate on other columns** — composes cleanly via the existing
  `binding_edges`.
- **With revoke / issuer / HolderPoP.** Orthogonal: `join_eq` consumes only the
  two scan commitments (which are already issuer-attested and revocation-checked
  per the existing gates). The hidden-revocation (`bind_hidden_revocation`,
  `verifier.rs:2365`) and hidden-issuer (`bind_hidden_issuer_attestations`,
  `verifier.rs:2528`) proofs cover the *same* commitments `join_eq` references, so
  a hidden join over two fully-hidden credentials composes — the join adds no new
  disclosure beyond R1/R2. The whole thing rides the same single-use nonce
  (audit-#4) and canonical-vk discipline.

### 3.5 Concrete flow sketch

```text
prover:   for each cross-credential shared variable in the query plan:
            witness row_a, row_b from the two graphs it holds; take the PUBLIC
            join slots slot_a/slot_b from the query (the shared variable's positions);
            draw blinding; compute join_commitment = h3(JOIN_DOMAIN, a_val, blinding);
            prove join_eq{commit_a, commit_b, join_commitment, slot_a, slot_b}
              (challenge = verifier nonce);
            append a SubProof + a JoinEdge to the manifest.

verifier (verify_manifest):
   stage 1/2  … existing prefilter + binding-edge + query-correctness …
   stage 2d   bind_issuer_attestations            (commitments attested, in K)   [existing]
   stage 2g   bind_joins:                                                          [NEW]
                for edge in join_edges:
                  assert join_eq.commit_a == scan[edge.scan_a].commitments[graph_a]   // anti-A2
                  assert join_eq.commit_b == scan[edge.scan_b].commitments[graph_b]
                  assert join_eq.slot_a/slot_b == query-derived s_a/s_b (variable_slots) // §4.4 slot bind
                  (query-correctness: a demanded cross-scan join has a JoinEdge)       // audit-#10
   stage 3    for each sub_proof (incl join_eq):                                   [existing loop]
                reconstruct_public_inputs (nonce as field 0) + byte-compare          // anti-A1
                canonical_vk by CircuitId::JoinEq; bb verify
   accept iff all stages pass.
```

---

## 4. Soundness

Against the §1.2 adversary, tied to the discipline in
`research/zk-soundness-audit.md` / `research/zk-verifier-reaudit.md` (the
load-bearing lesson: *every prover-supplied JSON field is reconstructed into and
byte-equalled against the bb public inputs, and every trust anchor is the
verifier's, never the prover's*).

### 4.1 Why the prover cannot forge a join across non-equal terms (A1)

The equality `assert(a_val == b_val)` is **inside the circuit** (§2.2 step 4). By
soundness of the proof system, a valid `join_eq` proof exists only if a witness
satisfying *all* constraints exists — including `a_val == b_val`. The verifier runs
`bb verify` against the **canonical vk** for `CircuitId::JoinEq` (recomputed
verifier-side, audit-#2, `driver.rs::canonical_vk:240-255`) over the
**reconstructed** public inputs (audit-#1, `verifier.rs:3280-3312`), so a forged
proof / attacker vk / mismatched public inputs all reject. A prover with
`row_a[slot_a] ≠ row_b[slot_b]` simply has no satisfying witness — **reject**.
(Contrast the *today* check at `verifier.rs:1656-1658`, which is a JSON string
equality the prover
chooses; the new member moves the equality under the proof.)

### 4.2 Why the prover cannot bind to unbound / swapped rows (A2)

- **Graph binding.** `join_eq.commit_a`/`commit_b` are public inputs the circuit
  *re-derives the graph from* (step 1, `assert_eq(commit_fold(...), commit_a)`),
  and the verifier requires them **byte-equal to the two scan proofs' published
  `commitments[g]`** (`bind_joins`, §3.3 step 2). Those scan commitments are (i)
  byte-bound into the scan proofs (audit-#1, `verifier.rs:3409-3410`) and (ii)
  issuer-signed (audit-#3, `bind_issuer_attestations`). So a prover cannot feed
  `join_eq` any graph other than the two attested credentials — the commitment
  would not match.
- **Row presence.** `row_a`/`row_b` are constrained present in their re-committed
  graphs (step 2, the `scan.nr:139-149` discipline). A row not in `G_a` has no
  witness.
- **Net.** The join is provably over two genuine, attested, present rows of the two
  presented credentials — A2 is closed by the *commitment-equality* binding, the
  same mechanism the binding-edge fix used (audit-#7, `verifier.rs:1640-1658`),
  but now over *commitments* rather than disclosed rows.

### 4.3 Why the hiding artefact does not create a forgery channel

`join_commitment = h3(JOIN_DOMAIN, a_val, blinding)` is **constrained in-circuit**
to bind the SAME `a_val` the equality used (step 5), with a domain tag distinct
from every other commitment (anti-cross-substitution, the audit's
domain-separation rule, cf. `revoke.nr:145`'s `SIG_DOMAIN_STATUS_IDX_COMMIT`). The
verifier never *opens* it; it only (optionally) byte-equals it across a multi-way
join chain (§2.4). Because the blinder is private and per-presentation, two honest
presentations are unlinkable (R4 closed) — and because `a_val` is bound, the prover
cannot commit to a *different* value than the one it proved equal.

### 4.4 The one genuinely new soundness obligation — slot binding

If the join slots were left private, the verifier would not learn which column is
the join key — but it would also be unable to pin the equality to the *right*
column. This is a **new obligation** the construction must discharge, analogous to
audit-#6 (the salary-slot-for-age forge):

> The query says `?p` is shared at pattern-A slot `s_a` and pattern-B slot `s_b`.
> The circuit must prove the equality is over **those** slots, not whichever slots
> the prover finds convenient.

The two ways to discharge it, and why the recommended one wins:

- **(Recommended, and the choice §2.2 takes) Make the slots PUBLIC.** Expose
  `slot_a: pub u32`, `slot_b: pub u32` as public inputs of `join_eq` (as in the
  §2.2 public-input sketch); the circuit selects with `select_slot(row, slot_a)` /
  `select_slot(row, slot_b)`, and `bind_query_correctness` requires
  `slot_a`/`slot_b` to equal the query-derived `(s_a, s_b)` for the shared variable
  (`variable_slots`, `verify.rs`). This discloses *which column* is the join key —
  but the query *already* reveals that (the shared variable's position is in the
  query text the verifier reads), so it is **not new leakage**. It closes the forge
  with a plain public-input equality (audit-#1 byte-bound). **This is the chosen
  construction: the slot is not secret; only the *value* is.**
- (Alternative, rejected) Keep the slots private and prove the join column matches a
  query-bound constant pattern — heavier, no privacy gain (the query reveals the
  column anyway). Rejected.

So: **the join VALUE is hidden; the join SLOTS are public (already implied by the
query).** This is the precise, honest privacy boundary — and §2.2's public-input
sketch, witnesses, and relation are all stated consistently with it.

### 4.5 New verifier obligations (tie-back to the audit discipline)

1. **Anchor `commit_a`/`commit_b` in the scan proofs, never the manifest alone.**
   The commitments `bind_joins` compares MUST be the scans' audit-#1-bound public
   inputs (and issuer-signed), not a prover JSON field read in isolation. A
   commitment read only from the `join_eq` JSON is the audit-#1 hole reopened.
2. **Fold `commit_a`/`commit_b`/`join_commitment`/`slot_a`/`slot_b`/`challenge`
   into the reconstructed public-input vector and byte-equal them (audit-#1);
   select the `join_eq` vk by re-derived `CircuitId::JoinEq` (audit-#2).** A
   `join_eq` proof whose public inputs disagree MUST reject.
3. **Fail-closed, no silent join skip.** A query with a cross-scan shared variable
   and no covering `JoinEdge` ⇒ `UnboundJoin` (the audit-#10 FILTER-add
   precedent). A `JoinEdge` whose commitments do not match the scans ⇒
   `JoinCommitmentMismatch`. A bnode-typed cross-graph join slot ⇒ reject (§2.5).
4. **Standing forge-and-verify regression tests**, one per failure mode
   (non-equal terms → bb verify fails; swapped graph → `JoinCommitmentMismatch`;
   unbound query join → `UnboundJoin`; wrong slot → slot-mismatch; replay under
   fresh nonce → reject), in the spirit of `sq-1gir`/`sq-ajl` and the
   `crates/sparq-zk-compose/tests/forge_*` suite.

If any of (1)–(3) is implemented as a JSON-only check, the join becomes a
prover-asserted lie — the exact failure class `sq-gbp4` was created to catch.

---

## 5. Performance / feasibility

Qualitative only — **no fabricated gate counts**. Per repo policy and the
`noir-optimisation` discipline, any cost claim MUST be measured with
`bb gates -s ultra_honk` before it is asserted; an unmeasured number is a
soundness-of-claims violation, not just an estimate.

- **Cost class — same as a scan, dominated by the re-commit.** The dominant cost
  of `join_eq` is the **two `commit_fold` re-commitments** (step 1) — one Poseidon2
  permutation per ~3 leaves per graph (`hashes.nr:13-15`,
  `ceil(N/3)+1` permutations), i.e. **roughly the per-graph hashing of a `scan`
  member with the same `N`**. The equality (step 4) and the hiding commitment
  (step 5, one `h3`) are negligible (~one permutation). So `join_eq` lands in the
  **same cost class as the scan members it consumes** — it re-pays the per-graph
  commitment fold, plus epsilon. It is **far cheaper than `hidden_issuer_d{D}`**,
  whose two ~251-bit Baby-JubJub scalar-muls dominate (`issuer.nr:267-273`); there
  are **no scalar-muls, no foreign-field emulation, no pairings** in `join_eq`.
- **Optimisation lever (measure first).** The re-commit is the only plausibly heavy
  part. If it dominates, Alt-A (§2.6 — scan exposes per-row hiding commitments,
  `join_eq` becomes a tiny open-and-compare with NO re-commit) removes it, at the
  price of a scan-layout change. **Whether Alt-A is worth it is a `bb gates`
  question**, not an intuition one (the `noir-optimisation` skill and
  `bench/SPIKES.md` record intuition misfiring on this codebase — PR #37). Two
  priors this section has carried are themselves unmeasured and must not be leaned
  on: that the re-commit dominates, and that removing it would collapse the member
  to "~2 Poseidon2 permutations". The second has an in-tree reason for doubt: the
  `sq-kndw` precedent (`_comment_sq_kndw` in
  `crates/sparq-zk-compose/tests/gate_count_snapshot.json`) measured a large FIXED
  integer-lookup-table cost for the FIRST integer-typed input — an *unused* integer
  input carried the same overhead, i.e. table setup, not the comparison — and
  `join_eq`'s public `slot_a`/`slot_b` are `u32` **by design** (§4.4 slot binding).
  If that floor applies here too, "~2 permutations" is unreachable. That is a
  hypothesis to measure, not a finding.

  **NO VERDICT IS RECORDED HERE, because no reviewable evidence exists yet.**
  Off-tree, one-shot probe circuits were run under `sq-uii0`, but their
  methodology, circuits and raw gate counts are not in this tree and there is **no**
  `_comment_sq_uii0` entry in
  `crates/sparq-zk-compose/tests/gate_count_snapshot.json`, so none of it is
  checkable against `gate_count_regression` and no reviewer can confirm the probes
  isolate the cost they claim to. Per this section's own rule — assert a cost claim
  only once it is measured *and* gated — nothing from those probes is reproduced or
  relied on above, and the Alt-A trade stays **open**. It is decided when `sq-uii0`
  lands the probe methodology plus a regression-gated `_comment_sq_uii0` snapshot
  entry; the verdict is written then, from that artifact. Whichever way it lands,
  Alt-A re-touches the audit-#1 public-input reconstruction, the empirical `bb`
  anchors (`verifier.rs:3582-3614`) and every `scan_k…` member, so it is a planned
  re-audit and never a drive-by.
- **Cardinality-hiding tier (`join_eq_rN`).** Proving R pairs in one member
  amortises the per-graph re-commit across R joins (the fold is paid once per
  graph regardless of R) and hides exact N up to R — likely the *better* shape for
  N>1 joins, but its gate count must be measured before recommending it as
  default.
- **Feasibility verdict: HIGH.** Every primitive (`commit_fold`, `h3`, `h2`,
  present-in-graph sweep, equality) is **already in-tree and compiled** in
  `scan.nr`. `join_eq` is assembled entirely from reused, already-benchmarked
  gadgets — no new cryptography. The actual `bb gates` figure MUST be measured and
  regression-gated (`gate_count.rs` + `gate_count_snapshot.json` precedent) before
  any number is published.

---

## 6. Relation to the MPC PSI join (`sq-t21`) — same goal, different trust model

| | This member (`sq-bwwl`) | MPC PSI join (`sq-t21`) |
|---|---|---|
| **Who holds the data** | ONE prover holds BOTH credentials | TWO mutually-distrusting holders, one each |
| **What is hidden, from whom** | join VALUE hidden from the **verifier** | each side's set hidden from the **other holder** AND the verifier |
| **Primitive** | single-prover ZK (in-circuit equality + commitment) | secure 2PC / circuit-PSI (oblivious, e.g. cuckoo-bin, `crates/sparq-mpc/src/join.rs`) |
| **Cost class** | ~one scan member (no scalar-mul) | minutes–tens-of-minutes for hidden-value joins (ORQ SOSP'25 cost centre, `sq-t21` notes); the *disclosed*-key MPC join is crypto-free |
| **When to use** | the holder legitimately has both credentials and wants to present a joined result without revealing the join key | the two sides are different parties who will NOT share plaintext with each other |

**Decision rule.** If **one party holds both credentials** → `join_eq` (this
design): cheap, single-prover, no interaction. If the **two sides are distinct,
mutually-private parties** → the MPC path (`sq-t21`): you fundamentally cannot use
single-prover ZK because there is no single party who can witness both sides. They
are complementary, not competing: `join_eq` is the single-prover specialisation
where MPC's machinery is unnecessary (`join.rs:36-38` makes the same point — the
hidden-value path is gated on a backend *because* it needs MPC; the single-prover
case does not). The architecture's "global-IRI-as-join-key" convention
(`join.rs:14-23`) applies to both; the difference is purely *who can see what*.

> Note: where the join KEY is a **disclosed** global IRI, neither member is needed
> — the join is a plaintext check (the MPC `DisclosedKeyJoin`, `join.rs:25-32`).
> `join_eq` is specifically for when the single prover wants the key **hidden from
> the verifier**.

---

## 7. Sequenced implementation plan → follow-up beads

Each step is a context-independent deliverable with its own forge-and-verify
tests; ordering respects deps. The follow-up beads (filed under `sq-bwwl`) are
listed with their step.

1. **[host crypto] Join-value hiding commitment + domain tag.** Add
   `JOIN_DOMAIN` (`"ZKSIG_JN"`) and a `join_value_commitment(value, blinding)`
   helper in `crates/sparq-zk/src/sig.rs` (mirroring `status_index_commitment`,
   the `revoke.nr:145` precedent), with a host↔Noir Poseidon2 cross-vector test.
   *(Foundation; no deps.)* → **bead step 1**
2. **[circuit] `join.nr` + `join_eq` member.** Implement the relation (§2.2)
   reusing `commit_fold`/`h3`/`h2`/present-in-graph from `scan.nr`/`hashes.nr`;
   add `zk/compose/join_eq/src/main.nr` with the public-input layout `[challenge,
   commit_a, commit_b, join_commitment, slot_a, slot_b]`; cross-vector test the
   in-circuit commitment vs the host. *(Deps: 1.)* → **bead step 2**
3. **[manifest] `CircuitId::JoinEq` + `ProofInputs::JoinEq` + `JoinEdge`.** Wire
   the schema in `manifest.rs` (id derive, `circuit_id()` arm, `join_edges` field).
   *(Deps: 2.)* → **bead step 3**
4. **[verifier] `bind_joins` + public-input reconstruction + canonical-vk + query
   binding.** Add the `bind_joins` stage, the `JoinEq` arm in
   `reconstruct_public_inputs`/`derive_id`/`canonical_vk`, the `UnboundJoin`
   query-correctness check, and the `JoinPolicy`; `CheckError` variants
   `JoinCommitmentMismatch`/`UnboundJoin`/`JoinSlotMismatch`. *(Deps: 3.)*
   → **bead step 4**
5. **[tests] Forge-and-verify regression suite.** One test per §4.5(4) failure
   mode in `crates/sparq-zk-compose/tests/` (extend `forge_*`/`e2e`/`verifier_errors`):
   non-equal terms rejected, swapped-graph rejected, unbound-query-join rejected,
   wrong-slot rejected, replay rejected. *(Deps: 4.)* → **bead step 5**
6. **[gates] Regression-gate the `join_eq` gate count** with `bb gates -s
   ultra_honk` (extend `gate_count.rs` + `gate_count_snapshot.json`); publish the
   measured number only after it lands. *(Deps: 2.)* → **bead step 6**
7. **[docs] README + SKILL.** Update `crates/sparq-zk-compose/README.md` (remove
   the "join consistency is verifier-side over disclosed rows" deferral for the
   hidden-key path) and the `verifiable-credentials-zk` skill. *(Deps: 4.)*
   → folded into step 4's bead acceptance (no separate bead).
8. **[optional v2] Cardinality-hiding `join_eq_rN`** (§2.5/§5) and **Alt-A
   scan-layout per-row commitments** (§2.6) — filed only if `bb gates` shows the
   re-commit dominates or cardinality leakage (R2) is a deployment blocker.
   → **bead step 7 (optional, P3)**.
   **Alt-A's trigger is not yet settled: it is gated on `sq-uii0`.** Whether the
   re-commit dominates is not established in-tree — no regression-gated measurement
   exists (§5), so the trigger above has neither fired nor been ruled out. Decide it
   from `sq-uii0`'s methodology + `_comment_sq_uii0` snapshot entry, and plan Alt-A
   as the scan-layout re-audit it is, never as a drive-by.

---

## 8. Open questions / honest limitations recap

- **R1/R2 are intrinsic.** A hidden-key join still reveals *that* a join exists and
  (by default) its cardinality. Hiding cardinality needs the fixed-R member (§2.5,
  step 7) and pays for padding. *That* a join was performed is unavoidable while
  answering the query.
- **The join SLOT is disclosed** (§4.4), because the query already reveals it. Only
  the join VALUE is hidden. A use-case needing a hidden *column position* is out of
  scope (and arguably contradicts presenting a query the verifier reads).
- **FILTER-over-joined-column is v2** (§3.4): v1 joins hide the join column and
  filters operate on other columns. A `filter_committed` member is the follow-up.
- **Gate count is unmeasured** by policy: §5 gives only a cost *class*; the figure
  must come from `bb gates` (step 6) before publication.
- **Bnodes are not valid cross-credential join keys** (§2.5) by the audit-#9 salt
  separation — the verifier rejects a cross-graph join on a bnode-typed slot. The
  global-IRI convention (`join.rs:14-23`) is the intended join-key domain.
