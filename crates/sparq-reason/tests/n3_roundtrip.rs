//! GH #6701 review rounds 1–6: the N3 writer's round-trip contract, checked over an
//! enumeration of term shapes rather than one example per review round.
//!
//! The contract (`n3::serialize`, the `Unit` docs), checked here per OCCURRENCE by an
//! independent oracle: `parse(write(x))` is `x` exactly, except that
//!
//! * a backward-chaining copy `__bw<n>___ua.<iri>` reads back as the universal;
//! * a universal no `@forAll` can scope — outside every formula of a statement, or at a
//!   formula level that mentions its IRI plainly at or after its first use there — reads
//!   back as a plain variable: one name per universal per statement, never the name of a
//!   source variable of that statement (the one lossy case).
//!
//! Every other formula therefore re-parses to a term EQUAL to the original — the same
//! term a `log:parsedAsN3` literal of that text yields. On top of that: a statement renders
//! the same whatever surrounds it, identity keys (`statement_keys`, which provenance
//! addresses facts by) are injective, and reasoning over a written document derives what
//! the source derives.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use sparq_reason::n3::serialize::{serialize_facts, statement_keys, write_statement};
use sparq_reason::n3::{parser, Term};
use sparq_reason::{reason_n3_pass_all, reason_n3_terms, RuleVars};

const UA: &str = "__ua.";
/// Oracle placeholder for "this occurrence falls back to a plain variable".
const FB: &str = "#fallback:";

fn iri(s: &str) -> Term {
    Term::Iri(s.into())
}
fn var(s: &str) -> Term {
    Term::Var(s.into())
}
fn formula(rows: Vec<[Term; 3]>) -> Term {
    Term::Formula(rows)
}

/// The atoms every shape is built from: a universal, its freshened copy, the plain IRI it
/// was declared from, a source variable with the universal's local name, a source variable
/// spelled like the first collision-free fallback, an IRI and a universal that need
/// `IRIREF` escaping, a universal from another namespace with the same local name, and an
/// unrelated constant.
fn atoms() -> Vec<Term> {
    vec![
        var("__ua.http://ex/x"),
        var("__bw0___ua.http://ex/x"),
        iri("http://ex/x"),
        var("x"),
        var("x_2"),
        iri("http://ex/a\\b"),
        var("__ua.http://ex/a\\b"),
        iri("http://ex/k"),
        var("__ua.http://other/x"),
    ]
}

/// `__bw<n>_` copies of a universal read back as the universal.
fn normalise(t: &Term) -> Term {
    match t {
        Term::Var(v) => {
            let mut rest = v.as_str();
            while let Some(r) = rest.strip_prefix("__bw") {
                let d = r.bytes().take_while(u8::is_ascii_digit).count();
                match r[d..].strip_prefix('_') {
                    Some(r2) if d > 0 && r2.starts_with("__") => rest = r2,
                    _ => break,
                }
            }
            if rest.starts_with(UA) { var(rest) } else { t.clone() }
        }
        Term::List(ms) => Term::List(ms.iter().map(normalise).collect()),
        Term::Triple(tr) => Term::Triple(Box::new(tr.clone().map(|m| normalise(&m)))),
        Term::Formula(ts) => formula(ts.iter().map(|r| r.clone().map(|m| normalise(&m))).collect()),
        _ => t.clone(),
    }
}

fn mentions(t: &Term, i: &str) -> bool {
    match t {
        Term::Iri(x) => x == i,
        Term::List(ms) => ms.iter().any(|m| mentions(m, i)),
        Term::Triple(tr) => tr.iter().any(|m| mentions(m, i)),
        Term::Formula(ts) => ts.iter().flatten().any(|m| mentions(m, i)),
        _ => false,
    }
}

fn level_universals(t: &Term, out: &mut BTreeSet<String>) {
    match t {
        Term::Var(v) if v.starts_with(UA) => {
            out.insert(v[UA.len()..].to_string());
        }
        Term::List(ms) => ms.iter().for_each(|m| level_universals(m, out)),
        Term::Triple(tr) => tr.iter().for_each(|m| level_universals(m, out)),
        _ => {}
    }
}

