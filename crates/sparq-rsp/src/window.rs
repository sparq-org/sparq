//! Window operators (the RSP-QL S2R step): time-based sliding windows
//! (`RANGE w STEP s`), count-based windows (`ROWS n`), and opt-in
//! gap-triggered session windows.
//!
//! # Time-window semantics (read this; the tests pin it down)
//!
//! * Windows are **half-open intervals `[start, end)`** with `start = t0 + k·step`,
//!   `end = start + range`, for `k = 0, 1, 2, …` — i.e. the window origin
//!   `t0` defaults to timestamp `0` (the RSP-QL parameterised origin is set
//!   with [`WindowSpec::with_t0`]). A triple with timestamp `t` belongs to
//!   window `k` iff `start ≤ t < end`: the start bound is INCLUSIVE, the end
//!   bound EXCLUSIVE, so `RANGE 10 STEP 10` windows `[0,10) [10,20) …`
//!   partition the timeline with no double-counting. A triple with `t < t0`
//!   predates the stream origin: it belongs to no window (and is not counted
//!   late — no window ever covered it), but it still advances the watermark,
//!   exactly like a gap arrival.
//! * **Closure is watermark-driven, never wall-clock-driven.** The watermark is
//!   `max_ts_seen − max_delay` (saturating). Window `k` CLOSES — its content is
//!   frozen and reported — as soon as the watermark reaches its `end`. Time
//!   only advances through pushed timestamps, so a quiet stream closes nothing
//!   until the next push (or [`WindowedStream::flush`]).
//! * **Out-of-order arrivals** within `max_delay` land in every still-open
//!   window that covers their timestamp. A triple whose LAST covering window
//!   has already closed is dropped and counted in
//!   [`late_dropped`](WindowedStream::late_dropped); if only some earlier
//!   covering windows closed, it still enters the open ones (standard
//!   streaming-system behaviour) and is not counted as late.
//! * **Empty windows are reported.** When the watermark jumps a gap, every
//!   window in between closes with no content — DSTREAM needs this (results
//!   must be observed disappearing), and it keeps emission deterministic.
//!   Windows wholly closed at the INITIAL watermark (the first arrival's `ts`
//!   minus `max_delay` — gap arrivals included) are skipped, so a stream
//!   starting at `ts = 10⁹` does not replay a billion empties — while every
//!   window the watermark holds open stays open even across the first push: a
//!   first push at `ts = 12` with `max_delay = 5` leaves `[0, 10)` open for a
//!   later `ts = 8`.
//! * `step > range` leaves timestamp gaps covered by no window; a triple in a
//!   gap enters no window and is not counted "late" — but its timestamp still
//!   ADVANCES the watermark (event time passed), closing earlier windows.
//!
//! # Count-window semantics
//!
//! `ROWS n` follows CQL: the window holds the last `min(n, arrivals)` triples
//! in ARRIVAL order (timestamps are carried but do not influence membership),
//! and is reported on every arrival — or every `slide`-th arrival with
//! [`WindowSpec::with_slide`]. Reported bounds are the **inclusive**
//! `[first.ts, last.ts]` of the content (empty never occurs: a count window is
//! only reported after an arrival). [`flush`](WindowedStream::flush) is a
//! no-op for count windows — there is no watermark to advance.
//!
//! # Session-window semantics (`session_windows` feature)
//!
//! `WindowSpec::session(gap)` groups timestamp-ordered events into maximal
//! runs whose consecutive timestamp gaps are strictly less than `gap`. A gap
//! equal to `gap` starts a new session. Closure is event-time-only: a session
//! closes when the watermark reaches `last_event_ts + gap`; `flush` closes the
//! final open session. Reported bounds are the inclusive
//! `[first_event_ts, last_event_ts]` of the session content.

use std::collections::{BTreeMap, VecDeque};

use oxrdf::Term;

use crate::stream::{Timestamped, TripleStream};

