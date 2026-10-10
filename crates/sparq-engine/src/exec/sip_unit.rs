//! Direct unit tests for the SIP helpers (`certain_vars` / `subst_pattern`),
//! complementing the end-to-end `tests/sip_join.rs`. (sq-7d3dj.30.3)
use super::*;
use oxrdf::{NamedNode, Variable};

fn var(s: &str) -> Variable {
    Variable::new(s).unwrap()
}
fn nn(s: &str) -> NamedNode {
    NamedNode::new(s).unwrap()
}
fn tp(s: &str, p: &str, o: &str) -> TriplePattern {
    TriplePattern {
        subject: TermPattern::Variable(var(s)),
        predicate: NamedNodePattern::NamedNode(nn(p)),
        object: TermPattern::Variable(var(o)),
    }
}

#[test]
fn certain_vars_union_is_branch_intersection() {
    // left binds {x, y}; right binds {x, z}; only x is certain in the UNION.
    let left = GraphPattern::Bgp { patterns: vec![tp("x", "http://e/p", "y")] };
    let right = GraphPattern::Bgp { patterns: vec![tp("x", "http://e/q", "z")] };
    let u = GraphPattern::Union { left: Box::new(left), right: Box::new(right) };
    let mut out = FxHashSet::default();
    certain_vars(&u, &mut out);
    assert!(out.contains(&var("x")), "x certain (bound in both branches)");
    assert!(!out.contains(&var("y")), "y bound in only one branch");
    assert!(!out.contains(&var("z")), "z bound in only one branch");
}

#[test]
fn certain_vars_leftjoin_excludes_optional_side() {
    let left = GraphPattern::Bgp { patterns: vec![tp("x", "http://e/p", "y")] };
    let right = GraphPattern::Bgp { patterns: vec![tp("x", "http://e/q", "opt")] };
    let lj = GraphPattern::LeftJoin {
        left: Box::new(left),
        right: Box::new(right),
        expression: None,
    };
    let mut out = FxHashSet::default();
    certain_vars(&lj, &mut out);
    assert!(out.contains(&var("x")) && out.contains(&var("y")));
    assert!(!out.contains(&var("opt")), "an OPTIONAL-right var is never certain");
}

#[test]
fn subst_pattern_replaces_variable_in_bgp_and_filter() {
    let inner = GraphPattern::Bgp { patterns: vec![tp("doc", "http://e/creator", "prin")] };
    let filt = GraphPattern::Filter {
        expr: Expression::Not(Box::new(Expression::Equal(
            Box::new(Expression::Variable(var("a"))),
            Box::new(Expression::Variable(var("prin"))),
        ))),
        inner: Box::new(inner),
    };
    let mut sub = FxHashMap::default();
    sub.insert(var("prin"), nn("http://e/paul"));
    let out = subst_pattern(&filt, &sub).expect("substitution should succeed");
    // The substituted variable must be fully gone (pattern positions AND filter).
    let dbg = format!("{:?}", out);
    assert!(dbg.contains("paul"), "result must carry the substituted IRI");
    assert!(!dbg.contains("prin"), "substituted variable ?prin must be gone: {}", dbg);
    assert!(dbg.contains("\"a\""), "the untouched ?a must remain");
}

#[test]
fn subst_pattern_bails_on_bound_of_substituted_var() {
    let inner = GraphPattern::Bgp { patterns: vec![tp("x", "http://e/p", "y")] };
    let filt = GraphPattern::Filter { expr: Expression::Bound(var("x")), inner: Box::new(inner) };
    let mut sub = FxHashMap::default();
    sub.insert(var("x"), nn("http://e/c"));
    assert!(
        subst_pattern(&filt, &sub).is_none(),
        "BOUND(?x) on a substituted variable must force the cold fallback"
    );
}
