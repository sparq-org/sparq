<!-- [OPUS-5] Formal simulator sketch / paper-grade write-up for the sparq-mpc pipeline (bead
sq-aaop, parent #2629, sibling sq-wj4k, epic sq-pwr). This is the slice BOTH sibling records
explicitly reserve for sq-aaop: `mpc-composition-uc-posture.md` §6 ("if sq-aaop is retained as a
distinct slice it should cover the formal simulator sketch / paper-grade write-up rather than
re-state this posture") and `mpc-zkp-federated-sparql-design.md` §8 item 6 ("sq-aaop, if retained,
is the formal simulator-sketch/paper-grade write-up"). Doc-only, no code. It states the model,
writes out the ideal functionalities, fixes ONE analyzed entry point (`pipeline::run_federated`)
and gives the complete inventory of the opens REACHABLE FROM THAT PATH — with the other production
reconstruction surfaces excluded by name rather than implied to be covered — plus a per-open
simulator and its exact simulation quality, composes them by a hybrid argument with a concrete
error budget, and closes with the ledger of proof obligations that remain UNFILLED. It is NOT a proof of security and makes NO soundness claim. Date: 2026-07-28. -->

# Formal simulator sketch for the sparq-mpc pipeline

**Status:** design record, **doc-only** (no implementation). Bead **sq-aaop** (parent **#2629**;
sibling **sq-wj4k**; epic **sq-pwr** — MPC over federated SPARQL + ZKP of correctness +
attested-source derivation). Author tier: Opus 5.

**What this record is.** The composition/UC *posture* record
([`mpc-composition-uc-posture.md`](./mpc-composition-uc-posture.md), bead `sq-wj4k`) says *which*
composition theorems would apply to a future distributed realization and *what obligation* each
open imposes. It deliberately stops short of the formal write-up. **This** record is that write-up:
the model, the ideal functionalities spelled out, the **complete inventory of every value one
fixed entry point — `pipeline::run_federated` — opens** (§2.1 states the analyzed configuration
exactly; §2.4 names the production reconstruction surfaces that are *out* of that scope), a
simulator per open with its exact simulation quality (perfect / statistical, with the constant),
the hybrid argument that composes them, and — the part that matters most for honesty — an explicit
**ledger of what the sketch does not establish**.

**What this record is NOT.** It is not a proof of security, not a UC realization claim, and not a
statement that any sparq-mpc operator is secure. Everything below is stated for a *hypothetical
distributed realization*; the shipped code is an in-process, centrally-driven simulation (§1.3), so
several ingredients a real proof needs **do not exist to be simulated** (§5). Per the standing
honesty gate: sparq's MPC estate is **honest-majority, semi-honest only**, its ZK/MPC work is
**research-grade and not externally audited** (external accredited-cryptographer sign-off is
pending, `sq-qhy4`), and no production privacy/soundness claim is made here.

**One correction to the sibling record, made here rather than silently.** `sq-wj4k` §0's stage
table marks stage 4 ("Secure aggregate") as *"no (linear, local)"* and stage 5 as the only
mid-protocol reveal besides `secure_equal`. That is accurate for `MpcBackend::run_secure` (the
cumulative sum, which is genuinely local and opens nothing) but **understates the production
pipeline**: `pipeline.rs` step 4 also runs `compare::disclose_threshold_verdict`, which opens
**64 field elements** per verdict (§2). Those opens are individually well-masked, but each carries
its own simulation obligation, and one of them is *statistical*, not perfect. Enumerating them is
the central contribution of this record. `sq-wj4k` §3's `secure_equal` lemma covers **1** of the
**4 distinct open shapes** on the analyzed path. (Stage 5 itself is corrected in the other
direction: it is *not* a mid-protocol reveal either, because on this path nothing is reconstructed
at step 5 — see §2.2.)

---

## 1. The formal model

### 1.1 Algebra and parties

The field is `Fp` with `p = 2^61 − 1`, a Mersenne prime (`crates/sparq-mpc/src/field.rs:34`).
`p ≡ 3 (mod 4)` (concretely `≡ 7 mod 8`), which is what makes the public-square-root step of the
random-bit protocol work (§3.2). There are `n` parties `P_1..P_n` on the canonical evaluation
points `x = 1..n`, with threshold `t = ⌊(n−1)/2⌋` fixed by the honest-majority constructor
(`shamir.rs`), so `n = 2t+1` for odd `n` and `n = 2t+2` for even `n`. Write `[x]_d` for a
degree-`d` Shamir sharing of `x ∈ Fp`; `[x]` abbreviates `[x]_t`.

### 1.2 Adversary and communication model

Throughout: a **static, semi-honest** adversary `A` corrupting a set `C ⊆ {1..n}` with `|C| ≤ t`.
Honest majority holds by construction; the type-level `CorruptionThreshold::HonestMajority`
invariant in `backend.rs` is what enforces it, and dishonest majority is deliberately refused
(`sq-j5ok`), not silently supported.

The write-up assumes, as the **hybrid-model resources** the composition theorems require:
synchronous rounds; ideal **authenticated and private** point-to-point channels between every pair
of parties; broadcast where the sub-protocol needs it. **None of these are provided by the
codebase** — that is obligation O2/O4 in §5, not an oversight of the model.

Two simulation qualities are used. `VIEW ≡ SIM` means the distributions are **identical**
(perfect); `VIEW ≈_ε SIM` means their statistical distance is at most `ε`.

### 1.3 What the code actually is (ground truth, verified against this checkout)

This matters because it bounds what the sketch can claim:

- **The default execution is an in-process simulation.** One `ShamirDealer` plays every party:
  `dealer.share(secret)` deals from the *cleartext* value, and `degree_reduce` performs each
  party's re-sharing centrally (`shamir.rs`). There are no per-party processes, so there are no
  per-party views to simulate.
- **`transport.rs` is a measurement harness, not a distributed realization.** It does run `n` real
  OS processes over loopback TCP, and it is honest about what it is: a **star coordinator** deals
  shares out to the party processes and collects their contributions
  (`transport.rs`, "the coordinator deals shares out to the parties … exactly as the in-process
  dealer does, but now over a socket"). The coordinator therefore holds the cleartext and **is a
  trusted dealer**. The star gives real bytes-on-wire and real round latency; it does **not** give
  the mesh protocol whose views a standalone proof would simulate.
- **Every open on the semi-honest path is unauthenticated.** `auth_disclose.rs` / `auth_compare.rs`
  are the IT-MAC'd *malicious twins* of the same routines (the `sq-km34.*` line); the semi-honest
  path this record models does not MAC-check before opening.

So: the lemmas in §3 characterise the **public transcript** (the opened values) of each operator.
That is the ingredient an eventual per-party simulator needs, and it is genuinely all that can be
established today. It is not itself a realization proof (§5, O1).

---

## 2. The open inventory of the analyzed path

### 2.1 The analyzed protocol, fixed exactly

Everything in §3–§4 is about **one** entry point in **one** configuration. Naming it precisely is a
precondition for the hybrid argument to mean anything, because "the production path" is not a
single protocol — the crate exposes several reconstruction surfaces (§2.4) and a transcript
composed across a union of them would be a transcript of no executable protocol.

> **The analyzed protocol** is `pipeline::run_federated` — the four-flatmates federated response —
> over a `ShamirBackend` in the honest-majority semi-honest tier of §1.2, with the default
> `Disclosed` routing for the membership join and `Hidden` routing for the salary aggregate, i.e.
> the exact per-operator routing `run_federated` itself emits.

Its stages, as the code runs them: (1–2) each holder evaluates its disclosed fragment locally;
(3) `DisclosedKeyJoin::join` folds the disclosed-key membership join; (2+4) `share_private_input`
shares the private salaries, `run_secure` sums them, `compare::disclose_threshold_verdict` opens the
threshold bit; (5) the answer is assembled; (6) the `ProofStatement` is assembled around the
`NotYetImplemented` proof stub.

### 2.2 What is *not* opened on this path

Two clarifications, because both are places an inventory can silently over- or under-count:

- **Step 5 reconstructs nothing.** In `run_federated` step 5 is a comment, not a computation: the
  disclosed join result and the already-opened verdict bit *are* the federated answer, both
  computed earlier. There is no aggregate reconstruction at step 5, and this path never calls
  `MpcBackend::reconstruct_disclosed`. An earlier draft of this record listed such an open (shape
  "E"); it is not on this path and has been removed. `reconstruct_disclosed` is a real production
  API — it is simply reached by other callers (§2.4).
- **The disclosed join result is public without ever being an open.** It comes from
  `DisclosedKeyJoin::join` over the holders' locally-evaluated fragments: crypto-free, outside the
  cryptographic core, never secret-shared. Its disclosure is a *routing* decision (leak L1, the
  disclosed-key surface), governed by the `F_join`-style bookkeeping in §3.5 — not a transcript
  element with a simulator obligation, since no shares are ever brought together.

### 2.3 The inventory

An "open" is any point where shares are brought together and a field element becomes public. This
is the complete list of opens **reachable from `run_federated`**:

| # | Open | Site | Degree | What becomes public | Simulation quality |
|---|------|------|--------|---------------------|--------------------|
| **A** | masked product `m = d·r`, `r` uniform **nonzero** | `compare.rs::secret_is_zero` (via `verify_value_in_range_rabbit`) | `2t` | one field element per test | **perfect**, from the zero/nonzero bit (§3.1) |
| **B** | square `c = a²`, `a` uniform nonzero | `compare.rs::square_protocol_random_bit` | `2t` | one uniform nonzero quadratic residue | **perfect**, independent of the emitted bit (§3.2) |
| **C** | Rabbit masked open `c = (x + r) mod p` | `compare.rs::secure_bit_decompose_rabbit` | `t` | one near-uniform field element | **statistical**, `ε ≤ 2^{−61}` (§3.3) |
| **D** | verdict bit | `compare.rs::open_verdict` | `t` | the 0/1 output | **it is the output** (§3.4) |

All four sit inside `disclose_threshold_verdict`; stages 1–3, 5 and 6 open nothing, and
`run_secure` (the cumulative sum) opens **0** — it is the zero-round local share-addition.

Everything else in the comparator is multiplication-only: `secret_and`, `secret_and_not`,
`secret_bit_eq`, `greater_than_public_bits_with`, `rabbit_lt_bits_public_less_than_shared`,
`rabbit_add_public_and_w_times_const`, `rabbit_sub_shared_bits` all reduce to
`mul_shares_raw` + `degree_reduce`, and **`degree_reduce` opens nothing** (it re-shares each
degree-`2t` share under a fresh degree-`t` polynomial and recombines with public Lagrange scalars).
This is why the inventory is short despite the comparator being deep.

**Per-invocation counts** (the numbers the §4 error budget uses). One `run_federated` call performs
exactly one `disclose_threshold_verdict` run, which opens **64** elements: **61** × B (one per mask
bit, since `deal_full_field_solved_bits` draws `RABBIT_MASK_BITS = 61` bits and each costs one
square-protocol open) + **1** × C (the Rabbit masked open) + **1** × A (the `secret_is_zero` inside
`verify_value_in_range_rabbit`) + **1** × D (the verdict). So a `run_federated` execution has 64
opens, full stop.

**How to reproduce this enumeration.** It is a call-graph walk, not a repo-wide grep, and it is
scoped to the entry point above:

1. Enumerate the candidate reconstruction primitives — `shamir::reconstruct_degree`,
   `shamir::reconstruct_at_zero`, `robust::reconstruct_robust`, `MpcBackend::reconstruct`,
   `MpcBackend::reconstruct_disclosed` — and grep their call sites under `crates/sparq-mpc/src/`.
2. Discard sites inside `shamir.rs` / `robust.rs` themselves (the primitives) and inside
   `#[cfg(test)]` regions.
3. **Keep only what the call graph rooted at `pipeline::run_federated` reaches.** That root reaches
   `holder::evaluate_local`, `DisclosedKeyJoin::join`, `share_private_input`, `run_secure`,
   `compare::disclose_threshold_verdict` (→ `deal_full_field_solved_bits` →
   `square_protocol_random_bit`; `secure_bit_decompose_rabbit`; `verify_value_in_range_rabbit` →
   `secret_is_zero`; `greater_than_public_bits_with`; `open_verdict`) and `verdict_from_partial`
   (which only parses an already-opened literal). The surviving sites are exactly A–D.

### 2.4 Production surfaces deliberately EXCLUDED from this scope

These are real, non-test production reconstruction sites that step 3 discards because
`run_federated` does not reach them. They are listed rather than implied-covered, because the
earlier repo-wide "every call site outside `shamir.rs` and `#[cfg(test)]`" phrasing was simply
false. **Each carries its own unfilled simulation obligation; none is analyzed here.**

- **`backend.rs::reconstruct_disclosed`** (impl in `shamir.rs`; production caller in `bench.rs`) —
  the disclosed-output API for a `Hidden`-routed aggregate. Where it *is* used, the opened value is
  the defined output of the functionality, so its simulator is the §3.4 trivial one; but it is an
  alternative output path, not a stage of `run_federated`.
- **`join.rs::HiddenValueJoin::join` / `secure_equal_shared`** — a shape-A open per candidate pair,
  `|L|·|R|` of them, which is leak **L2** (pinned by
  `join.rs::secure_equal_leaks_full_bipartite_match_graph`). Reached by the hidden-key join API;
  `run_federated` routes its join `Disclosed` and never calls it. Lemma A in §3.1 is stated
  generally enough to cover the individual open, but the *composition* for that API — in particular
  the match-graph leakage its `F_join` must carry (§3.5) — is not done here.
- **`hidden_distinct.rs::row_key`** — two degree-`t` opens per sort row, inside the in-process
  simulation.
- **`batched.rs::BatchedShares::reconstruct`** — element-wise opening of a disclosed batched result.
- **`oblivious_join.rs`** (tag open at degree `2t`, limb opens at degree `t`) and
  **`oblivious.rs`**'s robust compare-exchange opens.
- **`transport.rs`** — several degree-`t` / degree-`2t` opens in the star-coordinator measurement
  harness, which §1.3 and §5 O4 already exclude as not a distributed realization.
- **`auth_compare.rs` / `auth_disclose.rs`** — the IT-MAC'd malicious twins (§5 O5). A different
  adversary model with a different (check-then-open) discipline; out of the semi-honest scope.

So: this record analyzes `run_federated`, **not** "all production semi-honest APIs". Extending it to
any surface above means redoing §2.3's walk from that surface's own entry point and re-running §4's
budget over the resulting count.

---

## 3. The simulators, per open

Each lemma below simulates the *opened value* given the corresponding ideal functionality's output
and leakage. Each is stated with its precondition, because the preconditions are load-bearing.

### 3.1 Lemma A — the masked-product open is perfectly simulatable from the equality bit

**Functionality.** `F_eq` on inputs `[a]`, `[b]` outputs the bit `β = 1{a = b}` and leaks `β`.
(For `secret_is_zero`, `F_zero` on `[v]` outputs `β = 1{v = 0}`.)

**Protocol.** Draw `r ← Fp \ {0}` uniformly and independently; compute `[d] = [a] − [b]`;
`[m]_{2t} = [d]·[r]`; open `m`; return `m = 0`.

**Lemma.** Conditioned on `β`, the opened `m` is distributed as: `m = 0` deterministically if
`β = 1`; uniform on `Fp \ {0}` if `β = 0`. Hence the simulator `S_A(β)` — output `0` if `β = 1`,
else sample uniform nonzero — produces a distribution **identical** to the real opened value.
*Proof.* If `d = 0` then `d·r = 0` for every `r`. If `d ≠ 0` then `x ↦ d·x` is a bijection on
`Fp \ {0}`, so `d·r` is uniform on `Fp \ {0}` when `r` is. ∎

**Preconditions, and why each bites.**
1. **`r ≠ 0`.** A zero mask opens `m = 0` for unequal operands — a *false match*, i.e. a
   correctness failure, not only a simulation failure. `draw_nonzero_fp` rejects zero, and both
   call sites document this as load-bearing.
2. **`r` fresh and independent per open.** Reusing one `r` across two tests of `d₁`, `d₂` opens
   `m₁, m₂` with `m₁/m₂ = d₁/d₂` — a *ratio of secrets*, which `S_A(β₁, β₂)` cannot produce. Both
   call sites draw a fresh `r` inside the routine, so this holds; it is recorded because a future
   "hoist the mask out of the loop" optimisation would silently break the lemma.
3. **`r` unknown to any party.** In the in-process code the dealer draws `r` in the clear
   (obligation O2).

**Scope.** This is a *distribution/leakage lemma about the opened value*. It is **not** a claim
that `secure_equal` realizes `F_eq`: that would additionally require simulating a corrupted party's
whole view (its shares of `a`, `b`, `r`, `d`, `m`), which needs a distributed protocol that does
not exist (§5, O1). `sq-wj4k` §3 states the same scope limit; this record does not widen it.

### 3.2 Lemma B — the square-protocol open is perfectly simulatable and independent of the bit

**Functionality.** `F_randbit` outputs a fresh `[b]` with `b` uniform on `{0,1}`, known to nobody,
and leaks nothing.

**Protocol** (`square_protocol_random_bit`). Draw `a ← Fp \ {0}`; open `c = a²` at degree `2t`;
compute the public root `d = c^{(p+1)/4}` (valid because `p ≡ 3 mod 4`) and check `d² = c`
fail-closed; then `[s] = d^{-1}·[a] ∈ {+1,−1}` and `[b] = (s+1)·2^{-1}`.

**Lemma.** `c` is uniform on the set `QR*` of nonzero quadratic residues and is **independent of
`b`**. So `S_B()` — sample `a' ← Fp \ {0}`, output `a'²` — is a perfect simulation.
*Proof.* `a ↦ a²` maps `Fp \ {0}` two-to-one onto `QR*` (`a` and `−a` collide, and `a ≠ −a` since
`p` is odd), so `c` is uniform on `QR*`. Conditioned on `c`, the preimage `{+d, −d}` is equiprobable,
and `b` is exactly the indicator of which preimage `a` is; hence `b | c` is uniform on `{0,1}` and
independent of `c`. ∎

**Note on the retry branch.** The code retries when `c = 0`, and its comment prices that at
`1/p ≈ 2^{−61}`. Because `a` is drawn by `draw_nonzero_fp`, `a ≠ 0` always, so `c = a² ≠ 0` always
and **the retry branch is unreachable on the honest path**. That is good for the sketch — the
simulator has no retry-count distribution to reproduce, so `S_B` is a single unconditional sample.
(The branch is correct defensive coding; only its stated probability is loose.)

**Why this open is the interesting one.** `c` is opened at degree `2t` and is *not* masked by a
secret — it is a raw product open. It is safe because the *secret it derives from is itself fresh
protocol randomness with no relation to any party input*. That is a genuinely different safety
argument from Lemma A's, and it is the reason `deal_full_field_solved_bits` can afford 61 of them.

### 3.3 Lemma C — the Rabbit masked open is *statistically* simulatable, with `ε ≤ 2^{−61}`

**Functionality.** `F_decomp` on `[x]` with `x < 2^{60}` outputs fresh sharings of `x`'s
`RABBIT_VALUE_BITS = 60` bits and leaks nothing.

**Protocol** (`secure_bit_decompose_rabbit`). Deal solved bits `([r], [r]_B)` with
`r = Σ_{k<61} b_k 2^k` for uniform independent bits `b_k` (each from Lemma B); **open
`c = (x + r) mod p`** — the only opening; recover the wrap indicator `w = 1{c < r}` by a
public-vs-shared bitwise comparison and reconstruct `[x]_B` by bit circuits, all
multiplication-only.

**Lemma.** Let `N = 2^{61}` and `p = N − 1`. Then `Δ( (r mod p), U(Fp) ) = (N−2)/(N(N−1)) < 1/p`,
and since `r` is independent of `x`, `Δ(c, U(Fp)) ≤ (N−2)/(N(N−1)) < 2^{−61}`. So `S_C()` —
sample uniformly from `Fp` — is a simulation with error `ε_C < 2^{−61}`, **independent of `x` and
of the magnitude of `x`**.
*Proof.* `r` is uniform on `[0, N)`, which is exactly one element wider than `p`, so after
reduction `Pr[r ≡ 0] = 2/N` and `Pr[r ≡ k] = 1/N` for each of the other `p − 1 = N − 2` residues.
Then `Δ = ½( |2/N − 1/p| + (N−2)·|1/N − 1/p| ) = ½( (N−2)/(N(N−1)) + (N−2)/(N(N−1)) )
= (N−2)/(N(N−1))`. Adding the independent `x` is a bijection on `Fp` and preserves the distance. ∎

This **confirms** the bound the code's own doc-comment asserts (`≤ 1/p ≈ 2^{−61}`) and shows it is
in fact very slightly conservative. It is the only *non-perfect* simulator on the production path,
and it is the one that forces §4's error budget to be additive rather than trivially zero.

**Precondition, and how it is discharged.** Exact recovery requires `x < 2^{60}` — otherwise the
returned low bits truncate the value and the verdict is silently wrong. This cannot be checked on a
sharing without disclosing it, so it is discharged **in-protocol**: `verify_value_in_range_rabbit`
recomposes `Σ b_k 2^k` locally (free) and runs one Lemma-A zero-test on `x − Σ b_k 2^k`, aborting
fail-closed if nonzero. Two consequences for the model: (i) the precondition is a *proved* clause,
not an assumption; (ii) the in-range bit is therefore **an output of the functionality** — see
§3.5. (The test-only masked-open twin `secure_bit_decompose` has a weaker `2^{−40}` gap and an
*unchecked* magnitude precondition; it is off the production path and is modelled only as the
semi-honest reference for the `auth_disclose` malicious twin.)

### 3.4 Lemma D — the output open

`open_verdict` reconstructs `[β]_t` and refuses to coerce anything that is not exactly `0` or `1`.
`β` is a **defined output** of `F_thresh`, so the simulator receives it from the functionality by
definition and outputs it verbatim. There is nothing to prove — but there *is* something to state:
an open is only "free" in this sense if the functionality really does define it as an output. §3.5
is where that bookkeeping is done, and it is where an understated functionality would hide a leak.

The same trivial argument applies to `reconstruct_disclosed` wherever it *is* used (§2.4) — its
opened value is by construction the disclosed output. It is noted here only so the shape is not
lost; it is off the analyzed path and contributes no term to §4's budget.

### 3.5 The functionalities, written out with their leakage

The whole point of the exercise is that **each open must be accounted for in the functionality the
next stage composes against**. Writing them out:

- **`F_share`**: on input `x` from `P_i`, deliver `[x]_t`. Leaks nothing. (Realization: obligation
  O2 — the code deals from cleartext.)
- **`F_mult`**: on `[a]`, `[b]`, deliver a fresh `[ab]_t`. Leaks nothing. (Realized by
  `mul_shares_raw` + `degree_reduce` in the hybrid model; per-party view simulation is O3.)
- **`F_randbit`**: deliver `[b]`, `b` uniform on `{0,1}`. Leaks nothing (Lemma B).
- **`F_eq`** / **`F_zero`**: deliver the equality/zero **bit**, and **leak that bit** (Lemma A).
- **`F_decomp`**: deliver `[x]_B`; leaks nothing beyond a `2^{−61}` statistical slack (Lemma C).
- **`F_thresh`**: on `[x]` and a public `τ`, output **two** bits — `1{x < 2^{60}}` (the in-range
  clause, which aborts the protocol when false) and `1{x > τ}`. **Leaks both.** In an honest
  execution with an in-range aggregate the first is deterministically `1`, so it adds no entropy;
  it must still appear in `F_thresh`, because a downstream stage composing against a
  single-bit-output `F_thresh` would be composing against a functionality the real protocol does
  not realize.
- **`F_join`** (hidden-value; **off the analyzed path** — stated because §2.4 excludes the API and
  an excluded surface should still have its leakage on record): output the join, and **leak the
  full bipartite match graph** over all `|L|·|R|` pairs (leak L2) — *not* merely the output
  multiset. The fully-oblivious variant
  (`fully_oblivious_batched_join`, built on `secure_equal_to_bit`) keeps the per-pair bit
  secret-shared and never opens it, closing L2 **at the decision**; its `F_join` therefore carries
  only the output-level leakage its oblivious output bounds, not the match graph. It is the
  composition-cleanest of the join tiers and is what to prefer wherever the per-pair bit must not
  surface.
- **`F_prove`**: §6.

---

## 4. Composing the simulators — the hybrid argument and the error budget

Fix an execution of `run_federated` — equivalently, of the `disclose_threshold_verdict` inside it,
since §2.3 shows that is the only stage that opens anything. Define hybrids `H_0 .. H_64` where
`H_0` is the real transcript and `H_j` replaces the `j`-th open (in execution order) with its
simulated value.
Adjacent hybrids differ in exactly one opened element, so
`Δ(H_{j−1}, H_j) ≤ ε_j` with `ε_j` the simulation error of that open's lemma, and by the triangle
inequality `Δ(H_0, H_64) ≤ Σ_j ε_j`. With the §2 counts:

```text
ε_total(one verdict)  ≤  61·0  (Lemma B, perfect)
                       +  1·(N−2)/(N(N−1))   (Lemma C, N = 2^61)
                       +  1·0  (Lemma A zero-test, perfect)
                       +  0    (Lemma D — the output itself)
                      <  2^{-61}
```

So the public transcript of a threshold verdict is simulatable within `2^{−61}` — the field-size
floor, **independent of the aggregate's magnitude**. Over `Q` verdict queries the bound degrades
additively to `Q · 2^{−61}`, which is the right way to state a per-federation budget: it is
generous, but it *is* a budget, and it is the reason the Rabbit path (`2^{−61}`, uncoupled from
value width) is the correct production choice over the masked-open path (`2^{−40}`, coupled to a
20-bit value cap).

Because `disclose_threshold_verdict` is the only opening stage of `run_federated` (§2.3), that
bound is also the bound for the whole `run_federated` execution — there is no further term to add
for stage 5 or for the disclosed join, which open nothing.

**An aside, off the analyzed path.** For the hidden-value join API (§2.4) the transcript would be
simulated **perfectly** (`ε = 0`, all Lemma A), but only relative to the **match-graph-leaking**
`F_join`. Perfect simulation against a leaky functionality is not a privacy result; it is a
statement that the protocol leaks *exactly* what the functionality says and nothing more. Keeping
`F_join` honest is the whole content. This is recorded as an observation, not composed into the
budget above: `run_federated` does not route its join `Hidden`.

**The two composition steps this supports, and the one it does not.** Given (i) a distributed
realization of each stage and (ii) the per-stage functionalities above with their leakage,
**sequential modular composition** (Canetti J. Cryptology 2000; Goldreich Vol. 2 §7) carries the
pieces into `run_federated`'s straight-line pipeline, because the pipeline is single-session and
strictly sequential. **Concurrent composition is not addressed**: nothing here is a UC statement.
Honest majority keeps UC-without-CRS/PKI *reachable* for a future networked realization (Canetti
FOCS'01, given that framework's channel/broadcast resources) — that is `sq-wj4k`'s argument for the
default, and it remains aspirational, not established.

---

## 5. Ledger of unfilled obligations — what this sketch does **not** establish

This is the honest core of the record. Each item is a concrete gap between the lemmas above and a
standalone-security proof.

- **O1 — No per-party view is simulated.** §3 simulates the *public transcript*. A standalone proof
  must simulate `VIEW_C` for a corrupted set `C`: their share vectors, their `degree_reduce`
  re-sharing messages, their randomness tapes. Shamir's `t`-privacy makes this routine in principle
  (any `t` shares of a degree-`t` sharing are uniform and independent of the secret), but it is
  **not written**, and it is the difference between "the opened values leak nothing extra" and "the
  protocol realizes `F`". Every lemma above is scoped accordingly.
- **O2 — Trusted dealer.** `ShamirDealer::share` deals from cleartext; the mask `r` in Lemmas A and
  C is drawn by the one process that plays all parties. A deployment needs distributed input
  sharing (VSS) and dealer-less randomness (PRSS / `sq-yyro`). Until then **there is no sharing
  protocol to simulate** — `F_share` has no candidate realization.
- **O3 — `degree_reduce` view simulation.** The BGW/GRR re-share-and-recombine round is executed
  centrally. Its per-party messages (each party's fresh degree-`t` re-sharing of its degree-`2t`
  share) are the bulk of the real transcript in the comparator, and none of them are modelled here.
- **O4 — `transport.rs` does not close O1–O3.** Its star coordinator deals shares, so it is a
  trusted dealer with a socket. It measures bytes and rounds honestly; it does not realize the mesh
  protocol.
- **O5 — Every semi-honest open is unauthenticated.** At the minimal honest-majority `n = 2t+1`, a
  degree-`2t` open (shapes A and B) carries **zero Reed–Solomon redundancy**, so a forged product
  share is information-theoretically undetectable and can flip a match verdict or a range check;
  at even `n = 2t+2` there is exactly one redundant share, giving **detect-and-abort, never
  correct** (`join.rs`, `sq-ji5f`). Simulation-based *malicious* security needs the IT-MAC
  check-then-open line (`authenticated.rs` / `auth_disclose.rs`, `sq-km34.*`), which is the
  MPC-layer analogue of §6's validate-before-prove.
- **O6 — Abort behaviour is not modelled.** `verify_value_in_range_rabbit` and `open_verdict` both
  abort fail-closed on a malformed value. Under a malicious adversary, *which* inputs cause an
  abort is itself a channel (selective-failure leakage). The semi-honest model does not see it; a
  malicious treatment must, and must state the abort's leakage in the functionality.
- **O7 — Out of model entirely.** Engine and network timing side channels, memory-access patterns,
  and the planner's routing choices (`pipeline.rs`'s per-operator disclosed-vs-hidden decision) are
  all outside this analysis. The routing decision in particular is data an *untrusted* planner
  produces, and its influence on what is disclosed is a policy question, not a simulation one.

---

## 6. The collaborative-proof stage: validate-before-prove as a functionality-level requirement

This is where composition reasoning earns its keep, and it is the second item the bead names.

**Setup.** The intended stage 6 has the `N` holders act as co-provers: each holds a private witness
(its committed graph, its row encodings, its salary), they run an honest-majority MPC to compute
the correctness relation and to jointly emit **one** proof verifiable by the unchanged
single-prover verifier. Per **eprint 2025/1026** (Garg–Goel–Jain–Roberts–Sekar, CRYPTO'25), a
co-prover that proves over an **inconsistent / maliciously extended** witness can **leak the honest
provers' inputs** through the proving transcript and its induced openings — *even when the verifier
rejects the resulting proof*.

**Why this is precisely a composition failure, in the language of §3–§4.** Consider the naive
functionality `F_prove`: "take the shared extended witness `w`, output a proof `π` for the relation,
leak nothing about honest inputs." For the composed protocol to realize `F_prove`, a simulator must
produce the corrupted co-prover's view — including the values the proving MPC opens — from
`F_prove`'s output alone. Under an adversarially extended `w`, those induced openings are functions
of the **honest** parties' witness bits, and `F_prove`'s output (a proof, or `⊥`) does not determine
them. **No such simulator exists.** So the proving stage does not realize `F_prove`, the hybrid
step for stage 6 fails, and the pipeline's sequential composition breaks at exactly that seam.

Note what this refutes: "the SNARK is zero-knowledge" is a statement about the *proof object* under
an honestly-generated witness. It is precisely the kind of local, stand-alone reasoning a
composition treatment invalidates — ZK of `π` does not imply the *protocol that produces `π`* leaks
nothing, once the witness can be adversarially extended between the MPC output and the prover input.

**The fix, stated as a functionality.** Replace `F_prove` by `F_prove^{val}`:

> On the shared extended witness `w`, first evaluate a consistency predicate `Valid(w)` — each
> holder's contribution is a well-formed sharing consistent with its committed `C(G_i)` and with
> the cross-holder join. **If `¬Valid(w)`, output `⊥` to all parties and nothing else; in
> particular open no value derived from `w`.** Only if `Valid(w)` proceed to produce `π`.

In the invalid branch the entire observable is a single abort symbol, which is trivially
simulatable; in the valid branch the witness is by definition consistent with the honest inputs, so
2025/1026's leak path is not entered. `F_prove^{val}` is therefore realizable in principle, and
`F_prove` is not — which is exactly why the fix is *architectural* (an ordering constraint on the
protocol) rather than a parameter choice. Requirement **R-WV** in
[`mpc-cozk-reaudit.md`](./mpc-cozk-reaudit.md) §3 is this, in implementation terms; the key clause —
"no prove-anyway-and-let-the-verifier-reject path may exist" — is the statement that the invalid
branch must have **no** observable other than the abort.

**Mapping the encoded tests to the simulation obligations.** The obligation is already encoded (not
merely documented) in `crates/sparq-mpc/src/witness_validation_tests.rs` (bead `sq-7leq`). Each
test pins one clause of the argument above:

| Test | Simulation obligation it pins |
|------|-------------------------------|
| **T1** — inconsistent-share abort-before-open | the invalid branch's observable is *only* `⊥`: zero open-rounds, zero proof-commitments after the inconsistency |
| **T2** — witness-extension leakage probe | the pre-abort transcript is independent of honest-derived values (zero openings on their lineage) |
| **T3** — validation is load-bearing, not advisory | the validation genuinely precedes the first open in the hybrid's ordering — a refactor cannot silently reorder them |
| **T4** — commitment-binding of the validated witness | the witness that `Valid` accepted is the witness `π` is over; otherwise the hybrid substitutes a different witness at the binding seam |
| **C** — construction provenance | the adopted construction is a 2025/1026-*patched* variant, not a naive semi-honest→malicious compiler |

**Status, stated plainly.** The obligation is **OPEN, not met**. Every `CollaborativeProof` method
returns `MpcError::NotYetImplemented`, so there is no prover and nothing to validate; T1–T4 are
`#[ignore]`d against a `WitnessValidatingProver` contract the future implementation must satisfy,
with one passing meta-test pinning that the deferred prover never proves over any witness. **That
fail-closed state is the correct composition posture**: since validate-before-prove is not
enforceable, the honest move is to not compose the proof stage at all. No collaborative-proving
soundness or attestation claim may be made until the path is built, R-WV is enforced with T1–T4 + C
passing un-ignored, the honest-majority malicious-security line has landed, and an **external**
cryptographer audit covers the **multi-prover** construction — `sq-qhy4` audits the single-prover
verifier only and does **not** discharge this.

---

## 7. What a paper-grade proof would still need

In dependency order, the minimum viable set:

1. **A distributed protocol to prove things about** — replace the trusted dealer with VSS/PRSS input
   sharing and a party mesh (O2, O4). Until this exists, every statement is about a hypothetical.
2. **Per-party view simulators** for `F_share`, `F_mult` (including `degree_reduce`), `F_randbit`
   (O1, O3). Standard BGW/GRR arguments; the work is writing them for *sparq's* concrete parameters.
3. **Compose §3's transcript lemmas with (2)** into per-operator standalone-security statements for
   `F_eq`, `F_decomp`, `F_thresh`, `F_join`, each carrying its leakage explicitly.
4. **The sequential modular composition instance** for `run_federated`, with the §4 error budget
   carried through and the per-federation query bound `Q · 2^{−61}` stated as a deployment parameter.
5. **The malicious tier**: redo (2)–(4) against a malicious adversary with the IT-MAC
   check-then-open discipline, and model abort/selective-failure leakage (O5, O6).
6. **`F_prove^{val}`**: build the collaborative prover, enforce R-WV, un-ignore T1–T4, satisfy C,
   and obtain the multi-prover external audit (§6).
7. **Only then**, if wanted, the UC treatment — which additionally requires fixing the session and
   channel model and re-proving under concurrent composition.

A machine-checkable version of (2)–(4) is a heavier, separate, audit-gated deliverable and is not
proposed here.

---

## 8. Honesty caveats

- **This is a sketch, not a proof.** It constructs simulators for the *opened values* of every open
  reachable from `pipeline::run_federated` and composes them by a hybrid argument. It does not
  construct per-party view simulators, and it therefore establishes **no** realization of any
  functionality.
- **Its scope is one entry point, not the crate.** `run_federated` in the §2.1 configuration is the
  analyzed protocol. The other production reconstruction surfaces — `reconstruct_disclosed`, the
  hidden-value and oblivious joins, `hidden_distinct`, batched opens, `transport.rs`, and the
  authenticated malicious twins — are listed in §2.4 as **excluded**, not covered. Nothing here
  should be read as an inventory of "all sparq-mpc opens".
- **No security claim is made** about any sparq-mpc operator. sparq's MPC estate is
  **honest-majority, semi-honest only**; its ZK/MPC work is **research-grade and not externally
  audited** (external cryptographer sign-off pending, `sq-qhy4`).
- **The one quantitative result** — `ε < 2^{−61}` per threshold verdict — is a bound on the
  *simulation error of the public transcript* under the stated model, and it confirms rather than
  extends the bound the code already documents. It is not a privacy guarantee for a deployment.
- **The malicious-opener hole is real** and is closed by the IT-MAC line, not by this record.
- No performance numbers appear here, per the house rule.

---

## References

Canetti, *Security and Composition of Multiparty Cryptographic Protocols*, J. Cryptology 2000
(sequential modular composition). Canetti, *Universally Composable Security*, FOCS'01. Goldreich,
*Foundations of Cryptography Vol. 2* §7. Shamir, *How to Share a Secret*, CACM 1979.
Ben-Or–Goldwasser–Wigderson STOC'88 and Gennaro–Rabin–Rabin PODC'98 (the degree-reduction round).
Damgård–Fitzi–Kiltz–Nielsen–Toft, TCC'06 (solved bits / bit-decomposition). Makri–Rotaru–Vercauteren
–Wagh, *Rabbit: Efficient Comparison for Secure Multi-Party Computation*, eprint 2021/119 (the
exact-wrap-recovery decomposition). Goyal–Song, eprint 2020/134 (honest-majority malicious
security). Garg–Goel–Jain–Roberts–Sekar, *Malicious Security in Collaborative zk-SNARKs: More than
Meets the Eye*, eprint 2025/1026 (CRYPTO'25).

In-crate ground truth: `crates/sparq-mpc/src/{field,shamir,join,compare,pipeline,proof,transport,
authenticated,auth_disclose,witness_validation_tests}.rs`.

Companion records: [`mpc-composition-uc-posture.md`](./mpc-composition-uc-posture.md) (the
composition/UC posture — which theorems apply and the per-stage obligations),
[`mpc-cozk-reaudit.md`](./mpc-cozk-reaudit.md) (the adversarial coZK re-audit and R-WV / T1–T4),
[`mpc-malicious-security-design.md`](./mpc-malicious-security-design.md) (the IT-MAC upgrade),
[`mpc-security-models-and-benchmarks.md`](./mpc-security-models-and-benchmarks.md) (the
adversary/guarantee/threshold taxonomy and the leakage catalogue),
[`mpc-sparql-capability-matrix.md`](./mpc-sparql-capability-matrix.md) (the L1/L2 leak taxonomy),
[`mpc-bounded-property-path-design.md`](./mpc-bounded-property-path-design.md) (the no-mid-chain-open
operator family). Gates: `sq-qhy4` (external audit, single-prover only), collaborative proof
`NotYetImplemented` fail-closed.
