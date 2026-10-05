<!-- [OPUS-4.8] sq-toze / sq-egx6 — Constant-time / side-channel SOURCE-LEVEL review of
     sparq's bespoke crypto. Honesty-critical. Authored by Opus 4.8 while Fable 5 unavailable —
     re-review when Fable returns. This is ANALYSIS only; it changed NO crypto code. -->

# Cryptographic review — constant-time / side-channel analysis (CR-G5)

**Framework:** Cryptographic review of sparq's crypto estate (`sq-toze.20`).
**Bead:** `sq-egx6` (P2), addressing gap **CR-G5** in [`gap-register.md`](./gap-register.md).
**Companion docs:** [`README.md`](./README.md) (Tier-A/Tier-B assurance split),
[`controls.md`](./controls.md) (CR-14 documents the CT *posture*; this doc is its analysis pack),
[`evidence.md`](./evidence.md), [`fips-posture.md`](./fips-posture.md).

> ## Honesty banner — read before relying on anything here
>
> This is a **source-level (code-reading) review** of constant-time / side-channel exposure. It is
> **NOT** an instrumented timing study: no `dudect`, `ctgrind`, `cachegrind`, or hardware-counter
> measurement was run. An "appears constant-time" verdict below means *"no obvious secret-dependent
> branch, table index, or early-return was found by reading the source"* — it does **not** prove the
> absence of a timing or microarchitectural channel (compiler lowering, the underlying `arkworks`
> field arithmetic, and CPU behaviour are all out of source-level reach).
>
<!-- [OPUS-4.8] reconciled with post-remediation re-audit (sq-gbp4); see ZK-verdict cross-ref sweep -->
> A side-channel review **does not change** the headline of this framework: sparq's bespoke ZK/MPC
> crypto is **research-grade and externally unaudited** (gap **CR-G1**, bead `sq-qhy4`). The v1 ZK
> verifier is published (`SECURITY.md`) as **remediated but NOT externally audited** — the binding
> layer landed and the internal re-audit found it "sound as landed for the assumed threat model,"
> but a relying party must **not** present a "verified" result as a production-grade guarantee
> until the external cryptographer audit (`sq-qhy4`) completes; `sparq-mpc` carries **no security
> guarantee**. Nothing in this analysis upgrades that posture, and a clean timing finding on an
> externally-unaudited research-grade protocol is not a security claim. [OPUS-4.8]

## 0. Scope and method

- **Crates reviewed:** `sparq-zk` (Schnorr signing/verify, Poseidon2, field encoding),
  `sparq-mpc` (secure comparison, Shamir share/reconstruct, MAC/authenticated shares, masking RNG,
  hidden-value join), `sparq-zk-compose` (composition seam — no new primitive arithmetic, it drives
  the above).
- **What was looked for:** secret-dependent branches; secret-dependent array/table indexing;
  non-constant-time equality on secrets (`==` / early-return `memcmp`); variable-time field / bigint
  / scalar operations; timing-dependent error paths; absence of `subtle` (constant-time primitives)
  and `zeroize` (secret-memory hygiene).
- **What was NOT done:** any runtime timing measurement; any analysis of the `arkworks`
  (`ark-bn254` / `ark-ff` / `ark-ec` / `ark-ed-on-bn254`) internals — they are treated as a black box
  whose constant-time properties sparq does **not** assert (see §6).

## 1. Threat-model framing — where a timing channel would matter

A side channel only matters where a **long-term or reusable secret** flows through a timing-variable
operation that an adversary can observe (co-located process, shared cache, or a remote timing oracle).
sparq's design happens to keep most secrets off the timing-sensitive surface:

- **The relying party only ever calls `verify`** (`sparq-zk/src/sig.rs`), which operates over
  **public** commitments and a **public** key — there is no long-term secret in the verify path, so a
  verify-side timing leak reveals only public data.
- **The MPC protocols open only masked / verdict values** — operand shares are never reconstructed on
  the disclosure path (the secret stays secret-shared), so the timing of the *opening* reconstruction
  is over a value the protocol already intends to disclose.
- **The Schnorr secret key is used only at ISSUANCE** (signing), which v1 places in a trusted
  issuance environment, not on an exposed server surface.

