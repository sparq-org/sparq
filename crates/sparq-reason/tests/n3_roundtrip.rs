//! GH #6701 review rounds 1–5: the N3 writer's round-trip contract, checked over an
//! enumeration of term shapes rather than one example per review round.
//!
//! Contract (`n3::serialize`, the `Unit` docs): `parse(write(x)) == x` up to two
//! normalisations — a backward-chaining copy of a universal (`__bw<n>___ua.<iri>`) reads
//! back as the universal (`__ua.<iri>`), and a variable the writer cannot declare or spell
//! is renamed by a BIJECTION (distinct variables stay distinct, shared ones stay shared,
//! source variables keep their names). A universal whose IRI the unit never mentions
//! plainly is never renamed. And reasoning over a written document derives what the
//! original derives.

use std::collections::HashMap;

use sparq_reason::n3::serialize::{serialize_facts, write_statement};
use sparq_reason::n3::{parser, Term};
use sparq_reason::{reason_n3_pass_all, RuleVars};

const UA: &str = "__ua.";

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
/// `IRIREF` escaping, and an unrelated constant.
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

/// Is `t` (from what was written) the same as `u` (what was read back) up to a variable
/// bijection that fixes every source variable? `fwd`/`bwd` carry the bijection across
/// the whole unit.
fn alpha_eq(t: &Term, u: &Term, fwd: &mut HashMap<String, String>, bwd: &mut HashMap<String, String>) -> bool {
    match (t, u) {
        (Term::Var(a), Term::Var(b)) => {
            if !a.starts_with(UA) && a == b {
                // a source variable keeps its name — and nothing else may take it
                return bwd.get(b).is_none_or(|x| x == a) && {
                    bwd.insert(b.clone(), a.clone());
                    fwd.insert(a.clone(), b.clone());
                    true
                };
            }
            if !a.starts_with(UA) {
                return false;
            }
            match (fwd.get(a), bwd.get(b)) {
                (Some(x), _) if x != b => false,
                (_, Some(y)) if y != a => false,
                _ => {
                    fwd.insert(a.clone(), b.clone());
                    bwd.insert(b.clone(), a.clone());
                    true
                }
            }
        }
        (Term::List(a), Term::List(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| alpha_eq(x, y, fwd, bwd))
        }
        (Term::Triple(a), Term::Triple(b)) => a.iter().zip(b.iter()).all(|(x, y)| alpha_eq(x, y, fwd, bwd)),
        (Term::Formula(a), Term::Formula(b)) => {
            a.len() == b.len()
                && a.iter().zip(b).all(|(r, s)| r.iter().zip(s).all(|(x, y)| alpha_eq(x, y, fwd, bwd)))
        }
        _ => t == u,
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

/// Universals OUTSIDE every formula (no scope a lone statement can declare them in).
fn bare_universals(t: &Term, out: &mut Vec<String>) {
    match t {
        Term::Var(v) if v.starts_with(UA) => out.push(v[UA.len()..].to_string()),
        Term::List(ms) => ms.iter().for_each(|m| bare_universals(m, out)),
        Term::Triple(tr) => tr.iter().for_each(|m| bare_universals(m, out)),
        _ => {}
    }
}

/// Check one written unit against the statements it was written from.
fn check(written: &str, stmts: &[[Term; 3]], document: bool) {
    let back = parser::parse(written).unwrap_or_else(|e| panic!("does not re-parse ({e}):\n{written}"));
    assert!(back.rules.is_empty() && back.backward_rules.is_empty(), "{written}");
    assert_eq!(back.facts.len(), stmts.len(), "{written}");
    let want: Vec<[Term; 3]> = stmts.iter().map(|s| s.clone().map(|t| normalise(&t))).collect();
    let (mut fwd, mut bwd) = (HashMap::new(), HashMap::new());
    for (w, b) in want.iter().zip(&back.facts) {
        assert!(
            w.iter().zip(b).all(|(x, y)| alpha_eq(x, y, &mut fwd, &mut bwd)),
            "not the same up to a variable bijection:\n  wrote {w:?}\n  read  {b:?}\n  text  {written}"
        );
    }
    // A universal is renamed only when the unit forces it: its IRI also appears plainly,
    // or it sits outside every formula of a lone statement.
    let mut bare = Vec::new();
    want.iter().flatten().for_each(|t| bare_universals(t, &mut bare));
    for (a, b) in &fwd {
        if let Some(i) = a.strip_prefix(UA) {
            let forced = want.iter().flatten().any(|t| mentions(t, i)) || (!document && bare.iter().any(|x| x == i));
            assert!(forced || a == b, "{a} renamed to {b} without cause:\n{written}");
        }
    }
}

fn check_statement(s: &[Term; 3]) {
    let mut w = String::new();
    write_statement(s, &mut w);
    check(&w, std::slice::from_ref(s), false);
    check(&serialize_facts(std::iter::once(s)), std::slice::from_ref(s), true);
}

#[test]
fn every_shape_round_trips() {
    let a = atoms();
    let k = iri("http://ex/k");
    let lit = Term::Lit("l".into(), "http://ex/d\\t".into(), None);
    let mut n = 0;
    // 1. one triple in a formula, under each kind of subject
    for s in [&a[0], &a[2], &a[3], &k] {
        for x in &a {
            for y in a.iter().chain([&lit]) {
                check_statement(&[s.clone(), k.clone(), formula(vec![[x.clone(), k.clone(), y.clone()]])]);
                n += 1;
            }
        }
    }
    // 2. two triples in one formula: declaration placement and same-triple mixing
    let few = &a[..5];
    for x in few {
        for y in few {
            for z in few {
                for w in few {
                    let f = formula(vec![[x.clone(), k.clone(), y.clone()], [z.clone(), k.clone(), w.clone()]]);
                    check_statement(&[k.clone(), k.clone(), f]);
                    n += 1;
                }
            }
        }
    }
    // 3. nested formulae (an enclosing declaration must not capture an inner plain IRI)
    let mid = [&a[0], &a[2], &a[3], &a[4], &a[5], &a[6]];
    for x in mid {
        for y in mid {
            for z in mid {
                let inner = formula(vec![[y.clone(), k.clone(), z.clone()]]);
                check_statement(&[k.clone(), k.clone(), formula(vec![[x.clone(), k.clone(), inner]])]);
                n += 1;
            }
        }
    }
    // 4. lists and quoted triples inside a formula
    for x in few {
        for y in few {
            for z in few {
                let row = [Term::List(vec![x.clone(), y.clone()]), k.clone(), Term::Triple(Box::new([z.clone(), k.clone(), x.clone()]))];
                check_statement(&[k.clone(), k.clone(), formula(vec![row])]);
                n += 1;
            }
        }
    }
    // 5. bare top-level terms, alone and in a document with a formula that shares them
    let mut docs = 0;
    for x in &a {
        for y in a.iter().chain([&lit]) {
            let bare = [x.clone(), k.clone(), y.clone()];
            check_statement(&bare);
            for other in [&a[0], &a[2], &a[3]] {
                let f = [k.clone(), k.clone(), formula(vec![[other.clone(), k.clone(), x.clone()]])];
                let doc = [bare.clone(), f];
                check(&serialize_facts(doc.iter()), &doc, true);
                docs += 1;
            }
            n += 1;
        }
    }
    assert!(n > 1300 && docs > 200, "{n} shapes, {docs} documents");
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
        // A universal in the body and in a nested head formula, beside a source `?x`.
        (
            "@prefix : <http://ex/>. @forAll :x.\n{ :x :p ?x } => { :x :q { :x :r ?x } }.\n:a :p :b.\n",
            "<http://ex/c> <http://ex/p> <http://ex/d> .\n",
        ),
        // A derivation that puts the plain IRI and the universal in ONE triple — the case
        // that must fall back to a collision-free variable — next to a source `?x`.
        (
            "@prefix : <http://ex/>.\n:x :p :o.\n@forAll :x.\n{ ?s :p ?o. ?x :w ?y } => { :out :has { ?s :link :x } }.\n:k :w :v.\n",
            "<http://ex/m> <http://ex/p> <http://ex/n> .\n",
        ),
    ];
    for (src, extra) in cases {
        let doc = reason_n3_pass_all(src, RuleVars::N3).unwrap_or_else(|e| panic!("{e}\n{src}"));
        assert!(!doc.contains("__ua") && !doc.contains("__bw"), "{doc}");
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
    let text = serialize_facts(std::iter::once(&f));
    check(&text, std::slice::from_ref(&f), true);
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
    let orig = parser::parse(src).unwrap().facts;
    for f in &orig {
        assert!(back.facts.contains(f), "{f:?} lost:\n{doc}");
    }
}
