//! Sequenced-writer behavior (§6.5) against an instrumented mock
//! [`ApplyUpdates`]: group-commit batching, FIFO ordering, sync ack generation
//! numbers, failed-update isolation (including the partial-mutation recovery
//! path — the mock deliberately poisons the working copy before failing),
//! window/max-batch closing, graceful drain on drop, and the
//! readers-never-stalled stress.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use sparq_serve::{
    ApplyUpdates, CommitGranularity, GenerationRing, PodId, WriteError, Writer, WriterConfig,
};

/// Snapshot = the log of applied updates, in application order.
type Log = Vec<String>;

/// Mock applier: `fork` clones the base log (instrumented), `apply` appends the
/// update — or, for `"fail:<msg>"` updates, POISONS the working copy first and
/// then fails, exercising the writer's discard-and-replay recovery exactly the
/// way `update_in_place`'s prefix-application hazard would.
struct MockApplier {
    forks: Arc<AtomicUsize>,
    /// Artificial per-fork cost, for the reader-stall stress (simulates the
    /// O(graph) fork of the production applier).
    fork_delay: Duration,
    /// (sq-vpx4) Injected durable-write failures: when > 0, `seal`
    /// fails (returns `Err`) and decrements — modelling a transient `ENOSPC`/I/O
    /// error on a `--persist` mirror that later clears. Lets a test drive the
    /// "writer survives a seal failure, the batch is refused, a later batch after
    /// recovery succeeds" path against the mock.
    seal_fails: Arc<AtomicUsize>,
    /// Counts the seals the writer actually attempted (success or failure), so a
    /// test can assert the writer kept running and re-attempted after a refusal.
    seals: Arc<AtomicUsize>,
    /// (sq-x32t) Records the committed-update count SEEN by each `maintain`
    /// call (proving maintenance is sequenced AFTER the preceding batch); `len()` is a
    /// run-count. When `maintain_fails` > 0, `maintain` returns `Err` once (writer must
    /// stay alive).
    maintains: Arc<std::sync::Mutex<Vec<usize>>>,
    maintain_fails: Arc<AtomicUsize>,
    /// The applier's own count of committed updates (advanced in `seal`), so `maintain`
    /// can record how many it observed.
    committed: usize,
    /// (sq-ft7u) Restore control. When `restore_durable_supported` is true the mock
    /// models a DURABLE applier: `restore_durable` records the committed-update count it observed
    /// (proving FIFO sequencing) and returns the `fresh` snapshot to publish — unless
    /// `restore_fails` > 0, in which case it fails once (consuming one), modelling a crash-safe
    /// swap that left the old store intact. When false it is the IN-MEMORY applier and uses the
    /// trait's default no-op-error `restore_durable`.
    restore_durable_supported: bool,
    restores: Arc<std::sync::Mutex<Vec<usize>>>,
    restore_fails: Arc<AtomicUsize>,
}

