// sq-sg542 (epic sq-pbz04.3 / sq-6tykl):
// Integration tests for the BRANCH-AWARE emitter — per-branch FILTER/VALUES emission for a
// multi-branch UCQ. These shapes (`?x :p1|:p2 ?y FILTER(?x != :Bad)` after the #1671 alternation
// desugaring, a hand-written `{ … FILTER } UNION { … FILTER }`, per-branch VALUES) were previously
// REJECTED fail-closed because the single-passthrough emitter would hoist branch[0]'s modifier over
// the WHOLE union. The branch-aware emitter now answers them, each branch carrying ITS OWN modifier.
//
// SOUNDNESS INVARIANTS proven here (the load-bearing tests):
//   1. RESULT-EQUIVALENCE — the alternation+FILTER form rewrites to the same PROJECTED answer set
//      as its hand-desugared `{ … FILTER } UNION { … FILTER }` form, and both equal a hand-derived
//      oracle (evaluated over a concrete ABox by a faithful BGP/Union/Filter/Join/Values matcher).
//   2. NO CROSS-BRANCH LEAK — a FILTER/VALUES owned by branch i constrains branch i ALONE. Probed
//      in BOTH directions: a branch-0 filter that would (if leaked) delete an answer branch 1
//      legitimately contributes, and the symmetric branch-1 case.
//   3. MODIFIER NOT DROPPED — each branch's own modifier is actually applied to its own sub-union
//      (an answer excluded by a branch's own filter never reappears through that same branch).
//   4. rewrite_production PARITY — the minimised production path returns the same answers.
//   5. STILL-REJECTED shapes stay fail-closed (recursive path + FILTER; non-distinguished FILTER).
//
// The evaluator compares only the PROJECTED (distinguished) answers, so it is invariant to how
// non-distinguished positions are named — a faithful evaluation of the emitted query, not a
// re-derivation of the expected answer.

#[cfg(not(feature = "experimental"))]
#[test]
fn branch_aware_emit_skipped_without_experimental() {
    eprintln!(
        "SKIP: branch-aware per-branch FILTER/VALUES tests require the `experimental` feature — \
         build with `--features experimental` to run them."
    );
}

#[cfg(feature = "experimental")]
mod gated {
    use oxrdf::Triple;
    use spargebra::algebra::{Expression, GraphPattern};
    use spargebra::term::{NamedNodePattern, TermPattern};
    use spargebra::{Query, SparqlParser};
    use std::collections::{BTreeMap, BTreeSet};
    use std::str::FromStr;

    use sparq_reason_ql::{rewrite, rewrite_production, CqError};

    const PRE: &str = "PREFIX : <http://ex/> \
                       PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>";

    fn q(s: &str) -> Query {
        SparqlParser::new().parse_query(s).unwrap()
    }

    fn nt(lines: &[&str]) -> Vec<Triple> {
        lines.iter().map(|l| Triple::from_str(l).unwrap()).collect()
    }

    fn iri(local: &str) -> String {
        format!("<http://ex/{local}>")
    }

    // ---- Faithful UCQ evaluator (Bgp / Union / Join / Filter / Values / Project / Distinct) ----

    enum Slot {
        Fixed(String),
        Bind(String),
    }

    fn term_slot(t: &TermPattern) -> Slot {
        match t {
            TermPattern::NamedNode(n) => Slot::Fixed(n.to_string()),
            TermPattern::Literal(l) => Slot::Fixed(l.to_string()),
            TermPattern::Variable(v) => Slot::Bind(format!("?{}", v.as_str())),
            TermPattern::BlankNode(b) => Slot::Bind(format!("_:{}", b.as_str())),
            #[allow(unreachable_patterns)]
            other => panic!("unexpected term pattern in rewritten UCQ: {other:?}"),
        }
    }

