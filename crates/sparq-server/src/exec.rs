//! Query classification + dispatch onto the engine's public API.
//!
//! To implement the *protocol* correctly we must first know the query FORM —
//! SELECT / ASK / CONSTRUCT / DESCRIBE — because each maps to a different result media
//! type and HTTP shape. We therefore parse the query once here with `spargebra` (the
//! same parser the engine uses) to classify it, then:
//!
//! * **SELECT** — run on the engine, serialise via the requested results format.
//! * **ASK** — run on the engine's native `sparq_engine::ask` boolean path.
//! * **CONSTRUCT / DESCRIBE** — run on the engine's RDF-graph result path (T16,
//!   `sparq_engine::construct` / `sparq_engine::describe`), serialised as
//!   N-Triples / Turtle per content negotiation.

use spargebra::algebra::QueryDataset;
use spargebra::term::NamedNode;
use spargebra::{GraphUpdateOperation, Query, SparqlParser, Update};

/// The classified form of a parsed SPARQL query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryForm {
    Select,
    Ask,
    Construct,
    Describe,
}

/// Result of preparing a query for execution.
pub struct Prepared {
    pub form: QueryForm,
    /// The query string the engine runs (the engine re-parses internally and has a
    /// native entry point per form: `query*` / `ask*` / `construct*` / `describe*`).
    /// Retained for the CONSTRUCT / DESCRIBE / EXPLAIN string entry points (off the hot
    /// floor path) and for the dataset-override rewrite assertions in the unit tests.
    pub runnable: String,
    /// [OPUS-4.8] (sq-7d3dj.34.1) The query PARSED ONCE here, ready to hand to the engine's
    /// `*_prepared` entry points without a second parse. `prepare` already runs the full
    /// `spargebra` parse to classify the form and (when a protocol dataset override is present)
    /// rewrite the dataset clause; carrying the resulting algebra lets the SELECT / ASK floor
    /// path skip the engine's redundant re-parse of `runnable` — the per-request parse is paid
    /// exactly once instead of twice, for ARBITRARY novel queries (no cross-request cache).
    pub query: sparq_engine::PreparedQuery,
}

/// A failure classified for the HTTP layer.
#[derive(Debug)]
pub enum PrepareError {
    /// The query string did not parse — HTTP 400.
    Malformed(String),
    /// A SPARQL-Protocol `default-graph-uri` / `named-graph-uri` value was not a valid
    /// absolute IRI — HTTP 400 (the protocol parameter is caller input). [OPUS-4.8] sq-z33x.
    BadGraphUri(String),
}

/// [OPUS-4.8] sq-z33x: the SPARQL 1.1 Protocol §2.1.4 dataset-override carried OUT-OF-BAND on
/// the request (the repeated `default-graph-uri` / `named-graph-uri` parameters), as opposed
/// to an in-query `FROM` / `FROM NAMED` clause. Per the protocol, when this override is present
/// it DEFINES the RDF Dataset the query runs against — and if the query string ALSO carries a
/// `FROM` / `FROM NAMED` clause, "the SPARQL service must execute the query using the RDF Dataset
/// given in the protocol request" (the override REPLACES, it does not merge with, the in-query
/// clause). An empty override (both lists empty) is the no-op common case.
#[derive(Debug, Default, Clone)]
pub struct DatasetOverride {
    /// `default-graph-uri` values — the FROM (default-graph) sources of the active dataset.
    pub default: Vec<String>,
    /// `named-graph-uri` values — the FROM NAMED sources of the active dataset.
    pub named: Vec<String>,
}

impl DatasetOverride {
    /// True when no override was supplied (the request carried neither parameter): the query
    /// runs exactly as written, paying nothing for the override path.
    pub fn is_empty(&self) -> bool {
        self.default.is_empty() && self.named.is_empty()
    }

