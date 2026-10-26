//! Window materialisation strategies (the R2R step's graph production):
//! how each closed window becomes the [`sparq_core::Graph`] the engine
//! evaluates. See [`EvalMode`] for the four strategies and the crate README
//! for the measured throughput of each.
//!
//! (sq-2n1q3.4) The `Delta`/`Snapshot` consecutive-window diff (the
//! ISTREAM/DSTREAM-shaped slide) runs on the shared eval substrate: each window
//! is interned into a persistent diff dictionary and diffed as `Id = u32`
//! id-rows via `sparq_substrate::join::delta::DeltaTable` — see `WindowDiff`
//! for the semi-naive shape and the behaviour-neutrality argument.

use oxrdf::Term;
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::{is_inline, Dict, Id};
use sparq_core::Graph;
use sparq_substrate::join::delta::DeltaTable;
use sparq_substrate::join::{JoinKeys, NoBudget};
use sparq_substrate::rows::Row;

use crate::window::{Window, WindowSpec, WindowedStream};

/// How each closed window's graph is materialised for evaluation.
///
/// All four modes produce IDENTICAL query results (pinned by tests); they
/// differ only in how much per-window work is redone. Numbers below are the
/// crate benchmark (`examples/throughput.rs`, see README).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EvalMode {
    /// v1 baseline: every closed window interns its terms into a FRESH
    /// dictionary and builds fresh indexes. Honest, allocation-heavy; kept as
    /// the benchmark baseline and for streams with unbounded vocabulary
    /// (nothing persists between windows, so memory is bounded by one window).
    Rebuild,
    /// ONE dictionary for the continuous query's lifetime: every term is
    /// interned once at push time and each closed window builds its indexes
    /// from already-interned `[Id; 3]`s (`Graph::from_parts`), removing term
    /// hashing/allocation from the window loop. The benchmark winner across
    /// every scenario; the default.
    ///
    /// The dictionary no longer grows monotonically with the
    /// stream's vocabulary. Liveness is exact: a term is reachable iff some
    /// live window (a buffered/open window, the count ring, or a closed window
    /// not yet taken) still references it; terms that have aged out of every
    /// window are unreachable, so their dictionary entries are reclaimable. A
    /// bounded amount of post-eviction churn triggers a COMPACTION pass that
    /// rebuilds the dictionary from only the still-live terms and atomically
    /// remaps the live window indexes onto the fresh dense ids (see
    /// `WindowEval::compact_dict`). Bound: the dictionary stays O(distinct
    /// terms across the live windows), not O(distinct terms over all time. So a
    /// long-running query over an unbounded vocabulary holds memory proportional
    /// to its window content, not its history.
    #[default]
    PersistentDict,
    /// ONE live graph maintained by `Graph::apply_delta(inserts, deletes)` per
    /// slide — the set-semantic diff between consecutive windows, computed on
    /// the shared eval substrate (sq-2n1q3.4: each window is interned
    /// into a persistent diff dictionary and diffed as `Id = u32` id-rows
    /// against the previous window's persisted
    /// `sparq_substrate::join::delta::DeltaTable`, the semi-naive Δ-vs-full
    /// probe shape; see `WindowDiff`) — compacted when the pending overlay
    /// outgrows the live window. Measured SLOWER than `PersistentDict` in every
    /// benchmark scenario (`Graph::apply_delta` still does term-level work —
    /// interning inserts, `id_of` lookups for deletes — and overlay rows are
    /// re-sorted per scan); kept as an option because its per-slide cost is
    /// O(changes), so it can win when windows are huge and the engine
    /// evaluation itself is cheap relative to an index rebuild. Like
    /// `PersistentDict`, the live graph's dictionary grows monotonically (as
    /// does the separate diff dictionary, which tracks the same all-time
    /// vocabulary).
    Delta,
    /// (sq-j39) The TRUE overlay-snapshot mode: identical per-slide
    /// maintenance to [`Delta`](Self::Delta) — ONE live mutable graph evolved by
    /// `Graph::apply_delta` over the set-semantic diff between consecutive
    /// windows — but each closed window is evaluated over a cheap, IMMUTABLE
    /// [`Graph::snapshot`](sparq_core::Graph::snapshot) of that live graph rather
    /// than a borrow into it.
    ///
    /// The snapshot is `O(pending overlay)`, never `O(triples)`: the live
    /// graph's compacted base (the six permutation indexes + the dictionary
    /// arena) is `Arc`-shared into the snapshot, so only the small per-slide
    /// delta is copied. The result is a logically-independent, point-in-time,
    /// `Send + Sync` view of the window — unaffected by the next slide's
    /// mutation — so unlike [`Delta`](Self::Delta) (whose `&Graph` borrow ties
    /// the engine's read to the live graph and forbids retaining it) the engine
    /// could hold or publish this graph across windows. That is what makes it
    /// the production serving shape: keep one mutable master, hand out one cheap
    /// immutable snapshot per closed window.
    ///
    /// Results are IDENTICAL to every other mode (pinned by the equivalence
    /// tests). It is a thin wrapper over `Delta`'s maintenance, so its per-slide
    /// cost is the same `O(changes)` plus one O(overlay) snapshot per window;
    /// the snapshot itself adds no index rebuild.
    Snapshot,
}

