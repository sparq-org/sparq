//! A focused Notation3 parser — the subset needed for rule reasoning (EYE-style).
//!
//! oxttl parses Turtle/N-Triples but NOT N3's rule/formula extensions, so we hand-roll a
//! recursive-descent parser over the common N3 surface:
//! - `@prefix`/`@base`, prefixed names, `<iri>`, `a` (= rdf:type)
//! - literals: `"s"`, `"s"^^<dt>`, `"s"@lang`, integers, decimals, doubles, `true`/`false`
//! - `_:blank`, `?var` (universally-quantified N3 variables)
//! - `{ … }` formulae (graph terms), `( … )` collections (RDF lists)
//! - `<< s p o >>` / `<<( s p o )>>` RDF-star quoted-triple TERMS (nested,
//!   variables anywhere; both spellings are one [`Term::Triple`] value)
//! - predicate sugar `=>` (log:implies), `<=` (reverse implies), `=` (owl:sameAs),
//!   `is EXPR of` (inverse predicate) and `has EXPR`
//! - paths (`!`/`^`, also in predicate position), inverted predicates (`<-`),
//!   relative-IRI resolution against a document base (RFC 3986 incl. dot
//!   segments and query-only references), cwm's undeclared-`:`-prefix-as-`<#>`
//!   and well-known-prefix conventions
//! - `@forAll`/`@forSome` quantifiers (document and formula scope), `@keywords`
//!   (bare words as `:word`; the explicit `@a`/`@true`/`@false` forms),
//!   `[id :s …]` iriPropertyLists, zero-predicate statements, `{}` = `true`
//! - statement structure with `;` (predicate lists) and `,` (object lists)
//!
//! [`parse_turtle_with_base`] runs the same parser as STRICT W3C TURTLE:
//! every N3-only construct is rejected, statements must be `.`-terminated,
//! prefixes must be declared, and PN_LOCAL/LANGTAG/escape rules are enforced
//! (the TurtleTests suite passes 297/297 in this mode).

use super::model::{Rule, Term};
use super::serialize::PREMISE_BLANK_VAR;

pub const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
pub const RDF_NIL: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#nil";
pub const RDF_FIRST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#first";
pub const RDF_REST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#rest";
pub const LOG_IMPLIES: &str = "http://www.w3.org/2000/10/swap/log#implies";
/// The modern N3 CG spelling; `<=` parses to this.
pub const LOG_IMPLIED_BY: &str = "http://www.w3.org/2000/10/swap/log#isImpliedBy";
/// cwm's historical spelling — accepted on input, normalized to [`LOG_IMPLIED_BY`].
pub const LOG_IMPLIED_BY_LEGACY: &str = "http://www.w3.org/2000/10/swap/log#impliedBy";
pub const OWL_SAME_AS: &str = "http://www.w3.org/2002/07/owl#sameAs";
pub const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";
pub const XSD_DECIMAL: &str = "http://www.w3.org/2001/XMLSchema#decimal";
pub const XSD_DOUBLE: &str = "http://www.w3.org/2001/XMLSchema#double";
pub const XSD_BOOLEAN: &str = "http://www.w3.org/2001/XMLSchema#boolean";

/// A parsed N3 document: ground/rule statements separated by the caller.
pub struct Parsed {
    /// Top-level ground triples (facts).
    pub facts: Vec<[Term; 3]>,
    /// `{ premise } => { conclusion }` forward rules.
    pub rules: Vec<Rule>,
    /// `{ conclusion } <= { premise }` BACKWARD rules — goal-directed, never fired forward.
    /// `{ conclusion } <= true.` is a backward rule with an empty (always-provable) premise.
    pub backward_rules: Vec<Rule>,
    /// The document's base IRI ("" when none) — log:parsedAsN3/log:semantics
    /// resolve relative IRIs against it.
    pub base: String,
}

pub fn parse(src: &str) -> Result<Parsed, String> {
    parse_with_base(src, "")
}

/// Parse `src` as STRICT TURTLE (the W3C Turtle grammar): every N3-only
/// construct — formulae, `?vars`, paths, `=>`/`<=`/`=`, `is…of`/`has`,
/// quantifiers, undeclared-prefix leniency — is a syntax error, and
/// statements must be `.`-terminated.
pub fn parse_turtle_with_base(src: &str, base: &str) -> Result<Parsed, String> {
    let mut p = Parser::new(src);
    p.base = base.to_string();
    p.strict = true;
    let stmts = p.document()?;
    Ok(Parsed { facts: stmts, rules: Vec::new(), backward_rules: Vec::new(), base: base.to_string() })
}

/// As [`parse`], but resolves relative IRIs (in IRIREFs and `@prefix`/`@base`
/// directives) against `base` — the document's own location, RFC 3986-style.
/// An empty `base` keeps relative IRIs as written (the historical behavior).
pub fn parse_with_base(src: &str, base: &str) -> Result<Parsed, String> {
    let mut p = Parser::new(src);
    p.base = base.to_string();
    let stmts = p.document()?;
    let mut facts = Vec::new();
    let mut rules = Vec::new();
    let mut backward_rules = Vec::new();
    for [s, pred, o] in stmts {
        match (&pred, &s, &o) {
            // { premise } => { conclusion }
            (Term::Iri(i), Term::Formula(prem), Term::Formula(concl)) if i == LOG_IMPLIES => {
                rules.push(Rule { premise: prem.clone(), conclusion: concl.clone() });
            }
            // { conclusion } <= { premise } — a backward rule. EYE never fires these
            // forward: they are tried goal-directed when a forward-rule premise (or an
            // EYE-style query rule `{goal} => {goal}`) needs an atom they can conclude.
            // Verified against eyereasoner/eye reasoning/backward: the premise is often a
            // pure builtin over the goal's bindings, which can ONLY be evaluated once the
            // goal instantiates the variables — a forward reversal would derive nothing.
            (Term::Iri(i), Term::Formula(concl), Term::Formula(prem))
                if i == LOG_IMPLIED_BY || i == LOG_IMPLIED_BY_LEGACY =>
            {
                backward_rules.push(Rule { premise: prem.clone(), conclusion: concl.clone() });
            }
            // `{} => { conclusion }` — `{}` IS the literal true (N3 CG), an
            // always-true premise: fires unconditionally.
            (Term::Iri(i), Term::Lit(v, _, _), Term::Formula(concl))
                if i == LOG_IMPLIES && v == "true" =>
            {
                rules.push(Rule { premise: Vec::new(), conclusion: concl.clone() });
            }
            // { conclusion } <= true. — an always-provable backward fact schema (EYE's
            // idiom for backward base cases).
            (Term::Iri(i), Term::Formula(concl), Term::Lit(v, _, _))
                if (i == LOG_IMPLIED_BY || i == LOG_IMPLIED_BY_LEGACY) && v == "true" =>
            {
                backward_rules.push(Rule { premise: Vec::new(), conclusion: concl.clone() });
            }
            _ => facts.push([s, pred, o]),
        }
    }
    // N3 semantics: a blank node inside a rule is an EXISTENTIAL — in the
    // premise it matches anything (like a variable), and a premise-bound
    // label shared with the conclusion carries its binding (the cwm/EYE
    // `[is :p of :x]`-in-premise idiom). Rewrite rule-scoped premise blanks
    // to fresh per-rule variables so the matcher treats them so. Blanks
    // INSIDE quoted `{ … }` formulae are NOT rewritten — a quoted graph's
    // existentials belong to that graph (log:includes scoping needs them as
    // graph-local terms, not rule variables); blanks appearing ONLY in a
    // conclusion also stay blanks (instantiated fresh per firing).
    //
    // The generated name is UNFORGEABLE in source, which is what lets
    // [`super::serialize::write_rule`] undo the rewrite from the name alone:
    // `read_name` never accepts a `.`, so no `?…` a document can write is ever
    // spelled `__bn.<i>.<label>` — whereas an underscore form like `__bn0_x` IS
    // a legal source variable and would make the reverse mapping a guess. Both halves are
    // pinned by
    // `tests/n3_pass_all.rs::a_source_variable_spelled_like_the_rewrite_stays_a_variable`.
    for (ri, r) in rules.iter_mut().chain(backward_rules.iter_mut()).enumerate() {
        let mut premise_blanks: std::collections::HashSet<String> = Default::default();
        collect_blanks(&r.premise, false, &mut premise_blanks);
        if premise_blanks.is_empty() {
            continue;
        }
        let rename = |t: &mut Term| {
            if let Term::Blank(l) = t {
                if premise_blanks.contains(l.as_str()) {
                    *t = Term::Var(format!("{}{}.{}", PREMISE_BLANK_VAR, ri, l));
                }
            }
        };
        rewrite_terms(&mut r.premise, false, &rename);
        rewrite_terms(&mut r.conclusion, false, &rename);
    }
    Ok(Parsed { facts, rules, backward_rules, base: base.to_string() })
}

