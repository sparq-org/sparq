//! sq-qonbz.2 — delta-aware adjacency for the OWL-RL semi-naive fixpoint.
//!
//! Replaces the `out`/`inc` `FxHashMap<Id, FxHashMap<Id, Vec<Id>>>` adjacency tables in
//! [`crate::owl::owl_rl_closure`] with a pair of [`sparq_substrate::join::delta::DeltaTable`]
//! persistent build-side tables, gated behind the default-OFF `substrate-join` Cargo feature.
//!
//! # Layout
//!
//! | table | rows        | key columns | value column | replaces                |
//! |-------|-------------|-------------|--------------|-------------------------|
//! | `out` | `[p, s, o]` | `[p, s]`    | `o` col 2    | `out[p][s]` → `[o, ..]` |
//! | `inc` | `[p, o, s]` | `[p, o]`    | `s` col 2    | `inc[p][o]` → `[s, ..]` |
//!
//! Both tables are built once from the seed triple set and extended O(|Δ|) each round via
//! `extend` — exactly the asymptotic cost of the `or_default().push()` pattern they replace.
//! `rebuild` is called at union-find merge epochs and axiom-added reseeds, mirroring the
//! `build_adjacency` calls in the plain path.
//!
//! # Zero-overhead contract
//!
//! No `Box<dyn>` / `&dyn` anywhere on the probe path. `probe_out` and `probe_inc` take a
//! generic `F: FnMut(..)` emit closure — monomorphised per call site. The `DeltaTable`
//! `probe_emit` carries `#[inline]` so cross-crate LTO eliminates the call boundary.
//! `scripts/check-no-dyn-dispatch.py` covers the parent `owl.rs` file; this module is
//! imported only from `owl.rs`.
//!
//! # Determinism contract
//!
//! `DeltaTable` emits matches in insertion order (ascending arena offset). Given an identical
//! build/extend sequence the probe output is deterministic, preserving the OWL-RL ratchet
//! byte-identity invariant (§4.2 of `research/substrate-remaining-design.md`).
//!
//! # Feature requirement
//!
//! Compiled only with the `substrate-join` Cargo feature.

use rustc_hash::FxHashSet;
use sparq_core::dict::Id;
use sparq_substrate::join::delta::DeltaTable;
use sparq_substrate::join::{JoinKeys, NoBudget};
use sparq_substrate::rows::Row;

/// The `JoinKeys` for the `out` table: build key columns `[p, s]` = cols 0 and 1.
/// No `right_only` — the seam returns the full build row; callers extract the value column.
#[inline]
fn out_keys() -> JoinKeys {
    JoinKeys { key_cols: vec![(0, 0), (1, 1)], right_only: vec![] }
}

/// The `JoinKeys` for the `inc` table: build key columns `[p, o]` = cols 0 and 1.
/// No `right_only` — same convention as `out_keys`.
#[inline]
fn inc_keys() -> JoinKeys {
    JoinKeys { key_cols: vec![(0, 0), (1, 1)], right_only: vec![] }
}

/// Persistent delta-aware adjacency for the OWL-RL `prp-fp` / `prp-ifp` / `prp-trp` rules.
///
/// Wraps two [`DeltaTable`]s:
/// - `out_tbl`: forward adjacency — rows `[p, s, o]`, keyed on `[p, s]`.
/// - `inc_tbl`: backward adjacency — rows `[p, o, s]`, keyed on `[p, o]`.
///
/// Only populated for predicates in `need` (transitive / functional / inv-functional),
/// exactly like the `FxHashMap` adjacency it replaces.
///
/// sq-qonbz.2
pub(crate) struct DeltaAdj {
    out_tbl: DeltaTable,
    inc_tbl: DeltaTable,
}

