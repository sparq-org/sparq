//! The single sequenced writer with a group-commit window
//! (research/concurrent-serving.md §6.5) — Wave A deliverable 2, composing over
//! the A1 [`GenerationRing`].
//!
//! One writer thread owns the sole right to publish generations. Callers submit
//! updates (an opaque `Update` value plus the set of touched [`PodId`]s — for
//! prod-solid-server shapes statically the resource graph + parent graph, §6.5);
//! the writer batches every update that arrives within a **group-commit window**
//! (`min(T_window, N_max)`, default 3 ms / 256 updates) into ONE new snapshot and
//! publishes it as ONE generation, bumping the epochs of the union of the touched
//! pods. Readers are never involved: they keep loading
//! [`GenerationRing::current`] lock-free; the only writer/reader interaction is
//! the ring's arc-swap at publish.
//!
//! ## Adaptive group-commit ([`WriterConfig::adaptive_commit`])
//!
//! The fixed window is a batching heuristic, not a correctness requirement, and it
//! is pure latency for a **serial / low-concurrency** client that blocks on its own
//! ack (the interactive LDP-CRUD shape): nothing else can arrive to fill the window,
//! so each update pays ~`window` even though the engine's `update_in_place` is ~15 µs
//! (the sq-p7kk5 update-path gap — the 3 ms default window dominated a serial
//! client's ~99% of commit latency). Opt-in [`adaptive_commit`](WriterConfig::adaptive_commit)
//! drops the timed wait: after the first update the writer drains only the ALREADY-
//! queued backlog (`try_recv`) and commits the instant it empties. Serial clients
//! commit in engine-time; a genuinely concurrent stream still batches whatever piled
//! up between iterations, so group-commit amortisation under load is preserved. FIFO
//! order, per-update atomicity, and every ACID property below are unchanged — only
//! epoch-tick granularity may be finer (a Wave-B cache-key concern, never
//! correctness). sparq-server enables it for its write path.
//!
//! ## Sequencing and ACID (§6.5)
//!
//! Updates inside a batch are applied in **submission order** (strict FIFO — the
//! deterministic, replayable single-order log that replication will lean on;
//! commutativity is used as a *grouping license*, never to reorder). Cross-batch
//! reordering: none — batches are committed in sequence by the one thread.
//! A = per-update atomicity via the failed-update policy below; C = single-order
//! application; I = snapshot isolation for readers + the serial writer;
//! D = out of scope for the in-memory server v1 (as §6.5 records).
//!
//! ## Commutativity batching ([`CommitGranularity`])
//!
//! With the default [`CommitGranularity::Window`], one window = one generation =
//! one epoch tick — litreview A's epoch group-commit pattern ("one new
//! generation per epoch, not per statement"), and the §3.3/§6.5 prescription
//! that generation granularity come from batching windows. Correctness never
//! needs more: FIFO application of the whole window onto one working copy *is*
//! the serial order.
//!
//! Opt-in [`CommitGranularity::CommuteGroup`] splits each window's batch into
//! maximal runs of provably-commuting pure-data updates
//! ([`Footprint::commutes_with`] — conservative graph-level detection; an
//! unanalyzable update or a same-graph opposite-polarity data update is a
//! barrier) and publishes ONE generation per run, in arrival order. Every
//! published generation is then internally conflict-free — a log record whose
//! updates can be replayed or applied in any order (the parallel-delta /
//! replication seam of §6.8) — and each epoch tick covers exactly one commuting
//! group. The price is one fork/seal/publish per run instead of per window
//! (barrier-heavy streams degrade to per-update generations) plus one
//! footprint parse per update on the writer thread; `bench/serve/writer_spike`
//! measures both. Application order is FIFO in both modes, so the committed
//! result is byte-identical to serial execution either way (differential-tested
//! in `tests/commute.rs`).
//!
//! ## Failed-update policy (documented contract)
//!
//! A failing update must not poison its batch: updates are applied in submission
//! order against a writer-private working copy; an update that fails to apply is
//! **reported to its submitter and skipped — every other update in the batch
//! proceeds**. Rationale: batch membership is an artifact of arrival timing, not
//! of intent — two independent clients whose requests happen to share a 3 ms
//! window must not see each other's failures (atomicity stays per update, exactly
//! as today's `AppState::apply_update` provides). The epochs bumped at publish
//! are the union of the touched pods of the *successful* updates only — a skipped
//! update changed nothing, so bumping its pods would churn Wave B cache keys
//! spuriously. If every update in a batch fails, **no generation is published**
//! (an identical snapshot under a new number would invalidate nothing and help
//! nobody).
//!
//! Recovery detail: a failed [`ApplyUpdates::apply`] may have left the working
//! copy partially mutated (the engine's `update_in_place` documents exactly this
//! prefix-application hazard, and the existing server Writer discards its buffer
//! for the same reason). The writer therefore conservatively discards the working
//! copy, re-forks the base snapshot, and replays the batch's previously-successful
//! updates in order before continuing — one extra O(fork) per failed update,
//! acceptable because failures are the rare path (parse errors and constraint
//! violations, not steady state).
//!
//! ## Commit hook (per-publish observation seam)
//!
//! (sq-bdaw5) [`Writer::spawn_with_commit_hook`] installs a hook that runs on
//! the writer thread once per **commit-publish** with the `(predecessor, published)`
//! generation pair — the seam a durable change-data-capture recorder
//! (`ChangeLog::into_commit_hook`, feature `change-stream`) plugs into. Because the
//! writer thread is the sole publisher, the pairs chain gapless without any locking:
//! recording costs one append per PUBLISHED GENERATION on the writer thread (amortised
//! across the batch) instead of one append per update under a caller-side mutex held
//! across `submit` — group-commit concurrency is preserved with the stream on. Acks
//! happen-after the hook returns; restore publishes never fire it (not same-lineage).
//! No hook installed = zero cost.
//!
//! ## Snapshot production (the §6.4 "working copy")
//!
//! §6.4 imagines a persistent writer-private working copy folded periodically.
//! That scheme worked for the double buffer because the writer could *reclaim*
//! the previously published graph (`Arc::try_unwrap` + poll) — which is both the
//! measured pathology A1 removed (5.4 s/32 s pinned-snapshot stall, §4.3/§4.4)
//! and *impossible* under the ring by design: the ring itself retains up to K
//! old generations, so a published graph never drains back to the writer.
//! Every publish therefore mints a fresh `Graph` — via the STRUCTURAL FORK
//! (`Graph::fork`): the immutable storage (indexes, dict base, caches) is
//! Arc-shared with the base generation and only the small pending delta is
//! copied, so fork is O(delta) and the periodic fold §6.4 wanted lives in
//! [`ApplyUpdates::seal`] as a threshold compaction — see
//! [`GraphApplier`](crate::GraphApplier) for the policy and measured numbers.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use rustc_hash::FxHashSet;