/// The prefixes cwm resolves without declaration (its reference outputs rely
/// on them).
fn well_known_prefix(pfx: &str) -> Option<&'static str> {
    Some(match pfx {
        "rdf" => "http://www.w3.org/1999/02/22-rdf-syntax-ns#",
        "rdfs" => "http://www.w3.org/2000/01/rdf-schema#",
        "owl" => "http://www.w3.org/2002/07/owl#",
        "xsd" => "http://www.w3.org/2001/XMLSchema#",
        "log" => "http://www.w3.org/2000/10/swap/log#",
        "math" => "http://www.w3.org/2000/10/swap/math#",
        "string" => "http://www.w3.org/2000/10/swap/string#",
        "list" => "http://www.w3.org/2000/10/swap/list#",
        "time" => "http://www.w3.org/2000/10/swap/time#",
        _ => return None,
    })
}

/// `rdf:nil` IS the empty list — normalize the IRI spelling to `Term::List([])`
/// so `()` and `rdf:nil` are one value (cwm/EYE agree).
fn nil_to_list(t: Term) -> Term {
    match t {
        Term::Iri(i) if i == RDF_NIL => Term::List(Vec::new()),
        other => other,
    }
}

/// Collect blank labels; `into_formulae` controls whether quoted `{ … }`
/// graphs are descended into (lists always are — they are plain structure).
fn collect_blanks(stmts: &[[Term; 3]], into_formulae: bool, out: &mut std::collections::HashSet<String>) {
    for row in stmts {
        for t in row {
            collect_blanks_term(t, into_formulae, out);
        }
    }
}

fn collect_blanks_term(t: &Term, into_formulae: bool, out: &mut std::collections::HashSet<String>) {
    match t {
        Term::Blank(l) => {
            out.insert(l.clone());
        }
        Term::Formula(inner) if into_formulae => collect_blanks(inner, into_formulae, out),
        Term::List(ms) => {
            for m in ms {
                collect_blanks_term(m, into_formulae, out);
            }
        }
        // Quoted triples are TRANSPARENT structure (like lists): a premise
        // blank inside `<< … >>` is a rule-scoped existential and becomes a
        // matching variable. [FABLE-5]
        Term::Triple(t) => {
            for m in t.iter() {
                collect_blanks_term(m, into_formulae, out);
            }
        }
        _ => {}
    }
}

fn rewrite_terms(stmts: &mut [[Term; 3]], into_formulae: bool, f: &impl Fn(&mut Term)) {
    for row in stmts.iter_mut() {
        for t in row.iter_mut() {
            rewrite_term(t, into_formulae, f);
        }
    }
}

fn rewrite_term(t: &mut Term, into_formulae: bool, f: &impl Fn(&mut Term)) {
    match t {
        Term::Formula(inner) => {
            if into_formulae {
                rewrite_terms(inner, into_formulae, f);
            }
        }
        Term::List(ms) => {
            for m in ms.iter_mut() {
                rewrite_term(m, into_formulae, f);
            }
        }
        Term::Triple(tr) => {
            for m in tr.iter_mut() {
                rewrite_term(m, into_formulae, f);
            }
        }
        _ => f(t),
    }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    base: String,
    prefixes: std::collections::HashMap<String, String>,
    /// `@forAll`/`@forSome` quantifier scopes (document level at index 0, one
    /// frame per open formula): resolved IRI → the Var/Blank it stands for.
    quants: Vec<std::collections::HashMap<String, Term>>,
    /// STRICT TURTLE mode: reject every N3-only construct (formulae,
    /// variables, paths, rule arrows, quantifiers, undeclared prefixes) and
    /// require `.`-terminated statements — the W3C Turtle grammar.
    strict: bool,
    /// `@keywords` (cwm): when declared, BARE words read as `:word` in the
    /// default prefix unless listed as keywords.
    keywords: Option<std::collections::HashSet<String>>,
    bnode: usize,
    pathvar: usize,
    /// Current `{`/`(`/`[` nesting depth — bounded so pathological inputs
    /// produce a parse ERROR instead of exhausting the stack (the recursive-
    /// descent parser recurses per nesting level).
    depth: usize,
}

