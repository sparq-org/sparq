//! Stable-parser recovery of SPARQL `VERSION` announcements.
//!
//! [OPUS-5.5] The published `spargebra` 0.4.6 parser accepts `VERSION`
//! declarations (with its `sparql-12` feature) but discards their labels, while
//! the engine needs every label, in source order, to select EBV rules. The
//! helpers here use only that stable upstream API, so the published crate and
//! this repository's vendored parser run the same code. The caller's configured
//! [`SparqlParser`] parses the unchanged source first and remains the only
//! syntax authority. Only an accepted request's leading prologue is then
//! scanned, and each `VERSION` label is decoded exactly as the parser decodes
//! a short string literal.
//!
//! # Mirrored grammar
//!
//! The scanner follows the parser's own prologue productions token for token:
//!
//! ```text
//! Prologue    ::= ( BaseDecl _ | PrefixDecl _ | VersionDecl _ )*
//! BaseDecl    ::= 'BASE' _ IRIREF
//! PrefixDecl  ::= 'PREFIX' _ PNAME_NS _ IRIREF
//! VersionDecl ::= 'VERSION' _ ( STRING_LITERAL1 | STRING_LITERAL2 )
//! _           ::= ( [ \t\n\r] | '#' [^\r\n]* )*
//! ```
//!
//! Keywords match ASCII case-insensitively and need no trailing delimiter,
//! exactly as in the parser. The parser has only one prologue, before the query
//! form or the first update operation. Because no query or update form begins
//! with `BASE`, `PREFIX` or `VERSION`, the prologue of an accepted request ends
//! at the first position where none of those keywords starts. The query body is
//! never inspected, so a `VERSION` token in a comment, IRI, prefixed name or
//! literal is never mistaken for a declaration.
//!
//! # Unicode escape preprocessing
//!
//! `spargebra`'s optional `standard-unicode-escaping` feature rewrites every
//! `\uXXXX` and `\UXXXXXXXX` sequence before its grammar runs. That changes
//! prologue boundaries: an escaped line feed ends a comment early, and an
//! escaped keyword letter can create a declaration. Cargo feature unification
//! can enable the feature from any crate in the final dependency graph, so this
//! crate's configuration cannot tell which mode the linked parser uses. When a
//! source contains such a sequence, the helpers ask the linked parser directly
//! with a probe that is valid only after preprocessing. In that mode the scanner
//! reads a faithful replica of the upstream preprocessing, quirks included.
//!
//! # Errors
//!
//! A rejected request yields the parser's own syntax error, displayed
//! unchanged. If an accepted declaration cannot be accounted for, the helpers
//! return a prologue error instead of guessing labels or defaulting to REC 2013
//! semantics. That defensive path indicates a linked parser whose prologue
//! grammar differs from the productions above.

use std::borrow::Cow;
use std::fmt;

use spargebra::{Query, SparqlParser, SparqlSyntaxError, Update};

/// Parses a SPARQL query and returns its `VERSION` labels in source order.
///
/// The configured `parser` (base IRI, prefixes, custom aggregates) parses the
/// unchanged `query` through the stable `parse_query` API. Labels are decoded
/// but not validated: supported-label and EBV compatibility checks belong to
/// [`crate::PreparedQuery::from_query_with_versions`].
///
/// # Examples
///
/// ```
/// use spargebra::SparqlParser;
///
/// let (query, versions) = sparq_engine::parse_versioned_query(
///     SparqlParser::new(),
///     "# VERSION '1.1'\nVERSION '1.2' ASK { <urn:s> <urn:p> \"VERSION '1.1'\" }",
/// )?;
/// assert_eq!(versions, ["1.2"]);
/// let prepared = sparq_engine::PreparedQuery::from_query_with_versions(query, versions)?;
/// assert_eq!(prepared.versions(), ["1.2"]);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// # Errors
///
/// Returns a syntax error, with the parser's diagnostic text unchanged, when
/// the parser rejects `query`. Returns a prologue error when an accepted
/// declaration cannot be accounted for.
pub fn parse_versioned_query(
    parser: SparqlParser,
    query: &str,
) -> Result<(Query, Vec<String>), VersionedParseError> {
    let parsed = parser.parse_query(query).map_err(VersionedParseError::syntax)?;
    Ok((parsed, declared_versions(query)?))
}

