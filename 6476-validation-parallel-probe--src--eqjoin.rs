//! [FABLE-5] (sq-7d3dj.30.7) Equality-FILTER → value-join unification, behind the
//! opt-in `value-join` cargo feature (OFF by default; when off none of this compiles
//! and the conjunctive path is byte-identical to before).
//!
//! 🤖 SPARQ agent. A conjunctive group whose triple patterns split into variable-
//! disjoint connected components, glued only by a `FILTER` equality between two
//! variables (`FILTER(?a = ?b)` — SP2Bench q05a/q12a's star-intersection shape), is
//! currently evaluated as the full cross product of the components with the `=`
//! re-evaluated per emitted row. This pass evaluates each component separately and
//! joins them with a HASH JOIN keyed on the equality instead, turning an
//! `O(|L| × |R|)` product into `O(|L| + |R| + |output|)` for the keyable rows.
//!
//! ## Why an id-keyed join would be WRONG, and what is done instead
//!
//! SPARQL `=` is VALUE equality, not term identity: `"01"^^xsd:integer =
//! "1.0"^^xsd:decimal` is TRUE with two different dictionary ids, comparing
//! incompatible types is a TYPE ERROR (the row is eliminated), and high-precision
//! `xsd:decimal`s that collapse to one `f64` must NOT be reported equal (the
//! sq-lr2ii class). The pass is sound by a two-layer construction:
//!
//! 1. **No false negatives** — each row's join key ([`JKey`], via [`key_of`]) is
//!    chosen so that whenever the engine's own evaluator ([`equal_expr`]) would
//!    return TRUE for a pair, the two keys are EQUAL. Per term class:
//!    IRIs / blank nodes / unknown-datatype / ill-formed literals can only be
//!    `=`-TRUE via term identity → keyed by dictionary id; numeric literals are
//!    `=`-TRUE only when their cached `f64` values agree (mirrors `eval_numeric`,
//!    which the evaluator's numeric fast path uses) → keyed by the canonicalised
//!    `f64` bits from the SAME `Graph::numeric_value` cache (`-0.0` folded into
//!    `+0.0`); `xsd:string` by string value; language-tagged literals by
//!    (lowercased tag, value); well-formed `xsd:boolean` by parsed value. Rows
//!    with no provable key — local-vocab ids, RDF 1.2 triple terms,
//!    value-comparable temporals (`xsd:dateTime`/`dateTimeStamp`/`date`, where
//!    different lexicals can denote one instant), and numeric-DATATYPE literals
//!    that missed the numeric cache (as of sq-6b1lj the cache acceptance ⟺
//!    `Num::of_literal`, so a miss is a genuinely ill-formed-for-datatype lexical
//!    the exact evaluator also type-errors — `Hard` defers to it) — fall into a
//!    [`JKey::Hard`]
//!    class that is paired by running the EXACT evaluator (`equal_expr`) against
//!    every row of the other side: exactly today's semantics for those rows.
//! 2. **False positives are rechecked** — the pass NEVER consumes the FILTER: the
//!    original filter expressions (including the equality) are re-applied verbatim
//!    over the joined rows, so a key collision (e.g. two distinct high-precision
//!    decimals sharing an `f64`) is eliminated by the same `equal_expr` /
//!    `cmp_decimal_str` exact recheck that runs today — the sq-lr2ii decline
//!    machinery is inherited, not re-implemented.
//!
//! The candidate pair set is therefore a duplicate-free superset of the surviving
//! pairs and a subset of the full product, and the verbatim residual FILTER pass
//! reduces it to exactly today's multiset. (Row ORDER may differ from the
//! product-order path, as with any join reorder; SPARQL solution sequences are
//! unordered until `ORDER BY`.)
//!
//! ## Eligibility (everything else DECLINES to the existing path)
//!
//! * The group is conjunctive (`is_conjunctive`) with ≥ 2 patterns and no RDF 1.2
//!   quoted-triple patterns.
//! * Every group FILTER is a TOTAL expression ([`expr_total`]): built only from
//!   variables/constants, the logical + comparison + arithmetic operators, `IN`,
//!   `IF`, `COALESCE`, `BOUND` and `sameTerm` — no function calls (`RAND`/`UUID`/
//!   `BNODE` are row-nondeterministic, custom functions and `EXISTS` can raise
//!   evaluation errors). Totality means a filter can neither hard-error nor
//!   observe evaluation order, which is what makes splitting the filter list
//!   across components (and re-ordering it around the join) result-invariant.
//! * At least one filter has a TOP-LEVEL conjunct `?a = ?b` (possibly inside a
//!   conjunction) whose variables are pattern variables of two DIFFERENT
//!   connected components. Top-level-conjunct position matters: a surviving row
//!   needs that conjunct to be TRUE (false and type-error both eliminate), which
//!   is exactly join semantics. An equality under `||` / `!` / `IF` is never used
//!   as a key.
//! * No query budget is armed (`budget::active()`), mirroring the vectorized
//!   seam's I3 rule: the budget's debit schedule is plan-shaped, so the safe
//!   answer is the verbatim plan. With the `zk` feature, an armed proof trace
//!   also declines (the recorded FILTER obligation set must cover the full
//!   product-side operand stream, exactly as `columnar_filter` I2 declines).
//!
//! Filters whose variables all live inside ONE component are pushed into that
//! component's evaluation (`eval_flat_conjunctive`, so sargable pushdown still
//! applies); every other filter — including the join equalities — stays residual
//! and runs verbatim after the joins.

use super::*;
use std::cell::Cell;

thread_local! {
    /// Test-only kill switch: `true` forces [`try_eq_component_join`] to decline, so
    /// a differential test can evaluate the SAME query through the verbatim path on
    /// the same build.
    static DISABLED: Cell<bool> = const { Cell::new(false) };
    /// Count of times the pass FIRED (committed to the value-join plan) on this
    /// thread — the anti-vacuity witness for tests: eligible shapes must increment
    /// it, decline shapes must not.
    static FIRED: Cell<u64> = const { Cell::new(0) };
}

#[cfg(test)]
fn set_disabled(v: bool) {
    DISABLED.with(|d| d.set(v));
}

#[cfg(test)]
fn fired() -> u64 {
    FIRED.with(|f| f.get())
}

