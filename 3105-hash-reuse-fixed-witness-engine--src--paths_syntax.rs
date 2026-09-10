//! Dedicated syntax front end for materialized full-path queries.
//!
//! This module deliberately does not extend `spargebra`: [`query_paths`] is the
//! only query entry point accepting `PATHS`, leaving the standard SPARQL surface
//! and its conformance behavior unchanged.
//!
//! The result follows a row-per-hop shape. Every row binds the requested start
//! and end variables, `?pathIndex` (zero-based within the result), `?hopIndex`
//! (zero-based within the path), `?node` (the node reached by that hop), and
//! `?edge` (the predicate traversed by that hop).

use oxrdf::vocab::xsd;
use oxrdf::{Literal, NamedNode, Term, Variable};
use spargebra::algebra::GraphPattern;
use spargebra::term::NamedNodePattern;
use spargebra::{Query, SparqlParser};
use sparq_core::Graph;

use crate::{enumerate_paths, Endpoint, PathMode, PathSpec, QueryResult, Via};

#[derive(Debug)]
struct ParsedPaths {
    mode: PathMode,
    cyclic: bool,
    start_var: Variable,
    start: Option<Endpoint>,
    end_var: Variable,
    end: Option<Endpoint>,
    via: Via,
    max_length: Option<usize>,
}

/// Parses and evaluates the opt-in `PATHS` query form.
pub fn query_paths(graph: &Graph, src: &str) -> Result<QueryResult, String> {
    let parsed = parse_paths(src)?;
    let solutions = enumerate_paths(
        graph,
        &PathSpec {
            mode: parsed.mode,
            cyclic: parsed.cyclic,
            start: parsed.start,
            end: parsed.end,
            via: parsed.via,
            max_length: parsed.max_length,
        },
    )?;
    let vars = vec![
        parsed.start_var,
        parsed.end_var,
        Variable::new_unchecked("pathIndex"),
        Variable::new_unchecked("hopIndex"),
        Variable::new_unchecked("node"),
        Variable::new_unchecked("edge"),
    ];
    let mut rows = Vec::new();
    for (path_index, path) in solutions.into_iter().enumerate() {
        let start = path
            .nodes
            .first()
            .cloned()
            .expect("enumerated paths are non-empty");
        let end = path
            .nodes
            .last()
            .cloned()
            .expect("enumerated paths are non-empty");
        for (hop_index, edge) in path.edges.into_iter().enumerate() {
            rows.push(vec![
                Some(start.clone()),
                Some(end.clone()),
                Some(integer(path_index)),
                Some(integer(hop_index)),
                Some(path.nodes[hop_index + 1].clone()),
                Some(edge),
            ]);
        }
    }
    Ok(QueryResult { vars, rows })
}

/// Renders the typed operator represented by an opt-in `PATHS` query.
pub fn explain_paths(_graph: &Graph, src: &str) -> Result<String, String> {
    let parsed = parse_paths(src)?;
    let mode = match parsed.mode {
        PathMode::Shortest => "shortest",
        PathMode::All => "all",
    };
    let maximum = parsed
        .max_length
        .map_or_else(|| "none".to_owned(), |n| n.to_string());
    Ok(format!(
        "Paths mode={mode} cyclic={} via={} maxLength={maximum}",
        parsed.cyclic,
        display_via(&parsed.via)
    ))
}

fn integer(value: usize) -> Term {
    Term::Literal(Literal::new_typed_literal(value.to_string(), xsd::INTEGER))
}