/// Parses a SPARQL update and returns its `VERSION` labels in source order.
///
/// Behaves like [`parse_versioned_query`] for the update grammar, including an
/// update whose prologue is followed by no operation. Labels are not validated:
/// [`crate::parse_update_rec2013`] applies the engine's UPDATE restriction.
///
/// # Examples
///
/// ```
/// use spargebra::SparqlParser;
///
/// let (update, versions) = sparq_engine::parse_versioned_update(
///     SparqlParser::new(),
///     "PREFIX ex: <http://ex/> VERSION \"1.1\" INSERT DATA { ex:s ex:p ex:o }",
/// )?;
/// assert_eq!(versions, ["1.1"]);
/// assert_eq!(update.operations.len(), 1);
/// # Ok::<(), sparq_engine::VersionedParseError>(())
/// ```
///
/// # Errors
///
/// Returns a syntax error, with the parser's diagnostic text unchanged, when
/// the parser rejects `update`. Returns a prologue error when an accepted
/// declaration cannot be accounted for.
pub fn parse_versioned_update(
    parser: SparqlParser,
    update: &str,
) -> Result<(Update, Vec<String>), VersionedParseError> {
    let parsed = parser.parse_update(update).map_err(VersionedParseError::syntax)?;
    Ok((parsed, declared_versions(update)?))
}

/// Error from [`parse_versioned_query`] or [`parse_versioned_update`].
///
/// A syntax error displays exactly as the parser's own error, so callers that
/// report `to_string()` keep their existing diagnostics.
#[derive(Debug)]
pub struct VersionedParseError {
    kind: ErrorKind,
}

#[derive(Debug)]
enum ErrorKind {
    Syntax(SparqlSyntaxError),
    Prologue(&'static str),
}

impl VersionedParseError {
    fn syntax(error: SparqlSyntaxError) -> Self {
        Self { kind: ErrorKind::Syntax(error) }
    }

    fn prologue(detail: &'static str) -> Self {
        Self { kind: ErrorKind::Prologue(detail) }
    }

    /// Returns `true` when the parser rejected the request's syntax.
    pub fn is_syntax(&self) -> bool {
        matches!(self.kind, ErrorKind::Syntax(_))
    }

    /// Returns `true` when an accepted prologue could not be accounted for.
    pub fn is_prologue(&self) -> bool {
        matches!(self.kind, ErrorKind::Prologue(_))
    }
}

impl fmt::Display for VersionedParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ErrorKind::Syntax(error) => fmt::Display::fmt(error, f),
            ErrorKind::Prologue(detail) => write!(
                f,
                "SPARQL VERSION metadata could not be recovered from the accepted prologue: {detail}"
            ),
        }
    }
}

impl std::error::Error for VersionedParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        // Syntax errors are transparent: `Display` already shows the parser's
        // message, so expose its cause rather than the error a second time.
        match &self.kind {
            ErrorKind::Syntax(error) => std::error::Error::source(error),
            ErrorKind::Prologue(_) => None,
        }
    }
}

/// Query that parses only after upstream Unicode escape preprocessing.
///
/// `\u0041` decodes to `A`, giving `ASK{}`; read verbatim, the leading
/// backslash is a syntax error. The answer depends on the linked parser's
/// `standard-unicode-escaping` feature, which any crate in the final
/// dependency graph may enable, so it is asked rather than inferred.
const PREPROCESSING_PROBE: &str = "\\u0041SK{}";

/// Labels of an accepted request, read from the text its grammar actually saw.
fn declared_versions(source: &str) -> Result<Vec<String>, VersionedParseError> {
    let text = grammar_input(source)?;
    prologue_versions(&text).map_err(VersionedParseError::prologue)
}

