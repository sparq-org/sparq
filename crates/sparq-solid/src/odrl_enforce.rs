//! Request-time ODRL prohibitions (issue #6743).
//!
//! The materializing bridge ([`crate::odrl_bridge`]) stores a frozen projection of one
//! decision, so it can only deny every session on a prohibited asset, and only on assets
//! someone materialized. Policies attached here are instead evaluated through
//! [`matched_prohibition`] for the **accessing session** on every access decision: its
//! agent is the party (and default recipient), its clock is the request time, and the
//! recorded asset memberships are the collection evidence.
//!
//! The layer is **deny-only**. It can remove a mode the static WAC/ACP decision grants,
//! and never adds one, so it composes with ACP denies, origin restrictions and `Control`
//! as an intersection. Attached permissions have no effect here; a lasting grant still
//! goes through the materializing bridge. A mode is removed when some attached
//! prohibition applies (True or Unknown) to a request for any ODRL action that maps to
//! it:
//!
//! - `odrl:read`, `display`, `present`, `print` and `play` map to [`Mode::Read`];
//! - `append`, `modify`, `delete` and `write` map to both [`Mode::Append`] and
//!   [`Mode::Write`], since either mode can change the resource.
//!
//! [`Mode::Control`], access-control documents (`.acl`/`.acr`) and the reserved
//! `urn:sparq:` graphs are never touched, so an attached policy cannot lock an owner out
//! of their own rules. Policies whose `odrl:conflict` strategy `decide` cannot honour are
//! refused at attach time.
//!
//! Every [`crate::PodStore`] entry point that authorizes a session consults one deny gate:
//! the cached set behind `accessible`, the views, queries and `wac_allow`; point
//! `decide`, `decide_batch` and `decide_create`; and every update.

use crate::authindex::{Mode, Session};
use oxrdf::NamedNode;
use sparq_policy::{conflict_admissibility, matched_prohibition, Request, ValidatedPolicy};
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
        if self.is_empty() || !Self::governs(target) {
            return false;
        }
        actions.iter().any(|a| {
            let req = self.request(a, target, s);
            self.policies
                .iter()
                .any(|p| matched_prohibition(p, &req).is_some())
        })
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
