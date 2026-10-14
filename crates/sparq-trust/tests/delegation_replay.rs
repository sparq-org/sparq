//! End-to-end delegation invocation-binding tests (design §4, `sq-l5og`), plus the
//! adversarial **delegation-replay** forgery matrix — the analogue of
//! `acp_forged_*_in_acr_document_does_not_grant`.
//!
//! These convert the §7.4-K′ invocation-binding rule from *design intent* into a
//! *verified property*: an admitted delegation is NOT replayable because the gate binds
//! **authenticated invoker == the chain's terminal delegate, key-proven per request**.
//!
//! Positive: a root→agent chain, fresh, monotone, with the agent presenting a live PoP
//! over a fresh challenge ⇒ the effective capability is conferred. Negatives (each MUST
//! deny): an untrusted root; a forged/tampered hop signature; a broken chain link; a
//! non-monotone (escalating) attenuation; an expired terminal hop; an out-of-scope
//! target; the **replay** case (a third party who captured the chain but is not the
//! terminal delegate); the **stolen-chain** case (the right delegate WebID but no live
//! key / a stale PoP); and the intersection-vs-current-grant case (item N).
//!
//! sq-l5og: delegation invocation-binding gate (issue #940, epic sq-pfae P5).
//! 🤖 SPARQ agent — trust-graph delegation invocation binding.

use oxrdf::NamedNode;
use sparq_trust::delegation::{
    effective_against_current, hop_message, invoke, Capability, DelegationChain, DelegationHop,
    Invocation, InvocationDenied,
};
use sparq_zk::field::{field_from_hash_bytes, Fr};
use sparq_zk::sig::{holder_pop_message, PublicKey, SecretKey};

// --- fixtures ---------------------------------------------------------------

const ROOT: &str = "https://gov.example/root";
const ALICE: &str = "https://alice.ex/card#me"; // intermediate delegate
const AGENT: &str = "https://agent.ex/card#me"; // terminal delegate (the invoker)
const MALLORY: &str = "https://mallory.ex/card#me"; // attacker
const DOCS: &str = "https://pod.ex/docs/";
const DOC_X: &str = "https://pod.ex/docs/x";
const OTHER: &str = "https://pod.ex/other/y";

const NOW: i64 = 1_700_000_000;
const FAR_FUTURE: i64 = NOW + 365 * 86_400;
const SOON: i64 = NOW + 7 * 86_400;
const CHALLENGE: [u8; 32] = [0x42u8; 32];

fn iri(s: &str) -> NamedNode {
    NamedNode::new(s).unwrap()
}
fn read() -> NamedNode {
    iri("https://sparq.dev/ns/auth#read")
}
fn write() -> NamedNode {
    iri("https://sparq.dev/ns/auth#write")
}
fn control() -> NamedNode {
    iri("https://sparq.dev/ns/auth#control")
}

fn cap(actions: Vec<NamedNode>, target: &str) -> Capability {
    Capability {
        actions,
        target: iri(target),
    }
}

/// Mint a key from a seed (deterministic for the test).
fn key(seed: u64) -> (SecretKey, PublicKey) {
    let sk = SecretKey::from_seed(seed);
    let pk = sk.public_key();
    (sk, pk)
}

/// Build a hop and SIGN it with the delegator's key over the genuine hop message.
#[allow(clippy::too_many_arguments)]
fn signed_hop(
    delegator: &str,
    delegator_sk: &SecretKey,
    delegator_pk: &PublicKey,
    delegate: &str,
    delegate_pk: &PublicKey,
    capability: Capability,
    expires_at_unix_secs: i64,
) -> DelegationHop {
    let mut hop = DelegationHop {
        delegator: iri(delegator),
        delegator_key: *delegator_pk,
        delegate: iri(delegate),
        delegate_key: *delegate_pk,
        capability,
        expires_at_unix_secs,
        signature_hex: String::new(),
    };
    let msg = hop_message(&hop);
    hop.signature_hex = delegator_sk.sign_commitment(&msg);
    hop
}