/// The accepted source as the linked parser's grammar read it.
fn grammar_input(source: &str) -> Result<Cow<'_, str>, VersionedParseError> {
    // Mirrors upstream: without a `\u` or `\U` pair preprocessing is skipped,
    // so the probe parse is paid only by sources containing such a pair.
    if !contains_unicode_escape(source) || !linked_parser_preprocesses_escapes() {
        return Ok(Cow::Borrowed(source));
    }
    preprocess_unicode_escapes(source).map(Cow::Owned).ok_or_else(|| {
        VersionedParseError::prologue("Unicode escape preprocessing diverged from the linked parser")
    })
}

fn contains_unicode_escape(source: &str) -> bool {
    source
        .as_bytes()
        .windows(2)
        .any(|pair| pair[0] == b'\\' && matches!(pair[1], b'u' | b'U'))
}

fn linked_parser_preprocesses_escapes() -> bool {
    SparqlParser::new().parse_query(PREPROCESSING_PROBE).is_ok()
}

/// Replica of `spargebra` 0.4.6's `standard-unicode-escaping` pass.
///
/// Upstream decodes through a small pushback buffer: a backslash replayed from
/// the buffer can start a new escape, a failed escape is emitted verbatim, and
/// hexadecimal digits are read with `u32::from_str_radix` (which accepts a
/// leading `+`). Those quirks decide prologue boundaries, so they are kept
/// rather than normalized. Returns `None` only where upstream would slice
/// inside a multi-byte character and panic, which an accepted parse cannot
/// have reached.
fn preprocess_unicode_escapes(input: &str) -> Option<String> {
    let mut chars = input.chars();
    let mut buffer = String::with_capacity(9);
    let mut output = String::with_capacity(input.len());
    loop {
        let current = if buffer.is_empty() {
            let Some(next) = chars.next() else { break };
            next
        } else {
            buffer.remove(0)
        };
        if current != '\\' {
            output.push(current);
            continue;
        }
        match chars.next() {
            Some(marker @ ('u' | 'U')) => {
                buffer.push(marker);
                let digits = if marker == 'u' { 4 } else { 8 };
                let mut complete = true;
                for _ in 0..digits {
                    match chars.next() {
                        Some(digit) => buffer.push(digit),
                        None => {
                            complete = false;
                            break;
                        }
                    }
                }
                let decoded = if complete {
                    u32::from_str_radix(buffer.get(1..)?, 16).ok().and_then(char::from_u32)
                } else {
                    None
                };
                if let Some(decoded) = decoded {
                    buffer.clear();
                    output.push(decoded);
                } else {
                    output.push('\\');
                }
            }
            Some(other) => {
                buffer.push(other);
                output.push('\\');
            }
            None => output.push('\\'),
        }
    }
    Some(output)
}

/// Every `VERSION` label of an accepted request's leading prologue.
fn prologue_versions(text: &str) -> Result<Vec<String>, &'static str> {
    let mut cursor = Cursor { text, position: 0 };
    let mut versions = Vec::new();
    cursor.skip_ignored();
    loop {
        if cursor.eat_keyword("BASE") {
            cursor.skip_ignored();
            cursor.iriref("BASE declaration without an IRI reference")?;
        } else if cursor.eat_keyword("PREFIX") {
            cursor.skip_ignored();
            cursor.pname_ns()?;
            cursor.skip_ignored();
            cursor.iriref("PREFIX declaration without an IRI reference")?;
        } else if cursor.eat_keyword("VERSION") {
            cursor.skip_ignored();
            versions.push(cursor.version_label()?);
        } else {
            return Ok(versions);
        }
        cursor.skip_ignored();
    }
}

/// Byte position in the text the parser's grammar read.
struct Cursor<'a> {
    text: &'a str,
    position: usize,
}

impl<'a> Cursor<'a> {
    fn rest(&self) -> &'a str {
        &self.text[self.position..]
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn next_char(&mut self) -> Option<char> {
        let next = self.peek()?;
        self.position += next.len_utf8();
        Some(next)
    }

