// [GPT-6] Research-stage selected-result contract; not externally audited (sq-qhy4).
//! Builds and verifies bounded authenticated result witnesses with private intermediate data.
//!
//! Enable `successful-results` explicitly. This additive contract does not change
//! the legacy complete-scan verifier. It proves only support for released distinct
//! answers, not complete query answers, absence, wallet contents, or holder identity.
//! The public transcript contains the query, released terms, a fresh challenge,
//! one or two issuer key slots and the verifier's accepted status-policy root. Graph
//! roots, sizes, salts, status references and selected witness attributions are
//! private circuit inputs. Issuer identities, fixed capacities and result size
//! remain visible. Selected blank nodes and triple terms are rejected.
//!
//! Private integer predicates retain lexical-to-value binding and accept only
//! canonical nonnegative `xsd:integer` values through [`MAX_PRIVATE_INTEGER`].
//! Public predicates run in the independent verifier. An all-public predicate
//! query selects the member without any numeric predicate circuitry.

use crate::driver::{CircuitProver, DriverError};
use crate::manifest::{DisclosedTerm, FieldHex, StatusListSnapshot};
use crate::planner::signed::{NumericProfile, SignedDisclosureQuery};
use crate::planner::{
    optimize_disclosure_admitted, plan_disclosure_admitted, DisclosureQuery, MembershipRef,
    OptimizationCompletion, OptimizationLimits, PlannerLimits, QueryKind, QuerySlot,
};
use crate::revocation::{
    accepted_set_root, accepted_set_witness, merkle_root, merkle_witness, AcceptedStatusEntry,
};
use crate::verifier::{SeenNonces, VerifierNonce};
use oxrdf::{NamedOrBlankNode, Term, Triple};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sparq_zk::commit::GraphCommitment;
use sparq_zk::encode::encode_term;
use sparq_zk::field::{
    field_from_hash_bytes, field_from_hex_str, field_to_be_bytes_32, field_to_hex, Fr,
};
use sparq_zk::sig::{self, PublicKey, Signature};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

/// Maximum canonical nonnegative private integer accepted by the full-width profile.
pub const MAX_PRIVATE_INTEGER: u64 = u64::MAX;
const SMALL_PRIVATE_INTEGER: u64 = 99;
const MAX_CREDENTIALS: usize = 2;
const N: usize = 16;
const P: usize = 3;
const R: usize = 4;
const V: usize = 6;
const F: usize = 2;
const STATUS_DEPTHS: [u32; 3] = [10, 17, 20];
const POLICY_DEPTH: u32 = 4;

/// Separately versioned signed-integer successful-result preparation and verification.
pub mod signed;

// [GPT-6] Only the typed entry point selects the numeric contract. A signed
// capacity is never added to the legacy presentation's deserialization enum.
#[derive(Clone, Copy)]
enum NumericContract {
    Unsigned(PrivateIntegerCapacity),
    Signed,
}

impl NumericContract {
    fn profile(self) -> NumericProfile {
        match self {
            Self::Unsigned(_) => NumericProfile::Unsigned,
            Self::Signed => NumericProfile::Signed,
        }
    }

    fn parse(self, query: &str) -> Result<DisclosureQuery, ResultError> {
        parse_numeric_query(query, self.profile())
    }

    fn accepts_version(self, version: u32) -> bool {
        match self {
            Self::Unsigned(_) => matches!(version, 1 | 2),
            Self::Signed => version == signed::VERSION,
        }
    }

    fn package(
        self,
        credentials: usize,
        filters: usize,
        depth: u32,
    ) -> Result<(&'static str, u32), ResultError> {
        match self {
            Self::Unsigned(capacity) => package(credentials, filters, capacity, depth),
            Self::Signed => signed::package(credentials, filters, depth),
        }
    }
}

fn parse_numeric_query(
    query: &str,
    profile: NumericProfile,
) -> Result<DisclosureQuery, ResultError> {
    match profile {
        NumericProfile::Unsigned => DisclosureQuery::parse(query),
        NumericProfile::Signed => SignedDisclosureQuery::parse(query).map(|q| q.inner),
    }
    .map_err(|e| reject(e.to_string()))
}

/// A private credential and its existing issuer-authenticated status reference.
///
/// The signature must cover `commitment_message_with_status(root, salt,
/// status_ref_digest(list, index, version))`. No signature conversion or holder
/// re-attestation occurs. This type deliberately has no serialization or Debug.
pub struct ResultCredential {
    /// Canonical string-encoded graph signed by the issuer.
    pub graph: GraphCommitment,
    /// Issuer verification key.
    pub issuer: PublicKey,
    /// Signature over this graph, salt and exact status reference.
    pub signature: Signature,
    /// Issuer-signed status-list IRI, kept private during presentation.
    pub status_list: String,
    /// Issuer-signed list version.
    pub status_version: u64,
    /// Issuer-signed index, kept private during presentation.
    pub status_index: u64,
}

/// Relying-party trust inputs, resolved and authenticated outside this library.
///
/// Supply the verifier's own issuer allow-list and status snapshots. The prover
/// cannot replace these through the presentation. Versions outside the inclusive
/// interval are excluded from the accepted set; conflicting snapshots reject.
/// The largest accepted snapshot selects depth 10, 17 or 20 for the complete
/// policy. Every snapshot must fit that tree in full; none is truncated.
#[derive(Debug, Clone)]
pub struct ResultPolicy {
    /// Issuers the relying party accepts.
    pub trusted_issuers: Vec<PublicKey>,
    /// Independently authenticated status snapshots.
    pub snapshots: Vec<StatusListSnapshot>,
    /// Oldest accepted publication version.
    pub min_version: u64,
    /// Newest accepted publication version.
    pub max_version: u64,
}

// [GPT-6] Capacity is public; the exact decimal length remains a private witness.
/// Public private-integer capacity selected by the successful-result proof.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivateIntegerCapacity {
    /// Canonical integers from zero through 99, preserving the small circuit lane.
    #[default]
    TwoDigits,
    /// Canonical integers through u64::MAX, with no public decimal-length selector.
    FullU64,
}

impl PrivateIntegerCapacity {
    fn is_default(&self) -> bool {
        *self == Self::TwoDigits
    }
}

/// Public presentation for the versioned successful-result contract.
///
/// Deliberately omits a prover-selected verification key, circuit identifier,
/// public-input vector, graph roots, status references and hidden term handles.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResultPresentation {
    /// Exact statement version; unknown values reject.
    pub version: u32,
    /// Query that the verifier additionally matches to its expected request.
    pub query: String,
    /// Distinct released rows, keyed by variable name without `?`.
    pub rows: Vec<BTreeMap<String, DisclosedTerm>>,
    /// Fresh relying-party challenge.
    pub challenge: FieldHex,
    /// Public capacity bucket: one or two issuer slots, selected by prover policy.
    pub issuer_slots: Vec<String>,
    /// Public numeric capacity; omitted legacy values select the small profile.
    /// The verifier derives the canonical circuit from this capacity and its policy.
    #[serde(default, skip_serializing_if = "PrivateIntegerCapacity::is_default")]
    pub integer_capacity: PrivateIntegerCapacity,
    /// Barretenberg proof bytes under the explicitly ZK `noir-recursive` target.
    pub proof: Vec<u8>,
}

// A borrowed view avoids cloning an untrusted presentation before admission.
// Numeric interpretation is supplied separately by the typed verifier entry point.
struct PresentationView<'a> {
    version: u32,
    query: &'a str,
    rows: &'a [BTreeMap<String, DisclosedTerm>],
    challenge: &'a FieldHex,
    issuer_slots: &'a [String],
}

impl<'a> From<&'a ResultPresentation> for PresentationView<'a> {
    fn from(p: &'a ResultPresentation) -> Self {
        Self {
            version: p.version,
            query: &p.query,
            rows: &p.rows,
            challenge: &p.challenge,
            issuer_slots: &p.issuer_slots,
        }
    }
}

/// Private preparation statistics; these do not enter the public presentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultWork {
    /// Distinct credentials selected from the input wallet.
    pub selected_credentials: usize,
    /// Distinct selected credential/leaf pairs before repeated uses.
    pub shared_memberships: usize,
    /// Selected witness uses across every released row and pattern.
    pub witness_uses: usize,
    /// Host-checked public predicate evaluations.
    pub public_predicates: usize,
    /// Private predicate evaluations in the circuit.
    pub private_predicates: usize,
    /// Fixed signature slots actually checked by this bounded member.
    pub signature_checks: usize,
    /// Joint search completion; absent for the explicit first-success baseline.
    ///
    /// BudgetExhausted means the returned complete plan is feasible, without an
    /// established optimum. These diagnostics never enter the presentation.
    pub optimization: Option<OptimizationCompletion>,
}

/// Prover-only preparation, including sensitive circuit inputs.
///
/// This type intentionally has neither Debug nor Serialize. Its private input
/// file is materialized only by the existing nargo driver during proving.
pub struct PreparedResult {
    presentation: ResultPresentation,
    package: &'static str,
    toml: String,
    public_inputs: Vec<u8>,
    work: ResultWork,
}

/// A fail-closed result-building or verification error.
#[derive(Debug)]
pub enum ResultError {
    /// Malformed, unsupported, over-capacity, unauthenticated, or inconsistent input.
    Rejected(String),
    /// Search ended before finding a complete feasible plan; infeasibility is unknown.
    SearchExhausted,
    /// Prover/backend failure; never interpreted as successful verification.
    Driver(DriverError),
}

