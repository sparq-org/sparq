//! SPARQL window functions (sq-5qz9) — a NON-STANDARD, OPT-IN extension.
//!
//! [OPUS-4.8] **There is no W3C-REC syntax for SPARQL window functions.** SPARQL
//! 1.1 has no `OVER (PARTITION BY … ORDER BY …)` clause, and no SPARQL-community
//! SEP defines one. Engines that offer windowing (Stardog's `WINDOW`/`OVER`,
//! AnzoGraph, GraphDB's ranking extensions) each invent their own surface. So
//! rather than smuggle a non-standard `OVER` clause into the (W3C-conformance
//! tracking) vendored parser, this module exposes window functions as a
//! **programmatic pass over a [`QueryResult`]** whose semantics follow the
//! SQL:2003 windowing model (`PARTITION BY`, then `ORDER BY` within a partition,
//! then a ranking / aggregate function) — the model Stardog and AnzoGraph
//! expose. The caller runs an ordinary SELECT, then applies a [`WindowSpec`].
//!
//! This keeps the extension HONEST (the engine's SPARQL surface stays exactly
//! SPARQL 1.1; the window layer is an explicit, separate API a caller opts into)
//! and avoids any conformance-harness regression.
//!
//! ## Supported functions
//!
//! * [`WindowFunction::RowNumber`] — `ROW_NUMBER()`: 1-based sequential position
//!   within the partition, in the window `ORDER BY` order (ties broken by input
//!   order, i.e. stable).
//! * [`WindowFunction::Rank`] — `RANK()`: 1-based, with GAPS after ties (rows
//!   that compare equal under the window `ORDER BY` share a rank; the next
//!   distinct row's rank skips by the size of the tie group).
//! * [`WindowFunction::DenseRank`] — `DENSE_RANK()`: 1-based, NO gaps (ties share
//!   a rank, the next distinct row is +1).
//! * [`WindowFunction::Aggregate`] — a windowed aggregate over a FRAME within the
//!   partition. With no explicit [`WindowFrame`] the frame is the WHOLE partition
//!   (the SQL default for an aggregate with `PARTITION BY` and no frame): every
//!   row gets the same value, the aggregate of a chosen column over that
//!   partition. With an explicit frame (`ROWS`/`RANGE BETWEEN … AND …`) the
//!   aggregate is recomputed per row over only the rows in that row's frame, so a
//!   running total / moving average is expressible (see [`WindowFrame`]).
//! * [`WindowFunction::Lag`] / [`WindowFunction::Lead`] — `LAG`/`LEAD(?x[, n[,
//!   default]])`: the value of a column from the row `n` positions BEFORE (`Lag`)
//!   or AFTER (`Lead`) the current row in the window `ORDER BY` order. `n` defaults
//!   to `1`; a source row outside the partition yields the supplied `default`
//!   (unbound when none is given). These are POSITIONAL functions — they ignore any
//!   [`WindowFrame`]. [OPUS-4.8] (sq-hqhc)
//! * [`WindowFunction::Ntile`] — `NTILE(n)`: the 1-based bucket number when the
//!   ordered partition is split into `n` near-equal buckets (the first `size % n`
//!   buckets take one extra row). `n` must be `>= 1`. Also ignores any frame.
//!   [OPUS-4.8] (sq-hqhc)
//!
//! Each appends one new bound column (`new_var`) to every row; the row order of
//! the input `QueryResult` is preserved (window functions are computed, not a
//! re-sort).
//!
//! ## Window frames (`ROWS` / `RANGE`)  [OPUS-4.8] (sq-imj8)
//!
//! A [`WindowFrame`] narrows the set of rows an [`WindowFunction::Aggregate`]
//! folds over, per current row, in the window `ORDER BY` ordering. Two frame
//! UNITS, following SQL:2003:
//!
//! * [`FrameUnit::Rows`] — a PHYSICAL frame: bounds count rows by position in the
//!   ordered partition (`N PRECEDING` ≡ `N` rows back, `CURRENT ROW` ≡ this row,
//!   `N FOLLOWING` ≡ `N` rows on, and the `UNBOUNDED` ends). A running total is
//!   `ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW`; a 3-row moving sum is
//!   `ROWS BETWEEN 2 PRECEDING AND CURRENT ROW`.
//! * [`FrameUnit::Range`] — a LOGICAL frame: `CURRENT ROW` extends to the whole
//!   PEER group of the current row (all rows equal on every `ORDER BY` key), and
//!   the `UNBOUNDED` ends behave as for `Rows`. (A numeric value offset —
//!   `RANGE BETWEEN 5 PRECEDING` — is **not** modelled; only the `UNBOUNDED` /
//!   `CURRENT ROW` peer-based bounds are, which is the most-used RANGE form and
//!   the only one with an unambiguous meaning over the self-contained
//!   [`cmp_terms`] order. A numeric `N PRECEDING`/`N FOLLOWING` bound under
//!   `Range` is rejected by [`apply_window`].)
//!
//! Frames apply ONLY to aggregates; the ranking functions
//! (`ROW_NUMBER`/`RANK`/`DENSE_RANK`) and the positional functions
//! (`LAG`/`LEAD`/`NTILE`) ignore any frame (SQL forbids a frame on them, so
//! [`WindowSpec::frame`] is honoured only for [`WindowFunction::Aggregate`]).
//! A frame with no `ORDER BY` degenerates to the whole partition for `RANGE`
//! (every row is a peer) and is still well-defined for `ROWS` (input order within
//! the partition).

use oxrdf::{Literal, NamedNode, Term, Variable};
use std::cmp::Ordering;

use crate::QueryResult;

/// A caller-supplied windowed-aggregate fold: maps a partition's per-row cells
/// (`None` ≡ unbound for that row) to the partition's window value (`None` ≡
/// unbound). Used by [`WindowAggregate::Custom`].
pub type WindowFold = Box<dyn Fn(&[Option<Term>]) -> Option<Term>>;

/// A window sort key: a variable plus a direction. Comparison follows a
/// SPARQL-ORDER-BY-like total order over the bound terms (see [`cmp_terms`]):
/// numeric literals by value, everything else by a stable lexical/kind order;
/// an unbound cell sorts first (ascending).
#[derive(Debug, Clone)]
pub struct SortKey {
    pub var: Variable,
    /// `true` = descending (`DESC`), `false` = ascending (`ASC`, the default).
    pub descending: bool,
}

impl SortKey {
    pub fn asc(var: Variable) -> Self {
        Self { var, descending: false }
    }
    pub fn desc(var: Variable) -> Self {
        Self { var, descending: true }
    }
}

