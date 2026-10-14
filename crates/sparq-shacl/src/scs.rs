//! (sq-v0b8, #796) SHACL Compact Syntax (SCS) PARSER — text → SHACL
//! shapes triples.
//!
//! This is the *parse* direction of the W3C SHACL Compact Syntax (the
//! display/serialise direction shipped client-side in the site, #860). A
//! recursive-descent parser over the W3C `SHACLC.g4` grammar that emits the same
//! `oxrdf::Triple` shapes model the rest of `sparq-shacl` consumes — so an SCS
//! document feeds [`crate::validate`] exactly like a Turtle shapes graph would
//! (round-trip via [`crate::graph_from_triples`] /
//! [`crate::load_turtle_with_base`]).
//!
//! OPT-IN: behind the `scs` cargo feature so the base SHACL Core +
//! SHACL-SPARQL validation path (and the default + wasm bundles) carry zero SCS
//! parser code, deps, or runtime cost when off. No new dependencies — the lexer
//! and parser are hand-rolled over `oxrdf` term construction the crate already
//! pulls in.
//!
//! ## Coverage (honest)
//!
//! Covers the grammar the W3C `shacl12-cs` test corpus exercises:
//!
//!   * directives: `BASE`, `IMPORTS` (→ `owl:imports`), `PREFIX`, with the four
//!     implicit prefixes (`rdf` / `rdfs` / `sh` / `xsd`) the spec predefines;
//!   * `shape` node shapes (`-> targetClass+`) and `shapeClass` (adds the
//!     `rdfs:Class` type), with node-level params/constraints;
//!   * property shapes over full path expressions — IRI, `^`inverse,
//!     `/`sequence, `|`alternative, `?`/`*`/`+` modifiers, `(...)` grouping —
//!     emitted as the SHACL property-path triple encoding;
//!   * `[min..max]` counts, `nodeKind` keywords, bare-IRI `sh:class`, `@`shape
//!     refs (`sh:node`), `param=value` constraints, `!`negation (`sh:not`),
//!     `|` disjunction (`sh:or` list), nested `{...}` shapes (`sh:node`), and
//!     `[ ... ]` value arrays (RDF list — `sh:in` / `sh:ignoredProperties`).
//!
//! Any construct outside that surface returns a typed [`ScsError`] rather than
//! silently mis-parsing.

use crate::graph_from_triples;
use oxrdf::{BlankNode, Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use rustc_hash::FxHashMap;
use sparq_core::Graph;

const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const RDFS: &str = "http://www.w3.org/2000/01/rdf-schema#";
const SH: &str = "http://www.w3.org/ns/shacl#";
const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
const OWL: &str = "http://www.w3.org/2002/07/owl#";

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const RDF_FIRST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#first";
const RDF_REST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#rest";
const RDF_NIL: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#nil";

/// The default base IRI used when an SCS document declares no `BASE` directive,
/// matching the W3C SCS reference-implementation convention (and the
/// `shacl12-cs` expected-Turtle fixtures).
pub const DEFAULT_BASE: &str = "urn:x-base:default";

/// A SHACL Compact Syntax parse error. Carries the 1-based source line so the
/// caller can point at the offending construct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScsError {
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for ScsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SCS parse error (line {}): {}", self.line, self.message)
    }
}

impl std::error::Error for ScsError {}

/// Parses a SHACL Compact Syntax document into the shapes triples, then builds a
/// queryable [`Graph`] ready for [`crate::validate`]. Relative IRIs (and the
/// `owl:Ontology` subject) resolve against `base`; pass [`DEFAULT_BASE`] to mirror
/// the no-`BASE` convention.
///
/// A document-level `BASE` directive overrides `base` for subsequent IRIs.
pub fn parse_to_graph(text: &str, base: &str) -> Result<Graph, ScsError> {
    Ok(graph_from_triples(parse(text, base)?))
}

/// Parses a SHACL Compact Syntax document into its shapes triples (resolving
/// relative IRIs against `base`). The lower-level seam behind [`parse_to_graph`].
pub fn parse(text: &str, base: &str) -> Result<Vec<Triple>, ScsError> {
    let tokens = lex(text)?;
    Parser::new(tokens, base).document()
}

// ───────────────────────────── lexer ─────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    /// `<...>` — the raw (un-resolved) IRI text between the angle brackets.
    IriRef(String),
    /// `prefix:local` or `prefix:` (PNAME_NS has an empty local part).
    PName(String, String),
    /// `@prefix:local`, `@prefix:` shape references.
    AtPName(String, String),
    /// `@<iri>` shape reference.
    AtIri(String),
    /// A bareword: a keyword (`BASE`, `shape`, `IRI`, `targetNode`, …) — the
    /// parser disambiguates keyword vs. param by position.
    Word(String),
    /// A quoted string (already unescaped) plus optional `@lang` / `^^dt`.
    Str(String, StrSuffix),
    Int(String),
    Decimal(String),
    Double(String),
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    LParen,
    RParen,
    Arrow,
    Dot,
    DotDot,
    Pipe,
    Slash,
    Caret,
    Bang,
    Question,
    Star,
    Plus,
    Eq,
}

#[derive(Debug, Clone, PartialEq)]
enum StrSuffix {
    None,
    Lang(String),
    Datatype(Box<IriOrPrefixed>),
}

#[derive(Debug, Clone, PartialEq)]
enum IriOrPrefixed {
    Iri(String),
    Prefixed(String, String),
}

#[derive(Debug, Clone)]
struct Spanned {
    tok: Tok,
    line: usize,
}

