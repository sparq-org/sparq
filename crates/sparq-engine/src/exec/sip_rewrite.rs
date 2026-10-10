use super::*;

/// Maximum size of the already-evaluated (small) join side for which correlated
/// evaluation of the other child is attempted; above it the cold path is used.
pub(super) const SIP_MAX_SMALL_ROWS: usize = 64;

/// Certain (always-bound) REAL variables of a graph pattern — a sound
/// UNDER-approximation of the SPARQL bound set: it never adds a variable that some
/// solution may leave unbound. These are substitution candidates; `subst_pattern`
/// separately checks the algebra and expression scopes. Blank-node slots are excluded.
pub(super) fn certain_vars(p: &GraphPattern, out: &mut FxHashSet<Variable>) {
    use GraphPattern as G;
    // A directly-positioned (non-quoted, non-blank) query variable of a term slot.
    fn real(t: &TermPattern) -> Option<&Variable> {
        if let TermPattern::Variable(v) = t {
            Some(v)
        } else {
            None
        }
    }
    match p {
        G::Bgp { patterns } => {
            for tp in patterns {
                for v in [real(&tp.subject), nnp_var_ref(&tp.predicate), real(&tp.object)]
                    .into_iter()
                    .flatten()
                {
                    out.insert(v.clone());
                }
            }
        }
        G::Path { subject, object, .. } => {
            for v in [real(subject), real(object)].into_iter().flatten() {
                out.insert(v.clone());
            }
        }
        G::Join { left, right } => {
            certain_vars(left, out);
            certain_vars(right, out);
        }
        // Only the mandatory (left) side of an OPTIONAL / MINUS is certainly bound.
        G::LeftJoin { left, .. } | G::Minus { left, .. } => certain_vars(left, out),
        G::Filter { inner, .. }
        | G::OrderBy { inner, .. }
        | G::Distinct { inner }
        | G::Reduced { inner }
        | G::Slice { inner, .. } => certain_vars(inner, out),
        // A variable is certain in a UNION only if certain in BOTH branches.
        G::Union { left, right } => {
            let mut l = FxHashSet::default();
            let mut r = FxHashSet::default();
            certain_vars(left, &mut l);
            certain_vars(right, &mut r);
            out.extend(l.intersection(&r).cloned());
        }
        G::Graph { name, inner } => {
            if let NamedNodePattern::Variable(v) = name {
                out.insert(v.clone());
            }
            certain_vars(inner, out);
        }
        // BIND may evaluate to an error (leaving its target UNBOUND), so the extended
        // variable is NOT certain; only the inner pattern's certain vars are.
        G::Extend { inner, .. } => certain_vars(inner, out),
        G::Values { variables, bindings } => {
            for (i, v) in variables.iter().enumerate() {
                if bindings.iter().all(|row| row.get(i).is_some_and(Option::is_some)) {
                    out.insert(v.clone());
                }
            }
        }
        // Projection / grouping drop non-listed variables; keep the listed ones that
        // are certain in the inner pattern.
        G::Project { inner, variables } | G::Group { inner, variables, .. } => {
            let mut inner_c = FxHashSet::default();
            certain_vars(inner, &mut inner_c);
            for v in variables {
                if inner_c.contains(v) {
                    out.insert(v.clone());
                }
            }
        }
        // Service (and anything else): conservatively bind nothing certain.
        _ => {}
    }
}

