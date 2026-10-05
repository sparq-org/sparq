use super::*;

// ---- Modifiers ----------------------------------------------------------------

pub(super) fn project_bindings(b: Bindings, vars: &[Variable]) -> Bindings {
    let cols: Vec<Option<usize>> = vars.iter().map(|v| b.col(v)).collect();
    let rows = b
        .rows
        .iter()
        .map(|row| cols.iter().map(|c| c.map(|i| row[i]).unwrap_or(NO_ID)).collect())
        .collect();
    let sorted_by = b.sorted_by.filter(|sv| vars.contains(sv));
    Bindings { vars: vars.to_vec(), rows, sorted_by }
}

pub(super) fn distinct_bindings(b: &mut Bindings) {
    let mut seen = std::collections::HashSet::new();
    b.rows.retain(|r| seen.insert(r.clone()));
}

pub(super) fn slice_bindings(b: &mut Bindings, start: usize, length: Option<usize>) {
    if start > 0 {
        b.rows.drain(0..start.min(b.rows.len()));
    }
    if let Some(l) = length {
        b.rows.truncate(l);
    }
}

/// One precomputed ORDER BY key cell. A temporal (dateTime/date) graph term keeps only
/// its CACHED comparison value + id — no term materialised, no per-comparison lexical
/// re-parse (the q09-class fix: ORDER BY dateTime was re-parsing both lexicals on every
/// comparison of the sort). Everything else keeps the identity-preserving `Value`.
pub(super) enum SortCell {
    Temp { t: Temporal, id: Id },
    /// A numeric GRAPH term: the cached f64 for the fast compare PLUS its dictionary id, so
    /// an f64 TIE can be rechecked EXACTLY from the id's exact lexical — distinct integers
    /// beyond 2^53 / high-precision decimals that share one f64 (the numerics cache stores
    /// only f64, so the fast path alone collapses them). This makes `ORDER BY ?v` over a
    /// numeric column agree with the relational `<` (`cmp_expr`) and MIN/MAX, which already
    /// recheck. sq-rikm7
    Num { f: f64, id: Id },
    /// A named-node (IRI) GRAPH term: the IRI string precomputed once at key-build time,
    /// eliminating per-comparison `term_of` materialisation and `value_str` allocation for
    /// IRI-typed ORDER BY columns (the SP2Bench q11 case: all ?ee values are IRIs, so
    /// every comparison calls into `compare_terms` which allocates the string twice).
    /// `cmp_sort_cells` can then directly compare two `&str` slices — no allocation.
    /// sq-7d3dj.30.2
    Iri(Box<str>),
    /// sq-7d3dj.30.21 — a PLAIN `xsd:string` LITERAL GRAPH term carried as its
    /// DICTIONARY ID only, deferring the value materialisation the eager [`SortCell::Val`] arm
    /// does at construction time (a `reconstruct_ref` `Literal` alloc + a `value_str`
    /// collation-key alloc per input row). Under the `topk-lazy-strkey` feature the top-k
    /// ORDER BY key build emits this instead of `Val{..}` for a plain-string column, so an
    /// ORDER-BY on such a column over N input rows does ZERO key allocation up front (the
    /// SP2Bench q11 residual: only k of the 17663 keyed rows survive). `cmp_sort_cells`
    /// compares two `StrId` cells by their ZERO-COPY `Dict::term_parts` `value` bytes, which is
    /// BYTE-IDENTICAL to comparing the two eager `value_str` keys (a plain string orders by its
    /// `value()` lexical bytes — the sq-7d3dj.30.12 differential test proves it). Cross-kind /
    /// cross-class comparisons use the fixed `String`-kind / `Literal`-class ranks (a plain
    /// string literal is always Literal-class, String-kind), so those arms need no
    /// materialisation either.
    #[cfg(feature = "topk-lazy-strkey")]
    StrId(Id),
    /// Any other key value (a plain / typed / language-tagged literal, a blank node, a
    /// quoted triple term, a computed value, or an unbound / error). Extends the `Iri`
    /// fast-path idea to EVERY `SortCell` kind (bead sq-7d3dj.30.12): the SPARQL
    /// total-order dispatch is HOISTED to cell-CONSTRUCTION time so a heap comparison is
    /// an integer / byte-slice compare, never a per-comparison `lit_kind` +
    /// `is_numeric_dt` + `value_str`-allocation re-derivation.
    ///
    /// - `class` — the [`TermClass`] rank (cross-class compares reduce to `class.cmp`).
    /// - `kind` — the within-Literal [`LiteralKind`] rank (cross-kind compares reduce to
    ///   `kind.cmp`); a fixed filler for non-literal classes (never consulted there).
    /// - `key` — the PRECOMPUTED lexical collation key (`value_str`), present for exactly
    ///   the kinds whose within-kind order IS the lexical `value()` order — the `String`,
    ///   `Lang` and `Other` literal kinds and the `Blank` class (see `cmp_sort_cells`'s
    ///   `(Val, Val)` arm for why this reproduces `compare_terms` exactly there). `None`
    ///   for the VALUE-ordered kinds (numeric exact-tie / boolean / temporal timeline) and
    ///   for triple terms, which keep `v`'s full `compare_values` semantics.
    /// - `v` — retained so a value-ordered same-kind pair (and the triple-term recursion)
    ///   stays byte-identical to `compare_terms`.
    ///
    /// All ranks + the key are taken ONCE, from the very `CompareTerm` observations
    /// `compare_terms` would otherwise recompute on every comparison. sq-7d3dj.30.12
    Val { class: u8, kind: u8, key: Option<Box<str>>, v: Value },
}