    fn pred_slot(p: &NamedNodePattern) -> Slot {
        match p {
            NamedNodePattern::NamedNode(n) => Slot::Fixed(n.to_string()),
            NamedNodePattern::Variable(v) => Slot::Bind(format!("?{}", v.as_str())),
        }
    }

    fn bind(sol: &BTreeMap<String, String>, slot: Slot, value: &str) -> Option<BTreeMap<String, String>> {
        match slot {
            Slot::Fixed(f) => (f == value).then(|| sol.clone()),
            Slot::Bind(k) => match sol.get(&k) {
                Some(existing) if existing != value => None,
                _ => {
                    let mut next = sol.clone();
                    next.insert(k, value.to_string());
                    Some(next)
                }
            },
        }
    }

    fn triples_as_strings(data: &[Triple]) -> Vec<(String, String, String)> {
        data.iter()
            .map(|t| (t.subject.to_string(), t.predicate.to_string(), t.object.to_string()))
            .collect()
    }

    /// The string value of a term-valued expression (Variable / NamedNode / Literal), resolving a
    /// variable against the current solution. Returns `None` for an unbound variable.
    fn term_value(e: &Expression, sol: &BTreeMap<String, String>) -> Option<String> {
        match e {
            Expression::Variable(v) => sol.get(&format!("?{}", v.as_str())).cloned(),
            Expression::NamedNode(n) => Some(n.to_string()),
            Expression::Literal(l) => Some(l.to_string()),
            other => panic!("unexpected term-valued expression in test filter: {other:?}"),
        }
    }

    /// Effective boolean value of a FILTER expression over a fully-bound solution. Supports exactly
    /// the operators the tests use (`=`, `!=` as `Not(Equal)`, `&&`, `||`, `IN`, `BOUND`); an
    /// unsupported operator panics LOUD so a surprise construct can never silently pass a filter.
    fn eval_bool(e: &Expression, sol: &BTreeMap<String, String>) -> bool {
        match e {
            Expression::Equal(a, b) => term_value(a, sol) == term_value(b, sol),
            Expression::Not(a) => !eval_bool(a, sol),
            Expression::And(a, b) => eval_bool(a, sol) && eval_bool(b, sol),
            Expression::Or(a, b) => eval_bool(a, sol) || eval_bool(b, sol),
            Expression::In(inner, list) => {
                let v = term_value(inner, sol);
                list.iter().any(|item| term_value(item, sol) == v)
            }
            Expression::Bound(v) => sol.contains_key(&format!("?{}", v.as_str())),
            other => panic!("unsupported filter expression in test: {other:?}"),
        }
    }

    fn eval(gp: &GraphPattern, data: &[(String, String, String)]) -> Vec<BTreeMap<String, String>> {
        match gp {
            GraphPattern::Bgp { patterns } => {
                let mut sols = vec![BTreeMap::new()];
                for pat in patterns {
                    let mut next = Vec::new();
                    for sol in &sols {
                        for (s, p, o) in data {
                            let Some(a) = bind(sol, term_slot(&pat.subject), s) else { continue };
                            let Some(b) = bind(&a, pred_slot(&pat.predicate), p) else { continue };
                            if let Some(c) = bind(&b, term_slot(&pat.object), o) {
                                next.push(c);
                            }
                        }
                    }
                    sols = next;
                }
                sols
            }
            GraphPattern::Join { left, right } => {
                let ls = eval(left, data);
                let rs = eval(right, data);
                let mut out = Vec::new();
                for l in &ls {
                    for r in &rs {
                        let mut merged = l.clone();
                        let mut ok = true;
                        for (k, v) in r {
                            match merged.get(k) {
                                Some(existing) if existing != v => {
                                    ok = false;
                                    break;
                                }
                                _ => {
                                    merged.insert(k.clone(), v.clone());
                                }
                            }
                        }
                        if ok {
                            out.push(merged);
                        }
                    }
                }
                out
            }
            GraphPattern::Union { left, right } => {
                let mut sols = eval(left, data);
                sols.extend(eval(right, data));
                sols
            }
            GraphPattern::Filter { expr, inner } => eval(inner, data)
                .into_iter()
                .filter(|sol| eval_bool(expr, sol))
                .collect(),
            GraphPattern::Values { variables, bindings } => bindings
                .iter()
                .map(|row| {
                    let mut sol = BTreeMap::new();
                    for (var, cell) in variables.iter().zip(row.iter()) {
                        if let Some(gt) = cell {
                            sol.insert(format!("?{}", var.as_str()), gt.to_string());
                        }
                    }
                    sol
                })
                .collect(),
            GraphPattern::Project { inner, variables } => {
                let keep: BTreeSet<String> =
                    variables.iter().map(|v| format!("?{}", v.as_str())).collect();
                eval(inner, data)
                    .into_iter()
                    .map(|sol| sol.into_iter().filter(|(k, _)| keep.contains(k)).collect())
                    .collect()
            }
            GraphPattern::Distinct { inner }
            | GraphPattern::Reduced { inner }
            | GraphPattern::Slice { inner, .. } => eval(inner, data),
            other => panic!("unexpected graph pattern in rewritten UCQ: {other:?}"),
        }
    }

