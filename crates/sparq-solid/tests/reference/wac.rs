//! sq-t58w.7 — the **independent procedural WAC reference evaluator**.
//!
//! A from-scratch, hand-written reading of Solid Web Access Control
//! (<https://solidproject.org/TR/wac>) over the parsed N-Quad model — the second paradigm
//! the differential oracle diffs against the engine's N3-rules decider. It implements,
//! procedurally:
//!
//! - **ACL resolution (nearest-wins).** A resource `R`'s effective ACL is `R.acl` if it
//!   exists, else the ACL of the **nearest** ancestor container — a closer ACL fully
//!   SHADOWS the ones above it (the WAC difference from ACP's cumulative inheritance).
//! - **`acl:accessTo` vs `acl:default`.** Within the effective ACL, `accessTo <R>`
//!   authorizations apply to `R` itself (only when the ACL is `R`'s own); `default <C>`
//!   authorizations apply to `C`'s members (its IRI-structural descendants), not to `C`.
//! - **Access subjects.** `acl:agent` (a WebID), `acl:agentClass foaf:Agent` (public, every
//!   requestor incl. anonymous), `acl:agentClass acl:AuthenticatedAgent` (any authenticated
//!   requestor), `acl:agentGroup` via `vcard:hasMember` (every group member), and
//!   `acl:origin` (the grant is a (user, application) pair — the agent matches only through
//!   that client).
//! - **The mode set.** `acl:Read`/`Write`/`Append`/`Control`, independent.
//! - **Control governs the `.acl` document.** A `Control` grant on `R` is Read+Write on
//!   `R.acl` (and Control on `R`), NOT read access to `R`.
//! - **Fail-closed.** A resource with no applicable ACL grants nobody.
//!
//! WAC has no deny — access is the union of all applicable grants, narrowed only by the
//! nearest-ACL resolution. This code shares NO logic with `materialize.rs` / `rules/*.n3`.

use super::{
    ancestors_nearest_first, is_structural_descendant, parse_nquads, Obj, Quad, RefDecision,
    RefMode,
};
use std::collections::{BTreeMap, BTreeSet};

const ACL: &str = "http://www.w3.org/ns/auth/acl#";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const FOAF_AGENT: &str = "http://xmlns.com/foaf/0.1/Agent";
const VCARD_MEMBER: &str = "http://www.w3.org/2006/vcard/ns#hasMember";
const AUTHENTICATED_AGENT: &str = "http://www.w3.org/ns/auth/acl#AuthenticatedAgent";
const ACL_SUFFIX: &str = ".acl";

/// A request the reference evaluator decides: who (agent WebID, `None` = anonymous),
/// through what client (`None` = no client presented), in which mode, on which resource.
#[derive(Debug, Clone)]
pub struct Request<'a> {
    pub agent: Option<&'a str>,
    pub client: Option<&'a str>,
    pub mode: RefMode,
    pub resource: &'a str,
}

/// One parsed `acl:Authorization`: its target sets (`accessTo`/`default`), access subjects,
/// and modes. Independent of the engine's representation.
#[derive(Debug, Default, Clone)]
struct Authorization {
    access_to: BTreeSet<String>,
    default_for: BTreeSet<String>,
    agents: BTreeSet<String>,
    agent_classes: BTreeSet<String>,
    agent_groups: BTreeSet<String>,
    origins: BTreeSet<String>,
    modes: BTreeSet<String>, // acl: mode IRIs
}

/// The independent WAC model assembled from the corpus N-Quads.
#[derive(Debug, Default)]
pub struct WacModel {
    /// ACL graph IRI (`R.acl`) → the authorizations declared in it.
    acls: BTreeMap<String, Vec<Authorization>>,
    /// group IRI → its members (from `vcard:hasMember`).
    groups: BTreeMap<String, BTreeSet<String>>,
    /// Every concrete resource graph present (non-control, non-group document graphs),
    /// PLUS every structural container prefix (containers exist as inheritance anchors).
    resources: BTreeSet<String>,
    /// The set of ACL graph IRIs that exist (so nearest-ACL resolution can test ancestors).
    acl_graphs: BTreeSet<String>,
}

impl WacModel {
    /// Parse a scenario's raw N-Quads into the independent WAC model (no engine code).
    ///
    /// # Errors
    ///
    /// Propagates a parse failure from [`parse_nquads`] (fail-closed at the oracle).
    pub fn from_nquads(src: &str) -> Result<WacModel, String> {
        let quads = parse_nquads(src)?;
        Ok(Self::from_quads(&quads))
    }

