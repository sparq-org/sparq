//! sq-afqnp (GH #2012) — RDF-star / RDF 1.2 quoted-triple TERMS in N3 rules.
//!
//! Fixture tests (parse / match / bind / derive / nest / non-entailment) plus a
//! DIFFERENTIAL battery: the engine's closure must equal, as a SET, the closure
//! computed by an INDEPENDENT naive reference evaluator (brute-force cartesian
//! premise matching with its own unification and substitution — no shared code
//! with the engine's `FactIndex` / semi-naive delta / transitivity fast path /
//! `order_premise` machinery; only the PARSER is shared, which is exactly the
//! scope this differential does not target).
//!
//! NON-VACUITY: verified by mutation — with the engine's quoted-triple
//! unification arm deliberately broken (component binding skipped so inner
//! variables no longer bind), `differential_quoted_triples_vs_naive_reference`
//! and the fixtures go red. See the PR body for the exact mutation.

use sparq_reason::n3::{parser, Term};
use sparq_reason::{reason_n3, reason_n3_terms};
use sparq_core::dict::Dict;
use std::collections::{HashMap, HashSet};

fn iri(s: &str) -> Term {
    Term::Iri(format!("http://ex/{s}"))
}

fn qt(s: Term, p: Term, o: Term) -> Term {
    Term::Triple(Box::new([s, p, o]))
}

/// Engine closure as a set of ground triples.
fn engine_closure(src: &str) -> HashSet<[Term; 3]> {
    reason_n3_terms(src, None).expect("engine closure").facts.into_iter().collect()
}

// ---------------------------------------------------------------------------
// Parser fixtures
// ---------------------------------------------------------------------------

#[test]
fn parser_quoted_triple_forms() {
    // `<< s p o >>` parses to a first-class quoted-triple term.
    let p = parser::parse("<http://ex/s> <http://ex/p> << <http://ex/a> <http://ex/q> <http://ex/b> >> .")
        .expect("classic form");
    assert_eq!(p.facts.len(), 1);
    assert_eq!(p.facts[0][2], qt(iri("a"), iri("q"), iri("b")));

    // The RDF 1.2 triple-term spelling `<<( s p o )>>` is the SAME term.
    let p2 = parser::parse("<http://ex/s> <http://ex/p> <<( <http://ex/a> <http://ex/q> <http://ex/b> )>> .")
        .expect("rdf 1.2 triple-term form");
    assert_eq!(p2.facts[0][2], p.facts[0][2], "<<(…)>> and << … >> are one term");

    // Nested quotation, and a quoted triple in SUBJECT position.
    let p3 = parser::parse(
        "<< << <http://ex/a> <http://ex/q> <http://ex/b> >> <http://ex/r> <http://ex/c> >> <http://ex/meta> <http://ex/m> .",
    )
    .expect("nested quoted subject");
    assert_eq!(
        p3.facts[0][0],
        qt(qt(iri("a"), iri("q"), iri("b")), iri("r"), iri("c"))
    );

    // `<< (1 2) … >>` (whitespace before the paren) keeps its LIST subject —
    // only the literal token `<<(` selects the RDF 1.2 spelling.
    let p4 = parser::parse("<< (1 2) <http://ex/q> <http://ex/o> >> <http://ex/meta> <http://ex/m> .")
        .expect("list subject in classic form");
    match &p4.facts[0][0] {
        Term::Triple(t) => assert!(matches!(&t[0], Term::List(ms) if ms.len() == 2)),
        other => panic!("expected quoted triple subject, got {other:?}"),
    }

    // Malformed forms are loud parse errors, not silent re-reads.
    for bad in [
        "<http://ex/s> <http://ex/p> << <http://ex/a> <http://ex/q> >> .", // two terms
        "<http://ex/s> <http://ex/p> << <http://ex/a> <http://ex/q> <http://ex/b> .", // unterminated
        "<http://ex/s> <http://ex/p> <<( <http://ex/a> <http://ex/q> <http://ex/b> >> .", // missing )
    ] {
        assert!(parser::parse(bad).is_err(), "expected parse error for {bad:?}");
    }

    // STRICT W3C Turtle 1.1 mode keeps rejecting quoted triples.
    assert!(
        parser::parse_turtle_with_base(
            "<http://ex/s> <http://ex/p> << <http://ex/a> <http://ex/q> <http://ex/b> >> .",
            ""
        )
        .is_err(),
        "strict Turtle must reject <<"
    );
}

