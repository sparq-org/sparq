#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

// sq-ev41x (epic sq-qonbz, umbrella sq-6tykl) — see research/shared-eval-substrate.md.
//
// This crate is the KEYSTONE leaf of the shared-evaluation-substrate move-chain. It defines
// the shared id-tuple vocabulary (`rows`) and the XSD numeric value tower (`numeric`),
// behind DEFAULT-OFF features.
//
// PHASE 2 (sq-ev41x) landed `numeric`: the engine's id-level `Num` / `Dec` value tower +
// `as_numeric` classification + the numeric arithmetic ops (`binop` / `neg` / `abs` /
// rounding) and the XSD lexical helpers were MOVED here verbatim from `sparq-engine::exec`
// and the engine now consumes `sparq_substrate::numeric::{Num, Dec, as_numeric, ...}`.
//
// PHASE 3 (sq-hknqs) has now landed `join`: the FOUR id-tuple join kernels — sorted
// merge-join, radix-partitioned hash-join, index-nested-loop bind-join and leapfrog
// trie-join (WCOJ) — were MOVED here from `sparq-engine::exec`, generalised over a generic
// `JoinKeys` descriptor (the row→key projection + combine layout) and a generic `Budget`
// cooperative-cancel hook. The engine instantiates them with its own `Bindings`-derived key
// columns and its thread-local query budget; a future reasoner can instantiate the SAME
// probe loops with its own key projection — neither pays a vtable. The planner, `Bindings`,
// `LocalVocab` interning and `ScanCmp` filter pushdown stay engine-private.
//
// PHASE 4 (sq-vezew) has now landed `compare`: the SPARQL term TOTAL ORDER — the engine's
// `compare_values` (`ORDER BY` / `<` term ordering: error < blank < IRI < literal < triple,
// numeric-aware + strict typed/temporal + string fallback + recursive triple-term order) was
// MOVED here as the generic `compare::compare_terms`, generalised over a tiny `CompareTerm`
// trait. The engine implements `CompareTerm` for its `Value` (zero-cost wrappers over its
// existing `value_str` / numeric / `value_compare_strict` helpers) and calls the substrate
// algorithm; the `Value` enum, the `LitKind` literal-family classifier and `value_compare_strict`
// stay engine-private (they ALSO drive the relational `<`/`>`/`=` operators, not just ORDER BY,
// so a wholesale `Value` relocation would be non-neutral and sprawling — the correct seam moves
// the ALGORITHM, leaving `Value` engine-resident; see the deferred-remainder note in the PR).
//
// sq-v5evr (un-parked): the VALUE-SPACE RELATIONAL COMPARE — the XPath
// `op:numeric-less-than`/`op:numeric-equal` semantics, where NaN is incomparable (`None`)
// and same-tier pairs compare exactly via the `Dec` tower — has been added to the `numeric`
// module as `Num::cmp_relational`. This is the correct comparison for SPARQL `<`/`>`/`=`
// FILTER expressions, D-entailment value-space equality, and RIF `pred:numeric-equal` /
// `pred:numeric-less-than` builtins. The reasoner's `compare` module now delegates its
// `num_compare` helper to this shared function instead of duplicating the logic.
//
// All moves are behaviour-neutral (the W3C SPARQL conformance floor + the join/scan/BGP/sort
// micro-benches are bit-identical / within noise).
//
// ZERO-OVERHEAD INTENT (the contract every phase keeps): the shareable kernels are FREE
// FUNCTIONS / methods monomorphic over the concrete `Id = u32` and the numeric tiers —
// NEVER `Box<dyn>` / `&dyn` / a vtable on a hot path (research record §2.3, §4). Every
// `numeric` item carries `#[inline]`, so cross-crate inlining (with the workspace LTO
// profile) keeps the engine's FILTER / BIND / ORDER BY hot loops identical to pre-move.

#[cfg(feature = "rows")]
pub mod rows;

#[cfg(feature = "numeric")]
pub mod numeric;

// sq-hknqs (epic sq-qonbz) — Phase 3 of the substrate move-chain. The FOUR id-tuple
// join kernels (merge / hash / bind / leapfrog-trie) now live here, MOVED from
// `sparq-engine::exec`, generic over a `JoinKeys` descriptor + a generic `Budget` hook so the
// engine and a future reasoner share one probe loop with NO `Box<dyn>`/`&dyn`/vtable between a
// join's probe and its key comparison (research/shared-eval-substrate.md §2.3, §4).
#[cfg(feature = "join")]
pub mod join;

// sq-vezew (epic sq-qonbz) — Phase 4 of the substrate move-chain. The SPARQL term
// TOTAL ORDER (`compare_values` — the `ORDER BY` / `<` term ordering: error < blank < IRI <
// literal < triple, numeric-aware + strict typed/temporal + string fallback + recursive
// triple-term order) now lives here, MOVED from `sparq-engine::exec`, generalised over a tiny
// `CompareTerm` trait the engine implements for its `Value`. The algorithm monomorphises (no
// `Box<dyn>`/`&dyn`/vtable); the engine's `Value`/`LitKind`/`value_compare_strict` stay
// engine-private (they also drive the relational operators) and are surfaced through the trait
// (research/shared-eval-substrate.md §2.1, §2.3). A reasoner that orders entailed solutions can
// reuse the same algorithm by implementing `CompareTerm` for its own term type.
#[cfg(feature = "compare")]
pub mod compare;

// sq-atjue — the zero-overhead DELTA harness. This is the substrate half of the
// sparq-engine-systems paper's §8 protocol (`site/papers/sparq-engine-systems.typ`): it
// measures each shared kernel (`join` / `numeric` / `compare`) against a hand-rolled
// pre-extraction equivalent and emits the house JSON envelope with the evidence keys
// `substrate.overhead_<kernel>`. It is a MEASUREMENT of the title-level zero-overhead claim,
// not an assertion of it — the paper claim bends to the data. Behind the DEFAULT-OFF
// `overhead` feature (which implies `join` + `numeric` + `compare`), so the lean core / engine
// / wasm build compiles none of it.
#[cfg(feature = "overhead")]
pub mod overhead;
