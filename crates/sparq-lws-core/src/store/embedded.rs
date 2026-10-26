// AUTHORED-BY Claude Opus 4.8
//! The **embedded** [`SparqClient`] — the SPARQ query engine consumed IN-PROCESS as a library.
//!
//! [`EmbeddedSparqClient`] implements the authoritative-RDF seam by calling the `sparq-engine`
//! query/update entry points DIRECTLY against an in-process [`Graph`] (`sparq-core`), rather than
//! over SPARQ's HTTP service (the `HttpSparqClient` path — opt-in `http-sparq` feature; a code
//! span, since that module is compiled out of the default build). It is the FIRST-CLASS
//! `SparqClient` impl alongside the opt-in HTTP client and the
//! [`InMemorySparqClient`](crate::store::InMemorySparqClient) test double, selected at boot by
//! `PSS_SPARQ_BACKEND=embedded` (see `main.rs`). Behind the `embedded-sparq` build feature, ON BY
//! DEFAULT since sq-gg0qq.3 (this crate lives in the sparq workspace, so the in-process engine
//! binding is the default backend); `--no-default-features` builds the engine-free profile. See
//! `research/lws-design-records.md` §3 (RSS `decisions/0001-embed-sparq-in-process.md`).
//!
//! ## Why this is a net simplification over the HTTP path
//! - **Same queries, different transport.** Every query/update is built by the SAME injection-safe
//!   builders in [`super::sparql`] that the HTTP client uses — VERBATIM, no new query strings. So the
//!   conformance-equivalence to the HTTP/in-memory impls is trivial: identical SPARQL, identical
//!   named-graph model, only the execution path differs (in-process engine call vs an HTTP POST).
//! - **Named-graph isolation is REAL here (fixes the HTTP path's DEVIATION-1).** The engine's
//!   `query`/`update_in_place` fully support `GRAPH <g> { … }` over a single [`Graph`] that holds the
//!   default graph PLUS named graphs (graph IRI == resource IRI, the WAC-design model). The live
//!   HTTP `sparq-server` today folds named graphs into one default graph; embedding sidesteps that.
//! - **No marker/follow-up-ASK atomicity dance.** The HTTP impl needs per-operation create/delete
//!   markers + follow-up ASKs because a SPARQL UPDATE over HTTP cannot return rows, so the outcome
//!   (`NotFound`/`NotEmpty`/`Deleted`/created-or-not) must be probed afterwards — racing a concurrent
//!   mutation unless a nonce nothing else touches is used. IN-PROCESS, the whole op runs as ONE
//!   indivisible actor job on the [`Graph`]'s owning thread, so `create_child`/`delete_meta_if_empty`
//!   decide and return their outcome DIRECTLY (check-then-act with no interleaving) — the simpler,
//!   correct semantics.
//!
//! ## The sync↔tokio bridge: a dedicated-OS-thread actor owning the `Graph`
//! The [`SparqClient`] trait is `#[async_trait]` and the server is tokio, but the engine is a
//! BLOCKING, CPU-bound library (no async I/O — it computes against in-RAM indexes). Running an engine
//! call directly on a tokio worker would block the reactor. So the [`Graph`] is **owned outright by
//! one dedicated OS thread** — the private `GraphActor` (a code span, since it is an implementation
//! detail rather than public API) — and every engine call is a job sent to it over a tokio mpsc
//! channel; the caller awaits a `oneshot` reply. This mirrors the in-tree off-reactor precedent
//! (`redis_replay`'s dedicated-thread pattern + the verifier's `net.rs`, both compiled out of the
//! default build so named as code spans here).
//!
//! This REPLACES the first slice's `tokio::task::spawn_blocking` over an `Arc<Mutex<Graph>>`
//! (sq-gg0qq.3). The actor is the production shape because:
//! - **No lock on the `Graph`.** The `Graph` is not shared, so there is no `Mutex` to acquire per
//!   call, no `Arc` clone per call, and no poisoned-lock class of error. (Stated precisely: this
//!   moves the synchronisation, it does not delete it — the mpsc channel has its own internal
//!   synchronisation. What goes away is a lock held ACROSS the whole engine call, and the work a
//!   blocked waiter did to acquire it.)
//! - **No blocking-pool hop.** A job runs on the actor thread directly instead of being handed to a
//!   tokio blocking-pool thread (a fresh thread per burst, and a different thread each call).
//! - **One serialisation point.** Every mutation passes through a single thread in FIFO channel
//!   order. Directory-backed writes commit their resolved effects through the graph's redo journal
//!   before replying, so the actor sees every write exactly once, in commit order.
//!
//! These are STRUCTURAL properties of the design, not a measured speed-up: this change has not been
//! benchmarked, and the degree of serialisation is unchanged (the `Mutex` already admitted exactly
//! one engine call at a time, so no read parallelism is lost OR gained).
//!
//! Behavioural equivalence with the lock-based slice is preserved deliberately: jobs are serialised
//! (one at a time, in order), a whole job is indivisible (the check-then-act ops above still see no
//! interleaving), and a panicking job **poisons** the actor — it stops serving, and every subsequent
//! call fails closed with a [`SparqError::Backend`], exactly as a poisoned `Mutex` did. FIFO order is
//! the one strict improvement: `Mutex` wakeups were arbitrary, so a caller could be starved.
//!
//! The actor thread exits when the last [`EmbeddedSparqClient`] clone drops (the channel closes) —
//! no explicit shutdown handshake, no leaked thread.
//!
//! ## Persistence
//! [`EmbeddedSparqClient::in_memory`] creates an ephemeral graph. [`EmbeddedSparqClient::open`]
//! strictly opens an existing store; [`EmbeddedSparqClient::open_or_create`] also bootstraps an
//! empty directory. Mutators evaluate on a private fork, then commit the resolved effects to the
//! original graph. Directory-backed writes use one fsync'd `txn.log` frame before materializing
//! the per-graph WALs; acknowledged writes survive reopening without a snapshot or compaction.
//! A possibly applied commit failure or any compaction failure stops all subsequent actor jobs
//! (reads and writes) with the original cause until the client is reopened for recovery.
//! [`EmbeddedSparqClient::save`] is a snapshot helper for tests, not the production durability path.

use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use sparq_core::Graph;
use tokio::sync::{mpsc, oneshot};

use super::sparq::{DeleteOutcome, ReadPlan, ResourceMeta, SparqClient, SparqError};
use super::sparql;
use super::timestamp;

/// The OS-thread name the [`Graph`]-owning actor runs under. Named so a `ps`/profiler/crash dump
/// attributes engine CPU to the data path, and so the tests can assert that engine work really does
/// run on the dedicated thread (not a tokio worker or blocking-pool thread).
const ACTOR_THREAD_NAME: &str = "sparq-embedded-graph";

/// The fail-closed error every call gets once the actor is gone — it either poisoned itself on a
/// panicking job (the `Mutex`-poisoning analogue) or the thread could not be kept alive.
const ACTOR_STOPPED: &str = "fatal: embedded graph actor stopped (a prior engine op panicked)";

/// Published only after the saved graph and its directory entries have been synced.
const INIT_COMPLETE: &str = "init-complete";

/// One unit of work for the [`GraphActor`]: a closure run ON the actor thread with EXCLUSIVE
/// `&mut Graph` access. The job is type-erased (it carries and completes its OWN reply channel
/// internally), so the channel stays monomorphic across every differently-typed engine call.
/// The second argument holds the sticky reopen cause from a commit or compaction failure, and the
/// call wrapper refuses every later job before it can access the graph.
type Job = Box<dyn FnOnce(&mut Graph, &mut Option<String>) + Send + 'static>;

/// The dedicated OS thread that OWNS the [`Graph`] and serves engine ops over an mpsc channel.
///
/// Owning (not sharing) the `Graph` is the point: no `Mutex` is held across an engine call and no
/// `Arc` is cloned per call, and because the thread runs jobs strictly one at a time in channel
/// order, a whole job is indivisible — the check-then-act ops keep exactly the atomicity the held
/// lock gave them.
struct GraphActor {
    /// The job queue. Unbounded is safe here without backpressure risk: every caller `await`s its own
    /// reply, so a task can have at most ONE job outstanding — the queue is bounded in practice by
    /// the number of in-flight requests, which the server's own concurrency limits already cap.
    tx: mpsc::UnboundedSender<Job>,
}

impl GraphActor {
    /// Move `graph` onto a fresh dedicated OS thread and start serving jobs.
    ///
    /// The thread runs until the channel closes (i.e. until the last [`EmbeddedSparqClient`] clone
    /// drops the sender), so there is no shutdown handshake to get wrong and no leaked thread. A
    /// spawn failure (the OS refused a thread) is a fail-closed backend error rather than a panic.
    fn spawn(graph: Graph) -> Result<Self, SparqError> {
        let (tx, rx) = mpsc::unbounded_channel::<Job>();
        std::thread::Builder::new()
            .name(ACTOR_THREAD_NAME.to_string())
            .spawn(move || actor_loop(graph, rx))
            .map_err(|e| {
                SparqError::Backend(format!(
                    "fatal: embedded graph actor thread spawn failed: {}",
                    e
                ))
            })?;
        Ok(Self { tx })
    }

    /// Send one engine closure to the actor and await its result off the reactor.
    ///
    /// Both failure modes — the send (actor gone) and the reply (actor died mid-job, dropping the
    /// reply sender during unwinding) — map to the SAME fail-closed [`SparqError::Backend`], so a
    /// caller can never mistake a dead actor for a successful op.
    async fn call<T, F>(&self, f: F) -> Result<T, SparqError>
    where
        F: FnOnce(&mut Graph, &mut Option<String>) -> Result<T, SparqError> + Send + 'static,
        T: Send + 'static,
    {
        let (reply_tx, reply_rx) = oneshot::channel();
        let job: Job = Box::new(move |graph, needs_reopen| {
            // A dropped receiver means the caller's future was cancelled. The op still ran to
            // completion on the actor thread (same as a dropped `spawn_blocking` handle),
            // including recording any sticky reopen cause; only the result is discarded.
            let result = match needs_reopen {
                Some(cause) => Err(SparqError::Backend(cause.clone())),
                None => f(graph, needs_reopen),
            };
            let _ = reply_tx.send(result);
        });
        self.tx
            .send(job)
            .map_err(|_| SparqError::Backend(ACTOR_STOPPED.into()))?;
        reply_rx
            .await
            .map_err(|_| SparqError::Backend(ACTOR_STOPPED.into()))?
    }
}

