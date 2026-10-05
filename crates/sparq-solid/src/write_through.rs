//! [OPUS-4.8] issue #992 FR-3 (sq-snopa.5) — the AUTHORITATIVE ACL write-through.
//!
//! A Solid LDP server treats a `PUT`/`DELETE` to a `.acl`/`.acr` document as a change to
//! the access-control rules: the new rule text becomes the storage of record, and the
//! materialized authorization view (`<urn:sparq:auth>`) must be rebuilt so the change
//! takes effect immediately. This module is the storage-layer primitive for that:
//! [`crate::PodStore::put_acl`] / [`crate::PodStore::delete_acl`] update the access-control
//! GRAPH and re-materialize the auth view as ONE fail-closed, ATOMIC unit — keeping SPARQ
//! the source of truth (the auth view is always a pure function of the current `.acl`/`.acr`
//! graphs, never a stale snapshot).
//!
//! # Where this sits relative to [`crate::PodStore::update_as`]
//!
//! [`crate::PodStore::update_as`] is the SESSION-authorized write path: it checks the
//! actor's `acl:Control` (via the same auth view) before applying a SPARQL Update that
//! happens to touch an `.acl`/`.acr` graph, then re-materializes. `put_acl`/`delete_acl`
//! are the lower-level STORAGE primitive the server calls AFTER it has authorized the
//! request through its own pipeline (e.g. [`crate::PodStore::decide`] with
//! [`crate::Mode::Control`]): they take the new document content directly and replace the
//! whole control graph, rather than expressing the edit as a SPARQL `DELETE/INSERT`. They
//! perform NO session authorization themselves — authorization is the caller's job and
//! `update_as` remains the session-checked path. Use `put_acl`/`delete_acl` when the server
//! has the new `.acl` body in hand (the LDP `PUT` shape) and wants an authoritative,
//! atomic replace + re-materialize.
//!
//! # Atomicity (all-or-nothing)
//!
//! The two steps — swap the control graph, rebuild the auth view — succeed together or
//! leave the store byte-for-byte as it was:
//!
//! 1. the supplied content is PARSED first (a malformed document is rejected before
//!    anything is mutated);
//! 2. the prior control-graph slot is captured, then the new content is swapped in (or the
//!    slot removed, for `delete_acl`);
//! 3. the auth view is re-materialized. On success the store reflects the new rules.
//! 4. **on any re-materialization error** (e.g. the new ACL names a reserved-encoding
//!    principal) the captured prior slot is RESTORED and the auth view rebuilt from it, so
//!    the call leaves the store exactly as it found it and returns `Err` — never a control
//!    graph updated without its matching auth view, and never a partial write.
//!
//! # Fail-closed
//!
//! Every error path leaves the PRIOR (already-validated) authorization in force, never a
//! wider one: a rejected content parse, a reserved-encoding principal, or a non-control
//! target IRI all return `Err` with the previous auth view intact. A successful `delete_acl`
//! REMOVES grants (it can only narrow access — the deleted rules no longer grant anything),
//! and a successful `put_acl` replaces the old rules wholesale with the new ones.

use crate::loader::{ACL_SUFFIX, ACR_SUFFIX};
use crate::PodStore;
use oxrdf::{NamedNode, Term};
use sparq_core::Graph;

/// What a successful [`crate::PodStore::put_acl`] / [`crate::PodStore::delete_acl`] did, for
/// the server's audit log / response. The auth view has already been re-materialized when
/// this is returned. [OPUS-4.8] sq-snopa.5.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AclWriteOutcome {
    /// The control-document IRI that was written (`put_acl`) or removed (`delete_acl`).
    pub acl: NamedNode,
    /// Whether a control graph of that IRI already existed before the call (a `PUT` that
    /// CREATED a new `.acl` vs one that REPLACED an existing one; a `delete_acl` of an
    /// absent graph is a no-op success with `existed == false`).
    pub existed: bool,
    /// Triples in the control graph AFTER the write (0 for `delete_acl`).
    pub triples: usize,
}

/// Whether `iri` is an access-control document (`.acl`/`.acr`) by naming convention — the
/// only graphs `put_acl`/`delete_acl` accept (writing a content graph is the data path,
/// not the access-control path).
fn is_control_graph(iri: &str) -> bool {
    iri.ends_with(ACL_SUFFIX) || iri.ends_with(ACR_SUFFIX)
}

/// The number of triples a sub-graph currently holds.
fn graph_len(g: &Graph) -> usize {
    let pat: sparq_core::store::Pattern = [None, None, None];
    g.store.scan(&pat).rows.len()
}

