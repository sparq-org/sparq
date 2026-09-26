//! Versioned, deterministic LOCAL byte encoding of stored requests and descriptors.
//!
//! [OPUS-5.5] Profile [`LOCAL_ENCODING_PROFILE`] (`local-struct-v1`) maps a
//! [`StoredRequest`] or a [`MethodDescriptor`] to exact bytes. It is a
//! STRUCTURAL encoding of this crate's typed values: it is not RDF or SPARQL
//! canonicalization, not the draft §7.2 wire profile, not an interoperable
//! transport format, and there is no decoder. The encoder computes no digest;
//! an adapter that hashes these bytes names its own hash. Nothing here reads
//! `Debug` output, a serializer's field order or an enum discriminant.
//!
//! # Schema
//!
//! Each object starts with its domain separator ([`STORED_REQUEST_DOMAIN`] or
//! [`METHOD_DESCRIPTOR_DOMAIN`], each ending in a zero byte), then every field
//! as a one-byte tag followed by its value, in ascending tag order. Values:
//!
//! - `bytes`: u64 big-endian length, then the exact bytes;
//! - `u32` / `u64`: fixed-width big-endian;
//! - `fixed32`: the 32 raw bytes of a digest or challenge, no length;
//! - `enum`: one byte from the tables below;
//! - `option<T>`: `0x00` for absent, or `0x01` then `T`;
//! - `list<T>`: u64 big-endian count, then each `T` in STORED order (never sorted).
//!
//! Stored request:
//!
//! | Tag | Field | Value |
//! |---|---|---|
//! | `0x01` | query | `bytes` (exact UTF-8) |
//! | `0x02` | base IRI | `option<bytes>` |
//! | `0x03` | challenge | `fixed32` |
//! | `0x04` | audience | `bytes` |
//! | `0x05` | not before | `u64` |
//! | `0x06` | not after | `u64` |
//! | `0x07` | form | `enum` |
//! | `0x08` | DESCRIBE policy | `option<bytes>` |
//! | `0x09` | result contract | `enum` |
//! | `0x0a` | mode | `enum` |
//! | `0x0b` | authority | `enum` |
//! | `0x0c` | anchor | `option<fixed32>` |
//! | `0x0d` | source evidence | `enum` |
//! | `0x0e` | status | `enum` |
//! | `0x0f` | holder | `enum` |
//! | `0x10` | assembly | `enum` |
//! | `0x11` | dialect | `bytes` |
//! | `0x12` | fragment | `bytes` |
//! | `0x13` | accepted suites | `list<bytes>` |
//! | `0x14` | accepted mappings | `list<bytes>` |
//! | `0x15` | accepted linking | `list<bytes>` |
//! | `0x16` | released-row bound | `u32` |
//! | `0x17` | presentation-byte bound | `u32` |
//! | `0x18` | methods, preference order | `list<bytes>`, each a full descriptor encoding |
//!
//! Method descriptor:
//!
//! | Tag | Field | Value |
//! |---|---|---|
//! | `0x01` | method id | `bytes` |
//! | `0x02` | version | `u32` |
//! | `0x03` | parameter set | `bytes` |
//! | `0x04` | parameter digest | `fixed32` |
//! | `0x05` | artifact | variant byte, then its `fixed32` fields in declaration order |
//! | `0x06` | backend pin | `bytes` |
//!
//! Enum bytes: form `Select 01, Ask 02, Construct 03, Describe 04`; contract
//! `SelectDistinctSet 01, SelectBag 02, SelectSequence 03, AskBoolean 04,
//! AskTrueOnly 05, GraphRdfc10 06`; mode `SelectedSupport 01, ExactBounded 02`;
//! authority `VerifierAgreedAnchor 01, HolderDeclared 02`; source evidence
//! `None 01, IssuerAuthenticated 02, ReAttested 03`; status `Required 01,
//! NotRequested 02`; holder `Required 01, BearerAccepted 02`; assembly
//! `UnionDefaultGraph 01, CredentialNamedGraphs 02, ExactSourceCatalog 03`;
//! artifact `VerificationKey 01 (vk), ZkvmGuest 02 (artifact, image id),
//! Composite 03 (circuit, setup)`.
//!
//! Any change to a tag, table, order or value form needs a new profile name.
//! The encoder destructures every struct and matches every enum exhaustively,
//! so adding a field or variant fails to compile until the encoder handles it.

