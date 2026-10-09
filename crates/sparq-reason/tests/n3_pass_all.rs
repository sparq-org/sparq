//! [OPUS-5] sq-xqchl.2 (GH #3143) — the EYE `--pass-all` / `--pass-all-ground` output
//! document: the deductive closure PLUS the document's own rules, echoed back as N3.
//!
//! The load-bearing invariant is that the rules SURVIVE the round trip: under
//! [`RuleVars::N3`] the emitted document re-parses to the same rule set, so re-running the
//! reasoner over it is a fixpoint. That is what distinguishes `--pass-all` from `--pass`
//! (whose output can derive nothing further) and it is what the eye-js `…_plus_rules`
//! output modes buy.

use sparq_reason::n3::{parser, Term};
use sparq_reason::{reason_n3_pass_all, reason_n3_query_terms, reason_n3_terms, RuleVars};

type Stmts = Vec<[Term; 3]>;

/// Each parsed rule as `(premise, conclusion)` — what "the same rule" means term-for-term.
fn rule_terms(doc: &str) -> Vec<(Stmts, Stmts)> {
    let p = parser::parse(doc).expect("re-parses");
    p.rules.into_iter().map(|r| (r.premise, r.conclusion)).collect()
}

const S: &str = "http://example.org/socrates#";
const TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// The Socrates rule document: one asserted fact, one forward rule.
const SOCRATES: &str = r#"@prefix : <http://example.org/socrates#>.
:Socrates a :Human.
{ ?x a :Human } => { ?x a :Mortal }.
"#;

#[test]
fn pass_all_emits_the_closure_and_the_rule() {
    let doc = reason_n3_pass_all(SOCRATES, RuleVars::N3).expect("pass-all");
    // The `--pass` half: base fact AND entailed fact.
    assert!(doc.contains(&format!("<{S}Socrates> <{TYPE}> <{S}Human> .")), "{doc}");
    assert!(doc.contains(&format!("<{S}Socrates> <{TYPE}> <{S}Mortal> .")), "{doc}");
    // The `-all` half: the rule itself, which plain `--pass` output loses.
    let rule = format!("{{ ?x <{TYPE}> <{S}Human> . }} => {{ ?x <{TYPE}> <{S}Mortal> . }} .");
    assert!(doc.contains(&rule), "{doc}");
}

#[test]
fn pass_all_output_is_a_fixpoint_and_keeps_deriving() {
    let doc = reason_n3_pass_all(SOCRATES, RuleVars::N3).expect("pass-all");
    // Re-parsing the document yields the SAME closure and the SAME rule, so a second pass
    // is byte-identical. This is exactly what fails if rules are dropped (round two would
    // lose the rule line) or if a closure fact is lost.
    let again = reason_n3_pass_all(&doc, RuleVars::N3).expect("pass-all round two");
    assert_eq!(doc, again);

    // And the echoed rule is a LIVE rule, not decoration: adding a bare fact to the
    // document derives through it.
    let extended = format!("{doc}<{S}Plato> <{TYPE}> <{S}Human> .\n");
    let closure = reason_n3_terms(&extended, None).expect("closure of the extended document");
    let mortal_plato = [
        Term::Iri(format!("{S}Plato")),
        Term::Iri(TYPE.to_string()),
        Term::Iri(format!("{S}Mortal")),
    ];
    assert!(
        closure.facts.contains(&mortal_plato),
        "the echoed rule must still fire: {:?}",
        closure.facts
    );
}

#[test]
fn pass_only_the_closure_differs_from_pass_all() {
    // Guard against the two modes silently collapsing into one: `--pass` output has no rule.
    let mut dict = sparq_core::dict::Dict::default();
    let closure = sparq_reason::reason_n3(&mut dict, SOCRATES).expect("closure");
    assert_eq!(closure.len(), 2, "base + entailed typing");
    let doc = reason_n3_pass_all(SOCRATES, RuleVars::N3).expect("pass-all");
    assert_eq!(doc.matches("=>").count(), 1, "exactly one echoed rule: {doc}");
}

#[test]
fn grounded_mode_leaves_no_syntactic_variable() {
    let doc = reason_n3_pass_all(SOCRATES, RuleVars::VarIris).expect("pass-all-ground");
    assert!(!doc.contains('?'), "the grounded form carries no `?x`: {doc}");
    assert!(doc.contains("<http://www.w3.org/2000/10/swap/var#x>"), "{doc}");
    // The closure half is identical to the un-grounded form — only the rules change.
    let plain = reason_n3_pass_all(SOCRATES, RuleVars::N3).expect("pass-all");
    let facts = |d: &str| d.lines().filter(|l| !l.contains("=>")).collect::<Vec<_>>().join("\n");
    assert_eq!(facts(&doc), facts(&plain));
}