// ---------------------------------------------------------------------------
// Rule fixtures
// ---------------------------------------------------------------------------

#[test]
fn match_quoted_triple_binds_inner_vars() {
    let facts = engine_closure(
        "@prefix : <http://ex/> .\n\
         << :a :p :b >> :says :alice .\n\
         { << ?s :p ?o >> :says ?w } => { ?s :linked ?o . ?w :vouches ?s } .",
    );
    assert!(facts.contains(&[iri("a"), iri("linked"), iri("b")]), "inner vars bound: {facts:?}");
    assert!(facts.contains(&[iri("alice"), iri("vouches"), iri("a")]), "outer var joins inner");
}

#[test]
fn derive_quoted_triple() {
    let facts = engine_closure(
        "@prefix : <http://ex/> .\n\
         :a :p :b .\n\
         { ?x :p ?y } => { << ?x :p ?y >> :derivedFrom :rule1 } .",
    );
    assert!(
        facts.contains(&[qt(iri("a"), iri("p"), iri("b")), iri("derivedFrom"), iri("rule1")]),
        "quoted triple derivable in a rule head: {facts:?}"
    );
}

#[test]
fn nested_quoted() {
    // Match THROUGH two levels of quotation, binding at the innermost level,
    // and derive a fresh nested quotation.
    let facts = engine_closure(
        "@prefix : <http://ex/> .\n\
         << << :a :q :b >> :r :c >> :meta :m .\n\
         { << << ?x :q ?y >> :r ?z >> :meta ?m } => { ?x :innerLinked ?y . << ?m :saw ?z >> :level :two } .",
    );
    assert!(facts.contains(&[iri("a"), iri("innerLinked"), iri("b")]), "innermost vars bind");
    assert!(
        facts.contains(&[qt(iri("m"), iri("saw"), iri("c")), iri("level"), iri("two")]),
        "nested derivation: {facts:?}"
    );
}

#[test]
fn plain_rules_unregressed() {
    // The plain socrates chain is untouched by the quoted-triple machinery.
    let facts = engine_closure(
        "@prefix : <http://ex/> .\n\
         :Socrates a :Man .\n\
         { ?x a :Man } => { ?x a :Mortal } .",
    );
    assert!(facts.contains(&[
        iri("Socrates"),
        Term::Iri(parser::RDF_TYPE.into()),
        iri("Mortal")
    ]));
    assert_eq!(facts.len(), 2, "exactly the fact + the derivation");
}

#[test]
fn quoted_vs_asserted_non_entailment() {
    // A quoted-triple pattern must NOT match a plain asserted triple …
    let facts = engine_closure(
        "@prefix : <http://ex/> .\n\
         :a :p :b .\n\
         { << ?s :p ?o >> :says ?w } => { ?s :fromQuote ?o } .",
    );
    assert!(
        !facts.iter().any(|f| f[1] == iri("fromQuote")),
        "quoted pattern fired on an asserted triple: {facts:?}"
    );
    // … and a plain pattern must NOT match INSIDE a quotation (quoting is not
    // asserting: RDF-star quoted triples carry no truth commitment).
    let facts2 = engine_closure(
        "@prefix : <http://ex/> .\n\
         << :a :p :b >> :says :alice .\n\
         { ?s :p ?o } => { ?s :fromPlain ?o } .",
    );
    assert!(
        !facts2.iter().any(|f| f[1] == iri("fromPlain")),
        "plain pattern reached inside a quotation: {facts2:?}"
    );
}

#[test]
fn blank_in_quote_premise_is_existential() {
    // A premise blank inside `<< … >>` is a rule-scoped existential: it
    // matches ANY inner subject (the same N3 semantics as a top-level blank).
    let facts = engine_closure(
        "@prefix : <http://ex/> .\n\
         << :a :p :b >> :says :alice .\n\
         { << _:any :p ?o >> :says ?w } => { ?w :heardAbout ?o } .",
    );
    assert!(facts.contains(&[iri("alice"), iri("heardAbout"), iri("b")]), "{facts:?}");
}