impl MockApplier {
    fn new(forks: &Arc<AtomicUsize>) -> Self {
        MockApplier {
            forks: forks.clone(),
            fork_delay: Duration::ZERO,
            seal_fails: Arc::new(AtomicUsize::new(0)),
            seals: Arc::new(AtomicUsize::new(0)),
            maintains: Arc::new(std::sync::Mutex::new(Vec::new())),
            maintain_fails: Arc::new(AtomicUsize::new(0)),
            committed: 0,
            restore_durable_supported: false,
            restores: Arc::new(std::sync::Mutex::new(Vec::new())),
            restore_fails: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// (sq-ft7u) A mock modelling a DURABLE applier (one whose `--persist` store can be
    /// restored through), sharing the restore observation/fault handles with the test so it can
    /// verify the restore is sequenced after preceding updates, publishes a new generation, and
    /// survives an injected restore failure.
    fn with_restore_control(
        forks: &Arc<AtomicUsize>,
        restores: &Arc<std::sync::Mutex<Vec<usize>>>,
        restore_fails: &Arc<AtomicUsize>,
    ) -> Self {
        MockApplier {
            forks: forks.clone(),
            fork_delay: Duration::ZERO,
            seal_fails: Arc::new(AtomicUsize::new(0)),
            seals: Arc::new(AtomicUsize::new(0)),
            maintains: Arc::new(std::sync::Mutex::new(Vec::new())),
            maintain_fails: Arc::new(AtomicUsize::new(0)),
            committed: 0,
            restore_durable_supported: true,
            restores: restores.clone(),
            restore_fails: restore_fails.clone(),
        }
    }

    /// (sq-x32t) A mock sharing the maintenance observation/fault handles with
    /// the test, so the test can verify maintenance ordering and failure-survival.
    fn with_maintain_control(
        forks: &Arc<AtomicUsize>,
        maintains: &Arc<std::sync::Mutex<Vec<usize>>>,
        maintain_fails: &Arc<AtomicUsize>,
    ) -> Self {
        MockApplier {
            forks: forks.clone(),
            fork_delay: Duration::ZERO,
            seal_fails: Arc::new(AtomicUsize::new(0)),
            seals: Arc::new(AtomicUsize::new(0)),
            maintains: maintains.clone(),
            maintain_fails: maintain_fails.clone(),
            committed: 0,
            restore_durable_supported: false,
            restores: Arc::new(std::sync::Mutex::new(Vec::new())),
            restore_fails: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// (sq-vpx4) Build a mock sharing the seal-failure and seal-count
    /// handles with the test, so the test can arm transient durable-write failures
    /// and observe that the writer survived them and re-attempted later.
    fn with_seal_control(
        forks: &Arc<AtomicUsize>,
        seal_fails: &Arc<AtomicUsize>,
        seals: &Arc<AtomicUsize>,
    ) -> Self {
        MockApplier {
            forks: forks.clone(),
            fork_delay: Duration::ZERO,
            seal_fails: seal_fails.clone(),
            seals: seals.clone(),
            maintains: Arc::new(std::sync::Mutex::new(Vec::new())),
            maintain_fails: Arc::new(AtomicUsize::new(0)),
            committed: 0,
            restore_durable_supported: false,
            restores: Arc::new(std::sync::Mutex::new(Vec::new())),
            restore_fails: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl ApplyUpdates for MockApplier {
    type Snapshot = Log;
    type Working = Log;
    type Update = String;

    fn fork(&mut self, base: &Log) -> Result<Log, String> {
        self.forks.fetch_add(1, Ordering::SeqCst);
        if !self.fork_delay.is_zero() {
            std::thread::sleep(self.fork_delay);
        }
        Ok(base.clone())
    }

    fn apply(&mut self, working: &mut Log, update: &String) -> Result<(), String> {
        if let Some(msg) = update.strip_prefix("fail:") {
            working.push("POISON".into()); // partial mutation before the error
            return Err(msg.to_string());
        }
        if update == "panic" {
            panic!("applier panic requested by test");
        }
        working.push(update.clone());
        Ok(())
    }

    fn seal(&mut self, working: Log) -> Result<Log, String> {
        self.seals.fetch_add(1, Ordering::SeqCst);
        // (sq-vpx4) Transient durable-write failure injection: consume one
        // armed failure (if any) and refuse the batch. The writer must NOT publish and
        // must NOT panic — it fails the submitters with `Unavailable` and keeps running.
        if self.seal_fails.load(Ordering::SeqCst) > 0 {
            self.seal_fails.fetch_sub(1, Ordering::SeqCst);
            return Err("injected transient durable-write error (ENOSPC)".into());
        }
        // (sq-x32t) Track how many updates have been committed so a later
        // `maintain` can prove it observed the preceding batch (FIFO sequencing).
        self.committed = working.len();
        Ok(working)
    }

    /// (sq-x32t) Records the committed-update count it observed (so the test can
    /// assert maintenance ran AFTER the preceding updates), and fails once if armed (the
    /// writer must survive and keep serving later maintenance/updates).
    fn maintain(&mut self) -> Result<(), String> {
        self.maintains.lock().unwrap().push(self.committed);
        if self.maintain_fails.load(Ordering::SeqCst) > 0 {
            self.maintain_fails.fetch_sub(1, Ordering::SeqCst);
            return Err("injected maintenance failure".into());
        }
        Ok(())
    }

    /// (sq-ft7u) An IN-MEMORY applier delegates to the trait DEFAULT (no durable store
    /// → an error). A DURABLE applier (built via `with_restore_control`) records the committed-
    /// update count it observed — proving the restore ran AFTER the preceding batch (FIFO) — and
    /// returns the `fresh` snapshot for the writer to publish, unless an injected failure is armed
    /// (then it fails once, modelling a crash-safe swap that left the OLD store intact).
    fn restore_durable(&mut self, fresh: Log) -> Result<Log, String> {
        if !self.restore_durable_supported {
            // Exercise the trait's default no-op-error impl explicitly.
            return ApplyUpdates::restore_durable(
                &mut DefaultRestoreApplier,
                Vec::new(),
            );
        }
        self.restores.lock().unwrap().push(self.committed);
        if self.restore_fails.load(Ordering::SeqCst) > 0 {
            self.restore_fails.fetch_sub(1, Ordering::SeqCst);
            return Err("injected durable restore failure (old store intact)".into());
        }
        // The durable swap succeeded: the published snapshot is the restored image.
        Ok(fresh)
    }
}

/// (sq-ft7u) A trivial applier used ONLY to reach the trait's DEFAULT `restore_durable`
/// (the in-memory-server path that errors because there is no durable store to write through).
struct DefaultRestoreApplier;
impl ApplyUpdates for DefaultRestoreApplier {
    type Snapshot = Log;
    type Working = Log;
    type Update = String;
    fn fork(&mut self, base: &Log) -> Result<Log, String> {
        Ok(base.clone())
    }
    fn apply(&mut self, _working: &mut Log, _update: &String) -> Result<(), String> {
        Ok(())
    }
    fn seal(&mut self, working: Log) -> Result<Log, String> {
        Ok(working)
    }
    // restore_durable: uses the trait default (no-op error) — the path under test.
}

fn pods(ids: &[&str]) -> Vec<PodId> {
    ids.iter().map(|s| PodId::from(*s)).collect()
}

/// N submissions inside one window → ONE generation whose epochs are the union
/// of the touched pods, applied in submission order (FIFO).
#[test]
fn one_window_one_generation_epochs_union() {
    let forks = Arc::new(AtomicUsize::new(0));
    let ring = Arc::new(GenerationRing::new(Log::new()));
    let writer = Writer::spawn(
        ring.clone(),
        MockApplier::new(&forks),
        WriterConfig {
            window: Duration::from_millis(250),
            max_batch: 64,
            ..WriterConfig::default()
        },
    );

    // 7 fire-and-forget + 1 sync, all queued well inside the 250 ms window the
    // first one opens. The sync ack proves the whole batch published together.
    for i in 0..7 {
        writer
            .submit_detached(format!("u{i}"), pods(&[&format!("pod:{i}"), "pod:shared"]))
            .unwrap();
    }
    let generation = writer
        .submit("u7".to_string(), pods(&["pod:7", "pod:shared"]))
        .unwrap();

    assert_eq!(
        generation, 1,
        "all 8 updates collapse into the single generation 1"
    );
    let current = ring.current();
    assert_eq!(current.number(), 1);
    let expected: Log = (0..8).map(|i| format!("u{i}")).collect();
    assert_eq!(
        *current.snapshot(),
        expected,
        "strict FIFO submission order"
    );
    // Union of touched pods, each bumped ONCE per publish (dedup across updates).
    for i in 0..8 {
        assert_eq!(current.epochs().epoch(&PodId::from(format!("pod:{i}"))), 1);
    }
    assert_eq!(current.epochs().epoch(&PodId::from("pod:shared")), 1);
    assert_eq!(current.epochs().len(), 9);
    assert_eq!(
        forks.load(Ordering::SeqCst),
        1,
        "one working copy for the whole batch"
    );
}

/// Sequential sync submits land in consecutive generations and each ack carries
/// the generation that contains it.
#[test]
fn sync_submit_returns_its_generation_number() {
    let forks = Arc::new(AtomicUsize::new(0));
    let ring = Arc::new(GenerationRing::new(Log::new()));
    let writer = Writer::spawn(
        ring.clone(),
        MockApplier::new(&forks),
        WriterConfig {
            window: Duration::from_millis(1),
            max_batch: 256,
            ..WriterConfig::default()
        },
    );

    // Each submit blocks until publish, so the next opens a fresh window.
    assert_eq!(writer.submit("a".into(), pods(&["p"])).unwrap(), 1);
    assert_eq!(writer.submit("b".into(), pods(&["p"])).unwrap(), 2);
    assert_eq!(writer.submit("c".into(), pods(&["q"])).unwrap(), 3);

    let current = ring.current();
    assert_eq!(current.number(), 3);
    assert_eq!(
        *current.snapshot(),
        vec!["a".to_string(), "b".into(), "c".into()]
    );
    assert_eq!(current.epochs().epoch(&PodId::from("p")), 2);
    assert_eq!(current.epochs().epoch(&PodId::from("q")), 1);
}

/// A failing update is reported to its submitter and skipped; the rest of its
/// batch publishes (in order), and the failed update's pods are NOT bumped.
/// The mock poisons the working copy on failure, so this also proves the
/// re-fork-and-replay recovery discards the partial mutation.
#[test]
fn failed_update_is_isolated_and_recovered() {
    let forks = Arc::new(AtomicUsize::new(0));
    let ring = Arc::new(GenerationRing::new(Log::new()));
    let writer = Arc::new(Writer::spawn(
        ring.clone(),
        MockApplier::new(&forks),
        WriterConfig {
            window: Duration::from_millis(300),
            max_batch: 64,
            ..WriterConfig::default()
        },
    ));

    // Deterministic in-window order via staggered sync submitters: a (t≈0),
    // fail (t≈60 ms), c (t≈120 ms) — all inside the 300 ms window a opens.
    let spawn_submit = |update: &str, pod: &str, delay_ms: u64| {
        let writer = writer.clone();
        let update = update.to_string();
        let pod = PodId::from(pod);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(delay_ms));
            writer.submit(update, [pod])
        })
    };
    let a = spawn_submit("a", "pod:a", 0);
    let f = spawn_submit("fail:boom", "pod:bad", 60);
    let c = spawn_submit("c", "pod:c", 120);

    assert_eq!(a.join().unwrap().unwrap(), 1);
    assert_eq!(
        f.join().unwrap().unwrap_err(),
        WriteError::Rejected("boom".into()),
        "the failed update's submitter gets ITS error"
    );
    assert_eq!(
        c.join().unwrap().unwrap(),
        1,
        "the batch survivor publishes in the same generation"
    );

    let current = ring.current();
    assert_eq!(
        current.number(),
        1,
        "one generation despite the mid-batch failure"
    );
    assert_eq!(
        *current.snapshot(),
        vec!["a".to_string(), "c".into()],
        "no POISON, order kept"
    );
    assert_eq!(current.epochs().epoch(&PodId::from("pod:a")), 1);
    assert_eq!(current.epochs().epoch(&PodId::from("pod:c")), 1);
    assert_eq!(
        current.epochs().epoch(&PodId::from("pod:bad")),
        0,
        "skipped update bumps nothing"
    );
    assert_eq!(
        forks.load(Ordering::SeqCst),
        2,
        "initial fork + one recovery re-fork"
    );
}

/// A batch in which EVERY update fails publishes nothing — the generation
/// number does not move and no epochs are bumped.
#[test]
fn all_failed_batch_publishes_no_generation() {
    let forks = Arc::new(AtomicUsize::new(0));
    let ring = Arc::new(GenerationRing::new(Log::new()));
    let writer = Writer::spawn(
        ring.clone(),
        MockApplier::new(&forks),
        WriterConfig {
            window: Duration::from_millis(1),
            max_batch: 8,
            ..WriterConfig::default()
        },
    );

    let err = writer
        .submit("fail:nope".into(), pods(&["pod:x"]))
        .unwrap_err();
    assert_eq!(err, WriteError::Rejected("nope".into()));
    assert_eq!(
        ring.current().number(),
        0,
        "nothing changed, nothing published"
    );
    assert!(ring.current().epochs().is_empty());

    // The writer is still healthy afterwards.
    assert_eq!(writer.submit("ok".into(), pods(&["pod:x"])).unwrap(), 1);
}

/// The window closes early at max_batch: with max_batch = 2 and a 10 s window,
/// two concurrent sync submits ack long before the window would elapse.
#[test]
fn window_closes_early_on_max_batch() {
    let forks = Arc::new(AtomicUsize::new(0));
    let ring = Arc::new(GenerationRing::new(Log::new()));
    let writer = Arc::new(Writer::spawn(
        ring.clone(),
        MockApplier::new(&forks),
        WriterConfig {
            window: Duration::from_secs(10),
            max_batch: 2,
            ..WriterConfig::default()
        },
    ));

    let started = Instant::now();
    let t = {
        let writer = writer.clone();
        std::thread::spawn(move || writer.submit("a".into(), pods(&["p"])))
    };
    let g2 = writer.submit("b".into(), pods(&["p"])).unwrap();
    let g1 = t.join().unwrap().unwrap();
    let elapsed = started.elapsed();

    assert_eq!(
        (g1, g2),
        (1, 1),
        "both updates in the one max_batch-closed generation"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "max_batch must close the 10 s window early (took {elapsed:?})"
    );
}

/// Window timing is honored approximately: a lone update's sync ack waits out
/// the window (the writer holds the batch open for late arrivals) but not much
/// longer.
#[test]
fn window_timing_approximately_honored() {
    let forks = Arc::new(AtomicUsize::new(0));
    let ring = Arc::new(GenerationRing::new(Log::new()));
    let writer = Writer::spawn(
        ring.clone(),
        MockApplier::new(&forks),
        WriterConfig {
            window: Duration::from_millis(80),
            max_batch: 256,
            ..WriterConfig::default()
        },
    );

    let started = Instant::now();
    assert_eq!(writer.submit("a".into(), pods(&["p"])).unwrap(), 1);
    let elapsed = started.elapsed();

    assert!(
        elapsed >= Duration::from_millis(50),
        "the window stays open for late arrivals (acked after {elapsed:?}, window 80 ms)"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "the ack follows the window without unbounded slack (took {elapsed:?})"
    );
}

/// (sq-p7kk5) ADAPTIVE group-commit: a serial client does NOT wait the window.
/// The counterpart to `window_timing_approximately_honored` — with `adaptive_commit: true`
/// the SAME lone update against a 10 s window acks in engine-time (the writer drains the
/// empty backlog and commits immediately), never holding the batch open for the window.
#[test]
fn adaptive_commit_does_not_wait_the_window_for_a_serial_client() {
    let forks = Arc::new(AtomicUsize::new(0));
    let ring = Arc::new(GenerationRing::new(Log::new()));
    let writer = Writer::spawn(
        ring.clone(),
        MockApplier::new(&forks),
        WriterConfig {
            // A window so long that WAITING it would dominate — the whole point is that
            // adaptive mode never touches it when the backlog is empty.
            window: Duration::from_secs(10),
            max_batch: 256,
            adaptive_commit: true,
            ..WriterConfig::default()
        },
    );

    let started = Instant::now();
    assert_eq!(writer.submit("a".into(), pods(&["p"])).unwrap(), 1);
    let elapsed = started.elapsed();

    // The windowed default would block ~10 s here (see window_timing_approximately_honored);
    // adaptive mode commits in engine-time. A generous ceiling keeps this robust on a noisy CI
    // box while still proving the multi-second window was NOT waited.
    assert!(
        elapsed < Duration::from_secs(1),
        "adaptive commit must not wait the 10 s window for a serial client (acked after {elapsed:?})"
    );
    let current = ring.current();
    assert_eq!(current.number(), 1);
    assert_eq!(*current.snapshot(), vec!["a".to_string()]);
    assert_eq!(current.epochs().epoch(&PodId::from("p")), 1);
    assert_eq!(
        forks.load(Ordering::SeqCst),
        1,
        "one fork for the single-update commit"
    );
}

/// (sq-p7kk5) ADAPTIVE group-commit preserves batching of a QUEUED backlog:
/// updates already sitting in the queue when the writer wakes are all drained into ONE
/// generation (one fork/seal/publish), so group-commit amortisation under concurrency is
/// NOT lost — adaptive mode only skips the timed WAIT, it still batches what has arrived.
#[test]
fn adaptive_commit_still_batches_a_queued_backlog() {
    let forks = Arc::new(AtomicUsize::new(0));
    let ring = Arc::new(GenerationRing::new(Log::new()));
    let writer = Writer::spawn(
        ring.clone(),
        MockApplier::new(&forks),
        WriterConfig {
            window: Duration::from_millis(1),
            max_batch: 256,
            adaptive_commit: true,
            ..WriterConfig::default()
        },
    );

    // Fire-and-forget submits enqueue WITHOUT blocking, so by the time the writer wakes on the
    // first, several are already queued — exactly the "backlog piled up between iterations"
    // concurrency case. A trailing sync submit's ack proves what published together.
    for i in 0..5 {
        writer
            .submit_detached(format!("u{i}"), pods(&[&format!("pod:{i}"), "pod:shared"]))
            .unwrap();
    }
    let generation = writer
        .submit("u5".to_string(), pods(&["pod:5", "pod:shared"]))
        .unwrap();

    let current = ring.current();
    // The whole drained backlog collapses into as few generations as the drain saw it — in
    // practice generation 1 for a backlog present at wake. We assert the STRONG invariant the
    // fix must preserve: all six updates are committed, in strict FIFO order, and the trailing
    // sync's generation is the current one.
    assert_eq!(current.number(), generation);
    let expected: Log = (0..6).map(|i| format!("u{i}")).collect();
    assert_eq!(*current.snapshot(), expected, "strict FIFO, nothing dropped");
    for i in 0..6 {
        assert_eq!(current.epochs().epoch(&PodId::from(format!("pod:{i}"))), 1);
    }
    // The queued backlog was batched, not committed one-fork-per-update: far fewer forks than
    // six. (A pure per-update path would fork six times.)
    assert!(
        forks.load(Ordering::SeqCst) < 6,
        "a queued backlog must batch (forks={}, want < 6)",
        forks.load(Ordering::SeqCst)
    );
}

/// Dropping the writer drains: a pending batch is committed (early, without
/// waiting out a long window) before the thread exits.
#[test]
fn drop_drains_pending_batch() {
    let forks = Arc::new(AtomicUsize::new(0));
    let ring = Arc::new(GenerationRing::new(Log::new()));
    let writer = Writer::spawn(
        ring.clone(),
        MockApplier::new(&forks),
        WriterConfig {
            window: Duration::from_secs(10),
            max_batch: 256,
            ..WriterConfig::default()
        },
    );

    writer.submit_detached("a".into(), pods(&["p"])).unwrap();
    writer.submit_detached("b".into(), pods(&["q"])).unwrap();
    let started = Instant::now();
    drop(writer); // closes the queue; the open window ends early; batch commits
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "drop does not wait out the window"
    );

    let current = ring.current();
    assert_eq!(current.number(), 1);
    assert_eq!(*current.snapshot(), vec!["a".to_string(), "b".into()]);
    assert_eq!(current.epochs().epoch(&PodId::from("q")), 1);
}

/// A panicking applier kills the writer thread; pending and subsequent
/// submitters get `Shutdown`, not a hang.
#[test]
fn applier_panic_reports_shutdown() {
    let forks = Arc::new(AtomicUsize::new(0));
    let ring = Arc::new(GenerationRing::new(Log::new()));
    let writer = Writer::spawn(
        ring.clone(),
        MockApplier::new(&forks),
        WriterConfig {
            window: Duration::from_millis(1),
            max_batch: 8,
            ..WriterConfig::default()
        },
    );

    assert_eq!(
        writer.submit("panic".into(), pods(&["p"])).unwrap_err(),
        WriteError::Shutdown
    );
    // The thread is gone (the receiver dropped with it): later submissions fail
    // fast instead of queueing forever.
    assert_eq!(
        writer.submit("a".into(), pods(&["p"])).unwrap_err(),
        WriteError::Shutdown
    );
    assert_eq!(ring.current().number(), 0);
}

/// READERS ARE NEVER STALLED BY COMMITS (§6.4/§6.5 invariant): while the writer
/// commits continuously — with an artificially slow O(fork) like the production
/// applier's O(graph) rebuild — reader threads hammer `ring.current()` and
/// their p99 latency stays microseconds: no read ever blocks on the write path.
#[test]
fn readers_never_stall_while_writer_commits() {
    let forks = Arc::new(AtomicUsize::new(0));
    let ring = Arc::new(GenerationRing::new(Log::new()));
    let writer = Arc::new(Writer::spawn(
        ring.clone(),
        MockApplier {
            forks: forks.clone(),
            fork_delay: Duration::from_millis(2),
            seal_fails: Arc::new(AtomicUsize::new(0)),
            seals: Arc::new(AtomicUsize::new(0)),
            maintains: Arc::new(std::sync::Mutex::new(Vec::new())),
            maintain_fails: Arc::new(AtomicUsize::new(0)),
            committed: 0,
            restore_durable_supported: false,
            restores: Arc::new(std::sync::Mutex::new(Vec::new())),
            restore_fails: Arc::new(AtomicUsize::new(0)),
        },
        WriterConfig {
            window: Duration::from_millis(1),
            max_batch: 64,
            ..WriterConfig::default()
        },
    ));

    let stop = Arc::new(AtomicBool::new(false));

    // Feeder: keeps the writer committing back-to-back for the whole run.
    let feeder = {
        let writer = writer.clone();
        let stop = stop.clone();
        std::thread::spawn(move || {
            let mut i = 0u64;
            while !stop.load(Ordering::Relaxed) {
                if writer
                    .submit_detached(format!("u{i}"), [PodId::from("pod:w")])
                    .is_err()
                {
                    break;
                }
                i += 1;
                std::thread::sleep(Duration::from_micros(200));
            }
        })
    };

    // Readers: measure every `current()` call for ~1.2 s.
    let readers: Vec<_> = (0..2)
        .map(|_| {
            let ring = ring.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                let mut lat_ns: Vec<u64> = Vec::with_capacity(1 << 20);
                while !stop.load(Ordering::Relaxed) {
                    let t = Instant::now();
                    let g = ring.current();
                    lat_ns.push(t.elapsed().as_nanos() as u64);
                    std::hint::black_box(g.number());
                }
                lat_ns
            })
        })
        .collect();

    std::thread::sleep(Duration::from_millis(1200));
    stop.store(true, Ordering::Relaxed);
    feeder.join().unwrap();

    let committed = ring.current().number();
    assert!(
        committed > 50,
        "writer must have committed continuously (got {committed})"
    );

    for r in readers {
        let mut lat = r.join().unwrap();
        assert!(
            lat.len() > 10_000,
            "reader must have run hot ({} reads)",
            lat.len()
        );
        lat.sort_unstable();
        let p99 = lat[lat.len() * 99 / 100];
        // Lock-free load is ~tens of ns; 1 ms is two orders of magnitude of
        // scheduler-noise headroom. A reader blocked behind a 2 ms fork or the
        // writer's batch application would blow straight past this.
        assert!(
            p99 < 1_000_000,
            "reader p99 = {p99} ns — a read blocked on the write path"
        );
    }
}

/// (sq-vpx4) A TRANSIENT durable-write error (one armed seal failure that
/// then clears) must REFUSE the in-flight write with `WriteError::Unavailable` (a
/// retryable 503-class outcome) WITHOUT killing the writer thread, WITHOUT publishing,
/// and WITHOUT acking 2xx — and a SUBSEQUENT write after recovery must succeed and
/// publish. This is the core graceful-degradation contract.
#[test]
fn transient_seal_failure_refuses_then_recovers() {
    let forks = Arc::new(AtomicUsize::new(0));
    let seal_fails = Arc::new(AtomicUsize::new(0));
    let seals = Arc::new(AtomicUsize::new(0));
    let ring = Arc::new(GenerationRing::new(Log::new()));
    let writer = Writer::spawn(
        ring.clone(),
        MockApplier::with_seal_control(&forks, &seal_fails, &seals),
        // One update per window so each is its own batch / generation attempt.
        WriterConfig {
            window: Duration::from_millis(5),
            max_batch: 1,
            ..WriterConfig::default()
        },
    );

    // Baseline: a normal write publishes generation 1.
    let g1 = writer.submit("ok-1".to_string(), pods(&["pod:a"])).unwrap();
    assert_eq!(g1, 1, "happy-path write must publish");
    assert_eq!(ring.current().number(), 1);

    // Arm ONE transient durable-write failure, then submit. The seal fails once.
    seal_fails.store(1, Ordering::SeqCst);
    let refused = writer.submit("will-be-refused".to_string(), pods(&["pod:a"]));
    assert!(
        matches!(refused, Err(WriteError::Unavailable(_))),
        "transient durable-write error must be a retryable Unavailable refusal, got {refused:?}",
    );

    // FAIL-CLOSED: nothing was published — the ring is byte-identical to before
    // (still generation 1, the refused update's effect absent).
    assert_eq!(
        ring.current().number(),
        1,
        "a refused (non-durable) write must NOT publish a new generation",
    );
    assert_eq!(
        ring.current().snapshot(),
        &vec!["ok-1".to_string()],
        "the refused write must leave the published snapshot byte-identical",
    );

    // The writer thread SURVIVED: a subsequent write after recovery succeeds + publishes.
    let g2 = writer.submit("ok-2".to_string(), pods(&["pod:a"])).unwrap();
    assert_eq!(
        g2, 2,
        "writer must keep running and publish after a transient failure"
    );
    assert_eq!(
        ring.current().snapshot(),
        &vec!["ok-1".to_string(), "ok-2".to_string()]
    );

    // Sanity: the writer actually re-attempted the seal after the refusal.
    assert!(
        seals.load(Ordering::SeqCst) >= 3,
        "writer must have re-attempted seal after refusal"
    );
}

/// (sq-vpx4) Per-submitter failure semantics when `seal` fails on a MIXED
/// batch: one update is rejected during `apply` (a deterministic 400-class error) and one
/// applies cleanly (pending durable commit). The seal then fails. The rejected submitter
/// must keep its ORIGINAL `Rejected` reason (NOT laundered into a retryable 503), while the
/// clean-but-undurable submitter gets `Unavailable`. Fail-closed holds: nothing publishes.
#[test]
fn seal_failure_keeps_apply_rejection_distinct_from_unavailable() {
    let forks = Arc::new(AtomicUsize::new(0));
    let seal_fails = Arc::new(AtomicUsize::new(0));
    let seals = Arc::new(AtomicUsize::new(0));
    let ring = Arc::new(GenerationRing::new(Log::new()));
    // One wide window so both staggered submitters land in the SAME batch.
    let writer = Arc::new(Writer::spawn(
        ring.clone(),
        MockApplier::with_seal_control(&forks, &seal_fails, &seals),
        WriterConfig {
            window: Duration::from_millis(300),
            max_batch: 64,
            ..WriterConfig::default()
        },
    ));

    // Arm ONE seal failure: it fires when the writer seals this batch.
    seal_fails.store(1, Ordering::SeqCst);

    let spawn_submit = |update: &str, pod: &str, delay_ms: u64| {
        let writer = writer.clone();
        let update = update.to_string();
        let pod = PodId::from(pod);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(delay_ms));
            writer.submit(update, [pod])
        })
    };
    // A rejected-during-apply update + a valid one, both inside the open window.
    let rejected = spawn_submit("fail:bad-request", "pod:bad", 0);
    let valid = spawn_submit("ok", "pod:ok", 60);

    // The to-be-rejected update keeps ITS 400-class reason, not a 503 Unavailable.
    assert_eq!(
        rejected.join().unwrap().unwrap_err(),
        WriteError::Rejected("bad-request".into()),
        "an update rejected during apply must keep its original Rejected reason after a seal failure",
    );
    // The valid-but-undurable update is refused with the retryable Unavailable.
    assert!(
        matches!(valid.join().unwrap(), Err(WriteError::Unavailable(_))),
        "a clean update that was pending durable commit must get Unavailable when seal fails",
    );

    // FAIL-CLOSED: nothing published; the writer survived and recovers on the next write.
    assert_eq!(
        ring.current().number(),
        0,
        "a seal-failed batch must publish no generation"
    );
    assert!(ring.current().snapshot().is_empty());
    let g = writer
        .submit("ok-after".to_string(), pods(&["pod:ok"]))
        .unwrap();
    assert_eq!(
        g, 1,
        "writer must keep running and publish once durability recovers"
    );
}