fn lex(text: &str) -> Result<Vec<Spanned>, ScsError> {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let mut line = 1usize;
    let mut out = Vec::new();
    let n = chars.len();

    macro_rules! push {
        ($t:expr) => {
            out.push(Spanned { tok: $t, line })
        };
    }

    while i < n {
        let c = chars[i];
        match c {
            '\n' => {
                line += 1;
                i += 1;
            }
            ' ' | '\t' | '\r' => i += 1,
            '#' => {
                while i < n && chars[i] != '\n' {
                    i += 1;
                }
            }
            '{' => {
                push!(Tok::LBrace);
                i += 1;
            }
            '}' => {
                push!(Tok::RBrace);
                i += 1;
            }
            '[' => {
                push!(Tok::LBracket);
                i += 1;
            }
            ']' => {
                push!(Tok::RBracket);
                i += 1;
            }
            '(' => {
                push!(Tok::LParen);
                i += 1;
            }
            ')' => {
                push!(Tok::RParen);
                i += 1;
            }
            '|' => {
                push!(Tok::Pipe);
                i += 1;
            }
            '/' => {
                push!(Tok::Slash);
                i += 1;
            }
            '^' => {
                // `^^` only appears glued to a string literal (handled there);
                // a bare `^` is path-inverse.
                push!(Tok::Caret);
                i += 1;
            }
            '!' => {
                push!(Tok::Bang);
                i += 1;
            }
            '?' => {
                push!(Tok::Question);
                i += 1;
            }
            '*' => {
                push!(Tok::Star);
                i += 1;
            }
            '=' => {
                push!(Tok::Eq);
                i += 1;
            }
            '.' if i + 1 < n && chars[i + 1] == '.' => {
                push!(Tok::DotDot);
                i += 2;
            }
            // A `.` that begins a DECIMAL/DOUBLE (e.g. `.5`) is a number, else `.`.
            '.' if i + 1 < n && chars[i + 1].is_ascii_digit() => {
                let (tok, ni) = lex_number(&chars, i, line)?;
                push!(tok);
                i = ni;
            }
            '.' => {
                push!(Tok::Dot);
                i += 1;
            }
            '-' if i + 1 < n && chars[i + 1] == '>' => {
                push!(Tok::Arrow);
                i += 2;
            }
            '+' if !(i + 1 < n && chars[i + 1].is_ascii_digit()) => {
                // `+` path-mod (a `+number` is a signed numeric literal).
                push!(Tok::Plus);
                i += 1;
            }
            '<' => {
                let (iri, ni) = lex_iriref(&chars, i, line)?;
                push!(Tok::IriRef(iri));
                i = ni;
            }
            '"' | '\'' => {
                let (s, ni) = lex_string(&chars, i, line)?;
                i = ni;
                let (suffix, ni2) = lex_str_suffix(&chars, i, line)?;
                i = ni2;
                push!(Tok::Str(s, suffix));
            }
            '@' => {
                // ATPNAME_LN / ATPNAME_NS / `@<iri>` — a shape reference.
                let (tok, ni) = lex_at(&chars, i, line)?;
                push!(tok);
                i = ni;
            }
            c if c.is_ascii_digit() || c == '+' || c == '-' => {
                let (tok, ni) = lex_number(&chars, i, line)?;
                push!(tok);
                i = ni;
            }
            _ => {
                // Either a prefixed name (`pfx:local`, `pfx:`) or a bareword
                // keyword/param (`BASE`, `shape`, `IRI`, `targetNode`, …).
                let (tok, ni) = lex_name_or_word(&chars, i, line)?;
                push!(tok);
                i = ni;
            }
        }
    }
    Ok(out)
}

fn lex_iriref(chars: &[char], start: usize, line: usize) -> Result<(String, usize), ScsError> {
    // chars[start] == '<'
    let mut i = start + 1;
    let mut s = String::new();
    while i < chars.len() {
        let c = chars[i];
        if c == '>' {
            return Ok((s, i + 1));
        }
        if c == '\\' && i + 1 < chars.len() && (chars[i + 1] == 'u' || chars[i + 1] == 'U') {
            // \uXXXX / \UXXXXXXXX in IRIs.
            let hexlen = if chars[i + 1] == 'u' { 4 } else { 8 };
            let hex: String = chars[i + 2..(i + 2 + hexlen).min(chars.len())]
                .iter()
                .collect();
            let cp = u32::from_str_radix(&hex, 16)
                .ok()
                .and_then(char::from_u32)
                .ok_or_else(|| err(line, "invalid \\u escape in IRI"))?;
            s.push(cp);
            i += 2 + hexlen;
            continue;
        }
        s.push(c);
        i += 1;
    }
    Err(err(line, "unterminated <IRI>"))
}

fn lex_string(chars: &[char], start: usize, line: usize) -> Result<(String, usize), ScsError> {
    let quote = chars[start];
    // Long string (triple-quoted)?
    let long = start + 2 < chars.len() && chars[start + 1] == quote && chars[start + 2] == quote;
    let (delim_len, mut i) = if long { (3, start + 3) } else { (1, start + 1) };
    let mut s = String::new();
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            let e = chars[i + 1];
            match e {
                't' => s.push('\t'),
                'b' => s.push('\u{0008}'),
                'n' => s.push('\n'),
                'r' => s.push('\r'),
                'f' => s.push('\u{000C}'),
                '\\' => s.push('\\'),
                '"' => s.push('"'),
                '\'' => s.push('\''),
                'u' | 'U' => {
                    let hexlen = if e == 'u' { 4 } else { 8 };
                    let hex: String = chars[i + 2..(i + 2 + hexlen).min(chars.len())]
                        .iter()
                        .collect();
                    let cp = u32::from_str_radix(&hex, 16)
                        .ok()
                        .and_then(char::from_u32)
                        .ok_or_else(|| err(line, "invalid \\u escape in string"))?;
                    s.push(cp);
                    i += 2 + hexlen;
                    continue;
                }
                _ => return Err(err(line, "invalid string escape")),
            }
            i += 2;
            continue;
        }
        // End delimiter?
        if c == quote {
            if long {
                if i + 2 < chars.len() && chars[i + 1] == quote && chars[i + 2] == quote {
                    return Ok((s, i + delim_len));
                }
                // a lone quote inside a long string
                s.push(c);
                i += 1;
                continue;
            }
            return Ok((s, i + delim_len));
        }
        s.push(c);
        i += 1;
    }
    Err(err(line, "unterminated string literal"))
}

/// After a closing string quote: optional `@lang` or `^^datatype`.
fn lex_str_suffix(
    chars: &[char],
    start: usize,
    line: usize,
) -> Result<(StrSuffix, usize), ScsError> {
    let n = chars.len();
    if start < n && chars[start] == '@' {
        let mut i = start + 1;
        let s0 = i;
        while i < n && (chars[i].is_ascii_alphanumeric() || chars[i] == '-') {
            i += 1;
        }
        if i == s0 {
            return Err(err(line, "empty language tag"));
        }
        let lang: String = chars[s0..i].iter().collect();
        return Ok((StrSuffix::Lang(lang), i));
    }
    if start + 1 < n && chars[start] == '^' && chars[start + 1] == '^' {
        let i = start + 2;
        // datatype is an iri (IRIREF | prefixedName)
        if i < n && chars[i] == '<' {
            let (iri, ni) = lex_iriref(chars, i, line)?;
            return Ok((StrSuffix::Datatype(Box::new(IriOrPrefixed::Iri(iri))), ni));
        }
        let (tok, ni) = lex_name_or_word(chars, i, line)?;
        match tok {
            Tok::PName(p, l) => Ok((
                StrSuffix::Datatype(Box::new(IriOrPrefixed::Prefixed(p, l))),
                ni,
            )),
            _ => Err(err(line, "expected datatype IRI after ^^")),
        }
    } else {
        Ok((StrSuffix::None, start))
    }
}

fn lex_at(chars: &[char], start: usize, line: usize) -> Result<(Tok, usize), ScsError> {
    // chars[start] == '@'
    let i = start + 1;
    if i < chars.len() && chars[i] == '<' {
        let (iri, ni) = lex_iriref(chars, i, line)?;
        return Ok((Tok::AtIri(iri), ni));
    }
    // ATPNAME_NS / ATPNAME_LN — reuse the name lexer on the part after '@'.
    let (tok, ni) = lex_name_or_word(chars, i, line)?;
    match tok {
        Tok::PName(p, l) => Ok((Tok::AtPName(p, l), ni)),
        _ => Err(err(line, "expected prefixed name after @")),
    }
}

