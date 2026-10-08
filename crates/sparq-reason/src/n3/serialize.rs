//! N3 surface-syntax **serialization** — the crate's single writer for [`Term`]s, ground
//! statements, and (`sq-xqchl.2`) whole RULES.
//!
//! The term/statement writers moved here from `incremental` (which used them as the
//! fallback / differential-oracle path) so the rule writer below shares ONE definition of
//! how a term is rendered — a second copy is how a serializer and its parser drift apart.
//! Lossless for parser-shaped terms: blank labels round-trip verbatim, language tags are
//! already lowercase, plain strings re-acquire `xsd:string`.
//!
//! # Echoing rules back out (EYE `--pass-all` / `--pass-all-ground`)
//!
//! The forward chainer CONSUMES rules — [`crate::reason_n3`] returns only entailed ground
//! facts — so a caller that wants EYE's "deductive closure PLUS the rules" output needs the
//! rules rendered back into N3. [`write_rule`] does that from the PARSED rule (not the
//! source text), which is why two details differ from the input document verbatim:
//!
//! * **Premise blank nodes.** The parser rewrites a rule-scoped premise blank `_:x` to a
//!   fresh rule variable `?__bn.<rule>.x` (N3 semantics: a premise existential matches like a
//!   variable). [`write_rule`] maps that name back to `_:x`, so the emitted rule re-parses
//!   to the same rule rather than leaking an engine-internal variable name. The name is one
//!   no document can write — an N3 variable name cannot contain a `.` — so this reverses the
//!   parser's own rewrite and only that: a source variable spelled `?__bn0_x` is legal, is
//!   NOT the rewrite, and stays a variable.
//! * **`@forAll` universals.** The parser reads an `@forAll`-declared IRI as the variable
//!   `?__ua.<iri>` ([`UNIVERSAL_VAR`], unforgeable for the same reason). It is written BACK
//!   as that IRI under an `@forAll <iri> .` declaration, so re-parsing yields the very same
//!   `__ua.<iri>` term: identity is a function of the IRI alone, never of a per-document
//!   renaming. See [`write_term`] for the scoping rules (GH #5391, GH #6701 review).
//!   Only [`RuleVars::VarIris`], which grounds variables into `var:` IRIs and is not meant
//!   to be re-reasoned, names a universal by its local name.
//! * **Prefixes / layout.** Everything is written in full `<…>` IRI form, one statement per
//!   line — no `@prefix` declarations are reconstructed. The document is semantically the
//!   same N3, not byte-identical to the input.

use std::collections::{BTreeSet, HashMap, HashSet};

use super::model::{Rule, Term};

const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";

/// The SWAP **variable** namespace. A universal rule variable `?x` written as
/// `<http://www.w3.org/2000/10/swap/var#x>` is EYE's convention for a rule carried in a
/// document that must contain no syntactic variables — see [`RuleVars::VarIris`].
pub const VAR_NS: &str = "http://www.w3.org/2000/10/swap/var#";

/// The engine-internal prefix the N3 parser gives a rewritten premise blank node
/// (`_:x` in rule `i` becomes the variable `__bn.{i}.x`) — reversed by [`write_rule`].
///
/// The `.` is load-bearing, not decoration: the parser's variable-name lexer (`read_name`)
/// stops at a `.`, so NO variable a document can write is ever spelled this way. That is
/// what makes [`premise_blank_label`] a decode of the parser's own provenance rather than a
/// guess from a spelling a user could also pick — `?__bn0_x`, the underscore form, IS a
/// legal source variable and must stay a variable.
pub(super) const PREMISE_BLANK_VAR: &str = "__bn.";

/// The engine-internal prefix the N3 parser gives an `@forAll` universal (`@forAll :x`
/// reads `:x` as the variable `__ua.<full IRI of :x>`) — reversed by [`write_rule`]. The
/// `.` makes it unforgeable exactly as for [`PREMISE_BLANK_VAR`], and keying on the full
/// IRI keeps `@forAll a:x, b:x` two variables.
pub(super) const UNIVERSAL_VAR: &str = "__ua.";

