use super::*;

// ---- BGP evaluation ----------------------------------------------------------

/// Evaluates a basic graph pattern, dispatching between a binary join plan and a
/// worst-case-optimal (Leapfrog Triejoin) plan. Cyclic BGPs — where binary plans
/// can blow up to an intermediate result far larger than the final answer (e.g.
/// triangles) — go to WCOJ, which runs in time `Õ(AGM bound)` and so is provably
/// optimal in the worst case. Acyclic (tree-shaped) BGPs use binary joins, which
/// are optimal for them and avoid LFTJ's per-tuple overhead. Both paths are
/// differentially tested to produce identical results.
/// Evaluate a SPARQL property path `subject <path> object` into bindings over its endpoint
/// variables. Computes the path's (start,end) id-pair relation recursively — transitive `+`/`*`
/// via BFS, zero-length `*`/`?` add identity over the graph's nodes — then constrains it by any
/// bound or repeated endpoint. Bound endpoints are PUSHED DOWN into the relation computation
/// ([`PathEnds`]): a bound subject turns `+`/`*` into a single-source directed BFS over sorted
/// range scans, a bound object traverses in reverse, and both-bound is a reachability test with
/// early exit — so `:s p+ ?x` costs `O(edges reachable)`, not the all-pairs closure. The
/// post-filter below stays as the correctness backstop (the pushdown contract allows supersets).
pub(super) fn eval_path(
    graph: &Graph,
    local: &mut LocalVocab,
    subject: &TermPattern,
    path: &PropertyPathExpression,
    object: &TermPattern,
) -> Result<Bindings, String> {
    enum End {
        Var(Variable),
        Bound(Id),
        /// A concrete term ABSENT from the dictionary — unsatisfiable for ordinary
        /// paths, but still the start of a zero-length solution for `p*` / `p?`.
        Missing(Term),
    }
    let resolve = |t: &TermPattern| -> Result<End, String> {
        Ok(match t {
            TermPattern::Variable(v) => End::Var(v.clone()),
            TermPattern::BlankNode(b) => End::Var(bnode_var(b)),
            other => {
                let term = term_pattern_to_term(other)?;
                match graph.id_of(&term) {
                    Some(id) => End::Bound(id),
                    None => End::Missing(term),
                }
            }
        })
    };
    let (s_end, o_end) = (resolve(subject)?, resolve(object)?);
    // Paths that admit the ZERO-LENGTH solution connect every term to itself — even
    // terms that do not occur in the data (a constant endpoint on an empty graph).
    let zero_len = matches!(path, PropertyPathExpression::ZeroOrMore(_) | PropertyPathExpression::ZeroOrOne(_));

    let s_var = if let End::Var(v) = &s_end { Some(v.clone()) } else { None };
    let o_var = if let End::Var(v) = &o_end { Some(v.clone()) } else { None };
    let same_var = matches!((&s_var, &o_var), (Some(a), Some(b)) if a == b);
    let mut vars: Vec<Variable> = Vec::new();
    if let Some(v) = &s_var {
        vars.push(v.clone());
    }
    if let Some(v) = &o_var {
        if !same_var {
            vars.push(v.clone());
        }
    }
    // A concrete-but-absent endpoint makes the pattern unsatisfiable — unless the
    // path has a zero-length solution, handled below.
    if !zero_len && (matches!(s_end, End::Missing(_)) || matches!(o_end, End::Missing(_))) {
        return Ok(Bindings::unsorted(vars, Vec::new()));
    }
    let s_bound = if let End::Bound(id) = &s_end { Some(*id) } else { None };
    let o_bound = if let End::Bound(id) = &o_end { Some(*id) } else { None };

    let mut rows: Vec<Row> = Vec::new();
    let mut seen: FxHashSet<Row> = FxHashSet::default();
    // DefaultGraphMode::Empty (L1 dataset view): no data pairs at top-level graph
    // scope — exactly the empty-graph evaluation. The zero-length constant
    // solutions below still apply (`<s> p* <s>` holds even on an empty graph).
    if !(view::default_is_empty() || matches!(s_end, End::Missing(_)) || matches!(o_end, End::Missing(_))) {
        let ends = PathEnds { s: s_bound, o: o_bound };
        // `?x p ?x` (same variable at both ends — necessarily both unbound): only
        // diagonal pairs survive the filter, and for the recursive operators the
        // diagonal is computable WITHOUT the all-pairs closure: the zero-length
        // operators' diagonal is exactly the node domain, and `p+`'s diagonal is
        // the set of nodes on a directed cycle (SCC size >= 2, or a self-loop).
        let pairs: FxHashSet<(Id, Id)> = if same_var {
            match path {
                PropertyPathExpression::ZeroOrMore(_) | PropertyPathExpression::ZeroOrOne(_) => {
                    graph_nodes(graph).into_iter().map(|n| (n, n)).collect()
                }
                PropertyPathExpression::OneOrMore(a) => {
                    cyclic_nodes(graph, a)?.into_iter().map(|n| (n, n)).collect()
                }
                _ => path_pairs(graph, path, ends)?,
            }
        } else {
            path_pairs(graph, path, ends)?
        };
        for (s, o) in pairs {
            if s_bound.is_some_and(|b| s != b) || o_bound.is_some_and(|b| o != b) || (same_var && s != o) {
                continue;
            }
            let mut row: Row = SmallVec::new();
            if s_var.is_some() {
                row.push(s);
            }
            if o_var.is_some() && !same_var {
                row.push(o);
            }
            if seen.insert(row.clone()) {
                rows.push(row); // property-path solutions are a set (DISTINCT)
            }
        }
    }
    if zero_len {
        // The zero-length solution for a CONSTANT endpoint: `<s> p* ?x` yields
        // {?x -> <s>} even on the empty graph (interning the absent term locally);
        // `<s> p* <s>` yields the unit solution. (Variable–variable zero-length
        // solutions over the graph's nodes come from `path_pairs` above.)
        let const_id = |e: &End, local: &mut LocalVocab| match e {
            End::Bound(id) => Some(*id),
            End::Missing(t) => Some(local.intern(t.clone())),
            End::Var(_) => None,
        };
        let zrow: Option<Row> = match (&s_end, &o_end) {
            (End::Var(_), End::Var(_)) => None,
            (s_c, End::Var(_)) => const_id(s_c, local).map(|id| std::iter::once(id).collect()),
            (End::Var(_), o_c) => const_id(o_c, local).map(|id| std::iter::once(id).collect()),
            (s_c, o_c) => {
                let (a, b) = (const_id(s_c, local), const_id(o_c, local));
                (a == b).then(SmallVec::new)
            }
        };
        if let Some(row) = zrow {
            if seen.insert(row.clone()) {
                rows.push(row);
            }
        }
    }
    Ok(Bindings::unsorted(vars, rows))
}

