//! GH #6701 review rounds 1–10: the N3 writer's contract, checked over an enumeration of
//! term shapes, a seeded random generator, and a corpus of documents rather than one
//! example per review round.
//!
//! The contract (`n3::serialize`, the `Unit` docs): every RE-REASONABLE writer is exact or
//! refuses. `parse(write(x))` is `x` exactly — a backward-chaining copy
//! `__bw<n>___ua.<iri>` reading back as the universal it copies is the one normalisation —
//! or the write returns `NotRepresentable`, which it does exactly for the shapes with no
//! lossless N3 form (checked against an independent oracle): a universal that no ONE
//! formula of its statement encloses at every occurrence (outside every formula, or in two
//! of the statement's terms), a plain mention of its IRI that its one declaration would
//! capture, or two distinct variables (a universal and its copy) anywhere in the output.
//! There is no fallback spelling.
//!
//! Semantics, not just syntax: for every document in a corpus (Codex's examples among it),
//! `reason_n3_pass_all` either refuses or writes a document whose closure — read directly,
//! through `log:conclusion` of each formula-valued fact, and through `log:semantics` +
//! `log:conclusion` of the whole document — equals the source's. On top of that: a
//! statement renders the same whatever surrounds it, distinct variables (a universal and
//! its backward-chaining copy) are never merged, and identity keys (`statement_keys`,
//! which provenance addresses facts by) are injective over every `Term` field.

use std::collections::{BTreeMap, BTreeSet};

use sparq_reason::n3::serialize::{serialize_facts, statement_keys, write_rule, write_statement, NotRepresentable};
use sparq_reason::n3::Resolver;
use sparq_reason::n3::{parser, Rule, RuleKind, Term};
use sparq_reason::n3::reason_n3_terms_with_resolver;
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

/// A position inside a statement: the index taken at each step down (term, list member,
/// quoted-triple component, formula row then column).
type Pos = Vec<usize>;

/// Every occurrence of every universal in `t` (at `pos`) → the formulae enclosing it,
/// outermost first, as (the formula's position, the row the occurrence is in).
fn occurrences(t: &Term, pos: &mut Pos, up: &mut Vec<(Pos, usize)>, out: &mut BTreeMap<String, Vec<Vec<(Pos, usize)>>>) {
    match t {
        Term::Var(v) if v.starts_with(UA) => out.entry(v[UA.len()..].to_string()).or_default().push(up.clone()),
        Term::List(ms) => {
            for (i, m) in ms.iter().enumerate() {
                pos.push(i);
                occurrences(m, pos, up, out);
                pos.pop();
            }
        }
        Term::Triple(tr) => {
            for (i, m) in tr.iter().enumerate() {
                pos.push(i);
                occurrences(m, pos, up, out);
                pos.pop();
            }
        }
        Term::Formula(ts) => {
            for (r, row) in ts.iter().enumerate() {
                up.push((pos.clone(), r));
                for (c, m) in row.iter().enumerate() {
                    pos.extend([r, c]);
                    occurrences(m, pos, up, out);
                    pos.truncate(pos.len() - 2);
                }
                up.pop();
            }
        }
        _ => {}
    }
}

/// The formula at `pos` under `terms`.
fn at<'a>(terms: &'a [Term], pos: &[usize]) -> &'a [[Term; 3]] {
    let mut t = &terms[pos[0]];
    let mut rest = &pos[1..];
    while !rest.is_empty() {
        t = match t {
            Term::List(ms) => {
                let m = &ms[rest[0]];
                rest = &rest[1..];
                m
            }
            Term::Triple(tr) => {
                let m = &tr[rest[0]];
                rest = &rest[1..];
                m
            }
            Term::Formula(ts) => {
                let m = &ts[rest[0]][rest[1]];
                rest = &rest[2..];
                m
            }
            _ => unreachable!("a position runs through structure only"),
        };
    }
    match t {
        Term::Formula(ts) => ts,
        _ => unreachable!("not a formula"),
    }
}