/// (sq-vpx4) A PERSISTENT durable-write error → every write is refused with
/// `Unavailable` (no panic, no publish), the writer stays alive across all of them, and
/// reads keep being served from the last published snapshot the whole time (degraded
/// read-only mode). When durability finally recovers, writes resume.
#[test]
fn persistent_seal_failure_refuses_repeatedly_reads_survive() {
    let forks = Arc::new(AtomicUsize::new(0));
    let seal_fails = Arc::new(AtomicUsize::new(0));
    let seals = Arc::new(AtomicUsize::new(0));
    let ring = Arc::new(GenerationRing::new(Log::new()));
    let writer = Writer::spawn(
        ring.clone(),
        MockApplier::with_seal_control(&forks, &seal_fails, &seals),
        WriterConfig {
            window: Duration::from_millis(5),
            max_batch: 1,
            ..WriterConfig::default()
        },
    );

    // Establish a published snapshot, then jam durability "persistently".
    writer.submit("ok-1".to_string(), pods(&["pod:a"])).unwrap();
    seal_fails.store(5, Ordering::SeqCst);

    for n in 0..5 {
        let r = writer.submit(format!("refused-{n}"), pods(&["pod:a"]));
        assert!(
            matches!(r, Err(WriteError::Unavailable(_))),
            "write #{n} under persistent durability failure must be Unavailable, got {r:?}",
        );
        // Reads still served from the last good snapshot throughout the outage.
        assert_eq!(
            ring.current().number(),
            1,
            "reads must keep serving generation 1 during the outage"
        );
        assert_eq!(ring.current().snapshot(), &vec!["ok-1".to_string()]);
    }

    // Durability recovered (all armed failures consumed): writes resume.
    let g = writer.submit("ok-2".to_string(), pods(&["pod:a"])).unwrap();
    assert_eq!(g, 2, "writes must resume once durability recovers");
}

