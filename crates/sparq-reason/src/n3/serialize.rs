//! N3 surface-syntax **serialization** — the crate's single writer for [`Term`]s, ground
//! statements, and (`sq-xqchl.2`) whole RULES.
//!
//! The term/statement writers moved here from `incremental` (which used them as the
//! fallback / differential-oracle path) so the rule writer below shares ONE definition of
//! how a term is rendered — a second copy is how a serializer and its parser drift apart.
//! The writers are split by construction (see `Unit`): the EXACT writers ([`write_term`],
//! [`write_statement`], [`serialize_facts`], [`write_rule`]) return text that parses back
//! to the same terms — blank labels verbatim, language tags already lowercase, plain strings
//! re-acquiring `xsd:string`, IRIs `IRIREF`-escaped — or a [`NotRepresentable`] error, never
//! a fallback spelling; the DISPLAY writers ([`display_lossy`], [`statement_display_lossy`])
//! always produce text, for people to read only. Identity comes from [`statement_keys`].
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
//!   `?__ua.<iri>` (`UNIVERSAL_VAR`), unforgeable for the same reason. Every declaration of
//!   one IRI, at document level or inside a formula, is that ONE variable: the engine treats
//!   a formula-level `@forAll` as document-scoped (a known limitation, GH #6754). So it is
//!   written BACK as that IRI under one DOCUMENT-level `@forAll <iri> .` line before the
//!   first unit that uses it, which re-parses to the same variable (GH #5391, GH #6701
//!   review). See `Unit` for the shapes the exact writers refuse. Only
//!   [`RuleVars::VarIris`], which grounds variables into `var:` IRIs and is not meant to be
//!   re-reasoned, names a universal by its local name.
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
/// reads `:x` as the variable `__ua.<full IRI of :x>`, in every scope: GH #6754). The `.`
/// makes it unforgeable exactly as for [`PREMISE_BLANK_VAR`] — no source quickvar can be
/// spelled with it, so none can collide with or capture it — and keying on the full IRI
/// keeps `@forAll a:x, b:x` two variables.
pub(super) const UNIVERSAL_VAR: &str = "__ua.";

/// `v` with the backward chainer's freshened-copy prefixes (`__bw<n>_`, possibly
/// repeated) stripped, when what remains is an `@forAll` universal.
fn universal_base(v: &str) -> Option<&str> {
    let mut rest = v;
    loop {
        if rest.starts_with(UNIVERSAL_VAR) {
            return Some(rest);
        }
        let r = rest.strip_prefix("__bw")?;
        let digits = r.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        rest = r[digits..].strip_prefix('_')?;
    }
}

/// An `@forAll` universal variable name taken apart: its declared IRI and its base name
/// (backward-chaining copy prefixes stripped). `None` for any other variable.
///
/// Besides the parser's own names this accepts the backward chainer's freshened copies
/// (`rename_vars` prefixes `__bw<n>_`): standardizing a rule apart must not strip a
/// universal of its provenance. Exact, not heuristic: the `.` cannot occur in a source
/// variable name.
pub(super) fn universal(v: &str) -> Option<(&str, &str)> {
    let base = universal_base(v)?;
    Some((base.strip_prefix(UNIVERSAL_VAR)?, base))
}

/// The declared IRI behind an `@forAll` universal variable name, or `None` for any other
/// variable ([`universal`]).
pub(super) fn universal_iri(v: &str) -> Option<&str> {
    universal(v).map(|u| u.0)
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

/// Every IRI `t` mentions plainly (as an IRI term, at any depth).
fn plain_iris<'a>(t: &'a Term, out: &mut BTreeSet<&'a str>) {
    match t {
        Term::Iri(i) => {
            out.insert(i);
        }
        Term::List(ms) => ms.iter().for_each(|m| plain_iris(m, out)),
        Term::Triple(tr) => tr.iter().for_each(|m| plain_iris(m, out)),
        Term::Formula(ts) => ts.iter().flatten().for_each(|m| plain_iris(m, out)),
        _ => {}
    }
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

/// Why a re-reasonable writer ([`write_term`], [`write_statement`], [`serialize_facts`],
/// [`write_rule`], [`crate::reason_n3_pass_all`]) refused: the input has no exact N3 form,
/// so any text written for it would re-parse — and re-reason — as something else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotRepresentable {
    /// A universal has no place for its document-level `@forAll` in this output (a lone
    /// term or rule), or that declaration would capture a plain mention of its IRI. See
    /// `Unit`.
    UnrepresentableScope(String),
    /// Two distinct variables would be written as one (a universal and its
    /// backward-chaining copy, or two copies, anywhere in the output).
    MergesVariables(String),
    /// A variable whose internal name has no N3 spelling.
    Unspellable(String),
    /// The written text re-parses as something else — the parser normalizes a term, reads
    /// the statement as a rule, or rejects it.
    Reparse(String),
}

impl NotRepresentable {
    fn message(&self) -> &str {
        match self {
            Self::UnrepresentableScope(m) | Self::MergesVariables(m) | Self::Unspellable(m) | Self::Reparse(m) => m,
        }
    }

    /// The same refusal, its message prefixed with what was being written.
    fn within(self, what: &str) -> Self {
        let m = format!("{what}: {}", self.message());
        match self {
            Self::UnrepresentableScope(_) => Self::UnrepresentableScope(m),
            Self::MergesVariables(_) => Self::MergesVariables(m),
            Self::Unspellable(_) => Self::Unspellable(m),
            Self::Reparse(_) => Self::Reparse(m),
        }
    }
}

impl std::fmt::Display for NotRepresentable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for NotRepresentable {}

/// Why one universal of a unit has no placement ([`Unit::place`]).
enum Unplaced {
    /// The output declares no `@forAll` for it before the unit (a lone term or rule).
    NoOwner,
    /// A plain mention of the IRI in the unit would be captured by the declaration.
    Captured,
}

