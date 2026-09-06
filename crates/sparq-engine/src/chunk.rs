//! Vectorized (columnar / vector-at-a-time) execution primitives — the first
//! concrete building block of the **M4 plan** (`research/optimization-techniques.md`,
//! bead `sq-hvfe`). [OPUS-4.8]
//!
//! The general evaluator (the crate's internal `exec` module) is *row-materialising*: an intermediate
//! result is a `Vec<Row>` where `Row = SmallVec<[Id; 4]>` (one heap-friendly tuple
//! per solution), and a FILTER walks rows one at a time dispatching per cell. The
//! measured scaling loss (`research/optimization-techniques.md`) is exactly that
//! **per-row dispatch + per-row id→value lookup** — a vector engine instead keeps a
//! batch of solutions *column-major* and applies one tight, branch-predictable,
//! auto-vectorisable loop per column.
//!
//! This module provides that columnar intermediate, [`DataChunk`], plus the kernels a
//! vector-at-a-time FILTER needs:
//!
//! * a **decode kernel** ([`DataChunk::decode_numeric_column`]) — the actual SIMD
//!   enabler — that gathers one id column into a *contiguous* `Vec<f64>` value column
//!   **once** (a `f64::NAN` sentinel for a non-numeric cell), with a gather-free fast
//!   path for an all-inline-integer column (the value is in the id). Decoding once is
//!   what turns the per-element cache gather into a straight-line loop the compiler can
//!   auto-vectorise;
//! * a **comparison kernel** ([`DataChunk::select_numeric`]) that scans one column
//!   through the graph's f64 value cache and produces a [`SelVec`] (the indices that
//!   pass) — never materialising a term, mirroring the row evaluator's sargable
//!   numeric pushdown (`ScanCmp::Num` in the internal `exec` module);
//! * a **selection / gather kernel** ([`DataChunk::apply_selection`]) that compacts a
//!   chunk down to the surviving rows, the vector-at-a-time analogue of dropping
//!   filtered rows.
//!
//! It is **opt-in** (the `vectorized` cargo feature) and **OFF by default**: when the
//! feature is off, zero columnar code compiles and the default native + wasm builds
//! are byte-identical. It pulls in **no new dependencies** (only [`sparq_core::dict::Id`]
//! and [`sparq_core::Graph`], both already direct deps). The kernels are
//! `forbid(unsafe_code)`-clean — they are plain index loops the compiler is free to
//! auto-vectorise (`+simd128` on wasm, NEON/AVX on native), not hand-written SIMD.
//!
//! ## Equivalence contract
//!
//! A [`DataChunk`] is a column-major *transpose* of a slice of rows: column `c`,
//! position `r` holds the id that the row representation stores at `row[r][c]`. The
//! round-trip [`DataChunk::from_rows`] → [`DataChunk::to_rows`] preserves the ids and their
//! row/column order (see the tests — it is `==` the input only for `Vec<Id>` rows, since
//! `from_rows` accepts any `AsRef<[Id]>` row while `to_rows` materialises `Vec<Vec<Id>>`),
//! and `select_numeric` selects exactly the rows a row-at-a-time numeric
//! filter over the same column would keep. This is what lets the vector path be wired
//! into the evaluator later (roadmap T1.3 morsel-driven pipelining) without changing
//! query results.

use sparq_core::dict::{is_inline, Id, INLINE_BASE};
use sparq_core::Graph;

/// A numeric comparison operator for the vectorized FILTER kernel. Mirrors the
/// row evaluator's `NumCmp` (`crate::exec`) so a column filtered here keeps exactly
/// the rows the scalar path would. `NaN` (a non-numeric or unparseable cell) never
/// passes any comparison, matching the scalar `f64` operators (`NaN < t`, `NaN == t`,
/// … are all `false`) and the engine's treatment of a non-numeric FILTER operand as a
/// type error → row excluded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VecCmp {
    /// `value > threshold`
    Gt(f64),
    /// `value >= threshold`
    Ge(f64),
    /// `value < threshold`
    Lt(f64),
    /// `value <= threshold`
    Le(f64),
    /// `value == threshold`
    Eq(f64),
}