    fn top_pattern(query: &Query) -> &GraphPattern {
        match query {
            Query::Select { pattern, .. } | Query::Ask { pattern, .. } => pattern,
            other => panic!("rewritten query is neither SELECT nor ASK: {other:?}"),
        }
    }

    fn projection(query: &Query) -> Vec<String> {
        fn find(gp: &GraphPattern) -> Option<Vec<String>> {
            match gp {
                GraphPattern::Project { variables, .. } => {
                    Some(variables.iter().map(|v| format!("?{}", v.as_str())).collect())
                }
                GraphPattern::Distinct { inner }
                | GraphPattern::Reduced { inner }
                | GraphPattern::Slice { inner, .. } => find(inner),
                _ => None,
            }
        }
        find(top_pattern(query)).expect("rewritten SELECT must carry a Project node")
    }

    fn answers(query: &Query, data: &[Triple]) -> BTreeSet<Vec<String>> {
        let strs = triples_as_strings(data);
        let vars = projection(query);
        eval(top_pattern(query), &strs)
            .into_iter()
            .map(|sol| {
                vars.iter()
                    .map(|v| sol.get(v).cloned().unwrap_or_else(|| "<<unbound>>".into()))
                    .collect()
            })
            .collect()
    }

    fn singletons(locals: &[&str]) -> BTreeSet<Vec<String>> {
        locals.iter().map(|l| vec![iri(l)]).collect()
    }

    /// Rewrite BOTH `a` and `b` under `tbox`, evaluate over `abox`, assert their projected answer
    /// sets are EQUAL and equal to `oracle`. Also asserts `rewrite_production` matches (PARITY).
    fn assert_equivalent(
        a: &str,
        b: &str,
        tbox: &[Triple],
        abox: &[Triple],
        oracle: BTreeSet<Vec<String>>,
    ) {
        let ra = rewrite(&q(a), tbox).expect("form A must rewrite");
        let rb = rewrite(&q(b), tbox).expect("form B must rewrite");
        let aa = answers(&ra.query, abox);
        let ab = answers(&rb.query, abox);
        assert_eq!(
            aa, ab,
            "RESULT DIVERGENCE between the two forms\n A: {}\n B: {}\n A answers: {:?}\n B answers: {:?}",
            ra.query, rb.query, aa, ab
        );
        assert_eq!(aa, oracle, "form A must match the hand-derived oracle\n rewritten: {}\n got: {:?}", ra.query, aa);
        // PARITY: the minimised production path returns the same answers.
        let rap = rewrite_production(&q(a), tbox).expect("form A must rewrite (production)");
        assert_eq!(
            answers(&rap.query, abox),
            oracle,
            "rewrite_production diverged from the oracle for form A\n rewritten: {}",
            rap.query
        );
    }