    /// Builds the `spargebra` [`QueryDataset`] this override denotes, validating each IRI.
    /// `named` is `Some` iff at least one `named-graph-uri` was given: a `default-graph-uri`-only
    /// override yields `FROM …` with NO named graphs (so `GRAPH ?g` enumerates nothing), and a
    /// `named-graph-uri`-only override yields `FROM NAMED …` with an EMPTY default graph — exactly
    /// the engine's dataset-clause semantics (`sparq_engine` `build_active`). [OPUS-4.8] sq-z33x.
    fn to_query_dataset(&self) -> Result<QueryDataset, PrepareError> {
        let default = self
            .default
            .iter()
            .map(|i| parse_graph_iri(i))
            .collect::<Result<Vec<_>, _>>()?;
        let named = if self.named.is_empty() {
            None
        } else {
            Some(
                self.named
                    .iter()
                    .map(|i| parse_graph_iri(i))
                    .collect::<Result<Vec<_>, _>>()?,
            )
        };
        Ok(QueryDataset { default, named })
    }
}

/// Parses a protocol graph-URI value into a validated absolute-IRI [`NamedNode`].
fn parse_graph_iri(iri: &str) -> Result<NamedNode, PrepareError> {
    NamedNode::new(iri.to_string())
        .map_err(|e| PrepareError::BadGraphUri(format!("invalid graph IRI '{iri}': {e}")))
}

/// Parses + classifies the query, leaving the query text unchanged. Use
/// [`prepare_with_dataset`] when a SPARQL-Protocol dataset override may be present.
///
/// Note on datasets / named graphs: the engine DOES support a full RDF dataset — a default
/// graph plus named graphs — so a `GRAPH <iri>` / `GRAPH ?g` pattern, a cross-graph join,
/// and an in-query `FROM` / `FROM NAMED` dataset clause all execute correctly (conformance
/// rounds 3–4; covered end-to-end over HTTP by `tests/named_graphs.rs`, sq-fh4z). The
/// *protocol-level* dataset OVERRIDE — the SPARQL-Protocol `default-graph-uri` /
/// `named-graph-uri` request parameters — is wired via [`prepare_with_dataset`] (sq-z33x).
pub fn prepare(sparql: &str) -> Result<Prepared, PrepareError> {
    prepare_with_dataset(sparql, &DatasetOverride::default())
}

/// Parses + classifies the query, applying any SPARQL 1.1 Protocol dataset override
/// (`default-graph-uri` / `named-graph-uri`) per §2.1.4.
///
/// When `over` is non-empty the parsed query's dataset clause is REPLACED with the dataset the
/// override denotes (the protocol mandates the protocol-supplied dataset wins over an in-query
/// `FROM` / `FROM NAMED`), and `runnable` is re-serialised from the rewritten algebra so the
/// engine — which re-parses `runnable` — sees the synthesized clause. When `over` is empty the
/// query is returned verbatim (the no-op common path: no rewrite, no re-serialisation).
/// [OPUS-4.8] sq-z33x.
pub fn prepare_with_dataset(
    sparql: &str,
    over: &DatasetOverride,
) -> Result<Prepared, PrepareError> {
    let (mut parsed, version) = SparqlParser::new()
        .parse_query_with_versions(sparql)
        .map_err(|e| PrepareError::Malformed(e.to_string()))?;
    let form = match parsed {
        Query::Select { .. } => QueryForm::Select,
        Query::Ask { .. } => QueryForm::Ask,
        Query::Construct { .. } => QueryForm::Construct,
        Query::Describe { .. } => QueryForm::Describe,
    };
    let runnable = if over.is_empty() {
        sparql.to_string()
    } else {
        set_query_dataset(&mut parsed, over.to_query_dataset()?);
        // Re-serialise the rewritten algebra: spargebra's `Display` re-emits the FROM / FROM
        // NAMED clauses, and the engine re-parses this string (the CONSTRUCT / DESCRIBE path).
        let announcements = version.iter().map(|label| format!("VERSION \"{label}\"\n")).collect::<String>();
        format!("{announcements}{parsed}")
    };
    // [OPUS-4.8] (sq-7d3dj.34.1) Carry the algebra we JUST parsed (the original query, or — under
    // a dataset override — the rewritten one, which `runnable` was serialised FROM, so the two
    // denote the same query) so the SELECT / ASK floor path executes it prepared, without the
    // engine re-parsing `runnable`.
    let query = sparq_engine::PreparedQuery::from_query_with_versions(parsed, version)
        .map_err(PrepareError::Malformed)?;
    Ok(Prepared {
        form,
        runnable,
        query,
    })
}