/// Builds a `SortCell::Val`, precomputing its `(TermClass, LiteralKind)` collation ranks
/// AND — for the lexically-ordered kinds only — the `value_str` collation key, from the
/// SAME `CompareTerm` observations `compare_terms` reads. So a `Val`↔`Val` comparison is
/// an integer rank compare (cross-class / cross-kind) or a precomputed byte-slice compare
/// (same lexical kind), never a per-comparison `lit_kind` / `is_numeric_dt` /
/// `value_str`-allocation re-derivation. sq-7d3dj.30.12
#[inline]
pub(super) fn sort_cell_val(v: Value) -> SortCell {
    let class = v.term_class() as u8;
    // Only the literal class consults the kind rank; skip the (cheap but non-trivial)
    // `lit_kind` dispatch entirely for the non-literal classes.
    let kind = if class == TermClass::Literal as u8 { v.literal_kind() as u8 } else { 0 };
    // Precompute the lexical key for EXACTLY the kinds whose within-kind (and, for the
    // literal kinds, same-KIND) order is the `value_str` lexical order — so a same-kind
    // compare is a direct slice compare with NO per-comparison allocation, reproducing
    // `compare_terms`' within-kind result (proven byte-identical by the differential test):
    //   • String / Lang / Other literal kinds — `compare_terms`' `strict_cmp` arm (where it
    //     decides: same-tag / same-other-XSD) yields the SAME `value()` order as its
    //     `value_str` fallback (cross-tag / cross-other-XSD), so the whole kind orders by
    //     `value_str` (`value()`);
    //   • the Blank class — `compare_terms` orders blanks by their `value_str` label.
    // Numeric (exact-tie recheck), Boolean and the temporal kinds are VALUE-ordered, and
    // triple terms recurse — those keep `None` and defer to `compare_values`.
    let key = if class == TermClass::Blank as u8
        || (class == TermClass::Literal as u8
            && (kind == LiteralKind::String as u8
                || kind == LiteralKind::Lang as u8
                || kind == LiteralKind::Other as u8))
    {
        value_str(&v).map(String::into_boxed_str)
    } else {
        None
    };
    SortCell::Val { class, kind, key, v }
}

