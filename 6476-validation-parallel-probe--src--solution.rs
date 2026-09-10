//! Oxigraph-shaped per-solution view over a [`QueryResult`] (sq-xj29q). [OPUS-4.8]
//!
//! sparq stores SELECT results in a **columnar** [`QueryResult`] — a shared `vars`
//! header plus a `Vec<Vec<Option<Term>>>` of rows. That layout is what the engine,
//! the SPARQL-Results-JSON writer and the result cache want (the variable name is
//! stored once, not per cell). Oxigraph, by contrast, hands a SELECT consumer an
//! iterator of `QuerySolution` values, each of which behaves like a per-row map from
//! `Variable` to `Term` (`solution.get("x")`, `solution.iter()`, `solution["x"]`).
//!
//! For a Rust consumer migrating from / interoperating with Oxigraph (#1124 oxrdf
//! alignment, #1128 migration guide), iterating `result.rows` and indexing the `vars`
//! header by hand is the friction. This module closes that gap **without** abandoning
//! the columnar layout: [`QueryResult::solutions`] returns an iterator of
//! [`QuerySolution`], each a **borrowed, zero-copy view** of one row that shares the
//! `vars` header — no second materialisation of the table, no per-row `HashMap`, no
//! cloned terms. A `QuerySolution` is the size of two references plus a row index; the
//! `Term`s it yields are borrowed straight out of `QueryResult::rows`.
//!
//! The accessor surface mirrors Oxigraph's `QuerySolution` so the same call sites
//! compile against either engine:
//!
//! * [`QuerySolution::get`] — by `&str` name, by [`VariableRef`]/`&Variable`, or by
//!   positional `usize`; returns `Option<&Term>` (`None` = absent variable **or** an
//!   unbound cell, exactly like Oxigraph).
//! * [`QuerySolution::iter`] — over the **bound** `(VariableRef, &Term)` pairs only,
//!   skipping unbound cells (Oxigraph's `QuerySolution::iter` semantics).
//! * [`core::ops::Index`] by `&str` / `VariableRef` / `usize` — panics on a missing
//!   binding, mirroring Oxigraph's `Index` impl, for terse `&sol["x"]` access.
//! * [`QuerySolution::variables`], [`len`](QuerySolution::len),
//!   [`is_empty`](QuerySolution::is_empty), [`values`](QuerySolution::values).
//!
//! This is the OPT-IN `query-solution` feature: when it is off, zero of this code
//! compiles and the default native + wasm builds are byte-identical. It pulls in NO
//! new dependencies — `oxrdf` (`Term`, `Variable`, `VariableRef`) is already a direct
//! dependency — and contains no `unsafe`.

use crate::QueryResult;
use oxrdf::{Term, Variable, VariableRef};

impl QueryResult {
    /// Iterates the result table as Oxigraph-shaped [`QuerySolution`] views — one per
    /// row, each a borrowed zero-copy view sharing this result's `vars` header.
    ///
    /// The iterator is lazy and allocation-free: it walks `rows` by index and hands
    /// out a [`QuerySolution`] that borrows the shared header and the row in place. No
    /// terms are cloned and the columnar table is not re-materialised.
    ///
    /// ```
    /// # use sparq_core::Graph;
    /// let g = Graph::load_str(
    ///     "<http://ex/a> <http://ex/p> <http://ex/b> .",
    ///     "turtle",
    /// )
    /// .unwrap();
    /// let r = sparq_engine::query(&g, "SELECT ?s ?o WHERE { ?s ?p ?o }").unwrap();
    /// for sol in r.solutions() {
    ///     // Oxigraph-style per-solution access.
    ///     let s = sol.get("s").unwrap();
    ///     assert_eq!(s, &sol["s"]);
    /// }
    /// ```
    pub fn solutions(&self) -> QuerySolutionIter<'_> {
        QuerySolutionIter { vars: &self.vars, rows: &self.rows, pos: 0 }
    }
}

/// A borrowed, zero-copy view of a single [`QueryResult`] row, shaped like Oxigraph's
/// `QuerySolution`: a per-solution map from `Variable` to bound `Term`.
///
/// Constructed by [`QueryResult::solutions`]. It holds only a reference to the shared
/// `vars` header and to the row slice — no owned terms, no per-row map — so it is cheap
/// to create and to pass around. `None` from [`get`](Self::get) means the variable is
/// either absent from the projection **or** unbound in this row, matching Oxigraph.
#[derive(Debug, Clone, Copy)]
pub struct QuerySolution<'a> {
    vars: &'a [Variable],
    row: &'a [Option<Term>],
}

