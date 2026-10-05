//! The ODRL evaluator: `(Policy, Request) -> Decision`.
//!
//! Semantics (the single-node base case — the ODRL Formal-Semantics CG
//! *closed-world* default, restricted to one node's data):
//!
//! 1. A [`Rule`] *matches* a [`Request`] when its action permits the requested
//!    action, its `target` and `assignee` (if specified) agree with the request,
//!    and every one of its [`Constraint`]s is satisfied by the request context.
//! 2. A [`Permission`] grants access iff it matches **and** all of its
//!    [`Duty`]s are discharged (reported in the request context).
//! 3. A [`Prohibition`] that matches **overrides** any permission (it "carves
//!    out" a forbidden sub-set — ODRL Formal Semantics §conflict).
//! 4. **Fail-closed default:** no matching+discharged permission, OR any matching
//!    prohibition, ⇒ DENY. An empty/malformed policy denies everything.
//!
//! [OPUS-4.8]

use crate::model::{
    Action, Constraint, ConstraintNode, LogicalConstraint, LogicalOperator, Operator, Policy, Rule,
    Value,
};
use std::collections::{BTreeMap, BTreeSet};

/// The `odrl:purpose` left-operand IRI — the dimension a *purpose constraint*
/// restricts (the purpose of use a permission/prohibition is gated on, e.g.
/// `odrl:purpose eq <urn:purpose/research>`). [OPUS-4.8] sq-q56r.
pub const ODRL_PURPOSE: &str = "http://www.w3.org/ns/odrl/2/purpose";

/// The `odrl:count` left-operand IRI — the dimension a *count constraint* restricts
/// (the number of times a permission may be exercised, e.g. `odrl:count lteq 5`).
/// Stateful enforcement of this lives in the opt-in `crate::count` module.
/// [OPUS-4.8] sq-zi5w.
pub const ODRL_COUNT: &str = "http://www.w3.org/ns/odrl/2/count";

/// The `odrl:recipient` left-operand IRI — the dimension a *recipient constraint*
/// restricts (the party that data is disclosed to, e.g. `odrl:recipient neq <bob>`,
/// the "everyone EXCEPT bob" shape). The recipient-of-data is the requesting party,
/// so when a request carries **no** explicit `odrl:recipient` context value the
/// evaluator reads its [`Request::party`] as the recipient evidence (see
/// [`recipient_status`]). [OPUS-4.8] sq-5037.
pub const ODRL_RECIPIENT: &str = "http://www.w3.org/ns/odrl/2/recipient";

/// The `odrl:spatial` left-operand IRI — the dimension a *spatial constraint*
/// restricts (the geographic region a permission/prohibition is gated on, e.g.
/// `odrl:spatial isPartOf <country/EU>`, "anywhere in the EU"). A request supplies its
/// region as `odrl:spatial` evidence; an `isPartOf` spatial constraint matches a
/// sub-region the request declares part-of the named region by supplying the region
/// `isPartOf` tree as subsumption evidence via [`Request::with_purpose_subsumption`] /
/// [`Request::with_purpose_taxonomy`] (`DEU ⊑ EU` — the same caller-supplied closure the
/// DPV purpose taxonomy uses; see [`spatial_status`]). [OPUS-4.8] sq-wukl.
pub const ODRL_SPATIAL: &str = "http://www.w3.org/ns/odrl/2/spatial";

/// The `odrl:dateTime` left-operand IRI — the dimension a *temporal constraint*
/// restricts (the instant a permission/prohibition is gated on, e.g.
/// `odrl:dateTime lteq "2026-12-31T23:59:59Z"`, the upper edge of a validity
/// window). The actual instant is the request's evaluation time, supplied as
/// [`Value::DateTime`] evidence (see [`Request::at`] / [`datetime_status`]); a
/// request that supplies **no** time carries no temporal evidence, so a temporal
/// permission does not grant and a temporal prohibition is not withdrawn
/// (fail-closed). [OPUS-4.8] sq-idnv.
pub const ODRL_DATETIME: &str = "http://www.w3.org/ns/odrl/2/dateTime";

/// An access request evaluated against a [`Policy`]: who wants to do what, to
/// what, in what context (the "evaluation request" + "state of the world" of the
/// ODRL Formal Semantics, folded into one node-local view).
#[derive(Debug, Clone, Default)]
pub struct Request {
    /// The action being requested (`odrl:` action IRI), e.g. `odrl:read`.
    pub action: String,
    /// The target asset/graph IRI the action is on.
    pub target: Option<String>,
    /// The requesting party (the WebID / agent IRI — matched against
    /// `odrl:assignee`).
    pub party: Option<String>,
    /// The requesting party as an [`Value::Iri`], kept in lock-step with [`party`]
    /// (set by [`Request::by`]) so a `recipient` constraint with no explicit
    /// `odrl:recipient` context can be re-checked against *who is asking* without an
    /// allocation per constraint. Internal — [`party`] is the public field.
    /// [OPUS-4.8] sq-5037.
    ///
    /// [`party`]: Request::party
    pub(crate) recipient_party: Option<Value>,
    /// Context values keyed by ODRL `leftOperand` IRI (e.g. `odrl:dateTime`,
    /// `odrl:purpose`, `odrl:recipient`, `odrl:count`) — the state-of-the-world a
    /// constraint is evaluated against. Use [`Request::with`] to populate.
    pub context: BTreeMap<String, Value>,
    /// The set of duty *action* IRIs the caller asserts have been discharged
    /// (e.g. `odrl:anonymize`, a custom `proveAttestation`). A permission with an
    /// undischarged duty is denied.
    pub discharged_duties: BTreeSet<String>,
    /// The **subsumption** evidence the request supplies: the *transitive closure* of
    /// `(narrower, broader)` pairs — `narrower ⊑ broader`, i.e. the stated value
    /// `narrower` IS-A / is-part-of the broader value. Used for the two taxonomic
    /// dimensions — `odrl:purpose` (a DPV/purpose taxonomy, `skos:broader` /
    /// `rdfs:subClassOf` / `dpv:isSubTypeOf` edges) and `odrl:spatial` (a region
    /// `isPartOf` tree, e.g. `DEU ⊑ EU`). Supplied via
    /// [`Request::with_purpose_subsumption`] / [`Request::with_purpose_taxonomy`].
    /// Internal — populated only through those builders, which maintain the closure so
    /// a lookup is a single membership test.
    ///
    /// **Soundness:** subsumption matching draws ONLY on this caller-supplied closure
    /// (never inferred from IRI string structure), so with an empty closure matching is
    /// byte-for-byte the exact-IRI base case and access is never widened on an unproven
    /// relation. [OPUS-4.8] sq-z3ve / sq-wukl.
    pub(crate) purpose_subsumes: BTreeSet<(String, String)>,
    /// **Party-collection** membership evidence the request supplies: `(party,
    /// collection)` pairs asserting `party odrl:partOf collection` (an
    /// `odrl:PartyCollection`). A rule whose `assignee` names a collection matches a
    /// request whose [`party`](Request::party) is a member of that collection. Populated
    /// only via [`Request::with_party_membership`] / [`Request::with_party_memberships`].
    /// [OPUS-4.8] sq-k7itg.
    ///
    /// **Soundness:** membership draws ONLY on this caller-supplied set (read out of the
    /// state-of-the-world graph), never inferred — with an empty set, assignee matching is
    /// byte-for-byte the exact-IRI base case and access is never widened.
    pub(crate) party_memberships: BTreeSet<(String, String)>,
    /// **Asset-collection** membership evidence: `(asset, collection)` pairs asserting
    /// `asset odrl:partOf collection` (an `odrl:AssetCollection`). A rule whose `target`
    /// names a collection matches a request whose [`target`](Request::target) is a member
    /// of that collection. Populated only via [`Request::with_asset_membership`] /
    /// [`Request::with_asset_memberships`]. Same soundness contract as
    /// [`party_memberships`](Request::party_memberships). [OPUS-4.8] sq-k7itg.
    pub(crate) asset_memberships: BTreeSet<(String, String)>,
}

impl Request {
    /// A request for `action` on `target` by `party` with no context/duties yet.
    pub fn new(action: impl Into<String>) -> Request {
        Request {
            action: action.into(),
            ..Request::default()
        }
    }

    /// Set the target asset/graph IRI.
    pub fn on(mut self, target: impl Into<String>) -> Request {
        self.target = Some(target.into());
        self
    }

    /// Set the requesting party (assignee) IRI.
    ///
    /// The party doubles as the default **recipient-of-data**: an `odrl:recipient`
    /// constraint with no explicit context value is re-checked against this party
    /// (see [`recipient_status`]), so a `recipient neq X` rule gates on who is asking.
    /// [OPUS-4.8] sq-5037.
    pub fn by(mut self, party: impl Into<String>) -> Request {
        let p = party.into();
        self.recipient_party = Some(Value::Iri(p.clone()));
        self.party = Some(p);
        self
    }

    /// Add a context value for a `leftOperand` IRI (chainable).
    pub fn with(mut self, left_operand: impl Into<String>, value: Value) -> Request {
        self.context.insert(left_operand.into(), value);
        self
    }

    /// Mark a duty action IRI as discharged (chainable).
    pub fn discharge(mut self, duty_action: impl Into<String>) -> Request {
        self.discharged_duties.insert(duty_action.into());
        self
    }

    /// Declare the **purpose of use** this request carries as evidence (chainable)
    /// — the `odrl:purpose` left-operand value a purpose-gated rule is checked
    /// against. [OPUS-4.8] sq-q56r.
    ///
    /// This is first-class sugar over [`Request::with`]`(ODRL_PURPOSE, ..)`: it
    /// makes the *evidence the requester supplies* explicit (and auditable via
    /// [`Request::purpose`]) instead of relying on a magic context key. A request
    /// that does NOT call this carries **no** purpose evidence, so a permission
    /// gated on purpose does not grant and a prohibition gated on purpose is not
    /// withdrawn (fail-closed — see [`purpose_status`]).
    ///
    /// The value is stored verbatim: a DPV/purpose-taxonomy IRI as [`Value::Iri`]
    /// (`Value::Iri(..)`) or a purpose code as [`Value::Str`]. Matching is **exact**
    /// (IRI/string equality) by default; supply DPV/purpose-taxonomy edges via
    /// [`Request::with_purpose_subsumption`] / [`Request::with_purpose_taxonomy`] to
    /// also match a stated purpose against the *broader* purposes it falls under
    /// (`P ⊑ B`) — see [`purpose_status`].
    pub fn for_purpose(self, purpose: Value) -> Request {
        self.with(ODRL_PURPOSE, purpose)
    }

    /// The purpose-of-use evidence this request carries (`odrl:purpose`), or `None`
    /// if the request supplied none. The auditable answer to *"did the request
    /// actually state a purpose?"* — the question faithful purpose enforcement turns
    /// on. [OPUS-4.8] sq-q56r.
    pub fn purpose(&self) -> Option<&Value> {
        self.context.get(ODRL_PURPOSE)
    }

