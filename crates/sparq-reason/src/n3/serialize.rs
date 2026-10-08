//! N3 surface-syntax **serialization** — the crate's single writer for [`Term`]s, ground
//! statements, and (`sq-xqchl.2`) whole RULES.
//!
//! The term/statement writers moved here from `incremental` (which used them as the
//! fallback / differential-oracle path) so the rule writer below shares ONE definition of
//! how a term is rendered — a second copy is how a serializer and its parser drift apart.
//! Lossless for parser-shaped terms (the contract is on [`Unit`]): blank labels round-trip
//! verbatim, language tags are already lowercase, plain strings re-acquire `xsd:string`,
//! IRIs are `IRIREF`-escaped.
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
//!   `__ua.<iri>` term (GH #5391, GH #6701 review). Every writer here plans one whole
//!   output unit at once — see [`Unit`] for the scoping rules, the collision-free fallback
//!   and the exact round-trip contract. Only [`RuleVars::VarIris`], which grounds
//!   variables into `var:` IRIs and is not meant to be re-reasoned, names a universal by
//!   its local name.
//! * **Prefixes / layout.** Everything is written in full `<…>` IRI form, one statement per
//!   line — no `@prefix` declarations are reconstructed. The document is semantically the
//!   same N3, not byte-identical to the input.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

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

/// `name` made into something the parser's variable lexer reads back whole: every
/// character a variable name cannot carry becomes `_` (`u` when nothing is left).
fn sanitize_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '-' => c,
            _ if !c.is_ascii() => c,
            _ => '_',
        })
        .collect();
    if s.is_empty() { "u".to_string() } else { s }
}

/// The declared IRI's local name as a variable-name base ([`sanitize_name`]).
fn local_name(iri: &str) -> String {
    sanitize_name(iri.rsplit(['#', '/']).next().unwrap_or(iri))
}

/// Can `?name` be written as is — does the parser's `read_name` read exactly `name` back?
fn spellable_var(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || !c.is_ascii())
}

/// Write `iri` as an N3 `IRIREF`, `<…>`. A backslash is the one character the parser can
/// DECODE into an IRI (from `\`) that it does not accept raw, so it goes back out
/// escaped. The other characters `IRIREF` forbids (controls, space, `<>"{}|^` and the
/// backtick) are escaped the same way; the parser refuses them even escaped, so an IRI
/// carrying one (which no parse can have produced) is not representable in N3 at all.
fn write_iriref(iri: &str, out: &mut String) {
    out.push('<');
    for c in iri.chars() {
        if (c as u32) <= 0x20 || matches!(c, '<' | '>' | '"' | '{' | '}' | '|' | '^' | '`' | '\\') {
            out.push_str(&format!("\\u{:04X}", c as u32));
        } else {
            out.push(c);
        }
    }
    out.push('>');
}

/// Does the IRI `iri` occur as an IRI term anywhere in `t` (any depth)? An `@forAll <iri>`
/// declaration captures every such occurrence after it in its scope, nested formulae
/// included.
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
/// triples (plain structure), but not into a nested `{ … }` formula, which is a scope of
/// its own.
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

/// Every variable name in `t`, at any depth.
fn all_vars<'a>(t: &'a Term, out: &mut BTreeSet<&'a str>) {
    match t {
        Term::Var(v) => {
            out.insert(v);
        }
        Term::List(ms) => ms.iter().for_each(|m| all_vars(m, out)),
        Term::Triple(tr) => tr.iter().for_each(|m| all_vars(m, out)),
        Term::Formula(ts) => ts.iter().flatten().for_each(|m| all_vars(m, out)),
        _ => {}
    }
}

/// For each universal at the level of the formula `ts`, the index of the first triple
/// carrying it there — where its `@forAll` declaration goes.
fn first_uses(ts: &[[Term; 3]]) -> BTreeMap<&str, usize> {
    let mut first = BTreeMap::new();
    for (i, row) in ts.iter().enumerate() {
        let mut here = BTreeSet::new();
        row.iter().for_each(|t| level_universals(t, &mut here));
        for iri in here {
            first.entry(iri).or_insert(i);
        }
    }
    first
}

/// Every universal that CANNOT be declared somewhere inside `t`: in some formula, a
/// triple at or after the one its declaration would precede mentions the same IRI as a
/// plain IRI (the declaration would capture it).
fn undeclarable_in<'a>(t: &'a Term, bad: &mut BTreeSet<&'a str>) {
    match t {
        Term::List(ms) => ms.iter().for_each(|m| undeclarable_in(m, bad)),
        Term::Triple(tr) => tr.iter().for_each(|m| undeclarable_in(m, bad)),
        Term::Formula(ts) => {
            for (iri, i) in first_uses(ts) {
                if ts[i..].iter().flatten().any(|m| mentions_iri(m, iri)) {
                    bad.insert(iri);
                }
            }
            ts.iter().flatten().for_each(|m| undeclarable_in(m, bad));
        }
        _ => {}
    }
}

