> 🤖 **SPARQ agent** — I am @jeswr's agent for the sparq-org/sparq RDF/SPARQL engine. @jeswr runs multiple agents; this was written by the SPARQ agent, not the PSS agent (prod-solid-server).

## Summary

Repeated scans and estimates currently scan the complete deletion overlay to correct their counts. This adds the experimental `sparq-core/overlay-deleted-projections` Cargo feature, which caches a sorted deletion projection for each requested permutation and uses binary range counting on subsequent reads. The feature is **off by default**; default builds retain the original linear path and overlay representation.

Actual tombstone changes invalidate the cache. Insert-only and no-op batches retain it. The crate README and SPARQL query skill document explicit Rust opt-in, and the feature matrix gains a dedicated `test: true` configuration. A standalone `bench/overlay-count` harness makes the count and lifecycle comparisons reproducible. Addresses #4246; this does not establish a default-on optimization.

Cold first reads must sort and allocate; concurrent first readers wait for the same initializer. Forks copy warmed projections, and retained generations multiply their memory cost. Local generated measurements support keeping this experimental: repeated reads can amortize initialization, while cold/update-heavy use regresses and retained memory remains higher. There is no cache cap, eviction policy, universal crossover threshold, or canonical speedup claim.

## Review and validation

Implementation: GPT-6 Astra, extra-high reasoning. Actual independent Claude Opus 5, extra-high reasoning, reviewed the default-off implementation and the focused source correction at `6334b338587fe5c635c69a09e134917ec35eaaca`, approving progression to full validation with no source blockers. The correction preserves the original default function and tombstone statement verbatim so the existing neutral-tree proof can evaluate the feature-gated additions. This is not merge approval or default-on performance admission.

The original compiler protocol passed both obligations locally and in [Linux CI](https://github.com/sparq-org/sparq/actions/runs/34365605984/job/102513392880) on source commit `6334b338587fe5c635c69a09e134917ec35eaaca`: addition-neutral equals head, and deletion-neutral equals base. The correction also passes 13 targeted tests, six compiled negative controls and off/on all-targets core clippy.

Commit `fa3712df77e8acda4c447c6d0d4f87bc3376cc12` adds only `bench/feature-off-declarations/6469.json` from that Linux proof. Its generated evidence is unchanged; the reason string has exactly two edits requested by independent review: accurate GPT-6 Astra attribution and the actual CI job, measured source/base and toolchain scope. This records the proven metadata drift through the existing V2 protocol. Runtime source, gate code, workflow protections and ratchet floors are unchanged by that declaration commit; final-head protected CI remains required.

The subsequent review correction at `5757e70be09ed95bcbc2831cec7850fa29fdf620` changes only the default-off layout-test comment. It describes an empirical inline-footprint budget and removes an unsupported Rust layout guarantee. Actual independent Claude Opus 5, extra-high reasoning, approved this exact narrow correction. Both assertions and every executable line remain unchanged, verified mechanically. One compiled metadata-only negative control shows why this budget complements heap accounting. Compiler/target layout changes still require reassessment; no cross-target or scheduled-nightly coverage is claimed from that local control.

The benchmark-only allocator correction at `ccded1b4898cf5b317a6591f6ff6108ced23123c` reserves measurement-window ownership before counter reset and releases it after result capture. Existing allocation-counting boundaries, allocator hot-path code, core implementation, workloads and declaration are unchanged. A direct standard-library-only regression passes on the exact committed module; the unchanged old counter with the identical test reproduces the lost-counting bug and fails. Repeated clean calibration and targeted formatting checks pass. This detached-module test is documented separately and is not claimed covered by ordinary workspace CI. Actual independent Claude Opus 5, extra-high reasoning, reviewed this exact correction and approved progression to validation with no blocking findings. New-head protected CI remains required.

The source audit of the current harness found one coordinator for measurement windows, including the concurrent-reader scenario. This is a source-based assessment, not an execution trace of historical runs. Existing measurements remain frozen evidence for their original source hashes, with the same noncanonical and allocator-noise limitations; no new-head performance measurement is claimed.

Executed locally: 86 candidate Rust test executions across feature-off/on and compact configurations, four identical-main reference fixtures, and 105 feature-matrix assembly tests. Six compiled negative controls fail the intended assertions, including a forced-cache control that detects added heap in a feature-off build. Scoped core and harness clippy passed. Frozen manifests preserve the source, commands, raw generated measurement data and actual review output.

## Completed exact-head validation

Normal Linux CI completed successfully for `ccded1b4898cf5b317a6591f6ff6108ced23123c`. The [required aggregate gate](https://github.com/sparq-org/sparq/actions/runs/34378539115/job/102557363415) is successful on that exact head, produced by GitHub Actions integration 15368. The complete PR rollup has no failed or pending checks.

- [x] Full workspace build, default/all-feature lint and documentation gates pass in normal Linux CI.
- [x] Workspace test archive/shards and applicable conformance, storage, coverage and deterministic performance ratchets pass.
- [x] The [actual feature-matrix g02 job](https://github.com/sparq-org/sparq/actions/runs/34378539457/job/102560439383) executes build, tests and all-targets clippy for `sparq-core/overlay-deleted-projections`; all nine new feature tests pass. The selected-set artifact includes this test-enabled leg, and the report is bound to that same run and covers the selected set exactly.
- [x] Wasm dependency, execution and bundle-size checks, including feature-OFF artifact equality, pass.
- [x] The [Linux docs-quality job](https://github.com/sparq-org/sparq/actions/runs/34378539072/job/102557362642) passes `check-privacy-claims.sh` and both-direction privacy/performance honesty self-tests. This resolves the earlier local Bash 3 `mapfile` limitation through actual Linux execution; no check was waived.
- [x] Every inline review thread is resolved. Both concrete Copilot findings received reviewed fixes. The latest exact-head Copilot review has no new concrete findings.

The current PR body records successive reviews as source-bound historical evidence. The earlier validation-only verdicts were not merge-admission decisions. A separate final performance-discretion decision is recorded in the final admission comment. Normal merge-group checks remain required after enqueueing.

## Ratchets and conventions

- [x] No conformance, performance or coverage floor is lowered.
- [x] Markdown does not embed benchmark results; the harness emits structured data.
- [x] The new feature and its limitations are documented in the matching skill and crate README.
- [x] Separate operational discoveries are tracked in #6468 and kept out of this change.

Full-gate evidence comes from completed Linux CI. Local disk capacity and missing external corpora/tools still limit local checks; they are not presented as full-gate evidence. No release is requested.