    fn from_quads(quads: &[Quad]) -> WacModel {
        let mut model = WacModel::default();
        // Per-authorization scratch keyed by the authorization subject IRI within its graph.
        let mut auth_scratch: BTreeMap<(String, String), Authorization> = BTreeMap::new();

        // First pass: collect resources, ACL graphs, group docs, and per-auth attributes.
        for q in quads {
            let g = q.graph.as_str();
            if g.ends_with(ACL_SUFFIX) {
                model.acl_graphs.insert(g.to_owned());
                Self::absorb_acl_triple(&mut auth_scratch, q);
            } else {
                // A non-control graph. It might be a resource (its document placeholder) or a
                // group document (`vcard:hasMember`). Group membership lives in its own graph.
                if q.predicate == VCARD_MEMBER {
                    if let Obj::Iri(member) = &q.object {
                        model
                            .groups
                            .entry(q.subject.clone())
                            .or_default()
                            .insert(member.clone());
                    }
                } else {
                    // Any other triple in a non-control graph marks the graph as a resource.
                    model.resources.insert(g.to_owned());
                }
            }
        }

        // Fold the per-auth scratch into the model, keyed by ACL graph.
        for ((graph, _auth), authz) in auth_scratch {
            model.acls.entry(graph).or_default().push(authz);
        }

        // Containers are inheritance anchors even without their own graph: add every
        // structural prefix of a known resource (and of each ACL graph's governed resource).
        let seeds: Vec<String> = model
            .resources
            .iter()
            .cloned()
            .chain(
                model
                    .acl_graphs
                    .iter()
                    .map(|a| a[..a.len() - ACL_SUFFIX.len()].to_owned()),
            )
            .collect();
        for r in seeds {
            for anc in ancestors_nearest_first(&r) {
                model.resources.insert(anc);
            }
            model.resources.insert(r);
        }

        model
    }

    fn absorb_acl_triple(scratch: &mut BTreeMap<(String, String), Authorization>, q: &Quad) {
        let key = (q.graph.clone(), q.subject.clone());
        let local = match q.predicate.strip_prefix(ACL) {
            Some(l) => l,
            None => {
                // rdf:type acl:Authorization — confirms this subject is an authorization,
                // but carries no attribute. Touch the scratch entry so an empty auth still
                // exists (harmless; it grants nobody).
                if q.predicate == RDF_TYPE {
                    scratch.entry(key).or_default();
                }
                return;
            }
        };
        let entry = scratch.entry(key).or_default();
        match (local, &q.object) {
            ("accessTo", Obj::Iri(o)) => {
                entry.access_to.insert(o.clone());
            }
            ("default", Obj::Iri(o)) => {
                entry.default_for.insert(o.clone());
            }
            ("agent", Obj::Iri(o)) => {
                entry.agents.insert(o.clone());
            }
            ("agentClass", Obj::Iri(o)) => {
                entry.agent_classes.insert(o.clone());
            }
            ("agentGroup", Obj::Iri(o)) => {
                entry.agent_groups.insert(o.clone());
            }
            ("origin", Obj::Iri(o)) => {
                entry.origins.insert(o.clone());
            }
            ("mode", Obj::Iri(o)) => {
                entry.modes.insert(o.clone());
            }
            _ => {}
        }
    }

    /// Decide a request, procedurally, per the WAC spec. Returns
    /// [`RefDecision::Allow`]/[`RefDecision::Deny`].
    pub fn decide(&self, req: &Request) -> RefDecision {
        // Control governs the resource's `.acl` document, not the resource. So a request on a
        // graph that is itself an ACL document (`X.acl`) for Read/Write is granted iff the
        // requestor holds Control on `X`. (Control on `X.acl` itself is not modelled by the
        // corpus.)
        if let Some(target) = req.resource.strip_suffix(ACL_SUFFIX) {
            match req.mode {
                RefMode::Read | RefMode::Write => {
                    // Read/Write of the .acl ⇔ Control of the resource it governs.
                    let control_req = Request {
                        agent: req.agent,
                        client: req.client,
                        mode: RefMode::Control,
                        resource: target,
                    };
                    return self.decide_resource(&control_req);
                }
                _ => {}
            }
        }
        self.decide_resource(req)
    }

    /// Decide a request against the resource's effective (nearest) ACL.
    fn decide_resource(&self, req: &Request) -> RefDecision {
        let Some((acl_graph, is_own)) = self.effective_acl(req.resource) else {
            return RefDecision::Deny; // no applicable ACL → fail-closed.
        };
        let governed = &acl_graph[..acl_graph.len() - ACL_SUFFIX.len()];
        let Some(auths) = self.acls.get(&acl_graph) else {
            return RefDecision::Deny;
        };
        for authz in auths {
            if !Self::auth_has_mode(authz, req.mode) {
                continue;
            }
            // Does this authorization APPLY to `req.resource`?
            let applies = if is_own {
                // The resource's own ACL: `accessTo R` applies to R. `default R` applies to
                // members of R (R itself is not a member of itself).
                authz.access_to.contains(req.resource)
            } else {
                // Inherited from the nearest ancestor container `governed`: `default
                // governed` applies to the member `req.resource` (a descendant of governed).
                authz.default_for.contains(governed)
                    && is_structural_descendant(governed, req.resource)
            };
            if !applies {
                continue;
            }
            if self.auth_matches_subject(authz, req) {
                return RefDecision::Allow;
            }
        }
        RefDecision::Deny
    }

