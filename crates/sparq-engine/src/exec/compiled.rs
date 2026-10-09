use super::*;

// ── Expression compilation (sq-7d3dj.4) ──────────────────────────────────────────────────
//
// Walk each `Expression` tree ONCE per operator (before the row loop), resolving every
// `Variable`/`Bound` node to its column index in the operator's `Bindings`.  The per-row
// eval then uses `row[c]` directly, eliminating the per-row `b.col(v)` linear scan.
// This is also the substrate the flat compiled-expression program (sq-7d3dj.11) builds on.
// sq-7d3dj.4

/// Expression with all `Variable` / `Bound` nodes pre-resolved to column indices in the
/// operator's `Bindings`. Built once before the row loop by [`compile_expr`]; evaluated
/// per-row by [`eval_compiled`] without any per-row `Bindings` column-map scan.
#[derive(Clone, Debug)]
pub(super) enum CompiledExpr {
    /// Pre-resolved column: `Some(c)` → `row[c]`; `None` → variable not in scope (always unbound).
    Var(Option<usize>),
    /// An outer EXISTS term, including blank nodes and computed literals.
    Captured(Term),
    CapturedBound,
    /// `BOUND(?v)`: `Some(c)` → `row[c] != NO_ID`; `None` → always `false`.
    BoundCol(Option<usize>),
    NamedNode(oxrdf::NamedNode),
    /// A constant; the flag caches whether it is a valid exact-numeric lexical.
    Literal(Literal, bool),
    And(Box<Self>, Box<Self>),
    Or(Box<Self>, Box<Self>),
    Not(Box<Self>),
    Equal(Box<Self>, Box<Self>),
    /// (sq-7d3dj.30.11) `=` where BOTH operands are statically proven non-literal (a
    /// column bound only in subject/predicate positions, or a constant IRI). Evaluated by RAW-ID
    /// (in)equality — skips the per-row term materialisation the general `Equal` arm performs.
    /// Result-identical to `Equal` for these operands (canonicalising dict: IRI/bnode `=` is id
    /// equality; unbound operand is a type error). Only produced under `id-filter-fastpath`.
    #[cfg(feature = "id-filter-fastpath")]
    IdEqNonLit(IdOperand, IdOperand),
    SameTerm(Box<Self>, Box<Self>),
    Greater(Box<Self>, Box<Self>),
    GreaterOrEqual(Box<Self>, Box<Self>),
    Less(Box<Self>, Box<Self>),
    LessOrEqual(Box<Self>, Box<Self>),
    Add(Box<Self>, Box<Self>),
    Subtract(Box<Self>, Box<Self>),
    Multiply(Box<Self>, Box<Self>),
    Divide(Box<Self>, Box<Self>),
    UnaryPlus(Box<Self>),
    UnaryMinus(Box<Self>),
    If(Box<Self>, Box<Self>, Box<Self>),
    Coalesce(Vec<Self>),
    In(Box<Self>, Vec<Self>),
    FunctionCall(spargebra::algebra::Function, Vec<Self>),
    /// `EXISTS` is kept uncompiled: it re-evaluates the inner graph pattern per row.
    Exists(Box<GraphPattern>),
}

/// (sq-7d3dj.30.11) A `=`/`!=` operand resolved to a single RAW ID at eval time:
/// a bound column, or a constant IRI (resolved once against the graph dictionary). Both forms
/// are statically non-literal (columns proven so by `nonliteral_vars`; a NamedNode is an IRI).
#[cfg(feature = "id-filter-fastpath")]
#[derive(Clone, Debug)]
pub(super) enum IdOperand {
    /// `row[c]` — a column proven non-literal by the static analysis.
    Col(usize),
    /// A constant IRI. Its raw id (or `NO_ID` if the IRI is absent from the dictionary, in which
    /// case it can never equal any bound id) is resolved once at rewrite time.
    ConstIri(oxrdf::NamedNode),
}

/// Rewrites eligible `Equal(a, c)` nodes of a compiled FILTER expression into the id-level
/// [`CompiledExpr::IdEqNonLit`] fast path, in place. A node is eligible iff BOTH operands are
/// statically non-literal — a `Var(Some(c))` whose column `c` is in `nonlit_cols`, or a constant
/// `NamedNode` (always an IRI). `!=` is `Not(Equal(..))`, so rewriting the inner `Equal` covers it.
/// Everything else (numeric / temporal / possibly-literal operands, unresolved columns) is left
/// untouched and takes the existing exact path. (sq-7d3dj.30.11).
#[cfg(feature = "id-filter-fastpath")]
pub(super) fn idfast_rewrite(e: &mut CompiledExpr, nonlit_cols: &FxHashSet<usize>) {
    use CompiledExpr as C;
    // A non-literal operand descriptor, or `None` if the operand could be a literal / is not a
    // single-id term (arithmetic, function call, …).
    fn nonlit_operand(e: &CompiledExpr, nonlit_cols: &FxHashSet<usize>) -> Option<IdOperand> {
        match e {
            C::Var(Some(c)) if nonlit_cols.contains(c) => Some(IdOperand::Col(*c)),
            C::NamedNode(n) => Some(IdOperand::ConstIri(n.clone())),
            _ => None,
        }
    }
    match e {
        C::Equal(a, c) => {
            // Recurse first so nested Equals inside operands are handled uniformly (operands of an
            // Equal are never themselves the (in)equality we fast-path, but keep the walk total).
            idfast_rewrite(a, nonlit_cols);
            idfast_rewrite(c, nonlit_cols);
            if let (Some(l), Some(r)) =
                (nonlit_operand(a, nonlit_cols), nonlit_operand(c, nonlit_cols))
            {
                *e = C::IdEqNonLit(l, r);
            }
        }
        C::And(a, c)
        | C::Or(a, c)
        | C::Greater(a, c)
        | C::GreaterOrEqual(a, c)
        | C::Less(a, c)
        | C::LessOrEqual(a, c)
        | C::Add(a, c)
        | C::Subtract(a, c)
        | C::Multiply(a, c)
        | C::Divide(a, c)
        | C::SameTerm(a, c) => {
            idfast_rewrite(a, nonlit_cols);
            idfast_rewrite(c, nonlit_cols);
        }
        C::Not(a) | C::UnaryPlus(a) | C::UnaryMinus(a) => idfast_rewrite(a, nonlit_cols),
        C::If(a, b, c) => {
            idfast_rewrite(a, nonlit_cols);
            idfast_rewrite(b, nonlit_cols);
            idfast_rewrite(c, nonlit_cols);
        }
        C::Coalesce(es) | C::In(_, es) | C::FunctionCall(_, es) => {
            // `In(a, list)` keeps `a` unchanged (its top-level shape is not an Equal), but its list
            // items are ordinary expressions; only the list items recurse here (a is not rewritten
            // because `IN` is not an `=`; leaving it exact is correct and simplest).
            for x in es {
                idfast_rewrite(x, nonlit_cols);
            }
        }
        C::IdEqNonLit(..)
        | C::Var(_)
        | C::Captured(_)
        | C::CapturedBound
        | C::BoundCol(_)
        | C::NamedNode(_)
        | C::Literal(..)
        | C::Exists(_) => {}
    }
}

/// A compiled constant with its exact-numeric validity checked once, not per row.
pub(super) fn compiled_literal(l: &Literal) -> CompiledExpr {
    CompiledExpr::Literal(l.clone(), exact_lexical_of_literal(l).is_some())
}

/// Walk `e` once, resolving all `Variable`/`Bound` nodes to column indices. sq-7d3dj.4.
pub(super) fn compile_expr(e: &Expression, b: &Bindings, local: &LocalVocab) -> CompiledExpr {
    use Expression::*;
    match e {
        Variable(v) => match local.correlation.get(v) {
            Some(Term::NamedNode(n)) => CompiledExpr::NamedNode(n.clone()),
            Some(Term::Literal(l)) => compiled_literal(l),
            Some(term) => CompiledExpr::Captured(term.clone()),
            None => CompiledExpr::Var(b.col(v)),
        },
        Bound(v) if local.correlation.contains_key(v) => CompiledExpr::CapturedBound,
        Bound(v) => CompiledExpr::BoundCol(b.col(v)),
        NamedNode(n) => CompiledExpr::NamedNode(n.clone()),
        Literal(l) => compiled_literal(l),
        And(a, d) => CompiledExpr::And(Box::new(compile_expr(a, b, local)), Box::new(compile_expr(d, b, local))),
        Or(a, d) => CompiledExpr::Or(Box::new(compile_expr(a, b, local)), Box::new(compile_expr(d, b, local))),
        Not(a) => CompiledExpr::Not(Box::new(compile_expr(a, b, local))),
        Equal(a, d) => CompiledExpr::Equal(Box::new(compile_expr(a, b, local)), Box::new(compile_expr(d, b, local))),
        SameTerm(a, d) => CompiledExpr::SameTerm(Box::new(compile_expr(a, b, local)), Box::new(compile_expr(d, b, local))),
        Greater(a, d) => CompiledExpr::Greater(Box::new(compile_expr(a, b, local)), Box::new(compile_expr(d, b, local))),
        GreaterOrEqual(a, d) => {
            CompiledExpr::GreaterOrEqual(Box::new(compile_expr(a, b, local)), Box::new(compile_expr(d, b, local)))
        }
        Less(a, d) => CompiledExpr::Less(Box::new(compile_expr(a, b, local)), Box::new(compile_expr(d, b, local))),
        LessOrEqual(a, d) => {
            CompiledExpr::LessOrEqual(Box::new(compile_expr(a, b, local)), Box::new(compile_expr(d, b, local)))
        }
        Add(a, d) => CompiledExpr::Add(Box::new(compile_expr(a, b, local)), Box::new(compile_expr(d, b, local))),
        Subtract(a, d) => CompiledExpr::Subtract(Box::new(compile_expr(a, b, local)), Box::new(compile_expr(d, b, local))),
        Multiply(a, d) => CompiledExpr::Multiply(Box::new(compile_expr(a, b, local)), Box::new(compile_expr(d, b, local))),
        Divide(a, d) => CompiledExpr::Divide(Box::new(compile_expr(a, b, local)), Box::new(compile_expr(d, b, local))),
        UnaryPlus(a) => CompiledExpr::UnaryPlus(Box::new(compile_expr(a, b, local))),
        UnaryMinus(a) => CompiledExpr::UnaryMinus(Box::new(compile_expr(a, b, local))),
        If(cond, t, f) => CompiledExpr::If(
            Box::new(compile_expr(cond, b, local)),
            Box::new(compile_expr(t, b, local)),
            Box::new(compile_expr(f, b, local)),
        ),
        Coalesce(es) => CompiledExpr::Coalesce(es.iter().map(|ce| compile_expr(ce, b, local)).collect()),
        In(a, list) => {
            CompiledExpr::In(Box::new(compile_expr(a, b, local)), list.iter().map(|ce| compile_expr(ce, b, local)).collect())
        }
        FunctionCall(f, args) => {
            CompiledExpr::FunctionCall(f.clone(), args.iter().map(|ce| compile_expr(ce, b, local)).collect())
        }
        Exists(inner) => CompiledExpr::Exists(inner.clone()),
    }
}

/// Whether a compiled expression contains an arithmetic sub-expression (`+ - * /`). Mirrors
/// [`expr_has_arith`]. sq-7d3dj.4.
pub(super) fn compiled_expr_has_arith(e: &CompiledExpr) -> bool {
    use CompiledExpr::*;
    match e {
        Add(..) | Subtract(..) | Multiply(..) | Divide(..) => true,
        UnaryPlus(a) | UnaryMinus(a) => compiled_expr_has_arith(a),
        _ => false,
    }
}

// sq-ev41x: `Num` / `Dec` / `ArithOp` / `RoundMode` and the XSD lexical helpers
// (`apply_f64`, `parse_xsd_f64`, `parse_xsd_f32`, `fmt_xsd_double`) MOVED to
// `sparq_substrate::numeric` (imported at the top of this file). The only `Num` operation
// that produced an engine-private `Value` — `canonical_term` — stays here as a free helper,
// since the substrate must not know about the engine's `Value`/`Term`. Everything else is
// the substrate's `Num` verbatim, so behaviour is unchanged.

/// The numeric value as a TERM in strict canonical form (see [`Num::canonical_lexical`]).
/// Engine-side because it constructs the engine's `Value`; the value/lexical logic lives in
/// the shared substrate.
pub(super) fn num_canonical_term(n: Num) -> Value {
    Value::Term(Term::Literal(Literal::new_typed_literal(n.canonical_lexical(), n.datatype())))
}

pub(super) fn effective_boolean(v: &Value, semantics: crate::EbvSemantics) -> bool {
    ebv(v, semantics) == Some(true)
}

/// SPARQL effective boolean value, three-valued: `None` is a TYPE ERROR (unbound,
/// non-literal terms and unknown datatypes). Invalid numeric/boolean lexicals
/// instead have false EBV per SPARQL 1.1 §17.2.2, or error under the pinned
/// 1.2 draft, independently of arithmetic capacity errors.
pub(super) fn ebv(v: &Value, semantics: crate::EbvSemantics) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::Num(n) => Some(!n.is_zero() && !n.is_nan()),
        Value::Unbound | Value::Error => None,
        Value::Term(Term::Literal(l)) => {
            if l.language().is_some() {
                // SPARQL 1.1 §17.2.2 gives every plain literal, language-tagged ones
                // included, a length-based EBV. The 1.2 draft makes rdf:langString /
                // rdf:dirLangString a type error (`expression/not-not` pins this down);
                // directional strings postdate the Recommendation, so they error in both.
                return (semantics == crate::EbvSemantics::Rec2013 && l.direction().is_none())
                    .then(|| !l.value().is_empty());
            }
            let dt = l.datatype().as_str();
            if dt == xsd::BOOLEAN.as_str() {
                // Raw RDF booleans have exactly four lexical forms.
                // Constructor whitespace normalization applies only to string inputs.
                as_bool_val(v).or_else(|| (semantics == crate::EbvSemantics::Rec2013).then_some(false))
            } else if is_numeric_dt(l) {
                // EBV needs zero/NaN classification, not finite arithmetic.
                // Validate datatype facets before inspecting exact decimal digits;
                // converting them to f64 could underflow a nonzero value to false.
                if !sparq_core::numeric_literal_valid(l.value(), dt) {
                    (semantics == crate::EbvSemantics::Rec2013).then_some(false)
                } else if sparq_core::is_integer_datatype(dt) || dt == xsd::DECIMAL.as_str() {
                    Some(l.value().bytes().any(|b| matches!(b, b'1'..=b'9')))
                } else {
                    // Float/double zero is measured in that datatype's value space.
                    Num::of_literal(l).map(|n| !n.is_zero() && !n.is_nan())
                }
            } else if dt == xsd::STRING.as_str() {
                Some(!l.value().is_empty())
            } else {
                None
            }
        }
        Value::Term(_) => None,
    }
}

/// General expression operands share the constructor capacity boundary.
pub(super) fn checked_term_value(term: Term) -> Result<Value, String> {
    if let Term::Literal(literal) = &term {
        budget::check_temporal(literal.value(), literal.datatype().as_str())?;
    }
    Ok(Value::Term(term))
}

pub(super) fn eval_expr(graph: &Graph, local: &LocalVocab, b: &Bindings, row: &[Id], e: &Expression) -> Result<Value, String> {
    use Expression::*;
    match e {
        Variable(v) if local.correlation.contains_key(v) => checked_term_value(local.correlation[v].clone()),
        Variable(v) => match b.col(v) {
            // Always return the original term so term identity is preserved
            // (sameTerm, BIND passthrough, STR, etc.). The numeric fast path that
            // skips this materialisation lives in `eval_numeric`, used only by the
            // arithmetic / comparison operators where only the value matters.
            Some(c) if row[c] != NO_ID => checked_term_value(term_of(graph, local, row[c]).unwrap()),
            _ => Ok(Value::Unbound),
        },
        NamedNode(n) => Ok(Value::Term(Term::NamedNode(n.clone()))),
        Literal(l) => {
            budget::check_temporal(l.value(), l.datatype().as_str())?;
            Ok(Value::Term(Term::Literal(l.clone())))
        },
        And(a, c) => {
            // SPARQL 3-valued logic, short-circuiting: false dominates, so once the
            // left is false we return false WITHOUT evaluating the right (which may be
            // an error or an unsupported expression that would otherwise abort).
            let x = ebv3(&eval_expr(graph, local, b, row, a)?, local.ebv_semantics);
            if x == Some(false) {
                return Ok(Value::Bool(false));
            }
            let y = ebv3(&eval_expr(graph, local, b, row, c)?, local.ebv_semantics);
            Ok(and3(x, y))
        }
        Or(a, c) => {
            // SPARQL 3-valued logic, short-circuiting: true dominates.
            let x = ebv3(&eval_expr(graph, local, b, row, a)?, local.ebv_semantics);
            if x == Some(true) {
                return Ok(Value::Bool(true));
            }
            let y = ebv3(&eval_expr(graph, local, b, row, c)?, local.ebv_semantics);
            Ok(or3(x, y))
        }
        Not(a) => Ok(match ebv3(&eval_expr(graph, local, b, row, a)?, local.ebv_semantics) {
            Some(v) => Value::Bool(!v),
            None => Value::Error, // !error = error
        }),
        Equal(a, c) => equal_expr(graph, local, b, row, a, c),
        SameTerm(a, c) => {
            let (x, y) = (eval_expr(graph, local, b, row, a)?, eval_expr(graph, local, b, row, c)?);
            Ok(same_term_value(&x, &y))
        }
        Greater(a, c) => cmp_expr(graph, local, b, row, a, c, |o| o == Ordering::Greater),
        GreaterOrEqual(a, c) => cmp_expr(graph, local, b, row, a, c, |o| o != Ordering::Less),
        Less(a, c) => cmp_expr(graph, local, b, row, a, c, |o| o == Ordering::Less),
        LessOrEqual(a, c) => cmp_expr(graph, local, b, row, a, c, |o| o != Ordering::Greater),
        Add(a, c) => arith(graph, local, b, row, a, c, ArithOp::Add),
        Subtract(a, c) => arith(graph, local, b, row, a, c, ArithOp::Sub),
        Multiply(a, c) => arith(graph, local, b, row, a, c, ArithOp::Mul),
        Divide(a, c) => arith(graph, local, b, row, a, c, ArithOp::Div),
        UnaryPlus(a) => Ok(unary_plus(eval_expr(graph, local, b, row, a)?)),
        UnaryMinus(a) => {
            // Typed negation: the result keeps the argument's (promoted) numeric
            // datatype; a non-numeric operand is a type error.
            let v = eval_expr(graph, local, b, row, a)?;
            Ok(as_numeric(&v).and_then(|n| numeric_capacity::unary(n, Num::neg)).map(Value::Num).unwrap_or(Value::Error))
        }
        Bound(v) => Ok(Value::Bool(local.correlation.contains_key(v)
            || b.col(v).map(|c| row[c] != NO_ID).unwrap_or(false))),
        If(cond, t, f) => {
            // A type error in the condition propagates (it does NOT silently select
            // the else branch).
            match ebv3(&eval_expr(graph, local, b, row, cond)?, local.ebv_semantics) {
                Some(true) => eval_expr(graph, local, b, row, t),
                Some(false) => eval_expr(graph, local, b, row, f),
                None => Ok(Value::Error),
            }
        }
        Coalesce(es) => {
            // Returns the first argument that evaluates without error (unbound and
            // type errors are both skipped).
            for e in es {
                let v = eval_expr(graph, local, b, row, e)?;
                if !matches!(v, Value::Unbound | Value::Error) {
                    return Ok(v);
                }
            }
            Ok(Value::Unbound)
        }
        In(a, list) => {
            // `?x IN (..)` is the disjunction of `?x = e` under SPARQL `=` semantics:
            // true on the first match; otherwise a type error if ANY comparison
            // errored (e.g. an unbound operand), else false. Preserving the error
            // matters outside a plain FILTER (BIND, COALESCE).
            let x = eval_expr(graph, local, b, row, a)?;
            let mut errored = false;
            for c in list {
                let y = eval_expr(graph, local, b, row, c)?;
                match values_equal(&x, &y) {
                    Some(true) => return Ok(Value::Bool(true)),
                    Some(false) => {}
                    None => errored = true,
                }
            }
            Ok(if errored { Value::Error } else { Value::Bool(false) })
        }
        FunctionCall(f, args) => eval_function(graph, local, b, row, f, args),
        Exists(inner) => Ok(Value::Bool(eval_exists(graph, local, b, row, inner)?)),
    }
}