/// A window specification (the S2R operator of RSP-QL).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowSpec {
    /// `RANGE range STEP step` — sliding (or, when `step == range`, tumbling)
    /// time window. `max_delay` is the out-of-order tolerance: the watermark
    /// lags the maximum seen timestamp by this much, holding windows open for
    /// late arrivals (0 = closed at the first sight of a newer-window triple).
    /// `t0` is the RSP-QL window origin: window `k` covers
    /// `[t0 + k·step, t0 + k·step + range)`.
    Time { range: u64, step: u64, max_delay: u64, t0: u64 },
    /// `ROWS rows` — count window over the last `rows` arrivals, reported
    /// every `slide` arrivals (CQL default: every arrival, `slide = 1`).
    Count { rows: usize, slide: usize },
    /// A gap-triggered event-time session window. Consecutive events whose
    /// timestamp difference is less than `gap` share a session; a difference
    /// greater than or equal to `gap` starts a new session.
    #[cfg(feature = "session_windows")]
    Session { gap: u64 },
}

impl WindowSpec {
    /// `RANGE range STEP step`, no lateness tolerance, origin `t0 = 0`.
    ///
    /// # Panics
    /// If `range == 0` or `step == 0` (a zero-width or non-advancing window
    /// is meaningless and would loop forever).
    pub fn time(range: u64, step: u64) -> Self {
        assert!(range > 0, "window RANGE must be > 0");
        assert!(step > 0, "window STEP must be > 0");
        WindowSpec::Time { range, step, max_delay: 0, t0: 0 }
    }

    /// `ROWS rows`, reported on every arrival.
    ///
    /// # Panics
    /// If `rows == 0`.
    pub fn count(rows: usize) -> Self {
        assert!(rows > 0, "window ROWS must be > 0");
        WindowSpec::Count { rows, slide: 1 }
    }

    /// Creates a gap-triggered event-time session window.
    ///
    /// `gap` uses the same application-supplied `u64` logical timestamp unit
    /// as stream elements. A timestamp difference equal to `gap` starts a new
    /// session. Session bounds are inclusive.
    ///
    /// # Panics
    /// If `gap == 0`.
    #[cfg(feature = "session_windows")]
    pub fn session(gap: u64) -> Self {
        assert!(gap > 0, "session window GAP must be > 0");
        WindowSpec::Session { gap }
    }

    /// Sets the out-of-order tolerance of a time window.
    ///
    /// # Panics
    /// On a count window (count windows are arrival-ordered; lateness does
    /// not apply).
    pub fn with_max_delay(self, max_delay: u64) -> Self {
        match self {
            WindowSpec::Time { range, step, t0, .. } => {
                WindowSpec::Time { range, step, max_delay, t0 }
            }
            WindowSpec::Count { .. } => panic!("max_delay applies to time windows only"),
            #[cfg(feature = "session_windows")]
            WindowSpec::Session { .. } => panic!("max_delay applies to time windows only"),
        }
    }

    /// Sets the window origin `t0` of a time window (the RSP-QL `t0`
    /// parameter): window `k` covers `[t0 + k·step, t0 + k·step + range)`.
    /// Arrivals with `ts < t0` predate the stream origin — they enter no
    /// window (and are not counted late) but still advance the watermark.
    ///
    /// # Panics
    /// On a count window (count windows have no time axis).
    pub fn with_t0(self, t0: u64) -> Self {
        match self {
            WindowSpec::Time { range, step, max_delay, .. } => {
                WindowSpec::Time { range, step, max_delay, t0 }
            }
            WindowSpec::Count { .. } => panic!("t0 applies to time windows only"),
            #[cfg(feature = "session_windows")]
            WindowSpec::Session { .. } => panic!("t0 applies to time windows only"),
        }
    }

    /// Sets the report cadence of a count window (report every `slide`
    /// arrivals instead of every arrival).
    ///
    /// # Panics
    /// On a time window, or if `slide == 0`.
    pub fn with_slide(self, slide: usize) -> Self {
        assert!(slide > 0, "window SLIDE must be > 0");
        match self {
            WindowSpec::Count { rows, .. } => WindowSpec::Count { rows, slide },
            WindowSpec::Time { .. } => panic!("slide applies to count windows only (use STEP)"),
            #[cfg(feature = "session_windows")]
            WindowSpec::Session { .. } => panic!("slide applies to count windows only"),
        }
    }
}

