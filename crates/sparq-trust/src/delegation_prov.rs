//! # `delegation_prov.rs` — PROV-O delegation audit for human + AI agents (`sq-pfae.6`)
//!
//! The **delegation stratum's audit half**: render an invocation-bound delegation chain
//! (the [`crate::delegation`] gate) as a **minimal W3C PROV-O audit graph**, modelling the
//! human-and-AI-agent *on-behalf-of* relationship the design names in §4.2 — *"an AI agent
//! is a distinct principal (its own DID/key) holding an attenuated child of its human
//! principal's authority"* — and rendering the surviving effective capability as `auth:*`
//! RDF that reasons with the SAME `.acl`/`.acr` rules the rest of the estate uses (the §4.3
//! delegation value-add). Behind the **default-OFF `delegation-prov` feature**.
//!
//! ## Why this module exists (the gap it closes)
//!
//! The design record §4.3 / §7.4(K) flags the audit + on-behalf-of story as **proposed,
//! zero substrate**: `sparq-prov` records only a **single-agent** `prov:wasAssociatedWith`
//! per reasoner-derivation activity and has **no `prov:actedOnBehalfOf` / delegation-chain
//! modelling**, and "Human vs AI agent" is named as *an attested attribute on the delegate*
//! but never rendered. The [`crate::delegation`] gate (`sq-l5og`, merged) verifies the
//! chain and binds the invoker per request; this module records **who acted on whose
//! behalf**, as a standard PROV-O graph an auditor can replay.
//!
//! ## What the audit graph says (PROV-O, minimal)
//!
//! For an [`crate::delegation::EffectiveCapability`] that survived [`crate::delegation::invoke`]
//! over a chain `root → … → terminal`:
//!
//! - each principal `P` is a `prov:Agent`; the human root and any human delegate are also
//!   `trust:HumanPrincipal`, an AI delegate also `trust:AiAgent` (the attested attribute);
//! - for every hop `delegator → delegate`, `delegate` `prov:actedOnBehalfOf` `delegator`
//!   (the delegation chain rendered as PROV-O **on-behalf-of** edges — this is exactly the
//!   modelling `sparq-prov` lacks);
//! - the invocation is a `prov:Activity` the terminal delegate `prov:wasAssociatedWith`,
//!   the **whole chain** is recorded as a sequence of `actedOnBehalfOf` edges the activity
//!   `prov:used`, and (when supplied) the activity is time-stamped `prov:atTime`;
//! - the **effective grant** is a `prov:Entity` `prov:wasGeneratedBy` the invocation and
//!   `prov:wasAttributedTo` the terminal delegate, carrying the `auth:*` action triples the
//!   capability confers over its target — the grant rendered as RDF (the §4.3 value-add).
//!
//! ## Honest boundaries (this module adds NO security property)
//!
//! - **An audit record, never an authority source.** The graph is produced *after* a
//!   successful [`crate::delegation::invoke`] — it can only ever describe authority that
//!   already passed the gate. It is the OPTIONAL §4.1 audit record the obj-cap discipline
//!   demotes: `invoke()` still gates on the carried chain + the live PoP, never on graph
//!   presence. Feeding this graph back as input grants **nothing** (there is no admission
//!   rule that trusts a `prov:actedOnBehalfOf` triple — the §2.4 reserved-predicate guard
//!   and §3.3 "verified signature only" discipline are unchanged).
//! - **`trust:HumanPrincipal` / `trust:AiAgent` are NON-STANDARD**, invented for this
//!   proposal (the same posture as the rest of the `trust:` vocab); a WG would rehome them.
//!   They are an **attested attribute** on the delegate, not an authority — the AI agent's
//!   authority is the attenuated capability the signed chain carries.
//! - **No `sparq-prov` dependency.** The audit is a small fixed triple set, emitted directly
//!   via `oxrdf` (a handful of `prov:` IRI constants), NOT by pulling `sparq-prov` — which
//!   would drag `sparq-engine` onto this crate's dep graph and break lean-core. The PROV-O
//!   IRIs are the standard ones, so the graph round-trips through any PROV-O consumer.
//! - **No privacy / unlinkability.** Like the rest of this crate, the principals are named in
//!   the **clear**; the audit graph is the opposite of anonymous. The ZK estate is
//!   research-grade and externally UNAUDITED (`sq-qhy4`).
//!
//! [OPUS-4.8] sq-pfae.6: PROV-O delegation audit for human + AI agents (issue #940, #1190,
//! epic sq-pfae P5). Written while Fable unavailable — flag for re-review when Fable returns.
//! 🤖 SPARQ agent — trust-graph delegation PROV-O audit.

