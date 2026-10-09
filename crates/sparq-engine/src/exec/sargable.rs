use super::*;

// ---- Sargable numeric filters (pushed into the scan) -------------------------

/// A numeric comparison `value OP threshold` that can be pushed down into a
/// pattern scan (FILTER predicate evaluated inline, in the column's sorted order,
/// so the numeric access is sequential rather than a random dictionary gather —
/// the layout fix the hardware research measured as an 8–15× win).
#[derive(Clone, Copy)]
pub(crate) enum NumCmp {
    Gt(f64),
    Ge(f64),
    Lt(f64),
    Le(f64),
    Eq(f64),
}

impl NumCmp {
    /// Human-readable comparison for EXPLAIN output, e.g. `> 28`.
    pub(crate) fn render(&self) -> String {
        match *self {
            NumCmp::Gt(t) => format!("> {t}"),
            NumCmp::Ge(t) => format!(">= {t}"),
            NumCmp::Lt(t) => format!("< {t}"),
            NumCmp::Le(t) => format!("<= {t}"),
            NumCmp::Eq(t) => format!("= {t}"),
        }
    }

    #[inline]
    pub(super) fn test(&self, x: f64) -> bool {
        match *self {
            NumCmp::Gt(t) => x > t,
            NumCmp::Ge(t) => x >= t,
            NumCmp::Lt(t) => x < t,
            NumCmp::Le(t) => x <= t,
            NumCmp::Eq(t) => x == t,
        }
    }
}

/// A comparison operator, for the temporal pushed-down predicate.
#[derive(Clone, Copy)]
pub(crate) enum CmpOp {
    Gt,
    Ge,
    Lt,
    Le,
    Eq,
}

impl CmpOp {
    #[inline]
    pub(super) fn eval(self, o: Ordering) -> bool {
        match self {
            CmpOp::Gt => o == Ordering::Greater,
            CmpOp::Ge => o != Ordering::Less,
            CmpOp::Lt => o == Ordering::Less,
            CmpOp::Le => o != Ordering::Greater,
            CmpOp::Eq => o == Ordering::Equal,
        }
    }

    pub(super) fn render(self) -> &'static str {
        match self {
            CmpOp::Gt => ">",
            CmpOp::Ge => ">=",
            CmpOp::Lt => "<",
            CmpOp::Le => "<=",
            CmpOp::Eq => "=",
        }
    }
}

/// A sargable FILTER predicate pushed down into a pattern scan: numeric (via the f64
/// `numerics` cache) or temporal (via exact borrowed dateTime/date lexicals vs a
/// temporal constant).
#[derive(Clone, Copy)]
pub(crate) enum ScanCmp<'a> {
    Num(NumCmp),
    /// `value OP temporal-constant`. A row passes when the comparison is DECIDABLE and
    /// satisfies the operator; an indeterminate (mixed-timezone window), cross-family
    /// (dateTime vs date) or non-temporal operand is a FILTER type error — the row is
    /// excluded, which `false` reproduces exactly. (For `=`, cross-family is "known
    /// different" rather than an error — also excluded, also `false`.)
    /// The cached approximate key of the constant (when it has one) only lets
    /// [`temporal_cmp_of_id`] skip the exact key for rows far from the constant.
    Temp(CmpOp, ExactTemporal<'a>, Option<Temporal>),
}

impl ScanCmp<'_> {
    /// Human-readable comparison for EXPLAIN output, e.g. `> 28`.
    pub(crate) fn render(&self) -> String {
        match *self {
            ScanCmp::Num(c) => c.render(),
            ScanCmp::Temp(op, t, _) => format!("{} exact temporal {:?}", op.render(), t.timeline),
        }
    }

    /// Evaluates the pushed-down predicate against one scanned column id, through the
    /// numeric cache or exact borrowed temporal key; no term is materialised.
    #[inline]
    pub(super) fn test_id(&self, graph: &Graph, id: Id) -> bool {
        match *self {
            ScanCmp::Num(c) => graph.numeric_value(id).is_some_and(|x| c.test(x)),
            ScanCmp::Temp(op, t, approx) => temporal_cmp_of_id(graph, id, t, approx).is_some_and(|o| op.eval(o)),
        }
    }
}

