//! Per-test execution: load the test dataset into a `sparq_core::Graph`, run
//! the query/update through `sparq_engine`, and compare against the expected
//! results. Each engine invocation runs on a watchdog thread so a hang or
//! panic in the engine becomes a recorded FAIL instead of killing the run.

use crate::compare::{rows_equal, Row};
use crate::manifest::TestEntry;
use crate::rdf::{file_iri, iri_to_path, parse_file, parse_file_quads};
use crate::results::{parse_expected, Binding, Expected};
use oxrdf::{BlankNode, GraphName, Quad, Term, Triple};
use spargebra::algebra::GraphPattern;
use spargebra::{Query, SparqlParser};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

/// Per-test wall-clock budget before the watchdog declares a FAIL(timeout).
const TEST_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone)]
pub enum Status {
    Pass,
    Fail(String),
    Skip(String),
}

/// Builds an N-Quads document for the test dataset: `data` files contribute
/// their own quads (TriG/N-Quads named graphs preserved, Turtle = default
/// graph), `graph_data` files are triples poured into their labelled graph.
/// Blank node labels are prefixed per-file so distinct files never share bnodes.
fn build_nquads(data: &[PathBuf], graph_data: &[(String, PathBuf)]) -> Result<String, String> {
    let mut out = String::new();
    let mut file_idx = 0usize;
    for d in data {
        for q in parse_file_quads(d)? {
            let q = prefix_bnodes_quad(q, file_idx);
            match &q.graph_name {
                GraphName::DefaultGraph => {
                    out.push_str(&format!("{} {} {} .\n", q.subject, q.predicate, q.object))
                }
                g => out.push_str(&format!(
                    "{} {} {} {g} .\n",
                    q.subject, q.predicate, q.object
                )),
            }
        }
        file_idx += 1;
    }
    for (g, d) in graph_data {
        for t in parse_file(d)? {
            let t = prefix_bnodes(t, file_idx);
            out.push_str(&format!(
                "{} {} {} <{}> .\n",
                t.subject, t.predicate, t.object, g
            ));
        }
        file_idx += 1;
    }
    Ok(out)
}

/// Prefixes every blank node label in a term with a per-file marker, recursing
/// into RDF 1.2 triple terms.
fn prefix_bnodes_term(term: Term, idx: usize) -> Term {
    match term {
        Term::BlankNode(b) => {
            Term::BlankNode(BlankNode::new_unchecked(format!("f{idx}x{}", b.as_str())))
        }
        Term::Triple(t) => Term::Triple(Box::new(prefix_bnodes(*t, idx))),
        other => other,
    }
}

fn prefix_bnodes(t: Triple, idx: usize) -> Triple {
    let subject = match prefix_bnodes_term(Term::from(t.subject), idx) {
        Term::NamedNode(n) => oxrdf::NamedOrBlankNode::NamedNode(n),
        Term::BlankNode(b) => oxrdf::NamedOrBlankNode::BlankNode(b),
        _ => unreachable!("subject is never a literal/triple term"),
    };
    Triple {
        subject,
        predicate: t.predicate,
        object: prefix_bnodes_term(t.object, idx),
    }
}

fn prefix_bnodes_quad(q: Quad, idx: usize) -> Quad {
    let t = prefix_bnodes(Triple::new(q.subject, q.predicate, q.object), idx);
    let graph_name = match q.graph_name {
        GraphName::BlankNode(b) => {
            GraphName::BlankNode(BlankNode::new_unchecked(format!("f{idx}x{}", b.as_str())))
        }
        other => other,
    };
    Quad::new(t.subject, t.predicate, t.object, graph_name)
}

