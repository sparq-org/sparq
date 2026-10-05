//! # `policy.rs` — parse a trust-policy graph into `Vec<TrustRule>` (fail-closed)
//!
//! Parses a trust-policy RDF graph (authored in the **Control-gated channel**, the
//! same trusted channel as `.acl`/`.acr` — design §3.2) into a `Vec<TrustRule>`.
//! Each rule carries one admission condition: `source` + `issuer_key` + `shape` +
//! `scope` + `fresh_within`.
//!
//! ## Two equivalent authoring forms (both → the SAME `Vec<TrustRule>`)
//!
//! - **Reified** (`trust:TrustRule`) — a rule node groups `trust:source` +
//!   `trust:forShape`/`trust:forPredicate` + `trust:scope` + `trust:freshWithin` (+ key).
//!   This keeps `trust:scope`/`trust:freshWithin` **per-rule**, so one source may be
//!   trusted with different windows on different resources.
//! - **Claim-level relational** (`trust:trustsSourceFor`) — the foundational primitive
//!   (design §2.3.1): a `trust:Source` node carries
//!   `trust:trustsSourceFor <shape-or-predicate>` directly, alongside its key +
//!   `trust:scope` + `trust:freshWithin`. EACH `trustsSourceFor` statement is one rule;
//!   the source's key/scope/freshness are **shared** across its statement-types. This is
//!   the compact form that maps onto — and replaces — ACP's type-only, unimplemented
//!   `acp:vc` matcher with *per-(source, statement-type)* trust (the `sq-pfae.4` gap).
//!   The `trustsSourceFor` object is a `sh:NodeShape` node (carried like `forShape`) or a
//!   predicate IRI (desugared like `forPredicate`).
//!
//! Both forms produce the identical `TrustRule` the admission gate consumes, so the gate,
//! the DID-resolved key binding, and every soundness side-condition are unchanged: only
//! the surface syntax differs. A graph may mix both.
//!
//! ## Fail-closed Control-gating (design §3.2 / §3.3 item 3)
//!
//! The trust graph re-opens the content/reasoner boundary `sparq-solid` deliberately
//! closed, so a policy an UNTRUSTED writer could author must never take effect. The
//! parser therefore requires the caller to present a [`ControlGate`] — the token a
//! relying layer mints ONLY after it has verified the policy graph arrived through
//! the `acl:Control` / ACR-write channel (whoever may write `.acl` may write the
//! trust rules; nothing else may). [`parse_policy`] without the gate is a
//! compile-time impossibility; [`try_parse_ungated`] exists for tests and returns an
//! explicit [`PolicyError::NotControlGated`] so the fail-closed default is visible.
//!
//! [OPUS-4.8] sq-pfae PoC (issue #940). 🤖 SPARQ agent — trust-graph authorisation PoC.

use crate::vocab;
use oxrdf::{NamedNode, Term, Triple};
use sparq_zk::sig::{public_key_from_hex, PublicKey};

/// A capability token asserting that the accompanying policy graph arrived through
/// the **Control-gated channel** (`acl:Control` / ACR-write). The relying layer mints
/// one ONLY after it has established the policy was written by a principal holding
/// Control over the resource — exactly the NGAC self-administration discipline (the
/// policy is governed by the same access decision it configures, §3.2).
///
/// It is deliberately unforgeable from outside this crate (no public constructor that
/// takes untrusted data): a policy graph alone can never gate itself. This makes the
/// fail-closed property a TYPE-level guarantee, not a runtime convention.
#[derive(Debug, Clone, Copy)]
pub struct ControlGate(());

impl ControlGate {
    /// Mint a Control-gate. The caller asserts, by calling this, that it has verified
    /// the policy graph was authored behind `acl:Control` (WAC) / ACR-write (ACP) —
    /// the same trusted channel as `.acl`/`.acr`. This is the trust ROOT of the whole
    /// admission pipeline; calling it on an un-vetted graph is the one mistake that
    /// re-opens the §2.4 boundary, so it is named loudly rather than implicit.
    ///
    /// In the shipped `sparq-solid`, "behind Control" is decided by the WAC/ACP
    /// materialiser; a production wiring would call this only after that decision.
    pub fn assert_control_gated() -> ControlGate {
        ControlGate(())
    }
}

/// One reified admission rule (§2.3.1): admit statements matching `shape` from
/// `source` (verified against `issuer_key`), within `scope`, no staler than
/// `fresh_within`.
#[derive(Debug, Clone)]
pub struct TrustRule {
    /// `trust:source` — the named attesting authority IRI.
    pub source: NamedNode,
    /// The verification key the source signs with. Bound either from operator-asserted
    /// `trust:issuerKey` hex (the default [`parse_policy`] path — the live forgery vector
    /// D′ of §3.3) OR from a `trust:issuerDid` resolved by a `did::DidResolver` via
    /// `resolve_rule_keys` (the opt-in `did` feature, `sq-pfae.3` — narrows D′; plain
    /// code-spans, not intra-doc links, so the default-feature doc build does not reference
    /// feature-gated items). The admission gate consumes this concrete key identically
    /// regardless of how it was bound.
    pub issuer_key: PublicKey,
    /// `trust:forShape` — the statement-type SHACL node-shape (a `forPredicate P`
    /// rule is desugared into this at load; see [`crate::vocab::desugar_for_predicate`]).
    /// The shape's defining triples, with `shape_root` naming the `sh:NodeShape`.
    pub shape: ShapeRef,
    /// `trust:scope` — the resource/container this rule applies to (containment check
    /// at admission). v1 is exact-or-ancestor IRI matching.
    pub scope: NamedNode,
    /// `trust:freshWithin` — maximum admitted staleness, in seconds. Parsed from the
    /// `trust:freshWithin` literal, which must be an EXACT day/time duration
    /// (`PnDTnHnMnS`); the nominal `P1Y`/`P6M` forms are rejected fail-closed rather
    /// than approximated, so this second count is exactly what the author wrote.
    /// Consulted against `Session.now` as a per-request Rust side-condition, NOT an
    /// in-reasoner predicate (§3.3 B′).
    pub fresh_within_secs: i64,
}