use crate::delegation::{Capability, DelegationChain, EffectiveCapability};
use oxrdf::vocab::{rdf, xsd};
use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};

/// The W3C PROV-O namespace base.
const PROV_NS: &str = "http://www.w3.org/ns/prov#";

/// `prov:` term IRI (`http://www.w3.org/ns/prov#{local}`). Every `local` here is a
/// syntactically valid IRI suffix, so `new_unchecked` is safe.
fn prov(local: &str) -> NamedNode {
    NamedNode::new_unchecked(format!("{}{}", PROV_NS, local))
}

/// `trust:` term IRI for a vocab constant (already a full IRI — wrap it).
fn trust_node(full_iri: &str) -> NamedNode {
    NamedNode::new_unchecked(full_iri.to_owned())
}

/// Whether a delegation principal is a human or an AI agent — an **attested attribute**
/// on the delegate (design §4.2: *"Human vs AI is just an attested attribute / caveat on
/// the delegate"*). It confers NO authority; the AI agent's authority is the attenuated
/// capability the signed chain carries. The audit records it as a `trust:HumanPrincipal`
/// / `trust:AiAgent` type so an auditor can tell a human acting from an AI agent acting on
/// a human's behalf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrincipalKind {
    /// A human principal (the human root, or any human delegate).
    Human,
    /// An AI agent: a distinct principal (its own DID/key) holding an attenuated child of
    /// its human principal's authority (§4.2).
    AiAgent,
}

impl PrincipalKind {
    /// The `trust:` class IRI this kind classifies a principal as.
    fn type_iri(self) -> NamedNode {
        match self {
            PrincipalKind::Human => trust_node(crate::vocab::HUMAN_PRINCIPAL),
            PrincipalKind::AiAgent => trust_node(crate::vocab::AI_AGENT),
        }
    }
}

/// How the principals of a chain are classified (human vs AI agent) for the audit.
///
/// The classification is an **attested attribute** supplied by the caller (it is NOT
/// derivable from the chain's cryptography — a key does not say whether its holder is a
/// human or an AI agent). A principal NOT named here is recorded as a plain `prov:Agent`
/// with no human/AI type (honest: an unclassified principal is not silently assumed human).
#[derive(Debug, Clone, Default)]
pub struct PrincipalClassification {
    /// `(principal-WebID/DID, kind)` pairs. The canonical use is the §4.2 shape: the human
    /// root is [`PrincipalKind::Human`] and the terminal AI agent is
    /// [`PrincipalKind::AiAgent`], but any mix is allowed (a human may delegate to a human).
    classes: Vec<(NamedNode, PrincipalKind)>,
}

impl PrincipalClassification {
    /// An empty classification (every principal recorded as a plain `prov:Agent`).
    pub fn new() -> Self {
        Self::default()
    }

    /// Classify `principal` as `kind`. The last classification for a principal wins.
    pub fn with(mut self, principal: NamedNode, kind: PrincipalKind) -> Self {
        self.classes.push((principal, kind));
        self
    }

    /// The classification recorded for `principal`, if any (`None` ⇒ plain `prov:Agent`).
    fn kind_of(&self, principal: &NamedNode) -> Option<PrincipalKind> {
        self.classes
            .iter()
            .rev()
            .find(|(p, _)| p.as_str() == principal.as_str())
            .map(|(_, k)| *k)
    }
}

/// A completed PROV-O delegation audit: the audit RDF graph for one invocation-bound
/// chain + effective capability. Produced by [`audit_invocation`]; obtain the triples
/// with [`DelegationAudit::graph`].
#[derive(Debug, Clone)]
pub struct DelegationAudit {
    triples: Vec<Triple>,
    activity: NamedNode,
    grant_entity: NamedNode,
}

impl DelegationAudit {
    /// The PROV-O audit graph as RDF triples (standard `prov:` IRIs — round-trips through
    /// any PROV-O consumer). The graph is self-contained: every node IRI is absolute.
    pub fn graph(&self) -> &[Triple] {
        &self.triples
    }

    /// The minted `prov:Activity` IRI naming the invocation.
    pub fn activity(&self) -> &NamedNode {
        &self.activity
    }