impl PodStore {
    /// [OPUS-4.8] issue #992 FR-3 (sq-snopa.5) — authoritatively write an access-control
    /// document and re-materialize the auth view, ATOMICALLY (WAC pods).
    ///
    /// Replaces the `<acl_iri>` control graph with the parsed `content` and rebuilds
    /// `<urn:sparq:auth>` from the updated `.acl` graphs in ONE fail-closed unit — the LDP
    /// `PUT /resource.acl` storage primitive. `acl_iri` MUST be an `.acl`/`.acr` document
    /// IRI (writing a content graph is the data path — [`PodStore::update_as`] — not this
    /// one). `content` is parsed in `format` (`"turtle"`, `"ntriples"`, …) as the FULL new
    /// body of the document: the old rules are replaced wholesale, exactly as an LDP `PUT`
    /// replaces a resource.
    ///
    /// This is the AUTHORITATIVE write-through: the auth view becomes a pure function of the
    /// new `.acl` content, so SPARQ stays the source of truth — no stale grant survives a
    /// rule change. It performs NO session authorization (the server authorizes the request
    /// through its own pipeline, e.g. [`PodStore::decide`] with [`Mode::Control`](crate::Mode);
    /// the session-checked write path is [`PodStore::update_as`]).
    ///
    /// # Atomicity & fail-closed
    ///
    /// All-or-nothing: the content is parsed FIRST (a malformed document is rejected before
    /// anything mutates), then the graph is swapped and the view rebuilt. If
    /// re-materialization fails (e.g. the new ACL names a reserved-encoding principal) the
    /// PRIOR control graph + auth view are RESTORED and `Err` is returned — the store is left
    /// exactly as it was, and the previous (already-validated) authorization stays in force.
    ///
    /// For an **ACP** pod (`.acr` documents, three-stratum view) use
    /// [`PodStore::put_acl_acp`] so the re-materialization runs the ACP rules.
    ///
    /// # Errors
    ///
    /// `Err` if `acl_iri` is not an `.acl`/`.acr` document IRI, if `content` does not parse
    /// in `format`, or if re-materializing the resulting view fails (the store is rolled back
    /// to its prior state first).
    ///
    /// # Examples
    ///
    /// ```
    /// use sparq_solid::{Mode, PodStore, Session};
    ///
    /// // Start with an empty pod (no ACL anywhere) — every session fails closed.
    /// let nquads = "<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> \"hi\" <https://pod.ex/notes/n1> .\n";
    /// let mut store = PodStore::new(sparq_core::Graph::load_dataset(nquads, "nquads")?);
    /// store.materialize_wac()?;
    /// let alice = Session { agent: Some("https://alice.ex/card#me"), client: None, issuer: None, now: None };
    /// assert_eq!(store.accessible(&alice, Mode::Read).len(), 0); // no grant yet
    ///
    /// // PUT a root .acl granting alice Read by default — and re-materialize atomically.
    /// let acl = r#"
    /// <https://pod.ex/.acl#owner> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> .
    /// <https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> .
    /// <https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#agent> <https://alice.ex/card#me> .
    /// <https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> .
    /// "#;
    /// let out = store.put_acl("https://pod.ex/.acl", acl, "ntriples")?;
    /// assert!(!out.existed); // created
    /// // the change took effect immediately — no separate materialize_wac() call.
    /// assert_eq!(store.accessible(&alice, Mode::Read).len(), 2); // n1 + notes/ container
    /// # Ok::<(), String>(())
    /// ```
    pub fn put_acl(
        &mut self,
        acl_iri: &str,
        content: &str,
        format: &str,
    ) -> Result<AclWriteOutcome, String> {
        self.put_acl_inner(acl_iri, content, format, false)
    }

    /// [`PodStore::put_acl`] for **ACP** pods: identical atomic replace + re-materialize, but
    /// the view is rebuilt by the ACP rules ([`PodStore::materialize_acp`]) over the `.acr`
    /// graphs instead of the WAC ones. [OPUS-4.8] sq-snopa.5.
    pub fn put_acl_acp(
        &mut self,
        acl_iri: &str,
        content: &str,
        format: &str,
    ) -> Result<AclWriteOutcome, String> {
        self.put_acl_inner(acl_iri, content, format, true)
    }