    /// Resolve the nearest-wins effective ACL for `resource`: `resource.acl` if it exists
    /// (returned with `is_own = true`), else the ACL of the nearest ancestor container
    /// (`is_own = false`). `None` if no ancestor (or the resource) has an ACL — fail-closed.
    fn effective_acl(&self, resource: &str) -> Option<(String, bool)> {
        let own = format!("{resource}{ACL_SUFFIX}");
        if self.acl_graphs.contains(&own) {
            return Some((own, true));
        }
        for anc in ancestors_nearest_first(resource) {
            let candidate = format!("{anc}{ACL_SUFFIX}");
            if self.acl_graphs.contains(&candidate) {
                return Some((candidate, false));
            }
        }
        None
    }

    fn auth_has_mode(authz: &Authorization, mode: RefMode) -> bool {
        authz
            .modes
            .iter()
            .filter_map(|m| RefMode::from_acl_iri(m))
            .any(|m| m == mode)
    }

    /// Does this authorization's access-subject set match the request's principal?
    ///
    /// WAC subject semantics (disjunction over subjects), with the `acl:origin` (user,
    /// application) pair handled procedurally: an authorization carrying an `acl:origin`
    /// grants its named agents ONLY when the session presents that origin/client.
    fn auth_matches_subject(&self, authz: &Authorization, req: &Request) -> bool {
        // `acl:agentClass foaf:Agent` — public, grants every requestor incl. anonymous.
        if authz.agent_classes.contains(FOAF_AGENT) {
            return true;
        }
        // `acl:agentClass acl:AuthenticatedAgent` — any authenticated requestor.
        if authz.agent_classes.contains(AUTHENTICATED_AGENT) && req.agent.is_some() {
            return true;
        }

        // The agent half: the request's WebID is in this authorization's named agents, or a
        // member of one of its groups.
        let Some(agent) = req.agent else {
            // anonymous: only the public class (handled above) can grant; named-agent /
            // group / origin subjects all require an agent.
            return false;
        };
        let named = authz.agents.contains(agent);
        let in_group = authz
            .agent_groups
            .iter()
            .any(|grp| self.groups.get(grp).is_some_and(|m| m.contains(agent)));
        let agent_ok = named || in_group;
        if !agent_ok {
            return false;
        }

        // The origin half: if the authorization constrains origin, the session must present a
        // matching client (the (user, application) pair). No `acl:origin` ⇒ any client.
        if authz.origins.is_empty() {
            true
        } else {
            req.client.is_some_and(|c| authz.origins.contains(c))
        }
    }
}

/// Decide one request against a freshly-parsed model for `nquads` (the per-call convenience
/// used by the oracle; for many requests over one scenario, build a [`WacModel`] once).
///
/// # Errors
///
/// Propagates a corpus parse failure (fail-closed at the oracle).
pub fn decide(nquads: &str, req: &Request) -> Result<RefDecision, String> {
    Ok(WacModel::from_nquads(nquads)?.decide(req))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req<'a>(
        agent: Option<&'a str>,
        client: Option<&'a str>,
        mode: RefMode,
        resource: &'a str,
    ) -> Request<'a> {
        Request {
            agent,
            client,
            mode,
            resource,
        }
    }

    #[test]
    fn agent_access_to_grants_only_named() {
        let n = "\
<https://pod.ex/d1.acl#auth0> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/d1.acl> .
<https://pod.ex/d1.acl#auth0> <http://www.w3.org/ns/auth/acl#accessTo> <https://pod.ex/d1> <https://pod.ex/d1.acl> .
<https://pod.ex/d1.acl#auth0> <http://www.w3.org/ns/auth/acl#agent> <https://alice.ex/#me> <https://pod.ex/d1.acl> .
<https://pod.ex/d1.acl#auth0> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/d1.acl> .
<https://pod.ex/d1#it> <https://ex.dev/ns#prop> \"x\" <https://pod.ex/d1> .
";
        let m = WacModel::from_nquads(n).unwrap();
        assert_eq!(
            m.decide(&req(
                Some("https://alice.ex/#me"),
                None,
                RefMode::Read,
                "https://pod.ex/d1"
            )),
            RefDecision::Allow
        );
        assert_eq!(
            m.decide(&req(
                Some("https://bob.ex/#me"),
                None,
                RefMode::Read,
                "https://pod.ex/d1"
            )),
            RefDecision::Deny
        );
        assert_eq!(
            m.decide(&req(None, None, RefMode::Read, "https://pod.ex/d1")),
            RefDecision::Deny
        );
    }
}