/// The inclusive range of inline-integer *values* `[lo, hi]` (within `[0, INLINE_MAX]`)
/// that satisfy the comparison, or `None` if no integer can. Used to range-prune a
/// scan whose filter column holds inline integers (which sort by value). A TEMPORAL
/// predicate over an all-inline (integer) column is a type error on every row —
/// `None`, the empty range.
pub(super) fn inline_pass_values(cmp: ScanCmp) -> Option<(u32, u32)> {
    let cmp = match cmp {
        ScanCmp::Num(c) => c,
        ScanCmp::Temp(..) => return None,
    };
    let max = (dict::INLINE_BASE - 1) as i64;
    let (lo, hi): (i64, i64) = match cmp {
        NumCmp::Gt(t) => (t.floor() as i64 + 1, max),
        NumCmp::Ge(t) => (t.ceil() as i64, max),
        NumCmp::Lt(t) => (0, t.ceil() as i64 - 1),
        NumCmp::Le(t) => (0, t.floor() as i64),
        NumCmp::Eq(t) => {
            if t.fract() != 0.0 || t < 0.0 || t > max as f64 {
                return None;
            }
            let v = t as i64;
            (v, v)
        }
    };
    let (lo, hi) = (lo.max(0), hi.min(max));
    (lo <= hi).then_some((lo as u32, hi as u32))
}

/// Recognises a FILTER of the form `?v OP constant` (or the symmetric
/// `constant OP ?v`) over a numeric or temporal constant, returning the variable
/// and the comparison to push down.
///
/// (sq-lr2ii) A NUMERIC comparison is DECLINED (returns `None`) when the graph
/// holds an f64-inexact decimal ([`Graph::has_high_precision_decimal`]): the f64 `numerics`
/// cache the scan probes could then decide `=`/`<`/`>`/`<=`/`>=` wrongly for that value (e.g.
/// `"1.000000000000000001"^^xsd:decimal` collapses onto the f64 `1.0`), so such comparisons
/// fall back to the exact general evaluator instead. Temporal pushdown is unaffected, and a
/// graph with no f64-inexact decimal keeps the numeric fast path.
pub(super) fn extract_sargable<'a>(graph: &Graph, e: &'a Expression) -> Option<(Variable, ScanCmp<'a>)> {
    if budget::strict_numeric() { return None; }
    // (threshold, whether the float tier could round it) — see the decline below.
    fn lit_num(e: &Expression) -> Option<(f64, bool)> {
        match e {
            Expression::Literal(l) if is_numeric_dt(l) => {
                // sq-6b1lj: datatype-aware, verbatim-validated constant (`numeric_cache_f64`).
                // A datatype-ill-formed threshold (`"1.5"^^xsd:integer`) yields `None`, so
                // `extract_sargable` DECLINES the numeric fast path and the FILTER takes the
                // exact general comparison — which type-errors the ill-formed constant,
                // matching the reference semantics (a sargable f64 threshold would instead
                // compare against it, over-including).
                let v: f64 = numeric_cache_f64(l)?;
                // A threshold f64 can't represent precisely (> 15 significant digits —
                // large integers or high-precision decimals) makes the sargable f64 scan
                // unsafe; decline so the filter takes the exact general comparison path.
                if sig_digits(l.value()) > 15 {
                    return None;
                }
                // XPath compares an `xs:float` against a non-double operand in the FLOAT tier,
                // which the untyped f64 cache cannot reproduce (`"0.1"^^xsd:float = 0.1` is
                // true; their f64 images differ). A FLOAT constant is never pushed down (any
                // integer/decimal row would need f32 promotion); an integer/decimal constant
                // is unsafe only if it is not exactly an `f32`, and only against a float row.
                // A double constant promotes every row to double: the f64 compare is exact.
                let dt = l.datatype();
                if dt == xsd::FLOAT {
                    return None;
                }
                Some((v, dt != xsd::DOUBLE && f64::from(v as f32) != v))
            }
            _ => None,
        }
    }
    // A well-formed dateTime/dateTimeStamp/date constant: its cached-comparable value.
    // (The runtime compare through the cache is bit-identical to the per-row parse, so
    // no precision guard is needed — unlike the f64 numeric threshold above.)
    fn lit_temp(e: &Expression) -> Option<ExactTemporal<'_>> {
        match e {
            Expression::Literal(l) => temporal_of_lit(l),
            _ => None,
        }
    }
    fn var_of(e: &Expression) -> Option<Variable> {
        match e {
            Expression::Variable(v) => Some(v.clone()),
            _ => None,
        }
    }
    // (left, right, op-if-var-on-left, op-if-var-on-right)
    let (l, r, on_left, on_right): (&Expression, &Expression, CmpOp, CmpOp) = match e {
        Expression::Greater(l, r) => (l, r, CmpOp::Gt, CmpOp::Lt),
        Expression::GreaterOrEqual(l, r) => (l, r, CmpOp::Ge, CmpOp::Le),
        Expression::Less(l, r) => (l, r, CmpOp::Lt, CmpOp::Gt),
        Expression::LessOrEqual(l, r) => (l, r, CmpOp::Le, CmpOp::Ge),
        Expression::Equal(l, r) => (l, r, CmpOp::Eq, CmpOp::Eq),
        _ => return None,
    };
    let num_cmp = |op: CmpOp, t: f64| -> ScanCmp {
        ScanCmp::Num(match op {
            CmpOp::Gt => NumCmp::Gt(t),
            CmpOp::Ge => NumCmp::Ge(t),
            CmpOp::Lt => NumCmp::Lt(t),
            CmpOp::Le => NumCmp::Le(t),
            CmpOp::Eq => NumCmp::Eq(t),
        })
    };
    for (var, konst, op) in [(l, r, on_left), (r, l, on_right)] {
        let Some(v) = var_of(var) else { continue };
        if let Some((c, float_tier_rounds)) = lit_num(konst) {
            // sq-lr2ii: decline the f64 numeric fast path when the graph holds an f64-inexact
            // decimal — the scan's per-row f64 compare could be wrong for it. A numeric
            // constant is never also a temporal one, so declining here yields `None` for this
            // orientation; the exact general evaluator handles the residual FILTER correctly.
            // Likewise when a float row would compare against an `f32`-rounded constant.
            if !graph.has_high_precision_decimal() && !(float_tier_rounds && graph.has_float_literal()) {
                return Some((v, num_cmp(op, c)));
            }
            continue;
        }
        if let Some(t) = lit_temp(konst) {
            let approx = match konst {
                Expression::Literal(l) => Temporal::of_lit(l.value(), l.datatype().as_str()),
                _ => None,
            };
            return Some((v, ScanCmp::Temp(op, t, approx)));
        }
    }
    None
}