/// The declared IRI behind an `@forAll` universal variable name, or `None` for any other
/// variable.
///
/// Besides the parser's own `__ua.<iri>` this accepts the backward chainer's freshened
/// copies (`rename_vars` prefixes `__bw<n>_`, possibly more than once): standardizing a
/// rule apart must not strip a universal of its provenance, or a copy leaking into a
/// result would be written as an ordinary — and, with its `.`, unparseable — variable.
/// Exact, not heuristic: the `.` in `__ua.` cannot occur in a source variable name.
pub(super) fn universal_iri(v: &str) -> Option<&str> {
    let mut rest = v;
    loop {
        if let Some(iri) = rest.strip_prefix(UNIVERSAL_VAR) {
            return Some(iri);
        }
        let r = rest.strip_prefix("__bw")?;
        let digits = r.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        rest = r[digits..].strip_prefix('_')?;
    }
}

/// The declared IRI's local name, with every character an N3 variable name cannot carry
/// replaced by `_` (`u` when nothing is left).
fn local_name(iri: &str) -> String {
    let local = iri.rsplit(['#', '/']).next().unwrap_or(iri);
    let name: String = local
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '-' => c,
            _ if !c.is_ascii() => c,
            _ => '_',
        })
        .collect();
    if name.is_empty() { "u".to_string() } else { name }
}

/// The variable name a universal is written under when NO `@forAll` declaration can scope
/// it (see [`write_term`]): its local name plus a 64-bit FNV-1a hash of the full IRI. A
/// deterministic function of the IRI — the same universal gets the same name in every
/// document and every proof, and two universals sharing a local name stay apart.
fn undeclarable_name(iri: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in iri.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{}_u{h:016x}", local_name(iri))
}

/// Does the IRI `iri` occur as an IRI term anywhere in `t` (any depth)? A declaration
/// `@forAll <iri>` would capture every such occurrence in its scope.
fn mentions_iri(t: &Term, iri: &str) -> bool {
    match t {
        Term::Iri(i) => i == iri,
        Term::List(ms) => ms.iter().any(|m| mentions_iri(m, iri)),
        Term::Triple(tr) => tr.iter().any(|m| mentions_iri(m, iri)),
        Term::Formula(ts) => ts.iter().flatten().any(|m| mentions_iri(m, iri)),
        _ => false,
    }
}

/// The universals occurring in `t` at the CURRENT scope level: through lists and quoted
/// triples (plain structure), but not into a nested `{ … }` formula, which scopes its own.
fn level_universals<'a>(t: &'a Term, out: &mut BTreeSet<&'a str>) {
    match t {
        Term::Var(v) => {
            if let Some(iri) = universal_iri(v) {
                out.insert(iri);
            }
        }
        Term::List(ms) => ms.iter().for_each(|m| level_universals(m, out)),
        Term::Triple(tr) => tr.iter().for_each(|m| level_universals(m, out)),
        _ => {}
    }
}

/// The `@forAll` declarations one scope (a formula, or a whole document) carries: each
/// universal occurring at its own level whose IRI the scope does not ALSO mention as an
/// IRI, anywhere inside it (a declaration would capture that IRI, nested formulae
/// included).
fn scope_declarations<'a>(stmts: &[&'a [Term; 3]]) -> BTreeSet<&'a str> {
    let mut level = BTreeSet::new();
    for t in stmts.iter().copied().flatten() {
        level_universals(t, &mut level);
    }
    level.retain(|iri| !stmts.iter().copied().flatten().any(|t| mentions_iri(t, iri)));
    level
}

fn write_declarations(decls: &BTreeSet<&str>, out: &mut String) {
    out.push_str("@forAll");
    for (i, iri) in decls.iter().enumerate() {
        out.push_str(if i == 0 { " <" } else { ", <" });
        out.push_str(iri);
        out.push('>');
    }
    out.push_str(" .");
}