/// Correlated `EXISTS { inner }` for one outer solution row. The bounded
/// BGP/Join/UNION/FILTER/MINUS branch constrains captured IRI/literal values and
/// removes their columns before MINUS observes child domains, per SPARQL 1.1.
/// Other shapes retain the native practical compatibility evaluation, including
/// the unresolved blank-node and variable-only-position substitution cases.
/// These fallbacks are not a claim of complete published-2013 correlation support.
///
/// The inner pattern is evaluated against `graph`, which inside `GRAPH <g> { … }`
/// is the active named graph — so an EXISTS nested in a GRAPH pattern sees the same
/// dataset as its surrounding group, per the spec.
///
/// Note: the inner evaluation is re-run per outer row (the expression evaluator is
/// read-only, so there is no per-FILTER place to memoise the inner bindings). Fine
/// for correctness and small/mid results; a shared cache is a follow-up optimisation.
///
/// sq-rd2 early-exit: when the inner pattern shares NO in-scope variable
/// with a BOUND outer cell, the row's compatibility test is vacuous — EXISTS is true
/// iff the inner has ANY solution. That uncorrelated case routes through the same
/// first-solution-stop path as ASK (`Slice { LIMIT 1 }`, which the single-pattern
/// scan / count pushdown answers without materialising the whole relation) instead
/// of building every inner solution just to call `.any()` on it.
pub(super) fn eval_exists(graph: &Graph, local: &LocalVocab, b: &Bindings, row: &[Id], inner: &GraphPattern) -> Result<bool, String> {
    // (sq-7d3dj.30.11, PR #1785 review) Defence-in-depth against the id-fast column set
    // leaking into EXISTS re-entry: install an EMPTY set for the whole inner evaluation, so a
    // nested FILTER computes its OWN set against its OWN `Bindings` (via the wrapped dispatch) and
    // never inherits the outer pattern's column indices — which index a DIFFERENT layout. (The
    // consuming `apply_filter_scalar` already drains the set before the row loop, so by the time
    // this per-row re-entry runs the thread-local is empty anyway; this guard makes the invariant
    // hold regardless of future changes to WHEN the drain happens.)
    #[cfg(feature = "id-filter-fastpath")]
    return with_idfast_nonlit_cols(FxHashSet::default(), || {
        eval_exists_inner(graph, local, b, row, inner)
    });
    #[cfg(not(feature = "id-filter-fastpath"))]
    eval_exists_inner(graph, local, b, row, inner)
}

pub(super) fn eval_exists_inner(
    graph: &Graph,
    local: &LocalVocab,
    b: &Bindings,
    row: &[Id],
    inner: &GraphPattern,
) -> Result<bool, String> {
    // zk-trace: the inner pattern is re-run per outer row; tag its scans
    // `in_exists` and suppress their steps / filter obligations (EXISTS is
    // outside the stage-1 verifiable fragment — sparq-zk::verify rejects it;
    // the tag is for forensics, not proofs).
    #[cfg(feature = "zk")]
    let _zk = crate::zk::exists_scope();

    // SPARQL 1.1 §18.6 substitutes every bound outer variable,
    // including variables used only in FILTER expressions. A separate local
    // vocabulary preserves actual term identity across graph/local ID spaces.
    let mut inner_local = LocalVocab {
        ebv_semantics: local.ebv_semantics,
        correlation: local.correlation.clone(),
        dataset: local.dataset,
        ..LocalVocab::default()
    };
    for (column, variable) in b.vars.iter().enumerate() {
        if let Some(term) = term_of(graph, local, row[column]) {
            inner_local
                .correlation
                .entry(variable.clone())
                .or_insert(term);
        }
    }

    // MINUS observes solution domains before final compatibility. For
    // the admitted BGP/Join/UNION/FILTER/MINUS shape, restrict and remove each
    // bound IRI/literal column before its parent operator can inspect domains.
    if exists_domain::required(inner, &inner_local.correlation) {
        inner_local.substitute_exists_domains = true;
        let result = eval_graph_pattern(graph, &mut inner_local, inner)?;
        return Ok(!result.rows.is_empty());
    }

    // Only shared solution columns require the compatibility scan below.
    // Expression-only dependencies are already captured in inner_local and
    // therefore remain valid under the first-solution shortcut.
    let mut correlated = false;
    inner.on_in_scope_variable(|v| {
        if inner_local.correlation.contains_key(v) {
            correlated = true;
        }
    });
    if !correlated {
        // First-solution stop, reusing the ASK machinery (count pushdown / capped
        // single-pattern scan). zk-trace stays armed inside via the scope above.
        let sliced = GraphPattern::Slice {
            inner: Box::new(inner.clone()),
            start: 0,
            length: Some(1),
        };
        let b1 = eval_modified(graph, &mut inner_local, &sliced)?;
        budget::check(b1.rows.len())?;
        return Ok(!b1.rows.is_empty());
    }

    let inner_b = eval_graph_pattern(graph, &mut inner_local, inner)?;
    budget::check(inner_b.rows.len())?;
    // Include inherited captures: a nested EXISTS can refer to a
    // grandparent variable absent from its immediate parent's solution columns.
    let shared: Vec<(usize, &Term, Option<Id>)> = inner_b
        .vars
        .iter()
        .enumerate()
        .filter_map(|(ic, v)| {
            inner_local
                .correlation
                .get(v)
                .map(|term| (ic, term, graph.id_of(term)))
        })
        .collect();
    Ok(inner_b.rows.iter().any(|irow| {
        shared.iter().all(|&(ic, expected, graph_id)| {
            let actual = irow[ic];
            actual == NO_ID
                || if !is_local(actual) {
                    graph_id == Some(actual)
                } else {
                    inner_local.term(actual) == expected
                }
        })
    }))
}

/// Fast numeric evaluation that never materialises a term: a numeric variable
/// resolves to its value via the dictionary cache, a numeric literal via one
/// parse, and arithmetic recurses. Returns `None` for anything non-numeric, so
/// the caller falls back to the full (identity-preserving) `eval_expr` path.
/// Used only where the value — not the term — matters (comparison / arithmetic).
pub(super) fn eval_numeric(graph: &Graph, local: &LocalVocab, b: &Bindings, row: &[Id], e: &Expression) -> Option<f64> {
    use Expression::*;
    match e {
        Variable(v) if local.correlation.contains_key(v) => None,
        Variable(v) => {
            let c = b.col(v)?;
            let id = row[c];
            if id == NO_ID {
                None
            } else if is_local(id) {
                // Computed values resolve through the local vocab's numeric cache
                // (no term clone, no per-row lexical re-parse).
                local.numeric(id)
            } else {
                graph.numeric_value(id)
            }
        }
        // sq-6b1lj: the CONSTANT operand is datatype-aware and validated verbatim too
        // (`numeric_cache_f64`), so a datatype-ill-formed literal constant (`"1.5"^^xsd:integer`)
        // is a type error on this fast comparison path exactly as the graph-term side is.
        Literal(l) => numeric_cache_f64(l),
        Add(a, c) => Some(eval_numeric(graph, local, b, row, a)? + eval_numeric(graph, local, b, row, c)?),
        Subtract(a, c) => Some(eval_numeric(graph, local, b, row, a)? - eval_numeric(graph, local, b, row, c)?),
        Multiply(a, c) => Some(eval_numeric(graph, local, b, row, a)? * eval_numeric(graph, local, b, row, c)?),
        Divide(a, c) => Some(eval_numeric(graph, local, b, row, a)? / eval_numeric(graph, local, b, row, c)?),
        UnaryPlus(a) => eval_numeric(graph, local, b, row, a),
        UnaryMinus(a) => Some(-eval_numeric(graph, local, b, row, a)?),
        _ => None,
    }
}

/// Fast temporal (xsd:dateTime / xsd:dateTimeStamp / xsd:date) evaluation that never
/// materialises a term: a variable bound to a graph term resolves through the
/// borrowed dictionary lexical; constants and BIND-computed terms borrow their
/// original literal too. Parsing is linear in the literal length. Returns `None` for anything
/// non-temporal or ill-formed, so the caller falls back to the general path (which
/// yields the exact type-error semantics). Used only where the VALUE matters
/// (comparison operators); term identity is never needed there.
pub(super) fn eval_temporal<'a>(graph: &'a Graph, local: &'a LocalVocab, b: &Bindings, row: &[Id], e: &'a Expression) -> Option<ExactTemporal<'a>> {
    use Expression::*;
    match e {
        Variable(v) if local.correlation.contains_key(v) => None,
        Variable(v) => {
            let c = b.col(v)?;
            let id = row[c];
            if id == NO_ID {
                None
            } else if is_local(id) {
                // Computed values are rare; parse through the local vocab term.
                temporal_of_term(local.term(id))
            } else {
                temporal_of_id(graph, id)
            }
        }
        Literal(l) => temporal_of_lit(l),
        _ => None,
    }
}

/// The temporal value of a term, if it is a well-formed dateTime/date literal.
pub(super) fn temporal_of_term(t: &Term) -> Option<ExactTemporal<'_>> {
    match t {
        Term::Literal(l) => temporal_of_lit(l),
        _ => None,
    }
}

pub(super) fn temporal_of_lit(l: &Literal) -> Option<ExactTemporal<'_>> {
    if l.language().is_some() {
        return None;
    }
    budget::check_temporal(l.value(), l.datatype().as_str()).ok()?;
    ExactTemporal::of_lit(l.value(), l.datatype().as_str())
}

/// Stored temporal values obey the same evaluation domain as constructors.
pub(super) fn temporal_of_id(graph: &Graph, id: Id) -> Option<ExactTemporal<'_>> {
    if dict::is_inline(id) { return None; }
    // Capacity checks remain input-based even for malformed/out-of-cache values.
    // Native unbounded evaluation avoids dictionary/year parsing on the hot path.
    if budget::temporal_capacity_active() {
        if let dict::TermParts::Lit { value, datatype, .. } = graph.dict.term_parts(id) {
            budget::check_temporal(value, datatype).ok()?;
        }
    }
    graph.exact_temporal_value(id)
}

/// Compares a stored temporal id with an exact constant, skipping the exact key when
/// the cached f64 instants alone decide the order.
///
/// The cached instant of a value is within a few ulps (plus a sub-femtosecond
/// fraction error) of its exact instant, so a cached difference beyond `margin`
/// (and beyond the fourteen-hour window for mixed timezone presence) has the sign
/// of the exact difference. Everything closer, every capacity-checked evaluation
/// and every id without a cached value takes the exact comparison.
#[inline]
pub(super) fn temporal_cmp_of_id(graph: &Graph, id: Id, exact: ExactTemporal<'_>, approx: Option<Temporal>) -> Option<std::cmp::Ordering> {
    if let Some(c) = approx {
        if !budget::temporal_capacity_active() {
            if let Some(v) = graph.temporal_value(id) {
                if v.kind != c.kind {
                    return None;
                }
                if let Some(order) = approx_temporal_order(v, c) {
                    return Some(order);
                }
            }
        }
    }
    exact_temporal_cmp_of_id(graph, id, exact)
}

/// The exact fallback of [`temporal_cmp_of_id`], kept out of line so the cached
/// fast path stays small enough to inline into scans.
#[inline(never)]
pub(super) fn exact_temporal_cmp_of_id(graph: &Graph, id: Id, exact: ExactTemporal<'_>) -> Option<std::cmp::Ordering> {
    temporal_of_id(graph, id).and_then(|v| ExactTemporal::compare(v, exact))
}

/// The order of two same-family cached temporals when their f64 instants alone
/// decide it (see [`temporal_cmp_of_id`]); `None` means "use the exact keys".
#[inline]
pub(super) fn approx_temporal_order(v: Temporal, c: Temporal) -> Option<std::cmp::Ordering> {
    let d = v.instant - c.instant;
    let margin = 1.0 + 8.0 * f64::EPSILON * (v.instant.abs() + c.instant.abs());
    let band = if v.has_tz == c.has_tz { 0.0 } else { 14.0 * 3600.0 };
    if d > band + margin {
        Some(std::cmp::Ordering::Greater)
    } else if d < -(band + margin) {
        Some(std::cmp::Ordering::Less)
    } else {
        None
    }
}

/// The cached approximate temporal of a graph-term variable or a constant, for
/// [`approx_temporal_order`] only. `None` (including under an active temporal
/// capacity budget) sends the caller to the exact path.
pub(super) fn eval_approx_temporal(graph: &Graph, local: &LocalVocab, b: &Bindings, row: &[Id], e: &Expression) -> Option<Temporal> {
    if budget::temporal_capacity_active() {
        return None;
    }
    match e {
        Expression::Variable(v) if local.correlation.contains_key(v) => None,
        Expression::Variable(v) => {
            let id = row[b.col(v)?];
            if id == NO_ID || is_local(id) { None } else { graph.temporal_value(id) }
        }
        Expression::Literal(l) if l.language().is_none() => Temporal::of_lit(l.value(), l.datatype().as_str()),
        _ => None,
    }
}

/// The lexical form of an expression IF it is an exact-valued numeric operand (an
/// integer subtype or xsd:decimal — NOT float/double). Used to re-check comparisons that
/// the f64 fast path collapsed (integers > 2^53, high-precision decimals). Only reached
/// when the f64 comparison was equal, so the allocation is rare.
pub(super) fn eval_exact_lexical(graph: &Graph, local: &LocalVocab, b: &Bindings, row: &[Id], e: &Expression) -> Option<String> {
    use Expression::*;
    match e {
        Variable(v) if local.correlation.contains_key(v) => None,
        Variable(v) => {
            let id = row[b.col(v)?];
            if id == NO_ID {
                None
            } else if is_local(id) {
                exact_lexical_of_term(local.term(id))
            } else {
                graph.exact_numeric_lexical(id)
            }
        }
        Literal(_) => exact_lexical_of_term(&eval_expr(graph, local, b, row, e).ok().and_then(|v| match v {
            Value::Term(t) => Some(t),
            _ => None,
        })?),
        UnaryPlus(a) => eval_exact_lexical(graph, local, b, row, a),
        UnaryMinus(a) => eval_exact_lexical(graph, local, b, row, a).map(|s| match s.strip_prefix('-') {
            Some(r) => r.to_string(),
            None => format!("-{s}"),
        }),
        _ => None,
    }
}

// Every exact lexical shortcut validates the original RDF datatype first.
pub(super) fn exact_lexical_of_literal(l: &Literal) -> Option<&str> {
    (l.language().is_none() && sparq_core::exact_numeric_literal_valid(l.value(), l.datatype().as_str()))
        .then_some(l.value())
}

pub(super) fn exact_lexical_of_term(t: &Term) -> Option<String> {
    match t {
        Term::Literal(l) => exact_lexical_of_literal(l).map(str::to_owned),
        _ => None,
    }
}

// ── Compiled-expression helpers (sq-7d3dj.4) ─────────────────────────────────────────────
// Mirror of `eval_numeric` / `eval_temporal` / `eval_dec` / `eval_exact_lexical` for
// [`CompiledExpr`]: no `Bindings` look-up per row — `Var(c)` uses `row[c]` directly.
// sq-7d3dj.4

/// Fast numeric evaluation on a pre-compiled expression. Mirrors [`eval_numeric`]. sq-7d3dj.4.
pub(super) fn eval_compiled_numeric(graph: &Graph, local: &LocalVocab, row: &[Id], e: &CompiledExpr) -> Option<f64> {
    use CompiledExpr::*;
    match e {
        Var(col) => {
            let c = (*col)?;
            let id = row[c];
            if id == NO_ID { None } else if is_local(id) { local.numeric(id) } else { graph.numeric_value(id) }
        }
        // sq-6b1lj: datatype-aware, verbatim-validated constant, matching `eval_numeric`.
        Literal(l, _) => numeric_cache_f64(l),
        Add(a, d) => Some(eval_compiled_numeric(graph, local, row, a)? + eval_compiled_numeric(graph, local, row, d)?),
        Subtract(a, d) => {
            Some(eval_compiled_numeric(graph, local, row, a)? - eval_compiled_numeric(graph, local, row, d)?)
        }
        Multiply(a, d) => {
            Some(eval_compiled_numeric(graph, local, row, a)? * eval_compiled_numeric(graph, local, row, d)?)
        }
        Divide(a, d) => {
            Some(eval_compiled_numeric(graph, local, row, a)? / eval_compiled_numeric(graph, local, row, d)?)
        }
        UnaryPlus(a) => eval_compiled_numeric(graph, local, row, a),
        UnaryMinus(a) => Some(-eval_compiled_numeric(graph, local, row, a)?),
        _ => None,
    }
}

