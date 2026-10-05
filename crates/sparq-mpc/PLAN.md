<!-- [OPUS-4.8] sparq-mpc build plan. Blueprint: research/mpc-zkp-research-and-architecture.md. -->
# sparq-mpc — Build Plan (MPC over federated SPARQL, RQ2)

Blueprint: `research/mpc-zkp-research-and-architecture.md`.
ZK foundation it sits on: `research/zk-soundness-audit.md`.

**Goal (Jesse's):** widest query coverage, MINIMISE inter-node data sharing,
best performance, ZKP of (a) result correctness AND (b) the authoritative /
attested source. Privacy-first.

**The two design forks everything fork-dependent waits on:**
- **Q1 (research risk):** distributed signature / commitment-opening inside the
  collaborative proof over a *secret-shared* witness — unsolved in the
  literature. "The join nobody has built." (architecture §5.2 Q1)
- **Q2 (trust model):** honest-majority vs dishonest-majority — reshapes the MPC
  primitive, round structure, and preprocessing. (architecture §5.2 Q2)

The honest feasibility verdict (architecture §5.3): the *viable* first regime is
honest-majority, LAN/datacenter, small committed datasets (≤10³–10⁴ triples/
party), few-pattern BGPs, disclosed-property aggregates recomputed by the
verifier. Anything beyond that (dishonest-majority malicious on a WAN, heavy
BGP joins) is unquantified research risk — never budget it as "seconds".

---

## Milestones

### M0 — Scaffold + local sub-evaluation  ✅ (this milestone)
Invariant structure only; no MPC primitive, no collaborative proof.
- Crate `crates/sparq-mpc` (native-only; NOT in the wasm build — verified via
  `cargo tree -p sparq-wasm`).
- Module structure mapping to architecture sections (`backend`, `holder`,
  `join`, `partial`, `proof`).
- **REAL, tested:** per-holder local SPARQL sub-evaluation (`holder.rs`) — each
  holder evaluates a fragment over its OWN graphs via `sparq-engine` and ships
  only the disclosed partial. This is invariant to Q1/Q2.
- Trait boundaries (interfaces + docs only): `MpcBackend`, `GlobalJoin`,
  `CollaborativeProof` / `Attestation`. All crypto methods → honest
  `MpcError::NotYetImplemented` naming the gate.
- This plan; `STATUS.md`.

### M1 — ZK foundation  ⛔ HARD DEPENDENCY (gating, not optional)
The MPC proof layer is MEANINGLESS until the RQ1 verifier-soundness remediation
lands (architecture §5.1). Cross-ref `research/zk-soundness-audit.md`:
- **#3** issuer-signature + key-set membership (the attestation half is #3
  generalised to multiple sources — cannot be built before #3 lands single-
  source).
- **#4** replay/freshness binding (verifier-issued nonce bound into public
  inputs).
- **#5 / #6** FILTER operator/bound/verdict + operand-slot binding (the
  £25k/£100k threshold IS the FILTER — directly load-bearing for the use case).
- **#8 / #9** per-row source-graph attribution in-circuit + salt separation (the
  inter-source disclosure control RQ2a depends on).
- **#12** revocation (a revoked credential must not feed a federated aggregate).

(#1/#2 — public-input reconstruction + canonical-vk pinning — already fixed on
main; the proof layer plugs into that seam.)

**Gate:** do not start M3/M4 until #3/#4/#5/#6/#8/#9/#12 are closed AND have
negative e2e tests.

### M2 — `GlobalJoin` protocol  ✅ disclosed-key DONE; hidden-value now in M3
Join holders' partials on GLOBAL IRIs (architecture §2 #6, §4.3 step 4).
- **DONE ✅ — the disclosed-key equi-join** (`JoinPlan::key_disclosed == true`):
  [`DisclosedKeyJoin`] in `src/join.rs`. A crypto-free, plaintext equi-join over
  the disclosed global-IRI key, computed OUTSIDE the cryptographic core
  (convention #4). Invariant to Q1/Q2 — needs NO MPC primitive because a global
  IRI is a stable public cross-holder identifier. [OPUS-4.8]
  - **Semantics:** a faithful SPARQL inner join under PAG *compatible-mappings*
    over ALL shared columns (not just the planner-named key), folded left-to-
    right; output is the union schema, rows canonicalised so the disclosed
    multiset is order-independent.
  - **Soundness (§4.1):** does NOT trust the untrusted planner — independently
    verifies the named `join_var` is projected by every partial (else a
    `Protocol` error, never a silent empty join) and enforces compatibility on
    every shared var so a malicious plan cannot induce a result disagreeing with
    PAG eval over the union.
  - **Tested (`cargo test -p sparq-mpc`):** DIFFERENTIAL — the federated join of
    per-holder partials *equals* evaluating the whole query over the UNION of the
    holders' graphs in one `sparq-engine` store (2-holder, 3-holder chain,
    multi-row fan-out); plus empty-result holder, single-holder identity, empty
    federation, and the absent-key soundness error. Native-only invariant
    re-verified: `cargo tree -p sparq-wasm` still excludes `sparq-mpc`.