/// Maximum bracket-nesting depth (see `Parser::depth`).
const MAX_DEPTH: usize = 4096;

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Parser<'a> {
        Parser {
            s: src.as_bytes(),
            i: 0,
            base: String::new(),
            prefixes: Default::default(),
            quants: vec![Default::default()],
            strict: false,
            keywords: None,
            bnode: 0,
            pathvar: 0,
            depth: 0,
        }
    }

    // ---- lexing helpers ------------------------------------------------------
    fn ws(&mut self) {
        loop {
            while self.i < self.s.len() && is_ws_byte(self.s[self.i]) {
                self.i += 1;
            }
            if self.i < self.s.len() && self.s[self.i] == b'#' {
                while self.i < self.s.len() && self.s[self.i] != b'\n' {
                    self.i += 1;
                }
            } else {
                break;
            }
        }
    }
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
    fn eat(&mut self, b: u8) -> bool {
        self.ws();
        if self.peek() == Some(b) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn starts_with(&mut self, kw: &str) -> bool {
        self.ws();
        self.s[self.i..].starts_with(kw.as_bytes())
    }
    /// Case-insensitive keyword test (the SPARQL-style `PREFIX`/`BASE`
    /// directives are case-insensitive in Turtle), as a WHOLE token.
    fn starts_with_ci(&mut self, kw: &str) -> bool {
        self.ws();
        let end = self.i + kw.len();
        if end > self.s.len() {
            return false;
        }
        self.s[self.i..end].eq_ignore_ascii_case(kw.as_bytes())
            && self.s.get(end).is_none_or(|c| is_ws_byte(*c) || *c == b'<' || *c == b':')
    }

    // ---- top level -----------------------------------------------------------
    fn document(&mut self) -> Result<Vec<[Term; 3]>, String> {
        let mut out = Vec::new();
        loop {
            self.ws();
            if self.i >= self.s.len() {
                break;
            }
            if self.starts_with("@prefix") || self.starts_with_ci("PREFIX") {
                self.directive_prefix()?;
            } else if self.starts_with("@base") || self.starts_with_ci("BASE") {
                self.directive_base()?;
            } else if self.starts_with("@forAll") || self.starts_with("@forSome") {
                if self.strict {
                    return Err("quantifiers are not Turtle".into());
                }
                self.directive_quantifier()?;
            } else if self.starts_with("@keywords") {
                if self.strict {
                    return Err("@keywords is not Turtle".into());
                }
                self.directive_keywords()?;
            } else {
                self.statement(&mut out)?;
            }
        }
        Ok(out)
    }

    /// `@forAll t1, t2 .` / `@forSome t1, t2 .` — declare quantified terms for
    /// the CURRENT scope (document, or the innermost open formula). Each
    /// declared IRI thereafter reads as a variable (forAll) or an existential
    /// blank (forSome) within that scope. Names derive from the IRI so the
    /// same declaration in two documents (action vs reference) compares equal.
    fn directive_quantifier(&mut self) -> Result<(), String> {
        let universal = self.starts_with("@forAll");
        self.i += if universal { 7 } else { 8 };
        loop {
            self.ws();
            let iri = match self.peek() {
                Some(b'<') => self.read_iriref()?,
                _ => match self.read_prefixed_name()? {
                    Term::Iri(i) => i,
                    other => return Err(format!("expected IRI in quantifier, got {other:?}")),
                },
            };
            let local = iri.rsplit(['#', '/']).next().unwrap_or(&iri).to_string();
            let term = if universal {
                Term::Var(format!("__ua_{local}"))
            } else {
                Term::Blank(format!("__ex_{local}"))
            };
            self.quants.last_mut().expect("scope stack").insert(iri, term);
            if !self.eat(b',') {
                break;
            }
        }
        self.eat(b'.');
        Ok(())
    }

    /// The quantified stand-in for `iri`, innermost scope first.
    fn quantified(&self, iri: &str) -> Option<Term> {
        self.quants.iter().rev().find_map(|frame| frame.get(iri).cloned())
    }

    fn directive_prefix(&mut self) -> Result<(), String> {
        let at_form = self.starts_with("@prefix");
        self.i += if at_form { 7 } else { 6 };
        self.ws();
        let pfx = self.read_pname_prefix()?;
        self.ws();
        let iri = self.read_iriref()?;
        self.prefixes.insert(pfx, iri);
        let dotted = self.eat(b'.');
        if self.strict && at_form != dotted {
            return Err("'@prefix' needs a final '.'; SPARQL-style PREFIX must not have one".into());
        }
        Ok(())
    }
    fn directive_base(&mut self) -> Result<(), String> {
        let at_form = self.starts_with("@base");
        self.i += if at_form { 5 } else { 4 };
        self.ws();
        // read_iriref already resolves the new base against the current one.
        self.base = self.read_iriref()?;
        let dotted = self.eat(b'.');
        if self.strict && at_form != dotted {
            return Err("'@base' needs a final '.'; SPARQL-style BASE must not have one".into());
        }
        Ok(())
    }

    /// `@keywords a, is, of.` (cwm) — thereafter BARE words are `:word` names
    /// unless they appear in the declared keyword list.
    fn directive_keywords(&mut self) -> Result<(), String> {
        self.i += 9;
        let mut kws = std::collections::HashSet::new();
        loop {
            self.ws();
            if self.peek() == Some(b'.') {
                break;
            }
            let name = self.read_name();
            if name.is_empty() {
                break;
            }
            kws.insert(name);
            if !self.eat(b',') {
                break;
            }
        }
        self.eat(b'.');
        if kws.is_empty() {
            // `@keywords .` (no keywords at all) is rejected — the N3 CG
            // suite marks the empty form invalid (cwm_syntax keywords1/2)
            // while the with_keywords corpus always declares a non-empty set.
            return Err("@keywords requires at least one keyword".into());
        }
        self.keywords = Some(kws);
        Ok(())
    }

    /// `subject predicateObjectList .` with `;` and `,`. A bare
    /// blankNodePropertyList (`[ :p :o ] .`) needs no predicateObjectList —
    /// its triples were already emitted while reading the subject; N3 also
    /// allows a bare subject (`:a .`, `{:b} .` — zero predicates).
    fn statement(&mut self, out: &mut Vec<[Term; 3]>) -> Result<(), String> {
        self.ws();
        let bracket_subject = self.peek() == Some(b'[');
        let subj = self.term(out)?;
        if self.strict
            && !matches!(subj, Term::Iri(_) | Term::Blank(_) | Term::List(_))
        {
            return Err(format!("invalid Turtle subject {subj:?}"));
        }
        self.ws();
        if matches!(self.peek(), Some(b'.') | Some(b'}') | None)
            && (if self.strict {
                bracket_subject
            } else {
                true // N3: zero-predicate statements are legal
            })
        {
            if !self.eat(b'.') && self.strict {
                return Err("statement must end with '.'".into());
            }
            return Ok(());
        }
        self.predicate_object_list(&subj, out)?;
        if !self.eat(b'.') && self.strict {
            return Err("statement must end with '.'".into());
        }
        Ok(())
    }

    fn predicate_object_list(&mut self, subj: &Term, out: &mut Vec<[Term; 3]>) -> Result<(), String> {
        loop {
            self.ws();
            let (pred, swapped) = self.verb(out)?;
            loop {
                let obj = self.term(out)?;
                if swapped {
                    out.push([obj, pred.clone(), subj.clone()]);
                } else {
                    out.push([subj.clone(), pred.clone(), obj]);
                }
                if !self.eat(b',') {
                    break;
                }
            }
            if !self.eat(b';') {
                break;
            }
            while self.eat(b';') {} // `;;` is legal (empty list items)
            self.ws();
            if matches!(self.peek(), Some(b'.') | Some(b']') | Some(b'}') | None) {
                break; // trailing `;`
            }
        }
        Ok(())
    }

    /// A predicate position: returns `(predicate, swapped)` — `swapped` is the
    /// classic N3 inverse-predicate syntax `is EXPR of` (object and subject
    /// trade places); `has EXPR` is the explicit forward form.
    fn verb(&mut self, _out: &mut Vec<[Term; 3]>) -> Result<(Term, bool), String> {
        self.ws();
        if !self.strict {
            if self.starts_with("=>") {
                self.i += 2;
                return Ok((Term::Iri(LOG_IMPLIES.into()), false));
            }
            if self.starts_with("<=") {
                self.i += 2;
                // `conclusion <= premise` — log:isImpliedBy; `parse` routes it into
                // the backward-rule set (goal-directed resolution, see module doc).
                return Ok((Term::Iri(LOG_IMPLIED_BY.into()), false));
            }
            // `<- expr` — inverted predicate (N3 2023): `:s <- :p :o` ≡ `:o :p :s`.
            // Disambiguate from an IRIREF `<-s>`: an IRI closes with '>' before
            // any whitespace.
            if self.starts_with("<-") && !self.iri_ahead() {
                self.i += 2;
                let pred = self.term(_out)?;
                return Ok((pred, true));
            }
            if self.peek() == Some(b'=') {
                self.i += 1;
                return Ok((Term::Iri(OWL_SAME_AS.into()), false));
            }
        }
        // `a` keyword (rdf:type) — only when a standalone token. Under
        // `@keywords` the bare form needs 'a' declared; `@a` always works.
        let a_allowed = match &self.keywords {
            Some(kws) => kws.contains("a"),
            None => true,
        };
        if a_allowed && self.starts_with("a") {
            let j = self.i + 1;
            if j >= self.s.len() || is_ws_byte(self.s[j]) || self.s[j] == b'<' {
                self.i += 1;
                return Ok((Term::Iri(RDF_TYPE.into()), false));
            }
        }
        if !self.strict && self.starts_with("@a") {
            let j = self.i + 2;
            if j >= self.s.len() || is_ws_byte(self.s[j]) {
                self.i += 2;
                return Ok((Term::Iri(RDF_TYPE.into()), false));
            }
        }
        // `is EXPR of` — inverse predicate; `has EXPR` — explicit forward.
        if !self.strict {
            if self.keyword("is") {
                let pred = self.term(_out)?;
                self.ws();
                if !self.keyword("of") {
                    return Err(format!("expected 'of' after 'is <pred>' at byte {}", self.i));
                }
                return Ok((pred, true));
            }
            if self.keyword("has") {
                return Ok((self.term(_out)?, false));
            }
        }
        let pred = self.term(_out)?; // N3 allows paths in predicate position
        if self.strict && !matches!(pred, Term::Iri(_)) {
            return Err(format!("invalid Turtle predicate {pred:?}"));
        }
        Ok((pred, false))
    }

    /// After `<`, does an IRIREF close (a '>' before any whitespace)?
    fn iri_ahead(&self) -> bool {
        let mut j = self.i + 1;
        while let Some(&c) = self.s.get(j) {
            match c {
                b'>' => return true,
                b'<' => return false, // a second '<' — not inside any IRIREF
                c if is_ws_byte(c) => return false,
                _ => j += 1,
            }
        }
        false
    }

    /// Consume `kw` when it stands as a whole token (followed by whitespace);
    /// under `@keywords`, the explicit `@kw` form is always a keyword.
    fn keyword(&mut self, kw: &str) -> bool {
        self.ws();
        let at = self.peek() == Some(b'@');
        let start = self.i + usize::from(at);
        let end = start + kw.len();
        if self.s[start..].starts_with(kw.as_bytes())
            && self.s.get(end).is_none_or(|c| {
                is_ws_byte(*c) || matches!(c, b'.' | b';' | b',' | b')' | b']' | b'}')
            })
        {
            self.i = end;
            true
        } else {
            false
        }
    }

    // ---- terms ---------------------------------------------------------------
    /// A term, including N3 path expressions: `node (! pred | ^ pred)*`. `A!P` is the object
    /// reached by `A P ?o`; `A^P` is the subject reaching `A` via `?s P A`. Each step
    /// introduces a fresh variable and emits the connecting triple into `out` (so paths in
    /// rule premises join correctly).
    fn term(&mut self, out: &mut Vec<[Term; 3]>) -> Result<Term, String> {
        let mut base = self.atom(out)?;
        if self.strict {
            return Ok(base); // paths are not Turtle
        }
        loop {
            self.ws();
            match self.peek() {
                Some(b'!') => {
                    self.i += 1;
                    let pred = self.atom(out)?;
                    let next = self.fresh_pathvar();
                    out.push([base, pred, next.clone()]);
                    base = next;
                }
                Some(b'^') if self.s.get(self.i + 1) == Some(&b'^') => {
                    // a `^^dt` cast on a non-literal term (old euler/N3QL
                    // surface): consume and ignore the datatype.
                    self.i += 2;
                    let _ = self.read_iri_or_prefixed()?;
                }
                Some(b'^') => {
                    self.i += 1;
                    let pred = self.atom(out)?;
                    let next = self.fresh_pathvar();
                    out.push([next.clone(), pred, base]);
                    base = next;
                }
                _ => break,
            }
        }
        Ok(base)
    }

    fn fresh_pathvar(&mut self) -> Term {
        // Path steps mint EXISTENTIALS (cwm prints them as bnodes); inside a
        // rule premise the blank→variable rewrite turns them into join vars.
        self.pathvar += 1;
        Term::Blank(format!("__path{}", self.pathvar))
    }

    /// A single atomic term (no path operators).
    fn atom(&mut self, out: &mut Vec<[Term; 3]>) -> Result<Term, String> {
        self.ws();
        match self.peek() {
            // `<<` opens an RDF-star / RDF 1.2 quoted-triple term — never a valid
            // IRIREF prefix (`<` is excluded inside an IRIREF), so one byte of
            // lookahead disambiguates. [FABLE-5]
            Some(b'<') if self.s.get(self.i + 1) == Some(&b'<') => {
                if self.strict {
                    return Err("quoted triples are not Turtle 1.1".into());
                }
                self.read_quoted_triple(out)
            }
            Some(b'<') => {
                let iri = self.read_iriref()?;
                if let Some(q) = self.quantified(&iri) {
                    return Ok(q);
                }
                Ok(nil_to_list(Term::Iri(iri)))
            }
            Some(b'"') | Some(b'\'') => self.read_literal(),
            Some(b'?') => {
                if self.strict {
                    return Err("variables are not Turtle".into());
                }
                self.i += 1;
                let v = self.read_name();
                if self.peek() == Some(b'@') {
                    // `?D@de` (euler-era language cast on a variable): ignore.
                    self.i += 1;
                    let _ = self.read_name();
                }
                Ok(Term::Var(v))
            }
            Some(b'_') => {
                // _:label
                self.i += 1;
                self.eat(b':');
                Ok(Term::Blank(self.read_blank_label()))
            }
            Some(b'{') => {
                if self.strict {
                    return Err("formulae are not Turtle".into());
                }
                self.read_formula()
            }
            Some(b'(') => self.read_collection(out),
            Some(b'[') => self.read_bnode_propertylist(out),
            Some(c) if c.is_ascii_digit() || c == b'+' || c == b'-' || c == b'.' => self.read_number(),
            Some(b'@') if !self.strict => {
                // `@true` / `@false` (the explicit keyword forms under @keywords)
                if self.starts_with("@true") {
                    self.i += 5;
                    return Ok(Term::Lit("true".into(), XSD_BOOLEAN.into(), None));
                }
                if self.starts_with("@false") {
                    self.i += 6;
                    return Ok(Term::Lit("false".into(), XSD_BOOLEAN.into(), None));
                }
                Err(format!("unexpected '@' token at byte {}", self.i))
            }
            Some(_) => {
                // prefixed name, `true`/`false`, or `a`
                let bool_allowed = |kws: &Option<std::collections::HashSet<String>>, w: &str| {
                    match kws {
                        Some(set) => set.contains(w),
                        None => true,
                    }
                };
                if bool_allowed(&self.keywords, "true") && self.keyword("true") {
                    return Ok(Term::Lit("true".into(), XSD_BOOLEAN.into(), None));
                }
                if bool_allowed(&self.keywords, "false") && self.keyword("false") {
                    return Ok(Term::Lit("false".into(), XSD_BOOLEAN.into(), None));
                }
                match self.read_prefixed_name()? {
                    Term::Iri(i) => {
                        if let Some(q) = self.quantified(&i) {
                            return Ok(q);
                        }
                        Ok(nil_to_list(Term::Iri(i)))
                    }
                    other => Ok(other),
                }
            }
            None => Err("unexpected end of input".into()),
        }
    }

    fn read_iriref(&mut self) -> Result<String, String> {
        if !self.eat(b'<') {
            return Err(format!("expected IRI at byte {}", self.i));
        }
        let mut iri = String::new();
        loop {
            let Some(c) = self.peek() else { return Err("unterminated IRI".into()) };
            match c {
                b'>' => {
                    self.i += 1;
                    break;
                }
                b'\\' => {
                    // only \uXXXX / \UXXXXXXXX are legal inside an IRIREF —
                    // and the DECODED character is held to the same charset.
                    self.i += 1;
                    let ch = self.read_unicode_escape()?;
                    if (ch as u32) <= 0x20 || matches!(ch, '<' | '>' | '"' | '{' | '}' | '|' | '^' | '`') {
                        return Err(format!("escaped character {ch:?} is not allowed in an IRI"));
                    }
                    iri.push(ch);
                }
                // IRIREF excludes control chars, space and <>"{}|^`
                0x00..=0x20 | b'<' | b'"' | b'{' | b'}' | b'|' | b'^' | b'`' => {
                    return Err(format!("illegal character 0x{c:02x} in IRI"));
                }
                _ => {
                    let len = utf8_len(c);
                    iri.push_str(
                        std::str::from_utf8(&self.s[self.i..self.i + len])
                            .map_err(|e| e.to_string())?,
                    );
                    self.i += len;
                }
            }
        }
        Ok(resolve_iri(&self.base, &iri))
    }

    /// `\uXXXX` or `\UXXXXXXXX` with the backslash already consumed.
    fn read_unicode_escape(&mut self) -> Result<char, String> {
        let kind = self.peek().ok_or("truncated escape")?;
        let digits = match kind {
            b'u' => 4,
            b'U' => 8,
            other => return Err(format!("illegal escape '\\{}'", other as char)),
        };
        self.i += 1;
        if self.i + digits > self.s.len() {
            return Err("truncated unicode escape".into());
        }
        let hex = std::str::from_utf8(&self.s[self.i..self.i + digits]).map_err(|e| e.to_string())?;
        let cp = u32::from_str_radix(hex, 16).map_err(|_| format!("bad unicode escape '{hex}'"))?;
        self.i += digits;
        char::from_u32(cp).ok_or_else(|| format!("invalid code point U+{cp:X}"))
    }

    fn read_pname_prefix(&mut self) -> Result<String, String> {
        // read up to ':' (dots are legal inside a prefix name: `e.g:`)
        let start = self.i;
        while self.i < self.s.len() && self.s[self.i] != b':' && !is_ws_byte(self.s[self.i]) {
            self.i += 1;
        }
        // Unreachable on valid UTF-8 now that the scan only stops on ASCII bytes
        // (always char boundaries), but the parser must REPORT, never panic.
        let pfx = std::str::from_utf8(&self.s[start..self.i])
            .map_err(|e| e.to_string())?
            .to_string();
        if self.peek() != Some(b':') {
            return Err(format!("expected ':' after prefix name at byte {}", self.i));
        }
        self.i += 1;
        Ok(pfx)
    }

    fn read_prefixed_name(&mut self) -> Result<Term, String> {
        let start = self.i;
        let mut tok = String::new();
        while self.i < self.s.len() {
            let c = self.s[self.i];
            if is_ws_byte(c)
                || matches!(c, b';' | b',' | b']' | b'}' | b')' | b'(' | b'[' | b'{' | b'!' | b'^' | b'#')
            {
                break; // '#' starts a comment — never part of an (unescaped) pname
            }
            if c == b'\\' {
                // PN_LOCAL_ESC: the escaped punctuation char, literally.
                self.i += 1;
                let Some(e) = self.peek() else { return Err("truncated pname escape".into()) };
                if !br"_~.-!$&'()*+,;=/?#@%".contains(&e) {
                    return Err(format!("illegal pname escape '\\{}'", e as char));
                }
                tok.push(e as char);
                self.i += 1;
                continue;
            }
            if c == b'.' {
                // a dot is part of the name only when followed by another
                // name character (`:a.b` yes; `:a. ` no — statement dot).
                let cont = self.s.get(self.i + 1).is_some_and(|&n| {
                    (n as char).is_alphanumeric()
                        || matches!(n, b'_' | b'-' | b'.' | b'%' | b'\\' | b':')
                        || n >= 0x80
                });
                if !cont {
                    break;
                }
                if self.strict && tok.ends_with(':') {
                    return Err("local name cannot start with '.'".into());
                }
                tok.push('.');
                self.i += 1;
                continue;
            }
            if self.strict {
                // W3C Turtle PN rules on the RAW characters (escapes were
                // handled above): '%' needs two hex digits; other ASCII
                // punctuation must be escaped; locals cannot start with '-'.
                if c == b'%' {
                    let ok = self.s.get(self.i + 1).is_some_and(u8::is_ascii_hexdigit)
                        && self.s.get(self.i + 2).is_some_and(u8::is_ascii_hexdigit);
                    if !ok {
                        return Err("bad %-sequence in prefixed name".into());
                    }
                } else if c == b'-' && tok.ends_with(':') {
                    return Err("local name cannot start with '-'".into());
                } else if c.is_ascii()
                    && !c.is_ascii_alphanumeric()
                    && !matches!(c, b'_' | b'-' | b'.' | b':' | b'%')
                {
                    return Err(format!("character '{}' must be escaped in a local name", c as char));
                }
            }
            let len = utf8_len(c);
            tok.push_str(std::str::from_utf8(&self.s[self.i..self.i + len]).map_err(|e| e.to_string())?);
            self.i += len;
        }
        let tok = tok.as_str();
        let Some((pfx, local)) = tok.split_once(':') else {
            // a BARE word: legal only under `@keywords` (cwm) — it reads as
            // `:word` in the default prefix unless declared a keyword.
            if let Some(kws) = &self.keywords {
                if !tok.is_empty() && !kws.contains(tok) {
                    let ns = match self.prefixes.get("") {
                        Some(ns) => ns.clone(),
                        None => resolve_iri(&self.base, "#"),
                    };
                    return Ok(Term::Iri(format!("{ns}{tok}")));
                }
            }
            return Err(format!("bad token '{tok}' at {start}"));
        };
        let ns = match self.prefixes.get(pfx) {
            Some(ns) => ns.clone(),
            None if self.strict => return Err(format!("unknown prefix '{pfx}:'")),
            // cwm/EYE treat an undeclared default prefix as `@prefix : <#>.`
            // (document-local names); honor that for ':' only.
            None if pfx.is_empty() => resolve_iri(&self.base, "#"),
            // cwm's reference outputs use the well-known SWAP/RDF prefixes
            // without declaring them — resolve those like cwm does.
            None => match well_known_prefix(pfx) {
                Some(ns) => ns.to_string(),
                None => return Err(format!("unknown prefix '{pfx}:'")),
            },
        };
        Ok(Term::Iri(format!("{ns}{local}")))
    }

    fn read_name(&mut self) -> String {
        let start = self.i;
        while self.i < self.s.len() {
            let c = self.s[self.i];
            if c.is_ascii_alphanumeric() || c == b'_' || c == b'-' {
                self.i += 1;
            } else if c >= 0x80 {
                self.i += utf8_len(c); // whole code point (PN_CHARS go beyond ASCII)
            } else {
                break;
            }
        }
        String::from_utf8_lossy(&self.s[start..self.i]).into_owned()
    }

    /// A blank-node label: like a name, but dots are allowed INSIDE
    /// (`_:b.0` — Turtle BLANK_NODE_LABEL).
    fn read_blank_label(&mut self) -> String {
        let mut label = self.read_name();
        while self.peek() == Some(b'.') {
            let cont = self.s.get(self.i + 1).is_some_and(|&n| {
                n.is_ascii_alphanumeric() || matches!(n, b'_' | b'-') || n >= 0x80
            });
            if !cont {
                break;
            }
            self.i += 1;
            label.push('.');
            label.push_str(&self.read_name());
        }
        label
    }

    fn read_literal(&mut self) -> Result<Term, String> {
        let quote = self.s[self.i];
        // triple-quoted?
        let triple = self.s[self.i..].starts_with(&[quote, quote, quote]);
        if triple {
            self.i += 3;
        } else {
            self.i += 1;
        }
        let mut val = String::new();
        loop {
            if self.i >= self.s.len() {
                return Err("unterminated literal".into());
            }
            let c = self.s[self.i];
            if c == b'\\' {
                self.i += 1;
                let Some(e) = self.peek() else { return Err("truncated escape".into()) };
                match e {
                    b'n' => val.push('\n'),
                    b't' => val.push('\t'),
                    b'r' => val.push('\r'),
                    b'b' => val.push('\u{8}'),
                    b'f' => val.push('\u{c}'),
                    b'"' => val.push('"'),
                    b'\'' => val.push('\''),
                    b'\\' => val.push('\\'),
                    b'u' | b'U' => {
                        val.push(self.read_unicode_escape()?);
                        continue;
                    }
                    other => return Err(format!("illegal string escape '\\{}'", other as char)),
                }
                self.i += 1;
                continue;
            }
            if c == quote {
                if triple {
                    if self.s[self.i..].starts_with(&[quote, quote, quote]) {
                        self.i += 3;
                        break;
                    }
                } else {
                    self.i += 1;
                    break;
                }
            }
            // utf-8 safe push
            let ch_len = utf8_len(c);
            val.push_str(std::str::from_utf8(&self.s[self.i..self.i + ch_len]).map_err(|e| e.to_string())?);
            self.i += ch_len;
        }
        // datatype / lang
        if self.s[self.i..].starts_with(b"^^") {
            self.i += 2;
            let dt = self.read_iri_or_prefixed()?;
            Ok(Term::Lit(val, dt, None))
        } else if self.peek() == Some(b'@') {
            self.i += 1;
            let lang = self.read_name();
            // LANGTAG: [a-zA-Z]+('-'[a-zA-Z0-9]+)*; canonical form is lowercase.
            let valid = !lang.is_empty()
                && lang.split('-').enumerate().all(|(k, part)| {
                    !part.is_empty()
                        && part.bytes().all(|c| {
                            c.is_ascii_alphabetic() || (k > 0 && c.is_ascii_digit())
                        })
                });
            if !valid {
                return Err(format!("invalid language tag '@{lang}'"));
            }
            Ok(Term::Lit(
                val,
                "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString".into(),
                Some(lang.to_ascii_lowercase()),
            ))
        } else {
            Ok(Term::Lit(val, "http://www.w3.org/2001/XMLSchema#string".into(), None))
        }
    }

    fn read_iri_or_prefixed(&mut self) -> Result<String, String> {
        self.ws();
        if self.peek() == Some(b'<') {
            self.read_iriref()
        } else {
            match self.read_prefixed_name()? {
                Term::Iri(i) => Ok(i),
                _ => Err("expected datatype IRI".into()),
            }
        }
    }

    fn read_number(&mut self) -> Result<Term, String> {
        let start = self.i;
        let mut is_decimal = false;
        let mut is_double = false;
        let mut digits = 0usize;
        if matches!(self.peek(), Some(b'+') | Some(b'-')) {
            self.i += 1;
        }
        while self.i < self.s.len() {
            let c = self.s[self.i];
            let exp_next = |j: usize, s: &[u8]| {
                matches!(s.get(j), Some(b'e') | Some(b'E'))
                    && match s.get(j + 1) {
                        Some(b'+') | Some(b'-') => s.get(j + 2).is_some_and(u8::is_ascii_digit),
                        Some(d) => d.is_ascii_digit(),
                        None => false,
                    }
            };
            if c.is_ascii_digit() {
                digits += 1;
                self.i += 1;
            } else if c == b'.'
                && !is_decimal
                && !is_double
                && (self.s.get(self.i + 1).is_some_and(u8::is_ascii_digit)
                    || exp_next(self.i + 1, self.s))
            {
                // `1.5`, or `123.E+1` (Turtle DOUBLE allows an empty fraction)
                is_decimal = true;
                self.i += 1;
            } else if (c == b'e' || c == b'E') && digits > 0 && !is_double && exp_next(self.i, self.s)
            {
                is_double = true;
                self.i += 1;
                if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                    self.i += 1;
                }
            } else {
                break;
            }
        }
        if digits == 0 {
            return Err(format!("expected a number at byte {start}"));
        }
        let txt = std::str::from_utf8(&self.s[start..self.i])
            .map_err(|e| e.to_string())?
            .to_string();
        let dt = if is_double { XSD_DOUBLE } else if is_decimal { XSD_DECIMAL } else { XSD_INTEGER };
        Ok(Term::Lit(txt, dt.into(), None))
    }

    fn enter(&mut self) -> Result<(), String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(format!("nesting deeper than {MAX_DEPTH}"));
        }
        Ok(())
    }

    fn read_formula(&mut self) -> Result<Term, String> {
        self.enter()?;
        self.eat(b'{');
        self.quants.push(Default::default()); // formula-local quantifier scope
        let mut triples = Vec::new();
        loop {
            self.ws();
            if self.eat(b'}') {
                break;
            }
            if self.i >= self.s.len() {
                return Err("unterminated formula".into());
            }
            if self.starts_with("@forAll") || self.starts_with("@forSome") {
                self.directive_quantifier()?;
            } else if self.starts_with("@prefix") {
                self.directive_prefix()?;
            } else if self.starts_with("@base") {
                self.directive_base()?;
            } else {
                self.statement(&mut triples)?;
            }
        }
        self.quants.pop();
        self.depth -= 1;
        if triples.is_empty() {
            // `{}` IS the literal true (N3 CG: the empty graph holds vacuously).
            return Ok(Term::Lit("true".into(), XSD_BOOLEAN.into(), None));
        }
        Ok(Term::Formula(triples))
    }

    /// An RDF-star / RDF 1.2 quoted-triple term: `<< s p o >>` or the RDF 1.2
    /// triple-term spelling `<<( s p o )>>` (the `(` must FOLLOW `<<` with no
    /// whitespace — `<< (1 2) :p :o >>` keeps its list subject). Both forms
    /// produce the same first-class [`Term::Triple`] value; components may be
    /// any N3 term, including variables and nested quoted triples. NOTE: N3
    /// itself has not pinned RDF-star syntax (see GH #2012); this follows the
    /// classic RDF-star reading where `<< s p o >>` IS the triple term — it is
    /// NOT expanded into RDF 1.2's `rdf:reifies` reifier sugar. [FABLE-5]
    fn read_quoted_triple(&mut self, out: &mut Vec<[Term; 3]>) -> Result<Term, String> {
        self.enter()?;
        self.i += 2; // the caller verified `<<`
        // RDF 1.2 triple-term form: the literal token `<<(` (no whitespace).
        let paren = self.s.get(self.i) == Some(&b'(');
        if paren {
            self.i += 1;
        }
        let s = self.term(out)?;
        let p = self.term(out)?;
        let o = self.term(out)?;
        if paren && !self.eat(b')') {
            return Err(format!("expected ')' closing a <<( … )>> triple term at byte {}", self.i));
        }
        self.ws();
        if !self.s[self.i..].starts_with(b">>") {
            return Err(format!("expected '>>' closing a quoted triple at byte {}", self.i));
        }
        self.i += 2;
        self.depth -= 1;
        Ok(Term::Triple(Box::new([s, p, o])))
    }

    fn read_collection(&mut self, out: &mut Vec<[Term; 3]>) -> Result<Term, String> {
        self.enter()?;
        self.eat(b'(');
        let mut items = Vec::new();
        loop {
            self.ws();
            if self.eat(b')') {
                break;
            }
            if self.i >= self.s.len() {
                return Err("unterminated collection".into());
            }
            items.push(self.term(out)?);
        }
        self.depth -= 1;
        // A collection is a FIRST-CLASS list term (N3 semantics); `()` = rdf:nil
        // = the empty list.
        Ok(Term::List(items))
    }

    fn read_bnode_propertylist(&mut self, out: &mut Vec<[Term; 3]>) -> Result<Term, String> {
        self.enter()?;
        self.eat(b'[');
        self.ws();
        // N3's iriPropertyList: `[id :s :p :o]` names the node :s (not a bnode).
        let node = if !self.strict && self.keyword("id") {
            let named = self.term(out)?;
            if !matches!(named, Term::Iri(_)) {
                return Err(format!("iriPropertyList id must be an IRI, got {named:?}"));
            }
            named
        } else {
            self.fresh_bnode()
        };
        self.ws();
        if !self.eat(b']') {
            self.predicate_object_list(&node, out)?;
            self.ws();
            self.eat(b']');
        }
        self.depth -= 1;
        Ok(node)
    }

    fn fresh_bnode(&mut self) -> Term {
        self.bnode += 1;
        Term::Blank(format!("_b{}", self.bnode))
    }
}

