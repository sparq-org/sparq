//! [OPUS-4.8] sq-t58w.7 — the **independent procedural ACP reference evaluator**.
//!
//! A from-scratch, hand-written reading of Solid Access Control Policy
//! (<https://solidproject.org/TR/acp>) over the parsed N-Quad model — the second paradigm
//! the differential oracle diffs against the engine's N3-rules decider. It implements,
//! procedurally:
//!
//! - **Matchers.** `acp:anyOf` (disjunction — at least one matches), `acp:allOf`
//!   (conjunction — all match), `acp:noneOf` (the "except" carve-out — none may match). A
//!   matcher with both `acp:agent` and `acp:client` requires BOTH dimensions; the special
//!   agents `acp:PublicAgent` (every requestor) and `acp:AuthenticatedAgent` (any
//!   authenticated requestor) are recognised.
//! - **Policies, allow/deny + deny-overrides.** A satisfied policy contributes its
//!   `acp:allow` modes as grants and its `acp:deny` modes as denials; deny overrides allow
//!   for the same (request, mode, resource).
//! - **Cumulative inheritance.** `acp:accessControl` applies to the resource itself;
//!   `acp:memberAccessControl` applies to members (IRI-structural descendants), accruing
//!   from EVERY ancestor (the ACP difference from WAC's nearest-wins shadowing).
//! - **Fail-closed.** A resource with no applicable, satisfied allow policy is denied.
//!
//! This code shares NO logic with `materialize.rs` / `rules/*.n3`: the policy-satisfaction
//! and inheritance logic is written by hand over a plain model parsed by the independent
//! N-Quad reader.

use super::{
    ancestors_nearest_first, is_structural_descendant, parse_nquads, Obj, Quad, RefDecision,
    RefMode,
};
use std::collections::{BTreeMap, BTreeSet};

const ACP: &str = "http://www.w3.org/ns/solid/acp#";
const ACL: &str = "http://www.w3.org/ns/auth/acl#";
const PUBLIC_AGENT: &str = "http://www.w3.org/ns/solid/acp#PublicAgent";
const AUTHENTICATED_AGENT: &str = "http://www.w3.org/ns/solid/acp#AuthenticatedAgent";
const ACR_SUFFIX: &str = ".acr";

/// A request the reference evaluator decides (the ACP twin of the WAC one).
#[derive(Debug, Clone)]
pub struct Request<'a> {
    pub agent: Option<&'a str>,
    pub client: Option<&'a str>,
    pub mode: RefMode,
    pub resource: &'a str,
}

/// One matcher: its `acp:agent` and `acp:client` accept-sets. A matcher with an agent set
/// constrains the agent dimension; with a client set, the client dimension; with both, both.
#[derive(Debug, Default, Clone)]
struct Matcher {
    agents: BTreeSet<String>,
    clients: BTreeSet<String>,
}

impl Matcher {
    fn is_empty(&self) -> bool {
        self.agents.is_empty() && self.clients.is_empty()
    }

    /// Does this matcher match the request? A constrained dimension must be satisfied; an
    /// unconstrained one (empty set) imposes no requirement on that dimension.
    fn matches(&self, req: &Request) -> bool {
        let agent_ok = if self.agents.is_empty() {
            true
        } else {
            self.agents.iter().any(|a| Self::agent_matches(a, req))
        };
        let client_ok = if self.clients.is_empty() {
            true
        } else {
            req.client.is_some_and(|c| self.clients.contains(c))
        };
        // A matcher with no attributes at all matches nothing (fail-closed).
        agent_ok && client_ok && !self.is_empty()
    }

    fn agent_matches(spec: &str, req: &Request) -> bool {
        match spec {
            PUBLIC_AGENT => true,
            AUTHENTICATED_AGENT => req.agent.is_some(),
            concrete => req.agent == Some(concrete),
        }
    }
}

/// One policy: its allow/deny mode sets and its `allOf`/`anyOf`/`noneOf` matcher lists.
#[derive(Debug, Default, Clone)]
struct Policy {
    allow: BTreeSet<String>, // acl: mode IRIs
    deny: BTreeSet<String>,
    all_of: Vec<String>, // matcher IRIs
    any_of: Vec<String>,
    none_of: Vec<String>,
}

/// One access control: whether it is member-scoped, and the policies it applies.
#[derive(Debug, Default, Clone)]
struct AccessControl {
    /// `true` for `acp:memberAccessControl` (applies to descendants), `false` for
    /// `acp:accessControl` (applies to the resource itself).
    member: bool,
    policies: Vec<String>, // policy IRIs
}

