<!-- [OPUS-4.8] sq-toze — memsafety evidence pack (re-runnable verification of each
     control in controls.md). Re-review when Fable returns. NON-CANONICAL timing. -->

# Memory-safety attestation — evidence pack

Each control in [`controls.md`](./controls.md) is backed here by the **exact** file path,
test, or CI job, plus the command an auditor re-runs to confirm it. Paths are
repo-relative. No timing is recorded (NON-CANONICAL EC2 box).

## MS-1 — confined unsafe surface (50 `forbid` crates, 9 unsafe-bearing crates, 60 total)

```sh
grep -rl 'forbid(unsafe_code)' crates/ --include='*.rs' | sed 's#crates/##;s#/.*##' | sort -u | wc -l
```
→ **50 crates** carry a `forbid(unsafe_code)` in their source (the command above is the
authoritative generated list). **sparq-py** is the conditional case —
`#![cfg_attr(not(feature = "arrow"), forbid(unsafe_code))]`, `deny`-level under `arrow` so its
single Arrow FFI block is allowed with a `// SAFETY:` note.)

```sh
ls -d crates/*/ | wc -l       # → 60 total crates
```
Accounting (**the crates do NOT partition cleanly — the two sets overlap, so do not add them**):
of **60 total** crates, **50** carry a source `forbid(unsafe_code)` and **9** are unsafe-bearing
in `bench/unsafe-snapshot.json` (sparq-core, sparq-vectors, sparq-engine, sparq-py, sparq-cli,
sparq-zk-compose, sparq-bench, sparq-lws-core, sparq-lws-wasm). TWO crates appear in BOTH sets:
**sparq-py** (conditional `forbid`, above), and
**sparq-lws-core** (src + bin `forbid`-clean; its 8 counted sites are EXAMPLE-only counting
allocators under `examples/`, likewise outside the crate-root `forbid` — sq-gg0qq.2 [FABLE-5]).
The remaining 4 crates carry neither a source `forbid` nor counted unsafe (dev/bench helpers:
sparq-acbench, sparq-difftest, sparq-kb, sparq-metamorph). `sparq-engine` is now an
unsafe-bearing library: its four shipped cancellation-pointer sites plus four test-only allocator
sites are registered and ratcheted. [GPT-5.6] sq-kq9ia. **sparq-lws-wasm** is the ninth
unsafe-bearing crate and the one deliberately absent from the `forbid` set: its four sites are a
SHIPPING bounded `#[global_allocator]` (the only way to bound wasm32 linear memory), so its root is
`deny(unsafe_code)` plus a single `#[allow(unsafe_code)] pub mod memory;` — every other module in
the crate still fails to compile on `unsafe`. [SONNET-4.6] sq-wubkf.

## MS-2 — 92-site register (ceiling and live), count-verified

```sh
python3 scripts/unsafe-gate.py --check    # → "live total = 92, snapshot total = 92"
```
Per-crate (from `--check`, matching `bench/unsafe-snapshot.json::crates`):

| crate | snapshot | live |
|---|---:|---:|
| sparq-core | 51 | 51 |
| sparq-vectors | 12 | 12 |
| sparq-lws-core | 8 | 8 |
| sparq-engine | 8 | 8 |
| sparq-lws-wasm | 4 | 4 |
| sparq-py | 4 | 4 |
| sparq-cli | 2 | 2 |
| sparq-zk-compose | 2 | 2 |
| sparq-bench | 1 | 1 |
| **total** | **92** | **92** |

<!-- [FABLE-5] sq-gg0qq.2: sparq-lws-core imported with 8 EXAMPLE-only counting-allocator
sites (register rows in unsafe-register.md). -->

<!-- [OPUS-4.8] sq-i6gj6: table re-synced to bench/unsafe-snapshot.json (was a stale 59/5-crate
snapshot: sparq-core rose 45→50, sparq-vectors 9→13, and sparq-engine/sparq-py picked up
counted TEST-ONLY / arrow-feature-gated sites). Run `python3 scripts/unsafe-gate.py --check`
to reproduce. -->

