//! sparq-py: the `sparq` Python package (pyo3 + maturin).
//!
//! A thin, allocation-conscious wrapper over the workspace's public APIs:
//!
//! * `Graph` wraps `sparq_core::Graph` (load / save / open / len). `Graph.copy`
//!   returns a cheap, logically-independent copy over the core's structural
//!   snapshot (Arc-shared immutable base) — O(pending delta), not O(triples).
//! * `Graph.query` / `Graph.query_json` / `Graph.ask` / `Graph.construct` /
//!   `Graph.describe` call `sparq_engine` (all four query forms are native:
//!   ASK early-exits, CONSTRUCT/DESCRIBE return term triples).
//! * `Graph.update` applies SPARQL Update; the engine returns a NEW graph
//!   (immutable store, rebuild semantics), which the wrapper swaps in place so
//!   Python sees an in-place mutation. Named graphs survive every operation
//!   (GRAPH-scoped data ops, graph templates, USING, CLEAR/DROP/ADD/COPY/MOVE).
//! * `Graph.reason` materializes the RDFS / OWL-RL closure via `sparq_reason`
//!   (in place, same swap; named graphs are carried across the rebuild);
//!   `Graph.inconsistencies` reports OWL 2 RL clashes; `Graph.load_n3` runs the
//!   Notation3 forward-chainer over an N3 document (facts + `{…} => {…}` rules)
//!   and `Graph.reason_n3_with(rules)` runs caller-supplied N3 rules over an
//!   already-loaded graph.
//! * Full-text search (`sparq_text`): `Graph.text_search` returns BM25-ranked
//!   literal hits and `Graph.query_text` runs SELECTs with the `text:` magic
//!   predicates. The [`TextIndex`] lifecycle lives HERE (the follow-up tracked
//!   in beads — `bd list -l area:sparq-py`): built lazily on first use and cached on the graph, and
//!   invalidated by every mutating swap (`update` / `reason` /
//!   `reason_n3_with`) — those paths rebuild the graph (dictionary ids may
//!   change), so the native incremental `TextIndex::apply_delta` mirror has no
//!   corresponding wrapper path; lazy rebuild is the policy that matches.
//!
//! Long-running engine calls release the GIL (`py.detach`) so other Python
//! threads keep running during parse / query / reasoning.
//!
//! * `Graph.query_arrow` (opt-in `arrow` feature / PyPI `arrow` extra) runs a SELECT
//!   and hands the result to `pyarrow` as a [`pyarrow.Table`][arrow] over the merged
//!   `sparq-arrow` columnar export — one struct column per variable, bridged through
//!   the Arrow C Data Interface (no re-serialisation). OFF by default, so the lean
//!   wheel carries no Arrow code. See the `arrow_export` module.
//!
//! [arrow]: https://arrow.apache.org/docs/python/generated/pyarrow.Table.html
// [OPUS-4.8] sq-emay: the crate has zero `unsafe` EXCEPT the opt-in `arrow` C Data
// Interface bridge (`arrow_export`), which needs one tightly-scoped `unsafe` block to
// hand pyarrow a `PyCapsule` over an `FFI_ArrowArrayStream` pointer (the standard Arrow
// producer pattern — see that module). So `forbid` holds for the whole default crate and
// is relaxed to `deny` only when the `arrow` feature compiles that one module in, which
// carries its own `#[allow(unsafe_code)]` with a per-block SAFETY justification.
#![cfg_attr(not(feature = "arrow"), forbid(unsafe_code))]
#![cfg_attr(feature = "arrow", deny(unsafe_code))]

use pyo3::exceptions::{PyIOError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyString};
use sparq_core::dict::{Dict, Id};
use sparq_core::Graph as CoreGraph;
use sparq_text::TextIndex;
use spargebra::Query;

#[cfg(feature = "arrow")]
mod arrow_export;
mod term_render;

/// Map an engine `Err(String)` (parse/eval failure) to a Python `ValueError`.
fn engine_err(e: String) -> PyErr {
    PyValueError::new_err(e)
}

// ---------------------------------------------------------------------------
// Term
// ---------------------------------------------------------------------------

