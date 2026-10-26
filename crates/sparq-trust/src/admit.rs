//! # `admit.rs` — the admission gate (default-deny, short-circuit)
//!
//! For a presented credential graph `G`, the parsed `Vec<TrustRule>`, and the live
//! [`Session`], emit the `Vec<AdmittedFact>` that pass admission. The algorithm is
//! the design's §6.0 pseudocode, run exactly (short-circuit on first failure;
//! default-deny):
//!
//! ```text
//! admit(G, rules, session) -> admitted:
//!   cG := canonicalise(G)                              # sparq-canon RDFC-1.0
//!   for r in rules:
//!     if not scope_covers(r.scope, target):     continue
//!     if not verify_sig(cG.commitment, r.key):  continue   # CHECKED issuer sig
//!     if session.now - issued_at(G) > r.fresh:  continue   # per-request Rust check
//!     if revoked(G):                            continue   # input-stratified guard
//!     for t=(s,p,o) in G:
//!       if is_reserved(p):                      continue   # solidx:/urn:sparq: guard
//!       if not shape_admits(r.shape, t, G):     continue   # sparq-shacl
//!       if subject_of(t) != session.agent:      continue   # §3.4 holder binding
//!       emit AdmittedFact{ t, issuer: r.source, mark: trust:admitted }
//! ```
//!
//! `verify_sig`, `revoked`, freshness, and holder-binding are **Rust side-conditions**
//! (per-request, NOT in-reasoner — §3.3 B′); `shape_admits` runs the shipped,
//! terminating `sparq-shacl` validator; `scope_covers` is a containment check.
//!
//! ## The static / dynamic admission split (the `sq-xc4y` resolution)
//!
//! Admission has TWO classes of condition with DIFFERENT lifetimes (design §3.3
//! ADMISSION-VS-MATERIALIZE-ONCE GAP):
//!
//! - **STATIC** — the issuer signature over the RDFC-1.0 commitment, the
//!   statement-type scope (SHACL shape), the reserved-predicate guard, and the
//!   `trust:scope` containment. These depend ONLY on the credential + the policy, NOT
//!   on the request. They are session-independent and can be decided **once, at
//!   materialise-time**, exactly like the WAC/ACP derivation stratum.
//! - **DYNAMIC** — the holder binding (`credentialSubject == Session.agent`) and
//!   freshness (`Session.now - issued_at ≤ freshWithin`). These are **per-request**
//!   facts. They MUST be re-evaluated on every request and must NOT be frozen into the
//!   session-independent materialise-once view.
//!
//! [`admit`] runs BOTH classes against a single live [`Session`] (the snapshot path:
//! the decision is valid for *that* request only). [`admit_static`] runs ONLY the
//! static class and returns each fact carrying the dynamic conditions it still owes
//! ([`StaticAdmittedFact::holder`] and [`StaticAdmittedFact::not_after_unix_secs`]) so
//! the caller can install a **conditional grant** whose holder/freshness are re-checked
//! per request at query time — the shipped sq-0q7n `auth:notBefore`/`auth:notAfter`
//! conditional-grant precedent (`sparq-solid` `AuthIndex::cond_applies`). This is the
//! `sq-xc4y` decision **(a) split static-admission from dynamic-admission**, NOT
//! **(b) per-request re-materialise**: the static stratum composes with the epoch
//! cache, the dynamic conditions ride the existing per-request conditional-grant path.
//!
//! ## Open-problem hooks this gate RESPECTS (does not silently solve)
//!
//! - **`sq-xc4y`** (holder-binding/freshness are per-request) — RESOLVED by the
//!   static/dynamic split above: [`admit_static`] emits the static decision once and
//!   defers holder/freshness to the per-request conditional-grant check, so a stale or
//!   wrong-holder credential is denied at query time WITHOUT a re-materialise and is
//!   never frozen into the materialise-once view. The combined [`admit`] remains the
//!   per-request snapshot path for a single request.
//! - **`sq-wvne` / `sq-xc4y` (holder binding)** — the holder binding is the
//!   **clear-WebID v1 path** (`credentialSubject == Session.agent`), the
//!   **non-anonymous degraded path**. It authenticates the requester's WebID in the
//!   CLEAR; it does **NOT** deliver unlinkable/anonymous presentation. This is the
//!   documented degradation, not a solved problem.
//! - **`sq-tu4e`** — `revoked` is an **input-only** seeded predicate (a per-request
//!   Rust check here), never in-reasoner negation over a derived fact; there is **no
//!   deny-on-disagreement** rule in this PoC.
//!
//! ## Honest scope
//!
//! This gate verifies a CHECKED issuer signature; it does **NOT** provide privacy,
//! unlinkability, anonymity, or any cryptographic guarantee. The ZK estate it
//! composes with is research-grade and **externally UNAUDITED** (`sq-qhy4`). The issuer
//! key is operator-asserted by default (the `issuerKey → verifying-key` binding is the
//! live forgery vector D′ of §3.3); the opt-in `did` feature (`sq-pfae.3`, the `did`
//! module) can bind it from a `trust:issuerDid` instead, narrowing — not eliminating — D′.
//!
//! sq-pfae PoC (issue #940). 🤖 SPARQ agent — trust-graph authorisation PoC.

use std::collections::HashSet;

use crate::policy::{ShapeRef, TrustRule};
use crate::vocab;
use oxrdf::{NamedNode, NamedOrBlankNode, Term, Triple};
use sparq_zk::commit::commit_triples;
use sparq_zk::encode::salt_from_bytes;
use sparq_zk::field::Fr;
use sparq_zk::sig::{commitment_message, signature_from_hex, verify};

/// The live request context the admission gate binds against (§3.3 / §3.4). This is a
/// **per-request** value — the gate MUST be re-evaluated per request (`sq-xc4y`), not
/// frozen into a session-independent materialise-once view.
#[derive(Debug, Clone)]
pub struct Session {
    /// The authenticated requester's WebID (`Session.agent`). The holder binding
    /// requires the credential subject to equal this (§3.4) — the **clear-WebID**,
    /// **non-anonymous** v1 path (`sq-wvne`).
    pub agent: NamedNode,
    /// The current instant as a Unix timestamp (seconds), consulted against
    /// `trust:freshWithin` (§3.3 B′). A per-request value, never frozen.
    pub now_unix_secs: i64,
}

/// A presented credential: the claim graph `G`, the issuer signature over its
/// RDFC-1.0 commitment, the per-graph salt the commitment was minted under, the
/// issuance instant, and the revocation flag.
///
/// This is the verifiable unit — the same signed-graph unit the ZK estate already
/// produces (`sparq-zk` commits per-named-graph over RDFC-1.0-canonicalised leaves).
/// The signature is over the **checked** RDFC-1.0 commitment, NOT a self-asserted "I
/// am signed" triple (the load-bearing soundness condition, §3.3 item 1).
#[derive(Debug, Clone)]
pub struct PresentedCredential {
    /// The credential claim graph `G` (e.g. `<Jesse> schema:age 25`).
    pub graph: Vec<Triple>,
    /// The issuer Schnorr signature over `commitment_message(C(G))`, hex-encoded
    /// (`compressed(R) ‖ s`), exactly as `sparq_zk::sig::SecretKey::sign_commitment`
    /// produces. An absent/forged signature fails closed.
    pub issuer_signature_hex: String,
    /// The 32-byte per-graph salt the RDFC-1.0 commitment was minted under
    /// (`zk:rdfc10Salt`). The verifier re-commits under the same salt; a mismatched
    /// salt yields a different `C(G)` and fails the signature check.
    pub salt: [u8; 32],
    /// Issuance instant as a Unix timestamp (seconds). Compared against
    /// `Session.now_unix_secs` and the rule's `fresh_within_secs`.
    pub issued_at_unix_secs: i64,
    /// Per-request revocation flag (the **input-only** guard — `sq-tu4e`). A
    /// production wiring would consult a W3C Bitstring Status List (P6); the PoC
    /// takes the boolean directly so the fail-closed path is testable.
    pub revoked: bool,
}

/// A fact that passed admission, issuer-tagged and marked `trust:admitted` (§2.1
/// stratum boundary). It enters the **unchanged** `sparq-solid` materialiser ahead of
/// the derivation stratum.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AdmittedFact {
    /// The admitted triple (e.g. `<Jesse> schema:age 25`).
    pub triple: Triple,
    /// `trust:source` — the issuer this fact is tagged with.
    pub issuer: NamedNode,
}

/// A fact that passed the **STATIC** admission class (signature + statement-type scope +
/// reserved-predicate guard + `trust:scope`), carrying the **DYNAMIC** conditions it
/// still owes so the caller can defer them to a per-request check (`sq-xc4y`).
///
/// The static decision is session-independent — it depends only on the credential and
/// the policy — so it can be made **once, at materialise-time**. The dynamic conditions
/// ([`holder`](Self::holder) and [`not_after_unix_secs`](Self::not_after_unix_secs))
/// MUST NOT be frozen into the materialise-once view; they ride the shipped sq-0q7n
/// conditional-grant path (`auth:agent` re-checked against `Session.agent`,
/// `auth:notAfter` re-checked against `Session.now`) and are re-evaluated per request.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StaticAdmittedFact {
    /// The admitted triple (e.g. `<Jesse> schema:age 25`).
    pub triple: Triple,
    /// `trust:source` — the issuer this fact is tagged with.
    pub issuer: NamedNode,
    /// The bound holder: the credential subject the fact may only be used on behalf of.
    /// The per-request holder binding is `holder == Session.agent`, deferred to query
    /// time (NEVER frozen into the view — `sq-xc4y` / `sq-wvne` clear-WebID path).
    pub holder: NamedNode,
    /// The freshness deadline as a Unix timestamp (seconds): `issued_at + freshWithin`.
    /// The per-request freshness check is `Session.now ≤ not_after_unix_secs`, deferred
    /// to query time so a credential that lapses denies *that request* without a
    /// re-materialise (`sq-xc4y`; the sq-0q7n `auth:notAfter` precedent).
    pub not_after_unix_secs: i64,
}