/// A `trust:forShape` SHACL node-shape: the node naming the shape plus the triples
/// that define it.
#[derive(Debug, Clone)]
pub struct ShapeRef {
    /// The `sh:NodeShape` node (an IRI for an external shape, or a blank node for an
    /// inline / desugared shape).
    pub root: Term,
    /// The triples defining the shape (parsed `forShape` content, or the desugaring
    /// of a `forPredicate`).
    pub triples: Vec<Triple>,
}

/// Why a trust policy was rejected (fail-closed: a rejected policy admits NOTHING).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyError {
    /// The policy was not presented through the Control-gated channel (§3.2).
    NotControlGated,
    /// A `trust:TrustRule` was missing a mandatory property (source / issuerKey /
    /// shape-or-predicate / scope / freshWithin). Names the offending rule + field.
    IncompleteRule { rule: String, missing: &'static str },
    /// The `trust:issuerKey` literal/IRI did not parse as a verification key.
    BadIssuerKey(String),
    /// The `trust:issuerDid` (or its resolved key) failed to resolve (`sq-pfae.3`). Carries
    /// the DID and the resolver's reason. Fail-closed: a rule whose DID does not resolve
    /// admits nothing.
    BadIssuerDid { did: String, reason: String },
    /// The `trust:freshWithin` literal was not an exactly-representable day/time
    /// `xsd:duration` (`PnDTnHnMnS`). This includes the lexically-valid-but-nominal
    /// `P1Y`/`P6M`, which have no exact seconds value — see `parse_xsd_duration_secs`.
    BadDuration(String),
}

impl std::fmt::Display for PolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PolicyError::NotControlGated => write!(
                f,
                "trust policy is not Control-gated (§3.2): rejected fail-closed"
            ),
            PolicyError::IncompleteRule { rule, missing } => {
                write!(f, "trust rule <{}> is missing {}", rule, missing)
            }
            PolicyError::BadIssuerKey(k) => write!(f, "trust:issuerKey did not parse: {}", k),
            PolicyError::BadIssuerDid { did, reason } => {
                write!(f, "trust:issuerDid <{}> did not resolve: {}", did, reason)
            }
            PolicyError::BadDuration(d) => write!(
                f,
                "trust:freshWithin is not an exact day/time xsd:duration \
                 (PnDTnHnMnS; the nominal Y/M designators have no exact seconds value): {}",
                d
            ),
        }
    }
}

impl std::error::Error for PolicyError {}

/// Parse a Control-gated trust-policy graph into `Vec<TrustRule>`. Possession of a
/// [`ControlGate`] is required: a policy that did not arrive through the trusted
/// channel cannot reach this function (§3.2, fail-closed).
///
/// Accepts BOTH authoring forms (see the module docs): each `trust:TrustRule` node must
/// carry `trust:source`, `trust:issuerKey`, exactly one of `trust:forShape` /
/// `trust:forPredicate` (the latter desugared here), `trust:scope`, and
/// `trust:freshWithin`; and each `trust:Source` node bearing a
/// `trust:trustsSourceFor <shape-or-predicate>` statement must carry `trust:issuerKey`,
/// `trust:scope`, and `trust:freshWithin` (one rule per `trustsSourceFor` statement). A
/// rule missing any required property is a hard [`PolicyError::IncompleteRule`] — a
/// partially-specified rule never silently admits.
pub fn parse_policy(policy: &[Triple], _gate: ControlGate) -> Result<Vec<TrustRule>, PolicyError> {
    parse_rules(policy, &operator_asserted_key)
}

/// The fail-closed analogue used in tests: parse WITHOUT a [`ControlGate`]. Always
/// returns [`PolicyError::NotControlGated`] — it exists so the default-deny posture
/// is exercised by a test, never to provide an un-gated path in production.
pub fn try_parse_ungated(_policy: &[Triple]) -> Result<Vec<TrustRule>, PolicyError> {
    Err(PolicyError::NotControlGated)
}

/// Resolve a rule node's issuer key. A strategy fn so the operator-asserted-hex path
/// ([`parse_policy`]) and the DID-resolving path ([`resolve_rule_keys`]) share ALL other
/// per-rule parsing (scope / freshness / shape) — only the key binding differs.
type KeyResolver<'a> = dyn Fn(&Term, &[Triple]) -> Result<PublicKey, PolicyError> + 'a;

/// The operator-asserted key binding (today's behaviour): the rule's `trust:issuerKey`
/// hex is parsed directly. This is the live forgery vector D′ — the operator pastes the
/// key — kept as the default so the lean build needs no DID resolver.
fn operator_asserted_key(node: &Term, policy: &[Triple]) -> Result<PublicKey, PolicyError> {
    let key_term =
        object_of(node, vocab::ISSUER_KEY, policy).ok_or_else(|| PolicyError::IncompleteRule {
            rule: node_label(node),
            missing: "trust:issuerKey",
        })?;
    let key_hex = term_lexical(&key_term);
    public_key_from_hex(&key_hex).ok_or_else(|| PolicyError::BadIssuerKey(key_hex.clone()))
}

