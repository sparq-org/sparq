//! WAC/ACP-aware per-source authorisation for federated planning — the `source-auth`
//! feature.
//!
//! [SONNET-4.6] `sq-lzvl` (issue #3296), the **B7 hook** of
//! `research/mpc-untrusted-planner-routing-design.md` §8 Phase 7. That record left §9 Q4
//! open: *is access-control-aware source skipping in-scope for the MPC routing track, or
//! does it belong with `sparq-solid` (WAC/ACP) and only reference the seam?* It is
//! answered here as **the second option** — the policy lives with the access-control
//! engine that already owns WAC/ACP, and the MPC seam only *consumes* a decision.
//!
//! Concretely: **no dependency edge is created in either direction.** `sparq-solid` does
//! not depend on `sparq-fedplan-mpc`, and `sparq-fedplan-mpc` does not depend on
//! `sparq-solid`. The join is a plain `bool` — [`SourceAuthorization::participates`] is
//! exactly the value the seam's reserved authorisation field takes:
//!
//! ```text
//! // in an integrator that opts into BOTH crates (neither knows about the other):
//! let decision = pod_store.authorize_source(&session, Mode::Read, &source);
//! let descriptor = SourcePrivacyDescriptor::builder(source_id)
//!     .participates(decision.participates())   // ← the B7 hook, reserved by Phase 1
//!     .build();
//! ```
//!
//! # What a decision means (SAFE-style source skipping)
//!
//! A *source* is a federation participant declared as the set of named graphs it serves
//! ([`SourceDescriptor`]). It **participates** iff the session can read at least one of
//! those graphs under the store's materialized `<urn:sparq:auth>` view; otherwise it is
//! **skipped**, because a source from which the session may read nothing can contribute
//! nothing to the session's answer. A partially-authorised source participates with only
//! its authorised subset ([`SourceAuthorization::authorized_graphs`]) — the decision can
//! **narrow** what a source is asked for, never widen it.
//!
//! # Fail-closed semantics
//!
//! * A source declaring **no** graphs is skipped ([`SkipReason::NoDeclaredGraphs`]):
//!   absence of a declaration is not evidence of authorisation.
//! * A source whose every declared graph is outside the session's accessible set is
//!   skipped ([`SkipReason::NoAuthorizedGraph`]).
//! * Everything [`PodStore::accessible`] fails closed on, this fails closed on too —
//!   before the first `materialize_*` call, for a session with no matching grants, and
//!   for session values inside the reserved `urn:sparq:` encoding, *every* source is
//!   skipped.
//!
//! # Honest scope
//!
//! This is a **plan-time** decision over the local auth view: it enforces nothing at a
//! remote source and authenticates no participant, so a source that is asked for a graph
//! must still enforce its own access control. Skipping is a confidentiality-and-cost
//! measure, not a completeness guarantee — a source whose served graph set is undeclared
//! or stale is skipped, and the answer is correspondingly incomplete. It makes **no**
//! MPC, privacy or zero-knowledge claim: it is ordinary WAC/ACP evaluation, and the MPC
//! estate it can feed remains research-grade, semi-honest-only and externally unaudited
//! (`sq-qhy4`).

use crate::authindex::{Mode, Session};
use crate::PodStore;
use oxrdf::{NamedNode, Term};

/// A federation source declared as the set of named graphs it serves.
///
/// The graph IRIs are the *same* named-graph identities the auth view grants over, which
/// is what lets a WAC/ACP decision apply to a source at all. Declaration is the caller's
/// job (a service description, a registry, a config file); this type performs no
/// discovery and contacts nothing.
///
/// ```
/// use oxrdf::NamedNode;
/// use sparq_solid::SourceDescriptor;
///
/// let source = SourceDescriptor::new("pod-a")
///     .serving(NamedNode::new("https://a.ex/notes").unwrap())
///     .serving(NamedNode::new("https://a.ex/private").unwrap());
/// assert_eq!(source.id(), "pod-a");
/// assert_eq!(source.graphs().len(), 2);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDescriptor {
    id: String,
    graphs: Vec<NamedNode>,
}

impl SourceDescriptor {
    /// A source named `id` serving **no** graphs yet. Left empty it is skipped
    /// ([`SkipReason::NoDeclaredGraphs`]) — the default-deny baseline.
    pub fn new(id: impl Into<String>) -> SourceDescriptor {
        SourceDescriptor { id: id.into(), graphs: Vec::new() }
    }

    /// Declares one more named graph this source serves.
    pub fn serving(mut self, graph: NamedNode) -> SourceDescriptor {
        self.graphs.push(graph);
        self
    }

