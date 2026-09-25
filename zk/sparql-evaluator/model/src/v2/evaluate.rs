// [GPT-6] Complete source/catalog authentication and evaluation run in the guest.
use super::*;
use crate::evaluate::{DatasetProfile, admit_query, execute, term_string};
use oxrdf::{GraphName, NamedNode, NamedOrBlankNode, Term};
use sparq_core::{Graph, dict::Dict};
use std::collections::BTreeMap;

/// Checks the complete V2 query against the bounded named-dataset profile.
///
/// # Errors
/// Rejects unsupported forms, nondeterminism and excessive inputs.
pub fn admit(request: &Request) -> Result<(), Rejected> {
    validate_request(request)?;
    let query = spargebra::SparqlParser::new()
        .parse_query(&request.query)
        .map_err(|_| Rejected("SPARQL parse rejected"))?;
    admit_query(&query, DatasetProfile::NamedCatalog)
}

/// Executes the complete bounded dataset and produces its V2 public journal.
///
/// Every quad and empty named graph is anchored before parsing or evaluation.
/// Dataset clauses select from this local snapshot only; they never retrieve an
/// IRI over the network. The existing engine's active-dataset view performs the
/// RDF merge and named-graph selection inside this relation.
///
/// # Errors
/// Rejects mismatched anchors, unsupported source/query terms, catalog omissions,
/// malformed source and exhausted query or dataset capacities.
pub fn evaluate(witness: &Witness) -> Result<Journal, Rejected> {
    validate_request(&witness.request)?;
    let request = &witness.request;
    let commitment = dataset_commitment(&witness.dataset, &request.policy)?;
    let provenance = match request.authority {
        DatasetAuthority::VerifierAgreed {
            commitment: expected,
        } => {
            if commitment != expected {
                return Err(Rejected("V2 complete dataset anchor mismatch"));
            }
            Provenance::VerifierAcceptedCommitment
        }
        DatasetAuthority::HolderDeclared => Provenance::HolderDeclaredOnly,
    };
    let prepared = sparq_engine::PreparedQuery::parse(&request.query)
        .map_err(|_| Rejected("SPARQL parse rejected"))?;
    admit_query(prepared.query(), DatasetProfile::NamedCatalog)?;
    let graph = build_dataset(&witness.dataset, &request.policy)?;
    let result = execute(&graph, &prepared, request.policy.max_rows)?;
    Ok(Journal {
        version: VERSION,
        request_digest: request_digest(request)?,
        dataset_commitment: commitment,
        provenance,
        result,
    })
}

fn build_dataset(dataset: &PrivateDataset, policy: &Policy) -> Result<Graph, Rejected> {
    let names = canonical_catalog(dataset, policy)?;
    let mut slots = BTreeMap::new();
    let mut builders = Vec::with_capacity(names.len() + 1);
    builders.push((Dict::new(), Vec::new()));
    let mut graph_names = Vec::with_capacity(names.len());
    for name in names {
        let node = NamedNode::new(name).map_err(|_| Rejected("V2 graph IRI rejected"))?;
        slots.insert(name, builders.len());
        builders.push((Dict::new(), Vec::new()));
        graph_names.push(node);
    }
    for (quad_count, quad) in oxttl::NQuadsParser::new()
        .for_slice(dataset.nquads.as_bytes())
        .enumerate()
    {
        if quad_count >= policy.max_triples as usize {
            return Err(Rejected("V2 source quad capacity"));
        }
        let quad = quad.map_err(|_| Rejected("N-Quads parse rejected"))?;
        let NamedOrBlankNode::NamedNode(subject) = quad.subject else {
            return Err(Rejected("dataset blank nodes are not admitted"));
        };
        term_string(&quad.object)?;
        let slot = match &quad.graph_name {
            GraphName::DefaultGraph => 0,
            GraphName::NamedNode(name) => *slots.get(name.as_str()).ok_or(Rejected(
                "V2 source graph is missing from the complete catalog",
            ))?,
            GraphName::BlankNode(_) => {
                return Err(Rejected("blank-node graph names are not admitted"));
            }
        };
        let (dict, triples) = &mut builders[slot];
        triples.push([
            dict.intern(&Term::NamedNode(subject)),
            dict.intern(&Term::NamedNode(quad.predicate)),
            dict.intern(&quad.object),
        ]);
    }
    let mut graphs = builders
        .into_iter()
        .map(|(dict, triples)| Graph::from_parts(dict, triples));
    let mut graph = graphs
        .next()
        .ok_or(Rejected("V2 default graph construction"))?;
    graph.named = graph_names
        .into_iter()
        .zip(graphs)
        .map(|(name, graph)| (Term::NamedNode(name), graph))
        .collect();
    Ok(graph)
}