/// Compares two ORDER BY key cells under the lenient total order, reproducing
/// `compare_values` exactly: two temporals by `Temporal::cmp_t_total` (kind-first, then
/// the timeline order extended over the indeterminate mixed-timezone window — the same
/// definition `compare_values` reaches through `CompareTerm::strict_cmp`); a temporal
/// against any other key materialises the term lazily (rare: only mixed-type columns)
/// and defers to `compare_values` itself.
#[inline]
pub(super) fn cmp_sort_cells(graph: &Graph, local: &LocalVocab, a: &SortCell, c: &SortCell) -> Ordering {
    match (a, c) {
        (SortCell::Temp { t: ta, .. }, SortCell::Temp { t: tb, .. }) => {
            // sq-wjl8i KIND-FIRST + sq-2k5py: both now live in the
            // shared `Temporal::cmp_t_total` — a dateTime never value-compares against a
            // date (`LiteralKind::DateTime < Date`), and within one kind the timeline order
            // is TOTAL (instant, then timezone presence). The former lexical fallback for
            // the indeterminate window is gone: it mixed timeline-decided and
            // lexical-decided pairs inside one kind, which is intransitive. Ill-formed
            // temporals never enter the cache, so both cells here are well-formed.
            Temporal::cmp_t_total(*ta, *tb)
        }
        // Two numeric graph terms: f64 fast compare, with the EXACT tie recheck below.
        (SortCell::Num { f: fa, id: ia }, SortCell::Num { f: fb, id: ib }) => {
            cmp_sort_num(graph, *fa, *ia, *fb, *ib)
        }
        (SortCell::Temp { id, .. }, SortCell::Val { v, .. }) => {
            compare_values(&sort_cell_term(graph, local, *id), v).unwrap_or(Ordering::Equal)
        }
        (SortCell::Val { v, .. }, SortCell::Temp { id, .. }) => {
            compare_values(v, &sort_cell_term(graph, local, *id)).unwrap_or(Ordering::Equal)
        }
        // A numeric cell against a temporal or a general Value (a MIXED-type column, cold):
        // reconstruct the pre-change `Val(Num::Double)` representation and defer to the shared
        // `compare_values`, so the mixed-column order stays byte-identical to pre-change — only
        // the same-numeric-column (`Num`, `Num`) arm above adds the exact f64-tie recheck.
        (SortCell::Num { f, .. }, SortCell::Temp { id, .. }) => {
            compare_values(&Value::Num(Num::Double(*f)), &sort_cell_term(graph, local, *id)).unwrap_or(Ordering::Equal)
        }
        (SortCell::Temp { id, .. }, SortCell::Num { f, .. }) => {
            compare_values(&sort_cell_term(graph, local, *id), &Value::Num(Num::Double(*f))).unwrap_or(Ordering::Equal)
        }
        (SortCell::Num { f, .. }, SortCell::Val { v, .. }) => {
            compare_values(&Value::Num(Num::Double(*f)), v).unwrap_or(Ordering::Equal)
        }
        (SortCell::Val { v, .. }, SortCell::Num { f, .. }) => {
            compare_values(v, &Value::Num(Num::Double(*f))).unwrap_or(Ordering::Equal)
        }
        // Two IRI sort cells: direct string comparison — no allocation, no term_class
        // dispatch. sq-7d3dj.30.2
        (SortCell::Iri(a), SortCell::Iri(b)) => a.as_ref().cmp(b.as_ref()),
        // IRI against a Num/Temp cell (a MIXED-type column): IRIs are term-class Iri
        // (rank 2 in the SPARQL total order), literals are Literal (rank 3) — so IRIs
        // always sort before any literal, regardless of numeric or temporal subtype.
        (SortCell::Iri(_), SortCell::Num { .. }) => Ordering::Less,
        (SortCell::Num { .. }, SortCell::Iri(_)) => Ordering::Greater,
        (SortCell::Iri(_), SortCell::Temp { .. }) => Ordering::Less,
        (SortCell::Temp { .. }, SortCell::Iri(_)) => Ordering::Greater,
        // IRI against a generic Val: dispatch on the Val variant to determine rank.
        // Val can hold Unbound (rank 0), BlankNode (rank 1), NamedNode/IRI (rank 2),
        // Literal/Num/Bool (rank 3), Triple (rank 4).
        (SortCell::Iri(a), SortCell::Val { v, .. }) => match v {
            Value::Unbound | Value::Error => Ordering::Greater, // IRI after unbound
            Value::Term(Term::BlankNode(_)) => Ordering::Greater, // IRI after blank node
            Value::Term(Term::NamedNode(n)) => a.as_ref().cmp(n.as_str()), // same class
            _ => Ordering::Less, // IRI before literals and triple terms
        },
        (SortCell::Val { v, .. }, SortCell::Iri(a)) => match v {
            Value::Unbound | Value::Error => Ordering::Less,
            Value::Term(Term::BlankNode(_)) => Ordering::Less,
            Value::Term(Term::NamedNode(n)) => n.as_str().cmp(a.as_ref()),
            _ => Ordering::Greater,
        },
        // sq-7d3dj.30.21 — LAZY plain-string-literal cells. `StrId(id)` denotes a
        // plain `xsd:string` literal: TermClass::Literal (rank 3), LiteralKind::String (rank
        // 4), value = the literal's `value` slice (via `Dict::term_parts`). Every arm below
        // reproduces the order the eager `Val{class:3, kind:4, key:Some(value)}` cell would
        // give BYTE-IDENTICALLY, with no per-row `value_str` allocation. The homogeneous
        // `(StrId, StrId)` arm is the hot q11 comparator; the cross-type arms cover a mixed key
        // column (an ORDER BY expression that yields plain strings on some rows and other terms
        // on others).
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::StrId(a), SortCell::StrId(b)) => {
            // Same class (Literal) + same kind (String) → lexical value-byte order, exactly
            // the eager `(Some(la), Some(lb)) => la.cmp(lb)` `Val` arm.
            str_id_value(graph, *a).cmp(str_id_value(graph, *b))
        }
        // String literal (Literal-class rank 3) vs an IRI (rank 2): the literal sorts AFTER the
        // IRI — regardless of value.
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::StrId(_), SortCell::Iri(_)) => Ordering::Greater,
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::Iri(_), SortCell::StrId(_)) => Ordering::Less,
        // String literal (kind String rank 4) vs a numeric (kind Numeric rank 0) or temporal
        // (kind DateTime rank 2 / Date rank 3) — all Literal-class, and String's kind rank is
        // strictly greater than every one of those, so the string sorts AFTER, by kind rank.
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::StrId(_), SortCell::Num { .. }) => Ordering::Greater,
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::Num { .. }, SortCell::StrId(_)) => Ordering::Less,
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::StrId(_), SortCell::Temp { .. }) => Ordering::Greater,
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::Temp { .. }, SortCell::StrId(_)) => Ordering::Less,
        // String literal vs a generic Val — mirror the eager `Val` dispatch by the OTHER cell's
        // precomputed class/kind ranks against the fixed (Literal=3, String=4) ranks of a plain
        // string, then compare value bytes for the same-class-same-kind (String) case.
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::StrId(a), SortCell::Val { class: cb, kind: kb, key: kb_key, v: cv }) => {
            cmp_strid_val(graph, *a, *cb, *kb, kb_key.as_deref(), cv)
        }
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::Val { class: ca, kind: ka, key: ka_key, v: av }, SortCell::StrId(b)) => {
            cmp_strid_val(graph, *b, *ca, *ka, ka_key.as_deref(), av).reverse()
        }
        // Two generic key values: compare the PRECOMPUTED class rank, then (within the
        // literal class) the PRECOMPUTED kind rank — both integer compares, no per-comparison
        // `lit_kind` / `is_numeric_dt` re-derivation. Only when the ranks tie (same class,
        // and same literal kind if literal) do we fall through to `compare_values` for the
        // within-kind value order — which, given equal class+kind, reaches EXACTLY the same
        // within-kind arm `compare_terms` would (the numeric exact-tie recheck, the
        // timeline / strict compare, or the lexical fallback). So the total order is
        // byte-identical to `compare_values(av, cv)`, just with the cross-class / cross-kind
        // dispatch hoisted to construction time. sq-7d3dj.30.12
        (
            SortCell::Val { class: ca, kind: ka, key: ka_key, v: av },
            SortCell::Val { class: cb, kind: kb, key: kb_key, v: cv },
        ) => match ca.cmp(cb) {
            Ordering::Equal => {
                // Same class. Within the literal class, rank by kind first; other classes
                // (blank / IRI / triple / error-or-unbound) have no kind split.
                if *ca == TermClass::Literal as u8 && ka != kb {
                    return ka.cmp(kb);
                }
                // Same class (and same literal kind, if literal). When BOTH cells carry a
                // precomputed lexical key — the `String` / `Lang` / `Other` literal kinds
                // and the `Blank` class — the within-kind order IS the `value_str` (lexical
                // `value()`) order, so compare the precomputed slices directly: no
                // per-comparison `value_str` allocation, no re-dispatch. This is byte-
                // identical to `compare_values(av, cv)` for those kinds (the differential
                // test proves it). Anything else (numeric exact-tie, boolean, temporal,
                // triple, error/unbound) keeps `None` and defers to the shared comparator.
                match (ka_key, kb_key) {
                    (Some(la), Some(lb)) => la.cmp(lb),
                    _ => compare_values(av, cv).unwrap_or(Ordering::Equal),
                }
            }
            ord => ord,
        },
    }
}

