//! [`ApplyUpdates`] for the production store: `sparq_core::Graph` snapshots,
//! SPARQL Update strings as the update type.
//!
//! ## Snapshot production: the STRUCTURAL FORK (supersedes the A2 rebuild)
//!
//! A2 originally produced each generation via a full engine rebuild
//! (`sparq_engine::update(base, "")` — decode + re-intern + re-index), because
//! `Graph` shared no internal structure between instances. That fork was the
//! measured write-path bottleneck: **56 ms / 100 k triples, 1.07 s / 1 M**
//! (release, Apple Silicon), making batch-commit cost O(graph).
//!
//! [`Graph::fork`] now shares the base's immutable storage structurally — the
//! six permutation indexes, planner stats, the dictionary's frozen base and the
//! numeric/temporal caches are `Arc`-shared; only the pending delta (store
//! overlay + dict extension + cache side maps) is copied by value. So:
//!
//! - [`fork`](ApplyUpdates::fork) is `base.fork()` — **O(pending delta)**, not
//!   O(graph). (The very first fork of a freshly loaded flat graph pays a
//!   one-time O(n) dictionary/cache freeze to mint the shared base; every later
//!   generation is cheap, and compaction re-freezes, so the cost never recurs.)
//! - [`apply`](ApplyUpdates::apply) is the unchanged T17 delta-overlay path
//!   (`update_in_place`, measured 17 µs/update §4.3).
//! - [`seal`](ApplyUpdates::seal) carries the **compaction policy**: published
//!   generations DO retain their pending overlay now (readers' scans
//!   merge-on-read — the overlay machinery is exactly what `update_in_place`
//!   already exercised), so the overlay must not grow unboundedly. When the
//!   working copy's [`pending_delta_len`](Graph::pending_delta_len) reaches
//!   [`compact_threshold`](GraphApplier::with_compact_threshold) (default
//!   [`DEFAULT_COMPACT_THRESHOLD`]), seal folds it flat via [`Graph::compact`]
//!   — the old O(graph) cost, now amortised over the ~threshold/batch-size
//!   commits between compactions instead of paid on EVERY commit, and bounding
//!   both the merge-on-read overhead and the per-fork delta copy.
//!
//! A compaction failure (impossible for the in-memory graphs this path serves —
//! `compact` only errs on directory-backed I/O) publishes the un-compacted
//! working copy: correct, merely uncompacted; the next seal retries.
//!
//! Extension functions: this applier runs the plain engine update entry points.
//! A server that registers SPARQL extension functions (sparq-server's
//! `with_extensions`) should wrap its own `ApplyUpdates` around these calls —
//! the trait is the seam for exactly that.

use sparq_core::Graph;

use crate::footprint::Footprint;
use crate::writer::ApplyUpdates;

/// Default [`GraphApplier`] compaction threshold (pending-delta entries: overlay
/// triples + extension terms). Measured sweep (Apple Silicon, release, 10-triple
/// commits): per-commit cost is U-shaped in the threshold — small thresholds pay
/// the O(graph) fold too often, large ones clone a big delta on every fork.
/// 16 Ki sat at/near the minimum at BOTH 100 k (245 µs/commit) and 1 M
/// (593 µs/commit) base sizes (4 Ki: 290 µs/1.97 ms, 64 Ki: 667/699 µs), and is
/// graph-size-independent, so commit cost does not grow with the store.
pub const DEFAULT_COMPACT_THRESHOLD: usize = 16_384;

/// The production [`ApplyUpdates`]: `Graph` snapshots, SPARQL Update strings.
/// See the module docs for the structural-fork design and the compaction policy.
#[derive(Debug)]
pub struct GraphApplier {
    /// Fold the working copy's pending delta flat when it reaches this size
    /// (entries; see [`Graph::pending_delta_len`]).
    compact_threshold: usize,
}

impl Default for GraphApplier {
    fn default() -> Self {
        GraphApplier {
            compact_threshold: DEFAULT_COMPACT_THRESHOLD,
        }
    }
}

impl GraphApplier {
    /// An applier with the default compaction threshold.
    pub fn new() -> Self {
        Self::default()
    }

    /// An applier compacting once the pending delta reaches `threshold` entries
    /// (values below 1 are treated as 1 — compact after every batch).
    pub fn with_compact_threshold(threshold: usize) -> Self {
        GraphApplier {
            compact_threshold: threshold.max(1),
        }
    }
}

impl ApplyUpdates for GraphApplier {
    type Snapshot = Graph;
    type Working = Graph;
    type Update = String;

    /// O(pending delta): the structural fork — shared immutable base, copied
    /// delta. (One-time O(n) freeze on the first-ever fork of a flat graph.)
    fn fork(&mut self, base: &Graph) -> Result<Graph, String> {
        Ok(base.fork())
    }

    /// O(batch): the T17 delta-overlay path. On `Err` the graph may hold a
    /// partially applied prefix of the update's operations — exactly the hazard
    /// the writer's re-fork-and-replay recovery handles.
    fn apply(&mut self, working: &mut Graph, update: &String) -> Result<(), String> {
        sparq_engine::update_in_place(working, update)
    }

