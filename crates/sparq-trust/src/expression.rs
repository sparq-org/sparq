//! # `expression` — holder-side trust-expression contract evaluation (clear path)
//!
//! The runnable clear-path realisation of the verifier→holder trust-expression
//! contract (issue #1592; design record `research/trust-expression-spec.md`
//! §3.1–3.5, bead `sq-6syab.4`). The verifier sends exactly three things — a
//! SPARQL query `Q` (ASK or SELECT), a **trust-requirements document** `TR` (a
//! small RDF graph in the [`crate::framework_vocab`] `trustx:` vocabulary), and a
//! **nonce** ([`ChallengeNonce`] — the zkSPARQL challenge-response mechanism,
//! reused verbatim — no new freshness scheme). This module:
//!
//! 1. **parses the request** ([`parse_request`]) — fail-closed on a missing /
//!    duplicated requirements node, a missing question IRI, a missing status
//!    instant, a malformed `did:` issuer (validated through the [`crate::did`]
//!    binding layer), or a requirements document with **no trust mode** (which
//!    would admit nothing and is refused up front rather than silently evaluated);
//! 2. **generates the §3.1 normative reference rewrite `Q → Q'`**
//!    ([`rewrite_query`]) — each of `Q`'s triple patterns is wrapped in a
//!    `GRAPH ?g { … }` over the holder's attestation bundles and conjoined with
//!    the admissibility patterns generated from `TR`: **issuer membership**
//!    (mode 1, enumerated parties), **positive status-attestation validity at
//!    *t*** (design D3 — "unrevoked" is the *existence* of a covering
//!    `trustx:StatusAttestation` window, never evidence-of-absence), and
//!    **certification-scope conformance** (mode 2, framework-certified issuers —
//!    the "issued only what they are certified to issue" check of design D4).
//!    The two modes compose by plain `UNION` (design D2);
//! 3. **evaluates the contract** ([`evaluate_contract`]) — runs `Q'` via
//!    `sparq-engine` over the holder's attested dataset (one named graph per
//!    attestation bundle; provenance in the default graph), after the optional
//!    `trustx:methodPolicy` ODRL pre-check through the EXISTING
//!    [`crate::admissibility::admissible`] reduction (design D5: which *proof
//!    methods* are acceptable is an orthogonal axis to which *sources* are);
//! 4. **assembles the provenance-encoded response** `R` — the RDF 1.2 **reifier
//!    normative form** (`_:r rdf:reifies <<( s p o )>>` + PROV-O/`trustx:`
//!    qualification on the reifier; design §4 option (a)) AND the mechanically
//!    lossless **named-graph + PROV-O mapping** (option (b), reifier node ↔ graph
//!    IRI) that is runnable on every SPARQL 1.1 engine today — including this
//!    one, whose SPARQL surface cannot yet *match* triple terms;
//! 5. **re-checks like a verifier** ([`verify_response`]) — the invariant the
//!    bead pins: the response provenance is sufficient for an INDEPENDENT
//!    verifier to re-run `Q'` over `R` (the (b) form) and reproduce the answer.
//!
//! The status-attestation bridge [`mint_status_attestation`] turns a verified
//! [`crate::status_list`] live-status check (the merged signed Bitstring
//! machinery) into the positive, time-windowed `trustx:StatusAttestation`
//! triples the rewrite consumes — reuse, not duplication, of the P6 stratum
//! (fail-closed: only a [`crate::status_list::LiveStatus::Live`] verdict can
//! mint an attestation).
//!
//! ## Fail-closed invariant (load-bearing)
//!
//! **No admissible derivation ⇒ no binding.** A revoked / stale-windowed /
//! untrusted-issuer / scope-violating / uncovered contributing statement never
//! yields a `Q'` solution — the answer is simply `false` / zero rows and the
//! response carries **zero** bundles. There is no negation anywhere: every check
//! is the monotone existence of a positive attestation (OWA — the agreed #1592
//! constraint), so "reject" is always the *absence of an admissible derivation*,
//! never a derived denial.
//!
//! ## Supported query fragment (v1, fail-closed)
//!
//! `Q` must be an ASK or SELECT (optionally DISTINCT) over **one basic graph
//! pattern** — no property paths, FILTER, OPTIONAL, UNION, dataset clauses,
//! blank-node patterns, or RDF 1.2 triple-term patterns, and no variables in the
//! reserved `?__tx_*` namespace the rewrite mints. Anything outside the fragment
//! is REFUSED ([`ExpressionError::UnsupportedQuery`]), never partially evaluated.
//! The maintainer's eIDAS example question is an ASK in this fragment.
//!
//! ## Honest scope
//!
//! This is the CLEAR path: the verifier re-checks admissibility over `R` but
//! must trust the underlying attestations' signatures and the completeness of
//! what the holder disclosed (design §7.3). The `trustx:methodPolicy` pre-check
//! is IRI-bound to `TR` (a [`MethodPrecheck`] for a different policy is
//! refused), but resolving the named IRI into the policy's constraints remains
//! caller-owned — see the [`MethodPrecheck`] trust boundary. `trustx:question`
//! is an **opaque label, not an enforced binding**: parsing checks that `TR`
//! names exactly one question IRI, but nothing at this layer resolves or
//! compares that IRI against `Q` (there is no canonical question-IRI → query
//! resolution or digest scheme to check it against), so a `TR` authored for one
//! question paired with a different supported query is accepted here. Verifying
//! that `Q` *is* the named question — e.g. a signature over the whole
//! `(Q, TR, nonce)` request, or a trusted publication resolving the question
//! IRI to a canonical query — is caller-owned, exactly like the
//! [`MethodPrecheck`] resolution (design §7.7). Nonce *freshness* is likewise
//! caller-owned: [`ChallengeNonce`] makes a verifier's choice of a constant
//! visible at the construction site (issue #4621) but cannot detect one.
//! Framework trust is **anchored, not proven** (§7.2). No ZK claim is made
//! here; the ZK realisation is bead
//! `sq-6syab.5` on `sparq-zk-compose`, and the sparq ZK estate remains
//! internally re-audited with **external accredited-cryptographer sign-off
//! PENDING** (`sq-qhy4`); `sparq-mpc` is honest-majority semi-honest only.
//!
//! [FABLE-5] sq-6syab.4 (epic sq-6syab; issue #1592). 🤖 SPARQ agent —
//! trust-expression holder-side contract evaluation.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;

use oxrdf::vocab::{rdf, xsd};
use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use spargebra::algebra::GraphPattern;
use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern, Variable};
use spargebra::{Query, SparqlParser};
use sparq_core::Graph;

use crate::did::Did;
use crate::framework_vocab::{
    TRUSTX_ANY_SERVICE_SCOPE, TRUSTX_CERTIFICATION, TRUSTX_CERTIFICATION_SCOPE, TRUSTX_CERTIFIES,
    TRUSTX_COVERED_BY, TRUSTX_METHOD_POLICY, TRUSTX_QUESTION, TRUSTX_REQUIRES_SCOPE_CONFORMANCE,
    TRUSTX_REQUIRES_VALID_STATUS_AT, TRUSTX_STATUS_ATTESTATION, TRUSTX_TRUSTS_FRAMEWORK,
    TRUSTX_TRUSTS_ISSUER, TRUSTX_TRUST_REQUIREMENTS, TRUSTX_UNDER_FRAMEWORK, TRUSTX_VALID_FROM,
    TRUSTX_VALID_UNTIL,
};
use crate::status_list::LiveStatus;

/// PROV-O `prov:wasAttributedTo` — the qualification linking an attestation
/// bundle (reifier node / graph IRI) to the issuer identity that attested its
/// statements (design §4: PROV-O on the reifier).
pub const PROV_WAS_ATTRIBUTED_TO: &str = "http://www.w3.org/ns/prov#wasAttributedTo";
/// RDF 1.2 `rdf:reifies` — links a reifier node to the triple term it reifies in
/// the normative response encoding (design §4 option (a)).
pub const RDF_REIFIES: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies";

/// The reserved variable-name prefix the rewrite mints (`?__tx_g0`, `?__tx_st0`,
/// …). A query using it is refused so `Q`'s own bindings can never collide with
/// — or observe — the conjoined admissibility machinery.
const RESERVED_VAR_PREFIX: &str = "__tx_";

// ─────────────────────────────── errors ───────────────────────────────

/// Why a request / rewrite / evaluation was refused. Every variant is a
/// **fail-closed** outcome: nothing is evaluated and no response is produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpressionError {
    /// The nonce is empty — the challenge-response freshness binding (reused
    /// verbatim from the zkSPARQL contract) is mandatory. Raised by
    /// [`ChallengeNonce::from_wire`], the only path by which an outside value
    /// becomes a nonce.
    EmptyNonce,
    /// [`ChallengeNonce::generate`] could not draw from the OS CSPRNG — fail
    /// closed rather than mint a challenge nonce from a weaker source.
    NoEntropy,
    /// The query string is empty.
    EmptyQuery,
    /// The requirements graph declares no `trustx:TrustRequirements` node.
    NoRequirements,
    /// The requirements graph declares more than one `trustx:TrustRequirements`
    /// node — v1 carries exactly one (OR composes *within* it, design D2).
    MultipleRequirements,
    /// The requirements node has no (or a non-IRI) `trustx:question` value.
    MissingQuestion,
    /// The requirements node has no `trustx:requiresValidStatusAt` instant —
    /// without *t* there is no status check to conjoin.
    MissingValidStatusAt,
    /// A dateTime is not a UTC `xsd:dateTime` lexical (`…Z`); the offending
    /// lexical is carried.
    BadDateTime(String),
    /// A `trustx:requiresScopeConformance` value is not an `xsd:boolean`.
    BadBoolean(String),
    /// A `trustx:trustsIssuer` in the `did:` scheme failed DID syntax validation
    /// (through [`crate::did::Did::parse`] — the sq-pfae.3 binding layer).
    BadIssuer(String),
    /// A requirements property has the wrong term kind (the property IRI is
    /// carried).
    BadTerm(String),
    /// The requirements name neither enumerated issuers nor a framework: such a
    /// document admits nothing and is refused up front.
    NoTrustMode,
    /// An IRI to be embedded in the rewrite contains characters that could
    /// escape its `<…>` delimiters (injection guard; the IRI is carried).
    UnsafeIri(String),
    /// `Q` failed to parse as SPARQL.
    QueryParse(String),
    /// `Q` parsed but falls outside the supported v1 fragment (reason carried).
    UnsupportedQuery(String),
    /// `Q` uses a variable in the reserved `?__tx_*` namespace.
    ReservedVariable(String),
    /// `TR` names a `trustx:methodPolicy` but no [`MethodPrecheck`] data was
    /// supplied — fail-closed: the policy cannot be silently skipped.
    MethodPolicyWithoutPrecheck,
    /// The supplied [`MethodPrecheck`] resolves a DIFFERENT policy IRI than the
    /// `trustx:methodPolicy` named by `TR` — fail-closed: the named policy can
    /// never be silently substituted with a weaker one.
    MethodPolicyMismatch {
        /// The policy IRI `TR` names.
        required: String,
        /// The policy IRI the supplied pre-check data resolves.
        supplied: String,
    },
    /// The ODRL method pre-check ran and the presented method does NOT satisfy
    /// the policy (the unsatisfied constraint IRIs are carried).
    MethodNotAdmissible(Vec<String>),
    /// The admissibility reasoner itself failed (parse/closure error).
    Admissibility(String),
    /// The SPARQL engine refused `Q'` / the extraction query.
    Engine(String),
    /// The response dataset could not be parsed for the verifier re-check.
    Response(String),
    /// The response's nonce does not match the request's.
    NonceMismatch,
    /// [`mint_status_attestation`] was handed a non-[`LiveStatus::Live`] verdict
    /// (the fail-closed reason token is carried) — a positive attestation can
    /// only be minted from a verified-live check.
    NonPositiveStatus(&'static str),
    /// [`mint_status_attestation`] was handed a negative validity window.
    BadWindow,
}