fn lex_number(chars: &[char], start: usize, line: usize) -> Result<(Tok, usize), ScsError> {
    let n = chars.len();
    let mut i = start;
    let mut s = String::new();
    if i < n && (chars[i] == '+' || chars[i] == '-') {
        s.push(chars[i]);
        i += 1;
    }
    let mut saw_dot = false;
    let mut saw_exp = false;
    while i < n {
        let c = chars[i];
        // A '.' is a decimal point only as the first dot, before any exponent,
        // and not the `..` count-range token.
        let dot_dot = i + 1 < n && chars[i + 1] == '.';
        if c.is_ascii_digit() {
            s.push(c);
            i += 1;
        } else if c == '.' && !saw_dot && !saw_exp && !dot_dot {
            saw_dot = true;
            s.push(c);
            i += 1;
        } else if (c == 'e' || c == 'E') && !saw_exp {
            saw_exp = true;
            s.push(c);
            i += 1;
            if i < n && (chars[i] == '+' || chars[i] == '-') {
                s.push(chars[i]);
                i += 1;
            }
        } else {
            break;
        }
    }
    if s.is_empty() || s == "+" || s == "-" {
        return Err(err(line, "malformed numeric literal"));
    }
    let tok = if saw_exp {
        Tok::Double(s)
    } else if saw_dot {
        Tok::Decimal(s)
    } else {
        Tok::Int(s)
    };
    Ok((tok, i))
}

/// PN_CHARS-ish start predicate (intentionally lenient — accepts ASCII plus the
/// common Unicode name characters the SCS fixtures use; the full PN_CHARS_BASE
/// Unicode ranges are admitted via the `> 0x7F` catch-all).
fn is_name_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || (c as u32) > 0x7F
}

fn is_name_char(c: char) -> bool {
    is_name_start(c) || c.is_ascii_digit() || c == '-' || c == '.' || c == '\u{00B7}'
}

fn lex_name_or_word(chars: &[char], start: usize, line: usize) -> Result<(Tok, usize), ScsError> {
    let n = chars.len();
    let mut i = start;
    // Read the prefix part (PN_PREFIX or a bareword). May be empty (for `:local`).
    let pstart = i;
    while i < n && is_name_char(chars[i]) {
        // A trailing '.' that is actually the statement terminator must not be
        // swallowed: '.' counts as a name char only if a name char follows it.
        if chars[i] == '.' && !(i + 1 < n && is_name_char(chars[i + 1])) {
            break;
        }
        i += 1;
    }
    let prefix: String = chars[pstart..i].iter().collect();

    if i < n && chars[i] == ':' {
        // prefixed name: `prefix` is the namespace, read the optional local part.
        i += 1;
        let lstart = i;
        while i < n && is_local_char(chars[i]) {
            if chars[i] == '.' && !(i + 1 < n && is_local_char(chars[i + 1])) {
                break;
            }
            // PN_LOCAL_ESC: a backslash escapes the next char into the local part.
            if chars[i] == '\\' && i + 1 < n {
                i += 2;
                continue;
            }
            i += 1;
        }
        let local: String = chars[lstart..i].iter().collect();
        return Ok((Tok::PName(prefix, local), i));
    }

    if prefix.is_empty() {
        return Err(err(line, &format!("unexpected character '{}'", chars[start])));
    }
    Ok((Tok::Word(prefix), i))
}

fn is_local_char(c: char) -> bool {
    is_name_char(c) || c == ':' || c == '%' || c == '\\'
}

fn err(line: usize, msg: &str) -> ScsError {
    ScsError {
        line,
        message: msg.to_string(),
    }
}

// ───────────────────────────── parser ─────────────────────────────

struct Parser {
    toks: Vec<Spanned>,
    pos: usize,
    base: String,
    prefixes: FxHashMap<String, String>,
    triples: Vec<Triple>,
}

impl Parser {
    fn new(toks: Vec<Spanned>, base: &str) -> Self {
        let mut prefixes = FxHashMap::default();
        // The four implicit prefixes the SCS spec predefines.
        prefixes.insert("rdf".to_string(), RDF.to_string());
        prefixes.insert("rdfs".to_string(), RDFS.to_string());
        prefixes.insert("sh".to_string(), SH.to_string());
        prefixes.insert("xsd".to_string(), XSD.to_string());
        Parser {
            toks,
            pos: 0,
            base: base.to_string(),
            prefixes,
            triples: Vec::new(),
        }
    }