/// Every naming decision for ONE top-level unit of output — a statement (fact), a rule, or
/// a lone term — made before anything is written. A unit is planned from its own terms
/// (plus, for a rule in a document's trailer, the document-level declarations it sits
/// under), so it renders the same whatever else is in the document.
///
/// # Two writers, split by construction
///
/// * The **re-reasonable** writers ([`write_term`], [`write_statement`],
///   [`serialize_facts`], [`write_rule`], `write_document`) plan with [`Unit::exact`]. Their
///   output parses back to the very same terms, variable names of universals included, or
///   they return [`NotRepresentable`]. There is no fallback spelling.
///   Exactness is CHECKED, not predicted: after the static planning checks, every exact
///   writer re-parses its WHOLE output (the whole document for [`serialize_facts`] and
///   `write_document`) and compares it to its input up to one bijection over variables
///   that spans every statement ([`same`]), so any normalisation the parser applies, and
///   any many-to-one variable mapping, is a refusal. The cost is one parse of the written
///   text.
/// * The **display** writers ([`display_lossy`], [`statement_display_lossy`]) plan with
///   [`Unit::lossy`] and always produce text, for diagnostics a person reads. Their output
///   is not fed back to a parser anywhere in the crate.
///
/// # Scoping universals: one document-level declaration
///
/// The engine reads every `@forAll` of one IRI as ONE variable, `__ua.<iri>`, whatever
/// scope declared it (a formula-level `@forAll` is treated as document-scoped: GH #6754).
/// So the exact writers declare each universal ONCE, on a document `@forAll <iri> .` line
/// right before the first unit (fact or rule) that uses it: `serialize_facts` keeps the
/// facts' order; `write_document` orders its units so that every plain mention of an IRI
/// comes before its universal's line. That re-parses to the same variable. [`write_term`] and [`write_rule`] write no document, so
/// they refuse a universal ([`NotRepresentable::UnrepresentableScope`]); so does a plain
/// mention of a declared IRI in that unit or any later one, which the line would capture.
///
/// The GATE is the re-parse: every exact writer re-parses its whole output and requires the
/// same terms up to one variable bijection under which every universal keeps its EXACT
/// internal name (`log:equalTo` compares names). A placement that would change anything is
/// a refusal. A backward-chaining copy of a universal (`__bw<n>___ua.<iri>`) can only be
/// spelled as `<iri>`, which reads back as the universal itself — a different term — so it
/// is refused ([`NotRepresentable::Unspellable`]).
///
/// Also refused: two distinct variables for one universal anywhere in the output (a
/// universal and its backward-chaining copy, or two copies:
/// [`NotRepresentable::MergesVariables`]); a variable whose internal name has no legal
/// spelling ([`NotRepresentable::Unspellable`]).
///
/// The display writers write every universal, throughout the unit, as a plain variable named the IRI's local name, then `_2`, `_3`, …
/// until it differs from every variable name in the unit (unspellable variables are
/// renamed the same way). Two different facts can therefore display alike, so identity —
/// provenance addressing — comes from [`statement_keys`], never from display strings.
struct Unit {
    /// Document-level universals (base names) declared OUTSIDE the unit, before it.
    outer: BTreeSet<String>,
    /// Universals (base names) written as a plain variable (display only) → that name.
    names: BTreeMap<String, String>,
    /// Non-universal variables whose internal name is not a legal variable name (display
    /// only) → their written name.
    renamed: BTreeMap<String, String>,
}

impl Unit {
    /// The universals of the unit made of `terms` that cannot be written under `outer` (the
    /// universals, by base name, whose document-level `@forAll` is already declared before
    /// the unit), by base name with IRI and why.
    fn unplaced(terms: &[&Term], outer: &BTreeSet<String>) -> Vec<(String, String, Unplaced)> {
        let mut unplaced = Vec::new();
        for base in vars_of(terms.iter().copied()) {
            let Some((iri, base)) = universal(base) else { continue };
            if !outer.contains(base) {
                unplaced.push((base.to_string(), iri.to_string(), Unplaced::NoOwner));
            } else if terms.iter().any(|t| mentions_iri(t, iri)) {
                unplaced.push((base.to_string(), iri.to_string(), Unplaced::Captured));
            }
        }
        unplaced.dedup_by(|a, b| a.0 == b.0);
        unplaced
    }

    /// Plan the unit made of `terms` for an EXACT writer, under the document-level
    /// declarations `outer`: the plan, or why no exact N3 form exists.
    fn exact(terms: &[&Term], outer: &BTreeSet<String>) -> Result<Unit, NotRepresentable> {
        let mut vars = BTreeSet::new();
        terms.iter().for_each(|t| all_vars(t, &mut vars));
        check_no_merge(vars.iter().copied())?;
        if let Some(v) = vars.iter().find(|v| universal_iri(v).is_none() && !spellable_var(v)) {
            return Err(NotRepresentable::Unspellable(format!("the variable `{v}` has no N3 spelling")));
        }
        // A backward-chaining copy (`__bw<n>___ua.<iri>`) is a DIFFERENT term from the
        // universal: N3 text can only spell it as `<iri>`, which reads back as `__ua.<iri>`
        // and so compares differently under `log:equalTo` (GH #6701 review round 13).
        if let Some((v, iri)) = vars.iter().find_map(|v| universal(v).filter(|u| u.1 != *v).map(|u| (v, u.0))) {
            return Err(NotRepresentable::Unspellable(format!(
                "the variable `{v}` is a backward-chaining copy of the @forAll universal <{iri}>; N3 text can only \
                 spell it as <{iri}>, which reads back as the universal itself, a different term"
            )));
        }
        if let Some((_, iri, why)) = Unit::unplaced(terms, outer).first() {
            return Err(NotRepresentable::UnrepresentableScope(match why {
                Unplaced::NoOwner => format!(
                    "the @forAll universal <{iri}> needs a document-level @forAll line, and this output is not \
                     a document that can declare it before the unit"
                ),
                Unplaced::Captured => format!(
                    "the document-level @forAll <{iri}> declaration would also capture a plain mention of <{iri}>"
                ),
            }));
        }
        Ok(Unit { outer: outer.clone(), names: BTreeMap::new(), renamed: BTreeMap::new() })
    }