/// Loads the N-Quads dataset and then registers any EXPLICITLY-NAMED graphs that
/// contributed no quads: a named graph can exist and be EMPTY (`qt:graphData` /
/// `ut:graphData` pointing at an empty document), and `GRAPH ?g {}` / `GRAPH <g> {}`
/// must see it — but the N-Quads encoding cannot represent a graph with no triples,
/// so the loader is told the graph names separately.
fn load_dataset_with_graphs(
    nquads: &str,
    graph_names: &[String],
) -> Result<sparq_core::Graph, String> {
    let mut graph = sparq_core::Graph::load_dataset(nquads, "nquads")?;
    for name in graph_names {
        let Ok(nn) = oxrdf::NamedNode::new(name.clone()) else {
            continue;
        };
        let t = Term::NamedNode(nn);
        if !graph.named.iter().any(|(g, _)| *g == t) {
            graph.named.push((
                t,
                sparq_core::Graph::from_parts(sparq_core::dict::Dict::new(), Vec::new()),
            ));
        }
    }
    Ok(graph)
}

/// The documents a query's dataset clause (FROM / FROM NAMED) dereferences:
/// per the test-suite convention, those IRIs name *documents* next to the query
/// file, and the dataset is constructed from them. Each referenced IRI that is
/// not already a named graph of the test dataset and resolves to an existing
/// file is appended as `(graph IRI, file)` graph data — the engine then
/// assembles the active dataset from the store's named graphs. Loading each
/// document separately keeps blank nodes distinct per document (RDF merge
/// semantics) via the per-file prefixing in [`build_nquads`].
fn add_dataset_clause_docs(
    dataset: Option<&spargebra::algebra::QueryDataset>,
    graph_data: &mut Vec<(String, PathBuf)>,
) {
    let Some(ds) = dataset else { return };
    let named = ds.named.as_deref().unwrap_or_default();
    for iri in ds.default.iter().chain(named) {
        if graph_data.iter().any(|(g, _)| g == iri.as_str()) {
            continue; // already loaded (qt:graphData, or repeated in the clause)
        }
        if let Some(path) = iri_to_path(iri.as_str()) {
            if path.is_file() {
                graph_data.push((iri.as_str().to_string(), path));
            }
        }
    }
}

/// Prepends a BASE so relative IRIs in the query resolve against the query
/// file's location (the engine API takes no base; an explicit in-text BASE
/// later in the prologue simply overrides this one, which is fine).
fn with_base(text: &str, base: &str) -> String {
    format!("BASE <{base}>\n{text}")
}

/// True when the outermost solution modifiers include ORDER BY (the only case
/// where the W3C tests demand sequence comparison).
fn is_ordered(p: &GraphPattern) -> bool {
    match p {
        GraphPattern::OrderBy { .. } => true,
        GraphPattern::Project { inner, .. }
        | GraphPattern::Distinct { inner }
        | GraphPattern::Reduced { inner }
        | GraphPattern::Slice { inner, .. } => is_ordered(inner),
        _ => false,
    }
}

/// Syntax tests: positive = the document must parse (spargebra, our parser
/// dependency — so this measures the parser conformance posture we inherit),
/// negative = the parser must REJECT the document. `update` picks
/// `parse_update` over `parse_query`. Runs on a watchdog thread like the
/// evaluation tests so a parser panic/hang is a recorded FAIL.
pub fn run_syntax_test(entry: &TestEntry, positive: bool, update: bool) -> Status {
    let Some(path) = entry.action.query.clone() else {
        return Status::Fail("manifest entry has no action file".into());
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => return Status::Fail(format!("read action file: {e}")),
    };
    let base = file_iri(&path);

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| {
            let parser = SparqlParser::new()
                .with_base_iri(&base)
                .map_err(|e| format!("bad base IRI: {e}"))?;
            if update {
                parser
                    .parse_update(&text)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            } else {
                parser
                    .parse_query(&text)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
        })();
        let _ = tx.send(result);
    });
    match rx.recv_timeout(TEST_TIMEOUT) {
        Ok(Ok(())) if positive => Status::Pass,
        Ok(Ok(())) => Status::Fail("negative syntax: parser accepted an invalid document".into()),
        Ok(Err(_)) if !positive => Status::Pass,
        Ok(Err(e)) => Status::Fail(format!("positive syntax: parse error: {e}")),
        Err(mpsc::RecvTimeoutError::Timeout) => Status::Fail("timeout (20s)".into()),
        Err(mpsc::RecvTimeoutError::Disconnected) => Status::Fail("parser panicked".into()),
    }
}