/// Run the admission gate over a presented credential. Returns the admitted facts
/// (possibly empty — default-deny). NEVER panics on adversarial input: a malformed
/// signature, an uncanonicalisable graph, or a bad shape all fail closed (no admit).
///
/// `target` is the resource the request is for (the `trust:scope` containment check).
pub fn admit(
    cred: &PresentedCredential,
    rules: &[TrustRule],
    session: &Session,
    target: &NamedNode,
) -> Vec<AdmittedFact> {
    let mut admitted: Vec<AdmittedFact> = Vec::new();
    let mut seen = HashSet::new();

    // Step 1: canonicalise + commit G (sparq-canon RDFC-1.0, the same canonical unit
    // the ZK estate commits over). A graph that cannot be committed admits nothing.
    let salt = salt_from_bytes(&cred.salt);
    let Ok(commitment) = commit_triples(&cred.graph, salt) else {
        return admitted; // fail closed
    };
    let commitment_fr: Fr = commitment.commitment;

    // Parse the issuer signature once (fail closed on malformed hex).
    let Some(signature) = signature_from_hex(&cred.issuer_signature_hex) else {
        return admitted;
    };

    let mut data = None;
    for r in rules {
        // §3.2 scope: the rule must cover the target resource.
        if !scope_covers(&r.scope, target) {
            continue;
        }
        // §3.3 (1): the CHECKED issuer signature over the RDFC-1.0 commitment. A
        // self-asserted "I am signed" triple proves nothing; this verifies the
        // signature against the key the rule names.
        let msg = commitment_message(&commitment_fr);
        if !verify(&r.issuer_key, &msg, &signature) {
            continue;
        }
        // §3.3 (B′) freshness — a per-request Rust check, NOT an in-reasoner predicate.
        let age = session
            .now_unix_secs
            .saturating_sub(cred.issued_at_unix_secs);
        if age < 0 || age > r.fresh_within_secs {
            continue;
        }
        // §3.3 input-stratified revocation guard (input-only; never NAF over a
        // derived predicate — sq-tu4e).
        if cred.revoked {
            continue;
        }
        let data =
            data.get_or_insert_with(|| sparq_shacl::graph_from_triples(cred.graph.iter().cloned()));
        let shape = ShapeAdmission::new(&r.shape, &cred.graph, data);
        for t in &cred.graph {
            // The reserved-derivation-predicate guard stays in force UNDER admission:
            // a source trusted for schema:age cannot launder a solidx:/urn:sparq:/acl
            // internal triple in (§3.3 item 2). This mirrors sparq-solid's
            // `is_reserved_derivation_predicate` / `validate_principal_iri` guards.
            if is_reserved_triple(t) {
                continue;
            }
            // §2.3.2 statement-type scoping via the SHACL shape (forShape /
            // forPredicate-sugar). A triple whose subject the shape does not target,
            // or that violates the shape, is NOT of the trusted statement-type.
            if !shape.admits(t) {
                continue;
            }
            // §3.4 holder binding: the credential subject must equal the authenticated
            // requester (the clear-WebID, non-anonymous v1 path — sq-wvne). Presenting
            // a third party's credential without holder binding must NOT admit.
            if !subject_is(t, &session.agent) {
                continue;
            }
            let fact = AdmittedFact {
                triple: t.clone(),
                issuer: r.source.clone(),
            };
            if seen.insert(fact.clone()) {
                admitted.push(fact);
            }
        }
    }
    admitted
}

/// Run ONLY the **static** admission class over a presented credential, returning the
/// statically-admitted facts each tagged with the **dynamic** conditions it still owes
/// (the bound holder + the freshness deadline). This is the materialise-time half of
/// the `sq-xc4y` static/dynamic split: it decides everything that is session-
/// independent (signature, statement-type scope, reserved-predicate guard, `trust:scope`)
/// and DEFERS holder-binding + freshness to a per-request conditional-grant check.
///
/// Unlike [`admit`], this takes **no `Session`**: a static decision must not depend on
/// the request. The credential subject becomes the [`StaticAdmittedFact::holder`] the
/// grant is pinned to; the freshness deadline becomes
/// [`StaticAdmittedFact::not_after_unix_secs`]. The caller installs a conditional grant
/// that re-checks both per request — so a stale or wrong-holder credential is denied at
/// query time WITHOUT a re-materialise and is never frozen into the materialise-once
/// view.
///
/// Returns the statically-admitted facts (possibly empty — default-deny). NEVER panics
/// on adversarial input: a malformed signature, an uncanonicalisable graph, or a bad
/// shape all fail closed (no admit). The static class fails closed exactly as [`admit`]
/// does; only the holder/freshness checks move to query time.
pub fn admit_static(
    cred: &PresentedCredential,
    rules: &[TrustRule],
    target: &NamedNode,
) -> Vec<StaticAdmittedFact> {
    let mut admitted: Vec<StaticAdmittedFact> = Vec::new();
    let mut seen = HashSet::new();

    // Step 1: canonicalise + commit G (the same canonical unit the ZK estate commits
    // over). A graph that cannot be committed admits nothing.
    let salt = salt_from_bytes(&cred.salt);
    let Ok(commitment) = commit_triples(&cred.graph, salt) else {
        return admitted; // fail closed
    };
    let commitment_fr: Fr = commitment.commitment;

    let Some(signature) = signature_from_hex(&cred.issuer_signature_hex) else {
        return admitted;
    };

    let mut data = None;
    for r in rules {
        // §3.2 scope (STATIC: policy + credential only).
        if !scope_covers(&r.scope, target) {
            continue;
        }
        // §3.3 (1) CHECKED issuer signature (STATIC).
        let msg = commitment_message(&commitment_fr);
        if !verify(&r.issuer_key, &msg, &signature) {
            continue;
        }
        // §3.3 input-stratified revocation guard. Revocation is a per-credential input
        // fact (not request-dependent), so a credential KNOWN revoked at materialise is
        // dropped here too; a credential revoked LATER is handled by re-materialise
        // (epoch bump on trust-graph/status change — design §3.3 / G5).
        if cred.revoked {
            continue;
        }
        // The freshness deadline this rule imposes: issued_at + freshWithin. Carried as
        // a per-request DYNAMIC condition (NOT checked here against any `now`).
        let not_after_unix_secs = cred.issued_at_unix_secs.saturating_add(r.fresh_within_secs);

        let data =
            data.get_or_insert_with(|| sparq_shacl::graph_from_triples(cred.graph.iter().cloned()));
        let shape = ShapeAdmission::new(&r.shape, &cred.graph, data);
        for t in &cred.graph {
            // The reserved-predicate guard stays in force UNDER admission (STATIC).
            if is_reserved_triple(t) {
                continue;
            }
            // §2.3.2 statement-type scoping via the SHACL shape (STATIC).
            if !shape.admits(t) {
                continue;
            }
            // The bound holder is the credential subject — pinned into the grant's
            // `auth:agent` head, re-checked against `Session.agent` per request (the
            // DYNAMIC holder binding, deferred — §3.4 / sq-xc4y). A triple whose subject
            // is a blank node cannot be holder-bound (no stable WebID), so it is not
            // statically admissible for a credential-gated grant: fail closed.
            let NamedOrBlankNode::NamedNode(holder) = &t.subject else {
                continue;
            };
            let fact = StaticAdmittedFact {
                triple: t.clone(),
                issuer: r.source.clone(),
                holder: holder.clone(),
                not_after_unix_secs,
            };
            if seen.insert(fact.clone()) {
                admitted.push(fact);
            }
        }
    }
    admitted
}

/// Run the admission gate over a presented credential, gating on a **live** credential
/// status (the P6 status stratum, `sq-pfae.7`) instead of the PoC's per-credential
/// `revoked: bool` flag. This is the OPT-IN (`status-list` feature) wiring of the
/// [`crate::status_list`] gate into the REAL admission path.
///
/// The credential's [`crate::status_list::StatusListEntry`] is checked against the live
/// W3C Bitstring Status List via the caller's [`crate::status_list::LiveStatusCheck`] at
/// `session.now_unix_secs`. The credential is admitted ONLY when the status
/// [`crate::status_list::LiveStatus::admits`] (i.e. the bit is unset, the list resolved +
/// decoded, the index is in range, and the snapshot is fresh) — **fail-closed on
/// set/unknown/stale** exactly as the bead requires. On a non-admitting status the WHOLE
/// credential is denied (empty result), mirroring where the `cred.revoked` flag dropped it.
///
/// When the status admits, this delegates to the unchanged [`admit`] — every other gate
/// (signature, statement-type scope, freshness, holder binding) is untouched, so this is
/// strictly additive: it can only NARROW, never broaden, what [`admit`] admits.
///
/// Returns `(admitted, status)`: the admitted facts and the [`crate::status_list::LiveStatus`]
/// the decision rested on, so the caller can render a [`crate::status_list::justify_status_decision`]
/// justification for the allow OR the deny.
#[cfg(feature = "status-list")]
#[cfg_attr(docsrs, doc(cfg(feature = "status-list")))]
pub fn admit_with_status<R, D>(
    cred: &PresentedCredential,
    rules: &[TrustRule],
    session: &Session,
    target: &NamedNode,
    status_entry: &crate::status_list::StatusListEntry,
    status_check: &crate::status_list::LiveStatusCheck<R, D>,
) -> (Vec<AdmittedFact>, crate::status_list::LiveStatus)
where
    R: crate::status_list::StatusListResolver,
    D: crate::status_list::GzipDecoder,
{
    // The live status gate runs FIRST (fail-closed): a set/unknown/stale status denies the
    // whole credential before any fact is admitted. This replaces the `cred.revoked` flag
    // with a check against the live list — the bead's "gate derivations on validity".
    let status = status_check.check(status_entry, session.now_unix_secs);
    if !status.admits() {
        return (Vec::new(), status);
    }
    // Status admits ⇒ run the unchanged gate. (The `cred.revoked` boolean still applies as
    // the PoC's belt-and-braces input guard; a caller using the live path sets it `false`.)
    (admit(cred, rules, session, target), status)
}