use crate::epoch::PodId;
use crate::footprint::{Footprint, TargetGraph};
use crate::ring::{Generation, GenerationRing};

/// Default group-commit window. §6.5 prescribes 2–5 ms; the middle of the band
/// adds ≤3 ms p50 write latency (far inside the 45 s contract) while absorbing
/// ~hundreds of updates per window at the measured 17 µs/update in-place cost.
pub const DEFAULT_WINDOW: Duration = Duration::from_millis(3);

/// Default max batch size: the window closes early once this many updates have
/// been collected (§6.5's `min(T_window, N_max)`, e.g. "2–5 ms or 256 updates").
pub const DEFAULT_MAX_BATCH: usize = 256;

/// How many generations one group-commit window publishes (module docs:
/// "Commutativity batching").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CommitGranularity {
    /// One window = one generation (the default; litreview A's epoch
    /// group-commit pattern and the §6.5 prescription). Footprints are never
    /// computed — zero analysis overhead.
    #[default]
    Window,
    /// One generation per maximal run of provably-commuting data updates
    /// within the window ([`Footprint::commutes_with`]); a barrier (general /
    /// unanalyzable update, or a same-graph opposite-polarity data update)
    /// starts a new generation. Buys internally conflict-free log records and
    /// per-group epoch ticks at the cost of more publishes on barrier-heavy
    /// streams + one [`ApplyUpdates::footprint`] call per update.
    CommuteGroup,
}

/// Configuration for the sequenced [`Writer`].
#[derive(Clone, Debug)]
pub struct WriterConfig {
    /// Group-commit window: the first update to arrive opens the window; every
    /// update arriving before it elapses joins the same batch → one generation.
    pub window: Duration,
    /// The window also closes as soon as this many updates are batched
    /// (back-pressure bound; values below 1 are treated as 1).
    pub max_batch: usize,
    /// Whether a window commits as one generation or as one generation per
    /// commuting run (module docs).
    pub granularity: CommitGranularity,
    /// (sq-p7kk5) ADAPTIVE group-commit. When `false` (the default,
    /// unchanged behaviour) the writer always waits the full [`window`](Self::window)
    /// after the first update, absorbing any concurrent traffic that arrives within
    /// it into one generation. That fixed wait is pure latency for a **serial /
    /// low-concurrency** client (one in-flight update at a time — the interactive
    /// LDP-CRUD shape): the client is blocked on its own ack, so nothing else can
    /// arrive to fill the window, and it pays ~`window` per update no matter how fast
    /// the engine is (the engine's `update_in_place` is ~15 µs; the 3 ms default
    /// window is ~200× that — the measured update-path gap in sq-p7kk5).
    ///
    /// When `true` the writer instead DRAINS whatever is already queued
    /// non-blocking (`try_recv`) after the first update and commits as soon as the
    /// backlog is empty — **no timed wait**. A serial client commits in
    /// engine-time (µs), not window-time (ms); a genuinely concurrent stream still
    /// batches every update that had already piled up between the writer's
    /// iterations, so group-commit amortisation under real load is preserved. It is
    /// a strict LATENCY improvement with NO change to the mutation/durability
    /// contract: application stays strict FIFO, per-update atomicity is unchanged,
    /// every accepted update is still committed, and each committed generation is
    /// still the serial order. Two clients whose requests genuinely overlap MAY land
    /// in separate generations rather than one — that only affects epoch-tick
    /// granularity (a Wave-B cache-key concern), never correctness or visibility.
    ///
    /// [`max_batch`](Self::max_batch) still bounds a drained backlog. Off by default
    /// so the documented windowed contract is preserved for callers that rely on it;
    /// sparq-server opts in for its interactive write path.
    pub adaptive_commit: bool,
}

impl Default for WriterConfig {
    fn default() -> Self {
        WriterConfig {
            window: DEFAULT_WINDOW,
            max_batch: DEFAULT_MAX_BATCH,
            granularity: CommitGranularity::Window,
            adaptive_commit: false,
        }
    }
}

