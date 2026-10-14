//! **ANAPSID-style non-blocking streaming join with bounded operator spill**.
//!
//! The static [`crate::plan`] decides *which* algorithm a binary join uses. This module
//! is the **execution-side operator** for the streaming case: a symmetric (XJoin-style)
//! hash join that consumes two *incrementally-arriving* tuple streams — as federated
//! sub-queries to different sources return results at different rates — and emits joined
//! results **without first materialising either full input**. It is memory-bounded: when
//! the in-memory hash tables exceed a configured budget, whole join-key partitions are
//! **spilled** to a backing store and reconciled on probe, so the operator is bounded,
//! not OOM-prone, on inputs larger than memory.
//!
//! Like the rest of `sparq-fedplan`, this drives off in-memory incremental iterators /
//! fixtures rather than live network I/O, and is fully deterministic.
//!
//! ## The operator (symmetric hash join)
//!
//! A binary join over two streams `L` and `R` on a set of shared **join variables**. Both
//! sides build *and* probe:
//!
//! * On a new **left** tuple `t` (key `k` = its join-var values): probe the right side for
//!   key `k` and emit `t ⋈ r` for every right tuple `r` already seen with that key, then
//!   store `t` on the left side.
//! * On a new **right** tuple, symmetrically.
//!
//! Because each side probes the other *as tuples arrive*, a match `(l, r)` is emitted the
//! moment the **later-arriving** of the two is processed — never waiting for either input
//! to finish. That is the non-blocking property: output is produced before either stream
//! is exhausted (asserted in `emits_before_inputs_exhausted`).
//!
//! ## Bounded spill (XJoin-style)
//!
//! The operator caps the number of tuples held in memory at
//! [`StreamJoinOptions::mem_budget_tuples`]. When an insert would exceed the budget, it
//! **spills** the currently-largest in-memory partition (one join key, the larger of the
//! two sides for that key) to a backing run — by default a temp file under
//! [`std::env::temp_dir`], using only `std` (no new dependency); an in-memory simulation
//! is available for tests via [`SpillStore::Memory`]. Spilling only *moves* tuples out of
//! the live tables; it never drops them.
//!
//! ### Correctness invariant (load-bearing)
//!
//! > The streamed + spilled result is **multiset-equal** to the equivalent *blocking*
//! > hash join — same tuples, no loss, no duplication — for any interleaving of the two
//! > input streams and any memory budget (including a budget so low that every partition
//! > spills).
//!
//! *Proof sketch.* Every output row corresponds to a unique ordered pair `(l, r)` of a
//! left and a right tuple sharing a join key. A pair is emitted exactly once: when the
//! **second** of `l, r` to arrive is processed, it probes the opposite side for its key.
//! The probe consults **both** the in-memory bucket *and* every spilled run for that key,
//! and spilling only relocates a tuple (memory bucket → spill run) without deleting it, so
//! the first-arrived tuple of the pair is always found in exactly one place. Hence each
//! matching pair is emitted once and only once, regardless of budget or interleaving — the
//! same multiset the blocking join `{ l ⋈ r : key(l)=key(r) }` produces. Verified by
//! `streamed_equals_blocking_*` and `spill_path_equals_no_spill_*`.
//!
//! ## DEFERRED (filed as a bead, not built here)
//!
//! Live **adaptive re-planning** — switching the join algorithm or source mid-execution
//! when observed rates diverge from the estimate (the harder half of ANAPSID's
//! adaptivity) — is out of scope for this slice. This is the non-blocking operator + the
//! bounded spill only. See the roadmap bead filed from sq-vf7q under epic sq-3183.
//!
//! sq-vf7q.

use crate::pattern::Var;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, BufWriter, Write};

/// A solution tuple: variable bindings as `(variable, value)` pairs. Values are stored as
/// their lexical string (an IRI bare, a literal already rendered), matching the light term
/// model the planner uses ([`crate::pattern::Term`]). Bindings are kept in a stable,
/// variable-sorted order so equal tuples are byte-identical (deterministic spill + compare).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tuple {
    bindings: Vec<(Var, String)>,
}

impl Tuple {
    /// Builds a tuple from `(var, value)` pairs. Duplicate variables keep the first value;
    /// bindings are sorted by variable name for a canonical representation.
    pub fn new(bindings: impl IntoIterator<Item = (Var, String)>) -> Tuple {
        let mut seen: Vec<(Var, String)> = Vec::new();
        for (v, val) in bindings {
            if !seen.iter().any(|(sv, _)| sv == &v) {
                seen.push((v, val));
            }
        }
        seen.sort_by(|a, b| a.0.cmp(&b.0));
        Tuple { bindings: seen }
    }

