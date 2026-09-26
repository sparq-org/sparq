# Exact dependency review backlog

[GPT-6] The [accepted package dispositions](accepted-6f7/per-entry-dispositions.json)
record 143 individually screened, exact-version exceptions under the repository's
existing cargo-vet review-backlog policy. They are **not completed source audits**.
The independent reviewer was GPT-6 in a separate task from the proposal author.
That reviewer authored SDK feature patches; this proposal review does not approve
those patches. Their separate source reviews and provenance remain required.

The [frozen proposal](accepted-6f7/proposal-manifest.json), per-entry dependency
paths, package risks and [artifact hashes](accepted-6f7/recorded-artifacts.json)
are retained with the exact [review text](accepted-6f7/review-final.txt). The
original proposal-only and policy-not-applied fields describe their historical
review state. Applied entries in `supply-chain/config.toml` identify the actual
approval and tracking item `zkp-10.8.1`; they preserve the old root exemptions.
Three old same-version `safe-to-run` entries remain alongside separately approved
`safe-to-deploy` backlog entries, retaining their original scope and notes.

Version changes require renewed screening. Remove a backlog entry only after
matching accepted audit evidence, a reviewed replacement with its own valid
coverage, or verified dependency removal. Cohorts are nonexclusive: build and
procedural-macro packages can also contain runtime parsing, memory, FFI and
filesystem code. Resolved target-conditioned edges do not prove execution on
every host or in a guest.

The eight separately reviewed registry deltas and two Serde prerequisite deltas
are not covered by these exceptions. Imported audit-chain reconciliation, all
three actual vet checks, fresh advisory checks, source/license integrity, SBOM
validation and vendor feature checks remain separate requirements. Passing these
policy checks would not constitute external cryptographic assurance.

The [ten recorded delta reviews](reviewed-deltas/recorded-reviews.json) preserve
eight actual Claude Fable 5.1 source reviews and two separate GPT-6 Serde reviews.
Their original source manifests, complete patches, review text and hashes remain
available beside the records. GPT-6 assembled the cargo-vet entries; the model
names identify the actual reviewers. Existing baseline chains remain necessary,
and these reviews do not certify unrelated transitive packages or cryptography.

The [import reconciliation](import-reconciliation/record-reconciliation.json)
retains every original imported audit and wildcard record under the same six
configured sources. Live source records were compared before restoring records
omitted by the earlier graph-filtered refresh. Five publisher facts retain their
earlier cargo-vet refresh provenance: the attempted fresh crates.io API check
returned HTTP 403, so this is not a fresh publisher verification claim. No new
publisher trust, criterion or advisory exception was introduced.
