//! GH #6943 case 1: a quoted formula a backward (`<=`) rule RETURNS carries the bindings
//! the proof made for it. A formula is opaque to substitution, so a variable the proof
//! produced survived the projection onto the goal — and the substitution that bound it was
//! dropped the moment the projection ended, so the value named a variable nothing would
//! ever bind, and that name — not the term the proof found — became part of the derived
//! fact's term identity, its `why()` string and its `sparq-prov` IRI.
//!
//! Only what the PROOF produced is instantiated. A name the caller already binds, and a
//! name the goal itself writes, stay as they are: a formula the CALLER supplies is the
//! caller's to resolve, and baking one caller's value into it would change that term.
//! Each test below that fixes a caller's value in place is a regression guard for exactly
//! that, with the answer the engine gave before this change.
//!
//! The answers are not claimed to be EYE's — EYE copies a quoted formula's variables afresh
//! on each retrieval, which sparq does not. They are the base engine's own answers wherever
//! this change does not touch the shape, and otherwise the value the proof demonstrably
//! bound, named in each test.

use sparq_reason::n3::reason_n3_terms;

const P: &str = "@prefix : <http://ex/>. \
                 @prefix log: <http://www.w3.org/2000/10/swap/log#>. \
                 @prefix list: <http://www.w3.org/2000/10/swap/list#>.\n";

/// The `:out …` facts of the closure of `P` + `body`, as sorted `predicate object` debug
/// strings; `[]` when there are none.
fn out(body: &str) -> Vec<String> {
    let src = format!("{P}{body}");
    let c = reason_n3_terms(&src, None).unwrap_or_else(|e| panic!("{src}\n{e}"));
    let mut v: Vec<String> = c
        .facts
        .iter()
        .filter(|t| format!("{:?}", t[0]).contains("/out"))
        .map(|t| format!("{:?} {:?}", t[1], t[2]))
        .collect();
    v.sort();
    v
}

fn none() -> Vec<String> {
    Vec::new()
}

fn one(s: &str) -> Vec<String> {
    vec![s.to_string()]
}

/// `:out :list ( … )` of IRIs, as [`out`] prints it.
fn list(items: &[&str]) -> Vec<String> {
    let inner: Vec<String> = items.iter().map(|i| format!("Iri(\"http://ex/{i}\")")).collect();
    one(&format!("Iri(\"http://ex/list\") List([{}])", inner.join(", ")))
}

fn fired() -> Vec<String> {
    one("Iri(\"http://ex/fired\") Iri(\"http://ex/yes\")")
}

// ---------------------------------------------------------------- the fix itself

/// The issue's own case 1: `{ ?x :pattern ?f } <= { ( { ?x :p :b } ) list:member ?f }`
/// proved for `:c`. The proof binds the application's `?x` to `:c` through the goal, so the
/// formula it returns is `{ :c :p :b }`. It used to be `{ ?__bw<n>_x :p :b }` — the name of
/// a variable whose one binding went away with the proof.
#[test]
fn a_returned_formula_carries_the_binding_the_proof_made() {
    let got = out("{ ?x :pattern ?f } <= { ( { ?x :p :b } ) list:member ?f }.\n\
                   { :c :pattern ?f } => { :out :got ?f }.");
    assert_eq!(
        got,
        one("Iri(\"http://ex/got\") Formula([[Iri(\"http://ex/c\"), Iri(\"http://ex/p\"), Iri(\"http://ex/b\")]])")
    );
}

/// The review's own reproduction: the producer binds `?x` from a fact, and the formula it
/// returns must carry `:c`. Written as a formula in the CONCLUSION, so the value never
/// passes through a premise builtin at all.
#[test]
fn a_formula_in_a_conclusion_carries_the_premises_binding() {
    let got = out(":seed :value :c.\n\
                   { :cfg :cl { :a :q ?x } } <= { :seed :value ?x }.\n\
                   { :cfg :cl ?f } => { :out :got ?f }.");
    assert_eq!(
        got,
        one("Iri(\"http://ex/got\") Formula([[Iri(\"http://ex/a\"), Iri(\"http://ex/q\"), Iri(\"http://ex/c\")]])")
    );
}