/// The canonical two-hop chain: root → alice (read+write over /docs/) → agent (read
/// over /docs/x). Returns (chain, trusted_roots, agent_sk).
fn canonical_chain() -> (DelegationChain, Vec<PublicKey>, SecretKey) {
    let (root_sk, root_pk) = key(0x1001);
    let (_alice_sk, alice_pk) = key(0x1002);
    let (agent_sk, agent_pk) = key(0x1003);

    // Hop 0: root → alice, read+write over /docs/, far-future expiry.
    let hop0 = signed_hop(
        ROOT,
        &root_sk,
        &root_pk,
        ALICE,
        &alice_pk,
        cap(vec![read(), write()], DOCS),
        FAR_FUTURE,
    );
    // Hop 1: alice → agent, read over /docs/x (narrowed action + target), earlier expiry.
    let alice_sk = key(0x1002).0;
    let hop1 = signed_hop(
        ALICE,
        &alice_sk,
        &alice_pk,
        AGENT,
        &agent_pk,
        cap(vec![read()], DOC_X),
        SOON,
    );

    let chain = DelegationChain {
        hops: vec![hop0, hop1],
    };
    (chain, vec![root_pk], agent_sk)
}

/// The agent's live PoP over a challenge (the DPoP/GNAP key-proof leg).
fn pop_over(agent_sk: &SecretKey, challenge: &[u8; 32]) -> String {
    let challenge_fr: Fr = field_from_hash_bytes(challenge);
    agent_sk.sign_holder_pop(&challenge_fr)
}

fn invocation(agent: &str, agent_sk: &SecretKey, challenge: [u8; 32], now: i64) -> Invocation {
    Invocation {
        agent: iri(agent),
        challenge,
        pop_hex: pop_over(agent_sk, &challenge),
        now_unix_secs: now,
    }
}

// --- the positive case ------------------------------------------------------

#[test]
fn canonical_chain_with_live_pop_confers_the_capability() {
    let (chain, roots, agent_sk) = canonical_chain();
    let inv = invocation(AGENT, &agent_sk, CHALLENGE, NOW);
    let eff = invoke(&chain, &roots, &inv, &iri(DOC_X)).expect("invocation binds");
    assert_eq!(
        eff.delegate.as_str(),
        AGENT,
        "the terminal delegate is the invoker"
    );
    assert_eq!(
        eff.capability.actions,
        vec![read()],
        "the attenuated action survives"
    );
    assert_eq!(eff.capability.target.as_str(), DOC_X);
}

// --- the adversarial negative matrix (each MUST deny) -----------------------

#[test]
fn replay_by_non_delegate_is_denied() {
    // THE CORE REPLAY VECTOR (sq-l5og): Mallory captured the chain (a standing graph
    // fact) and tries to ride it. She authenticates as her OWN WebID, not the terminal
    // delegate, so the invoker==terminal-delegate leg denies — even if she could somehow
    // produce a PoP. This is the delegation analogue of the third-party-credential test.
    let (chain, roots, _agent_sk) = canonical_chain();
    let (mallory_sk, _mallory_pk) = key(0xBEEF);
    // Mallory signs the challenge with HER key, but she is not the terminal delegate.
    let inv = Invocation {
        agent: iri(MALLORY),
        challenge: CHALLENGE,
        pop_hex: pop_over(&mallory_sk, &CHALLENGE),
        now_unix_secs: NOW,
    };
    let err = invoke(&chain, &roots, &inv, &iri(DOC_X)).unwrap_err();
    assert_eq!(
        err,
        InvocationDenied::InvokerNotTerminalDelegate,
        "a captured chain cannot be ridden by a session that is not the terminal delegate"
    );
}

#[test]
fn stolen_chain_without_the_terminal_key_is_denied() {
    // Mallory steals the chain AND spoofs the agent's WebID (e.g. a compromised
    // session-IRI claim), but does NOT hold the terminal delegate's signing key. She
    // cannot produce a valid PoP over the fresh challenge ⇒ the key-proof leg denies.
    // This is the leg that defeats replay even when the invoker identity is spoofed.
    let (chain, roots, _agent_sk) = canonical_chain();
    let (mallory_sk, _mallory_pk) = key(0xBEEF);
    let inv = Invocation {
        agent: iri(AGENT), // spoofs the agent's WebID
        challenge: CHALLENGE,
        pop_hex: pop_over(&mallory_sk, &CHALLENGE), // but signs with the WRONG key
        now_unix_secs: NOW,
    };
    let err = invoke(&chain, &roots, &inv, &iri(DOC_X)).unwrap_err();
    assert_eq!(
        err,
        InvocationDenied::KeyProofInvalid,
        "without the terminal key, no valid PoP over the fresh challenge ⇒ denied"
    );
}