// ---------------------------------------------------------------------------
// (sq-x32t) Out-of-band MAINTENANCE (admin WAL compaction/vacuum) sequencing.
// ---------------------------------------------------------------------------

/// `Writer::maintain` runs the applier's maintenance op on the writer thread, STRICTLY AFTER the
/// updates submitted before it are committed (FIFO) — so an admin compaction observes exactly the
/// writes that preceded it — and publishes NO generation (the live snapshot is unchanged).
#[test]
fn maintain_runs_after_preceding_updates_no_new_generation() {
    let forks = Arc::new(AtomicUsize::new(0));
    let maintains = Arc::new(std::sync::Mutex::new(Vec::new()));
    let maintain_fails = Arc::new(AtomicUsize::new(0));
    let ring = Arc::new(GenerationRing::new(Log::new()));
    let writer = Writer::spawn(
        ring.clone(),
        MockApplier::with_maintain_control(&forks, &maintains, &maintain_fails),
        // Tiny window so each submit commits as its own generation promptly.
        WriterConfig {
            window: Duration::from_millis(5),
            max_batch: 64,
            ..WriterConfig::default()
        },
    );

    // Commit three updates (FIFO), each its own generation.
    for n in 0..3 {
        writer.submit(format!("u{n}"), pods(&["pod:a"])).unwrap();
    }
    let gen_before = ring.current().number();
    assert_eq!(
        ring.current().snapshot().len(),
        3,
        "three updates committed"
    );

    // Maintenance: blocks, runs on the writer thread, returns Ok.
    assert_eq!(writer.maintain().unwrap(), Ok(()));

    // It observed all three committed updates (ran after them), and published no generation.
    assert_eq!(
        *maintains.lock().unwrap(),
        vec![3],
        "maintain ran once, after the 3 updates"
    );
    assert_eq!(
        ring.current().number(),
        gen_before,
        "maintenance must not publish a new generation"
    );
    assert_eq!(
        ring.current().snapshot().len(),
        3,
        "the live snapshot is unchanged by maintenance"
    );
}

