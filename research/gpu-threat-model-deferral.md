<!-- [OPUS-5] sq-vrye: deferral RECORD for the sparq-gpu threat model (not the model itself).
     Referenced from research/threat-model.md's out-of-scope table; enforced by
     crates/sparq-gpu/tests/deferral_premise.rs. -->
# `sparq-gpu` threat model — deferral record and exit trigger (sq-vrye)

**Status: DEFERRED, conditionally and mechanically.** No threat model for
`sparq-gpu` exists, and none is written here. What this record does is make the
deferral *legible and self-firing*: it states the evidence that makes deferring
safe **today**, names the exact conditions that end the deferral, wires those
conditions to a test that fails when they are met, and pre-scopes what the model
must cover so its future author does not start from a blank page.

Bead **sq-vrye**, parented under the STRIDE core model
([`threat-model.md`](threat-model.md), bead `sq-o9u4`), which lists `sparq-gpu`
as out of scope. Companion verdict: [`gpu-verdict.md`](gpu-verdict.md) (T24d —
**PARK**).

## 1. What this document is and is not

- **Is:** the justification, exit trigger, and enforcement for *not* modelling
  `sparq-gpu` yet, plus a scoping outline for the eventual model.
- **Is not:** a threat model. Nothing below asserts that any threat is
  *mitigated*. Every item in §6 is a **candidate threat identified by reading
  the code**, unassessed for likelihood or severity — do not cite one as an
  audit finding, and do not cite this document as evidence that `sparq-gpu` is
  safe for any use.

## 2. Why deferring is defensible today

The deferral rests on the crate being unreachable from any adversary, not on it
being benign. Each row is a checkable fact, not a judgement:

| Fact | Evidence |
|---|---|
| Not published; ships in no release artifact | `publish = false` in `crates/sparq-gpu/Cargo.toml`; listed among the unpublished crates in [`docs/release.md`](../docs/release.md) |
| Nothing in the workspace depends on it | no dependency resolving to `sparq-gpu` in any other `crates/*/Cargo.toml`, under that name or a rename; asserted by `crates/sparq-gpu/tests/deferral_premise.rs` |
| No path from untrusted input | it has no `sparq-core`/`sparq-engine` dependency at all, so no query, RDF byte, or HTTP request can reach a kernel; its only callers are its own tests and `examples/gpu_bench.rs` |
| Never enters the wasm build | `wgpu` appears in exactly one manifest in the workspace (`crates/sparq-gpu/Cargo.toml`) |
| No `unsafe` of its own | `#![forbid(unsafe_code)]` in `crates/sparq-gpu/src/lib.rs` |
| Parked, not in-flight | [`gpu-verdict.md`](gpu-verdict.md) §5 — PARK, with named re-open conditions |

A threat model written against this shape would have an empty attacker column:
the only actor who can invoke a kernel is a developer who has already checked
out the repository and typed `cargo run`. That is why the work is deferred
rather than merely unscheduled — the model would be vacuous *and* would have to
be rewritten from scratch the moment the crate is wired in, because wiring it in
is precisely what creates the boundary worth modelling.

## 3. What the deferral does *not* cover

Two exposures exist **today** and are not deferred by this record. Neither is a
reason to write the model now; both are reasons not to read §2 as "no risk".

1. **Supply chain.** The workspace declares no `default-members`, so a bare
   `cargo build` / `cargo test` at the root builds `sparq-gpu` — and with it the
   `wgpu`/`naga` tree — just as `--workspace` does. Ten crates in that tree
   (`wgpu`, `wgpu-core`, the three `wgpu-core-deps-*`, `wgpu-hal`,
   `wgpu-naga-bridge`, `wgpu-types`, `naga`, `naga-types`) carry
   `safe-to-deploy` **exemptions** — i.e. are accepted without an audit record — in
   [`supply-chain/config.toml`](../supply-chain/config.toml). The exposure is to
   the *build and developer* environment, not to any shipped artifact, and it is
   governed by the existing `cargo-deny`/`cargo-vet` lanes rather than by this
   bead.