/// How a trust rule binds its issuer's verifying key. Either the operator pastes the
/// key directly (`trust:issuerKey` hex — the live forgery vector D′), or the rule names
/// the issuer by `trust:issuerDid` and a resolver binds the key (`sq-pfae.3`). The DID
/// form **narrows** D′ — it does not eliminate it (a `did:key` is self-certifying, a
/// `did:web` is only as strong as host/TLS control; see [`crate::did`]).
#[cfg(feature = "did")]
#[cfg_attr(docsrs, doc(cfg(feature = "did")))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssuerBinding {
    /// `trust:issuerKey` — operator-asserted verifying-key hex (the default path).
    OperatorAssertedKey,
    /// `trust:issuerDid` — the DID string a resolver binds to a verifying key.
    Did(String),
}

/// Parse a Control-gated trust policy whose rules may name their issuer by
/// **`trust:issuerDid`** (resolved via `resolver`) as an alternative to the
/// operator-asserted `trust:issuerKey` hex, producing the same `Vec<TrustRule>` the
/// admission gate consumes (`sq-pfae.3`).
///
/// For each `trust:TrustRule`: if it carries `trust:issuerDid`, the DID is resolved to a
/// verifying key via `resolver` (`did:key` offline / `did:web` via the resolver's
/// fetcher); otherwise it falls back to `trust:issuerKey` hex. A rule carrying **neither**
/// is a [`PolicyError::IncompleteRule`]; a DID that fails to resolve is a
/// [`PolicyError::BadIssuerDid`] — **fail-closed**, so an unresolvable issuer admits
/// nothing. Everything else (scope, freshness, shape) parses exactly as [`parse_policy`].
///
/// Possession of a [`ControlGate`] is still required (§3.2): the issuer-key BINDING is
/// strengthened, but the policy itself must still arrive through the trusted channel —
/// DID resolution narrows forgery vector D′, it does not relax the Control-gating root.
#[cfg(feature = "did")]
#[cfg_attr(docsrs, doc(cfg(feature = "did")))]
pub fn resolve_rule_keys(
    policy: &[Triple],
    resolver: &dyn crate::did::DidResolver,
    _gate: ControlGate,
) -> Result<Vec<TrustRule>, PolicyError> {
    let key_for = move |node: &Term, policy: &[Triple]| -> Result<PublicKey, PolicyError> {
        // Prefer trust:issuerDid (the resolver-bound, D′-narrowing path).
        if let Some(did_term) = object_of(node, vocab::ISSUER_DID, policy) {
            let did_str = term_lexical(&did_term);
            return resolver.resolve_str(&did_str).map_err(|e| PolicyError::BadIssuerDid {
                did: did_str,
                reason: e.to_string(),
            });
        }
        // Fall back to the operator-asserted hex key.
        if object_of(node, vocab::ISSUER_KEY, policy).is_some() {
            return operator_asserted_key(node, policy);
        }
        Err(PolicyError::IncompleteRule {
            rule: node_label(node),
            missing: "trust:issuerDid or trust:issuerKey",
        })
    };
    parse_rules(policy, &key_for)
}

fn parse_rules(policy: &[Triple], key_for: &KeyResolver) -> Result<Vec<TrustRule>, PolicyError> {
    let mut out: Vec<TrustRule> = Vec::new();

    // Form A — the **reified** rule: a node typed `trust:TrustRule` grouping
    // `source` + `forShape`/`forPredicate` + `scope` + `freshWithin` (+ key).
    for t in policy {
        if t.predicate.as_str() == vocab::RDF_TYPE {
            if let Term::NamedNode(o) = &t.object {
                if o.as_str() == vocab::TRUST_RULE {
                    out.push(parse_one_rule(&subject_term(&t.subject), policy, key_for)?);
                }
            }
        }
    }

    // Form B — the **claim-level relational** form (the foundational `trustsSourceFor`
    // primitive, design §2.3.1; the direct replacement for ACP's type-only `acp:vc`):
    // a `trust:Source` node carries `trust:trustsSourceFor <shape-or-predicate>`
    // alongside its key + `trust:scope` + `trust:freshWithin`. EACH `trustsSourceFor`
    // statement on that source node is ONE admission rule (a source trusted for two
    // statement-types yields two rules), all over the source's shared key/scope/freshness.
    // The source node is BOTH the `trust:source` (the named attesting authority) and the
    // qualifier-bearing subject — collapsing the rule reification into the source itself
    // (design §2.3.2, the "fold `trust:source` into `trustsSourceFor`" compaction made
    // concrete). A reified `trust:TrustRule` node is not a `trust:Source`, so the two
    // forms never double-count the same rule.
    for (source_node, shape_obj) in trusts_source_for_statements(policy) {
        out.push(parse_claim_rule(&source_node, &shape_obj, policy, key_for)?);
    }

    Ok(out)
}

/// Collect every `(source, shape-object)` pair asserted by a `trust:trustsSourceFor`
/// triple, in policy order — the claim-level statements (Form B). One rule is parsed per
/// pair, so a source trusted for several statement-types produces several rules.
fn trusts_source_for_statements(policy: &[Triple]) -> Vec<(Term, Term)> {
    policy
        .iter()
        .filter(|t| t.predicate.as_str() == vocab::TRUSTS_SOURCE_FOR)
        .map(|t| (subject_term(&t.subject), t.object.clone()))
        .collect()
}