/// One CLOSED (time/session) or REPORTED (count) window: bounds + frozen content.
/// Generic over the stream payload (the public API works with `[Term; 3]`;
/// the persistent-dictionary evaluation mode windows `[Id; 3]`s).
#[derive(Debug, Clone)]
pub struct Window<T = [Term; 3]> {
    /// Time window: inclusive start `t0 + k·step`. Count/session window:
    /// inclusive `ts` of the oldest content triple.
    pub start: u64,
    /// Time window: EXCLUSIVE end `start + range`. Count/session window:
    /// INCLUSIVE `ts` of the newest content triple.
    pub end: u64,
    /// Content. Time/session windows: timestamp order (arrival order within
    /// equal timestamps). Count windows: arrival order.
    pub triples: Vec<Timestamped<T>>,
}

/// Maintains the active window content of one stream incrementally and
/// surfaces windows as they close.
///
/// Time windows buffer by timestamp in a `BTreeMap<ts, Vec<triple>>`; closing
/// window `k` is a range read `[start, start+range)` and sliding evicts every
/// entry older than the next window's start with one `split_off`. Count windows
/// are a ring buffer (`VecDeque`) capped at `rows`.
///
/// Closed windows accumulate internally; drain them with
/// [`take_closed`](Self::take_closed) (or let [`ContinuousQuery`](crate::ContinuousQuery)
/// drive the whole loop).
#[derive(Debug)]
pub struct WindowedStream<T = [Term; 3]> {
    spec: WindowSpec,
    /// Time-window buffer: triples (in arrival order per timestamp) keyed by ts.
    /// Holds everything from the oldest OPEN window's start upwards.
    buffer: BTreeMap<u64, Vec<T>>,
    /// Index `k` of the oldest window not yet closed; `None` until the first
    /// accepted push fixes the starting window.
    next_close: Option<u64>,
    /// Maximum timestamp seen (watermark = `max_ts − max_delay`).
    max_ts: u64,
    /// Too-late arrivals dropped (every covering window already closed).
    late_dropped: u64,
    /// Count-window ring buffer (last `rows` arrivals).
    ring: VecDeque<Timestamped<T>>,
    /// Count-window arrival counter (drives `slide`).
    arrivals: u64,
    /// End-of-stream horizon for session windows. A flush freezes the final
    /// session, so later arrivals at or below this timestamp are late.
    #[cfg(feature = "session_windows")]
    session_flushed_horizon: Option<u64>,
    /// Whether a session event has been accepted. Distinguishes an empty
    /// stream from a first event at timestamp zero when recording a flush.
    #[cfg(feature = "session_windows")]
    session_seen_event: bool,
    /// Windows closed/reported but not yet taken.
    closed: Vec<Window<T>>,
}

impl WindowedStream<[Term; 3]> {
    /// A windowed view over a scripted stream: every buffered element is
    /// pushed in order. Drain whatever already closed with
    /// [`take_closed`](Self::take_closed); keep pushing live elements with
    /// [`push`](Self::push).
    pub fn new(stream: TripleStream, spec: WindowSpec) -> Self {
        let mut ws = Self::empty(spec);
        for item in stream.into_items() {
            ws.push(item.triple, item.ts);
        }
        ws
    }
}

impl<T: Clone> WindowedStream<T> {
    /// A windowed view with no history (live pushes only).
    pub fn empty(spec: WindowSpec) -> Self {
        WindowedStream {
            spec,
            buffer: BTreeMap::new(),
            next_close: None,
            max_ts: 0,
            late_dropped: 0,
            ring: VecDeque::new(),
            arrivals: 0,
            #[cfg(feature = "session_windows")]
            session_flushed_horizon: None,
            #[cfg(feature = "session_windows")]
            session_seen_event: false,
            closed: Vec::new(),
        }
    }