impl DeltaAdj {
    /// Build the tables from the current `all` triple set, populating only triples whose
    /// predicate is in `need`. Mirrors `build_adjacency` in `owl.rs`.
    #[inline]
    pub fn build(all: &FxHashSet<[Id; 3]>, need: &FxHashSet<Id>) -> Self {
        let mut out_rows: Vec<Row> = Vec::new();
        let mut inc_rows: Vec<Row> = Vec::new();
        if !need.is_empty() {
            for &[s, p, obj] in all {
                if need.contains(&p) {
                    out_rows.push(Row::from_slice(&[p, s, obj]));
                    inc_rows.push(Row::from_slice(&[p, obj, s]));
                }
            }
        }
        let out_tbl = DeltaTable::build(&out_rows, &out_keys());
        let inc_tbl = DeltaTable::build(&inc_rows, &inc_keys());
        DeltaAdj { out_tbl, inc_tbl }
    }

    /// Full reconstruction from `all` at a merge epoch or axiom-added reseed.
    /// Mirrors `build_adjacency(&all, &need, &mut out, &mut inc)` in the plain path.
    #[inline]
    pub fn rebuild(&mut self, all: &FxHashSet<[Id; 3]>, need: &FxHashSet<Id>) {
        let mut out_rows: Vec<Row> = Vec::new();
        let mut inc_rows: Vec<Row> = Vec::new();
        if !need.is_empty() {
            for &[s, p, obj] in all {
                if need.contains(&p) {
                    out_rows.push(Row::from_slice(&[p, s, obj]));
                    inc_rows.push(Row::from_slice(&[p, obj, s]));
                }
            }
        }
        self.out_tbl.rebuild(&out_rows, &out_keys());
        self.inc_tbl.rebuild(&inc_rows, &inc_keys());
    }

    /// Extend both tables with one newly-committed triple `[s, p, o]` (predicate already
    /// checked to be in `need` by the caller). Mirrors the `or_default().push()` calls in
    /// `commit_serial`.
    #[inline]
    pub fn extend_one(&mut self, s: Id, p: Id, obj: Id) {
        // out: forward row [p, s, o]
        let out_row = Row::from_slice(&[p, s, obj]);
        self.out_tbl.extend(&[out_row], &out_keys());
        // inc: backward row [p, o, s]
        let inc_row = Row::from_slice(&[p, obj, s]);
        self.inc_tbl.extend(&[inc_row], &inc_keys());
    }

    /// Probe the forward table (`out`): for the predicate `p` and subject `s`, invoke `emit(o)`
    /// for every object `o` known in `out[p][s]`. Mirrors `out.get(&p).and_then(|m| m.get(&s))`.
    ///
    /// The budget is `NoBudget` (materialisation runs to completion — see the module-level doc).
    #[inline]
    pub fn probe_out(&self, p: Id, s: Id, mut emit: impl FnMut(Id)) {
        let probe_row = Row::from_slice(&[p, s]);
        // Build a 2-column probe row; out_keys maps build col 0 -> probe col 0, build col 1 ->
        // probe col 1, so the key is `[p, s]` matching build rows `[p, s, o]`.
        let keys = out_keys();
        self.out_tbl.probe_emit(&[probe_row], &keys, &NoBudget, &mut |build, _probe| {
            emit(build[2]); // col 2 of the build row is `o`
        });
    }