- **DONE in M3 ✅ — the hidden-value path** (`key_disclosed == false`): PRIVATE
  join values via the secret-shared equality test (circuit-PSI core). See M3
  ([`HiddenValueJoin`]). The disclosed-key [`DisclosedKeyJoin`] still routes the
  hidden regime away (it is the crypto-free path), so asking it to handle private
  keys remains an honest `NotYetImplemented` — the private capability lives in
  the dedicated type, not faked into the disclosed-key one.

### M3 — `MpcBackend` (honest-majority Shamir) + hidden-value join  ✅ DONE [OPUS-4.8]
First concrete secret-sharing impl behind the trait + the hidden-value join,
**plus the RS tampered-share-detection hardening (guarantee (D), sq-uu0u)** —
detect-and-abort / robust-correct on every reconstruction path, surfaced via the
`MaliciousSecurity` enum. See the "Security model" bullet below and the
"Deferred malicious-security seams" subsection for what remains beyond v1.
- **Configurable security descriptor (sq-mq8q) ✅ DONE [OPUS-4.8].** The old
  `TrustModel` (binary) + `MaliciousSecurity` (adversary-axis bundled with
  output-guarantee-axis, "HonestMajority" soldered into variant names) are
  replaced — *behind the unchanged `MpcBackend` trait* — by THREE orthogonal axes
  in `backend.rs`: `AdversaryModel` (SemiHonest / Covert{ε} / Malicious),
  `OutputGuarantee` (Abort(AbortKind) / Fairness / GuaranteedOutput), and
  `CorruptionThreshold` (DishonestMajority / HonestMajority / SuperHonestMajority,
  each carrying `t`), plus a `PublicVerifiability` marker, composed into a
  `SecurityDescriptor`. **Cleve's impossibility is a type-level invariant**:
  `Fairness`/`GuaranteedOutput` are constructible ONLY under an honest majority
  (private-witness constructors that reject `DishonestMajority`). `TrustModel` /
  `MaliciousSecurity` are KEPT as a back-compat projection
  (`SecurityDescriptor::{trust_model,malicious_security}`), and guarantees are
  reported PER-OPERATOR (`MpcBackend::operator_security` / `OperatorClass`) — the
  degree-`t` aggregate is robust while the degree-`2t` equality open is
  semi-honest-only at `n = 2t+1`. The fail-closed selection/negotiation registry
  that consumes these axes is the follow-up **bead sq-a6p1** (depends on this).
