// [OPUS-5.5] Host-only engine-replay bridge; experimental and not externally audited.
//! Prepares retained engine-replay originals as exact V3 witnesses.
//!
//! `sparq-bench fuzz-replay` retains each native matrix cell as unchanged
//! `query.rq` and `data.ttl` bytes plus a `record.json` observation. [`prepare`]
//! checks those bytes against a verifier-owned [`VerifierManifest`], converts the
//! original Turtle into exact V3 N-Quads with an explicit named-graph catalog, and
//! applies the existing V3 commitment rule. [`Prepared::evaluate_against`] then
//! applies the existing V3 admission and evaluation rules natively and compares
//! the result with an independently supplied [`ExpectedManifest`].
//!
//! The record is read only for its schema, cell identity, format, catalog and its
//! copies of the original query and dataset. `observed_ntriples`, native and
//! reference result rows, and any request-like or expectation-like record fields
//! never become an input. The verifier's request, nonce and agreed commitment are
//! used unchanged; a mismatch fails instead of being recomputed from holder data.
//!
//! Preparation is not proof. The companion host crate proves and verifies the
//! returned witness. Holder-declared authority authenticates neither the dataset
//! nor wallet completeness, and this module authenticates no source credential.
//! Only explicitly synthetic inputs with a published fixed salt are admitted.
//!
//! Conversion and original-file digest checks are host harness checks; the guest
//! does not execute them. A receipt covers V3 evaluation of the converted
//! N-Quads only. Only a verifier-agreed anchor that the verifier independently
//! prepared from the originals binds those converted inputs to them. A
//! holder-declared proof alone authenticates neither the original Turtle digests
//! nor the correctness of the conversion.
//!
//! # Conversion
//!
//! [`convert_turtle`] parses with the already-resolved `oxttl` parser and no
//! external base. IRIs, datatype IRIs and literal lexical forms are written
//! unchanged, so Turtle `01` stays `"01"^^xsd:integer`; language tags follow the
//! parser's own handling. Prefixed names and a document-declared `@base` are
//! expanded. Blank nodes are relabeled `_:b0`, `_:b1`, ... in first-occurrence
//! order, which keeps graph structure while making anonymous parser labels
//! deterministic. Statement order and duplicate statements are retained. Turtle
//! carries only the default graph; the record's catalog names empty named graphs,
//! whose remaining rules (IRI syntax, uniqueness, capacity) V3 checks itself.
//!
//! # Failure dispositions
//!
//! [`ReplayError::disposition`] classifies V3 diagnostics only through exact
//! lookups in the reviewed `bench/zk-bindings/rejections.json` registry and the
//! actual typed engine causes. An unregistered diagnostic is
//! [`Disposition::Unexpected`], never an exclusion.

use crate::{
    BudgetExceeded, DatasetAuthority, EvaluationCapacity, EvaluationError, Rejected, RowOrder, v3,
};
use oxrdf::{BlankNode, NamedOrBlankNode, Term, Triple};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Schema of the retained native record this module reads.
pub const RECORD_SCHEMA: &str = "sparq.engine-seed-replay.v1";
/// Exact protocol version of [`VerifierManifest`].
pub const VERIFIER_SCHEMA: &str = "sparq.engine-replay-proof.verifier.v1";
/// Exact protocol version of [`ExpectedManifest`].
pub const EXPECTED_SCHEMA: &str = "sparq.engine-replay-proof.expected.v1";
/// Exact protocol version of [`HolderManifest`].
pub const HOLDER_SCHEMA: &str = "sparq.engine-replay-proof.holder.v1";

/// Base used only to classify a failed parse; its triples are discarded.
///
/// The reserved `.invalid` domain cannot name real data. A document that fails
/// without a base but parses with this one is reported as
/// [`SourceError::RelativeIriWithoutBase`]; no witness is ever built from it.
const CLASSIFICATION_BASE: &str = "http://sparq.invalid/engine-replay/unknown-base/";

/// Bound on blank-node bijection work when matching an independent expectation.
///
/// Counts candidate bijections multiplied by compared cells. It is an adapter
/// bound, not a V3 capacity; exceeding it reports [`ReplayError::OracleCapacity`]
/// and never agreement. Raising it only permits slower comparisons.
const MAX_BIJECTION_STEPS: u64 = 1 << 24;

/// Review bound on oracle provenance text, so manifests stay inspectable.
const MAX_PROVENANCE_BYTES: usize = 1024;

/// Bound on cell identifier length, matching the native controller's safe IDs.
const MAX_IDENTIFIER_BYTES: usize = 64;