#[test]
fn stolen_chain_with_substituted_terminal_key_is_denied() {
    // THE KEY-SUBSTITUTION REPLAY (sq-l5og soundness fix). This is the exploit the
    // adversarial review proved: stronger than `stolen_chain_without_the_terminal_key_*`
    // (which covers the wrong-key-IN-the-same-slot case). Here Mallory does NOT merely
    // sign with the wrong key against the genuine terminal key — she SUBSTITUTES her own
    // public key into the terminal hop's `delegate_key` slot, keeps the GENUINE delegator
    // signature untouched, claims the genuine delegate's WebID, and signs the fresh
    // challenge with HER key.
    //
    // Pre-fix, this CONFERRED the capability: `hop_message` excluded `delegate_key`, so
    // the genuine delegator signature still verified over the tampered hop, and the PoP
    // verified under Mallory's substituted key. Post-fix, `delegate_key` is folded into
    // `hop_message`, so swapping the terminal key changes the signed preimage ⇒ the
    // delegator's signature over the terminal hop no longer verifies ⇒ DENIED at step 2.
    let (mut chain, roots, _agent_sk) = canonical_chain();
    let (mallory_sk, mallory_pk) = key(0xBEEF);

    // Capture-and-substitute: replace the TERMINAL hop's delegate_key with Mallory's,
    // WITHOUT re-signing the hop (the genuine delegator signature stays as-is — Mallory
    // does not hold alice's key, so she cannot re-sign).
    let terminal_idx = chain.hops.len() - 1;
    chain.hops[terminal_idx].delegate_key = mallory_pk;

    // She claims the genuine delegate's WebID and PoPs with her OWN key (which now sits in
    // the terminal delegate_key slot, so leg-2 alone would have passed pre-fix).
    let inv = Invocation {
        agent: iri(AGENT),
        challenge: CHALLENGE,
        pop_hex: pop_over(&mallory_sk, &CHALLENGE),
        now_unix_secs: NOW,
    };

    let err = invoke(&chain, &roots, &inv, &iri(DOC_X)).unwrap_err();
    assert_eq!(
        err,
        InvocationDenied::HopSignatureInvalid { hop: terminal_idx },
        "substituting the terminal delegate_key must break the delegator's hop signature \
         (sq-l5og: the key is bound into the signed preimage), denying the stolen chain"
    );

    // Belt-and-braces: the gate must NOT have admitted anything — a denial confers nothing.
    assert!(
        invoke(&chain, &roots, &inv, &iri(DOC_X)).is_err(),
        "the key-substituted chain is rejected, fail-closed"
    );
}

#[test]
fn substituted_terminal_key_on_a_single_hop_chain_is_denied() {
    // Same key-substitution attack on the degenerate one-hop (root→agent) chain, to prove
    // the binding holds for the TERMINAL hop even when it is also the ROOT hop (the
    // linkage check constrains zero key pairs here, so the binding is the ONLY defence).
    let (root_sk, root_pk) = key(0x1001);
    let (_agent_sk, agent_pk) = key(0x1003);
    let mut hop0 = signed_hop(
        ROOT,
        &root_sk,
        &root_pk,
        AGENT,
        &agent_pk,
        cap(vec![read()], DOC_X),
        FAR_FUTURE,
    );
    // Substitute Mallory's key as the (terminal == root) delegate_key, keep the genuine
    // root signature.
    let (mallory_sk, mallory_pk) = key(0xBEEF);
    hop0.delegate_key = mallory_pk;
    let chain = DelegationChain { hops: vec![hop0] };

    let inv = Invocation {
        agent: iri(AGENT),
        challenge: CHALLENGE,
        pop_hex: pop_over(&mallory_sk, &CHALLENGE),
        now_unix_secs: NOW,
    };
    let err = invoke(&chain, &[root_pk], &inv, &iri(DOC_X)).unwrap_err();
    assert_eq!(
        err,
        InvocationDenied::HopSignatureInvalid { hop: 0 },
        "on a single-hop chain the terminal key binding is the sole defence and must hold"
    );
}