#[test]
fn quoted_conclusion_existential_skolemizes_per_firing() {
    // A conclusion blank INSIDE a quoted triple mints one fresh existential
    // per distinct firing (not per round — the non-monotonic re-run guard).
    let facts = engine_closure(
        "@prefix : <http://ex/> .\n\
         :a :p :b .\n\
         :c :p :d .\n\
         { ?x :p ?y } => { << _:w :witnessed ?y >> :meta :m } .",
    );
    let minted: Vec<&[Term; 3]> = facts
        .iter()
        .filter(|f| f[1] == iri("meta"))
        .collect();
    assert_eq!(minted.len(), 2, "one skolemized quoted triple per firing: {minted:?}");
    for f in &minted {
        let Term::Triple(t) = &f[0] else { panic!("expected quoted subject") };
        assert!(matches!(&t[0], Term::Blank(_)), "existential stays a blank: {t:?}");
    }
}

#[test]
fn backward_rule_proves_quoted_goal() {
    // `<=` rule whose PREMISE matches a quoted triple: the forward rule's
    // premise atom is proven goal-directed through `unify_walked`'s
    // quoted-triple arm.
    let facts = engine_closure(
        "@prefix : <http://ex/> .\n\
         << :a :p :b >> :says :alice .\n\
         { ?s :verified ?o } <= { << ?s :p ?o >> :says :alice } .\n\
         { ?s :verified ?o } => { ?s :out ?o } .",
    );
    assert!(facts.contains(&[iri("a"), iri("out"), iri("b")]), "{facts:?}");
}

#[test]
fn quoted_triples_as_join_values_through_transitive_rule() {
    // Quoted triples as plain NODES of a transitive relation — exercises the
    // engine's transitivity fast path with Triple-valued endpoints.
    let facts = engine_closure(
        "@prefix : <http://ex/> .\n\
         << :a :p :b >> :sub << :c :p :d >> .\n\
         << :c :p :d >> :sub << :e :p :f >> .\n\
         { ?x :sub ?y . ?y :sub ?z } => { ?x :sub ?z } .",
    );
    assert!(
        facts.contains(&[
            qt(iri("a"), iri("p"), iri("b")),
            iri("sub"),
            qt(iri("e"), iri("p"), iri("f"))
        ]),
        "transitive closure over quoted-triple nodes: {facts:?}"
    );
}

// ---------------------------------------------------------------------------
// Id-level interning (reason_n3 → Dict)
// ---------------------------------------------------------------------------

#[test]
fn id_level_closure_interns_quoted_triples_content_addressed() {
    let mut dict = Dict::new();
    let ids = reason_n3(
        &mut dict,
        "@prefix : <http://ex/> .\n\
         :a :p :b .\n\
         { ?x :p ?y } => { << ?x :p ?y >> :derivedFrom :rule1 } .",
    )
    .expect("id-level closure with a derived quoted triple");
    // The derived subject must be the SAME id `Dict` gives the equivalent
    // oxrdf triple term (content-addressed by component ids — store parity).
    let expected = oxrdf::Term::Triple(Box::new(oxrdf::Triple::new(
        oxrdf::NamedNode::new_unchecked("http://ex/a"),
        oxrdf::NamedNode::new_unchecked("http://ex/p"),
        oxrdf::NamedNode::new_unchecked("http://ex/b"),
    )));
    let tid = dict.lookup(&expected);
    assert_ne!(tid, sparq_core::dict::NO_ID, "triple term interned");
    let df = dict.lookup(&oxrdf::Term::from(oxrdf::NamedNode::new_unchecked("http://ex/derivedFrom")));
    assert!(
        ids.iter().any(|t| t[0] == tid && t[1] == df),
        "closure row uses the content-addressed triple-term id"
    );
}

#[test]
fn id_level_closure_rejects_non_rdf12_quoted_shapes_loudly() {
    // N3 admits a literal subject inside a quotation; RDF 1.2 triple terms do
    // not — the ID-LEVEL entry must refuse loudly (the TERM-level API,
    // `reason_n3_terms`, still handles the document).
    let mut dict = Dict::new();
    let err = reason_n3(
        &mut dict,
        "@prefix : <http://ex/> . << \"lit\" :p :o >> :meta :m .",
    )
    .expect_err("literal quoted-triple subject has no RDF 1.2 representation");
    assert!(err.contains("subject"), "error names the offending position: {err}");
    assert!(
        reason_n3_terms("@prefix : <http://ex/> . << \"lit\" :p :o >> :meta :m .", None).is_ok(),
        "term-level API still handles generalized quoted triples"
    );
}

// ---------------------------------------------------------------------------
// Differential: engine vs an INDEPENDENT naive reference evaluator
// ---------------------------------------------------------------------------

