//! The bounded, versioned canonical delta envelope (research §6.1, proposal
//! `CRDT-WIRE-1..4`) and its strict codec.
//!
//! Wire shape (canonical JSON, all scalars strings, keys sorted):
//!
//! ```json
//! {"adds":[{"dot":["<b64url>","<dec>"],"quad":"<n-quads line>"}],
//!  "basis":{"clock":{"<b64url>":"<dec>"},"cloud":[["<b64url>","<dec>"]]},
//!  "dataset":"<absolute iri>",
//!  "epoch":"<dec>",
//!  "format":"sparq-crdt-delta/1",
//!  "origin":"<b64url>",
//!  "removes":[{"dots":[["<b64url>","<dec>"]],"quad":"<n-quads line>"}],
//!  "sequence":"<dec>"}
//! ```
//!
//! The `epoch` (membership epoch) field follows research §6.1; the proposal
//! draft's envelope sketch does not show it yet (freezing it is decision 5 of
//! research §11) and this implementation bead requires wrong-epoch rejection,
//! so it is part of the version-1 surface here.
//!
//! Decode order is: byte bound → strict JSON parse → structural + semantic
//! validation under [`Limits`] and [`Admission`] → re-encode → byte-compare.
//! The final comparison makes the codec reject *any* second byte form of an
//! envelope identity (whitespace, key order, unsorted arrays, duplicate keys,
//! non-shortest counters, alternative escapes …), which is what lets a
//! signature or AEAD at a surrounding security layer authenticate exactly one
//! encoding (research §6.1).

use crate::CrdtError;
use crate::codec::{
    expect_array, expect_object, expect_str, parse_dec_u64, parse_dot, parse_summary,
    write_json_string, write_summary,
};
use crate::id::{DatasetId, Dot, EnvelopeId, ReplicaId};
use crate::quad::CanonicalQuad;
use crate::summary::CausalSummary;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

/// The version-1 envelope format tag.
pub const ENVELOPE_FORMAT_V1: &str = "sparq-crdt-delta/1";

/// Domain separator for [`envelope_hash`] (research §6.1: the hash domain is
/// separated so an envelope digest can never collide with another artefact's
/// digest over the same bytes).
const ENVELOPE_HASH_DOMAIN: &[u8] = b"sparq-crdt:envelope:v1\0";

/// Resource bounds checked **before** allocation while decoding (research
/// §6.1: "Resource limits are checked before allocation"). All fields are
/// public so deployments can tighten them; [`Limits::default`] is the
/// recommended baseline.
#[derive(Clone, Debug)]
pub struct Limits {
    /// Maximum canonical envelope size in bytes.
    pub max_envelope_bytes: usize,
    /// Maximum number of dotted adds per envelope.
    pub max_adds: usize,
    /// Maximum number of observed-remove entries per envelope.
    pub max_removes: usize,
    /// Maximum observed dots per single remove entry.
    pub max_dots_per_remove: usize,
    /// Maximum bytes per canonical N-Quads quad line.
    pub max_quad_bytes: usize,
    /// Maximum clock entries per causal summary.
    pub max_clock_entries: usize,
    /// Maximum cloud dots per causal summary (malicious sparse-cloud growth
    /// is a mandatory concern, research §6.2).
    pub max_cloud_dots: usize,
    /// Maximum snapshot payload size in bytes.
    pub max_snapshot_bytes: usize,
    /// Maximum store entries (distinct quads) per snapshot.
    pub max_store_entries: usize,
    /// Maximum live dots per quad in a snapshot store entry.
    pub max_dots_per_quad: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_envelope_bytes: 1 << 20,       // 1 MiB
            max_adds: 65_536,
            max_removes: 65_536,
            max_dots_per_remove: 1_024,
            max_quad_bytes: 16 * 1024,
            max_clock_entries: 4_096,
            max_cloud_dots: 65_536,
            max_snapshot_bytes: 256 << 20,     // 256 MiB
            max_store_entries: 4_194_304,
            max_dots_per_quad: 1_024,
        }
    }
}