/// The actor thread's body: run jobs one at a time until the channel closes.
///
/// A panicking job POISONS the actor — it is caught (so the panic never unwinds out of the thread
/// and is never silently swallowed either), then the loop BREAKS, dropping the receiver. That makes
/// every subsequent `send` fail, so all later calls get the fail-closed [`ACTOR_STOPPED`] error.
/// This deliberately reproduces the previous slice's `Mutex` poisoning: a panic mid-update may have
/// left the `Graph` half-mutated, so the fail-closed posture is to serve nothing further rather than
/// hand out reads of a torn index.
fn actor_loop(mut graph: Graph, mut rx: mpsc::UnboundedReceiver<Job>) {
    // A possibly applied commit or failed compaction keeps its cause for every subsequent job.
    let mut needs_reopen = None;
    // `blocking_recv` is correct here precisely because this thread is NOT a tokio worker — it has
    // no runtime context, so parking it blocks nothing but itself.
    while let Some(job) = rx.blocking_recv() {
        // `AssertUnwindSafe`: on a caught panic we never touch `graph` again (the loop breaks), so a
        // logically-torn index can never be observed — which is exactly the unwind-safety obligation.
        if std::panic::catch_unwind(AssertUnwindSafe(|| job(&mut graph, &mut needs_reopen))).is_err() {
            break;
        }
    }
}

/// A live [`SparqClient`] backed by an IN-PROCESS SPARQ [`Graph`] + the `sparq-engine` query/update
/// entry points. Cheap to clone (the inner `Arc` is shared); construct once and share.
#[derive(Clone)]
pub struct EmbeddedSparqClient {
    /// The dedicated OS thread owning the authoritative RDF index (the default graph + one named
    /// graph per resource IRI). Shared by `Arc` across clones — a clone is the same logical backend,
    /// talking to the same actor, so there is exactly ONE `Graph` and ONE serialisation point per
    /// construction.
    actor: Arc<GraphActor>,
    /// The count of ENGINE ROUND-TRIPS this client has dispatched — one increment per
    /// [`Self::dispatch`], i.e. one per engine job sent to the actor. This is the embedded analogue
    /// of the HTTP client's network round-trips and the deterministic observability the beyond-50k
    /// round-trip work targets: each [`SparqClient`] method costs EXACTLY one dispatch (one job, so a
    /// check-then-act op like `create_child`/`delete_meta_if_empty` — many engine ops inside ONE
    /// indivisible job — is still ONE round-trip). Shared by `Arc` across clones (a clone is the same
    /// logical backend). Exposed via [`Self::engine_round_trips`] for benchmarks/tests; not on the hot
    /// path beyond one relaxed atomic increment.
    round_trips: Arc<AtomicU64>,
}

impl EmbeddedSparqClient {
    /// Build a client over a FRESH, empty in-memory [`Graph`].
    ///
    /// The empty graph is an empty N-Triples parse — the cheapest way to mint a zero-triple `Graph`
    /// without depending on a private constructor. A parse of the empty string cannot fail, but we
    /// surface a [`SparqError::Backend`] rather than panicking if the engine ever changes that.
    pub fn in_memory() -> Result<Self, SparqError> {
        let graph = Graph::load_str("", "ntriples")
            .map_err(|e| SparqError::Backend(format!("fatal: empty graph init failed: {e}")))?;
        Self::over(graph)
    }

    /// Open an existing directory-backed graph, replaying its write-ahead log.
    ///
    /// Unlike [`Self::open_or_create`], this never initializes an empty directory.
    /// A missing initialization marker completes the durability barriers before serving jobs.
    pub fn open(dir: &Path) -> Result<Self, SparqError> {
        // Validate the store and recover any interrupted compaction swap first. The graph
        // stays private until bootstrap recovery has synced it and published its marker.
        let graph = Graph::open(dir).map_err(|e| {
            SparqError::Backend(format!("fatal: graph open({}) failed: {e}", dir.display()))
        })?;
        let io_error =
            |e| SparqError::Backend(format!("fatal: graph init({}) failed: {e}", dir.display()));
        if !dir.join(INIT_COMPLETE).try_exists().map_err(io_error)? {
            // After a restart, existing ancestors may still have unsynced entries from
            // create_dir_all. Their original creation boundary is no longer knowable,
            // so conservatively sync every ancestor through the filesystem root.
            let absolute = std::path::absolute(dir).map_err(io_error)?;
            let ancestors: Vec<_> = absolute
                .ancestors()
                .take_while(|path| path.parent().is_some())
                .map(Path::to_path_buf)
                .collect();
            finish_bootstrap(dir, &ancestors).map_err(io_error)?;
        }
        Self::over(graph)
    }

    /// Open an existing graph or initialize an empty directory with a durable graph.
    ///
    /// An existing `dict-meta.bin` or legacy `dict.bin` selects [`Self::open`], including
    /// its corruption errors and interrupted bootstrap recovery. Initialization publishes
    /// an `init-complete` marker only after syncing the saved graph and directory entries.
    /// A directory containing only a regular `LOCK` file counts as
    /// empty; other non-empty directories are refused without overwriting their contents.
    /// Callers must arrange exclusive access, with a lock outside the graph directory:
    /// compaction replaces that directory. Mutations commit through the directory-backed
    /// graph's redo journal before returning success.
    ///
    /// # Errors
    /// Returns [`SparqError::Backend`] for foreign contents, filesystem errors, an invalid
    /// existing graph, or an actor startup failure.
    pub fn open_or_create(dir: &Path) -> Result<Self, SparqError> {
        let io_error =
            |e| SparqError::Backend(format!("fatal: graph init({}) failed: {e}", dir.display()));
        if dir.join("dict-meta.bin").try_exists().map_err(io_error)?
            || dir.join("dict.bin").try_exists().map_err(io_error)?
        {
            return Self::open(dir);
        }
        // Let Graph::open recover an interrupted directory swap before creating anything.
        // Creating an empty canonical directory here would discard the recoverable store.
        if !dir.try_exists().map_err(io_error)?
            && (dir
                .with_extension("compact-new")
                .try_exists()
                .map_err(io_error)?
                || dir
                    .with_extension("compact-old")
                    .try_exists()
                    .map_err(io_error)?)
        {
            return Self::open(dir);
        }
        // Remember the missing path components before create_dir_all hides which
        // directory entries need to be made durable. An empty relative parent is `.`.
        let mut created_dirs = Vec::new();
        let mut ancestor = dir;
        while !ancestor.try_exists().map_err(io_error)? {
            created_dirs.push(ancestor.to_path_buf());
            ancestor = ancestor
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
        }
        std::fs::create_dir_all(dir).map_err(io_error)?;
        for entry in std::fs::read_dir(dir).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            if entry.file_name() != "LOCK" || !entry.file_type().map_err(io_error)?.is_file() {
                return Err(SparqError::Backend(format!(
                    "fatal: refusing to initialize non-empty non-graph directory {}",
                    dir.display()
                )));
            }
        }
        Graph::load_str("", "ntriples")
            .map_err(|e| engine_err("empty graph init", e))?
            .save(dir)
            .map_err(io_error)?;
        finish_bootstrap(dir, &created_dirs).map_err(io_error)?;
        Self::open(dir)
    }

    /// Hand `graph` to a freshly-spawned [`GraphActor`] and wrap it as a client. The single place a
    /// `Graph` is moved onto its owning thread, shared by the constructors.
    fn over(graph: Graph) -> Result<Self, SparqError> {
        Ok(Self {
            actor: Arc::new(GraphActor::spawn(graph)?),
            round_trips: Arc::new(AtomicU64::new(0)),
        })
    }

    /// The number of engine round-trips (`Self::dispatch` calls) this client — and every clone
    /// sharing its `Arc` — has issued. The deterministic round-trip metric the beyond-50k benchmarks
    /// pin: each [`SparqClient`] read/write method is exactly ONE round-trip, so a combined
    /// [`SparqClient::read_plan`] (one dispatch) vs the trait-default fan-out (`1 + N` `get_meta`
    /// dispatches) is directly observable here.
    pub fn engine_round_trips(&self) -> u64 {
        self.round_trips.load(Ordering::Relaxed)
    }

    /// Dispatch a BLOCKING engine closure to the actor thread, counting it as ONE engine round-trip.
    ///
    /// The single choke point every [`SparqClient`] engine call routes through: it increments the
    /// [`Self::round_trips`] counter (one relaxed atomic add) BEFORE handing the closure to
    /// [`GraphActor::call`], so the round-trip count is the number of engine jobs — the embedded
    /// analogue of the HTTP client's request count. A query that fails to BUILD (a rejected untrusted
    /// IRI) returns before ever reaching here, so a fail-closed rejection is correctly NOT counted as
    /// a round-trip.
    /// The children `q` selects, or `None` once their IRIs pass `max_bytes`.
    async fn children(
        &self,
        q: String,
        max_bytes: usize,
    ) -> Result<Option<Vec<String>>, SparqError> {
        self.dispatch(move |graph, _| {
            let result =
                sparq_engine::query(graph, &q).map_err(|e| engine_err("list_children", e))?;
            let child_col = var_col(&result, "child").ok_or_else(|| {
                SparqError::Backend("fatal: select-children result missing ?child column".into())
            })?;
            let mut children = Vec::with_capacity(result.rows.len());
            let mut bytes = 0usize;
            for row in &result.rows {
                // A `SELECT ?child` row MUST carry a bound `child`. A missing/unbound value is a
                // malformed result, NOT an empty list — surface it as a fatal error (the same
                // fail-closed posture as the HTTP impl), because `list_children` feeds the
                // empty-container DELETE check (a silently-shortened list could wrongly allow a
                // non-empty container's delete).
                let child =
                    term_value(row.get(child_col).and_then(|c| c.as_ref())).ok_or_else(|| {
                        SparqError::Backend(
                            "fatal: select-children row missing the 'child' binding".into(),
                        )
                    })?;
                bytes = bytes.saturating_add(child.len());
                if bytes > max_bytes {
                    return Ok(None);
                }
                children.push(child);
            }
            Ok(Some(children))
        })
        .await
    }

    async fn dispatch<T, F>(&self, f: F) -> Result<T, SparqError>
    where
        F: FnOnce(&mut Graph, &mut Option<String>) -> Result<T, SparqError> + Send + 'static,
        T: Send + 'static,
    {
        self.round_trips.fetch_add(1, Ordering::Relaxed);
        self.actor.call(f).await
    }

    /// Compact the graph and its named graphs on the owning actor thread.
    ///
    /// Logs the parent WAL bytes before and after compaction. Directory-backed graphs
    /// persist the rebuilt base and truncate their WALs; in-memory graphs fold their
    /// overlays. Call after draining writes. Cancelling this future stops waiting, but
    /// cannot interrupt a job already submitted to the actor.
    ///
    /// # Errors
    /// Returns [`SparqError::Backend`] if compaction fails or the actor stops. Any compaction
    /// failure requires reopening the client: a failed directory swap may have detached a WAL,
    /// so every subsequent actor job fails with the original cause.
    pub async fn compact(&self) -> Result<(), SparqError> {
        self.actor
            .call(|graph, needs_reopen| {
                let wal = graph.persist_dir().map(|dir| dir.join("wal.log"));
                let wal_bytes = || {
                    wal.as_ref().and_then(|path| std::fs::metadata(path).ok().map(|m| m.len()))
                };
                let before = wal_bytes();
                eprintln!("  SHUTDOWN: embedded graph compaction starting; parent WAL bytes before={before:?}.");
                let result = graph.compact().map_err(|e| {
                    // The engine exposes no failure stage. Even a named-graph swap can
                    // detach its WAL before failing, leaving the parent directory intact.
                    // Refuse all later jobs rather than acknowledging memory-only writes.
                    let cause = format!(
                        "fatal: embedded engine compact: {e}; store requires reopen because compaction may have detached WAL backing"
                    );
                    *needs_reopen = Some(cause.clone());
                    SparqError::Backend(cause)
                });
                let after = wal_bytes();
                eprintln!(
                    "  SHUTDOWN: embedded graph compaction finished; parent WAL bytes before={before:?} after={after:?}; success={}.",
                    result.is_ok()
                );
                result
            })
            .await
    }

    /// Snapshot the current graph to `dir` for tests. Runs on the
    /// actor thread, so the snapshot cannot interleave with a write — but is deliberately NOT counted
    /// as an engine round-trip (it is an operator/persistence action, not a data-path query), keeping
    /// [`Self::engine_round_trips`] the same metric the counter tests pin.
    pub async fn save(&self, dir: &Path) -> Result<(), SparqError> {
        let dir = dir.to_path_buf();
        self.actor
            .call(move |graph, _| {
                graph.save(&dir).map_err(|e| {
                    SparqError::Backend(format!("fatal: graph save({}) failed: {e}", dir.display()))
                })
            })
            .await
    }
}