impl fmt::Display for ExpressionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyNonce => write!(f, "empty nonce (challenge-response binding is mandatory)"),
            Self::NoEntropy => {
                write!(f, "no OS entropy source available to draw a fresh challenge nonce")
            }
            Self::EmptyQuery => write!(f, "empty query"),
            Self::NoRequirements => {
                write!(f, "no trustx:TrustRequirements node in the requirements graph")
            }
            Self::MultipleRequirements => {
                write!(f, "more than one trustx:TrustRequirements node (v1 carries exactly one)")
            }
            Self::MissingQuestion => write!(f, "missing or non-IRI trustx:question value"),
            Self::MissingValidStatusAt => {
                write!(f, "missing trustx:requiresValidStatusAt instant")
            }
            Self::BadDateTime(lex) => {
                write!(f, "not a UTC xsd:dateTime lexical: {}", lex)
            }
            Self::BadBoolean(lex) => write!(f, "not an xsd:boolean lexical: {}", lex),
            Self::BadIssuer(iri) => write!(f, "malformed did: issuer identity: {}", iri),
            Self::BadTerm(prop) => write!(f, "wrong term kind for property {}", prop),
            Self::NoTrustMode => write!(
                f,
                "requirements name neither trustsIssuer nor trustsFramework (admits nothing)"
            ),
            Self::UnsafeIri(iri) => write!(f, "IRI unsafe to embed in a query: {}", iri),
            Self::QueryParse(e) => write!(f, "query parse error: {}", e),
            Self::UnsupportedQuery(why) => {
                write!(f, "query outside the supported v1 fragment: {}", why)
            }
            Self::ReservedVariable(v) => {
                write!(f, "query uses reserved variable namespace: ?{}", v)
            }
            Self::MethodPolicyWithoutPrecheck => write!(
                f,
                "TR names a trustx:methodPolicy but no method pre-check data was supplied"
            ),
            Self::MethodPolicyMismatch { required, supplied } => write!(
                f,
                "method pre-check resolves policy {} but TR names trustx:methodPolicy {}",
                supplied, required
            ),
            Self::MethodNotAdmissible(unsat) => write!(
                f,
                "presented method fails the ODRL method policy ({} unsatisfied constraint(s))",
                unsat.len()
            ),
            Self::Admissibility(e) => write!(f, "admissibility reasoner error: {}", e),
            Self::Engine(e) => write!(f, "engine error: {}", e),
            Self::Response(e) => write!(f, "response dataset error: {}", e),
            Self::NonceMismatch => write!(f, "response nonce does not match the request"),
            Self::NonPositiveStatus(reason) => write!(
                f,
                "refusing to mint a positive status attestation from a non-live check: {}",
                reason
            ),
            Self::BadWindow => write!(f, "negative status-attestation validity window"),
        }
    }
}

impl std::error::Error for ExpressionError {}

// ─────────────────────────── the challenge nonce ────────────────────────────

/// The verifier's challenge nonce — the zkSPARQL challenge-response freshness
/// binding, carried as a type that makes the *provenance* of the value visible
/// at every construction site.
///
/// Replay protection here rests entirely on the nonce being **freshly
/// generated and unpredictable per request**. That obligation used to live
/// only in prose on [`parse_request`]'s `nonce: &str` parameter, so a verifier
/// could pass a compile-time constant, silently void the freshness guarantee,
/// and have nothing in the crate object (issue #4621). There are now exactly
/// two ways to obtain a nonce, and which one a call site reached for is
/// readable in the source:
///
/// * [`ChallengeNonce::generate`] — the **verifier-side** path: bytes drawn
///   from the OS CSPRNG. The only constructor that *creates* freshness.
/// * [`ChallengeNonce::from_wire`] — the **adopt-an-outside-value** path, for
///   a nonce that legitimately arrives from elsewhere (the holder echoing a
///   challenge, a response decoded off the wire, a verifier whose session
///   nonce is minted by a layer above this crate). It is named for what it
///   does: it asserts nothing about freshness, so a call site that reaches for
///   it to wrap a literal is *visibly* opting out of the guarantee.
///
/// ## Honest scope
///
/// This is a **hardening** measure, not a soundness claim in either direction.
/// The type relocates the freshness obligation from prose into the
/// construction site so that voiding it is explicit rather than invisible; it
/// cannot *detect* a caller that feeds [`from_wire`](Self::from_wire) a
/// constant, and it changes no protocol, wire format, or verifier check. The
/// sparq ZK/trust estate remains internally re-audited with external
/// accredited-cryptographer sign-off PENDING (`sq-qhy4`).
///
/// ```
/// # use sparq_trust::expression::ChallengeNonce;
/// let fresh = ChallengeNonce::generate().expect("OS entropy");
/// assert_ne!(fresh, ChallengeNonce::generate().expect("OS entropy"));
///
/// // A value that came from outside has to say so.
/// let echoed = ChallengeNonce::from_wire(fresh.as_str()).expect("non-empty");
/// assert_eq!(echoed, fresh);
/// ```
///
/// The call shape [`parse_request`] accepts, for contrast with the one below:
///
/// ```
/// # use sparq_trust::expression::{parse_request, ChallengeNonce, ExpressionError};
/// let nonce = ChallengeNonce::generate().expect("OS entropy");
/// assert_eq!(
///     parse_request("ASK { ?s ?p ?o }", &[], &nonce),
///     Err(ExpressionError::NoRequirements),
/// );
/// ```
///
/// A bare `&str` literal is not a nonce — the same call does not compile:
///
/// ```compile_fail
/// # use sparq_trust::expression::parse_request;
/// let _ = parse_request("ASK { ?s ?p ?o }", &[], "a-constant-nonce");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChallengeNonce(String);

impl ChallengeNonce {
    /// How many CSPRNG bytes [`Self::generate`] draws (256 bits, rendered as
    /// lowercase hex into the wire value).
    pub const ENTROPY_BYTES: usize = 32;

    /// Draw a FRESH, unpredictable challenge nonce from the OS CSPRNG — the
    /// verifier-side construction path. [`Self::ENTROPY_BYTES`] bytes are
    /// filled from the platform entropy source and hex-encoded.
    ///
    /// Fails closed with [`ExpressionError::NoEntropy`] when that source is
    /// unavailable: a nonce is never fabricated from a weaker fallback.
    pub fn generate() -> Result<Self, ExpressionError> {
        let mut bytes = [0u8; Self::ENTROPY_BYTES];
        // The OS CSPRNG — the same primitive (and the same `fill()` API of the
        // getrandom 0.4 line) the sparq-zk salt mint draws its per-graph salt
        // from. An error here means the platform has no entropy source.
        getrandom::fill(&mut bytes).map_err(|_| ExpressionError::NoEntropy)?;
        // The crate's existing lowercase-hex encoder (`expression` implies `did`).
        Ok(Self(crate::did::to_hex(&bytes)))
    }

    /// Adopt a nonce that arrived from OUTSIDE this crate — the holder echoing
    /// a challenge, a response decoded off the wire, or a verifier whose
    /// session nonce is minted a layer above. **Asserts nothing about
    /// freshness**: the name is the signal that the obligation stays with the
    /// caller here, exactly as it did for the old `&str` parameter.
    ///
    /// The only check is the one [`parse_request`] used to make: an empty (or
    /// all-whitespace) value is refused with [`ExpressionError::EmptyNonce`],
    /// because the challenge-response binding is mandatory. There is
    /// deliberately no minimum-length or entropy heuristic — it would reject
    /// `"n"` while happily accepting a 32-byte constant, buying false
    /// confidence rather than protection.
    pub fn from_wire(value: &str) -> Result<Self, ExpressionError> {
        if value.trim().is_empty() {
            return Err(ExpressionError::EmptyNonce);
        }
        Ok(Self(value.to_string()))
    }

    /// The nonce's wire value — what is transported to the holder and echoed
    /// back in the response.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ChallengeNonce {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

// ─────────────────────────────── request ────────────────────────────────

/// The parsed verifier→holder contract carrier: the SPARQL query `Q`, the
/// trust-requirements document `TR` (parsed), and the challenge nonce — the
/// three things design §3.1 (D1) says the verifier sends, nothing more. The
/// wire mechanism for the nonce is the zkSPARQL challenge-response, reused
/// verbatim; this struct is the in-memory form after transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractRequest {
    /// The SPARQL query `Q` (ASK or SELECT; see the module docs for the
    /// supported v1 fragment).
    pub query: String,
    /// The verifier's challenge nonce (freshness binding; echoed in the
    /// response and checked by [`verify_response`]). A verifier minting a new
    /// challenge MUST build this with [`ChallengeNonce::generate`] — see that
    /// type for why reuse voids replay protection and what
    /// [`ChallengeNonce::from_wire`] does and does not promise.
    pub nonce: ChallengeNonce,
    /// The parsed trust-requirements document `TR`.
    pub requirements: TrustRequirements,
}

/// The parsed `trustx:TrustRequirements` document — the verifier's trust
/// conditions, which live HERE and never in the query (design D1: no new query
/// syntax).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustRequirements {
    /// The `trustx:question` IRI — an **opaque label** naming the question
    /// `TR` was authored for. Parse-time checks presence and IRI-ness only;
    /// this layer never resolves or compares it against `Q`, so the
    /// question↔query association is a caller-owned trust boundary (see the
    /// module docs' honest scope).
    pub question: NamedNode,
    /// **Mode 1** (enumerated parties): the issuer identities whose attributed
    /// statements are admissible. `did:` IRIs are syntax-validated through
    /// [`crate::did::Did::parse`] at request-parse time.
    pub trusted_issuers: Vec<NamedNode>,
    /// **Mode 2** (framework-certified issuers): the frameworks whose valid,
    /// status-covered certifications admit an issuer. Composes with mode 1 by
    /// plain OR (design D2).
    pub trusted_frameworks: Vec<NamedNode>,
    /// Whether mode 2 additionally requires every contributing statement to
    /// fall under its issuer's certification scope (the "only issued what they
    /// are certified to issue" check). Defaults to **true** when absent — the
    /// stricter, fail-closed reading.
    pub requires_scope_conformance: bool,
    /// The instant *t* (a UTC `xsd:dateTime` lexical) at which every covering
    /// positive status attestation — and every mode-2 certification window —
    /// must be valid. A verifier-chosen parameter, never a spec constant.
    pub valid_status_at: String,
    /// Optional `trustx:methodPolicy` — an ODRL policy the presented proof
    /// method must satisfy via the EXISTING [`crate::admissibility::admissible`]
    /// pre-check before any evaluation (design D5).
    pub method_policy: Option<NamedNode>,
}

/// The caller-resolved inputs for the optional `trustx:methodPolicy` ODRL
/// pre-check — the policy IRI the data resolves plus the four arguments the
/// existing [`crate::admissibility::admissible`] reduction takes. The holder
/// resolves the policy IRI named in `TR` into these (the policy constraints as
/// N3, the presented method's secprop annotations) and [`evaluate_contract`]
/// runs the UNCHANGED reduction — reuse, not restatement, of the sq-0dksu
/// machinery.
///
/// ## Trust boundary (read before relying on the pre-check)
///
/// [`evaluate_contract`] BINDS the pre-check to `TR`: [`Self::policy`] must
/// equal the `trustx:methodPolicy` IRI the request names, else the whole
/// evaluation is refused ([`ExpressionError::MethodPolicyMismatch`]) — a
/// pre-check resolved for a different (e.g. weaker) policy can never be
/// substituted. What remains **caller-owned** is the *resolution itself*:
/// nothing here authenticates that [`Self::policy_n3`] /
/// [`Self::constraint_iris`] are the faithful dereference of that IRI. The
/// caller must resolve the named policy from a source it trusts (and the
/// verifier's own re-check of the method-policy axis is out of scope for the
/// clear-path [`verify_response`], which re-checks the data-admissibility axis
/// only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodPrecheck<'a> {
    /// The IRI of the policy this pre-check data resolves. MUST equal the
    /// `trustx:methodPolicy` IRI named by `TR` (checked fail-closed by
    /// [`evaluate_contract`]).
    pub policy: &'a str,
    /// The presented proof method's IRI (for the clear path, the method
    /// describing clear disclosure).
    pub method: &'a str,
    /// The policy's `odrl:Constraint` IRIs (every one must be satisfied —
    /// default-deny).
    pub constraint_iris: &'a [&'a str],
    /// The policy constraints as an N3/Turtle fragment (no `@prefix` preamble).
    pub policy_n3: &'a str,
    /// The method's `secx:hasProperty` annotation graph (no `@prefix` preamble).
    pub annotations: &'a str,
}

