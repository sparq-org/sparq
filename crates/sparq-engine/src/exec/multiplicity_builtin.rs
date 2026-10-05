use super::*;

/// `?x` over `{10×3, 20×2}`: three subjects with value 10, two with value 20. The
/// sub-SELECT projects only `?x`, so the outer aggregate sees the bag {10,10,10,20,20}.
fn g() -> Graph {
    Graph::load_str(
        "@prefix ex: <http://ex/> . \
             ex:a ex:v 10 . ex:b ex:v 10 . ex:c ex:v 10 . \
             ex:d ex:v 20 . ex:e ex:v 20 .",
        "turtle",
    )
    .unwrap()
}

fn run(q: &str) -> QueryResult {
    crate::query(&g(), &format!("PREFIX ex: <http://ex/> {q}")).unwrap()
}

fn int_lit(n: i64) -> Term {
    Term::Literal(oxrdf::Literal::new_typed_literal(n.to_string(), oxrdf::vocab::xsd::INTEGER))
}

/// The headline identity: over the DISTINCT solutions weighted by `MULTIPLICITY()`,
/// `SUM(?x * MULTIPLICITY())` equals plain `SUM(?x)` over the bag. 10·3 + 20·2 = 70,
/// and 10+10+10+20+20 = 70. This is exactly the `SUM(?x * multiplicity())` example from
/// the survey §B2 / `research/sparql12-engine.md` §1.4.
#[test]
fn weighted_sum_equals_bag_sum() {
    let r = run(
        "SELECT (SUM(?x) AS ?bag) (SUM(?x * MULTIPLICITY()) AS ?w) \
             WHERE { SELECT ?x WHERE { ?s ex:v ?x } }",
    );
    assert_eq!(r.rows.len(), 1);
    assert_eq!(r.rows[0][0], Some(int_lit(70)), "plain SUM over the bag");
    assert_eq!(r.rows[0][1], Some(int_lit(70)), "MULTIPLICITY-weighted SUM must match");
}

/// Bare `SUM(MULTIPLICITY())` over the distinct members sums the bag cardinalities, i.e.
/// recovers the total bag size: 3 + 2 = 5 = `COUNT(*)`.
#[test]
fn sum_of_multiplicities_is_bag_size() {
    let r = run(
        "SELECT (SUM(MULTIPLICITY()) AS ?n) (COUNT(*) AS ?c) \
             WHERE { SELECT ?x WHERE { ?s ex:v ?x } }",
    );
    assert_eq!(r.rows.len(), 1);
    assert_eq!(r.rows[0][0], Some(int_lit(5)), "Σ multiplicity = bag size");
    assert_eq!(r.rows[0][1], Some(int_lit(5)), "COUNT(*) = bag size");
}

/// Per-GROUP multiplicity: grouping by `?x` gives one group per distinct value, each a
/// single distinct member whose multiplicity is the group's bag count (3 for x=10, 2 for
/// x=20). `MAX(MULTIPLICITY())` surfaces that count directly per group.
#[test]
fn per_group_multiplicity_in_grouped_select() {
    let r = run(
        "SELECT ?x (MAX(MULTIPLICITY()) AS ?m) \
             WHERE { SELECT ?x WHERE { ?s ex:v ?x } } GROUP BY ?x ORDER BY ?x",
    );
    assert_eq!(r.rows.len(), 2, "two distinct values ⇒ two groups");
    // ORDER BY ?x: 10 then 20.
    assert_eq!(r.rows[0], vec![Some(int_lit(10)), Some(int_lit(3))]);
    assert_eq!(r.rows[1], vec![Some(int_lit(20)), Some(int_lit(2))]);
}

/// A group with NO duplicates: every distinct member has multiplicity 1, so
/// `SUM(?x * MULTIPLICITY())` is just `SUM(?x)` and `MULTIPLICITY()` is uniformly 1.
#[test]
fn all_unique_solutions_have_multiplicity_one() {
    let graph = Graph::load_str(
        "@prefix ex: <http://ex/> . ex:a ex:v 1 . ex:b ex:v 2 . ex:c ex:v 3 .",
        "turtle",
    )
    .unwrap();
    let r = crate::query(
        &graph,
        "PREFIX ex: <http://ex/> SELECT (SUM(?x * MULTIPLICITY()) AS ?w) (SUM(MULTIPLICITY()) AS ?n) \
             WHERE { SELECT ?x WHERE { ?s ex:v ?x } }",
    )
    .unwrap();
    assert_eq!(r.rows[0][0], Some(int_lit(6)), "1+2+3, each ×1");
    assert_eq!(r.rows[0][1], Some(int_lit(3)), "three distinct members, each multiplicity 1");
}