/// A variable the proof did NOT bind stays a variable: `?z` is in the returned formula and
/// in no premise, so there is nothing to instantiate it to, and it keeps the one
/// `__bw<fold>_` name that makes this derivation's value its own. The sibling `?y` IS bound
/// and is instantiated, so the two halves of the same formula show both rules at once.
#[test]
fn an_unbound_variable_in_a_returned_formula_keeps_its_name() {
    let got = out(":s :p :v.\n\
                   { ?x :has { ?y :inner ?z } } <= { ?x :p ?y }.\n\
                   { :s :has ?f } => { :out :got ?f }.");
    assert_eq!(got.len(), 1, "{got:?}");
    let row = &got[0];
    assert!(row.contains("Iri(\"http://ex/v\"), Iri(\"http://ex/inner\")"), "?y instantiated: {row}");
    assert_eq!(row.matches("Var(\"__bw").count(), 1, "?z, and only ?z, survives: {row}");
    assert!(row.contains("_z\")"), "the surviving name is ?z's: {row}");
}

/// A quoted formula's OWN binder is not a variable any proof binds, so the instantiation
/// never reaches it: a formula-level `@forAll` survives a projection that instantiates the
/// rule's `?x` beside it.
#[test]
fn a_formula_local_universal_survives_the_projection() {
    let got = out(":seed :value :c.\n\
                   { :cfg :cl { @forAll :u. :u :q ?x } } <= { :seed :value ?x }.\n\
                   { :cfg :cl ?f } => { :out :got ?f }.");
    assert_eq!(got.len(), 1, "{got:?}");
    let row = &got[0];
    assert!(row.contains("Iri(\"http://ex/c\")"), "?x instantiated: {row}");
    assert!(row.contains("Var(\"__uf."), "the formula's own universal is kept: {row}");
}

// ------------------------------------------------- a caller's value is never touched

/// A FORWARD caller passes a formula as a goal argument. Its `?y` is the CALLER's, bound by
/// the caller's own premise, and the callee reads it through the inherited substitution —
/// so the clause has one solution and the template collects one item. Pinned at the answer
/// the engine gave before this change.
#[test]
fn a_forward_callers_formula_argument_keeps_its_own_binding() {
    let got = out(":k :v :b.\n\
                   { ?cl :coll ?l } <= { ( :hit ?cl ?l ) log:collectAllIn { :a :p :b. :a :p :c } }.\n\
                   { :k :v ?y . { :a :p ?y } :coll ?l } => { :out :list ?l }.");
    assert_eq!(got, list(&["hit"]));
}

/// The same, through `log:forAllIn`: the caller's `?y` is `:b` in both of the clauses it
/// passes, so every solution of the first extends to one of the second and the premise
/// holds. Pinned at the answer the engine gave before this change.
#[test]
fn a_forward_callers_clauses_keep_their_shared_binding() {
    let got = out(":k :v :b.\n\
                   { ?cs :allok true } <= { ?cs log:forAllIn { :a :p :b. :a :p :c. :b :ok true } }.\n\
                   { :k :v ?y . ( { :a :p ?y } { ?y :ok true } ) :allok true } => { :out :fired :yes }.");
    assert_eq!(got, fired());
}