/// One RDF term in a query solution.
///
/// `kind` follows the SPARQL 1.1 JSON results convention (the same strings
/// `Graph.query_json` emits): `"uri"`, `"literal"`, or `"bnode"`.
/// `value` is the IRI / lexical form / blank-node label. For literals,
/// `language` is the language tag (if any) and `datatype` the datatype IRI
/// (always set — plain literals carry `xsd:string`, language-tagged ones
/// `rdf:langString`, directional ones `rdf:dirLangString`). `direction` is the
/// RDF 1.2 base direction (`"ltr"` / `"rtl"`) of a directional language string,
/// else `None` — it is the SPARQL 1.2 `its:dir` value `Graph.query_json` emits.
/// Non-literals have `language = datatype = direction = None`.
// [OPUS-4.8] sq-bj7o: `direction` added so a directional language string
// (`"hi"@en--ltr`) keeps its base direction through `query` — previously the
// Term path dropped it while `query_json` kept it, so the two diverged.
// `skip_from_py_object`: pyo3 0.29 makes the auto `FromPyObject` for Clone
// pyclasses opt-in; nothing here extracts a `Term` from Python, so skip it.
#[pyclass(frozen, eq, hash, skip_from_py_object, module = "sparq")]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Term {
    #[pyo3(get)]
    kind: String,
    #[pyo3(get)]
    value: String,
    #[pyo3(get)]
    language: Option<String>,
    #[pyo3(get)]
    datatype: Option<String>,
    /// RDF 1.2 base direction (`"ltr"` / `"rtl"`) of a directional language
    /// string; `None` for every other term. [OPUS-4.8] sq-bj7o
    #[pyo3(get)]
    direction: Option<String>,
}

impl Term {
    fn from_oxrdf(t: &oxrdf::Term) -> Term {
        match t {
            oxrdf::Term::NamedNode(n) => Term {
                kind: "uri".into(),
                value: n.as_str().to_string(),
                language: None,
                datatype: None,
                direction: None,
            },
            oxrdf::Term::BlankNode(b) => Term {
                kind: "bnode".into(),
                value: b.as_str().to_string(),
                language: None,
                datatype: None,
                direction: None,
            },
            oxrdf::Term::Literal(l) => Term {
                kind: "literal".into(),
                value: l.value().to_string(),
                language: l.language().map(str::to_string),
                datatype: Some(l.datatype().as_str().to_string()),
                // [OPUS-4.8] sq-bj7o: preserve the RDF 1.2 base direction (the
                // SPARQL 1.2 `its:dir` value) so a directional language string
                // round-trips through `query` at parity with `query_json`.
                direction: l.direction().map(|d| d.to_string()),
            },
            // RDF 1.2 triple term: keep the N-Triples rendering as the value.
            oxrdf::Term::Triple(t) => Term {
                kind: "triple".into(),
                value: t.to_string(),
                language: None,
                datatype: None,
                direction: None,
            },
        }
    }

    /// N-Triples-style rendering (shared by `__repr__`).
    fn n3(&self) -> String {
        term_render::render_n3(
            &self.kind,
            &self.value,
            self.language.as_deref(),
            self.datatype.as_deref(),
            self.direction.as_deref(),
        )
    }
}

#[pymethods]
impl Term {
    fn __repr__(&self) -> String {
        format!("Term({})", self.n3())
    }

    /// The bare value (IRI / lexical form / label) — handy in f-strings.
    fn __str__(&self) -> String {
        self.value.clone()
    }
}

// ---------------------------------------------------------------------------
// QueryResult
// ---------------------------------------------------------------------------

/// A materialised SELECT result: `.vars` (projection order) and `.rows`
/// (a list of `{var: Term}` dicts; unbound variables are simply absent).
/// Supports `len(result)`, indexing, and iteration (via the sequence protocol).
#[pyclass(frozen, module = "sparq")]
struct QueryResult {
    #[pyo3(get)]
    vars: Vec<String>,
    rows: Py<PyList>,
}

#[pymethods]
impl QueryResult {
    #[getter]
    fn rows(&self, py: Python<'_>) -> Py<PyList> {
        self.rows.clone_ref(py)
    }

    fn __len__(&self, py: Python<'_>) -> usize {
        self.rows.bind(py).len()
    }

    /// Delegates to the underlying list, so negative indices and slices work,
    /// and Python's sequence-protocol fallback makes the result iterable.
    fn __getitem__<'py>(&self, py: Python<'py>, index: Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        self.rows.bind(py).as_any().get_item(index)
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        format!("QueryResult(vars={:?}, rows={})", self.vars, self.rows.bind(py).len())
    }
}