This architecture is the reason the residual exposure is rated **LOW today** — not the presence of
constant-time primitives. The risk register below records where that framing would break.

## 2. Per-component assessment

### 2.1 Schnorr signature — VERIFY path (`sparq-zk/src/sig.rs::verify`)

- **Secret on this path?** No. `verify(pk, m, sig)` takes a public key, a public message field
  element, and a signature; it recomputes the Poseidon2 challenge and checks `s·G == R + e·pk`.
- **Equality:** the final accept/reject is `lhs == rhs` on `arkworks` curve-group points (derived
  `PartialEq`). This is **not** asserted constant-time, but it compares **public** values (the
  reconstructed verification equation over public inputs), so a timing difference leaks nothing
  secret. The early `pk.0.is_zero()` / on-curve / subgroup rejections are likewise over public inputs.
- **Verdict:** **No secret-dependent timing risk** (verifier-side, public data). Source-level only.

### 2.2 Schnorr signature — SIGN path (`sign_deterministic`, `derive_nonce`, `challenge`)

- **Secret on this path?** Yes — the issuer secret key `sk.0` (a Baby-JubJub scalar) and the derived
  nonce `k`. `s = k + e·sk.0`; `R = G·k`; `derive_nonce` maps `sk` through Poseidon2.
- **Constant-time?** **Not asserted** — this verdict is unchanged by the remediations recorded below,
  which narrow the exposure without upgrading the claim. *As originally assessed:* the scalar
  multiplication `G·k` and the scalar arithmetic `e·sk.0` use `arkworks` (`ark-ed-on-bn254` /
  `ark-ff`), which makes **no default constant-time guarantee** (§6). A co-located attacker observing
  signing could in principle obtain a timing/cache signal correlated with `sk` or `k`. *As of
  `sq-j3b9`:* the scalar multiplications no longer use arkworks' bit-branching `mul_bigint` (see
  below), so the square-and-multiply structure is gone; the underlying arkworks FIELD operations —
  and `e·sk.0` — are unchanged and are still not asserted constant-time.
- **Nonce derivation is otherwise sound for its purpose:** deterministic RFC6979-style `(sk, m)`
  derivation removes the seed-reuse key-recovery hazard (a *correctness/misuse* property, not a
  timing one) — see the module docstring's audit-#3/codex-#4 note.
- **Mitigating placement:** signing is **issuance-side** (trusted environment), and the relying party
  never signs. **Exposure LOW today**; becomes load-bearing if the deferred in-circuit hidden-key
  upgrade (`sq-1s2`) or any online/server-side signing grows the secret-key timing surface.
- **Verdict:** **Potential secret-dependent timing risk via `arkworks` scalar ops; LOW today by
  placement.** Recorded as bead **`sq-8jv7`** (document non-CT signing + trusted-issuance constraint;
  evaluate a constant-time Schnorr/EdDSA scalar-mul if signing ever moves to an exposed surface).
  **ADDRESSED ([OPUS-4.8], Fable re-review pending):** the one secret-dependent branch in code sparq
  OWNS — `derive_nonce`'s degenerate-`k` guard (`if k.is_zero()`, on the secret nonce) — is now
  **branchless** (always compute the re-fold candidate, then `subtle`-select it via
  `ConditionallySelectable` keyed on a `ConstantTimeEq` zero-test), so our emitted control flow is
  data-independent of the secret nonce. The arkworks scalar-mul `G·k` / `e·sk` residual is
  **documented, NOT claimed constant-time** (a module CONSTANT-TIME POSTURE note in `sig.rs`); closing
  it needs a constant-time scalar-mul (curve/dep swap), deferred. No protocol change — the guard is
  byte-identical to the old select for all reachable inputs (`k == 0` is ~2^-251), proven by test; all
  `sparq-zk` tests pass unchanged.