/// How the writer turns the current snapshot plus a batch of updates into the
/// next snapshot. Implemented by [`GraphApplier`](crate::GraphApplier) for the
/// production store; tests use instrumented mocks.
///
/// Contract the writer relies on:
/// - [`fork`](Self::fork) produces a private working copy of `base`; the writer
///   mutates only that copy, so readers of `base` are never affected.
/// - [`apply`](Self::apply) on `Err` may leave the working copy in ANY state
///   (partial prefix application is allowed — the writer recovers by re-forking
///   and replaying, see the module docs). It must, however, be deterministic
///   enough that an update that succeeded once succeeds again on replay against
///   the same prefix; a replay failure is handled (the update is demoted to
///   failed) but is reported as an anomaly in its error message.
/// - [`seal`](Self::seal) finalizes the working copy into the published snapshot
///   type (fold/compact hooks live here).
pub trait ApplyUpdates: Send + 'static {
    /// The published snapshot type — the `S` of the [`GenerationRing`].
    type Snapshot: Send + Sync + 'static;
    /// The writer-private mutable working copy.
    type Working;
    /// One submitted update (e.g. a SPARQL Update string).
    type Update: Send + 'static;

    /// Produces a writer-private working copy of `base`.
    fn fork(&mut self, base: &Self::Snapshot) -> Result<Self::Working, String>;

    /// Applies one update to the working copy. On `Err` the copy may be
    /// partially mutated; the writer discards and re-forks (module docs).
    fn apply(&mut self, working: &mut Self::Working, update: &Self::Update) -> Result<(), String>;

    /// Finalizes the working copy into a publishable snapshot.
    ///
    /// (sq-vpx4) Fallible so an applier with a DURABLE side (e.g. the
    /// server's `--persist` mirror) can refuse a batch whose durable commit failed
    /// WITHOUT tearing down the writer thread. `Err(msg)` means "this batch did not
    /// durably commit": the writer publishes NOTHING for it and fails every pending
    /// submitter with [`WriteError::Unavailable`] — the fail-closed contract (never
    /// ack/publish a write that didn't durably commit) is preserved, but a transient
    /// I/O error (e.g. a brief `ENOSPC` that later clears) becomes a refused request
    /// the client can retry rather than a process-killing panic. The writer thread
    /// survives, so reads keep being served from the last published snapshot and a
    /// later write succeeds once durability recovers. Purely in-memory appliers never
    /// fail here (return `Ok`).
    fn seal(&mut self, working: Self::Working) -> Result<Self::Snapshot, String>;

    /// The update's conservative write footprint, used only under
    /// [`CommitGranularity::CommuteGroup`] to form commuting runs. The default
    /// is the safe one — every update a [`Footprint::Barrier`] (commutes with
    /// nothing), which degrades CommuteGroup mode to one generation per update
    /// but can never mis-group. [`GraphApplier`](crate::GraphApplier) overrides
    /// this with [`Footprint::of_sparql`].
    fn footprint(&self, _update: &Self::Update) -> Footprint {
        Footprint::Barrier
    }

    /// (sq-x32t) Runs an out-of-band MAINTENANCE operation on the writer
    /// thread — the only thread that mutates the applier's durable side. The default
    /// is a no-op (an in-memory applier has nothing to maintain).
    ///
    /// This is how an admin compaction/vacuum reaches a durable store WITHOUT a lock:
    /// the durable [`Graph`](sparq_core::Graph) is owned by the applier and only ever
    /// touched on the writer thread (fork/apply/seal), so a maintenance op routed here
    /// runs strictly between batches — never concurrently with a commit. It does NOT
    /// publish a generation: maintenance (e.g. WAL compaction) preserves the live
    /// triple set exactly, so the readable snapshot is byte-equivalent before and
    /// after. The published in-memory lineage is therefore untouched; only the durable
    /// on-disk image is rewritten.
    ///
    /// `Err(msg)` reports a maintenance failure to the caller; the writer thread stays
    /// alive (a failed maintenance op never tears down the writer or affects reads).
    fn maintain(&mut self) -> Result<(), String> {
        Ok(())
    }

    /// (sq-ft7u) RESTORE the applier's durable side from a freshly-built, already-
    /// validated `fresh` snapshot, and return the new published snapshot the ring should serve.
    /// Runs on the WRITER THREAD (the only thread that touches the durable store), sequenced
    /// through the same queue as updates — so the durable swap never races a commit and the
    /// served lineage is replaced atomically by the publish the caller does with the returned
    /// snapshot. UNLIKE [`maintain`](Self::maintain), this CHANGES the live triple set (it
    /// replaces the whole store), so the writer publishes a new generation from the return value.
    ///
    /// The default is a no-op error: an applier with no durable side (the in-memory server) has
    /// no on-disk store to write through, so it uses the ring/writer-swap restore path instead.
    /// An applier with a `--persist` durable mirror overrides this to perform the crash-safe
    /// on-disk swap (e.g. via `Graph::restore_into_durable`) and adopt the re-opened store.
    ///
    /// `Err(msg)` is a restore failure that leaves the writer alive and the OLD durable store
    /// intact (the durable swap is fail-closed); the caller is told and nothing is published.
    fn restore_durable(&mut self, _fresh: Self::Snapshot) -> Result<Self::Snapshot, String> {
        Err("restore_durable is not supported by this applier (no durable store)".to_string())
    }
}

/// Why a submitted update did not land in a published generation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WriteError {
    /// The update failed to apply and was skipped; the rest of its batch
    /// proceeded (per-update atomicity — module docs). Carries the application
    /// error (e.g. the SPARQL parse error).
    Rejected(String),
    /// The writer has shut down (or its thread panicked); the update was not
    /// applied.
    Shutdown,
    /// (sq-vpx4) The update applied cleanly in memory but its batch
    /// could NOT be made durable ([`ApplyUpdates::seal`] returned `Err`, e.g. a
    /// transient `ENOSPC`/I/O error on the `--persist` mirror). Nothing was
    /// published and nothing was acked — the write is REFUSED, not silently lost,
    /// and is safe to RETRY (a 503-class outcome). The writer thread stays alive,
    /// so reads keep being served and a later write succeeds once durability
    /// recovers. Carries the durability error.
    Unavailable(String),
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteError::Rejected(e) => write!(f, "update rejected: {e}"),
            WriteError::Shutdown => f.write_str("writer has shut down"),
            WriteError::Unavailable(e) => {
                write!(f, "update not durably committed (retryable): {e}")
            }
        }
    }
}