/// Compact the delta-mode overlay when the cumulative churn since the last
/// compaction exceeds the live graph size (clamped: tiny windows shouldn't
/// compact on every slide).
const MIN_COMPACT_CHURN: usize = 256;

/// Persistent-dictionary compaction trigger. After each batch of
/// closed windows, the dictionary holds `live + dead` distinct terms, where
/// `live` are still referenced by some window and `dead` have aged out. A
/// compaction rebuild is O(live + dead) (it walks the live payloads and
/// reconstructs the survivors), so triggering it on EVERY slide would add a
/// full-dictionary pass per window — defeating the point of the persistent
/// dictionary. Instead we let the dead set grow and compact only when it is
/// both ABSOLUTELY large enough to matter and a SUBSTANTIAL fraction of the
/// dictionary, so the amortised compaction cost per interned term is O(1) and
/// the steady-state dictionary size is bounded by `live · (1 + DEAD_RATIO)`.
///
/// `MIN_DEAD` keeps small/low-churn streams from ever compacting (the rebuild
/// would cost more than the few bytes it reclaims); `DEAD_RATIO` makes the
/// reclaimed-fraction worthwhile when we do pay it.
const MIN_DEAD_TERMS: usize = 1024;
/// Compact once the estimated dead terms reach this fraction of the live set.
const DEAD_RATIO_NUM: usize = 1;
const DEAD_RATIO_DEN: usize = 2;

// The PersistentDict variant holds the live `Dict` inline — that IS the
// persistent-dictionary mode (the dict lives across windows). Boxing it to equalise
// variant sizes would add a hot-path indirection per tick for no real benefit: there
// is exactly ONE Materializer per ContinuousQuery (never a large collection), so the
// "every instance padded to the largest variant" cost the lint guards against does not
// apply here. (Only trips under workspace feature-unification, not isolated-crate clippy.)
#[allow(clippy::large_enum_variant)]
enum Materializer {
    Rebuild {
        window: WindowedStream<[Term; 3]>,
    },
    PersistentDict {
        /// `None` only transiently while the dictionary is lent to the
        /// per-window graph (`Graph::from_parts` takes it by value; it is
        /// moved back out with `Graph::into_dict` after evaluation).
        dict: Option<Dict>,
        window: WindowedStream<[Id; 3]>,
        /// `dict.len()` (distinct non-inline terms) at the last
        /// compaction. The cheap compaction trigger: once `dict.len()` has
        /// grown by at least [`MIN_DEAD_TERMS`] beyond this, enough terms have
        /// been interned since the last rebuild that a liveness scan is worth
        /// running (and `compact_dict` then resets this to the post-rebuild
        /// length). Scanning only every `MIN_DEAD_TERMS` interns keeps the
        /// amortised compaction cost per interned term O(1).
        dict_len_at_compact: usize,
    },
    Delta {
        window: WindowedStream<[Term; 3]>,
        /// Boxed: a `Graph` (dictionary + indexes) is much larger than the
        /// other variants' state.
        graph: Box<Graph>,
        /// (sq-2n1q3.4) The persistent id-level diff state — the diff
        /// dictionary, the previous window's deduplicated id-rows + terms, and
        /// the substrate `DeltaTable` build side (the diff base for the next
        /// slide).
        diff: WindowDiff,
        /// Inserts + deletes applied since the last compaction.
        churn: usize,
    },
    /// (sq-j39) Same live-graph maintenance as `Delta`; differs only
    /// in that each closed window is evaluated over `graph.snapshot()` (an O(1)
    /// immutable point-in-time view) rather than `&*graph`.
    Snapshot {
        window: WindowedStream<[Term; 3]>,
        graph: Box<Graph>,
        diff: WindowDiff,
        churn: usize,
    },
}

