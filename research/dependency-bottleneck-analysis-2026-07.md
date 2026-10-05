# Dependency + Rust-toolchain bottleneck analysis (2026-07)

> 🤖 SPARQ agent [FABLE-5] — maintainer-commissioned assessment (front 2 of 3). Epic: **sq-98w7z**
> ("Upstream dependency + Rust bottleneck program"). This record is the decomposition artifact:
> it ranks every dependency on sparq's hot paths, classifies each bottleneck's remedy, and cuts
> the work into nine disjoint child beads. **No new measurements were taken for this record** —
> every number is cited from a prior record with its own provenance label (canonical = the AWS
> bench fleet per `research/hw-bench-results.md`; work-box/M1 = NON-canonical by standing rule).
> This front covers what is **upstream of sparq** (third-party crates + the Rust toolchain);
> the in-house custom-parser program (sq-jocpn et al.) and the comparative-benchmark program
> (sq-hmd7l) are adjacent fronts — cited, not duplicated.

## 1. Scope and method

The mandate: sparq must dominate every axis, so upstream dependencies that cap an axis are
in-scope to patch, upstream, swap, or replace — the same calculus that produced the custom
N-Triples parser (`research/custom-parsers-baseline.md`) and the D4 compression sinks
(`research/custom-parsers-D4-compressed-serialization.md`).