impl std::error::Error for WriteError {}

/// One queued submission.
struct Msg<U> {
    update: U,
    touched: Vec<PodId>,
    /// `Some` for sync submissions: receives `Ok(generation number)` once the
    /// generation containing the update is published, or the `WriteError`.
    /// `None` for fire-and-forget.
    ack: Option<SyncSender<Result<u64, WriteError>>>,
}

/// (sq-x32t) What the writer thread receives on its queue: either an UPDATE
/// submission (the steady-state path) or a MAINTENANCE request (an admin op such as WAL
/// compaction). Maintenance is sequenced through the SAME single queue as updates, so it
/// runs strictly between batches on the one thread that owns the durable store — no lock,
/// no concurrent access to the durable graph, no reordering relative to in-flight writes.
enum Cmd<U, S> {
    /// A normal update submission.
    Update(Msg<U>),
    /// Run [`ApplyUpdates::maintain`] on the writer thread; the result is sent back on the
    /// channel so the caller can block for completion. Publishes NO generation (maintenance
    /// preserves the live snapshot exactly).
    Maintain(SyncSender<Result<(), String>>),
    /// (sq-ft7u) RESTORE the durable store from the freshly-built, already-validated
    /// snapshot `fresh` on the writer thread, then PUBLISH the restored snapshot as a new
    /// generation so the ring serves it. Sequenced through the same queue as updates, so the
    /// durable swap runs strictly between batches — never concurrently with a commit. The ack
    /// carries the published generation number (or the restore error). UNLIKE `Maintain`, this
    /// CHANGES the live triple set (it replaces the whole store), so a generation IS published.
    Restore {
        fresh: Box<S>,
        ack: SyncSender<Result<u64, String>>,
    },
}

/// The single sequenced writer (§6.5): owns the one thread allowed to call
/// [`GenerationRing::publish`], batching submissions in a group-commit window.
///
/// `Writer` is `Send + Sync`; share it as `Arc<Writer<_>>` (or by reference) —
/// both submission methods take `&self`. Dropping the writer closes the queue,
/// commits any in-flight batch (graceful drain — pending sync submitters still
/// get their generation number), and joins the thread.
///
/// (sq-ft7u) The second type parameter `S` is the applier's SNAPSHOT type (e.g. the
/// server's `Graph`): it is the payload of the out-of-band [`restore`](Self::restore) op, which
/// installs a freshly-built snapshot as the new served lineage (the durable-restore primitive).
/// It is inferred from the applier at [`spawn`](Self::spawn), so existing call sites that name
/// only `Writer<U>` keep compiling via the `S = ()` default (no restore payload).
pub struct Writer<U: Send + 'static, S: Send + Sync + 'static = ()> {
    /// `Some` until drop; dropping the sender is what tells the thread to drain.
    tx: Option<mpsc::Sender<Cmd<U, S>>>,
    thread: Option<JoinHandle<()>>,
}

impl<U: Send + 'static, S: Send + Sync + 'static> Writer<U, S> {
    /// Spawns the writer thread over `ring`, owning `applier` as its snapshot
    /// production strategy.
    pub fn spawn<A>(
        ring: Arc<GenerationRing<A::Snapshot>>,
        applier: A,
        config: WriterConfig,
    ) -> Self
    where
        A: ApplyUpdates<Update = U, Snapshot = S>,
    {
        // A never-installed hook: the `fn` pointer type only names `H` for the
        // hook-less spawn (zero runtime cost — the Option is `None` forever).
        Self::spawn_inner(
            ring,
            applier,
            config,
            None::<fn(&Generation<S>, &Generation<S>)>,
        )
    }

    /// (sq-bdaw5) [`spawn`](Self::spawn) with a **commit hook**: `hook` runs ON
    /// THE WRITER THREAD immediately after every commit-publish, receiving the
    /// `(predecessor, published)` generation pair of that commit. Because the writer
    /// thread is the ring's ONLY publisher, consecutive pairs CHAIN (each call's
    /// `predecessor` is the previous call's `published`) — exactly the gapless,
    /// monotonic same-lineage stream a change-data-capture recorder needs, with **no
    /// caller-side locking and no serialisation of submitters**: recording happens once
    /// per PUBLISHED GENERATION (amortised over the whole group-commit batch), not once
    /// per update under a global mutex held across `submit`.
    ///
    /// Contract:
    /// - **One call per commit-publish.** Under [`CommitGranularity::Window`] that is
    ///   one call per committed window; under [`CommitGranularity::CommuteGroup`], one
    ///   per published commuting run. A batch whose every update failed publishes
    ///   nothing and the hook is NOT called (there is no new generation to report).
    /// - **Acks happen-after the hook returns.** A submitter's [`submit`](Self::submit)
    ///   does not get its generation number until the hook has run for the generation
    ///   containing its update — so a durable side effect the hook performs (e.g. a
    ///   `ChangeLog` record append + fsync, via `ChangeLog::into_commit_hook`, feature
    ///   `change-stream`) is complete by the time the write is acknowledged. The hook's
    ///   runtime therefore adds to write ack latency, once per generation; keep it
    ///   O(commit), not O(graph).
    /// - **Restore publishes do NOT fire the hook.** [`restore`](Self::restore) replaces
    ///   the whole store with a freshly-built snapshot that does NOT share the writer
    ///   lineage of its predecessor, so a lineage-diffing consumer (the change stream's
    ///   same-lineage quad diff) must not be handed the pair. A change stream spanning a
    ///   restore needs an explicit re-baseline — the fail-closed discontinuity is
    ///   surfaced by the recorder, never silently diffed through; the operator resync is
    ///   `ChangeLog::rebase_to` (feature `change-stream`, sq-r2cu1), which appends an
    ///   honest gap record and re-arms recording from the restored generation.
    /// - The hook runs on the writer thread: a panic in it kills the writer (submitters
    ///   see [`WriteError::Shutdown`]). Return, don't panic, on recoverable errors.
    pub fn spawn_with_commit_hook<A, H>(
        ring: Arc<GenerationRing<A::Snapshot>>,
        applier: A,
        config: WriterConfig,
        hook: H,
    ) -> Self
    where
        A: ApplyUpdates<Update = U, Snapshot = S>,
        H: FnMut(&Generation<S>, &Generation<S>) + Send + 'static,
    {
        Self::spawn_inner(ring, applier, config, Some(hook))
    }

