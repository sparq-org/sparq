# Engine performance review — runtime, build-cost, and coverage-lane levers from the 2026-07 profiling digests

**Authored by Claude Fable 5 (SPARQ architect tier), 2026-07-03.**
Inputs: two Opus profiling digests (runtime hot-path `perf` capture of release `sparq-cli`
on `51b2b379`; cold `cargo build --timings` build-cost profile of `sparq-engine`), both
measured on the aarch64 work box. **All figures cited here are RELATIVE, NON-CANONICAL
shapes** — they steer prioritization only and must never be baked into docs, tests, or CI
thresholds. Any adopted change proves itself on the canonical EC2 lane
(`research/ci-ec2-design.md`, greenlit sq-vw3ax.12) and against the deterministic
`scripts/perf-gate.py` ratchets, which must hold or improve. Mind the split those ratchets
enforce: every `mode: auto` metric in `bench/perf-baseline.json` hard-fails, while the
`mode: noise` timing metrics are advisory. `parse_ns_per_byte` is the sole `mode: noise`
metric — wall-clock-derived, an **advisory timing signal (tracked/warned, non-blocking)** —
and the measurement obligations below cite it in that sense, never as a hard ratchet.

## 0. Disjointness map — what this record must NOT restate

| Owned elsewhere | Owner | Boundary respected here |
|---|---|---|
| Change-based test/benchmark selection | sq-fmx4u (+ `.6` empty-affected coverage skip) | we reuse its dependency-closure logic; we do not redesign selection |
| Cross-job build caching | sq-3sbrr / #1395 + sq-6vshe.5 | no cache design here; one sequencing note (§2.2) |
| Coverage 4-way shard (done) / engine-shard deeper | sq-p0hcd / sq-piapk | we shrink what a shard *measures* and *links*, not how shards are cut |
| Runtime perf roadmap (14+2 beads) | sq-7d3dj.1–.16 | the five runtime items below are exactly the profile buckets its cross-note found un-beaded |
| M4 vector-at-a-time pipeline | sq-pntvh | composition notes per item; no vectorization design here |
| CI structural program: profile trio, feature pyramid, cache topology, heavy lanes, non-coverage exec topology, **engine crate-split RFC** | sq-6vshe.1–.7 / PR #1396 | build items here are *source-level* levers that record explicitly deferred or scoped out |

Firm position on the **engine crate split**: I endorse it, and it is already correctly
staged as a maintainer-gated RFC (sq-6vshe.3) + staged execution (sq-6vshe.4). This record
does **not** re-bead it. The monomorphization audit (§2.3) is deliberately sequenced as an
input to that RFC: if codegen weight is mostly generic instantiation, a file-split alone
under-delivers, and the RFC needs that number.

---

## 1. Runtime — the product win (extends sq-7d3dj; five un-beaded profile buckets)

### 1.1 Radix-sort the permutation indexes (+ pre-sized perm vecs, derived cold perms)

**Profile shape:** the single largest ingest self-time bucket is the comparison quicksort
family that `Graph::build` runs over integer `(Id,Id,Id)` tuples for the six permutation
indexes (~13–16% relative self-time), plus a residual `finish_grow` allocator tail on the
permutation `Vec`s.