/// Endpoint constraints pushed down into a path-relation computation (`None` =
/// that end is unbound). CONTRACT: `path_pairs(graph, path, ends)` returns a
/// SUBSET of the path's full (start,end) relation that contains EVERY pair
/// satisfying the bounds. A sub-evaluation is free to IGNORE the hint and
/// return extra relation pairs (callers always post-filter), but must never
/// invent pairs outside the relation — so the pushdown is purely an
/// optimisation and the post-filter in `eval_path` is the correctness backstop.
#[derive(Clone, Copy, Default)]
pub(super) struct PathEnds {
    pub(super) s: Option<Id>,
    pub(super) o: Option<Id>,
}

impl PathEnds {
    pub(super) const NONE: PathEnds = PathEnds { s: None, o: None };
    /// The constraint seen through `^path` (endpoints exchange roles).
    #[inline]
    pub(super) fn swapped(self) -> PathEnds {
        PathEnds { s: self.o, o: self.s }
    }
}

/// When a `Sequence` has a bound outer endpoint, the midpoints reached by the
/// near hop are pushed one at a time into the far hop — but only while the
/// fan-out stays below this limit. Above it, the per-midpoint sub-evaluations
/// (each at least a binary search; a whole traversal for a recursive hop)
/// can exceed the single bulk evaluation they replace, so the far hop then
/// DELIBERATELY gets only the outer endpoint pushed and the midpoints meet in
/// the hash join instead.
pub(super) const SEQ_MIDPOINT_FANOUT_LIMIT: usize = 1024;

