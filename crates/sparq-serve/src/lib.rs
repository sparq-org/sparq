// [OPUS-4.8] sq-ieqz: the crate's rustdoc front page IS its crate-local README
// (`crates/sparq-serve/README.md`, the same file crates.io/docs.rs surface), so the
// two never drift. The Wave A/B design narrative (ring / sequenced writer / scheduler
// invariants, the time-travel opt-in, the library-first + no-wasm-dep guards) lives in
// that README; the per-item rustdoc on the re-exports below carries the API detail.
#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)] // [OPUS-4.8] sq-emay: crate has zero `unsafe`

mod applier;
/// [OPUS-4.8] sq-xa15c (#1248 items 1+2) — the **in-process embedding seam**: a small,
/// documented facade over the engine's read/write/probe entry points
/// ([`query_json`](embed::query_json) / [`ask`](embed::ask) /
/// [`update_in_place`](embed::update_in_place) / [`apply_delta_nquads`](embed::apply_delta_nquads)
/// / [`exists`](embed::exists)+[`metadata`](embed::metadata)) plus a re-export of the
/// runtime-agnostic concurrency wrapper ([`GenerationRing`] + [`GraphApplier`] / [`Writer`]),
/// so an external host can call sparq in-process instead of over HTTP. **API tier-1
/// (proposed-stable)** — the proposed semver-stable embedding surface in the [API stability
/// policy]; the formal freeze is **pending maintainer ratification on #1248** (see the module
/// docs), not unilaterally frozen here. Thin re-exports only; no new behaviour.
///
/// [API stability policy]: https://github.com/sparq-org/sparq/blob/main/docs/api-stability.md
pub mod embed;
/// [OPUS-4.8] (sq-o5bi) ONLINE consistent-snapshot backup + restore for the serving store
/// — export an already-immutable pinned [`Generation`] to a single self-describing artifact
/// WHILE SERVING (no stop-the-world), and re-hydrate a [`sparq_core::Graph`] from one
/// (fail-closed on a corrupt/mismatched artifact). Compiled only behind the opt-in `backup`
/// feature (default OFF); the serving core is fully buildable without it. See the module docs
/// for the Option-A artifact format and the at-rest-encryption out-of-scope boundary.
///
/// [OPUS-4.8] (sq-b4fns) Also compiled (privately) when the `change-stream` feature is on
/// WITHOUT `backup`: the change stream reuses this module's `BackupError` and the
/// self-contained FNV-1a `body_digest`, so the two share one integrity scheme + error type.
/// In that case the module is `pub(crate)` (the `export`/`import` backup surface is NOT part
/// of the change-stream feature's public API). `#[allow(dead_code)]` covers the
/// backup-only export/import helpers that go unused in a change-stream-only build.
#[cfg(feature = "backup")]
pub mod backup;
#[cfg(all(feature = "change-stream", not(feature = "backup")))]
#[allow(dead_code)]
pub(crate) mod backup;
/// [OPUS-4.8] (sq-b4fns, gh-906) DURABLE, REPLAYABLE change-data-capture stream in the
/// Neptune-Streams shape — each commit emits an ordered, monotonically-sequenced change
/// record (op, quad(s), commit seq/generation, timestamp) persisted to a segmented, fsync'd
/// append-only log so a consumer can [`poll`](change_stream::ChangeLog::poll) from a given
/// offset and REPLAY after a restart ([`open`](change_stream::ChangeLog::open) re-reads the
/// segments). Compiled only behind the opt-in `change-stream` feature (default OFF); the
/// serving core is byte-identical without it. See the module docs for the segment format and
/// the same-lineage / fail-closed boundaries.
#[cfg(feature = "change-stream")]
pub mod change_stream;
/// [OPUS-5] (sq-l6zks, gh-3216) The EXTERNAL-BROKER seam over that durable log — a
/// [`ChangeSink`](change_sink::ChangeSink) trait, a stable JSON broker-message encoding, a
/// resumable [`BrokerRelay`](change_sink::BrokerRelay) pump with a durable delivered-through
/// watermark, and one in-tree [`NatsSink`](change_sink::NatsSink) (core-NATS publish, std-only).
/// Compiled only behind the opt-in `change-sink` feature (default OFF, implies `change-stream`)
/// — the SEPARATE, HEAVIER opt-in: it is the only thing in this crate that opens a network
/// socket, and relaying change records to a broker is a data-egress decision, so it never rides
/// along with `change-stream`. Kafka (and TLS/SASL/retries) is reached by implementing the trait
/// over the host's own client — no broker client, no async runtime enters this crate. See the
/// module docs for the at-least-once contract and the off-the-writer-thread rationale.
#[cfg(feature = "change-sink")]
pub mod change_sink;
/// [OPUS-4.8] (sq-bu1a) The INCREMENTAL DELTA-STREAM / point-in-time-recovery companion to the
/// Option-A base backup ([`backup`]): export the change between two same-lineage generations as a
/// self-describing delta artifact keyed off generation/writer-seq, and replay an ordered chain of
/// deltas forward onto a restored base to reach a chosen recovery point (fail-closed on a corrupt
/// or discontinuous stream). Compiled only behind the opt-in `backup` feature (default OFF). See
/// the module docs for the delta artifact format and the same-lineage boundary.
#[cfg(feature = "backup")]
pub mod backup_delta;
mod epoch;
mod footprint;
mod ring;
mod scheduler;
mod writer;