/// The canonical position (0=subject, 1=predicate, 2=object) of a variable in a
/// triple pattern, if it occurs there.
pub(super) fn pattern_var_pos(tp: &TriplePattern, var: &Variable) -> Option<usize> {
    if matches!(&tp.subject, TermPattern::Variable(v) if v == var) {
        return Some(0);
    }
    if matches!(&tp.predicate, NamedNodePattern::Variable(v) if v == var) {
        return Some(1);
    }
    if matches!(&tp.object, TermPattern::Variable(v) if v == var) {
        return Some(2);
    }
    None
}

/// Splits FILTERs into per-pattern sargable numeric predicates (pushed into the
/// scan of the first pattern that binds the variable) and the residual filters
/// (applied normally afterwards).
pub(crate) fn split_sargable<'a>(graph: &Graph, patterns: &[TriplePattern], filters: &'a [Expression]) -> (Vec<Option<(usize, ScanCmp<'a>)>>, Vec<Expression>) {
    // zk-trace: a sargable FILTER pushed into the scan would make the scan
    // record only the POST-filter rows (the rows that PASSED), losing the
    // FILTER obligation and under-capturing the input set — the proof must
    // witness the operand of every filtered row and the rows it excluded.
    // Keep every filter residual while recording, so it flows through
    // apply_filter (one FilterObligation) over the FULL unfiltered scan set.
    // Result-equivalent; only the plan changes (zk module docs).
    #[cfg(feature = "zk")]
    if crate::zk::enabled() {
        return (vec![None; patterns.len()], filters.to_vec());
    }
    let mut pat_filters: Vec<Option<(usize, ScanCmp)>> = vec![None; patterns.len()];
    let mut residual = Vec::new();
    for f in filters {
        if let Some((var, cmp)) = extract_sargable(graph, f) {
            if let Some((i, pos)) = patterns
                .iter()
                .enumerate()
                .find_map(|(i, tp)| pattern_var_pos(tp, &var).filter(|_| pat_filters[i].is_none()).map(|pos| (i, pos)))
            {
                pat_filters[i] = Some((pos, cmp));
                continue;
            }
        }
        residual.push(f.clone());
    }
    (pat_filters, residual)
}
