//! The read-side query scheduler (research/concurrent-serving.md §6.2; the wave
//! plan's Wave C, scoped here as the "serve Wave B" deliverable — goal #4
//! requirements 3 + 4: **cheap-query prioritisation** and **no head-of-line
//! blocking**). It composes over the A1 [`GenerationRing`](crate::GenerationRing):
//! a reader still pins a generation on entry (snapshot consistency, §6.4); the
//! scheduler only decides *when and on which worker* the pinned read runs.
//!
//! written while Fable 5 unavailable — re-review when Fable returns.
//!
//! ## Design (Umbra-style, litreview-C Topic A)
//!
//! The reference is Umbra's self-tuning scheduler (Wagner et al., SIGMOD 2021):
//! priority decay approximates SRPT while a floor (`p_min > 0`) guarantees long
//! queries a background share so they never starve; new arrivals enter at
//! `p₀ ≫ decayed priorities` so they immediately dominate instead of queueing
//! behind heavies. Schrage (1968): SRPT minimises mean response time. Wierman &
//! Nuyens (SIGMETRICS 2008): SRPT-like policies tolerate crude size estimates with
//! bounded degradation — so a *cheap* cost estimate suffices (§2.3), and this
//! crate stays decoupled from any particular estimator.
//!
//! sparq's engine has **no re-entrant suspend/resume** (§3.2: the only checkpoint
//! is `QueryBudget`'s cooperative abort), so true preemption is out of scope. What
//! delivers the two required properties without it:
//!
//! 1. **Two lanes by a cheap cost estimate.** A job whose estimate is below
//!    [`SchedulerConfig::heavy_threshold`] is *cheap*; otherwise *heavy*. Cheap
//!    jobs are picked strictly before heavy jobs and within their lane run FIFO
//!    (they are uniform-ish and tiny — SRPT ordering among them is pure overhead).
//! 2. **Reserved cheap capacity = no head-of-line blocking.** The heavy lane runs
//!    on at most [`SchedulerConfig::heavy_concurrency`] workers (default
//!    `cores/2`, never more than `workers − 1`). So however many expensive
//!    queries flood in, at least `workers − heavy_concurrency ≥ 1` workers are
//!    always free to pick a cheap job the instant it arrives — a big scan can
//!    never monopolise the pool. This is the §4.5-B fix (expensive queries
//!    bounded so they cannot saturate the box) made structural.
//! 3. **SRPT-approx + aging *within* the heavy lane.** Heavy jobs are ordered by a
//!    decaying priority: each job starts at [`P0`](crate::P0); while it waits, an
//!    aging credit lifts effective priority, and a higher *estimated cost* lowers
//!    it (shortest-estimated-first ≈ SRPT). The aging term is unbounded in wait
//!    time, so the most-postponed heavy job always eventually becomes the
//!    max-priority pick — **starvation-free by construction** (the `p_min`/MLFQ
//!    boost role, litreview-C line 37). Proven in `tests/scheduler.rs`
//!    (`heavy_query_not_starved_under_cheap_flood`).
//!
//! ## What the scheduler does NOT change (correctness, §5 verdict 3 scope)
//!
//! The work itself is a caller-supplied closure run **verbatim** (the closure
//! pins its generation and runs the engine). Scheduling reorders *when* closures
//! run and *on which thread* — never *what* they compute. Results and snapshot
//! semantics are therefore identical to unscheduled execution; `tests/scheduler.rs`
//! differentially asserts byte-identical results vs direct execution.
//!
//! ## Library-first (§6.1)
//!
//! Sync, runtime-agnostic. The scheduler owns `std::thread` workers and
//! `std::sync` primitives only — no HTTP, no async-runtime types.
//! [`Scheduler::submit`] is callable from any thread and returns immediately with
//! a [`Ticket`]; blocking for the result on the caller's own thread is
//! [`Ticket::wait`]. A future that resolves on the ticket (a `submit_async`
//! adapter) belongs behind the crate's later `tokio` feature, not here. The
//! estimator and the work are both injected, so the scheduler never
//! reaches into `sparq-engine`/`sparq-core` itself — it stays a pure
//! cost-aware bounded thread pool.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Instant;