    /// Probe the backward table (`inc`): for the predicate `p` and object `obj`, invoke
    /// `emit(s)` for every subject `s` known in `inc[p][obj]`. Mirrors
    /// `inc.get(&p).and_then(|m| m.get(&obj))`.
    #[inline]
    pub fn probe_inc(&self, p: Id, obj: Id, mut emit: impl FnMut(Id)) {
        let probe_row = Row::from_slice(&[p, obj]);
        let keys = inc_keys();
        self.inc_tbl.probe_emit(&[probe_row], &keys, &NoBudget, &mut |build, _probe| {
            emit(build[2]); // col 2 of the inc build row is `s`
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustc_hash::FxHashSet;

    fn triple(s: Id, p: Id, o: Id) -> [Id; 3] {
        [s, p, o]
    }

    /// Smoke test: build from a small triple set and probe both orientations.
    #[test]
    fn build_and_probe_basic() {
        // Predicate p=10 is in `need`; p=99 is not.
        let need: FxHashSet<Id> = [10u32].iter().copied().collect();
        let all: FxHashSet<[Id; 3]> = [
            triple(1, 10, 100),
            triple(1, 10, 101),
            triple(2, 10, 200),
            triple(5, 99, 500), // excluded: p not in need
        ]
        .iter()
        .copied()
        .collect();

        let adj = DeltaAdj::build(&all, &need);

        // Forward probe out[10][1] -> {100, 101}
        let mut fwd: Vec<Id> = Vec::new();
        adj.probe_out(10, 1, |o| fwd.push(o));
        fwd.sort_unstable();
        assert_eq!(fwd, vec![100u32, 101u32], "out[p=10][s=1] must contain both objects");

        // Forward probe out[10][2] -> {200}
        let mut fwd2: Vec<Id> = Vec::new();
        adj.probe_out(10, 2, |o| fwd2.push(o));
        assert_eq!(fwd2, vec![200u32]);

        // Forward probe out[10][99] -> {} (no such subject)
        let mut miss: Vec<Id> = Vec::new();
        adj.probe_out(10, 99, |o| miss.push(o));
        assert!(miss.is_empty(), "probe for absent key must yield nothing");

        // Backward probe inc[10][100] -> {1}
        let mut bwd: Vec<Id> = Vec::new();
        adj.probe_inc(10, 100, |s| bwd.push(s));
        assert_eq!(bwd, vec![1u32]);

        // Backward probe inc[10][200] -> {2}
        let mut bwd2: Vec<Id> = Vec::new();
        adj.probe_inc(10, 200, |s| bwd2.push(s));
        assert_eq!(bwd2, vec![2u32]);

        // Excluded predicate: out[99][5] -> {} (p=99 not in need, never indexed)
        let mut excl: Vec<Id> = Vec::new();
        adj.probe_out(99, 5, |o| excl.push(o));
        assert!(excl.is_empty(), "predicate not in need must not be indexed");
    }

    /// `extend_one` grows the table; subsequent probes see the new entry.
    #[test]
    fn extend_one_is_visible_on_next_probe() {
        let need: FxHashSet<Id> = [7u32].iter().copied().collect();
        let all: FxHashSet<[Id; 3]> = [triple(10, 7, 70)].iter().copied().collect();
        let mut adj = DeltaAdj::build(&all, &need);

        // Before extend: out[7][10] -> {70}, inc[7][70] -> {10}
        let mut before: Vec<Id> = Vec::new();
        adj.probe_out(7, 10, |o| before.push(o));
        assert_eq!(before, vec![70u32]);

        // Extend with a new triple (10, 7, 71)
        adj.extend_one(10, 7, 71);

        // After extend: out[7][10] -> {70, 71} (insertion order)
        let mut after: Vec<Id> = Vec::new();
        adj.probe_out(7, 10, |o| after.push(o));
        assert_eq!(after, vec![70u32, 71u32], "extend_one must append in insertion order");

        // Backward: inc[7][71] -> {10}
        let mut bwd: Vec<Id> = Vec::new();
        adj.probe_inc(7, 71, |s| bwd.push(s));
        assert_eq!(bwd, vec![10u32]);
    }

    /// `rebuild` discards all prior content and reseeds from the given triple set.
    #[test]
    fn rebuild_replaces_all_content() {
        let need: FxHashSet<Id> = [5u32].iter().copied().collect();
        let all: FxHashSet<[Id; 3]> = [triple(1, 5, 50), triple(2, 5, 60)].iter().copied().collect();
        let mut adj = DeltaAdj::build(&all, &need);

        // Sanity: old content present.
        let mut before: Vec<Id> = Vec::new();
        adj.probe_out(5, 1, |o| before.push(o));
        assert!(!before.is_empty());

        // Rebuild with different triples.
        let new_all: FxHashSet<[Id; 3]> = [triple(9, 5, 90)].iter().copied().collect();
        adj.rebuild(&new_all, &need);

        // Old key must be gone.
        let mut old_hit: Vec<Id> = Vec::new();
        adj.probe_out(5, 1, |o| old_hit.push(o));
        assert!(old_hit.is_empty(), "rebuild must clear old entries");

        // New key must be present.
        let mut new_hit: Vec<Id> = Vec::new();
        adj.probe_out(5, 9, |o| new_hit.push(o));
        assert_eq!(new_hit, vec![90u32]);
    }

    /// Equivalence gate: `DeltaAdj` probes return the same values as a plain
    /// `FxHashMap<Id, FxHashMap<Id, Vec<Id>>>` built from the same triple set.
    /// This is the load-bearing cross-assert for the delta-adj seam.
    #[test]
    fn delta_adj_matches_plain_hashmap() {
        use rustc_hash::FxHashMap;

        let need: FxHashSet<Id> = [3u32, 7u32].iter().copied().collect();
        let triples = [
            triple(10, 3, 30),
            triple(10, 3, 31),
            triple(11, 3, 30),
            triple(20, 7, 70),
            triple(20, 7, 71),
            triple(30, 3, 30),
            // predicate 99 not in need
            triple(5, 99, 5),
        ];
        let all: FxHashSet<[Id; 3]> = triples.iter().copied().collect();

        // Build DeltaAdj.
        let mut adj = DeltaAdj::build(&all, &need);

        // Build plain FxHashMap adjacency (mirrors `build_adjacency` in owl.rs).
        let mut plain_out: FxHashMap<Id, FxHashMap<Id, Vec<Id>>> = FxHashMap::default();
        let mut plain_inc: FxHashMap<Id, FxHashMap<Id, Vec<Id>>> = FxHashMap::default();
        for &[s, p, obj] in &all {
            if need.contains(&p) {
                plain_out.entry(p).or_default().entry(s).or_default().push(obj);
                plain_inc.entry(p).or_default().entry(obj).or_default().push(s);
            }
        }

        // Probe every (p, s) combination that exists in the plain map.
        for (&p, inner) in &plain_out {
            for (&s, plain_objs) in inner {
                let mut delta_objs: Vec<Id> = Vec::new();
                adj.probe_out(p, s, |o| delta_objs.push(o));
                // Both sets must be equal (order may differ between the two implementations).
                let mut expected = plain_objs.clone();
                expected.sort_unstable();
                delta_objs.sort_unstable();
                assert_eq!(
                    delta_objs, expected,
                    "out[p={p}][s={s}]: DeltaAdj must match plain FxHashMap"
                );
            }
        }

        // Probe every (p, o) combination in the plain inc map.
        for (&p, inner) in &plain_inc {
            for (&obj, plain_subjs) in inner {
                let mut delta_subjs: Vec<Id> = Vec::new();
                adj.probe_inc(p, obj, |s| delta_subjs.push(s));
                let mut expected = plain_subjs.clone();
                expected.sort_unstable();
                delta_subjs.sort_unstable();
                assert_eq!(
                    delta_subjs, expected,
                    "inc[p={p}][o={obj}]: DeltaAdj must match plain FxHashMap"
                );
            }
        }

        // Also check that absent keys emit nothing.
        let mut miss: Vec<Id> = Vec::new();
        adj.probe_out(3, 999, |o| miss.push(o));
        assert!(miss.is_empty());
        adj.probe_inc(3, 999, |s| miss.push(s));
        assert!(miss.is_empty());

        // Extend with a new triple and check consistency.
        let new_s = 50u32;
        let new_p = 3u32;
        let new_o = 300u32;
        adj.extend_one(new_s, new_p, new_o);
        plain_out.entry(new_p).or_default().entry(new_s).or_default().push(new_o);
        plain_inc.entry(new_p).or_default().entry(new_o).or_default().push(new_s);

        let mut delta_os: Vec<Id> = Vec::new();
        adj.probe_out(new_p, new_s, |o| delta_os.push(o));
        assert_eq!(delta_os, vec![new_o], "extended triple must be probed in forward direction");

        let mut delta_ss: Vec<Id> = Vec::new();
        adj.probe_inc(new_p, new_o, |s| delta_ss.push(s));
        assert_eq!(delta_ss, vec![new_s], "extended triple must be probed in backward direction");
    }
}