fn parse_paths(src: &str) -> Result<ParsedPaths, String> {
    let tokens = tokenize(src)?;
    let paths_at = tokens
        .iter()
        .position(|token| token.eq_ignore_ascii_case("PATHS"))
        .ok_or_else(|| "expected PATHS query form".to_owned())?;
    let prologue = tokens[..paths_at].join(" ");
    validate_prologue(&prologue)?;
    let mut parser = Tokens::new(&tokens[paths_at..]);
    parser.keyword("PATHS")?;
    let mode = if parser.consume_keyword("SHORTEST") {
        PathMode::Shortest
    } else if parser.consume_keyword("ALL") {
        PathMode::All
    } else {
        return Err("PATHS requires SHORTEST or ALL".to_owned());
    };
    let cyclic = parser.consume_keyword("CYCLIC");
    parser.keyword("START")?;
    let start_var = parser.variable()?;
    let start = parser.optional_endpoint(&prologue, &start_var)?;
    parser.keyword("END")?;
    let end_var = parser.variable()?;
    let end = parser.optional_endpoint(&prologue, &end_var)?;
    parser.keyword("VIA")?;
    let via_token = parser.next("IRI or graph pattern after VIA")?;
    let via = if via_token.starts_with('{') {
        Via::Pattern(canonical_pattern(
            &prologue,
            via_token,
            &[Variable::new_unchecked("from"), Variable::new_unchecked("to")],
            "VIA",
        )?)
    } else {
        Via::Predicate(resolve_iri(&prologue, via_token)?)
    };
    let max_length = if parser.consume_keyword("MAX") {
        parser.keyword("LENGTH")?;
        Some(
            parser
                .next("integer after MAX LENGTH")?
                .parse::<usize>()
                .map_err(|_| "MAX LENGTH must be a non-negative integer".to_owned())?,
        )
    } else {
        None
    };
    if parser.remaining() {
        return Err(format!(
            "unexpected token `{}` after PATHS query",
            parser.peek().unwrap()
        ));
    }
    if mode == PathMode::All && max_length.is_none() {
        return Err("PATHS ALL requires MAX LENGTH".to_owned());
    }
    Ok(ParsedPaths {
        mode,
        cyclic,
        start_var,
        start,
        end_var,
        end,
        via,
        max_length,
    })
}

fn display_via(via: &Via) -> String {
    match via {
        Via::Predicate(predicate) => format!("<{}>", predicate.as_str()),
        Via::Pattern(pattern) => format!("{{ {pattern} }}"),
    }
}

/// Resolves a braced graph-pattern token against the prologue and re-serializes it
/// as a prefix-free canonical form, so downstream evaluation needs no prologue.
fn canonical_pattern(
    prologue: &str,
    token: &str,
    project: &[Variable],
    clause: &str,
) -> Result<String, String> {
    let source = token
        .strip_prefix('{')
        .and_then(|value| value.strip_suffix('}'))
        .ok_or_else(|| format!("unterminated graph pattern after {}", clause))?
        .trim();
    let projection = project
        .iter()
        .map(Variable::to_string)
        .collect::<Vec<_>>()
        .join(" ");
    let query = SparqlParser::new()
        .parse_query(&format!(
            "{} SELECT {} WHERE {{ {} }}",
            prologue, projection, source
        ))
        .map_err(|error| format!("invalid graph pattern after {}: {}", clause, error))?;
    let Query::Select { pattern, .. } = query else {
        unreachable!()
    };
    let GraphPattern::Project { inner, .. } = pattern else {
        return Err(format!("invalid SELECT projection for PATHS {} pattern", clause));
    };
    Ok(inner.to_string())
}

fn validate_prologue(prologue: &str) -> Result<(), String> {
    if prologue.is_empty() {
        return Ok(());
    }
    SparqlParser::new()
        .parse_query(&format!("{prologue} ASK {{ }}"))
        .map(|_| ())
        .map_err(|error| format!("invalid PATHS prologue: {error}"))
}

fn resolve_iri(prologue: &str, token: &str) -> Result<NamedNode, String> {
    let query = SparqlParser::new()
        .parse_query(&format!(
            "{prologue} SELECT * WHERE {{ <urn:sparq:source> {token} <urn:sparq:target> }}"
        ))
        .map_err(|error| format!("invalid IRI `{token}`: {error}"))?;
    let Query::Select { pattern, .. } = query else {
        unreachable!()
    };
    let pattern = match pattern {
        GraphPattern::Project { inner, .. } => *inner,
        other => other,
    };
    let GraphPattern::Bgp { patterns } = pattern else {
        unreachable!()
    };
    let NamedNodePattern::NamedNode(node) = &patterns[0].predicate else {
        return Err(format!("expected IRI, found `{token}`"));
    };
    Ok(node.clone())
}