#[test]
fn backward_rules_are_echoed_with_their_arrow() {
    let src = r#"@prefix : <http://ex/>.
{ ?x a :Mortal } <= { ?x a :Human }.
:Socrates a :Human.
{ ?x a :Mortal } => { ?x a :Doomed }.
"#;
    let doc = reason_n3_pass_all(src, RuleVars::N3).expect("pass-all");
    assert_eq!(doc.matches("<=").count(), 1, "the backward rule survives: {doc}");
    assert_eq!(doc.matches("=>").count(), 1, "the forward rule survives: {doc}");
    // Backward rules are goal-directed, so the closure still contains the derived fact.
    assert!(doc.contains("<http://ex/Doomed>"), "{doc}");
    // ... and the echoed document re-derives it identically.
    assert_eq!(doc, reason_n3_pass_all(&doc, RuleVars::N3).expect("round two"));
}

#[test]
fn a_premise_blank_node_is_echoed_as_a_blank_node() {
    // The parser rewrites a rule-scoped premise blank to `?__bn.<i>.<label>`; that
    // engine-internal name must never reach the output.
    let src = r#"@prefix : <http://ex/>.
{ _:s a :Human } => { :Someone a :Mortal }.
:a a :Human.
"#;
    let doc = reason_n3_pass_all(src, RuleVars::N3).expect("pass-all");
    assert!(!doc.contains("__bn"), "no engine-internal variable name: {doc}");
    assert!(doc.contains("_:s"), "{doc}");
    assert_eq!(doc, reason_n3_pass_all(&doc, RuleVars::N3).expect("round two"));
}

/// GH #5372 review round 1: `--pass-all-ground` must ground a rule variable that sits
/// inside a quoted `{ … }` formula too. N3 quantifies `?x` in the OUTERMOST formula, so the
/// nested `?x` is the same variable — and formula arguments are the ordinary shape for the
/// `log:` builtins, not a corner case. Left ungrounded, the "no `?x`" contract is false.
#[test]
fn grounded_mode_grounds_variables_inside_a_quoted_formula() {
    let src = r#"@prefix : <http://ex/>.
{ ?s :says { ?x a :Good } } => { ?s a :Trusted }.
"#;
    let doc = reason_n3_pass_all(src, RuleVars::VarIris).expect("pass-all-ground");
    assert!(!doc.contains('?'), "no syntactic variable at any depth: {doc}");
    assert!(doc.contains("<http://www.w3.org/2000/10/swap/var#x>"), "the nested one: {doc}");
    assert!(doc.contains("<http://www.w3.org/2000/10/swap/var#s>"), "{doc}");
    // The un-grounded form still writes that nested variable as `?x`.
    let plain = reason_n3_pass_all(src, RuleVars::N3).expect("pass-all");
    assert!(plain.contains("{ ?x "), "{plain}");

    // The documented boundary: VarIris grounds RULES, not data. A formula-valued FACT is
    // echoed verbatim in the closure half, `?x` and all — so "no `?` in the document" is
    // a claim about rule documents, not about every input.
    let with_a_formula_fact = ":a <http://ex/p> { ?x <http://ex/q> :b }.\n";
    let doc = reason_n3_pass_all(with_a_formula_fact, RuleVars::VarIris).expect("pass-all");
    assert!(doc.contains("{ ?x "), "a formula-valued fact keeps its variable: {doc}");
}

/// GH #5372 review round 1: `?__bn0_x` is a LEGAL N3 variable (a variable name is letters,
/// digits, `_`, `-` and non-ASCII `PN_CHARS`), so the serializer must not read it as the
/// parser's own premise-blank rewrite. Decoding by spelling turned it into `_:x` — and for a
/// variable occurring only in the conclusion, that silently converts a bound variable into a
/// fresh-per-firing existential. The rewrite now uses a name a document CANNOT write, which
/// is the property the last assertion here pins.
#[test]
fn a_source_variable_spelled_like_the_rewrite_stays_a_variable() {
    let src = r#"@prefix : <http://ex/>.
{ ?__bn0_x a :Human } => { ?__bn0_x a :Mortal }.
:a a :Human.
"#;
    let doc = reason_n3_pass_all(src, RuleVars::N3).expect("pass-all");
    assert!(doc.contains("?__bn0_x"), "the author's variable survives verbatim: {doc}");
    assert!(!doc.contains("_:"), "and is never demoted to a blank node: {doc}");
    // Still a live rule over the same closure, and re-emitting is a fixpoint.
    assert!(doc.contains(&format!("<http://ex/a> <{TYPE}> <http://ex/Mortal> .")), "{doc}");
    assert_eq!(doc, reason_n3_pass_all(&doc, RuleVars::N3).expect("round two"));

    // The conclusion-only occurrence is where the old decode actually changed meaning.
    let concl_only = r#"@prefix : <http://ex/>.
{ ?y a :Human } => { ?y :peer ?__bn7_z }.
:a a :Human.
"#;
    let doc = reason_n3_pass_all(concl_only, RuleVars::N3).expect("pass-all");
    assert!(doc.contains("?__bn7_z"), "{doc}");
    assert!(!doc.contains("_:"), "{doc}");

    // Why the reverse mapping is now exact rather than a guess: the rewrite's own name
    // (`__bn.<i>.<label>`) contains a `.`, and a variable name is lexed by `read_name`,
    // which stops at one. So the engine-internal name is UNFORGEABLE — a document that
    // tries to write it does not parse, and no legal `?…` can ever decode to a blank node.
    let forged = "@prefix : <http://ex/>.\n{ ?__bn.0.x a :Human } => { :a a :Mortal }.\n";
    assert!(reason_n3_pass_all(forged, RuleVars::N3).is_err(), "{forged}");
}