    fn line(&self) -> usize {
        self.toks
            .get(self.pos)
            .or_else(|| self.toks.last())
            .map(|s| s.line)
            .unwrap_or(0)
    }

    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|s| &s.tok)
    }

    fn peek2(&self) -> Option<&Tok> {
        self.toks.get(self.pos + 1).map(|s| &s.tok)
    }

    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).map(|s| s.tok.clone());
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn expect(&mut self, t: &Tok, what: &str) -> Result<(), ScsError> {
        if self.peek() == Some(t) {
            self.pos += 1;
            Ok(())
        } else {
            Err(err(self.line(), &format!("expected {}", what)))
        }
    }

    fn add(&mut self, s: NamedOrBlankNode, p: &str, o: Term) {
        self.triples.push(Triple {
            subject: s,
            predicate: NamedNode::new_unchecked(p),
            object: o,
        });
    }

    fn fresh_bnode(&self) -> BlankNode {
        BlankNode::default()
    }

    /// shaclDoc : directive* (nodeShape|shapeClass)* EOF
    fn document(mut self) -> Result<Vec<Triple>, ScsError> {
        // directives
        loop {
            match self.peek() {
                Some(Tok::Word(w)) if w == "BASE" => self.base_decl()?,
                Some(Tok::Word(w)) if w == "IMPORTS" => self.imports_decl()?,
                Some(Tok::Word(w)) if w == "PREFIX" => self.prefix_decl()?,
                _ => break,
            }
        }
        // (sq-0g57x) The document IRI is the empty reference resolved
        // against the final base, so RFC 3986 resolution removes its fragment.
        // Imports were already emitted against the base in effect at each directive.
        let base_term = Term::NamedNode(NamedNode::new_unchecked(self.resolve("")));
        self.add(
            iri_subject(&base_term)?,
            RDF_TYPE,
            Term::NamedNode(NamedNode::new_unchecked(format!("{}{}", OWL, "Ontology"))),
        );
        // shapes
        loop {
            match self.peek() {
                Some(Tok::Word(w)) if w == "shapeClass" => self.shape_class()?,
                Some(Tok::Word(w)) if w == "shape" => self.node_shape()?,
                None => break,
                _ => {
                    return Err(err(
                        self.line(),
                        "expected `shape` or `shapeClass` (or end of document)",
                    ))
                }
            }
        }
        Ok(self.triples)
    }

    fn base_decl(&mut self) -> Result<(), ScsError> {
        self.next(); // BASE
        let iri = self.iriref_only("BASE IRI")?;
        self.base = self.resolve(&iri);
        Ok(())
    }

    fn imports_decl(&mut self) -> Result<(), ScsError> {
        self.next(); // IMPORTS
        let iri = self.iriref_only("IMPORTS IRI")?;
        let target = self.resolve(&iri);
        let subj = Term::NamedNode(NamedNode::new_unchecked(&self.base));
        self.add(
            iri_subject(&subj)?,
            &format!("{}{}", OWL, "imports"),
            Term::NamedNode(NamedNode::new_unchecked(&target)),
        );
        Ok(())
    }

    fn prefix_decl(&mut self) -> Result<(), ScsError> {
        self.next(); // PREFIX
        // PNAME_NS : `pfx:` (empty local).
        let pfx = match self.next() {
            Some(Tok::PName(p, l)) if l.is_empty() => p,
            _ => return Err(err(self.line(), "expected `prefix:` after PREFIX")),
        };
        let iri = self.iriref_only("prefix IRI")?;
        let resolved = self.resolve(&iri);
        self.prefixes.insert(pfx, resolved);
        Ok(())
    }

    fn iriref_only(&mut self, what: &str) -> Result<String, ScsError> {
        match self.next() {
            Some(Tok::IriRef(s)) => Ok(s),
            _ => Err(err(self.line(), &format!("expected <{}>", what))),
        }
    }

    /// shapeClass : 'shapeClass' iri nodeShapeBody
    fn shape_class(&mut self) -> Result<(), ScsError> {
        self.next(); // shapeClass
        let shape = self.iri()?;
        let subj = iri_subject(&shape)?;
        self.add(
            subj.clone(),
            RDF_TYPE,
            Term::NamedNode(NamedNode::new_unchecked(format!("{}{}", SH, "NodeShape"))),
        );
        self.add(
            subj.clone(),
            RDF_TYPE,
            Term::NamedNode(NamedNode::new_unchecked(format!("{}{}", RDFS, "Class"))),
        );
        self.node_shape_body(&subj)?;
        Ok(())
    }

    /// nodeShape : 'shape' iri targetClass? nodeShapeBody
    fn node_shape(&mut self) -> Result<(), ScsError> {
        self.next(); // shape
        let shape = self.iri()?;
        let subj = iri_subject(&shape)?;
        self.add(
            subj.clone(),
            RDF_TYPE,
            Term::NamedNode(NamedNode::new_unchecked(format!("{}{}", SH, "NodeShape"))),
        );
        // targetClass : '->' iri+
        if self.peek() == Some(&Tok::Arrow) {
            self.next();
            let mut got = false;
            while self.is_iri_start() {
                let c = self.iri()?;
                self.add(subj.clone(), &format!("{}{}", SH, "targetClass"), c);
                got = true;
            }
            if !got {
                return Err(err(self.line(), "expected target class IRI after `->`"));
            }
        }
        self.node_shape_body(&subj)?;
        Ok(())
    }

    /// nodeShapeBody : '{' constraint* '}'
    fn node_shape_body(&mut self, subj: &NamedOrBlankNode) -> Result<(), ScsError> {
        self.expect(&Tok::LBrace, "`{`")?;
        while self.peek() != Some(&Tok::RBrace) {
            if self.peek().is_none() {
                return Err(err(self.line(), "unterminated `{ ... }` shape body"));
            }
            self.constraint(subj)?;
        }
        self.expect(&Tok::RBrace, "`}`")?;
        Ok(())
    }

    /// constraint : ( nodeOr+ | propertyShape ) '.'
    ///
    /// Disambiguation: a constraint is a node-level constraint when it starts
    /// with a `param=` (`nodeParam '='`) or a `!` negation thereof; otherwise it
    /// is a property shape (which starts with a path).
    fn constraint(&mut self, subj: &NamedOrBlankNode) -> Result<(), ScsError> {
        if self.starts_node_constraint() {
            // nodeOr+ — apply each disjunction group directly on the shape.
            while self.starts_node_constraint() {
                self.node_or(subj)?;
            }
            self.expect(&Tok::Dot, "`.` after node constraint")?;
        } else {
            self.property_shape(subj)?;
            self.expect(&Tok::Dot, "`.` after property shape")?;
        }
        Ok(())
    }

    /// Does the current position begin a node constraint (`nodeOr`)? True iff the
    /// next token is `!` (negation) or a `nodeParam` bareword immediately
    /// followed by `=`.
    fn starts_node_constraint(&self) -> bool {
        match self.peek() {
            Some(Tok::Bang) => true,
            Some(Tok::Word(w)) => is_node_param(w) && self.peek2() == Some(&Tok::Eq),
            _ => false,
        }
    }

    /// nodeOr : nodeNot ('|' nodeNot)*
    ///
    /// A single `nodeNot` applies its constraints directly on `subj`; two or
    /// more become an `sh:or ( ... )` of singleton shapes.
    fn node_or(&mut self, subj: &NamedOrBlankNode) -> Result<(), ScsError> {
        let mut branches: Vec<NamedOrBlankNode> = Vec::new();
        let first = self.fresh_bnode();
        let first_s = NamedOrBlankNode::BlankNode(first);
        self.node_not(&first_s)?;
        branches.push(first_s);
        while self.peek() == Some(&Tok::Pipe) {
            self.next();
            let b = self.fresh_bnode();
            let bs = NamedOrBlankNode::BlankNode(b);
            self.node_not(&bs)?;
            branches.push(bs);
        }
        if branches.len() == 1 {
            // Splice the singleton branch's triples onto `subj` directly.
            self.rehome(&branches[0], subj);
        } else {
            let list = self.make_list(branches.into_iter().map(blank_or_named_to_term).collect());
            self.add(subj.clone(), &format!("{}{}", SH, "or"), list);
        }
        Ok(())
    }

    /// nodeNot : negation? nodeValue ; nodeValue : nodeParam '=' iriOrLiteralOrArray
    fn node_not(&mut self, subj: &NamedOrBlankNode) -> Result<(), ScsError> {
        let negated = self.peek() == Some(&Tok::Bang);
        if negated {
            self.next();
            let inner = self.fresh_bnode();
            let inner_s = NamedOrBlankNode::BlankNode(inner.clone());
            self.node_value(&inner_s)?;
            self.add(
                subj.clone(),
                &format!("{}{}", SH, "not"),
                Term::BlankNode(inner),
            );
        } else {
            self.node_value(subj)?;
        }
        Ok(())
    }

    fn node_value(&mut self, subj: &NamedOrBlankNode) -> Result<(), ScsError> {
        let param = match self.next() {
            Some(Tok::Word(w)) if is_node_param(&w) => w,
            _ => return Err(err(self.line(), "expected node parameter")),
        };
        self.expect(&Tok::Eq, "`=`")?;
        self.param_value(subj, &param)
    }

    /// propertyShape : path (propertyCount | propertyOr)*
    fn property_shape(&mut self, parent: &NamedOrBlankNode) -> Result<(), ScsError> {
        let prop = self.fresh_bnode();
        let prop_s = NamedOrBlankNode::BlankNode(prop.clone());
        // sh:path
        let path_term = self.path()?;
        self.add(prop_s.clone(), &format!("{}{}", SH, "path"), path_term);
        // params / counts / or-groups
        loop {
            match self.peek() {
                Some(Tok::LBracket) => self.property_count(&prop_s)?,
                _ if self.starts_property_atom_or_or() => self.property_or(&prop_s)?,
                _ => break,
            }
        }
        self.add(
            parent.clone(),
            &format!("{}{}", SH, "property"),
            Term::BlankNode(prop),
        );
        Ok(())
    }

    /// propertyCount : '[' propertyMinCount '..' propertyMaxCount ']'
    fn property_count(&mut self, prop: &NamedOrBlankNode) -> Result<(), ScsError> {
        self.expect(&Tok::LBracket, "`[`")?;
        let min = match self.next() {
            Some(Tok::Int(s)) => s,
            _ => return Err(err(self.line(), "expected min count integer")),
        };
        self.expect(&Tok::DotDot, "`..`")?;
        let max = match self.peek() {
            Some(Tok::Int(_)) => match self.next() {
                Some(Tok::Int(s)) => Some(s),
                _ => unreachable!(),
            },
            Some(Tok::Star) => {
                self.next();
                None
            }
            _ => return Err(err(self.line(), "expected max count integer or `*`")),
        };
        self.expect(&Tok::RBracket, "`]`")?;
        // sh:minCount when min != 0; sh:maxCount when max is bounded.
        if min != "0" {
            self.add(
                prop.clone(),
                &format!("{}{}", SH, "minCount"),
                integer_literal(&min),
            );
        }
        if let Some(m) = max {
            self.add(
                prop.clone(),
                &format!("{}{}", SH, "maxCount"),
                integer_literal(&m),
            );
        }
        Ok(())
    }

    fn starts_property_atom_or_or(&self) -> bool {
        match self.peek() {
            Some(Tok::Bang) => true,
            Some(Tok::Word(w)) => {
                // A bareword in property-atom position is a nodeKind keyword or a
                // `propertyParam=` constraint. (propertyType IRIs lex as
                // PName/IriRef, never a bareword.)
                is_node_kind(w) || (is_property_param(w) && self.peek2() == Some(&Tok::Eq))
            }
            Some(Tok::PName(..)) | Some(Tok::IriRef(_)) => true, // propertyType (sh:class)
            Some(Tok::AtPName(..)) | Some(Tok::AtIri(_)) => true, // shapeRef
            Some(Tok::LBrace) => true,                           // nested nodeShapeBody
            _ => false,
        }
    }

    /// propertyOr : propertyNot ('|' propertyNot)*
    fn property_or(&mut self, prop: &NamedOrBlankNode) -> Result<(), ScsError> {
        let mut branches: Vec<NamedOrBlankNode> = Vec::new();
        let first = self.fresh_bnode();
        let first_s = NamedOrBlankNode::BlankNode(first);
        self.property_not(&first_s)?;
        branches.push(first_s);
        while self.peek() == Some(&Tok::Pipe) {
            self.next();
            let b = self.fresh_bnode();
            let bs = NamedOrBlankNode::BlankNode(b);
            self.property_not(&bs)?;
            branches.push(bs);
        }
        if branches.len() == 1 {
            self.rehome(&branches[0], prop);
        } else {
            let list = self.make_list(branches.into_iter().map(blank_or_named_to_term).collect());
            self.add(prop.clone(), &format!("{}{}", SH, "or"), list);
        }
        Ok(())
    }

    /// propertyNot : negation? propertyAtom
    fn property_not(&mut self, subj: &NamedOrBlankNode) -> Result<(), ScsError> {
        let negated = self.peek() == Some(&Tok::Bang);
        if negated {
            self.next();
            let inner = self.fresh_bnode();
            let inner_s = NamedOrBlankNode::BlankNode(inner.clone());
            self.property_atom(&inner_s)?;
            self.add(
                subj.clone(),
                &format!("{}{}", SH, "not"),
                Term::BlankNode(inner),
            );
        } else {
            self.property_atom(subj)?;
        }
        Ok(())
    }

    /// propertyAtom : propertyType | nodeKind | shapeRef | propertyValue | nodeShapeBody
    fn property_atom(&mut self, subj: &NamedOrBlankNode) -> Result<(), ScsError> {
        match self.peek() {
            Some(Tok::Word(w)) if is_node_kind(w) => {
                let kind = w.clone();
                self.next();
                self.add(
                    subj.clone(),
                    &format!("{}{}", SH, "nodeKind"),
                    Term::NamedNode(NamedNode::new_unchecked(format!("{}{}", SH, kind))),
                );
                Ok(())
            }
            Some(Tok::Word(w)) if is_property_param(w) && self.peek2() == Some(&Tok::Eq) => {
                let param = w.clone();
                self.next();
                self.next(); // '='
                self.param_value(subj, &param)
            }
            Some(Tok::AtPName(..)) | Some(Tok::AtIri(_)) => {
                // shapeRef : sh:node
                let target = self.shape_ref()?;
                self.add(subj.clone(), &format!("{}{}", SH, "node"), target);
                Ok(())
            }
            Some(Tok::LBrace) => {
                // nested nodeShapeBody : sh:node [ ... ]
                let nested = self.fresh_bnode();
                let nested_s = NamedOrBlankNode::BlankNode(nested.clone());
                self.node_shape_body(&nested_s)?;
                self.add(
                    subj.clone(),
                    &format!("{}{}", SH, "node"),
                    Term::BlankNode(nested),
                );
                Ok(())
            }
            Some(Tok::PName(..)) | Some(Tok::IriRef(_)) => {
                // propertyType : a bare IRI. Per the W3C SCS rule it maps to
                // `sh:datatype` when it is one of the RDF datatypes supported by
                // SPARQL 1.1 (e.g. `xsd:string`, `rdf:langString`), else
                // `sh:class`.
                let term = self.iri()?;
                let pred = match &term {
                    Term::NamedNode(n) if is_sparql_datatype(n.as_str()) => "datatype",
                    _ => "class",
                };
                self.add(subj.clone(), &format!("{}{}", SH, pred), term);
                Ok(())
            }
            _ => Err(err(self.line(), "expected property constraint atom")),
        }
    }

    fn shape_ref(&mut self) -> Result<Term, ScsError> {
        match self.next() {
            Some(Tok::AtPName(p, l)) => self.prefixed(&p, &l),
            Some(Tok::AtIri(iri)) => Ok(Term::NamedNode(NamedNode::new_unchecked(
                self.resolve(&iri),
            ))),
            _ => Err(err(self.line(), "expected shape reference (@...)")),
        }
    }

    // ──────────────── path expressions ────────────────

    /// path : pathAlternative ; returns the SHACL path term (IRI or blank-node).
    fn path(&mut self) -> Result<Term, ScsError> {
        self.path_alternative()
    }

    /// pathAlternative : pathSequence ('|' pathSequence)*
    fn path_alternative(&mut self) -> Result<Term, ScsError> {
        let mut seqs = vec![self.path_sequence()?];
        while self.peek() == Some(&Tok::Pipe) {
            self.next();
            seqs.push(self.path_sequence()?);
        }
        if seqs.len() == 1 {
            Ok(seqs.pop().unwrap())
        } else {
            let list = self.make_list(seqs);
            let b = self.fresh_bnode();
            self.add(
                NamedOrBlankNode::BlankNode(b.clone()),
                &format!("{}{}", SH, "alternativePath"),
                list,
            );
            Ok(Term::BlankNode(b))
        }
    }

    /// pathSequence : pathEltOrInverse ('/' pathEltOrInverse)*
    fn path_sequence(&mut self) -> Result<Term, ScsError> {
        let mut elts = vec![self.path_elt_or_inverse()?];
        while self.peek() == Some(&Tok::Slash) {
            self.next();
            elts.push(self.path_elt_or_inverse()?);
        }
        if elts.len() == 1 {
            Ok(elts.pop().unwrap())
        } else {
            // A sequence path is a plain RDF list of the member paths.
            Ok(self.make_list(elts))
        }
    }

    /// pathEltOrInverse : pathElt | pathInverse pathElt
    fn path_elt_or_inverse(&mut self) -> Result<Term, ScsError> {
        if self.peek() == Some(&Tok::Caret) {
            self.next();
            let inner = self.path_elt()?;
            let b = self.fresh_bnode();
            self.add(
                NamedOrBlankNode::BlankNode(b.clone()),
                &format!("{}{}", SH, "inversePath"),
                inner,
            );
            Ok(Term::BlankNode(b))
        } else {
            self.path_elt()
        }
    }

    /// pathElt : pathPrimary pathMod?
    fn path_elt(&mut self) -> Result<Term, ScsError> {
        let primary = self.path_primary()?;
        let modifier = match self.peek() {
            Some(Tok::Question) => Some("zeroOrOnePath"),
            Some(Tok::Star) => Some("zeroOrMorePath"),
            Some(Tok::Plus) => Some("oneOrMorePath"),
            _ => None,
        };
        if let Some(m) = modifier {
            self.next();
            let b = self.fresh_bnode();
            self.add(
                NamedOrBlankNode::BlankNode(b.clone()),
                &format!("{}{}", SH, m),
                primary,
            );
            Ok(Term::BlankNode(b))
        } else {
            Ok(primary)
        }
    }

    /// pathPrimary : iri | '(' path ')'
    fn path_primary(&mut self) -> Result<Term, ScsError> {
        if self.peek() == Some(&Tok::LParen) {
            self.next();
            let p = self.path()?;
            self.expect(&Tok::RParen, "`)`")?;
            Ok(p)
        } else {
            self.iri()
        }
    }

    // ──────────────── values / arrays / literals ────────────────

    /// param_value : iriOrLiteralOrArray, emitting `subj param value`.
    /// Arrays become an RDF list.
    fn param_value(&mut self, subj: &NamedOrBlankNode, param: &str) -> Result<(), ScsError> {
        let pred = param_predicate(param);
        if self.peek() == Some(&Tok::LBracket) {
            let items = self.array()?;
            let list = self.make_list(items);
            self.add(subj.clone(), &pred, list);
        } else {
            let v = self.iri_or_literal()?;
            self.add(subj.clone(), &pred, v);
        }
        Ok(())
    }

    /// array : '[' iriOrLiteral* ']'
    fn array(&mut self) -> Result<Vec<Term>, ScsError> {
        self.expect(&Tok::LBracket, "`[`")?;
        let mut items = Vec::new();
        while self.peek() != Some(&Tok::RBracket) {
            if self.peek().is_none() {
                return Err(err(self.line(), "unterminated `[ ... ]` array"));
            }
            items.push(self.iri_or_literal()?);
        }
        self.expect(&Tok::RBracket, "`]`")?;
        Ok(items)
    }

    fn iri_or_literal(&mut self) -> Result<Term, ScsError> {
        match self.peek() {
            Some(Tok::IriRef(_)) | Some(Tok::PName(..)) => self.iri(),
            Some(Tok::Str(..)) => self.rdf_literal(),
            Some(Tok::Int(_)) | Some(Tok::Decimal(_)) | Some(Tok::Double(_)) => {
                self.numeric_literal()
            }
            Some(Tok::Word(w)) if w == "true" || w == "false" => {
                let v = w.clone();
                self.next();
                Ok(boolean_literal(&v))
            }
            _ => Err(err(self.line(), "expected IRI or literal")),
        }
    }

    fn rdf_literal(&mut self) -> Result<Term, ScsError> {
        match self.next() {
            Some(Tok::Str(s, suffix)) => match suffix {
                StrSuffix::None => Ok(Term::Literal(Literal::new_simple_literal(s))),
                StrSuffix::Lang(l) => Ok(Term::Literal(
                    Literal::new_language_tagged_literal(s, l)
                        .map_err(|e| err(self.line(), &e.to_string()))?,
                )),
                StrSuffix::Datatype(dt) => {
                    let dt_iri = match *dt {
                        IriOrPrefixed::Iri(iri) => self.resolve(&iri),
                        IriOrPrefixed::Prefixed(p, l) => self.prefixed_iri(&p, &l)?,
                    };
                    Ok(Term::Literal(Literal::new_typed_literal(
                        s,
                        NamedNode::new_unchecked(&dt_iri),
                    )))
                }
            },
            _ => Err(err(self.line(), "expected string literal")),
        }
    }

    fn numeric_literal(&mut self) -> Result<Term, ScsError> {
        match self.next() {
            Some(Tok::Int(s)) => Ok(integer_literal(&s)),
            Some(Tok::Decimal(s)) => Ok(Term::Literal(Literal::new_typed_literal(
                s,
                NamedNode::new_unchecked(format!("{}{}", XSD, "decimal")),
            ))),
            Some(Tok::Double(s)) => Ok(Term::Literal(Literal::new_typed_literal(
                s,
                NamedNode::new_unchecked(format!("{}{}", XSD, "double")),
            ))),
            _ => Err(err(self.line(), "expected numeric literal")),
        }
    }

    // ──────────────── iri / prefixed names ────────────────

    fn is_iri_start(&self) -> bool {
        matches!(self.peek(), Some(Tok::IriRef(_)) | Some(Tok::PName(..)))
    }

    fn iri(&mut self) -> Result<Term, ScsError> {
        match self.next() {
            Some(Tok::IriRef(iri)) => {
                Ok(Term::NamedNode(NamedNode::new_unchecked(self.resolve(&iri))))
            }
            Some(Tok::PName(p, l)) => self.prefixed(&p, &l),
            _ => Err(err(self.line(), "expected IRI or prefixed name")),
        }
    }

    fn prefixed(&self, prefix: &str, local: &str) -> Result<Term, ScsError> {
        Ok(Term::NamedNode(NamedNode::new_unchecked(
            &self.prefixed_iri(prefix, local)?,
        )))
    }

    fn prefixed_iri(&self, prefix: &str, local: &str) -> Result<String, ScsError> {
        match self.prefixes.get(prefix) {
            Some(ns) => Ok(format!("{}{}", ns, unescape_local(local))),
            None => Err(err(self.line(), &format!("undefined prefix `{}:`", prefix))),
        }
    }

    /// Resolves an IRI reference against the current base.
    fn resolve(&self, iri: &str) -> String {
        resolve_against(&self.base, iri)
    }

    // ──────────────── RDF list construction ────────────────

    /// Builds an `rdf:first`/`rdf:rest` list and returns its head term
    /// (`rdf:nil` for the empty list).
    fn make_list(&mut self, items: Vec<Term>) -> Term {
        if items.is_empty() {
            return Term::NamedNode(NamedNode::new_unchecked(RDF_NIL));
        }
        let nodes: Vec<BlankNode> = (0..items.len()).map(|_| self.fresh_bnode()).collect();
        let head = nodes[0].clone();
        for (idx, item) in items.into_iter().enumerate() {
            let cell = NamedOrBlankNode::BlankNode(nodes[idx].clone());
            self.add(cell.clone(), RDF_FIRST, item);
            let rest = if idx + 1 < nodes.len() {
                Term::BlankNode(nodes[idx + 1].clone())
            } else {
                Term::NamedNode(NamedNode::new_unchecked(RDF_NIL))
            };
            self.add(cell, RDF_REST, rest);
        }
        Term::BlankNode(head)
    }

    /// Moves every triple whose subject is `from` (a singleton-disjunction
    /// blank node) onto `to`, so a `nodeOr`/`propertyOr` of one branch attaches
    /// its constraints directly to the shape (no spurious blank node). Triples
    /// where `from` appears as an object (lists, nested shapes) are untouched —
    /// the moved subject triples still reference them correctly.
    fn rehome(&mut self, from: &NamedOrBlankNode, to: &NamedOrBlankNode) {
        let from_term = blank_or_named_to_term(from.clone());
        for t in &mut self.triples {
            if blank_or_named_to_term(t.subject.clone()) == from_term {
                t.subject = to.clone();
            }
        }
    }
}