/// (sq-7d3dj.30.11) Static term-kind analysis for the id-level FILTER fast path:
/// the REAL variables a graph pattern proves can never bind to a LITERAL — i.e. every place
/// the variable is (or could be) bound is a subject or predicate slot of a triple/path pattern.
/// Subjects are IRIs / blank nodes (or RDF 1.2 quoted triples) and predicates are always IRIs;
/// none of those is a literal. A variable is EXCLUDED the moment it appears anywhere it could
/// take a literal: an OBJECT slot, a `BIND` target, a `VALUES` column, or under any construct we
/// do not analyse (`SERVICE`, a sub-`SELECT` projecting it, …). Sound UNDER-approximation: it
/// only reports a variable when literal-binding is provably impossible, so a `=`/`!=` over two
/// reported variables (or a reported variable and a constant IRI) is exactly id (in)equality.
///
/// Implementation: collect `nonlit_positioned` (appears in a subject/predicate slot at least
/// once) and `lit_possible` (appears in a literal-capable position, or is produced by an
/// unanalysable construct), then return `nonlit_positioned \ lit_possible`.
///
/// (sq-1ivw7) The `obj_may_be_literal` classifier extends the reach to OBJECT-position
/// variables: a BGP object slot with a CONSTANT predicate whose object column is provably
/// literal-free in the CURRENT store snapshot is treated as a non-literal slot (like a subject),
/// rather than poisoned to literal-risk. `obj_may_be_literal(p)` returns `true` when predicate `p`
/// MIGHT have a literal object (the conservative default), so the object stays literal-risk. It is
/// snapshot-scoped: the caller must build it against the same `&Graph` the query executes on, and
/// the whole analysis is re-run per evaluation (see `predicate_has_literal_object`).
#[cfg(feature = "id-filter-fastpath")]
pub(super) fn nonliteral_vars(p: &GraphPattern, obj_may_be_literal: &dyn Fn(&oxrdf::NamedNode) -> bool) -> FxHashSet<Variable> {
    let mut nonlit_positioned = FxHashSet::default();
    let mut lit_possible = FxHashSet::default();
    collect_kind_positions(p, &mut nonlit_positioned, &mut lit_possible, obj_may_be_literal);
    nonlit_positioned.retain(|v| !lit_possible.contains(v));
    nonlit_positioned
}

