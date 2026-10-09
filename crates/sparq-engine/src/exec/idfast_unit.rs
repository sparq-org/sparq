//! Differential + unit tests for the id-level `=`/`!=` FILTER fast path.
//! (sq-7d3dj.30.11). The load-bearing invariant: for EVERY row and EVERY operand pair the
//! fast path (path (a) static-non-literal id (in)equality + path (b) equal-id short-circuit)
//! returns the IDENTICAL three-valued verdict as the exact term-materialising path
//! (`term_of` + `values_equal`), which is the ground-truth SPARQL `=` semantics.
use super::*;
use oxrdf::{NamedNode, Variable};

fn var(s: &str) -> Variable {
    Variable::new(s).unwrap()
}
fn nn(s: &str) -> NamedNode {
    NamedNode::new(s).unwrap()
}
fn bgp(tps: Vec<TriplePattern>) -> GraphPattern {
    GraphPattern::Bgp { patterns: tps }
}
fn tp_svo(s: &str, p: &str, o: TermPattern) -> TriplePattern {
    TriplePattern {
        subject: TermPattern::Variable(var(s)),
        predicate: NamedNodePattern::NamedNode(nn(p)),
        object: o,
    }
}

/// GROUND TRUTH for SPARQL `=`: materialise both operand terms and run the exact evaluator.
/// `None` = type error (unbound / cross-family / ill-typed), `Some(b)` = decided boolean.
fn reference_equal(graph: &Graph, local: &LocalVocab, ida: Id, idc: Id) -> Option<bool> {
    let x = match term_of(graph, local, ida) {
        Some(t) => Value::Term(t),
        None => Value::Unbound,
    };
    let y = match term_of(graph, local, idc) {
        Some(t) => Value::Term(t),
        None => Value::Unbound,
    };
    values_equal(&x, &y)
}

/// Run a `?a OP ?b` FILTER through the FULL compiled pipeline WITH the fast-path rewrite
/// applied for the given non-literal column set, returning the per-row three-valued verdict.
fn eval_filter_fast(
    graph: &Graph,
    local: &LocalVocab,
    b: &Bindings,
    expr: &Expression,
    nonlit_cols: &FxHashSet<usize>,
) -> Vec<Option<bool>> {
    let mut compiled = compile_expr(expr, b, local);
    idfast_rewrite(&mut compiled, nonlit_cols);
    b.rows
        .iter()
        .map(|row| ebv3(&eval_compiled(graph, local, b, row, &compiled).unwrap(), crate::EbvSemantics::Rec2013))
        .collect()
}

/// A three-column table of ids: `?a`, `?b` (both compared), `?s` (an IRI subject anchor).
/// Columns 0 and 1 are the compared operands; the test varies their id contents.
fn table(rows: &[[Id; 2]]) -> Bindings {
    Bindings::unsorted(
        vec![var("a"), var("b")],
        rows.iter().map(|r| Row::from_slice(&[r[0], r[1]])).collect(),
    )
}

fn eq_expr() -> Expression {
    Expression::Equal(Box::new(Expression::Variable(var("a"))), Box::new(Expression::Variable(var("b"))))
}
fn ne_expr() -> Expression {
    Expression::Not(Box::new(eq_expr()))
}
fn eq_expr_xy() -> Expression {
    Expression::Equal(Box::new(Expression::Variable(var("x"))), Box::new(Expression::Variable(var("y"))))
}

/// A literal-heavy, mixed graph: IRIs, blank nodes, and literals including numeric-promotion
/// pairs, whitespace-padded numerics, language tags, and cross-datatype pairs.
fn mixed_graph() -> Graph {
    let nt = concat!(
        "<http://ex/s1> <http://ex/p> <http://ex/o1> .\n",
        "<http://ex/s2> <http://ex/p> _:b1 .\n",
        "<http://ex/s3> <http://ex/p> \"1\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
        "<http://ex/s4> <http://ex/p> \"1.0\"^^<http://www.w3.org/2001/XMLSchema#decimal> .\n",
        "<http://ex/s5> <http://ex/p> \" 1 \"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
        "<http://ex/s6> <http://ex/p> \"hi\" .\n",
        "<http://ex/s7> <http://ex/p> \"hi\"@en .\n",
        "<http://ex/s8> <http://ex/p> \"1.0\"^^<http://www.w3.org/2001/XMLSchema#double> .\n",
        "<http://ex/s9> <http://ex/p> \"01\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
        "<http://ex/s10> <http://ex/p> \"foo\"^^<http://ex/weird> .\n",
    );
    Graph::load_str(nt, "ntriples").unwrap()
}

