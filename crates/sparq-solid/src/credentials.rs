//! [SONNET-4.6] sq-ysv3u — the TRUSTED verified-credential channel behind ACP `acp:vc`.
//!
//! ACP's `acp:vc` gates a matcher on the requesting agent holding a Verifiable Credential
//! satisfying a stated requirement. sparq-solid deliberately does **not** verify credentials
//! inside the reasoner: the authorizer answers a pure graph question over already-resolved,
//! already-trusted principals, and a per-request signature check does not fit the
//! materialize-once/cache model. The sound split — mandated by the design record
//! (`research/solid-vocab-gaps-design.md` §2, *"verify-in-PSS/VC-estate, match-in-rules"*) —
//! is therefore:
//!
//! 1. the **caller** (a Solid server, or the [`acp-vc`](self#the-acp-vc-feature) backend
//!    below) verifies a presented credential and decides which requirements it satisfies;
//! 2. it records `(agent WebID, requirement IRI)` pairs in a [`VerifiedCredentials`] map;
//! 3. the loader synthesizes one `<agent> solidx:holdsVc <requirement>` fact per pair, and
//!    the ACP rules match `acp:vc` against those facts.
//!
//! # Security boundary (design doc §2.4)
//!
//! `holdsVc` facts arrive through this map and **nowhere else** — never from pod content and
//! never from an `.acr`. An agent who can write a document they control must not be able to
//! assert that they hold a credential; the loader's `solidx:` reserved-predicate guard drops
//! any forged `holdsVc` triple found in a control document. Putting a pair in this map is an
//! assertion of trust by the caller, exactly like [`crate::AccessProvenance`].
//!
//! # Fail-closed
//!
//! An empty map (the default) means no agent holds any credential, so **every** `acp:vc`
//! matcher rejects every candidate. That is the conservative direction: a credential-gated
//! policy denies until a credential is actually presented and verified.
//!
//! # Requirement matching is exact-IRI, by design
//!
//! An `acp:vc` object is treated as an opaque **requirement IRI** and matched by exact IRI
//! equality. Structured claim comparison (`age >= 18`) is deliberately NOT interpreted in the
//! rules — the design record scopes v1 to exact-match predicates (§2 open question 1) and a
//! range/threshold language is separate analysis. The requirement IRI names the whole
//! predicate; deciding whether a given credential satisfies it is the verifier's job.
//!
//! # The `acp-vc` feature
//!
//! With the opt-in `acp-vc` feature ON this module also ships the **trust-the-issuer**
//! backend: [`VcRequirement`] + [`VerifiedCredentials::admit_data_integrity`] check a
//! presented credential's W3C Data Integrity proof (`eddsa-rdfc-2022`, Ed25519 over RDFC-1.0)
//! via [`sparq_vc`], confirm the signer is an accepted issuer, check the credential type and
//! exact-match claims, and record the satisfied requirements. See that method for its honest
//! scope — in particular, it is authenticity/integrity only and makes **no** privacy,
//! zero-knowledge or selective-disclosure claim.

use rustc_hash::{FxHashMap, FxHashSet};

/// Caller-supplied (TRUSTED) *already-verified* credential holdings: which ACP `acp:vc`
/// requirement IRIs each agent WebID has been proven to satisfy.
///
/// Built by the storage/authentication layer (or by the `acp-vc` backend's
/// `VerifiedCredentials::admit_data_integrity` — a code span rather than an intra-doc link,
/// since that method exists only under that feature and the workspace rustdoc gate also runs
/// `-D warnings` in the DEFAULT feature state) and passed to
/// [`crate::PodStore::materialize_acp_with_credentials`] /
/// [`crate::materialize_acp_with_credentials`]. An empty map (the default) means no agent
/// holds any credential, so no `acp:vc` matcher ever grants — fully fail-closed.
///
/// These holdings are asserted by the caller and are **never** read from pod or `.acr`
/// content — see the module docs for the full trust boundary.
///
/// # Examples
///
/// ```
/// use sparq_solid::VerifiedCredentials;
///
/// let mut creds = VerifiedCredentials::new();
/// creds.hold("https://alice.ex/card#me", "https://issuer.ex/req#OverEighteen");
/// assert!(creds.holds("https://alice.ex/card#me", "https://issuer.ex/req#OverEighteen"));
/// // an agent with nothing recorded satisfies nothing -> fail-closed
/// assert!(!creds.holds("https://bob.ex/card#me", "https://issuer.ex/req#OverEighteen"));
/// ```
#[derive(Debug, Clone, Default)]
pub struct VerifiedCredentials {
    by_agent: FxHashMap<String, FxHashSet<String>>,
}

