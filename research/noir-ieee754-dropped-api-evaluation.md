<!-- [OPUS-5] sq-i6f4l (#3155): evaluation record for the three API items dropped from
`sparq_ieee754`'s published surface during the Float-API migration. DESIGN-FOR-REVIEW only
— no production code, and none is possible here: the kernels live in sparq-org/noir_IEEE754,
not in this repo. -->
# Re-adding the dropped `sparq_ieee754` API: `abs`, directed-rounding arithmetic, `Field`↔float

> 🤖 **SPARQ agent** — evaluation record for @jeswr's review. DESIGN-FOR-REVIEW only.

Maintainer-review decision record for bead **`sq-i6f4l`** (issue **#3155**). The migration
from the deprecated free-function float API to the published `Float`/codegen API narrowed
the exported surface; three capabilities did not come across, and the face repo's
`TESTING.md` still carries their deprecated tests as an unported backlog. This record asks,
per item: **re-add, re-add in a different shape, or decline-and-narrow-the-advertised-scope.**

Companion records — this one does not restate or contradict them:

- [`noir-ieee754-directed-rounding-design.md`](noir-ieee754-directed-rounding-design.md) —
  already decides item (2). §4 here **routes to it** rather than re-deciding it.
- [`zk-correctness-and-proof-program.md`](zk-correctness-and-proof-program.md) §1.1 — owns
  the `sq-3x7dl.*` ieee754 findings, including the `sq-3x7dl.1` canonical-decode soundness
  fix that §5 here must not weaken.
- [`zk-test-bench-design.md`](zk-test-bench-design.md) §2.6 — the committed exact-rational
  vector oracle the acceptance criteria in §7 reuse.
- [`zk-audit-readiness-dossier.md`](zk-audit-readiness-dossier.md) §1.3 / §2 — the pinned
  face-repo tag and the `IEEE754-CONSTR` claim row this work sits under.

---

## 0. Honesty framing (read first)

- **This record decides; it does not implement.** Per `sq-5reoy` (#1599) the `zk/ieee754`
  tree was externalized and **removed from this repo** — `git ls-files zk/ieee754` is empty.
  `zk/compose/compose_core/Nargo.toml` consumes `sparq_ieee754` as a pinned Nargo git
  dependency. The implementation lands in the **face repo** (source of truth), behind a tag
  bump here. Nothing in this record can be, or is, code.
- **Grounding, and its limits.** The IEEE semantics below are from **IEEE 754-2019** (§5.5.1
  sign-bit operations, §6.3 the sign of a zero result) and the consumer requirements from
  **SPARQL 1.1 Query §17.4.4.2** (`ABS`) and **XPath 2.0 F&O** (`fn:abs`,
  `op:numeric-unary-minus`) — normative and citable. The *in-repo* facts (what `zk/compose`
  actually calls, the `sq-3x7dl.1` hole and its fix, the pinned tag) are read from this
  checkout and are cited by path. Everything about the **face repo's current source** —
  which symbols the published API actually exports today, whether `bits()` is public per
  width, what `TESTING.md` lists, the exact `from_parts` signature — comes from **#3155's
  own description and the in-tree records**, because this box has no copy of the face repo
  and no warm `~/.nargo` cache. Every such item is marked **[verify-at-impl]** and must be
  re-checked against the face repo before a line is written. **No claim here rests on unread
  code**, and in particular this record does not assert that any of the three items is in
  fact absent from the published surface today — it asks what to do *if* it is, and §1
  makes re-confirming that the first step.
- **No security claim.** `sparq_ieee754` is a value-semantics library inside the ZK
  query-proof trust base; the estate remains **pending external accredited-cryptographer
  sign-off (`sq-qhy4`)**. Nothing here asserts any circuit is sound, proven, or audited.
  §5 records the one *soundness-relevant* consequence of an API choice as an obligation,
  not an achievement.
- **No performance numbers.** Every cost statement is a *direction* plus a *measurement
  obligation* discharged by `bb gates` (§7.6). Per `AGENTS.md`, gate figures are never
  hard-coded into markdown.

---

## 1. The three items, and this record's verdict

| # | Dropped item | Real consumer in the estate? | Touches the trust boundary? | Verdict | § |
|---|---|---|---|---|---|
| 1 | `abs` (and the IEEE 754-2019 §5.5.1 sign-bit family) | **YES** — SPARQL `ABS()` / `fn:abs`, listed "IN (phase 3)" in `zksparql-fragment-extension.md` | No — a sign-lane transform on an already-canonical decode | **RE-ADD**, highest priority of the three | §3 |
| 2 | Directed-rounding arithmetic (`rndu`/`rndd`/`rndz`/`rna`) | No | Yes — `round_pack_normalized{,_u64}` | **Already decided elsewhere** — route to `sq-xs0pa` (#3140), do not re-decide | §4 |
| 3 | `Field`↔float conversions | Prospective only (a numeric value lane), and **direction-dependent** | Yes for `Field`→float; naming-hazard for float→`Field` | **SPLIT** — decline the `From`-shaped forms; re-add only explicitly-named, explicitly-asserted ones **if** a consumer lands | §5 |

**Step 0 for whoever implements any of this:** re-confirm against the face repo that the
item is actually absent from the published surface. #3155's premise is that these were
dropped in the migration; three of the four in-repo records that discuss this library
predate the v0.11.0 gap-resolution work (`sq-dtmg9`), so the premise may be partly stale.
An item that turns out to already exist is closed by a `TESTING.md` reconciliation (§6),
not by new kernel work. [verify-at-impl]

---

## 2. Consumer reality — the honest, per-item case

The sibling directed-rounding record's §2 concludes "no consumer needs it". That conclusion
is **specific to item (2)** and must not be generalised to the other two; the three items
have genuinely different urgency, and that is the main thing this record contributes.

- **`abs` HAS a named consumer.** SPARQL 1.1 exposes `ABS()` as a built-in and XPath 2.0
  F&O exposes `fn:abs`; `zksparql-fragment-extension.md` already lists
  "Numeric fns `abs` `round` `ceil` `floor`" as **IN** the target fragment, with the float
  lanes explicitly delegated to `noir_IEEE754`. So `abs` is not spec-completeness hygiene —
  it is a prerequisite for a fragment this repo has already committed to on paper.
- **Directed rounding has none** — see the sibling record §2; nothing changes here.
- **`Field`↔float has a *prospective* consumer, not a live one.** The live in-tree float
  consumer is `zk/compose/compose_core/src/filter_float.nr`, which uses exactly
  `f64::new(u64)`, `f64::from(u64)`, and the six predicates `lt/le/gt/ge/eq/ne` — no
  `Field` conversion anywhere. The prospective consumer is the **numeric value lane** that
  `filter_float.nr`'s own module doc names as the missing piece ("a numeric value lane in
  the sparq-zk encoding") and that `zk-age-gatecount-reduction.md` sketches as
  `Enc_num = h2(NUM_TYPE, Poseidon2(value_as_field, scale, sign))`. That lane is designed,
  not built. §5.4 argues the conversion should follow it rather than precede it — and §5.3
  flags a semantic decision that lane must make *first*.

---

## 3. Decision E1 — re-add `abs`; it is a sign-lane transform, not arithmetic

### 3.1 The decision

**Re-add `abs`.** Also re-add **`neg`** if it is likewise absent (it has its own consumer:
unary minus in SPARQL/XPath expressions, `op:numeric-unary-minus`). **Do not** speculatively
add `copySign` — it completes the §5.5.1 family but has no consumer, and the scope rule is
add-what-is-needed. [verify-at-impl] whether `neg` is already exported (the
`noir-circuit-patterns` skill lists `std::ops::Neg` as available in this stack, which is
suggestive but not evidence).

### 3.2 Why it is the cheap one

IEEE 754-2019 §5.5.1 classes `abs`/`negate`/`copySign` as **sign-bit operations**: they are
non-arithmetic, exact, never signal, and affect **only the sign bit**. On a value that has
already been through the constrained `new()` decode, the struct carries `sign` as its own
field, so:

```rust
// Noir, in the generated per-width float type. [verify-at-impl] against the face repo's
// actual codegen.nr field names and its private constructor's signature.
fn abs(self) -> Self {
    // sign := 0; exponent and mantissa carried through UNCHANGED from a canonical decode.
    Self::from_parts(0, self.exponent, self.mantissa)
}
```

No rounding, no packing, no `round_pack_normalized`, no new `unconstrained` block, and no
new arithmetic on the exponent or mantissa lanes. That is what makes it separable from §4's
kernel surgery. It does **not** exempt it from §8's audit-window constraint — it ships as a
face-repo tag bump like everything else here, so §8 governs when it lands.

### 3.3 The soundness obligation (not a claim)

`from_parts` is the **private** constructor covered by the `sq-l9ulg` static forge-map — it
does *not* re-run `new()`'s canonicality assertions. An `abs` built on it is canonical **by
carrying**, not by checking: `sign = 0` is trivially in range, and `exponent`/`mantissa`
arrive unmodified from a value that already passed `new()`'s
`assert_max_bit_size::<EXP_SIZE>()` / `::<MANT_SIZE>()` bounds (the `sq-3x7dl.1` fix). That
argument is sound **only if** every path that can produce a `Self` reaching `abs` is
canonical. The implementation PR must show that, not assume it. If any kernel-internal
`from_parts` site can hand `abs` a non-canonical value, `abs` re-exports the `sq-3x7dl.1`
hole through a new public door. This is an obligation on the PR; it is **not** a statement
that the result is sound.

### 3.4 Two traps that make a wrong `abs` look right

Both produce a silently wrong value — the dangerous class in a proof system.

1. **Do not implement `abs` as a comparison-select.** `if self.lt(zero) { self.neg() } else { self }`
   returns **`−0`** for `−0`, because `−0 < 0` is `false` — but `abs(−0)` is `+0`. It also
   returns NaN with its sign bit **intact**, where §5.5.1 requires the sign bit cleared.
   A magnitude-only differential corpus never sees either miss.
2. **Do not branch on the sign at all.** The sign bit is a secret in every circuit that
   matters; Noir compiles an `if` over a witness into a predicated select that evaluates
   both arms, so branching is both *more* expensive and no more constant-shape than the
   unconditional `sign := 0`. This is the same constant-shape rule `filter_float.nr` already
   documents for `f64_verdict`.

### 3.5 The one convention that must be pinned, either way

The library canonicalises NaN somewhere in the kernel path. IEEE 754-2019 §5.5.1 says the
sign-bit operations preserve the payload and are non-arithmetic — so a strict reading has
`abs(qNaN(payload))` keep its payload with the sign cleared. Whichever convention the face
repo already documents, `abs` must **match it and be tested against it**. [verify-at-impl]
what the library's stated NaN convention is. A NaN behaviour that is merely whatever the
implementation happens to do is a latent cross-version incompatibility; §7.2 makes pinning
it an acceptance criterion.

---

## 4. Decision E2 — directed-rounding arithmetic is already decided; route, do not re-decide

Item (2) of #3155 is the same gap as **`sq-xs0pa` / #3140**, whose decision record is
[`noir-ieee754-directed-rounding-design.md`](noir-ieee754-directed-rounding-design.md).
That record settles the API shape (a comptime-generic `MODE`, four generic methods per
width, five modes), the per-mode overflow table, the underflow and exact-zero-sign rules,
the host oracle, the gate-neutrality obligation, and the phasing.

**Nothing in this record supersedes, weakens, or re-opens it.** The only addition here is
scheduling: `abs` (§3) is *not* a prerequisite for it and does not conflict with it — `abs`
never enters `round_pack_normalized` — so the two can proceed independently, subject to §8.

If #3155 is worked as one unit, item (2)'s content is "adopt #3140's record"; if #3155 is
split, item (2) should simply be closed onto #3140 rather than duplicated.

---

## 5. Decision E3 — `Field`↔float, split by direction

"Field↔float" is two conversions with completely different risk profiles, and collapsing
them into one API decision is how the dangerous one gets waved through with the safe one.

### 5.1 float → `Field`: it is a **bit-pattern** accessor, and the name is the hazard

The deprecated form recorded in the `noir-circuit-patterns` skill (from
`ieee754/src/float.nr:141-145`) is:

```rust
fn to_field(self) -> Field {
    self.sign() * Self::sign_field()
        + self.exponent() * Self::implicit_bit_field()
        + self.mantissa()
}
```

That is the **`bits()` packing expression** — the same expression whose non-injectivity was
the `sq-3x7dl.1` soundness hole. Post-fix it *is* injective for values that came through the
constrained `new()`, so the mechanics are fine. The problem is the **name**: `to_field` reads
like a value conversion and is not one. It yields the IEEE bit pattern reinterpreted as a
field element, under which:

- `+0` and `−0` map to **different** `Field`s though they are IEEE-equal;
- every NaN payload maps to a **different** `Field` though all are IEEE-unequal to everything
  including each other;
- ordering the resulting `Field`s does **not** order the floats (the sign-magnitude encoding
  reverses on negatives).

A consumer that reaches for the "obvious" `to_field` to compare or key values gets a result
that is wrong in exactly the places IEEE semantics are subtle — and it is wrong *silently*,
inside a proof.

**Decision:** if this direction is re-added at all, it is re-added as an explicitly-named
bit-pattern accessor — `bits_as_field()` or equivalent — with a doc-comment stating in one
line that it is the bit pattern, not the value, and that it is not order- or
equality-compatible with the IEEE predicates. **Do not restore the name `to_field`**, and do
not restore it as a `From`/`Into` impl (the orphan rule already blocks
`impl From<FloatN> for Field`, which is why the inherent method existed; that accident is
now a feature — keep the conversion explicit at every call site).

### 5.2 …and it may be redundant

For f16/f32/f64 the packing already fits a `u64`, so a caller outside the crate can write
`Field::from(x.bits())` with no new API at all. The only width where a `Field`-typed
accessor is genuinely load-bearing is **f128**, whose bit pattern exceeds `u64` and whose
kernel is already the `u128`/`Field` wide kernel. [verify-at-impl] whether `bits()` is public
for every width and what it returns for f128. **If it is, the honest disposition for this
direction is decline-as-redundant** — recorded as such in the README rather than left listed
as a gap.

### 5.3 The decision the value lane must make *before* consuming either form

The prospective consumer (§2) is a numeric value lane keyed on a Poseidon2 commitment over
`value_as_field`. A bit-pattern-keyed commitment **distinguishes `+0` from `−0` and
distinguishes NaN payloads**; the FILTER lane's `eq` predicate does not. So a value lane
keyed on the raw pattern and a FILTER lane keyed on `eq` **disagree on exactly those
inputs** — which is a value/lexical-agreement question of the same family as the
`INV-VL` row in `zk-audit-readiness-dossier.md` §2, and it is a *soundness-relevant*
disagreement, not a cosmetic one.

That decision (canonicalise before hashing, or document the lane as explicitly bit-keyed and
exclude the divergent inputs from the fragment) belongs to the value-lane design, and it must
be made **before** a float→`Field` accessor is offered as that lane's primitive. Shipping the
accessor first invites the lane to be built on whichever behaviour the accessor happens to
have. This is the substantive reason §1 sequences item (3) behind a consumer rather than
ahead of one.

### 5.4 `Field` → float: decline the `From` shape outright

This is the direction that reopens the trust boundary. It has two possible meanings, and
neither should ship as `From<Field>`:

1. **Reinterpret a `Field` as a bit pattern.** A `Field` is a BN254 element with far more
   inhabitants than any float width, so this is `new(bits)` *plus* a range assertion. Without
   an explicit `f.assert_max_bit_size::<W>()` a prover supplies an out-of-range `Field` and
   the decode's canonicality argument — the entire content of the `sq-3x7dl.1` fix — no
   longer covers the entry point. Re-adding this as an implicit `From` puts a **new public
   door onto the ZK trust boundary** whose safety depends on a range check a reader of the
   call site cannot see.
   **Decision:** never as `From`. If a `Field`-shaped constructor is genuinely needed, it is
   an explicit `from_field_bits(f: Field) -> Self` that asserts the width bound and then goes
   through the **constrained** `new()` path — never through the private `from_parts` — and it
   ships with its own forge test (§7.5).
2. **Convert a `Field`'s numeric value to the nearest float.** **Decline outright.** It is a
   254-bit integer→float conversion needing RNE rounding for values at or above 2^53 (far
   larger than the existing `From<u64>` kernel); it has no consumer; and it is semantically
   ill-defined — BN254 elements are unsigned residues with no canonical signed reading, so
   any caller who means "the signed value" gets a wrong answer for half the domain with no
   diagnostic. An API that cannot be given an unambiguous meaning should not be given an
   implementation.

---

## 6. The `TESTING.md` migration backlog — disposition follows the API decision

The face repo's `TESTING.md` carries the deprecated tests for all three items as an unported
backlog. The rule, mirroring the sibling record's closing principle: **every listed test
gets a disposition; none stays listed indefinitely.**

| Bucket | Disposition |
|---|---|
| `abs` / sign-bit tests | **Port**, extended per §7.2 (the `−0` and NaN cases the deprecated suite may not have covered). |
| Directed-rounding tests | **Owned by #3140 Phase C** — reference it from `TESTING.md`; do not re-list them here as an independent backlog. |
| `Field`↔float tests | **Retire with a stated reason** if §5's decline holds — a one-line "declined; see this record" beats a permanently-open TODO. Port only the subset that survives as a test of the renamed accessor / explicit constructor. |

[verify-at-impl] the actual contents and count of that backlog before claiming any of it
closed; #3155's own count is a starting point, not evidence.

---

## 7. Acceptance criteria for the face-repo PR(s)

Scoped to items (1) and (3); item (2)'s criteria are #3140 §7 and are not restated.

1. **`abs` differential vectors** — every width, from the §2.6 exact-rational reference
   generator (`zk-test-bench-design.md`), covering normals, subnormals, both zeros, both
   infinities, and NaN.
2. **`abs` special cases, pinned individually** — `abs(−0) == +0` **with the sign bit
   asserted** (not merely `abs(−0).eq(zero)`, which is true for `−0` too and is exactly the
   vacuous assertion the §3.4 bug survives); `abs(−∞) == +∞`; `abs(NaN)` matches the
   library's documented NaN convention (§3.5), with that convention stated in the doc-comment
   the test cites. Per width.
3. **`abs` mutation check** — replacing the body with the §3.4 comparison-select form must
   turn at least one test **red**. A suite that stays green under that mutation does not
   discharge these criteria and is not evidence.
4. **Idempotence + involution** — `abs(abs(x)) == abs(x)` and, if `neg` lands,
   `abs(neg(x)) == abs(x)`, over the full corpus. Oracle-free, and catches sign-lane
   threading bugs directly.
5. **`from_field_bits` forge test (only if §5.4(1) is taken)** — a `should_fail` negative
   test per width feeding a `Field` at or above `2^W`, plus the `sq-3x7dl.1`
   non-canonical-witness forge (`{exp−1, mantissa+2^mant_size}`) routed through the new
   entry point. Both must reject. Without these the constructor does not ship.
6. **Gate evidence** — the PR body reports the measured `bb gates` delta; and, after the tag
   bump lands here, `bench/zk-compose/scripts/gate_counts.sh` reproduces values against
   `bench/zk-compose/gate_counts_latest.json` and
   `crates/sparq-zk-compose/tests/gate_count_snapshot.json`. Because none of §3/§5 changes a
   path `zk/compose` instantiates, the expectation is **no change on the existing circuits**;
   any drift falsifies that and must be root-caused, **not** absorbed by re-baselining the
   snapshot.
7. **Soundness surface unchanged** — the `sq-3x7dl.1` width bounds in `new()` intact; **no
   new `unconstrained` block** (§3.2 needs none, and §5.4's constructor must not introduce
   one); the `sq-l9ulg` forge-map's conclusions unaffected, or its delta stated explicitly.
8. **Regression** — every pre-existing `#[test]` unchanged and green; no test edited to
   accommodate the change.
9. **Docs** — the face repo's README gap entries for the items actually closed are **removed,
   not reworded**; the items declined per §5 are recorded as **declined with their reason**
   rather than left listed as gaps; `TESTING.md` reconciled per §6.

---

## 8. Sequencing against the `sq-qhy4` audit window

The sibling record's §8 warns that kernel surgery during an external-audit window
invalidates the artefact under review. That constraint applies **unevenly** across these
three items, and the difference is worth acting on:

- **`abs`/`neg` (§3) are additive and outside the audited kernel arithmetic** — a sign-lane
  transform on an already-canonical decode, no `round_pack_normalized` involvement, no new
  `unconstrained` block. Subject to §7.7 holding, that makes it the **lowest review-delta**
  of the three, and it is also the one with a real consumer.
  **It does not follow that it is snapshot-neutral.** The implementation lands in the face
  repo and reaches this estate as a **tag bump** (§0, §7.6), and the `IEEE754-CONSTR` row in
  `zk-audit-readiness-dossier.md` §2 pins its evidence to a *specific* face-repo tag
  (`sparq_ieee754 @ v0.11.0`, static-analysed at a pinned commit under `sq-l9ulg`). Bumping
  that tag changes the dependency source and the published trust-base surface under review,
  whether or not any instantiated `zk/compose` circuit changes. **No audit-scope evidence in
  this repo establishes that an additive sign-lane API is inside the accepted audit delta.**
  So the same "before or after a snapshot, never across it" rule applies by default; the
  only thing that lifts it is the **external auditor explicitly confirming** that additive,
  non-kernel API additions are within the accepted delta for the pinned artefact. Until that
  confirmation exists, treat the low review-delta as an argument about *cost of re-review*,
  not as permission to land mid-window.
- **Directed rounding (§4) reopens `round_pack_normalized`** — #3140 §8's
  "before or after a snapshot, never across it" rule stands unchanged.
- **`from_field_bits` (§5.4) touches the trust-boundary entry point** — treat it exactly like
  kernel surgery for scheduling purposes even though it is small, because what it changes is
  the surface the forge-map is *about*.

---

## 9. Recommendation to the maintainer

1. **Greenlight `abs` (+ `neg` if absent), scheduled around the audit window.** It is the only
   one of the three with a named consumer this repo has already committed to on paper, it is
   the cheapest to implement, it has the clearest canonicality argument, and it carries the
   smallest re-review delta. Conditions: §3.4's two traps avoided, §3.5's NaN convention
   pinned, §7.1–.4 and §7.6–.9 discharged — **and §8's audit coordination**: because it ships
   as a face-repo tag bump, it lands **before or after** the pinned audit snapshot unless the
   external auditor confirms additive sign-lane APIs are within the accepted delta. Low
   review-risk is not the same as snapshot-safe, and this record does not claim the latter.
2. **Close item (2) onto #3140.** No second decision, no duplicated record.
3. **Decline `Field`↔float in the shapes it was dropped in.** `From<Field>` and the numeric
   `Field`-value conversion should be declined outright (§5.4); `to_field` should be declined
   under that name (§5.1) and, if `bits()` already covers every width, declined as redundant
   (§5.2). Revisit only when the numeric value lane is actually being built — and only after
   that lane has made the `+0`/`−0`/NaN keying decision in §5.3, not before.
4. **Whatever is declined, narrow the advertised scope to match** (§6, §7.9). An accurate
   smaller published API beats three permanently-open TODOs — which is the same disposition
   #3140's record reaches for its own decline branch.

Two honest counterweights, stated plainly:

- **The premise needs re-confirming.** This record cannot see the face repo. If any of the
  three is already exported at the pinned tag, its section collapses to a `TESTING.md`
  reconciliation. §1's Step 0 exists for that reason and should be done first.
- **`abs` is a prerequisite, not a deliverable.** It unblocks a *phase-3* fragment item that
  nothing schedules today. It should be sized as the small additive change it is and should
  not pull the rest of the numeric-function fragment forward with it.

**Decision required:** greenlight `abs`(+`neg`) / route item (2) to #3140 / decline-and-narrow
item (3). Everything else above is settled.