/// The oracle for one unit (a statement's terms, or a rule's two sides, already
/// normalised): does some universal have no single exact declaration? Its owner is the
/// deepest formula on every occurrence's path; it is declared before the first row holding
/// an occurrence, and must not capture a plain mention there or later.
fn unplaceable(terms: &[Term]) -> bool {
    let mut occ = BTreeMap::new();
    for (i, t) in terms.iter().enumerate() {
        occurrences(t, &mut vec![i], &mut Vec::new(), &mut occ);
    }
    occ.iter().any(|(iri, paths)| {
        let depth = (0..paths[0].len())
            .take_while(|&d| paths.iter().all(|p| p.get(d).map(|e| &e.0) == Some(&paths[0][d].0)))
            .count();
        if depth == 0 {
            return true;
        }
        let row = paths.iter().map(|p| p[depth - 1].1).min().unwrap();
        at(terms, &paths[0][depth - 1].0)[row..].iter().flatten().any(|m| mentions(m, iri))
    })
}

/// Does the oracle say `s` has no lossless form?
fn lossy(s: &[Term; 3]) -> bool {
    unplaceable(&s.clone().map(|t| normalise(&t))) || merges(s.iter())
}

/// Would normalising `terms` merge two distinct variables (a universal and a copy of it)?
fn merges<'a>(terms: impl Iterator<Item = &'a Term>) -> bool {
    fn vars(t: &Term, out: &mut BTreeSet<String>) {
        match t {
            Term::Var(v) => {
                out.insert(v.clone());
            }
            Term::List(ms) => ms.iter().for_each(|m| vars(m, out)),
            Term::Triple(tr) => tr.iter().for_each(|m| vars(m, out)),
            Term::Formula(ts) => ts.iter().flatten().for_each(|m| vars(m, out)),
            _ => {}
        }
    }
    let mut all = BTreeSet::new();
    terms.for_each(|t| vars(t, &mut all));
    let images: BTreeSet<String> = all.iter().map(|v| format!("{:?}", normalise(&var(v)))).collect();
    images.len() < all.len()
}

/// Check one statement: exact or refused (exactly when the oracle says lossy), and the same
/// outcome inside a document between unrelated statements. The written text, if any.
fn check_statement(s: &[Term; 3]) -> Option<String> {
    let mut text = String::new();
    let wrote = write_statement(s, &mut text);
    let noise = [
        [iri("http://noise/n"), iri("http://ex/k"), formula(vec![[var("__ua.http://noise/n"), iri("http://ex/k"), var("x")]])],
        [var("x"), iri("http://ex/k"), formula(vec![[iri("http://noise/n"), iri("http://ex/k"), var("x_2")]])],
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
    // 3. nested formulae — one declaration, in the deepest formula enclosing every use
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

const PRE: &str = "@prefix : <http://ex/>. @prefix log: <http://www.w3.org/2000/10/swap/log#>. \
                   @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>.\n";

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
        // Body and head share a universal (W3C N3 tests: 23 rules in 12 files): one
        // document-level declaration, before the rule, scopes both sides.
        "@forAll :x.\n{ :x :p :b } => { :x :q :b }.\n:c :p :b.\n",
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
        // Builtin-generated terms that DO have an exact form (Codex round 8).
        "{ ( \"hi\" \"en\" ) log:langlit ?l } => { :s :p ?l }.\n",
        "{ ( \"1\" <http://ex/d> ) log:dtlit ?l } => { :s :p ?l }.\n",
        "{ ( \"x\" rdf:langString ) log:dtlit ?l } => { :s :p ?l }.\n",
        "{ ?u log:uri \"http://ex/a\\\\b\" } => { :s :p ?u }.\n",
    ];
    for body in representable {
        check_document(body, false);
    }
    // `log:conclusion` really ran the quoted rule, binding its universal on both sides.
    let c = closure(&format!("{PRE}{}", representable[6]));
    let shared = reason_n3_pass_all(&format!("{PRE}{}", representable[0]), RuleVars::N3).expect("pass-all");
    assert_eq!(shared.matches("@forAll").count(), 1, "{shared}");
    assert!(c.iter().any(|f| f.starts_with("[Iri(\"http://ex/a\"), Iri(\"http://ex/concl\")") && f.contains("Iri(\"http://ex/m\"), Iri(\"http://ex/r\"), Iri(\"http://ex/o\")")), "{c:#?}");
    let lossy = [
        // Codex round 5 (1): body and head share a universal, AND the body mentions the IRI
        // plainly. Only a document-level declaration scopes both sides, and it would capture
        // the plain mention (round 10: separate per-side declarations are two quantifiers).
        "{ :marker :ref :x. @forAll :x. :x :p :b } => { @forAll :x. :x :q :b }.\n:c :p :b. :marker :ref :x.\n",
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
        // Codex round 8: builtin-generated terms the PARSER normalizes. An uppercase tag
        // (`"hi"@EN` reads back as `@en`) — with a `:bad` probe comparing it to `@en`.
        "{ ( \"hi\" \"EN\" ) log:langlit ?l } => { :s :p ?l }.\n\
         { :s :p ?a. :s :p ?b. ?a log:notEqualTo ?b } => { :bad :is true }.\n",
        // `rdf:nil` as an IRI reads back as the empty list `()`.
        "{ ?u log:uri \"http://www.w3.org/1999/02/22-rdf-syntax-ns#nil\" } => { :s :p ?u }.\n",
        // An IRI and a language tag the grammar rejects.
        "{ ?u log:uri \"http://ex/a b\" } => { :s :p ?u }.\n",
        "{ ( \"hi\" \"e n\" ) log:langlit ?l } => { :s :p ?l }.\n",
        // A DERIVED `{ … } log:implies { … }` fact: N3 text spells it as a rule, which
        // would then fire.
        "{ :go :go :go } => { { :a :b :c } log:implies { :d :e :f } }.\n:go :go :go.\n:a :b :c.\n",
    ];
    for body in lossy {
        check_document(body, true);
    }
    // The normalized terms really are in the source closure (else the refusals prove nothing).
    let c = closure(&format!("{PRE}{}", lossy[5]));
    assert!(c.iter().any(|f| f.contains("Some(\"EN\")")), "{c:#?}");
    let c = closure(&format!("{PRE}{}", lossy[6]));
    assert!(c.iter().any(|f| f.contains("Iri(\"http://www.w3.org/1999/02/22-rdf-syntax-ns#nil\")")), "{c:#?}");
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
/// two terms share a key exactly when they are equal (`Term`'s own `Eq`).
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
    // Equal keys exactly for equal terms.
    let distinct: BTreeSet<String> = all.iter().map(|t| format!("{t:?}")).collect();
    assert_eq!(by_key.len(), distinct.len());
}