/// One windowed stream + one [`EvalMode`]: pushes elements, and surfaces every
/// closed window as a `(start, end, &Graph)` callback.
pub(crate) struct WindowEval {
    mode: EvalMode,
    m: Materializer,
}

impl WindowEval {
    pub fn new(spec: WindowSpec, mode: EvalMode) -> Self {
        let m = match mode {
            EvalMode::Rebuild => Materializer::Rebuild { window: WindowedStream::empty(spec) },
            EvalMode::PersistentDict => Materializer::PersistentDict {
                dict: Some(Dict::new()),
                window: WindowedStream::empty(spec),
                dict_len_at_compact: 0,
            },
            EvalMode::Delta => Materializer::Delta {
                window: WindowedStream::empty(spec),
                graph: Box::new(Graph::from_parts(Dict::new(), Vec::new())),
                diff: WindowDiff::new(),
                churn: 0,
            },
            EvalMode::Snapshot => Materializer::Snapshot {
                window: WindowedStream::empty(spec),
                graph: Box::new(Graph::from_parts(Dict::new(), Vec::new())),
                diff: WindowDiff::new(),
                churn: 0,
            },
        };
        WindowEval { mode, m }
    }

    pub fn mode(&self) -> EvalMode {
        self.mode
    }

    pub fn spec(&self) -> WindowSpec {
        match &self.m {
            Materializer::Rebuild { window }
            | Materializer::Delta { window, .. }
            | Materializer::Snapshot { window, .. } => window.spec(),
            Materializer::PersistentDict { window, .. } => window.spec(),
        }
    }

    pub fn late_dropped(&self) -> u64 {
        match &self.m {
            Materializer::Rebuild { window }
            | Materializer::Delta { window, .. }
            | Materializer::Snapshot { window, .. } => window.late_dropped(),
            Materializer::PersistentDict { window, .. } => window.late_dropped(),
        }
    }

    /// The number of distinct non-inline terms currently held by the
    /// persistent dictionary, or `None` for the modes without a lifetime
    /// dictionary ([`EvalMode::Rebuild`] builds and drops one per window;
    /// [`EvalMode::Delta`] keeps the live graph's dictionary, not exposed here).
    /// An observability hook: this is the quantity that compaction bounds, so an
    /// embedder running a long continuous query can watch it stay bounded
    /// instead of growing with the all-time vocabulary.
    pub fn dict_len(&self) -> Option<usize> {
        match &self.m {
            Materializer::PersistentDict { dict, .. } => {
                Some(dict.as_ref().expect("dict restored between evaluations").len())
            }
            _ => None,
        }
    }

    pub fn push(&mut self, triple: [Term; 3], ts: u64) {
        match &mut self.m {
            Materializer::Rebuild { window }
            | Materializer::Delta { window, .. }
            | Materializer::Snapshot { window, .. } => window.push(triple, ts),
            Materializer::PersistentDict { dict, window, .. } => {
                let d = dict.as_mut().expect("dict is always restored after evaluation");
                let ids = [d.intern(&triple[0]), d.intern(&triple[1]), d.intern(&triple[2])];
                window.push(ids, ts);
            }
        }
    }