/// The aggregate a windowed [`WindowFunction::Aggregate`] computes over a
/// partition. The numeric ones operate on the numeric VALUE of each bound,
/// non-null cell; `Count` counts bound cells (`Count`-of-`*` semantics for the
/// chosen column); a fully custom fold is [`Self::Custom`].
pub enum WindowAggregate {
    /// COUNT of bound (non-null) values of the column.
    Count,
    /// SUM of the numeric values; an empty/all-unbound partition is `0`.
    Sum,
    /// AVG of the numeric values; an empty partition yields an unbound result.
    Avg,
    /// MIN by the [`cmp_terms`] order over bound cells (unbound for empty).
    Min,
    /// MAX by the [`cmp_terms`] order over bound cells (unbound for empty).
    Max,
    /// A caller-supplied fold over the partition's per-row cells (`None` ≡
    /// unbound for that row): returns the partition's window value (`None` ≡
    /// unbound result).
    Custom(WindowFold),
}

/// The window function to evaluate per partition.
pub enum WindowFunction {
    /// `ROW_NUMBER()` — see the module docs.
    RowNumber,
    /// `RANK()` — gaps after ties.
    Rank,
    /// `DENSE_RANK()` — no gaps.
    DenseRank,
    /// A windowed aggregate over the row's FRAME within the partition (the whole
    /// partition when [`WindowSpec::frame`] is `None`), of the given column.
    Aggregate { agg: WindowAggregate, of: Variable },
    /// `LAG(?of[, offset[, default]])` — the value of column `of` from the row
    /// `offset` positions BEFORE the current row, in the window `ORDER BY` order.
    /// `offset` defaults to `1`. When the source row falls outside the partition
    /// the result is `default` (unbound when no default is given). [OPUS-4.8] (sq-hqhc)
    Lag { of: Variable, offset: u64, default: Option<Term> },
    /// `LEAD(?of[, offset[, default]])` — as [`Self::Lag`] but `offset` positions
    /// AFTER the current row. [OPUS-4.8] (sq-hqhc)
    Lead { of: Variable, offset: u64, default: Option<Term> },
    /// `NTILE(n)` — the 1-based bucket number (`1..=n`) when the ordered partition
    /// is split into `n` buckets as evenly as possible (earlier buckets take the
    /// extra row when the partition size is not a multiple of `n`). `n` must be
    /// `>= 1` (a `0` bucket count is rejected by [`apply_window`]). [OPUS-4.8] (sq-hqhc)
    Ntile { buckets: u64 },
}

/// The unit of a window [`WindowFrame`]: physical row offsets (`ROWS`) or the
/// logical, peer-based `RANGE`. See the module docs.  [OPUS-4.8] (sq-imj8)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameUnit {
    /// `ROWS` — bounds count rows by physical position in the ordered partition.
    Rows,
    /// `RANGE` — `CURRENT ROW` covers the current row's whole PEER group; numeric
    /// `N PRECEDING`/`N FOLLOWING` offsets are not modelled (see the module docs).
    Range,
}

/// One end of a window [`WindowFrame`], in the window `ORDER BY` ordering.
/// [OPUS-4.8] (sq-imj8)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameBound {
    /// `UNBOUNDED PRECEDING` — the first row of the partition.
    UnboundedPreceding,
    /// `N PRECEDING` — `N` rows before the current row (`ROWS` only; rejected for
    /// `RANGE`). `0 PRECEDING` ≡ `CURRENT ROW`.
    Preceding(u64),
    /// `CURRENT ROW` — the current row (`ROWS`), or its whole peer group (`RANGE`).
    CurrentRow,
    /// `N FOLLOWING` — `N` rows after the current row (`ROWS` only; rejected for
    /// `RANGE`). `0 FOLLOWING` ≡ `CURRENT ROW`.
    Following(u64),
    /// `UNBOUNDED FOLLOWING` — the last row of the partition.
    UnboundedFollowing,
}

/// A window frame: a unit (`ROWS`/`RANGE`) and a `[start, end]` bound pair
/// (`BETWEEN start AND end`). The frame narrows the rows an
/// [`WindowFunction::Aggregate`] folds over, per current row; ranking functions
/// ignore it. `None` for [`WindowSpec::frame`] ≡ the whole partition (the SQL
/// default for an aggregate).  [OPUS-4.8] (sq-imj8)
///
/// A frame is well-formed iff `start` does not begin AFTER `end` in the ordering
/// (e.g. `UNBOUNDED FOLLOWING` may not be a `start`, `UNBOUNDED PRECEDING` may not
/// be an `end`); [`apply_window`] returns `Err` for a malformed frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowFrame {
    pub unit: FrameUnit,
    pub start: FrameBound,
    pub end: FrameBound,
}

impl WindowFrame {
    /// `ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW` — a running aggregate.
    pub fn rows_running() -> Self {
        Self { unit: FrameUnit::Rows, start: FrameBound::UnboundedPreceding, end: FrameBound::CurrentRow }
    }
    /// `ROWS BETWEEN n PRECEDING AND CURRENT ROW` — a trailing moving aggregate of
    /// width `n + 1`.
    pub fn rows_preceding(n: u64) -> Self {
        Self { unit: FrameUnit::Rows, start: FrameBound::Preceding(n), end: FrameBound::CurrentRow }
    }
}

/// A window specification: PARTITION BY zero or more columns, ORDER BY zero or
/// more sort keys, a [`WindowFunction`], and the name of the column to append.
///
/// Empty `partition_by` ≡ a single partition over the whole result. Empty
/// `order_by` for a ranking function ≡ every row is a peer (so `RANK`/
/// `DENSE_RANK` are all `1` and `ROW_NUMBER` follows input order).
pub struct WindowSpec {
    pub partition_by: Vec<Variable>,
    pub order_by: Vec<SortKey>,
    pub function: WindowFunction,
    /// An explicit window frame for an [`WindowFunction::Aggregate`] (`None` ≡ the
    /// whole partition, the SQL default). Ignored for the ranking functions.
    /// [OPUS-4.8] (sq-imj8)
    pub frame: Option<WindowFrame>,
    /// The new variable the computed window value is bound to.
    pub new_var: Variable,
}