    fn eat(&mut self, expected: char) -> bool {
        let matched = self.peek() == Some(expected);
        if matched {
            self.position += expected.len_utf8();
        }
        matched
    }

    /// Consumes the longest run of matching characters; `true` if non-empty.
    fn eat_while(&mut self, predicate: impl Fn(char) -> bool) -> bool {
        let start = self.position;
        while self.peek().is_some_and(&predicate) {
            self.next_char();
        }
        self.position > start
    }

    /// The parser's `i()` rule: the next `keyword.len()` characters, ASCII
    /// case-insensitively, with no word boundary required afterwards.
    fn eat_keyword(&mut self, keyword: &str) -> bool {
        let matched = self
            .rest()
            .get(..keyword.len())
            .is_some_and(|word| word.eq_ignore_ascii_case(keyword));
        if matched {
            self.position += keyword.len();
        }
        matched
    }

    /// The parser's `_` rule: whitespace and `#` comments up to a line break.
    fn skip_ignored(&mut self) {
        loop {
            match self.peek() {
                Some(' ' | '\t' | '\n' | '\r') => self.position += 1,
                Some('#') => {
                    self.eat_while(|next| next != '\r' && next != '\n');
                }
                _ => return,
            }
        }
    }

    /// `IRIREF ::= '<' [^>]* '>'`; validity was already checked by the parser.
    fn iriref(&mut self, missing: &'static str) -> Result<(), &'static str> {
        if !self.eat('<') {
            return Err(missing);
        }
        let end = self.rest().find('>').ok_or("unterminated IRI reference")?;
        self.position += end + 1;
        Ok(())
    }

    /// `PNAME_NS ::= PN_PREFIX? ':'` with the parser's greedy repetitions.
    fn pname_ns(&mut self) -> Result<(), &'static str> {
        // PN_PREFIX ::= PN_CHARS_BASE PN_CHARS* ('.'+ PN_CHARS+)*
        if self.peek().is_some_and(is_pn_chars_base) {
            self.eat_while(is_pn_chars);
            loop {
                let checkpoint = self.position;
                if !(self.eat_while(|next| next == '.') && self.eat_while(is_pn_chars)) {
                    self.position = checkpoint;
                    break;
                }
            }
        }
        if self.eat(':') {
            Ok(())
        } else {
            Err("PREFIX declaration without a prefix name")
        }
    }

    /// `STRING_LITERAL1 | STRING_LITERAL2`, decoded like the parser does.
    fn version_label(&mut self) -> Result<String, &'static str> {
        let quote = match self.next_char() {
            Some(quote @ ('\'' | '"')) => quote,
            _ => return Err("VERSION declaration without a short string label"),
        };
        let mut label = String::new();
        loop {
            match self.next_char() {
                Some(next) if next == quote => return Ok(label),
                Some('\\') => label.push(self.escaped_char()?),
                Some('\n' | '\r') | None => return Err("unterminated VERSION label"),
                Some(next) => label.push(next),
            }
        }
    }

    /// `ECHAR | UCHAR` after its backslash.
    fn escaped_char(&mut self) -> Result<char, &'static str> {
        Ok(match self.next_char() {
            Some('t') => '\t',
            Some('b') => '\u{0008}',
            Some('n') => '\n',
            Some('r') => '\r',
            Some('f') => '\u{000C}',
            Some(literal @ ('"' | '\'' | '\\')) => literal,
            Some('u') => self.hex_scalar(4)?,
            Some('U') => self.hex_scalar(8)?,
            _ => return Err("invalid escape sequence in a VERSION label"),
        })
    }

    fn hex_scalar(&mut self, digits: usize) -> Result<char, &'static str> {
        let hex = self
            .rest()
            .get(..digits)
            .filter(|hex| hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .ok_or("invalid Unicode escape in a VERSION label")?;
        self.position += digits;
        u32::from_str_radix(hex, 16)
            .ok()
            .and_then(char::from_u32)
            .ok_or("VERSION label escape is not a Unicode scalar value")
    }
}

