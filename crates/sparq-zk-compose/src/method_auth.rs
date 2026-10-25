//! Verifier-side authentication of a commitment's COMMITMENT METHOD from the
//! issuer signature, and the relying party's policy over the result.
//!
//! # What this module decides
//! A committed graph's [`CommitmentMethod`] determines which leaf shape it was
//! committed under, and therefore which circuit family may legally be run
//! against it. Until now the verifier had no way to learn that method: the
//! manifest's [`crate::manifest::CommitmentAttestation`] carries no scheme
//! field, and inventing one would make the method a prover-supplied value — the
//! same shape as the `manifest.key_set` anchor that made audit #3 vacuous
//! before the trust anchor moved outside the proof.
//!
//! So the method is not read anywhere. It is RECOVERED from the signature. The
//! METHOD-BOUND presentation messages are `ZKSIG_M5`
//! ([`sparq_zk::sig::presentation_message_with_status_and_scheme`], for a bearer
//! attestation) and `ZKSIG_M4`
//! ([`sparq_zk::sig::commitment_message_with_holder_and_scheme`], for a
//! holder-bound one), whose signed object folds the method's
//! [`sparq_zk::sig::scheme_iri_to_field`] digest. The verifier builds one
//! candidate message per recognised method from values it computed ITSELF (the
//! commitment, the issuer-attested salt, the issuer-bound status reference, and
//! — when the attestation is holder-bound — the attested holder digest), and
//! asks which candidate the one presented signature verifies over. The answer
//! is an [`AuthenticatedMethod`].
//!
//! A registry entry's own `ZKSIG_M3` signature
//! ([`sparq_zk::sig::commitment_message_with_status_and_scheme`]) is NOT a
//! candidate: recording a commitment in an issuer's registry is a different
//! statement from authorising its presentation, so it is left under its own
//! domain tag and never authenticates a presentation.
//!
//! # Who chooses what
//! Every candidate MESSAGE is verifier-built: the verifier computes the whole set
//! from its own values, so no prover field enters a candidate. On the CLEAR path
//! the prover supplies only the signature, and which candidate verifies is
//! decided by the signature scheme. On the HIDDEN path the prover additionally
//! declares WHICH candidate its hidden-issuer proof is over, and that declared
//! message must EQUAL one of them — an unmatched message is refused
//! (`CheckError::HiddenIssuerMessageMismatch`) rather than ignored, and whichever
//! candidate it selects, the prover must still exhibit a key-set-membership proof
//! over that exact message under the relying party's own root. Either way a
//! prover cannot make a method be recovered that its issuer did not sign, because
//! every candidate is a distinct signed message under a distinct domain tag, and
//! producing a signature over a message the issuer never signed is exactly the
//! forgery the signature scheme is assumed to resist.
//!
//! # Back-compatibility is an explicit opt-in, not the default
//! Every credential issued before the method-bound message shapes existed signs
//! a method-UNBOUND message (`ZKSIG_C3` or `ZKSIG_C4`). Those are candidates
//! too, and a signature that verifies only over one of them authenticates as
//! [`AuthenticatedMethod::Unbound`] — "this issuer attested the commitment, the
//! salt and the status reference, but attested NO method". Under the default
//! [`MethodPolicy::current`] that is now REFUSED: a commitment whose method is
//! not issuer-attested has no authenticated leaf shape. `Unbound` is never read
//! as a method — it is the explicit absence of one — and a relying party that
//! must keep accepting already-issued method-unbound material opts into
//! [`MethodPolicy::admit_unbound`].
//!
//! # Honest scope
//! Authenticating the method does NOT open any value-lane gate. The stage-0
//! value-lane refusal and the stage-2 operand match are unchanged; what this
//! adds is the input those gates will need. Research-grade, NOT externally
//! audited (sq-qhy4); no soundness or privacy property is claimed as achieved.

use sparq_zk::commit::CommitmentMethod;
use sparq_zk::field::Fr;
use sparq_zk::sig::{
    commitment_message_with_holder, commitment_message_with_holder_and_scheme,
    commitment_message_with_status, presentation_message_with_status_and_scheme,
    scheme_iri_to_field,
};

/// What a commitment's issuer signature attests about the COMMITMENT METHOD.
///
/// Deliberately a two-case enum rather than an `Option<CommitmentMethod>`: the
/// two cases are not "present / missing data" but two different attested facts,
/// and naming the absent case stops it being read as a default. There is no
/// variant for "some method, unknown which" — a signature that verifies over no
/// candidate is not authenticated at all, and the caller reports it as the
/// existing invalid-signature refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthenticatedMethod {
    /// The issuer signed a METHOD-BOUND message (`ZKSIG_M5` / `ZKSIG_M4`) whose
    /// folded scheme digest is this method's. The method is issuer-attested.
    Bound(CommitmentMethod),
    /// The issuer signed a method-UNBOUND message (`ZKSIG_C3` / `ZKSIG_C4`). The
    /// commitment, salt and status reference are attested; the METHOD is not.
    /// This is the absence of an attested method, never a default one.
    Unbound,
}