/// `scope_covers(scope, target)` — exact-IRI or ancestor-container containment
/// (Solid slash-semantics). v1 is exact-or-prefix: a rule scoped to a container
/// covers its members; a rule scoped to a resource covers exactly that resource. A
/// rule never broadens beyond its scope (fail-closed direction).
pub fn scope_covers(scope: &NamedNode, target: &NamedNode) -> bool {
    let s = scope.as_str();
    let t = target.as_str();
    if s == t {
        return true;
    }
    // Container scope (slash-terminated) covers any descendant resource.
    s.ends_with('/') && t.starts_with(s)
}

/// Whether a predicate is in the reserved derivation-internal / auth space that no
/// external source may ever assert (the `solidx:` / `urn:sparq:` / `acl:` / `acp:`
/// guard — the analogue of `sparq-solid`'s reserved-predicate guard). A trust rule
/// cannot grant a source the right to launder these in (§3.3 item 2).
fn is_reserved_predicate(p: &NamedNode) -> bool {
    is_reserved_iri(p.as_str())
}

fn is_reserved_iri(s: &str) -> bool {
    s.starts_with("https://sparq.dev/ns/solidx#")
        || s.starts_with("https://sparq.dev/ns/auth#")
        || s.starts_with("urn:sparq:")
        || s.starts_with("http://www.w3.org/ns/auth/acl#")
        || s.starts_with("http://www.w3.org/ns/solid/acp#")
        || s == vocab::ADMITTED
}

/// [`is_reserved_predicate`], or an `rdf:type` naming a class in the same reserved space.
fn is_reserved_triple(t: &oxrdf::Triple) -> bool {
    is_reserved_predicate(&t.predicate)
        || (t.predicate.as_str() == "http://www.w3.org/1999/02/22-rdf-syntax-ns#type"
            && matches!(&t.object, oxrdf::Term::NamedNode(o) if is_reserved_iri(o.as_str())))
}

/// One SHACL validation per (rule, credential), indexed by focus node. Target selection
/// retains the admission gate's subjects-of semantics, including graph-wide violations.
struct ShapeAdmission {
    targeted: std::collections::HashSet<Term>,
    violations: std::collections::HashSet<Term>,
}

impl ShapeAdmission {
    fn new(shape: &ShapeRef, graph: &[Triple], data: &sparq_core::Graph) -> Self {
        let shapes = sparq_shacl::graph_from_triples(shape.triples.iter().cloned());
        let report = sparq_shacl::validate(data, &shapes);
        let predicates: std::collections::HashSet<&NamedNode> = shape
            .triples
            .iter()
            .filter(|t| t.predicate.as_str() == vocab::SH_TARGET_SUBJECTS_OF)
            .filter_map(|t| match &t.object {
                Term::NamedNode(n) => Some(n),
                _ => None,
            })
            .collect();
        let targeted = graph
            .iter()
            .filter(|t| predicates.contains(&t.predicate))
            .map(|t| subject_term(&t.subject))
            .collect();
        let violations = report
            .results
            .into_iter()
            .filter(|r| r.severity.ends_with("#Violation"))
            .map(|r| r.focus_node)
            .collect();
        Self {
            targeted,
            violations,
        }
    }

    fn admits(&self, t: &Triple) -> bool {
        let subject = subject_term(&t.subject);
        self.targeted.contains(&subject) && !self.violations.contains(&subject)
    }
}

/// Former per-triple validation retained as an independent differential oracle.
#[cfg(test)]
fn shape_admits_reference(shape: &ShapeRef, t: &Triple, graph: &[Triple]) -> bool {
    use sparq_shacl::{graph_from_triples, validate};

    // Only the triple's subject matters for whether the statement-type is trusted; but
    // the shape is validated over the WHOLE credential graph so that a property shape
    // referencing the subject's other triples can see them.
    let data = graph_from_triples(graph.iter().cloned());
    let shapes = graph_from_triples(shape.triples.iter().cloned());
    let report = validate(&data, &shapes);

    // The triple's subject must be selected as a focus node (i.e. the shape TARGETS
    // it). We re-derive the targeted-subject set from the desugared/inline shape: a
    // node is a focus node iff there is a `sh:targetSubjectsOf P` whose `P` the subject
    // asserts. Conformance is then "no violation for that focus node".
    let subj = subject_term(&t.subject);
    if !shape_targets_subject(shape, &subj, graph) {
        return false;
    }
    // Conformance for this focus node: no Violation result on it.
    !report
        .results
        .iter()
        .any(|r| r.focus_node == subj && r.severity.ends_with("#Violation"))
}

/// Whether the shape's `sh:targetSubjectsOf` predicates select `subject` over `graph`
/// — i.e. `subject` asserts at least one of the shape's target predicates. This is the
/// statement-type gate: an off-type triple's subject is not selected.
#[cfg(test)]
fn shape_targets_subject(shape: &ShapeRef, subject: &Term, graph: &[Triple]) -> bool {
    let target_preds: Vec<&str> = shape
        .triples
        .iter()
        .filter(|t| t.predicate.as_str() == vocab::SH_TARGET_SUBJECTS_OF)
        .filter_map(|t| match &t.object {
            Term::NamedNode(n) => Some(n.as_str()),
            _ => None,
        })
        .collect();
    if target_preds.is_empty() {
        return false; // a shape with no subjects-of target selects nothing here
    }
    graph.iter().any(|t| {
        subject_term(&t.subject) == *subject && target_preds.contains(&t.predicate.as_str())
    })
}

fn subject_is(t: &Triple, agent: &NamedNode) -> bool {
    matches!(&t.subject, NamedOrBlankNode::NamedNode(n) if n.as_str() == agent.as_str())
}

// ── Phase 5: the opt-in property-admissibility PRE-CHECK (sq-dt5hv) ────────────
//
// A single OPTIONAL pre-admission check (design §5b), behind the default-OFF
// `secprop-precheck` feature. Before the existing signature / freshness / holder
// checks, consult the merged secprop admissibility reduction
// (`crate::admissibility::admissible`) to decide whether the PRESENTED proof method
// satisfies the requester's ODRL privacy preference. Fail-closed (default-deny): a
// method that does NOT satisfy every constraint admits NOTHING — a **principled
// refusal** ("no admissible proof") rather than serving a non-conforming one.
//
// STRICT ADDITIVITY: the pre-check is consulted ONLY when a caller passes a
// `AdmissibilityPreference`; with `None` the gate is **byte-identical** to `admit`
// (the opt-in property G6 — the same posture the rest of this crate keeps). It does NOT
// redefine the reduction: it is a thin driver that hands the method's annotation graph
// and the requester's `gteq` constraints to the Phase-2 `admissible()` API.

#[cfg(feature = "secprop-precheck")]
#[cfg_attr(docsrs, doc(cfg(feature = "secprop-precheck")))]
pub use precheck::{
    admit_with_precheck, AdmissibilityConstraint, AdmissibilityPreference, PrecheckOutcome,
};

#[cfg(feature = "secprop-precheck")]
mod precheck {
    use super::{admit, AdmittedFact, PresentedCredential, Session};
    use crate::admissibility::admissible;
    use crate::secprop as secx;
    use oxrdf::NamedNode;
    use sparq_zk::secprop::{parse_annotations, Assurance, MethodAnnotations, Scope};
    use std::sync::LazyLock;

    /// The bundled secprop annotations, parsed **once** and cached for the process
    /// lifetime. `parse_annotations()` reparses the compiled-in `secprop-methods.ttl` and
    /// rebuilds the whole `BTreeMap` on every call; since [`resolve_bundled_annotations_n3`]
    /// runs on the real admission path when `secprop-precheck` is enabled, caching removes
    /// that per-admission reparse/allocation (and the DoS-amplification surface a frequent
    /// admission caller would otherwise expose). The bundled graph is a compile-time
    /// constant, so a single parse is always sound.
    static BUNDLED_ANNOTATIONS: LazyLock<std::collections::BTreeMap<String, MethodAnnotations>> =
        LazyLock::new(parse_annotations);

    /// One `gteq` admissibility constraint the requester requires (design §4.3.2): the
    /// method's asserted level for `left_operand`'s dimension must be `secx:atLeast`
    /// `right_operand`. The operator is always `odrl:gteq` — the only operator the
    /// reduction discharges (a richer ODRL profile is a later phase, `sq-uor3g`).
    ///
    /// Both operands are `NamedNode` IRIs. This is deliberate: the pre-check synthesises
    /// the policy N3 from these IRIs itself (never from caller-supplied raw N3), and emits
    /// each operand ONLY as an `odrl:` object inside a `<…>` IRIREF term, so a caller has
    /// **no channel** to inject an annotation / order / `secx:satisfies` triple or an N3
    /// rule into the reasoning document (the `sq-nrwqs` tamper-resistance property — see
    /// [`AdmissibilityPreference`]). A `NamedNode` built via `NamedNode::new` cannot carry
    /// a term-breaking character (RFC 3987), but these fields are `pub` and
    /// `NamedNode::new_unchecked` bypasses that validation; the synthesiser therefore
    /// RE-VALIDATES each operand at the emission site and **fails closed**
    /// ([`PrecheckOutcome::MalformedConstraint`]) on any non-IRIREF-safe operand, so the
    /// escape channel is structurally absent even off the `NamedNode::new` path
    /// (`sq-ddbm8` defence in depth).
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct AdmissibilityConstraint {
        /// The `odrl:leftOperand` — a `secx:requires…` leftOperand (e.g.
        /// `secx:requiresAssurance`). The reduction maps it to a property dimension via
        /// the bundled `secx:overDimension` fact base; an unmapped leftOperand satisfies
        /// nothing (fail-closed).
        pub left_operand: NamedNode,
        /// The `odrl:rightOperand` — the required threshold level (e.g. `secx:Proven`).
        pub right_operand: NamedNode,
    }

