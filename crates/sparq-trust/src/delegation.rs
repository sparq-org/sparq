//! # `delegation.rs` — the invocation-binding gate (closes the §4 replay hole, `sq-l5og`)
//!
//! UCAN/ZCAP deliberately separate **delegation** (the signed chain document — *who
//! may act*) from **invocation** (exercising it on *this* request, key-bound to the
//! caller). The §4.1 "rides the admission rail" model captures the chain's
//! **existence** but **not** the per-request *invoker binding*. Without that binding an
//! admitted delegation becomes a **standing graph fact any session reaching it could
//! ride** — the exact replay / re-delegation escalation vector, the same severity class
//! as the §2.4 boundary re-opening.
//!
//! This module specifies and enforces the **missing rule** (design §4, `sq-l5og`,
//! the delegation analogue of §3.4 holder binding):
//!
//! > **authenticated invoker == the carried chain's terminal delegate, key-proven per
//! > request.**
//!
//! ## The invocation-binding gate (the algorithm — default-deny, short-circuit)
//!
//! ```text
//! invoke(chain, pop, session, target) -> EffectiveCapability | DENY:
//!   # 1. ANCHOR. The chain's root delegator must be a trust-anchored root key.
//!   if chain.root_key not in trusted_roots:                 DENY  # §4.1 anchor
//!   # 2. CHAIN INTEGRITY. Each hop is signed by its DELEGATOR's key over the hop
//!   #    message (delegator ‖ delegator_key ‖ delegate ‖ delegate_key ‖ capability ‖
//!   #    expiry), checked — never a self-asserted "I am delegated" triple (the
//!   #    §3.3-item-1 soundness condition). Binding delegate_key means the delegator
//!   #    ATTESTS which key the delegate holds — a substituted terminal key (the
//!   #    sq-l5og stolen-chain key-substitution vector) breaks this signature.
//!   for hop in chain.hops:
//!     if not verify_sig(hop.delegator_key, hop_message(hop), hop.sig):  DENY
//!   # 3. ATTENUATION (monotone narrowing, §4.2). Each hop may only RESTRICT: its
//!   #    capability ⊆ parent's, its expiry ≤ parent's. Never broadens the grant.
//!   for (parent, child) in adjacent(chain.hops):
//!     if not child.capability ⊆ parent.capability:          DENY  # §4.2
//!     if child.expiry > parent.expiry:                       DENY
//!   # 4. EXPIRY / FRESHNESS. The terminal hop must not be expired at Session.now
//!   #    (a per-request Rust check, like §3.3 B′ freshness — never in-reasoner).
//!   if session.now > terminal.expiry:                        DENY
//!   # 5. SCOPE. The terminal capability must cover the requested target.
//!   if not capability_covers(terminal.capability, target):  DENY
//!   # 6. INVOCATION BINDING (the core fix, §4.3-b). The authenticated invoker MUST
//!   #    BE the chain's terminal delegate AND prove possession of that key on THIS
//!   #    request via a fresh-challenge PoP (DPoP/GNAP-style). A captured chain alone
//!   #    cannot be ridden without the live terminal key + the live challenge.
//!   if terminal.delegate != session.agent:                   DENY  # invoker == delegate
//!   if not verify_pop(terminal.delegate_key, session.challenge, pop):  DENY  # key-proof
//!   emit EffectiveCapability{ capability: terminal.capability, delegate: terminal.delegate }
//! ```
//!
//! ## Why this defeats stolen-chain replay (and the residual it does NOT close)
//!
//! The replay vector (§4.1 / §7.4-K′) is: an attacker reads an admitted delegation (a
//! standing graph fact, or captures it off the wire) and "rides" it. Defeating it rests
//! on **three** legs working together — step 6's two PoP legs PLUS the key being bound
//! into the signed chain (step 2):
//!
//! - **invoker == terminal delegate** — the request must authenticate AS the terminal
//!   delegate's WebID, not merely *reach* the delegation graph. Anyone else is denied.
//! - **fresh-challenge key-proof (PoP)** — even a session authenticated as the terminal
//!   delegate must sign a **per-request, server-issued challenge** with the terminal
//!   key. This is the GNAP/DPoP proof-of-possession the design names in §4.3(b) —
//!   modelled here on the SAME challenge-response Schnorr PoP the holder binding uses
//!   (`sparq-zk::sig` `holder_pop_message` / `sign_holder_pop`), so no new crypto is
//!   invented.
//! - **delegate_key bound into the signed hop (step 2, the `sq-l5og` soundness fix)** —
//!   the terminal `delegate_key` is folded into [`hop_message`], so the DELEGATOR's
//!   signature attests exactly which key the delegate holds. WITHOUT this, the PoP leg
//!   alone was bypassable: an attacker could substitute its OWN public key as the
//!   terminal `delegate_key` (the genuine WebID + the genuine delegator signature still
//!   verified, because the old preimage excluded the key), claim the genuine delegate's
//!   WebID, and produce a valid PoP under its *own* key. Binding the key makes any such
//!   substitution break the delegator's signature ⇒ the chain is rejected at step 2.
//!
//! **Residual (do NOT overclaim full non-replayability).** With the key bound, a chain
//! captured off the wire cannot have its terminal key swapped, so a captured chain alone
//! no longer yields a usable PoP under an attacker key. BUT the property is only as sound
//! as the trust in the delegate's key in the first place: the delegate key is attested by
//! the delegator's signature, and the delegator's OWN key is still **operator-/chain-
//! asserted** — there is no DID resolver binding a WebID to a key yet (`sq-pfae.3`, the
//! live forgery vector D′). So this gate defeats stolen-chain **key-substitution** replay;
//! it does NOT close the upstream key-trust gap. An adversary who can mint a chain rooted
//! at a key the relying party (wrongly) anchors, or who controls the delegator's asserted
//! key, is out of scope here and tracked by `sq-pfae.3`.
//!
//! ## Object-capability discipline — resolves the §4.1↔§4.3 ambient-authority tension (item M)
//!
//! The design records a self-contradiction (§7.4 item M): §4.1 stores delegations as
//! **ambient graph facts**, while §4.3(a)'s obj-cap discipline says *carry the chain
//! WITH the invocation, no ambient lookup*. **This module takes the obj-cap side
//! explicitly:** the chain is presented [`carried with the invocation`](DelegationChain)
//! — [`invoke`] never looks a chain up from an ambient graph. Storing a delegation as a
//! graph fact (the §4.1 model) is then merely an OPTIONAL audit record; it is **never**
//! the authority source, because [`invoke`] gates on the *carried* chain + the live PoP,
//! not on graph presence. That is the design choice this PoC makes for `sq-l5og`
//! (documented in the PR + bead note; flagged to the maintainer to steer — issue).
//!
//! ## Intersection invariant binds the CURRENT delegator grant (item N)
//!
//! [`effective_against_current`] composes the chain's terminal capability with the
//! **current** delegator grant (re-supplied per request), per §4.2:
//! `effective = terminal_capability ∩ current_delegator_grant`. It NEVER binds a
//! delegation-time snapshot — a stale snapshot would let an AI agent retain escalated
//! authority after the delegator's grant is revoked (§7.4 item N). The caller MUST pass
//! the freshly re-materialised delegator grant; this is the per-request discipline
//! (`sq-xc4y`'s delegation analogue).
//!
//! ## Open-problem hooks this gate RESPECTS (never silently solved)
//!
//! - **`sq-l5og` (this bead)** — the invocation-binding gate is now SPECIFIED, enforced,
//!   and tested (the replay analogue of `acp_forged_*_in_acr_document_does_not_grant`).
//!   What stays open and is documented (not solved): deep-chain *incremental* revocation
//!   (full re-materialisation only, so the §4.4 / item-N stale-authority window is bounded
//!   by re-materialisation cadence, NOT closed here), and DID-resolver key binding
//!   (`sq-pfae.3`, the live forgery vector D′ — delegate keys are operator-/chain-asserted).
//! - **`sq-xc4y`** — the gate is a **per-request** decision: it takes the live
//!   [`Invocation`] (challenge + PoP + now); it must be re-run per request, never frozen
//!   into a session-independent materialise-once view.
//! - **No privacy / unlinkability.** Like the attribute gate, the invoker authenticates
//!   in the **clear** (`terminal.delegate == Session.agent`); this is NOT an anonymous
//!   or unlinkable invocation. The ZK estate is research-grade and externally UNAUDITED
//!   (`sq-qhy4`).
//!
//! ## Honest scope — RESEARCH PROTOTYPE, NOT a security guarantee
//!
//! This closes the *specification + enforcement + test* gap the design flagged as
//! `sq-l5og`. It is a research prototype to take to the LWS/Solid WGs, **not** a shipped
//! feature or a security guarantee. It does not provide a DID resolver, anonymity, deep
//! incremental revocation, or an external-audited crypto path.
//!
//! sq-l5og: delegation invocation-binding gate (issue #940, epic sq-pfae P5).
//! Written while Fable unavailable — flag for re-review when Fable returns.
//! 🤖 SPARQ agent — trust-graph delegation invocation binding.