/// All `(subject, object)` id pairs connected by a property path expression,
/// narrowed by any bound endpoints (see [`PathEnds`] for the exact contract).
/// Bound endpoints reach the leaves as range-scan prefixes and turn the
/// recursive operators into single-source directed traversals.
pub(super) fn path_pairs(graph: &Graph, path: &PropertyPathExpression, ends: PathEnds) -> Result<FxHashSet<(Id, Id)>, String> {
    use PropertyPathExpression as P;
    Ok(match path {
        P::NamedNode(p) => predicate_pairs(graph, p, ends),
        P::Reverse(a) => path_pairs(graph, a, ends.swapped())?.into_iter().map(|(s, o)| (o, s)).collect(),
        P::Sequence(a, c) => {
            if let Some(s) = ends.s {
                // Bound start: evaluate the near hop from `s` only, then push each
                // reached midpoint into the far hop (which also receives the bound
                // object, enabling early exit deeper down).
                let av = path_pairs(graph, a, PathEnds { s: Some(s), o: None })?;
                let mids: FxHashSet<Id> = av.iter().filter(|&&(x, _)| x == s).map(|&(_, m)| m).collect();
                if mids.len() <= SEQ_MIDPOINT_FANOUT_LIMIT {
                    let mut out = FxHashSet::default();
                    for &m in &mids {
                        for (m2, o) in path_pairs(graph, c, PathEnds { s: Some(m), o: ends.o })? {
                            if m2 == m {
                                out.insert((s, o));
                            }
                        }
                    }
                    out
                } else {
                    // Fan-out too large for per-midpoint pushes: the far hop gets
                    // only the outer bound endpoint.
                    join_seq(av, path_pairs(graph, c, PathEnds { s: None, o: ends.o })?)
                }
            } else if let Some(o) = ends.o {
                // Bound object only: mirror image — far hop backwards from `o`,
                // midpoints pushed into the near hop as bound objects.
                let cv = path_pairs(graph, c, PathEnds { s: None, o: Some(o) })?;
                let mids: FxHashSet<Id> = cv.iter().filter(|&&(_, y)| y == o).map(|&(m, _)| m).collect();
                if mids.len() <= SEQ_MIDPOINT_FANOUT_LIMIT {
                    let mut out = FxHashSet::default();
                    for &m in &mids {
                        for (s, m2) in path_pairs(graph, a, PathEnds { s: None, o: Some(m) })? {
                            if m2 == m {
                                out.insert((s, o));
                            }
                        }
                    }
                    out
                } else {
                    join_seq(path_pairs(graph, a, PathEnds::NONE)?, cv)
                }
            } else {
                join_seq(path_pairs(graph, a, PathEnds::NONE)?, path_pairs(graph, c, PathEnds::NONE)?)
            }
        }
        // Endpoints push into BOTH branches of an alternative unchanged.
        P::Alternative(a, c) => {
            let mut s = path_pairs(graph, a, ends)?;
            s.extend(path_pairs(graph, c, ends)?);
            s
        }
        P::OneOrMore(a) => match (ends.s, ends.o) {
            (Some(s), _) => directed_reach(graph, a, s, ends.o, Dir::Fwd)?.into_iter().map(|r| (s, r)).collect(),
            (None, Some(o)) => directed_reach(graph, a, o, None, Dir::Rev)?.into_iter().map(|r| (r, o)).collect(),
            (None, None) => transitive_closure_pairs(path_pairs(graph, a, PathEnds::NONE)?),
        },
        P::ZeroOrMore(a) => match (ends.s, ends.o) {
            // A bound endpoint needs only ITS reflexive pair, not the whole node
            // domain (`<s> p* <s>` holds for any term, see the zero-length rules).
            (Some(s), _) => {
                let mut c: FxHashSet<(Id, Id)> =
                    directed_reach(graph, a, s, ends.o, Dir::Fwd)?.into_iter().map(|r| (s, r)).collect();
                c.insert((s, s));
                c
            }
            (None, Some(o)) => {
                let mut c: FxHashSet<(Id, Id)> =
                    directed_reach(graph, a, o, None, Dir::Rev)?.into_iter().map(|r| (r, o)).collect();
                c.insert((o, o));
                c
            }
            (None, None) => {
                let mut c = transitive_closure_pairs(path_pairs(graph, a, PathEnds::NONE)?);
                c.extend(graph_nodes(graph).into_iter().map(|n| (n, n)));
                c
            }
        },
        P::ZeroOrOne(a) => {
            let mut s = path_pairs(graph, a, ends)?;
            match (ends.s, ends.o) {
                // Bound endpoint: only its own reflexive pair (no full-store node scan).
                (Some(x), None) | (None, Some(x)) => {
                    s.insert((x, x));
                }
                (Some(x), Some(y)) => {
                    if x == y {
                        s.insert((x, x));
                    }
                }
                (None, None) => s.extend(graph_nodes(graph).into_iter().map(|n| (n, n))),
            }
            s
        }
        P::NegatedPropertySet(props) => negated_property_pairs(graph, props, ends),
    })
}

