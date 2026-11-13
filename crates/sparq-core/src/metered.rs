//! Serial RDF ingestion with charges before retaining rows and building indexes.
use crate::dict::Dict;
use crate::meter::{Dim, Ledger, LoadError, TermRef};
use crate::Graph;
use oxrdf::{GraphName, Term};
use oxttl::{NQuadsParser, NTriplesParser, TriGParser, TurtleParser};
use std::collections::{HashMap, HashSet};

impl Graph {
    /// Load a dataset serially under `ledger`, preserving named graphs.
    /// Charges source bytes before parsing, then term bytes and facts before retaining
    /// each row. Refusals keep earlier charges; no graph is returned on failure.
    /// Formats and aliases match [`Self::load_dataset`].
    pub fn load_metered(text: &str, format: &str, ledger: &mut Ledger) -> Result<Self, LoadError> {
        load(text, format, None, true, true, ledger)
    }

    /// Load text whose external source was already charged by the caller.
    /// Intended for a source plus separately bounded generated facts. The caller
    /// must charge source bytes before allocating or appending generated text.
    /// Terms, facts and buffers retain the ordinary metered loader's checks.
    pub fn load_metered_precharged(
        text: &str,
        format: &str,
        ledger: &mut Ledger,
    ) -> Result<Self, LoadError> {
        load(text, format, None, true, false, ledger)
    }

    /// Project an already admitted dataset into one metered union graph.
    /// Charges retained terms and auxiliary rows, without charging source bytes or
    /// facts already admitted during ingestion. Named graph labels are omitted, as for a folded load.
    pub fn union_metered(&self, ledger: &mut Ledger) -> Result<Self, LoadError> {
        let mut dict = Dict::new();
        let mut rows = Vec::new();
        for graph in std::iter::once(self).chain(self.named().iter().map(|(_, graph)| graph)) {
            for row in graph.iter_ids() {
                for id in row {
                    ledger.stored_term(graph.dict(), id)?;
                }
                ledger.reserve(&mut rows, 1, Dim::Nodes)?;
                let mut projected = [0; 3];
                for (position, id) in row.into_iter().enumerate() {
                    let term = graph.dict().term(id);
                    projected[position] = dict.intern(&term);
                }
                rows.push(projected);
            }
        }
        dict.shrink_term_capacity();
        Ok(Self::from_parts(dict, rows))
    }

    /// Metered counterpart of [`Self::load_str_with_base`], folding named graphs.
    /// The source, terms and rows share the caller's ledger with subsequent work.
    /// Respects its document window; call [`Ledger::open_document`] between documents.
    pub fn load_metered_with_base(
        text: &str,
        format: &str,
        base: &str,
        ledger: &mut Ledger,
    ) -> Result<Self, LoadError> {
        load(text, format, Some(base), false, true, ledger)
    }
}