/// `PN_CHARS_BASE` exactly as the parser's grammar spells it.
fn is_pn_chars_base(c: char) -> bool {
    matches!(c,
        'A'..='Z' | 'a'..='z' | '\u{00C0}'..='\u{00D6}' | '\u{00D8}'..='\u{00F6}'
        | '\u{00F8}'..='\u{02FF}' | '\u{0370}'..='\u{037D}' | '\u{037F}'..='\u{1FFF}'
        | '\u{200C}'..='\u{200D}' | '\u{2070}'..='\u{218F}' | '\u{2C00}'..='\u{2FEF}'
        | '\u{3001}'..='\u{D7FF}' | '\u{F900}'..='\u{FDCF}' | '\u{FDF0}'..='\u{FFFD}')
}

/// `PN_CHARS ::= PN_CHARS_U | '-' | [0-9] | #xB7 | [#x300-#x36F] | [#x203F-#x2040]`.
fn is_pn_chars(c: char) -> bool {
    c == '_'
        || is_pn_chars_base(c)
        || matches!(c, '-' | '0'..='9' | '\u{00B7}' | '\u{0300}'..='\u{036F}' | '\u{203F}'..='\u{2040}')
}

#[cfg(test)]
mod tests {
    use super::{preprocess_unicode_escapes, prologue_versions};

    type Scan = Result<Vec<String>, &'static str>;

    /// Labels as the verbatim grammar and the preprocessing grammar read `text`.
    fn both_modes(text: &str) -> (Scan, Scan) {
        let preprocessed = preprocess_unicode_escapes(text).expect("replica must not diverge");
        (prologue_versions(text), prologue_versions(&preprocessed))
    }

    fn labels(expected: &[&str]) -> Scan {
        Ok(expected.iter().map(|label| (*label).to_owned()).collect())
    }

    #[test]
    fn scanner_reads_only_prologue_declarations() {
        for (text, expected) in [
            ("SELECT * WHERE { ?s ?p ?o }", labels(&[])),
            ("", labels(&[])),
            ("VERSION '1.2' ASK {}", labels(&["1.2"])),
            ("VERSION \"1.2-basic\" ASK {}", labels(&["1.2-basic"])),
            ("version'1.1'Prefix ex:<http://ex/>vErSiOn\"1.2\"ASK{}", labels(&["1.1", "1.2"])),
            ("PREFIXex:<http://ex/>VERSION'1.1'ASK{}", labels(&["1.1"])),
            (
                "BASE <http://ex/> PREFIX a.b-c: <p#> VERSION '1.2' BASE <s/> PREFIX : <q#> VERSION '1.2' ASK {}",
                labels(&["1.2", "1.2"]),
            ),
            ("# VERSION '1.1'\r\nVERSION '1.2' # VERSION 'x'\n\tASK {}", labels(&["1.2"])),
            ("PREFIX version: <http://ex/VERSION> ASK { version:VERSION ?VERSION \"VERSION '1.1'\" }", labels(&[])),
            ("ASK {} # VERSION '1.2'", labels(&[])),
            ("VERSION 'a\\'b\\\"c\\\\d\\te\\bf\\ng\\rh\\f' ASK {}", labels(&["a'b\"c\\d\te\u{8}f\ng\rh\u{c}"])),
            ("VERSION '\u{e9}\u{2014}1' VERSION '' ASK {}", labels(&["\u{e9}\u{2014}1", ""])),
            ("VERSION '1.1'", labels(&["1.1"])),
            // A long string ends the prologue after an empty short label; the
            // parser, not this scanner, rejects the remainder.
            ("VERSION '''1.2''' ASK {}", labels(&[""])),
        ] {
            assert_eq!(prologue_versions(text), expected, "{text:?}");
        }
    }