    /// Plan the unit made of `terms` for DISPLAY: always succeeds; what [`Unit::exact`]
    /// refuses gets a collision-free plain-variable spelling unit-wide.
    fn lossy(terms: &[&Term]) -> Unit {
        let unplaced = Unit::unplaced(terms, &BTreeSet::new());
        let mut vars = BTreeSet::new();
        terms.iter().for_each(|t| all_vars(t, &mut vars));
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
        let names = unplaced.into_iter().map(|(base, iri, _)| (base, fresh(local_name(&iri)))).collect();
        let renamed = vars
            .iter()
            .filter(|v| universal_iri(v).is_none() && !spellable_var(v))
            .map(|v| (v.to_string(), fresh(sanitize_name(v))))
            .collect();
        Unit { outer: BTreeSet::new(), names, renamed }
    }

    /// Write `t`, a term at the unit's top level.
    fn term(&self, t: &Term, out: &mut String) {
        let active = self.outer.iter().filter_map(|b| universal(b).map(|(iri, _)| (iri.to_string(), b.clone()))).collect();
        self.scoped(t, &active, out);
    }

    /// Write `t`; `active` maps each IRI whose document-level `@forAll` is in effect to the
    /// universal (base name) it binds.
    fn scoped(&self, t: &Term, active: &BTreeMap<String, String>, out: &mut String) {
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
            Term::Var(v) => match universal(v) {
                Some((iri, base)) if active.get(iri).is_some_and(|b| b == base) => write_iriref(iri, out),
                Some((iri, base)) => match self.names.get(base) {
                    Some(name) => {
                        out.push('?');
                        out.push_str(name);
                    }
                    // Not reachable from an exact plan; written as its IRI, it would re-parse
                    // as something else and the exactness check would refuse.
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
                    self.scoped(m, active, out);
                }
                out.push_str(" )");
            }
            Term::Formula(ts) => {
                out.push('{');
                for row in ts {
                    for t in row {
                        out.push(' ');
                        self.scoped(t, active, out);
                    }
                    out.push_str(" .");
                }
                out.push_str(" }");
            }
            // RDF-star quoted-triple term — round-trips through the N3 parser's
            // `<< s p o >>` form (GH #2012). [FABLE-5]
            Term::Triple(tr) => {
                out.push_str("<< ");
                self.scoped(&tr[0], active, out);
                out.push(' ');
                self.scoped(&tr[1], active, out);
                out.push(' ');
                self.scoped(&tr[2], active, out);
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

/// Refuse when two distinct variable names among `vars` are the same universal (a
/// universal and its backward-chaining copy, or two copies): N3 text spells every one of
/// them as the universal's IRI under its one declaration, which would merge them.
fn check_no_merge<'a>(vars: impl Iterator<Item = &'a str>) -> Result<(), NotRepresentable> {
    let mut by_base: BTreeMap<&str, &str> = BTreeMap::new();
    for v in vars {
        if let Some((iri, base)) = universal(v) {
            if let Some(other) = by_base.insert(base, v) {
                if other != v {
                    return Err(NotRepresentable::MergesVariables(format!(
                        "the variables `{other}` and `{v}` are distinct, but both are the @forAll universal <{iri}> \
                         (backward-chaining copies) and N3 text can only spell either as <{iri}>, which would \
                         merge them"
                    )));
                }
            }
        }
    }
    Ok(())
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

fn write_declarations<'a>(iris: impl Iterator<Item = &'a str>, out: &mut String) {
    out.push_str("@forAll");
    for (i, iri) in iris.enumerate() {
        out.push_str(if i == 0 { " " } else { ", " });
        write_iriref(iri, out);
    }
    out.push_str(" .");
}

/// A lossless, injective key for one term: two keys are equal EXACTLY when the terms are
/// equal under `Term`'s own (derived) equality — the identity the engine uses for facts,
/// hashing, `log:equalTo` and formula unification. One tagged, prefix-free, structural
/// encoding of EVERY field — `I` IRI, `L` literal (lexical form, datatype, then `@` + tag
/// or `-` for none), `B` blank, `V` variable (full internal name), `(…)` list, `{…;}`
/// formula (rows in their order, duplicates kept), `<…>` quoted triple — with each string
/// quoted and its `"` and `\` escaped. Never a rendering: the display writer drops fields
/// (a language-tagged literal's datatype) and spells different variables alike.
///
/// No normalisation the engine does not do: a formula is an ORDERED vector of rows to the
/// engine, so two formulae with the same rows in another order, or with a duplicated row,
/// are different facts and get different keys. Blank-node labels and backward-chaining
/// copies (`__bw<n>___ua.<iri>`) are likewise kept as the distinct terms they are. The
/// key itself uses no interning ids, pointers, hashes, locale or number formatting, but it is
/// only as reproducible as the TERMS: it inherits the engine's traversal-order dependence.
/// An existential's skolem label (`__sk<n>_<label>`) is allocated in hash-set delta order,
/// and `log:conclusion` appends derived rows in that order, so the same input can key
/// differently across runs or platforms (as `why()` strings already do on `main`; GH #6749).
fn term_key(t: &Term) -> String {
    fn quoted(s: &str, out: &mut String) {
        out.push('"');
        for c in s.chars() {
            if matches!(c, '"' | '\\') {
                out.push('\\');
            }
            out.push(c);
        }
        out.push('"');
    }
    fn enc(t: &Term, out: &mut String) {
        match t {
            Term::Iri(i) => {
                out.push('I');
                quoted(i, out);
            }
            Term::Lit(v, dt, lang) => {
                out.push('L');
                quoted(v, out);
                quoted(dt, out);
                match lang {
                    Some(l) => {
                        out.push('@');
                        quoted(l, out);
                    }
                    None => out.push('-'),
                }
            }
            Term::Blank(b) => {
                out.push('B');
                quoted(b, out);
            }
            Term::Var(v) => {
                out.push('V');
                quoted(v, out);
            }
            Term::List(ms) => {
                out.push('(');
                ms.iter().for_each(|m| enc(m, out));
                out.push(')');
            }
            Term::Formula(ts) => {
                out.push('{');
                for row in ts {
                    row.iter().for_each(|m| enc(m, out));
                    out.push(';');
                }
                out.push('}');
            }
            Term::Triple(tr) => {
                out.push('<');
                tr.iter().for_each(|m| enc(m, out));
                out.push('>');
            }
        }
    }
    let mut s = String::new();
    enc(t, &mut s);
    s
}

/// The identity keys of one statement's three terms (`term_key`): equal exactly when the
/// statements are equal, whatever their renderings. What a proof carries for provenance
/// addressing (`ProofNode::key`).
pub fn statement_keys(f: &[Term; 3]) -> [String; 3] {
    #[cfg(test)]
    KEY_CALLS.with(|c| c.set(c.get() + 1));
    [term_key(&f[0]), term_key(&f[1]), term_key(&f[2])]
}

#[cfg(test)]
thread_local! {
    /// How many statements this thread has keyed — lets tests check that the plain
    /// (unkeyed) reasoning paths never pay for proof identity.
    pub(crate) static KEY_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// `t` as its written text re-parses: a backward-chaining copy of a universal
/// (`__bw<n>___ua.<iri>`) reads back as the universal itself — a change, used only to name
/// the refusal (the exact writers refuse copies before writing).
fn as_reparsed(t: &Term) -> Term {
    match t {
        Term::Var(v) => match universal(v) {
            Some((_, base)) => Term::Var(base.to_string()),
            None => t.clone(),
        },
        Term::List(ms) => Term::List(ms.iter().map(as_reparsed).collect()),
        Term::Triple(tr) => Term::Triple(Box::new([as_reparsed(&tr[0]), as_reparsed(&tr[1]), as_reparsed(&tr[2])])),
        Term::Formula(ts) => Term::Formula(ts.iter().map(|r| r.clone().map(|m| as_reparsed(&m))).collect()),
        _ => t.clone(),
    }
}

/// A one-to-one pairing of names (variables, or the blank nodes of one formula) between the
/// terms a writer was given and the terms its output re-parses to.
#[derive(Default)]
struct Bijection<'a> {
    fwd: HashMap<&'a str, &'a str>,
    back: HashMap<&'a str, &'a str>,
}

impl<'a> Bijection<'a> {
    /// Pair `w` (given) with `g` (re-parsed): true if consistent with every pair so far in
    /// BOTH directions — so two distinct names never meet one name, and one name never
    /// splits into two.
    fn pair(&mut self, w: &'a str, g: &'a str) -> bool {
        match (self.fwd.get(w), self.back.get(g)) {
            (None, None) => {
                self.fwd.insert(w, g);
                self.back.insert(g, w);
                true
            }
            (Some(&x), Some(&y)) => x == g && y == w,
            _ => false,
        }
    }
}

/// Is `g` (re-parsed) the term `w` (given), up to a BIJECTIVE renaming — never by name?
///
/// * Variables pair one-to-one across the whole unit (an N3 variable is quantified in the
///   outermost formula), and a universal (or a backward-chaining copy of one) must read
///   back under its EXACT internal name — that name is what `log:equalTo` and other
///   documents' formulae compare against — while a non-universal stays a non-universal.
/// * Blank nodes pair one-to-one within each formula, the scope a blank node has in N3 —
///   each `{ … }` starts a fresh pairing — and within the unit's top level.
/// * Everything else must be equal.
fn same<'a>(w: &'a Term, g: &'a Term, vars: &mut Bijection<'a>, blanks: &mut Bijection<'a>) -> bool {
    match (w, g) {
        // A universal must come back under its EXACT internal name (`log:equalTo` compares
        // names): no renumbering, and no copy prefix stripped (GH #6701 review round 13).
        (Term::Var(a), Term::Var(b)) if universal(a).is_some() || universal(b).is_some() => a == b && vars.pair(a, b),
        (Term::Var(a), Term::Var(b)) => vars.pair(a, b),
        (Term::Blank(a), Term::Blank(b)) => blanks.pair(a, b),
        (Term::List(a), Term::List(b)) => a.len() == b.len() && a.iter().zip(b).all(|(x, y)| same(x, y, vars, blanks)),
        (Term::Triple(a), Term::Triple(b)) => a.iter().zip(b.iter()).all(|(x, y)| same(x, y, vars, blanks)),
        (Term::Formula(a), Term::Formula(b)) => {
            let mut inner = Bijection::default();
            a.len() == b.len()
                && a.iter().zip(b).all(|(r, q)| r.iter().zip(q).all(|(x, y)| same(x, y, vars, &mut inner)))
        }
        _ => w == g,
    }
}

/// [`same`] over parallel term sequences — a whole output unit or document, in written order
/// — sharing ONE variable bijection and one top-level blank pairing across all of them; on a
/// mismatch, why.
fn same_terms(want: &[Term], got: &[Term]) -> Result<(), NotRepresentable> {
    let (mut vars, mut blanks) = (Bijection::default(), Bijection::default());
    if want.len() == got.len() && want.iter().zip(got).all(|(w, g)| same(w, g, &mut vars, &mut blanks)) {
        return Ok(());
    }
    let normalized: Vec<Term> = want.iter().map(as_reparsed).collect();
    if normalized == got {
        return Err(NotRepresentable::MergesVariables(
            "writing it would merge distinct variables into one (a universal and its backward-chaining \
             copy, or two copies, all spell as the same @forAll IRI)"
                .into(),
        ));
    }
    let k = (0..want.len().min(got.len())).find(|&k| normalized[k] != got[k]).unwrap_or(0);
    Err(NotRepresentable::Reparse(match (normalized.get(k), got.get(k)) {
        (Some(w), Some(g)) => format!(
            "the term {} re-parses as {} (the N3 parser normalizes it, or reads it under another scope)",
            shown(w, g),
            shown(g, w)
        ),
        _ => format!("its N3 form re-parses as {} terms, not {}", got.len(), want.len()),
    }))
}

/// One rule as written into a document: its two sides in written order, and its arrow.
type RuleOut = ([Term; 2], RuleKind);

/// The exactness CHECK every exact writer runs on its WHOLE output: parse `text` back with
/// the crate's N3 parser and require exactly the statements `facts` (in written order) and
/// the rules `rules` (in written order), up to ONE bijective renaming of variables across
/// all of them ([`same`]). This is what makes "exact or refused" hold by construction
/// rather than by enumerating the parser's normalisations (a lowercased language tag,
/// `rdf:nil` read as `()`, a formula-`log:implies`-formula statement read as a rule, an IRI
/// the `IRIREF` grammar rejects, a universal read under a different `@forAll`, …): whatever
/// the parser would change, the writer refuses. Because the bijection spans the document,
/// two variables that are distinct in different statements (`__bw0___ua.x` in one,
/// `__bw1___ua.x` in another) can never both read back as one.
fn verify_document(
    text: &str,
    facts: &[&[Term; 3]],
    rules: &[RuleOut],
    cuts: &super::bounded::Cuts,
) -> Result<(), NotRepresentable> {
    // Re-read under the caller's run record: a nesting cut while re-reading is a refusal
    // like any other.
    let p = super::bounded::parse_n3(text, "", cuts)
        .map_err(|e| NotRepresentable::Reparse(format!("its N3 form does not re-parse ({e})")))?;
    let (fwd, bwd): (Vec<&RuleOut>, Vec<&RuleOut>) = rules.iter().partition(|r| r.1 == RuleKind::Forward);
    if p.facts.len() != facts.len() || p.rules.len() != fwd.len() || p.backward_rules.len() != bwd.len() {
        return Err(NotRepresentable::Reparse(format!(
            "its N3 form re-parses as {} statements, {} forward and {} backward rules, not {}, {} and {} (a \
             formula related by log:implies / log:isImpliedBy is a rule in N3 text)",
            p.facts.len(),
            p.rules.len(),
            p.backward_rules.len(),
            facts.len(),
            fwd.len(),
            bwd.len()
        )));
    }
    let mut want: Vec<Term> = Vec::new();
    let mut got: Vec<Term> = Vec::new();
    for (w, g) in facts.iter().zip(&p.facts) {
        want.extend(w.iter().cloned());
        got.extend(g.iter().cloned());
    }
    for (ws, gs, kind) in [(&fwd, &p.rules, RuleKind::Forward), (&bwd, &p.backward_rules, RuleKind::Backward)] {
        for (w, g) in ws.iter().zip(gs) {
            want.extend(w.0.iter().cloned());
            got.extend(rule_sides(g, kind, RuleVars::N3));
        }
    }
    same_terms(&want, &got)
}

/// The subject/predicate a lone term is checked under ([`write_term`]).
const CHECK_IRI: &str = "urn:sparq:serialize:check";

/// Write one N3 term in its surface syntax: IRIs `<…>`, literals `"lex"` (+ `@lang` /
/// `^^<dt>`, `xsd:string` left implicit), blanks `_:l`, variables `?v`, lists `( … )`,
/// formulae `{ … }`, RDF-star quoted triples `<< s p o >>`.
///
/// The term is its own unit. Exact: the text parses back to `t` — the writer re-parses its
/// own output to check — or this returns [`NotRepresentable`] and writes nothing (see
/// `Unit`). The check costs one parse of the written text.
pub fn write_term(t: &Term, out: &mut String) -> Result<(), NotRepresentable> {
    let mut s = String::new();
    Unit::exact(&[t], &BTreeSet::new())?.term(t, &mut s);
    let k = Term::Iri(CHECK_IRI.into());
    verify_document(&format!("<{CHECK_IRI}> <{CHECK_IRI}> {s} ."), &[&[k.clone(), k, t.clone()]], &[], &super::bounded::Cuts::top_level())?;
    out.push_str(&s);
    Ok(())
}

/// A term as N3-like text for a person to READ — a diagnostic, a proof-node string. Never
/// fails, and never exact: a universal no `@forAll` can scope is shown as a plain variable
/// (see `Unit`). Not for anything a parser or the reasoner reads back; use
/// [`write_term`] for that, and [`statement_keys`] for identity.
pub fn display_lossy(t: &Term) -> String {
    let mut s = String::new();
    Unit::lossy(&[t]).term(t, &mut s);
    s
}

/// Write one statement as `s p o .` plus a newline — its own unit. Exact, like
/// [`write_term`] (re-parsed and compared before it is written): on [`NotRepresentable`]
/// nothing is written.
pub fn write_statement(f: &[Term; 3], out: &mut String) -> Result<(), NotRepresentable> {
    out.push_str(&serialize_facts(std::iter::once(f))?);
    Ok(())
}

/// The three terms of one statement rendered as that ONE unit for a person to read — the
/// strings a proof node shows ([`display_lossy`] semantics: never fails, not exact). A
/// function of the statement alone, so a fact reads the same in every proof; for identity
/// use [`statement_keys`].
pub fn statement_display_lossy(f: &[Term; 3]) -> [String; 3] {
    let unit = Unit::lossy(&[&f[0], &f[1], &f[2]]);
    // Index the borrowed terms — the plan is keyed by their formulae's addresses.
    [0, 1, 2].map(|k| {
        let mut s = String::new();
        unit.term(&f[k], &mut s);
        s
    })
}

/// Plan and write one statement of a document as its own unit, under the document-level
/// universals `outer` declared before it (not yet verified).
fn statement_text(f: &[Term; 3], outer: &BTreeSet<String>) -> Result<String, NotRepresentable> {
    let mut s = String::new();
    Unit::exact(&[&f[0], &f[1], &f[2]], outer).map_err(|e| in_statement(f, e))?.statement(f, &mut s);
    Ok(s)
}

/// The universals (base names) in `terms` — each declared at document level.
fn document_universals<'a>(terms: impl Iterator<Item = &'a Term>) -> BTreeSet<String> {
    vars_of(terms).into_iter().filter_map(universal).map(|(_, base)| base.to_string()).collect()
}