/// Reviewed static diagnostics and typed causes shared with the binding harness.
const REJECTION_REGISTRY: &str = include_str!("../../../../bench/zk-bindings/rejections.json");

/// Exact bytes of one retained replay cell; never rendered by `Debug`.
#[derive(Clone, Copy)]
pub struct Originals<'a> {
    /// The retained `record.json` bytes.
    pub record: &'a [u8],
    /// The retained `query.rq` bytes.
    pub query: &'a [u8],
    /// The retained `data.ttl` bytes.
    pub data: &'a [u8],
}

impl fmt::Debug for Originals<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Originals([REDACTED])")
    }
}

/// Identity of one native matrix cell; the profile is declared, not recorded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cell {
    /// Compiled feature profile named by the native matrix.
    pub profile: String,
    /// Original generator category.
    pub category: String,
    /// Original generator seed.
    pub seed: u64,
    /// Native storage variant that produced the retained record.
    pub storage: String,
}

/// Lowercase hexadecimal SHA-256 digests of the exact retained files.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OriginalDigests {
    /// Digest of `record.json`.
    pub record_sha256: String,
    /// Digest of `query.rq`.
    pub query_sha256: String,
    /// Digest of `data.ttl`.
    pub data_sha256: String,
}

/// Verifier-owned V3 request and the original cell it expects.
///
/// Obtain this independently of the holder. [`prepare`] never rewrites the
/// request, including its authority, commitment, policy or nonce.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifierManifest {
    /// Must equal [`VERIFIER_SCHEMA`].
    pub schema: String,
    /// The retained native cell being replayed.
    pub cell: Cell,
    /// Expected digests of the exact retained files.
    pub originals: OriginalDigests,
    /// The complete V3 request, whose query must equal `query.rq` bytes.
    pub request: v3::Request,
}

/// Declared derivation of an independent expected result.
///
/// There is deliberately no variant for the evaluator under test or for an
/// unreviewed native or reference replay observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OracleKind {
    /// Derived by review from the original inputs and published SPARQL semantics.
    ReviewedDerivation,
    /// A reviewed result from a separately implemented system, such as a
    /// reviewed conversion of the retained reference-store observation.
    ReviewedIndependentImplementation,
}

/// Provenance of an expected result; descriptive, not authenticated.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Oracle {
    /// How the expectation was derived.
    pub kind: OracleKind,
    /// What the expectation was derived from.
    pub source: String,
    /// Who or what reviewed the derivation.
    pub reviewer: String,
}

/// Independent expected V3 result bound to the original input digests.
///
/// The expectation depends only on the query and dataset, so storage and profile
/// copies of the same originals share it. SELECT tables are compared with one
/// global blank-node bijection and bag multiplicity; ASK values and RDFC-1.0
/// canonical graph N-Triples compare exactly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedManifest {
    /// Must equal [`EXPECTED_SCHEMA`].
    pub schema: String,
    /// Digest of the `query.rq` bytes this expectation was derived for.
    pub query_sha256: String,
    /// Digest of the `data.ttl` bytes this expectation was derived for.
    pub data_sha256: String,
    /// Declared derivation; never a normative-golden label.
    pub oracle: Oracle,
    /// The expected V3 result.
    pub result: v3::CanonicalResult,
}

impl ExpectedManifest {
    /// Compares an actual V3 result with this independent expectation.
    ///
    /// SELECT variables, order kind and row count must match exactly. Blank-node
    /// cells use one bijection across the whole table; bag rows compare as
    /// multisets and sequences compare in order. Other forms compare exactly.
    ///
    /// # Errors
    /// Returns [`ReplayError::OracleCapacity`] when bijection search exceeds its
    /// bound; that outcome is never agreement.
    pub fn matches(&self, actual: &v3::CanonicalResult) -> Result<bool, ReplayError> {
        same_result(&self.result, actual)
    }

    fn validate(&self) -> Result<(), ReplayError> {
        if self.schema != EXPECTED_SCHEMA {
            return Err(ReplayError::Manifest(ManifestError::ExpectedSchema));
        }
        if !valid_digest(&self.query_sha256) || !valid_digest(&self.data_sha256) {
            return Err(ReplayError::Manifest(ManifestError::DigestFormat));
        }
        if !reviewable(&self.oracle.source) || !reviewable(&self.oracle.reviewer) {
            return Err(ReplayError::Manifest(ManifestError::OracleProvenance));
        }
        Ok(())
    }
}

fn reviewable(text: &str) -> bool {
    !text.trim().is_empty() && text.len() <= MAX_PROVENANCE_BYTES
}