    /// Materialises and evaluates every window that closed since the last
    /// call (all remaining windows when `flush` is true), oldest first. On an
    /// `Err` from `f`, remaining closed windows are dropped.
    pub fn eval_closed(
        &mut self,
        flush: bool,
        f: &mut impl FnMut(u64, u64, &Graph) -> Result<(), String>,
    ) -> Result<(), String> {
        match &mut self.m {
            Materializer::Rebuild { window } => {
                for w in take(window, flush) {
                    let graph = materialize(&w);
                    f(w.start, w.end, &graph)?;
                }
            }
            Materializer::PersistentDict { dict, window, dict_len_at_compact } => {
                for w in take(window, flush) {
                    let ids: Vec<[Id; 3]> = w.triples.iter().map(|t| t.triple).collect();
                    // Lend the persistent dictionary to the per-window graph;
                    // `Graph::from_parts` dedups (RDF set semantics) and builds
                    // the indexes from the already-interned ids — no term
                    // hashing or allocation here.
                    let d = dict.take().expect("dict is always restored after evaluation");
                    let graph = Graph::from_parts(d, ids);
                    let r = f(w.start, w.end, &graph);
                    *dict = Some(graph.into_dict());
                    r?;
                }
                // Reclaim the dictionary entries of terms that have
                // aged out of every live window. Safe HERE: every closed window
                // for this call has been evaluated and dropped (the `take`
                // vector is gone), no `Graph` borrows the dictionary, and the
                // only remaining references are the live window payloads —
                // which `compact_dict` remaps atomically.
                let len = dict.as_ref().expect("dict restored above").len();
                if len >= dict_len_at_compact.saturating_add(MIN_DEAD_TERMS) {
                    Self::compact_dict(dict, window, dict_len_at_compact);
                }
            }
            Materializer::Delta { window, graph, diff, churn } => {
                for w in take(window, flush) {
                    apply_window_delta(&w, graph, diff, churn)?;
                    // Delta hands the engine a direct borrow into the live graph:
                    // it is read, then the next slide mutates it in place.
                    f(w.start, w.end, graph)?;
                }
            }
            Materializer::Snapshot { window, graph, diff, churn } => {
                for w in take(window, flush) {
                    apply_window_delta(&w, graph, diff, churn)?;
                    // (sq-j39) The true overlay-snapshot step: evaluate
                    // this window over a cheap O(overlay) IMMUTABLE point-in-time
                    // view of the live graph, not a borrow into it. The snapshot
                    // `Arc`-shares the compacted base and copies only the small
                    // pending delta, so it is logically independent of the next
                    // slide's `apply_delta` — the engine (or the caller, through
                    // the callback) could retain or publish it across windows. It
                    // derefs to `&Graph`, so `f`'s `&Graph` contract is unchanged.
                    let snap = graph.snapshot();
                    f(w.start, w.end, &snap)?;
                }
            }
        }
        Ok(())
    }

    /// Persistent-dictionary compaction: rebuild the lifetime
    /// dictionary from ONLY the terms still referenced by a live window, and
    /// remap the live window payloads onto the fresh dense ids — bounding the
    /// dictionary at the live vocabulary instead of the all-time vocabulary.
    ///
    /// Correctness contract:
    /// * **No dangling ids.** Every live payload is remapped through `old → new`
    ///   in [`WindowedStream::remap_live_payloads`]; the remap covers exactly
    ///   the ids gathered by [`WindowedStream::for_each_live_payload`], so after
    ///   the swap no live id points into the freed dictionary.
    /// * **A still-referenced term stays resolvable.** Every live id is
    ///   re-interned (its term reconstructed from the old dict, interned into
    ///   the new one), so its value survives; only ids referenced by NO live
    ///   window are dropped.
    /// * **Results are unchanged.** The remap is a bijection on live ids, so
    ///   per-window triple sets, membership, ordering and counts are identical;
    ///   only the integer encoding changes. (Inline-integer ids carry their
    ///   value in the id itself and are never dictionary entries, so they map to
    ///   themselves and need no rebuild.)
    /// * **Atomic.** The new dict + remap are built fully before any live
    ///   payload is rewritten; the rebuild cannot fail (it only reconstructs and
    ///   re-interns terms already present), so there is no partial state.
    ///
    /// Skips the rebuild when nothing would be reclaimed (dead set below
    /// [`MIN_DEAD_TERMS`] or below the [`DEAD_RATIO_NUM`]/[`DEAD_RATIO_DEN`]
    /// fraction of the live set), but always advances `dict_len_at_compact` so
    /// the next liveness scan is again `MIN_DEAD_TERMS` interns away.
    fn compact_dict(
        dict: &mut Option<Dict>,
        window: &mut WindowedStream<[Id; 3]>,
        dict_len_at_compact: &mut usize,
    ) {
        let old = dict.as_ref().expect("dict present when compacting");

        // Gather the DISTINCT live (non-inline) ids referenced by any window.
        let mut live: FxHashSet<Id> = FxHashSet::default();
        window.for_each_live_payload(|ids| {
            for &id in ids {
                if !is_inline(id) {
                    live.insert(id);
                }
            }
        });

        // Decide whether reclaiming is worth a full rebuild. `dead` is the
        // number of dictionary entries no live window references any more.
        let total = old.len();
        let dead = total.saturating_sub(live.len());
        let worth_it = dead >= MIN_DEAD_TERMS
            && dead * DEAD_RATIO_DEN >= live.len().max(1) * DEAD_RATIO_NUM;
        if !worth_it {
            // Defer: record the current size so the next scan is MIN_DEAD_TERMS
            // interns away (otherwise a once-large-but-now-mostly-live dict
            // would re-scan on every batch).
            *dict_len_at_compact = total;
            return;
        }

        // Rebuild: a fresh dictionary holding only the survivors, plus the
        // old→new id remap. Re-intern via the reconstructed term so the new
        // dict's prefix/datatype tables and value caches are rebuilt correctly.
        let mut fresh = Dict::with_capacity(live.len());
        let mut remap: FxHashMap<Id, Id> = FxHashMap::with_capacity_and_hasher(live.len(), Default::default());
        for &id in &live {
            let new_id = fresh.intern(&old.term(id));
            remap.insert(id, new_id);
        }

        // Atomically swing every live payload onto the fresh ids. Inline ids
        // pass through unchanged (they are not in `remap`); every non-inline
        // live id is present in `remap` by construction.
        window.remap_live_payloads(|ids| {
            [
                remap_id(ids[0], &remap),
                remap_id(ids[1], &remap),
                remap_id(ids[2], &remap),
            ]
        });

        *dict = Some(fresh);
        *dict_len_at_compact = live.len();
    }
}