/// What a receiver admits: one dataset identity and one membership epoch.
/// Everything else is rejected before join (research §6.2 step 3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Admission {
    /// The dataset this receiver replicates.
    pub dataset: DatasetId,
    /// The current membership epoch.
    pub epoch: u64,
}

impl Admission {
    /// Convenience constructor.
    pub fn new(dataset: DatasetId, epoch: u64) -> Self {
        Admission { dataset, epoch }
    }

    /// Checks a decoded document's dataset + epoch against this admission.
    pub(crate) fn check(&self, dataset: &DatasetId, epoch: u64) -> Result<(), CrdtError> {
        if dataset != &self.dataset {
            return Err(CrdtError::WrongDataset {
                expected: self.dataset.as_str().to_owned(),
                found: dataset.as_str().to_owned(),
            });
        }
        if epoch != self.epoch {
            return Err(CrdtError::WrongEpoch {
                expected: self.epoch,
                found: epoch,
            });
        }
        Ok(())
    }
}

/// One dotted addition: a fresh [`Dot`] asserting one canonical quad.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DottedAdd {
    /// The asserted quad.
    pub quad: CanonicalQuad,
    /// The globally unique addition event.
    pub dot: Dot,
}

/// One observed removal: every live dot the origin snapshot held for the quad
/// (`CRDT-MUT-2`; never a wildcard or timestamp tombstone).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ObservedRemove {
    /// The quad whose observed dots are removed.
    pub quad: CanonicalQuad,
    /// The observed dots, strictly ascending, non-empty.
    pub dots: Vec<Dot>,
}

/// One atomic origin transaction's concrete CRDT delta, in canonical order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeltaEnvelope {
    dataset: DatasetId,
    epoch: u64,
    origin: ReplicaId,
    sequence: u64,
    basis: CausalSummary,
    adds: Vec<DottedAdd>,
    removes: Vec<ObservedRemove>,
}

impl DeltaEnvelope {
    /// Builds a well-formed envelope, sorting `adds`/`removes`/`dots` into
    /// canonical order and enforcing the `CRDT-WIRE-4` invariants: non-zero
    /// sequence; dot uniqueness across all adds; no add dot inside the remove
    /// set; at most one remove entry per quad; non-empty dot sets per remove.
    ///
    /// An envelope with empty `adds` **and** empty `removes` is permitted: it
    /// is the idempotency record of a semantic no-op (`CRDT-WIRE-2`).
    pub fn new(
        dataset: DatasetId,
        epoch: u64,
        origin: ReplicaId,
        sequence: u64,
        basis: CausalSummary,
        mut adds: Vec<DottedAdd>,
        mut removes: Vec<ObservedRemove>,
    ) -> Result<Self, CrdtError> {
        if sequence == 0 {
            return Err(CrdtError::Invalid {
                what: "envelope sequence",
                reason: "must be non-zero".into(),
            });
        }
        adds.sort();
        for pair in adds.windows(2) {
            if pair[0] == pair[1] {
                return Err(CrdtError::NonCanonical {
                    reason: "duplicate add entry".into(),
                });
            }
        }
        removes.sort_by(|a, b| a.quad.cmp(&b.quad));
        for pair in removes.windows(2) {
            if pair[0].quad == pair[1].quad {
                return Err(CrdtError::NonCanonical {
                    reason: "more than one remove entry for one quad".into(),
                });
            }
        }
        let mut removed_dots: BTreeSet<&Dot> = BTreeSet::new();
        for remove in &mut removes {
            if remove.dots.is_empty() {
                return Err(CrdtError::Invalid {
                    what: "envelope remove",
                    reason: "observed dot set must be non-empty".into(),
                });
            }
            remove.dots.sort();
            for pair in remove.dots.windows(2) {
                if pair[0] == pair[1] {
                    return Err(CrdtError::DuplicateDot {
                        replica: pair[0].replica().to_base64url(),
                        counter: pair[0].counter(),
                    });
                }
            }
        }
        for remove in &removes {
            removed_dots.extend(remove.dots.iter());
        }
        let mut add_dots: BTreeSet<&Dot> = BTreeSet::new();
        for add in &adds {
            if !add_dots.insert(&add.dot) || removed_dots.contains(&add.dot) {
                return Err(CrdtError::DuplicateDot {
                    replica: add.dot.replica().to_base64url(),
                    counter: add.dot.counter(),
                });
            }
        }
        Ok(DeltaEnvelope {
            dataset,
            epoch,
            origin,
            sequence,
            basis,
            adds,
            removes,
        })
    }