impl VecCmp {
    /// Tests one f64 against the threshold. `NaN` fails every comparison (the IEEE-754
    /// default for `f64` relational/equality operators).
    #[inline]
    #[must_use]
    pub fn test(self, x: f64) -> bool {
        match self {
            VecCmp::Gt(t) => x > t,
            VecCmp::Ge(t) => x >= t,
            VecCmp::Lt(t) => x < t,
            VecCmp::Le(t) => x <= t,
            VecCmp::Eq(t) => x == t,
        }
    }
}

/// A *selection vector*: the positions (row indices into a [`DataChunk`]) that
/// survived a kernel, in ascending order. This is the canonical vector-engine way to
/// represent a filtered batch without copying the surviving rows out (DuckDB / Velox
/// call it a *selection vector*); the alternative — a packed copy — is
/// [`DataChunk::apply_selection`].
pub type SelVec = Vec<usize>;

/// A column-major batch of id-level solutions: `width` columns, each holding the
/// same number of ids (one per row). The vector-at-a-time analogue of a slice of the
/// row evaluator's `Vec<Row>` — see the module docs for the equivalence contract.
///
/// Invariant: every column has the same length, recorded as [`DataChunk::len`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DataChunk {
    /// `columns[c][r]` is the id bound to column `c` in row `r`.
    columns: Vec<Vec<Id>>,
    /// The number of rows (the shared length of every column).
    len: usize,
}

impl DataChunk {
    /// An empty chunk with `width` columns and no rows.
    #[must_use]
    pub fn empty(width: usize) -> Self {
        DataChunk {
            columns: vec![Vec::new(); width],
            len: 0,
        }
    }

    /// Builds a chunk directly from columns. Returns `None` if the columns are not all
    /// `len` long (the [`DataChunk`] invariant). A zero-column input is the unit chunk
    /// with `len` rows (each an empty tuple) — the representation of a result that has
    /// solutions but binds no variables.
    #[must_use]
    pub fn from_columns(columns: Vec<Vec<Id>>, len: usize) -> Option<Self> {
        if columns.iter().any(|c| c.len() != len) {
            return None;
        }
        Some(DataChunk { columns, len })
    }

    /// Transposes a slice of row-major tuples into a column-major chunk. All rows must
    /// have the same `width`; returns `None` otherwise. The inverse of [`Self::to_rows`].
    ///
    /// Generic over the row representation (`impl AsRef<[Id]>`) so it transposes the engine's
    /// `SmallVec<[Id; 4]>` rows directly — no intermediate `Vec<Id>` per row — as well as plain
    /// `Vec<Id>` rows in tests.
    #[must_use]
    pub fn from_rows<R: AsRef<[Id]>>(rows: &[R], width: usize) -> Option<Self> {
        if rows.iter().any(|r| r.as_ref().len() != width) {
            return None;
        }
        let mut columns: Vec<Vec<Id>> =
            (0..width).map(|_| Vec::with_capacity(rows.len())).collect();
        for row in rows {
            for (c, &id) in row.as_ref().iter().enumerate() {
                columns[c].push(id);
            }
        }
        Some(DataChunk {
            columns,
            len: rows.len(),
        })
    }

    /// Materialises the chunk back into row-major tuples. The inverse of
    /// [`Self::from_rows`] up to the row representation: `to_rows()` of `from_rows(rows, w)`
    /// yields the same ids, in the same row and column order, for any well-formed `rows`.
    /// Since `from_rows` is generic over `AsRef<[Id]>` while this always materialises
    /// `Vec<Vec<Id>>`, the round-trip is `== rows` only when the input rows are themselves
    /// `Vec<Id>`; for another row type (e.g. the engine's `SmallVec<[Id; 4]>`) it is content
    /// and order equality, not `==`.
    #[must_use]
    pub fn to_rows(&self) -> Vec<Vec<Id>> {
        (0..self.len)
            .map(|r| self.columns.iter().map(|col| col[r]).collect())
            .collect()
    }