/// Parse a request: the query string `Q`, the trust-requirements graph `TR`
/// (as parsed RDF triples — the same input shape as [`crate::policy::parse_policy`]),
/// and the [`ChallengeNonce`]. Fail-closed on every malformation (see
/// [`ExpressionError`]); notably a `TR` with **no trust mode** is refused here
/// rather than admitted as a vacuous evaluation, and every `did:`-scheme issuer
/// must pass [`crate::did::Did::parse`].
///
/// The nonce arrives already constructed, so the emptiness refusal that used to
/// live here now lives in [`ChallengeNonce::from_wire`] — and a verifier
/// minting a new challenge reaches for [`ChallengeNonce::generate`] instead.
/// Nothing at this layer can verify that a nonce is *fresh*; the type makes the
/// choice visible at the call site, which is as far as the obligation can be
/// moved (see [`ChallengeNonce`]'s honest scope).
pub fn parse_request(
    query: &str,
    requirements: &[Triple],
    nonce: &ChallengeNonce,
) -> Result<ContractRequest, ExpressionError> {
    if query.trim().is_empty() {
        return Err(ExpressionError::EmptyQuery);
    }

    // Exactly ONE trustx:TrustRequirements node (design D2: modes OR *within* it).
    let mut nodes: Vec<&NamedOrBlankNode> = Vec::new();
    for t in requirements {
        if t.predicate.as_ref() == rdf::TYPE
            && matches!(&t.object, Term::NamedNode(n) if n.as_str() == TRUSTX_TRUST_REQUIREMENTS)
            && !nodes.contains(&&t.subject)
        {
            nodes.push(&t.subject);
        }
    }
    let node: &NamedOrBlankNode = match nodes.as_slice() {
        [] => return Err(ExpressionError::NoRequirements),
        [one] => one,
        _ => return Err(ExpressionError::MultipleRequirements),
    };
    let objects_of = |prop: &'static str| {
        requirements
            .iter()
            .filter(move |t| &t.subject == node && t.predicate.as_str() == prop)
            .map(|t| &t.object)
    };

    // trustx:question — exactly one IRI.
    let mut questions = objects_of(TRUSTX_QUESTION);
    let question = match (questions.next(), questions.next()) {
        (Some(Term::NamedNode(n)), None) => n.clone(),
        (None, _) => return Err(ExpressionError::MissingQuestion),
        _ => return Err(ExpressionError::BadTerm(TRUSTX_QUESTION.to_string())),
    };

    // Mode 1 issuers — IRIs; did:-scheme identities are DID-syntax-validated
    // through the sq-pfae.3 binding layer (reuse, not re-derivation).
    let mut trusted_issuers = Vec::new();
    for o in objects_of(TRUSTX_TRUSTS_ISSUER) {
        let Term::NamedNode(n) = o else {
            return Err(ExpressionError::BadTerm(TRUSTX_TRUSTS_ISSUER.to_string()));
        };
        if n.as_str().starts_with("did:") && Did::parse(n.as_str()).is_none() {
            return Err(ExpressionError::BadIssuer(n.as_str().to_string()));
        }
        trusted_issuers.push(n.clone());
    }

    // Mode 2 frameworks — IRIs (e.g. the framework_vocab trustx:eIDAS2 / trustx:DIATF
    // individuals, which rdfs:seeAlso the vendored sec-req: instances).
    let mut trusted_frameworks = Vec::new();
    for o in objects_of(TRUSTX_TRUSTS_FRAMEWORK) {
        let Term::NamedNode(n) = o else {
            return Err(ExpressionError::BadTerm(TRUSTX_TRUSTS_FRAMEWORK.to_string()));
        };
        trusted_frameworks.push(n.clone());
    }

    if trusted_issuers.is_empty() && trusted_frameworks.is_empty() {
        return Err(ExpressionError::NoTrustMode);
    }

    // trustx:requiresValidStatusAt — exactly one UTC xsd:dateTime.
    let mut instants = objects_of(TRUSTX_REQUIRES_VALID_STATUS_AT);
    let valid_status_at = match (instants.next(), instants.next()) {
        (Some(Term::Literal(l)), None) => {
            let lex = l.value().to_string();
            if l.datatype() != xsd::DATE_TIME || !is_utc_datetime_lexical(&lex) {
                return Err(ExpressionError::BadDateTime(lex));
            }
            lex
        }
        (None, _) => return Err(ExpressionError::MissingValidStatusAt),
        _ => {
            return Err(ExpressionError::BadTerm(
                TRUSTX_REQUIRES_VALID_STATUS_AT.to_string(),
            ))
        }
    };

    // trustx:requiresScopeConformance — optional xsd:boolean; ABSENT ⇒ true
    // (the stricter, fail-closed default for mode 2).
    let mut scope_flags = objects_of(TRUSTX_REQUIRES_SCOPE_CONFORMANCE);
    let requires_scope_conformance = match (scope_flags.next(), scope_flags.next()) {
        (None, _) => true,
        (Some(Term::Literal(l)), None) if l.datatype() == xsd::BOOLEAN => match l.value() {
            "true" | "1" => true,
            "false" | "0" => false,
            other => return Err(ExpressionError::BadBoolean(other.to_string())),
        },
        (Some(Term::Literal(l)), None) => {
            return Err(ExpressionError::BadBoolean(l.value().to_string()))
        }
        _ => {
            return Err(ExpressionError::BadTerm(
                TRUSTX_REQUIRES_SCOPE_CONFORMANCE.to_string(),
            ))
        }
    };

    // trustx:methodPolicy — optional IRI.
    let mut policies = objects_of(TRUSTX_METHOD_POLICY);
    let method_policy = match (policies.next(), policies.next()) {
        (None, _) => None,
        (Some(Term::NamedNode(n)), None) => Some(n.clone()),
        _ => return Err(ExpressionError::BadTerm(TRUSTX_METHOD_POLICY.to_string())),
    };

    Ok(ContractRequest {
        query: query.to_string(),
        nonce: nonce.clone(),
        requirements: TrustRequirements {
            question,
            trusted_issuers,
            trusted_frameworks,
            requires_scope_conformance,
            valid_status_at,
            method_policy,
        },
    })
}

// ─────────────────────────── the reference rewrite ───────────────────────────

/// The supported query shapes after fragment scoping.
enum Form {
    Ask,
    Select { distinct: bool, vars: Vec<Variable> },
}

/// `Q` reduced to its fragment-scoped shape: the form + the single BGP.
struct Shape {
    form: Form,
    patterns: Vec<TriplePattern>,
}

fn unsupported(why: &str) -> ExpressionError {
    ExpressionError::UnsupportedQuery(why.to_string())
}

fn check_reserved(v: &Variable) -> Result<(), ExpressionError> {
    if v.as_str().starts_with(RESERVED_VAR_PREFIX) {
        return Err(ExpressionError::ReservedVariable(v.as_str().to_string()));
    }
    Ok(())
}

/// A pattern term admissible in the v1 fragment: IRI, literal, or a
/// non-reserved variable. Blank nodes are refused (they cannot be projected for
/// response assembly) and RDF 1.2 triple-term patterns are refused (the engine
/// cannot match them yet — design §7.5 names this gap honestly).
fn check_term_pattern(t: &TermPattern) -> Result<(), ExpressionError> {
    match t {
        TermPattern::NamedNode(_) | TermPattern::Literal(_) => Ok(()),
        TermPattern::Variable(v) => check_reserved(v),
        TermPattern::BlankNode(_) => Err(unsupported("blank-node patterns")),
        _ => Err(unsupported("RDF 1.2 triple-term patterns")),
    }
}

fn bgp_patterns(p: &GraphPattern) -> Result<Vec<TriplePattern>, ExpressionError> {
    let GraphPattern::Bgp { patterns } = p else {
        return Err(unsupported(
            "the v1 clear-path fragment is a single basic graph pattern",
        ));
    };
    if patterns.is_empty() {
        return Err(unsupported("empty basic graph pattern"));
    }
    for t in patterns {
        check_term_pattern(&t.subject)?;
        check_term_pattern(&t.object)?;
        if let NamedNodePattern::Variable(v) = &t.predicate {
            check_reserved(v)?;
        }
    }
    Ok(patterns.clone())
}

/// Parse `Q` and scope it to the supported fragment (fail-closed).
fn parse_shape(query: &str) -> Result<Shape, ExpressionError> {
    let q = SparqlParser::new()
        .parse_query(query)
        .map_err(|e| ExpressionError::QueryParse(e.to_string()))?;
    match q {
        Query::Ask { dataset: None, pattern, .. } => {
            // The parser wraps ASK's group pattern in a Project of its
            // in-scope variables; unwrap it (nothing is projected by ASK).
            let inner = match &pattern {
                GraphPattern::Project { inner, .. } => inner.as_ref(),
                other => other,
            };
            Ok(Shape { form: Form::Ask, patterns: bgp_patterns(inner)? })
        }
        Query::Select { dataset: None, pattern, .. } => {
            let (distinct, inner) = match &pattern {
                GraphPattern::Distinct { inner } => (true, inner.as_ref()),
                other => (false, other),
            };
            let GraphPattern::Project { inner, variables } = inner else {
                return Err(unsupported(
                    "SELECT must project a single basic graph pattern (no solution modifiers)",
                ));
            };
            for v in variables {
                check_reserved(v)?;
            }
            Ok(Shape {
                form: Form::Select { distinct, vars: variables.clone() },
                patterns: bgp_patterns(inner)?,
            })
        }
        Query::Ask { .. } | Query::Select { .. } => {
            Err(unsupported("FROM / FROM NAMED dataset clauses"))
        }
        _ => Err(unsupported("only ASK and SELECT are supported")),
    }
}

/// Injection guard for IRIs interpolated into `<…>` in the generated query.
fn safe_iri(iri: &str) -> Result<&str, ExpressionError> {
    if iri.is_empty() || iri.chars().any(|c| c == '>' || c == '<' || c.is_whitespace()) {
        return Err(ExpressionError::UnsafeIri(iri.to_string()));
    }
    Ok(iri)
}

/// The admissibility patterns conjoined per triple pattern (the §3.1 rewrite
/// body). Shared verbatim by `Q'` and the response-extraction query so the two
/// can never diverge.
fn admissibility_body(
    req: &TrustRequirements,
    patterns: &[TriplePattern],
) -> Result<String, ExpressionError> {
    if req.trusted_issuers.is_empty() && req.trusted_frameworks.is_empty() {
        return Err(ExpressionError::NoTrustMode);
    }
    if !is_utc_datetime_lexical(&req.valid_status_at) {
        return Err(ExpressionError::BadDateTime(req.valid_status_at.clone()));
    }
    let t = format!("\"{}\"^^<{}>", req.valid_status_at, xsd::DATE_TIME.as_str());

    let mut b = String::new();
    for (i, p) in patterns.iter().enumerate() {
        // The statement itself, inside its attestation-bundle graph.
        b.push_str(&format!("  GRAPH ?__tx_g{} {{ {} . }}\n", i, p));
        // Positive status attestation covering the bundle at t (design D3:
        // existence of a covering window — monotone, never absence).
        b.push_str(&format!(
            "  ?__tx_g{} <{}> ?__tx_st{} .\n",
            i, TRUSTX_COVERED_BY, i
        ));
        b.push_str(&format!(
            "  ?__tx_st{} a <{}> ; <{}> ?__tx_stf{} ; <{}> ?__tx_stu{} .\n",
            i, TRUSTX_STATUS_ATTESTATION, TRUSTX_VALID_FROM, i, TRUSTX_VALID_UNTIL, i
        ));
        b.push_str(&format!(
            "  FILTER(?__tx_stf{} <= {} && {} <= ?__tx_stu{})\n",
            i, t, t, i
        ));
        // Attribution: which issuer attested this bundle (PROV-O on the reifier).
        b.push_str(&format!(
            "  ?__tx_g{} <{}> ?__tx_iss{} .\n",
            i, PROV_WAS_ATTRIBUTED_TO, i
        ));

        // Issuer admissibility: mode 1 (enumerated) OR mode 2 (framework-certified).
        let mode1 = if req.trusted_issuers.is_empty() {
            None
        } else {
            let mut vals = String::new();
            for iss in &req.trusted_issuers {
                vals.push_str(&format!("<{}> ", safe_iri(iss.as_str())?));
            }
            Some(format!("VALUES ?__tx_iss{} {{ {}}}", i, vals))
        };
        let mode2 = if req.trusted_frameworks.is_empty() {
            None
        } else {
            let mut c = String::new();
            let ind = "    ";
            c.push_str(&format!(
                "{}?__tx_cert{} a <{}> ; <{}> ?__tx_iss{} ; <{}> ?__tx_fw{} ; <{}> ?__tx_cf{} ; <{}> ?__tx_cu{} ; <{}> ?__tx_cst{} .\n",
                ind, i, TRUSTX_CERTIFICATION, TRUSTX_CERTIFIES, i, TRUSTX_UNDER_FRAMEWORK, i,
                TRUSTX_VALID_FROM, i, TRUSTX_VALID_UNTIL, i, TRUSTX_COVERED_BY, i
            ));
            let mut vals = String::new();
            for fw in &req.trusted_frameworks {
                vals.push_str(&format!("<{}> ", safe_iri(fw.as_str())?));
            }
            c.push_str(&format!("{}VALUES ?__tx_fw{} {{ {}}}\n", ind, i, vals));
            // Certification window valid at t.
            c.push_str(&format!(
                "{}FILTER(?__tx_cf{} <= {} && {} <= ?__tx_cu{})\n",
                ind, i, t, t, i
            ));
            // The certification is itself status-covered at t — certifications
            // are status-checked exactly like credentials (design §6 case 7).
            c.push_str(&format!(
                "{}?__tx_cst{} a <{}> ; <{}> ?__tx_csf{} ; <{}> ?__tx_csu{} .\n",
                ind, i, TRUSTX_STATUS_ATTESTATION, TRUSTX_VALID_FROM, i, TRUSTX_VALID_UNTIL, i
            ));
            c.push_str(&format!(
                "{}FILTER(?__tx_csf{} <= {} && {} <= ?__tx_csu{})\n",
                ind, i, t, t, i
            ));
            // Scope conformance: service-level (the honest DIATF granularity)
            // or the statement's own predicate falls inside the certified scope
            // (design D4 — "issued only what they are certified to issue").
            if req.requires_scope_conformance {
                c.push_str(&format!(
                    "{}{{ ?__tx_cert{} <{}> <{}> }} UNION {{ ?__tx_cert{} <{}> {} }}\n",
                    ind,
                    i,
                    TRUSTX_CERTIFICATION_SCOPE,
                    TRUSTX_ANY_SERVICE_SCOPE,
                    i,
                    TRUSTX_CERTIFICATION_SCOPE,
                    p.predicate
                ));
            }
            Some(c)
        };
        match (mode1, mode2) {
            (Some(v), Some(c)) => {
                b.push_str(&format!("  {{\n    {}\n  }}\n  UNION\n  {{\n{}  }}\n", v, c));
            }
            (Some(v), None) => b.push_str(&format!("  {}\n", v)),
            (None, Some(c)) => b.push_str(&c),
            (None, None) => unreachable!("guarded by the NoTrustMode check above"),
        }
    }
    Ok(b)
}

