//! [OPUS-4.8] (sq-iywur) Opt-in dynamic-programming join-order enumerator.
//!
//! The default planner is the greedy GOO heuristic in `crate::exec` (`goo_seed` /
//! `goo_pick`): it builds a left-deep order one pattern at a time, always extending
//! with the connected candidate of smallest estimated output. Greedy is fast and
//! usually good, but it can be arbitrarily far from optimal on adversarial BGP
//! shapes (it never reconsiders an early bad choice).
//!
//! This module adds an OPT-IN alternative: a **connected-subgraph-complement-pair
//! (DPccp) dynamic program** (Moerkotte & Neumann, *"Analysis of Two Existing and
//! One New Dynamic Programming Algorithm for the Generation of Optimal Bushy Join
//! Trees without Cross Products"*, VLDB 2006). It enumerates every connected
//! subgraph of the BGP join graph and, for each, its connected complements, filling
//! a DP table `best[S]` = the minimum-`Cout` join tree over the pattern set `S`. It
//! considers **bushy** trees and never introduces a cross product between two
//! patterns that do not share a variable, so on shapes where greedy mis-orders it
//! finds a provably `Cout`-optimal tree.
//!
//! ## Order-only; result-equivalent
//!
//! A BGP is a conjunctive natural join, which is commutative and associative on
//! multisets, so **every** join tree over the same patterns yields the **same**
//! bindings. The DP therefore only changes the *order* (and shape) in which the
//! executor joins — never the answer. Cost is seeded from the same
//! index-range/characteristic-set cardinality estimator the greedy planner uses
//! (`Prepared::est` + `pattern_var_ndv`), so the two planners consume identical
//! statistics.
//!
//! ## Bounded blow-up: greedy fallback above a budget
//!
//! DPccp is worst-case exponential (a clique BGP has `2ⁿ−1` connected subgraphs, and
//! ≈`3ⁿ` csg/cmp pairs). To keep planning cheap this module counts the connected
//! subgraphs first and, if the count would exceed a configurable **budget**
//! (`DpConfig::max_subgraphs`), returns `None` so the caller transparently falls back
//! to greedy GOO. Because the pair count grows *super-linearly* in the subgraph count
//! (≈`#csg^1.585`), a second guard caps emitted csg/cmp pairs at `max_subgraphs`
//! times `PAIR_BUDGET_FACTOR` — that is what actually bounds the `pairs` allocation
//! and sort, even for a very large budget. It also bails (returns `None`) when the BGP
//! join graph is **disconnected** (a deliberate cross-product query, or an all-constant
//! pattern with no variables) — those have no single connected plan, and the greedy
//! path already handles cross products.
//!
//! ## Default-on when compiled; opt-out available [SONNET-4.6] sq-7d3dj.30.5
//!
//! The whole module compiles only under the non-default `dp-planner` cargo feature,
//! keeping `sparq-core` / `sparq-engine` lean when the feature is off (zero DP code
//! compiles; the default native + wasm builds are byte-identical to the greedy-only
//! build). When the feature IS compiled, DPccp runs **by default** on every thread
//! for any BGP within the `4096`-connected-subgraph budget — no explicit install is
//! required.
//!
//! To restore greedy GOO within a scoped call, use [`without_dp_planner`]. To set a
//! budget different from the compiled-in default, use [`with_dp_planner_budget`].
//! [`with_dp_planner`] re-installs the default budget explicitly and is provided for
//! symmetry and backward compatibility. Pulls in ZERO new dependencies and contains
//! no `unsafe`.

use rustc_hash::FxHashMap;
use std::cell::RefCell;

/// Default connected-subgraph budget for [`with_dp_planner`]. Above this many
/// connected subgraphs the enumerator returns `None` and the caller falls back to
/// greedy GOO. Chosen so a dense (near-clique) BGP of ~12 patterns still fits while
/// a genuinely large/dense one degrades to greedy instead of blowing up planning.
const DEFAULT_MAX_SUBGRAPHS: usize = 4096;

/// Pattern-count ceiling: the join graph is a `u64` bitset over patterns, and beyond
/// this the budget would trip first anyway. A BGP with more patterns than this is
/// planned greedily.
const MAX_PATTERNS: usize = 63;