    /// A requester's machine-reasonable ODRL **privacy preference** to enforce against
    /// the presented proof method BEFORE admission (design §5b). The caller supplies
    /// only the presented method's IRI and the requester's `gteq` constraints as
    /// **structured, validated IRIs** — it supplies **no raw N3 at all**. The method's
    /// `secprop` property annotations are resolved from the bundled, drift-pinned
    /// sparq-zk `secprop-methods.ttl`, and the policy triples are synthesised internally
    /// from the [`AdmissibilityConstraint`] IRIs (the `sq-nrwqs` Phase-5.1 input-trust-
    /// boundary hardening).
    ///
    /// This makes the admissibility check **self-contained and tamper-resistant**: because
    /// there is no caller-supplied raw-N3 channel — neither for the method annotations nor
    /// for the policy — a caller **cannot** inject a `secx:hasProperty` / `secx:atLeast` /
    /// `secx:satisfies` triple (or an N3 rule) to widen a method's recorded posture, and
    /// an unknown method fails closed. (The Phase-5 `policy_n3`/`annotations_n3` raw-string
    /// fields, which raw-concatenated caller text into the reasoning document, are gone.)
    ///
    /// The honesty of the verdict is exactly the honesty of the **bundled** annotation
    /// graph: it reasons over recorded (method, property)→level claims, **not** over
    /// cryptography, and every positive sparq ZK property is at most `secx:Claimed`
    /// while the external audit (`sq-qhy4`) is open. A `requiresAssurance gteq
    /// secx:Proven` constraint therefore mechanically denies **every** sparq ZK method
    /// today — the principled refusal. This hardens the input boundary only; it makes
    /// **no** new soundness/privacy claim.
    #[derive(Debug, Clone)]
    pub struct AdmissibilityPreference {
        /// The presented proof's method IRI (the `zk:scheme` / `zk:cryptosuite` the
        /// `sparq-zk` registry records, e.g. `zk:poseidon2-rdfc10-v1`). The annotation
        /// graph is resolved from the bundled ontology keyed on this IRI; a method with
        /// no bundled annotation block is an **unknown method** and fails closed
        /// ([`PrecheckOutcome::UnknownMethod`]).
        pub method_iri: NamedNode,
        /// The requester's `gteq` constraints. The method must satisfy **every** one
        /// (default-deny). An EMPTY set is vacuously satisfied for a KNOWN method — the
        /// caller is responsible for not handing an empty preference where one is
        /// required (the `admissible` contract). An **unknown** method is denied
        /// regardless.
        pub constraints: Vec<AdmissibilityConstraint>,
    }

    /// Why a pre-check DENIED admission — the honest, machine-readable refusal detail
    /// (`PrecheckOutcome::Denied` carries it; an empty admitted set is the effect).
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum PrecheckOutcome {
        /// The method satisfied every constraint — admission proceeded normally.
        Admitted,
        /// The presented method IRI has **no annotation block in the bundled
        /// `secprop-methods.ttl`** — an unknown method. Fail-closed (default-deny): a
        /// method the estate does not record a posture for admits **nothing**,
        /// regardless of the (possibly empty) constraint set. Carries the method IRI.
        UnknownMethod {
            /// The presented method IRI that is not in the bundled annotation graph.
            method: String,
        },
        /// The method failed at least one constraint; admission was refused
        /// (fail-closed). Carries the **sorted, de-duplicated** `left_operand` IRIs of
        /// the constraints the method does not `secx:satisfies` — the "why no admissible
        /// proof" dimensions (e.g. `secx:requiresAssurance`).
        Denied {
            /// The `left_operand` IRIs of the constraints the method fails (sorted,
            /// de-duplicated, non-empty).
            unsatisfied: Vec<String>,
        },
        /// The reduction itself errored (a malformed policy fragment). This is **also**
        /// a fail-closed denial — a pre-check that cannot be evaluated must NEVER admit.
        /// Carries the reasoner's error string.
        ReductionError {
            /// The `sparq-reason` parse/closure error the reduction returned.
            error: String,
        },
        /// A constraint carried an operand IRI that is **not a well-formed IRIREF** — it
        /// contains a character the N3 `<…>` term grammar excludes (`>` / whitespace /
        /// `<` / `"` / `{` / `}` / `|` / `^` / `` ` `` / `\` / any control char) that could
        /// otherwise let the value **break out of object position** and inject a
        /// `secx:hasProperty` / `secx:satisfies` triple or an N3 rule into the synthesised
        /// policy. `NamedNode::new` (oxiri, RFC 3987) already rejects these, so this can
        /// only arise from a `NamedNode::new_unchecked` / future unvalidated-`Deserialize`
        /// operand; the pre-check **re-checks and fails closed** (`sq-ddbm8` defence in
        /// depth) rather than emitting a policy it cannot trust. Admits **nothing**. This
        /// is input-sanitisation hardening only; it makes no new soundness/privacy claim.
        MalformedConstraint {
            /// The offending operand IRI (inert — echoed only for the honest refusal
            /// detail; it is never re-emitted into any reasoning document).
            operand: String,
        },
    }

    /// Run the admission gate with an OPTIONAL Phase-5 property-admissibility pre-check.
    ///
    /// With `preference == None` this is **byte-identical** to `admit` (strict
    /// additivity — the opt-in property): no reasoning runs, no behaviour changes.
    ///
    /// With `preference == Some(p)` the gate FIRST **resolves `p.method_iri`'s secprop
    /// annotation graph from the bundled, drift-pinned sparq-zk `secprop-methods.ttl`**
    /// (`sq-nrwqs` Phase 5.1) — the caller does NOT supply the annotations, so it cannot
    /// widen the method's recorded posture. An unknown method (no bundled block) fails
    /// closed ([`PrecheckOutcome::UnknownMethod`]). It then **synthesises** the policy N3
    /// from `p`'s structured [`AdmissibilityConstraint`]s (the caller supplies no raw N3,
    /// so it cannot inject annotation / order / `secx:satisfies` triples or rules) and
    /// consults `crate::admissibility::admissible` over the method IRI, the synthesised
    /// policy, and the **bundled** annotation graph. If the method does **not** satisfy
    /// every constraint — or the reduction errors — the gate **fails closed**: it returns
    /// an EMPTY admitted set and the `PrecheckOutcome` explaining the refusal, and the
    /// existing signature / freshness / holder checks are **never reached**. Only when the
    /// pre-check is satisfied does the unchanged `admit` gate run.
    ///
    /// The pre-check is **default-deny and orthogonal** to the cryptographic checks: it
    /// constrains *which proof method* is acceptable, NOT whether the signature
    /// verifies. A satisfied pre-check does NOT weaken any downstream check — `admit`
    /// still verifies the checked issuer signature, freshness, statement-type scope, and
    /// the clear-WebID holder binding. The pre-check can only ever DENY, never broaden.
    /// It hardens the input trust boundary; it makes **no** new soundness/privacy claim
    /// (the estate stays externally UNAUDITED, `sq-qhy4`).
    ///
    /// Returns the admitted facts (empty on a pre-check denial — default-deny) and the
    /// `PrecheckOutcome`.
    pub fn admit_with_precheck(
        cred: &PresentedCredential,
        rules: &[crate::policy::TrustRule],
        session: &Session,
        target: &NamedNode,
        preference: Option<&AdmissibilityPreference>,
    ) -> (Vec<AdmittedFact>, PrecheckOutcome) {
        let Some(pref) = preference else {
            // OPT-IN: no preference => byte-identical to the current gate.
            return (
                admit(cred, rules, session, target),
                PrecheckOutcome::Admitted,
            );
        };

        // Resolve the method's annotation graph from the BUNDLED ontology (NOT the
        // caller). An unknown method fails closed: it admits nothing, whatever the
        // constraint set.
        let Some(annotations_n3) = resolve_bundled_annotations_n3(pref.method_iri.as_str()) else {
            return (
                Vec::new(),
                PrecheckOutcome::UnknownMethod {
                    method: pref.method_iri.as_str().to_owned(),
                },
            );
        };

        // Synthesise the policy N3 from the STRUCTURED constraints. Each constraint gets a
        // fresh internal `urn:sparq:` constraint IRI, and its VALIDATED left/right operand
        // IRIs are emitted only as `odrl:` objects — the caller supplies no raw N3, so it
        // has no way to inject a `secx:hasProperty` / `secx:atLeast` / `secx:satisfies`
        // triple (or an N3 rule) into the reasoning document. `cid -> left_operand` lets
        // us report the failed dimensions.
        //
        // sq-ddbm8 DEFENCE-IN-DEPTH: `synthesise_policy` re-validates that every
        // operand is a well-formed IRIREF before writing it inside a `<…>` term. A
        // `NamedNode::new_unchecked` / unvalidated-`Deserialize` operand carrying a
        // term-breaking char (only reachable OUTSIDE the `NamedNode::new` path) is refused
        // BEFORE it can reach the reasoning document — fail closed, admit nothing.
        let (constraint_iris, policy_n3) = match synthesise_policy(&pref.constraints) {
            Ok(v) => v,
            Err(operand) => {
                return (Vec::new(), PrecheckOutcome::MalformedConstraint { operand });
            }
        };
        let cid_to_left: std::collections::BTreeMap<&str, &str> = constraint_iris
            .iter()
            .zip(pref.constraints.iter())
            .map(|(cid, c)| (cid.as_str(), c.left_operand.as_str()))
            .collect();
        let constraint_refs: Vec<&str> = constraint_iris.iter().map(String::as_str).collect();
        match admissible(
            pref.method_iri.as_str(),
            &constraint_refs,
            &policy_n3,
            &annotations_n3,
        ) {
            // The method satisfies every constraint: proceed to the unchanged gate.
            Ok(a) if a.admissible => (
                admit(cred, rules, session, target),
                PrecheckOutcome::Admitted,
            ),
            // The method fails at least one constraint: fail-closed, admit nothing. Map
            // the failed synthetic constraint IRIs back to their left_operand dimensions
            // (sorted, de-duplicated) for the honest "why not admissible" detail.
            Ok(a) => {
                let unsatisfied: Vec<String> = a
                    .unsatisfied
                    .iter()
                    .filter_map(|cid| cid_to_left.get(cid.as_str()))
                    .map(|lo| (*lo).to_owned())
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect();
                (Vec::new(), PrecheckOutcome::Denied { unsatisfied })
            }
            // The reduction errored: ALSO fail-closed (a pre-check that cannot be
            // evaluated must never admit).
            Err(error) => (Vec::new(), PrecheckOutcome::ReductionError { error }),
        }
    }

