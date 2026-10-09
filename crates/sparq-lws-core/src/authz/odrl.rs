// AUTHORED-BY Claude Sonnet 4.6
//! The OPT-IN ODRL policy gate seam on the read/query path (`odrl-authz`, sq-elg47).
//!
//! `tests/odrl_query_enforcement.rs` pinned the graph-granularity enforcement contract with an
//! honest scope note: this crate had NO reach to the ODRL bridge — its only shipped decide surface
//! was the WAC authorizer ([`super::wac`]), while the ODRL estate (`sparq-policy`,
//! `sparq-solid`'s odrl-bridge, `sparq-server`'s `/authz/*` lane) lived elsewhere. This module is
//! that missing src seam: a small, object-safe gate the LDP read path and the SPARQL endpoint
//! consult AFTER the WAC decision, so an ODRL policy can gate a read natively — superseding the
//! WAC-only contract on the graphs it targets.
//!
//! ## The composition contract (deny-overrides, permit-extends)
//! Per target graph (in the lws data model every resource IS the named graph its reads' SPARQL
//! runs over), the gate returns an [`OdrlVerdict`] that composes with the WAC decision exactly as
//! the `sparq-server` ODRL lane composes bridged grants/denies with the WAC/ACP view
//! (`∪ allow ∖ ∪ deny`):
//! - [`Deny`](OdrlVerdict::Deny) beats any static WAC grant (a matched prohibition — or a policy
//!   that targets the graph but grants this requester nothing — never yields a silent allow);
//! - [`Permit`](OdrlVerdict::Permit) is a native grant of the READ action: it admits the read even
//!   where WAC alone grants nothing (the "superseding" half). It never widens a non-read
//!   requirement (the caller applies it only when the required mode is `Read`);
//! - [`NotApplicable`](OdrlVerdict::NotApplicable) leaves the WAC decision untouched — a policy
//!   governs ONLY the graphs it targets.
//!
//! ## Fail-closed scope (mirroring the server lane, sq-lrtc3.1)
//! The shipped [`PolicyOdrlGate`] refuses AT CONSTRUCTION any policy it cannot faithfully enforce:
//! an unparseable body, an `odrl:conflict` strategy other than deny-overrides
//! (`sparq_policy::conflict_admissibility`), or a rule with no concrete target graph IRI (a
//! targetless PROHIBITION would have to apply to every graph; under-materialising it widens
//! access — pattern targets are the sq-lrtc3.3 design track). At decision time an ANONYMOUS
//! requester is denied on any targeted graph (a principal-scoped grant/deny cannot be evaluated
//! against no principal), and a constraint dimension with no evidence never grants — the
//! `sparq_policy` evaluator is fail-closed on missing evidence.
//!
//! The trait seam is deliberately implementation-agnostic: an embedder can plug a gate that
//! consults the `sparq-solid` odrl-bridge (`PodStore::materialize_odrl_policy`) or a remote PDP;
//! the shipped default evaluates the SAME `sparq-policy` primitives that bridge builds on,
//! in-process, with no HTTP round-trip.

use sparq_policy::{conflict_admissibility, evaluate, parse_policy_str, Request, ValidatedPolicy, Value};

/// The ODRL action a read/query exercises (`odrl:read`) — the only action this gate evidences
/// (the full query-action contract is sq-lrtc3.2, matching the server lane's scope).
const ODRL_READ: &str = "http://www.w3.org/ns/odrl/2/read";

/// The gate's per-(graph, requester) verdict, composed with the WAC decision by the caller
/// (deny-overrides, permit-extends — see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OdrlVerdict {
    /// A permission matched (un-prohibited, duties discharged): admit the read even where WAC
    /// alone grants nothing.
    Permit,
    /// The policy targets this graph and does NOT grant this requester the read (a matched
    /// prohibition, an unmet constraint, an undischarged duty, no matching permission, or an
    /// anonymous requester): deny, overriding any static WAC grant.
    Deny,
    /// The policy does not target this graph: the WAC decision stands unchanged.
    NotApplicable,
}

/// The ODRL gate seam the read/query path consults after WAC. Object-safe and synchronous on
/// purpose: a decision is pure rule-matching over an already-loaded policy (no I/O on the hot
/// path); an implementation that needs I/O resolves it at construction / refresh time.
pub trait OdrlGate: Send + Sync {
    /// The verdict for a READ of `target_graph` (the resource IRI, which IS its named graph) by
    /// `web_id` (`None` ⇒ anonymous). Must be fail-closed: when the policy targets the graph and
    /// the grant cannot be established, the answer is [`OdrlVerdict::Deny`], never a fallthrough.
    fn decide_read(&self, target_graph: &str, web_id: Option<&str>) -> OdrlVerdict;
}

/// The shipped [`OdrlGate`]: a static ODRL policy (parsed once, at construction) evaluated
/// in-process with `sparq_policy::evaluate` for `(party = requester WebID, action = odrl:read,
/// target = the graph)` — the requester is also supplied as the `odrl:recipient` evidence (the
/// read's results are delivered to the requester), matching the server lane's request shape.
#[derive(Debug)]
pub struct PolicyOdrlGate {
    policy: ValidatedPolicy,
}