/// Every object id in the mixed graph, so the differential test can pair each id with each.
fn object_ids(g: &Graph) -> Vec<Id> {
    let mk = |t: Term| g.id_of(&t).unwrap();
    vec![
        mk(Term::NamedNode(nn("http://ex/o1"))),
        mk(Term::Literal(Literal::new_typed_literal("1", xsd::INTEGER))),
        mk(Term::Literal(Literal::new_typed_literal("1.0", xsd::DECIMAL))),
        mk(Term::Literal(Literal::new_typed_literal(" 1 ", xsd::INTEGER))),
        mk(Term::Literal(Literal::new_simple_literal("hi"))),
        mk(Term::Literal(Literal::new_language_tagged_literal("hi", "en").unwrap())),
        mk(Term::Literal(Literal::new_typed_literal("1.0", xsd::DOUBLE))),
        mk(Term::Literal(Literal::new_typed_literal("01", xsd::INTEGER))),
        mk(Term::Literal(Literal::new_typed_literal("foo", nn("http://ex/weird")))),
    ]
}

// ---- Static analysis (nonliteral_vars) -------------------------------------------------

#[test]
fn nonliteral_vars_subject_and_predicate_are_nonliteral() {
    // ?s in subject, ?p in predicate, ?o in object.
    let p = bgp(vec![TriplePattern {
        subject: TermPattern::Variable(var("s")),
        predicate: NamedNodePattern::Variable(var("p")),
        object: TermPattern::Variable(var("o")),
    }]);
    let nl = nonliteral_vars(&p, &|_| true);
    assert!(nl.contains(&var("s")), "subject var is non-literal");
    assert!(nl.contains(&var("p")), "predicate var is non-literal");
    assert!(!nl.contains(&var("o")), "object var CAN be a literal");
}

#[test]
fn nonliteral_vars_subject_then_object_is_literal_risk() {
    // ?x in subject of pattern 1 AND object of pattern 2 -> could be a literal -> EXCLUDED.
    let p = bgp(vec![
        tp_svo("x", "http://ex/p", TermPattern::Variable(var("y"))),
        tp_svo("z", "http://ex/q", TermPattern::Variable(var("x"))),
    ]);
    let nl = nonliteral_vars(&p, &|_| true);
    assert!(!nl.contains(&var("x")), "a var used as an OBJECT anywhere is literal-risk");
    assert!(nl.contains(&var("z")), "?z only ever a subject -> non-literal");
}

#[test]
fn nonliteral_vars_bind_and_values_are_literal_risk() {
    let inner = bgp(vec![tp_svo("s", "http://ex/p", TermPattern::Variable(var("o")))]);
    let ext = GraphPattern::Extend {
        inner: Box::new(inner),
        variable: var("s"), // BIND target shadows the subject -> literal-risk wins.
        expression: Expression::Literal(Literal::new_simple_literal("x")),
    };
    let nl = nonliteral_vars(&ext, &|_| true);
    assert!(!nl.contains(&var("s")), "a BIND target could be a literal");

    let values = GraphPattern::Values {
        variables: vec![var("v")],
        bindings: vec![vec![Some(GroundTerm::Literal(Literal::new_simple_literal("x")))]],
    };
    let j = GraphPattern::Join {
        left: Box::new(bgp(vec![tp_svo("v", "http://ex/p", TermPattern::Variable(var("o2")))])),
        right: Box::new(values),
    };
    let nl2 = nonliteral_vars(&j, &|_| true);
    assert!(!nl2.contains(&var("v")), "a VALUES column could be a literal");
}

#[test]
fn nonliteral_vars_graph_name_is_nonliteral() {
    let inner = bgp(vec![tp_svo("s", "http://ex/p", TermPattern::Variable(var("o")))]);
    let g = GraphPattern::Graph { name: NamedNodePattern::Variable(var("g")), inner: Box::new(inner) };
    let nl = nonliteral_vars(&g, &|_| true);
    assert!(nl.contains(&var("g")), "a GRAPH-name var binds an IRI");
}

// ---- static analysis: soundness holes flagged in PR #1785 review ------------------------

#[test]
fn nonliteral_vars_path_object_is_literal_risk() {
    // A property-path OBJECT can bind a literal (the final step of an alternative/sequence is
    // an ordinary object slot). `?x (:p|:q) ?name` must NOT report ?name non-literal, and — to
    // be safe under Reverse / zero-length matches — must NOT report the subject ?x either
    // (unless a BGP positions it as a subject/predicate).
    let path = GraphPattern::Path {
        subject: TermPattern::Variable(var("x")),
        path: PropertyPathExpression::Alternative(
            Box::new(PropertyPathExpression::NamedNode(nn("http://ex/p"))),
            Box::new(PropertyPathExpression::NamedNode(nn("http://ex/q"))),
        ),
        object: TermPattern::Variable(var("name")),
    };
    let nl = nonliteral_vars(&path, &|_| true);
    assert!(!nl.contains(&var("name")), "a path OBJECT can be a literal");
    assert!(!nl.contains(&var("x")), "a path endpoint is not credited non-literal on its own");
}