/// The document-level `@forAll` declarations of one output document, written lazily: each
/// universal gets its line right before the first unit that uses it, and every unit after that line is checked for a plain mention of its IRI, which the
/// line would capture.
#[derive(Default)]
struct DocumentBinders {
    declared: BTreeSet<String>,
}

impl DocumentBinders {
    /// Before writing the unit made of `terms`: declare its new document-level universals,
    /// refuse a plain mention of any declared IRI, and return the universals in effect.
    fn before<'a>(&mut self, terms: impl Iterator<Item = &'a Term> + Clone, out: &mut String) -> Result<BTreeSet<String>, NotRepresentable> {
        let new: BTreeSet<String> = document_universals(terms.clone()).difference(&self.declared).cloned().collect();
        if !new.is_empty() {
            write_declarations(new.iter().filter_map(|b| universal_iri(b)), out);
            out.push('\n');
            self.declared.extend(new);
        }
        for iri in self.declared.iter().filter_map(|b| universal_iri(b)) {
            if terms.clone().any(|t| mentions_iri(t, iri)) {
                return Err(NotRepresentable::UnrepresentableScope(format!(
                    "the document-level @forAll <{iri}> declaration would capture a plain mention of <{iri}> \
                     written after it"
                )));
            }
        }
        Ok(self.declared.clone())
    }
}