use oxrdf::{NamedNode, Term};
use sparq_zk::encode::{encode_term, salt_from_bytes};
use sparq_zk::field::{field_from_hash_bytes, Fr};
use sparq_zk::poseidon2;
use sparq_zk::sig::{
    commitment_message, holder_key_digest, holder_pop_message, public_key_to_hex,
    signature_from_hex, verify, PublicKey,
};

/// A single delegation hop: *delegator → delegate*, granting (an attenuation of) a
/// capability, signed by the **delegator's** key over the hop message.
///
/// The signature is over [`hop_message`] — `Poseidon2`-bound over the structural fields
/// (delegator ‖ delegator_key ‖ delegate ‖ delegate_key ‖ capability ‖ expiry) — a
/// **checked** signature, never a self-asserted "I am delegated" triple (the §3.3-item-1
/// soundness condition applied to delegation). Binding `delegate_key` is the `sq-l5og`
/// soundness fix: it attests which key the delegate holds, so a substituted terminal key
/// breaks the delegator's signature.
#[derive(Debug, Clone)]
pub struct DelegationHop {
    /// The delegator's WebID/DID (the principal granting authority on this hop).
    pub delegator: NamedNode,
    /// The delegator's verification key — the hop signature must verify under it. For
    /// the ROOT hop this is the trust-anchored root key (`anchor` step). Operator-/
    /// chain-asserted by default (the live forgery vector D′); the root anchor is
    /// DID-bindable via the opt-in `did` feature (`sq-pfae.3`), narrowing D′.
    pub delegator_key: PublicKey,
    /// The delegate's WebID/DID (the principal receiving authority on this hop). For
    /// the *next* hop this becomes the delegator; for the TERMINAL hop this must equal
    /// the authenticated invoker (`Session.agent`) — the invocation-binding rule.
    pub delegate: NamedNode,
    /// The delegate's verification key — the **terminal** hop's `delegate_key` is the
    /// one the per-request PoP must verify under (the key-proof leg of step 6). It is
    /// folded into [`hop_message`], so the delegator's hop signature ATTESTS this key
    /// (the `sq-l5og` key-substitution fix): an attacker cannot swap in its own key on a
    /// captured chain without breaking the delegator's signature.
    pub delegate_key: PublicKey,
    /// The capability this hop grants (an attenuation of the parent's — see
    /// [`Capability`]). The chain must be monotone: `child ⊆ parent` (§4.2).
    pub capability: Capability,
    /// Hop expiry as a Unix timestamp (seconds). Must be `≤` the parent's expiry
    /// (monotone narrowing) and `≥ Session.now` at the terminal hop (not expired).
    pub expires_at_unix_secs: i64,
    /// The delegator's Schnorr signature over [`hop_message`], hex (`compressed(R) ‖ s`),
    /// exactly as `sparq_zk::sig` produces. An absent/forged signature fails closed.
    pub signature_hex: String,
}