/// Hash join of two path relations on the shared midpoint (`a.end == c.start`).
pub(super) fn join_seq(av: FxHashSet<(Id, Id)>, cv: FxHashSet<(Id, Id)>) -> FxHashSet<(Id, Id)> {
    let mut by_start: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
    for (m, o) in cv {
        by_start.entry(m).or_default().push(o);
    }
    let mut out = FxHashSet::default();
    for (s, m) in av {
        if let Some(os) = by_start.get(&m) {
            for &o in os {
                out.insert((s, o));
            }
        }
    }
    out
}

/// Traversal direction for [`directed_reach`]: forward follows the sub-path
/// from a bound SUBJECT; reverse walks it backwards from a bound OBJECT (the
/// O-leading permutations answer the reversed leaf scans).
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Dir {
    Fwd,
    Rev,
}

/// Single-source reachability (one or more steps) from `start` under the
/// sub-path: a budget-checked BFS whose frontier expansion is a bounded
/// sub-evaluation — for a plain-predicate sub-path, one sorted range scan
/// (`[Some(n), Some(p), None]`, or `[None, Some(p), Some(n)]` reversed) per
/// node. With `target` bound the walk is a reachability TEST and stops as soon
/// as the target is reached.
pub(super) fn directed_reach(
    graph: &Graph,
    sub: &PropertyPathExpression,
    start: Id,
    target: Option<Id>,
    dir: Dir,
) -> Result<FxHashSet<Id>, String> {
    // Plain-predicate fast path: successors come straight from a range scan
    // (no per-node relation set). `Some(None)` = predicate not in the
    // dictionary, hence no edges at all.
    let pid: Option<Option<Id>> = match sub {
        PropertyPathExpression::NamedNode(p) => Some(graph.id_of(&Term::NamedNode(p.clone()))),
        _ => None,
    };
    let step = |node: Id, out: &mut Vec<Id>| -> Result<(), String> {
        match pid {
            Some(None) => {}
            Some(Some(pid)) => {
                let pat: IdPattern = match dir {
                    Dir::Fwd => [Some(node), Some(pid), None],
                    Dir::Rev => [None, Some(pid), Some(node)],
                };
                let scan = graph.store.scan(&pat);
                let col = match dir {
                    Dir::Fwd => 2,
                    Dir::Rev => 0,
                };
                out.extend(scan.rows.iter().map(|r| scan.to_spo(r)[col]));
            }
            // Composite sub-path: one bounded sub-evaluation per node (its own
            // endpoints push recursively). The filter enforces the contract's
            // "may return extra relation pairs" clause.
            None => {
                let ends = match dir {
                    Dir::Fwd => PathEnds { s: Some(node), o: None },
                    Dir::Rev => PathEnds { s: None, o: Some(node) },
                };
                for (s, o) in path_pairs(graph, sub, ends)? {
                    match dir {
                        Dir::Fwd if s == node => out.push(o),
                        Dir::Rev if o == node => out.push(s),
                        _ => {}
                    }
                }
            }
        }
        Ok(())
    };
    let mut seen: FxHashSet<Id> = FxHashSet::default();
    let mut stack: Vec<Id> = Vec::new();
    step(start, &mut stack)?;
    let mut pops = 0usize;
    while let Some(n) = stack.pop() {
        // Budget granularity: check INSIDE the walk (every 1024 pops), so one
        // runaway traversal respects the budget promptly.
        pops += 1;
        if pops & 0x3FF == 0 {
            budget::check(seen.len())?;
        }
        if seen.insert(n) {
            if target == Some(n) {
                break;
            }
            step(n, &mut stack)?;
        }
    }
    Ok(seen)
}

