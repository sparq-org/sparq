//! [OPUS-4.8] sq-ddpgx (epic sq-my8wd) — the W3C SPARQL 1.1 `sparql11/service`
//! EVALUATION conformance lane, driven through the merged in-process loopback
//! harness from sq-ushvx ([`crate::service_loopback::LoopbackEndpoint`], #1291).
//!
//! ## What it does
//!
//! For each `mf:QueryEvaluationTest` of the W3C `sparql11/service` manifest this
//! runner:
//!
//! 1. parses the test's `mf:action` for the query file, the (optional) default-graph
//!    `qt:data` documents, and EACH `qt:serviceData [ qt:endpoint <IRI> ; qt:data <doc> ]`
//!    block (the per-endpoint remote dataset);
//! 2. stands up ONE real [`sparq_server::serve`] endpoint per `qt:serviceData` endpoint
//!    over that block's remote data, on a fresh ephemeral `127.0.0.1:0` loopback port
//!    (the merged harness fixture — serves the RDF, returns the bound URL, RAII teardown);
//! 3. REWRITES every well-known endpoint IRI (e.g. `http://example.org/sparql`) to the
//!    loopback `sparql_url()` of its stood-up endpoint, BOTH in the query text AND in the
//!    local/default-graph data (the `service5` test resolves `SERVICE ?endpoint` from a
//!    `void:sparqlEndpoint` triple in the data, so the rewrite has to reach the data too);
//! 4. runs the rewritten federated query over the local graph through the engine's REAL
//!    `ureq` HTTP transport ([`sparq_engine::query`] under the `service` feature), with the
//!    egress allowlist scoped (for that call only) to the stood-up loopback hosts; and
//! 5. compares the engine's solutions to the suite's `.srx` result oracle under the same
//!    bag/bnode-bijection machinery the SPARQL ratchet uses ([`crate::compare::rows_equal`]).
//!
//! So the WHOLE federated path is exercised end-to-end: HTTP request/response, `Accept`
//! content negotiation, SPARQL-Results parsing, the bind-join over the wire — not a
//! `pub(crate)` canned mock. Per the maintainer-approved mocking the "remote" endpoints
//! are in-process loopback servers serving the suite's own fixture data.
//!
//! ## SILENT vs non-SILENT semantics
//!
//! `service6` / `service7` carry a `SERVICE SILENT <http://invalid.endpoint.org/sparql>`
//! to an endpoint that is never stood up — the suite's oracle is that `SILENT` SWALLOWS
//! the failed call (the join continues with no remote contribution). This runner therefore
//! NEVER stands up an endpoint for an IRI that is not allowlisted, so a `SILENT` clause
//! targeting one really does hit a refusal and must be swallowed. The complementary
//! non-SILENT direction (a CLOSED port must PROPAGATE the error) is pinned by a dedicated
//! unit test in `tests/service_eval_suite.rs` rather than the manifest (the manifest has no
//! non-SILENT-to-a-dead-endpoint case).
//!
//! ## Honest floor
//!
//! The lane reports its results as [`SuiteOutcome`]; the gating test
//! (`tests/service_eval_suite.rs`) asserts the MEASURED `SERVICE_EVAL_FLOOR` pass count
//! (the rest is tracked-not-asserted — no skip-laundering). The floor may only RISE.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use oxrdf::{NamedOrBlankNode, Term};
use spargebra::algebra::GraphPattern;
use spargebra::term::NamedNodePattern;
use spargebra::{Query, SparqlParser};

use sparq_server::{AppState, ServerConfig};

use crate::compare::{rows_equal, Row};
use crate::manifest::{MF, QT};
use crate::rdf::{as_node, file_iri, iri_to_path, parse_file, MiniGraph};
use crate::results::{parse_expected, Binding, Expected};
use crate::service_loopback::LoopbackEndpoint;

/// One `qt:serviceData [ qt:endpoint <IRI> ; qt:data <doc> ]` block of a test.
#[derive(Debug, Clone)]
struct ServiceData {
    /// The well-known endpoint IRI used in the query / data (e.g. `http://example.org/sparql`).
    endpoint: String,
    /// The remote dataset document the loopback endpoint serves.
    data: PathBuf,
}

