//! Codegen-audit probe (sq-98w7z.5 / issue #3079): `#[no_mangle] #[inline(never)]`
//! wrappers around the audited hot functions so their **post-LTO release** assembly can
//! be located by symbol name in the emitted `.s` file. The root release profile uses
//! fat LTO + codegen-units=1, so per-rlib asm would NOT be the shipped codegen — the
//! only honest place to read it is after the final LTO link, which is what this bin's
//! `cargo rustc --release -- --emit asm` output is.
//!
//! The wrappers force each audited function to be codegen'd with its real cross-crate
//! inlining context (e.g. `Graph::numeric_value` inlined into the decode gather,
//! `key_hash` + `raw_entry().from_hash` inlined into the probe). `main` calls each one
//! through `black_box` so LTO cannot internalise-and-drop them.
//!
//! Audit-only: this crate is never a workspace member and nothing ships from it.

use std::hint::black_box;

use sparq_core::dict::{Dict, Id};
use sparq_core::Graph;
use sparq_engine::{DataChunk, SelVec, VecCmp};
use sparq_substrate::join::{probe_emit, probe_gather_indices, JoinKeys, JoinTable};
use sparq_substrate::rows::Row;

// ---- chunk.rs kernels (sparq-engine) — the "compiler is free to auto-vectorise" claims

#[no_mangle]
#[inline(never)]
pub fn audit_select_decoded(decoded: &[f64], cmp: VecCmp) -> SelVec {
    DataChunk::select_decoded(decoded, cmp)
}

#[no_mangle]
#[inline(never)]
pub fn audit_decode_numeric_column(chunk: &DataChunk, graph: &Graph, c: usize) -> Vec<f64> {
    chunk.decode_numeric_column(graph, c)
}

#[no_mangle]
#[inline(never)]
pub fn audit_select_numeric(chunk: &DataChunk, graph: &Graph, c: usize, cmp: VecCmp) -> SelVec {
    chunk.select_numeric(graph, c, cmp)
}

// ---- join.rs raw-hash probe path (sparq-substrate) — bounds-check-elision claims

#[no_mangle]
#[inline(never)]
pub fn audit_probe_emit(
    prow: &Row,
    keys: &JoinKeys,
    build: &[Row],
    tables: &[JoinTable],
    probe_only: &[usize],
    out: &mut Vec<Row>,
) {
    probe_emit(prow, keys, build, tables, probe_only, out);
}

#[no_mangle]
#[inline(never)]
pub fn audit_probe_gather_indices(
    prow: &Row,
    keys: &JoinKeys,
    tables: &[JoinTable],
    build_indices: &mut Vec<usize>,
) {
    probe_gather_indices(prow, keys, tables, build_indices);
}

// ---- dict find_iri memcmp path (sparq-core) — private fn, reached via its only
// public hot caller `Dict::intern_iri` (hash → find_iri → stored_is_iri compare).

#[no_mangle]
#[inline(never)]
pub fn audit_intern_iri(dict: &mut Dict, iri: &str) -> Id {
    dict.intern_iri(iri)
}

fn main() {
    // Exercise every wrapper through black_box so fat LTO keeps them all.
    let decoded: Vec<f64> = black_box(vec![1.0, 2.0, 3.0]);
    black_box(audit_select_decoded(black_box(&decoded), black_box(VecCmp::Lt(2.5))));

    let graph = Graph::new();
    let chunk = DataChunk::from_columns(vec![vec![1, 2, 3]], 3).expect("well-formed chunk");
    black_box(audit_decode_numeric_column(black_box(&chunk), black_box(&graph), black_box(0)));
    black_box(audit_select_numeric(
        black_box(&chunk),
        black_box(&graph),
        black_box(0),
        black_box(VecCmp::Gt(0.0)),
    ));

    let prow: Row = black_box(Row::from_slice(&[1, 2]));
    let keys = JoinKeys { key_cols: vec![(0, 0)], right_only: vec![1] };
    let build: Vec<Row> = vec![Row::from_slice(&[1, 9])];
    let tables: Vec<JoinTable> = vec![JoinTable::default()];
    let mut out: Vec<Row> = Vec::new();
    audit_probe_emit(&prow, black_box(&keys), &build, black_box(&tables), &[1], &mut out);
    black_box(&out);
    let mut idxs: Vec<usize> = Vec::new();
    audit_probe_gather_indices(&prow, black_box(&keys), black_box(&tables), &mut idxs);
    black_box(&idxs);

    let mut dict = Dict::new();
    black_box(audit_intern_iri(black_box(&mut dict), black_box("http://example.org/s1")));
}