/// Holder-side blinding for the prepared dataset commitment.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HolderSalt {
    /// A published fixed salt, admitted only for explicitly synthetic inputs.
    ///
    /// It is reproducible by design and is not production blinding.
    SyntheticFixed([u8; 32]),
}

impl fmt::Debug for HolderSalt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SyntheticFixed([REDACTED])")
    }
}

/// Holder-owned preparation settings, kept separate from verifier expectations.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HolderManifest {
    /// Must equal [`HOLDER_SCHEMA`].
    pub schema: String,
    /// Explicit declaration that the retained inputs are synthetic test data.
    pub synthetic_inputs: bool,
    /// Blinding source for the dataset commitment.
    pub salt: HolderSalt,
}

impl HolderManifest {
    fn validated_salt(&self) -> Result<[u8; 32], ReplayError> {
        if self.schema != HOLDER_SCHEMA {
            return Err(ReplayError::Manifest(ManifestError::HolderSchema));
        }
        let HolderSalt::SyntheticFixed(salt) = &self.salt;
        if !self.synthetic_inputs {
            return Err(ReplayError::Manifest(
                ManifestError::FixedSaltWithoutSyntheticInputs,
            ));
        }
        if *salt == [0; 32] {
            return Err(ReplayError::Manifest(ManifestError::ZeroSalt));
        }
        Ok(*salt)
    }
}

/// Exact V3 source converted from one original Turtle document.
#[derive(Clone)]
pub struct ConvertedSource {
    nquads: String,
    named_graphs: Vec<String>,
    statements: u32,
    blank_nodes: u32,
}

impl fmt::Debug for ConvertedSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ConvertedSource([REDACTED])")
    }
}

impl ConvertedSource {
    /// Exact N-Quads bytes, one default-graph statement per line.
    pub fn nquads(&self) -> &str {
        &self.nquads
    }

    /// Named-graph catalog carried unchanged from the retained record.
    pub fn named_graphs(&self) -> &[String] {
        &self.named_graphs
    }

    /// Source statements, counted before any RDF graph deduplication.
    pub fn statements(&self) -> u32 {
        self.statements
    }

    /// Distinct blank nodes after deterministic relabeling.
    pub fn blank_nodes(&self) -> u32 {
        self.blank_nodes
    }

    /// Builds the V3 private dataset with the holder's blinding salt.
    pub fn into_dataset(self, salt: [u8; 32]) -> v3::PrivateDataset {
        v3::PrivateDataset {
            nquads: self.nquads,
            named_graphs: self.named_graphs,
            salt,
        }
    }
}

/// Original Turtle conversion failure, before any V3 rule runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceError {
    /// Turtle syntax or term validation failed.
    Syntax,
    /// The document parses only with a base IRI that it does not declare.
    RelativeIriWithoutBase,
    /// A catalog entry is a blank-node graph name, which V3 excludes.
    BlankGraphName,
    /// Converted N-Quads plus catalog bytes exceed the request's V3 byte policy.
    Capacity,
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Syntax => "original Turtle parse rejected",
            Self::RelativeIriWithoutBase => "original Turtle needs an undeclared base IRI",
            Self::BlankGraphName => "blank-node graph names are not admitted",
            Self::Capacity => "converted N-Quads exceed the V3 dataset byte policy",
        })
    }
}

impl std::error::Error for SourceError {}