    /// The compaction policy (module docs): fold the pending delta flat once it
    /// reaches the threshold, so overlay merge-on-read cost and per-fork delta
    /// copies stay bounded while the O(graph) fold runs only once per
    /// ~threshold/batch commits.
    fn seal(&mut self, mut working: Graph) -> Result<Graph, String> {
        if working.pending_delta_len() >= self.compact_threshold {
            // In-memory compaction cannot fail; a (directory-backed) failure
            // publishes the correct-but-uncompacted copy and retries next seal.
            let _ = working.compact();
        }
        // [OPUS-4.8] (sq-vpx4) This applier has no durable side, so seal never fails.
        Ok(working)
    }

    /// Conservative graph-level write footprint parsed from the update text
    /// ([`Footprint::of_sparql`]). Costs one spargebra parse (~2 µs at
    /// prod-solid-server update sizes) on the writer thread, paid only under
    /// [`CommitGranularity::CommuteGroup`](crate::CommitGranularity) — the
    /// engine's own parse inside [`apply`](ApplyUpdates::apply) still happens
    /// (there is no pre-parsed update entry point in the engine today; an
    /// engine-seams ask if CommuteGroup mode ever carries hot traffic).
    fn footprint(&self, update: &String) -> Footprint {
        Footprint::of_sparql(update)
    }
}

/// [SONNET-5] (sq-artifact-keeper-topk) [`ApplyUpdates`] over
/// [`sparq_engine::PreparedUpdate`] instead of raw SPARQL Update text — for a caller that
/// submits the SAME update template repeatedly with only a bound value changing (a
/// job-queue claim, an upsert, any recurring parameterized write), avoiding the re-parse
/// [`GraphApplier`] pays on every single commit.
///
/// `GraphApplier::apply` calls [`sparq_engine::update_in_place`], which parses the update's
/// full text FRESH every time — measured ~13.5 µs of a ~21.2 µs total per-update cost
/// (~63%) on a representative claim template, because [`PreparedUpdate::bind`] (a
/// structural placeholder substitution over the already-parsed algebra, never string
/// concatenation) costs under 1 µs by comparison. A caller parses its template ONCE (e.g.
/// [`PreparedUpdate::parse`] at startup or on first use), binds a fresh value per
/// submission, and submits the bound [`PreparedUpdate`](sparq_engine::PreparedUpdate) —
/// this applier then pays only bind + eval + apply, roughly half the raw-text cost.
///
/// Same compaction policy as [`GraphApplier`] (module docs): `fork`/`seal` are identical,
/// only `apply`'s update type and engine entry point differ.
///
/// Only with the opt-in `params` feature — the serving core (ring + sequenced writer) is
/// fully buildable without it; a build without the feature is byte-identical to before.
#[cfg(feature = "params")]
#[derive(Debug)]
pub struct PreparedGraphApplier {
    compact_threshold: usize,
}

#[cfg(feature = "params")]
impl Default for PreparedGraphApplier {
    fn default() -> Self {
        PreparedGraphApplier {
            compact_threshold: DEFAULT_COMPACT_THRESHOLD,
        }
    }
}

#[cfg(feature = "params")]
impl PreparedGraphApplier {
    /// An applier with the default compaction threshold.
    pub fn new() -> Self {
        Self::default()
    }

    /// An applier compacting once the pending delta reaches `threshold` entries
    /// (values below 1 are treated as 1 — compact after every batch).
    pub fn with_compact_threshold(threshold: usize) -> Self {
        PreparedGraphApplier {
            compact_threshold: threshold.max(1),
        }
    }
}

#[cfg(feature = "params")]
impl ApplyUpdates for PreparedGraphApplier {
    type Snapshot = Graph;
    type Working = Graph;
    type Update = sparq_engine::PreparedUpdate;

    /// O(pending delta): identical to [`GraphApplier::fork`] — see the module docs.
    fn fork(&mut self, base: &Graph) -> Result<Graph, String> {
        Ok(base.fork())
    }

    /// O(batch): the same T17 delta-overlay path as [`GraphApplier::apply`]
    /// ([`sparq_engine::update_in_place_prepared`]), over an ALREADY-PARSED,
    /// already-bound update — no text re-parse on the writer thread. Same partial-apply
    /// hazard on `Err` as [`GraphApplier::apply`]; the writer's re-fork-and-replay
    /// recovery handles it identically.
    fn apply(&mut self, working: &mut Graph, update: &sparq_engine::PreparedUpdate) -> Result<(), String> {
        sparq_engine::update_in_place_prepared(working, update)
    }

    /// Identical compaction policy to [`GraphApplier::seal`] — see the module docs.
    fn seal(&mut self, mut working: Graph) -> Result<Graph, String> {
        if working.pending_delta_len() >= self.compact_threshold {
            let _ = working.compact();
        }
        Ok(working)
    }

    /// Always [`Footprint::Barrier`] (the trait's own documented safe default): there is
    /// no parsed-algebra entry point into [`Footprint::of_sparql`] (text-only), and
    /// re-serialising the already-parsed update back to text just to re-parse it would
    /// defeat this applier's entire purpose. Under the opt-in
    /// [`CommitGranularity::CommuteGroup`](crate::CommitGranularity) mode this degrades to
    /// one generation per update (correct, just not maximally batched) — CommuteGroup is
    /// orthogonal to this applier's goal (avoiding re-parse under the default `Window`
    /// granularity, which never calls `footprint`).
    fn footprint(&self, _update: &sparq_engine::PreparedUpdate) -> Footprint {
        Footprint::Barrier
    }
}