    /// The number of rows in the chunk.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the chunk has no rows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The number of columns.
    #[must_use]
    pub fn width(&self) -> usize {
        self.columns.len()
    }

    /// Read-only access to column `c` (`None` if out of range).
    #[must_use]
    pub fn column(&self, c: usize) -> Option<&[Id]> {
        self.columns.get(c).map(Vec::as_slice)
    }

    /// **Decode kernel — the actual SIMD enabler.** Gathers id column `c` into a
    /// *contiguous* `Vec<f64>` value column: each id's numeric value where it has one, and
    /// the `f64::NAN` **sentinel** where it does not (a non-numeric / unparseable cell — the
    /// `Graph::numeric_value` cache yields `None`). Returns an empty `Vec` (not a column of
    /// `NaN`) if `c` is out of range, mirroring `select_numeric`.
    ///
    /// Why a separate decode step rather than reusing `select_numeric`: that kernel
    /// re-gathers `numeric_value(id)` per element — a random-access value-cache lookup plus a
    /// branch, the *same* access pattern the row path pays and **not** auto-vectorisable.
    /// Decoding **once** into a contiguous `f64` column lets every subsequent compare / reduce
    /// run over contiguous memory with no per-element cache access, so the compare itself is
    /// auto-vectorisable (NEON/AVX on native, `+simd128` on wasm). It therefore pays only when
    /// the decoded column is *reused* by ≥1 downstream kernel, or when the column is inline-int
    /// (below).
    ///
    /// The **NaN sentinel** is chosen so the decoded column drops straight into the numeric
    /// kernels with no separate validity bitmap: `VecCmp::test` already rejects `NaN` under
    /// every operator (IEEE-754), so a non-numeric cell is excluded at the compare step exactly
    /// as the row FILTER's type-error path excludes it.
    ///
    /// **Inline-integer gather-free fast path.** An `xsd:integer` in the inline range carries
    /// its value in the id itself (`numeric_value` returns `id - INLINE_BASE` with no lookup).
    /// When *every* id in the column is inline, the decode is a straight-line
    /// `id - INLINE_BASE` map with no cache access and no per-cell numeric branch — the
    /// portable, wasm-friendly best case. Its result is bit-identical to the general path (an
    /// all-inline column contains no `NaN` sentinel), so it is a pure speed specialisation.
    #[must_use]
    pub fn decode_numeric_column(&self, graph: &Graph, c: usize) -> Vec<f64> {
        let Some(col) = self.columns.get(c) else {
            return Vec::new();
        };
        // Fast path: an all-inline-integer column decodes gather-free — the value is in the id,
        // so this stays a branchless straight-line loop the compiler is free to vectorise. The
        // `is_inline` scan is itself a tight branch-predictable pass over the contiguous column.
        if col.iter().all(|&id| is_inline(id)) {
            return col.iter().map(|&id| f64::from(id - INLINE_BASE)).collect();
        }
        // General path: one gather through the value cache; a non-numeric id becomes the NaN
        // sentinel so `VecCmp` (and later reducers) exclude it without a side validity mask.
        col.iter()
            .map(|&id| graph.numeric_value(id).unwrap_or(f64::NAN))
            .collect()
    }

    /// **Vectorized comparison kernel.** Scans column `c` through the graph's f64 value
    /// cache and returns the [`SelVec`] of row positions whose value passes `cmp`. A
    /// non-numeric / unparseable cell (the cache yields `None`) never passes — the row
    /// evaluator's `ScanCmp::Num` semantics (a FILTER type error excludes the row).
    /// Returns an empty [`SelVec`] if `c` is out of range.
    ///
    /// This is one tight loop over a contiguous column (the compiler is free to
    /// auto-vectorise the value test), versus the row path's per-row dispatch — the
    /// per-row-overhead win the M4 plan targets.
    #[must_use]
    pub fn select_numeric(&self, graph: &Graph, c: usize, cmp: VecCmp) -> SelVec {
        let Some(col) = self.columns.get(c) else {
            return SelVec::new();
        };
        let mut sel = SelVec::new();
        for (r, &id) in col.iter().enumerate() {
            if graph.numeric_value(id).is_some_and(|x| cmp.test(x)) {
                sel.push(r);
            }
        }
        sel
    }

