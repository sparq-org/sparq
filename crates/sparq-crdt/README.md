# sparq-crdt

<!-- sq-tag1q.7.2: internal-stub README for a publish = false crate. -->

Opt-in SPARQL-CRDT replication primitives for sparq (epic `sq-tag1q.7`): the
bounded **canonical delta-envelope codec**, **causal summaries** (version-vector
clock + dot cloud), the **durable append/recovery journal** with atomic
snapshots, and **sync primitives** (hello handshake, missing-interval
computation, membership-epoch + causal-stability frontier tracking).

Design: `research/sparql-crdt-gpt56-2026-07.md`; proposal draft:
`site/specs/sparql-crdt.typ`. This crate is an intentional partial surface —
the replicated dot-store state/merge algebra and the evaluate-at-origin SPARQL
Update compiler are sibling beads — so it stays `publish = false` and claims
**no** conformance class from the proposal yet. Nothing in the workspace
depends on it: core builds and bundles carry zero CRDT code.

API detail lives in rustdoc: `cargo doc -p sparq-crdt --open`.

License: MIT — see the workspace [LICENSE](../../LICENSE).