/// Pragmatic RFC 3986 reference resolution: enough for the forms test suites
/// and real documents use (`#frag`, `name.n3#frag`, `../x`, absolute IRIs,
/// scheme-relative and root-relative paths). With no base, or an absolute
/// reference, the reference is returned as written.
pub(super) fn resolve_iri(base: &str, iri: &str) -> String {
    let is_absolute = |s: &str| -> bool {
        match s.find(':') {
            Some(i) if i > 0 => {
                let scheme = &s[..i];
                scheme.starts_with(|c: char| c.is_ascii_alphabetic())
                    && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
                    // a pname-like "a:b" inside <> is still an IRI; treat any
                    // syntactically valid scheme as absolute (RFC 3986 §4.3)
            }
            _ => false,
        }
    };
    if base.is_empty() || is_absolute(iri) {
        return iri.to_string();
    }
    // Strip any fragment from the base.
    let base = base.split('#').next().unwrap_or(base);
    if iri.is_empty() {
        return base.to_string();
    }
    if let Some(frag) = iri.strip_prefix('#') {
        return format!("{base}#{frag}");
    }
    if iri.starts_with('?') {
        // query-only reference: replace the base's query, keep its path
        return format!("{}{iri}", base.split('?').next().unwrap_or(base));
    }
    // scheme = up to ':'; authority = '//…' if present.
    let scheme_end = base.find(':').map(|i| i + 1).unwrap_or(0);
    let (authority_end, has_authority) = if base[scheme_end..].starts_with("//") {
        let rest = &base[scheme_end + 2..];
        (scheme_end + 2 + rest.find('/').unwrap_or(rest.len()), true)
    } else {
        (scheme_end, false)
    };
    if iri.starts_with("//") && has_authority {
        return format!("{}{}", &base[..scheme_end], iri);
    }
    let merged = if iri.starts_with('/') {
        format!("{}{}", &base[..authority_end], iri)
    } else {
        // Merge with the base path minus its last segment.
        let path_end = base.rfind('/').map(|i| i + 1).unwrap_or(base.len());
        let dir = if path_end > authority_end { &base[..path_end] } else { base };
        if dir.ends_with('/') {
            format!("{dir}{iri}")
        } else {
            format!("{dir}/{iri}")
        }
    };
    // Remove dot segments in the path part (after the authority); the
    // query/fragment must not take part (RFC 3986 §5.2.4).
    let (prefix, rest) = merged.split_at(authority_end.min(merged.len()));
    let qpos = rest.find(['?', '#']).unwrap_or(rest.len());
    let (path, tail) = rest.split_at(qpos);
    let mut out: Vec<&str> = Vec::new();
    let mut trailing_slash = path.ends_with('/');
    for seg in path.split('/') {
        match seg {
            "." => trailing_slash = true,
            ".." => {
                // never pop the leading empty segment (the path root)
                if out.len() > 1 || (out.len() == 1 && !out[0].is_empty()) {
                    out.pop();
                }
                trailing_slash = true;
            }
            s => {
                trailing_slash = s.is_empty();
                out.push(s);
            }
        }
    }
    let mut joined = out.join("/");
    if trailing_slash && !joined.ends_with('/') {
        joined.push('/');
    }
    format!("{prefix}{joined}{tail}")
}

