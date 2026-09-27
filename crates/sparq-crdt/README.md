# sparq-crdt

<!-- [FABLE-5] sq-tag1q.7.2 + [OPUS-5] sq-tag1q.7.4: internal-stub README for a publish = false crate. -->

Opt-in SPARQL-CRDT replication primitives for sparq (epic `sq-tag1q.7`), in two
independent layers:

- **Codec + journal** (`sq-tag1q.7.2`) — bounded canonical delta-envelope codec,
  causal summaries (version-vector clock + dot cloud), the durable append/recovery
  journal with atomic snapshots, and sync primitives (hello handshake,
  missing-interval computation, membership-epoch + causal-stability tracking).
- **Executable formal model + convergence harness** (`sq-tag1q.7.4`) — the exact
  dotted-set join equations (`CRDT-JOIN-1`), clock/cloud causal contexts
  (`CRDT-CTX-1`), evaluate-at-origin update compilation (`CRDT-MUT-*`/`CRDT-UPD-*`),
  a bounded exhaustive multi-replica model checker, and generated schedules that
  permute, duplicate, batch, snapshot, compact, and replay deltas.

Design: `research/sparql-crdt-gpt56-2026-07.md`; proposal draft
`site/specs/sparql-crdt.typ`. A partial surface still: the model is the specification
the production dot-store must be differentially tested against, not that implementation,
so the crate stays `publish = false` and claims **no** conformance class yet. Nothing in
the workspace depends on it: core builds and bundles carry zero CRDT code.

**Evidence, not proof.** The bounded model check is exhaustive only within its stated
bounds; the generated-schedule tests are sampled evidence. No convergence proof is
claimed — the proposal's semilattice argument stays an open, unreviewed obligation.

API detail lives in rustdoc: `cargo doc -p sparq-crdt --open`.

License: MIT — see the workspace [LICENSE](../../LICENSE).