/// The independent ACP model assembled from the corpus N-Quads.
#[derive(Debug, Default)]
pub struct AcpModel {
    /// resource IRI → its access controls (from `acp:resource` + `acp:accessControl` /
    /// `acp:memberAccessControl`).
    controls_by_resource: BTreeMap<String, Vec<AccessControl>>,
    policies: BTreeMap<String, Policy>,
    matchers: BTreeMap<String, Matcher>,
    /// Every concrete protected resource graph present (so containment anchors are known).
    resources: BTreeSet<String>,
}

impl AcpModel {
    /// Parse a scenario's raw N-Quads into the independent ACP model (no engine code).
    ///
    /// # Errors
    ///
    /// Propagates a parse failure from [`parse_nquads`] (fail-closed at the oracle).
    pub fn from_nquads(src: &str) -> Result<AcpModel, String> {
        let quads = parse_nquads(src)?;
        Ok(Self::from_quads(&quads))
    }

    fn from_quads(quads: &[Quad]) -> AcpModel {
        let mut model = AcpModel::default();
        // ACR-graph → (resource it binds); a control node carries its scope (member?) and
        // its applied policies. We resolve these in two passes since N-Quads are unordered.
        let mut acr_resource: BTreeMap<String, String> = BTreeMap::new(); // acr graph → resource
                                                                          // control IRI → (member?, policies); built directly into model.controls below.
        let mut control_member: BTreeMap<String, bool> = BTreeMap::new();
        let mut control_policies: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut control_graph: BTreeMap<String, String> = BTreeMap::new(); // control IRI → acr graph

        for q in quads {
            let g = q.graph.as_str();
            if g.ends_with(ACR_SUFFIX) {
                self_absorb_acr(
                    q,
                    &mut acr_resource,
                    &mut control_member,
                    &mut control_policies,
                    &mut control_graph,
                    &mut model,
                );
            } else {
                // A non-control graph: any triple marks it a concrete resource.
                model.resources.insert(g.to_owned());
            }
        }

        // Stitch controls onto their bound resource. A control in ACR graph `acr` applies to
        // the resource `acr_resource[acr]`.
        for (control, member) in &control_member {
            let Some(acr) = control_graph.get(control) else {
                continue;
            };
            let Some(resource) = acr_resource.get(acr) else {
                continue;
            };
            let policies = control_policies.get(control).cloned().unwrap_or_default();
            model
                .controls_by_resource
                .entry(resource.clone())
                .or_default()
                .push(AccessControl {
                    member: *member,
                    policies,
                });
            // The bound resource is itself a known resource / inheritance anchor.
            model.resources.insert(resource.clone());
        }

        // Containers are inheritance anchors even without their own graph: add structural
        // prefixes of every known resource.
        let seeds: Vec<String> = model.resources.iter().cloned().collect();
        for r in seeds {
            for anc in ancestors_nearest_first(&r) {
                model.resources.insert(anc);
            }
        }

        model
    }

    /// Decide a request, procedurally, per the ACP spec (deny-overrides over the cumulative
    /// applicable policy set).
    pub fn decide(&self, req: &Request) -> RefDecision {
        // [GPT-5.6] sq-61uvs: an ACR is a control document, not an ordinary member
        // resource. A member Read/Write policy must not disclose it. ACP has no WAC-style
        // Control-to-control-document translation, so this corpus surface fails closed.
        if req.resource.ends_with(ACR_SUFFIX)
            && matches!(req.mode, RefMode::Read | RefMode::Write)
        {
            return RefDecision::Deny;
        }

        let mut any_allow = false;
        let mut any_deny = false;

        for (resource, controls) in &self.controls_by_resource {
            for ctl in controls {
                // Does this control apply to `req.resource`?
                let applies = if ctl.member {
                    // memberAccessControl: applies to the resource's members (descendants).
                    is_structural_descendant(resource, req.resource)
                } else {
                    // accessControl: applies to the resource itself.
                    resource == req.resource
                };
                if !applies {
                    continue;
                }
                for pol_iri in &ctl.policies {
                    let Some(pol) = self.policies.get(pol_iri) else {
                        continue;
                    };
                    if !self.policy_satisfied(pol, req) {
                        continue;
                    }
                    if Self::policy_has_mode(&pol.allow, req.mode) {
                        any_allow = true;
                    }
                    if Self::policy_has_mode(&pol.deny, req.mode) {
                        any_deny = true;
                    }
                }
            }
        }

        // Deny overrides allow; an absent allow is fail-closed.
        if any_deny || !any_allow {
            RefDecision::Deny
        } else {
            RefDecision::Allow
        }
    }

