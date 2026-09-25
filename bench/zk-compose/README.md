# bench/zk-compose

Benchmarks for the ZK proof-composition layer (`crates/sparq-zk-compose` +
the `zk/compose/` Noir circuit family).

Historical measurements record Opus 4.8 (Fable 5 unavailable). Newer result
records identify GPT-6 and their exact measurement provenance separately.

## What is measured

1. **Gate counts** (`bb gates -s ultra_honk`, ground truth per the
   noir-optimisation skill) for every compiled circuit-family member, in the
   `sparq_ieee754` JSON convention (`circuit_size`) — the shape that originated
   in the ieee754 lineage, now the `sparq-org/noir_IEEE754` face repo.
2. **Wall-clock prove/verify** for the small credential-scale e2e cases.
3. **SPARQL feature → ZK gate-cost catalog** (`sparql_feature_catalog.json`) — a
   coverage map AND an optimisation-target list spanning SPARQL 1.1, joining each
   query's `circuit_size` from the gate-count snapshot. See "SPARQL feature
   catalog" below.
4. **Per-config bb-gates matrix** (`bb_gates_matrix.json`) — for every
   `(commitment-method × circuit)` configuration, whether the pair is LEGAL and
   the circuit's `circuit_size`, so the maintainer can compare the methods (#769).
   See "per-config bb-gates matrix" below.

## Files

| file                              | contents |
|-----------------------------------|----------|
| `gate_counts_latest.json`         | per-member ultra_honk gate counts |
| [result_capacity_gates.json](result_capacity_gates.json) | [GPT-6] exact measured result capacities, toolchain output and source hashes |
| [result_v1_compatibility.json](result_v1_compatibility.json) | [GPT-6] baseline/current v1 ACIR, ABI and key comparisons, with baseline key bytes |
| `prove_verify_timing.json`        | bb prove/verify wall-clock + proof sizes (early, 2-member, darwin) |
| `family_cost_curve.json`          | sq-pn2 full-family (k,n,r,d) prove/verify/size curve |
| `family_curve/`                   | sq-pn2 standalone timing harness (own cargo project) |
| `sparql_feature_catalog.json`     | sq-1s2.1.2 SPARQL 1.1 feature → ZK coverage + gate-cost catalog |
| `bb_gates_matrix.json`            | sq-ot3x per-config (commitment-method × circuit) legality + gate-cost matrix |
| `constraint_counts.json`          | sq-gum8.5 constraint-count evaluation pack (machine-readable) |
| `CONSTRAINT_COUNTS.md`            | sq-gum8.5 constraint-count evaluation pack (reviewer-facing tables) |
| `scripts/gate_counts.sh`          | regenerate the gate-count JSON |
| `scripts/prove_verify.sh`         | time prove+verify for one member |
| `scripts/sparql_catalog.py`       | regenerate the SPARQL feature catalog (joins the snapshot) |
| `scripts/bb_gates_matrix.py`      | regenerate the per-config bb-gates matrix (joins the snapshot) |
| `scripts/constraint_pack.py`      | regenerate + verify the constraint-count evaluation pack |
| [verify_result_evidence.py](scripts/verify_result_evidence.py) | [GPT-6] default evidence consistency checks and independent fixed-foundation rebuild |

> The gate-count JSON is also the source the in-crate **regression gate**
> (`crates/sparq-zk-compose/tests/gate_count.rs`, sq-c5f) baselines against —
> re-run `scripts/gate_counts.sh` after an intentional circuit change and update
> both that JSON and `crates/sparq-zk-compose/tests/gate_count_snapshot.json`.

## sq-1s2.1.2: SPARQL feature → ZK gate-cost catalog

`sparql_feature_catalog.json` is a **comprehensive SPARQL 1.1 benchmark catalog**
that doubles as (i) a **ZK-coverage map** — for each SPARQL feature, which circuit
member(s) (if any) it compiles to today — and (ii) an **optimisation-target list**
— per-member `circuit_size` with the high-gate members flagged as reduction
targets. It implements the §6 catalog design of
`research/zk-field-native-encoding.md`.

