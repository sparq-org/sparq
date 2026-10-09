//! GH #6701 review rounds 1–10: the N3 writer's contract, checked over an enumeration of
//! term shapes, a seeded random generator, and a corpus of documents rather than one
//! example per review round.
//!
//! The contract (`n3::serialize`, the `Unit` docs): every RE-REASONABLE writer is exact or
//! refuses. `parse(write(x))` is `x` exactly — a backward-chaining copy
//! `__bw<n>___ua.<iri>` reading back as the universal it copies is the one normalisation —
//! or the write returns `NotRepresentable`, which it does exactly for the shapes with no
//! lossless N3 form (checked against an independent oracle). The engine reads every
//! `@forAll` of one IRI as ONE variable, `__ua.<iri>` (formula-level declarations are
//! treated as document-scoped: GH #6754), so the writer declares each universal once, at
//! document level, and refuses a plain mention of its IRI that the declaration would
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

/// The IRI of an `@forAll` universal's variable name.
fn universal_of(name: &str) -> Option<&str> {
    name.strip_prefix(UA)
}

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
/// `IRIREF` escaping, an unrelated constant, and a universal from another namespace with
/// the same local name.
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

/// Every universal (by IRI) and every plain IRI in `t`.
fn names(t: &Term, universals: &mut BTreeSet<String>, plain: &mut BTreeSet<String>) {
    match t {
        Term::Var(v) => {
            if let Some(i) = universal_of(v) {
                universals.insert(i.to_string());
            }
        }
        Term::Iri(i) => {
            plain.insert(i.clone());
        }
        Term::List(ms) => ms.iter().for_each(|m| names(m, universals, plain)),
        Term::Triple(tr) => tr.iter().for_each(|m| names(m, universals, plain)),
        Term::Formula(ts) => ts.iter().flatten().for_each(|m| names(m, universals, plain)),
        _ => {}
    }
}

/// The oracle for one unit (a statement's terms, already normalised): every universal is
/// declared once at document level, before the unit (only when `document` allows one), so
/// a plain mention of its IRI anywhere in the unit would be captured. Does some universal
/// fail that?
fn unplaceable(terms: &[Term], document: bool) -> bool {
    let (mut universals, mut plain) = (BTreeSet::new(), BTreeSet::new());
    terms.iter().for_each(|t| names(t, &mut universals, &mut plain));
    !universals.is_empty() && (!document || universals.intersection(&plain).next().is_some())
}

/// Does the oracle say `s` has no lossless form? A backward-chaining copy of a universal
/// never has one: its only spelling reads back as the universal, a different term
/// (round 13).
fn lossy(s: &[Term; 3]) -> bool {
    let norm = s.clone().map(|t| normalise(&t));
    unplaceable(&norm, true) || merges(s.iter()) || norm != *s
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
        [iri("http://noise/n"), iri("http://ex/k"), formula(vec![[var("y"), iri("http://ex/k"), var("x")]])],
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
    // Render independence: the same lines inside a document of unrelated statements.
    let doc = doc.expect("noise and statement are representable");
    let lines: Vec<&str> = doc.lines().collect();
    let middle: String = lines[1..lines.len() - 1].iter().map(|l| format!("{l}\n")).collect();
    assert_eq!(middle, text, "rendering depends on neighbours:\n{doc}");
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
    check_document_with(body, want_refusal, want_refusal)
}