    /// [OPUS-4.8] issue #992 FR-3 (sq-snopa.5) — authoritatively REMOVE an access-control
    /// document and re-materialize the auth view, ATOMICALLY (WAC pods).
    ///
    /// Removes the `<acl_iri>` control graph entirely and rebuilds `<urn:sparq:auth>` from the
    /// remaining `.acl` graphs — the LDP `DELETE /resource.acl` storage primitive. Deleting an
    /// ACL can only NARROW access: the removed rules no longer grant anything, so a resource
    /// that relied on this ACL falls back to its nearest ancestor container's ACL (or becomes
    /// un-protected, and every session fails closed on it). `acl_iri` MUST be an `.acl`/`.acr`
    /// document IRI. Deleting an ACL that does not exist is a no-op success
    /// (`existed == false`), still re-materializing so the caller need not branch.
    ///
    /// Like [`PodStore::put_acl`] this is the AUTHORITATIVE write-through (the auth view stays
    /// a pure function of the present `.acl` graphs) and performs NO session authorization. For
    /// an **ACP** pod use [`PodStore::delete_acl_acp`].
    ///
    /// # Atomicity & fail-closed
    ///
    /// All-or-nothing, exactly as [`PodStore::put_acl`]: the prior slot is captured, removed,
    /// then the view is rebuilt; on a re-materialization error the slot is restored and the
    /// prior auth view rebuilt, leaving the store untouched and returning `Err`. (Removing an
    /// ACL can only narrow grants, so a re-materialization error here is unusual — but the
    /// rollback keeps the contract uniform with `put_acl`.)
    ///
    /// # Errors
    ///
    /// `Err` if `acl_iri` is not an `.acl`/`.acr` document IRI, or if re-materializing after
    /// the removal fails (the store is rolled back first).
    ///
    /// # Examples
    ///
    /// ```
    /// use sparq_solid::{Mode, PodStore, Session};
    ///
    /// let nquads = r#"
    /// <https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> "hi" <https://pod.ex/notes/n1> .
    /// <https://pod.ex/.acl#owner> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .
    /// <https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> <https://pod.ex/.acl> .
    /// <https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#agent> <https://alice.ex/card#me> <https://pod.ex/.acl> .
    /// <https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .
    /// "#;
    /// let mut store = PodStore::new(sparq_core::Graph::load_dataset(nquads, "nquads")?);
    /// store.materialize_wac()?;
    /// let alice = Session { agent: Some("https://alice.ex/card#me"), client: None, issuer: None, now: None };
    /// assert_eq!(store.accessible(&alice, Mode::Read).len(), 2); // granted by the root .acl
    ///
    /// // DELETE the root .acl — alice's grant disappears, the view is rebuilt atomically.
    /// let out = store.delete_acl("https://pod.ex/.acl")?;
    /// assert!(out.existed);
    /// assert_eq!(store.accessible(&alice, Mode::Read).len(), 0); // un-protected ⇒ fail closed
    /// # Ok::<(), String>(())
    /// ```
    pub fn delete_acl(&mut self, acl_iri: &str) -> Result<AclWriteOutcome, String> {
        self.delete_acl_inner(acl_iri, false)
    }

    /// [`PodStore::delete_acl`] for **ACP** pods: identical atomic removal + re-materialize,
    /// rebuilding the view with the ACP rules. [OPUS-4.8] sq-snopa.5.
    pub fn delete_acl_acp(&mut self, acl_iri: &str) -> Result<AclWriteOutcome, String> {
        self.delete_acl_inner(acl_iri, true)
    }

    /// Shared `put_acl` body — parse, swap, re-materialize, rollback-on-error.
    fn put_acl_inner(
        &mut self,
        acl_iri: &str,
        content: &str,
        format: &str,
        acp: bool,
    ) -> Result<AclWriteOutcome, String> {
        if !is_control_graph(acl_iri) {
            return Err(format!(
                "put_acl denied: <{}> is not an access-control document (must end in \
                 `{}` or `{}`) — write content graphs through `update_as`",
                acl_iri, ACL_SUFFIX, ACR_SUFFIX
            ));
        }
        let name = NamedNode::new(acl_iri)
            .map_err(|e| format!("put_acl denied: malformed ACL IRI <{}>: {}", acl_iri, e))?;
        // PARSE FIRST — a malformed document is rejected before anything mutates.
        let new_graph = Graph::load_str(content, format).map_err(|e| {
            format!("put_acl denied: ACL content for <{}> did not parse as {}: {}", acl_iri, format, e)
        })?;
        let new_len = graph_len(&new_graph);

        // Capture the prior slot (for rollback) and swap the new content in.
        let term = Term::NamedNode(name.clone());
        let prior = self.take_named_slot(&term);
        let existed = prior.is_some();
        self.graph.named.push((term.clone(), new_graph));

        // Re-materialize; roll back to the prior content on any error. [OPUS-4.8] sq-b7k7u
        // (issue #1571): the whole WAC/ACP view is still re-derived, but the session cache
        // uses diff-based invalidation — `reindex_with` diffs old vs new AuthIndex per-origin
        // and invalidates exactly the origins whose buckets changed ([SONNET-4.6] sq-b7k7u
        // fix). Every other pod's cached view stays warm if its grants are unaffected.
        // The rollback path stays a full clear (conservative).
        let scope = crate::ReindexScope::Origin;
        if let Err(e) = self.rematerialize_scoped(acp, scope) {
            self.restore_named_slot(&term, prior, acp);
            return Err(e);
        }
        Ok(AclWriteOutcome { acl: name, existed, triples: new_len })
    }