/// Walks `p`, recording each REAL variable into `nonlit` if it appears in a subject/predicate
/// slot (or a literal-free constant-predicate object slot) and into `maybe_lit` if it appears
/// anywhere it could be bound to a literal (a literal-capable object slot, a `BIND`/`VALUES`
/// binding, or any un-analysed construct that could bind it).
#[cfg(feature = "id-filter-fastpath")]
pub(super) fn collect_kind_positions(
    p: &GraphPattern,
    nonlit: &mut FxHashSet<Variable>,
    maybe_lit: &mut FxHashSet<Variable>,
    obj_may_be_literal: &dyn Fn(&oxrdf::NamedNode) -> bool,
) {
    use GraphPattern as G;
    // A quoted-triple slot (`TermPattern::Triple`) recurses: its INNER object can itself be a
    // literal, so a variable appearing anywhere inside a quoted triple is treated as literal-risk.
    // Poison EVERY variable syntactically inside a term slot into `maybe_lit` — used for slots
    // that can bind a literal (object slots, path endpoints) and for quoted-triple slots whose
    // inner object can be a literal. Conservative: only ever ADDS literal-risk, never non-literal.
    fn poison_slot(t: &TermPattern, maybe_lit: &mut FxHashSet<Variable>) {
        match t {
            TermPattern::Variable(v) => {
                maybe_lit.insert(v.clone());
            }
            TermPattern::Triple(tp) => {
                poison_slot(&tp.subject, maybe_lit);
                if let NamedNodePattern::Variable(v) = &tp.predicate {
                    maybe_lit.insert(v.clone());
                }
                poison_slot(&tp.object, maybe_lit);
            }
            _ => {}
        }
    }
    // Poison every IN-SCOPE variable of an un-analysed sub-pattern (SERVICE, Lateral, …): we
    // cannot prove ANY of them non-literal, and — crucially — a variable one of these binds must
    // NOT be claimed non-literal on the strength of a subject occurrence in a sibling UNION branch.
    fn poison_all(p: &GraphPattern, maybe_lit: &mut FxHashSet<Variable>) {
        p.on_in_scope_variable(|v| {
            maybe_lit.insert(v.clone());
        });
    }
    match p {
        G::Bgp { patterns } => {
            for tp in patterns {
                // Subject / predicate slots are IRI / bnode — never a literal. A quoted-triple
                // subject can carry an inner literal, so poison it instead of crediting it.
                match &tp.subject {
                    TermPattern::Variable(v) => {
                        nonlit.insert(v.clone());
                    }
                    other => poison_slot(other, maybe_lit),
                }
                if let Some(v) = nnp_var_ref(&tp.predicate) {
                    nonlit.insert(v.clone());
                }
                // The object slot can bind a literal (directly or inside a quoted triple) — UNLESS
                // the predicate is a CONSTANT IRI proven literal-free in this snapshot, in which
                // case a plain object VARIABLE is non-literal (an IRI or bnode), like a subject.
                // (sq-1ivw7) A quoted-triple object still poisons (its inner object can be
                // a literal regardless of the outer predicate), so the credit applies only to the
                // bare `TermPattern::Variable` object form.
                match (&tp.predicate, &tp.object) {
                    (NamedNodePattern::NamedNode(pred), TermPattern::Variable(ov))
                        if !obj_may_be_literal(pred) =>
                    {
                        nonlit.insert(ov.clone());
                    }
                    _ => poison_slot(&tp.object, maybe_lit),
                }
            }
        }
        // A property-path binds subject and object, but the OBJECT end can be a literal (the final
        // step of a sequence/alternative is an ordinary object slot), and under `Reverse` or a
        // zero-length `ZeroOrMore`/`ZeroOrOne` match the SUBJECT end can equal a literal object.
        // So NEITHER endpoint is credited non-literal here — both are literal-risk. A path variable
        // is still proven non-literal if it appears in a subject/predicate slot of some BGP.
        G::Path { subject, object, .. } => {
            poison_slot(subject, maybe_lit);
            poison_slot(object, maybe_lit);
        }
        G::Join { left, right } | G::Union { left, right } => {
            collect_kind_positions(left, nonlit, maybe_lit, obj_may_be_literal);
            collect_kind_positions(right, nonlit, maybe_lit, obj_may_be_literal);
        }
        // LATERAL binds its right side correlated on the left; the right can be any construct
        // (incl. a sub-SELECT projecting a computed literal), so poison the right's in-scope vars
        // and analyse the left normally.
        G::Lateral { left, right } => {
            collect_kind_positions(left, nonlit, maybe_lit, obj_may_be_literal);
            poison_all(right, maybe_lit);
        }
        // OPTIONAL / MINUS: the right side still POSITIONS its variables, so its subject/predicate
        // occurrences remain non-literal evidence and its object occurrences remain literal-risk.
        G::LeftJoin { left, right, .. } => {
            collect_kind_positions(left, nonlit, maybe_lit, obj_may_be_literal);
            collect_kind_positions(right, nonlit, maybe_lit, obj_may_be_literal);
        }
        G::Minus { left, right } => {
            collect_kind_positions(left, nonlit, maybe_lit, obj_may_be_literal);
            collect_kind_positions(right, nonlit, maybe_lit, obj_may_be_literal);
        }
        G::Filter { inner, .. }
        | G::OrderBy { inner, .. }
        | G::Distinct { inner }
        | G::Reduced { inner }
        | G::Slice { inner, .. } => collect_kind_positions(inner, nonlit, maybe_lit, obj_may_be_literal),
        G::Graph { name, inner } => {
            // A GRAPH-name variable binds a graph IRI — non-literal.
            if let NamedNodePattern::Variable(v) = name {
                nonlit.insert(v.clone());
            }
            // (sq-1ivw7) Recurse with the ALWAYS-CONSERVATIVE classifier (`&|_| true`),
            // NOT the caller's `obj_may_be_literal`. The predicate-range object credit is scoped to
            // ONE graph's object column: `obj_may_be_literal` is built by `nonliteral_filter_cols`
            // against the graph the OUTER dispatch executes on (default graph for a FILTER sitting
            // OUTSIDE the GRAPH block), but a plain object variable inside `GRAPH <g>`/`GRAPH ?g`
            // is bound from a DIFFERENT store (the named sub-graph). Crediting it off the outer
            // graph's column would be unsound (a named-graph literal object gets id-compared for
            // `=`/`!=`). So object slots under ANY GRAPH block stay literal-risk at an outer
            // dispatch. The INSIDE-GRAPH dispatch is already correct and needs no widening here:
            // when the FILTER sits inside the block, `eval_translated` evaluates `inner` with
            // `graph = sub`, so its own `nonliteral_filter_cols` scans the right (named) store.
            collect_kind_positions(inner, nonlit, maybe_lit, &|_| true);
        }
        // A BIND target can evaluate to a literal — literal-risk.
        G::Extend { inner, variable, .. } => {
            maybe_lit.insert(variable.clone());
            collect_kind_positions(inner, nonlit, maybe_lit, obj_may_be_literal);
        }
        // Every VALUES column can carry a literal — literal-risk for all of them.
        G::Values { variables, .. } => {
            for v in variables {
                maybe_lit.insert(v.clone());
            }
        }
        // PROJECT (a sub-SELECT) only exposes its projected variables; its inner variables are NOT
        // visible to the surrounding query, so inner-only vars must contribute NEITHER non-literal
        // evidence NOR literal-risk to the outer scope. Analyse the inner into local sets and merge
        // back only the classifications for the PROJECTED variables.
        G::Project { inner, variables } => {
            let mut inner_nl = FxHashSet::default();
            let mut inner_ml = FxHashSet::default();
            collect_kind_positions(inner, &mut inner_nl, &mut inner_ml, obj_may_be_literal);
            let proj: FxHashSet<&Variable> = variables.iter().collect();
            for v in inner_nl.into_iter().filter(|v| proj.contains(v)) {
                nonlit.insert(v);
            }
            for v in inner_ml.into_iter().filter(|v| proj.contains(v)) {
                maybe_lit.insert(v);
            }
        }
        // GROUP: a grouped-by variable keeps the inner verdict; an aggregate result variable
        // (SUM/COUNT/… AS ?v) is a computed literal. Like PROJECT, only the OUTPUT columns
        // (group keys + aggregate targets) are visible, so analyse the inner locally and merge only
        // the group-key variables' verdicts; the aggregate targets are added as literal-risk.
        G::Group { inner, variables, aggregates } => {
            let mut inner_nl = FxHashSet::default();
            let mut inner_ml = FxHashSet::default();
            collect_kind_positions(inner, &mut inner_nl, &mut inner_ml, obj_may_be_literal);
            let keys: FxHashSet<&Variable> = variables.iter().collect();
            for v in inner_nl.into_iter().filter(|v| keys.contains(v)) {
                nonlit.insert(v);
            }
            for v in inner_ml.into_iter().filter(|v| keys.contains(v)) {
                maybe_lit.insert(v);
            }
            for (v, _) in aggregates {
                maybe_lit.insert(v.clone());
            }
        }
        // SERVICE and anything else un-analysed: we cannot prove ANY of its variables non-literal,
        // and — since a sibling UNION branch might position one of them as a subject — we must
        // POISON all of its in-scope variables into `maybe_lit` so no such false non-literal claim
        // survives the `nonlit \ maybe_lit` difference in `nonliteral_vars`.
        other => poison_all(other, maybe_lit),
    }
}