impl AuthenticatedMethod {
    /// The attested method, or `None` for [`Self::Unbound`]. The `None` means
    /// "no method was attested" — a caller must not substitute a default.
    pub fn method(self) -> Option<CommitmentMethod> {
        match self {
            AuthenticatedMethod::Bound(m) => Some(m),
            AuthenticatedMethod::Unbound => None,
        }
    }
}

/// The authenticated commitment method of each commitment a verified sub-proof
/// references, keyed by canonical `sparq_zk::field::field_to_hex` commitment hex.
///
/// A commitment ABSENT from the map had no method authenticated for it. That is
/// NOT the same fact as an explicit [`AuthenticatedMethod::Unbound`] entry, which
/// records an attestation that verified and attested no method, and the consumers
/// do not read it alike:
///
/// * the per-sub-proof member resolution (`verifier`, `dual-leaf`) refuses an
///   absent entry BY NAME (`CheckError::SubProofCommitmentNotAuthenticated`) —
///   nothing was authenticated for the commitment, so there is nothing to fold;
/// * the pattern-constant ENCODER selection reads an absent entry as `Unbound`,
///   which selects the string-canonical encoder — the state that predates method
///   authentication.
///
/// What no consumer may do is read absence as a DEFAULT METHOD.
pub type AuthenticatedMethods = std::collections::BTreeMap<String, AuthenticatedMethod>;

/// The relying party's policy over an [`AuthenticatedMethod`] — the external
/// decision about which attested methods it will act on.
///
/// It lives beside the issuer trust anchor on [`crate::verifier::KeySet`]
/// (`KeySet::with_method_policy`) for the same reason the key set does: it is
/// the relying party's own input, never read from the manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MethodPolicy {
    admit_legacy_dual_leaf_v1: bool,
    accept_unbound: bool,
}

impl MethodPolicy {
    /// Today's default: REFUSE a method-UNBOUND attestation, and refuse an
    /// attested `DualLeafV1`, which is recognised and verifiable but superseded
    /// for new issuance.
    ///
    /// # The unbound default is fail-closed
    /// It used to accept an unbound attestation, so that every credential issued
    /// before the method-bound message shapes existed kept verifying. That was
    /// the transitional setting, and it is the one no value-lane gate can be
    /// opened over: a commitment whose method is not issuer-attested has no
    /// authenticated leaf shape, so there is nothing to pin a lane constant or a
    /// pattern-slot encoding against. Refusing it by default is a PREREQUISITE
    /// (sq-j506), not the gate itself — the Stage-0 value-lane refusals stay
    /// closed regardless.
    ///
    /// A relying party that must keep accepting already-issued method-unbound
    /// material states so with [`Self::admit_unbound`]. That is a deliberate,
    /// written-down downgrade rather than a silent default.
    pub fn current() -> Self {
        MethodPolicy {
            admit_legacy_dual_leaf_v1: false,
            accept_unbound: false,
        }
    }

    /// Also admit an attested `DualLeafV1` — the written-down opt-in for a
    /// relying party that must keep accepting credentials issued under the
    /// superseded dual-leaf version. Mirrors
    /// `CommitmentMethod::checked_for(MethodUse::LegacyAlreadyIssued)`: legacy
    /// acceptance has to be stated at the call site instead of being silent.
    #[must_use]
    pub fn admit_legacy_dual_leaf_v1(mut self) -> Self {
        self.admit_legacy_dual_leaf_v1 = true;
        self
    }

    /// Require an attested method: a method-UNBOUND attestation is REFUSED.
    ///
    /// This is now the DEFAULT ([`Self::current`]), so calling it on a fresh
    /// policy changes nothing. It is kept because it states the requirement
    /// explicitly at a call site, and because it is how the requirement is taken
    /// back after [`Self::admit_unbound`].
    #[must_use]
    pub fn require_method_bound(mut self) -> Self {
        self.accept_unbound = false;
        self
    }

    /// Also accept a method-UNBOUND attestation (`ZKSIG_C3` / `ZKSIG_C4`) — the
    /// written-down opt-in for a relying party that must keep verifying
    /// credentials issued before the method-bound message shapes existed.
    ///
    /// It is a DOWNGRADE, and the weaker of the two settings: an unbound
    /// attestation covers the commitment, the salt and the status reference but
    /// attests NO commitment method, so nothing authenticates which leaf shape
    /// the graph was committed under. `Unbound` is still never read AS a method;
    /// this only decides whether its explicit absence is tolerated.
    #[must_use]
    pub fn admit_unbound(mut self) -> Self {
        self.accept_unbound = true;
        self
    }