/// Byte-level whitespace test for the byte-wise lexer — ONLY ASCII bytes count.
/// In valid UTF-8 the byte values 0x85 / 0xA0 occur exclusively as CONTINUATION
/// bytes of a multibyte character, yet read as U+0085 NEL / U+00A0 NBSP through
/// `(byte as char).is_whitespace()`, which split token scans MID-character and
/// panicked the `from_utf8` in `read_pname_prefix` (sq-t8z0r randomized-fuzz
/// finding, GH #1903). Genuine non-ASCII whitespace (NEL / NBSP / U+2028 …) is
/// not token whitespace in Turtle/N3 anyway, so restricting to ASCII both fixes
/// the split and matches the spec. [FABLE-5]
fn is_ws_byte(b: u8) -> bool {
    b.is_ascii() && (b as char).is_whitespace()
}

fn utf8_len(b: u8) -> usize {
    if b < 0x80 {
        1
    } else if b >> 5 == 0b110 {
        2
    } else if b >> 4 == 0b1110 {
        3
    } else {
        4
    }
}

#[cfg(test)]
mod resolve_iri_tests {
    use super::resolve_iri;

    // [OPUS-4.8] roborev 2318 regression: excess `..` segments must NOT ascend
    // above the path root and swallow the authority (RFC 3986 §5.2.4 step 2C:
    // a leading `..` against an empty/root output buffer is discarded, never
    // popping past root). The finding claimed `../../../x` against
    // `http://example.org/a/b/c.n3` would yield `http://example.orgx`.
    #[test]
    fn excess_dotdot_does_not_pop_root() {
        assert_eq!(
            resolve_iri("http://example.org/a/b/c.n3", "../../../x"),
            "http://example.org/x"
        );
        // Exactly enough `..` to reach root, plus more, all clamp at root.
        assert_eq!(
            resolve_iri("http://example.org/a/b/c.n3", "../../x"),
            "http://example.org/x"
        );
        assert_eq!(
            resolve_iri("http://example.org/a/b/c.n3", "../x"),
            "http://example.org/a/x"
        );
        // Authority is preserved even with a single excess `..` from a shallow base.
        assert_eq!(
            resolve_iri("http://example.org/a", "../../x"),
            "http://example.org/x"
        );
    }