    /// Declare one **purpose-subsumption** edge `narrower ⊑ broader` (chainable) —
    /// "the purpose `narrower` IS-A / is-part-of the broader purpose `broader`" —
    /// from a DPV/purpose-taxonomy (`skos:broader` / `rdfs:subClassOf` /
    /// `dpv:isSubTypeOf`). [OPUS-4.8] sq-z3ve.
    ///
    /// With one or more such edges supplied, an `odrl:purpose` constraint that names
    /// a purpose `B` is satisfied by a request whose stated purpose `P` is `B` **or
    /// transitively narrower than `B`** (`P ⊑ B`): a permission gated on the broad
    /// `research` purpose then covers a request for the narrow `clinical-research`
    /// sub-purpose, and a `neq research` carve-out *also* excludes that sub-purpose
    /// (a sub-purpose IS a research purpose). The relation is the **caller-supplied**
    /// transitive closure only — it is never inferred from IRI string structure — so
    /// matching is *sound*: with no edge supplied it is the exact-IRI base case, and
    /// access is never widened on an unproven subsumption. The closure is maintained
    /// incrementally (adding `a⊑b` when `b⊑c` is already known also records `a⊑c`).
    ///
    /// Subsumption is consulted for `odrl:purpose` constraints only (the DPV
    /// dimension that forms a taxonomy); `recipient`/`dateTime`/`count` are
    /// unaffected.
    pub fn with_purpose_subsumption(
        mut self,
        narrower: impl Into<String>,
        broader: impl Into<String>,
    ) -> Request {
        insert_subsumption(&mut self.purpose_subsumes, narrower.into(), broader.into());
        self
    }

    /// Declare many **purpose-subsumption** edges `(narrower, broader)` at once
    /// (chainable) — the bulk form of [`Request::with_purpose_subsumption`], e.g. the
    /// `skos:broader`/`rdfs:subClassOf` edges read out of a DPV taxonomy graph. The
    /// transitive closure is computed across all supplied edges (order-independent).
    /// [OPUS-4.8] sq-z3ve.
    pub fn with_purpose_taxonomy<N, B, I>(mut self, edges: I) -> Request
    where
        N: Into<String>,
        B: Into<String>,
        I: IntoIterator<Item = (N, B)>,
    {
        for (n, b) in edges {
            insert_subsumption(&mut self.purpose_subsumes, n.into(), b.into());
        }
        self
    }

    /// Whether the request's supplied taxonomy proves the purpose `p` is `target`
    /// or transitively narrower than it (`p ⊑ target`). [OPUS-4.8] sq-z3ve.
    pub(crate) fn purpose_subsumed_by(&self, p: &str, target: &str) -> bool {
        p == target
            || self
                .purpose_subsumes
                .contains(&(p.to_owned(), target.to_owned()))
    }

    /// Declare the **evaluation time** this request is made at as evidence
    /// (chainable) — the `odrl:dateTime` left-operand value a temporal (time-window)
    /// rule is checked against. [OPUS-4.8] sq-idnv.
    ///
    /// First-class sugar over [`Request::with`]`(ODRL_DATETIME, Value::DateTime(..))`:
    /// it makes the *time evidence the requester supplies* explicit (and auditable via
    /// [`Request::request_time`]) instead of relying on a magic context key. A request
    /// that does NOT call this carries **no** time evidence, so a permission gated on a
    /// time window does not grant and a prohibition gated on a time window is not
    /// withdrawn (fail-closed — see [`datetime_status`]).
    ///
    /// The lexical form is stored verbatim and parsed to a **UTC instant** at
    /// comparison time, so the `lt`/`gt`/`lteq`/`gteq`/`eq`/`neq` operators compare
    /// the *point in time* the value denotes — mixed timezone offsets are normalized
    /// (`…T13:00:00+02:00` == `…T11:00:00Z`) rather than compared lexically
    /// ([OPUS-4.8] sq-qj2q). An unparseable instant compares fail-closed.
    pub fn at(self, instant: impl Into<String>) -> Request {
        self.with(ODRL_DATETIME, Value::DateTime(instant.into()))
    }

    /// The evaluation-time evidence this request carries (`odrl:dateTime`), or `None`
    /// if the request supplied none. The auditable answer to *"did the request supply
    /// a time to check the window against?"* — the question faithful time-window
    /// enforcement turns on (a missing time is Unprovable, not "any time").
    /// [OPUS-4.8] sq-idnv.
    pub fn request_time(&self) -> Option<&Value> {
        self.context.get(ODRL_DATETIME)
    }

    /// Declare one **party-collection membership** edge `party odrl:partOf
    /// collection` (chainable) — read out of the request's state-of-the-world.
    /// [OPUS-4.8] sq-k7itg.
    ///
    /// With one or more such edges supplied, a rule whose `odrl:assignee` names an
    /// `odrl:PartyCollection` is matched by a request whose [`party`](Request::party) is
    /// a *member* of that collection — not only by a request whose party IS the
    /// collection IRI. The membership relation is the **caller-supplied** set only (read
    /// from the sotw, never inferred), so with no edge supplied assignee matching is the
    /// exact-IRI base case and access is never widened.
    pub fn with_party_membership(
        mut self,
        party: impl Into<String>,
        collection: impl Into<String>,
    ) -> Request {
        self.party_memberships
            .insert((party.into(), collection.into()));
        self
    }

    /// Declare many **party-collection membership** edges `(party, collection)` at once
    /// (chainable) — the bulk form of [`Request::with_party_membership`]. [OPUS-4.8]
    /// sq-k7itg.
    pub fn with_party_memberships<P, C, I>(mut self, edges: I) -> Request
    where
        P: Into<String>,
        C: Into<String>,
        I: IntoIterator<Item = (P, C)>,
    {
        for (p, c) in edges {
            self.party_memberships.insert((p.into(), c.into()));
        }
        self
    }

    /// The parties this request's evidence proves are **members** of the party
    /// collection `collection` (`party odrl:partOf collection`) — the public READ side
    /// of [`Request::with_party_membership`]. Returned in the deterministic sorted
    /// order of the underlying set; empty when `collection` is not a collection the
    /// request supplied evidence for (including when it is a plain party IRI).
    /// [SONNET-4.6] sq-rf9uv.
    ///
    /// A consumer that PERSISTS an ODRL rule as a re-checked head (the sparq-solid ODRL
    /// bridge) cannot call the crate-internal `party_matches` per session — a
    /// session carries no membership evidence — so it needs the member set itself to
    /// expand a collection-valued `odrl:assignee` into one head per member. Because the
    /// evidence is caller-supplied and possibly PARTIAL, the expansion is exact only
    /// with respect to the edges supplied here: sound to widen an ALLOW head to (never
    /// beyond) them, but NOT sound to narrow a DENY to them — an unlisted member would
    /// escape the deny. Callers must keep that asymmetry.
    ///
    /// **Not a collection TEST.** Because the empty result is shared by a plain party
    /// IRI and by a collection this request simply supplied no edges for, an empty
    /// return does NOT mean "not a collection", and a caller must not read it as one —
    /// a `Request` carries no `rdf:type odrl:PartyCollection` fact to distinguish them.
    /// A caller that needs to fail CLOSED on collection-valued input (the bridge's DENY
    /// and carve-out directions) can use a non-empty result as proof it is looking at a
    /// collection, but gets no signal at all in the un-evidenced case — for that it must
    /// consult collection IDENTITY, carried separately by
    /// [`Policy::party_collections`](crate::Policy::party_collections).
    pub fn party_collection_members(&self, collection: &str) -> Vec<&str> {
        self.party_memberships
            .iter()
            .filter(|(_, c)| c == collection)
            .map(|(p, _)| p.as_str())
            .collect()
    }

    /// Declare one **asset-collection membership** edge `asset odrl:partOf collection`
    /// (chainable) — the asset twin of [`Request::with_party_membership`]. A rule whose
    /// `odrl:target` names an `odrl:AssetCollection` is then matched by a request whose
    /// [`target`](Request::target) is a member of that collection. [OPUS-4.8] sq-k7itg.
    pub fn with_asset_membership(
        mut self,
        asset: impl Into<String>,
        collection: impl Into<String>,
    ) -> Request {
        self.asset_memberships
            .insert((asset.into(), collection.into()));
        self
    }

    /// Declare many **asset-collection membership** edges `(asset, collection)` at once
    /// (chainable) — the bulk form of [`Request::with_asset_membership`]. [OPUS-4.8]
    /// sq-k7itg.
    pub fn with_asset_memberships<A, C, I>(mut self, edges: I) -> Request
    where
        A: Into<String>,
        C: Into<String>,
        I: IntoIterator<Item = (A, C)>,
    {
        for (a, c) in edges {
            self.asset_memberships.insert((a.into(), c.into()));
        }
        self
    }

    /// Whether the request's party is `target` or a member of the collection `target`
    /// (`party odrl:partOf target`) under the supplied membership evidence. [OPUS-4.8]
    /// sq-k7itg.
    pub(crate) fn party_matches(&self, target: &str) -> bool {
        match self.party.as_deref() {
            Some(p) => {
                p == target
                    || self
                        .party_memberships
                        .contains(&(p.to_owned(), target.to_owned()))
            }
            None => false,
        }
    }

    /// Whether the request's target asset is `target` or a member of the collection
    /// `target` (`asset odrl:partOf target`) under the supplied membership evidence.
    /// [OPUS-4.8] sq-k7itg.
    pub(crate) fn asset_matches(&self, target: &str) -> bool {
        match self.target.as_deref() {
            Some(a) => {
                a == target
                    || self
                        .asset_memberships
                        .contains(&(a.to_owned(), target.to_owned()))
            }
            None => false,
        }
    }
}

/// The result of evaluating a policy against a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    /// `true` ⇒ ALLOW (a permission matched, was un-prohibited, and all its
    /// duties were discharged). `false` ⇒ DENY (fail-closed).
    pub allow: bool,
    /// The IDs of the rule(s) that justify the decision: the granting permission
    /// on an ALLOW; the overriding prohibition(s) (and/or the would-be permission
    /// blocked by an unmet duty/constraint) on a DENY.
    pub matched_rules: Vec<String>,
    /// Human-readable explanations of why a candidate permission did *not* grant
    /// (unmet constraint, undischarged duty, overriding prohibition). Empty on a
    /// clean ALLOW with no caveats.
    pub unmet_constraints: Vec<String>,
}

impl Decision {
    fn deny(matched: Vec<String>, unmet: Vec<String>) -> Decision {
        Decision {
            allow: false,
            matched_rules: matched,
            unmet_constraints: unmet,
        }
    }
}

/// Evaluate `policy` against `request`, returning a fail-closed [`Decision`].
///
/// See the module docs for the exact semantics. This is the single-node
/// base case of ODRL — it reduces to the same allow/deny shape `sparq-solid`'s
/// WAC/ACP path produces, with ODRL's richer purpose/recipient/time constraints
/// and duty obligations layered on top.
///
/// # Examples
///
/// ```
/// use sparq_policy::{evaluate, Policy, Request};
/// // An empty policy denies everything (fail-closed).
/// let d = evaluate(&Policy::default(), &Request::new("http://www.w3.org/ns/odrl/2/read"));
/// assert!(!d.allow);
/// ```
pub fn evaluate(policy: &Policy, request: &Request) -> Decision {
    let req_action = Action(request.action.clone());

    // 1. A matching prohibition overrides everything (fail-closed carve-out).
    let mut blocking: Vec<String> = Vec::new();
    for rule in &policy.prohibitions {
        if rule_matches(rule, request, &req_action).is_match {
            blocking.push(rule.id.clone());
        }
    }
    if !blocking.is_empty() {
        let why = blocking
            .iter()
            .map(|id| format!("prohibition {id} matches the request"))
            .collect();
        return Decision::deny(blocking, why);
    }

    // 2. Find a permission that matches AND has all duties discharged.
    let mut caveats: Vec<String> = Vec::new();
    for rule in &policy.permissions {
        let m = rule_matches(rule, request, &req_action);
        if !m.is_match {
            caveats.extend(m.reasons);
            continue;
        }
        // Matched — now require every duty discharged.
        let undischarged: Vec<&str> = rule
            .duties
            .iter()
            .filter(|d| !request.discharged_duties.contains(&d.action.0))
            .map(|d| d.action.0.as_str())
            .collect();
        if undischarged.is_empty() {
            return Decision {
                allow: true,
                matched_rules: vec![rule.id.clone()],
                unmet_constraints: Vec::new(),
            };
        }
        for a in undischarged {
            caveats.push(format!(
                "permission {} requires undischarged duty {a}",
                rule.id
            ));
        }
    }

    // 3. No grant → DENY (fail-closed).
    if caveats.is_empty() {
        caveats.push("no permission matches the request".to_owned());
    }
    Decision::deny(Vec::new(), caveats)
}