impl<'a> QuerySolution<'a> {
    /// The variable header shared by every solution of the result (the projected
    /// SELECT variables, in projection order). Same slice for every row.
    pub fn variables(&self) -> &'a [Variable] {
        self.vars
    }

    /// Looks up the bound [`Term`] for a variable. Accepts a `&str` name (without the
    /// leading `?`), a [`VariableRef`] / `&Variable` (via [`VarIndex`]), or a
    /// positional `usize`. Returns `None` if the variable is not projected **or** is
    /// unbound in this solution — the Oxigraph `QuerySolution::get` contract.
    pub fn get<V: VarIndex>(&self, var: V) -> Option<&'a Term> {
        var.position(self.vars).and_then(|i| self.row.get(i).and_then(Option::as_ref))
    }

    /// Iterates the **bound** `(VariableRef, &Term)` pairs of this solution, skipping
    /// unbound cells — the Oxigraph `QuerySolution::iter` semantics. Pairs are yielded
    /// in projection (variable header) order.
    pub fn iter(&self) -> QuerySolutionBindings<'a> {
        QuerySolutionBindings { vars: self.vars, row: self.row, pos: 0 }
    }

    /// The bound `Term`s of this solution in projection order, skipping unbound cells.
    /// The value half of [`iter`](Self::iter).
    pub fn values(&self) -> impl Iterator<Item = &'a Term> {
        self.row.iter().filter_map(Option::as_ref)
    }

    /// The number of **bound** variables in this solution (unbound cells excluded), as
    /// in Oxigraph. Note this can be smaller than `variables().len()`.
    pub fn len(&self) -> usize {
        self.row.iter().filter(|c| c.is_some()).count()
    }

    /// Whether this solution binds no variable at all (every projected cell unbound).
    pub fn is_empty(&self) -> bool {
        self.row.iter().all(Option::is_none)
    }
}

impl<'a> IntoIterator for QuerySolution<'a> {
    type Item = (VariableRef<'a>, &'a Term);
    type IntoIter = QuerySolutionBindings<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a> IntoIterator for &QuerySolution<'a> {
    type Item = (VariableRef<'a>, &'a Term);
    type IntoIter = QuerySolutionBindings<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// `&solution[var]` — panicking accessor, mirroring Oxigraph's `Index` impl. Use
/// [`QuerySolution::get`] for the non-panicking form. Indexable by `&str`,
/// [`VariableRef`] / `&Variable`, or positional `usize`.
impl<V: VarIndex> core::ops::Index<V> for QuerySolution<'_> {
    type Output = Term;

    fn index(&self, var: V) -> &Term {
        self.get(var).expect("the variable is not bound in this query solution")
    }
}

/// Iterator of [`QuerySolution`] views over a [`QueryResult`], returned by
/// [`QueryResult::solutions`]. Lazy and allocation-free.
#[derive(Debug, Clone)]
pub struct QuerySolutionIter<'a> {
    vars: &'a [Variable],
    rows: &'a [Vec<Option<Term>>],
    pos: usize,
}

impl<'a> Iterator for QuerySolutionIter<'a> {
    type Item = QuerySolution<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let row = self.rows.get(self.pos)?;
        self.pos += 1;
        Some(QuerySolution { vars: self.vars, row })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.rows.len() - self.pos;
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for QuerySolutionIter<'_> {}

/// Iterator of the **bound** `(VariableRef, &Term)` pairs of one [`QuerySolution`],
/// returned by [`QuerySolution::iter`]. Skips unbound cells; projection order.
#[derive(Debug, Clone)]
pub struct QuerySolutionBindings<'a> {
    vars: &'a [Variable],
    row: &'a [Option<Term>],
    pos: usize,
}

impl<'a> Iterator for QuerySolutionBindings<'a> {
    type Item = (VariableRef<'a>, &'a Term);

    fn next(&mut self) -> Option<Self::Item> {
        while self.pos < self.row.len() {
            let i = self.pos;
            self.pos += 1;
            if let Some(term) = &self.row[i] {
                // `vars` and `row` are parallel (same projection); `i < row.len()` and
                // the result invariant gives `row.len() == vars.len()`, so this indexes
                // safely.
                return Some((self.vars[i].as_ref(), term));
            }
        }
        None
    }
}

/// Lookup strategy for indexing a [`QuerySolution`] by variable: a `&str` name, a
/// [`VariableRef`] / `&Variable`, or a positional `usize`. Mirrors the key types
/// Oxigraph's `QuerySolution::get` / `Index` accept.
pub trait VarIndex {
    /// Resolves this key to a positional index into the projection `vars` header, or
    /// `None` if no projected variable matches.
    fn position(self, vars: &[Variable]) -> Option<usize>;
}

impl VarIndex for usize {
    fn position(self, vars: &[Variable]) -> Option<usize> {
        (self < vars.len()).then_some(self)
    }
}

impl VarIndex for &str {
    fn position(self, vars: &[Variable]) -> Option<usize> {
        // Accept both the bare name (`"x"`) and the SPARQL-rendered `"?x"` form so a
        // caller can pass either; the engine's `Variable::as_str` is the bare name.
        let name = self.strip_prefix('?').unwrap_or(self);
        vars.iter().position(|v| v.as_str() == name)
    }
}

