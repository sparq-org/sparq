//! GH #6943: a `log:collectAllIn` / `log:forAllIn` subject, subject member or scope given
//! THROUGH A VARIABLE inside a backward (`<=`) rule is resolved into that rule
//! application's variable namespace, so it shares the rule's bindings exactly as the same
//! formula written in the rule does — and exactly as either form does in a forward rule,
//! whose atoms are never standardized apart.
//!
//! Every test here is a PARITY table over the four forms the same aggregation can take:
//! forward or backward, clause written inline or arriving through a variable. The answers
//! are the ones EYE gives, derived by hand from the scope each test states; the table is
//! the reference, and the forward/inline cell is the one this change does not touch.

use sparq_reason::n3::reason_n3_terms;

const EX: &str = "http://ex/";
const P: &str = "@prefix : <http://ex/>. \
                 @prefix log: <http://www.w3.org/2000/10/swap/log#>. \
                 @prefix list: <http://www.w3.org/2000/10/swap/list#>.\n";

/// The `:out …` facts of a closure, as sorted debug strings.
fn out(src: &str) -> Vec<String> {
    let c = reason_n3_terms(src, None).unwrap_or_else(|e| panic!("{src}\n{e}"));
    let mut v: Vec<String> = c
        .facts
        .iter()
        .filter(|t| t[0] == sparq_reason::n3::Term::Iri(format!("{EX}out")))
        .map(|t| format!("{:?} {:?}", t[1], t[2]))
        .collect();
    v.sort();
    v
}

/// `premise` run as a FORWARD rule concluding `:out :fired :yes`, and behind a BACKWARD
/// rule whose conclusion a forward rule then consumes. The marker predicate is distinct
/// from anything the premise reads, so the backward form adds no other difference.
fn both(data: &str, premise: &str) -> (Vec<String>, Vec<String>) {
    let fwd = format!("{P}{data}\n{{ {premise} }} => {{ :out :fired :yes }}.\n");
    let bwd = format!(
        "{P}{data}\n{{ :mark :proved :yes }} <= {{ {premise} }}.\n\
         {{ :mark :proved :yes }} => {{ :out :fired :yes }}.\n"
    );
    (out(&fwd), out(&bwd))
}

/// As [`both`], for a premise that binds `?l`: the conclusion carries it, so the test sees
/// WHAT was collected and not merely that something was.
fn both_collecting(data: &str, premise: &str) -> (Vec<String>, Vec<String>) {
    let fwd = format!("{P}{data}\n{{ {premise} }} => {{ :out :list ?l }}.\n");
    let bwd = format!(
        "{P}{data}\n{{ :mark :list ?l }} <= {{ {premise} }}.\n\
         {{ :mark :list ?l }} => {{ :out :list ?l }}.\n"
    );
    (out(&fwd), out(&bwd))
}

fn fired() -> Vec<String> {
    vec![format!("Iri(\"{EX}fired\") Iri(\"{EX}yes\")")]
}

/// `:out :list ( :b :c )`, as [`out`] prints it.
fn collected_bc() -> Vec<String> {
    vec![format!("Iri(\"{EX}list\") List([Iri(\"{EX}b\"), Iri(\"{EX}c\")])")]
}

/// A `log:collectAllIn` clause arriving through a variable collects the template's
/// instantiations, in a backward rule as in a forward one and as the inline clause does.
///
/// `{ :a :p :b. :a :p :c } ` has two solutions of `{ :a :p ?y }`, so the template `?y`
/// collects `( :b :c )` — the clause's `?y` and the template's `?y` are one variable.
/// Behind a `<=` the rule's own `?y` was renamed, so without resolving the arriving clause
/// into the application the template is never bound and nothing is collected at all.
#[test]
fn a_bound_collect_clause_shares_the_rules_variables() {
    let data = ":cfg :cl { :a :p ?y }.";
    for clause in ["{ :a :p ?y }", "?cl"] {
        let premise =
            format!(":cfg :cl ?cl. ( ?y {clause} ?l ) log:collectAllIn {{ :a :p :b. :a :p :c }}");
        let (fwd, bwd) = both_collecting(data, &premise);
        assert_eq!(fwd, collected_bc(), "forward, clause {clause}");
        assert_eq!(bwd, collected_bc(), "backward, clause {clause}");
    }
}