/// The first [`Prohibition`](crate::model::Rule) in `policy` that **matches**
/// `request` (its action permits the requested action, its target/assignee agree,
/// and every constraint is satisfied), or `None` if no prohibition carves the
/// request out. [OPUS-4.8] sq-w693.
///
/// This is the same match test [`evaluate`] applies in step 1 (a matching
/// prohibition overrides everything) — exposed so the `sparq-solid` ODRL→AUTH_GRAPH
/// bridge can materialize a matched prohibition as an explicit `auth:deny*` triple
/// (deny-overrides) WITHOUT re-implementing the match logic. A `Decision` with
/// `allow == false` is NOT sufficient: it conflates a carve-out prohibition with a
/// plain no-matching-permission deny, and only the former should materialize a deny.
///
/// # Examples
///
/// ```
/// use sparq_policy::{matched_prohibition, parse_policy_str, Request};
/// let pol = parse_policy_str(r#"
/// @prefix odrl: <http://www.w3.org/ns/odrl/2/> .
/// <urn:pol/p> a odrl:Set ; odrl:prohibition [
///     odrl:action odrl:write ;
///     odrl:target <https://pod.ex/n1> ;
///     odrl:assignee <https://alice.ex/card#me> ] .
/// "#, "turtle").unwrap();
/// let req = Request::new("http://www.w3.org/ns/odrl/2/write")
///     .on("https://pod.ex/n1").by("https://alice.ex/card#me");
/// assert!(matched_prohibition(&pol, &req).is_some());
/// // a different party is not carved out
/// let other = Request::new("http://www.w3.org/ns/odrl/2/write")
///     .on("https://pod.ex/n1").by("https://bob.ex/card#me");
/// assert!(matched_prohibition(&pol, &other).is_none());
/// ```
pub fn matched_prohibition<'p>(policy: &'p Policy, request: &Request) -> Option<&'p Rule> {
    let req_action = Action(request.action.clone());
    policy
        .prohibitions
        .iter()
        .find(|rule| rule_matches(rule, request, &req_action).is_match)
}

/// Whether `policy`'s prohibitions still carve `request` out — and, when they do
/// not, **whether that is a definite no or merely unprovable**. [OPUS-4.8] sq-2pcf.
///
/// This is the *deny-retraction dual* of [`matched_prohibition`]. A bare
/// `matched_prohibition(..).is_none()` collapses two semantically different worlds:
///
/// - the prohibition was genuinely **withdrawn / no longer structurally applies**
///   (its action/target/assignee no longer name this request, or a constraint it
///   carries is *definitely* false because the request supplies evidence that fails
///   the bound), and
/// - the prohibition still structurally names this request but a constraint is
///   **unprovable** because the request lacks evidence for that dimension
///   (`constraint_satisfied` returns `false` on a missing context value).
///
/// For a *grant* both collapse to "deny access" — fail-closed. But for **retracting a
/// materialized `auth:deny*`** they must NOT: retracting on the second case would
/// RESTORE access on missing evidence (fail-OPEN). This function keeps them apart so
/// the bridge only retracts a deny on a *definite* "no longer holds".
///
/// Returns:
/// - [`ProhibitionStatus::Applies`] if some prohibition still carves the request out
///   (every structural attribute agrees AND every constraint is *satisfied*).
/// - [`ProhibitionStatus::Ambiguous`] if no prohibition matches, but at least one
///   prohibition still structurally names the request (action/target/assignee agree)
///   and fails ONLY because a constraint is unprovable for lack of evidence.
/// - [`ProhibitionStatus::Withdrawn`] otherwise — every prohibition either does not
///   structurally name the request or carries a constraint that is *definitely* false
///   given the supplied evidence. This is the only case in which a deny may be retracted.
///
/// # Examples
///
/// ```
/// use sparq_policy::{prohibition_status, ProhibitionStatus, parse_policy_str, Request, Value};
/// let pol = parse_policy_str(r#"
/// @prefix odrl: <http://www.w3.org/ns/odrl/2/> .
/// <urn:pol/p> a odrl:Set ; odrl:prohibition [
///     odrl:action odrl:write ;
///     odrl:target <https://pod.ex/n1> ;
///     odrl:assignee <https://alice.ex/card#me> ;
///     odrl:constraint [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lt ;
///        odrl:rightOperand "2026-01-01T00:00:00Z"^^<http://www.w3.org/2001/XMLSchema#dateTime> ] ] .
/// "#, "turtle").unwrap();
/// let base = Request::new("http://www.w3.org/ns/odrl/2/write")
///     .on("https://pod.ex/n1").by("https://alice.ex/card#me");
///
/// // Evidence the window holds → still carves out.
/// let inside = base.clone()
///     .with("http://www.w3.org/ns/odrl/2/dateTime", Value::DateTime("2025-06-01T00:00:00Z".into()));
/// assert_eq!(prohibition_status(&pol, &inside), ProhibitionStatus::Applies);
///
/// // Evidence the window has LAPSED → definitely withdrawn.
/// let lapsed = base.clone()
///     .with("http://www.w3.org/ns/odrl/2/dateTime", Value::DateTime("2026-06-01T00:00:00Z".into()));
/// assert_eq!(prohibition_status(&pol, &lapsed), ProhibitionStatus::Withdrawn);
///
/// // NO evidence for the window → unprovable → ambiguous (keep the deny).
/// assert_eq!(prohibition_status(&pol, &base), ProhibitionStatus::Ambiguous);
/// ```
pub fn prohibition_status(policy: &Policy, request: &Request) -> ProhibitionStatus {
    let req_action = Action(request.action.clone());
    let mut any_ambiguous = false;
    for rule in &policy.prohibitions {
        match classify_prohibition(rule, request, &req_action) {
            RuleClass::Match => return ProhibitionStatus::Applies,
            RuleClass::Ambiguous => any_ambiguous = true,
            RuleClass::DefinitelyNo => {}
        }
    }
    if any_ambiguous {
        ProhibitionStatus::Ambiguous
    } else {
        ProhibitionStatus::Withdrawn
    }
}

/// The deny-retraction verdict for a request's prohibitions — see
/// [`prohibition_status`]. [OPUS-4.8] sq-2pcf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProhibitionStatus {
    /// A prohibition still carves the request out — a materialized deny must be KEPT.
    Applies,
    /// No prohibition matches, but one still structurally names the request and fails
    /// only for lack of evidence — the deny must be KEPT (fail-closed: do not restore
    /// access on an unprovable carve-out).
    Ambiguous,
    /// No prohibition structurally names the request, or every one that does carries a
    /// constraint that is *definitely* false given the evidence — the only case in
    /// which a materialized deny may be RETRACTED (access restored).
    Withdrawn,
}

/// The three-valued verdict of a rule's `odrl:purpose` constraints against the
/// purpose evidence a request carries — a FAITHFUL report of exactly what
/// [`evaluate`] checks for purpose (it reuses the same `constraint_status` the
/// evaluator's purpose constraints go through). [OPUS-4.8] sq-q56r.
///
/// The honesty contract: this never claims a stronger verdict than the evaluator
/// would act on. `Satisfied`/`Unprovable`/`DefinitelyUnsatisfied` map 1:1 onto the
/// evaluator's gating — a `Satisfied` purpose is the *only* verdict under which a
/// purpose-gated permission can grant, and anything other than `DefinitelyUnsatisfied`
/// (i.e. `Satisfied` OR `Unprovable`) keeps a purpose-gated prohibition in force.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PurposeMatch {
    /// The rule has no `odrl:purpose` constraint — purpose places no restriction on
    /// it (the rule's other attributes/constraints decide).
    NotConstrained,
    /// The rule constrains purpose AND the request's stated purpose **matches** every
    /// purpose constraint — exact IRI/string equality, or (when the request supplies a
    /// DPV/purpose-taxonomy via [`Request::with_purpose_subsumption`]) a purpose
    /// transitively *narrower* than the constraint's (`P ⊑ B`).
    Satisfied,
    /// The rule constrains purpose but the request carries **no** purpose evidence —
    /// *unprovable*. Fail-closed: a permission does NOT grant on this; a prohibition is
    /// NOT withdrawn. (Never silently read as "any purpose allowed".)
    Unprovable,
    /// The rule constrains purpose and the request's stated purpose **does not match**
    /// — a definite mismatch (a permission does not grant; a prohibition no longer
    /// carves *this* purpose out).
    DefinitelyUnsatisfied,
}

/// Report how `rule`'s `odrl:purpose` constraint(s) stand against the purpose
/// evidence `request` carries — the auditable surface of faithful purpose
/// enforcement. [OPUS-4.8] sq-q56r.
///
/// This does NOT re-implement matching: it runs the request through the SAME
/// `constraint_status` every `odrl:purpose` constraint goes through inside
/// [`evaluate`], so the verdict it reports is exactly what the evaluator acts on —
/// the whole point of the bead (no claimed enforcement that isn't actually checked).
///
/// **Match semantics (the boundary, not over-claimed):** a purpose matches by
/// **exact** IRI/string equality (an `eq`/`isA` purpose constraint), or by membership
/// in an explicit `isPartOf` purpose *set* (the `|`/space/comma-separated right
/// operand). When the request supplies a DPV/purpose-taxonomy (via
/// [`Request::with_purpose_subsumption`] / [`Request::with_purpose_taxonomy`]), a
/// stated purpose ALSO matches a constraint that names a *broader* purpose it falls
/// under (`P ⊑ B`) — a permission gated on `research` covers a request for
/// `clinical-research`. The subsumption relation is the **caller-supplied** transitive
/// closure only (never inferred from IRI string structure), so with no taxonomy
/// supplied this is byte-for-byte the exact-IRI base case (access is never widened on
/// an unproven relation). A `neq` purpose constraint is honoured (purpose ≠ the named
/// one, AND ≠ any sub-purpose of it — a sub-purpose IS that purpose, so it stays
/// carved out). Several purpose constraints on one rule are ANDed (every one must
/// hold), mirroring the evaluator's constraint conjunction. [OPUS-4.8] sq-z3ve.
///
/// Returns [`PurposeMatch::NotConstrained`] when the rule has no purpose constraint.
///
/// # Examples
///
/// ```
/// use sparq_policy::{purpose_status, PurposeMatch, parse_policy_str, Request, Value};
/// let pol = parse_policy_str(r#"
/// @prefix odrl: <http://www.w3.org/ns/odrl/2/> .
/// <urn:pol/p> a odrl:Set ; odrl:permission [
///     odrl:action odrl:use ; odrl:target <urn:asset/x> ;
///     odrl:constraint [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ;
///                       odrl:rightOperand <urn:purpose/research> ] ] .
/// "#, "turtle").unwrap();
/// let rule = &pol.permissions[0];
/// let base = Request::new("http://www.w3.org/ns/odrl/2/use").on("urn:asset/x");
///
/// // Stated purpose matches exactly → Satisfied.
/// let ok = base.clone().for_purpose(Value::Iri("urn:purpose/research".into()));
/// assert_eq!(purpose_status(rule, &ok), PurposeMatch::Satisfied);
/// // Stated a different purpose → DefinitelyUnsatisfied.
/// let bad = base.clone().for_purpose(Value::Iri("urn:purpose/marketing".into()));
/// assert_eq!(purpose_status(rule, &bad), PurposeMatch::DefinitelyUnsatisfied);
/// // NO purpose evidence → Unprovable (fail-closed; not "any purpose").
/// assert_eq!(purpose_status(rule, &base), PurposeMatch::Unprovable);
///
/// // A NARROWER purpose + the taxonomy edge clinical ⊑ research → Satisfied
/// // (a research permission covers clinical research). [OPUS-4.8] sq-z3ve.
/// let sub = base.clone()
///     .for_purpose(Value::Iri("urn:purpose/research/clinical".into()))
///     .with_purpose_subsumption("urn:purpose/research/clinical", "urn:purpose/research");
/// assert_eq!(purpose_status(rule, &sub), PurposeMatch::Satisfied);
/// ```
pub fn purpose_status(rule: &Rule, request: &Request) -> PurposeMatch {
    let mut constrained = false;
    let mut any_unprovable = false;
    for c in &rule.constraints {
        if c.left != ODRL_PURPOSE {
            continue;
        }
        constrained = true;
        match constraint_status(c, request) {
            ConstraintStatus::Satisfied => {}
            ConstraintStatus::DefinitelyUnsatisfied => return PurposeMatch::DefinitelyUnsatisfied,
            ConstraintStatus::Unprovable => any_unprovable = true,
        }
    }
    if !constrained {
        PurposeMatch::NotConstrained
    } else if any_unprovable {
        PurposeMatch::Unprovable
    } else {
        PurposeMatch::Satisfied
    }
}

