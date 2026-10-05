# sparq v0.1.1 — incomplete bootstrap

[GPT-6] Historical status corrected on 2026-09-12. The immutable tag identifies
commit `1a63aa7c638bd80da55f1811d5fb97e8d014f631`; it is not a complete release.

The source snapshot contains the experimental RDF store, SPARQL engine, inference,
bindings, and server described in its [changelog](../CHANGELOG.md#011---2026-08-31).
The tag-time release workflow failed at startup. No GitHub Release, crates.io
packages, or PyPI `sparq-rdf` distributions were published. The two npm versions
`@sparq-org/sparq@0.1.1` and `@sparq-org/solid-server@0.1.1` exist without
`dist.attestations`; these immutable versions cannot be republished to add provenance.

Preserve `v0.1.1` and the earlier incomplete `v0.1.0` tag. Packaging, source-integrity,
and engine fixes landed later and belong to [v0.1.2](release-notes-v0.1.2.md), not
to this source snapshot. Follow the [recovery runbook](release.md) rather than
building newer source under an old release name.