/// [`check_document`], with the verdict for the document plus [`CONSUMERS`] given apart.
fn check_document_with(body: &str, want_refusal: bool, consumed_refusal: bool) {
    let src = format!("{PRE}{body}");
    let derives_bad = |c: &BTreeSet<String>| c.iter().any(|f| f.starts_with("[Iri(\"http://ex/bad\")"));
    let original = closure(&src);
    assert!(!derives_bad(&original), "the source itself derives :bad:\n{src}");
    assert!(original.iter().any(|f| f.starts_with("[Iri(\"http://ex/doc\"), Iri(\"http://ex/closure\")")), "log:semantics did not run:\n{src}");
    let consumed = format!("{src}{CONSUMERS}");
    for doc_src in [&src, &consumed] {
        let want_refusal = if doc_src == &src { want_refusal } else { consumed_refusal };
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
        // A bare top-level universal fact: its binder is the document, which declares it
        // (round 11, MEDIUM).
        "@forAll :x.\n:x :p :o.\n",
        // A quoted rule in data: its body and head share the universal, and
        // `log:conclusion` runs it — one shared variable after the round trip too.
        "@forAll :x.\n:a :p { :m :q :o. { :x :q ?o } => { :x :r ?o } }.\n",
        // The same quoted rule, DERIVED (its universal reaches the data through a rule).
        "@forAll :x.\n{ :go :go :go } => { :out :has { :m :q :o. { :x :q :o } => { :x :r :o } } }.\n:go :go :go.\n",
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
        // Codex round 5 (1): the premise and the conclusion each declare `:x` — one
        // variable in the engine (GH #6754) — and the premise mentions `:x` plainly before
        // its declaration: the one document line would capture that mention.
        "{ :marker :ref :x. @forAll :x. :x :p :b } => { @forAll :x. :x :q :b }.\n:c :p :b. :marker :ref :x.\n",
        // A mid-formula declaration after a plain mention in the same formula: likewise.
        ":a :p { :m :r :x. @forAll :x. :x :p :b }.\n",
        // A derived formula holding the universal and, after it, the plain IRI: the
        // document's declaration would capture the plain mention.
        ":x :p :o.\n@forAll :x.\n{ ?s :p ?o } => { :out :has { { :x :link :k } :k ?s } }.\n",
        // Codex round 7: a derived formula carries the plain IRI and the universal in ONE
        // triple. Any written spelling re-parses as a second, different formula, and
        // re-reasoning then derives `:bad`.
        ":x :p :o.\n@forAll :x.\n{ ?s :p ?o } => { :out :has { ?s :link :x } }.\n{ :out :has ?f. :out :has ?g. ?f log:notEqualTo ?g } => { :bad :is true }.\n",
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
    let c = closure(&format!("{PRE}{}", lossy[6]));
    assert!(c.iter().any(|f| f.contains("Some(\"EN\")")), "{c:#?}");
    let c = closure(&format!("{PRE}{}", lossy[7]));
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
    // A bare top-level universal fact is NOT lossy any more (GH #6701 round 11): its binder
    // is the document, and the document declares it.
    let src = "@prefix : <http://ex/>. @forAll :x. :x :p :o.\n";
    let doc = reason_n3_pass_all(src, RuleVars::N3).expect("a document-level binder is written");
    assert_eq!(doc, "@forAll <http://ex/x> .\n<http://ex/x> <http://ex/p> <http://ex/o> .\n");
    assert_eq!(parser::parse(&doc).unwrap().facts, parser::parse(src).unwrap().facts);
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
    let src = "@prefix : <http://ex/>.\n:a :p { @forAll :x. :x :r ?x. ?x_2 :s :x }.\n";
    let back = |doc: &str| parser::parse(doc).expect("re-parses").facts;
    let doc = reason_n3_pass_all(src, RuleVars::N3).expect("pass-all");
    assert_eq!(back(&doc), back(src), "{doc}");
    // A plain mention before the declaration: the document line would capture it.
    assert!(reason_n3_pass_all("@prefix : <http://ex/>.\n:a :p { :x :q :z. @forAll :x. :x :r ?x }.\n", RuleVars::N3).is_err());
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
    // One copy on its own is refused too (round 13): it would read back as the universal,
    // which `log:equalTo` tells apart from the copy.
    let lone = [k.clone(), k.clone(), formula(vec![[c0.clone(), k.clone(), k.clone()]])];
    assert!(check_statement(&lone).is_none());
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

/// GH #6701 review rounds 10–12: the engine reads every `@forAll` of one IRI as ONE
/// variable (formula-level declarations are treated as document-scoped: GH #6754), so the
/// writer declares each universal ONCE, on a document line before its first use — whatever
/// scope(s) declared it in the source — or refuses with the dedicated error.
#[test]
fn every_universal_gets_one_document_declaration_or_is_refused() {
    let p = "@prefix : <http://ex/>.\n";
    let facts = |src: &str| parser::parse(&format!("{p}{src}")).unwrap_or_else(|e| panic!("{e}\n{src}")).facts;
    let scope_error = |r: Result<String, NotRepresentable>| match r {
        Err(NotRepresentable::UnrepresentableScope(_)) => {}
        other => panic!("expected UnrepresentableScope, got {other:?}"),
    };
    let x = "<http://ex/x>";
    let written = [
        // A universal in one formula.
        (":a :p { @forAll :x. :x :q :o }.", format!("{{ {x} <http://ex/q> <http://ex/o> . }}")),
        // Nested and sibling declarations of one IRI: ONE variable in the engine, so one line.
        (":a :p { @forAll :x. :x :q { @forAll :x. :x :r :o } }.", format!("{{ {x} <http://ex/q> {{ {x} <http://ex/r>")),
        (":a :p { :s :q { @forAll :x. :x :q :o }. :s :r { @forAll :x. :x :r :o } }.", format!("<http://ex/s> <http://ex/q> {{ {x}")),
        ("{ @forAll :x. :x :q :o } :p { @forAll :x. :x :r :o }.", format!("{{ {x} <http://ex/q> <http://ex/o> . }} <http://ex/p> {{ {x}")),
        (":a :p ( { @forAll :x. :x :q :o } { @forAll :x. :x :r :o } ).", format!("( {{ {x}")),
        (":a :p { @forAll :x. :s :q { :x :q :o } }.", format!("{{ <http://ex/s> <http://ex/q> {{ {x} <http://ex/q>")),
        // A top-level fact.
        ("@forAll :x. :x :p :o.", format!("{x} <http://ex/p> <http://ex/o> .")),
    ];
    for (src, frag) in written {
        let f = facts(src);
        let text = serialize_facts(f.iter()).unwrap_or_else(|e| panic!("{e}\n{src}"));
        assert_eq!(decls(&text), 1, "{src}\n{text}");
        assert!(text.starts_with(&format!("@forAll {x} .\n")) && text.contains(&frag), "{src}\n{text}");
        assert_eq!(parser::parse(&text).expect("re-parses").facts, f, "{text}");
    }
    // Refused, UnrepresentableScope: a lone term has no document to declare in; a plain
    // mention of the IRI in the unit (inside or outside the declaring formula) would be
    // captured by the document line.
    let k = iri("http://ex/k");
    let mut out = String::new();
    assert!(matches!(
        sparq_reason::n3::serialize::write_term(&var("__ua.http://ex/x"), &mut out),
        Err(NotRepresentable::UnrepresentableScope(_))
    ));
    for src in [":a :p { :m :r :x. :s :q { @forAll :x. :x :q :o } }.", ":a :p { :m :r :x. @forAll :x. :x :p :b }."] {
        scope_error(serialize_facts(facts(src).iter()));
    }
    let captured = [k.clone(), k.clone(), formula(vec![[var("__ua.http://ex/x"), k.clone(), formula(vec![[k.clone(), k.clone(), iri("http://ex/x")]])]])];
    scope_error(serialize_facts([&captured].into_iter()));
    // A plain mention in an EARLIER fact is not captured: the line comes after it.
    let plain = [iri("http://ex/x"), k.clone(), k.clone()];
    let fu = [k.clone(), k.clone(), formula(vec![[var("__ua.http://ex/x"), k.clone(), k.clone()]])];
    let text = serialize_facts([&plain, &fu].into_iter()).expect("declared after the plain fact");
    assert_eq!(parser::parse(&text).unwrap().facts, vec![plain.clone(), fu.clone()], "{text}");
    scope_error(serialize_facts([&fu, &plain].into_iter()));
    assert!(out.is_empty());

    // A rule whose body and head share a universal: the whole document puts ONE
    // declaration before it; a rule written on its own has nowhere to put one.
    let rule = "@forAll :x. { :x :p :b } => { :x :q :b }.";
    let doc = reason_n3_pass_all(&format!("{p}{rule}\n:a :p :b.\n"), RuleVars::N3).expect("pass-all");
    assert_eq!(decls(&doc), 1, "{doc}");
    let back = parser::parse(&doc).expect("re-parses");
    assert_eq!(parts(&back.rules), parts(&[one_rule(&format!("{p}{rule}"))]), "{doc}");
    for r in [rule.to_string(), "{ ?s :p :b } => { @forAll :x. :x :q ?s }.".to_string()] {
        let mut out = String::new();
        match write_rule(&one_rule(&format!("{p}{r}")), RuleKind::Forward, RuleVars::N3, &mut out) {
            Err(NotRepresentable::UnrepresentableScope(_)) => assert!(out.is_empty()),
            other => panic!("expected UnrepresentableScope, got {other:?}"),
        }
    }

    // Backward-chaining copies: refused alone (round 13: `Unspellable`), and two distinct
    // ones in two FACTS would spell as one <http://ex/x> — the document-wide check refuses
    // that merge first (round 10, MEDIUM).
    let fact = |v: &str| [k.clone(), k.clone(), formula(vec![[var(v), k.clone(), k.clone()]])];
    let (f0, f1) = (fact("__bw0___ua.http://ex/x"), fact("__bw1___ua.http://ex/x"));
    for f in [&f0, &f1] {
        assert!(matches!(serialize_facts([f].into_iter()), Err(NotRepresentable::Unspellable(_))));
    }
    match serialize_facts([&f0, &f1].into_iter()) {
        Err(NotRepresentable::MergesVariables(_)) => {}
        other => panic!("expected MergesVariables, got {other:?}"),
    }
    assert!(matches!(serialize_facts([&fu, &f1].into_iter()), Err(NotRepresentable::MergesVariables(_))));
    // The same universal in two facts is one variable, and writes fine.
    assert!(serialize_facts([&fu, &fu.clone()].into_iter()).is_ok());
}

/// GH #6701 review round 13 (MEDIUM), Codex's example: sorting `:a` before `:z` would put
/// the `@forAll :y` line before `:z`'s plain `:y`. The document is ordered so every plain
/// mention precedes its universal's line; only a unit mixing the two, or a cycle, refuses.
#[test]
fn units_are_ordered_so_no_declaration_captures_a_plain_mention() {
    let p = "@prefix : <http://ex/>.\n";
    let src = format!("{p}@forAll :x.\n:z :p {{ :x :q :y }}.\n@forAll :y.\n:a :p {{ :y :q :o }}.\n");
    let doc = reason_n3_pass_all(&src, RuleVars::N3).expect("representable");
    assert_eq!(parser::parse(&doc).unwrap().facts.len(), 2, "{doc}");
    let want: BTreeSet<_> = parser::parse(&src).unwrap().facts.into_iter().map(|f| format!("{f:?}")).collect();
    let got: BTreeSet<_> = parser::parse(&doc).unwrap().facts.into_iter().map(|f| format!("{f:?}")).collect();
    assert_eq!(got, want, "{doc}");
    let (z, y) = (doc.find("<http://ex/z>").expect(":z"), doc.find("@forAll <http://ex/y>").expect(":y line"));
    assert!(z < y, "the plain :y in :z's fact comes before the :y line:\n{doc}");
    // A cycle: `:x` plain beside universal `:y`, `:y` plain beside universal `:x`.
    let k = iri("http://ex/k");
    let f = |u: &str, plain: &str| [iri(plain), k.clone(), formula(vec![[var(&format!("{UA}{u}")), k.clone(), k.clone()]])];
    let (a, b) = (f("http://ex/y", "http://ex/x"), f("http://ex/x", "http://ex/y"));
    match serialize_facts([&a, &b].into_iter()) {
        Err(NotRepresentable::UnrepresentableScope(_)) => {}
        other => panic!("{other:?}"),
    }
    let cyc = format!("{p}:x :k {{ @forAll :y. :y :k :k }}.\n:y :k {{ @forAll :x. :x :k :k }}.\n");
    let e = reason_n3_pass_all(&cyc, RuleVars::N3).expect_err("no order exists");
    assert!(e.contains("cycle"), "{e}");
}

/// GH #6701 review round 11 (HIGH), Codex's counterexample: a document-level universal
/// used only in a rule's premise is written with a DOCUMENT-level line, never moved into
/// the premise. Round 12: the premise-level declaration is the same one variable in the
/// engine (GH #6754), so both sources write the same document.
#[test]
fn a_document_level_binder_is_never_moved_into_a_formula() {
    let p = "@prefix : <http://ex/>.\n";
    let doc_level = format!("{p}@forAll :x. {{ :x :p :o }} => {{ :ok :is true }}.\n");
    let in_premise = format!("{p}{{ @forAll :x. :x :p :o }} => {{ :ok :is true }}.\n");
    let premise = |src: &str| one_rule(src).premise[0][0].clone();
    assert_eq!(premise(&doc_level), var("__ua.http://ex/x"));
    assert_eq!(premise(&in_premise), var("__ua.http://ex/x"), "one variable per IRI (GH #6754)");
    let want = "@forAll <http://ex/x> .\n{ <http://ex/x> <http://ex/p> <http://ex/o> . } => { <http://ex/ok> <http://ex/is> \"true\"^^<http://www.w3.org/2001/XMLSchema#boolean> . } .\n";
    for src in [&doc_level, &in_premise] {
        let doc = reason_n3_pass_all(src, RuleVars::N3).expect("pass-all");
        assert_eq!(doc, want);
        assert_eq!(premise(&doc), var("__ua.http://ex/x"));
    }
}

/// Round 12: a generated universal name is outside the lexical space of source quickvars.
/// The parser's variable lexer reads `[A-Za-z0-9_-]` and non-ASCII (N3 `VARNAME` /
/// `PN_CHARS`: never `.`), so `__ua.<iri>` cannot be written as a `?…` — a quickvar spelled
/// like it (`?__ua_x`, `?__ua`, `?__bn0_x`) is its own variable, never the universal.
#[test]
fn a_source_quickvar_never_collides_with_a_universal() {
    let p = "@prefix : <http://ex/>.\n";
    // Every quickvar the lexer can read carries no `.`; `?__ua.x` lexes as `?__ua` then `.`.
    let vars_of = |src: &str| {
        let parsed = parser::parse(&format!("{p}{src}")).unwrap_or_else(|e| panic!("{e}\n{src}"));
        let mut out = BTreeSet::new();
        for r in &parsed.rules {
            for t in r.premise.iter().chain(&r.conclusion).flatten() {
                if let Term::Var(v) = t {
                    out.insert(v.clone());
                }
            }
        }
        out
    };
    let vs = vars_of("@forAll :x. { :x :p ?__ua_x. ?__ua :p ?__bn0_x. ?__ua_x :r ?__bw0___ua_x } => { :x :pair ?__ua_x }.");
    assert_eq!(
        vs,
        ["__bn0_x", "__bw0___ua_x", "__ua", "__ua.http://ex/x", "__ua_x"].map(String::from).into_iter().collect(),
    );
    assert!(parser::parse(&format!("{p}{{ ?__ua.x :p :o }} => {{ :a :b :c }}.")).map_or(true, |d| {
        d.rules.iter().flat_map(|r| r.premise.iter().flatten()).all(|t| !matches!(t, Term::Var(v) if v.contains('.')))
    }));
    // And they reason apart: the universal and `?__ua_x` must bind DIFFERENT nodes here.
    let src = format!("{p}@forAll :x.\n{{ :x :p :b. ?__ua_x :p :c }} => {{ :x :pair ?__ua_x }}.\n:a :p :b. :d :p :c.\n");
    let derived = |doc: &str| -> BTreeSet<String> {
        sparq_reason::reason_n3_terms(doc, None).unwrap_or_else(|e| panic!("{e}\n{doc}")).facts.iter().map(|f| format!("{f:?}")).collect()
    };
    let got = derived(&src);
    assert!(got.contains(&format!("{:?}", [iri("http://ex/a"), iri("http://ex/pair"), iri("http://ex/d")])), "{got:#?}");
    assert_eq!(got.len(), 3, "nothing extra: {got:#?}");
    let doc = reason_n3_pass_all(&src, RuleVars::N3).expect("pass-all");
    assert_eq!(derived(&doc), got, "{doc}");
}

/// GH #6701 review round 12 (Codex at e35a0db5): a formula fact compared with one
/// INDEPENDENTLY parsed from a literal (`log:parsedAsN3`). Under per-binder numbering a
/// re-parse could renumber binders and flip `log:equalTo`; with one full-IRI name per
/// universal there is no numbering, so the `pass_all` output re-reasoned derives exactly
/// what the original derives.
#[test]
fn a_parsed_as_n3_formula_compares_the_same_after_a_round_trip() {
    let src = "@prefix : <http://ex/>.\n@prefix log: <http://www.w3.org/2000/10/swap/log#>.\n\
               :z :p { @forAll :x. :x :q :o }.\n\
               :a :p { @forAll :x. :x :q :o }.\n\
               {\n  :a :p ?f.\n  \"@prefix : <http://ex/>. :a :p { @forAll :x. :x :q :o }.\" log:parsedAsN3 ?doc.\n\
               ?doc log:includes { :a :p ?g }.\n  ?f log:equalTo ?g.\n} => { :bad :is true }.\n";
    let closure = |doc: &str| -> BTreeSet<String> {
        sparq_reason::reason_n3_terms(doc, None).unwrap_or_else(|e| panic!("{e}\n{doc}")).facts.iter().map(|f| format!("{f:?}")).collect()
    };
    let original = closure(src);
    let doc = reason_n3_pass_all(src, RuleVars::N3).expect("pass-all");
    assert_eq!(closure(&doc), original, "re-reasoning the output changed what it derives:\n{doc}");
}

/// Round 12: `log:equalTo` / `log:notEqualTo` between quoted formulae — with `@forAll`
/// universals, quickvars and blank nodes, as facts and as formulae parsed from literals —
/// give the same results before and after a `pass_all` round trip.
#[test]
fn formula_equality_is_the_same_after_a_round_trip() {
    let src = "@prefix : <http://ex/>.\n@prefix log: <http://www.w3.org/2000/10/swap/log#>.\n\
               @forAll :x.\n\
               :f1 :is { :x :q :o }.\n\
               :f2 :is { :x :q :o }.\n\
               :f3 :is { ?v :q :o }.\n\
               :f4 :is { ?w :q :o }.\n\
               :f5 :is { _:b :q :o }.\n\
               :f6 :is { _:c :q :o }.\n\
               :f7 :is { :x :q ?v. _:b :r :x }.\n\
               \"@prefix : <http://ex/>. @forAll :x. :g :is { :x :q :o }.\" log:parsedAsN3 :lit.\n\
               { ?a :is ?f. ?b :is ?g. ?f log:equalTo ?g } => { ?a :eq ?b }.\n\
               { ?a :is ?f. ?b :is ?g. ?f log:notEqualTo ?g } => { ?a :ne ?b }.\n\
               { ?a :is ?f. \"@prefix : <http://ex/>. @forAll :x. :g :is { :x :q :o }.\" log:parsedAsN3 ?d. \
                 ?d log:includes { :g :is ?g }. ?f log:equalTo ?g } => { ?a :eq :parsed }.\n\
               { ?a :is ?f. \"@prefix : <http://ex/>. :g :is { ?v :q :o }.\" log:parsedAsN3 ?d. \
                 ?d log:includes { :g :is ?g }. ?f log:equalTo ?g } => { ?a :eq :parsedVar }.\n";
    let closure = |doc: &str| -> BTreeSet<String> {
        sparq_reason::reason_n3_terms(doc, None)
            .unwrap_or_else(|e| panic!("{e}\n{doc}"))
            .facts
            .iter()
            .filter(|f| f[1] == iri("http://ex/eq") || f[1] == iri("http://ex/ne"))
            .map(|f| format!("{f:?}"))
            .collect()
    };
    let original = closure(src);
    // The comparisons really ran, both ways, including against the parsed formulae.
    for want in ["f1\"), Iri(\"http://ex/eq\"), Iri(\"http://ex/f2", "f1\"), Iri(\"http://ex/ne\"), Iri(\"http://ex/f3", "f1\"), Iri(\"http://ex/eq\"), Iri(\"http://ex/parsed\")", "f3\"), Iri(\"http://ex/eq\"), Iri(\"http://ex/parsedVar\")"] {
        assert!(original.iter().any(|f| f.contains(want)), "{want}: {original:#?}");
    }
    let doc = reason_n3_pass_all(src, RuleVars::N3).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(closure(&doc), original, "{doc}");
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

/// A mention of `:x` / `:y`: `Some(local name)` when an `@forAll` of it is in effect
/// there (it is then the one universal of that IRI), `None` for a plain IRI.
type Mention = Option<&'static str>;

/// One generated N3 document with its INDEPENDENT binding oracle: for every statement, the
/// mentions of `:x` / `:y` in text order, each a universal or plain under N3's lexical
/// scoping (an `@forAll` in effect in this or an enclosing scope, "thereafter"). Every
/// declaration of one IRI is the same variable (GH #6754).
struct Generated {
    text: String,
    /// Per statement: is it a rule; its mentions in text order.
    statements: Vec<(bool, Vec<Mention>)>,
}

struct Gen<'r> {
    r: &'r mut Rng,
    text: String,
    /// Scope frames, document first: the IRI local names declared `@forAll` in each.
    frames: Vec<BTreeSet<&'static str>>,
    mentions: Vec<Mention>,
}

impl Gen<'_> {
    /// `@forAll :x.` / `:y.` here.
    fn declare(&mut self) {
        let name = ["x", "y"][self.r.below(2)];
        self.text.push_str(&format!("@forAll :{name}. "));
        self.frames.last_mut().unwrap().insert(name);
    }
    fn mention(&mut self, name: &'static str) {
        let bound = self.frames.iter().any(|f| f.contains(name));
        self.mentions.push(bound.then_some(name));
        self.text.push_str(&format!(":{name} "));
    }
    fn term(&mut self, depth: usize) {
        match (depth, self.r.below(10)) {
            (0, _) | (_, 0..=5) => match self.r.below(5) {
                0 | 1 => self.mention("x"),
                2 => self.mention("y"),
                3 => self.text.push_str("?v "),
                _ => self.text.push_str(":k "),
            },
            (_, 6) => {
                self.text.push_str("( ");
                for _ in 0..1 + self.r.below(2) {
                    self.term(depth - 1);
                }
                self.text.push_str(") ");
            }
            _ => self.formula(depth - 1, None),
        }
    }
    /// `{ rows }`, with `@forAll` declarations at random row boundaries; `first`, if any,
    /// is written as its first row.
    fn formula(&mut self, depth: usize, first: Option<&str>) {
        self.text.push_str("{ ");
        self.frames.push(BTreeSet::new());
        if let Some(row) = first {
            self.text.push_str(row);
        }
        for _ in 0..1 + self.r.below(3) {
            if self.r.below(3) == 0 {
                self.declare();
            }
            self.term(depth);
            self.text.push_str(":q ");
            self.term(depth);
            self.text.push_str(". ");
        }
        self.frames.pop();
        self.text.push_str("} ");
    }
}

fn generate(r: &mut Rng) -> Generated {
    let mut g = Gen { r, text: "@prefix : <http://ex/>.\n".into(), frames: vec![BTreeSet::new()], mentions: Vec::new() };
    let mut statements = Vec::new();
    for i in 0..1 + g.r.below(3) {
        if g.r.below(3) == 0 {
            g.declare();
        }
        let rule = g.r.below(3) == 0;
        if rule {
            // A rule that never fires (nothing is ever `<urn:never>`), tagged `<urn:s{i}>`.
            g.formula(2, Some(&format!("<urn:never> <urn:never> <urn:s{i}> . ")));
            g.text.push_str("=> ");
            g.formula(2, None);
        } else {
            g.term(3);
            g.text.push_str(&format!("<urn:p{i}> "));
            g.term(3);
        }
        g.text.push_str(".\n");
        statements.push((rule, std::mem::take(&mut g.mentions)));
    }
    Generated { text: g.text, statements }
}

/// Every `:x` / `:y` mention in `t`, in text order: the universal's variable name, or `None`
/// for the plain IRI.
fn mentions_in(t: &Term, out: &mut Vec<Option<String>>) {
    match t {
        Term::Var(v) if universal_of(v).is_some() => out.push(Some(v.clone())),
        Term::Iri(i) if i == "http://ex/x" || i == "http://ex/y" => out.push(None),
        Term::List(ms) => ms.iter().for_each(|m| mentions_in(m, out)),
        Term::Triple(tr) => tr.iter().for_each(|m| mentions_in(m, out)),
        Term::Formula(ts) => ts.iter().flatten().for_each(|m| mentions_in(m, out)),
        _ => {}
    }
}

/// The parsed statements of `doc` tagged `i` (a fact with predicate `<urn:p{i}>`, or a
/// rule whose premise starts with the `<urn:s{i}>` row), as their mentions in text order.
fn mentions_of(doc: &parser::Parsed, i: usize, rule: bool) -> Vec<Option<String>> {
    let mut out = Vec::new();
    if rule {
        let s = Term::Iri(format!("urn:s{i}"));
        let r = doc.rules.iter().find(|r| r.premise.first().is_some_and(|row| row[2] == s)).expect("the rule is there");
        r.premise.iter().chain(&r.conclusion).flatten().for_each(|t| mentions_in(t, &mut out));
    } else {
        let p = Term::Iri(format!("urn:p{i}"));
        let f = doc.facts.iter().find(|f| f[1] == p).expect("the fact is there");
        f.iter().for_each(|t| mentions_in(t, &mut out));
    }
    out
}

/// The oracle's binding of every mention against the parsed names: the universal of
/// exactly that IRI where one is in effect, the plain IRI where none is.
fn same_binding(g: &Generated, doc: &parser::Parsed) -> Result<(), String> {
    for (i, (rule, want)) in g.statements.iter().enumerate() {
        let got = mentions_of(doc, i, *rule);
        if got.len() != want.len() {
            return Err(format!("statement {i}: {} mentions, oracle {}", got.len(), want.len()));
        }
        for (w, n) in want.iter().zip(&got) {
            let ok = match (w, n) {
                (None, None) => true,
                (Some(local), Some(name)) => *name == format!("{UA}http://ex/{local}"),
                _ => false,
            };
            if !ok {
                return Err(format!("statement {i}: oracle {w:?}, parsed {n:?}"));
            }
        }
    }
    Ok(())
}

/// GH #6701 review rounds 11–12, as a property over SCOPE-CARRYING input: random N3
/// documents of 1–3 statements (facts, and rules that never fire), with `@forAll :x` / `:y`
/// declarations at random scopes — the document, any formula, between any two rows — and
/// plain mentions outside every scope. An oracle computes, from the GENERATED text alone,
/// which mentions are the universal. The parse must agree with it; and `reason_n3_pass_all`
/// either writes a document whose re-parse agrees with it too, or refuses. Nothing is ever
/// re-bound silently, and the re-parse gate never has to catch a placement.
#[test]
fn random_scopes_round_trip_or_are_refused() {
    let mut r = Rng(0x9E37_79B9_7F4A_7C15);
    let (mut wrote, mut refused, mut caught) = (0, 0, 0);
    for _ in 0..3000 {
        let g = generate(&mut r);
        let src = parser::parse(&g.text).unwrap_or_else(|e| panic!("{e}\n{}", g.text));
        same_binding(&g, &src).unwrap_or_else(|e| panic!("the PARSER disagrees with the oracle: {e}\n{}", g.text));
        match reason_n3_pass_all(&g.text, RuleVars::N3) {
            Ok(doc) => {
                let back = parser::parse(&doc).unwrap_or_else(|e| panic!("{e}\n{doc}"));
                same_binding(&g, &back).unwrap_or_else(|e| panic!("re-bound silently: {e}\n{}\n---\n{doc}", g.text));
                wrote += 1;
            }
            Err(e) => {
                refused += 1;
                caught += usize::from(e.contains("re-parses as") || e.contains("does not re-parse"));
                // The only refusals: a unit mixing a plain mention and the universal (the one
                // document line would capture it), or units that need each other first.
                assert!(e.contains("capture a plain mention") || e.contains("form a cycle"), "{e}\n{}", g.text);
            }
        }
    }
    eprintln!("{wrote} written, {refused} refused ({caught} by the re-parse gate)");
    // Over-refusal is the safe failure mode: plain mentions are frequent in the generator.
    assert!(wrote > 800 && refused > 10 && caught == 0, "{wrote} written, {refused} refused, {caught} by the gate");
}