/// Parse one claim-level statement (Form B): a `source trust:trustsSourceFor shape_obj`
/// pair, over the source node's shared key + `trust:scope` + `trust:freshWithin`. The
/// source node's own IRI is the rule's `trust:source` (a blank-node source is rejected —
/// a named attesting authority must have a stable identity to tag admitted facts with).
/// `shape_obj` is the statement-type: a `sh:NodeShape` node (carry its closure, like
/// `forShape`) OR a predicate IRI (desugar, like `forPredicate`). Fails closed exactly as
/// [`parse_one_rule`]: any missing qualifier is a [`PolicyError::IncompleteRule`].
fn parse_claim_rule(
    source_node: &Term,
    shape_obj: &Term,
    policy: &[Triple],
    key_for: &KeyResolver,
) -> Result<TrustRule, PolicyError> {
    let rule_id = || node_label(source_node);

    // The source node IS the named attesting authority — it must be an IRI.
    let Term::NamedNode(source) = source_node else {
        return Err(PolicyError::IncompleteRule {
            rule: rule_id(),
            missing: "a named trust:Source (trustsSourceFor on a blank node is rejected)",
        });
    };

    // The issuer key is read off the SAME source node (`trust:issuerKey` hex or
    // `trust:issuerDid`), shared across every shape this source is trusted for.
    let issuer_key = key_for(source_node, policy)?;

    // `trust:scope` + `trust:freshWithin` qualify the source's trust (per-source here;
    // the reified form keeps them per-rule). Both are mandatory — fail closed.
    let scope = object_iri(source_node, vocab::SCOPE, policy).ok_or_else(|| {
        PolicyError::IncompleteRule {
            rule: rule_id(),
            missing: "trust:scope",
        }
    })?;
    let fresh_term = object_of(source_node, vocab::FRESH_WITHIN, policy).ok_or_else(|| {
        PolicyError::IncompleteRule {
            rule: rule_id(),
            missing: "trust:freshWithin",
        }
    })?;
    let fresh_lex = term_lexical(&fresh_term);
    let fresh_within_secs =
        parse_xsd_duration_secs(&fresh_lex).ok_or(PolicyError::BadDuration(fresh_lex))?;

    let shape = shape_from_trusts_object(shape_obj, policy);

    Ok(TrustRule {
        source: source.clone(),
        issuer_key,
        shape,
        scope,
        fresh_within_secs,
    })
}

/// Resolve a `trust:trustsSourceFor` object into a [`ShapeRef`]: a predicate IRI is
/// desugared to the normative single-predicate node-shape (exactly the `forPredicate`
/// rewrite — design §2.3.3), while a node naming a `sh:NodeShape` carries its closure of
/// defining triples (exactly the `forShape` primitive). A bare IRI is treated as a
/// predicate UNLESS the policy types it `sh:NodeShape`, in which case its shape closure is
/// carried — so `trustsSourceFor <ageShape>` (with `<ageShape> a sh:NodeShape`) and
/// `trustsSourceFor schema:age` both work.
fn shape_from_trusts_object(obj: &Term, policy: &[Triple]) -> ShapeRef {
    // A node explicitly typed `sh:NodeShape` is a shape: carry its closure (forShape path).
    let is_node_shape = policy.iter().any(|t| {
        subject_term(&t.subject) == *obj
            && t.predicate.as_str() == vocab::RDF_TYPE
            && matches!(&t.object, Term::NamedNode(n) if n.as_str() == vocab::SH_NODE_SHAPE)
    });
    if is_node_shape {
        return ShapeRef {
            root: obj.clone(),
            triples: shape_closure(obj, policy),
        };
    }
    // Otherwise an IRI object is a predicate: desugar it (forPredicate path).
    if let Term::NamedNode(p) = obj {
        let (root, triples) = vocab::desugar_for_predicate(p);
        return ShapeRef {
            root: shape_root_to_term(root),
            triples,
        };
    }
    // A blank-node object that is not typed sh:NodeShape: carry its closure anyway (an
    // inline shape with no explicit type) so an inline `[ sh:targetSubjectsOf … ]` works.
    ShapeRef {
        root: obj.clone(),
        triples: shape_closure(obj, policy),
    }
}

fn parse_one_rule(
    node: &Term,
    policy: &[Triple],
    key_for: &KeyResolver,
) -> Result<TrustRule, PolicyError> {
    let rule_id = || node_label(node);

    // trust:source — a named attesting authority.
    let source =
        object_iri(node, vocab::SOURCE, policy).ok_or_else(|| PolicyError::IncompleteRule {
            rule: rule_id(),
            missing: "trust:source",
        })?;

    // The issuer verifying key — operator-asserted hex (`trust:issuerKey`) or
    // DID-resolved (`trust:issuerDid`), depending on the supplied strategy.
    let issuer_key = key_for(node, policy)?;

    // trust:scope — exact-or-ancestor resource/container IRI.
    let scope =
        object_iri(node, vocab::SCOPE, policy).ok_or_else(|| PolicyError::IncompleteRule {
            rule: rule_id(),
            missing: "trust:scope",
        })?;

    // trust:freshWithin — xsd:duration → seconds.
    let fresh_term = object_of(node, vocab::FRESH_WITHIN, policy).ok_or_else(|| {
        PolicyError::IncompleteRule {
            rule: rule_id(),
            missing: "trust:freshWithin",
        }
    })?;
    let fresh_lex = term_lexical(&fresh_term);
    let fresh_within_secs =
        parse_xsd_duration_secs(&fresh_lex).ok_or(PolicyError::BadDuration(fresh_lex))?;

    // trust:forShape (primitive) OR trust:forPredicate (sugar → desugar here).
    let shape = parse_shape(node, policy).ok_or_else(|| PolicyError::IncompleteRule {
        rule: rule_id(),
        missing: "trust:forShape or trust:forPredicate",
    })?;

    Ok(TrustRule {
        source,
        issuer_key,
        shape,
        scope,
        fresh_within_secs,
    })
}