fn build_rewrite(req: &TrustRequirements, shape: &Shape) -> Result<String, ExpressionError> {
    let body = admissibility_body(req, &shape.patterns)?;
    Ok(match &shape.form {
        Form::Ask => format!("ASK {{\n{}}}", body),
        Form::Select { distinct, vars } => {
            let head = vars.iter().map(Variable::to_string).collect::<Vec<_>>().join(" ");
            format!(
                "SELECT {}{} WHERE {{\n{}}}",
                if *distinct { "DISTINCT " } else { "" },
                head,
                body
            )
        }
    })
}

/// The §3.1 **normative reference rewrite** `Q → Q'`: a plain SPARQL query that
/// conjoins `Q`'s patterns (each wrapped in its attestation-bundle `GRAPH`)
/// with the admissibility patterns generated from `TR` — issuer membership,
/// positive status-attestation validity at *t*, and (mode 2)
/// certification-validity + scope conformance. `Q'` is what DEFINES "evaluate
/// `Q` under `TR`": it is checkable by ANY conformant SPARQL 1.1 engine over
/// the response's named-graph form, which is exactly what [`verify_response`]
/// does.
pub fn rewrite_query(request: &ContractRequest) -> Result<String, ExpressionError> {
    let shape = parse_shape(&request.query)?;
    build_rewrite(&request.requirements, &shape)
}

// ─────────────────────────────── evaluation ───────────────────────────────

/// The contract answer: `Q'`'s result over the holder's attested dataset.
#[derive(Debug, Clone, PartialEq)]
pub enum ContractAnswer {
    /// An ASK answer. `false` includes every fail-closed case: no admissible
    /// derivation ⇒ no binding.
    Boolean(bool),
    /// A SELECT answer: the original projection's variable names and rows
    /// (`None` = unbound).
    Solutions {
        /// The projected variable names (without `?`), in projection order.
        vars: Vec<String>,
        /// One entry per variable per row.
        rows: Vec<Vec<Option<Term>>>,
    },
}

/// The provenance-encoded response `R` (design §4): the contributing
/// statements plus, per statement, the provenance sufficient for an
/// independent verifier re-check — issuer attribution, covering status
/// attestation(s), and (mode 2) the certification with its scope, window and
/// status coverage. **Empty when nothing was admissible** (fail-closed: no
/// admissible derivation ⇒ no binding ⇒ zero bundles disclosed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvenanceResponse {
    /// The request's nonce, echoed (checked by [`verify_response`]). This
    /// value legitimately comes off the wire, so a caller reconstructing a
    /// response builds it with [`ChallengeNonce::from_wire`].
    pub nonce: ChallengeNonce,
    /// Option (a), the NORMATIVE encoding: RDF 1.2 reifiers — one
    /// `<bundle> rdf:reifies <<( s p o )>>` statement per contributing
    /// statement, with the PROV-O/`trustx:` qualification triples on the
    /// reifier node. sparq parses triple terms (`sparq-core::nt`) but cannot
    /// yet *query* them — named honestly in design §7.5.
    pub reifier_form: String,
    /// Option (b), the mechanically lossless runnable-today mapping (reifier
    /// node ↔ graph IRI): a TriG dataset — one named graph per contributing
    /// bundle, provenance in the default graph. This is the form
    /// [`verify_response`] re-checks `Q'` over.
    pub dataset_form: String,
    /// How many contributing statements the response carries (0 ⇒ no binding).
    pub contributing_statements: usize,
}

/// A full evaluation outcome.
#[derive(Debug, Clone, PartialEq)]
pub struct ContractOutcome {
    /// `Q'`'s answer over the holder's attested dataset.
    pub answer: ContractAnswer,
    /// The generated `Q'` (the normative semantics artifact, returned so the
    /// verifier can inspect exactly what was evaluated).
    pub rewritten_query: String,
    /// The provenance-encoded response `R`.
    pub response: ProvenanceResponse,
}

/// Resolve a pattern term against one solution row (constants pass through;
/// variables read their binding).
fn resolve_term(
    tp: &TermPattern,
    row: &[Option<Term>],
    col: &HashMap<String, usize>,
) -> Result<Term, ExpressionError> {
    match tp {
        TermPattern::NamedNode(n) => Ok(Term::NamedNode(n.clone())),
        TermPattern::Literal(l) => Ok(Term::Literal(l.clone())),
        TermPattern::Variable(v) => col
            .get(v.as_str())
            .and_then(|i| row.get(*i).cloned().flatten())
            .ok_or_else(|| ExpressionError::Engine(format!("unbound pattern variable ?{}", v.as_str()))),
        _ => Err(unsupported("non-fragment pattern term")),
    }
}

fn resolve_predicate(
    np: &NamedNodePattern,
    row: &[Option<Term>],
    col: &HashMap<String, usize>,
) -> Result<Term, ExpressionError> {
    match np {
        NamedNodePattern::NamedNode(n) => Ok(Term::NamedNode(n.clone())),
        NamedNodePattern::Variable(v) => col
            .get(v.as_str())
            .and_then(|i| row.get(*i).cloned().flatten())
            .ok_or_else(|| ExpressionError::Engine(format!("unbound pattern variable ?{}", v.as_str()))),
    }
}

/// The variable names (no `?`) appearing in the BGP, order-stable, deduplicated.
fn pattern_var_names(patterns: &[TriplePattern]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    let mut push = |name: &str| {
        if seen.insert(name.to_string()) {
            out.push(name.to_string());
        }
    };
    for p in patterns {
        if let TermPattern::Variable(v) = &p.subject {
            push(v.as_str());
        }
        if let NamedNodePattern::Variable(v) = &p.predicate {
            push(v.as_str());
        }
        if let TermPattern::Variable(v) = &p.object {
            push(v.as_str());
        }
    }
    out
}

/// Assemble the provenance-encoded response from the extraction query's
/// solutions: per solution row and pattern, the concrete contributing statement
/// in its bundle graph, then the default-graph provenance closure over the
/// bundle / status / certification nodes (following `trustx:coveredBy` links to
/// their attestations).
fn assemble_response(
    request: &ContractRequest,
    shape: &Shape,
    holder: &Graph,
) -> Result<ProvenanceResponse, ExpressionError> {
    let req = &request.requirements;
    let body = admissibility_body(req, &shape.patterns)?;

    // Projection: every pattern variable + the per-pattern provenance handles.
    let mode2 = !req.trusted_frameworks.is_empty();
    let mut proj: Vec<String> = pattern_var_names(&shape.patterns)
        .iter()
        .map(|n| format!("?{}", n))
        .collect();
    for i in 0..shape.patterns.len() {
        proj.push(format!("?__tx_g{}", i));
        proj.push(format!("?__tx_st{}", i));
        proj.push(format!("?__tx_iss{}", i));
        if mode2 {
            proj.push(format!("?__tx_cert{}", i));
            proj.push(format!("?__tx_cst{}", i));
        }
    }
    let extraction = format!("SELECT {} WHERE {{\n{}}}", proj.join(" "), body);
    let r = sparq_engine::query(holder, &extraction).map_err(ExpressionError::Engine)?;
    let col: HashMap<String, usize> = r
        .vars
        .iter()
        .enumerate()
        .map(|(i, v)| (v.as_str().to_string(), i))
        .collect();

    // bundle graph term (serialized) → its contributing statements (serialized s/p/o).
    let mut bundles: BTreeMap<String, BTreeSet<(String, String, String)>> = BTreeMap::new();
    // Default-graph provenance seed nodes (serialized term keys).
    let mut seeds: BTreeSet<String> = BTreeSet::new();
    for row in &r.rows {
        for (i, p) in shape.patterns.iter().enumerate() {
            let bound = |name: String| {
                col.get(&name).and_then(|ix| row.get(*ix).cloned().flatten())
            };
            let Some(g) = bound(format!("__tx_g{}", i)) else {
                continue;
            };
            let s = resolve_term(&p.subject, row, &col)?;
            let pr = resolve_predicate(&p.predicate, row, &col)?;
            let o = resolve_term(&p.object, row, &col)?;
            bundles
                .entry(g.to_string())
                .or_default()
                .insert((s.to_string(), pr.to_string(), o.to_string()));
            seeds.insert(g.to_string());
            for handle in ["__tx_st", "__tx_iss", "__tx_cert", "__tx_cst"] {
                if let Some(t) = bound(format!("{}{}", handle, i)) {
                    seeds.insert(t.to_string());
                }
            }
        }
    }

    // Default-graph provenance closure over the seeds; `trustx:coveredBy`
    // objects join the seed set so an attestation covering an included node is
    // always included (fixpoint — bounded by the default graph).
    let mut prov: BTreeSet<String> = BTreeSet::new();
    loop {
        let before = (prov.len(), seeds.len());
        for [si, pi, oi] in holder.iter_ids() {
            let s = holder.dict.term(si);
            if !seeds.contains(&s.to_string()) {
                continue;
            }
            let p = holder.dict.term(pi);
            let o = holder.dict.term(oi);
            if prov.insert(format!("{} {} {} .", s, p, o))
                && matches!(&p, Term::NamedNode(n) if n.as_str() == TRUSTX_COVERED_BY)
            {
                seeds.insert(o.to_string());
            }
        }
        if (prov.len(), seeds.len()) == before {
            break;
        }
    }

    // Option (b): TriG — provenance in the default graph, one named graph per bundle.
    let mut dataset_form = String::new();
    for line in &prov {
        dataset_form.push_str(line);
        dataset_form.push('\n');
    }
    for (g, triples) in &bundles {
        dataset_form.push_str(&format!("GRAPH {} {{\n", g));
        for (s, p, o) in triples {
            dataset_form.push_str(&format!("  {} {} {} .\n", s, p, o));
        }
        dataset_form.push_str("}\n");
    }

    // Option (a), normative: RDF 1.2 reifiers — the bundle node reifies each
    // contributing statement as a triple term; the SAME provenance qualifies
    // the reifier (the fixed bidirectional (a)↔(b) mapping: reifier ↔ graph IRI).
    let mut reifier_form = String::new();
    for (g, triples) in &bundles {
        for (s, p, o) in triples {
            reifier_form.push_str(&format!(
                "{} <{}> <<( {} {} {} )>> .\n",
                g, RDF_REIFIES, s, p, o
            ));
        }
    }
    for line in &prov {
        reifier_form.push_str(line);
        reifier_form.push('\n');
    }

    let contributing_statements = bundles.values().map(BTreeSet::len).sum();
    Ok(ProvenanceResponse {
        nonce: request.nonce.clone(),
        reifier_form,
        dataset_form,
        contributing_statements,
    })
}

fn answer_over(shape: &Shape, q_prime: &str, graph: &Graph) -> Result<ContractAnswer, ExpressionError> {
    match &shape.form {
        Form::Ask => Ok(ContractAnswer::Boolean(
            sparq_engine::ask(graph, q_prime).map_err(ExpressionError::Engine)?,
        )),
        Form::Select { .. } => {
            let r = sparq_engine::query(graph, q_prime).map_err(ExpressionError::Engine)?;
            Ok(ContractAnswer::Solutions {
                vars: r.vars.iter().map(|v| v.as_str().to_string()).collect(),
                rows: r.rows,
            })
        }
    }
}