/// Multiplier that derives the csg/cmp **pair** budget from `max_subgraphs`. The
/// connected-subgraph count (the DP-table size) bounds `max_subgraphs`, but the number
/// of csg/cmp *pairs* the DP materialises grows super-linearly in it (≈`#csg^1.585` on
/// a clique, since `3ⁿ = (2ⁿ)^{log₂3}`), so the subgraph budget alone does *not* bound
/// the `pairs` allocation + sort. Capping emitted pairs at `max_subgraphs *`
/// this factor makes `max_subgraphs` reliably bound planner resource usage while still
/// admitting the documented dense-`~12`-pattern case (a 12-clique emits `261 625`
/// unordered pairs `< 4096 * 128`); denser/larger graphs abort to greedy GOO.
const PAIR_BUDGET_FACTOR: usize = 128;

// ---- thread-local installation (mirrors cs::with_cs_table) --------------------

/// Runtime configuration for the DP planner path.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DpConfig {
    /// Fall back to greedy GOO once the BGP has more connected subgraphs than this.
    pub(crate) max_subgraphs: usize,
}

/// Three-way thread-local install state for the DPccp planner. [SONNET-4.6] sq-7d3dj.30.5
///
/// * `Default` — no explicit override; [`active()`] returns the compiled-in default
///   (`DEFAULT_MAX_SUBGRAPHS`). This is the initial state on every thread.
/// * `Enabled(DpConfig)` — explicit install at a specific budget (set by
///   [`install`] / [`with_dp_planner_budget`]).
/// * `Disabled` — explicit opt-out; [`active()`] returns `None` (greedy GOO). Set by
///   [`without_dp_planner`].
#[derive(Clone, Copy, Debug)]
enum Install {
    Default,
    Enabled(DpConfig),
    Disabled,
}

thread_local! {
    static ACTIVE: RefCell<Install> = const { RefCell::new(Install::Default) };
}

/// Restores the previous install state when dropped (including on unwind, so a
/// panicking query never leaks an install into the calling thread).
pub(crate) struct Guard {
    prev: Install,
}

impl Drop for Guard {
    fn drop(&mut self) {
        ACTIVE.with(|a| *a.borrow_mut() = self.prev);
    }
}

/// Atomically swaps the current install state with `s`, returning a [`Guard`] that
/// restores the previous state on drop.
fn set_install(s: Install) -> Guard {
    let prev = ACTIVE.with(|a| {
        let mut borrow = a.borrow_mut();
        let prev = *borrow;
        *borrow = s;
        prev
    });
    Guard { prev }
}

/// Installs a specific `DpConfig` on this thread, returning a guard that restores
/// the previous state on drop. Called by [`with_dp_planner`] / [`with_dp_planner_budget`].
pub(crate) fn install(cfg: DpConfig) -> Guard {
    set_install(Install::Enabled(cfg))
}

/// Returns the effective `DpConfig` for the current thread:
///
/// * `Install::Default` — returns `Some(DpConfig { max_subgraphs: DEFAULT_MAX_SUBGRAPHS })`.
/// * `Install::Enabled(cfg)` — returns `Some(cfg)`.
/// * `Install::Disabled` — returns `None` (greedy GOO).
pub(crate) fn active() -> Option<DpConfig> {
    ACTIVE.with(|a| match *a.borrow() {
        Install::Default => Some(DpConfig { max_subgraphs: DEFAULT_MAX_SUBGRAPHS }),
        Install::Enabled(cfg) => Some(cfg),
        Install::Disabled => None,
    })
}

/// Runs `f` with the DP join-order planner installed at the default subgraph budget:
/// every BGP the closure evaluates on this thread is planned by the DPccp enumerator
/// when it fits the budget and is connected, and by greedy GOO otherwise. The result
/// of any query is identical to the greedy default — only join order changes.
///
/// This is equivalent to the compiled-in default and is provided for backward
/// compatibility and explicit scoped-override semantics.
///
/// ```ignore
/// let result = sparq_engine::with_dp_planner(|| sparq_engine::query(&graph, sparql))?;
/// ```
pub fn with_dp_planner<T>(f: impl FnOnce() -> T) -> T {
    with_dp_planner_budget(DEFAULT_MAX_SUBGRAPHS, f)
}

/// Like [`with_dp_planner`] but with an explicit connected-subgraph budget: a BGP
/// whose join graph has more than `max_subgraphs` connected subgraphs falls back to
/// greedy GOO instead of running the DP (bounding planning cost). `max_subgraphs = 0`
/// disables the DP entirely (always greedy).
pub fn with_dp_planner_budget<T>(max_subgraphs: usize, f: impl FnOnce() -> T) -> T {
    let _guard = install(DpConfig { max_subgraphs });
    f()
}

