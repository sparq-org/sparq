<!-- [OPUS-4.8] Distributed / dealer-less correlated-randomness design for sparq-mpc:
PRSS vs honest-majority coin-toss + dealer-less VSS, replacing the single-dealer simulation.
Design-for-review (design + code seam, no full impl), Opus 4.8 (Fable unavailable) —
re-review when Fable returns. Date: 2026-07-19. Bead sq-yyro (parent MPC epic sq-pwr). -->

# Distributed randomness (PRSS / honest-majority coin-toss) + dealer-less VSS

**Status:** Deep-research design record + code seam. Author: Opus 4.8 (Fable unavailable — flag
for re-review). Date: 2026-07-19. Bead **sq-yyro** (parent MPC epic **sq-pwr**; issue #2989,
parent #2629).

**Implementation status (updated 2026-08-01, issue #3531).** The record was authored *before* any
dealer-less implementation existed and the tier statements below are written in that voice. Since
then the **PRSS online generator has landed** (`crate::prss`, §5 item 1) on a **simulated** seed
setup. §4a is the authoritative accounting of what is and is not implemented; where the
"as authored" text below conflicts with it, §4a wins.

**Scope.** How sparq-mpc replaces its **single-trusted-dealer randomness simulation** with a
**dealer-less** correlated-randomness layer suitable for a real federation: (1) the choice
between **PRSS** (replicated-PRF pseudo-random secret sharing) and **distributed coin-toss** for
jointly generating masks / correlated randomness non-interactively; (2) **dealer-less VSS** so
each holder shares its OWN input verifiably rather than a trusted dealer sharing on everyone's
behalf; and (3) the **`r = 0` mask-forcing threat** that a semi-honest-only mask generator does
not defend against. This is the "design + seam" half of sq-yyro; the concrete PRSS/coin-toss/VSS
implementations are follow-on beads behind the seam this record specifies.

**This record EXTENDS, does not duplicate:**
- [`mpc-security-models-and-benchmarks.md`](./mpc-security-models-and-benchmarks.md) §7
  ("Randomness / CSPRNG + trusted setup") — which *names* the two gaps (no trusted dealer; the
  `r = 0` verdict-flip) and points at PRSS (eprint 2021/1223, IETF draft-thomson-ppm-prss). That
  doc names the gap; this doc is the construction + seam.
- [`mpc-sparql-capability-matrix.md`](./mpc-sparql-capability-matrix.md) §1.2 ("Single-dealer
  randomness simulation … `dealer()` is a stand-in") and §"highest-leverage next steps" item 6
  ("Distributed randomness (PRSS / dealer-less VSS)").
- [`mpc-malicious-security-design.md`](./mpc-malicious-security-design.md) — the IT-MAC
  authenticated-sharing layer. The `r = 0` defense composes with (and in the malicious tier
  *depends on*) that layer; this record states the interface, not the MAC construction.

**The achieved tier, stated up front (not over-claimed) — as authored, 2026-07-19; see §4a for
what has since landed.** This record is a **design record + a compiling code seam**
([`crate::randomness`]). *At the time of writing* it shipped **NO** dealer-less crypto: PRSS, the
coin-toss, and dealer-less VSS were all honest `NotYetImplemented`-gated behind the seam. The one
thing that changed in code was that the correlated-randomness contract became an explicit trait
with an honest `RandomnessModel` self-description, so the single-dealer path is *labelled* as the
simulation it is (`RandomnessModel::TrustedDealerSim`, `deployable() == false`) rather than
silently masquerading as a federation-ready source. No security claim was made or strengthened
here; `sq-qhy4` external sign-off remains pending for the whole crate — and, per §4a, it remains
pending *after* the PRSS generator landed, which is why no variant is `deployable()` yet.

---

## 0. Ground truth: where randomness comes from TODAY (verified against `origin/main`)

- **The masking CSPRNG is honest at the single-dealer level (sq-1vt, CLOSED).**
  `crate::rng::SecureRng` is an owned ChaCha20 CSPRNG seeded from OS entropy (`getrandom::fill`),
  drawn as **uniform** `F_p` elements by rejection sampling (no modulo bias), with the key
  schedule `ZeroizeOnDrop`-scrubbed (sq-it50) and the deterministic `InsecureTestRng` gated out
  of default builds. **This is not the gap** — the *quality* of the randomness is fine.
- **The gap is WHO draws it.** A single [`crate::shamir::ShamirDealer`] owns the live RNG and
  performs, centrally, every randomness-consuming step of the in-process simulation:
  - `ShamirDealer::share(secret)` — draws the `t` free masking coefficients and hands out all `n`
    shares. In the simulation the *dealer* knows every holder's secret and shares it on their
    behalf. A real federation has no such party.
  - `ShamirDealer::draw_nonzero_fp()` → the equality-test mask `r`. `secure_equal` hides the key
    difference `d = a − b` behind `m = d · r` and opens `m`; `m = 0 ⇔ d = 0` **only if `r ≠ 0`**.
    Today the *dealer* draws `r` and knows it (a simulation artefact — "a real deployment cannot
    let any party know `r`", capability-matrix §"sq-mnv5").
  - `ShamirDealer::degree_reduce(...)` — each simulated party re-shares under fresh masking
    coefficients drawn from the *same* dealer RNG (sq-dvuc).
- **Prior art already in-crate: the square-protocol random bit (sq-mnv5 / sq-bgsn).**
  `disclose_threshold_verdict`'s Rabbit decomposition no longer lets the dealer deal the mask in
  cleartext: a random `[a]` is jointly held, only `c = a²` is opened, and `[b] = (a·d⁻¹+1)·2⁻¹`
  for the public root `d = c^{(p+1)/4}` (valid because `p = 2^61−1 ≡ 3 mod 4`). **No party knows
  the bit.** This is the *only* dealer-less randomness in the crate today, it is honest-majority
  **semi-honest**, and it is the natural template PRSS/coin-toss generalise from a single bit to
  arbitrary correlated randomness.
- **The crate is an in-process simulation.** Every "party" is a function call; there is no real
  network for a coin-toss round. So "dealer-less" here is first a *correctness/honesty* property
  (no party may know a mask) and only later a *networked* protocol (the `transport` tier).

---

## 1. The threat: a maliciously-forced `r = 0` flips equality verdicts

The load-bearing correctness invariant of the hidden-value join is the **nonzero-mask**
guarantee of `secure_equal` / `secure_equal_to_bit`:

```text
d = a − b            (secret-shared key difference)
m = d · r            (r a secret-shared random mask)
open(m):  m == 0  ⇔  d == 0  ⇔  a == b
```

This equivalence holds **iff `r ≠ 0`**. If an adversary can force the mask to `r = 0`, then
`m = 0` **regardless of `d`**, so *every* pair reports "equal" — a false match on every row.
The join's entire soundness rests on `r` being (a) uniform and (b) nonzero, jointly generated so
that **no single party controls it**.

- **Under the current single-dealer simulation** this is not exploitable *because there is one
  honest dealer* who draws `r` from the CSPRNG via `draw_nonzero_fp()` (which rejection-loops away
  the `1/p` zero). But that is precisely the property a real federation cannot assume: there is no
  trusted dealer to guarantee `r ≠ 0`.
- **Semi-honest dealer-less generation (PRSS / coin-toss) fixes the DISTRIBUTION** — `r` is
  uniform and unknown to any `≤ t` parties — **but does NOT by itself enforce `r ≠ 0` against an
  ACTIVE adversary.** A malicious party that contributes to the mask can bias its contribution so
  the combined `r` lands on `0` (or on any chosen value), unless the generation is *verified*.
- **Therefore the `r = 0` defense is an axis-1 (adversary) property, not an axis-3 (threshold)
  one.** It belongs with the malicious tier, and it must actually **establish that the combined
  value is nonzero**: the mask must be produced so that (a) no single party can steer the combined
  `r` and (b) `r ≠ 0` is *verified* by a distributed zero-test, with the opens of that test
  authenticated so a tampered share aborts. MAC-checking the final `open(m)` **alone is not
  sufficient** — an IT-MAC authenticates only that the opened `m` equals the shared `[d·r]`, not
  that `r ≠ 0`, so a *correctly-authenticated* `r = 0` still opens `m = 0` and flips the verdict.
  The construction therefore has two composed parts:
  1. **Nonzero establishment: biasing-resistant generation + a distributed zero-test with abort
     (this is what defends `r = 0`).** Generate `[r]` with a *biasing-resistant* joint protocol
     (commit-before-open coin-toss, or PRSS) so no party can force the combined value — then
     `r = 0` occurs only with the uniform `1/p ≈ 2^−61` probability — and run a distributed
     zero-test on `[r]` (the existing square-protocol / a nonzero check), *aborting-and-redrawing*
     if `r = 0`. The negligible zero probability makes at most one redraw overwhelmingly likely.
     For the zero-test to be sound against an active adversary its opens must themselves be
     authenticated (part 2); in the pure semi-honest tier its soundness reduces to "one honest
     contributor".
  2. **IT-MAC authentication (necessary to make part 1 sound — NOT a nonzero proof by itself).**
     Carry an IT-MAC on `[r]`, on the inputs, and on `[m] = [d·r]` (the
     `mpc-malicious-security-design.md` layer) and MAC-check every open before acting on it. This
     is what makes part 1's zero-test — and the final `open(m)` — sound: a party that *tampers*
     with a share/opening (a value not matching its authenticated `α·x` MAC) is caught and the
     protocol aborts fail-closed at the minimal `n = 2t+1`. What it does **not** do on its own is
     prove `r ≠ 0`: if a malicious contributor biases *generation* so `[r]` is a
     correctly-authenticated sharing of `0`, then `[m] = [d·r] = [0]` opens to `0` with a perfectly
     valid MAC and the equality verdict still flips. Nonzeroness is established by part 1; the
     IT-MAC layer authenticates that machinery, it does not replace it. This is why the seam's
     `shared_nonzero_mask` stays semi-honest-only until BOTH the biasing-resistant generation and
     the authenticated zero-test (sq-km34.*) land.

**Honesty consequence for the seam.** The single-dealer `shared_nonzero_mask` is honest *only*
because the one dealer is honest; a PRSS/coin-toss `shared_nonzero_mask` is honest-majority
**semi-honest** and does **not** defend `r = 0` against a malicious contributor. The seam must
therefore report its adversary tier, and the malicious defense — biasing-resistant generation plus
an authenticated distributed nonzero zero-test, *built on* the IT-MAC layer (not the IT-MAC'd open
alone) — is a follow-on bead, not this record.

---

## 2. PRSS vs distributed coin-toss — the decision

Both produce randomness that no `≤ t` parties know. They differ on interaction and on what
"correlated" randomness they yield.

### 2.1 PRSS — replicated-PRF pseudo-random secret sharing (eprint 2021/1223; draft-thomson-ppm-prss)

**Construction.** In a one-time setup, replicated PRF seeds are distributed so that each of the
`C(n, t)` maximal unqualified sets `A` (`|A| = t`) has a shared seed `k_A` held by *exactly* the
parties **not** in `A`. To generate the next shared random field element, every party locally
evaluates `PRF(k_A, ctr)` for each seed it holds and sums `Σ_A PRF(k_A, ctr) · f_A(x_i)`, where
`f_A` is the fixed public Lagrange-style polynomial that is `1` at `0` and `0` at every point in
`A`. The result is a fresh degree-`t` sharing of `Σ_A PRF(k_A, ctr)` — **a value no `≤ t` parties
know, generated with ZERO online communication.**

- **Pros.** *Non-interactive* after setup — 0 rounds, 0 bytes per random element (only local PRF
  calls). This is the dominant honest-majority win: masks, the equality mask `r`, degree-reduction
  re-sharing randomness, and the oblivious-shuffle control bits all become *free* per element.
  Directly matches the crate's "linear ops are zero-round" cost profile. Post-quantum-fine if the
  PRF is (ChaCha/AES-based is not PQ-broken for this use).
- **Cons.** (1) *Setup* — the replicated seeds must be distributed once (a small dealer-less
  key-agreement or a one-time trusted setup), and the seed count `C(n, t)` **blows up
  combinatorially** in `n` (e.g. `C(9,4) = 126` seeds/party). PRSS is the right tool for **small
  `n`** — exactly the four-flatmates deployment target, borderline by `n ≈ 10`. (2) It is
  *pseudo*-random (computational), a step down from the information-theoretic Shamir core — an
  honest caveat, though ChaCha20 masking already sits at this tier (sq-1vt). (3) Correlated
  randomness *beyond* a plain random sharing (Beaver triples, random bits) needs the PRSS output
  fed into a further protocol (e.g. the square-protocol bit, or a triple-generation step).

### 2.2 Distributed coin-toss — commit-open joint randomness

**Construction.** Each party `i` samples `r_i`, **commits** (`Com(r_i)`), broadcasts the
commitment, then opens; the shared randomness is `r = Σ r_i` (or each `r_i` is VSS-shared and the
sharing is summed). Malicious hardening uses VSS so a party cannot abort selectively after seeing
others' openings.

- **Pros.** *No combinatorial setup* — scales to any `n` with `O(n)` per party. Naturally yields
  a value **provably unknown** to anyone (additive over all parties, so one honest party
  randomises it). With VSS-based sharing it upgrades cleanly to malicious-with-abort.
- **Cons.** *Interactive* — at least one commit+open round **per batch** of randomness (amortised
  by generating many elements per round). On a WAN (the federation setting, open question §RQ2)
  rounds dominate. This is the classic PRSS-vs-coin-toss trade: coin-toss pays rounds, PRSS pays
  setup+seed-count.

### 2.3 Decision

**Primary: PRSS for the small-`n` honest-majority deployment (the flatmates use case), with a
distributed coin-toss as the general-`n` / setup-free fallback.** Rationale, matching the crate's
existing decisions:

- The driving use case is **small `n`** (four flatmates; capability-matrix). PRSS's seed blow-up
  is a non-issue at `n ≤ ~8`, and its **zero-round** generation is exactly the cost profile the
  honest-majority Shamir backend was chosen for ("linear ops are free"). Masks become free.
- The crate is **LAN-first** (open question §RQ2 item 2). PRSS's non-interactivity is worth most
  on the WAN, but even on LAN removing a commit-open round per mask batch is a clean win, and it
  avoids threading a new interactive round through the in-process simulation.
- **Coin-toss stays specified** as the fallback for larger `n` (where `C(n,t)` seeds are
  impractical) and for the setup-averse deployment, and as the VSS-based malicious upgrade path.
- Both sit behind the SAME seam ([`crate::randomness::DistributedRandomness`]); the choice is a
  `RandomnessModel` value a federation inspects, never a hard-coded type — mirroring how
  `MpcBackend` defers the primitive choice.

---

## 3. Dealer-less VSS — each holder shares its OWN input

The `ShamirDealer::share(secret)` simulation has the dealer share every holder's secret. The real
protocol requires each holder to be the dealer of **its own** input, with the *other* parties able
to verify the sharing is a consistent degree-`t` polynomial — otherwise a malicious holder can
distribute inconsistent shares that reconstruct to different values for different quorums (the
classic VSS attack), silently corrupting the aggregate.

- **Semi-honest tier — no VSS needed.** A semi-honest holder shares its own input correctly by
  assumption; `share_private_input` already models "the holder shares its own value". The only
  change is *labelling*: the holder, not a global dealer, owns that sharing.
- **Malicious tier — honest-majority VSS.** The standard honest-majority VSS (Feldman/Pedersen
  for the committed variant, or the BGW/CDN information-theoretic VSS using pairwise consistency
  checks) lets each holder share its input with a distributed check that the shares lie on ONE
  degree-`t` polynomial, aborting on a cheater. This is the *input* analogue of the robust
  reconstruction the crate already has on the *output* side (`crate::robust`, Reed–Solomon /
  Berlekamp–Welch, sq-m34i): robust reconstruction catches a tampered *open*; VSS catches a
  tampered *share-out*. Together they close the malicious loop at both ends.
- **Composition.** Dealer-less VSS produces the `[input]` sharings; PRSS/coin-toss produces the
  `[r]` masks and degree-reduction randomness; the existing linear ops + `degree_reduce` +
  `secure_equal` + robust reconstruct run unchanged on top. No join/proof-layer signature changes
  — the seam absorbs the difference exactly as `MpcBackend::Share` absorbs the primitive change.

---

## 4. The code seam ([`crate::randomness`])

The seam is a single new module carrying:

- **`trait DistributedRandomness`** — the correlated-randomness contract the protocol consumes,
  independent of *how* the randomness is produced. The per-method contracts are **structural
  only** (a well-formed degree-`t` sharing of the stated value); the security guarantees a
  federation cares about (mask secrecy, VSS verification) are capabilities of a validated
  dealer-less *implementation*, never of a method or a self-reported label:
  - `randomness_model() -> RandomnessModel` — honest self-description of the regime
    (descriptive only — a label is never deployment evidence).
  - `shared_mask() -> Result<Vec<Share>, MpcError>` — a fresh degree-`t` sharing of a
    **uniform** mask. Structural contract only: secrecy from up to `t` parties is a capability
    of a validated dealer-less implementation, not a guarantee of the method itself — the
    single-dealer implementor draws, and therefore knows, every mask.
  - `shared_nonzero_mask() -> Result<Vec<Share>, MpcError>` — the equality-test mask; **`r ≠ 0`**
    (the §1 threat). Documented semi-honest-only until the nonzero-establishment path
    (biasing-resistant generation + an authenticated distributed zero-test, §1) backs it — an
    IT-MAC on the open alone does not prove `r ≠ 0`.
  - `vss_own_input(secret) -> Result<Vec<Share>, MpcError>` — a holder shares its OWN input
    (dealer-less VSS in a real deployment; a plain sharing in the semi-honest simulation).
- **`enum RandomnessModel { TrustedDealerSim, Prss, HonestMajorityCoinToss }`** — with
  `is_dealer_less()` (regime description) and `deployable()` (**`false` for every variant
  today**: the dealer-less variants stay descriptive until a validated implementation lands,
  because the model is self-reported and any type can claim `Prss`), so a caller /
  `BackendInfo`-style report can state honestly which regime is in force. The
  `require_deployable()` refusal gate is accordingly **necessary, not sufficient** — it
  currently refuses every source fail-closed, including a self-labelled dealer-less stub.
- **`impl DistributedRandomness for ShamirDealer`** — the CURRENT single-dealer **simulation**.
  It reports `RandomnessModel::TrustedDealerSim` (`deployable() == false`), and its `shared_mask`
  / `shared_nonzero_mask` / `vss_own_input` route to the existing `draw_fp` / `draw_nonzero_fp` /
  `share`. This is the honest label: the dealer *knows* the mask, so it is a simulation, not a
  federation-ready source. Making the contract explicit is the whole point of the seam — the
  dealer-less impls slot in behind it without touching the join/compare/oblivious callers.
- **No PRSS/coin-toss/VSS implementor ships *with the seam itself*.** Those are follow-on beads
  (below); the seam adds NO fake dealer-less crypto, it only names the contract and labels the
  simulation. The PRSS implementor has since landed behind this trait — §4a.

**Why a trait `ShamirDealer` implements (not free functions / an unused stub).** Implementing the
trait for the existing dealer makes the seam *used* (no dead code), pins the exact call-shape the
future PRSS/coin-toss source must satisfy, and lets a caller migrate to `&mut dyn
DistributedRandomness` incrementally. A dealer-less source is then a new type implementing the
same trait — swapped in by `RandomnessModel`, never by concrete type.

---

## 4a. Implementation status (authoritative; updated 2026-08-01, issue #3531)

What has actually landed behind the seam, separated from what the sections above *specify*.

**Implemented — the PRSS ONLINE GENERATOR (`crate::prss`, §5 item 1, partial).**
`PrssRandomness: DistributedRandomness` reports `RandomnessModel::Prss` and generates degree-`t`
sharings by the real `Σ_A PRF(k_A, ctr)·f_A` rule with **zero online rounds**. Each party's share
is summed over that party's own seed view; the generation path is *instrumented* so a test records
which seeds each party's share touched and requires that set to be exactly the complement of `A`
(a leak that is arithmetically invisible — coefficient `f_A(x_i) = 0` — still goes red). The
`C(n, t)` seed count is capped by `prss::MAX_PRSS_SEEDS`, refusing `n ≥ 10` fail-closed and naming
the §2.2 coin-toss fallback. This answers §6 Q1 for this crate: the ceiling is set at `n ≤ 9`.

**PRF instantiation (the construction's PRF assumption, made concrete).** `PRF(k_A, ctr)` is a
domain-separated SHA-512 key derivation — `SHA-512("sparq-mpc/prss/v1/prf" ‖ k_A ‖ ctr)`, all
inputs fixed-length so distinct `(k_A, ctr)` pairs cannot collide by concatenation ambiguity —
keying a fresh ChaCha20 stream from which one field element is drawn by the crate's uniform
rejection sampler (no modulo bias). The construction needs a PRF keyed by `k_A` and indexed by
`ctr` whose outputs are computationally indistinguishable from uniform `F_p` to a party lacking
`k_A`; a keyed-hash-to-stream instantiation is the standard way to meet it, and it reuses the same
ChaCha20 generator the masking CSPRNG already relies on (sq-1vt), so it introduces no primitive
the crate was not already trusting at the computational tier. This is an *engineering* argument
from standard primitives, **not** a proof and **not** an audited reduction — the pseudorandomness
caveat of §2.1 cons 2 applies unchanged, and `sq-qhy4` external sign-off is pending.

**NOT implemented — the gaps that keep this non-deployable.**
- **Setup is a SIMULATED one-time trusted setup, not dealer-less key agreement.** The `C(n, t)`
  replicated seeds are drawn locally from OS entropy. §6 Q3 is still open, and the real
  replicated-seed distribution is not built.
- **The `r ≠ 0` check is CENTRAL, a simulation artefact.** The rejection evaluates
  `r = Σ_A PRF(k_A, ctr)` directly, which is possible only because this in-process simulation
  holds every seed; no party can do it. The distributed zero-test of §1 part 1 / §5 item 4 is the
  replacement and has not landed, so `shared_nonzero_mask` stays **semi-honest-only**.
- **Dealer-less VSS is absent.** `PrssRandomness::vss_own_input` returns `NotYetImplemented`:
  PRSS generates correlated randomness, it does not verifiably share a holder's own input (§3,
  §5 item 3).
- **No malicious-tier property.** Nothing detects a party emitting a wrong share; §5 item 4 is
  untouched.
- **In-process simulation.** All `n` shares are computed in one process, which holds every seed.
  What is demonstrated is protocol *structure* — which seed may enter which share, and that the
  output is a correct degree-`t` sharing — never process or network isolation.
- **Coin-toss (§5 item 2) and caller wiring (§5 item 5) are untouched.**

**Deployment status: unchanged.** `RandomnessModel::Prss.deployable()` is still `false`, so
`require_deployable()` refuses this source exactly as it refuses the single-dealer simulation.
Acceptance stays tied to a *validated* construction (§5 item 5), never to the self-reported label,
and the crate remains research-grade and externally unaudited (`sq-qhy4` pending).

---

## 5. Follow-on beads (implementation, behind the seam)

1. **PRSS setup + generator** (small-`n` honest-majority): replicated-seed distribution + the
   `Σ_A PRF(k_A, ctr)·f_A` degree-`t` generator; a `PrssRandomness: DistributedRandomness`
   reporting `RandomnessModel::Prss`. Seed-count guard fails closed for large `n`.
   **Status: PARTIALLY LANDED** (#3531) — the generator and the seed-count guard ship; the
   replicated-seed *distribution* is still a simulated trusted setup. See §4a.
2. **Distributed coin-toss** (general-`n` / setup-free fallback): commit-open (or VSS-summed)
   joint randomness; `CoinTossRandomness: DistributedRandomness`.
3. **Dealer-less honest-majority VSS** for `vss_own_input` in the malicious tier (pairwise
   consistency / Feldman), composing with `crate::robust` on the output side.
4. **`r = 0` active defense**: biasing-resistant joint generation **plus** a distributed nonzero
   zero-test-and-redraw whose opens are IT-MAC-authenticated (`mpc-malicious-security-design.md`
   sq-km34.*) — the two composed (§1), NOT a MAC-checked open alone, which authenticates that `m`
   equals `[d·r]` but not that `r ≠ 0`. Promotes `shared_nonzero_mask` from semi-honest-only to
   malicious-with-abort.
5. **Wire the callers** (`secure_equal`, `degree_reduce`, oblivious-shuffle control bits) to draw
   through `&mut dyn DistributedRandomness` instead of the inherent dealer methods — with the
   production entry points gated through `require_deployable()` — so the dealer-less source is
   selectable end-to-end. Deployment acceptance must then be tied to the validated PRSS /
   coin-toss constructions themselves (an unforgeable construction boundary), not to the
   descriptive `RandomnessModel` label a source self-reports.

---

## 6. Open questions carried forward

1. **PRSS seed-count ceiling.** At what `n` does `C(n, t)` force the coin-toss fallback for THIS
   deployment? (Flatmates `n = 4` → `C(4,1) = 4` seeds; fine. `n = 9` → 126; borderline.)
   **Answered in code** (#3531): `prss::MAX_PRSS_SEEDS` puts the ceiling at the borderline case —
   `n ≤ 9` builds, `n ≥ 10` (`C(10,4) = 210`) is refused fail-closed pointing at the coin-toss.
   Whether the *federation* wants the ceiling that high is still a deployment question.
2. **LAN vs WAN (shared with the security-models doc §9.2).** If v1 is WAN, PRSS's
   non-interactivity is decisive and the coin-toss round-cost is a real tax; if LAN, either works.
3. **Setup trust for PRSS.** The replicated-seed setup is itself a small dealer-less protocol (or
   a one-time trusted setup) — does the federation accept a one-time setup, keeping the *online*
   phase trust-minimal, or must setup also be dealer-less (key-agreement)?
4. **Does the semi-honest tier even need dealer-less randomness for the FLATMATES case?** If the
   holders are cooperating-but-curious (the crate's stated first target), the single honest
   contributor suffices and the `r = 0` active defense is only needed once a *malicious* holder is
   in scope — tying this bead's priority to the sq-km34 malicious epic.

---

## Sources

PRSS (Cramer–Damgård–Ishai TCC'05; eprint 2021/1223; IETF draft-thomson-ppm-prss); BGW VSS
(STOC'88); Feldman VSS (FOCS'87); Pedersen VSS (CRYPTO'91); CDN (EUROCRYPT'01); Damgård–Nielsen
DN07 (CRYPTO'07); Rabbit (eprint 2021/119); Canetti UC-without-setup for honest majority
(FOCS'01). In-crate: `crate::rng` (sq-1vt/sq-it50), `crate::shamir` (sq-dvuc degree reduction),
`crate::robust` (sq-m34i), the square-protocol random bit (sq-mnv5/sq-bgsn), and
`mpc-malicious-security-design.md` (sq-km34.*).