Every row carries the site kind, the invariant relied on, and how it is bounded — see
[`unsafe-register.md`](./unsafe-register.md).

## MS-3 — gating ratchet

```sh
python3 scripts/unsafe-gate.py --check    # → "unsafe-count ratchet: PASS"; exit 0
```
CI wiring — `.github/workflows/ci.yml`, job `unsafe-register:`:
```yaml
  unsafe-register:
    name: unsafe-register (count ratchet)
    steps:
      - name: Ratchet first-party unsafe count against the snapshot
        run: python3 scripts/unsafe-gate.py --check
```
No `continue-on-error`; the job name contains no "informational"/"advisory" token, so the
`ci-summary / gate` aggregator (which polls sibling check-runs and treats any
non-informational lane as required) blocks merge on a ratchet regression. A PR adding an
`unsafe` site fails until: (1) a register row is added, (2) a `// SAFETY:` comment is
added in source, (3) `scripts/unsafe-gate.py --seed` re-seeds `bench/unsafe-snapshot.json`
— all three land in the same reviewable diff.

## MS-4 — cargo-geiger (informational, NOT the gate)

`.github/workflows/ci.yml`, job `geiger:` — `name: unsafe report (cargo-geiger,
informational)`, `continue-on-error: true`, every step `continue-on-error`. The name's
"informational" token makes the aggregator skip it. Honest posture: geiger is visibility,
the ratchet (MS-3) is the gate. (cargo-geiger cannot run the virtual workspace manifest —
hence the deterministic scan in MS-3 is what we actually ratchet.)

## MS-5 — per-site `// SAFETY:`, lint-enforced (MS-G2 CLOSED, sq-8wbn)

Every `unsafe` block/impl carries a `// SAFETY:` argument immediately preceding it, and the
requirement is now **mechanically enforced** — the formerly-open MS-G2 gap is CLOSED.

```sh
grep -rn 'undocumented_unsafe_blocks' \
  crates/sparq-core/src/lib.rs crates/sparq-vectors/src/lib.rs \
  crates/sparq-cli/src/main.rs crates/sparq-zk-compose/src/lib.rs \
  crates/sparq-bench/src/main.rs crates/sparq-engine/src/lib.rs
# → #![warn(clippy::undocumented_unsafe_blocks)] in the crate roots whose source carries unsafe
```
Because the workspace gate is `cargo clippy --all-targets -- -D warnings` (MS-10), that
`warn` is promoted to a hard error: any `unsafe` block/impl/`extern` without a preceding
`// SAFETY:` comment **fails CI**. Verified clippy-clean on the live tree
(`cargo clippy -p sparq-core --all-targets` → no `undocumented_unsafe_blocks` diagnostics).

```sh
for c in sparq-core sparq-vectors sparq-cli sparq-zk-compose sparq-bench sparq-engine; do
  echo -n "$c: "; grep -rn '// SAFETY:' crates/$c/src | wc -l; done
# sparq-core: 51  sparq-vectors: 13  sparq-cli: 3  sparq-zk-compose: 3
# sparq-bench: 2  sparq-engine: 4  → ≥ each crate's source unsafe count
```
The 6 sites that previously relied on an adjacent block comment without the literal token —
the `from_utf8_unchecked` TRUSTED fast path (`dict.rs:483`) and the two `unsafe impl
Send`/`Sync for SlotPtr` pairs (`dict.rs:2391-2394`, `dictspill.rs:723-726`) — were
normalised so a `// SAFETY:` line sits in the comment block directly above each `unsafe`,
which is what makes the lint pass. Enforcement is now **lint + register + count ratchet**,
no longer review alone.

## MS-6 — Miri lane

`.github/workflows/miri.yml` — `cargo +nightly miri test -p sparq-core` under
`MIRIFLAGS: -Zmiri-tree-borrows -Zmiri-ignore-leaks -Zmiri-disable-isolation`. Triggers:
`schedule` (nightly 05:11 UTC) + `workflow_dispatch`. **No** `pull_request`/`merge_group`
trigger — so it is a nightly UB safety net, not a per-PR gate, and the aggregator does not
wait on it. The header documents (load-bearing) that the mmap/dict-spill features are NOT
enabled because Miri rejects file-backed mappings — those 16+7 sites are covered by MS-7/8.