    // ---- 1. Alternation + FILTER: result-equivalent to the hand-desugared UNION+FILTER form ----

    #[test]
    fn alternation_filter_result_equivalent_to_hand_union() {
        // TBox `:sub ⊑ :p1` makes PerfectRef fire on the p1 branch (so the rewrite is non-trivial).
        let tbox = nt(&["<http://ex/sub> <http://www.w3.org/2000/01/rdf-schema#subPropertyOf> <http://ex/p1> ."]);
        let abox = nt(&[
            "<http://ex/x1> <http://ex/p1> <http://ex/y1> .",  // p1 → x1
            "<http://ex/x2> <http://ex/sub> <http://ex/y2> .", // sub ⊑ p1 → x2 (FILTERED OUT)
            "<http://ex/x3> <http://ex/p2> <http://ex/y3> .",  // p2 → x3
        ]);
        // `?x :p1|:p2 ?y FILTER(?x != :x2)` — the FILTER is distributed into BOTH branches by #1671.
        // Oracle: {x1, x3}; x2 (a certain p1 via sub) is removed by the per-branch FILTER.
        assert_equivalent(
            &format!("{PRE} SELECT ?x WHERE {{ ?x :p1|:p2 ?y FILTER(?x != :x2) }}"),
            &format!(
                "{PRE} SELECT ?x WHERE {{ \
                   {{ ?x :p1 ?y FILTER(?x != :x2) }} UNION {{ ?x :p2 ?y FILTER(?x != :x2) }} }}"
            ),
            &tbox,
            &abox,
            singletons(&["x1", "x3"]),
        );
    }

    // ---- 2. Hand-written UNION with DIFFERENT per-branch FILTERs (+ embedded leak witness) ----

    #[test]
    fn hand_union_distinct_per_branch_filters() {
        // Branch A filters `?x != :bad`; branch B filters `?x != :worse`. `:bad` is BOTH an :A and a
        // :B: branch A must EXCLUDE it (its own filter), but branch B must KEEP it (branch A's filter
        // must NOT leak onto branch B). Oracle: {good, ok, bad} — `bad` present ⟹ no branch-A leak.
        let abox = nt(&[
            "<http://ex/bad>  <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/A> .",
            "<http://ex/good> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/A> .",
            "<http://ex/worse> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/B> .",
            "<http://ex/ok>   <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/B> .",
            "<http://ex/bad>  <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/B> .",
        ]);
        let query = format!(
            "{PRE} SELECT ?x WHERE {{ \
               {{ ?x rdf:type :A FILTER(?x != :bad) }} \
               UNION \
               {{ ?x rdf:type :B FILTER(?x != :worse) }} }}"
        );
        // Equivalent to itself (self-consistency across rewrite + production) + oracle.
        assert_equivalent(&query, &query, &[], &abox, singletons(&["good", "ok", "bad"]));
    }

    // ---- 3. VALUES per branch ----

    #[test]
    fn per_branch_values_applied_only_to_its_branch() {
        // Branch B carries `VALUES ?x { :c }`. `:a1` (an :A) must survive branch A even though it is
        // NOT in branch B's VALUES set — i.e. the VALUES must not leak onto branch A. `:d` (a :B not
        // in the VALUES) must be excluded by branch B's own VALUES. Oracle: {a1, c}.
        let abox = nt(&[
            "<http://ex/a1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/A> .",
            "<http://ex/c>  <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/B> .",
            "<http://ex/d>  <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/B> .",
        ]);
        let query = format!(
            "{PRE} SELECT ?x WHERE {{ \
               {{ ?x rdf:type :A }} UNION {{ ?x rdf:type :B VALUES ?x {{ :c }} }} }}"
        );
        assert_equivalent(&query, &query, &[], &abox, singletons(&["a1", "c"]));
    }