    // RFC 3986 §5.4 normal + abnormal reference-resolution examples (the ones
    // exercising dot-segment removal), against base `http://a/b/c/d;p?q`.
    #[test]
    fn rfc3986_dot_segment_examples() {
        let base = "http://a/b/c/d;p?q";
        for (rel, want) in [
            ("g", "http://a/b/c/g"),
            ("./g", "http://a/b/c/g"),
            ("g/", "http://a/b/c/g/"),
            ("/g", "http://a/g"),
            ("..", "http://a/b/"),
            ("../", "http://a/b/"),
            ("../g", "http://a/b/g"),
            ("../..", "http://a/"),
            ("../../", "http://a/"),
            ("../../g", "http://a/g"),
            // Abnormal: extra `..` must not climb past root.
            ("../../../g", "http://a/g"),
            ("../../../../g", "http://a/g"),
            ("/./g", "http://a/g"),
            ("/../g", "http://a/g"),
            (".", "http://a/b/c/"),
            ("./", "http://a/b/c/"),
        ] {
            assert_eq!(resolve_iri(base, rel), want, "rel={rel}");
        }
    }
}

// ---- behavioural tests for the public N3 parser surface (sq-jd5s) [OPUS-4.8] ----
// Drive the `parse` / `parse_with_base` / `parse_turtle_with_base` API and assert on the
// produced facts/rules and on the error path, exercising the lowest-covered parser branches.
#[cfg(test)]
mod parse_surface_tests {
    use super::*;