/// The value-join attempt over a flattened conjunctive group. `Ok(None)` = declined
/// (the caller runs the verbatim path); `Ok(Some(b))` = the complete, filtered result
/// of the group (residual filters already applied). See the module docs for the
/// eligibility conditions and the soundness argument.
pub(super) fn try_eq_component_join(
    graph: &Graph,
    local: &mut LocalVocab,
    patterns: &[TriplePattern],
    filters: &[Expression],
) -> Result<Option<Bindings>, String> {
    if DISABLED.with(|d| d.get()) {
        return Ok(None);
    }
    // zk proof-trace armed: decline so the scalar path records the complete per-row
    // FILTER obligation set over the full product (mirrors `columnar_filter` I2).
    #[cfg(feature = "zk")]
    if crate::zk::enabled() {
        return Ok(None);
    }
    // Budget armed: decline (mirrors `columnar_filter` I3) — the row/byte debit
    // schedule is plan-shaped, so the verbatim plan is the safe answer.
    if budget::active() {
        return Ok(None);
    }
    if patterns.len() < 2 || filters.is_empty() {
        return Ok(None);
    }
    // Quoted-triple patterns bind variables inside nested triple terms; keep those
    // shapes on the verbatim path rather than reason about their connectivity.
    if patterns.iter().any(has_quoted_triple_term) {
        return Ok(None);
    }
    // Every filter must be total (no hard errors, no row-nondeterminism) for the
    // filter-list split + re-ordering around the join to be result-invariant.
    if !filters.iter().all(expr_total) {
        return Ok(None);
    }

    // Connected components of the pattern set (patterns sharing a variable join
    // anyway; only DISCONNECTED components cross-product today).
    let (comp_of_pattern, n_comps) = pattern_components(patterns);
    if n_comps < 2 {
        return Ok(None);
    }
    let mut comp_vars: Vec<FxHashSet<Variable>> = vec![FxHashSet::default(); n_comps];
    let mut var_comp: FxHashMap<Variable, usize> = FxHashMap::default();
    for (i, tp) in patterns.iter().enumerate() {
        for v in collect_vars(std::slice::from_ref(tp)) {
            var_comp.insert(v.clone(), comp_of_pattern[i]);
            comp_vars[comp_of_pattern[i]].insert(v);
        }
    }

    // Join edges: top-level `?a = ?b` conjuncts whose variables are pattern
    // variables of two different components.
    let mut edges: Vec<(Variable, Variable)> = Vec::new();
    for f in filters {
        for c in top_conjuncts(f) {
            if let Some((a, b)) = as_var_eq(c) {
                match (var_comp.get(a), var_comp.get(b)) {
                    (Some(ca), Some(cb)) if ca != cb => edges.push((a.clone(), b.clone())),
                    _ => {}
                }
            }
        }
    }
    if edges.is_empty() {
        return Ok(None);
    }

    // Committed: everything below returns `Some`.
    FIRED.with(|f| f.set(f.get() + 1));

    // Split the filters: a filter whose variables all live in one component is
    // evaluated WITH that component (sargable pushdown included); everything else —
    // the join equalities, cross-component filters, variable-free filters and
    // filters over variables bound nowhere — stays residual. Totality (checked
    // above) makes this split order-invariant.
    let mut comp_filters: Vec<Vec<Expression>> = vec![Vec::new(); n_comps];
    let mut residual: Vec<Expression> = Vec::new();
    for f in filters {
        let mut vs = FxHashSet::default();
        expr_vars(f, &mut vs);
        match (0..n_comps).find(|&i| !vs.is_empty() && vs.is_subset(&comp_vars[i])) {
            Some(ci) => comp_filters[ci].push(f.clone()),
            None => residual.push(f.clone()),
        }
    }

    // Evaluate each component through exactly the machinery the whole group uses.
    let mut clusters: Vec<Option<Bindings>> = Vec::with_capacity(n_comps);
    for (ci, cf) in comp_filters.into_iter().enumerate() {
        let pats: Vec<TriplePattern> = patterns
            .iter()
            .enumerate()
            .filter(|(i, _)| comp_of_pattern[*i] == ci)
            .map(|(_, tp)| tp.clone())
            .collect();
        clusters.push(Some(eval_flat_conjunctive(graph, local, &pats, cf)?));
    }

    // Merge clusters along the equality edges (each merge is one value join). An
    // edge whose endpoints already share a cluster stays a residual recheck only.
    let mut cluster_of_comp: Vec<usize> = (0..n_comps).collect();
    loop {
        let mut merged = false;
        for (a, b) in &edges {
            let ca = cluster_of_comp[var_comp[a]];
            let cb = cluster_of_comp[var_comp[b]];
            if ca == cb {
                continue;
            }
            let left = clusters[ca].take().expect("live cluster");
            let right = clusters[cb].take().expect("live cluster");
            clusters[ca] = Some(value_join(graph, local, left, a, right, b)?);
            for c in cluster_of_comp.iter_mut() {
                if *c == cb {
                    *c = ca;
                }
            }
            merged = true;
            break;
        }
        if !merged {
            break;
        }
    }

    // Any clusters still unconnected cross-product exactly as today (join of
    // variable-disjoint relations), then the residual filters run verbatim — the
    // join equalities among them are the exact recheck that kills key collisions.
    let mut live = clusters.into_iter().flatten();
    let mut b = live.next().expect("at least one cluster");
    for next in live {
        b = join_bindings(b, next);
    }
    for f in &residual {
        apply_filter(graph, local, &mut b, f)?;
    }
    Ok(Some(b))
}

/// Connected components of the pattern set under the "shares a variable" relation:
/// returns (component index per pattern, component count). Component indices are
/// dense and ordered by first appearance.
fn pattern_components(patterns: &[TriplePattern]) -> (Vec<usize>, usize) {
    let n = patterns.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut [usize], x: usize) -> usize {
        let mut r = x;
        while parent[r] != r {
            r = parent[r];
        }
        let mut c = x;
        while parent[c] != r {
            let next = parent[c];
            parent[c] = r;
            c = next;
        }
        r
    }
    let mut first_use: FxHashMap<Variable, usize> = FxHashMap::default();
    for (i, tp) in patterns.iter().enumerate() {
        for v in collect_vars(std::slice::from_ref(tp)) {
            match first_use.get(&v) {
                Some(&j) => {
                    let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                    if ri != rj {
                        parent[ri] = rj;
                    }
                }
                None => {
                    first_use.insert(v, i);
                }
            }
        }
    }
    let mut label: FxHashMap<usize, usize> = FxHashMap::default();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let r = find(&mut parent, i);
        let next = label.len();
        out.push(*label.entry(r).or_insert(next));
    }
    let k = label.len();
    (out, k)
}