pub fn run_query_test(entry: &TestEntry) -> Status {
    if let Some(feat) = &entry.action.unsupported_feature {
        return Status::Skip(format!("{feat} not supported"));
    }
    run_query_test_on(entry, None)
}

/// The answer restriction of the SPARQL 1.1 Entailment Regimes spec, applied
/// to engine solutions produced by query-over-materialized-closure evaluation
/// (the closure may entail solutions the regime's conditions exclude).
pub struct EntailmentAnswerFilter {
    /// Blank-node labels occurring in the queried graph SG (the test's input
    /// data, BEFORE materialization). Condition (C1) of every regime requires
    /// `sk(P(BGP))` to be ground, and the Skolemization function `sk` is
    /// defined exactly for the blank nodes of SG — so a binding to any OTHER
    /// blank node (e.g. one introduced by the saturation) leaves `sk(P(BGP))`
    /// non-ground and is never a solution (Entailment Regimes §2 condition C1;
    /// §3.1: "new blank nodes introduced in the saturation process are not to
    /// be returned in the solutions").
    pub data_bnodes: rustc_hash::FxHashSet<String>,
    /// Variables standing in class-name / property-name positions of the BGP,
    /// populated when the test's expected answers are sanctioned under the
    /// OWL 2 Direct Semantics regime (`sd:entailmentRegime` includes
    /// OWL-Direct). Under that regime variables stand "in place of class
    /// names, object property names, datatype property names, individual
    /// names, or literals" (Entailment Regimes §7) — an anonymous class
    /// expression (a blank node) is not a class NAME, so a bnode binding for
    /// such a variable is not a solution there, and the suite's expected
    /// results for the dual-tagged (OWL-Direct + OWL-RDF-Based) tests are the
    /// Direct-regime answers.
    pub name_position_vars: rustc_hash::FxHashSet<String>,
}

impl EntailmentAnswerFilter {
    /// True when the regime's conditions exclude this solution.
    fn drops(&self, order: &[String], row: &Row) -> bool {
        row.iter().zip(order.iter()).any(|(t, v)| match t {
            Some(Term::BlankNode(b)) => {
                !self.data_bnodes.contains(b.as_str()) || self.name_position_vars.contains(v)
            }
            _ => false,
        })
    }
}

/// As [`run_query_test`], but with an optional N-Quads DATA OVERRIDE replacing
/// the entry's own data files — the entailment-regime runner materializes the
/// closure first and evaluates the query over it (and deliberately bypasses
/// the `unsupported_feature` skip, which exists for the plain SPARQL runner).
pub fn run_query_test_on(entry: &TestEntry, data_override: Option<String>) -> Status {
    run_query_test_filtered(entry, data_override, None)
}