// ───────────────────────────── helpers ─────────────────────────────

fn iri_subject(t: &Term) -> Result<NamedOrBlankNode, ScsError> {
    match t {
        Term::NamedNode(n) => Ok(NamedOrBlankNode::NamedNode(n.clone())),
        Term::BlankNode(b) => Ok(NamedOrBlankNode::BlankNode(b.clone())),
        _ => Err(err(0, "a literal cannot be a subject")),
    }
}

fn blank_or_named_to_term(s: NamedOrBlankNode) -> Term {
    match s {
        NamedOrBlankNode::NamedNode(n) => Term::NamedNode(n),
        NamedOrBlankNode::BlankNode(b) => Term::BlankNode(b),
    }
}

fn integer_literal(s: &str) -> Term {
    Term::Literal(Literal::new_typed_literal(
        s,
        NamedNode::new_unchecked(format!("{}{}", XSD, "integer")),
    ))
}

fn boolean_literal(s: &str) -> Term {
    Term::Literal(Literal::new_typed_literal(
        s,
        NamedNode::new_unchecked(format!("{}{}", XSD, "boolean")),
    ))
}

/// The `sh:` predicate for a node/property parameter name. The predicate IRI is
/// uniformly `sh:<param>` (booleans like `closed` and lists like `in` differ
/// only in object encoding, handled by the value parser).
fn param_predicate(param: &str) -> String {
    format!("{}{}", SH, param)
}

