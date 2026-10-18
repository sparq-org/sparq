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

/// The run's refusal, for a shape that must fail closed rather than answer.
fn err(body: &str) -> String {
    let src = format!("{P}{body}");
    match reason_n3_terms(&src, None) {
        Ok(_) => panic!("{src}\nexpected a refusal, got an answer"),
        Err(e) => e.to_string(),
    }
}

/// The refusal an executable pattern that arrived through a variable and is not ground
/// inside a backward proof fails with.
const NOT_GROUND: &str = "bound through a variable inside a backward rule must be ground";

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
    // An exact match, not a `contains`: asserting only that SOME `__uf.` name is present
    // cannot fail, because nothing in the engine ever substitutes one. What has to hold is
    // that the universal is kept WHILE `?x` beside it is instantiated, and that the
    // universal is the one its own formula declared.
    assert_eq!(
        got,
        one(
            "Iri(\"http://ex/got\") \
             Formula([[Var(\"__uf.0.http://ex/u\"), Iri(\"http://ex/q\"), Iri(\"http://ex/c\")]])"
        ),
        "the universal kept, ?x instantiated beside it"
    );
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

/// The same symmetry for the UNBOUND-ALIAS case, where the caller's variable is in the
/// binding only because head unification aliased it to a callee variable nothing has bound.
/// Both spellings must reach the same verdict; the first revision of this PR refused the `?q`
/// spelling and let the `?y` spelling FIRE, which made a caller's arbitrary choice of name
/// decide a callee's soundness.
#[test]
fn the_verdict_does_not_depend_on_the_callers_spelling_through_an_alias() {
    for (op, caller) in [("forAllIn", "ok"), ("collectAllIn", "coll")] {
        let callee = match op {
            "forAllIn" => format!(
                ":cfg :cl {{ :a :p ?y }}.\n\
                 {{ ?w :{caller} :yes }} <= {{ :cfg :cl ?cl . ( ?cl {{ :a :q ?y }} ) \
                 log:forAllIn {{ :a :p :b. :a :p :c. :a :q :b }} }}.\n"
            ),
            _ => format!(
                ":cfg :cl {{ :a :p ?y }}.\n\
                 {{ ?w :{caller} ?l }} <= {{ :cfg :cl ?cl . ( ?y ?cl ?l ) \
                 log:collectAllIn {{ :a :p :b. :a :p :c }} }}.\n"
            ),
        };
        let head = |v: &str| match op {
            "forAllIn" => format!("{{ {v} :{caller} :yes }} => {{ :out :fired :yes }}."),
            _ => format!("{{ {v} :{caller} ?l }} => {{ :out :list ?l }}."),
        };
        let y = err(&format!("{callee}{}", head("?y")));
        let q = err(&format!("{callee}{}", head("?q")));
        assert!(y.contains(NOT_GROUND), "{op} ?y: {y}");
        assert_eq!(y, q, "{op}: the two spellings must agree");
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

// ------------------------------- answer safety: the shape that must fail closed

/// THE answer-safety case. `:cfg :cl { :a :p ?y }` is read through `?cl`, and the rule
/// writes `{ :a :q ?y }` beside it. The document means one `?y`: every `:a :p ?y` solution
/// must extend to `:a :q ?y`, and over this scope `:a :p :c` has no `:a :q :c`, so
/// `log:forAllIn` must NOT hold and `:mark :m :yes` must not be derived.
///
/// Behind a `<=` the rule's own `{ :a :q ?y }` was renamed and the arriving clause's `?y`
/// was not, so `?y` is left FREE in the arriving clause: nothing the proof holds can bind
/// it, and the clause's own matching will not either. The first clause then constrains
/// nothing the second reads, every solution extends trivially, `forAllIn` holds VACUOUSLY
/// and the rule FIRES — which `origin/main` does. There is no sound reading, so the shape
/// is refused.
#[test]
fn a_bound_forall_clause_that_is_not_ground_is_refused() {
    let e = err(":cfg :cl { :a :p ?y }.\n\
                 { :mark :m :yes } <= { :cfg :cl ?cl. \
                 ( ?cl { :a :q ?y } ) log:forAllIn { :a :p :b. :a :p :c. :a :q :b } }.\n\
                 { :mark :m :yes } => { :out :fired :yes }.");
    assert!(e.contains(NOT_GROUND), "{e}");
    // The same document with the clause written INLINE is the rule's own text, renamed with
    // the rest of it, so it shares correctly and is answered: it does not fire.
    let inline = out(":cfg :cl { :a :p ?y }.\n\
                      { :mark :m :yes } <= { \
                      ( { :a :p ?y } { :a :q ?y } ) log:forAllIn { :a :p :b. :a :p :c. :a :q :b } }.\n\
                      { :mark :m :yes } => { :out :fired :yes }.");
    assert_eq!(inline, none(), "inline: one variable, shared, so the premise fails");
}

/// The `log:collectAllIn` twin: the arriving clause leaves `?y` free, so the template it is
/// meant to bind is never bound and the aggregation collects nothing — silently, with no
/// sign the question was not answered (`origin/main` answers `( )` here).
#[test]
fn a_bound_collect_clause_that_is_not_ground_is_refused() {
    let e = err(":cfg :cl { :a :p ?y }.\n\
                 { :mark :m ?l } <= { :cfg :cl ?cl. \
                 ( ?y ?cl ?l ) log:collectAllIn { :a :p :b. :a :p :c } }.\n\
                 { :mark :m ?l } => { :out :list ?l }.");
    assert!(e.contains(NOT_GROUND), "{e}");
    // Inline, the same shape is the rule's own text and collects both.
    let inline = out("{ :mark :m ?l } <= { \
                      ( ?y { :a :p ?y } ?l ) log:collectAllIn { :a :p :b. :a :p :c } }.\n\
                      { :mark :m ?l } => { :out :list ?l }.");
    assert_eq!(inline, list(&["b", "c"]), "inline: the template is bound by the clause");
}

/// Case (1): the projection follows an ALIAS CHAIN to the value the proof found. Head
/// unification aliases the goal's one `?t` to both of the head's variables, and the premise
/// then binds the far end to `:c`, so `?x` inside the returned formula reaches `:c` only
/// through `?y`. Read one hop deep it comes back as the alias — `origin/main` derives
/// `{ :a :q ?__bw<n>_x }` here — and the derived fact names a variable instead of a term.
#[test]
fn an_alias_chain_is_followed_to_the_value_the_proof_found() {
    let got = out(":seed :value :c.\n\
                   { ( ?x ?y ) :cl { :a :q ?x } } <= { :seed :value ?y }.\n\
                   { ( ?t ?t ) :cl ?cl } => { :out :got ?cl }.");
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(got[0].contains("Iri(\"http://ex/c\")"), "the value, not the alias: {}", got[0]);
    assert!(!got[0].contains("Var("), "no variable survives: {}", got[0]);
}

// ------------------------------- answer safety: the shape that must fail closed

/// Case (2), and the reviewer's own repro. The caller's `?y` IS in the binding, but only
/// because head unification aliased it to the callee's renamed `?w`, which nothing has
/// bound. A "is this name in the binding?" test reads that as constrained and lets the
/// unsound firing through; following the chain to a non-variable shows it is still free.
///
/// `origin/main` FIRES here, and so did the first revision of this PR when the caller spelled
/// its variable `?y`.
#[test]
fn an_unbound_alias_does_not_bypass_the_refusal() {
    let e = err(":cfg :cl { :a :p ?y }.\n\
                 { ?w :ok :yes } <= { :cfg :cl ?cl . ( ?cl { :a :q ?y } ) \
                 log:forAllIn { :a :p :b. :a :p :c. :a :q :b } }.\n\
                 { ?y :ok :yes } => { :out :fired :yes }.");
    assert!(e.contains(NOT_GROUND), "{e}");
}

/// Case (5): a CONTAINMENT pattern that arrived through a variable and is not ground. Inside
/// a backward proof it used to fall back to the EMPTY pattern, which makes `log:includes`
/// hold of everything and `log:notIncludes` of nothing — on `origin/main` the `includes` rule
/// FIRES and the `notIncludes` rule does not, both wrong, and both over a pattern
/// (`{ :a :p ?z }`) the scope `{ :a :p :b }` was never tested against. Refused now.
///
/// The same premise in a FORWARD rule keeps main's behaviour, which
/// `tests/n3_bound_containment.rs` pins; nothing here touches it.
#[test]
fn a_bound_containment_pattern_that_is_not_ground_is_refused() {
    for op in ["includes", "notIncludes"] {
        let e = err(&format!(
            ":cfg :pat {{ :a :p ?z }}.\n\
             {{ :mark :m :yes }} <= {{ :cfg :pat ?p . {{ :a :p :b }} log:{op} ?p }}.\n\
             {{ :mark :m :yes }} => {{ :out :fired :yes }}."
        ));
        assert!(e.contains(NOT_GROUND), "{op}: {e}");
    }
}

/// Case (6): a formula's OWN binder is not a free variable. `__uf.` names are bound inside
/// their formula, no substitution reaches them, and matching treats them as constants — so a
/// value carrying one is ground for this purpose and must NOT be refused. The answer is
/// `origin/main`'s.
#[test]
fn a_formula_local_binder_is_not_a_free_variable() {
    let got = out(":cfg :t { @forAll :u. :u :p :b }.\n\
                   { :mark :m ?l } <= { :cfg :t ?tm . ( ?tm { :a :p :b } ?l ) \
                   log:collectAllIn { :a :p :b } }.\n\
                   { :mark :m ?l } => { :out :list ?l }.");
    assert_eq!(got, none(), "accepted, and answered as on main");
}

/// Groundness is decided after following each variable to the END of its chain. Here the
/// arriving clause's `?y` is ground only through an alias: head unification binds it to the
/// callee's `?w`, and the premise then binds that to `:b`. A one-hop test reads the alias as
/// a free variable and REFUSES a clause the proof did in fact ground, so the walk is
/// transitive. The answer is `origin/main`'s.
#[test]
fn a_clause_ground_only_through_an_alias_is_accepted() {
    let got = out(":cfg :cl { :a :p ?y }. :pick :one :b.\n\
                   { ?w :coll ?l } <= { :pick :one ?w . :cfg :cl ?cl . \
                   ( :hit ?cl ?l ) log:collectAllIn { :a :p :b } }.\n\
                   { ?y :coll ?l } => { :out :list ?l }.");
    assert_eq!(got, list(&[]), "accepted, and answered as on main");
}

/// A GROUND clause that arrived through a variable has no free variable to leave unbound, so
/// it is matched as stored exactly as before — the refusal is about groundness, not about
/// having arrived.
#[test]
fn a_ground_bound_clause_is_still_matched_as_stored() {
    let got = out(":cfg :cl { :a :p :b }.\n\
                   { :mark :m ?l } <= { :cfg :cl ?c . ( :hit ?c ?l ) \
                   log:collectAllIn { :a :p :b } }.\n\
                   { :mark :m ?l } => { :out :list ?l }.");
    assert_eq!(got, list(&["hit"]));
}

// --------------------------------------- a SCOPE is searched data, not a pattern

/// A stored SCOPE holding a variable is searched, not refused: the scope is the data a clause
/// is matched against, not a pattern that has to match, and a free variable in it is simply a
/// statement nothing matches. All three of these are `origin/main`'s answers, and the first
/// two agree with the forward twin — requiring the scope to be ground refused them instead.
#[test]
fn a_stored_scope_holding_a_variable_is_searched_not_refused() {
    let prem = ":k :v ?x . :cfg :sc ?s . ( ?t { ?t :type :T } ?l ) log:collectAllIn ?s";
    let data = ":cfg :sc { :a :type :T . :b :type :T . ?x :knows :a }. :k :v :w.\n";
    let bwd = out(&format!(
        "{data}{{ :mark :m ?l }} <= {{ {prem} }}.\n{{ :mark :m ?l }} => {{ :out :list ?l }}."
    ));
    assert_eq!(bwd, list(&["a", "b"]), "backward");
    let fwd = out(&format!("{data}{{ {prem} }} => {{ :out :list ?l }}."));
    assert_eq!(fwd, list(&["a", "b"]), "forward: the same answer");
    let fa = out(":cfg :sc { :a :type :T . ?x :knows :a }. :k :v :w.\n\
                  { :mark :m :yes } <= { :k :v ?x . :cfg :sc ?s . \
                  ( { ?t :type :T } { :missing :q :yes } ) log:forAllIn ?s }.\n\
                  { :mark :m :yes } => { :out :fired :yes }.");
    assert_eq!(fa, none(), "forAllIn over a stored scope: main's answer");
}

// ------------------------------------------------------- no spelling is special

/// Nothing anywhere reads a variable's NAME to decide whose it is, so a document that
/// spells a variable in the shape the chainer's own renaming uses (`?__bw<digits>_…`) is
/// answered exactly as the same document with an ordinary name. A name test here would
/// make the two differ, and a document is entitled to that spelling.
///
/// This document's shape is the one that now fails closed, so what must match across the
/// spellings is the REFUSAL: were the decision made from a name's text, the `?__bw1_y`
/// spelling would be taken for a generated name and answered differently.
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
    assert!(err(&doc("?y")).contains(NOT_GROUND), "{}", err(&doc("?y")));
    assert_eq!(err(&doc("?y")), err(&doc("?__bw1_y")));
    assert_eq!(err(&doc("?y")), err(&doc("?__bw99999999999999999999999_y")));
}

/// A stored formula reaching the callee through the GOAL, still holding a free `?y` once the
/// binding is followed out. The aggregation would collect nothing, and collecting nothing is
/// a wrong ANSWER, not a safe one, so it is refused.
#[test]
fn a_stored_non_ground_clause_reaching_the_callee_is_refused() {
    let e = err(":cfg :cl { :a :p ?y }.\n\
                 { ?cl :coll ?l } <= { ( ?y ?cl ?l ) log:collectAllIn { :a :p :b. :a :p :c } }.\n\
                 { :cfg :cl ?cl. ?cl :coll ?l } => { :out :list ?l }.");
    assert!(e.contains(NOT_GROUND), "{e}");
}

/// The SAME premise, forward and backward: the divergence that is now fail-closed. A
/// forward rule's atoms are never standardized apart, so the document's `?y` is one variable
/// there, the arriving clause's `?y` is ground by the time it matters, and the answer is
/// `( :b :c )` — unchanged. Behind a `<=` the atoms are renamed, `?y` stays free, and the
/// aggregation would silently collect nothing; that half is refused rather than answered.
#[test]
fn the_forward_answer_stands_and_the_backward_one_is_refused() {
    let data = ":cfg :cl { :a :p ?y }.\n";
    let prem = ":cfg :cl ?cl. ( ?y ?cl ?l ) log:collectAllIn { :a :p :b. :a :p :c }";
    let e = err(&format!(
        "{data}{{ :mark :m ?l }} <= {{ {prem} }}.\n{{ :mark :m ?l }} => {{ :out :list ?l }}."
    ));
    assert!(e.contains(NOT_GROUND), "{e}");
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