/// As [`run_query_test_on`], with the entailment regimes' answer restriction
/// applied to the engine's solutions before comparison (SELECT/ASK only; the
/// suite's entailment tests are all SELECT).
pub fn run_query_test_filtered(
    entry: &TestEntry,
    data_override: Option<String>,
    filter: Option<&EntailmentAnswerFilter>,
) -> Status {
    let Some(query_path) = entry.action.query.clone() else {
        return Status::Fail("manifest entry has no qt:query".into());
    };
    let query_text = match std::fs::read_to_string(&query_path) {
        Ok(t) => t,
        Err(e) => return Status::Fail(format!("read query: {e}")),
    };
    let base = file_iri(&query_path);

    // Classify the query up front (the engine re-parses internally).
    let parsed = match SparqlParser::new().with_base_iri(&base) {
        Ok(p) => p.parse_query(&query_text),
        Err(e) => return Status::Fail(format!("bad base IRI: {e}")),
    };
    // A dataset clause (FROM / FROM NAMED) names documents to load as named
    // graphs; the engine builds the active dataset from them.
    let mut graph_data = entry.action.graph_data.clone();
    if let Ok(q) = &parsed {
        add_dataset_clause_docs(q.dataset(), &mut graph_data);
    }
    let (pattern, is_ask) = match &parsed {
        Ok(Query::Select { pattern, .. }) => (pattern, false),
        Ok(Query::Ask { pattern, .. }) => (pattern, true),
        Ok(Query::Construct { .. }) => {
            return run_construct_test(entry, &query_text, &base, &graph_data, data_override);
        }
        Ok(Query::Describe { .. }) => {
            // The engine implements DESCRIBE (CBD), but the spec leaves the result
            // form to the implementation, so expected graphs are not comparable.
            return Status::Skip(
                "DESCRIBE result form is implementation-defined (engine returns CBD)".into(),
            );
        }
        Err(e) => return Status::Fail(format!("query parse error: {e}")),
    };
    let ordered = is_ordered(pattern);

    let Some(result_path) = entry.result_file.clone() else {
        return Status::Skip("no mf:result".into());
    };
    let expected = match parse_expected(&result_path) {
        Ok(e) => e,
        Err(e) if e.starts_with("unsupported result format") => return Status::Skip(e),
        Err(e) => return Status::Fail(format!("expected-result parse error: {e}")),
    };

    let nquads = match data_override {
        Some(n) => n,
        None => match build_nquads(&entry.action.data, &graph_data) {
            Ok(n) => n,
            Err(e) => return Status::Fail(format!("data load error: {e}")),
        },
    };
    let query_with_base = with_base(&query_text, &base);

    // ASK: run on the engine's native boolean path and compare to the expected boolean.
    if is_ask {
        let expected_bool = match expected {
            Expected::Boolean(b) => b,
            Expected::Bindings { .. } => {
                return Status::Fail("expected result is a binding set, query is ASK".into())
            }
        };
        let graph_names: Vec<String> = graph_data.iter().map(|(g, _)| g.clone()).collect();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = (|| {
                let graph = load_dataset_with_graphs(&nquads, &graph_names)?;
                sparq_engine::ask(&graph, &query_with_base)
            })();
            let _ = tx.send(result);
        });
        return match rx.recv_timeout(TEST_TIMEOUT) {
            Ok(Ok(actual)) if actual == expected_bool => Status::Pass,
            Ok(Ok(actual)) => Status::Fail(format!(
                "ASK mismatch: expected {expected_bool}, got {actual}"
            )),
            Ok(Err(e)) => Status::Fail(format!("engine error: {e}")),
            Err(mpsc::RecvTimeoutError::Timeout) => Status::Fail("timeout (20s)".into()),
            Err(mpsc::RecvTimeoutError::Disconnected) => Status::Fail("engine panicked".into()),
        };
    }

    // Engine invocation on a watchdog thread.
    let graph_names: Vec<String> = graph_data.iter().map(|(g, _)| g.clone()).collect();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| {
            let graph = load_dataset_with_graphs(&nquads, &graph_names)?;
            let res = sparq_engine::query(&graph, &query_with_base)?;
            let vars: Vec<String> = res.vars.iter().map(|v| v.as_str().to_string()).collect();
            Ok::<_, String>((vars, res.rows))
        })();
        let _ = tx.send(result);
    });
    let (actual_vars, actual_rows) = match rx.recv_timeout(TEST_TIMEOUT) {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => return Status::Fail(format!("engine error: {e}")),
        Err(mpsc::RecvTimeoutError::Timeout) => return Status::Fail("timeout (20s)".into()),
        Err(mpsc::RecvTimeoutError::Disconnected) => return Status::Fail("engine panicked".into()),
    };

    let (exp_vars, exp_rows, indexed) = match expected {
        Expected::Bindings {
            vars,
            rows,
            indexed,
        } => (vars, rows, indexed),
        Expected::Boolean(b) => {
            return Status::Fail(format!("expected result is boolean ({b}), query is SELECT"))
        }
    };

    // Variable sets must agree (when the expected file declares them).
    let actual_var_set: BTreeSet<&str> = actual_vars.iter().map(|s| s.as_str()).collect();
    if !exp_vars.is_empty() {
        let exp_var_set: BTreeSet<&str> = exp_vars.iter().map(|s| s.as_str()).collect();
        if exp_var_set != actual_var_set {
            return Status::Fail(format!(
                "variables mismatch: expected {{{}}}, got {{{}}}",
                exp_vars.join(", "),
                actual_vars.join(", ")
            ));
        }
    }

    // Align both sides on a shared variable order.
    let mut all_vars: BTreeSet<String> = actual_vars.iter().cloned().collect();
    all_vars.extend(exp_vars.iter().cloned());
    for row in &exp_rows {
        all_vars.extend(row.iter().map(|(v, _)| v.clone()));
    }
    let order: Vec<String> = all_vars.into_iter().collect();

    let exp: Vec<Row> = exp_rows.iter().map(|r| align_binding(r, &order)).collect();
    let mut act: Vec<Row> = actual_rows
        .iter()
        .map(|r| {
            order
                .iter()
                .map(|v| {
                    actual_vars
                        .iter()
                        .position(|av| av == v)
                        .and_then(|i| r.get(i).cloned().flatten())
                })
                .collect()
        })
        .collect();
    if let Some(f) = filter {
        act.retain(|row| !f.drops(&order, row));
    }

    // Sequence comparison only when the query orders AND the expected encoding
    // preserves order (SRX/SRJ/TSV document order, or rs:index in result-set
    // graphs).
    let compare_ordered = ordered
        && (result_path
            .extension()
            .is_some_and(|e| e == "srx" || e == "srj" || e == "tsv")
            || indexed);
    match rows_equal(&exp, &act, compare_ordered) {
        Ok(true) => Status::Pass,
        Ok(false) => Status::Fail(format!(
            "result mismatch: expected {} solution(s), got {}{}{}",
            exp.len(),
            act.len(),
            if compare_ordered { " (ordered)" } else { "" },
            diff_sample(&order, &exp, &act)
        )),
        Err(e) => Status::Fail(e),
    }
}

