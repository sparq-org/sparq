//! W3C test-manifest parsing: walks `mf:include` trees and `mf:entries` lists,
//! extracting `mf:QueryEvaluationTest` / `mf:UpdateEvaluationTest` entries with
//! their `qt:`/`ut:` action descriptions, plus the four
//! `mf:{Positive,Negative}[Update]SyntaxTest*` kinds (action = the bare file).

use crate::rdf::{as_node, iri_to_path, MiniGraph};
use oxrdf::{NamedOrBlankNode, Term};
use std::path::{Path, PathBuf};

pub const MF: &str = "http://www.w3.org/2001/sw/DataAccess/tests/test-manifest#";
pub const QT: &str = "http://www.w3.org/2001/sw/DataAccess/tests/test-query#";
pub const UT: &str = "http://www.w3.org/2009/sparql/tests/test-update#";
pub const DAWGT: &str = "http://www.w3.org/2001/sw/DataAccess/tests/test-dawg#";
const RDFS_LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";

#[derive(Debug, Clone, PartialEq)]
pub enum EntryKind {
    QueryEval,
    UpdateEval,
    /// `mf:PositiveSyntaxTest*` / `mf:PositiveUpdateSyntaxTest*` — the document
    /// must parse. `update` selects `parse_update` over `parse_query`.
    PositiveSyntax {
        update: bool,
    },
    /// `mf:NegativeSyntaxTest*` / `mf:NegativeUpdateSyntaxTest*` — the document
    /// must be REJECTED by the parser.
    NegativeSyntax {
        update: bool,
    },
    /// Protocol / CSV-format / … tests — out of scope for this runner.
    Other(String),
}

/// One named graph in a test dataset: `(graph IRI, data file)`.
pub type GraphData = (String, PathBuf);

#[derive(Debug, Clone, Default)]
pub struct QueryAction {
    pub query: Option<PathBuf>,
    pub data: Vec<PathBuf>,
    pub graph_data: Vec<GraphData>,
    /// `sd:entailmentRegime` / `qt:serviceData` present — feature the SPARQL
    /// evaluation runner skips (the inference runner handles regimes it knows).
    pub unsupported_feature: Option<String>,
    /// The `sd:entailmentRegime` IRIs of the action (e.g. `ent:RDFS`), when
    /// present — consumed by the entailment-regime runner in `inference`.
    pub entailment_regimes: Vec<String>,
    /// [OPUS-4.8] sq-kuvu3 (epic sq-pbz04) — the `sd:EntailmentProfile` IRIs of
    /// the action (e.g. `pr:QL`), when present. A test lists every OWL 2 profile
    /// its expected answers are sanctioned under; the EXPERIMENTAL OWL 2 QL
    /// query-rewriting arm (`inference::sparql_entail`, behind the opt-in
    /// `ql-experimental` feature) selects the `pr:QL`-tagged cases and reports
    /// what the `sparq-reason-ql` rewriter genuinely computes — as
    /// experimental/OutOfScope, never a graduated conformance pass.
    pub entailment_profiles: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateState {
    pub data: Vec<PathBuf>,
    pub graph_data: Vec<GraphData>,
}

#[derive(Debug, Clone)]
pub struct TestEntry {
    /// The entry IRI (test id).
    pub id: String,
    pub name: String,
    /// Leaf-manifest directory relative to the suites root, e.g. `sparql11/aggregates`.
    pub suite: String,
    pub kind: EntryKind,
    pub withdrawn: bool,
    pub action: QueryAction,
    /// Query tests: expected-result file.
    pub result_file: Option<PathBuf>,
    /// Update tests: `ut:request` plus pre/post dataset states.
    pub update_request: Option<PathBuf>,
    pub update_pre: UpdateState,
    pub update_post: UpdateState,
}

/// Recursively loads a manifest (following `mf:include`) and appends its test
/// entries. `suite_root` is the directory suite labels are made relative to.
pub fn collect(
    manifest_path: &Path,
    suite_root: &Path,
    out: &mut Vec<TestEntry>,
) -> Result<(), String> {
    let manifest_path = manifest_path
        .canonicalize()
        .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let g = MiniGraph::load(&manifest_path)?;
    let manifests = g.subjects_with_type(&format!("{MF}Manifest"));
    let suite = manifest_path
        .parent()
        .and_then(|d| d.strip_prefix(suite_root).ok())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| manifest_path.display().to_string());