    /// Declares every named graph in `graphs`.
    pub fn serving_all(mut self, graphs: impl IntoIterator<Item = NamedNode>) -> SourceDescriptor {
        self.graphs.extend(graphs);
        self
    }

    /// The source's identifier, as declared.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The named graphs this source declares it serves, in declaration order.
    pub fn graphs(&self) -> &[NamedNode] {
        &self.graphs
    }
}

/// Why a source was skipped. Both variants are **fail-closed** outcomes: neither says the
/// source is untrustworthy, only that the session was not shown a graph it may read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// The source declared no served graphs, so nothing could be authorised.
    NoDeclaredGraphs,
    /// Every graph the source declared is outside the session's accessible set.
    NoAuthorizedGraph,
}

impl SkipReason {
    /// A short stable label, for logging and for a plan-explanation line.
    pub fn as_str(&self) -> &'static str {
        match self {
            SkipReason::NoDeclaredGraphs => "no-declared-graphs",
            SkipReason::NoAuthorizedGraph => "no-authorized-graph",
        }
    }
}

/// The per-source decision: participate (with the authorised graph subset) or skip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceAuthorization {
    source_id: String,
    authorized_graphs: Vec<NamedNode>,
    skip_reason: Option<SkipReason>,
}

impl SourceAuthorization {
    /// The identifier of the source this decision is about.
    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    /// Whether the source takes part in the federated plan — `true` iff at least one of
    /// its declared graphs is readable by the session.
    ///
    /// This is the value the MPC seam's reserved authorisation field takes (§8 Phase 7);
    /// see the module docs for the hand-off.
    pub fn participates(&self) -> bool {
        self.skip_reason.is_none()
    }

    /// The subset of the source's declared graphs the session may access, sorted by IRI
    /// and deduplicated (the seam is deterministic throughout). Empty exactly when the
    /// source is skipped. Ask the source for **these** graphs only.
    pub fn authorized_graphs(&self) -> &[NamedNode] {
        &self.authorized_graphs
    }

    /// Why the source was skipped, or `None` when it participates.
    pub fn skip_reason(&self) -> Option<SkipReason> {
        self.skip_reason
    }

    fn skipped(source_id: String, reason: SkipReason) -> SourceAuthorization {
        SourceAuthorization { source_id, authorized_graphs: Vec::new(), skip_reason: Some(reason) }
    }
}

impl PodStore {
    /// Decides whether `source` participates in a federated plan for this session, under
    /// the store's materialized WAC/ACP view — the B7 source-skipping hook.
    ///
    /// Fail-closed in every direction; see the [module docs](self) for the exact rules
    /// and the honest scope (this is a plan-time decision, not remote enforcement).
    ///
    /// ```
    /// use oxrdf::NamedNode;
    /// use sparq_solid::{Mode, PodStore, Session, SkipReason, SourceDescriptor};
    ///
    /// # let nquads = r#"
    /// # <https://a.ex/notes#n> <https://ex.dev/ns#title> "hello" <https://a.ex/notes> .
    /// # <https://a.ex/notes.acl#r> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://a.ex/notes.acl> .
    /// # <https://a.ex/notes.acl#r> <http://www.w3.org/ns/auth/acl#accessTo> <https://a.ex/notes> <https://a.ex/notes.acl> .
    /// # <https://a.ex/notes.acl#r> <http://www.w3.org/ns/auth/acl#agent> <https://alice.ex/card#me> <https://a.ex/notes.acl> .
    /// # <https://a.ex/notes.acl#r> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://a.ex/notes.acl> .
    /// # "#;
    /// # let mut store = PodStore::new(sparq_core::Graph::load_dataset(nquads, "nquads")?);
    /// # store.materialize_wac()?;
    /// let source = SourceDescriptor::new("pod-a")
    ///     .serving(NamedNode::new("https://a.ex/notes").unwrap());
    /// let alice =
    ///     Session { agent: Some("https://alice.ex/card#me"), client: None, issuer: None, now: None };
    ///
    /// let allowed = store.authorize_source(&alice, Mode::Read, &source);
    /// assert!(allowed.participates());
    /// assert_eq!(allowed.authorized_graphs().len(), 1);
    ///
    /// // An anonymous session holds no grant, so the source is skipped entirely.
    /// let anon = store.authorize_source(&Session::default(), Mode::Read, &source);
    /// assert!(!anon.participates());
    /// assert_eq!(anon.skip_reason(), Some(SkipReason::NoAuthorizedGraph));
    /// # Ok::<(), String>(())
    /// ```
    pub fn authorize_source(
        &self,
        s: &Session,
        mode: Mode,
        source: &SourceDescriptor,
    ) -> SourceAuthorization {
        if source.graphs.is_empty() {
            return SourceAuthorization::skipped(source.id.clone(), SkipReason::NoDeclaredGraphs);
        }
        let visible = self.accessible_set(s, mode);
        let mut authorized: Vec<NamedNode> = source
            .graphs
            .iter()
            .filter(|g| visible.contains(&Term::NamedNode((*g).clone())))
            .cloned()
            .collect();
        // Deterministic order + no duplicate declaration survives into the decision.
        authorized.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        authorized.dedup();
        if authorized.is_empty() {
            return SourceAuthorization::skipped(source.id.clone(), SkipReason::NoAuthorizedGraph);
        }
        SourceAuthorization {
            source_id: source.id.clone(),
            authorized_graphs: authorized,
            skip_reason: None,
        }
    }