#[test]
fn replayed_pop_over_a_different_challenge_is_denied() {
    // The agent's PoP from an EARLIER request (a different challenge) is replayed against
    // a NEW challenge. The PoP is bound to the challenge it signed, so it does not verify
    // against the new one ⇒ denied. This is why the challenge must be fresh per request.
    let (chain, roots, agent_sk) = canonical_chain();
    let old_challenge = [0x11u8; 32];
    let new_challenge = [0x22u8; 32];
    let inv = Invocation {
        agent: iri(AGENT),
        challenge: new_challenge,
        pop_hex: pop_over(&agent_sk, &old_challenge), // PoP for the OLD challenge
        now_unix_secs: NOW,
    };
    let err = invoke(&chain, &roots, &inv, &iri(DOC_X)).unwrap_err();
    assert_eq!(
        err,
        InvocationDenied::KeyProofInvalid,
        "a PoP captured over an old challenge does not verify against a fresh one (non-replayable)"
    );
}

#[test]
fn untrusted_root_is_denied() {
    // The chain roots at a key the relying party does not anchor ⇒ denied at step 1.
    let (chain, _roots, agent_sk) = canonical_chain();
    let (_other_sk, other_root_pk) = key(0x9999);
    let inv = invocation(AGENT, &agent_sk, CHALLENGE, NOW);
    let err = invoke(&chain, &[other_root_pk], &inv, &iri(DOC_X)).unwrap_err();
    assert_eq!(err, InvocationDenied::UntrustedRoot);
}

#[test]
fn forged_hop_signature_is_denied() {
    // Tamper the root hop's signature ⇒ the checked hop-signature step denies (a
    // self-asserted / forged delegation proves nothing — §3.3 item 1 for delegation).
    let (mut chain, roots, agent_sk) = canonical_chain();
    let mut sig: Vec<char> = chain.hops[0].signature_hex.chars().collect();
    let last = sig.len() - 1;
    sig[last] = if sig[last] == '0' { '1' } else { '0' };
    chain.hops[0].signature_hex = sig.into_iter().collect();
    let inv = invocation(AGENT, &agent_sk, CHALLENGE, NOW);
    let err = invoke(&chain, &roots, &inv, &iri(DOC_X)).unwrap_err();
    assert_eq!(err, InvocationDenied::HopSignatureInvalid { hop: 0 });
}

#[test]
fn lifted_signature_onto_a_broadened_capability_is_denied() {
    // An attacker lifts the root's genuine signature but swaps the hop's capability to a
    // broader one. Because the hop signature is over the WHOLE hop (delegator ‖ delegate
    // ‖ capability ‖ expiry — hop_message), the signature no longer verifies ⇒ denied.
    let (mut chain, roots, agent_sk) = canonical_chain();
    // Keep the genuine signature but broaden hop0's capability to add control.
    chain.hops[0].capability = cap(vec![read(), write(), control()], DOCS);
    let inv = invocation(AGENT, &agent_sk, CHALLENGE, NOW);
    let err = invoke(&chain, &roots, &inv, &iri(DOC_X)).unwrap_err();
    assert_eq!(
        err,
        InvocationDenied::HopSignatureInvalid { hop: 0 },
        "the hop signature binds the capability, so a lifted-onto-broader-cap forgery fails"
    );
}

#[test]
fn non_monotone_escalating_attenuation_is_denied() {
    // The child hop GENUINELY signs a broader capability than its parent (alice grants
    // agent `control` she was never delegated). Even with valid signatures, the
    // attenuation step denies — a child can only restrict.
    let (root_sk, root_pk) = key(0x1001);
    let (alice_sk, alice_pk) = key(0x1002);
    let (agent_sk, agent_pk) = key(0x1003);
    let hop0 = signed_hop(
        ROOT,
        &root_sk,
        &root_pk,
        ALICE,
        &alice_pk,
        cap(vec![read()], DOCS), // alice only got read
        FAR_FUTURE,
    );
    let hop1 = signed_hop(
        ALICE,
        &alice_sk,
        &alice_pk,
        AGENT,
        &agent_pk,
        cap(vec![read(), control()], DOC_X), // but grants agent read+control (escalation)
        SOON,
    );
    let chain = DelegationChain {
        hops: vec![hop0, hop1],
    };
    let inv = invocation(AGENT, &agent_sk, CHALLENGE, NOW);
    let err = invoke(&chain, &[root_pk], &inv, &iri(DOC_X)).unwrap_err();
    assert_eq!(
        err,
        InvocationDenied::NonMonotoneAttenuation { hop: 1 },
        "a child cannot grant authority the parent never held (§4.2 monotone narrowing)"
    );
}

