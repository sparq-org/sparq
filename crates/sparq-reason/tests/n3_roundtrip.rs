//! GH #6701 review rounds 1–7: the N3 writer's contract, checked over an enumeration of
//! term shapes and a corpus of documents rather than one example per review round.
//!
//! The contract (`n3::serialize`, the `Unit` docs): every RE-REASONABLE writer is exact or
//! refuses. `parse(write(x))` is `x` exactly — a backward-chaining copy
//! `__bw<n>___ua.<iri>` reading back as the universal it copies is the one normalisation —
//! or the write returns `NotRepresentable`, which it does exactly for the two shapes with no
//! lossless N3 form (checked against an independent oracle): a universal outside every
//! formula of a statement, or one at a formula level that mentions its IRI plainly at or
//! after its first use there. There is no fallback spelling.
//!
//! Semantics, not just syntax: for every document in a corpus (Codex's examples among it),
//! `reason_n3_pass_all` either refuses or writes a document whose closure — read directly,
//! through `log:conclusion` of each formula-valued fact, and through `log:semantics` +
//! `log:conclusion` of the whole document — equals the source's. On top of that: a
//! statement renders the same whatever surrounds it, and identity keys (`statement_keys`,
//! which provenance addresses facts by) are injective over every `Term` field.

use std::collections::{BTreeMap, BTreeSet};

use sparq_reason::n3::serialize::{serialize_facts, statement_keys, write_statement};
use sparq_reason::n3::Resolver;
use sparq_reason::n3::{parser, Term};
use sparq_reason::n3::reason_n3_terms_with_resolver;
use sparq_reason::{reason_n3_pass_all, RuleVars};

const UA: &str = "__ua.";
/// Oracle placeholder for "no `@forAll` can scope this occurrence".
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

/// The oracle: `t` (already normalised) with every occurrence no `@forAll` can scope
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

fn has_placeholder(t: &Term) -> bool {
    match t {
        Term::Var(x) => x.starts_with(FB),
        Term::List(ms) => ms.iter().any(has_placeholder),
        Term::Triple(tr) => tr.iter().any(has_placeholder),
        Term::Formula(ts) => ts.iter().flatten().any(has_placeholder),
        _ => false,
    }
}

/// Does the oracle say `s` has no lossless form?
fn lossy(s: &[Term; 3]) -> bool {
    s.iter().any(|t| has_placeholder(&expect(&normalise(t), &BTreeSet::new())))
}

/// Check one statement: exact or refused (exactly when the oracle says lossy), and the same
/// outcome inside a document between unrelated statements. The written text, if any.
fn check_statement(s: &[Term; 3]) -> Option<String> {
    let mut text = String::new();
    let wrote = write_statement(s, &mut text);
    let noise = [
        [iri("http://ex/x"), iri("http://ex/k"), formula(vec![[var("__ua.http://ex/x"), iri("http://ex/k"), var("x")]])],
        [var("x"), iri("http://ex/k"), formula(vec![[iri("http://ex/x"), iri("http://ex/k"), var("x_2")]])],
    ];
    let doc = serialize_facts([&noise[0], s, &noise[1]].into_iter());
    if lossy(s) {
        assert!(wrote.is_err(), "a lossy shape was written instead of refused:\n  {s:?}\n  {text}");
        assert_eq!(text, "", "a refused write wrote something");
        assert!(doc.is_err(), "a document holding a lossy statement was written");
        return None;
    }
    wrote.unwrap_or_else(|e| panic!("refused a representable statement ({e}):\n  {s:?}"));
    let back = parser::parse(&text).unwrap_or_else(|e| panic!("does not re-parse ({e}):\n{text}"));
    assert!(back.rules.is_empty() && back.facts.len() == 1, "{text}");
    let norm = s.clone().map(|t| normalise(&t));
    assert_eq!(back.facts[0], norm, "not exact:\n  wrote {s:?}\n  text  {text}");
    // Render independence: the same line inside a document of unrelated statements.
    let doc = doc.expect("noise and statement are representable");
    assert_eq!(doc.lines().nth(1).map(|l| format!("{l}\n")), Some(text.clone()), "rendering depends on neighbours:\n{doc}");
    Some(text)
}

