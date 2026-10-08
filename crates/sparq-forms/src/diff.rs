//! Pure form-value differencing and SPARQL 1.1 Update rendering.
//! [GPT-5.6] sq-wn788

use crate::{derive::render_path, FormDescription, FormField, TermRef};
use oxrdf::{BaseDirection, BlankNode, Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use serde::{Deserialize, Serialize};
use sparq_shacl::Path;

/// One value added to or removed from a bare forward-predicate field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldValueDiff {
    /// The field path, always a bare forward predicate such as `<http://example.org/name>`.
    pub path: String,
    /// The RDF term added or removed at that path.
    pub value: TermRef,
}

/// The editable term-level difference between two descriptions of one focus node.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FormDiff {
    /// Values present only in the edited description.
    pub added: Vec<FieldValueDiff>,
    /// Values present only in the original description.
    pub removed: Vec<FieldValueDiff>,
}

impl FormDiff {
    /// Computes the writable difference between two descriptions.
    ///
    /// A mismatched focus node produces an empty diff. Only fields editable in
    /// both descriptions are compared; a newly introduced editable field may
    /// contribute additions. Inverse, read-only, and complex-path fields are ignored.
    pub fn between(before: &FormDescription, after: &FormDescription) -> Self {
        if before.focus != after.focus {
            return Self::default();
        }

        let mut diff = Self::default();
        for after_field in fields(after).filter(|field| eligible(field)) {
            let Some(predicate) = bare_predicate(&after_field.path) else {
                continue;
            };
            let before_field = fields(before)
                .find(|field| eligible(field) && bare_predicate(&field.path) == Some(predicate));

            for value in &after_field.values {
                let was_present = before_field
                    .is_some_and(|field| field.values.iter().any(|old| old.term == value.term));
                if !was_present {
                    diff.added.push(FieldValueDiff {
                        path: after_field.path.clone(),
                        value: value.term.clone(),
                    });
                }
            }

            if let Some(before_field) = before_field {
                for value in &before_field.values {
                    if !after_field.values.iter().any(|new| new.term == value.term) {
                        diff.removed.push(FieldValueDiff {
                            path: after_field.path.clone(),
                            value: value.term.clone(),
                        });
                    }
                }
            }
        }
        diff
    }
}

/// Builds one SPARQL 1.1 `DELETE`/`INSERT` update for an edited form.
///
/// Returns an empty string when there is no writable change (including when
/// the descriptions name different focus nodes). Callers may treat that as a no-op.
///
/// The build is all-or-nothing: if any term or field path cannot be rendered
/// safely (an invalid IRI, blank-node label or language tag, triple-term text
/// that does not parse as one RDF 1.2 triple term, or an unknown `kind`), no
/// update is produced and the empty string is returned, so a deserialized
/// [`TermRef`] or field path can never splice update syntax into the request.
/// Every editable forward field's path must be a single valid `<IRI>` or a
/// recognised complex SHACL path as derive renders it; any other path fails
/// the whole update rather than being skipped.
pub fn to_sparql_update(before: &FormDescription, after: &FormDescription) -> String {
    render_update(before, after).unwrap_or_default()
}

fn render_update(before: &FormDescription, after: &FormDescription) -> Option<String> {
    // `FormDiff::between` skips fields it cannot write, so classify every
    // editable forward path first. It must be the exact rendering of a valid
    // SHACL path: a single IRI (writable) or a complex path (excluded). Any
    // other text is malformed and fails the whole update, never just its field.
    for field in fields(before).chain(fields(after)) {
        if field.editable && !field.inverse {
            parse_field_path(&field.path)?;
        }
    }
    let diff = FormDiff::between(before, after);
    if diff.added.is_empty() && diff.removed.is_empty() {
        return None;
    }

    let subject = term_to_ntriples(&after.focus)?;
    let triples = |changes: &[FieldValueDiff]| {
        changes
            .iter()
            .map(|change| {
                let predicate = iri(bare_predicate(&change.path)?)?;
                let object = term_to_ntriples(&change.value)?;
                Some(format!("  {subject} {predicate} {object} .\n"))
            })
            .collect::<Option<String>>()
    };

    Some(format!(
        "DELETE {{\n{}}}\nINSERT {{\n{}}}\nWHERE {{}}",
        triples(&diff.removed)?,
        triples(&diff.added)?
    ))
}

fn fields(form: &FormDescription) -> impl Iterator<Item = &FormField> {
    form.groups.iter().flat_map(|group| group.fields.iter())
}

fn eligible(field: &FormField) -> bool {
    field.editable && !field.inverse && bare_predicate(&field.path).is_some()
}