impl VerifiedCredentials {
    /// An empty map (no agent holds any credential).
    pub fn new() -> VerifiedCredentials {
        VerifiedCredentials::default()
    }

    /// Assert (TRUSTED) that `agent` holds a verified credential satisfying the `acp:vc`
    /// requirement IRI `requirement`. The assertion is taken on trust from the caller — see
    /// the module docs for the security boundary.
    pub fn hold(&mut self, agent: impl Into<String>, requirement: impl Into<String>) {
        self.by_agent.entry(agent.into()).or_default().insert(requirement.into());
    }

    /// Whether `agent` has been asserted to satisfy the `acp:vc` requirement `requirement`.
    pub fn holds(&self, agent: &str, requirement: &str) -> bool {
        self.by_agent.get(agent).is_some_and(|s| s.contains(requirement))
    }

    /// Whether no holding at all was supplied (an empty map grants through no `acp:vc`
    /// matcher).
    pub fn is_empty(&self) -> bool {
        self.by_agent.is_empty()
    }

    /// Iterate `(agent, requirement)` for every asserted holding. Internal — the loader
    /// synthesizes one `solidx:holdsVc` reasoning fact per pair.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.by_agent
            .iter()
            .flat_map(|(a, reqs)| reqs.iter().map(move |r| (a.as_str(), r.as_str())))
    }
}

// ---------------------------------------------------------------------------
// The `acp-vc` backend: trust-the-issuer W3C Data Integrity verification.
// ---------------------------------------------------------------------------

/// One `acp:vc` requirement, expressed as what a presented credential must show for the
/// requirement IRI to be satisfied. [SONNET-4.6] sq-ysv3u, `acp-vc` feature.
///
/// This is the **trust-the-issuer** model: the requirement names an issuer whose word is
/// taken, a credential type, and zero or more exact-match claims. It is *not* a claim
/// language — the comparison is IRI/term equality (design record §2 open question 1 scopes
/// v1 to exact match).
///
/// # Examples
///
/// ```
/// use sparq_solid::VcRequirement;
///
/// let req = VcRequirement::new(
///     "https://issuer.ex/req#OverEighteen",   // the acp:vc object in the .acr
///     "did:key:z6MkiTBz1ymuepAQ4HEHYSF1H8quG5GLVVQR3djdX3mDooWp", // the accepted issuer
///     "https://issuer.ex/vocab#AgeCredential",
/// );
/// assert_eq!(req.requirement(), "https://issuer.ex/req#OverEighteen");
/// ```
#[cfg(feature = "acp-vc")]
#[cfg_attr(docsrs, doc(cfg(feature = "acp-vc")))]
#[derive(Debug, Clone)]
pub struct VcRequirement {
    requirement: String,
    issuer: String,
    credential_type: String,
    claims: Vec<(String, oxrdf::Term)>,
}

#[cfg(feature = "acp-vc")]
#[cfg_attr(docsrs, doc(cfg(feature = "acp-vc")))]
impl VcRequirement {
    /// A requirement satisfied by a credential of type `credential_type` signed by `issuer`
    /// (a DID — the `verificationMethod` of the proof must resolve under it).
    pub fn new(
        requirement: impl Into<String>,
        issuer: impl Into<String>,
        credential_type: impl Into<String>,
    ) -> VcRequirement {
        VcRequirement {
            requirement: requirement.into(),
            issuer: issuer.into(),
            credential_type: credential_type.into(),
            claims: Vec::new(),
        }
    }

    /// Additionally require the credential subject to carry `predicate` with exactly `object`.
    /// Every claim added must be present for the requirement to be satisfied.
    pub fn with_claim(
        mut self,
        predicate: impl Into<String>,
        object: impl Into<oxrdf::Term>,
    ) -> VcRequirement {
        self.claims.push((predicate.into(), object.into()));
        self
    }

    /// The `acp:vc` requirement IRI this requirement is the definition of.
    pub fn requirement(&self) -> &str {
        &self.requirement
    }
}

/// Why [`VerifiedCredentials::admit_data_integrity`] refused a presented credential.
/// [SONNET-4.6] sq-ysv3u, `acp-vc` feature.
#[cfg(feature = "acp-vc")]
#[cfg_attr(docsrs, doc(cfg(feature = "acp-vc")))]
#[derive(Debug)]
pub enum VcAdmitError {
    /// The `eddsa-rdfc-2022` Data Integrity proof did not verify over the presented triples.
    Proof(sparq_vc::VcError),
    /// The credential graph names no `credentialSubject` IRI, so there is no agent to bind
    /// the holding to.
    NoSubject,
    /// The credential graph names more than one distinct `credentialSubject`, so which agent
    /// the holding would bind to depends on the order the triples were presented in. Refused
    /// fail-closed — carries the number of distinct subjects found.
    AmbiguousSubject(usize),
    /// The subject WebID is inside the reserved `urn:sparq:` principal space (or carries the
    /// pair-principal delimiter) and could impersonate a minted principal.
    ReservedSubject(String),
}