/// Applies `spec` to `result`, returning a new [`QueryResult`] with `spec.new_var`
/// appended as the last column. Row order is preserved.
///
/// `Err` if a referenced variable (`partition_by` / `order_by` / the aggregate's
/// `of`) is not a column of `result`.
pub fn apply_window(result: &QueryResult, spec: &WindowSpec) -> Result<QueryResult, String> {
    let col = |v: &Variable| -> Result<usize, String> {
        result
            .vars
            .iter()
            .position(|c| c == v)
            .ok_or_else(|| format!("window: variable ?{} is not a result column", v.as_str()))
    };
    let part_cols: Vec<usize> = spec.partition_by.iter().map(&col).collect::<Result<_, _>>()?;
    let order_cols: Vec<(usize, bool)> =
        spec.order_by.iter().map(|k| col(&k.var).map(|c| (c, k.descending))).collect::<Result<_, _>>()?;

    // A frame is only meaningful for an aggregate; validate it (well-formedness +
    // the RANGE numeric-offset restriction) up front so a bad frame is one clear
    // error rather than a per-partition surprise.
    if let Some(frame) = &spec.frame {
        if matches!(spec.function, WindowFunction::Aggregate { .. }) {
            validate_frame(frame)?;
        }
    }
    // NTILE(0) is undefined (you cannot split into zero buckets); reject up front.
    // [OPUS-4.8] (sq-hqhc)
    if let WindowFunction::Ntile { buckets: 0 } = spec.function {
        return Err("window: NTILE requires a bucket count >= 1".into());
    }
    // The offset / positional functions read the column named by `of`, so validate
    // it like the aggregate's `of` (a missing column is one clear error).
    if let WindowFunction::Lag { of, .. } | WindowFunction::Lead { of, .. } = &spec.function {
        col(of)?;
    }

    // Group row indices into partitions by their partition-key tuple, preserving
    // first-seen partition order and input row order within a partition.
    let mut partitions: Vec<Vec<usize>> = Vec::new();
    let mut keys: Vec<Vec<Option<Term>>> = Vec::new();
    for (ri, row) in result.rows.iter().enumerate() {
        let key: Vec<Option<Term>> = part_cols.iter().map(|&c| row[c].clone()).collect();
        match keys.iter().position(|k| *k == key) {
            Some(pi) => partitions[pi].push(ri),
            None => {
                keys.push(key);
                partitions.push(vec![ri]);
            }
        }
    }

    // The computed window value for each input row index.
    let mut values: Vec<Option<Term>> = vec![None; result.rows.len()];
    for part in &partitions {
        // Order the partition's rows by the window ORDER BY (stable, so input
        // order breaks ties — required for ROW_NUMBER determinism).
        let mut ordered: Vec<usize> = part.clone();
        ordered.sort_by(|&a, &b| order_rows(result, &order_cols, a, b));
        compute_partition(result, spec, part, &ordered, &order_cols, &mut values)?;
    }

    let mut vars = result.vars.clone();
    vars.push(spec.new_var.clone());
    let rows: Vec<Vec<Option<Term>>> = result
        .rows
        .iter()
        .enumerate()
        .map(|(ri, row)| {
            let mut r = row.clone();
            r.push(values[ri].take());
            r
        })
        .collect();
    Ok(QueryResult { vars, rows })
}

/// Computes the window value for one partition, writing into `values` at each
/// row's input index. `ordered` is the partition's rows in window-ORDER-BY order.
fn compute_partition(
    result: &QueryResult,
    spec: &WindowSpec,
    part: &[usize],
    ordered: &[usize],
    order_cols: &[(usize, bool)],
    values: &mut [Option<Term>],
) -> Result<(), String> {
    match &spec.function {
        WindowFunction::RowNumber => {
            for (n, &ri) in ordered.iter().enumerate() {
                values[ri] = Some(int_term((n + 1) as i64));
            }
        }
        WindowFunction::Rank => {
            let mut rank: i64 = 0;
            let mut seen: i64 = 0;
            let mut prev: Option<usize> = None;
            for &ri in ordered {
                seen += 1;
                let tie = prev.is_some_and(|p| peers(result, order_cols, p, ri));
                if !tie {
                    rank = seen; // gap: jump to the running count
                }
                values[ri] = Some(int_term(rank));
                prev = Some(ri);
            }
        }
        WindowFunction::DenseRank => {
            let mut rank: i64 = 0;
            let mut prev: Option<usize> = None;
            for &ri in ordered {
                let tie = prev.is_some_and(|p| peers(result, order_cols, p, ri));
                if !tie {
                    rank += 1; // no gap
                }
                values[ri] = Some(int_term(rank));
                prev = Some(ri);
            }
        }
        WindowFunction::Aggregate { agg, of } => {
            let of_col = result.vars.iter().position(|c| c == of);
            // The chosen column's cell for each row of the partition, in window
            // ORDER BY order (`ordered`); the frame is computed over THIS ordered
            // list (positions are physical offsets within it).
            let ordered_cells: Vec<Option<Term>> = ordered
                .iter()
                .map(|&ri| of_col.and_then(|c| result.rows[ri][c].clone()))
                .collect();
            match &spec.frame {
                // No explicit frame ⇒ the whole partition (SQL default): one fold,
                // every row gets the same value. (Cheap path; preserves prior
                // behaviour exactly.)
                None => {
                    let v = eval_window_aggregate(agg, &ordered_cells);
                    for &ri in part {
                        values[ri] = v.clone();
                    }
                }
                // An explicit frame ⇒ recompute the aggregate per row over the rows
                // in that row's frame `[lo, hi]` (inclusive, by ordered position).
                Some(frame) => {
                    for (pos, &ri) in ordered.iter().enumerate() {
                        let (lo, hi) = frame_bounds(frame, pos, ordered, result, order_cols)?;
                        // An empty frame (`hi < lo`) yields the aggregate of the
                        // empty multiset (e.g. COUNT 0, SUM 0, AVG/MIN/MAX unbound).
                        let slice: &[Option<Term>] =
                            if lo <= hi { &ordered_cells[lo..=hi] } else { &[] };
                        values[ri] = eval_window_aggregate(agg, slice);
                    }
                }
            }
        }
        // LAG / LEAD: offset the current ordered position by ±`offset`; out of the
        // partition ⇒ the supplied default (unbound if none). The frame is ignored
        // (SQL: an offset function takes no frame). [OPUS-4.8] (sq-hqhc)
        WindowFunction::Lag { of, offset, default } => {
            eval_offset(result, of, ordered, values, |pos| pos.checked_sub(*offset as i128), default);
        }
        WindowFunction::Lead { of, offset, default } => {
            eval_offset(result, of, ordered, values, |pos| pos.checked_add(*offset as i128), default);
        }
        // NTILE(n): split the ordered partition into `n` near-equal buckets; the
        // first `size % n` buckets get one extra row. The frame is ignored.
        // [OPUS-4.8] (sq-hqhc)
        WindowFunction::Ntile { buckets } => {
            let n = *buckets;
            // `buckets == 0` is validated away in `apply_window`; guard anyway.
            if n == 0 {
                return Err("window: NTILE requires a bucket count >= 1".into());
            }
            let total = ordered.len() as u64;
            let base = total / n; // minimum rows per bucket
            let extra = total % n; // first `extra` buckets get one more
            let mut pos: u64 = 0; // 0-based ordered position consumed so far
            for bucket in 0..n {
                let this = base + u64::from(bucket < extra);
                for _ in 0..this {
                    let ri = ordered[pos as usize];
                    values[ri] = Some(int_term((bucket + 1) as i64));
                    pos += 1;
                }
            }
        }
    }
    Ok(())
}

