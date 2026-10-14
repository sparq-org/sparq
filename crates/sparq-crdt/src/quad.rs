//! Canonical, blank-node-free N-Quads quad lines — the `quad` field of the
//! envelope (`CRDT-WIRE-2`) and of snapshot store entries.
//!
//! Canonical form is defined operationally: the line must be
//! byte-identical to re-serialising its parsed terms through the same
//! `oxrdf` Display path `sparq-canon` uses (default graph as a 3-term line —
//! `GraphName`'s own Display renders the default graph as the word `DEFAULT`,
//! which is not N-Quads). Blank nodes are rejected in every position
//! (`CRDT-BNODE-1`: replication identity is skolem IRIs; the wire never
//! carries blank-node labels), and RDF 1.2 triple terms are rejected because
//! they are outside version 1 of the design (research §4.1).

use crate::CrdtError;
use oxrdf::{GraphName, NamedOrBlankNode, Quad, Term};
use oxttl::NQuadsParser;

/// One validated, canonical, blank-node-free N-Quads line (no trailing
/// newline): `<s> <p> <o> .` or `<s> <p> <o> <g> .`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CanonicalQuad(String);

impl CanonicalQuad {
    /// Validates one canonical N-Quads line of at most `max_bytes` bytes.
    ///
    /// Rejections: oversized input; embedded line breaks (an envelope quad is
    /// exactly one line); N-Quads syntax errors; blank nodes in any position;
    /// RDF 1.2 triple terms; and any non-canonical byte form (extra
    /// whitespace, non-canonical escapes, an explicit `xsd:string` datatype,
    /// comments, …) via parse → re-serialise → byte-compare.
    pub fn parse(line: &str, max_bytes: usize) -> Result<Self, CrdtError> {
        if line.len() > max_bytes {
            return Err(CrdtError::Oversized {
                what: "quad bytes",
                len: line.len(),
                max: max_bytes,
            });
        }
        if line.contains('\n') || line.contains('\r') {
            return Err(CrdtError::Invalid {
                what: "quad",
                reason: "must be a single N-Quads line without line breaks".into(),
            });
        }
        let mut quads = Vec::with_capacity(1);
        for parsed in NQuadsParser::new().for_slice(line.as_bytes()) {
            let quad = parsed.map_err(|e| CrdtError::Invalid {
                what: "quad",
                reason: format!("not valid N-Quads: {e}"),
            })?;
            quads.push(quad);
        }
        if quads.len() != 1 {
            return Err(CrdtError::Invalid {
                what: "quad",
                reason: format!("expected exactly 1 quad, found {}", quads.len()),
            });
        }
        let quad = &quads[0];
        reject_forbidden_terms(quad)?;
        let canonical = serialize_quad(quad);
        if canonical != line {
            return Err(CrdtError::NonCanonical {
                reason: format!(
                    "quad line is not in canonical N-Quads form (canonical: {canonical:?})"
                ),
            });
        }
        Ok(CanonicalQuad(canonical))
    }