    /// The value bound to `var`, if any.
    pub fn get(&self, var: &Var) -> Option<&str> {
        self.bindings
            .iter()
            .find(|(v, _)| v == var)
            .map(|(_, val)| val.as_str())
    }

    /// The bindings, in canonical (variable-sorted) order.
    pub fn bindings(&self) -> &[(Var, String)] {
        &self.bindings
    }

    /// The join key for this tuple over `join_vars`: the values of those variables in the
    /// given order, joined with a delimiter that cannot occur in an IRI/literal lexical
    /// form unescaped (a unit separator). A tuple missing a join var yields `None` (it
    /// cannot join and is dropped — same as a blocking join's unbound-key behaviour).
    fn key(&self, join_vars: &[Var]) -> Option<String> {
        let mut k = String::new();
        for (i, v) in join_vars.iter().enumerate() {
            if i > 0 {
                k.push('\u{1f}');
            }
            k.push_str(self.get(v)?);
        }
        Some(k)
    }

    /// Merges this (left) tuple with a `right` tuple. The join variables already agree (the
    /// keys matched); the right's non-join bindings are added. On the join variables the
    /// left value is kept (they are equal by construction). Result is re-canonicalised.
    fn merge(&self, right: &Tuple) -> Tuple {
        let mut pairs = self.bindings.clone();
        for (v, val) in &right.bindings {
            if !pairs.iter().any(|(pv, _)| pv == v) {
                pairs.push((v.clone(), val.clone()));
            }
        }
        Tuple::new(pairs)
    }

    /// Serialises to one line for a spill run: `var\u{1f}value\u{1e}var\u{1f}value...`.
    fn encode(&self) -> String {
        let mut s = String::new();
        for (i, (v, val)) in self.bindings.iter().enumerate() {
            if i > 0 {
                s.push('\u{1e}');
            }
            s.push_str(&v.0);
            s.push('\u{1f}');
            s.push_str(val);
        }
        s
    }

    /// Parses a line written by [`Tuple::encode`].
    fn decode(line: &str) -> Tuple {
        if line.is_empty() {
            return Tuple {
                bindings: Vec::new(),
            };
        }
        let bindings = line.split('\u{1e}').filter_map(|field| {
            let (v, val) = field.split_once('\u{1f}')?;
            Some((Var::new(v), val.to_string()))
        });
        // Already canonical on disk, but re-canonicalise defensively.
        Tuple::new(bindings)
    }
}

/// Where spilled partitions are kept. Defaults to a temp file under [`std::env::temp_dir`]
/// (real, OS-backed, bounded memory — std only, no new dependency). [`SpillStore::Memory`]
/// keeps spilled runs in a side `Vec` instead — a deterministic in-process simulation used
/// by tests to exercise the spill *logic* without touching the filesystem. Both paths
/// produce the identical result multiset (the spill is transparent to correctness).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpillStore {
    /// Spill partitions to append-only temp files (default; bounds resident memory).
    TempFile,
    /// Keep spilled partitions in process memory (a simulation for deterministic tests).
    Memory,
}

/// Tuning knobs for [`StreamJoin`].
#[derive(Debug, Clone)]
pub struct StreamJoinOptions {
    /// Maximum number of tuples held across both in-memory hash tables before a partition
    /// is spilled. A smaller budget forces more (and earlier) spilling. `0` is treated as
    /// `1` (always spill on the second tuple) so the spill path is reachable in tests.
    pub mem_budget_tuples: usize,
    /// Where spilled partitions go.
    pub spill_store: SpillStore,
}

impl Default for StreamJoinOptions {
    fn default() -> Self {
        StreamJoinOptions {
            // A generous default: most federated intermediates fit; spill is the safety
            // valve for the unselective tail, not the common path.
            mem_budget_tuples: 1 << 20,
            spill_store: SpillStore::TempFile,
        }
    }
}

/// One side's state: live (in-memory) buckets keyed by join key, plus spilled runs.
struct Side {
    /// Live tuples per join key.
    mem: HashMap<String, Vec<Tuple>>,
    /// Spilled runs per join key. Each run is a [`SpillRun`] (a temp file or memory vec).
    spilled: HashMap<String, Vec<SpillRun>>,
    /// Resident tuple count (sum over `mem` buckets).
    resident: usize,
}