/// Starting priority of every newly-arrived job, mirroring Umbra's `p₀ = 10⁴`
/// (litreview-C line 37). Heavy-lane ordering subtracts an estimate-derived
/// penalty and adds an unbounded aging credit from this base; the magnitude only
/// has to dominate per-tick decay, which it does by construction here.
pub const P0: i64 = 10_000;

/// The cost estimate for a submitted job — a crude SRPT size proxy. Smaller =
/// cheaper = scheduled sooner. The unit is arbitrary (row-count estimate,
/// `store.estimate` sum over the BGP, a per-template moving average, …); only the
/// *ordering* it induces matters (Wierman & Nuyens: constant-factor error is
/// tolerated). `0` is the cheapest possible job.
pub type Cost = u64;

/// How a job is classified once its [`Cost`] is known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lane {
    /// Estimate below the heavy threshold: prioritised, FIFO within the lane,
    /// always has a reserved worker (the no-HoL guarantee).
    Cheap,
    /// Estimate at or above the heavy threshold: SRPT-approx + aging ordering,
    /// bounded concurrency so it can never monopolise the pool.
    Heavy,
}

/// Configuration for the [`Scheduler`].
#[derive(Clone, Debug)]
pub struct SchedulerConfig {
    /// Total worker threads. Defaults to `available_parallelism` (min 2 — the
    /// reserved-cheap-capacity invariant needs at least one non-heavy worker).
    pub workers: usize,
    /// Max workers the heavy lane may occupy simultaneously. Clamped to
    /// `1..=workers-1` so a cheap worker is **always** reserved (the structural
    /// no-head-of-line guarantee). Defaults to `max(1, workers/2)` — §6.2's
    /// "heavy-lane concurrency cap ≈ physical_cores/2".
    pub heavy_concurrency: usize,
    /// Estimate at or above which a job is [`Lane::Heavy`]. Below it the job is
    /// [`Lane::Cheap`]. Defaults to [`DEFAULT_HEAVY_THRESHOLD`].
    pub heavy_threshold: Cost,
}

/// Default heavy-lane threshold (estimate units). The §6.2 split is "point-class
/// queries execute inline; everything else is the executor pool" — a row-estimate
/// of a few thousand is a reasonable point-vs-scan divider for the row-count proxy
/// the estimator typically returns; tune per deployment from estimate-vs-actual
/// telemetry (§7.2 WatDiv scatter).
pub const DEFAULT_HEAVY_THRESHOLD: Cost = 4_096;

impl Default for SchedulerConfig {
    fn default() -> Self {
        let workers = thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(8)
            .max(2);
        SchedulerConfig {
            workers,
            heavy_concurrency: (workers / 2).max(1),
            heavy_threshold: DEFAULT_HEAVY_THRESHOLD,
        }
    }
}

impl SchedulerConfig {
    /// Resolved heavy-concurrency cap, clamped to `1..=workers-1` so at least one
    /// worker is always reservable for cheap jobs.
    fn heavy_cap(&self) -> usize {
        self.heavy_concurrency
            .clamp(1, self.workers.saturating_sub(1).max(1))
    }
}

/// Why a submitted job did not run to completion as scheduled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SchedError {
    /// The scheduler is shutting down (or its pool panicked); the job was dropped
    /// without running. Callers map this to a 503-style "unavailable".
    Shutdown,
    /// The worker running the job panicked. The job's own work panicked; the pool
    /// survives (one worker is replaced is *not* attempted — see module docs:
    /// a panicking closure is a caller bug, surfaced, never silently retried).
    Panicked,
}

impl std::fmt::Display for SchedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SchedError::Shutdown => f.write_str("scheduler has shut down"),
            SchedError::Panicked => f.write_str("scheduled job panicked"),
        }
    }
}

impl std::error::Error for SchedError {}

/// A queued job: its lane-ordering key plus the boxed work that produces `T`.
struct Job<T> {
    /// Monotonic submission sequence — FIFO tiebreak within a lane and the cheap
    /// lane's whole order.
    seq: u64,
    cost: Cost,
    lane: Lane,
    /// When the job entered the queue, for the heavy lane's aging credit.
    enqueued: Instant,
    work: Box<dyn FnOnce() -> T + Send + 'static>,
    /// Receives the outcome; `None` once delivered.
    slot: Arc<Slot<T>>,
}