- **`sq-j3b9` is now PARTIALLY ADDRESSED — the scalar-mul SHAPE, not the field ops ([OPUS-5], crypto
  re-review pending).** The deferred follow-up above was scoped as "a curve/dep swap for a true
  constant-time scalar-mul". sq-j3b9 takes the part of it that is achievable *without* a dependency
  change and is explicit that this is a part, not the whole. What changed: the two SECRET-scalar
  multiplications — `pk = sk·G` (`SecretKey::public_key`) and `R = G·k` (`sign_deterministic`) — no
  longer call arkworks' generic `Group::mul_bigint`, whose textbook double-and-add
  (`for b in BitIteratorBE::without_leading_zeros(k) { res.double_in_place(); if b { res += self } }`)
  leaked the secret scalar *twice*: the `without_leading_zeros` short-circuit made the loop trip count
  a function of the scalar's bit LENGTH, and `if b` made per-iteration work a function of each
  individual scalar BIT. That is the textbook square-and-multiply side channel and it was the
  strongest secret-dependent signal on the signing path. They now call a sparq-owned fixed-width
  double-and-ALWAYS-add ladder (`sparq-zk/src/ct.rs::mul_ct`): a constant `MODULUS_BIT_SIZE` trip
  count regardless of the scalar, and identical per-iteration work regardless of the bit — the addend
  is selected between the base point and the twisted-Edwards affine identity `(0, 1)` by pure field
  arithmetic (`(b·x, 1 + b·(y−1))`), not by a branch and not by a table index. Adding the identity is
  *correct*, not merely harmless, because Baby-JubJub's addition law is complete and `ark-ec`
  implements the unified `E^e` addition — the same property `mul_bigint` already relied on.
  **What is explicitly NOT closed:** every operation still bottoms out in arkworks `ark-ff` field
  arithmetic (Montgomery reduction ending in a conditional subtraction), which sparq still does **not
  assert** is constant-time — that is the curve/dep swap, still open (§6.2). The scalar arithmetic
  `s = k + e·sk` is likewise untouched; unlike a scalar multiplication it is a fixed two-operation
  sequence with no loop and no branch, so it carries no square-and-multiply structure, but it inherits
  the same field-op residual. **The LOW rating and the "NOT asserted constant-time" posture are
  UNCHANGED** — hardening the ladder's shape does not upgrade the claim. The public-data `verify` path
  deliberately KEEPS the faster variable-time arkworks multiplication (no secret on it, so a ladder
  would buy nothing and would cost relying parties an extra group addition per scalar bit).
  **No protocol/value change:** the ladder is
  value-identical to the multiplication it replaces, so every derived key and signature is
  byte-identical and already-issued credentials stay valid — pinned by a randomized differential
  against the arkworks reference (`ct.rs`, including `k = 0`, `k = r−1`, and a non-generator base) plus
  a call-site equivalence test in `sig.rs`. Still **source-level only**: no instrumented `dudect` /
  `ctgrind` measurement (§6.1), and the crate remains research-grade and externally unaudited
  (CR-G1, `sq-qhy4`).

### 2.3 Poseidon2 hash / permutation (`sparq-zk/src/poseidon2.rs`)

- **Structure:** a fixed-round (`R_F = 8`, `R_P = 56`, `t = 4`) sponge. The S-box (`x^5` via
  `square`/`mul`), the MDS / internal-matrix multiplications, and the round-constant additions are
  **straight-line, data-independent control flow** — every input takes the **same number of rounds
  and the same operations**; there are no secret-dependent branches, no table lookups indexed by a
  secret, and no early exit.
- **The only data dependence is inside the field multiply/add** (the `arkworks` `Fr` arithmetic),
  which is the §6 black box.
- **Verdict:** **Appears constant-time at the algorithm level** (fixed rounds, no secret-indexed
  control flow); inherits whatever CT property `arkworks` `Fr` mul/add provides, which sparq does not
  assert. Source-level only.

### 2.4 MPC secure comparison / threshold (`sparq-mpc/src/compare.rs`)

This is the disclosure-minimising `>` / threshold operator (the "four-flatmates £100k" path).

- **Both-secret comparison** (`secure_greater_than` → `greater_than_bits`): iterates a **fixed**
  `COMPARE_BITS = 60` rounds MSB→LSB; each round does the same `secret_and` / `secret_and_not` /
  `secret_bit_eq` multiplications. The loop bound and the operations are **data-independent** — the
  secret operand bits flow only through Shamir multiplications, never through a branch condition or an
  index. **No secret-dependent control flow.**
- **Public-threshold comparison** (`greater_than_public_bits_with`): branches on `(pub_val >> k) & 1`.
  This is a **PUBLIC** value's bits (a public threshold or a public constant), so the branch is
  **data-independent of any secret** — a documented optimisation that skips a known-zero multiply.
  Not a side channel.