/// GH #5391: the parser rewrites an `@forAll :x` universal to an engine-internal rule
/// variable. That name must never reach the output. `--pass-all` writes the universal back
/// as its own IRI under ONE `@forAll` declaration that scopes both sides — a document-level
/// line before the rules that share a universal across premise and conclusion (GH #6701
/// round 10: a declaration per side would be two quantifiers) — so the document re-parses
/// to the SAME rule; `--pass-all-ground` grounds it to its `var:x` IRI.
#[test]
fn a_for_all_universal_is_echoed_under_its_declared_iri() {
    let src = "@prefix : <http://ex/>. @forAll :x.
{ :x a :Human } => { :x a :Mortal }. :a a :Human.
";
    let doc = reason_n3_pass_all(src, RuleVars::N3).expect("pass-all");
    assert!(!doc.contains("__ua"), "no engine-internal variable name: {doc}");
    let rule = format!(
        "\n@forAll <http://ex/x> .\n{{ <http://ex/x> <{TYPE}> <http://ex/Human> . }} => \
         {{ <http://ex/x> <{TYPE}> <http://ex/Mortal> . }} .\n"
    );
    assert_eq!(doc.matches("@forAll").count(), 1, "{doc}");
    assert!(doc.ends_with(&rule), "the shared-universal rule is the trailer: {doc}");
    assert!(doc.contains(&rule), "{doc}");
    assert_eq!(rule_terms(&doc), rule_terms(src), "{doc}");
    assert!(doc.contains(&format!("<http://ex/a> <{TYPE}> <http://ex/Mortal> .")), "{doc}");
    assert_eq!(doc, reason_n3_pass_all(&doc, RuleVars::N3).expect("round two"));

    let ground = reason_n3_pass_all(src, RuleVars::VarIris).expect("pass-all-ground");
    assert!(!ground.contains("__ua"), "{ground}");
    assert!(ground.contains("<http://www.w3.org/2000/10/swap/var#x>"), "{ground}");
}

/// GH #5391: a universal must not be captured by a source variable of the same name, nor
/// merge with another universal sharing its local name across namespaces — either would
/// change which positions must bind equal terms. The echoed rule re-parses to the very
/// same rule terms.
#[test]
fn a_for_all_universal_never_collides_with_another_variable() {
    let src = r#"@prefix : <http://ex/>. @prefix o: <http://other/>.
@forAll :x, o:x.
{ :x :p ?x . o:x :q :x } => { :x :r ?x , o:x }.
:a :p :b . :c :q :a .
"#;
    let doc = reason_n3_pass_all(src, RuleVars::N3).expect("pass-all");
    assert!(!doc.contains("__ua"), "{doc}");
    assert_eq!(rule_terms(&doc), rule_terms(src), "{doc}");
    assert!(doc.contains("<http://ex/a> <http://ex/r> <http://ex/b> ."), "{doc}");
    assert!(doc.contains("<http://ex/a> <http://ex/r> <http://ex/c> ."), "{doc}");
    assert_eq!(doc, reason_n3_pass_all(&doc, RuleVars::N3).expect("round two"));
    // Grounded: three distinct `var:` names.
    let ground = reason_n3_pass_all(src, RuleVars::VarIris).expect("pass-all-ground");
    let rule = ground.lines().find(|l| l.contains("=>")).expect("the rule line");
    let mut names: Vec<&str> = rule.split_whitespace().filter(|w| w.contains("swap/var#")).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), 3, "{rule}");
}