/// The whole SUBJECT list arriving through a variable: its template, its clause and its
/// result pattern are resolved into ONE namespace — the application's — so the clause's
/// solutions bind the template and the rule's own `?l` receives the list.
#[test]
fn a_bound_collect_subject_resolves_template_clause_and_result_together() {
    let data = ":cfg :args ( ?y { :a :p ?y } ?l ).";
    for subject in ["( ?y { :a :p ?y } ?l )", "?args"] {
        let premise =
            format!(":cfg :args ?args. {subject} log:collectAllIn {{ :a :p :b. :a :p :c }}");
        let (fwd, bwd) = both_collecting(data, &premise);
        assert_eq!(fwd, collected_bc(), "forward, subject {subject}");
        assert_eq!(bwd, collected_bc(), "backward, subject {subject}");
    }
}

/// `log:forAllIn` is the ANSWER-SAFETY case: it holds iff every solution of the first
/// clause extends to one of the second, so a first clause that shares no variable with the
/// second holds vacuously and fires a rule that must not fire.
///
/// Scope `{ :a :p :b. :a :p :c. :a :q :b }`: `{ :a :p ?y }` has the solutions `?y = :b` and
/// `?y = :c`, and `{ :a :q ?y }` extends only the first, so the premise is FALSE. Before
/// this change the bound-clause backward form derived `:out :fired :yes` — the one wrong
/// ANSWER this change removes, rather than an answer it adds.
#[test]
fn a_bound_forall_clause_is_not_vacuously_true() {
    let data = ":cfg :cl { :a :p ?y }.";
    let scope = "{ :a :p :b. :a :p :c. :a :q :b }";
    for clause in ["{ :a :p ?y }", "?cl"] {
        let premise =
            format!(":cfg :cl ?cl. ( {clause} {{ :a :q ?y }} ) log:forAllIn {scope}");
        let (fwd, bwd) = both(data, &premise);
        assert_eq!(fwd, Vec::<String>::new(), "forward, clause {clause}");
        assert_eq!(bwd, Vec::<String>::new(), "backward, clause {clause}");
    }
}

/// The positive control for the test above: with `:a :q :c` in the scope too, every
/// solution of the first clause extends, and all four forms hold.
#[test]
fn a_bound_forall_clause_holds_when_every_solution_extends() {
    let data = ":cfg :cl { :a :p ?y }.";
    let scope = "{ :a :p :b. :a :p :c. :a :q :b. :a :q :c }";
    for clause in ["{ :a :p ?y }", "?cl"] {
        let premise =
            format!(":cfg :cl ?cl. ( {clause} {{ :a :q ?y }} ) log:forAllIn {scope}");
        let (fwd, bwd) = both(data, &premise);
        assert_eq!(fwd, fired(), "forward, clause {clause}");
        assert_eq!(bwd, fired(), "backward, clause {clause}");
    }
}

/// A clause the PROOF ITSELF built keeps the names it carries. `( { :a :p ?y } ) list:member
/// ?cl` inside a backward rule hands `?cl` a formula over the application's OWN renamed
/// `?y` — the name its binding is held under. Resolving that name again would point it at a
/// variable nothing has bound and the template would collect unbound terms, so the
/// resolution leaves a name the chainer minted alone.
#[test]
fn a_clause_the_application_built_keeps_its_own_names() {
    let premise = "( { :a :p ?y } ) list:member ?cl. \
                   ( ?y ?cl ?l ) log:collectAllIn { :a :p :b. :a :p :c }";
    let (fwd, bwd) = both_collecting("", premise);
    assert_eq!(fwd, collected_bc(), "forward");
    assert_eq!(bwd, collected_bc(), "backward");
}