    /// The minted `prov:Entity` IRI naming the effective grant.
    pub fn grant_entity(&self) -> &NamedNode {
        &self.grant_entity
    }
}

/// IRI prefix the audit mints its `prov:Activity` / `prov:Entity` nodes under when not
/// supplied — stable + self-describing-shaped, distinct from the `urn:sparq:auth:`
/// allow-list and the `urn:sparq:prov:` derivation namespace.
pub const SPARQ_DELEGATION_PROV_NS: &str = "urn:sparq:delegation-prov:";

/// Optional knobs for the audit (IRIs to use, the wall-clock instant). Defaults mint fresh
/// `urn:sparq:delegation-prov:` nodes and omit the timestamp.
#[derive(Debug, Clone, Default)]
pub struct AuditConfig {
    /// IRI naming the `prov:Activity` (the invocation). `None` ⇒ mint a fresh one keyed on
    /// the terminal delegate (so two invocations by the same delegate share an activity IRI
    /// — deterministic, replayable). Supply one to integrate with an external audit store.
    pub activity: Option<NamedNode>,
    /// IRI naming the `prov:Entity` (the effective grant). `None` ⇒ mint a fresh one keyed
    /// on the delegate + target.
    pub entity: Option<NamedNode>,
    /// Wall-clock instant of the invocation as a Unix timestamp (seconds). `Some` ⇒ recorded
    /// as `prov:atTime "…"^^xsd:dateTime` on the activity; `None` ⇒ omitted (honest: we do
    /// not fabricate a timestamp).
    pub at_time_unix_secs: Option<i64>,
    /// How the chain's principals are classified human/AI (§4.2). Default: all unclassified
    /// (plain `prov:Agent`).
    pub principals: PrincipalClassification,
}