    /// The one true spawn: `hook`, when present, is moved onto the writer thread and
    /// fired once per commit-publish (see [`spawn_with_commit_hook`](Self::spawn_with_commit_hook)).
    fn spawn_inner<A, H>(
        ring: Arc<GenerationRing<A::Snapshot>>,
        applier: A,
        config: WriterConfig,
        hook: Option<H>,
    ) -> Self
    where
        A: ApplyUpdates<Update = U, Snapshot = S>,
        H: FnMut(&Generation<S>, &Generation<S>) + Send + 'static,
    {
        let (tx, rx) = mpsc::channel::<Cmd<U, S>>();
        let thread = thread::Builder::new()
            .name("sparq-serve-writer".into())
            .spawn(move || run(ring, applier, config, rx, hook))
            .expect("spawn writer thread");
        Writer {
            tx: Some(tx),
            thread: Some(thread),
        }
    }

    /// Submits an update and **blocks until the generation containing it is
    /// published**, returning that generation's number (the group-commit ack:
    /// at most one window + one batch application of latency). `touched` is the
    /// update's epoch tag — the pods (named graphs) it writes, whose epochs
    /// the publish bumps (commutativity grouping never trusts it; that comes
    /// from [`ApplyUpdates::footprint`]).
    ///
    /// The returned number doubles as the **read-your-writes token** (Wave A3,
    /// the single-node `shard_seq`): the publish happens-before this ack, so
    /// [`GenerationRing::read_your_writes`]`(n)` succeeds on this ring from the
    /// moment `submit` returns, and a client that round-trips `n` as a session
    /// token can be routed to any replica that has applied through `n`.
    ///
    /// `Err(Rejected)` means *this* update failed and was skipped while its
    /// batch proceeded; `Err(Shutdown)` means it was never applied.
    pub fn submit(
        &self,
        update: U,
        touched: impl IntoIterator<Item = PodId>,
    ) -> Result<u64, WriteError> {
        let (ack_tx, ack_rx) = mpsc::sync_channel(1);
        self.send(Msg {
            update,
            touched: touched.into_iter().collect(),
            ack: Some(ack_tx),
        })?;
        // A dropped ack sender means the writer thread died mid-batch.
        ack_rx.recv().map_err(|_| WriteError::Shutdown)?
    }

    /// (sq-x32t) Runs an out-of-band MAINTENANCE op ([`ApplyUpdates::maintain`])
    /// on the writer thread and **blocks until it completes**, returning its result. The
    /// request is sequenced through the same queue as updates, so it runs strictly between
    /// batches — never concurrently with a commit, and never reordered across an in-flight
    /// write. It publishes no generation (maintenance preserves the live snapshot exactly),
    /// so readers are unaffected for the whole operation. The admin WAL compaction/vacuum
    /// reaches the durable store this way. `Err(Shutdown)` if the writer is gone; otherwise
    /// the inner `Result` is the maintenance outcome (an `Err(String)` is a maintenance
    /// failure that left the writer alive).
    pub fn maintain(&self) -> Result<Result<(), String>, WriteError> {
        let (done_tx, done_rx) = mpsc::sync_channel(1);
        self.tx
            .as_ref()
            .expect("sender present until drop")
            .send(Cmd::Maintain(done_tx))
            .map_err(|_| WriteError::Shutdown)?;
        // A dropped done sender means the writer thread died running the op.
        done_rx.recv().map_err(|_| WriteError::Shutdown)
    }

    /// (sq-ft7u) RESTORE the durable store from the freshly-built, already-validated
    /// snapshot `fresh` ON THE WRITER THREAD, then PUBLISH the restored snapshot as a new
    /// generation, and **block until it completes** — returning the published generation number
    /// (or the restore error). Routed through the SAME queue as updates, so the durable swap
    /// ([`ApplyUpdates::restore_durable`]) runs strictly between batches on the single thread that
    /// owns the durable store — never concurrently with a commit, never reordered across an
    /// in-flight write. UNLIKE [`maintain`](Self::maintain) it CHANGES the live triple set, so a
    /// new generation IS published from the restored snapshot (the ring then serves it). A restore
    /// failure leaves the writer alive and the OLD durable store intact (the underlying swap is
    /// fail-closed): `Err(Shutdown)` if the writer is gone, else the inner `Result` is the outcome.
    pub fn restore(&self, fresh: S) -> Result<Result<u64, String>, WriteError> {
        let (ack_tx, ack_rx) = mpsc::sync_channel(1);
        self.tx
            .as_ref()
            .expect("sender present until drop")
            .send(Cmd::Restore {
                fresh: Box::new(fresh),
                ack: ack_tx,
            })
            .map_err(|_| WriteError::Shutdown)?;
        ack_rx.recv().map_err(|_| WriteError::Shutdown)
    }

