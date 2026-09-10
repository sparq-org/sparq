//! [OPUS-4.8] (sq-jocpn) A native, hand-rolled byte-level **Turtle** tokenizer/parser that interns
//! each S/P/O directly into the [`Dict`], modelled on the N-Triples parser in [`crate::nt`]. It is
//! the structural fix for the single-thread Turtle parse gap (sq-wrn61 profiled ~53% of the parse
//! in oxttl's tokenizer/state-machine + prefixed-name expansion), which the pre-sizing hint could
//! not touch. OPT-IN behind the `native-ttl` cargo feature (OFF by default): the incumbent oxttl
//! path stays shipped until this drop-in is A/B-validated byte-identical.
//!
//! # The load-bearing correctness claim
//!
//! For any Turtle document, this parser MUST produce a graph EQUAL to the oxttl path's — same
//! ground terms (IRIs after RFC-3987 base resolution, literals with their datatype/lang, labelled
//! blank nodes), the same triple set, and the SAME accept/reject decision on malformed input. The
//! only permitted difference is the *labels* of ANONYMOUS blank nodes (`[]`, collections `()`, and
//! `[ … ]` property lists): oxttl mints those as random 128-bit ids, so two oxttl parses of the
//! same document already differ there — every differential/conformance comparison is therefore
//! blank-node **isomorphic**. [GPT-5.6] This parser mints labels with a process-global monotonic
//! seed so independently minted anonymous nodes remain distinct. Turtle does not reserve that
//! internal label spelling, however, so a deliberately matching user-written `_:label` could
//! collide in principle.
//!
//! # How byte-identity is achieved, not re-derived
//!
//! - **IRI resolution + validation** is delegated to [`oxiri`] — the SAME RFC-3987 automaton oxttl
//!   resolves through — so a resolved absolute IRI is byte-exact by construction, not a hand-rolled
//!   re-implementation of relative-reference resolution (the highest-risk piece).
//! - **Literal lexical forms are preserved verbatim** (oxttl keeps `1.0e0`, `-0`, `.5`, `007`
//!   unchanged and only assigns the datatype from the token shape — confirmed against oxttl); string
//!   escapes are decoded exactly as N-Triples/Turtle ECHAR + UCHAR.
//! - **Language tags are ASCII-lowercased** (matching [`crate::nt`] and oxttl/oxrdf).
//! - **`a`** in predicate position expands to `rdf:type`; numeric/boolean shortcuts type to
//!   xsd:integer / xsd:decimal / xsd:double / xsd:boolean by token shape.
//!
//! Anything the parser is not certain it can reproduce byte-identically is a parse `Err` (never a
//! silent accept of invalid Turtle, never a silent divergence) — the same fail-closed posture the
//! `native_ttl_matches_oxttl` differential and the W3C TurtleTests ratchet pin.

#![cfg(feature = "native-ttl")]

use crate::dict::{Dict, Id};
use oxiri::Iri;
use std::borrow::Cow;
use std::collections::HashMap;

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const RDF_FIRST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#first";
const RDF_REST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#rest";
const RDF_NIL: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#nil";
const RDF_REIFIES: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies";
const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";
const XSD_DECIMAL: &str = "http://www.w3.org/2001/XMLSchema#decimal";
const XSD_DOUBLE: &str = "http://www.w3.org/2001/XMLSchema#double";
const XSD_BOOLEAN: &str = "http://www.w3.org/2001/XMLSchema#boolean";
const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";
const RDF_LANG_STRING: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString";
const RDF_DIR_LANG_STRING: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#dirLangString";

/// [OPUS-4.8] (sq-jocpn, ASVS V5.5.2) Maximum nesting depth of `[ … ]` property lists / `( … )`
/// collections before the parser returns a clean error instead of recursing until the native stack
/// overflows on adversarial input like `[[[[ … ]]]]` nested tens of thousands deep. Far deeper than
/// any real Turtle nests; kept in step with `nt::MAX_TRIPLE_TERM_DEPTH` and spargebra's recursion
/// bound. Applies to the ONE construct that recurses (nested anonymous blank nodes / collections).
const MAX_NEST_DEPTH: usize = 128;

/// Parse a whole Turtle document `bytes` into `dict`, resolving relative IRIs against `base`
/// (`None` = no base; a relative IRI is then an error, matching oxttl). Returns the id-triples or a
/// parse error carrying a byte offset. This is the entry point the `native-ttl` build routes both
/// the serial per-chunk worker AND `parse_to_triples_with_base` through.
pub fn parse(bytes: &[u8], base: Option<&str>, dict: &mut Dict) -> Result<Vec<[Id; 3]>, String> {
    let base = match base {
        Some(b) => Some(Iri::parse(b.to_string()).map_err(|e| format!("Turtle: invalid base IRI: {}", e))?),
        None => None,
    };
    let mut p = Parser {
        b: bytes,
        i: 0,
        base,
        prefixes: HashMap::new(),
        dict,
        out: Vec::with_capacity(bytes.len() / 40 + 1),
        bnode_counter: 0,
    };
    p.document()?;
    Ok(p.out)
}

struct Parser<'a, 'd> {
    b: &'a [u8],
    i: usize,
    base: Option<Iri<String>>,
    prefixes: HashMap<String, String>,
    dict: &'d mut Dict,
    out: Vec<[Id; 3]>,
    bnode_counter: u64,
}

impl Parser<'_, '_> {
    // ---- lexer primitives -------------------------------------------------

    #[inline]
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    /// Skip whitespace and `#`-comments (Turtle comments run to end-of-line).
    fn skip_ws(&mut self) {
        let n = self.b.len();
        loop {
            while self.i < n && matches!(self.b[self.i], b' ' | b'\t' | b'\r' | b'\n') {
                self.i += 1;
            }
            if self.i < n && self.b[self.i] == b'#' {
                while self.i < n && self.b[self.i] != b'\n' {
                    self.i += 1;
                }
            } else {
                return;
            }
        }
    }

    fn err(&self, msg: &str) -> String {
        format!("Turtle: {} at byte {}", msg, self.i)
    }

    // ---- document / directives -------------------------------------------