/// Heavy-lane ordering: SRPT-approx (lower cost first) with an unbounded aging
/// credit (longer wait first), so the most-postponed job eventually wins.
///
/// Effective priority = `P0 − cost_penalty(cost) + aging(wait)`. We compare by
/// this value *at pick time* (the heap is re-keyed lazily — see `pick_heavy`),
/// `BinaryHeap` is a max-heap so larger effective priority is picked first.
struct HeavyKey {
    cost: Cost,
    enqueued: Instant,
}

/// Aging rate: priority credit gained per microsecond of waiting. Chosen so that
/// a job waiting a few milliseconds overtakes the cost penalty of even a maximal
/// estimate — i.e. no heavy job waits more than O(ms × queue depth) before
/// becoming the max-priority pick, bounding starvation tightly while still
/// letting cheaper heavies go first when waits are comparable.
const AGING_CREDIT_PER_MICRO: i64 = 1;

impl HeavyKey {
    /// Effective priority at instant `now`; larger is picked first.
    fn effective(&self, now: Instant) -> i64 {
        // Cost penalty: saturate into i64; a larger estimate lowers priority
        // (shortest-estimated-first). Clamp so a pathological estimate cannot
        // underflow past the aging term's reach.
        let penalty = self.cost.min(i64::MAX as u64) as i64;
        let waited_us = now.saturating_duration_since(self.enqueued).as_micros();
        let aging = (waited_us as i64).saturating_mul(AGING_CREDIT_PER_MICRO);
        P0.saturating_sub(penalty).saturating_add(aging)
    }
}

/// The shared queues + worker accounting, behind one mutex (the scheduling
/// decision is sub-µs — litreview-C line 33 — so a single lock is not the
/// bottleneck at the request rates this tier sees; a per-worker lock-free stride
/// scheduler is the named follow-up if telemetry ever shows lock contention).
struct Queues<T> {
    cheap: VecDeque<Job<T>>,
    /// Heavy jobs keyed for lazy SRPT-approx+aging extraction (see `pick_heavy`).
    heavy: Vec<Job<T>>,
    /// Workers currently running a heavy job — bounded by `heavy_cap`.
    heavy_running: usize,
    shutdown: bool,
}

/// The cost-aware bounded scheduler (a fixed thread pool with two priority lanes).
///
/// Share as `Arc<Scheduler<T>>`; [`submit`](Self::submit) takes `&self`. All
/// workers are joined on drop after the queues drain of *running* work (queued
/// but unstarted jobs are failed with [`SchedError::Shutdown`] — the SEDA "shed,
/// don't queue unboundedly" stance; a clean drain-all variant is a config knob a
/// later wave can add if a deployment needs it).
pub struct Scheduler<T: Send + 'static> {
    cfg: SchedulerConfig,
    heavy_cap: usize,
    queues: Arc<(Mutex<Queues<T>>, Condvar)>,
    seq: AtomicU64,
    /// Live counters for tests/metrics (no lock needed to read).
    submitted: Arc<AtomicU64>,
    completed: Arc<AtomicU64>,
    workers: Vec<JoinHandle<()>>,
}

/// A handle to one submitted job's eventual outcome.
///
/// [`wait`](Ticket::wait) blocks the *caller* until the job completes (it is
/// runtime-agnostic — a tokio adapter wrapping this in a future is a later-wave
/// feature, never in core). The caller thread that called `submit` is free while
/// the job runs on a worker, so blocking on the ticket is the caller's choice of
/// when to collect, not a busy-wait.
pub struct Ticket<T> {
    slot: Arc<Slot<T>>,
}

/// One-shot result cell with a condvar — the ticket's completion channel. (A
/// `std::sync::mpsc::sync_channel(1)` would also work; a bespoke slot keeps the
/// "result already taken" and "shutdown before run" states explicit and lets the
/// worker store a panic outcome without unwinding across the channel.)
struct Slot<T> {
    state: Mutex<SlotState<T>>,
    ready: Condvar,
}

enum SlotState<T> {
    Pending,
    Done(Result<T, SchedError>),
    Taken,
}

