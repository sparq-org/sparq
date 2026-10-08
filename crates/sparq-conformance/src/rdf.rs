//! Minimal RDF plumbing for the harness: parse a test file into triples with a
//! `file://` base IRI, and query the resulting triple soup manifest-style.

use oxrdf::{NamedOrBlankNode, Quad, Term, Triple};
use std::path::{Path, PathBuf};

/// The `file://` IRI of a (test-suite) file — the base against which every
/// relative IRI in manifests, data files and queries is resolved, so that
/// graph names referenced from queries line up with the loaded graph data.
pub fn file_iri(path: &Path) -> String {
    format!("file://{}", path.display())
}

/// Inverse of [`file_iri`].
pub fn iri_to_path(iri: &str) -> Option<PathBuf> {
    iri.strip_prefix("file://").map(PathBuf::from)
}

/// Parses an RDF document (Turtle / N-Triples by default, RDF/XML for `.rdf`)
/// into triples, resolving relative IRIs against the file's own location.
pub fn parse_file(path: &Path) -> Result<Vec<Triple>, String> {
    let text = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let base = file_iri(path);
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let mut triples = Vec::new();
    if ext == "rdf" {
        let parser = oxrdfxml::RdfXmlParser::new()
            .with_base_iri(&base)
            .map_err(|e| e.to_string())?;
        for t in parser.for_slice(&text) {
            triples.push(t.map_err(|e| format!("{}: {e}", path.display()))?);
        }
    } else {
        // Turtle is a superset of N-Triples, so `.ttl` and `.nt` both go here.
        let parser = oxttl::TurtleParser::new()
            .with_base_iri(&base)
            .map_err(|e| e.to_string())?;
        for t in parser.for_slice(&text) {
            triples.push(t.map_err(|e| format!("{}: {e}", path.display()))?);
        }
    }
    Ok(triples)
}

/// Parses an RDF document into QUADS: TriG (`.trig`) and N-Quads (`.nq`) keep
/// their named graphs; everything [`parse_file`] handles lands in the default
/// graph. Relative IRIs resolve against the file's own location.
pub fn parse_file_quads(path: &Path) -> Result<Vec<Quad>, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if !matches!(ext, "trig" | "nq") {
        return Ok(parse_file(path)?
            .into_iter()
            .map(|t| {
                Quad::new(
                    t.subject,
                    t.predicate,
                    t.object,
                    oxrdf::GraphName::DefaultGraph,
                )
            })
            .collect());
    }
    let text = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let mut quads = Vec::new();
    if ext == "nq" {
        for q in oxttl::NQuadsParser::new().for_slice(&text) {
            quads.push(q.map_err(|e| format!("{}: {e}", path.display()))?);
        }
    } else {
        let parser = oxttl::TriGParser::new()
            .with_base_iri(file_iri(path))
            .map_err(|e| e.to_string())?;
        for q in parser.for_slice(&text) {
            quads.push(q.map_err(|e| format!("{}: {e}", path.display()))?);
        }
    }
    Ok(quads)
}

/// A parsed manifest / expected-result document with the lookups the harness needs.
pub struct MiniGraph {
    pub triples: Vec<Triple>,
}

impl MiniGraph {
    pub fn load(path: &Path) -> Result<Self, String> {
        Ok(Self {
            triples: parse_file(path)?,
        })
    }

    pub fn objects<'a>(&'a self, s: &NamedOrBlankNode, p: &str) -> Vec<&'a Term> {
        self.triples
            .iter()
            .filter(|t| &t.subject == s && t.predicate.as_str() == p)
            .map(|t| &t.object)
            .collect()
    }

    pub fn object<'a>(&'a self, s: &NamedOrBlankNode, p: &str) -> Option<&'a Term> {
        self.objects(s, p).into_iter().next()
    }

    /// The lexical form of a literal object, if present.
    pub fn str_object(&self, s: &NamedOrBlankNode, p: &str) -> Option<String> {
        match self.object(s, p) {
            Some(Term::Literal(l)) => Some(l.value().to_string()),
            _ => None,
        }
    }

    pub fn subjects_with_type(&self, type_iri: &str) -> Vec<NamedOrBlankNode> {
        self.triples
            .iter()
            .filter(|t| {
                t.predicate.as_str() == "http://www.w3.org/1999/02/22-rdf-syntax-ns#type"
                    && matches!(&t.object, Term::NamedNode(n) if n.as_str() == type_iri)
            })
            .map(|t| t.subject.clone())
            .collect()
    }

    /// All `rdf:type` IRIs of a node.
    pub fn types_of(&self, s: &NamedOrBlankNode) -> Vec<String> {
        self.objects(s, "http://www.w3.org/1999/02/22-rdf-syntax-ns#type")
            .into_iter()
            .filter_map(|t| match t {
                Term::NamedNode(n) => Some(n.as_str().to_string()),
                _ => None,
            })
            .collect()
    }

    /// Walks an `rdf:first`/`rdf:rest` list starting at `head`.
    pub fn list(&self, head: &Term) -> Vec<Term> {
        const FIRST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#first";
        const REST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#rest";
        const NIL: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#nil";
        let mut out = Vec::new();
        let mut cur = head.clone();
        loop {
            let node = match &cur {
                Term::NamedNode(n) if n.as_str() == NIL => break,
                Term::NamedNode(n) => NamedOrBlankNode::NamedNode(n.clone()),
                Term::BlankNode(b) => NamedOrBlankNode::BlankNode(b.clone()),
                _ => break,
            };
            match self.object(&node, FIRST) {
                Some(f) => out.push(f.clone()),
                None => break,
            }
            match self.object(&node, REST) {
                Some(r) => cur = r.clone(),
                None => break,
            }
        }
        out
    }
}

/// Converts a term reference usable as a manifest node, or `None` for literals.
pub fn as_node(t: &Term) -> Option<NamedOrBlankNode> {
    match t {
        Term::NamedNode(n) => Some(NamedOrBlankNode::NamedNode(n.clone())),
        Term::BlankNode(b) => Some(NamedOrBlankNode::BlankNode(b.clone())),
        _ => None,
    }
}