/// A maintenance FAILURE is reported to the caller (inner `Err`) WITHOUT tearing down the writer:
/// later maintenance and updates still work.
#[test]
fn maintain_failure_keeps_writer_alive() {
    let forks = Arc::new(AtomicUsize::new(0));
    let maintains = Arc::new(std::sync::Mutex::new(Vec::new()));
    let maintain_fails = Arc::new(AtomicUsize::new(1)); // fail the first maintain call only
    let ring = Arc::new(GenerationRing::new(Log::new()));
    let writer = Writer::spawn(
        ring.clone(),
        MockApplier::with_maintain_control(&forks, &maintains, &maintain_fails),
        WriterConfig {
            window: Duration::from_millis(5),
            max_batch: 64,
            ..WriterConfig::default()
        },
    );

    writer.submit("u0".to_string(), pods(&["pod:a"])).unwrap();
    // First maintenance fails (inner Err), but the writer thread survives.
    let r = writer.maintain().unwrap();
    assert!(
        r.is_err(),
        "armed maintenance failure must surface as Err, got {r:?}"
    );
    // A subsequent write still commits…
    writer.submit("u1".to_string(), pods(&["pod:a"])).unwrap();
    assert_eq!(
        ring.current().snapshot().len(),
        2,
        "writes work after a maintenance failure"
    );
    // …and a later maintenance now succeeds.
    assert_eq!(writer.maintain().unwrap(), Ok(()));
    assert_eq!(
        maintains.lock().unwrap().len(),
        2,
        "both maintenance attempts ran"
    );
}