/// A compact sample of rows present on only one side (label-sensitive, so it is
/// a debugging hint, not the verdict), to make mismatch reports actionable.
fn diff_sample(order: &[String], exp: &[Row], act: &[Row]) -> String {
    let fmt_row = |r: &Row| -> String {
        let cells: Vec<String> = r
            .iter()
            .enumerate()
            .filter_map(|(i, t)| {
                t.as_ref()
                    .map(|t| format!("?{}={t}", order.get(i).map(String::as_str).unwrap_or("?")))
            })
            .collect();
        format!("{{{}}}", cells.join(" "))
    };
    let keys = |rows: &[Row]| -> Vec<String> { rows.iter().map(fmt_row).collect() };
    let (ek, ak) = (keys(exp), keys(act));
    let only = |a: &[String], b: &[String]| -> Vec<String> {
        let mut pool = b.to_vec();
        a.iter()
            .filter(|k| {
                if let Some(p) = pool.iter().position(|x| x == *k) {
                    pool.remove(p);
                    false
                } else {
                    true
                }
            })
            .take(2)
            .cloned()
            .collect()
    };
    let miss = only(&ek, &ak);
    let extra = only(&ak, &ek);
    let mut s = String::new();
    if !miss.is_empty() {
        s.push_str(&format!("; expected-only e.g. {}", miss.join(", ")));
    }
    if !extra.is_empty() {
        s.push_str(&format!("; actual-only e.g. {}", extra.join(", ")));
    }
    s
}

fn align_binding(binding: &Binding, order: &[String]) -> Row {
    order
        .iter()
        .map(|v| {
            binding
                .iter()
                .find(|(bv, _)| bv == v)
                .map(|(_, t)| t.clone())
        })
        .collect()
}

