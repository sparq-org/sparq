//! RDFC-1.0 canonicalization of a named graph's content (plan §2.2).
//!
//! The commitment target is the **RDFC-1.0-canonicalized content graph**: a
//! credential document's triples, canonicalized per graph at ingest, giving
//! canonical bnode labels (`c14n0`, `c14n1`, …) and a total order over the
//! triples (canonical code-point-sorted N-Quads). The position of a triple in
//! that order is its **leaf index** in the per-graph commitment.
//!
//! As of sq-0qip the RDFC-1.0 implementation is single-sourced in the opt-in
//! [`sparq_canon`] crate (the public canonicalization surface). This module is
//! a thin re-export so the ZK pipeline's call sites are unchanged; the bridge
//! (oxrdf 0.3 ↔ oxrdf 0.2 over canonical N-Quads text) and the W3C
//! rdf-canon-test-suite validation live there.
//!
//! RDFC-1.0 has pathological-input blow-ups (plan §8): the HNDQ-call-limit
//! guard is kept at `sparq_canon`'s default, and limit hits surface as
//! [`CanonError::Canonicalization`] — ingest fails closed on poison graphs.
//! [OPUS-4.8] sq-0qip.

// Re-export the public canonicalization surface so `sparq_zk::canon::*` paths
// (used by `commit`, `ingest`, `trace`, and `sparq-zk-compose`) keep working
// without change.
pub use sparq_canon::{
    canonicalize_graph_content, canonicalize_triples, graph_triples, CanonError, CanonicalGraph,
};