#[cfg(feature = "acp-vc")]
impl std::fmt::Display for VcAdmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VcAdmitError::Proof(e) => write!(f, "credential proof did not verify: {}", e),
            VcAdmitError::NoSubject => {
                write!(f, "credential names no <{}> subject", CREDENTIAL_SUBJECT)
            }
            VcAdmitError::AmbiguousSubject(n) => write!(
                f,
                "credential names {} distinct <{}> subjects, so the holding would be ambiguous",
                n, CREDENTIAL_SUBJECT
            ),
            VcAdmitError::ReservedSubject(s) => {
                write!(f, "credential subject <{}> is in the reserved principal space", s)
            }
        }
    }
}

#[cfg(feature = "acp-vc")]
impl std::error::Error for VcAdmitError {}

/// The W3C VC `credentialSubject` predicate — the seam that binds a credential to the
/// agent whose `acp:vc` holdings it establishes.
#[cfg(feature = "acp-vc")]
pub(crate) const CREDENTIAL_SUBJECT: &str = "https://www.w3.org/2018/credentials#credentialSubject";
/// `rdf:type`, used to check a requirement's `credential_type`.
#[cfg(feature = "acp-vc")]
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

#[cfg(feature = "acp-vc")]
#[cfg_attr(docsrs, doc(cfg(feature = "acp-vc")))]
impl VerifiedCredentials {
    /// Verify a presented credential's W3C Data Integrity proof and record every requirement
    /// in `requirements` that it satisfies for its subject. Returns the requirement IRIs
    /// recorded (empty = the proof verified but no requirement matched).
    ///
    /// `credential` is the credential as RDF triples — the form the `eddsa-rdfc-2022` proof
    /// binds to (RDFC-1.0 canonical N-Quads); transforming a JSON-LD credential to RDF is the
    /// caller's job, as it is for [`sparq_vc::verify`] itself. `resolver` resolves the proof's
    /// `verificationMethod` (use [`sparq_vc::did::DidKeyResolver`] for offline `did:key`).
    ///
    /// A requirement is satisfied iff **all** of:
    /// * the proof verifies over `credential` (Ed25519 over the RDFC-1.0 canonical form);
    /// * the credential graph names exactly **one** distinct `credentialSubject` — the proof
    ///   binds to the graph's canonical form, so a credential with two subjects would bind
    ///   its holding to whichever one was presented first, and is refused instead;
    /// * the verified `verificationMethod` is the requirement's `issuer` DID, or a fragment
    ///   of it (`did:key:z6Mk…#z6Mk…`) — i.e. the accepted issuer actually signed;
    /// * the credential subject carries `rdf:type <credential_type>`;
    /// * the subject carries every exact-match claim the requirement added.
    ///
    /// # Honest scope
    ///
    /// This is **trust-the-issuer**: authenticity, integrity and non-repudiation only. It
    /// reveals the whole credential to the verifier and makes **no** privacy,
    /// unlinkability, zero-knowledge or selective-disclosure claim — those would need the ZK
    /// estate, whose external cryptographic audit is still pending (`sq-qhy4`), so no ZK
    /// backend is wired here. Revocation and credential expiry are **not** checked: this call
    /// establishes a holding at the instant it is made, and the caller controls its lifetime
    /// by choosing when to re-materialize (design record §2 open question 3).
    ///
    /// # Errors
    ///
    /// [`VcAdmitError::Proof`] if the signature/issuer resolution fails,
    /// [`VcAdmitError::NoSubject`] if the credential names no `credentialSubject` IRI,
    /// [`VcAdmitError::AmbiguousSubject`] if it names more than one distinct
    /// `credentialSubject`, and [`VcAdmitError::ReservedSubject`] if that subject is inside
    /// the reserved `urn:sparq:` principal space.
    ///
    /// Every one of these refuses the WHOLE credential: no holding is recorded for any
    /// requirement, so an ambiguous or reserved-space presentation cannot half-succeed.
    pub fn admit_data_integrity<R: sparq_vc::did::DidResolver>(
        &mut self,
        credential: &[oxrdf::Triple],
        proof: &sparq_vc::DataIntegrityProof,
        resolver: &R,
        requirements: &[VcRequirement],
    ) -> Result<Vec<String>, VcAdmitError> {
        let verified =
            sparq_vc::verify(credential, proof, resolver).map_err(VcAdmitError::Proof)?;
        let subject = credential_subject(credential)?;
        if !crate::loader::session_value_allowed(&subject) {
            return Err(VcAdmitError::ReservedSubject(subject));
        }
        let signer = issuer_did(&verified.verification_method);
        let mut recorded = Vec::new();
        for req in requirements {
            if signer != req.issuer {
                continue;
            }
            if !subject_has(credential, &subject, RDF_TYPE, &req.credential_type) {
                continue;
            }
            if !req
                .claims
                .iter()
                .all(|(p, o)| subject_has_term(credential, &subject, p, o))
            {
                continue;
            }
            self.hold(subject.clone(), req.requirement.clone());
            recorded.push(req.requirement.clone());
        }
        Ok(recorded)
    }
}