    /// Fire-and-forget submission: queues the update and returns immediately.
    /// `Err(Shutdown)` if the writer is gone; otherwise the outcome (including a
    /// possible [`WriteError::Rejected`]) is not reported — callers that need
    /// the result use [`submit`](Self::submit).
    pub fn submit_detached(
        &self,
        update: U,
        touched: impl IntoIterator<Item = PodId>,
    ) -> Result<(), WriteError> {
        self.send(Msg {
            update,
            touched: touched.into_iter().collect(),
            ack: None,
        })
    }

    fn send(&self, msg: Msg<U>) -> Result<(), WriteError> {
        self.tx
            .as_ref()
            .expect("sender present until drop")
            .send(Cmd::Update(msg))
            .map_err(|_| WriteError::Shutdown)
    }
}

impl<U: Send + 'static, S: Send + Sync + 'static> Drop for Writer<U, S> {
    fn drop(&mut self) {
        // Closing the channel ends the thread's loop after it drains + commits
        // whatever batch is in flight (the disconnect also closes an open window
        // early — no point waiting for arrivals that can no longer happen).
        drop(self.tx.take());
        if let Some(t) = self.thread.take() {
            // A panicked writer thread already failed its submitters via their
            // dropped ack channels; never panic in drop ourselves.
            let _ = t.join();
        }
    }
}

/// (sq-ft7u) A restore request held until the in-flight batch commits: the
/// freshly-built snapshot to install + the ack channel for the published generation number.
type PendingRestore<A> = (
    <A as ApplyUpdates>::Snapshot,
    SyncSender<Result<u64, String>>,
);

/// The writer thread: collect a batch per group-commit window, commit, repeat.
/// (sq-bdaw5) `hook`, when present, fires once per commit-publish
/// (never for a restore publish — see `Writer::spawn_with_commit_hook`).
fn run<A: ApplyUpdates, H>(
    ring: Arc<GenerationRing<A::Snapshot>>,
    mut applier: A,
    config: WriterConfig,
    rx: Receiver<Cmd<A::Update, A::Snapshot>>,
    mut hook: Option<H>,
) where
    H: FnMut(&Generation<A::Snapshot>, &Generation<A::Snapshot>),
{
    let max_batch = config.max_batch.max(1);
    loop {
        // Block until the first command arrives. An update opens a group-commit window; a
        // maintenance request runs immediately (no pending batch to flush — it is the first
        // thing seen this iteration).
        let first = match rx.recv() {
            Ok(c) => c,
            Err(_) => return, // all senders gone, nothing queued: done
        };
        let first = match first {
            Cmd::Update(m) => m,
            // (sq-x32t) Maintenance arrived with no pending batch: run it now.
            Cmd::Maintain(done) => {
                let _ = done.send(applier.maintain());
                continue;
            }
            // (sq-ft7u) Restore arrived with no pending batch: run it now.
            Cmd::Restore { fresh, ack } => {
                run_restore(&ring, &mut applier, *fresh, ack);
                continue;
            }
        };
        let deadline = Instant::now() + config.window;
        let mut batch = vec![first];
        let mut disconnected = false;
        // (sq-x32t) A maintenance request that arrives mid-window CLOSES the window
        // (we commit the batch collected so far) and is then run AFTER the commit, preserving
        // strict FIFO: every update queued before the maintenance request is committed first,
        // so a compaction observes exactly the writes that preceded it. The request is held
        // here and serviced once the batch is committed below.
        let mut pending_maintain: Option<SyncSender<Result<(), String>>> = None;
        // (sq-ft7u) Likewise a RESTORE request that arrives mid-window closes the
        // window: the in-flight batch is committed FIRST (strict FIFO), then the durable store is
        // replaced — so a restore never races, or is reordered against, a concurrent update.
        let mut pending_restore: Option<PendingRestore<A>> = None;
        while batch.len() < max_batch {
            // (sq-p7kk5) In ADAPTIVE mode we never wait the window: after the
            // first update we drain only what is ALREADY queued (`try_recv`) and commit the
            // moment the backlog empties. A serial client (blocked on its own ack) has an
            // empty queue here, so it commits in engine-time instead of paying ~window per
            // update; a concurrent stream still batches everything that piled up between
            // iterations. In windowed mode we block up to the remaining window as before.
            let next = if config.adaptive_commit {
                match rx.try_recv() {
                    Ok(cmd) => Ok(cmd),
                    // Nothing more queued: close the batch NOW (semantically the window's
                    // Timeout — commit what we have).
                    Err(mpsc::TryRecvError::Empty) => Err(RecvTimeoutError::Timeout),
                    Err(mpsc::TryRecvError::Disconnected) => Err(RecvTimeoutError::Disconnected),
                }
            } else {
                let now = Instant::now();
                if now >= deadline {
                    break;
                }
                rx.recv_timeout(deadline - now)
            };
            match next {
                Ok(Cmd::Update(m)) => batch.push(m),
                Ok(Cmd::Maintain(done)) => {
                    pending_maintain = Some(done);
                    break; // close the window; commit the batch, then run maintenance
                }
                Ok(Cmd::Restore { fresh, ack }) => {
                    pending_restore = Some((*fresh, ack));
                    break; // close the window; commit the batch, then run the restore
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    disconnected = true; // drain: commit what we have, then exit
                    break;
                }
            }
        }
        match config.granularity {
            CommitGranularity::Window => commit(&ring, &mut applier, batch, &mut hook),
            CommitGranularity::CommuteGroup => {
                for group in split_commute_groups(&applier, batch) {
                    commit(&ring, &mut applier, group, &mut hook);
                }
            }
        }
        // (sq-x32t) Run the maintenance op AFTER the preceding batch is committed +
        // published (so a compaction folds in every write that arrived before it), then ack the
        // caller. A maintenance failure leaves the writer thread alive (it is reported via the
        // returned `Result`, not by tearing down the writer).
        if let Some(done) = pending_maintain {
            let _ = done.send(applier.maintain());
        }
        // (sq-ft7u) Run the restore AFTER the preceding batch is committed + published
        // (strict FIFO — the restore observes every write that arrived before it, then replaces
        // the store), then ack the caller. A restore failure leaves the writer alive (it is
        // reported via the returned `Result`, not by tearing down the writer).
        if let Some((fresh, ack)) = pending_restore {
            run_restore(&ring, &mut applier, fresh, ack);
        }
        if disconnected {
            return;
        }
    }
}

