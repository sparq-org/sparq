// Per-holder local SPARQL sub-evaluation — the REAL M0 core.
//! Per-holder local SPARQL sub-evaluation.
//!
//! Architecture refs: §4.1 (holders own one-or-more signed named graphs),
//! §4.3 step 1 (holders ingest their VCs and evaluate locally), and the
//! load-bearing §4.2 invariant: **minimise inter-node data sharing**. A holder
//! evaluates a query fragment over ITS OWN graphs *locally* and ships only the
//! disclosed partial result — it never hands its raw graphs to anyone.
//!
//! This is the one piece that is genuinely implemented at Milestone 0, because
//! it is **invariant to both open design forks** (§5.2): no matter which MPC
//! primitive (Q2) or which collaborative-proof construction (Q1) is chosen
//! later, the federation always begins by each holder answering what it can
//! over its own data. Everything cryptographic is layered ON TOP of these
//! partials by [`crate::backend`], [`crate::join`] and [`crate::proof`].
//!
//! What this is NOT: it is not the MPC. Joining partials across holders on
//! global IRIs, hiding intermediate values, and proving correctness/attestation
//! are all deferred. A [`Holder`] here is a plaintext local evaluator — the
//! honest substrate the privacy layer wraps.

use crate::partial::{HolderId, MpcError, PartialResult};
use sparq_core::Graph;
use sparq_engine::query;

/// One federation participant: an identity plus the holder's own local RDF data
/// (its named-graph wallet, modelled at M0 as a single in-memory [`Graph`]).
///
/// Architecture §2 convention #1: each VC is a named-graph unit and a holder's
/// wallet is a *set* of such graphs. `sparq-core`'s loader folds named graphs
/// into one queryable graph (the engine evaluates over the active dataset), so
/// at M0 a holder's wallet is represented by a single [`Graph`]; per-graph
/// commitment/attestation boundaries (which graph a row came from) are an M1+
/// concern bound in-circuit (ZK-remediation #8) and are not modelled here.
pub struct Holder {
    id: HolderId,
    /// This holder's private data. NEVER leaves the holder — only
    /// [`PartialResult`]s derived from it are shared (§4.2 minimise sharing).
    graph: Graph,
}

/// Convenience alias: a holder's local sub-evaluation yields either a disclosed
/// partial or a (real) local-eval error.
pub type HolderResult = Result<PartialResult, MpcError>;

impl Holder {
    /// Create a holder from an already-built graph.
    pub fn new(id: impl Into<String>, graph: Graph) -> Self {
        Holder {
            id: HolderId::new(id),
            graph,
        }
    }

    /// Create a holder by loading its wallet from an RDF document.
    ///
    /// `format`: `"turtle"` | `"ntriples"` | `"nquads"` | `"trig"` (passed
    /// through to [`Graph::load_str`]). A parse failure is a real
    /// [`MpcError::LocalEval`].
    pub fn from_rdf(id: impl Into<String>, text: &str, format: &str) -> Result<Self, MpcError> {
        let id = HolderId::new(id);
        let graph = Graph::load_str(text, format).map_err(|e| MpcError::LocalEval {
            holder: id.clone(),
            message: e,
        })?;
        Ok(Holder { id, graph })
    }

    /// This holder's identity.
    pub fn id(&self) -> &HolderId {
        &self.id
    }

    /// Number of triples in this holder's local wallet (diagnostics only; never
    /// disclosed across the federation).
    pub fn local_triple_count(&self) -> usize {
        self.graph.len()
    }