    /// The dataset this envelope belongs to.
    pub fn dataset(&self) -> &DatasetId {
        &self.dataset
    }

    /// The membership epoch the origin committed under.
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// The origin replica.
    pub fn origin(&self) -> &ReplicaId {
        &self.origin
    }

    /// The origin journal sequence (non-zero).
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    /// The origin's complete data causal context observed before evaluation.
    /// Audit/provenance metadata — **not** joined as a removal context.
    pub fn basis(&self) -> &CausalSummary {
        &self.basis
    }

    /// The dotted adds, in canonical (quad, replica-bytes, counter) order.
    pub fn adds(&self) -> &[DottedAdd] {
        &self.adds
    }

    /// The observed removes, in canonical quad order.
    pub fn removes(&self) -> &[ObservedRemove] {
        &self.removes
    }

    /// The envelope identity `(origin, sequence)` (`CRDT-UPD-RETRY-1`).
    pub fn id(&self) -> EnvelopeId {
        EnvelopeId::new(self.origin.clone(), self.sequence)
            .expect("sequence is validated non-zero at construction")
    }

    /// Encodes the canonical byte form. Infallible: every reachable
    /// `DeltaEnvelope` value is well-formed by construction.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = String::new();
        out.push_str("{\"adds\":[");
        for (i, add) in self.adds.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str("{\"dot\":");
            crate::codec::write_dot(&mut out, &add.dot);
            out.push_str(",\"quad\":");
            write_json_string(&mut out, add.quad.as_str());
            out.push('}');
        }
        out.push_str("],\"basis\":");
        write_summary(&mut out, &self.basis);
        out.push_str(",\"dataset\":");
        write_json_string(&mut out, self.dataset.as_str());
        out.push_str(",\"epoch\":");
        write_json_string(&mut out, &self.epoch.to_string());
        out.push_str(",\"format\":");
        write_json_string(&mut out, ENVELOPE_FORMAT_V1);
        out.push_str(",\"origin\":");
        write_json_string(&mut out, &self.origin.to_base64url());
        out.push_str(",\"removes\":[");
        for (i, remove) in self.removes.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str("{\"dots\":[");
            for (j, dot) in remove.dots.iter().enumerate() {
                if j > 0 {
                    out.push(',');
                }
                crate::codec::write_dot(&mut out, dot);
            }
            out.push_str("],\"quad\":");
            write_json_string(&mut out, remove.quad.as_str());
            out.push('}');
        }
        out.push_str("],\"sequence\":");
        write_json_string(&mut out, &self.sequence.to_string());
        out.push('}');
        out.into_bytes()
    }

    /// Strictly decodes and validates one canonical envelope for `admission`
    /// under `limits`.
    ///
    /// Rejects, in this order: oversized input; malformed JSON; an unknown
    /// `format` version (fail closed); a wrong dataset; a wrong membership
    /// epoch; every structural/bounds violation; and finally — by re-encoding
    /// and byte-comparing — any non-canonical byte form.
    pub fn decode(
        bytes: &[u8],
        admission: &Admission,
        limits: &Limits,
    ) -> Result<Self, CrdtError> {
        if bytes.len() > limits.max_envelope_bytes {
            return Err(CrdtError::Oversized {
                what: "envelope bytes",
                len: bytes.len(),
                max: limits.max_envelope_bytes,
            });
        }
        let value: Value = serde_json::from_slice(bytes).map_err(|e| CrdtError::Invalid {
            what: "envelope",
            reason: format!("not valid JSON: {e}"),
        })?;
        let map = expect_object(
            &value,
            "envelope",
            &[
                "adds", "basis", "dataset", "epoch", "format", "origin", "removes", "sequence",
            ],
        )?;
        let format = expect_str(map, "envelope", "format")?;
        if format != ENVELOPE_FORMAT_V1 {
            return Err(CrdtError::UnsupportedFormat {
                found: format.to_owned(),
            });
        }
        let dataset = DatasetId::new(expect_str(map, "envelope", "dataset")?)?;
        let epoch = parse_dec_u64(expect_str(map, "envelope", "epoch")?)?;
        admission.check(&dataset, epoch)?;
        let origin = ReplicaId::from_base64url(expect_str(map, "envelope", "origin")?)?;
        let sequence = parse_dec_u64(expect_str(map, "envelope", "sequence")?)?;
        let basis = parse_summary(
            map.get("basis").expect("key checked above"),
            "envelope basis",
            limits.max_clock_entries,
            limits.max_cloud_dots,
        )?;

        let adds_arr = expect_array(map, "envelope", "adds")?;
        if adds_arr.len() > limits.max_adds {
            return Err(CrdtError::Oversized {
                what: "envelope adds",
                len: adds_arr.len(),
                max: limits.max_adds,
            });
        }
        let mut adds = Vec::with_capacity(adds_arr.len());
        for item in adds_arr {
            let entry = expect_object(item, "envelope add", &["dot", "quad"])?;
            let quad =
                CanonicalQuad::parse(expect_str(entry, "envelope add", "quad")?, limits.max_quad_bytes)?;
            let dot = parse_dot(entry.get("dot").expect("key checked above"), "envelope add")?;
            adds.push(DottedAdd { quad, dot });
        }

        let removes_arr = expect_array(map, "envelope", "removes")?;
        if removes_arr.len() > limits.max_removes {
            return Err(CrdtError::Oversized {
                what: "envelope removes",
                len: removes_arr.len(),
                max: limits.max_removes,
            });
        }
        let mut removes = Vec::with_capacity(removes_arr.len());
        for item in removes_arr {
            let entry = expect_object(item, "envelope remove", &["dots", "quad"])?;
            let quad = CanonicalQuad::parse(
                expect_str(entry, "envelope remove", "quad")?,
                limits.max_quad_bytes,
            )?;
            let dots_arr = expect_array(entry, "envelope remove", "dots")?;
            if dots_arr.len() > limits.max_dots_per_remove {
                return Err(CrdtError::Oversized {
                    what: "envelope remove dots",
                    len: dots_arr.len(),
                    max: limits.max_dots_per_remove,
                });
            }
            let mut dots = Vec::with_capacity(dots_arr.len());
            for dot in dots_arr {
                dots.push(parse_dot(dot, "envelope remove")?);
            }
            removes.push(ObservedRemove { quad, dots });
        }

        let envelope =
            DeltaEnvelope::new(dataset, epoch, origin, sequence, basis, adds, removes)?;
        if envelope.encode() != bytes {
            return Err(CrdtError::NonCanonical {
                reason: "envelope bytes are not the canonical encoding of their content".into(),
            });
        }
        Ok(envelope)
    }
}

