//! SPARQL 1.2 RL syntax, following data-shapes `04ecbb4` productions 1–161.
//!
//! This parser does not check variable well-formedness, stratification or evaluate
//! rules. Source slices retain lexical spelling and document-order declarations.

mod lexer;

use lexer::{Kind, Token};
use std::collections::HashSet;

/// A syntactically accepted rule set, borrowing the original source.
#[derive(Debug, PartialEq, Eq)]
pub struct Document<'a> {
    /// Declarations, rules and DATA blocks in source order.
    pub statements: Vec<Statement<'a>>,
}

/// A top-level statement. IRI and string slices include their delimiters.
#[derive(Debug, PartialEq, Eq)]
pub enum Statement<'a> {
    /// A BASE declaration, effective from this point onward.
    Base(&'a str),
    /// A namespace declaration (prefix includes its trailing colon).
    Prefix { prefix: &'a str, iri: &'a str },
    /// A version hint, retained without interpreting its value.
    Version(&'a str),
    /// An import reference; parsing never loads it.
    Imports(&'a str),
    /// A DATA block, including braces.
    Data(&'a str),
    /// A rule.
    Rule(Rule<'a>),
}

/// A rule's concrete syntax components.
#[derive(Debug, PartialEq, Eq)]
pub struct Rule<'a> {
    /// Optional lexical IRI identifying the rule.
    pub name: Option<&'a str>,
    /// Head template, including braces.
    pub head: &'a str,
    /// Whether WHERE DATA restricts matching to the base graph.
    pub base_only: bool,
    /// Body elements in evaluation order.
    pub body: Vec<Element<'a>>,
}

/// A validated rule-body element. Inner patterns have grammar-specific limits.
#[derive(Debug, PartialEq, Eq)]
pub enum Element<'a> {
    /// A contiguous triple block, including its optional final dot.
    Triples(&'a str),
    /// FILTER's constraint, without the FILTER keyword.
    Filter(&'a str),
    /// SET's variable and expression, without surrounding assignment syntax.
    Assignment {
        variable: &'a str,
        expression: &'a str,
    },
    /// NOT or NOT DATA, whose body contains only triples and filters.
    Negation {
        base_only: bool,
        body: Vec<Element<'a>>,
    },
    /// GROUP, whose pattern contains only triples, filters and assignments.
    Group {
        keys: Vec<&'a str>,
        body: Vec<Element<'a>>,
        assignments: Vec<Aggregate<'a>>,
    },
}

/// One GROUP aggregation assignment.
#[derive(Debug, PartialEq, Eq)]
pub struct Aggregate<'a> {
    /// Target variable, including `?` or `$`.
    pub variable: &'a str,
    /// Aggregate call, including parentheses.
    pub function: &'a str,
}

/// Distinguishes syntax rejection from a parser resource limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Invalid lexical or grammatical input.
    Syntax,
    /// Source size or recursive nesting exceeds the parser's bounds.
    Limit,
}

/// A parse failure positioned in the original UTF-8 source.
#[derive(Debug, PartialEq, Eq)]
pub struct ParseError {
    /// Failure category; a limit error does not establish invalid syntax.
    pub kind: ErrorKind,
    /// Zero-based byte offset.
    pub offset: usize,
    /// One-based source line.
    pub line: usize,
    /// One-based Unicode character column.
    pub column: usize,
    /// Explanation of the failure.
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.column, self.message)
    }
}
impl std::error::Error for ParseError {}

const MAX_SOURCE_BYTES: usize = 1024 * 1024;
const MAX_NESTING: usize = 64;
type Result<T> = std::result::Result<T, ParseError>;

fn error(source: &str, offset: usize, kind: ErrorKind, message: impl Into<String>) -> ParseError {
    let before = &source[..offset];
    let line = before
        .bytes()
        .filter(|b| *b == b'\n' || *b == b'\r')
        .count()
        - before
            .as_bytes()
            .windows(2)
            .filter(|w| *w == b"\r\n")
            .count()
        + 1;
    let column = before
        .rsplit(['\n', '\r'])
        .next()
        .unwrap_or("")
        .chars()
        .count()
        + 1;
    ParseError {
        kind,
        offset,
        line,
        column,
        message: message.into(),
    }
}