/// A capability: a set of `auth:*` actions over a target resource/container prefix.
/// Attenuation (§4.2) is the partial order: `child ⊆ parent` iff `child.actions ⊆
/// parent.actions` AND `child.target` is `parent.target` or a descendant of it.
///
/// v1 models the capability as (action-set, target-prefix) — enough to express the
/// ZCAP-LD `allowedAction ⊆ parent` + suffix-narrowed-target attenuation and the
/// `auth:read|write|append|control` action lattice the shipped WAC/ACP view uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capability {
    /// The granted actions (e.g. `auth:read`, `auth:write`), as IRIs. A child's set
    /// must be a subset of the parent's.
    pub actions: Vec<NamedNode>,
    /// The target resource/container this capability is over. Container targets
    /// (slash-terminated) cover descendants; a child target must be the parent target
    /// or a descendant (suffix-narrowing only — never broadening).
    pub target: NamedNode,
}

impl Capability {
    /// Whether `self` is an attenuation of (`⊆`) `parent` (§4.2 monotone narrowing):
    /// every action in `self` is in `parent`, AND `self.target` is `parent.target` or a
    /// descendant of it (a child can only narrow the target, never broaden it).
    pub fn is_attenuation_of(&self, parent: &Capability) -> bool {
        let actions_subset = self
            .actions
            .iter()
            .all(|a| parent.actions.iter().any(|p| p.as_str() == a.as_str()));
        actions_subset && target_within(&self.target, &parent.target)
    }