/// `MULTIPLICITY()` is only meaningful inside an aggregate argument. Used in a plain BIND
/// (no aggregate context), the thread-local context is absent, so it is an expression
/// error → the BIND leaves `?m` UNBOUND (SPARQL error discipline), NOT a hard failure.
#[test]
fn multiplicity_outside_aggregate_is_unbound() {
    let r = run("SELECT ?s ?m WHERE { ?s ex:v ?x . BIND(MULTIPLICITY() AS ?m) } ORDER BY ?s");
    assert_eq!(r.rows.len(), 5, "one row per matching subject");
    for row in &r.rows {
        assert!(row[0].is_some(), "?s bound");
        assert_eq!(row[1], None, "MULTIPLICITY() outside an aggregate is an error ⇒ ?m unbound");
    }
}

/// The common case — an aggregate that never mentions `MULTIPLICITY()` — is byte-for-byte
/// unchanged: the duplicate-collapsing path is not taken, the full bag is folded. Plain
/// `SUM(?x)` over {10×3,20×2} stays 70 (NOT the distinct-set 30).
#[test]
fn aggregate_without_multiplicity_folds_full_bag() {
    let r = run("SELECT (SUM(?x) AS ?bag) (COUNT(?x) AS ?c) WHERE { SELECT ?x WHERE { ?s ex:v ?x } }");
    assert_eq!(r.rows[0][0], Some(int_lit(70)), "full bag sum, no distinct collapse");
    assert_eq!(r.rows[0][1], Some(int_lit(5)), "COUNT folds the full bag");
}

/// Direct unit coverage of the `group_multiplicities` helper: it returns `None` when the
/// argument never calls `MULTIPLICITY()` (so the common path pays nothing) and the
/// distinct-with-cardinalities table when it does.
#[test]
fn group_multiplicities_helper_gating() {
    use spargebra::algebra::Function;
    let b = Bindings::unsorted(
        vec![],
        vec![
            std::iter::once(dict::INLINE_BASE + 10).collect::<Row>(),
            std::iter::once(dict::INLINE_BASE + 10).collect::<Row>(),
            std::iter::once(dict::INLINE_BASE + 20).collect::<Row>(),
        ],
    );
    let members = [0usize, 1, 2];
    // A variable-only argument: no MULTIPLICITY() ⇒ None (full-bag path).
    let var = Expression::Variable(Variable::new_unchecked("x"));
    assert!(group_multiplicities(&b, &members, &var).is_none());
    // A MULTIPLICITY() call ⇒ distinct rows + cardinalities, first-seen order.
    let mult = Expression::FunctionCall(
        Function::Custom(oxrdf::NamedNode::new_unchecked(MULTIPLICITY_FN_IRI)),
        vec![],
    );
    let table = group_multiplicities(&b, &members, &mult).expect("multiplicity present ⇒ Some");
    let cards: Vec<u64> = table.iter().map(|&(_, c)| c).collect();
    assert_eq!(cards, vec![2, 1], "two distinct rows: first with cardinality 2, second 1");
}