/// Every variable name in `terms`.
fn vars_of<'a>(terms: impl Iterator<Item = &'a Term>) -> BTreeSet<&'a str> {
    let mut vars = BTreeSet::new();
    terms.for_each(|t| all_vars(t, &mut vars));
    vars
}

/// Serialize facts back to N3, one statement per line in the given order, each its own
/// unit; each universal gets its document `@forAll` line right before the first fact
/// that uses it. Exact: the WHOLE text is re-parsed and compared to the facts under
/// one variable bijection spanning every statement (`Unit`); the
/// first refusal fails the whole call.
pub fn serialize_facts<'a>(facts: impl Iterator<Item = &'a [Term; 3]>) -> Result<String, NotRepresentable> {
    let facts: Vec<&[Term; 3]> = facts.collect();
    check_no_merge(vars_of(facts.iter().flat_map(|f| f.iter())).into_iter())?;
    let mut binders = DocumentBinders::default();
    let mut out = String::new();
    for f in &facts {
        let outer = binders.before(f.iter(), &mut out).map_err(|e| in_statement(f, e))?;
        out.push_str(&statement_text(f, &outer)?);
    }
    verify_document(&out, &facts, &[], &super::bounded::Cuts::top_level())?;
    Ok(out)
}

/// `e`, prefixed with the statement it was raised for (shown via [`statement_display_lossy`]).
fn in_statement(f: &[Term; 3], e: NotRepresentable) -> NotRepresentable {
    let [s, p, o] = statement_display_lossy(f);
    e.within(&format!("cannot write `{s} {p} {o} .` as N3 that re-parses to it"))
}