    /// The canonical N-Quads line (no trailing newline).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Blank nodes are forbidden in every position (`CRDT-BNODE-1`); triple terms
/// are outside version 1.
fn reject_forbidden_terms(quad: &Quad) -> Result<(), CrdtError> {
    let blank = |position: &str| CrdtError::Invalid {
        what: "quad",
        reason: format!("blank node in {position} position; the wire carries skolem IRIs only"),
    };
    match &quad.subject {
        NamedOrBlankNode::NamedNode(_) => {}
        NamedOrBlankNode::BlankNode(_) => return Err(blank("subject")),
    }
    match &quad.object {
        Term::NamedNode(_) | Term::Literal(_) => {}
        Term::BlankNode(_) => return Err(blank("object")),
        Term::Triple(_) => {
            return Err(CrdtError::Invalid {
                what: "quad",
                reason: "RDF 1.2 triple terms are outside SPARQL-CRDT version 1".into(),
            });
        }
    }
    match &quad.graph_name {
        GraphName::DefaultGraph | GraphName::NamedNode(_) => {}
        GraphName::BlankNode(_) => return Err(blank("graph")),
    }
    Ok(())
}

/// The canonical serialisation (the `sparq-canon` bridge convention).
fn serialize_quad(quad: &Quad) -> String {
    match &quad.graph_name {
        GraphName::DefaultGraph => {
            format!("{} {} {} .", quad.subject, quad.predicate, quad.object)
        }
        g => format!("{} {} {} {} .", quad.subject, quad.predicate, quad.object, g),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX: usize = 16 * 1024;

    #[test]
    fn parse_accepts_canonical_default_and_named_graph_lines() {
        let g3 = "<https://ex/s> <https://ex/p> \"new\" .";
        assert_eq!(CanonicalQuad::parse(g3, MAX).unwrap().as_str(), g3);
        let g4 = "<https://ex/s> <https://ex/p> \"new\" <https://ex/g> .";
        assert_eq!(CanonicalQuad::parse(g4, MAX).unwrap().as_str(), g4);
        let typed = "<https://ex/s> <https://ex/p> \"1\"^^<http://www.w3.org/2001/XMLSchema#integer> .";
        assert_eq!(CanonicalQuad::parse(typed, MAX).unwrap().as_str(), typed);
        let lang = "<https://ex/s> <https://ex/p> \"hi\"@en .";
        assert_eq!(CanonicalQuad::parse(lang, MAX).unwrap().as_str(), lang);
    }

    #[test]
    fn parse_rejects_non_canonical_byte_forms() {
        for line in [
            "<https://ex/s>  <https://ex/p> \"x\" .",       // double space
            "<https://ex/s> <https://ex/p> \"x\".",         // missing space before dot
            " <https://ex/s> <https://ex/p> \"x\" .",       // leading space
            "<https://ex/s> <https://ex/p> \"x\" . ",       // trailing space
            "<https://ex/s> <https://ex/p> \"x\" . # note", // comment
            // Explicit xsd:string is non-canonical (simple-literal form wins).
            "<https://ex/s> <https://ex/p> \"x\"^^<http://www.w3.org/2001/XMLSchema#string> .",
        ] {
            assert!(
                matches!(CanonicalQuad::parse(line, MAX), Err(CrdtError::NonCanonical { .. })),
                "{line:?} must be rejected as non-canonical"
            );
        }
    }

    #[test]
    fn parse_rejects_blank_nodes_in_every_position() {
        for line in [
            "_:b <https://ex/p> \"x\" .",
            "<https://ex/s> <https://ex/p> _:b .",
            "<https://ex/s> <https://ex/p> \"x\" _:g .",
        ] {
            assert!(
                CanonicalQuad::parse(line, MAX).is_err(),
                "{line:?} must be rejected: blank node"
            );
        }
    }

    #[test]
    fn parse_rejects_syntax_errors_multi_quads_and_line_breaks() {
        assert!(CanonicalQuad::parse("not n-quads", MAX).is_err());
        assert!(CanonicalQuad::parse("", MAX).is_err());
        assert!(CanonicalQuad::parse(
            "<https://ex/s> <https://ex/p> \"a\" .\n<https://ex/s> <https://ex/p> \"b\" .",
            MAX
        )
        .is_err());
    }

    #[test]
    fn parse_enforces_the_byte_bound() {
        let line = "<https://ex/s> <https://ex/p> \"x\" .";
        assert!(matches!(
            CanonicalQuad::parse(line, 10),
            Err(CrdtError::Oversized { .. })
        ));
    }

    #[test]
    fn as_str_returns_the_canonical_line() {
        let line = "<https://ex/s> <https://ex/p> <https://ex/o> .";
        assert_eq!(CanonicalQuad::parse(line, MAX).unwrap().as_str(), line);
    }
}