use crate::descriptor::{
    ArtifactIdentity, DatasetAssembly, EvaluationMode, HolderPolicy, MethodDescriptor,
    ResourceBounds, ResultContract, ScopeAuthority, SourceEvidence, StatusPolicy,
};
use crate::ids::{Identifier, QueryProfile};
use crate::negotiate::QueryRequirements;
use crate::request::{QueryForm, StoredRequest};

/// Name of this local encoding profile.
pub const LOCAL_ENCODING_PROFILE: &str = "local-struct-v1";

/// Domain separator that starts every stored-request encoding.
pub const STORED_REQUEST_DOMAIN: &[u8] = b"sparq:vcq:stored-request:local-struct-v1\0";

/// Domain separator that starts every method-descriptor encoding.
pub const METHOD_DESCRIPTOR_DOMAIN: &[u8] = b"sparq:vcq:method-descriptor:local-struct-v1\0";

// Every length and count is written as u64, which holds any `usize` on every
// supported target; this makes the conversion below infallible.
const _: () = assert!(usize::BITS <= u64::BITS);

struct Writer(Vec<u8>);

impl Writer {
    fn field(&mut self, tag: u8) -> &mut Self {
        self.0.push(tag);
        self
    }

    fn byte(&mut self, value: u8) {
        self.0.push(value);
    }

    fn u32(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_be_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.0.extend_from_slice(&value.to_be_bytes());
    }

    fn len(&mut self, len: usize) {
        self.u64(u64::try_from(len).expect("usize fits in u64, asserted at compile time"));
    }

    fn bytes(&mut self, bytes: &[u8]) {
        self.len(bytes.len());
        self.0.extend_from_slice(bytes);
    }

    fn fixed(&mut self, bytes: &[u8; 32]) {
        self.0.extend_from_slice(bytes);
    }

    fn opt_bytes(&mut self, bytes: Option<&[u8]>) {
        match bytes {
            None => self.byte(0x00),
            Some(bytes) => {
                self.byte(0x01);
                self.bytes(bytes);
            }
        }
    }

    fn ids(&mut self, ids: &[Identifier]) {
        self.len(ids.len());
        for id in ids {
            self.bytes(id.as_str().as_bytes());
        }
    }
}

/// Encodes `request` under [`LOCAL_ENCODING_PROFILE`].
///
/// Every field of the stored request, its requirements and each nested
/// descriptor is bound, in the order and form of the module schema. Equal
/// requests encode to equal bytes; this is not a canonical form of the query.
///
/// # Examples
///
/// See the crate-level example; `encode_stored_request(&request)` returns bytes
/// starting with [`STORED_REQUEST_DOMAIN`].
#[must_use]
pub fn encode_stored_request(request: &StoredRequest) -> Vec<u8> {
    let StoredRequest {
        requirements,
        query,
        challenge,
        audience,
        not_before,
        not_after,
        form,
        base_iri,
        describe_policy,
    } = request;
    let QueryRequirements {
        contract,
        mode,
        authority,
        anchor,
        source_evidence,
        status,
        holder,
        assembly,
        query_profile: QueryProfile { dialect, fragment },
        accepted_suites,
        accepted_mappings,
        accepted_linking,
        resources:
            ResourceBounds {
                released_rows,
                presentation_bytes,
            },
        methods,
    } = requirements;
    let mut w = Writer(STORED_REQUEST_DOMAIN.to_vec());
    w.field(0x01).bytes(query.as_bytes());
    w.field(0x02).opt_bytes(base_iri.as_ref().map(|base| base.as_str().as_bytes()));
    w.field(0x03).fixed(challenge.as_bytes());
    w.field(0x04).bytes(audience.as_str().as_bytes());
    w.field(0x05).u64(*not_before);
    w.field(0x06).u64(*not_after);
    w.field(0x07).byte(form_byte(*form));
    w.field(0x08).opt_bytes(describe_policy.as_ref().map(|p| p.as_str().as_bytes()));
    w.field(0x09).byte(contract_byte(*contract));
    w.field(0x0a).byte(mode_byte(*mode));
    w.field(0x0b).byte(authority_byte(*authority));
    match anchor {
        None => w.field(0x0c).byte(0x00),
        Some(anchor) => {
            w.field(0x0c).byte(0x01);
            w.fixed(anchor.as_bytes());
        }
    }
    w.field(0x0d).byte(source_evidence_byte(*source_evidence));
    w.field(0x0e).byte(status_byte(*status));
    w.field(0x0f).byte(holder_byte(*holder));
    w.field(0x10).byte(assembly_byte(*assembly));
    w.field(0x11).bytes(dialect.as_str().as_bytes());
    w.field(0x12).bytes(fragment.as_str().as_bytes());
    w.field(0x13).ids(accepted_suites);
    w.field(0x14).ids(accepted_mappings);
    w.field(0x15).ids(accepted_linking);
    w.field(0x16).u32(*released_rows);
    w.field(0x17).u32(*presentation_bytes);
    w.field(0x18).len(methods.len());
    for method in methods {
        w.bytes(&encode_method_descriptor(method));
    }
    w.0
}