    fn iri(s: &str) -> Term {
        Term::Iri(s.into())
    }

    #[test]
    fn prefixed_names_and_a_keyword_and_base_resolution() {
        // @base resolves the relative IRI <Alice>; `a` is rdf:type; the prefixed name expands.
        let p = parse_with_base(
            "@prefix ex: <http://ex/> . <Alice> a ex:Person .",
            "http://base/",
        )
        .expect("valid turtle-ish N3");
        assert_eq!(p.facts.len(), 1);
        assert_eq!(
            p.facts[0],
            [iri("http://base/Alice"), iri(RDF_TYPE), iri("http://ex/Person")],
            "relative subject resolved against @base; `a` -> rdf:type; prefix expanded"
        );
        assert_eq!(p.base, "http://base/");
    }

    #[test]
    fn typed_lang_bool_integer_decimal_double_literals() {
        let p = parse(
            r#"<http://ex/s> <http://ex/p> "txt"@en, "v"^^<http://ex/dt>, true, 42, 3.5, 6.0e2 ."#,
        )
        .expect("literal object list");
        let objs: Vec<&Term> = p.facts.iter().map(|t| &t[2]).collect();
        let rdf_lang_string = "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString";
        assert!(
            objs.contains(&&Term::Lit("txt".into(), rdf_lang_string.into(), Some("en".into()))),
            "lang-tagged literal carries rdf:langString + the lowercased tag"
        );
        assert!(objs.contains(&&Term::Lit("v".into(), "http://ex/dt".into(), None)), "typed");
        assert!(objs.contains(&&Term::Lit("true".into(), XSD_BOOLEAN.into(), None)), "boolean");
        assert!(objs.contains(&&Term::Lit("42".into(), XSD_INTEGER.into(), None)), "integer");
        assert!(objs.contains(&&Term::Lit("3.5".into(), XSD_DECIMAL.into(), None)), "decimal");
        assert!(objs.contains(&&Term::Lit("6.0e2".into(), XSD_DOUBLE.into(), None)), "double");
    }