// ---------------------------------------------------------------------------
// (sq-ft7u) Writer::restore — the out-of-band durable-restore op routed through the
// single writer thread (the durable-write-through primitive sparq-server's /admin/restore?persist
// and --restore-persist sit on top of). Behavioural coverage against the mock applier:
//   - the IN-MEMORY applier (no durable store) reaches the trait DEFAULT restore_durable -> Err,
//     and the writer survives;
//   - a DURABLE applier's restore is sequenced AFTER the preceding batch (FIFO) and PUBLISHES the
//     restored snapshot as a new generation the ring then serves;
//   - a restore FAILURE is reported to the caller WITHOUT tearing down the writer (the old store
//     is intact and later writes/restores still work).
// ---------------------------------------------------------------------------

/// An IN-MEMORY applier (no durable store) refuses a write-through restore via the trait DEFAULT
/// `restore_durable` (inner `Err`), and the writer thread survives — a subsequent update commits.
#[test]
fn restore_on_in_memory_applier_errors_and_keeps_writer_alive() {
    let forks = Arc::new(AtomicUsize::new(0));
    let ring = Arc::new(GenerationRing::new(Log::new()));
    let writer = Writer::spawn(
        ring.clone(),
        MockApplier::new(&forks), // restore_durable_supported = false → default no-op error
        WriterConfig {
            window: Duration::from_millis(5),
            max_batch: 64,
            ..WriterConfig::default()
        },
    );

    let gen_before = ring.current().number();
    // The restore is refused (no durable store): inner Err, nothing published.
    let r = writer.restore(vec!["restored".to_string()]).unwrap();
    assert!(
        r.is_err(),
        "an in-memory applier must refuse a write-through restore (inner Err), got {r:?}"
    );
    assert_eq!(
        ring.current().number(),
        gen_before,
        "a refused restore publishes no generation"
    );
    // The writer survives — a later update still commits.
    writer.submit("u0".to_string(), pods(&["pod:a"])).unwrap();
    assert_eq!(
        ring.current().snapshot(),
        &vec!["u0".to_string()],
        "writes still work after a refused restore"
    );
}