/// Publish initialization only after the saved files and directory entries are durable.
/// An interruption before publication leaves no marker, so `open` repeats the sync barriers.
fn finish_bootstrap(dir: &Path, created_dirs: &[std::path::PathBuf]) -> std::io::Result<()> {
    sync_bootstrap(dir, created_dirs)?;
    let path = dir.join(INIT_COMPLETE);
    std::fs::write(&path, b"1\n")?;
    sync_bootstrap_file(&path)?;
    fsync_dir(dir)
}

/// Make a saved graph's bootstrap durable after `Graph::save` has only flushed its files.
/// Sync every regular file, then the graph directory, then the parent of each directory
/// created by `create_dir_all`, deepest first through the first pre-existing ancestor.
/// Recurse through named graphs too: older stores and compaction images lack the marker.
/// Callers supply the missing directories recorded before creation, or every ancestor on
/// retry. Windows skips directory syncing but still syncs every regular file.
/// Propagate every open/fsync error instead of acknowledging durability.
fn sync_bootstrap(dir: &Path, created_dirs: &[std::path::PathBuf]) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_file() {
            sync_bootstrap_file(&entry.path())?;
        } else if kind.is_dir() {
            sync_bootstrap(&entry.path(), &[])?;
        }
    }
    fsync_dir(dir)?;
    for created in created_dirs {
        let parent = created
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fsync_dir(parent)?;
    }
    Ok(())
}

fn sync_bootstrap_file(path: &Path) -> std::io::Result<()> {
    // Windows FlushFileBuffers requires a handle opened with write access.
    std::fs::OpenOptions::new()
        .read(true)
        .write(cfg!(windows))
        .open(path)?
        .sync_all()?;
    // Observe successful sync calls without replacing the real I/O.
    #[cfg(test)]
    tests::BOOTSTRAP_SYNCS.with_borrow_mut(|paths| {
        paths.push((
            path.to_path_buf(),
            path.parent().unwrap().join(INIT_COMPLETE).exists(),
        ))
    });
    Ok(())
}

/// Windows directory syncing is a no-op: NTFS journals directory metadata, and std cannot
/// open directory handles without FILE_FLAG_BACKUP_SEMANTICS. Regular files are still synced.
#[cfg(windows)]
fn fsync_dir(_dir: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(not(windows))]
fn fsync_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::File::open(dir)?.sync_all()?;
    #[cfg(test)]
    tests::BOOTSTRAP_SYNCS
        .with_borrow_mut(|paths| paths.push((dir.to_path_buf(), dir.join(INIT_COMPLETE).exists())));
    Ok(())
}

/// Map an engine error string (the `Result<_, String>` the `sparq-engine` entry points return) into
/// the opaque trait [`SparqError::Backend`]. The engine error is a query/update execution failure on
/// a TRUSTED, builder-constructed query — never untrusted input — so it is a fatal backend condition.
fn engine_err(context: &str, e: String) -> SparqError {
    SparqError::Backend(format!("fatal: embedded engine {context}: {e}"))
}

/// Evaluate an update atomically and commit its effects to the original graph.
///
/// Evaluation errors discard the private fork without changing `graph`. The single durable
/// commit point is `apply_effects_checked`'s fsync of one `txn.log` frame, followed by per-graph
/// WAL materialization. In-memory graphs apply the same effects without journal I/O. A failure
/// after the commit point leaves the journal poisoned for recovery on reopen; never repair it
/// in-process or replace the original graph with its WAL-less fork. Record that failure's
/// cause in the actor state before replying, so later jobs cannot observe partial effects.
fn commit_update(
    graph: &mut Graph,
    needs_reopen: &mut Option<String>,
    update: &str,
    context: &str,
) -> Result<(), SparqError> {
    let mut working = graph.fork();
    let effects = sparq_engine::update_in_place_capturing(
        &mut working,
        update,
        &sparq_engine::QueryBudget::unlimited(),
    )
    .map_err(|e| engine_err(context, e))?;
    sparq_engine::apply_effects_checked(graph, &effects).map_err(|e| {
        if e.applied {
            let cause = format!(
                "fatal: embedded engine {context}: {}; the index must be reopened to recover the poisoned journal",
                e.message
            );
            *needs_reopen = Some(cause.clone());
            SparqError::Backend(cause)
        } else {
            engine_err(context, e.message)
        }
    })
}

/// Extract the lexical string of a SELECT cell ([`oxrdf::Term`]): a literal's value, or an IRI's
/// string. The data-path SELECTs bind either plain literals (the metadata fields) or IRIs (a
/// `?child`); both are returned as their value string, matching the HTTP impl's
/// SPARQL-results-JSON `value` field. A missing/unexpected term yields `None` (the caller maps that
/// to a malformed-result error or a fail-closed `NotFound`).
fn term_value(term: Option<&oxrdf::Term>) -> Option<String> {
    match term? {
        oxrdf::Term::Literal(l) => Some(l.value().to_string()),
        oxrdf::Term::NamedNode(n) => Some(n.as_str().to_string()),
        oxrdf::Term::BlankNode(b) => Some(b.as_str().to_string()),
        // RDF-1.2 quoted triples never appear in a metadata/containment binding; treat as absent.
        _ => None,
    }
}

/// Find the column index of a SELECT variable by name in the result header. The builders bind known
/// variable names (`ct`/`bk`/`etag`/`child`/`bk`), so a missing column is a malformed result.
fn var_col(result: &sparq_engine::QueryResult, name: &str) -> Option<usize> {
    result.vars.iter().position(|v| v.as_str() == name)
}

/// Decode a `select_meta` result into a [`ResourceMeta`]; no row ⇒ [`SparqError::NotFound`].
fn meta_from_result(result: &sparq_engine::QueryResult) -> Result<ResourceMeta, SparqError> {
    // No row ⇒ the resource is not indexed (fail-closed: never invent metadata).
    let row = result.rows.first().ok_or(SparqError::NotFound)?;
    let ct_col = var_col(result, "ct")
        .ok_or_else(|| SparqError::Backend("fatal: meta result missing ?ct column".into()))?;
    let bk_col = var_col(result, "bk")
        .ok_or_else(|| SparqError::Backend("fatal: meta result missing ?bk column".into()))?;
    let et_col = var_col(result, "etag")
        .ok_or_else(|| SparqError::Backend("fatal: meta result missing ?etag column".into()))?;
    let content_type = term_value(row.get(ct_col).and_then(|c| c.as_ref()))
        .ok_or_else(|| SparqError::Backend("fatal: meta row missing contentType".into()))?;
    let blob_key = term_value(row.get(bk_col).and_then(|c| c.as_ref()))
        .ok_or_else(|| SparqError::Backend("fatal: meta row missing blobKey".into()))?;
    let etag = term_value(row.get(et_col).and_then(|c| c.as_ref()))
        .ok_or_else(|| SparqError::Backend("fatal: meta row missing etag".into()))?;
    // `?mod` (`pss:modified`) is OPTIONAL — the column may be absent or unbound. Absent /
    // unbound / unparseable ⇒ `None` (fail OPEN — never a spurious 304).
    let last_modified = var_col(result, "mod")
        .and_then(|col| term_value(row.get(col).and_then(|c| c.as_ref())))
        .and_then(|s| timestamp::from_xsd_datetime(&s));
    Ok(ResourceMeta {
        content_type,
        blob_key,
        etag,
        last_modified,
    })
}

#[async_trait]
impl SparqClient for EmbeddedSparqClient {
    async fn get_meta(&self, iri: &str) -> Result<ResourceMeta, SparqError> {
        // SAME query as the HTTP/in-mem paths — the injection-safe `select_meta` builder VERBATIM.
        let q = sparql::select_meta(iri)?;
        self.dispatch(move |graph, _| {
            let result =
                sparq_engine::query(graph, &q).map_err(|e| engine_err("select_meta", e))?;
            meta_from_result(&result)
        })
        .await
    }

    async fn put_meta(&self, iri: &str, meta: ResourceMeta) -> Result<(), SparqError> {
        // The SAME `update_put_meta` builder — a single `;`-joined update (3 targeted deletes + one
        // insert). Evaluate the whole request on a fork before committing its effects through the
        // original graph's durable commit path, so an evaluation error leaves no partial rewrite.
        let modified = meta.last_modified.and_then(timestamp::to_xsd_datetime);
        let u = sparql::update_put_meta(
            iri,
            &meta.content_type,
            &meta.blob_key,
            &meta.etag,
            modified.as_deref(),
        )?;
        self.dispatch(move |graph, needs_reopen| {
            commit_update(graph, needs_reopen, &u, "put_meta")
        })
        .await
    }

    async fn replace_meta(
        &self,
        iri: &str,
        meta: ResourceMeta,
    ) -> Result<Option<ResourceMeta>, SparqError> {
        // The read of the previous record and the upsert run in ONE actor job, so no other index
        // operation can interleave: the returned record is exactly the one this update replaced.
        let q = sparql::select_meta(iri)?;
        let modified = meta.last_modified.and_then(timestamp::to_xsd_datetime);
        let u = sparql::update_put_meta(
            iri,
            &meta.content_type,
            &meta.blob_key,
            &meta.etag,
            modified.as_deref(),
        )?;
        self.dispatch(move |graph, needs_reopen| {
            let result =
                sparq_engine::query(graph, &q).map_err(|e| engine_err("select_meta", e))?;
            let previous = match meta_from_result(&result) {
                Ok(m) => Some(m),
                Err(SparqError::NotFound) => None,
                Err(e) => return Err(e),
            };
            commit_update(graph, needs_reopen, &u, "put_meta")?;
            Ok(previous)
        })
        .await
    }

