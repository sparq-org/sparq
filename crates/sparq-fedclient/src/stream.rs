//! The `SolutionStream` boundary (design §4.4, §7) — **Phase 5**.
//!
//! `SolutionStream` is the async/iterator abstraction the client owns **at its boundary**.
//! It carries federated solutions ([`Solution`]) as they become derivable — adapters
//! produce it, the [`crate::operators`] streaming operators consume and produce it — so a
//! result can be handed to the caller **before** every input source has been exhausted.
//!
//! # The honest limit (design §7)
//!
//! `sparq-engine` stays **materialised**: it returns a fully-materialised `QueryResult` and
//! evaluates operators blocking. The client does NOT re-architect the engine into a global
//! async stream; it streams at *its own* boundary (adapter outputs + federation operators),
//! and a *local* sub-BGP evaluation through the engine remains a single materialised call.
//! End-to-end solution-level laziness *through* the engine is out of scope.
//!
//! # Why a channel and not an async runtime (design §7, the ASYNC/RUNTIME decision)
//!
//! The remote transport ([`crate::source::Transport::fetch`]) is **blocking**. Phase 5 does
//! NOT pull an async runtime (tokio / async-std) into the dependency tree — that would push
//! a heavy, viral dependency onto an opt-in crate and violate the lean-core constraint. All
//! concurrency stays *inside* this crate, built on `std` only: a producer side runs on a
//! **bounded blocking thread-pool** ([`crate::operators::ScatterPool`]) over the blocking
//! transport, and hands solutions across a **bounded** [`std::sync::mpsc::sync_channel`].
//! The channel's bound IS the backpressure: when the consumer is slower than the producers,
//! a `send` blocks the producer thread until the consumer drains, so a fast source cannot
//! grow an unbounded in-flight buffer. No GC heap, no runtime — just Rust ownership + the
//! channel bound + the reused [`StreamJoin`](sparq_fedplan::StreamJoin) spill.
//!
//! # Backpressure + bounded memory
//!
//! `SolutionStream` is a blocking `Iterator<Item = Result<Solution, FedError>>`. Pulling an
//! item dequeues one solution (or blocks until one is ready / the producers finish). Because
//! the channel is bounded, the resident in-flight set is capped at the channel capacity plus
//! whatever the streaming join operator holds (itself bounded by the `StreamJoin` spill).
//!
//! sq-vtba (epic sq-dnko): Phase-5 streaming boundary. Flagged for Fable
//! re-review when available.

use crate::source::FedError;
use oxrdf::Term;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};

/// One federated solution: a column header (`vars`, variable names without the leading
/// `?`) and one cell per column, `Some(term)` bound or `None` unbound. The same shape as
/// the Phase-3 [`Relation`](crate::operators::Relation) row, lifted to a single streamed
/// item so it can travel a channel. sq-vtba.
#[derive(Debug, Clone, PartialEq)]
pub struct Solution {
    /// Variable names (without `?`) in cell order. Every [`Solution`] from one stream
    /// shares the same `vars` order (the stream's output schema).
    pub vars: Vec<String>,
    /// One cell per `vars` column; `None` is unbound.
    pub cells: Vec<Option<Term>>,
}

impl Solution {
    /// A solution with the given header + cells.
    pub fn new(vars: Vec<String>, cells: Vec<Option<Term>>) -> Solution {
        Solution { vars, cells }
    }

    /// The bound term for `var`, if this solution binds it.
    pub fn get(&self, var: &str) -> Option<&Term> {
        self.vars
            .iter()
            .position(|v| v == var)
            .and_then(|i| self.cells.get(i).and_then(|c| c.as_ref()))
    }
}

/// The item a [`SolutionStream`] yields: a solution, or a producer-side error (a transport
/// refusal, an SSRF block, a malformed remote body). An error item is **terminal-ish**: the
/// stream surfaces it to the consumer; the consumer decides whether to keep pulling (a later
/// source may still produce). sq-vtba.
pub type StreamItem = Result<Solution, FedError>;

/// The **producer half** of a [`SolutionStream`]: a bounded sender the operator threads push
/// solutions into. Cloneable, so a fan-out operator can hand one [`SolutionSink`] to each of
/// several producer threads (the channel multiplexes them). When the last sink is dropped
/// the stream ends (the receiver sees the channel close). sq-vtba.
#[derive(Clone)]
pub struct SolutionSink {
    tx: SyncSender<StreamItem>,
}

impl SolutionSink {
    /// Push one solution. Returns `false` if the consumer has gone away (the receiver was
    /// dropped) — the producer should then stop early (no point computing more). **Blocks**
    /// when the bounded channel is full: that block IS the backpressure that bounds memory.
    pub fn emit(&self, sol: Solution) -> bool {
        self.tx.send(Ok(sol)).is_ok()
    }