/// Shared LAG/LEAD body: for each ordered position `pos`, read column `of` from the
/// row at `src(pos)` (a position-offset closure) and write it into `values` at the
/// current row's input index; an out-of-partition source yields `default` (which may
/// be unbound). [OPUS-4.8] (sq-hqhc)
fn eval_offset(
    result: &QueryResult,
    of: &Variable,
    ordered: &[usize],
    values: &mut [Option<Term>],
    src: impl Fn(i128) -> Option<i128>,
    default: &Option<Term>,
) {
    let of_col = result.vars.iter().position(|c| c == of);
    let n = ordered.len() as i128;
    for (pos, &ri) in ordered.iter().enumerate() {
        let value = match src(pos as i128) {
            Some(j) if j >= 0 && j < n => {
                let src_ri = ordered[j as usize];
                of_col.and_then(|c| result.rows[src_ri][c].clone())
            }
            // Source row is outside the partition: fall back to the default.
            _ => default.clone(),
        };
        values[ri] = value;
    }
}

/// Validates a window frame for an aggregate: well-formedness (a `start` may not
/// be `UNBOUNDED FOLLOWING`, an `end` may not be `UNBOUNDED PRECEDING`) and the
/// `RANGE` restriction (no numeric `N PRECEDING`/`N FOLLOWING` offset under
/// `RANGE`).  [OPUS-4.8] (sq-imj8)
fn validate_frame(frame: &WindowFrame) -> Result<(), String> {
    if matches!(frame.start, FrameBound::UnboundedFollowing) {
        return Err("window frame: start bound may not be UNBOUNDED FOLLOWING".into());
    }
    if matches!(frame.end, FrameBound::UnboundedPreceding) {
        return Err("window frame: end bound may not be UNBOUNDED PRECEDING".into());
    }
    if frame.unit == FrameUnit::Range {
        for b in [&frame.start, &frame.end] {
            if matches!(b, FrameBound::Preceding(_) | FrameBound::Following(_)) {
                return Err(
                    "window frame: RANGE supports only UNBOUNDED PRECEDING / CURRENT ROW / UNBOUNDED FOLLOWING bounds (a numeric N PRECEDING/FOLLOWING RANGE offset is not supported; use ROWS, or the programmatic API)".into(),
                );
            }
        }
    }
    Ok(())
}

/// The inclusive ordered-position interval `[lo, hi]` of the frame for the row at
/// ordered position `pos`. `lo > hi` ⇒ an empty frame. For `RANGE` a `CURRENT ROW`
/// bound extends to the current row's whole PEER group (per the `start`/`end`
/// side); for `ROWS` a `CURRENT ROW` bound is `pos` itself.  [OPUS-4.8] (sq-imj8)
fn frame_bounds(
    frame: &WindowFrame,
    pos: usize,
    ordered: &[usize],
    result: &QueryResult,
    order_cols: &[(usize, bool)],
) -> Result<(usize, usize), String> {
    let n = ordered.len();
    // `lo` resolves a START bound to its first ordered position; `hi` resolves an
    // END bound to its last. CURRENT ROW differs per unit (and per side for RANGE).
    let pos = pos as isize;
    let lo: isize = match frame.start {
        FrameBound::UnboundedPreceding => 0,
        FrameBound::Preceding(k) => pos.saturating_sub(clamp_offset(k)),
        FrameBound::CurrentRow => match frame.unit {
            FrameUnit::Rows => pos,
            // RANGE: the start of the current row's peer group.
            FrameUnit::Range => peer_group_start(pos as usize, ordered, result, order_cols) as isize,
        },
        FrameBound::Following(k) => pos.saturating_add(clamp_offset(k)),
        FrameBound::UnboundedFollowing => {
            return Err("window frame: start bound may not be UNBOUNDED FOLLOWING".into())
        }
    };
    let hi: isize = match frame.end {
        FrameBound::UnboundedPreceding => {
            return Err("window frame: end bound may not be UNBOUNDED PRECEDING".into())
        }
        FrameBound::Preceding(k) => pos.saturating_sub(clamp_offset(k)),
        FrameBound::CurrentRow => match frame.unit {
            FrameUnit::Rows => pos,
            // RANGE: the end of the current row's peer group.
            FrameUnit::Range => peer_group_end(pos as usize, ordered, result, order_cols) as isize,
        },
        FrameBound::Following(k) => pos.saturating_add(clamp_offset(k)),
        FrameBound::UnboundedFollowing => n as isize - 1,
    };
    // Clamp to the partition; an entirely out-of-range frame becomes empty.
    let lo = lo.clamp(0, n as isize);
    let hi = hi.clamp(-1, n as isize - 1);
    if lo > hi {
        // Signal "empty" with a lo>hi pair the caller treats as the empty slice.
        Ok((1, 0))
    } else {
        Ok((lo as usize, hi as usize))
    }
}

/// Saturates a frame offset to `isize::MAX` so `pos ± k` (with `saturating_*`)
/// cannot overflow for an absurd `N PRECEDING`/`N FOLLOWING` (it clamps to the ends).
fn clamp_offset(k: u64) -> isize {
    isize::try_from(k).unwrap_or(isize::MAX)
}

/// The first ordered position that is a PEER of `pos` (same `ORDER BY` keys).
fn peer_group_start(pos: usize, ordered: &[usize], result: &QueryResult, order_cols: &[(usize, bool)]) -> usize {
    let mut lo = pos;
    while lo > 0 && peers(result, order_cols, ordered[lo - 1], ordered[pos]) {
        lo -= 1;
    }
    lo
}

/// The last ordered position that is a PEER of `pos` (same `ORDER BY` keys).
fn peer_group_end(pos: usize, ordered: &[usize], result: &QueryResult, order_cols: &[(usize, bool)]) -> usize {
    let mut hi = pos;
    while hi + 1 < ordered.len() && peers(result, order_cols, ordered[hi + 1], ordered[pos]) {
        hi += 1;
    }
    hi
}