/// The three-valued verdict of a rule's `odrl:recipient` constraints against the
/// recipient evidence a request carries — a FAITHFUL report of exactly what
/// [`evaluate`] checks for recipient (it reuses the same `constraint_status` the
/// evaluator's recipient constraints go through). [OPUS-4.8] sq-5037.
///
/// The honesty contract mirrors [`PurposeMatch`]: this never claims a stronger verdict
/// than the evaluator acts on. `Satisfied` is the ONLY verdict under which a
/// recipient-gated permission grants; anything other than `DefinitelyUnsatisfied`
/// (i.e. `Satisfied` OR `Unprovable`) keeps a recipient-gated prohibition in force.
///
/// **The `neq` / "everyone-except" shape:** a `recipient neq X` constraint is
/// `Satisfied` for any recipient that is NOT `X`, `DefinitelyUnsatisfied` for the
/// recipient `X` (the carved-out party), and `Unprovable` when the request supplies
/// no identity at all (no `odrl:recipient` context AND no [`Request::party`]) —
/// fail-closed: a `neq` permission does NOT grant to an unknown recipient.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecipientMatch {
    /// The rule has no `odrl:recipient` constraint — recipient places no restriction.
    NotConstrained,
    /// The rule constrains recipient AND the request's recipient **satisfies** every
    /// recipient constraint (for `neq X`: the recipient is some party other than `X`).
    Satisfied,
    /// The rule constrains recipient but the request carries **no** recipient evidence
    /// (no `odrl:recipient` context and no requesting party) — *unprovable*.
    /// Fail-closed: a permission does NOT grant; a prohibition is NOT withdrawn.
    Unprovable,
    /// The rule constrains recipient and the request's recipient **fails** the
    /// constraint (for `neq X`: the recipient IS the carved-out party `X`).
    DefinitelyUnsatisfied,
}

/// Report how `rule`'s `odrl:recipient` constraint(s) stand against the recipient
/// evidence `request` carries — the auditable surface of faithful recipient (incl.
/// `neq` / "everyone-except") enforcement. [OPUS-4.8] sq-5037.
///
/// Like [`purpose_status`], this does NOT re-implement matching: it runs the request
/// through the SAME `constraint_status` every `odrl:recipient` constraint goes
/// through inside [`evaluate`], so the verdict it reports is exactly what the
/// evaluator acts on (no claimed enforcement that isn't actually checked). The
/// recipient evidence is the explicit `odrl:recipient` context value, or — when none
/// is set — the requesting [`Request::party`] (the recipient-of-data is who is asking;
/// see `resolve_actual`).
///
/// Returns [`RecipientMatch::NotConstrained`] when the rule has no recipient constraint.
///
/// # Examples
///
/// ```
/// use sparq_policy::{recipient_status, RecipientMatch, parse_policy_str, Request};
/// // "everyone EXCEPT bob may receive the data"
/// let pol = parse_policy_str(r#"
/// @prefix odrl: <http://www.w3.org/ns/odrl/2/> .
/// <urn:pol/p> a odrl:Set ; odrl:permission [
///     odrl:action odrl:read ; odrl:target <urn:asset/x> ;
///     odrl:constraint [ odrl:leftOperand odrl:recipient ; odrl:operator odrl:neq ;
///                       odrl:rightOperand <https://bob.ex/card#me> ] ] .
/// "#, "turtle").unwrap();
/// let rule = &pol.permissions[0];
/// let base = Request::new("http://www.w3.org/ns/odrl/2/read").on("urn:asset/x");
///
/// // Some other party → Satisfied (not the excluded one).
/// let carol = base.clone().by("https://carol.ex/card#me");
/// assert_eq!(recipient_status(rule, &carol), RecipientMatch::Satisfied);
/// // The excluded party → DefinitelyUnsatisfied.
/// let bob = base.clone().by("https://bob.ex/card#me");
/// assert_eq!(recipient_status(rule, &bob), RecipientMatch::DefinitelyUnsatisfied);
/// // No identity at all → Unprovable (fail-closed; not "any recipient").
/// assert_eq!(recipient_status(rule, &base), RecipientMatch::Unprovable);
/// ```
pub fn recipient_status(rule: &Rule, request: &Request) -> RecipientMatch {
    let mut constrained = false;
    let mut any_unprovable = false;
    for c in &rule.constraints {
        if c.left != ODRL_RECIPIENT {
            continue;
        }
        constrained = true;
        match constraint_status(c, request) {
            ConstraintStatus::Satisfied => {}
            ConstraintStatus::DefinitelyUnsatisfied => {
                return RecipientMatch::DefinitelyUnsatisfied
            }
            ConstraintStatus::Unprovable => any_unprovable = true,
        }
    }
    if !constrained {
        RecipientMatch::NotConstrained
    } else if any_unprovable {
        RecipientMatch::Unprovable
    } else {
        RecipientMatch::Satisfied
    }
}

/// The three-valued verdict of a rule's `odrl:dateTime` (time-window) constraints
/// against the evaluation-time evidence a request carries — a FAITHFUL report of
/// exactly what [`evaluate`] checks for the clock (it reuses the same
/// `constraint_status` the evaluator's `odrl:dateTime` constraints go through).
/// [OPUS-4.8] sq-idnv.
///
/// The honesty contract mirrors [`PurposeMatch`] / [`RecipientMatch`]: this never
/// claims a stronger verdict than the evaluator acts on. `Satisfied` is the ONLY
/// verdict under which a time-gated permission grants; anything other than
/// `DefinitelyUnsatisfied` (i.e. `Satisfied` OR `Unprovable`) keeps a time-gated
/// prohibition in force.
///
/// **The time-window shape:** a `dateTime lteq T` (or `lt`/`gteq`/`gt`/`eq`/`neq T`)
/// constraint is `Satisfied` when the request's instant meets the bound,
/// `DefinitelyUnsatisfied` when the request supplies an instant that fails the bound
/// (e.g. a `lteq` upper edge that has provably lapsed), and `Unprovable` when the
/// request supplies no time at all — fail-closed: a time-gated permission does NOT
/// grant on an unknown clock, and a time-gated prohibition is NOT withdrawn. Several
/// `dateTime` constraints on one rule are ANDed (a two-sided window `gteq lower` +
/// `lteq upper` must both hold), mirroring the evaluator's constraint conjunction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateTimeMatch {
    /// The rule has no `odrl:dateTime` constraint — time places no restriction on it.
    NotConstrained,
    /// The rule constrains time AND the request's instant **meets** every time
    /// constraint (inside the window).
    Satisfied,
    /// The rule constrains time but the request carries **no** time evidence —
    /// *unprovable*. Fail-closed: a permission does NOT grant; a prohibition is NOT
    /// withdrawn. (Never silently read as "any time allowed".)
    Unprovable,
    /// The rule constrains time and the request's instant **fails** the window — a
    /// definite no (the window has not opened yet, or has provably lapsed).
    DefinitelyUnsatisfied,
}

/// Report how `rule`'s `odrl:dateTime` (time-window) constraint(s) stand against the
/// evaluation-time evidence `request` carries — the auditable surface of faithful
/// temporal enforcement. [OPUS-4.8] sq-idnv.
///
/// Like [`purpose_status`] / [`recipient_status`], this does NOT re-implement
/// matching: it runs the request through the SAME `constraint_status` every
/// `odrl:dateTime` constraint goes through inside [`evaluate`], so the verdict it
/// reports is exactly what the evaluator acts on (no claimed enforcement that isn't
/// actually checked — the whole point of the bead). The time evidence is the
/// `odrl:dateTime` context value the request supplies (via [`Request::at`]).
///
/// Returns [`DateTimeMatch::NotConstrained`] when the rule has no time constraint.
///
/// # Examples
///
/// ```
/// use sparq_policy::{datetime_status, DateTimeMatch, parse_policy_str, Request};
/// // "may read until 2026-12-31" (a half-open validity window).
/// let pol = parse_policy_str(r#"
/// @prefix odrl: <http://www.w3.org/ns/odrl/2/> .
/// @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
/// <urn:pol/p> a odrl:Set ; odrl:permission [
///     odrl:action odrl:read ; odrl:target <urn:asset/x> ;
///     odrl:constraint [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lteq ;
///                       odrl:rightOperand "2026-12-31T23:59:59Z"^^xsd:dateTime ] ] .
/// "#, "turtle").unwrap();
/// let rule = &pol.permissions[0];
/// let base = Request::new("http://www.w3.org/ns/odrl/2/read").on("urn:asset/x");
///
/// // Inside the window → Satisfied.
/// let inside = base.clone().at("2026-06-16T09:00:00Z");
/// assert_eq!(datetime_status(rule, &inside), DateTimeMatch::Satisfied);
/// // After the window → DefinitelyUnsatisfied.
/// let lapsed = base.clone().at("2027-03-01T00:00:00Z");
/// assert_eq!(datetime_status(rule, &lapsed), DateTimeMatch::DefinitelyUnsatisfied);
/// // No time at all → Unprovable (fail-closed; not "any time").
/// assert_eq!(datetime_status(rule, &base), DateTimeMatch::Unprovable);
/// ```
pub fn datetime_status(rule: &Rule, request: &Request) -> DateTimeMatch {
    let mut constrained = false;
    let mut any_unprovable = false;
    for c in &rule.constraints {
        if c.left != ODRL_DATETIME {
            continue;
        }
        constrained = true;
        match constraint_status(c, request) {
            ConstraintStatus::Satisfied => {}
            ConstraintStatus::DefinitelyUnsatisfied => return DateTimeMatch::DefinitelyUnsatisfied,
            ConstraintStatus::Unprovable => any_unprovable = true,
        }
    }
    if !constrained {
        DateTimeMatch::NotConstrained
    } else if any_unprovable {
        DateTimeMatch::Unprovable
    } else {
        DateTimeMatch::Satisfied
    }
}