/// The oracle: `t` (already normalised) with every occurrence that MUST fall back
/// replaced by a placeholder. `scoped`: the universals declarable at this level.
fn expect(t: &Term, scoped: &BTreeSet<String>) -> Term {
    match t {
        Term::Var(v) if v.starts_with(UA) => {
            let i = &v[UA.len()..];
            if scoped.contains(i) { t.clone() } else { var(&format!("{FB}{i}")) }
        }
        Term::List(ms) => Term::List(ms.iter().map(|m| expect(m, scoped)).collect()),
        Term::Triple(tr) => Term::Triple(Box::new(tr.clone().map(|m| expect(&m, scoped)))),
        Term::Formula(ts) => {
            // Declarable here: no plain mention at or after the first triple using it.
            let mut ok = BTreeSet::new();
            let mut seen = BTreeSet::new();
            for (n, row) in ts.iter().enumerate() {
                let mut here = BTreeSet::new();
                row.iter().for_each(|m| level_universals(m, &mut here));
                for i in here {
                    if seen.insert(i.clone()) && !ts[n..].iter().flatten().any(|m| mentions(m, &i)) {
                        ok.insert(i);
                    }
                }
            }
            formula(ts.iter().map(|r| r.clone().map(|m| expect(&m, &ok))).collect())
        }
        _ => t.clone(),
    }
}

fn source_vars(t: &Term, out: &mut BTreeSet<String>) {
    match t {
        Term::Var(v) if !v.starts_with(UA) && !v.starts_with(FB) => {
            out.insert(v.clone());
        }
        Term::List(ms) => ms.iter().for_each(|m| source_vars(m, out)),
        Term::Triple(tr) => tr.iter().for_each(|m| source_vars(m, out)),
        Term::Formula(ts) => ts.iter().flatten().for_each(|m| source_vars(m, out)),
        _ => {}
    }
}

/// `got` (read back) matches `want` (oracle): identical, except each placeholder is one
/// plain variable per universal, distinct per universal, and not a source variable.
fn matches(want: &Term, got: &Term, names: &mut HashMap<String, String>, sources: &BTreeSet<String>) -> bool {
    match (want, got) {
        (Term::Var(w), Term::Var(g)) if w.starts_with(FB) => {
            if sources.contains(g) || g.starts_with(UA) {
                return false;
            }
            match names.get(w) {
                Some(n) => n == g,
                None => {
                    if names.values().any(|n| n == g) {
                        return false;
                    }
                    names.insert(w.clone(), g.clone());
                    true
                }
            }
        }
        (Term::List(a), Term::List(b)) => a.len() == b.len() && a.iter().zip(b).all(|(x, y)| matches(x, y, names, sources)),
        (Term::Triple(a), Term::Triple(b)) => a.iter().zip(b.iter()).all(|(x, y)| matches(x, y, names, sources)),
        (Term::Formula(a), Term::Formula(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(r, s)| r.iter().zip(s).all(|(x, y)| matches(x, y, names, sources)))
        }
        _ => want == got,
    }
}

/// Every formula term in `t` (any depth).
fn formulas<'a>(t: &'a Term, out: &mut Vec<&'a Term>) {
    match t {
        Term::Formula(ts) => {
            out.push(t);
            ts.iter().flatten().for_each(|m| formulas(m, out));
        }
        Term::List(ms) => ms.iter().for_each(|m| formulas(m, out)),
        Term::Triple(tr) => tr.iter().for_each(|m| formulas(m, out)),
        _ => {}
    }
}

fn has_placeholder(t: &Term) -> bool {
    match t {
        Term::Var(x) => x.starts_with(FB),
        Term::List(ms) => ms.iter().any(has_placeholder),
        Term::Triple(tr) => tr.iter().any(has_placeholder),
        Term::Formula(ts) => ts.iter().flatten().any(has_placeholder),
        _ => false,
    }
}