impl Side {
    fn new() -> Side {
        Side {
            mem: HashMap::new(),
            spilled: HashMap::new(),
            resident: 0,
        }
    }
}

/// A spilled partition: tuples for a single join key relocated out of memory.
enum SpillRun {
    /// Tuples written to (and re-read from) a temp file.
    File(std::path::PathBuf),
    /// Tuples kept in memory (the [`SpillStore::Memory`] simulation).
    Memory(Vec<Tuple>),
}

impl SpillRun {
    /// Streams the tuples in this run for probing.
    fn read(&self) -> Vec<Tuple> {
        match self {
            SpillRun::Memory(v) => v.clone(),
            SpillRun::File(path) => {
                let f = std::fs::File::open(path).expect("[OPUS-4.8] sq-vf7q: spill run readable");
                BufReader::new(f)
                    .lines()
                    .map(|l| Tuple::decode(&l.expect("spill line")))
                    .collect()
            }
        }
    }
}

impl Drop for SpillRun {
    fn drop(&mut self) {
        if let SpillRun::File(path) = self {
            let _ = std::fs::remove_file(path); // best-effort temp cleanup.
        }
    }
}

/// A symmetric, non-blocking streaming hash join with bounded operator spill.
///
/// Feed it tuples from either side as they arrive ([`StreamJoin::push_left`] /
/// [`StreamJoin::push_right`]); each call returns the joined tuples that newly became
/// derivable from that arrival. The operator never blocks on either input being complete.
pub struct StreamJoin {
    join_vars: Vec<Var>,
    left: Side,
    right: Side,
    opts: StreamJoinOptions,
    /// Monotonic id for unique spill file names.
    spill_seq: u64,
}

impl StreamJoin {
    /// Creates a streaming join on the shared `join_vars` (the variables both inputs bind
    /// — the equi-join key). The order of `join_vars` is canonicalised internally.
    pub fn new(join_vars: impl IntoIterator<Item = Var>, opts: StreamJoinOptions) -> StreamJoin {
        let mut jv: Vec<Var> = join_vars.into_iter().collect();
        jv.sort();
        jv.dedup();
        StreamJoin {
            join_vars: jv,
            left: Side::new(),
            right: Side::new(),
            opts,
            spill_seq: 0,
        }
    }

    /// Feeds one tuple from the **left** input; returns every result that this arrival
    /// completes (probing the right side, both in memory and spilled).
    pub fn push_left(&mut self, tuple: Tuple) -> Vec<Tuple> {
        self.push(tuple, true)
    }

    /// Feeds one tuple from the **right** input; returns every newly-derivable result.
    pub fn push_right(&mut self, tuple: Tuple) -> Vec<Tuple> {
        self.push(tuple, false)
    }

    fn push(&mut self, tuple: Tuple, from_left: bool) -> Vec<Tuple> {
        let Some(key) = tuple.key(&self.join_vars) else {
            // A tuple that does not bind every join variable can never match — drop it,
            // exactly as a blocking equi-join would (an unbound key joins with nothing).
            return Vec::new();
        };

        // Probe the OPPOSITE side for this key: in-memory bucket + every spilled run.
        let mut out = Vec::new();
        let opposite = if from_left { &self.right } else { &self.left };
        if let Some(bucket) = opposite.mem.get(&key) {
            for other in bucket {
                out.push(merge_ordered(&tuple, other, from_left));
            }
        }
        if let Some(runs) = opposite.spilled.get(&key) {
            for run in runs {
                for other in run.read() {
                    out.push(merge_ordered(&tuple, &other, from_left));
                }
            }
        }

        // Store this tuple on its OWN side, then enforce the memory budget.
        {
            let side = if from_left {
                &mut self.left
            } else {
                &mut self.right
            };
            side.mem.entry(key).or_default().push(tuple);
            side.resident += 1;
        }
        self.enforce_budget();
        out
    }