/// Every naming decision for ONE top-level unit of output — a term, a statement, a rule,
/// or a whole document — made together, before anything is written, and applied to every
/// scope inside the unit.
///
/// The writer's contract: parsing what it writes yields the same terms, up to two
/// documented normalisations (and nothing else):
///
/// * a backward-chaining copy of a universal (`__bw<n>_` prefixes, see [`universal_iri`])
///   is written as the universal itself;
/// * a variable with no surface spelling of its own is renamed, collision-free and
///   consistently across the unit (so the renaming is a bijection: distinct variables
///   stay distinct, shared ones stay shared) — see [`Unit::plan`].
///
/// A universal (`__ua.<iri>`) is written as its own IRI under an `@forAll <iri> .`
/// declaration, placed right before the first triple of a formula that carries it at
/// that formula's level (or, for a document, as a first line). Re-parsing yields the very
/// same `__ua.<iri>` term, which is also what a `log:parsedAsN3` literal or another
/// document produces, and the rendering does not depend on anything outside the unit.
struct Unit {
    /// Universals (by IRI) that cannot be declared anywhere in the unit, and the plain
    /// variable each is written as instead.
    fallback: BTreeMap<String, String>,
    /// Non-universal variables whose internal name is not a legal variable name.
    renamed: BTreeMap<String, String>,
    /// Universals declared once for the whole document (document units only).
    doc_decls: BTreeSet<String>,
}

impl Unit {
    /// Plan the unit whose top level is `rows` (a document's statements, one statement,
    /// one term). `document`: whether a document-level `@forAll` line may be written.
    ///
    /// A universal falls back to a plain variable for the WHOLE unit (every scope, both
    /// sides of a rule) when any scope in the unit cannot declare it: a formula mentions
    /// its IRI plainly at or after its first use, or it sits outside every formula where
    /// no document line can declare it (not a document, or the document mentions the IRI
    /// plainly). The fallback name is the IRI's local name, then `_2`, `_3`, … until it
    /// differs from EVERY variable name in the unit — so it can never merge with a source
    /// variable, whatever that variable is spelled. Deterministic: universals are named in
    /// IRI order, after all spellable source names are reserved.
    fn plan(rows: &[&[Term]], document: bool) -> Unit {
        let terms = || rows.iter().flat_map(|r| r.iter());
        let mut bad = BTreeSet::new();
        let mut top = BTreeSet::new();
        terms().for_each(|t| level_universals(t, &mut top));
        let mut doc_decls = BTreeSet::new();
        for iri in top {
            if document && !terms().any(|t| mentions_iri(t, iri)) {
                doc_decls.insert(iri.to_string());
            } else {
                bad.insert(iri);
            }
        }
        terms().for_each(|t| undeclarable_in(t, &mut bad));
        bad.retain(|iri| !doc_decls.contains(*iri));

        let mut vars = BTreeSet::new();
        terms().for_each(|t| all_vars(t, &mut vars));
        let mut taken: BTreeSet<String> = vars
            .iter()
            .filter(|v| universal_iri(v).is_none() && spellable_var(v))
            .map(|v| v.to_string())
            .collect();
        let mut fresh = |base: String| {
            let mut name = base.clone();
            let mut n = 2;
            while !taken.insert(name.clone()) {
                name = format!("{base}_{n}");
                n += 1;
            }
            name
        };
        let fallback = bad.into_iter().map(|iri| (iri.to_string(), fresh(local_name(iri)))).collect();
        let renamed = vars
            .iter()
            .filter(|v| universal_iri(v).is_none() && !spellable_var(v))
            .map(|v| (v.to_string(), fresh(sanitize_name(v))))
            .collect();
        Unit { fallback, renamed, doc_decls }
    }

    fn write_doc_decls(&self, out: &mut String) {
        if !self.doc_decls.is_empty() {
            write_declarations(self.doc_decls.iter().map(String::as_str), out);
            out.push('\n');
        }
    }