/// Resolve the rule's statement-type shape: a `trust:forShape` IRI/node (an external
/// shape carried in the policy) OR a `trust:forPredicate P` desugared into the
/// normative single-predicate node-shape.
fn parse_shape(node: &Term, policy: &[Triple]) -> Option<ShapeRef> {
    // Sugar first: forPredicate P → forShape (so a policy may use either).
    if let Some(Term::NamedNode(p)) = object_of(node, vocab::FOR_PREDICATE, policy) {
        let (root, triples) = vocab::desugar_for_predicate(&p);
        return Some(ShapeRef {
            root: shape_root_to_term(root),
            triples,
        });
    }
    // Primitive: forShape names a sh:NodeShape; carry its closure of defining triples.
    if let Some(shape_node) = object_of(node, vocab::FOR_SHAPE, policy) {
        let triples = shape_closure(&shape_node, policy);
        return Some(ShapeRef {
            root: shape_node,
            triples,
        });
    }
    None
}

/// The transitive closure of triples reachable from a shape node within the policy
/// graph (so an inline `sh:property [ … ]` chain is carried whole). Bounded, since
/// the policy graph is finite and visited-tracked.
fn shape_closure(root: &Term, policy: &[Triple]) -> Vec<Triple> {
    let mut frontier = vec![root.clone()];
    let mut seen: Vec<Term> = Vec::new();
    let mut out: Vec<Triple> = Vec::new();
    while let Some(s) = frontier.pop() {
        if seen.iter().any(|x| x == &s) {
            continue;
        }
        seen.push(s.clone());
        for t in policy {
            if subject_term(&t.subject) == s {
                out.push(t.clone());
                // Follow object nodes (blank or IRI) to gather nested property shapes.
                match &t.object {
                    Term::BlankNode(_) | Term::NamedNode(_) => frontier.push(t.object.clone()),
                    _ => {}
                }
            }
        }
    }
    out
}

// --- small RDF helpers (no extra deps; the policy graph is small) ----------

fn subject_term(s: &oxrdf::NamedOrBlankNode) -> Term {
    match s {
        oxrdf::NamedOrBlankNode::NamedNode(n) => Term::NamedNode(n.clone()),
        oxrdf::NamedOrBlankNode::BlankNode(b) => Term::BlankNode(b.clone()),
    }
}

fn shape_root_to_term(s: oxrdf::NamedOrBlankNode) -> Term {
    match s {
        oxrdf::NamedOrBlankNode::NamedNode(n) => Term::NamedNode(n),
        oxrdf::NamedOrBlankNode::BlankNode(b) => Term::BlankNode(b),
    }
}

fn object_of(subject: &Term, predicate: &str, policy: &[Triple]) -> Option<Term> {
    policy
        .iter()
        .find(|t| subject_term(&t.subject) == *subject && t.predicate.as_str() == predicate)
        .map(|t| t.object.clone())
}

fn object_iri(subject: &Term, predicate: &str, policy: &[Triple]) -> Option<NamedNode> {
    match object_of(subject, predicate, policy)? {
        Term::NamedNode(n) => Some(n),
        _ => None,
    }
}

fn term_lexical(t: &Term) -> String {
    match t {
        Term::NamedNode(n) => n.as_str().to_string(),
        Term::Literal(l) => l.value().to_string(),
        Term::BlankNode(b) => b.as_str().to_string(),
        other => other.to_string(),
    }
}

fn node_label(t: &Term) -> String {
    match t {
        Term::NamedNode(n) => n.as_str().to_string(),
        Term::BlankNode(b) => format!("_:{}", b.as_str()),
        other => other.to_string(),
    }
}

/// The ordered day/time designators, each with its EXACT length in seconds. A field's
/// index in this table enforces both the XSD ordering (`nDTnHnMnS`) and at-most-once —
/// the scan only ever looks at or after the last-matched index, so a repeated or
/// out-of-order designator simply finds no entry and fails closed.
///
/// The nominal `Y`/`M` (year/month) designators are deliberately ABSENT: a year is 365
/// or 366 days and a month is 28–31, so neither has an exact seconds value without an
/// anchor instant to resolve the calendar against (the same partial-order hazard
/// `sparq-reason`'s datatype notes call out for `P1M` vs `P30D`). The non-XSD ISO `W`
/// (weeks) designator is absent too — it is not in the `xsd:duration` lexical space.
const DAY_TIME_FIELDS: [(bool, char, i64); 4] = [
    // (is-in-time-part, designator, seconds-per-unit)
    (false, 'D', 86_400),
    (true, 'H', 3_600),
    (true, 'M', 60),
    (true, 'S', 1),
];