/// Check one statement: written alone and inside a document between noise statements.
fn check_statement(s: &[Term; 3]) -> String {
    let mut text = String::new();
    write_statement(s, &mut text);
    let back = parser::parse(&text).unwrap_or_else(|e| panic!("does not re-parse ({e}):\n{text}"));
    assert!(back.rules.is_empty() && back.facts.len() == 1, "{text}");
    let norm = s.clone().map(|t| normalise(&t));
    // Top level: no formula scope, so no universal is declared there.
    let want = norm.clone().map(|t| expect(&t, &BTreeSet::new()));
    let mut sources = BTreeSet::new();
    norm.iter().for_each(|t| source_vars(t, &mut sources));
    let mut names = HashMap::new();
    assert!(
        want.iter().zip(&back.facts[0]).all(|(w, g)| matches(w, g, &mut names, &sources)),
        "not the contract:\n  wrote {s:?}\n  want  {want:?}\n  read  {:?}\n  text  {text}",
        back.facts[0]
    );
    // (a) formula identity: a formula with no fallback inside re-parses EQUAL.
    let (mut wf, mut gf) = (Vec::new(), Vec::new());
    want.iter().for_each(|t| formulas(t, &mut wf));
    back.facts[0].iter().for_each(|t| formulas(t, &mut gf));
    assert_eq!(wf.len(), gf.len(), "{text}");
    for (w, g) in wf.iter().zip(&gf) {
        if !has_placeholder(w) {
            assert_eq!(w, g, "a formula changed identity:\n{text}");
        }
    }
    // (c) render independence: same line inside a document of unrelated statements.
    let noise = [
        [iri("http://ex/x"), iri("http://ex/k"), var("__ua.http://ex/x")],
        [var("x"), iri("http://ex/k"), formula(vec![[iri("http://ex/x"), iri("http://ex/k"), var("x_2")]])],
    ];
    let doc = serialize_facts([&noise[0], s, &noise[1]].into_iter());
    assert_eq!(doc.lines().nth(1).map(|l| format!("{l}\n")), Some(text.clone()), "rendering depends on neighbours:\n{doc}");
    text
}

/// N3 string-literal body for `text`.
fn n3_string(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
}

/// (a) against the reasoner: `log:parsedAsN3` of the written text yields the same formula
/// the writer started from (for a statement with no fallback).
fn check_parsed_as_n3(s: &[Term; 3], text: &str) {
    let norm = s.clone().map(|t| normalise(&t));
    if norm.iter().any(|t| has_placeholder(&expect(t, &BTreeSet::new()))) {
        return;
    }
    let src = format!(
        "{{ \"{}\" <http://www.w3.org/2000/10/swap/log#parsedAsN3> ?g }} => {{ <http://ex/got> <http://ex/is> ?g }}.\n",
        n3_string(text)
    );
    let closure = reason_n3_terms(&src, None).unwrap_or_else(|e| panic!("{e}\n{src}")).facts;
    let got = closure.iter().find(|f| f[0] == iri("http://ex/got")).unwrap_or_else(|| panic!("no parse:\n{src}"));
    assert_eq!(got[2], formula(vec![norm]), "log:parsedAsN3 disagrees:\n{text}");
}

#[test]
fn every_shape_round_trips() {
    let a = atoms();
    let k = iri("http://ex/k");
    let lit = Term::Lit("l".into(), "http://ex/d\\t".into(), None);
    let mut all: Vec<[Term; 3]> = Vec::new();
    // 1. one triple in a formula, under each kind of subject
    for s in [&a[0], &a[2], &a[3], &a[8], &k] {
        for x in &a {
            for y in a.iter().chain([&lit]) {
                all.push([s.clone(), k.clone(), formula(vec![[x.clone(), k.clone(), y.clone()]])]);
            }
        }
    }
    // 2. two triples in one formula: declaration placement and same-triple mixing
    let few = [&a[0], &a[1], &a[2], &a[3], &a[4], &a[8]];
    for x in few {
        for y in few {
            for z in few {
                for w in few {
                    let f = formula(vec![[x.clone(), k.clone(), y.clone()], [z.clone(), k.clone(), w.clone()]]);
                    all.push([k.clone(), k.clone(), f]);
                }
            }
        }
    }
    // 3. nested formulae — inner scopes decide for themselves
    let mid = [&a[0], &a[2], &a[3], &a[4], &a[5], &a[6]];
    for x in mid {
        for y in mid {
            for z in mid {
                let inner = formula(vec![[y.clone(), k.clone(), z.clone()]]);
                all.push([x.clone(), k.clone(), formula(vec![[x.clone(), k.clone(), inner]])]);
            }
        }
    }
    // 4. lists and quoted triples inside a formula
    for x in few {
        for y in few {
            for z in few {
                let row = [Term::List(vec![x.clone(), y.clone()]), k.clone(), Term::Triple(Box::new([z.clone(), k.clone(), x.clone()]))];
                all.push([k.clone(), k.clone(), formula(vec![row])]);
            }
        }
    }
    // 5. bare top-level terms
    for x in &a {
        for y in a.iter().chain([&lit]) {
            all.push([x.clone(), k.clone(), y.clone()]);
        }
    }
    assert!(all.len() > 2200, "{} shapes", all.len());
    for (n, s) in all.iter().enumerate() {
        let text = check_statement(s);
        if n % 23 == 0 {
            check_parsed_as_n3(s, &text);
        }
    }
    // (b) identity keys: distinct statements ↔ distinct keys; the same statement, the same key.
    let mut by_key: BTreeMap<[String; 3], &[Term; 3]> = BTreeMap::new();
    for s in &all {
        let key = statement_keys(s);
        assert_eq!(key, statement_keys(&s.clone()));
        if let Some(prev) = by_key.insert(key, s) {
            assert_eq!(prev, s, "two different facts share an identity key");
        }
    }
    let distinct: BTreeSet<String> = all.iter().map(|s| format!("{s:?}")).collect();
    assert_eq!(by_key.len(), distinct.len());
}