/// The nodes lying on a directed cycle of the sub-path's relation — exactly the
/// solutions of `?x p+ ?x` — via Kosaraju SCC over the base relation: every
/// member of an SCC of size >= 2, plus self-loop nodes. `O(V + E)` instead of
/// the all-pairs closure's `O(V·E)`.
pub(super) fn cyclic_nodes(graph: &Graph, sub: &PropertyPathExpression) -> Result<FxHashSet<Id>, String> {
    let base = path_pairs(graph, sub, PathEnds::NONE)?;
    let mut adj: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
    let mut radj: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
    let mut cyclic: FxHashSet<Id> = FxHashSet::default();
    for &(s, o) in &base {
        if s == o {
            cyclic.insert(s); // self-loop: a 1-cycle
        }
        adj.entry(s).or_default().push(o);
        radj.entry(o).or_default().push(s);
    }
    // Pass 1: iterative DFS post-order (every node with an out-edge is a root
    // candidate; sinks are reached as children — a cycle member always has an
    // out-edge, so coverage is complete).
    let mut order: Vec<Id> = Vec::new();
    let mut visited: FxHashSet<Id> = FxHashSet::default();
    let mut steps = 0usize;
    let roots: Vec<Id> = adj.keys().copied().collect();
    for &root in &roots {
        if !visited.insert(root) {
            continue;
        }
        let mut stack: Vec<(Id, usize)> = vec![(root, 0)];
        while let Some(frame) = stack.last_mut() {
            steps += 1;
            if steps & 0x3FF == 0 {
                budget::check(order.len())?;
            }
            let (n, i) = *frame;
            match adj.get(&n).and_then(|v| v.get(i).copied()) {
                Some(m) => {
                    frame.1 += 1;
                    if visited.insert(m) {
                        stack.push((m, 0));
                    }
                }
                None => {
                    order.push(n);
                    stack.pop();
                }
            }
        }
    }
    // Pass 2: components of the TRANSPOSE graph, roots taken in reverse
    // post-order; a component of >= 2 members is a cycle through all of them.
    let mut assigned: FxHashSet<Id> = FxHashSet::default();
    for &root in order.iter().rev() {
        if !assigned.insert(root) {
            continue;
        }
        let mut members: Vec<Id> = vec![root];
        let mut stack: Vec<Id> = vec![root];
        while let Some(n) = stack.pop() {
            steps += 1;
            if steps & 0x3FF == 0 {
                budget::check(members.len())?;
            }
            if let Some(ps) = radj.get(&n) {
                for &m in ps {
                    if assigned.insert(m) {
                        members.push(m);
                        stack.push(m);
                    }
                }
            }
        }
        if members.len() >= 2 {
            cyclic.extend(members);
        }
    }
    Ok(cyclic)
}

/// All `(s, o)` for a single predicate IRI (empty if the predicate isn't in the
/// graph), narrowed by any bound endpoints — `[s?, p, o?]` is always a single
/// contiguous range in some built permutation.
pub(super) fn predicate_pairs(graph: &Graph, p: &oxrdf::NamedNode, ends: PathEnds) -> FxHashSet<(Id, Id)> {
    match graph.id_of(&Term::NamedNode(p.clone())) {
        None => FxHashSet::default(),
        Some(pid) => {
            let pat: IdPattern = [ends.s, Some(pid), ends.o];
            let scan = graph.store.scan(&pat);
            scan.rows
                .iter()
                .map(|r| {
                    let t = scan.to_spo(r);
                    (t[0], t[2])
                })
                .collect()
        }
    }
}

/// `!(...)` — every edge whose predicate is NOT in the excluded set. A bound
/// endpoint narrows the scan to that one node's triples; the fully-unbound case
/// walks a P-leading permutation and skips each excluded predicate's contiguous
/// block wholesale (binary search to the block end — no per-triple set probe
/// over the excluded mass).
pub(super) fn negated_property_pairs(graph: &Graph, props: &[oxrdf::NamedNode], ends: PathEnds) -> FxHashSet<(Id, Id)> {
    let excluded: FxHashSet<Id> = props.iter().filter_map(|p| graph.id_of(&Term::NamedNode(p.clone()))).collect();
    if ends.s.is_some() || ends.o.is_some() {
        let pat: IdPattern = [ends.s, None, ends.o];
        let scan = graph.store.scan(&pat);
        return scan
            .rows
            .iter()
            .filter_map(|r| {
                let t = scan.to_spo(r);
                (!excluded.contains(&t[1])).then_some((t[0], t[2]))
            })
            .collect();
    }
    // Both BUILT index sets contain a P-leading permutation (PSO full / POS
    // compact), so `scan_sorted` finds one; the fallback filter-scan defends
    // against a future index set where it doesn't.
    let scan = graph.store.scan_sorted(&[None, None, None], 1);
    let rows = &scan.rows[..];
    let mut out = FxHashSet::default();
    if scan.perm.order()[0] == 1 {
        let mut i = 0;
        while i < rows.len() {
            let p = rows[i][0];
            let j = i + rows[i..].partition_point(|r| r[0] == p);
            if !excluded.contains(&p) {
                for r in &rows[i..j] {
                    let t = scan.to_spo(r);
                    out.insert((t[0], t[2]));
                }
            }
            i = j;
        }
    } else {
        for r in rows {
            let t = scan.to_spo(r);
            if !excluded.contains(&t[1]) {
                out.insert((t[0], t[2]));
            }
        }
    }
    out
}