/// TOTAL filter expressions: evaluation can neither return a hard `Err` nor observe
/// per-row evaluation order/count. Structurally: variables, constants, the logical /
/// comparison / equality / arithmetic operators, `IN`, `IF`, `COALESCE`, `BOUND` and
/// `sameTerm` — whose `eval_expr` arms only propagate errors from sub-expressions
/// (leaves never fail). `FunctionCall` is excluded wholesale (`RAND`/`UUID`/
/// `STRUUID`/`BNODE` are row-nondeterministic, custom functions may error), as is
/// `EXISTS` (re-enters pattern evaluation; also never reaches here — `is_conjunctive`
/// already rejects it).
fn expr_total(e: &Expression) -> bool {
    use Expression::*;
    match e {
        NamedNode(_) | Literal(_) | Variable(_) | Bound(_) => true,
        And(a, b) | Or(a, b) | Equal(a, b) | SameTerm(a, b) | Greater(a, b)
        | GreaterOrEqual(a, b) | Less(a, b) | LessOrEqual(a, b) | Add(a, b)
        | Subtract(a, b) | Multiply(a, b) | Divide(a, b) => expr_total(a) && expr_total(b),
        UnaryPlus(a) | UnaryMinus(a) | Not(a) => expr_total(a),
        In(a, list) => expr_total(a) && list.iter().all(expr_total),
        If(c, t, f) => expr_total(c) && expr_total(t) && expr_total(f),
        Coalesce(list) => list.iter().all(expr_total),
        FunctionCall(..) | Exists(_) => false,
    }
}

/// Flattens a top-level `&&` tree into its conjuncts (left-to-right); a non-`And`
/// expression is a single conjunct. A surviving FILTER row needs EVERY conjunct
/// TRUE (false and type-error both eliminate), which is what licenses treating one
/// equality conjunct as a join key.
fn top_conjuncts(e: &Expression) -> Vec<&Expression> {
    match e {
        Expression::And(a, b) => {
            let mut v = top_conjuncts(a);
            v.extend(top_conjuncts(b));
            v
        }
        other => vec![other],
    }
}

/// `Some((a, b))` iff `e` is exactly `?a = ?b` for two distinct variables.
fn as_var_eq(e: &Expression) -> Option<(&Variable, &Variable)> {
    if let Expression::Equal(x, y) = e {
        if let (Expression::Variable(a), Expression::Variable(b)) = (x.as_ref(), y.as_ref()) {
            if a != b {
                return Some((a, b));
            }
        }
    }
    None
}

/// Collects every variable an expression reads (including `BOUND`). Only called on
/// [`expr_total`] shapes, so `EXISTS` / `FunctionCall` are unreachable — handled
/// conservatively anyway (their variables are collected, keeping the ⊆-component
/// check safe if the whitelist ever widens).
fn expr_vars(e: &Expression, out: &mut FxHashSet<Variable>) {
    use Expression::*;
    match e {
        Variable(v) | Bound(v) => {
            out.insert(v.clone());
        }
        NamedNode(_) | Literal(_) => {}
        And(a, b) | Or(a, b) | Equal(a, b) | SameTerm(a, b) | Greater(a, b)
        | GreaterOrEqual(a, b) | Less(a, b) | LessOrEqual(a, b) | Add(a, b)
        | Subtract(a, b) | Multiply(a, b) | Divide(a, b) => {
            expr_vars(a, out);
            expr_vars(b, out);
        }
        UnaryPlus(a) | UnaryMinus(a) | Not(a) => expr_vars(a, out),
        In(a, list) => {
            expr_vars(a, out);
            for x in list {
                expr_vars(x, out);
            }
        }
        If(c, t, f) => {
            expr_vars(c, out);
            expr_vars(t, out);
            expr_vars(f, out);
        }
        Coalesce(list) | FunctionCall(_, list) => {
            for x in list {
                expr_vars(x, out);
            }
        }
        Exists(p) => {
            let mut vs = FxHashSet::default();
            collect_pattern_vars(p, &mut vs);
            out.extend(vs);
        }
    }
}

/// A row's join key. The load-bearing property (module docs): whenever the engine's
/// evaluator would return TRUE for `=` on a pair of bound terms, their keys are
/// EQUAL — so keyed hashing never loses a pair, and the residual FILTER recheck
/// removes any collision the coarser key admits.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum JKey {
    /// No provable key: pair via the exact evaluator against every opposite row.
    Hard,
    /// Term identity is the only TRUE route (IRI, blank node, unknown/ill-formed
    /// literal classes): the dictionary id.
    Term(Id),
    /// Numeric literal: canonicalised bits of the SAME `f64` the evaluator's
    /// numeric fast path compares (`Graph::numeric_value`; `-0.0` folded to `+0.0`).
    Num(u64),
    /// `xsd:string` (plain literals included — RDF 1.1 gives them this datatype):
    /// the string VALUE, robust to any dictionary-level spelling split.
    Str(String),
    /// Language-tagged literal: (lowercased tag, value) — tags compare
    /// case-insensitively.
    LangLit(String, String),
    /// Well-formed `xsd:boolean` VALUE (`"1"` and `"true"` are `=`-equal).
    Bool(bool),
}

/// `is_numeric_dt` by datatype-IRI string (the dict's `term_parts` hands out the
/// datatype without materialising a `Literal`): the numeric-datatype family whose
/// `=` is decided by VALUE, exactly the set `lit_kind` sends to `Num::of_literal`.
fn is_numeric_datatype(dt: &str) -> bool {
    sparq_core::is_integer_datatype(dt)
        || dt == xsd::DECIMAL.as_str()
        || dt == xsd::DOUBLE.as_str()
        || dt == xsd::FLOAT.as_str()
}