Ground truth was established against the real tree (not the epic's framing): `Cargo.lock`
(575 packages, 52 workspace crates), per-crate manifests, grep of actual usage sites in
`sparq-core` / `sparq-engine` / `sparq-substrate` / `sparq-vectors` / `sparq-parse` /
`sparq-server`, the release profiles in the root `Cargo.toml`, CI RUSTFLAGS, and the prior
research estate (`hw-bench-results.md`, `custom-parsers-*.md`, `gap-vector-*.md`,
`optimization-audit-2026-07.md`, `engine-performance-review.md`, `parallelism-scaling.md`).

Two corrections to the commissioning premise, found by grounding:

- **ahash/foldhash are not sparq's hashers.** They are transitive only. The hot maps are
  `FxHashMap`/`FxHashSet` (rustc-hash 2.x) and `hashbrown::HashMap` with `FxBuildHasher`
  (the substrate join table, `crates/sparq-substrate/src/join.rs`).
- **chrono/time are not in the tree at all.** xsd:dateTime parse/compare is a hand-rolled
  `Timeline`/`Temporal` implementation (`crates/sparq-core/src/temporal.rs`) with an
  `FxHashMap<Id, Temporal>` side-cache — the "datetime dependency" axis has already been
  rolled in-house and is not a bottleneck.

## 2. Ranked bottleneck table

Remedy classes: **(a)** patch-and-upstream, **(b)** roll-our-own, **(c)** swap,
**(d)** feature-flag/config already available, **(e)** local misuse of a fine dependency
(the fix is in sparq, not the dep). Rank = hot-path impact × fixability.

| # | Dependency (lock version) | Axis | Hot-path weight (evidence, provenance) | Bottleneck | Remedy | Recommendation | Bead |
|---|---------------------------|------|----------------------------------------|------------|--------|----------------|------|
| 1 | `regex` 1.12.4 | FILTER eval | `build_regex()` is called **inside the per-row scalar eval** for `REGEX()`/`REPLACE()` (`crates/sparq-engine/src/exec.rs` ≈12634/12646, no cache anywhere — grep-verified). Regex compile is µs-scale vs ns-scale match, so a constant-pattern FILTER over N rows pays N compiles. | Local misuse — the crate itself is fine | (e) | Capped string-keyed compile memo; same bead fixes `RAND()` per-row OS-RNG draw (`uuid::Uuid::new_v4()` per row) and audits `NOW()` per-row re-evaluation, which if real violates SPARQL 17.4.5.1 query-constancy (**correctness**, not just perf). | sq-98w7z.1 (P1, sonnet) |
| 2 | `flate2` 1.1.9 / `miniz_oxide` 0.8.9 | Result serialization (D4) | gzip is the **only** codec that cannot keep up with the serializer: gzip -6 at 8T ≈ 750–811 MB/s vs ≈3,099 MB/s production; zstd -3 hides entirely (D4 record, work-box, NON-canonical). | Default miniz_oxide backend is the slow leg | (d)/(c) | Enable flate2's `zlib-rs` backend as a cargo feature and A/B it on the D4 harness. Upstream reports it substantially faster at compression — **verify locally, adopt only on a measured win**. | sq-98w7z.2 (P2, sonnet) |
| 3 | `oxttl` 0.2.3 | Turtle parse | ~53% of single-thread Turtle parse is inside oxttl's tokenizer, incl. a per-prefixed-name `format!`/`ToOwned` String allocation; single-thread Turtle is ~2× BEHIND serd (sq-wrn61, canonical c6i.4xlarge). | Missing low-copy fast path in the dep | (a) | Upstream a low-copy prefixed-name expansion to oxigraph/oxigraph. Complements — does not preempt — sq-jocpn's roll-own-vs-stay decision: even if sparq goes custom, wasm + TriG/N-Quads stay on oxttl. Dep-gated on sq-jocpn. | sq-98w7z.3 — **MEASURED, see `research/oxttl-prefixed-name-alloc-2026-07.md`: already fixed upstream (oxigraph `f5383d8`, unreleased); no PR to file, the lever is the version pin** |
| 4 | rustc/LLVM (toolchain) | All compiled hot loops | Release profile already saturated (fat LTO, CGU=1, panic=abort); `-Ctarget-cpu` tiers measured **zero** uplift (canonical, `hw-bench-results.md`). PGO/BOLT have **never been evaluated** (verified: no record, no bead). | Unexplored profile-feedback lever | (d) | Measured PGO experiment (instrument → train on watdiv/sp2b/bsbm + ingest → use); BOLT only if PGO ≥3%. Bench scripts only; adoption is a separate decision. | sq-98w7z.4 (P2, sonnet) |
| 5 | rustc/LLVM (toolchain) | Decode/join inner loops | `chunk.rs` decode loops are *commented* as auto-vectorizable; the substrate join probe and dict `find_iri` have never had bounds-check elision verified against actual asm. | Assumed-good codegen, unverified | (a) if real | cargo-asm audit of ~3 named hot functions; classify confirmed / locally-fixable-shape / genuine missed-optimization → minimized testcase + drafted rust-lang/rust issue. Honest expectation: most will be confirmed fine; the value is evidence over assumption. | sq-98w7z.5 (P3, opus, upstream-pr) |
| 6 | `rustc-hash` 2.1.3 (FxHash) | Dict intern (parse) | Intern is ~30% of single-thread parse+intern (sq-wrn61, canonical); the ingest intern cache is `FxHashMap<Box<[u8]>, u32>` (`dictspill.rs:619,684`). FxHash is near-optimal for `Id` keys but not SOTA for variable-length byte strings. | Possibly sub-optimal hasher for byte keys only | (c) | A/B foldhash (already in-tree transitively) on the byte-keyed cache **only**; honest ceiling is low-single-digit % end-to-end — adopt only if ≥~3%, else close as measured-no. | sq-98w7z.6 (P3, sonnet) |
| 7 | duplicate lock versions | Build/binary size | hashbrown ×4 (0.14/0.15/0.16/0.17), rustc-hash ×2, oxrdf ×2, oxttl ×2, quick-xml ×2 (0.37 via `sparesults` 0.3.3 + 0.41), foldhash ×2. | Compile-time + binary-size waste — **not** a runtime hot-path cost | (c) | Dedupe where upstream requirements allow; record hard pins (sparesults→quick-xml 0.37) and move on. Sequenced after beads 2 and 6 (manifest touchers). | sq-98w7z.7 (P3, haiku) |
| 8 | `spargebra` (vendored 0.4.6) | SPARQL parse | Vendored with 6 conformance patches; all 6 fixes already on oxigraph main (`dabda10`, `c29be03`) but unreleased (`docs/upstream-proposals.md`). Not a perf bottleneck — a maintenance liability. | Upstream release lag | (a) done — awaiting release | Retire `vendor/spargebra` + `[patch.crates-io]` when a >0.4.6 release ships; full W3C conformance re-run is the gate. Blocked-external. | sq-98w7z.8 (P3, haiku) |
| 9 | `instant-distance` 0.6.1 | Vector ANN | HNSW QPS 6–20× BEHIND hnswlib at matched recall (SIFT1M, work-box, ranking stable); 1M build >30 min vs hnswlib 374 s. Root cause: scalar kernel (now overlaid by sparq's own NEON/AVX2 SIMD, sq-lfo84 merged) + **serial graph construction**. | Serial build in the dep | (c) in flight | **Already owned by in-flight beads** — sq-ose80 (parallel-build backend swap: hnsw_rs/usearch) and sq-9wrkc (extend the SIMD kernel). No new bead; do not duplicate. | — (cite sq-ose80/sq-9wrkc) |

The cross-cutting gate: **sq-98w7z.9** (P2, sonnet) — canonical re-measure of every landed
remedy on the canonical bench instance before any dominance/adoption claim, per the standing
non-canonical-work-box rule. It is dep-blocked on beads .1/.2/.4/.6.

## 3. Confirmed NOT the bottleneck (honest negatives)

Verified against code + prior measurements; re-proposing these would waste fleet cycles.

| Candidate | Verdict | Evidence |
|-----------|---------|----------|
| `serde_json` | Not on the results hot path | `sparq-engine/src/json.rs` is a hand-written escaping String-builder (deliberately serde-free, shared with wasm); `sparq-engine-serialize` uses serde_json as a dev-dep only. Remaining uses: server SSE/subscriptions/WAL metrics (feature-gated, not the SELECT path). |
| `zstd` 0.13.3 | Measured headroom, not a limit | zstd -3 at 8T compresses ~2× faster than the serializer produces; decode single-thread already outruns 8T parse+build (D4/ADDENDUM records, work-box). |
| `bzip2` 0.6.1 | Rejected remedy stands | ~15–20 MB/s single-stream; verdict remains "recompress `.bz2`→`.zst` once", not a parallel decoder (`custom-parsers-baseline.md` REJECT list). |
| chrono/time/jiff | Not in the tree | Custom `temporal.rs` + `FxHashMap<Id, Temporal>` side-cache is the datetime hot path. Nothing to swap. |
| Global allocator | Already done | mimalloc 0.1.52 is the global allocator in sparq-server (unconditional) and sparq-cli (default-on feature), shipped for the per-row SmallVec contention finding (`parallelism-scaling.md`). A jemalloc re-measure is not currently justified. |
| `-Ctarget-cpu` tiers | Measured inert | Zero uplift within ±2% noise on both Sapphire Rapids AVX-512 and Graviton3 (canonical, `hw-bench-results.md`). |
| std::simd / portable-SIMD (Rust itself) | Not the binding constraint | Still a nightly-only feature; sparq policies a stable toolchain (MSRV 1.88). The hot SIMD (sparq-vectors L2 kernel; core prefetch hints) already uses stable `core::arch` with runtime dispatch, and the D4/baseline records show byte-scanning SIMD is not where the time goes. No rustc/std contribution is warranted on this axis today; re-check at stabilization. |
| `rayon` 1.12 | Not the ceiling | The 8T plateau (2.2–2.5× on most subsystems) is memory bandwidth + serial merge points (`Dict::merge_remap`, merge-join), i.e. sparq-side structure (morsel/columnar territory), not rayon overhead. |
| `smallvec` / `memchr` | Correct choices, hot and healthy | `Row = SmallVec<[Id; 4]>` and the memchr-driven terminator scans are the already-optimized incumbents; the baseline record explicitly rejects SIMD-scanner rewrites as first-order work. |
| axum/hyper/tokio | Streaming path already sound | SELECT JSON/CSV/TSV stream through `spawn_blocking` + bounded mpsc + `Bytes` (one copy per chunk). XML stays buffered — minor, low-traffic; not upstream's fault. |
| wasm-bindgen boundary | Defer to harness | Results cross as UTF-8 JSON strings (no serde-wasm-bindgen). Whether string-crossing + `JSON.parse` is a real cost is exactly what the in-browser latency harness (sq-hmd7l.17) exists to measure — no dep action until it reports. |
| `quick-xml` | Cold path | Only on SERVICE-client response parsing, RIF import, conformance, and the server's sparesults path — not on mainline query eval. The ×2 version split is bead .7's (hygiene) concern only. |

## 4. Rust-itself verdict

Honest summary: **Rust is not sparq's bottleneck today.** The three toolchain-level levers
that remain are profile feedback (PGO/BOLT — genuinely unexplored, bead .4), codegen
*verification* of the assumed-vectorized loops (bead .5, with an upstream testcase only if a
real missed-optimization falls out), and portable-SIMD stabilization (watch, no action).
Everything else at this layer is either done (mimalloc, fat LTO/CGU=1, per-ISA prefetch
tuning with the Graviton3 OFF default) or measured inert (`-Ctarget-cpu` tiers). The MSRV
floor (1.88) is upstream-driven (`geo` 0.33.1) and has no perf consequence.

## 5. Decomposition (the child beads)

All nine beads live under epic **sq-98w7z**; each carries `{crate, model_tier, invariant,
acceptance_test}` in its body and labels. Disjointness: no two beads share a file. The three
root-manifest touchers are serialized by dep edges instead of split-brain edits
(.2 → .7 → .8, and .6 → .7). Bead .3 is dep-gated on sq-jocpn so the two fronts of the
Turtle-parse work cannot diverge. Bead .9 (canonical re-measure) is dep-blocked on
.1/.2/.4/.6 and is the only place a dominance/adoption claim may be minted.

| Bead | Title (short) | Crate/surface | Tier | File-area |
|------|---------------|---------------|------|-----------|
| sq-98w7z.1 | Memoize per-row builtin costs (REGEX/REPLACE cache, RAND PRNG, NOW constancy) | sparq-engine | sonnet | `crates/sparq-engine/src/exec.rs` + engine tests |
| sq-98w7z.2 | flate2 `zlib-rs` backend A/B for D4 gzip | sparq-parse | sonnet | `crates/sparq-parse/**` (+ ≤1 root dep line) |
| sq-98w7z.3 | Upstream oxttl low-copy prefixed-name expansion | upstream (oxigraph) | opus | `bench/parse/oxttl-prefix-alloc/` + external PR |
| sq-98w7z.4 | PGO (+BOLT) evaluation | bench | sonnet | `bench/pgo/` |
| sq-98w7z.5 | Codegen audit of hot loops (asm evidence; rust-lang testcase if real) | bench / upstream | opus | `bench/codegen-audit/` + external |
| sq-98w7z.6 | Intern-cache hasher A/B (FxHash vs foldhash, byte keys only) | sparq-core | sonnet | `crates/sparq-core/src/dictspill.rs` + its manifest |
| sq-98w7z.7 | Dedupe multi-version lock entries | workspace | haiku | `Cargo.lock` + manifests (after .2/.6) |
| sq-98w7z.8 | Retire vendor/spargebra on upstream release | workspace | haiku | `vendor/spargebra/**` + root patch table (after .7; blocked-external) |
| sq-98w7z.9 | Canonical re-measure gate | bench | sonnet | `bench/canonical-competitor-results/**` (after .1/.2/.4/.6) |

Coordination notes for the fleet: bead .1 shares `exec.rs` with open PR #1878 (index-carry
top-k) — rebase after it merges. Beads .3 and .6 both cite the sq-wrn61 profile; .3 changes
no sparq code, .6 changes only `dictspill.rs`, so they parallelize. Nothing here touches
ZK/MPC surfaces, so no soundness-sensitive maintainer-arm flag applies to this program.

## 6. What was deliberately not beaded

- **instant-distance replacement** — owned by in-flight sq-ose80/sq-9wrkc; duplicating the
  backend-swap decision here would collide with live work.
- **JSON-LD / RDF/XML throughput** — no measurement exists yet; harnesses are sq-hmd7l.15
  and the comparative program's job. Beading a remedy before a measurement would violate the
  profiling-first rule.
- **A `release-fast` CI profile** — already proposed in `research/optimization-audit-2026-07.md`;
  it is a CI-time concern, not a runtime-perf-upstream concern.
- **w3c/rdf-tests issue filings** — tracked in existing beads per `docs/upstream-proposals.md`;
  not perf.