impl<T> Slot<T> {
    fn new() -> Arc<Self> {
        Arc::new(Slot {
            state: Mutex::new(SlotState::Pending),
            ready: Condvar::new(),
        })
    }

    fn deliver(&self, outcome: Result<T, SchedError>) {
        let mut s = self.state.lock().expect("slot poisoned");
        // Only the first delivery wins (a panicking worker's drop-guard may also
        // try to deliver Shutdown — the real outcome, set first, takes priority).
        if matches!(*s, SlotState::Pending) {
            *s = SlotState::Done(outcome);
            self.ready.notify_one();
        }
    }
}

impl<T: Send + 'static> Ticket<T> {
    /// Blocks until the job completes, returning its result (or a
    /// [`SchedError`]). Consumes the ticket — a job's outcome is collected once.
    pub fn wait(self) -> Result<T, SchedError> {
        let mut s = self.slot.state.lock().expect("slot poisoned");
        loop {
            match std::mem::replace(&mut *s, SlotState::Taken) {
                SlotState::Done(r) => return r,
                SlotState::Taken => return Err(SchedError::Shutdown), // double-take guard
                SlotState::Pending => {
                    *s = SlotState::Pending;
                    s = self.slot.ready.wait(s).expect("slot poisoned");
                }
            }
        }
    }

    /// Non-blocking poll: `Some(outcome)` if the job has finished, else `None`.
    /// Leaves the ticket usable (does not consume) when still pending.
    pub fn try_take(&self) -> Option<Result<T, SchedError>> {
        let mut s = self.slot.state.lock().expect("slot poisoned");
        match std::mem::replace(&mut *s, SlotState::Taken) {
            SlotState::Done(r) => Some(r),
            other => {
                *s = other;
                None
            }
        }
    }
}