// ---------------------------------------------------------------------------
// Graph
// ---------------------------------------------------------------------------

/// Resolve `(text, format)` for `Graph.load`: `os.PathLike` is always a file;
/// a `str` is a file iff it names an existing file (no newline in it), else it
/// is parsed as RDF content. The format defaults from the file extension
/// (`.ttl` / `.nt` / `.nq` / `.trig` / `.jsonld`), falling back to `"turtle"`.
///
/// [OPUS-4.8] sq-oy1f.20: the `.jsonld` extension maps to `"jsonld"` unconditionally
/// (it is not behind a `#[cfg]`), so the mapping is feature-agnostic. With the default
/// `jsonld` feature ON, `sparq-core` parses it; under `--no-default-features` the same
/// `"jsonld"` string reaches `sparq-core`, which rejects the unknown format — a clean
/// fail-closed error rather than feeding JSON to the Turtle parser.
fn resolve_source(source: &Bound<'_, PyAny>, format: Option<&str>) -> PyResult<(String, String)> {
    let from_file = |path: &std::path::Path, format: Option<&str>| -> PyResult<(String, String)> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| PyIOError::new_err(format!("cannot read {}: {e}", path.display())))?;
        let fmt = match format {
            Some(f) => f.to_string(),
            None => match path.extension().and_then(|e| e.to_str()) {
                Some("nt") => "ntriples".into(),
                Some("nq") => "nquads".into(),
                Some("trig") => "trig".into(),
                Some("jsonld") => "jsonld".into(),
                _ => "turtle".into(),
            },
        };
        Ok((text, fmt))
    };

    if source.is_instance_of::<PyString>() {
        let s: String = source.extract()?;
        // A multi-line string is never a path; a single-line one is a path only
        // if that file actually exists — otherwise it is (single-line) RDF data.
        if !s.contains('\n') && std::path::Path::new(&s).is_file() {
            return from_file(std::path::Path::new(&s), format);
        }
        Ok((s, format.unwrap_or("turtle").to_string()))
    } else {
        // pathlib.Path / any os.PathLike.
        let path: std::path::PathBuf = source.extract().map_err(|_| {
            PyValueError::new_err("Graph.load expects a str (RDF data or file path) or an os.PathLike")
        })?;
        from_file(&path, format)
    }
}

/// All triples of the (default) graph as canonical `[s, p, o]` id rows.
fn all_triples(g: &CoreGraph) -> Vec<[Id; 3]> {
    let scan = g.store.scan(&[None, None, None]);
    scan.rows.iter().map(|r| scan.to_spo(r)).collect()
}

/// An engine `oxrdf::Triple` as a Python `(subject, predicate, object)` tuple
/// of [`Term`]s (the same term convention `Graph.query` rows use).
fn triple_terms(t: &oxrdf::Triple) -> (Term, Term, Term) {
    (
        Term::from_oxrdf(&oxrdf::Term::from(t.subject.clone())),
        Term::from_oxrdf(&oxrdf::Term::from(t.predicate.clone())),
        Term::from_oxrdf(&t.object),
    )
}

/// An engine SELECT result materialised as a Python [`QueryResult`]
/// (shared by `Graph.query` and `Graph.query_text`).
fn select_result(py: Python<'_>, res: &sparq_engine::QueryResult) -> PyResult<QueryResult> {
    let vars: Vec<String> = res.vars.iter().map(|v| v.as_str().to_string()).collect();
    let rows = PyList::empty(py);
    for row in &res.rows {
        let d = PyDict::new(py);
        for (var, cell) in vars.iter().zip(row) {
            if let Some(t) = cell {
                d.set_item(var, Term::from_oxrdf(t))?;
            }
        }
        rows.append(d)?;
    }
    Ok(QueryResult { vars, rows: rows.unbind() })
}