const NODE_PARAMS: &[&str] = &[
    "targetNode",
    "targetObjectsOf",
    "targetSubjectsOf",
    "deactivated",
    "severity",
    "message",
    "class",
    "datatype",
    "nodeKind",
    "minExclusive",
    "minInclusive",
    "maxExclusive",
    "maxInclusive",
    "minLength",
    "maxLength",
    "pattern",
    "flags",
    "languageIn",
    "equals",
    "disjoint",
    "closed",
    "ignoredProperties",
    "hasValue",
    "in",
];

const PROPERTY_PARAMS: &[&str] = &[
    "deactivated",
    "severity",
    "message",
    "class",
    "datatype",
    "nodeKind",
    "minExclusive",
    "minInclusive",
    "maxExclusive",
    "maxInclusive",
    "minLength",
    "maxLength",
    "pattern",
    "flags",
    "languageIn",
    "uniqueLang",
    "equals",
    "disjoint",
    "lessThan",
    "lessThanOrEquals",
    "qualifiedValueShape",
    "qualifiedMinCount",
    "qualifiedMaxCount",
    "qualifiedValueShapesDisjoint",
    "closed",
    "ignoredProperties",
    "hasValue",
    "in",
];

const NODE_KINDS: &[&str] = &[
    "BlankNode",
    "IRI",
    "Literal",
    "BlankNodeOrIRI",
    "BlankNodeOrLiteral",
    "IRIOrLiteral",
];