#[test]
fn nonliteral_vars_reverse_path_endpoints_are_literal_risk() {
    // Under Reverse the subject slot is the traversal OBJECT end, which can be a literal.
    let path = GraphPattern::Path {
        subject: TermPattern::Variable(var("s")),
        path: PropertyPathExpression::Reverse(Box::new(PropertyPathExpression::NamedNode(nn(
            "http://ex/p",
        )))),
        object: TermPattern::Variable(var("o")),
    };
    let nl = nonliteral_vars(&path, &|_| true);
    assert!(!nl.contains(&var("s")), "a Reverse-path subject end can be a literal");
    assert!(!nl.contains(&var("o")), "a Reverse-path object end can be a literal");
}

#[test]
fn nonliteral_vars_union_with_unanalysable_branch_poisons() {
    // ?x is a SUBJECT in the left branch but bound by an un-analysed SERVICE in the right —
    // the SERVICE could bind it to a literal, so the subject occurrence must NOT win.
    let left = bgp(vec![tp_svo("x", "http://ex/p", TermPattern::Variable(var("o")))]);
    let service = GraphPattern::Service {
        name: NamedNodePattern::NamedNode(nn("http://remote/sparql")),
        inner: Box::new(bgp(vec![tp_svo("y", "http://ex/q", TermPattern::Variable(var("x")))])),
        silent: false,
    };
    let u = GraphPattern::Union { left: Box::new(left), right: Box::new(service) };
    let nl = nonliteral_vars(&u, &|_| true);
    assert!(!nl.contains(&var("x")), "a SERVICE-bound var must not be claimed non-literal via a sibling subject slot");
}

#[test]
fn nonliteral_vars_quoted_triple_object_is_literal_risk() {
    // A quoted-triple subject `<<( ?s :p ?lit )>> :q ?o` carries an INNER object ?lit that can
    // be a literal — it must be literal-risk, not credited as a (subject-position) non-literal.
    let quoted = TermPattern::Triple(Box::new(TriplePattern {
        subject: TermPattern::Variable(var("s")),
        predicate: NamedNodePattern::NamedNode(nn("http://ex/p")),
        object: TermPattern::Variable(var("lit")),
    }));
    let p = bgp(vec![TriplePattern {
        subject: quoted,
        predicate: NamedNodePattern::NamedNode(nn("http://ex/q")),
        object: TermPattern::Variable(var("o")),
    }]);
    let nl = nonliteral_vars(&p, &|_| true);
    assert!(!nl.contains(&var("lit")), "an inner quoted-triple object can be a literal");
    assert!(!nl.contains(&var("s")), "a quoted-triple inner subject is not credited here");
}

#[test]
fn differential_path_object_numeric_promotion_end_to_end() {
    // The concrete result-change the review found: two path OBJECTS holding value-equal but
    // id-DISTINCT numerics ("1"^^integer vs "1.0"^^decimal). `FILTER(?a = ?b)` must be TRUE
    // (numeric promotion) whether the feature is on or off — the fix keeps ?a/?b literal-risk
    // so path (a) declines and the exact value path runs.
    let nt = concat!(
        "<http://ex/s1> <http://ex/val> \"1\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
        "<http://ex/s2> <http://ex/val> \"1.0\"^^<http://www.w3.org/2001/XMLSchema#decimal> .\n",
    );
    let g = Graph::load_str(nt, "ntriples").unwrap();
    // Path so the operands are PATH objects, exercising the G::Path arm's classification.
    let q = concat!(
        "SELECT ?a ?b WHERE { ",
        "?s1 <http://ex/val> ?a . ?s2 (<http://ex/val>|<http://ex/other>) ?b . ",
        "FILTER(?a = ?b) }"
    );
    let r = crate::query(&g, q).unwrap();
    // 1 = 1.0 by value -> every (a,b) pair over the two value-equal literals matches: 2x2 = 4.
    assert_eq!(r.rows.len(), 4, "numeric-promotion `=` over path objects must stay value-true");
}