/// An immutable, dictionary-encoded RDF graph with SPARQL query / update and
/// opt-in reasoning. Build one with `Graph.load(...)`, `Graph.load_n3(...)`,
/// or `Graph.open(...)`.
#[pyclass(module = "sparq")]
struct Graph {
    inner: CoreGraph,
    /// Lazily built BM25 full-text index over the default graph's string
    /// literals (`sparq_text::TextIndex`), cached for `text_search` /
    /// `query_text`. INVARIANT: any method that swaps `inner` for a rebuilt
    /// graph (`update`, `reason`, `reason_n3_with`) resets this to `None` —
    /// a rebuild may re-encode the dictionary, and the index's document ids
    /// are dictionary term ids.
    text: Option<TextIndex>,
}

impl Graph {
    /// Builds (and caches) the full-text index if absent — the lazy half of
    /// the lifecycle. The dictionary scan releases the GIL.
    fn ensure_text_index(&mut self, py: Python<'_>) {
        if self.text.is_none() {
            let inner = &self.inner;
            self.text = Some(py.detach(|| TextIndex::build(inner)));
        }
    }
}

#[pymethods]
impl Graph {
    /// Load RDF from a string of data or from a file path.
    ///
    /// `source`: RDF content (e.g. a Turtle document), a path to an RDF file
    /// (`str` or `os.PathLike`), and `format` one of `"turtle"`, `"ntriples"`,
    /// `"nquads"`, `"trig"`, `"jsonld"` (default: from the file extension, else
    /// `"turtle"`). N-Quads / TriG / JSON-LD named graphs are preserved and
    /// queryable via `GRAPH`. JSON-LD support is on by default; a wheel built with
    /// `--no-default-features` drops it and `format="jsonld"` then errors.
    #[staticmethod]
    #[pyo3(signature = (source, format=None))]
    fn load(py: Python<'_>, source: &Bound<'_, PyAny>, format: Option<&str>) -> PyResult<Graph> {
        let (text, fmt) = resolve_source(source, format)?;
        let inner = py
            .detach(|| CoreGraph::load_dataset(&text, &fmt))
            .map_err(engine_err)?;
        Ok(Graph { inner, text: None })
    }

    /// Parse a Notation3 document (facts + `{ premise } => { conclusion }` rules)
    /// and forward-chain the rules to fixpoint; the resulting graph contains the
    /// full ground closure. To apply N3 rules to an already-loaded graph instead,
    /// use `Graph.reason_n3_with(rules)`.
    #[staticmethod]
    fn load_n3(py: Python<'_>, text: &str) -> PyResult<Graph> {
        let inner = py
            .detach(|| {
                let mut dict = Dict::new();
                let triples = sparq_reason::reason_n3(&mut dict, text)?;
                Ok::<_, String>(CoreGraph::from_parts(dict, triples))
            })
            .map_err(engine_err)?;
        Ok(Graph { inner, text: None })
    }

    /// Open a graph previously persisted with `save(dir)` — the permutation
    /// indexes are memory-mapped (paged in on demand), so opening is near-instant
    /// even for datasets larger than RAM.
    #[staticmethod]
    fn open(py: Python<'_>, dir: std::path::PathBuf) -> PyResult<Graph> {
        let inner = py
            .detach(|| CoreGraph::open(&dir))
            .map_err(|e| PyIOError::new_err(format!("cannot open {}: {e}", dir.display())))?;
        Ok(Graph { inner, text: None })
    }

    /// Persist the graph (indexes + dictionary) into `dir` for later `Graph.open`.
    fn save(&self, py: Python<'_>, dir: std::path::PathBuf) -> PyResult<()> {
        py.detach(|| self.inner.save(&dir))
            .map_err(|e| PyIOError::new_err(format!("cannot save to {}: {e}", dir.display())))
    }

    /// Run a SPARQL SELECT, materialising the solutions as a `QueryResult`
    /// (`.vars` + `.rows`, where each row is a `{var: Term}` dict).
    fn query(&self, py: Python<'_>, sparql: &str) -> PyResult<QueryResult> {
        let res = py
            .detach(|| sparq_engine::query(&self.inner, sparql))
            .map_err(engine_err)?;
        select_result(py, &res)
    }

    /// Run a SPARQL SELECT and return the SPARQL 1.1 JSON results document as a
    /// `str` — the fast path: rows are serialised straight from the dictionary,
    /// skipping per-cell term materialisation.
    fn query_json(&self, py: Python<'_>, sparql: &str) -> PyResult<String> {
        py.detach(|| sparq_engine::query_json(&self.inner, sparql))
            .map_err(engine_err)
    }