    /// Push a producer-side error. Same backpressure + closed-consumer semantics as
    /// [`emit`](Self::emit).
    pub fn emit_err(&self, err: FedError) -> bool {
        self.tx.send(Err(err)).is_ok()
    }
}

/// A bounded, backpressured stream of federated [`Solution`]s — the boundary abstraction the
/// client owns (design §4.4). Iterate it to pull solutions as they arrive; the producers run
/// on the [`ScatterPool`](crate::operators::ScatterPool) and feed a bounded channel, so a
/// solution is delivered **before** the inputs are exhausted and resident memory stays
/// bounded by the channel capacity (+ the streaming-join spill).
///
/// Construct a connected (sink, stream) pair with [`SolutionStream::bounded`]; or wrap an
/// already-materialised set of rows with [`SolutionStream::from_rows`] (the adapter for a
/// blocking source — e.g. a local engine eval — that has nothing to stream incrementally).
/// sq-vtba.
pub struct SolutionStream {
    rx: Receiver<StreamItem>,
}

impl SolutionStream {
    /// A connected (sink, stream) pair over a **bounded** channel of `capacity` in-flight
    /// solutions. `capacity` is the backpressure bound: a producer `emit` blocks once that
    /// many solutions are queued un-consumed. A `capacity` of `0` is promoted to `1` (a
    /// rendezvous channel would serialise every hand-off; `1` keeps one item of slack).
    /// sq-vtba.
    pub fn bounded(capacity: usize) -> (SolutionSink, SolutionStream) {
        let (tx, rx) = sync_channel(capacity.max(1));
        (SolutionSink { tx }, SolutionStream { rx })
    }

    /// A stream that yields an already-materialised set of `rows` under header `vars`, then
    /// ends. The bridge for a source that cannot stream incrementally (the engine stays
    /// materialised, §7): its whole answer is pushed onto a buffered channel up front, so it
    /// composes with the streaming operators uniformly. Runs no thread — the rows are loaded
    /// before the first pull. sq-vtba.
    pub fn from_rows(vars: Vec<String>, rows: Vec<Vec<Option<Term>>>) -> SolutionStream {
        // A buffered channel sized to hold every row (no producer thread to block on it).
        let (tx, rx) = sync_channel(rows.len().max(1));
        for cells in rows {
            // The receiver is still alive here (we hold `rx`), so every send succeeds.
            let _ = tx.send(Ok(Solution::new(vars.clone(), cells)));
        }
        SolutionStream { rx }
    }

    /// Unwrap into the underlying bounded receiver. Used by the Phase-5 streaming-join feeder
    /// in [`crate::operators`], which multiplexes two streams by readiness (`try_recv` /
    /// `recv`) without an async runtime. `pub(crate)` — the channel is an implementation
    /// detail of the boundary, not public surface. sq-vtba.
    pub(crate) fn into_rx(self) -> Receiver<StreamItem> {
        self.rx
    }

    /// An empty stream (no solutions). sq-vtba.
    pub fn empty() -> SolutionStream {
        let (_tx, rx) = sync_channel::<StreamItem>(1);
        // `_tx` drops here → the channel is closed → the receiver yields nothing.
        SolutionStream { rx }
    }

    /// Pull every remaining item into a `Vec`, **blocking** until the producers finish. The
    /// materialising drain used by tests and by a caller that wants the whole answer (it is
    /// exactly the bag the streaming operators produced, just collected). Short-circuits on
    /// the first error item. sq-vtba.
    pub fn collect_solutions(self) -> Result<Vec<Solution>, FedError> {
        let mut out = Vec::new();
        for item in self {
            out.push(item?);
        }
        Ok(out)
    }
}

impl Iterator for SolutionStream {
    type Item = StreamItem;