/// Converts one original Turtle document into exact V3 N-Quads.
///
/// No external base is supplied. Lexical forms and IRIs are copied unchanged;
/// blank nodes become `_:b0`, `_:b1`, ... by first occurrence. Output stops as
/// soon as N-Quads plus catalog bytes exceed `policy.dataset.max_dataset_bytes`,
/// the same accounting V3 uses, so prefix expansion cannot grow without bound.
/// The first failure in document order is reported.
///
/// # Examples
///
/// ```
/// use sparq_proved_evaluator_model::{replay, v3};
///
/// let turtle = "@prefix ex: <http://ex/> .\nex:a ex:p 01 .\n";
/// let source = replay::convert_turtle(turtle, &[], &v3::Policy::default())?;
/// assert_eq!(
///     source.nquads(),
///     "<http://ex/a> <http://ex/p> \"01\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
/// );
/// # Ok::<(), replay::SourceError>(())
/// ```
///
/// # Errors
/// Returns [`SourceError`] for malformed Turtle, a needed but undeclared base,
/// a blank-node graph name in the catalog, or converted bytes above the policy.
///
/// # Panics
/// Panics only if the fixed classification base is not an absolute IRI, which
/// is a programming error.
pub fn convert_turtle(
    turtle: &str,
    named_graphs: &[String],
    policy: &v3::Policy,
) -> Result<ConvertedSource, SourceError> {
    if named_graphs.iter().any(|name| name.starts_with("_:")) {
        return Err(SourceError::BlankGraphName);
    }
    let catalog = named_graphs
        .iter()
        .try_fold(0_usize, |bytes, name| bytes.checked_add(name.len()))
        .ok_or(SourceError::Capacity)?;
    let limit = (policy.dataset.max_dataset_bytes as usize)
        .checked_sub(catalog)
        .ok_or(SourceError::Capacity)?;
    let mut labels = Labels::default();
    let mut nquads = String::new();
    let mut statements = 0_u32;
    for triple in oxttl::TurtleParser::new().for_slice(turtle.as_bytes()) {
        let Ok(mut triple) = triple else {
            return Err(classify_failure(turtle));
        };
        labels.relabel(&mut triple);
        nquads += &triple.to_string();
        nquads += " .\n";
        if nquads.len() > limit {
            return Err(SourceError::Capacity);
        }
        statements = statements.checked_add(1).ok_or(SourceError::Capacity)?;
    }
    Ok(ConvertedSource {
        nquads,
        named_graphs: named_graphs.to_vec(),
        statements,
        blank_nodes: u32::try_from(labels.0.len()).map_err(|_| SourceError::Capacity)?,
    })
}

// A second, discarded parse distinguishes a missing base from other syntax errors
// without inspecting parser message text.
fn classify_failure(turtle: &str) -> SourceError {
    let parser = oxttl::TurtleParser::new()
        .with_base_iri(CLASSIFICATION_BASE)
        .expect("the classification base is an absolute IRI");
    if parser
        .for_slice(turtle.as_bytes())
        .all(|triple| triple.is_ok())
    {
        SourceError::RelativeIriWithoutBase
    } else {
        SourceError::Syntax
    }
}

#[derive(Default)]
struct Labels(BTreeMap<String, BlankNode>);

impl Labels {
    fn node(&mut self, node: &BlankNode) -> BlankNode {
        let next = self.0.len();
        self.0
            .entry(node.as_str().to_owned())
            .or_insert_with(|| BlankNode::new_unchecked(format!("b{next}")))
            .clone()
    }

    // Recursion depth equals triple-term nesting, which the parser has already
    // materialized; V3 later rejects every triple term in source data.
    fn relabel(&mut self, triple: &mut Triple) {
        if let NamedOrBlankNode::BlankNode(node) = &mut triple.subject {
            *node = self.node(node);
        }
        match &mut triple.object {
            Term::BlankNode(node) => *node = self.node(node),
            Term::Triple(inner) => self.relabel(inner),
            Term::NamedNode(_) | Term::Literal(_) => {}
        }
    }
}

/// Retained file named by a digest or identity failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetainedFile {
    /// `record.json`.
    Record,
    /// `query.rq`.
    Query,
    /// `data.ttl`.
    Data,
}

impl fmt::Display for RetainedFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Record => "record.json",
            Self::Query => "query.rq",
            Self::Data => "data.ttl",
        })
    }
}

/// Manifest protocol or field-format failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManifestError {
    /// The verifier manifest names another protocol version.
    VerifierSchema,
    /// The expected-result manifest names another protocol version.
    ExpectedSchema,
    /// The holder manifest names another protocol version.
    HolderSchema,
    /// A cell identifier is empty, too long or has unsafe characters.
    CellIdentity,
    /// A declared digest is not 64 lowercase hexadecimal digits.
    DigestFormat,
    /// Oracle provenance text is empty or exceeds its review bound.
    OracleProvenance,
    /// A fixed salt was supplied without declaring synthetic inputs.
    FixedSaltWithoutSyntheticInputs,
    /// The salt is all zero.
    ZeroSalt,
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::VerifierSchema => "verifier manifest protocol version rejected",
            Self::ExpectedSchema => "expected-result manifest protocol version rejected",
            Self::HolderSchema => "holder manifest protocol version rejected",
            Self::CellIdentity => "cell identity rejected",
            Self::DigestFormat => "declared digest is not lowercase SHA-256 hex",
            Self::OracleProvenance => "oracle provenance text rejected",
            Self::FixedSaltWithoutSyntheticInputs => {
                "fixed salt requires explicitly synthetic inputs"
            }
            Self::ZeroSalt => "all-zero salt rejected",
        })
    }
}