/// Maps one id through the compaction remap: inline-integer ids (and only
/// those) are absent from the remap and carry their value in the id, so they
/// map to themselves; every live dictionary id is present by construction.
#[inline]
fn remap_id(id: Id, remap: &FxHashMap<Id, Id>) -> Id {
    if is_inline(id) {
        id
    } else {
        *remap.get(&id).expect("every live non-inline id is in the compaction remap")
    }
}

/// (sq-2n1q3.4) The persistent id-level window-diff state shared by
/// `EvalMode::Delta` and `EvalMode::Snapshot` — the consecutive-window set-diff
/// (the ISTREAM/DSTREAM shape) computed on the shared eval substrate
/// (`sparq_substrate::join::delta::DeltaTable` over `sparq_substrate::rows::Row`)
/// instead of per-slide term-level hash sets.
///
/// ## Semi-naive shape
///
/// Per slide, the previous window is the persisted BUILD side and the closed
/// window is the probe (Δ) side, keyed on the full triple (all three id
/// columns): a probe miss against `prev_table` is an INSERT, a previous row
/// missing from the window's own freshly-built table is a DELETE, and that
/// fresh table then BECOMES `prev_table` — the previous window is never
/// re-hashed. That carried-over build side is exactly the extendable-table
/// property `DeltaTable` was built for (`research/substrate-remaining-design.md`
/// §3), where the old code rebuilt BOTH term-level hash sets on every slide.
///
/// ## Behaviour-neutrality (the load-bearing argument)
///
/// Diffing id-rows computes EXACTLY the sets the term-level diff computed,
/// because interning is injective on lexically-distinct terms: `Dict::intern`
/// assigns equal ids iff the terms are equal — inline ids exist only for the
/// CANONICAL `xsd:integer` lexical form (`"042"^^xsd:integer` stays a distinct
/// dictionary entry) and stored terms dedup on the exact
/// (value, datatype, lang) tuple — which is `oxrdf::Term` equality. The terms
/// handed to `Graph::apply_delta` are the ORIGINAL pushed terms (kept in
/// `prev_terms`), never a dictionary reconstruction, so the live graph receives
/// the same insert/delete SETS as before the substrate adoption. Pinned by the
/// four-EvalMode cross-check tests and the SRBench expressivity ratchet.
///
/// ## Monomorphic (no dynamic dispatch)
///
/// The probe path carries no trait object: `NoBudget` is a zero-sized generic
/// `Budget`, the emit hook is a monomorphised closure, and the key projection
/// is plain `Id = u32` column data — `scripts/check-no-dyn-dispatch.py` guards
/// this file structurally (the #1303 invariant, consumer side of the seam).
///
/// ## Memory bound
///
/// `dict` grows monotonically with the stream's all-time vocabulary — the SAME
/// documented bound as the live graph's own dictionary in these two modes. It
/// is deliberately SEPARATE from the live graph's dict so the graph's
/// id-assignment order (and thus its observable behaviour) is untouched by the
/// substrate adoption.
struct WindowDiff {
    /// Persistent term→id dictionary for the diff (see the memory-bound note
    /// above for why it is separate from the live graph's dictionary).
    dict: Dict,
    /// The previous window's DISTINCT triples as id-rows, in first-appearance
    /// order (parallel to `prev_terms`; the arena order of `prev_table`).
    prev_rows: Vec<Row>,
    /// The original `Term` form of each `prev_rows` entry — deletes hand
    /// `Graph::apply_delta` these exact terms.
    prev_terms: Vec<[Term; 3]>,
    /// The substrate build-side table over `prev_rows`, persisted across
    /// slides.
    prev_table: DeltaTable,
    /// The full-triple key projection: all three id columns on both sides.
    keys: JoinKeys,
}