2. **Assurance signal.** The crate's correctness tests **skip-pass** when no
   adapter is present (`Gpu::new()` returns `None`), which is the normal case on
   CI; it carries a coverage floor of `0` (`bench/coverage-floor.json`, note in
   `scripts/coverage-gate.py`) and is excluded from mutation testing
   (`.cargo/mutants.toml`). So the kernels are, in practice, **unverified by the
   per-commit gates** — only by a manual run on a machine with a GPU. Any exit
   from the prototype stage has to fix this *before* the threat model means
   anything, because a model's mitigations would be claimed against tests that
   do not run.

## 4. Exit trigger — what ends the deferral

The deferral ends when **any one** of these becomes true. (a) and (b) are
mechanically enforced (§5); (c) is a consequence of (a) worth stating separately
because it is the security-relevant one; (d) is a human judgement call.

- **(a) Integration.** Any workspace crate takes a dependency on `sparq-gpu`.
- **(b) Publication.** `publish = false` is removed, or the crate otherwise
  enters a release artifact.
- **(c) Reachability.** A kernel becomes reachable from input the operator does
  not control — a query, an RDF document, an HTTP request.
- **(d) Re-opened for integration.** T24d is re-opened under
  [`gpu-verdict.md`](gpu-verdict.md) §5 (discrete high-bandwidth GPU; an engine
  residency cache landing for another reason; in-browser WebGPU) **and** the
  re-opened work targets wiring rather than re-measurement. Re-running the
  existing harness on new hardware is measurement and does *not* trigger this.

When the trigger fires, the required output is a threat model at
`research/gpu-threat-model.md` scoped by §6, plus: moving the `sparq-gpu` row in
[`threat-model.md`](threat-model.md) out of the out-of-scope table into a modelled
boundary, and — if the GPU path can hold personal data — a row in
[`compliance/threat-model.md`](../compliance/threat-model.md).

## 5. Enforcement

`crates/sparq-gpu/tests/deferral_premise.rs` asserts (a) and (b) directly: it
reads the crate's own manifest for a `publish` key whose value is the literal
boolean `false`, and every other `crates/*/Cargo.toml` for a dependency that
resolves to `sparq-gpu`, then fails pointing at this document.

It parses those manifests rather than grepping them, because a substring search
misses the two quiet ways the premise breaks. `publish = ["false"]` is a registry
allow-list naming a registry called `false` and is *publishable*, yet it contains
the word. And a member that inherits a renamed workspace dependency —
`gpu.workspace = true` against a root `gpu = { package = "sparq-gpu" }` — is a
real dependency edge whose manifest never spells `sparq-gpu`; the test therefore
resolves aliases out of the root `[workspace.dependencies]` table first. Both
cases are pinned by their own regression tests in the same file.

It is an ordinary `cargo test -p sparq-gpu` test, so the PR that wires
the GPU in cannot merge without either landing the threat model or consciously
editing away the trip-wire — which is a visible diff, not a silent lapse.

This is the whole point of the record. A deferral held only in a bead's prose
expires silently; one held in a test expires loudly, in the PR that invalidates it.

## 6. Pre-scoped outline for the eventual model

What follows is the *agenda* for the future model — code-cited starting points
so its author inherits the reading already done. Severity and likelihood are
deliberately absent: assessing them is the deferred work.

### 6.1 New assets and the new trust boundary

Wiring a GPU in adds a boundary the STRIDE core model has no analogue for: the
engine hands data and a computation to a **vendor GPU driver loaded into the host
process** — frequently closed-source, and reached through `wgpu`'s shader
compiler rather than through code sparq reviews — and then trusts the answer
verbatim. Call it B-GPU. New assets:
correctness of a device-computed result (it becomes a query answer), the
confidentiality of dataset values resident in device memory, and the
availability of the host's GPU — which the sparq process shares with everything
else on the machine, including the user's display server.

### 6.2 Candidate threats (unassessed)