impl PolicyOdrlGate {
    /// Parse + admit a Turtle ODRL policy, refusing (fail-closed) anything this gate cannot
    /// faithfully enforce: an unparseable body, a conflict strategy other than deny-overrides,
    /// or a rule without a concrete target graph IRI. The error is for the OPERATOR (router
    /// assembly / config load) — it is never surfaced to a requester.
    pub fn from_turtle(turtle: &str) -> Result<PolicyOdrlGate, String> {
        let policy = parse_policy_str(turtle, "turtle")?;
        conflict_admissibility(&policy)
            .map_err(|e| format!("inadmissible odrl:conflict strategy: {:?}", e))?;
        for rule in policy.permissions.iter().chain(policy.prohibitions.iter()) {
            if rule.target.is_none() {
                return Err(format!(
                    "ODRL rule {} has no concrete target graph IRI (fail-closed; \
                     pattern targets are a tracked design follow-up)",
                    rule.id
                ));
            }
        }
        Ok(PolicyOdrlGate { policy })
    }

    /// Does any rule of the policy target `graph`? Concrete-IRI equality only — the same
    /// graph-granularity scope the construction refusals guarantee.
    fn targets(&self, graph: &str) -> bool {
        self.policy
            .permissions
            .iter()
            .chain(self.policy.prohibitions.iter())
            .any(|rule| rule.target.as_deref() == Some(graph))
    }
}

impl OdrlGate for PolicyOdrlGate {
    fn decide_read(&self, target_graph: &str, web_id: Option<&str>) -> OdrlVerdict {
        if !self.targets(target_graph) {
            return OdrlVerdict::NotApplicable;
        }
        // A targeted graph + no principal: principal-scoped grants/denies cannot be evaluated
        // against an anonymous requester — deny (fail-closed), exactly as the server lane
        // refuses an anonymous session wherever its ODRL lane fires.
        let Some(agent) = web_id else {
            return OdrlVerdict::Deny;
        };
        let request = Request::new(ODRL_READ)
            .on(target_graph)
            .by(agent)
            .with(
                sparq_policy::ODRL_RECIPIENT,
                Value::Iri(agent.to_owned()),
            );
        if evaluate(&self.policy, &request).allow {
            OdrlVerdict::Permit
        } else {
            OdrlVerdict::Deny
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "https://pod.example/alice/c/doc";
    const ALICE: &str = "https://agents.example/alice#me";
    const BOB: &str = "https://agents.example/bob#me";

    fn permit_alice_prohibit_bob() -> PolicyOdrlGate {
        PolicyOdrlGate::from_turtle(&format!(
            r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <{DOC}> ; odrl:assignee <{ALICE}> ] ;
  odrl:prohibition [ odrl:action odrl:read ; odrl:target <{DOC}> ; odrl:assignee <{BOB}> ] .
"#
        ))
        .expect("admissible policy")
    }

    #[test]
    fn from_turtle_refuses_unparseable_policy() {
        assert!(PolicyOdrlGate::from_turtle("@@@ not turtle").is_err());
    }

    #[test]
    fn from_turtle_refuses_targetless_rule() {
        let err = PolicyOdrlGate::from_turtle(
            r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol> a odrl:Set ;
  odrl:prohibition [ odrl:action odrl:read ;
                     odrl:assignee <https://agents.example/bob#me> ] .
"#,
        )
        .unwrap_err();
        assert!(err.contains("no concrete target"), "{err}");
    }

    #[test]
    fn from_turtle_refuses_inadmissible_conflict_strategy() {
        // `odrl:perm` (permission-overrides) is NOT the deny-overrides strategy the gate
        // enforces — it must be refused up front, never silently coerced.
        let result = PolicyOdrlGate::from_turtle(
            r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol> a odrl:Set ; odrl:conflict odrl:perm ;
  odrl:permission [ odrl:action odrl:read ;
                    odrl:target <https://pod.example/alice/c/doc> ;
                    odrl:assignee <https://agents.example/alice#me> ] .
"#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn permitted_party_gets_permit() {
        let gate = permit_alice_prohibit_bob();
        assert_eq!(gate.decide_read(DOC, Some(ALICE)), OdrlVerdict::Permit);
    }

    #[test]
    fn prohibited_party_gets_deny() {
        let gate = permit_alice_prohibit_bob();
        assert_eq!(gate.decide_read(DOC, Some(BOB)), OdrlVerdict::Deny);
    }

    #[test]
    fn unnamed_party_gets_deny_not_fallthrough() {
        // Carol is neither permitted nor prohibited, but the policy TARGETS the graph — a
        // targeted graph with no grant denies (never rounds up, never falls through to WAC).
        let gate = permit_alice_prohibit_bob();
        let carol = "https://agents.example/carol#me";
        assert_eq!(gate.decide_read(DOC, Some(carol)), OdrlVerdict::Deny);
    }

    #[test]
    fn anonymous_on_targeted_graph_is_denied() {
        let gate = permit_alice_prohibit_bob();
        assert_eq!(gate.decide_read(DOC, None), OdrlVerdict::Deny);
    }

    #[test]
    fn untargeted_graph_is_not_applicable() {
        let gate = permit_alice_prohibit_bob();
        let other = "https://pod.example/alice/c/other";
        assert_eq!(gate.decide_read(other, Some(ALICE)), OdrlVerdict::NotApplicable);
        assert_eq!(gate.decide_read(other, None), OdrlVerdict::NotApplicable);
    }
}