/// Evaluate the contract on the holder side (clear path): the optional
/// `trustx:methodPolicy` ODRL pre-check (through the EXISTING
/// [`crate::admissibility::admissible`] reduction — fail-closed: a named policy
/// with no [`MethodPrecheck`] data, a pre-check resolving a DIFFERENT policy
/// IRI than `TR` names, or a non-admissible method, refuses the whole
/// evaluation; the resolution of the named IRI into the pre-check data remains
/// caller-owned — see the [`MethodPrecheck`] trust boundary), then `Q'` over
/// the holder's attested dataset, then the provenance-encoded response.
///
/// `holder` is the holder's attested dataset: **one named graph per
/// attestation bundle** (the credential's claim triples), with attribution
/// ([`PROV_WAS_ATTRIBUTED_TO`]), status attestations, and certifications in the
/// **default graph** (e.g. built with `sparq_core::Graph::load_dataset` from
/// TriG, or with [`mint_status_attestation`] for the status stratum).
///
/// Fail-closed: no admissible derivation ⇒ `Boolean(false)` / zero rows AND a
/// response with zero bundles.
pub fn evaluate_contract(
    request: &ContractRequest,
    holder: &Graph,
    precheck: Option<&MethodPrecheck<'_>>,
) -> Result<ContractOutcome, ExpressionError> {
    // D5: the method-policy axis is consulted BEFORE any evaluation.
    if let Some(required) = &request.requirements.method_policy {
        let pc = precheck.ok_or(ExpressionError::MethodPolicyWithoutPrecheck)?;
        // Bind the pre-check to the policy TR names: data resolved for a
        // different (weaker) policy is refused, never silently accepted.
        if pc.policy != required.as_str() {
            return Err(ExpressionError::MethodPolicyMismatch {
                required: required.as_str().to_string(),
                supplied: pc.policy.to_string(),
            });
        }
        let verdict =
            crate::admissibility::admissible(pc.method, pc.constraint_iris, pc.policy_n3, pc.annotations)
                .map_err(ExpressionError::Admissibility)?;
        if !verdict.admissible {
            return Err(ExpressionError::MethodNotAdmissible(verdict.unsatisfied));
        }
    }
    let shape = parse_shape(&request.query)?;
    let q_prime = build_rewrite(&request.requirements, &shape)?;
    let answer = answer_over(&shape, &q_prime, holder)?;
    let response = assemble_response(request, &shape, holder)?;
    Ok(ContractOutcome { answer, rewritten_query: q_prime, response })
}

/// The INDEPENDENT verifier re-check (the bead's second invariant): given only
/// the request and the response, re-derive `Q'` and evaluate it over `R`'s
/// named-graph form. A response whose provenance was stripped or tampered with
/// simply yields no admissible derivation (fail-closed `false` / zero rows); a
/// wrong nonce is refused outright.
pub fn verify_response(
    request: &ContractRequest,
    response: &ProvenanceResponse,
) -> Result<ContractAnswer, ExpressionError> {
    if response.nonce != request.nonce {
        return Err(ExpressionError::NonceMismatch);
    }
    let shape = parse_shape(&request.query)?;
    let q_prime = build_rewrite(&request.requirements, &shape)?;
    let r = Graph::load_dataset(&response.dataset_form, "trig")
        .map_err(ExpressionError::Response)?;
    answer_over(&shape, &q_prime, &r)
}

// ────────────────────── the status-attestation bridge ──────────────────────

/// Mint the positive, time-windowed `trustx:StatusAttestation` triples for an
/// attestation bundle from a VERIFIED [`crate::status_list`] live-status check
/// — the bridge from the merged signed-Bitstring status machinery (design D3:
/// IETF/W3C status lists map to the same positive-attestation shape) to the
/// graph form the §3.1 rewrite consumes.
///
/// **Fail-closed:** only a [`LiveStatus::Live`] verdict (the single admitting
/// variant of the P6 stratum) can mint an attestation — a set / unknown / stale
/// check returns [`ExpressionError::NonPositiveStatus`], so a revoked
/// credential can never acquire a covering window. The window is
/// `[as_of, as_of + max_age]` — the snapshot instant and freshness budget the
/// check itself used (design §7.4 names the freshness/caching trade-off: a
/// revocation inside the window is invisible until the next attestation).
pub fn mint_status_attestation(
    covered: &NamedNode,
    attestation: &NamedNode,
    status: LiveStatus,
    as_of_unix_secs: i64,
    max_age_secs: i64,
) -> Result<Vec<Triple>, ExpressionError> {
    if !status.admits() {
        return Err(ExpressionError::NonPositiveStatus(status.reason()));
    }
    if max_age_secs < 0 {
        return Err(ExpressionError::BadWindow);
    }
    let from = unix_to_datetime_lexical(as_of_unix_secs);
    let until = unix_to_datetime_lexical(as_of_unix_secs.saturating_add(max_age_secs));
    let att = NamedOrBlankNode::NamedNode(attestation.clone());
    Ok(vec![
        Triple::new(
            att.clone(),
            rdf::TYPE.into_owned(),
            Term::NamedNode(NamedNode::new_unchecked(TRUSTX_STATUS_ATTESTATION)),
        ),
        Triple::new(
            att.clone(),
            NamedNode::new_unchecked(TRUSTX_VALID_FROM),
            Term::Literal(Literal::new_typed_literal(from, xsd::DATE_TIME)),
        ),
        Triple::new(
            att,
            NamedNode::new_unchecked(TRUSTX_VALID_UNTIL),
            Term::Literal(Literal::new_typed_literal(until, xsd::DATE_TIME)),
        ),
        Triple::new(
            NamedOrBlankNode::NamedNode(covered.clone()),
            NamedNode::new_unchecked(TRUSTX_COVERED_BY),
            Term::NamedNode(attestation.clone()),
        ),
    ])
}

// ─────────────────────────────── helpers ───────────────────────────────

/// `true` for a **valid** UTC `xsd:dateTime` lexical `YYYY-MM-DDTHH:MM:SS[.fff]Z`.
/// Deliberately strict (UTC only): the rewrite embeds the lexical in FILTER
/// comparisons, and a single timezone form keeps lexical and timeline order
/// aligned across engines.
///
/// Beyond the fixed `…Z` structure this range-checks the calendar and clock
/// fields — Gregorian month (01–12), day-of-month under the proleptic-Gregorian
/// leap-year rule, hour (00–23), minute (00–59) and second (00–59) — so an
/// impossible instant such as `2026-99-99T99:99:99Z`, `2026-02-30T00:00:00Z` or a
/// non-leap-year `02-29` is rejected here rather than interpolated into a status
/// / certification-validity FILTER and left to downstream engine behaviour.
/// (`xsd:dateTime` forbids leap seconds, so a `:60` second is out of range.)
fn is_utc_datetime_lexical(lex: &str) -> bool {
    let b = lex.as_bytes();
    if b.len() < 20 || b[b.len() - 1] != b'Z' {
        return false;
    }
    for (i, c) in b[..19].iter().enumerate() {
        let ok = match i {
            4 | 7 => *c == b'-',
            10 => *c == b'T',
            13 | 16 => *c == b':',
            _ => c.is_ascii_digit(),
        };
        if !ok {
            return false;
        }
    }
    let frac = &b[19..b.len() - 1];
    if !(frac.is_empty()
        || (frac[0] == b'.' && frac.len() > 1 && frac[1..].iter().all(u8::is_ascii_digit)))
    {
        return false;
    }

    // The structural loop above fixed every position below to an ASCII digit, so
    // these field reads cannot underflow: range-check the calendar and clock.
    let two = |i: usize| u32::from(b[i] - b'0') * 10 + u32::from(b[i + 1] - b'0');
    let year = u32::from(b[0] - b'0') * 1000
        + u32::from(b[1] - b'0') * 100
        + u32::from(b[2] - b'0') * 10
        + u32::from(b[3] - b'0');
    let (month, day, hour, minute, second) = (two(5), two(8), two(11), two(14), two(17));
    if !(1..=12).contains(&month) || day == 0 || hour > 23 || minute > 59 || second > 59 {
        return false;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if leap {
                29
            } else {
                28
            }
        }
    };
    day <= max_day
}