/// Parse a `trust:freshWithin` literal into whole seconds, accepting the non-negative,
/// whole-second part of the day/time subset of `xsd:duration` (a restriction of the
/// `xsd:dayTimeDuration` lexical space): `PnDTnHnMnS`, with every component optional but
/// at least one present.
///
/// Every accepted duration has an exact seconds value, so the freshness window a policy
/// author writes is the window the admission gate enforces — no calendar approximation.
/// Everything else is rejected fail-closed (`None` → [`PolicyError::BadDuration`], and a
/// rejected policy admits nothing), specifically:
///
/// - **nominal `Y`/`M`** (`P1Y`, `P6M`) — no exact seconds value without an anchor
///   instant; a policy meaning "a year" must say `P365D` and mean it;
/// - **`W`** (`P1W`) — not in the `xsd:duration` lexical space at all;
/// - **negative durations** (`-P1D`) — lexically valid `xsd:duration`, but a negative
///   staleness ceiling is meaningless;
/// - **fractional seconds** (`PT0.5S`) — not representable in whole seconds;
/// - **malformed lexical forms** — out-of-order (`PT1M1H`) or repeated (`P1D1D`)
///   designators, a dangling `T` (`P1DT`), digits with no designator (`P30`), an empty
///   numeral (`PD`), and the empty duration `P`;
/// - **out-of-range values** — every multiply and add is checked, so an absurd
///   `P9223372036854775807D` is rejected rather than overflowing.
///
/// The literal's datatype IRI is not inspected — this validates the LEXICAL form, so a
/// day/time value tagged `^^xsd:duration` (as the vocabulary's examples are) is accepted.
fn parse_xsd_duration_secs(s: &str) -> Option<i64> {
    // A leading `P` is mandatory; requiring it also rejects the negative `-P…` sign.
    let rest = s.strip_prefix('P')?;
    let (date_part, time_part) = match rest.split_once('T') {
        Some((date, time)) => (date, Some(time)),
        None => (rest, None),
    };
    // A `T` separator MUST introduce at least one time component: `P1DT` is malformed.
    if time_part == Some("") {
        return None;
    }

    let mut total: i64 = 0;
    // The lowest table index still available — monotonically advancing, which is what
    // enforces designator ordering and uniqueness across BOTH parts.
    let mut next_field = 0usize;
    accumulate_duration_part(date_part, false, &mut total, &mut next_field)?;
    if let Some(time) = time_part {
        accumulate_duration_part(time, true, &mut total, &mut next_field)?;
    }
    // `next_field` is still 0 iff no component matched — the empty duration `P`.
    if next_field == 0 {
        return None;
    }
    Some(total)
}

