<!-- [OPUS-4.8] Governance: contributor guide (bead sq-rau7). Intentionally thin — AGENTS.md is the source of truth. -->
# Contributing to sparq

Thanks for your interest in `sparq`! This file is deliberately thin: the **single source
of truth for how to work on this repo is [`AGENTS.md`](./AGENTS.md)**. Read it first. It
covers what sparq is, how to use it (the `skills/` tree), and — for contributors — the
build/test/lint gate, the conformance ratchets, merge discipline, and our repository
conventions. This file only adds the human-facing bits.

## Before you start

- Read [`AGENTS.md`](./AGENTS.md), especially **"Working on this repo"** (the build/test/
  lint gate and merge discipline) and the **"Post-batch re-evaluation checklist"** (which
  gates a given change must re-run).
- `sparq` is **experimental and pre-1.0; the API is unstable.** Expect churn.

## The gate — what must be green to land a change

The full definition lives in `AGENTS.md`; in short, a change lands only when **all** of
these pass (CI enforces them — see [`docs/branch-protection.md`](./docs/branch-protection.md)):

- `cargo build --workspace` and `cargo test --workspace`.
- `cargo clippy --workspace --exclude sparq-py --all-targets -- -D warnings`
  (run over the **full workspace** — feature unification surfaces lints a single-crate
  check misses). `cargo fmt --all --check` also runs in CI but is **informational, not a
  gate** — see [The gates a change must pass](#the-gates-a-change-must-pass) below.
- The W3C SPARQL / SHACL / inference **conformance ratchets** and the performance/
  coverage ratchets, all green.
- If your change touches Cargo dependencies: `cargo audit` + `cargo deny check` (and the
  SBOM) once those gates are in place.

Use the [pull-request template](./.github/PULL_REQUEST_TEMPLATE.md) — its checklist is
tied directly to the post-batch re-evaluation table in `AGENTS.md`.

### The conformance-ratchet "never lower" rule

The conformance floors are **ratchets: they only ever go UP.** They are reported by
[`sparq-conformance`](crates/sparq-conformance) — the SPARQL report (`conformance-report.md`)
is git-ignored and regenerated locally by `cargo run -p sparq-conformance` (CI re-runs it and
publishes it as an artifact), while the inference report is committed at
[`inference-conformance-report.md`](inference-conformance-report.md). The SHACL floors,
`bench/perf-baseline.json`, and the coverage floors ratchet the same way. Never lower a ratchet
to make a change pass — fix the regression instead. If a test newly diverges for a *documented,
spec-justified* reason, record the rationale in the report alongside the divergence
(don't just drop the count). Raising a floor when coverage genuinely improves is
encouraged.

<!-- [OPUS-4.8] sq-8qzz: contributor note guarding new golden/snapshot tests against the
     thread-count-dependent dict-id order. No current test is at risk (the audit verified
     this) — this is a forward-looking convention, not a fix for a present bug. -->
### A golden over serialised RDF or an unordered/tied result must canonicalise or pin threads

`sparq-core` assigns dictionary ids in a **thread-count-dependent** order (the parallel
sharded dict merge, `default_shards = (rayon::current_num_threads()*2).clamp(4,64)`), and
that id order surfaces — spec-permittedly — in several **observable but unspecified** places:
RDF serialisation row order (Turtle / TriG / N-Quads / N-Triples / JSON-LD), `SELECT` row
order without `ORDER BY`, `CONSTRUCT` / `DESCRIBE` triple order, and `ORDER BY`-tie order.
None is a correctness bug (the SPARQL / RDF specs leave all of them unspecified), and no
golden/snapshot file in the repo pins any of them today — the conformance harness compares
unordered results as a **bag (multiset)** and existing snapshot/differential tests sort the
rendered rows by term before asserting.

So if you add a test that pins one of those outputs **as an exact string / file**, make it
robust to thread count one of two ways, or it will be host-flaky exactly like the #691
`label_propagation` test was:

- **Canonicalise** — sort the rendered rows (or compare as a set/multiset of term strings)
  before asserting. This is the default; it matches every existing test.
- **Pin threads** — set `RAYON_NUM_THREADS=1` for that test if you genuinely must assert an
  exact byte sequence (rare; prefer canonicalising). CI does not pin threads on the
  conformance / bulk shards, and nextest runs shards on differently-sized runners.

Background and the full per-surface analysis: `research/dict-id-order-determinism-audit.md`.

### Every `unsafe` block needs a `// SAFETY:` comment and a register row

`sparq`'s `unsafe` is confined to five crates (mmap loaders, the zero-copy dictionary,
SIMD/FFI); the other twenty-six are `#![forbid(unsafe_code)]`. If you add an `unsafe` block,
`fn`, `impl`, `trait`, or `extern`, you **must** in the same change:

1. Put a **`// SAFETY:`** comment immediately above it stating the invariant it relies on
   and why that invariant holds (clippy's `undocumented_unsafe_blocks` is the local check).
2. Add a row to [`compliance/memsafety/unsafe-register.md`](./compliance/memsafety/unsafe-register.md)
   (the GX-5 justification register) — file:line, the invariant, why it is sound, how it is
   tested/bounded. If you cannot give a sound argument, mark the row `NEEDS-REVIEW` and open
   a bead rather than fabricating one.
3. Re-seed the count snapshot: `scripts/unsafe-gate.py --seed`.

The **`unsafe-register (count ratchet)`** CI lane (a required gating check) fails the PR
until all three are done — it counts first-party `unsafe` per crate and rejects any rise
above `bench/unsafe-snapshot.json`. Run `scripts/unsafe-gate.py --check` locally first.

### Changing a public API → update the matching skill

If you change any **public API** (a `pub` item, a CLI flag, an HTTP route, or a Python/JS
binding), update the corresponding `skills/<surface>/SKILL.md` **in the same change**.
This is a required, enforced convention — see the MAINTENANCE RULE in `AGENTS.md`.

## Secure coding

<!-- [OPUS-4.8] Secure-coding standard (bead sq-toze.7 / gap GX-6). Maps the cert
     expectations (SSDF PW, ASVS V1/V5) onto sparq's ACTUAL gates. Points to AGENTS.md
     and the workflows rather than duplicating them. -->

`sparq` processes untrusted input (RDF text, SPARQL queries, HTTP requests). This section
is the contributor-facing secure-coding standard; it maps to the secure-SDLC (NIST SSDF)
and ASVS expectations and is grounded in the gates CI already enforces. The detail of each
gate lives in [`AGENTS.md`](./AGENTS.md) and the workflows under
[`.github/workflows/`](./.github/workflows/) — this is the index, not a duplicate.

### `unsafe` policy

- **Prefer safe Rust.** Do not add `unsafe` to a crate that has none. The `unsafe` we keep
  is concentrated in `sparq-core` (mmap, the zero-copy dictionary, SIMD).
- **Justify every `unsafe` site.** A new `unsafe` block needs an inline `// SAFETY:` comment
  stating the invariant you rely on and why it holds. Adding `unsafe` is visible in CI: the
  `unsafe report (cargo-geiger, informational)` step in [`ci.yml`](./.github/workflows/ci.yml)
  surfaces the count in the run summary. (A consolidated per-site justification register is a
  tracked gap, GX-5, not yet a file in the repo — until it lands, the inline `// SAFETY:`
  comment at the site is the register of record.)
- **Run it under the UB lanes.** Pure-Rust `unsafe` in `sparq-core` is exercised by the
  `miri` lane ([`miri.yml`](./.github/workflows/miri.yml)) and the parser/loader fuzzers
  ([`fuzz.yml`](./.github/workflows/fuzz.yml)). The mmap sites that Miri cannot reach are
  covered by an oracle + fuzz matrix — extend that coverage when you touch them.

### Input validation and the hardened surfaces

- The parser, query engine, and HTTP server (`sparq-core`, `sparq-engine`, `sparq-server`)
  are the surfaces most likely to see untrusted input. Validate at the boundary, prefer
  total functions, and **never let untrusted input drive an `unwrap`/`expect`/`panic!` or an
  unbounded allocation** — a reachable panic or OOM is an in-scope DoS bug (see
  [`SECURITY.md`](./SECURITY.md)).
- Keep error and log output clean: do not leak RDF/SPARQL content, internal paths, or stack
  detail into HTTP error responses or logs (ASVS V7).
- The `sparq-zk*` and `sparq-mpc` crates are research scaffolds with **no security
  guarantee** — do not present their output as a cryptographic assurance. See `SECURITY.md`.

### Supply chain

- If your change touches `Cargo.toml`/`Cargo.lock`, the supply-chain gate applies:
  `cargo deny check bans sources licenses` gates on every PR
  ([`supply-chain.yml`](./.github/workflows/supply-chain.yml)), the daily advisory watchdog
  ([`dependency-monitoring.yml`](./.github/workflows/dependency-monitoring.yml)) flags new
  RustSec advisories, and a CycloneDX SBOM is generated per build. The policy lives in
  [`deny.toml`](./deny.toml).
- Add the **smallest** dependency that does the job, from a maintained source, and record any
  tolerated advisory with a justification in `deny.toml` (each `ignore` entry carries a
  reason and a tracking bead).

### The gates a change must pass

A contribution lands only when the gate in **"The gate"** above is green: `cargo build` /
`cargo test`, `cargo clippy --workspace --all-targets -- -D warnings` (a hard gate),
and the W3C conformance + performance/coverage ratchets (never lowered). `cargo fmt --all
--check` also runs in CI but is currently **informational** (`continue-on-error: true`),
to be flipped to a hard gate once the one-time `cargo fmt --all` reformat lands.

Until that reformat lands, `cargo fmt --all --check` **fails on files your change did not
touch** — the workspace has never been formatted, so the diff it reports is pre-existing, not
your regression. Format the code you touched (matching the surrounding committed style) and
leave the rest; do not "fix" the check by running `cargo fmt --all`, which would produce a
workspace-wide diff that collides with every open PR. The formatter *version* is pinned:
`rust-toolchain.toml` fixes the channel and ships the `rustfmt` component, so the check is
reproducible between CI and a local checkout even while it is non-blocking.

Security-specific lanes also run: **CodeQL** SAST
([`codeql.yml`](./.github/workflows/codeql.yml), `security-and-quality` queries), **miri**,
**fuzz**, **supply-chain**, and the **OpenSSF Scorecard** analysis
([`scorecard.yml`](./.github/workflows/scorecard.yml)). The aggregate `ci-summary` check
gates the merge.

### Reporting a vulnerability you find while contributing

If you discover a security issue, **do not open a public issue or PR** — use the private
channels in [`SECURITY.md`](./SECURITY.md) (GitHub Security Advisories or
jesse@jeswr.org). The machine-readable pointer is
[`.well-known/security.txt`](./.well-known/security.txt).

### Secure-SDLC (SSDF) touchpoints

The above maps onto the NIST SSDF practices: a documented secure-coding standard and threat
model (PW.1 — this section plus [`SECURITY.md`](./SECURITY.md) and the threat model),
review and analysis before merge (PW.7/PW.8 — code review, CodeQL, clippy, miri, fuzz),
supply-chain provenance (PW.4/PS.3 — cargo-deny, SBOM), and coordinated vulnerability
handling (RV — the disclosure policy and `security.txt`). Follow the PR conventions in
`AGENTS.md` (small, reviewed, all review comments resolved before merge).

## Tasks and TODOs live in beads, not markdown

This repo tracks work in **beads** (`bd`, a git-native dependency-graph issue tracker).
**Do not add `TODO` / `FIXME` markers to code or markdown, and do not create a `TODO.md`
or a checklist of pending work in a tracked doc.** Capture follow-up or discovered work as
a bead instead:

```sh
bd create "<imperative title>" -t <task|bug|feature|chore|spike> -p <0-4> \
  -l <area:crate,kind:...> -d "<what + why + where>"
```

Run `bd ready` to see unblocked work and `bd close <id>` to close. See the "Task
tracking — beads" section of `AGENTS.md` for the full rules (and never hand-edit
`.beads/issues.jsonl`).

## Crate README conventions

<!-- [OPUS-4.8] Canonical concise-README template (bead sq-kuqa, per design sq-9jw5). The
     per-crate README beads (sq-xogx / sq-h1s7 / sq-rt1t / sq-q2em / sq-puyy / sq-4kr5) follow
     this. The root README.md is the worked example of the publishable-crate shape. -->

Every crate carries a README, and they are **concise and human-readable**: a front page, not a
manual. The detail lives where it is single-sourced — how-tos in `skills/<surface>/SKILL.md`,
format/API reference in the crate's own rustdoc (`//!` and item docs), design rationale in
`research/`, and **all performance numbers in the benchmark dashboard / harness output** (the
"no hard-coded performance numbers" rule in `AGENTS.md` applies to every README). A README's job
is to orient and link, not to duplicate.

**Publishable crate (the default).** Target **~110 lines, hard cap 120**. Exactly **three emoji
section markers — 🚀 ✨ 📚, in that order** (tasteful, no others), with a final unmarked License
section. Required sections, in order:

1. **Title + badges** — crate name, then a badge row: crates.io, docs.rs, license (and CI where
   it fits). The root README is the example.
2. **One-line pitch** — what this crate is, in a sentence.
3. **What / why** — 3–5 lines: the capability and why it exists.
4. **## 🚀 Quickstart** — *one* `rust` code block, runnable as a doctest. Hide the scaffolding
   with `#` lines (`# fn main() -> Result<(), Box<dyn std::error::Error>> {` … `# Ok(()) }`) so
   the visible body is the API call, and end fallible lines with `?`. CLI/server/JS crates may
   use a `sh` block instead.
5. **## ✨ Features** — a short bullet list of *capabilities* (what it can do), not internals.
6. **## 📚 Learn more** — links only: `skills/<surface>/SKILL.md` (how-to), docs.rs (API),
   the relevant `research/<doc>.md` (design + measured verdicts), the benchmarks dashboard for
   numbers, and `AGENTS.md`.
7. **License** — one line.

Wire the README into rustdoc so the two never drift: `#![doc = include_str!("../README.md")]` at
the crate root, and `[package.metadata.docs.rs] all-features = true` in `Cargo.toml`. If a
Quickstart block contains a ```` ```rust ```` fence, `cargo test --doc -p <crate>` must pass.

**Internal crate (`publish = false`).** A **~15-line stub**, *no badges*: title + 3–5 lines of
what/why + a one-line "internal crate, not published to crates.io" note + a link to `AGENTS.md`.

**Tone.** Plain, factual, honest about scope. State limitations rather than overselling; move the
caveats and edge-cases into the SKILL.md / rustdoc rather than the README. Do not delete content
when trimming — relocate it to its single-source home and link from the README.

## Reporting security issues

**Do not report security vulnerabilities through public issues or PRs.** Use the private
channels in [`SECURITY.md`](./SECURITY.md) (GitHub Security Advisories or
jesse@jeswr.org). Note in particular that the `sparq-zk*` and `sparq-mpc` crates are
research scaffolds with **no security guarantee** — see `SECURITY.md` for the caveats.

## Filing issues

Use the [issue templates](./.github/ISSUE_TEMPLATE/) — there's a form for bug reports and
one for feature requests. Security reports are redirected to the private channels above.

## License

By contributing, you agree that your contributions are licensed under the same terms as
the project (see [`LICENSE`](./LICENSE)).