/// CONSTRUCT evaluation test (T16): the expected result is an RDF *graph* document,
/// compared against the engine's constructed graph as triples-as-rows under the same
/// bnode-bijection machinery the update tests use (graphs are sets — unordered).
fn run_construct_test(
    entry: &TestEntry,
    query_text: &str,
    base: &str,
    graph_data: &[(String, PathBuf)],
    data_override: Option<String>,
) -> Status {
    let Some(result_path) = entry.result_file.clone() else {
        return Status::Skip("no mf:result".into());
    };
    let mut expected: Vec<Row> = match parse_file(&result_path) {
        Ok(triples) => triples
            .into_iter()
            .map(|t| {
                vec![
                    Some(Term::from(t.subject)),
                    Some(Term::NamedNode(t.predicate)),
                    Some(t.object),
                ]
            })
            .collect(),
        Err(e) => return Status::Fail(format!("expected-graph parse error: {e}")),
    };
    dedup_rows(&mut expected); // an RDF graph is a set

    let nquads = match data_override {
        Some(n) => n,
        None => match build_nquads(&entry.action.data, graph_data) {
            Ok(n) => n,
            Err(e) => return Status::Fail(format!("data load error: {e}")),
        },
    };
    let query_with_base = with_base(query_text, base);

    let graph_names: Vec<String> = graph_data.iter().map(|(g, _)| g.clone()).collect();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| {
            let graph = load_dataset_with_graphs(&nquads, &graph_names)?;
            let triples = sparq_engine::construct(&graph, &query_with_base)?;
            let rows: Vec<Row> = triples
                .into_iter()
                .map(|t| {
                    vec![
                        Some(Term::from(t.subject)),
                        Some(Term::NamedNode(t.predicate)),
                        Some(t.object),
                    ]
                })
                .collect();
            Ok::<_, String>(rows)
        })();
        let _ = tx.send(result);
    });
    let actual = match rx.recv_timeout(TEST_TIMEOUT) {
        Ok(Ok(r)) => r, // the engine already dedups (set semantics)
        Ok(Err(e)) => return Status::Fail(format!("engine error: {e}")),
        Err(mpsc::RecvTimeoutError::Timeout) => return Status::Fail("timeout (20s)".into()),
        Err(mpsc::RecvTimeoutError::Disconnected) => return Status::Fail("engine panicked".into()),
    };

    match rows_equal(&expected, &actual, false) {
        Ok(true) => Status::Pass,
        Ok(false) => Status::Fail(format!(
            "graph mismatch: expected {} triple(s), got {}",
            expected.len(),
            actual.len()
        )),
        Err(e) => Status::Fail(e),
    }
}