/// The IRI of a path that is a single `<...>` token, or `None` for a complex
/// path. Whether that IRI is valid is checked separately (see `render_update`).
fn bare_predicate(path: &str) -> Option<&str> {
    let iri = path.strip_prefix('<')?.strip_suffix('>')?;
    (!iri.is_empty() && !iri.contains(['<', '>'])).then_some(iri)
}

/// Renders one term for the update template, or `None` when a component has
/// no escape form and fails validation.
fn term_to_ntriples(term: &TermRef) -> Option<String> {
    Some(match term.kind.as_str() {
        "iri" => iri(&term.value)?,
        // Blank-node labels have no escape form: validate against BLANK_NODE_LABEL.
        "bnode" => blank_node(&term.value)?.to_string(),
        "literal" => {
            let literal = format!("\"{}\"", escape_literal(&term.value));
            if let Some(language) = &term.language {
                // Language tags have no escape form either: require a valid BCP 47 tag.
                Literal::new_language_tagged_literal(&term.value, language).ok()?;
                format!("{literal}@{language}")
            } else if let Some(datatype) = &term.datatype {
                format!("{literal}^^{}", iri(datatype)?)
            } else {
                literal
            }
        }
        // RDF 1.2 triple terms are carried as N-Triples text: re-parse it and
        // emit the canonical serialization of exactly one triple term.
        "triple" => Term::from(parse_triple_term(&term.value)?).to_string(),
        _ => return None,
    })
}

/// Renders an absolute IRI as `<...>`, or `None` when it is not a valid IRI.
///
/// IRIs are validated, never escaped: SPARQL decodes `\uXXXX` escapes before
/// parsing, so an escaped `>` would still end the IRIREF.
fn iri(value: &str) -> Option<String> {
    Some(NamedNode::new(value).ok()?.to_string())
}

/// Parses N-Triples text holding exactly one RDF 1.2 triple term.
///
/// Hand-written rather than `oxrdf::Term::from_str`, which rejects valid
/// blank-node labels with internal dots (`_:a..b`). Every component is still
/// validated by the matching `oxrdf` constructor, so anything that is not one
/// well-formed triple term (and nothing else) yields `None`.
fn parse_triple_term(text: &str) -> Option<Triple> {
    let mut parser = TermParser {
        rest: text,
        depth: 0,
    };
    let triple = parser.triple_term()?;
    skip_ws(parser.rest).is_empty().then_some(triple)
}

/// Maximum `<<(` nesting, matching `MAX_TRIPLE_TERM_DEPTH` in sparq-core's
/// N-Triples parser, so deep input fails closed instead of overflowing the stack.
const MAX_TRIPLE_TERM_DEPTH: usize = 128;

struct TermParser<'a> {
    rest: &'a str,
    depth: usize,
}

impl TermParser<'_> {
    fn eat(&mut self, token: &str) -> Option<()> {
        self.rest = skip_ws(self.rest).strip_prefix(token)?;
        Some(())
    }

    fn triple_term(&mut self) -> Option<Triple> {
        self.eat("<<(")?;
        self.depth += 1;
        if self.depth > MAX_TRIPLE_TERM_DEPTH {
            return None;
        }
        let subject: NamedOrBlankNode = match skip_ws(self.rest).chars().next()? {
            '<' => self.iri()?.into(),
            '_' => self.blank_node()?.into(),
            _ => return None,
        };
        let predicate = self.iri()?;
        let object = self.object()?;
        self.eat(")>>")?;
        self.depth -= 1;
        Some(Triple::new(subject, predicate, object))
    }

    fn object(&mut self) -> Option<Term> {
        let rest = skip_ws(self.rest);
        Some(if rest.starts_with("<<(") {
            self.triple_term()?.into()
        } else if rest.starts_with('<') {
            self.iri()?.into()
        } else if rest.starts_with('_') {
            self.blank_node()?.into()
        } else if rest.starts_with('"') {
            self.literal()?.into()
        } else {
            return None;
        })
    }

    fn iri(&mut self) -> Option<NamedNode> {
        self.eat("<")?;
        let end = self.rest.find('>')?;
        let raw = &self.rest[..end];
        self.rest = &self.rest[end + 1..];
        NamedNode::new(unescape(raw, false)?).ok()
    }

    fn blank_node(&mut self) -> Option<BlankNode> {
        self.eat("_:")?;
        // The label token ends at the first character BLANK_NODE_LABEL cannot
        // contain; `blank_node` then checks the full production.
        let end = self
            .rest
            .find(|c: char| !(is_pn_chars(c) || c == '.'))
            .unwrap_or(self.rest.len());
        let label = &self.rest[..end];
        self.rest = &self.rest[end..];
        blank_node(label)
    }

    fn literal(&mut self) -> Option<Literal> {
        self.eat("\"")?;
        let mut end = None;
        let mut escaped = false;
        for (i, c) in self.rest.char_indices() {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => {
                    end = Some(i);
                    break;
                }
                // STRING_LITERAL_QUOTE forbids raw line breaks.
                '\n' | '\r' => return None,
                _ => {}
            }
        }
        let end = end?;
        let value = unescape(&self.rest[..end], true)?;
        // N-Triples allows whitespace between the string and its suffix.
        self.rest = skip_ws(&self.rest[end + 1..]);
        if let Some(tagged) = self.rest.strip_prefix('@') {
            let end = tagged
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                .unwrap_or(tagged.len());
            let tag = &tagged[..end];
            self.rest = &tagged[end..];
            match tag.split_once("--") {
                Some((language, "ltr")) => Literal::new_directional_language_tagged_literal(
                    value,
                    language,
                    BaseDirection::Ltr,
                )
                .ok(),
                Some((language, "rtl")) => Literal::new_directional_language_tagged_literal(
                    value,
                    language,
                    BaseDirection::Rtl,
                )
                .ok(),
                Some(_) => None,
                None => Literal::new_language_tagged_literal(value, tag).ok(),
            }
        } else if let Some(datatype) = self.rest.strip_prefix("^^") {
            self.rest = skip_ws(datatype);
            if !self.rest.starts_with('<') {
                return None;
            }
            Some(Literal::new_typed_literal(value, self.iri()?))
        } else {
            Some(Literal::new_simple_literal(value))
        }
    }
}