fn load(
    text: &str,
    format: &str,
    base: Option<&str>,
    dataset: bool,
    charge_source: bool,
    ledger: &mut Ledger,
) -> Result<Graph, LoadError> {
    if charge_source {
        ledger.charge(Dim::SourceBytes, text.len())?;
    }
    if dataset {
        ledger.close_document();
    }
    let mut groups: Vec<(Option<Term>, Dict, Vec<[crate::dict::Id; 3]>)> = Vec::new();
    let mut index = HashMap::new();
    let mut document_rows = HashSet::new();
    let mut push = |subject: oxrdf::NamedOrBlankNode,
                    predicate: oxrdf::NamedNode,
                    object: Term,
                    graph: GraphName|
     -> Result<(), LoadError> {
        let terms = [
            crate::subject_term(&subject),
            Term::NamedNode(predicate),
            object,
        ];
        if !dataset {
            if let Some((_, dict, _)) = groups.first() {
                let ids = terms.each_ref().map(|term| dict.lookup(term));
                if document_rows.contains(&ids) {
                    return Ok(());
                }
            }
        }
        for term in &terms {
            ledger.term(TermRef::from(term))?;
        }
        let name = if dataset {
            match graph {
                GraphName::DefaultGraph => None,
                GraphName::NamedNode(n) => Some(Term::NamedNode(n)),
                GraphName::BlankNode(b) => Some(Term::BlankNode(b)),
            }
        } else {
            None
        };
        if let Some(term) = &name {
            ledger.term(TermRef::from(term))?;
        }
        ledger.charge(Dim::Facts, 1)?;
        let slot = match index.get(&name) {
            Some(&slot) => slot,
            None => {
                ledger.reserve(&mut groups, 1, Dim::Nodes)?;
                index.try_reserve(1).map_err(|_| {
                    LoadError::Ledger(crate::meter::Exceeded::new(Dim::Alloc, usize::MAX, 1))
                })?;
                let slot = groups.len();
                index.insert(name.clone(), slot);
                groups.push((name, Dict::new(), Vec::new()));
                slot
            }
        };
        let (_, dict, rows) = &mut groups[slot];
        ledger.reserve(rows, 1, Dim::Nodes)?;
        let row = terms.each_ref().map(|term| dict.intern(term));
        if !dataset {
            document_rows.try_reserve(1).map_err(|_| {
                LoadError::Ledger(crate::meter::Exceeded::new(Dim::Alloc, usize::MAX, 1))
            })?;
            document_rows.insert(row);
        }
        rows.push(row);
        Ok(())
    };
    macro_rules! with_base {
        ($parser:expr) => {
            match base {
                Some(base) => $parser
                    .with_base_iri(base)
                    .map_err(|e| LoadError::Parse(e.to_string()))?,
                None => $parser,
            }
        };
    }
    macro_rules! quads {
        ($parser:expr) => {
            for quad in $parser.for_slice(text.as_bytes()) {
                let quad = quad.map_err(|e| LoadError::Parse(e.to_string()))?;
                push(quad.subject, quad.predicate, quad.object, quad.graph_name)?;
            }
        };
    }
    macro_rules! triples {
        ($parser:expr) => {
            for triple in $parser.for_slice(text.as_bytes()) {
                let triple = triple.map_err(|e| LoadError::Parse(e.to_string()))?;
                push(
                    triple.subject,
                    triple.predicate,
                    triple.object,
                    GraphName::DefaultGraph,
                )?;
            }
        };
    }
    match format {
        "nquads" | "n-quads" | "nq" | "application/n-quads" => quads!(NQuadsParser::new()),
        "ntriples" | "n-triples" | "nt" | "application/n-triples" => {
            triples!(NTriplesParser::new())
        }
        "trig" | "application/trig" => quads!(with_base!(TriGParser::new())),
        _ if crate::is_turtle_format(format) => triples!(with_base!(TurtleParser::new())),
        #[cfg(feature = "jsonld")]
        _ if crate::is_jsonld_format(format) => quads!(with_base!(oxjsonld::JsonLdParser::new())),
        #[cfg(feature = "rdfxml")]
        _ if crate::is_rdfxml_format(format) => triples!(with_base!(oxrdfxml::RdfXmlParser::new())),
        _ => return Err(LoadError::Parse(crate::unknown_format_err(format))),
    }
    let mut graph = None;
    let mut named = Vec::new();
    for (name, mut dict, rows) in groups {
        dict.shrink_term_capacity();
        let part = Graph::from_parts(dict, rows);
        if let Some(name) = name {
            ledger.reserve(&mut named, 1, Dim::Nodes)?;
            named.push((name, part));
        } else {
            graph = Some(part);
        }
    }
    let mut graph = graph.unwrap_or_default();
    // Every graph is freshly built; reuse the admitted vector as the ordinary loaders do.
    graph.named = named;
    Ok(graph)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meter::Limits;

    fn snapshot(graph: &Graph) -> Vec<String> {
        let mut rows: Vec<_> = graph
            .iter_ids()
            .map(|row| format!("{:?}", row.map(|id| graph.dict().term(id))))
            .collect();
        for (name, part) in graph.named() {
            rows.extend(
                part.iter_ids()
                    .map(|row| format!("{:?} {name}", row.map(|id| part.dict().term(id)))),
            );
        }
        rows.sort();
        rows
    }

    #[test]
    fn metered_load_preserves_dataset_and_folded_base_semantics() {
        for (source, format) in [
            (
                "<urn:s> <urn:p> <urn:o> <urn:g> .\n<urn:d> <urn:p> \"v\" .",
                "nquads",
            ),
            (
                "<urn:g> { <urn:s> <urn:p> <urn:o> } <urn:d> <urn:p> \"v\" .",
                "trig",
            ),
            ("<urn:s> <urn:p> \"v\" .", "ntriples"),
            ("@prefix e: <urn:> . e:s e:p \"v\" .", "turtle"),
        ] {
            let graph =
                Graph::load_metered(source, format, &mut Ledger::new(Limits::default())).unwrap();
            assert_eq!(
                snapshot(&graph),
                snapshot(&Graph::load_dataset(source, format).unwrap())
            );
            let mut projection_work = Ledger::new(Limits::default());
            let projection = graph.union_metered(&mut projection_work).unwrap();
            assert_eq!(projection_work.spent(crate::meter::Dim::SourceBytes), 0);
            assert_eq!(projection_work.spent(crate::meter::Dim::Facts), 0);
            let mut refused = Ledger::new(Limits { facts: 0, ..Limits::default() });
            assert!(graph.union_metered(&mut refused).is_ok());
            assert_eq!(
                snapshot(&projection),
                snapshot(&Graph::load_str(source, format).unwrap())
            );
            let graph = Graph::load_metered_with_base(
                source,
                format,
                "https://pod.ex/x",
                &mut Ledger::new(Limits::default()),
            )
            .unwrap();
            assert_eq!(
                snapshot(&graph),
                snapshot(&Graph::load_str_with_base(source, format, "https://pod.ex/x").unwrap())
            );
        }
        let graph = Graph::load_metered_with_base(
            "<#s> <urn:p> <relative> .",
            "ttl",
            "https://pod.ex/x",
            &mut Ledger::new(Limits::default()),
        )
        .unwrap();
        assert_eq!(
            snapshot(&graph),
            snapshot(
                &Graph::load_str_with_base("<#s> <urn:p> <relative> .", "ttl", "https://pod.ex/x")
                    .unwrap()
            )
        );
    }

    #[test]
    fn metered_load_refuses_source_and_facts_before_later_syntax() {
        for (source, bytes, facts, dimension) in [
            ("invalid", 0, usize::MAX, Dim::SourceBytes),
            (
                "<urn:s> <urn:p> <urn:o> .\ninvalid",
                usize::MAX,
                0,
                Dim::Facts,
            ),
        ] {
            let limits = Limits {
                source_bytes: bytes,
                facts,
                ..Limits::default()
            };
            let error = Graph::load_metered(source, "nquads", &mut Ledger::new(limits))
                .err()
                .unwrap();
            assert!(matches!(error, LoadError::Ledger(e) if e.dim == dimension));
        }
        let error = Graph::load_metered("invalid", "nquads", &mut Ledger::new(Limits::default()))
            .err()
            .unwrap();
        assert!(matches!(error, LoadError::Parse(_)));
    }
}