**Mechanism.** Keys are fixed-width packed integers, the textbook radix case: pack each
tuple into a `u128` (or two-word key) in permutation order and LSD-radix-sort by byte —
O(n) passes, branchless inner loop, sequential access, trivially chunkable under rayon.
Two adjacent wins in the same code region: (a) pre-size the permutation `Vec<(Id,Id,Id)>`
from the exact triple count (the digest's remaining allocator-tail item); (b) reduce the
number of *full* sorts below six — a sorted SPO run is one stable in-place pass away from
SOP-family orders, and rarely-used permutations can be built lazily on first index probe
(measure-first sub-decision; lazy build shifts cost to first query, which the server warms
anyway).

**Expected impact (qualitative):** removes the largest single ingest bucket; ingest is the
dominant cold-load / `query-mmap` cost, so this is a direct product win for load time.

**Canonical measurement:** EC2 bench (tag `sparq-bench`, orphan-proof self-terminate):
ingest wall on the synthetic social graph + WatDiv at two scales; `perf` self-time share of
the sort family before/after; the `bytes/triple` ratchet holds or improves, and
`parse_ns_per_byte` (advisory timing signal — tracked/warned, non-blocking) shows no
regression. Correctness gate: byte-identical index content vs the comparison sort (exact
output-equivalence test), and dict-id-order determinism per
`research/dict-id-order-determinism-audit.md`.

**M4 composition:** ingest-side; fully orthogonal to sq-pntvh.

### 1.2 Prefix-memoized IRI validation fast path

**Profile shape:** the largest *aggregate* ingest cost (~30% relative) is oxttl/oxiri IRI
lexing + full RFC-3987 re-validation of every IRI occurrence, even when a dump uses a
handful of uniform prefixes. Distinct from sq-7d3dj.9, which only shortcuts *interning*
(memchr delimiters + recently-seen-term cache) and does not touch validation cost.

**Mechanism.** Two stacked fast paths in front of the oxiri automaton, both
falling back to the full parser on any miss so the accepted language is unchanged:
1. **Prefix memo:** cache the last-N validated `scheme://authority/`-prefixes (byte
   ranges); an incoming IRI that byte-matches a memoized prefix skips
   `parse_authority`/`parse_host` and validates only the suffix path/query/fragment.
2. **ASCII pre-scan:** a vectorizable one-pass check for the ASCII `iunreserved`/
   sub-delims class over the suffix; only a non-ASCII byte, `%`-escape, or delimiter
   anomaly drops to the char-by-char automaton.

**Safety (load-bearing):** this is a parser fast path in the conformance-critical ingest
lane. The acceptance proof is **differential fuzzing**: fast-path-vs-oxiri
accept/reject/normalization equivalence over the existing fuzz corpus plus an
IRI-structured generator, wired into the fuzz lane's committed corpus so the equivalence
replays deterministically per PR. No adoption without that lane green.

**Expected impact:** attacks the largest aggregate ingest cost; effect scales with prefix
uniformity (real dumps are highly prefix-uniform; WatDiv/BSBM/Wikidata all are).

**Canonical measurement:** EC2 ingest wall + `perf` share of the oxiri symbol family;
`parse_ns_per_byte` as an advisory timing signal (tracked/warned, non-blocking). A
prefix-diverse adversarial input must show no regression (memo miss cost ≈ one short
memcmp).

**M4 composition:** orthogonal (ingest).

### 1.3 Hash-join: single hash + batch row emission in `probe_emit`

**Profile shape:** the probe path hashes each join key **twice** (once for
`key_hash % JOIN_PARTS` partition selection, again inside `table.get`) and clones the full
build `Row` per emitted match — an allocation per output row on high-fanout joins.

**Mechanism.** (a) Compute the FxHash once; derive the partition from its bits and look up
via `raw_entry().from_hash(h, eq)` (hashbrown supports this without re-hashing).
(b) Replace per-match `build[bi].clone()` with fanout-aware emission: `reserve` the exact
match count, then extend a reused row buffer — or emit `(build_idx, probe_cols)` pairs and
stitch once per batch.

**M4 composition (design constraint, not an option):** sq-pntvh's morsel pipeline will
re-shape this loop. Implement (b) as the **batch emission contract** the vectorized
operator wants — emit index-pair runs, materialize per output chunk — so the scalar fix is
the same code M4 inherits rather than code M4 deletes. Coordinate on the sq-pntvh operator
spec before landing; the feature-OFF bit-identical guarantee (#1386) must hold.

**Expected impact:** removes an allocation and a redundant hash per output row on the
join-heavy shapes (star joins, high-fanout probes) that dominate BGP workloads.

**Canonical measurement:** EC2 criterion benches on star/WatDiv high-fanout queries;
allocation counts via the bench harness's counting allocator; operator-coverage `.rq`
suite for correctness; substrate perf-neutrality gate (#1303) stays green.

### 1.4 `Leapfrog::search` small-arity specialization

**Profile shape:** the WCO/LFTJ kernel is the dominant genuine query-eval cost on cyclic
shapes (~11% relative combined; `search` alone ~6.5%), carrying two `% k` modulo ops and a
double `order[..]→iters[..]` indirection per iteration.

**Mechanism.** (a) Replace `% k` with a branchless wrap (`p += 1; if p == k { p = 0 }` —
compiles to cmov) since the index only ever advances by one. (b) Hoist the
`iters[order[..]]` re-resolution out of the loop into a pre-permuted local slice. (c) A
monomorphized `k == 3` fast path for the triangle/3-clique case that dominates WCO
workloads — small, contained, no trait surface change.

**Expected impact:** micro but concentrated — this is the hottest single query-eval loop
on cyclic/analytic queries, sparq's WCO differentiator.

**Canonical measurement:** EC2 triangle/clique microbench + the graph-pattern slice of the
bench suite; `perf` self-time of `Leapfrog::search`. Arity ≠ 3 paths covered by the
existing LFTJ correctness tests (unchanged results required).

**M4 composition:** safe independently — trie-based WCO join stays tuple-at-a-time under
M4 (it does not vectorize naturally); this loop survives the morsel refactor.

### 1.5 `from_utf8_unchecked` on parser-validated spans + hash-before-memcmp in dict dedup

**Profile shape:** ~2% relative re-validating UTF-8 on spans oxttl already proved valid,
plus `memcmp` on dict hash-collision key comparison.

**Mechanism.** (a) `from_utf8_unchecked` where the producing parser guarantees UTF-8
boundaries — each site gets an entry in the unsafe-justification register (threat-model
boundary B5 discipline, `unsafe-rust-attestation`), a debug-assertion re-check, and Miri
lane coverage. (b) In `Dict` dedup, compare the stored 64-bit hash before falling to
`memcmp` (skip if already done — verify first).

**Expected impact:** small, steady ingest win; zero algorithmic risk but real attestation
cost — which is why it ranks below the three items above despite being trivial.

**Canonical measurement:** EC2 ingest wall delta; Miri lane green; cargo-geiger ratchet
accounted; `parse_ns_per_byte` (advisory timing signal — tracked/warned, non-blocking)
shows no regression.

---

## 2. Build cost — dev + CI product velocity (source-level; disjoint from sq-6vshe's CI-side program)

Cold-build shape (relative): the test profile is the expensive lane by a wide margin, with
**27 distinct `sparq-engine` compile units** (lib + ~26 integration-test crates) that each
re-link and re-monomorphize the engine; release CPU is ~1.7× debug purely from
`codegen-units=1 + fat LTO`; dev incremental is already sub-second and needs nothing.

### 2.1 Consolidate the engine integration-test fan-out (~27 units → ~5 harnesses)

**Mechanism.** Merge the ~26 `tests/*.rs` files into a handful of harness crates (a
`tests/main.rs` per theme that `mod`-includes the existing files — file contents move
verbatim, only the crate boundary changes). Keep: any test needing a genuinely separate
process/feature-cfg stays its own binary; the `cargo test --doc` lane is untouched (nextest
does not run doctests — sq-6vshe §8's constraint, honored here). nextest partitioning is
per-test, not per-binary, so shard balance survives.

**Expected impact:** the test-profile lane is dominated by this fan-out (engine test units
were the largest CPU sum in the profile); consolidation cuts N re-monomorphizations +
N links to ~5, shortening every `cargo test`/clippy-with-tests/CI test leg **and** the
sq-piapk coverage shard (each integration-test crate is a separately instrumented+linked
binary in the coverage build — same fan-out, higher per-unit cost).

**Risk:** low-medium, mechanical — test-name collisions on merge, per-file `cfg` gates,
serial-test interactions. No production code touched.

**Measurement:** `cargo build --timings` unit-count + CPU-sum before/after on EC2 or the
CI trend; coverage-shard wall trend. No baked thresholds.

**Disjointness:** sq-6vshe.7 rebalances *execution* of existing binaries; this changes the
*source topology* that fixes their compile+link bill. Cross-filed with sq-piapk.

### 2.2 Split the ship profile from the everything-else release profile

**Mechanism.** `[profile.release]` today carries `codegen-units=1 + fat LTO` — correct for
the shipped artifact, waste for every other `--release` consumer (CI release-lane smoke,
perf-gate compile, local perf iteration). Add `[profile.release-fast]` (`inherits =
"release"`, `codegen-units=16`, `lto="thin"` or off) and point non-shipping lanes at it;
release/bench/canonical artifacts keep the fat profile.

**Honesty guard (load-bearing):** anything **measured** — canonical EC2 benches, the
perf-gate ratchets, published numbers — stays on the ship profile. `release-fast` is for
"does it build / does it pass", never for perf claims. The lane assignment list goes in
the PR body for maintainer steer (proceed-and-document).

**Sequencing:** profile/flag changes rekey compiler caches — land alongside or before the
sq-6vshe.1 flag set so caches are primed once against the final set (its §5 sequencing
rule; this bead extends that set with the release-lane axis it explicitly left alone).

**Expected impact:** removes the fat-LTO single-threaded tail and CGU=1 serialization from
every non-shipping release build; also the cheapest dev-side lever for local perf-ish
iteration.

**Risk:** low; main failure mode is someone benchmarking `release-fast` by accident —
mitigated by naming + the honesty guard note in `AGENTS.md`'s bench guidance.

### 2.3 Monomorphization diet in `exec.rs` — measure-first, feeds the split RFC

**Mechanism.** `cargo llvm-lines` + `--timings` self/codegen attribution over the engine:
rank generic instantiation weight (the `JoinKeys` kernel family, `CompareTerm`, per-row
FILTER/BIND evaluators). Then outline only **cold** call sites: `#[inline(never)]` shims,
arg-struct de-duplication, or enum dispatch at setup/plan-time boundaries — the hot row
path stays generic/monomorphized. Hard constraint: the substrate **no-dyn-dispatch
perf-neutrality gate** (#1303) and the EC2 runtime benches must stay green; any candidate
that costs runtime is rejected regardless of build win.

**Expected impact:** shrinks the engine's release/test codegen (its lib is the largest
single unit in every profile and codegen-dominated in release); multiplies 2.1 (less to
re-monomorphize per harness). **Sequencing value:** its output is exactly the number the
sq-6vshe.3 split RFC needs — if generics dominate codegen, a file split alone
under-delivers and the RFC should say so.

**Risk:** medium — runtime-perf coupling; strictly measure-first, small PRs per site.

**Measured outcome (sq-6vshe.12, indicative workbox `cargo llvm-lines -p sparq-engine
--release`; raw numbers live in the bead per repo hygiene).** The measure-first step
returned a NO-CHANGE verdict for the diet *as an intervention*, and a NOT-A-BLOCKER
verdict for the split. Engine-defined monomorphization is a low-single-digit fraction of
engine IR: `exec.rs` has only two generic fns, the substrate `JoinKeys` is a concrete
struct and `compare_terms` monomorphizes once (to `Value`), and the crate exports ~zero
generic surface. Engine's own codegen is dominated by large *non-generic* function bodies
(`eval_function`, `eval_cast`, `path_pairs`, `eval_expr`, `single_pattern_scan_json`,
plus `explain`/`update`); the majority-of-IR monomorphization is std/rayon/hashbrown
library generics triggered at call sites, which are not "engine generics to diet." The
few engine multi-copy families (`digest_hex` ×5 hash builtins, `cmp_expr` ×4 ORDER BY,
scan/group closures) all sit on per-call FILTER/BIND, sort, or scan eval paths, so
de-monomorphizing them (e.g. `DynDigest`) would add runtime dispatch and is **rejected
under #1303 regardless of the build win**. The only strictly-cold engine generics
(`explain::render_*` closures) are sub-0.3% and not worth a change. Net: no safe cold
outlining candidate with a material build win exists → deliver measurement + verdict, no
code PR. Sequencing payoff instead flows to Option A (the opt-in periphery — serialize,
window, service, params, zk, txn — roughly *doubles* engine IR when fully on; peeling it
removes real feature-on codegen) and to Option C (in-crate `exec.rs` modularization),
per the split RFC's re-scored D2.

### 2.4 Dev-dep / proc-macro trim

**Mechanism.** (a) `serde_derive` enters the test profile only via the `serde_json`
dev-dependency — scope it to the test crates that actually parse JSON (post-2.1 this is a
few harnesses, not 26 crates). (b) Audit whether derive-heavy `zerocopy` is needed in the
default dependency graph or is a transitively-activated feature that can be trimmed.
(c) Position on `regex`/`digest` defaults: **do not flip** — SPARQL `REGEX`/hash builtins
working out-of-the-box is conformance posture, and default-feature changes are
semver-visible; the fast-feedback win belongs to the sq-6vshe.2 check-tier and (optionally)
a documented local `--no-default-features` check loop instead.

**Expected impact:** small constant CPU off the test-profile critical path; near-zero risk.

**Audit outcome (sq-6vshe.13).** Mechanism (a) as stated above is **refuted**, and (b) found
nothing trimmable. Recorded so the item is not re-opened on the same premise:

- **(a) `serde_derive` does not enter via `serde_json`.** `serde_json` depends on the `serde`
  facade with the `derive` feature *off*; the proc-macro is activated instead by ten crates'
  **non-dev** `serde = { features = ["derive"] }` edges (`sparq-nlq`, `-forms`, `-lws-core`,
  `-mcp`, `-zk-compose`, `-metamorph`, `-introspect`, `-shacl`, plus optional edges in
  `-fedclient`/`-kb`). Because features unify across a workspace resolution, `serde_derive`
  compiles in *any* workspace build regardless of how `serde_json` dev-deps are scoped — so
  scoping them buys no proc-macro time, only `serde_json`+`itoa`+`zmij` codegen units.
- **Scoping is already tight.** All thirteen crates carrying a `serde_json` dev-dep genuinely
  use it from a dev target, so there is nothing to delete on usage grounds. The one real
  finding was a *redundant* declaration: `sparq-lws-core` listed `serde_json` in both
  `[dependencies]` (unconditional, non-optional) and `[dev-dependencies]`; the dev entry was
  removed. `sparq-engine` and `sparq-vectors` also carry both, but there the non-dev edge is
  `optional = true`, so their dev entries are load-bearing and stay.
- **(b) `zerocopy` is not in the lean default graph and is not trimmable from our manifests.**
  No sparq crate depends on it directly. It enters only transitively via `ppv-lite86`
  (← `rand_chacha` ← `proptest`/`rand`), `ahash` (← `hashbrown 0.14` inside `hdt`/`parquet`,
  and the arkworks stack under `sparq-zk`), and `half` (← `ciborium` ← `criterion`, and
  `arrow`/`parquet`/`naga`). `sparq-core`'s own `hashbrown 0.17` hashes with `foldhash`, not
  `ahash`, so the lean core/engine build never reaches `zerocopy`. `zerocopy-derive` is turned
  on by those third-party manifests' own feature selections, which we cannot override without
  dropping `proptest`/`criterion` — not on the table. **No action.**
- **(c)** Position on `regex`/`digest` defaults is unchanged: do not flip.

---

## 3. Coverage-lane speed (beyond sq-p0hcd's shard, sq-piapk's engine shard, fmx4u.6's empty-set skip)

The structural insight: the ratchet is a **per-crate floor**, so the unit of work is the
crate — which makes coverage the *most* scopable lane in CI, and today it is the least
scoped (every PR measures every crate, 4-way sharded).

### 3.1 Changed-cone coverage with baseline inheritance (the big one)

**Mechanism.** Per PR, compute the changed-crate set and its **reverse dependency
closure** (the same closure sq-fmx4u §5 builds — reuse, don't re-implement). Run
instrumented coverage **only for crates in that closure**; every crate outside it inherits
the floor comparison result from `main`'s latest full measurement (its lines cannot change
and the tests that produce its coverage are untouched — the same soundness argument
fmx4u's test-skipping already committed to, applied to measurement). Fail-safe identical
to fmx4u: shared/CI/lockfile changes ⇒ full run; nightly full coverage on `main` stays
the drift backstop, and per-crate floors remain enforced exactly as today for measured
crates.

**Expected impact:** the modal leaf-crate PR measures a handful of crates instead of the
whole workspace ×4 shards; even engine-touching PRs skip every crate outside the engine's
cone. This attacks total coverage compute, where sq-piapk attacks the single worst shard —
they compose.

**Risk:** low-medium — correctness of the closure (inherited from fmx4u, fail-safe), plus
baseline bookkeeping (which `main` run produced the inherited numbers must be recorded in
the gate output for auditability).

**Measurement:** coverage-lane wall-clock trend by PR class; zero change in enforced
floors; a deliberate canary (PR that lowers a dependent crate's coverage) must still fail.

### 3.2 Instrumentation-weight diet: `except-unused-generics` A/B (+ instrumented-profile knobs)

**Mechanism.** The engine is generics-heavy (§2.3), and default
`-C instrument-coverage` instruments **every monomorphized instantiation** — unused-generic
counters bloat instrumented binaries, link time, and profraw size across 27 (→ ~5 after
2.1) test binaries. A/B `-C instrument-coverage=except-unused-generics` (nightly-gated
value; the coverage lane already tolerates pinned-nightly patterns — verify, else park
until stable). Also audit instrumented-profile debuginfo: coverage mapping does not need
full DWARF, so the sq-6vshe.1 line-tables-only trio should apply to this lane too (verify
it reaches the coverage jobs' env).

**Governance flag (maintainer-visible):** changing the instrumentation mode changes
per-crate coverage **denominators** — a one-time floor re-baseline under the new mode is a
measurement-definition change, not a regression, and must be a separate, loudly-labeled PR
with before/after floor tables so the ratchet's hold-or-improve invariant stays honest.

**Expected impact:** smaller instrumented binaries + faster coverage build/link/run; the
generics-heavy engine shard benefits most (composes with sq-piapk).

**Risk:** medium — nightly flag stability + the re-baseline governance step.

#### 3.2 OUTCOME (sq-6vshe.11) — instrumentation A/B PARKED, trio audit found a real gap

This section's two halves resolved differently, so both verdicts are recorded here rather
than left to be re-derived.

**Half 1 — `except-unused-generics`: PARKED, and not "until stable".** The premise above is
stale. The value is not merely nightly-gated; `rustc` no longer accepts it *at all*.
Measured directly against `rustc`:

- `-C instrument-coverage=except-unused-generics` is rejected with
  *"incorrect value … one of: `y`, `yes`, `on`, `true`, `n`, `no`, `off` or `false` was
  expected"* — i.e. `-C instrument-coverage` now takes only booleans (plus the deprecated
  `all` alias, which still parses).
- The same value is **also** rejected under `-Z unstable-options`, so escaping the coverage
  lane to a pinned nightly would buy nothing. There is no channel on which this works.
- The surviving nightly knob is `-Z coverage-options`, whose accepted values are
  `block | branch | condition | mcdc | no-mir-spans`. None of these is an unused-generics
  diet; they add finer-grained coverage, they do not remove monomorphization counters.

Consequence for governance: **there is no floor re-baseline to do.** The re-baseline was
required only because a changed instrumentation mode would change per-crate coverage
denominators. The mode cannot change, so the denominators do not move, and no
measurement-definition PR is owed. The "loud standalone PR with before/after floor tables"
obligation is discharged as *not applicable*, not skipped.

Caveat on the measurement: it was taken on the `rustc` available to the implementing agent
(1.88.0), not on the repo's pinned toolchain — that pin could not be materialised in the
sandbox (read-only `rustup` directory). Option removals are not reverted, and the `-Z
coverage-options` enumeration observed there is already the modern surface, so the verdict is
not expected to differ on the pin. Anyone re-opening this should re-run those two commands on
the pinned toolchain before spending further effort.

**Half 2 — the sq-6vshe.1 trio audit: one leg was NOT reaching the coverage lane.** Verified
per leg rather than assumed:

| Leg | Reaches the coverage jobs? | How verified |
|---|---|---|
| `CARGO_INCREMENTAL=0` | yes | plain cargo env var, no competing source |
| `CARGO_PROFILE_DEV_DEBUG=line-tables-only` | yes | ran `cargo test --no-run -v` with it set; `rustc` is invoked with `-C debuginfo=line-tables-only` (the `test` profile inherits `dev`, and a config/env profile key overrides the manifest) |
| `CARGO_TARGET_<triple>_RUSTFLAGS="-C link-arg=-fuse-ld=lld"` | **no** | see below |

Cargo resolves rustflags from `CARGO_ENCODED_RUSTFLAGS` > `RUSTFLAGS` >
`target.<triple>.rustflags` > `build.rustflags` and uses **only the first source that is
set** — it never merges them. Confirmed directly: with only the per-target variable set,
`rustc` receives its flags; with `RUSTFLAGS` also set, the per-target variable is dropped
entirely. `cargo-llvm-cov` sets the top two variables to inject `-C instrument-coverage`
(that is what `cargo llvm-cov show-env` exports, and what `coverage-engine-shard.sh
build-objects` sources), so **every coverage job was linking with the default linker** while
every sibling native job used lld — silently, since nothing fails when a flag stops applying.

Fix landed with this bead: the four `cargo-llvm-cov` jobs (`coverage-measure`,
`coverage-engine-run`, `coverage-engine-merge`, `coverage-nightly`) set `RUSTFLAGS` at job
level, on the source `cargo-llvm-cov` reads and extends rather than one it shadows. That
"extends rather than replaces" step is `cargo-llvm-cov`'s documented behaviour and was **not**
measured here — the tool is not installed on the authoring box, so it should be confirmed on
the first CI run from a coverage job's compile line. The failure mode is benign either way: if
it replaced `RUSTFLAGS`, this change is a no-op (status quo) rather than a regression. All
four use one identical value
— required independently by the engine coverage split, whose cross-runner `.profraw` merge is
only valid while every compile sees identical inputs. `scripts/check-coverage-rustflags.py`
gates both the presence and the sameness so the shadowing cannot silently return.

Not claimed: any specific speed-up. This bead makes the flag *apply*; whether lld is faster
on instrumented links is the same bet sq-6vshe.1 already made and merged for every other
native lane, and the coverage-lane wall-clock trend is the measurement that settles it.

### 3.3 Cross-benefit accounting (no new bead)

§2.1's test-crate consolidation is itself a first-order coverage lever (fewer
instrumented links); §2.2's `release-fast` does not touch coverage (coverage builds are
test-profile). Recorded so sq-piapk's implementer sequences after 2.1 lands.

---

## 4. Bead program — ordered by impact-per-risk

Runtime beads attach to the sq-7d3dj optimization epic (extending its roadmap exactly
where its own cross-note found gaps); build/coverage beads attach to the sq-6vshe CI
structural program (source-level + coverage levers its record scoped out or deferred).

| # | Bead | Dim | What (one line) | Impact | Risk | Tier |
|---|---|---|---|---|---|---|
| 1 | sq-7d3dj.17 | runtime | radix-sort permutation indexes + pre-sized perm vecs + derived/lazy cold perms | largest single ingest bucket | low (output-equivalence testable) | sonnet |
| 2 | sq-6vshe.8 | coverage | measure only the changed-crate reverse-closure; inherit main baselines outside it | whole-lane, every PR | low-medium (fmx4u closure reuse, fail-safe) | sonnet |
| 3 | sq-6vshe.9 | build | ~27 engine test crates → ~5 harnesses (source topology) | dominant test-profile CPU + link tail + coverage shard | low-medium (mechanical) | sonnet |
| 4 | sq-7d3dj.18 | runtime | prefix-memo + ASCII pre-scan in front of oxiri, differential-fuzz equivalence gate | largest aggregate ingest cost | medium (conformance-critical; fuzz-gated) | opus |
| 5 | sq-6vshe.10 | build | `release-fast` (CGU16/thin-LTO) for non-shipping lanes; fat LTO reserved for ship/bench | every non-shipping release build; dev iteration | low (honesty guard on measured lanes) | sonnet |
| 6 | sq-7d3dj.19 | runtime | single-hash `raw_entry` probe + batch row emission designed as the M4 contract | alloc+hash per output row on fanout joins | medium (M4 coordination) | sonnet |
| 7 | sq-6vshe.11 | coverage | `except-unused-generics` A/B + instrumented-profile knobs + governed floor re-baseline | coverage build/link weight, engine shard most | medium (nightly flag + re-baseline governance) | sonnet |
| 8 | sq-7d3dj.20 | runtime | branchless wrap + hoisted iters + `k==3` fast path in `Leapfrog::search` | hottest query-eval loop on WCO shapes | low (contained) | sonnet |
| 9 | sq-6vshe.12 | build | `cargo llvm-lines` audit + cold-site outlining; feeds the sq-6vshe.3 split RFC | engine codegen in every profile; de-risks the split | medium (runtime coupling; measure-first) | opus |
| 10 | sq-7d3dj.21 | runtime | `from_utf8_unchecked` on parser-proved spans + hash-before-memcmp, full unsafe-register discipline | small steady ingest win | low perf / real attestation cost | opus |
| 11 | sq-6vshe.13 | build | scope `serde_json` dev-dep, audit zerocopy; do NOT flip regex/digest defaults | small constant test-profile CPU | near-zero | haiku |

Maintainer-decision flags: the engine **crate split** stays gated at sq-6vshe.3/.4
(endorsed, not re-beaded; #9 feeds it). #7's floor **re-baseline** was flagged as a
measurement-definition change requiring a loud standalone PR — **resolved as not applicable**:
§3.2 OUTCOME measured that the instrumentation mode cannot change, so no denominator moves
and no re-baseline is owed. #7 shipped only its trio-audit half. #5's lane assignment
(what stays on the fat ship profile) is listed for steer in its PR body
(proceed-and-document).