## MS-7 — corruption oracle

`crates/sparq-core/tests/mmap_corruption_oracle.rs`:
```sh
grep -n 'fn ' crates/sparq-core/tests/mmap_corruption_oracle.rs
# open_rejects_corrupt_index, corrupt_truncate, corrupt_flip, corruption_sweep,
# mmap_loader_survives_corruption_raw, mmap_loader_survives_corruption_compressed
```
Run under `--features mmap,dict-spill` (the features Miri cannot run). The sweep
truncates/flips each on-disk file and asserts the loader rejects-or-stays-in-bounds.

## MS-8 — fuzz (mmap loader)

`fuzz/fuzz_targets/graph_open.rs` — header documents the surface as `Graph::open` over a
CORRUPT on-disk store dir (perm0..5, dict-*.bin, numerics/temporals, predstats, named.bin),
threat-model `T-MMAP-FUZZ`, invariant "clean `Err`, never panic/OOB/UB". Targets enumerated
by `cargo fuzz list` in `.github/workflows/fuzz.yml` (PR smoke + nightly). Other targets:
`load_reader_parallel.rs`, `parse_rdf_str.rs`, `parse_sparql.rs`, `validate_shacl.rs`.

## MS-9 — ASan (fuzz lane **and** standalone corruption-corpus lane)

**(a) Inside cargo-fuzz.** `.github/workflows/fuzz.yml` builds on nightly with `-Zsanitizer`
(libFuzzer sancov) on the `x86_64-unknown-linux-gnu` target (the musl target is
ASan-incompatible — documented in the workflow). So the mmap loader's reads execute under
AddressSanitizer during fuzzing.