/// The credential's one `credentialSubject` IRI — refused unless the graph names exactly one
/// distinct subject.
///
/// Order-independence is a *security* property here, not a tidiness one: the
/// `eddsa-rdfc-2022` proof binds to the RDFC-1.0 CANONICAL form of the graph, so one signed
/// credential still verifies when its triples are presented in any order. Taking the FIRST
/// `credentialSubject` object would therefore let a re-ordered presentation of the *same*
/// valid credential bind the holding to a different agent. So every distinct object of the
/// predicate is collected and anything but exactly one is an error.
///
/// A non-IRI object (blank node, literal) still counts as a distinct subject: it names no
/// bindable agent, but counting it keeps a mixed graph AMBIGUOUS rather than silently
/// binding to whichever object happens to be a named node.
#[cfg(feature = "acp-vc")]
fn credential_subject(credential: &[oxrdf::Triple]) -> Result<String, VcAdmitError> {
    let mut distinct: Vec<&oxrdf::Term> = Vec::new();
    for t in credential {
        if t.predicate.as_str() == CREDENTIAL_SUBJECT && !distinct.contains(&&t.object) {
            distinct.push(&t.object);
        }
    }
    match distinct.as_slice() {
        [oxrdf::Term::NamedNode(o)] => Ok(o.as_str().to_owned()),
        // none at all, or a single object that is not an IRI to bind a holding to
        [] | [_] => Err(VcAdmitError::NoSubject),
        many => Err(VcAdmitError::AmbiguousSubject(many.len())),
    }
}

/// The issuer DID a `verificationMethod` belongs to — the part before any `#fragment`.
#[cfg(feature = "acp-vc")]
fn issuer_did(verification_method: &str) -> &str {
    verification_method.split('#').next().unwrap_or(verification_method)
}

/// Whether `subject` carries `predicate` with the named-node object `object`.
#[cfg(feature = "acp-vc")]
fn subject_has(
    credential: &[oxrdf::Triple],
    subject: &str,
    predicate: &str,
    object: &str,
) -> bool {
    subject_has_term(
        credential,
        subject,
        predicate,
        &oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(object)),
    )
}

/// Whether `subject` carries `predicate` with exactly the term `object`.
#[cfg(feature = "acp-vc")]
fn subject_has_term(
    credential: &[oxrdf::Triple],
    subject: &str,
    predicate: &str,
    object: &oxrdf::Term,
) -> bool {
    credential.iter().any(|t| {
        matches!(&t.subject, oxrdf::NamedOrBlankNode::NamedNode(s) if s.as_str() == subject)
            && t.predicate.as_str() == predicate
            && &t.object == object
    })
}

#[cfg(test)]
mod tests {
    // The `iter()` accessor is `pub(crate)` (consumed by the loader), so the trusted-channel
    // behaviour can only be exercised from an in-crate test. [SONNET-4.6] sq-ysv3u.
    use super::*;

    #[test]
    fn empty_map_holds_nothing() {
        let creds = VerifiedCredentials::new();
        assert!(creds.is_empty());
        assert!(!creds.holds("https://alice.ex/card#me", "https://req.ex/r"));
        assert_eq!(creds.iter().count(), 0);
    }

    #[test]
    fn holdings_are_per_agent_and_accumulate() {
        let mut creds = VerifiedCredentials::new();
        creds.hold("https://alice.ex/card#me", "https://req.ex/r1");
        creds.hold("https://alice.ex/card#me", "https://req.ex/r2");
        creds.hold("https://bob.ex/card#me", "https://req.ex/r1");
        assert!(!creds.is_empty());
        assert!(creds.holds("https://alice.ex/card#me", "https://req.ex/r1"));
        assert!(creds.holds("https://alice.ex/card#me", "https://req.ex/r2"));
        // bob's holding does NOT leak alice's second requirement
        assert!(creds.holds("https://bob.ex/card#me", "https://req.ex/r1"));
        assert!(!creds.holds("https://bob.ex/card#me", "https://req.ex/r2"));
        assert_eq!(creds.iter().count(), 3);
    }