/// Every id that appears as a subject or object (the domain of zero-length path matches).
pub(super) fn graph_nodes(graph: &Graph) -> FxHashSet<Id> {
    let pat: IdPattern = [None, None, None];
    let scan = graph.store.scan(&pat);
    let mut s = FxHashSet::default();
    for r in scan.rows.iter() {
        let t = scan.to_spo(r);
        s.insert(t[0]);
        s.insert(t[2]);
    }
    s
}

/// Transitive (NOT reflexive) closure of a pair relation — BFS of reachability from each start.
pub(super) fn transitive_closure_pairs(pairs: FxHashSet<(Id, Id)>) -> FxHashSet<(Id, Id)> {
    let mut adj: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
    for (s, o) in &pairs {
        adj.entry(*s).or_default().push(*o);
    }
    let mut out: FxHashSet<(Id, Id)> = FxHashSet::default();
    let starts: Vec<Id> = adj.keys().copied().collect();
    let mut pops = 0usize;
    for start in starts {
        // Budget check per BFS start node (sticky; the caller's next check
        // raises the error)…
        if budget::exhausted(out.len()) {
            break;
        }
        let mut seen: FxHashSet<Id> = FxHashSet::default();
        let mut stack: Vec<Id> = adj.get(&start).cloned().unwrap_or_default();
        while let Some(n) = stack.pop() {
            // …and every 1024 expansions INSIDE the walk, so one runaway start
            // node cannot overshoot the budget by a whole graph traversal.
            pops += 1;
            if pops & 0x3FF == 0 && budget::exhausted(out.len()) {
                return out;
            }
            if seen.insert(n) {
                out.insert((start, n));
                if let Some(nexts) = adj.get(&n) {
                    stack.extend(nexts.iter().copied());
                }
            }
        }
    }
    out
}

pub(super) fn eval_bgp(graph: &Graph, patterns: &[TriplePattern]) -> Result<Bindings, String> {
    if patterns.is_empty() {
        return Ok(Bindings { vars: vec![], rows: vec![Row::new()], sorted_by: None });
    }
    // DefaultGraphMode::Empty (L1 dataset view): a non-empty BGP at top-level
    // graph scope has ZERO rows, with its normal variable schema (the empty BGP
    // above keeps its unit row; GRAPH scope suspends the flag).
    if view::default_is_empty() {
        return Ok(Bindings::unsorted(collect_vars(patterns), vec![]));
    }
    // RDF 1.2 triple-term patterns with variables decompose into synthetic-variable
    // slots + structural-unification relations, joined by the ordinary machinery (F14).
    let (rewritten, constraints) = extract_quoted_constraints(patterns);
    if !constraints.is_empty() {
        // zk-trace: structural-unification relations scan the store without
        // per-pattern attribution — mark Op::QuotedTriples so a consumer
        // fails closed (ZkTrace::first_uncaptured).
        #[cfg(feature = "zk")]
        let _zk = crate::zk::op_scope(crate::zk::Op::QuotedTriples);
        let mut b = eval_bgp(graph, &rewritten)?;
        for c in &constraints {
            b = join_bindings(b, quoted_relation(graph, c));
        }
        return Ok(b);
    }
    if patterns.len() >= 3 && bgp_is_cyclic(patterns) {
        return eval_bgp_wcoj(graph, patterns);
    }
    // (sq-5zf8i / §A4) Acyclic BGP: optionally run the Yannakakis bottom-up
    // full-semijoin PREPASS before the binary join (opt-in `yannakakis` feature, OFF by
    // default). Routed here, on the SAME acyclic branch that already chooses the binary
    // plan over LFTJ — so cyclic BGPs (handled above) keep the existing LFTJ unchanged.
    // The prepass internally cost-gates and falls back to `eval_bgp_binary` when there is
    // nothing to gain; its result is identical to the binary plan (semijoin reduction is
    // answer-preserving), so with the feature OFF this is byte-identical to before.
    #[cfg(feature = "yannakakis")]
    if patterns.len() >= 2 {
        return eval_bgp_yannakakis(graph, patterns, &[]);
    }
    eval_bgp_binary(graph, patterns, &[])
}