    /// The intersection `self ∩ other` (§4.2 intersection rule): the actions present in
    /// BOTH, over the NARROWER of the two targets (the descendant). If neither target
    /// contains the other, the intersection is empty (`None`) — disjoint scopes grant
    /// nothing. This is how [`effective_against_current`] composes the chain capability
    /// with the *current* delegator grant.
    pub fn intersect(&self, other: &Capability) -> Option<Capability> {
        let actions: Vec<NamedNode> = self
            .actions
            .iter()
            .filter(|a| other.actions.iter().any(|o| o.as_str() == a.as_str()))
            .cloned()
            .collect();
        if actions.is_empty() {
            return None;
        }
        // The intersected target is the narrower (descendant) of the two; disjoint ⇒ None.
        let target = if target_within(&self.target, &other.target) {
            self.target.clone()
        } else if target_within(&other.target, &self.target) {
            other.target.clone()
        } else {
            return None;
        };
        Some(Capability { actions, target })
    }

    /// Whether this capability's target covers `target` (the §3.2 scope-containment
    /// check applied to a capability): exact-IRI or ancestor-container (slash-semantics).
    pub fn covers(&self, target: &NamedNode) -> bool {
        target_within(target, &self.target)
    }
}

/// `target_within(inner, outer)` — is `inner` exactly `outer`, or a descendant under
/// `outer`'s container prefix (Solid slash-semantics)? An empty/non-container `outer`
/// covers only the exact IRI.
fn target_within(inner: &NamedNode, outer: &NamedNode) -> bool {
    let i = inner.as_str();
    let o = outer.as_str();
    if i == o {
        return true;
    }
    o.ends_with('/') && i.starts_with(o)
}

/// A delegation chain carried **with** the invocation (object-capability discipline —
/// resolves the §4.1↔§4.3 ambient-authority tension, item M). Hops are ordered
/// root-first: `hops[0]` is the root delegation (signed by a trust-anchored root key),
/// `hops[last]` is the terminal hop whose delegate is the invoker.
#[derive(Debug, Clone)]
pub struct DelegationChain {
    /// The hops, root-first. A well-formed chain links: `hops[i].delegate ==
    /// hops[i+1].delegator` (checked in [`invoke`]).
    pub hops: Vec<DelegationHop>,
}

/// The live per-request invocation context — the values that make the binding
/// **per-request** (`sq-xc4y`): the authenticated invoker, the server-issued fresh
/// challenge, the invoker's PoP over it, and `now`.
#[derive(Debug, Clone)]
pub struct Invocation {
    /// The authenticated invoker's WebID (`Session.agent`). The invocation-binding rule
    /// requires this to equal the chain's **terminal delegate** (§4.3-b, the
    /// delegation analogue of §3.4 holder binding).
    pub agent: NamedNode,
    /// The server-issued **fresh** challenge nonce for THIS request (32 bytes). The
    /// invoker signs it; a captured chain cannot produce a PoP over a *new* nonce, so
    /// the binding is non-replayable. A relying layer MUST issue a fresh,
    /// single-use challenge per request (the DPoP/GNAP discipline; reuse would re-open
    /// the replay window — flagged, not enforced here, see crate docs).
    pub challenge: [u8; 32],
    /// The invoker's proof-of-possession over [`Invocation::challenge`], hex
    /// (`compressed(R) ‖ s`), produced by the terminal delegate's key via
    /// `sparq_zk::sig::SecretKey::sign_holder_pop`. An absent/forged PoP fails closed.
    pub pop_hex: String,
    /// The current instant as a Unix timestamp (seconds), checked against the terminal
    /// hop's expiry (a per-request Rust check, §3.3 B′; never in-reasoner).
    pub now_unix_secs: i64,
}