/// The three-valued verdict of a rule's `odrl:spatial` constraints against the spatial
/// (region) evidence a request carries — a FAITHFUL report of exactly what [`evaluate`]
/// checks for the location (it reuses the same `constraint_status` the evaluator's
/// `odrl:spatial` constraints go through). [OPUS-4.8] sq-wukl.
///
/// The honesty contract mirrors [`PurposeMatch`] / [`RecipientMatch`] / [`DateTimeMatch`]:
/// it never claims a stronger verdict than the evaluator acts on. `Satisfied` is the ONLY
/// verdict under which a spatially-gated permission grants; anything other than
/// `DefinitelyUnsatisfied` (i.e. `Satisfied` OR `Unprovable`) keeps a spatially-gated
/// prohibition in force.
///
/// **The spatial-tree / `isPartOf` shape:** a `spatial isPartOf <Region>` constraint is
/// `Satisfied` when the request's stated region IS the named region or is **transitively
/// part-of** it under the region `isPartOf` tree the request supplies as subsumption
/// evidence (via [`Request::with_purpose_subsumption`] / [`Request::with_purpose_taxonomy`]
/// — e.g. `Berlin ⊑ DEU ⊑ EU` for a `spatial isPartOf EU` rule, the SAME caller-supplied
/// closure the DPV purpose taxonomy uses), `DefinitelyUnsatisfied` when the stated region
/// is provably outside the named region (no edge path reaches it), and `Unprovable` when
/// the request supplies no region at all — fail-closed. **No invented subsumption:** a
/// sub-region grants only when the request asserts the `isPartOf` edge; a missing edge
/// fails closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpatialMatch {
    /// The rule has no `odrl:spatial` constraint — location places no restriction on it.
    NotConstrained,
    /// The rule constrains location AND the request's region **meets** every spatial
    /// constraint (the named region, or part-of it under the supplied tree).
    Satisfied,
    /// The rule constrains location but the request carries **no** region evidence —
    /// *unprovable*. Fail-closed: a permission does NOT grant; a prohibition is NOT
    /// withdrawn. (Never silently read as "anywhere allowed".)
    Unprovable,
    /// The rule constrains location and the request's region **fails** the constraint —
    /// a definite no (outside the named region; no `isPartOf` path reaches it).
    DefinitelyUnsatisfied,
}

/// Report how `rule`'s `odrl:spatial` constraint(s) stand against the region evidence
/// `request` carries — the auditable surface of faithful spatial (incl. `isPartOf`-tree)
/// enforcement. [OPUS-4.8] sq-wukl.
///
/// Like [`purpose_status`] / [`recipient_status`] / [`datetime_status`], this does NOT
/// re-implement matching: it runs the request through the SAME `constraint_status`
/// every `odrl:spatial` constraint goes through inside [`evaluate`] (including the
/// transitive `isPartOf` match over the request's supplied subsumption closure — see
/// [`Request::with_purpose_subsumption`]), so the verdict it reports is exactly what the
/// evaluator acts on — no claimed enforcement that isn't actually checked.
///
/// Returns [`SpatialMatch::NotConstrained`] when the rule has no spatial constraint.
///
/// # Examples
///
/// ```
/// use sparq_policy::{spatial_status, SpatialMatch, parse_policy_str, Request, Value, ODRL_SPATIAL};
/// // "may distribute to anywhere in the EU" — spatial isPartOf EU.
/// let pol = parse_policy_str(r#"
/// @prefix odrl: <http://www.w3.org/ns/odrl/2/> .
/// <urn:pol/geo> a odrl:Set ; odrl:permission [
///     odrl:action odrl:distribute ; odrl:target <urn:asset/x> ;
///     odrl:constraint [ odrl:leftOperand odrl:spatial ; odrl:operator odrl:isPartOf ;
///                       odrl:rightOperand <urn:country/EU> ] ] .
/// "#, "turtle").unwrap();
/// let rule = &pol.permissions[0];
/// let base = Request::new("http://www.w3.org/ns/odrl/2/distribute").on("urn:asset/x");
///
/// // Germany part-of EU (edge supplied) → Satisfied.
/// let de = base.clone()
///     .with(ODRL_SPATIAL, Value::Iri("urn:country/DEU".into()))
///     .with_purpose_subsumption("urn:country/DEU", "urn:country/EU");
/// assert_eq!(spatial_status(rule, &de), SpatialMatch::Satisfied);
/// // A region outside EU → DefinitelyUnsatisfied.
/// let us = base.clone().with(ODRL_SPATIAL, Value::Iri("urn:country/USA".into()));
/// assert_eq!(spatial_status(rule, &us), SpatialMatch::DefinitelyUnsatisfied);
/// // No region at all → Unprovable (fail-closed; not "anywhere").
/// assert_eq!(spatial_status(rule, &base), SpatialMatch::Unprovable);
/// ```
pub fn spatial_status(rule: &Rule, request: &Request) -> SpatialMatch {
    let mut constrained = false;
    let mut any_unprovable = false;
    for c in &rule.constraints {
        if c.left != ODRL_SPATIAL {
            continue;
        }
        constrained = true;
        match constraint_status(c, request) {
            ConstraintStatus::Satisfied => {}
            ConstraintStatus::DefinitelyUnsatisfied => return SpatialMatch::DefinitelyUnsatisfied,
            ConstraintStatus::Unprovable => any_unprovable = true,
        }
    }
    if !constrained {
        SpatialMatch::NotConstrained
    } else if any_unprovable {
        SpatialMatch::Unprovable
    } else {
        SpatialMatch::Satisfied
    }
}

/// How one prohibition rule re-evaluates against a request, distinguishing a
/// *definite* non-match from an *unprovable* one (missing-evidence constraint).
/// [OPUS-4.8] sq-2pcf.
enum RuleClass {
    /// Structural attributes agree AND every constraint is satisfied.
    Match,
    /// Structural attributes agree, but a constraint is unprovable (no evidence).
    Ambiguous,
    /// A structural attribute disagrees, OR a constraint is *definitely* false
    /// (evidence present, comparison failed), OR a constraint is malformed.
    DefinitelyNo,
}

/// Classify a single prohibition rule for deny-retraction. Structural attributes
/// (action / target / assignee) are definite (no "world state" needed). Constraints
/// split into *definitely false* (evidence present, bound not met — a definite no) vs
/// *unprovable* (no evidence for the dimension — ambiguous). [OPUS-4.8] sq-2pcf.
fn classify_prohibition(rule: &Rule, request: &Request, req_action: &Action) -> RuleClass {
    // Structural attributes are definite: if any disagrees the rule no longer names
    // this request at all (a genuine withdrawal of *this* carve-out). Action/target/
    // assignee use the SAME action-hierarchy + collection-membership matching as the
    // grant path (sq-euhr3 / sq-k7itg) so a deny carved by a collection/`use` rule is
    // classified consistently.
    if !rule.action.permits(req_action) {
        return RuleClass::DefinitelyNo;
    }
    if let Some(t) = &rule.target {
        if !request.asset_matches(t) {
            return RuleClass::DefinitelyNo;
        }
    }
    if let Some(a) = &rule.assignee {
        if !request.party_matches(a) {
            return RuleClass::DefinitelyNo;
        }
    }
    // Structural attributes agree — the constraints decide. A constraint that is
    // *definitely* false (we have evidence and it fails) is a definite no; one that is
    // unprovable for lack of evidence is ambiguous (a deny must NOT be retracted on it).
    // Atomic and compound (logical) constraints fold in identically.
    let mut any_ambiguous = false;
    for c in &rule.constraints {
        match constraint_status(c, request) {
            ConstraintStatus::Satisfied => {}
            ConstraintStatus::DefinitelyUnsatisfied => return RuleClass::DefinitelyNo,
            ConstraintStatus::Unprovable => any_ambiguous = true,
        }
    }
    for lc in &rule.logical_constraints {
        match logical_constraint_status(lc, request) {
            ConstraintStatus::Satisfied => {}
            ConstraintStatus::DefinitelyUnsatisfied => return RuleClass::DefinitelyNo,
            ConstraintStatus::Unprovable => any_ambiguous = true,
        }
    }
    if any_ambiguous {
        RuleClass::Ambiguous
    } else {
        RuleClass::Match
    }
}

/// Whether one constraint is satisfied, *definitely* unsatisfied, or *unprovable* for
/// lack of evidence — the three-valued refinement of [`constraint_satisfied`] that
/// deny-retraction needs (a plain bool conflates the last two). [OPUS-4.8] sq-2pcf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConstraintStatus {
    /// The request supplies evidence and the comparison holds.
    Satisfied,
    /// The request supplies evidence and the comparison FAILS — a definite no.
    DefinitelyUnsatisfied,
    /// The request supplies no value for this dimension — we cannot tell.
    Unprovable,
}

fn constraint_status(c: &Constraint, request: &Request) -> ConstraintStatus {
    match resolve_actual(c, request) {
        None => ConstraintStatus::Unprovable,
        Some(actual) => {
            if compare_constraint(c, actual, request) {
                ConstraintStatus::Satisfied
            } else {
                ConstraintStatus::DefinitelyUnsatisfied
            }
        }
    }
}

/// The three-valued verdict of one [`ConstraintNode`] operand — an atomic constraint
/// goes through [`constraint_status`], a nested compound recurses through
/// [`logical_constraint_status`]. [OPUS-4.8] sq-a0zef.
fn constraint_node_status(node: &ConstraintNode, request: &Request) -> ConstraintStatus {
    match node {
        ConstraintNode::Atomic(c) => constraint_status(c, request),
        ConstraintNode::Compound(lc) => logical_constraint_status(lc, request),
    }
}

/// The three-valued verdict of a compound `odrl:LogicalConstraint` against a request —
/// each operand's three-valued [`constraint_status`] folded through the combinator,
/// **fail-closed** (an indeterminate operand never silently satisfies an `and`/`xone`).
/// [OPUS-4.8] sq-a0zef.
///
/// Combinator semantics (operands carry one of `Satisfied` / `DefinitelyUnsatisfied` /
/// `Unprovable`):
///
/// - **`and`** — `Satisfied` iff EVERY operand is `Satisfied`; `DefinitelyUnsatisfied`
///   iff ANY operand is `DefinitelyUnsatisfied` (one definite false sinks the
///   conjunction regardless of unprovable siblings); else `Unprovable` (some operand is
///   unprovable, none definitely false). An **empty** operand set is `Unprovable`
///   (a compound that asserts nothing is not a positive grant — fail-closed).
/// - **`or`** — `Satisfied` iff ANY operand is `Satisfied`; `DefinitelyUnsatisfied` iff
///   EVERY operand is `DefinitelyUnsatisfied`; else `Unprovable`. An empty operand set is
///   `DefinitelyUnsatisfied` (a disjunction with no operand can never hold).
/// - **`xone`** — exclusive-or, `Satisfied` iff EXACTLY ONE operand is `Satisfied` AND
///   no operand is `Unprovable` (an unprovable operand could be the disqualifying second
///   true, so the exact-one count is not provable → `Unprovable`, never silently
///   `Satisfied`). `DefinitelyUnsatisfied` iff the count of `Satisfied` is provably ≠ 1
///   (0 with no unprovable operand, or ≥ 2). Otherwise `Unprovable`.
fn logical_constraint_status(lc: &LogicalConstraint, request: &Request) -> ConstraintStatus {
    use ConstraintStatus::*;
    let mut n_sat = 0usize;
    let mut n_unsat = 0usize;
    let mut n_unprov = 0usize;
    for operand in &lc.operands {
        match constraint_node_status(operand, request) {
            Satisfied => n_sat += 1,
            DefinitelyUnsatisfied => n_unsat += 1,
            Unprovable => n_unprov += 1,
        }
    }
    match lc.operator {
        LogicalOperator::And => {
            if n_unsat > 0 {
                DefinitelyUnsatisfied
            } else if n_unprov > 0 || lc.operands.is_empty() {
                Unprovable
            } else {
                Satisfied
            }
        }
        LogicalOperator::Or => {
            if n_sat > 0 {
                Satisfied
            } else if n_unprov > 0 {
                Unprovable
            } else {
                // every operand definitely-unsatisfied (incl. the empty set)
                DefinitelyUnsatisfied
            }
        }
        LogicalOperator::Xone => {
            if n_unprov > 0 {
                // An unprovable operand could flip the satisfied-count → not provable.
                Unprovable
            } else if n_sat == 1 {
                Satisfied
            } else {
                // provably 0 or ≥2 satisfied (no unprovable operands)
                DefinitelyUnsatisfied
            }
        }
    }
}

