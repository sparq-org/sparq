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

/// A caller passes a formula as a goal argument, and its `?y` is a name the caller's own
/// premise also binds. The formula is still a QUOTED formula, so its `?y` is local to it: the
/// callee renames it apart and the clause has one solution per matching scope statement. The
/// caller's `:b` is not substituted into it, which is what keeps the answer the same for
/// every spelling the caller might have picked.
///
/// `origin/main` collects one item here, by reading the stored formula's `?y` out of the
/// caller's binding.
#[test]
fn a_callers_formula_argument_is_matched_as_its_own_formula() {
    let doc = |v: &str| {
        format!(
            ":k :v :b.\n\
             {{ ?cl :coll ?l }} <= {{ ( :hit ?cl ?l ) log:collectAllIn {{ :a :p :b. :a :p :c }} }}.\n\
             {{ :k :v {v} . {{ :a :p {v} }} :coll ?l }} => {{ :out :list ?l }}."
        )
    };
    assert_eq!(out(&doc("?y")), list(&["hit", "hit"]), "one solution per scope statement");
    assert_eq!(out(&doc("?y")), out(&doc("?q")), "the two spellings must agree");
}

/// The same through `log:forAllIn`. The caller passes two clauses that share a `?y` it has
/// also bound; the two clauses share it with EACH OTHER, because one renaming serves the
/// whole arriving value, but neither reads the caller's `:b`. `:a :p :c` then has no
/// `:c :ok true` in the scope, so the premise fails. `origin/main` fires, by reading `?y` out
/// of the caller's binding and so testing only `:b`.
#[test]
fn a_callers_clauses_share_with_each_other_not_with_the_caller() {
    let doc = |v: &str| {
        format!(
            ":k :v :b.\n\
             {{ ?cs :allok true }} <= {{ ?cs log:forAllIn {{ :a :p :b. :a :p :c. :b :ok true }} }}.\n\
             {{ :k :v {v} . ( {{ :a :p {v} }} {{ {v} :ok true }} ) :allok true }} => {{ :out :fired :yes }}."
        )
    };
    assert_eq!(out(&doc("?y")), none(), ":a :p :c has no :c :ok true");
    assert_eq!(out(&doc("?y")), out(&doc("?q")), "the two spellings must agree");
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

/// A caller's choice of variable NAME must not change a callee's answer, even when the name
/// it picks is one a formula the callee reads also uses. `origin/main` answers these two
/// documents differently — `( :hit )` against `( )` for `collectAllIn`, fired against not
/// fired for `forAllIn` — because the caller's `?y`, aliased into the proof, was read as the
/// stored formula's `?y` as well. Renaming the arriving formula's own variables apart makes
/// the two spellings one answer.
#[test]
fn the_answer_does_not_depend_on_the_callers_spelling_through_an_alias() {
    let collect = |v: &str| {
        format!(
            ":cfg :cl {{ :a :p ?y }}. :pick :one :b.\n\
             {{ ?w :coll ?l }} <= {{ :pick :one ?w . :cfg :cl ?cl . \
             ( :hit ?cl ?l ) log:collectAllIn {{ :a :p :b }} }}.\n\
             {{ {v} :coll ?l }} => {{ :out :list ?l }}."
        )
    };
    assert_eq!(out(&collect("?y")), list(&["hit"]), "collectAllIn, caller spells ?y");
    assert_eq!(out(&collect("?y")), out(&collect("?q")), "collectAllIn: the spellings agree");

    let forall = |v: &str| {
        format!(
            ":cfg :cl {{ :a :p ?y }}. :pick :one :b.\n\
             {{ ?w :all :yes }} <= {{ :pick :one ?w . :cfg :cl ?cl . \
             ( ?cl {{ :a :ok true }} ) log:forAllIn {{ :a :p :b }} }}.\n\
             {{ {v} :all ?r }} => {{ :out :fired ?r }}."
        )
    };
    assert_eq!(out(&forall("?y")), none(), "forAllIn, caller spells ?y");
    assert_eq!(out(&forall("?y")), out(&forall("?q")), "forAllIn: the spellings agree");
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

/// THE case GH #6943 is about, on `log:forAllIn`. `:cfg :cl { :a :p ?y }` is read through
/// `?cl` and used as the aggregation's first clause, and the rule writes
/// `{ :a :ok true }` beside it as the second.
///
/// A variable written inside a stored, quoted formula is local to that formula in N3, so the
/// arriving clause is matched with its own variables renamed apart: it has one solution over
/// this scope, that solution has to extend to `{ :a :ok true }`, the scope holds no such
/// statement, and the premise FAILS. Matched with its variable still in it the clause
/// constrains nothing, every solution extends trivially, `log:forAllIn` holds VACUOUSLY and
/// the rule FIRES — which `origin/main` does when the goal's caller happens to spell its own
/// variable `?y` too.
#[test]
fn a_configured_forall_clause_is_matched_not_held_vacuously() {
    for caller in ["?q", "?y"] {
        let got = out(&format!(
            ":cfg :cl {{ :a :p ?y }}. :pick :one :b.\n\
             {{ ?w :all :yes }} <= {{ :pick :one ?w . :cfg :cl ?cl . \
             ( ?cl {{ :a :ok true }} ) log:forAllIn {{ :a :p :b }} }}.\n\
             {{ {caller} :all ?r }} => {{ :out :fired ?r }}."
        ));
        assert_eq!(got, none(), "caller spelling {caller}: the premise must fail");
    }
}

/// A clause the RULE built out of its own text keeps sharing with the rule.
/// `( { ?x :type :T } ) list:member ?c` hands the aggregation a formula made of the rule's
/// own premise text, so its `?x` is the rule's `?x` — the name the engine minted for it —
/// and the rule writes `{ ?x :ok true }` beside it as the second clause. One `?x`: `:b` is
/// `:T` with no `:b :ok true`, so the premise fails. `rename_free_apart` leaves an engine
/// name alone for exactly this reason; renaming it would cut the clause off from the rule
/// that wrote it and the premise would hold instead.
#[test]
fn a_rule_built_clause_still_shares_with_the_rule_that_wrote_it() {
    let doc = |scope: &str| {
        format!(
            "{{ :mark :m :yes }} <= {{ ( {{ ?x :type :T }} ) list:member ?c . \
             ( ?c {{ ?x :ok true }} ) log:forAllIn {{ {scope} }} }}.\n\
             {{ :mark :m :yes }} => {{ :out :fired :yes }}."
        )
    };
    let got = out(&doc(":a :type :T. :b :type :T. :a :ok true"));
    assert_eq!(got, none(), ":b is :T and not :ok, so the premise must fail");
    // With every `:T` also `:ok`, the same premise HOLDS — so the `none()` above is the
    // clause actually constraining the match, not the premise failing for another reason.
    let all = out(&doc(":a :type :T. :b :type :T. :a :ok true. :b :ok true"));
    assert_eq!(all, fired(), "every :T is :ok, so the premise holds");
}

/// An INLINE clause is never spliced with the formula a variable beside it is bound to.
/// `( { :d :has ?f } { :k0 :ok ?f } ) log:forAllIn` asks whether the formula `:d :has` names
/// is the one `:k0 :ok` names. A quoted formula is a TERM, compared by identity, and these
/// two are different formulas, so the premise must fail. Substituting the first clause's
/// value into the second would put a stored formula into an inline clause position, where the
/// matcher reads a formula as a structural PATTERN: `{ ?j :p :b }` would then MATCH
/// `{ :w :p :b }` and the rule would fire.
#[test]
fn an_inline_clause_is_not_spliced_with_a_formula_bound_beside_it() {
    let got = out(":d :has { ?j :p :b }. :k0 :ok { :w :p :b }.\n\
                   { :mark :m :yes } <= { ( { :d :has ?f } { :k0 :ok ?f } ) log:forAllIn ?scope }.\n\
                   { :mark :m :yes } => { :out :fired :yes }.");
    assert_eq!(got, none(), "two different formulas: the premise must fail");
    // The same document whose two facts name the SAME formula: the premise holds, so the
    // `none()` above is identity failing, not the clause never being matched at all.
    let same = out(":d :has { ?j :p :b }. :k0 :ok { ?j :p :b }.\n\
                    { :mark :m :yes } <= { ( { :d :has ?f } { :k0 :ok ?f } ) log:forAllIn ?scope }.\n\
                    { :mark :m :yes } => { :out :fired :yes }.");
    assert_eq!(same, fired(), "one formula named twice: the premise holds");
}

/// A binding whose values share subterms is a DAG, and resolving a variable INTO its value
/// builds that DAG's unfolding, which is exponential in its depth: thirteen links of
/// `( { ?v<n> ?v<n> ?v<n> } ) list:member ?v<n-1>` is tens of megabytes from a few hundred
/// bytes of N3. Untrusted input reaches this, so the size is computed from the borrowed terms
/// first and the run is REFUSED past its term-size limit rather than building the term.
///
/// The assertion is on the refusal, not on a time or a memory figure, so it cannot be flaky.
#[test]
fn a_dag_shaped_binding_is_refused_rather_than_unfolded() {
    let links = (2..=14)
        .map(|n| format!("( {{ ?v{n} ?v{n} ?v{n} }} ) list:member ?v{} . ", n - 1))
        .collect::<String>();
    let e = err(&format!(
        ":s :p :z.\n\
         {{ :q :pat {{ ?v1 :r :z }} }} <= {{ {links}( :z ) list:member ?v14 }}.\n\
         {{ :q :pat ?f }} => {{ :out :got ?f }}."
    ));
    assert!(e.contains("term"), "a term-size refusal, not {e}");
    // Four links unfold small enough to answer, so the refusal above is the SIZE limit and
    // not this shape being rejected outright.
    let short = (2..=5)
        .map(|n| format!("( {{ ?v{n} ?v{n} ?v{n} }} ) list:member ?v{} . ", n - 1))
        .collect::<String>();
    let got = out(&format!(
        ":s :p :z.\n\
         {{ :q :pat {{ ?v1 :r :z }} }} <= {{ {short}( :z ) list:member ?v5 }}.\n\
         {{ :q :pat ?f }} => {{ :out :got ?f }}."
    ));
    assert_eq!(got.len(), 1, "four links are answered: {got:?}");
    assert!(!got[0].contains("Var("), "fully resolved: {}", got[0]);
}

/// An `@forAll` formula arriving through a variable is still refused as an antecedent
/// ([`QUANTIFIED_ANTECEDENT`] on the engine side). Arrival is classified BEFORE anything is
/// substituted, so preparing the pattern cannot turn a quantified value into an inline
/// formula that the check no longer sees.
#[test]
fn an_arrived_quantified_pattern_is_still_refused() {
    let e = err(":cfg :pat { @forAll :u. :u :p :b }. :sc :has { @forAll :v. :v :p :b }.\n\
                 { :mark :m :yes } <= { :cfg :pat ?pat . \
                 ( { :sc :has ?f } { ?f log:notIncludes ?pat } ) log:forAllIn ?scope }.\n\
                 { :mark :m :yes } => { :out :fired :yes }.");
    assert!(e.contains("universally quantified antecedents are unsupported"), "{e}");
}

/// The SAME premise with the formula coming from the DOCUMENT rather than from the rule's
/// own text. Its `?w` is a stored formula's own variable, not the proof's, so it is renamed
/// apart and matched — and the aggregation answers, with `origin/main`'s answer. This is the
/// pair that makes the rule a narrowing and not a name test: the two documents differ only
/// in where the formula came from.
#[test]
fn a_document_built_collect_clause_with_the_same_shape_is_matched() {
    let got = out(":cfg :one { :a :p ?w }.\n\
                   { :mark :m ?l } <= { :cfg :one ?c . ( ?c ) list:member ?p . \
                   ( :hit ?p ?l ) log:collectAllIn { :a :p :b. :a :p :c } }.\n\
                   { :mark :m ?l } => { :out :list ?l }.");
    assert_eq!(got, list(&["hit", "hit"]));
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

/// Case (2), and the reviewer's own repro: the caller's `?y` is in the binding only because
/// head unification aliased it to the callee's renamed `?w`. The verdict must not turn on
/// that, so the arriving clause's `?y` is renamed apart whichever way the alias runs, and the
/// two spellings get the same answer. `origin/main` fires for one spelling and not the other.
#[test]
fn an_unbound_alias_does_not_change_the_answer() {
    let callee = ":cfg :cl { :a :p ?y }.\n\
                  { ?w :ok :yes } <= { :cfg :cl ?cl . ( ?cl { :a :ok true } ) \
                  log:forAllIn { :a :p :b } }.\n";
    let y = out(&format!("{callee}{{ ?y :ok :yes }} => {{ :out :fired :yes }}."));
    let q = out(&format!("{callee}{{ ?q :ok :yes }} => {{ :out :fired :yes }}."));
    assert_eq!(y, none(), "the premise must fail: the scope holds no `:a :ok true`");
    assert_eq!(y, q, "the two spellings must agree");
}

/// Case (5): a CONTAINMENT pattern that arrived through a variable. Inside a backward proof
/// it used to fall back to the EMPTY pattern, which makes `log:includes` hold of everything
/// and `log:notIncludes` of nothing — so on `origin/main` the `includes` rule FIRES and the
/// `notIncludes` rule does not over a configured pattern (`{ :a :q ?z }`) the scope
/// (`{ :a :p :b }`) was never tested against. The pattern is now renamed apart and MATCHED,
/// so it does not occur, `includes` fails and `notIncludes` holds.
///
/// The same premise in a FORWARD rule keeps main's behaviour, which
/// `tests/n3_bound_containment.rs` pins; nothing here touches it.
#[test]
fn a_configured_containment_pattern_is_matched_not_read_as_empty() {
    for (op, want) in [("includes", false), ("notIncludes", true)] {
        let got = out(&format!(
            ":cfg :pat {{ :a :q ?z }}.\n\
             {{ :mark :m :yes }} <= {{ :cfg :pat ?p . {{ :a :p :b }} log:{op} ?p }}.\n\
             {{ :mark :m :yes }} => {{ :out :fired :yes }}."
        ));
        assert_eq!(got, if want { fired() } else { none() }, "{op}");
    }
    // The same pattern against a scope it DOES occur in: matched, with its renamed variable
    // bound by the match rather than left to make the pattern empty.
    for (op, want) in [("includes", true), ("notIncludes", false)] {
        let got = out(&format!(
            ":cfg :pat {{ :a :p ?z }}.\n\
             {{ :mark :m :yes }} <= {{ :cfg :pat ?p . {{ :a :p :b }} log:{op} ?p }}.\n\
             {{ :mark :m :yes }} => {{ :out :fired :yes }}."
        ));
        assert_eq!(got, if want { fired() } else { none() }, "{op}, occurring");
    }
}

/// Case (6): a formula's OWN binder is exempt from the rename-apart. `__uf.` names are bound
/// inside their formula, no substitution reaches them, and matching treats them as constants,
/// so renaming one would change what the formula means. `rename_free_apart` leaves them, and
/// the answer is `origin/main`'s.
#[test]
fn a_formula_local_binder_is_not_a_free_variable() {
    let got = out(":cfg :t { @forAll :u. :u :p :b }.\n\
                   { :mark :m ?l } <= { :cfg :t ?tm . ( ?tm { :a :p :b } ?l ) \
                   log:collectAllIn { :a :p :b } }.\n\
                   { :mark :m ?l } => { :out :list ?l }.");
    assert_eq!(got, none(), "accepted, and answered as on main");
}

/// Resolution follows a chain to its END before the clause is matched. Here the arriving
/// clause's `?y` reaches a value only through an alias: head unification binds it to the
/// callee's `?w`, and the premise then binds that to `:b`. So the clause is `{ :a :p :b }`,
/// the scope holds it, and one item is collected.
///
/// Two ways to get this wrong, and both are mutation-tested. Resolving one hop leaves the
/// alias in the clause. And matching the clause AS STORED, with `?y` still in it, constrains
/// nothing — `origin/main` collects nothing here, and an earlier revision of this PR pinned
/// that empty answer.
#[test]
fn a_clause_ground_only_through_an_alias_is_matched_instantiated() {
    let got = out(":cfg :cl { :a :p ?y }. :pick :one :b.\n\
                   { ?w :coll ?l } <= { :pick :one ?w . :cfg :cl ?cl . \
                   ( :hit ?cl ?l ) log:collectAllIn { :a :p :b } }.\n\
                   { ?y :coll ?l } => { :out :list ?l }.");
    assert_eq!(got, list(&["hit"]), "the clause is matched as `{{ :a :p :b }}`");
}

/// A pattern whose value arrives only through an alias used to be matched AS STORED, where
/// its variable made `stored_ground_statements` read it as non-ground and fall back to the
/// EMPTY pattern — so `log:includes` held of a scope that does not contain the pattern.
/// `origin/main` and the first revisions of this PR both FIRE here.
///
/// The second half is the positive control: the identical rule over a scope that DOES hold
/// the instantiated pattern fires. Without it, "does not fire" would also pass if the pattern
/// had become unmatchable instead of correctly matched.
#[test]
fn a_containment_pattern_ground_through_an_alias_is_matched_instantiated() {
    let doc = |scope: &str| {
        format!(
            ":pick :one :b.\n\
             {{ ?w :inc :yes }} <= {{ :pick :one ?w . ( {{ :a :p ?w }} ) list:member ?cl . \
             {{ {scope} }} log:includes ?cl }}.\n\
             {{ ?y :inc ?r }} => {{ :out :fired ?r }}."
        )
    };
    assert_eq!(out(&doc(":a :p :zz")), none(), "the scope does not hold `:a :p :b`");
    assert_eq!(out(&doc(":a :p :b")), fired(), "the scope does hold it");
}

/// The `log:forAllIn` twin of the same root cause: the arriving clause reaches its value only
/// through an alias, so matched as stored it constrains nothing, every solution extends
/// trivially and the premise holds VACUOUSLY. It FIRES on `origin/main` and on the first
/// revisions of this PR. Instantiated, the clause is `{ :a :p :b }`, the scope holds it, and the consequent
/// `{ :a :ok true }` does not — so the rule must not fire.
///
/// The second half is the positive control: add `:a :ok true` to the scope and it fires.
#[test]
fn a_forall_clause_ground_through_an_alias_is_matched_instantiated() {
    let doc = |scope: &str| {
        format!(
            ":cfg :cl {{ :a :p ?y }}. :pick :one :b.\n\
             {{ ?w :all :yes }} <= {{ :pick :one ?w . :cfg :cl ?cl . \
             ( ?cl {{ :a :ok true }} ) log:forAllIn {{ {scope} }} }}.\n\
             {{ ?y :all ?r }} => {{ :out :fired ?r }}."
        )
    };
    assert_eq!(out(&doc(":a :p :b")), none(), "`:a :ok true` is not in the scope");
    assert_eq!(out(&doc(":a :p :b. :a :ok true")), fired(), "now it is");
}

/// A value a proof bound can itself hold a variable the same substitution binds, so the
/// projection resolves INTO what each chain lands on rather than cloning it. `origin/main`
/// returns `{ { ?__bw<n>_y :q :r } :p :b }` here, with `?y` bound to `:v` all along.
#[test]
fn the_projection_resolves_inside_a_value_it_landed_on() {
    let got = out(":s :val :v.\n\
                   { ?s :pat { ?l :p :b } } <= { ( { ?y :q :r } ) list:member ?l . ?s :val ?y }.\n\
                   { :s :pat ?f } => { :out :seen ?f }.");
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(got[0].contains("Iri(\"http://ex/v\")"), "resolved inside: {}", got[0]);
    assert!(!got[0].contains("Var("), "no variable survives: {}", got[0]);
}

/// A GROUND clause that arrived through a variable has no free variable to rename, so
/// preparing it is the identity and it is matched exactly as it was stored.
#[test]
fn a_ground_bound_clause_is_still_matched_as_stored() {
    let got = out(":cfg :cl { :a :p :b }.\n\
                   { :mark :m ?l } <= { :cfg :cl ?c . ( :hit ?c ?l ) \
                   log:collectAllIn { :a :p :b } }.\n\
                   { :mark :m ?l } => { :out :list ?l }.");
    assert_eq!(got, list(&["hit"]));
}

// --------------------------------------- a SCOPE is searched data, not a pattern

/// A stored SCOPE holding a variable is searched as it stands: the scope is the data a clause
/// is matched against, not a pattern that has to match, and a free variable in it is simply a
/// statement nothing matches. All three of these are `origin/main`'s answers, and the first
/// two agree with the forward twin — requiring the scope to be ground refused them instead.
#[test]
fn a_stored_scope_holding_a_variable_is_searched_as_it_stands() {
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

/// Nothing reads a variable's NAME to decide a document's answer, so a document that spells
/// a variable in the shape the chainer's own renaming uses (`?__bw<digits>_…`) is answered
/// exactly as the same document with an ordinary name. A document is entitled to that
/// spelling.
#[test]
fn a_document_that_spells_a_generated_name_is_answered_the_same() {
    let doc = |y: &str| {
        format!(
            ":cfg :cl {{ :a :p {y} }}.\n\
             {{ :mark :m :yes }} <= {{ :cfg :cl ?cl. \
             ( ?cl {{ :a :ok true }} ) log:forAllIn {{ :a :p :b }} }}.\n\
             {{ :mark :m :yes }} => {{ :out :fired :yes }}."
        )
    };
    assert_eq!(out(&doc("?y")), none(), "the scope holds no `:a :ok true`");
    for minted in ["?__bw1_y", "?__bw99999999999999999999999_y"] {
        assert_eq!(out(&doc("?y")), out(&doc(minted)), "spelling {minted}");
    }
}

/// A stored formula reaching the callee through the GOAL, used as the aggregation's clause
/// while the TEMPLATE is the callee's own `?y`. The clause's `?y` is the stored formula's,
/// renamed apart, so it cannot bind the callee's template and nothing ground is collected —
/// `origin/main`'s answer, and the same answer `the_answer_does_not_depend_on_the_callers_spelling`
/// pins for a template the clause genuinely cannot reach.
#[test]
fn a_stored_clause_cannot_bind_the_callees_own_template() {
    let got = out(":cfg :cl { :a :p ?y }.\n\
                   { ?cl :coll ?l } <= { ( ?y ?cl ?l ) log:collectAllIn { :a :p :b. :a :p :c } }.\n\
                   { :cfg :cl ?cl. ?cl :coll ?l } => { :out :list ?l }.");
    assert_eq!(got, none());
}

/// The SAME premise, forward and backward. A forward rule's atoms are never standardized
/// apart, so the document's `?y` is one variable there, the arriving clause's `?y` is ground
/// by the time it matters, and the answer is `( :b :c )` — unchanged by this PR, and pinned
/// because the forward path is what `tests/n3_bound_containment.rs` covers. Behind a `<=` the
/// atoms are renamed, the arriving clause's `?y` is its own, and it cannot bind the rule's
/// template, so nothing is collected. Both halves are `origin/main`'s answers.
#[test]
fn the_forward_and_backward_answers_both_stand() {
    let data = ":cfg :cl { :a :p ?y }.\n";
    let prem = ":cfg :cl ?cl. ( ?y ?cl ?l ) log:collectAllIn { :a :p :b. :a :p :c }";
    let bwd = out(&format!(
        "{data}{{ :mark :m ?l }} <= {{ {prem} }}.\n{{ :mark :m ?l }} => {{ :out :list ?l }}."
    ));
    assert_eq!(bwd, none(), "backward: the clause cannot bind the rule's own template");
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

// ------------------------------------------------- review round 4: the idioms that must answer

/// A `log:collectAllIn` whose CLAUSE arrives configured from the document, read with the two
/// caller spellings that `origin/main` answers differently. The arriving clause's `?y` is the
/// stored formula's own, renamed apart, so it matches `:a :p :b` and the template `:hit` is
/// collected once — whichever name the forward rule happens to write.
///
/// `origin/main` collects `( )` when the caller writes `?y` and `( :hit )` when it writes
/// `?q`: the stored clause's `?y` and the caller's `?y` are one name in one flat binding
/// domain there, so the caller's choice of spelling decided the callee's answer. Both
/// spellings now give the forward twin's answer.
#[test]
fn the_collected_answer_does_not_depend_on_the_callers_spelling() {
    let doc = |caller: &str| {
        format!(
            ":cfg :cl {{ :a :p ?y }}. :pick :one :b.\n\
             {{ ?w :coll ?l }} <= {{ :pick :one ?w . :cfg :cl ?cl . \
             ( :hit ?cl ?l ) log:collectAllIn {{ :a :p :b }} }}.\n\
             {{ {caller} :coll ?l }} => {{ :out :list ?l }}."
        )
    };
    assert_eq!(out(&doc("?y")), list(&["hit"]), "caller spelling ?y");
    assert_eq!(out(&doc("?q")), list(&["hit"]), "caller spelling ?q");
    assert_eq!(out(&doc("?y")), out(&doc("?q")), "the two spellings agree");
}

/// A TEMPLATE written in place, beside a clause that arrived. Only what arrived through a
/// variable is prepared; the rule's own text is left exactly as it is, so the template still
/// shares with the clause the rule wrote around it and the aggregation answers.
///
/// Both halves are `origin/main`'s and the forward twin's answers. An earlier revision of
/// this PR refused them, because it classified "arrived" by the shape of the RESOLVED slot
/// rather than by whether the written slot was a variable, and a list or a triple term
/// written in place fell through to the refusable arm.
#[test]
fn a_template_written_in_place_is_matched_not_refused() {
    // A LIST template.
    let lst = out(":a :p :b. :a :p :c.\n\
                   { :mark :m ?l } <= { ( ( :a ?o ) { :a :p ?o } ?l ) log:collectAllIn ?scope }.\n\
                   { :mark :m ?l } => { :out :list ?l }.");
    assert_eq!(lst.len(), 1, "{lst:?}");
    assert!(lst[0].contains("List([List([Iri(\"http://ex/a\"), Iri(\"http://ex/b\")])"), "{}", lst[0]);
    // A TRIPLE TERM template.
    let tt = out(":a :p :b. :a :p :c.\n\
                  { :mark :m ?l } <= { ( << :a :p ?o >> { :a :p ?o } ?l ) log:collectAllIn ?scope }.\n\
                  { :mark :m ?l } => { :out :list ?l }.");
    assert_eq!(tt.len(), 1, "{tt:?}");
    assert!(tt[0].contains("Triple("), "a triple term is collected, not refused: {}", tt[0]);
}

/// A clause LIST configured in the document and handed over whole through one variable. Its
/// two clauses are written with one `?x`, so they must stay one variable after the
/// rename-apart: `log:forAllIn` holds iff every `?x` with `:p :b` also has `:q :c`.
///
/// One prefix per atom evaluation is what makes this work. A fresh prefix per arriving VALUE
/// would rename the two `?x` apart from each other, the second clause would stop constraining
/// the first, and the negative half would hold vacuously and fire.
#[test]
fn a_configured_clause_list_keeps_its_own_variable_shared() {
    let doc = |scope: &str| {
        format!(
            ":cfg :cs ( {{ ?x :p :b }} {{ ?x :q :c }} ).\n\
             {{ :m :m :yes }} <= {{ :cfg :cs ?cs . ?cs log:forAllIn {{ {scope} }} }}.\n\
             {{ :m :m :yes }} => {{ :out :fired :yes }}."
        )
    };
    assert_eq!(out(&doc(":a :p :b. :d :p :b. :a :q :c")), none(), ":d has :p :b and no :q :c");
    assert_eq!(out(&doc(":a :p :b. :a :q :c")), fired(), "every :p :b also has :q :c");
}

/// The same sharing between a configured TEMPLATE and a configured CLAUSE: the document
/// writes `( ?t { ?t :type :T } )`, the rule reads the pair out of it, and the template is
/// the clause's own variable. Collecting `( :a :b )` is only possible if the two arriving
/// values were renamed apart together.
#[test]
fn a_configured_template_and_clause_keep_their_shared_variable() {
    let got = out(":cfg :args ( ?t { ?t :type :T } ).\n\
                   { :m :m ?l } <= { :cfg :args (?tp ?c) . \
                   ( ?tp ?c ?l ) log:collectAllIn { :a :type :T. :b :type :T } }.\n\
                   { :m :m ?l } => { :out :list ?l }.");
    assert_eq!(got, list(&["a", "b"]), "the template is the clause's own variable");
}

/// Two arriving patterns in ONE premise share a prefix, so they share a renamed name — and
/// that must not let the first atom's match constrain the second. A prepared pattern's
/// `__bp.` bindings are dropped with the containment's own existential witnesses, so each
/// atom matches on its own.
///
/// The first half is the control: both atoms match, each binding its own `?x`, and the rule
/// fires (it also fires on `origin/main` and on the forward twin). The second half is the one
/// that bites: the second atom's pattern occurs nowhere, so the rule must NOT fire —
/// `origin/main` fires, because the pattern had become the EMPTY pattern.
#[test]
fn two_arriving_patterns_in_one_premise_do_not_share_bindings() {
    let doc = |scope2: &str| {
        format!(
            ":cfg :p1 {{ :a :p ?x }}. :cfg :p2 {{ :d :q ?x }}.\n\
             {{ :m :m :yes }} <= {{ :cfg :p1 ?P1 . :cfg :p2 ?P2 . \
             {{ :a :p :b }} log:includes ?P1 . {{ {scope2} }} log:includes ?P2 }}.\n\
             {{ :m :m :yes }} => {{ :out :fired :yes }}."
        )
    };
    assert_eq!(out(&doc(":d :q :c")), fired(), "both patterns match, on their own `?x`");
    assert_eq!(out(&doc(":zz :zz :zz")), none(), "the second pattern occurs nowhere");
}

/// Resolution reaches a variable nested inside a formula, a list or a triple term in the
/// arriving pattern, not only one at its top level. In each shape `?y` is bound to `:zz`, the
/// instantiated pattern is therefore not the one the scope holds, and `log:includes` must not
/// hold. `origin/main` FIRES on all three: the pattern's variable made it fall back to the
/// EMPTY pattern, which is included in everything.
#[test]
fn resolution_reaches_a_variable_nested_in_an_arriving_pattern() {
    for (nested, scope) in [
        ("{ :s :q ?y }", "{ :s :q :b }"),
        ("( ?y )", "( :b )"),
        ("<< :s :q ?y >>", "<< :s :q :b >>"),
    ] {
        let got = out(&format!(
            ":kk :vv :zz.\n\
             {{ :m :m :yes }} <= {{ :kk :vv ?y . ( {{ :a :p {nested} }} ) list:member ?p . \
             {{ :a :p {scope} }} log:includes ?p }}.\n\
             {{ :m :m :yes }} => {{ :out :fired :yes }}."
        ));
        assert_eq!(got, none(), "nested {nested}: `?y` is `:zz`, not `:b`");
    }
}

/// The projection resolves an alias chain wherever it sits, not only inside a quoted formula.
/// `( ?x )` in a conclusion is a LIST holding the alias, and nothing else resolves it: the
/// projection then declined to bind the goal's `?cl` at all and the derived fact was dropped,
/// so `origin/main` and every earlier revision of this PR derive NOTHING here — while the
/// same position written as `{ :a :q ?x }` derives `{ :a :q :c }`.
#[test]
fn the_projection_resolves_an_alias_chain_inside_a_list() {
    let got = out(":seed :value :c.\n\
                   { ( ?x ?y ) :cl ( ?x ) } <= { :seed :value ?y }.\n\
                   { ( ?t ?t ) :cl ?cl } => { :out :got ?cl }.");
    assert_eq!(got, one("Iri(\"http://ex/got\") List([Iri(\"http://ex/c\")])"));
    // The formula twin of the same conclusion, which already resolved: the two agree now.
    let f = out(":seed :value :c.\n\
                 { ( ?x ?y ) :cl { :a :q ?x } } <= { :seed :value ?y }.\n\
                 { ( ?t ?t ) :cl ?cl } => { :out :got ?cl }.");
    assert!(f[0].contains("Iri(\"http://ex/c\")") && !f[0].contains("Var("), "{}", f[0]);
}

/// A variable written in DATA is never looked up in the proof's binding, whatever it is
/// spelled. `:cfg :pat { :a :p ?y }` arrives as a `log:includes` pattern, and the goal's
/// caller is aliased to the rule's `?w` — which is bound to `:b` — under the two spellings
/// that make `?y` reachable in that binding and not reachable. The pattern's `?y` is the
/// stored formula's own: it is renamed apart and bound by the MATCH, to `:zz`, so the
/// containment holds and the rule fires either way.
///
/// This is the shape the round-3 revision got wrong. Resolving a document's name out of the
/// live binding instantiates the pattern to `{ :a :p :b }`, which the scope does not hold, so
/// the `?y` spelling stops firing while the `?q` spelling still does — a soundness verdict
/// decided by a caller's arbitrary choice of name.
#[test]
fn a_variable_written_in_data_is_not_resolved_from_the_proofs_binding() {
    let doc = |caller: &str| {
        format!(
            ":cfg :pat {{ :a :p ?y }}. :pick :one :b.\n\
             {{ ?w :inc :yes }} <= {{ :pick :one ?w . :cfg :pat ?p . \
             {{ :a :p :zz }} log:includes ?p }}.\n\
             {{ {caller} :inc ?r }} => {{ :out :fired ?r }}."
        )
    };
    assert_eq!(out(&doc("?y")), fired(), "`?y`: the pattern is matched, binding its own `?y`");
    assert_eq!(out(&doc("?q")), fired(), "`?q`: the same");
    assert_eq!(out(&doc("?y")), out(&doc("?q")), "the two spellings agree");
    // The `log:forAllIn` twin of the same lookup, where the wrong resolution shows up as a
    // firing rather than as a non-firing.
    let fa = |caller: &str| {
        format!(
            ":cfg :cl {{ :a :p ?y }}. :pick :one :b.\n\
             {{ ?w :all :yes }} <= {{ :pick :one ?w . :cfg :cl ?cl . \
             ( ?cl {{ :a :ok true }} ) log:forAllIn {{ :a :p :b. :a :p :zz. :a :ok true }} }}.\n\
             {{ {caller} :all ?r }} => {{ :out :fired ?r }}."
        )
    };
    // `:a :p :zz` is a solution of the clause and `:a :ok true` holds, so the premise holds
    // for both spellings. Resolved from the binding the clause would be `{ :a :p :b }` only.
    assert_eq!(out(&fa("?y")), fired(), "forAllIn, `?y`");
    assert_eq!(out(&fa("?q")), fired(), "forAllIn, `?q`");
}
