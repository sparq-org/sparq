# Native dependency policy reconciliation

[GPT-6] This record applies the reviewed policy inheritance to the detached
native workspace. It is a dependency-policy change, not a source audit of the
remaining packages, an external cryptographic review, or permission to merge.
The native implementation, its lockfile and its proof evidence are unchanged.

## Preserved decisions and exact additions

The policy combines the foundation's existing records with reviewed SDK commit
`2a67f00d60627100709304036d0a0a93c75a6d61` and the already accepted main
security-remediation records. The root lock update is the reviewed four-package
TLS delta from `6317c88015e186a0d8106de4f69525aacbc33c54`. Its four existing
exemptions retain their original attribution and limitations; the AWS-LC
vendored C and assembly are not represented as a local full audit.

All existing configuration, audit, exemption, publisher and import records are
retained. The inherited SDK backlog exemptions and actual source-delta audits
remain separate in the [unchanged historical record](../zk-dependency-backlog/README.md).
That history describes its original SDK graph, not execution of this native graph.
This native change creates no exemption, local source audit or new trust source.

The [trusted chains](trusted-chains.json) add sixteen already published records
from the six configured sources, covering seven exact native package versions.
The [source manifest](trusted-SOURCES.json) pins their upstream commits and hashes.
Existing criteria mappings remain unchanged. A source's review of an upstream
release does not attest the native Ark patch: its `audit-as-crates-io` mapping
retains the upstream obligation, while the
[patch provenance](../../zk/native-composition/vendor-support/README.md) is separate.

Fourteen package-and-version-specific license entries use the existing exception
format. The [license evidence](license-package-evidence.json) records exact
registry checksums and license text, including VCS-pinned text where an archive
omitted it. The applicable notices are retained in `licenses/`. The global
license allowlist is unchanged. Historical screening records retain their
original proposed status; application and gate results are recorded separately.

## Advisory explanation correction

The advisory ignore identifiers and enforcement are unchanged. One explanatory
value is intentionally superseded: the foundation's bincode reason described
only the HDT path and did not account for SDK deserialization paths. The replacement
is the exact previously reviewed SDK wording. The
[integration record](integration.json) retains both values and their source.

The companion VEX correction comes from
`a816cf6b0e3bf7ed700f749a08240da3e22a552b`. It removes the unsupported
`not_affected` / `code_not_reachable` assertion for bincode and preserves every
sibling advisory. Its documentation link points here because this branch does
not contain the later SDK gate document. The drift checker changes only its
explanatory comment.

The existing bincode exception concerns maintenance status. The reviewed SDK
scope includes HDT's qwt dependency and the detached evaluator host's
`risc0-zkvm 3.0.6/client` dependency. Its nested guest omits bincode. The host's
ordinary `ExternalProver` response uses protobuf, while other SDK RPC, asset and
storage paths use bincode. These observations do not establish an exploitability
verdict or general code unreachability. The exact reviewed SDK registry release
names VCS commit `8c215e2f4ccdd935f0517bf05d90f1ae032840a9`; this explanation
does not assert that the SDK runtime is introduced by the native policy change.

## Validation and remaining failures

The [candidate validation record](validation.json) binds the candidate's own
manifests, lockfiles and policy. Cached, frozen metadata checks pass root vet and
root deny, and pass the native bans, sources and licenses checks. Native vet
still fails for the exact [134-unit inventory](residual-134.json). Native
advisories still fail for `derivative 2.2.0` (`RUSTSEC-2024-0388`) and
`proc-macro-error2 2.0.1` (`RUSTSEC-2026-0173`). No new ignore masks them.
An imported source review of a package does not dismiss its maintenance notice.

These checks ran without fetching, compiling, running build scripts or proving.
The advisory check uses a pinned, already cached database, so a fresh hosted
advisory check remains required. A first default-cache advisory attempt failed
to acquire a read-only lock; the separate successful analysis uses the same
policy with only the cache-directory path changed. Neither attempt is omitted.
The all-feature, all-target resolved graph is not evidence that every target or
executable was run.

The original private proposal also retained an initial cargo-vet formatting
failure. Its later post-format report and the separately source-bound current
candidate run support the residual count; the formatting failure does not.
Those distinct receipts and their hashes are identified in `validation.json`.
The remaining native audit coverage and maintenance failures prevent a clean
dependency-gate claim.