#[test]
fn differential_exists_nested_filter_no_column_leak() {
    // PR #1785 re-review: the id-fast column set installed for the OUTER FILTER must NOT leak
    // into an EXISTS-nested FILTER, whose columns index a DIFFERENT Bindings layout. The outer
    // FILTER is over IRI subject columns (?a, ?b — non-literal), but the nested EXISTS FILTER
    // is over two LITERAL columns (?v1, ?v2) holding value-equal, id-DISTINCT numerics
    // ("1"^^integer vs "1.0"^^decimal). If the outer set (cols {0,1}) leaked, the nested
    // `?v1 = ?v2` would rewrite to IdEqNonLit and evaluate FALSE (unequal ids), flipping EXISTS
    // to false; correct SPARQL is TRUE (numeric promotion). Must hold feature-ON.
    let nt = concat!(
        "<http://ex/s1> <http://ex/val> \"1\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
        "<http://ex/s2> <http://ex/val> \"1.0\"^^<http://www.w3.org/2001/XMLSchema#decimal> .\n",
        "<http://ex/a> <http://ex/p> <http://ex/o1> .\n",
        "<http://ex/b> <http://ex/q> <http://ex/o2> .\n",
    );
    let g = Graph::load_str(nt, "ntriples").unwrap();
    // The OUTER pattern is a UNION (non-conjunctive), so its FILTER routes through the WRAPPED
    // dispatch that INSTALLS the id-fast column set (cols for ?a — an IRI subject). During that
    // filter's row loop the set is live; the EXISTS inner FILTER `?v1 = ?v2` (over two LITERAL
    // columns) re-enters on the same thread — pre-fix it would rewrite using the OUTER cols and
    // flip EXISTS to false. Post-fix (drain-on-consume + EXISTS empty-set guard) it stays TRUE.
    let q = concat!(
        "SELECT ?a WHERE { ",
        "{ ?a <http://ex/p> <http://ex/o1> } UNION { ?a <http://ex/q> <http://ex/o2> } ",
        "FILTER(EXISTS { ?s1 <http://ex/val> ?v1 . ?s2 <http://ex/val> ?v2 . FILTER(?v1 = ?v2) }) }"
    );
    let r = crate::query(&g, q).unwrap();
    // EXISTS is TRUE (1 = 1.0 by value), so both UNION rows (?a = ex:a, ex:b) survive.
    assert_eq!(r.rows.len(), 2, "EXISTS-nested numeric-promotion `=` must stay value-TRUE (no column-set leak)");
}

#[test]
fn apply_filter_scalar_drains_installed_cols_before_nested_reentry() {
    // Directly exercises the drain-on-consume invariant that fixes the EXISTS-re-entry leak
    // (PR #1785 re-review): the installed set is valid ONLY for the Bindings it was computed
    // against, and `eval_compiled` can re-enter FILTER evaluation (via EXISTS) DURING the outer
    // filter's row loop — while the outer set is still installed (the restore-guard has not yet
    // run). If the set were merely BORROWED (not drained before the loop), that nested filter
    // would apply the OUTER column indices to a DIFFERENT Bindings layout.
    //
    // We model exactly that window: install {0,1}, and while it is installed run a NESTED
    // `apply_filter_scalar` over LITERAL columns at the same indices {0,1} (value-equal,
    // id-distinct "1"^^integer / "1.0"^^decimal). The nested filter must NOT see {0,1} — it
    // must take the exact numeric-promotion path and KEEP the row. A stale {0,1} would rewrite
    // `?x = ?y` to IdEqNonLit and DROP it.
    let g = mixed_graph();
    let local = LocalVocab::default();
    let ids = object_ids(&g); // ids[1]="1"^^integer, ids[2]="1.0"^^decimal (value-equal)
    let s1 = g.id_of(&Term::NamedNode(nn("http://ex/s1"))).unwrap();

    let mut outer = Bindings::unsorted(vec![var("x"), var("y")], vec![Row::from_slice(&[s1, s1])]);
    with_idfast_nonlit_cols([0usize, 1usize].into_iter().collect(), || {
        // The OUTER consuming call drains the set to empty BEFORE its row loop.
        apply_filter_scalar(&g, &local, &mut outer, &eq_expr_xy()).unwrap();
        // Simulate an EXISTS re-entry that happens WHILE the guard is still live (i.e. before
        // the `with_` scope exits): a nested filter over literal columns at indices {0,1}.
        let mut nested =
            Bindings::unsorted(vec![var("x"), var("y")], vec![Row::from_slice(&[ids[1], ids[2]])]);
        apply_filter_scalar(&g, &local, &mut nested, &eq_expr_xy()).unwrap();
        assert_eq!(
            nested.rows.len(),
            1,
            "1 = 1.0 by value must survive — the outer set must have been DRAINED, not leaked into re-entry"
        );
    });
    assert_eq!(outer.rows.len(), 1, "s1 = s1 keeps the outer row");
}

// ---- equal_idfast unit behaviour -------------------------------------------------------

#[test]
fn equal_idfast_unbound_column_is_type_error() {
    let g = mixed_graph();
    let row = [NO_ID, 5u32];
    // A COLUMN holding NO_ID is unbound -> type error, not false.
    let v = equal_idfast(&g, &row, &IdOperand::Col(0), &IdOperand::Col(1));
    assert!(matches!(v, Value::Error), "unbound operand must be a type error");
}

#[test]
fn equal_idfast_absent_const_iri_is_false_not_error() {
    let g = mixed_graph();
    let ids = object_ids(&g);
    let row = [ids[0], 0]; // ids[0] = <http://ex/o1>
    // A constant IRI absent from the dict resolves to NO_ID but is NOT unbound -> "never equal".
    let absent = IdOperand::ConstIri(nn("http://ex/NOT_IN_GRAPH"));
    let v = equal_idfast(&g, &row, &IdOperand::Col(0), &absent);
    assert!(matches!(v, Value::Bool(false)), "absent const IRI compares as not-equal, not error");
}