    /// The WIDEST policy over attested methods — the one that admits every method
    /// some policy admits.
    ///
    /// The orphan / no-method-attested member rule in `crate::verifier` has to
    /// quantify over the methods a commitment could ever have authenticated.
    /// Stage 2d refuses any method the relying party's policy does not admit, so
    /// that set is exactly the union over all policies, and this names it in one
    /// place instead of each caller rebuilding it from the settings.
    ///
    /// `accept_unbound` is deliberately NOT part of it: it decides whether the
    /// ABSENCE of a method is tolerated, not which method is admitted, and
    /// [`Self::admits_method`] does not read it.
    #[must_use]
    pub fn widest_admissible() -> Self {
        MethodPolicy::current().admit_legacy_dual_leaf_v1()
    }

    /// Whether a method-UNBOUND attestation is accepted.
    pub fn accepts_unbound(&self) -> bool {
        self.accept_unbound
    }

    /// Apply the policy to a classified result: `Ok` when the relying party will
    /// act on it, else the refusal.
    ///
    /// Two refusals, both fail-closed:
    /// - an UNBOUND attestation under [`Self::require_method_bound`];
    /// - an attested method the policy does not admit — `DualLeafV1` without
    ///   [`Self::admit_legacy_dual_leaf_v1`], and the `value-only` research dial
    ///   under every policy (it is never admissible, so there is no setting that
    ///   turns it on).
    pub fn admit(&self, authenticated: AuthenticatedMethod) -> Result<AuthenticatedMethod, MethodAuthError> {
        match authenticated {
            AuthenticatedMethod::Unbound => {
                if self.accept_unbound {
                    Ok(authenticated)
                } else {
                    Err(MethodAuthError::MethodNotAuthenticated)
                }
            }
            AuthenticatedMethod::Bound(m) => {
                if self.admits_method(m) {
                    Ok(authenticated)
                } else {
                    Err(MethodAuthError::MethodNotAdmitted(m))
                }
            }
        }
    }

    /// Whether this policy admits an ATTESTED method.
    ///
    /// The `match` is exhaustive over [`CommitmentMethod`] on purpose: a new
    /// method does not compile until this policy states whether it is admitted,
    /// so a method can never be admitted by falling through a wildcard.
    pub fn admits_method(&self, method: CommitmentMethod) -> bool {
        match method {
            CommitmentMethod::StringCanonicalV1 | CommitmentMethod::DualLeafV2 => true,
            CommitmentMethod::DualLeafV1 => self.admit_legacy_dual_leaf_v1,
            // The research dial is never admitted on a verify path, under any
            // policy: it drops term identity entirely. There is no setting for it.
            #[cfg(feature = "commitment-value-only")]
            CommitmentMethod::ValueOnlyV1 => false,
        }
    }
}

impl Default for MethodPolicy {
    fn default() -> Self {
        MethodPolicy::current()
    }
}

/// Why method authentication refused. Mapped to a
/// [`crate::verifier::CheckError`] by the caller, so the verifier's error
/// surface stays one enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodAuthError {
    /// More than one METHOD-BOUND candidate verified, so the attested method is
    /// not unique. Fail-closed: the verifier refuses rather than picking one.
    AmbiguousMethod,
    /// No candidate verified at all. The caller reports its existing
    /// invalid-signature refusal, so the error precedence a manifest sees for a
    /// genuinely bad signature is unchanged.
    NoCandidateVerified,
    /// Only a method-UNBOUND candidate verified, and the policy requires an
    /// attested method ([`MethodPolicy::require_method_bound`]).
    MethodNotAuthenticated,
    /// A method WAS attested, and the policy does not admit it.
    MethodNotAdmitted(CommitmentMethod),
}

impl std::fmt::Display for MethodAuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MethodAuthError::AmbiguousMethod => write!(
                f,
                "more than one commitment method's signed message verified; the attested method is not unique"
            ),
            MethodAuthError::NoCandidateVerified => {
                write!(f, "no candidate issuer-signed message verified")
            }
            MethodAuthError::MethodNotAuthenticated => write!(
                f,
                "the issuer signature attests no commitment method, and the policy requires an authenticated one"
            ),
            MethodAuthError::MethodNotAdmitted(m) => write!(
                f,
                "the authenticated commitment method {} is not admitted by the policy",
                m.scheme_iri()
            ),
        }
    }
}

impl std::error::Error for MethodAuthError {}

/// The verifier-computed inputs every candidate signed message is built from.
///
/// All four come from values the verifier already established: `commitment` is
/// byte-bound into the proof's public inputs, `salt` and `status_ref` are the
/// issuer-attested ones the existing gates recompute, and `holder_digest` is the
/// attested holder binding. None of them is a free prover field, which is what
/// makes the candidate set verifier-owned.
#[derive(Debug, Clone, Copy)]
pub struct CommitmentMessageInputs<'a> {
    /// `C(G)`.
    pub commitment: &'a Fr,
    /// The per-graph RDFC10 salt.
    pub salt: &'a Fr,
    /// The issuer-signed status-reference digest.
    pub status_ref: &'a Fr,
    /// The attested holder-key digest, for a HOLDER-BOUND attestation. `Some`
    /// selects the holder-bound message shapes (`ZKSIG_C4` / `ZKSIG_M4`);
    /// `None` selects the status-only ones (`ZKSIG_C3` / `ZKSIG_M5`).
    pub holder_digest: Option<&'a Fr>,
}