    #[test]
    fn forward_and_backward_rules_and_sameas_sugar() {
        // `=>` is a forward rule; `<=` is a backward rule; `=` is owl:sameAs sugar.
        let p = parse(
            "{ ?x <http://ex/p> ?y } => { ?x <http://ex/q> ?y } .\n\
             { <http://ex/a> <http://ex/r> <http://ex/b> } <= true .\n\
             <http://ex/a> = <http://ex/b> .",
        )
        .expect("rules + sameAs");
        assert_eq!(p.rules.len(), 1, "one forward rule");
        assert_eq!(p.rules[0].premise.len(), 1);
        assert_eq!(p.rules[0].conclusion.len(), 1);
        assert_eq!(p.backward_rules.len(), 1, "one backward rule");
        assert!(p.backward_rules[0].premise.is_empty(), "`<= true` is an empty premise");
        assert!(
            p.facts.iter().any(|t| t[1] == iri(OWL_SAME_AS)),
            "`=` desugars to owl:sameAs"
        );
    }

    #[test]
    fn is_of_inverse_predicate_swaps_subject_object() {
        // `<http://ex/b> is <http://ex/p> of <http://ex/a>` asserts (a p b) — inverse predicate.
        let p = parse("<http://ex/b> is <http://ex/p> of <http://ex/a> .").expect("is..of");
        assert_eq!(
            p.facts[0],
            [iri("http://ex/a"), iri("http://ex/p"), iri("http://ex/b")],
            "`X is P of Y` asserts (Y P X)"
        );
    }

    #[test]
    fn collection_expands_to_first_rest_nil() {
        // A `( ... )` list object is materialised as an N3 List term value.
        let p = parse("<http://ex/s> <http://ex/p> ( <http://ex/a> <http://ex/b> ) .")
            .expect("collection");
        match &p.facts[0][2] {
            Term::List(items) => {
                assert_eq!(items, &vec![iri("http://ex/a"), iri("http://ex/b")], "list members");
            }
            other => panic!("expected a List object, got {other:?}"),
        }
    }

    #[test]
    fn predicate_and_object_lists_with_semicolon_and_comma() {
        // `;` shares the subject; `,` shares subject+predicate.
        let p = parse(
            "<http://ex/s> <http://ex/p> <http://ex/o1>, <http://ex/o2> ; <http://ex/q> <http://ex/o3> .",
        )
        .expect("predicate/object lists");
        assert_eq!(p.facts.len(), 3, "two objects on p + one on q");
        assert!(p.facts.iter().all(|t| t[0] == iri("http://ex/s")), "subject shared via `;`");
    }

    #[test]
    fn strict_turtle_rejects_n3_only_constructs() {
        // parse_turtle_with_base is STRICT W3C Turtle: N3-only forms are syntax errors.
        for src in [
            "{ ?x <http://ex/p> ?y } => { ?x <http://ex/q> ?y } .", // formula + =>
            "?x <http://ex/p> <http://ex/o> .",                     // ?var
            "<http://ex/a> = <http://ex/b> .",                      // = sugar
            "<http://ex/b> is <http://ex/p> of <http://ex/a> .",    // is..of
        ] {
            assert!(
                parse_turtle_with_base(src, "").is_err(),
                "strict Turtle must reject N3-only construct: {src}"
            );
        }
        // ...but it ACCEPTS plain Turtle.
        let ok = parse_turtle_with_base("<http://ex/s> <http://ex/p> <http://ex/o> .", "")
            .expect("plain turtle accepted in strict mode");
        assert_eq!(ok.facts.len(), 1);
        assert!(ok.rules.is_empty() && ok.backward_rules.is_empty());
    }

    #[test]
    fn malformed_input_returns_err_not_panic() {
        // Each is a genuinely-broken document the parser must REPORT (Err) rather than panic on —
        // exercising distinct error-return branches.
        for bad in [
            "<http://ex/s> <http://ex/p>",                  // missing object + terminator
            "<http://ex/s> <http://ex/p> \"unterminated .", // unterminated string literal
            "{ ?x <http://ex/p> ?y => .",                   // unbalanced formula brace
            "<http://ex/s> <http://ex/p> \"v\"@1bad .",      // invalid language tag (digit-led)
        ] {
            assert!(parse(bad).is_err(), "expected a parse error for: {bad:?}");
        }
    }
}