#[test]
fn broadened_expiry_in_child_is_denied() {
    // The child hop tries to live LONGER than the parent (expiry broadening).
    let (root_sk, root_pk) = key(0x1001);
    let (alice_sk, alice_pk) = key(0x1002);
    let (agent_sk, agent_pk) = key(0x1003);
    let hop0 = signed_hop(
        ROOT,
        &root_sk,
        &root_pk,
        ALICE,
        &alice_pk,
        cap(vec![read()], DOCS),
        SOON, // parent expires SOON
    );
    let hop1 = signed_hop(
        ALICE,
        &alice_sk,
        &alice_pk,
        AGENT,
        &agent_pk,
        cap(vec![read()], DOC_X),
        FAR_FUTURE, // child tries to outlive the parent
    );
    let chain = DelegationChain {
        hops: vec![hop0, hop1],
    };
    let inv = invocation(AGENT, &agent_sk, CHALLENGE, NOW);
    let err = invoke(&chain, &[root_pk], &inv, &iri(DOC_X)).unwrap_err();
    assert_eq!(err, InvocationDenied::NonMonotoneAttenuation { hop: 1 });
}

#[test]
fn broken_chain_link_is_denied() {
    // hop0.delegate (alice) != hop1.delegator: the chain does not actually link. An
    // attacker splices two unrelated genuine hops together ⇒ denied.
    let (root_sk, root_pk) = key(0x1001);
    let (_alice_sk, alice_pk) = key(0x1002);
    let (bob_sk, bob_pk) = key(0x2002); // a DIFFERENT principal
    let (agent_sk, agent_pk) = key(0x1003);
    let hop0 = signed_hop(
        ROOT,
        &root_sk,
        &root_pk,
        ALICE,
        &alice_pk,
        cap(vec![read()], DOCS),
        FAR_FUTURE,
    );
    // hop1 is delegated by BOB (genuinely signed), not alice ⇒ broken link.
    let hop1 = signed_hop(
        "https://bob.ex/card#me",
        &bob_sk,
        &bob_pk,
        AGENT,
        &agent_pk,
        cap(vec![read()], DOC_X),
        SOON,
    );
    let chain = DelegationChain {
        hops: vec![hop0, hop1],
    };
    let inv = invocation(AGENT, &agent_sk, CHALLENGE, NOW);
    let err = invoke(&chain, &[root_pk], &inv, &iri(DOC_X)).unwrap_err();
    assert_eq!(err, InvocationDenied::BrokenLink { hop: 0 });
}

#[test]
fn expired_terminal_hop_is_denied() {
    let (chain, roots, agent_sk) = canonical_chain();
    // now is past SOON (the terminal hop's expiry).
    let inv = invocation(AGENT, &agent_sk, CHALLENGE, SOON + 1);
    let err = invoke(&chain, &roots, &inv, &iri(DOC_X)).unwrap_err();
    assert_eq!(err, InvocationDenied::Expired);
}

#[test]
fn out_of_scope_target_is_denied() {
    let (chain, roots, agent_sk) = canonical_chain();
    let inv = invocation(AGENT, &agent_sk, CHALLENGE, NOW);
    // The terminal capability is over /docs/x; a request for /other/y is out of scope.
    let err = invoke(&chain, &roots, &inv, &iri(OTHER)).unwrap_err();
    assert_eq!(err, InvocationDenied::OutOfScope);
}

