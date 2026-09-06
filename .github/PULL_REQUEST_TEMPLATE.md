<!-- [OPUS-4.8] Governance: PR template (bead sq-rau7). Checklist tied to AGENTS.md's post-batch re-evaluation table. -->
## Summary

<!-- What does this change do, and why? Link any related bead id(s), e.g. sq-xxxx. -->

## Base gate (always required)

- [ ] `cargo build --workspace` succeeds.
- [ ] `cargo clippy --workspace --exclude sparq-py --all-targets -- -D warnings` is clean (run over the **full workspace** — feature unification surfaces lints a single-crate check misses).
- [ ] The code **this PR touches** is formatted (matching the surrounding committed style). `cargo fmt --all --check` is informational, not gating — it reports pre-existing diffs in untouched files until the deferred one-time workspace reformat lands (see `rustfmt.toml`), so do not run `cargo fmt --all` to clear it.
- [ ] `cargo test` passes for every crate this PR touches.

## Targeted re-evaluation (check the rows that apply to your change)

> See the **"Post-batch re-evaluation checklist"** in [`AGENTS.md`](../AGENTS.md) — re-run only the evaluations whose inputs your change touched.

- [ ] **Parser** (turtle/nt/nq/trig, `sparq-core` parse, `spargebra`): re-ran W3C SPARQL + rdf-turtle conformance, the chunked-vs-serial parser oracle, and `sparq-bench fuzz` (fixed regression windows now also CI-enforced per-PR by fuzz.yml's `differential smoke` job — sq-0iqzw).
- [ ] **Query execution / operators** (`sparq-engine` exec/optimizer): re-ran the full conformance ratchet, the operator-coverage bench, and the per-builtin error table.
- [ ] **Reasoner** (`sparq-reason`, rules, closure): re-ran the inference conformance ratchet, the incremental==batch property tests, and the LUBM entailed tier.
- [ ] **Public API** (`pub` item / CLI flag / HTTP route / Py/JS binding): updated the matching `skills/<surface>/SKILL.md` in **this** change, and re-ran that surface's tests. (REQUIRED — see the MAINTENANCE RULE in AGENTS.md.)
- [ ] **Wasm** (`sparq-wasm` / the wasm graph): ran `scripts/wasm-deps-guard.sh`, `wasm-pack test --node`, and the `wasm_bundle_bytes` size gate.
- [ ] **Cargo dependencies** (`Cargo.toml` / `Cargo.lock`): ran `cargo audit` + `cargo deny check` and regenerated the SBOM (supply-chain gate).
- [ ] **ZK verifier / circuits** (`sparq-zk`, `sparq-zk-compose`, `zk/`): ran `forge_gates` + `differential_fuzz` and the gate-count snapshot, and re-opened the soundness audit ([`research/zk-soundness-audit.md`](../research/zk-soundness-audit.md)).
- [ ] **SHACL** (`sparq-shacl`): re-ran the W3C SHACL conformance ratchet (core ≥ 98, sparql ≥ 5).
- [ ] **Storage / encoding** (`sparq-core` store/dict/compress, mmap, dict-spill): re-ran the deterministic perf-gate metrics, the byte-identity differentials, and coverage with `--features dict-spill`.
- [ ] **Anything merged**: the per-crate coverage ratchet + test-presence gate (`scripts/coverage*.py`) still pass.

## Ratchets and conventions

- [ ] I did **not lower** any conformance / perf / coverage ratchet (they only go up; documented spec divergences carry their rationale in the report).
- [ ] No hard-coded performance numbers added to markdown (cite generated structured data instead).
- [ ] Follow-up / discovered work is captured as **beads** (`bd create`), not as `TODO`/`FIXME` markers or a `TODO.md`.
- [ ] If this change makes a doc statement false (in either direction), I updated that doc in the same change.

## Security

- [ ] This change does not introduce a security regression. If it touches a security-sensitive surface (`sparq-zk*`, `sparq-mpc`, `sparq-core` unsafe, `sparq-server` network), I have noted the impact above. (Vulnerability reports go through the private channels in [`SECURITY.md`](../SECURITY.md), not a public PR.)