    /// **Vectorized compare over a pre-decoded column — the auto-vectorising kernel.**
    /// Given a contiguous value column `decoded` (as produced by
    /// [`Self::decode_numeric_column`]), returns the ascending [`SelVec`] of positions whose
    /// value passes `cmp`. Unlike [`Self::select_numeric`], this does **no** per-element cache
    /// gather — it is one contiguous-memory, auto-vectorisable compare over `f64` (NEON/AVX on
    /// native, `+simd128` on wasm). The compare itself is what vectorises; appending a passing
    /// position stays data-dependent, so the loop is *not* branchless. It is the
    /// "select over the decoded column" step: decode **once**, then compare (and, later, reuse
    /// the same decoded column for a reducer aggregate) without re-touching the value cache.
    ///
    /// The `f64::NAN` sentinel a non-numeric cell decodes to never passes any comparison
    /// (IEEE-754), so a non-numeric / unbound cell is excluded here exactly as the row FILTER's
    /// type-error path excludes it — the selection is identical to
    /// `select_numeric` over the same column (see the `decode_then_compare_matches_select_numeric`
    /// test).
    #[must_use]
    pub fn select_decoded(decoded: &[f64], cmp: VecCmp) -> SelVec {
        let mut sel = SelVec::with_capacity(decoded.len());
        for (r, &x) in decoded.iter().enumerate() {
            if cmp.test(x) {
                sel.push(r);
            }
        }
        sel
    }