/// Whether a constraint's `leftOperand` is a **taxonomic** dimension that subsumption
/// (the request's supplied `narrower ⊑ broader` closure) applies to — `odrl:purpose`
/// (a DPV/purpose taxonomy — [OPUS-4.8] sq-z3ve) and `odrl:spatial` (a region
/// `isPartOf` tree — [OPUS-4.8] sq-wukl). Every other dimension matches by exact
/// value / magnitude only.
fn is_subsumable_dimension(left: &str) -> bool {
    left == ODRL_PURPOSE || left == ODRL_SPATIAL
}

/// Compare a constraint's `actual` request value against its bound, applying
/// **subsumption** for the taxonomic dimensions — `odrl:purpose` (a DPV/purpose
/// taxonomy) and `odrl:spatial` (a region `isPartOf` tree). The stated value is
/// matched against the constraint value *and every broader value* the request's
/// supplied subsumption closure proves it falls under (see
/// [`Request::with_purpose_subsumption`]); every other dimension is the plain
/// [`compare`]. One source of truth for both [`constraint_status`] and
/// [`constraint_satisfied`]. [OPUS-4.8] sq-z3ve / sq-wukl.
///
/// The `odrl:recipient` dimension is additionally **party-collection resolvable**
/// ([FABLE-5] sq-c2aze): when the request supplies party-membership evidence
/// ([`Request::with_party_membership`]), a recipient bound may name an
/// `odrl:PartyCollection` the recipient is a *member* of — the same
/// equality-or-membership lookup the `odrl:assignee` field gets via
/// `Request::party_matches`. With no membership evidence, recipient matching is
/// byte-for-byte the flat base case (access is never widened on absent evidence).
fn compare_constraint(c: &Constraint, actual: &Value, request: &Request) -> bool {
    if is_subsumable_dimension(&c.left) && !request.purpose_subsumes.is_empty() {
        return compare_subsumed(actual, c.operator, &c.right, request);
    }
    if c.left == ODRL_RECIPIENT && !request.party_memberships.is_empty() {
        return compare_recipient(actual, c.operator, &c.right, request);
    }
    compare(actual, c.operator, &c.right)
}

/// Subsumption-aware comparison for a taxonomic constraint (`odrl:purpose`,
/// `odrl:spatial`). The stated value `actual` matches the named value `bound` when it
/// equals it OR is a transitively-narrower value under the request's supplied closure
/// (`actual ⊑ bound`) — `clinical ⊑ research` for purpose, `DEU ⊑ EU` for spatial.
/// [OPUS-4.8] sq-z3ve / sq-wukl.
///
/// - `eq`/`isA`: `actual ⊑ bound`.
/// - `neq`: NOT `actual ⊑ bound` — a sub-value of the excluded value is also excluded
///   (a sub-purpose IS that purpose; a sub-region IS in that region), so the carve-out
///   is not widened away.
/// - `isPartOf` / `isAnyOf`: `actual ⊑ some member` of the named set (the spatial
///   `isPartOf EU` region-tree case, and the purpose-set-with-hierarchy case;
///   `isAnyOf` is the same set-membership relation — [FABLE-5] sq-uaz85).
/// - `isNoneOf`: `actual ⊑ NO member` of the named set — a sub-value of an excluded
///   member is ALSO excluded (mirrors `neq`: the carve-out is not widened away), and
///   the same representability guard as the flat [`is_none_of`] applies (fail-closed).
/// - order operators (`lt`/`lteq`/`gt`/`gteq`): a taxonomic value is not orderable, so
///   these delegate to the plain [`compare`] (which already fail-closes on non-orderable).
fn compare_subsumed(actual: &Value, op: Operator, bound: &Value, request: &Request) -> bool {
    let a = actual.as_str();
    match op {
        Operator::Eq | Operator::IsA => request.purpose_subsumed_by(a, bound.as_str()),
        Operator::Neq => !request.purpose_subsumed_by(a, bound.as_str()),
        Operator::IsPartOf | Operator::IsAnyOf => bound
            .as_str()
            .split(['|', ' ', ','])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .any(|member| request.purpose_subsumed_by(a, member)),
        Operator::IsNoneOf => {
            set_negation_representable(actual, bound)
                && !bound
                    .as_str()
                    .split(['|', ' ', ','])
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .any(|member| request.purpose_subsumed_by(a, member))
        }
        Operator::Lt | Operator::Lteq | Operator::Gt | Operator::Gteq => compare(actual, op, bound),
    }
}

/// Party-collection-aware comparison for the `odrl:recipient` dimension — invoked
/// only when the request supplies [`Request::party_memberships`] evidence (see
/// [`compare_constraint`]). [FABLE-5] sq-c2aze.
///
/// A recipient may be a party **or a member of an `odrl:PartyCollection`**: a
/// recipient value `r` matches a bound member `m` when `r == m` OR the request's
/// caller-supplied membership evidence asserts `r odrl:partOf m` — the SAME
/// equality-or-membership lookup [`Request::party_matches`] applies to the
/// `odrl:assignee` field. So a `recipient isPartOf <PartyCollectionIRI>` constraint
/// is satisfied by a *member* of that collection, not only by the collection IRI.
///
/// - `eq`/`isA`: the recipient IS the named party/collection or a member of it.
/// - `neq`: the negative dual — a member of the excluded collection is ALSO excluded
///   (the carve-out is not widened away; mirrors the taxonomic `neq`).
/// - `isPartOf`/`isAnyOf`: identity-or-membership against ANY member of the
///   `|`/space/comma set.
/// - `isNoneOf`: identity-or-membership against NO member (same representability
///   fail-closed guard as the flat [`is_none_of`]).
/// - order operators: not meaningful on a recipient — delegate to the plain
///   [`compare`] (which fail-closes on non-orderable values).
///
/// **Soundness:** membership draws ONLY on the caller-supplied `party_memberships`
/// set (read out of the state-of-the-world, never inferred). With an empty set this
/// function is never reached and recipient matching is byte-for-byte the flat base
/// case — access is never widened on absent evidence.
fn compare_recipient(actual: &Value, op: Operator, bound: &Value, request: &Request) -> bool {
    let r = actual.as_str();
    let member_match = |m: &str| -> bool {
        r == m
            || request
                .party_memberships
                .contains(&(r.to_owned(), m.to_owned()))
    };
    let any_in_set = |bound: &Value| -> bool {
        bound
            .as_str()
            .split(['|', ' ', ','])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .any(&member_match)
    };
    match op {
        Operator::Eq | Operator::IsA => member_match(bound.as_str()),
        Operator::Neq => !member_match(bound.as_str()),
        Operator::IsPartOf | Operator::IsAnyOf => any_in_set(bound),
        Operator::IsNoneOf => set_negation_representable(actual, bound) && !any_in_set(bound),
        Operator::Lt | Operator::Lteq | Operator::Gt | Operator::Gteq => compare(actual, op, bound),
    }
}

/// The *actual* world value a constraint's `leftOperand` is compared against, or
/// `None` when the request supplies no evidence for that dimension (⇒ unprovable /
/// fail-closed). [OPUS-4.8] sq-5037.
///
/// Dimensions take their value from [`Request::context`] keyed by the left operand —
/// EXCEPT `odrl:recipient`, where the recipient-of-data IS the requesting party: a
/// request that names a `party` but supplies no explicit `odrl:recipient` context is
/// read as `recipient = party`. This is what makes a `recipient neq X` rule actually
/// gate on *who is asking* end-to-end. An explicit `odrl:recipient` context value (if
/// the caller sets one) still takes precedence — the disclosure target need not be the
/// authenticated principal in every deployment.
fn resolve_actual<'r>(c: &Constraint, request: &'r Request) -> Option<&'r Value> {
    if let Some(v) = request.context.get(&c.left) {
        return Some(v);
    }
    if c.left == ODRL_RECIPIENT {
        return request.recipient_party.as_ref();
    }
    None
}

/// Outcome of matching a single rule against a request.
struct Match {
    is_match: bool,
    reasons: Vec<String>,
}

fn rule_matches(rule: &Rule, request: &Request, req_action: &Action) -> Match {
    let mut reasons = Vec::new();

    // Action: the rule's action must permit the requested action (the ODRL action
    // hierarchy — `use` subsumes its sub-actions but not the transfer subtree, sq-euhr3).
    if !rule.action.permits(req_action) {
        reasons.push(format!(
            "rule {} action {} != requested {}",
            rule.id, rule.action, request.action
        ));
        return Match {
            is_match: false,
            reasons,
        };
    }

    // Target: if the rule names a target, the request target must equal it OR be a
    // member of it (an odrl:AssetCollection — sq-k7itg).
    if let Some(t) = &rule.target {
        if !request.asset_matches(t) {
            reasons.push(format!(
                "rule {} target {t} != requested {:?}",
                rule.id, request.target
            ));
            return Match {
                is_match: false,
                reasons,
            };
        }
    }

    // Assignee: if the rule names an assignee party, the requester must equal it OR be
    // a member of it (an odrl:PartyCollection — sq-k7itg).
    if let Some(a) = &rule.assignee {
        if !request.party_matches(a) {
            reasons.push(format!(
                "rule {} assignee {a} != requester {:?}",
                rule.id, request.party
            ));
            return Match {
                is_match: false,
                reasons,
            };
        }
    }

    // Compound logical constraints: every one must be satisfied (logical AND with the
    // atomic constraints — sq-a0zef). Fail-closed: an indeterminate compound is unsat.
    for lc in &rule.logical_constraints {
        if logical_constraint_status(lc, request) != ConstraintStatus::Satisfied {
            reasons.push(format!(
                "rule {} logical constraint {} ({:?}) unsatisfied",
                rule.id, lc.id, lc.operator
            ));
            return Match {
                is_match: false,
                reasons,
            };
        }
    }

    // Constraints: every one must be satisfied (logical AND).
    for c in &rule.constraints {
        if !constraint_satisfied(c, request) {
            reasons.push(format!(
                "rule {} constraint ({} {:?} {}) unsatisfied",
                rule.id, c.left, c.operator, c.right
            ));
            return Match {
                is_match: false,
                reasons,
            };
        }
    }

    Match {
        is_match: true,
        reasons,
    }
}

/// Is a single constraint satisfied by the request context?
///
/// The request supplies the *actual* value for the constraint's `leftOperand`
/// (e.g. the actual request time for `odrl:dateTime`, or the requesting party for
/// `odrl:recipient` — see [`resolve_actual`]); the constraint's `rightOperand` is the
/// bound. A constraint whose left operand has **no** value (no context value AND, for
/// recipient, no party) is **unsatisfied** (fail-closed: we cannot prove the world
/// meets a constraint we have no evidence about — including a `recipient neq X`
/// constraint with no identity to compare).
fn constraint_satisfied(c: &Constraint, request: &Request) -> bool {
    let Some(actual) = resolve_actual(c, request) else {
        return false; // no evidence for this dimension → fail-closed
    };
    compare_constraint(c, actual, request)
}