/// Render a PROV-O delegation audit for an invocation-bound chain + effective capability.
///
/// `chain` is the carried delegation chain that passed [`crate::delegation::invoke`];
/// `effective` is the surviving capability (the terminal capability, optionally already
/// intersected with the current delegator grant via
/// [`crate::delegation::effective_against_current`]). The audit records the on-behalf-of
/// lineage of the **whole** chain plus the effective grant the terminal delegate exercised.
///
/// This is an **audit record only** — it adds no security property and is never an
/// authority source (see the module docs). It is the caller's responsibility to call this
/// only on a chain that actually passed the gate; the function does not re-verify the chain
/// (that is [`crate::delegation::invoke`]'s job, and re-doing it here would invite the
/// caller to treat the audit as the gate). It NEVER panics.
pub fn audit_invocation(
    chain: &DelegationChain,
    effective: &EffectiveCapability,
    config: &AuditConfig,
) -> DelegationAudit {
    let mut out: Vec<Triple> = Vec::new();

    let terminal_delegate = effective.delegate.clone();
    let target = effective.capability.target.clone();

    // Mint (or take) the activity + grant-entity IRIs deterministically.
    let activity = config.activity.clone().unwrap_or_else(|| {
        NamedNode::new_unchecked(format!(
            "{}invocation:{}",
            SPARQ_DELEGATION_PROV_NS,
            slug(terminal_delegate.as_str())
        ))
    });
    let grant_entity = config.entity.clone().unwrap_or_else(|| {
        NamedNode::new_unchecked(format!(
            "{}grant:{}:{}",
            SPARQ_DELEGATION_PROV_NS,
            slug(terminal_delegate.as_str()),
            slug(target.as_str())
        ))
    });

    // 1. Every principal in the chain is a prov:Agent, optionally typed human/AI.
    //    Collect principals in chain order (delegators then the terminal delegate), de-duped.
    let mut principals: Vec<NamedNode> = Vec::new();
    for hop in &chain.hops {
        for p in [&hop.delegator, &hop.delegate] {
            if !principals.iter().any(|q| q.as_str() == p.as_str()) {
                principals.push(p.clone());
            }
        }
    }
    for p in &principals {
        out.push(Triple {
            subject: NamedOrBlankNode::NamedNode(p.clone()),
            predicate: rdf::TYPE.into_owned(),
            object: Term::NamedNode(prov("Agent")),
        });
        if let Some(kind) = config.principals.kind_of(p) {
            out.push(Triple {
                subject: NamedOrBlankNode::NamedNode(p.clone()),
                predicate: rdf::TYPE.into_owned(),
                object: Term::NamedNode(kind.type_iri()),
            });
        }
    }

    // 2. The on-behalf-of lineage: for each hop, delegate prov:actedOnBehalfOf delegator.
    //    This is the modelling sparq-prov lacks — the delegation chain as PROV-O.
    for hop in &chain.hops {
        out.push(Triple {
            subject: NamedOrBlankNode::NamedNode(hop.delegate.clone()),
            predicate: prov("actedOnBehalfOf"),
            object: Term::NamedNode(hop.delegator.clone()),
        });
    }

    // 3. The invocation activity: typed, associated with the terminal delegate, using the
    //    chain principals, optionally timestamped.
    out.push(Triple {
        subject: NamedOrBlankNode::NamedNode(activity.clone()),
        predicate: rdf::TYPE.into_owned(),
        object: Term::NamedNode(prov("Activity")),
    });
    out.push(Triple {
        subject: NamedOrBlankNode::NamedNode(activity.clone()),
        predicate: prov("wasAssociatedWith"),
        object: Term::NamedNode(terminal_delegate.clone()),
    });
    for p in &principals {
        out.push(Triple {
            subject: NamedOrBlankNode::NamedNode(activity.clone()),
            predicate: prov("used"),
            object: Term::NamedNode(p.clone()),
        });
    }
    if let Some(secs) = config.at_time_unix_secs {
        out.push(Triple {
            subject: NamedOrBlankNode::NamedNode(activity.clone()),
            predicate: prov("atTime"),
            object: Term::Literal(Literal::new_typed_literal(
                unix_to_xsd_datetime(secs),
                xsd::DATE_TIME.into_owned(),
            )),
        });
    }

    // 4. The effective grant entity: generated by + attributed to the terminal delegate,
    //    carrying the auth:* action triples (the grant rendered as RDF — §4.3 value-add).
    out.push(Triple {
        subject: NamedOrBlankNode::NamedNode(grant_entity.clone()),
        predicate: rdf::TYPE.into_owned(),
        object: Term::NamedNode(prov("Entity")),
    });
    out.push(Triple {
        subject: NamedOrBlankNode::NamedNode(grant_entity.clone()),
        predicate: prov("wasGeneratedBy"),
        object: Term::NamedNode(activity.clone()),
    });
    out.push(Triple {
        subject: NamedOrBlankNode::NamedNode(grant_entity.clone()),
        predicate: prov("wasAttributedTo"),
        object: Term::NamedNode(terminal_delegate.clone()),
    });
    for grant in render_capability_grants(&effective.capability, &terminal_delegate) {
        out.push(grant);
    }

    DelegationAudit {
        triples: out,
        activity,
        grant_entity,
    }
}

/// The `auth:*` view namespace (the SAME predicates the shipped WAC/ACP rules emit and
/// `sparq_solid::AuthIndex` reads — see [`crate::wire::AUTH_NS`]). Re-declared here so the
/// audit module does not depend on the N3-merge module's wiring.
const AUTH_NS: &str = "https://sparq.dev/ns/auth#";

/// Render the effective [`Capability`] as `auth:*` grant triples over its target, subject =
/// the terminal delegate (the principal the grant is *for*). This is the §4.3 value-add: the
/// surviving delegated authority expressed as RDF that reasons with the SAME `.acl`/`.acr`
/// rules the rest of the estate uses — `<delegate> auth:read <target>`, etc.
///
/// Only actions in the `auth:` namespace are rendered (the gate's capability lattice is the
/// `auth:read|write|append|control` action set the WAC/ACP view uses); a non-`auth:` action
/// is recorded too (it is still a granted action), so the audit is faithful to the chain.
fn render_capability_grants(cap: &Capability, delegate: &NamedNode) -> Vec<Triple> {
    cap.actions
        .iter()
        .map(|action| Triple {
            subject: NamedOrBlankNode::NamedNode(delegate.clone()),
            predicate: action.clone(),
            object: Term::NamedNode(cap.target.clone()),
        })
        .collect()
}

/// Whether `action` is an `auth:` view predicate (exposed so a consumer can filter the
/// audit's grant triples to the WAC/ACP action lattice if it wants only those).
pub fn is_auth_action(action: &NamedNode) -> bool {
    action.as_str().starts_with(AUTH_NS)
}