- **In-MPC bit-decomposition** (`secure_bit_decompose`): the only opened value is the
  **statistically-masked** `c = sum + r` (`r` is fresh `DECOMP_MASK_BITS = 60`-bit dealer randomness,
  gap `κ = 40`). The borrow-ripple loop runs a **fixed** `DECOMP_MASK_BITS` rounds and branches on the
  **public** bits of `c` (`(c >> k) & 1`) — again public, not the secret. The secret sum is never
  reconstructed.
- **Range proof / zero-test** (`verify_sum_in_range`, `secret_is_zero`): opens only a `v·r` product
  for a fresh **nonzero** mask `r`; the opened value is either exactly `0` (in-range) or a
  **uniform-nonzero** field element, so the `m == Fp::zero()` comparison reveals only the single
  protocol-disclosed bit "was it zero?", not the operand. The `== Fp::zero()` is a derived `Eq` on a
  value that is itself the intended disclosure.
- **Verdict open** (`open_verdict`): reconstructs and matches `0` / `1`; the verdict bit is the
  protocol output.
- **Verdict:** **No secret-dependent branch or index found.** The disclosure-minimisation invariant
  (only masked products / the verdict bit are opened) is structurally enforced and regression-tested
  (`compare.rs` tests `masked_open_is_independent_of_the_sum`,
  `verdict_output_does_not_distinguish_sums_on_the_same_side`). Underlying `Fp` arithmetic is the §3 /
  §6 consideration. Source-level only; honest-majority **semi-honest** model (not malicious) per the
  module docs — a malicious party can still deviate inside a re-sharing round (a *protocol-security*
  gap, separate from timing).

### 2.5 Shamir share / reconstruct (`sparq-mpc/src/shamir.rs`, `robust.rs`)

- **`share` (dealing):** evaluates a degree-`t` polynomial with **uniform CSPRNG** coefficients
  (`next_fp`) at fixed public points `x = 1..=n`. Control flow is data-independent of the secret;
  the secret is only added as the constant term.
- **`reconstruct` / `reconstruct_at_zero` / `reconstruct_robust`:** Lagrange interpolation +
  (where redundancy exists) Berlekamp–Welch error correction. The loops iterate over the **share
  count and degree** (public protocol parameters), not over secret values. The Berlekamp–Welch path
  branches on **consistency of the shares** (tampering detection), which is a malicious-input
  property, not a secret-value branch — and reconstruct is over values the protocol is opening anyway.
- **Field inverse in Lagrange (`Fp::inv` → `Fp::pow(P-2)`):** the exponent `P - 2` is a **PUBLIC
  constant** (the modulus), so the square-and-multiply branch pattern is **data-independent**; and
  `inv` is only ever called on **non-zero differences of distinct PUBLIC evaluation points**
  (`x = 1..=n`), so the value inverted is public too. **Not a secret-value timing risk today** — but
  see §3 for the latent `Fp::pow` hazard if it is ever called with a secret exponent.
- **Verdict:** **No secret-dependent control flow found**; reconstruction is over disclosed values
  and public parameters. Source-level only.

### 2.6 MAC / authenticated shares (`sparq-mpc/src/authenticated.rs`)

- **MAC key `α` is never reconstructed by the public API** — structurally enforced and tested
  (`mac_key_is_never_reconstructed_by_the_api`, `x_equals_one_cannot_extract_alpha_via_public_api`),
  and `Debug` redacts the `α` / MAC shares (`mac_key_debug_redacts_alpha_shares`).
- **MAC verification** is the relation `reconstruct([α·x]) == α·x`, checked over **reconstructed
  (disclosed) values** via the same Lagrange path — the `==` is on opened values, not on a secret
  held constant. There is **no early-return `memcmp` over a secret MAC tag** here; the check operates
  on field elements the protocol has opened.
- **Verdict:** **No secret-dependent equality / early-return found.** The strong control is the
  *structural* "α is never reconstructed" guarantee, not a constant-time tag compare (there is no
  byte-tag compare on this path). Source-level only.

### 2.7 Hidden-value join `secure_equal` (`sparq-mpc/src/join.rs`)

