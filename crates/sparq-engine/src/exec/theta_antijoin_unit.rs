//! Direct unit tests for the correlated theta anti-join's pure shape helpers,
//! complementing the end-to-end `tests/theta_antijoin.rs`.
//! (bead sq-7d3dj.30.9)
use super::*;
use oxrdf::{NamedNode, Variable};

fn var(s: &str) -> Variable {
    Variable::new(s).unwrap()
}
fn nn(s: &str) -> NamedNode {
    NamedNode::new(s).unwrap()
}
fn ev(s: &str) -> Expression {
    Expression::Variable(var(s))
}

#[test]
fn as_not_bound_var_matches_only_not_bound() {
    assert_eq!(
        as_not_bound_var(&Expression::Not(Box::new(Expression::Bound(var("a"))))),
        Some(var("a")),
        "!bound(?a) must extract ?a"
    );
    // Not a `!bound`: bare bound, bare not, equality.
    assert!(as_not_bound_var(&Expression::Bound(var("a"))).is_none());
    assert!(as_not_bound_var(&Expression::Not(Box::new(ev("a")))).is_none());
    assert!(as_not_bound_var(&Expression::Equal(Box::new(ev("a")), Box::new(ev("b")))).is_none());
}

#[test]
fn as_correlation_pair_classifies_eq_sameterm_and_single_var_in() {
    // `?a = ?b` → Eq correlation.
    assert_eq!(
        as_correlation_pair(&Expression::Equal(Box::new(ev("a")), Box::new(ev("b")))),
        Some((var("a"), var("b"), CorrKind::Eq))
    );
    // `sameTerm(?a, ?b)` → SameTerm correlation (sq-3cmr4 extension).
    assert_eq!(
        as_correlation_pair(&Expression::SameTerm(Box::new(ev("a")), Box::new(ev("b")))),
        Some((var("a"), var("b"), CorrKind::SameTerm))
    );
    // `?a IN (?b)` desugars to `?a = ?b` → Eq correlation.
    assert_eq!(
        as_correlation_pair(&Expression::In(Box::new(ev("a")), vec![ev("b")])),
        Some((var("a"), var("b"), CorrKind::Eq))
    );
    // A var = IRI equality is NOT a correlation (one side is a constant).
    assert!(as_correlation_pair(&Expression::Equal(
        Box::new(ev("a")),
        Box::new(Expression::NamedNode(nn("http://e/x")))
    ))
    .is_none());
    // A CONSTANT-list `IN` is NOT a correlation (nothing to seed) — brief's rule.
    assert!(as_correlation_pair(&Expression::In(
        Box::new(ev("a")),
        vec![Expression::NamedNode(nn("http://e/x"))]
    ))
    .is_none());
    // A MULTI-element `IN` (even all-vars) is NOT the single-target seedable shape.
    assert!(
        as_correlation_pair(&Expression::In(Box::new(ev("a")), vec![ev("b"), ev("c")])).is_none()
    );
    // An inequality is not a correlation.
    assert!(as_correlation_pair(&Expression::Less(Box::new(ev("a")), Box::new(ev("b")))).is_none());
}

#[test]
fn corr_recheck_expr_uses_the_correlations_own_relation() {
    // An Eq correlation re-checks with `=`; a SameTerm correlation with `sameTerm`
    // (they DIFFER on value-equal id-distinct literals — the soundness point).
    let out = vec![var("outer"), var("inner")];
    let eq = Correlation { left_col: 0, inner_var: var("inner"), kind: CorrKind::Eq };
    assert!(matches!(corr_recheck_expr(&eq, &out), Expression::Equal(..)));
    let st = Correlation { left_col: 0, inner_var: var("inner"), kind: CorrKind::SameTerm };
    assert!(matches!(corr_recheck_expr(&st, &out), Expression::SameTerm(..)));
}

#[test]
fn corr_id_probeable_is_kind_and_term_aware() {
    use oxrdf::{BlankNode, Literal};
    let iri = Term::NamedNode(nn("http://e/x"));
    let blank = Term::BlankNode(BlankNode::new("b0").unwrap());
    let lit = Term::Literal(Literal::new_simple_literal("hi"));
    // sameTerm: EVERY bound kind is id-probeable (id-equality IS sameTerm).
    assert!(corr_id_probeable(CorrKind::SameTerm, Some(&iri)));
    assert!(corr_id_probeable(CorrKind::SameTerm, Some(&blank)));
    assert!(corr_id_probeable(CorrKind::SameTerm, Some(&lit)));
    assert!(!corr_id_probeable(CorrKind::SameTerm, None), "unbound is not probeable");
    // Eq: only NamedNode/BlankNode; a LITERAL is the value-equality hazard.
    assert!(corr_id_probeable(CorrKind::Eq, Some(&iri)));
    assert!(corr_id_probeable(CorrKind::Eq, Some(&blank)));
    assert!(!corr_id_probeable(CorrKind::Eq, Some(&lit)), "literal Eq must take value scan");
    assert!(!corr_id_probeable(CorrKind::Eq, None));
}

#[test]
fn split_and_flattens_left_to_right() {
    let e = Expression::And(
        Box::new(Expression::And(Box::new(ev("a")), Box::new(ev("b")))),
        Box::new(ev("c")),
    );
    let cs = split_and(&e);
    assert_eq!(cs.len(), 3, "((a && b) && c) → three conjuncts");
    assert_eq!(cs[0], &ev("a"));
    assert_eq!(cs[1], &ev("b"));
    assert_eq!(cs[2], &ev("c"));
    // A non-And yields a single-element list.
    assert_eq!(split_and(&ev("z")).len(), 1);
}

#[test]
fn pattern_has_var_reaches_every_node_kind() {
    let bgp = |s: &str, p: &str, o: &str| GraphPattern::Bgp {
        patterns: vec![TriplePattern {
            subject: TermPattern::Variable(var(s)),
            predicate: NamedNodePattern::NamedNode(nn(p)),
            object: TermPattern::Variable(var(o)),
        }],
    };
    // A var buried in the OPTIONAL condition (not in any triple slot) must be found.
    let lj = GraphPattern::LeftJoin {
        left: Box::new(bgp("a", "http://e/p", "b")),
        right: Box::new(bgp("c", "http://e/q", "d")),
        expression: Some(Expression::Less(Box::new(ev("yr2")), Box::new(ev("yr")))),
    };
    assert!(pattern_has_var(&lj, &var("yr")), "outer var in the OPTIONAL condition");
    assert!(pattern_has_var(&lj, &var("d")), "right-side triple var");
    assert!(pattern_has_var(&lj, &var("a")), "left-side triple var");
    assert!(!pattern_has_var(&lj, &var("absent")), "an unrelated var must not be found");
    // EXISTS sub-pattern var.
    let filt = GraphPattern::Filter {
        expr: Expression::Exists(Box::new(bgp("e", "http://e/r", "f"))),
        inner: Box::new(bgp("a", "http://e/p", "b")),
    };
    assert!(pattern_has_var(&filt, &var("f")), "var inside an EXISTS sub-pattern");
}