    /// Synthesise the policy N3 (and the parallel internal constraint IRIs) from the
    /// requester's STRUCTURED `gteq` constraints. Each constraint `i` becomes a fresh
    /// internal `urn:sparq:secprop:c{i}` `odrl:Constraint` whose `odrl:leftOperand` /
    /// `odrl:rightOperand` are the caller's **validated** operand IRIs emitted as `<…>`
    /// objects and a fixed `odrl:operator odrl:gteq`. The fragment has NO `@prefix`
    /// preamble (the reduction prepends it).
    ///
    /// SECURITY (`sq-nrwqs` + `sq-ddbm8`): the only caller-controlled text placed in the
    /// reasoning document is the operand IRIs, and always as `odrl:` OBJECTS — never as a
    /// predicate or a method-subject triple. Each operand is emitted inside a `<…>` IRIREF
    /// term, so breaking out of that term to inject a `secx:hasProperty` / `secx:atLeast`
    /// / `secx:satisfies` triple or an N3 rule would require the IRI to carry the term
    /// closer `>` (or the whitespace / control / `< " { } | ^` `` ` `` `\` characters that
    /// also delimit an IRIREF). `NamedNode::new` validates against RFC 3987 (oxiri), which
    /// excludes exactly those characters — but the operand fields are `pub` and a
    /// `NamedNode::new_unchecked` / future unvalidated-`Deserialize` operand bypasses that
    /// validation. So this synthesiser **re-checks each operand with [`is_iriref_safe`]
    /// at the emission site** and **fails closed** (`Err(offending_operand)`) if any is
    /// not a well-formed IRIREF, rather than emitting a term a caller could escape. That
    /// makes the widening channel structurally absent even off the `NamedNode::new` path —
    /// not merely renamed — and the guard cannot be bypassed by a future caller of this fn.
    ///
    /// # Errors
    /// Returns the offending operand IRI string if any constraint's `left_operand` or
    /// `right_operand` is not IRIREF-safe (the caller turns this into a fail-closed
    /// [`PrecheckOutcome::MalformedConstraint`]).
    fn synthesise_policy(
        constraints: &[AdmissibilityConstraint],
    ) -> Result<(Vec<String>, String), String> {
        use std::fmt::Write as _;
        let mut iris = Vec::with_capacity(constraints.len());
        let mut n3 = String::new();
        for (i, c) in constraints.iter().enumerate() {
            // sq-ddbm8: re-validate BOTH operands are IRIREF-safe before either
            // is written inside a `<…>` term. Fail closed on the first unsafe operand — an
            // operand that could break out of object position must NEVER reach the policy.
            for operand in [c.left_operand.as_str(), c.right_operand.as_str()] {
                if !is_iriref_safe(operand) {
                    return Err(operand.to_owned());
                }
            }
            let cid = format!("urn:sparq:secprop:c{}", i);
            // Write the per-constraint fragment straight into `n3` — writing to
            // an in-memory `String` is infallible, so no temporary `String` allocation on
            // the admission path. Byte-output-identical (same fragment, trailing `.\n`).
            writeln!(
                n3,
                "<{}> odrl:leftOperand <{}> ; odrl:operator odrl:gteq ; odrl:rightOperand <{}> .",
                cid,
                c.left_operand.as_str(),
                c.right_operand.as_str(),
            )
            .expect("writing to an in-memory String is infallible");
            iris.push(cid);
        }
        Ok((iris, n3))
    }

    /// Whether `iri` is safe to embed verbatim inside an N3/Turtle `<…>` IRIREF term —
    /// i.e. it contains **none** of the characters the IRIREF grammar excludes and that
    /// would otherwise let the value break out of object position and inject a triple or
    /// rule into the synthesised policy. The excluded set is exactly the N-Triples/Turtle
    /// `IRIREF` production's: every code point `<= U+0020` (all C0 controls **and** space),
    /// plus `<`, `>`, `"`, `{`, `}`, `|`, `^`, `` ` ``, `\`. This is precisely the set
    /// oxiri (`NamedNode::new`, RFC 3987) already rejects, so a `NamedNode::new`-validated
    /// operand ALWAYS passes; the check only ever catches a `NamedNode::new_unchecked` /
    /// unvalidated-`Deserialize` operand (`sq-ddbm8` defence in depth). An empty IRI is
    /// also rejected (a degenerate operand that is never a real dimension/level).
    fn is_iriref_safe(iri: &str) -> bool {
        !iri.is_empty()
            && iri.chars().all(|ch| {
                ch > '\u{20}' && !matches!(ch, '<' | '>' | '"' | '{' | '}' | '|' | '^' | '`' | '\\')
            })
    }

    /// Resolve the presented method's secprop annotation graph from the **bundled,
    /// drift-pinned** sparq-zk `secprop-methods.ttl` (the canonical source of truth,
    /// behind sparq-zk's `secprop-annotations` feature), serialised into the N3 fragment
    /// [`admissible`] reads. Returns `None` iff `method_iri` has no bundled annotation
    /// block — an **unknown method**, which the caller turns into a fail-closed
    /// [`PrecheckOutcome::UnknownMethod`].
    ///
    /// This is the `sq-nrwqs` hardening: the annotations are NEVER taken from the
    /// caller, so a caller cannot inject a widened posture. The bundled graph is parsed
    /// once and cached in [`BUNDLED_ANNOTATIONS`]; this call only looks up `method_iri`
    /// and serialises its per-method fragment.
    fn resolve_bundled_annotations_n3(method_iri: &str) -> Option<String> {
        let ann = BUNDLED_ANNOTATIONS.get(method_iri)?;
        Some(annotations_n3_from_bundled(ann))
    }

    /// Serialise the **query-proof-layer** assertions of one bundled method annotation
    /// into the `secx:hasProperty [ secx:property … ; secx:level … ]` N3 fragment the
    /// admissibility reduction reads, plus a derived method-wide `secx:AssuranceLevel`
    /// block (see [`derived_assurance_level`]).
    ///
    /// SOURCE-LAYER-ONLY assertions are EXCLUDED (the §5a rule 2 non-transfer rule): a
    /// property of a source credential's cryptosuite must never satisfy a query-proof
    /// constraint. Only `secx:property` + `secx:level` are emitted (the two the
    /// discharge rule reads); the assurance/audit/scope reification is not needed by the
    /// reduction. The subject is the full method IRI so it matches the `method` argument
    /// `admissible` filters on.
    fn annotations_n3_from_bundled(ann: &MethodAnnotations) -> String {
        let mut blocks: Vec<String> = Vec::new();
        for a in ann
            .assertions
            .iter()
            .filter(|a| a.scope == Scope::QueryProofLayer)
        {
            if let Some(level) = &a.level {
                blocks.push(format!(
                    "[ secx:property <{}> ; secx:level <{}> ]",
                    a.property, level
                ));
            }
        }
        if let Some(level) = derived_assurance_level(ann) {
            blocks.push(format!(
                "[ secx:property <{}> ; secx:level <{}> ]",
                secx::SECX_ASSURANCE_LEVEL,
                level
            ));
        }
        if blocks.is_empty() {
            return String::new();
        }
        format!(
            "<{}> secx:hasProperty {} .\n",
            ann.method,
            blocks.join(" ,\n  ")
        )
    }