/// Fast temporal evaluation on a pre-compiled expression. Mirrors [`eval_temporal`]. sq-7d3dj.4.
pub(super) fn eval_compiled_temporal<'a>(graph: &'a Graph, local: &'a LocalVocab, row: &[Id], e: &'a CompiledExpr) -> Option<ExactTemporal<'a>> {
    use CompiledExpr::*;
    match e {
        Var(col) => {
            let c = (*col)?;
            let id = row[c];
            if id == NO_ID {
                None
            } else if is_local(id) {
                temporal_of_term(local.term(id))
            } else {
                temporal_of_id(graph, id)
            }
        }
        Literal(l, _) => temporal_of_lit(l),
        _ => None,
    }
}

/// Mirrors [`eval_approx_temporal`] on a pre-compiled expression.
pub(super) fn eval_compiled_approx_temporal(graph: &Graph, row: &[Id], e: &CompiledExpr) -> Option<Temporal> {
    if budget::temporal_capacity_active() {
        return None;
    }
    match e {
        CompiledExpr::Var(col) => {
            let id = row[(*col)?];
            if id == NO_ID || is_local(id) { None } else { graph.temporal_value(id) }
        }
        CompiledExpr::Literal(l, _) if l.language().is_none() => Temporal::of_lit(l.value(), l.datatype().as_str()),
        _ => None,
    }
}

/// Exact decimal evaluation on a pre-compiled expression. Mirrors [`eval_dec`]. sq-7d3dj.4.
pub(super) fn eval_compiled_dec(graph: &Graph, local: &LocalVocab, row: &[Id], e: &CompiledExpr) -> Option<Dec> {
    use CompiledExpr::*;
    match e {
        Var(col) => {
            let c = (*col)?;
            let id = row[c];
            if id == NO_ID {
                None
            } else if is_local(id) {
                exact_lexical_of_term(local.term(id)).and_then(|s| Dec::parse(&s))
            } else {
                Dec::parse(&graph.exact_numeric_lexical(id)?)
            }
        }
        Literal(l, exact) => if *exact { Dec::parse(l.value()) } else { None },
        Add(a, d) => eval_compiled_dec(graph, local, row, a)?.checked_add(eval_compiled_dec(graph, local, row, d)?),
        Subtract(a, d) => {
            eval_compiled_dec(graph, local, row, a)?.checked_sub(eval_compiled_dec(graph, local, row, d)?)
        }
        Multiply(a, d) => {
            eval_compiled_dec(graph, local, row, a)?.checked_mul(eval_compiled_dec(graph, local, row, d)?)
        }
        UnaryPlus(a) => eval_compiled_dec(graph, local, row, a),
        UnaryMinus(a) => {
            let d = eval_compiled_dec(graph, local, row, a)?;
            Some(Dec { mant: d.mant.checked_neg()?, scale: d.scale })
        }
        _ => None,
    }
}

/// Exact lexical form for precise decimal/integer comparison. Mirrors [`eval_exact_lexical`]. sq-7d3dj.4.
pub(super) fn eval_compiled_exact_lexical(graph: &Graph, local: &LocalVocab, row: &[Id], e: &CompiledExpr) -> Option<String> {
    use CompiledExpr::*;
    match e {
        Var(col) => {
            let c = (*col)?;
            let id = row[c];
            if id == NO_ID {
                None
            } else if is_local(id) {
                exact_lexical_of_term(local.term(id))
            } else {
                graph.exact_numeric_lexical(id)
            }
        }
        Literal(l, exact) => exact.then(|| l.value().to_owned()),
        UnaryPlus(a) => eval_compiled_exact_lexical(graph, local, row, a),
        UnaryMinus(a) => eval_compiled_exact_lexical(graph, local, row, a).map(|s| match s.strip_prefix('-') {
            Some(r) => r.to_string(),
            None => format!("-{}", s),
        }),
        _ => None,
    }
}

/// Exact comparison of two `xsd:decimal` / integer lexical forms (no f64). `None` if
/// either is not a well-formed decimal. Integers are decimals with an empty fraction.
pub(super) fn cmp_decimal_str(a: &str, b: &str) -> Option<Ordering> {
    let (na, ia, fa) = split_decimal(a)?;
    let (nb, ib, fb) = split_decimal(b)?;
    let a_zero = ia.is_empty() && fa.is_empty();
    let b_zero = nb_is_zero(ib, fb);
    if a_zero && b_zero {
        return Some(Ordering::Equal);
    }
    let mag = ia
        .len()
        .cmp(&ib.len())
        .then_with(|| ia.cmp(ib))
        .then_with(|| {
            let n = fa.len().max(fb.len());
            // Compare fractional digits with implicit trailing-zero padding.
            (0..n)
                .map(|i| (fa.as_bytes().get(i).copied().unwrap_or(b'0'), fb.as_bytes().get(i).copied().unwrap_or(b'0')))
                .find_map(|(x, y)| (x != y).then(|| x.cmp(&y)))
                .unwrap_or(Ordering::Equal)
        });
    let neg_a = na && !a_zero;
    let neg_b = nb && !b_zero;
    Some(match (neg_a, neg_b) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => mag,
        (true, true) => mag.reverse(),
    })
}

pub(super) fn nb_is_zero(int: &str, frac: &str) -> bool {
    int.is_empty() && frac.is_empty()
}

/// Significant decimal digits in a numeric lexical — used to decide whether the f64
/// sargable path is precision-safe (<= 15 digits round-trips through f64 unambiguously).
pub(super) fn sig_digits(s: &str) -> usize {
    let (_, int, frac) = match split_decimal(s) {
        Some(p) => p,
        None => return usize::MAX,
    };
    if int.is_empty() {
        // 0.00123 -> significant digits start at the first non-zero fraction digit.
        frac.trim_start_matches('0').len()
    } else {
        int.len() + frac.len()
    }
}

/// `true` if the expression performs arithmetic (`+ - * /`, possibly under a unary sign), so a
/// comparison over it must not use the untyped f64 fast path: integer/decimal arithmetic is
/// decided exactly, and float/double arithmetic in its promoted tier by the typed evaluator
/// (value comparison is monotonic; arithmetic introduces rounding the tier determines). The
/// unary sign alone is exact in every tier, and numeric functions are not evaluated by the
/// fast path at all.
pub(super) fn expr_has_arith(e: &Expression) -> bool {
    use Expression::*;
    match e {
        Add(..) | Subtract(..) | Multiply(..) | Divide(..) => true,
        UnaryPlus(a) | UnaryMinus(a) => expr_has_arith(a),
        _ => false,
    }
}

/// Evaluates an expression EXACTLY as a fixed-point decimal, for integer/decimal operands
/// and `+ - *`. `None` for anything not exactly representable this way (division, doubles,
/// non-numeric operands, overflow) — the caller then uses the f64 path.
pub(super) fn eval_dec(graph: &Graph, local: &LocalVocab, b: &Bindings, row: &[Id], e: &Expression) -> Option<Dec> {
    use Expression::*;
    match e {
        Variable(v) if local.correlation.contains_key(v) => None,
        Variable(v) => {
            let id = row[b.col(v)?];
            if id == NO_ID {
                None
            } else if is_local(id) {
                exact_lexical_of_term(local.term(id)).and_then(|s| Dec::parse(&s))
            } else {
                Dec::parse(&graph.exact_numeric_lexical(id)?)
            }
        }
        Literal(l) => Dec::parse(exact_lexical_of_literal(l)?),
        Add(a, c) => eval_dec(graph, local, b, row, a)?.checked_add(eval_dec(graph, local, b, row, c)?),
        Subtract(a, c) => eval_dec(graph, local, b, row, a)?.checked_sub(eval_dec(graph, local, b, row, c)?),
        Multiply(a, c) => eval_dec(graph, local, b, row, a)?.checked_mul(eval_dec(graph, local, b, row, c)?),
        UnaryPlus(a) => eval_dec(graph, local, b, row, a),
        UnaryMinus(a) => {
            let d = eval_dec(graph, local, b, row, a)?;
            Some(Dec { mant: d.mant.checked_neg()?, scale: d.scale })
        }
        _ => None,
    }
}

/// An ORDERING comparison (`<`, `>`, `<=`, `>=`). SPARQL only orders operands of
/// compatible types (numeric vs numeric, boolean vs boolean, or two literals of the
/// same datatype); anything else is a TYPE ERROR (`Value::Error`), which a FILTER
/// turns into "excluded". (Distinct from the lenient total order `compare_values`
/// used by ORDER BY / MIN / MAX, which must order across every type.)
pub(super) fn cmp_expr(graph: &Graph, local: &LocalVocab, b: &Bindings, row: &[Id], a: &Expression, c: &Expression, f: impl Fn(Ordering) -> bool) -> Result<Value, String> {
    if budget::strict_numeric() {
        let (x, y) = (eval_expr(graph, local, b, row, a)?, eval_expr(graph, local, b, row, c)?);
        if let (Some(a), Some(b)) = (as_numeric(&x), as_numeric(&y)) {
            if a.is_nan() || b.is_nan() { return Ok(Value::Bool(false)); }
        }
        return Ok(value_compare_strict(&x, &y).map(|o| Value::Bool(f(o))).unwrap_or(Value::Error));
    }
    // EXACT path: integer/decimal arithmetic (`+ - *`) must not round through f64, which
    // can flip an ordering (`0.1 + 0.2` < `0.3` in f64). Only attempted when arithmetic is
    // present (the common, arithmetic-free comparison keeps the f64 fast path below).
    let arith = expr_has_arith(a) || expr_has_arith(c);
    if arith {
        if let (Some(da), Some(db)) = (eval_dec(graph, local, b, row, a), eval_dec(graph, local, b, row, c)) {
            if let Some(o) = da.cmp(db) {
                return Ok(Value::Bool(f(o)));
            }
        }
    }
    // Fast path: both sides numeric -> compare f64 directly, no term materialised.
    // Reaching here means BOTH operands are numeric, so a `None` partial_cmp is a
    // NaN value (op:numeric ordering of NaN is false) — NOT a cross-type error.
    // The f64 fast path below evaluates arithmetic UNTYPED (always in f64), but XPath evaluates
    // it in the promoted tier: `"16777217"^^xsd:float + 1` is a FLOAT and rounds to 16777216.
    // Arithmetic the exact path above did not decide (a float/double operand) takes the typed
    // evaluator instead.
    let fast = if arith { None } else { eval_numeric(graph, local, b, row, a).zip(eval_numeric(graph, local, b, row, c)) };
    if let Some((x, y)) = fast {
        // f64 rounding is monotonic — it only ever COLLAPSES distinct values to equal,
        // never flips an ordering. So re-check exactly ONLY when f64 says equal (catches
        // integers > 2^53 and high-precision decimals that share an f64).
        if x == y {
            if let (Some(la), Some(lb)) = (eval_exact_lexical(graph, local, b, row, a), eval_exact_lexical(graph, local, b, row, c)) {
                if let Some(ord) = cmp_decimal_str(&la, &lb) {
                    return Ok(Value::Bool(f(ord)));
                }
            }
        }
        // An unequal pair that `xs:float` promotion could tie takes the typed path below.
        if x == y || !f32_promotion_may_tie(x, y) {
            return Ok(Value::Bool(x.partial_cmp(&y).map(&f).unwrap_or(false)));
        }
    }
    // Cached f64 instants decide same-family pairs that are far apart; anything
    // closer falls through to the exact keys below.
    if let (Some(va), Some(vb)) = (eval_approx_temporal(graph, local, b, row, a), eval_approx_temporal(graph, local, b, row, c)) {
        if va.kind == vb.kind {
            if let Some(o) = approx_temporal_order(va, vb) {
                return Ok(Value::Bool(f(o)));
            }
        }
    }
    // Fast path: both sides temporal -> compare exact borrowed timeline values, no term
    // materialised. `None` from `cmp_t` is exactly the strict path's type-error cases
    // (cross-family dateTime vs date, or mixed timezone presence inside the ±14h window).
    if let (Some(ta), Some(tb)) = (eval_temporal(graph, local, b, row, a), eval_temporal(graph, local, b, row, c)) {
        return Ok(match ExactTemporal::compare(ta, tb) {
            Some(o) => Value::Bool(f(o)),
            None => Value::Error,
        });
    }
    let (x, y) = (eval_expr(graph, local, b, row, a)?, eval_expr(graph, local, b, row, c)?);
    Ok(relational_value(&x, &y, arith, f))
}

/// The typed result of a relational operator (`<`, `<=`, `>`, `>=`) once both operands are
/// evaluated. An incomparable pair is a type error, EXCEPT two numerics that are unordered
/// because one is NaN: XPath `op:numeric-less-than` / `-greater-than` return false there,
/// whether the NaN is stored or computed. Other unordered numeric pairs are false only when
/// `arith` sent the comparison here (an arithmetic operand the f64 fast path used to decide).
pub(super) fn relational_value(x: &Value, y: &Value, arith: bool, f: impl Fn(Ordering) -> bool) -> Value {
    // A NaN operand (stored NaN misses the numeric cache and lands here) is false, never a
    // type error, matching the strict-capacity path and XPath numeric comparisons.
    if let (Some(a), Some(b)) = (as_numeric(x), as_numeric(y)) {
        if a.is_nan() || b.is_nan() {
            return Value::Bool(false);
        }
    }
    match value_compare_strict(x, y) {
        Some(o) => Value::Bool(f(o)),
        None if arith && as_numeric(x).is_some() && as_numeric(y).is_some() => Value::Bool(false),
        None => Value::Error,
    }
}

/// SPARQL `=` (and, negated, `!=`). See [`values_equal`].
pub(super) fn equal_expr(graph: &Graph, local: &LocalVocab, b: &Bindings, row: &[Id], a: &Expression, c: &Expression) -> Result<Value, String> {
    if budget::strict_numeric() {
        let (x, y) = (eval_expr(graph, local, b, row, a)?, eval_expr(graph, local, b, row, c)?);
        return Ok(values_equal(&x, &y).map(Value::Bool).unwrap_or(Value::Error));
    }
    // EXACT integer/decimal arithmetic equality (see `cmp_expr`) — `0.1 + 0.2 = 0.3`.
    let arith = expr_has_arith(a) || expr_has_arith(c);
    if arith {
        if let (Some(da), Some(db)) = (eval_dec(graph, local, b, row, a), eval_dec(graph, local, b, row, c)) {
            if let Some(o) = da.cmp(db) {
                return Ok(Value::Bool(o == Ordering::Equal));
            }
        }
    }
    // Fast path: both numeric (NaN == NaN is false, matching op:numeric-equal).
    // The f64 fast path below evaluates arithmetic UNTYPED (always in f64), but XPath evaluates
    // it in the promoted tier: `"16777217"^^xsd:float + 1` is a FLOAT and rounds to 16777216.
    // Arithmetic the exact path above did not decide (a float/double operand) takes the typed
    // evaluator instead.
    let fast = if arith { None } else { eval_numeric(graph, local, b, row, a).zip(eval_numeric(graph, local, b, row, c)) };
    if let Some((x, y)) = fast {
        // Re-check exactly when f64 says equal (see `cmp_expr`): distinct integers > 2^53
        // or high-precision decimals can share an f64 and must not be reported equal.
        if x == y {
            if let (Some(la), Some(lb)) = (eval_exact_lexical(graph, local, b, row, a), eval_exact_lexical(graph, local, b, row, c)) {
                if let Some(ord) = cmp_decimal_str(&la, &lb) {
                    return Ok(Value::Bool(ord == Ordering::Equal));
                }
            }
        }
        // An unequal pair that `xs:float` promotion could tie takes the typed path below.
        if x == y || !f32_promotion_may_tie(x, y) {
            return Ok(Value::Bool(x == y));
        }
    }
    // Fast path: both temporal. Same-family operands decide by timeline (`None` =
    // the indeterminate mixed-timezone window -> type error); dateTime and date are
    // DISJOINT value spaces -> known different (matching `values_equal`).
    if let (Some(ta), Some(tb)) = (eval_temporal(graph, local, b, row, a), eval_temporal(graph, local, b, row, c)) {
        if ta.kind != tb.kind {
            return Ok(Value::Bool(false));
        }
        return Ok(match ExactTemporal::compare(ta, tb) {
            Some(o) => Value::Bool(o == Ordering::Equal),
            None => Value::Error,
        });
    }
    let (x, y) = (eval_expr(graph, local, b, row, a)?, eval_expr(graph, local, b, row, c)?);
    Ok(match values_equal(&x, &y) {
        Some(eq) => Value::Bool(eq),
        None => Value::Error,
    })
}