    /// The window specification this stream was built with.
    pub fn spec(&self) -> WindowSpec {
        self.spec
    }

    /// Pushes one stream element. Any window this closes is queued for
    /// [`take_closed`](Self::take_closed).
    pub fn push(&mut self, triple: T, ts: u64) {
        match self.spec {
            WindowSpec::Time { range, step, max_delay, t0 } => {
                self.push_time(triple, ts, range, step, max_delay, t0)
            }
            WindowSpec::Count { rows, slide } => self.push_count(triple, ts, rows, slide),
            #[cfg(feature = "session_windows")]
            WindowSpec::Session { gap } => self.push_session(triple, ts, gap),
        }
    }

    /// sq-zckkq: Inserts into the still-open event-time session, or
    /// starts a new one. With the v1 session watermark delay fixed at zero, an
    /// out-of-order event is accepted only when it joins an open session; an
    /// isolated session whose inactivity deadline is already behind the
    /// watermark is late and cannot retroactively surface.
    #[cfg(feature = "session_windows")]
    fn push_session(&mut self, triple: T, ts: u64, gap: u64) {
        if self.session_flushed_horizon.is_some_and(|horizon| ts <= horizon) {
            self.late_dropped += 1;
            return;
        }

        let watermark = self.max_ts.max(ts);
        let joins_open_session = self
            .buffer
            .range(..=ts)
            .next_back()
            .is_some_and(|(&previous, _)| ts - previous < gap)
            || self
                .buffer
                .range(ts..)
                .next()
                .is_some_and(|(&next, _)| next - ts < gap);

        if !joins_open_session && ts.saturating_add(gap) <= watermark {
            self.late_dropped += 1;
            return;
        }

        self.buffer.entry(ts).or_default().push(triple);
        self.session_seen_event = true;
        self.max_ts = watermark;
        self.close_ready_sessions(gap, watermark, false);
    }

    /// Closes timestamp-ordered session components from oldest to newest.
    /// `force` is the end-of-stream path and ignores inactivity deadlines.
    #[cfg(feature = "session_windows")]
    fn close_ready_sessions(&mut self, gap: u64, watermark: u64, force: bool) {
        while let Some(start) = self.buffer.first_key_value().map(|(&ts, _)| ts) {
            let mut last = start;
            for ts in self.buffer.range(start..).map(|(&ts, _)| ts) {
                if ts - last >= gap {
                    break;
                }
                last = ts;
            }

            if !force && watermark < last.saturating_add(gap) {
                break;
            }

            let triples = self
                .buffer
                .range(start..=last)
                .flat_map(|(&ts, ts_triples)| {
                    ts_triples.iter().map(move |t| Timestamped { triple: t.clone(), ts })
                })
                .collect();
            self.closed.push(Window { start, end: last, triples });

            let mut after = self.buffer.split_off(&last);
            after.remove(&last);
            self.buffer = after;
        }
    }

