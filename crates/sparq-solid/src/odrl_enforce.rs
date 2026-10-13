//! Request-time ODRL policies (issue #6743).
//!
//! The materializing bridge ([`crate::odrl_bridge`]) stores a frozen projection of one
//! decision, so it can only store a grant that nothing can end, deny every session on a
//! prohibited asset, and act only on assets someone materialized. Policies attached here
//! are instead evaluated for the **accessing session** on every access decision: its
//! agent is the party (and default recipient), its clock is the request time, and the
//! recorded asset memberships are the collection evidence.
//!
//! [`Mode::Read`] is **granted** when some attached policy's [`decide`] permits
//! `odrl:read`. The store applies a grant only to an eligible target: a graph that
//! exists now, that the last static WAC/ACP materialization covered (so every deny that
//! applies to it is in the view), whose static decision is authoritative, and on which
//! the auth view does not explicitly deny `Read`. So an ACP deny still wins, and a graph
//! no access-control document governs, or one created since, is never granted. No other
//! mode is granted: creating and changing resources stay with the static decision.
//!
//! A mode is **removed** when some attached prohibition applies (True or Unknown) to a
//! request for any ODRL action that maps to it, after any grant:
//!
//! - `odrl:read`, `display`, `present`, `print` and `play` map to [`Mode::Read`];
//! - `append`, `modify`, `delete` and `write` map to both [`Mode::Append`] and
//!   [`Mode::Write`], since either mode can change the resource.
//!
//! A request carries no party-membership evidence, so when a policy names an
//! `odrl:PartyCollection` a prohibition applies if it applies under any membership the
//! agent could have, and a permission grants only if it permits under every one: a
//! prohibition on a collection applies to every authenticated agent, as the
//! materializing bridge's denies do, one excluding a collection still applies to
//! non-members, and a permission for a collection's members grants nobody.
//!
//! [`Mode::Control`], access-control documents (`.acl`/`.acr`) and the reserved
//! `urn:sparq:` graphs are never touched, so an attached policy cannot lock an owner out
//! of their own rules. Policies whose `odrl:conflict` strategy `decide` cannot honour, and
//! policies with prohibitions naming more than four party collections, are refused at
//! attach time.
//!
//! Every [`crate::PodStore`] entry point that authorizes a session consults one gate:
//! the cached set behind `accessible`, the views, queries and `wac_allow`; point
//! `decide`, `decide_batch` and `decide_create`; and every update.

use crate::authindex::{Mode, Session};
use oxrdf::NamedNode;
use sparq_policy::{conflict_admissibility, decide, matched_prohibition, Request, ValidatedPolicy};
use std::collections::HashMap;
use std::sync::Arc;

const ODRL_NS: &str = "http://www.w3.org/ns/odrl/2/";
/// Reserved graph names (the auth view and its bookkeeping) are never ODRL targets.
const RESERVED_PREFIX: &str = "urn:sparq:";

const READ_ACTIONS: &[&str] = &["read", "display", "present", "print", "play"];
const CHANGE_ACTIONS: &[&str] = &["append", "modify", "delete", "write"];

/// The most party collections a policy with prohibitions may name: every subset of them
/// is evaluated per request, so a larger policy is refused at attach time.
const MAX_PARTY_COLLECTIONS: usize = 4;

/// Whether a prohibition of `policy` applies to `req`. No party-membership evidence
/// reaches a request, so when the policy names party collections the agent's
/// membership is unknown: the prohibition applies if it applies under any membership
/// the agent could have (member of any subset of those collections). `attach` keeps
/// that at most [`MAX_PARTY_COLLECTIONS`] collections.
fn prohibited(policy: &ValidatedPolicy, req: &Request, agent: Option<&str>) -> bool {
    if policy.prohibitions.is_empty() {
        return false;
    }
    if matched_prohibition(policy, req).is_some() {
        return true;
    }
    let Some(agent) = agent else { return false };
    let collections: Vec<&str> = policy.party_collections.iter().map(String::as_str).collect();
    if collections.is_empty() {
        return false;
    }
    (1u32..1 << collections.len()).any(|mask| {
        let member_of = collections
            .iter()
            .enumerate()
            .filter(|(i, _)| mask & (1 << i) != 0)
            .map(|(_, c)| (agent, *c));
        matched_prohibition(policy, &req.clone().with_party_memberships(member_of)).is_some()
    })
}

/// Whether `policy` permits `req` through [`decide`] whatever party memberships the
/// agent has. No membership evidence reaches a request, so when the policy names party
/// collections every membership the agent could have (each subset of them) must
/// permit; a policy naming more than [`MAX_PARTY_COLLECTIONS`] never grants.
fn permitted(policy: &ValidatedPolicy, req: &Request, agent: Option<&str>) -> bool {
    if policy.permissions.is_empty() {
        return false;
    }
    let collections: Vec<&str> = policy.party_collections.iter().map(String::as_str).collect();
    if collections.len() > MAX_PARTY_COLLECTIONS {
        return false;
    }
    if !decide(policy, req).allow {
        return false;
    }
    let Some(agent) = agent else { return true };
    (1u32..1 << collections.len()).all(|mask| {
        let member_of = collections
            .iter()
            .enumerate()
            .filter(|(i, _)| mask & (1 << i) != 0)
            .map(|(_, c)| (agent, *c));
        decide(policy, &req.clone().with_party_memberships(member_of)).allow
    })
}