/// A DURABLE applier's restore runs AFTER the preceding batch (strict FIFO — it observes every
/// committed update) and PUBLISHES the restored snapshot as a NEW generation the ring serves; the
/// returned generation number is that new generation.
#[test]
fn restore_publishes_new_generation_after_preceding_updates() {
    let forks = Arc::new(AtomicUsize::new(0));
    let restores = Arc::new(std::sync::Mutex::new(Vec::new()));
    let restore_fails = Arc::new(AtomicUsize::new(0));
    let ring = Arc::new(GenerationRing::new(Log::new()));
    let writer = Writer::spawn(
        ring.clone(),
        MockApplier::with_restore_control(&forks, &restores, &restore_fails),
        WriterConfig {
            window: Duration::from_millis(5),
            max_batch: 64,
            ..WriterConfig::default()
        },
    );

    // Commit two updates first (the restore must observe them, then replace the whole store).
    writer.submit("u0".to_string(), pods(&["pod:a"])).unwrap();
    writer.submit("u1".to_string(), pods(&["pod:a"])).unwrap();
    assert_eq!(ring.current().snapshot().len(), 2, "two updates committed");
    let gen_before = ring.current().number();

    // Restore: the durable swap succeeds, publishing the restored snapshot as a new generation.
    let restored = vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()];
    let published = writer.restore(restored.clone()).unwrap().expect("restore ok");
    assert!(
        published > gen_before,
        "restore must publish a NEW generation ({published} > {gen_before})"
    );
    assert_eq!(
        ring.current().number(),
        published,
        "the ring now serves the published restore generation"
    );
    // The live snapshot IS the restored image (the whole store was replaced).
    assert_eq!(
        ring.current().snapshot(),
        &restored,
        "the served snapshot is the restored image, not the prior log"
    );
    // It ran exactly once, after both updates committed (FIFO): it saw committed == 2.
    assert_eq!(
        *restores.lock().unwrap(),
        vec![2],
        "restore ran once, after the 2 preceding updates"
    );
}

/// A restore FAILURE is reported to the caller (inner `Err`) WITHOUT tearing down the writer: the
/// OLD live snapshot is untouched, and a subsequent update AND a later successful restore work.
#[test]
fn restore_failure_keeps_writer_alive_and_store_intact() {
    let forks = Arc::new(AtomicUsize::new(0));
    let restores = Arc::new(std::sync::Mutex::new(Vec::new()));
    let restore_fails = Arc::new(AtomicUsize::new(1)); // fail the first restore only
    let ring = Arc::new(GenerationRing::new(Log::new()));
    let writer = Writer::spawn(
        ring.clone(),
        MockApplier::with_restore_control(&forks, &restores, &restore_fails),
        WriterConfig {
            window: Duration::from_millis(5),
            max_batch: 64,
            ..WriterConfig::default()
        },
    );

    writer.submit("keep".to_string(), pods(&["pod:a"])).unwrap();
    let gen_before = ring.current().number();

    // First restore fails (inner Err): nothing published, the OLD store is intact.
    let r = writer.restore(vec!["discarded".to_string()]).unwrap();
    assert!(r.is_err(), "armed restore failure must surface as Err, got {r:?}");
    assert_eq!(
        ring.current().number(),
        gen_before,
        "a failed restore publishes no generation"
    );
    assert_eq!(
        ring.current().snapshot(),
        &vec!["keep".to_string()],
        "the old live snapshot survives a failed restore"
    );

    // The writer survives — a later update commits…
    writer.submit("more".to_string(), pods(&["pod:a"])).unwrap();
    assert_eq!(ring.current().snapshot().len(), 2, "writes work after a failed restore");

    // …and a later restore now succeeds, replacing the store.
    let ok = writer
        .restore(vec!["final".to_string()])
        .unwrap()
        .expect("second restore ok");
    assert_eq!(ring.current().number(), ok, "the successful restore is now served");
    assert_eq!(
        ring.current().snapshot(),
        &vec!["final".to_string()],
        "the second restore replaced the store"
    );
    assert_eq!(
        restores.lock().unwrap().len(),
        2,
        "both restore attempts ran on the writer thread"
    );
}

// ---------------------------------------------------------------------------
// (sq-bdaw5) Commit hook: the per-publish observation seam that lets a
// change-stream recorder ride the writer thread instead of serialising submitters.
// ---------------------------------------------------------------------------