/// Runs `f` with the DPccp join-order planner explicitly disabled for this thread:
/// every BGP evaluated inside the closure falls back to greedy GOO regardless of the
/// compiled-in default. This is the inverse of [`with_dp_planner`].
///
/// ```ignore
/// // Obtain the greedy baseline inside a scope that normally uses the default DPccp:
/// let greedy = sparq_engine::without_dp_planner(|| sparq_engine::query(&graph, sparql))?;
/// ```
pub fn without_dp_planner<T>(f: impl FnOnce() -> T) -> T {
    let _guard = set_install(Install::Disabled);
    f()
}

// ---- the join tree ------------------------------------------------------------

/// A (bushy) join tree over pattern indices, the output of the DP enumerator. The
/// executor evaluates it recursively: a `Leaf` scans one BGP pattern, a `Join`
/// natural-joins the two sub-results.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum JoinTree {
    /// Scan the BGP pattern at this index.
    Leaf(usize),
    /// Natural-join the two sub-trees.
    Join(Box<JoinTree>, Box<JoinTree>),
}

// ---- the query graph (cardinality/adjacency inputs) ---------------------------

/// One join variable (a variable shared by at least two patterns): the bitmask of
/// patterns it appears in, plus its per-pattern distinct-value estimate. Variables
/// confined to a single pattern never affect a join, so they are not stored here.
struct VarEntry {
    /// Bit `p` set iff pattern `p` contains this variable.
    pats: u64,
    /// `(pattern index, distinct-value estimate)` for each pattern containing it.
    ndv: Vec<(usize, f64)>,
}

/// The planner's view of a BGP: per-pattern base cardinality, the pattern-adjacency
/// graph (an edge iff two patterns share a variable), and the shared-variable
/// statistics used to estimate a join's output cardinality. Built by the executor
/// from its `Prepared` patterns; the DP itself is graph-agnostic (pure arithmetic),
/// which keeps it unit-testable without a store.
pub(crate) struct QueryGraph {
    n: usize,
    /// Per-pattern single-pattern cardinality estimate.
    est: Vec<f64>,
    /// Per-pattern adjacency mask (bit `j` set iff patterns `i`,`j` share a variable).
    neigh: Vec<u64>,
    /// Join variables (those in ≥2 patterns), for the cardinality model.
    vars: Vec<VarEntry>,
}

impl QueryGraph {
    /// Builds a `QueryGraph` from raw per-pattern / per-variable statistics.
    ///
    /// * `est[p]` — single-pattern cardinality estimate of pattern `p`.
    /// * `var_pats[k]` — bitmask of patterns containing variable `k`.
    /// * `var_ndv[k]` — `(pattern, distinct-value estimate)` for each pattern of variable `k`.
    ///
    /// `neigh` (pattern adjacency) is derived: two patterns are adjacent iff some
    /// variable appears in both.
    pub(crate) fn build(est: Vec<f64>, var_pats: Vec<u64>, var_ndv: Vec<Vec<(usize, f64)>>) -> QueryGraph {
        let n = est.len();
        let mut neigh = vec![0u64; n];
        let mut vars = Vec::new();
        for (k, &pats) in var_pats.iter().enumerate() {
            if pats.count_ones() >= 2 {
                // Every pair of patterns sharing this variable is adjacent.
                let mut m = pats;
                while m != 0 {
                    let p = m.trailing_zeros() as usize;
                    m &= m - 1;
                    neigh[p] |= pats & !(1u64 << p);
                }
                vars.push(VarEntry { pats, ndv: var_ndv[k].clone() });
            }
        }
        QueryGraph { n, est, neigh, vars }
    }

    /// The union of the neighbours of every node in `s`, excluding `s` itself.
    #[inline]
    fn neighborhood(&self, s: u64) -> u64 {
        let mut out = 0u64;
        let mut m = s;
        while m != 0 {
            let v = m.trailing_zeros() as usize;
            m &= m - 1;
            out |= self.neigh[v];
        }
        out & !s
    }

    /// `{0,1,…,i}` — the nodes with index ≤ `i`.
    #[inline]
    fn b(&self, i: usize) -> u64 {
        // i < MAX_PATTERNS ≤ 63, so `1 << (i+1)` never overflows `u64`.
        (1u64 << (i + 1)) - 1
    }

    /// Whether the sub-graph induced by `mask` is connected (single component).
    fn is_connected(&self, mask: u64) -> bool {
        if mask == 0 {
            return true;
        }
        let start = mask & mask.wrapping_neg();
        let mut seen = start;
        let mut frontier = start;
        while frontier != 0 {
            let mut next = 0u64;
            let mut f = frontier;
            while f != 0 {
                let v = f.trailing_zeros() as usize;
                f &= f - 1;
                next |= self.neigh[v] & mask & !seen;
            }
            seen |= next;
            frontier = next;
        }
        seen == mask
    }