    /// Spills partitions until resident memory is within budget. The victim each round is
    /// the largest live bucket across both sides (most memory reclaimed per spill);
    /// ties break on (side, key) for determinism.
    fn enforce_budget(&mut self) {
        let budget = self.opts.mem_budget_tuples.max(1);
        while self.left.resident + self.right.resident > budget {
            // Find the largest bucket; prefer left on a tie, then smallest key.
            let left_best = largest_bucket(&self.left.mem).map(|(k, n)| (k.clone(), n));
            let right_best = largest_bucket(&self.right.mem).map(|(k, n)| (k.clone(), n));
            let spill_left = match (&left_best, &right_best) {
                (None, None) => break, // nothing left to spill (every bucket empty).
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (Some((_, ln)), Some((_, rn))) => ln >= rn,
            };
            if spill_left {
                let key = left_best.unwrap().0;
                self.spill_key(true, &key);
            } else {
                let key = right_best.unwrap().0;
                self.spill_key(false, &key);
            }
        }
    }

    /// Moves one join-key bucket out of memory into a spill run on the given side.
    fn spill_key(&mut self, from_left: bool, key: &str) {
        let store = self.opts.spill_store;
        let seq = self.spill_seq;
        self.spill_seq += 1;
        let side = if from_left {
            &mut self.left
        } else {
            &mut self.right
        };
        let Some(bucket) = side.mem.remove(key) else {
            return;
        };
        side.resident -= bucket.len();
        let run = match store {
            SpillStore::Memory => SpillRun::Memory(bucket),
            SpillStore::TempFile => {
                let mut path = std::env::temp_dir();
                path.push(format!(
                    "sparq-fedplan-spill-{}-{}-{}.run",
                    std::process::id(),
                    if from_left { "l" } else { "r" },
                    seq
                ));
                let f =
                    std::fs::File::create(&path).expect("[OPUS-4.8] sq-vf7q: spill run creatable");
                let mut w = BufWriter::new(f);
                for t in &bucket {
                    writeln!(w, "{}", t.encode()).expect("[OPUS-4.8] sq-vf7q: spill write");
                }
                w.flush().expect("spill flush");
                SpillRun::File(path)
            }
        };
        side.spilled.entry(key.to_string()).or_default().push(run);
    }

    /// Whether any partition has been spilled (for tests / introspection).
    pub fn spilled(&self) -> bool {
        !self.left.spilled.is_empty() || !self.right.spilled.is_empty()
    }
}

/// Merges in the canonical (left, right) order regardless of which side `tuple` came from,
/// so the output binding set is independent of arrival order.
fn merge_ordered(tuple: &Tuple, other: &Tuple, tuple_is_left: bool) -> Tuple {
    if tuple_is_left {
        tuple.merge(other)
    } else {
        other.merge(tuple)
    }
}

/// The largest bucket in a side's live map, as `(&key, len)`; ties break on smallest key.
fn largest_bucket(mem: &HashMap<String, Vec<Tuple>>) -> Option<(&String, usize)> {
    mem.iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| (k, v.len()))
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0)))
}

/// A reference **blocking** hash join, used both as a building block and as the test
/// oracle: materialise the whole right side into a hash table keyed by join key, then scan
/// the left and emit every match. Result order is left-scan × bucket order. This is the
/// multiset the streaming operator must reproduce.
pub fn blocking_hash_join(left: &[Tuple], right: &[Tuple], join_vars: &[Var]) -> Vec<Tuple> {
    let mut jv: Vec<Var> = join_vars.to_vec();
    jv.sort();
    jv.dedup();
    let mut table: HashMap<String, Vec<&Tuple>> = HashMap::new();
    for r in right {
        if let Some(k) = r.key(&jv) {
            table.entry(k).or_default().push(r);
        }
    }
    let mut out = Vec::new();
    for l in left {
        if let Some(k) = l.key(&jv) {
            if let Some(bucket) = table.get(&k) {
                for r in bucket {
                    out.push(l.merge(r));
                }
            }
        }
    }
    out
}