/// Reference unification — written independently of the engine's `unify_term`.
fn ref_unify(pat: &Term, val: &Term, b: &mut HashMap<String, Term>) -> bool {
    match (pat, val) {
        (Term::Var(v), _) => {
            if let Some(bound) = b.get(v) {
                bound == val
            } else {
                b.insert(v.clone(), val.clone());
                true
            }
        }
        (Term::Triple(p), Term::Triple(q)) => (0..3).all(|i| ref_unify(&p[i], &q[i], b)),
        (Term::List(ps), Term::List(vs)) => {
            ps.len() == vs.len() && ps.iter().zip(vs).all(|(p, v)| ref_unify(p, v, b))
        }
        _ => pat == val,
    }
}

/// Reference substitution — recurses through quoted triples and lists.
fn ref_subst(t: &Term, b: &HashMap<String, Term>) -> Term {
    match t {
        Term::Var(v) => b.get(v).cloned().unwrap_or_else(|| t.clone()),
        Term::Triple(tr) => Term::Triple(Box::new([
            ref_subst(&tr[0], b),
            ref_subst(&tr[1], b),
            ref_subst(&tr[2], b),
        ])),
        Term::List(ms) => Term::List(ms.iter().map(|m| ref_subst(m, b)).collect()),
        other => other.clone(),
    }
}

fn ref_ground(t: &Term) -> bool {
    match t {
        Term::Var(_) => false,
        Term::Triple(tr) => tr.iter().all(ref_ground),
        Term::List(ms) => ms.iter().all(ref_ground),
        _ => true,
    }
}

/// Naive fixpoint: every round, every rule tries EVERY assignment of premise
/// atoms to facts (brute-force backtracking over the full cartesian space) and
/// asserts each fully-ground instantiated conclusion. No indexes, no deltas,
/// no ordering — O(|facts|^|premise|) per round, fine at battery scale.
fn ref_closure(doc: &str) -> HashSet<[Term; 3]> {
    let parsed = parser::parse(doc).expect("battery doc parses");
    assert!(parsed.backward_rules.is_empty(), "battery is forward-only");
    let mut facts: HashSet<[Term; 3]> = parsed.facts.iter().cloned().collect();
    loop {
        let snapshot: Vec<[Term; 3]> = facts.iter().cloned().collect();
        let mut new_facts: Vec<[Term; 3]> = Vec::new();
        for rule in &parsed.rules {
            let mut stack: Vec<HashMap<String, Term>> = vec![HashMap::new()];
            for atom in &rule.premise {
                let mut next = Vec::new();
                for b in &stack {
                    for f in &snapshot {
                        let mut nb = b.clone();
                        if (0..3).all(|i| ref_unify(&atom[i], &f[i], &mut nb)) {
                            next.push(nb);
                        }
                    }
                }
                stack = next;
            }
            for b in &stack {
                for c in &rule.conclusion {
                    let g = [ref_subst(&c[0], b), ref_subst(&c[1], b), ref_subst(&c[2], b)];
                    if g.iter().all(ref_ground) {
                        new_facts.push(g);
                    }
                }
            }
        }
        let before = facts.len();
        facts.extend(new_facts);
        if facts.len() == before {
            return facts;
        }
    }
}