    /// Shared `delete_acl` body — capture, remove, re-materialize, rollback-on-error.
    fn delete_acl_inner(&mut self, acl_iri: &str, acp: bool) -> Result<AclWriteOutcome, String> {
        if !is_control_graph(acl_iri) {
            return Err(format!(
                "delete_acl denied: <{}> is not an access-control document (must end in \
                 `{}` or `{}`)",
                acl_iri, ACL_SUFFIX, ACR_SUFFIX
            ));
        }
        let name = NamedNode::new(acl_iri)
            .map_err(|e| format!("delete_acl denied: malformed ACL IRI <{}>: {}", acl_iri, e))?;
        let term = Term::NamedNode(name.clone());
        let prior = self.take_named_slot(&term);
        let existed = prior.is_some();

        // [OPUS-4.8] sq-b7k7u: use diff-based invalidation — reindex_with diffs old vs new
        // AuthIndex per-origin and invalidates exactly the origins whose buckets changed.
        let scope = crate::ReindexScope::Origin;
        if let Err(e) = self.rematerialize_scoped(acp, scope) {
            self.restore_named_slot(&term, prior, acp);
            return Err(e);
        }
        Ok(AclWriteOutcome { acl: name, existed, triples: 0 })
    }

    /// Remove and return the sub-graph currently stored under `name`, if any (so it can be
    /// swapped or restored). The reserved auth view is never a target here — these methods
    /// reject non-control IRIs up front.
    fn take_named_slot(&mut self, name: &Term) -> Option<Graph> {
        let pos = self.graph.named.iter().position(|(n, _)| n == name)?;
        Some(self.graph.named.swap_remove(pos).1)
    }

    /// Restore a captured prior slot after a failed re-materialization, then rebuild the
    /// auth view from the restored content so the store is left consistent with the PRIOR
    /// rules (the rollback path). The earlier swap already removed the new content; here we
    /// re-insert the old content (or leave the slot absent if there was none) and
    /// re-materialize from it.
    fn restore_named_slot(&mut self, name: &Term, prior: Option<Graph>, acp: bool) {
        // Drop whatever is currently in the slot (the new content that failed) and put the
        // prior content back.
        let _ = self.take_named_slot(name);
        if let Some(g) = prior {
            self.graph.named.push((name.clone(), g));
        }
        // Rebuild the auth view from the restored (prior) rules. This re-runs the SAME
        // materialization that previously succeeded for this content, so it is expected to
        // succeed; if it somehow fails the prior auth view left in place by the failed run
        // still stands (materialize leaves the previous view untouched on error) and the
        // index is rebuilt from it — fail-closed either way.
        let _ = self.rematerialize(acp);
    }

    /// Re-materialize the WAC or ACP view with a FULL session-cache clear (the rollback
    /// path uses this — conservative is correct when restoring prior rules).
    fn rematerialize(&mut self, acp: bool) -> Result<(), String> {
        if acp {
            self.materialize_acp().map(|_| ())
        } else {
            self.materialize_wac().map(|_| ())
        }
    }

    /// [OPUS-4.8] sq-b7k7u (issue #1571) — re-materialize the WAC or ACP view with
    /// `scope`-controlled session-cache invalidation. The successful `put_acl`/`delete_acl`
    /// path passes [`crate::ReindexScope::Origin`], which is **diff-based**: `reindex_with`
    /// diffs the old vs new `AuthIndex` per-origin and invalidates exactly the origins whose
    /// buckets changed — NOT merely the written ACL's own origin (a write at origin A can
    /// affect origin B's grants; the diff catches every such case). [FABLE-5] sq-vhhl0 doc
    /// sweep. ACP re-materializes with no provenance, exactly as the non-scoped
    /// `materialize_acp()` the write path used before — and, [SONNET-4.6] sq-ysv3u, with no
    /// verified credentials either, so an `acp:vc` matcher stays fail-closed across an ACL
    /// write exactly as it is on a plain `materialize_acp()`.
    fn rematerialize_scoped(&mut self, acp: bool, scope: crate::ReindexScope) -> Result<(), String> {
        if acp {
            self.materialize_acp_with_scoped(
                &crate::AccessProvenance::new(),
                &crate::VerifiedCredentials::new(),
                scope,
            )
            .map(|_| ())
        } else {
            self.materialize_wac_scoped(scope).map(|_| ())
        }
    }
}