    fn document(&mut self) -> Result<(), String> {
        loop {
            self.skip_ws();
            match self.peek() {
                None => return Ok(()),
                Some(b'@') => self.at_directive()?,
                Some(c) if (c == b'P' || c == b'p') && self.looks_like_sparql_kw(b"prefix") => {
                    self.i += 6; // "PREFIX"
                    self.sparql_prefix()?;
                }
                Some(c) if (c == b'B' || c == b'b') && self.looks_like_sparql_kw(b"base") => {
                    self.i += 4; // "BASE"
                    self.sparql_base()?;
                }
                _ => self.statement()?,
            }
        }
    }

    /// Is the token at the cursor the SPARQL keyword `kw` (case-insensitive) followed by whitespace
    /// or a `<`/`#`? Used to disambiguate `PREFIX`/`BASE` from a prefixed name like `prefix:x`.
    fn looks_like_sparql_kw(&self, kw: &[u8]) -> bool {
        let end = self.i + kw.len();
        end <= self.b.len()
            && self.b[self.i..end].eq_ignore_ascii_case(kw)
            && matches!(self.b.get(end), Some(b' ' | b'\t' | b'\r' | b'\n' | b'<' | b'#'))
    }

    /// Exact-case (NOT case-insensitive) keyword match at the cursor, not followed by a name char.
    /// The Turtle `@`-directives are `@prefix`/`@base` in lowercase ONLY — `@BASE`/`@Prefix` are a
    /// syntax error (unlike the SPARQL-style `BASE`/`PREFIX`, which ARE case-insensitive). Pinned by
    /// the W3C `turtle-syntax-bad-base-02` case.
    fn eat_keyword_exact(&mut self, kw: &[u8]) -> bool {
        let end = self.i + kw.len();
        if end <= self.b.len()
            && &self.b[self.i..end] == kw
            && !matches!(self.b.get(end), Some(c) if is_pn_chars(*c) || *c == b':')
        {
            self.i = end;
            true
        } else {
            false
        }
    }

    /// `@prefix` / `@base` (each `.`-terminated). Keyword case is EXACT (lowercase).
    fn at_directive(&mut self) -> Result<(), String> {
        self.i += 1; // '@'
        if self.eat_keyword_exact(b"prefix") {
            self.skip_ws();
            let ns = self.pname_ns()?;
            self.skip_ws();
            let iri = self.iriref_resolved()?;
            self.prefixes.insert(ns, iri);
            self.skip_ws();
            self.expect_dot()
        } else if self.eat_keyword_exact(b"base") {
            self.skip_ws();
            let iri = self.iriref_raw()?;
            self.set_base(&iri)?;
            self.skip_ws();
            self.expect_dot()
        } else {
            Err(self.err("unknown @-directive (expected @prefix or @base)"))
        }
    }

    fn sparql_prefix(&mut self) -> Result<(), String> {
        self.skip_ws();
        let ns = self.pname_ns()?;
        self.skip_ws();
        let iri = self.iriref_resolved()?;
        self.prefixes.insert(ns, iri);
        Ok(())
    }

    fn sparql_base(&mut self) -> Result<(), String> {
        self.skip_ws();
        let iri = self.iriref_raw()?;
        self.set_base(&iri)
    }

    fn set_base(&mut self, raw: &str) -> Result<(), String> {
        // A relative BASE resolves against the current base (Turtle allows this). With no current
        // base, BASE must be absolute.
        let resolved = match &self.base {
            Some(b) => b.resolve(raw).map_err(|e| self.errmsg(&format!("invalid base IRI: {}", e)))?,
            None => Iri::parse(raw.to_string())
                .map_err(|e| self.errmsg(&format!("invalid base IRI: {}", e)))?,
        };
        self.base = Some(resolved);
        Ok(())
    }

    fn errmsg(&self, msg: &str) -> String {
        format!("Turtle: {} at byte {}", msg, self.i)
    }

    fn expect_dot(&mut self) -> Result<(), String> {
        if self.peek() == Some(b'.') {
            self.i += 1;
            Ok(())
        } else {
            Err(self.err("expected '.'"))
        }
    }

    // ---- statement / triples ---------------------------------------------

    fn statement(&mut self) -> Result<(), String> {
        // `triples ::= subject predicateObjectList | blankNodePropertyList predicateObjectList?`.
        // Only a NON-EMPTY `blankNodePropertyList` subject (`[ … ]`, which already emitted its own
        // triples) may stand alone before the `.`; an ordinary subject — IRI / prefixed name /
        // collection / blank-node label / reifier / a bare `[]` anon — MUST be followed by a
        // predicateObjectList. A bare `subject .` is a syntax error (W3C
        // `turtle-syntax-bad-n3-extras-03`/`-06` and `[] .`, matching oxttl).
        let (subject, standalone_ok) = self.subject_with_standalone(0)?;
        self.skip_ws();
        if standalone_ok && self.peek() == Some(b'.') {
            self.i += 1;
            return Ok(());
        }
        self.predicate_object_list(subject, 0)?;
        self.skip_ws();
        self.expect_dot()
    }

    /// `predicateObjectList ::= verb objectList ( ';' ( verb objectList )? )*`
    ///
    /// Turtle permits empty `;` separators (`a ;; b`) and a trailing `;` before the closing `.`/`]`.
    fn predicate_object_list(&mut self, subject: Id, depth: usize) -> Result<(), String> {
        loop {
            self.skip_ws();
            let verb = self.verb()?;
            self.object_list(subject, verb, depth)?;
            self.skip_ws();
            // Consume a run of `;` separators; a `;` immediately before `.`/`]`/EOF is a trailing
            // separator (no further verb follows).
            if self.peek() != Some(b';') {
                return Ok(());
            }
            while self.peek() == Some(b';') {
                self.i += 1;
                self.skip_ws();
            }
            match self.peek() {
                Some(b'.') | Some(b']') | None => return Ok(()),
                _ => continue,
            }
        }
    }

    /// `objectList ::= object annotation? ( ',' object annotation? )*` (RDF 1.2: an object may carry
    /// a trailing reifier `~ id?` and/or an annotation block `{| … |}`).
    fn object_list(&mut self, subject: Id, verb: Id, depth: usize) -> Result<(), String> {
        loop {
            self.skip_ws();
            let object = self.object(depth)?;
            self.out.push([subject, verb, object]);
            // RDF 1.2 annotation on this (subject, verb, object) statement.
            self.annotation(subject, verb, object, depth)?;
            self.skip_ws();
            if self.peek() == Some(b',') {
                self.i += 1;
                continue;
            }
            return Ok(());
        }
    }