/// Datatype family of a literal-valued operand, the basis for OPEN-WORLD `=` and
/// the ordering operators: only operands within a comparable family decide; unknown
/// datatypes and ill-formed lexicals are type errors unless the terms are identical.
pub(super) enum LitKind<'a> {
    /// A numeric-datatype operand; `None` = ill-formed lexical.
    Num(Option<Num>),
    Str(&'a str),
    /// A boolean-datatype operand; `None` = ill-formed lexical.
    Bool(Option<bool>),
    /// xsd:dateTime / xsd:dateTimeStamp on the timeline; `None` = ill-formed.
    DateTime(Option<ExactTimeline<'a>>),
    /// xsd:date on the timeline (midnight); `None` = ill-formed.
    Date(Option<ExactTimeline<'a>>),
    /// Another XSD datatype (time, duration, gYear, …): (datatype IRI, lexical).
    OtherXsd(&'a str, &'a str),
    /// Language-tagged: (lowercased tag, value).
    Lang(String, &'a str),
    /// A literal of a NON-XSD (unknown) datatype: open-world, never decidable.
    Unknown,
    NotLiteral,
}

pub(super) fn lit_kind(v: &Value) -> LitKind<'_> {
    match v {
        Value::Num(n) => LitKind::Num(Some(*n)),
        Value::Bool(b) => LitKind::Bool(Some(*b)),
        Value::Term(Term::Literal(l)) => {
            if let Some(tag) = l.language() {
                return LitKind::Lang(tag.to_ascii_lowercase(), l.value());
            }
            let dt = l.datatype();
            if is_numeric_dt(l) {
                LitKind::Num(numeric_capacity::operand(l))
            } else if dt == xsd::STRING {
                LitKind::Str(l.value())
            } else if dt == xsd::BOOLEAN {
                LitKind::Bool(as_bool_val(v))
            } else if dt == xsd::DATE_TIME || dt == xsd::DATE_TIME_STAMP {
                LitKind::DateTime(temporal_of_lit(l).map(|value| value.timeline))
            } else if dt == xsd::DATE {
                LitKind::Date(temporal_of_lit(l).map(|value| value.timeline))
            } else if dt.as_str().starts_with("http://www.w3.org/2001/XMLSchema#") {
                LitKind::OtherXsd(dt.as_str(), l.value())
            } else {
                LitKind::Unknown
            }
        }
        _ => LitKind::NotLiteral,
    }
}

/// Whether `t` is an `xsd:double` / `xsd:float` `NaN` literal: the one RDF term that is
/// not `=` to itself (op:numeric-equal), so identical-term shortcuts must skip it.
pub(super) fn term_is_nan_literal(t: &Term) -> bool {
    matches!(t, Term::Literal(l) if l.language().is_none() && is_nan_lexical(l.value(), l.datatype().as_str()))
}

/// [`term_is_nan_literal`] for an id, without materialising the term.
#[cfg(feature = "id-filter-fastpath")]
pub(super) fn id_is_nan_literal(graph: &Graph, local: &LocalVocab, id: Id) -> bool {
    if id == NO_ID || dict::is_inline(id) {
        false
    } else if is_local(id) {
        term_is_nan_literal(local.term(id))
    } else {
        matches!(graph.dict.term_parts(id), dict::TermParts::Lit { value, datatype, lang: None } if is_nan_lexical(value, datatype))
    }
}

/// XSD's only `NaN` lexical is the exact string `NaN` (no padding, no sign).
pub(super) fn is_nan_lexical(value: &str, datatype: &str) -> bool {
    value == "NaN" && (datatype == xsd::DOUBLE.as_str() || datatype == xsd::FLOAT.as_str())
}

// Exact borrowed date/dateTime keys live in core; approximate load-time epoch
// caches remain available separately for representation consumers.

/// SPARQL `=` (and, negated, `!=`) as a three-valued result: `Some(true/false)` for a
/// decided comparison, `None` for a type error. OPEN-WORLD rules: identical terms are
/// equal regardless of datatype; non-literal terms decide by identity; within a
/// comparable literal family the VALUES decide; a language-tagged literal is KNOWN
/// different from any non-language literal; everything else — unknown datatypes,
/// ill-formed lexicals, cross-family pairs — is a TYPE ERROR (`"a"^^ex:dt != "b"^^ex:other`
/// filters the row out rather than evaluating to true).
pub(super) fn values_equal(x: &Value, y: &Value) -> Option<bool> {
    if budget::strict_numeric() {
        // Equality of identical terms may otherwise bypass every numeric consumer.
        if let (Some(a), Some(b)) = (as_numeric(x), as_numeric(y)) {
            // NaN is not numerically equal to itself, even with identical RDF terms.
            return Some(num_compare(a, b) == Some(Ordering::Equal));
        }
    }
    if matches!(x, Value::Unbound | Value::Error) || matches!(y, Value::Unbound | Value::Error) {
        return None;
    }
    if let (Value::Term(p), Value::Term(q)) = (x, y) {
        if p == q {
            // sameTerm decides even for unknown datatypes, except a float/double NaN:
            // op:numeric-equal(NaN, NaN) is false, matching the strict-capacity path above.
            return Some(!term_is_nan_literal(p));
        }
        // RDF 1.2 triple terms compare componentwise, with VALUE equality on the
        // objects (`<<(:a :b 01)>> = <<(:a :b 1)>>` is true, errors propagate).
        if let (Term::Triple(a), Term::Triple(b)) = (p, q) {
            if a.subject != b.subject || a.predicate != b.predicate {
                return Some(false);
            }
            return values_equal(&Value::Term(a.object.clone()), &Value::Term(b.object.clone()));
        }
        if !matches!(p, Term::Literal(_)) || !matches!(q, Term::Literal(_)) {
            return Some(false); // IRI / bnode: identity decides
        }
    }
    let (ka, kb) = (lit_kind(x), lit_kind(y));
    if matches!(ka, LitKind::NotLiteral) || matches!(kb, LitKind::NotLiteral) {
        return Some(false); // computed literal vs non-literal term
    }
    use LitKind::*;
    match (ka, kb) {
        (Num(Some(a)), Num(Some(b))) => Some(num_compare(a, b) == Some(Ordering::Equal)),
        (Num(_), Num(_)) => None, // ill-formed numeric (and not sameTerm)
        (Str(a), Str(b)) => Some(a == b),
        (Bool(Some(a)), Bool(Some(b))) => Some(a == b),
        (Bool(_), Bool(_)) => None,
        (DateTime(Some(a)), DateTime(Some(b))) => ExactTimeline::compare(a, b).map(|o| o == Ordering::Equal),
        (Date(Some(a)), Date(Some(b))) => ExactTimeline::compare(a, b).map(|o| o == Ordering::Equal),
        (DateTime(_), DateTime(_)) | (Date(_), Date(_)) => None,
        // date and dateTime values are disjoint -> known different.
        // An ill-formed operand (e.g. a timezone-free dateTimeStamp) is not a value: error.
        (DateTime(Some(_)), Date(Some(_))) | (Date(Some(_)), DateTime(Some(_))) => Some(false),
        // Nor is it known different from a language-tagged literal (#3902): still an error.
        (DateTime(None) | Date(None), _) | (_, DateTime(None) | Date(None)) => None,
        // A language-tagged literal equals only a literal with the same (ci) tag.
        (Lang(t1, v1), Lang(t2, v2)) => Some(t1 == t2 && v1 == v2),
        (Lang(..), _) | (_, Lang(..)) => Some(false),
        (OtherXsd(d1, l1), OtherXsd(d2, l2)) if d1 == d2 => Some(l1 == l2),
        // Cross-family, unknown datatypes, unknown XSD pairings: open world -> error.
        _ => None,
    }
}

/// Strict SPARQL value comparison for relational operators: `Some(ordering)` only
/// when the operands are value-comparable (same family per [`lit_kind`]), else
/// `None` (a type error).
pub(super) fn value_compare_strict(x: &Value, y: &Value) -> Option<Ordering> {
    use LitKind::*;
    match (lit_kind(x), lit_kind(y)) {
        (Num(Some(a)), Num(Some(b))) => num_compare(a, b),
        (Str(a), Str(b)) => Some(a.cmp(b)),
        (Bool(Some(a)), Bool(Some(b))) => Some(a.cmp(&b)),
        (DateTime(Some(a)), DateTime(Some(b))) => ExactTimeline::compare(a, b),
        (Date(Some(a)), Date(Some(b))) => ExactTimeline::compare(a, b),
        // Same language tag: compare values (the suites' lenient extension).
        (Lang(t1, v1), Lang(t2, v2)) if t1 == t2 => Some(v1.cmp(v2)),
        // Same other-XSD datatype: lexical order (correct for time, gYear, …).
        (OtherXsd(d1, l1), OtherXsd(d2, l2)) if d1 == d2 => Some(l1.cmp(l2)),
        _ => None,
    }
}

/// The TOTAL-order verdict for a same-family temporal pair `value_compare_strict` left
/// indeterminate (the mixed-timezone ±14h window) — `None` for every other pair, which
/// keeps the caller's own fallback. Reached ONLY from the `CompareTerm` total order, never
/// from a relational operator. sq-2k5py
#[cold]
pub(super) fn temporal_total_cmp(x: &Value, y: &Value) -> Option<Ordering> {
    match (lit_kind(x), lit_kind(y)) {
        (LitKind::DateTime(Some(a)), LitKind::DateTime(Some(b)))
        | (LitKind::Date(Some(a)), LitKind::Date(Some(b))) => Some(ExactTimeline::compare_total(a, b)),
        _ => None,
    }
}

pub(super) fn as_bool_val(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::Term(Term::Literal(l)) if l.datatype() == xsd::BOOLEAN => match l.value() {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

/// Three-valued effective boolean: `None` is a SPARQL error (type error or unbound),
/// used by the logical operators to implement SPARQL's 3-valued `&&` / `||` / `!`.
pub(super) fn ebv3(v: &Value, semantics: crate::EbvSemantics) -> Option<bool> {
    ebv(v, semantics)
}

pub(super) fn and3(x: Option<bool>, y: Option<bool>) -> Value {
    match (x, y) {
        (Some(false), _) | (_, Some(false)) => Value::Bool(false),
        (Some(true), Some(true)) => Value::Bool(true),
        _ => Value::Error,
    }
}

pub(super) fn or3(x: Option<bool>, y: Option<bool>) -> Value {
    match (x, y) {
        (Some(true), _) | (_, Some(true)) => Value::Bool(true),
        (Some(false), Some(false)) => Value::Bool(false),
        _ => Value::Error,
    }
}

/// TYPED arithmetic with XPath operand promotion: the result carries the promoted
/// datatype (int+int→int, decimal involved→decimal, …, int/int→decimal) and exact
/// int/decimal value. Used for RESULT CONSTRUCTION (BIND / SELECT expressions /
/// aggregates) — the comparison operators keep their f64 fast path (`eval_numeric`),
/// where only the value matters.
pub(super) fn arith(graph: &Graph, local: &LocalVocab, b: &Bindings, row: &[Id], a: &Expression, c: &Expression, op: ArithOp) -> Result<Value, String> {
    let (x, y) = (eval_expr(graph, local, b, row, a)?, eval_expr(graph, local, b, row, c)?);
    Ok(match (as_numeric(&x), as_numeric(&y)) {
        (Some(p), Some(q)) => numeric_capacity::binop(p, q, op).map(Value::Num).unwrap_or(Value::Error),
        _ => Value::Error,
    })
}

/// The TYPED numeric value of an evaluated operand: a computed numeric as-is, a
/// numeric literal parsed per its datatype. `None` for non-numerics AND for
/// ill-formed numeric literals (both are SPARQL type errors in arithmetic).
pub(super) fn as_numeric(v: &Value) -> Option<Num> {
    match v {
        Value::Num(n) => Some(*n),
        Value::Term(Term::Literal(l)) => numeric_capacity::operand(l),
        _ => None,
    }
}

// XPath numeric-unary-plus returns its numeric operand unchanged.
// Validate the value space without imposing the arithmetic representation bound.
pub(super) fn unary_plus(value: Value) -> Value {
    match &value {
        Value::Num(_) => value,
        Value::Term(Term::Literal(l))
            if sparq_core::numeric_literal_valid(l.value(), l.datatype().as_str()) => value,
        _ => Value::Error,
    }
}

// SUBSTR's SPARQL signature requires integer operands, not numeric coercion.
pub(super) fn integer_argument(v: &Value) -> Option<i128> {
    match v {
        Value::Num(Num::Int(n)) => Some(i128::from(*n)),
        Value::Term(Term::Literal(l))
            if l.language().is_none()
                && sparq_core::is_integer_datatype(l.datatype().as_str())
                && sparq_core::numeric_literal_valid(l.value(), l.datatype().as_str()) =>
        {
            numeric_capacity::representable(true,
                l.value().parse().ok())
        }
        _ => None,
    }
}

/// The lexical of a well-formed `xsd:decimal`, `xsd:integer` or unbounded integer-subtype
/// literal too large for the i128 tower (`Num::of_literal` declines it). Such a value is
/// still a number: the ORDER BY total order compares it exactly by its lexical instead of
/// as an opaque string. The raw lexical is validated as is by `numeric_literal_valid`
/// (no trimming of any whitespace, ASCII or Unicode), which also checks a subtype's sign
/// facet. The bounded subtypes (`xsd:long`, ...) cannot hold such a value, so they stay `None`.
pub(super) fn beyond_tower_lexical(v: &Value) -> Option<&str> {
    let Value::Term(Term::Literal(l)) = v else { return None };
    if l.language().is_some() || Num::of_literal(l).is_some() {
        return None;
    }
    let dt = l.datatype();
    let unbounded = dt == xsd::DECIMAL
        || dt == xsd::INTEGER
        || dt == xsd::NEGATIVE_INTEGER
        || dt == xsd::NON_POSITIVE_INTEGER
        || dt == xsd::POSITIVE_INTEGER
        || dt == xsd::NON_NEGATIVE_INTEGER;
    let lex = l.value();
    (unbounded && sparq_core::numeric_literal_valid(lex, dt.as_str()) && split_decimal(lex).is_some())
        .then_some(lex)
}

/// An exact decimal lexical for a numeric `Value`: an integer/decimal (in or beyond the
/// tower). `None` for float/double, whose exact value is its `f64`.
#[cold]
pub(super) fn exact_decimal_lexical(v: &Value) -> Option<String> {
    match as_numeric(v) {
        Some(n) if n.to_dec().is_some() => Some(n.lexical()),
        Some(_) => None,
        None => beyond_tower_lexical(v).map(str::to_string),
    }
}

pub(super) fn as_num(v: &Value) -> Option<f64> {
    if budget::strict_numeric() && !matches!(v, Value::Bool(_)) {
        return as_numeric(v).map(Num::f64);
    }
    match v {
        Value::Num(n) => Some(n.f64()),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        // Keep scalar comparisons and caches on the same raw
        // lexical/facet acceptance path; string constructors preprocess separately.
        Value::Term(Term::Literal(l)) => numeric_cache_f64(l),
        _ => None,
    }
}

pub(super) fn is_numeric_dt(l: &Literal) -> bool {
    let dt = l.datatype().as_str();
    sparq_core::is_integer_datatype(dt)
        || dt == xsd::DECIMAL.as_str()
        || dt == xsd::DOUBLE.as_str()
        || dt == xsd::FLOAT.as_str()
}

/// Returns the raw literal's numeric image within the shared cache lane.
/// Invalid lexical forms, subtype facets and unsupported representations return None.
#[inline]
pub(super) fn numeric_cache_f64(l: &Literal) -> Option<f64> {
    if is_numeric_dt(l) && Num::of_literal(l).is_some() {
        sparq_core::numeric_lexical_f64(l.value(), l.datatype().as_str())
    } else {
        None
    }
}

// sq-vezew (epic sq-qonbz, Phase 4): the engine implements the substrate's
// `CompareTerm` trait for its `Value` — a set of ZERO-COST wrappers over the existing
// `value_str` / `as_num` / `value_compare_strict` helpers — so the shared
// `compare::compare_terms` total-order algorithm can drive the engine's `Value` with no vtable.
// The class ranks, the within-class string / numeric / strict arms, and the recursive
// triple-term order all live in the substrate now; this impl only surfaces the observations.
impl CompareTerm for Value {
    #[inline]
    fn term_class(&self) -> TermClass {
        match self {
            Value::Unbound | Value::Error => TermClass::ErrorOrUnbound,
            Value::Term(Term::BlankNode(_)) => TermClass::Blank,
            Value::Term(Term::NamedNode(_)) => TermClass::Iri,
            // SPARQL 1.2 total-order extension: triple terms sort AFTER literals.
            Value::Term(Term::Triple(_)) => TermClass::Triple,
            _ => TermClass::Literal, // literals, incl. computed numerics / booleans
        }
    }
    #[inline]
    fn literal_kind(&self) -> LiteralKind {
        // sq-wjl8i / sq-74oy4: the kind-first rank of the total order. Numeric
        // tracks the `as_num` membership (the same set the numeric arm compares), which is
        // now DATATYPE-AWARE (sq-74oy4): a lexical ill-formed FOR its datatype (e.g.
        // "1.5"^^xsd:integer) is NOT numeric here and classifies as Other (lexical order) —
        // matching `of_literal` and keeping the Numeric kind purely value-ordered (a kind
        // mixing value-ordered and type-error pairs is intransitive — the very bug this rank
        // removes; pre-sq-74oy4 the lenient `as_num` wrongly kept such a lexical Numeric).
        // A computed boolean (whose f64 view is 0.0/1.0) classifies as Boolean, keeping it in
        // one kind with boolean LITERALS (both order `false < true` via `strict_cmp`).
        match lit_kind(self) {
            LitKind::Bool(_) => LiteralKind::Boolean,
            LitKind::Num(_) if as_num(self).is_some() || beyond_tower_lexical(self).is_some() => {
                LiteralKind::Numeric
            }
            LitKind::Str(_) => LiteralKind::String,
            LitKind::Lang(..) => LiteralKind::Lang,
            LitKind::DateTime(Some(_)) => LiteralKind::DateTime,
            LitKind::Date(Some(_)) => LiteralKind::Date,
            // Ill-formed numerics/temporals, other-XSD, unknown datatypes (and the
            // unreachable non-literal case — `compare_terms` gates on the class).
            _ => LiteralKind::Other,
        }
    }
    #[inline]
    fn value_str(&self) -> Option<String> {
        value_str(self)
    }
    #[inline]
    fn as_f64(&self) -> Option<f64> {
        // A well-formed integer/decimal beyond the i128 tower has no `Num`, but it is still a
        // number: its correctly-rounded f64 (monotonic, possibly +-INF) orders it, and an f64
        // tie is rechecked exactly in `exact_cmp`.
        as_num(self).or_else(|| beyond_tower_lexical(self).and_then(parse_xsd_f64))
    }
    #[inline]
    fn exact_cmp(&self, other: &Self) -> Option<Ordering> {
        // sq-rikm7 / sq-wjl8i: f64-collapse recheck — when the
        // lenient `as_num` arm reports two numeric literals EQUAL, the pair rechecks
        // under `Num::cmp_total`, the EXACT-RATIONAL total order: exact via the decimal
        // tower for int/decimal pairs, exact against the double's exact decimal
        // expansion for the MIXED exact/inexact pair (the pre-fix `num_compare`
        // fallback kept the collapsed f64 verdict there, which made the order
        // intransitive at the 2^53 collapse — witness 1 of sq-wjl8i). The relational
        // `<`/`=` (`cmp_expr`, via `num_compare`) deliberately KEEPS its own semantics; this total
        // order refines only their ties. `None` (a lexical beyond the exact tower) keeps the tie.
        match (as_numeric(self), as_numeric(other)) {
            (Some(a), Some(b)) => {
                if !numeric_capacity::comparable(a, b) {
                    return None;
                }
                // Two in-tower decimals whose scale alignment overflows i128 would fall
                // back to their (equal) f64 images; compare their exact lexicals instead,
                // the same exact order a beyond-tower lexical gets below, so the total
                // order stays transitive across the tower boundary.
                if let (Some(x), Some(y)) = (a.to_dec(), b.to_dec()) {
                    if x.cmp(y).is_none() {
                        return cmp_decimal_str(&a.lexical(), &b.lexical());
                    }
                }
                Some(a.cmp_total(b))
            }
            // A strict numeric budget has already failed capacity on a lexical beyond the tower.
            _ if budget::strict_numeric() => None,
            // At least one side is beyond the i128 tower: compare exact decimal lexicals
            // (arbitrary precision), or an exact lexical against a float/double's value.
            _ => match (exact_decimal_lexical(self), exact_decimal_lexical(other)) {
                (Some(a), Some(b)) => cmp_decimal_str(&a, &b),
                (Some(a), None) => Some(cmp_exact_lex_f64(&a, as_num(other)?)),
                (None, Some(b)) => Some(cmp_exact_lex_f64(&b, as_num(self)?).reverse()),
                (None, None) => None,
            },
        }
    }
    #[inline]
    fn strict_cmp(&self, other: &Self) -> Option<Ordering> {
        // dateTime/date by timeline, same-tag / same-other-XSD lexically — when comparable.
        // sq-2k5py: `value_compare_strict` is the RELATIONAL comparison, so it
        // leaves the mixed-timezone window indeterminate; for the TOTAL order that `None`
        // would drop the pair to `compare_terms`' lexical fallback INSIDE the DateTime kind,
        // which is intransitive (`Timeline::cmp_tl_total`'s witness). Only on that `None` —
        // so the decided path costs nothing — re-decide a same-family temporal pair under
        // the total-order extension. Relational `<`/`>`/`=` keep the type error.
        value_compare_strict(self, other).or_else(|| temporal_total_cmp(self, other))
    }
    #[inline]
    fn triple_parts(&self) -> Option<[Self; 3]> {
        let Value::Term(Term::Triple(t)) = self else {
            return None;
        };
        // The subject (named-or-blank) and predicate (always an IRI) are lifted to `Value`s so
        // the generic recursion classifies + compares them by exactly the rules `compare_values`
        // applied inline: the subject by blank/IRI string, the predicate by IRI string (== its
        // `as_str().cmp(..)`), the object under the full order.
        let subject = Value::Term(match &t.subject {
            NamedOrBlankNode::NamedNode(n) => Term::NamedNode(n.clone()),
            NamedOrBlankNode::BlankNode(b) => Term::BlankNode(b.clone()),
        });
        let predicate = Value::Term(Term::NamedNode(t.predicate.clone()));
        let object = Value::Term(t.object.clone());
        Some([subject, predicate, object])
    }
}

/// The TOTAL order for ORDER BY (and the MIN/MAX fallback): SPARQL orders
/// unbound < blank nodes < IRIs < literals (< triple terms), then literals KIND-FIRST —
/// a fixed `LiteralKind` rank between literal kinds (numeric < boolean < dateTime <
/// date < string < language-tagged < other), value order only WITHIN a kind (numerics
/// by exact value with NaN first, dateTimes by timeline, else lexically).
///
/// sq-vezew: the algorithm lives in `sparq_substrate::compare::compare_terms`,
/// generic over the `CompareTerm` trait the engine implements for `Value` above — a thin
/// monomorphised call into the shared substrate. sq-wjl8i: the order is
/// deliberately NOT the pre-move lexical-fallback body — cross-kind lexical fallback,
/// collapsed mixed-tier f64 ties and NaN partiality made it intransitive (three
/// machine-checked witnesses; see the substrate module docs for what is spec-mandated
/// vs a documented extension). Relational `<` / `=` semantics are UNTOUCHED.
#[inline]
pub(super) fn compare_values(x: &Value, y: &Value) -> Option<Ordering> {
    if budget::strict_numeric() {
        if let (Some(a), Some(b)) = (as_numeric(x), as_numeric(y)) {
            return numeric_capacity::comparable(a, b).then(|| a.cmp_total(b));
        }
    }
    compare_terms(x, y)
}

/// SPARQL built-in function calls (`STR`, `LANG`, `CONCAT`, `SUBSTR`, type tests, numeric, …).
/// Unsupported functions (hashes, dateTime, REGEX, BNODE/RAND/UUID) return a clear error rather
/// than a silent wrong answer. SPARQL type errors map to `Value::Error` (EBV false; unbound on BIND).
///
/// Inner body of SPARQL function evaluation, generic over the per-argument evaluator `ev`.
/// Called both from [`eval_function`] (with `eval_expr` as evaluator) and from [`eval_compiled`]
/// (with `eval_compiled` as evaluator). sq-7d3dj.4.
pub(super) fn eval_function_inner<E: Fn(usize) -> Result<Value, String>>(
    f: &spargebra::algebra::Function,
    nargs: usize,
    ev: E,
) -> Result<Value, String> {
    use spargebra::algebra::Function as F;
    let simple = |s: String| Value::Term(Term::Literal(Literal::new_simple_literal(s)));
    // Both operands as ARGUMENT-COMPATIBLE string literals (second simple/xsd:string,
    // or same language tag as the first), else `Value::Error`.
    let str_compat2 = |a: &Value, c: &Value, g: &dyn Fn(&str, &str) -> bool| match (str_lit(a), str_lit(c)) {
        (Some((x, lx)), Some((y, ly))) if ly.is_none() || ly == lx => Value::Bool(g(&x, &y)),
        _ => Value::Error,
    };
    Ok(match f {
        // (sq-qeltv) STR is defined over LITERALS and IRIs only, per SPARQL 1.1
        // §17.4.2.5 (`simple literal STR(literal ltrl)` / `simple literal STR(IRI rsrc)`).
        // A blank-node operand is a TYPE ERROR — the old `value_str` coercion leaked the
        // ENGINE-INTERNAL bnode label (not a principled §17.3.1 extension: the label is
        // dictionary-private and unstable). Unbound / errored / triple-term operands stay
        // errors as before; computed numerics/booleans are literal values, so they pass.
        // Differential (bead): Oxigraph errors here too; rdflib leniently returns ITS
        // internal label — divergent-by-engine, exactly why the label must not leak.
        F::Str => match ev(0)? {
            Value::Term(Term::NamedNode(n)) => simple(n.as_str().to_string()),
            Value::Term(Term::Literal(l)) => simple(l.value().to_string()),
            Value::Num(n) => simple(n.lexical()),
            Value::Bool(b) => simple(b.to_string()),
            _ => Value::Error,
        },
        // STRLEN's operand is a STRING LITERAL (simple / lang-tagged /
        // xsd:string), per SPARQL 1.1 §17.4.3.2 `xsd:integer STRLEN(string literal)`.
        // An IRI / number / boolean is a TYPE ERROR — NOT its STR() length (the old
        // `value_str` coercion wrongly returned `STRLEN(<iri>)` = the IRI's length).
        F::StrLen => str_lit(&ev(0)?).map(|(s, _)| Value::Num(Num::Int(s.chars().count() as i64))).unwrap_or(Value::Error),
        // UCASE/LCASE/SUBSTR operate on string literals and preserve the language tag
        // of the argument (simple in, simple out; "bar"@en in, "BAR"@en out).
        F::UCase => match str_lit(&ev(0)?) {
            Some((s, lang)) => lit_with_lang(s.to_uppercase(), lang.as_deref()),
            None => Value::Error,
        },
        F::LCase => match str_lit(&ev(0)?) {
            Some((s, lang)) => lit_with_lang(s.to_lowercase(), lang.as_deref()),
            None => Value::Error,
        },
        F::Lang => match ev(0)? {
            Value::Term(Term::Literal(l)) => simple(l.language().unwrap_or("").to_string()),
            Value::Num(_) | Value::Bool(_) => simple(String::new()),
            _ => Value::Error,
        },
        F::Datatype => match ev(0)? {
            Value::Term(Term::Literal(l)) => Value::Term(Term::NamedNode(l.datatype().into_owned())),
            // A computed numeric knows its promoted XSD type (the type-promotion suite
            // checks `datatype(?l + ?r)` across the whole tower).
            Value::Num(n) => Value::Term(Term::NamedNode(n.datatype().into_owned())),
            Value::Bool(_) => Value::Term(Term::NamedNode(xsd::BOOLEAN.into_owned())),
            _ => Value::Error,
        },
        // CONCAT: all operands must be string literals. The result carries a language
        // tag only when every operand has the SAME tag; otherwise it is simple.
        F::Concat => {
            let mut s = String::new();
            let mut lang: Option<Option<String>> = None; // common-tag accumulator
            for i in 0..nargs {
                match str_lit(&ev(i)?) {
                    Some((p, l)) => {
                        s.push_str(&p);
                        lang = Some(match lang {
                            None => l,
                            Some(prev) if prev == l => prev,
                            Some(_) => None,
                        });
                    }
                    None => return Ok(Value::Error),
                }
            }
            lit_with_lang(s, lang.flatten().as_deref())
        }
        F::Contains => str_compat2(&ev(0)?, &ev(1)?, &|a, c| a.contains(c)),
        F::StrStarts => str_compat2(&ev(0)?, &ev(1)?, &|a, c| a.starts_with(c)),
        F::StrEnds => str_compat2(&ev(0)?, &ev(1)?, &|a, c| a.ends_with(c)),
        // STRBEFORE/STRAFTER: arguments must be compatible (second simple/xsd:string,
        // or same language tag). On a match the result carries the FIRST argument's
        // language tag; no match gives the empty simple literal.
        F::StrBefore => match (str_lit(&ev(0)?), str_lit(&ev(1)?)) {
            (Some((a, la)), Some((c, lc))) if lc.is_none() || lc == la => match a.find(&c) {
                Some(i) => lit_with_lang(a[..i].to_string(), la.as_deref()),
                None => simple(String::new()),
            },
            _ => Value::Error,
        },
        F::StrAfter => match (str_lit(&ev(0)?), str_lit(&ev(1)?)) {
            (Some((a, la)), Some((c, lc))) if lc.is_none() || lc == la => match a.find(&c) {
                Some(i) => lit_with_lang(a[i + c.len()..].to_string(), la.as_deref()),
                None => simple(String::new()),
            },
            _ => Value::Error,
        },
        F::SubStr => {
            let (s, lang) = match str_lit(&ev(0)?) {
                Some(x) => x,
                None => return Ok(Value::Error),
            };
            let start = match integer_argument(&ev(1)?) {
                Some(n) => n,
                None => return Ok(Value::Error),
            };
            // XPath positions satisfy start <= position < start+length.
            // Clipping the start before adding length incorrectly extends slices
            // that start before position one. Widen before addition to avoid overflow.
            let end = if nargs >= 3 {
                let len = match integer_argument(&ev(2)?) {
                    Some(n) => n.max(0),
                    None => return Ok(Value::Error),
                };
                start.saturating_add(len)
            } else {
                i128::MAX
            };
            let out = s.chars().enumerate().take_while(|(i, _)| (*i as i128 + 1) < end).filter_map(|(i, ch)| {
                let position = i as i128 + 1;
                (position >= start && position < end).then_some(ch)
            }).collect();
            lit_with_lang(out, lang.as_deref())
        }
        // ENCODE_FOR_URI's operand is a STRING LITERAL, per SPARQL 1.1
        // §17.4.3.12 `simple literal ENCODE_FOR_URI(string literal)`. A number / IRI /
        // boolean is a TYPE ERROR — the old `value_str` coercion wrongly encoded e.g.
        // `ENCODE_FOR_URI(123)` to the simple literal "123".
        F::EncodeForUri => str_lit(&ev(0)?).map(|(s, _)| simple(encode_for_uri(&s))).unwrap_or(Value::Error),
        F::Iri => match ev(0)? {
            // An IRI argument passes through unchanged.
            Value::Term(Term::NamedNode(n)) => Value::Term(Term::NamedNode(n)),
            v => match str_lit(&v) {
                // String literal: absolute IRIs pass; relative ones resolve against BASE.
                Some((s, None)) => resolve_iri(&s).map(|n| Value::Term(Term::NamedNode(n))).unwrap_or(Value::Error),
                _ => Value::Error,
            },
        },
        // (sq-qeltv) The type-test builtins take an RDF TERM: an unbound (or
        // already-errored) operand is a TYPE ERROR per SPARQL 1.1 §17.2 — so
        // `FILTER(!isIRI(?unbound))` DROPS the row (`!error` = error), where the old
        // `false` leniency kept it. Mirrors the `IsTriple` / `HasLang` arms below;
        // differential (bead): Oxigraph AND rdflib both drop the row.
        F::IsIri => match ev(0)? {
            Value::Unbound | Value::Error => Value::Error,
            v => Value::Bool(matches!(v, Value::Term(Term::NamedNode(_)))),
        },
        F::IsBlank => match ev(0)? {
            Value::Unbound | Value::Error => Value::Error,
            v => Value::Bool(matches!(v, Value::Term(Term::BlankNode(_)))),
        },
        F::IsLiteral => match ev(0)? {
            Value::Unbound | Value::Error => Value::Error,
            v => Value::Bool(matches!(v, Value::Term(Term::Literal(_)) | Value::Num(_) | Value::Bool(_))),
        },
        F::IsNumeric => match ev(0)? {
            Value::Unbound | Value::Error => Value::Error,
            Value::Num(_) => Value::Bool(true),
            Value::Term(Term::Literal(l)) => Value::Bool(sparq_core::numeric_literal_valid(l.value(), l.datatype().as_str())),
            _ => Value::Bool(false),
        },
        // ABS/CEIL/FLOOR/ROUND preserve the argument's numeric DATATYPE
        // (CEIL("2.5"^^xsd:decimal) is "3"^^xsd:decimal, not xsd:integer).
        F::Abs => as_numeric(&ev(0)?).and_then(|n| numeric_capacity::unary(n, Num::abs)).map(Value::Num).unwrap_or(Value::Error),
        F::Ceil => as_numeric(&ev(0)?).and_then(|n| numeric_capacity::unary(n, Num::ceil)).map(Value::Num).unwrap_or(Value::Error),
        F::Floor => as_numeric(&ev(0)?).and_then(|n| numeric_capacity::unary(n, Num::floor)).map(Value::Num).unwrap_or(Value::Error),
        F::Round => as_numeric(&ev(0)?).and_then(|n| numeric_capacity::unary(n, Num::round)).map(Value::Num).unwrap_or(Value::Error),
        // STRDT(lexical, datatypeIRI) -> typed literal. The first argument must be a
        // SIMPLE literal (= xsd:string in RDF 1.1) — lang-tagged / typed input errors.
        F::StrDt => match (str_lit(&ev(0)?), ev(1)?) {
            (Some((lex, None)), Value::Term(Term::NamedNode(dt))) => {
                budget::check_temporal(&lex, dt.as_str())?;
                Value::Term(Term::Literal(Literal::new_typed_literal(lex, dt)))
            }
            _ => Value::Error,
        },
        // STRLANG(lexical, langTag) -> language-tagged literal; both arguments must be
        // simple literals.
        F::StrLang => match (str_lit(&ev(0)?), str_lit(&ev(1)?)) {
            (Some((lex, None)), Some((lang, None))) => match Literal::new_language_tagged_literal(lex, lang) {
                Ok(l) => Value::Term(Term::Literal(l)),
                Err(_) => Value::Error,
            },
            _ => Value::Error,
        },
        // LANGMATCHES(tag, range) — RFC 4647 basic filtering (`*` matches any non-empty tag).
        // Both operands are STRING LITERALS, per SPARQL 1.1 §17.4.3.15
        // `xsd:boolean langMatches(simple literal, simple literal)`. A number / IRI /
        // boolean operand is a TYPE ERROR — the old `value_str` coercion wrongly
        // stringified e.g. `LANGMATCHES(123, "en")` to "123" and returned `false`
        // (a value) instead of an error.
        F::LangMatches => match (str_lit(&ev(0)?), str_lit(&ev(1)?)) {
            (Some((tag, _)), Some((range, _))) => {
                let (tag, range) = (tag.to_ascii_lowercase(), range.to_ascii_lowercase());
                let m = if range == "*" {
                    !tag.is_empty()
                } else {
                    tag == range || tag.starts_with(&format!("{range}-"))
                };
                Value::Bool(m)
            }
            _ => Value::Error,
        },
        // Hash builtins: operand must be a simple literal / xsd:string; lowercase hex out.
        #[cfg(feature = "digest")]
        F::Md5 => digest_hex::<md5::Md5>(&ev(0)?),
        #[cfg(feature = "digest")]
        F::Sha1 => digest_hex::<sha1::Sha1>(&ev(0)?),
        #[cfg(feature = "digest")]
        F::Sha256 => digest_hex::<sha2::Sha256>(&ev(0)?),
        #[cfg(feature = "digest")]
        F::Sha384 => digest_hex::<sha2::Sha384>(&ev(0)?),
        #[cfg(feature = "digest")]
        F::Sha512 => digest_hex::<sha2::Sha512>(&ev(0)?),
        // TZ(xsd:dateTime) -> the timezone part of the lexical form as a simple
        // literal ("Z", "±hh:mm", or "" when absent).
        F::Tz => datetime_arg_tz(&ev(0)?).map(simple).unwrap_or(Value::Error),
        // TIMEZONE(xsd:dateTime) -> xsd:dayTimeDuration; no timezone is a type error.
        F::Timezone => match datetime_arg_tz(&ev(0)?).as_deref().and_then(tz_to_duration) {
            Some(d) => Value::Term(Term::Literal(Literal::new_typed_literal(d, xsd::DAY_TIME_DURATION))),
            None => Value::Error,
        },
        F::BNode => {
            if nargs == 0 {
                // BNODE(): a fresh blank node per call.
                static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Value::Term(Term::BlankNode(BlankNode::new_unchecked(format!("fnb{n}"))))
            } else {
                // BNODE(str): the label is derived from (solution-row scope, argument),
                // so equal arguments within ONE solution map to the same blank node and
                // everything else stays distinct (see ROW_SCOPE).
                match string_literal(&ev(0)?) {
                    Some(s) => {
                        let (scope, idx) = ROW_SCOPE.get();
                        Value::Term(Term::BlankNode(BlankNode::new_unchecked(format!(
                            "fnb{scope:x}r{idx}h{:016x}",
                            fx64(&s)
                        ))))
                    }
                    None => Value::Error,
                }
            }
        }
        // ---- Builtin memoisation policy (sq-98w7z.1) ----------------------
        // ALLOW (memoised, keyed on the argument VALUES — pure, so a hit is
        //   result-identical to a fresh evaluation): the REGEX()/REPLACE() compiled
        //   `(pattern, flags)` via `regex_cache`, the one per-row builtin cost worth a
        //   memo (compile is µs-scale vs ns-scale match). Compile FAILURES are memoised
        //   too — an invalid pattern stays a per-row type error.
        // PINNED (not memoised — spec-MANDATED constancy): NOW() formats the
        //   execution-scoped `query_now` instant (SPARQL 1.1 §17.4.5.1); the next
        //   execution re-samples.
        // DENY (non-deterministic — MUST re-evaluate on every call; memoising any of
        //   these would silently collapse per-row freshness): RAND(), UUID(), STRUUID(),
        //   no-arg BNODE() (fresh label per call), and every REGISTERED custom function
        //   (`FunctionRegistry` makes no purity promise). The freshness-per-row test is
        //   `nondeterministic_builtins_fresh_per_call_never_memoised`.
        // Everything else (UCASE, STR, numeric ops, …) evaluates directly: value-keyed
        // memoisation of ns-scale pure builtins costs more in key hashing than it saves.
        //
        // NOW(): the execution's pinned instant as xsd:dateTime. RAND(): xsd:double in
        // [0, 1) from a per-thread splitmix64 seeded once from the OS RNG (see
        // `rand_unit`) — both native-only for the same reason as UUID()/STRUUID().
        #[cfg(all(not(target_arch = "wasm32"), not(target_os = "zkvm")))]
        F::Now => Value::Term(Term::Literal(Literal::new_typed_literal(now_lexical(), xsd::DATE_TIME))),
        #[cfg(all(not(target_arch = "wasm32"), not(target_os = "zkvm")))]
        F::Rand => Value::Num(Num::Double(rand_unit::next())),
        #[cfg(all(not(target_arch = "wasm32"), not(target_os = "zkvm")))]
        F::Uuid => Value::Term(Term::NamedNode(oxrdf::NamedNode::new_unchecked(format!(
            "urn:uuid:{}",
            uuid::Uuid::new_v4()
        )))),
        #[cfg(all(not(target_arch = "wasm32"), not(target_os = "zkvm")))]
        F::StrUuid => simple(uuid::Uuid::new_v4().to_string()),
        // xsd:dateTime accessors — parse the lexical form and return the numeric component.
        F::Year => datetime_field(&ev(0)?, 0),
        F::Month => datetime_field(&ev(0)?, 1),
        F::Day => datetime_field(&ev(0)?, 2),
        F::Hours => datetime_field(&ev(0)?, 3),
        F::Minutes => datetime_field(&ev(0)?, 4),
        F::Seconds => datetime_field(&ev(0)?, 5),
        // REGEX/REPLACE: the text operand must be a string literal (an IRI or non-string
        // literal is a type error); REPLACE's result keeps the text's language tag.
        #[cfg(feature = "regex")]
        F::Regex => {
            let (text, pat) = match (str_lit(&ev(0)?), value_str(&ev(1)?)) {
                (Some((t, _)), Some(p)) => (t, p),
                _ => return Ok(Value::Error),
            };
            let flags = if nargs >= 3 { value_str(&ev(2)?).unwrap_or_default() } else { String::new() };
            match regex_cache::get(&pat, &flags) {
                Some(re) => Value::Bool(re.is_match(&text)),
                None => Value::Error,
            }
        }
        #[cfg(feature = "regex")]
        F::Replace => {
            let (text, lang, pat, rep) = match (str_lit(&ev(0)?), value_str(&ev(1)?), value_str(&ev(2)?)) {
                (Some((t, lang)), Some(p), Some(r)) => (t, lang, p, r),
                _ => return Ok(Value::Error),
            };
            let flags = if nargs >= 4 { value_str(&ev(3)?).unwrap_or_default() } else { String::new() };
            match regex_cache::get(&pat, &flags) {
                Some(re) => lit_with_lang(re.replace_all(&text, rep.as_str()).into_owned(), lang.as_deref()),
                None => Value::Error,
            }
        }
        // ---- SPARQL 1.2 triple-term builtins ------------------------------------
        // TRIPLE(s, p, o): s must be an IRI / blank node, p an IRI, o any RDF term.
        F::Triple => {
            let subject = match ev(0)? {
                Value::Term(Term::NamedNode(n)) => NamedOrBlankNode::NamedNode(n),
                Value::Term(Term::BlankNode(b)) => NamedOrBlankNode::BlankNode(b),
                _ => return Ok(Value::Error),
            };
            let predicate = match ev(1)? {
                Value::Term(Term::NamedNode(n)) => n,
                _ => return Ok(Value::Error),
            };
            let Some(object) = value_as_term(&ev(2)?) else {
                return Ok(Value::Error);
            };
            Value::Term(Term::Triple(Box::new(oxrdf::Triple::new(subject, predicate, object))))
        }
        F::IsTriple => match ev(0)? {
            Value::Term(Term::Triple(_)) => Value::Bool(true),
            Value::Unbound | Value::Error => Value::Error,
            _ => Value::Bool(false),
        },
        F::Subject => match ev(0)? {
            Value::Term(Term::Triple(t)) => Value::Term(match t.subject {
                NamedOrBlankNode::NamedNode(n) => Term::NamedNode(n),
                NamedOrBlankNode::BlankNode(b) => Term::BlankNode(b),
            }),
            _ => Value::Error,
        },
        F::Predicate => match ev(0)? {
            Value::Term(Term::Triple(t)) => Value::Term(Term::NamedNode(t.predicate)),
            _ => Value::Error,
        },
        F::Object => match ev(0)? {
            Value::Term(Term::Triple(t)) => Value::Term(t.object),
            _ => Value::Error,
        },
        // ---- SPARQL 1.2 language / base-direction builtins -----------------------
        // hasLANG / hasLANGDIR: a boolean property of any RDF TERM (an IRI is simply
        // `false`); only unbound/error operands propagate the error.
        F::HasLang => match ev(0)? {
            Value::Term(Term::Literal(l)) => Value::Bool(l.language().is_some()),
            Value::Unbound | Value::Error => Value::Error,
            _ => Value::Bool(false),
        },
        F::HasLangDir => match ev(0)? {
            Value::Term(Term::Literal(l)) => Value::Bool(l.direction().is_some()),
            Value::Unbound | Value::Error => Value::Error,
            _ => Value::Bool(false),
        },
        // LANGDIR mirrors LANG: "" for a literal without a base direction, type error
        // on non-literals.
        F::LangDir => match ev(0)? {
            Value::Term(Term::Literal(l)) => simple(l.direction().map(|d| d.to_string()).unwrap_or_default()),
            Value::Num(_) | Value::Bool(_) => simple(String::new()),
            _ => Value::Error,
        },
        // STRLANGDIR(lexical, langTag, "ltr"|"rtl") -> directional language-tagged
        // literal; all three must be simple literals, the tag non-empty and valid,
        // the direction exactly lowercase "ltr"/"rtl".
        F::StrLangDir => match (str_lit(&ev(0)?), str_lit(&ev(1)?), str_lit(&ev(2)?)) {
            (Some((lex, None)), Some((lang, None)), Some((dir, None))) => {
                let dir = match dir.as_str() {
                    "ltr" => oxrdf::BaseDirection::Ltr,
                    "rtl" => oxrdf::BaseDirection::Rtl,
                    _ => return Ok(Value::Error),
                };
                match Literal::new_directional_language_tagged_literal(lex, lang, dir) {
                    Ok(l) => Value::Term(Term::Literal(l)),
                    Err(_) => Value::Error,
                }
            }
            _ => Value::Error,
        },
        // XSD constructor casts: xsd:integer(?x), xsd:decimal(?x), … (SPARQL 17.5),
        // then the installed extension-function registry (SPARQL 17.6; see
        // `query_with_functions` / `with_functions` in lib.rs). An IRI that is
        // neither stays the same hard query error as before the registry existed.
        F::Custom(nn) => {
            // (sq-v411r, survey §B2) MULTIPLICITY(): the SPARQL 1.2 algebra's
            // `multiplicity` device, parsed as the reserved-IRI `Function::Custom` so the
            // shared enum stays byte-compatible downstream. Inside an aggregate argument it
            // is the bag cardinality (`xsd:integer`) of the current group member's solution
            // within its group multiset, so `SUM(?x * MULTIPLICITY())` is the multiset-
            // weighted sum. The evaluator installs the per-member value before each
            // `eval_expr`; OUTSIDE an aggregate the context is absent and the call is an
            // expression error (only meaningful over a multiset). Checked BEFORE the cast /
            // registry so the reserved IRI can never be shadowed. NOT a W3C-standard
            // callable builtin — see the README.
            if nargs == 0 && nn.as_str() == MULTIPLICITY_FN_IRI {
                return Ok(match multiplicity::current() {
                    Some(card) => Value::Num(Num::Int(card as i64)),
                    None => Value::Error,
                });
            }
            let mut vals = Vec::with_capacity(nargs);
            for i in 0..nargs {
                vals.push(ev(i)?);
            }
            if vals.len() == 1 {
                if let Value::Term(Term::Literal(literal)) = &vals[0] {
                    // Capacity applies to the constructed lexical after the
                    // string-cast preprocessing, while raw typed terms stay strict.
                    let lexical = if literal.datatype() == xsd::STRING && nn.as_str() == xsd::DATE_TIME.as_str() {
                        literal.value().trim_matches([' ', '\t', '\r', '\n'])
                    } else { literal.value() };
                    budget::check_temporal(lexical, nn.as_str())?;
                }
                if let Some(out) = eval_cast(nn.as_str(), &vals[0]) {
                    return Ok(out);
                }
            }
            if let Some(f) = functions::lookup(nn.as_str()) {
                // Arguments are materialised as concrete RDF terms; an unbound or
                // errored argument is an expression ERROR (row filtered / BIND
                // unbound), exactly like the builtins. The extension returning
                // `Err` (wrong arity, bad lexical, …) is the same expression
                // error — per-row, never a hard query error.
                let mut terms = Vec::with_capacity(vals.len());
                for v in &vals {
                    match value_as_term(v) {
                        Some(t) => terms.push(t),
                        None => return Ok(Value::Error),
                    }
                }
                return Ok(match f(&terms) {
                    Ok(t) => {
                        if let Term::Literal(literal) = &t {
                            budget::check_temporal(literal.value(), literal.datatype().as_str())?;
                        }
                        Value::Term(t)
                    }
                    Err(_) => Value::Error,
                });
            }
            return Err(format!("unsupported SPARQL function: Custom({})", nn.as_str()));
        }
        // With every default feature on, the arms above are exhaustive (the
        // `F::Custom` arm is no longer guarded on arity); this arm is reached
        // only when the feature-gated builtins (regex / digest / native-only
        // UUID) are compiled out — i.e. the wasm build.
        #[allow(unreachable_patterns)]
        other => return Err(format!("unsupported SPARQL function: {other:?}")),
    })
}

/// Thin wrapper — the hot inner body lives in [`eval_function_inner`]. sq-7d3dj.4.
pub(super) fn eval_function(
    graph: &Graph,
    local: &LocalVocab,
    b: &Bindings,
    row: &[Id],
    f: &spargebra::algebra::Function,
    args: &[Expression],
) -> Result<Value, String> {
    eval_function_inner(f, args.len(), |i| eval_expr(graph, local, b, row, &args[i]))
}

// ── Compiled comparison / arithmetic helpers (sq-7d3dj.4) ───────────────────────────────
// Mirrors of `cmp_expr` / `equal_expr` / `arith` for [`CompiledExpr`].
// All `Var` accesses are pre-resolved column indices; no `b.col(v)` per row.
// sq-7d3dj.4

pub(super) fn cmp_compiled(
    graph: &Graph,
    local: &LocalVocab,
    b: &Bindings,
    row: &[Id],
    a: &CompiledExpr,
    c: &CompiledExpr,
    f: impl Fn(Ordering) -> bool,
) -> Result<Value, String> {
    if budget::strict_numeric() {
        let (x, y) = (eval_compiled(graph, local, b, row, a)?, eval_compiled(graph, local, b, row, c)?);
        if let (Some(a), Some(b)) = (as_numeric(&x), as_numeric(&y)) {
            if a.is_nan() || b.is_nan() { return Ok(Value::Bool(false)); }
        }
        return Ok(value_compare_strict(&x, &y).map(|o| Value::Bool(f(o))).unwrap_or(Value::Error));
    }
    let arith = compiled_expr_has_arith(a) || compiled_expr_has_arith(c);
    if arith {
        if let (Some(da), Some(db)) =
            (eval_compiled_dec(graph, local, row, a), eval_compiled_dec(graph, local, row, c))
        {
            if let Some(o) = da.cmp(db) {
                return Ok(Value::Bool(f(o)));
            }
        }
    }
    // The f64 fast path below evaluates arithmetic UNTYPED (always in f64), but XPath evaluates
    // it in the promoted tier: `"16777217"^^xsd:float + 1` is a FLOAT and rounds to 16777216.
    // Arithmetic the exact path above did not decide (a float/double operand) takes the typed
    // evaluator instead.
    let fast = if arith {
        None
    } else {
        eval_compiled_numeric(graph, local, row, a).zip(eval_compiled_numeric(graph, local, row, c))
    };
    if let Some((x, y)) = fast {
        if x == y {
            if let (Some(la), Some(lb)) = (
                eval_compiled_exact_lexical(graph, local, row, a),
                eval_compiled_exact_lexical(graph, local, row, c),
            ) {
                if let Some(ord) = cmp_decimal_str(&la, &lb) {
                    return Ok(Value::Bool(f(ord)));
                }
            }
        }
        // An unequal pair that `xs:float` promotion could tie takes the typed path below.
        if x == y || !f32_promotion_may_tie(x, y) {
            return Ok(Value::Bool(x.partial_cmp(&y).map(&f).unwrap_or(false)));
        }
    }
    // Mirrors `cmp_expr`: cached instants decide far-apart same-family pairs.
    if let (Some(va), Some(vb)) =
        (eval_compiled_approx_temporal(graph, row, a), eval_compiled_approx_temporal(graph, row, c))
    {
        if va.kind == vb.kind {
            if let Some(o) = approx_temporal_order(va, vb) {
                return Ok(Value::Bool(f(o)));
            }
        }
    }
    if let (Some(ta), Some(tb)) =
        (eval_compiled_temporal(graph, local, row, a), eval_compiled_temporal(graph, local, row, c))
    {
        return Ok(match ExactTemporal::compare(ta, tb) {
            Some(o) => Value::Bool(f(o)),
            None => Value::Error,
        });
    }
    let (x, y) = (eval_compiled(graph, local, b, row, a)?, eval_compiled(graph, local, b, row, c)?);
    Ok(relational_value(&x, &y, arith, f))
}

/// (sq-7d3dj.30.11) The single RAW ID an operand resolves to, if it is a bound column
/// or a constant IRI — else `None` (arithmetic, function call, literal cast, unbound column).
/// A constant IRI absent from the dictionary yields `NO_ID`, which never equals a bound id.
#[cfg(feature = "id-filter-fastpath")]
#[inline]
pub(super) fn operand_single_id(graph: &Graph, row: &[Id], e: &CompiledExpr) -> Option<Id> {
    match e {
        CompiledExpr::Var(Some(c)) => Some(row[*c]),
        CompiledExpr::NamedNode(n) => Some(graph.id_of(&Term::NamedNode(n.clone())).unwrap_or(NO_ID)),
        _ => None,
    }
}

/// (sq-7d3dj.30.11) Evaluates `=` between two STATICALLY NON-LITERAL operands by raw-id
/// (in)equality. Both operands are IRI/bnode (or absent-const → `NO_ID`); the canonicalising dict
/// gives each such term one id, so `=` is id equality — result-identical to the general path (which
/// decides IRI/bnode pairs by `p == q` term identity). An UNBOUND operand (`NO_ID` FROM A COLUMN)
/// is a type error, exactly as the exact path treats an unbound variable. A constant-IRI operand is
/// never unbound; its `NO_ID` (IRI absent from the dict) legitimately means "never equal".
#[cfg(feature = "id-filter-fastpath")]
#[inline]
pub(super) fn equal_idfast(graph: &Graph, row: &[Id], a: &IdOperand, c: &IdOperand) -> Value {
    let resolve = |op: &IdOperand| -> (Id, bool) {
        match op {
            // A column can be unbound (NO_ID); that is a type error in `=`.
            IdOperand::Col(col) => (row[*col], true),
            // A constant IRI is always "bound"; NO_ID here means the IRI is not in the dict.
            IdOperand::ConstIri(n) => (graph.id_of(&Term::NamedNode(n.clone())).unwrap_or(NO_ID), false),
        }
    };
    let (ida, from_col_a) = resolve(a);
    let (idc, from_col_c) = resolve(c);
    if (from_col_a && ida == NO_ID) || (from_col_c && idc == NO_ID) {
        return Value::Error; // unbound operand -> type error (matches the exact path)
    }
    Value::Bool(ida == idc)
}

pub(super) fn equal_compiled(
    graph: &Graph,
    local: &LocalVocab,
    b: &Bindings,
    row: &[Id],
    a: &CompiledExpr,
    c: &CompiledExpr,
) -> Result<Value, String> {
    if budget::strict_numeric() {
        let (x, y) = (eval_compiled(graph, local, b, row, a)?, eval_compiled(graph, local, b, row, c)?);
        return Ok(values_equal(&x, &y).map(Value::Bool).unwrap_or(Value::Error));
    }
    // (sq-7d3dj.30.11) Fast path (b): EQUAL ids of ANY kind are the SAME term (the
    // canonicalising dict gives each term one id), so `=` is `true` — mirroring the `p == q`
    // sameTerm short-circuit `values_equal` takes today, which is safe even for ill-typed
    // literals (sameTerm returns a boolean, never a type error). Requires both operands to be
    // single-id terms and BOTH bound (an unbound `NO_ID` column would be a type error, not
    // equal). UNEQUAL ids fall through to the exact path unchanged — crucially, unequal ids of
    // value-equal literals (`"1"^^integer` = `"1.0"^^decimal`, sq-lr2ii) must NOT be decided here.
    // An active temporal year range must still see both operands, so it takes the exact path.
    #[cfg(feature = "id-filter-fastpath")]
    if !budget::temporal_capacity_active() {
        if let (Some(ida), Some(idc)) = (operand_single_id(graph, row, a), operand_single_id(graph, row, c)) {
            if ida != NO_ID && ida == idc && !id_is_nan_literal(graph, local, ida) {
                return Ok(Value::Bool(true));
            }
        }
    }
    let arith = compiled_expr_has_arith(a) || compiled_expr_has_arith(c);
    if arith {
        if let (Some(da), Some(db)) =
            (eval_compiled_dec(graph, local, row, a), eval_compiled_dec(graph, local, row, c))
        {
            if let Some(o) = da.cmp(db) {
                return Ok(Value::Bool(o == Ordering::Equal));
            }
        }
    }
    // The f64 fast path below evaluates arithmetic UNTYPED (always in f64), but XPath evaluates
    // it in the promoted tier: `"16777217"^^xsd:float + 1` is a FLOAT and rounds to 16777216.
    // Arithmetic the exact path above did not decide (a float/double operand) takes the typed
    // evaluator instead.
    let fast = if arith {
        None
    } else {
        eval_compiled_numeric(graph, local, row, a).zip(eval_compiled_numeric(graph, local, row, c))
    };
    if let Some((x, y)) = fast {
        if x == y {
            if let (Some(la), Some(lb)) = (
                eval_compiled_exact_lexical(graph, local, row, a),
                eval_compiled_exact_lexical(graph, local, row, c),
            ) {
                if let Some(ord) = cmp_decimal_str(&la, &lb) {
                    return Ok(Value::Bool(ord == Ordering::Equal));
                }
            }
        }
        // An unequal pair that `xs:float` promotion could tie takes the typed path below.
        if x == y || !f32_promotion_may_tie(x, y) {
            return Ok(Value::Bool(x == y));
        }
    }
    if let (Some(ta), Some(tb)) =
        (eval_compiled_temporal(graph, local, row, a), eval_compiled_temporal(graph, local, row, c))
    {
        if ta.kind != tb.kind {
            return Ok(Value::Bool(false));
        }
        return Ok(match ExactTemporal::compare(ta, tb) {
            Some(o) => Value::Bool(o == Ordering::Equal),
            None => Value::Error,
        });
    }
    let (x, y) = (eval_compiled(graph, local, b, row, a)?, eval_compiled(graph, local, b, row, c)?);
    Ok(match values_equal(&x, &y) {
        Some(eq) => Value::Bool(eq),
        None => Value::Error,
    })
}

pub(super) fn arith_compiled(
    graph: &Graph,
    local: &LocalVocab,
    b: &Bindings,
    row: &[Id],
    a: &CompiledExpr,
    c: &CompiledExpr,
    op: ArithOp,
) -> Result<Value, String> {
    let (x, y) = (eval_compiled(graph, local, b, row, a)?, eval_compiled(graph, local, b, row, c)?);
    Ok(match (as_numeric(&x), as_numeric(&y)) {
        (Some(p), Some(q)) => numeric_capacity::binop(p, q, op).map(Value::Num).unwrap_or(Value::Error),
        _ => Value::Error,
    })
}

/// Per-row expression evaluator with all `Variable`/`Bound` nodes pre-resolved to column
/// indices. Call [`compile_expr`] once before the row loop; `b` is passed only for the
/// `EXISTS` arm (rare). sq-7d3dj.4.
pub(super) fn eval_compiled(
    graph: &Graph,
    local: &LocalVocab,
    b: &Bindings,
    row: &[Id],
    e: &CompiledExpr,
) -> Result<Value, String> {
    use CompiledExpr::*;
    match e {
        Var(col) => match col {
            Some(c) if row[*c] != NO_ID => checked_term_value(term_of(graph, local, row[*c]).unwrap()),
            _ => Ok(Value::Unbound),
        },
        Captured(term) => checked_term_value(term.clone()),
        CapturedBound => Ok(Value::Bool(true)),
        BoundCol(col) => Ok(Value::Bool(col.map(|c| row[c] != NO_ID).unwrap_or(false))),
        NamedNode(n) => Ok(Value::Term(Term::NamedNode(n.clone()))),
        Literal(l, _) => {
            budget::check_temporal(l.value(), l.datatype().as_str())?;
            Ok(Value::Term(Term::Literal(l.clone())))
        },
        And(a, c) => {
            let x = ebv3(&eval_compiled(graph, local, b, row, a)?, local.ebv_semantics);
            if x == Some(false) {
                return Ok(Value::Bool(false));
            }
            let y = ebv3(&eval_compiled(graph, local, b, row, c)?, local.ebv_semantics);
            Ok(and3(x, y))
        }
        Or(a, c) => {
            let x = ebv3(&eval_compiled(graph, local, b, row, a)?, local.ebv_semantics);
            if x == Some(true) {
                return Ok(Value::Bool(true));
            }
            let y = ebv3(&eval_compiled(graph, local, b, row, c)?, local.ebv_semantics);
            Ok(or3(x, y))
        }
        Not(a) => Ok(match ebv3(&eval_compiled(graph, local, b, row, a)?, local.ebv_semantics) {
            Some(v) => Value::Bool(!v),
            None => Value::Error,
        }),
        Equal(a, c) => equal_compiled(graph, local, b, row, a, c),
        #[cfg(feature = "id-filter-fastpath")]
        IdEqNonLit(a, c) => Ok(equal_idfast(graph, row, a, c)),
        SameTerm(a, c) => {
            let (x, y) = (eval_compiled(graph, local, b, row, a)?, eval_compiled(graph, local, b, row, c)?);
            Ok(same_term_value(&x, &y))
        }
        Greater(a, c) => cmp_compiled(graph, local, b, row, a, c, |o| o == Ordering::Greater),
        GreaterOrEqual(a, c) => cmp_compiled(graph, local, b, row, a, c, |o| o != Ordering::Less),
        Less(a, c) => cmp_compiled(graph, local, b, row, a, c, |o| o == Ordering::Less),
        LessOrEqual(a, c) => cmp_compiled(graph, local, b, row, a, c, |o| o != Ordering::Greater),
        Add(a, c) => arith_compiled(graph, local, b, row, a, c, ArithOp::Add),
        Subtract(a, c) => arith_compiled(graph, local, b, row, a, c, ArithOp::Sub),
        Multiply(a, c) => arith_compiled(graph, local, b, row, a, c, ArithOp::Mul),
        Divide(a, c) => arith_compiled(graph, local, b, row, a, c, ArithOp::Div),
        UnaryPlus(a) => Ok(unary_plus(eval_compiled(graph, local, b, row, a)?)),
        UnaryMinus(a) => {
            let v = eval_compiled(graph, local, b, row, a)?;
            Ok(as_numeric(&v).and_then(|n| numeric_capacity::unary(n, Num::neg)).map(Value::Num).unwrap_or(Value::Error))
        }
        If(cond, t, f) => match ebv3(&eval_compiled(graph, local, b, row, cond)?, local.ebv_semantics) {
            Some(true) => eval_compiled(graph, local, b, row, t),
            Some(false) => eval_compiled(graph, local, b, row, f),
            None => Ok(Value::Error),
        },
        Coalesce(es) => {
            for ce in es {
                let v = eval_compiled(graph, local, b, row, ce)?;
                if !matches!(v, Value::Unbound | Value::Error) {
                    return Ok(v);
                }
            }
            Ok(Value::Unbound)
        }
        In(a, list) => {
            let x = eval_compiled(graph, local, b, row, a)?;
            let mut errored = false;
            for c in list {
                let y = eval_compiled(graph, local, b, row, c)?;
                match values_equal(&x, &y) {
                    Some(true) => return Ok(Value::Bool(true)),
                    Some(false) => {}
                    None => errored = true,
                }
            }
            Ok(if errored { Value::Error } else { Value::Bool(false) })
        }
        FunctionCall(f, args) => {
            eval_function_inner(f, args.len(), |i| eval_compiled(graph, local, b, row, &args[i]))
        }
        Exists(inner) => Ok(Value::Bool(eval_exists(graph, local, b, row, inner)?)),
    }
}

/// Plain decimal form of an f64 with at least one fraction digit ("0.0", "1.0",
/// "1.25") — the form the W3C cast expected-results use for float/double sources.
pub(super) fn plain_min1(f: f64) -> String {
    if f.fract() == 0.0 && f.abs() < 1e15 {
        format!("{:.1}", f)
    } else {
        format!("{f}")
    }
}

/// A decimal lexical with trailing fraction zeros trimmed but AT LEAST one fraction
/// digit kept ("33.3300" -> "33.33", "0" -> "0.0") — the xsd:decimal cast convention.
pub(super) fn dec_trim_min1(d: Dec) -> String {
    let mut d = d;
    while d.scale > 1 && d.mant % 10 == 0 {
        d.mant /= 10;
        d.scale -= 1;
    }
    if d.scale == 0 {
        // Adding a lexical fraction digit must not overflow the numeric mantissa.
        return format!("{}.0", d.mant);
    }
    d.lexical()
}

/// Trim ALL trailing fraction zeros ("0.0" -> "0", "2.50" -> "2.5") — the xsd:string
/// cast convention for decimal sources.
pub(super) fn dec_trim(d: Dec) -> String {
    let mut d = d;
    while d.scale > 0 && d.mant % 10 == 0 {
        d.mant /= 10;
        d.scale -= 1;
    }
    d.lexical()
}

/// The XSD constructor-cast table. `None` when `target` is not a recognised cast IRI
/// (the caller reports an unsupported function); `Some(Value::Error)` for a cast that
/// fails per XPath (invalid source lexical, wrong source type, NaN/INF to exact types).
/// The result LEXICAL forms follow the conventions of the W3C cast test expected
/// results (which track the reference implementations), varying by source type.
pub(super) fn eval_cast(target: &str, v: &Value) -> Option<Value> {
    // The source as a STRING lexical only when it is a simple/xsd:string literal
    // (language-tagged literals and non-string types are NOT castable as strings).
    let src_str = || match v {
        Value::Term(Term::Literal(l)) if l.language().is_none() && l.datatype() == xsd::STRING => {
            // XSD whitespace collapse excludes Unicode spaces such as NBSP.
            Some(l.value().trim_matches([' ', '\t', '\r', '\n']).to_string())
        }
        _ => None,
    };
    let typed = |lex: String, dt: oxrdf::NamedNodeRef<'_>| Value::Term(Term::Literal(Literal::new_typed_literal(lex, dt)));
    // Classify a numeric SOURCE by its tower type (computed value or literal).
    let src_num = || as_numeric(v).filter(|_| !matches!(v, Value::Bool(_)));
    if target == xsd::STRING.as_str() {
        // STR semantics with VALUE canonicalisation for typed sources: booleans print
        // true/false, decimals trim trailing zeros ("0.0" -> "0"), float/double print
        // plain when integral ("0E1" -> "0") — everything else keeps its lexical.
        if let Some(b) = as_bool_val(v) {
            return Some(typed(b.to_string(), xsd::STRING));
        }
        if let Some(n) = src_num() {
            let s = match n {
                Num::Int(i) => i.to_string(),
                Num::Dec(d) => dec_trim(d),
                // plain shortest decimal form, never scientific ("0E1" -> "0",
                // "1.25"^^xsd:float -> "1.25")
                Num::Float(f) if f.is_finite() => format!("{f}"),
                Num::Double(f) if f.is_finite() => format!("{f}"),
                other => other.lexical(),
            };
            return Some(typed(s, xsd::STRING));
        }
        return Some(match v {
            Value::Term(Term::BlankNode(_)) | Value::Term(Term::Triple(_)) | Value::Unbound | Value::Error => Value::Error,
            other => value_str(other).map(|s| typed(s, xsd::STRING)).unwrap_or(Value::Error),
        });
    }
    if target == xsd::BOOLEAN.as_str() {
        if let Some(b) = as_bool_val(v) {
            return Some(Value::Bool(b));
        }
        if let Some(n) = as_numeric(v) {
            return Some(Value::Bool(!n.is_zero() && !n.is_nan()));
        }
        return Some(match src_str().as_deref() {
            Some("true") | Some("1") => Value::Bool(true),
            Some("false") | Some("0") => Value::Bool(false),
            _ => Value::Error,
        });
    }
    if target == xsd::DATE_TIME.as_str() {
        return Some(match v {
            Value::Term(Term::Literal(l))
                if (l.datatype() == xsd::DATE_TIME || l.datatype() == xsd::DATE_TIME_STAMP)
                    && temporal_of_lit(l).is_some() =>
            {
                typed(l.value().to_string(), xsd::DATE_TIME)
            }
            _ => match src_str() {
                Some(s) if parse_datetime(&s).is_some() => typed(s, xsd::DATE_TIME),
                _ => Value::Error,
            },
        });
    }
    let is_int = target == xsd::INTEGER.as_str();
    let is_dec = target == xsd::DECIMAL.as_str();
    let is_flt = target == xsd::FLOAT.as_str();
    let is_dbl = target == xsd::DOUBLE.as_str();
    if !(is_int || is_dec || is_flt || is_dbl) {
        return None; // not a cast IRI this engine knows
    }
    if is_int {
        // Truncate toward zero; strings must be valid xsd:integer lexicals.
        if let Some(b) = as_bool_val(v) {
            return Some(Value::Num(Num::Int(b as i64)));
        }
        if let Some(n) = src_num() {
            return Some(match n {
                Num::Int(i) => Value::Num(Num::Int(i)),
                Num::Dec(d) => {
                    // An unrepresentable power means |value| < 1, so
                    // truncation is zero. Reject an out-of-range integer instead
                    // of wrapping its i128 mantissa through an `as i64` cast.
                    let integer = 10i128.checked_pow(d.scale).map_or(0, |p| d.mant / p);
                    numeric_capacity::representable(true, i64::try_from(integer).ok())
                        .map(|i| Value::Num(Num::Int(i))).unwrap_or(Value::Error)
                },
                Num::Float(_) | Num::Double(_) => {
                    let f = n.f64();
                    // i64::MAX rounds up to 2^63 in f64: use an exclusive
                    // positive bound and inclusive negative bound.
                    let lower = i64::MIN as f64;
                    if (lower..-lower).contains(&f) {
                        Value::Num(Num::Int(f.trunc() as i64))
                    } else {
                        let _ = numeric_capacity::representable::<()>(f.is_finite(), None);
                        Value::Error
                    }
                }
            });
        }
        return Some(src_str().and_then(|s| numeric_capacity::representable(
            sparq_core::numeric_literal_valid(&s, xsd::INTEGER.as_str()), s.parse::<i64>().ok()))
            .map(|i| Value::Num(Num::Int(i))).unwrap_or(Value::Error));
    }
    if is_dec {
        if let Some(b) = as_bool_val(v) {
            return Some(typed(if b { "1.0" } else { "0.0" }.to_string(), xsd::DECIMAL));
        }
        if let Some(n) = src_num() {
            return Some(match n {
                // integer -> N.0 (zero prints bare, per the reference results)
                Num::Int(0) => typed("0".to_string(), xsd::DECIMAL),
                Num::Int(i) => typed(format!("{i}.0"), xsd::DECIMAL),
                // decimal -> decimal keeps its lexical
                Num::Dec(d) => typed(d.lexical(), xsd::DECIMAL),
                Num::Float(_) | Num::Double(_) => {
                    let f = n.f64();
                    if f.is_finite() {
                        typed(plain_min1(f), xsd::DECIMAL)
                    } else {
                        Value::Error
                    }
                }
            });
        }
        // string -> parse (no exponent allowed), trim trailing zeros, keep >= 1
        // fraction digit ("+33.3300" -> "33.33", "0" -> "0.0").
        return Some(
            src_str()
                .and_then(|s| numeric_capacity::decimal_cast(&s))
                .map(|d| typed(dec_trim_min1(d), xsd::DECIMAL))
                .unwrap_or(Value::Error),
        );
    }
    // float / double
    let dt = if is_flt { xsd::FLOAT } else { xsd::DOUBLE };
    if let Some(b) = as_bool_val(v) {
        return Some(typed(if b { "1.0E0" } else { "0E0" }.to_string(), dt));
    }
    if let Some(n) = src_num() {
        return Some(match n {
            // integer -> N.0 (zero prints bare)
            Num::Int(0) => typed("0".to_string(), dt),
            Num::Int(i) => typed(format!("{i}.0"), dt),
            // decimal -> keeps its lexical
            Num::Dec(d) => typed(d.lexical(), dt),
            // float/double -> plain decimal form with >= 1 fraction digit
            Num::Float(f) => typed(plain_min1(f as f64), dt),
            Num::Double(f) => typed(plain_min1(f), dt),
        });
    }
    Some(match src_str() {
        Some(s) => {
            // A valid integer lexical keeps its form verbatim ("13" -> "13"^^xsd:double);
            // anything else parses and serialises in canonical scientific form.
            if s.parse::<i64>().is_ok() {
                typed(s, dt)
            } else if is_flt {
                match parse_xsd_f32(&s) {
                    Some(f) if !f.is_nan() || s == "NaN" => typed(if f.is_finite() { format!("{f:E}") } else { Num::Float(f).lexical() }, dt),
                    _ => Value::Error,
                }
            } else {
                match parse_xsd_f64(&s) {
                    Some(f) if !f.is_nan() || s == "NaN" => typed(if f.is_finite() { format!("{f:E}") } else { Num::Double(f).lexical() }, dt),
                    _ => Value::Error,
                }
            }
        }
        None => Value::Error,
    })
}

thread_local! {
    /// The query's BASE IRI (when declared), used by IRI()/URI() to resolve relative
    /// references. Set by the `lib.rs` query entry points after parsing.
    pub(super) static QUERY_BASE: std::cell::RefCell<Option<oxiri::Iri<String>>> = const { std::cell::RefCell::new(None) };
}

/// Installs the active query's base IRI for expression evaluation (IRI()/URI()
/// relative-reference resolution). Called by the query entry points; `None` clears it.
///
/// The returned guard restores the previous base when dropped (including on unwind), so
/// a query run re-entrantly on the same thread — e.g. from an extension function — cannot
/// leak its BASE (or its lack of one) into the enclosing query (#6479). Bind it to a named
/// variable for the duration of evaluation.
pub(crate) fn set_query_base(base: Option<&str>) -> QueryBaseGuard {
    let new = base.and_then(|s| oxiri::Iri::parse(s.to_string()).ok());
    QueryBaseGuard { previous: Some(QUERY_BASE.with(|b| b.replace(new))) }
}

/// The calling thread's query base, for re-installing on rayon workers with
/// [`query_base_worker_install`] (a worker thread has its own, empty, `QUERY_BASE`).
#[cfg_attr(not(feature = "parallel"), allow(dead_code))]
pub(crate) fn query_base_snapshot() -> Option<oxiri::Iri<String>> {
    QUERY_BASE.with(|b| b.borrow().clone())
}

/// Installs a [`query_base_snapshot`] on the current (worker) thread until the guard drops.
/// Free when the query declares no BASE and the worker has none installed.
#[cfg_attr(not(feature = "parallel"), allow(dead_code))]
pub(crate) fn query_base_worker_install(base: &Option<oxiri::Iri<String>>) -> QueryBaseGuard {
    if base.is_none() && QUERY_BASE.with(|b| b.borrow().is_none()) {
        return QueryBaseGuard { previous: None };
    }
    QueryBaseGuard { previous: Some(QUERY_BASE.with(|b| b.replace(base.clone()))) }
}

/// Restores the enclosing query's base IRI on drop; see [`set_query_base`].
#[must_use = "the query base is restored when the guard drops; bind it to a named variable"]
pub(crate) struct QueryBaseGuard {
    pub(super) previous: Option<Option<oxiri::Iri<String>>>,
}

impl Drop for QueryBaseGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.take() {
            QUERY_BASE.with(|b| *b.borrow_mut() = previous);
        }
    }
}

/// `IRI(str)`: absolute IRIs pass through; relative references resolve against the
/// query's BASE (a relative reference without a base is a type error).
pub(super) fn resolve_iri(s: &str) -> Option<oxrdf::NamedNode> {
    if let Ok(abs) = oxiri::Iri::parse(s.to_string()) {
        return Some(oxrdf::NamedNode::new_unchecked(abs.into_inner()));
    }
    QUERY_BASE.with(|b| {
        b.borrow()
            .as_ref()
            .and_then(|base| base.resolve(s).ok())
            .map(|iri| oxrdf::NamedNode::new_unchecked(iri.into_inner()))
    })
}

thread_local! {
    /// The identity of the solution row an expression is being evaluated for:
    /// (bindings identity — the rows buffer address —, row index). Set by the per-row
    /// evaluation loops (BIND / FILTER); BNODE(str) derives its label from it, so equal
    /// arguments within one solution share a blank node while distinct solutions get
    /// distinct ones. The buffer address is stable across consecutive Extends over the
    /// same Bindings (the SELECT-expression case the per-solution rule exists for).
    pub(super) static ROW_SCOPE: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

/// The query execution's `NOW()` instant as an `xsd:dateTime` lexical (civil-from-days
/// conversion, no time-crate dependency). The instant comes from the `query_now`
/// scope pinned in `eval_modified`, so every `NOW()` in one execution — across rows,
/// rayon workers and `EXISTS` re-entry — formats the SAME value (SPARQL 1.1
/// §17.4.5.1); an un-scoped call falls back to a fresh sample. sq-98w7z.1
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "zkvm")))]
pub(super) fn now_lexical() -> String {
    let secs = query_now::epoch_secs();
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    format!("{year:04}-{month:02}-{day:02}T{h:02}:{mi:02}:{s:02}Z")
}