    /// Run a SPARQL SELECT and return the solutions as a `pyarrow.Table`
    /// (opt-in `arrow` feature / PyPI `arrow` extra; `pip install sparq-rdf[arrow]`).
    ///
    /// The table has one column per SELECT variable, each an Arrow
    /// `struct<kind, value, datatype, language, direction>` of nullable strings —
    /// the faithful, round-trippable RDF-term projection from the `sparq-arrow`
    /// crate (an **unbound** binding is a null struct slot, distinct from a bound
    /// empty-string literal). The RecordBatch is handed to pyarrow through the Arrow
    /// C Data Interface (a `PyCapsule`), so the columns are NOT re-serialised.
    ///
    /// Same v1 boundary as `sparq-arrow`: no numeric narrowing yet (`42^^xsd:integer`
    /// is the string `"42"` plus its `datatype`, not an Arrow `Int64`) and RDF 1.2
    /// triple terms are stringified to N-Triples in `value`.
    ///
    /// Requires `pyarrow` at call time (the term-struct ingestion uses pyarrow's
    /// Arrow-PyCapsule support, i.e. `pyarrow >= 14`); a clear `ImportError` is raised
    /// if it is missing.
    #[cfg(feature = "arrow")]
    fn query_arrow<'py>(&self, py: Python<'py>, sparql: &str) -> PyResult<Bound<'py, PyAny>> {
        let res = py
            .detach(|| sparq_engine::query(&self.inner, sparql))
            .map_err(engine_err)?;
        arrow_export::query_result_to_pyarrow_table(py, &res)
    }

    /// Answer an ASK query (a SELECT is also accepted: True iff it has rows).
    ///
    /// ASK runs on the engine's native entry point (evaluation early-exits at the
    /// first solution); a SELECT is answered by the engine's lazy solution count.
    fn ask(&self, py: Python<'_>, sparql: &str) -> PyResult<bool> {
        // [GPT-6] Keep VERSION metadata on both ASK and SELECT/count paths.
        let (query, versions) = sparq_engine::parse_versioned_query(spargebra::SparqlParser::new(), sparql)
            .map_err(|error| engine_err(error.to_string()))?;
        let prepared = sparq_engine::PreparedQuery::from_query_with_versions(query, versions)
            .map_err(engine_err)?;
        match prepared.query() {
            Query::Ask { .. } => {
                py.detach(|| sparq_engine::ask_prepared(&self.inner, &prepared)).map_err(engine_err)
            }
            Query::Select { .. } => {
                let n = py
                    .detach(|| sparq_engine::count_prepared(&self.inner, &prepared))
                    .map_err(engine_err)?;
                Ok(n > 0)
            }
            _ => Err(PyValueError::new_err("ask() takes an ASK (or SELECT) query")),
        }
    }

    /// Run a SPARQL CONSTRUCT, returning the constructed graph as a list of
    /// `(subject, predicate, object)` `Term` triples — a deduplicated set in
    /// first-production order (per SPARQL 1.1 §16.2: template triples with an
    /// unbound variable or an illegal RDF position are silently dropped, and
    /// template blank nodes are fresh per solution).
    fn construct(&self, py: Python<'_>, sparql: &str) -> PyResult<Vec<(Term, Term, Term)>> {
        let triples = py
            .detach(|| sparq_engine::construct(&self.inner, sparql))
            .map_err(engine_err)?;
        Ok(triples.iter().map(triple_terms).collect())
    }

    /// Run a SPARQL DESCRIBE, returning the union of the concise bounded
    /// descriptions (CBD: every triple whose subject is the resource, recursing
    /// through blank-node objects) of each described resource, as a list of
    /// `(subject, predicate, object)` `Term` triples.
    fn describe(&self, py: Python<'_>, sparql: &str) -> PyResult<Vec<(Term, Term, Term)>> {
        let triples = py
            .detach(|| sparq_engine::describe(&self.inner, sparql))
            .map_err(engine_err)?;
        Ok(triples.iter().map(triple_terms).collect())
    }

    /// Apply a SPARQL Update (`INSERT DATA` / `DELETE DATA` / `DELETE/INSERT …
    /// WHERE` / `CLEAR` / `DROP` / `CREATE` / `ADD` / `COPY` / `MOVE`) in place.
    /// The engine's store is immutable, so the update produces a NEW graph which
    /// this wrapper swaps in; on error the graph is left unchanged. The full
    /// dataset is modelled: `GRAPH`-scoped data and templates, `USING (NAMED)`,
    /// and the graph-management operations all work on named graphs.
    fn update(&mut self, py: Python<'_>, sparql: &str) -> PyResult<()> {
        let inner = &self.inner;
        let new = py.detach(|| sparq_engine::update(inner, sparql)).map_err(engine_err)?;
        self.inner = new;
        self.text = None; // rebuilt graph: the cached text index is stale
        Ok(())
    }

    /// Materialize the entailed closure for `profile` (`"rdfs"` or `"owl"`)
    /// in place, returning the number of NEW triples added. Idempotent.
    /// Reasoning runs over the default graph; named graphs are carried across
    /// the rebuild untouched.
    ///
    /// `"n3"` is not valid here — N3 rules live in an N3 document, so use
    /// `Graph.load_n3(text)` (rules + facts in one document) or
    /// `Graph.reason_n3_with(rules)` (rules applied to this graph) instead.
    fn reason(&mut self, py: Python<'_>, profile: &str) -> PyResult<usize> {
        if profile.eq_ignore_ascii_case("n3") {
            return Err(PyValueError::new_err(
                "N3 rules live in an N3 document; use Graph.load_n3(text) or Graph.reason_n3_with(rules) instead of reason(\"n3\")",
            ));
        }
        let prof = sparq_reason::Profile::parse(profile).ok_or_else(|| {
            PyValueError::new_err(format!("unknown reasoning profile {profile:?} (known: \"rdfs\", \"owl\")"))
        })?;
        // Take the graph apart through its public fields (no Clone on Dict/Graph):
        // move `dict` and `store` out, re-derive the canonical triples, materialize,
        // rebuild (re-attaching the named graphs, which reasoning does not touch).
        // A placeholder empty graph holds the slot during the closure.
        let g = std::mem::replace(&mut self.inner, CoreGraph::from_parts(Dict::new(), Vec::new()));
        let (inner, added) = py.detach(move || {
            let mut triples = all_triples(&g);
            let named = g.named;
            let mut dict = g.dict;
            let added = sparq_reason::materialize(prof, &mut dict, &mut triples);
            let mut rebuilt = CoreGraph::from_parts(dict, triples);
            rebuilt.named = named;
            (rebuilt, added)
        });
        self.inner = inner;
        self.text = None; // rebuilt graph: the cached text index is stale
        Ok(added)
    }

    /// Forward-chain caller-supplied Notation3 `rules` (an N3 document: `{ premise }
    /// => { conclusion }` rules, plus any facts it contains) over THIS graph's
    /// default graph, in place, returning the number of NEW triples added.
    /// Named graphs are carried across the rebuild untouched; on error (e.g. a
    /// rules parse failure) the graph is left unchanged.
    ///
    /// The graph's triples join the document as ground facts. This graph's blank
    /// nodes are renamed under a reserved `sparqg` prefix before composition, so
    /// a blank-node label in `rules` can NOT alias an existing data node (rule-
    /// local blanks stay rule-local; review 1961). RDF 1.2 triple terms have no
    /// N3 form and are rejected.
    fn reason_n3_with(&mut self, py: Python<'_>, rules: &str) -> PyResult<usize> {
        use std::fmt::Write as _;
        let inner = &self.inner;
        let before = inner.len();
        // Render the default graph as N-Triples (a syntactic subset of N3) under
        // the rules document and run the same chainer as `load_n3` — exactly the
        // composition sparq-reason's own MaterializedN3Graph uses in fallback mode.
        let mut new = py
            .detach(|| {
                let rows = all_triples(inner);
                let mut src = String::with_capacity(rules.len() + 1 + 64 * rows.len());
                src.push_str(rules);
                src.push('\n');
                for [s, p, o] in rows {
                    for id in [s, p, o] {
                        let term = inner.dict.term(id);
                        match term {
                            oxrdf::Term::Triple(_) => {
                                return Err("RDF 1.2 triple terms cannot participate in N3 reasoning".into());
                            }
                            // [OPUS-4.8] roborev 1961: rename this graph's blank
                            // nodes under the reserved `sparqg` prefix before
                            // composing the N3 document, so a blank-node label
                            // that also appears in the caller's `rules` cannot
                            // alias (and thereby silently merge with / rewrite)
                            // an existing data node. `sparqg` is reserved for
                            // this internal composition: an ordinary `_:x` label
                            // in the rules can no longer collide with data, and
                            // a data node's renamed form `_:sparqg<orig>` is
                            // unique to the data side.
                            oxrdf::Term::BlankNode(b) => {
                                let _ = write!(src, "_:sparqg{} ", b.as_str());
                            }
                            other => {
                                let _ = write!(src, "{other} ");
                            }
                        }
                    }
                    src.push_str(".\n");
                }
                let mut dict = Dict::new();
                let triples = sparq_reason::reason_n3(&mut dict, &src)?;
                Ok::<_, String>(CoreGraph::from_parts(dict, triples))
            })
            .map_err(engine_err)?;
        new.named = std::mem::take(&mut self.inner.named);
        let added = new.len().saturating_sub(before);
        self.inner = new;
        self.text = None; // rebuilt graph (fresh dictionary): the cached text index is stale
        Ok(added)
    }

    /// Full-text search over the default graph's string literals, returning a
    /// BM25-ranked best-first list of `(literal Term, score)` pairs.
    ///
    /// `query` is tokenized like the indexed text (UAX #29 words, lowercased);
    /// a token ending in `*` matches as a prefix (autocomplete). By default a
    /// literal must contain EVERY token (AND); `any=True` requires at least one
    /// (OR, scores sum over the tokens present). `limit` keeps only the best n
    /// hits. Scores are comparable within one query only.
    ///
    /// Indexed text: plain/`xsd:string` and language-tagged literals of the
    /// DEFAULT graph (named graphs keep their own dictionaries and are not
    /// indexed). The index is built lazily on first use, cached, and
    /// invalidated by `update` / `reason` / `reason_n3_with` (those rebuild
    /// the graph, so the next text call rebuilds the index).
    #[pyo3(signature = (query, any=false, limit=None))]
    fn text_search(
        &mut self,
        py: Python<'_>,
        query: &str,
        any: bool,
        limit: Option<usize>,
    ) -> Vec<(Term, f64)> {
        self.ensure_text_index(py);
        let index = self.text.as_ref().expect("ensure_text_index just built it");
        let inner = &self.inner;
        let mut hits =
            py.detach(|| if any { index.search_any(query) } else { index.search(query) });
        if let Some(n) = limit {
            hits.truncate(n);
        }
        hits.iter()
            .map(|h| (Term::from_oxrdf(&inner.dict.term(h.id)), f64::from(h.score)))
            .collect()
    }

    /// Run a SPARQL SELECT that may use the `text:` full-text magic predicates
    /// (`PREFIX text: <http://sparq.dev/text#>`):
    ///
    /// * `?lit text:matches "query"` — `?lit` ranges over indexed literals
    ///   containing EVERY query token (`*`-suffix = prefix match);
    /// * `?lit text:matchesAny "query"` — at least one token;
    /// * `?lit text:score ?s` — binds the BM25 score (`xsd:double`; sort with
    ///   `ORDER BY DESC(?s)` — the hit table carries no order through joins).
    ///
    /// The magic patterns are rewritten into inline `VALUES` over the index's
    /// hits and the query runs on the standard engine, so `?lit` joins to
    /// triples like any other variable. A query without `text:` patterns
    /// behaves exactly like `Graph.query`. Uses the same lazily built, cached
    /// index as `text_search` (see there for lifecycle and what is indexed).
    fn query_text(&mut self, py: Python<'_>, sparql: &str) -> PyResult<QueryResult> {
        self.ensure_text_index(py);
        let index = self.text.as_ref().expect("ensure_text_index just built it");
        let inner = &self.inner;
        let res = py
            .detach(|| sparq_text::query_text(inner, sparql, index))
            .map_err(engine_err)?;
        select_result(py, &res)
    }

    /// Eagerly (re)build the full-text index now (it is otherwise built lazily
    /// by the first `text_search` / `query_text`), returning the number of
    /// indexed literals — useful to pay the build cost at load time instead of
    /// on the first query.
    fn build_text_index(&mut self, py: Python<'_>) -> usize {
        self.text = None;
        self.ensure_text_index(py);
        self.text.as_ref().expect("ensure_text_index just built it").len()
    }

    /// Drop the cached full-text index, freeing its memory. Purely a resource
    /// release: the next `text_search` / `query_text` lazily rebuilds it.
    fn drop_text_index(&mut self) {
        self.text = None;
    }

    /// Detect OWL 2 RL inconsistencies (clashes) in the default graph, returning
    /// one human-readable description per clash (empty list = no detected
    /// inconsistency). Covers the OWL 2 RL false rules: cax-dw (disjointWith) /
    /// cax-adc (AllDisjointClasses), cls-com (complementOf), cls-nothing
    /// (owl:Nothing instances), cls-maxc1 / cls-maxqc1/2 (cardinality-0
    /// violations), eq-diff1/2/3 (sameAs vs differentFrom / AllDifferent,
    /// including sameAs forced between distinct literal values), prp-asyp /
    /// prp-irp (asymmetric & irreflexive violations), prp-pdw / prp-adp
    /// (disjoint properties), and prp-npa1/2 (negative property assertions).
    ///
    /// Detection is over ASSERTED triples: run `reason("owl")` first to surface
    /// clashes that only follow by entailment.
    fn inconsistencies(&self, py: Python<'_>) -> PyResult<Vec<String>> {
        let inner = &self.inner;
        Ok(py.detach(|| sparq_reason::inconsistencies(&inner.dict, &all_triples(inner))))
    }

    /// A cheap, logically-INDEPENDENT copy of this graph that can then be
    /// mutated separately — `update` / `reason` / `reason_n3_with` on either the
    /// original or the copy leave the other untouched.
    ///
    /// Built on the core's structural snapshot (`sparq_core::Graph::snapshot`):
    /// the immutable base storage (the six permutation indexes, the frozen
    /// dictionary base, the numeric caches) is `Arc`-shared, so this is
    /// O(pending delta), NEVER O(triples) — it duplicates neither the indexes nor
    /// the dictionary arena. (The first copy of a freshly-loaded graph pays a
    /// one-time O(n) freeze to mint the shareable base; subsequent copies of the
    /// frozen lineage are cheap.) Named graphs are copied too.
    ///
    /// The cached full-text index is NOT carried over: the copy starts without
    /// one and lazily rebuilds on its first `text_search` / `query_text`, exactly
    /// as a freshly loaded graph would (the same lazy-rebuild policy a mutating
    /// swap uses). [OPUS-4.8] sq-6h8
    fn copy(&self, py: Python<'_>) -> Graph {
        let inner = &self.inner;
        // `snapshot()` yields an immutable point-in-time view; `into_graph()`
        // drops the read-only wrapper, leaving an independently-mutable `Graph`
        // that already owns its own Arc-shared base + per-generation delta.
        let copied = py.detach(|| inner.snapshot().into_graph());
        Graph { inner: copied, text: None }
    }

    /// Number of triples in the default graph.
    fn __len__(&self) -> usize {
        self.inner.len()
    }

    fn __repr__(&self) -> String {
        format!("Graph({} triples)", self.inner.len())
    }

    // [OPUS-4.8] sq-f9tu: test-only affordance to exercise the panic SURFACE of
    // the FFI boundary. The whole point of the `python-release` profile is
    // `panic = "unwind"` (workspace Cargo.toml) so that a Rust panic inside the
    // `sparq` module is caught by pyo3 and re-raised as
    // `pyo3_runtime.PanicException` instead of `panic = "abort"` killing the
    // interpreter. The engine itself proved robust — no normal-input
    // query/update/load reaches a panic (verified empirically while writing
    // test_parity.py: division-by-zero, bad regex, RDF 1.2 triple terms, unsupported
    // patterns all return a clean ValueError), so there is no organic input that
    // drives a panic. This private hook is the only reliable way to assert the
    // unwind path is wired correctly. The leading underscore marks it private;
    // it is intentionally undocumented in SKILL.md (not part of the public API).
    fn _panic_for_test(&self) {
        panic!("sparq-py panic-surface test: this must reach Python as a PanicException, not abort");
    }
}

// ---------------------------------------------------------------------------
// Module
// ---------------------------------------------------------------------------

/// The `sparq` Python module.
#[pymodule]
fn sparq(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Graph>()?;
    m.add_class::<QueryResult>()?;
    m.add_class::<Term>()?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