/// (sq-v411r) Direct unit coverage of `expr_uses_multiplicity` across EVERY
/// arm of its expression-tree walk. This boolean is load-bearing for correctness — it is
/// the gate that decides whether `eval_aggregate` / `eval_custom_aggregate` take the
/// distinct-collapsing (`group_multiplicities`-built) path, so a missed/false detection
/// would silently fold the wrong multiset. The query-level tests above reach only the
/// `Multiply`, `FunctionCall(Custom)` and `Variable` arms; this exercises the leaves
/// (`NamedNode`/`Literal`/`Variable`/`Bound`/`Exists`), the binary/unary recursion, the
/// `If`/`In`/`Coalesce`/`FunctionCall(args)` arms, and — crucially — the `Exists`
/// non-descent contract (a `MULTIPLICITY()` buried inside an `EXISTS` belongs to that
/// sub-query's scope and must NOT be reported here).
#[test]
fn expr_uses_multiplicity_walks_every_arm() {
    use spargebra::algebra::{Function, GraphPattern};

    let mult = || {
        Expression::FunctionCall(
            Function::Custom(oxrdf::NamedNode::new_unchecked(MULTIPLICITY_FN_IRI)),
            vec![],
        )
    };
    let var = || Expression::Variable(Variable::new_unchecked("x"));
    let lit = || Expression::Literal(oxrdf::Literal::from(1));
    let bx = Box::new;

    // --- Leaves: never contain MULTIPLICITY() on their own. ---
    assert!(!expr_uses_multiplicity(&var()));
    assert!(!expr_uses_multiplicity(&lit()));
    assert!(!expr_uses_multiplicity(&Expression::NamedNode(oxrdf::NamedNode::new_unchecked(
        "http://ex/p"
    ))));
    assert!(!expr_uses_multiplicity(&Expression::Bound(Variable::new_unchecked("y"))));

    // A `MULTIPLICITY()` with arguments is NOT the reserved zero-arg builtin ⇒ false
    // (the `if args.is_empty()` guard), and a non-Custom function with no inner
    // MULTIPLICITY() is false too.
    let mult_with_args = Expression::FunctionCall(
        Function::Custom(oxrdf::NamedNode::new_unchecked(MULTIPLICITY_FN_IRI)),
        vec![var()],
    );
    assert!(!expr_uses_multiplicity(&mult_with_args), "zero-arg guard: args ⇒ not the builtin");

    // --- The matching leaf. ---
    assert!(expr_uses_multiplicity(&mult()), "the reserved zero-arg call IS detected");

    // --- Binary recursion (left and right branch). ---
    assert!(expr_uses_multiplicity(&Expression::Multiply(bx(var()), bx(mult()))));
    assert!(expr_uses_multiplicity(&Expression::Add(bx(mult()), bx(var()))));
    assert!(expr_uses_multiplicity(&Expression::Or(bx(var()), bx(mult()))));
    assert!(expr_uses_multiplicity(&Expression::And(bx(mult()), bx(var()))));
    assert!(expr_uses_multiplicity(&Expression::Equal(bx(var()), bx(mult()))));
    assert!(expr_uses_multiplicity(&Expression::SameTerm(bx(mult()), bx(var()))));
    assert!(expr_uses_multiplicity(&Expression::Greater(bx(var()), bx(mult()))));
    assert!(expr_uses_multiplicity(&Expression::Less(bx(mult()), bx(var()))));
    assert!(!expr_uses_multiplicity(&Expression::Subtract(bx(var()), bx(lit()))));

    // --- Unary recursion. ---
    assert!(expr_uses_multiplicity(&Expression::UnaryMinus(bx(mult()))));
    assert!(expr_uses_multiplicity(&Expression::Not(bx(mult()))));
    assert!(!expr_uses_multiplicity(&Expression::UnaryPlus(bx(var()))));

    // --- Ternary `If` (each of the three positions). ---
    assert!(expr_uses_multiplicity(&Expression::If(bx(mult()), bx(var()), bx(lit()))));
    assert!(expr_uses_multiplicity(&Expression::If(bx(var()), bx(mult()), bx(lit()))));
    assert!(expr_uses_multiplicity(&Expression::If(bx(var()), bx(lit()), bx(mult()))));
    assert!(!expr_uses_multiplicity(&Expression::If(bx(var()), bx(lit()), bx(var()))));

    // --- List arms: `In` (head + list), `Coalesce`, `FunctionCall(_, args)`. ---
    assert!(expr_uses_multiplicity(&Expression::In(bx(mult()), vec![var()])));
    assert!(expr_uses_multiplicity(&Expression::In(bx(var()), vec![lit(), mult()])));
    assert!(!expr_uses_multiplicity(&Expression::In(bx(var()), vec![lit()])));
    assert!(expr_uses_multiplicity(&Expression::Coalesce(vec![var(), mult()])));
    assert!(!expr_uses_multiplicity(&Expression::Coalesce(vec![var(), lit()])));
    assert!(expr_uses_multiplicity(&Expression::FunctionCall(Function::Str, vec![mult()])));
    assert!(!expr_uses_multiplicity(&Expression::FunctionCall(Function::Str, vec![var()])));

    // --- The `Exists` non-descent contract: a MULTIPLICITY() inside an EXISTS belongs to
    //     that sub-query's scope, so it must NOT be reported for THIS aggregate. ---
    let exists_with_mult = Expression::Exists(Box::new(GraphPattern::Filter {
        expr: mult(),
        inner: Box::new(GraphPattern::Bgp { patterns: vec![] }),
    }));
    assert!(
        !expr_uses_multiplicity(&exists_with_mult),
        "MULTIPLICITY() inside EXISTS belongs to the sub-query scope, not descended into"
    );
    // ...but a MULTIPLICITY() in the OUTER branch of an expression whose other branch is an
    // EXISTS is still detected (proves we recurse around, not into, the Exists).
    assert!(expr_uses_multiplicity(&Expression::And(bx(exists_with_mult), bx(mult()))));
}

/// (sq-v411r) Direct unit coverage of the `multiplicity` thread-local context
/// and its RAII `Guard`: `current()` is `None` outside any `set`; `set(k)` makes
/// `current()` read `Some(k)`; a NESTED `set` shadows and the inner guard's drop RESTORES
/// the outer value (the re-entrancy contract that stops an `EXISTS`/sub-query inside an
/// aggregate argument from observing a stale multiplicity); and dropping the outer guard
/// clears the context back to `None`.
#[test]
fn multiplicity_context_guard_nests_and_restores() {
    assert_eq!(multiplicity::current(), None, "no context outside an aggregate");
    {
        let _outer = multiplicity::set(3);
        assert_eq!(multiplicity::current(), Some(3));
        {
            let _inner = multiplicity::set(7);
            assert_eq!(multiplicity::current(), Some(7), "inner set shadows outer");
        }
        assert_eq!(multiplicity::current(), Some(3), "inner guard drop restores outer");
    }
    assert_eq!(multiplicity::current(), None, "outer guard drop clears the context");
}