/// Classifies one bound id into its [`JKey`]. Mirrors the evaluator's dispatch
/// order in `equal_expr`: the numeric-cache route first (`eval_numeric` reads the
/// same cache), then the temporal cache (value-comparable ⇒ `Hard`), then the term
/// classes. Local-vocab ids (query-computed terms) are `Hard` — the local vocab is
/// not consulted here, and BGP rows never contain them anyway.
fn key_of(graph: &Graph, id: Id) -> JKey {
    if id == NO_ID || is_local(id) {
        return JKey::Hard;
    }
    if let Some(v) = graph.numeric_value(id) {
        let v = if v == 0.0 { 0.0 } else { v };
        return JKey::Num(v.to_bits());
    }
    if graph.temporal_value(id).is_some() {
        return JKey::Hard;
    }
    // Inline-integer ids never reach here (`numeric_value` is total for them).
    match graph.dict.term_parts(id) {
        dict::TermParts::Iri { .. } | dict::TermParts::Blank(_) => JKey::Term(id),
        dict::TermParts::Triple(_) => JKey::Hard,
        dict::TermParts::Lit { value, datatype, lang } => {
            if let Some(tag) = lang {
                return JKey::LangLit(tag.to_ascii_lowercase(), value.to_string());
            }
            if datatype == xsd::STRING.as_str() {
                return JKey::Str(value.to_string());
            }
            if datatype == xsd::BOOLEAN.as_str() {
                return match value {
                    "true" | "1" => JKey::Bool(true),
                    "false" | "0" => JKey::Bool(false),
                    // Ill-formed boolean: term identity is the only TRUE route.
                    _ => JKey::Term(id),
                };
            }
            // A NUMERIC datatype reaching here missed the `numeric_value` cache. As of
            // sq-9781x + sq-6b1lj the cache is aligned with the DATATYPE-AWARE
            // `Num::of_literal` acceptance set (the same set `values_equal` uses): it trims
            // (XSD `collapse` facet) and gates on per-datatype well-formedness, so a
            // whitespace-padded `" 1"^^xsd:integer` HITS the cache above (→ `JKey::Num`), the
            // `inf`/`nan` Rust-only spellings MISS, AND a lexical ill-formed FOR its datatype
            // (`"1.5"^^xsd:integer`, `"1E2"^^xsd:decimal`, an i128-overflow `xsd:decimal`)
            // now MISSES too — reaching THIS branch. Such a lexical is a SPARQL type error
            // under `values_equal`/`of_literal`, so `Hard` (defer to the exact evaluator,
            // which pairs it against nothing on `=`) is the correct backstop: the fast
            // `JKey::Num` route and `values_equal` now AGREE for every numeric lexical (the
            // pre-fix `JKey::Num`-vs-`values_equal` residual is closed). A numeric datatype
            // reaching `Hard` is exactly the ill-formed/`NaN`-sentinel case the exact
            // evaluator also type-errors — no false negative.
            if is_numeric_datatype(datatype) {
                return JKey::Hard;
            }
            // Well-formed temporals were caught above; an ill-formed temporal lexical
            // lands here and — like every remaining datatype family — is `=`-TRUE only
            // via term identity (same-datatype comparison is lexical, i.e. identity;
            // identical (lexical, datatype) interns to one id).
            JKey::Term(id)
        }
    }
}