/// Compares two numeric ORDER BY sort cells by f64, rechecking EXACTLY on an f64 tie.
/// sq-rikm7: f64 rounding is MONOTONIC, so it only ever collapses distinct
/// numeric values to Equal — so on (and only on) an f64 tie, disambiguate via the ids'
/// exact lexicals (`exact_numeric_lexical` + `cmp_decimal_str`, the SAME exact path the
/// relational `cmp_expr` uses). sq-wjl8i extends the recheck to the MIXED
/// exact/inexact tie: a float/double id has no exact lexical but its VALUE is exactly
/// the tied f64, so the exact side compares against the f64's exact decimal expansion —
/// mirroring `compare_values`' `Num::cmp_total` recheck, so the numeric fast path and
/// the general path order identically. Perf-neutral: the allocations happen only on a
/// tie, never on the fast path.
#[inline]
pub(super) fn cmp_sort_num(graph: &Graph, fa: f64, ia: Id, fb: f64, ib: Id) -> Ordering {
    match fa.partial_cmp(&fb) {
        Some(Ordering::Equal) => {
            // Same dictionary id ⇒ identical value: skip the two lexical allocations.
            // In a numeric ORDER BY column with repeated values this is the common f64
            // tie (a sort compares equal-id rows often). sq-rikm7
            if ia == ib {
                return Ordering::Equal;
            }
            match (graph.exact_numeric_lexical(ia), graph.exact_numeric_lexical(ib)) {
                (Some(la), Some(lb)) => cmp_decimal_str(&la, &lb).unwrap_or(Ordering::Equal),
                (Some(la), None) => cmp_exact_lex_f64(&la, fb),
                (None, Some(lb)) => cmp_exact_lex_f64(&lb, fa).reverse(),
                // Both float/double: the value IS the tied f64 — a true tie.
                (None, None) => Ordering::Equal,
            }
        }
        Some(o) => o,
        // NaN. Unreachable from ORDER BY sort cells today (the numerics cache uses NaN
        // as its "not numeric" sentinel, so a NaN literal never becomes a `SortCell::Num`
        // and routes through `compare_values` instead) — kept in lock-step with the
        // total order's NaN-first rule so the fast path can never diverge. sq-wjl8i
        None => match (fa.is_nan(), fb.is_nan()) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            _ => Ordering::Equal,
        },
    }
}

