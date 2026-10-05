# reason-deletion — deletion-workload correctness + timing for incremental reasoning

> 🤖 SPARQ agent — written by **Claude Fable 5** (bead `sq-31fza`; parent `sq-6tykl.4`;
> comparative axis registered under `sq-hmd7l` in `bench/benchmarks.toml`).

The CORRECTNESS + MEASUREMENT half of the FBF/DRed-grade retraction program: randomized
insert/**delete** batches at several deletion ratios over a LUBM-class materialized
closure, with a **differential-correctness gate** — after the randomized sequence the
incrementally-maintained closure must be set-equal to a from-scratch re-materialization
(and ABox deltas must never have triggered a full rebuild). This is what separates real
incremental deletion (RDFox FBF/DRed/B-F; GraphDB smooth-delete) from a demo reasoner.

**Profiling verdict (`sq-6tykl.4`).** RDFox's FBF/DRed exists to bound *over-deletion*: a
counting-free maintainer that deletes every consequence then re-derives the still-supported
ones. sparq took the derivation-**counting** route instead — a bounded per-fact support
count — so a retraction decrements counts and only unsupported facts leave; there is no
over-deletion pass to bound. Measured on this suite (work-box, non-canonical), pure-ABox
deletion is orders of magnitude cheaper than from-scratch across every tier, so **no
FBF-style over-deletion optimization is warranted** — the counting design already avoids the
cost FBF was invented to remove. The one residual cost axis is the *full-rebuild fallbacks*
(TBox / guard-predicate / recursive-layer ownership-transfer deltas), tracked as follow-up
bead `sq-6tykl.6`. Of those, the N3 **recursive-layer ownership transfer** is now settled by
targeted local re-derivation instead of a rebuild — the affected layer's own local fixpoint
decides the base↔layer hand-off, so it costs at most one layer recompute (already paid in that
round) rather than a from-scratch closure, and nothing at all when the assertion contributes no
delta and the hand-off defers to the next recompute of that layer (an inert interim state, see
the note in `crates/sparq-reason/src/incremental.rs`); the TBox and guard-predicate axes remain
open. The
re-derivation correctness invariant itself is guarded in-crate by
`crates/sparq-reason/tests/incremental_deletion_heavy.rs`, and the transfer's incrementality
by `incremental_n3_prop::ownership_transfer_deltas_stay_incremental_and_match_from_scratch`.

## How it works

```
bash bench/reason-deletion/run.sh                       # per-commit tiers (small medium)
REASONDEL_TIERS="small medium large" bash bench/reason-deletion/run.sh   # + nightly tier
```

1. `gen.sh <units>` → a deterministic LUBM-class synthetic ABox (`gen_reason_deletion.py`;
   1 unit = 1 athlete + 1 result = 8 triples; shared teams/games/events scale
   sub-linearly, like LUBM's departments-per-university).
2. The **existing, unmodified** driver
   `crates/sparq-reason/examples/incremental_olympics_bench.rs` runs on that corpus. The
   driver itself performs the randomized (fixed-seed xorshift) insert/delete batches over
   three workloads — RDFS (`MaterializedGraph`), OWL 2 RL mono + fixpoint
   (`MaterializedOwlGraph`) — and carries the load-bearing built-in asserts:
   incremental == from-scratch closure after the sequence, zero full rebuilds on ABox
   deltas, plus the N3/WAC section's own asserts. A violated assert panics → non-zero
   exit → `run.sh` is RED.
3. `parse_and_assert.py` additionally pins every tier's parsed ABox count and all three
   closure sizes to `expected.tsv` (deterministic; catches silent under/over-derivation
   and generator drift), emits `<metric>\t<value>\t<unit>` TSV timing lines, and writes
   the standard JSON results envelope (default under `/tmp/reason-deletion/`, git-ignored
   territory; `REASONDEL_JSON_OUT` overrides). The envelope's host block marks work-box
   timings NON-canonical (`quiet_box: false`) — timings are trend-only, never perf-gated
   and never baked into docs.

## Deletion ratios

The driver's batch sizes are fixed (1 / 100 / 10,000 triples), so the ratio axis comes
from varying the base size per tier: a 10,000-triple delete batch is ~42% of the small
tier's ABox (24,000 triples), ~8.3% of medium (120,000), ~1.7% of large (600,000). The
envelope records `delete_ratio = delta / abox_triples` per cell. Ratios here are dataset
geometry (fixture constants), not measured performance.

## Honesty notes

- **Why not literal LUBM data:** the driver synthesizes its TBox over the olympics
  vocabulary (`foaf:Person`/`dbo:team`/`oly:athlete`/…), and this bead forbids touching
  sparq-reason source. Feeding it univ-bench IRIs would make every rule a no-op and the
  deletion workload vacuous. The generator is LUBM-**class** (deterministic,
  scale-parameterized instance generator over a fixed schema with class hierarchies and
  domain/range/subPropertyOf/inverseOf-typed properties) bound to the vocabulary the
  driver's TBox actually ranges over, so the closure is materially larger than the ABox
  and deletion has real re-derivation work.
- **Round tier sizes (`sq-x58ow` fixed):** the driver's delete-sampler now mirrors
  `MaterializedOwlGraph::triggers_rebuild` — it excludes the six TBox *predicates* AND
  axiom-typing triples (`p == rdf:type` && an axiom-class object such as
  `owl:TransitiveProperty`) AND occurrence-guarded ids — so no ABox delta can ever
  spuriously take the rebuild path. Tier unit counts are therefore round (`3000`, not the
  old draw-safe `3020`); the closure fixtures in `expected.tsv` are re-validated for the
  new sizes.
- **Timing scope:** delta timings are single-shot (the driver measures one batch per
  size) and machine-dependent — comparative claims belong in envelope-carrying gathers
  on a quiet box, per `bench/CATALOG.md`. The RDFox/GraphDB competitor comparison is the
  parent bead's follow-up, not part of this suite.