/// Reviewed interpretation of a failure; only `Unsupported` is an exclusion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Disposition {
    /// Retained inputs, manifests, anchor or independent expectation disagree.
    ContractViolation,
    /// A registered V3 profile refusal, or an excluded blank-node graph name.
    Unsupported,
    /// A registered V3 capacity, typed row/byte/domain limit, or adapter bound.
    Capacity,
    /// Malformed original input or a registered parse refusal.
    Malformed,
    /// Anything else, including diagnostics absent from the reviewed registry.
    Unexpected,
}

/// Fail-closed preparation or native-check failure with its exact cause.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplayError {
    /// A manifest has another protocol version or a malformed field.
    Manifest(ManifestError),
    /// A retained file differs from its verifier-declared digest.
    OriginalDigest(RetainedFile),
    /// The retained record is not the expected JSON shape.
    RecordSyntax,
    /// The retained record has another schema.
    RecordSchema,
    /// The record's copy of an original differs from the retained file.
    RecordFileMismatch(RetainedFile),
    /// The record's seed, category or storage differs from the verifier cell.
    CellMismatch,
    /// The retained dataset format is not Turtle.
    UnsupportedFormat,
    /// The verifier request is not a valid V3 request.
    Request(Rejected),
    /// The verifier request query differs from the retained query bytes.
    RequestQuery,
    /// The original Turtle could not be converted.
    Source(SourceError),
    /// The V3 commitment rejected the prepared dataset.
    Commitment(Rejected),
    /// The prepared input does not open the verifier-agreed commitment.
    AnchorMismatch,
    /// The expectation names other original query or dataset bytes.
    ExpectedBinding,
    /// V3 admission refused the query before evaluation.
    Admission(Rejected),
    /// Native V3 evaluation rejected the prepared witness.
    Evaluation(EvaluationError),
    /// Blank-node bijection search exceeded its adapter bound.
    OracleCapacity,
    /// The result differs from the independent expectation.
    ExpectedMismatch,
}

impl ReplayError {
    /// Returns the stage name recorded in status files.
    pub fn stage(&self) -> &'static str {
        match self {
            Self::Manifest(_) => "manifest",
            Self::OriginalDigest(_) => "originals",
            Self::RecordSyntax
            | Self::RecordSchema
            | Self::RecordFileMismatch(_)
            | Self::CellMismatch
            | Self::UnsupportedFormat => "record",
            Self::Request(_) | Self::RequestQuery => "request",
            Self::Source(_) => "source",
            Self::Commitment(_) | Self::AnchorMismatch => "commitment",
            Self::Admission(_) => "admission",
            Self::Evaluation(_) => "evaluation",
            Self::ExpectedBinding | Self::OracleCapacity | Self::ExpectedMismatch => "oracle",
        }
    }

    /// Returns the reviewed disposition; unknown causes are never exclusions.
    ///
    /// # Panics
    /// Panics only if the committed rejection registry is not valid JSON, which
    /// is a programming error.
    pub fn disposition(&self) -> Disposition {
        match self {
            Self::Manifest(_)
            | Self::OriginalDigest(_)
            | Self::RecordSyntax
            | Self::RecordSchema
            | Self::RecordFileMismatch(_)
            | Self::CellMismatch
            | Self::UnsupportedFormat
            | Self::Request(_)
            | Self::RequestQuery
            | Self::AnchorMismatch
            | Self::ExpectedBinding
            | Self::ExpectedMismatch => Disposition::ContractViolation,
            Self::Source(SourceError::Syntax | SourceError::RelativeIriWithoutBase) => {
                Disposition::Malformed
            }
            Self::Source(SourceError::BlankGraphName) => Disposition::Unsupported,
            Self::Source(SourceError::Capacity) | Self::OracleCapacity => Disposition::Capacity,
            Self::Commitment(rejected) | Self::Admission(rejected) => registered(rejected.0),
            Self::Evaluation(error) => evaluation_disposition(error),
        }
    }
}