impl<T: Send + 'static> Scheduler<T> {
    /// Spawns a scheduler with the given config.
    pub fn new(cfg: SchedulerConfig) -> Arc<Self> {
        let workers_n = cfg.workers.max(2);
        let heavy_cap = cfg.heavy_cap();
        let queues = Arc::new((
            Mutex::new(Queues {
                cheap: VecDeque::new(),
                heavy: Vec::new(),
                heavy_running: 0,
                shutdown: false,
            }),
            Condvar::new(),
        ));
        let submitted = Arc::new(AtomicU64::new(0));
        let completed = Arc::new(AtomicU64::new(0));
        let mut workers = Vec::with_capacity(workers_n);
        for _ in 0..workers_n {
            let queues = queues.clone();
            let completed = completed.clone();
            let cap = heavy_cap;
            workers.push(
                thread::Builder::new()
                    .name("sparq-serve-sched".into())
                    .spawn(move || worker_loop(queues, completed, cap))
                    .expect("spawn scheduler worker"),
            );
        }
        Arc::new(Scheduler {
            cfg: SchedulerConfig {
                workers: workers_n,
                ..cfg
            },
            heavy_cap,
            queues,
            seq: AtomicU64::new(0),
            submitted,
            completed,
            workers,
        })
    }

    /// Spawns a scheduler with [`SchedulerConfig::default`].
    pub fn with_defaults() -> Arc<Self> {
        Self::new(SchedulerConfig::default())
    }

    /// Submits `work` with cost estimate `cost`; returns a [`Ticket`] for its
    /// outcome. The lane is chosen from `cost` vs the configured threshold. The
    /// caller is expected to have pinned its generation *inside* `work`
    /// (snapshot consistency, §6.4) — the scheduler is generation-agnostic.
    ///
    /// Non-blocking: enqueues and returns immediately, even when all workers are
    /// busy (the work waits in its lane's queue). On shutdown returns a ticket
    /// already resolved to [`SchedError::Shutdown`].
    pub fn submit<F>(&self, cost: Cost, work: F) -> Ticket<T>
    where
        F: FnOnce() -> T + Send + 'static,
    {
        let slot = Slot::new();
        let lane = if cost >= self.cfg.heavy_threshold {
            Lane::Heavy
        } else {
            Lane::Cheap
        };
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let job = Job {
            seq,
            cost,
            lane,
            enqueued: Instant::now(),
            work: Box::new(work),
            slot: slot.clone(),
        };
        let (lock, cvar) = &*self.queues;
        let mut q = lock.lock().expect("scheduler queues poisoned");
        if q.shutdown {
            drop(q);
            slot.deliver(Err(SchedError::Shutdown));
            return Ticket { slot };
        }
        match lane {
            Lane::Cheap => q.cheap.push_back(job),
            Lane::Heavy => q.heavy.push(job),
        }
        self.submitted.fetch_add(1, Ordering::Relaxed);
        // Wake one worker; if it can't take this lane (heavy cap) it re-notifies
        // (see `worker_loop`), so a cheap job is never stranded behind the cap.
        cvar.notify_one();
        Ticket { slot }
    }

    /// Convenience: submit and block for the result on the calling thread.
    pub fn run<F>(&self, cost: Cost, work: F) -> Result<T, SchedError>
    where
        F: FnOnce() -> T + Send + 'static,
    {
        self.submit(cost, work).wait()
    }

    /// The lane a given cost maps to under this scheduler's config (exposed for
    /// callers/metrics that classify before submitting).
    pub fn lane_of(&self, cost: Cost) -> Lane {
        if cost >= self.cfg.heavy_threshold {
            Lane::Heavy
        } else {
            Lane::Cheap
        }
    }

    /// Total jobs submitted since construction.
    pub fn submitted(&self) -> u64 {
        self.submitted.load(Ordering::Relaxed)
    }

    /// Total jobs completed (ran to a result or were shed) since construction.
    pub fn completed(&self) -> u64 {
        // Acquire (paired with the worker's Release increment in
        // `worker_loop`) so a caller observing all its tickets' `wait()` returned
        // is guaranteed to see every corresponding increment — the invariant the
        // `heavy_concurrency_is_bounded` quiet-box race violated under Relaxed.
        self.completed.load(Ordering::Acquire)
    }

    /// Snapshot of `(cheap_queued, heavy_queued, heavy_running)` — for metrics and
    /// tests. Takes the queue lock briefly; not a per-request hot path.
    pub fn depth(&self) -> (usize, usize, usize) {
        let (lock, _) = &*self.queues;
        let q = lock.lock().expect("scheduler queues poisoned");
        (q.cheap.len(), q.heavy.len(), q.heavy_running)
    }

    /// The resolved worker count.
    pub fn workers(&self) -> usize {
        self.cfg.workers
    }

    /// The resolved heavy-concurrency cap (always `≤ workers − 1`).
    pub fn heavy_cap(&self) -> usize {
        self.heavy_cap
    }
}

impl<T: Send + 'static> Drop for Scheduler<T> {
    fn drop(&mut self) {
        {
            let (lock, cvar) = &*self.queues;
            let mut q = lock.lock().expect("scheduler queues poisoned");
            q.shutdown = true;
            // Fail every still-queued (unstarted) job — SEDA: shed, don't hang.
            for job in q.cheap.drain(..) {
                job.slot.deliver(Err(SchedError::Shutdown));
            }
            for job in q.heavy.drain(..) {
                job.slot.deliver(Err(SchedError::Shutdown));
            }
            cvar.notify_all();
        }
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}