/// GH #5391 review: a universal inside a formula-valued FACT must not merge with a source
/// variable of the same name either — `{ :x :q ?x }` holds two distinct variables — and
/// the fact must re-parse to the same term.
#[test]
fn a_for_all_universal_in_a_formula_fact_never_collides() {
    let src = "@prefix : <http://ex/>. @forAll :x. :a :p { :x :q ?x }.\n";
    let doc = reason_n3_pass_all(src, RuleVars::N3).expect("pass-all");
    assert!(!doc.contains("__ua"), "{doc}");
    assert!(doc.contains("{ @forAll <http://ex/x> . <http://ex/x> <http://ex/q> ?x . }"), "{doc}");
    assert_eq!(parser::parse(&doc).expect("re-parses").facts, parser::parse(src).unwrap().facts);
    assert_eq!(doc, reason_n3_pass_all(&doc, RuleVars::N3).expect("round two"));
}

/// GH #6701 review round 2: facts and echoed rules must write one universal the SAME way —
/// here both declare it by its IRI. Were the fact and the rule to spell it differently,
/// the two formulae the rule compares would stop being equal and re-reasoning would derive
/// `:bad :is true`.
#[test]
fn a_for_all_universal_gets_one_name_across_facts_and_rules() {
    let src = "@prefix : <http://ex/>. @prefix log: <http://www.w3.org/2000/10/swap/log#>.
@forAll :x.
:a :p { :x :q :z }.
:b :p { ?x :q :z }.
{ :a :p ?f. ?f log:notEqualTo { :x :q :z } } => { :bad :is true }.
";
    // The echoed rule mentions `:bad` too — only a closure LINE starting with it is a
    // derivation.
    let derives_bad = |doc: &str| doc.lines().any(|l| l.starts_with("<http://ex/bad> "));
    let first = reason_n3_pass_all(src, RuleVars::N3).expect("pass-all");
    assert!(!derives_bad(&first), "the source does not derive :bad: {first}");
    assert!(!first.contains("__ua"), "{first}");
    let again = reason_n3_pass_all(&first, RuleVars::N3).expect("round two");
    assert!(!derives_bad(&again), "re-reasoning the output changed its meaning: {again}");
    assert_eq!(first, again);
}

/// GH #6701 review round 4 (1): a formula re-created from TEXT by `log:parsedAsN3` holds
/// the parser's own `__ua.<iri>` universal. The pass-all output must hold that same term,
/// or re-reasoning flips `log:notEqualTo` and derives `:bad`.
#[test]
fn a_universal_compares_equal_to_one_parsed_from_a_literal_after_a_round_trip() {
    let src = r#"@prefix : <http://ex/>.
@prefix log: <http://www.w3.org/2000/10/swap/log#>.
@forAll :x.
:a :p { :x :q :z }.
{
  :a :p ?f.
  "@prefix : <http://ex/>. @forAll :x. :x :q :z." log:parsedAsN3 ?g.
  ?f log:notEqualTo ?g
} => { :bad :is true }.
"#;
    let derives_bad = |doc: &str| doc.lines().any(|l| l.starts_with("<http://ex/bad> "));
    let first = reason_n3_pass_all(src, RuleVars::N3).expect("pass-all");
    assert!(!derives_bad(&first), "the source does not derive :bad: {first}");
    let again = reason_n3_pass_all(&first, RuleVars::N3).expect("round two");
    assert!(!derives_bad(&again), "re-reasoning the output changed its meaning: {again}");
    assert_eq!(first, again);
}

/// GH #6701 review round 4 (2): backward resolution standardizes a rule apart by
/// prefixing its variables (`__bw0_…`). A universal copied that way must still be written
/// as the universal — not as the unparseable variable `?__bw0___ua.http://ex/x`.
#[test]
fn a_freshened_backward_rule_universal_serializes_as_the_universal() {
    let data = "@prefix : <http://ex/>. @forAll :x.\n{ :a :p { :x :q :z } } <= true.\n";
    let query = "@prefix : <http://ex/>.\n{ :a :p ?f } => { :result :is ?f }.\n";
    let answers = reason_n3_query_terms(data, query).expect("query");
    assert_eq!(answers.len(), 1, "{answers:?}");
    let text = sparq_reason::n3::serialize::serialize_facts(answers.iter()).expect("exactly representable");
    assert!(!text.contains("__bw") && !text.contains("__ua"), "{text}");
    let back = parser::parse(&text).expect("the answer re-parses");
    // The same formula a forward derivation would carry: the universal itself.
    let want = parser::parse("@prefix : <http://ex/>. @forAll :x. :result :is { :x :q :z }.").unwrap();
    assert_eq!(back.facts, want.facts, "{text}");
}

#[test]
fn a_parse_error_propagates() {
    assert!(reason_n3_pass_all("{ ?x a :Human } =>", RuleVars::N3).is_err());
}