/// A borrow of a predicate-slot variable (predicates are never blank/quoted).
pub(super) fn nnp_var_ref(p: &NamedNodePattern) -> Option<&Variable> {
    match p {
        NamedNodePattern::Variable(v) => Some(v),
        NamedNodePattern::NamedNode(_) => None,
    }
}

/// Restricts positive patterns by IRI bindings without crossing scopes.
/// Both ordinary SIP and the theta anti-join seed path use this admission rule.
/// Other algebra uses ordinary evaluation: constant replacement is not selection
/// through binding, negative, nullable-domain, or solution-modifier boundaries.
pub(super) fn subst_pattern(p: &GraphPattern, sub: &FxHashMap<Variable, oxrdf::NamedNode>) -> Option<GraphPattern> {
    use GraphPattern as G;
    Some(match p {
        G::Bgp { patterns } => G::Bgp {
            patterns: patterns.iter().map(|tp| subst_triple(tp, sub)).collect::<Option<Vec<_>>>()?,
        },
        G::Path { subject, path, object } => {
            // Variable-bearing triple terms are decomposed only in
            // BGPs. Grounding one here would erase the ordinary path error.
            if matches!(subject, TermPattern::Triple(_)) || matches!(object, TermPattern::Triple(_)) {
                return None;
            }
            let substituted = |term: &TermPattern| {
                matches!(term, TermPattern::Variable(v) if sub.contains_key(v))
            };
            if path_nullable(path) && (substituted(subject) || substituted(object)) {
                return None;
            }
            G::Path {
                subject: subst_term(subject, sub)?,
                path: path.clone(),
                object: subst_term(object, sub)?,
            }
        },
        G::Join { left, right } => G::Join {
            left: Box::new(subst_pattern(left, sub)?),
            right: Box::new(subst_pattern(right, sub)?),
        },
        G::Union { left, right } => G::Union {
            left: Box::new(subst_pattern(left, sub)?),
            right: Box::new(subst_pattern(right, sub)?),
        },
        G::Filter { expr, inner } => {
            // A binding supplied by a sibling is not in this FILTER's input.
            // Rewriting it would turn an unbound/error expression into a value.
            let mut bound = FxHashSet::default();
            certain_vars(inner, &mut bound);
            if !sub.keys().all(|v| bound.contains(v)) {
                return None;
            }
            G::Filter {
                expr: subst_expr(expr, sub)?,
                inner: Box::new(subst_pattern(inner, sub)?),
            }
        },
        // MINUS depends on domains and its full right relation; OPTIONAL and
        // binders introduce scopes; projection/modifiers can change which rows
        // exist. Graph/service boundaries also stay outside this positive shape.
        _ => return None,
    })
}

