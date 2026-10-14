//! Criterion micro-benches for `sparq-substrate`: the four join kernels and the
//! `Num` arithmetic tower.
//!
//! Enabled by `--features join,numeric` (which imply `rows`). The benches measure
//! throughput shapes; **no hard-coded performance numbers are expected** — the bench
//! records measurements to `target/criterion/` for local comparison with Criterion's
//! own baseline mechanism. Do not bake numbers into documentation or tests.
//!
//! # sq-qonbz.4 — epic sq-qonbz
//!
//! Surfaces covered:
//! - `join::merge_join` — sorted merge join on a single shared key column.
//! - `join::build_table` + `join::hash_probe_serial` — hash build and serial
//!   hash probe over the built `FxHashMap`.
//! - `numeric::Num::binop` — `+` / `*` arithmetic on the `Int` / `Dec` / `Double`
//!   tiers.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
// criterion 0.8 deprecated its own black_box in favour of std::hint::black_box.
use std::hint::black_box;
use sparq_substrate::{
    join::{build_table, hash_probe_serial, lftj_recurse, merge_join, JoinKeys, NoBudget, Trie, TrieIter},
    numeric::{ArithOp, Dec, Num},
    rows::{Row, NO_ID},
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build two sorted `Row` slices of `n` rows each, joined on column 0 (every
/// key is unique, so the output is exactly `n` combined rows).
fn sorted_rows(n: u32) -> (Vec<Row>, Vec<Row>) {
    let left: Vec<Row> = (0..n)
        .map(|i| {
            let mut r = Row::new();
            r.extend_from_slice(&[i, i + 1000]);
            r
        })
        .collect();
    let right: Vec<Row> = (0..n)
        .map(|i| {
            let mut r = Row::new();
            r.extend_from_slice(&[i, i + 2000]);
            r
        })
        .collect();
    (left, right)
}

/// `JoinKeys` descriptor: single equi-join key on column 0; append right col 1.
fn key_col0() -> JoinKeys {
    JoinKeys { key_cols: vec![(0, 0)], right_only: vec![1] }
}

// ---------------------------------------------------------------------------
// Bench: merge join
// ---------------------------------------------------------------------------

fn bench_merge_join(c: &mut Criterion) {
    let mut group = c.benchmark_group("merge_join");
    for n in [100u32, 1_000, 10_000] {
        let (left, right) = sorted_rows(n);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            let mut out: Vec<Row> = Vec::with_capacity(n as usize);
            b.iter(|| {
                out.clear();
                // lk=0, rk=0, no extra shared columns, append right col 1
                merge_join(&left, 0, &right, 0, &[], &[1], &NoBudget, &mut out);
                assert_eq!(out.len(), n as usize);
            });
        });
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// Bench: hash build + probe
// ---------------------------------------------------------------------------

fn bench_hash_build(c: &mut Criterion) {
    let mut group = c.benchmark_group("hash_build");
    for n in [100u32, 1_000, 10_000] {
        let (left, _) = sorted_rows(n);
        let keys = key_col0();
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| {
                // black_box prevents LLVM from eliding the build entirely
                // when the result would otherwise be dropped without being observed.
                black_box(build_table(&left, &keys));
            });
        });
    }
    group.finish();
}

fn bench_hash_probe(c: &mut Criterion) {
    let mut group = c.benchmark_group("hash_probe");
    for n in [100u32, 1_000, 10_000] {
        let (left, right) = sorted_rows(n);
        let keys = key_col0();
        let table = build_table(&left, &keys);
        // hash_probe_serial takes &[FxHashMap<Key,Posting>]; wrap in a one-element slice.
        let tables = std::slice::from_ref(&table);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            let mut out: Vec<Row> = Vec::with_capacity(n as usize);
            b.iter(|| {
                out.clear();
                hash_probe_serial(&right, &keys, &left, tables, &[1], &NoBudget, &mut out);
                assert_eq!(out.len(), n as usize);
            });
        });
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// Bench: Num arithmetic
// ---------------------------------------------------------------------------