    /// [OPUS-4.8] (sq-jocpn) Parse zero or more RDF 1.2 annotations attached to the just-emitted
    /// statement `(s, p, o)`: a reifier `~` (optionally `~ reifierId`) and/or an annotation block
    /// `{| predicateObjectList |}`. Each mints (or reuses) a reifier `R`, emits
    /// `R rdf:reifies <<( s p o )>>`, and — for a block — attaches the predicate-object list to `R`.
    /// This desugaring is byte-identical to oxttl's (probed): the `~`-only form and the block form
    /// both add exactly the `rdf:reifies` triple + any block triples, with a FRESH reifier blank when
    /// no id is written. Multiple annotations may chain (`~ r1 {| … |}`), sharing one reifier.
    fn annotation(&mut self, s: Id, p: Id, o: Id, depth: usize) -> Result<(), String> {
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'~') => {
                    self.i += 1;
                    self.skip_ws();
                    // Optional explicit reifier id (an iri / blank node) — else a fresh blank.
                    let reifier = self.reifier_id_or_fresh()?;
                    self.emit_reifies(reifier, s, p, o);
                    // A block may immediately follow the `~ id`.
                    if self.at_annotation_block() {
                        self.annotation_block(reifier, depth)?;
                    }
                }
                Some(b'{') if self.b.get(self.i + 1) == Some(&b'|') => {
                    let reifier = self.fresh_blank();
                    self.emit_reifies(reifier, s, p, o);
                    self.annotation_block(reifier, depth)?;
                }
                _ => return Ok(()),
            }
        }
    }

    fn at_annotation_block(&mut self) -> bool {
        let save = self.i;
        self.skip_ws();
        let yes = self.peek() == Some(b'{') && self.b.get(self.i + 1) == Some(&b'|');
        self.i = save;
        yes
    }

    /// After a `~`, read an optional explicit reifier id (IRI / prefixed name / blank node); if the
    /// next token is not one, mint a fresh reifier blank node.
    fn reifier_id_or_fresh(&mut self) -> Result<Id, String> {
        match self.peek() {
            Some(b'<') if self.b.get(self.i + 1) != Some(&b'<') => self.iri_term(),
            Some(b'_') if self.b.get(self.i + 1) == Some(&b':') => self.blank_node_label(),
            Some(b'[') if self.b.get(self.i + 1) == Some(&b']') => {
                self.i += 2;
                Ok(self.fresh_blank())
            }
            Some(c) if is_pn_chars_base(c) || c == b':' => self.prefixed_name(),
            _ => Ok(self.fresh_blank()),
        }
    }

    /// `{| predicateObjectList |}` attached to reifier `R`.
    fn annotation_block(&mut self, reifier: Id, depth: usize) -> Result<(), String> {
        if depth >= MAX_NEST_DEPTH {
            return Err(self.err("annotation nesting too deep"));
        }
        self.skip_ws();
        // consume `{|`
        if self.peek() != Some(b'{') || self.b.get(self.i + 1) != Some(&b'|') {
            return Err(self.err("expected '{|' opening an annotation block"));
        }
        self.i += 2;
        self.predicate_object_list(reifier, depth + 1)?;
        self.skip_ws();
        if self.peek() == Some(b'|') && self.b.get(self.i + 1) == Some(&b'}') {
            self.i += 2;
            Ok(())
        } else {
            Err(self.err("expected '|}' closing an annotation block"))
        }
    }

    /// Emit `reifier rdf:reifies <<( s p o )>>` — the triple term is interned structurally, exactly
    /// as [`crate::nt`]'s triple-term path and oxttl's reification desugaring do.
    fn emit_reifies(&mut self, reifier: Id, s: Id, p: Id, o: Id) {
        let reifies = self.dict.intern_iri(RDF_REIFIES);
        let tt = self.dict.intern_triple_ids([s, p, o]);
        self.out.push([reifier, reifies, tt]);
    }

    /// `verb ::= predicate | 'a'`
    fn verb(&mut self) -> Result<Id, String> {
        if self.peek() == Some(b'a') && !matches!(self.b.get(self.i + 1), Some(c) if is_pn_chars(*c) || *c == b':') {
            self.i += 1;
            return Ok(self.dict.intern_iri(RDF_TYPE));
        }
        // predicate ::= iri
        self.iri_term()
    }

    // ---- terms ------------------------------------------------------------

    /// Does the cursor sit on `<<(` (an RDF 1.2 triple term)?
    #[inline]
    fn at_triple_term(&self) -> bool {
        self.peek() == Some(b'<') && self.b.get(self.i + 1) == Some(&b'<') && self.b.get(self.i + 2) == Some(&b'(')
    }

    /// Does the cursor sit on `<<` NOT followed by `(` (an RDF 1.2 reifier term)?
    #[inline]
    fn at_reifier(&self) -> bool {
        self.peek() == Some(b'<') && self.b.get(self.i + 1) == Some(&b'<') && self.b.get(self.i + 2) != Some(&b'(')
    }

    /// `subject ::= iri | BlankNode | collection | reifiedTriple` (RDF 1.2 adds `<< … >>`).
    fn subject(&mut self, depth: usize) -> Result<Id, String> {
        Ok(self.subject_with_standalone(depth)?.0)
    }

    /// [`subject`], additionally reporting whether the subject is a NON-EMPTY `[ … ]`
    /// blankNodePropertyList — the only subject form the top-level statement may leave without a
    /// following predicateObjectList (a bare `[]` returns `false`, so `[] .` is rejected).
    fn subject_with_standalone(&mut self, depth: usize) -> Result<(Id, bool), String> {
        if self.at_reifier() {
            return Ok((self.reified_triple(depth)?, false));
        }
        match self.peek() {
            Some(b'<') => Ok((self.iri_term()?, false)),
            Some(b'_') => Ok((self.blank_node_label()?, false)),
            Some(b'[') => self.blank_node_property_list(depth),
            Some(b'(') => Ok((self.collection(depth)?, false)),
            Some(_) => Ok((self.prefixed_name()?, false)), // pname (incl. `:local` empty prefix)
            None => Err(self.err("unexpected end of input, expected a subject")),
        }
    }

    /// `object ::= iri | BlankNode | collection | blankNodePropertyList | literal | reifiedTriple |
    /// tripleTerm` (RDF 1.2 adds the last two).
    fn object(&mut self, depth: usize) -> Result<Id, String> {
        if self.at_triple_term() {
            return self.triple_term(depth);
        }
        if self.at_reifier() {
            return self.reified_triple(depth);
        }
        match self.peek() {
            Some(b'<') => self.iri_term(),
            Some(b'_') => self.blank_node_label(),
            Some(b'[') => Ok(self.blank_node_property_list(depth)?.0),
            Some(b'(') => self.collection(depth),
            Some(b'"') | Some(b'\'') => self.string_literal(),
            Some(c) if c == b'+' || c == b'-' || c.is_ascii_digit() || c == b'.' => self.numeric_literal(),
            Some(b't') | Some(b'f') | Some(b'T') | Some(b'F') if self.at_boolean() => self.boolean_literal(),
            Some(_) => self.prefixed_name(),
            None => Err(self.err("unexpected end of input, expected an object")),
        }
    }

    /// `reifiedTriple ::= '<<' subject predicate object reifier? '>>'` — RDF 1.2. Desugars to a
    /// reifier node `R` with `R rdf:reifies <<( s p o )>>` (byte-identical to oxttl), and returns
    /// `R` as the term standing in the subject/object slot. The optional inner `~ id` sets `R`.
    fn reified_triple(&mut self, depth: usize) -> Result<Id, String> {
        if depth >= MAX_NEST_DEPTH {
            return Err(self.err("reified-triple nesting too deep"));
        }
        self.i += 2; // '<<'
        self.skip_ws();
        let s = self.subject(depth + 1)?;
        self.skip_ws();
        let p = self.verb()?;
        self.skip_ws();
        let o = self.object(depth + 1)?;
        self.skip_ws();
        // Optional inner reifier `~ id?`.
        let reifier = if self.peek() == Some(b'~') {
            self.i += 1;
            self.skip_ws();
            self.reifier_id_or_fresh()?
        } else {
            self.fresh_blank()
        };
        self.skip_ws();
        if self.peek() == Some(b'>') && self.b.get(self.i + 1) == Some(&b'>') {
            self.i += 2;
        } else {
            return Err(self.err("expected '>>' closing a reified triple"));
        }
        self.emit_reifies(reifier, s, p, o);
        Ok(reifier)
    }

    /// `tripleTerm ::= '<<(' subject predicate object ')>>'` — RDF 1.2, object-only. Interned
    /// STRUCTURALLY via `intern_triple_ids`, exactly as [`crate::nt`]'s triple-term path.
    fn triple_term(&mut self, depth: usize) -> Result<Id, String> {
        if depth >= MAX_NEST_DEPTH {
            return Err(self.err("triple-term nesting too deep"));
        }
        self.i += 3; // '<<('
        self.skip_ws();
        let s = self.subject(depth + 1)?;
        self.skip_ws();
        let p = self.verb()?;
        self.skip_ws();
        let o = self.object(depth + 1)?;
        self.skip_ws();
        if self.peek() == Some(b')')
            && self.b.get(self.i + 1) == Some(&b'>')
            && self.b.get(self.i + 2) == Some(&b'>')
        {
            self.i += 3;
            Ok(self.dict.intern_triple_ids([s, p, o]))
        } else {
            Err(self.err("expected ')>>' closing a triple term"))
        }
    }

    /// `iri ::= IRIREF | PrefixedName` — used where only an IRI is valid (predicate / non-`a` verb).
    fn iri_term(&mut self) -> Result<Id, String> {
        match self.peek() {
            Some(b'<') => {
                let iri = self.iriref_resolved()?;
                Ok(self.dict.intern_iri(&iri))
            }
            Some(_) => self.prefixed_name(),
            None => Err(self.err("expected an IRI")),
        }
    }

    /// Read an `IRIREF` `<…>`, resolve it against the base, and return the absolute IRI string.
    fn iriref_resolved(&mut self) -> Result<String, String> {
        let raw = self.iriref_raw()?;
        self.resolve(&raw)
    }

    fn resolve(&self, raw: &str) -> Result<String, String> {
        match &self.base {
            Some(base) => base
                .resolve(raw)
                .map(|r| r.into_inner())
                .map_err(|e| self.errmsg(&format!("cannot resolve IRI <{}>: {}", raw, e))),
            None => {
                // No base: the IRI must already be absolute (validate through oxiri, exactly what
                // oxttl requires — a relative reference with no base is a parse error).
                Iri::parse(raw.to_string())
                    .map(|i| i.into_inner())
                    .map_err(|e| self.errmsg(&format!("relative IRI <{}> with no base: {}", raw, e)))
            }
        }
    }

    /// Read the raw (escape-decoded, unresolved) content of an `IRIREF` `<…>`, advancing past `>`.
    /// Rejects the characters the Turtle IRIREF production forbids unescaped (control chars,
    /// spaces, `<>"{}|^\` and backtick), matching oxttl.
    fn iriref_raw(&mut self) -> Result<String, String> {
        if self.peek() != Some(b'<') {
            return Err(self.err("expected '<' to start an IRIREF"));
        }
        self.i += 1;
        let start = self.i;
        let n = self.b.len();
        let mut out: Option<String> = None; // lazily built only if an escape is present
        while self.i < n {
            let c = self.b[self.i];
            match c {
                b'>' => {
                    let raw = &self.b[start..self.i];
                    self.i += 1;
                    return match out {
                        Some(s) => Ok(s),
                        None => std::str::from_utf8(raw)
                            .map(|s| s.to_owned())
                            .map_err(|_| self.err("invalid UTF-8 in IRIREF")),
                    };
                }
                b'\\' => {
                    // UCHAR: `\uXXXX` / `\UXXXXXXXX`. No other escape is legal in an IRIREF.
                    let buf = out.get_or_insert_with(|| {
                        String::from_utf8_lossy(&self.b[start..self.i]).into_owned()
                    });
                    let ch = self.read_uchar()?;
                    // The scanned codepoint is still subject to the IRIREF character restrictions.
                    if is_iri_forbidden(ch) {
                        return Err(self.err("forbidden character in IRIREF"));
                    }
                    buf.push(ch);
                }
                // Forbidden unescaped bytes per the IRIREF production.
                0x00..=0x20 | b'<' | b'"' | b'{' | b'}' | b'|' | b'^' | b'`' => {
                    return Err(self.err("forbidden character in IRIREF"));
                }
                _ => {
                    // Ordinary IRI content byte. On the no-escape fast path (`out` is None) we just
                    // advance and return the raw slice at the closing `>`; once an escape has forced
                    // the owned buffer, copy the whole UTF-8 codepoint (ASCII or multi-byte) into it.
                    if c < 0x80 {
                        if let Some(buf) = out.as_mut() {
                            buf.push(c as char);
                        }
                        self.i += 1;
                    } else {
                        let len = utf8_len(c);
                        let end = (self.i + len).min(n);
                        let s = std::str::from_utf8(&self.b[self.i..end])
                            .map_err(|_| self.err("invalid UTF-8 in IRIREF"))?;
                        if let Some(buf) = out.as_mut() {
                            buf.push_str(s);
                        }
                        self.i = end;
                    }
                }
            }
        }
        Err(self.err("unterminated IRIREF"))
    }

    /// Read `PNAME_NS` — a prefix label up to and including the `:` — returning the label WITHOUT
    /// the colon (`""` for the empty/default prefix `:`).
    ///
    /// Enforces the grammar `PN_PREFIX ::= PN_CHARS_BASE ((PN_CHARS | '.')* PN_CHARS)?`: the first
    /// char (if any) must be a PN_CHARS_BASE (not `_`/digit/`-`/`.`), interior `.` is allowed, but
    /// the label MUST NOT end in `.` — so `invalid.:` is rejected (W3C
    /// `turtle-syntax-bad-missing-ns-dot-end`), matching oxttl.
    fn pname_ns(&mut self) -> Result<String, String> {
        let start = self.i;
        // Empty prefix (`:`) is legal (the default namespace).
        if self.peek() != Some(b':') {
            // First char: PN_CHARS_BASE only.
            match self.peek() {
                Some(c) if is_pn_chars_base(c) => self.i += 1,
                _ => return Err(self.err("invalid prefix name (must start with a name char)")),
            }
            // Interior: PN_CHARS | '.'; track the last non-'.' so a trailing '.' is rejected.
            while self.i < self.b.len() && self.b[self.i] != b':' {
                let c = self.b[self.i];
                if is_pn_chars(c) || c >= 0x80 || c == b'.' {
                    self.i += 1;
                } else {
                    return Err(self.err("expected ':' terminating a prefix name"));
                }
            }
            // The char immediately before ':' must not be '.'.
            if self.i > start && self.b[self.i - 1] == b'.' {
                return Err(self.err("prefix name must not end in '.'"));
            }
        }
        if self.peek() != Some(b':') {
            return Err(self.err("expected ':' terminating a prefix name"));
        }
        let ns = std::str::from_utf8(&self.b[start..self.i])
            .map_err(|_| self.err("invalid UTF-8 in prefix name"))?
            .to_owned();
        self.i += 1; // ':'
        Ok(ns)
    }

    /// `PrefixedName ::= PNAME_LN | PNAME_NS` in term position: expand against the prefix table.
    fn prefixed_name(&mut self) -> Result<Id, String> {
        let ns = self.pname_ns()?;
        let local = self.pn_local()?;
        let namespace = self
            .prefixes
            .get(&ns)
            .ok_or_else(|| self.errmsg(&format!("the prefix {}: has not been declared", ns)))?;
        // The expanded IRI is `namespace + local` — Turtle does NOT re-resolve prefixed names against
        // the base (the namespace was already resolved when the @prefix was declared), matching oxttl.
        let full = format!("{}{}", namespace, local);
        Ok(self.dict.intern_iri(&full))
    }

    /// Read a `PN_LOCAL`, decoding PN_LOCAL_ESC (`\` + reserved char → the char) and keeping
    /// percent-encodings verbatim. Returns the decoded local part (may be empty).
    fn pn_local(&mut self) -> Result<String, String> {
        let n = self.b.len();
        let mut out = String::new();
        // PN_LOCAL first char: PN_CHARS_U | ':' | [0-9] | PLX
        // subsequent: (PN_CHARS | '.' | ':' | PLX)* with the last not a '.'
        let mut last_was_dot = false;
        let mut first = true;
        while self.i < n {
            let c = self.b[self.i];
            match c {
                b'\\' => {
                    // PN_LOCAL_ESC: backslash then one reserved char, kept literally.
                    let esc = self.b.get(self.i + 1).copied();
                    match esc {
                        Some(e) if is_pn_local_esc(e) => {
                            out.push(e as char);
                            self.i += 2;
                            last_was_dot = false;
                            first = false;
                        }
                        _ => return Err(self.err("invalid PN_LOCAL_ESC escape")),
                    }
                }
                b'%' => {
                    // PERCENT: `%` HEX HEX, kept verbatim.
                    let h1 = self.b.get(self.i + 1).copied();
                    let h2 = self.b.get(self.i + 2).copied();
                    match (h1, h2) {
                        (Some(a), Some(bb)) if a.is_ascii_hexdigit() && bb.is_ascii_hexdigit() => {
                            out.push('%');
                            out.push(a as char);
                            out.push(bb as char);
                            self.i += 3;
                            last_was_dot = false;
                            first = false;
                        }
                        _ => return Err(self.err("invalid percent-encoding in local name")),
                    }
                }
                b':' => {
                    out.push(':');
                    self.i += 1;
                    last_was_dot = false;
                    first = false;
                }
                b'.' => {
                    // A `.` is legal INTERIOR to a PN_LOCAL but not as the final char. Peek ahead:
                    // consume it provisionally; the trailing-dot trim below removes it if it ends up
                    // last. But a `.` followed by whitespace/EOF is the statement terminator.
                    let next = self.b.get(self.i + 1).copied();
                    let next_is_local = matches!(next, Some(x) if is_pn_chars(x) || x == b'.' || x == b':' || x == b'%' || x == b'\\');
                    if next_is_local {
                        out.push('.');
                        self.i += 1;
                        last_was_dot = true;
                    } else {
                        break; // `.` terminator (or end of local)
                    }
                }
                _ if first && (is_pn_chars_u(c) || c.is_ascii_digit() || c >= 0x80) => {
                    self.push_utf8(&mut out, c);
                    last_was_dot = false;
                    first = false;
                }
                _ if !first && (is_pn_chars(c) || c >= 0x80) => {
                    self.push_utf8(&mut out, c);
                    last_was_dot = false;
                }
                _ => break,
            }
        }
        // A PN_LOCAL must not end in '.'; if it does, that dot is the terminator, back it out.
        if last_was_dot {
            out.pop();
            self.i -= 1;
        }
        Ok(out)
    }

    /// Push byte `c` (ASCII or the leading byte of a multi-byte UTF-8 sequence) into `out`,
    /// advancing past the whole codepoint. Non-ASCII name chars are copied through verbatim.
    fn push_utf8(&mut self, out: &mut String, c: u8) {
        if c < 0x80 {
            out.push(c as char);
            self.i += 1;
        } else {
            let len = utf8_len(c);
            let end = (self.i + len).min(self.b.len());
            if let Ok(s) = std::str::from_utf8(&self.b[self.i..end]) {
                out.push_str(s);
            }
            self.i = end;
        }
    }

    // ---- blank nodes / collections ---------------------------------------

    /// `_:label` — a labelled blank node (label kept verbatim, document-scoped by the dict).
    fn blank_node_label(&mut self) -> Result<Id, String> {
        if self.b.get(self.i + 1) != Some(&b':') {
            return Err(self.err("expected ':' after '_' in a blank node label"));
        }
        self.i += 2;
        let start = self.i;
        let n = self.b.len();
        // BLANK_NODE_LABEL ::= '_:' (PN_CHARS_U | [0-9]) ((PN_CHARS | '.')* PN_CHARS)?
        // First char: PN_CHARS_U | digit.
        match self.peek() {
            Some(c) if is_pn_chars_u(c) || c.is_ascii_digit() || c >= 0x80 => self.i += 1,
            _ => return Err(self.err("invalid blank node label")),
        }
        while self.i < n {
            let c = self.b[self.i];
            if is_pn_chars(c) || c >= 0x80 {
                self.i += 1;
            } else if c == b'.' {
                // interior '.' allowed, trailing '.' is not part of the label
                let next = self.b.get(self.i + 1).copied();
                if matches!(next, Some(x) if is_pn_chars(x) || x == b'.' || x >= 0x80) {
                    self.i += 1;
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        let label = std::str::from_utf8(&self.b[start..self.i])
            .map_err(|_| self.err("invalid UTF-8 in blank node label"))?;
        Ok(self.dict.intern_blank(label))
    }

    /// [GPT-5.6] Mint a fresh anonymous blank node id. The label embeds a PROCESS-GLOBAL monotonic
    /// seed (not just a per-parse counter) so that two chunks parsed independently by the parallel
    /// loader — each a fresh [`Parser`] — never mint the SAME anon label and get wrongly unified by
    /// the label-based dict merge. Its spelling uses legal Turtle `PN_CHARS`, so it is not
    /// structurally disjoint from a deliberately matching user-written `_:label`. This mirrors
    /// oxttl's random-128-bit anonymous ids for the operational requirement: distinct anon nodes
    /// stay distinct across chunk boundaries, and no anon node crosses a chunk boundary anyway (a
    /// `[]`/`()` nest is confined to one statement). Two ground-equal documents parsed twice differ
    /// only in these labels, exactly as two oxttl parses do — every comparison is
    /// blank-node-isomorphic.
    fn fresh_blank(&mut self) -> Id {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEED: AtomicU64 = AtomicU64::new(0);
        let g = SEED.fetch_add(1, Ordering::Relaxed);
        self.bnode_counter += 1;
        let label = format!("__ttl_anon_{}_{}", g, self.bnode_counter);
        self.dict.intern_blank(&label)
    }

    /// `blankNodePropertyList ::= '[' predicateObjectList ']'` and the bare `[]` anon node. Returns
    /// `(node, non_empty)` — `non_empty` is `true` only when the brackets held a predicateObjectList
    /// (a real blankNodePropertyList, which may stand alone as a top-level statement); a bare `[]` is
    /// an anonymous node and returns `false`.
    fn blank_node_property_list(&mut self, depth: usize) -> Result<(Id, bool), String> {
        if depth >= MAX_NEST_DEPTH {
            return Err(self.err("blank node / collection nesting too deep"));
        }
        // consume '['
        self.i += 1;
        self.skip_ws();
        let node = self.fresh_blank();
        if self.peek() == Some(b']') {
            self.i += 1;
            return Ok((node, false)); // bare `[]` anonymous node
        }
        self.predicate_object_list(node, depth + 1)?;
        self.skip_ws();
        if self.peek() != Some(b']') {
            return Err(self.err("expected ']' closing a blank node property list"));
        }
        self.i += 1;
        Ok((node, true))
    }

    /// `collection ::= '(' object* ')'` — desugars to an rdf:first/rdf:rest/rdf:nil list, minting a
    /// fresh blank per element. Empty `()` is rdf:nil.
    fn collection(&mut self, depth: usize) -> Result<Id, String> {
        if depth >= MAX_NEST_DEPTH {
            return Err(self.err("blank node / collection nesting too deep"));
        }
        self.i += 1; // '('
        self.skip_ws();
        if self.peek() == Some(b')') {
            self.i += 1;
            return Ok(self.dict.intern_iri(RDF_NIL));
        }
        // Non-empty: build the chain head → … → nil.
        let head = self.fresh_blank();
        let first_pred = self.dict.intern_iri(RDF_FIRST);
        let rest_pred = self.dict.intern_iri(RDF_REST);
        let nil = self.dict.intern_iri(RDF_NIL);
        let mut current = head;
        loop {
            let obj = self.object(depth + 1)?;
            self.out.push([current, first_pred, obj]);
            self.skip_ws();
            if self.peek() == Some(b')') {
                self.i += 1;
                self.out.push([current, rest_pred, nil]);
                return Ok(head);
            }
            let next = self.fresh_blank();
            self.out.push([current, rest_pred, next]);
            current = next;
        }
    }

    // ---- literals ---------------------------------------------------------

    fn at_boolean(&self) -> bool {
        let rest = &self.b[self.i..];
        let matches_kw = |kw: &[u8]| {
            rest.len() >= kw.len()
                && rest[..kw.len()].eq_ignore_ascii_case(kw)
                && !matches!(rest.get(kw.len()), Some(c) if is_pn_chars(*c) || *c == b':')
        };
        matches_kw(b"true") || matches_kw(b"false")
    }

    fn boolean_literal(&mut self) -> Result<Id, String> {
        // Turtle booleans are case-sensitive (`true`/`false`), but we mirror oxttl which stores the
        // written lexical form. Only lowercase `true`/`false` are valid; anything else falls through
        // to a prefixed name (already routed away by `at_boolean`, which is case-insensitive to
        // avoid misparsing — but the value must be exactly `true`/`false`).
        let rest = &self.b[self.i..];
        let (lex, len) = if rest.starts_with(b"true") {
            ("true", 4)
        } else if rest.starts_with(b"false") {
            ("false", 5)
        } else {
            return Err(self.err("invalid boolean literal (must be lowercase true/false)"));
        };
        self.i += len;
        Ok(self.dict.intern_lit(lex, XSD_BOOLEAN, None))
    }

    /// A numeric literal — INTEGER / DECIMAL / DOUBLE. The lexical form is preserved verbatim and
    /// the datatype is assigned from the token shape, exactly as oxttl does.
    fn numeric_literal(&mut self) -> Result<Id, String> {
        let start = self.i;
        let n = self.b.len();
        let mut has_dot = false;
        let mut has_exp = false;
        // optional sign
        if matches!(self.peek(), Some(b'+') | Some(b'-')) {
            self.i += 1;
        }
        let digits_before = self.consume_digits();
        // fraction
        if self.peek() == Some(b'.') {
            // A '.' is part of the number only if it is a decimal point (followed by a digit) OR the
            // number already has integer digits and an exponent may follow — but a trailing '.' with
            // no fraction digit and no exponent is the statement terminator, not part of the number.
            let after = self.b.get(self.i + 1).copied();
            let dot_is_numeric = matches!(after, Some(x) if x.is_ascii_digit())
                || matches!(after, Some(b'e') | Some(b'E'));
            if dot_is_numeric {
                has_dot = true;
                self.i += 1;
                self.consume_digits();
            }
        }
        // exponent
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            has_exp = true;
            self.i += 1;
            if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                self.i += 1;
            }
            let exp_digits = self.consume_digits();
            if exp_digits == 0 {
                return Err(self.err("invalid number: exponent has no digits"));
            }
        }
        let lex = std::str::from_utf8(&self.b[start..self.i])
            .map_err(|_| self.err("invalid UTF-8 in number"))?;
        // Reject a bare sign / bare '.' / empty.
        let sign_only = matches!(lex, "+" | "-" | "" | "." | "+." | "-.");
        if sign_only || (digits_before == 0 && !has_dot && !has_exp) {
            return Err(self.err("invalid numeric literal"));
        }
        // A DECIMAL/DOUBLE with a leading '.' but no digit before it is valid (`.5`); an integer
        // needs at least one digit (guaranteed by digits_before>0 in that branch).
        let dt = if has_exp {
            XSD_DOUBLE
        } else if has_dot {
            XSD_DECIMAL
        } else {
            XSD_INTEGER
        };
        // Guard: an INTEGER must have at least one digit.
        if dt == XSD_INTEGER && digits_before == 0 {
            return Err(self.err("invalid integer literal"));
        }
        // Guard: a DECIMAL like `.` alone is invalid; require a fraction digit if it started with '.'
        if dt == XSD_DECIMAL && digits_before == 0 && !lex.trim_start_matches(['+', '-']).starts_with('.') {
            return Err(self.err("invalid decimal literal"));
        }
        let _ = n;
        Ok(self.dict.intern_lit(lex, dt, None))
    }

    fn consume_digits(&mut self) -> usize {
        let start = self.i;
        while self.i < self.b.len() && self.b[self.i].is_ascii_digit() {
            self.i += 1;
        }
        self.i - start
    }

    /// A string literal (all four quote forms) with an optional `^^<datatype>` / `@lang`.
    fn string_literal(&mut self) -> Result<Id, String> {
        let value = self.read_string()?;
        match self.peek() {
            Some(b'^') if self.b.get(self.i + 1) == Some(&b'^') => {
                self.i += 2;
                self.skip_ws();
                let dt = self.iri_datatype()?;
                Ok(self.dict.intern_lit(&value, &dt, None))
            }
            Some(b'@') => {
                self.i += 1;
                let tag = self.langtag()?;
                let (dt, lang) = normalize_lang(&tag);
                Ok(self.dict.intern_lit(&value, dt, Some(&lang)))
            }
            _ => Ok(self.dict.intern_lit(&value, XSD_STRING, None)),
        }
    }

    /// A datatype IRI after `^^` — either an IRIREF or a prefixed name.
    fn iri_datatype(&mut self) -> Result<String, String> {
        match self.peek() {
            Some(b'<') => self.iriref_resolved(),
            Some(_) => {
                let ns = self.pname_ns()?;
                let local = self.pn_local()?;
                let namespace = self
                    .prefixes
                    .get(&ns)
                    .ok_or_else(|| self.errmsg(&format!("the prefix {}: has not been declared", ns)))?;
                Ok(format!("{}{}", namespace, local))
            }
            None => Err(self.err("expected a datatype IRI after '^^'")),
        }
    }

    /// Read a LANGTAG after `@`: `[a-zA-Z]+ ('-' [a-zA-Z0-9]+)*`, plus the RDF 1.2 `--dir` suffix.
    fn langtag(&mut self) -> Result<String, String> {
        let start = self.i;
        let n = self.b.len();
        // primary subtag: one or more ASCII letters
        while self.i < n && self.b[self.i].is_ascii_alphabetic() {
            self.i += 1;
        }
        if self.i == start {
            return Err(self.err("empty language tag"));
        }
        // ('-' alnum+)*  and the RDF 1.2 '--' direction
        while self.i < n && self.b[self.i] == b'-' {
            self.i += 1;
            let seg = self.i;
            while self.i < n && (self.b[self.i].is_ascii_alphanumeric() || self.b[self.i] == b'-') {
                self.i += 1;
            }
            if self.i == seg && self.b.get(self.i).copied().is_none_or(|c| c != b'-') {
                // A dangling '-' with no following alnum: back it out (it isn't part of the tag).
                self.i -= 1;
                break;
            }
        }
        std::str::from_utf8(&self.b[start..self.i])
            .map(|s| s.to_owned())
            .map_err(|_| self.err("invalid UTF-8 in language tag"))
    }

    /// Read a string literal body (any of `"`, `'`, `"""`, `'''`), decoding escapes, returning the
    /// unescaped value. The cursor is left just past the closing quote(s).
    fn read_string(&mut self) -> Result<String, String> {
        let q = self.peek().unwrap(); // caller guarantees `"` or `'`
        let n = self.b.len();
        let triple = self.b.get(self.i + 1) == Some(&q) && self.b.get(self.i + 2) == Some(&q);
        let mut out = String::new();
        if triple {
            self.i += 3;
            loop {
                match self.peek() {
                    None => return Err(self.err("unterminated triple-quoted string")),
                    Some(b'\\') => out.push(self.read_echar_or_uchar()?),
                    Some(c) if c == q && self.b.get(self.i + 1) == Some(&q) && self.b.get(self.i + 2) == Some(&q) => {
                        self.i += 3;
                        return Ok(out);
                    }
                    Some(_) => {
                        let c = self.copy_char(&mut out)?;
                        let _ = c;
                    }
                }
            }
        } else {
            self.i += 1;
            while self.i < n {
                match self.b[self.i] {
                    b'\\' => out.push(self.read_echar_or_uchar()?),
                    c if c == q => {
                        self.i += 1;
                        return Ok(out);
                    }
                    b'\n' | b'\r' => return Err(self.err("unescaped newline in a single-quoted string")),
                    _ => {
                        self.copy_char(&mut out)?;
                    }
                }
            }
            Err(self.err("unterminated string literal"))
        }
    }

    /// Copy one UTF-8 codepoint at the cursor into `out`, advancing. Rejects invalid UTF-8.
    fn copy_char(&mut self, out: &mut String) -> Result<char, String> {
        let c = self.b[self.i];
        if c < 0x80 {
            out.push(c as char);
            self.i += 1;
            Ok(c as char)
        } else {
            let len = utf8_len(c);
            let end = self.i + len;
            if end > self.b.len() {
                return Err(self.err("invalid UTF-8"));
            }
            let s = std::str::from_utf8(&self.b[self.i..end]).map_err(|_| self.err("invalid UTF-8"))?;
            let ch = s.chars().next().unwrap();
            out.push_str(s);
            self.i = end;
            Ok(ch)
        }
    }

    /// Decode an ECHAR or UCHAR escape at a `\`, returning the decoded char and advancing.
    fn read_echar_or_uchar(&mut self) -> Result<char, String> {
        // cursor at '\'
        match self.b.get(self.i + 1).copied() {
            Some(b't') => { self.i += 2; Ok('\t') }
            Some(b'b') => { self.i += 2; Ok('\u{8}') }
            Some(b'n') => { self.i += 2; Ok('\n') }
            Some(b'r') => { self.i += 2; Ok('\r') }
            Some(b'f') => { self.i += 2; Ok('\u{C}') }
            Some(b'"') => { self.i += 2; Ok('"') }
            Some(b'\'') => { self.i += 2; Ok('\'') }
            Some(b'\\') => { self.i += 2; Ok('\\') }
            // `read_uchar` expects the cursor AT the `\`, so do not pre-advance here.
            Some(b'u') | Some(b'U') => self.read_uchar(),
            _ => Err(self.err("invalid string escape")),
        }
    }

    /// At a cursor pointing to `\`, read the `\uXXXX` or `\UXXXXXXXX` codepoint; cursor left past it.
    fn read_uchar(&mut self) -> Result<char, String> {
        // cursor at '\'
        let kind = self.b.get(self.i + 1).copied();
        let count = match kind {
            Some(b'u') => 4,
            Some(b'U') => 8,
            _ => return Err(self.err("invalid \\u escape")),
        };
        let hs = self.i + 2;
        let he = hs + count;
        if he > self.b.len() {
            return Err(self.err("truncated \\u escape"));
        }
        let hex = std::str::from_utf8(&self.b[hs..he]).map_err(|_| self.err("invalid \\u escape"))?;
        let cp = u32::from_str_radix(hex, 16).map_err(|_| self.err("invalid \\u escape digits"))?;
        let ch = char::from_u32(cp).ok_or_else(|| self.err("invalid code point in \\u escape"))?;
        self.i = he;
        Ok(ch)
    }
}