/// Parse a complete SPARQL-RL document, resolving IRI syntax against `base`.
///
/// `base` must be an absolute IRI. At most 1 MiB of UTF-8 source and 64 recursive
/// grammar levels are accepted. IMPORTS is retained without fetching documents.
/// Successful syntax parsing does not establish well-formedness or stratification.
pub fn parse<'a>(source: &'a str, base: &str) -> Result<Document<'a>> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err(error(
            source,
            0,
            ErrorKind::Limit,
            "Source size limit exceeded",
        ));
    }
    let iri_parser = spargebra::SparqlParser::new()
        .with_base_iri(base)
        .map_err(|e| {
            error(
                source,
                0,
                ErrorKind::Syntax,
                format!("Invalid base IRI: {e}"),
            )
        })?;
    Parser {
        source,
        tokens: lexer::lex(source)?,
        pos: 0,
        depth: 0,
        iri_parser,
        checked_iris: HashSet::new(),
    }
    .document()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Data,
    Template,
    Pattern,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum BodyMode {
    Rule,
    Negation,
    Group,
}

struct Parser<'a> {
    source: &'a str,
    tokens: Vec<Token<'a>>,
    pos: usize,
    depth: usize,
    iri_parser: spargebra::SparqlParser,
    checked_iris: HashSet<&'a str>,
}

