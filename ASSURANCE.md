<!-- [OPUS-4.8] sq-wir9k: assurance walkthrough (maintainer 2026-07-04 item 2). Every claim
     links its artifact; no perf numbers; ZK/MPC hedged per SECURITY.md (privacy-claims gate
     scans this file); conformance floors cite their single source since they ratchet up. -->

# Assurance — how to check that sparq works, in 15 minutes

sparq makes strong claims (W3C conformance, memory safety, honest documentation). This page is
the walkthrough for **verifying those claims yourself without reading the codebase**: the
5-minute health check first, then one short section per assurance layer — what it checks, where
to watch it run, what green means, and what it does **not** mean. Every claim links the artifact
that enforces it. Related front doors: [`SECURITY.md`](SECURITY.md) (threat model + disclosure)
and [`CONTRIBUTING.md`](CONTRIBUTING.md) (the contributor-facing gate).

## The 5-minute version

**One check summarizes tree health: `ci-summary / gate`.** It is the single required
branch-protection status on `main` ([`docs/branch-protection.md`](docs/branch-protection.md)):
it polls **every other check-run on the same commit** — build + tests, `clippy -D warnings`,
the conformance / coverage / unsafe-count ratchets, the opt-in feature matrix, supply-chain,
and the docs-honesty gates — and passes only when none failed. (**CodeQL is NOT among them** —
it has been disabled since 2026-07-18; see §11.) So:

1. Open any merged PR (or the latest commit on `main`) and look at its **checks list** —
   green `ci-summary / gate` ≈ everything below in this document that gates was green.
   Definition: [`.github/workflows/ci-summary.yml`](.github/workflows/ci-summary.yml), logic in
   [`scripts/ci_summary_gate.py`](scripts/ci_summary_gate.py) (itself unit-tested in CI by
   [`scripts/tests/test_ci_summary_gate.py`](scripts/tests/test_ci_summary_gate.py)).
2. For the *"does it implement the specs?"* question, open the **conformance scoreboard**: the
   job summary of the conformance job in [`.github/workflows/ci.yml`](.github/workflows/ci.yml)
   renders every suite with its ratchet floor. Locally:
   `cargo run --release -p sparq-conformance --bin sparq-conformance-scoreboard`.
3. Locally, the core gate is just: `cargo build --workspace && cargo test --workspace` plus
   `cargo clippy --workspace --exclude sparq-py --all-targets -- -D warnings`.