/// Reasoning over a written document derives what the source derives: the pass-all output
/// is a fixpoint, and adding the same new facts to the source and to the written document
/// yields the same pass-all output.
#[test]
fn derivations_survive_the_round_trip() {
    let cases: [(&str, &str); 5] = [
        // Codex round 5 (1): body and head share a universal; the body also mentions the
        // IRI plainly, before its declaration.
        (
            "@prefix : <http://ex/>.\n{ :marker :ref :x. @forAll :x. :x :p :b } => { @forAll :x. :x :q :b }.\n:c :p :b. :marker :ref :x.\n",
            "<http://ex/d> <http://ex/p> <http://ex/b> .\n",
        ),
        // Codex round 2: a formula fact compared with the rule's own formula.
        (
            "@prefix : <http://ex/>. @prefix log: <http://www.w3.org/2000/10/swap/log#>.\n@forAll :x.\n:a :p { :x :q :z }.\n:b :p { ?x :q :z }.\n{ :a :p ?f. ?f log:notEqualTo { :x :q :z } } => { :bad :is true }.\n",
            "<http://ex/e> <http://ex/p> <http://ex/z> .\n",
        ),
        // Codex round 4 (1): against a formula parsed from a literal.
        (
            "@prefix : <http://ex/>. @prefix log: <http://www.w3.org/2000/10/swap/log#>.\n@forAll :x.\n:a :p { :x :q :z }.\n{ :a :p ?f. \"@prefix : <http://ex/>. @forAll :x. :x :q :z.\" log:parsedAsN3 ?g. ?f log:notEqualTo ?g } => { :bad :is true }.\n",
            "<http://ex/e> <http://ex/p> <http://ex/z> .\n",
        ),
        // Codex round 6 (1): the same, with the universal's IRI ALSO used plainly and
        // bare at the top level — the formula must still declare for itself.
        (
            "@prefix : <http://ex/>. @prefix log: <http://www.w3.org/2000/10/swap/log#>.\n:x :marker :z.\n@forAll :x.\n:x :other :z.\n:a :p { :x :q :z }.\n{ :a :p ?f. \"@prefix : <http://ex/>. @forAll :x. :x :q :z.\" log:parsedAsN3 ?g. ?f log:notEqualTo ?g } => { :bad :is true }.\n",
            "<http://ex/e> <http://ex/p> <http://ex/z> .\n",
        ),
        // A universal in the body and in a nested head formula, beside a source `?x`.
        (
            "@prefix : <http://ex/>. @forAll :x.\n{ :x :p ?x } => { :x :q { :x :r ?x } }.\n:a :p :b.\n",
            "<http://ex/c> <http://ex/p> <http://ex/d> .\n",
        ),
    ];
    let derives_bad = |doc: &str| doc.lines().any(|l| l.starts_with("<http://ex/bad> "));
    for (src, extra) in cases {
        let doc = reason_n3_pass_all(src, RuleVars::N3).unwrap_or_else(|e| panic!("{e}\n{src}"));
        assert!(!doc.contains("__ua") && !doc.contains("__bw"), "{doc}");
        assert!(!derives_bad(&doc), "{doc}");
        assert_eq!(reason_n3_pass_all(&doc, RuleVars::N3).expect("round two"), doc, "not a fixpoint:\n{doc}");
        let grown_src = reason_n3_pass_all(&format!("{src}{extra}"), RuleVars::N3).expect("source + extra");
        let grown_doc = reason_n3_pass_all(&format!("{doc}{extra}"), RuleVars::N3).expect("written + extra");
        assert_eq!(grown_src, grown_doc, "the written rules derive differently:\n{src}");
    }
    // Codex round 5 (1): the echoed rule still fires on new matching facts.
    let doc = reason_n3_pass_all(cases[0].0, RuleVars::N3).unwrap();
    let grown = reason_n3_pass_all(&format!("{doc}{}", cases[0].1), RuleVars::N3).unwrap();
    assert!(grown.contains("<http://ex/d> <http://ex/q> <http://ex/b> ."), "{grown}");
}