/// The resolution is the innermost APPLICATION's, and a step inside one stays inside it: a
/// `log:collectAllIn` with a bound clause nested in a store-scoped `log:includes` clause of
/// a backward rule resolves into that rule's application, so it collects as it does at the
/// rule's top level.
#[test]
fn a_bound_clause_nested_in_a_store_scoped_clause_resolves_into_the_application() {
    let data = ":cfg :cl { :a :p ?y }. :a :p :b. :a :p :c. ( :b :c ) :is :ok.";
    let inner = "?any log:includes { :cfg :cl ?cl. \
                 ( ?y ?cl ?l ) log:collectAllIn { :a :p :b. :a :p :c }. ?l :is :ok }";
    let (fwd, bwd) = both(data, inner);
    assert_eq!(fwd, fired(), "forward");
    assert_eq!(bwd, fired(), "backward");
}

/// Two levels of application: the goal is proved by a backward rule whose own premise is
/// proved by a second one, and the bound clause in the inner rule resolves into the INNER
/// application (the one that renamed the atoms around it).
#[test]
fn a_bound_clause_resolves_into_the_innermost_application() {
    let src = format!(
        "{P}:cfg :cl {{ :a :p ?y }}.\n\
         {{ :mark :outer ?l }} <= {{ :mark :inner ?l }}.\n\
         {{ :mark :inner ?l }} <= {{ :cfg :cl ?cl. ( ?y ?cl ?l ) log:collectAllIn {{ :a :p :b. :a :p :c }} }}.\n\
         {{ :mark :outer ?l }} => {{ :out :list ?l }}.\n"
    );
    assert_eq!(out(&src), collected_bc());
}

/// A formula-local `@forAll` keeps its own scope: it is bound inside its formula, so the
/// resolution leaves it alone and the quantified-antecedent refusal a bound clause carrying
/// one already had still fails the run, in a backward rule as in a forward one.
#[test]
fn a_bound_clause_with_a_formula_local_universal_is_still_refused() {
    let data = ":cfg :cl { @forAll :y. :a :p :y }.";
    let premise = ":cfg :cl ?cl. ( ?y ?cl ?l ) log:collectAllIn { :a :p :b }";
    for src in [
        format!("{P}{data}\n{{ {premise} }} => {{ :out :fired :yes }}.\n"),
        format!(
            "{P}{data}\n{{ :mark :proved :yes }} <= {{ {premise} }}.\n\
             {{ :mark :proved :yes }} => {{ :out :fired :yes }}.\n"
        ),
    ] {
        let e = reason_n3_terms(&src, None)
            .err()
            .expect("a quantified bound clause is refused");
        assert!(e.contains("universally quantified antecedents are unsupported"), "{e}");
    }
}

/// A clause arriving through a variable binds the rule's variables and nothing else: a
/// variable the stored clause names that the rule does not is its own, so it neither reads
/// nor writes a binding of the rule's.
///
/// `{ :a :p ?z }` arrives where the rule writes `?y`: the clause has two solutions, the
/// template `?y` is bound by neither, and the aggregation collects nothing the rule can
/// use — the same in both directions.
#[test]
fn a_clause_variable_the_rule_does_not_write_binds_nothing_of_the_rules() {
    let data = ":cfg :cl { :a :p ?z }.";
    let premise = ":cfg :cl ?cl. ( ?y ?cl ?l ) log:collectAllIn { :a :p :b. :a :p :c }";
    let (fwd, bwd) = both_collecting(data, premise);
    assert_eq!(fwd, Vec::<String>::new(), "forward");
    assert_eq!(bwd, Vec::<String>::new(), "backward");
}