/// The capability that survived the invocation-binding gate, ready to compose with the
/// derivation stratum (its actions become `auth:*` grants over its target).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveCapability {
    /// The terminal capability the chain conferred (after attenuation checks).
    pub capability: Capability,
    /// The terminal delegate (== the authenticated invoker — the binding held).
    pub delegate: NamedNode,
}

/// Why an invocation was denied (fail-closed: a denied invocation confers NOTHING).
/// Each variant maps to one gate step, so a denial is auditable to its cause.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvocationDenied {
    /// The chain is empty — there is nothing to invoke.
    EmptyChain,
    /// The chain's root delegator key is not among the trust-anchored roots (step 1).
    UntrustedRoot,
    /// A hop's signature did not verify under its delegator's key (step 2) — a
    /// self-asserted / forged delegation.
    HopSignatureInvalid { hop: usize },
    /// The chain is not properly linked: `hops[i].delegate != hops[i+1].delegator`.
    BrokenLink { hop: usize },
    /// A child hop broadened the capability or expiry beyond its parent (step 3) — a
    /// non-monotone (escalating) attenuation.
    NonMonotoneAttenuation { hop: usize },
    /// The terminal hop is expired at `Invocation.now` (step 4).
    Expired,
    /// The terminal capability does not cover the requested target (step 5).
    OutOfScope,
    /// The authenticated invoker is NOT the chain's terminal delegate (step 6, leg 1) —
    /// the core replay defence: a session that merely *reached* the delegation cannot
    /// ride it.
    InvokerNotTerminalDelegate,
    /// The per-request proof-of-possession did not verify under the terminal delegate's
    /// key (step 6, leg 2) — the key-proof defence: a captured chain carries no key, so
    /// it cannot sign a fresh challenge.
    KeyProofInvalid,
}

impl std::fmt::Display for InvocationDenied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InvocationDenied::EmptyChain => write!(f, "delegation chain is empty"),
            InvocationDenied::UntrustedRoot => {
                write!(f, "chain root key is not a trust-anchored root (§4.1 anchor)")
            }
            InvocationDenied::HopSignatureInvalid { hop } => {
                write!(f, "hop {hop} signature did not verify under its delegator key")
            }
            InvocationDenied::BrokenLink { hop } => {
                write!(f, "hop {hop} delegate != hop {} delegator (broken chain)", hop + 1)
            }
            InvocationDenied::NonMonotoneAttenuation { hop } => write!(
                f,
                "hop {hop} broadened capability/expiry beyond its parent (non-monotone §4.2)"
            ),
            InvocationDenied::Expired => write!(f, "terminal hop is expired at Invocation.now"),
            InvocationDenied::OutOfScope => {
                write!(f, "terminal capability does not cover the requested target")
            }
            InvocationDenied::InvokerNotTerminalDelegate => write!(
                f,
                "authenticated invoker != chain terminal delegate (replay/re-delegation §4.3-b)"
            ),
            InvocationDenied::KeyProofInvalid => write!(
                f,
                "per-request proof-of-possession failed under the terminal delegate key (replay §4.3-b)"
            ),
        }
    }
}

impl std::error::Error for InvocationDenied {}

