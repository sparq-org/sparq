//! Triple store: the six sorted permutation indexes over dictionary-encoded
//! triples (Hexastore / RDF-3X / QLever design).
//!
//! Storing all six orderings (SPO SOP PSO POS OSP OPS) means every triple
//! pattern is answered by a single contiguous range (binary search on the
//! bound prefix), and the scan output is sorted by the remaining positions —
//! which is exactly what merge joins need. M1 holds each permutation as a
//! sorted `Vec<[Id; 3]>`; later milestones replace these with block-compressed,
//! optionally memory-mapped columns.

use crate::dict::Id;
#[cfg(feature = "parallel")]
use rayon::prelude::*;

/// The six permutations. Each names the order of (subject, predicate, object)
/// columns as stored.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Perm {
    Spo,
    Sop,
    Pso,
    Pos,
    Osp,
    Ops,
}

/// The permutations actually built and searched. The full six give every triple
/// pattern a sorted scan in the order any merge join wants. The `compact-index` set
/// {SPO, POS, OSP} still answers EVERY triple pattern from one index (SPO→S*/SP*,
/// POS→P*/PO*, OSP→O*/OS*) at half the memory, at the cost of some merge joins (and
/// some lazy-count fast paths) falling back to hashing / sorting.
// Compact set on wasm ALWAYS (memory-bound target), or on native opt-in via the
// `compact-index` feature (for testing). Keyed on `target_arch` — NOT just a feature —
// so the wasm choice does not leak to the native build via Cargo feature unification.
#[cfg(not(any(target_arch = "wasm32", feature = "compact-index")))]
pub const BUILT: &[Perm] = &[Perm::Spo, Perm::Sop, Perm::Pso, Perm::Pos, Perm::Osp, Perm::Ops];
#[cfg(any(target_arch = "wasm32", feature = "compact-index"))]
pub const BUILT: &[Perm] = &[Perm::Spo, Perm::Pos, Perm::Osp];

impl Perm {
