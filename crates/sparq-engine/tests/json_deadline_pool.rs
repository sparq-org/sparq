//! [GPT-6] An expired SELECT must not queue behind unrelated Rayon work.
#![cfg(all(feature = "parallel", not(target_arch = "wasm32")))]

use sparq_core::Graph;
use sparq_engine::{query_json, query_json_stream_with_budget, QueryBudget};
use std::ops::ControlFlow;
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[test]
fn expired_json_does_not_wait_for_busy_rayon_pool() {
    // This integration-test process owns its global pool; unit tests cannot share it.
    rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build_global()
        .unwrap();
    let mut ttl = String::from("@prefix ex: <http://ex/> .\n");
    for i in 0..60_000 {
        ttl.push_str(&format!("ex:s{i} ex:p \"value-{i}\" .\n"));
    }
    let graph = Graph::load_str(&ttl, "turtle").unwrap();
    let query = "SELECT * WHERE { ?s ?p ?o }";
    assert_eq!(
        query_json(&graph, query).unwrap().matches("\"s\":").count(),
        60_000
    );

    let (occupied_tx, occupied_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    rayon::spawn(move || {
        occupied_tx.send(()).unwrap();
        // Sender drop also releases the worker if the test unwinds.
        let _ = release_rx.recv();
    });
    occupied_rx.recv().unwrap();

    let (done_tx, done_rx) = mpsc::channel();
    let request = std::thread::spawn(move || {
        let budget = QueryBudget {
            deadline: Some(Instant::now() - Duration::from_millis(1)),
            ..QueryBudget::unlimited()
        };
        let mut emitted = 0;
        let start = Instant::now();
        let result = query_json_stream_with_budget(&graph, query, &budget, |_| {
            emitted += 1;
            ControlFlow::Continue(())
        });
        let elapsed = start.elapsed();
        let _ = done_tx.send(());
        (result, emitted, elapsed)
    });
    // A deadlock watchdog, not a performance threshold: the request must complete
    // BEFORE the worker is released. No amount of faster serialization can pass
    // if the expired request submits any work to the occupied pool.
    let completed_while_occupied = done_rx.recv_timeout(Duration::from_secs(10)).is_ok();
    drop(release_tx);
    let (result, emitted, elapsed) = request.join().unwrap();
    eprintln!(
        "expired request: {elapsed:?}; completed while pool occupied: {completed_while_occupied}"
    );
    assert!(
        completed_while_occupied,
        "expired request waited for unrelated Rayon work"
    );
    assert_eq!(result.unwrap_err(), "query budget exceeded (timeout)");
    assert_eq!(
        emitted, 0,
        "an expired request must not emit even a JSON header"
    );
}