/// Write one output DOCUMENT: closure `facts` and `rules` — the `--pass-all` layout
/// ([`crate::reason_n3_pass_all`]). Every statement and rule is its own unit, so none
/// renders differently for its neighbours. Each universal gets a document `@forAll` line
/// right before the first unit that uses it, so every unit that mentions its IRI PLAINLY
/// must come before every unit that uses the universal: units are laid out in a stable
/// topological order of those constraints, ties broken by the plain layout (facts sorted
/// by their text — deterministic output — then rules in order). Refused only when no order
/// exists: one unit mixing a plain mention and the universal, or a cycle. Exact: the whole
/// document is re-parsed and compared; any statement or rule with no lossless form fails
/// the document.
pub(super) fn write_document(
    facts: &[&[Term; 3]],
    rules: &[(&Rule, RuleKind)],
    vars: RuleVars,
    cuts: &super::bounded::Cuts,
    out: &mut String,
) -> Result<(), NotRepresentable> {
    let sides: Vec<RuleOut> = rules.iter().map(|(r, kind)| (rule_sides(r, *kind, vars), *kind)).collect();
    check_no_merge(vars_of(facts.iter().flat_map(|f| f.iter()).chain(sides.iter().flat_map(|r| r.0.iter()))).into_iter())?;
    // The units in their plain layout (sorted facts, then rules), each with its terms.
    let mut lines = Vec::with_capacity(facts.len());
    for f in facts {
        lines.push((statement_text(f, &document_universals(f.iter()))?, *f));
    }
    lines.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    let units: Vec<Vec<&Term>> =
        lines.iter().map(|(_, f)| f.iter().collect()).chain(sides.iter().map(|r| r.0.iter().collect())).collect();
    let refuse = |k: usize, e: NotRepresentable| match k.checked_sub(lines.len()) {
        None => in_statement(lines[k].1, e),
        Some(r) => in_rule(&sides[r].0, sides[r].1, e),
    };
    // Constraints: every unit mentioning an IRI plainly comes before every unit using its
    // universal. One gate node per such IRI (plain units → gate → universal units) keeps the
    // graph linear in the document's size.
    let mut users: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (j, u) in units.iter().enumerate() {
        for v in vars_of(u.iter().copied()) {
            if let Some((iri, _)) = universal(v) {
                let js = users.entry(iri).or_default();
                if js.last() != Some(&j) {
                    js.push(j);
                }
            }
        }
    }
    let gates: BTreeMap<&str, usize> = users.keys().enumerate().map(|(g, iri)| (*iri, units.len() + g)).collect();
    let mut after: Vec<Vec<usize>> = vec![Vec::new(); units.len() + gates.len()];
    let mut blockers = vec![0usize; after.len()];
    for (iri, js) in &users {
        for &j in js {
            after[gates[iri]].push(j);
            blockers[j] += 1;
        }
    }
    for (k, u) in units.iter().enumerate() {
        let mut plain = BTreeSet::new();
        u.iter().for_each(|t| plain_iris(t, &mut plain));
        for iri in plain.into_iter().filter(|i| gates.contains_key(i)) {
            if users[iri].binary_search(&k).is_ok() {
                return Err(refuse(k, NotRepresentable::UnrepresentableScope(format!(
                    "the document-level @forAll <{iri}> declaration would also capture a plain mention of <{iri}>"
                ))));
            }
            after[k].push(gates[iri]);
            blockers[gates[iri]] += 1;
        }
    }
    // Kahn's algorithm, always taking the earliest ready unit of the plain layout (a gate
    // opens as soon as it is ready).
    let mut ready: BTreeSet<usize> = (0..after.len()).filter(|&k| blockers[k] == 0).collect();
    let mut order = Vec::with_capacity(units.len());
    while let Some(k) = ready.range(units.len()..).next().copied().or_else(|| ready.first().copied()) {
        ready.remove(&k);
        if k < units.len() {
            order.push(k);
        }
        for &j in &after[k] {
            blockers[j] -= 1;
            if blockers[j] == 0 {
                ready.insert(j);
            }
        }
    }
    if order.len() < units.len() {
        let k = (0..units.len()).find(|k| blockers[*k] > 0).unwrap_or(0);
        return Err(refuse(k, NotRepresentable::UnrepresentableScope(
            "its @forAll universals and plain mentions of their IRIs form a cycle across units: no order of the \
             document puts every plain mention before the universal's declaration"
                .into(),
        )));
    }
    let mut binders = DocumentBinders::default();
    let mut doc = String::new();
    let (mut fact_order, mut rule_order): (Vec<&[Term; 3]>, Vec<RuleOut>) = (Vec::new(), Vec::new());
    for k in order {
        match k.checked_sub(lines.len()) {
            None => {
                let (text, f) = &lines[k];
                binders.before(f.iter(), &mut doc).map_err(|e| in_statement(f, e))?;
                doc.push_str(text);
                fact_order.push(f);
            }
            Some(r) => {
                let (s, kind) = &sides[r];
                let outer = binders.before(s.iter(), &mut doc).map_err(|e| in_rule(s, *kind, e))?;
                let unit = Unit::exact(&[&s[0], &s[1]], &outer).map_err(|e| in_rule(s, *kind, e))?;
                write_rule_sides(&unit, s, *kind, &mut doc);
                rule_order.push((s.clone(), *kind));
            }
        }
    }
    verify_document(&doc, &fact_order, &rule_order, cuts)?;
    out.push_str(&doc);
    Ok(())
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
/// a `RuleVars::N3` round trip yields the same rule. Exact, like [`write_term`] (the
/// written rule is re-parsed and compared before it is written): a rule that would change
/// on re-parse is [`NotRepresentable`], and nothing is written. That includes a rule whose
/// premise and conclusion share an `@forAll` universal
/// ([`NotRepresentable::UnrepresentableScope`]): only a document-level declaration scopes
/// both sides, and a lone rule has no document to put it in — `write_document` (used by
/// [`crate::reason_n3_pass_all`]) writes that shape (see `Unit`).
pub fn write_rule(r: &Rule, kind: RuleKind, vars: RuleVars, out: &mut String) -> Result<(), NotRepresentable> {
    let sides = rule_sides(r, kind, vars);
    check_no_merge(vars_of(sides.iter()).into_iter()).map_err(|e| in_rule(&sides, kind, e))?;
    let unit = Unit::exact(&[&sides[0], &sides[1]], &BTreeSet::new()).map_err(|e| in_rule(&sides, kind, e))?;
    let mut s = String::new();
    write_rule_sides(&unit, &sides, kind, &mut s);
    verify_document(&s, &[], &[(sides.clone(), kind)], &super::bounded::Cuts::top_level()).map_err(|e| in_rule(&sides, kind, e))?;
    out.push_str(&s);
    Ok(())
}

/// `e`, prefixed with the rule it was raised for (shown via a display plan).
fn in_rule(sides: &[Term; 2], kind: RuleKind, e: NotRepresentable) -> NotRepresentable {
    let lossy = Unit::lossy(&[&sides[0], &sides[1]]);
    let mut shown = String::new();
    write_rule_sides(&lossy, sides, kind, &mut shown);
    e.within(&format!("cannot write the rule `{}` as N3 that re-parses to it", shown.trim_end()))
}

/// `t` for a refusal message: its display form, plus its full structure when that display
/// form is the same as `other`'s (they differ in a field the surface syntax does not show,
/// such as a language-tagged literal's datatype).
fn shown(t: &Term, other: &Term) -> String {
    let d = display_lossy(t);
    if d == display_lossy(other) { format!("`{d}` ({t:?})") } else { format!("`{d}`") }
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
        write_term(t, &mut s).expect("exactly representable");
        s
    }

    const LANG_STRING: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString";

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
        let tagged = Term::Lit("hi".into(), LANG_STRING.into(), Some("en".into()));
        assert_eq!(rendered(&tagged), "\"hi\"@en");
        // Shapes the parser would normalize are refused, not written (GH #6701 round 8).
        let mut out = String::new();
        for odd in [
            Term::Lit("hi".into(), XSD_STRING.into(), Some("en".into())), // tag + non-langString
            Term::Lit("hi".into(), LANG_STRING.into(), Some("EN".into())), // uppercase tag
            Term::Lit("hi".into(), LANG_STRING.into(), Some("e n".into())), // invalid tag
            Term::Iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#nil".into()), // reads as `()`
            Term::Iri("http://ex/a b".into()), // the IRIREF grammar rejects it
        ] {
            assert!(write_term(&odd, &mut out).is_err(), "{odd:?}");
        }
        assert_eq!(out, "");
        assert_eq!(rendered(&Term::List(vec![Term::Var("x".into())])), "( ?x )");
        // The empty formula IS the literal `true` to the parser, so it has no exact form.
        assert!(write_term(&Term::Formula(vec![]), &mut String::new()).is_err());
        assert_eq!(display_lossy(&Term::Formula(vec![])), "{ }");
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
            Term::Var("__ua.http://ex/x".to_string()),
            Term::Iri("http://ex/q".into()),
            Term::Var("x".into()),
        ]])
    }

    /// A universal is written as its own IRI under ONE document-level `@forAll`, so it can
    /// never merge with a source variable and re-parses to the same term (GH #6701).
    #[test]
    fn a_universal_is_written_under_a_document_declaration() {
        let k = Term::Iri("http://ex/k".into());
        let f = [k.clone(), k.clone(), universal_formula()];
        let doc = serialize_facts(std::iter::once(&f)).expect("exact");
        assert_eq!(doc, "@forAll <http://ex/x> .\n<http://ex/k> <http://ex/k> { <http://ex/x> <http://ex/q> ?x . } .\n");
        let back = super::super::parser::parse(&doc).expect("re-parses");
        assert_eq!(back.facts[0], f);
    }

    /// A lone term has no document to declare a universal in, and a declaration would
    /// capture a plain mention of the IRI in its unit: the exact writers refuse both. The
    /// display writer names the universal collision-free against every variable of the
    /// unit — here `?x` and `?x_2` are taken.
    #[test]
    fn an_undeclarable_universal_is_refused_and_only_displayed() {
        let x = Term::Var("__ua.http://ex/x".to_string());
        let both = Term::Formula(vec![
            [x.clone(), Term::Iri("http://ex/x".into()), Term::Var("x".into())],
            [Term::Var("x_2".into()), Term::Iri("http://ex/q".into()), x.clone()],
        ]);
        let mut out = String::new();
        assert!(write_term(&both, &mut out).is_err());
        assert!(write_term(&x, &mut out).is_err());
        assert!(write_term(&universal_formula(), &mut out).is_err(), "no document to declare it in");
        assert!(write_term(&Term::Var("a.b".into()), &mut out).is_err(), "unspellable");
        assert_eq!(out, "", "a refused write writes nothing");
        let k = Term::Iri("http://ex/k".into());
        let captured = [k.clone(), k.clone(), both.clone()];
        assert!(matches!(serialize_facts(std::iter::once(&captured)), Err(NotRepresentable::UnrepresentableScope(_))));
        assert_eq!(display_lossy(&both), "{ ?x_3 <http://ex/x> ?x . ?x_2 <http://ex/q> ?x_3 . }");
        assert_eq!(display_lossy(&x), "?x");
        // Freshened backward-rule copies keep their provenance.
        assert_eq!(universal_iri("__bw3___bw0___ua.http://ex/x"), Some("http://ex/x"));
        assert_eq!(universal_iri("__bw3_x"), None);
        assert_eq!(display_lossy(&Term::Var("__bw0___ua.http://ex/x".into())), display_lossy(&x));
        // … but no exact writer writes a copy: its only spelling reads back as the universal.
        let copy = [k.clone(), k.clone(), Term::Formula(vec![[Term::Var("__bw0___ua.http://ex/x".into()), k.clone(), k.clone()]])];
        assert!(matches!(serialize_facts(std::iter::once(&copy)), Err(NotRepresentable::Unspellable(_))));
    }

    /// GH #6701 review round 9: the re-parse comparison is a BIJECTION, never by name —
    /// variables one-to-one across the unit, blank nodes one-to-one within each formula.
    #[test]
    fn the_reparse_check_is_a_bijection() {
        let v = |s: &str| Term::Var(s.into());
        let b = |s: &str| Term::Blank(s.into());
        let k = Term::Iri("http://ex/k".into());
        let f = |rows: Vec<[Term; 3]>| Term::Formula(rows);
        let (ua, c0) = (v("__ua.http://ex/x"), v("__bw0___ua.http://ex/x"));
        // A universal and its copy read back as ONE variable: a merge, refused.
        let e = same_terms(&[f(vec![[ua.clone(), k.clone(), c0.clone()]])], &[f(vec![[ua.clone(), k.clone(), ua.clone()]])]);
        assert!(matches!(e, Err(NotRepresentable::MergesVariables(_))));
        // A lone copy reading back as the universal changes its name: refused too (round 13).
        assert!(same_terms(&[f(vec![[c0.clone(), k.clone(), k.clone()]])], &[f(vec![[ua.clone(), k.clone(), k.clone()]])]).is_err());
        // A universal never becomes a plain variable, nor another IRI's universal.
        assert!(same_terms(std::slice::from_ref(&ua), &[v("x")]).is_err());
        assert!(same_terms(std::slice::from_ref(&ua), &[v("__ua.http://ex/y")]).is_err());
        // Two variables never split from one, nor merge.
        assert!(same_terms(&[v("a"), v("a")], &[v("p"), v("q")]).is_err());
        assert!(same_terms(&[v("a"), v("b")], &[v("p"), v("p")]).is_err());
        // Blank nodes: one-to-one within a formula …
        let two = f(vec![[b("a"), k.clone(), b("b")]]);
        assert!(same_terms(std::slice::from_ref(&two), &[f(vec![[b("c"), k.clone(), b("c")]])]).is_err());
        assert!(same_terms(std::slice::from_ref(&two), &[f(vec![[b("c"), k.clone(), b("d")]])]).is_ok());
        // … and scoped to it: each formula pairs its own blanks.
        let a1 = f(vec![[b("a"), k.clone(), k.clone()]]);
        assert!(same_terms(&[a1.clone(), a1.clone()], &[f(vec![[b("x"), k.clone(), k.clone()]]), f(vec![[b("y"), k.clone(), k.clone()]])]).is_ok());
    }

    /// The one character the parser can decode into an IRI but not read raw is `\`.
    #[test]
    fn iris_are_iriref_escaped() {
        assert_eq!(rendered(&Term::Iri("http://ex/a\\b".into())), "<http://ex/a\\u005Cb>");
        let dt = Term::Lit("1".into(), "http://ex/d\\t".into(), None);
        assert_eq!(rendered(&dt), "\"1\"^^<http://ex/d\\u005Ct>");
    }
}