/// Run the **invocation-binding gate** over a carried delegation chain and a live
/// invocation. Returns the surviving [`EffectiveCapability`] or the [`InvocationDenied`]
/// cause (fail-closed). NEVER panics on adversarial input.
///
/// - `chain` is carried WITH the invocation (obj-cap discipline — no ambient lookup,
///   item M).
/// - `trusted_roots` are the trust-anchored root keys (the delegation analogue of the
///   trust-policy issuer keys; operator-asserted, `sq-pfae.3`).
/// - `inv` is the live per-request context (invoker + fresh challenge + PoP + now).
/// - `target` is the resource the invocation is for (the scope check).
///
/// The terminal hop's `delegate` must equal `inv.agent` AND the terminal hop's
/// `delegate_key` must verify the PoP over `inv.challenge` — the two-leg replay defence
/// (`sq-l5og`).
pub fn invoke(
    chain: &DelegationChain,
    trusted_roots: &[PublicKey],
    inv: &Invocation,
    target: &NamedNode,
) -> Result<EffectiveCapability, InvocationDenied> {
    // Step 0: a chain must have at least one hop.
    let Some(root) = chain.hops.first() else {
        return Err(InvocationDenied::EmptyChain);
    };
    let terminal = chain.hops.last().expect("non-empty checked above");

    // Step 1: ANCHOR — the root delegator key must be a trust-anchored root.
    if !trusted_roots
        .iter()
        .any(|k| public_key_to_hex(k) == public_key_to_hex(&root.delegator_key))
    {
        return Err(InvocationDenied::UntrustedRoot);
    }

    // Step 2 + chain-linkage + step 3 attenuation, walked hop by hop.
    for (i, hop) in chain.hops.iter().enumerate() {
        // Step 2: CHECKED hop signature under the DELEGATOR's key over the hop message.
        // The delegator signs the hop message via `SecretKey::sign_commitment` (which
        // wraps it in the domain-separated `commitment_message`), so the gate verifies
        // against the same wrapped message — never a self-asserted "I am delegated"
        // triple (the §3.3-item-1 soundness condition applied to delegation).
        let msg = commitment_message(&hop_message(hop));
        let Some(sig) = signature_from_hex(&hop.signature_hex) else {
            return Err(InvocationDenied::HopSignatureInvalid { hop: i });
        };
        if !verify(&hop.delegator_key, &msg, &sig) {
            return Err(InvocationDenied::HopSignatureInvalid { hop: i });
        }

        // Chain linkage: hops[i].delegate must equal hops[i+1].delegator (and their
        // keys must match — the delegate of one hop signs the next as delegator).
        if let Some(next) = chain.hops.get(i + 1) {
            if hop.delegate.as_str() != next.delegator.as_str()
                || public_key_to_hex(&hop.delegate_key) != public_key_to_hex(&next.delegator_key)
            {
                return Err(InvocationDenied::BrokenLink { hop: i });
            }
            // Step 3: ATTENUATION — child ⊆ parent (capability AND expiry).
            if !next.capability.is_attenuation_of(&hop.capability)
                || next.expires_at_unix_secs > hop.expires_at_unix_secs
            {
                return Err(InvocationDenied::NonMonotoneAttenuation { hop: i + 1 });
            }
        }
    }

    // Step 4: EXPIRY — the terminal hop must not be expired at `now` (per-request).
    if inv.now_unix_secs > terminal.expires_at_unix_secs {
        return Err(InvocationDenied::Expired);
    }

    // Step 5: SCOPE — the terminal capability must cover the requested target.
    if !terminal.capability.covers(target) {
        return Err(InvocationDenied::OutOfScope);
    }

    // Step 6, leg 1: INVOKER == TERMINAL DELEGATE. A session that merely *reached* the
    // delegation graph (the replay vector) is denied here — it is not the delegate.
    if terminal.delegate.as_str() != inv.agent.as_str() {
        return Err(InvocationDenied::InvokerNotTerminalDelegate);
    }

    // Step 6, leg 2: KEY-PROOF — the invoker proves possession of the terminal key on
    // THIS request by signing the fresh challenge (DPoP/GNAP PoP). The terminal
    // `delegate_key` this verifies under is itself ATTESTED by the delegator's hop
    // signature (step 2 binds it via `hop_message`), so an attacker cannot substitute its
    // own key on a captured chain and then PoP under it — the sq-l5og soundness fix.
    let challenge_fr: Fr = field_from_hash_bytes(&inv.challenge);
    let pop_msg = holder_pop_message(&challenge_fr);
    let Some(pop_sig) = signature_from_hex(&inv.pop_hex) else {
        return Err(InvocationDenied::KeyProofInvalid);
    };
    if !verify(&terminal.delegate_key, &pop_msg, &pop_sig) {
        return Err(InvocationDenied::KeyProofInvalid);
    }

    Ok(EffectiveCapability {
        capability: terminal.capability.clone(),
        delegate: terminal.delegate.clone(),
    })
}

/// Compose a gate-surviving [`EffectiveCapability`] with the **CURRENT** delegator
/// grant (re-supplied per request) — the §4.2 intersection rule, item N:
/// `effective = terminal_capability ∩ current_delegator_grant`.
///
/// `current_delegator_grant` MUST be the freshly re-materialised delegator capability,
/// **never** a delegation-time snapshot — a stale snapshot lets an AI agent retain
/// escalated authority after the delegator's grant is revoked (§7.4 item N). Returns
/// `None` (no effective authority) if the intersection is empty (e.g. the delegator's
/// grant has been narrowed/revoked below the delegated capability).
pub fn effective_against_current(
    effective: &EffectiveCapability,
    current_delegator_grant: &Capability,
) -> Option<EffectiveCapability> {
    effective
        .capability
        .intersect(current_delegator_grant)
        .map(|capability| EffectiveCapability {
            capability,
            delegate: effective.delegate.clone(),
        })
}