/// Drives a [`StreamJoin`] over two fully-known inputs by interleaving their arrivals
/// according to `schedule` (each `true` takes the next left tuple, each `false` the next
/// right), returning the full result. Whatever the schedule does not cover is drained at
/// the end. Used by tests to exercise arbitrary stream interleavings deterministically.
pub fn run_streaming(
    left: &[Tuple],
    right: &[Tuple],
    join_vars: &[Var],
    opts: StreamJoinOptions,
    schedule: &[bool],
) -> Vec<Tuple> {
    let mut join = StreamJoin::new(join_vars.iter().cloned(), opts);
    let mut li = 0usize;
    let mut ri = 0usize;
    let mut out = Vec::new();
    for &take_left in schedule {
        if take_left && li < left.len() {
            out.extend(join.push_left(left[li].clone()));
            li += 1;
        } else if !take_left && ri < right.len() {
            out.extend(join.push_right(right[ri].clone()));
            ri += 1;
        }
    }
    // Drain whatever the schedule did not cover (schedule is a hint, not a guarantee).
    while li < left.len() {
        out.extend(join.push_left(left[li].clone()));
        li += 1;
    }
    while ri < right.len() {
        out.extend(join.push_right(right[ri].clone()));
        ri += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(pairs: &[(&str, &str)]) -> Tuple {
        Tuple::new(pairs.iter().map(|(v, val)| (Var::new(*v), val.to_string())))
    }

    /// Multiset equality: same elements with the same multiplicities, order-insensitive.
    fn multiset_eq(a: &[Tuple], b: &[Tuple]) -> bool {
        if a.len() != b.len() {
            return false;
        }
        let mut a: Vec<String> = a.iter().map(|x| x.encode()).collect();
        let mut b: Vec<String> = b.iter().map(|x| x.encode()).collect();
        a.sort();
        b.sort();
        a == b
    }

    fn jv(names: &[&str]) -> Vec<Var> {
        names.iter().map(|n| Var::new(*n)).collect()
    }

    // ---- The blocking oracle itself, on a known case.
    #[test]
    fn blocking_oracle_basic() {
        let left = [t(&[("s", "a"), ("o", "1")]), t(&[("s", "b"), ("o", "2")])];
        let right = [t(&[("s", "a"), ("n", "x")]), t(&[("s", "a"), ("n", "y")])];
        let out = blocking_hash_join(&left, &right, &jv(&["s"]));
        // s=a left row joins both s=a right rows; s=b joins nothing.
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|r| r.get(&Var::new("s")) == Some("a")));
    }

    // ---- Correctness invariant: streamed == blocking, several interleavings.
    #[test]
    fn streamed_equals_blocking_interleavings() {
        let left: Vec<Tuple> = (0..20)
            .map(|i| t(&[("s", &format!("k{}", i % 5)), ("o", &format!("lo{}", i))]))
            .collect();
        let right: Vec<Tuple> = (0..20)
            .map(|i| t(&[("s", &format!("k{}", i % 5)), ("n", &format!("rn{}", i))]))
            .collect();
        let oracle = blocking_hash_join(&left, &right, &jv(&["s"]));
        // Several deterministic schedules: all-left-first, all-right-first, alternating,
        // and a skewed bursty one.
        let schedules: Vec<Vec<bool>> = vec![
            vec![true; 40],
            vec![false; 40],
            (0..40).map(|i| i % 2 == 0).collect(),
            (0..40).map(|i| i % 3 != 0).collect(),
        ];
        for sched in &schedules {
            let got = run_streaming(
                &left,
                &right,
                &jv(&["s"]),
                StreamJoinOptions::default(),
                sched,
            );
            assert!(
                multiset_eq(&got, &oracle),
                "streamed result must equal blocking join for schedule"
            );
        }
    }

    // ---- Spill correctness: a tiny budget (forces heavy spilling) == no-spill result.
    #[test]
    fn spill_path_equals_no_spill_memory_store() {
        let left: Vec<Tuple> = (0..30)
            .map(|i| t(&[("s", &format!("k{}", i % 4)), ("o", &format!("lo{}", i))]))
            .collect();
        let right: Vec<Tuple> = (0..30)
            .map(|i| t(&[("s", &format!("k{}", i % 4)), ("n", &format!("rn{}", i))]))
            .collect();
        let oracle = blocking_hash_join(&left, &right, &jv(&["s"]));
        let sched: Vec<bool> = (0..60).map(|i| i % 2 == 0).collect();
        // Budget of 2 tuples forces almost everything to spill.
        let tiny = StreamJoinOptions {
            mem_budget_tuples: 2,
            spill_store: SpillStore::Memory,
        };
        let got = run_streaming(&left, &right, &jv(&["s"]), tiny, &sched);
        assert!(
            multiset_eq(&got, &oracle),
            "spill (memory store) == blocking"
        );
    }

    // ---- Spill correctness with the real temp-FILE backing store.
    #[test]
    fn spill_path_equals_no_spill_file_store() {
        let left: Vec<Tuple> = (0..30)
            .map(|i| t(&[("s", &format!("k{}", i % 4)), ("o", &format!("lo{}", i))]))
            .collect();
        let right: Vec<Tuple> = (0..30)
            .map(|i| t(&[("s", &format!("k{}", i % 4)), ("n", &format!("rn{}", i))]))
            .collect();
        let oracle = blocking_hash_join(&left, &right, &jv(&["s"]));
        let sched: Vec<bool> = (0..60).map(|i| i % 3 != 0).collect();
        let tiny = StreamJoinOptions {
            mem_budget_tuples: 3,
            spill_store: SpillStore::TempFile,
        };
        let mut join = StreamJoin::new(jv(&["s"]), tiny);
        let mut out = Vec::new();
        let (mut li, mut ri) = (0usize, 0usize);
        for &take_left in &sched {
            if take_left && li < left.len() {
                out.extend(join.push_left(left[li].clone()));
                li += 1;
            } else if !take_left && ri < right.len() {
                out.extend(join.push_right(right[ri].clone()));
                ri += 1;
            }
        }
        while li < left.len() {
            out.extend(join.push_left(left[li].clone()));
            li += 1;
        }
        while ri < right.len() {
            out.extend(join.push_right(right[ri].clone()));
            ri += 1;
        }
        assert!(join.spilled(), "tiny budget must trigger the spill path");
        assert!(multiset_eq(&out, &oracle), "spill (temp file) == blocking");
    }

    // ---- Non-blocking behaviour: a match is emitted BEFORE either input is exhausted.
    #[test]
    fn emits_before_inputs_exhausted() {
        let mut join = StreamJoin::new(jv(&["s"]), StreamJoinOptions::default());
        // Feed one matching tuple per side, with MORE tuples still to come on both sides.
        let r0 = join.push_left(t(&[("s", "a"), ("o", "1")]));
        assert!(r0.is_empty(), "no right tuple yet ⇒ nothing emitted");
        let r1 = join.push_right(t(&[("s", "a"), ("n", "x")]));
        assert_eq!(
            r1.len(),
            1,
            "match emitted as soon as both sides have key a"
        );
        // Crucially we have NOT exhausted either input — more arrive after the emission:
        let r2 = join.push_left(t(&[("s", "b"), ("o", "2")]));
        let r3 = join.push_right(t(&[("s", "b"), ("n", "y")]));
        assert!(r2.is_empty());
        assert_eq!(r3.len(), 1);
        // The first emission happened at r1, before these later arrivals — non-blocking.
    }

    // ---- Edge: empty inputs.
    #[test]
    fn empty_inputs() {
        let empty: Vec<Tuple> = Vec::new();
        let some = [t(&[("s", "a"), ("o", "1")])];
        assert!(blocking_hash_join(&empty, &some, &jv(&["s"])).is_empty());
        assert!(blocking_hash_join(&some, &empty, &jv(&["s"])).is_empty());
        let got = run_streaming(
            &empty,
            &empty,
            &jv(&["s"]),
            StreamJoinOptions::default(),
            &[],
        );
        assert!(got.is_empty());
    }

    // ---- Edge: one-sided (no matching keys ⇒ empty, both algos agree).
    #[test]
    fn one_sided_no_match() {
        let left = [t(&[("s", "a"), ("o", "1")]), t(&[("s", "b"), ("o", "2")])];
        let right = [t(&[("s", "c"), ("n", "x")])];
        let oracle = blocking_hash_join(&left, &right, &jv(&["s"]));
        assert!(oracle.is_empty());
        let sched: Vec<bool> = vec![true, false, true, false];
        let got = run_streaming(
            &left,
            &right,
            &jv(&["s"]),
            StreamJoinOptions::default(),
            &sched,
        );
        assert!(multiset_eq(&got, &oracle));
    }

    // ---- Edge: duplicate join keys produce the full cross product per key (multiset).
    #[test]
    fn duplicate_keys_cross_product() {
        let left = [
            t(&[("s", "a"), ("o", "1")]),
            t(&[("s", "a"), ("o", "2")]),
            t(&[("s", "a"), ("o", "3")]),
        ];
        let right = [t(&[("s", "a"), ("n", "x")]), t(&[("s", "a"), ("n", "y")])];
        let oracle = blocking_hash_join(&left, &right, &jv(&["s"]));
        assert_eq!(oracle.len(), 6, "3×2 cross product on key a");
        // Streamed, with spill, must reproduce all 6 exactly once each.
        let sched: Vec<bool> = vec![true, false, true, false, true, false, false, true];
        let tiny = StreamJoinOptions {
            mem_budget_tuples: 1,
            spill_store: SpillStore::Memory,
        };
        let got = run_streaming(&left, &right, &jv(&["s"]), tiny, &sched);
        assert!(
            multiset_eq(&got, &oracle),
            "duplicate-key cross product preserved under spill"
        );
    }

    // ---- Multi-variable join key.
    #[test]
    fn multi_var_join_key() {
        let left = [
            t(&[("s", "a"), ("p", "1"), ("o", "x")]),
            t(&[("s", "a"), ("p", "2"), ("o", "y")]),
        ];
        let right = [
            t(&[("s", "a"), ("p", "1"), ("n", "hit")]),
            t(&[("s", "a"), ("p", "9"), ("n", "miss")]),
        ];
        let oracle = blocking_hash_join(&left, &right, &jv(&["s", "p"]));
        assert_eq!(oracle.len(), 1, "only (a,1) matches on both s and p");
        let got = run_streaming(
            &left,
            &right,
            &jv(&["s", "p"]),
            StreamJoinOptions::default(),
            &[true, true, false, false],
        );
        assert!(multiset_eq(&got, &oracle));
    }

    // ---- Tuple with a missing join var is dropped (cannot key), matching blocking.
    #[test]
    fn unbound_join_var_dropped() {
        let left = [t(&[("o", "1")])]; // no `s` binding
        let right = [t(&[("s", "a"), ("n", "x")])];
        let oracle = blocking_hash_join(&left, &right, &jv(&["s"]));
        assert!(oracle.is_empty());
        let got = run_streaming(
            &left,
            &right,
            &jv(&["s"]),
            StreamJoinOptions::default(),
            &[true, false],
        );
        assert!(multiset_eq(&got, &oracle));
    }

    // ============================================================================
    // sq-bif — correctness suite: the public `Tuple` value type's documented
    // invariants (canonical variable-sorted bindings, duplicate-variable-keeps-first,
    // `get` lookup incl. the missing-var `None`), the `encode`/`decode` spill round-trip
    // (incl. the previously-uncovered empty-tuple line), and the load-bearing arrival-order
    // independence of `merge_ordered`. These drive the REAL operator code and each asserts a
    // specific documented behaviour a regression would break — not a "does not panic".
    // ============================================================================

    // ---- `Tuple::bindings` is in canonical *variable-sorted* order regardless of the input
    //      order. This is load-bearing: equal tuples must be byte-identical so spill/compare
    //      and the `PartialEq` used by the multiset oracle agree. Two tuples built from the
    //      same pairs in different orders MUST be equal.
    #[test]
    fn tuple_bindings_are_canonical_and_order_independent() {
        // Built object-first then subject-first — the canonical form sorts by variable name.
        let a = Tuple::new([
            (Var::new("z"), "26".to_string()),
            (Var::new("a"), "1".to_string()),
            (Var::new("m"), "13".to_string()),
        ]);
        // Bindings come back ascending by variable name (a, m, z), values carried along.
        assert_eq!(
            a.bindings(),
            &[
                (Var::new("a"), "1".to_string()),
                (Var::new("m"), "13".to_string()),
                (Var::new("z"), "26".to_string()),
            ]
        );
        // A different *insertion* order yields an EQUAL tuple (canonicalisation is total).
        let b = Tuple::new([
            (Var::new("m"), "13".to_string()),
            (Var::new("z"), "26".to_string()),
            (Var::new("a"), "1".to_string()),
        ]);
        assert_eq!(
            a, b,
            "tuple equality is insertion-order-independent (canonical)"
        );
        // …and they encode to byte-identical lines (the property the spill path relies on).
        assert_eq!(a.encode(), b.encode());
    }

    // ---- `Tuple::new` keeps the FIRST value on a duplicate variable (documented). A regression
    //      to last-wins would silently change join results, so assert the exact retained value.
    #[test]
    fn tuple_new_duplicate_variable_keeps_first_value() {
        let dup = Tuple::new([
            (Var::new("s"), "first".to_string()),
            (Var::new("s"), "second".to_string()), // dropped — first wins.
            (Var::new("o"), "x".to_string()),
        ]);
        assert_eq!(dup.get(&Var::new("s")), Some("first"));
        // Exactly one binding survives for the duplicated variable (plus the unique `o`).
        assert_eq!(dup.bindings().len(), 2);
    }

    // ---- `Tuple::get` returns the bound value for a present variable and `None` for an absent
    //      one (the key/merge logic depends on this distinction).
    #[test]
    fn tuple_get_present_and_absent() {
        let tup = t(&[("s", "alice"), ("o", "bob")]);
        assert_eq!(tup.get(&Var::new("s")), Some("alice"));
        assert_eq!(tup.get(&Var::new("o")), Some("bob"));
        assert_eq!(
            tup.get(&Var::new("missing")),
            None,
            "an unbound variable looks up to None"
        );
    }

    // ---- `encode` → `decode` is a round-trip for a non-trivial multi-binding tuple (the spill
    //      backing store serialises with `encode` and rehydrates with `decode`, so a regression
    //      here would corrupt spilled partitions and break the spill correctness invariant).
    #[test]
    fn tuple_encode_decode_round_trip() {
        let original = t(&[("s", "http://ex/a"), ("p", "1"), ("o", "literal value")]);
        let line = original.encode();
        let back = Tuple::decode(&line);
        assert_eq!(back, original, "encode∘decode is the identity on a tuple");
        // The decoded tuple is itself canonical (decode re-canonicalises defensively).
        assert_eq!(back.bindings(), original.bindings());
    }

    // ---- `decode("")` yields the EMPTY tuple (previously-uncovered empty-line branch). An empty
    //      tuple — zero bindings — is what a key-less / projected-away row encodes to, and it must
    //      round-trip through the spill store rather than mis-parsing into a phantom binding.
    #[test]
    fn tuple_decode_empty_line_is_empty_tuple() {
        let empty = Tuple::new(std::iter::empty::<(Var, String)>());
        assert!(empty.bindings().is_empty());
        assert_eq!(
            empty.encode(),
            "",
            "an empty tuple encodes to the empty string"
        );
        let decoded = Tuple::decode("");
        assert!(
            decoded.bindings().is_empty(),
            "decoding the empty line yields a binding-less tuple, not a phantom one"
        );
        assert_eq!(decoded, empty);
    }

    // ---- Arrival-order independence (the documented `merge_ordered` invariant): the SAME pair
    //      of matching tuples must produce the SAME merged binding set whether the left or the
    //      right arrives first. The discriminating case is a NON-join variable bound by BOTH sides
    //      to DIFFERENT values: `merge` keeps the LEFT side's value on a conflict, so the canonical
    //      `merge_ordered` must pick the left value regardless of which side arrived last. A
    //      regression that merged in arrival order would still pass the multiset COUNT tests but
    //      flip this shared variable's value when the right arrives last — this pins it.
    #[test]
    fn merge_result_is_independent_of_arrival_order() {
        // Join on `s`; both sides ALSO bind `c` (a non-join var) to different values. The
        // canonical merge keeps the LEFT side's `c = leftc`.
        let l = t(&[("s", "k"), ("c", "leftc"), ("o", "leftval")]);
        let r = t(&[("s", "k"), ("c", "rightc"), ("n", "rightval")]);

        // Schedule 1: left arrives first, then right completes the match.
        let mut j1 = StreamJoin::new(jv(&["s"]), StreamJoinOptions::default());
        assert!(j1.push_left(l.clone()).is_empty());
        let out_lr = j1.push_right(r.clone());

        // Schedule 2: right arrives first, then left completes the match.
        let mut j2 = StreamJoin::new(jv(&["s"]), StreamJoinOptions::default());
        assert!(j2.push_right(r.clone()).is_empty());
        let out_rl = j2.push_left(l.clone());

        assert_eq!(out_lr.len(), 1);
        assert_eq!(out_rl.len(), 1);
        // The single merged row is byte-identical across the two arrival orders.
        let merged = &out_lr[0];
        assert_eq!(
            merged, &out_rl[0],
            "merge result is independent of which side arrives last"
        );
        assert_eq!(merged.get(&Var::new("s")), Some("k"));
        assert_eq!(merged.get(&Var::new("o")), Some("leftval"));
        assert_eq!(merged.get(&Var::new("n")), Some("rightval"));
        // The discriminating assertion: the LEFT side's value of the conflicting non-join var
        // wins in BOTH arrival orders (an arrival-order merge would yield `rightc` for j2).
        assert_eq!(
            merged.get(&Var::new("c")),
            Some("leftc"),
            "the left side's value wins a non-join-variable conflict, regardless of arrival order"
        );
        // It also equals the blocking oracle's merge of the same pair (left.merge(right)).
        let oracle = blocking_hash_join(&[l], &[r], &jv(&["s"]));
        assert_eq!(oracle.len(), 1);
        assert_eq!(merged, &oracle[0]);
    }
}