/// Standardizing apart means the callee's `?y` is not the caller's, whatever the caller
/// spells it: the template is the callee's own `?y`, the clause the caller passed binds
/// only the caller's, so nothing ground is collected — the same answer for a caller that
/// spells it `?y`, one that spells it `?q`, and a backward caller.
#[test]
fn the_answer_does_not_depend_on_the_callers_spelling() {
    let callee = ":k :v :b.\n\
                  { ?cl :coll ?l } <= { ( ?y ?cl ?l ) log:collectAllIn { :a :p :b. :a :p :c } }.\n";
    for caller in [
        "{ :k :v ?y . { :a :p ?y } :coll ?l } => { :out :list ?l }.",
        "{ :k :v ?q . { :a :p ?q } :coll ?l } => { :out :list ?l }.",
        "{ :mark :m ?l } <= { :k :v ?y . { :a :p ?y } :coll ?l }.\n\
         { :mark :m ?l } => { :out :list ?l }.",
    ] {
        assert_eq!(out(&format!("{callee}{caller}")), none(), "{caller}");
    }
}

// ------------------------------------------- a written clause keeps its own seed

/// A variable inside a clause the rule WROTE is the rule's own, and what it is bound to is
/// ordinary data: here `?f` holds a stored formula that the clause matches against a stored
/// fact. Rewriting that data would stop the fact matching, the antecedent would be empty,
/// and `log:forAllIn` would hold vacuously — firing a rule that must not fire.
#[test]
fn a_data_binding_inside_a_written_forall_clause_is_not_rewritten() {
    let got = out(":cfg :f { ?z :p :b }. :a :says { ?z :p :b }.\n\
                   { :mark :m :yes } <= { :cfg :f ?f. \
                   ( { :a :says ?f } { :missing :q :yes } ) log:forAllIn ?scope }.\n\
                   { :mark :m :yes } => { :out :fired :yes }.");
    assert_eq!(got, none(), "the antecedent has a solution, and the consequent has none");
}

/// The `log:collectAllIn` twin of the test above: the written clause matches the stored
/// fact, so the template is collected once. Rewriting the data would make the clause match
/// nothing and collect the empty list.
#[test]
fn a_data_binding_inside_a_written_collect_clause_is_not_rewritten() {
    let got = out(":cfg :f { ?z :p :b }. :a :says { ?z :p :b }.\n\
                   { :mark :m ?l } <= { :cfg :f ?f. \
                   ( :hit { :a :says ?f } ?l ) log:collectAllIn ?scope }.\n\
                   { :mark :m ?l } => { :out :list ?l }.");
    assert_eq!(got, list(&["hit"]));
}

// ------------------------------------------------------- no spelling is special

/// Nothing anywhere reads a variable's NAME to decide whose it is, so a document that
/// spells a variable in the shape the chainer's own renaming uses (`?__bw<digits>_…`) is
/// answered exactly as the same document with an ordinary name. A name test here would
/// make the two differ, and a document is entitled to that spelling.
#[test]
fn a_document_that_spells_a_generated_name_is_answered_the_same() {
    let doc = |y: &str| {
        format!(
            ":cfg :cl {{ :a :p {y} }}.\n\
             {{ :mark :m :yes }} <= {{ :cfg :cl ?cl. \
             ( ?cl {{ :a :q {y} }} ) log:forAllIn {{ :a :p :b. :a :p :c. :a :q :b }} }}.\n\
             {{ :mark :m :yes }} => {{ :out :fired :yes }}."
        )
    };
    assert_eq!(out(&doc("?y")), out(&doc("?__bw1_y")));
    assert_eq!(out(&doc("?y")), out(&doc("?__bw99999999999999999999999_y")));
}

/// A stored formula that reaches the callee through the GOAL is a caller's value like any
/// other: the callee's template is its own variable, so nothing is collected. Pinned at the
/// answer the engine gave before this change.
#[test]
fn a_stored_formula_passed_through_the_goal_binds_nothing_of_the_callees() {
    let got = out(":cfg :cl { :a :p ?y }.\n\
                   { ?cl :coll ?l } <= { ( ?y ?cl ?l ) log:collectAllIn { :a :p :b. :a :p :c } }.\n\
                   { :cfg :cl ?cl. ?cl :coll ?l } => { :out :list ?l }.");
    assert_eq!(got, none());
}