fn quote_into(v: &str, out: &mut String) {
    for c in v.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
}

/// Write one N3 term in its surface syntax: IRIs `<…>`, literals `"lex"` (+ `@lang` /
/// `^^<dt>`, `xsd:string` left implicit), blanks `_:l`, variables `?v`, lists `( … )`,
/// formulae `{ … }`, RDF-star quoted triples `<< s p o >>`.
///
/// An `@forAll` universal (`__ua.<iri>`, see [`universal_iri`]) is written as its own IRI
/// under an `@forAll <iri> .` declaration, so the text re-parses to the SAME term. The
/// rendering is context-free — a deterministic function of the term — which is what lets a
/// proof, a closure dump and a re-parse all agree on one fact's identity:
///
/// * each `{ … }` formula declares, at its start, the universals occurring at its own
///   level (`{ @forAll <http://ex/x> . <http://ex/x> <http://ex/q> ?x . }`); a nested
///   formula declares its own;
/// * a universal is left undeclared when its scope ALSO mentions the same IRI as a plain
///   IRI (the declaration would capture it), and one that sits outside every formula has
///   no scope this writer can declare in; both are written as the plain variable
///   [`undeclarable_name`] instead (local name + IRI hash). A document writer
///   ([`serialize_facts`]) declares such top-level universals for the whole document when
///   it can.
pub fn write_term(t: &Term, out: &mut String) {
    write_scoped(t, &BTreeSet::new(), out);
}

/// [`write_term`] with `declared` the `@forAll` IRIs in scope at `t`'s level.
fn write_scoped(t: &Term, declared: &BTreeSet<&str>, out: &mut String) {
    match t {
        Term::Iri(i) => {
            out.push('<');
            out.push_str(i);
            out.push('>');
        }
        Term::Lit(v, _, Some(lang)) => {
            out.push('"');
            quote_into(v, out);
            out.push('"');
            out.push('@');
            out.push_str(lang);
        }
        Term::Lit(v, dt, None) => {
            out.push('"');
            quote_into(v, out);
            out.push('"');
            if dt != XSD_STRING {
                out.push_str("^^<");
                out.push_str(dt);
                out.push('>');
            }
        }
        Term::Blank(l) => {
            out.push_str("_:");
            out.push_str(l);
        }
        Term::Var(v) => match universal_iri(v) {
            Some(iri) if declared.contains(iri) => {
                out.push('<');
                out.push_str(iri);
                out.push('>');
            }
            Some(iri) => {
                out.push('?');
                out.push_str(&undeclarable_name(iri));
            }
            None => {
                out.push('?');
                out.push_str(v);
            }
        },
        Term::List(ms) => {
            out.push('(');
            for m in ms {
                out.push(' ');
                write_scoped(m, declared, out);
            }
            out.push_str(" )");
        }
        Term::Formula(ts) => {
            // A formula is its own scope: its rendering ignores `declared` (an enclosing
            // declaration of an IRI it ALSO mentions plainly is impossible by construction —
            // that enclosing scope would mention the IRI too).
            let decls = scope_declarations(&ts.iter().collect::<Vec<_>>());
            out.push('{');
            if !decls.is_empty() {
                out.push(' ');
                write_declarations(&decls, out);
            }
            for t in ts {
                out.push(' ');
                write_scoped(&t[0], &decls, out);
                out.push(' ');
                write_scoped(&t[1], &decls, out);
                out.push(' ');
                write_scoped(&t[2], &decls, out);
                out.push_str(" .");
            }
            out.push_str(" }");
        }
        // RDF-star quoted-triple term — round-trips through the N3 parser's
        // `<< s p o >>` form (GH #2012). [FABLE-5]
        Term::Triple(tr) => {
            out.push_str("<< ");
            write_scoped(&tr[0], declared, out);
            out.push(' ');
            write_scoped(&tr[1], declared, out);
            out.push(' ');
            write_scoped(&tr[2], declared, out);
            out.push_str(" >>");
        }
    }
}