pub fn run_update_test(entry: &TestEntry) -> Status {
    let Some(request_path) = entry.update_request.clone() else {
        return Status::Fail("manifest entry has no ut:request".into());
    };
    let request_text = match std::fs::read_to_string(&request_path) {
        Ok(t) => t,
        Err(e) => return Status::Fail(format!("read request: {e}")),
    };
    let request = with_base(&request_text, &file_iri(&request_path));

    let nquads = match build_nquads(&entry.update_pre.data, &entry.update_pre.graph_data) {
        Ok(n) => n,
        Err(e) => return Status::Fail(format!("data load error: {e}")),
    };

    // Expected post-state as QUADS: [s, p, o, graph] rows (graph = None for the
    // default graph), so named-graph post-states compare exactly.
    let mut expected: Vec<Row> = Vec::new();
    let mut file_idx = 0usize;
    for d in &entry.update_post.data {
        match parse_file_quads(d) {
            Ok(quads) => {
                for q in quads {
                    let q = prefix_bnodes_quad(q, file_idx);
                    expected.push(vec![
                        Some(Term::from(q.subject)),
                        Some(Term::NamedNode(q.predicate)),
                        Some(q.object),
                        match q.graph_name {
                            GraphName::DefaultGraph => None,
                            GraphName::NamedNode(n) => Some(Term::NamedNode(n)),
                            GraphName::BlankNode(b) => Some(Term::BlankNode(b)),
                        },
                    ]);
                }
            }
            Err(e) => return Status::Fail(format!("expected-data parse error: {e}")),
        }
        file_idx += 1;
    }
    for (g, d) in &entry.update_post.graph_data {
        let graph = match oxrdf::NamedNode::new(g.clone()) {
            Ok(n) => Term::NamedNode(n),
            Err(e) => return Status::Fail(format!("bad expected graph label {g}: {e}")),
        };
        match parse_file(d) {
            Ok(triples) => {
                for t in triples {
                    let t = prefix_bnodes(t, file_idx);
                    expected.push(vec![
                        Some(Term::from(t.subject)),
                        Some(Term::NamedNode(t.predicate)),
                        Some(t.object),
                        Some(graph.clone()),
                    ]);
                }
            }
            Err(e) => return Status::Fail(format!("expected-data parse error: {e}")),
        }
        file_idx += 1;
    }
    dedup_rows(&mut expected);

    let graph_names: Vec<String> = entry
        .update_pre
        .graph_data
        .iter()
        .map(|(g, _)| g.clone())
        .collect();
    // [OPUS-4.8] roborev 1646: `LOAD <file://…>` is refused by default. The conformance suite
    // is a TRUSTED local run, so allowlist the request file's directory (where the W3C update
    // tests keep the data files they LOAD) as the LOAD base.
    let load_base: PathBuf = request_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| {
            let graph = load_dataset_with_graphs(&nquads, &graph_names)?;
            let updated =
                sparq_engine::with_load_base(load_base, || sparq_engine::update(&graph, &request))?;
            // Dump the resulting dataset: default graph + every named graph.
            let mut rows: Vec<Row> = Vec::new();
            let mut dump = |g: &sparq_core::Graph, name: Option<&Term>| {
                let scan = g.store.scan(&[None, None, None]);
                for r in scan.rows.iter() {
                    let [s, p, o] = scan.to_spo(r);
                    rows.push(vec![
                        Some(g.dict.term(s)),
                        Some(g.dict.term(p)),
                        Some(g.dict.term(o)),
                        name.cloned(),
                    ]);
                }
            };
            dump(&updated, None);
            for (name, g) in &updated.named {
                dump(g, Some(name));
            }
            Ok::<_, String>(rows)
        })();
        let _ = tx.send(result);
    });
    let mut actual = match rx.recv_timeout(TEST_TIMEOUT) {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            // Operations the engine refuses (named-graph ops, USING, LOAD…) are
            // explicit "not yet supported" errors — report them as skips.
            return if e.contains("not yet supported") || e.contains("not supported") {
                Status::Skip(format!("engine: {e}"))
            } else {
                Status::Fail(format!("engine error: {e}"))
            };
        }
        Err(mpsc::RecvTimeoutError::Timeout) => return Status::Fail("timeout (20s)".into()),
        Err(mpsc::RecvTimeoutError::Disconnected) => return Status::Fail("engine panicked".into()),
    };
    dedup_rows(&mut actual);

    match rows_equal(&expected, &actual, false) {
        Ok(true) => Status::Pass,
        Ok(false) => {
            let cols: Vec<String> = ["s", "p", "o", "graph"].map(String::from).into();
            Status::Fail(format!(
                "dataset mismatch: expected {} quad(s), got {}{}",
                expected.len(),
                actual.len(),
                diff_sample(&cols, &expected, &actual)
            ))
        }
        Err(e) => Status::Fail(e),
    }
}

/// Syntactic (label-sensitive) dedup — RDF graphs are sets; the bnode
/// bijection in the comparison handles label differences across the two sides.
fn dedup_rows(rows: &mut Vec<Row>) {
    let mut seen = std::collections::HashSet::new();
    rows.retain(|r| {
        let key = r
            .iter()
            .map(|t| t.as_ref().map(|t| t.to_string()).unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\u{1}");
        seen.insert(key)
    });
}