/// A random term for the key property: IRIs, literals (with and without tags, awkward
/// characters), blanks, variables (a universal and its copy among them), and — below
/// `depth` — lists, quoted triples and formulae, each formula also yielding variants with
/// its rows PERMUTED and with a row DUPLICATED.
fn key_term(r: &mut Rng, depth: usize, out: &mut Vec<Term>) -> Term {
    let strs = ["a", "b", "a\"b", "a\\", ""];
    let s = |r: &mut Rng| strs[r.below(strs.len())].to_string();
    let t = match (depth, r.below(10)) {
        (0, _) | (_, 0..=5) => match r.below(6) {
            0 => Term::Iri(s(r)),
            1 => Term::Lit(s(r), ["http://ex/d", "http://ex/e"][r.below(2)].into(), None),
            2 => Term::Lit(s(r), "http://ex/d".into(), [Some("en".to_string()), Some("EN".into()), None][r.below(3)].clone()),
            3 => Term::Blank(s(r)),
            4 => var(["x", "__ua.http://ex/x", "__bw0___ua.http://ex/x"][r.below(3)]),
            _ => var(&s(r)),
        },
        (_, 6) => Term::List((0..r.below(3)).map(|_| key_term(r, depth - 1, out)).collect()),
        (_, 7) => Term::Triple(Box::new([key_term(r, depth - 1, out), iri("http://ex/p"), key_term(r, depth - 1, out)])),
        _ => {
            let rows: Vec<[Term; 3]> = (0..1 + r.below(3))
                .map(|_| [key_term(r, depth - 1, out), iri("http://ex/p"), key_term(r, depth - 1, out)])
                .collect();
            // Variants the engine keeps APART from `rows` (unless they happen to be equal).
            let mut permuted = rows.clone();
            permuted.rotate_left(1);
            permuted.reverse();
            out.push(formula(permuted));
            let mut duplicated = rows.clone();
            duplicated.push(rows[r.below(rows.len())].clone());
            out.push(formula(duplicated));
            formula(rows)
        }
    };
    out.push(t.clone());
    t
}