    #[test]
    fn scanner_refuses_declarations_it_cannot_account_for() {
        for text in [
            "VERSION 1.2 ASK {}",
            "VERSION",
            "VERSION '1.2",
            "VERSION '1.2\n' ASK {}",
            "VERSION '1\\q' ASK {}",
            "VERSION '1\\u12' ASK {}",
            "VERSION '\\uD800' ASK {}",
            "VERSION '\\U00110000' ASK {}",
            "BASE http://ex/ ASK {}",
            "BASE <http://ex/",
            "PREFIX 1a: <http://ex/> ASK {}",
            "PREFIX a.: <http://ex/> ASK {}",
            "PREFIX ex <http://ex/> ASK {}",
            "PREFIX ex: http://ex/ ASK {}",
        ] {
            assert!(prologue_versions(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn replica_reproduces_upstream_preprocessing_quirks() {
        for (input, expected) in [
            ("\\u0041SK{}", "ASK{}"),
            ("\\U0001F600", "\u{1F600}"),
            // Incomplete, non-hexadecimal and surrogate escapes stay verbatim.
            ("\\u004", "\\u004"),
            ("\\uZZZZ", "\\uZZZZ"),
            ("\\uD800", "\\uD800"),
            // A backslash replayed from the pushback buffer starts an escape.
            ("\\\\u0041", "\\A"),
            // `from_str_radix` accepts a leading plus sign.
            ("\\u+056", "V"),
            // A failed escape misaligns the next one, which stays verbatim.
            ("\\u12\\u0041", "\\u12\\u0041"),
            ("\\x", "\\x"),
            ("tail\\", "tail\\"),
        ] {
            assert_eq!(preprocess_unicode_escapes(input).as_deref(), Some(expected), "{input:?}");
        }
        // Upstream would slice inside `é` and panic; the replica refuses instead.
        assert_eq!(preprocess_unicode_escapes("\\u\\\u{e9}abu1234"), None);
    }

    #[test]
    fn escaped_prologue_boundaries_follow_the_active_mode() {
        for (text, verbatim, preprocessed) in [
            ("# note\\u000A VERSION '1.2'\nASK {}", labels(&[]), labels(&["1.2"])),
            ("# note\\U0000000AVERSION \"1.1\"\nASK {}", labels(&[]), labels(&["1.1"])),
            ("VERSION \"1\\u002E2\" ASK {}", labels(&["1.2"]), labels(&["1.2"])),
            ("VERSION '\\U0001F600' ASK {}", labels(&["\u{1F600}"]), labels(&["\u{1F600}"])),
            ("VERSION '1.2' ASK { FILTER(\"\\u0041\" = \"A\") }", labels(&["1.2"]), labels(&["1.2"])),
            ("VERSION \"\\u0022\" ASK {}", labels(&["\""]), labels(&[""])),
            ("VERSION '\\\\u0031.2' ASK {}", labels(&["\\u0031.2"]), Err("")),
        ] {
            let (raw, decoded) = both_modes(text);
            assert_eq!(raw, verbatim, "verbatim {text:?}");
            match preprocessed {
                Ok(expected) => assert_eq!(decoded, Ok(expected), "preprocessed {text:?}"),
                Err(_) => assert!(decoded.is_err(), "preprocessed {text:?}"),
            }
        }
        // Escaped keyword letters form declarations only after preprocessing.
        for (text, expected) in [
            ("\\u0056ERSION '1.2' ASK {}", labels(&["1.2"])),
            ("\\u+056ERSION '1.2' ASK {}", labels(&["1.2"])),
            ("PREFIX \\u0065x: <http://ex/> VERSION '1.1' ASK {}", labels(&["1.1"])),
            ("\\u0023 VERSION '1.2'\nVERSION '1.1' ASK {}", labels(&["1.1"])),
        ] {
            let (raw, decoded) = both_modes(text);
            assert_eq!(decoded, expected, "preprocessed {text:?}");
            // Read verbatim the scanner stops before the escape; the parser rejects it.
            assert!(raw.is_err() || raw != decoded, "verbatim {text:?}");
        }
    }
}