/// Overwrites the dataset specification of a parsed query (every form carries the same
/// `Option<QueryDataset>` field). [OPUS-4.8] sq-z33x.
fn set_query_dataset(q: &mut Query, dataset: QueryDataset) {
    match q {
        Query::Select { dataset: d, .. }
        | Query::Construct { dataset: d, .. }
        | Query::Describe { dataset: d, .. }
        | Query::Ask { dataset: d, .. } => *d = Some(dataset),
    }
}

/// [OPUS-4.8] sq-z33x: the SPARQL 1.1 Protocol §2.2 UPDATE dataset override carried OUT-OF-BAND
/// (the repeated `using-graph-uri` / `using-named-graph-uri` parameters). Per the protocol these
/// re-scope the WHERE clause of every operation as if a `USING <g>` / `USING NAMED <g>` clause had
/// been written — BUT it is an error to supply them alongside an update that already carries an
/// in-string `USING` / `USING NAMED` / `WITH` clause (the override SUPPLEMENTS, it never overrides).
#[derive(Debug, Default, Clone)]
pub struct UsingOverride {
    /// `using-graph-uri` values — the default-graph (`USING <g>`) sources of the WHERE dataset.
    pub default: Vec<String>,
    /// `using-named-graph-uri` values — the `USING NAMED <g>` sources of the WHERE dataset.
    pub named: Vec<String>,
}

impl UsingOverride {
    /// True when no override was supplied (the common in-update / no-dataset case).
    pub fn is_empty(&self) -> bool {
        self.default.is_empty() && self.named.is_empty()
    }

    fn to_query_dataset(&self) -> Result<QueryDataset, UpdateDatasetError> {
        let map = |list: &[String]| {
            list.iter()
                .map(|i| {
                    NamedNode::new(i.to_string()).map_err(|e| {
                        UpdateDatasetError::BadGraphUri(format!("invalid graph IRI '{i}': {e}"))
                    })
                })
                .collect::<Result<Vec<_>, _>>()
        };
        let default = map(&self.default)?;
        let named = if self.named.is_empty() {
            None
        } else {
            Some(map(&self.named)?)
        };
        Ok(QueryDataset { default, named })
    }
}

/// A failure applying the UPDATE dataset override — both map to an HTTP 400. [OPUS-4.8] sq-z33x.
#[derive(Debug)]
pub enum UpdateDatasetError {
    /// The update did not parse.
    Malformed(String),
    /// A `using-graph-uri` / `using-named-graph-uri` value was not a valid absolute IRI.
    BadGraphUri(String),
    /// The protocol `using-*` parameters were supplied alongside an update operation that already
    /// carries an in-string `USING` / `USING NAMED` / `WITH` clause — a protocol error (§2.2).
    UsingConflict,
}