impl<'a> Parser<'a> {
    fn offset(&self) -> usize {
        self.tokens
            .get(self.pos)
            .map_or(self.source.len(), |t| t.start)
    }
    fn fail<T>(&self, message: impl Into<String>) -> Result<T> {
        Err(error(
            self.source,
            self.offset(),
            ErrorKind::Syntax,
            message,
        ))
    }
    fn nested<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        if self.depth >= MAX_NESTING {
            return Err(error(
                self.source,
                self.offset(),
                ErrorKind::Limit,
                "Nesting limit exceeded",
            ));
        }
        self.depth += 1;
        let out = f(self);
        self.depth -= 1;
        out
    }
    fn text(&self) -> &'a str {
        self.tokens.get(self.pos).map_or("", |t| t.text)
    }
    fn at(&self, s: &str) -> bool {
        if s == "a" {
            self.text() == s
        } else {
            self.text().eq_ignore_ascii_case(s)
        }
    }
    fn eat(&mut self, s: &str) -> bool {
        if self.at(s) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, s: &str) -> Result<()> {
        if self.eat(s) {
            Ok(())
        } else {
            self.fail(format!("Expected {s}"))
        }
    }
    fn kind(&self, kind: Kind) -> bool {
        self.tokens.get(self.pos).is_some_and(|t| t.kind == kind)
    }
    fn take(&mut self, kind: Kind) -> Result<&'a str> {
        if !self.kind(kind) {
            return self.fail(format!("Expected {kind:?}"));
        }
        let text = self.text();
        self.pos += 1;
        Ok(text)
    }
    fn slice(&self, start: usize) -> &'a str {
        let end = self
            .pos
            .checked_sub(1)
            .map_or(start, |p| self.tokens[p].end);
        &self.source[start..end]
    }
    fn iri_start(&self) -> bool {
        self.kind(Kind::Iri) || self.kind(Kind::PName)
    }
    fn iri(&mut self) -> Result<&'a str> {
        if !self.iri_start() {
            return self.fail("Expected IRI");
        }
        let text = self.text();
        if !self.checked_iris.contains(text) {
            self.iri_parser
                .clone()
                .parse_query(&format!("SELECT ({text} AS ?__srl) WHERE {{}}"))
                .map_err(|e| error(self.source, self.offset(), ErrorKind::Syntax, e.to_string()))?;
            self.checked_iris.insert(text);
        }
        self.pos += 1;
        Ok(text)
    }
    fn document(mut self) -> Result<Document<'a>> {
        let mut statements = Vec::new();
        while self.pos < self.tokens.len() {
            let start = self.offset();
            let statement = if self.eat("BASE") {
                let iri = self.take(Kind::Iri)?;
                self.declaration(start, iri, None)?;
                Statement::Base(iri)
            } else if self.eat("PREFIX") {
                let prefix = self.take(Kind::PName)?;
                if prefix
                    .split_once(':')
                    .is_none_or(|(_, local)| !local.is_empty())
                {
                    return self.fail("Expected namespace prefix");
                }
                let iri = self.take(Kind::Iri)?;
                self.declaration(start, iri, Some(prefix))?;
                Statement::Prefix { prefix, iri }
            } else if self.eat("VERSION") {
                Statement::Version(self.take(Kind::ShortString)?)
            } else if self.eat("IMPORTS") {
                Statement::Imports(self.iri()?)
            } else if self.eat("DATA") {
                let brace = self.offset();
                self.expect("{")?;
                self.triples(Mode::Data)?;
                self.expect("}")?;
                Statement::Data(self.slice(brace))
            } else if self.eat("RULE") {
                let name = if self.iri_start() {
                    Some(self.iri()?)
                } else {
                    None
                };
                let head_start = self.offset();
                self.expect("{")?;
                self.triples(Mode::Template)?;
                self.expect("}")?;
                let head = self.slice(head_start);
                self.expect("WHERE")?;
                let base_only = self.eat("DATA");
                let body = self.body(BodyMode::Rule)?;
                Statement::Rule(Rule {
                    name,
                    head,
                    base_only,
                    body,
                })
            } else {
                return self.fail("Expected declaration, RULE or DATA");
            };
            statements.push(statement);
        }
        Ok(Document { statements })
    }
    fn declaration(&mut self, start: usize, iri: &str, prefix: Option<&str>) -> Result<()> {
        // A temporary BASE resolves the IRI against the current base without
        // retaining/reparsing an ever-growing prologue for every later token.
        let query = self
            .iri_parser
            .clone()
            .parse_query(&format!("BASE {iri} SELECT * WHERE {{}}"))
            .map_err(|e| error(self.source, start, ErrorKind::Syntax, e.to_string()))?;
        let resolved = query.base_iri().expect("BASE declaration").as_str();
        self.iri_parser = if let Some(prefix) = prefix {
            self.iri_parser
                .clone()
                .with_prefix(&prefix[..prefix.len() - 1], resolved)
        } else {
            self.iri_parser.clone().with_base_iri(resolved)
        }
        .map_err(|e| error(self.source, start, ErrorKind::Syntax, e.to_string()))?;
        self.checked_iris.clear();
        Ok(())
    }
    fn body(&mut self, mode: BodyMode) -> Result<Vec<Element<'a>>> {
        self.nested(|p| {
            p.expect("{")?;
            let mut elements = Vec::new();
            loop {
                let start = p.offset();
                if p.triples(Mode::Pattern)? {
                    elements.push(Element::Triples(p.slice(start)));
                }
                if p.eat("}") {
                    return Ok(elements);
                }
                let element = if p.eat("FILTER") {
                    let start = p.offset();
                    if p.at("(") {
                        p.bracketed()?;
                    } else if p.iri_start() {
                        p.iri()?;
                        p.arguments()?;
                    } else {
                        p.builtin()?;
                    }
                    Element::Filter(p.slice(start))
                } else if mode != BodyMode::Negation && p.eat("SET") {
                    p.expect("(")?;
                    let variable = p.take(Kind::Var)?;
                    p.expect(":=")?;
                    let start = p.offset();
                    p.expression()?;
                    let expression = p.slice(start);
                    p.expect(")")?;
                    Element::Assignment {
                        variable,
                        expression,
                    }
                } else if mode == BodyMode::Rule && p.eat("NOT") {
                    let base_only = p.eat("DATA");
                    Element::Negation {
                        base_only,
                        body: p.body(BodyMode::Negation)?,
                    }
                } else if mode == BodyMode::Rule && p.eat("GROUP") {
                    let mut keys = Vec::new();
                    if p.eat("BY") {
                        keys.push(p.take(Kind::Var)?);
                        while p.kind(Kind::Var) {
                            keys.push(p.take(Kind::Var)?);
                        }
                    }
                    let body = p.body(BodyMode::Group)?;
                    p.expect("[")?;
                    let mut assignments = Vec::new();
                    if !p.at("]") {
                        loop {
                            let variable = p.take(Kind::Var)?;
                            p.expect(":=")?;
                            let start = p.offset();
                            if p.eat("COUNT") {
                                p.expect("(")?;
                                p.eat("DISTINCT");
                                if !p.eat("*") {
                                    p.take(Kind::Var)?;
                                }
                            } else if p.eat("MIN") || p.eat("MAX") || p.eat("SUM") {
                                p.expect("(")?;
                                p.take(Kind::Var)?;
                            } else {
                                return p.fail("Expected COUNT, MIN, MAX or SUM");
                            }
                            p.expect(")")?;
                            assignments.push(Aggregate {
                                variable,
                                function: p.slice(start),
                            });
                            if !p.eat(",") {
                                break;
                            }
                        }
                    }
                    p.expect("]")?;
                    Element::Group {
                        keys,
                        body,
                        assignments,
                    }
                } else {
                    return p.fail("Expected rule element or closing brace");
                };
                elements.push(element);
                p.eat(".");
            }
        })
    }
    fn term_start(&self, mode: Mode) -> bool {
        self.iri_start()
            || self.kind(Kind::Blank)
            || self.kind(Kind::Number)
            || self.kind(Kind::ShortString)
            || self.kind(Kind::LongString)
            || (mode != Mode::Data && self.kind(Kind::Var))
            || self.at("true")
            || self.at("false")
            || self.at("(")
            || self.at("[")
            || self.at("<<")
            || self.at("<<(")
    }
    fn triples(&mut self, mode: Mode) -> Result<bool> {
        if !self.term_start(mode) {
            return Ok(false);
        }
        loop {
            let compound = self.at("[") && !self.next_is("]")
                || self.at("(") && !self.next_is(")")
                || self.at("<<");
            self.node(mode, true)?;
            self.properties(mode, !compound)?;
            if !self.eat(".") || !self.term_start(mode) {
                return Ok(true);
            }
        }
    }
    fn next_is(&self, s: &str) -> bool {
        self.tokens.get(self.pos + 1).is_some_and(|t| t.text == s)
    }
    fn verb_start(&self, mode: Mode) -> bool {
        self.iri_start()
            || self.at("a")
            || mode != Mode::Data && self.kind(Kind::Var)
            || mode == Mode::Pattern && (self.at("^") || self.at("("))
    }
    fn properties(&mut self, mode: Mode, required: bool) -> Result<()> {
        if !self.verb_start(mode) {
            return if required {
                self.fail("Expected predicate")
            } else {
                Ok(())
            };
        }
        loop {
            self.verb(mode)?;
            loop {
                self.node(mode, true)?;
                loop {
                    if self.eat("~") {
                        if self.iri_start() {
                            self.iri()?;
                        } else if self.kind(Kind::Blank)
                            || mode != Mode::Data && self.kind(Kind::Var)
                        {
                            self.pos += 1;
                        } else if self.eat("[") {
                            self.expect("]")?;
                        }
                    } else if self.eat("{|") {
                        self.nested(|p| p.properties(mode, true))?;
                        self.expect("|}")?;
                    } else {
                        break;
                    }
                }
                if !self.eat(",") {
                    break;
                }
            }
            if !self.eat(";") {
                return Ok(());
            }
            while self.eat(";") {}
            if !self.verb_start(mode) {
                return Ok(());
            }
        }
    }
    fn verb(&mut self, mode: Mode) -> Result<()> {
        if mode != Mode::Data && self.kind(Kind::Var) {
            self.pos += 1;
            return Ok(());
        }
        if mode == Mode::Pattern {
            return self.path();
        }
        if self.eat("a") {
            Ok(())
        } else {
            self.iri().map(|_| ())
        }
    }
    fn path(&mut self) -> Result<()> {
        loop {
            self.eat("^");
            if self.eat("(") {
                self.nested(Self::path)?;
                self.expect(")")?;
            } else if !self.eat("a") {
                self.iri()?;
            }
            if !self.eat("/") {
                return Ok(());
            }
        }
    }
    // `compound` permits collections and property lists; `reified` permits << >>.
    fn node(&mut self, mode: Mode, compound: bool) -> Result<()> {
        self.nested(|p| {
            if p.iri_start() {
                p.iri()?;
            } else if p.kind(Kind::Number)
                || p.kind(Kind::Blank)
                || mode != Mode::Data && p.kind(Kind::Var)
            {
                p.pos += 1;
            } else if p.kind(Kind::ShortString) || p.kind(Kind::LongString) {
                p.literal()?;
            } else if p.eat("true") || p.eat("false") {
            } else if p.eat("<<(") {
                p.triple_term(mode, false)?;
            } else if p.eat("<<") {
                p.triple_term(mode, true)?;
            } else if p.eat("[") {
                if !p.eat("]") {
                    if !compound {
                        return p.fail("Property list not allowed here");
                    }
                    p.properties(mode, true)?;
                    p.expect("]")?;
                }
            } else if p.eat("(") {
                if !p.eat(")") {
                    if !compound {
                        return p.fail("Collection not allowed here");
                    }
                    p.node(mode, true)?;
                    while !p.at(")") {
                        p.node(mode, true)?;
                    }
                    p.expect(")")?;
                } else if !compound {
                    return p.fail("NIL not allowed here");
                }
            } else {
                return p.fail("Expected RDF term");
            }
            Ok(())
        })
    }
    fn triple_term(&mut self, mode: Mode, reified: bool) -> Result<()> {
        if !reified && self.at("<<") {
            return self.fail("Reified triple inside triple term");
        }
        self.node(mode, false)?;
        // Triple terms and reified triples use Verb, never a property path.
        self.verb(if mode == Mode::Pattern {
            Mode::Template
        } else {
            mode
        })?;
        if !reified && self.at("<<") {
            return self.fail("Reified triple inside triple term");
        }
        self.node(mode, false)?;
        if reified && self.eat("~") {
            if self.iri_start() {
                self.iri()?;
            } else if self.kind(Kind::Blank) || mode != Mode::Data && self.kind(Kind::Var) {
                self.pos += 1;
            } else if self.eat("[") {
                self.expect("]")?;
            }
        }
        self.expect(if reified { ">>" } else { ")>>" })
    }
    fn literal(&mut self) -> Result<()> {
        self.pos += 1;
        if self.kind(Kind::Lang) {
            let tag = self.text().trim_start_matches('@');
            let language = if let Some((language, direction)) = tag.split_once("--") {
                if !matches!(direction, "ltr" | "rtl") {
                    return self.fail("Invalid base direction");
                }
                language
            } else {
                tag
            };
            oxrdf::Literal::new_language_tagged_literal("", language)
                .map_err(|e| error(self.source, self.offset(), ErrorKind::Syntax, e.to_string()))?;
            self.pos += 1;
        } else if self.eat("^^") {
            self.iri()?;
        }
        Ok(())
    }
    fn expression(&mut self) -> Result<()> {
        self.nested(|p| p.binary(0))
    }
    // Precedence levels: OR, AND, relation, addition, multiplication, primary.
    fn binary(&mut self, level: u8) -> Result<()> {
        if level == 5 {
            return self.primary();
        }
        self.binary(level + 1)?;
        loop {
            let signed =
                level == 3 && self.kind(Kind::Number) && self.text().starts_with(['+', '-']);
            let matched = match level {
                0 => self.eat("||"),
                1 => self.eat("&&"),
                2 => {
                    self.eat("=")
                        || self.eat("!=")
                        || self.eat("<=")
                        || self.eat(">=")
                        || self.eat("<")
                        || self.eat(">")
                }
                3 => !signed && (self.eat("+") || self.eat("-")),
                4 => self.eat("*") || self.eat("/"),
                _ => false,
            };
            if matched || signed {
                self.binary(level + 1)?;
            } else if level == 2 && (self.at("IN") || self.at("NOT")) {
                self.eat("NOT");
                self.expect("IN")?;
                self.arguments()?;
                return Ok(());
            } else {
                break;
            }
            if level == 2 {
                break;
            }
        }
        Ok(())
    }
    fn bracketed(&mut self) -> Result<()> {
        self.expect("(")?;
        self.expression()?;
        self.expect(")")
    }
    fn arguments(&mut self) -> Result<usize> {
        self.expect("(")?;
        let mut count = 0;
        if !self.at(")") {
            loop {
                self.expression()?;
                count += 1;
                if !self.eat(",") {
                    break;
                }
            }
        }
        self.expect(")")?;
        Ok(count)
    }
    fn primary(&mut self) -> Result<()> {
        let _ = self.eat("!") || self.eat("+") || self.eat("-");
        if self.at("(") {
            self.bracketed()
        } else if self.iri_start() {
            self.iri()?;
            if self.at("(") {
                self.arguments()?;
            }
            Ok(())
        } else if self.kind(Kind::Var) || self.kind(Kind::Number) {
            self.pos += 1;
            Ok(())
        } else if self.kind(Kind::ShortString) || self.kind(Kind::LongString) {
            self.literal()
        } else if self.eat("true") || self.eat("false") {
            Ok(())
        } else if self.eat("<<(") {
            self.expr_triple(false)?;
            Ok(())
        } else {
            self.builtin()
        }
    }
    fn expr_triple(&mut self, object: bool) -> Result<()> {
        self.nested(|p| {
            if object && p.eat("<<(") {
                return p.expr_triple(false);
            }
            if p.iri_start() {
                p.iri()?;
            } else if p.kind(Kind::ShortString) || p.kind(Kind::LongString) {
                p.literal()?;
            } else if p.kind(Kind::Var) || p.kind(Kind::Number) {
                p.pos += 1;
            } else if p.eat("true") || p.eat("false") {
            } else {
                return p.fail("Expected expression triple term component");
            }
            if !object {
                p.verb(Mode::Template)?;
                p.expr_triple(true)?;
                p.expect(")>>")?;
            }
            Ok(())
        })
    }
    fn builtin(&mut self) -> Result<()> {
        let name = self.text().to_ascii_uppercase();
        let (min, max) = match name.as_str() {
            "NOW" | "UUID" | "STRUUID" => (0, 0),
            "BNODE" => (0, 1),
            "CONCAT" => (0, usize::MAX),
            "SUBSTR" | "REGEX" => (2, 3),
            "REPLACE" => (3, 4),
            "IF" | "STRLANGDIR" | "TRIPLE" => (3, 3),
            "LANGMATCHES" | "CONTAINS" | "STRSTARTS" | "STRENDS" | "STRBEFORE" | "STRAFTER"
            | "STRLANG" | "STRDT" | "SAMETERM" => (2, 2),
            "STR" | "LANG" | "LANGDIR" | "DATATYPE" | "IRI" | "URI" | "ABS" | "CEIL" | "FLOOR"
            | "ROUND" | "STRLEN" | "UCASE" | "LCASE" | "ENCODE_FOR_URI" | "YEAR" | "MONTH"
            | "DAY" | "HOURS" | "MINUTES" | "SECONDS" | "TIMEZONE" | "TZ" | "ISIRI" | "ISURI"
            | "ISBLANK" | "ISLITERAL" | "ISNUMERIC" | "HASLANG" | "HASLANGDIR" | "ISTRIPLE"
            | "SUBJECT" | "PREDICATE" | "OBJECT" => (1, 1),
            _ => return self.fail("Expected SPARQL-RL expression or built-in"),
        };
        self.pos += 1;
        let count = self.arguments()?;
        if count < min || count > max {
            return self.fail(format!("Wrong arity for {name}"));
        }
        Ok(())
    }
}