/// The seeded battery: every quoted-triple usage mode, each a self-contained
/// N3 document (plain join atoms + quoted-triple terms only — the constructs
/// whose match/bind/derive logic this bead adds).
const BATTERY: &[(&str, &str)] = &[
    (
        "match-only",
        "@prefix : <http://ex/> .\n\
         << :a :p :b >> :says :alice .\n\
         << :c :p :d >> :says :bob .\n\
         { << ?s :p ?o >> :says ?w } => { ?s :linked ?o . ?w :vouches ?s } .",
    ),
    (
        "derive-only",
        "@prefix : <http://ex/> .\n\
         :a :p :b . :c :p :d .\n\
         { ?x :p ?y } => { << ?x :p ?y >> :derivedFrom :r1 } .",
    ),
    (
        "chain: derive quoted then match it",
        "@prefix : <http://ex/> .\n\
         :a :p :b .\n\
         { ?x :p ?y } => { << ?x :p ?y >> :level :one } .\n\
         { << ?x :p ?y >> :level :one } => { ?y :reachedFrom ?x } .",
    ),
    (
        "nested quotation",
        "@prefix : <http://ex/> .\n\
         << << :a :q :b >> :r :c >> :meta :m1 .\n\
         << << :d :q :e >> :r :f >> :meta :m2 .\n\
         { << << ?x :q ?y >> :r ?z >> :meta ?m } => { ?x :inner ?y . << ?m :saw ?z >> :lvl :two } .",
    ),
    (
        "mixed plain + quoted join",
        "@prefix : <http://ex/> .\n\
         << :a :p :b >> :says :alice .\n\
         << :c :p :d >> :says :eve .\n\
         :alice :trusted :yes .\n\
         { << ?s :p ?o >> :says ?w . ?w :trusted :yes } => { ?s :p2 ?o } .",
    ),
    (
        "fully-variable quote (var predicate inside)",
        "@prefix : <http://ex/> .\n\
         << :a :p :b >> :says :alice .\n\
         << :c :q :d >> :says :alice .\n\
         { << ?s ?p ?o >> :says ?w } => { ?s ?p ?o } .",
    ),
    (
        "quoted subject AND object in one atom",
        "@prefix : <http://ex/> .\n\
         << :a :p :b >> :implies << :c :q :d >> .\n\
         << :c :q :d >> :says :w .\n\
         { << ?s :p ?o >> :implies << ?s2 :q ?o2 >> } => { ?s2 :from ?s . ?o2 :from ?o } .",
    ),
    (
        "repeated var inside and outside the quote",
        "@prefix : <http://ex/> .\n\
         << :x :p :x >> :says :x .\n\
         << :a :p :b >> :says :a .\n\
         { << ?x :p ?x >> :says ?x } => { ?x :selfQuoted :yes } .",
    ),
    (
        "ground quoted-triple constant in the premise",
        "@prefix : <http://ex/> .\n\
         << :a :p :b >> :says :alice .\n\
         << :a :p :c >> :says :bob .\n\
         { << :a :p :b >> :says ?w } => { ?w :saidExactly :ab } .",
    ),
    (
        "transitive relation over quoted-triple nodes",
        "@prefix : <http://ex/> .\n\
         << :a :p :b >> :sub << :c :p :d >> .\n\
         << :c :p :d >> :sub << :e :p :f >> .\n\
         << :e :p :f >> :sub << :g :p :h >> .\n\
         { ?x :sub ?y . ?y :sub ?z } => { ?x :sub ?z } .",
    ),
    (
        "no cross-talk: quoted pattern vs asserted triple and back",
        "@prefix : <http://ex/> .\n\
         :a :p :b .\n\
         << :c :p :d >> :says :w .\n\
         { << ?s :p ?o >> :says ?w2 } => { ?s :fromQuote ?o } .\n\
         { ?s :p ?o } => { ?s :fromPlain ?o } .",
    ),
];

#[test]
fn differential_quoted_triples_vs_naive_reference() {
    for (name, doc) in BATTERY {
        let engine = engine_closure(doc);
        let reference = ref_closure(doc);
        let only_engine: Vec<_> = engine.difference(&reference).collect();
        let only_ref: Vec<_> = reference.difference(&engine).collect();
        assert!(
            only_engine.is_empty() && only_ref.is_empty(),
            "closure divergence on battery case {name:?}:\n  engine-only: {only_engine:?}\n  reference-only: {only_ref:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Incremental graph: quoted-triple rules disqualify the counting profile and
// stay CORRECT through the engine fallback (which round-trips << … >> through
// the N3 serializer).
// ---------------------------------------------------------------------------

#[test]
fn incremental_graph_falls_back_and_stays_correct_with_quoted_rules() {
    use sparq_reason::{MaterializedN3Graph, N3Mode};
    let rules = "@prefix : <http://ex/> .\n\
                 { << ?s :p ?o >> :says ?w } => { ?s :linked ?o } .";
    let base = vec![[qt(iri("a"), iri("p"), iri("b")), iri("says"), iri("alice")]];
    let mut g = MaterializedN3Graph::new(rules, &base).expect("graph builds");
    assert_eq!(g.mode(), N3Mode::Fallback, "quoted-triple rules are outside the counting profile");
    assert!(g.contains(&[iri("a"), iri("linked"), iri("b")]), "fallback closure is correct");
    // A mutation re-runs the batch engine — quoted-triple facts round-trip
    // through the serializer and keep deriving.
    g.insert(&[[qt(iri("c"), iri("p"), iri("d")), iri("says"), iri("bob")]]);
    assert!(g.contains(&[iri("c"), iri("linked"), iri("d")]), "post-insert closure is correct");
}