    /// The method-wide `secx:AssuranceLevel` for the `requiresAssurance` dimension =
    /// the **weakest** assurance across the method's POSITIVE, query-proof-layer
    /// property claims (`None` if it has none — then `requiresAssurance` denies). A
    /// settled-NEGATIVE level (`PQForgeable` / `Replayable` / `SchemeRevealed`) may be
    /// `secx:Proven` and is EXCLUDED: it is a fact ABOUT the construction, not a
    /// positive assurance the requester relies on. While `sq-qhy4` is open every
    /// positive sparq ZK claim is `secx:Claimed`, so this is `Claimed` for every sparq
    /// method today — a `requiresAssurance gteq secx:Proven` constraint therefore denies
    /// every method (the principled refusal, unchanged from Phase 5). This derivation is
    /// bounded by the honesty of the bundled graph; it makes no new soundness claim.
    fn derived_assurance_level(ann: &MethodAnnotations) -> Option<&'static str> {
        // rank: Conjectured < Claimed < Proven; take the MIN over positive QP claims.
        let mut min_rank: Option<u8> = None;
        for a in ann
            .assertions
            .iter()
            .filter(|a| a.scope == Scope::QueryProofLayer)
        {
            if is_settled_negative_level(a.level.as_deref()) {
                continue;
            }
            let rank = match a.assurance {
                Assurance::Conjectured => 0u8,
                Assurance::Claimed => 1,
                Assurance::Proven => 2,
            };
            min_rank = Some(min_rank.map_or(rank, |m| m.min(rank)));
        }
        match min_rank? {
            0 => Some(secx::SECX_CONJECTURED),
            1 => Some(secx::SECX_CLAIMED),
            _ => Some(secx::SECX_PROVEN),
        }
    }

    /// Whether `level` is one of the three SETTLED-NEGATIVE levels that may legitimately
    /// carry `secx:Proven` (a fact about the construction, not a positive assurance).
    /// Excluded from [`derived_assurance_level`] so a Proven negative never inflates a
    /// method's positive assurance. Mirrors sparq-zk `secprop`'s own `is_positive`
    /// negative set; the level IRIs are the drift-pinned `crate::secprop` constants.
    fn is_settled_negative_level(level: Option<&str>) -> bool {
        match level {
            Some(l) => {
                l == secx::SECX_PQ_FORGEABLE
                    || l == secx::SECX_REPLAYABLE
                    || l == secx::SECX_SCHEME_REVEALED
            }
            None => false,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::policy::TrustRule;
        use oxrdf::NamedNode;

        const METHOD: &str = "https://sparq.dev/ns/zk#poseidon2-rdfc10-v1";
        // Requester leftOperand IRIs (the §4.3.2 `secx:requires…` operands). They map to
        // property dimensions via the bundled `secx:overDimension` fact base.
        const REQ_UNLINK: &str = "https://w3id.org/zkp-sparql/sec-prop#requiresUnlinkabilityScope";
        const REQ_ASSUR: &str = "https://w3id.org/zkp-sparql/sec-prop#requiresAssurance";
        const REQ_SOUND: &str = "https://w3id.org/zkp-sparql/sec-prop#requiresSoundness";
        const REQ_ZK: &str = "https://w3id.org/zkp-sparql/sec-prop#requiresZeroKnowledge";

        /// A single `gteq` constraint from two IRI strings.
        fn c(left: &str, right: &str) -> AdmissibilityConstraint {
            AdmissibilityConstraint {
                left_operand: NamedNode::new(left).unwrap(),
                right_operand: NamedNode::new(right).unwrap(),
            }
        }

        /// Build a preference. The method's property annotations are NOT supplied here —
        /// they are resolved from the bundled `secprop-methods.ttl` by the pre-check, and
        /// the policy N3 is synthesised from the STRUCTURED constraints (the `sq-nrwqs`
        /// hardening — the caller supplies no raw N3).
        fn pref(method: &str, constraints: Vec<AdmissibilityConstraint>) -> AdmissibilityPreference {
            AdmissibilityPreference {
                method_iri: NamedNode::new(method).unwrap(),
                constraints,
            }
        }

        /// The pre-check NEVER admits without a real `admit` pass: it returns no facts
        /// here (no rules), so the OUTCOME — not the fact set — is what these unit tests
        /// pin. The `admit` integration path is covered end-to-end in
        /// `tests/secprop_precheck_e2e.rs` (with a real signed credential).
        fn run(p: Option<&AdmissibilityPreference>) -> PrecheckOutcome {
            let cred = PresentedCredential {
                graph: Vec::new(),
                issuer_signature_hex: String::new(),
                salt: [0u8; 32],
                issued_at_unix_secs: 0,
                revoked: false,
            };
            let rules: Vec<TrustRule> = Vec::new();
            let session = Session {
                agent: NamedNode::new("https://a.ex/me").unwrap(),
                now_unix_secs: 0,
            };
            let target = NamedNode::new("https://pod.ex/r").unwrap();
            let (_facts, outcome) = admit_with_precheck(&cred, &rules, &session, &target, p);
            outcome
        }

        /// With NO preference the outcome is `Admitted` and the gate runs unchanged
        /// (strict additivity — the opt-in property). No reasoning happens.
        #[test]
        fn no_preference_is_admitted_and_runs_the_unchanged_gate() {
            assert_eq!(run(None), PrecheckOutcome::Admitted);
        }

        /// A satisfiable `gteq` preference admits — resolved from the BUNDLED graph: the
        /// string-canonical method holds `UnlinkabilityScope PerPresentation` (`atLeast`
        /// the required PerPresentation, reflexive) and its derived AssuranceLevel is
        /// `Claimed` (≥ Claimed).
        #[test]
        fn satisfiable_preference_admits() {
            let p = pref(
                METHOD,
                vec![
                    c(REQ_UNLINK, crate::secprop::SECX_PER_PRESENTATION),
                    c(REQ_ASSUR, crate::secprop::SECX_CLAIMED),
                ],
            );
            assert_eq!(run(Some(&p)), PrecheckOutcome::Admitted);
        }

        /// BUNDLED-RESOLUTION direct test: a constraint over a dimension present ONLY in
        /// the bundled ontology (`Soundness KnowledgeSound`, which no caller ever
        /// supplied here) is satisfiable — proving the annotations are resolved from the
        /// shipped `secprop-methods.ttl`, not from any caller-supplied fragment.
        #[test]
        fn bundled_resolution_uses_the_shipped_ontology() {
            let p = pref(
                METHOD,
                vec![c(REQ_SOUND, crate::secprop::SECX_KNOWLEDGE_SOUND)],
            );
            assert_eq!(run(Some(&p)), PrecheckOutcome::Admitted);
        }

        /// An unsatisfiable constraint FAILS CLOSED: `requiresAssurance gteq Proven`
        /// removes the Claimed-only method — every sparq ZK method while `sq-qhy4` is
        /// open. The outcome names the unsatisfied dimension (the honest refusal).
        #[test]
        fn assurance_gteq_proven_denies_fail_closed() {
            let p = pref(METHOD, vec![c(REQ_ASSUR, crate::secprop::SECX_PROVEN)]);
            assert_eq!(
                run(Some(&p)),
                PrecheckOutcome::Denied {
                    unsatisfied: vec![REQ_ASSUR.to_owned()]
                },
            );
        }

        /// TAMPER-RESISTANCE (answers the residual-channel review): a caller cannot widen
        /// the method's recorded posture through the POLICY either. A caller that TRIES to
        /// inject a `secx:hasProperty [ secx:property secx:AssuranceLevel ; secx:level
        /// secx:Proven ]` fact — the exact Phase-5 raw-`policy_n3` exploit — has NO channel
        /// to do so: operands are validated `NamedNode`s emitted only as `odrl:` objects.
        /// Even naming the injection predicate/level as operands cannot forge a
        /// `secx:satisfies` fact, so `requiresAssurance gteq Proven` STILL denies (the
        /// bundled graph pins the method at `Claimed`). This is the `sq-nrwqs` fail-closed
        /// input-boundary hardening; no new soundness claim.
        #[test]
        fn caller_cannot_widen_bundled_assurance_via_policy_injection() {
            // The closest a structured caller can get to the old raw-N3 injection: name
            // the annotation predicate + a Proven level as the constraint operands. These
            // are inert as `odrl:` objects — they cannot assert a method posture.
            let inject = pref(
                METHOD,
                vec![
                    c(REQ_ASSUR, crate::secprop::SECX_PROVEN),
                    c(
                        crate::secprop::SECX_HAS_PROPERTY,
                        crate::secprop::SECX_ASSURANCE_LEVEL,
                    ),
                ],
            );
            assert!(
                matches!(run(Some(&inject)), PrecheckOutcome::Denied { .. }),
                "no structured-operand injection can widen the method to Proven",
            );
            // And the derived AssuranceLevel the reduction sees is Claimed, never Proven.
            let ann = super::parse_annotations();
            let m = ann.get(METHOD).expect("string-canonical is bundled");
            assert_eq!(
                super::derived_assurance_level(m),
                Some(crate::secprop::SECX_CLAIMED),
                "the bundled method's derived assurance must be Claimed, never Proven",
            );
            // The synthesised policy contains ONLY `odrl:` predicates — no `secx:` sink.
            let (_iris, policy) = super::synthesise_policy(&inject.constraints)
                .expect("validated operands synthesise a policy");
            assert!(
                !policy.contains("secx:hasProperty")
                    && !policy.contains("secx:level")
                    && !policy.contains("secx:satisfies")
                    && !policy.contains("secx:atLeast"),
                "the synthesised policy must never carry a secx annotation/order/satisfies predicate: {}",
                policy,
            );
        }

        /// Requiring a STRONGER level than the bundled method actually holds fails closed:
        /// the method holds `ZeroKnowledgeType ComputationalZK`, which is NOT `atLeast`
        /// the required `PerfectZK`.
        #[test]
        fn stronger_level_than_bundled_denies_fail_closed() {
            let p = pref(METHOD, vec![c(REQ_ZK, crate::secprop::SECX_PERFECT_ZK)]);
            assert!(matches!(run(Some(&p)), PrecheckOutcome::Denied { .. }));
        }

        /// A method IRI with NO bundled annotation block is an UNKNOWN method ⇒ fail
        /// closed with the dedicated `UnknownMethod` outcome — regardless of the
        /// constraints (the graph is the source of truth; a wrong/unknown method is
        /// denied).
        #[test]
        fn unknown_method_denies_fail_closed() {
            let p = pref(
                "https://sparq.dev/ns/zk#no-such-method",
                vec![c(REQ_ASSUR, crate::secprop::SECX_CLAIMED)],
            );
            assert_eq!(
                run(Some(&p)),
                PrecheckOutcome::UnknownMethod {
                    method: "https://sparq.dev/ns/zk#no-such-method".to_owned()
                },
            );
        }

        /// An unknown method fails closed EVEN with an EMPTY constraint set — the
        /// fail-closed-on-unknown-method rule (`sq-nrwqs`) is stronger than the vacuous
        /// admit for a known method.
        #[test]
        fn unknown_method_with_no_constraints_denies() {
            let p = pref("https://sparq.dev/ns/zk#no-such-method", vec![]);
            assert!(matches!(run(Some(&p)), PrecheckOutcome::UnknownMethod { .. }));
        }

        /// An EMPTY constraint set is vacuously satisfied for a KNOWN method (the
        /// `admissible` contract) — admitted. (A caller wanting deny-on-empty must not
        /// hand an empty preference.)
        #[test]
        fn empty_constraint_set_is_vacuously_admitted_for_known_method() {
            let p = pref(METHOD, vec![]);
            assert_eq!(run(Some(&p)), PrecheckOutcome::Admitted);
        }

        /// The synthesised policy is well-formed and carries every constraint's operands
        /// as `odrl:` triples with a fresh internal `urn:sparq:` constraint IRI — the
        /// only caller text in the reasoning document, always in object position.
        #[test]
        fn synthesised_policy_is_well_formed_and_secx_free() {
            let (iris, policy) = super::synthesise_policy(&[
                c(REQ_ASSUR, crate::secprop::SECX_CLAIMED),
                c(REQ_SOUND, crate::secprop::SECX_KNOWLEDGE_SOUND),
            ])
            .expect("validated operands synthesise a policy");
            assert_eq!(iris.len(), 2);
            assert!(iris.iter().all(|i| i.starts_with("urn:sparq:secprop:c")));
            assert!(policy.contains(REQ_ASSUR) && policy.contains(REQ_SOUND));
            assert!(policy.contains("odrl:operator odrl:gteq"));
            // No `secx:` predicate is ever emitted by the policy synthesiser.
            for pred in ["secx:hasProperty", "secx:level", "secx:property", "secx:satisfies"] {
                assert!(!policy.contains(pred), "policy leaked a secx predicate: {}", pred);
            }
        }

        /// DEFENCE-IN-DEPTH (`sq-ddbm8`): [`is_iriref_safe`] accepts every IRI the real
        /// admission path uses and rejects **exactly** the characters the N-Triples/Turtle
        /// `IRIREF` grammar excludes — the chars that could break an operand out of its
        /// `<…>` object term. This is the closure the injection guard rests on.
        #[test]
        fn is_iriref_safe_accepts_valid_iris_and_rejects_term_breaking_chars() {
            for ok in [
                METHOD,
                REQ_ASSUR,
                REQ_SOUND,
                REQ_ZK,
                REQ_UNLINK,
                crate::secprop::SECX_PROVEN,
                crate::secprop::SECX_CLAIMED,
                "urn:sparq:secprop:c0",
            ] {
                assert!(super::is_iriref_safe(ok), "must accept valid IRI: {}", ok);
            }
            // An empty operand is a degenerate, never-real dimension/level — rejected.
            assert!(!super::is_iriref_safe(""));
            // Every excluded IRIREF character makes an otherwise-valid IRI unsafe.
            for bad in [
                '>', '<', '"', '{', '}', '|', '^', '`', '\\', ' ', '\t', '\n', '\r', '\u{0}',
                '\u{1f}',
            ] {
                let iri = format!("urn:x{}y", bad);
                assert!(
                    !super::is_iriref_safe(&iri),
                    "must reject term-breaking char {:?}",
                    bad,
                );
            }
        }

        /// INJECTION CLOSURE (`sq-ddbm8`): a caller that constructs an operand via
        /// `NamedNode::new_unchecked` — the ONLY route past `NamedNode::new`'s RFC-3987
        /// validation — and tries to close the `<…>` object term with `>` and inject a
        /// `<METHOD> secx:hasProperty [ … secx:level secx:Proven ]` widening triple is
        /// **rejected before synthesis**: the pre-check fails closed with
        /// `MalformedConstraint`, the synthesiser returns `Err` (never emits the payload),
        /// and the bundled method's derived assurance stays `Claimed` — the injection could
        /// not (and did not) widen it. Input-sanitisation hardening only; no soundness claim.
        #[test]
        fn new_unchecked_escaping_left_operand_fails_closed_no_widening() {
            let payload = format!(
                "urn:x> secx:hasProperty [ secx:property secx:AssuranceLevel ; secx:level \
                 secx:Proven ] . <{}> secx:hasProperty [ secx:property secx:AssuranceLevel ; \
                 secx:level secx:Proven ] . <urn:sink",
                METHOD,
            );
            let p = AdmissibilityPreference {
                method_iri: NamedNode::new(METHOD).unwrap(),
                constraints: vec![AdmissibilityConstraint {
                    left_operand: NamedNode::new_unchecked(payload.clone()),
                    right_operand: NamedNode::new(crate::secprop::SECX_PROVEN).unwrap(),
                }],
            };
            // Fail closed: the pre-check refuses the malformed operand and admits nothing.
            assert_eq!(
                run(Some(&p)),
                PrecheckOutcome::MalformedConstraint {
                    operand: payload.clone(),
                },
            );
            // The synthesiser NEVER emits the payload: it returns Err with the operand.
            assert_eq!(
                super::synthesise_policy(&p.constraints).unwrap_err(),
                payload,
            );
            // Belt-and-braces: the bundled method's derived assurance is STILL Claimed.
            let ann = super::parse_annotations();
            let m = ann.get(METHOD).expect("string-canonical is bundled");
            assert_eq!(
                super::derived_assurance_level(m),
                Some(crate::secprop::SECX_CLAIMED),
                "an injection attempt must never widen the bundled method to Proven",
            );
        }

        /// The RIGHT operand is guarded identically (`sq-ddbm8`): an escaping
        /// `new_unchecked` `right_operand` also fails closed with `MalformedConstraint`.
        #[test]
        fn new_unchecked_escaping_right_operand_fails_closed() {
            let payload = "urn:x> . <urn:sink";
            let p = AdmissibilityPreference {
                method_iri: NamedNode::new(METHOD).unwrap(),
                constraints: vec![AdmissibilityConstraint {
                    left_operand: NamedNode::new(REQ_ASSUR).unwrap(),
                    right_operand: NamedNode::new_unchecked(payload),
                }],
            };
            assert_eq!(
                run(Some(&p)),
                PrecheckOutcome::MalformedConstraint {
                    operand: payload.to_owned(),
                },
            );
        }

        /// A malformed operand fails closed EVEN when a first, VALID constraint precedes it
        /// (`sq-ddbm8`): the guard is per-operand and the whole pre-check denies — the
        /// injection can never be "diluted" into a vacuous admit by neutralising it.
        #[test]
        fn one_malformed_operand_among_valid_ones_fails_the_whole_precheck() {
            let bad = "urn:evil> . <urn:sink";
            let p = AdmissibilityPreference {
                method_iri: NamedNode::new(METHOD).unwrap(),
                constraints: vec![
                    c(REQ_ASSUR, crate::secprop::SECX_CLAIMED),
                    AdmissibilityConstraint {
                        left_operand: NamedNode::new_unchecked(bad),
                        right_operand: NamedNode::new(crate::secprop::SECX_PROVEN).unwrap(),
                    },
                ],
            };
            assert_eq!(
                run(Some(&p)),
                PrecheckOutcome::MalformedConstraint {
                    operand: bad.to_owned(),
                },
            );
        }

        /// INVARIANT (`sq-ddbm8`): `synthesise_policy` emits every caller operand ONLY as an
        /// `odrl:` OBJECT under an internally-generated `urn:sparq:secprop:c{i}` subject.
        /// This pins the positional structure of each synthesised statement, so it FAILS if
        /// the synthesiser is ever extended to emit an operand as a SUBJECT or a PREDICATE —
        /// the exact regression that would reopen the widening channel.
        #[test]
        fn synthesise_policy_emits_operands_only_as_objects() {
            const L0: &str = "urn:test:left-operand-zero";
            const R0: &str = "urn:test:right-operand-zero";
            const L1: &str = "urn:test:left-operand-one";
            const R1: &str = "urn:test:right-operand-one";
            let (iris, policy) = super::synthesise_policy(&[c(L0, R0), c(L1, R1)])
                .expect("validated operands synthesise a policy");
            assert_eq!(iris, vec!["urn:sparq:secprop:c0", "urn:sparq:secprop:c1"]);
            let operands = [L0, R0, L1, R1];
            let expected = [(L0, R0), (L1, R1)];
            let lines: Vec<&str> = policy.lines().filter(|l| !l.trim().is_empty()).collect();
            assert_eq!(lines.len(), 2, "one statement per constraint: {}", policy);
            for (i, line) in lines.iter().enumerate() {
                let tok: Vec<&str> = line.split_whitespace().collect();
                // Deterministic `S P O ; P O ; P O .` shape (operands carry no whitespace).
                assert_eq!(tok.len(), 10, "unexpected statement shape: {}", line);
                // SUBJECT (token 0) is the INTERNAL cid — never a caller operand.
                assert_eq!(tok[0], format!("<urn:sparq:secprop:c{}>", i).as_str());
                assert!(
                    !operands.iter().any(|o| tok[0] == format!("<{}>", o)),
                    "an operand appeared in SUBJECT position: {}",
                    line,
                );
                // PREDICATES (tokens 1, 4, 7) are fixed `odrl:` predicates — never an operand.
                for p in [tok[1], tok[4], tok[7]] {
                    assert!(p.starts_with("odrl:"), "non-odrl predicate {}: {}", p, line);
                    assert!(
                        !operands.iter().any(|o| p == format!("<{}>", o)),
                        "an operand appeared in PREDICATE position: {}",
                        line,
                    );
                }
                assert_eq!(tok[4], "odrl:operator");
                assert_eq!(tok[5], "odrl:gteq");
                // OBJECTS (tokens 2, 8) are exactly this constraint's operands.
                let (l, r) = expected[i];
                assert_eq!(tok[2], format!("<{}>", l).as_str());
                assert_eq!(tok[8], format!("<{}>", r).as_str());
            }
            // No statement BEGINS with an operand IRI (belt-and-braces subject check).
            for o in operands {
                assert!(
                    !policy
                        .lines()
                        .any(|l| l.trim_start().starts_with(&format!("<{}>", o))),
                    "operand {} began a statement (subject position)",
                    o,
                );
            }
        }

        /// DIRECT resolver test: the resolved N3 fragment for a known method carries the
        /// bundled `Soundness`/`KnowledgeSound` pair and a derived `AssuranceLevel`, and
        /// an unknown method resolves to `None` (fail-closed).
        #[test]
        fn resolver_serialises_bundled_levels_and_derives_assurance() {
            let n3 = super::resolve_bundled_annotations_n3(METHOD)
                .expect("string-canonical is bundled");
            assert!(
                n3.contains(crate::secprop::SECX_SOUNDNESS)
                    && n3.contains(crate::secprop::SECX_KNOWLEDGE_SOUND),
                "the resolved fragment must carry the bundled Soundness/KnowledgeSound pair: {}",
                n3,
            );
            assert!(
                n3.contains(crate::secprop::SECX_ASSURANCE_LEVEL)
                    && n3.contains(crate::secprop::SECX_CLAIMED),
                "the resolved fragment must carry a derived AssuranceLevel of Claimed: {}",
                n3,
            );
            assert!(
                super::resolve_bundled_annotations_n3("https://sparq.dev/ns/zk#no-such-method")
                    .is_none(),
                "an unknown method must resolve to None (fail-closed)",
            );
        }

        /// DIRECT test that a settled-NEGATIVE Proven level (`PQForgeable`) is EXCLUDED
        /// from the derived assurance, so a method's positive assurance is never inflated
        /// by a negative Proven fact about the construction.
        #[test]
        fn settled_negative_is_excluded_from_derived_assurance() {
            assert!(super::is_settled_negative_level(Some(
                crate::secprop::SECX_PQ_FORGEABLE
            )));
            assert!(!super::is_settled_negative_level(Some(
                crate::secprop::SECX_KNOWLEDGE_SOUND
            )));
            assert!(!super::is_settled_negative_level(None));
        }
    }
}