    fn raises_markers(&self) -> bool {
        true
    }

    async fn raise_marker(&self, iri: &str, value: i64) -> Result<i64, SparqError> {
        // Read and written in ONE actor job, so no other index operation lands in between.
        let q = sparql::select_meta(iri)?;
        let iri = iri.to_string();
        self.dispatch(move |graph, needs_reopen| {
            let result =
                sparq_engine::query(graph, &q).map_err(|e| engine_err("select_meta", e))?;
            let stored = match meta_from_result(&result) {
                Ok(m) => super::sparq::marker_time(&m),
                Err(SparqError::NotFound) => None,
                Err(e) => return Err(e),
            };
            let raised = stored.map_or(value, |t| t.max(value));
            if stored != Some(raised) {
                let meta = super::sparq::marker_meta(&raised.to_string());
                let u = sparql::update_put_meta(
                    &iri,
                    &meta.content_type,
                    &meta.blob_key,
                    &meta.etag,
                    None,
                )?;
                commit_update(graph, needs_reopen, &u, "raise_marker")?;
            }
            Ok(raised)
        })
        .await
    }

    async fn exists(&self, iri: &str) -> Result<bool, SparqError> {
        let q = sparql::ask_exists(iri)?;
        self.dispatch(move |graph, _| {
            sparq_engine::ask(graph, &q).map_err(|e| engine_err("ask_exists", e))
        })
        .await
    }

    async fn begin_reclaim(&self, blob_key: &str) -> Result<bool, SparqError> {
        // Asked and named in ONE actor job: nothing commits in between.
        let referenced = sparql::ask_blob_referenced(blob_key);
        let u = sparql::update_begin_reclaim(blob_key);
        self.dispatch(move |graph, needs_reopen| {
            if sparq_engine::ask(graph, &referenced).map_err(|e| engine_err("referenced", e))? {
                return Ok(false);
            }
            commit_update(graph, needs_reopen, &u, "begin_reclaim")?;
            Ok(true)
        })
        .await
    }

    async fn end_reclaim(&self, blob_key: &str) -> Result<(), SparqError> {
        let u = sparql::update_end_reclaim(blob_key);
        self.dispatch(move |graph, needs_reopen| {
            commit_update(graph, needs_reopen, &u, "end_reclaim")
        })
        .await
    }

    async fn reclaiming(&self) -> Result<Vec<String>, SparqError> {
        let q = sparql::select_reclaiming();
        self.dispatch(move |graph, _| {
            let result = sparq_engine::query(graph, &q).map_err(|e| engine_err("reclaiming", e))?;
            let col = var_col(&result, "k").ok_or_else(|| {
                SparqError::Backend("fatal: reclaiming result missing ?k column".into())
            })?;
            Ok(result
                .rows
                .iter()
                .filter_map(|row| term_value(row.get(col).and_then(|c| c.as_ref())))
                .collect())
        })
        .await
    }

    async fn delete_meta(&self, iri: &str) -> Result<(), SparqError> {
        // DROP SILENT the resource's whole named graph (idempotent on an absent graph) — the SAME
        // `update_delete_resource` builder. Atomic single-op.
        let u = sparql::update_delete_resource(iri)?;
        self.dispatch(move |graph, needs_reopen| {
            commit_update(graph, needs_reopen, &u, "delete_meta")
        })
        .await
    }

    async fn delete_meta_if_empty(
        &self,
        iri: &str,
        parent: Option<&str>,
    ) -> Result<DeleteOutcome, SparqError> {
        // IN-PROCESS atomicity simplification (the documented difference from the HTTP impl): the
        // whole op is ONE indivisible actor job, so the existence + empty checks and the delete decide
        // their outcome DIRECTLY with no interleaving — NO marker/follow-up-ASK dance is needed. The
        // HTTP path needs the markers because a SPARQL UPDATE over HTTP cannot return rows AND a
        // concurrent op could mutate state between the update and a separate probe; here the actor
        // thread runs one job at a time and OWNS the graph, so check-then-act IS atomic.
        let exists_q = sparql::ask_exists(iri)?;
        let children_q = sparql::select_children(iri)?;
        // Build the conditional-delete update with a nonce the builder requires, even though we do not
        // consult the marker afterwards — the marker INSERT is harmless (it lands in the scratch
        // graph; pruned by the reconciler) and keeps the SAME query string as the HTTP path. The
        // builder's WHERE guard (exists AND empty) still protects the delete; our pre-checks under the
        // lock decide the returned `DeleteOutcome` directly.
        let nonce = next_nonce();
        let delete_u = sparql::update_delete_container_if_empty(iri, parent, &nonce)?;
        self.dispatch(move |graph, needs_reopen| {
            // 1. Exists? (absent ⇒ NotFound, nothing deleted.)
            if !sparq_engine::ask(graph, &exists_q)
                .map_err(|e| engine_err("delete_if_empty/exists", e))?
            {
                return Ok(DeleteOutcome::NotFound);
            }
            // 2. Empty? (any `ldp:contains` member ⇒ NotEmpty, nothing deleted.)
            let children = sparq_engine::query(graph, &children_q)
                .map_err(|e| engine_err("delete_if_empty/children", e))?;
            if !children.rows.is_empty() {
                return Ok(DeleteOutcome::NotEmpty);
            }
            // 3. Exists + empty ⇒ run the guarded atomic delete (container graph + parent edge +
            //    marker). The builder's own WHERE guard re-confirms exists+empty against the same
            //    graph (belt-and-braces; it can only AGREE with our check, since nothing else can run
            //    between the two inside one actor job), then deletes atomically.
            commit_update(graph, needs_reopen, &delete_u, "delete_if_empty/delete")?;
            Ok(DeleteOutcome::Deleted)
        })
        .await
    }

    async fn create_child(
        &self,
        container: &str,
        child: &str,
        meta: ResourceMeta,
    ) -> Result<(), SparqError> {
        // IN-PROCESS atomicity simplification: inside ONE actor job, verify the container exists, then
        // commit the child record + the containment edge atomically. No create-marker/follow-up-ASK
        // is needed (the HTTP path needs them only because an UPDATE over HTTP cannot report whether
        // its container-EXISTS guard matched). The builder's guarded `DELETE/INSERT … WHERE` is still
        // used VERBATIM, so the committed triples are identical to the HTTP path.
        let exists_q = sparql::ask_exists(container)?;
        let nonce = next_nonce();
        let modified = meta.last_modified.and_then(timestamp::to_xsd_datetime);
        let create_u = sparql::update_create_child(
            container,
            child,
            &meta.content_type,
            &meta.blob_key,
            &meta.etag,
            modified.as_deref(),
            &nonce,
        )?;
        self.dispatch(move |graph, needs_reopen| {
            // Container must exist (else 404). Checked inside the same job as the insert, so no
            // concurrent delete can race between this check and the insert.
            if !sparq_engine::ask(graph, &exists_q)
                .map_err(|e| engine_err("create_child/exists", e))?
            {
                return Err(SparqError::NotFound);
            }
            commit_update(graph, needs_reopen, &create_u, "create_child/insert")
        })
        .await
    }

    async fn remove_child(&self, container: &str, child: &str) -> Result<(), SparqError> {
        let u = sparql::update_remove_child(container, child)?;
        self.dispatch(move |graph, needs_reopen| {
            commit_update(graph, needs_reopen, &u, "remove_child")
        })
        .await
    }

    async fn list_children(&self, container: &str) -> Result<Vec<String>, SparqError> {
        let children = self.children(sparql::select_children(container)?, usize::MAX);
        Ok(children.await?.unwrap_or_default())
    }