/// Every candidate signed message for one commitment: the METHOD-BOUND message
/// of each [`CommitmentMethod::recognised`] method, then the method-UNBOUND one.
///
/// The holder-bound and status-only shapes are never mixed: an attestation
/// carries a holder binding or it does not, and that choice selects one message
/// family. Mixing them would mean asking whether a signature over
/// `(C, salt, status_ref)` verifies as one over
/// `(C, salt, status_ref, holder_digest)`, which is not a question about the
/// method at all.
///
/// The UNBOUND candidate is LAST so a caller iterating in order sees the bound
/// ones first; the classification does not depend on the order (it counts bound
/// hits), and a test pins that.
pub fn candidate_messages(inputs: &CommitmentMessageInputs<'_>) -> Vec<(AuthenticatedMethod, Fr)> {
    let mut out: Vec<(AuthenticatedMethod, Fr)> =
        Vec::with_capacity(CommitmentMethod::recognised().len() + 1);
    for method in CommitmentMethod::recognised() {
        let scheme = scheme_iri_to_field(method.scheme_iri());
        let m = match inputs.holder_digest {
            Some(holder) => commitment_message_with_holder_and_scheme(
                inputs.commitment,
                inputs.salt,
                inputs.status_ref,
                holder,
                &scheme,
            ),
            None => presentation_message_with_status_and_scheme(
                inputs.commitment,
                inputs.salt,
                inputs.status_ref,
                &scheme,
            ),
        };
        out.push((AuthenticatedMethod::Bound(*method), m));
    }
    let unbound = match inputs.holder_digest {
        Some(holder) => commitment_message_with_holder(
            inputs.commitment,
            inputs.salt,
            inputs.status_ref,
            holder,
        ),
        None => commitment_message_with_status(inputs.commitment, inputs.salt, inputs.status_ref),
    };
    out.push((AuthenticatedMethod::Unbound, unbound));
    out
}

/// Decide which [`AuthenticatedMethod`] a set of candidates authenticates, given
/// a predicate that says whether a candidate message verifies.
///
/// A PURE function of `candidates` and `verifies` — it reads no manifest, runs
/// no signature check of its own, and applies no policy. That is what makes the
/// decision rule testable on its own, independently of signing keys.
///
/// The rule, in order:
/// 1. **More than one BOUND candidate verifies** => [`MethodAuthError::AmbiguousMethod`].
///    Two distinct methods' messages verifying under one signature would mean
///    the attested method is not unique, so the verifier refuses rather than
///    choosing. (It should be unreachable: the messages are distinct under
///    distinct domain tags and distinct scheme digests. Refusing anyway keeps
///    the rule total without resting on that.)
/// 2. **Exactly one BOUND candidate verifies** => that method is the attested
///    one. The decision keys on BOUND hits alone, so this holds whether or not
///    an unbound candidate also verified.
/// 3. **No bound candidate, but an UNBOUND one verifies** =>
///    [`AuthenticatedMethod::Unbound`].
/// 4. **Nothing verifies** => [`MethodAuthError::NoCandidateVerified`].
pub fn classify(
    candidates: &[(AuthenticatedMethod, Fr)],
    verifies: impl Fn(&Fr) -> bool,
) -> Result<AuthenticatedMethod, MethodAuthError> {
    let mut bound_hit: Option<CommitmentMethod> = None;
    let mut bound_hits = 0usize;
    let mut unbound_hit = false;
    for (which, message) in candidates {
        if !verifies(message) {
            continue;
        }
        match which {
            AuthenticatedMethod::Bound(m) => {
                bound_hits += 1;
                bound_hit = Some(*m);
            }
            AuthenticatedMethod::Unbound => unbound_hit = true,
        }
    }
    if bound_hits > 1 {
        return Err(MethodAuthError::AmbiguousMethod);
    }
    if let Some(m) = bound_hit {
        return Ok(AuthenticatedMethod::Bound(m));
    }
    if unbound_hit {
        return Ok(AuthenticatedMethod::Unbound);
    }
    Err(MethodAuthError::NoCandidateVerified)
}

/// [`candidate_messages`] + [`classify`] + [`MethodPolicy::admit`] in one call —
/// the single entry point every verifier-side authentication site uses, so the
/// rule cannot be stated twice and drift.
pub fn authenticate(
    inputs: &CommitmentMessageInputs<'_>,
    policy: &MethodPolicy,
    verifies: impl Fn(&Fr) -> bool,
) -> Result<AuthenticatedMethod, MethodAuthError> {
    let candidates = candidate_messages(inputs);
    let authenticated = classify(&candidates, verifies)?;
    policy.admit(authenticated)
}

