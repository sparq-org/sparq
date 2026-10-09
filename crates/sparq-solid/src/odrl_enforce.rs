//! Request-time ODRL decisions (issue #6743).
//!
//! The materializing bridge ([`crate::odrl_bridge`]) stores a frozen projection of one
//! decision, so it can only deny every session on a prohibited asset and can only grant
//! what `decide` settled for good. Policies attached here are instead evaluated through
//! [`sparq_policy::decide`] for the **accessing session** on every access decision: its
//! agent is the party (and default recipient), its clock is the request time, and every
//! named graph in the store is a candidate target.
//!
//! For a session and mode, a graph is accessible when the static view or some attached
//! policy grants it, and no attached policy prohibits it:
//!
//! - **Prohibit:** some policy's prohibition applies ([`matched_prohibition`], True or
//!   Unknown) to a request for any ODRL action that maps to the mode. `odrl:read`,
//!   `display`, `present`, `print` and `play` map to [`Mode::Read`]; `append`, `modify`,
//!   `delete` and `write` map to both [`Mode::Append`] and [`Mode::Write`], since either
//!   mode can change the resource.
//! - **Grant:** some policy's [`decide`] allows the mode's canonical action (`read`,
//!   `append`, `modify`) for the session.
//!
//! [`Mode::Control`] has no ODRL action, so attached policies leave it unchanged.
//! Policies whose `odrl:conflict` strategy the bridge cannot honour are refused at
//! attach time.

use crate::authindex::{Mode, Session};
use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use sparq_policy::{conflict_admissibility, decide, matched_prohibition, Request, ValidatedPolicy};
use std::collections::BTreeSet;
use std::sync::Arc;

const ODRL_NS: &str = "http://www.w3.org/ns/odrl/2/";
/// Reserved graph names (the auth view and its bookkeeping) are never ODRL targets.
const RESERVED_PREFIX: &str = "urn:sparq:";

const READ_ACTIONS: &[&str] = &["read", "display", "present", "print", "play"];
const CHANGE_ACTIONS: &[&str] = &["append", "modify", "delete", "write"];

/// The ODRL policies a [`crate::PodStore`] evaluates per request, with the asset
/// membership evidence (`asset odrl:partOf collection`) each request carries.
#[derive(Debug, Clone, Default)]
pub struct OdrlEnforcement {
    policies: Vec<Arc<ValidatedPolicy>>,
    asset_memberships: Vec<(String, String)>,
}

impl OdrlEnforcement {
    /// Whether no policy is attached (the store then behaves exactly as without this
    /// layer).
    pub fn is_empty(&self) -> bool {
        self.policies.is_empty()
    }

    /// Attach `policy`. Refused when its declared `odrl:conflict` strategy is one
    /// `decide` cannot honour.
    pub(crate) fn attach(&mut self, policy: ValidatedPolicy) -> Result<(), String> {
        conflict_admissibility(&policy)?;
        self.policies.push(Arc::new(policy));
        Ok(())
    }

    pub(crate) fn add_asset_membership(&mut self, asset: String, collection: String) {
        self.asset_memberships.push((asset, collection));
    }

    pub(crate) fn clear(&mut self) {
        self.policies.clear();
        self.asset_memberships.clear();
    }

    fn request(&self, action: &str, target: &str, s: &Session) -> Request {
        let mut req = Request::new(format!("{ODRL_NS}{action}"))
            .on(target)
            .with_asset_memberships(self.asset_memberships.iter().cloned());
        if let Some(agent) = s.agent {
            req = req.by(agent);
        }
        if let Some(now) = s.now {
            req = req.at(now);
        }
        req
    }

    /// Whether an attached prohibition applies to `session` acting on `target` in `mode`.
    pub(crate) fn prohibits(&self, s: &Session, mode: Mode, target: &str) -> bool {
        let actions = match mode {
            Mode::Read => READ_ACTIONS,
            Mode::Append | Mode::Write => CHANGE_ACTIONS,
            Mode::Control => return false,
        };
        actions.iter().any(|a| {
            let req = self.request(a, target, s);
            self.policies.iter().any(|p| matched_prohibition(p, &req).is_some())
        })
    }

    /// Whether an attached policy grants `session` the mode's canonical action on
    /// `target`. A prohibition still overrides it (see [`OdrlEnforcement::allows`]).
    pub(crate) fn grants(&self, s: &Session, mode: Mode, target: &str) -> bool {
        let action = match mode {
            Mode::Read => "read",
            Mode::Append => "append",
            Mode::Write => "modify",
            Mode::Control => return false,
        };
        let req = self.request(action, target, s);
        self.policies.iter().any(|p| decide(p, &req).allow)
    }

    /// The access verdict for `target` given the static verdict `static_allow`.
    pub(crate) fn allows(&self, s: &Session, mode: Mode, target: &str, static_allow: bool) -> bool {
        (static_allow || self.grants(s, mode, target)) && !self.prohibits(s, mode, target)
    }

    /// Apply the attached policies to the static accessible set `base` (sorted
    /// ascending by IRI), over every named graph of `graph`. Returns the sorted result.
    pub(crate) fn apply(&self, graph: &Graph, s: &Session, mode: Mode, base: &[NamedNode]) -> Vec<NamedNode> {
        let allowed: BTreeSet<&str> = base.iter().map(NamedNode::as_str).collect();
        let mut candidates: BTreeSet<&str> = allowed.clone();
        for (name, _) in &graph.named {
            if let Term::NamedNode(n) = name {
                if !n.as_str().starts_with(RESERVED_PREFIX) {
                    candidates.insert(n.as_str());
                }
            }
        }
        candidates
            .into_iter()
            .filter(|g| self.allows(s, mode, g, allowed.contains(g)))
            .map(NamedNode::new_unchecked)
            .collect()
    }
}