#[test]
fn equal_idfast_iri_identity() {
    let g = mixed_graph();
    let ids = object_ids(&g);
    let row = [ids[0], ids[0]];
    assert!(matches!(equal_idfast(&g, &row, &IdOperand::Col(0), &IdOperand::Col(1)), Value::Bool(true)));
    let row2 = [ids[0], ids[7]]; // o1 vs "01"^^integer literal id — different ids
    assert!(matches!(equal_idfast(&g, &row2, &IdOperand::Col(0), &IdOperand::Col(1)), Value::Bool(false)));
}

// ---- DIFFERENTIAL: fast path vs ground-truth exact semantics, every id pair --------------

#[test]
fn differential_static_nonliteral_pairs_iri_only() {
    // When both columns are STATICALLY non-literal, only IRI/bnode ids ever appear. Pair the
    // IRI/bnode ids of the mixed graph and assert fast == exact for `=` and `!=`.
    let g = mixed_graph();
    let local = LocalVocab::default();
    let iri_bnode: Vec<Id> = {
        let s = |t: Term| g.id_of(&t).unwrap();
        vec![
            s(Term::NamedNode(nn("http://ex/s1"))),
            s(Term::NamedNode(nn("http://ex/o1"))),
            s(Term::NamedNode(nn("http://ex/p"))),
            s(Term::NamedNode(nn("http://ex/s2"))),
        ]
    };
    let mut rows = Vec::new();
    for &a in &iri_bnode {
        for &c in &iri_bnode {
            rows.push([a, c]);
        }
    }
    let b = table(&rows);
    // Columns 0 and 1 are both non-literal.
    let nonlit: FxHashSet<usize> = [0usize, 1usize].into_iter().collect();

    for expr in [eq_expr(), ne_expr()] {
        let negated = matches!(expr, Expression::Not(_));
        let fast = eval_filter_fast(&g, &local, &b, &expr, &nonlit);
        for (i, row) in b.rows.iter().enumerate() {
            let mut want = reference_equal(&g, &local, row[0], row[1]);
            if negated {
                want = want.map(|x| !x);
            }
            assert_eq!(fast[i], want, "row {} ids ({},{}) negated={}", i, row[0], row[1], negated);
        }
    }
}

#[test]
fn differential_equal_id_shortcircuit_all_kinds() {
    // Path (b): when both columns hold the SAME id (any kind, incl. weird/ill-typed literals),
    // `=` must be true and `!=` false — matching the ground-truth sameTerm decision. Here the
    // columns are NOT declared non-literal (empty nonlit set), so ONLY path (b) can fire.
    let g = mixed_graph();
    let local = LocalVocab::default();
    let ids = object_ids(&g);
    let rows: Vec<[Id; 2]> = ids.iter().map(|&x| [x, x]).collect();
    let b = table(&rows);
    let empty: FxHashSet<usize> = FxHashSet::default();

    for expr in [eq_expr(), ne_expr()] {
        let negated = matches!(expr, Expression::Not(_));
        let fast = eval_filter_fast(&g, &local, &b, &expr, &empty);
        for (i, row) in b.rows.iter().enumerate() {
            let mut want = reference_equal(&g, &local, row[0], row[1]);
            if negated {
                want = want.map(|x| !x);
            }
            assert_eq!(fast[i], want, "equal-id row {} id {}", i, row[0]);
        }
    }
}

#[test]
fn differential_unequal_literal_ids_take_exact_path() {
    // The sq-lr2ii caution: unequal ids of VALUE-EQUAL literals ("1"^^integer, "1.0"^^decimal,
    // " 1 "^^integer, "01"^^integer, "1.0"^^double) must NOT be decided by the id fast path —
    // they must fall through to the exact value comparison. Columns are literal-capable (empty
    // nonlit set), so path (a) is off and path (b) can't fire on unequal ids; the result must
    // match the exact numeric-promotion semantics.
    let g = mixed_graph();
    let local = LocalVocab::default();
    let ids = object_ids(&g);
    let mut rows = Vec::new();
    for &a in &ids {
        for &c in &ids {
            if a != c {
                rows.push([a, c]);
            }
        }
    }
    let b = table(&rows);
    let empty: FxHashSet<usize> = FxHashSet::default();

    for expr in [eq_expr(), ne_expr()] {
        let negated = matches!(expr, Expression::Not(_));
        let fast = eval_filter_fast(&g, &local, &b, &expr, &empty);
        for (i, row) in b.rows.iter().enumerate() {
            let mut want = reference_equal(&g, &local, row[0], row[1]);
            if negated {
                want = want.map(|x| !x);
            }
            assert_eq!(
                fast[i], want,
                "unequal-id literal row {} ids ({},{}) must match exact semantics",
                i, row[0], row[1]
            );
        }
    }
}