/// #6735 review round 2: the identity key mirrors the engine's own term identity EXACTLY —
/// `key(a) == key(b)` if and only if `a == b` under `Term`'s derived `Eq`, which is what
/// facts, hashing, `log:equalTo` and formula unification use. So a formula's row order and
/// duplicate rows are part of its key, as they are part of the term; no normalisation the
/// engine does not do. Seeded generator (xorshift64*, like the round-10 property test).
#[test]
fn identity_keys_are_equal_exactly_when_terms_are_equal() {
    let mut r = Rng(0xD1B5_4A32_D192_ED03);
    let k = iri("http://ex/k");
    let mut by_key: std::collections::HashMap<[String; 3], Term> = std::collections::HashMap::new();
    let mut terms: std::collections::HashSet<Term> = std::collections::HashSet::new();
    let (mut formulas, mut collisions_checked) = (0, 0);
    for _ in 0..4000 {
        let mut pool = Vec::new();
        key_term(&mut r, 3, &mut pool);
        for t in pool {
            formulas += usize::from(matches!(t, Term::Formula(_)));
            let key = statement_keys(&[k.clone(), k.clone(), t.clone()]);
            if let Some(prev) = by_key.get(&key) {
                assert_eq!(prev, &t, "different terms share a key");
                collisions_checked += 1;
            }
            by_key.insert(key, t.clone());
            terms.insert(t);
        }
    }
    // Every distinct term got a distinct key, and equal terms (re-generated) the same one.
    assert_eq!(by_key.len(), terms.len());
    assert!(formulas > 1000 && collisions_checked > 1000, "{formulas} formulae, {collisions_checked} equal pairs");
    // The named cases, explicitly: permuted and duplicated rows are different facts.
    let row = |a: &str| [iri(a), k.clone(), k.clone()];
    let key = |t: Term| statement_keys(&[k.clone(), k.clone(), t]);
    assert_ne!(key(formula(vec![row("1"), row("2")])), key(formula(vec![row("2"), row("1")])));
    assert_ne!(key(formula(vec![row("1")])), key(formula(vec![row("1"), row("1")])));
    assert_ne!(formula(vec![row("1"), row("2")]), formula(vec![row("2"), row("1")]), "the engine keeps them apart too");
}

/// #6735 review round 2: the one engine producer that built formula rows from hash-set
/// iteration — `log:conclusion`, whose derived rows came out of the closure's hash set —
/// now emits them in ONE canonical order (their identity keys), after the formula's own
/// rows. So the formula value, and its key, do not depend on hash-set order (or the
/// hasher's word size on wasm32).
#[test]
fn log_conclusion_rows_come_out_in_canonical_order() {
    let src = "@prefix : <http://ex/>. @prefix log: <http://www.w3.org/2000/10/swap/log#>.\n\
               { { :e :p :f. :a :p :b. :c :p :d. :g :p :h. { ?x :p ?y } => { ?y :q ?x } } log:conclusion ?c } => { :r :is ?c }.\n\
               :go :go :go.\n";
    let facts = reason_n3_terms_with_resolver(src, None, None).expect("reasons").facts;
    let c = facts.iter().find(|f| f[0] == iri("http://ex/r")).expect("log:conclusion ran");
    let Term::Formula(rows) = &c[2] else { panic!("{c:?}") };
    let q = iri("http://ex/q");
    let derived: Vec<&[Term; 3]> = rows.iter().filter(|r| r[1] == q).collect();
    assert_eq!(derived.len(), 4, "{rows:?}");
    let keys: Vec<[String; 3]> = derived.iter().map(|r| statement_keys(r)).collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted, "derived rows in canonical key order");
    // The formula's own rows come first, as written.
    assert_eq!(rows[0], [iri("http://ex/e"), iri("http://ex/p"), iri("http://ex/f")]);
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