impl std::fmt::Display for ResultError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rejected(s) => write!(f, "successful-result rejected: {s}"),
            Self::SearchExhausted => write!(
                f,
                "successful-result search exhausted without a feasible plan"
            ),
            Self::Driver(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for ResultError {}
impl From<DriverError> for ResultError {
    fn from(e: DriverError) -> Self {
        Self::Driver(e)
    }
}
fn reject(message: impl Into<String>) -> ResultError {
    ResultError::Rejected(message.into())
}

impl ResultPolicy {
    // Select from all freshness-accepted snapshots, not only the prover's chosen list.
    fn status_depth(&self) -> Result<u32, ResultError> {
        let bytes = self
            .snapshots
            .iter()
            .filter(|s| (self.min_version..=self.max_version).contains(&s.version))
            .map(|s| s.bits.len())
            .max()
            .unwrap_or(0);
        STATUS_DEPTHS
            .into_iter()
            .find(|depth| bytes <= (1usize << depth) / 8)
            .ok_or_else(|| reject("accepted status snapshot exceeds maximum tree capacity"))
    }

    fn entries(&self) -> Result<Vec<AcceptedStatusEntry>, ResultError> {
        if self.min_version > self.max_version {
            return Err(reject("inverted status freshness interval"));
        }
        if self.trusted_issuers.is_empty()
            || self.trusted_issuers.iter().any(|k| !k.is_prime_order())
        {
            return Err(reject("empty or invalid issuer trust set"));
        }
        let mut unique = BTreeMap::new();
        for snapshot in &self.snapshots {
            if snapshot.version < self.min_version || snapshot.version > self.max_version {
                continue;
            }
            oxrdf::NamedNode::new(&snapshot.status_list)
                .map_err(|_| reject("invalid status-list IRI"))?;
            let key = (snapshot.status_list.clone(), snapshot.version);
            if let Some(previous) = unique.insert(key, snapshot) {
                if previous != snapshot {
                    return Err(reject("conflicting authoritative status snapshots"));
                }
            }
        }
        if unique.is_empty() || unique.len() > (1 << POLICY_DEPTH) {
            return Err(reject("accepted status-policy capacity"));
        }
        let depth = self.status_depth()?;
        unique
            .into_values()
            .map(|s| {
                Ok(AcceptedStatusEntry {
                    status_list: s.status_list.clone(),
                    version: s.version,
                    status_list_root: merkle_root(s, depth)
                        .ok_or_else(|| reject("status root unavailable"))?,
                })
            })
            .collect()
    }
}

fn package(
    credentials: usize,
    hidden_filters: usize,
    capacity: PrivateIntegerCapacity,
    depth: u32,
) -> Result<(&'static str, u32), ResultError> {
    let bits = match (hidden_filters == 0, capacity) {
        (true, PrivateIntegerCapacity::TwoDigits) => 0,
        (true, PrivateIntegerCapacity::FullU64) => {
            return Err(reject("numeric capacity without a private predicate"))
        }
        (false, PrivateIntegerCapacity::TwoDigits) => 8,
        (false, PrivateIntegerCapacity::FullU64) => 64,
    };
    match (credentials, bits, depth) {
        (1, 0, 10) => Ok(("result_v1_k1_n16_p3_r4_f0", 1)),
        (1, 0, 17) => Ok(("result_v2_k1_n16_p3_r4_f0_i0_d17", 2)),
        (1, 0, 20) => Ok(("result_v2_k1_n16_p3_r4_f0_i0_d20", 2)),
        (1, 8, 10) => Ok(("result_v1_k1_n16_p3_r4_f2", 1)),
        (1, 8, 17) => Ok(("result_v2_k1_n16_p3_r4_f2_i8_d17", 2)),
        (1, 8, 20) => Ok(("result_v2_k1_n16_p3_r4_f2_i8_d20", 2)),
        (1, 64, 10) => Ok(("result_v2_k1_n16_p3_r4_f2_i64_d10", 2)),
        (1, 64, 17) => Ok(("result_v2_k1_n16_p3_r4_f2_i64_d17", 2)),
        (1, 64, 20) => Ok(("result_v2_k1_n16_p3_r4_f2_i64_d20", 2)),
        (2, 0, 10) => Ok(("result_v1_k2_n16_p3_r4_f0", 1)),
        (2, 0, 17) => Ok(("result_v2_k2_n16_p3_r4_f0_i0_d17", 2)),
        (2, 0, 20) => Ok(("result_v2_k2_n16_p3_r4_f0_i0_d20", 2)),
        (2, 8, 10) => Ok(("result_v1_k2_n16_p3_r4_f2", 1)),
        (2, 8, 17) => Ok(("result_v2_k2_n16_p3_r4_f2_i8_d17", 2)),
        (2, 8, 20) => Ok(("result_v2_k2_n16_p3_r4_f2_i8_d20", 2)),
        (2, 64, 10) => Ok(("result_v2_k2_n16_p3_r4_f2_i64_d10", 2)),
        (2, 64, 17) => Ok(("result_v2_k2_n16_p3_r4_f2_i64_d17", 2)),
        (2, 64, 20) => Ok(("result_v2_k2_n16_p3_r4_f2_i64_d20", 2)),
        _ => Err(reject("unsupported result capacity bucket")),
    }
}

fn variables(query: &DisclosureQuery) -> Vec<String> {
    query
        .patterns
        .iter()
        .flatten()
        .filter_map(|s| match s {
            QuerySlot::Variable(v) => Some(v.clone()),
            QuerySlot::Constant(_) => None,
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn disclosed(term: &Term) -> Result<DisclosedTerm, ResultError> {
    match term {
        Term::NamedNode(n) => Ok(DisclosedTerm::Iri {
            value: n.as_str().into(),
        }),
        Term::Literal(l) => Ok(DisclosedTerm::Literal {
            value: l.value().into(),
            datatype: Some(l.datatype().as_str().into()),
            language: l.language().map(str::to_owned),
        }),
        _ => Err(reject(
            "blank nodes and triple terms are outside the result contract",
        )),
    }
}

fn field(term: &Term) -> Result<Fr, ResultError> {
    disclosed(term)?;
    encode_term(term, &Fr::from(0u64)).ok_or_else(|| reject("unencodable public term"))
}

fn term_parts(triple: &Triple) -> [Term; 3] {
    let subject = match &triple.subject {
        NamedOrBlankNode::NamedNode(n) => Term::NamedNode(n.clone()),
        NamedOrBlankNode::BlankNode(n) => Term::BlankNode(n.clone()),
    };
    [
        subject,
        Term::NamedNode(triple.predicate.clone()),
        triple.object.clone(),
    ]
}

fn type_opening(term: &Term) -> Result<(u32, Fr), ResultError> {
    let (ty, bytes) = match term {
        Term::NamedNode(n) => (1, n.as_str().as_bytes().to_vec()),
        Term::Literal(l) => (2, l.to_string().into_bytes()),
        _ => {
            return Err(reject(
                "selected blank nodes and triple terms are unsupported",
            ))
        }
    };
    Ok((ty, field_from_hash_bytes(blake3::hash(&bytes).as_bytes())))
}

#[derive(Debug)]
struct PublicStatement {
    query: DisclosureQuery,
    vars: Vec<String>,
    rows: Vec<BTreeMap<String, Term>>,
    hidden_filters: Vec<usize>,
    entries: Vec<AcceptedStatusEntry>,
    fields: Vec<(&'static str, Value)>,
    status_depth: u32,
    package: &'static str,
}

fn public_statement(
    p: &ResultPresentation,
    policy: &ResultPolicy,
    nonce: &VerifierNonce,
) -> Result<PublicStatement, ResultError> {
    public_statement_for(
        p.into(),
        NumericContract::Unsigned(p.integer_capacity),
        policy,
        nonce,
    )
}

fn public_statement_for(
    p: PresentationView<'_>,
    contract: NumericContract,
    policy: &ResultPolicy,
    nonce: &VerifierNonce,
) -> Result<PublicStatement, ResultError> {
    if !contract.accepts_version(p.version) || p.challenge != &nonce.as_field_hex() {
        return Err(reject("version or challenge mismatch"));
    }
    if p.issuer_slots.is_empty() || p.issuer_slots.len() > MAX_CREDENTIALS {
        return Err(reject("unsupported credential capacity bucket"));
    }
    let query = contract.parse(p.query)?;
    // Keep ASK out of this first wire contract; no accidental false/absence claim.
    if query.kind != QueryKind::SelectDistinct {
        return Err(reject("only SELECT DISTINCT is supported"));
    }
    let vars = variables(&query);
    if vars.len() > V
        || query.patterns.is_empty()
        || query.patterns.len() > P
        || p.rows.is_empty()
        || p.rows.len() > R
    {
        return Err(reject("successful-result query or row capacity"));
    }
    let expected: BTreeSet<_> = query.projection.iter().cloned().collect();
    if expected.is_empty() || !expected.iter().all(|v| vars.contains(v)) {
        return Err(reject("unbound projection"));
    }
    let mut rows = Vec::new();
    let mut distinct = BTreeSet::new();
    for row in p.rows {
        if row.keys().cloned().collect::<BTreeSet<_>>() != expected {
            return Err(reject("released row does not match projection"));
        }
        let row: BTreeMap<String, Term> = row
            .iter()
            .map(|(v, t)| {
                Ok((
                    v.clone(),
                    t.to_term()
                        .ok_or_else(|| reject("malformed released RDF term"))?,
                ))
            })
            .collect::<Result<_, ResultError>>()?;
        if !distinct.insert(
            row.iter()
                .map(|(v, t)| (v.clone(), t.to_string()))
                .collect::<Vec<_>>(),
        ) {
            return Err(reject("duplicate released DISTINCT row"));
        }
        rows.push(row);
    }
    let hidden_filters: Vec<_> = query
        .filters
        .iter()
        .enumerate()
        .filter_map(|(i, f)| (!expected.contains(&f.variable)).then_some(i))
        .collect();
    if hidden_filters.len() > F {
        return Err(reject("private filter capacity"));
    }
    for filter in &query.filters {
        if !vars.contains(&filter.variable) {
            return Err(reject("unbound FILTER variable"));
        }
        if expected.contains(&filter.variable) {
            for row in &rows {
                let value = contract
                    .profile()
                    .value(&row[&filter.variable])
                    .ok_or_else(|| {
                        reject(match contract {
                            NumericContract::Unsigned(_) => {
                                "public FILTER operand is not a canonical nonnegative integer"
                            }
                            NumericContract::Signed => {
                                "public FILTER operand is not a canonical signed integer"
                            }
                        })
                    })?;
                if !crate::planner::integer_comparison(value, filter.op, filter.bound) {
                    return Err(reject("public FILTER is false"));
                }
            }
        }
    }
    let entries = policy.entries()?;
    let status_depth = policy.status_depth()?;
    let (package, version) =
        contract.package(p.issuer_slots.len(), hidden_filters.len(), status_depth)?;
    if p.version != version {
        return Err(reject(
            "version does not match independently derived capacity profile",
        ));
    }
    let root = accepted_set_root(&entries, POLICY_DEPTH)
        .ok_or_else(|| reject("status policy root unavailable"))?;
    let mut keys = Vec::new();
    for text in p.issuer_slots {
        let key = sig::public_key_from_hex(text).ok_or_else(|| reject("malformed issuer key"))?;
        if sig::public_key_to_hex(&key) != *text || !policy.trusted_issuers.contains(&key) {
            return Err(reject("untrusted or noncanonical issuer key"));
        }
        let (x, y) = key.coords().ok_or_else(|| reject("identity issuer key"))?;
        keys.push([field_to_hex(&x), field_to_hex(&y)]);
    }
    if p.issuer_slots.windows(2).any(|pair| pair[0] > pair[1]) {
        return Err(reject("noncanonical issuer slot order"));
    }
    let mut pattern_vars = [[0u32; 3]; P];
    let zero = field_to_hex(&Fr::from(0u64));
    let mut pattern_constants = vec![vec![zero.clone(); 3]; P];
    for (i, pattern) in query.patterns.iter().enumerate() {
        for (s, slot) in pattern.iter().enumerate() {
            match slot {
                QuerySlot::Variable(v) => {
                    pattern_vars[i][s] = (vars
                        .iter()
                        .position(|x| x == v)
                        .ok_or_else(|| reject("unbound pattern variable"))?
                        + 1) as u32
                }
                QuerySlot::Constant(t) => pattern_constants[i][s] = field_to_hex(&field(t)?),
            }
        }
    }
    let mut projection = [false; V];
    let mut result_enc = vec![vec![zero; V]; R];
    for (i, v) in vars.iter().enumerate() {
        projection[i] = expected.contains(v);
        if projection[i] {
            for (r, row) in rows.iter().enumerate() {
                result_enc[r][i] = field_to_hex(&field(&row[v])?);
            }
        }
    }
    let mut fields = vec![
        ("challenge", json!(p.challenge.0)),
        ("version", json!(p.version)),
        ("accepted_root", json!(field_to_hex(&root))),
        ("issuer_keys", json!(keys)),
        ("pattern_count", json!(query.patterns.len())),
        ("pattern_vars", json!(pattern_vars)),
        ("pattern_constants", json!(pattern_constants)),
        ("projection", json!(projection)),
        ("result_count", json!(rows.len())),
        ("results", json!(result_enc)),
        ("filter_count", json!(hidden_filters.len())),
    ];
    if !hidden_filters.is_empty() {
        let mut fvars = [1u32; F];
        let mut ops = [0u32; F];
        let mut bounds = [0u64; F];
        for (f, &index) in hidden_filters.iter().enumerate() {
            let filter = &query.filters[index];
            fvars[f] = (vars
                .iter()
                .position(|v| v == &filter.variable)
                .ok_or_else(|| reject("unbound FILTER"))?
                + 1) as u32;
            ops[f] = filter.op.code();
            bounds[f] = filter.bound;
        }
        fields.extend([
            ("filter_vars", json!(fvars)),
            ("filter_ops", json!(ops)),
            ("filter_bounds", json!(bounds)),
        ]);
    }
    Ok(PublicStatement {
        query,
        vars,
        rows,
        hidden_filters,
        entries,
        fields,
        status_depth,
        package,
    })
}

fn public_bytes(fields: &[(&str, Value)]) -> Result<Vec<u8>, ResultError> {
    fn append(value: &Value, out: &mut Vec<u8>) -> Result<(), ResultError> {
        let f = match value {
            Value::Array(v) => {
                for x in v {
                    append(x, out)?;
                }
                return Ok(());
            }
            Value::Bool(b) => Fr::from(u64::from(*b)),
            Value::Number(n) => {
                Fr::from(n.as_u64().ok_or_else(|| reject("invalid public integer"))?)
            }
            Value::String(s) => {
                field_from_hex_str(s).ok_or_else(|| reject("invalid public field"))?
            }
            _ => return Err(reject("invalid public-input shape")),
        };
        out.extend_from_slice(&field_to_be_bytes_32(&f));
        Ok(())
    }
    let mut out = Vec::new();
    for (_, value) in fields {
        append(value, &mut out)?;
    }
    Ok(out)
}

/// Selects the public credential-capacity bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CredentialCapacity {
    /// Use one signature slot when possible, revealing that smaller capacity.
    #[default]
    Smallest,
    /// Always use two slots, repeating a private credential when necessary.
    HideInTwo,
}

/// Selects the local witness-search strategy for a fixed released result set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WitnessSelection {
    /// Jointly minimize authentications then memberships within the search budget.
    #[default]
    Optimize,
    /// Retain deterministic first-success selection for controlled comparisons.
    FirstSuccess,
}

/// Chooses whether to disclose the small private-integer capacity.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum IntegerCapacityPolicy {
    /// Use the small member when every selected private operand is at most 99.
    #[default]
    Smallest,
    /// Use the full-u64 member for any private predicate, hiding the small bucket.
    HideInU64,
}

/// Prover-side choices that never waive independent verifier checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResultOptions {
    /// Smallest selects the smallest admitted circuit; HideInTwo retains two issuer slots.
    pub credential_capacity: CredentialCapacity,
    /// Public range-capacity disclosure; neither choice discloses exact digit length.
    pub integer_capacity: IntegerCapacityPolicy,
    /// Joint bounded optimization is the default; FirstSuccess provides a baseline.
    pub witness_selection: WitnessSelection,
    /// Maximum candidate triple attempts, including rejected candidates.
    pub max_search_steps: usize,
}

impl Default for ResultOptions {
    fn default() -> Self {
        Self {
            credential_capacity: CredentialCapacity::default(),
            integer_capacity: IntegerCapacityPolicy::default(),
            witness_selection: WitnessSelection::default(),
            max_search_steps: PlannerLimits::default().max_search_steps,
        }
    }
}

/// Prepares private successful witnesses and removes irrelevant wallet credentials.
///
/// # Errors
/// Rejects unsupported syntax, unsubstantiated results, failed authentication or
/// status, selected blank nodes, and inputs outside the fixed circuit capacity.
/// Returns [`ResultError::SearchExhausted`] when the optimization budget ends
/// without a complete feasible assignment. A complete feasible incumbent can be
/// used after exhaustion; [`PreparedResult::work`] records that completion state.
pub fn prepare_result(
    query: &str,
    credentials: &[ResultCredential],
    rows: &[BTreeMap<String, Term>],
    policy: &ResultPolicy,
    nonce: &VerifierNonce,
) -> Result<PreparedResult, ResultError> {
    prepare_result_with_options(
        query,
        credentials,
        rows,
        policy,
        nonce,
        ResultOptions::default(),
    )
}

/// Prepares a result with an explicit public capacity-disclosure choice.
///
/// # Errors
/// Returns the same fail-closed errors as [`prepare_result`].
pub fn prepare_result_with_options(
    query: &str,
    credentials: &[ResultCredential],
    rows: &[BTreeMap<String, Term>],
    policy: &ResultPolicy,
    nonce: &VerifierNonce,
    options: ResultOptions,
) -> Result<PreparedResult, ResultError> {
    prepare_result_numeric(
        query,
        credentials,
        rows,
        policy,
        nonce,
        options,
        NumericProfile::Unsigned,
    )
}

fn prepare_result_numeric(
    query: &str,
    credentials: &[ResultCredential],
    rows: &[BTreeMap<String, Term>],
    policy: &ResultPolicy,
    nonce: &VerifierNonce,
    options: ResultOptions,
    profile: NumericProfile,
) -> Result<PreparedResult, ResultError> {
    // [GPT-6] Reject the wallet shape before authentication or graph cloning;
    // empty and ineligible credentials still count toward resource admission.
    if credentials.len() > crate::planner::MAX_DISCLOSURE_CREDENTIALS {
        return Err(reject("input credentials exceed disclosure planning limit"));
    }
    let parsed = parse_numeric_query(query, profile)?;
    let entries = policy.entries()?;
    let status_depth = policy.status_depth()?;
    let eligible: Vec<bool> = credentials
        .iter()
        .map(|c| {
            if c.graph.canonical.triples.is_empty()
                || c.graph.canonical.triples.len() > N
                || !policy.trusted_issuers.contains(&c.issuer)
                || !entries
                    .iter()
                    .any(|e| e.status_list == c.status_list && e.version == c.status_version)
            {
                return false;
            }
            let Some(snapshot) = policy
                .snapshots
                .iter()
                .find(|s| s.status_list == c.status_list && s.version == c.status_version)
            else {
                return false;
            };
            if snapshot.bit(c.status_index) || c.status_index >= (1 << status_depth) {
                return false;
            }
            let leaves = c
                .graph
                .canonical
                .triples
                .iter()
                .map(|t| sparq_zk::encode::encode_triple(t, &c.graph.salt))
                .collect::<Option<Vec<_>>>();
            if !leaves
                .is_some_and(|leaves| sparq_zk::poseidon2::hash(&leaves) == c.graph.commitment)
            {
                return false;
            }
            let reference = sig::status_ref_digest(
                &sig::status_list_id_to_field(&c.status_list),
                c.status_index,
                c.status_version,
            );
            let message =
                sig::commitment_message_with_status(&c.graph.commitment, &c.graph.salt, &reference);
            sig::verify(&c.issuer, &message, &c.signature)
        })
        .collect();
    let graphs: Vec<_> = credentials.iter().map(|c| c.graph.clone()).collect();
    let admit = |pattern: usize, witness: MembershipRef, triple: &Triple| {
        if !eligible[witness.credential] {
            return false;
        }
        term_parts(triple).iter().enumerate().all(|(slot, term)| {
            if !matches!(term, Term::NamedNode(_) | Term::Literal(_)) {
                return false;
            }
            if let QuerySlot::Variable(v) = &parsed.patterns[pattern][slot] {
                if !parsed.projection.contains(v) && parsed.filters.iter().any(|f| &f.variable == v)
                {
                    return profile.value(term).is_some();
                }
            }
            true
        })
    };
    let limits = PlannerLimits {
        max_patterns: P,
        max_results: R,
        max_search_steps: options.max_search_steps,
    };
    let (plan, optimization) = match options.witness_selection {
        WitnessSelection::FirstSuccess => (
            match profile {
                NumericProfile::Unsigned => {
                    plan_disclosure_admitted(&parsed, &graphs, rows, limits, admit)
                }
                NumericProfile::Signed => crate::planner::signed::plan_signed_disclosure_admitted(
                    &SignedDisclosureQuery {
                        inner: parsed.clone(),
                    },
                    &graphs,
                    rows,
                    limits,
                    admit,
                ),
            }
            .map_err(|e| reject(e.to_string()))?,
            None,
        ),
        WitnessSelection::Optimize => {
            let optimization_limits = OptimizationLimits {
                planner: limits,
                max_pattern_occurrences: P * R,
                max_authentications: MAX_CREDENTIALS,
            };
            let report = match profile {
                NumericProfile::Unsigned => {
                    optimize_disclosure_admitted(&parsed, &graphs, rows, optimization_limits, admit)
                }
                NumericProfile::Signed => {
                    crate::planner::signed::optimize_signed_disclosure_admitted(
                        &SignedDisclosureQuery {
                            inner: parsed.clone(),
                        },
                        &graphs,
                        rows,
                        optimization_limits,
                        admit,
                    )
                }
            }
            .map_err(|e| reject(e.to_string()))?;
            let plan = report.plan.ok_or_else(|| match report.completion {
                OptimizationCompletion::BudgetExhausted => ResultError::SearchExhausted,
                _ => reject("no feasible successful-result witness within credential capacity"),
            })?;
            (plan, Some(report.completion))
        }
    };
    let mut used: Vec<usize> = plan
        .rows
        .iter()
        .flat_map(|r| r.witnesses.iter().map(|w| w.credential))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if used.is_empty() || used.len() > MAX_CREDENTIALS {
        return Err(reject("selected credential capacity"));
    }
    used.sort_by_key(|&i| sig::public_key_to_hex(&credentials[i].issuer));
    let selected_credentials = used.len();
    while options.credential_capacity == CredentialCapacity::HideInTwo
        && used.len() < MAX_CREDENTIALS
    {
        used.push(used[0]);
    }
    let hidden_filters = parsed
        .filters
        .iter()
        .filter(|f| !parsed.projection.contains(&f.variable))
        .count();
    let mut needs_wide = false;
    'selected_operands: for row in &plan.rows {
        for (pattern, witness) in row.witnesses.iter().enumerate() {
            let triple = &credentials[witness.credential].graph.canonical.triples[witness.leaf];
            for (slot, term) in term_parts(triple).iter().enumerate() {
                let QuerySlot::Variable(variable) = &parsed.patterns[pattern][slot] else {
                    continue;
                };
                let private_operand = !parsed.projection.contains(variable)
                    && parsed
                        .filters
                        .iter()
                        .any(|filter| &filter.variable == variable);
                if private_operand
                    && profile
                        .value(term)
                        .is_some_and(|value| value > SMALL_PRIVATE_INTEGER)
                {
                    needs_wide = true;
                    break 'selected_operands;
                }
            }
        }
    }
    let integer_capacity = if hidden_filters != 0
        && (matches!(profile, NumericProfile::Signed)
            || needs_wide
            || options.integer_capacity == IntegerCapacityPolicy::HideInU64)
    {
        PrivateIntegerCapacity::FullU64
    } else {
        PrivateIntegerCapacity::TwoDigits
    };
    let contract = match profile {
        NumericProfile::Unsigned => NumericContract::Unsigned(integer_capacity),
        NumericProfile::Signed => NumericContract::Signed,
    };
    let (_, version) = contract.package(used.len(), hidden_filters, status_depth)?;
    // Shared private storage only: the signed wrapper emits its separate wire
    // type, omitting this unsigned-only capacity field, after proving succeeds.
    let presentation = ResultPresentation {
        version,
        integer_capacity,
        query: query.into(),
        challenge: nonce.as_field_hex(),
        rows: rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|(v, t)| Ok((v.clone(), disclosed(t)?)))
                    .collect()
            })
            .collect::<Result<_, ResultError>>()?,
        issuer_slots: used
            .iter()
            .map(|&i| sig::public_key_to_hex(&credentials[i].issuer))
            .collect(),
        proof: Vec::new(),
    };
    let statement = public_statement_for((&presentation).into(), contract, policy, nonce)?;
    let public_inputs = public_bytes(&statement.fields)?;
    let mut fields = statement.fields.clone();
    let zero = field_to_hex(&Fr::from(0u64));
    let mut counts = vec![0usize; used.len()];
    let mut salts = Vec::new();
    let mut enc = vec![vec![vec![zero.clone(); 3]; N]; used.len()];
    let mut status_lists = Vec::new();
    let mut status_versions = Vec::new();
    let mut status_indices = Vec::new();
    let mut status_roots = Vec::new();
    let mut status_siblings = Vec::new();
    let mut policy_indices = Vec::new();
    let mut policy_siblings = Vec::new();
    let mut signatures = Vec::new();
    for (slot, &original) in used.iter().enumerate() {
        let c = &credentials[original];
        counts[slot] = c.graph.canonical.triples.len();
        if counts[slot] == 0 || counts[slot] > N {
            return Err(reject("selected graph capacity"));
        }
        for (i, t) in c.graph.canonical.triples.iter().enumerate() {
            for (s, term) in term_parts(t).iter().enumerate() {
                enc[slot][i][s] = field_to_hex(
                    &encode_term(term, &c.graph.salt)
                        .ok_or_else(|| reject("unsupported graph term"))?,
                );
            }
        }
        let policy_index = statement
            .entries
            .iter()
            .position(|e| e.status_list == c.status_list && e.version == c.status_version)
            .ok_or_else(|| reject("credential status reference outside verifier policy"))?;
        let snapshot = policy
            .snapshots
            .iter()
            .find(|s| s.status_list == c.status_list && s.version == c.status_version)
            .ok_or_else(|| reject("authoritative snapshot missing"))?;
        if snapshot.bit(c.status_index) {
            return Err(reject("credential revoked or status index unavailable"));
        }
        let list = sig::status_list_id_to_field(&c.status_list);
        let reference = sig::status_ref_digest(&list, c.status_index, c.status_version);
        let message =
            sig::commitment_message_with_status(&c.graph.commitment, &c.graph.salt, &reference);
        if !sig::verify(&c.issuer, &message, &c.signature) {
            return Err(reject("credential issuer signature invalid"));
        }
        let w = sig::in_circuit_witness(&c.issuer, &message, &c.signature)
            .ok_or_else(|| reject("issuer witness unavailable"))?;
        signatures.push([w.r_x, w.r_y, w.s, w.e, w.e_k].map(|f| field_to_hex(&f)));
        salts.push(field_to_hex(&c.graph.salt));
        status_lists.push(field_to_hex(&list));
        status_versions.push(c.status_version);
        status_indices.push(field_to_hex(&Fr::from(c.status_index)));
        status_roots.push(field_to_hex(
            &statement.entries[policy_index].status_list_root,
        ));
        status_siblings.push(
            merkle_witness(snapshot, statement.status_depth, c.status_index)
                .ok_or_else(|| reject("status witness unavailable"))?
                .siblings
                .iter()
                .map(field_to_hex)
                .collect::<Vec<_>>(),
        );
        policy_indices.push(field_to_hex(&Fr::from(policy_index as u64)));
        policy_siblings.push(
            accepted_set_witness(&statement.entries, POLICY_DEPTH, policy_index as u64)
                .ok_or_else(|| reject("policy witness unavailable"))?
                .iter()
                .map(field_to_hex)
                .collect::<Vec<_>>(),
        );
    }
    let mut selected_graphs = [[0usize; P]; R];
    let mut selected_leaves = [[0usize; P]; R];
    let mut values = vec![vec![zero.clone(); V]; R];
    let mut selected_types = [[[0u32; 3]; P]; R];
    let mut selected_hashes = vec![vec![vec![zero; 3]; P]; R];
    let mut filter_values = [[0u64; F]; R];
    let mut memberships = BTreeSet::new();
    for (r, row) in plan.rows.iter().enumerate() {
        let mut bindings = BTreeMap::new();
        for (p, witness) in row.witnesses.iter().enumerate() {
            let g = used
                .iter()
                .position(|&i| i == witness.credential)
                .ok_or_else(|| reject("unselected credential reference"))?;
            selected_graphs[r][p] = g;
            selected_leaves[r][p] = witness.leaf;
            memberships.insert((witness.credential, witness.leaf));
            let triple = &credentials[witness.credential].graph.canonical.triples[witness.leaf];
            for (s, term) in term_parts(triple).into_iter().enumerate() {
                let (ty, hs) = type_opening(&term)?;
                selected_types[r][p][s] = ty;
                selected_hashes[r][p][s] = field_to_hex(&hs);
                if let QuerySlot::Variable(v) = &statement.query.patterns[p][s] {
                    if let Some(old) = bindings.insert(v.clone(), term.clone()) {
                        if old != term {
                            return Err(reject("selected witnesses disagree on joined variable"));
                        }
                    }
                }
            }
        }
        for (v, name) in statement.vars.iter().enumerate() {
            values[r][v] = field_to_hex(&field(
                bindings
                    .get(name)
                    .ok_or_else(|| reject("selected variable unbound"))?,
            )?);
        }
        for (f, &index) in statement.hidden_filters.iter().enumerate() {
            let filter = &statement.query.filters[index];
            let term = &bindings[&filter.variable];
            let value = profile.value(term).ok_or_else(|| {
                reject(match profile {
                    NumericProfile::Unsigned => {
                        "private FILTER operand is not a canonical nonnegative integer"
                    }
                    NumericProfile::Signed => {
                        "private FILTER operand is not a canonical signed integer"
                    }
                })
            })?;
            if integer_capacity == PrivateIntegerCapacity::TwoDigits
                && value > SMALL_PRIVATE_INTEGER
            {
                return Err(reject("private FILTER exceeds bounded integer lane"));
            }
            filter_values[r][f] = value;
        }
    }
    fields.extend([
        ("counts", json!(counts)),
        ("salts", json!(salts)),
        ("enc", json!(enc)),
        ("status_lists", json!(status_lists)),
        ("status_versions", json!(status_versions)),
        ("status_indices", json!(status_indices)),
        ("status_roots", json!(status_roots)),
        ("status_siblings", json!(status_siblings)),
        ("policy_indices", json!(policy_indices)),
        ("policy_siblings", json!(policy_siblings)),
        ("signatures", json!(signatures)),
        ("selected_graphs", json!(selected_graphs)),
        ("selected_leaves", json!(selected_leaves)),
        ("values", json!(values)),
        ("selected_types", json!(selected_types)),
        ("selected_hashes", json!(selected_hashes)),
    ]);
    if !statement.hidden_filters.is_empty() {
        fields.push(("filter_values", json!(filter_values)));
    }
    let toml = fields
        .iter()
        .map(|(name, value)| format!("{name} = {}\n", toml_witness_value(value)))
        .collect();
    let work = ResultWork {
        selected_credentials,
        shared_memberships: memberships.len(),
        witness_uses: rows.len() * parsed.patterns.len(),
        public_predicates: rows.len() * (parsed.filters.len() - statement.hidden_filters.len()),
        private_predicates: rows.len() * statement.hidden_filters.len(),
        signature_checks: used.len(),
        optimization,
    };
    Ok(PreparedResult {
        presentation,
        package: statement.package,
        toml,
        public_inputs,
        work,
    })
}