    async fn list_children_within(
        &self,
        container: &str,
        max_bytes: usize,
    ) -> Result<Option<Vec<String>>, SparqError> {
        // Read from the container's own graph's index rather than through a query: the engine
        // evaluates `GRAPH` whole before a LIMIT applies. The index counts the members exactly
        // before any is read, so a listing with more than can fit is refused unread, and one
        // that fits is read whole (nothing is left out).
        let rows = super::sparq::max_rows(container, max_bytes);
        let name = oxrdf::NamedNode::new(container)
            .map_err(|e| SparqError::Backend(format!("container IRI: {e}")))?;
        self.dispatch(move |graph, _| {
            let Some(g) = graph.named_graph(&oxrdf::Term::NamedNode(name)) else {
                return Ok(Some(Vec::new()));
            };
            let id = |iri: &str| {
                let id = g
                    .dict
                    .lookup(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(
                        iri,
                    )));
                (id != sparq_core::dict::NO_ID).then_some(id)
            };
            let (Some(s), Some(p)) = (id(&sparql::s_record()), id(sparql::LDP_CONTAINS)) else {
                return Ok(Some(Vec::new()));
            };
            let pattern = [Some(s), Some(p), None];
            if g.store.estimate(&pattern) > rows {
                return Ok(None);
            }
            let scan = g.store.scan(&pattern);
            let mut children = Vec::with_capacity(scan.rows.len());
            let mut bytes = 0usize;
            for row in scan.rows.iter() {
                let [_, _, o] = scan.to_spo(row);
                // A member is an IRI; anything else is a malformed index (see `children`).
                let oxrdf::Term::NamedNode(child) = g.dict.term(o) else {
                    return Err(SparqError::Backend(
                        "fatal: a containment member is not an IRI".into(),
                    ));
                };
                bytes = bytes.saturating_add(child.as_str().len());
                if bytes > max_bytes {
                    return Ok(None);
                }
                children.push(child.into_string());
            }
            Ok(Some(children))
        })
        .await
    }

    async fn referenced_blob_keys(&self) -> Result<std::collections::HashSet<String>, SparqError> {
        let q = sparql::select_referenced_blob_keys();
        self.dispatch(move |graph, _| {
            let result = sparq_engine::query(graph, &q)
                .map_err(|e| engine_err("referenced_blob_keys", e))?;
            let bk_col = var_col(&result, "bk").ok_or_else(|| {
                SparqError::Backend("fatal: referenced-blob-keys result missing ?bk column".into())
            })?;
            let mut keys = std::collections::HashSet::with_capacity(result.rows.len());
            for row in &result.rows {
                // A missing `bk` binding is a malformed result — FATAL, never silently dropped: a
                // shortened referenced set would make the reconciler treat a still-referenced blob as
                // an orphan and DELETE live bytes. Fail-closed (the reconciler aborts the sweep).
                let bk = term_value(row.get(bk_col).and_then(|c| c.as_ref())).ok_or_else(|| {
                    SparqError::Backend(
                        "fatal: referenced-blob-keys row missing the 'bk' binding".into(),
                    )
                })?;
                keys.insert(bk);
            }
            Ok(keys)
        })
        .await
    }

    /// ONE combined read-plan lookup in a SINGLE engine round-trip — the embedded analogue of the
    /// HTTP client's one combined SELECT (`super::http::HttpSparqClient::read_plan`), replacing the
    /// trait DEFAULT's `1 + N` sequential `get_meta` dispatches with ONE.
    ///
    /// It executes the SAME injection-safe [`sparql::select_read_plan`] query the HTTP client uses,
    /// VERBATIM — `VALUES ?g { <target> <cand₁> … } GRAPH ?g { <record> … }` — against the in-process
    /// engine as ONE actor job (one `Self::dispatch`). The engine JOINs the `VALUES` graph list
    /// with the per-graph record pattern, so the returned rows are EXACTLY the target + present
    /// candidates keyed by graph IRI — semantically identical to probing each IRI with
    /// [`sparql::select_meta`] in turn (the default loop), only in one pass. Every IRI flows through
    /// the fallible `iri` builder inside `select_read_plan`, so an IRIREF-invalid value REJECTS the
    /// whole build (fail-closed) BEFORE any dispatch — never string-concatenated. Fail-closed on
    /// execution too: any engine/parse error fails the WHOLE plan (no per-candidate partial degrade —
    /// trait invariant 4).
    async fn read_plan(
        &self,
        target: &str,
        acl_candidates: &[String],
    ) -> Result<ReadPlan, SparqError> {
        // Build the combined SELECT (fail-closed on any invalid IRI — never reaches the engine).
        let q = sparql::select_read_plan(target, acl_candidates)?;
        let target = target.to_string();
        let candidates: Vec<String> = acl_candidates.to_vec();
        self.dispatch(move |graph, _| {
            let result =
                sparq_engine::query(graph, &q).map_err(|e| engine_err("select_read_plan", e))?;
            // Resolve the result columns once (a row missing a REQUIRED binding is a malformed
            // result — fatal, never silently dropped: dropping a present own-ACL row could wrongly
            // inherit an ancestor's grants, an authz-affecting change).
            let g_col = var_col(&result, "g").ok_or_else(|| {
                SparqError::Backend("fatal: read-plan result missing ?g column".into())
            })?;
            let ct_col = var_col(&result, "ct").ok_or_else(|| {
                SparqError::Backend("fatal: read-plan result missing ?ct column".into())
            })?;
            let bk_col = var_col(&result, "bk").ok_or_else(|| {
                SparqError::Backend("fatal: read-plan result missing ?bk column".into())
            })?;
            let et_col = var_col(&result, "etag").ok_or_else(|| {
                SparqError::Backend("fatal: read-plan result missing ?etag column".into())
            })?;
            let mod_col = var_col(&result, "mod"); // OPTIONAL — may be absent from the header.

            // Key each row by its graph IRI. First row per graph wins (the record is single-valued
            // per graph — `update_put_meta`/`update_create_child` keep it so — mirroring
            // `select_meta`'s `LIMIT 1` determinism and the HTTP client's `entry().or_insert`).
            let mut by_graph: std::collections::HashMap<String, ResourceMeta> =
                std::collections::HashMap::with_capacity(result.rows.len());
            for row in &result.rows {
                let graph_iri =
                    term_value(row.get(g_col).and_then(|c| c.as_ref())).ok_or_else(|| {
                        SparqError::Backend("fatal: read-plan row missing the ?g binding".into())
                    })?;
                let content_type = term_value(row.get(ct_col).and_then(|c| c.as_ref()))
                    .ok_or_else(|| {
                        SparqError::Backend("fatal: read-plan row missing contentType".into())
                    })?;
                let blob_key =
                    term_value(row.get(bk_col).and_then(|c| c.as_ref())).ok_or_else(|| {
                        SparqError::Backend("fatal: read-plan row missing blobKey".into())
                    })?;
                let etag =
                    term_value(row.get(et_col).and_then(|c| c.as_ref())).ok_or_else(|| {
                        SparqError::Backend("fatal: read-plan row missing etag".into())
                    })?;
                // `?mod` (`pss:modified`) is OPTIONAL — an absent column, an unbound cell, or an
                // unparseable value all ⇒ `None` (fail OPEN — never a spurious 304), exactly as
                // `get_meta` treats it.
                let last_modified = mod_col
                    .and_then(|col| term_value(row.get(col).and_then(|c| c.as_ref())))
                    .and_then(|s| timestamp::from_xsd_datetime(&s));
                by_graph.entry(graph_iri).or_insert(ResourceMeta {
                    content_type,
                    blob_key,
                    etag,
                    last_modified,
                });
            }

            // Assemble the plan in the caller's roles: the RAW target's metadata, and one entry per
            // candidate in the caller's (nearest-first) order — `Some(etag)` iff indexed. Absence
            // here is authoritative (the row came from the engine, not a cache), so a missing entry
            // is a genuinely-absent ACL, keeping the "nearest present wins" WAC walk exact.
            Ok(ReadPlan {
                target: by_graph.get(&target).cloned(),
                acls: candidates
                    .iter()
                    .map(|c| (c.clone(), by_graph.get(c).map(|m| m.etag.clone())))
                    .collect(),
            })
        })
        .await
    }
}