    /// Estimated output cardinality of joining every pattern in the set `s`, under
    /// the per-variable independence model shared with greedy GOO: the product of
    /// the single-pattern estimates, divided — for each join variable present in
    /// `occ ≥ 2` of the patterns — by its distinct-value estimate `occ-1` times
    /// (each extra pattern that shares the variable is one equi-join, cutting the
    /// product by the variable's selectivity). The distinct-value estimate is the
    /// smallest (most selective) over the patterns in `s`.
    fn card(&self, s: u64) -> f64 {
        let mut c = 1.0f64;
        let mut m = s;
        while m != 0 {
            let p = m.trailing_zeros() as usize;
            m &= m - 1;
            c *= self.est[p];
        }
        for v in &self.vars {
            let occ = (v.pats & s).count_ones();
            if occ >= 2 {
                let mut ndv = f64::INFINITY;
                for &(p, d) in &v.ndv {
                    if (s >> p) & 1 == 1 {
                        ndv = ndv.min(d);
                    }
                }
                let ndv = ndv.max(1.0);
                c /= ndv.powi((occ - 1) as i32);
            }
        }
        // Map an overflowed (+∞, the product of many large `est[p]`) or ill-defined
        // (NaN — e.g. ∞/∞ when the product AND a distinct-value power both saturate)
        // cardinality to a large FINITE sentinel so it ranks as the WORST plan. A bare
        // `max(0.0)` would silently fold NaN → 0.0 (`f64::max` returns the non-NaN
        // operand), making the DP PREFER the pathological join instead of avoiding it.
        // [OPUS-4.8] sq-iywur
        if c.is_finite() {
            c.max(0.0)
        } else {
            f64::MAX
        }
    }

    // ---- connected-subgraph counting (budget pre-check) -----------------------

    /// Counts the connected subgraphs of the join graph, stopping as soon as the
    /// count exceeds `cap` (so the work is bounded by `cap`, not by `2ⁿ`). Used to
    /// decide whether to run the DP or fall back to greedy.
    fn count_csg(&self, cap: usize) -> usize {
        let mut count = 0usize;
        for i in (0..self.n).rev() {
            count += 1; // the singleton {i}
            if count > cap {
                return count;
            }
            let mut overflow = false;
            self.count_csg_rec(1u64 << i, self.b(i), &mut count, cap, &mut overflow);
            if overflow {
                return count;
            }
        }
        count
    }

    fn count_csg_rec(&self, s: u64, x: u64, count: &mut usize, cap: usize, overflow: &mut bool) {
        let n = self.neighborhood(s) & !x;
        let mut sub = n;
        while sub != 0 {
            *count += 1;
            if *count > cap {
                *overflow = true;
                return;
            }
            sub = sub.wrapping_sub(1) & n;
        }
        let mut sub = n;
        while sub != 0 {
            self.count_csg_rec(s | sub, x | n, count, cap, overflow);
            if *overflow {
                return;
            }
            sub = sub.wrapping_sub(1) & n;
        }
    }

    // ---- the DPccp csg-cmp-pair enumeration -----------------------------------

    /// Enumerates every connected-subgraph / connected-complement pair `(s1, s2)`
    /// (disjoint, each connected, joined by ≥1 edge) exactly once, in an order where
    /// both components are enumerated before their union — the emission order DPccp
    /// relies on so `best[s1]` / `best[s2]` are final when the pair is combined.
    ///
    /// Returns `false` (aborting the walk) as soon as more than `cap` pairs have been
    /// emitted, so the caller can bound the `pairs` allocation + sort even though the
    /// pair count grows super-linearly in the connected-subgraph budget. Generic over
    /// `F: FnMut` (rather than `&mut dyn FnMut`) so the hot per-emission call inlines
    /// and carries no vtable indirection.
    fn enumerate_csg_cmp_pairs<F: FnMut(u64, u64)>(&self, cap: usize, emit: &mut F) -> bool {
        let mut count = 0usize;
        for i in (0..self.n).rev() {
            let s1 = 1u64 << i;
            if !self.emit_csg_cmp(s1, cap, &mut count, emit) {
                return false;
            }
            if !self.enumerate_csg_rec(s1, self.b(i), cap, &mut count, emit) {
                return false;
            }
        }
        true
    }