/// The mixed exact/inexact leg of the f64-tie recheck: an exact int/decimal LEXICAL
/// against a (finite, non-NaN) f64's exact decimal expansion — string arithmetic
/// throughout, arbitrary precision. An infinite f64 image (a numeric lexical beyond
/// f64's finite range collapsed onto ±INF) bounds every finite exact value directly.
/// sq-wjl8i
#[cold]
pub(super) fn cmp_exact_lex_f64(lex: &str, f: f64) -> Ordering {
    if f == f64::INFINITY {
        return Ordering::Less;
    }
    if f == f64::NEG_INFINITY {
        return Ordering::Greater;
    }
    match f64_exact_decimal(f) {
        Some(exp) => cmp_decimal_str(lex, &exp).unwrap_or(Ordering::Equal),
        None => Ordering::Equal, // NaN: handled by the caller's NaN rule
    }
}

// sq-2k5py: the lexical-form fallback for temporal sort cells (`cmp_sort_cells_lex`)
// is GONE — `Temporal::cmp_t_total` decides every temporal pair by value now, and a lexical
// fallback inside one temporal kind is exactly the intransitivity this bead removed.

/// Materialises a temporal sort cell's term for the (rare) mixed-type-column
/// comparison against a non-temporal key.
#[cold]
pub(super) fn sort_cell_term(graph: &Graph, local: &LocalVocab, id: Id) -> Value {
    Value::Term(term_of(graph, local, id).expect("sort key id resolves"))
}

/// sq-7d3dj.30.21 — the ZERO-COPY value `&str` of a plain-string-literal dictionary
/// id, borrowed straight from the record store (no allocation). Only called on ids `cell_of`
/// already proved are plain `xsd:string` literals (`Dict::plain_string_value` returned `Some`),
/// so the non-plain-string arm is unreachable; it returns the empty slice as a safe
/// placeholder rather than panic.
#[cfg(feature = "topk-lazy-strkey")]
#[inline]
pub(super) fn str_id_value(graph: &Graph, id: Id) -> &str {
    graph.dict.plain_string_value(id).unwrap_or("")
}

/// sq-7d3dj.30.21 — order a lazy plain-string-literal cell (`StrId(a)`: fixed
/// `TermClass::Literal` rank 3, `LiteralKind::String` rank 4, value = the literal's `value`
/// slice) against an EAGER `Val` cell carrying its own precomputed `(class, kind, key, v)`.
/// This is byte-identical to the `(Val, Val)` arm of [`cmp_sort_cells`] with the string's fixed
/// ranks: compare class ranks first; within the Literal class compare kind ranks; only when
/// BOTH are Literal-class String-kind (the string's kind) do we compare value bytes — the
/// eager String-kind `Val` always carries `key = Some(value_str)` (see `sort_cell_val`), which
/// for a plain string IS the `value()` bytes, so a direct slice compare against
/// `str_id_value(a)` reproduces the eager `(Some, Some) => la.cmp(lb)` order exactly.
#[cfg(feature = "topk-lazy-strkey")]
#[inline]
pub(super) fn cmp_strid_val(graph: &Graph, a: Id, vclass: u8, vkind: u8, vkey: Option<&str>, v: &Value) -> Ordering {
    let str_class = TermClass::Literal as u8;
    let str_kind = LiteralKind::String as u8;
    match str_class.cmp(&vclass) {
        Ordering::Equal => {
            // Both Literal-class. Rank by kind first (String vs the Val's kind).
            if str_kind != vkind {
                return str_kind.cmp(&vkind);
            }
            // Both Literal-class String-kind: value-byte order. A String-kind `Val` always
            // carries a precomputed lexical key; fall back to `value_str` only if (unexpectedly)
            // absent, so the comparison never silently mis-orders.
            match vkey {
                Some(lb) => str_id_value(graph, a).cmp(lb),
                None => match value_str(v) {
                    Some(lb) => str_id_value(graph, a).cmp(lb.as_str()),
                    None => Ordering::Equal,
                },
            }
        }
        ord => ord,
    }
}