/// Unix seconds → canonical UTC `xsd:dateTime` lexical (proleptic Gregorian;
/// the standard civil-from-days algorithm).
fn unix_to_datetime_lexical(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, min, s) = (rem / 3_600, (rem % 3_600) / 60, rem % 60);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(mo <= 2);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y, mo, d, h, min, s
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framework_vocab::TRUSTX_EIDAS2;

    // ── fixtures ─────────────────────────────────────────────────────────

    /// The verifier-chosen status instant t (design D3: a parameter, never a constant).
    const T: &str = "2026-07-05T00:00:00Z";
    const FOAF_AGE: &str = "http://xmlns.com/foaf/0.1/age";
    const FOAF_NAME: &str = "http://xmlns.com/foaf/0.1/name";
    const ISS_X: &str = "did:web:x.example";
    const ISS_CERT: &str = "did:web:cert.example";
    const ISS_EVIL: &str = "did:web:evil.example";
    const AGE_25: &str = "\"25\"^^<http://www.w3.org/2001/XMLSchema#integer>";
    const ASK_AGE: &str = "ASK { <urn:jesse> <http://xmlns.com/foaf/0.1/age> ?age }";
    const SELECT_AGE: &str =
        "SELECT ?age WHERE { <urn:jesse> <http://xmlns.com/foaf/0.1/age> ?age }";

    fn nn(iri: &str) -> NamedNode {
        NamedNode::new_unchecked(iri)
    }

    fn dtlit(lex: &str) -> String {
        format!("\"{}\"^^<http://www.w3.org/2001/XMLSchema#dateTime>", lex)
    }

    fn rdf_type() -> &'static str {
        "http://www.w3.org/1999/02/22-rdf-syntax-ns#type"
    }

    /// Default-graph status-attestation lines covering `covered` over [from, until].
    fn status_lines(covered: &str, att: &str, from: &str, until: &str) -> String {
        format!(
            "<{}> <{}> <{}> .\n<{}> <{}> <{}> .\n<{}> <{}> {} .\n<{}> <{}> {} .\n",
            covered,
            TRUSTX_COVERED_BY,
            att,
            att,
            rdf_type(),
            TRUSTX_STATUS_ATTESTATION,
            att,
            TRUSTX_VALID_FROM,
            dtlit(from),
            att,
            TRUSTX_VALID_UNTIL,
            dtlit(until)
        )
    }

    /// Mode-1 holder dataset: bundle `urn:b1` (jesse's age) attributed to
    /// `issuer`, optionally covered by a status attestation over [from, until].
    fn holder_mode1(issuer: &str, from: &str, until: &str, with_status: bool) -> Graph {
        let mut d = String::new();
        d.push_str(&format!("<urn:b1> <{}> <{}> .\n", PROV_WAS_ATTRIBUTED_TO, issuer));
        if with_status {
            d.push_str(&status_lines("urn:b1", "urn:st1", from, until));
        }
        d.push_str(&format!(
            "GRAPH <urn:b1> {{ <urn:jesse> <{}> {} . }}\n",
            FOAF_AGE, AGE_25
        ));
        Graph::load_dataset(&d, "trig").expect("fixture TriG parses")
    }

    /// Mode-2 holder dataset: bundle attributed to a NON-enumerated issuer that
    /// carries a `trustx:Certification` under eIDAS2 with the given `scope`
    /// object and window; the certification is itself status-covered when
    /// `with_cert_status`.
    fn holder_mode2(scope: &str, cert_from: &str, cert_until: &str, with_cert_status: bool) -> Graph {
        let mut d = String::new();
        d.push_str(&format!("<urn:b1> <{}> <{}> .\n", PROV_WAS_ATTRIBUTED_TO, ISS_CERT));
        d.push_str(&status_lines("urn:b1", "urn:st1", "2026-07-01T00:00:00Z", "2026-07-10T00:00:00Z"));
        d.push_str(&format!("<urn:cert1> <{}> <{}> .\n", rdf_type(), TRUSTX_CERTIFICATION));
        d.push_str(&format!("<urn:cert1> <{}> <{}> .\n", TRUSTX_CERTIFIES, ISS_CERT));
        d.push_str(&format!("<urn:cert1> <{}> <{}> .\n", TRUSTX_UNDER_FRAMEWORK, TRUSTX_EIDAS2));
        d.push_str(&format!("<urn:cert1> <{}> {} .\n", TRUSTX_VALID_FROM, dtlit(cert_from)));
        d.push_str(&format!("<urn:cert1> <{}> {} .\n", TRUSTX_VALID_UNTIL, dtlit(cert_until)));
        d.push_str(&format!("<urn:cert1> <{}> <{}> .\n", TRUSTX_CERTIFICATION_SCOPE, scope));
        if with_cert_status {
            d.push_str(&status_lines(
                "urn:cert1",
                "urn:cst1",
                "2026-07-01T00:00:00Z",
                "2026-07-10T00:00:00Z",
            ));
        }
        d.push_str(&format!(
            "GRAPH <urn:b1> {{ <urn:jesse> <{}> {} . }}\n",
            FOAF_AGE, AGE_25
        ));
        Graph::load_dataset(&d, "trig").expect("fixture TriG parses")
    }

    /// A trust-requirements graph (design §3.2 shapes).
    fn tr_triples(
        issuers: &[&str],
        frameworks: &[&str],
        scope_conformance: Option<bool>,
        method_policy: Option<&str>,
    ) -> Vec<Triple> {
        let node = NamedOrBlankNode::NamedNode(nn("urn:tr1"));
        let mut v = vec![
            Triple::new(
                node.clone(),
                rdf::TYPE.into_owned(),
                Term::NamedNode(nn(TRUSTX_TRUST_REQUIREMENTS)),
            ),
            Triple::new(
                node.clone(),
                nn(TRUSTX_QUESTION),
                Term::NamedNode(nn("urn:q1")),
            ),
            Triple::new(
                node.clone(),
                nn(TRUSTX_REQUIRES_VALID_STATUS_AT),
                Term::Literal(Literal::new_typed_literal(T, xsd::DATE_TIME)),
            ),
        ];
        for i in issuers {
            v.push(Triple::new(
                node.clone(),
                nn(TRUSTX_TRUSTS_ISSUER),
                Term::NamedNode(nn(i)),
            ));
        }
        for f in frameworks {
            v.push(Triple::new(
                node.clone(),
                nn(TRUSTX_TRUSTS_FRAMEWORK),
                Term::NamedNode(nn(f)),
            ));
        }
        if let Some(b) = scope_conformance {
            v.push(Triple::new(
                node.clone(),
                nn(TRUSTX_REQUIRES_SCOPE_CONFORMANCE),
                Term::Literal(Literal::new_typed_literal(
                    if b { "true" } else { "false" },
                    xsd::BOOLEAN,
                )),
            ));
        }
        if let Some(p) = method_policy {
            v.push(Triple::new(node, nn(TRUSTX_METHOD_POLICY), Term::NamedNode(nn(p))));
        }
        v
    }

    /// A fixture nonce. Deliberately routed through the named
    /// `from_wire` constructor: a test value is exactly the "came from
    /// outside, promises nothing about freshness" case that constructor is for.
    fn nonce(value: &str) -> ChallengeNonce {
        ChallengeNonce::from_wire(value).expect("fixture nonce is non-empty")
    }

    fn request(query: &str, issuers: &[&str], frameworks: &[&str]) -> ContractRequest {
        parse_request(query, &tr_triples(issuers, frameworks, None, None), &nonce("nonce-1"))
            .expect("fixture request parses")
    }

    // ── parse_request ───────────────────────────────────────────────────

    #[test]
    fn parse_request_happy_path_two_modes() {
        let req = parse_request(
            ASK_AGE,
            &tr_triples(&[ISS_X], &[TRUSTX_EIDAS2], None, None),
            &nonce("n-1"),
        )
        .expect("parses");
        assert_eq!(req.nonce.as_str(), "n-1");
        assert_eq!(req.requirements.question.as_str(), "urn:q1");
        assert_eq!(req.requirements.trusted_issuers, vec![nn(ISS_X)]);
        assert_eq!(req.requirements.trusted_frameworks, vec![nn(TRUSTX_EIDAS2)]);
        // ABSENT scope-conformance flag defaults to true (the stricter reading).
        assert!(req.requirements.requires_scope_conformance);
        assert_eq!(req.requirements.valid_status_at, T);
        assert_eq!(req.requirements.method_policy, None);
    }

    #[test]
    fn parse_request_rejects_empty_query() {
        let tr = tr_triples(&[ISS_X], &[], None, None);
        assert_eq!(parse_request(" ", &tr, &nonce("n")), Err(ExpressionError::EmptyQuery));
    }

    // ── ChallengeNonce (issue #4621) ─────────────────────────────────────

    #[test]
    fn from_wire_refuses_an_empty_or_whitespace_nonce() {
        // The mandatory challenge-response binding: the refusal parse_request
        // used to make now lives at the one constructor an outside value can
        // enter through.
        assert_eq!(ChallengeNonce::from_wire(""), Err(ExpressionError::EmptyNonce));
        assert_eq!(ChallengeNonce::from_wire("  "), Err(ExpressionError::EmptyNonce));
        assert_eq!(ChallengeNonce::from_wire("\t\n"), Err(ExpressionError::EmptyNonce));
    }

    #[test]
    fn from_wire_preserves_the_value_verbatim() {
        // It adopts, it does not normalise: the echo check in verify_response
        // is a byte comparison against whatever the verifier put on the wire.
        let n = ChallengeNonce::from_wire(" padded ").expect("non-empty");
        assert_eq!(n.as_str(), " padded ");
        assert_eq!(n.to_string(), " padded ");
    }

    #[test]
    fn generate_draws_a_fresh_unpredictable_nonce_every_time() {
        // THE red test for the freshness guard: a `generate` that returned a
        // constant — the exact failure mode issue #4621 is about — collapses
        // this set to one element.
        const DRAWS: usize = 64;
        let drawn: BTreeSet<String> = (0..DRAWS)
            .map(|_| ChallengeNonce::generate().expect("OS entropy").as_str().to_string())
            .collect();
        assert_eq!(drawn.len(), DRAWS, "generate() repeated a nonce across {} draws", DRAWS);
        for n in &drawn {
            // 32 CSPRNG bytes, lowercase hex.
            assert_eq!(n.len(), ChallengeNonce::ENTROPY_BYTES * 2, "unexpected width: {}", n);
            assert!(
                n.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')),
                "not lowercase hex: {}",
                n
            );
        }
        // A generated nonce is a legal wire value (it round-trips through the
        // holder's echo without the emptiness refusal firing).
        let fresh = ChallengeNonce::generate().expect("OS entropy");
        assert_eq!(ChallengeNonce::from_wire(fresh.as_str()).expect("round-trips"), fresh);
    }

    #[test]
    fn a_generated_nonce_drives_the_contract_and_is_echoed() {
        // The verifier-side path end-to-end: the freshly generated challenge is
        // what parse_request carries and what the response has to echo back.
        let fresh = ChallengeNonce::generate().expect("OS entropy");
        let req = parse_request(ASK_AGE, &tr_triples(&[ISS_X], &[], None, None), &fresh)
            .expect("parses");
        assert_eq!(req.nonce, fresh);
        let holder = holder_mode1(ISS_X, "2026-07-01T00:00:00Z", "2026-07-10T00:00:00Z", true);
        let out = evaluate_contract(&req, &holder, None).expect("evaluates");
        assert_eq!(out.response.nonce, fresh);
        assert_eq!(verify_response(&req, &out.response), Ok(ContractAnswer::Boolean(true)));
    }

    #[test]
    fn parse_request_rejects_missing_or_duplicated_requirements_node() {
        assert_eq!(parse_request(ASK_AGE, &[], &nonce("n")), Err(ExpressionError::NoRequirements));
        let mut two = tr_triples(&[ISS_X], &[], None, None);
        two.push(Triple::new(
            nn("urn:tr2"),
            rdf::TYPE.into_owned(),
            Term::NamedNode(nn(TRUSTX_TRUST_REQUIREMENTS)),
        ));
        assert_eq!(
            parse_request(ASK_AGE, &two, &nonce("n")),
            Err(ExpressionError::MultipleRequirements)
        );
    }

    #[test]
    fn parse_request_rejects_missing_question_and_missing_instant() {
        let no_q: Vec<Triple> = tr_triples(&[ISS_X], &[], None, None)
            .into_iter()
            .filter(|t| t.predicate.as_str() != TRUSTX_QUESTION)
            .collect();
        assert_eq!(parse_request(ASK_AGE, &no_q, &nonce("n")), Err(ExpressionError::MissingQuestion));
        let no_t: Vec<Triple> = tr_triples(&[ISS_X], &[], None, None)
            .into_iter()
            .filter(|t| t.predicate.as_str() != TRUSTX_REQUIRES_VALID_STATUS_AT)
            .collect();
        assert_eq!(
            parse_request(ASK_AGE, &no_t, &nonce("n")),
            Err(ExpressionError::MissingValidStatusAt)
        );
    }

    #[test]
    fn question_iri_is_an_opaque_label_never_compared_with_the_query() {
        // The DOCUMENTED trust boundary (module docs, honest scope): the
        // question IRI names the question TR was authored for, but nothing at
        // this layer resolves or verifies it against Q — a TR minted for
        // `urn:q1` paired with a DIFFERENT supported query still parses,
        // evaluates, and re-checks. Verifying that Q is the named question
        // (request authentication / trusted question resolution) is
        // caller-owned; if an in-band binding is ever enforced, this test must
        // flip to a rejection.
        let tr = tr_triples(&[ISS_X], &[], None, None); // question = urn:q1
        let q = format!("ASK {{ <urn:jesse> <{}> ?name }}", FOAF_NAME);
        let req = parse_request(&q, &tr, &nonce("n")).expect("unrelated query accepted");
        assert_eq!(req.requirements.question.as_str(), "urn:q1");

        let mut d = String::new();
        d.push_str(&format!("<urn:b1> <{}> <{}> .\n", PROV_WAS_ATTRIBUTED_TO, ISS_X));
        d.push_str(&status_lines("urn:b1", "urn:st1", "2026-07-01T00:00:00Z", "2026-07-10T00:00:00Z"));
        d.push_str(&format!(
            "GRAPH <urn:b1> {{ <urn:jesse> <{}> \"Jesse\" . }}\n",
            FOAF_NAME
        ));
        let holder = Graph::load_dataset(&d, "trig").expect("fixture parses");
        let out = evaluate_contract(&req, &holder, None).expect("evaluates");
        assert_eq!(out.answer, ContractAnswer::Boolean(true));
        assert_eq!(
            verify_response(&req, &out.response).expect("re-checks"),
            ContractAnswer::Boolean(true)
        );
    }

    #[test]
    fn parse_request_rejects_non_utc_datetime() {
        let mut tr: Vec<Triple> = tr_triples(&[ISS_X], &[], None, None)
            .into_iter()
            .filter(|t| t.predicate.as_str() != TRUSTX_REQUIRES_VALID_STATUS_AT)
            .collect();
        tr.push(Triple::new(
            nn("urn:tr1"),
            nn(TRUSTX_REQUIRES_VALID_STATUS_AT),
            Term::Literal(Literal::new_typed_literal(
                "2026-07-05T00:00:00+02:00",
                xsd::DATE_TIME,
            )),
        ));
        assert_eq!(
            parse_request(ASK_AGE, &tr, &nonce("n")),
            Err(ExpressionError::BadDateTime("2026-07-05T00:00:00+02:00".to_string()))
        );
    }

    #[test]
    fn parse_request_rejects_no_trust_mode() {
        // Neither issuers nor frameworks: such a TR admits nothing — refused up front.
        assert_eq!(
            parse_request(ASK_AGE, &tr_triples(&[], &[], None, None), &nonce("n")),
            Err(ExpressionError::NoTrustMode)
        );
    }

    #[test]
    fn parse_request_validates_did_issuers_and_accepts_plain_iris() {
        // Uppercase DID method is invalid DID syntax → fail-closed via Did::parse.
        assert_eq!(
            parse_request(ASK_AGE, &tr_triples(&["did:WEB:x.example"], &[], None, None), &nonce("n")),
            Err(ExpressionError::BadIssuer("did:WEB:x.example".to_string()))
        );
        // A non-did IRI identity is accepted opaquely.
        let req = parse_request(
            ASK_AGE,
            &tr_triples(&["https://issuers.example/x"], &[], None, None),
            &nonce("n"),
        )
        .expect("plain IRI issuer accepted");
        assert_eq!(req.requirements.trusted_issuers, vec![nn("https://issuers.example/x")]);
    }

    #[test]
    fn parse_request_reads_explicit_scope_conformance_flag() {
        let req = parse_request(
            ASK_AGE,
            &tr_triples(&[], &[TRUSTX_EIDAS2], Some(false), None),
            &nonce("n"),
        )
        .expect("parses");
        assert!(!req.requirements.requires_scope_conformance);
    }

    // ── rewrite_query (the §3.1 normative reference rewrite) ─────────────

    #[test]
    fn rewrite_mode1_conjoins_graph_status_and_issuer_membership() {
        let q = rewrite_query(&request(ASK_AGE, &[ISS_X], &[])).expect("rewrites");
        assert!(q.starts_with("ASK {"), "ASK form preserved: {}", q);
        assert!(q.contains("GRAPH ?__tx_g0"), "bundle GRAPH wrap: {}", q);
        assert!(q.contains(TRUSTX_COVERED_BY), "status coverage pattern: {}", q);
        assert!(q.contains(TRUSTX_STATUS_ATTESTATION), "positive attestation type: {}", q);
        assert!(
            q.contains(&format!("\"{}\"^^<http://www.w3.org/2001/XMLSchema#dateTime>", T)),
            "instant t embedded: {}",
            q
        );
        assert!(q.contains(PROV_WAS_ATTRIBUTED_TO), "attribution pattern: {}", q);
        assert!(
            q.contains(&format!("VALUES ?__tx_iss0 {{ <{}> }}", ISS_X)),
            "issuer membership VALUES: {}",
            q
        );
        // Mode 1 only: no certification machinery.
        assert!(!q.contains(TRUSTX_CERTIFICATION), "no cert block in mode 1: {}", q);
    }

    #[test]
    fn rewrite_mode2_conjoins_certification_window_status_and_scope() {
        let q = rewrite_query(&request(ASK_AGE, &[], &[TRUSTX_EIDAS2])).expect("rewrites");
        assert!(q.contains(TRUSTX_CERTIFICATION), "certification pattern: {}", q);
        assert!(q.contains(TRUSTX_CERTIFIES), "certifies binding: {}", q);
        assert!(
            q.contains(&format!("VALUES ?__tx_fw0 {{ <{}> }}", TRUSTX_EIDAS2)),
            "framework membership: {}",
            q
        );
        // Certifications are status-checked like credentials (design §6 case 7).
        assert!(q.contains("?__tx_cst0"), "cert status coverage: {}", q);
        // Scope conformance: AnyServiceScope OR the statement's own predicate.
        assert!(q.contains(TRUSTX_ANY_SERVICE_SCOPE), "service-level scope arm: {}", q);
        assert!(
            q.contains(&format!("<{}> <{}>", TRUSTX_CERTIFICATION_SCOPE, FOAF_AGE)),
            "predicate-scope arm: {}",
            q
        );
    }

    #[test]
    fn rewrite_composes_the_two_modes_by_union() {
        let q = rewrite_query(&request(ASK_AGE, &[ISS_X], &[TRUSTX_EIDAS2])).expect("rewrites");
        assert!(q.contains("UNION"), "modes compose by OR: {}", q);
        assert!(q.contains("VALUES ?__tx_iss0"), "mode 1 arm present: {}", q);
        assert!(q.contains(TRUSTX_CERTIFICATION), "mode 2 arm present: {}", q);
    }

    #[test]
    fn rewrite_scope_conformance_false_omits_the_scope_check() {
        let req = parse_request(
            ASK_AGE,
            &tr_triples(&[], &[TRUSTX_EIDAS2], Some(false), None),
            &nonce("n"),
        )
        .expect("parses");
        let q = rewrite_query(&req).expect("rewrites");
        assert!(!q.contains(TRUSTX_ANY_SERVICE_SCOPE), "scope arm omitted: {}", q);
        // Certification validity + status coverage still required (fail-closed core).
        assert!(q.contains(TRUSTX_CERTIFICATION) && q.contains("?__tx_cst0"));
    }

    #[test]
    fn rewrite_preserves_select_projection_and_distinct() {
        let q = rewrite_query(&request(SELECT_AGE, &[ISS_X], &[])).expect("rewrites");
        assert!(q.starts_with("SELECT ?age WHERE {"), "projection preserved: {}", q);
        let qd = rewrite_query(&request(
            "SELECT DISTINCT ?age WHERE { <urn:jesse> <http://xmlns.com/foaf/0.1/age> ?age }",
            &[ISS_X],
            &[],
        ))
        .expect("rewrites");
        assert!(qd.starts_with("SELECT DISTINCT ?age WHERE {"), "DISTINCT preserved: {}", qd);
    }

    #[test]
    fn rewrite_refuses_queries_outside_the_v1_fragment() {
        let cases: &[&str] = &[
            // graph-valued form
            "CONSTRUCT { <urn:s> <urn:p> <urn:o> } WHERE { ?s ?p ?o }",
            // FILTER inside Q (trust conditions live in TR, not Q)
            "ASK { ?s <urn:p> ?o FILTER(?o > 5) }",
            // property path
            "ASK { ?s <urn:p>+ ?o }",
            // OPTIONAL
            "ASK { ?s <urn:p> ?o OPTIONAL { ?s <urn:q> ?z } }",
            // dataset clause
            "ASK FROM <urn:g> WHERE { ?s ?p ?o }",
            // blank-node pattern (cannot be projected for response assembly)
            "ASK { _:b <urn:p> ?o }",
            // empty BGP (vacuous)
            "ASK { }",
        ];
        for c in cases {
            let req = request(ASK_AGE, &[ISS_X], &[]);
            let req = ContractRequest { query: (*c).to_string(), ..req };
            assert!(
                matches!(rewrite_query(&req), Err(ExpressionError::UnsupportedQuery(_))),
                "expected UnsupportedQuery for: {}",
                c
            );
        }
    }

    #[test]
    fn rewrite_refuses_reserved_variables_and_garbage() {
        let req = request(ASK_AGE, &[ISS_X], &[]);
        let reserved = ContractRequest {
            query: "ASK { ?__tx_g0 <urn:p> ?o }".to_string(),
            ..req.clone()
        };
        assert!(matches!(
            rewrite_query(&reserved),
            Err(ExpressionError::ReservedVariable(_))
        ));
        let garbage = ContractRequest { query: "NOT SPARQL".to_string(), ..req };
        assert!(matches!(rewrite_query(&garbage), Err(ExpressionError::QueryParse(_))));
    }

    #[test]
    fn rewrite_fails_closed_on_hand_built_invalid_requirements() {
        // The pub-field structs allow bypassing parse_request; the rewrite
        // re-validates fail-closed.
        let req = request(ASK_AGE, &[ISS_X], &[]);
        let mut no_mode = req.clone();
        no_mode.requirements.trusted_issuers.clear();
        assert_eq!(rewrite_query(&no_mode), Err(ExpressionError::NoTrustMode));
        let mut bad_t = req;
        bad_t.requirements.valid_status_at = "garbage\") . } #injection".to_string();
        assert!(matches!(rewrite_query(&bad_t), Err(ExpressionError::BadDateTime(_))));
    }

    // ── evaluate_contract + verify_response: mode 1 ──────────────────────

    #[test]
    fn mode1_admissible_derivation_binds_and_recheck_reproduces_it() {
        let req = request(ASK_AGE, &[ISS_X], &[]);
        let holder = holder_mode1(ISS_X, "2026-07-01T00:00:00Z", "2026-07-10T00:00:00Z", true);
        let out = evaluate_contract(&req, &holder, None).expect("evaluates");
        assert_eq!(out.answer, ContractAnswer::Boolean(true));
        assert_eq!(out.response.contributing_statements, 1);
        // The (b) form carries the bundle AND its provenance.
        assert!(out.response.dataset_form.contains("GRAPH <urn:b1>"));
        assert!(out.response.dataset_form.contains(PROV_WAS_ATTRIBUTED_TO));
        assert!(out.response.dataset_form.contains(TRUSTX_STATUS_ATTESTATION));
        // The (a) normative form reifies the statement as an RDF 1.2 triple term.
        assert!(out.response.reifier_form.contains(RDF_REIFIES));
        assert!(out.response.reifier_form.contains("<<("));
        // INDEPENDENT verifier re-check: Q' over R reproduces the answer.
        assert_eq!(
            verify_response(&req, &out.response).expect("re-checks"),
            ContractAnswer::Boolean(true)
        );
    }

    #[test]
    fn mode1_untrusted_issuer_fails_closed() {
        let req = request(ASK_AGE, &[ISS_X], &[]);
        let holder = holder_mode1(ISS_EVIL, "2026-07-01T00:00:00Z", "2026-07-10T00:00:00Z", true);
        let out = evaluate_contract(&req, &holder, None).expect("evaluates");
        // No admissible derivation ⇒ no binding AND an empty response.
        assert_eq!(out.answer, ContractAnswer::Boolean(false));
        assert_eq!(out.response.contributing_statements, 0);
        assert!(out.response.dataset_form.is_empty());
        assert_eq!(
            verify_response(&req, &out.response).expect("re-checks"),
            ContractAnswer::Boolean(false)
        );
    }

    #[test]
    fn mode1_stale_status_window_fails_closed() {
        // Attestation exists but its window does not cover t (design §6 case 3).
        let req = request(ASK_AGE, &[ISS_X], &[]);
        let holder = holder_mode1(ISS_X, "2026-07-01T00:00:00Z", "2026-07-04T00:00:00Z", true);
        let out = evaluate_contract(&req, &holder, None).expect("evaluates");
        assert_eq!(out.answer, ContractAnswer::Boolean(false));
        assert_eq!(out.response.contributing_statements, 0);
    }

    #[test]
    fn mode1_missing_status_attestation_fails_closed() {
        // The revocation shape under OWA: a revoked credential simply has NO
        // covering positive attestation (design §6 case 2 — never "false
        // because revoked", just no admissible derivation).
        let req = request(ASK_AGE, &[ISS_X], &[]);
        let holder = holder_mode1(ISS_X, "", "", false);
        let out = evaluate_contract(&req, &holder, None).expect("evaluates");
        assert_eq!(out.answer, ContractAnswer::Boolean(false));
        assert_eq!(out.response.contributing_statements, 0);
    }

    #[test]
    fn mode1_select_returns_admissible_bindings_and_recheck_matches() {
        let req = request(SELECT_AGE, &[ISS_X], &[]);
        let holder = holder_mode1(ISS_X, "2026-07-01T00:00:00Z", "2026-07-10T00:00:00Z", true);
        let out = evaluate_contract(&req, &holder, None).expect("evaluates");
        let ContractAnswer::Solutions { vars, rows } = &out.answer else {
            panic!("expected solutions, got {:?}", out.answer);
        };
        assert_eq!(vars, &["age".to_string()]);
        assert_eq!(rows.len(), 1);
        let bound = rows[0][0].as_ref().expect("age bound").to_string();
        assert!(bound.contains("25"), "age binding: {}", bound);
        assert_eq!(verify_response(&req, &out.response).expect("re-checks"), out.answer);
    }

    #[test]
    fn multi_pattern_query_requires_admissibility_per_statement() {
        // Two patterns over one bundle: both contribute, both provenance-checked.
        let mut d = String::new();
        d.push_str(&format!("<urn:b1> <{}> <{}> .\n", PROV_WAS_ATTRIBUTED_TO, ISS_X));
        d.push_str(&status_lines("urn:b1", "urn:st1", "2026-07-01T00:00:00Z", "2026-07-10T00:00:00Z"));
        d.push_str(&format!(
            "GRAPH <urn:b1> {{ <urn:jesse> <{}> {} . <urn:jesse> <{}> \"Jesse\" . }}\n",
            FOAF_AGE, AGE_25, FOAF_NAME
        ));
        let holder = Graph::load_dataset(&d, "trig").expect("fixture parses");
        let q = format!(
            "ASK {{ <urn:jesse> <{}> ?age . <urn:jesse> <{}> ?name }}",
            FOAF_AGE, FOAF_NAME
        );
        let req = request(&q, &[ISS_X], &[]);
        let out = evaluate_contract(&req, &holder, None).expect("evaluates");
        assert_eq!(out.answer, ContractAnswer::Boolean(true));
        assert_eq!(out.response.contributing_statements, 2);
        assert_eq!(
            verify_response(&req, &out.response).expect("re-checks"),
            ContractAnswer::Boolean(true)
        );
    }

    // ── evaluate_contract: mode 2 (framework-certified issuers) ──────────

    #[test]
    fn mode2_scope_conformant_certification_binds() {
        let req = request(ASK_AGE, &[], &[TRUSTX_EIDAS2]);
        let holder = holder_mode2(FOAF_AGE, "2026-07-01T00:00:00Z", "2026-07-10T00:00:00Z", true);
        let out = evaluate_contract(&req, &holder, None).expect("evaluates");
        assert_eq!(out.answer, ContractAnswer::Boolean(true));
        // The response provenance includes the certification + BOTH attestations,
        // so the verifier can re-check scope + windows independently.
        assert!(out.response.dataset_form.contains(TRUSTX_CERTIFICATION));
        assert!(out.response.dataset_form.contains("<urn:cst1>"));
        assert_eq!(
            verify_response(&req, &out.response).expect("re-checks"),
            ContractAnswer::Boolean(true)
        );
    }

    #[test]
    fn mode2_any_service_scope_binds_the_diatf_granularity() {
        let req = request(ASK_AGE, &[], &[TRUSTX_EIDAS2]);
        let holder = holder_mode2(
            TRUSTX_ANY_SERVICE_SCOPE,
            "2026-07-01T00:00:00Z",
            "2026-07-10T00:00:00Z",
            true,
        );
        let out = evaluate_contract(&req, &holder, None).expect("evaluates");
        assert_eq!(out.answer, ContractAnswer::Boolean(true));
    }

    #[test]
    fn mode2_scope_violation_fails_closed() {
        // Certified issuer, valid windows — but the statement's predicate is
        // OUTSIDE the certified scope (design §6 case 6: the "only issued what
        // they are certified to issue" reject).
        let req = request(ASK_AGE, &[], &[TRUSTX_EIDAS2]);
        let holder = holder_mode2(FOAF_NAME, "2026-07-01T00:00:00Z", "2026-07-10T00:00:00Z", true);
        let out = evaluate_contract(&req, &holder, None).expect("evaluates");
        assert_eq!(out.answer, ContractAnswer::Boolean(false));
        assert_eq!(out.response.contributing_statements, 0);
    }

    #[test]
    fn mode2_expired_certification_fails_closed() {
        let req = request(ASK_AGE, &[], &[TRUSTX_EIDAS2]);
        let holder = holder_mode2(FOAF_AGE, "2026-06-01T00:00:00Z", "2026-07-04T00:00:00Z", true);
        let out = evaluate_contract(&req, &holder, None).expect("evaluates");
        assert_eq!(out.answer, ContractAnswer::Boolean(false));
    }

    #[test]
    fn mode2_status_uncovered_certification_fails_closed() {
        // Certifications are status-checked exactly like credentials (design §6
        // case 7): no covering attestation on the certification ⇒ no binding.
        let req = request(ASK_AGE, &[], &[TRUSTX_EIDAS2]);
        let holder = holder_mode2(FOAF_AGE, "2026-07-01T00:00:00Z", "2026-07-10T00:00:00Z", false);
        let out = evaluate_contract(&req, &holder, None).expect("evaluates");
        assert_eq!(out.answer, ContractAnswer::Boolean(false));
    }

    // ── the trustx:methodPolicy ODRL pre-check (design D5 reuse) ─────────

    const METHOD_M1: &str = "https://sparq.dev/ns/zk#m1";
    const CONSTRAINT_PER: &str = "https://sparq.dev/ns/zk#cReqPer";
    const CONSTRAINT_CROSS: &str = "https://sparq.dev/ns/zk#cReqCross";
    const M1_ANNOTATIONS: &str = "zk:m1 secx:hasProperty [ secx:property secx:UnlinkabilityScope ; secx:level secx:PerPresentation ] .\n";
    const POLICY_PER: &str = "zk:cReqPer odrl:leftOperand secx:requiresUnlinkabilityScope ; odrl:operator odrl:gteq ; odrl:rightOperand secx:PerPresentation .\n";
    const POLICY_CROSS: &str = "zk:cReqCross odrl:leftOperand secx:requiresUnlinkabilityScope ; odrl:operator odrl:gteq ; odrl:rightOperand secx:CrossPresentation .\n";

    #[test]
    fn method_policy_without_precheck_data_is_refused() {
        let req = parse_request(
            ASK_AGE,
            &tr_triples(&[ISS_X], &[], None, Some("urn:policy1")),
            &nonce("n"),
        )
        .expect("parses");
        let holder = holder_mode1(ISS_X, "2026-07-01T00:00:00Z", "2026-07-10T00:00:00Z", true);
        assert_eq!(
            evaluate_contract(&req, &holder, None),
            Err(ExpressionError::MethodPolicyWithoutPrecheck)
        );
    }

    #[test]
    fn method_policy_inadmissible_method_is_refused_before_evaluation() {
        let req = parse_request(
            ASK_AGE,
            &tr_triples(&[ISS_X], &[], None, Some("urn:policy1")),
            &nonce("n"),
        )
        .expect("parses");
        let holder = holder_mode1(ISS_X, "2026-07-01T00:00:00Z", "2026-07-10T00:00:00Z", true);
        let pc = MethodPrecheck {
            policy: "urn:policy1",
            method: METHOD_M1,
            constraint_iris: &[CONSTRAINT_CROSS],
            policy_n3: POLICY_CROSS,
            annotations: M1_ANNOTATIONS,
        };
        // m1 holds PerPresentation; the policy requires gteq CrossPresentation.
        assert_eq!(
            evaluate_contract(&req, &holder, Some(&pc)),
            Err(ExpressionError::MethodNotAdmissible(vec![CONSTRAINT_CROSS.to_string()]))
        );
    }

    #[test]
    fn method_policy_precheck_for_a_different_policy_is_refused() {
        // TR names policy A; the supplied pre-check resolves policy B — data
        // that would be ADMISSIBLE under B. Fail-closed: the named policy can
        // never be substituted with a weaker one the holder picked.
        let req = parse_request(
            ASK_AGE,
            &tr_triples(&[ISS_X], &[], None, Some("urn:policyA")),
            &nonce("n"),
        )
        .expect("parses");
        let holder = holder_mode1(ISS_X, "2026-07-01T00:00:00Z", "2026-07-10T00:00:00Z", true);
        let pc = MethodPrecheck {
            policy: "urn:policyB",
            method: METHOD_M1,
            constraint_iris: &[CONSTRAINT_PER],
            policy_n3: POLICY_PER,
            annotations: M1_ANNOTATIONS,
        };
        assert_eq!(
            evaluate_contract(&req, &holder, Some(&pc)),
            Err(ExpressionError::MethodPolicyMismatch {
                required: "urn:policyA".to_string(),
                supplied: "urn:policyB".to_string(),
            })
        );
    }

    #[test]
    fn method_policy_admissible_method_proceeds_to_evaluation() {
        let req = parse_request(
            ASK_AGE,
            &tr_triples(&[ISS_X], &[], None, Some("urn:policy1")),
            &nonce("n"),
        )
        .expect("parses");
        let holder = holder_mode1(ISS_X, "2026-07-01T00:00:00Z", "2026-07-10T00:00:00Z", true);
        let pc = MethodPrecheck {
            policy: "urn:policy1",
            method: METHOD_M1,
            constraint_iris: &[CONSTRAINT_PER],
            policy_n3: POLICY_PER,
            annotations: M1_ANNOTATIONS,
        };
        let out = evaluate_contract(&req, &holder, Some(&pc)).expect("evaluates");
        assert_eq!(out.answer, ContractAnswer::Boolean(true));
    }

    // ── verify_response tamper cases ─────────────────────────────────────

    #[test]
    fn verify_response_refuses_a_nonce_mismatch() {
        let req = request(ASK_AGE, &[ISS_X], &[]);
        let holder = holder_mode1(ISS_X, "2026-07-01T00:00:00Z", "2026-07-10T00:00:00Z", true);
        let out = evaluate_contract(&req, &holder, None).expect("evaluates");
        let mut replayed = out.response;
        replayed.nonce = nonce("some-other-session");
        assert_eq!(verify_response(&req, &replayed), Err(ExpressionError::NonceMismatch));
    }

    #[test]
    fn verify_response_fails_closed_on_stripped_provenance() {
        let req = request(ASK_AGE, &[ISS_X], &[]);
        let holder = holder_mode1(ISS_X, "2026-07-01T00:00:00Z", "2026-07-10T00:00:00Z", true);
        let out = evaluate_contract(&req, &holder, None).expect("evaluates");
        // A tampered response keeping the claim but dropping the default-graph
        // provenance yields no admissible derivation on re-check.
        let mut tampered = out.response;
        tampered.dataset_form = tampered
            .dataset_form
            .lines()
            .skip_while(|l| !l.starts_with("GRAPH"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            verify_response(&req, &tampered).expect("re-checks"),
            ContractAnswer::Boolean(false)
        );
    }

    // ── mint_status_attestation (the status-list bridge, design D3) ──────

    #[test]
    fn mint_status_attestation_from_a_live_check_emits_the_window() {
        // as_of = 2026-07-01T00:00:00Z, freshness budget 9 days.
        let triples = mint_status_attestation(
            &nn("urn:b1"),
            &nn("urn:st1"),
            LiveStatus::Live,
            1_782_864_000,
            9 * 86_400,
        )
        .expect("mints");
        assert_eq!(triples.len(), 4);
        let text: Vec<String> = triples.iter().map(|t| t.to_string()).collect();
        assert!(
            text.iter().any(|t| t.contains(TRUSTX_STATUS_ATTESTATION)),
            "type triple: {:?}",
            text
        );
        assert!(
            text.iter()
                .any(|t| t.contains(TRUSTX_VALID_FROM) && t.contains("2026-07-01T00:00:00Z")),
            "validFrom window: {:?}",
            text
        );
        assert!(
            text.iter()
                .any(|t| t.contains(TRUSTX_VALID_UNTIL) && t.contains("2026-07-10T00:00:00Z")),
            "validUntil window: {:?}",
            text
        );
        assert!(
            text.iter().any(|t| t.contains(TRUSTX_COVERED_BY) && t.contains("urn:st1")),
            "coveredBy link: {:?}",
            text
        );
    }

    #[test]
    fn mint_status_attestation_epoch_window() {
        let triples =
            mint_status_attestation(&nn("urn:b"), &nn("urn:st"), LiveStatus::Live, 0, 0)
                .expect("mints");
        let text = triples[1].to_string();
        assert!(text.contains("1970-01-01T00:00:00Z"), "epoch lexical: {}", text);
    }

    #[test]
    fn mint_status_attestation_refuses_every_non_live_verdict() {
        for status in [LiveStatus::Set, LiveStatus::Unknown, LiveStatus::Stale] {
            assert_eq!(
                mint_status_attestation(&nn("urn:b"), &nn("urn:st"), status, 0, 60),
                Err(ExpressionError::NonPositiveStatus(status.reason())),
                "non-live verdict {:?} must not mint",
                status
            );
        }
    }

    #[test]
    fn mint_status_attestation_refuses_a_negative_window() {
        assert_eq!(
            mint_status_attestation(&nn("urn:b"), &nn("urn:st"), LiveStatus::Live, 0, -1),
            Err(ExpressionError::BadWindow)
        );
    }

    #[test]
    fn minted_attestation_round_trips_through_the_contract() {
        // End-to-end reuse: the bridge's triples ARE the coverage the rewrite
        // consumes — a holder graph whose only status attestation was minted
        // from a Live check binds; nothing else changed.
        let mut d = String::new();
        d.push_str(&format!("<urn:b1> <{}> <{}> .\n", PROV_WAS_ATTRIBUTED_TO, ISS_X));
        for t in mint_status_attestation(
            &nn("urn:b1"),
            &nn("urn:st1"),
            LiveStatus::Live,
            1_782_864_000,
            9 * 86_400,
        )
        .expect("mints")
        {
            d.push_str(&format!("{} .\n", t));
        }
        d.push_str(&format!(
            "GRAPH <urn:b1> {{ <urn:jesse> <{}> {} . }}\n",
            FOAF_AGE, AGE_25
        ));
        let holder = Graph::load_dataset(&d, "trig").expect("fixture parses");
        let req = request(ASK_AGE, &[ISS_X], &[]);
        let out = evaluate_contract(&req, &holder, None).expect("evaluates");
        assert_eq!(out.answer, ContractAnswer::Boolean(true));
    }

    // ── helpers + stable public constants ────────────────────────────────

    #[test]
    fn public_encoding_iris_are_stable() {
        assert_eq!(PROV_WAS_ATTRIBUTED_TO, "http://www.w3.org/ns/prov#wasAttributedTo");
        assert_eq!(RDF_REIFIES, "http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies");
    }

    #[test]
    fn utc_datetime_lexical_validation_is_strict() {
        // Valid: canonical, fractional seconds, boundary clock, and genuine leap days.
        assert!(is_utc_datetime_lexical("2026-07-05T00:00:00Z"));
        assert!(is_utc_datetime_lexical("2026-07-05T23:59:59.123Z"));
        assert!(is_utc_datetime_lexical("2024-02-29T00:00:00Z")); // 2024 divisible by 4 → leap
        assert!(is_utc_datetime_lexical("2000-02-29T12:30:45Z")); // 2000 divisible by 400 → leap
        for bad in [
            // structural
            "2026-07-05T00:00:00",       // no zone
            "2026-07-05T00:00:00+02:00", // non-UTC offset
            "2026-07-05 00:00:00Z",      // no T
            "garbage",
            "2026-07-05T00:00:00.Z", // empty fraction
            // calendar range
            "2026-99-99T99:99:99Z", // every field out of range
            "2026-00-05T00:00:00Z", // month 00
            "2026-13-05T00:00:00Z", // month 13
            "2026-07-00T00:00:00Z", // day 00
            "2026-07-32T00:00:00Z", // day 32
            "2026-02-30T00:00:00Z", // February never has 30 days
            "2026-04-31T00:00:00Z", // April has 30 days
            "2026-02-29T00:00:00Z", // 2026 is NOT a leap year
            "1900-02-29T00:00:00Z", // 1900 divisible by 100 but not 400 → not leap
            // clock range
            "2026-07-05T24:00:00Z", // hour 24
            "2026-07-05T00:60:00Z", // minute 60
            "2026-07-05T00:00:60Z", // second 60 (xsd:dateTime forbids leap seconds)
        ] {
            assert!(!is_utc_datetime_lexical(bad), "must reject: {}", bad);
        }
    }

    #[test]
    fn unix_to_datetime_lexical_matches_known_instants() {
        assert_eq!(unix_to_datetime_lexical(0), "1970-01-01T00:00:00Z");
        assert_eq!(unix_to_datetime_lexical(86_399), "1970-01-01T23:59:59Z");
        assert_eq!(unix_to_datetime_lexical(1_783_209_600), T);
        assert_eq!(unix_to_datetime_lexical(1_782_000_000), "2026-06-21T00:00:00Z");
    }
}