/// Parses a field path as derive renders it (`Path::to_sparql_property_path`,
/// or `Path::to_turtle` when that has no SPARQL form), with every predicate a
/// valid IRI. Returns `None` unless the text is exactly such a rendering.
fn parse_field_path(text: &str) -> Option<Path> {
    for turtle in [false, true] {
        let mut parser = PathParser {
            rest: text,
            depth: 0,
        };
        let parsed = if turtle {
            parser.turtle()
        } else {
            parser.sparql()
        };
        if let Some(path) = parsed.filter(|_| parser.rest.is_empty()) {
            if render_path(&path) == text {
                return Some(path);
            }
        }
    }
    None
}

const SH: &str = "http://www.w3.org/ns/shacl#";

/// Nesting cap for path text, so deep input fails closed instead of
/// overflowing the stack (same bound as triple terms).
const MAX_PATH_DEPTH: usize = MAX_TRIPLE_TERM_DEPTH;

struct PathParser<'a> {
    rest: &'a str,
    depth: usize,
}

impl PathParser<'_> {
    fn eat(&mut self, token: &str) -> Option<()> {
        self.rest = self.rest.strip_prefix(token)?;
        Some(())
    }

    fn descend(&mut self) -> Option<()> {
        self.depth += 1;
        (self.depth <= MAX_PATH_DEPTH).then_some(())
    }

    fn predicate(&mut self) -> Option<Path> {
        self.eat("<")?;
        let end = self.rest.find('>')?;
        let iri = NamedNode::new(&self.rest[..end]).ok()?;
        self.rest = &self.rest[end + 1..];
        Some(Path::Predicate(iri.into_string()))
    }

    /// `<p>`, `^(P)`, `(P/P..)`, `(P|P..)`, `(P)*`, `(P)+`, `(P)?`.
    fn sparql(&mut self) -> Option<Path> {
        if self.rest.starts_with('<') {
            return self.predicate();
        }
        self.descend()?;
        let path = if self.eat("^(").is_some() {
            let inner = self.sparql()?;
            self.eat(")")?;
            Path::Inverse(Box::new(inner))
        } else {
            self.eat("(")?;
            let mut items = vec![self.sparql()?];
            let alternative = self.rest.starts_with('|');
            while self.eat(if alternative { "|" } else { "/" }).is_some() {
                items.push(self.sparql()?);
            }
            self.eat(")")?;
            let single = || Box::new(items[0].clone());
            if items.len() == 1 && self.eat("*").is_some() {
                Path::ZeroOrMore(single())
            } else if items.len() == 1 && self.eat("+").is_some() {
                Path::OneOrMore(single())
            } else if items.len() == 1 && self.eat("?").is_some() {
                Path::ZeroOrOne(single())
            } else if alternative {
                Path::Alternative(items)
            } else {
                Path::Sequence(items)
            }
        };
        self.depth -= 1;
        Some(path)
    }

    /// `<p>`, `( P .. )`, and `[ <sh:…Path> P ]` (alternative takes a list).
    fn turtle(&mut self) -> Option<Path> {
        if self.rest.starts_with('<') {
            return self.predicate();
        }
        self.descend()?;
        let path = if self.rest.starts_with('(') {
            Path::Sequence(self.turtle_list()?)
        } else {
            self.eat("[ <")?;
            self.eat(SH)?;
            let end = self.rest.find("> ")?;
            let kind = &self.rest[..end];
            self.rest = &self.rest[end + 2..];
            let path = match kind {
                "alternativePath" => Path::Alternative(self.turtle_list()?),
                "inversePath" => Path::Inverse(Box::new(self.turtle()?)),
                "zeroOrMorePath" => Path::ZeroOrMore(Box::new(self.turtle()?)),
                "oneOrMorePath" => Path::OneOrMore(Box::new(self.turtle()?)),
                "zeroOrOnePath" => Path::ZeroOrOne(Box::new(self.turtle()?)),
                _ => return None,
            };
            self.eat(" ]")?;
            path
        };
        self.depth -= 1;
        Some(path)
    }

    fn turtle_list(&mut self) -> Option<Vec<Path>> {
        self.eat("( ")?;
        let mut items = Vec::new();
        while self.eat(" )").is_none() {
            if !items.is_empty() {
                self.eat(" ")?;
            }
            items.push(self.turtle()?);
        }
        Some(items)
    }
}