/// Whether rows `a` and `b` are PEERS under the window ORDER BY (compare equal on
/// every order key). With no `ORDER BY`, all rows are peers.
fn peers(result: &QueryResult, order_cols: &[(usize, bool)], a: usize, b: usize) -> bool {
    order_rows(result, order_cols, a, b) == Ordering::Equal
}

/// Compare two rows by the window ORDER BY keys (direction applied), then Equal.
/// (Stable sort preserves input order for full ties, so ROW_NUMBER is deterministic.)
fn order_rows(result: &QueryResult, order_cols: &[(usize, bool)], a: usize, b: usize) -> Ordering {
    for &(c, desc) in order_cols {
        let ord = cmp_terms(result.rows[a][c].as_ref(), result.rows[b][c].as_ref());
        let ord = if desc { ord.reverse() } else { ord };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    Ordering::Equal
}

fn eval_window_aggregate(agg: &WindowAggregate, cells: &[Option<Term>]) -> Option<Term> {
    match agg {
        WindowAggregate::Count => Some(int_term(cells.iter().filter(|c| c.is_some()).count() as i64)),
        WindowAggregate::Sum => {
            let sum: f64 = cells.iter().filter_map(|c| c.as_ref()).filter_map(numeric_value).sum();
            Some(num_term(sum))
        }
        WindowAggregate::Avg => {
            let vals: Vec<f64> = cells.iter().filter_map(|c| c.as_ref()).filter_map(numeric_value).collect();
            if vals.is_empty() {
                None
            } else {
                Some(num_term(vals.iter().sum::<f64>() / vals.len() as f64))
            }
        }
        WindowAggregate::Min => cells
            .iter()
            .filter_map(|c| c.as_ref())
            .min_by(|a, b| cmp_terms(Some(a), Some(b)))
            .cloned(),
        WindowAggregate::Max => cells
            .iter()
            .filter_map(|c| c.as_ref())
            .max_by(|a, b| cmp_terms(Some(a), Some(b)))
            .cloned(),
        WindowAggregate::Custom(f) => f(cells),
    }
}

/// The numeric VALUE of a term, if it is a numeric literal (xsd integer / decimal
/// / float / double, or anything whose lexical form parses as an `f64`).
fn numeric_value(t: &Term) -> Option<f64> {
    match t {
        Term::Literal(l) => l.value().parse::<f64>().ok(),
        _ => None,
    }
}

/// A SPARQL-ORDER-BY-like total order over optional bound terms, self-contained
/// so the window pass needs no engine internals. Ordering: unbound (`None`) sorts
/// FIRST; then by KIND (blank < IRI < literal — a stable, deterministic order);
/// within literals, two NUMERIC literals compare by value, otherwise by
/// `(datatype, language, lexical)`. This is a TOTAL order (every pair is
/// comparable), which a window sort requires.
pub fn cmp_terms(a: Option<&Term>, b: Option<&Term>) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(x), Some(y)) => cmp_bound(x, y),
    }
}

fn kind_rank(t: &Term) -> u8 {
    match t {
        Term::BlankNode(_) => 0,
        Term::NamedNode(_) => 1,
        Term::Literal(_) => 2,
        // oxrdf may expose a Triple term under RDF 1.2; order it last, stably.
        _ => 3,
    }
}

fn cmp_bound(x: &Term, y: &Term) -> Ordering {
    match (x, y) {
        (Term::Literal(lx), Term::Literal(ly)) => {
            if let (Some(nx), Some(ny)) = (numeric_value(&Term::Literal(lx.clone())), numeric_value(&Term::Literal(ly.clone()))) {
                if let Some(o) = nx.partial_cmp(&ny) {
                    if o != Ordering::Equal {
                        return o;
                    }
                }
            }
            lx.datatype()
                .as_str()
                .cmp(ly.datatype().as_str())
                .then_with(|| lx.language().unwrap_or("").cmp(ly.language().unwrap_or("")))
                .then_with(|| lx.value().cmp(ly.value()))
        }
        (Term::NamedNode(a), Term::NamedNode(b)) => a.as_str().cmp(b.as_str()),
        (Term::BlankNode(a), Term::BlankNode(b)) => a.as_str().cmp(b.as_str()),
        _ => kind_rank(x).cmp(&kind_rank(y)).then_with(|| x.to_string().cmp(&y.to_string())),
    }
}

/// An `xsd:integer` term for a rank / count.
fn int_term(n: i64) -> Term {
    Term::Literal(Literal::from(n))
}