    /// Evaluate a query fragment LOCALLY over this holder's own graphs and
    /// return the disclosed partial result.
    ///
    /// This is the invariant minimise-data-sharing core (§4.2 / §4.3 step 1):
    /// the holder runs the (SELECT) fragment through `sparq-engine` against its
    /// private data and emits only the projected rows. Its raw graph is never
    /// shared.
    ///
    /// At M0 the fragment is evaluated in full and its complete SELECT result is
    /// disclosed (convention #4's disclosed-property regime). The hidden-value
    /// regime — where a per-source intermediate (e.g. each flatmate's exact
    /// salary in the cumulative-£100k aggregate) must NEVER leave the source and
    /// instead enters the secure computation — is the job of [`crate::backend`]
    /// and is **not** performed here. So this method is appropriate for the
    /// fragment a holder is willing to disclose locally; cross-source secret
    /// values are handled by the (deferred) MPC layer, not by widening this.
    pub fn evaluate_local(&self, fragment_sparql: &str) -> HolderResult {
        let result = query(&self.graph, fragment_sparql).map_err(|e| MpcError::LocalEval {
            holder: self.id.clone(),
            message: e,
        })?;
        Ok(PartialResult {
            holder: self.id.clone(),
            vars: result.vars,
            rows: result.rows,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::Term;

    /// The driving use case (architecture §2), shrunk to the invariant core:
    /// TWO holders, each with its OWN payslip-style data, each answering the
    /// SAME fragment LOCALLY over only its own graphs. This exercises exactly
    /// the M0 guarantee — holders never share raw graphs, only their disclosed
    /// partials — and asserts each holder's partial reflects ONLY its own data.
    #[test]
    fn two_holders_each_answer_locally_over_their_own_graphs() {
        const PREFIX: &str =
            "@prefix ex: <http://ex/> . @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n";

        // Alice's wallet: one person earning 30k.
        let alice = Holder::from_rdf(
            "alice",
            &format!("{PREFIX} ex:alice ex:name \"Alice\" ; ex:salary \"30000\"^^xsd:integer ."),
            "turtle",
        )
        .expect("alice wallet parses");

        // Bob's wallet: one person earning 45k. Bob's data is disjoint from
        // Alice's — neither holder can see the other's graph.
        let bob = Holder::from_rdf(
            "bob",
            &format!("{PREFIX} ex:bob ex:name \"Bob\" ; ex:salary \"45000\"^^xsd:integer ."),
            "turtle",
        )
        .expect("bob wallet parses");

        // The SAME fragment, run independently against each holder's own graph.
        let fragment = "PREFIX ex: <http://ex/> \
             SELECT ?name ?salary WHERE { ?p ex:name ?name ; ex:salary ?salary }";

        let pa = alice.evaluate_local(fragment).expect("alice local eval ok");
        let pb = bob.evaluate_local(fragment).expect("bob local eval ok");

        // Each partial is attributed to its producing holder.
        assert_eq!(pa.holder, HolderId::new("alice"));
        assert_eq!(pb.holder, HolderId::new("bob"));

        // Same projected schema (a precondition the join layer will rely on).
        assert_eq!(pa.vars, pb.vars);

        // Each holder discloses exactly ONE row — its own — and nothing of the
        // other's data leaks into its partial. This is the minimise-data-sharing
        // invariant made concrete.
        assert_eq!(pa.len(), 1, "alice discloses only her own row");
        assert_eq!(pb.len(), 1, "bob discloses only his own row");

        let name_of = |p: &PartialResult| match p.rows[0][0].as_ref().unwrap() {
            Term::Literal(l) => l.value().to_string(),
            other => other.to_string(),
        };
        assert_eq!(name_of(&pa), "Alice");
        assert_eq!(name_of(&pb), "Bob");

        // Alice's partial contains no trace of Bob and vice-versa.
        assert!(!format!("{:?}", pa.rows).contains("Bob"));
        assert!(!format!("{:?}", pb.rows).contains("Alice"));
    }

    /// A holder that holds nothing matching the fragment discloses an empty
    /// partial — it does not error, and it certainly does not borrow another
    /// holder's data. (OWA-safe: an absent fact is absent, not forged — §2
    /// convention #8.)
    #[test]
    fn holder_with_no_match_discloses_empty_partial() {
        let h = Holder::from_rdf(
            "carol",
            "@prefix ex: <http://ex/> . ex:carol ex:hobby \"chess\" .",
            "turtle",
        )
        .unwrap();
        let p = h
            .evaluate_local("PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:salary ?v }")
            .expect("eval ok even with no matches");
        assert!(p.is_empty());
        assert_eq!(p.holder, HolderId::new("carol"));
    }

    /// A malformed fragment is a REAL local-eval error attributed to the
    /// holder — not a panic, not a silent empty result.
    #[test]
    fn malformed_fragment_is_a_real_local_error() {
        let h = Holder::from_rdf(
            "dave",
            "@prefix ex: <http://ex/> . ex:a ex:b ex:c .",
            "turtle",
        )
        .unwrap();
        let err = h.evaluate_local("this is not sparql").unwrap_err();
        match err {
            MpcError::LocalEval { holder, .. } => assert_eq!(holder, HolderId::new("dave")),
            other => panic!("expected LocalEval, got {other:?}"),
        }
    }
}