/// Codex round 9: a universal and its backward-chaining copy — or two copies — are
/// DISTINCT variables, and N3 text can spell each only as the universal's IRI. Writing them
/// would merge them, so every exact writer refuses; neither may ever come back as one.
#[test]
fn a_universal_and_its_copies_are_never_merged() {
    let (ua, c0, c1) = (var("__ua.http://ex/x"), var("__bw0___ua.http://ex/x"), var("__bw1___ua.http://ex/x"));
    let k = iri("http://ex/k");
    for (a, b) in [(&ua, &c0), (&c0, &c1), (&c1, &ua)] {
        let f = [k.clone(), k.clone(), formula(vec![[a.clone(), k.clone(), b.clone()]])];
        let mut out = String::new();
        let e = write_statement(&f, &mut out).expect_err("would merge two variables");
        assert!(out.is_empty() && e.to_string().contains("merge"), "{e}");
        // Also across formulae of one statement (a variable spans the statement).
        let g = [formula(vec![[a.clone(), k.clone(), k.clone()]]), k.clone(), formula(vec![[b.clone(), k.clone(), k.clone()]])];
        assert!(write_statement(&g, &mut out).is_err() && out.is_empty());
        assert!(check_statement(&f).is_none());
    }
    // One copy on its own is fine: it reads back as the universal, merging nothing.
    let lone = [k.clone(), k.clone(), formula(vec![[c0.clone(), k.clone(), k.clone()]])];
    assert!(check_statement(&lone).is_some());
}

/// `src`'s one rule, its written text, and that text re-parsed.
fn one_rule(src: &str) -> Rule {
    let p = parser::parse(src).unwrap_or_else(|e| panic!("{e}\n{src}"));
    assert_eq!(p.rules.len(), 1, "{src}");
    p.rules[0].clone()
}

/// Rules as comparable [premise, conclusion] formula pairs.
fn parts(rules: &[Rule]) -> Vec<[Term; 2]> {
    rules.iter().map(|r| [formula(r.premise.clone()), formula(r.conclusion.clone())]).collect()
}

/// The `@forAll` declarations in `text`.
fn decls(text: &str) -> usize {
    text.matches("@forAll").count()
}