// [OPUS-4.8] sq-jluc: the response-bytes result cache (research/concurrent-serving.md
// §5 row 2 + §6.3) is OPT-IN and OFF by default — the `result-cache` cargo feature.
// The default `sparq-serve` build (and every consumer that does not turn the feature
// on) carries ZERO cache code: no module, no canonicalization, no extra dependency
// (the cache reuses the in-tree `rustc-hash` and `std`). Gating the modules — not just
// the re-exports — is what keeps the default build lean.
//
// LAYER DISTINCTION (do not conflate): `sparq-engine` ALSO has a feature named
// `result-cache`, but it is a DIFFERENT crate and a DIFFERENT layer — the engine's is
// the embedded whole-query algebra-keyed LRU inside one engine instance (caller bumps a
// version on mutation). THIS cache is the SERVING-layer response-bytes cache: keyed on
// the visibility scope and invalidated by the per-pod epoch bumps of the generation
// ring, neither of which the engine layer knows about. Cargo features are per-package,
// so the shared name is not a conflict; the module docs in `cache.rs` spell out the
// boundary.
#[cfg(feature = "result-cache")]
mod cache;
#[cfg(feature = "result-cache")]
mod canon;

#[cfg(feature = "backup")]
pub use backup::{export as backup_export, import as backup_import, BackupError, BackupMeta};
// [OPUS-4.8] (sq-bu1a) The change-stream / PITR delta companion (feature `backup`):
// `export_delta` between two same-lineage generations, `import_delta` to decode one (fail-closed),
// and `replay` to apply an ordered chain forward onto a restored base. `DELTA_MAGIC` is the
// artifact's distinguishing magic line; `Delta`/`DeltaMeta` are the decoded forms.
#[cfg(feature = "backup")]
pub use backup_delta::{
    export_delta as backup_export_delta, import_delta as backup_import_delta,
    replay as backup_replay, Delta, DeltaMeta, DELTA_MAGIC,
};
// [OPUS-4.8] (sq-b4fns, gh-906) The durable change-data-capture stream (feature
// `change-stream`): `ChangeLog` is the segmented, fsync'd append-only log; `record_commit`
// appends one ordered change record per commit and `poll(from_seq)` replays from an offset
// (resumable after a restart via `ChangeLog::open`). `ChangeRecord`/`Change`/`ChangeOp` are
// the read-back forms; `ChangeLogConfig` tunes segment size + fsync; `SEGMENT_MAGIC` is the
// distinguishing magic line. `BackupError` (re-exported here too so a change-stream-only
// consumer can name the error type) is the shared fail-closed error of the backup family.
// [FABLE-5] (sq-n9s4d) `RetentionPolicy`/`RetentionReport` are the retention/truncation
// surface: `ChangeLog::apply_retention` drops whole old segments by consumer-ack (hard
// safety bound) / age / total-size pressure; `ChangeLog::first_seq` is the trim horizon.
// [FABLE-5] (sq-r2cu1) `ChangeLog::rebase_to` is the explicit operator resync for a BROKEN
// stream (a dropped record / a `Writer::restore` fail-closes every later `record_commit`):
// it appends an honest gap record (`ChangeRecord::rebase`) and re-arms recording without
// wiping the log.
// [FABLE-5] (gh-2436) `ChangeStreamControl` (from `ChangeLog::into_commit_hook_with_control`)
// runs that resync on a RUNNING recorder — no stop/re-open; `rebase_to_new_lineage` is the
// post-restore variant for a replaced lineage whose generation numbering restarted.
#[cfg(feature = "change-stream")]
pub use change_stream::{
    Change, ChangeLog, ChangeLogConfig, ChangeOp, ChangeRecord, ChangeStreamControl,
    RetentionPolicy, RetentionReport, DEFAULT_SEGMENT_TARGET_BYTES, SEGMENT_MAGIC,
};
#[cfg(all(feature = "change-stream", not(feature = "backup")))]
pub use backup::BackupError;
// [OPUS-5] (sq-l6zks, gh-3216) The external-broker sink surface (feature `change-sink`):
// `ChangeSink` is the pluggable seam a host implements over its OWN broker client (Kafka via
// `rdkafka`, a webhook, …); `BrokerMessage`/`encode_message`/`SinkConfig` are the stable
// broker-message encoding; `BrokerRelay` is the resumable pump from the durable change log to
// a sink (`pump` → `PumpReport`, carrying a durable delivered-through watermark that feeds
// `RetentionPolicy::acked_through_seq`); `RecordingSink` is the in-memory test/dry-run sink;
// `NatsSink`/`NatsOptions` is the one in-tree broker client (core-NATS publish over plain TCP,
// std-only — no TLS). `SinkError` is the surface's fail-closed error type.
#[cfg(feature = "change-sink")]
pub use change_sink::{
    encode_message, BrokerMessage, BrokerRelay, ChangeSink, NatsOptions, NatsSink, PumpReport,
    RecordingSink, SinkConfig, SinkError, DEFAULT_PUMP_MAX_BATCH, DEFAULT_SUBJECT,
};
pub use applier::{GraphApplier, DEFAULT_COMPACT_THRESHOLD};
pub use epoch::{Epoch, PodEpochs, PodId};
pub use footprint::{Footprint, TargetGraph};
pub use ring::{Generation, GenerationRing, RingConfig, TimeTravelConfig, DEFAULT_RETAIN};
pub use scheduler::{
    Cost, Lane, SchedError, Scheduler, SchedulerConfig, Ticket, DEFAULT_HEAVY_THRESHOLD, P0,
};
pub use writer::{
    ApplyUpdates, CommitGranularity, WriteError, Writer, WriterConfig, DEFAULT_MAX_BATCH,
    DEFAULT_WINDOW,
};

// [OPUS-4.8] sq-jluc: result-cache public surface (feature `result-cache`). The cache's
// invalidation `Footprint` (the per-pod / unbounded READ footprint) is a DISTINCT type
// from this crate's write-side `footprint::Footprint` (the commutativity conflict tag),
// so it is re-exported under the disambiguating name `ReadFootprint`.
#[cfg(feature = "result-cache")]
pub use cache::{
    Bytes, CacheConfig, CacheStats, Footprint as ReadFootprint, LeadGuard, LeaseOutcome,
    ResultCache, ScopeKey, WaitGuard, WaitOutcome, DEFAULT_MAX_BYTES, DEFAULT_MAX_ENTRY_BYTES,
};
#[cfg(feature = "result-cache")]
pub use canon::{canonicalize, canonicalize_renamed};