#[test]
fn differential_end_to_end_q08_shape() {
    // A q08-like UNION-branch FILTER over IRI subject vars, run through the PUBLIC query
    // entry point (which installs the static analysis + fast path). The answer must be
    // correct regardless of the fast path — an independent recompute of the expected set.
    let nt = concat!(
        "<http://ex/d1> <http://ex/creator> <http://ex/alice> .\n",
        "<http://ex/d1> <http://ex/creator> <http://ex/bob> .\n",
        "<http://ex/d2> <http://ex/creator> <http://ex/alice> .\n",
    );
    let g = Graph::load_str(nt, "ntriples").unwrap();
    // Pairs of DISTINCT co-creators of the same document (?a != ?b over IRI subjects/objects
    // of a creator triple — the != is exactly id inequality here).
    let q = concat!(
        "SELECT ?a ?b WHERE { ",
        "?doc <http://ex/creator> ?a . ?doc <http://ex/creator> ?b . ",
        "FILTER(?a != ?b) }"
    );
    let r = crate::query(&g, q).unwrap();
    assert_eq!(r.rows.len(), 2, "alice/bob and bob/alice for d1 only");
    for row in &r.rows {
        assert_ne!(row[0], row[1], "the != filter must exclude self-pairs");
    }
}

// ---- sq-1ivw7: predicate-range object term-kind widening -------------------------------

/// The q08/q12b-shaped store: `creator` is IRI-only (unblockable), `title`/`year` carry
/// literal objects (must decline). Includes a bnode creator object (still literal-free).
fn predrange_graph() -> Graph {
    let nt = concat!(
        "<http://ex/d1> <http://ex/creator> <http://ex/alice> .\n",
        "<http://ex/d1> <http://ex/creator> <http://ex/bob> .\n",
        "<http://ex/d2> <http://ex/creator> _:anon .\n",
        "<http://ex/d1> <http://ex/title> \"A Paper\" .\n",
        "<http://ex/d1> <http://ex/year> \"2020\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
    );
    Graph::load_str(nt, "ntriples").unwrap()
}

/// Does `nonliteral_filter_cols` prove `col` (a column of `b`) non-literal for the FILTER over
/// `inner`, in the snapshot `g`? Exercises the REAL snapshot-aware analysis path.
fn col_is_nonliteral(g: &Graph, inner: &GraphPattern, b: &Bindings, col: usize) -> bool {
    nonliteral_filter_cols(g, inner, b).contains(&col)
}

#[test]
fn predrange_credits_iri_only_object_declines_literal() {
    let g = predrange_graph();
    // `?a` bound ONLY as the object of a constant literal-free predicate `creator`.
    let inner = bgp(vec![tp_svo("doc", "http://ex/creator", TermPattern::Variable(var("a")))]);
    let b = Bindings::unsorted(vec![var("doc"), var("a")], vec![]);
    // `?a` is column 1; the snapshot-aware analysis must prove it non-literal.
    assert!(
        col_is_nonliteral(&g, &inner, &b, b.col(&var("a")).unwrap()),
        "an object of a literal-free constant predicate must be credited non-literal"
    );
    // `?t` bound as object of `title` (has a literal object) must DECLINE.
    let inner_t = bgp(vec![tp_svo("doc", "http://ex/title", TermPattern::Variable(var("t")))]);
    let bt = Bindings::unsorted(vec![var("doc"), var("t")], vec![]);
    assert!(
        !col_is_nonliteral(&g, &inner_t, &bt, bt.col(&var("t")).unwrap()),
        "an object of a literal-having predicate must NOT be credited"
    );
    // `?y` bound as object of `year` (inline-integer literal object) must DECLINE.
    let inner_y = bgp(vec![tp_svo("doc", "http://ex/year", TermPattern::Variable(var("y")))]);
    let by = Bindings::unsorted(vec![var("doc"), var("y")], vec![]);
    assert!(
        !col_is_nonliteral(&g, &inner_y, &by, by.col(&var("y")).unwrap()),
        "an inline-integer literal object predicate must NOT be credited"
    );
}

#[test]
fn predrange_mixed_use_of_same_var_declines() {
    // `?x` is the object of literal-free `creator` in one triple but the object of `title`
    // (literal-having) in another. It CAN be a literal, so it must NOT be credited — the
    // `nonlit \ maybe_lit` difference must exclude it.
    let g = predrange_graph();
    let inner = bgp(vec![
        tp_svo("d1", "http://ex/creator", TermPattern::Variable(var("x"))),
        tp_svo("d2", "http://ex/title", TermPattern::Variable(var("x"))),
    ]);
    let b = Bindings::unsorted(vec![var("d1"), var("d2"), var("x")], vec![]);
    assert!(
        !col_is_nonliteral(&g, &inner, &b, b.col(&var("x")).unwrap()),
        "a var used as a literal-having-predicate object anywhere must decline"
    );
}