/// Encodes `descriptor` under [`LOCAL_ENCODING_PROFILE`].
///
/// The same bytes appear, length-prefixed, inside a stored-request encoding.
#[must_use]
pub fn encode_method_descriptor(descriptor: &MethodDescriptor) -> Vec<u8> {
    let MethodDescriptor {
        method,
        version,
        parameter_set,
        parameter_digest,
        artifact,
        backend,
    } = descriptor;
    let mut w = Writer(METHOD_DESCRIPTOR_DOMAIN.to_vec());
    w.field(0x01).bytes(method.as_str().as_bytes());
    w.field(0x02).u32(*version);
    w.field(0x03).bytes(parameter_set.as_str().as_bytes());
    w.field(0x04).fixed(parameter_digest.as_bytes());
    match artifact {
        ArtifactIdentity::VerificationKey { vk_digest } => {
            w.field(0x05).byte(0x01);
            w.fixed(vk_digest.as_bytes());
        }
        ArtifactIdentity::ZkvmGuest {
            artifact_digest,
            image_id,
        } => {
            w.field(0x05).byte(0x02);
            w.fixed(artifact_digest.as_bytes());
            w.fixed(image_id.as_bytes());
        }
        ArtifactIdentity::Composite {
            circuit_digest,
            setup_digest,
        } => {
            w.field(0x05).byte(0x03);
            w.fixed(circuit_digest.as_bytes());
            w.fixed(setup_digest.as_bytes());
        }
    }
    w.field(0x06).bytes(backend.as_str().as_bytes());
    w.0
}

fn form_byte(form: QueryForm) -> u8 {
    match form {
        QueryForm::Select => 0x01,
        QueryForm::Ask => 0x02,
        QueryForm::Construct => 0x03,
        QueryForm::Describe => 0x04,
    }
}

fn contract_byte(contract: ResultContract) -> u8 {
    match contract {
        ResultContract::SelectDistinctSet => 0x01,
        ResultContract::SelectBag => 0x02,
        ResultContract::SelectSequence => 0x03,
        ResultContract::AskBoolean => 0x04,
        ResultContract::AskTrueOnly => 0x05,
        ResultContract::GraphRdfc10 => 0x06,
    }
}

fn mode_byte(mode: EvaluationMode) -> u8 {
    match mode {
        EvaluationMode::SelectedSupport => 0x01,
        EvaluationMode::ExactBounded => 0x02,
    }
}

fn authority_byte(authority: ScopeAuthority) -> u8 {
    match authority {
        ScopeAuthority::VerifierAgreedAnchor => 0x01,
        ScopeAuthority::HolderDeclared => 0x02,
    }
}

fn source_evidence_byte(evidence: SourceEvidence) -> u8 {
    match evidence {
        SourceEvidence::None => 0x01,
        SourceEvidence::IssuerAuthenticated => 0x02,
        SourceEvidence::ReAttested => 0x03,
    }
}

fn status_byte(status: StatusPolicy) -> u8 {
    match status {
        StatusPolicy::Required => 0x01,
        StatusPolicy::NotRequested => 0x02,
    }
}

fn holder_byte(holder: HolderPolicy) -> u8 {
    match holder {
        HolderPolicy::Required => 0x01,
        HolderPolicy::BearerAccepted => 0x02,
    }
}

fn assembly_byte(assembly: DatasetAssembly) -> u8 {
    match assembly {
        DatasetAssembly::UnionDefaultGraph => 0x01,
        DatasetAssembly::CredentialNamedGraphs => 0x02,
        DatasetAssembly::ExactSourceCatalog => 0x03,
    }
}