/// The ODRL policies a [`crate::PodStore`] evaluates per request, with the asset
/// membership evidence (`asset odrl:partOf collection`) each request carries.
#[derive(Debug, Clone, Default)]
pub struct OdrlEnforcement {
    policies: Vec<Arc<ValidatedPolicy>>,
    /// `asset -> collections`. A rule's target test only consults the edges of the
    /// request's own target, so a request carries exactly that asset's edges.
    asset_memberships: HashMap<String, Vec<String>>,
}

impl OdrlEnforcement {
    /// Whether no policy is attached (the store then behaves exactly as without this
    /// layer).
    pub fn is_empty(&self) -> bool {
        self.policies.is_empty()
    }

    /// Attach `policy`. Refused when its declared `odrl:conflict` strategy is one
    /// `decide` cannot honour, or when it has prohibitions and names more than
    /// [`MAX_PARTY_COLLECTIONS`] party collections.
    pub(crate) fn attach(&mut self, policy: ValidatedPolicy) -> Result<(), String> {
        conflict_admissibility(&policy)?;
        if !policy.prohibitions.is_empty() && policy.party_collections.len() > MAX_PARTY_COLLECTIONS
        {
            return Err(format!(
                "a policy with prohibitions may name at most {MAX_PARTY_COLLECTIONS} party \
                 collections; this one names {}",
                policy.party_collections.len()
            ));
        }
        self.policies.push(Arc::new(policy));
        Ok(())
    }

    pub(crate) fn add_asset_membership(&mut self, asset: String, collection: String) {
        let of = self.asset_memberships.entry(asset).or_default();
        if !of.contains(&collection) {
            of.push(collection);
        }
    }

    pub(crate) fn clear(&mut self) {
        self.policies.clear();
        self.asset_memberships.clear();
    }

    fn request(&self, action: &str, target: &str, s: &Session) -> Request {
        let mut req = Request::new(format!("{ODRL_NS}{action}"))
            .on(target)
            .with_asset_memberships(
                self.asset_memberships
                    .get(target)
                    .into_iter()
                    .flatten()
                    .map(|c| (target, c.as_str())),
            );
        if let Some(agent) = s.agent {
            req = req.by(agent);
        }
        if let Some(now) = s.now {
            req = req.at(now);
        }
        req
    }

    /// Whether `target` is a graph attached policies may restrict: not a reserved
    /// `urn:sparq:` graph and not an access-control document.
    pub(crate) fn governs(target: &str) -> bool {
        !(target.starts_with(RESERVED_PREFIX)
            || target.ends_with(".acl")
            || target.ends_with(".acr"))
    }

    /// Whether an attached prohibition removes `mode` on `target` for `session`. The one
    /// gate every authorizing entry point of [`crate::PodStore`] goes through.
    pub(crate) fn denies(&self, s: &Session, mode: Mode, target: &str) -> bool {
        let actions = match mode {
            Mode::Read => READ_ACTIONS,
            Mode::Append | Mode::Write => CHANGE_ACTIONS,
            Mode::Control => return false,
        };
        // Permission-only policies cannot remove a mode, so they cost nothing here.
        if !Self::governs(target) || self.policies.iter().all(|p| p.prohibitions.is_empty()) {
            return false;
        }
        actions.iter().any(|a| {
            let req = self.request(a, target, s);
            self.policies.iter().any(|p| prohibited(p, &req, s.agent))
        })
    }

    /// Whether an attached permission grants `mode` on `target` to `session`: only
    /// [`Mode::Read`], when some attached policy's [`decide`] permits `odrl:read`.
    /// Access-control documents and reserved graphs are never granted. The caller still
    /// checks the target's eligibility and applies [`OdrlEnforcement::denies`].
    pub(crate) fn grants(&self, s: &Session, mode: Mode, target: &str) -> bool {
        mode == Mode::Read
            && Self::governs(target)
            && self.has_permissions()
            && self
                .policies
                .iter()
                .any(|p| permitted(p, &self.request("read", target, s), s.agent))
    }

    /// Whether any attached policy has a permission (only then can [`grants`] hold).
    ///
    /// [`grants`]: OdrlEnforcement::grants
    pub(crate) fn has_permissions(&self) -> bool {
        self.policies.iter().any(|p| !p.permissions.is_empty())
    }

    /// The static accessible set `base` minus the graphs [`OdrlEnforcement::denies`]
    /// removes. Order is preserved.
    pub(crate) fn filter(&self, s: &Session, mode: Mode, base: &[NamedNode]) -> Vec<NamedNode> {
        base.iter()
            .filter(|g| !self.denies(s, mode, g.as_str()))
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A request carries only its own target's membership edges, however many other
    /// assets have recorded memberships, and repeated edges are stored once.
    #[test]
    fn requests_carry_only_their_targets_memberships() {
        let mut odrl = OdrlEnforcement::default();
        for i in 0..1000 {
            odrl.add_asset_membership(format!("https://pod.ex/other/{i}"), "urn:c:other".into());
        }
        for _ in 0..3 {
            odrl.add_asset_membership("https://pod.ex/n1".into(), "urn:c:notes".into());
        }
        let s = Session { agent: None, client: None, issuer: None, now: None };
        let req = odrl.request("read", "https://pod.ex/n1", &s);
        assert!(req.asset_matches("urn:c:notes"));
        let shown = format!("{req:?}");
        assert!(!shown.contains("urn:c:other"), "{shown}");
        assert_eq!(shown.matches("urn:c:notes").count(), 1, "{shown}");
        let other = odrl.request("read", "https://pod.ex/other/7", &s);
        assert!(other.asset_matches("urn:c:other") && !other.asset_matches("urn:c:notes"));
    }
}
