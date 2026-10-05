<!-- [OPUS-5] sq-xs0pa (#3140): decision record for the last documented gap in the
externalized sparq_ieee754 face repo. DESIGN-FOR-REVIEW only — no production code, and
none is possible here: the kernels live in sparq-org/noir_IEEE754, not in this repo. -->
# Directed-rounding arithmetic (`rndu`/`rndd`/`rndz`/`rna`) for `sparq_ieee754`

Maintainer-review decision record for bead **`sq-xs0pa`** (issue **#3140**) — the last
unresolved documented gap in [`sparq-org/noir_IEEE754`](https://github.com/sparq-org/noir_IEEE754)
(README "Known gaps"; the deprecated jeswr rounding tests listed unported in its
`TESTING.md`). Deliberately **deferred** from the `sq-dtmg9` v0.11.0 gap-resolution PR
because it is a kernel-level change on the ZK trust boundary needing a design decision,
not an additive API.

Sits on top of [`zk-correctness-and-proof-program.md`](zk-correctness-and-proof-program.md)
(which owns the `sq-3x7dl.*` ieee754 findings, including the `sq-3x7dl.1` canonical-decode
soundness fix this design must not weaken) and
[`zk-test-bench-design.md`](zk-test-bench-design.md) §2.6 (the committed exact-rational
vector oracle). It does not restate or contradict them.

---

## 0. Honesty framing (read first)

- **This record decides; it does not implement.** Per `sq-5reoy` (#1599) the `zk/ieee754`
  tree was externalized and **removed from this repo** — `git ls-files zk/ieee754` is empty.
  `zk/compose/compose_core/Nargo.toml` consumes `sparq_ieee754` as a pinned Nargo git
  dependency at tag `v0.11.0`. The implementation therefore lands in the **face repo**
  (source of truth), behind a tag bump here. Nothing in this record can be, or is, code.
- **Grounding.** The rounding semantics below are from **IEEE 754-2019** (§4.3 rounding
  attributes, §6.3 the sign of an exact-zero result, §7.4 overflow) — normative and
  citable. The face-repo specifics (the exact `round_pack_normalized{,_u64}` signatures,
  the existing `round_to_integral` `MODE` constant encoding, the current instantiation
  sites, the exact count of deprecated tests) are taken from **#3140's own description and
  the in-tree records**; this box has no copy of the face repo and no warm `~/.nargo`
  cache. Every such item is marked **[verify-at-impl]** and must be re-checked against the
  face repo before a line is written. No claim here rests on unread code.
- **No security claim.** `sparq_ieee754` is a value-semantics library inside the ZK
  query-proof trust base; the estate remains **pending external accredited-cryptographer
  sign-off (`sq-qhy4`)**. Nothing here asserts any circuit is sound, proven, or audited.
  §3.3 records the one *soundness-relevant* consequence of the API choice — that a
  comptime mode adds no prover-chosen witness — as an obligation, not an achievement.
- **No performance numbers.** Every cost statement below is a *direction* plus a
  *measurement obligation* discharged by `bb gates` (§6). Per `AGENTS.md`, gate figures are
  never hard-coded into markdown.

---

## 1. What #3140 blocks on, and what this record answers

| # | Blocker as filed | Answered in | Verdict |
|---|---|---|---|
| 1 | Mode must thread through `round_pack_normalized{,_u64}` in **both** kernels; overflow behaviour differs per mode | §4 | Tractable — one predicate + one overflow table; §4.4 is the exhaustive spec |
| 2 | **API shape**: comptime-generic mode methods (⇒ 16 new methods per type) vs a runtime mode param (gate cost on the RNE hot path) | §3 | **Decided: comptime-generic — and it is 4 new methods per type, not 16.** The filed framing over-counts |
| 3 | "No directed-rounding host oracle in stable Rust" — harness would need `rustc_apfloat` | §5 | **Dissolved.** The *committed* oracle is exact-rational Python, which is mode-parametric for free and is its own ground truth. `rustc_apfloat` is an optional cross-check, not a prerequisite |
| 4 | Gate-budget implications must be measured | §6 | Gate-neutrality on the RNE path is an **invariant by construction** under §3's shape, and is falsifiable by an existing measurement |

The only question that is genuinely the **maintainer's** is the greenlight itself (§8).
Blockers 1–4 are engineering questions this record closes.

---

## 2. Consumer reality (the honest case against urgency)

No SPARQL or XPath surface needs non-RNE arithmetic. XSD/XPath `xsd:double` arithmetic is
round-to-nearest-even; SPARQL 1.1 `ROUND()` and `fn:round` are *round-to-integral*
operations, already covered by the round-to-integral family (and by `sq-3x7dl.2`'s
round-half-toward-+∞ mode). `zk/compose`'s only float consumer is `filter_float.nr`
(comparison, not arithmetic).

So the value of this work is **not** unblocking a consumer. It is:

1. closing the last documented gap in a **public, maintainer-authored library** whose
   README currently advertises it as missing;
2. retiring the deprecated rounding tests carried unported in the face repo's `TESTING.md`;
3. spec-completeness for a library that presents itself as *IEEE 754 for Noir* — a
   library missing four of the five rounding attributes is materially incomplete against
   the standard it names.

That is a real but **non-urgent** benefit, and §8's recommendation is sized to it.

---

## 3. Decision D1 — API shape: comptime-generic `MODE`

### 3.1 The decision

Add **one comptime-mode-generic method per operation**, and re-express the existing
fixed-RNE entry points as thin wrappers:

```rust
// Noir, in the generated per-width float type. [verify-at-impl] against the face repo's
// actual codegen.nr shape and its existing round_to_integral MODE constants.
fn add_rm<let MODE: u8>(self, other: Self) -> Self {
    // §3.5: MODE's domain is closed — reject anything outside the five constants at
    // compile time before threading it into the kernel.
    /* mode-threaded kernel */
}
fn add(self, other: Self) -> Self { self.add_rm::<RNE>(other) }   // unchanged behaviour
```

Four new public methods per width — `add_rm`, `sub_rm`, `mul_rm`, `div_rm` — **not** the
16 the issue projects. The 16 comes from enumerating `add_rndu`/`add_rndd`/… as separate
named methods; a single numeric-generic parameter collapses that axis. It also covers
`rna` at zero additional API cost, so the surface is 4 methods for **five** modes rather
than 16 methods for four.

### 3.2 Why not a runtime mode parameter

A runtime `mode: u8` argument would:

- add live constraints to **every existing `add`/`sub`/`mul`/`div` call in every circuit**
  that already uses the library (the mode comparison and the resulting conditional
  selection are witnessed, not folded), directly violating the repo's optimization rule
  that a change is gated on a measured `bb gates` delta;
- have **no consumer** — nothing in the estate needs to select a rounding direction from
  circuit-runtime data; and
- introduce a prover-controlled input into the trust boundary (§3.3).

The comptime shape has all the benefit and none of these costs. Reject the runtime param.

### 3.3 The soundness-relevant consequence (obligation, not a claim)

With a **comptime** `MODE`, the rounding direction is fixed at monomorphisation and is
**not a witness** — there is no new prover-chosen value at the trust boundary, so the
`sq-3x7dl.1` canonical-decode surface is unchanged in shape. A **runtime** mode parameter
would be prover-supplied unless explicitly constrained or promoted to a public input, and
would need its own forge analysis. This is an argument *for* the comptime shape; it is
**not** a statement that the resulting circuits are sound. The obligation stands: the
implementation PR must show the `sq-3x7dl.1` width-bound assertions in `new()` are intact
and that no new `unconstrained` hint is introduced.

### 3.4 Mode encoding

**Reuse and extend the face repo's existing `round_to_integral` `MODE` constants — do not
introduce a second, divergent encoding.** [verify-at-impl] The library already carries a
comptime mode parameter for round-to-integral; the arithmetic modes must share one
namespace and one set of named constants, so that `MODE` means the same thing everywhere.
If the existing encoding cannot express `roundTiesToAway`, extend it additively; never
renumber an existing constant (that would silently change the meaning of any call already
written against a literal).

### 3.5 Invalid `MODE` — the domain is closed and must fail closed

`MODE` is carried as `u8` for want of a narrower Noir generic, but its domain is exactly
the five constants of §3.4. `add_rm::<255>` is nonetheless expressible by any caller, and
nothing in §4 assigns it a meaning — a kernel written as an `if`/`else if`/`else` chain
would hand it whichever arm falls through (truncation, or the RNE overflow path), i.e. a
silently wrong result on a public entry point of a library sitting inside the ZK
query-proof trust base. Reusing named constants is a convention, not an enforcement.

**Requirement.** Every mode-generic entry point — `add_rm`/`sub_rm`/`mul_rm`/`div_rm`, and
the mode-threaded `round_pack_normalized{,_u64}` kernels beneath them — must **reject any
`MODE` outside {`RNE`, `RNA`, `RNDZ`, `RNDU`, `RNDD`} at compile time**, using the face
repo's supported comptime-assertion mechanism (`std::static_assert` over the comptime
generic, or an equivalent `comptime` block). [verify-at-impl] which mechanism that Noir
version actually provides. If none does, the fallback is a plain `assert` on the comptime
constant, which is unsatisfiable for an out-of-domain mode; it should constant-fold away for
a valid one by the same mechanism §6 relies on — and, exactly as §6 insists, that is
measured by the byte-identical RNE gate evidence, not assumed. Fail-closed in either form;
never fall through to a default arm.

A **closed comptime representation** — a mode type whose only inhabitants are the five
constants — is strictly preferable to raw `u8` and should be taken if the face repo's Noir
version admits it as a generic argument. [verify-at-impl] It is not assumed here only
because §3.4's existing `round_to_integral` encoding is `u8`-shaped, and splitting that
namespace would cost more than it buys; if the closed form is adopted, §3.4's "one
namespace" rule means `round_to_integral` moves to it in the same change.

**Both kernels share one validated domain.** The `u64` work kernel and the `u128`/`Field`
wide kernel must use the same constant set and the same rejection, so that no mode can be
accepted by one width and fall through in another.

---

## 4. Decision D2 — kernel threading

The change is confined to the round-and-pack step. Both kernels — the `u64` "work" kernel
(f16/f32/f64) and the `u128`/`Field` "wide" kernel (f128) — take the same treatment; the
predicate is width-independent, only the operand types differ.

### 4.1 The rounding-increment predicate

At the point of packing, the kernel has the truncated significand with least-significant
bit `l`, the round (guard) bit `r`, the sticky bit `s`, and the result sign bit `sign_bit`
(`0` = positive). Let `inexact` = (`r` OR `s`). The significand is incremented iff:

| Mode | IEEE 754-2019 attribute | Increment iff |
|---|---|---|
| `RNE` | `roundTiesToEven` | `r` AND (`s` OR `l`) |
| `RNA` | `roundTiesToAway` | `r` |
| `RNDZ` | `roundTowardZero` | never (truncate) |
| `RNDU` | `roundTowardPositive` | `inexact` AND NOT `sign_bit` |
| `RNDD` | `roundTowardNegative` | `inexact` AND `sign_bit` |

The `RNE` row is exactly the existing behaviour, so the `RNE` instantiation must reduce to
the current constraint set (§6). This table and §4.2's are **exhaustive** over the closed
domain of §3.5: neither may carry a fall-through `else` arm standing in for "some other
mode" — an unlisted `MODE` is rejected at compile time, never defaulted to truncation or to
the RNE path.

### 4.2 Overflow — the per-mode table (IEEE 754-2019 §7.4)

This is the part #3140 flags, and it is the highest-risk line of the change: **directed
modes do not overflow to infinity.** When the rounded magnitude exceeds the format's
largest finite value `maxFinite`:

| Mode | Positive intermediate | Negative intermediate |
|---|---|---|
| `RNE` | `+∞` | `−∞` |
| `RNA` | `+∞` | `−∞` |
| `RNDZ` | `+maxFinite` | `−maxFinite` |
| `RNDU` | `+∞` | `−maxFinite` |
| `RNDD` | `+maxFinite` | `−∞` |

A kernel that keeps the current unconditional overflow-to-infinity path produces a
**silently wrong finite/infinite result** in three of the five modes. This table is the
single most important acceptance target in §7.

### 4.3 Underflow and subnormals

Rounding is applied at the reduced subnormal precision, so the §4.1 predicate governs
underflow too — but the *directed* outcomes are asymmetric and easy to get wrong: a tiny
nonzero positive exact result rounds to the **smallest positive subnormal** under `RNDU`
and to **`+0`** under `RNDD`/`RNDZ` (mirrored for negatives). The existing deep-underflow
flush-to-signed-zero behaviour (fixed for f128 mul/div in `sq-25mgo`, #1558) must be
re-derived per mode rather than reused verbatim. [verify-at-impl] against that fix's shape.

### 4.4 The sign of an exact zero (IEEE 754-2019 §6.3)

The one place the *sign* — not just the magnitude — depends on the mode:

> When the sum of two operands with opposite signs (or the difference of two operands with
> like signs) is exactly zero, that sum/difference is `+0` in all rounding attributes
> **except `roundTowardNegative`, under which it is `−0`**.

So `x - x` is `+0` under `RNE`/`RNA`/`RNDZ`/`RNDU` and `−0` under `RNDD`; likewise
`(+0) + (−0)`. Product and quotient signs are the XOR of the operand signs and are
**mode-independent**. This rule is invisible to a magnitude-only differential corpus and
must be pinned by a targeted test (§7).

---

## 5. Decision D3 — the host oracle

**#3140's premise is too pessimistic.** Stable Rust indeed exposes no way to set the
hardware rounding mode (no stable `fesetround`), so a *hardware* directed-rounding oracle
is genuinely unavailable — but the library's **committed** oracle is not hardware. Per
[`zk-test-bench-design.md`](zk-test-bench-design.md) §2.6, `scripts/generate_float_vectors.py`
is a self-contained **exact-rational** reference (`fractions.Fraction`) with
round-to-nearest-even packing, deliberately chosen because exact-rational is its own
ground truth and needs no external library to be trustworthy.

Extending it to directed modes is a change to the **pack** step only, and #3140 already
concedes it is easy. Concretely: the exact result `q` of `a op b` is a rational; bracket it
between the two adjacent representables `lo ≤ q ≤ hi` in the target format; then select per
mode — `RNDZ` picks the one nearer zero, `RNDU` picks `hi`, `RNDD` picks `lo`, `RNA` picks
the nearer with ties away from zero, `RNE` picks the nearer with ties to even — with §4.2's
overflow table and §4.4's zero-sign rule applied at the boundaries. **Zero new
dependencies**, and the same generator emits all five modes from one exact computation.

**Optional second oracle.** `rustc_apfloat` (the LLVM APFloat port) exposes explicitly
rounded `add_r`/`sub_r`/`mul_r`/`div_r` over `Half`/`Single`/`Double`/`Quad` — a good
*independent* cross-check for a subset of vectors, since agreement between two
independently-derived oracles is stronger evidence than either alone. It is **not a
prerequisite**, it would be a dev-dependency of the face repo's harness only, and its
licence (Apache-2.0 WITH LLVM-exception) must be cleared against that repo's policy before
adoption. [verify-at-impl]

**Recommendation:** exact-rational Python is the primary oracle; treat `rustc_apfloat` as
an optional Phase-C hardening step. Blocker (3) does not gate the greenlight.

---

## 6. Decision D4 — gate budget

Two obligations, both falsifiable with measurements that already exist:

1. **RNE gate-neutrality (hard).** Because `MODE` is a comptime numeric generic, the §4.1
   predicate and §4.2 table constant-fold at monomorphisation, so the `RNE` instantiation
   should emit the *identical* constraint set it does today. The acceptance evidence is
   that `bench/zk-compose/scripts/gate_counts.sh` reproduces **byte-identical** values
   against `bench/zk-compose/gate_counts_latest.json` and
   `crates/sparq-zk-compose/tests/gate_count_snapshot.json` after the face-repo tag bump.
   Any drift on an RNE-only circuit falsifies the shape and must be root-caused before
   merge — do **not** re-baseline the snapshot to absorb it.
   *Caveat:* constant-folding of a comptime generic in an `if` condition is the mechanism
   the library's existing comptime width parameter already relies on, so the precedent is
   in-library — but it is a compiler behaviour, so it must be **measured, not assumed**. If
   folding does not occur, express the predicate as arithmetic over comptime constants and
   re-measure.
2. **Directed-mode cost (informational).** The per-mode instantiation cost is reported as a
   measured `bb gates` delta in the face repo's PR body. It is **not** written into any
   markdown in this repo, and it does not gate: nothing in `zk/compose` instantiates a
   directed mode, so it cannot regress an existing circuit.

---

## 7. Acceptance criteria for the face-repo PR

Differential coverage alone under-samples exactly the cases §4 says are dangerous, so the
suite is corpus **plus** targeted invariants **plus** a self-consistency property:

1. **Generated differential vectors** for every (op × mode × width) from the §5
   exact-rational reference.
2. **Overflow direction**, per §4.2 — 5 modes × 2 signs × 4 widths, at the
   `maxFinite`-to-overflow boundary. Non-negotiable.
3. **Exact-cancellation zero sign**, per §4.4 — `x - x` and `(+0) + (−0)` yield `−0` under
   `RNDD` and `+0` under the other four, per width.
4. **Underflow direction**, per §4.3 — smallest-subnormal vs signed zero under
   `RNDU`/`RNDD`/`RNDZ`.
5. **Tie behaviour** — `RNA` vs `RNE` at an exactly-halfway significand (they differ
   precisely when `l = 0`).
6. **Bracketing self-consistency (oracle-free, highest leverage).** For every op and every
   operand pair with a non-NaN result:
   `rndd(a op b) ≤ rne(a op b) ≤ rndu(a op b)` in the extended-real order, and
   `rndz(a op b) ∈ {rndd(a op b), rndu(a op b)}` selected by the result sign. This needs no
   oracle at all and catches whole classes of threading bugs, including a mis-signed
   predicate in §4.1.
7. **RNE regression** — every pre-existing `#[test]` unchanged and green; no test edited to
   accommodate the change.
8. **Mutation check** — inverting the increment predicate for one mode (e.g. swapping the
   `RNDU`/`RNDD` sign test) must turn at least one test **red**. A suite that stays green
   under that mutation is vacuous and does not discharge these criteria.
9. **Invalid-mode rejection (fail-closed)**, per §3.5 — a compile-failure test per
   mode-generic entry point, in **both** kernels, showing that an out-of-domain
   instantiation such as `add_rm::<255>` does not compile. [verify-at-impl] the face repo's
   harness for asserting a compile failure. *Paired mutation:* with the domain check
   deleted, that test must go red — the invalid instantiation would then compile and pick
   up some arm's behaviour, which is precisely the defect being guarded. A criterion that
   stays green without the check is vacuous.
10. **Soundness surface unchanged** — the `sq-3x7dl.1` width-bound assertions in `new()`
    intact; no new `unconstrained` block; the forge-map's conclusions unaffected.
11. **Docs** — the face repo's README "Known gaps" entry removed (not reworded), and
    `TESTING.md`'s deprecated-rounding-test backlog reconciled against what actually ported.
    [verify-at-impl] the filed count of 37 against the face repo before claiming closure.

---

## 8. Recommendation to the maintainer

**Greenlight the design; sequence the implementation deliberately.** The API-shape question
(§3) has a clear answer that is strictly better than the one the issue sketched — 4 generic
methods rather than 16 named ones, five modes rather than four, gate-neutral on the RNE hot
path by construction, and no new prover-chosen witness. The oracle blocker (§5) dissolves
against the already-committed exact-rational reference. What remains is careful kernel work
against a fully-specified target (§4) with a falsifiable acceptance suite (§7).

Two honest counterweights:

- **No consumer needs it** (§2). This is spec-completeness and public-library hygiene, not
  unblocking work. It should not displace anything with a consumer.
- **It re-opens an audited-surface file.** `round_pack_normalized{,_u64}` sits inside the
  kernel surface covered by the `sq-l9ulg` static forge-map, and the estate is pending
  `sq-qhy4` external sign-off. Landing kernel surgery *during* an external audit window
  invalidates the artefact under review. **Sequence this either before an audit snapshot is
  cut or after it lands — not across it.**

Suggested phasing, each phase independently valuable and independently revertible:

- **Phase A — oracle only (zero risk).** Extend `generate_float_vectors.py` with the five
  modes and emit the vectors. No kernel change, no gate change; the vectors are inert until
  Phase B consumes them, and they are the ground truth Phase B is graded against.
- **Phase B — kernel threading.** §4 in both kernels behind the §3 comptime `MODE` (with
  §3.5's fail-closed domain check), with §6's byte-identical RNE gate evidence and §7's
  criteria 1–10.
- **Phase C — closure.** Port the deprecated rounding tests, clear the README gap entry,
  optionally add the `rustc_apfloat` cross-check, then bump the `sparq_ieee754` tag in
  `zk/compose/compose_core/Nargo.toml` here and re-run the forge suite + gate snapshot.

If the answer is instead **decline**, the honest disposition is to say so in the face repo's
README — "directed rounding is out of scope; this library implements `roundTiesToEven`
arithmetic" — rather than leaving it listed as a known gap indefinitely. An accurate
narrower scope beats a permanently-open TODO.

**Decision required:** greenlight (with phasing) / decline-and-narrow-scope. Everything
else above is settled.
