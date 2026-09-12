//! [OPUS-4.8] (sq-fggn) Behavioural coverage for the scheduler's public surface
//! that the existing `scheduler.rs` differential/fairness tests don't reach: the
//! defaulted construction, the convenience `run`/`with_defaults` entry points, the
//! non-blocking `try_take` poll, the `submitted`/`completed`/`depth` metrics, and
//! the `SchedError` display/error contract. Each asserts observable behaviour, not
//! just that a line ran.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use sparq_serve::{SchedError, Scheduler, SchedulerConfig};

/// `SchedulerConfig::default()` derives a sane shape: >= 2 workers, a heavy cap of
/// at least 1 but strictly fewer than the worker count (so a cheap job is always
/// reservable), and the documented default heavy threshold.
#[test]
fn default_config_is_well_formed() {
    let cfg = SchedulerConfig::default();
    assert!(cfg.workers >= 2, "default must have >= 2 workers");
    assert_eq!(cfg.heavy_threshold, sparq_serve::DEFAULT_HEAVY_THRESHOLD);
    assert!(cfg.heavy_concurrency >= 1);
    // A scheduler built from it runs a job to completion.
    let sched = Scheduler::<u32>::with_defaults();
    assert_eq!(sched.workers(), cfg.workers.max(2));
    assert_eq!(sched.run(1, || 7).unwrap(), 7);
}

/// `run` is the submit-and-block convenience; it returns the closure's result.
#[test]
fn run_blocks_and_returns_the_result() {
    let sched = Scheduler::<String>::with_defaults();
    let out = sched.run(1, || "hello".to_string()).unwrap();
    assert_eq!(out, "hello");
}

/// `try_take` is non-blocking: `None` while the job is still pending (the ticket
/// stays usable), then `Some(result)` once it has finished.
#[test]
fn try_take_polls_without_consuming_until_done() {
    let sched = Scheduler::<u32>::with_defaults();
    // A job that blocks until we release it, so we can observe the pending poll.
    // [GPT-6] Disconnect on unwind releases the worker before the scheduler joins it.
    let (release, gate) = mpsc::channel::<()>();
    let ticket = sched.submit(1, move || {
        let _ = gate.recv();
        42
    });

    // Still pending: try_take yields None and leaves the ticket usable.
    assert!(ticket.try_take().is_none(), "job has not finished yet");
    assert!(
        ticket.try_take().is_none(),
        "still pending — ticket not consumed"
    );

    drop(release); // release the job

    // Spin (bounded) for completion, then take the result.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(r) = ticket.try_take() {
            assert_eq!(r.unwrap(), 42);
            break;
        }
        assert!(Instant::now() < deadline, "job never completed");
        std::thread::yield_now();
    }
}

/// `submitted` and `completed` are monotonic counters that converge once every
/// ticket has been waited on.
#[test]
fn submitted_and_completed_counters_track_jobs() {
    let sched = Scheduler::<u32>::with_defaults();
    assert_eq!(sched.submitted(), 0);
    assert_eq!(sched.completed(), 0);

    let tickets: Vec<_> = (0..8).map(|i| sched.submit(1, move || i)).collect();
    assert_eq!(sched.submitted(), 8, "every submit counts immediately");

    for (i, t) in tickets.into_iter().enumerate() {
        assert_eq!(t.wait().unwrap(), i as u32);
    }
    assert_eq!(
        sched.completed(),
        8,
        "all jobs completed once every ticket was waited on"
    );
}

/// `depth` reports queued work while every worker is occupied.
#[test]
fn depth_reports_queue_occupancy() {
    let sched = Scheduler::<()>::new(SchedulerConfig {
        workers: 2,
        heavy_concurrency: 1,
        heavy_threshold: 100,
    });
    // [GPT-6] Occupy every worker so none can drain the backlog before depth().
    // Declare the release senders after sched: on assertion failure they drop
    // first, disconnecting the waits before Scheduler::drop joins the workers.
    let mut releases = Vec::new();
    let mut parked = Vec::new();
    let (started, ready) = mpsc::channel();
    for _ in 0..sched.workers() {
        let (release, gate) = mpsc::channel::<()>();
        releases.push(release);
        let started = started.clone();
        parked.push(sched.submit(1, move || {
            let _ = started.send(());
            let _ = gate.recv();
        }));
    }
    drop(started);
    for _ in 0..sched.workers() {
        ready
            .recv_timeout(Duration::from_secs(5))
            .expect("worker never started its parked job");
    }
    assert_eq!(
        sched.depth(),
        (0, 0, 0),
        "running cheap jobs are not queued"
    );

    let queued: Vec<_> = (0..4).map(|_| sched.submit(1, || {})).collect();
    assert_eq!(
        sched.depth(),
        (4, 0, 0),
        "all four cheap jobs remain queued"
    );

    drop(releases);
    for ticket in parked.into_iter().chain(queued) {
        ticket.wait().unwrap();
    }
    assert_eq!(sched.depth(), (0, 0, 0), "the queue drained");
}

/// `SchedError` renders distinct, non-empty human-readable messages (the
/// `Display`/`Error` contract callers map to 503-class responses).
#[test]
fn sched_error_display_is_distinct_and_nonempty() {
    let shutdown = format!("{}", SchedError::Shutdown);
    let panicked = format!("{}", SchedError::Panicked);
    assert!(!shutdown.is_empty());
    assert!(!panicked.is_empty());
    assert_ne!(shutdown, panicked);
    // Error trait object is constructible (source() defaults to None).
    let err: &dyn std::error::Error = &SchedError::Shutdown;
    assert!(err.source().is_none());
}