    for m in &manifests {
        // mf:include ( <sub/manifest.ttl> … )
        if let Some(inc) = g.object(m, &format!("{MF}include")) {
            for item in g.list(inc) {
                if let Term::NamedNode(n) = &item {
                    if let Some(p) = iri_to_path(n.as_str()) {
                        collect(&p, suite_root, out)?;
                    }
                }
            }
        }
        // mf:entries ( :test1 :test2 … )
        if let Some(entries) = g.object(m, &format!("{MF}entries")) {
            for item in g.list(entries) {
                if let Some(node) = as_node(&item) {
                    out.push(parse_entry(&g, &node, &suite));
                }
            }
        }
    }
    Ok(())
}

fn parse_entry(g: &MiniGraph, node: &NamedOrBlankNode, suite: &str) -> TestEntry {
    let id = match node {
        NamedOrBlankNode::NamedNode(n) => n.as_str().to_string(),
        NamedOrBlankNode::BlankNode(b) => format!("_:{}", b.as_str()),
    };
    let types = g.types_of(node);
    // Local names of the entry's mf: types, version suffix stripped — the suites
    // use mf:PositiveSyntaxTest (1.0/1.2) and mf:PositiveSyntaxTest11 (1.1) etc.
    // for the same test shape.
    let local = |t: &String| -> Option<String> {
        t.strip_prefix(MF)
            .map(|n| n.trim_end_matches(|c: char| c.is_ascii_digit()).to_string())
    };
    let has = |name: &str| types.iter().filter_map(local).any(|n| n == name);
    let kind = if has("QueryEvaluationTest") {
        EntryKind::QueryEval
    } else if has("UpdateEvaluationTest") {
        EntryKind::UpdateEval
    } else if has("PositiveSyntaxTest") {
        EntryKind::PositiveSyntax { update: false }
    } else if has("NegativeSyntaxTest") {
        EntryKind::NegativeSyntax { update: false }
    } else if has("PositiveUpdateSyntaxTest") {
        EntryKind::PositiveSyntax { update: true }
    } else if has("NegativeUpdateSyntaxTest") {
        EntryKind::NegativeSyntax { update: true }
    } else {
        EntryKind::Other(
            types
                .first()
                .map(|t| t.rsplit(['#', '/']).next().unwrap_or(t).to_string())
                .unwrap_or_else(|| "untyped".into()),
        )
    };
    let name = g
        .str_object(node, &format!("{MF}name"))
        .unwrap_or_else(|| id.clone());
    let withdrawn = matches!(
        g.object(node, &format!("{DAWGT}approval")),
        Some(Term::NamedNode(n)) if n.as_str() == format!("{DAWGT}Withdrawn")
    );

    let mut entry = TestEntry {
        id,
        name,
        suite: suite.to_string(),
        kind,
        withdrawn,
        action: QueryAction::default(),
        result_file: None,
        update_request: None,
        update_pre: UpdateState::default(),
        update_post: UpdateState::default(),
    };

    if let Some(action_t) = g.object(node, &format!("{MF}action")) {
        if let Some(action) = as_node(action_t) {
            entry.action = parse_query_action(g, &action, action_t);
            if let Some(Term::NamedNode(n)) = g.object(&action, &format!("{UT}request")) {
                entry.update_request = iri_to_path(n.as_str());
            }
            entry.update_pre = parse_update_state(g, &action);
        }
    }
    if let Some(result_t) = g.object(node, &format!("{MF}result")) {
        match result_t {
            Term::NamedNode(n) => entry.result_file = iri_to_path(n.as_str()),
            _ => {
                if let Some(result) = as_node(result_t) {
                    entry.update_post = parse_update_state(g, &result);
                }
            }
        }
    }
    entry
}