The catalog spans the full feature surface: BGP (single / multi-pattern), every
numeric-FILTER datatype + operator (the complex filters: integer, signed integer,
decimal, double), boolean / string / dateTime FILTER, OPTIONAL, UNION, **property
paths including `+` / `*` / `?` / `/` / `|` / `^`**, aggregates / GROUP BY,
subqueries, BIND, VALUES, MINUS / negation, and the hidden-credential primitives
(revocation, issuer attestation, holder possession). The point of the map is the
**gaps** — it is a coverage map and an optimisation-target list, not a win list.

**Honesty is load-bearing.** Each query records its coverage status:

- `covered` — compiles to a real circuit member that proves the feature **as
  stated**; its `circuit_size` is **joined from**
  `crates/sparq-zk-compose/tests/gate_count_snapshot.json` (the regression-gated
  source of truth), never hand-typed, so it can never drift.
- `partial (verifier-side / desugars …)` — no dedicated member: composed
  verifier-side or desugared into a covered primitive (UNION, alternative paths,
  subquery, VALUES); `circuit_size: null`, because the cost belongs to the
  underlying scan/join member.
- `partial (BOUNDED in-circuit statement …)` — a **real** member proving a
  **strictly weaker** statement than the SPARQL feature: the `path_reach` family
  for `p+` / `p*` proves *there exists a chain of ≤ D committed triples*, with `D`
  a **public** input. Existence only — never "no longer path exists", never
  completeness. These carry a real joined `circuit_size` but are **not**
  `covered`.
- `NO ZK CIRCUIT YET (gap)` — the feature has **no circuit today** (zero-or-one
  paths `?`, BIND expression eval, string predicates, dateTime compare). Carries
  `circuit_size: null`. **A gate number is NEVER fabricated for a gap.**
- `NO ZK CIRCUIT YET (gap — EXCLUDED BY DESIGN …)` — not "unbuilt" but "not
  admissible": OPTIONAL, aggregation and negation are non-monotone / closed-world
  (`research/zksparql-fragment-extension.md` §3.2), so no circuit size would make
  them sound. Also null.

Flags: `HIGH_GATE_blake3_binding` marks the numeric-FILTER family (the value-hook
reduction target of §2.6 — the blake3 token-binding the encoding overhaul
removes), `HIGH_GATE_lattice` marks a scan/join member that is large because of
the (k,n,r)/(na,nb) lattice corner, and `BOUNDED_DEPTH_EXISTENCE_ONLY` marks the
bounded rows above. Where the value hook has **landed** (integer / decimal /
double), the row carries `value_lane_member` + a joined
`value_lane_circuit_size` — a **measurement**; only a lane with no compiled
value-lane member (signed integer) still carries the self-labelled
`projected_after` ESTIMATE, and the guard test fails if a projection is ever left
sitting next to a measurement.

Nothing in the catalog is a soundness or privacy claim: the ZK estate is
internally re-audited but **NOT externally audited** (`sq-qhy4`) and the
composition verifier is NOT-yet-sound. "Covered" means *a member exists and
dispatch binds it*, never *proved secure*.

### Regenerate

```sh
bench/zk-compose/scripts/sparql_catalog.py > bench/zk-compose/sparql_feature_catalog.json
bench/zk-compose/scripts/sparql_catalog.py --check   # committed copy is up to date?
```

The generator needs **no** `nargo`/`bb` (it reads the committed snapshot), so it
is deterministic. `--check` fails unless the committed JSON is **byte-identical**
to a fresh generation *and* self-consistent with the snapshot, so regeneration is
provably idempotent. The committed JSON is gated by
`crates/sparq-zk-compose/tests/sparql_catalog.rs`, which fails if a covered query
names a member absent from the snapshot, if any joined number (headline, per
member, floor, or value lane) disagrees with the snapshot, or if a gap carries a
non-null gate number anywhere on the row. After an
**intentional** circuit change, re-run `scripts/gate_counts.sh` (re-baseline the
snapshot) and then re-run the catalog generator to re-join the new numbers.