/// (sq-ft7u) Run a durable RESTORE on the writer thread and PUBLISH the restored
/// snapshot as a new generation so the ring serves it. The applier's
/// [`restore_durable`](ApplyUpdates::restore_durable) performs the crash-safe on-disk swap and
/// returns the snapshot to publish; on its `Err` nothing is published and the OLD store is intact
/// (the swap is fail-closed). The publish uses an empty epoch-touched set: a full-store restore
/// invalidates everything, and the generation bump alone re-evaluates active subscriptions.
///
/// (sq-bdaw5) A restore publish deliberately does NOT fire the commit hook: the
/// restored snapshot does not share the predecessor's writer lineage, so a lineage-diffing
/// consumer (the change stream) must never be handed the pair (`Writer::spawn_with_commit_hook`).
fn run_restore<A: ApplyUpdates>(
    ring: &GenerationRing<A::Snapshot>,
    applier: &mut A,
    fresh: A::Snapshot,
    ack: SyncSender<Result<u64, String>>,
) {
    let result = match applier.restore_durable(fresh) {
        Ok(snapshot) => Ok(ring.publish(snapshot, std::iter::empty::<PodId>()).number()),
        Err(e) => Err(e),
    };
    let _ = ack.send(result);
}

/// Splits one window's batch into maximal runs of pairwise-commuting data
/// updates, preserving arrival order (no reordering — commutativity licenses
/// *grouping*, the FIFO application inside each group stays the serial order).
/// A barrier always forms a singleton group; a data update conflicting with the
/// current group (it inserts into a graph the group deletes from, or deletes
/// from a graph the group inserts into) starts a new group. Concatenating the
/// groups reproduces the batch exactly.
fn split_commute_groups<A: ApplyUpdates>(
    applier: &A,
    batch: Vec<Msg<A::Update>>,
) -> Vec<Vec<Msg<A::Update>>> {
    let mut groups: Vec<Vec<Msg<A::Update>>> = Vec::new();
    let mut cur: Vec<Msg<A::Update>> = Vec::new();
    // The current group's accumulated graph-level write sets (data groups only).
    let mut cur_inserts: FxHashSet<TargetGraph> = FxHashSet::default();
    let mut cur_deletes: FxHashSet<TargetGraph> = FxHashSet::default();
    for msg in batch {
        match applier.footprint(&msg.update) {
            Footprint::Data { inserts, deletes } => {
                let commutes_with_group =
                    inserts.is_disjoint(&cur_deletes) && deletes.is_disjoint(&cur_inserts);
                if !cur.is_empty() && !commutes_with_group {
                    groups.push(std::mem::take(&mut cur));
                    cur_inserts.clear();
                    cur_deletes.clear();
                }
                cur_inserts.extend(inserts);
                cur_deletes.extend(deletes);
                cur.push(msg);
            }
            Footprint::Barrier => {
                if !cur.is_empty() {
                    groups.push(std::mem::take(&mut cur));
                    cur_inserts.clear();
                    cur_deletes.clear();
                }
                groups.push(vec![msg]);
            }
        }
    }
    if !cur.is_empty() {
        groups.push(cur);
    }
    groups
}