    /// Grows the connected subgraph `s` (whose minimum node fixed the outer seed),
    /// emitting the complements of each larger connected subgraph it reaches. Returns
    /// `false` once `*count` exceeds `cap`.
    fn enumerate_csg_rec<F: FnMut(u64, u64)>(&self, s: u64, x: u64, cap: usize, count: &mut usize, emit: &mut F) -> bool {
        let n = self.neighborhood(s) & !x;
        let mut sub = n;
        while sub != 0 {
            if !self.emit_csg_cmp(s | sub, cap, count, emit) {
                return false;
            }
            sub = sub.wrapping_sub(1) & n;
        }
        let mut sub = n;
        while sub != 0 {
            if !self.enumerate_csg_rec(s | sub, x | n, cap, count, emit) {
                return false;
            }
            sub = sub.wrapping_sub(1) & n;
        }
        true
    }

    /// For a connected subgraph `s1`, enumerates and emits all its connected
    /// complements (each with minimum node greater than `min(s1)`, so each unordered
    /// pair is emitted once). Returns `false` once `*count` exceeds `cap`.
    fn emit_csg_cmp<F: FnMut(u64, u64)>(&self, s1: u64, cap: usize, count: &mut usize, emit: &mut F) -> bool {
        let min = s1.trailing_zeros() as usize;
        let x = s1 | self.b(min);
        let nbh = self.neighborhood(s1) & !x;
        // Descending index over the neighbourhood seeds. The forbidden set passed to
        // the growth is `x ∪ (B(v) ∩ nbh)` — NOT `x ∪ B(v)`: we must forbid only the
        // OTHER neighbourhood seeds with index ≤ v (to emit each pair once), while
        // still allowing the complement to grow *inward* to lower-indexed nodes that
        // are not themselves adjacent to `s1` (e.g. a complement `{2,3}` reached via
        // its non-minimum node 3). Forbidding all of `B(v)` would drop those pairs.
        let mut m = nbh;
        while m != 0 {
            let v = 63 - m.leading_zeros() as usize;
            m &= !(1u64 << v);
            let s2 = 1u64 << v;
            emit(s1, s2);
            *count += 1;
            if *count > cap {
                return false;
            }
            if !self.enumerate_cmp_rec(s1, s2, x | (self.b(v) & nbh), cap, count, emit) {
                return false;
            }
        }
        true
    }

    /// Grows the connected complement `s2` of `s1`, emitting `(s1, s2∪s')` for each
    /// larger connected complement reachable within the allowed node set. Returns
    /// `false` once `*count` exceeds `cap`.
    fn enumerate_cmp_rec<F: FnMut(u64, u64)>(&self, s1: u64, s2: u64, x: u64, cap: usize, count: &mut usize, emit: &mut F) -> bool {
        let n = self.neighborhood(s2) & !x;
        let mut sub = n;
        while sub != 0 {
            emit(s1, s2 | sub);
            *count += 1;
            if *count > cap {
                return false;
            }
            sub = sub.wrapping_sub(1) & n;
        }
        let mut sub = n;
        while sub != 0 {
            if !self.enumerate_cmp_rec(s1, s2 | sub, x | n, cap, count, emit) {
                return false;
            }
            sub = sub.wrapping_sub(1) & n;
        }
        true
    }
}

/// Best plan found for a subset so far: the minimum-`Cout` join tree and its cost.
struct Entry {
    cost: f64,
    tree: JoinTree,
}