    #[test]
    fn hold_is_idempotent() {
        let mut creds = VerifiedCredentials::new();
        creds.hold("https://alice.ex/card#me", "https://req.ex/r1");
        creds.hold(String::from("https://alice.ex/card#me"), String::from("https://req.ex/r1"));
        assert_eq!(creds.iter().count(), 1);
    }

    #[test]
    fn iter_yields_every_agent_requirement_pair() {
        let mut creds = VerifiedCredentials::new();
        creds.hold("https://alice.ex/card#me", "https://req.ex/r1");
        creds.hold("https://bob.ex/card#me", "https://req.ex/r2");
        let mut seen: Vec<(String, String)> =
            creds.iter().map(|(a, r)| (a.to_owned(), r.to_owned())).collect();
        seen.sort();
        assert_eq!(
            seen,
            vec![
                ("https://alice.ex/card#me".to_owned(), "https://req.ex/r1".to_owned()),
                ("https://bob.ex/card#me".to_owned(), "https://req.ex/r2".to_owned()),
            ]
        );
    }

    #[cfg(feature = "acp-vc")]
    fn subject_triple(object: oxrdf::Term) -> oxrdf::Triple {
        oxrdf::Triple::new(
            oxrdf::NamedOrBlankNode::NamedNode(oxrdf::NamedNode::new_unchecked(
                "https://issuer.ex/creds/1",
            )),
            oxrdf::NamedNode::new_unchecked(CREDENTIAL_SUBJECT),
            object,
        )
    }

    #[cfg(feature = "acp-vc")]
    fn named(iri: &str) -> oxrdf::Term {
        oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(iri))
    }

    #[cfg(feature = "acp-vc")]
    #[test]
    fn one_credential_subject_binds_however_often_it_is_repeated() {
        let alice = subject_triple(named("https://alice.ex/card#me"));
        assert_eq!(
            credential_subject(std::slice::from_ref(&alice)).expect("one subject"),
            "https://alice.ex/card#me"
        );
        // the SAME subject stated twice is still unambiguous
        assert_eq!(
            credential_subject(&[alice.clone(), alice]).expect("one distinct subject"),
            "https://alice.ex/card#me"
        );
    }

    #[cfg(feature = "acp-vc")]
    #[test]
    fn two_distinct_credential_subjects_are_refused_in_either_order() {
        let alice = subject_triple(named("https://alice.ex/card#me"));
        let bob = subject_triple(named("https://bob.ex/card#me"));
        // Both slice orders below are the same GRAPH, so the binding must not depend on which
        // one is presented. (That a real SIGNED credential does verify under either order is
        // pinned end-to-end by `tests/acp_vc_backend.rs`, which carries an actual proof.)
        for order in [vec![alice.clone(), bob.clone()], vec![bob, alice]] {
            assert!(
                matches!(credential_subject(&order), Err(VcAdmitError::AmbiguousSubject(2))),
                "an ambiguous subject must be refused, not resolved by slice order"
            );
        }
    }

    #[cfg(feature = "acp-vc")]
    #[test]
    fn a_non_iri_subject_binds_nothing_and_still_counts_toward_ambiguity() {
        let literal = subject_triple(oxrdf::Term::Literal(oxrdf::Literal::new_simple_literal(
            "not-a-webid",
        )));
        assert!(matches!(
            credential_subject(std::slice::from_ref(&literal)),
            Err(VcAdmitError::NoSubject)
        ));
        // a literal alongside an IRI is AMBIGUOUS, not "the IRI wins"
        let alice = subject_triple(named("https://alice.ex/card#me"));
        assert!(matches!(
            credential_subject(&[literal, alice]),
            Err(VcAdmitError::AmbiguousSubject(2))
        ));
    }

    #[cfg(feature = "acp-vc")]
    #[test]
    fn a_credential_with_no_subject_triple_is_refused() {
        assert!(matches!(credential_subject(&[]), Err(VcAdmitError::NoSubject)));
    }

    #[cfg(feature = "acp-vc")]
    #[test]
    fn issuer_did_strips_the_verification_method_fragment() {
        assert_eq!(issuer_did("did:key:z6Mk#z6Mk"), "did:key:z6Mk");
        assert_eq!(issuer_did("did:key:z6Mk"), "did:key:z6Mk");
    }
}