/// One `mf:QueryEvaluationTest` of the service suite.
#[derive(Debug, Clone)]
struct ServiceTest {
    name: String,
    query: PathBuf,
    /// Default-graph `qt:data` documents (loaded into the LOCAL graph).
    data: Vec<PathBuf>,
    /// Each remote endpoint's `(endpoint IRI, served data)`.
    service_data: Vec<ServiceData>,
    result: PathBuf,
}

/// Per-test verdict.
#[derive(Debug, Clone)]
pub enum Outcome {
    Pass,
    Fail(String),
    Skip(String),
}

/// The result of running the whole service evaluation suite.
#[derive(Debug, Default)]
pub struct SuiteOutcome {
    pub pass: usize,
    pub fail: usize,
    pub skip: usize,
    /// `(test name, why)` for every failing test — surfaced by the runner so a
    /// regression is actionable, never silently counted.
    pub failures: Vec<(String, String)>,
}

/// Parses the `sparql11/service` manifest into the per-test descriptors.
fn parse_manifest(manifest: &Path) -> Result<Vec<ServiceTest>, String> {
    let g = MiniGraph::load(manifest)?;
    let dir = manifest
        .parent()
        .ok_or_else(|| format!("{}: no parent dir", manifest.display()))?;
    // Resolve a relative manifest IRI (e.g. <service01.rq>) to a file next to it.
    let resolve = |t: &Term| -> Option<PathBuf> {
        match t {
            Term::NamedNode(n) => {
                // The manifest base is the manifest file, so relative names are
                // already absolute file:// IRIs after parse; fall back to joining
                // the bare last path segment onto the manifest directory.
                iri_to_path(n.as_str())
                    .or_else(|| n.as_str().rsplit('/').next().map(|seg| dir.join(seg)))
            }
            _ => None,
        }
    };

    let mut tests = Vec::new();
    for m in g.subjects_with_type(&format!("{MF}Manifest")) {
        let Some(entries) = g.object(&m, &format!("{MF}entries")) else {
            continue;
        };
        for item in g.list(entries) {
            let Some(node) = as_node(&item) else { continue };
            // Only evaluation tests; the suite has no other kinds, but be defensive.
            let is_eval = g
                .types_of(&node)
                .iter()
                .any(|t| t == &format!("{MF}QueryEvaluationTest"));
            if !is_eval {
                continue;
            }
            let name = g
                .str_object(&node, &format!("{MF}name"))
                .unwrap_or_else(|| match &node {
                    NamedOrBlankNode::NamedNode(n) => n.as_str().to_string(),
                    NamedOrBlankNode::BlankNode(b) => format!("_:{}", b.as_str()),
                });
            let Some(action_t) = g.object(&node, &format!("{MF}action")) else {
                continue;
            };
            let Some(action) = as_node(action_t) else {
                continue;
            };
            let Some(query) = g.object(&action, &format!("{QT}query")).and_then(resolve) else {
                continue;
            };
            let data: Vec<PathBuf> = g
                .objects(&action, &format!("{QT}data"))
                .into_iter()
                .filter_map(resolve)
                .collect();
            let mut service_data = Vec::new();
            for sd in g.objects(&action, &format!("{QT}serviceData")) {
                let Some(sd) = as_node(sd) else { continue };
                let endpoint = match g.object(&sd, &format!("{QT}endpoint")) {
                    Some(Term::NamedNode(n)) => n.as_str().to_string(),
                    _ => continue,
                };
                let Some(ddoc) = g.object(&sd, &format!("{QT}data")).and_then(resolve) else {
                    continue;
                };
                service_data.push(ServiceData {
                    endpoint,
                    data: ddoc,
                });
            }
            let Some(result) = g.object(&node, &format!("{MF}result")).and_then(resolve) else {
                continue;
            };
            tests.push(ServiceTest {
                name,
                query,
                data,
                service_data,
                result,
            });
        }
    }
    Ok(tests)
}

/// Loads `doc` and serves it on a fresh ephemeral loopback endpoint.
fn stand_up(doc: &Path) -> Result<LoopbackEndpoint, String> {
    let triples = parse_file(doc)?;
    // Re-serialize to N-Triples so the in-memory `Graph` loads exactly the remote
    // dataset (the loopback server answers SPARQL over it).
    let mut nt = String::new();
    for t in &triples {
        nt.push_str(&format!("{} {} {} .\n", t.subject, t.predicate, t.object));
    }
    let graph = sparq_core::Graph::load_str(&nt, "ntriples")
        .map_err(|e| format!("load remote dataset {}: {e}", doc.display()))?;
    Ok(LoopbackEndpoint::serve(graph))
}