impl VarIndex for VariableRef<'_> {
    fn position(self, vars: &[Variable]) -> Option<usize> {
        vars.iter().position(|v| v.as_ref() == self)
    }
}

impl VarIndex for &Variable {
    fn position(self, vars: &[Variable]) -> Option<usize> {
        vars.iter().position(|v| v == self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparq_core::Graph;

    const DATA: &str = r#"@prefix ex: <http://ex/> .
        ex:a ex:p ex:b .
        ex:a ex:q "lit" .
        ex:c ex:p ex:d .
    "#;

    fn g() -> Graph {
        Graph::load_str(DATA, "turtle").unwrap()
    }

    /// `solutions()` yields one view per row, sharing the `vars` header, and `get`
    /// resolves by name / ref / position to the SAME term the columnar `rows` hold.
    #[test]
    fn solutions_match_columnar_rows() {
        let r = super::super::query(&g(), "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE { ?s ex:p ?o } ORDER BY ?s").unwrap();
        let sols: Vec<_> = r.solutions().collect();
        assert_eq!(sols.len(), r.rows.len());
        assert_eq!(r.solutions().len(), r.rows.len(), "ExactSizeIterator len");
        for (sol, row) in sols.iter().zip(&r.rows) {
            assert_eq!(sol.variables(), &r.vars[..]);
            // by name, by ?-name, by VariableRef, by &Variable, by position — all agree.
            let by_name = sol.get("s");
            assert_eq!(by_name, sol.get("?s"));
            assert_eq!(by_name, sol.get(VariableRef::new_unchecked("s")));
            assert_eq!(by_name, sol.get(&r.vars[0]));
            assert_eq!(by_name, sol.get(0usize));
            // and it equals the raw columnar cell.
            assert_eq!(by_name, row[0].as_ref());
            assert_eq!(sol.get("o"), row[1].as_ref());
        }
    }

    /// Absent variable, out-of-range position, and unbound cell all return `None`.
    #[test]
    fn missing_and_unbound_are_none() {
        // OPTIONAL leaves ?o unbound on the row where ex:nope matches nothing.
        let r = super::super::query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE { ?s ex:p ?x OPTIONAL { ?s ex:nope ?o } } ORDER BY ?s",
        )
        .unwrap();
        let sol = r.solutions().next().unwrap();
        assert!(sol.get("missing").is_none(), "absent variable -> None");
        assert!(sol.get("?missing").is_none());
        assert!(sol.get(99usize).is_none(), "out-of-range position -> None");
        assert!(sol.get("o").is_none(), "unbound cell -> None");
        assert!(sol.get("s").is_some(), "bound cell -> Some");
        // len/is_empty count only BOUND cells.
        assert_eq!(sol.len(), 1, "only ?s is bound");
        assert!(!sol.is_empty());
    }

    /// `iter()` yields only the BOUND pairs, in projection order, and round-trips
    /// through `get`. `IntoIterator` on a borrow agrees.
    #[test]
    fn iter_yields_bound_pairs_only() {
        let r = super::super::query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE { ?s ex:p ?x OPTIONAL { ?s ex:nope ?o } } ORDER BY ?s",
        )
        .unwrap();
        let sol = r.solutions().next().unwrap();
        let pairs: Vec<_> = sol.iter().collect();
        assert_eq!(pairs.len(), 1, "unbound ?o is skipped");
        let (var, term) = pairs[0];
        assert_eq!(var.as_str(), "s");
        assert_eq!(Some(term), sol.get("s"));
        // borrow-IntoIterator agrees with iter().
        let via_for: Vec<_> = (&sol).into_iter().collect();
        assert_eq!(via_for, pairs);
        // values() is the value half.
        let vals: Vec<_> = sol.values().collect();
        assert_eq!(vals, vec![term]);
    }

    /// The panicking `Index` accessor returns the bound term and matches `get`.
    #[test]
    fn index_returns_bound_term() {
        let r = super::super::query(&g(), "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE { ?s ex:p ?o } ORDER BY ?s").unwrap();
        let sol = r.solutions().next().unwrap();
        assert_eq!(&sol["s"], sol.get("s").unwrap());
        assert_eq!(&sol[0usize], sol.get(0usize).unwrap());
        assert_eq!(&sol[VariableRef::new_unchecked("o")], sol.get("o").unwrap());
    }

    /// `Index` panics on a missing binding, like Oxigraph.
    #[test]
    #[should_panic(expected = "not bound")]
    fn index_panics_on_missing() {
        let r = super::super::query(&g(), "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:p ?o }").unwrap();
        let sol = r.solutions().next().unwrap();
        let _ = &sol["does_not_exist"];
    }

    /// An empty result yields no solutions.
    #[test]
    fn empty_result_no_solutions() {
        let r = super::super::query(&g(), "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:never ?o }").unwrap();
        assert_eq!(r.solutions().count(), 0);
    }
}