/// The hop message a delegator signs (and the gate verifies): a domain-separated
/// `Poseidon2` hash over the hop's structural fields
/// (`delegator ‖ delegator_key ‖ delegate ‖ delegate_key ‖ capability ‖ expiry`).
/// Binding ALL of these means a forger cannot lift a signature from one hop onto a
/// different delegate / KEY / capability / expiry — the hop signature attests the WHOLE
/// hop, not just its existence.
///
/// ## Why the keys MUST be in the signed preimage (`sq-l5og` soundness fix)
///
/// Binding `delegate_key` (and `delegator_key`) is **load-bearing**, not cosmetic. If the
/// delegate's key were excluded, the TERMINAL hop's `delegate_key` would be attested by NO
/// signature: the linkage check only constrains the keys of NON-terminal hops (`hops[i]`'s
/// delegate-key == `hops[i+1]`'s delegator-key), and the per-request PoP merely verifies a
/// signature *against* whatever `delegate_key` is presented. An attacker could then capture
/// the chain off the wire, substitute its OWN public key as the terminal `delegate_key`
/// (the genuine WebID + the genuine delegator signature would still verify, because they
/// would not cover the key), set `inv.agent` to the genuine delegate's WebID, and sign the
/// fresh challenge with its own key — riding the stolen chain. Folding `delegate_key` into
/// `hop_message` closes this: substituting the terminal key now breaks the *delegator's*
/// signature over the hop, so the chain is rejected at step 2 (`HopSignatureInvalid`).
/// Every hop (including the terminal) binds its key, so the property holds chain-wide.
///
/// This is the delegation analogue of `sparq_zk::sig::commitment_message`: a
/// domain-separated message so a delegation signature can never be confused with a
/// credential-commitment signature or a holder-PoP signature (distinct domain tags).
///
/// The delegator signs this via `SecretKey::sign_commitment` (which wraps it in
/// `commitment_message` before signing), and [`invoke`] verifies against the same
/// wrapped form — so the value returned here is the *pre-wrap* hop message.
///
/// ## Wire-format / back-compat note
///
/// Folding the keys CHANGES the signed preimage: a `signature_hex` produced over the
/// pre-fix message (keys excluded) will NOT verify under this message, and vice versa.
/// This is intentional — the pre-fix preimage was forgeable (see above). There is no
/// shipped on-wire delegation format yet (this is a research prototype, `sq-l5og`), so no
/// migration is required; any future serialisation MUST sign over THIS preimage.
pub fn hop_message(hop: &DelegationHop) -> Fr {
    // Domain tag "TRUSTDLG" so a hop signature is never confused with a commitment or
    // PoP signature (cross-protocol message-confusion defence).
    const SIG_DOMAIN_DELEGATION_HOP: u64 = 0x5452_5553_5444_4c47; // "TRUSTDLG"
                                                                  // Hash the structural fields into field elements. IRIs/actions go through the SAME
                                                                  // domain-separated `encode_term` embedding the rest of the estate uses (salt is
                                                                  // irrelevant for IRIs — they are salt-independent in `encode_term`). The keys are
                                                                  // folded as their domain-separated coordinate digests so the delegator's signature
                                                                  // ATTESTS exactly which keys this hop is for (the sq-l5og key-substitution fix).
    let mut inputs: Vec<Fr> = vec![
        Fr::from(SIG_DOMAIN_DELEGATION_HOP),
        iri_to_field(&hop.delegator),
        key_to_field(&hop.delegator_key),
        iri_to_field(&hop.delegate),
        key_to_field(&hop.delegate_key),
        iri_to_field(&hop.capability.target),
        Fr::from(hop.expires_at_unix_secs.unsigned_abs()),
        // Sign over the expiry SIGN too, so a negative expiry can't be lifted to positive.
        Fr::from(u64::from(hop.expires_at_unix_secs < 0)),
        Fr::from(hop.capability.actions.len() as u64),
    ];
    // Actions are signed in their listed order; a reorder yields a different message,
    // which is fine — the honest signer re-serialises the same hop identically.
    for a in &hop.capability.actions {
        inputs.push(iri_to_field(a));
    }
    poseidon2::hash(&inputs)
}