    /// [`PodStore::authorize_source`] over a whole candidate source set, returning one
    /// decision per input **in input order** (a plan step is deterministic). Every source
    /// resolves the same `(session, mode)` accessible set through the store's session
    /// cache, so past the first lookup the per-source work is one hash probe per declared
    /// graph.
    pub fn authorize_sources(
        &self,
        s: &Session,
        mode: Mode,
        sources: &[SourceDescriptor],
    ) -> Vec<SourceAuthorization> {
        sources.iter().map(|src| self.authorize_source(s, mode, src)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparq_core::Graph;

    const ALICE: &str = "https://alice.ex/card#me";
    const NOTES: &str = "https://a.ex/notes";
    const PRIVATE: &str = "https://a.ex/private";
    const PUBLIC: &str = "https://a.ex/public";

    fn node(iri: &str) -> NamedNode {
        NamedNode::new(iri).unwrap()
    }

    /// A pod where alice may read `notes`, everyone may read `public`, and nobody has a
    /// grant on `private`.
    fn store() -> PodStore {
        let rdf_type = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
        let acl_ns = "http://www.w3.org/ns/auth/acl#";
        let mut nq = String::new();
        // Control-document naming convention: <R + ".acl"> governs <R>.
        for (who_pred, who, target) in [
            ("agent", ALICE, NOTES),
            ("agentClass", "http://xmlns.com/foaf/0.1/Agent", PUBLIC),
        ] {
            let acl = format!("{}.acl", target);
            let a = format!("{}#auth", acl);
            nq.push_str(&format!(
                "<{}> <{}> <{}Authorization> <{}> .\n",
                a, rdf_type, acl_ns, acl
            ));
            nq.push_str(&format!("<{}> <{}accessTo> <{}> <{}> .\n", a, acl_ns, target, acl));
            nq.push_str(&format!("<{}> <{}{}> <{}> <{}> .\n", a, acl_ns, who_pred, who, acl));
            nq.push_str(&format!("<{}> <{}mode> <{}Read> <{}> .\n", a, acl_ns, acl_ns, acl));
        }
        for g in [NOTES, PRIVATE, PUBLIC] {
            nq.push_str(&format!("<{}#it> <https://ex.dev/ns#t> \"x\" <{}> .\n", g, g));
        }
        let mut store = PodStore::new(Graph::load_dataset(&nq, "nquads").unwrap());
        store.materialize_wac().unwrap();
        store
    }

    fn alice() -> Session<'static> {
        Session { agent: Some(ALICE), client: None, issuer: None, now: None }
    }

    #[test]
    fn descriptor_records_id_and_declared_graphs_in_order() {
        let src = SourceDescriptor::new("pod-a")
            .serving(node(NOTES))
            .serving_all([node(PRIVATE), node(PUBLIC)]);
        assert_eq!(src.id(), "pod-a");
        assert_eq!(
            src.graphs().iter().map(|g| g.as_str()).collect::<Vec<_>>(),
            vec![NOTES, PRIVATE, PUBLIC]
        );
        assert!(SourceDescriptor::new("empty").graphs().is_empty());
    }

    #[test]
    fn skip_reason_labels_are_stable() {
        assert_eq!(SkipReason::NoDeclaredGraphs.as_str(), "no-declared-graphs");
        assert_eq!(SkipReason::NoAuthorizedGraph.as_str(), "no-authorized-graph");
    }

    #[test]
    fn authorized_source_participates_with_only_the_readable_subset() {
        let store = store();
        // The source serves one readable and one unreadable graph.
        let src = SourceDescriptor::new("pod-a").serving_all([node(PRIVATE), node(NOTES)]);
        let d = store.authorize_source(&alice(), Mode::Read, &src);
        assert_eq!(d.source_id(), "pod-a");
        assert!(d.participates());
        assert_eq!(d.skip_reason(), None);
        // NARROWED to the authorized subset — the private graph is not carried over.
        assert_eq!(
            d.authorized_graphs().iter().map(|g| g.as_str()).collect::<Vec<_>>(),
            vec![NOTES]
        );
    }

    #[test]
    fn source_with_no_readable_graph_is_skipped() {
        let store = store();
        let src = SourceDescriptor::new("pod-b").serving(node(PRIVATE));
        let d = store.authorize_source(&alice(), Mode::Read, &src);
        assert!(!d.participates());
        assert_eq!(d.skip_reason(), Some(SkipReason::NoAuthorizedGraph));
        assert!(d.authorized_graphs().is_empty());
    }

    #[test]
    fn source_declaring_no_graphs_is_skipped_without_consulting_the_view() {
        let store = store();
        let d = store.authorize_source(&alice(), Mode::Read, &SourceDescriptor::new("pod-c"));
        assert!(!d.participates());
        assert_eq!(d.skip_reason(), Some(SkipReason::NoDeclaredGraphs));
    }

    #[test]
    fn decision_is_per_session_and_per_mode() {
        let store = store();
        let src = SourceDescriptor::new("pod-a").serving_all([node(NOTES), node(PUBLIC)]);
        // Anonymous sees only the public graph — it still participates, but narrowed.
        let anon = store.authorize_source(&Session::default(), Mode::Read, &src);
        assert!(anon.participates());
        assert_eq!(
            anon.authorized_graphs().iter().map(|g| g.as_str()).collect::<Vec<_>>(),
            vec![PUBLIC]
        );
        // No Write grant exists anywhere, so the same source is skipped for writes.
        let write = store.authorize_source(&alice(), Mode::Write, &src);
        assert!(!write.participates());
        assert_eq!(write.skip_reason(), Some(SkipReason::NoAuthorizedGraph));
    }

    #[test]
    fn reserved_space_session_is_skipped_even_for_a_public_graph() {
        let store = store();
        // A session impersonating a minted pair principal has an EMPTY accessible set
        // (`AuthIndex::accessible`'s reserved-encoding guard), so even the graph everyone
        // may read does not make its source participate.
        let forged = Session {
            agent: Some("urn:sparq:pair?agent=x&client=y"),
            client: None,
            issuer: None,
            now: None,
        };
        let src = SourceDescriptor::new("pod-a").serving(node(PUBLIC));
        assert!(store.authorize_source(&Session::default(), Mode::Read, &src).participates());
        let d = store.authorize_source(&forged, Mode::Read, &src);
        assert!(!d.participates());
        assert_eq!(d.skip_reason(), Some(SkipReason::NoAuthorizedGraph));
    }

    #[test]
    fn unmaterialized_store_skips_every_source() {
        let store = PodStore::new(Graph::load_dataset("", "nquads").unwrap());
        let src = SourceDescriptor::new("pod-a").serving(node(NOTES));
        let d = store.authorize_source(&alice(), Mode::Read, &src);
        assert!(!d.participates());
        assert_eq!(d.skip_reason(), Some(SkipReason::NoAuthorizedGraph));
    }

    #[test]
    fn authorize_sources_preserves_input_order_and_decides_each() {
        let store = store();
        let sources = [
            SourceDescriptor::new("pod-private").serving(node(PRIVATE)),
            SourceDescriptor::new("pod-notes").serving(node(NOTES)),
            SourceDescriptor::new("pod-empty"),
        ];
        let ds = store.authorize_sources(&alice(), Mode::Read, &sources);
        assert_eq!(
            ds.iter().map(|d| d.source_id()).collect::<Vec<_>>(),
            vec!["pod-private", "pod-notes", "pod-empty"]
        );
        assert_eq!(ds.iter().map(|d| d.participates()).collect::<Vec<_>>(), vec![false, true, false]);
        assert_eq!(ds[2].skip_reason(), Some(SkipReason::NoDeclaredGraphs));
    }

    #[test]
    fn authorized_graphs_are_sorted_and_deduplicated() {
        let store = store();
        let src = SourceDescriptor::new("pod-a")
            .serving_all([node(PUBLIC), node(NOTES), node(PUBLIC), node(NOTES)]);
        let d = store.authorize_source(&alice(), Mode::Read, &src);
        assert_eq!(
            d.authorized_graphs().iter().map(|g| g.as_str()).collect::<Vec<_>>(),
            vec![NOTES, PUBLIC] // ascending by IRI, each once
        );
    }
}