/// A term as N3 text for a DIAGNOSTIC (an error or fallback reason a user reads) — the
/// [`write_term`] rendering, so no message leaks an engine-internal spelling such as
/// `__ua.<iri>`, and a term reads the same in a message as in any output.
pub(crate) fn display(t: &Term) -> String {
    let mut s = String::new();
    write_term(t, &mut s);
    s
}

/// Write one statement as `s p o .` plus a newline, each term per [`write_term`].
pub fn write_statement(f: &[Term; 3], out: &mut String) {
    write_statement_scoped(f, &BTreeSet::new(), out);
}

fn write_statement_scoped(f: &[Term; 3], declared: &BTreeSet<&str>, out: &mut String) {
    write_scoped(&f[0], declared, out);
    out.push(' ');
    write_scoped(&f[1], declared, out);
    out.push(' ');
    write_scoped(&f[2], declared, out);
    out.push_str(" .\n");
}

/// Serialize facts back to N3 as one document: a document-level `@forAll` line when a
/// universal sitting outside every formula needs one (and the document never mentions its
/// IRI plainly), then one statement per line in the given order ([`write_term`]).
/// Re-parsing it yields the same terms, universals included.
pub fn serialize_facts<'a>(facts: impl Iterator<Item = &'a [Term; 3]>) -> String {
    let facts: Vec<&[Term; 3]> = facts.collect();
    let decls = scope_declarations(&facts);
    let mut out = String::new();
    if !decls.is_empty() {
        write_declarations(&decls, &mut out);
        out.push('\n');
    }
    for f in facts {
        write_statement_scoped(f, &decls, &mut out);
    }
    out
}

/// Write one output DOCUMENT: the document-level `@forAll` line its top-level universals
/// need (computed over the facts AND the rules, since a document-wide declaration would
/// capture the IRI in either), then `facts` one per line, SORTED (deterministic output),
/// then `rules` in order — the `--pass-all` layout ([`crate::reason_n3_pass_all`]).
pub(super) fn write_document(
    facts: &[&[Term; 3]],
    rules: &[(&Rule, RuleKind)],
    vars: RuleVars,
    out: &mut String,
) {
    let as_stmts: Vec<[Term; 3]> = rules
        .iter()
        .map(|(r, _)| {
            [Term::Formula(r.premise.clone()), Term::Iri(String::new()), Term::Formula(r.conclusion.clone())]
        })
        .collect();
    let mut scope: Vec<&[Term; 3]> = facts.to_vec();
    scope.extend(as_stmts.iter());
    let decls = scope_declarations(&scope);
    if !decls.is_empty() {
        write_declarations(&decls, out);
        out.push('\n');
    }
    let mut lines: Vec<String> = facts
        .iter()
        .map(|f| {
            let mut s = String::new();
            write_statement_scoped(f, &decls, &mut s);
            s
        })
        .collect();
    lines.sort_unstable();
    out.push_str(&lines.concat());
    for (r, kind) in rules {
        write_rule(r, *kind, vars, out);
    }
}

/// How a rule's universal variables are written when the rule is echoed into an output
/// document ([`write_rule`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleVars {
    /// N3 variables, `?x` — EYE `--pass-all`. The emitted rule re-parses as the SAME rule,
    /// so the document can be fed straight back into the reasoner.
    N3,
    /// SWAP `var:` IRIs, `<http://www.w3.org/2000/10/swap/var#x>` ([`VAR_NS`]) — EYE
    /// `--pass-all-ground`. EVERY variable of the echoed rule is grounded, at any depth:
    /// through lists and quoted triples, and into quoted `{ … }` formulae too (an N3 `?x`
    /// is quantified in the outermost formula, so the same name is the same variable
    /// however deeply it is nested). No `?x` survives in a rule, which is what a
    /// downstream consumer that cannot handle one needs. It is NOT re-reasonable:
    /// re-parsed, those IRIs are constants, and the rule only fires on literally-matching
    /// `var:` data. Blank nodes are left as blank nodes.
    ///
    /// This grounds RULES. A formula-valued FACT is data and is echoed verbatim, so a
    /// document that asserts one carrying a variable (`:a :p { ?x :q :b }.`) still shows
    /// that `?x` in the closure half — see [`crate::reason_n3_pass_all`].
    VarIris,
}

