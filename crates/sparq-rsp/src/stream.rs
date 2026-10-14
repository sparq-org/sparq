//! The stream model: application-timestamped triples and a push builder.
//!
//! Timestamps are **application-supplied** `u64`s (logical ticks, epoch millis,
//! sequence numbers — the engine does not care, and it NEVER reads the wall
//! clock). Window assignment, closure and lateness are therefore pure functions
//! of the pushed `(triple, ts)` sequence: deterministic and unit-testable.

use oxrdf::Term;

/// One stream element: a payload paired with its application-supplied
/// timestamp. The payload is generic so the same window machinery serves both
/// term triples (`[Term; 3]`, the public stream surface) and already-interned
/// id triples (`[Id; 3]`, the persistent-dictionary evaluation mode — see
/// [`EvalMode`](crate::EvalMode)).
#[derive(Debug, Clone, PartialEq)]
pub struct Timestamped<T> {
    /// The stream element (a subject/predicate/object triple in the public API).
    pub triple: T,
    /// Application timestamp (logical or epoch — any monotone-ish `u64` scale).
    pub ts: u64,
}

/// One stream element: a triple paired with its application-supplied timestamp.
pub type TimestampedTriple = Timestamped<[Term; 3]>;

/// A scripted RDF stream: a builder accepting pushes.
///
/// `TripleStream` is just an ordered buffer of [`TimestampedTriple`]s — handy
/// for scripting test streams and for handing a prefix of history to
/// [`WindowedStream::new`](crate::WindowedStream::new). Live pushes after
/// construction go directly to [`WindowedStream::push`](crate::WindowedStream::push)
/// or [`ContinuousQuery::push`](crate::ContinuousQuery::push).
#[derive(Debug, Clone, Default)]
pub struct TripleStream {
    items: Vec<TimestampedTriple>,
}

impl TripleStream {
    /// An empty stream.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends one element; chainable (`stream.push(t1, 0).push(t2, 1)`).
    pub fn push(&mut self, triple: [Term; 3], ts: u64) -> &mut Self {
        self.items.push(TimestampedTriple { triple, ts });
        self
    }

    /// Appends an already-wrapped element.
    pub fn push_item(&mut self, item: TimestampedTriple) -> &mut Self {
        self.items.push(item);
        self
    }

    /// Appends already-wrapped elements in iterator order.
    pub fn push_batch<I: IntoIterator<Item = TimestampedTriple>>(&mut self, items: I) -> &mut Self {
        for item in items {
            self.push_item(item);
        }
        self
    }

    /// Number of buffered elements.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The buffered elements, in push (arrival) order.
    pub fn items(&self) -> &[TimestampedTriple] {
        &self.items
    }

    /// Consumes the stream, yielding the elements in push order.
    pub fn into_items(self) -> Vec<TimestampedTriple> {
        self.items
    }
}