/// Applies a SPARQL 1.1 Protocol §2.2 UPDATE dataset override (`using-graph-uri` /
/// `using-named-graph-uri`) to an update string, returning the rewritten update the engine runs.
///
/// * No override → returns the update verbatim (the common path; no parse, no rewrite).
/// * Override present, but some operation already carries an in-string `USING`/`USING NAMED`/`WITH`
///   clause → [`UpdateDatasetError::UsingConflict`] (the protocol error; the HTTP layer answers 400).
/// * Otherwise the override's `USING`/`USING NAMED` dataset is injected into every `DeleteInsert`
///   operation (data-only operations — `INSERT DATA`, `LOAD`, `CLEAR`, … — have no WHERE clause to
///   re-scope and are left untouched), and the rewritten update is re-serialised for the engine.
///
/// [OPUS-4.8] sq-z33x.
pub fn apply_update_dataset(
    update: &str,
    over: &UsingOverride,
) -> Result<String, UpdateDatasetError> {
    if over.is_empty() {
        return Ok(update.to_string());
    }
    let mut parsed: Update = sparq_engine::parse_update_rec2013(update)
        .map_err(|e| UpdateDatasetError::Malformed(e.to_string()))?;
    // §2.2: it is an error to supply the protocol params when ANY operation already names its WHERE
    // dataset in-string (USING / USING NAMED / WITH — all encoded as `using: Some(..)`).
    if parsed.operations.iter().any(|op| {
        matches!(
            op,
            GraphUpdateOperation::DeleteInsert { using: Some(_), .. }
        )
    }) {
        return Err(UpdateDatasetError::UsingConflict);
    }
    let dataset = over.to_query_dataset()?;
    for op in &mut parsed.operations {
        if let GraphUpdateOperation::DeleteInsert { using, .. } = op {
            *using = Some(dataset.clone());
        }
    }
    Ok(parsed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparq_core::Graph;

    const DATA: &str = "@prefix ex: <http://ex/> . ex:a ex:p ex:b . ex:c ex:p ex:d .";

    fn g() -> Graph {
        Graph::load_str(DATA, "turtle").unwrap()
    }

    #[test]
    fn classifies_forms() {
        assert_eq!(
            prepare("SELECT * WHERE { ?s ?p ?o }").unwrap().form,
            QueryForm::Select
        );
        assert_eq!(prepare("ASK { ?s ?p ?o }").unwrap().form, QueryForm::Ask);
        assert_eq!(
            prepare("CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }")
                .unwrap()
                .form,
            QueryForm::Construct
        );
        assert_eq!(
            prepare("DESCRIBE <http://ex/a>").unwrap().form,
            QueryForm::Describe
        );
    }

    #[test]
    fn malformed_is_error() {
        assert!(matches!(
            prepare("SELECT WHERE {"),
            Err(PrepareError::Malformed(_))
        ));
    }

    #[test]
    fn ask_runs_natively_on_engine_true_and_false() {
        // ASK with a matching pattern => true.
        let p = prepare("PREFIX ex: <http://ex/> ASK { ?s ex:p ?o }").unwrap();
        assert_eq!(p.form, QueryForm::Ask);
        assert!(sparq_engine::ask(&g(), &p.runnable).unwrap());

        // ASK with no match => false.
        let p = prepare("PREFIX ex: <http://ex/> ASK { ?s ex:nope ?o }").unwrap();
        assert!(!sparq_engine::ask(&g(), &p.runnable).unwrap());
    }

    #[test]
    fn construct_runs_natively_on_engine() {
        let p = prepare("PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:q ?o } WHERE { ?s ex:p ?o }")
            .unwrap();
        assert_eq!(p.form, QueryForm::Construct);
        assert_eq!(sparq_engine::construct(&g(), &p.runnable).unwrap().len(), 2);

        let p = prepare("DESCRIBE <http://ex/a>").unwrap();
        assert_eq!(p.form, QueryForm::Describe);
        assert_eq!(sparq_engine::describe(&g(), &p.runnable).unwrap().len(), 1);
    }

    // ---------------------------------------------------------------------------
    // [OPUS-4.8] sq-z33x — SPARQL 1.1 Protocol §2.1.4 dataset override
    // ---------------------------------------------------------------------------

    #[test]
    fn empty_override_leaves_query_verbatim() {
        let q = "SELECT * WHERE { ?s ?p ?o }";
        let p = prepare_with_dataset(q, &DatasetOverride::default()).unwrap();
        assert_eq!(
            p.runnable, q,
            "an absent override must not rewrite the query"
        );
    }

    #[test]
    fn default_graph_uri_synthesizes_from_clause() {
        let over = DatasetOverride {
            default: vec!["http://ex/g".into()],
            named: vec![],
        };
        let p = prepare_with_dataset("SELECT * WHERE { ?s ?p ?o }", &over).unwrap();
        // The rewritten runnable carries a FROM clause and re-parses cleanly.
        assert!(
            p.runnable.contains("FROM <http://ex/g>"),
            "runnable: {}",
            p.runnable
        );
        assert!(
            !p.runnable.contains("FROM NAMED"),
            "default-only must not emit FROM NAMED"
        );
        assert!(SparqlParser::new().parse_query(&p.runnable).is_ok());
    }

    #[test]
    fn named_graph_uri_synthesizes_from_named_clause() {
        let over = DatasetOverride {
            default: vec![],
            named: vec!["http://ex/g".into()],
        };
        let p = prepare_with_dataset("ASK { ?s ?p ?o }", &over).unwrap();
        assert!(
            p.runnable.contains("FROM NAMED <http://ex/g>"),
            "runnable: {}",
            p.runnable
        );
        assert!(SparqlParser::new().parse_query(&p.runnable).is_ok());
    }

    #[test]
    fn override_replaces_in_query_from_clause() {
        // Per §2.1.4 the protocol dataset REPLACES the in-query clause — the only FROM in the
        // rewritten query is the override's, never the original `FROM <http://ex/orig>`.
        let over = DatasetOverride {
            default: vec!["http://ex/over".into()],
            named: vec![],
        };
        let p = prepare_with_dataset("SELECT * FROM <http://ex/orig> WHERE { ?s ?p ?o }", &over)
            .unwrap();
        assert!(
            p.runnable.contains("FROM <http://ex/over>"),
            "runnable: {}",
            p.runnable
        );
        assert!(
            !p.runnable.contains("orig"),
            "in-query FROM must be replaced: {}",
            p.runnable
        );
    }

    #[test]
    fn bad_graph_uri_is_error() {
        let over = DatasetOverride {
            default: vec!["not a valid iri".into()],
            named: vec![],
        };
        assert!(matches!(
            prepare_with_dataset("SELECT * WHERE { ?s ?p ?o }", &over),
            Err(PrepareError::BadGraphUri(_))
        ));
    }

    #[test]
    fn override_re_scopes_active_dataset_on_engine() {
        // A FROM of a graph the store does not hold ⇒ empty default graph ⇒ zero rows; the same
        // query without the override sees the store's two triples. End-to-end through the engine.
        let over = DatasetOverride {
            default: vec!["http://ex/absent".into()],
            named: vec![],
        };
        let p = prepare_with_dataset("SELECT * WHERE { ?s ?p ?o }", &over).unwrap();
        let res = sparq_engine::query(&g(), &p.runnable).unwrap();
        assert_eq!(
            res.rows.len(),
            0,
            "FROM of an absent graph must yield an empty default graph"
        );

        let plain = prepare("SELECT * WHERE { ?s ?p ?o }").unwrap();
        assert_eq!(
            sparq_engine::query(&g(), &plain.runnable)
                .unwrap()
                .rows
                .len(),
            2
        );
    }

    // [GPT-6] Protocol dataset rewriting must retain query VERSION metadata.
    #[test]
    fn version_survives_protocol_dataset_rewrite_and_conflicts_reject() {
        let over = DatasetOverride { default: vec!["http://ex/absent".into()], named: vec![] };
        for override_dataset in [&DatasetOverride::default(), &over] {
            for label in ["1.1", "1.2", "1.2-basic"] {
                let p = prepare_with_dataset(&format!("VERSION '{label}' SELECT (!!\"z\"^^<http://www.w3.org/2001/XMLSchema#boolean> AS ?v) {{}}"), override_dataset).unwrap();
                assert_eq!(p.query.versions(), [label]);
                let from_text = sparq_engine::PreparedQuery::parse(&p.runnable).unwrap();
                assert_eq!(from_text.versions(), [label]);
                let pin = sparq_engine::QueryBudget { ebv_semantics: Some(sparq_engine::EbvSemantics::Rec2013), ..Default::default() };
                let pinned = sparq_engine::query_prepared_with_budget(&g(), &p.query, &pin);
                assert_eq!(pinned.is_ok(), label == "1.1");
                let rows = sparq_engine::query_prepared(&g(), &p.query).unwrap().rows;
                assert_eq!(rows[0][0].is_none(), label != "1.1");
            }
            let p = prepare_with_dataset("VERSION '1.2' VERSION '1.2-basic' VERSION '1.2' ASK {}", override_dataset).unwrap();
            assert_eq!(p.query.versions(), ["1.2", "1.2-basic", "1.2"]);
            assert_eq!(sparq_engine::PreparedQuery::parse(&p.runnable).unwrap().versions(), p.query.versions());
        }
        assert!(prepare("VERSION 'unsupported' ASK {}").is_err());
    }

    // ---------------------------------------------------------------------------
    // [OPUS-4.8] sq-z33x — SPARQL 1.1 Protocol §2.2 UPDATE dataset override
    // ---------------------------------------------------------------------------

    #[test]
    fn empty_using_override_leaves_update_verbatim() {
        let u = "INSERT DATA { <http://ex/a> <http://ex/p> <http://ex/b> }";
        assert_eq!(
            apply_update_dataset(u, &UsingOverride::default()).unwrap(),
            u
        );
    }

    #[test]
    fn using_override_cannot_erase_unsupported_update_version() {
        // [GPT-6] Protocol dataset rewriting must reject before serialization
        // could discard a VERSION announcement. Empty overrides retain text
        // for the engine's same REC-only validation.
        let over = UsingOverride { default: vec!["urn:g".into()], named: vec![] };
        for label in ["1.2", "1.2-basic", "unknown"] {
            let text = format!("VERSION '{label}' INSERT {{?s <urn:q> ?o}} WHERE {{?s <urn:p> ?o}}");
            assert!(matches!(apply_update_dataset(&text, &over), Err(UpdateDatasetError::Malformed(_))));
            let retained = apply_update_dataset(&text, &UsingOverride::default()).unwrap();
            assert!(sparq_engine::parse_update_rec2013(&retained).is_err());
        }
        assert!(apply_update_dataset("VERSION '1.1' INSERT {?s <urn:q> ?o} WHERE {?s <urn:p> ?o}", &over).is_ok());
    }

    #[test]
    fn using_override_injects_using_into_delete_insert() {
        let over = UsingOverride {
            default: vec!["http://ex/g".into()],
            named: vec![],
        };
        let out = apply_update_dataset(
            "INSERT { ?s <http://ex/q> ?o } WHERE { ?s <http://ex/p> ?o }",
            &over,
        )
        .unwrap();
        assert!(out.contains("USING <http://ex/g>"), "out: {out}");
        // The rewritten update re-parses cleanly.
        assert!(SparqlParser::new().parse_update(&out).is_ok());
    }

    #[test]
    fn using_named_override_injects_using_named() {
        let over = UsingOverride {
            default: vec![],
            named: vec!["http://ex/g".into()],
        };
        let out = apply_update_dataset(
            "DELETE { ?s ?p ?o } WHERE { GRAPH <http://ex/g> { ?s ?p ?o } }",
            &over,
        )
        .unwrap();
        assert!(out.contains("USING NAMED <http://ex/g>"), "out: {out}");
        assert!(SparqlParser::new().parse_update(&out).is_ok());
    }

    #[test]
    fn using_override_conflicts_with_in_string_using() {
        // §2.2: supplying the protocol params alongside an in-string USING/WITH is an error.
        let over = UsingOverride {
            default: vec!["http://ex/g".into()],
            named: vec![],
        };
        assert!(matches!(
            apply_update_dataset(
                "INSERT { ?s <http://ex/q> ?o } USING <http://ex/h> WHERE { ?s <http://ex/p> ?o }",
                &over,
            ),
            Err(UpdateDatasetError::UsingConflict)
        ));
        // WITH is encoded the same way — also a conflict.
        assert!(matches!(
            apply_update_dataset(
                "WITH <http://ex/h> DELETE { ?s ?p ?o } WHERE { ?s ?p ?o }",
                &over,
            ),
            Err(UpdateDatasetError::UsingConflict)
        ));
    }

    #[test]
    fn using_override_leaves_data_only_ops_untouched() {
        // INSERT DATA has no WHERE clause to re-scope: the override applies to no operation, so the
        // update is structurally unchanged (still parses, still an INSERT DATA).
        let over = UsingOverride {
            default: vec!["http://ex/g".into()],
            named: vec![],
        };
        let out = apply_update_dataset(
            "INSERT DATA { <http://ex/a> <http://ex/p> <http://ex/b> }",
            &over,
        )
        .unwrap();
        assert!(
            !out.contains("USING"),
            "data-only op must not gain a USING: {out}"
        );
        assert!(SparqlParser::new().parse_update(&out).is_ok());
    }

    #[test]
    fn bad_using_graph_uri_is_error() {
        let over = UsingOverride {
            default: vec!["not a valid iri".into()],
            named: vec![],
        };
        assert!(matches!(
            apply_update_dataset("DELETE { ?s ?p ?o } WHERE { ?s ?p ?o }", &over),
            Err(UpdateDatasetError::BadGraphUri(_))
        ));
    }
}