    /// Is `policy` satisfied for `req`? Every `allOf` matcher matches AND (no `anyOf`, or at
    /// least one `anyOf` matches) AND no `noneOf` matcher matches. A positive matcher is required.
    fn policy_satisfied(&self, pol: &Policy, req: &Request) -> bool {
        // [GPT-6] ACP §6.4: noneOf alone cannot satisfy a policy.
        // https://solidproject.org/TR/acp#satisfied-policy
        if pol.all_of.is_empty() && pol.any_of.is_empty() {
            return false;
        }
        let all_of_ok = pol
            .all_of
            .iter()
            .all(|m| self.matcher(m).is_some_and(|mat| mat.matches(req)));
        if !all_of_ok {
            return false;
        }
        let any_of_ok = pol.any_of.is_empty()
            || pol
                .any_of
                .iter()
                .any(|m| self.matcher(m).is_some_and(|mat| mat.matches(req)));
        if !any_of_ok {
            return false;
        }
        let none_of_blocks = pol
            .none_of
            .iter()
            .any(|m| self.matcher(m).is_some_and(|mat| mat.matches(req)));
        !none_of_blocks
    }

    fn matcher(&self, iri: &str) -> Option<&Matcher> {
        self.matchers.get(iri)
    }

    fn policy_has_mode(modes: &BTreeSet<String>, mode: RefMode) -> bool {
        modes
            .iter()
            .filter_map(|m| RefMode::from_acl_iri(m))
            .any(|m| m == mode)
    }
}

/// Absorb one ACR-graph triple into the in-progress model + scratch maps. A free function
/// (not a method) so it can take disjoint `&mut` borrows of the scratch maps + model.
fn self_absorb_acr(
    q: &Quad,
    acr_resource: &mut BTreeMap<String, String>,
    control_member: &mut BTreeMap<String, bool>,
    control_policies: &mut BTreeMap<String, Vec<String>>,
    control_graph: &mut BTreeMap<String, String>,
    model: &mut AcpModel,
) {
    let g = q.graph.clone();
    let local = match q.predicate.strip_prefix(ACP) {
        Some(l) => l,
        None => {
            // An `acp:allow`/`acp:deny` predicate is in ACP space; modes are acl: IRIs in
            // the object, predicate in acp:. Anything else (e.g. a stray triple) is ignored.
            return;
        }
    };
    match (local, &q.object) {
        ("resource", Obj::Iri(o)) => {
            acr_resource.insert(g, o.clone());
        }
        ("accessControl", Obj::Iri(ctl)) => {
            control_member.insert(ctl.clone(), false);
            control_graph.insert(ctl.clone(), g);
        }
        ("memberAccessControl", Obj::Iri(ctl)) => {
            control_member.insert(ctl.clone(), true);
            control_graph.insert(ctl.clone(), g);
        }
        ("apply", Obj::Iri(pol)) => {
            control_policies
                .entry(q.subject.clone())
                .or_default()
                .push(pol.clone());
        }
        ("allow", Obj::Iri(mode)) => {
            model
                .policies
                .entry(q.subject.clone())
                .or_default()
                .allow
                .insert(mode.clone());
        }
        ("deny", Obj::Iri(mode)) => {
            model
                .policies
                .entry(q.subject.clone())
                .or_default()
                .deny
                .insert(mode.clone());
        }
        ("allOf", Obj::Iri(m)) => {
            model
                .policies
                .entry(q.subject.clone())
                .or_default()
                .all_of
                .push(m.clone());
        }
        ("anyOf", Obj::Iri(m)) => {
            model
                .policies
                .entry(q.subject.clone())
                .or_default()
                .any_of
                .push(m.clone());
        }
        ("noneOf", Obj::Iri(m)) => {
            model
                .policies
                .entry(q.subject.clone())
                .or_default()
                .none_of
                .push(m.clone());
        }
        ("agent", Obj::Iri(o)) => {
            model
                .matchers
                .entry(q.subject.clone())
                .or_default()
                .agents
                .insert(o.clone());
        }
        ("client", Obj::Iri(o)) => {
            model
                .matchers
                .entry(q.subject.clone())
                .or_default()
                .clients
                .insert(o.clone());
        }
        _ => {}
    }
    // The acl: mode constant on allow/deny is handled by the object branch above; the ACL
    // namespace itself is never a predicate in an ACR graph here.
    let _ = ACL;
}

