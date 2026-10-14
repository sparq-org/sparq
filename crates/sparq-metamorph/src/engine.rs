//! The engine-under-test abstraction, the in-process sparq driver, and the
//! deliberately-injected wrong-result mutant used by the oracle self-tests.
//! sq-gum8.6
//!
//! Everything the oracles need from an engine is one operation: evaluate a SPARQL
//! query and return parsed results ([`SparqlEngine::select`]). Results come back in
//! sparq-difftest's engine-independent neutral model ([`sparq_difftest::QueryResults`]),
//! so the oracles never touch an engine's in-memory types.
//!
//! Drivers implemented here:
//!
//! * [`InProcessSparq`] — sparq itself, in-process, via `sparq_engine::query_json` →
//!   `sparq_difftest::parse_results_json`. This deliberately round-trips through the
//!   engine's REAL SPARQL-Results-JSON serialisation (the same bytes an HTTP client
//!   would see), not a private fast path, so the self-tests exercise the production
//!   result surface. sparq is also an honest campaign *target*: finding our own bugs
//!   counts.
//! * [`FilterDropsRow`] — the mutation self-test's wrong-result mutant (see its docs).
//!
//! HTTP drivers for external engines over the SPARQL 1.1 Protocol live in the
//! feature-gated `driver` module (`protocol-drivers` cargo feature).

use sparq_difftest::{parse_results_json, QueryResults};

use crate::verdict::{EngineFailure, FailureKind};

/// An engine under test: evaluate a SPARQL query, return neutral-model results.
///
/// `select` covers both `SELECT` (→ `QueryResults::Solutions`) and `ASK`
/// (→ `QueryResults::Boolean`). An implementation must map every non-evaluation
/// (rejection, transport error, malformed response) to an [`EngineFailure`] — never to
/// an empty result (that would convert an engine failure into a plausible wrong-result
/// signal and break the strict wrong-result / engine-error distinction).
pub trait SparqlEngine {
    /// Stable engine name for verdicts and the found-bug ledger.
    fn name(&self) -> &str;

    /// Evaluate `sparql` and return the parsed result set.
    fn select(&self, sparql: &str) -> Result<QueryResults, EngineFailure>;
}

/// sparq itself, in-process: a [`sparq_core::Graph`] queried through
/// [`sparq_engine::query_json`], with the JSON parsed back through sparq-difftest's
/// engine-independent reader.
pub struct InProcessSparq {
    name: String,
    graph: sparq_core::Graph,
}

impl InProcessSparq {
    /// Build the engine over an N-Triples document.
    ///
    /// A load failure is an [`EngineFailure`] of kind [`FailureKind::Load`].
    pub fn from_ntriples(name: &str, ntriples: &str) -> Result<Self, EngineFailure> {
        let graph = sparq_core::Graph::load_str(ntriples, "ntriples").map_err(|e| EngineFailure {
            engine: name.to_string(),
            query: String::new(),
            kind: FailureKind::Load,
            message: e,
        })?;
        Ok(InProcessSparq {
            name: name.to_string(),
            graph,
        })
    }
}

impl SparqlEngine for InProcessSparq {
    fn name(&self) -> &str {
        &self.name
    }

    fn select(&self, sparql: &str) -> Result<QueryResults, EngineFailure> {
        let json = sparq_engine::query_json(&self.graph, sparql).map_err(|e| EngineFailure {
            engine: self.name.clone(),
            query: sparql.to_string(),
            kind: FailureKind::Evaluation,
            message: e,
        })?;
        parse_results_json(&json).map_err(|e| EngineFailure {
            engine: self.name.clone(),
            query: sparql.to_string(),
            kind: FailureKind::InvalidResults,
            message: e.to_string(),
        })
    }
}