fn is_node_param(w: &str) -> bool {
    NODE_PARAMS.contains(&w)
}

fn is_property_param(w: &str) -> bool {
    PROPERTY_PARAMS.contains(&w)
}

fn is_node_kind(w: &str) -> bool {
    NODE_KINDS.contains(&w)
}

/// The RDF datatypes "supported by SPARQL 1.1" — the set the W3C SCS rule uses
/// to decide a bare-IRI `propertyType` between `sh:datatype` and `sh:class`. It
/// is the XSD datatype space SPARQL operates over plus the RDF datatypes
/// (`rdf:langString`, `rdf:HTML`, `rdf:XMLLiteral`). Anything else is a class.
fn is_sparql_datatype(iri: &str) -> bool {
    if let Some(local) = iri.strip_prefix(XSD) {
        return XSD_DATATYPES.contains(&local);
    }
    if let Some(local) = iri.strip_prefix(RDF) {
        return matches!(local, "langString" | "HTML" | "XMLLiteral" | "PlainLiteral");
    }
    false
}

/// XSD datatype local names recognised as datatypes (the full XSD built-in set,
/// so any `xsd:*` built-in used as a property type maps to `sh:datatype`).
const XSD_DATATYPES: &[&str] = &[
    "string",
    "boolean",
    "decimal",
    "integer",
    "double",
    "float",
    "date",
    "time",
    "dateTime",
    "dateTimeStamp",
    "gYear",
    "gMonth",
    "gDay",
    "gYearMonth",
    "gMonthDay",
    "duration",
    "yearMonthDuration",
    "dayTimeDuration",
    "byte",
    "short",
    "int",
    "long",
    "unsignedByte",
    "unsignedShort",
    "unsignedInt",
    "unsignedLong",
    "positiveInteger",
    "nonNegativeInteger",
    "negativeInteger",
    "nonPositiveInteger",
    "hexBinary",
    "base64Binary",
    "anyURI",
    "language",
    "normalizedString",
    "token",
    "NMTOKEN",
    "Name",
    "NCName",
];