fn parse_query_action(g: &MiniGraph, action: &NamedOrBlankNode, action_term: &Term) -> QueryAction {
    let mut qa = QueryAction::default();
    match g.object(action, &format!("{QT}query")) {
        Some(Term::NamedNode(n)) => qa.query = iri_to_path(n.as_str()),
        // Syntax tests put the query file directly as mf:action.
        _ => {
            if let Term::NamedNode(n) = action_term {
                qa.query = iri_to_path(n.as_str());
            }
        }
    }
    for d in g.objects(action, &format!("{QT}data")) {
        if let Term::NamedNode(n) = d {
            if let Some(p) = iri_to_path(n.as_str()) {
                qa.data.push(p);
            }
        }
    }
    for gd in g.objects(action, &format!("{QT}graphData")) {
        match gd {
            Term::NamedNode(n) => {
                if let Some(p) = iri_to_path(n.as_str()) {
                    qa.graph_data.push((n.as_str().to_string(), p));
                }
            }
            // [ qt:graph <file> ; rdfs:label "iri" ]
            Term::BlankNode(_) => {
                if let Some(b) = as_node(gd) {
                    if let (Some(Term::NamedNode(f)), Some(label)) = (
                        g.object(&b, &format!("{QT}graph")),
                        g.str_object(&b, RDFS_LABEL),
                    ) {
                        if let Some(p) = iri_to_path(f.as_str()) {
                            qa.graph_data.push((label, p));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    for (pred, feat) in [
        (
            "http://www.w3.org/ns/sparql-service-description#entailmentRegime",
            "entailment regime",
        ),
        (&format!("{QT}serviceData") as &str, "SERVICE / federation"),
    ] {
        if g.object(action, pred).is_some() {
            qa.unsupported_feature = Some(feat.to_string());
        }
    }
    for r in g.objects(
        action,
        "http://www.w3.org/ns/sparql-service-description#entailmentRegime",
    ) {
        match r {
            Term::NamedNode(n) => qa.entailment_regimes.push(n.as_str().to_string()),
            // The suite writes a LIST of regimes: ( ent:RDF ent:RDFS … ).
            Term::BlankNode(_) => {
                for item in g.list(r) {
                    if let Term::NamedNode(n) = item {
                        qa.entailment_regimes.push(n.as_str().to_string());
                    }
                }
            }
            _ => {}
        }
    }
    // [OPUS-4.8] sq-kuvu3 — `sd:EntailmentProfile` (the OWL 2 profile set a
    // test's expected answers are sanctioned under: `pr:DL`/`pr:EL`/`pr:QL`/
    // `pr:RL`/`pr:Full`). Same single-IRI-or-list shape as the regimes above.
    for p in g.objects(
        action,
        "http://www.w3.org/ns/sparql-service-description#EntailmentProfile",
    ) {
        match p {
            Term::NamedNode(n) => qa.entailment_profiles.push(n.as_str().to_string()),
            Term::BlankNode(_) => {
                for item in g.list(p) {
                    if let Term::NamedNode(n) = item {
                        qa.entailment_profiles.push(n.as_str().to_string());
                    }
                }
            }
            _ => {}
        }
    }
    qa
}

fn parse_update_state(g: &MiniGraph, node: &NamedOrBlankNode) -> UpdateState {
    let mut st = UpdateState::default();
    for d in g.objects(node, &format!("{UT}data")) {
        if let Term::NamedNode(n) = d {
            if let Some(p) = iri_to_path(n.as_str()) {
                st.data.push(p);
            }
        }
    }
    for gd in g.objects(node, &format!("{UT}graphData")) {
        if let Some(b) = as_node(gd) {
            // [ ut:graph <file> ; rdfs:label "graph-iri" ]
            if let (Some(Term::NamedNode(f)), Some(label)) = (
                g.object(&b, &format!("{UT}graph")),
                g.str_object(&b, RDFS_LABEL),
            ) {
                if let Some(p) = iri_to_path(f.as_str()) {
                    st.graph_data.push((label, p));
                }
            }
        }
    }
    st
}