/// The lossy case, pinned: a derivation that puts the plain IRI and the universal in ONE
/// triple. The derived fact falls back to a plain variable (distinct from the rule's source
/// `?x`), while the rule — which can declare — keeps the universal; so re-reasoning derives
/// the universal form again beside the written fallback form. Nothing merges.
#[test]
fn the_documented_lossy_case() {
    let src = "@prefix : <http://ex/>.\n:x :p :o.\n@forAll :x.\n{ ?s :p ?o. ?x :w ?y } => { :out :has { ?s :link :x } }.\n:k :w :v.\n";
    let doc = reason_n3_pass_all(src, RuleVars::N3).expect("pass-all");
    assert!(doc.contains("<http://ex/out> <http://ex/has> { <http://ex/x> <http://ex/link> ?x . } ."), "{doc}");
    assert!(doc.contains("{ @forAll <http://ex/x> . ?s <http://ex/link> <http://ex/x> . }"), "{doc}");
    let rules = |d: &str| {
        parser::parse(d).unwrap().rules.into_iter().map(|r| (r.premise, r.conclusion)).collect::<Vec<_>>()
    };
    assert_eq!(rules(&doc), rules(src), "the rule itself round-trips exactly");
}

/// Codex round 5 (2): a source variable spelled like a fallback name stays distinct.
#[test]
fn a_source_variable_spelled_like_the_fallback_stays_distinct() {
    let src = "@prefix : <http://ex/>.\n:a :p { :x :q :z. @forAll :x. :x :r ?x. ?x_2 :s :x }.\n";
    let back = |doc: &str| parser::parse(doc).expect("re-parses").facts;
    let doc = reason_n3_pass_all(src, RuleVars::N3).expect("pass-all");
    assert_eq!(back(&doc), back(src), "{doc}");
    // And where no declaration fits (one triple mixes the plain IRI and the universal):
    let x = var("__ua.http://ex/x");
    let f = [iri("http://ex/a"), iri("http://ex/p"), formula(vec![
        [x.clone(), iri("http://ex/x"), var("x")],
        [var("x_2"), iri("http://ex/q"), x],
    ])];
    let text = check_statement(&f);
    let vars: Vec<&str> = text.split_whitespace().filter(|w| w.starts_with('?')).collect();
    assert_eq!(vars, ["?x_3", "?x", "?x_2", "?x_3"], "{text}");
}

/// Codex round 5 (3): an IRI holding a decoded backslash goes back out as `\`.
#[test]
fn a_backslash_iri_round_trips() {
    let src = "@forAll <http://ex/a\\u005Cb>.\n<http://ex/s> <http://ex/p> { <http://ex/a\\u005Cb> <http://ex/q> <http://ex/o> }.\n<http://ex/t> <http://ex/p> <http://ex/c\\u005Cd>.\n";
    let doc = reason_n3_pass_all(src, RuleVars::N3).expect("pass-all");
    let back = parser::parse(&doc).unwrap_or_else(|e| panic!("{e}\n{doc}"));
    assert_eq!(back.facts.len(), 2);
    for f in &parser::parse(src).unwrap().facts {
        assert!(back.facts.contains(f), "{f:?} lost:\n{doc}");
    }
}