/// Accumulate one side of the `T` separator into `total`, advancing `next_field` past
/// each designator it consumes. Returns `None` on anything the restricted grammar does
/// not accept, so the caller propagates the fail-closed rejection.
fn accumulate_duration_part(
    part: &str,
    in_time: bool,
    total: &mut i64,
    next_field: &mut usize,
) -> Option<()> {
    let mut num = String::new();
    for c in part.chars() {
        if c.is_ascii_digit() {
            num.push(c);
            continue;
        }
        // No entry at-or-after `next_field` ⇒ the designator is unsupported (`Y`/`M` in
        // date position, `W`), already consumed, or out of order. All fail closed.
        let (idx, field) = DAY_TIME_FIELDS
            .iter()
            .enumerate()
            .skip(*next_field)
            .find(|(_, (is_time, designator, _))| *is_time == in_time && *designator == c)?;
        // An empty numeral (`PD`) or a value beyond `i64` fails here.
        let v: i64 = num.parse().ok()?;
        num.clear();
        *total = total.checked_add(v.checked_mul(field.2)?)?;
        *next_field = idx + 1;
    }
    // Trailing digits with no designator (`P30`) are malformed.
    if num.is_empty() {
        Some(())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ungated_policy_is_rejected_fail_closed() {
        assert!(matches!(
            try_parse_ungated(&[]),
            Err(PolicyError::NotControlGated)
        ));
    }

    /// Every accepted duration is EXACT: the seconds value is the arithmetic sum of its
    /// components, with no calendar approximation anywhere.
    #[test]
    fn exact_day_time_durations_parse_to_their_arithmetic_seconds() {
        assert_eq!(parse_xsd_duration_secs("P30D"), Some(30 * 86_400));
        assert_eq!(parse_xsd_duration_secs("P7D"), Some(7 * 86_400));
        assert_eq!(parse_xsd_duration_secs("PT1H"), Some(3_600));
        assert_eq!(parse_xsd_duration_secs("PT90M"), Some(5_400));
        assert_eq!(parse_xsd_duration_secs("PT0S"), Some(0));
        assert_eq!(
            parse_xsd_duration_secs("P1DT2H3M4S"),
            Some(86_400 + 7_200 + 180 + 4)
        );
        // Interior components may be omitted, as long as the order is preserved.
        assert_eq!(parse_xsd_duration_secs("P1DT4S"), Some(86_400 + 4));
    }

    /// The precision fix (sq-pfae.11 item 1): the nominal designators no longer parse.
    /// A year is 365 OR 366 days and a month 28–31, so neither has an exact seconds
    /// value without an anchor instant — approximating them silently widened or
    /// narrowed the author's staleness ceiling. Rejecting is fail-closed: the policy
    /// is refused outright, never admitted with a guessed window.
    #[test]
    fn nominal_year_and_month_durations_are_rejected_not_approximated() {
        assert_eq!(parse_xsd_duration_secs("P1Y"), None);
        assert_eq!(parse_xsd_duration_secs("P6M"), None);
        assert_eq!(parse_xsd_duration_secs("P1Y6M"), None);
        assert_eq!(parse_xsd_duration_secs("P1Y2M3DT4H"), None);
        // `W` is ISO 8601 but is NOT in the xsd:duration lexical space.
        assert_eq!(parse_xsd_duration_secs("P1W"), None);
        // The time-position `M` is minutes and stays accepted — only the DATE-position
        // `M` (months) is the nominal one.
        assert_eq!(parse_xsd_duration_secs("PT6M"), Some(360));
    }

    /// A negative staleness ceiling is meaningless, so the lexically-valid `-P…` sign
    /// is refused rather than silently admitting a rule nothing can satisfy.
    #[test]
    fn negative_durations_are_rejected() {
        assert_eq!(parse_xsd_duration_secs("-P1D"), None);
        assert_eq!(parse_xsd_duration_secs("-PT1H"), None);
    }

    /// Lexical strictness: the restricted grammar is `PnDTnHnMnS`, components ordered
    /// and each at most once, with a `T` only where it introduces a time component.
    #[test]
    fn malformed_lexical_forms_are_rejected() {
        assert_eq!(parse_xsd_duration_secs("garbage"), None);
        assert_eq!(parse_xsd_duration_secs(""), None);
        assert_eq!(parse_xsd_duration_secs("P"), None, "empty duration");
        assert_eq!(parse_xsd_duration_secs("PT"), None, "dangling T, no time part");
        assert_eq!(parse_xsd_duration_secs("P1DT"), None, "dangling T after a date");
        assert_eq!(parse_xsd_duration_secs("P30"), None, "digits, no designator");
        assert_eq!(parse_xsd_duration_secs("PD"), None, "designator, no numeral");
        assert_eq!(parse_xsd_duration_secs("PT1M1H"), None, "out of order");
        assert_eq!(parse_xsd_duration_secs("P1D1D"), None, "repeated designator");
        assert_eq!(parse_xsd_duration_secs("PT1H1H"), None, "repeated designator");
        assert_eq!(parse_xsd_duration_secs("P1H"), None, "time unit in date part");
        assert_eq!(parse_xsd_duration_secs("PT1D"), None, "date unit in time part");
        assert_eq!(parse_xsd_duration_secs("PT0.5S"), None, "fractional seconds");
        assert_eq!(parse_xsd_duration_secs("1D"), None, "missing leading P");
    }

    /// Out-of-range components are rejected by checked arithmetic rather than
    /// overflowing (which previously panicked in a debug build).
    #[test]
    fn out_of_range_durations_are_rejected_not_overflowed() {
        assert_eq!(parse_xsd_duration_secs("P9223372036854775807D"), None);
        assert_eq!(parse_xsd_duration_secs("P99999999999999999999D"), None);
        assert_eq!(
            parse_xsd_duration_secs("P106751991167300DT15H30M8S"),
            None,
            "the sum overflows even though each component fits"
        );
    }

    // --- the claim-level `trust:trustsSourceFor` relational form (Form B, sq-pfae.4) ---

    use oxrdf::{Literal, NamedOrBlankNode};
    use sparq_zk::sig::{public_key_to_hex, SecretKey};

    const GOV: &str = "https://gov.example/issuer";
    const SCHEMA_AGE: &str = "http://schema.org/age";
    const SCHEMA_NAME: &str = "http://schema.org/name";
    const RES: &str = "https://pod.ex/resourceX";

    fn nn(s: &str) -> NamedNode {
        NamedNode::new(s).unwrap()
    }

    fn key_hex() -> String {
        public_key_to_hex(&SecretKey::from_seed(0xC0FFEE).public_key())
    }

    /// A `trust:Source` node carrying `n_shapes` `trustsSourceFor predicate` statements
    /// plus a shared `issuerKey` + `scope` + `freshWithin` — the claim-level form.
    fn source_trusts(source: &str, predicates: &[&str], with_key: bool) -> Vec<Triple> {
        let s = || NamedOrBlankNode::NamedNode(nn(source));
        let mut g = vec![Triple::new(
            s(),
            nn(vocab::RDF_TYPE),
            Term::NamedNode(nn(vocab::SOURCE_CLASS)),
        )];
        for p in predicates {
            g.push(Triple::new(
                s(),
                nn(vocab::TRUSTS_SOURCE_FOR),
                Term::NamedNode(nn(p)),
            ));
        }
        if with_key {
            g.push(Triple::new(
                s(),
                nn(vocab::ISSUER_KEY),
                Term::Literal(Literal::new_simple_literal(key_hex())),
            ));
        }
        g.push(Triple::new(s(), nn(vocab::SCOPE), Term::NamedNode(nn(RES))));
        g.push(Triple::new(
            s(),
            nn(vocab::FRESH_WITHIN),
            Term::Literal(Literal::new_typed_literal(
                "P30D",
                nn("http://www.w3.org/2001/XMLSchema#duration"),
            )),
        ));
        g
    }

    #[test]
    fn trusts_source_for_predicate_parses_to_one_rule() {
        let g = source_trusts(GOV, &[SCHEMA_AGE], true);
        let rules = parse_policy(&g, ControlGate::assert_control_gated()).unwrap();
        assert_eq!(rules.len(), 1, "one trustsSourceFor statement ⇒ one rule");
        let r = &rules[0];
        assert_eq!(
            r.source.as_str(),
            GOV,
            "the source node IS the named source"
        );
        assert_eq!(r.scope.as_str(), RES);
        assert_eq!(r.fresh_within_secs, 30 * 86_400);
        // The predicate desugars to the normative single-predicate shape (forPredicate path).
        assert!(
            r.shape
                .triples
                .iter()
                .any(|t| t.predicate.as_str() == vocab::SH_TARGET_SUBJECTS_OF
                    && matches!(&t.object, Term::NamedNode(o) if o.as_str() == SCHEMA_AGE)),
            "the trustsSourceFor predicate desugared to a sh:targetSubjectsOf shape"
        );
    }

    #[test]
    fn trusts_source_for_fans_out_one_rule_per_statement_type() {
        // A source trusted for TWO statement-types yields TWO rules, sharing the key/scope.
        let g = source_trusts(GOV, &[SCHEMA_AGE, SCHEMA_NAME], true);
        let rules = parse_policy(&g, ControlGate::assert_control_gated()).unwrap();
        assert_eq!(rules.len(), 2, "two trustsSourceFor statements ⇒ two rules");
        assert!(rules.iter().all(|r| r.source.as_str() == GOV));
        // Both desugared shapes are present (age and name), one per rule.
        let targets: Vec<String> = rules
            .iter()
            .flat_map(|r| r.shape.triples.iter())
            .filter(|t| t.predicate.as_str() == vocab::SH_TARGET_SUBJECTS_OF)
            .filter_map(|t| match &t.object {
                Term::NamedNode(o) => Some(o.as_str().to_string()),
                _ => None,
            })
            .collect();
        assert!(targets.contains(&SCHEMA_AGE.to_string()));
        assert!(targets.contains(&SCHEMA_NAME.to_string()));
    }

    #[test]
    fn trusts_source_for_node_shape_carries_its_closure() {
        // trustsSourceFor a node EXPLICITLY typed sh:NodeShape carries the shape closure
        // (the forShape path), not the desugaring.
        let shape = "https://pod.ex/.acr#ageShape";
        let s = || NamedOrBlankNode::NamedNode(nn(GOV));
        let mut g = source_trusts(GOV, &[], true);
        g.push(Triple::new(
            s(),
            nn(vocab::TRUSTS_SOURCE_FOR),
            Term::NamedNode(nn(shape)),
        ));
        // Define the shape: a sh:NodeShape targeting subjects-of schema:age.
        g.push(Triple::new(
            NamedOrBlankNode::NamedNode(nn(shape)),
            nn(vocab::RDF_TYPE),
            Term::NamedNode(nn(vocab::SH_NODE_SHAPE)),
        ));
        g.push(Triple::new(
            NamedOrBlankNode::NamedNode(nn(shape)),
            nn(vocab::SH_TARGET_SUBJECTS_OF),
            Term::NamedNode(nn(SCHEMA_AGE)),
        ));
        let rules = parse_policy(&g, ControlGate::assert_control_gated()).unwrap();
        assert_eq!(rules.len(), 1);
        assert!(
            matches!(&rules[0].shape.root, Term::NamedNode(n) if n.as_str() == shape),
            "the shape root is the named sh:NodeShape (forShape path), not a fresh blank node"
        );
        assert!(
            rules[0]
                .shape
                .triples
                .iter()
                .any(|t| t.predicate.as_str() == vocab::SH_TARGET_SUBJECTS_OF),
            "the node-shape closure is carried"
        );
    }

    #[test]
    fn trusts_source_for_missing_scope_is_incomplete_fail_closed() {
        // Drop the scope triple → fail-closed IncompleteRule (never a silent admit).
        let mut g = source_trusts(GOV, &[SCHEMA_AGE], true);
        g.retain(|t| t.predicate.as_str() != vocab::SCOPE);
        let err = parse_policy(&g, ControlGate::assert_control_gated());
        assert!(
            matches!(err, Err(PolicyError::IncompleteRule { missing, .. }) if missing.contains("scope")),
            "a trustsSourceFor source with no scope is rejected fail-closed: {err:?}"
        );
    }

    #[test]
    fn trusts_source_for_missing_key_is_incomplete_fail_closed() {
        let g = source_trusts(GOV, &[SCHEMA_AGE], false); // no issuerKey
        let err = parse_policy(&g, ControlGate::assert_control_gated());
        assert!(
            matches!(err, Err(PolicyError::IncompleteRule { .. })),
            "a trustsSourceFor source with no issuer key is rejected fail-closed: {err:?}"
        );
    }

    #[test]
    fn trusts_source_for_on_a_blank_node_is_rejected() {
        // A blank-node source has no stable identity to tag admitted facts with.
        let b = NamedOrBlankNode::BlankNode(oxrdf::BlankNode::new("src").unwrap());
        let g = vec![
            Triple::new(
                b.clone(),
                nn(vocab::TRUSTS_SOURCE_FOR),
                Term::NamedNode(nn(SCHEMA_AGE)),
            ),
            Triple::new(
                b.clone(),
                nn(vocab::ISSUER_KEY),
                Term::Literal(Literal::new_simple_literal(key_hex())),
            ),
            Triple::new(b.clone(), nn(vocab::SCOPE), Term::NamedNode(nn(RES))),
            Triple::new(
                b,
                nn(vocab::FRESH_WITHIN),
                Term::Literal(Literal::new_typed_literal(
                    "P30D",
                    nn("http://www.w3.org/2001/XMLSchema#duration"),
                )),
            ),
        ];
        let err = parse_policy(&g, ControlGate::assert_control_gated());
        assert!(
            matches!(err, Err(PolicyError::IncompleteRule { missing, .. }) if missing.contains("named")),
            "trustsSourceFor on a blank-node source is rejected fail-closed: {err:?}"
        );
    }
}