/// Undoes PN_LOCAL_ESC backslash-escapes in a prefixed-name local part
/// (`ex:a\-b` → `a-b`). The escapable set is the SCS/Turtle PN_LOCAL_ESC set.
fn unescape_local(local: &str) -> String {
    if !local.contains('\\') {
        return local.to_string();
    }
    let mut out = String::with_capacity(local.len());
    let mut chars = local.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Minimal IRI-reference resolution against `base` covering the cases the SCS
/// fixtures produce: an absolute IRI (has a scheme) is returned unchanged; an
/// empty reference resolves to the base (minus its fragment); a fragment
/// (`#frag`) replaces the base fragment; an absolute-path / network-path ref
/// inherits the base scheme/authority; otherwise the reference replaces the
/// base's last path segment. (Full RFC-3986 dot-segment merging is not needed —
/// the corpus only exercises absolute IRIs, `<>`, and fragment / last-segment
/// relatives.)
fn resolve_against(base: &str, reference: &str) -> String {
    if reference.is_empty() {
        return match base.split_once('#') {
            Some((b, _)) => b.to_string(),
            None => base.to_string(),
        };
    }
    if has_scheme(reference) {
        return reference.to_string();
    }
    if let Some(frag) = reference.strip_prefix('#') {
        let b = base.split_once('#').map(|(b, _)| b).unwrap_or(base);
        return format!("{}#{}", b, frag);
    }
    if reference.starts_with("//") {
        if let Some(scheme) = base.split_once(':').map(|(s, _)| s) {
            return format!("{}:{}", scheme, reference);
        }
        return reference.to_string();
    }
    if reference.starts_with('/') {
        return join_abs_path(base, reference);
    }
    // relative-path reference: replace base's last path segment.
    let base_no_frag = base.split_once('#').map(|(b, _)| b).unwrap_or(base);
    let base_no_query = base_no_frag
        .split_once('?')
        .map(|(b, _)| b)
        .unwrap_or(base_no_frag);
    match base_no_query.rfind('/') {
        Some(idx) => format!("{}{}", &base_no_query[..=idx], reference),
        None => reference.to_string(),
    }
}

fn has_scheme(s: &str) -> bool {
    // scheme = ALPHA *( ALPHA / DIGIT / "+" / "-" / "." ) ":"
    let bytes = s.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_alphabetic() {
        return false;
    }
    let mut i = 1;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b':' {
            return true;
        }
        if !(b.is_ascii_alphanumeric() || b == b'+' || b == b'-' || b == b'.') {
            return false;
        }
        i += 1;
    }
    false
}

fn join_abs_path(base: &str, path: &str) -> String {
    match base.split_once("://") {
        Some((scheme, rest)) => {
            let authority = rest.split('/').next().unwrap_or("");
            format!("{}://{}{}", scheme, authority, path)
        }
        None => match base.split_once(':') {
            Some((scheme, _)) => format!("{}:{}", scheme, path),
            None => path.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_doc_emits_ontology() {
        let g = parse("", DEFAULT_BASE).unwrap();
        assert_eq!(g.len(), 1);
        let t = &g[0];
        assert_eq!(t.predicate.as_str(), RDF_TYPE);
        match &t.subject {
            NamedOrBlankNode::NamedNode(n) => assert_eq!(n.as_str(), DEFAULT_BASE),
            other => panic!("expected named ontology subject, got {:?}", other),
        }
    }

    #[test]
    fn ontology_subject_resolves_empty_reference_against_effective_base() {
        // (sq-0g57x) The document production resolves an empty
        // reference against the final base, which strips any fragment.
        for (source, expected) in [
            ("BASE <#>\n", "urn:x-base:default"),
            (
                "BASE <http://example.org/basic-shape-with-targets#thing>\n",
                "http://example.org/basic-shape-with-targets",
            ),
        ] {
            let triples = parse(source, DEFAULT_BASE).unwrap();
            let ontology = triples
                .iter()
                .find(|triple| {
                    triple.predicate.as_str() == RDF_TYPE
                        && matches!(
                            &triple.object,
                            Term::NamedNode(node)
                                if node.as_str() == "http://www.w3.org/2002/07/owl#Ontology"
                        )
                })
                .expect("document ontology triple");
            match &ontology.subject {
                NamedOrBlankNode::NamedNode(node) => assert_eq!(node.as_str(), expected),
                other => panic!("expected named ontology subject, got {other:?}"),
            }
        }
    }

    #[test]
    fn base_override_resolves_shape() {
        let ts = parse("BASE <http://example.org/x>\nshape <#S> {\n}\n", DEFAULT_BASE).unwrap();
        assert!(ts.iter().any(|t| matches!(&t.subject,
            NamedOrBlankNode::NamedNode(n) if n.as_str() == "http://example.org/x#S")));
    }

    #[test]
    fn undefined_prefix_errors() {
        let e = parse("shape nope:S {\n}\n", DEFAULT_BASE).unwrap_err();
        assert!(e.message.contains("undefined prefix"), "got: {}", e.message);
    }

    #[test]
    fn parsed_shapes_drive_validation() {
        // An SCS shapes graph must feed the validator exactly like the Turtle
        // equivalent: a min-count + datatype property shape with a class target.
        use sparq_core::Graph;
        let shapes = parse_to_graph(
            "PREFIX ex: <http://example.org/>\n\
             shape ex:PersonShape -> ex:Person {\n\
                 ex:age xsd:integer [1..1] .\n\
             }\n",
            DEFAULT_BASE,
        )
        .unwrap();

        // Conforming data.
        let ok = Graph::load_str(
            "@prefix ex: <http://example.org/> .\n ex:alice a ex:Person ; ex:age 30 .",
            "turtle",
        )
        .unwrap();
        assert!(crate::validate(&ok, &shapes).conforms, "expected conformance");

        // Violating data: wrong datatype + missing value handled by the same shape.
        let bad = Graph::load_str(
            "@prefix ex: <http://example.org/> .\n ex:bob a ex:Person ; ex:age \"old\" .",
            "turtle",
        )
        .unwrap();
        assert!(
            !crate::validate(&bad, &shapes).conforms,
            "expected a datatype violation"
        );
    }

    #[test]
    fn unsupported_construct_errors_not_misparses() {
        // A `<` that does not open a complete IRI must be a typed error.
        let e = parse("shape <unterminated {\n}\n", DEFAULT_BASE).unwrap_err();
        assert!(e.message.contains("unterminated") || e.message.contains("IRI"));
    }

    #[test]
    fn resolve_cases() {
        assert_eq!(
            resolve_against("http://e.org/base", "http://other/x"),
            "http://other/x"
        );
        assert_eq!(resolve_against("http://e.org/base", ""), "http://e.org/base");
        assert_eq!(
            resolve_against("http://e.org/path/file", "#f"),
            "http://e.org/path/file#f"
        );
        assert_eq!(
            resolve_against("http://e.org/path/file", "other"),
            "http://e.org/path/other"
        );
        assert_eq!(resolve_against("http://e.org/a/b", "/c"), "http://e.org/c");
    }
}