/// And a stored formula an application's OWN atom reads is unchanged too: a clause given
/// through a variable shares nothing with the rule's variables behind a `<=`, exactly as
/// before this change. (It DOES share them in the forward rule below, because a forward
/// rule's atoms are never standardized apart and the document's `?y` is then one variable
/// — the forward/backward difference this PR no longer tries to close; see the PR body.)
#[test]
fn a_stored_clause_read_by_the_applications_own_atom_is_unchanged() {
    let data = ":cfg :cl { :a :p ?y }.\n";
    let prem = ":cfg :cl ?cl. ( ?y ?cl ?l ) log:collectAllIn { :a :p :b. :a :p :c }";
    let bwd = out(&format!(
        "{data}{{ :mark :m ?l }} <= {{ {prem} }}.\n{{ :mark :m ?l }} => {{ :out :list ?l }}."
    ));
    assert_eq!(bwd, none(), "backward: the rule's own variables were standardized apart");
    let fwd = out(&format!("{data}{{ {prem} }} => {{ :out :list ?l }}."));
    assert_eq!(fwd, list(&["b", "c"]), "forward: one document variable, shared");
}

/// A name the CALLER binds is the caller's to resolve. Here the callee hands the caller's
/// own formula straight back, and the caller has `?y` bound to `:b`: the returned term is
/// the formula the caller wrote, not one with that caller's value baked in. Substituting it
/// would make the derived fact — and its identity, its `why()` and its `sparq-prov` IRI —
/// depend on which caller asked, for a term no proof built.
#[test]
fn a_name_the_caller_binds_is_left_to_the_caller() {
    let got = out(":k :v :b. :any :p :p.\n\
                   { ?cl :echo ?cl } <= { :any :p :p }.\n\
                   { :k :v ?y . { :a :p ?y } :echo ?e } => { :out :got ?e }.");
    assert_eq!(
        got,
        one("Iri(\"http://ex/got\") Formula([[Iri(\"http://ex/a\"), Iri(\"http://ex/p\"), Var(\"y\")]])")
    );
}

/// A name the GOAL itself writes is the caller's too, even when the proof binds it:
/// resolving a goal against a conclusion binds a free goal variable to one of the
/// application's, so such a name IS in the proof's substitution. Here the goal's output
/// variable also occurs inside the formula the goal passes, so instantiating it would nest
/// the returned formula inside itself.
#[test]
fn a_name_the_goal_writes_is_left_alone_even_when_the_proof_binds_it() {
    let got = out(":any :p :p.\n\
                   { ?cl :echo ?cl } <= { :any :p :p }.\n\
                   { { :a :p ?e } :echo ?e } => { :out :got ?e }.");
    assert_eq!(
        got,
        one("Iri(\"http://ex/got\") Formula([[Iri(\"http://ex/a\"), Iri(\"http://ex/p\"), Var(\"e\")]])")
    );
}

/// The caller's binding domain is a wider exclusion than the goal's own names, and this is
/// the case that separates them: the callee reads a STORED formula naming `?y` and returns
/// it, the caller binds `?y` to something of its own, and the goal never mentions `?y` at
/// all. The stored term must come back as it is stored. Instantiating it would leak one
/// caller's unrelated binding into a document's own term.
#[test]
fn a_caller_binding_the_goal_never_mentions_does_not_leak_into_a_stored_formula() {
    let got = out(":cfg :f { :a :p ?y }. :k :v :b.\n\
                   { :any :has ?f } <= { :cfg :f ?f }.\n\
                   { :k :v ?y . :any :has ?g } => { :out :got ?g }.");
    assert_eq!(
        got,
        one("Iri(\"http://ex/got\") Formula([[Iri(\"http://ex/a\"), Iri(\"http://ex/p\"), Var(\"y\")]])")
    );
}