/// 64-bit FxHash of a string (label material for BNODE(str)).
pub(super) fn fx64(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = rustc_hash::FxHasher::default();
    s.hash(&mut h);
    h.finish()
}

/// The string value of a simple literal / xsd:string argument — the only operand type
/// the hash builtins and BNODE(str) accept; anything else is a type error (`None`).
pub(super) fn string_literal(v: &Value) -> Option<String> {
    match v {
        Value::Term(Term::Literal(l)) if l.language().is_none() && l.datatype() == xsd::STRING => {
            Some(l.value().to_string())
        }
        _ => None,
    }
}

/// A STRING-LITERAL operand (simple/xsd:string or language-tagged): `(value, language)`.
/// IRIs, blank nodes, non-string literals and computed numerics/booleans are type
/// errors (`None`) — per the SPARQL string-function operand rules.
///
/// An RDF 1.2 base direction rides COMBINED into the language slot as `lang--dir`
/// (`--` cannot occur in a BCP47 tag): the string functions then preserve language AND
/// direction together, equality of the slot means "same language and same direction"
/// (exactly the SPARQL 1.2 CONCAT/compatibility rule), and [`lit_with_lang`] splits the
/// pair back out.
pub(super) fn str_lit(v: &Value) -> Option<(String, Option<String>)> {
    match v {
        Value::Term(Term::Literal(l)) => {
            if let Some(lang) = l.language() {
                let tag = match l.direction() {
                    Some(d) => format!("{lang}--{d}"),
                    None => lang.to_string(),
                };
                Some((l.value().to_string(), Some(tag)))
            } else if l.datatype() == xsd::STRING {
                Some((l.value().to_string(), None))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// A simple or language-tagged literal value, per the (possibly `lang--dir` combined,
/// see [`str_lit`]) tag the operand carried.
pub(super) fn lit_with_lang(s: String, lang: Option<&str>) -> Value {
    Value::Term(Term::Literal(match lang {
        Some(l) => match l.split_once("--") {
            Some((tag, dir)) => Literal::new_directional_language_tagged_literal_unchecked(
                s,
                tag,
                if dir == "rtl" { oxrdf::BaseDirection::Rtl } else { oxrdf::BaseDirection::Ltr },
            ),
            None => Literal::new_language_tagged_literal_unchecked(s, l),
        },
        None => Literal::new_simple_literal(s),
    }))
}

/// Lowercase-hex digest of a string-literal argument, or a type error.
#[cfg(feature = "digest")]
pub(super) fn digest_hex<D: md5::Digest>(v: &Value) -> Value {
    use std::fmt::Write;
    match string_literal(v) {
        Some(s) => {
            let out = D::digest(s.as_bytes());
            let mut hex = String::with_capacity(out.len() * 2);
            for byte in out {
                let _ = write!(hex, "{byte:02x}");
            }
            Value::Term(Term::Literal(Literal::new_simple_literal(hex)))
        }
        None => Value::Error,
    }
}

/// The timezone part of an xsd:dateTime argument's lexical form: `"Z"`, `"±hh:mm"`, or
/// `""` when absent. `None` (type error) when the argument is not a valid xsd:dateTime.
pub(super) fn datetime_arg_tz(v: &Value) -> Option<String> {
    let l = match v {
        Value::Term(Term::Literal(l))
            if l.datatype() == xsd::DATE_TIME || l.datatype() == xsd::DATE_TIME_STAMP =>
        {
            l
        }
        _ => return None,
    };
    temporal_of_lit(l)?; // Shared datatype validation includes dateTimeStamp's required timezone.
    let s = l.value();
    parse_datetime(s)?; // lexical shape check
    let (_, time) = s.split_once('T')?;
    Some(match time.find(['Z', '+', '-']) {
        Some(i) => time[i..].to_string(),
        None => String::new(),
    })
}

/// XSD-canonical `xsd:dayTimeDuration` for a timezone string (`Z` / `±hh:mm`); an empty
/// timezone is a type error (`None`).
pub(super) fn tz_to_duration(tz: &str) -> Option<String> {
    if tz.is_empty() {
        return None;
    }
    if tz == "Z" {
        return Some("PT0S".to_string());
    }
    let (sign, hm) = tz.split_at(1);
    let (h, m) = hm.split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    if h == 0 && m == 0 {
        return Some("PT0S".to_string());
    }
    let mut out = String::new();
    if sign == "-" {
        out.push('-');
    }
    out.push_str("PT");
    if h > 0 {
        out.push_str(&format!("{h}H"));
    }
    if m > 0 {
        out.push_str(&format!("{m}M"));
    }
    Some(out)
}


/// Build a regex honouring the SPARQL flag string (`i` case-insensitive, `s` dot-all, `m`
/// multi-line, `x` extended/ignore-whitespace, `q` literal-pattern mode per XPath F&O —
/// every pattern character is matched literally, combinable with `i`).
/// Returns `None` on an invalid pattern or an unknown flag (→ type error).
/// Compile-per-call — go through [`regex_cache::get`] on any per-row path.
#[cfg(feature = "regex")]
pub(super) fn build_regex(pattern: &str, flags: &str) -> Option<regex::Regex> {
    if !flags.chars().all(|c| matches!(c, 'i' | 's' | 'm' | 'x' | 'q')) {
        return None;
    }
    let literal = flags.contains('q');
    let pattern = if literal { regex::escape(pattern) } else { pattern.to_string() };
    regex::RegexBuilder::new(&pattern)
        .case_insensitive(flags.contains('i'))
        // `q` suppresses the meaning of the OTHER flags' metacharacters too (per
        // XPath, only `i` keeps its effect alongside `q`).
        .dot_matches_new_line(!literal && flags.contains('s'))
        .multi_line(!literal && flags.contains('m'))
        .ignore_whitespace(!literal && flags.contains('x'))
        .build()
        .ok()
}

/// SPARQL `ENCODE_FOR_URI`: percent-encode everything except the unreserved set (RFC 3986
/// ALPHA / DIGIT / `-` `.` `_` `~`).
pub(super) fn encode_for_uri(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push_str(&format!("{byte:02X}"));
        }
    }
    out
}

/// Extract a numeric `xsd:dateTime` component (0=year…5=seconds) from a value's lexical
/// form. YEAR…MINUTES return xsd:integer; SECONDS returns xsd:decimal (per SPARQL),
/// parsed from the lexical so fractional seconds stay exact.
pub(super) fn datetime_field(v: &Value, idx: usize) -> Value {
    // Date accessors accept typed dateTime values, not strings or IRIs
    // whose text happens to look like a timestamp.
    let s = match v {
        Value::Term(Term::Literal(l))
            if l.datatype() == xsd::DATE_TIME || l.datatype() == xsd::DATE_TIME_STAMP => {
                // `parse_datetime` below validates the lexical; this adds only the checks
                // `temporal_of_lit` makes beyond it, without a second full parse.
                if budget::check_temporal(l.value(), l.datatype().as_str()).is_err()
                    || (l.datatype() == xsd::DATE_TIME_STAMP
                        && !l.value().split_once('T').is_some_and(|(_, time)| time.contains(['Z', '+', '-'])))
                {
                    return Value::Error;
                }
                l.value()
            },
        _ => return Value::Error,
    };
    let fields = match parse_datetime(s) {
        Some(f) => f,
        None => return Value::Error,
    };
    if idx == 5 {
        // Re-extract the seconds lexical (timezone stripped) for an exact decimal.
        let lex = s
            .split_once('T')
            .map(|(_, t)| if let Some(i) = t.find(['Z', '+', '-']) { &t[..i] } else { t })
            .and_then(|t| t.rsplit_once(':').map(|(_, sec)| sec));
        return match lex {
            Some(lex) => match Dec::parse_lexical(lex) {
                Some(d) => Value::Num(Num::Dec(d)),
                None => {
                    // Accessor output is lexical, not bounded-decimal arithmetic.
                    // Preserve every validated fractional digit without f64/i128 rounding.
                    let (whole, fraction) = lex.split_once('.').unwrap_or((lex, ""));
                    let whole = whole.trim_start_matches('0');
                    let fraction = fraction.trim_end_matches('0');
                    Value::Term(Term::Literal(Literal::new_typed_literal(
                        format!("{}.{}", if whole.is_empty() { "0" } else { whole },
                            if fraction.is_empty() { "0" } else { fraction }),
                        xsd::DECIMAL,
                    )))
                }
            },
            None => Value::Error,
        };
    }
    Value::Num(Num::Int(fields[idx] as i64))
}

/// Parse an `xsd:dateTime` lexical (`[-]YYYY-MM-DDThh:mm:ss[.frac][TZ]`) into
/// `[year, month, day, hours, minutes, seconds]`. Timezone is stripped (component accessors are on
/// the local time per SPARQL); seconds keeps any fractional part.
pub(super) fn parse_datetime(s: &str) -> Option<[f64; 6]> {
    // One validation boundary also serves the graph's temporal cache and
    // comparison fast paths. Component extraction below preserves local time
    // and reads fixed-width digits the validator has already checked.
    ExactTimeline::parse_datetime(s)?;
    let (date, time) = s.split_once('T')?;
    let neg = date.starts_with('-');
    let date = date.strip_prefix('-').unwrap_or(date);
    let (year_lex, month_day) = date.split_at(date.len().checked_sub(6)?);
    let two = |b: &[u8]| i64::from(b[0] - b'0') * 10 + i64::from(b[1] - b'0');
    let (md, tb) = (month_day.as_bytes(), time.as_bytes());
    let mut year: i64 = year_lex.parse().ok()?;
    if neg {
        year = -year;
    }
    let (mut month, mut day) = (two(&md[1..3]), two(&md[4..6]));
    let (mut hours, minutes) = (two(&tb[0..2]), two(&tb[3..5]));
    // Strip the timezone (Z, or +hh:mm / -hh:mm after the seconds — the time part itself has no '-').
    let seconds_lex = &time[6..time.find(['Z', '+', '-']).unwrap_or(time.len())];
    let seconds: f64 = seconds_lex.parse().ok()?;
    // XPath component extraction uses the value: 24:00 is next-day midnight.
    // Reuse the shared calendar validator for month length and leap years.
    if hours == 24 {
        hours = 0;
        day += 1;
        let next_date = format!("{}{}-{:02}-{:02}", if neg { "-" } else { "" }, year_lex, month, day);
        if sparq_core::temporal::parse_civil_date(&next_date).is_none() {
            day = 1;
            month += 1;
            if month == 13 {
                month = 1;
                year += 1; // XSD 1.1 counts through year zero.
            }
        }
    }
    Some([year as f64, month as f64, day as f64, hours as f64, minutes as f64, seconds])
}

pub(super) fn value_str(v: &Value) -> Option<String> {
    match v {
        Value::Term(Term::Literal(l)) => Some(l.value().to_string()),
        Value::Term(Term::NamedNode(n)) => Some(n.as_str().to_string()),
        Value::Term(Term::BlankNode(b)) => Some(b.as_str().to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Num(n) => Some(n.lexical()),
        Value::Unbound | Value::Error => None,
        Value::Term(_) => None,
    }
}

/// Resolves a computed NUMERIC value to an id without constructing a Term when
/// possible: an inline-range integer encodes straight into its id (no allocation at
/// all); any other numeric probes the dictionary by (lexical, datatype) parts.
/// `Ok(id)` on a hit; `Err(lexical)` carries the (already formatted) lexical form for
/// the caller's local-vocab miss path.
#[inline]
pub(super) fn num_to_id(graph: &Graph, n: Num) -> Result<Id, String> {
    if let Num::Int(i) = n {
        if let Some(id) = dict::inline_id_of_int(i) {
            return Ok(id);
        }
    }
    let lex = n.lexical();
    match graph.dict.lookup_lit(&lex, n.datatype().as_str(), None) {
        NO_ID => Err(lex),
        id => Ok(id),
    }
}

/// Converts an evaluated value into an id. Computed terms are resolved against
/// the graph dictionary first (so they join and deduplicate against the data);
/// terms not already present get a per-query local id. Computed numerics skip
/// Term construction entirely when they resolve to an inline id or a dictionary
/// term (the BIND fast path).
pub(super) fn value_to_id(graph: &Graph, local: &mut LocalVocab, v: &Value) -> Id {
    let term = match v {
        Value::Unbound | Value::Error => return NO_ID,
        Value::Num(n) => match num_to_id(graph, *n) {
            Ok(id) => return id,
            Err(lex) => Term::Literal(Literal::new_typed_literal(lex, n.datatype())),
        },
        Value::Bool(b) => {
            let lex = if *b { "true" } else { "false" };
            match graph.dict.lookup_lit(lex, xsd::BOOLEAN.as_str(), None) {
                NO_ID => Term::Literal(Literal::new_typed_literal(lex, xsd::BOOLEAN)),
                id => return id,
            }
        }
        Value::Term(t) => {
            if let Some(id) = graph.id_of(t) {
                return id;
            }
            t.clone()
        }
    };
    local.intern(term)
}

/// `sameTerm` over evaluated operands. Computed numerics and booleans compare as the
/// literal a BIND of them would produce, so `sameTerm(1 + 0, 1)` is true; an unbound
/// or error operand is a type error rather than `false`.
pub(super) fn same_term_value(x: &Value, y: &Value) -> Value {
    if let (Value::Term(p), Value::Term(q)) = (x, y) {
        return Value::Bool(p == q);
    }
    match (value_as_term(x), value_as_term(y)) {
        (Some(p), Some(q)) => Value::Bool(p == q),
        _ => Value::Error,
    }
}

/// A computed value as a concrete RDF term (`None` for unbound / type error).
pub(super) fn value_as_term(v: &Value) -> Option<Term> {
    Some(match v {
        Value::Unbound | Value::Error => return None,
        Value::Bool(b) => Term::Literal(Literal::new_typed_literal(b.to_string(), xsd::BOOLEAN)),
        Value::Num(n) => Term::Literal(Literal::new_typed_literal(n.lexical(), n.datatype())),
        Value::Term(t) => t.clone(),
    })
}

/// Read-only half of [`value_to_id`], for the parallel resolve pass (T1.0b). Most computed values
/// resolve WITHOUT touching the mutable vocab: small integers inline into the id, and terms
/// already in the graph dictionary (or already interned locally) are read-only lookups. Only a
/// genuinely new term returns `Err(term)` — carrying the constructed `Term` so the serial
/// intern-the-misses pass does no re-construction. Splitting this way removes the bulk of the B1
/// serialization point (research/parallelism-scaling.md) without sharded-vocab complexity, and
/// stays byte-identical: misses are interned in row order, exactly as the serial path would.
pub(super) fn value_to_id_readonly(graph: &Graph, local: &LocalVocab, v: &Value) -> Result<Id, Term> {
    let term = match v {
        Value::Unbound | Value::Error => return Ok(NO_ID),
        // Computed numerics/booleans skip Term construction when they resolve to an
        // inline id or a dictionary term — the constructed-Term path only runs for
        // values that genuinely head to the local vocab (the BIND fast path).
        Value::Num(n) => match num_to_id(graph, *n) {
            Ok(id) => return Ok(id),
            Err(lex) => Term::Literal(Literal::new_typed_literal(lex, n.datatype())),
        },
        Value::Bool(b) => {
            let lex = if *b { "true" } else { "false" };
            match graph.dict.lookup_lit(lex, xsd::BOOLEAN.as_str(), None) {
                NO_ID => Term::Literal(Literal::new_typed_literal(lex, xsd::BOOLEAN)),
                id => return Ok(id),
            }
        }
        Value::Term(t) => {
            if let Some(id) = graph.id_of(t) {
                return Ok(id);
            }
            t.clone()
        }
    };
    if let Some(&id) = local.ids.get(&term) {
        return Ok(id);
    }
    Err(term)
}