/// Fold a delegation key into the signed hop preimage as its domain-separated coordinate
/// digest, so the delegator's hop signature ATTESTS the exact key. Uses the estate's
/// `sparq_zk::sig::holder_key_digest` (`Poseidon2([ZKSIG_HK, x, y])`) — no new hashing
/// primitive is introduced. The curve identity has no affine coordinates and is never a
/// usable signing key (`verify`/`sign_holder_pop` reject it independently); rather than
/// silently folding `(0, 0)`, it is bound here as a FIXED sentinel field value so an
/// identity-key hop is still deterministically signed/verified, and the Schnorr layer's
/// own identity rejection keeps the path fail-closed.
fn key_to_field(pk: &PublicKey) -> Fr {
    // Distinct from any real `holder_key_digest` output (that folds `ZKSIG_HK ‖ x ‖ y`
    // over genuine coordinates), so an identity key can never collide with a real one.
    const IDENTITY_KEY_SENTINEL: u64 = 0x5452_5553_544b_4944; // "TRUSTKID"
    holder_key_digest(pk).unwrap_or_else(|_| Fr::from(IDENTITY_KEY_SENTINEL))
}

/// Embed an IRI as a field element via the estate's domain-separated `encode_term`
/// (IRIs are salt-independent there, so the constant salt is immaterial). Stable and
/// collision-resistant for distinct IRIs at the field's security level — no new hashing
/// primitive is introduced.
fn iri_to_field(n: &NamedNode) -> Fr {
    // IRI encoding is salt-independent in `encode_term`; a fixed salt is fine.
    let salt = salt_from_bytes(&[0u8; 32]);
    encode_term(&Term::NamedNode(n.clone()), &salt)
        .expect("a NamedNode always encodes (only RDF-1.2 triple terms return None)")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iri(s: &str) -> NamedNode {
        NamedNode::new(s).unwrap()
    }

    fn read() -> NamedNode {
        iri("https://sparq.dev/ns/auth#read")
    }
    fn write() -> NamedNode {
        iri("https://sparq.dev/ns/auth#write")
    }

    fn cap(actions: Vec<NamedNode>, target: &str) -> Capability {
        Capability {
            actions,
            target: iri(target),
        }
    }

    #[test]
    fn attenuation_is_monotone() {
        let parent = cap(vec![read(), write()], "https://pod.ex/docs/");
        let child = cap(vec![read()], "https://pod.ex/docs/x");
        assert!(
            child.is_attenuation_of(&parent),
            "narrower actions + target ⊆ parent"
        );
        // Broadening actions is NOT attenuation.
        let escalate = cap(vec![read(), write()], "https://pod.ex/docs/x");
        assert!(
            escalate.is_attenuation_of(&parent),
            "same actions, narrower target ⊆"
        );
        let broaden_target = cap(vec![read()], "https://pod.ex/");
        assert!(
            !broaden_target.is_attenuation_of(&parent),
            "broadening the target is NOT attenuation"
        );
        let broaden_action = cap(
            vec![read(), iri("https://sparq.dev/ns/auth#control")],
            "https://pod.ex/docs/x",
        );
        assert!(
            !broaden_action.is_attenuation_of(&parent),
            "an action not in the parent is NOT attenuation"
        );
    }

    #[test]
    fn intersection_takes_narrower_target_and_common_actions() {
        let a = cap(vec![read(), write()], "https://pod.ex/docs/");
        let b = cap(vec![read()], "https://pod.ex/docs/x");
        let i = a.intersect(&b).expect("non-empty intersection");
        assert_eq!(i.actions, vec![read()], "only the common action survives");
        assert_eq!(
            i.target.as_str(),
            "https://pod.ex/docs/x",
            "narrower target"
        );
        // Disjoint actions ⇒ no intersection.
        let c = cap(
            vec![iri("https://sparq.dev/ns/auth#control")],
            "https://pod.ex/docs/x",
        );
        assert!(b.intersect(&c).is_none(), "disjoint actions ⇒ empty");
        // Disjoint targets ⇒ no intersection.
        let d = cap(vec![read()], "https://pod.ex/other/");
        let e = cap(vec![read()], "https://pod.ex/docs/x");
        assert!(d.intersect(&e).is_none(), "disjoint targets ⇒ empty");
    }
}