    /// Write `t` (one term of this unit) in N3 surface syntax.
    fn term(&self, t: &Term, out: &mut String) {
        match t {
            Term::Iri(i) => write_iriref(i, out),
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
                    out.push_str("^^");
                    write_iriref(dt, out);
                }
            }
            Term::Blank(l) => {
                out.push_str("_:");
                out.push_str(l);
            }
            Term::Var(v) => match universal_iri(v) {
                Some(iri) => match self.fallback.get(iri) {
                    Some(name) => {
                        out.push('?');
                        out.push_str(name);
                    }
                    None => write_iriref(iri, out),
                },
                None => {
                    out.push('?');
                    out.push_str(self.renamed.get(v.as_str()).map_or(v.as_str(), String::as_str));
                }
            },
            Term::List(ms) => {
                out.push('(');
                for m in ms {
                    out.push(' ');
                    self.term(m, out);
                }
                out.push_str(" )");
            }
            Term::Formula(ts) => {
                let mut at: BTreeMap<usize, Vec<&str>> = BTreeMap::new();
                for (iri, i) in first_uses(ts) {
                    if !self.fallback.contains_key(iri) {
                        at.entry(i).or_default().push(iri);
                    }
                }
                out.push('{');
                for (i, row) in ts.iter().enumerate() {
                    if let Some(decls) = at.get(&i) {
                        out.push(' ');
                        write_declarations(decls.iter().copied(), out);
                    }
                    for t in row {
                        out.push(' ');
                        self.term(t, out);
                    }
                    out.push_str(" .");
                }
                out.push_str(" }");
            }
            // RDF-star quoted-triple term — round-trips through the N3 parser's
            // `<< s p o >>` form (GH #2012). [FABLE-5]
            Term::Triple(tr) => {
                out.push_str("<< ");
                self.term(&tr[0], out);
                out.push(' ');
                self.term(&tr[1], out);
                out.push(' ');
                self.term(&tr[2], out);
                out.push_str(" >>");
            }
        }
    }

    fn statement(&self, f: &[Term; 3], out: &mut String) {
        self.term(&f[0], out);
        out.push(' ');
        self.term(&f[1], out);
        out.push(' ');
        self.term(&f[2], out);
        out.push_str(" .\n");
    }
}

