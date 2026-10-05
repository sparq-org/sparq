# AGENTS.md — sparq

> A README for coding agents. If you are an AI agent working on or with this repo, read this first.

## What sparq is

sparq is a from-scratch **RDF triplestore and SPARQL 1.1 engine in Rust** — dictionary-encoded, six sorted permutation indexes, parallel + streaming execution, RDFS/OWL-RL/N3 inference, an out-of-core (mmap) mode with a compressed on-disk format, a WebAssembly build, and a W3C-conformant HTTP server. The engine is published across several surfaces:

- **Rust crates** (crates.io): `sparq-core`, `sparq-engine` (core), `sparq-cli`, `sparq-server`, plus opt-in capability crates (`sparq-reason`, `sparq-reason-el`, `sparq-shacl`, `sparq-geo`, `sparq-text`, `sparq-rsp`, `sparq-hdt`, `sparq-solid`, `sparq-arrow`, `sparq-mcp`, `sparq-vc`, ...). `sparq-reason-el` is a **separate** opt-in crate (depending on it is the opt-in): an OWL 2 EL consequence-based classifier that computes the **complete** `rdfs:subClassOf` subsumption lattice that OWL 2 RL (`sparq-reason`) is sound but silently incomplete for — see [`skills/inference/SKILL.md`](skills/inference/SKILL.md).
- **In-workspace, `publish = false` server estate** (NOT on crates.io): `sparq-lws-core` — an **EXPERIMENTAL** native Solid/LDP (Linked Web Storage) server core imported from [jeswr/solid-server-rs](https://github.com/jeswr/solid-server-rs), SPARQ-authoritative for RDF + WAC, with Solid-OIDC/DPoP auth delegated to the pinned [solid-oidc-verifier](https://github.com/jeswr/solid-oidc-verifier). It does **not** replace the TypeScript prod-solid-server and its default storage is ephemeral. `sparq-lws-wasm` is its opt-in wasm adapter (the local-development host behind `@sparq-org/solid-server`), and `sparq-wac-oracle` is the server-independent WAC/ACP decision test-vector corpus. Usage: [`skills/solid-lws-server/SKILL.md`](skills/solid-lws-server/SKILL.md); design decisions: [`research/lws-design-records.md`](research/lws-design-records.md).
- **npm**: `@sparq-org/sparq` — RDF/JS-typed API over the wasm build, zero runtime deps;
  `@sparq-org/solid-server` — loopback-only Solid/LDP development host over the separate
  in-memory wasm adapter, with fixed-owner default and opt-in Node-side Solid-OIDC verification.
- **PyPI**: `sparq-rdf` (import name `sparq`) — pyo3/maturin bindings.

Status: experimental research engine; the API is unstable.

## Skills — how to USE sparq from your code

Usage instructions for each public surface are packaged as Agent Skills under [`skills/`](skills/) (the [agentskills.io](https://agentskills.io) open format — `name`/`description` frontmatter + Markdown). Read the one that matches the surface you are integrating:

Read [`skills/SKILL.md`](skills/SKILL.md) first — it is the router skill that lists every surface and points you at the right one. The main entry points:

- [`skills/sparql-query/SKILL.md`](skills/sparql-query/SKILL.md) — run SPARQL from Rust (`sparq-core` + `sparq-engine`).
- [`skills/data-formats/SKILL.md`](skills/data-formats/SKILL.md) — parse/load RDF (Turtle/N-Triples/N-Quads/TriG, HDT) into a Graph.
- [`skills/rdf-wrapper/SKILL.md`](skills/rdf-wrapper/SKILL.md) — traverse RDF as Rust-native focus objects with the opt-in `sparq-wrapper` crate.
- [`skills/cli/SKILL.md`](skills/cli/SKILL.md) — the `sparq` CLI (query, mmap build/query, reason, bench).
- [`skills/http-server/SKILL.md`](skills/http-server/SKILL.md) — the SPARQL 1.1 Protocol HTTP server.
- [`skills/helm-deploy/SKILL.md`](skills/helm-deploy/SKILL.md) — deploy either native server to Kubernetes with the secure-default Helm chart.
- [`skills/javascript-wasm/SKILL.md`](skills/javascript-wasm/SKILL.md) — the `@sparq-org/sparq` npm package.
- [`skills/python/SKILL.md`](skills/python/SKILL.md) — the `sparq` Python package.

The capability surfaces (reasoning — RDFS/OWL-RL/N3 in `sparq-reason` plus the opt-in OWL 2 EL classifier in `sparq-reason-el`, both covered by [`skills/inference/SKILL.md`](skills/inference/SKILL.md) — SHACL, full-text, vector, GeoSPARQL, streaming RSP-QL, RDFC-1.0 dataset canonicalization, ZK query proofs, MPC, GenAI retrieval) each have their own `skills/<surface>/SKILL.md` — the router in [`skills/SKILL.md`](skills/SKILL.md) enumerates them.

If your agent runtime supports the Agent Skills standard, these load via progressive disclosure (name+description first, body on demand). If not, just read the SKILL.md files directly.

> Note: `.claude/skills/` (separate tree) holds INTERNAL skills for agents working *on* the engine's source (parsing perf, ZK circuits, etc.), not usage docs. Do not confuse the two.
## Working on this repo (contributor agents)

- Build: `cargo build --workspace`. Test: `cargo test --workspace`.
- Lint is enforcing (CI gates on it): `cargo clippy --workspace --exclude sparq-py --all-targets -- -D warnings` must pass. Run clippy over the **full workspace**, not a single crate — feature unification surfaces lints that an isolated-crate check misses. (`sparq-py` is excluded because it needs the Python/maturin toolchain.)
- **`cargo fmt --all --check` is NOT a gate — it is informational** (`continue-on-error: true` in `ci-fast.yml` and `ci.yml`). The one-time workspace reformat has never been run, so the check fails on files your change did not touch; do not treat that failure as your regression and do not fix it by running `cargo fmt --all` (a workspace-wide reformat is its own reviewed change — see `rustfmt.toml`). Format the code **you** touched, matching the surrounding committed style. The formatter *version* is no longer ambient: `rust-toolchain.toml` pins the channel and ships the `rustfmt` component, so CI and a local checkout run the same rustfmt (this closes the reproducibility half of issue #2360).
- The core crates (`sparq-core`, `sparq-engine`) must stay dependency-free of the opt-in capability crates, and the wasm build must not regress — both are enforced in CI.
- **New capabilities are opt-in by default** (a dedicated crate and/or a cargo feature that is OFF by default), so `sparq-core`/`sparq-engine` stay lean and the lean wasm bundle never grows. **One maintainer-directed exception (sq-oy1f.4 / sq-oy1f.20, user-prioritised epic [sq-oy1f]):** **JSON-LD is DEFAULT-ON in the native binaries + the Python wheel** — `sparq-cli` and `sparq-server` carry `jsonld` in their `default` feature set (the CLI parses + re-serialises JSON-LD, the server speaks `application/ld+json` in both directions), and `sparq-py` does too (`Graph.load(..., format="jsonld")` + a `.jsonld` path work out of the box; the wheel has no bundle-size floor). It stays toggleable (`--no-default-features` drops the `oxjsonld` parser in all three), and the lean wasm bundle keeps JSON-LD **opt-in** (`sparq-wasm/jsonld`, OFF by default — the `wasm_bundle_bytes` floor is unchanged). The `sparq-core`/`sparq-engine` *library* defaults also stay lean (oxjsonld enters only via the binaries' `jsonld` feature). What is default-on now: **JSON-LD parse + serialise (expanded/flattened/prefix-compacted) + full W3C 1.1 Compaction/Framing + server content-negotiation**; full conneg-conformance ratcheting is on the `sq-oy1f` roadmap.
- **Frontend optional-code policy (sq-mrrn4):** any net-new site/GUI feature that is uncertain-value or rarely used and increases bundle size MUST load through a literal ESM dynamic `import` only when the user invokes it; a feature flag or conditional render does not keep a static import out of the initial bundle. Use `next/dynamic`/`React.lazy` for components and invocation-path `import` for libraries/codecs. Classify new frontend dependencies as core/shared or optional in the PR, and extend `site/scripts/check-bundle.mjs` for material optional chunks. See `.claude/skills/frontend-design/SKILL.md` for the decision rule, exceptions, and audit procedure.
- Conformance: the W3C SPARQL, inference, W3C SHACL (core + SPARQL), OGC GeoSPARQL and Solid WAC + ACP suites must stay green and are each **ratcheted** (the committed floor only goes up). All of them are indexed in ONE central scoreboard — `cargo run -p sparq-conformance --bin sparq-conformance-scoreboard` (registry: `crates/sparq-conformance/src/scoreboard.rs`) — so a single artifact reports every suite + its floor + the CI job that gates it. The per-suite detail reports are **generated** by that crate, not committed: the SPARQL report `conformance-report.md` is git-ignored and regenerated locally by `cargo run -p sparq-conformance` (the CI job re-runs it and publishes it as a build artifact); the inference report is committed at [`inference-conformance-report.md`](inference-conformance-report.md), and the SHACL/geo/Solid job scoreboards are emitted the same way. Performance is gated the same way against a best-ever floor (`bench/perf-baseline.json`).
- **CI shape:** the one required PR check is **`ci-fast`** (clippy `-D warnings` + nextest + doctests on the core crates + the W3C SPARQL conformance ratchet). The heavy suites — `ci.yml` (full workspace, wasm, MSRV, every conformance ratchet, coverage, mutants), `feature-matrix.yml`, `fuzz.yml`, `formal-verification.yml` — run **nightly and on `workflow_dispatch`**; `supply-chain.yml` and `bench.yml` also run on push to `main`. Run a heavy suite on your branch before merging when your change is in its territory: `gh workflow run ci.yml --ref <branch>`. **Add the `bench` label to a PR to run Benchmarks against `main`.** Details: [`docs/branch-protection.md`](docs/branch-protection.md).
- **Merge discipline:** locally, the gate for a change is *full-workspace clippy + `cargo test` + the conformance/perf ratchets it touches*. When work is done in parallel git worktrees, merge **one branch at a time**; never edit `.beads/` files inside a worktree (it conflicts at merge — `bd export` regenerates the JSONL).
### sparq-substrate (shared eval substrate)

`sparq-substrate` is a **leaf crate** (depends only on `sparq-core`) holding the shared evaluation substrate consumed by **BOTH `sparq-engine` AND the reasoners**, so neither consumer depends on the other and they share one eval body (epic [sq-6tykl]/[sq-qonbz]; design `research/shared-eval-substrate.md`). It holds, behind **default-OFF features**:

- `numeric` — the XSD numeric value tower (`Num`/`Dec` + `as_numeric` classification + the arithmetic ops and XSD lexical helpers), driving the engine's FILTER/BIND/ORDER BY.
- `join` — the **four id-tuple join kernels** (sorted merge-join, radix-partitioned hash-join, index-nested-loop bind-join, leapfrog trie-join/WCOJ) behind a **generic `JoinKeys` descriptor** (the row→key projection + combine layout) and a **generic `Budget`** cooperative-cancel hook.
- `compare` — the **SPARQL term total order** (`compare::compare_terms` — the engine's `compare_values`: error/unbound < blank < IRI < literal < triple, numeric-aware + strict typed/temporal + string fallback + recursive triple-term order) behind a **generic `CompareTerm` trait** the consumer implements for its term type. Drives the engine's `ORDER BY` / sort / `MIN`/`MAX` fallback.
- `rows` — the shared `Row`/`Key`/`Posting` id-tuple vocabulary both kernels operate on.

**INVARIANT (perf-neutrality — enforced):** the hot loops are **MONOMORPHISED** — **NO `Box<dyn>`/`&dyn`/`dyn` trait-object dispatch** on any per-row / per-key-group / per-distinct-value hot loop. Generic type parameters bounded by a trait (`fn merge_join<B: Budget>(…)`) are fine (they monomorphise + inline); a trait *object* inserts a vtable the optimiser cannot inline, which would make the substrate non-zero-overhead for its two consumers and risk regressing the deterministic byte ratchets (`wasm_bundle_bytes`, store/dict bytes). This is enforced structurally by **`scripts/check-no-dyn-dispatch.py`** (the `no-dyn-dispatch (substrate)` gate in `docs-quality.yml` — comment-aware, with a narrow `// perf-neutrality-allow: <reason>` per-line opt-out for a genuinely-cold path). A new hot-path module in the crate must be added to that script's scanned set.

The engine's `compare_values` total order **now lives in `compare` here** (bead [sq-vezew], Phase 4): the ALGORITHM moved as the generic `compare::compare_terms` over a tiny `CompareTerm` trait, and the engine implements that trait for its `Value` (zero-cost wrappers over `value_str` / `as_num` / `value_compare_strict`) and calls the substrate body — so the engine AND the reasoners share ONE total-order body with no `Box<dyn>` on the compare path. The seam deliberately leaves `Value` **engine-resident**: the engine's `Value` enum, its `LitKind` literal-family classifier and `value_compare_strict` typed/temporal comparison ALSO drive the relational `<`/`>`/`=` operators (not just ORDER BY) and are coupled to `oxrdf::Term`, so a wholesale `Value` relocation would be non-perf-neutral and sprawling; moving the algorithm while surfacing the term observations through the trait is the clean perf-neutral seam (mirrors how `join` keeps `Bindings` engine-private behind `JoinKeys`). The genuinely-deferred remainder — should a reasoner ever need the *full* `Value`/`LitKind` value-space (not just the ordering) shared — is captured as a follow-up bead, not faked complete.

#### Substrate boundary: what lives in sparq-substrate vs. what stays engine-private (durable architecture fact)

sq-qonbz.7 — the substrate boundary is architecturally explicit: sparq-substrate holds **ONLY** the four generic hot-loop modules (rows / numeric / join—including join::delta / compare) and nothing else. These modules are consumed by BOTH the SPARQL engine and the reasoners, monomorphised and vtable-free.

**In sparq-substrate (generic, consumer-agnostic, monomorphised, shared):**
- `rows` — Row/Key/Posting id-tuple vocabulary.
- `numeric` — XSD numeric value tower + arithmetic ops (monomorphic over the concrete numeric tiers `i64`/`i128`/`f32`/`f64`; `#[inline]` accessors, no `Box<dyn>`/`&dyn`/vtable anywhere).
- `join` — four id-tuple join kernels (merge-join, hash-join, bind-join, trie-join) + `join::delta` (persistent extendable hash table for semi-naive Δ⋈full join). All generic over JoinKeys descriptor and Budget cooperative-cancel hook; no vtable on the hot path.
- `compare` — SPARQL term total order (compare_terms over CompareTerm trait). Generic over the trait; concrete consumer implements it for its term type; no vtable on the per-comparison hot loop.

**In sparq-engine (engine-private, NOT in sparq-substrate — never moved to the shared crate):**
- `Value` enum + `LitKind` literal-family classifier + `value_compare_strict` typed/temporal comparison (drives relational ops, ORDER BY, and all value semantics; coupled to oxrdf::Term).
- `Bindings` struct (engine's result binding representation; the join kernels expose Row/Key instead).
- `LocalVocab` interning (engine-specific; reasoners have their own vocab).
- `QueryBudget` thread-local cancellation (engine-specific; reasoners may supply their own Budget impl).
- `ScanCmp` pushdown filter logic (engine optimizer detail).
- `service.rs` (SPARQL SERVICE federation).
- Serializers and EXISTS/aggregation (engine executor details).

The seam is **clean and intentional**: generic algorithms flow outward to sparq-substrate; engine-private types and optimizations stay inward. This is verified structurally by the perf-neutrality gate (scripts/check-no-dyn-dispatch.py enumerates the four hot-loop modules — rows/numeric/join (including the join::delta submodule)/compare — and fails if any Box<dyn> / &dyn enters a hot path).

## MAINTENANCE RULE (REQUIRED — read before changing any public surface)

**When you change a public API, update the matching skill in the SAME change (same commit/PR).** A "public API" means any of:

- a `pub` item in a crate's public surface (a published crate's exported types, traits, functions, or their signatures);
- a CLI flag, subcommand, or its behavior in `sparq-cli`;
- an HTTP route, query/body parameter, or response shape in `sparq-server`;
- a Python binding (the `sparq` package) or a JS/RDF-JS binding (`@sparq-org/sparq`).

Then edit the corresponding `skills/<surface>/SKILL.md` (sparql-query / data-formats / cli / http-server / python / javascript-wasm) so its instructions and examples still compile and run against the new surface. Do not split this across a follow-up PR — a skill that documents a removed flag or a changed signature is worse than no skill. If the change spans surfaces (e.g. a new query option exposed in both the CLI and the HTTP server), update every affected `SKILL.md`. Keep each `SKILL.md` body under ~500 lines; move long flag/route tables and runnable examples into that skill's `references/` and `scripts/`.

If you add a brand-new public surface, add a new `skills/<surface>/` (dir name == the skill's `name` frontmatter) and link it from the list above and from the README.
## Fix a shared issue everywhere it applies — cross-crate/cross-surface parity
When a bug or review finding describes a **class** of problem affecting more than one place — a parser edge case in Turtle that also hits TriG, an operator bug whose sibling operators share a code path, a `pub`-surface footgun repeated across the CLI / HTTP / Python / JS-WASM bindings — it must **eventually be addressed in every instance, not patched only where it surfaced.** **Prefer fixing the pattern ONCE in the shared place** (the common code path, or a `sparq-core` helper) so all surfaces inherit it; if a shared fix isn't feasible, fix each instance to the **same spec** and file a bead for the consolidation. Either way, when you fix one instance, **file a bead** (see *Task tracking* below) covering the other affected crates/surfaces so the parity work is tracked, not lost. This is the cross-crate analogue of the differential-fuzz philosophy (a finding in one path implies checking the others — see the *Post-batch re-evaluation checklist*).

## Task tracking — beads, not markdown TODOs

This repo tracks work in **beads** (`bd`, a git-native dependency-graph issue tracker; the committed source-of-record is `.beads/issues.jsonl`). Rules for any agent working here:

- **Do NOT write TODO/FIXME into markdown or leave them in `TODO.md` files.** Capture future work as a bead instead.
- **When you identify follow-up/future work, create a bead for it** (from the repo root, with `bd` on your PATH):
  ```sh
  bd create "<imperative title>" -t <task|bug|feature|chore|spike> -p <0-4> -l <area:crate,kind:...> -d "<what + why + where>"
  ```
  This writes the shared Dolt DB (exclusive-lock-serialized — safe across parallel agents). For the rationale behind a *deferred* task, put it in the bead's `-d` description or `--design` field so the bead is self-contained. **Never edit `.beads/issues.jsonl` (or any `.beads/` file) by hand** — it causes merge conflicts; `bd export` regenerates it.
- Run `bd ready` to see unblocked work; close with `bd close <id>`.

**Beads session-context hook.** `.claude/settings.json` registers a `SessionStart` hook (`scripts/bd-session-context.sh`) that injects a concise bead snapshot — the `bd ready` list + open count — at the start of every Claude Code session, so a new or post-compaction session recovers the task state automatically. It's a graceful no-op when `bd` isn't installed or `.beads/` is absent. We deliberately do **not** use beads' own `bd setup claude` / `bd prime` injection: that path ships generic rules ("do not use TaskCreate / MEMORY.md") that conflict with this harness's task tracker and auto-memory, and it duplicates the beads guidance already in this file. The hook ships only the useful, non-conflicting part. (A committed `.claude/settings.json` hook takes effect on the *next* session start / `/hooks` reload, not the current session.)
## Agent working rules

These apply to every agent (and every sub-agent) changing this repo. The role prompts in
[`.claude/agents/`](.claude/agents/) point here; if they disagree, this section wins.

1. **Worktree + branch.** A mutating agent works in its OWN git worktree on its own branch
   from current `main` (`git fetch origin main && git checkout -b <kind>-<topic> origin/main`).
   Never switch branches in a checkout another agent is using — a working tree has one branch,
   index and working directory, so concurrent writers clobber each other. Read-only agents may
   share a checkout.
2. **Staging.** Stage only the files you changed, by explicit path. Never `git add -A`; never
   stage `.beads/`.
3. **Gate honestly.** Run the gates for what you touched (see the checklist below). If a gate
   fails on real content, fix the content or report it — never disable, regex-weaken or
   blanket-exclude a gate to go green.
4. **Honest scoping.** If the premise of a task is wrong, the thing already exists, or a claim
   lacks evidence, say so. Don't fabricate work, tests or numbers, and don't open a no-op PR.
   Capture follow-up work as beads (or list it for whoever owns the bead graph).
5. **Commit attribution.** Credit the model that wrote a change with a `Co-Authored-By:` git
   trailer. Do not stamp model names or tags into source, comments or docs.
6. **Agent-authored issues/PRs/comments say so** in their first line (the maintainer runs more
   than one agent under one account).
7. **A permission denial is final.** If the harness or a hook denies a tool call, do not retry
   the same change through a different tool. Report what was blocked and why it was needed.
   `.claude/agents/*.md` (the role prompts) are maintainer-owned: propose changes, don't make
   them as a side effect of another task.
8. **Out-of-scope discoveries** (a latent bug, doc drift, a footgun) go in a bead or a GitHub
   issue, not an unrelated fix in your PR.
9. **Pre-flight your own diff** before reporting done (*AUTHOR pre-flight*, below).

### Worktree lifecycle — remove every worktree the moment its task is done

Worktrees and their build artifacts (`target/`) are a large disk sink and accumulate fast. Standing requirements:

- **Remove every worktree the moment its task is done** — once its branch has merged (or its work is captured/abandoned), `git worktree remove --force <path>`. The **branch persists in `.git`**, so removal loses nothing; only the working copy + its `target/` go. Remove it in the same step that closes the bead / lands the merge. Don't leave worktrees lying around "in case."
- **Don't spawn a worktree you don't need.** Read-only or single-stream work uses the main checkout (per the isolation rule above); a worktree is justified only for *concurrent* mutating work. Reuse one scratch worktree for serial tasks rather than churning fresh ones.
- **Periodic sweep:** `git worktree prune` + `scripts/worktree-gc.sh` (dry-run by default; `--apply` removes only worktrees whose work is merged or gone) and `scripts/disk-guard.sh`; if disk is tight this is the first lever (before asking the user). Safe to delete: `target/` dirs, and the *git-ignored* benchmark outputs (per `.gitignore`: `bench/native-qlever/`, `bench/competitor-results/`) plus generated datasets, which suites write **outside the tree** — e.g. `bench/bsbm/gen.sh` defaults its output to `/tmp/bsbm/…` — and are regenerable. But **most of `bench/` is tracked** (~300 files): generators/runners (`gen.sh`/`run.sh`), queries (`*.rq`), expected results (`*.tsv`) and baselines like `bench/perf-baseline.json` are committed — never delete tracked bench assets, scripts, or `.gitignore`. When in doubt, `git ls-files bench/` shows what's tracked.

## Post-batch re-evaluation checklist — what to re-run after a change

**Run `python3 scripts/preflight.py` first.** It is one diff-scoped command that runs the
new-crate completeness check, the public-API → skill check, the config-documented check,
`check-no-perf-numbers.py`, `check-readme-template.py` and `check-privacy-claims.sh` against
YOUR diff, plus a `guard-untested` check, and prints the two obligations no script can decide
(mutate your headline guard; read your own prose against your own diff).

The base gate is full-workspace `clippy -D warnings` plus `cargo test` (scope to the affected
crates while iterating; the workspace-wide run catches feature-unification regressions). Also
run `cargo doc --workspace --no-deps --all-features` with `RUSTDOCFLAGS="-D warnings"` — a
public doc-comment that links to a private item inside a default-OFF feature module fails
`ci.yml`'s lint job even though `cargo clippy` never surfaces it. Then map change → evaluation:

| If the change touches… | Re-run |
|---|---|
| a parser (turtle/nt/nq/trig, `sparq-core` parse, `spargebra`) | W3C SPARQL + rdf-turtle conformance; the chunked-vs-serial parser oracle; `sparq-bench fuzz` (differential oracle); the `fuzz.yml` cargo-fuzz targets (`parse_rdf_str` / `load_reader_parallel` / `parse_sparql`). Locally: `cd fuzz && cargo +nightly fuzz run <target> corpus/<target> seeds/<target> -- -runs=0` (replay) or `-- -max_total_time=15` (randomized). |
| query execution / operators (`sparq-engine` exec/optimizer) | full conformance ratchet; the operator-coverage bench; per-builtin error table |
| the reasoner (`sparq-reason`, rules, closure) | inference conformance ratchet; incremental==batch property tests; LUBM entailed tier |
| a public API (`pub` item / CLI flag / HTTP route / Py/JS binding) | update the matching `skills/<surface>/SKILL.md` in the same change (*MAINTENANCE RULE*); the surface's tests (`scripts/gate-api-skill.py`) |
| a public config key / CLI flag / env var | document it (value, default, effect) in the matching `SKILL.md` / crate README (`scripts/check-config-documented.py`) |
| `sparq-wasm` / the wasm graph | `scripts/wasm-deps-guard.sh`; `wasm-pack test --node`; the `wasm_bundle_bytes` size gate |
| Cargo dependencies (`Cargo.toml`/`Cargo.lock`) | `cargo deny check` + `cargo vet` + the SBOM (`supply-chain.yml`) |
| the ZK verifier / circuits (`sparq-zk`, `sparq-zk-compose`) | `forge_gates` + `differential_fuzz`; the gate-count snapshot (`crates/sparq-zk-compose/tests/gate_count.rs`, see the circuit-member checklist below); the `zk-toolchain.yml` forge suite; re-open the soundness audit. If you change the public-input serialization (`verifier.rs::reconstruct_public_inputs`) re-capture the bb anchors via the `probe_*_public_inputs_hex` e2e probes. |
| SHACL (`sparq-shacl`) | the W3C SHACL conformance ratchet; `shacl-diff-fuzz.yml` (sparq-shacl vs pySHACL) |
| an outward ZK/MPC claim (README / `SKILL.md` / site copy / compliance text about `sparq-zk*` / `sparq-mpc` privacy or soundness) | keep every assertion hedged: the v1 ZK verifier is pending external cryptographer sign-off and `sparq-mpc` is honest-majority semi-honest only. `bash scripts/check-privacy-claims.sh` must pass; a legitimate line carries `privacy-claims-allow: <why>`. Weakening the regex or excluding a live doc surface is itself an honesty defect. |
| storage/encoding (`sparq-core` store/dict/compress, mmap, dict-spill) | the deterministic perf-gate metrics; byte-identity differentials; coverage with `--features dict-spill`; the `graph_open` fuzz target (corrupt store files must give `Err`, never a panic/OOM/UB) |
| a new `unsafe` block / `unsafe fn` | a `// SAFETY:` justification (lint-required) + a row in `compliance/memsafety/unsafe-register.md`; re-seed `bench/unsafe-snapshot.json`; the crate in the Miri lane |
| an opt-in cargo feature, or a test behind a default-OFF feature | wire it into a `feature-matrix.yml` leg (a per-crate fragment in `.github/feature-matrix.d/` **plus** a line in `scripts/tests/feature-matrix-legnames.golden.txt`). `ci.yml`'s nextest archive builds no other default-OFF feature, so such a test otherwise compiles empty and never runs. Prove it is reached: `cargo nextest list -p <crate> --features <set>`. `scripts/check-feature-test-execution.py --check` and `scripts/feature-matrix-tiers.py --enforce` fail on a gated test with no executor. |
| a new crate (`crates/<x>/Cargo.toml`) | a concise `README.md` (≤120 lines, or a ≤30-line `publish = false` stub with ``), a registered bench in `bench/benchmarks.toml` (or stub), and a `skills/<surface>/SKILL.md` if it is a public surface (`scripts/gate-new-crate.py`) |
| a new bench suite (`bench/<suite>/`) | register it in `bench/benchmarks.toml` and add a `FEATURED_SUITES` row in `bench/dashboard/dashboard.js` (or `featured = false`) (`scripts/check-new-bench-registered.py`) |
| a `research/*.md` design that is now shipped | graduate it into an architecture doc / crate README / `SKILL.md` (*Documents must stay current*) |
| coverage | the per-crate coverage ratchet + test-presence gate (`scripts/coverage-gate.py`, `scripts/coverage-presence.py`). Floors only rise: `coverage-gate.py --check-monotonic` fails on a lowered floor without an explicit, reviewed `--allow-lower`. |
| test quality (a test that runs a line but never asserts on it) | the mutation ratchet (`bench/mutants-baseline.json`, `scripts/mutants-gate.py`; nightly in `ci.yml`, advisory while seeding) — and mutate your headline guard by hand |

### ZK circuit-member checklist — adding or removing a `zk/compose/` member
A `zk/compose/` **member** is a `bin`-type Noir package directory under `zk/compose/` that uses the shared `compose_core` library (e.g. `join_eq_na16_nb16`, `filter_int_d4`, `scan_k1_n16_r4`). It is NOT `compose_core` itself (the shared lib) and NOT `target`. The membership-and-count gate lives in the **Rust** test suite (`crates/sparq-zk-compose/tests/gate_count.rs`), not in `nargo`, so `nargo test` passing is not evidence the gate passes. When you **add or remove** a member:

1. **Update the gate-count snapshot.** Add (or delete) the member's entry in `crates/sparq-zk-compose/tests/gate_count_snapshot.json` (`members` map) with its real `circuit_size` from `bb gates -s ultra_honk`. Re-baseline by running `bench/zk-compose/scripts/gate_counts.sh` and copying the values into BOTH the snapshot and `bench/zk-compose/gate_counts_latest.json` — `snapshot_covers_every_member` fails if a compiled member has no baseline, and the snapshot↔bench parity test fails if the two views drift. (For a member that is intentionally not gate-count-baselined, add an `exempt_circuits` entry instead.)
2. **Run the Rust gate locally:** `cargo nextest run -p sparq-zk-compose` (NOT just `nargo test`). `snapshot_covers_every_member` runs WITHOUT the `nargo`/`bb` toolchain — it only reads the `zk/compose/` directory names against the snapshot — so this step catches a missing baseline even on a box without the ZK toolchain. The `gate_count_regression` bloat check additionally needs `nargo`+`bb` (it skips cleanly when absent).

This is **Gate G5** territory for top-level `zk/` `bin` circuits (`snapshot_covers_top_level_circuits`); the `zk/compose/`-family coverage is enforced by `snapshot_covers_every_member` in the same test file.
**Externalized Noir dependencies (sq-5reoy / #1599).** The former in-tree `zk/ieee754` and `zk/xpath` trees were split out to the [`sparq-org/noir_IEEE754`](https://github.com/sparq-org/noir_IEEE754) and [`sparq-org/noir_XPath`](https://github.com/sparq-org/noir_XPath) face repos and removed from this repo. Their latest releases are **`v0.11.0`** (noir_IEEE754) and **`v0.3.0`** (noir_XPath, cut 2026-07-06 — see `research/zk-audit-readiness-dossier.md` §1.3). Those are the face repos' own release trains and are NOT automatically what this repo consumes: `zk/compose` pins `sparq_ieee754 @ v0.11.0`, and the XPath differential harness pins `XPATH_TAG: "v0.3.0"` (`.github/workflows/xpath-differential.yml`, `zk/xpath/scripts/run_differential_harness.sh` — bumped off v0.2.0 by #5456, so that lane's evidence is now about the current release) — read the pin, not this note, when you need to know what a lane actually verifies. `zk/compose` is the only in-tree Noir tree left; `zk/compose/compose_core/Nargo.toml` now consumes `sparq_ieee754` as a **pinned Nargo git dependency** (`{ git = "…/noir_IEEE754", tag = "v0.11.0" }`), exactly like its existing `poseidon` git dep. Two consequences for agents: (1) any `nargo compile` of `zk/compose` (the forge suite, `bench/zk-compose/scripts/gate_counts.sh`, the `sparq-zk-compose` test estate) now **fetches that git dep from GitHub** — a cold `~/.nargo` cache needs network access; a network-restricted runner must warm `~/.nargo` first. (2) **Toolchain-pin drift:** the `NARGO_VERSION`/`BB_VERSION` pins in `zk-toolchain.yml` and the face repos' pins are now maintained independently — when you bump the Noir toolchain here, confirm the released `sparq_ieee754` tag was cut on a compatible `nargo` (and re-run the forge suite, which compiles the git dep). Their nargo tests + the ieee754 differential oracle now run in the face repos' own CI, not in `zk-toolchain.yml`.

### New-parser correctness checklist — shipping a hand-written parser
A hand-written parser (replacing or bypassing a reference implementation) ships only with **ALL** of (design record: [`research/testing-strategy-assessment-2026-07.md`](research/testing-strategy-assessment-2026-07.md), section 8):

1. **Round-trip property tests** — `parse ∘ serialize ∘ parse` fixpoint over generated inputs (proptest), plus `serialize ∘ parse` identity on canonical forms, in the parser's crate.
2. **Differential fuzz vs the reference it replaces** — a cargo-fuzz target feeding identical bytes to both (e.g. native tokenizer vs `oxttl`), asserting identical triple streams/errors-modulo-documented-divergences; wired into the auto-discovered `fuzz/` workspace so per-PR corpus replay + nightly randomized runs apply automatically.
3. **Conformance-suite tie** — the relevant W3C ratchet floor (e.g. TurtleTests) unchanged or raised in the same PR; byte/count-identical parse on the suite corpus.
4. **Robustness target** — hostile-input fuzz (never panic/OOB), separate from (2), if the grammar entry point is new.
5. **Feature/fallback discipline** — a fast-path parser that falls back to the reference on unrecognized shapes must differential-test the *dispatch decision* too (fallback taken ⇒ results identical).
6. **Honest perf claim** — measured on the canonical bench path, never a work-box number in docs (see *No hard-coded performance numbers*).
## Documents must stay current — research records become architecture docs

A document must never describe the code as it ISN'T. Concretely:

- **No "not implemented" / "TODO" / "future work" statements left standing in a doc when they describe a real gap** — that is a disguised markdown TODO. Convert it to a **bead** and edit the doc to either delete the claim or replace it with a forward reference to the bead id. (If the feature IS now implemented, the statement is stale — fix the doc.)
- **A `research/` design record is provisional.** Once its design is implemented, it should graduate: either rewrite it into an **architecture document** describing what the code actually does (and where), or fold the durable parts into the relevant crate `README.md` / `skills/<surface>/SKILL.md` and delete the speculative design. A research doc that still says "we will…" or "X is not implemented" about shipped code is a bug in the docs.
- When you touch code, check the docs that describe it; if your change makes a doc statement false (in either direction), update the doc in the SAME change.
- **Top-level human-facing docs (the root `README.md`) state capabilities and link the relevant standards + in-repo docs; they do NOT explain engine internals and do NOT enumerate the contents of a linked spec.** Engine internals (dictionary encoding, permutation indexes, join algorithms, planning, delta overlays, mmap/compression, closure maintenance, …) live in `research/` design docs and crate `README.md`s — link them, don't inline them. When a doc says it supports a standard, hyperlink the standard and stop; don't list what's in it (no "SELECT/ASK/CONSTRUCT/property-paths/…" after a SPARQL link).

## Upstream blockers — roll your own, then contribute back

When a feature or a performance goal is **blocked by an upstream dependency** (a parser that rejects valid input, a missing API, a slow hot path), do NOT just mark it "unsupported/blocked-upstream" and stop. Instead:

1. **Vendor a local copy** of the upstream code (under `vendor/` or as a forked crate via `[patch.crates-io]`, as already done for `spargebra`), implement the feature/fix there, and ship it so sparq is unblocked.
2. **Open an issue + PR upstream** offering the change. Record the upstream issue/PR URL in the relevant bead and in the vendored copy's `*-PATCHES.md` (as `vendor/spargebra/SPARQ-PATCHES.md` does).
3. **Keep the PR live:** if you later change that vendored code, update the open PR; if the upstream PR was already merged/closed, open a new one for the delta.
4. A `blocked-upstream` bead is therefore a signal to roll-your-own + contribute, NOT a dead end. When the local implementation lands, the bead is unblocked.

**Proactive upstreaming, not just unblocking.** The rule above triggers on a blocker, but it also runs the other direction: when a fix or feature built here against a vendored or forked upstream (`spargebra`, the `hdt` crate, any `[patch.crates-io]` dependency) would be useful to that upstream even though nothing here was blocked, proactively offer it upstream (an issue + PR) rather than siloing it in the vendored copy. Record the upstream URL in the relevant bead and in the vendored copy's `*-PATCHES.md`. Keeping vendored deltas flowing upstream shrinks the patch set we carry.

### Upstream contributions — how to open the PR (the N3.js practice)

Every upstream PR an agent opens on a third-party repo (`KonradHoeffner/hdt`, `rossanoventurini/qwt`, any external dependency) **must** follow @jeswr's standing N3.js upstream-contribution practice. This mirrors how he opens PRs against `rdfjs/N3.js`:

1. **Open it as a DRAFT.** Never a ready-for-review PR.
2. **Explicitly identify the author as an agent** — a 🤖 SPARQ-agent self-id line in the body, e.g. *"This PR was opened by an autonomous agent (a SPARQ agent) operating on @jeswr's behalf."*
3. **Assign @jeswr as reviewer** (`gh pr edit <n> --add-reviewer jeswr`, and/or `--add-assignee jeswr`). If GitHub rejects this — the PR author IS @jeswr and you lack write/triage on the upstream repo, so `RequestReviewsByLogin` / `ReplaceActorsForAssignable` fail — **@-mention @jeswr in the body as the review gate instead** (the only available mechanism on a fork-based PR you authored). Note in the bead which mechanism was used.
4. **Carry a "NOT yet ready for maintainer review" note** in the body — the PR is pending @jeswr's own review first; the upstream maintainers should not review/merge until he marks it ready. **Never mark a PR ready-for-maintainer-review yourself — that is @jeswr's call.**
5. **Include a clear, concise "Why" section** — why this PR exists and why *this* repo needs it (e.g. "sparq vendors its own decoder purely to read every triple on bulk import; this provides that upstream so the wrapper can delete the vendored copy"). Why first, then What/How.
6. **Keep it as minimal as possible**, and **split unrelated changes across separate PRs** — one self-contained feature/fix per PR. Do not bundle independent changes.

When asked to revise such a PR, **edit the existing PR on its existing branch** — never open a new upstream PR, and never destructively force-push without flagging it. If a PR bundles unrelated changes, *note* which way it should split rather than silently rewriting it.

## Proactively maintain this file (and the skills)

Do NOT wait to be told. Whenever you notice a **repeated behaviour, a standing rule, a convention, or a hard-won lesson** that future agents should follow, add it to this `AGENTS.md` (or the matching `skills/<surface>/SKILL.md`) as part of the same work — the same way you'd capture a follow-up as a bead. This file is the durable home for "how we work here"; keep it current without prompting.

## Contribution workflow — PRs, reviews resolved, `ci-fast`

Changes land on `main` via **pull requests**. For every PR:

1. Branch → open a PR (`gh pr create`) and request review, including GitHub Copilot.
2. **Address and RESOLVE every review comment** before merge — make the change or reply with
   the reason it's declined, then mark the conversation resolved.
3. **`ci-fast` must be green** — it is the one required check. Other workflows report but do
   not block; still look at the ones your change touches, and dispatch the heavy suites on your
   branch when the change is in their territory (see *Working on this repo*).
4. Squash-merge and delete the branch.
5. **Close the GitHub issue when the fix lands on `main`**, with a comment linking the PR
   (prefer `Closes #N` in the PR body), and close the matching bead.

**Never merge the release-plz Release PR or a stacked PR by automation.** A stacked PR (base ≠
`main`) squash-merges into its stacked base, not `main`, when the lower PR lands — retarget it
to `main` first (`gh pr edit <n> --base main`). A crates.io version can never be unpublished, so
the Release PR is merged by the maintainer. `scripts/check-pr-arm-base.py` enforces both for
`gh pr merge … --auto` as a `PreToolUse` hook (`.claude/settings.json`); verify with
`python3 scripts/check-pr-arm-base.py --self-test`.

**Security & quality gating:** new security/quality regressions must not merge. CodeQL (SAST) + `cargo clippy -D warnings` + `cargo-deny` + the coverage/conformance ratchets watch for them. Keep the GitHub **code-scanning** alert count at zero — SHA-pin every action (`uses: owner/action@<full-sha> # vX.Y.Z`), and resolve/triage Scorecard + CodeQL alerts as they appear.

**Web + GUI E2E gating & flake-quarantine** — the deterministic site (`site/e2e/`) and GUI (`gui/e2e-playwright/`) Playwright lanes are **advisory-first**: they become hard checks only after earning it on a probation bar (**50 consecutive green runs on `main` spanning ≥ 10 distinct PRs, OR two weeks — whichever is LONGER**), and a flaky test is quarantined (`test.fixme`) same-day with a P2 fix bead — never re-run to green. Promotion is deleting the lane's entry from `.github/advisory-registry.json` and dropping the `advisory` token from its name. The checked-in policy + evidence ledger is [`.github/E2E-GATING-POLICY.md`](.github/E2E-GATING-POLICY.md); `tauri-driver` + nightlies never promote (design `research/web-gui-test-program.md` §6.3).

**Scorecard supply-chain + token conventions (born-compliant, so future config needs no clean-up):**
- **Pin published-artifact dependencies by digest.** Dockerfile base images are SHA-pinned (`FROM image:tag@sha256:… # image:tag`, keeping the readable tag as a trailing comment for legible bumps), same as CI action `uses:` pins. This covers everything in the **released** supply chain (the `ghcr.io` server image, the action graph).
- **SHA-pinned `taiki-e/install-action` MUST carry `with: tool:`** — `taiki-e/install-action` selects which tool to install from its `@<tool>` git ref (`@cargo-llvm-cov`); the SHA-pin above **drops that selector**, so without an explicit `with: tool: <name>` the action installs **nothing** and the downstream `cargo <tool>` ENOENTs — silently making the gate **vacuous** (this is exactly the coverage-gate regression root-caused 2026-06-18). `scripts/check-install-action-tool.py` is a stdlib-only lint that scans `.github/workflows/*.yml` and **fails** on any SHA-pinned `install-action` step missing `with: tool:`; it runs (with a `--self-test`) in `docs-quality.yml`. When adding a SHA-pinned `install-action`, always include the `with: tool:` input.
- **Ephemeral bench/bootstrap scripts are exempt from hash-pinning.** The throwaway self-terminating EC2 bench/hardware-run scripts (`scripts/aws-bootstrap.sh`, `hwrun/*.sh`, `bench/**/remote.sh`) `curl … | sh` rustup and best-effort `pip3 install` transient tools (e.g. `rapidgzip`); they're outside any released artifact, so their Scorecard `PinnedDependencies` alerts are **dismissed** (`won't fix`, with a per-file reason) rather than given a brittle fake pin. CI helpers that *are* part of the workflow graph (e.g. `python.yml`'s build-tool install) are pinned with `==` where it's a small fixed set.
- **Least-privilege workflow tokens.** Every CI workflow declares a top-level `permissions: contents: read`; any job that needs to write (push a branch, create a release/deployment, comment on a PR, assume an OIDC role) opts into the **narrowest** scope **per-job**, so every other job inherits read-only. A job-level `contents: write` that is genuinely required (e.g. the release job publishing a GitHub release) is the accepted least-privilege necessity — keep it scoped to that one job (dismiss its Scorecard `TokenPermissions` alert with that reason rather than removing the needed grant).

**Supply-chain attestation stack (cert epic sq-toze — GX-1/2/7).**
- **`cargo deny check advisories` is a hard check** in `supply-chain.yml` (push to `main`, nightly, dispatch; no `continue-on-error`). The old CVSS-4.0 parse blocker (sq-q8de) is resolved; the policy is **fail-closed** (`deny.toml`: `yanked = "deny"`, advisories v2 ⇒ every unignored advisory fails). Every `deny.toml [advisories].ignore` entry carries a justification + a tracking bead, and the list only ever shrinks by REMOVING the dependency. Most recently sq-5ah3p retired `rustls-pemfile` (RUSTSEC-2025-0134) for good: the archived crate was the last PEM decoder in `sparq-lws-core/src/tls.rs` + `sparq-server/src/main.rs`, and both now call `rustls-pki-types`' `PemObject` through rustls' own `pki_types` re-export, so it is absent from Cargo.lock. A real vuln, a yanked crate, or any regression that reintroduces an unmaintained dep reds it. Keep `deny.toml [advisories].ignore` and the VEX (below) **1:1 in sync**.
- **Per-release CycloneDX SBOM + VEX.** `scripts/gen-sbom-vex.sh` emits a CycloneDX SBOM per released binary (`sparq-cli`, `sparq-server`) + a version-stamped **VEX** (`supply-chain/vex.cdx.json` is the checked-in source of truth; it states `not_affected` + justification for every advisory `deny.toml` ignores). The `release.yml` `sbom` job runs it, SLSA-attests the outputs, and attaches them to the GitHub Release (covered by `SHA256SUMS`). Editing the ignore set ⇒ update `supply-chain/vex.cdx.json` to match. Each shipped SBOM is normalized through `scripts/sbom-normalize.jq` (a deterministic, idempotent `jq` transform, also applied in `supply-chain.yml#sbom`) so no host-revealing absolute build path leaks into a `bom-ref`/`purl`: `path+file://…#<ver>` refs become canonical `pkg:cargo/<name>@<version>` and the dependency graph is rewritten in lock-step (gap GS-6 / sq-toze.30).
- **`cargo-auditable`** wraps **every shipped-binary build path** — the `release.yml` `package` job, the `dist.yml` matrix, the `Dockerfile` builder, and the local `scripts/build-dist.sh` (sq-ytnq) — so the shipped binary/image **embeds its dependency manifest** (`cargo audit bin <file>` post-build). **`cargo-vet`** (`supply-chain/{config,audits.toml,imports.lock}`) is a hard check (`supply-chain.yml`, `cargo vet --locked`): every crate must be audited, covered by an imported trusted audit set (Mozilla/Google/Bytecode-Alliance/ISRG/Embark/Zcash), or hold an explicit `[[exemptions.*]]` entry. The bootstrap exemption set makes it pass today; the gate's value is the **ratchet** (a new unaudited/unexempted dep fails until audited/exempted). `cargo vet suggest` shows what to audit to shrink exemptions; the vendored `spargebra` patch is `audit-as-crates-io`.
- **Screen a NEW crate BEFORE you add it — not only at the post-hoc gate.** The gates above (`cargo deny`, `cargo vet`, the SBOM/VEX) run *after* a dependency is in the tree; they catch advisories, bans, and unaudited deps, but they do not by themselves stop you reaching for a **typosquatted / slopsquatted name** (an LLM-hallucinated or look-alike crate), a **suspiciously brand-new or single-release** crate, or a **low-reputation / unmaintained** one in the first place. Before adding any new dependency, do a quick provenance check — confirm the crate name is the one you mean (not a homoglyph/typo of a popular crate), that it has a real release history + repository + non-trivial reverse-dep/download footprint, and that it is maintained — and prefer an already-vetted crate or the std/`sparq-core` path over pulling a new one. A *new* dependency is a supply-chain decision, so the bar is "is this crate trustworthy and necessary," not just "does the gate pass."

**Perf gate — deterministic strict, timing advisory (`scripts/perf-gate.py`, sq-dzfu/sq-perf).** The perf ratchet hard-gates the **DETERMINISTIC** metrics (integer byte counts — `store_bytes_per_triple{,_small}`, `dict_bytes_per_term`, `wasm_bundle_bytes`) strictly against the committed best-ever floor in `bench/perf-baseline.json`: any value past its band fails (exit 2). The **TIMING** metric (`parse_ns_per_byte`, wall-clock-derived) is **ADVISORY / non-blocking** — it is still measured, still warned-on loudly (a band trip prints a prominent `WARNING (advisory, non-blocking)` with the reading + band), and still tracked/published on the dashboard, but a timing-only regression contributes **exit 0** and never fails the run. Reason: shared GitHub-runner wall-clock variance exceeds any useful band even with the best-of-N re-measure — `parse_ns_per_byte` flapped the merge train repeatedly (it tripped on main, was "fixed" by raising the floor to the series median in #133, then flapped *again* on unrelated PRs like #130, an MPC-only change that touches zero parsing code) because the published `parse_ns_per_byte` series (the dashboard / `bench/perf-baseline.json` history) spans a band wider than any useful threshold. The best-of-N re-measure (`ci-bench.sh --parse-only`, up to K reads, keep the min) still runs to squeeze the *tracked* number toward the true cost, but its outcome is advisory. The deterministic-vs-timing split is data-driven from each metric's `mode` (`noise`=timing/advisory, else deterministic/hard), not a hard-coded name list; a mixed run exits 2 (the deterministic fail dominates) while still emitting the timing advisory. Result: CI-runner timing noise never blocks a merge (we removed the false-positive merge-block, not the visibility), real deterministic regressions are still caught.

### Review lessons — checkable rules distilled from caught defects

These are the review reflexes that caught **real** defects at the verdict gate (2026-07-06). Apply each as a PASS/FAIL check, never a vibe; each cites the PR whose defect motivated it. The unifying failure mode: *green-and-configured is not the same as executed-and-gating* — trust the live evidence, not the shape of the config.

- **EFFECT-EVIDENCE RULE — never accept "the config looks right"; open the live check-run log and confirm the new tests EXECUTED and the enclosing JOB is required.** A green PR whose new tests never ran, ran in a non-required job, or exercised nothing the engine consumed is **vacuous**. Read the actual run (`gh run view <id> --log` / the check-run output), see the assertions print, and confirm the job is not advisory. (Caught: EL abox tests compiled-but-ungated #1672; an ACP "coverage" benchmark the engine consumed **none** of the emitted policy shape #1650; a step "gating" from inside an advisory job #1679.)
- **DELETION-EVIDENCE RULE — a green suite after a REMOVAL proves nothing about what the removal took with it.** A guard nested inside the region it protects is deleted along with that region, and the diff looks like a coherent feature removal, so review and CI both miss it. The signature is a mutant moving **`KILLED` → `SURVIVED` when a feature is removed** (its target still exists, so it did not merely leave the spec) — i.e. **diff the mutation kill set across the deletion**, and report the transitions, not just the post-deletion total. For a contract crossing a module or repo boundary the mutant must instead be a **symmetric rename** (rename the symbol in production *and* its own tests together; if nothing reds, no assertion checks the name the CONSUMER resolves). (Caught: a `fetch_lanes` guard deleted with the queue-wait region it was nested inside, #4810 / registry #1031; an enrichment call site deletable with the suite green, same PRs; an inertness contract bound to the defining module instead of the one the consumer loads, #4823 / registry #1032.) Full class, detection method, worked instances and limits: `research/guard-mortality-and-kill-set-diffing.md`.
- **FEATURE-LEG PAIRING RULE — a new feature is not done until it has all three: (a) its own CI feature-matrix leg, (b) an `LC_ALL=C`-sorted golden-fixture line, and (c) an assemble/self-test that goes RED when the leg or the line is missing.** Compiling under `--all-features` is NOT execution; a feature with no dedicated leg is silently never run, and a golden fixture without the deterministic-locale sort flaps. (Caught: EL abox — missing matrix leg, then missing golden line, **TWICE**, #1672.)
- **ORACLE-STRENGTH RULE — a differential/conformance oracle compares term structure and full answer SETS, never row COUNTS.** A count-equal oracle silently passes a shared-blank-node cartesian product, a duplicated-binding blow-up, or a wrong-term / same-cardinality answer. Strengthen to set/term-structure equality before trusting any "matches the reference engine" claim. (Caught: a QL shared-blank-node cartesian product exposed **only** by strengthening the oracle to term-structure equality #1653.)
- **FAIL-CLOSED-BRANCH RULE — every multi-branch operator (UNION / OPTIONAL / FILTER combinations) must fail CLOSED, and the oracle must include a case where exactly ONE branch is permissive.** A filter dropped on a single UNION arm "fails open" and leaks rows; a single-branch happy-path test never sees it. (Caught: QL multi-branch UNION+FILTER fail-open #1647.)
- **GRADUATION-EVIDENCE RULE — a ratchet floor moves only with per-CASE oracle evidence for each newly-passing case; never force-pin or hand-edit a floor to turn a ratchet green.** When a feature genuinely graduates cases the floor rises **legitimately** — but prove it case-by-case; a pinned floor hides a regression behind a passing ratchet. (Caught: QL graduation-floor gap where the feature really did graduate cases yet the ratchet arm tripped #1653.)
- **DOCS-HONESTY RULE — a doc that asserts a soundness / correctness property IS itself a soundness surface; review it as adversarially as code.** "Feature X supports Y" in a README / SKILL / rustdoc is a claim that must trace to a passing test on the REAL path; if the code is unsound for that case the doc is a false soundness claim, not a cosmetic nit. (Caught: RIF docs claiming variable-equality works when it is unsound #1651.)
- **REACHABILITY-SEEDING RULE — a reachability-pruned / orphan-dropping validator must not seed its reach set from anything the validated data controls (e.g. declaration-typing).** Declaration-typing reachability lets an orphan node re-enter the reach set and bypass the validator; confirm the seed set is closed over TRUSTED roots only. (Caught: DL orphan-validator bypass via declaration-typing reachability seeding #1652.)
- **LANE-ISOLATION RULE — prove a per-PR lane and a nightly / heavy lane are disjoint by running the selector (`--list` / `testMatch`) in BOTH env-flag states and diffing the two lists.** An unfiltered `testMatch` leaks nightly visual specs into the per-PR lane (slow, flaky, wrong gate). Env-gate the selector and show the two lists differ by exactly the intended set. (Caught: nightly visual specs leaking into the per-PR lane via an unfiltered `testMatch` #1676.)
- **ADVISORY-vs-HARD RULE — "does it fail?" keys on the enclosing JOB, not on a step's wording.** A load-bearing step inside an advisory job (declared in `.github/advisory-registry.json`, `continue-on-error`, or findings swallowed at the step) does NOT fail anything, however authoritative its `run:` reads — put hermetic checks in their own hard job. And only `ci-fast` blocks a merge; everything else is a signal you must read. (Caught: a CI step that claimed to gate while running in an advisory job #1679; then #3773 — the name rule itself neutralising four real gates.)
- **MISSING-LEG-ATTRIBUTION RULE — a missing check is not a failing check: when one `opt-in *` leg looks red or blocking on MULTIPLE unrelated PRs at once, check the `assemble feature matrix` job FIRST.** If that `setup` job fails (golden leg-name drift in `scripts/tests/feature-matrix-legnames.golden.txt` after a branch adds a new opt-in leg, the C1 feature-gated-test-execution guard, the tier ratchet, or a malformed `.github/feature-matrix.d/` fragment), the whole opt-in matrix is never generated and **every** `opt-in *` check goes unreported — which masquerades as a single-leg cross-PR main regression. The job emits a loud `::error` + job-summary attribution on failure (pinned by `test_feature_matrix_assemble.py`); triage the assemble job's own failing step, never the phantom leg. (Caught: the false 'spqv-provenance regression' drain-blocker alarm — three unrelated PRs, zero actual leg failures, 2026-07-11.)

### AUTHOR pre-flight — run these on your OWN diff before you report done

The rules above are the *reviewer's*. This list is the **author's**, and it exists because on 2026-07-27/28 essentially every PR that went through independent adversarial review here and in the sibling `jeswr/agent-account-registry` **failed its first review** — not on design, on this small repeating set. Running them yourself costs one pass and saves a whole review round. Each cites the PR that earned it (`reg #N` = the registry).

⚠️ **This list exists in FULL in two repos with no shared owner** — here and in `jeswr/agent-account-registry`'s `AGENTS.md`, which has no `CLAUDE.md` and whose worker container is offline, so it cannot point at this file. **Treat this copy as canonical**: change a rule here and mirror it there in the same wave, or say why not. The two have already diverged on lane-specific detail (tooling, examples, emphasis). That is the `reg #958` shape applied to prose, and `reg #945` measured the cost of duplication directly — two copies of one guard make **each copy individually unkillable**.

1. **Line coverage FIRST — and read it LINE-granular, not function-granular.** Run the module's own `--self-test` / test binary under coverage (`python3 -m trace --count …`; stdlib, no install needed — or `cargo llvm-cov`) and list the **never-executed LINES** before you mutate anything. Four for four as a predictor of where mutants survive: `reg #756` (`cmd_record` + `_read_json` never executed → the shipped tree printed `planned_rows=4` where the mutant printed `0`), `reg #956` (the module's **only two write methods** had never executed anywhere), `reg #937` (`main` + `_gh_readers` at 0 % → **13 of 13** one-line edits there survived a **248**-check suite, including an `apply=false` "census-only preview" that writes real ledger records), `#4743` (**17 of 29** functions at 0 %; `main` at 55 % with its whole `sweep` branch unexecuted, so `return 1 if …sweep else 0` → `return 0` survived all **111** tests). Helpers get tested because they are easy to call; **entry points get skipped because the test has to construct the real world — which is exactly where a *fabricating* bug survives.** ⚠️ **"Nothing at 0 %" does NOT clear you.** The function-granular headline is the weak form and it misses the worst regions: `reg #956`'s `main` is at **8/18**, not 0 %, and a fresh sweep of exactly that region found **10 survivors out of 10**; `reg #941`'s `_escalate_two_head` had **1 of 3 call sites covered at 3/3 confidence**, which `reg #945` re-derived as **3 of 4 site lines never executed while the enclosing functions read 75 %**. ⚠️ **Validate the coverage instrument against a function you know is never called**: `reg #756`'s counted docstring lines as covered, scored a never-called function at 6.2 %, and printed *"no code unit is entirely unexecuted"*; `reg #956`'s reported zero uncovered lines from a mode that **cannot emit one**. An instrument that cannot fail has told you nothing.
2. **Ask FOUR independent questions of every assertion** — none subsumes another, and each found holes the others swept past (`reg #941`). (a) Does the **call site** recompute or re-wire this value? (`reg #937` `Z6`: dropping one argument at the single production call site bound the wrong issue with a 219-check suite green.) (b) Does my **expected** value come from the same place the code reads it? (`reg #958`: `review:parked` defined four times — every assertion compared what a module wrote against the constant it writes from, a tautology that cannot fail.) (c) Does my **input** derive from the same constant the code reads? (`reg #941`: every over-cap input derived from `STUCK_UNPARK_MAX`, so setting `STUCK_UNPARK_MAX = 999` left 76/76 green.) (d) Does this control ever **execute**, and does the check test the flag's **value** or merely its **presence**? (`reg #941`: `--stuck-grace-hours 6` → `100000` survived 76/76.)
3. **Two mutants per guard: DELETE it, and separately make it conditionally inert** — in a **non-crashing** form. They are different experiments. `reg #938`: deleting a census emission was caught; wrapping it in `if census.get("total")` was **not**, so it would have vanished on exactly the quiet tick an operator interrogates. ⚠️ **One-at-a-time is structurally blind to a DUPLICATED guard** — see item 4's fourth outcome; that experiment needs both copies gone at once.
4. **FIVE false mutation outcomes — say which one you have.** *False kill*: an exception raised **by the mutated line itself** is malformedness, not detection (`reg #956`: two mutants "died" to an `IndexError` that aborted the suite before any row printed). *Equivalent survivor*: declare it and show it unreachable (`reg #937` `D1-default`). *Value-identical survivor*: the substituted value collides with one the fixture already uses (`reg #941` pins a fixture head as `'b'*40`) — **choose mutant values that appear nowhere in the harness.** *Mutually-masking duplicates*: two copies of one guard make **each copy individually unkillable** — three of `reg #945`'s four survivors were a single `MIN_ARG_TOKEN` floor written at both the producer and the consumer, where *"removing either copy alone left the suite green."* Item 3's one-at-a-time protocol **cannot see this**: find it by asking whether the value is written twice, and by deleting **both** copies as one mutant. *Crash-after-partial-run*: a mutant that reds some rows and then **aborts** the suite records as KILLED while every check below it never ran (`reg #945`: an emission block raising `IndexError` after three named `FAIL` rows). Require the mutant run's **total check count** to equal the pristine run's before you call it a kill.
5. **Ask of every control: WHO can write the thing this reads?** Three arm-capable holes in one night, all from evidence read out of **author-controlled** text with no author filter: `reg #681` (per-provider review markers parsed from `pull["body"]` → the required-review count goes **2 → 1** and the surviving lone review arms it), `reg #937` (closing-reference declarations from title/body), `#4743` (a marker in **any** comment, with no `login` / `author_association` check, on a **public** repo → a drive-by comment forges `route=preserve` and re-arms). Evidence *about* a review must be written by the party that did it, filtered by author, and read with **quoted contexts stripped** — a marker inside a fenced block otherwise self-marks (`reg #937` `Z1`–`Z5`).
6. **The YAML seam is where the vacuity lives** — every uncaught mutant measured that night sat at a workflow `if:`, a step, or a call site, never in the module logic. **Pin exact-match, not containment**: `reg #956`'s `--apply-DROPPED` and `--reconcile-max-DROPPED` both survived a substring check; `reg #941`'s `if: false` on the resolver **step** and on the whole **job** each survived 76/76; `#4743`'s `route != 'preserve' && false` satisfies a substring assertion while killing the lane.
7. **Never substring-grep for a kill.** These suites print each row's name on the **pass** path too — extract kills line-anchored (`^FAIL:` / `^\s+FAIL\b`) or from the traceback frame. Measured on one real 6100-line gate log: **62** lines contained `FAIL` as a substring, **44 of them passing `ok` rows**; the anchored form extracted **1** (`reg #949`).
8. **A census must always emit, including a zero row.** Ask: *would this alarm fire if this branch took 100 % of the population?* (`reg #938`: the reservation census never zero-sealed.) And **a residual computed from rows that ENTERED a pipeline cannot see a loss that prevented entry** — `reg #756`'s `chain_unaccounted` read 0 in both the shipped and the mutant tree, so its own missing-edge detector was structurally blind to the break.
9. **Verify the marquee claim against the EVIDENCE path, not the object it names.** `reg #681`'s headline held for the *record* and failed for the *review-set evidence* it is actually enforced through. The feature in the **title** is disproportionately the one with no red test — mutate it first.
10. **Publish corrected counts.** Four headline numbers moved that night once someone asked a specific question of every row: `reg #941` 22/22 → **21/22** then 26/26 → **25/26**; `reg #937` 48/48 → **48/56**; `reg #956`'s "52 mutants, 52 killed, 0 survivors" → six reproducible survivors. **A downward correction is what makes the rest of the report trustworthy** — the counts that never moved are the ones a reviewer rejects.
11. **Check what the transition DELIVERS INTO.** A park exit that re-admits into an unchanged tree, a mint that yields no review, or a fix that lands one layer short of the binding layer has produced nothing (`reg #956`; `reg #937`'s root cause was swept only as far as `sweep`'s call sites and stopped one layer short).
12. **Do not "re-run your sweep" — ask a NAMED question.** A re-sweep returns the same answer. One precise question — *"which of my assertions reads its expected value from the code under test?"* — is what turned up real defects in the same authors' own patches that night, including one author catching its **own fix** one layer short.

### Hermetic suites must be UNABLE to reach the network — poison the runners

A "hermetic" test suite that merely *intends* to inject fakes is not hermetic. `def __init__(self, ..., gh=run_gh)`
binds the runner **at definition time**, so patching `module.run_gh` from a test cannot reach it — and a suite
whose reads were faked while its writes were real posted **567 real comments to a production PR** across a
mutation sweep (#4652). Two rules follow:

- **Late-bind injected collaborators** (`gh=None` → resolve inside `__init__`), never a module-level default arg.
- **Poison the real runners at test-module import** so any path that forgets to inject raises instead of
  reaching the network, and add a test asserting the poison is in place. Restore **every** patched name in
  `finally` — a block that restored one of two leaked a stale fake into every later test.

**Never exercise a write path against a live production PR/issue.** Use a scratch object you own, or a fixture;
if a test needs a real remote value, *read* it. And when a probe reports hostile input, **verify the author
before reporting an exploit** — a false attack report costs twice: unwarranted alarm now, and a discounted
warning when the real thing arrives.

**Mutation harnesses need a preflight.** A greedy edit silently deleted 15 mutants and the harness reported a
clean total — a dropped mutant is indistinguishable from a killed one, so the number improves as coverage
disappears. Fail the sweep outright on a duplicate id or an anchor that no longer exists in the tree, count a
`SyntaxError` as a broken mutant rather than a kill, and prefer a test-file traceback frame over a crash frame
(the first failure is arbitrary).

## Monitor CI after every push to main — and fix red

A local gate is necessary but NOT sufficient: CI runs on a clean checkout with different toolchains/targets/feature-unification than your incrementally-built box, so it catches things your local gate cannot (e.g. a `cargo test --workspace` that includes a crate your local gate `--exclude`s; a wasm-target-only lint; an action container with an older cargo). Therefore: **after pushing to main, watch the CI runs to completion (and the next nightly heavy suites) and fix any failure immediately** — a red main is a stop-the-line condition. `gh run watch <id> --exit-status` (background) or `gh run list --branch main`; on failure `gh run view <id> --log-failed`. Roll the fix into the next push and re-watch. Do not pile more pushes onto a red main.

## Where this runs, build profiles, deploy images

**Where this runs.** Agent sessions typically run on a persistent work box (an EC2 instance), not a laptop and not a throwaway bench instance (confirm with `systemd-detect-virt --vm` → `amazon`, or query the EC2 instance-metadata service, e.g. `curl -s http://169.254.169.254/latest/meta-data/instance-id`; note `uname -r` showing `…-aws` only reflects the AWS-tuned kernel flavour and is not a reliable provider check across distros/AMIs), not a local laptop and not a throwaway bench instance. Three things to never get wrong: (1) **a wall-clock/throughput number measured on this box is NON-CANONICAL** — never bake it into markdown or into a test's expected values; gate only DETERMINISTIC metrics (byte/triple/gate counts), keep timings advisory, and treat a controlled quiet box or the CI runner as the authoritative perf source (this is *why* the timing perf-gate is advisory and *why* "no hard-coded performance numbers" exists). A benchmark/CI-suite agent must assert deterministic invariants (e.g. closure **triple counts**), not timings it measured here. (2) **This box is the work box, NOT a throwaway** — never self-terminate it; the orphan-proof self-terminate rule + orphan-checks apply only to bench instances you explicitly *launch* for measurement. (3) **It is not a CI runner** — benchmark/conformance suites run in GitHub Actions on clean ephemeral runners; running gen/run here is only quick local verification.

**Build-profile honesty guard.** The `release-fast` Cargo profile exists only for optimized correctness/smoke lanes and local iteration. Never use it for a measured lane: canonical EC2 benchmarks, perf-gate ratchets, published performance numbers, benchmark gatherers, and shipped artifacts must use the shipping `release` profile (or the explicitly shipping `release-wasm` / `python-release` profile). Results produced with `release-fast` are not valid performance claims.

**Deploy-image hygiene.** Build the published `sparq-server` container (`ghcr.io`, SHA-pinned base — see *Scorecard supply-chain* above) from a **minimal, artifact-only build context** — the compiled server binary plus its runtime config — never the repository root, so source and any local secrets cannot ride into the published image layers. (The release workflow's `docker buildx` job already scopes its context this way; keep it that way.)

## Inputs needed from the user — the `needs:user` bead queue

Anything blocked on a human decision, credential, or out-of-repo action (re-auth SSO, a
ruleset change, approving a destructive step, a product decision) is a **bead labelled
`needs:user`** with the exact ask in the description (`bd list -l needs:user`). Surface the open
ones at the end of a work session. Everything else: make the best-judgment call, document it
(PR body / bead), and proceed — except never label an unaudited ZK/MPC capability sound.

## Helper scripts (bead + worktree bookkeeping)

All dry-run by default; mutation only behind `--apply`.

- `scripts/bd-session-context.sh` — the `SessionStart` hook: a `bd ready` snapshot.
- `scripts/push-frontier.sh` — the launchable-bead frontier (ready beads minus in-flight ones,
  at most one per crate/surface).
- `scripts/reconcile-merged-beads.sh` — open beads whose fix already merged (close candidates).
- `scripts/bead-close-on-merge.sh <pr>` — close the beads a merged PR resolves.
- `scripts/refill-candidates.sh` — read-only list of candidate work.
- `scripts/worktree-gc.sh` / `scripts/disk-guard.sh` — remove merged/abandoned worktrees;
  check free disk and escalate.
- `scripts/orphan-check-bench.sh` — find orphaned EC2 bench instances.

## No hard-coded performance numbers

Do not bake benchmark numbers (MB/s, ×-faster, recall, gate counts, latencies) into markdown. Reference the **generated structured data** instead (the benchmark harnesses emit JSON; CI publishes results). If you cite a number, cite where it was generated.

## Repository hygiene — where things live (READ THIS; it keeps the repo clean by default)

Everything you produce has exactly **one** correct home. Putting it anywhere else creates the cruft that forces periodic "clean-up runs" — so don't create it in the first place.

- **Tasks / TODOs / follow-ups / "future work" → a bead.** Never a `TODO`/`FIXME`/`XXX` marker in a markdown file, never a `TODO.md`, never a `- [ ]` checklist of pending work in a tracked doc. If you catch yourself writing "we should later…", run `bd create` (see the beads section above) and move on. Code-comment `TODO`s are discouraged too — prefer a bead and reference its id.
- **Durable knowledge → `AGENTS.md` / `CLAUDE.md`, a `skills/<surface>/SKILL.md`, a crate `README.md`, or a `research/` design record — whichever fits.** Workspace-wide conventions and contributor rules go here in `AGENTS.md` (Claude Code also auto-reads `CLAUDE.md`, which just points here). Usage knowledge goes in the matching skill. Per-crate caveats go in that crate's `README.md`. Design rationale and measured verdicts go in `research/` — or, for the rationale behind a specific deferred task, in that bead's description / `--design` field.
- **Do NOT commit narrative scratch docs.** No `HANDOVER*.md`, no `SESSION*.md`, no "current state" / "what I'm doing now" / progress-log markdown in the repo. Session and orchestration state belongs in beads (for work) or in your own un-tracked notes — never in a tracked file. The only living operational markdown allowed is **genuine reference** (a runbook, the benchmark catalog) and **generated reports** (the CI-published perf/conformance data) — not a story about a session.
- **No hard-coded performance numbers in markdown** (restated; see the section above): cite the generated structured data, not a baked-in figure.
- **RDF/SPARQL terminology → match the W3C specs.** Before writing or editing any doc that names an RDF/SPARQL feature, check [`skills/terminology/SKILL.md`](./skills/terminology/SKILL.md) — the single source of truth for preferred wording (say **RDF 1.2** / **SPARQL 1.2** and **triple term** / **reifier** / **reified triple**, never the community-era "RDF-star" / "RDF\*" / "SPARQL-star" / "quoted triple" / "embedded triple"). Enforced by the `terminology` HARD gate (`scripts/check-terminology.py` in `docs-quality.yml`); a hit fails the build unless the line is a legitimate proper-noun / paper-title / third-party-doc / URL mention or carries an inline `terminology-allow: <why>` marker.
- **Banned terms are DATA, and they are checked in CODE + VOCABULARY too.** The banned list lives in [`scripts/banned-terminology.json`](./scripts/banned-terminology.json) — adding a maintainer-banned term is **one object**, not a code change — and each term declares its own file surface (`.rs` / `.ttl` / `.md` / `.typ` / manifests / workflows, not just markdown). This exists because a banned term once reached a merge-ready PR as a `pub` Rust type **and** a published `rdfs:comment` with a fully green `ci-summary / gate`, since the gate then scanned `*.md` only (issue #3811). Escapes are narrow and reviewed: the inline `terminology-allow: <why>` marker, per-term proper-noun patterns, and an **enumerated** `exemptPaths` list where every entry carries a `why`. Widening a path exclusion to make a violation pass defeats the gate — reword instead. `scripts/tests/test_banned_terminology.py` pins all of it (fixtures + the workflow wiring).

Honour these homes and the repo never accumulates stale TODO lists or handover docs — no clean-up pass is ever needed.

## Public-API → SKILL.md maintenance rule

Important enough to state twice: see **MAINTENANCE RULE (REQUIRED)** near the top. In short — when you change any public API (`pub` item, CLI flag, HTTP route, Python/JS binding), update the corresponding `skills/<surface>/SKILL.md` in the SAME change. The surface→skill map is in [`skills/SKILL.md`](./skills/SKILL.md).
