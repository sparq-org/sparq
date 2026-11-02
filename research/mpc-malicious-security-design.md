<!-- [OPUS-4.8] IT-MAC authenticated-sharing design for honest-majority malicious-with-abort
security in sparq-mpc. Design-for-review (no code), Opus 4.8 (Fable unavailable) — re-review
when Fable returns. Date: 2026-06-15. Bead sq-km34 (parent epic sq-0jsc → sq-pwr). -->

# IT-MAC Authenticated Secret Sharing: closing the semi-honest holes in sparq-mpc

**Status:** Deep-research design record (no implementation; doc-only). Author: Opus 4.8
(Fable unavailable — flag for re-review). Date: 2026-06-15. Bead **sq-km34**
(parent research epic **sq-0jsc**, MPC epic **sq-pwr**).

**Scope.** The concrete design for upgrading sparq-mpc's *semi-honest* Shamir primitives to
**honest-majority malicious-with-abort** security via information-theoretic MACs (IT-MACs,
the SPDZ / authenticated-Shamir family), selectable and reported per-operator through the
existing three-axis backend registry. This is the directive's "configurable SECURITY MODELS"
half on the *adversary* axis (AXIS-1: semi-honest → malicious).

**This record EXTENDS, does not duplicate:**
- [`mpc-security-models-and-benchmarks.md`](./mpc-security-models-and-benchmarks.md) — the
  3-axis `AdversaryModel × OutputGuarantee × CorruptionThreshold` framing (§1.2), the
  `MaliciousSecurity` projection, the fail-closed registry (§1.3), and §8 step 5 ("IT-MAC for
  the degree-`2t` equality open at `n=2t+1`"). That doc *names* the hole; this doc is the
  *construction*.
- [`mpc-sparql-capability-matrix.md`](./mpc-sparql-capability-matrix.md) — which names
  dishonest-majority-malicious correctness as out-of-reach (§"OUT OF REACH") and identifies the
  degree-`2t` open at minimal `n=2t+1` as the one real semi-honest hole (§"Mal, HM, minimal
  N=2t+1 — OPEN (the one real hole)"), plus §4.4 ("the cost of going semi-honest → malicious").
- The honest-SOTA verdict in
  [`mpc-zkp-research-and-architecture.md`](./mpc-zkp-research-and-architecture.md) and the
  `mpc-protocols` skill (authenticated-Shamir / SPDZ-family / "malicious comes free in honest
  majority").

**The achieved tier, stated up front (and not over-claimed):** this design delivers
**honest-majority, malicious-with-abort (unanimous abort)** security for the operators it
covers. It does **NOT** deliver dishonest-majority (that is a different construction — full
SPDZ with offline OT/SHE triples, the truthfully-refused `sq-j5ok` slot), and it does **NOT**
deliver fairness or guaranteed output (GOD) against a *malicious* adversary in general — Cleve
(STOC'86) forbids those without an honest majority, and even *with* one, malicious-with-abort
is the cheap tier; GOD-against-malicious (Goyal–Song–Liu) is a strictly heavier add-on tracked
separately. The Cleve invariant is already encoded in the type system
(`backend.rs:261–297`, `OutputGuarantee::fairness`/`guaranteed_output` gated on an honest
majority); this design must not violate it.

---

## 0. Ground truth: where the crate is TODAY (verified against `origin/main`)

The relevant facts, re-verified against the live source (all citations are `origin/main`
line numbers; the crate is an **in-process multi-party simulation** — every "party" is a
function call, no real network):

- **Field.** `F_p`, `p = 2^61 − 1` (Mersenne), `u128` products folded by `2^61 ≡ 1`
  (`field.rs`). Statistical-security parameter of any IT-MAC over this field is therefore
  bounded by `≈ 1/p ≈ 2^-61` per MAC (see §2.5 — this is the one genuine *parameter* choice).
- **Shamir.** `t`-of-`n`, honest-majority `t = ⌊(n−1)/2⌋` (`shamir.rs:160–168`). Linear ops
  (`add_shares`, `sub_shares`, `scale`, `add_constant`) are **local / free** (`shamir.rs`
  ~586–650). The cumulative-SUM aggregate `run_secure` is zero-round local addition
  (`shamir.rs:794–803`).
- **Multiplication.** `mul_shares_raw` (`shamir.rs:670`) is the *component-wise* product →
  a **degree-`2t`** sharing of `a·b`, NO degree reduction in itself. The BGW/DN
  reshare-and-recombine **degree-reduction round** now EXISTS (`degree_reduce`,
  `shamir.rs:406`; bead **sq-dvuc CLOSED**) — so multiplication chains are now buildable. The
  reduction's re-sharing step is `shamir.rs:457–458` (each recombination party re-shares its
  own degree-`2t` share under a fresh degree-`t` polynomial).
- **Reconstruction / open.** `reconstruct` (degree-`t`, `shamir.rs` ~565) and
  `reconstruct_degree` (`shamir.rs:714`, used for the degree-`2t` equality open) both route
  through the RS / Berlekamp–Welch checker `robust::reconstruct_robust` (`robust.rs:81`). This
  gives **detect-and-abort** (and robust correction up to `e = ⌊(n−degree−1)/2⌋`) **whenever
  there is RS redundancy** — but **NONE at exactly `degree+1` shares** (`robust.rs:36–53`,
  pinned by a boundary test).
- **Equality / hidden join.** `secure_equal` (`join.rs:411`): shares `a`,`b`,`r` (nonzero
  mask); `d = a − b` (local, `join.rs:430`); `m = d·r` (one multiplication, degree `2t`,
  `join.rs:432`); open `m` at degree `2t` (`join.rs:434`); `m == 0 ⇔ a == b`. The all-pairs
  `HiddenValueJoin::join` loops `secure_equal` over `|L|·|R|` pairs (`join.rs:459–467`).
  `HiddenValueJoin::join` was removed; see `HiddenValueJoin::batched_join`.
- **Comparison / threshold.** The secret-shared greater-than for the £100k verdict
  (**sq-rrz4, OPEN**) is **NOT yet realized in-crypto**: `OperatorClass::Comparison` reports
  `semi_honest_only` (`shamir.rs:250`, `backend.rs:518–522`) and the only in-crate comparator
  over secret values is the **test-only, INSECURE** `SimulatedSecretComparator`
  (`oblivious.rs:962–986`, which *reconstructs* both operands in the clear). The
  degree-reduction prerequisite (sq-dvuc) is now in place, so a real Rabbit/edaBits comparison
  is now *buildable* — but it does not exist yet, and when it lands it is semi-honest unless it
  carries MACs (this design covers exactly that).
- **Backend registry.** Three orthogonal axes (`backend.rs:106–358`): `AdversaryModel`
  (`SemiHonest | Covert | Malicious`), `OutputGuarantee` (`Abort(AbortKind) | Fairness | GOD`,
  Cleve-gated), `CorruptionThreshold` (`Dishonest | Honest | SuperHonest`), composed into
  `SecurityDescriptor` (`backend.rs:374–386`), reported backend-level via `BackendInfo`
  (`backend.rs:617–641`) and **per-operator** via `operator_security` (`backend.rs:712`,
  `shamir.rs:760–762`). Today **every** Shamir operator reports `adversary: SemiHonest`
  (`backend.rs:430`, `:443`) — the active-security hardening that *does* exist (RS detect/
  correct) is surfaced on the *output-guarantee* axis, NOT the adversary axis. This is the
  honest framing: RS redundancy hardens the *open*, but the parties are still trusted to feed
  consistent shares and re-sharings.
- **Metrics / transport harness.** `metrics.rs` (deterministic Tier-1 counters:
  `field_elements_opened`, `field_elements_shared`, `multiplications`, rounds) and
  `transport.rs` (Tier-2/3) EXIST (sq-5gnv / sq-sxm / sq-tg6b). The cost of authentication is
  therefore *measurable* by the existing harness, not hypothetical (see §5).

---

## 1. The honest hole, precisely (where semi-honest is load-bearing)

The crate's confidentiality + correctness rest on **every party following the protocol**. RS
redundancy (`robust.rs`) closes the active-security gap *for reconstructions that carry
redundancy*, but it cannot close it where there is none, and it does nothing for *deviations
that are not a tampered open value* (a wrong re-sharing, a wrong product input). The four
load-bearing semi-honest assumptions, each with file:line and the exact undetected deviation:

### Hole 1 — the degree-`2t` equality open at minimal `n = 2t+1` (the headline)

`secure_equal` opens the masked product `m = d·r` at degree `2t` (`join.rs:434` →
`shamir::reconstruct_degree`, `shamir.rs:714` → `robust::reconstruct_robust` at `degree = 2t`).
A degree-`2t` sharing over `n` points is an `[n, 2t+1]` RS codeword, so redundancy exists only
when `n > 2t+1`. The honest-majority constructor fixes `t = ⌊(n−1)/2⌋`, so for **odd `n`
(3,5,7,9 — the minimal, cheapest configuration) `n = 2t+1` exactly**: **ZERO RS redundancy at
degree `2t`** (`robust.rs:47–53`, pinned by a boundary test). A malicious party can add any
offset `δ` to its product share `h_i = d(x_i)·r(x_i)` and the opened `m` shifts to a *different
degree-`2t` polynomial's* value at 0; there is no second codeword to disagree with, so the
forgery is **information-theoretically undetectable** and **silently flips a join/equality
verdict** (a non-match becomes a match or vice-versa). Worse, per coZK eprint 2025/1026, opening
a value computed on an *inconsistent witness* can **leak honest inputs** — so this is a
**confidentiality** hole, not only a correctness one. Cited as the deferred MAC seam in
`robust.rs:47–53`, the `reconstruct_degree` docs (`shamir.rs:692–713`), and `backend.rs:514–518`
(`OperatorClass::EqualityJoin` reports `SemiHonestOnly` at `n=2t+1`). **This is the one cell
sq-km34 exists to close.**

### Hole 2 — the `degree_reduce` re-sharing has no consistency check

`degree_reduce` (`shamir.rs:406`) is the BGW reshare-and-recombine round. Step 2
(`shamir.rs:457–458`) has each of the `2t+1` recombination parties **re-share its own
degree-`2t` share `h_i` under a fresh degree-`t` polynomial** (`self.share(s.y)`). Step 3
locally recombines with public Lagrange weights `λ_i`. The protocol **assumes each party
re-shares its TRUE `h_i`**. A malicious party can instead re-share `h_i + δ` (or any value):
the recombination then produces a degree-`t` sharing of `a·b + λ_i·δ` — a **silently wrong
product**, fully consistent (it is a clean degree-`t` sharing), so the *eventual* RS-checked
open at degree `t` finds nothing wrong. The crate's own doc states this honestly
(`shamir.rs:392–406`-area: "NOT maliciously secure: a deviating party can feed an inconsistent
re-sharing, and … there is no in-protocol check here that detects it"). This is the deviation
that breaks *any* multiplication chain (comparison, conjunctive joins, segmented group-agg),
and it is undetectable **even at over-provisioned `n`**, because the wrong value is re-shared
as a perfectly consistent codeword. **RS redundancy cannot help here at all** — the tampering
is not on the open, it is on the *value being shared*. Only a MAC on the multiplied value
catches it.

### Hole 3 — the masked-open value `r` (and `m`) in the hidden join

Two sub-deviations in `secure_equal` (`join.rs:411–436`), beyond Hole 1's open:
- The mask `r` is dealt by the (simulated) dealer (`join.rs:428`, `dealer.share(mask_value)`).
  In the in-process sim a single dealer is honest by construction; in a *real* federation the
  mask must be **jointly** generated (PRSS / coin-toss — bead sq-yyro). A party that biases
  `r` toward 0 (or learns it) can force `m = 0` (a false match) or learn `d` (the key
  difference, a confidentiality break). The crate rejects a *literal* zero mask
  (`draw_nonzero_fp`, `join.rs:427`) but not a *maliciously chosen* one.
- The inputs `a`,`b` to the equality are *shared* by the dealer (`join.rs:425–426`). A
  malicious holder could share an inconsistent (not degree-`t`) input. RS redundancy on the
  degree-`t` *input* sharing would catch this at `n > t+1`, but the **inputs are never opened**
  (only `m` is), so the inconsistency is never checked — it just propagates into `m`. A MAC on
  the *input shares*, checked at the final open, catches it.

### Hole 4 — the comparison's bit operations (when sq-rrz4 lands)

The secure greater-than (sq-rrz4) is multiplication-*depth* > 1 (bit-decomposition + prefix-OR
/ carry chains — Rabbit eprint 2021/119 / edaBits Crypto'20). It is **not built** yet
(`oblivious.rs:953–986` is a test-only insecure stand-in), but when it is, it will be a chain
of: (a) `mul_shares_raw` products (Hole-1-style degree-`2t` intermediates), (b) `degree_reduce`
rounds (Hole-2 re-sharings), (c) bit-multiplications (each a product → reduce), and (d) one
final boolean open. **Every** product and **every** reduction in that chain is a place a
malicious party can inject an undetected offset, and the final boolean open at minimal `n`
has the same zero-redundancy problem as Hole 1. So the comparison is *the worst case*: a deep
arithmetic circuit, every gate of which needs authentication for the boolean verdict to be
trustworthy. This is why malicious comparison is its own bead (§6) and depends on the
authenticated mul/reduce primitives.

### Summary table — the holes and what catches them

| # | Where (file:line, `origin/main`) | Undetected deviation | RS redundancy help? | Fix |
|---|---|---|---|---|
| 1 | `join.rs:434` → `shamir.rs:714` (degree-`2t` open at `n=2t+1`) | forge product share → flip match bit; leak (coZK 2025/1026) | **No** (zero redundancy at `n=2t+1`) | IT-MAC on the product, MAC-check before open |
| 2 | `shamir.rs:457–458` (`degree_reduce` re-share) | re-share `h_i+δ` → wrong product, consistent codeword | **No** (tampering is on the value, not the open) | MAC carried through the reduction; check at output |
| 3 | `join.rs:425–428` (input shares + mask in `secure_equal`) | biased/known mask `r`; inconsistent input never opened | partial (only if inputs were opened — they aren't) | MAC on inputs; jointly-generated mask (sq-yyro) |
| 4 | `oblivious.rs:953–986` / future sq-rrz4 comparison | any product/reduce in the bit-decomp chain | **No** (chain of Holes 1+2) | authenticated mul/reduce end-to-end + MAC-checked boolean open |

---

## 2. The IT-MAC / authenticated-secret-sharing construction

The standard fix (SPDZ family — Damgård et al. CRYPTO'12; the honest-majority specialisation
is "authenticated Shamir", and Goyal–Song eprint 2020/134 "malicious security comes free in
honest majority"): attach an **information-theoretic MAC** to every secret-shared value, so any
tampered share/open/re-sharing changes the value *without* changing its MAC consistently, and a
**single batched MAC-check at output time** catches it with probability `≥ 1 − negl`. Below,
adapted to sparq's Shamir-over-`F_p` (`p = 2^61−1`) in-process simulation.

### 2.1 What the MAC key is, and how it is shared

Pick a single **global MAC key** `α ∈ F_p`, *itself secret-shared* across the `n` parties as a
degree-`t` Shamir sharing `[α]` — **no party knows `α`**. (SPDZ uses additive shares of `α`;
the honest-majority Shamir analogue is a degree-`t` Shamir sharing of `α`, consistent with the
rest of the backend. One global `α` for the whole session is the standard, cheapest choice;
per-value keys are not needed.) In the in-process simulation, the dealer (`ShamirDealer`,
`shamir.rs:294`) draws `α` once per session from the masking RNG (OS-seeded ChaCha20 in
production, `rng.rs`) and shares it; **we simulate the MAC key and the checks across the
simulated parties** exactly as we simulate the shares today — no new transport is required for
Tier-1 correctness (the *cost* is what Tier-2/3 measures, §5). In a real federation `α` is
generated jointly (PRSS / coin-toss, sq-yyro), never dealt by one party.

### 2.2 The authenticated share type

An authenticated sharing of a secret `x ∈ F_p` is the pair:

```text
[[x]] = ( [x] , [m_x] )      where  m_x = α · x   (the MAC),  both degree-t Shamir sharings.
```

So an authenticated value carries **two** Shamir sharings (the value and its MAC), i.e. the
share *storage roughly doubles* (§5). The MAC `m_x = α·x` is the authenticator: a party cannot
change its share of `[x]` to flip `x` *and* simultaneously fix `[m_x]` to still equal `α·x`,
because it does not know `α`. This is the new `MpcBackend::Share` for a malicious backend — and
crucially it hides behind the existing associated `type Share` (`backend.rs:696`), so the join
and proof layers compose onto it unchanged (the registry already documents this absorption,
`backend.rs:47–62`).

### 2.3 Carrying MACs through ADDITION (and linear ops) — FREE

The MAC is linear in the value, and the field is linear, so authentication is free under all of
sparq's local linear ops:

```text
[[x]] + [[y]] = ( [x]+[y] , [m_x]+[m_y] )            since α·(x+y) = α·x + α·y
c · [[x]]      = ( c·[x]   , c·[m_x] )                since α·(c·x) = c·(α·x)
[[x]] + c      = ( [x]+c   , [m_x] + c·[α] )          public constant: MAC of c is α·c, and
                                                       [α] is shared, so c·[α] is local
```

This maps directly onto the existing free local ops `add_shares` (`shamir.rs:586`), `scale`
(`shamir.rs:623`), `sub_shares` (`shamir.rs:634`), `add_constant` (`shamir.rs:611`): each is
applied **twice** (once to `[x]`, once to `[m_x]`), with the public-constant MAC term using the
shared `[α]`. So the **zero-round cumulative-SUM aggregate** (`run_secure`, `shamir.rs:794`)
stays zero-round; it just also maintains the MAC sharing. No new interaction. This is the
"malicious comes free in honest majority" property for linear circuits.

### 2.4 Carrying MACs through MULTIPLICATION — the authenticated degree-reduce / Beaver step

Multiplication is where authentication needs real work, because `α·(x·y) ≠ (α·x)·(α·y)` and the
degree reduction is itself a deviation surface (Hole 2). Two interoperable routes, both fitting
the existing in-process structure:

**(a) Carry the MAC forward through the INPUT MAC — two independent mult-then-reduce rounds
(the honest-majority route, no preprocessing).** Today `secure_equal` does `mul_shares_raw`
(`join.rs:432`) then opens at degree `2t`; the chained path does `mul_shares_raw` then
`degree_reduce` (`shamir.rs:406`). To authenticate the product `z = x·y` the parties run **two
independent mult-then-reduce rounds over DIFFERENT input shares**:

- the value `[z] = reduce([x]·[y])`;
- the MAC `[m_z] = [α·z] = reduce([α·x]·[y])`, from the **input** MAC `[α·x]` times the **input**
  value `[y]` — correct because `(α·x)·y = α·(x·y) = α·z`.

This doubles the multiplication cost (value-mult + MAC-mult), reuses `degree_reduce` verbatim,
and is the standard honest-majority malicious-multiplication shape (Chida et al., "Fast
Large-Scale Honest-Majority MPC for Malicious Adversaries", CRYPTO'18).

**Why this covers Hole 2 — the INDEPENDENCE is what is load-bearing.** Adversary model:
static, up to `t` corruptions with `n ≥ 2t+1`, rushing, and *fully coordinated* — the corrupted
parties may deviate in **both** reduces of the same multiplication, choosing the deviations
jointly and adaptively on their own views. A party deviating inside a re-sharing shifts that
reduce's secret by a value it controls; write `δ_v` for the net shift of the value reduce and
`δ_m` for the net shift of the MAC reduce. Because the two reduces consume *disjoint* input
sharings (`[x]·[y]` vs `[α·x]·[y]`), the pair the adversary lands on is
`(z + δ_v, α·z + δ_m)`, and the §2.5 check computes

  `σ = m_z − z·α = (α·z + δ_m) − (z + δ_v)·α = δ_m − α·δ_v`.

So `σ = 0` iff `δ_m = α·δ_v` — for any `δ_v ≠ 0` that pins `α`, which the adversary's view is
independent of (`≤ t` shares of `[α]` are independent of `α`), so it succeeds with probability
`1/p ≈ 2^-61`. Tampering only the value reduce (`δ_m = 0`) gives `σ = −α·δ_v ≠ 0`; only the MAC
reduce (`δ_v = 0`) gives `σ = δ_m ≠ 0`. The re-sharing in `degree_reduce` (Hole 2) is therefore
**MAC-covered in both rounds, including under coordinated deviation** — not because the reduce
gained a check, but because the MAC is derived from inputs the value reduce never touched.

> **REJECTED variant — recompute the MAC from the reduced value (this record's original
> route (a); do NOT implement).** An earlier draft of this section specified the "cheapest"
> shape: after obtaining `[z]`, get `[m_z]` by **one extra multiplication of `[z]` by the shared
> `[α]`**, i.e. `[α·z] = reduce([z]·[α])`. **That construction does not close Hole 2 and must not
> be used.** Its claimed key point ("the *same* MAC is computed over the *reduced* result, so a
> wrong re-sharing breaks `m_z = α·z`") is backwards: computing the MAC *from* the tampered `[z]`
> makes the MAC **track** the tamper. With a value-reduce shift `δ_v` the pair is
> `(z + δ_v, α·(z + δ_v))`, so `σ = 0` and the batched check **passes on a wrong product** — a
> silent incorrect result, exactly the failure Hole 2 describes. (It still catches a deviation in
> the second reduce, `σ = δ_m`; that is not enough.) Only a MAC computed from inputs *independent*
> of the value reduce covers the first round, which is why route (a) is specified as
> `reduce([α·x]·[y])` above. `[SONNET-4.6] 2026-07-26 — correction made during the sq-km34.3
> implementation review; the implementation was already the independent-input construction, this
> record was not.`

**Where this lives in code, and what pins it.** `MacSession::auth_mul` (`shamir.rs`) implements
the two-independent-reduce route above. The discrimination is pinned by
`auth_compare::tests::mac_carry_soundness_distinguished_by_in_reduce_tamper` (bead sq-81gd),
which injects the genuine Hole-2 deviation *inside* the value re-sharing and asserts the two
carries diverge: the independent-input MAC aborts, the rejected `[z]·[α]` recompute passes. The
coordinated-`(δ_v, δ_m)` case above is pinned by
`auth_compare::tests::coordinated_tamper_in_both_reduces_is_caught`. Note the honesty caveat that
applies to this whole record: none of it carries external accredited-cryptographer sign-off
(`sq-qhy4`); the argument here is an internal one.

**(b) Authenticated Beaver triples (the SPDZ-canonical route, and what a dishonest-majority
backend would share).** Preprocess authenticated triples `([[u]],[[v]],[[w]])` with `w = u·v`.
Online multiplication of `[[x]]·[[y]]`: open `ε = x−u` and `ρ = y−v` (both masked by fresh
preprocessed randomness, so they leak nothing), then
`[[z]] = [[w]] + ε·[[y]] + ρ·[[x]] + ε·ρ`, all **linear** post-opening → the MAC is carried for
free by §2.3, and the two openings (`ε`,`ρ`) are themselves MAC-checked. This is the route that
generalises to dishonest-majority (the triples come from OT/SHE then, not from honest-majority
resharing) and is the natural home for the `requires_preprocessing` cost field (sq-4i39). For
the **honest-majority** target of *this* bead, route (a) is sufficient and avoids a
preprocessing phase; route (b) is documented as the dishonest-majority continuation (sq-j5ok).

Either way, the invariant is: **every multiplication produces an authenticated result
`([z], [α·z])`**, and the degree-reduce re-sharing is no longer trusted — its correctness is
*checked* via the MAC at output.

### 2.5 The batched MAC-check at OUTPUT time (the one place that catches everything)

Authentication is only as good as the check. The check happens **once, just before any value is
opened** (the equality verdict `m`, the aggregate sum, the comparison boolean — whatever
`reconstruct_disclosed` / `reconstruct_degree` is about to reveal). The SPDZ batched check,
adapted to Shamir-over-`F_p`:

1. Suppose the parties are about to open values `y_1..y_k` (in `secure_equal` there is one
   per pair: the masked product `m`). Each has an authenticated sharing `[[y_j]] = ([y_j],
   [m_{y_j}])` with `m_{y_j} = α·y_j`.
2. Draw public random challenge coefficients `χ_1..χ_k ∈ F_p` (Fiat–Shamir / a jointly-tossed
   coin), derived *after* shares are fixed, so a party cannot adapt to it. **As implemented
   (sq-km34.4):** Fiat–Shamir — a domain-separated SHA-512 transcript over `(n, t, k)` and both
   halves `([y_j], [α·y_j])` of every authenticated sharing under check, expanded to one `χ_j`
   per value and folded into `F_p`. That makes "after the shares are fixed" *structural*: the
   coin is a function of exactly those shares, so it cannot precede them and re-randomises under
   any tamper — where a draw from the dealer's private RNG gave that by call ordering only.

   **What is NOT implemented (scope, honesty).** The transcript is the *complete* share
   vectors, which exist in one place only because the backend is an in-process simulation of
   all `n` parties; a real party holds only its own share, and broadcasting the vectors would
   reconstruct the opened values and leak MAC-sharing material. So the implemented coin is
   **not jointly derivable by the protocol parties and does not remove the trusted dealer** —
   it is scoped to the trusted-dealer simulation, and the tests (one `MacSession` holding every
   share) are correspondingly silent on the distributed property. A genuinely public coin needs
   the step this crate does not yet have: each party broadcasts a *binding commitment* to its
   own fixed shares (or the parties run commit-then-reveal coin tossing), then
   `χ = H(commitments)`, with specified broadcast, verification and abort behaviour — a
   different transcript needing its own soundness argument.

   **Grinding (correction).** An earlier revision of this section claimed testing a candidate
   `χ` requires the secret `α`. That is false in general: with the MACs left unchanged
   (`δ_m = 0`) the deficit is `σ = −α·Σ_j χ_j·δ_{y,j}`, so `σ = 0` iff `Σ_j χ_j·δ_{y,j} = 0` —
   a relation between the hash-derived `χ` and the attacker's own deviations, checkable without
   `α`. The `≥ 1 − 1/p` bound below therefore holds against a deviation fixed *before* `χ` is
   known; an adversary that can vary the transcript and re-derive the coin grinds at `≈ 1/p`
   per trial, i.e. `≈ p ≈ 2^61` expected work. In a 61-bit field that is a **computational**
   bound, not the information-theoretic one step 5 states. The distributed protocol above must
   therefore bind each contribution before `χ` is derived; until it exists with a written
   argument over a stated adversary model, the ideal-public-coin claim is not made here.
3. Open the values `y_j` (the actual results). Compute the public linear combination
   `y = Σ_j χ_j·y_j`.
4. Each party locally forms its share of `[σ] = Σ_j χ_j·[m_{y_j}] − y·[α]` (all linear → free,
   §2.3), then the parties **open `σ`**.
5. **Accept iff `σ == 0`.** Because `σ = Σ_j χ_j·(α·y_j) − (Σ_j χ_j·y_j)·α = 0` for honest
   values; any party that tampered with a share/open/re-sharing changed some `y_j` *without* a
   consistent matching change to `m_{y_j}` (it cannot, not knowing `α`), so `σ ≠ 0` with
   probability `≥ 1 − 1/p ≈ 1 − 2^-61` over the random `χ`. (The single-MAC soundness is `1/p`;
   the random-linear-combination batching keeps it `≈ 1/p`, the field-size statistical
   parameter.) This is the ideal-coin bound, and it is stated against a deviation fixed before
   `χ` is known — see step 2's grinding note for what the Fiat–Shamir instantiation actually
   gives.
6. **`σ ≠ 0` ⇒ ABORT** (`MpcError::Tampered` / a new `MpcError::MacCheckFailed`). No value is
   trusted; the protocol returns an error rather than a wrong/leaky answer.

**Why this closes all four holes:** the check binds the *opened values* to the *MAC sharing*,
which was carried correctly through every linear op (free) and every multiplication (the MAC
was re-multiplied, §2.4). A tampered open (Hole 1), a wrong re-sharing (Hole 2), an inconsistent
input or biased mask that changes the effective value (Hole 3), or any product/reduce in the
comparison chain (Hole 4) all change `y_j` away from the value whose MAC was authenticated, and
the check fires. Crucially, **the check itself works at `n = 2t+1`** — it needs no RS
redundancy, because soundness comes from the *secret* `α`, not from over-determination of the
codeword. That is exactly why RS (`robust.rs`) could not close Hole 1 and the MAC can.

**Confidentiality-before-open discipline (coZK 2025/1026).** The MAC-check at step 4–5 opens
`σ`, a value that is **zero for honest executions and reveals nothing about the inputs** (it is
a random-coefficient combination of MACs minus the same of values; identically zero when
correct). So unlike opening `m` on an inconsistent witness, the check itself is leakage-free,
and — critically — it runs **before** the result `y_j` is *acted on*, so a detected tamper
aborts *before* the inconsistent-witness leak path of 2025/1026 can be exploited. This is the
direct mitigation of the §4.1 (D × A) malicious-confidentiality interaction the security-models
doc flags.

---

## 3. What it upgrades, by operator (and the residual limits)

Mapping onto `OperatorClass` (`backend.rs:509–522`), with the precise tier each reaches under
this design at the honest-majority default `t = ⌊(n−1)/2⌋`:

| Operator (`OperatorClass`) | Today (`origin/main`) | With IT-MACs | Residual limit |
|---|---|---|---|
| **LinearAggregate** (SUM/COUNT, `run_secure` `shamir.rs:794`) | SemiHonest adversary; RS detect/robust on the degree-`t` open (already has redundancy) | **Malicious, abort** at *every* valid `(n,t)` incl. `n=2t+1`: MAC-check before the single open. The "near-frontier" aggregate becomes malicious-secure end-to-end. | abort (a cheater forces abort); no GOD-vs-malicious unless Goyal–Song–Liu added |
| **EqualityJoin** (`secure_equal`/`HiddenValueJoin` `join.rs:411`) | SemiHonest at `n=2t+1` (Hole 1, the headline); RS detect/abort only at `n>2t+1` | **Malicious, abort** at *minimal* `n=2t+1`: the masked product `m` is authenticated and MAC-checked before open; forged product share / wrong reduce / inconsistent input all caught. **This is sq-km34's core promotion: `SemiHonestOnly → Abort` at minimal N.** | per-pair match-bit leak (L2) is a *confidentiality* axis, orthogonal — fixed by sq-jnkm, not by MACs; abort only |
| **Comparison** (sq-rrz4, currently `semi_honest_only` `shamir.rs:250`) | not in-crypto (insecure test stand-in `oblivious.rs:962`) | **Malicious, abort** *when built on authenticated mul/reduce*: every gate in the bit-decomp chain authenticated, boolean verdict MAC-checked before open. | depends on sq-rrz4 landing first; abort only; round-per-depth (WAN-wrong, sq-38zk) |
| **Hidden join (set-returning)** | semi-honest, all-pairs | **Malicious, abort** for *correctness of the match bits*; combine with sq-jnkm (oblivious result-size + match-bit aggregation) for the confidentiality side | obliviousness (L1/L2) is a separate axis; abort only |

**The achieved security tier, named precisely (honesty anchor):**
**Honest-majority, malicious-with-abort, unanimous abort.** On the three-axis registry
(`backend.rs`):
- AXIS-1 `AdversaryModel::Malicious` (the upgrade — today every operator is `SemiHonest`).
- AXIS-2 `OutputGuarantee::Abort(AbortKind::Unanimous)` — detect-and-abort, *not* identifiable
  (true IA needs authenticated per-party transcripts + broadcast the in-process sim lacks; the
  heuristic `Tampered{cheaters}` blame must NOT be advertised as IA — `backend.rs:194–199`),
  *not* fairness/GOD-against-malicious.
- AXIS-3 `CorruptionThreshold::HonestMajority` / `SuperHonestMajority` — **unchanged**; this
  design does **not** move to dishonest-majority.

**What it does NOT give (state loudly):**
- **NOT dishonest-majority.** Soundness of honest-majority authenticated Shamir relies on `≤ t`
  corruptions for *privacy* (the `[α]`, `[x]` sharings) and on the honest majority for the
  degree-reduce/recombine. A dishonest majority needs full SPDZ (additive shares + OT/SHE
  triples + the same MAC machinery but *no* honest-majority reduction) — a different backend
  (sq-j5ok), truthfully refused by the registry today.
- **NOT fairness / GOD against a malicious adversary.** Cleve (STOC'86) forbids fairness/GOD
  without honest majority; even *with* honest majority, malicious-with-abort is the cheap tier,
  and GOD-against-malicious (Goyal–Song–Liu, eprint 2020/189) is a strictly heavier compiler we
  do not build here. A single cheater can force an abort. (Note: the *existing* RS-robust path
  gives GOD against a *semi-honest-modelled* tamper up to `e` cheaters on the degree-`t` open;
  that is a different and weaker claim than GOD-against-malicious and lives on the
  output-guarantee axis already.)
- **NOT a fix for the confidentiality leaks L1/L2/L4** (result cardinality, per-pair match
  graph, source linkability — security-models doc §4.1). Those are the *obliviousness* axis
  (sq-jnkm/sq-y32f/sq-shk5), orthogonal to malicious security. MACs make the *match bits
  correct and tamper-evident*; they do not hide them.

---

## 4. Fit to the backend registry

The registry is built for exactly this (`backend.rs:35–65` "how the trust model stays
CONFIGURABLE"; the share type absorbs the scheme change). Two viable shapes; recommend (B):

**(A) A new `MaliciousShamirBackend` type.** A sibling of `ShamirBackend` whose
`type Share = AuthenticatedShareVec` (`([Share], [Share])`), implementing `MpcBackend`
(`backend.rs:693`) with the authenticated add/mul/reduce/open of §2. Clean separation, but
duplicates the dealer/sharing plumbing.

**(B, recommended) A malicious *mode* on the existing backend** — `ShamirBackend` gains a
private `adversary_model: AdversaryModel` field (default `SemiHonest`, set via a
`new_malicious(n)` constructor), and a parallel authenticated share path. Rationale: the
sharing, dealer, RNG, degree-reduce, and RS machinery are shared verbatim; only the MAC
maintenance + batched check are new. The associated `type Share` becomes an enum
(`Plain(ShareVec) | Authenticated(AuthShareVec)`) — or, cleaner, a malicious backend exposes
`type Share = AuthShareVec` while the existing semi-honest backend keeps `ShareVec`, both behind
the same trait, selected by the registry. Given the existing code's tight reuse of `dealer()`
and `degree_reduce`, a thin `Authenticated` wrapper that *contains* a `ShamirDealer` and adds
`[α]` + MAC arithmetic is the smallest change.

**Per-operator reporting (the descriptor change).** `operator_descriptor` (`shamir.rs:222`)
becomes adversary-aware. Today it returns `SemiHonest` + an output-guarantee derived from RS
redundancy. A malicious-mode backend returns, **for every operator including EqualityJoin at
`n=2t+1`**:

```text
SecurityDescriptor {
    adversary: AdversaryModel::Malicious,              // the upgrade
    output_guarantee: OutputGuarantee::Abort(AbortKind::Unanimous),  // MAC-check → unanimous abort
    threshold: CorruptionThreshold::from_n_t(n, t),    // unchanged (honest/super-honest)
    public_verifiability: PublicVerifiability(false),  // unless a coZK-public check is added
}
```

This requires a **new descriptor constructor** beside `shamir_degree_recon` /
`semi_honest_only` (`backend.rs:388–447`) — e.g. `SecurityDescriptor::authenticated_abort(n, t)`
— that builds the `Malicious` + `Abort(Unanimous)` descriptor and **fails closed under a
dishonest majority** exactly as `shamir_degree_recon` does (`backend.rs:411–413`): an authenticated
*honest-majority* claim must degrade rather than over-claim if `n ≤ 2t`. The back-compat
`malicious_security` projection (`backend.rs:479–498`) maps `Malicious + Abort(Unanimous)` →
`MaliciousSecurity::HonestMajorityAbort` — note the *projection already collapses the adversary
axis*, so the old enum reads correctly without change (the richer truth — that the *adversary*
is now malicious, not merely that the open is RS-checked — lives in `security.adversary`).

**Fail-closed selection still refuses the impossible cells.** The `SecurityRequirement` +
fail-closed registry (sq-a6p1, CLOSED) compares a federation's `min_adversary` /
`min_output_guarantee` / `max_corruption` against `BackendInfo` / `operator_security`. After
this design:
- A request for `min_adversary = Malicious`, `max_corruption = HonestMajority` over
  EqualityJoin is **now satisfiable** at minimal `n=2t+1` (was refused / downgraded before).
- A request for `min_adversary = Malicious`, `max_corruption = DishonestMajority` over *any*
  SPARQL operator is **still refused** (`NoBackendSatisfies`) — no authenticated *Shamir*
  backend is dishonest-majority; that needs the SPDZ backend (sq-j5ok) which has no impl. The
  registry must not silently downgrade.
- A request for `OutputGuarantee::Fairness`/`GuaranteedOutput` *against a malicious adversary*
  is **refused** for this backend (we provide abort, not GOD-against-malicious) — and Cleve
  already blocks it under dishonest-majority at the type level (`backend.rs:261–297`).

---

## 5. Cost (qualitative; the harness measures it — no fabricated numbers)

Authentication's overhead vs semi-honest, by mechanism (all **measurable** by the existing
`metrics.rs` deterministic counters and `transport.rs` Tier-2/3 emulation — sq-5gnv/sq-sxm/
sq-tg6b — so these are *predictions the harness will confirm*, not claims):

- **Share storage ≈ 2×.** Every authenticated value carries `([x], [m_x])` (§2.2) → roughly
  doubles `field_elements_shared` and in-memory share storage. (One-time `[α]` sharing is
  negligible.) Counted directly by `metrics.rs`'s `field_elements_shared`.
- **Linear ops: no extra rounds, ~2× local work.** Add/scale/sub applied to value *and* MAC
  (§2.3); still **zero communication rounds** for the SUM aggregate. `multiplications` counter
  unchanged for linear ops (stays 0). The aggregate stays the zero-round sweet spot.
- **Multiplication: ~2× the multiplication cost + 1 MAC-mult per product.** Route (a)
  (§2.4): each secure product runs a second, independent `[α·x]·[y]` product → an extra
  `mul_shares_raw` + `degree_reduce` per multiplication, roughly **doubling `multiplications`
  and the degree-reduce rounds**. Route (b) (Beaver) shifts this into an **offline
  preprocessing** phase (record it explicitly via the `requires_preprocessing` field, sq-4i39,
  so it is not a hidden cost — the Ozdemir-Boneh / ORQ lesson). For the equality path
  specifically: one product → one MAC-mult, so `secure_equal` roughly doubles its `multiplications`
  and per-pair `field_elements_opened` (the masked product *and* its participation in the batched
  MAC open).
- **Output: one batched MAC-check.** §2.5 is **one extra open of `σ`** for an entire batch of
  results (amortised — the all-pairs join's `|L|·|R|` equality opens share *one* batched check,
  not one each), plus the local `Σχ_j·[m_{y_j}] − y·[α]` (free). So the marginal `round_count`
  cost is **O(1) per opened batch**, not O(per-value) — cheap relative to the per-pair opens
  themselves.

**Headline (consistent with capability-matrix §4.4, do not contradict it):** at **honest
majority** the malicious upgrade is **close to free** — linear ops free, multiplication ~2×,
output +1 batched check, no preprocessing under route (a). The expensive regime is
*dishonest-majority* (SPDZ preprocessing tax), which this design does **not** enter. The
existing harness should add an AXIS-1 dimension (semi-honest vs malicious) to the
(model × N × query) matrix so the lift is reported, never asserted.

---

## 6. Sequenced implementation plan → follow-up beads

Dependency-ordered (each is a small, independently-reviewable, differential-tested deliverable,
per the smallest-context-independent-deliverable discipline). Beads filed under sq-km34 / epic
sq-pwr — ids recorded in the report:

1. **Authenticated sharing type + shared MAC key `[α]`.** `AuthenticatedShare = ([x],[α·x])`;
   `ShamirDealer` mints + shares one `α` per session and produces authenticated sharings.
   Acceptance: authenticated round-trip; `[α]` never reconstructed; any `≤ t` views independent
   of `α`. (Foundation; everything depends on it.)
2. **MAC-carrying add / scale / sub / add-constant.** Apply each existing local linear op
   (`shamir.rs:586–650`) to value *and* MAC, with the public-constant term using `[α]`.
   Acceptance: authenticated SUM aggregate round-trips; MAC stays consistent; zero extra rounds.
   Depends on (1).
3. **MAC-carrying multiplication + authenticated degree-reduce.** Route (a): alongside the
   value's `mul_shares_raw` + `degree_reduce`, compute `[α·z] = reduce([α·x]·[y])` via a second
   mult-then-reduce over the INPUT MAC (not from the reduced `[z]` — see the rejected variant in
   §2.4); the re-sharing (Hole 2) becomes MAC-covered in both rounds. Acceptance: `a·b·c` chain
   authenticated and tamper-evident; a wrong re-sharing in the value reduce, in the MAC reduce,
   or in **both** (coordinated) is caught by the §2.5 check. Depends on (1),(2), and
   sq-dvuc (CLOSED).
4. **Batched MAC-check at open (the catch-everything step).** §2.5: random-challenge batched
   check before `reconstruct_degree` / `reconstruct_disclosed`; `MpcError::MacCheckFailed` on
   mismatch; leakage-free `σ` open. Acceptance: a single batched check amortises `|L|·|R|`
   equality opens. Depends on (1)–(3).
5. **Malicious-secure equality / hidden join (sq-km34 core).** Wire (1)–(4) into `secure_equal`
   / `HiddenValueJoin` (`join.rs:411`,`:443`): authenticate `m = d·r`, MAC-check before open.
   Acceptance: `secure_equal` promotes `EqualityJoin` from `SemiHonestOnly → Abort` at
   `n=2t+1`; differential parity with plaintext join preserved. Depends on (1)–(4).
6. **Malicious-secure comparison (depends on sq-rrz4 + this stack).** When the secure
   greater-than lands (sq-rrz4), build it on authenticated mul/reduce so the boolean verdict is
   MAC-checked. Acceptance: secure verdict == plaintext `(sum > threshold)`, tamper in any gate
   aborts. Depends on (1)–(4) and **sq-rrz4**.
7. **Registry wiring + per-operator reporting.** `new_malicious(n)` / malicious mode;
   `SecurityDescriptor::authenticated_abort(n,t)` (fail-closed under dishonest majority);
   `operator_descriptor` (`shamir.rs:222`) returns `Malicious + Abort(Unanimous)` per operator;
   registry (sq-a6p1) satisfies a malicious-honest-majority request and still refuses
   malicious-dishonest-majority. Acceptance: registry select tests for both. Depends on (5)
   (and (6) for the comparison cell).
8. **Adversarial tests: a tampering party is CAUGHT.** Extend `adversarial_tests.rs`: a party
   that (a) forges a product share at `n=2t+1`, (b) feeds a wrong re-sharing in `degree_reduce`,
   (c) shares an inconsistent input, (d) biases the mask — each must `MacCheckFailed`-abort, and
   the honest path must still pass. The witness that the upgrade is real. Depends on (4)–(5).
9. **Cost reporting: add AXIS-1 (semi-honest vs malicious) to the metrics matrix.** Extend
   `metrics.rs`/`bench.rs` so the ~2× share / ~2× mult / +1 batched-check lift is *measured*
   across (N × query), per §5. Depends on (5); independent of (6).

(Optional continuation, NOT this bead: route (b) authenticated Beaver triples + the
`requires_preprocessing` field (sq-4i39) as the bridge toward the dishonest-majority SPDZ
backend (sq-j5ok) — documented as the next axis, out of scope here.)

---

## 7. Recommended construction, in one paragraph

Attach one information-theoretic MAC `m_x = α·x` to every secret-shared value under a single
session-global, secret-shared MAC key `[α]` no party knows; carry the MAC through addition and
scaling for free (apply each existing local linear op to both `[x]` and `[m_x]`), and through
each multiplication by a SECOND, independent mult-then-reduce over the input MAC,
`[α·z] = reduce([α·x]·[y])` (route (a), reusing the now-built BGW `degree_reduce`; *not* by
multiplying the reduced product by `[α]`, which would make the MAC track a tampered value —
§2.4) so the trusted re-sharing becomes MAC-covered; then, just before
any value is opened (the equality verdict, the sum, the comparison boolean), run **one batched
random-challenge MAC-check** — open a leakage-free `σ = Σχ_j·[m_{y_j}] − (Σχ_j·y_j)·[α]` and
abort iff `σ ≠ 0`. Because soundness comes from the secret `α` (`≈ 1 − 2^-61` per check over
`F_p`) and **not** from Reed–Solomon over-determination, this works at the minimal `n = 2t+1`
where the degree-`2t` equality open has zero RS redundancy — closing the one real semi-honest
hole (and the coZK-2025/1026 confidentiality interaction, by aborting *before* an inconsistent
value is acted on), promoting `secure_equal` from `SemiHonestOnly → Abort`, and yielding
**honest-majority malicious-with-abort** across the linear aggregate, equality/hidden-join, and
(once sq-rrz4 lands) the secure comparison — selectable and per-operator-reported through the
three-axis registry, while the registry still fails closed on the genuinely-impossible
dishonest-majority-malicious cells.

---

## Sources

Damgård–Pastro–Smart–Zakarias SPDZ (CRYPTO'12); MASCOT OT-triples (eprint 2016/505); Overdrive
SHE-triples (eprint 2017/1230); Goyal–Song "malicious security comes free in honest majority"
(eprint 2020/134); Goyal–Song–Liu "GOD comes free in honest-majority MPC" (CRYPTO'20, eprint
2020/189); Damgård–Nielsen DN07 (CRYPTO'07); ATLAS (eprint 2021/833); Cleve (STOC'86); Rabbit
(eprint 2021/119); edaBits (Crypto'20); Baum et al. identifiable abort (CRYPTO'20, eprint
2020/767); coZK malicious pitfalls / inconsistent-witness leak (CRYPTO'25, eprint 2025/1026);
PRSS (eprint 2021/1223). **In-repo ground truth (`origin/main`):**
`crates/sparq-mpc/src/{shamir,join,backend,robust,oblivious,metrics,transport}.rs`;
`research/{mpc-security-models-and-benchmarks,mpc-sparql-capability-matrix,
mpc-zkp-research-and-architecture,mpc-m4-distributed-sig-feasibility}.md`. Beads: sq-km34
(this), sq-dvuc (CLOSED, degree-reduce), sq-rrz4 (OPEN, secure comparison), sq-a6p1 (CLOSED,
fail-closed registry), sq-mq8q (CLOSED, 3-axis descriptor), sq-4i39 (preprocessing field),
sq-j5ok (dishonest-majority SPDZ backend), sq-jnkm/sq-y32f (obliviousness), sq-6d6g (the
original deferred-seam doc this design fulfils for the equality path).