/// A numeric term: an `xsd:integer` if the value is integral, else an
/// `xsd:double`. (Window SUM/AVG over integer inputs commonly want an integer
/// back when exact; AVG of an odd sum stays a double.)
fn num_term(v: f64) -> Term {
    if v.fract() == 0.0 && v.is_finite() && v.abs() < 9.007_199_254_740_992e15 {
        int_term(v as i64)
    } else {
        Term::Literal(Literal::new_typed_literal(
            {
                let mut s = v.to_string();
                if !s.contains(['.', 'e', 'E', 'N', 'i']) {
                    s.push_str(".0");
                }
                s
            },
            NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#double"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::NamedNode as NN;

    fn var(s: &str) -> Variable {
        Variable::new(s).unwrap()
    }
    fn iri(s: &str) -> Term {
        Term::NamedNode(NN::new(s).unwrap())
    }
    fn int(n: i64) -> Term {
        Term::Literal(Literal::from(n))
    }

    /// Three depts, sales values, to rank within partition.
    /// (?emp ?dept ?sales): a/eng/30, b/eng/30, c/eng/20, d/sales/10
    fn sample() -> QueryResult {
        QueryResult {
            vars: vec![var("emp"), var("dept"), var("sales")],
            rows: vec![
                vec![Some(iri("http://ex/a")), Some(iri("http://ex/eng")), Some(int(30))],
                vec![Some(iri("http://ex/b")), Some(iri("http://ex/eng")), Some(int(30))],
                vec![Some(iri("http://ex/c")), Some(iri("http://ex/eng")), Some(int(20))],
                vec![Some(iri("http://ex/d")), Some(iri("http://ex/sales")), Some(int(10))],
            ],
        }
    }

    fn val(r: &QueryResult, row: usize, col: usize) -> String {
        r.rows[row][col].as_ref().unwrap().to_string()
    }

    #[test]
    fn row_number_partition_order() {
        // PARTITION BY dept ORDER BY sales DESC -> ROW_NUMBER per dept.
        let spec = WindowSpec {
            partition_by: vec![var("dept")],
            order_by: vec![SortKey::desc(var("sales"))],
            function: WindowFunction::RowNumber,
            frame: None,
            new_var: var("rn"),
        };
        let out = apply_window(&sample(), &spec).unwrap();
        // new column is appended at index 3; rows stay in input order.
        // eng partition (a=30,b=30,c=20) ordered desc -> a,b are 30 (tie, input order a<b), c=20.
        // a -> 1, b -> 2, c -> 3 ; sales partition d -> 1.
        assert_eq!(val(&out, 0, 3), int(1).to_string()); // a
        assert_eq!(val(&out, 1, 3), int(2).to_string()); // b
        assert_eq!(val(&out, 2, 3), int(3).to_string()); // c
        assert_eq!(val(&out, 3, 3), int(1).to_string()); // d
    }

    #[test]
    fn rank_has_gaps_dense_rank_does_not() {
        let mk = |f| WindowSpec {
            partition_by: vec![var("dept")],
            order_by: vec![SortKey::desc(var("sales"))],
            function: f,
            frame: None,
            new_var: var("r"),
        };
        let rank = apply_window(&sample(), &mk(WindowFunction::Rank)).unwrap();
        // eng: a=30,b=30 -> both rank 1; c=20 -> rank 3 (GAP).
        assert_eq!(val(&rank, 0, 3), int(1).to_string());
        assert_eq!(val(&rank, 1, 3), int(1).to_string());
        assert_eq!(val(&rank, 2, 3), int(3).to_string());
        let dense = apply_window(&sample(), &mk(WindowFunction::DenseRank)).unwrap();
        // eng: a,b -> 1; c -> 2 (NO gap).
        assert_eq!(val(&dense, 0, 3), int(1).to_string());
        assert_eq!(val(&dense, 1, 3), int(1).to_string());
        assert_eq!(val(&dense, 2, 3), int(2).to_string());
    }

    #[test]
    fn windowed_sum_over_partition() {
        let spec = WindowSpec {
            partition_by: vec![var("dept")],
            order_by: vec![],
            function: WindowFunction::Aggregate { agg: WindowAggregate::Sum, of: var("sales") },
            frame: None,
            new_var: var("total"),
        };
        let out = apply_window(&sample(), &spec).unwrap();
        // eng total = 30+30+20 = 80 for every eng row; sales total = 10.
        assert_eq!(val(&out, 0, 3), int(80).to_string());
        assert_eq!(val(&out, 1, 3), int(80).to_string());
        assert_eq!(val(&out, 2, 3), int(80).to_string());
        assert_eq!(val(&out, 3, 3), int(10).to_string());
    }

    #[test]
    fn windowed_count_and_custom() {
        let cnt = apply_window(
            &sample(),
            &WindowSpec {
                partition_by: vec![var("dept")],
                order_by: vec![],
                function: WindowFunction::Aggregate { agg: WindowAggregate::Count, of: var("sales") },
                frame: None,
                new_var: var("n"),
            },
        )
        .unwrap();
        assert_eq!(val(&cnt, 0, 3), int(3).to_string()); // eng count
        assert_eq!(val(&cnt, 3, 3), int(1).to_string()); // sales count

        // Custom fold: max sales as a string, to prove the closure boundary.
        let custom = apply_window(
            &sample(),
            &WindowSpec {
                partition_by: vec![var("dept")],
                order_by: vec![],
                function: WindowFunction::Aggregate {
                    agg: WindowAggregate::Custom(Box::new(|cells: &[Option<Term>]| {
                        cells.iter().filter_map(|c| c.as_ref()).max_by(|a, b| cmp_terms(Some(a), Some(b))).cloned()
                    })),
                    of: var("sales"),
                },
                frame: None,
                new_var: var("m"),
            },
        )
        .unwrap();
        assert_eq!(val(&custom, 0, 3), int(30).to_string()); // eng max
        assert_eq!(val(&custom, 3, 3), int(10).to_string()); // sales max
    }

    #[test]
    fn unknown_variable_is_error() {
        let spec = WindowSpec {
            partition_by: vec![var("nope")],
            order_by: vec![],
            function: WindowFunction::RowNumber,
            frame: None,
            new_var: var("rn"),
        };
        assert!(apply_window(&sample(), &spec).is_err());
    }

    // ---- window frames (ROWS / RANGE) — sq-imj8 [OPUS-4.8] ----

    /// A single partition with distinct, ascending values so frame offsets are
    /// unambiguous: (?k) = 10, 20, 20, 40 in input (and ORDER BY ?k) order.
    fn seq() -> QueryResult {
        QueryResult {
            vars: vec![var("k")],
            rows: vec![vec![Some(int(10))], vec![Some(int(20))], vec![Some(int(20))], vec![Some(int(40))]],
        }
    }

    fn agg(frame: Option<WindowFrame>, a: WindowAggregate) -> WindowSpec {
        WindowSpec {
            partition_by: vec![],
            order_by: vec![SortKey::asc(var("k"))],
            function: WindowFunction::Aggregate { agg: a, of: var("k") },
            frame,
            new_var: var("w"),
        }
    }

    #[test]
    fn rows_running_total_is_cumulative() {
        // ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW: running SUM.
        let out = apply_window(&seq(), &agg(Some(WindowFrame::rows_running()), WindowAggregate::Sum)).unwrap();
        // 10, 10+20, 10+20+20, 10+20+20+40 = 10, 30, 50, 90.
        assert_eq!(val(&out, 0, 1), int(10).to_string());
        assert_eq!(val(&out, 1, 1), int(30).to_string());
        assert_eq!(val(&out, 2, 1), int(50).to_string());
        assert_eq!(val(&out, 3, 1), int(90).to_string());
    }

    #[test]
    fn rows_trailing_moving_sum_and_count() {
        // ROWS BETWEEN 1 PRECEDING AND CURRENT ROW: 2-row trailing window.
        let sum = apply_window(&seq(), &agg(Some(WindowFrame::rows_preceding(1)), WindowAggregate::Sum)).unwrap();
        // 10, 10+20, 20+20, 20+40 = 10, 30, 40, 60.
        assert_eq!(val(&sum, 0, 1), int(10).to_string());
        assert_eq!(val(&sum, 1, 1), int(30).to_string());
        assert_eq!(val(&sum, 2, 1), int(40).to_string());
        assert_eq!(val(&sum, 3, 1), int(60).to_string());
        // COUNT over the same 2-row trailing window: 1, 2, 2, 2.
        let cnt = apply_window(&seq(), &agg(Some(WindowFrame::rows_preceding(1)), WindowAggregate::Count)).unwrap();
        assert_eq!(val(&cnt, 0, 1), int(1).to_string());
        assert_eq!(val(&cnt, 1, 1), int(2).to_string());
        assert_eq!(val(&cnt, 3, 1), int(2).to_string());
    }

    #[test]
    fn rows_centered_following_window() {
        // ROWS BETWEEN CURRENT ROW AND 1 FOLLOWING: this row + next.
        let frame = WindowFrame { unit: FrameUnit::Rows, start: FrameBound::CurrentRow, end: FrameBound::Following(1) };
        let out = apply_window(&seq(), &agg(Some(frame), WindowAggregate::Sum)).unwrap();
        // 10+20, 20+20, 20+40, 40 = 30, 40, 60, 40 (last row has no follower).
        assert_eq!(val(&out, 0, 1), int(30).to_string());
        assert_eq!(val(&out, 1, 1), int(40).to_string());
        assert_eq!(val(&out, 2, 1), int(60).to_string());
        assert_eq!(val(&out, 3, 1), int(40).to_string());
    }

    #[test]
    fn range_running_includes_peer_group() {
        // RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW: the running SUM is the
        // same as ROWS at distinct rows, BUT at the two equal `20` peers the whole
        // peer group is included, so BOTH 20-rows see 10+20+20 = 50 (not 30 then 50).
        let frame = WindowFrame { unit: FrameUnit::Range, start: FrameBound::UnboundedPreceding, end: FrameBound::CurrentRow };
        let out = apply_window(&seq(), &agg(Some(frame), WindowAggregate::Sum)).unwrap();
        assert_eq!(val(&out, 0, 1), int(10).to_string()); // 10
        assert_eq!(val(&out, 1, 1), int(50).to_string()); // 10+20+20 (peer group)
        assert_eq!(val(&out, 2, 1), int(50).to_string()); // 10+20+20 (peer group)
        assert_eq!(val(&out, 3, 1), int(90).to_string()); // all
    }

    #[test]
    fn no_frame_is_whole_partition_unchanged() {
        // Without a frame an aggregate is still the whole-partition fold (the prior
        // behaviour): every row sees the total.
        let out = apply_window(&seq(), &agg(None, WindowAggregate::Sum)).unwrap();
        for r in 0..4 {
            assert_eq!(val(&out, r, 1), int(90).to_string());
        }
    }

    #[test]
    fn empty_frame_yields_empty_aggregate() {
        // ROWS BETWEEN 3 PRECEDING AND 2 PRECEDING: empty for the first rows.
        let frame = WindowFrame { unit: FrameUnit::Rows, start: FrameBound::Preceding(3), end: FrameBound::Preceding(2) };
        let sum = apply_window(&seq(), &agg(Some(frame), WindowAggregate::Sum)).unwrap();
        // Row 0: frame ends 2 rows BEFORE row 0 → entirely before the partition →
        // empty. The empty-frame SUM is 0 and COUNT is 0; MIN/MAX/AVG are unbound.
        assert_eq!(val(&sum, 0, 1), int(0).to_string());
        let cnt = apply_window(&seq(), &agg(Some(frame), WindowAggregate::Count)).unwrap();
        assert_eq!(val(&cnt, 0, 1), int(0).to_string());
        // MIN over an empty frame is unbound.
        let min = apply_window(&seq(), &agg(Some(frame), WindowAggregate::Min)).unwrap();
        assert!(min.rows[0][1].is_none());
    }

    #[test]
    fn malformed_frame_is_error() {
        // start = UNBOUNDED FOLLOWING is malformed.
        let bad = WindowFrame { unit: FrameUnit::Rows, start: FrameBound::UnboundedFollowing, end: FrameBound::CurrentRow };
        assert!(apply_window(&seq(), &agg(Some(bad), WindowAggregate::Sum)).is_err());
        // end = UNBOUNDED PRECEDING is malformed.
        let bad2 = WindowFrame { unit: FrameUnit::Rows, start: FrameBound::CurrentRow, end: FrameBound::UnboundedPreceding };
        assert!(apply_window(&seq(), &agg(Some(bad2), WindowAggregate::Sum)).is_err());
        // RANGE with a numeric offset is unsupported.
        let bad3 = WindowFrame { unit: FrameUnit::Range, start: FrameBound::Preceding(1), end: FrameBound::CurrentRow };
        let e = apply_window(&seq(), &agg(Some(bad3), WindowAggregate::Sum)).unwrap_err();
        assert!(e.contains("RANGE"), "{e}");
    }

    #[test]
    fn absurd_frame_offset_saturates_not_overflows() {
        // A huge N PRECEDING/FOLLOWING must clamp to the partition ends, not panic.
        let huge = WindowFrame {
            unit: FrameUnit::Rows,
            start: FrameBound::Preceding(u64::MAX),
            end: FrameBound::Following(u64::MAX),
        };
        let out = apply_window(&seq(), &agg(Some(huge), WindowAggregate::Sum)).unwrap();
        // The frame spans the whole partition for every row → the grand total 90.
        for r in 0..4 {
            assert_eq!(val(&out, r, 1), int(90).to_string());
        }
    }

    #[test]
    fn frame_is_ignored_for_ranking_functions() {
        // A frame on ROW_NUMBER is silently ignored (SQL forbids it; we don't error).
        let spec = WindowSpec {
            partition_by: vec![],
            order_by: vec![SortKey::asc(var("k"))],
            function: WindowFunction::RowNumber,
            frame: Some(WindowFrame::rows_running()),
            new_var: var("rn"),
        };
        let out = apply_window(&seq(), &spec).unwrap();
        assert_eq!(val(&out, 0, 1), int(1).to_string());
        assert_eq!(val(&out, 3, 1), int(4).to_string());
    }

    // ---- offset / positional functions: LAG / LEAD / NTILE — sq-hqhc [OPUS-4.8] ----

    /// `(?emp ?sales)` in one partition: a=10, b=20, c=30, d=40 in input order.
    fn seq2() -> QueryResult {
        QueryResult {
            vars: vec![var("emp"), var("sales")],
            rows: vec![
                vec![Some(iri("http://ex/a")), Some(int(10))],
                vec![Some(iri("http://ex/b")), Some(int(20))],
                vec![Some(iri("http://ex/c")), Some(int(30))],
                vec![Some(iri("http://ex/d")), Some(int(40))],
            ],
        }
    }

    #[test]
    fn lag_default_offset_one() {
        // LAG(?sales) over ORDER BY ?sales asc: each row sees the previous row's value;
        // the first row's predecessor is out of partition → unbound (no default).
        let spec = WindowSpec {
            partition_by: vec![],
            order_by: vec![SortKey::asc(var("sales"))],
            function: WindowFunction::Lag { of: var("sales"), offset: 1, default: None },
            frame: None,
            new_var: var("prev"),
        };
        let out = apply_window(&seq2(), &spec).unwrap();
        // ordered 10,20,30,40 → lag: unbound, 10, 20, 30 (mapped back to input order a,b,c,d).
        assert!(out.rows[0][2].is_none()); // a: no predecessor
        assert_eq!(val(&out, 1, 2), int(10).to_string()); // b sees a=10
        assert_eq!(val(&out, 2, 2), int(20).to_string()); // c sees b=20
        assert_eq!(val(&out, 3, 2), int(30).to_string()); // d sees c=30
    }

    #[test]
    fn lag_with_offset_and_default() {
        // LAG(?sales, 2, -1): two rows back, default -1 when out of partition.
        let spec = WindowSpec {
            partition_by: vec![],
            order_by: vec![SortKey::asc(var("sales"))],
            function: WindowFunction::Lag { of: var("sales"), offset: 2, default: Some(int(-1)) },
            frame: None,
            new_var: var("prev2"),
        };
        let out = apply_window(&seq2(), &spec).unwrap();
        // ordered 10,20,30,40 → lag2: default(-1), default(-1), 10, 20.
        assert_eq!(val(&out, 0, 2), int(-1).to_string()); // a: out → default
        assert_eq!(val(&out, 1, 2), int(-1).to_string()); // b: out → default
        assert_eq!(val(&out, 2, 2), int(10).to_string()); // c sees a=10
        assert_eq!(val(&out, 3, 2), int(20).to_string()); // d sees b=20
    }

    #[test]
    fn lead_default_offset_one() {
        // LEAD(?sales): the NEXT row's value; the last row's successor is out → unbound.
        let spec = WindowSpec {
            partition_by: vec![],
            order_by: vec![SortKey::asc(var("sales"))],
            function: WindowFunction::Lead { of: var("sales"), offset: 1, default: None },
            frame: None,
            new_var: var("next"),
        };
        let out = apply_window(&seq2(), &spec).unwrap();
        // ordered 10,20,30,40 → lead: 20, 30, 40, unbound.
        assert_eq!(val(&out, 0, 2), int(20).to_string()); // a sees b=20
        assert_eq!(val(&out, 2, 2), int(40).to_string()); // c sees d=40
        assert!(out.rows[3][2].is_none()); // d: no successor
    }

    #[test]
    fn ntile_even_and_uneven() {
        // NTILE(2) over 4 rows → buckets [1,1,2,2] (even split).
        let two = WindowSpec {
            partition_by: vec![],
            order_by: vec![SortKey::asc(var("sales"))],
            function: WindowFunction::Ntile { buckets: 2 },
            frame: None,
            new_var: var("nt"),
        };
        let out = apply_window(&seq2(), &two).unwrap();
        assert_eq!(val(&out, 0, 2), int(1).to_string()); // a
        assert_eq!(val(&out, 1, 2), int(1).to_string()); // b
        assert_eq!(val(&out, 2, 2), int(2).to_string()); // c
        assert_eq!(val(&out, 3, 2), int(2).to_string()); // d

        // NTILE(3) over 4 rows → the first (4 % 3 = 1) bucket gets the extra row:
        // bucket sizes 2,1,1 → [1,1,2,3].
        let three = WindowSpec {
            partition_by: vec![],
            order_by: vec![SortKey::asc(var("sales"))],
            function: WindowFunction::Ntile { buckets: 3 },
            frame: None,
            new_var: var("nt3"),
        };
        let out3 = apply_window(&seq2(), &three).unwrap();
        assert_eq!(val(&out3, 0, 2), int(1).to_string());
        assert_eq!(val(&out3, 1, 2), int(1).to_string());
        assert_eq!(val(&out3, 2, 2), int(2).to_string());
        assert_eq!(val(&out3, 3, 2), int(3).to_string());
    }

    #[test]
    fn ntile_more_buckets_than_rows() {
        // NTILE(10) over 4 rows: 4 % 10 = 4 buckets of size 1, the rest empty →
        // each row in its own bucket 1..=4.
        let spec = WindowSpec {
            partition_by: vec![],
            order_by: vec![SortKey::asc(var("sales"))],
            function: WindowFunction::Ntile { buckets: 10 },
            frame: None,
            new_var: var("nt"),
        };
        let out = apply_window(&seq2(), &spec).unwrap();
        for (i, expect) in [1, 2, 3, 4].into_iter().enumerate() {
            assert_eq!(val(&out, i, 2), int(expect).to_string());
        }
    }

    #[test]
    fn ntile_zero_is_error() {
        let spec = WindowSpec {
            partition_by: vec![],
            order_by: vec![SortKey::asc(var("sales"))],
            function: WindowFunction::Ntile { buckets: 0 },
            frame: None,
            new_var: var("nt"),
        };
        assert!(apply_window(&seq2(), &spec).is_err());
    }

    #[test]
    fn lag_lead_respect_partitions() {
        // LAG must not cross a partition boundary: with PARTITION BY ?dept, the first
        // row of each dept has no predecessor.
        let data = QueryResult {
            vars: vec![var("dept"), var("sales")],
            rows: vec![
                vec![Some(iri("http://ex/eng")), Some(int(10))],
                vec![Some(iri("http://ex/eng")), Some(int(20))],
                vec![Some(iri("http://ex/sales")), Some(int(30))],
            ],
        };
        let spec = WindowSpec {
            partition_by: vec![var("dept")],
            order_by: vec![SortKey::asc(var("sales"))],
            function: WindowFunction::Lag { of: var("sales"), offset: 1, default: None },
            frame: None,
            new_var: var("prev"),
        };
        let out = apply_window(&data, &spec).unwrap();
        assert!(out.rows[0][2].is_none()); // eng/10: first of partition
        assert_eq!(val(&out, 1, 2), int(10).to_string()); // eng/20 sees eng/10
        assert!(out.rows[2][2].is_none()); // sales/30: first (and only) of its partition
    }

    #[test]
    fn lag_unknown_column_is_error() {
        let spec = WindowSpec {
            partition_by: vec![],
            order_by: vec![SortKey::asc(var("sales"))],
            function: WindowFunction::Lag { of: var("nope"), offset: 1, default: None },
            frame: None,
            new_var: var("prev"),
        };
        assert!(apply_window(&seq2(), &spec).is_err());
    }
}