/// N3 string-literal body for `text`.
fn n3_string(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
}

/// Against the reasoner: `log:parsedAsN3` of the written text, and `log:semantics` of it
/// as a resolved document, both yield the very formula the writer started from.
fn check_through_the_reasoner(s: &[Term; 3], text: &str) {
    let norm = s.clone().map(|t| normalise(&t));
    let src = format!(
        "{{ \"{}\" <http://www.w3.org/2000/10/swap/log#parsedAsN3> ?g }} => {{ <http://ex/got> <http://ex/is> ?g }}.\n\
         {{ <http://ex/doc> <http://www.w3.org/2000/10/swap/log#semantics> ?g }} => {{ <http://ex/sem> <http://ex/is> ?g }}.\n",
        n3_string(text)
    );
    let doc = text.to_string();
    let resolve = move |u: &str| (u == "http://ex/doc").then(|| doc.clone());
    let closure = reason_n3_terms_with_resolver(&src, None, Some(&resolve as &Resolver))
        .unwrap_or_else(|e| panic!("{e}\n{src}"))
        .facts;
    for who in ["http://ex/got", "http://ex/sem"] {
        let got = closure.iter().find(|f| f[0] == iri(who)).unwrap_or_else(|| panic!("no {who}:\n{src}"));
        assert_eq!(got[2], formula(vec![norm.clone()]), "{who} disagrees:\n{text}");
    }
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
    let (mut written, mut refused) = (0, 0);
    for (n, s) in all.iter().enumerate() {
        match check_statement(s) {
            Some(text) => {
                written += 1;
                if n % 23 == 0 {
                    check_through_the_reasoner(s, &text);
                }
            }
            None => refused += 1,
        }
    }
    assert!(written > 1000 && refused > 200, "{written} written, {refused} refused");
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

const PRE: &str = "@prefix : <http://ex/>. @prefix log: <http://www.w3.org/2000/10/swap/log#>.\n";

/// Rules that CONSUME formula-valued data: `log:conclusion` of every formula-valued fact
/// (which runs any quoted `log:implies` statement inside it), and `log:semantics` +
/// `log:conclusion` of the document itself, resolved as `<http://ex/doc>`.
const CONSUMERS: &str = "{ ?a ?b ?f. ?f log:conclusion ?c } => { ?a :concl ?c }.\n\
                         { <http://ex/doc> log:semantics ?f. ?f log:conclusion ?c } => { :doc :closure ?c }.\n";

/// `t` with every formula's rows sorted, recursively: a closure is a set, so a formula
/// built from it (`log:conclusion`) may list the same triples in another order.
fn canon(t: &Term) -> Term {
    match t {
        Term::List(ms) => Term::List(ms.iter().map(canon).collect()),
        Term::Triple(tr) => Term::Triple(Box::new(tr.clone().map(|m| canon(&m)))),
        Term::Formula(ts) => {
            let mut rows: Vec<[Term; 3]> = ts.iter().map(|r| r.clone().map(|m| canon(&m))).collect();
            rows.sort_by_key(|r| format!("{r:?}"));
            rows.dedup();
            Term::Formula(rows)
        }
        _ => t.clone(),
    }
}

/// The closure of `doc` plus [`CONSUMERS`], with `<http://ex/doc>` resolving to `doc`
/// itself — canonicalised, as a set.
fn closure(doc: &str) -> BTreeSet<String> {
    let text = doc.to_string();
    let resolve = move |u: &str| (u == "http://ex/doc").then(|| text.clone());
    let src = format!("{PRE}{doc}{CONSUMERS}");
    reason_n3_terms_with_resolver(&src, None, Some(&resolve as &Resolver))
        .unwrap_or_else(|e| panic!("{e}\n{src}"))
        .facts
        .iter()
        .map(|f| format!("{:?}", f.clone().map(|t| canon(&t))))
        .collect()
}

/// Semantic preservation of `reason_n3_pass_all`: refused (`want_refusal`), or a document
/// whose closure — directly, via `log:conclusion`, and via `log:semantics` — is the
/// source's, and which is itself a fixpoint. The `:bad :is true` marker is never derived.
fn check_document(body: &str, want_refusal: bool) {
    let src = format!("{PRE}{body}");
    let derives_bad = |c: &BTreeSet<String>| c.iter().any(|f| f.starts_with("[Iri(\"http://ex/bad\")"));
    let original = closure(&src);
    assert!(!derives_bad(&original), "the source itself derives :bad:\n{src}");
    assert!(original.iter().any(|f| f.starts_with("[Iri(\"http://ex/doc\"), Iri(\"http://ex/closure\")")), "log:semantics did not run:\n{src}");
    let consumed = format!("{src}{CONSUMERS}");
    for doc_src in [&src, &consumed] {
        match reason_n3_pass_all(doc_src, RuleVars::N3) {
            Err(e) => assert!(want_refusal, "refused a representable document ({e}):\n{doc_src}"),
            Ok(doc) => {
                assert!(!want_refusal, "a lossy document was written instead of refused:\n{doc}");
                assert!(!doc.contains("__ua") && !doc.contains("__bw"), "{doc}");
                if doc_src == &src {
                    assert_eq!(closure(&doc), original, "re-reasoning the output changed its meaning:\n{src}\n---\n{doc}");
                }
                assert_eq!(reason_n3_pass_all(&doc, RuleVars::N3).expect("round two"), doc, "not a fixpoint:\n{doc}");
            }
        }
    }
}

/// A corpus of documents — Codex's examples among them — through [`check_document`].
#[test]
fn every_re_reasonable_write_preserves_semantics_or_refuses() {
    let representable = [
        // Codex round 5 (1): body and head share a universal; the body also mentions the
        // IRI plainly, before its declaration.
        "{ :marker :ref :x. @forAll :x. :x :p :b } => { @forAll :x. :x :q :b }.\n:c :p :b. :marker :ref :x.\n",
        // Codex round 2: a formula fact compared with the rule's own formula.
        "@forAll :x.\n:a :p { :x :q :z }.\n:b :p { ?x :q :z }.\n{ :a :p ?f. ?f log:notEqualTo { :x :q :z } } => { :bad :is true }.\n",
        // Codex round 4 (1): against a formula parsed from a literal.
        "@forAll :x.\n:a :p { :x :q :z }.\n{ :a :p ?f. \"@prefix : <http://ex/>. @forAll :x. :x :q :z.\" log:parsedAsN3 ?g. ?f log:notEqualTo ?g } => { :bad :is true }.\n",
        // Codex round 6 (1): the IRI also used plainly at the top level, before the
        // declaration — the formula declares for itself.
        ":x :marker :z.\n@forAll :x.\n:a :p { :x :q :z }.\n{ :a :p ?f. \"@prefix : <http://ex/>. @forAll :x. :x :q :z.\" log:parsedAsN3 ?g. ?f log:notEqualTo ?g } => { :bad :is true }.\n",
        // A universal in the body and in a nested head formula, beside a source `?x`.
        "@forAll :x.\n{ :x :p ?x } => { :x :q { :x :r ?x } }.\n:a :p :b.\n",
        // A mid-formula declaration after a plain mention.
        ":a :p { :m :r :x. @forAll :x. :x :p :b }.\n",
        // A quoted rule in data: its body and head share the universal, and
        // `log:conclusion` runs it — one shared variable after the round trip too.
        "@forAll :x.\n:a :p { :m :q :o. { :x :q ?o } => { :x :r ?o } }.\n",
        // The same quoted rule, DERIVED (its universal reaches the data through a rule).
        "@forAll :x.\n{ :go :go :go } => { :out :has { :m :q :o. { :x :q :o } => { :x :r :o } } }.\n:go :go :go.\n",
        // A derived formula holding the universal in a nested formula and the plain IRI
        // after it, outside that formula's scope: each level is scoped on its own.
        ":x :p :o.\n@forAll :x.\n{ ?s :p ?o } => { :out :has { { :x :link :k } :k ?s } }.\n",
    ];
    for body in representable {
        check_document(body, false);
    }
    // `log:conclusion` really ran the quoted rule, binding its universal on both sides.
    let c = closure(&format!("{PRE}{}", representable[6]));
    assert!(c.iter().any(|f| f.starts_with("[Iri(\"http://ex/a\"), Iri(\"http://ex/concl\")") && f.contains("Iri(\"http://ex/m\"), Iri(\"http://ex/r\"), Iri(\"http://ex/o\")")), "{c:#?}");
    let lossy = [
        // Codex round 7: a derived formula carries the plain IRI and the universal in ONE
        // triple. Any written spelling re-parses as a second, different formula, and
        // re-reasoning then derives `:bad`.
        ":x :p :o.\n@forAll :x.\n{ ?s :p ?o } => { :out :has { ?s :link :x } }.\n{ :out :has ?f. :out :has ?g. ?f log:notEqualTo ?g } => { :bad :is true }.\n",
        // A bare top-level universal fact: no statement-local scope exists.
        "@forAll :x.\n:x :p :o.\n",
        // A quoted rule in derived data whose BODY mentions the plain IRI after the
        // universal: no placement scopes one and not the other, and declaring only the head
        // would split the variable `log:conclusion` binds across both sides.
        ":x :p :o.\n@forAll :x.\n{ ?s :p ?o } => { :out :rule { { :x :q ?o. ?s :q ?o } => { :x :r :k } } }.\n",
        // The same with the plain mention in the quoted rule's HEAD, in the universal's triple.
        ":x :p :o.\n@forAll :x.\n{ ?s :p ?o } => { :out :rule { { :x :q ?o } => { :x :r ?s } } }.\n",
    ];
    for body in lossy {
        check_document(body, true);
    }
}

/// Codex round 7, verbatim: the first pass does not derive `:bad`; a written fallback
/// would have made the second pass derive it. The writer refuses instead, and says why.
#[test]
fn a_lossy_closure_is_refused_not_written() {
    let src = "@prefix : <http://ex/>.\n:x :p :o.\n@forAll :x.\n{ ?s :p ?o } => { :out :has { ?s :link :x } }.\n\
               { :out :has ?f. :out :has ?g. ?f log:notEqualTo ?g } => { :bad :is true }.\n";
    let src = format!("@prefix log: <http://www.w3.org/2000/10/swap/log#>.\n{src}");
    let e = reason_n3_pass_all(&src, RuleVars::N3).expect_err("no lossless form exists");
    assert!(e.contains("<http://ex/x>") && e.contains("plain mention"), "{e}");
    let e = reason_n3_pass_all("@prefix : <http://ex/>. @forAll :x. :x :p :o.\n", RuleVars::N3).expect_err("bare");
    assert!(e.contains("outside every formula"), "{e}");
    // The display writer still shows such a statement, as the one function allowed to.
    let x = var("__ua.http://ex/x");
    let f = [iri("http://ex/out"), iri("http://ex/has"), formula(vec![[iri("http://ex/x"), iri("http://ex/link"), x]])];
    let mut out = String::new();
    assert!(write_statement(&f, &mut out).is_err() && out.is_empty());
    assert_eq!(
        sparq_reason::n3::serialize::statement_display_lossy(&f)[2],
        "{ <http://ex/x> <http://ex/link> ?x . }"
    );
}

/// Codex round 5 (2): a source variable spelled like a display name stays distinct — and
/// where no declaration fits, the exact writer refuses rather than renaming.
#[test]
fn a_source_variable_spelled_like_the_display_name_stays_distinct() {
    let src = "@prefix : <http://ex/>.\n:a :p { :x :q :z. @forAll :x. :x :r ?x. ?x_2 :s :x }.\n";
    let back = |doc: &str| parser::parse(doc).expect("re-parses").facts;
    let doc = reason_n3_pass_all(src, RuleVars::N3).expect("pass-all");
    assert_eq!(back(&doc), back(src), "{doc}");
    let x = var("__ua.http://ex/x");
    let f = [iri("http://ex/a"), iri("http://ex/p"), formula(vec![
        [x.clone(), iri("http://ex/x"), var("x")],
        [var("x_2"), iri("http://ex/q"), x],
    ])];
    assert_eq!(check_statement(&f), None);
    let shown = sparq_reason::n3::serialize::statement_display_lossy(&f)[2].clone();
    let vars: Vec<&str> = shown.split_whitespace().filter(|w| w.starts_with('?')).collect();
    assert_eq!(vars, ["?x_3", "?x", "?x_2", "?x_3"], "{shown}");
}

/// Codex round 7 (MEDIUM): identity keys are structural over EVERY field of a term — a
/// language-tagged literal's datatype included, noncanonical combinations included — so
/// two terms share a key exactly when they are equal.
#[test]
fn identity_keys_are_injective_over_every_term_field() {
    const LANG: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString";
    const XS: &str = "http://www.w3.org/2001/XMLSchema#string";
    let strs = ["", "hi", "a\"b", "a\\", "\"", "x"];
    let dts = [XS, LANG, "http://ex/a", "http://ex/b", ""];
    let langs = [None, Some("en"), Some(""), Some("EN")];
    let mut atoms: Vec<Term> = Vec::new();
    for s in strs {
        atoms.push(iri(s));
        atoms.push(Term::Blank(s.into()));
        atoms.push(var(s));
        for dt in dts {
            for l in langs {
                atoms.push(Term::Lit(s.into(), dt.into(), l.map(str::to_string)));
            }
        }
    }
    atoms.push(var("__ua.http://ex/x"));
    atoms.push(var("__bw0___ua.http://ex/x"));
    let few = [&atoms[0], &atoms[1], &atoms[2], &atoms[7], &atoms[8]];
    let mut all = atoms.clone();
    all.push(Term::List(vec![]));
    all.push(formula(vec![]));
    all.push(Term::List(vec![Term::List(vec![])]));
    all.push(Term::List(vec![formula(vec![])]));
    for a in few {
        all.push(Term::List(vec![a.clone()]));
        all.push(Term::Triple(Box::new([a.clone(), a.clone(), a.clone()])));
        for b in few {
            all.push(Term::List(vec![a.clone(), b.clone()]));
            all.push(formula(vec![[a.clone(), b.clone(), a.clone()]]));
            all.push(formula(vec![[a.clone(), a.clone(), a.clone()], [b.clone(), b.clone(), b.clone()]]));
            all.push(Term::Triple(Box::new([a.clone(), b.clone(), Term::List(vec![a.clone()])])));
        }
    }
    // The case Codex named: same lexical form and tag, different datatypes.
    let ha = Term::Lit("hi".into(), "http://ex/a".into(), Some("en".into()));
    let hb = Term::Lit("hi".into(), "http://ex/b".into(), Some("en".into()));
    assert!(all.contains(&ha) && all.contains(&hb));
    let k = iri("http://ex/k");
    let mut by_key: BTreeMap<[String; 3], Term> = BTreeMap::new();
    for t in &all {
        let key = statement_keys(&[k.clone(), k.clone(), t.clone()]);
        if let Some(prev) = by_key.insert(key, t.clone()) {
            assert_eq!(&prev, t, "two different terms share an identity key");
        }
    }
    let distinct: BTreeSet<String> = all.iter().map(|t| format!("{t:?}")).collect();
    assert_eq!(by_key.len(), distinct.len());
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