// [GPT-6] TOML integer literals are signed i64; Noir also accepts decimal
// strings for integer inputs. This witness-only conversion preserves the public
// field encoding and admits the full u64 FILTER-bound range without truncation.
fn toml_witness_value(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(toml_witness_value).collect()),
        Value::Number(number) if number.as_u64().is_some_and(|n| n > i64::MAX as u64) => {
            Value::String(number.to_string())
        }
        other => other.clone(),
    }
}

fn pinned_toolchain() -> Result<(), ResultError> {
    for (tool, version) in [("nargo", "1.0.0-beta.21"), ("bb", "5.0.0-nightly.20260324")] {
        let out = Command::new(tool)
            .arg("--version")
            .output()
            .map_err(|e| reject(format!("{tool}: {e}")))?;
        if !out.status.success() || !String::from_utf8_lossy(&out.stdout).contains(version) {
            return Err(reject(format!("{tool} must use pinned version {version}")));
        }
    }
    Ok(())
}

impl PreparedResult {
    /// Returns prover-local work counts without any hidden term values.
    pub fn work(&self) -> &ResultWork {
        &self.work
    }

    /// Proves this statement with the pinned ZK backend.
    ///
    /// Use a nonempty ASCII filename label (alphanumerics, `_`, `-`, `.`). The input and witness
    /// files written by the existing driver contain credential secrets.
    ///
    /// # Errors
    /// Rejects mismatched toolchain versions, invalid tags, unsatisfiable
    /// witnesses, backend failures, and public-input serialization drift.
    pub fn prove(
        &self,
        prover: &CircuitProver,
        out_dir: &Path,
        tag: &str,
    ) -> Result<ResultPresentation, ResultError> {
        if tag.is_empty() || crate::driver::validate_witness_tag(tag).is_err() {
            return Err(reject("proof tag must be a nonempty ASCII filename label"));
        }
        pinned_toolchain()?;
        let artifact = prover.prove_private_package(self.package, &self.toml, out_dir, tag)?;
        if artifact.public_inputs != self.public_inputs {
            return Err(reject(
                "public-input serialization differs from pinned circuit ABI",
            ));
        }
        let mut presentation = self.presentation.clone();
        presentation.proof = artifact.proof;
        Ok(presentation)
    }
}