pub(super) fn subst_triple(tp: &TriplePattern, sub: &FxHashMap<Variable, oxrdf::NamedNode>) -> Option<TriplePattern> {
    Some(TriplePattern {
        subject: subst_term(&tp.subject, sub)?,
        predicate: subst_nnp(&tp.predicate, sub),
        object: subst_term(&tp.object, sub)?,
    })
}

pub(super) fn subst_term(t: &TermPattern, sub: &FxHashMap<Variable, oxrdf::NamedNode>) -> Option<TermPattern> {
    Some(match t {
        TermPattern::Variable(v) => match sub.get(v) {
            Some(nn) => TermPattern::NamedNode(nn.clone()),
            None => TermPattern::Variable(v.clone()),
        },
        // A quoted triple term (RDF 1.2) can embed a substituted variable — recurse.
        TermPattern::Triple(inner) => TermPattern::Triple(Box::new(subst_triple(inner, sub)?)),
        other => other.clone(),
    })
}

pub(super) fn subst_nnp(p: &NamedNodePattern, sub: &FxHashMap<Variable, oxrdf::NamedNode>) -> NamedNodePattern {
    match p {
        NamedNodePattern::Variable(v) => match sub.get(v) {
            Some(nn) => NamedNodePattern::NamedNode(nn.clone()),
            None => NamedNodePattern::Variable(v.clone()),
        },
        NamedNodePattern::NamedNode(_) => p.clone(),
    }
}

pub(super) fn subst_expr(e: &Expression, sub: &FxHashMap<Variable, oxrdf::NamedNode>) -> Option<Expression> {
    use Expression as E;
    let bx = |x: Option<Expression>| x.map(Box::new);
    Some(match e {
        E::NamedNode(_) | E::Literal(_) => e.clone(),
        E::Variable(v) => match sub.get(v) {
            Some(nn) => E::NamedNode(nn.clone()),
            None => E::Variable(v.clone()),
        },
        E::Or(a, b) => E::Or(bx(subst_expr(a, sub))?, bx(subst_expr(b, sub))?),
        E::And(a, b) => E::And(bx(subst_expr(a, sub))?, bx(subst_expr(b, sub))?),
        E::Equal(a, b) => E::Equal(bx(subst_expr(a, sub))?, bx(subst_expr(b, sub))?),
        E::SameTerm(a, b) => E::SameTerm(bx(subst_expr(a, sub))?, bx(subst_expr(b, sub))?),
        E::Greater(a, b) => E::Greater(bx(subst_expr(a, sub))?, bx(subst_expr(b, sub))?),
        E::GreaterOrEqual(a, b) => E::GreaterOrEqual(bx(subst_expr(a, sub))?, bx(subst_expr(b, sub))?),
        E::Less(a, b) => E::Less(bx(subst_expr(a, sub))?, bx(subst_expr(b, sub))?),
        E::LessOrEqual(a, b) => E::LessOrEqual(bx(subst_expr(a, sub))?, bx(subst_expr(b, sub))?),
        E::In(a, list) => E::In(
            bx(subst_expr(a, sub))?,
            list.iter().map(|x| subst_expr(x, sub)).collect::<Option<Vec<_>>>()?,
        ),
        E::Add(a, b) => E::Add(bx(subst_expr(a, sub))?, bx(subst_expr(b, sub))?),
        E::Subtract(a, b) => E::Subtract(bx(subst_expr(a, sub))?, bx(subst_expr(b, sub))?),
        E::Multiply(a, b) => E::Multiply(bx(subst_expr(a, sub))?, bx(subst_expr(b, sub))?),
        E::Divide(a, b) => E::Divide(bx(subst_expr(a, sub))?, bx(subst_expr(b, sub))?),
        E::UnaryPlus(a) => E::UnaryPlus(bx(subst_expr(a, sub))?),
        E::UnaryMinus(a) => E::UnaryMinus(bx(subst_expr(a, sub))?),
        E::Not(a) => E::Not(bx(subst_expr(a, sub))?),
        E::If(c, t, f) => E::If(
            bx(subst_expr(c, sub))?,
            bx(subst_expr(t, sub))?,
            bx(subst_expr(f, sub))?,
        ),
        E::Coalesce(list) => E::Coalesce(list.iter().map(|x| subst_expr(x, sub)).collect::<Option<Vec<_>>>()?),
        E::FunctionCall(f, args) => {
            use spargebra::algebra::Function as F;
            // Volatile or externally supplied behavior must not be duplicated
            // across the optimizer's binding partitions.
            if matches!(f, F::Now | F::Rand | F::Uuid | F::StrUuid | F::BNode | F::Custom(_)) {
                return None;
            }
            E::FunctionCall(f.clone(), args.iter().map(|x| subst_expr(x, sub)).collect::<Option<Vec<_>>>()?)
        },
        E::Exists(_) => return None,
        // `BOUND(?v)` on a substituted (certainly-bound) variable is always true, but
        // rewriting it changes nothing measurable here — bail conservatively rather
        // than synthesise a boolean literal.
        E::Bound(v) if sub.contains_key(v) => return None,
        E::Bound(v) => E::Bound(v.clone()),
    })
}

