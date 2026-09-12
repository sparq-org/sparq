# Dependency gates for the detached evaluator

[GPT-6] The evaluator and its nested guest have independent lockfiles. The root
workspace's cargo-deny, cargo-vet and SBOM run cannot cover them implicitly.
`scripts/rust-dependency-graphs.py` enumerates all three manifests for the existing
supply-chain gate and daily advisory watchdog. Changes to nested lockfiles or SDK
patch provenance trigger the Rust dependency checks.

Every graph uses the root deny policy. Vet checks use the root audit store and pin
both its imported attestations (`--locked`) and the Cargo lockfile
(`--cargo-arg=--locked`). `--frozen` prevents network refresh during verification.
Missing attestations are failures, not exemptions inferred from successful builds.
Audit refresh and new audit records remain explicit reviewed policy changes.

Each vendored SDK package must explicitly opt into `audit-as-crates-io` in the vet
policy. This establishes the upstream registry release's review obligation; it
does not attest the patched source. The independent source review must additionally
cover the exact delta in `vendor/zk-sdk/UPSTREAM.json` and its patch files. The
provenance checker verifies file inventories and hashes; synthetic API/feature
checks exercise the modified dependency configuration. None is a cryptographic
audit, nor a substitute for a genuine final-artifact guest proof.

CycloneDX generation includes all features, target-specific dependencies and build
dependencies for each graph. Generation runs with network disabled after locked
fetch, and any lockfile change fails the gate. Every workspace member must produce
its own correctly identified SBOM before normalization and the existing purl and
supplier assertions. The required inventory is written by a checked command, so
an error cannot disappear inside shell process substitution. SDK provenance and
patch files accompany the SBOM upload: a canonical registry purl alone does not
identify the modified bytes.

The watchdog collects each graph's report and fails if any graph fails, even if
its siblings pass. The permanent gates do not lower audit criteria, add license
exceptions, ignore RustSec findings, or treat a local path crate as automatically
audited because it builds.