/// The chained-pairs + ack-ordering invariant: the hook fires exactly once per
/// published generation, ON the writer thread, with `(predecessor, published)`
/// pairs that CHAIN gapless — and a submitter's ack happens-after the hook, so
/// by the time `submit` returns its generation's pair is already observed.
/// (Mutation witness: knock out the `hook` call in the writer's `commit` and
/// `pairs` stays empty — every assertion below goes red.)
#[test]
fn commit_hook_fires_once_per_publish_with_chained_pairs_before_ack() {
    let forks = Arc::new(AtomicUsize::new(0));
    let ring = Arc::new(GenerationRing::<Log>::new(Vec::new()));
    let pairs: Arc<std::sync::Mutex<Vec<(u64, u64)>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let pairs_hook = pairs.clone();

    let writer = Writer::spawn_with_commit_hook(
        ring.clone(),
        MockApplier::new(&forks),
        WriterConfig {
            adaptive_commit: true, // serial submits: one generation each, no window wait
            ..WriterConfig::default()
        },
        move |from: &sparq_serve::Generation<Log>, to: &sparq_serve::Generation<Log>| {
            pairs_hook.lock().unwrap().push((from.number(), to.number()));
        },
    );

    for (i, u) in ["a", "b", "c"].iter().enumerate() {
        let n = writer.submit(u.to_string(), pods(&["pod:a"])).unwrap();
        assert_eq!(n, i as u64 + 1);
        // Ack happens-after the hook: the pair for THIS generation is already recorded.
        assert!(
            pairs.lock().unwrap().iter().any(|&(_, to)| to == n),
            "submit({u}) acked before its commit hook ran"
        );
    }
    drop(writer);

    // Exactly one hook fire per published generation, chained gapless from gen 0.
    assert_eq!(*pairs.lock().unwrap(), vec![(0, 1), (1, 2), (2, 3)]);
}

/// A batch whose every update failed publishes nothing — and must not fire the
/// hook (there is no new generation to report; a change-stream record for it
/// would be a non-forward append the log rejects).
#[test]
fn commit_hook_not_fired_when_nothing_publishes() {
    let forks = Arc::new(AtomicUsize::new(0));
    let ring = Arc::new(GenerationRing::<Log>::new(Vec::new()));
    let fires = Arc::new(AtomicUsize::new(0));
    let fires_hook = fires.clone();

    let writer = Writer::spawn_with_commit_hook(
        ring.clone(),
        MockApplier::new(&forks),
        WriterConfig {
            adaptive_commit: true,
            ..WriterConfig::default()
        },
        move |_: &sparq_serve::Generation<Log>, _: &sparq_serve::Generation<Log>| {
            fires_hook.fetch_add(1, Ordering::SeqCst);
        },
    );

    let err = writer
        .submit("fail:nope".to_string(), pods(&["pod:a"]))
        .unwrap_err();
    assert_eq!(err, WriteError::Rejected("nope".to_string()));
    assert_eq!(
        fires.load(Ordering::SeqCst),
        0,
        "an all-failed batch publishes nothing, so the hook must not fire"
    );

    writer.submit("ok".to_string(), pods(&["pod:a"])).unwrap();
    assert_eq!(fires.load(Ordering::SeqCst), 1);
}

/// A RESTORE publish must NOT fire the commit hook (the restored snapshot does
/// not share the predecessor's writer lineage — a lineage-diffing recorder must
/// never be handed the pair). Commits after the restore chain FROM the restored
/// generation, so the skipped publish is visible to the recorder as an explicit
/// discontinuity, never a silently-diffed-through one.
#[test]
fn commit_hook_not_fired_for_restore_publish() {
    let forks = Arc::new(AtomicUsize::new(0));
    let restores = Arc::new(std::sync::Mutex::new(Vec::new()));
    let restore_fails = Arc::new(AtomicUsize::new(0));
    let ring = Arc::new(GenerationRing::<Log>::new(Vec::new()));
    let pairs: Arc<std::sync::Mutex<Vec<(u64, u64)>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let pairs_hook = pairs.clone();

    let writer = Writer::spawn_with_commit_hook(
        ring.clone(),
        MockApplier::with_restore_control(&forks, &restores, &restore_fails),
        WriterConfig {
            adaptive_commit: true,
            ..WriterConfig::default()
        },
        move |from: &sparq_serve::Generation<Log>, to: &sparq_serve::Generation<Log>| {
            pairs_hook.lock().unwrap().push((from.number(), to.number()));
        },
    );

    writer.submit("a".to_string(), pods(&["pod:a"])).unwrap(); // gen 1: hook fires
    let restored = writer
        .restore(vec!["fresh".to_string()])
        .unwrap()
        .expect("restore ok"); // gen 2: published, hook NOT fired
    assert_eq!(restored, 2);
    writer.submit("b".to_string(), pods(&["pod:a"])).unwrap(); // gen 3: hook fires
    drop(writer);

    assert_eq!(
        *pairs.lock().unwrap(),
        vec![(0, 1), (2, 3)],
        "commit publishes fire the hook; the restore publish (gen 2) must not"
    );
}

/// Under `CommitGranularity::CommuteGroup` every published commuting run fires
/// the hook once — the mock's default `footprint` is `Barrier`, so one window's
/// batch degrades to one generation per update, and the pairs still chain.
#[test]
fn commit_hook_fires_per_commute_group() {
    let forks = Arc::new(AtomicUsize::new(0));
    let ring = Arc::new(GenerationRing::<Log>::new(Vec::new()));
    let pairs: Arc<std::sync::Mutex<Vec<(u64, u64)>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let pairs_hook = pairs.clone();

    let writer = Writer::spawn_with_commit_hook(
        ring.clone(),
        MockApplier::new(&forks),
        WriterConfig {
            window: Duration::from_millis(200), // wide window: all submissions share one batch
            granularity: CommitGranularity::CommuteGroup,
            ..WriterConfig::default()
        },
        move |from: &sparq_serve::Generation<Log>, to: &sparq_serve::Generation<Log>| {
            pairs_hook.lock().unwrap().push((from.number(), to.number()));
        },
    );

    // Queue three fire-and-forget updates, then a sync one; barriers split the
    // (single) window into one generation per update.
    writer.submit_detached("a".to_string(), pods(&["pod:a"])).unwrap();
    writer.submit_detached("b".to_string(), pods(&["pod:a"])).unwrap();
    writer.submit_detached("c".to_string(), pods(&["pod:a"])).unwrap();
    let last = writer.submit("d".to_string(), pods(&["pod:a"])).unwrap();
    drop(writer);

    let pairs = pairs.lock().unwrap();
    assert_eq!(
        pairs.len() as u64,
        last,
        "one hook fire per published generation (one per barrier-split group)"
    );
    // The pairs chain gapless from generation 0 regardless of batching splits.
    for (i, &(from, to)) in pairs.iter().enumerate() {
        assert_eq!(from, i as u64, "pair {i} chains from the previous publish");
        assert_eq!(to, i as u64 + 1);
    }
}