impl WindowDiff {
    fn new() -> Self {
        WindowDiff {
            dict: Dict::new(),
            prev_rows: Vec::new(),
            prev_terms: Vec::new(),
            prev_table: DeltaTable::new(),
            keys: JoinKeys { key_cols: vec![(0, 0), (1, 1), (2, 2)], right_only: Vec::new() },
        }
    }

    /// Whether `row` has a match in `table` under the full-triple key: one hash
    /// probe through the substrate kernel. Distinct rows carry distinct keys
    /// (the key IS the whole row), so the emit hook fires at most once.
    #[inline]
    fn contains(&self, table: &DeltaTable, row: &Row) -> bool {
        let mut hit = false;
        table.probe_emit(std::slice::from_ref(row), &self.keys, &NoBudget, &mut |_, _| {
            hit = true;
        });
        hit
    }
}

/// (sq-j39) One slide of the live-graph maintenance shared by
/// [`EvalMode::Delta`] and [`EvalMode::Snapshot`]: diff the closed window `w`
/// against the previous window under RDF set semantics — a triple present at
/// several timestamps counts once, so inserts/deletes are computed over the
/// DISTINCT triple sets — apply the diff to the live `graph`, advance the diff
/// base, and fold the overlay back into the base once cumulative churn
/// outgrows the live window (overlay rows are re-sorted per scan). After this
/// returns, `graph` holds exactly the distinct triples of `w`; the two modes
/// differ only in WHAT they then hand the engine (a live borrow vs an immutable
/// snapshot).
///
/// (sq-2n1q3.4) The diff itself runs on the shared eval substrate —
/// the semi-naive Δ-vs-full probe shape of
/// `sparq_substrate::join::delta::DeltaTable` over `Id = u32` id-rows; see
/// `WindowDiff` for the exact-equivalence argument.
fn apply_window_delta(
    w: &Window<[Term; 3]>,
    graph: &mut Graph,
    diff: &mut WindowDiff,
    churn: &mut usize,
) -> Result<(), String> {
    // Intern + dedup the closed window into id-rows, probing as we go.
    let mut cur_table = DeltaTable::new();
    let mut cur_rows: Vec<Row> = Vec::with_capacity(w.triples.len());
    let mut cur_terms: Vec<[Term; 3]> = Vec::with_capacity(w.triples.len());
    let mut inserts: Vec<[Term; 3]> = Vec::new();
    for t in &w.triples {
        let row = Row::from_slice(&[
            diff.dict.intern(&t.triple[0]),
            diff.dict.intern(&t.triple[1]),
            diff.dict.intern(&t.triple[2]),
        ]);
        // RDF set semantics: a triple present at several timestamps counts
        // once — the window's OWN build table doubles as the dedup set.
        if diff.contains(&cur_table, &row) {
            continue;
        }
        // ISTREAM half: in the current window but not the previous one.
        if !diff.contains(&diff.prev_table, &row) {
            inserts.push(t.triple.clone());
        }
        cur_table.extend(std::slice::from_ref(&row), &diff.keys);
        cur_rows.push(row);
        cur_terms.push(t.triple.clone());
    }
    // DSTREAM half: in the previous window but not the current one.
    let mut deletes: Vec<[Term; 3]> = Vec::new();
    for (i, row) in diff.prev_rows.iter().enumerate() {
        if !diff.contains(&cur_table, row) {
            deletes.push(diff.prev_terms[i].clone());
        }
    }
    // Advance the diff base: this window's table IS the next slide's build
    // side (the previous window is never re-hashed).
    diff.prev_rows = cur_rows;
    diff.prev_terms = cur_terms;
    diff.prev_table = cur_table;
    *churn += inserts.len() + deletes.len();
    graph.apply_delta(&inserts, &deletes)?;
    if *churn >= graph.len().max(MIN_COMPACT_CHURN) {
        graph.compact()?;
        *churn = 0;
    }
    Ok(())
}

fn take<T: Clone>(window: &mut WindowedStream<T>, flush: bool) -> Vec<Window<T>> {
    if flush {
        window.flush()
    } else {
        window.take_closed()
    }
}