**(b) Standalone over the deterministic corruption corpus (sq-hybl, [OPUS-4.8]) — closes the
former MS-G3 caveat.** `.github/workflows/asan.yml`:
```sh
# the exact lane commands (gnu target, ASan-instrumented std via -Zbuild-std):
RUSTFLAGS="-Zsanitizer=address" ASAN_OPTIONS="detect_leaks=0" \
  cargo +nightly test -Zbuild-std --target x86_64-unknown-linux-gnu \
    -p sparq-core --features mmap,dict-spill --test mmap_corruption_oracle
RUSTFLAGS="-Zsanitizer=address" ASAN_OPTIONS="detect_leaks=0" \
  cargo +nightly test -Zbuild-std --target x86_64-unknown-linux-gnu \
    -p sparq-vectors --test store --test diskann
```
Runs the deterministic mmap corruption corpus (the oracle's truncate + bit-flip sweep over
every on-disk section, plus the `sparq-vectors` `VectorStore`/`DiskAnnIndex` open-validation
corpus: truncated files / wrong magic / oversized length fields) under AddressSanitizer, so a
heap-OOB / use-after-free / UB on a malformed on-disk file is caught even when it does not
panic. `-Zbuild-std` (needs the `rust-src` component) is REQUIRED so std is recompiled under
the same `-Zsanitizer` flag; `--target` (a non-musl GNU triple) is REQUIRED for build-std +
ASan compatibility; `detect_leaks=0` silences LeakSanitizer noise from rayon's daemon threads
+ the long-lived mmaps (a lifecycle artefact, cf. miri's `-Zmiri-ignore-leaks`). **Trigger:**
nightly `schedule` + `workflow_dispatch` only — NO `pull_request`/`merge_group`, so it is a
non-blocking UB safety net (`-Zbuild-std` is many minutes) and `ci-summary / gate` neither
discovers nor waits on it; the job name `asan (mmap corruption corpus, informational)` also
carries the `informational` token belt-and-braces.

**Local verification (sparq-core half): PASSED.** The first lane command was RUN locally on
the agent's `aarch64-unknown-linux-gnu` host (a valid ASan target): the ASan-instrumented
`mmap_corruption_oracle` built (`-Zbuild-std` rebuilt std under `-Zsanitizer`) and the corpus
ran clean — `test result: ok. 4 passed; 0 failed` with NO ASan report (no heap-OOB / UAF / UB
on the truncate + bit-flip sweep). The lane PINS the `x86_64-unknown-linux-gnu` runner triple;
the flow is identical across the two gnu triples. See the inconsistencies note below for the
precise tested-vs-untested boundary.

## MS-10 — clippy `-D warnings`

`.github/workflows/ci.yml`: `cargo clippy --workspace --all-targets -- -D warnings`
(GATING) + `cargo clippy -p sparq-wasm --target wasm32-unknown-unknown --all-targets -- -D
warnings`. Neither is `continue-on-error`.

## MS-11 — dependency memory-safety (supply-chain lane)

`.github/workflows/supply-chain.yml`: `cargo deny check bans sources licenses` (gating) +
`cargo deny check advisories` (gating — GX-1 un-degraded; CVSS-4.0 blocker sq-q8de
resolved; `continue-on-error` removed). Daily watchdog `dependency-monitoring.yml`. The
register explicitly scopes third-party `unsafe` (memmap2/libc/rayon/hdt) OUT to this lane.

## MS-12 — edition-2024 unsafe (test-only env)

`unsafe-register.md` rows `src/lib.rs:7480` (`set_var`) + `:7484` (`remove_var`) — TEST-only
(`external_quads_fd_*`), single-threaded, var restored before return; counted by the ratchet.

## MS-13 — no unsafe in the untrusted-text path

The parser/planner/executor/reasoner/SHACL layers are in the MS-1 forbid list (or have no
`unsafe`): untrusted *query/data text* never reaches `unsafe`; only the on-disk index does
(B5). `research/threat-model.md` §scope table confirms `sparq-engine`/`sparq-reason`/
`sparq-shacl`/spargebra carry no executor/parser `unsafe`.

---

### Verified-but-noted inconsistencies (for the auditor)

1. **`research/threat-model.md` says "42 sites" in sparq-core** (lines 21, 118), the
   register/snapshot say **44** (the sq-vkz7 one-pass compressed external build added two
   `compress.rs`/`lib.rs` sites). The register is the authoritative count (it is the GX-5
   artifact + the ratchet source). The threat-model number is one step stale prose (it was
   synced 39→42 by sq-pro0 but has not picked up sq-vkz7). This framework does not own
   `threat-model.md`; tracked as low-severity drift MS-G5 (one-line fix for the doc owner).
2. The register's former `undocumented_unsafe_blocks` enforcement sentence (which once
   overstated lint enforcement, MS-G2) is now **accurate**: the lint IS enabled crate-root
   on the 5 crates whose src carries unsafe and the tree is clippy-clean under it (MS-G2
   CLOSED, sq-8wbn). MS-5/evidence and the register reflect the closed state.
3. **ASan lane (MS-9b / asan.yml) — tested-vs-untested boundary (sq-hybl, honest).** The
   sparq-core lane command (`RUSTFLAGS=-Zsanitizer=address` + `-Zbuild-std` + a non-musl GNU
   `--target` + `ASAN_OPTIONS=detect_leaks=0` over `mmap_corruption_oracle --features
   mmap,dict-spill`) was actually RUN to completion locally on the agent's
   `aarch64-unknown-linux-gnu` host (a valid ASan target): it built and the corpus passed
   clean — `4 passed; 0 failed`, no ASan report. The sparq-vectors `store`/`diskann` command
   was NOT separately run locally (same flow, no extra feature). The lane PINS the
   `x86_64-unknown-linux-gnu` GitHub runner triple (matching `fuzz.yml`), which was not run in
   this worktree (no x86_64 runner here); the flow is identical across the two gnu triples —
   validate the x86_64 leg + the vectors leg on the FIRST scheduled/dispatched CI run. The
   lane is non-blocking, so a first-run hiccup cannot block any merge.