#[test]
fn predrange_variable_predicate_declines() {
    // A VARIABLE predicate is not a constant IRI, so the snapshot check cannot apply and the
    // object stays literal-risk (the store could bind ?p to a literal-having predicate).
    let g = predrange_graph();
    let inner = bgp(vec![TriplePattern {
        subject: TermPattern::Variable(var("doc")),
        predicate: NamedNodePattern::Variable(var("p")),
        object: TermPattern::Variable(var("a")),
    }]);
    let b = Bindings::unsorted(vec![var("doc"), var("p"), var("a")], vec![]);
    assert!(
        !col_is_nonliteral(&g, &inner, &b, b.col(&var("a")).unwrap()),
        "a variable-predicate object must stay literal-risk"
    );
}

#[test]
fn predrange_fires_on_q08_shape_witness() {
    // WITNESS that path (a) now FIRES on the q08/q12b shape: `?a`/`?b` are objects of the
    // literal-free `creator` predicate, so `nonliteral_filter_cols` proves both columns
    // non-literal and `idfast_rewrite` rewrites `?a = ?b` into `IdEqNonLit`. (Under #1785 this
    // declined — the honest null result this bead closes.)
    let g = predrange_graph();
    let inner = bgp(vec![
        tp_svo("doc", "http://ex/creator", TermPattern::Variable(var("a"))),
        tp_svo("doc", "http://ex/creator", TermPattern::Variable(var("b"))),
    ]);
    let b = Bindings::unsorted(vec![var("doc"), var("a"), var("b")], vec![]);
    let cols = nonliteral_filter_cols(&g, &inner, &b);
    assert!(cols.contains(&b.col(&var("a")).unwrap()), "?a credited");
    assert!(cols.contains(&b.col(&var("b")).unwrap()), "?b credited");
    // The rewrite converts `?a = ?b` into the id fast path.
    let expr = Expression::Equal(
        Box::new(Expression::Variable(var("a"))),
        Box::new(Expression::Variable(var("b"))),
    );
    let mut compiled = compile_expr(&expr, &b, &LocalVocab::default());
    idfast_rewrite(&mut compiled, &cols);
    assert!(
        matches!(compiled, CompiledExpr::IdEqNonLit(..)),
        "the q08-shape `=` must compile to the id fast path; got {:?}",
        compiled
    );
}

#[test]
fn predrange_end_to_end_q08_result_unchanged() {
    // Full public-entry differential: the fast path fires (previous test), yet the ANSWER is an
    // independent recompute — distinct co-creator IRI pairs of the same document.
    let g = predrange_graph();
    let q = concat!(
        "SELECT ?a ?b WHERE { ",
        "?doc <http://ex/creator> ?a . ?doc <http://ex/creator> ?b . ",
        "FILTER(?a != ?b) }"
    );
    let r = crate::query(&g, q).unwrap();
    // d1 has {alice, bob}; d2 has {_:anon} alone. Distinct ordered pairs for d1: (alice,bob),(bob,alice).
    assert_eq!(r.rows.len(), 2, "distinct co-creator pairs of d1 only");
    for row in &r.rows {
        assert_ne!(row[0], row[1], "!= excludes self-pairs");
    }
}

#[test]
fn predrange_snapshot_update_flips_credit() {
    // The snapshot-lifecycle invariant end-to-end: `creator` is literal-free -> credited; an
    // UPDATE inserting a literal creator object publishes a new snapshot in which the same
    // analysis DECLINES. Proves the credit never outlives the snapshot it was computed against.
    let mut g = predrange_graph();
    let inner = bgp(vec![tp_svo("doc", "http://ex/creator", TermPattern::Variable(var("a")))]);
    let b = Bindings::unsorted(vec![var("doc"), var("a")], vec![]);
    let a_col = b.col(&var("a")).unwrap();
    assert!(col_is_nonliteral(&g, &inner, &b, a_col), "literal-free before update -> credited");
    // UPDATE: a literal object now exists for `creator`.
    g.apply_delta(
        &[[
            Term::NamedNode(nn("http://ex/d3")),
            Term::NamedNode(nn("http://ex/creator")),
            Term::Literal(Literal::new_simple_literal("Anonymous")),
        ]],
        &[],
    )
    .unwrap();
    assert!(
        !col_is_nonliteral(&g, &inner, &b, a_col),
        "after the update the same analysis must DECLINE against the new snapshot"
    );
    // And the end-to-end result stays correct across the update (exact path now runs for `!=`).
    let q = concat!(
        "SELECT ?a ?b WHERE { ",
        "?doc <http://ex/creator> ?a . ?doc <http://ex/creator> ?b . ",
        "FILTER(?a != ?b) }"
    );
    let r = crate::query(&g, q).unwrap();
    for row in &r.rows {
        assert_ne!(row[0], row[1], "!= still excludes self-pairs post-update");
    }
}