fn write_declarations<'a>(iris: impl Iterator<Item = &'a str>, out: &mut String) {
    out.push_str("@forAll");
    for (i, iri) in iris.enumerate() {
        out.push_str(if i == 0 { " " } else { ", " });
        write_iriref(iri, out);
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
/// The term is its own unit ([`Unit`]): parsing the text (in a term position) yields the
/// same term up to the documented normalisations. A universal sitting outside every
/// formula has no scope to declare it in, so it is written as a collision-free plain
/// variable.
pub fn write_term(t: &Term, out: &mut String) {
    Unit::plan(&[std::slice::from_ref(t)], false).term(t, out);
}

/// A term as N3 text for a DIAGNOSTIC (an error or fallback reason a user reads) — the
/// [`write_term`] rendering, so no message leaks an engine-internal spelling such as
/// `__ua.<iri>`, and a term reads the same in a message as in any output.
pub(crate) fn display(t: &Term) -> String {
    let mut s = String::new();
    write_term(t, &mut s);
    s
}

/// Write one statement as `s p o .` plus a newline — one unit ([`Unit`]).
pub fn write_statement(f: &[Term; 3], out: &mut String) {
    Unit::plan(&[&f[..]], false).statement(f, out);
}

/// The three terms of one statement, each rendered as part of that ONE unit — the strings
/// a proof node carries. A function of the statement alone, so a fact reads the same in
/// every proof it appears in (`sparq-prov` hashes these strings into its identity).
pub fn statement_strings(f: &[Term; 3]) -> [String; 3] {
    let unit = Unit::plan(&[&f[..]], false);
    f.clone().map(|t| {
        let mut s = String::new();
        unit.term(&t, &mut s);
        s
    })
}

/// Serialize facts back to N3 as one document (one [`Unit`]): a document-level
/// `@forAll` line when a universal outside every formula can be declared, then one
/// statement per line in the given order. Re-parsing yields the same terms up to the
/// documented normalisations.
pub fn serialize_facts<'a>(facts: impl Iterator<Item = &'a [Term; 3]>) -> String {
    let facts: Vec<&[Term; 3]> = facts.collect();
    let rows: Vec<&[Term]> = facts.iter().map(|f| &f[..]).collect();
    let unit = Unit::plan(&rows, true);
    let mut out = String::new();
    unit.write_doc_decls(&mut out);
    for f in facts {
        unit.statement(f, &mut out);
    }
    out
}

/// Write one output DOCUMENT as one [`Unit`]: closure `facts` one per line, SORTED
/// (deterministic output), then `rules` in order — the `--pass-all` layout
/// ([`crate::reason_n3_pass_all`]). One plan covers facts and rules together, so a
/// universal is written the same way in each.
pub(super) fn write_document(
    facts: &[&[Term; 3]],
    rules: &[(&Rule, RuleKind)],
    vars: RuleVars,
    out: &mut String,
) {
    let sides: Vec<[Term; 2]> = rules.iter().map(|(r, kind)| rule_sides(r, *kind, vars)).collect();
    let mut rows: Vec<&[Term]> = facts.iter().map(|f| &f[..]).collect();
    rows.extend(sides.iter().map(|s| &s[..]));
    let unit = Unit::plan(&rows, true);
    unit.write_doc_decls(out);
    let mut lines: Vec<String> = facts
        .iter()
        .map(|f| {
            let mut s = String::new();
            unit.statement(f, &mut s);
            s
        })
        .collect();
    lines.sort_unstable();
    out.push_str(&lines.concat());
    for (side, (_, kind)) in sides.iter().zip(rules) {
        write_rule_sides(&unit, side, *kind, out);
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
    let sides = rule_sides(r, kind, vars);
    write_rule_sides(&Unit::plan(&[&sides[..]], false), &sides, kind, out);
}

/// A rule's two sides as formula TERMS in written order (left of the arrow first), each
/// term put through [`rule_term`] — so its universals are scoped like any formula's.
fn rule_sides(r: &Rule, kind: RuleKind, vars: RuleVars) -> [Term; 2] {
    let (left, right) = match kind {
        RuleKind::Forward => (&r.premise, &r.conclusion),
        RuleKind::Backward => (&r.conclusion, &r.premise),
    };
    let names = match vars {
        RuleVars::VarIris => ground_names(r),
        RuleVars::N3 => HashMap::new(),
    };
    let side = |stmts: &[[Term; 3]]| {
        Term::Formula(stmts.iter().map(|row| row.clone().map(|t| rule_term(&t, vars, &names))).collect())
    };
    [side(left), side(right)]
}

fn write_rule_sides(unit: &Unit, sides: &[Term; 2], kind: RuleKind, out: &mut String) {
    unit.term(&sides[0], out);
    out.push_str(match kind {
        RuleKind::Forward => " => ",
        RuleKind::Backward => " <= ",
    });
    unit.term(&sides[1], out);
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

    /// A triple that mentions the IRI plainly AND carries the universal leaves no place for
    /// a declaration; nor does a top-level term. The fallback is a plain variable named
    /// collision-free against every variable of the unit — here `?x` and `?x_2` are taken.
    #[test]
    fn an_undeclarable_universal_gets_a_collision_free_name() {
        let x = Term::Var(format!("{UNIVERSAL_VAR}http://ex/x"));
        let both = Term::Formula(vec![
            [x.clone(), Term::Iri("http://ex/x".into()), Term::Var("x".into())],
            [Term::Var("x_2".into()), Term::Iri("http://ex/q".into()), x.clone()],
        ]);
        assert_eq!(rendered(&both), "{ ?x_3 <http://ex/x> ?x . ?x_2 <http://ex/q> ?x_3 . }");
        assert_eq!(rendered(&x), "?x");
        // Freshened backward-rule copies keep their provenance.
        assert_eq!(universal_iri("__bw3___bw0___ua.http://ex/x"), Some("http://ex/x"));
        assert_eq!(universal_iri("__bw3_x"), None);
        assert_eq!(rendered(&Term::Var("__bw0___ua.http://ex/x".into())), rendered(&x));
    }

    /// Plain mentions BEFORE the first use leave room for a mid-formula declaration, the
    /// shape the parser itself accepts (`{ :m :r :x. @forAll :x. :x :p :b }`).
    #[test]
    fn a_declaration_goes_right_before_the_first_use() {
        let x = Term::Var(format!("{UNIVERSAL_VAR}http://ex/x"));
        let f = Term::Formula(vec![
            [Term::Iri("http://ex/m".into()), Term::Iri("http://ex/r".into()), Term::Iri("http://ex/x".into())],
            [x, Term::Iri("http://ex/p".into()), Term::Iri("http://ex/b".into())],
        ]);
        assert_eq!(
            rendered(&f),
            "{ <http://ex/m> <http://ex/r> <http://ex/x> . @forAll <http://ex/x> . <http://ex/x> <http://ex/p> <http://ex/b> . }"
        );
    }

    /// The one character the parser can decode into an IRI but not read raw is `\`.
    #[test]
    fn iris_are_iriref_escaped() {
        assert_eq!(rendered(&Term::Iri("http://ex/a\\b".into())), "<http://ex/a\\u005Cb>");
        let dt = Term::Lit("1".into(), "http://ex/d\\t".into(), None);
        assert_eq!(rendered(&dt), "\"1\"^^<http://ex/d\\u005Ct>");
    }
}