impl fmt::Display for ReplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Manifest(error) => fmt::Display::fmt(error, f),
            Self::OriginalDigest(file) => write!(f, "{file} differs from its verifier digest"),
            Self::RecordSyntax => f.write_str("retained record JSON rejected"),
            Self::RecordSchema => f.write_str("retained record schema rejected"),
            Self::RecordFileMismatch(file) => write!(f, "record copy differs from {file}"),
            Self::CellMismatch => f.write_str("record cell differs from the verifier cell"),
            Self::UnsupportedFormat => f.write_str("retained dataset format is not turtle"),
            Self::Request(rejected)
            | Self::Commitment(rejected)
            | Self::Admission(rejected) => f.write_str(rejected.0),
            Self::RequestQuery => f.write_str("verifier query differs from query.rq"),
            Self::Source(error) => fmt::Display::fmt(error, f),
            Self::AnchorMismatch => {
                f.write_str("prepared input does not open the verifier-agreed commitment")
            }
            Self::ExpectedBinding => f.write_str("expectation names other original inputs"),
            Self::Evaluation(error) => fmt::Display::fmt(error, f),
            Self::OracleCapacity => f.write_str("expectation bijection capacity exceeded"),
            Self::ExpectedMismatch => f.write_str("result differs from the independent expectation"),
        }
    }
}

impl std::error::Error for ReplayError {}

fn registry() -> Value {
    serde_json::from_str(REJECTION_REGISTRY).expect("the committed rejection registry is JSON")
}

fn category(entry: &Value) -> Disposition {
    match entry["category"].as_str() {
        Some("profile") => Disposition::Unsupported,
        Some("capacity") => Disposition::Capacity,
        Some("parse") => Disposition::Malformed,
        _ => Disposition::Unexpected,
    }
}

fn registered(diagnostic: &str) -> Disposition {
    category(&registry()["diagnostics"][diagnostic])
}

fn evaluation_disposition(error: &EvaluationError) -> Disposition {
    let cause = match error {
        EvaluationError::Rejected(rejected) => return registered(rejected.0),
        EvaluationError::Budget(BudgetExceeded::Rows) => "budget_rows",
        EvaluationError::Budget(BudgetExceeded::Bytes) => "budget_bytes",
        EvaluationError::Budget(BudgetExceeded::Deadline) => "budget_deadline",
        EvaluationError::Budget(BudgetExceeded::Cancelled) => "budget_cancelled",
        EvaluationError::Capacity(EvaluationCapacity::NumericRepresentation) => {
            "numeric_representation"
        }
        EvaluationError::Capacity(EvaluationCapacity::TemporalYear) => "temporal_year",
        EvaluationError::Execution => "execution",
    };
    category(&registry()["typed_causes"][cause])
}

#[derive(Deserialize)]
struct Record {
    schema: String,
    seed: u64,
    category: String,
    storage: String,
    input: RecordInput,
    comparison: RecordComparison,
}

#[derive(Deserialize)]
struct RecordInput {
    query: String,
    dataset: String,
    format: String,
    named_graph_catalog: Vec<String>,
}

#[derive(Deserialize)]
struct RecordComparison {
    status: String,
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn valid_digest(digest: &str) -> bool {
    digest.len() == 64 && digest.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn valid_identifier(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_IDENTIFIER_BYTES
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl VerifierManifest {
    fn validate(&self) -> Result<(), ReplayError> {
        if self.schema != VERIFIER_SCHEMA {
            return Err(ReplayError::Manifest(ManifestError::VerifierSchema));
        }
        let cell = &self.cell;
        if ![&cell.profile, &cell.category, &cell.storage]
            .into_iter()
            .all(|text| valid_identifier(text))
        {
            return Err(ReplayError::Manifest(ManifestError::CellIdentity));
        }
        let digests = &self.originals;
        if ![&digests.record_sha256, &digests.query_sha256, &digests.data_sha256]
            .into_iter()
            .all(|digest| valid_digest(digest))
        {
            return Err(ReplayError::Manifest(ManifestError::DigestFormat));
        }
        Ok(())
    }
}

/// A checked V3 witness for one retained cell; this is not a proof.
#[derive(Clone)]
pub struct Prepared {
    witness: v3::Witness,
    cell: Cell,
    originals: OriginalDigests,
    native_comparison: String,
    statements: u32,
    blank_nodes: u32,
    commitment: [u8; 32],
}

impl fmt::Debug for Prepared {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Prepared")
            .field("cell", &self.cell)
            .field("native_comparison", &self.native_comparison)
            .finish_non_exhaustive()
    }
}

/// Sensitive local summary of a prepared witness; never publish it.
///
/// Digests of low-entropy test data can be matched by guessing, so this belongs
/// only in a protected local test artifact, never in a public presentation.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct PrivateSummary {
    /// SHA-256 of the exact prepared N-Quads bytes.
    pub nquads_sha256: String,
    /// SHA-256 of the JSON-serialized V3 witness, including request and salt.
    pub witness_json_sha256: String,
    /// Source statements before deduplication.
    pub statements: u32,
    /// Distinct relabeled blank nodes.
    pub blank_nodes: u32,
    /// Entries in the named-graph catalog.
    pub named_graphs: usize,
}

impl fmt::Debug for PrivateSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PrivateSummary([REDACTED])")
    }
}