#[test]
fn empty_chain_is_denied() {
    let (_chain, roots, agent_sk) = canonical_chain();
    let chain = DelegationChain { hops: vec![] };
    let inv = invocation(AGENT, &agent_sk, CHALLENGE, NOW);
    let err = invoke(&chain, &roots, &inv, &iri(DOC_X)).unwrap_err();
    assert_eq!(err, InvocationDenied::EmptyChain);
}

// --- intersection-vs-current-grant (item N) ---------------------------------

#[test]
fn intersection_binds_the_current_delegator_grant_not_a_snapshot() {
    // The gate confers read over /docs/x. The CURRENT delegator grant has since been
    // narrowed to read over /docs/x still ⇒ intersection is non-empty.
    let (chain, roots, agent_sk) = canonical_chain();
    let inv = invocation(AGENT, &agent_sk, CHALLENGE, NOW);
    let eff = invoke(&chain, &roots, &inv, &iri(DOC_X)).expect("binds");

    // Current delegator grant: still read over /docs/ ⇒ intersection survives.
    let current = cap(vec![read()], DOCS);
    let composed = effective_against_current(&eff, &current).expect("intersection non-empty");
    assert_eq!(composed.capability.actions, vec![read()]);
    assert_eq!(composed.capability.target.as_str(), DOC_X);
}

#[test]
fn revoked_current_delegator_grant_yields_no_effective_authority() {
    // The delegator's grant has been REVOKED down to a disjoint scope (item N): the
    // intersection against the CURRENT grant is empty, so the agent retains NOTHING —
    // it does NOT keep escalated authority from a delegation-time snapshot.
    let (chain, roots, agent_sk) = canonical_chain();
    let inv = invocation(AGENT, &agent_sk, CHALLENGE, NOW);
    let eff = invoke(&chain, &roots, &inv, &iri(DOC_X)).expect("binds");

    // Current delegator grant narrowed to a DISJOINT container ⇒ empty intersection.
    let current = cap(vec![read()], "https://pod.ex/elsewhere/");
    assert!(
        effective_against_current(&eff, &current).is_none(),
        "intersecting against the current (revoked/narrowed) delegator grant yields nothing — \
         no stale-snapshot escalation (item N)"
    );

    // And an action revocation: the delegator no longer holds read ⇒ empty.
    let current_no_read = cap(vec![write()], DOCS);
    assert!(
        effective_against_current(&eff, &current_no_read).is_none(),
        "if the current delegator grant no longer includes the action, the agent loses it"
    );
}

#[test]
fn single_hop_root_to_agent_chain_works() {
    // A degenerate one-hop chain (root delegates straight to the agent) also binds.
    let (root_sk, root_pk) = key(0x1001);
    let (agent_sk, agent_pk) = key(0x1003);
    let hop0 = signed_hop(
        ROOT,
        &root_sk,
        &root_pk,
        AGENT,
        &agent_pk,
        cap(vec![read()], DOC_X),
        FAR_FUTURE,
    );
    let chain = DelegationChain { hops: vec![hop0] };
    let inv = invocation(AGENT, &agent_sk, CHALLENGE, NOW);
    let eff = invoke(&chain, &[root_pk], &inv, &iri(DOC_X)).expect("single-hop binds");
    assert_eq!(eff.delegate.as_str(), AGENT);
}

// --- the message-binding sanity (cross-protocol confusion defence) ----------

#[test]
fn hop_message_differs_from_holder_pop_message() {
    // The hop message and the holder-PoP message use distinct domain tags, so a hop
    // signature can never be confused for a PoP signature (and vice versa).
    let (_sk, root_pk) = key(0x1001);
    let (_sk2, agent_pk) = key(0x1003);
    let hop = DelegationHop {
        delegator: iri(ROOT),
        delegator_key: root_pk,
        delegate: iri(AGENT),
        delegate_key: agent_pk,
        capability: cap(vec![read()], DOC_X),
        expires_at_unix_secs: FAR_FUTURE,
        signature_hex: String::new(),
    };
    let hop_msg = hop_message(&hop);
    let challenge_fr = field_from_hash_bytes(&CHALLENGE);
    let pop_msg = holder_pop_message(&challenge_fr);
    assert_ne!(
        hop_msg, pop_msg,
        "domain separation: a delegation-hop message is never a holder-PoP message"
    );
}