/// Compare an actual request value against a constraint right-operand under an
/// operator. Numeric and dateTime operands compare by magnitude; everything
/// else compares by string/IRI value. An order comparison (`lt`/`gt`/…) on
/// non-orderable values is **false** (fail-closed).
///
/// This is the **exact / flat** base case (no subsumption). The taxonomic dimensions
/// (`odrl:purpose`, `odrl:spatial`) route through [`compare_subsumed`] instead when the
/// request supplies a subsumption closure — see [`compare_constraint`].
fn compare(actual: &Value, op: Operator, bound: &Value) -> bool {
    match op {
        Operator::Eq | Operator::IsA => value_eq(actual, bound),
        Operator::Neq => !value_eq(actual, bound),
        // `isAnyOf` (sq-uaz85) is the same set-membership relation `isPartOf` uses in
        // this flat single-value base case: the actual value equals AT LEAST ONE
        // member of the right-operand set (same `|`/space/comma encoding).
        Operator::IsPartOf | Operator::IsAnyOf => is_part_of(actual, bound),
        Operator::IsNoneOf => is_none_of(actual, bound),
        Operator::Lt | Operator::Lteq | Operator::Gt | Operator::Gteq => {
            let Some(ord) = order(actual, bound) else {
                return false;
            };
            match op {
                Operator::Lt => ord == std::cmp::Ordering::Less,
                Operator::Lteq => ord != std::cmp::Ordering::Greater,
                Operator::Gt => ord == std::cmp::Ordering::Greater,
                Operator::Gteq => ord != std::cmp::Ordering::Less,
                _ => unreachable!(),
            }
        }
    }
}

/// Insert one `narrower ⊑ broader` purpose edge into `set`, maintaining the
/// **transitive closure** so a subsumption check stays a single membership test.
/// Adding `a⊑b` also derives `a⊑c` for every already-known `b⊑c`, and `x⊑b` for
/// every already-known `x⊑a`. Self-edges (`a⊑a`) are dropped (handled by the
/// reflexive `p == target` short-circuit in [`Request::purpose_subsumed_by`]).
/// [OPUS-4.8] sq-z3ve.
fn insert_subsumption(set: &mut BTreeSet<(String, String)>, narrower: String, broader: String) {
    if narrower == broader || set.contains(&(narrower.clone(), broader.clone())) {
        return;
    }
    // Ancestors of `broader` (incl. `broader`) and descendants of `narrower`
    // (incl. `narrower`) — every (desc, anc) pair becomes an edge of the closure.
    let mut ancestors: Vec<String> = vec![broader.clone()];
    ancestors.extend(
        set.iter()
            .filter(|(n, _)| n == &broader)
            .map(|(_, b)| b.clone()),
    );
    let mut descendants: Vec<String> = vec![narrower.clone()];
    descendants.extend(
        set.iter()
            .filter(|(_, b)| b == &narrower)
            .map(|(n, _)| n.clone()),
    );
    for d in &descendants {
        for a in &ancestors {
            if d != a {
                set.insert((d.clone(), a.clone()));
            }
        }
    }
}

fn value_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Num(x), Value::Num(y)) => x == y,
        // Two dateTimes are equal iff they denote the same *instant* — a
        // mixed-offset pair like `12:00:00Z` and `14:00:00+02:00` is equal even
        // though the lexical forms differ (sq-qj2q). Fall back to lexical only
        // when an operand is not a parseable instant.
        (Value::DateTime(x), Value::DateTime(y)) => match (parse_instant(x), parse_instant(y)) {
            (Some(ix), Some(iy)) => ix == iy,
            _ => x == y,
        },
        // Cross-type: compare by canonical string (an IRI right-operand vs an IRI
        // actual, a string code vs a string code).
        _ => a.as_str() == b.as_str(),
    }
}

/// `isPartOf` / set membership: the right operand is a `|`-or-space-separated
/// set (the common compact encoding) OR a single IRI/string the actual must
/// equal. We treat the right operand's string as a set: actual ∈ set.
///
/// This is the **flat** base case. Transitive `isPartOf` over a taxonomy (a DPV
/// purpose subtree, a spatial region tree) is handled by [`compare_subsumed`] using the
/// request's caller-supplied subsumption closure — see [`Request::with_purpose_subsumption`].
fn is_part_of(actual: &Value, bound: &Value) -> bool {
    let a = actual.as_str();
    bound
        .as_str()
        .split(['|', ' ', ','])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .any(|member| member == a)
}

/// `odrl:isNoneOf` (flat base case): the actual value equals **no** member of the
/// right-operand set — the negative dual of [`is_part_of`]. [FABLE-5] sq-uaz85.
///
/// **Fail-closed representability guard:** negating a lexical set-membership test is
/// only faithful when both operands live in the IRI/string space the
/// `|`/space/comma encoding covers. A numeric operand has no lexical form here
/// (`Value::as_str` is `""`) and a dateTime's set membership would be lexical, not
/// instant-normalized — negating either gap would *satisfy* the constraint for a
/// value that IS in the set, merely encoded differently (fail-OPEN). Such operands
/// are therefore never satisfied (see [`set_negation_representable`]). An **empty**
/// set with string/IRI operands is genuinely empty, so `isNoneOf` over it is
/// (vacuously) satisfied — nothing is excluded, mirroring a `neq` no value equals.
fn is_none_of(actual: &Value, bound: &Value) -> bool {
    set_negation_representable(actual, bound) && !is_part_of(actual, bound)
}

/// Whether negating a lexical set-membership test over `(actual, bound)` is
/// faithful: both operands must be IRI/string values (the space the
/// `|`/space/comma set encoding covers). See [`is_none_of`]. [FABLE-5] sq-uaz85.
fn set_negation_representable(actual: &Value, bound: &Value) -> bool {
    matches!(actual, Value::Iri(_) | Value::Str(_)) && matches!(bound, Value::Iri(_) | Value::Str(_))
}

/// A total-ish order for orderable values: numeric by magnitude, dateTime by
/// the **instant** the lexical form denotes (mixed timezone offsets are
/// normalized to UTC before comparing — sq-qj2q). Returns `None` for
/// incomparable pairs (e.g. an unparseable dateTime under an order operator).
fn order(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (Value::Num(x), Value::Num(y)) => x.partial_cmp(y),
        (Value::DateTime(x), Value::DateTime(y)) => cmp_datetime(x, y),
        // A numeric actual against a numeric-looking string bound, or vice versa.
        _ => {
            let (Ok(x), Ok(y)) = (
                a.as_str().trim().parse::<f64>(),
                b.as_str().trim().parse::<f64>(),
            ) else {
                return None;
            };
            x.partial_cmp(&y)
        }
    }
}

/// Compare two `xsd:dateTime`/`xsd:date` lexical forms by the **instant** they
/// denote: each is normalized to a UTC timeline position (`Instant`), so a
/// mixed-offset pair such as `2026-06-16T13:00:00+02:00` (= 11:00Z) and
/// `2026-06-16T12:00:00Z` orders correctly (the offset form is *earlier*).
///
/// Returns `None` when **either** operand is not a parseable instant — an order
/// comparison against a malformed operand is undefined, and `compare` treats
/// `None` as fail-closed (the constraint is not satisfied). This is the sq-qj2q
/// fix for what was previously a raw `x.cmp(y)` lexical comparison.
///
/// **Public so other crates compare dateTime bounds by the *same* instant
/// normalizer the evaluator uses — never a divergent (lexical) one.** It is the one
/// source of truth for `xsd:dateTime` ordering across the workspace: `sparq-solid`'s
/// opt-in live-clock window re-check ([OPUS-4.8] sq-0q7n) calls this so a persisted
/// `not_before`/`not_after` window is enforced on the real UTC instant, exactly as
/// the evaluator's `odrl:dateTime` constraint was, rather than on `str::cmp`.
pub fn cmp_datetime(x: &str, y: &str) -> Option<std::cmp::Ordering> {
    Some(parse_instant(x)?.cmp(&parse_instant(y)?))
}

/// Crate-internal accessor for [`cmp_datetime`] so [`crate::compare`]'s static
/// containment analysis orders dateTime bounds by the **same** instant normalizer
/// the evaluator uses (one source of truth — no duplicated xsd:dateTime parser).
/// [OPUS-4.8] sq-zabv.
pub(crate) fn cmp_datetime_pub(x: &str, y: &str) -> Option<std::cmp::Ordering> {
    cmp_datetime(x, y)
}

/// A point on the UTC timeline, as `(days-since-epoch, nanoseconds-into-day)`
/// after applying the lexical form's timezone offset. Comparable and equatable
/// by derive — that is exactly instant ordering / instant equality.
///
/// We carry the day as a proleptic-Gregorian day number (not a calendar tuple)
/// so that an offset crossing midnight (e.g. `2026-06-15T23:00:00-02:00`
/// = `2026-06-16T01:00:00Z`) lands on the correct day without special-casing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Instant {
    /// Days since 1970-01-01 (may be negative for pre-epoch dates).
    day: i64,
    /// Nanoseconds since 00:00:00 of that UTC day, in `0..86_400_000_000_000`.
    nanos_in_day: i64,
}

/// Parse an `xsd:dateTime` or `xsd:date` lexical form into a UTC [`Instant`].
///
/// Self-contained and std-only (this crate deliberately carries no `chrono`/
/// `time` dependency — see the crate `Cargo.toml` rationale). Accepts:
/// * `YYYY-MM-DDThh:mm:ss` with optional fractional seconds (`.s…`) and an
///   optional timezone (`Z`, `+hh:mm`, or `-hh:mm`),
/// * `YYYY-MM-DD` (`xsd:date`), taken as `00:00:00` of that day, with the same
///   optional timezone suffix.
///
/// A missing timezone is treated as UTC (the policy author's intent is an
/// absolute instant; an unzoned local time has no absolute position, and
/// assuming UTC is the conventional fail-safe — the previous code likewise did
/// not localize). Returns `None` for anything not matching this grammar so the
/// caller can fail closed.
fn parse_instant(s: &str) -> Option<Instant> {
    let s = s.trim();
    // Split off the calendar date (`YYYY-MM-DD`) from the optional `Thh:mm:ss…`.
    let (date, rest) = match s.split_once('T') {
        Some((d, r)) => (d, Some(r)),
        None => (s, None),
    };
    let (year, month, day_of_month) = parse_date(date)?;
    let epoch_day = days_from_civil(year, month, day_of_month)?;

    let Some(rest) = rest else {
        // Bare `xsd:date` → midnight UTC. (`xsd:date` may itself carry a tz, but
        // by the grammar the `T` is only present for dateTime; a zoned date has
        // its offset on the date component, handled by `parse_date`'s tz split.)
        return Some(Instant {
            day: epoch_day,
            nanos_in_day: 0,
        });
    };

    // Separate the time-of-day from a trailing timezone designator.
    let (time, offset_min) = split_timezone(rest)?;
    let (hh, mm, ss, frac_nanos) = parse_time(time)?;

    // Combine to nanoseconds since local midnight, then shift by the offset to
    // reach UTC. The offset is "local = UTC + offset", so UTC = local − offset.
    let local_nanos = (hh as i64) * 3_600_000_000_000
        + (mm as i64) * 60_000_000_000
        + (ss as i64) * 1_000_000_000
        + frac_nanos;
    let utc_nanos = local_nanos - (offset_min as i64) * 60_000_000_000;

    // Normalize the offset-induced day carry/borrow into a clean day + in-day.
    let day_carry = utc_nanos.div_euclid(86_400_000_000_000);
    let nanos_in_day = utc_nanos.rem_euclid(86_400_000_000_000);
    Some(Instant {
        day: epoch_day + day_carry,
        nanos_in_day,
    })
}