/// GH #6701 review round 10: quantifier SCOPE is part of what must round-trip. Each row of
/// the table is written with exactly one `@forAll` per universal, at the one scope that
/// owns every occurrence, or refused with the dedicated error — never a per-formula
/// redeclaration that would read back as a different quantifier structure.
#[test]
fn every_universal_gets_one_declaration_at_its_owning_scope_or_is_refused() {
    let p = "@prefix : <http://ex/>.\n";
    let facts = |src: &str| parser::parse(&format!("{p}{src}")).unwrap_or_else(|e| panic!("{e}\n{src}")).facts;
    let scope_error = |r: Result<String, NotRepresentable>| match r {
        Err(NotRepresentable::UnrepresentableScope(_)) => {}
        other => panic!("expected UnrepresentableScope, got {other:?}"),
    };
    // Written: (source, the declaration count, a fragment of the text).
    let written = [
        // Positive control: a universal in one formula.
        (":a :p { @forAll :x. :x :q :o }.", 1, "{ @forAll <http://ex/x> . <http://ex/x> <http://ex/q> <http://ex/o> . }"),
        // Nested shadowing (the parser reads both declarations as the same universal): ONE
        // declaration, in the outer formula; the inner one does not redeclare.
        (":a :p { @forAll :x. :x :q { @forAll :x. :x :r :o } }.", 1, "{ @forAll <http://ex/x> . <http://ex/x> <http://ex/q> { <http://ex/x>"),
        // Sibling formulae inside one formula: declared once, in the formula holding both.
        (":a :p { :s :q { @forAll :x. :x :q :o }. :s :r { @forAll :x. :x :r :o } }.", 1, "{ @forAll <http://ex/x> . <http://ex/s> <http://ex/q> {"),
        // Only the inner formula uses it, after a plain mention in the outer one: declared
        // in the inner formula, which the plain mention is outside of.
        (":a :p { :m :r :x. :s :q { @forAll :x. :x :q :o } }.", 1, "{ <http://ex/m> <http://ex/r> <http://ex/x> . <http://ex/s> <http://ex/q> { @forAll"),
    ];
    for (src, n, frag) in written {
        let f = facts(src);
        let text = serialize_facts(f.iter()).unwrap_or_else(|e| panic!("{e}\n{src}"));
        assert_eq!(decls(&text), n, "{src}\n{text}");
        assert!(text.contains(frag), "{src}\n{text}");
        assert_eq!(parser::parse(&text).expect("re-parses").facts, f, "{text}");
    }
    // Refused, UnrepresentableScope: sibling formulae as two terms of one statement (no
    // formula of the statement owns both), a universal outside every formula, and a plain
    // mention inside the owner after the declaration point.
    for src in [
        "{ @forAll :x. :x :q :o } :p { @forAll :x. :x :r :o }.",
        ":a :p ( { @forAll :x. :x :q :o } { @forAll :x. :x :r :o } ).",
        "@forAll :x. :x :p :o.",
    ] {
        let f = facts(src);
        scope_error(serialize_facts(f.iter()));
        let mut out = String::new();
        assert!(write_statement(&f[0], &mut out).is_err() && out.is_empty(), "{src}");
    }

    // A plain mention of the IRI inside the owning formula, after the declaration point
    // (built as terms: the parser itself reads every later `:x` there as the universal).
    let k = iri("http://ex/k");
    let captured = [k.clone(), k.clone(), formula(vec![[var("__ua.http://ex/x"), k.clone(), formula(vec![[k.clone(), k.clone(), iri("http://ex/x")]])]])];
    scope_error(serialize_facts([&captured].into_iter()));

    // A rule whose body and head share a universal: the whole document puts ONE
    // declaration before it; the rule written on its own has nowhere to put it.
    let rule = "@forAll :x. { :x :p :b } => { :x :q :b }.";
    let doc = reason_n3_pass_all(&format!("{p}{rule}\n:a :p :b.\n"), RuleVars::N3).expect("pass-all");
    assert_eq!(decls(&doc), 1, "{doc}");
    let back = parser::parse(&doc).expect("re-parses");
    assert_eq!(parts(&back.rules), parts(&[one_rule(&format!("{p}{rule}"))]), "{doc}");
    let mut out = String::new();
    match write_rule(&one_rule(&format!("{p}{rule}")), RuleKind::Forward, RuleVars::N3, &mut out) {
        Err(NotRepresentable::UnrepresentableScope(_)) => assert!(out.is_empty()),
        other => panic!("expected UnrepresentableScope, got {other:?}"),
    }
    // … while a rule whose universal stays on one side is its own unit.
    let one_side = one_rule(&format!("{p}{{ ?s :p :b }} => {{ @forAll :x. :x :q ?s }}."));
    write_rule(&one_side, RuleKind::Forward, RuleVars::N3, &mut out).expect("one side");
    assert_eq!(parts(&parser::parse(&out).expect("re-parses").rules), parts(&[one_side]), "{out}");

    // Two FACTS holding distinct backward-chaining copies: each is writable alone, but the
    // document would spell both as <http://ex/x> — one bijection over the whole document
    // refuses the merge (round 10, MEDIUM).
    let fact = |v: &str| [k.clone(), k.clone(), formula(vec![[var(v), k.clone(), k.clone()]])];
    let (f0, f1) = (fact("__bw0___ua.http://ex/x"), fact("__bw1___ua.http://ex/x"));
    assert!(serialize_facts([&f0].into_iter()).is_ok() && serialize_facts([&f1].into_iter()).is_ok());
    match serialize_facts([&f0, &f1].into_iter()) {
        Err(NotRepresentable::MergesVariables(_)) => {}
        other => panic!("expected MergesVariables, got {other:?}"),
    }
    let fu = fact("__ua.http://ex/x");
    assert!(matches!(serialize_facts([&fu, &f1].into_iter()), Err(NotRepresentable::MergesVariables(_))));
    // The same universal in two facts is one variable, and writes fine.
    assert!(serialize_facts([&fu, &fu.clone()].into_iter()).is_ok());
}