impl Prepared {
    /// The exact V3 witness to prove; it contains the private dataset and salt.
    pub fn witness(&self) -> &v3::Witness {
        &self.witness
    }

    /// The verifier-declared cell that this witness replays.
    pub fn cell(&self) -> &Cell {
        &self.cell
    }

    /// Checked digests of the exact retained files.
    pub fn originals(&self) -> &OriginalDigests {
        &self.originals
    }

    /// Native differential comparison status retained from the record.
    ///
    /// This is an observation of the native replay, not an independent golden,
    /// and a later proof does not change that classification.
    pub fn native_comparison(&self) -> &str {
        &self.native_comparison
    }

    /// V3 commitment of the prepared dataset under the verifier's policy.
    pub fn dataset_commitment(&self) -> [u8; 32] {
        self.commitment
    }

    /// Builds the sensitive local witness summary.
    ///
    /// # Panics
    /// Panics only if JSON serialization of the witness fails, which is a
    /// programming error for these plain data types.
    pub fn private_summary(&self) -> PrivateSummary {
        let witness =
            serde_json::to_vec(&self.witness).expect("V3 witness JSON serialization is infallible");
        PrivateSummary {
            nquads_sha256: sha256_hex(self.witness.dataset.nquads.as_bytes()),
            witness_json_sha256: sha256_hex(&witness),
            statements: self.statements,
            blank_nodes: self.blank_nodes,
            named_graphs: self.witness.dataset.named_graphs.len(),
        }
    }

    /// Runs V3 admission and native evaluation, then checks the expectation.
    ///
    /// The returned journal is native differential evidence about this witness,
    /// not a proof and not a golden. The expectation is never derived from it.
    ///
    /// # Errors
    /// Rejects a malformed or differently bound expectation, V3 admission or
    /// evaluation refusal, bijection capacity and any result mismatch.
    pub fn evaluate_against(
        &self,
        expected: &ExpectedManifest,
    ) -> Result<v3::Journal, ReplayError> {
        expected.validate()?;
        if expected.query_sha256 != self.originals.query_sha256
            || expected.data_sha256 != self.originals.data_sha256
        {
            return Err(ReplayError::ExpectedBinding);
        }
        v3::admit(&self.witness.request).map_err(ReplayError::Admission)?;
        let journal = v3::evaluate_detailed(&self.witness).map_err(ReplayError::Evaluation)?;
        if !expected.matches(&journal.result)? {
            return Err(ReplayError::ExpectedMismatch);
        }
        Ok(journal)
    }
}

/// Checks one retained cell against verifier expectations and builds its witness.
///
/// Checks run in order: manifests, verifier-declared file digests, the record
/// schema, record copies of both originals, cell identity and format, the V3
/// request and its query bytes, Turtle conversion, the V3 commitment and, for
/// verifier-agreed authority, the agreed commitment. The request is cloned
/// unchanged into the witness.
///
/// # Errors
/// Returns the first failing check as a [`ReplayError`].
pub fn prepare(
    originals: Originals<'_>,
    verifier: &VerifierManifest,
    holder: &HolderManifest,
) -> Result<Prepared, ReplayError> {
    verifier.validate()?;
    let salt = holder.validated_salt()?;
    let actual = OriginalDigests {
        record_sha256: sha256_hex(originals.record),
        query_sha256: sha256_hex(originals.query),
        data_sha256: sha256_hex(originals.data),
    };
    let declared = &verifier.originals;
    for (file, expected, observed) in [
        (RetainedFile::Record, &declared.record_sha256, &actual.record_sha256),
        (RetainedFile::Query, &declared.query_sha256, &actual.query_sha256),
        (RetainedFile::Data, &declared.data_sha256, &actual.data_sha256),
    ] {
        if expected != observed {
            return Err(ReplayError::OriginalDigest(file));
        }
    }
    let record: Record =
        serde_json::from_slice(originals.record).map_err(|_| ReplayError::RecordSyntax)?;
    if record.schema != RECORD_SCHEMA {
        return Err(ReplayError::RecordSchema);
    }
    if record.input.query.as_bytes() != originals.query {
        return Err(ReplayError::RecordFileMismatch(RetainedFile::Query));
    }
    if record.input.dataset.as_bytes() != originals.data {
        return Err(ReplayError::RecordFileMismatch(RetainedFile::Data));
    }
    let cell = &verifier.cell;
    if record.seed != cell.seed || record.category != cell.category || record.storage != cell.storage
    {
        return Err(ReplayError::CellMismatch);
    }
    if record.input.format != "turtle" {
        return Err(ReplayError::UnsupportedFormat);
    }
    let request = &verifier.request;
    v3::validate_request(request).map_err(ReplayError::Request)?;
    if request.query.as_bytes() != originals.query {
        return Err(ReplayError::RequestQuery);
    }
    let converted = convert_turtle(
        &record.input.dataset,
        &record.input.named_graph_catalog,
        &request.policy,
    )
    .map_err(ReplayError::Source)?;
    let (statements, blank_nodes) = (converted.statements, converted.blank_nodes);
    let dataset = converted.into_dataset(salt);
    let commitment =
        v3::dataset_commitment(&dataset, &request.policy).map_err(ReplayError::Commitment)?;
    // The verifier's agreed value is compared, never recomputed or replaced.
    if matches!(
        &request.authority,
        DatasetAuthority::VerifierAgreed { commitment: agreed } if *agreed != commitment
    ) {
        return Err(ReplayError::AnchorMismatch);
    }
    Ok(Prepared {
        witness: v3::Witness {
            request: request.clone(),
            dataset,
        },
        cell: cell.clone(),
        originals: actual,
        native_comparison: record.comparison.status,
        statements,
        blank_nodes,
        commitment,
    })
}