fn subject_term(s: &NamedOrBlankNode) -> Term {
    match s {
        NamedOrBlankNode::NamedNode(n) => Term::NamedNode(n.clone()),
        NamedOrBlankNode::BlankNode(b) => Term::BlankNode(b.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iri(s: &str) -> NamedNode {
        NamedNode::new(s).unwrap()
    }

    // Preserve the former gate independently of the cached-report implementation.
    fn admit_reference(
        cred: &PresentedCredential,
        rules: &[TrustRule],
        session: &Session,
        target: &NamedNode,
    ) -> Vec<AdmittedFact> {
        let mut admitted: Vec<AdmittedFact> = Vec::new();

        let salt = salt_from_bytes(&cred.salt);
        let Ok(commitment) = commit_triples(&cred.graph, salt) else {
            return admitted; // fail closed
        };
        let commitment_fr: Fr = commitment.commitment;

        let Some(signature) = signature_from_hex(&cred.issuer_signature_hex) else {
            return admitted;
        };

        for r in rules {
            if !scope_covers(&r.scope, target) {
                continue;
            }
            let msg = commitment_message(&commitment_fr);
            if !verify(&r.issuer_key, &msg, &signature) {
                continue;
            }
            let age = session
                .now_unix_secs
                .saturating_sub(cred.issued_at_unix_secs);
            if age < 0 || age > r.fresh_within_secs {
                continue;
            }
            if cred.revoked {
                continue;
            }
            for t in &cred.graph {
                if is_reserved_triple(t) {
                    continue;
                }
                if !shape_admits_reference(&r.shape, t, &cred.graph) {
                    continue;
                }
                if !subject_is(t, &session.agent) {
                    continue;
                }
                let fact = AdmittedFact {
                    triple: t.clone(),
                    issuer: r.source.clone(),
                };
                if !admitted.contains(&fact) {
                    admitted.push(fact);
                }
            }
        }
        admitted
    }

    fn admit_static_reference(
        cred: &PresentedCredential,
        rules: &[TrustRule],
        target: &NamedNode,
    ) -> Vec<StaticAdmittedFact> {
        let mut admitted: Vec<StaticAdmittedFact> = Vec::new();

        let salt = salt_from_bytes(&cred.salt);
        let Ok(commitment) = commit_triples(&cred.graph, salt) else {
            return admitted; // fail closed
        };
        let commitment_fr: Fr = commitment.commitment;

        let Some(signature) = signature_from_hex(&cred.issuer_signature_hex) else {
            return admitted;
        };

        for r in rules {
            if !scope_covers(&r.scope, target) {
                continue;
            }
            let msg = commitment_message(&commitment_fr);
            if !verify(&r.issuer_key, &msg, &signature) {
                continue;
            }
            if cred.revoked {
                continue;
            }
            for t in &cred.graph {
                if is_reserved_triple(t) {
                    continue;
                }
                if !shape_admits_reference(&r.shape, t, &cred.graph) {
                    continue;
                }
                let NamedOrBlankNode::NamedNode(holder) = &t.subject else {
                    continue;
                };
                let fact = StaticAdmittedFact {
                    holder: holder.clone(),
                    not_after_unix_secs: cred
                        .issued_at_unix_secs
                        .saturating_add(r.fresh_within_secs),
                    triple: t.clone(),
                    issuer: r.source.clone(),
                };
                if !admitted.contains(&fact) {
                    admitted.push(fact);
                }
            }
        }
        admitted
    }

    fn check_admission_differential(graph: Vec<Triple>, seed: u64) {
        use sparq_zk::sig::SecretKey;
        let key = SecretKey::from_seed(seed);
        let salt = [7; 32];
        let commitment = commit_triples(&graph, salt_from_bytes(&salt)).unwrap();
        let mut credential = PresentedCredential {
            issuer_signature_hex: key.sign_commitment(&commitment.commitment),
            graph,
            salt,
            issued_at_unix_secs: 100,
            revoked: false,
        };
        let predicates: std::collections::BTreeSet<_> = credential
            .graph
            .iter()
            .map(|t| t.predicate.clone())
            .collect();
        let mut rules = Vec::new();
        for predicate in predicates {
            let root = iri("urn:shape");
            let property = iri("urn:property");
            let sh = "http://www.w3.org/ns/shacl#";
            let shape = ShapeRef {
                root: root.clone().into(),
                triples: vec![
                    Triple::new(
                        root.clone(),
                        iri(vocab::SH_TARGET_SUBJECTS_OF),
                        predicate.clone(),
                    ),
                    Triple::new(root, iri(&format!("{sh}property")), property.clone()),
                    Triple::new(property.clone(), iri(&format!("{sh}path")), predicate),
                    Triple::new(
                        property.clone(),
                        iri(&format!("{sh}minCount")),
                        oxrdf::Literal::from(1),
                    ),
                    Triple::new(
                        property,
                        iri(&format!("{sh}maxCount")),
                        oxrdf::Literal::from(2),
                    ),
                ],
            };
            rules.push(TrustRule {
                source: iri("urn:issuer"),
                issuer_key: key.public_key(),
                shape,
                scope: iri("https://pod.ex/"),
                fresh_within_secs: 50,
            });
        }
        // Duplicate rules and triples exercise result deduplication and output ordering.
        rules.extend(rules.clone());
        let mut subjects: Vec<NamedNode> = credential
            .graph
            .iter()
            .filter_map(|t| match &t.subject {
                NamedOrBlankNode::NamedNode(n) => Some(n.clone()),
                _ => None,
            })
            .collect();
        subjects.push(iri("urn:wrong-holder"));
        subjects.sort();
        subjects.dedup();
        for now in [99, 100, 150, 151] {
            for agent in &subjects {
                let session = Session {
                    agent: agent.clone(),
                    now_unix_secs: now,
                };
                let target = iri("https://pod.ex/doc");
                assert_eq!(
                    admit(&credential, &rules, &session, &target),
                    admit_reference(&credential, &rules, &session, &target),
                    "seed {seed}, now {now}"
                );
            }
        }
        let target = iri("https://pod.ex/doc");
        assert_eq!(
            admit_static(&credential, &rules, &target),
            admit_static_reference(&credential, &rules, &target),
            "static admission seed {seed}"
        );
        credential.revoked = true;
        assert_eq!(
            admit_static(&credential, &rules, &target),
            admit_static_reference(&credential, &rules, &target),
            "revoked static admission seed {seed}"
        );
        let session = Session {
            agent: subjects[0].clone(),
            now_unix_secs: 100,
        };
        let target = iri("https://pod.ex/doc");
        assert_eq!(
            admit(&credential, &rules, &session, &target),
            admit_reference(&credential, &rules, &session, &target)
        );
    }

    #[test]
    fn cached_shape_admission_matches_per_triple_validation_on_trust_fixtures() {
        let suite = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/trust-expression");
        let mut paths: Vec<_> = std::fs::read_dir(suite)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.is_dir())
            .collect();
        paths.sort();
        let mut count = 0;
        for path in paths {
            let fixture = path.join("holder.trig");
            if !fixture.exists() {
                continue;
            }
            let source = std::fs::read(fixture).unwrap();
            let graph = oxttl::TriGParser::new()
                .for_slice(&source)
                .map(|q| {
                    let q = q.unwrap();
                    Triple::new(q.subject, q.predicate, q.object)
                })
                .collect();
            check_admission_differential(graph, 31);
            count += 1;
        }
        assert!(count >= 8, "the fixture suite was exercised");
    }

    #[test]
    fn cached_shape_admission_matches_per_triple_validation_on_randomized_graphs() {
        let mut state = 0x7184_u64;
        for case in 0..32 {
            let mut graph = Vec::new();
            for _ in 0..20 {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                let subject = if state & 7 == 0 {
                    NamedOrBlankNode::BlankNode(oxrdf::BlankNode::new("b").unwrap())
                } else {
                    iri(&format!("urn:holder:{}", state % 4)).into()
                };
                let predicate = if state & 15 == 0 {
                    iri("http://www.w3.org/ns/auth/acl#agent")
                } else {
                    iri(&format!("urn:p:{}", (state >> 8) % 3))
                };
                graph.push(Triple::new(
                    subject,
                    predicate,
                    oxrdf::Literal::from(((state >> 16) % 5) as i64),
                ));
            }
            graph.push(graph[0].clone());
            check_admission_differential(graph, case + 1);
        }
    }

    #[test]
    fn scope_covers_exact_and_container() {
        let r = iri("https://pod.ex/docs/x");
        assert!(scope_covers(&r, &iri("https://pod.ex/docs/x")));
        assert!(!scope_covers(&r, &iri("https://pod.ex/docs/y")));
        let c = iri("https://pod.ex/docs/");
        assert!(scope_covers(&c, &iri("https://pod.ex/docs/x")));
        assert!(!scope_covers(&c, &iri("https://pod.ex/other/x")));
    }

    #[test]
    fn reserved_predicate_guard_blocks_internal_vocab() {
        assert!(is_reserved_predicate(&iri(
            "https://sparq.dev/ns/solidx#creator"
        )));
        assert!(is_reserved_predicate(&iri(
            "http://www.w3.org/ns/auth/acl#agent"
        )));
        assert!(is_reserved_predicate(&iri(vocab::ADMITTED)));
        assert!(!is_reserved_predicate(&iri("http://schema.org/age")));
        let typed = |class: &str| {
            oxrdf::Triple::new(
                iri("https://alice.ex/card#me"),
                iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type"),
                iri(class),
            )
        };
        assert!(is_reserved_triple(&typed("https://sparq.dev/ns/auth#ConditionalGrant")));
        assert!(is_reserved_triple(&typed("http://www.w3.org/ns/auth/acl#Authorization")));
        assert!(!is_reserved_triple(&typed("http://schema.org/Person")));
    }
}