/// Slugify an IRI into a node-name-safe suffix (non-alphanumeric → `_`), so a minted
/// `prov:` IRI is a valid, stable, collision-resistant-enough node name for the audit.
fn slug(iri: &str) -> String {
    iri.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

use sparq_core::civil::format_utc as unix_to_xsd_datetime;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delegation::Capability;

    fn iri(s: &str) -> NamedNode {
        NamedNode::new(s).unwrap()
    }
    fn read() -> NamedNode {
        iri("https://sparq.dev/ns/auth#read")
    }
    fn write() -> NamedNode {
        iri("https://sparq.dev/ns/auth#write")
    }

    /// A human → AI-agent chain over /docs/ (human root grants read+write, the AI agent
    /// child attenuates to read over /docs/x). Mirrors the §4.2 on-behalf-of shape.
    fn human_to_ai_chain() -> (DelegationChain, EffectiveCapability) {
        use crate::delegation::DelegationHop;
        use sparq_zk::sig::SecretKey;
        let human = iri("https://alice.ex/card#me");
        let ai = iri("https://alice-bot.ex/card#me");
        let (hk, hpk) = {
            let sk = SecretKey::from_seed(1);
            let pk = sk.public_key();
            (sk, pk)
        };
        let (ak, apk) = {
            let sk = SecretKey::from_seed(2);
            let pk = sk.public_key();
            (sk, pk)
        };
        let _ = (&hk, &ak); // keys not re-verified here — this is the audit, not the gate
        let hop = DelegationHop {
            delegator: human.clone(),
            delegator_key: hpk,
            delegate: ai.clone(),
            delegate_key: apk,
            capability: Capability {
                actions: vec![read()],
                target: iri("https://pod.ex/docs/x"),
            },
            expires_at_unix_secs: 2_000_000_000,
            signature_hex: String::new(),
        };
        let chain = DelegationChain { hops: vec![hop] };
        let effective = EffectiveCapability {
            capability: Capability {
                actions: vec![read()],
                target: iri("https://pod.ex/docs/x"),
            },
            delegate: ai,
        };
        (chain, effective)
    }

    #[test]
    fn audit_records_acted_on_behalf_of_and_principal_kinds() {
        let (chain, eff) = human_to_ai_chain();
        let human = iri("https://alice.ex/card#me");
        let ai = iri("https://alice-bot.ex/card#me");
        let cfg = AuditConfig {
            principals: PrincipalClassification::new()
                .with(human.clone(), PrincipalKind::Human)
                .with(ai.clone(), PrincipalKind::AiAgent),
            ..Default::default()
        };
        let audit = audit_invocation(&chain, &eff, &cfg);
        let g = audit.graph();

        // The on-behalf-of edge: the AI agent acted on behalf of the human.
        let acted = Triple {
            subject: NamedOrBlankNode::NamedNode(ai.clone()),
            predicate: prov("actedOnBehalfOf"),
            object: Term::NamedNode(human.clone()),
        };
        assert!(
            g.contains(&acted),
            "AI agent prov:actedOnBehalfOf the human delegator"
        );

        // The principal-kind attestations.
        let ai_is_agent = Triple {
            subject: NamedOrBlankNode::NamedNode(ai.clone()),
            predicate: rdf::TYPE.into_owned(),
            object: Term::NamedNode(trust_node(crate::vocab::AI_AGENT)),
        };
        let human_is_human = Triple {
            subject: NamedOrBlankNode::NamedNode(human.clone()),
            predicate: rdf::TYPE.into_owned(),
            object: Term::NamedNode(trust_node(crate::vocab::HUMAN_PRINCIPAL)),
        };
        assert!(g.contains(&ai_is_agent), "AI delegate typed trust:AiAgent");
        assert!(
            g.contains(&human_is_human),
            "human root typed trust:HumanPrincipal"
        );

        // Both principals are prov:Agent.
        for p in [&human, &ai] {
            let is_agent = Triple {
                subject: NamedOrBlankNode::NamedNode(p.clone()),
                predicate: rdf::TYPE.into_owned(),
                object: Term::NamedNode(prov("Agent")),
            };
            assert!(g.contains(&is_agent), "principal is a prov:Agent");
        }
    }

    #[test]
    fn audit_renders_the_effective_grant_as_auth_rdf() {
        let (chain, eff) = human_to_ai_chain();
        let ai = iri("https://alice-bot.ex/card#me");
        let audit = audit_invocation(&chain, &eff, &AuditConfig::default());
        let g = audit.graph();

        // The grant is rendered as <ai-agent> auth:read <target>.
        let grant = Triple {
            subject: NamedOrBlankNode::NamedNode(ai.clone()),
            predicate: read(),
            object: Term::NamedNode(iri("https://pod.ex/docs/x")),
        };
        assert!(
            g.contains(&grant),
            "effective grant rendered as auth:* RDF for the delegate"
        );

        // The grant entity is generated by + attributed to the terminal delegate.
        let gen = Triple {
            subject: NamedOrBlankNode::NamedNode(audit.grant_entity().clone()),
            predicate: prov("wasGeneratedBy"),
            object: Term::NamedNode(audit.activity().clone()),
        };
        let attr = Triple {
            subject: NamedOrBlankNode::NamedNode(audit.grant_entity().clone()),
            predicate: prov("wasAttributedTo"),
            object: Term::NamedNode(ai),
        };
        assert!(g.contains(&gen), "grant wasGeneratedBy the invocation");
        assert!(g.contains(&attr), "grant wasAttributedTo the delegate");
    }

    #[test]
    fn audit_records_only_an_auth_action_that_the_capability_grants() {
        // The capability grants read only — write must NOT appear in the rendered grant.
        let (chain, eff) = human_to_ai_chain();
        let ai = iri("https://alice-bot.ex/card#me");
        let audit = audit_invocation(&chain, &eff, &AuditConfig::default());
        let g = audit.graph();
        let write_grant = Triple {
            subject: NamedOrBlankNode::NamedNode(ai),
            predicate: write(),
            object: Term::NamedNode(iri("https://pod.ex/docs/x")),
        };
        assert!(
            !g.contains(&write_grant),
            "an action the capability does NOT grant is never rendered"
        );
    }

    #[test]
    fn timestamp_recorded_only_when_supplied() {
        let (chain, eff) = human_to_ai_chain();
        // Without a timestamp: no prov:atTime.
        let audit = audit_invocation(&chain, &eff, &AuditConfig::default());
        assert!(
            !audit
                .graph()
                .iter()
                .any(|t| t.predicate.as_str() == format!("{}atTime", PROV_NS)),
            "no fabricated timestamp when none is supplied"
        );
        // With a timestamp: a prov:atTime xsd:dateTime literal.
        let cfg = AuditConfig {
            at_time_unix_secs: Some(1_700_000_000),
            ..Default::default()
        };
        let audit = audit_invocation(&chain, &eff, &cfg);
        let at = audit
            .graph()
            .iter()
            .find(|t| t.predicate.as_str() == format!("{}atTime", PROV_NS))
            .expect("prov:atTime present when supplied");
        match &at.object {
            Term::Literal(l) => {
                assert_eq!(l.datatype().as_str(), xsd::DATE_TIME.as_str());
                assert_eq!(l.value(), "2023-11-14T22:13:20Z", "correct xsd:dateTime");
            }
            _ => panic!("prov:atTime must be a typed literal"),
        }
    }

    #[test]
    fn unclassified_principal_is_plain_agent_not_assumed_human() {
        // No classification supplied → no human/AI type, only prov:Agent (honest).
        let (chain, eff) = human_to_ai_chain();
        let ai = iri("https://alice-bot.ex/card#me");
        let audit = audit_invocation(&chain, &eff, &AuditConfig::default());
        let g = audit.graph();
        let typed_human = Triple {
            subject: NamedOrBlankNode::NamedNode(ai),
            predicate: rdf::TYPE.into_owned(),
            object: Term::NamedNode(trust_node(crate::vocab::HUMAN_PRINCIPAL)),
        };
        assert!(
            !g.contains(&typed_human),
            "an unclassified principal is NOT silently assumed human"
        );
    }

    #[test]
    fn datetime_conversion_handles_epoch_and_pre_epoch() {
        assert_eq!(unix_to_xsd_datetime(0), "1970-01-01T00:00:00Z");
        assert_eq!(unix_to_xsd_datetime(-1), "1969-12-31T23:59:59Z");
        assert_eq!(unix_to_xsd_datetime(1_700_000_000), "2023-11-14T22:13:20Z");
    }

    #[test]
    fn is_auth_action_classifies_the_lattice() {
        assert!(is_auth_action(&read()));
        assert!(!is_auth_action(&iri("https://example.org/other#act")));
    }
}