/// [SONNET-4.6] sq-my8wd.1 — Like [`stand_up`] but configures the loopback server
/// with a custom service egress allowlist so it can federate onward to other loopback
/// endpoints. Used for service3-style tests where a top-level loopback endpoint hosts
/// a body that contains nested non-SILENT `SERVICE` calls to a second loopback endpoint.
///
/// `allowed_entries` are `"host:port"` strings (e.g. `"127.0.0.1:54321"`) added via
/// [`sparq_server::ServiceAllowlist::add`]. The server's per-request egress mode is
/// `AllowlistOnly` — the same strict mode the stock server installs — so only the
/// explicitly listed inner endpoints are reachable; all other destinations stay refused.
fn stand_up_with_egress(
    doc: &Path,
    allowed_entries: &[String],
) -> Result<LoopbackEndpoint, String> {
    let triples = parse_file(doc)?;
    let mut nt = String::new();
    for t in &triples {
        nt.push_str(&format!("{} {} {} .\n", t.subject, t.predicate, t.object));
    }
    let graph = sparq_core::Graph::load_str(&nt, "ntriples")
        .map_err(|e| format!("load remote dataset {}: {e}", doc.display()))?;
    let mut config = ServerConfig::default();
    for entry in allowed_entries {
        config
            .service_allow
            .add(entry)
            .map_err(|e| format!("allowlist entry \"{}\": {}", entry, e))?;
    }
    Ok(LoopbackEndpoint::serve_with(move || {
        AppState::with_config(graph, config)
    }))
}

/// Rewrites every well-known endpoint IRI to its loopback `sparql_url()` in `text`.
/// Endpoints are replaced longest-IRI-first so a prefix IRI never clobbers a longer one.
fn rewrite_endpoints(text: &str, map: &BTreeMap<String, String>) -> String {
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort_by_key(|k| std::cmp::Reverse(k.len()));
    let mut out = text.to_string();
    for k in keys {
        out = out.replace(k.as_str(), &map[k]);
    }
    out
}

/// Builds the LOCAL (default-graph) N-Triples document from the test's `qt:data`,
/// rewriting any well-known endpoint IRI it carries (the `service5` `void:sparqlEndpoint`
/// triples) to the loopback URL so `SERVICE ?endpoint` dials the loopback server.
fn build_local_nt(data: &[PathBuf], map: &BTreeMap<String, String>) -> Result<String, String> {
    let mut out = String::new();
    for d in data {
        for t in parse_file(d)? {
            let line = format!("{} {} {} .\n", t.subject, t.predicate, t.object);
            out.push_str(&rewrite_endpoints(&line, map));
        }
    }
    Ok(out)
}

