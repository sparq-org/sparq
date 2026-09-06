<!-- [OPUS-5] sq-3705: internal-stub README for a publish=false crate. -->
# sparq-secprop-vocab

The shared `sec-prop:`/`secx:` security-property IRI constants, the canonical
`ontologies/secprop-ext.ttl`, and the single TTL↔constant drift test — in one
**dependency-free leaf** crate (issue #3705).

`sparq-trust` owns the vocabulary but depends on `sparq-zk`, so `sparq-zk` could not
import it (a cycle) and `sparq-policy` could not either without giving up its lean
graph. All three kept their own copy, pinned by a cross-package `include_str!` that
also broke `cargo package` for `sparq-zk`. This crate is the leaf below all three:
zero `[dependencies]`, so each takes the edge behind its own default-OFF feature and
its shipping graph is unchanged.

It **records** security-property claims and their epistemic basis; it asserts none.
sparq's ZK estate is research-grade and **externally UNAUDITED** (`sq-qhy4`) — the
default assurance is `secx:Claimed` and `secx:Proven` is barred on a positive
property while that gate is open.

See `src/lib.rs`, `ontologies/PROVENANCE.md`, and design record
`research/security-properties-ontology-design.md` §4.1.

**Internal tooling — not published** (`publish = false`).

License: MIT