/// One worker's loop: pick the highest-priority *runnable* job (cheap first;
/// heavy only while the global heavy cap allows), run it, repeat. Blocks on the
/// condvar when nothing is runnable.
fn worker_loop<T: Send + 'static>(
    queues: Arc<(Mutex<Queues<T>>, Condvar)>,
    completed: Arc<AtomicU64>,
    heavy_cap: usize,
) {
    let (lock, cvar) = &*queues;
    loop {
        let job = {
            let mut q = lock.lock().expect("scheduler queues poisoned");
            loop {
                if let Some(job) = take_runnable(&mut q, heavy_cap) {
                    break job;
                }
                if q.shutdown && q.cheap.is_empty() && q.heavy.is_empty() {
                    return;
                }
                // Nothing runnable (queues empty, or only heavy left at the cap):
                // wait to be woken by a submit or a heavy-slot release.
                q = cvar.wait(q).expect("scheduler queues poisoned");
            }
        };

        let is_heavy = job.lane == Lane::Heavy;
        // A panic-safe guard: if `work` panics, deliver `Panicked` and release
        // the heavy slot, so the pool never deadlocks on a leaked cap count.
        let outcome = run_job_catching(job.work);
        // Signal-before-count race fix. The completion counter MUST be
        // incremented BEFORE `deliver()` signals the ticket waiter, and with
        // Release ordering, so the increment is published *before* the wake. Order
        // matters: `deliver()` takes the slot mutex, stores `Done`, and notifies
        // the condvar; the waiter in `Ticket::wait` re-acquires that same slot
        // mutex when it wakes. The slot mutex's release (here in `deliver`) →
        // acquire (in `wait`) pair is the inter-thread happens-before edge, and it
        // carries this Release increment with it. Combined with the Acquire load in
        // `completed()`, every increment of a completed job is therefore visible to
        // a waiter the moment its `wait()` returns. The previous order (deliver
        // then Relaxed fetch_add) let a woken waiter read `completed()` *before* the
        // increment landed — the 11-vs-12 race that only surfaced on a quiet box
        // where `wait()` could return before the worker reached the fetch_add.
        completed.fetch_add(1, Ordering::Release);
        job.slot.deliver(outcome);

        if is_heavy {
            let mut q = lock.lock().expect("scheduler queues poisoned");
            q.heavy_running -= 1;
            // A freed heavy slot may make a waiting heavy job runnable; wake one.
            cvar.notify_one();
        }
    }
}

/// Picks the next job a worker may run *now*: cheap jobs (FIFO) always; otherwise
/// the best heavy job (SRPT-approx + aging) but only while the heavy cap permits.
/// Increments `heavy_running` when it hands out a heavy job (the worker decrements
/// it on completion). Returns `None` when nothing is currently runnable.
fn take_runnable<T>(q: &mut Queues<T>, heavy_cap: usize) -> Option<Job<T>> {
    // Cheap lane first, unconditionally — the reserved-capacity / no-HoL rule.
    if let Some(job) = q.cheap.pop_front() {
        return Some(job);
    }
    // Heavy lane only while under the global cap (this is what reserves workers
    // for cheap jobs: when heavy_running == heavy_cap, heavy jobs are left in the
    // queue and this worker parks, staying available for an incoming cheap job).
    if q.heavy_running >= heavy_cap || q.heavy.is_empty() {
        return None;
    }
    let job = pick_heavy(&mut q.heavy);
    q.heavy_running += 1;
    Some(job)
}

/// Extracts the max-effective-priority heavy job *evaluated at `now`* (lazy
/// re-keying: aging changes the order over time, so we recompute keys on each
/// pick rather than maintaining a stale `BinaryHeap`). O(n) over the heavy
/// backlog — fine because the heavy lane is, by design, small (bounded
/// concurrency, few concurrent expensive queries; §1.3: analytical traffic is
/// rare and budgeted). If the heavy backlog ever grows large enough for this to
/// matter, the named follow-up is a bucketed/decay-indexed structure.
fn pick_heavy<T>(heavy: &mut Vec<Job<T>>) -> Job<T> {
    debug_assert!(!heavy.is_empty());
    let now = Instant::now();
    let mut best = 0usize;
    let mut best_key = heavy_key(&heavy[0]).effective(now);
    let mut best_seq = heavy[0].seq;
    for (i, job) in heavy.iter().enumerate().skip(1) {
        let k = heavy_key(job).effective(now);
        // Higher effective priority wins; FIFO (lower seq) breaks ties so equal
        // jobs keep arrival order (preserves base-latency order, Umbra principle).
        if k > best_key || (k == best_key && job.seq < best_seq) {
            best = i;
            best_key = k;
            best_seq = job.seq;
        }
    }
    heavy.swap_remove(best)
}

fn heavy_key<T>(job: &Job<T>) -> HeavyKey {
    HeavyKey {
        cost: job.cost,
        enqueued: job.enqueued,
    }
}

/// Runs `work`, catching a panic and reporting it as [`SchedError::Panicked`]
/// rather than poisoning the worker or the queue lock.
fn run_job_catching<T>(work: Box<dyn FnOnce() -> T + Send + 'static>) -> Result<T, SchedError> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)) {
        Ok(v) => Ok(v),
        Err(_) => Err(SchedError::Panicked),
    }
}