- **Q2 RESOLVED for v1: honest-majority trust model** (Jesse's decision:
  honest-majority now, configurable long-term). The four flatmates *cooperate*
  among themselves to prove an aggregate to an external landlord — the regime
  Shamir serves. LAN secret-sharing chosen (the aggregate is linear → zero-round
  under Shamir; WAN garbled-circuit unneeded for v1). The *adversary model* is no
  longer purely semi-honest on integrity: tamper-detection / robust-correction
  (guarantee (D)) is now wired through the reconstruction layer (see the "Security
  model" bullet), so the exact integrity guarantee a given `(n, t)` delivers is
  reported by the `MaliciousSecurity` enum rather than asserted as a blanket
  "semi-honest" label here.
- **Scheme: Shamir `t`-of-`n` over `F_p`** (`p = 2^61-1` Mersenne, dependency-
  free `u128` reduction). Chosen over **replicated 3PC** because the use case is
  "any N cooperating flatmates" (replicated is n=3-only) and the secured
  aggregate is linear → Shamir's free local addition is the right cost profile.
  `t = ⌊(n-1)/2⌋` (honest majority). `field.rs` + `shamir.rs`. [OPUS-4.8]
- **Implemented (REAL, in-process multi-party simulation):**
  - `MpcBackend` for `ShamirBackend`: `share_private_input` (Shamir-share a
    holder's private salary), `run_secure` (cumulative sum — zero rounds),
    `reconstruct_disclosed` (checked/robust reconstruction at x=0 when redundancy
    is present — RS / Berlekamp–Welch via `robust.rs`, detect-and-abort or
    correct, see the "Security model" bullet; reduces to Lagrange at 0 only at the
    no-redundancy `t+1` boundary → the disclosed integer; the verifier recomputes
    `> £100k` OUTSIDE the crypto, M5).
  - Secret-shared **equality test** (`secure_equal`): `d=a-b`, mask by fresh
    nonzero `r`, one Shamir multiplication (`mul_shares_raw`, degree 2t), open
    `m=d·r`; `m==0 ⇔ a==b`, leaking ONLY the match bit. Keys never reconstructed.
  - `HiddenValueJoin`: all-pairs oblivious join on a PRIVATE key driven by the
    equality test, disclosing only the matching payload columns.
- **Tested (`cargo test -p sparq-mpc`):** DIFFERENTIAL —
  (a) secure cumulative sum == plaintext sum; (b) hidden-value join == plaintext
  inner join over the union (overlap, no-overlap, multi-match fan-out, empty
  side). Plus: field arithmetic vs reference modulo; share/reconstruct round-
  trip; the threshold actually hides (a <=t-share set is consistent with a
  DIFFERENT secret — information-theoretic hiding witness); reconstruction below
  t+1 errors; the equality primitive in isolation (n=5,t=2). The adversarial
  suite (`adversarial_tests.rs`) pins guarantee (D): tampered-share detection /
  RS correction across the `(n, t, e)` regimes and at the end-to-end
  `reconstruct_disclosed` API boundary, the non-detection boundaries (exact `t+1`
  and the degree-`2t` `n=2t+1` equality open), and the "no fake crypto" stub
  table. Native-only invariant re-verified: `cargo tree -p sparq-wasm` excludes
  `sparq-mpc`.
- **Security model (stated, not papered over):** honest-majority. Privacy holds
  while `< t+1` parties pool shares; each party learns only its shares + the
  disclosed output (confidentiality, guarantee (A)). **Tampered-share detection
  (guarantee (D)) IS now provided at the Shamir reconstruction layer** (sq-uu0u
  WI-1/WI-2/WI-3, merged) — *not* the old semi-honest-only stance:
  - Shamir shares are an `[n, t+1]` Reed–Solomon codeword over the same `F_p`, so
    `robust.rs::reconstruct_robust` (Berlekamp–Welch via Gaussian elimination,
    **zero new deps** — no DLOG group, no SPDZ preprocessing) DETECTS any
    tampering and CORRECTS up to `e = ⌊(n−t−1)/2⌋` cheaters, else ABORTS with the
    typed `MpcError::Tampered`. Every production reconstruction routes through it
    (`ShamirBackend::reconstruct` / `reconstruct_disclosed`, and the degree-`2t`
    equality open via `reconstruct_degree`). The guarantee a given `(n, t)`
    delivers is surfaced through `BackendInfo` as the `MaliciousSecurity` enum
    (`SemiHonestOnly` / `HonestMajorityAbort` / `HonestMajorityRobust{max_cheaters}`).
  - **Honest boundaries where RS cannot help (pinned by tests, not papered over):**
    at exactly `t+1` shares (no redundancy) tampering is information-theoretically
    undetectable; and the degree-`2t` equality/mult open has zero redundancy at
    the honest-majority minimum `n = 2t+1` (odd `n`), so a forged product share is
    undetectable there. A true fix at those no-redundancy points needs an
    information-theoretic MAC (the deferred SPDZ-style seam below, bead sq-6d6g),
    not RS redundancy. **Cheater *attribution* in the abort message is best-effort
    (heuristic) when correction is impossible — detection is sound; sharpening
    blame is bead sq-6u6b.**
  - Robust *with guaranteed output* still assumes an honest majority; a
    dishonest-majority malicious backend (SPDZ/MASCOT IT-MACs, ≈2×) remains future
    hardening behind the SAME trait (deferred seam below).
- **Honest scope / what is scaffolded (not faked):** the join is `O(|L|·|R|)`
  all-pairs (real circuit-PSI uses cuckoo bins — that ~linear optimisation is
  **Q3 / RQ2b, NOT done**); the key→`Fp` encoding is the holder's responsibility
  and needs a collision-resistant hash whose correctness is proven in-circuit in
  production (here controlled so the differential is exact). The dealer's masking
  randomness is a **CSPRNG** (OS-seeded ChaCha20, `rng.rs`, sq-1vt) — a
  deterministic SplitMix64 is reachable only behind a test-only feature gate, so
  the real protocol's masks are unpredictable.
- **Configurability (Jesse's requirement):** the trust model is a property of
  the chosen `MpcBackend` value (`BackendInfo::trust_model`), never hardcoded in
  the join/proof layer. A dishonest-majority (SPDZ/MASCOT) backend slots in
  behind the SAME trait — adds preprocessing (Beaver triples + MACs) and a
  MAC-check before opening, changes only `type Share`, leaves
  `share_private_input`/`run_secure`/`reconstruct_disclosed` SIGNATURES and all
  callers untouched. Documented in `backend.rs`.
- **Feasibility envelope (honest — minutes, not seconds):** one multiplication
  round per candidate pair → `|L|·|R|` secure equality tests for the join; the
  all-pairs structure IS the cost center the literature flags (ORQ SOSP'25: TPC-H
  joins under MPC run minutes-to-tens-of-minutes; "joins are the cost center,
  obliviousness forces worst-case padding"). Viable regime only: honest-majority,
  LAN, ≤10³–10⁴ rows/holder. Do NOT extrapolate to WAN / dishonest-majority
  (zero published data points).

#### Deferred malicious-security seams (sq-uu0u WI-4, bead sq-6d6g) [OPUS-4.8]
The RS detect-and-abort / robust path (WI-1/2/3) closes guarantee (D) **in the
honest-majority, redundancy-present regime** (`reconstruct` at degree `t`; the
degree-`2t` equality open when `n > 2t+1`). Two gaps are explicitly NOT covered
by RS redundancy and are deferred to dedicated future backends behind the SAME
`MpcBackend` trait — recorded here so the boundary is a documented design seam,
not a silent hole (architecture §4.2 guarantee (D), §5.2 Q2; sq-uu0u DESIGN
"REJECTED for now"):

1. **SPDZ-style information-theoretic-MAC backend (dishonest-majority + the
   no-redundancy points).** An additive-share + per-share MAC scheme
   (SPDZ/MASCOT/Overdrive) is the only family that gives malicious security
   *without* reconstruction redundancy, and the only one that extends to a
   **dishonest majority** (up to `n−1` corrupt). It is the principled fix for the
   two RS blind spots: (a) reconstruction at exactly `t+1` shares, and (b) the
   degree-`2t` equality/mult open at the honest-majority minimum `n = 2t+1`
   (`join::secure_equal` / `shamir::reconstruct_degree`) — both have zero RS
   redundancy, so a MAC-check before opening (abort on a failed tag) is required,
   not Berlekamp–Welch. Cost: input-independent preprocessing (Beaver triples +
   MACs) and ≈`2^-61` soundness over `F_p = 2^61−1`. It slots in as a new backend
   reporting `TrustModel::DishonestMajority` + a non-`SemiHonestOnly`
   `MaliciousSecurity`, changing only `type Share` (value + MAC tag) and adding a
   MAC-check in `reconstruct_disclosed` — the `share_private_input` / `run_secure`
   / `reconstruct_disclosed` SIGNATURES and all callers stay put (`backend.rs`).
   REJECTED for *this* increment: it needs preprocessing + a fresh soundness
   argument and is a whole backend, not a Shamir-layer change.
   *Tracked by:* `sq-j5ok` (the dishonest-majority SPDZ/MASCOT/Overdrive backend
   design record — MASCOT-OT vs Overdrive-AHE triples, what it adds vs Shamir,
   the `BackendInfo` preprocessing/PQ/trusted-setup fields), `sq-km34` (the IT-MAC
   on the degree-`2t` equality/mult open at the minimal `n = 2t+1`, promoting
   `secure_equal` `SemiHonestOnly → Abort`), and `sq-4i39` (the
   `BackendInfo.requires_preprocessing` offline-cost field). Design record:
   `research/mpc-malicious-security-design.md`.

2. **Poseidon2 / Schnorr share-commitments for the M4 attestation layer.**
   Binding a *reconstructed* (or shared) value to an issuer-signed commitment
   inside the collaborative proof (guarantees (B) correctness end-to-end and (C)
   attested-source) needs a commitment in a circuit-friendly group — Poseidon2
   over a SNARK field (BN254 `Fr`), with Schnorr/EdDSA opening — NOT the lean
   `F_p = 2^61−1` arithmetic this crate carries. That deliberately keeps arkworks
   /DLOG out of `sparq-mpc`; it lives in the M4 proof/attestation seam (`proof.rs`,
   gated on Q1 + the ZK-foundation remediation), which is a separate milestone.
   REJECTED for this increment: wrong field, would drag a heavy dependency into a
   crate that is intentionally dependency-light and wasm-excluded.
   *Tracked by:* `sq-bjl` (the M4 collaborative-proof + distributed-attestation
   SPIKE — the Q1 research risk), `sq-f7bu` (the buildable M4-v1: verifier-side
   authenticated-input attestation gate, Dutta/Artemis commit-and-prove anchor),
   and `sq-34ml` (the M4-v1 freshness/replay binding + federated
   `reconstruct_public_inputs` layout prerequisites) — **`sq-34ml` LANDED** as
   `src/federated_binding.rs`: it is the *scoping + out-of-circuit binding* half
   only (a deterministic multi-source public-input byte layout and a fail-closed
   freshness/replay transcript), it closes **no** ZK-audit item, and `proof.rs`
   remains an honest `NotYetImplemented`. Feasibility record (the
   EdDSA-Poseidon / Schnorr-Baby-JubJub constraint sizing):
   `research/mpc-m4-distributed-sig-feasibility.md`.

Neither seam is on the v1 critical path: v1 is honest-majority, and the RS path
already delivers (D) there. This subsection (bead `sq-6d6g`, the WI-4 doc-only
deliverable) is the durable record of the boundary; the *implementation* of each
seam is tracked by its successor beads named above (SPDZ → `sq-j5ok` / `sq-km34`
/ `sq-4i39`; M4 attestation → `sq-bjl` / `sq-f7bu` / `sq-34ml`) and named at the
call sites, so the deferral is auditable.

### M4 — Collaborative proof + distributed attestation  ⚠️ THE HARD PROBLEM (SPIKE)
The contribution AND the principal research risk (architecture §5.2 Q1, §3.4).
**This is what remains after M3** — M3 gives confidentiality (A) over real
secret-shared computation, but NOT correctness-proof (B) or attestation (C) to a
relying party. Those are M4, and M4 is the Q1 dependency:
- **SPIKE, not routine engineering:** distributed signature/commitment-opening
  over secret-shared witnesses inside a collaborative proof — **Q1, unsolved in
  the literature** ("the join nobody has built"). M3's secret-sharing layer is
  the substrate the collaborative witness would be shared over, but verifying a
  BBS+/EdDSA signature over that shared witness inside one emitted proof is the
  open research problem; M3 does NOT touch it.
- Use PATCHED collaborative-SNARK constructions (eprint 2025/1026 soundness
  pitfalls) + a FRESH soundness audit (architecture §5.2 Q4).
- Bind: correctness (`Disclosed(π) ⊆ Eval_PAG(Q,D)`), attested source
  (issuer-key set-membership over K), query digest, FILTER, per-row attribution,
  fresh challenge.
- **Hard-blocked on M1** (the whole ZK foundation #3/#4/#5/#6/#8/#9/#12 — until
  these land, a collaborative proof composes onto a verifier that returns `Ok`
  for forged results: theatre) **AND on M3** (the chosen backend — now DONE).
  M3 → M4 is gated on Q1 + the ZK estate, NOT on more MPC-primitive work.

### M5 — Disclosed-aggregate recompute OUTSIDE the crypto
Implement convention #4 end-to-end: DISTINCT / ORDER BY / LIMIT / OFFSET /
COUNT / SUM / AVG / MIN / MAX over the DISCLOSED multiset recomputed by the
verifier; joins/UNIONs/OPTIONALs over disclosed values as out-of-circuit checks.
HAVING / GROUP BY over hidden multisets forbidden. Keeps cross-source
join/aggregation reasoning out of the cryptographic core wherever values are
disclosed (architecture §4.3 step 3).

### M6 — Performance + benchmarks
**Tier-1 (in-process counting) DELIVERED (sq-sxm):** the
(security model × N × query class) benchmark matrix is built — `src/metrics.rs`
(the deterministic communication / round / multiplication counter `CommCounter`)
+ `src/bench.rs` (the matrix harness: query classes × N ∈ {2,3,5,7} × the ACTUAL
per-operator security read off `ShamirBackend::operator_descriptor`) + the
runnable `examples/mpc_bench_matrix.rs`. Because the crate is an in-process
simulation, the load-bearing metric is the **deterministic modelled
communication** (bytes/party, rounds, multiplications) — NOT a single-process
wall-clock, which is not an MPC latency. Each cost cell co-runs the real
primitive so correctness gates cost. The harness emits the numbers (a structured
JSON schema + a table); per the no-hard-coded-perf rule they are not baked into
any doc.

**Network tiers (still open, beaded):** the real multi-process transport +
`tc`/`netem` LAN/WAN emulation (Tier 2/3) is **bead sq-tg6b**; the EC2 scale-out
(heavy data scales, real inter-AZ/region) is **bead sq-hoaj**. Those tiers obey
the disk-space + empirical-honesty discipline: check `df` during runs, cap
dataset size, clean `/tmp` scratch, bench data git-ignored. Report real envelopes
— minutes, not "seconds" — and never extrapolate to the WAN/dishonest-majority
regime that has zero published data points.

The sq-hoaj **ceiling-run HARNESS is built** — `scripts/mpc-ec2-ceiling.sh`,
the orphan-proof EC2 orchestrator that sweeps the hidden-value join across
N ∈ {7,9,11} × rows ∈ {100,1000,10000} (10⁴ rows = 10⁸ `secure_equal` opens, the
deliberate ceiling probe) under the netem LAN profile on a box that HAS the
`CAP_NET_ADMIN` the dev box lacks. It carries the full safety recipe (tag
`purpose=sparq-bench`, `--instance-initiated-shutdown-behavior terminate`, two
independent watchdogs + a `df` floor watchdog, `/tmp` cleanup, the 10⁴ row cap,
and an `orphan-check-bench.sh` sweep on exit) and a hermetic `--self-test` that
pins those rails without touching AWS. It records the dishonest-majority and
WAN-at-scale regimes as explicit `no-data-research-risk` cells (never a fabricated
number); the row cap is HARD (a larger scale is refused, not extrapolated).
*Executing* it — producing the git-ignored minutes-envelope JSON — still needs a
credentialed host, so the measured envelope is not baked into any doc.

---

## Decision-point / dependency summary
- **Q1** decided/spiked at **M4** (gates the collaborative proof + attestation) —
  **the principal thing that remains.** Unsolved in the literature; SPIKE.
- **Q2** ✅ DECIDED at **M3**: honest-majority Shamir for v1 (configurable behind
  the trait long-term). Done.
- **Guarantee (D), malicious security** ✅ DELIVERED at **M3** for the honest-
  majority, redundancy-present regime (sq-uu0u WI-1/2/3): RS / Berlekamp–Welch
  detect-and-abort / robust-correct on every reconstruction path, surfaced via
  the `MaliciousSecurity` enum. Remaining seams (no-redundancy points + dishonest
  majority via an IT-MAC backend; M4 attestation commitments) are deferred and
  documented under M3 "Deferred malicious-security seams" (bead sq-6d6g);
  attribution sharpening is bead sq-6u6b.
- **Q3** (BGP-join obliviousness cost / RQ2b) — surfaced concretely at **M3**:
  the hidden-value join is `O(|L|·|R|)` all-pairs; the ~linear cuckoo-bin /
  oblivious-hashing optimisation and the fragment it applies to is **still open**
  (analysis + impl remain).
- **Q4** (coZK soundness post-2025/1026) re-audited at **M4**.
- **Hard dependency:** M1 (ZK foundation #3/#4/#5/#6/#8/#9/#12) gates M4 entirely
  and is a prerequisite for any relying-party meaning.