fn tokenize(src: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut chars = src.char_indices().peekable();
    while let Some((start, ch)) = chars.next() {
        if ch.is_whitespace() {
            continue;
        }
        if ch == '#' {
            while chars.next().is_some_and(|(_, c)| c != '\n') {}
            continue;
        }
        if ch == '=' {
            tokens.push("=".to_owned());
            continue;
        }
        if ch == '<' {
            let mut end = None;
            for (index, c) in chars.by_ref() {
                if c == '>' {
                    end = Some(index + 1);
                    break;
                }
            }
            let end = end.ok_or_else(|| "unterminated IRI reference".to_owned())?;
            tokens.push(src[start..end].to_owned());
            continue;
        }
        if ch == '{' {
            let end = braced_group_end(src, start)?;
            while chars.peek().is_some_and(|(index, _)| *index < end) {
                chars.next();
            }
            tokens.push(src[start..end].to_owned());
            continue;
        }
        let mut end = start + ch.len_utf8();
        while let Some(&(index, c)) = chars.peek() {
            if c.is_whitespace() || c == '=' || c == '#' || c == '<' {
                break;
            }
            chars.next();
            end = index + c.len_utf8();
        }
        tokens.push(src[start..end].to_owned());
    }
    Ok(tokens)
}

fn braced_group_end(src: &str, start: usize) -> Result<usize, String> {
    let mut depth = 0usize;
    let mut quote = None;
    let mut escaped = false;
    let mut iri = false;
    let mut comment = false;
    for (offset, ch) in src[start..].char_indices() {
        if comment {
            if ch == '\n' {
                comment = false;
            }
            continue;
        }
        if let Some(delimiter) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == delimiter {
                quote = None;
            }
            continue;
        }
        if iri {
            if ch == '>' {
                iri = false;
            }
            continue;
        }
        match ch {
            '#' => comment = true,
            '<' => iri = true,
            '\'' | '"' => quote = Some(ch),
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(start + offset + ch.len_utf8());
                }
            }
            _ => {}
        }
    }
    Err("unterminated graph pattern after VIA".to_owned())
}

struct Tokens<'a> {
    tokens: &'a [String],
    position: usize,
}

impl<'a> Tokens<'a> {
    fn new(tokens: &'a [String]) -> Self {
        Self {
            tokens,
            position: 0,
        }
    }
    fn peek(&self) -> Option<&str> {
        self.tokens.get(self.position).map(String::as_str)
    }
    fn next(&mut self, expected: &str) -> Result<&'a str, String> {
        let token = self
            .tokens
            .get(self.position)
            .ok_or_else(|| format!("expected {expected}"))?;
        self.position += 1;
        Ok(token)
    }
    fn keyword(&mut self, expected: &str) -> Result<(), String> {
        let found = self.next(expected)?;
        if found.eq_ignore_ascii_case(expected) {
            Ok(())
        } else {
            Err(format!("expected {expected}, found `{found}`"))
        }
    }
    fn consume_keyword(&mut self, keyword: &str) -> bool {
        if self
            .peek()
            .is_some_and(|token| token.eq_ignore_ascii_case(keyword))
        {
            self.position += 1;
            true
        } else {
            false
        }
    }
    fn variable(&mut self) -> Result<Variable, String> {
        let token = self.next("variable")?;
        let name = token
            .strip_prefix('?')
            .or_else(|| token.strip_prefix('$'))
            .ok_or_else(|| format!("expected variable, found `{token}`"))?;
        Variable::new(name).map_err(|error| error.to_string())
    }
    /// Parses the optional `= <iri>` or `= { pattern }` restriction on an endpoint.
    /// A pattern must bind the endpoint variable declared just before it.
    fn optional_endpoint(
        &mut self,
        prologue: &str,
        variable: &Variable,
    ) -> Result<Option<Endpoint>, String> {
        if self.peek() != Some("=") {
            return Ok(None);
        }
        self.position += 1;
        let token = self.next("IRI or graph pattern after =")?;
        if token.starts_with('{') {
            Ok(Some(Endpoint::Pattern {
                source: canonical_pattern(
                    prologue,
                    token,
                    std::slice::from_ref(variable),
                    "endpoint",
                )?,
                variable: variable.clone(),
            }))
        } else {
            Ok(Some(Endpoint::Node(Term::NamedNode(resolve_iri(
                prologue, token,
            )?))))
        }
    }
    fn remaining(&self) -> bool {
        self.position < self.tokens.len()
    }
}