/// Decide one request against a freshly-parsed model for `nquads` (per-call convenience).
///
/// # Errors
///
/// Propagates a corpus parse failure (fail-closed at the oracle).
pub fn decide(nquads: &str, req: &Request) -> Result<RefDecision, String> {
    Ok(AcpModel::from_nquads(nquads)?.decide(req))
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
    fn agent_anyof_grants_only_named() {
        let n = "\
<https://pod.ex/d1.acr> <http://www.w3.org/ns/solid/acp#resource> <https://pod.ex/d1> <https://pod.ex/d1.acr> .
<https://pod.ex/d1.acr> <http://www.w3.org/ns/solid/acp#accessControl> <https://pod.ex/d1.acr#ctl0> <https://pod.ex/d1.acr> .
<https://pod.ex/d1.acr#ctl0> <http://www.w3.org/ns/solid/acp#apply> <https://pod.ex/d1.acr#pol0> <https://pod.ex/d1.acr> .
<https://pod.ex/d1.acr#pol0> <http://www.w3.org/ns/solid/acp#allow> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/d1.acr> .
<https://pod.ex/d1.acr#pol0> <http://www.w3.org/ns/solid/acp#anyOf> <https://pod.ex/d1.acr#m0> <https://pod.ex/d1.acr> .
<https://pod.ex/d1.acr#m0> <http://www.w3.org/ns/solid/acp#agent> <https://alice.ex/#me> <https://pod.ex/d1.acr> .
<https://pod.ex/d1#it> <https://ex.dev/ns#prop> \"x\" <https://pod.ex/d1> .
";
        let m = AcpModel::from_nquads(n).unwrap();
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

    // [GPT-6] Independent fixtures for ACP §6.4's positive-matcher requirement.
    fn policy_fixture(conditions: &str) -> String {
        let acr = "https://pod.ex/d1.acr";
        format!(
            r#"<{acr}> <{ACP}resource> <https://pod.ex/d1> <{acr}> .
<{acr}> <{ACP}accessControl> <{acr}#control> <{acr}> .
<{acr}#control> <{ACP}apply> <{acr}#policy> <{acr}> .
<{acr}#policy> <{ACP}allow> <{ACL}Read> <{acr}> .
<{acr}#public> <{ACP}agent> <{PUBLIC_AGENT}> <{acr}> .
<https://pod.ex/d1#it> <https://ex.dev/ns#prop> "x" <https://pod.ex/d1> .
{conditions}"#
        )
    }

    #[test]
    fn policies_without_positive_matchers_never_grant() {
        let acr = "https://pod.ex/d1.acr";
        let none_of = format!("<{acr}#policy> <{ACP}noneOf> <{acr}#except> <{acr}> .\n");
        let nonmatching_exception =
            format!("{none_of}<{acr}#except> <{ACP}agent> <https://alice.ex/#me> <{acr}> .\n");
        for conditions in ["", &none_of, &nonmatching_exception] {
            let model = AcpModel::from_nquads(&policy_fixture(conditions)).unwrap();
            for agent in [None, Some("https://bob.ex/#me")] {
                assert_eq!(
                    model.decide(&req(agent, None, RefMode::Read, "https://pod.ex/d1")),
                    RefDecision::Deny,
                    "no positive matcher: {conditions}"
                );
            }
        }
    }

    #[test]
    fn positive_matcher_with_empty_exception_is_satisfied() {
        let acr = "https://pod.ex/d1.acr";
        for combinator in ["allOf", "anyOf"] {
            let positive = format!("<{acr}#policy> <{ACP}{combinator}> <{acr}#public> <{acr}> .\n");
            let empty_exception =
                format!("{positive}<{acr}#policy> <{ACP}noneOf> <{acr}#except> <{acr}> .\n");
            let matching_exception = format!(
                "{empty_exception}<{acr}#except> <{ACP}agent> <{PUBLIC_AGENT}> <{acr}> .\n"
            );
            for (conditions, expected) in [
                (&positive, RefDecision::Allow),
                (&empty_exception, RefDecision::Allow),
                (&matching_exception, RefDecision::Deny),
            ] {
                let model = AcpModel::from_nquads(&policy_fixture(conditions)).unwrap();
                assert_eq!(
                    model.decide(&req(None, None, RefMode::Read, "https://pod.ex/d1")),
                    expected
                );
            }
        }
    }
}
