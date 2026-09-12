# Dependency gates for the detached evaluator

[GPT-6] The evaluator and its nested guest have independent lockfiles. The root
workspace's cargo-deny, cargo-vet and SBOM run cannot cover them implicitly.
`scripts/rust-dependency-graphs.py` enumerates all three manifests for the existing
supply-chain gate and daily advisory watchdog. Changes to nested lockfiles or SDK
patch provenance trigger the Rust dependency checks.

This inventory covers the root workspace, evaluator workspace and nested guest.
Other detached tools and examples with their own lockfiles are outside this
change and remain a separate dependency-coverage follow-up.

Every graph uses the root deny policy. Vet checks use the root audit store and pin
both its imported attestations (`--locked`) and the Cargo lockfile
(`--cargo-arg=--locked`). `--frozen` prevents network refresh during verification.
Missing attestations are failures, not exemptions inferred from successful builds.
Audit refresh and new audit records remain explicit reviewed policy changes.
The deny policy path is absolute; callers' current directories cannot select a
different file. Cargo-deny 0.20.2 accepts `--config` before `check`, as exercised
by the real three-graph helper smoke. This is version-specific CLI evidence.

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

## Existing bincode maintenance exception

[GPT-6] The existing `RUSTSEC-2025-0141` ignore remains a maintenance-only
exception. The [primary RustSec advisory](https://rustsec.org/advisories/RUSTSEC-2025-0141.html)
reports that bincode is unmaintained and has no patched version; it does not
report a vulnerability. This does not establish that every use is safe.

The locked `bincode 1.3.3` paths include `sparq-hdt -> hdt 0.7.3 -> qwt 0.4.0`
and the detached evaluator host's `risc0-zkvm 3.0.6/client` feature. The nested
guest graph has no bincode. This was checked against the locked metadata and
the exact registry source, rather than inferred from the prior HDT-only note.

Sparq's host explicitly constructs `ExternalProver`. Its ordinary `prove` response
passes through `ApiClient::prove` and protobuf decoding. The same SDK's
`DefaultProver` uses bincode RPC, API asset conversions serialize receipts with
bincode, and prover server/session paths deserialize assets or stored segments.
The reviewed SDK source is the registry release whose VCS metadata names commit
`8c215e2f4ccdd935f0517bf05d90f1ae032840a9` (`risc0/zkvm`), including
`src/host/client/prove/{external,default}.rs`, `src/host/api/{client,convert,server}.rs`
and `src/host/server/session.rs`. These paths do not establish an attacker-controlled
bincode input in Sparq's normal presentation verifier, but they do invalidate a
blanket statement that the dependency is only internal qwt code or unreachable.

The VEX entry therefore preserves the advisory and its scope while omitting the
unsupported `not_affected` / `code_not_reachable` analysis. No new advisory
exception, exploitability finding or trust in an arbitrary prover process is
introduced. Revisit this exact dependency at SDK/HDT upgrades and remove both the
ignore and VEX entry when all affected paths replace it. The dependency review
backlog is tracked under `zkp-10.8.1`; its entries are not completed source audits.