/// Runs the DPccp enumerator over `qg`, returning the minimum-`Cout` bushy join tree
/// spanning **all** patterns, or `None` — meaning "fall back to greedy GOO" — when:
///
/// * there are fewer than **3** patterns or more than `MAX_PATTERNS` — for n=1 there is
///   nothing to order; for n=2 greedy GOO is already `Cout`-optimal (there is exactly one
///   connected join, and greedy seeds from the more selective pattern) and carries less
///   overhead than the full DPccp enumeration path. [SONNET-4.6] sq-7d3dj.30.5
/// * the BGP join graph is disconnected (no single connected plan; greedy does the
///   cross products);
/// * the connected-subgraph count exceeds `max_subgraphs` (DP-table budget guard); or
/// * the csg/cmp pair count exceeds `max_subgraphs * PAIR_BUDGET_FACTOR` (a separate
///   guard: pairs grow super-linearly in the subgraph count, so this is what bounds the
///   `pairs` allocation + sort even for a very large `max_subgraphs`).
pub(crate) fn plan(qg: &QueryGraph, max_subgraphs: usize) -> Option<JoinTree> {
    let n = qg.n;
    if !(3..=MAX_PATTERNS).contains(&n) || max_subgraphs == 0 {
        return None;
    }
    // `n <= MAX_PATTERNS` (63) after the guard above, so the shift never overflows and
    // the full set is always `(1 << n) - 1` — no `n == 64` special case is reachable.
    let full: u64 = (1u64 << n) - 1;
    if !qg.is_connected(full) {
        return None;
    }
    if qg.count_csg(max_subgraphs) > max_subgraphs {
        return None;
    }

    // Collect the complete SET of connected-subgraph / connected-complement pairs,
    // then combine them in ascending union-size order. The DPccp enumeration yields
    // exactly the right pairs; processing by increasing `|s1∪s2|` makes the DP
    // trivially correct — both components (which are strictly smaller) are always
    // finalised before their union is built, with no reliance on emission order.
    //
    // The pair count grows super-linearly in `count_csg` (≈`#csg^1.585` on a clique),
    // so the DP-table budget above does not bound this allocation + sort; cap emitted
    // pairs at `max_subgraphs * PAIR_BUDGET_FACTOR` and fall back to greedy on overflow.
    let pair_cap = max_subgraphs.saturating_mul(PAIR_BUDGET_FACTOR);
    let mut pairs: Vec<(u64, u64)> = Vec::new();
    if !qg.enumerate_csg_cmp_pairs(pair_cap, &mut |s1, s2| pairs.push((s1, s2))) {
        return None;
    }
    pairs.sort_by_key(|&(s1, s2)| (s1 | s2).count_ones());

    let mut best: FxHashMap<u64, Entry> = FxHashMap::default();
    for v in 0..n {
        best.insert(1u64 << v, Entry { cost: 0.0, tree: JoinTree::Leaf(v) });
    }
    for (s1, s2) in pairs {
        let s = s1 | s2;
        let (cost1, cost2) = match (best.get(&s1), best.get(&s2)) {
            (Some(a), Some(b)) => (a.cost, b.cost),
            _ => continue,
        };
        let cost = cost1 + cost2 + qg.card(s);
        if best.get(&s).is_none_or(|e| cost < e.cost) {
            let t1 = best[&s1].tree.clone();
            let t2 = best[&s2].tree.clone();
            best.insert(s, Entry { cost, tree: JoinTree::Join(Box::new(t1), Box::new(t2)) });
        }
    }
    best.remove(&full).map(|e| e.tree)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a `QueryGraph` from an adjacency-free description: per-pattern est plus
    /// a list of `(pattern-mask, ndv)` variables. `ndv` is applied uniformly to every
    /// pattern in the mask.
    fn qg(est: &[f64], vars: &[(u64, f64)]) -> QueryGraph {
        let mut var_pats = Vec::new();
        let mut var_ndv = Vec::new();
        for &(pats, ndv) in vars {
            var_pats.push(pats);
            let mut v = Vec::new();
            let mut m = pats;
            while m != 0 {
                let p = m.trailing_zeros() as usize;
                m &= m - 1;
                v.push((p, ndv));
            }
            var_ndv.push(v);
        }
        QueryGraph::build(est.to_vec(), var_pats, var_ndv)
    }

    /// Independent, obviously-correct reference: the minimum `Cout` over all bushy
    /// join trees, via a plain subset DP in increasing popcount order (subsets always
    /// precede supersets, so no ordering subtlety). Returns `None` if `full` is
    /// disconnected.
    fn ref_opt_cost(qg: &QueryGraph, n: usize) -> Option<f64> {
        let full: u64 = (1u64 << n) - 1;
        if !qg.is_connected(full) {
            return None;
        }
        let mut cost = vec![f64::INFINITY; 1usize << n];
        // Order subsets by popcount so every proper subset is solved first.
        let mut masks: Vec<u64> = (1u64..(1u64 << n)).collect();
        masks.sort_by_key(|m| m.count_ones());
        for &s in &masks {
            if s.count_ones() == 1 {
                cost[s as usize] = 0.0;
                continue;
            }
            if !qg.is_connected(s) {
                continue;
            }
            let mut sub = (s - 1) & s;
            while sub != 0 {
                let comp = s ^ sub;
                // Both halves connected and connected to each other (share an edge).
                if qg.is_connected(sub)
                    && qg.is_connected(comp)
                    && qg.neighborhood(sub) & comp != 0
                {
                    let c = cost[sub as usize] + cost[comp as usize] + qg.card(s);
                    if c < cost[s as usize] {
                        cost[s as usize] = c;
                    }
                }
                sub = (sub - 1) & s;
            }
        }
        Some(cost[full as usize])
    }

    /// The `Cout` of an actual `JoinTree` (sum of every intermediate-join cardinality).
    fn tree_cost(qg: &QueryGraph, t: &JoinTree) -> (f64, u64) {
        match t {
            JoinTree::Leaf(i) => (0.0, 1u64 << i),
            JoinTree::Join(l, r) => {
                let (cl, sl) = tree_cost(qg, l);
                let (cr, sr) = tree_cost(qg, r);
                let s = sl | sr;
                (cl + cr + qg.card(s), s)
            }
        }
    }

    #[test]
    fn chain_of_three_optimal_order() {
        // ?a p ?b . ?b q ?c . ?c r ?d  — a 3-pattern chain. The middle pattern (1)
        // is huge; the ends (0,2) are tiny and each shares one variable with 1.
        // Optimal joins the two cheap ends against the middle, never the two ends
        // together (no shared variable ⇒ cross product) — which greedy could also
        // find, but here we assert the DP returns THE optimal cost.
        let g = qg(&[10.0, 100_000.0, 10.0], &[(0b011, 5.0), (0b110, 5.0)]);
        let tree = plan(&g, 4096).expect("connected 3-chain must plan");
        let (cost, span) = tree_cost(&g, &tree);
        assert_eq!(span, 0b111, "tree must span all patterns");
        assert_eq!(Some(cost), ref_opt_cost(&g, 3), "DP must match brute-force optimum");
    }

    #[test]
    fn star_of_four_optimal() {
        // A 4-pattern star: patterns 1,2,3 each share the centre variable with 0.
        let g = qg(&[50.0, 20.0, 5.0, 100.0], &[(0b1111, 10.0)]);
        let tree = plan(&g, 4096).expect("connected star must plan");
        let (cost, span) = tree_cost(&g, &tree);
        assert_eq!(span, 0b1111);
        assert_eq!(Some(cost), ref_opt_cost(&g, 4));
    }

    #[test]
    fn disconnected_falls_back() {
        // Two patterns sharing no variable ⇒ disconnected ⇒ None (greedy handles it).
        let g = qg(&[10.0, 10.0], &[(0b01, 3.0), (0b10, 3.0)]);
        assert!(plan(&g, 4096).is_none());
    }

    #[test]
    fn single_pattern_falls_back() {
        let g = qg(&[10.0], &[]);
        assert!(plan(&g, 4096).is_none());
    }

    #[test]
    fn budget_zero_falls_back() {
        // n=2 now returns None for any budget (n<3 threshold), and zero budget would
        // also trip for n>=3. Use n=3 to test the budget-zero guard specifically.
        let g = qg(&[10.0, 20.0, 30.0], &[(0b011, 4.0), (0b110, 4.0)]);
        assert!(plan(&g, 0).is_none(), "a zero budget must always fall back to greedy");
    }

    /// For n=2 connected BGPs, DPccp adds overhead without benefit — greedy GOO is
    /// already optimal (one join order). `plan()` must return `None`. [SONNET-4.6] sq-7d3dj.30.5
    #[test]
    fn two_pattern_bgp_falls_back_to_greedy() {
        let g = qg(&[10.0, 20.0], &[(0b11, 4.0)]);
        assert!(plan(&g, 4096).is_none(), "n=2 connected BGP must fall back (n<3 threshold)");
        assert!(plan(&g, DEFAULT_MAX_SUBGRAPHS).is_none(), "n=2 must fall back at default budget");
    }

    #[test]
    fn tight_budget_falls_back_on_clique() {
        // A 5-clique has 2^5-1 = 31 connected subgraphs; a budget of 8 must trip.
        let g = qg(
            &[10.0, 10.0, 10.0, 10.0, 10.0],
            &[(0b11111, 7.0), (0b01111, 6.0), (0b00111, 5.0)],
        );
        assert!(plan(&g, 8).is_none(), "clique above budget must fall back");
        // With a generous budget it plans and matches the optimum.
        let tree = plan(&g, 4096).expect("clique within budget plans");
        let (cost, span) = tree_cost(&g, &tree);
        assert_eq!(span, 0b11111);
        assert_eq!(Some(cost), ref_opt_cost(&g, 5));
    }

    #[test]
    fn pair_budget_aborts_enumeration() {
        // The csg/cmp pair count grows ≈3ⁿ while the subgraph budget bounds ≈2ⁿ, so the
        // pair cap is a distinct guard. A 6-clique emits 301 unordered pairs; enumerating
        // with a tiny cap must abort early and never materialise more than `cap + 1`,
        // so `max_subgraphs` reliably bounds the planner's `pairs` allocation.
        let g = qg(&[10.0; 6], &[(0b111111, 5.0)]);
        let mut pairs = Vec::new();
        let completed = g.enumerate_csg_cmp_pairs(4, &mut |s1, s2| pairs.push((s1, s2)));
        assert!(!completed, "a tiny pair cap must abort the enumeration");
        assert!(pairs.len() <= 5, "aborted enumeration must stay within cap+1, got {}", pairs.len());

        // With a generous cap the same enumeration completes and yields every pair.
        let mut all = Vec::new();
        let completed = g.enumerate_csg_cmp_pairs(usize::MAX, &mut |s1, s2| all.push((s1, s2)));
        assert!(completed, "a generous cap must let the full enumeration complete");
        assert_eq!(all.len(), 301, "6-clique has (3⁶−2⁷+1)/2 = 301 unordered csg/cmp pairs");
    }

    #[test]
    fn matches_bruteforce_on_many_random_graphs() {
        // Deterministic PRNG (xorshift) — no dev-dep. For many random connected
        // graphs of 3..=6 patterns, the DP's Cout must equal the brute-force optimum,
        // and the returned tree must span every pattern and realise that cost.
        let mut state: u64 = 0x9E3779B97F4A7C15;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut checked = 0;
        for _ in 0..4000 {
            let n = 3 + (next() % 4) as usize; // 3..=6
            let est: Vec<f64> = (0..n).map(|_| 1.0 + (next() % 5000) as f64).collect();
            // Random variables, each spanning a random ≥2-pattern subset.
            let nvars = 1 + (next() % 4) as usize;
            let mut var_pats = Vec::new();
            let mut var_ndv = Vec::new();
            for _ in 0..nvars {
                let mut pats = next() & ((1u64 << n) - 1);
                // Force ≥2 patterns so the variable is a join edge.
                while pats.count_ones() < 2 {
                    pats |= 1u64 << (next() % n as u64);
                }
                var_pats.push(pats);
                let ndv = 1.0 + (next() % 200) as f64;
                let mut v = Vec::new();
                let mut m = pats;
                while m != 0 {
                    let p = m.trailing_zeros() as usize;
                    m &= m - 1;
                    v.push((p, ndv));
                }
                var_ndv.push(v);
            }
            let g = QueryGraph::build(est, var_pats, var_ndv);
            match (plan(&g, 1 << 20), ref_opt_cost(&g, n)) {
                (Some(tree), Some(opt)) => {
                    let (cost, span) = tree_cost(&g, &tree);
                    assert_eq!(span, (1u64 << n) - 1, "tree must span all patterns");
                    assert!(
                        (cost - opt).abs() <= 1e-6 * opt.max(1.0),
                        "DP cost {} != brute-force optimum {}",
                        cost,
                        opt
                    );
                    checked += 1;
                }
                (None, None) => {} // both agree the graph is disconnected
                (a, b) => panic!("DP/reference disagree on connectivity: {:?} vs {:?}", a.is_some(), b.is_some()),
            }
        }
        assert!(checked > 100, "expected many connected samples, got {}", checked);
    }

    // ---- default-on and opt-out tests [SONNET-4.6] sq-7d3dj.30.5 ----------------

    /// `active()` returns `None` inside `without_dp_planner` — direct unit test for
    /// the `Disabled` install state and the `without_dp_planner` public function.
    #[test]
    fn disabled_state_makes_active_return_none() {
        without_dp_planner(|| {
            assert!(active().is_none(), "active() must be None inside without_dp_planner");
        });
    }

    /// A scoped `install` at a custom budget restores the enclosing state when the guard drops.
    /// Uses `with_dp_planner` as the outer scope to establish a known `Enabled` state.
    #[test]
    fn install_override_restores_on_drop() {
        with_dp_planner(|| {
            // Outer: Enabled(DEFAULT_MAX_SUBGRAPHS).
            assert_eq!(active().unwrap().max_subgraphs, DEFAULT_MAX_SUBGRAPHS);
            {
                let _g = install(DpConfig { max_subgraphs: 7 });
                assert_eq!(active().unwrap().max_subgraphs, 7);
            }
            // Guard dropped: Enabled(DEFAULT_MAX_SUBGRAPHS) restored.
            assert_eq!(active().unwrap().max_subgraphs, DEFAULT_MAX_SUBGRAPHS);
        });
    }

    /// After `without_dp_planner` guard drops, the previously enclosing state is restored
    /// (here `Enabled(DEFAULT_MAX_SUBGRAPHS)` from `with_dp_planner`).
    #[test]
    fn opt_out_restores_prior_state_on_drop() {
        with_dp_planner(|| {
            let result = without_dp_planner(|| {
                assert!(active().is_none());
                99u64 // Return a sentinel to confirm the closure ran.
            });
            assert_eq!(result, 99);
            // Restored to Enabled(DEFAULT_MAX_SUBGRAPHS).
            assert_eq!(active().unwrap().max_subgraphs, DEFAULT_MAX_SUBGRAPHS);
        });
    }
}