/// Which arrow a rule is written with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleKind {
    /// A forward rule, `{ premise } => { conclusion } .` (`log:implies`).
    Forward,
    /// A goal-directed backward rule, `{ conclusion } <= { premise } .` (`log:isImpliedBy`).
    Backward,
}

/// Write one rule as an N3 statement (`{ … } => { … } .` / `{ … } <= { … } .`) plus a
/// newline, rendering its variables per `vars`.
///
/// The premise-blank rewrite the parser applies is undone first — see the module docs — so
/// a `RuleVars::N3` round trip yields an equivalent rule.
///
pub fn write_rule(r: &Rule, kind: RuleKind, vars: RuleVars, out: &mut String) {
    let (left, arrow, right) = match kind {
        RuleKind::Forward => (&r.premise, " => ", &r.conclusion),
        RuleKind::Backward => (&r.conclusion, " <= ", &r.premise),
    };
    let names = match vars {
        RuleVars::VarIris => ground_names(r),
        RuleVars::N3 => HashMap::new(),
    };
    // Each side is written as a formula TERM, so universals get `@forAll` declarations
    // scoped exactly as for data ([`write_term`]).
    let side = |stmts: &[[Term; 3]]| {
        Term::Formula(stmts.iter().map(|row| row.clone().map(|t| rule_term(&t, vars, &names))).collect())
    };
    write_term(&side(left), out);
    out.push_str(arrow);
    write_term(&side(right), out);
    out.push_str(" .\n");
}

/// [`RuleVars::VarIris`] only: the `var:` local name of each universal in `r` — its
/// [`local_name`], suffixed `_2`, `_3`, … until it differs from every other variable name
/// in the rule, so grounding never merges two variables.
fn ground_names(r: &Rule) -> HashMap<String, String> {
    fn walk<'a>(t: &'a Term, seen: &mut BTreeSet<&'a str>) {
        match t {
            Term::Var(v) => {
                seen.insert(v);
            }
            Term::List(ms) => ms.iter().for_each(|m| walk(m, seen)),
            Term::Triple(tr) => tr.iter().for_each(|m| walk(m, seen)),
            Term::Formula(ts) => ts.iter().flatten().for_each(|m| walk(m, seen)),
            _ => {}
        }
    }
    let mut seen = BTreeSet::new();
    for t in r.premise.iter().chain(&r.conclusion).flatten() {
        walk(t, &mut seen);
    }
    let (universals, rest): (Vec<&str>, Vec<&str>) =
        seen.into_iter().partition(|v| universal_iri(v).is_some());
    let mut taken: HashSet<String> = rest.into_iter().map(str::to_string).collect();
    let mut names = HashMap::new();
    for v in universals {
        let base = local_name(universal_iri(v).unwrap_or(v));
        let mut name = base.clone();
        let mut n = 2;
        while !taken.insert(name.clone()) {
            name = format!("{base}_{n}");
            n += 1;
        }
        names.insert(v.to_string(), name);
    }
    names
}