    // ---- 4. LEAK PROBES (both directions), each a dedicated differential ----

    #[test]
    fn leak_probe_branch0_filter_does_not_constrain_branch1() {
        // `{ ?x a :A FILTER(?x != :bad) } UNION { ?x a :B }`. Data: `bad` is both :A and :B; `good`
        // is :A. Branch 0 excludes `bad` (own filter); branch 1 (unfiltered :B) contributes `bad`.
        // If branch-0's filter LEAKED over the union, `bad` would vanish. Oracle: {good, bad}.
        let abox = nt(&[
            "<http://ex/bad>  <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/A> .",
            "<http://ex/bad>  <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/B> .",
            "<http://ex/good> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/A> .",
        ]);
        let query = format!(
            "{PRE} SELECT ?x WHERE {{ \
               {{ ?x rdf:type :A FILTER(?x != :bad) }} UNION {{ ?x rdf:type :B }} }}"
        );
        let r = rewrite(&q(&query), &[]).expect("must rewrite");
        assert_eq!(
            answers(&r.query, &abox),
            singletons(&["good", "bad"]),
            "branch-0 FILTER must NOT constrain branch 1 (bad must still appear via :B)\n rewritten: {}",
            r.query
        );
    }

    #[test]
    fn leak_probe_branch1_filter_does_not_constrain_branch0() {
        // Symmetric reverse: `{ ?x a :A } UNION { ?x a :B FILTER(?x != :bad) }`. Branch 0 (unfiltered
        // :A) contributes `bad`; branch 1 excludes it. If branch-1's filter LEAKED onto branch 0,
        // `bad` would vanish. Oracle: {bad, good}.
        let abox = nt(&[
            "<http://ex/bad>  <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/A> .",
            "<http://ex/bad>  <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/B> .",
            "<http://ex/good> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/B> .",
        ]);
        let query = format!(
            "{PRE} SELECT ?x WHERE {{ \
               {{ ?x rdf:type :A }} UNION {{ ?x rdf:type :B FILTER(?x != :bad) }} }}"
        );
        let r = rewrite(&q(&query), &[]).expect("must rewrite");
        assert_eq!(
            answers(&r.query, &abox),
            singletons(&["bad", "good"]),
            "branch-1 FILTER must NOT constrain branch 0 (bad must still appear via :A)\n rewritten: {}",
            r.query
        );
    }

    // ---- 5. STILL-REJECTED shapes stay fail-closed ----

    #[test]
    fn recursive_path_with_filter_still_rejected() {
        // A RECURSIVE path is rejected at the gate BEFORE any emitter concern — even with a
        // distinguished-only FILTER. The branch-aware emitter does not weaken this.
        let query = q(&format!("{PRE} SELECT ?x WHERE {{ ?x :r+ ?y FILTER(?x != :bad) }}"));
        let err = rewrite(&query, &[]).unwrap_err();
        assert!(
            matches!(err, CqError::OutOfScope(ref r) if r.contains("property path")),
            "recursive path + FILTER must stay rejected fail-closed; got {:?}",
            err
        );
    }

    #[test]
    fn non_distinguished_branch_filter_still_rejected() {
        // A per-branch FILTER over a NON-distinguished variable is still rejected (B3 fail-closed):
        // its value on an anonymous witness is undefined, so it cannot be a per-branch modifier.
        let query = q(&format!(
            "{PRE} SELECT ?x WHERE {{ \
               {{ ?x rdf:type :A }} \
               UNION \
               {{ ?x :r ?y FILTER(?y != :bad) }} }}"
        ));
        let err = rewrite(&query, &[]).unwrap_err();
        assert!(
            matches!(err, CqError::OutOfScope(ref r) if r.contains("non-distinguished") || r.contains("FILTER")),
            "non-distinguished per-branch FILTER must stay rejected; got {:?}",
            err
        );
    }
}
