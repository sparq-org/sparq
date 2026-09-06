# PROVENANCE — `secprop-ext.ttl`

> 🤖 **SPARQ agent** [OPUS-5] — provenance pointer for the sparq-authored
> `sec-prop:` extension graph that lives in this crate.

`secprop-ext.ttl` is a **sparq-authored EXTENSION** (bead `sq-5oru9`, issue #1001)
of the vendored `sec-prop:` namespace `https://w3id.org/zkp-sparql/sec-prop#` — the
companion ontology to the **ISWC 2025** ZKP-SPARQL paper. It **extends** that
namespace; it does not fork it (design record
`research/security-properties-ontology-design.md` §4.1).

It is **ours to edit** — unlike the verbatim vendored files, whose byte-for-byte
fidelity to origin SHA `0fe80ea7d858de9f02bd29df29f6e50cdada14a0` is the provenance
guarantee.

## Where the vendored source lives

The vendored `sparql-zkp-ontologies` copy this file extends — `vocab/*.yaml.ld`,
`shapes/*.shapes.ttl`, `sigimpl-ext.ttl`, the origin/authorship/licensing record, and
the vendored-copy edit policy — stays in
[`crates/sparq-trust/ontologies/zkp-sparql/`](../../sparq-trust/ontologies/zkp-sparql/).
Read [that `PROVENANCE.md`](../../sparq-trust/ontologies/zkp-sparql/PROVENANCE.md)
for origin, authorship (Jesse Wright and the ISWC 2025 co-authors), the maintainer's
2026-06-20 publication decision, and the MIT license grant — all of which govern this
file's namespace too.

## Why it moved here (issue #3705)

The Rust constants that name every term in this file, and the drift test that pins
the two together, now live in this dependency-free leaf crate so `sparq-trust`,
`sparq-policy`, and `sparq-zk` can share one copy instead of three. The Turtle moved
with its constants: a vocabulary and the test that pins it to its constants belong in
the same crate, and keeping them apart would have left a cross-package `include_str!`
— exactly the `cargo package` breakage #3705 removed.

License: MIT (see
[`crates/sparq-trust/ontologies/zkp-sparql/LICENSE`](../../sparq-trust/ontologies/zkp-sparql/LICENSE)).