/// Sort or bounded-select the bindings according to `exprs`.
///
/// When `row_budget` is `Some(k)` AND `k < b.rows.len()`, uses bounded selection
/// (quickselect O(n) + sort O(k log k)) instead of a full stable sort O(n log n).
/// The top-k path uses `input_idx` as a tiebreaker to reproduce the stable sort's
/// tie behaviour EXACTLY — byte-identical to `order_bindings(…, None)` + slice
/// `[0..k]` for any k. When `row_budget` is `None` or `k >= n`, uses the full
/// stable sort (unchanged). sq-7d3dj.30.2
///
/// Each ORDER expression is compiled ONCE up front (`compile_expr`) so the
/// Variable → column-index resolution is hoisted out of the per-row loop; both the
/// bounded-selection and full-sort paths share the same `key_of`/`cell_of`, so the
/// hoist and the top-k bound compose without either path re-resolving columns per
/// row. sq-7d3dj.4
pub(super) fn order_bindings(
    graph: &Graph,
    local: &LocalVocab,
    b: &mut Bindings,
    exprs: &[OrderExpression],
    row_budget: Option<usize>,
) -> Result<(), String> {
    // Pre-resolve Variable → column index once for each ORDER expression. sq-7d3dj.4.
    let compiled_order: Vec<(bool, CompiledExpr)> = exprs
        .iter()
        .map(|oe| match oe {
            OrderExpression::Asc(e) => (false, compile_expr(e, b)),
            OrderExpression::Desc(e) => (true, compile_expr(e, b)),
        })
        .collect();

    // The sort key cell for one compiled ORDER expression of one row. Numeric keys use the
    // numerics cache and temporal keys the temporals cache (no per-comparison reparse); IRI
    // terms precompute the IRI string once (SortCell::Iri) — eliminating per-comparison
    // term_of materialisation + value_str allocation for IRI ORDER BY columns; other
    // expressions fall back to identity-preserving evaluation. The plain-variable case is
    // unpacked here so the column lookup and the cache probes happen exactly once per row
    // (column index was pre-resolved above). sq-7d3dj.4 / sq-7d3dj.30.2 (Iri).
    let cell_of = |row: &Row, e: &CompiledExpr| -> Result<SortCell, String> {
        if let CompiledExpr::Var(Some(c)) = e {
            let id = row[*c];
            if id != NO_ID && !is_local(id) {
                if let Some(n) = graph.numeric_value(id) {
                    // sq-rikm7: keep the id so an f64 TIE can be rechecked
                    // exactly (integers > 2^53 / high-precision decimals sharing one f64).
                    return Ok(SortCell::Num { f: n, id });
                }
                if let Some(t) = graph.temporal_value(id) {
                    return Ok(SortCell::Temp { t, id });
                }
                // sq-7d3dj.30.21 — LAZY STRING-LITERAL key: a plain `xsd:string`
                // store-literal becomes a zero-allocation `SortCell::StrId(id)` (compared via
                // the literal's ZERO-COPY `Dict::term_parts` `value` bytes) instead of eagerly
                // reconstructing a `Literal` (`term_of`) AND allocating a `value_str` collation
                // key (`sort_cell_val`) for every input row. `plain_string_value` is a single
                // record probe that returns `Some` for EXACTLY the `LiteralKind::String` set
                // (datatype `xsd:string`, no lang tag), so the SPARQL value-order is preserved.
                // The eager `Val{..}` key remains the feature-OFF path (byte-identical order).
                // This targets the SP2Bench q11 residual: N=17663 keyed, k=60 survive.
                #[cfg(feature = "topk-lazy-strkey")]
                if graph.dict.plain_string_value(id).is_some() {
                    return Ok(SortCell::StrId(id));
                }
                // Neither numeric nor temporal: materialise the term once. For IRI terms,
                // store the IRI string in SortCell::Iri so comparisons are direct &str
                // comparisons with no per-comparison allocation. sq-7d3dj.30.2
                let term = term_of(graph, local, id).expect("bound id resolves");
                if let Term::NamedNode(n) = &term {
                    return Ok(SortCell::Iri(n.as_str().into()));
                }
                return Ok(sort_cell_val(Value::Term(term)));
            }
        }
        Ok(match eval_compiled_numeric(graph, local, row, e) {
            Some(n) => sort_cell_val(Value::Num(Num::Double(n))),
            None => sort_cell_val(eval_compiled(graph, local, b, row, e)?),
        })
    };
    // The sort key (vector of (descending, SortCell)) for one row.
    let key_of = |row: &Row| -> Result<Vec<(bool, SortCell)>, String> {
        let mut key = Vec::with_capacity(compiled_order.len());
        for (desc, ce) in &compiled_order {
            key.push((*desc, cell_of(row, ce)?));
        }
        Ok(key)
    };

    let n = b.rows.len();

    // Top-k bounded-selection path: O(n + k log k) instead of O(n log n).
    // Activated when the caller supplies a row_budget k < n. The comparator
    // adds `input_idx` as a tiebreaker so the partition and final sort reproduce
    // the stable-sort tie semantics exactly (smaller input_idx appears first).
    // sq-7d3dj.30.2
    if let Some(k) = row_budget {
        // LIMIT 0 (k == 0) is legal SPARQL and is exercised by the W3C conformance
        // suite. The result is empty regardless of order, so short-circuit here:
        // `select_nth_unstable_by(k - 1)` below would underflow `k - 1` to
        // `usize::MAX` and abort the process (SIGABRT). sq-7d3dj.30.2
        if k == 0 {
            b.rows.clear();
            b.sorted_by = None;
            return Ok(());
        }
        // Invariant for the bounded-selection path below: 0 < k < n, so
        // `k - 1 < n = keyed.len()` and `select_nth_unstable_by(k - 1)` is in range.
        if k < n {
            // Build (key, input_idx) — parallel for large sets. INDEX-CARRY: the keyed
            // vector holds only the sort key + the original `b.rows` index, NOT a cloned
            // `Row`. `key_of` borrows each row read-only (via `cell_of`, which touches
            // `graph`/`local` but never `b.rows`), so `b.rows` is still owned + intact
            // after this build; only the k surviving rows are cloned when we gather the
            // output below. This eliminates the n up-front `Row::clone`s of the previous
            // `(key, input_idx, Row)` tuple — only k clones happen (n=17663 → k=60 on the
            // SP2Bench q11 residual). Byte-identical output: the tuple index reproduces the
            // same stable-sort tie order, and the gathered row is `b.rows[i]` unchanged.
            // sq-7d3dj.30.23
            // Use Vec<_> to avoid the clippy::type_complexity lint on the explicit type.
            #[cfg(feature = "parallel")]
            let mut keyed: Vec<_> = if n >= PAR_THRESHOLD {
                use rayon::prelude::*;
                // sq-6aefu: mirror FILTER/BIND worker_install pattern — key_of -> eval_compiled
                // can re-enter EXISTS / custom extension functions / spatial expressions on rayon
                // workers; without the snapshot+reinstall the workers see NO view (named-graph
                // leak) and NO function registry (spurious error).
                let fns = functions::snapshot();
                let vw = view::snapshot();
                let spx = spatial::snapshot();
                // (sq-lsp7k.2.2) Keep LOCAL SERVICE handlers visible on the worker too:
                // a worker that missed the registry would dial the IRI instead of answering it.
                #[cfg(feature = "service-local")]
                let lsv = local_services::snapshot();
                #[cfg(not(target_arch = "wasm32"))]
                let qn = query_now::snapshot(); // sq-98w7z.1: keep NOW() pinned on workers
                b.rows
                    .par_iter()
                    .enumerate()
                    .map(|(i, row)| {
                        let _fns = functions::worker_install(&fns);
                        let _vw = view::worker_install(&vw);
                        let _spx = spatial::worker_install(&spx);
                        #[cfg(feature = "service-local")]
                        let _lsv = local_services::worker_install(&lsv);
                        #[cfg(not(target_arch = "wasm32"))]
                        let _qn = query_now::worker_install(qn);
                        Ok((key_of(row)?, i))
                    })
                    .collect::<Result<Vec<_>, String>>()?
            } else {
                b.rows
                    .iter()
                    .enumerate()
                    .map(|(i, row)| Ok((key_of(row)?, i)))
                    .collect::<Result<Vec<_>, String>>()?
            };
            #[cfg(not(feature = "parallel"))]
            let mut keyed: Vec<_> = b
                .rows
                .iter()
                .enumerate()
                .map(|(i, row)| Ok((key_of(row)?, i)))
                .collect::<Result<_, String>>()?;

            // Strict total-order comparator: sort key first (descending as flagged),
            // then input_idx as tiebreaker (smaller index = appears earlier in the
            // stable sort, so it sorts LESS here — reproduces the stable sort exactly).
            let cmp_total = |a: &(Vec<(bool, SortCell)>, usize), c: &(Vec<(bool, SortCell)>, usize)| {
                for ((desc, av), (_, cv)) in a.0.iter().zip(c.0.iter()) {
                    let ord = cmp_sort_cells(graph, local, av, cv);
                    let ord = if *desc { ord.reverse() } else { ord };
                    if ord != Ordering::Equal {
                        return ord;
                    }
                }
                // Tiebreak: smaller input_idx wins (matches stable sort input order).
                a.1.cmp(&c.1)
            };

            // Partition: after select_nth_unstable_by(k-1), keyed[..k] holds the k
            // smallest elements by cmp_total (not yet sorted within that prefix).
            // select_nth_unstable_by requires k > 0 and k-1 < len, both guaranteed here.
            debug_assert!(k > 0 && k <= keyed.len());
            keyed.select_nth_unstable_by(k - 1, |a, c| cmp_total(a, c));

            // Sort the selected prefix into the correct final order.
            keyed[..k].sort_by(|a, c| cmp_total(a, c));

            // Gather the k surviving rows by index — the ONLY `Row::clone`s in this path.
            // `keyed[..k]` is now in final sorted order, so the gathered rows land in that
            // exact order. sq-7d3dj.30.23
            b.rows = keyed[..k].iter().map(|(_, i)| b.rows[*i].clone()).collect();
            b.sorted_by = None;
            return Ok(());
        }
        // k >= n: top-k budget covers all rows, fall through to full stable sort.
    }

    // Full stable sort path (unchanged). Precompute the keys (independent, read-only)
    // — in parallel for large result sets.
    #[cfg(feature = "parallel")]
    let mut keyed: Vec<(Vec<(bool, SortCell)>, Row)> = if n >= PAR_THRESHOLD {
        use rayon::prelude::*;
        // sq-6aefu: mirror FILTER/BIND worker_install pattern — same rationale as the
        // top-k path above: key_of -> eval_compiled can re-enter EXISTS / custom
        // extension functions / spatial on rayon workers.
        let fns = functions::snapshot();
        let vw = view::snapshot();
        let spx = spatial::snapshot();
        // (sq-lsp7k.2.2) Keep LOCAL SERVICE handlers visible on the worker too:
        // a worker that missed the registry would dial the IRI instead of answering it.
        #[cfg(feature = "service-local")]
        let lsv = local_services::snapshot();
        #[cfg(not(target_arch = "wasm32"))]
        let qn = query_now::snapshot(); // sq-98w7z.1: keep NOW() pinned on workers
        b.rows
            .par_iter()
            .map(|row| {
                let _fns = functions::worker_install(&fns);
                let _vw = view::worker_install(&vw);
                let _spx = spatial::worker_install(&spx);
                #[cfg(feature = "service-local")]
                let _lsv = local_services::worker_install(&lsv);
                #[cfg(not(target_arch = "wasm32"))]
                let _qn = query_now::worker_install(qn);
                Ok((key_of(row)?, row.clone()))
            })
            .collect::<Result<_, String>>()?
    } else {
        b.rows.iter().map(|row| Ok((key_of(row)?, row.clone()))).collect::<Result<_, String>>()?
    };
    #[cfg(not(feature = "parallel"))]
    let mut keyed: Vec<(Vec<(bool, SortCell)>, Row)> =
        b.rows.iter().map(|row| Ok((key_of(row)?, row.clone()))).collect::<Result<_, String>>()?;

    let cmp = |a: &(Vec<(bool, SortCell)>, Row), c: &(Vec<(bool, SortCell)>, Row)| {
        for ((desc, av), (_, cv)) in a.0.iter().zip(c.0.iter()) {
            let ord = cmp_sort_cells(graph, local, av, cv);
            let ord = if *desc { ord.reverse() } else { ord };
            if ord != Ordering::Equal {
                return ord;
            }
        }
        Ordering::Equal
    };
    // ORDER BY tie handling: `cmp` returns `Ordering::Equal` only for rows that
    // tie on *every* ORDER BY key (no secondary tie-breaker), and SPARQL leaves the relative
    // order of ORDER BY-tied solutions unspecified, so any tie order is conformant. We still
    // keep it *deterministic at a fixed thread count*: both branches use a STABLE sort
    // (`rayon::par_sort_by` is a stable parallel merge sort, same guarantee as `slice::sort_by`),
    // so ties retain their `b.rows` (scan) input order rather than being shuffled by the
    // parallel partitioning. The only residual cross-thread-count variation is that `b.rows`
    // itself is in dict-id (scan) order, which is thread-count-dependent — that is the
    // umbrella dict-id-order property (research/dict-id-order-determinism-audit.md), not a
    // tie-break defect in this sort. So no total-order tie-breaker is added: it would cost a
    // term materialisation per tied row for an order the spec does not constrain.
    #[cfg(feature = "parallel")]
    if keyed.len() >= PAR_THRESHOLD {
        use rayon::prelude::*;
        keyed.par_sort_by(cmp);
    } else {
        keyed.sort_by(cmp);
    }
    #[cfg(not(feature = "parallel"))]
    keyed.sort_by(cmp);

    b.rows = keyed.into_iter().map(|(_, r)| r).collect();
    b.sorted_by = None;
    Ok(())
}