- Draws a fresh **nonzero** mask `r`, opens `m = (a − b)·r` at degree `2t`, tests `m == 0`. Same shape
  as `secret_is_zero` (§2.4): `m` is exactly `0` (equal) or **uniform-nonzero** (unequal), so the
  `== 0` reveals only the protocol-disclosed match bit. Keys `a`, `b` are never reconstructed.
- **Verdict:** **No secret leaked by the equality timing** — the compared value is the intended
  disclosure. Source-level only.

### 2.8 Masking RNG (`sparq-mpc/src/rng.rs`)

- **`SecureRng`** is `ChaCha20Rng` seeded from OS entropy; **not `Clone`** (state cannot be duplicated
  into a reused mask stream); `Debug` redacts state. Insecure deterministic PRNG is `cfg`-gated OFF by
  default (`insecure-test-rng`). These are **correctness/unpredictability** controls (CR-11/CR-12),
  not timing controls.
- **`next_fp` rejection sampling:** loops re-drawing on the single non-canonical value `2^61−1`
  (probability `1/2^61`). The module documents this as **"effectively constant-time in expectation"**
  — i.e. it is **not** strictly constant-time (the loop *can* iterate more than once), but the
  re-draw probability is `2^{-61}`, so the variable-iteration leak is negligible **and** what would
  leak is only "a re-draw happened", which is independent of the masking secret being generated.
- **`next_nonzero_fp`:** re-draws on the `1/p` chance of zero — same negligible-probability
  variable-iteration note; the rejected value (`0`) is not a secret.
- **Verdict:** **Effectively-constant-time-in-expectation** (honestly labelled in-source); the
  variable-iteration paths are `2^{-61}` / `1/p` rare and leak nothing about the masked secret.
  Source-level only.

### 2.9 Field arithmetic (`sparq-mpc/src/field.rs` `Fp` over `p = 2^61−1`)

- **`add` / `sub` / `neg` / `mul`:** branchless modular arithmetic; `reduce_mersenne` does two folds
  plus a **single final conditional subtraction** `if r >= P { r -= P }`. That conditional is a
  data-dependent branch on the reduced value — **standard** for a Mersenne reduction and a very weak
  channel (one branch on the top bit of a reduced product). Worth noting, not a finding on its own.
- **`pow` (square-and-multiply, `field.rs`):** `if exp & 1 == 1 { acc = acc.mul(base) }` — a
  **classic data-dependent multiply branch on the exponent bits**. Today the only caller is `inv`,
  whose exponent is the **public** `P − 2`, so the branch pattern is public and there is **no leak
  today**. The hazard is **latent**: if `Fp::pow` is ever called with a **secret** exponent, it leaks
  the exponent's bit pattern / Hamming weight via timing. Recorded as bead **`sq-7ltf`** (document
  `Fp::pow` as non-constant-time and restrict it to public exponents, or convert `inv` to a
  fixed-window / Montgomery-ladder constant-time exponentiation).
- **Verdict:** **Branchless for the hot ops; one standard conditional subtraction; one latent
  non-CT `pow` guarded today by a public exponent.** Source-level only.

## 3. The latent `Fp::pow` hazard (detail)

`Fp::pow` (`sparq-mpc/src/field.rs`) is the one place a future change could turn a non-issue into a
real leak. It is safe **only because** its sole caller `inv` passes the public constant `P − 2` and
inverts only public point differences. There is no compiler or type-level guard stopping a future
caller from passing a secret exponent. The recommended fix (bead `sq-7ltf`) is the cheap, honest one:
**mark the function non-constant-time and constrain its callers**, and/or give `inv` a constant-time
implementation so the property holds by construction rather than by current-caller convention.

**RESOLVED ([OPUS-4.8], Fable re-review pending).** Both halves of the recommendation were applied:
`Fp::pow` is now `Fp::pow_vartime` (a load-bearing suffix) carrying an explicit public-exponent
contract — its sole caller, the robust Berlekamp–Welch decoder, raises a public evaluation point to
the public error-correction parameter `e` — and a new branchless, fixed-64-iteration `Fp::pow_ct`
(conditional multiply selected with `subtle::ConditionallySelectable`) is constant-time in the base
by construction. `Fp::inv` routes through `pow_ct`, so field inversion holds the constant-time-in-the-
inverted-value property by construction, not by current-caller convention. The latent gap is closed
without any arithmetic change (`pow_ct ≡ pow_vartime` proven by test; all 270 MPC tests pass
unchanged).