> This file is the **DATA layer**. The website surface that renders it (the
> SPARQL → ZK coverage + gate-cost table on `/benchmarks/zk`) landed in sq-1s2.1.3
> (#777): `site/scripts/sync-zk-catalog.mjs` copies this JSON into
> `site/src/data/zk-sparql-catalog.generated.json`, which the
> `ZkSparqlCatalog` component (`site/src/components/benchmarks/zk-sparql-catalog.tsx`)
> renders — so the rendered numbers always trace back to this snapshot-joined data.

## sq-ot3x: per-config bb-gates matrix (commitment-method × circuit)

`bb_gates_matrix.json` is the maintainer-requested (#769) **comparison surface**:
for every `(commitment-method, circuit)` **configuration** it records whether the
pair is **legal** (dispatch-compatible — see the load-bearing caveat below) and —
per circuit — its ultra_honk `bb-gates`
`circuit_size`. So the maintainer can compare the three commitment methods head to
head: e.g. the `string-canonical` blake3-token integer FILTER lane
(`filter_int_d2`) against the `dual-leaf` **value lane** (`filter_value_dl_int`) it
unlocks — a ~5.7× gate reduction — and see exactly which configs each method
admits.

Two axes, both **joined from a regression-gated source of truth** — nothing is
hand-typed that could drift:

- The **gate-count** axis (`circuit_size`) is joined from the regression-gated
  snapshot (`crates/sparq-zk-compose/tests/gate_count_snapshot.json`), the same
  source `gate_counts.sh` and `sparql_catalog.py` use.
- The **legality** axis mirrors the fail-closed dispatch rule
  (`crates/sparq-zk-compose/src/dispatch.rs`, `resolve_circuit`, sq-cfmv): value-lane
  members (`filter_value_dl_*`) are legal only against a method that committed a
  value handle (`dual-leaf` / the `value-only` research dial); string-lane members
  (scan, join, path, revoke, issuer, holder, the blake3-token `filter_*` lanes) are
  legal against `string-canonical` / `dual-leaf` but not `value-only`.

**`legal: true` means DISPATCH-COMPATIBLE, not end-to-end provable today.** It says
the `(method, circuit)` pair is *admitted* by the resolver rule — nothing more. It
does **not** assert the configuration can currently be committed, proved, and
verified end to end: the `dual-leaf` host-leaf encoding (sq-j506) is **not yet
implemented**, and `resolve_circuit` is **not yet wired into `verify_manifest`**. So
a `legal: true` cell records design-level (structural) legality under the still-
unwired resolver, not an operational provable path — a distinction that is
load-bearing on a ZK surface. Only `string-canonical` is implemented end-to-end
today.

An **illegal** cell carries `legal: false` + the fail-closed `reason` and NO
per-cell gate number (the config does not exist, so it has no cost); the circuit's
intrinsic cost lives once on the row's `circuit_size`.

**Honesty is load-bearing.** The bb-gates are **NON-CANONICAL** work-box numbers —
a comparison tool, not a canonical performance figure. The whole ZK estate is
internally re-audited but **NOT externally audited** (sq-qhy4). `dual-leaf` and
`value-only` carry the documented **INV-VL downgrade** (value↔lexical agreement on
the value-FILTER lane is trusted-issuer-honesty, not machine-enforced; CR-G8, #769
accepted at research grade); `value-only` is a research/benchmark dial, never
production. The matrix asserts **no** soundness or privacy property — only which
configurations are structurally legal and how many gates each circuit costs.

### Regenerate

```sh
bench/zk-compose/scripts/bb_gates_matrix.py > bench/zk-compose/bb_gates_matrix.json
```

The generator needs **no** `nargo`/`bb` (it reads the committed snapshot), so it is
deterministic. The committed JSON is gated by
`crates/sparq-zk-compose/tests/bb_gates_matrix.rs`, which fails if a circuit is
absent from the snapshot, if a `circuit_size` disagrees with it, if a snapshot
member is missing a matrix row, if a legality cell violates the dispatch rule
(cross-checked against the real `resolve_circuit` under the `dual-leaf` feature),
or if an illegal cell carries a fabricated cost. After an **intentional** circuit
change, re-run `scripts/gate_counts.sh` (re-baseline the snapshot) and then this
generator to re-join the new numbers.

## sq-gum8.5: the constraint-count evaluation pack

`constraint_counts.json` + `CONSTRAINT_COUNTS.md` are the **submission-support** evaluation
artefact for the live zkSPARQL ISWC 2026 submission (`zksparql.org`; design record
`research/paper-selection.md` §3.1 and §5-P1). They exist because the one thing this estate
can report **canonically** is a deterministic integer — a compiled circuit size — and a
reviewer asks about it in a shape the raw snapshot does not have: per family, per family
parameter, with the scaling and the invariances made explicit.

The pack reorganises the regression-gated gate counts into:

1. **Per-family member tables**, with each member's family parameters parsed out of its name
   (`scan(k, n, r)`, `join_eq(na, nb)`, `path_reach(d, k, n)`, the four FILTER lanes, and the
   credential-layer members), split into a **query layer** and a **credential layer** — the
   two things the manifest unifies.
2. **Single-parameter scaling pairs** — every pair inside a family differing in exactly one
   *numeric* parameter, with the gate delta and ratio. No curve is fitted and nothing is
   extrapolated: only the pairs the compiled family actually contains are reported. A
   categorical parameter (the value lane's datatype) is deliberately **not** swept, because a
   "delta" along it would describe two unrelated circuits rather than a trend.
3. **Invariances** — axes along which the count provably does not move in this snapshot. The
   load-bearing case is the blake3-token FILTER lanes: the canonical token fits one 64-byte
   blake3 block for every supported digit count, so `d` does not move the circuit at all.
4. The **string-lane vs value-lane** comparison, both sides joined from the snapshot.
5. A **related-work block** recording, per cited system, that **no** figure of it is
   transcribed — and why a cross-system constraint ratio would not be a measurement.

**Honesty is load-bearing** (the same rules the spec draft's §16.2 states):

- A `circuit_size` is a **size**, not a **time**. The pack carries **no** wall-clock figure by
  design; work-box timings are non-canonical and live separately in `family_cost_curve.json`.
- A gate count says nothing about **what** a circuit proves. Coverage is
  `site/specs/zksparql.typ` §7.1 + `sparql_feature_catalog.json`; the `path_reach` family
  proves a strictly weaker **bounded-existence** statement whatever its size.
- The estate is internally re-audited but **NOT externally audited** (`sq-qhy4`); the forge
  suite is toolchain-gated (`sq-1gir`); the value lane carries the INV-VL / CR-G8 downgrade.
  The pack asserts **no** soundness, privacy, or zero-knowledge property.
- **No third-party figure is reproduced.** Constraint counts are incomparable across proof
  systems and arithmetizations, so no cross-system ratio is computed.

### Regenerate and verify

```sh
bench/zk-compose/scripts/constraint_pack.py --write   # rewrite both artefacts
bench/zk-compose/scripts/constraint_pack.py --check   # committed copies still current?
bench/zk-compose/scripts/constraint_pack.py --format markdown   # to stdout
```

Like the other two generators it reads the regression-gated snapshot
(`crates/sparq-zk-compose/tests/gate_count_snapshot.json`) and needs **no** `nargo`/`bb`, so
it is deterministic and a re-run is byte-identical — that byte-identity is the acceptance
criterion of the bead, and `--check` is what enforces it. `--check` additionally re-verifies
the *committed* JSON against the snapshot (every member size, every scaling delta, every
invariance, both lanes of every comparison), so a hand edit cannot introduce a number the
generator would never emit.

Two guards make the pack non-vacuous:

- `classify()` **fails** on a snapshot member matching no described family — a new circuit
  family cannot silently drop out of the evaluation; describe it in `FAMILIES` first.
- `--check` fails if any related-work entry ever claims transcribed figures, or if the metric
  block is ever flipped to wall-clock / canonical.

`--check` is **not** a manual courtesy: it runs as a **GATING** step of the `docs-quality
quick-gates` job (`.github/workflows/docs-quality.yml`), which carries no paths filter and so
runs on every PR — a re-baselined snapshot, a hand-edited artefact, or an unclassified new
snapshot member reds the required `ci-summary / gate` instead of merging unnoticed.

After an **intentional** circuit change: re-run `scripts/gate_counts.sh`, re-baseline
`crates/sparq-zk-compose/tests/gate_count_snapshot.json`, then re-run this generator so the
pack re-joins the new numbers.

## sq-pn2: full-family prove/verify cost curve

`family_curve/` is a STANDALONE cargo project (own `[workspace]`, same isolation
pattern as `bench/zk`) that drives the `sparq-zk-compose` prover (nargo + bb
subprocesses) once per circuit-family member and emits the full **(k, n, r, d)
cost curve** — prove time, verify time, proof size, vk size. CI **builds** the
harness (a compile-only schema-drift guard, see the blockquote below)
but never **runs** it: each row is a real `bb prove` (~1-2 s) so the curve is
generated manually, not in CI. It is a plain timing harness, not criterion
(criterion's repeated sampling over ~1-2 s proofs would take hours).

`k` = committed graphs (disclosed-attribution width), `n` = slot bucket, `r` =
disclosed-row bucket (the scan family); `d` = digit count (the filter families).
The scan sizes are chosen to land on each compiled member; both filter families
sweep `d ∈ {1,2,4}`. As of sq-q7e/sq-tat the `filter_f64` member is **manifest-
composable** (it carries `d` in its `CircuitId::FilterF64 { d }`, has a real
`ProofInputs::FilterF64`, and renders its `Prover.toml` via `prover_toml_for`
from a canonical decimal-digit witness, exactly like `filter_int`), so the
harness drives it through that composable path rather than the old hand-written
raw `Prover.toml`. ([OPUS-4.8] sq-kep2 ported the harness to that schema.)

> **Schema-drift guard (sq-kep2).** Because `family_curve` is a STANDALONE cargo
> project (own `[workspace]`, see below) it is NOT part of the root workspace and
> so is **not** covered by the `cargo build --workspace` CI gate (ci.yml) — that
> is why a `CircuitId`/`ProofInputs`/`prover_toml_for` signature change in
> `sparq-zk-compose` (such as `FilterF64` going unit→struct) once broke it
> silently. It is now rebuilt on every ZK-touching PR by a lightweight
> `cargo build` step in `.github/workflows/zk-toolchain.yml` (a lane that already
> triggers on `crates/sparq-zk-compose/**`), so such drift is a visible red check.
> **When you change a `CircuitId`/`ProofInputs` variant or `prover_toml_for`,
> update this harness too.**
>
> The CI build is deliberately **not** `--locked` ([FABLE-5] sq-q134e): the
> harness path-deps on in-repo crates whose manifests bump dependencies on lanes
> that never run this guard, so the committed `family_curve/Cargo.lock` drifts
> silently on main and `--locked` then spuriously failed the *next* zk-touching
> PR (#1962, a main-side `hashbrown` bump). A plain `cargo build` keeps every
> still-compatible pin from the committed lock (cargo re-resolves minimally) and
> only absorbs the in-repo manifest bumps; if the lock did drift, CI emits an
> advisory notice + lock diff asking for a refresh commit instead of failing.

### Run

```sh
cd bench/zk-compose/family_curve
cargo run --release > ../family_cost_curve.json   # JSON to stdout, table to stderr
# average each member over N proves (reduces noise):
REPEATS=3 cargo run --release > ../family_cost_curve.json
```

Requires `nargo` + `bb` on PATH (it exits with an error if absent). Writes its bb
scratch under the system temp dir and cleans it per member.

### Full-family prove/verify/proof-size results

The per-member prove/verify wall-clock, proof bytes, and vk bytes are committed as
machine-readable JSON — `bench/zk-compose/family_cost_curve.json` (the consistent
full-family measurement, the authority for verify timing) — and regenerated by the
scripts below. Read the JSON for current figures rather than a baked table here.

Observations (from the committed measurements):
- **Prove time tracks gate count**: the scan members (largest gate counts) are
  the slowest; the `filter_*` members the fastest. The
  `filter_int_d{1,2,4}` members are gate-identical (17,416) and prove in the
  same ~1.06 s — `d` does not move cost (see the gate-count notes above).
- **proof size, vk size, and verify time are CONSTANT across the family**
  (14,656 B / 3,680 B / ~12 ms) — the ultra_honk succinctness property: a
  constant-size proof and a verify cost dominated by the fixed protocol, not the
  circuit. (This differs from the earlier 2-member `prove_verify_timing.json`,
  which reported a ~0.95 s scan verify on darwin/8-thread; on this aarch64 box
  verify is uniformly ~12 ms for every member. The earlier figure looks like a
  cold-vk / measurement artefact — `family_cost_curve.json` is the consistent
  full-family measurement and supersedes it for verify timing.)

## Reproduce

```sh
# gate counts (compiles the workspace, emits JSON to stdout)
bench/zk-compose/scripts/gate_counts.sh > bench/zk-compose/gate_counts_latest.json

# prove/verify timing for one member (needs a Prover.toml; the crate e2e tests
# leave real ones behind, or write one by hand)
bench/zk-compose/scripts/prove_verify.sh filter_int_d1
```

Toolchain: nargo 1.0.0-beta.21, bb 5.0.0-nightly.20260324.

## Gate counts and timing — committed JSON

### Gate counts (ultra_honk `circuit_size`)

Per-member gate counts are committed as `bench/zk-compose/gate_counts_latest.json`
(regenerated by `gate_counts.sh`, above) and regression-gated by
`crates/sparq-zk-compose/tests/gate_count_snapshot.json`. Read those for current
figures. The qualitative shape:

Notes:
- The scan members scale roughly linearly in `k * n` (the commitment-recompute
  sweep + the completeness double-loop dominate); `r` adds the row-soundness
  pass.
- `filter_int_d{1,2,4}` are **identical in gate count** (17,416): the blake3
  blackbox over the canonical token is the cost driver, and the token fits one
  64-byte blake3 block for all `d <= 19`, so digit count does not move the
  circuit size. The `d` family parameter exists only because the blackbox
  needs a comptime byte length; it leaks `ceil(log10(value))`, not gates.
- `filter_f64` was originally the cheapest member — a pure `sparq_ieee754`
  comparison, no string hashing — as a raw building block. As of sq-q7e/sq-tat it
  is **manifest-composable** (`filter_f64_d{D}`), binding the operand via the same
  canonical decimal-digit token as `filter_int`; read `gate_counts_latest.json`
  for the current per-`D` figures.

### Prove / verify wall-clock (small e2e)

The two-member e2e prove/verify timing is committed as
`bench/zk-compose/prove_verify_timing.json` (note: `family_cost_curve.json` above is
the fuller, more consistent measurement and supersedes it for verify timing).
`prove` includes `--write_vk`. Proof size is constant (ultra_honk) regardless of
circuit size; verify time grows with public-input count (scan carries the commitment
+ rows vectors, hence the higher verify cost).