/// Hash-joins two variable-disjoint relations on `left.?lv = right.?rv` under SPARQL
/// VALUE-equality semantics: keyable rows go through an id-free [`JKey`] hash table;
/// [`JKey::Hard`] rows are paired by the exact evaluator against every opposite row
/// (today's semantics for exactly those rows). Emits `left ++ right` rows; the
/// caller's residual FILTER pass performs the exact recheck on every emitted pair.
fn value_join(
    graph: &Graph,
    local: &LocalVocab,
    left: Bindings,
    lv: &Variable,
    right: Bindings,
    rv: &Variable,
) -> Result<Bindings, String> {
    let (Some(lc), Some(rc)) = (left.col(lv), right.col(rv)) else {
        // Defensive: an edge variable missing from its relation (cannot happen for
        // BGP-derived bindings) — fall back to the generic join (cross product).
        return Ok(join_bindings(left, right));
    };
    let lkeys: Vec<JKey> = left.rows.iter().map(|r| key_of(graph, r[lc])).collect();
    let rkeys: Vec<JKey> = right.rows.iter().map(|r| key_of(graph, r[rc])).collect();

    let mut out_vars = left.vars.clone();
    out_vars.extend(right.vars.iter().cloned());
    let mut rows: Vec<Row> = Vec::new();
    let emit = |rows: &mut Vec<Row>, l: &Row, r: &Row| {
        let mut row = l.clone();
        row.extend_from_slice(r);
        rows.push(row);
    };

    // Keyed rows: build on the right, probe with the left.
    let mut table: FxHashMap<&JKey, Vec<u32>> = FxHashMap::default();
    for (ri, k) in rkeys.iter().enumerate() {
        if !matches!(k, JKey::Hard) {
            table.entry(k).or_default().push(ri as u32);
        }
    }
    for (li, lk) in lkeys.iter().enumerate() {
        if matches!(lk, JKey::Hard) {
            continue;
        }
        if let Some(idx) = table.get(lk) {
            for &ri in idx {
                emit(&mut rows, &left.rows[li], &right.rows[ri as usize]);
            }
        }
    }

    // Hard rows: exact evaluator against every opposite row. The three loops
    // partition the product — keyed×keyed above, hard-left×all, keyed-left×hard-right
    // — so no pair is emitted twice.
    let scratch = Bindings::unsorted(vec![lv.clone(), rv.clone()], Vec::new());
    let (ea, eb) = (Expression::Variable(lv.clone()), Expression::Variable(rv.clone()));
    let pair_true = |ida: Id, idb: Id| -> Result<bool, String> {
        Ok(effective_boolean(&equal_expr(graph, local, &scratch, &[ida, idb], &ea, &eb)?))
    };
    for (li, lk) in lkeys.iter().enumerate() {
        if !matches!(lk, JKey::Hard) {
            continue;
        }
        let ida = left.rows[li][lc];
        for (ri, rrow) in right.rows.iter().enumerate() {
            if pair_true(ida, rrow[rc])? {
                emit(&mut rows, &left.rows[li], &right.rows[ri]);
            }
        }
    }
    for (ri, rk) in rkeys.iter().enumerate() {
        if !matches!(rk, JKey::Hard) {
            continue;
        }
        let idb = right.rows[ri][rc];
        for (li, lk) in lkeys.iter().enumerate() {
            if matches!(lk, JKey::Hard) {
                continue;
            }
            if pair_true(left.rows[li][lc], idb)? {
                emit(&mut rows, &left.rows[li], &right.rows[ri]);
            }
        }
    }
    Ok(Bindings::unsorted(out_vars, rows))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{query, query_with_budget, QueryBudget};

    const PFX: &str = concat!(
        "PREFIX ex: <http://ex/> ",
        "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ",
    );

    fn load(nt: &str) -> Graph {
        Graph::load_str(nt, "ntriples").unwrap()
    }

    /// Order-independent result bag: each row as sorted `var=term` cells, rows sorted.
    fn bag(g: &Graph, q: &str) -> Vec<Vec<String>> {
        let r = query(g, q).unwrap();
        let mut bag: Vec<Vec<String>> = r
            .rows
            .iter()
            .map(|row| {
                let mut cells: Vec<String> = r
                    .vars
                    .iter()
                    .zip(row.iter())
                    .map(|(v, cell)| match cell {
                        Some(t) => format!("{}={}", v, t),
                        None => format!("{}=UNBOUND", v),
                    })
                    .collect();
                cells.sort();
                cells
            })
            .collect();
        bag.sort();
        bag
    }

    /// The load-bearing differential: the SAME query through the value-join path
    /// (asserting it actually FIRED) and through the verbatim path (kill switch),
    /// which must produce the SAME bag. Returns the bag.
    fn assert_on_off_equal(g: &Graph, q: &str) -> Vec<Vec<String>> {
        let before = fired();
        let on = bag(g, q);
        assert!(fired() > before, "value-join must fire for this shape: {}", q);
        set_disabled(true);
        let off = bag(g, q);
        set_disabled(false);
        assert_eq!(on, off, "value-join result differs from the verbatim path: {}", q);
        on
    }

    /// Asserts the pass does NOT fire (decline witness) and returns the bag.
    fn assert_declines(g: &Graph, q: &str) -> Vec<Vec<String>> {
        let before = fired();
        let out = bag(g, q);
        assert_eq!(fired(), before, "value-join must DECLINE for this shape: {}", q);
        out
    }

    // ---- q05a-shaped acceptance: two stars glued only by FILTER(?n1 = ?n2) ----
    #[test]
    fn q05a_shape_matches_verbatim_and_independent_oracle() {
        let g = load(concat!(
            "<http://ex/art1> <http://ex/type> <http://ex/Article> .\n",
            "<http://ex/art1> <http://ex/creator> <http://ex/p1> .\n",
            "<http://ex/art2> <http://ex/type> <http://ex/Article> .\n",
            "<http://ex/art2> <http://ex/creator> <http://ex/p2> .\n",
            "<http://ex/p1> <http://ex/name> \"alice\" .\n",
            "<http://ex/p2> <http://ex/name> \"bob\" .\n",
            "<http://ex/inp1> <http://ex/itype> <http://ex/Inproc> .\n",
            "<http://ex/inp1> <http://ex/icreator> <http://ex/q1> .\n",
            "<http://ex/inp2> <http://ex/itype> <http://ex/Inproc> .\n",
            "<http://ex/inp2> <http://ex/icreator> <http://ex/q2> .\n",
            "<http://ex/q1> <http://ex/name> \"alice\" .\n",
            "<http://ex/q2> <http://ex/name> \"carol\" .\n",
        ));
        let q = format!(
            "{PFX} SELECT DISTINCT ?person ?name WHERE {{ \
               ?article ex:type ex:Article . ?article ex:creator ?person . \
               ?person ex:name ?name . \
               ?inproc ex:itype ex:Inproc . ?inproc ex:icreator ?person2 . \
               ?person2 ex:name ?name2 . \
               FILTER(?name = ?name2) }}"
        );
        let on = assert_on_off_equal(&g, &q);
        // Independent oracle: `!(?a != ?b)` has the same three-valued keep-set as
        // `?a = ?b` (error stays error, both eliminate) but is NOT a top-level
        // equality conjunct, so it takes the verbatim path.
        let oracle = assert_declines(&g, &q.replace("FILTER(?name = ?name2)", "FILTER(!(?name != ?name2))"));
        assert_eq!(on, oracle);
        // Exactly one intersecting author.
        assert_eq!(on.len(), 1);
        assert!(on[0].iter().any(|c| c.contains("alice")), "row: {:?}", on[0]);
    }

    // ---- SPARQL `=` is value equality with promotion: "01"^^integer = "1.0"^^decimal ----
    #[test]
    fn numeric_promotion_across_datatypes() {
        let g = load(concat!(
            "<http://ex/a> <http://ex/p> \"01\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
            "<http://ex/a> <http://ex/p> \"7\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
            "<http://ex/b> <http://ex/q> \"1.0\"^^<http://www.w3.org/2001/XMLSchema#decimal> .\n",
            "<http://ex/b> <http://ex/q> \"1.0E0\"^^<http://www.w3.org/2001/XMLSchema#double> .\n",
        ));
        let q = format!(
            "{PFX} SELECT ?x ?y WHERE {{ <http://ex/a> ex:p ?x . <http://ex/b> ex:q ?y . FILTER(?x = ?y) }}"
        );
        let on = assert_on_off_equal(&g, &q);
        // "01" pairs with BOTH "1.0"^^decimal and "1.0E0"^^double; "7" with neither.
        assert_eq!(on.len(), 2, "bag: {:?}", on);
        assert!(on.iter().all(|row| row.iter().any(|c| c.contains("01"))), "bag: {:?}", on);
    }

    // ---- incompatible types are a TYPE ERROR (row eliminated), IRI vs literal is false ----
    #[test]
    fn type_error_and_cross_kind_pairs_eliminated() {
        let g = load(concat!(
            "<http://ex/a> <http://ex/p> \"1\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
            "<http://ex/a> <http://ex/p> \"x\"^^<http://ex/dtA> .\n",
            "<http://ex/a> <http://ex/p> <http://ex/iri1> .\n",
            "<http://ex/b> <http://ex/q> \"a\" .\n",
            "<http://ex/b> <http://ex/q> \"y\"^^<http://ex/dtB> .\n",
            "<http://ex/b> <http://ex/q> \"x\"^^<http://ex/dtA> .\n",
        ));
        let q = format!(
            "{PFX} SELECT ?x ?y WHERE {{ <http://ex/a> ex:p ?x . <http://ex/b> ex:q ?y . FILTER(?x = ?y) }}"
        );
        let on = assert_on_off_equal(&g, &q);
        // Only the identical unknown-datatype term pairs (term identity decides);
        // integer-vs-string and dtA-vs-dtB are type errors, IRI-vs-literal is false.
        assert_eq!(on.len(), 1, "bag: {:?}", on);
        assert!(on[0].iter().all(|c| c.contains("\"x\"") || c.starts_with("?x=") || c.starts_with("?y=")), "bag: {:?}", on);
    }

    // ---- the sq-lr2ii class: high-precision decimals sharing an f64 stay UNEQUAL ----
    #[test]
    fn high_precision_decimal_not_collapsed_through_f64() {
        let g = load(concat!(
            "<http://ex/a> <http://ex/p> \"1.00000000000000001\"^^<http://www.w3.org/2001/XMLSchema#decimal> .\n",
            "<http://ex/a> <http://ex/p> \"9007199254740993\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
            "<http://ex/b> <http://ex/q> \"1.00000000000000002\"^^<http://www.w3.org/2001/XMLSchema#decimal> .\n",
            "<http://ex/b> <http://ex/q> \"9007199254740992\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
            "<http://ex/b> <http://ex/q> \"1.000000000000000010\"^^<http://www.w3.org/2001/XMLSchema#decimal> .\n",
        ));
        let q = format!(
            "{PFX} SELECT ?x ?y WHERE {{ <http://ex/a> ex:p ?x . <http://ex/b> ex:q ?y . FILTER(?x = ?y) }}"
        );
        let on = assert_on_off_equal(&g, &q);
        // The hash key collapses all three decimals to one f64 bucket and the two
        // big integers to another — the verbatim FILTER recheck (cmp_decimal_str)
        // must keep ONLY the exact-value pair 1.00000000000000001 = 1.000000000000000010.
        assert_eq!(on.len(), 1, "bag: {:?}", on);
        assert!(on[0].iter().any(|c| c.contains("1.000000000000000010")), "bag: {:?}", on);
    }

    // ---- a numeric lexical the CACHE rejects but the evaluator's fallback accepts:
    // `Num::of_literal` TRIMS, so " 1"^^xsd:integer is a cache miss yet value-equal
    // to "1"^^xsd:integer under a different id. Keying it by term identity would
    // silently DROP the pair; it must take the Hard (exact-evaluator) class. ----
    #[test]
    fn whitespace_padded_numeric_pairs_by_value() {
        let g = load(concat!(
            "<http://ex/a> <http://ex/p> \" 1\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
            "<http://ex/b> <http://ex/q> \"1\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
            "<http://ex/b> <http://ex/q> \"2\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
        ));
        let q = format!(
            "{PFX} SELECT ?x ?y WHERE {{ <http://ex/a> ex:p ?x . <http://ex/b> ex:q ?y . FILTER(?x = ?y) }}"
        );
        let on = assert_on_off_equal(&g, &q);
        assert_eq!(on.len(), 1, "bag: {:?}", on);
    }

    // ---- plain literal and explicit ^^xsd:string are the same value ----
    #[test]
    fn plain_literal_equals_xsd_string() {
        let g = load(concat!(
            "<http://ex/a> <http://ex/p> \"x\" .\n",
            "<http://ex/b> <http://ex/q> \"x\"^^<http://www.w3.org/2001/XMLSchema#string> .\n",
            "<http://ex/b> <http://ex/q> \"y\" .\n",
        ));
        let q = format!(
            "{PFX} SELECT ?x ?y WHERE {{ <http://ex/a> ex:p ?x . <http://ex/b> ex:q ?y . FILTER(?x = ?y) }}"
        );
        let on = assert_on_off_equal(&g, &q);
        assert_eq!(on.len(), 1, "bag: {:?}", on);
    }

    // ---- language tags: equal (case-insensitive) tag + value; never a plain string ----
    #[test]
    fn language_tagged_literals() {
        let g = load(concat!(
            "<http://ex/a> <http://ex/p> \"x\"@en .\n",
            "<http://ex/a> <http://ex/p> \"z\"@en-GB .\n",
            "<http://ex/b> <http://ex/q> \"x\"@en .\n",
            "<http://ex/b> <http://ex/q> \"x\"@fr .\n",
            "<http://ex/b> <http://ex/q> \"x\" .\n",
            "<http://ex/b> <http://ex/q> \"z\"@EN-GB .\n",
        ));
        let q = format!(
            "{PFX} SELECT ?x ?y WHERE {{ <http://ex/a> ex:p ?x . <http://ex/b> ex:q ?y . FILTER(?x = ?y) }}"
        );
        let on = assert_on_off_equal(&g, &q);
        // "x"@en = "x"@en, and "z"@en-GB = "z"@EN-GB (tags compare case-insensitively);
        // @en vs @fr, and lang vs plain, are known-different.
        assert_eq!(on.len(), 2, "bag: {:?}", on);
    }

    // ---- temporals are the Hard class: exact evaluator pairs them ----
    #[test]
    fn temporal_values_pair_exactly() {
        let g = load(concat!(
            "<http://ex/a> <http://ex/p> \"2020-01-01T12:00:00Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime> .\n",
            "<http://ex/a> <http://ex/p> \"2020-01-01T09:00:00\"^^<http://www.w3.org/2001/XMLSchema#dateTime> .\n",
            "<http://ex/b> <http://ex/q> \"2020-01-01T13:00:00+01:00\"^^<http://www.w3.org/2001/XMLSchema#dateTime> .\n",
            "<http://ex/b> <http://ex/q> \"2020-01-01\"^^<http://www.w3.org/2001/XMLSchema#date> .\n",
        ));
        let q = format!(
            "{PFX} SELECT ?x ?y WHERE {{ <http://ex/a> ex:p ?x . <http://ex/b> ex:q ?y . FILTER(?x = ?y) }}"
        );
        let on = assert_on_off_equal(&g, &q);
        // 12:00Z = 13:00+01:00 (same instant, different lexicals/ids); the tz-less
        // operand is indeterminate against both (type error), date is cross-family.
        assert_eq!(on.len(), 1, "bag: {:?}", on);
        assert!(on[0].iter().any(|c| c.contains("13:00:00+01:00")), "bag: {:?}", on);
    }

    // ---- xsd:boolean compares by VALUE: "1" = "true" ----
    #[test]
    fn boolean_value_equality() {
        let g = load(concat!(
            "<http://ex/a> <http://ex/p> \"true\"^^<http://www.w3.org/2001/XMLSchema#boolean> .\n",
            "<http://ex/b> <http://ex/q> \"1\"^^<http://www.w3.org/2001/XMLSchema#boolean> .\n",
            "<http://ex/b> <http://ex/q> \"false\"^^<http://www.w3.org/2001/XMLSchema#boolean> .\n",
        ));
        let q = format!(
            "{PFX} SELECT ?x ?y WHERE {{ <http://ex/a> ex:p ?x . <http://ex/b> ex:q ?y . FILTER(?x = ?y) }}"
        );
        let on = assert_on_off_equal(&g, &q);
        assert_eq!(on.len(), 1, "bag: {:?}", on);
        assert!(on[0].iter().any(|c| c.contains("\"1\"")), "bag: {:?}", on);
    }

    // ---- IRIs join by identity through the id key ----
    #[test]
    fn iri_identity_join() {
        let g = load(concat!(
            "<http://ex/a1> <http://ex/p> <http://ex/t1> .\n",
            "<http://ex/a2> <http://ex/p> <http://ex/t2> .\n",
            "<http://ex/b1> <http://ex/q> <http://ex/t1> .\n",
            "<http://ex/b2> <http://ex/q> <http://ex/t3> .\n",
        ));
        let q = format!(
            "{PFX} SELECT ?s ?t WHERE {{ ?s ex:p ?x . ?t ex:q ?y . FILTER(?x = ?y) }}"
        );
        let on = assert_on_off_equal(&g, &q);
        assert_eq!(on.len(), 1, "bag: {:?}", on);
        assert!(on[0].iter().any(|c| c.contains("a1")) && on[0].iter().any(|c| c.contains("b1")), "bag: {:?}", on);
    }

    // ---- a conjunction of equalities: one is the key, the rest recheck residually ----
    #[test]
    fn conjoined_equalities_between_two_components() {
        let g = load(concat!(
            "<http://ex/a1> <http://ex/p> \"k\" .\n<http://ex/a1> <http://ex/r> \"v1\" .\n",
            "<http://ex/a2> <http://ex/p> \"k\" .\n<http://ex/a2> <http://ex/r> \"v2\" .\n",
            "<http://ex/b1> <http://ex/q> \"k\" .\n<http://ex/b1> <http://ex/s> \"v1\" .\n",
            "<http://ex/b2> <http://ex/q> \"k\" .\n<http://ex/b2> <http://ex/s> \"vX\" .\n",
        ));
        let q = format!(
            "{PFX} SELECT ?a ?b WHERE {{ ?a ex:p ?k1 . ?a ex:r ?v1 . ?b ex:q ?k2 . ?b ex:s ?v2 . \
             FILTER(?k1 = ?k2 && ?v1 = ?v2) }}"
        );
        let on = assert_on_off_equal(&g, &q);
        assert_eq!(on.len(), 1, "bag: {:?}", on);
        assert!(on[0].iter().any(|c| c.contains("a1")) && on[0].iter().any(|c| c.contains("b1")), "bag: {:?}", on);
    }

    // ---- three components: two chained by equalities, one cross-products ----
    #[test]
    fn three_components_chain_plus_cartesian() {
        let g = load(concat!(
            "<http://ex/a> <http://ex/p> \"m\" .\n",
            "<http://ex/b> <http://ex/q> \"m\" .\n",
            "<http://ex/b> <http://ex/q> \"n\" .\n",
            "<http://ex/c> <http://ex/r> \"u1\" .\n",
            "<http://ex/c> <http://ex/r> \"u2\" .\n",
        ));
        let q = format!(
            "{PFX} SELECT ?x ?y ?z WHERE {{ <http://ex/a> ex:p ?x . <http://ex/b> ex:q ?y . \
             <http://ex/c> ex:r ?z . FILTER(?x = ?y) }}"
        );
        let on = assert_on_off_equal(&g, &q);
        // One matching (x, y) pair × two unconnected ?z rows.
        assert_eq!(on.len(), 2, "bag: {:?}", on);
    }

    // ---- component-local filters still apply (and sargable pushdown still runs) ----
    #[test]
    fn component_local_filter_pushdown() {
        let g = load(concat!(
            "<http://ex/a1> <http://ex/p> \"k\" .\n<http://ex/a1> <http://ex/n> \"5\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
            "<http://ex/a2> <http://ex/p> \"k\" .\n<http://ex/a2> <http://ex/n> \"50\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
            "<http://ex/b1> <http://ex/q> \"k\" .\n",
        ));
        let q = format!(
            "{PFX} SELECT ?a ?b WHERE {{ ?a ex:p ?k1 . ?a ex:n ?n . ?b ex:q ?k2 . \
             FILTER(?n > 10) FILTER(?k1 = ?k2) }}"
        );
        let on = assert_on_off_equal(&g, &q);
        assert_eq!(on.len(), 1, "bag: {:?}", on);
        assert!(on[0].iter().any(|c| c.contains("a2")), "bag: {:?}", on);
    }

    // ---- DECLINE witnesses: each shape must run the verbatim path ----
    #[test]
    fn declines_same_component_equality() {
        let g = load("<http://ex/s> <http://ex/p> \"v\" .\n<http://ex/s> <http://ex/q> \"v\" .\n");
        let q = format!(
            "{PFX} SELECT ?a ?b WHERE {{ ?s ex:p ?a . ?s ex:q ?b . FILTER(?a = ?b) }}"
        );
        let out = assert_declines(&g, &q);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn declines_equality_under_disjunction() {
        let g = load("<http://ex/a> <http://ex/p> \"v\" .\n<http://ex/b> <http://ex/q> \"v\" .\n");
        let q = format!(
            "{PFX} SELECT ?x ?y WHERE {{ <http://ex/a> ex:p ?x . <http://ex/b> ex:q ?y . \
             FILTER(?x = ?y || false) }}"
        );
        let out = assert_declines(&g, &q);
        assert_eq!(out.len(), 1, "the disjunction still keeps the row: {:?}", out);
    }

    #[test]
    fn declines_group_with_function_call_filter() {
        let g = load("<http://ex/a> <http://ex/p> \"v\" .\n<http://ex/b> <http://ex/q> \"v\" .\n");
        let q = format!(
            "{PFX} SELECT ?x ?y WHERE {{ <http://ex/a> ex:p ?x . <http://ex/b> ex:q ?y . \
             FILTER(?x = ?y) FILTER(STRLEN(?x) > 0) }}"
        );
        let out = assert_declines(&g, &q);
        assert_eq!(out.len(), 1, "bag: {:?}", out);
    }

    #[test]
    fn declines_when_budget_armed() {
        let g = load("<http://ex/a> <http://ex/p> \"v\" .\n<http://ex/b> <http://ex/q> \"v\" .\n");
        let q = format!(
            "{PFX} SELECT ?x ?y WHERE {{ <http://ex/a> ex:p ?x . <http://ex/b> ex:q ?y . FILTER(?x = ?y) }}"
        );
        let before = fired();
        let r = query_with_budget(&g, &q, &QueryBudget { max_rows: Some(1_000_000), ..Default::default() }).unwrap();
        assert_eq!(fired(), before, "must decline under an armed budget");
        assert_eq!(r.rows.len(), 1);
    }

    // ---- direct unit coverage of the helpers ----
    #[test]
    fn key_of_classifies_term_classes() {
        let g = load(concat!(
            "<http://ex/s> <http://ex/p> \"01\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
            "<http://ex/s> <http://ex/p> \"1.0\"^^<http://www.w3.org/2001/XMLSchema#decimal> .\n",
            "<http://ex/s> <http://ex/p> \"-0.0\"^^<http://www.w3.org/2001/XMLSchema#double> .\n",
            "<http://ex/s> <http://ex/p> \"0\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
            "<http://ex/s> <http://ex/p> \"str\" .\n",
            "<http://ex/s> <http://ex/p> \"tag\"@EN .\n",
            "<http://ex/s> <http://ex/p> \"true\"^^<http://www.w3.org/2001/XMLSchema#boolean> .\n",
            "<http://ex/s> <http://ex/p> \"1\"^^<http://www.w3.org/2001/XMLSchema#boolean> .\n",
            "<http://ex/s> <http://ex/p> \"2020-01-01T00:00:00Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime> .\n",
            "<http://ex/s> <http://ex/p> \" 7\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
            "<http://ex/s> <http://ex/p> \"u\"^^<http://ex/unknownDt> .\n",
        ));
        let id_of = |needle: &str| -> Id {
            (1..=g.dict.len() as Id)
                .find(|&i| format!("{}", g.dict.term(i)).contains(needle))
                .unwrap_or_else(|| panic!("term containing {} not interned", needle))
        };
        // Same numeric VALUE, three datatypes/lexicals -> one key (the "01"/"1.0" pair).
        assert_eq!(key_of(&g, id_of("\"01\"")), key_of(&g, id_of("\"1.0\"")));
        // -0.0 folds into +0.0. Small integers like "0" are INLINE ids (never in the
        // dictionary): `numeric_value` decodes them directly, and the keys agree.
        assert_eq!(key_of(&g, id_of("-0.0")), key_of(&g, dict::INLINE_BASE));
        // Strings key by value, language-tagged by (lowercased tag, value).
        assert_eq!(key_of(&g, id_of("\"str\"")), JKey::Str("str".to_string()));
        assert_eq!(key_of(&g, id_of("\"tag\"")), JKey::LangLit("en".to_string(), "tag".to_string()));
        // Well-formed booleans key by value: "true" and "1" agree.
        assert_eq!(key_of(&g, id_of("\"true\"")), JKey::Bool(true));
        assert_eq!(key_of(&g, id_of("\"1\"^^<http://www.w3.org/2001/XMLSchema#boolean>")), JKey::Bool(true));
        // Value-comparable temporals are the Hard class.
        assert_eq!(key_of(&g, id_of("2020-01-01T00:00:00Z")), JKey::Hard);
        // [FABLE-5] (sq-9781x) A whitespace-padded numeric lexical now HITS the aligned
        // numeric cache (trim-then-parse via the shared `parse_xsd_f64`) and keys as
        // `JKey::Num(7.0)` — the SAME key the inline id `7` gets — instead of the former
        // `Hard` (the cache/evaluator acceptance sets are now aligned, so it no longer needs
        // the exact-evaluator backstop for the whitespace case).
        assert_eq!(key_of(&g, id_of("\" 7\"")), JKey::Num(7.0f64.to_bits()));
        assert_eq!(key_of(&g, id_of("\" 7\"")), key_of(&g, dict::INLINE_BASE + 7));
        // Unknown datatype: identity key. IRIs: identity key.
        let uid = id_of("unknownDt");
        assert_eq!(key_of(&g, uid), JKey::Term(uid));
        let iri = id_of("http://ex/p");
        assert_eq!(key_of(&g, iri), JKey::Term(iri));
        // Unbound / local-vocab ids are Hard.
        assert_eq!(key_of(&g, NO_ID), JKey::Hard);
        assert_eq!(key_of(&g, LOCAL_BASE), JKey::Hard);
    }

    #[test]
    fn expr_total_whitelist() {
        let v = |n: &str| Expression::Variable(Variable::new(n).unwrap());
        let eq = Expression::Equal(Box::new(v("a")), Box::new(v("b")));
        assert!(expr_total(&eq));
        assert!(expr_total(&Expression::And(
            Box::new(eq.clone()),
            Box::new(Expression::Bound(Variable::new("c").unwrap()))
        )));
        // Any function call is out (row-nondeterminism / evaluation errors).
        assert!(!expr_total(&Expression::FunctionCall(spargebra::algebra::Function::Str, vec![v("a")])));
        assert!(!expr_total(&Expression::Not(Box::new(Expression::FunctionCall(
            spargebra::algebra::Function::Rand,
            vec![]
        )))));
    }

    #[test]
    fn pattern_components_split_and_merge() {
        let parse = |q: &str| -> Vec<TriplePattern> {
            let query = spargebra::SparqlParser::new().parse_query(q).unwrap();
            let spargebra::Query::Select { pattern, .. } = query else { unreachable!() };
            let mut pats = Vec::new();
            let mut filters = Vec::new();
            flatten_conjunction(&pattern_inner(&pattern), &mut pats, &mut filters);
            pats
        };
        fn pattern_inner(p: &GraphPattern) -> GraphPattern {
            match p {
                GraphPattern::Project { inner, .. } => pattern_inner(inner),
                other => other.clone(),
            }
        }
        // Chain: a-b, b-c share variables -> one component.
        let pats = parse("SELECT * WHERE { ?a <http://ex/p> ?b . ?b <http://ex/q> ?c }");
        let (labels, k) = pattern_components(&pats);
        assert_eq!((labels, k), (vec![0, 0], 1));
        // Disjoint pairs -> two components.
        let pats = parse("SELECT * WHERE { ?a <http://ex/p> ?b . ?c <http://ex/q> ?d }");
        let (labels, k) = pattern_components(&pats);
        assert_eq!((labels, k), (vec![0, 1], 2));
    }

    #[test]
    fn top_conjuncts_and_var_eq_extraction() {
        let v = |n: &str| Expression::Variable(Variable::new(n).unwrap());
        let eq = |a: &str, b: &str| Expression::Equal(Box::new(v(a)), Box::new(v(b)));
        let e = Expression::And(
            Box::new(Expression::And(Box::new(eq("a", "b")), Box::new(v("c")))),
            Box::new(eq("d", "e")),
        );
        let cs = top_conjuncts(&e);
        assert_eq!(cs.len(), 3);
        assert!(as_var_eq(cs[0]).is_some());
        assert!(as_var_eq(cs[1]).is_none());
        assert!(as_var_eq(cs[2]).is_some());
        // Self-equality and non-variable operands are not join edges.
        assert!(as_var_eq(&eq("a", "a")).is_none());
        let lit = Expression::Equal(Box::new(v("a")), Box::new(Expression::Literal(oxrdf::Literal::new_simple_literal("x"))));
        assert!(as_var_eq(&lit).is_none());
    }

    #[test]
    fn expr_vars_collects_bound_and_nested() {
        let v = |n: &str| Expression::Variable(Variable::new(n).unwrap());
        let e = Expression::And(
            Box::new(Expression::Bound(Variable::new("a").unwrap())),
            Box::new(Expression::In(Box::new(v("b")), vec![v("c")])),
        );
        let mut vs = FxHashSet::default();
        expr_vars(&e, &mut vs);
        let mut names: Vec<&str> = vs.iter().map(|x| x.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, vec!["a", "b", "c"]);
    }
}