/// A rule-side term prepared for output: the parser's `?__bn.<i>.x` premise-blank variables
/// become `_:x` again, and — under [`RuleVars::VarIris`] — every remaining variable becomes
/// its `var:` IRI.
///
/// The premise-blank half mirrors the parser's own rule rewrite (`rewrite_term`,
/// `into_formulae = false`): it descends through lists and quoted triples, which are
/// transparent rule structure, but a quoted `{ … }` formula's BLANKS are graph-local and
/// were never rewritten, so there is nothing to undo in there.
///
/// Variables are different, and [`RuleVars::VarIris`] therefore does descend into a
/// formula: an N3 `?x` is quantified in the OUTERMOST formula, so the `?x` inside a quoted
/// graph is the same rule variable (`apply_deep` substitutes bindings into formulae). Left
/// alone it would leave a `?x` in a document whose whole point is to carry none — N3
/// builtins routinely take formula arguments, so that is a common shape, not a corner.
/// Under [`RuleVars::VarIris`] an `@forAll` universal is grounded too, under its
/// [`ground_names`] entry; under [`RuleVars::N3`] it is left for [`write_term`] to declare.
fn rule_term(t: &Term, vars: RuleVars, names: &HashMap<String, String>) -> Term {
    let sub = |m: &Term| rule_term(m, vars, names);
    match t {
        Term::List(ms) => Term::List(ms.iter().map(sub).collect()),
        Term::Triple(tr) => Term::Triple(Box::new([sub(&tr[0]), sub(&tr[1]), sub(&tr[2])])),
        Term::Formula(ts) => {
            Term::Formula(ts.iter().map(|r| [sub(&r[0]), sub(&r[1]), sub(&r[2])]).collect())
        }
        Term::Var(v) => {
            if let Some(label) = premise_blank_label(v) {
                return Term::Blank(label.to_string());
            }
            match vars {
                RuleVars::VarIris => {
                    Term::Iri(format!("{VAR_NS}{}", names.get(v).map_or(v.as_str(), String::as_str)))
                }
                RuleVars::N3 => t.clone(),
            }
        }
        _ => t.clone(),
    }
}