/// Parse `YYYY-MM-DD`, allowing a leading `-` for BCE years (`-0044-03-15`).
fn parse_date(s: &str) -> Option<(i64, u32, u32)> {
    let (neg, body) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let mut parts = body.splitn(3, '-');
    let y: i64 = parts.next()?.parse().ok()?;
    let mo: u32 = parse_fixed(parts.next()?, 2)?;
    let d: u32 = parse_fixed(parts.next()?, 2)?;
    if parts.next().is_some() || !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    Some((if neg { -y } else { y }, mo, d))
}

/// Parse `hh:mm:ss` (with optional `.fraction`) into `(h, m, s, frac_nanos)`.
fn parse_time(s: &str) -> Option<(u32, u32, u32, i64)> {
    let (whole, frac) = match s.split_once('.') {
        Some((w, f)) => (w, Some(f)),
        None => (s, None),
    };
    let mut parts = whole.splitn(3, ':');
    let h: u32 = parse_fixed(parts.next()?, 2)?;
    let m: u32 = parse_fixed(parts.next()?, 2)?;
    let sec: u32 = parse_fixed(parts.next()?, 2)?;
    if parts.next().is_some() || h > 23 || m > 59 || sec > 60 {
        // `sec == 60` is a leap second — accept it, clamped below.
        return None;
    }
    let frac_nanos = match frac {
        None => 0,
        Some(f) => {
            if f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            // Take up to 9 significant digits (nanosecond resolution), pad/truncate.
            let mut digits = [b'0'; 9];
            for (slot, b) in digits.iter_mut().zip(f.bytes()) {
                *slot = b;
            }
            std::str::from_utf8(&digits).ok()?.parse::<i64>().ok()?
        }
    };
    // Clamp a leap second to the last representable instant of the minute.
    let sec_nanos = if sec == 60 { 59 } else { sec };
    Some((
        h,
        m,
        sec_nanos,
        if sec == 60 { 999_999_999 } else { frac_nanos },
    ))
}

/// Split a time-or-date tail into `(body, offset_minutes)`. `Z`/none ⇒ `0`.
fn split_timezone(s: &str) -> Option<(&str, i32)> {
    if let Some(body) = s.strip_suffix('Z') {
        return Some((body, 0));
    }
    // A `+hh:mm` / `-hh:mm` suffix. Scan from the right for the sign so a
    // negative *year* (handled earlier) is never confused with an offset.
    if let Some(idx) = s.rfind(['+', '-']) {
        // The sign must introduce a `±hh:mm` tail (6 chars) at the very end.
        let (body, tz) = s.split_at(idx);
        if let Some(min) = parse_offset(tz) {
            return Some((body, min));
        }
    }
    // No timezone designator → treat as UTC.
    Some((s, 0))
}

/// Parse a `±hh:mm` offset into signed minutes.
fn parse_offset(s: &str) -> Option<i32> {
    let sign = match s.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let (h, m) = s[1..].split_once(':')?;
    let hh: i32 = parse_fixed(h, 2)?;
    let mm: i32 = parse_fixed(m, 2)?;
    if hh > 14 || mm > 59 {
        return None;
    }
    Some(sign * (hh * 60 + mm))
}

/// Parse a base-10 field of exactly `width` ASCII digits.
fn parse_fixed<T: std::str::FromStr>(s: &str, width: usize) -> Option<T> {
    if s.len() != width || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// Days from 1970-01-01 to `year-month-day` (proleptic Gregorian). Adapted from
/// Howard Hinnant's `days_from_civil` (public-domain `date` algorithms): a
/// branch-free civil-to-days conversion valid for the full `i64` year range.
fn days_from_civil(year: i64, month: u32, day: u32) -> Option<i64> {
    // Reject obviously-out-of-range days-of-month for the given month.
    let dim = days_in_month(year, month)?;
    if day < 1 || day > dim {
        return None;
    }
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let m = month as i64;
    let d = day as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    Some(era * 146_097 + doe - 719_468)
}

/// Days in `month` of `year` (Gregorian leap rules).
fn days_in_month(year: i64, month: u32) -> Option<u32> {
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    Some(match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    })
}

// [OPUS-4.8] sq-qj2q — unit tests for the self-contained instant normalizer that
// backs mixed-offset dateTime comparison (the public-API behaviour is covered by
// tests/odrl_eval.rs; these pin the internal arithmetic + the rejection grammar).
#[cfg(test)]
mod instant_tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn epoch_day_anchors() {
        // 1970-01-01 is day 0; 1969-12-31 is day -1; 1972-02-29 (leap) sanity.
        assert_eq!(days_from_civil(1970, 1, 1), Some(0));
        assert_eq!(days_from_civil(1969, 12, 31), Some(-1));
        assert_eq!(days_from_civil(2000, 1, 1), Some(10_957));
        // 2000-02-29 exists (leap), 1900-02-29 does not (century non-leap).
        assert!(days_from_civil(2000, 2, 29).is_some());
        assert!(days_from_civil(1900, 2, 29).is_none());
    }

    #[test]
    fn mixed_offsets_normalize_to_same_instant() {
        // All three name 2026-06-16T11:00:00Z.
        let z = parse_instant("2026-06-16T11:00:00Z").unwrap();
        let plus = parse_instant("2026-06-16T13:00:00+02:00").unwrap();
        let minus = parse_instant("2026-06-16T09:00:00-02:00").unwrap();
        assert_eq!(z, plus);
        assert_eq!(z, minus);
        // Missing tz is treated as UTC.
        assert_eq!(z, parse_instant("2026-06-16T11:00:00").unwrap());
    }

    #[test]
    fn offset_can_borrow_or_carry_across_midnight() {
        // 2026-06-15T23:00:00-02:00 == 2026-06-16T01:00:00Z (carry forward a day).
        assert_eq!(
            parse_instant("2026-06-15T23:00:00-02:00"),
            parse_instant("2026-06-16T01:00:00Z")
        );
        // 2026-06-16T01:00:00+02:00 == 2026-06-15T23:00:00Z (borrow back a day).
        assert_eq!(
            parse_instant("2026-06-16T01:00:00+02:00"),
            parse_instant("2026-06-15T23:00:00Z")
        );
    }

    #[test]
    fn fractional_seconds_subsecond_ordering() {
        let a = parse_instant("2026-06-16T12:00:00.250Z").unwrap();
        let b = parse_instant("2026-06-16T12:00:00.750Z").unwrap();
        assert!(a < b);
        // Differing precision but equal value: .5 == .500000000.
        assert_eq!(
            parse_instant("2026-06-16T12:00:00.5Z"),
            parse_instant("2026-06-16T12:00:00.500000000Z")
        );
    }

    #[test]
    fn bare_date_is_midnight_utc() {
        assert_eq!(
            parse_instant("2026-06-16"),
            parse_instant("2026-06-16T00:00:00Z")
        );
    }

    #[test]
    fn cmp_datetime_orders_by_instant_not_lexical() {
        // Lexically "13:..+02:00" > "12:..Z", but as instants the offset form is earlier.
        assert_eq!(
            cmp_datetime("2026-06-16T13:00:00+02:00", "2026-06-16T12:00:00Z"),
            Some(Ordering::Less)
        );
        assert_eq!(
            cmp_datetime("2026-06-16T12:00:00Z", "2026-06-16T12:00:00Z"),
            Some(Ordering::Equal)
        );
    }

    #[test]
    fn malformed_inputs_rejected() {
        for bad in [
            "",
            "not-a-date",
            "2026-13-01T00:00:00Z",      // month 13
            "2026-06-32T00:00:00Z",      // day 32
            "2026-02-29T00:00:00Z",      // 2026 is not a leap year
            "2026-06-16T24:00:00Z",      // hour 24
            "2026-06-16T12:60:00Z",      // minute 60
            "2026-06-16T12:00:00+15:00", // offset > ±14:00
            "2026-6-16T00:00:00Z",       // unpadded month
            "2026-06-16T12:00:00.Z",     // empty fraction
        ] {
            assert!(parse_instant(bad).is_none(), "should reject {bad:?}");
        }
        // An order comparison against a malformed operand is undefined (fail-closed).
        assert_eq!(cmp_datetime("not-a-date", "2026-06-16T00:00:00Z"), None);
    }

    #[test]
    fn leap_second_clamped_not_rejected() {
        // xsd permits :60 (a leap second); we accept and clamp to end-of-minute.
        let leap = parse_instant("2026-06-30T23:59:60Z").unwrap();
        let next = parse_instant("2026-07-01T00:00:00Z").unwrap();
        assert!(leap < next);
    }
}

// [OPUS-4.8] sq-z3ve — unit tests for the incremental transitive-closure
// maintenance that backs purpose-subsumption matching. The public-API behaviour
// is covered by tests/odrl_eval.rs; these pin the closure arithmetic, including
// the order-independence and cycle-tolerance that a string-prefix shortcut would
// get wrong.
#[cfg(test)]
mod subsumption_tests {
    use super::*;

    fn closure(edges: &[(&str, &str)]) -> BTreeSet<(String, String)> {
        let mut s = BTreeSet::new();
        for (n, b) in edges {
            insert_subsumption(&mut s, (*n).to_owned(), (*b).to_owned());
        }
        s
    }

    fn has(s: &BTreeSet<(String, String)>, n: &str, b: &str) -> bool {
        s.contains(&(n.to_owned(), b.to_owned()))
    }

    #[test]
    fn direct_edge_recorded() {
        let s = closure(&[("a", "b")]);
        assert!(has(&s, "a", "b"));
        // No spurious reverse edge.
        assert!(!has(&s, "b", "a"));
    }

    #[test]
    fn chain_forward_is_transitive() {
        // a⊑b then b⊑c ⇒ a⊑c also recorded.
        let s = closure(&[("a", "b"), ("b", "c")]);
        assert!(has(&s, "a", "c"), "a⊑c must be derived");
    }

    #[test]
    fn chain_reverse_order_is_transitive() {
        // Insert b⊑c FIRST, then a⊑b ⇒ a⊑c still derived (order-independent).
        let s = closure(&[("b", "c"), ("a", "b")]);
        assert!(has(&s, "a", "c"), "a⊑c must be derived regardless of order");
    }

    #[test]
    fn deep_chain_full_closure() {
        // a⊑b⊑c⊑d in scrambled insertion order ⇒ every ancestor edge present.
        let s = closure(&[("c", "d"), ("a", "b"), ("b", "c")]);
        for (n, b) in [("a", "c"), ("a", "d"), ("b", "d")] {
            assert!(has(&s, n, b), "{n}⊑{b} must be derived");
        }
    }

    #[test]
    fn self_edge_dropped() {
        // A reflexive edge is never stored (the p == target short-circuit owns it).
        let s = closure(&[("a", "a")]);
        assert!(s.is_empty());
    }

    #[test]
    fn cycle_does_not_loop_forever() {
        // a⊑b, b⊑a is a (degenerate) cycle — insertion must terminate and record
        // both directions without diverging.
        let s = closure(&[("a", "b"), ("b", "a")]);
        assert!(has(&s, "a", "b") && has(&s, "b", "a"));
    }
}