/// A default graph whose `creator` predicate is LITERAL-FREE, plus a named graph `<http://ex/g>`
/// whose `creator` objects are a `xsd:integer`/`xsd:decimal` pair that are `=`-equal by VALUE.
/// Loaded via TriG so `.named` is populated (a named graph is a self-contained sub-`Graph`).
fn cross_graph_dataset() -> Graph {
    let trig = concat!(
        "<http://ex/d0> <http://ex/creator> <http://ex/alice> .\n",
        "<http://ex/g> {\n",
        "  <http://ex/d1> <http://ex/creator> \"1\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
        "  <http://ex/d1> <http://ex/creator> \"1.0\"^^<http://www.w3.org/2001/XMLSchema#decimal> .\n",
        "}\n",
    );
    Graph::load_dataset(trig, "trig").unwrap()
}

#[test]
fn predrange_graph_block_declines_object_credit_at_outer_dispatch() {
    // ANALYSIS-LEVEL decline witness for the cross-graph hole (PR #1795 reviewer counterexample).
    // The FILTER sits OUTSIDE the GRAPH block, so `nonliteral_filter_cols` classifies against the
    // DEFAULT graph — whose `creator` column is literal-free. But `?a`/`?b` are bound from INSIDE
    // `GRAPH <http://ex/g>`, a DIFFERENT store carrying literal objects. Crediting them off the
    // default column would be unsound. After the fix (`G::Graph` recurses with `&|_| true`), the
    // outer dispatch gives an object var under any GRAPH block NO credit.
    let g = cross_graph_dataset();
    let inner = GraphPattern::Graph {
        name: NamedNodePattern::NamedNode(nn("http://ex/g")),
        inner: Box::new(bgp(vec![
            tp_svo("d", "http://ex/creator", TermPattern::Variable(var("a"))),
            tp_svo("d", "http://ex/creator", TermPattern::Variable(var("b"))),
        ])),
    };
    let b = Bindings::unsorted(vec![var("d"), var("a"), var("b")], vec![]);
    let cols = nonliteral_filter_cols(&g, &inner, &b);
    assert!(
        !cols.contains(&b.col(&var("a")).unwrap()),
        "?a bound under GRAPH must NOT be credited at the outer (default-graph) dispatch"
    );
    assert!(
        !cols.contains(&b.col(&var("b")).unwrap()),
        "?b bound under GRAPH must NOT be credited at the outer (default-graph) dispatch"
    );
}

#[test]
fn predrange_cross_graph_filter_result_identical() {
    // END-TO-END differential (feature ON) for the reviewer's exact counterexample. `?a`/`?b`
    // are `"1"^^xsd:integer` and `"1.0"^^xsd:decimal` bound inside `GRAPH <http://ex/g>`; the
    // FILTER sits outside. `=` is VALUE equality, so `1 = 1.0` holds and all 4 ordered pairs
    // (2×2) qualify. Before the fix the object credit fired off the default graph's literal-free
    // column, the pair was id-compared, and only the 2 self-pairs survived (RED: 2 != 4).
    let g = cross_graph_dataset();
    let q = concat!(
        "SELECT ?a ?b WHERE { ",
        "GRAPH <http://ex/g> { ?d <http://ex/creator> ?a . ?d <http://ex/creator> ?b } ",
        "FILTER(?a = ?b) }"
    );
    let r = crate::query(&g, q).unwrap();
    assert_eq!(
        r.rows.len(),
        4,
        "value `=` matches the integer/decimal pair: all 4 ordered pairs qualify (feature ON must equal OFF)"
    );
}

// ---- MUTATION check: a deliberately-wrong fast result must be caught by the differential ---

#[test]
fn mutation_wrong_shortcircuit_would_be_caught() {
    // Sanity that the differential oracle is non-vacuous: an INVERTED fast verdict on an
    // equal-id pair disagrees with the reference. (We assert the disagreement directly rather
    // than mutate production code.)
    let g = mixed_graph();
    let local = LocalVocab::default();
    let ids = object_ids(&g);
    let id = ids[4]; // "hi" simple literal
    let correct = equal_idfast(&g, &[id, id], &IdOperand::Col(0), &IdOperand::Col(1));
    assert!(matches!(correct, Value::Bool(true)));
    // The reference agrees with the correct verdict...
    assert_eq!(reference_equal(&g, &local, id, id), Some(true));
    // ...so an inverted verdict (false) would be a detectable mismatch.
    let mutated = Value::Bool(false);
    assert_ne!(
        ebv3(&mutated, crate::EbvSemantics::Rec2013),
        reference_equal(&g, &local, id, id),
        "an inverted equal-id verdict must disagree with the oracle"
    );
}