/// The signed message an authenticated method implies — the message a verifier
/// must use wherever it has to RECOMPUTE this commitment's issuer-signed value
/// (the hidden-issuer public-input binding, for instance) rather than re-check a
/// clear signature.
///
/// For [`AuthenticatedMethod::Unbound`] this is the method-unbound message, i.e.
/// exactly what the verifier computed before method authentication existed. So a
/// commitment that authenticates as `Unbound` reproduces the previous bytes.
pub fn authenticated_message(
    inputs: &CommitmentMessageInputs<'_>,
    authenticated: AuthenticatedMethod,
) -> Fr {
    candidate_messages(inputs)
        .into_iter()
        .find(|(which, _)| *which == authenticated)
        .map(|(_, m)| m)
        .expect("candidate_messages covers every AuthenticatedMethod this build can produce")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs<'a>(holder: Option<&'a Fr>, c: &'a Fr, s: &'a Fr, r: &'a Fr) -> CommitmentMessageInputs<'a> {
        CommitmentMessageInputs { commitment: c, salt: s, status_ref: r, holder_digest: holder }
    }

    #[test]
    fn candidates_cover_every_recognised_method_plus_unbound_and_are_distinct() {
        let (c, s, r) = (Fr::from(2u64), Fr::from(3u64), Fr::from(5u64));
        for holder in [None, Some(Fr::from(7u64))] {
            let cands = candidate_messages(&inputs(holder.as_ref(), &c, &s, &r));
            assert_eq!(cands.len(), CommitmentMethod::recognised().len() + 1);
            for m in CommitmentMethod::recognised() {
                assert!(
                    cands.iter().any(|(w, _)| *w == AuthenticatedMethod::Bound(*m)),
                    "no candidate for {}",
                    m.scheme_iri()
                );
            }
            assert_eq!(cands.last().unwrap().0, AuthenticatedMethod::Unbound);
            // Every candidate message is a distinct field element: that is what
            // makes at most one of them verify under one signature.
            for (i, (_, a)) in cands.iter().enumerate() {
                for (_, b) in cands.iter().skip(i + 1) {
                    assert_ne!(a, b, "two candidate messages collide");
                }
            }
        }
        // The holder-bound and status-only families share no message.
        let plain = candidate_messages(&inputs(None, &c, &s, &r));
        let holder = Fr::from(7u64);
        let bound = candidate_messages(&inputs(Some(&holder), &c, &s, &r));
        for (_, a) in &plain {
            for (_, b) in &bound {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn classify_picks_the_single_bound_hit() {
        let (c, s, r) = (Fr::from(1u64), Fr::from(1u64), Fr::from(1u64));
        let cands = candidate_messages(&inputs(None, &c, &s, &r));
        let target = cands
            .iter()
            .find(|(w, _)| *w == AuthenticatedMethod::Bound(CommitmentMethod::DualLeafV2))
            .map(|(_, m)| *m)
            .expect("DualLeafV2 is recognised");
        assert_eq!(
            classify(&cands, |m| *m == target),
            Ok(AuthenticatedMethod::Bound(CommitmentMethod::DualLeafV2))
        );
    }

    #[test]
    fn classify_refuses_two_bound_hits_and_reports_nothing_verifying() {
        let (c, s, r) = (Fr::from(1u64), Fr::from(2u64), Fr::from(3u64));
        let cands = candidate_messages(&inputs(None, &c, &s, &r));
        let bound: Vec<Fr> = cands
            .iter()
            .filter(|(w, _)| matches!(w, AuthenticatedMethod::Bound(_)))
            .map(|(_, m)| *m)
            .collect();
        assert!(bound.len() >= 2);
        let two = [bound[0], bound[1]];
        assert_eq!(
            classify(&cands, |m| two.contains(m)),
            Err(MethodAuthError::AmbiguousMethod)
        );
        assert_eq!(
            classify(&cands, |_| false),
            Err(MethodAuthError::NoCandidateVerified)
        );
    }

    /// A BOUND hit wins over a simultaneous UNBOUND hit, and the result does not
    /// depend on candidate order — the classification counts bound hits, it does
    /// not take the first match.
    #[test]
    fn classify_prefers_a_bound_hit_and_is_order_independent() {
        let (c, s, r) = (Fr::from(4u64), Fr::from(8u64), Fr::from(16u64));
        let cands = candidate_messages(&inputs(None, &c, &s, &r));
        let bound_v2 = cands
            .iter()
            .find(|(w, _)| *w == AuthenticatedMethod::Bound(CommitmentMethod::DualLeafV2))
            .map(|(_, m)| *m)
            .unwrap();
        let unbound = cands
            .iter()
            .find(|(w, _)| *w == AuthenticatedMethod::Unbound)
            .map(|(_, m)| *m)
            .unwrap();
        let hits = [bound_v2, unbound];
        assert_eq!(
            classify(&cands, |m| hits.contains(m)),
            Ok(AuthenticatedMethod::Bound(CommitmentMethod::DualLeafV2))
        );
        let mut reversed = cands.clone();
        reversed.reverse();
        assert_eq!(
            classify(&reversed, |m| hits.contains(m)),
            Ok(AuthenticatedMethod::Bound(CommitmentMethod::DualLeafV2))
        );
        // Only the unbound candidate verifying authenticates as Unbound.
        assert_eq!(
            classify(&cands, |m| *m == unbound),
            Ok(AuthenticatedMethod::Unbound)
        );
    }

    #[test]
    fn default_policy_refuses_unbound_and_refuses_legacy_v1() {
        let p = MethodPolicy::current();
        assert_eq!(p, MethodPolicy::default());
        assert!(!p.accepts_unbound());
        assert_eq!(
            p.admit(AuthenticatedMethod::Unbound),
            Err(MethodAuthError::MethodNotAuthenticated)
        );
        for m in [CommitmentMethod::StringCanonicalV1, CommitmentMethod::DualLeafV2] {
            assert_eq!(
                p.admit(AuthenticatedMethod::Bound(m)),
                Ok(AuthenticatedMethod::Bound(m))
            );
        }
        assert_eq!(
            p.admit(AuthenticatedMethod::Bound(CommitmentMethod::DualLeafV1)),
            Err(MethodAuthError::MethodNotAdmitted(CommitmentMethod::DualLeafV1))
        );
    }

    #[test]
    fn the_legacy_opt_in_is_what_admits_dual_leaf_v1() {
        let p = MethodPolicy::current().admit_legacy_dual_leaf_v1();
        assert_eq!(
            p.admit(AuthenticatedMethod::Bound(CommitmentMethod::DualLeafV1)),
            Ok(AuthenticatedMethod::Bound(CommitmentMethod::DualLeafV1))
        );
        // The opt-in does not widen anything else — in particular it does not
        // also admit a method-UNBOUND attestation.
        assert!(!p.accepts_unbound());
        assert_eq!(
            p.admit(AuthenticatedMethod::Unbound),
            Err(MethodAuthError::MethodNotAuthenticated)
        );
    }

    #[test]
    fn require_method_bound_refuses_an_unbound_attestation_only() {
        let p = MethodPolicy::current().require_method_bound();
        assert!(!p.accepts_unbound());
        assert_eq!(
            p.admit(AuthenticatedMethod::Unbound),
            Err(MethodAuthError::MethodNotAuthenticated)
        );
        assert_eq!(
            p.admit(AuthenticatedMethod::Bound(CommitmentMethod::DualLeafV2)),
            Ok(AuthenticatedMethod::Bound(CommitmentMethod::DualLeafV2))
        );
    }

    /// `widest_admissible` really is the union over every policy this build can
    /// construct: no setting admits a method it does not.
    ///
    /// The verifier's method-independence rule quantifies over this set, so a
    /// policy that admitted a method outside it would make that rule unsound —
    /// a member could be legal under every method the rule checks and illegal
    /// under the one actually authenticated.
    #[test]
    fn widest_admissible_is_a_superset_of_every_constructible_policy() {
        let widest = MethodPolicy::widest_admissible();
        for p in [
            MethodPolicy::default(),
            MethodPolicy::current(),
            MethodPolicy::current().admit_unbound(),
            MethodPolicy::current().admit_legacy_dual_leaf_v1(),
            MethodPolicy::current().require_method_bound(),
            MethodPolicy::current()
                .admit_legacy_dual_leaf_v1()
                .admit_unbound(),
            MethodPolicy::current()
                .admit_legacy_dual_leaf_v1()
                .require_method_bound(),
        ] {
            for m in CommitmentMethod::recognised() {
                assert!(
                    !p.admits_method(*m) || widest.admits_method(*m),
                    "{} is admitted by a policy but not by widest_admissible",
                    m.scheme_iri()
                );
            }
        }
        // Not vacuous: the widest policy admits a method the DEFAULT one refuses.
        assert!(widest.admits_method(CommitmentMethod::DualLeafV1));
        assert!(!MethodPolicy::current().admits_method(CommitmentMethod::DualLeafV1));
        // And it is a policy over METHODS only: it does not tolerate the absence
        // of one.
        assert!(!widest.accepts_unbound());
    }

    /// The research dial is never admitted, under any policy setting.
    #[cfg(feature = "commitment-value-only")]
    #[test]
    fn the_value_only_dial_is_never_admitted() {
        for p in [
            MethodPolicy::current(),
            MethodPolicy::current().admit_legacy_dual_leaf_v1(),
            MethodPolicy::current().require_method_bound(),
            MethodPolicy::current()
                .admit_legacy_dual_leaf_v1()
                .require_method_bound(),
        ] {
            assert!(!p.admits_method(CommitmentMethod::ValueOnlyV1));
            assert_eq!(
                p.admit(AuthenticatedMethod::Bound(CommitmentMethod::ValueOnlyV1)),
                Err(MethodAuthError::MethodNotAdmitted(CommitmentMethod::ValueOnlyV1))
            );
        }
    }

    /// `authenticated_message(.., Unbound)` must be byte-identical to the
    /// method-unbound message the verifier computed before this module existed —
    /// that equality is the whole back-compatibility argument.
    #[test]
    fn the_unbound_authenticated_message_is_the_pre_existing_message() {
        let (c, s, r) = (Fr::from(31u64), Fr::from(41u64), Fr::from(59u64));
        assert_eq!(
            authenticated_message(&inputs(None, &c, &s, &r), AuthenticatedMethod::Unbound),
            commitment_message_with_status(&c, &s, &r)
        );
        let holder = Fr::from(26u64);
        assert_eq!(
            authenticated_message(
                &inputs(Some(&holder), &c, &s, &r),
                AuthenticatedMethod::Unbound
            ),
            commitment_message_with_holder(&c, &s, &r, &holder)
        );
        // And a BOUND method yields that method's own message.
        assert_eq!(
            authenticated_message(
                &inputs(None, &c, &s, &r),
                AuthenticatedMethod::Bound(CommitmentMethod::StringCanonicalV1)
            ),
            presentation_message_with_status_and_scheme(
                &c,
                &s,
                &r,
                &scheme_iri_to_field(CommitmentMethod::StringCanonicalV1.scheme_iri())
            )
        );
    }

    /// End to end against a REAL signature: an issuer that signs the `ZKSIG_M5`
    /// presentation message of one method is authenticated as that method, and
    /// the same signature authenticates as nothing else.
    #[test]
    fn a_real_m5_signature_authenticates_as_its_own_method() {
        use sparq_zk::sig::{public_key_from_hex, signature_from_hex, verify, SecretKey};
        let sk = SecretKey::from_seed(9);
        let pk_hex = sparq_zk::sig::public_key_to_hex(&sk.public_key());
        let (c, s, r) = (Fr::from(12u64), Fr::from(34u64), Fr::from(56u64));
        for method in [CommitmentMethod::StringCanonicalV1, CommitmentMethod::DualLeafV2] {
            let scheme = scheme_iri_to_field(method.scheme_iri());
            let sig_hex = sk.sign_presentation_with_status_and_scheme(&c, &s, &r, &scheme);
            let pk = public_key_from_hex(&pk_hex).unwrap();
            let sig = signature_from_hex(&sig_hex).unwrap();
            assert_eq!(
                authenticate(&inputs(None, &c, &s, &r), &MethodPolicy::current(), |m| {
                    verify(&pk, m, &sig)
                }),
                Ok(AuthenticatedMethod::Bound(method))
            );
        }
        // A method-UNBOUND signature is refused under the default policy, and
        // authenticates as Unbound only under the explicit downgrade.
        let sig_hex = sk.sign_commitment_with_status(&c, &s, &r);
        let pk = public_key_from_hex(&pk_hex).unwrap();
        let sig = signature_from_hex(&sig_hex).unwrap();
        assert_eq!(
            authenticate(&inputs(None, &c, &s, &r), &MethodPolicy::current(), |m| {
                verify(&pk, m, &sig)
            }),
            Err(MethodAuthError::MethodNotAuthenticated)
        );
        assert_eq!(
            authenticate(
                &inputs(None, &c, &s, &r),
                &MethodPolicy::current().admit_unbound(),
                |m| verify(&pk, m, &sig)
            ),
            Ok(AuthenticatedMethod::Unbound)
        );
        assert_eq!(
            authenticate(
                &inputs(None, &c, &s, &r),
                &MethodPolicy::current().require_method_bound(),
                |m| verify(&pk, m, &sig)
            ),
            Err(MethodAuthError::MethodNotAuthenticated)
        );
    }

    /// A REGISTRY entry's signature is over `ZKSIG_M3`, which is not in the
    /// candidate set, so it authenticates NOTHING — publishing a commitment to an
    /// issuer's registry does not make it presentable. The equivalent `ZKSIG_M5`
    /// presentation signature over the same tuple does authenticate, which shows
    /// the refusal is the domain tag and nothing else.
    #[test]
    fn a_registry_signature_authenticates_no_presentation_method() {
        use oxrdf::NamedNode;
        use sparq_zk::registry::{RegistryEntry, Status};
        use sparq_zk::sig::{
            public_key_from_hex, signature_from_hex, status_list_id_to_field, status_ref_digest,
            verify, SecretKey,
        };
        const LIST: &str = "http://ex/status/1";
        const INDEX: u64 = 3;
        const VERSION: u64 = 1;
        let sk = SecretKey::from_seed(23);
        let pk = public_key_from_hex(&sparq_zk::sig::public_key_to_hex(&sk.public_key())).unwrap();
        let (c, s) = (Fr::from(2u64), Fr::from(4u64));
        let r = status_ref_digest(&status_list_id_to_field(LIST), INDEX, VERSION);
        let method = CommitmentMethod::DualLeafV2;

        // A real registry entry's own signature, over `ZKSIG_M3`.
        let entry = RegistryEntry::issued_with_status(
            NamedNode::new("http://ex/vc/1").unwrap(),
            c,
            s,
            method,
            NamedNode::new(LIST).unwrap(),
            INDEX,
            VERSION,
            Status::Active,
            &sk,
        )
        .expect("DualLeafV2 is selectable for new issuance");
        assert!(
            entry.verify_commitment_signature_with_status(),
            "the fixture really is a valid registry signature"
        );
        let registry_sig = signature_from_hex(
            entry
                .commitment_signature
                .as_ref()
                .expect("an issued entry carries a signature"),
        )
        .unwrap();
        assert_eq!(
            authenticate(&inputs(None, &c, &s, &r), &MethodPolicy::current(), |m| {
                verify(&pk, m, &registry_sig)
            }),
            Err(MethodAuthError::NoCandidateVerified)
        );

        // The issuer's PRESENTATION signature over the same tuple does authenticate.
        let scheme = scheme_iri_to_field(method.scheme_iri());
        let presentation_sig =
            signature_from_hex(&sk.sign_presentation_with_status_and_scheme(&c, &s, &r, &scheme))
                .unwrap();
        assert_eq!(
            authenticate(&inputs(None, &c, &s, &r), &MethodPolicy::current(), |m| {
                verify(&pk, m, &presentation_sig)
            }),
            Ok(AuthenticatedMethod::Bound(method))
        );
    }

    /// An attested `DualLeafV1` is refused by the default policy at the POLICY
    /// step, not by failing to authenticate — the two refusals are distinct.
    #[test]
    fn an_attested_legacy_method_is_refused_by_policy_not_by_authentication() {
        use sparq_zk::sig::{public_key_from_hex, signature_from_hex, verify, SecretKey};
        let sk = SecretKey::from_seed(11);
        let pk = public_key_from_hex(&sparq_zk::sig::public_key_to_hex(&sk.public_key())).unwrap();
        let (c, s, r) = (Fr::from(1u64), Fr::from(2u64), Fr::from(3u64));
        let scheme = scheme_iri_to_field(CommitmentMethod::DualLeafV1.scheme_iri());
        let sig = signature_from_hex(&sk.sign_presentation_with_status_and_scheme(&c, &s, &r, &scheme))
            .unwrap();
        let i = inputs(None, &c, &s, &r);
        // It DOES authenticate.
        assert_eq!(
            classify(&candidate_messages(&i), |m| verify(&pk, m, &sig)),
            Ok(AuthenticatedMethod::Bound(CommitmentMethod::DualLeafV1))
        );
        // The policy is what refuses it.
        assert_eq!(
            authenticate(&i, &MethodPolicy::current(), |m| verify(&pk, m, &sig)),
            Err(MethodAuthError::MethodNotAdmitted(CommitmentMethod::DualLeafV1))
        );
        assert_eq!(
            authenticate(&i, &MethodPolicy::current().admit_legacy_dual_leaf_v1(), |m| {
                verify(&pk, m, &sig)
            }),
            Ok(AuthenticatedMethod::Bound(CommitmentMethod::DualLeafV1))
        );
    }

    /// A HOLDER-BOUND attestation authenticates over the `ZKSIG_M4` family, and
    /// an M4 signature is not authenticated when the holder digest is absent
    /// (the two families do not cross).
    #[test]
    fn a_real_m4_signature_authenticates_only_with_the_holder_digest() {
        use sparq_zk::sig::{public_key_from_hex, signature_from_hex, verify, SecretKey};
        let sk = SecretKey::from_seed(13);
        let pk = public_key_from_hex(&sparq_zk::sig::public_key_to_hex(&sk.public_key())).unwrap();
        let (c, s, r, h) = (Fr::from(5u64), Fr::from(6u64), Fr::from(7u64), Fr::from(8u64));
        let scheme = scheme_iri_to_field(CommitmentMethod::DualLeafV2.scheme_iri());
        let sig = signature_from_hex(
            &sk.sign_commitment_with_holder_and_scheme(&c, &s, &r, &h, &scheme),
        )
        .unwrap();
        assert_eq!(
            authenticate(
                &inputs(Some(&h), &c, &s, &r),
                &MethodPolicy::current(),
                |m| verify(&pk, m, &sig)
            ),
            Ok(AuthenticatedMethod::Bound(CommitmentMethod::DualLeafV2))
        );
        assert_eq!(
            authenticate(&inputs(None, &c, &s, &r), &MethodPolicy::current(), |m| {
                verify(&pk, m, &sig)
            }),
            Err(MethodAuthError::NoCandidateVerified)
        );
    }

    #[test]
    fn method_accessor_names_the_absent_case() {
        assert_eq!(AuthenticatedMethod::Unbound.method(), None);
        assert_eq!(
            AuthenticatedMethod::Bound(CommitmentMethod::DualLeafV2).method(),
            Some(CommitmentMethod::DualLeafV2)
        );
    }
}