    fn push_time(&mut self, triple: T, ts: u64, range: u64, step: u64, max_delay: u64, t0: u64) {
        // The FIRST arrival fixes the starting window: the oldest window its
        // WATERMARK (ts − max_delay) still holds open, i.e. the first k with
        // t0 + k·step + range > ts − max_delay. Anchoring on the watermark
        // instead of on ts keeps the lateness contract honest from the very
        // first push — with max_delay = 5, a first push at ts = 12 must leave
        // [0,10) open for a subsequent ts = 8 (watermark 7 < 10). With
        // max_delay = 0 this is exactly the first window covering ts, so the
        // all-empty prefix of the axis is still skipped. GAP arrivals
        // (step > range, see below) anchor too: they advance event time, so
        // the windows their watermark holds open must be tracked — and
        // eventually emitted empty — not silently skipped (roborev 1693).
        if self.next_close.is_none() {
            let wm = ts.saturating_sub(max_delay).saturating_sub(t0);
            let k_min = if wm >= range { (wm - range) / step + 1 } else { 0 };
            self.next_close = Some(k_min);
        }
        // Window k covers [t0 + k·step, t0 + k·step + range). The LAST window
        // covering ts is k_max = (ts − t0) / step — valid only if ts actually
        // falls inside it ((ts − t0) % step < range can fail when step > range:
        // gap timestamps are covered by no window) and ts ≥ t0 at all (a
        // pre-origin timestamp belongs to no window — like a gap arrival, it
        // only advances the watermark).
        if ts >= t0 {
            let rel = ts - t0;
            let k_max = rel / step;
            if rel - k_max * step < range {
                if k_max < self.next_close.expect("anchored above") {
                    // Every window covering ts already closed: too late.
                    self.late_dropped += 1;
                    return;
                }
                self.buffer.entry(ts).or_default().push(triple);
            }
        }
        // A gap triple (step > range) enters no window, but it is still
        // evidence of event time passing: it advances the watermark and can
        // close (and empty-report) earlier windows, exactly like any arrival.
        if ts > self.max_ts {
            self.max_ts = ts;
        }
        // Close every window whose end the watermark has reached.
        let watermark = self.max_ts.saturating_sub(max_delay);
        while let Some(k) = self.next_close {
            let start = t0 + k * step;
            if watermark < start + range {
                break;
            }
            self.close_time_window(k, range, step, t0);
        }
    }

    /// Advances event time to `ts` WITHOUT inserting a triple — a
    /// watermark heartbeat. Closes every time/session window the new watermark
    /// has reached, exactly as a gap arrival at `ts` would, but buffers nothing.
    /// Time windows empty-report skipped intervals; session windows have no
    /// empty intervals to report. A no-op for count windows (no watermark) and
    /// when `ts` does not advance `max_ts`.
    ///
    /// Used by [`ContinuousMultiQuery`](crate::ContinuousMultiQuery) to drive
    /// every window off a SHARED event-time clock: a triple on one stream
    /// advances the watermark of windows on the OTHER streams, so closure is
    /// synchronized across the join. Treated like a gap arrival for anchoring:
    /// a heartbeat is evidence of event time passing, so a first heartbeat
    /// anchors the starting window on its watermark (windows it holds open are
    /// tracked and eventually emitted, possibly empty), matching the documented
    /// gap-first-arrival rule.
    pub fn advance(&mut self, ts: u64) {
        #[cfg(feature = "session_windows")]
        if let WindowSpec::Session { gap } = self.spec {
            if ts > self.max_ts {
                self.max_ts = ts;
            }
            self.close_ready_sessions(gap, self.max_ts, false);
            return;
        }

        let WindowSpec::Time { range, step, max_delay, t0 } = self.spec else {
            return; // count windows have no time axis / watermark
        };
        if self.next_close.is_none() {
            let wm = ts.saturating_sub(max_delay).saturating_sub(t0);
            let k_min = if wm >= range { (wm - range) / step + 1 } else { 0 };
            self.next_close = Some(k_min);
        }
        if ts > self.max_ts {
            self.max_ts = ts;
        }
        let watermark = self.max_ts.saturating_sub(max_delay);
        while let Some(k) = self.next_close {
            let start = t0 + k * step;
            if watermark < start + range {
                break;
            }
            self.close_time_window(k, range, step, t0);
        }
    }

    /// Freezes window `k`'s content, queues it, slides to `k + 1` and evicts
    /// everything older than the new oldest open window.
    fn close_time_window(&mut self, k: u64, range: u64, step: u64, t0: u64) {
        let start = t0 + k * step;
        let end = start + range;
        let triples = self
            .buffer
            .range(start..end)
            .flat_map(|(&ts, ts_triples)| {
                ts_triples.iter().map(move |t| Timestamped { triple: t.clone(), ts })
            })
            .collect();
        self.closed.push(Window { start, end, triples });
        self.next_close = Some(k + 1);
        // Evict: nothing below the next window's start can be read again.
        self.buffer = self.buffer.split_off(&(t0 + (k + 1) * step));
    }

