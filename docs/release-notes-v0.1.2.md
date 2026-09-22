# sparq v0.1.2 — exact-source recovery

> [GPT-6] **Historical incomplete release attempt.** The immutable v0.1.2 tag
> published both container lanes, but macOS GUI staging failed and the alias gate
> blocked the GitHub Release. Do not use the publication instructions below to retry
> that failure. The staging fix is merged; the next candidate is
> [v0.1.3](release-notes-v0.1.3.md). Preserve the old tag and existing containers.

[GPT-6] Prepared 2026-09-20. This is the next complete-release candidate after the
[incomplete v0.1.1 bootstrap](release-notes-v0.1.1.md); publication is not implied
by this document. All artifacts must come from the same immutable `v0.1.2` tag.

Since v0.1.1, nested queries correctly restore outer budgets, predicate statistics
serialize deterministically, and scheduler test cleanup is panic-safe. Bind joins
reuse validated sorted groups and bounded right-hand scans; core updates preserve
unchanged numeric memo state. Experimental deletion-projection caching remains
default-off with documented cold-read and memory costs. The private native LWS
server adds same-pod, fail-closed WAC group membership resolution.

Release fixes align SLSA permissions, validate the Cargo bootstrap dependency order,
and require tag/build/manifest identity before release or package jobs. The workspace,
Python distribution, and all three public npm manifests now target `0.1.2`.
`@sparq-org/eyereasoner-compat` skips `0.1.1`; private `sparq-lws-core` keeps its
independent Cargo version. See the [changelog](../CHANGELOG.md#012---2026-09-20)
for the post-tag changes and the [release runbook](release.md) for the exact order.

After the corresponding publications and attestations have been verified:

```sh
cargo install sparq-cli --version 0.1.2
npm install @sparq-org/sparq@0.1.2
npm install @sparq-org/solid-server@0.1.2
npm install @sparq-org/eyereasoner-compat@0.1.2
python -m pip install sparq-rdf==0.1.2
docker pull ghcr.io/sparq-org/sparq-server:0.1.2
```

APIs remain experimental and unstable. The Solid npm host is for local development,
not production deployment. Crates.io builds use upstream `spargebra` rather than the
repository's vendored parser fixes; see `vendor/spargebra/SPARQ-PATCHES.md`. ZK/MPC
capabilities remain research-grade, with external cryptographer sign-off pending.
New attestations do not retroactively cover the published npm `0.1.1` bytes.