/// Returns the released mappings after all statement and cryptographic checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedResult {
    /// Exact relying-party query matched during verification.
    pub query: String,
    /// Verified released mappings, without private witnesses.
    pub rows: Vec<BTreeMap<String, Term>>,
}

/// Verifies a released result against the relying party's exact request and policy.
///
/// Re-parses the query, checks public predicates, reconstructs every public input
/// and derives the canonical member key locally. No bundled VK or public-input
/// bytes are trusted. Use a durable [`SeenNonces`] implementation in deployments.
/// A valid proof remains research-stage evidence, not external security assurance.
///
/// # Errors
/// Rejects any statement, policy, nonce, proof or pinned-toolchain mismatch.
pub fn verify_result(
    expected_query: &str,
    presentation: &ResultPresentation,
    policy: &ResultPolicy,
    nonce: &VerifierNonce,
    seen: &dyn SeenNonces,
    prover: &CircuitProver,
    work_dir: &Path,
) -> Result<VerifiedResult, ResultError> {
    if presentation.query != expected_query {
        return Err(reject("query differs from relying-party request"));
    }
    let statement = public_statement(presentation, policy, nonce)?;
    verify_statement(
        expected_query,
        &presentation.proof,
        statement,
        nonce,
        seen,
        prover,
        work_dir,
    )
}