Two honest qualifiers, up front: some heavyweight lanes (Kani proofs, Miri, ASan, long fuzz,
the ZK toolchain suite, mutation testing) run **nightly, not per-PR**, so a green `gate` shows
they were green on their *last scheduled run*, not on this exact commit; and the
`sparq-zk*` / `sparq-mpc` crates are **research scaffolds with no security guarantee yet** —
see [What green does *not* mean](#what-green-does-not-mean).

## The layers at a glance

| Layer | Question it answers | Blocks a PR? |
|---|---|---|
| [W3C conformance ratchets](#1-w3c-conformance-ratchets) | Does it implement the specs? | yes |
| [Independent oracles](#2-independent-oracles--differential-testing) | Would an independent implementation agree? | partly (nightly lanes) |
| [Metamorphic self-checks](#3-metamorphic-self-checks-tlp--norec) | Is the engine internally consistent? | yes (self-tests) |
| [Fuzzing](#4-fuzzing) | Does hostile input crash it? | yes (smoke tier) |
| [Memory safety](#5-memory-safety-the-unsafe-register-miri-asan) | Is the `unsafe` surface bounded + justified? | yes (count ratchet) |
| [Bounded proofs (Kani)](#6-bounded-mechanized-proofs-kani) | Is any input in a bounded domain missed? | no (nightly, informational) |
| [Test-quality ratchets](#7-test-quality-ratchets-coverage-floors--mutation-ceilings) | Do the tests actually assert? | coverage yes / mutation advisory |
| [Both-feature-state builds](#8-both-feature-state-builds) | Does every opt-in feature build, on and off? | yes |
| [End-to-end personas](#9-end-to-end-product-journeys-playwright-personas) | Does the product work for a user? | yes |
| [Docs honesty gates](#10-docs-honesty-gates) | Do the docs overclaim? | yes |
| [Supply chain + SAST](#11-supply-chain--static-analysis) | Are the dependencies and the build attested? | yes — **but SAST is disabled**, see §11 |

## 1. W3C conformance ratchets

sparq's spec-conformance is not a README claim — it is a set of **floors that only ever go
up**, enforced per commit. The [`sparq-conformance`](crates/sparq-conformance) harness runs the
official W3C suites (fetched into the git-ignored `tests/w3c/` tree by the conformance jobs) —
SPARQL 1.0/1.1/1.2 query, update and syntax; the inference regimes (RDF-MT, OWL 2 RL, N3,
SPARQL entailment); SHACL core and SHACL-SPARQL; OGC GeoSPARQL; Solid WAC/ACP; JSON-LD — and CI
fails if any suite's pass count drops below its committed floor. The floors live in **one
registry**, [`crates/sparq-conformance/src/scoreboard.rs`](crates/sparq-conformance/src/scoreboard.rs)
(at the time of writing: SPARQL ≥ 1229, inference ≥ 1967, SHACL core ≥ 98 — the registry is the
source of truth as they ratchet up), and a guard test
([`tests/scoreboard_floors.rs`](crates/sparq-conformance/tests/scoreboard_floors.rs)) asserts
the central board matches the constants each runner actually enforces, so it cannot silently
drift from CI. The OWL 2 **Direct-Semantics** arm is pinned even harder:
[`tests/dl_suite.rs`](crates/sparq-conformance/tests/dl_suite.rs) asserts **exact equality**
(94 profile-lane passes, 186 direct consistency/entailment passes) rather than `>=`, because
its invariant is *"an abstention is never counted as a pass"* — a floor cannot catch that.

- **See it run:** the conformance jobs in [`ci.yml`](.github/workflows/ci.yml); the scoreboard
  renders into the job summary. The SPARQL report (`conformance-report.md`) is regenerated on
  every run and published as a CI artifact (it is deliberately git-ignored); the inference
  report is committed at [`inference-conformance-report.md`](inference-conformance-report.md).
- **Green means:** every suite ≥ its floor, and no floor was lowered ("never lower" rule —
  [`CONTRIBUTING.md`](CONTRIBUTING.md)). Failures are 0; the gap to the suite totals is
  **documented divergences** (each with a spec-justified rationale in the report) or
  out-of-scope entries — never silent skips.
- **Limits:** the HTTP-protocol-shaped suites (SPARQL protocol, Graph Store, service) are only
  partially in scope — the report's footer lists what is *not yet run*. Some scoreboard rows
  (full-text, RSP-QL, OWL 2 QL/EL) are honestly labelled **measured extension suites**, not
  W3C-conformance claims; the DL arm is a scoped fragment, **not** full OWL 2 DL
  ([design record](research/owl2-direct-semantics-scoping.md)).

## 2. Independent oracles + differential testing

Where an independent implementation exists, sparq is checked **against it**, not against
itself. (a) *SHACL*: a nightly differential fuzzer
([`shacl-diff-fuzz.yml`](.github/workflows/shacl-diff-fuzz.yml), harness
[`crates/sparq-shacl/tests/diff_fuzz.rs`](crates/sparq-shacl/tests/diff_fuzz.rs)) generates
random shapes + data and validates through sparq **and** independent reference engines
(pySHACL, Jena SHACL, Zazuko); any disagreement is a candidate bug. (b) *Solid WAC/ACP*: the
scoreboard carries a differential-oracle row whose floor is **zero divergences**. (c) *XPath
functions inside the ZK toolchain*: the Noir circuits are tested against the W3C **qt3** suite,
but every expected value is **re-derived by an independent oracle** — the `scripts/generate_tests.py`
generator in the [`sparq-org/noir_XPath`](https://github.com/sparq-org/noir_XPath) face repo
(externalized from the in-tree `zk/xpath/` tree, bead sq-5reoy / #1599) evaluates each
expression with the Python `elementpath` engine and (for floats) pins the IEEE-754 **bit
pattern**, so circuits are checked bit-exactly against a second implementation, not against
hand-copied strings. **PROOF M1 (bead sq-3x7dl.14.2)** adds a *second, sparq-side* oracle for the
same circuits: [`zk/xpath/differential`](zk/xpath/differential) generates a Noir test file whose
expected values are read back from **sparq's own Rust SPARQL/XSD evaluator** (bar a labelled
handful — see *Limits*), over a
unicode-aware corpus covering precisely the edges qt3 lacks (non-exact `op:numeric-divide`,
`fn:substring` with `start < 1`, mixed int/float comparisons outside the i8 range, NUL-padded and
multibyte strings, pre-1970 `xs:dateTime`) — the cases beads `sq-3x7dl.4`–`.7` fixed.

- **See it run:** [`shacl-diff-fuzz.yml`](.github/workflows/shacl-diff-fuzz.yml) (nightly);
  [`xpath-differential.yml`](.github/workflows/xpath-differential.yml) (the M1 oracle, plus a
  fault-injection self-test that fails the lane if a deliberately corrupted expected value still
  passes `nargo test`); the `nargo` unit-test lane now lives in the
  [`sparq-org/noir_XPath`](https://github.com/sparq-org/noir_XPath)
  face repo's CI (`scripts/run_real_tests.sh`), which dynamically detects the generated real test
  packages under its `test_packages/` and fails closed if it finds none. (Until sq-5reoy this ran
  in sparq's `zk-toolchain.yml`; the harness moved upstream with the tree.)
- **Green means:** no cross-engine disagreement; every real qt3-derived package passes. The face
  repo's `KNOWN_FAILING` skip-list mechanism (the single source post-split — the former sparq
  `zk-toolchain.yml` array is retired) exists for beaded latent failures and is **currently
  empty** — no real package is being skipped.
- **Limits:** the sparq lane is nightly (see qualifier above); oracle scope is bounded by what
  the reference engines implement. The M1 xpath oracle is **verification, not proof**: its TCB is
  the (itself unaudited) sparq Rust XSD evaluator, the sampled corpus, and the trusted
  Noir→ACIR→Barretenberg lowering — `nargo test` exercises witness generation only. Two live
  divergences where *sparq's own evaluator* is wrong against XPath F&O are recorded in the
  generated file's header. Those rows are still asserted **live**, but against the F&O value and
  labelled `SPEC-REFERENCE` — read them as *circuit vs the spec*, not *circuit vs sparq* — so the
  edges stay executable and a `noir_XPath` regression on one fails the lane. Unit tests pin that
  no assertion is ever emitted commented out, and self-expiring tests retire the special-casing
  the day the engine is fixed.

## 3. Metamorphic self-checks (TLP / NoREC)

The [`sparq-metamorph`](crates/sparq-metamorph) crate hunts wrong-*results* (not crashes)
without needing any oracle: **Ternary Logic Partitioning** splits a query on a predicate `c`
into `FILTER(c)` / `FILTER(!c)` / an error-branch reified via `COALESCE`/`IF`, and asserts the
three result multisets union back to the unpartitioned query
([`src/tlp.rs`](crates/sparq-metamorph/src/tlp.rs)); **NoREC** rephrases the predicate into
projection position as a cardinality cross-check. The suite proves its own teeth: a
deliberately injected wrong-result mutant (a FILTER that silently drops a row) **must** be
flagged by TLP, NoREC and the differential oracle, or the tests fail
([`tests/oracle_self_tests.rs`](crates/sparq-metamorph/tests/oracle_self_tests.rs)).

- **See it run:** the self-tests run in the ordinary workspace test lane in
  [`ci.yml`](.github/workflows/ci.yml) — they gate every PR.
- **Green means:** the oracles hold on the real in-process engine, every engine error is
  fail-closed (`EngineFailure`, never a silent pass), and the injected mutant is caught.
- **Limits:** the crate is the *instrument*; long-running bug-hunting campaigns against live
  endpoints run off-CI, and the oracles have documented scope preconditions (deterministic
  predicates, no `EXISTS`, no cross-engine blank-node projection).

## 4. Fuzzing

Hostile bytes must produce a clean `Err`, never a panic, OOM or undefined behaviour. Thirteen
libFuzzer targets under [`fuzz/fuzz_targets/`](fuzz/fuzz_targets/) cover the RDF text parsers,
the parallel N-Triples reader, the SPARQL parser, SHACL parsing/traversal, and — load-bearing —
the **mmap on-disk store loader** (`graph_open`), which is exactly the surface Miri cannot
reach (§5).

- **See it run:** [`fuzz.yml`](.github/workflows/fuzz.yml) — a short smoke budget per target
  **gates every PR**; longer budgets run on push and nightly. Locally:
  `cargo +nightly fuzz run <target>` from `fuzz/`.
- **Green means:** no crash/leak/timeout in the budget; on failure CI uploads the reproducer.
- **Limits:** fuzzing is execution-based — it proves the inputs actually reached, in a bounded
  wall-clock budget, and gives no coverage guarantee. That gap is the reason the Kani lane (§6)
  exists.

## 5. Memory safety: the unsafe register, Miri, ASan

Most of the workspace is `#![forbid(unsafe_code)]`; the `unsafe` that remains is concentrated
in a handful of crates (mmap loaders, the zero-copy dictionary, SIMD, FFI) and is **enumerated,
justified and ratcheted**. Every site needs an inline `// SAFETY:` comment (enforced by clippy)
plus a row in the per-site justification register
([`compliance/memsafety/unsafe-register.md`](compliance/memsafety/unsafe-register.md)) stating
the invariant, why it is sound, and how it is tested/bounded. A **count ratchet**
([`scripts/unsafe-gate.py`](scripts/unsafe-gate.py) against
[`bench/unsafe-snapshot.json`](bench/unsafe-snapshot.json)) fails any PR that adds an `unsafe`
site without doing all of that. The pure-Rust unsafe paths run under **Miri** (Tree Borrows);
the mmap-backed paths Miri structurally cannot execute are covered instead by a deterministic
corruption oracle, the `graph_open` fuzz target, and an **AddressSanitizer** lane.

- **See it run:** the `unsafe-register (count ratchet)` job in
  [`ci.yml`](.github/workflows/ci.yml) (gating, every PR); [`miri.yml`](.github/workflows/miri.yml)
  and [`asan.yml`](.github/workflows/asan.yml) (nightly). Locally:
  `python3 scripts/unsafe-gate.py --check`.
- **Green means:** no crate's unsafe count rose above its snapshot; no UB detected on the
  Miri-reachable surface; ASan-clean on the mmap surface.
- **Limits:** Miri cannot reach file-backed mmap (documented in
  [`miri.yml`](.github/workflows/miri.yml) itself); third-party `unsafe` (memmap2, rayon, …)
  is out of the register's scope and governed by the supply-chain layer (§11). The threat-model
  boundary for all of this is B5 in [`research/threat-model.md`](research/threat-model.md).

## 6. Bounded mechanized proofs (Kani)

For the highest-risk seam — parsing an **attacker-supplied on-disk index/vector file** — tests
and fuzzing are complemented by [Kani](https://model-checking.github.io/kani/) bounded model
checking: for *every* byte buffer up to a small bound, the format validators return cleanly and
never panic or read out of bounds, plus pure id-encoding invariants. The harnesses live behind
`#[cfg(kani)]` in [`crates/sparq-core/src/dict.rs`](crates/sparq-core/src/dict.rs) and
[`crates/sparq-vectors/src/store.rs`](crates/sparq-vectors/src/store.rs); they deliberately
target the mmap-*free* seam that the real mmap path delegates to.

- **See it run:** [`kani.yml`](.github/workflows/kani.yml), nightly + on demand. Locally:
  `cargo kani -p <crate> --harness <name>`.
- **Green means:** every harness verified exhaustively **within its bound** — the "no input in
  the bounded domain was missed" guarantee a fuzz corpus cannot give.
- **Limits (important):** these are **bounded** proofs over small buffers, informational
  (nightly, not a PR gate), and Kani cannot model mmap/file-I/O/FFI. At the time of writing the
  two slowest harnesses exceed the lane's time budget and are honestly reported as TIMEOUT
  (bead `sq-og8u8`). Kani complements, never replaces, the fuzz/oracle/ASan coverage.

## 7. Test-quality ratchets: coverage floors + mutation ceilings

Two ratchets keep the *tests themselves* honest. **Coverage floors**: every crate's measured
line coverage must stay at or above its committed floor
([`bench/coverage-floor.json`](bench/coverage-floor.json)), floors may only rise, and a
no-compile monotonicity check plus a per-crate test-*presence* check
([`scripts/coverage-gate.py`](scripts/coverage-gate.py),
[`scripts/coverage-presence.py`](scripts/coverage-presence.py)) run alongside the measured
shards. **Mutation ceilings**: nightly `cargo-mutants` runs enforce a per-crate
*surviving-mutant ceiling* that only ever falls ([`scripts/mutants-gate.py`](scripts/mutants-gate.py)
against [`bench/mutants-baseline.json`](bench/mutants-baseline.json)) — catching tests that
execute a line but never assert on it.

- **See it run:** the coverage jobs in [`ci.yml`](.github/workflows/ci.yml) (gating); the
  mutation job runs nightly. Locally: `bash scripts/coverage.sh` then
  `python3 scripts/coverage-gate.py --check-robust`.
- **Green means:** no crate below floor, no floor lowered vs `main`, presence counts met; no
  crate above its mutant ceiling.
- **Limits:** the mutation ratchet is **advisory** while its baseline seeds across the
  workspace (it is **declared** in [`.github/advisory-registry.json`](.github/advisory-registry.json), which is
  the only thing that makes a check non-gating — naming alone does nothing since #3773); a few driver-style crates carry floor 0
  by design but remain presence-gated.

## 8. Both-feature-state builds

Every capability outside the core is an **opt-in** cargo feature/crate, so it must be proven
in *both* states. The ON side: [`feature-matrix.yml`](.github/workflows/feature-matrix.yml)
builds + tests + clippy-gates each opt-in feature set (assembled from
[`.github/feature-matrix.d/`](.github/feature-matrix.d/)), plus a
`--no-default-features` server leg. The OFF side:
[`vectorized-feature-off.yml`](.github/workflows/vectorized-feature-off.yml) asserts the
default build is genuinely feature-free — down to a **byte-identical** wasm artifact check.

- **See it run:** the per-leg `opt-in <name>` check-runs on any PR touching those crates.
- **Green means:** every feature builds, tests and lints in both states; the core stayed lean.
- **Limits:** the main workspace test archive only carries a fixed feature set — coverage of
  any *other* opt-in feature is strictly the matrix's job (a suite not in the matrix would run
  silently-zero in the main lane; that is why the matrix is a hard gate).

## 9. End-to-end product journeys (Playwright personas)

Above the engine, CI drives the **product** the way a user would, under two personas over the
same static export: a *desktop* persona (Tauri IPC mocked in, real in-tab WASM engine) and a
*browser* persona (no Tauri at all — the pure-web `/app` paths, e.g.
[`web-persona.web.spec.ts`](gui/e2e-playwright/specs/web-persona.web.spec.ts)). Journeys cover
querying, file/drag-drop import, SHACL upload, N3 inference toggles, full-text, graph view,
streaming, export, workspaces and the keyboard spine, under a strict determinism doctrine: zero
retries (a flake is a broken test), serial execution, external network blocked, no sleeps.

- **See it run:** [`gui.yml`](.github/workflows/gui.yml) (mocked-IPC lane over
  [`gui/e2e-playwright/`](gui/e2e-playwright/)); the marketing-site e2e in
  [`site-e2e.yml`](.github/workflows/site-e2e.yml) over [`site/e2e/`](site/e2e/).
- **Green means:** the shipped journeys work end-to-end in both personas, and honest
  capability notes are asserted where a persona genuinely cannot do something (e.g. the web
  persona shows — and the test *requires* — the "HDT/compressed import unavailable" note).
- **Limits:** presence-style assertions (no timing/row-count pins) by design; only Tauri IPC
  is stubbed, everything else is real.

## 10. Docs honesty gates

The documentation is linted for **overclaiming**, not just typos. The flagship is the
**privacy-claims gate** ([`scripts/check-privacy-claims.sh`](scripts/check-privacy-claims.sh)):
it greps the outward claim surface (root docs — including this file — skills, site copy,
papers) for unqualified ZK/MPC privacy-or-soundness claims stated as settled fact, and fails
the build on a hit unless the line carries a hedge/negation or a justified allow-marker; it
ships a both-direction self-test
([`scripts/tests/test_privacy_claims.sh`](scripts/tests/test_privacy_claims.sh)). Alongside it:
**no-perf-numbers** ([`scripts/check-no-perf-numbers.py`](scripts/check-no-perf-numbers.py) —
measured figures live only in the [benchmarks dashboard](https://sparq.jeswr.org/dev/bench),
never in markdown), terminology, internal-links (lychee), typos, markdownlint, README-template,
and "a new public API/config key must be documented in the same diff" gates.

- **See it run:** [`docs-quality.yml`](.github/workflows/docs-quality.yml) and
  [`flow-on-gates.yml`](.github/workflows/flow-on-gates.yml), on every PR. Locally:
  `bash scripts/check-privacy-claims.sh`.
- **Green means:** no unqualified crypto claim, no baked-in perf number, no dead internal
  link on the gated surface.
- **Limits:** it is a phrase/pattern gate, not semantic review — it raises the floor on
  honesty; maintainer + reviewer judgment still sit above it.

## 11. Supply chain + static analysis

Dependencies and builds are attested, not trusted. Per PR (gating, in
[`supply-chain.yml`](.github/workflows/supply-chain.yml)): **cargo-deny** (advisories, bans,
licenses, sources — policy in [`deny.toml`](deny.toml), where any tolerated advisory carries a
written justification), **cargo-vet** (per-dependency audit attestations — an unaudited crate
cannot enter silently), a CycloneDX **SBOM**, and a VEX↔deny.toml drift check. Per release
([`release.yml`](.github/workflows/release.yml)): SBOM + VEX per artifact and **SLSA build
provenance** attestations (verifiable with `gh attestation verify`). Continuously: a daily
advisory watchdog ([`dependency-monitoring.yml`](.github/workflows/dependency-monitoring.yml)),
and the public **OpenSSF Scorecard** ([`scorecard.yml`](.github/workflows/scorecard.yml)).

> **CodeQL SAST is currently DISABLED — this section previously claimed otherwise.**
>
> It read: *"Continuously: **CodeQL** SAST ([`codeql.yml`](.github/workflows/codeql.yml), kept at
> zero open alerts)"*. **Both halves of that were false.** The workflow has been
> `disabled_manually` since **2026-07-18** (a deliberate, recorded decision taken to cut merge
> latency), its last run was 2026-07-18T14:01Z, and code scanning currently reports **35 open
> alerts, all `critical`** — every one `rust/hard-coded-cryptographic-value`, the oldest opened
> **2026-07-11**, in two files: `crates/sparq-lws-core/tests/pop_dpop_sk.rs` (25) and
> `crates/sparq-lws-core/src/store/sparql.rs` (10).
>
> **All 35 have since been triaged (sparq-org/sparq#4615): none is exploitable.** Every one is a
> false positive of a single query-model defect — the query models its sink by *parameter name*
> while classifying test code by *file path*, and Rust colocates unit tests inside source files.
> Both clusters sit inside `#[cfg(test)] mod tests` (the `sparql.rs` module spans lines 740–1370,
> i.e. to EOF), and in most cases the flagged value is not a cryptographic nonce at all. Production
> never hard-codes these: the marker is generated per operation and session keys are drawn from the
> OS CSPRNG.
>
> **"Triaged" is not "covered."** These 35 being benign says nothing about what an enabled scanner
> would find; SAST remains off. The durable posture is under decision in
> sparq-org/sparq#4620, and one real (latent, unexploited) finding surfaced during triage is
> sparq-org/sparq#4621.
>
> The correction is quoted rather than deleted because this document is what a reader would rely
> on: a stated control that is switched off is worse than an acknowledged gap, since it stops
> anyone looking.
>
> **There is currently no compensating SAST control.** `clippy -D warnings`, the unsafe-count
> ratchet, `cargo-deny`/`cargo-vet` and the fuzz lanes all remain green and are genuine, but none
> of them is a substitute for taint/crypto-misuse analysis. The durable decision (re-enable
> advisory-only, re-enable on a schedule, or accept and document no SAST) is still open in
> sparq-org/sparq#4620.

- **Green means:** license/advisory/source policy holds, every dependency carries an audit
  attestation, and released artifacts have verifiable provenance.
- **Limits:** provenance attests *who built it*, it is not code-signing (desktop GUI bundles
  are currently unsigned); tolerated advisories are listed, justified and time-bounded in
  [`deny.toml`](deny.toml).

## What green does *not* mean

- **ZK / MPC are research scaffolds — no security guarantee yet.** The `sparq-zk*` crates model
  proving query results over committed data, but the v1 verifier has **not** been externally
  audited: independent cryptographer sign-off (bead `sq-qhy4`) is required before any production
  ZK claim, and until then a "verified" result must not be presented as a guarantee to a relying
  party. `sparq-mpc` is honest-majority/semi-honest only and **not** maliciously secure. <!-- privacy-claims-allow: negative caveat ("not maliciously secure"), same as README.md; sq-wir9k -->
  The authoritative wording is [`SECURITY.md`](SECURITY.md); §10's gate keeps every doc
  consistent with it.
- **Kani proofs are bounded** (§6): exhaustive only up to a small input bound, on the
  mmap-free seam, in an informational nightly lane.
- **Nightly ≠ per-commit:** Kani, Miri, ASan, long-budget fuzz, the ZK toolchain suite,
  SHACL differential fuzz and mutation testing run on schedule; a PR's green `gate` reflects
  their last scheduled run.
- **Conformance floors are floors:** the documented-divergence and not-yet-run lists in the
  reports are part of the claim — read them, they are deliberately not hidden.
- **This is an experimental, pre-1.0 engine** ([`README.md`](README.md)): the assurance estate
  above is how correctness is *kept*, not a maturity certification.

## Reproduce it locally

| Claim | Command (repo root) |
|---|---|
| Everything builds + tests pass | `cargo build --workspace && cargo test --workspace` |
| Lint-clean at `-D warnings` | `cargo clippy --workspace --exclude sparq-py --all-targets -- -D warnings` |
| SPARQL conformance + scoreboard | `cargo run --release -p sparq-conformance` · `--bin sparq-conformance-scoreboard` |
| Unsafe surface unchanged | `python3 scripts/unsafe-gate.py --check` |
| Coverage floors hold | `bash scripts/coverage.sh` then `python3 scripts/coverage-gate.py --check-robust` |
| No overclaiming docs | `bash scripts/check-privacy-claims.sh` |
| Parsers survive hostile input | `cargo +nightly fuzz run parse_rdf_str` (from `fuzz/`) |
| Bounded proofs | `cargo kani -p sparq-vectors --harness open_from_bytes_never_panics` |
| Product journeys | `npx playwright test` (from `gui/e2e-playwright/`) |

Questions this page doesn't answer are welcome as issues; security concerns go through
[`SECURITY.md`](SECURITY.md), never a public issue.