// ---- free helpers ---------------------------------------------------------

/// Normalise a raw language tag: ASCII-lowercase, and pick the RDF datatype (`rdf:langString`, or
/// `rdf:dirLangString` when a `--dir` suffix is present) — identical to [`crate::nt`]/oxttl.
fn normalize_lang(raw: &str) -> (&'static str, Cow<'_, str>) {
    let dt = if raw.contains("--") { RDF_DIR_LANG_STRING } else { RDF_LANG_STRING };
    let lang = if raw.bytes().any(|c| c.is_ascii_uppercase()) {
        Cow::Owned(raw.to_ascii_lowercase())
    } else {
        Cow::Borrowed(raw)
    };
    (dt, lang)
}

#[inline]
fn is_pn_chars_base(c: u8) -> bool {
    c.is_ascii_alphabetic() || c >= 0x80
}

#[inline]
fn is_pn_chars_u(c: u8) -> bool {
    is_pn_chars_base(c) || c == b'_'
}

#[inline]
fn is_pn_chars(c: u8) -> bool {
    is_pn_chars_u(c) || c == b'-' || c.is_ascii_digit() || c == 0xB7 || c >= 0x80
}

/// The reserved characters that PN_LOCAL_ESC may escape (`\_`, `\.`, `\#`, …).
#[inline]
fn is_pn_local_esc(c: u8) -> bool {
    matches!(
        c,
        b'_' | b'~' | b'.' | b'-' | b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b',' | b';'
            | b'=' | b'/' | b'?' | b'#' | b'@' | b'%'
    )
}

/// Characters forbidden anywhere in an IRIREF (even when produced by a UCHAR escape).
#[inline]
fn is_iri_forbidden(ch: char) -> bool {
    let c = ch as u32;
    c <= 0x20
        || matches!(ch, '<' | '>' | '"' | '{' | '}' | '|' | '^' | '`' | '\\')
}

/// Length in bytes of the UTF-8 sequence whose leading byte is `c`.
#[inline]
fn utf8_len(c: u8) -> usize {
    if c < 0x80 {
        1
    } else if c >> 5 == 0b110 {
        2
    } else if c >> 4 == 0b1110 {
        3
    } else {
        4
    }
}

#[cfg(test)]
mod tests;