    fn push_count(&mut self, triple: T, ts: u64, rows: usize, slide: usize) {
        self.arrivals += 1;
        self.ring.push_back(Timestamped { triple, ts });
        if self.ring.len() > rows {
            self.ring.pop_front();
        }
        // (sq-qmth) stable-1.96 clippy `manual_is_multiple_of` (is_multiple_of
        // stable since 1.87, within the 1.88 MSRV floor).
        if self.arrivals.is_multiple_of(slide as u64) {
            // Reported on arrival, so the ring is never empty here.
            let start = self.ring.front().expect("non-empty ring").ts;
            let end = self.ring.back().expect("non-empty ring").ts;
            self.closed.push(Window { start, end, triples: self.ring.iter().cloned().collect() });
        }
    }

    /// End-of-stream: closes every time window up to and including the last
    /// one covering `max_ts` regardless of `max_delay`, or the final open
    /// session, and returns ALL pending windows (the flushed ones plus any not
    /// yet taken). No-op for count windows beyond draining. After a flush,
    /// further pushes at or below the flushed horizon count as late.
    pub fn flush(&mut self) -> Vec<Window<T>> {
        #[cfg(feature = "session_windows")]
        if let WindowSpec::Session { gap } = self.spec {
            if self.session_seen_event {
                self.session_flushed_horizon = Some(self.max_ts);
            }
            self.close_ready_sessions(gap, self.max_ts, true);
            return self.take_closed();
        }

        if let WindowSpec::Time { range, step, t0, .. } = self.spec {
            while let Some(k) = self.next_close {
                if t0 + k * step > self.max_ts {
                    break; // window starts after every triple we ever saw
                }
                self.close_time_window(k, range, step, t0);
            }
        }
        self.take_closed()
    }

    /// Drains the windows that closed since the last call, oldest first.
    pub fn take_closed(&mut self) -> Vec<Window<T>> {
        std::mem::take(&mut self.closed)
    }

    /// How many arrivals were dropped as too late (time/session windows only).
    pub fn late_dropped(&self) -> u64 {
        self.late_dropped
    }

    /// Visits every LIVE payload still held by this stream — the buffered open
    /// windows (time) or the ring (count), PLUS any windows already closed but
    /// not yet taken. The complete set of payloads this stream can still surface
    /// to the caller; nothing outside it is reachable any more (the slide's
    /// `split_off` has dropped it). Used by persistent-dictionary
    /// compaction to compute which interned terms are still referenced by a live
    /// window — every id NOT visited here has aged out of every window and its
    /// dictionary entry is reclaimable.
    pub(crate) fn for_each_live_payload(&self, mut f: impl FnMut(&T)) {
        for ts_triples in self.buffer.values() {
            for t in ts_triples {
                f(t);
            }
        }
        for item in &self.ring {
            f(&item.triple);
        }
        for w in &self.closed {
            for t in &w.triples {
                f(&t.triple);
            }
        }
    }

    /// Rewrites every LIVE payload in place (same set visited by
    /// [`for_each_live_payload`](Self::for_each_live_payload)). The remap MUST be
    /// a bijection on the live payloads (it is: a dictionary rebuild assigns each
    /// surviving id a fresh dense id), so window membership, ordering and counts
    /// are untouched — only the encoding of each payload changes. Used to swing
    /// the live window indexes onto a freshly-compacted dictionary ATOMICALLY:
    /// after this call no live payload references an old (reclaimed) id.
    pub(crate) fn remap_live_payloads(&mut self, mut f: impl FnMut(&T) -> T) {
        for ts_triples in self.buffer.values_mut() {
            for t in ts_triples {
                *t = f(t);
            }
        }
        for item in &mut self.ring {
            item.triple = f(&item.triple);
        }
        for w in &mut self.closed {
            for t in &mut w.triples {
                t.triple = f(&t.triple);
            }
        }
    }
}