fn bench_num_arithmetic(c: &mut Criterion) {
    let mut group = c.benchmark_group("num_arithmetic");

    // Integer addition: `Num::Int(i) + Num::Int(j)`
    {
        let vals: Vec<(Num, Num)> = (0i64..1_000)
            .map(|i| (Num::Int(i), Num::Int(i + 1)))
            .collect();
        group.throughput(Throughput::Elements(vals.len() as u64));
        group.bench_function("int_add", |b| {
            b.iter(|| {
                let mut acc: i64 = 0;
                for &(a, bv) in &vals {
                    if let Some(Num::Int(v)) = a.binop(bv, ArithOp::Add) {
                        acc = acc.wrapping_add(v);
                    }
                }
                acc
            });
        });
    }

    // Decimal multiplication: `Dec * Dec`
    {
        let vals: Vec<(Num, Num)> = (1i128..=1_000)
            .map(|i| {
                (
                    Num::Dec(Dec { mant: i, scale: 2 }),
                    Num::Dec(Dec { mant: i + 1, scale: 2 }),
                )
            })
            .collect();
        group.throughput(Throughput::Elements(vals.len() as u64));
        group.bench_function("dec_mul", |b| {
            b.iter(|| {
                let mut found = 0usize;
                for &(a, bv) in &vals {
                    if a.binop(bv, ArithOp::Mul).is_some() {
                        found += 1;
                    }
                }
                found
            });
        });
    }

    // Double addition: `f64 + f64` through the `Num` tower
    {
        let vals: Vec<(Num, Num)> = (0..1_000)
            .map(|i| (Num::Double(i as f64 * 0.1), Num::Double(i as f64 * 0.01)))
            .collect();
        group.throughput(Throughput::Elements(vals.len() as u64));
        group.bench_function("double_add", |b| {
            b.iter(|| {
                let mut acc = 0.0f64;
                for &(a, bv) in &vals {
                    if let Some(Num::Double(v)) = a.binop(bv, ArithOp::Add) {
                        acc += v;
                    }
                }
                acc
            });
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Bench: LFTJ triangle query (k=2 per level, classic cyclic WCO path)
// ---------------------------------------------------------------------------
//
// Triangle query: edge(a,b) ∧ edge(b,c) ∧ edge(a,c) on a complete graph K_n.
// At each of the 3 variable levels (a, b, c) exactly two tries participate (k=2),
// exercising the generic leapfrog path.  The result set has C(n,3) = n(n-1)(n-2)/6
// rows; the measurement captures LFTJ hot-loop throughput on a cyclic pattern.
//
// Variable order: [a=0, b=1, c=2].  Tries:
//   0 = R(a,b): tuples (a,b) for all 0 ≤ a < b < n, sorted (a,b).
//   1 = S(b,c): tuples (b,c) for all 0 ≤ b < c < n, sorted (b,c).
//   2 = T(a,c): tuples (a,c) for all 0 ≤ a < c < n, sorted (a,c).
// parts_at_level: level 0 → [0,2]; level 1 → [0,1]; level 2 → [1,2].
//
// sq-7d3dj.20

fn bench_lftj_triangle(c: &mut Criterion) {
    let mut group = c.benchmark_group("lftj_triangle");
    for n in [10u32, 30, 100] {
        // Complete-graph edges: (i,j) for all 0 ≤ i < j < n, sorted lexicographically.
        let edges: Vec<(u32, u32)> =
            (0..n).flat_map(|i| ((i + 1)..n).map(move |j| (i, j))).collect();
        let r0 = Trie { tuples: edges.iter().map(|&(a, b)| vec![a, b]).collect() };
        let r1 = Trie { tuples: edges.iter().map(|&(b, c)| vec![b, c]).collect() };
        let r2 = Trie { tuples: edges.iter().map(|&(a, c)| vec![a, c]).collect() };
        // level 0 (?a): tries 0 and 2; level 1 (?b): tries 0 and 1; level 2 (?c): tries 1 and 2.
        let parts_at_level = vec![vec![0usize, 2], vec![0usize, 1], vec![1usize, 2]];
        let expected = n * (n.saturating_sub(1)) * (n.saturating_sub(2)) / 6;
        group.throughput(Throughput::Elements(expected as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| {
                let mut iters =
                    vec![TrieIter::new(&r0), TrieIter::new(&r1), TrieIter::new(&r2)];
                let mut current = vec![NO_ID; 3];
                let mut out = Vec::new();
                lftj_recurse(
                    &mut iters,
                    &parts_at_level,
                    0,
                    3,
                    &mut current,
                    &NoBudget,
                    &mut out,
                );
                black_box(out.len())
            });
        });
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// Bench: LFTJ 3-way set intersection (k=3, monomorphized search_k3 path)
// ---------------------------------------------------------------------------
//
// Query: R(x) ∧ S(x) ∧ T(x) — all three tries participate at level 0 (k=3),
// routing through the monomorphized search_k3 fast path.  Three sets with
// controlled overlap: even values (R), multiples-of-3 (S), multiples-of-5 (T)
// in 0..n.  The intersection is multiples of lcm(2,3,5)=30 — easy to verify.
//
// sq-7d3dj.20

fn bench_lftj_intersection_k3(c: &mut Criterion) {
    let mut group = c.benchmark_group("lftj_intersection_k3");
    for n in [1_000u32, 10_000, 100_000] {
        let r = Trie { tuples: (0..n).step_by(2).map(|v| vec![v]).collect() };
        let s = Trie { tuples: (0..n).step_by(3).map(|v| vec![v]).collect() };
        let t = Trie { tuples: (0..n).step_by(5).map(|v| vec![v]).collect() };
        // All three tries participate at level 0: k=3.
        let parts_at_level = vec![vec![0usize, 1, 2]];
        // multiples of lcm(2,3,5)=30 in 0..n — 0 IS included, so the count is
        // ceil(n/30), not floor.  E.g. n=1000 → {0,30,...,990} = 34 values.
        // off-by-one fix sq-7d3dj.20
        let expected = n.div_ceil(30);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| {
                let mut iters =
                    vec![TrieIter::new(&r), TrieIter::new(&s), TrieIter::new(&t)];
                let mut current = vec![NO_ID; 1];
                let mut out = Vec::new();
                lftj_recurse(
                    &mut iters,
                    &parts_at_level,
                    0,
                    1,
                    &mut current,
                    &NoBudget,
                    &mut out,
                );
                // Light sanity check so the result is observed (prevent elision).
                debug_assert_eq!(out.len() as u32, expected);
                black_box(out.len())
            });
        });
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

criterion_group!(join_benches, bench_merge_join, bench_hash_build, bench_hash_probe, bench_lftj_triangle, bench_lftj_intersection_k3);
criterion_group!(numeric_benches, bench_num_arithmetic);
criterion_main!(join_benches, numeric_benches);