/// Aligns an expected binding onto a shared variable order.
fn align(binding: &Binding, order: &[String]) -> Row {
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

/// Walks a graph pattern, invoking `f` on every nested pattern (pre-order).
fn walk(p: &GraphPattern, f: &mut impl FnMut(&GraphPattern)) {
    f(p);
    match p {
        GraphPattern::Join { left, right }
        | GraphPattern::LeftJoin { left, right, .. }
        | GraphPattern::Union { left, right }
        | GraphPattern::Lateral { left, right }
        | GraphPattern::Minus { left, right } => {
            walk(left, f);
            walk(right, f);
        }
        GraphPattern::Filter { inner, .. }
        | GraphPattern::Extend { inner, .. }
        | GraphPattern::OrderBy { inner, .. }
        | GraphPattern::Project { inner, .. }
        | GraphPattern::Distinct { inner }
        | GraphPattern::Reduced { inner }
        | GraphPattern::Slice { inner, .. }
        | GraphPattern::Group { inner, .. }
        | GraphPattern::Service { inner, .. }
        | GraphPattern::Graph { inner, .. } => walk(inner, f),
        // Bgp / Path / Values / table-shaped leaves have no nested patterns.
        _ => {}
    }
}

/// [SONNET-4.6] sq-my8wd.1 — Walks the query pattern and returns a map from each
/// outer non-SILENT named `SERVICE` endpoint IRI to the list of non-SILENT named
/// `SERVICE` endpoint IRIs nested within its body. Used in [`run_test`] to determine
/// endpoint startup order and to configure each outer endpoint's egress allowlist so it
/// can reach its inner neighbours (e.g. the `service3` test: `SERVICE <ep1>` whose body
/// contains `OPTIONAL { SERVICE <ep2> { … } }`).
///
/// A nested **SILENT** `SERVICE` is excluded from the map: the inner server's strict
/// empty-allowlist refuses it, and `SILENT` swallows the refusal — no allowlist entry
/// is needed. A **variable** endpoint (`SERVICE ?var`) is not matched by this function
/// (it cannot appear in the map key); `out_of_scope_reason` handles that case before
/// `find_service_nesting` is called.
fn find_service_nesting(pattern: &GraphPattern) -> BTreeMap<String, Vec<String>> {
    let mut result: BTreeMap<String, Vec<String>> = BTreeMap::new();
    walk(pattern, &mut |p| {
        // Only consider non-SILENT SERVICE nodes with a named (IRI) endpoint.
        if let GraphPattern::Service {
            name: NamedNodePattern::NamedNode(outer_name),
            inner,
            ..
        } = p
        {
            if !matches!(p, GraphPattern::Service { silent: true, .. }) {
                let outer_iri = outer_name.as_str().to_string();
                let mut nested = Vec::new();
                walk(inner, &mut |q| {
                    if let GraphPattern::Service {
                        name: NamedNodePattern::NamedNode(inner_name),
                        ..
                    } = q
                    {
                        if !matches!(q, GraphPattern::Service { silent: true, .. }) {
                            nested.push(inner_name.as_str().to_string());
                        }
                    }
                });
                if !nested.is_empty() {
                    result.insert(outer_iri, nested);
                }
            }
        }
    });
    result
}

/// Classifies a query's SERVICE shape against what THIS in-process loopback
/// fixture can serve, returning a documented `Skip` reason for the known
/// out-of-scope shape (an honest tracked-not-asserted divergence — never
/// skip-laundered into the pass count), or `None` when the test is in scope.
///
/// * A **variable** SERVICE endpoint (`SERVICE ?var { … }`, the `service5` test)
///   is not supported by the engine's federation evaluator (it requires resolving
///   the endpoint from a binding before the remote call); reported as an explicit
///   engine "not supported" error, classified here BEFORE the run so it is a Skip,
///   not a Fail.
///
/// A **nested non-SILENT** SERVICE (e.g. the `service3` test, where `SERVICE <ep1>`
/// contains `OPTIONAL { SERVICE <ep2> { … } }`) is now IN scope: [`run_test`] detects
/// this topology via [`find_service_nesting`] and configures the outer loopback
/// endpoint's egress allowlist to include the inner endpoint, so ep1 can reach ep2.
/// A nested **SILENT** SERVICE (`service6`, `service7`) has always been in scope — the
/// inner server's strict empty-allowlist refuses it and `SILENT` swallows the refusal.
fn out_of_scope_reason(pattern: &GraphPattern) -> Option<String> {
    let mut variable_endpoint = false;
    walk(pattern, &mut |p| {
        if let GraphPattern::Service { name, .. } = p {
            if matches!(name, NamedNodePattern::Variable(_)) {
                variable_endpoint = true;
            }
        }
    });
    if variable_endpoint {
        Some(
            "variable SERVICE endpoint (`SERVICE ?var`) — engine federation evaluator does not \
             resolve a bound endpoint (tracked-not-asserted, sq-my8wd child)"
                .into(),
        )
    } else {
        None
    }
}

/// Runs ONE service evaluation test end-to-end through the loopback harness.
fn run_test(test: &ServiceTest) -> Outcome {
    let expected = match parse_expected(&test.result) {
        Ok(e) => e,
        Err(e) => return Outcome::Fail(format!("expected-result parse error: {e}")),
    };
    let Expected::Bindings {
        vars: exp_vars,
        rows: exp_rows,
        ..
    } = expected
    else {
        return Outcome::Fail("service tests are all SELECT, expected a binding set".into());
    };

    let query_text = match std::fs::read_to_string(&test.query) {
        Ok(t) => t,
        Err(e) => return Outcome::Fail(format!("read query: {e}")),
    };
    let base = file_iri(&test.query);

    // Classify the query's SERVICE shape up front: a variable endpoint becomes a
    // documented Skip (tracked-not-asserted, never skip-laundered). We also keep the
    // parsed pattern for nested-SERVICE topology analysis below.
    let parser = match SparqlParser::new().with_base_iri(&base) {
        Ok(p) => p,
        Err(e) => return Outcome::Fail(format!("bad base IRI: {e}")),
    };
    let pattern_opt = match parser.parse_query(&query_text) {
        Ok(Query::Select { pattern, .. }) => {
            if let Some(why) = out_of_scope_reason(&pattern) {
                return Outcome::Skip(why);
            }
            Some(pattern)
        }
        Ok(_) => None, // service suite is all SELECT; let the run path report any surprise
        Err(e) => return Outcome::Fail(format!("query parse error: {e}")),
    };

    // [SONNET-4.6] sq-my8wd.1 — Analyse nested SERVICE topology so inner endpoints
    // (those nested in another SERVICE's body) are started BEFORE the outer endpoints
    // that need to reach them, and the outer endpoints are configured with an egress
    // allowlist that permits onward federation to the already-bound inner endpoints.
    // For queries with no nesting (all currently passing tests), `service_deps` is
    // empty and the loop below degenerates to the original single-pass stand_up().
    let service_deps: BTreeMap<String, Vec<String>> = pattern_opt
        .as_ref()
        .map(find_service_nesting)
        .unwrap_or_default();
    let all_inner_iris: std::collections::BTreeSet<&str> = service_deps
        .values()
        .flat_map(|v| v.iter().map(|s| s.as_str()))
        .collect();

    // `started` holds inner endpoints (keyed by original IRI) until they are moved
    // into `endpoints` at the end; both Vecs are dropped after the query returns.
    let mut started: BTreeMap<String, LoopbackEndpoint> = BTreeMap::new();
    let mut endpoints: Vec<LoopbackEndpoint> = Vec::new();
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    let mut allow_hosts: Vec<String> = Vec::new();

    // Phase 1: start inner endpoints first so their loopback addresses are known
    // before the outer endpoint's allowlist is built.
    for sd in &test.service_data {
        if !all_inner_iris.contains(sd.endpoint.as_str()) {
            continue;
        }
        let ep = match stand_up(&sd.data) {
            Ok(ep) => ep,
            Err(e) => return Outcome::Fail(e),
        };
        map.insert(sd.endpoint.clone(), ep.sparql_url());
        allow_hosts.push(ep.host());
        started.insert(sd.endpoint.clone(), ep);
    }

    // Phase 2: start outer or plain endpoints. Outer endpoints (those with nested deps)
    // receive a custom egress allowlist containing `host:port` entries for each
    // already-started inner endpoint, so their per-request `AllowlistOnly` policy
    // permits calling those inner endpoints. Plain endpoints use the stock config.
    for sd in &test.service_data {
        if all_inner_iris.contains(sd.endpoint.as_str()) {
            continue; // already started in phase 1
        }
        let ep = if let Some(inner_iris) = service_deps.get(&sd.endpoint) {
            // Build `host:port` allowlist entries from the already-started inner endpoints.
            let allowed: Vec<String> = inner_iris
                .iter()
                .filter_map(|iri| started.get(iri.as_str()))
                .map(|ep| format!("{}", ep.addr()))
                .collect();
            match stand_up_with_egress(&sd.data, &allowed) {
                Ok(ep) => ep,
                Err(e) => return Outcome::Fail(e),
            }
        } else {
            match stand_up(&sd.data) {
                Ok(ep) => ep,
                Err(e) => return Outcome::Fail(e),
            }
        };
        map.insert(sd.endpoint.clone(), ep.sparql_url());
        allow_hosts.push(ep.host());
        endpoints.push(ep);
    }
    // Keep inner endpoints alive for the full query lifetime (RAII teardown on return).
    endpoints.extend(started.into_values());

    // Rewrite the endpoint IRIs in the query so SERVICE dials the loopback ports.
    let query_rewritten = rewrite_endpoints(&query_text, &map);
    // Prepend a BASE so any relative IRIs in the query resolve to the query file.
    let query_with_base = format!("BASE <{base}>\n{query_rewritten}");

    let local_nt = match build_local_nt(&test.data, &map) {
        Ok(n) => n,
        Err(e) => return Outcome::Fail(format!("local data load error: {e}")),
    };

    // Drive the federated query through the engine's REAL ureq transport, with the
    // egress allowlist scoped to exactly the loopback hosts we stood up (so the
    // SILENT-to-an-unallowlisted-endpoint cases really hit a refusal). The filter is
    // NOT globally disabled.
    let result = sparq_engine::with_service_egress_allow(allow_hosts, || {
        let graph = sparq_core::Graph::load_str(&local_nt, "ntriples")
            .map_err(|e| format!("load local graph: {e}"))?;
        sparq_engine::query(&graph, &query_with_base)
    });
    let res = match result {
        Ok(r) => r,
        Err(e) => return Outcome::Fail(format!("engine error: {e}")),
    };

    let actual_vars: Vec<String> = res.vars.iter().map(|v| v.as_str().to_string()).collect();
    // Variable sets must agree (the suite's .srx declares them).
    if !exp_vars.is_empty() {
        use std::collections::BTreeSet;
        let exp_set: BTreeSet<&str> = exp_vars.iter().map(|s| s.as_str()).collect();
        let act_set: BTreeSet<&str> = actual_vars.iter().map(|s| s.as_str()).collect();
        if exp_set != act_set {
            return Outcome::Fail(format!(
                "variables mismatch: expected {{{}}}, got {{{}}}",
                exp_vars.join(", "),
                actual_vars.join(", ")
            ));
        }
    }

    // Align both sides on a shared variable order and compare under bag semantics
    // (the service queries carry no ORDER BY).
    let mut all: std::collections::BTreeSet<String> = actual_vars.iter().cloned().collect();
    all.extend(exp_vars.iter().cloned());
    for r in &exp_rows {
        all.extend(r.iter().map(|(v, _)| v.clone()));
    }
    let order: Vec<String> = all.into_iter().collect();
    let exp: Vec<Row> = exp_rows.iter().map(|r| align(r, &order)).collect();
    let act: Vec<Row> = res
        .rows
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

    match rows_equal(&exp, &act, false) {
        Ok(true) => Outcome::Pass,
        Ok(false) => Outcome::Fail(format!(
            "result mismatch: expected {} solution(s), got {}",
            exp.len(),
            act.len()
        )),
        Err(e) => Outcome::Fail(e),
    }
}

/// Walks the `sparql11/service` manifest and runs every evaluation test through
/// the loopback harness, returning the aggregate outcome. `root` is the rdf-tests
/// clone root (the same one the SPARQL harness uses).
pub fn run_service_suite(root: &Path) -> Result<SuiteOutcome, String> {
    let manifest = root.join("sparql/sparql11/service/manifest.ttl");
    let tests = parse_manifest(&manifest)?;
    let mut out = SuiteOutcome::default();
    for test in &tests {
        match run_test(test) {
            Outcome::Pass => out.pass += 1,
            Outcome::Skip(_) => out.skip += 1,
            Outcome::Fail(why) => {
                out.fail += 1;
                out.failures.push((test.name.clone(), why));
            }
        }
    }
    Ok(out)
}

/// Stands up a single loopback endpoint over `nt_data` (N-Triples) and runs
/// `query_template` against an empty local graph, with `{EP}` in the template
/// replaced by the endpoint's `sparql_url()`. Used by the SILENT-semantics tests
/// to exercise a LIVE endpoint; the CLOSED-port direction does not stand one up.
pub fn run_against_live_endpoint(nt_data: &str, query_template: &str) -> Result<usize, String> {
    let graph = sparq_core::Graph::load_str(nt_data, "ntriples")
        .map_err(|e| format!("load remote dataset: {e}"))?;
    let ep = LoopbackEndpoint::serve(graph);
    let query = query_template.replace("{EP}", &ep.sparql_url());
    let res = sparq_engine::with_service_egress_allow([ep.host()], || {
        let local = sparq_core::Graph::load_str("", "ntriples")
            .map_err(|e| format!("load empty local graph: {e}"))?;
        sparq_engine::query(&local, &query)
    })?;
    Ok(res.rows.len())
}