/// The **deliberately-injected wrong-result mutant** for the mutation self-tests
/// (the crate's non-vacuity invariant): wraps a real engine and silently removes the
/// last solution from the result of any query whose text contains `FILTER`.
///
/// This mimics a classic real logic-bug shape — a predicate/filter implementation that
/// loses a matching row (silently incorrect results; nothing errors) — and each oracle
/// must flag it:
///
/// * **TLP**: the unfiltered base query is untouched while every partition query
///   (each carries a `FILTER`) loses a row, so the partition-union multiset cannot
///   equal the base.
/// * **NoREC**: the optimizable form (`FILTER`) loses a row while the non-optimizable
///   rewrite (predicate in projection position, no `FILTER` keyword) is untouched, so
///   the cardinalities disagree.
/// * **Differential** (both modes): the mutant disagrees with a pristine engine on any
///   `FILTER` query with a non-empty result.
/// * **TLP/`DISTINCT`** ([`crate::distinct`]): the dropped row is removed from an
///   already-deduplicated branch result, so it disappears from the branch *set* union
///   while the (`FILTER`-free) base keeps it.
/// * **TLP/aggregate** ([`crate::aggregate`]): each branch query returns exactly one
///   solution, so dropping it breaks the single-group row count the oracle requires.
///
/// A self-test suite whose oracles do NOT flag this mutant is vacuous; the integration
/// tests assert every one of those flags. This mutant perturbs **cardinality** only, so
/// the `sq-gum8.12` variants carry their own mutants for the laws it cannot touch — an
/// off-by-one aggregate value, a silently unbound aggregate, and a reversed result
/// order (the last of which the *unordered* differential oracle provably cannot see).
pub struct FilterDropsRow<E: SparqlEngine> {
    name: String,
    inner: E,
}

impl<E: SparqlEngine> FilterDropsRow<E> {
    /// Wrap `inner`; the mutant's name is `<inner-name>+filter-drops-row`.
    pub fn new(inner: E) -> Self {
        let name = format!("{}+filter-drops-row", inner.name());
        FilterDropsRow { name, inner }
    }
}

impl<E: SparqlEngine> SparqlEngine for FilterDropsRow<E> {
    fn name(&self) -> &str {
        &self.name
    }

    fn select(&self, sparql: &str) -> Result<QueryResults, EngineFailure> {
        let mut results = self.inner.select(sparql)?;
        if sparql.contains("FILTER") {
            if let QueryResults::Solutions { solutions, .. } = &mut results {
                solutions.pop();
            }
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DATA: &str = concat!(
        "<http://example.org/s1> <http://example.org/v> \"1\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
        "<http://example.org/s2> <http://example.org/v> \"2\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
    );

    #[test]
    fn from_ntriples_loads_and_selects() {
        let engine = InProcessSparq::from_ntriples("sparq", DATA).unwrap();
        assert_eq!(engine.name(), "sparq");
        let results = engine
            .select("SELECT * WHERE { ?s <http://example.org/v> ?v }")
            .unwrap();
        match results {
            QueryResults::Solutions { solutions, .. } => assert_eq!(solutions.len(), 2),
            QueryResults::Boolean(_) => panic!("SELECT returned a boolean"),
        }
    }

    #[test]
    fn from_ntriples_load_failure_is_engine_failure_of_kind_load() {
        let err = match InProcessSparq::from_ntriples("sparq", "this is not n-triples") {
            Err(failure) => failure,
            Ok(_) => panic!("garbage input must not load"),
        };
        assert_eq!(err.kind, FailureKind::Load);
        assert_eq!(err.engine, "sparq");
    }

    #[test]
    fn invalid_query_is_engine_failure_not_empty_result() {
        let engine = InProcessSparq::from_ntriples("sparq", DATA).unwrap();
        let err = engine.select("SELECT * WHERE { unparseable").unwrap_err();
        assert_eq!(err.kind, FailureKind::Evaluation);
        assert!(!err.message.is_empty());
    }

    #[test]
    fn filter_drops_row_mutant_drops_exactly_on_filter_queries() {
        let mutant = FilterDropsRow::new(InProcessSparq::from_ntriples("sparq", DATA).unwrap());
        assert_eq!(mutant.name(), "sparq+filter-drops-row");
        let no_filter = mutant
            .select("SELECT * WHERE { ?s <http://example.org/v> ?v }")
            .unwrap();
        let with_filter = mutant
            .select("SELECT * WHERE { ?s <http://example.org/v> ?v FILTER(?v > 0) }")
            .unwrap();
        match (no_filter, with_filter) {
            (
                QueryResults::Solutions { solutions: a, .. },
                QueryResults::Solutions { solutions: b, .. },
            ) => {
                assert_eq!(a.len(), 2, "non-FILTER query untouched");
                assert_eq!(b.len(), 1, "FILTER query loses one row");
            }
            _ => panic!("SELECT returned a boolean"),
        }
    }
}