type Rows = Vec<Vec<Option<String>>>;

fn same_result(
    expected: &v3::CanonicalResult,
    actual: &v3::CanonicalResult,
) -> Result<bool, ReplayError> {
    let (
        v3::CanonicalResult::Select {
            variables,
            order,
            rows,
        },
        v3::CanonicalResult::Select {
            variables: actual_variables,
            order: actual_order,
            rows: actual_rows,
        },
    ) = (expected, actual)
    else {
        // ASK booleans and RDFC-1.0 canonical graph bytes each have one exact form.
        return Ok(expected == actual);
    };
    if variables != actual_variables
        || order != actual_order
        || rows.len() != actual_rows.len()
        || rows
            .iter()
            .chain(actual_rows)
            .any(|row| row.len() != variables.len())
    {
        return Ok(false);
    }
    let labels: Vec<&str> = blank_labels(rows).into_iter().collect();
    let mut candidates: Vec<&str> = blank_labels(actual_rows).into_iter().collect();
    if labels.len() != candidates.len() {
        return Ok(false);
    }
    let cells = u64::try_from(rows.len().saturating_mul(variables.len()).max(1))
        .unwrap_or(u64::MAX);
    (1..=labels.len() as u64)
        .try_fold(cells, |steps, n| {
            steps
                .checked_mul(n)
                .filter(|steps| *steps <= MAX_BIJECTION_STEPS)
        })
        .ok_or(ReplayError::OracleCapacity)?;
    let target = arranged(actual_rows.clone(), order);
    let mut found = mapped_equal(rows, &labels, &candidates, order, &target);
    // Iterative Heap's algorithm visits every remaining candidate bijection once.
    let mut counters = vec![0_usize; candidates.len()];
    let mut index = 1;
    while !found && index < candidates.len() {
        if counters[index] < index {
            let other = if index % 2 == 0 { 0 } else { counters[index] };
            candidates.swap(other, index);
            found = mapped_equal(rows, &labels, &candidates, order, &target);
            counters[index] += 1;
            index = 1;
        } else {
            counters[index] = 0;
            index += 1;
        }
    }
    Ok(found)
}

// Canonical N-Triples term syntax: only blank nodes start with `_:`.
fn blank_labels(rows: &[Vec<Option<String>>]) -> BTreeSet<&str> {
    rows.iter()
        .flatten()
        .flatten()
        .map(String::as_str)
        .filter(|term| term.starts_with("_:"))
        .collect()
}

fn arranged(mut rows: Rows, order: &RowOrder) -> Rows {
    if *order == RowOrder::Bag {
        rows.sort();
    }
    rows
}

fn mapped_equal(
    rows: &[Vec<Option<String>>],
    labels: &[&str],
    candidates: &[&str],
    order: &RowOrder,
    target: &[Vec<Option<String>>],
) -> bool {
    let mapping: BTreeMap<&str, &str> = labels.iter().copied().zip(candidates.iter().copied()).collect();
    let mapped = rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| {
                    cell.as_deref()
                        .map(|term| mapping.get(term).copied().unwrap_or(term).to_owned())
                })
                .collect()
        })
        .collect();
    arranged(mapped, order) == target
}