/// The original blank-node label behind a rewritten premise blank
/// `__bn.<digits>.<label>`, or `None` for an ordinary variable.
///
/// Exact, not heuristic: only the parser can mint a variable name containing a `.`
/// ([`PREMISE_BLANK_VAR`]), so a `Some` here is that rewrite and nothing else. The rule
/// index and the `.` separator are both mandatory, and a label may itself contain dots
/// (`_:a.b` is a legal Turtle blank label) — hence the split on the FIRST dot after the
/// digits, which cannot be part of either.
fn premise_blank_label(v: &str) -> Option<&str> {
    let rest = v.strip_prefix(PREMISE_BLANK_VAR)?;
    let (digits, label) = rest.split_once('.')?;
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())).then_some(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(t: &Term) -> String {
        let mut s = String::new();
        write_term(t, &mut s);
        s
    }

    #[test]
    fn writes_each_term_kind() {
        assert_eq!(rendered(&Term::Iri("http://ex/a".into())), "<http://ex/a>");
        assert_eq!(rendered(&Term::Blank("b0".into())), "_:b0");
        assert_eq!(rendered(&Term::Var("x".into())), "?x");
        assert_eq!(rendered(&Term::Lit("hi".into(), XSD_STRING.into(), None)), "\"hi\"");
        assert_eq!(
            rendered(&Term::Lit("1".into(), "http://ex/d".into(), None)),
            "\"1\"^^<http://ex/d>"
        );
        let tagged = Term::Lit("hi".into(), XSD_STRING.into(), Some("en".into()));
        assert_eq!(rendered(&tagged), "\"hi\"@en");
        assert_eq!(rendered(&Term::List(vec![Term::Var("x".into())])), "( ?x )");
        assert_eq!(rendered(&Term::Formula(vec![])), "{ }");
    }

    #[test]
    fn quotes_escapes() {
        let t = Term::Lit("a\"b\\c\nd".into(), XSD_STRING.into(), None);
        assert_eq!(rendered(&t), "\"a\\\"b\\\\c\\nd\"");
    }

    #[test]
    fn recognises_only_well_formed_premise_blank_names() {
        assert_eq!(premise_blank_label("__bn.0.x"), Some("x"));
        assert_eq!(premise_blank_label("__bn.12.a.b"), Some("a.b")); // `_:a.b` is a legal label
        assert_eq!(premise_blank_label("__bn.x.y"), None); // no digits
        assert_eq!(premise_blank_label("__bn.0"), None); // no separator
        assert_eq!(premise_blank_label("x"), None);
        // The underscore spelling is a LEGAL source variable, not the parser's rewrite:
        // decoding it would turn `?__bn0_x` into `_:x` (GH #5372 review round 1).
        assert_eq!(premise_blank_label("__bn0_x"), None);
    }

    #[test]
    fn var_iris_grounds_nested_formulae_and_keeps_blanks() {
        // A quoted formula's `?y` is the SAME rule variable (N3 quantifies `?y` in the
        // outermost formula), so the grounded form must carry no `?` at any depth.
        let inner = Term::Formula(vec![[
            Term::Var("y".into()),
            Term::Iri("http://ex/p".into()),
            Term::Var("y".into()),
        ]]);
        let ground = rendered(&rule_term(&inner, RuleVars::VarIris, &HashMap::new()));
        assert!(!ground.contains('?'), "{ground}");
        assert_eq!(ground, format!("{{ <{VAR_NS}y> <http://ex/p> <{VAR_NS}y> . }}"));
        // `RuleVars::N3` leaves the formula exactly as parsed.
        assert_eq!(rendered(&rule_term(&inner, RuleVars::N3, &HashMap::new())), rendered(&inner));
        // A rewritten premise blank goes back to `_:x` in BOTH styles.
        for style in [RuleVars::N3, RuleVars::VarIris] {
            let t = rule_term(&Term::Var("__bn.3.x".into()), style, &HashMap::new());
            assert_eq!(rendered(&t), "_:x");
        }
        assert_eq!(
            rendered(&rule_term(&Term::Var("z".into()), RuleVars::VarIris, &HashMap::new())),
            "<http://www.w3.org/2000/10/swap/var#z>"
        );
    }

    fn universal_formula() -> Term {
        Term::Formula(vec![[
            Term::Var(format!("{UNIVERSAL_VAR}http://ex/x")),
            Term::Iri("http://ex/q".into()),
            Term::Var("x".into()),
        ]])
    }

    /// A universal is written as its own IRI under a formula-scoped `@forAll`, so it can
    /// never merge with a source variable and re-parses to the same term (GH #6701).
    #[test]
    fn a_universal_is_written_under_a_scoped_declaration() {
        assert_eq!(
            rendered(&universal_formula()),
            "{ @forAll <http://ex/x> . <http://ex/x> <http://ex/q> ?x . }"
        );
        let back = super::super::parser::parse(&format!(":a :p {} .", rendered(&universal_formula())))
            .expect("re-parses");
        assert_eq!(back.facts[0][2], universal_formula());
    }

    /// A formula that ALSO mentions the IRI plainly cannot declare it; nor can anything
    /// outside a formula. Both fall back to the IRI-derived name, never to a bare local name.
    #[test]
    fn an_undeclarable_universal_gets_a_name_derived_from_its_iri() {
        let x = Term::Var(format!("{UNIVERSAL_VAR}http://ex/x"));
        let both = Term::Formula(vec![[x.clone(), Term::Iri("http://ex/x".into()), Term::Var("x".into())]]);
        let r = rendered(&both);
        assert!(!r.contains("@forAll"), "{r}");
        let name = format!("?{}", undeclarable_name("http://ex/x"));
        assert!(name.starts_with("?x_u"), "{name}");
        assert_eq!(r, format!("{{ {name} <http://ex/x> ?x . }}"));
        assert_eq!(rendered(&x), name);
        // Freshened backward-rule copies keep their provenance.
        assert_eq!(universal_iri("__bw3___bw0___ua.http://ex/x"), Some("http://ex/x"));
        assert_eq!(universal_iri("__bw3_x"), None);
        assert_eq!(rendered(&Term::Var("__bw0___ua.http://ex/x".into())), rendered(&x));
    }
}