fn verify_statement(
    expected_query: &str,
    proof: &[u8],
    statement: PublicStatement,
    nonce: &VerifierNonce,
    seen: &dyn SeenNonces,
    prover: &CircuitProver,
    work_dir: &Path,
) -> Result<VerifiedResult, ResultError> {
    if proof.is_empty() {
        return Err(reject("missing proof"));
    }
    pinned_toolchain()?;
    if !seen.record_fresh(nonce) {
        return Err(reject("challenge already consumed"));
    }
    let inputs = public_bytes(&statement.fields)?;
    let vk = prover.canonical_package_vk(statement.package, &work_dir.join("canonical"))?;
    if !prover.verify_with(proof, &inputs, &vk, &work_dir.join("verify"))? {
        return Err(reject("cryptographic proof rejected"));
    }
    Ok(VerifiedResult {
        query: expected_query.to_owned(),
        rows: statement.rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verifier::InMemorySeenNonces;
    use oxrdf::{Literal, NamedNode};
    use sparq_zk::commit::commit_triples;
    use sparq_zk::sig::SecretKey;

    const QUERY: &str = "SELECT DISTINCT ?name WHERE { ?person <urn:name> ?name . ?person <urn:age> ?age . ?person <urn:licensed> <urn:yes> . FILTER(?age >= 18) }";
    const PUBLIC_QUERY: &str =
        "SELECT DISTINCT ?age WHERE { ?person <urn:age> ?age . FILTER(?age >= 18) }";

    fn iri(s: &str) -> NamedNode {
        NamedNode::new(s).unwrap()
    }
    fn integer(n: u64) -> Term {
        Term::Literal(Literal::new_typed_literal(
            n.to_string(),
            iri("http://www.w3.org/2001/XMLSchema#integer"),
        ))
    }
    fn triple(s: &str, p: &str, o: Term) -> Triple {
        Triple::new(iri(s), iri(p), o)
    }
    fn credential(triples: Vec<Triple>, seed: u64, index: u64, list: &str) -> ResultCredential {
        let graph = commit_triples(&triples, Fr::from(seed + 100)).unwrap();
        let sk = SecretKey::from_seed(seed);
        let status = sig::status_ref_digest(&sig::status_list_id_to_field(list), index, 7);
        let message = sig::commitment_message_with_status(&graph.commitment, &graph.salt, &status);
        ResultCredential {
            graph,
            issuer: sk.public_key(),
            signature: sig::sign_deterministic(&sk, &message),
            status_list: list.into(),
            status_version: 7,
            status_index: index,
        }
    }
    fn fixture() -> (
        Vec<ResultCredential>,
        ResultPolicy,
        VerifierNonce,
        Vec<BTreeMap<String, Term>>,
    ) {
        let credentials = vec![
            credential(
                vec![
                    triple(
                        "urn:alice",
                        "urn:name",
                        Term::Literal(Literal::new_simple_literal("Alice")),
                    ),
                    triple("urn:alice", "urn:age", integer(42)),
                    triple(
                        "urn:bob",
                        "urn:name",
                        Term::Literal(Literal::new_simple_literal("Bob")),
                    ),
                    triple("urn:bob", "urn:age", integer(12)),
                ],
                1,
                3,
                "urn:status:people",
            ),
            credential(
                vec![
                    triple("urn:alice", "urn:licensed", Term::NamedNode(iri("urn:yes"))),
                    triple("urn:bob", "urn:licensed", Term::NamedNode(iri("urn:yes"))),
                ],
                2,
                9,
                "urn:status:licenses",
            ),
        ];
        let policy = ResultPolicy {
            trusted_issuers: credentials.iter().map(|c| c.issuer).collect(),
            snapshots: vec![
                StatusListSnapshot {
                    status_list: "urn:status:people".into(),
                    version: 7,
                    bits: vec![0; 128],
                },
                StatusListSnapshot {
                    status_list: "urn:status:licenses".into(),
                    version: 7,
                    bits: vec![0; 128],
                },
            ],
            min_version: 7,
            max_version: 7,
        };
        let nonce = VerifierNonce::from_field(Fr::from(424242u64));
        let rows = vec![BTreeMap::from([(
            "name".into(),
            Term::Literal(Literal::new_simple_literal("Alice")),
        )])];
        (credentials, policy, nonce, rows)
    }

    fn shared_alternative_fixture() -> (
        Vec<ResultCredential>,
        ResultPolicy,
        VerifierNonce,
        Vec<BTreeMap<String, Term>>,
    ) {
        let triples = vec![
            triple(
                "urn:alice",
                "urn:name",
                Term::Literal(Literal::new_simple_literal("Alice")),
            ),
            triple("urn:alice", "urn:age", integer(42)),
            triple("urn:alice", "urn:licensed", Term::NamedNode(iri("urn:yes"))),
        ];
        // Three early credentials force the baseline above K2. The final
        // credential supports the same fixed answer using one authentication.
        let mut credentials: Vec<_> = triples
            .iter()
            .enumerate()
            .map(|(i, t)| credential(vec![t.clone()], 1, i as u64, "urn:status:people"))
            .collect();
        credentials.push(credential(triples, 1, 3, "urn:status:people"));
        let (_, mut policy, nonce, rows) = fixture();
        policy.trusted_issuers = vec![credentials[0].issuer];
        (credentials, policy, nonce, rows)
    }

    #[test]
    fn supplied_credential_limit_precedes_authentication_and_query_parsing() {
        let template = credential(Vec::new(), 1, 0, "urn:status:people");
        let credentials: Vec<_> = (0..=crate::planner::MAX_DISCLOSURE_CREDENTIALS)
            .map(|_| ResultCredential {
                graph: template.graph.clone(),
                issuer: template.issuer,
                signature: template.signature,
                status_list: template.status_list.clone(),
                status_version: template.status_version,
                status_index: template.status_index,
            })
            .collect();
        let (_, policy, nonce, rows) = fixture();
        let err = prepare_result("invalid query", &credentials, &rows, &policy, &nonce)
            .err()
            .unwrap();
        assert!(err.to_string().contains("input credentials exceed"));
    }

    #[test]
    fn optimized_default_avoids_aggregate_capacity_failure_and_reports_budget_outcomes() {
        let (credentials, policy, nonce, rows) = shared_alternative_fixture();
        let baseline = prepare_result_with_options(
            QUERY,
            &credentials,
            &rows,
            &policy,
            &nonce,
            ResultOptions {
                witness_selection: WitnessSelection::FirstSuccess,
                ..ResultOptions::default()
            },
        );
        assert!(baseline
            .err()
            .unwrap()
            .to_string()
            .contains("selected credential capacity"));
        let optimized = prepare_result(QUERY, &credentials, &rows, &policy, &nonce).unwrap();
        assert_eq!(optimized.work.selected_credentials, 1);
        assert_eq!(optimized.work.signature_checks, 1);
        assert_eq!(
            optimized.work.optimization,
            Some(OptimizationCompletion::Optimal)
        );
        assert_eq!(optimized.package, "result_v1_k1_n16_p3_r4_f2");
        let exhausted = prepare_result_with_options(
            QUERY,
            &credentials,
            &rows,
            &policy,
            &nonce,
            ResultOptions {
                max_search_steps: 0,
                ..ResultOptions::default()
            },
        );
        assert!(matches!(exhausted, Err(ResultError::SearchExhausted)));

        // Reach a feasible incumbent but stop before completing the search.
        let parsed = DisclosureQuery::parse(QUERY).unwrap();
        let graphs: Vec<_> = credentials.iter().map(|c| c.graph.clone()).collect();
        let first_incumbent_budget = (1..500)
            .find(|&steps| {
                let report = optimize_disclosure_admitted(
                    &parsed,
                    &graphs,
                    &rows,
                    OptimizationLimits {
                        planner: PlannerLimits {
                            max_search_steps: steps,
                            ..PlannerLimits::default()
                        },
                        max_authentications: MAX_CREDENTIALS,
                        ..OptimizationLimits::default()
                    },
                    |_, _, _| true,
                )
                .unwrap();
                report.completion == OptimizationCompletion::BudgetExhausted
                    && report.plan.is_some()
            })
            .unwrap();
        let feasible = prepare_result_with_options(
            QUERY,
            &credentials,
            &rows,
            &policy,
            &nonce,
            ResultOptions {
                max_search_steps: first_incumbent_budget,
                ..ResultOptions::default()
            },
        )
        .unwrap();
        assert_eq!(
            feasible.work.optimization,
            Some(OptimizationCompletion::BudgetExhausted)
        );
        assert_eq!(feasible.presentation.rows.len(), rows.len());
        assert_eq!(feasible.work.witness_uses, 3);
    }

    #[test]
    #[ignore = "requires pinned nargo and bb; proves optimized witness selection reaches K1"]
    fn result_real_joint_optimization_reaches_one_credential_member() {
        pinned_toolchain().unwrap();
        let (credentials, policy, nonce, rows) = shared_alternative_fixture();
        let prepared = prepare_result(QUERY, &credentials, &rows, &policy, &nonce).unwrap();
        assert_eq!(prepared.work.signature_checks, 1);
        let driver = CircuitProver::from_crate_root();
        let dir =
            std::env::temp_dir().join(format!("sparq_result_optimized_{}", std::process::id()));
        let presentation = prepared.prove(&driver, &dir, "optimized").unwrap();
        let verified = verify_result(
            QUERY,
            &presentation,
            &policy,
            &nonce,
            &InMemorySeenNonces::default(),
            &driver,
            &dir,
        )
        .unwrap();
        assert_eq!(verified.rows, rows);
        let mut tampered = presentation.clone();
        tampered.rows[0].insert(
            "name".into(),
            disclosed(&Term::Literal(Literal::new_simple_literal("Mallory"))).unwrap(),
        );
        assert!(verify_result(
            QUERY,
            &tampered,
            &policy,
            &nonce,
            &InMemorySeenNonces::default(),
            &driver,
            &dir
        )
        .is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    fn prepared() -> PreparedResult {
        let (credentials, policy, nonce, rows) = fixture();
        prepare_result(QUERY, &credentials, &rows, &policy, &nonce).unwrap()
    }

    #[test]
    fn selected_success_survives_failing_scan_candidates_and_independent_status_slots() {
        let p = prepared();
        assert_eq!(p.work.selected_credentials, 2);
        assert_eq!(p.work.witness_uses, 3);
        assert_eq!(p.work.shared_memberships, 3);
        assert_eq!(p.work.private_predicates, 1);
        assert_eq!(p.package, "result_v1_k2_n16_p3_r4_f2");
        assert!(p.toml.contains("status_indices"));
    }

    #[test]
    fn public_predicate_uses_member_without_numeric_circuitry() {
        let (credentials, policy, nonce, _) = fixture();
        let rows = vec![BTreeMap::from([("age".into(), integer(42))])];
        let p = prepare_result(PUBLIC_QUERY, &credentials, &rows, &policy, &nonce).unwrap();
        assert_eq!(p.work.public_predicates, 1);
        assert_eq!(p.work.private_predicates, 0);
        assert_eq!(p.work.selected_credentials, 1);
        assert_eq!(p.work.signature_checks, 1);
        assert_eq!(p.package, "result_v1_k1_n16_p3_r4_f0");
        assert!(!p.toml.contains("filter_values"));
        assert_eq!(p.presentation.issuer_slots.len(), 1);
        let mut bad = p.presentation.clone();
        bad.rows[0].insert("age".into(), disclosed(&integer(12)).unwrap());
        assert!(public_statement(&bad, &policy, &nonce)
            .unwrap_err()
            .to_string()
            .contains("public FILTER is false"));
    }

    #[test]
    fn explicit_padding_retains_two_slot_policy_without_a_distinct_count_claim() {
        let (credentials, policy, nonce, _) = fixture();
        let rows = vec![BTreeMap::from([("age".into(), integer(42))])];
        let prepared = prepare_result_with_options(
            PUBLIC_QUERY,
            &credentials,
            &rows,
            &policy,
            &nonce,
            ResultOptions {
                credential_capacity: CredentialCapacity::HideInTwo,
                ..ResultOptions::default()
            },
        )
        .unwrap();
        assert_eq!(prepared.work.selected_credentials, 1);
        assert_eq!(prepared.work.signature_checks, 2);
        assert_eq!(prepared.presentation.issuer_slots.len(), 2);
        assert_eq!(
            prepared.presentation.issuer_slots[0],
            prepared.presentation.issuer_slots[1]
        );
        assert_eq!(prepared.package, "result_v1_k2_n16_p3_r4_f0");
    }

    #[test]
    fn public_manifest_contains_no_private_roots_salts_status_or_witness_attribution() {
        let (credentials, policy, nonce, rows) = fixture();
        let p = prepare_result(QUERY, &credentials, &rows, &policy, &nonce).unwrap();
        let serialized = serde_json::to_value(&p.presentation).unwrap();
        let object = serialized.as_object().unwrap();
        assert_eq!(
            object.keys().map(String::as_str).collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "version",
                "query",
                "rows",
                "challenge",
                "issuer_slots",
                "proof"
            ])
        );
        let bytes = String::from_utf8(serde_json::to_vec(&p.presentation).unwrap()).unwrap();
        for c in &credentials {
            assert!(!bytes.contains(&field_to_hex(&c.graph.commitment)));
            assert!(!bytes.contains(&field_to_hex(&c.graph.salt)));
            assert!(!bytes.contains(&c.status_list));
            assert!(!bytes.contains(&sig::signature_to_hex(&c.signature)));
        }
        let statement = public_statement(&p.presentation, &policy, &nonce).unwrap();
        let public = public_bytes(&statement.fields).unwrap();
        for c in &credentials {
            let root = field_to_be_bytes_32(&c.graph.commitment);
            assert!(!public.chunks_exact(32).any(|v| v == root));
        }
    }

    #[test]
    fn changed_reference_and_revoked_or_stale_policy_reject() {
        let (mut credentials, mut policy, nonce, rows) = fixture();
        credentials[0].status_index = 4;
        assert!(prepare_result(QUERY, &credentials, &rows, &policy, &nonce).is_err());
        credentials[0].status_index = 3;
        policy.snapshots[0].bits[0] |= 1 << 3;
        assert!(prepare_result(QUERY, &credentials, &rows, &policy, &nonce).is_err());
        policy.snapshots[0].bits[0] = 0;
        policy.min_version = 8;
        policy.max_version = 9;
        assert!(prepare_result(QUERY, &credentials, &rows, &policy, &nonce).is_err());
    }

    #[test]
    fn oversized_status_snapshots_reject_preparation_and_public_statement() {
        let (credentials, mut policy, nonce, rows) = fixture();
        let prepared = prepare_result(QUERY, &credentials, &rows, &policy, &nonce).unwrap();
        assert_eq!(policy.snapshots[0].bits.len(), 128);
        assert!(public_statement(&prepared.presentation, &policy, &nonce).is_ok());
        for suffix in [0, 0xFF] {
            policy.snapshots[0].bits = vec![0; (1 << 20) / 8 + 1];
            *policy.snapshots[0].bits.last_mut().unwrap() = suffix;
            assert!(policy.entries().is_err());
            assert!(prepare_result(QUERY, &credentials, &rows, &policy, &nonce).is_err());
            assert!(public_statement(&prepared.presentation, &policy, &nonce).is_err());
            policy.snapshots[0].bits = vec![0; 128];
        }
    }

    #[test]
    fn unknown_version_wrong_request_nonce_and_untrusted_issuer_reject() {
        let (credentials, mut policy, nonce, rows) = fixture();
        let p = prepare_result(QUERY, &credentials, &rows, &policy, &nonce).unwrap();
        let mut malformed = p.presentation.clone();
        malformed.version = 2;
        assert!(public_statement(&malformed, &policy, &nonce).is_err());
        let other = VerifierNonce::from_field(Fr::from(777u64));
        assert!(public_statement(&p.presentation, &policy, &other).is_err());
        policy.trusted_issuers.remove(0);
        assert!(public_statement(&p.presentation, &policy, &nonce).is_err());
        let error = verify_result(
            PUBLIC_QUERY,
            &p.presentation,
            &policy,
            &nonce,
            &InMemorySeenNonces::default(),
            &CircuitProver::from_crate_root(),
            Path::new("unused"),
        )
        .unwrap_err();
        assert!(error.to_string().contains("query differs"));
    }

    #[test]
    fn false_result_and_duplicate_distinct_rows_reject() {
        let (credentials, policy, nonce, mut rows) = fixture();
        rows[0].insert(
            "name".into(),
            Term::Literal(Literal::new_simple_literal("Bob")),
        );
        assert!(prepare_result(QUERY, &credentials, &rows, &policy, &nonce).is_err());
        rows[0].insert(
            "name".into(),
            Term::Literal(Literal::new_simple_literal("Alice")),
        );
        rows.push(rows[0].clone());
        assert!(prepare_result(QUERY, &credentials, &rows, &policy, &nonce).is_err());
    }

    #[test]
    fn ineligible_first_credentials_do_not_hide_later_eligible_witnesses() {
        let (mut credentials, mut policy, nonce, rows) = fixture();
        let triples = credentials[0].graph.canonical.triples.clone();
        let revoked = credential(triples.clone(), 1, 4, "urn:status:people");
        policy.snapshots[0].bits[0] |= 1 << 4;
        credentials.insert(0, revoked);
        assert!(prepare_result(QUERY, &credentials, &rows, &policy, &nonce).is_ok());
        credentials.swap(0, 1);
        assert!(prepare_result(QUERY, &credentials, &rows, &policy, &nonce).is_ok());
        let untrusted = credential(triples.clone(), 99, 3, "urn:status:people");
        credentials.insert(0, untrusted);
        assert!(prepare_result(QUERY, &credentials, &rows, &policy, &nonce).is_ok());
        let oversized = credential(
            (0..17)
                .map(|i| triple(&format!("urn:padding:{i}"), "urn:age", integer(42)))
                .chain(triples)
                .collect(),
            1,
            3,
            "urn:status:people",
        );
        credentials.insert(0, oversized);
        assert!(prepare_result(QUERY, &credentials, &rows, &policy, &nonce).is_ok());
    }

    #[test]
    fn unsupported_numeric_candidate_does_not_block_an_eligible_tuple_in_same_graph() {
        let (_, mut policy, nonce, _) = fixture();
        let c = credential(
            vec![
                triple(
                    "urn:alice",
                    "urn:age",
                    Term::Literal(Literal::new_typed_literal(
                        "0100",
                        iri("http://www.w3.org/2001/XMLSchema#integer"),
                    )),
                ),
                triple("urn:alice", "urn:age", integer(42)),
            ],
            1,
            3,
            "urn:status:people",
        );
        policy.trusted_issuers = vec![c.issuer];
        let q = "SELECT DISTINCT ?person WHERE { ?person <urn:age> ?age FILTER(?age >= 18) }";
        let rows = vec![BTreeMap::from([(
            "person".into(),
            Term::NamedNode(iri("urn:alice")),
        )])];
        let prepared = prepare_result(q, &[c], &rows, &policy, &nonce).unwrap();
        let line = prepared
            .toml
            .lines()
            .find(|l| l.starts_with("filter_values = "))
            .unwrap();
        assert!(line.contains("42"));
        assert!(!line.contains("100"));
    }

    #[test]
    fn canonical_large_sibling_remains_eligible_or_fails_its_actual_predicate() {
        // [GPT-6] Keep the valid large-sibling case distinct from lexical rejection.
        let (_, mut policy, nonce, _) = fixture();
        let c = credential(
            vec![
                triple("urn:alice", "urn:age", integer(100)),
                triple("urn:alice", "urn:age", integer(42)),
            ],
            1,
            3,
            "urn:status:people",
        );
        policy.trusted_issuers = vec![c.issuer];
        let rows = vec![BTreeMap::from([(
            "person".into(),
            Term::NamedNode(iri("urn:alice")),
        )])];
        for (predicate, value, capacity) in [
            ("?age >= 18", 100, PrivateIntegerCapacity::FullU64),
            ("?age < 100", 42, PrivateIntegerCapacity::TwoDigits),
        ] {
            let query = format!(
                "SELECT DISTINCT ?person WHERE {{ ?person <urn:age> ?age FILTER({predicate}) }}"
            );
            let p =
                prepare_result(&query, std::slice::from_ref(&c), &rows, &policy, &nonce).unwrap();
            assert_eq!(p.presentation.integer_capacity, capacity);
            assert!(p
                .toml
                .lines()
                .any(|line| line.starts_with("filter_values = ")
                    && line.contains(&value.to_string())));
        }
    }

    // [GPT-6] Goldens cover value, lexical, type and capacity boundaries independently.
    pub(super) fn numeric_fixture(
        term: Term,
        index: u64,
        bytes: usize,
    ) -> (
        ResultCredential,
        ResultPolicy,
        VerifierNonce,
        Vec<BTreeMap<String, Term>>,
    ) {
        let c = credential(
            vec![triple("urn:alice", "urn:age", term)],
            1,
            index,
            "urn:status:people",
        );
        let policy = ResultPolicy {
            trusted_issuers: vec![c.issuer],
            snapshots: vec![StatusListSnapshot {
                status_list: "urn:status:people".into(),
                version: 7,
                bits: vec![0; bytes],
            }],
            min_version: 7,
            max_version: 7,
        };
        (
            c,
            policy,
            VerifierNonce::from_field(Fr::from(111u64)),
            vec![BTreeMap::from([(
                "person".into(),
                Term::NamedNode(iri("urn:alice")),
            )])],
        )
    }

    #[test]
    fn full_u64_private_operands_keep_exact_witness_and_select_capacity() {
        for value in [
            0,
            9,
            10,
            99,
            100,
            i64::MAX as u64,
            1u64 << 63,
            10_000_000_000_000_000_000,
            u64::MAX,
        ] {
            let (c, policy, nonce, rows) = numeric_fixture(integer(value), 3, 128);
            let q = format!(
                "SELECT DISTINCT ?person WHERE {{ ?person <urn:age> ?age FILTER(?age = {value}) }}"
            );
            let p = prepare_result(&q, &[c], &rows, &policy, &nonce).unwrap();
            let expected = if value <= 99 {
                PrivateIntegerCapacity::TwoDigits
            } else {
                PrivateIntegerCapacity::FullU64
            };
            assert_eq!(p.presentation.integer_capacity, expected);
            let values: Value = serde_json::from_str(
                p.toml
                    .lines()
                    .find_map(|line| line.strip_prefix("filter_values = "))
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(values[0][0], toml_witness_value(&json!(value)));
            assert_eq!(
                public_statement(&p.presentation, &policy, &nonce)
                    .unwrap()
                    .package,
                p.package
            );
            if value > 99 {
                assert_eq!(p.package, "result_v2_k1_n16_p3_r4_f2_i64_d10");
                let mut downgrade = p.presentation.clone();
                downgrade.integer_capacity = PrivateIntegerCapacity::TwoDigits;
                assert!(public_statement(&downgrade, &policy, &nonce).is_err());
            }
        }
    }

    #[test]
    fn lexical_overflow_sign_and_type_substitution_reject() {
        for (lexical, datatype) in [
            ("18446744073709551616", "integer"),
            ("0100", "integer"),
            ("+100", "integer"),
            ("-1", "integer"),
            (" 100", "integer"),
            ("100", "string"),
            ("100", "decimal"),
            ("100", "unsignedLong"),
        ] {
            let term = Term::Literal(Literal::new_typed_literal(
                lexical,
                iri(&format!("http://www.w3.org/2001/XMLSchema#{datatype}")),
            ));
            let (c, policy, nonce, rows) = numeric_fixture(term, 3, 128);
            let q = "SELECT DISTINCT ?person WHERE { ?person <urn:age> ?age FILTER(?age >= 0) }";
            assert!(
                prepare_result(q, &[c], &rows, &policy, &nonce).is_err(),
                "{lexical} / {datatype}"
            );
        }
    }

    #[test]
    fn full_width_hiding_is_explicit_and_public_predicates_remain_circuit_free() {
        let (c, policy, nonce, rows) = numeric_fixture(integer(42), 3, 128);
        let q = "SELECT DISTINCT ?person WHERE { ?person <urn:age> ?age FILTER(?age >= 18) }";
        let options = ResultOptions {
            integer_capacity: IntegerCapacityPolicy::HideInU64,
            ..ResultOptions::default()
        };
        let p = prepare_result_with_options(q, &[c], &rows, &policy, &nonce, options).unwrap();
        assert_eq!(
            p.presentation.integer_capacity,
            PrivateIntegerCapacity::FullU64
        );
        let serialized = serde_json::to_value(&p.presentation).unwrap();
        assert_eq!(serialized["integer_capacity"], "full_u64");
        assert!(serialized.get("decimal_length").is_none());
        let (c, policy, nonce, _) = numeric_fixture(integer(u64::MAX), 3, 128);
        let rows = vec![BTreeMap::from([("age".into(), integer(u64::MAX))])];
        let public =
            prepare_result_with_options(PUBLIC_QUERY, &[c], &rows, &policy, &nonce, options)
                .unwrap();
        assert_eq!(public.package, "result_v1_k1_n16_p3_r4_f0");
        assert!(!public.toml.contains("filter_values"));
    }

    #[test]
    fn status_capacity_uses_complete_verifier_policy_and_preserves_padding() {
        let q = "SELECT DISTINCT ?person WHERE { ?person <urn:age> ?age FILTER(?age >= 18) }";
        for (bytes, depth) in [(128, 10), (129, 17), (16384, 17), (16385, 20), (131072, 20)] {
            let index = (bytes * 8 - 1) as u64;
            let (c, mut policy, nonce, rows) = numeric_fixture(integer(100), index, bytes);
            let p = prepare_result(q, &[c], &rows, &policy, &nonce).unwrap();
            assert_eq!(policy.status_depth().unwrap(), depth);
            let siblings: Value = serde_json::from_str(
                p.toml
                    .lines()
                    .find_map(|line| line.strip_prefix("status_siblings = "))
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(siblings[0].as_array().unwrap().len(), depth as usize);
            policy.snapshots[0].bits[bytes - 1] |= 128;
            assert_ne!(
                public_statement(&p.presentation, &policy, &nonce)
                    .unwrap()
                    .fields,
                public_statement(
                    &p.presentation,
                    &ResultPolicy {
                        snapshots: vec![StatusListSnapshot {
                            status_list: "urn:status:people".into(),
                            version: 7,
                            bits: vec![0; bytes]
                        }],
                        ..policy.clone()
                    },
                    &nonce
                )
                .unwrap()
                .fields
            );
        }
        let (c, mut policy, nonce, rows) = numeric_fixture(integer(42), 3, 128);
        policy.snapshots.push(StatusListSnapshot {
            status_list: "urn:status:unused".into(),
            version: 7,
            bits: vec![0; 16385],
        });
        let p = prepare_result(q, &[c], &rows, &policy, &nonce).unwrap();
        assert_eq!(p.package, "result_v2_k1_n16_p3_r4_f2_i8_d20");
        let (c, policy, nonce, rows) = numeric_fixture(integer(42), 129 * 8, 129);
        assert!(
            prepare_result(q, &[c], &rows, &policy, &nonce).is_err(),
            "padding is revoked"
        );
    }

    pub(super) fn alter_input(toml: &str, key: &str, edit: impl FnOnce(&mut Value)) -> String {
        let mut edit = Some(edit);
        toml.lines()
            .map(|line| {
                if let Some(value) = line.strip_prefix(&format!("{key} = ")) {
                    let mut value: Value = serde_json::from_str(value).unwrap();
                    edit.take().unwrap()(&mut value);
                    format!("{key} = {value}\n")
                } else {
                    format!("{line}\n")
                }
            })
            .collect()
    }

    /// Real adversarial witnesses exercise the compiled relation, not host prechecks.
    #[test]
    #[ignore = "requires pinned nargo; run explicitly in zk-toolchain lane"]
    fn result_relation_rejects_tampered_private_witnesses() {
        pinned_toolchain().unwrap();
        let p = prepared();
        let driver = CircuitProver::from_crate_root();
        driver.compile_package(p.package).unwrap();
        let mutations = [
            (
                "signature",
                alter_input(&p.toml, "signatures", |v| v[0][2] = json!("0x01")),
            ),
            (
                "status",
                alter_input(&p.toml, "status_indices", |v| v[0] = json!("0x05")),
            ),
            (
                "joined_value",
                alter_input(&p.toml, "values", |v| v[0][2] = json!("0x01")),
            ),
            (
                "hidden_integer",
                alter_input(&p.toml, "filter_values", |v| v[0][0] = json!(12)),
            ),
            (
                "blank_type",
                alter_input(&p.toml, "selected_types", |v| v[0][0][0] = json!(3)),
            ),
            (
                "padding_leaf",
                alter_input(&p.toml, "selected_leaves", |v| v[0][0] = json!(15)),
            ),
            (
                "issuer_identity",
                alter_input(&p.toml, "issuer_keys", |v| {
                    v[0][0] = json!("0x00");
                    v[0][1] = json!("0x01");
                }),
            ),
            (
                "status_policy",
                alter_input(&p.toml, "accepted_root", |v| *v = json!("0x01")),
            ),
        ];
        for (name, inputs) in mutations {
            let tag = format!("result_tamper_{name}");
            let result = driver.private_package_witness(p.package, &inputs, &tag);
            assert!(
                result.is_err(),
                "malicious witness {name} unexpectedly satisfied the circuit"
            );
        }
    }

    #[test]
    #[ignore = "requires pinned nargo and bb; validates every public capacity/member combination"]
    fn result_real_capacity_and_predicate_member_matrix() {
        pinned_toolchain().unwrap();
        let (credentials, policy, nonce, _) = fixture();
        let hidden_query =
            "SELECT DISTINCT ?person WHERE { ?person <urn:age> ?age FILTER(?age >= 18) }";
        let hidden_rows = vec![BTreeMap::from([(
            "person".into(),
            Term::NamedNode(iri("urn:alice")),
        )])];
        let public_rows = vec![BTreeMap::from([("age".into(), integer(42))])];
        let driver = CircuitProver::from_crate_root();
        let dir = std::env::temp_dir().join(format!("sparq_result_matrix_{}", std::process::id()));
        for (q, rows) in [(hidden_query, &hidden_rows), (PUBLIC_QUERY, &public_rows)] {
            for capacity in [CredentialCapacity::Smallest, CredentialCapacity::HideInTwo] {
                let prepared = prepare_result_with_options(
                    q,
                    &credentials,
                    rows,
                    &policy,
                    &nonce,
                    ResultOptions {
                        credential_capacity: capacity,
                        ..ResultOptions::default()
                    },
                )
                .unwrap();
                let tag = format!("matrix_{}", prepared.package);
                let p = prepared.prove(&driver, &dir.join(&tag), &tag).unwrap();
                let got = verify_result(
                    q,
                    &p,
                    &policy,
                    &nonce,
                    &InMemorySeenNonces::default(),
                    &driver,
                    &dir.join("verify"),
                )
                .unwrap();
                assert_eq!(&got.rows, rows);
                let mut wrong_bucket = p.clone();
                if wrong_bucket.issuer_slots.len() == 1 {
                    wrong_bucket
                        .issuer_slots
                        .push(wrong_bucket.issuer_slots[0].clone());
                } else {
                    wrong_bucket.issuer_slots.pop();
                }
                assert!(verify_result(
                    q,
                    &wrong_bucket,
                    &policy,
                    &nonce,
                    &InMemorySeenNonces::default(),
                    &driver,
                    &dir.join("wrong_bucket")
                )
                .is_err());
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn full_u64_filter_bounds_preserve_public_fields_at_toml_boundary() {
        for bound in [i64::MAX as u64, 1u64 << 63, u64::MAX] {
            let fields = [("filter_bounds", json!([bound, 0]))];
            let expected = [Fr::from(bound), Fr::from(0u64)]
                .iter()
                .flat_map(field_to_be_bytes_32)
                .collect::<Vec<_>>();
            assert_eq!(public_bytes(&fields).unwrap(), expected);
            let rendered = toml_witness_value(&fields[0].1);
            if bound <= i64::MAX as u64 {
                assert_eq!(rendered[0], json!(bound));
            } else {
                assert_eq!(rendered[0], json!(bound.to_string()));
            }
            assert_eq!(rendered[1], json!(0));
        }
    }

    #[test]
    #[ignore = "requires pinned nargo and bb; checks signed-TOML and full-u64 boundaries"]
    fn result_real_full_u64_filter_bounds() {
        pinned_toolchain().unwrap();
        let (credentials, policy, nonce, _) = fixture();
        let rows = vec![BTreeMap::from([(
            "person".into(),
            Term::NamedNode(iri("urn:alice")),
        )])];
        let driver = CircuitProver::from_crate_root();
        let dir = std::env::temp_dir().join(format!("sparq_result_u64_{}", std::process::id()));
        for bound in [i64::MAX as u64, 1u64 << 63, u64::MAX] {
            let query = format!(
                "SELECT DISTINCT ?person WHERE {{ ?person <urn:age> ?age FILTER(?age <= {bound}) }}"
            );
            let prepared = prepare_result(&query, &credentials, &rows, &policy, &nonce).unwrap();
            // Actual Nargo parsing and relation execution at each boundary.
            let witness = driver
                .private_package_witness(prepared.package, &prepared.toml, "u64_bound")
                .unwrap();
            assert!(witness.path.exists());
            if bound == u64::MAX {
                let proof = prepared.prove(&driver, &dir, "u64_max").unwrap();
                let verified = verify_result(
                    &query,
                    &proof,
                    &policy,
                    &nonce,
                    &InMemorySeenNonces::default(),
                    &driver,
                    &dir,
                )
                .unwrap();
                assert_eq!(verified.rows, rows);
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Every expanded wrapper executes; representative full proofs cover each new status depth.
    #[test]
    #[ignore = "requires pinned nargo and bb; full-u64/status-capacity matrix and adversarial proofs"]
    fn result_real_expanded_capacity_matrix() {
        pinned_toolchain().unwrap();
        let driver = CircuitProver::from_crate_root();
        let dir =
            std::env::temp_dir().join(format!("sparq_result_expanded_{}", std::process::id()));
        let hidden_query =
            "SELECT DISTINCT ?person WHERE { ?person <urn:age> ?age FILTER(?age >= 0) }";
        let mut exercised = BTreeSet::new();
        for depth in [10, 17, 20] {
            for capacity in [CredentialCapacity::Smallest, CredentialCapacity::HideInTwo] {
                for lane in [0, 8, 64] {
                    if depth == 10 && lane != 64 {
                        continue;
                    }
                    let value = if lane == 8 { 42 } else { u64::MAX };
                    let bytes = (1usize << depth) / 8;
                    let (c, policy, nonce, hidden_rows) =
                        numeric_fixture(integer(value), (bytes * 8 - 1) as u64, bytes);
                    let public_rows = vec![BTreeMap::from([("age".into(), integer(value))])];
                    let (query, rows) = if lane == 0 {
                        (PUBLIC_QUERY, &public_rows)
                    } else {
                        (hidden_query, &hidden_rows)
                    };
                    let prepared = prepare_result_with_options(
                        query,
                        &[c],
                        rows,
                        &policy,
                        &nonce,
                        ResultOptions {
                            credential_capacity: capacity,
                            ..ResultOptions::default()
                        },
                    )
                    .unwrap();
                    assert_eq!(prepared.presentation.version, 2);
                    exercised.insert(prepared.package);
                    let witness = driver
                        .private_package_witness(
                            prepared.package,
                            &prepared.toml,
                            "expanded_matrix",
                        )
                        .unwrap();
                    assert!(witness.path.exists());
                    drop(witness);
                    let prove_this = lane == 64
                        && ((depth == 10 && capacity == CredentialCapacity::Smallest)
                            || (depth != 10 && capacity == CredentialCapacity::HideInTwo));
                    if !prove_this {
                        continue;
                    }
                    for (name, inputs) in [
                        (
                            "integer_detach",
                            alter_input(&prepared.toml, "filter_values", |v| {
                                v[0][0] = json!((u64::MAX - 1).to_string())
                            }),
                        ),
                        (
                            "integer_overflow",
                            alter_input(&prepared.toml, "filter_values", |v| {
                                v[0][0] = json!("18446744073709551616")
                            }),
                        ),
                        (
                            "status_path",
                            alter_input(&prepared.toml, "status_siblings", |v| {
                                v[0][0] = json!("0x01")
                            }),
                        ),
                        (
                            "status_index",
                            alter_input(&prepared.toml, "status_indices", |v| v[0] = json!("0x00")),
                        ),
                    ] {
                        assert!(
                            driver
                                .private_package_witness(prepared.package, &inputs, name)
                                .is_err(),
                            "{name}"
                        );
                    }
                    let proof = prepared
                        .prove(&driver, &dir.join(format!("d{depth}")), "expanded_proof")
                        .unwrap();
                    let checked = verify_result(
                        query,
                        &proof,
                        &policy,
                        &nonce,
                        &InMemorySeenNonces::default(),
                        &driver,
                        &dir,
                    )
                    .unwrap();
                    assert_eq!(&checked.rows, rows);
                    let mut wrong_capacity = proof.clone();
                    wrong_capacity.integer_capacity = PrivateIntegerCapacity::TwoDigits;
                    assert!(verify_result(
                        query,
                        &wrong_capacity,
                        &policy,
                        &nonce,
                        &InMemorySeenNonces::default(),
                        &driver,
                        &dir
                    )
                    .is_err());
                    let mut revoked = policy.clone();
                    *revoked.snapshots[0].bits.last_mut().unwrap() |= 128;
                    assert!(verify_result(
                        query,
                        &proof,
                        &revoked,
                        &nonce,
                        &InMemorySeenNonces::default(),
                        &driver,
                        &dir
                    )
                    .is_err());
                }
            }
        }
        assert_eq!(exercised.len(), 14);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[ignore = "requires pinned nargo; private full-u64 boundary execution independent of host admission"]
    fn result_relation_executes_full_width_private_boundaries() {
        pinned_toolchain().unwrap();
        let driver = CircuitProver::from_crate_root();
        let query = "SELECT DISTINCT ?person WHERE { ?person <urn:age> ?age FILTER(?age >= 0) }";
        for value in [0, 99, 100, 1u64 << 63, 10_000_000_000_000_000_000, u64::MAX] {
            let (c, policy, nonce, rows) = numeric_fixture(integer(value), 3, 128);
            let p = prepare_result_with_options(
                query,
                &[c],
                &rows,
                &policy,
                &nonce,
                ResultOptions {
                    integer_capacity: IntegerCapacityPolicy::HideInU64,
                    ..ResultOptions::default()
                },
            )
            .unwrap();
            assert!(driver
                .private_package_witness(p.package, &p.toml, "private_u64_boundary")
                .is_ok());
        }
    }

    #[test]
    #[ignore = "requires pinned nargo; expanded tiny-profile witness narrowing must not wrap"]
    fn result_relation_tiny_capacity_rejects_wrapping_private_values() {
        pinned_toolchain().unwrap();
        let driver = CircuitProver::from_crate_root();
        let query = "SELECT DISTINCT ?person WHERE { ?person <urn:age> ?age FILTER(?age >= 0) }";
        for depth in [17, 20] {
            for capacity in [CredentialCapacity::Smallest, CredentialCapacity::HideInTwo] {
                let (c, policy, nonce, rows) = numeric_fixture(integer(42), 3, (1 << depth) / 8);
                let p = prepare_result_with_options(
                    query,
                    &[c],
                    &rows,
                    &policy,
                    &nonce,
                    ResultOptions {
                        credential_capacity: capacity,
                        ..ResultOptions::default()
                    },
                )
                .unwrap();
                assert_eq!(
                    p.presentation.integer_capacity,
                    PrivateIntegerCapacity::TwoDigits
                );
                assert!(driver
                    .private_package_witness(p.package, &p.toml, "tiny_positive")
                    .is_ok());
                // 298 truncated to u8 is 42, so deleting the pre-cast guard would accept
                // this malicious private value while the signed lexical token stays 42.
                let malicious = alter_input(&p.toml, "filter_values", |v| v[0][0] = json!(298));
                assert!(driver
                    .private_package_witness(p.package, &malicious, "tiny_wrap")
                    .is_err());
            }
        }
    }

    #[test]
    #[ignore = "requires pinned nargo and bb; preserves actual baseline v1 verification keys"]
    fn result_legacy_v1_keys_match_pinned_foundation_keys() {
        // [GPT-6] Expected bytes were built from the independent foundation713 archive.
        pinned_toolchain().unwrap();
        let expected: Value = serde_json::from_str(include_str!(
            "../../../bench/zk-compose/result_v1_compatibility.json"
        ))
        .unwrap();
        let driver = CircuitProver::from_crate_root();
        let dir = std::env::temp_dir().join(format!("sparq_v1_key_compat_{}", std::process::id()));
        for (package, evidence) in expected["members"].as_object().unwrap() {
            let key = driver.canonical_package_vk(package, &dir).unwrap();
            let actual: String = key.iter().map(|byte| format!("{byte:02x}")).collect();
            assert!(
                actual == evidence["base"]["verification_key_hex"].as_str().unwrap(),
                "{package}: canonical verification key differs from the pinned baseline"
            );
        }
        assert_eq!(expected["members"].as_object().unwrap().len(), 4);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[ignore = "requires pinned nargo and bb; genuine expanded tiny and predicate-free proofs"]
    fn result_real_proofs_cover_expanded_tiny_and_predicate_free_public_abis() {
        // [GPT-6] These are genuine proofs, separately counted from wrapper executions.
        pinned_toolchain().unwrap();
        let driver = CircuitProver::from_crate_root();
        let dir =
            std::env::temp_dir().join(format!("sparq_result_small_v2_{}", std::process::id()));
        for (depth, capacity, query, expected_package) in [
            (
                17,
                CredentialCapacity::Smallest,
                "SELECT DISTINCT ?person WHERE { ?person <urn:age> ?age FILTER(?age >= 18) }",
                "result_v2_k1_n16_p3_r4_f2_i8_d17",
            ),
            (
                20,
                CredentialCapacity::HideInTwo,
                "SELECT DISTINCT ?person WHERE { ?person <urn:age> ?age }",
                "result_v2_k2_n16_p3_r4_f0_i0_d20",
            ),
        ] {
            let (c, policy, nonce, rows) = numeric_fixture(integer(42), 3, (1 << depth) / 8);
            let p = prepare_result_with_options(
                query,
                &[c],
                &rows,
                &policy,
                &nonce,
                ResultOptions {
                    credential_capacity: capacity,
                    ..ResultOptions::default()
                },
            )
            .unwrap();
            assert_eq!(p.package, expected_package);
            assert_eq!(p.presentation.version, 2);
            let proof = p.prove(&driver, &dir, "small_v2_public_abi").unwrap();
            let checked = verify_result(
                query,
                &proof,
                &policy,
                &nonce,
                &InMemorySeenNonces::default(),
                &driver,
                &dir,
            )
            .unwrap();
            assert_eq!(&checked.rows, &rows);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Full proofs anchor ABI reconstruction, proof mode, independent policy and request binding.
    #[test]
    #[ignore = "requires pinned nargo and bb; run explicitly in zk-toolchain lane"]
    fn result_real_proof_and_adversarial_verifier() {
        pinned_toolchain().unwrap();
        let (credentials, policy, nonce, rows) = fixture();
        let prepared = prepare_result(QUERY, &credentials, &rows, &policy, &nonce).unwrap();
        let dir = std::env::temp_dir().join(format!("sparq_result_proof_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let driver = CircuitProver::from_crate_root();
        let p = prepared
            .prove(
                &driver,
                &dir.join("private_proof"),
                "--result_roundtrip-v1.2",
            )
            .unwrap();
        let seen = InMemorySeenNonces::default();
        let verified = verify_result(
            QUERY,
            &p,
            &policy,
            &nonce,
            &seen,
            &driver,
            &dir.join("positive"),
        )
        .unwrap();
        assert_eq!(verified.rows, rows);
        assert!(verify_result(
            QUERY,
            &p,
            &policy,
            &nonce,
            &seen,
            &driver,
            &dir.join("replay")
        )
        .is_err());

        let mut changed = p.clone();
        changed.rows[0].insert(
            "name".into(),
            DisclosedTerm::Iri {
                value: "urn:mallory".into(),
            },
        );
        assert!(verify_result(
            QUERY,
            &changed,
            &policy,
            &nonce,
            &InMemorySeenNonces::default(),
            &driver,
            &dir.join("changed_result")
        )
        .is_err());
        let shared = dir.join("shared_verification_root");
        std::thread::scope(|scope| {
            let valid = scope.spawn(|| {
                verify_result(
                    QUERY,
                    &p,
                    &policy,
                    &nonce,
                    &InMemorySeenNonces::default(),
                    &driver,
                    &shared,
                )
            });
            let invalid = scope.spawn(|| {
                verify_result(
                    QUERY,
                    &changed,
                    &policy,
                    &nonce,
                    &InMemorySeenNonces::default(),
                    &driver,
                    &shared,
                )
            });
            assert_eq!(valid.join().unwrap().unwrap().rows, rows);
            assert!(invalid.join().unwrap().is_err());
        });
        let mut changed_policy = policy.clone();
        changed_policy.snapshots[0].bits[0] |= 1 << 3;
        assert!(verify_result(
            QUERY,
            &p,
            &changed_policy,
            &nonce,
            &InMemorySeenNonces::default(),
            &driver,
            &dir.join("revoked")
        )
        .is_err());
        let mut broken = p.clone();
        broken.proof[100] ^= 1;
        assert!(verify_result(
            QUERY,
            &broken,
            &policy,
            &nonce,
            &InMemorySeenNonces::default(),
            &driver,
            &dir.join("broken")
        )
        .is_err());

        // A second VALID proof answers another query under the same nonce. The
        // verifier must reject the entire substituted statement at the API boundary.
        let public_rows = vec![BTreeMap::from([("age".into(), integer(42))])];
        let public_prepared =
            prepare_result(PUBLIC_QUERY, &credentials, &public_rows, &policy, &nonce).unwrap();
        let public_proof = public_prepared
            .prove(
                &driver,
                &dir.join("public_proof"),
                "result_public_roundtrip",
            )
            .unwrap();
        verify_result(
            PUBLIC_QUERY,
            &public_proof,
            &policy,
            &nonce,
            &InMemorySeenNonces::default(),
            &driver,
            &dir.join("public_positive"),
        )
        .unwrap();
        assert!(verify_result(
            QUERY,
            &public_proof,
            &policy,
            &nonce,
            &InMemorySeenNonces::default(),
            &driver,
            &dir.join("substitution")
        )
        .is_err());
        // Changing both the envelope and verifier nonce passes host equality;
        // the old cryptographic proof must still reject the different challenge.
        let fresh_nonce = VerifierNonce::from_field(Fr::from(999_999u64));
        let mut rechallenged = p.clone();
        rechallenged.challenge = fresh_nonce.as_field_hex();
        assert!(verify_result(
            QUERY,
            &rechallenged,
            &policy,
            &fresh_nonce,
            &InMemorySeenNonces::default(),
            &driver,
            &dir.join("challenge_binding")
        )
        .is_err());

        // VKs can be shared between backend modes. Test an actual non-ZK proof,
        // rather than incorrectly assuming the VK itself records masking mode.
        let witness = driver
            .private_package_witness(prepared.package, &prepared.toml, "result_no_zk_probe")
            .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&witness.path)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            assert_eq!(
                std::fs::metadata(witness.path.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        let private_path = witness.path.clone();
        let out = Command::new("bb")
            .args(["prove", "-b"])
            .arg(driver.compile_package(prepared.package).unwrap())
            .arg("-w")
            .arg(&witness.path)
            .arg("-o")
            .arg(dir.join("no_zk"))
            .args(["--write_vk", "-t", "noir-recursive-no-zk"])
            .output()
            .unwrap();
        drop(witness);
        assert!(!private_path.exists());
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let mut unmasked = p.clone();
        unmasked.proof = std::fs::read(dir.join("no_zk/proof")).unwrap();
        assert!(verify_result(
            QUERY,
            &unmasked,
            &policy,
            &nonce,
            &InMemorySeenNonces::default(),
            &driver,
            &dir.join("mode_mismatch")
        )
        .is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