/// Attempts sideways-information-passing evaluation of `Join(left, right)` given the
/// already-evaluated `left`. Returns `Ok(Some(result))` when correlated evaluation
/// fired (bag-equivalent to the cold join), or `Ok(None)` to fall back to cold. See
/// the module comment for the soundness argument.
pub(super) fn try_sip_join(
    graph: &Graph,
    local: &mut LocalVocab,
    left: &Bindings,
    right: &GraphPattern,
) -> Result<Option<Bindings>, String> {
    if !sip::enabled() || left.rows.is_empty() || left.rows.len() > SIP_MAX_SMALL_ROWS {
        return Ok(None);
    }
    // Variables that are certain in `right` — the only ones safe to substitute.
    let mut cert = FxHashSet::default();
    certain_vars(right, &mut cert);
    if cert.is_empty() {
        return Ok(None);
    }
    // Candidate push columns: a left variable that is certain in `right` AND bound to a
    // NamedNode (IRI) in EVERY left row (term identity keeps substitution exact).
    let mut push: Vec<(Variable, usize, Vec<oxrdf::NamedNode>)> = Vec::new();
    for (col, v) in left.vars.iter().enumerate() {
        if !cert.contains(v) {
            continue;
        }
        let mut iris: Vec<oxrdf::NamedNode> = Vec::with_capacity(left.rows.len());
        let mut ok = true;
        for row in &left.rows {
            match term_of(graph, local, row[col]) {
                Some(Term::NamedNode(n)) => iris.push(n),
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            push.push((v.clone(), col, iris));
        }
    }
    if push.is_empty() {
        return Ok(None);
    }
    // Group left rows by their pushed-value combination `C` (dedup for EVALUATION,
    // never for the join result). `key_of` builds the ordered IRI-string key.
    let key_of = |ri: usize| -> Vec<String> {
        push.iter().map(|(_, _, iris)| iris[ri].as_str().to_string()).collect()
    };
    let mut order: Vec<Vec<String>> = Vec::new();
    let mut groups: FxHashMap<Vec<String>, Vec<usize>> = FxHashMap::default();
    for ri in 0..left.rows.len() {
        let k = key_of(ri);
        match groups.entry(k.clone()) {
            std::collections::hash_map::Entry::Occupied(mut e) => e.get_mut().push(ri),
            std::collections::hash_map::Entry::Vacant(e) => {
                order.push(k);
                e.insert(vec![ri]);
            }
        }
    }

    let mut result: Option<Bindings> = None;
    let mut total_child_rows = 0usize;
    for key in &order {
        let members = &groups[key];
        // Build the var -> IRI substitution for this C from the first member row.
        let ri0 = members[0];
        let mut smap: FxHashMap<Variable, oxrdf::NamedNode> = FxHashMap::default();
        for (v, _, iris) in &push {
            smap.insert(v.clone(), iris[ri0].clone());
        }
        // Substitute into `right`; a non-substitutable construct aborts SIP entirely
        // (fall back to the cold join for the whole operator, bit-for-bit).
        let Some(subst) = subst_pattern(right, &smap) else {
            return Ok(None);
        };
        let b_prime = eval_graph_pattern(graph, local, &subst)?;
        total_child_rows += b_prime.rows.len();
        // `A_C`: the left rows whose pushed values equal this C (multiplicity intact).
        let a_c = Bindings::unsorted(
            left.vars.clone(),
            members.iter().map(|&ri| left.rows[ri].clone()).collect(),
        );
        let joined = join_bindings(a_c, b_prime);
        result = Some(match result {
            None => joined,
            Some(acc) => union_bindings(acc, joined),
        });
    }
    sip::record(total_child_rows, order.len());
    Ok(result)
}