/// The domain-separated SHA-256 content hash of one canonical envelope's
/// bytes. This is the digest a surrounding signature/AEAD layer should bind
/// (research §6.1: neither should sign an ambiguous alternate encoding — the
/// decoder guarantees there is none).
pub fn envelope_hash(canonical_bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(ENVELOPE_HASH_DOMAIN);
    hasher.update(canonical_bytes);
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rid(bytes: &[u8]) -> ReplicaId {
        ReplicaId::new(bytes.to_vec()).expect("valid replica id")
    }

    fn dot(r: &[u8], c: u64) -> Dot {
        Dot::new(rid(r), c).expect("valid dot")
    }

    fn quad(line: &str) -> CanonicalQuad {
        CanonicalQuad::parse(line, 16 * 1024).expect("valid quad")
    }

    fn admission() -> Admission {
        Admission::new(DatasetId::new("https://example.test/datasets/team").unwrap(), 3)
    }

    fn sample() -> DeltaEnvelope {
        let mut basis = CausalSummary::new();
        basis.insert(dot(b"peer-a", 1));
        basis.insert(dot(b"peer-b", 2)); // gap ⇒ cloud entry
        DeltaEnvelope::new(
            admission().dataset,
            3,
            rid(b"peer-a"),
            42,
            basis,
            vec![DottedAdd {
                quad: quad("<https://ex/s> <https://ex/p> \"new\" <https://ex/g> ."),
                dot: dot(b"peer-a", 2),
            }],
            vec![ObservedRemove {
                quad: quad("<https://ex/s> <https://ex/p> \"old\" <https://ex/g> ."),
                dots: vec![dot(b"peer-b", 1)],
            }],
        )
        .expect("well-formed sample envelope")
    }

    #[test]
    fn new_sorts_into_canonical_order() {
        let q1 = quad("<https://ex/a> <https://ex/p> \"1\" .");
        let q2 = quad("<https://ex/b> <https://ex/p> \"2\" .");
        let env = DeltaEnvelope::new(
            admission().dataset,
            3,
            rid(b"peer-a"),
            1,
            CausalSummary::new(),
            vec![
                DottedAdd { quad: q2.clone(), dot: dot(b"peer-a", 3) },
                DottedAdd { quad: q1.clone(), dot: dot(b"peer-a", 2) },
            ],
            vec![ObservedRemove {
                quad: q1.clone(),
                dots: vec![dot(b"peer-b", 2), dot(b"peer-b", 1)],
            }],
        )
        .unwrap();
        assert_eq!(env.adds()[0].quad, q1);
        assert_eq!(env.adds()[1].quad, q2);
        assert_eq!(env.removes()[0].dots, vec![dot(b"peer-b", 1), dot(b"peer-b", 2)]);
    }

    #[test]
    fn new_rejects_wire4_violations() {
        let q = quad("<https://ex/s> <https://ex/p> \"x\" .");
        // Zero sequence.
        assert!(DeltaEnvelope::new(
            admission().dataset,
            3,
            rid(b"peer-a"),
            0,
            CausalSummary::new(),
            Vec::new(),
            Vec::new()
        )
        .is_err());
        // Same dot under two adds.
        assert!(DeltaEnvelope::new(
            admission().dataset,
            3,
            rid(b"peer-a"),
            1,
            CausalSummary::new(),
            vec![
                DottedAdd { quad: q.clone(), dot: dot(b"peer-a", 2) },
                DottedAdd {
                    quad: quad("<https://ex/t> <https://ex/p> \"y\" ."),
                    dot: dot(b"peer-a", 2),
                },
            ],
            Vec::new()
        )
        .is_err());
        // An add dot inside the remove set.
        assert!(DeltaEnvelope::new(
            admission().dataset,
            3,
            rid(b"peer-a"),
            1,
            CausalSummary::new(),
            vec![DottedAdd { quad: q.clone(), dot: dot(b"peer-a", 2) }],
            vec![ObservedRemove { quad: q.clone(), dots: vec![dot(b"peer-a", 2)] }]
        )
        .is_err());
        // Empty observed dot set.
        assert!(DeltaEnvelope::new(
            admission().dataset,
            3,
            rid(b"peer-a"),
            1,
            CausalSummary::new(),
            Vec::new(),
            vec![ObservedRemove { quad: q.clone(), dots: Vec::new() }]
        )
        .is_err());
        // Two remove entries for one quad.
        assert!(DeltaEnvelope::new(
            admission().dataset,
            3,
            rid(b"peer-a"),
            1,
            CausalSummary::new(),
            Vec::new(),
            vec![
                ObservedRemove { quad: q.clone(), dots: vec![dot(b"peer-a", 1)] },
                ObservedRemove { quad: q.clone(), dots: vec![dot(b"peer-b", 1)] },
            ]
        )
        .is_err());
    }

    #[test]
    fn accessors_expose_the_constructed_fields() {
        let env = sample();
        assert_eq!(env.dataset().as_str(), "https://example.test/datasets/team");
        assert_eq!(env.epoch(), 3);
        assert_eq!(env.origin(), &rid(b"peer-a"));
        assert_eq!(env.sequence(), 42);
        assert!(env.basis().contains(&dot(b"peer-a", 1)));
        assert_eq!(env.adds().len(), 1);
        assert_eq!(env.removes().len(), 1);
    }

    #[test]
    fn id_is_origin_plus_sequence() {
        assert_eq!(sample().id(), Dot::new(rid(b"peer-a"), 42).unwrap());
    }

    #[test]
    fn encode_produces_the_golden_canonical_bytes() {
        let bytes = sample().encode();
        let expected = concat!(
            "{\"adds\":[{\"dot\":[\"cGVlci1h\",\"2\"],",
            "\"quad\":\"<https://ex/s> <https://ex/p> \\\"new\\\" <https://ex/g> .\"}],",
            "\"basis\":{\"clock\":{\"cGVlci1h\":\"1\"},\"cloud\":[[\"cGVlci1i\",\"2\"]]},",
            "\"dataset\":\"https://example.test/datasets/team\",",
            "\"epoch\":\"3\",",
            "\"format\":\"sparq-crdt-delta/1\",",
            "\"origin\":\"cGVlci1h\",",
            "\"removes\":[{\"dots\":[[\"cGVlci1i\",\"1\"]],",
            "\"quad\":\"<https://ex/s> <https://ex/p> \\\"old\\\" <https://ex/g> .\"}],",
            "\"sequence\":\"42\"}",
        );
        assert_eq!(String::from_utf8(bytes).unwrap(), expected);
    }

    #[test]
    fn decode_round_trips_the_canonical_bytes() {
        let env = sample();
        let bytes = env.encode();
        let decoded = DeltaEnvelope::decode(&bytes, &admission(), &Limits::default()).unwrap();
        assert_eq!(decoded, env);
    }

    #[test]
    fn decode_rejects_wrong_dataset_and_wrong_epoch() {
        let bytes = sample().encode();
        let other = Admission::new(DatasetId::new("https://example.test/other").unwrap(), 3);
        assert!(matches!(
            DeltaEnvelope::decode(&bytes, &other, &Limits::default()),
            Err(CrdtError::WrongDataset { .. })
        ));
        let stale = Admission::new(admission().dataset, 2);
        assert!(matches!(
            DeltaEnvelope::decode(&bytes, &stale, &Limits::default()),
            Err(CrdtError::WrongEpoch { .. })
        ));
    }

    #[test]
    fn decode_fails_closed_on_unknown_format_versions() {
        let bytes = String::from_utf8(sample().encode())
            .unwrap()
            .replace("sparq-crdt-delta/1", "sparq-crdt-delta/2")
            .into_bytes();
        assert!(matches!(
            DeltaEnvelope::decode(&bytes, &admission(), &Limits::default()),
            Err(CrdtError::UnsupportedFormat { .. })
        ));
    }

    #[test]
    fn decode_rejects_oversized_input_before_parsing() {
        let bytes = sample().encode();
        let limits = Limits { max_envelope_bytes: bytes.len() - 1, ..Limits::default() };
        assert!(matches!(
            DeltaEnvelope::decode(&bytes, &admission(), &limits),
            Err(CrdtError::Oversized { .. })
        ));
        // Item-count bounds too.
        let limits = Limits { max_adds: 0, ..Limits::default() };
        assert!(matches!(
            DeltaEnvelope::decode(&sample().encode(), &admission(), &limits),
            Err(CrdtError::Oversized { .. })
        ));
        let limits = Limits { max_removes: 0, ..Limits::default() };
        assert!(matches!(
            DeltaEnvelope::decode(&sample().encode(), &admission(), &limits),
            Err(CrdtError::Oversized { .. })
        ));
    }

    #[test]
    fn decode_rejects_every_second_byte_form() {
        let canonical = String::from_utf8(sample().encode()).unwrap();
        let variants = [
            format!(" {canonical}"),                                  // leading whitespace
            canonical.replace("\"epoch\":\"3\"", "\"epoch\":\"03\""), // non-shortest counter
            canonical.replace("\"epoch\":\"3\"", "\"epoch\" :\"3\""), // inner whitespace
            canonical.replace(
                "\"adds\":", // duplicate key: JSON parsers keep one, bytes differ
                "\"adds\":[],\"adds\":",
            ),
        ];
        for bytes in variants {
            assert!(
                DeltaEnvelope::decode(bytes.as_bytes(), &admission(), &Limits::default())
                    .is_err(),
                "{bytes:?} must be rejected"
            );
        }
        // Unsorted adds: build unsorted bytes by swapping two entries.
        let q1 = quad("<https://ex/a> <https://ex/p> \"1\" .");
        let q2 = quad("<https://ex/b> <https://ex/p> \"2\" .");
        let env = DeltaEnvelope::new(
            admission().dataset,
            3,
            rid(b"peer-a"),
            1,
            CausalSummary::new(),
            vec![
                DottedAdd { quad: q1, dot: dot(b"peer-a", 1) },
                DottedAdd { quad: q2, dot: dot(b"peer-a", 2) },
            ],
            Vec::new(),
        )
        .unwrap();
        let text = String::from_utf8(env.encode()).unwrap();
        // Renaming ex/a to ex/z leaves entry order [z, b]: valid quads, but no
        // longer in canonical (sorted) order, so re-encoding differs.
        let unsorted = text.replace("https://ex/a", "https://ex/z");
        assert!(matches!(
            DeltaEnvelope::decode(unsorted.as_bytes(), &admission(), &Limits::default()),
            Err(CrdtError::NonCanonical { .. })
        ));
    }

    #[test]
    fn decode_rejects_missing_or_extra_keys_and_non_string_scalars() {
        let canonical = String::from_utf8(sample().encode()).unwrap();
        let no_epoch = canonical.replace("\"epoch\":\"3\",", "");
        assert!(DeltaEnvelope::decode(no_epoch.as_bytes(), &admission(), &Limits::default())
            .is_err());
        let extra = canonical.replace("{\"adds\"", "{\"aaaa\":\"x\",\"adds\"");
        assert!(
            DeltaEnvelope::decode(extra.as_bytes(), &admission(), &Limits::default()).is_err()
        );
        let numeric = canonical.replace("\"sequence\":\"42\"", "\"sequence\":42");
        assert!(
            DeltaEnvelope::decode(numeric.as_bytes(), &admission(), &Limits::default()).is_err()
        );
    }

    #[test]
    fn decode_allows_the_empty_idempotency_record() {
        let env = DeltaEnvelope::new(
            admission().dataset,
            3,
            rid(b"peer-a"),
            7,
            CausalSummary::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let decoded =
            DeltaEnvelope::decode(&env.encode(), &admission(), &Limits::default()).unwrap();
        assert!(decoded.adds().is_empty());
        assert!(decoded.removes().is_empty());
    }

    #[test]
    fn envelope_hash_is_domain_separated_and_deterministic() {
        let bytes = sample().encode();
        assert_eq!(envelope_hash(&bytes), envelope_hash(&bytes));
        // Domain separation: differs from the plain SHA-256 of the bytes.
        let plain: [u8; 32] = Sha256::digest(&bytes).into();
        assert_ne!(envelope_hash(&bytes), plain);
        assert_ne!(envelope_hash(&bytes), envelope_hash(b"other"));
    }

    #[test]
    fn limits_default_is_the_recommended_baseline() {
        let limits = Limits::default();
        assert_eq!(limits.max_envelope_bytes, 1 << 20);
        assert!(limits.max_adds >= 1024);
        assert!(limits.max_quad_bytes >= 4096);
    }

    #[test]
    fn admission_new_and_check() {
        let a = admission();
        assert!(a.check(&a.dataset.clone(), 3).is_ok());
        assert!(a.check(&a.dataset.clone(), 4).is_err());
        assert!(a
            .check(&DatasetId::new("https://example.test/other").unwrap(), 3)
            .is_err());
    }
}