    /// Block until the next solution is ready, or `None` when every producer sink has been
    /// dropped and the channel is drained. sq-vtba.
    fn next(&mut self) -> Option<StreamItem> {
        self.rx.recv().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::NamedNode;
    use std::thread;

    fn nn(s: &str) -> Term {
        Term::NamedNode(NamedNode::new(s).unwrap())
    }

    fn sol(s: &str) -> Solution {
        Solution::new(vec!["s".into()], vec![Some(nn(s))])
    }

    #[test]
    fn from_rows_yields_then_ends() {
        let stream = SolutionStream::from_rows(
            vec!["s".into()],
            vec![vec![Some(nn("http://ex/a"))], vec![Some(nn("http://ex/b"))]],
        );
        let got = stream.collect_solutions().unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].get("s"), Some(&nn("http://ex/a")));
        assert_eq!(got[1].get("s"), Some(&nn("http://ex/b")));
    }

    #[test]
    fn empty_stream_yields_nothing() {
        let stream = SolutionStream::empty();
        assert!(stream.collect_solutions().unwrap().is_empty());
    }

    #[test]
    fn bounded_channel_delivers_before_producer_finishes() {
        // A producer thread emits 3 solutions over a bounded channel; the consumer can pull
        // the FIRST before the producer has emitted the rest — the non-blocking property.
        let (sink, mut stream) = SolutionStream::bounded(1);
        let producer = thread::spawn(move || {
            for s in ["http://ex/a", "http://ex/b", "http://ex/c"] {
                if !sink.emit(sol(s)) {
                    return;
                }
            }
            // sink drops here → stream ends.
        });
        // Pull the first item; the producer is still running (it has more to send, and the
        // bound-1 channel means it is blocked on the 3rd send until we pull).
        let first = stream.next().expect("a first solution").expect("ok");
        assert_eq!(first.get("s"), Some(&nn("http://ex/a")));
        // Drain the rest.
        let rest: Vec<_> = stream.map(|i| i.unwrap()).collect();
        assert_eq!(rest.len(), 2);
        producer.join().unwrap();
    }

    #[test]
    fn dropped_consumer_signals_producer_to_stop() {
        let (sink, stream) = SolutionStream::bounded(1);
        drop(stream); // consumer gone before any pull.
        // The first emit may succeed (slack) or fail; a later one must report closed.
        let _ = sink.emit(sol("http://ex/a"));
        // Once the slack is used, emit returns false (consumer gone) — the producer's stop
        // signal. (We loop to consume any slack deterministically.)
        let mut saw_closed = false;
        for _ in 0..4 {
            if !sink.emit(sol("http://ex/x")) {
                saw_closed = true;
                break;
            }
        }
        assert!(saw_closed, "a dropped consumer must eventually signal closed");
    }

    #[test]
    fn error_item_short_circuits_collect() {
        let (sink, stream) = SolutionStream::bounded(4);
        sink.emit(sol("http://ex/a"));
        sink.emit_err(FedError::Transport("boom".into()));
        sink.emit(sol("http://ex/b"));
        drop(sink);
        assert!(stream.collect_solutions().is_err());
    }

    // sq-qcnn.22: Additional direct unit tests for coverage ratchet
    #[test]
    fn solution_get_returns_none_for_unbound() {
        let sol = Solution::new(
            vec!["x".into(), "y".into()],
            vec![Some(nn("http://ex/a")), None],
        );
        assert_eq!(sol.get("x"), Some(&nn("http://ex/a")));
        assert_eq!(sol.get("y"), None);
        assert_eq!(sol.get("z"), None); // variable not in solution
    }

    #[test]
    fn solution_get_empty_solution() {
        let sol = Solution::new(vec![], vec![]);
        assert_eq!(sol.get("x"), None);
    }

    #[test]
    fn solution_new_builds_correctly() {
        let vars = vec!["a".into(), "b".into()];
        let cells = vec![Some(nn("http://ex/1")), None];
        let sol = Solution::new(vars.clone(), cells.clone());
        assert_eq!(sol.vars, vars);
        assert_eq!(sol.cells, cells);
    }

    #[test]
    fn bounded_zero_capacity_promoted_to_one() {
        // Capacity 0 → promoted to 1 (rendezvous → 1 item slack), so a single `emit` on this
        // thread does not block on the bound. Drop the sink before draining so the channel
        // closes and `next()` terminates (same discipline as `error_item_short_circuits_collect`);
        // otherwise the live sender keeps the receiver blocking after the last item. sq-qcnn.22
        let (sink, stream) = SolutionStream::bounded(0);
        assert!(sink.emit(sol("http://ex/a")), "promoted-to-1 slack accepts one item without blocking");
        drop(sink);
        let items: Vec<_> = stream.collect();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].as_ref().unwrap().get("s"), Some(&nn("http://ex/a")));
    }

    #[test]
    fn from_rows_zero_rows() {
        let stream = SolutionStream::from_rows(vec!["x".into()], vec![]);
        let got = stream.collect_solutions().unwrap();
        assert!(got.is_empty());
    }

    #[test]
    fn from_rows_single_row() {
        let stream = SolutionStream::from_rows(
            vec!["x".into()],
            vec![vec![Some(nn("http://ex/obj"))]],
        );
        let got = stream.collect_solutions().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].get("x"), Some(&nn("http://ex/obj")));
    }

    #[test]
    fn solution_sink_clone_multiple_emits() {
        // Cloned sink can emit independently; solutions arrive in emission order. sq-qcnn.22
        let (sink, stream) = SolutionStream::bounded(2);
        let sink2 = sink.clone();
        let ok1 = sink.emit(sol("http://ex/a"));
        let ok2 = sink2.emit(sol("http://ex/b"));
        drop(sink);
        drop(sink2);
        assert!(ok1);
        assert!(ok2);
        let got: Vec<_> = stream.map(|i| i.unwrap()).collect();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].get("s"), Some(&nn("http://ex/a")));
        assert_eq!(got[1].get("s"), Some(&nn("http://ex/b")));
    }
}