## 4. Dependency-level posture — `subtle` / `zeroize` / `arkworks`

- **`subtle` (constant-time equality / selection): ABSENT** from all three crypto crates' manifests.
  No `ConstantTimeEq` / `ConditionalSelect` is used anywhere. Today this is tolerable because the
  architecture **avoids secret-vs-secret equality** (the protocols compare only masked / disclosed
  values), but there is **no defensive guarantee** for any future secret comparison.
- **`zeroize` (secret-memory hygiene): ABSENT.** Secret-bearing types — `SecretKey(JjScalar)` in
  `sparq-zk/src/sig.rs`, the masking-RNG seed/state in `SecureRng`, the MAC-key `α` shares, secret
  `Fp` / `Share` operands — are **dropped without zeroization**, so secret material can linger in
  freed heap / stack memory (a memory-disclosure rather than a timing channel, but in the same
  side-channel-hygiene family). `SecureRng` *does* redact its `Debug` output and forbids `Clone`, but
  does not zeroize on drop. Recorded as bead **`sq-u8a8`**.
  *(Remediation status — see [`gap-register.md`](./gap-register.md) CR-G5: `sq-u8a8` LANDED the
  zeroize-on-drop of `SecretKey` / `α` / share vectors / `Fp` / `Share`; `sq-19ej` then scrubbed the
  one masking-RNG secret buffer `sparq-mpc` controls — `SecureRng::from_os` now seeds via a
  `Zeroizing` buffer and `Drop` best-effort-scrubs the cached keystream block. Irreducible residual,
  documented not claimed: the inner `ChaCha20Rng` key schedule is not scrubbable in place —
  `rand_chacha` has no `zeroize` feature at the pinned `0.9` or the latest `0.10.0`; exposure LOW.)*
- **`arkworks` (`ark-bn254` / `ark-ff` / `ark-ec` / `ark-ed-on-bn254`): constant-time NOT asserted by
  sparq.** sparq does **not** claim, and arkworks does **not** by default guarantee, constant-time
  field arithmetic or constant-time scalar multiplication. Every "appears constant-time at the
  algorithm level" verdict above (Poseidon2, the MPC bit-circuit, Schnorr) ultimately bottoms out in
  arkworks field ops whose microarchitectural behaviour is out of source-level reach. This is stated,
  not papered over — it is exactly why this whole document is labelled source-level-only.

## 5. Summary table

| Component | Secret on path? | CT posture (source-level) | Residual risk | Bead |
|---|---|---|---|---|
| Schnorr **verify** (`sig.rs`) | No (public data) | No secret-dependent timing | None | — |
| Schnorr **sign** (`sig.rs`) | Yes (issuer `sk`) | Nonce-guard branchless; secret-scalar mul now a fixed-width always-add ladder (`ct.rs`); arkworks FIELD-op residual still not asserted CT (documented) | LOW (issuance-side) | `sq-8jv7` **ADDRESSED**; `sq-j3b9` **PARTIAL** (shape, not field ops) |
| **Poseidon2** (`poseidon2.rs`) | Hashes secrets in MPC/sig | Fixed rounds, no secret branch/index | Inherits arkworks `Fr` | — |
| MPC **secure compare** (`compare.rs`) | Operands secret-shared | Fixed loop; branches on PUBLIC bits only | Inherits `Fp` ops | — |
| Shamir **share/reconstruct** (`shamir.rs`/`robust.rs`) | Reconstructs disclosed values | No secret-value control flow | `inv` over public points | — |
| **MAC / authenticated** (`authenticated.rs`) | `α` structurally never opened | No early-return tag compare; checks opened values | None new | — |
| Hidden-join **`secure_equal`** (`join.rs`) | Keys secret-shared | Opens only masked product; `==0` is the disclosure | None new | — |
| Masking **RNG** (`rng.rs`) | Generates masks | Effectively-CT-in-expectation (rare re-draw) | Negligible (`2^{-61}`) | — |
| **`Fp` field** (`field.rs`) | Operands secret-shared | Branchless mul/add; 1 cond-sub; latent non-CT `pow` | Latent if `pow` gets secret exp | `sq-7ltf` |
| **`subtle`/`zeroize`** (deps) | — | ABSENT in all 3 crypto crates | No CT-eq / no secret zeroization | `sq-u8a8` |