    /// **Selection / gather kernel.** Returns a new chunk containing only the rows named
    /// by `sel` (ascending, in-range positions — as produced by [`Self::select_numeric`]).
    /// Out-of-range positions are skipped. The width is preserved; this is the
    /// vector-at-a-time analogue of dropping filtered rows.
    #[must_use]
    pub fn apply_selection(&self, sel: &[usize]) -> DataChunk {
        let columns: Vec<Vec<Id>> = self
            .columns
            .iter()
            .map(|col| {
                sel.iter()
                    .filter_map(|&r| col.get(r).copied())
                    .collect::<Vec<Id>>()
            })
            .collect();
        let len = columns.first().map_or_else(
            // Zero-column chunk: the row count is the count of in-range selections.
            || sel.iter().filter(|&&r| r < self.len).count(),
            Vec::len,
        );
        DataChunk { columns, len }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparq_core::Graph;

    /// A graph whose objects are the integer literals `0..n` (subject `ex:s{i}`,
    /// predicate `ex:age`), and the parallel list of object ids in value order, so a
    /// chunk built from the object column can be filtered numerically and checked
    /// against a hand-computed expectation.
    fn ages_graph(n: usize) -> (Graph, Vec<Id>) {
        let mut nt = String::new();
        for i in 0..n {
            nt.push_str(&format!(
                "<http://ex/s{i}> <http://ex/age> \"{i}\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n"
            ));
        }
        let g = Graph::load_str(&nt, "ntriples").unwrap();
        let obj_ids: Vec<Id> = (0..n)
            .map(|i| {
                let lit = oxrdf::Literal::new_typed_literal(
                    i.to_string(),
                    oxrdf::NamedNode::new("http://www.w3.org/2001/XMLSchema#integer").unwrap(),
                );
                g.id_of(&oxrdf::Term::Literal(lit))
                    .expect("integer literal interned")
            })
            .collect();
        (g, obj_ids)
    }

    /// A graph whose object column spans the three decode cases in one column:
    /// **inline** integers (`5`, `9`), **non-inline cache-backed** numerics (a negative
    /// integer `-7` and a decimal `3.5`, neither of which inlines), and **non-numeric**
    /// terms (an IRI and a plain-string literal). Returns the object-id column in subject
    /// order — the mixed input the decode primitive must handle.
    fn mixed_column() -> (Graph, Vec<Id>) {
        let nt = concat!(
            "<http://ex/s0> <http://ex/p> \"5\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
            "<http://ex/s1> <http://ex/p> \"9\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
            "<http://ex/s2> <http://ex/p> \"-7\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
            "<http://ex/s3> <http://ex/p> \"3.5\"^^<http://www.w3.org/2001/XMLSchema#decimal> .\n",
            "<http://ex/s4> <http://ex/p> <http://ex/thing> .\n",
            "<http://ex/s5> <http://ex/p> \"hello\" .\n",
        );
        let g = Graph::load_str(nt, "ntriples").unwrap();
        let int = |v: &str| {
            oxrdf::Term::Literal(oxrdf::Literal::new_typed_literal(
                v,
                oxrdf::NamedNode::new("http://www.w3.org/2001/XMLSchema#integer").unwrap(),
            ))
        };
        let terms = [
            int("5"),
            int("9"),
            int("-7"),
            oxrdf::Term::Literal(oxrdf::Literal::new_typed_literal(
                "3.5",
                oxrdf::NamedNode::new("http://www.w3.org/2001/XMLSchema#decimal").unwrap(),
            )),
            oxrdf::Term::NamedNode(oxrdf::NamedNode::new("http://ex/thing").unwrap()),
            oxrdf::Term::Literal(oxrdf::Literal::new_simple_literal("hello")),
        ];
        let ids: Vec<Id> = terms
            .iter()
            .map(|t| g.id_of(t).expect("term interned"))
            .collect();
        (g, ids)
    }

    #[test]
    fn from_columns_rejects_ragged() {
        assert!(DataChunk::from_columns(vec![vec![1, 2], vec![3]], 2).is_none());
        let c = DataChunk::from_columns(vec![vec![1, 2], vec![3, 4]], 2).unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!(c.width(), 2);
    }

    #[test]
    fn from_rows_rejects_ragged() {
        assert!(DataChunk::from_rows(&[vec![1, 2], vec![3]], 2).is_none());
    }

    #[test]
    fn rows_roundtrip_is_identity() {
        let rows = vec![vec![10, 20, 30], vec![11, 21, 31], vec![12, 22, 32]];
        let chunk = DataChunk::from_rows(&rows, 3).unwrap();
        assert_eq!(chunk.len(), 3);
        assert_eq!(chunk.width(), 3);
        assert_eq!(chunk.column(1).unwrap(), &[20, 21, 22]);
        assert_eq!(chunk.to_rows(), rows);
    }

    #[test]
    fn empty_and_zero_width() {
        let e = DataChunk::empty(2);
        assert!(e.is_empty());
        assert_eq!(e.width(), 2);
        assert_eq!(e.to_rows(), Vec::<Vec<Id>>::new());

        // Zero-column unit chunk: rows that bind no variables.
        let unit = DataChunk::from_columns(vec![], 3).unwrap();
        assert_eq!(unit.len(), 3);
        assert_eq!(unit.width(), 0);
        // [OPUS-4.8] (sq-xx3u) Annotate the expected value as `Vec<Vec<Id>>`. The inner
        // `vec![]` literals carry no elements to pin their type, and once `serde_json`
        // unifies into the test build's dependency graph (3+ features, e.g.
        // `service`/`serialize-rdf`), its `impl PartialEq<serde_json::Value> for u32`
        // makes the `Id: PartialEq<_>` resolution ambiguous (E0282/E0283). The explicit
        // type fixes the element to `Id` regardless of which `PartialEq` impls are in scope.
        let expected: Vec<Vec<Id>> = vec![vec![], vec![], vec![]];
        assert_eq!(unit.to_rows(), expected);
    }

    #[test]
    fn veccmp_nan_never_passes() {
        for cmp in [
            VecCmp::Gt(0.0),
            VecCmp::Ge(0.0),
            VecCmp::Lt(0.0),
            VecCmp::Le(0.0),
            VecCmp::Eq(0.0),
        ] {
            assert!(!cmp.test(f64::NAN), "{cmp:?} should reject NaN");
        }
    }

    #[test]
    fn select_numeric_matches_scalar_semantics() {
        let n = 64;
        let (g, obj_ids) = ages_graph(n);
        // One-column chunk over the object ids (the integer values 0..n).
        let chunk = DataChunk::from_columns(vec![obj_ids.clone()], n).unwrap();

        for cmp in [
            VecCmp::Gt(30.0),
            VecCmp::Ge(30.0),
            VecCmp::Lt(10.0),
            VecCmp::Le(10.0),
            VecCmp::Eq(42.0),
        ] {
            let sel = chunk.select_numeric(&g, 0, cmp);
            // Scalar reference: the exact rows a row-at-a-time numeric FILTER keeps.
            let expected: Vec<usize> = (0..n)
                .filter(|&r| g.numeric_value(obj_ids[r]).is_some_and(|x| cmp.test(x)))
                .collect();
            assert_eq!(sel, expected, "selection mismatch for {cmp:?}");

            // Gather, then verify every surviving value passes and the count is right.
            let filtered = chunk.apply_selection(&sel);
            assert_eq!(filtered.len(), expected.len());
            for &id in filtered.column(0).unwrap() {
                assert!(g.numeric_value(id).is_some_and(|x| cmp.test(x)));
            }
        }
    }

    #[test]
    fn decode_numeric_column_matches_scalar_mixed_and_nan_sentinel() {
        let (g, ids) = mixed_column();
        let n = ids.len();
        let chunk = DataChunk::from_columns(vec![ids.clone()], n).unwrap();
        // Mixed column => the general (gather) path runs, not the inline fast path.
        assert!(!ids.iter().all(|&id| is_inline(id)), "column must be mixed");

        let decoded = chunk.decode_numeric_column(&g, 0);
        assert_eq!(decoded.len(), n);

        // (1) Bit-exact against the scalar decode the row path computes per id
        //     (`.to_bits()` compares the NaN sentinel too — both paths use `f64::NAN`).
        for (r, &id) in ids.iter().enumerate() {
            let scalar = g.numeric_value(id).unwrap_or(f64::NAN);
            assert_eq!(
                decoded[r].to_bits(),
                scalar.to_bits(),
                "decode[{}] must equal scalar numeric_value",
                r
            );
        }

        // (2) Pin the concrete numeric values (inline + cache-backed).
        assert_eq!(decoded[0], 5.0);
        assert_eq!(decoded[1], 9.0);
        assert_eq!(decoded[2], -7.0);
        assert_eq!(decoded[3], 3.5);

        // (3) Non-vacuous sentinel check: the two non-numeric cells decode to the NaN
        //     sentinel, NOT a spurious value. A wrong sentinel (e.g. 0.0) would (a) not be
        //     NaN and (b) spuriously PASS `>= 0.0` — so each assert independently catches it.
        for r in [4usize, 5] {
            assert!(
                decoded[r].is_nan(),
                "non-numeric cell {} must be the NaN sentinel",
                r
            );
            assert!(
                !VecCmp::Ge(0.0).test(decoded[r]),
                "NaN sentinel at {} must not pass any comparison",
                r
            );
        }
    }

    #[test]
    fn decode_numeric_column_inline_fast_path_is_exact() {
        // An all-inline-integer column exercises the gather-free fast path; its output
        // must be bit-identical to the general (scalar) path and carry no NaN.
        let (g, obj_ids) = ages_graph(48);
        assert!(
            obj_ids.iter().all(|&id| is_inline(id)),
            "0..n are canonical non-negative integers => inline ids (fast-path precondition)"
        );
        let chunk = DataChunk::from_columns(vec![obj_ids.clone()], obj_ids.len()).unwrap();
        let decoded = chunk.decode_numeric_column(&g, 0);

        let expected: Vec<f64> = (0..48u32).map(f64::from).collect();
        assert_eq!(decoded, expected);
        assert!(
            decoded.iter().all(|x| !x.is_nan()),
            "no NaN in an all-inline column"
        );
        for (r, &id) in obj_ids.iter().enumerate() {
            assert_eq!(
                decoded[r].to_bits(),
                g.numeric_value(id).unwrap().to_bits(),
                "fast path must match the scalar path at {}",
                r
            );
        }
    }

    #[test]
    fn decode_then_compare_matches_select_numeric() {
        // The decoded column + `VecCmp` must select exactly the rows the row-shaped
        // `select_numeric` kernel keeps — proving the NaN sentinel yields the same
        // filtering (non-numeric cells excluded) as the direct per-id gather.
        let (g, ids) = mixed_column();
        let n = ids.len();
        let chunk = DataChunk::from_columns(vec![ids], n).unwrap();
        let decoded = chunk.decode_numeric_column(&g, 0);
        for cmp in [
            VecCmp::Gt(0.0),
            VecCmp::Ge(-7.0),
            VecCmp::Lt(5.0),
            VecCmp::Le(3.5),
            VecCmp::Eq(9.0),
        ] {
            let via_decode: SelVec = (0..n).filter(|&r| cmp.test(decoded[r])).collect();
            let via_kernel = chunk.select_numeric(&g, 0, cmp);
            // The `select_decoded` kernel (compare over the pre-decoded column) must agree
            // with both the hand loop and the direct `select_numeric` gather.
            let via_select_decoded = DataChunk::select_decoded(&decoded, cmp);
            assert_eq!(
                via_decode, via_kernel,
                "decode+compare != select_numeric for {:?}",
                cmp
            );
            assert_eq!(
                via_select_decoded, via_kernel,
                "select_decoded != select_numeric for {:?}",
                cmp
            );
        }
    }

    #[test]
    fn select_decoded_rejects_nan_sentinel_and_is_ascending() {
        // A hand-built decoded column mixing real values with the NaN sentinel: the kernel
        // keeps exactly the passing NON-NaN positions, ascending, and never the NaN cells.
        let decoded = [1.0, f64::NAN, 5.0, 5.0, f64::NAN, 9.0];
        assert_eq!(DataChunk::select_decoded(&decoded, VecCmp::Ge(5.0)), vec![2, 3, 5]);
        assert_eq!(DataChunk::select_decoded(&decoded, VecCmp::Eq(5.0)), vec![2, 3]);
        assert_eq!(DataChunk::select_decoded(&decoded, VecCmp::Lt(5.0)), vec![0]);
        // No comparison ever selects a NaN-sentinel position (1 and 4).
        for cmp in [VecCmp::Gt(-1e18), VecCmp::Ge(-1e18), VecCmp::Le(1e18)] {
            let sel = DataChunk::select_decoded(&decoded, cmp);
            assert!(!sel.contains(&1) && !sel.contains(&4), "{cmp:?} must skip NaN cells");
        }
    }

    #[test]
    fn decode_numeric_column_out_of_range_is_empty() {
        let (g, _ids) = ages_graph(4);
        let chunk = DataChunk::empty(1);
        assert!(chunk.decode_numeric_column(&g, 5).is_empty());
    }

    #[test]
    fn select_numeric_out_of_range_column_is_empty() {
        let (g, _ids) = ages_graph(4);
        let chunk = DataChunk::empty(1);
        assert!(chunk.select_numeric(&g, 5, VecCmp::Gt(0.0)).is_empty());
    }

    #[test]
    fn apply_selection_preserves_columns_and_skips_out_of_range() {
        let rows = vec![vec![1, 100], vec![2, 200], vec![3, 300]];
        let chunk = DataChunk::from_rows(&rows, 2).unwrap();
        // Keep rows 0 and 2; an out-of-range index (9) is ignored.
        let out = chunk.apply_selection(&[0, 2, 9]);
        assert_eq!(out.to_rows(), vec![vec![1, 100], vec![3, 300]]);
    }

    #[test]
    fn apply_selection_on_zero_width_counts_in_range() {
        let unit = DataChunk::from_columns(vec![], 3).unwrap();
        let out = unit.apply_selection(&[0, 2, 9]); // 9 is out of range
        assert_eq!(out.width(), 0);
        assert_eq!(out.len(), 2);
    }
}