/// Skips N-Triples whitespace (`WS ::= #x20 | #x9`), and nothing else.
fn skip_ws(text: &str) -> &str {
    text.trim_start_matches([' ', '\t'])
}

/// Builds a blank node whose label matches the N-Triples / SPARQL
/// `BLANK_NODE_LABEL` production. `oxrdf::BlankNode::new` is more lenient
/// (it accepts `a:b`, which SPARQL rejects), so the grammar is checked here
/// first, for direct and nested terms alike. Internal dots stay valid.
fn blank_node(label: &str) -> Option<BlankNode> {
    let mut chars = label.chars();
    let first = chars.next()?;
    let valid = (is_pn_chars_u(first) || first.is_ascii_digit())
        && chars.all(|c| is_pn_chars(c) || c == '.')
        && !label.ends_with('.');
    valid.then(|| BlankNode::new(label).ok())?
}

fn is_pn_chars_u(c: char) -> bool {
    c == '_'
        || c.is_ascii_alphabetic()
        || matches!(c,
            '\u{C0}'..='\u{D6}'
            | '\u{D8}'..='\u{F6}'
            | '\u{F8}'..='\u{2FF}'
            | '\u{370}'..='\u{37D}'
            | '\u{37F}'..='\u{1FFF}'
            | '\u{200C}'..='\u{200D}'
            | '\u{2070}'..='\u{218F}'
            | '\u{2C00}'..='\u{2FEF}'
            | '\u{3001}'..='\u{D7FF}'
            | '\u{F900}'..='\u{FDCF}'
            | '\u{FDF0}'..='\u{FFFD}'
            | '\u{10000}'..='\u{EFFFF}')
}

fn is_pn_chars(c: char) -> bool {
    is_pn_chars_u(c)
        || c == '-'
        || c.is_ascii_digit()
        || matches!(c, '\u{B7}' | '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}')
}

/// Decodes `\uXXXX` / `\UXXXXXXXX` (and, in strings, ECHAR) escapes.
fn unescape(raw: &str, string: bool) -> Option<String> {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let digits = match chars.next()? {
            'u' => 4,
            'U' => 8,
            e if string => {
                out.push(match e {
                    't' => '\t',
                    'b' => '\u{8}',
                    'n' => '\n',
                    'r' => '\r',
                    'f' => '\u{c}',
                    '"' | '\'' | '\\' => e,
                    _ => return None,
                });
                continue;
            }
            _ => return None,
        };
        let hex: String = chars.by_ref().take(digits).collect();
        if hex.len() != digits || !hex.chars().all(|h| h.is_ascii_hexdigit()) {
            return None;
        }
        out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
    }
    Some(out)
}

fn escape_literal(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '\\' => "\\\\".to_string(),
            '"' => "\\\"".to_string(),
            '\n' => "\\n".to_string(),
            '\r' => "\\r".to_string(),
            '\t' => "\\t".to_string(),
            '\u{8}' => "\\b".to_string(),
            '\u{c}' => "\\f".to_string(),
            c if c < '\u{20}' => unicode_escape(c),
            c => c.to_string(),
        })
        .collect()
}

fn unicode_escape(c: char) -> String {
    let n = c as u32;
    if n <= 0xffff {
        format!("\\u{n:04X}")
    } else {
        format!("\\U{n:08X}")
    }
}