## 6. Limitations of this review (do not over-read it)

1. **Source-level only.** No `dudect` / `ctgrind` / `cachegrind` / hardware-counter measurement was
   run. A clean reading does not prove the absence of a timing or cache channel introduced by compiler
   lowering or by the CPU.
2. **`arkworks` is a black box.** Its constant-time properties are neither asserted by sparq nor
   verified here. The algorithm-level CT verdicts assume nothing about the underlying field-op timing.
3. **This does not touch protocol soundness.** A constant-time finding (or its absence) on an
   externally-unaudited, semi-honest, research-grade protocol is not a security claim. The governing
   posture remains `SECURITY.md` + **CR-G1** (external cryptographer audit, `sq-qhy4`): the v1 ZK
   verifier is **remediated but NOT externally audited** — do not present a "verified" result as a
   production-grade guarantee, and treat `sparq-mpc` as providing **no guarantee**, until that
   external sign-off closes. [OPUS-4.8]
4. **The fixes are bead-tracked, not applied.** This pass deliberately changed **no crypto code**;
   `sq-7ltf`, `sq-u8a8`, `sq-8jv7` track the recommended remediations for a separate, test-guarded
   pass.

## 7. Recommendations (prioritised, all bead-tracked)

1. **`sq-u8a8` — adopt `zeroize` for secret-bearing types** (`SecretKey`, masking-RNG seed, MAC-key
   shares) and `subtle` for any future secret-vs-secret equality; document the policy in the crate
   docs. *Highest hygiene value, lowest risk to apply.*
2. **`sq-7ltf` — make `Fp::pow` non-CT-explicit and constrain it / give `inv` a constant-time path.**
   Closes the one latent code-level hazard before a future caller can trip it. **DONE
   ([OPUS-4.8], Fable re-review pending):** `Fp::pow` was split into `pow_vartime` (variable-time,
   explicit public-exponent contract; sole caller is the public-`e` Berlekamp–Welch decoder) and a
   new branchless fixed-iteration `pow_ct` (conditional multiply via
   `subtle::ConditionallySelectable`, constant-time in the base). `Fp::inv` now routes through
   `pow_ct`, so inversion is constant-time in the inverted value by construction. No arithmetic
   changed (`pow_ct ≡ pow_vartime` proven by test; all 270 MPC tests pass unchanged). See
   `crates/sparq-mpc/README.md`.
3. **`sq-8jv7` — document Schnorr signing as non-constant-time + trusted-issuance-only**, and
   re-evaluate a constant-time scalar-mul if/when the in-circuit hidden-key upgrade (`sq-1s2`) or any
   online signing moves the secret key onto an exposed surface. **ADDRESSED ([OPUS-4.8], Fable
   re-review pending):** the signing path's one secret-dependent branch we own (`derive_nonce`'s
   `k == 0` guard) is now branchless (`subtle` select); the arkworks scalar-mul residual is documented
   (module CONSTANT-TIME POSTURE note in `sig.rs`), explicitly NOT claimed constant-time, with a
   curve/dep-swap follow-up beaded for true CT scalar-mul if signing ever moves to an exposed surface.
   No protocol/value change; all `sparq-zk` tests pass unchanged. **`sq-j3b9` PARTIALLY delivers that
   follow-up ([OPUS-5], crypto re-review pending):** the two secret-scalar multiplications now use a
   sparq-owned fixed-width double-and-always-add ladder (`sparq-zk/src/ct.rs`) instead of arkworks'
   bit-branching, leading-zero-short-circuiting `mul_bigint`, removing the square-and-multiply
   control-flow leak *without* a dependency change. The arkworks FIELD-op residual — the actual
   curve/dep swap — remains OPEN, the "not asserted constant-time" posture and the LOW rating are
   unchanged, and the change is value-identical (differential-tested). See §2.2.
4. **(Folded into CR-G1, not a separate bead) — an instrumented `dudect`/`ctgrind` pass** on the sign
   path and the MPC mask/compare paths is the natural deliverable for the external cryptographer audit
   (`sq-qhy4`); this source-level review is the input pack for it, not a substitute.