/// Deterministic xorshift64* RNG — no dev-dependency needed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// A random term: the universals `x` and `y` (and copies of `x`), the plain IRI `x`, a
/// source variable, constants, blank nodes, and — below `depth` — formulae (1–3 rows),
/// lists and quoted triples, so the same universal lands in nested, sibling and unrelated
/// formulae at random.
fn random_term(r: &mut Rng, depth: usize, blanks: bool) -> Term {
    let leaves = [
        var("__ua.http://ex/x"),
        var("__ua.http://ex/y"),
        var("__bw0___ua.http://ex/x"),
        iri("http://ex/x"),
        var("v"),
        iri("http://ex/k"),
        Term::Lit("l".into(), "http://ex/d".into(), None),
        Term::Blank("b".into()),
    ];
    let n = leaves.len() - usize::from(!blanks);
    match (depth, r.below(10)) {
        (0, _) | (_, 0..=4) => leaves[r.below(n)].clone(),
        (_, 5..=7) => formula((0..1 + r.below(3)).map(|_| random_row(r, depth - 1, blanks)).collect()),
        (_, 8) => Term::List((0..r.below(3)).map(|_| random_term(r, depth - 1, blanks)).collect()),
        _ => Term::Triple(Box::new(random_row(r, depth - 1, blanks))),
    }
}

fn random_row(r: &mut Rng, depth: usize, blanks: bool) -> [Term; 3] {
    [random_term(r, depth, blanks), iri("http://ex/p"), random_term(r, depth, blanks)]
}

/// GH #6701 review round 10, as a property: random documents of 1–3 statements, and random
/// rules, with universals and blank nodes at random scopes (the same universal in sibling
/// and nested formulae, the same variable reused across statements). Every write is exact
/// — the text re-parses to the very same terms — or refused, exactly when the independent
/// oracle says no single declaration can place a universal or two variables would merge.
/// Nothing ever changes silently.
#[test]
fn random_scopes_round_trip_exactly_or_are_refused() {
    let mut r = Rng(0x9E37_79B9_7F4A_7C15);
    let (mut wrote, mut refused) = (0, 0);
    let (mut rules_wrote, mut rules_refused) = (0, 0);
    for _ in 0..3000 {
        let doc: Vec<[Term; 3]> = (0..1 + r.below(3)).map(|_| random_row(&mut r, 3, true)).collect();
        let want_refusal = doc.iter().any(lossy) || merges(doc.iter().flatten());
        match serialize_facts(doc.iter()) {
            Ok(text) => {
                assert!(!want_refusal, "written although the oracle refuses:\n{doc:?}\n{text}");
                let back = parser::parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
                let norm: Vec<[Term; 3]> = doc.iter().map(|f| f.clone().map(|t| normalise(&t))).collect();
                assert!(back.rules.is_empty() && back.backward_rules.is_empty(), "{text}");
                assert_eq!(back.facts, norm, "changed silently:\n{text}");
                wrote += 1;
            }
            Err(e) => {
                // A formula related by log:implies is not generated; an empty list is
                // `()`, which is exact. So the oracle's reasons are the only ones.
                assert!(want_refusal, "refused a representable document ({e}):\n{doc:?}");
                refused += 1;
            }
        }
        // A rule from two random formulae (no blanks: a premise blank is a rule variable).
        let side = |r: &mut Rng| (0..1 + r.below(2)).map(|_| random_row(r, 2, false)).collect::<Vec<_>>();
        let rule = Rule { premise: side(&mut r), conclusion: side(&mut r) };
        let sides = [formula(rule.premise.clone()), formula(rule.conclusion.clone())];
        let n = sides.clone().map(|t| normalise(&t));
        let want_refusal = unplaceable(&n) || merges(sides.iter());
        let mut out = String::new();
        match write_rule(&rule, RuleKind::Forward, RuleVars::N3, &mut out) {
            Ok(()) => {
                assert!(!want_refusal, "rule written although the oracle refuses:\n{rule:?}\n{out}");
                let back = parser::parse(&out).unwrap_or_else(|e| panic!("{e}\n{out}"));
                assert_eq!(back.rules.len(), 1, "{out}");
                let got = [formula(back.rules[0].premise.clone()), formula(back.rules[0].conclusion.clone())];
                assert_eq!(got, n, "rule changed silently:\n{out}");
                rules_wrote += 1;
            }
            Err(e) => {
                assert!(want_refusal && out.is_empty(), "refused a representable rule ({e}):\n{rule:?}");
                rules_refused += 1;
            }
        }
    }
    assert!(wrote > 300 && refused > 300, "{wrote} written, {refused} refused");
    assert!(rules_wrote > 100 && rules_refused > 300, "{rules_wrote} rules written, {rules_refused} refused");
}