/// A process-unique nonce for the marker triples the conditional-delete / create builders require.
/// IN-PROCESS we do not consult the marker (the held lock makes the outcome directly observable), so
/// only uniqueness matters — a monotonic counter combined with the boot-time nanos. Mirrors the HTTP
/// impl's `next_nonce` so the SAME builder is fed an equivalently-unique value.
fn next_nonce() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("op-{nanos:x}-{n:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    std::thread_local! {
        pub(super) static BOOTSTRAP_SYNCS: std::cell::RefCell<Vec<(std::path::PathBuf, bool)>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    /// A runtime so the async `SparqClient` API can be driven from a sync unit test without the
    /// `#[tokio::test]` macro for every case. The multi-thread builder is used so the concurrency
    /// tests below can drive genuinely-parallel callers at the actor; the engine work itself never
    /// needs a runtime worker (or a blocking-pool thread) — it runs on the actor's own OS thread.
    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("test runtime")
            .block_on(f)
    }

    fn client() -> EmbeddedSparqClient {
        EmbeddedSparqClient::in_memory().expect("empty in-memory graph")
    }

    fn meta(ct: &str, bk: &str, etag: &str) -> ResourceMeta {
        ResourceMeta {
            content_type: ct.into(),
            blob_key: bk.into(),
            etag: etag.into(),
            last_modified: None,
        }
    }

    /// Identify the thread a closure is running on: its `ThreadId` (unique for a live thread) plus
    /// its name. Used to prove WHERE engine work executes.
    fn thread_ident() -> (std::thread::ThreadId, Option<String>) {
        let t = std::thread::current();
        (t.id(), t.name().map(|n| n.to_string()))
    }

    /// **The actor invariant this task exists for (sq-gg0qq.13).** Every engine op runs on ONE
    /// dedicated, named OS thread that OWNS the `Graph` — never on the calling reactor thread, and
    /// never on a rotating `spawn_blocking` pool thread (the retired first-slice behaviour, where
    /// consecutive calls land wherever the blocking pool puts them).
    #[test]
    fn every_engine_op_runs_on_the_one_dedicated_actor_thread() {
        block_on(async {
            let c = client();
            let caller = std::thread::current().id();

            let first = c.dispatch(|_g, _| Ok(thread_ident())).await.unwrap();
            // Interleave a REAL data-path op, so the identity claim covers actual engine work and
            // not just back-to-back probes.
            c.put_meta("http://pod/alice/doc", meta("text/turtle", "b", "\"e\""))
                .await
                .unwrap();
            let second = c.dispatch(|_g, _| Ok(thread_ident())).await.unwrap();
            // A CLONE is the same logical backend, so it must reach the SAME actor.
            let via_clone = c.clone().dispatch(|_g, _| Ok(thread_ident())).await.unwrap();

            assert_eq!(
                first.1.as_deref(),
                Some(ACTOR_THREAD_NAME),
                "engine work runs on the named actor thread, got {:?}",
                first.1
            );
            assert_eq!(
                first.0, second.0,
                "every op runs on the SAME owning thread — one graph, one serialisation point"
            );
            assert_eq!(first.0, via_clone.0, "a clone shares the actor thread");
            assert_ne!(
                first.0, caller,
                "the blocking engine never runs on the calling reactor thread"
            );

            // A SECOND client owns a SEPARATE graph, so it must get its own actor thread (the actor
            // is per-graph, not a process-wide singleton).
            let other = client();
            let other_ident = other.dispatch(|_g, _| Ok(thread_ident())).await.unwrap();
            assert_ne!(
                first.0, other_ident.0,
                "distinct clients own distinct graphs on distinct actor threads"
            );
        });
    }

    /// Genuinely-parallel callers are SERIALISED by the actor: every concurrent write lands exactly
    /// once (no lost update, no duplicate containment edge), and the round-trip metric still counts
    /// exactly one dispatch per `SparqClient` call.
    #[test]
    fn concurrent_callers_are_serialised_and_every_write_lands() {
        block_on(async {
            const N: usize = 16;
            let c = client();
            let container = "http://pod/alice/c/";
            c.put_meta(container, meta("text/turtle", "cbk", "\"ce\""))
                .await
                .unwrap();

            let mut tasks = Vec::with_capacity(N);
            for i in 0..N {
                let c = c.clone();
                tasks.push(tokio::spawn(async move {
                    let child = format!("http://pod/alice/c/n{}", i);
                    let bk = format!("bk{}", i);
                    c.create_child(container, &child, meta("text/turtle", &bk, "\"e\""))
                        .await
                }));
            }
            for t in tasks {
                t.await.expect("task joined").expect("create_child");
            }

            let mut children = c.list_children(container).await.unwrap();
            children.sort();
            let expected: Vec<String> = {
                let mut e: Vec<String> = (0..N)
                    .map(|i| format!("http://pod/alice/c/n{}", i))
                    .collect();
                e.sort();
                e
            };
            assert_eq!(
                children, expected,
                "every concurrent create landed exactly once, no duplicates"
            );
            assert_eq!(
                c.engine_round_trips(),
                (N + 2) as u64,
                "one dispatch per call: 1 put_meta + {} create_child + 1 list_children",
                N
            );
        });
    }

    /// Review finding: the index read a container's whole listing before charging it. A listing
    /// read within a budget is counted before it is read, and refused unread when it cannot fit.
    #[test]
    fn a_listing_read_within_a_budget_stops_at_it() {
        block_on(async {
            let c = client();
            let container = "http://pod/alice/c/";
            c.put_meta(container, meta("text/turtle", "cbk", "\"ce\""))
                .await
                .unwrap();
            for i in 0..10 {
                let child = format!("{container}n{i}");
                c.create_child(
                    container,
                    &child,
                    meta("text/turtle", &format!("bk{i}"), "\"e\""),
                )
                .await
                .unwrap();
            }
            let all: usize = c
                .list_children(container)
                .await
                .unwrap()
                .iter()
                .map(String::len)
                .sum();
            assert_eq!(
                c.list_children_within(container, all)
                    .await
                    .unwrap()
                    .map(|v| v.len()),
                Some(10)
            );
            assert_eq!(
                c.list_children_within(container, all - 1).await.unwrap(),
                None
            );
            assert_eq!(c.list_children_within(container, 0).await.unwrap(), None);
            let mut read = c
                .list_children_within(container, all)
                .await
                .unwrap()
                .unwrap();
            let mut listed = c.list_children(container).await.unwrap();
            read.sort();
            listed.sort();
            assert_eq!(read, listed);
            // Review finding: members shorter than their container (not below it) fit more of
            // them in the bytes than the rows asked for, and the rest was silently left out. A
            // listing with more members than can be below it is refused, never cut short.
            let other = "http://pod/alice/d/";
            c.put_meta(other, meta("text/turtle", "dbk", "\"de\""))
                .await
                .unwrap();
            for (i, child) in ["urn:a", "urn:b", "urn:c", "urn:d", "http://pod/alice/d/x"]
                .iter()
                .enumerate()
            {
                c.create_child(other, child, meta("text/plain", &format!("d{i}"), "\"e\""))
                    .await
                    .unwrap();
            }
            let budget = 3 * (other.len() + 1);
            assert_eq!(super::super::sparq::max_rows(other, budget), 3);
            assert_eq!(c.list_children_within(other, budget).await.unwrap(), None);
            let memory = super::super::sparq::InMemorySparqClient::new();
            memory
                .put_meta(other, meta("text/turtle", "dbk", "\"de\""))
                .await
                .unwrap();
            for (i, child) in ["urn:a", "urn:b", "urn:c", "urn:d", "http://pod/alice/d/x"]
                .iter()
                .enumerate()
            {
                memory
                    .create_child(other, child, meta("text/plain", &format!("d{i}"), "\"e\""))
                    .await
                    .unwrap();
            }
            assert_eq!(
                memory.list_children_within(other, budget).await.unwrap(),
                None
            );
        });
    }

    /// A panicking engine op POISONS the actor — the `Mutex`-poisoning parity the previous slice had.
    /// The panicking call itself fails closed, and so does every LATER call (a panic mid-update may
    /// have left the index torn, so serving nothing further is the fail-closed posture).
    ///
    /// NB the panic message the default hook prints while this test runs is EXPECTED output.
    #[test]
    fn a_panicking_engine_op_poisons_the_actor_and_later_calls_fail_closed() {
        block_on(async {
            let c = client();
            let iri = "http://pod/alice/doc";
            c.put_meta(iri, meta("text/turtle", "b", "\"e\""))
                .await
                .unwrap();
            assert!(c.exists(iri).await.unwrap(), "healthy before the panic");

            let err = c
                .dispatch(|_g, _| -> Result<(), SparqError> { panic!("deliberate engine-op panic") })
                .await
                .unwrap_err();
            assert!(
                matches!(err, SparqError::Backend(_)),
                "the panicking op itself fails closed (never a hang, never a bogus Ok): {err:?}"
            );

            // Poisoned: no later op is served, on this handle or a clone. Repeated so a single
            // one-shot error cannot pass for a permanently-closed backend.
            for _ in 0..3 {
                let err = c.exists(iri).await.unwrap_err();
                assert!(
                    matches!(err, SparqError::Backend(_)),
                    "poisoned actor fails closed: {err:?}"
                );
            }
            assert!(
                matches!(c.clone().get_meta(iri).await, Err(SparqError::Backend(_))),
                "a clone shares the poisoned actor"
            );
        });
    }

    struct TestDir(std::path::PathBuf);

    impl TestDir {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("sparq-lws-bootstrap-{}", next_nonce()));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Assert publication follows every bootstrap barrier, then return those earlier syncs.
    fn bootstrap_syncs_before_publication(dir: &Path) -> Vec<std::path::PathBuf> {
        let synced = BOOTSTRAP_SYNCS.with_borrow_mut(std::mem::take);
        let publication_len = if cfg!(windows) { 1 } else { 2 };
        assert!(
            synced.len() >= publication_len,
            "initialization must be published"
        );
        let (barriers, publication) = synced.split_at(synced.len() - publication_len);
        assert_eq!(publication[0], (dir.join("init-complete"), true));
        if !cfg!(windows) {
            assert_eq!(publication[1], (dir.to_path_buf(), true));
        }
        assert_eq!(std::fs::read(dir.join("init-complete")).unwrap(), b"1\n");
        barriers
            .iter()
            .map(|(path, marker_visible)| {
                assert!(
                    !marker_visible,
                    "marker must not precede sync of {}",
                    path.display()
                );
                path.clone()
            })
            .collect()
    }

    #[cfg(windows)]
    #[test]
    fn fsync_dir_is_a_noop_on_windows() {
        let dir = TestDir::new();
        let file = dir.0.join("file");
        std::fs::write(&file, b"unchanged").unwrap();
        for path in [&dir.0, &file, &dir.0.join("missing")] {
            fsync_dir(path).unwrap();
        }
        assert_eq!(std::fs::read(file).unwrap(), b"unchanged");
    }

    #[test]
    fn open_or_create_bootstraps_empty_missing_and_lock_only_directories() {
        let root = TestDir::new();
        let sample = root.0.join("sample");
        Graph::load_str("", "ntriples").unwrap().save(&sample).unwrap();
        let base_files: std::collections::HashSet<_> = std::fs::read_dir(&sample)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        for dir in [
            root.0.join("empty"),
            root.0.join("missing/graph"),
            root.0.join("locked"),
        ] {
            if dir.ends_with("empty") {
                std::fs::create_dir(&dir).unwrap();
                assert!(
                    EmbeddedSparqClient::open(&dir).is_err(),
                    "open stays strict"
                );
            } else if dir.ends_with("locked") {
                std::fs::create_dir(&dir).unwrap();
                std::fs::write(dir.join("LOCK"), b"1234\n").unwrap();
            }
            BOOTSTRAP_SYNCS.with_borrow_mut(Vec::clear);
            let c = EmbeddedSparqClient::open_or_create(&dir).unwrap();
            let synced = bootstrap_syncs_before_publication(&dir);
            let file_count = base_files.len() + usize::from(dir.ends_with("locked"));
            let synced_files: std::collections::HashSet<_> = synced[..file_count]
                .iter()
                .map(|path| {
                    assert_eq!(path.parent(), Some(dir.as_path()));
                    assert!(path.is_file());
                    path.file_name().unwrap().to_owned()
                })
                .collect();
            let mut expected_files = base_files.clone();
            if dir.ends_with("locked") {
                expected_files.insert("LOCK".into());
            }
            assert_eq!(synced_files, expected_files, "sync every saved base file first");
            let mut expected_dirs = if cfg!(windows) {
                vec![]
            } else {
                vec![dir.clone()]
            };
            if !cfg!(windows) && dir.ends_with("graph") {
                expected_dirs.extend([root.0.join("missing"), root.0.clone()]);
            }
            assert_eq!(
                synced[file_count..], expected_dirs,
                "sync the graph directory, then each new directory's parent up to the existing root"
            );
            assert!(dir.join("dict-meta.bin").is_file());
            assert!(Graph::open(&dir).unwrap().is_directory_backed());
            if dir.ends_with("locked") {
                assert_eq!(std::fs::read(dir.join("LOCK")).unwrap(), b"1234\n");
            }
            drop(c);
        }
    }

    #[test]
    fn open_retries_interrupted_bootstrap_before_serving_jobs() {
        block_on(async {
            for open in [
                EmbeddedSparqClient::open,
                EmbeddedSparqClient::open_or_create,
            ] {
                let root = TestDir::new();
                let dir = root.0.join("new/ancestors/graph");
                // Stop exactly after Graph::save, before sync_bootstrap or publication.
                Graph::load_str("", "ntriples").unwrap().save(&dir).unwrap();
                assert!(dir.join("dict-meta.bin").is_file());
                assert!(!dir.join("init-complete").exists());
                BOOTSTRAP_SYNCS.with_borrow_mut(Vec::clear);
                let c = open(&dir).unwrap();
                let synced = bootstrap_syncs_before_publication(&dir);
                let files: std::collections::HashSet<_> = std::fs::read_dir(&dir)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .filter(|path| path.file_name().unwrap() != "init-complete")
                    .collect();
                assert_eq!(
                    synced[..files.len()]
                        .iter()
                        .cloned()
                        .collect::<std::collections::HashSet<_>>(),
                    files
                );
                let dirs: Vec<_> = if cfg!(windows) {
                    vec![]
                } else {
                    dir.ancestors().map(Path::to_path_buf).collect()
                };
                assert_eq!(
                    synced[files.len()..],
                    dirs,
                    "retry must sync even ancestors that now exist"
                );

                let iri = "http://pod/alice/doc";
                let expected = meta("text/plain", "blob-1", "\"e1\"");
                c.put_meta(iri, expected.clone()).await.unwrap();
                drop(c);
                BOOTSTRAP_SYNCS.with_borrow_mut(Vec::clear);
                let reopened = open(&dir).unwrap();
                assert!(
                    BOOTSTRAP_SYNCS.with_borrow(|paths| paths.is_empty()),
                    "a published store needs no bootstrap retry"
                );
                assert_eq!(reopened.get_meta(iri).await.unwrap(), expected);
            }
        });
    }

    #[test]
    fn open_syncs_unmarked_named_graph_trees_without_overwriting_them() {
        block_on(async {
            let root = TestDir::new();
            let dir = root.0.join("graph");
            let iri = "http://pod/alice/doc";
            let expected = meta("text/plain", "blob-1", "\"e1\"");
            let c = client();
            c.put_meta(iri, expected.clone()).await.unwrap();
            c.save(&dir).await.unwrap();
            drop(c);

            BOOTSTRAP_SYNCS.with_borrow_mut(Vec::clear);
            let reopened = EmbeddedSparqClient::open(&dir).unwrap();
            let synced = bootstrap_syncs_before_publication(&dir);
            let named = dir.join("named/0");
            for entry in std::fs::read_dir(&named).unwrap() {
                let path = entry.unwrap().path();
                assert!(
                    synced.contains(&path),
                    "sync named graph file {}",
                    path.display()
                );
                if !cfg!(windows) {
                    assert!(
                        synced.iter().position(|p| p == &path)
                            < synced.iter().position(|p| p == &named)
                    );
                }
            }
            if !cfg!(windows) {
                for (child, parent) in [
                    (&named, dir.join("named")),
                    (&dir.join("named"), dir.clone()),
                ] {
                    assert!(
                        synced.iter().position(|p| p == child).unwrap()
                            < synced.iter().position(|p| p == &parent).unwrap()
                    );
                }
            }
            assert_eq!(reopened.get_meta(iri).await.unwrap(), expected);
        });
    }

    #[test]
    fn open_or_create_preserves_existing_durable_writes() {
        block_on(async {
            let dir = TestDir::new();
            let iri = "http://pod/alice/doc";
            let expected = meta("text/turtle", "blob-1", "\"e1\"");
            let c = EmbeddedSparqClient::open_or_create(&dir.0).unwrap();
            c.put_meta(iri, expected.clone()).await.unwrap();
            drop(c);
            let reopened = EmbeddedSparqClient::open(&dir.0).unwrap();
            assert_eq!(reopened.get_meta(iri).await.unwrap(), expected);
        });
    }

    #[test]
    fn open_or_create_preserves_existing_client_writes() {
        block_on(async {
            let dir = TestDir::new();
            let iri = "http://pod/alice/doc";
            let expected = meta("text/turtle", "blob-1", "\"e1\"");
            let c = EmbeddedSparqClient::open_or_create(&dir.0).unwrap();
            c.put_meta(iri, expected.clone()).await.unwrap();
            drop(c);
            let reopened = EmbeddedSparqClient::open_or_create(&dir.0).unwrap();
            assert_eq!(reopened.get_meta(iri).await.unwrap(), expected);

            let rewritten = meta("application/ld+json", "blob-2", "\"e2\"");
            reopened.put_meta(iri, rewritten.clone()).await.unwrap();
            drop(reopened);
            let reopened = EmbeddedSparqClient::open(&dir.0).unwrap();
            assert_eq!(reopened.get_meta(iri).await.unwrap(), rewritten);

            let replaced = meta("text/plain", "blob-3", "\"e3\"");
            assert_eq!(
                reopened.replace_meta(iri, replaced.clone()).await.unwrap(),
                Some(rewritten)
            );
            let new_iri = "http://pod/alice/new";
            assert_eq!(
                reopened
                    .replace_meta(new_iri, expected.clone())
                    .await
                    .unwrap(),
                None
            );
            drop(reopened);
            let reopened = EmbeddedSparqClient::open_or_create(&dir.0).unwrap();
            assert_eq!(reopened.get_meta(iri).await.unwrap(), replaced);
            assert_eq!(reopened.get_meta(new_iri).await.unwrap(), expected);
        });
    }

    #[test]
    fn failed_update_discards_the_partial_fork_without_committing() {
        block_on(async {
            let dir = TestDir::new();
            let c = EmbeddedSparqClient::open_or_create(&dir.0).unwrap();
            let iri = "http://pod/alice/doc";
            let expected = meta("text/plain", "original", "\"original\"");
            c.put_meta(iri, expected.clone()).await.unwrap();
            let update =
                sparql::update_put_meta(iri, "text/turtle", "changed", "changed", None).unwrap();
            // The first operations change the fork; the final file LOAD is refused by the
            // default policy, without accessing the network. No prefix may reach the live WAL.
            let update = format!("{update}; LOAD <file:///sparq-lws-rollback-denied>");
            let error = c
                .dispatch(move |graph, needs_reopen| {
                    commit_update(graph, needs_reopen, &update, "rollback test")
                })
                .await
                .unwrap_err();
            assert!(matches!(error, SparqError::Backend(_)));
            assert_eq!(c.get_meta(iri).await.unwrap(), expected);
            assert_eq!(std::fs::metadata(dir.0.join("txn.log")).unwrap().len(), 0);
            drop(c);
            let reopened = EmbeddedSparqClient::open(&dir.0).unwrap();
            assert_eq!(reopened.get_meta(iri).await.unwrap(), expected);
        });
    }

    #[test]
    fn partially_materialized_commit_refuses_every_job_until_reopen() {
        block_on(async {
            let dir = TestDir::new();
            let c = EmbeddedSparqClient::open_or_create(&dir.0).unwrap();
            let first = "http://pod/alice/first";
            let second = "http://pod/alice/second";
            let expected = meta("text/plain", "committed", "\"committed\"");
            let update = [first, second]
                .map(|iri| {
                    sparql::update_put_meta(
                        iri,
                        &expected.content_type,
                        &expected.blob_key,
                        &expected.etag,
                        None,
                    )
                    .unwrap()
                })
                .join("; ");
            // The first graph can materialize, but the second hits a real filesystem
            // obstruction AFTER the whole transaction frame has been fsync'd.
            std::fs::create_dir(dir.0.join("named")).unwrap();
            let obstruction = dir.0.join("named/1");
            std::fs::write(&obstruction, b"not a directory").unwrap();
            let error = c
                .dispatch(move |graph, needs_reopen| {
                    commit_update(graph, needs_reopen, &update, "multi-graph test")
                })
                .await
                .unwrap_err();
            let cause = error.to_string();
            assert!(cause.contains("multi-graph test"), "{cause}");
            assert!(cause.contains("index must be reopened"), "{cause}");
            let materialized = Graph::open(&dir.0.join("named/0")).unwrap();
            assert_eq!(materialized.store.len(), 3, "the first record reached its WAL");
            drop(materialized);
            let frame = std::fs::read(dir.0.join("txn.log")).unwrap();
            assert!(!frame.is_empty(), "keep the complete transaction for recovery");

            // Even after the filesystem is repaired, the sticky failure covers clones,
            // reads, writes, snapshots and compaction, and keeps the original cause.
            std::fs::remove_file(obstruction).unwrap();
            let clone = c.clone();
            let refused = "http://pod/alice/refused";
            let snapshot = dir.0.join("refused-snapshot");
            for _ in 0..2 {
                assert_eq!(c.get_meta(first).await.unwrap_err().to_string(), cause);
                assert_eq!(clone.exists(second).await.unwrap_err().to_string(), cause);
                assert_eq!(c.list_children(first).await.unwrap_err().to_string(), cause);
                assert_eq!(
                    clone
                        .put_meta(refused, expected.clone())
                        .await
                        .unwrap_err()
                        .to_string(),
                    cause
                );
                assert_eq!(c.compact().await.unwrap_err().to_string(), cause);
                assert_eq!(c.save(&snapshot).await.unwrap_err().to_string(), cause);
                assert_eq!(std::fs::read(dir.0.join("txn.log")).unwrap(), frame);
            }
            assert!(!snapshot.exists(), "a refused job must not run");
            drop(clone);
            drop(c);
            let reopened = EmbeddedSparqClient::open(&dir.0).unwrap();
            for iri in [first, second] {
                assert_eq!(reopened.get_meta(iri).await.unwrap(), expected);
            }
            assert!(!reopened.exists(refused).await.unwrap());
            assert_eq!(std::fs::metadata(dir.0.join("txn.log")).unwrap().len(), 0);
            reopened.put_meta(refused, expected.clone()).await.unwrap();
            assert_eq!(reopened.get_meta(refused).await.unwrap(), expected);
        });
    }

    #[test]
    fn open_or_create_refuses_foreign_contents_and_corrupt_stores() {
        let dir = TestDir::new();
        std::fs::write(dir.0.join("foreign"), b"preserve me").unwrap();
        assert!(EmbeddedSparqClient::open_or_create(&dir.0).is_err());
        assert_eq!(
            std::fs::read(dir.0.join("foreign")).unwrap(),
            b"preserve me"
        );
        assert!(!dir.0.join("dict-meta.bin").exists());
        std::fs::write(dir.0.join("dict-meta.bin"), b"corrupt").unwrap();
        assert!(EmbeddedSparqClient::open_or_create(&dir.0).is_err());
        assert_eq!(
            std::fs::read(dir.0.join("dict-meta.bin")).unwrap(),
            b"corrupt"
        );
    }

    #[test]
    fn open_or_create_recovers_an_interrupted_compaction() {
        block_on(async {
            let root = TestDir::new();
            let dir = root.0.join("graph");
            let c = EmbeddedSparqClient::open_or_create(&dir).unwrap();
            let iri = "http://pod/alice/doc";
            let expected = meta("text/plain", "blob-2", "\"e2\"");
            c.put_meta(iri, expected.clone()).await.unwrap();
            drop(c);
            std::fs::rename(&dir, dir.with_extension("compact-old")).unwrap();
            let reopened = EmbeddedSparqClient::open_or_create(&dir).unwrap();
            assert_eq!(reopened.get_meta(iri).await.unwrap(), expected);
        });
    }

    /// `save` runs as an actor job (so a snapshot cannot interleave with a write) and is
    /// DELIBERATELY not counted as an engine round-trip — it is a persistence action, not a
    /// data-path query, and the counter tests pin that metric. The snapshot is real: reopening it
    /// yields the same records.
    #[test]
    fn save_snapshots_via_the_actor_without_counting_a_round_trip() {
        block_on(async {
            let c = client();
            let iri = "http://pod/alice/doc";
            c.put_meta(iri, meta("text/turtle", "blob-1", "\"e1\""))
                .await
                .unwrap();
            let before = c.engine_round_trips();

            let dir = std::env::temp_dir().join(format!("sparq-lws-embedded-{}", next_nonce()));
            c.save(&dir).await.expect("snapshot written");
            assert_eq!(
                c.engine_round_trips(),
                before,
                "save is not an engine round-trip"
            );

            let reopened = EmbeddedSparqClient::open(&dir).expect("snapshot reopened");
            assert_eq!(
                reopened.get_meta(iri).await.unwrap(),
                meta("text/turtle", "blob-1", "\"e1\""),
                "the snapshot round-trips the record"
            );
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// The notification marker is as durable as the embedded index is: a snapshot keeps the
    /// value raised before it, and a raise after it is not in the snapshot.
    #[test]
    fn a_marker_is_as_durable_as_the_last_snapshot() {
        block_on(async {
            let c = client();
            let iri = "http://pod/.lws/subscriptions/#high-water";
            assert_eq!(c.raise_marker(iri, 10).await.unwrap(), 10);
            let dir = std::env::temp_dir().join(format!("sparq-lws-embedded-{}", next_nonce()));
            c.save(&dir).await.expect("snapshot written");
            assert_eq!(c.raise_marker(iri, 20).await.unwrap(), 20);
            let reopened = EmbeddedSparqClient::open(&dir).expect("snapshot reopened");
            let stored = reopened.get_meta(iri).await.unwrap();
            assert!(super::super::sparq::is_marker(&stored));
            assert_eq!(
                stored.etag, "10",
                "the snapshot holds the mark as it was saved"
            );
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    #[test]
    fn put_then_get_meta_round_trips() {
        block_on(async {
            let c = client();
            let iri = "http://pod/alice/doc";
            assert!(!c.exists(iri).await.unwrap(), "absent before write");
            assert!(matches!(c.get_meta(iri).await, Err(SparqError::NotFound)));
            c.put_meta(iri, meta("text/turtle", "blob-1", "\"e1\""))
                .await
                .unwrap();
            assert!(c.exists(iri).await.unwrap(), "present after write");
            let got = c.get_meta(iri).await.unwrap();
            assert_eq!(got, meta("text/turtle", "blob-1", "\"e1\""));
        });
    }

    #[test]
    fn put_meta_replaces_not_accumulates() {
        block_on(async {
            let c = client();
            let iri = "http://pod/alice/doc";
            c.put_meta(iri, meta("text/turtle", "blob-1", "\"e1\""))
                .await
                .unwrap();
            c.put_meta(iri, meta("application/ld+json", "blob-2", "\"e2\""))
                .await
                .unwrap();
            // A second put must REPLACE the single-valued record (no duplicate ct/bk/etag), so
            // get_meta is deterministic and returns the latest.
            let got = c.get_meta(iri).await.unwrap();
            assert_eq!(got, meta("application/ld+json", "blob-2", "\"e2\""));
        });
    }

    #[test]
    fn delete_meta_is_idempotent() {
        block_on(async {
            let c = client();
            let iri = "http://pod/alice/doc";
            c.put_meta(iri, meta("text/turtle", "b", "\"e\""))
                .await
                .unwrap();
            c.delete_meta(iri).await.unwrap();
            assert!(!c.exists(iri).await.unwrap());
            // Deleting an absent IRI is Ok (idempotent at the index layer).
            c.delete_meta(iri).await.unwrap();
        });
    }

    #[test]
    fn create_child_records_membership_and_guards_missing_container() {
        block_on(async {
            let c = client();
            let container = "http://pod/alice/c/";
            let child = "http://pod/alice/c/note";
            // A create into a non-indexed container is NotFound.
            assert!(matches!(
                c.create_child(container, child, meta("text/turtle", "bk", "\"e\""))
                    .await,
                Err(SparqError::NotFound)
            ));
            // Index the container, then create — the edge + the child's metadata both land.
            c.put_meta(container, meta("text/turtle", "cbk", "\"ce\""))
                .await
                .unwrap();
            c.create_child(container, child, meta("text/turtle", "bk", "\"e\""))
                .await
                .unwrap();
            assert_eq!(
                c.list_children(container).await.unwrap(),
                vec![child.to_string()]
            );
            assert!(c.exists(child).await.unwrap());
            assert_eq!(
                c.get_meta(child).await.unwrap(),
                meta("text/turtle", "bk", "\"e\"")
            );
        });
    }

    #[test]
    fn delete_meta_if_empty_outcomes() {
        block_on(async {
            let c = client();
            let container = "http://pod/alice/c/";
            let child = "http://pod/alice/c/note";
            // Absent ⇒ NotFound.
            assert_eq!(
                c.delete_meta_if_empty(container, None).await.unwrap(),
                DeleteOutcome::NotFound
            );
            // Indexed + a member ⇒ NotEmpty, nothing deleted.
            c.put_meta(container, meta("text/turtle", "cbk", "\"ce\""))
                .await
                .unwrap();
            c.create_child(container, child, meta("text/turtle", "bk", "\"e\""))
                .await
                .unwrap();
            assert_eq!(
                c.delete_meta_if_empty(container, None).await.unwrap(),
                DeleteOutcome::NotEmpty
            );
            assert!(
                c.exists(container).await.unwrap(),
                "a non-empty container is never deleted"
            );
            // Remove the child (its membership edge + its record — the non-container delete path), so
            // the container becomes empty, then the empty container deletes.
            c.remove_child(container, child).await.unwrap();
            c.delete_meta(child).await.unwrap();
            assert_eq!(
                c.delete_meta_if_empty(container, None).await.unwrap(),
                DeleteOutcome::Deleted
            );
            assert!(!c.exists(container).await.unwrap());
        });
    }

    #[test]
    fn referenced_blob_keys_collects_all_pointers() {
        block_on(async {
            let c = client();
            c.put_meta("http://pod/a", meta("text/turtle", "k1", "\"e\""))
                .await
                .unwrap();
            c.put_meta("http://pod/b", meta("text/turtle", "k2", "\"e\""))
                .await
                .unwrap();
            let keys = c.referenced_blob_keys().await.unwrap();
            assert!(keys.contains("k1") && keys.contains("k2"), "got {keys:?}");
            assert_eq!(keys.len(), 2);
        });
    }

    /// The embedded client names a reclaim only on bytes no record names, a hold's included,
    /// lists what it names, and withdraws it.
    #[test]
    fn a_reclaim_is_named_only_on_unnamed_bytes() {
        block_on(async {
            let c = client();
            c.put_meta("urn:x-sparq-lws:held/a/b", meta("x", "k1", "\"h\""))
                .await
                .unwrap();
            assert!(!c.begin_reclaim("k1").await.unwrap());
            assert!(c.begin_reclaim("k2").await.unwrap());
            assert_eq!(c.reclaiming().await.unwrap(), ["k2"]);
            c.end_reclaim("k2").await.unwrap();
            assert!(c.reclaiming().await.unwrap().is_empty());
        });
    }

    /// The combined `read_plan` override is a SINGLE engine round-trip AND returns exactly the same
    /// `ReadPlan` as the trait-default `1 + N` `get_meta` loop — a round-trip reduction, not a
    /// behaviour change. This is the semantic-equivalence + round-trip-count proof for the WAC read
    /// path (the ACL candidate set the override resolves MUST match the loop's, or an authz decision
    /// could differ).
    #[test]
    fn read_plan_is_one_round_trip_and_matches_the_default_loop() {
        block_on(async {
            let c = client();
            // A doc at depth k = 3 with only the ROOT ACL present among its candidates (the
            // `read_path_counters.rs` / `embedded_read_counters.rs` fixture shape).
            let target = "https://pod.example/alice/c/doc";
            let root_acl = "https://pod.example/.acl";
            c.put_meta(target, meta("text/turtle", "doc-blob", "\"d1\""))
                .await
                .unwrap();
            c.put_meta(root_acl, meta("text/turtle", "acl-blob", "\"a1\""))
                .await
                .unwrap();
            let candidates: Vec<String> = [
                "https://pod.example/alice/c/doc.acl",
                "https://pod.example/alice/c/.acl",
                "https://pod.example/alice/.acl",
                "https://pod.example/.acl",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect();

            // (1) The OVERRIDE: exactly ONE engine round-trip for the whole plan.
            let before = c.engine_round_trips();
            let plan = c.read_plan(target, &candidates).await.unwrap();
            let combined_cost = c.engine_round_trips() - before;
            assert_eq!(
                combined_cost, 1,
                "combined read_plan is ONE engine round-trip (was 1 + N via the default loop)"
            );

            // (2) The plan is CORRECT: target present; only the root ACL present among candidates.
            assert_eq!(
                plan.target,
                Some(meta("text/turtle", "doc-blob", "\"d1\"")),
                "target metadata returned exactly"
            );
            let present: Vec<&String> = plan
                .acls
                .iter()
                .filter(|(_, etag)| etag.is_some())
                .map(|(iri, _)| iri)
                .collect();
            assert_eq!(present, vec![root_acl], "only the root ACL is present");
            // Every candidate is represented, in the caller's nearest-first order.
            assert_eq!(
                plan.acls.iter().map(|(i, _)| i.clone()).collect::<Vec<_>>(),
                candidates,
                "one entry per candidate, order preserved"
            );

            // (3) BASELINE on the SAME client + SAME metric: the sequential `get_meta` probes the
            // default loop performs cost `1 + N` round-trips — the cost the override collapses to 1.
            let before_loop = c.engine_round_trips();
            let loop_target = match c.get_meta(target).await {
                Ok(m) => Some(m),
                Err(SparqError::NotFound) => None,
                Err(e) => panic!("unexpected error: {e}"),
            };
            let mut loop_acls = Vec::with_capacity(candidates.len());
            for cand in &candidates {
                let etag = match c.get_meta(cand).await {
                    Ok(m) => Some(m.etag),
                    Err(SparqError::NotFound) => None,
                    Err(e) => panic!("unexpected error: {e}"),
                };
                loop_acls.push((cand.clone(), etag));
            }
            let loop_cost = c.engine_round_trips() - before_loop;
            assert_eq!(
                loop_cost,
                1 + candidates.len() as u64,
                "the sequential loop costs 1 + N round-trips (N={})",
                candidates.len()
            );
            // The override's plan is byte-for-byte the loop's plan — same metas, same ACL set.
            assert_eq!(
                plan,
                ReadPlan {
                    target: loop_target,
                    acls: loop_acls,
                },
                "combined read_plan is semantically identical to the 1 + N loop"
            );
        });
    }

    /// A `.acl` target (the "two roles" case): the RAW target IS a candidate's IRI. The combined
    /// query DEDUPES it in the `VALUES` list, and the plan still resolves both roles correctly in
    /// ONE round-trip.
    #[test]
    fn read_plan_handles_acl_target_that_is_its_own_candidate() {
        block_on(async {
            let c = client();
            let acl = "https://pod.example/alice/c/.acl";
            c.put_meta(acl, meta("text/turtle", "acl-blob", "\"a1\""))
                .await
                .unwrap();
            // The target IS the acl; its first candidate is the SAME IRI (a GET of `.acl` serves
            // `.acl`'s bytes, and its own ACL chain starts at itself).
            let candidates = vec![acl.to_string(), "https://pod.example/.acl".to_string()];
            let before = c.engine_round_trips();
            let plan = c.read_plan(acl, &candidates).await.unwrap();
            assert_eq!(c.engine_round_trips() - before, 1, "one round-trip");
            assert_eq!(
                plan.target,
                Some(meta("text/turtle", "acl-blob", "\"a1\"")),
                "the .acl target's own bytes-metadata is returned"
            );
            assert_eq!(
                plan.acls,
                vec![
                    (acl.to_string(), Some("\"a1\"".to_string())),
                    ("https://pod.example/.acl".to_string(), None),
                ],
                "the self-candidate resolves present; the absent ancestor resolves None"
            );
        });
    }

    /// An IRIREF-invalid candidate REJECTS the whole build fail-closed BEFORE any dispatch — no
    /// engine round-trip is spent on a malformed (potentially injection-vector) IRI.
    #[test]
    fn read_plan_rejects_invalid_iri_without_dispatching() {
        block_on(async {
            let c = client();
            let before = c.engine_round_trips();
            let err = c
                .read_plan(
                    "https://pod.example/ok",
                    &["https://pod.example/> DROP GRAPH <urn:x".to_string()],
                )
                .await
                .unwrap_err();
            assert!(
                matches!(err, SparqError::Backend(_)),
                "fail-closed: {err:?}"
            );
            assert_eq!(
                c.engine_round_trips(),
                before,
                "a rejected build never reaches the engine — no round-trip counted"
            );
        });
    }
}