/// Materialises one window's content into a fresh dictionary-encoded graph
/// (the [`EvalMode::Rebuild`] baseline). RDF graphs are sets: a triple present
/// at several timestamps in the window appears once.
fn materialize(w: &Window<[Term; 3]>) -> Graph {
    let mut dict = Dict::new();
    let ids: Vec<[Id; 3]> = w
        .triples
        .iter()
        .map(|t| [dict.intern(&t.triple[0]), dict.intern(&t.triple[1]), dict.intern(&t.triple[2])])
        .collect();
    Graph::from_parts(dict, ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::vocab::xsd;
    use oxrdf::{Literal, NamedNode};
    use sparq_core::GraphSnapshot;

    fn triple(s: &str, p: &str, o: &str) -> [Term; 3] {
        [
            NamedNode::new_unchecked(format!("http://ex/{s}")).into(),
            NamedNode::new_unchecked(format!("http://ex/{p}")).into(),
            NamedNode::new_unchecked(format!("http://ex/{o}")).into(),
        ]
    }

    /// (sq-j39) The graph `EvalMode::Snapshot` hands the engine per
    /// closed window is a TRUE point-in-time IMMUTABLE view of the live master,
    /// logically independent of every later slide — that is the whole point of
    /// the mode (vs `Delta`'s live borrow). Test it on the REAL internal graph:
    /// the `eval_closed` callback receives that snapshot as `&Graph`, so calling
    /// `Graph::snapshot()` on it yields an owned, independent copy we move into a
    /// `Vec`. Only AFTER the entire stream has driven the live master through
    /// many `apply_delta` slides (each window has a distinct, growing vocabulary,
    /// crossing the overlay-compaction threshold) do we check each captured
    /// snapshot: it must STILL read exactly its own window's triple count. A view
    /// that aliased the live store would report the FINAL window for every
    /// capture; a `Delta`-style live borrow could not be retained past the
    /// callback at all.
    #[test]
    fn snapshot_mode_yields_point_in_time_independent_views() {
        let spec = WindowSpec::time(10, 10); // tumbling: disjoint windows
        let mut ev = WindowEval::new(spec, EvalMode::Snapshot);

        let mut captured: Vec<GraphSnapshot> = Vec::new();
        let mut expected: Vec<usize> = Vec::new();

        // Window k holds k+1 DISTINCT triples, all timestamped at the window's
        // start `k*10` so they stay inside `[k*10, k*10+10)` (the distinctness
        // is in the object, not the timestamp). The growing per-window size
        // means the live master changes every slide and crosses
        // MIN_COMPACT_CHURN, so the graph actually compacts mid-stream — and a
        // stale view would read the wrong (final) count.
        let windows = 40u64;
        for k in 0..windows {
            let n = k + 1;
            for j in 0..n {
                ev.push(triple("s", "p", &format!("o{k}_{j}")), k * 10);
            }
            expected.push(n as usize);
        }

        {
            let captured = &mut captured;
            ev.eval_closed(true, &mut |_start, _end, graph: &Graph| {
                // Capture an OWNED snapshot of the exact graph the engine sees.
                captured.push(graph.snapshot());
                Ok(())
            })
            .unwrap();
        }

        assert_eq!(captured.len(), expected.len(), "one snapshot per closed window");
        let counts: Vec<usize> = captured.iter().map(|s| s.len()).collect();
        assert_eq!(counts, expected, "a captured snapshot drifted from its window's content");
        // The windows genuinely differ in size, so a stale/aliasing view would
        // be caught (every capture would read the last window's count).
        assert!(counts.windows(2).all(|w| w[0] < w[1]), "windows strictly grow, so views must differ");
    }

    /// (sq-2n1q3.4) Per-window solutions for one mode, sorted
    /// order-insensitively (ties between value-equal literals may order
    /// differently across materialisation strategies).
    fn windows_of(mode: EvalMode, spec: WindowSpec, pushes: &[([Term; 3], u64)]) -> Vec<Vec<String>> {
        let mut ev = WindowEval::new(spec, mode);
        for (t, ts) in pushes {
            ev.push(t.clone(), *ts);
        }
        let mut out = Vec::new();
        ev.eval_closed(true, &mut |_s, _e, g: &Graph| {
            let rows =
                sparq_engine::query(g, "SELECT ?o WHERE { ?s ?p ?o }").expect("window eval");
            let mut objs: Vec<String> = rows.rows.iter().map(|r| format!("{:?}", r)).collect();
            objs.sort();
            out.push(objs);
            Ok(())
        })
        .unwrap();
        out
    }

    /// (sq-2n1q3.4) The substrate-backed id-level diff must stay
    /// LEXICAL, exactly like the term-level hash-set diff it replaced:
    /// `"42"^^xsd:integer` and `"042"^^xsd:integer` are value-equal but
    /// lexically DISTINCT terms (only the canonical form takes an inline id; a
    /// non-canonical lexical form stays its own dictionary entry), so a window
    /// swapping one form for the other must register a real insert + delete. A
    /// diff that conflated the two ids would leave a stale triple in the live
    /// graph and diverge from the diff-free `Rebuild` baseline here.
    #[test]
    fn delta_diff_distinguishes_lexically_distinct_value_equal_literals() {
        let lit = |lex: &str| -> [Term; 3] {
            [
                NamedNode::new_unchecked("http://ex/s").into(),
                NamedNode::new_unchecked("http://ex/p").into(),
                Literal::new_typed_literal(lex, xsd::INTEGER).into(),
            ]
        };
        let spec = WindowSpec::time(10, 10); // tumbling
        let pushes = vec![
            (lit("42"), 0),   // w0: the canonical form only
            (lit("42"), 12),  // w1: BOTH lexical forms of the value 42
            (lit("042"), 13),
            (lit("042"), 25), // w2: non-canonical only — "42" must be deleted
        ];
        let rebuild = windows_of(EvalMode::Rebuild, spec, &pushes);
        assert_eq!(rebuild.len(), 3, "three tumbling windows");
        assert_eq!(rebuild[1].len(), 2, "w1 holds TWO lexically-distinct objects");
        assert_eq!(rebuild[2].len(), 1, "w2 holds only the non-canonical form");
        assert_eq!(
            windows_of(EvalMode::Delta, spec, &pushes),
            rebuild,
            "Delta's id-level diff diverged from the Rebuild baseline"
        );
        assert_eq!(
            windows_of(EvalMode::Snapshot, spec, &pushes),
            rebuild,
            "Snapshot's id-level diff diverged from the Rebuild baseline"
        );
    }

    /// (sq-2n1q3.4) The `DeltaTable`-backed slide end to end:
    /// duplicate timestamps dedup to one triple (RDF set semantics via the
    /// window's own build table), a partially-overlapping next window diffs to
    /// exactly the changed triples, an empty window deletes everything, and the
    /// stream refills after it — each window's content matching the diff-free
    /// `Rebuild` baseline exactly.
    #[test]
    fn substrate_diff_matches_rebuild_on_dedup_partial_overlap_and_empty_windows() {
        let spec = WindowSpec::time(10, 10); // tumbling
        let pushes = vec![
            // w0 [0,10): {a, b}, with `a` present at two timestamps.
            (triple("s", "p", "a"), 0),
            (triple("s", "p", "a"), 3),
            (triple("s", "p", "b"), 5),
            // w1 [10,20): {b, c} — partial overlap: insert c, delete a.
            (triple("s", "p", "b"), 12),
            (triple("s", "p", "c"), 15),
            // w2 [20,30): empty — everything is deleted.
            // w3 [30,40): {d} — refill after the empty window.
            (triple("s", "p", "d"), 30),
        ];
        let rebuild = windows_of(EvalMode::Rebuild, spec, &pushes);
        assert_eq!(rebuild.len(), 4, "w0..w3 including the empty w2");
        assert_eq!(rebuild[0].len(), 2, "duplicate timestamps dedup to {{a, b}}");
        assert!(rebuild[2].is_empty(), "w2 is the empty window");
        assert_eq!(rebuild[3].len(), 1, "w3 refills after the empty window");
        assert_eq!(
            windows_of(EvalMode::Delta, spec, &pushes),
            rebuild,
            "Delta's id-level diff diverged from the Rebuild baseline"
        );
        assert_eq!(
            windows_of(EvalMode::Snapshot, spec, &pushes),
            rebuild,
            "Snapshot's id-level diff diverged from the Rebuild baseline"
        );
    }

    /// `Snapshot` mode produces the same per-window triple counts as `Delta`
    /// (and so the same content) — they share the live-graph maintenance and
    /// differ only in borrow-vs-snapshot at the hand-off.
    #[test]
    fn snapshot_and_delta_agree_per_window() {
        let spec = WindowSpec::time(10, 5); // sliding overlap
        let counts = |mode: EvalMode| -> Vec<usize> {
            let mut ev = WindowEval::new(spec, mode);
            for ts in 0..40u64 {
                ev.push(triple("s", "p", &format!("o{}", ts % 6)), ts);
            }
            let mut out = Vec::new();
            ev.eval_closed(true, &mut |_s, _e, g: &Graph| {
                out.push(g.len());
                Ok(())
            })
            .unwrap();
            out
        };
        assert_eq!(counts(EvalMode::Snapshot), counts(EvalMode::Delta));
        assert!(!counts(EvalMode::Snapshot).is_empty());
    }
}