/// Applies one batch to a fresh working copy and publishes ONE generation
/// (module docs: FIFO order, failed updates skipped + re-fork recovery, epochs =
/// union of successful updates' pods, no publish if nothing succeeded).
/// (sq-bdaw5) On publish, `hook` fires with `(base, published)` BEFORE
/// the batch's acks are sent — so a hook-side durable effect (a change-stream
/// record) is complete by the time any submitter's `submit` returns.
fn commit<A: ApplyUpdates, H>(
    ring: &GenerationRing<A::Snapshot>,
    applier: &mut A,
    batch: Vec<Msg<A::Update>>,
    hook: &mut Option<H>,
) where
    H: FnMut(&Generation<A::Snapshot>, &Generation<A::Snapshot>),
{
    let base = ring.current();
    let mut errs: Vec<Option<String>> = (0..batch.len()).map(|_| None).collect();

    let mut working = match applier.fork(base.snapshot()) {
        Ok(w) => w,
        Err(e) => return fail_all(batch, &format!("snapshot fork failed: {e}")),
    };
    // Indexes (submission order) of updates successfully applied to `working`.
    let mut applied: Vec<usize> = Vec::with_capacity(batch.len());

    for i in 0..batch.len() {
        match applier.apply(&mut working, &batch[i].update) {
            Ok(()) => applied.push(i),
            Err(e) => {
                errs[i] = Some(e);
                // `working` may be partially mutated: rebuild and replay.
                match replay(applier, base.snapshot(), &batch, &mut applied, &mut errs) {
                    Ok(w) => working = w,
                    Err(e) => return fail_all(batch, &format!("snapshot fork failed: {e}")),
                }
            }
        }
    }

    if applied.is_empty() {
        // Every update failed: nothing changed, publish nothing (module docs).
        for (i, msg) in batch.into_iter().enumerate() {
            if let Some(ack) = msg.ack {
                let e = errs[i].take().expect("unapplied update carries its error");
                let _ = ack.send(Err(WriteError::Rejected(e)));
            }
        }
        return;
    }

    // (sq-vpx4) Seal is the durable-commit point for an applier with a
    // `--persist` mirror. On `Err` the batch did NOT durably commit: publish NOTHING
    // (so no reader ever sees it) and ack NOTHING with success. We then `return` — the
    // writer thread survives the failure, so reads keep flowing from the last published
    // generation and a subsequent batch is tried once durability recovers. This turns a
    // transient durable-write error from a process-killing panic into a refused-but-honest
    // write, WITHOUT weakening fail-closed: a write that didn't durably commit is still
    // never acked 2xx nor published.
    //
    // Failure semantics are per-submitter, NOT batch-wide: only submitters whose writes
    // were actually pending durable commit (they applied cleanly — `errs[i] == None`) get
    // the retryable `Unavailable` (503-class). Submitters already rejected during `apply`
    // (`errs[i] == Some(_)`, e.g. a malformed/unauthorized update) keep their original
    // `Rejected` reason — a deterministic 400-class error must NOT be masked as a retryable
    // 503 just because an unrelated peer in the same batch failed to seal.
    let snapshot = match applier.seal(working) {
        Ok(s) => s,
        Err(e) => return fail_batch_after_seal_error(batch, errs, &e),
    };
    let touched = applied
        .iter()
        .flat_map(|&i| batch[i].touched.iter().cloned());
    let published = ring.publish(snapshot, touched);
    // (sq-bdaw5) Fire the commit hook (once per published generation, on this
    // — the only publishing — thread) BEFORE acking, so a hook-side durable record is
    // complete when submitters unblock. `base` is the generation this commit forked
    // from; since only this thread publishes, `(base, published)` pairs chain gapless
    // across commits — the same-lineage contract a change-stream recorder relies on.
    if let Some(h) = hook.as_mut() {
        h(base.as_ref(), published.as_ref());
    }
    let number = published.number();
    for (i, msg) in batch.into_iter().enumerate() {
        if let Some(ack) = msg.ack {
            let _ = ack.send(match errs[i].take() {
                Some(e) => Err(WriteError::Rejected(e)),
                None => Ok(number),
            });
        }
    }
}

/// Failure recovery: re-fork the base and replay the already-successful updates
/// in submission order. An update that fails on replay (anomalous: it succeeded
/// once against the same prefix) is demoted to failed with an explanatory error
/// and the replay restarts — `applied` strictly shrinks, so this terminates.
fn replay<A: ApplyUpdates>(
    applier: &mut A,
    base: &A::Snapshot,
    batch: &[Msg<A::Update>],
    applied: &mut Vec<usize>,
    errs: &mut [Option<String>],
) -> Result<A::Working, String> {
    'rebuild: loop {
        let mut working = applier.fork(base)?;
        for k in 0..applied.len() {
            let i = applied[k];
            if let Err(e) = applier.apply(&mut working, &batch[i].update) {
                errs[i] = Some(format!(
                    "update applied cleanly but failed on deterministic replay \
                     (non-deterministic ApplyUpdates::apply?): {e}"
                ));
                applied.remove(k);
                continue 'rebuild;
            }
        }
        return Ok(working);
    }
}

/// Fails every pending submitter in the batch with `Rejected(reason)` — used
/// when no working copy can be produced at all (fork failure).
fn fail_all<U>(batch: Vec<Msg<U>>, reason: &str) {
    for msg in batch {
        if let Some(ack) = msg.ack {
            let _ = ack.send(Err(WriteError::Rejected(reason.to_string())));
        }
    }
}

/// (sq-vpx4) Resolves a batch when [`ApplyUpdates::seal`] could not make it
/// durable. Failure semantics are per-submitter, preserving the rejected-vs-pending
/// distinction:
///   * a submitter rejected during `apply` (`errs[i] == Some(reason)`) keeps that
///     original `Rejected` reason — a deterministic 400-class error is NOT laundered into
///     a retryable 503 just because an unrelated peer failed to seal;
///   * a submitter that applied cleanly (`errs[i] == None`) was pending durable commit, so
///     it gets [`WriteError::Unavailable`] (a retryable 503-class refusal of an otherwise
///     valid write).
///
/// Crucially the writer thread does NOT exit, so reads stay served and later writes can
/// recover, and fail-closed is preserved: nothing was published or acked 2xx.
fn fail_batch_after_seal_error<U>(batch: Vec<Msg<U>>, mut errs: Vec<Option<String>>, reason: &str) {
    for (i, msg) in batch.into_iter().enumerate() {
        if let Some(ack) = msg.ack {
            let _ = ack.send(match errs[i].take() {
                Some(rejected) => Err(WriteError::Rejected(rejected)),
                None => Err(WriteError::Unavailable(reason.to_string())),
            });
        }
    }
}