| # | Candidate | Where |
|---|---|---|
| G-1 | `group_aggregate` validates `groups <= MAX_GROUPS` and `keys.len == vals.len` but **never validates that each key is `< groups`**; the WGSL indexes workgroup-shared arrays with the key unchecked, so out-of-range keys are contained only by the shader compiler's bounds-check policy, not by any sparq code. Wired in, those keys would derive from dictionary ids. | `crates/sparq-gpu/src/lib.rs` (`group_aggregate`, `WGSL_GROUP_AGG`) |
| G-2 | The WGSL probe loop terminates only if the resident table contains an empty slot. `cpu::build_hash_table` guarantees that (load factor ≤ 0.5), but `upload_table` is public and checks the power-of-two shape with a `debug_assert!` only — so a release build accepts a hand-built full table, and the kernel then loops forever: a device hang / driver reset affecting the whole host, not just the query. | `crates/sparq-gpu/src/lib.rs` (`upload_table`, `WGSL_HASH_PROBE`) |
| G-3 | `upload_u32`/`upload_f64` record length as `data.len() as u32` — a silent truncation above `u32::MAX` elements — and never consult `max_storage_bytes`, which the crate exposes publicly but does not itself enforce. | `crates/sparq-gpu/src/lib.rs` (`upload_u32`, `upload_f64`) |
| G-4 | Integrity of a device-computed answer: a driver bug, a miscompiled shader, or a hardware fault yields a *wrong query result*, not a crash. The only cross-check that exists is the CPU reference in the crate's own tests, which do not run without an adapter (§3.2). What second-source check, if any, a wired-in backend owes is an open design question. | `crates/sparq-gpu/tests/correctness.rs`; `src/cpu.rs` |
| G-5 | Residency as data lifetime: a resident-column cache — the only configuration the verdict leaves discussable — means dataset values persist in device memory with nothing wiping the buffers on drop and no isolation from other processes sharing the GPU. This has no equivalent in the CPU path. | design-level; `ColU32`/`ColF64`/`HashTable` |
| G-6 | Availability: an unbounded or long dispatch is not covered by `QueryBudget`, and the readback path blocks the calling thread on the device indefinitely (`PollType::wait_indefinitely()`), so a wedged device wedges a server thread. | `crates/sparq-gpu/src/lib.rs` (`run`) |
| G-7 | Supply chain at *ship* time: §3.1's exposure changes character the moment `wgpu` enters a published artifact, and would need the vet exemptions discharged rather than carried. | `supply-chain/config.toml` |

### 6.3 Method note

The eventual model should follow [`threat-model.md`](threat-model.md)'s form —
STRIDE per boundary, every mitigation cited to a test or gate, every gap stated
plainly and mapped to a bead — and must not claim a mitigation backed by a test
that skip-passes on the CI that would gate it (§3.2).

## 7. Open questions for the maintainer

1. **Is (d) the right human trigger?** It reads "re-opened *for integration*",
   deliberately letting a re-measurement on new hardware proceed without a
   threat model. If re-measuring on an untrusted or shared machine (a cloud GPU
   instance) should also trigger the model, (d) needs widening.
2. **Should the trip-wire be broader than the crate's own test suite?** It fires
   under `cargo test -p sparq-gpu`, which runs in the workspace test lane. A
   `scripts/`-level gate would also catch a PR that deletes the crate's tests
   wholesale; a crate-local test was chosen as the smaller change.
3. **§3's two live exposures** are recorded here but owned by nobody. If either
   deserves its own bead — particularly §3.2, since it silently weakens the
   assurance story for a crate that is otherwise well-tested on paper — that is
   a maintainer call, not something this record should decide.

## 8. Honesty notes

- No threat here has been assessed, ranked, or mitigated. §6.2 is a reading
  list, not findings.
- §2 justifies deferral by *unreachability*. It says nothing about whether the
  kernels are correct or the crate is safe to use, and the assurance gaps in §3.2
  mean the honest answer to "are the kernels correct?" is "verified by a manual
  run on one machine, on one date, per [`gpu-verdict.md`](gpu-verdict.md)".
- No performance figures are restated here; the measured tables and their
  provenance live in [`gpu-verdict.md`](gpu-verdict.md).
