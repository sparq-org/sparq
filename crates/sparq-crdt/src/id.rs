//! Identifier contract of the SPARQL-CRDT design (research §4.1): replica
//! identifiers, dataset identifiers, dots, and envelope identities.
//!
//! `ReplicaId` ordering is by **raw identifier bytes** (not by the
//! base64url text), because the proposal's canonical sort rules
//! (`CRDT-WIRE-3`) order dot arrays "by replica bytes then numeric counter"
//! and the base64url alphabet is not order-preserving over raw bytes.

use crate::CrdtError;
use crate::codec::{b64url_decode, b64url_encode};

/// Hard upper bound on a replica identifier's raw byte length. The design
/// recommends 128- or 256-bit random identifiers (research §4.1); 64 bytes
/// leaves headroom while keeping every identifier bounded.
pub const MAX_REPLICA_ID_BYTES: usize = 64;

/// Hard upper bound on a dataset identifier IRI's UTF-8 byte length.
pub const MAX_DATASET_IRI_BYTES: usize = 4096;

/// Opaque, globally unique identifier of one durable counter lineage
/// (research §4.1). Wire form is unpadded base64url of the raw bytes.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReplicaId(Vec<u8>);

impl ReplicaId {
    /// Wraps raw identifier bytes. Rejects an empty identifier and one longer
    /// than [`MAX_REPLICA_ID_BYTES`].
    pub fn new(bytes: impl Into<Vec<u8>>) -> Result<Self, CrdtError> {
        let bytes = bytes.into();
        if bytes.is_empty() {
            return Err(CrdtError::Invalid {
                what: "replica id",
                reason: "must not be empty".into(),
            });
        }
        if bytes.len() > MAX_REPLICA_ID_BYTES {
            return Err(CrdtError::Oversized {
                what: "replica id bytes",
                len: bytes.len(),
                max: MAX_REPLICA_ID_BYTES,
            });
        }
        Ok(ReplicaId(bytes))
    }

    /// The raw identifier bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The canonical wire form: unpadded base64url (RFC 4648 §5).
    pub fn to_base64url(&self) -> String {
        b64url_encode(&self.0)
    }

    /// Parses the canonical wire form. Strict: rejects padding, non-alphabet
    /// characters, impossible lengths, and non-zero trailing bits, so exactly
    /// one text form per identifier is accepted.
    pub fn from_base64url(s: &str) -> Result<Self, CrdtError> {
        ReplicaId::new(b64url_decode(s)?)
    }
}

/// Stable identifier of one independently replicated dataset (an absolute
/// IRI, research §4.1). Prevents applying a valid delta to the wrong dataset.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DatasetId(String);

impl DatasetId {
    /// Validates and wraps an absolute dataset IRI.
    pub fn new(iri: &str) -> Result<Self, CrdtError> {
        if iri.len() > MAX_DATASET_IRI_BYTES {
            return Err(CrdtError::Oversized {
                what: "dataset iri bytes",
                len: iri.len(),
                max: MAX_DATASET_IRI_BYTES,
            });
        }
        oxrdf::NamedNode::new(iri).map_err(|e| CrdtError::Invalid {
            what: "dataset id",
            reason: format!("not a valid absolute IRI: {e}"),
        })?;
        Ok(DatasetId(iri.to_owned()))
    }

    /// The dataset IRI text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One globally unique quad-addition (or envelope-identity) event:
/// `(replica, counter)` with a non-zero counter (`CRDT-DOT-1`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Dot {
    replica: ReplicaId,
    counter: u64,
}

impl Dot {
    /// Builds a dot; the counter must be non-zero.
    pub fn new(replica: ReplicaId, counter: u64) -> Result<Self, CrdtError> {
        if counter == 0 {
            return Err(CrdtError::Invalid {
                what: "dot counter",
                reason: "must be non-zero".into(),
            });
        }
        Ok(Dot { replica, counter })
    }

    /// The replica component.
    pub fn replica(&self) -> &ReplicaId {
        &self.replica
    }

    /// The counter component (always non-zero).
    pub fn counter(&self) -> u64 {
        self.counter
    }
}

/// An envelope identity `(origin replica, origin journal sequence)`.
///
/// Envelope identities form their own causal-summary namespace, separate from
/// data dots (proposal §"Journal frontier"): the same `clock`-plus-`cloud`
/// representation applies, so the type is shared with [`Dot`].
pub type EnvelopeId = Dot;

#[cfg(test)]
mod tests {
    use super::*;

    fn rid(bytes: &[u8]) -> ReplicaId {
        ReplicaId::new(bytes.to_vec()).expect("valid replica id")
    }

    #[test]
    fn replica_id_new_enforces_bounds() {
        assert!(ReplicaId::new(Vec::new()).is_err());
        assert!(ReplicaId::new(vec![0u8; MAX_REPLICA_ID_BYTES]).is_ok());
        assert!(ReplicaId::new(vec![0u8; MAX_REPLICA_ID_BYTES + 1]).is_err());
    }

    #[test]
    fn replica_id_as_bytes_round_trips() {
        assert_eq!(rid(b"peer-a").as_bytes(), b"peer-a");
    }

    #[test]
    fn replica_id_base64url_round_trips_unpadded() {
        let id = rid(b"peer-a");
        let text = id.to_base64url();
        assert_eq!(text, "cGVlci1h");
        assert!(!text.contains('='));
        assert_eq!(ReplicaId::from_base64url(&text).unwrap(), id);
    }

    #[test]
    fn replica_id_from_base64url_rejects_padding_and_junk() {
        assert!(ReplicaId::from_base64url("cGVlci1h=").is_err());
        assert!(ReplicaId::from_base64url("c GVl").is_err());
        assert!(ReplicaId::from_base64url("").is_err());
        // Length ≡ 1 (mod 4) is impossible for any byte string.
        assert!(ReplicaId::from_base64url("AAAAA").is_err());
        // Non-zero trailing bits: "cGVlci1i" ends ...1i; flipping the final
        // sextet's low bits produces a second text form of the same bytes,
        // which strict decoding must reject.
        assert!(ReplicaId::from_base64url("cGVlci1j").is_ok()); // canonical for other bytes
        assert!(ReplicaId::from_base64url("cB").is_err()); // 'B' = 1 ⇒ trailing bits set
    }

    #[test]
    fn replica_id_orders_by_raw_bytes() {
        // Raw-byte order and base64url-text order disagree here: 0xFB encodes
        // to "-w" (starts '-', ASCII 0x2D) while 0x00 encodes to "AA".
        let lo = rid(&[0x00]);
        let hi = rid(&[0xFB]);
        assert!(lo < hi);
        assert!(lo.to_base64url() > hi.to_base64url());
    }

    #[test]
    fn dataset_id_new_validates_iri_and_bound() {
        assert!(DatasetId::new("https://example.test/datasets/team").is_ok());
        assert!(DatasetId::new("not an iri").is_err());
        let long = format!("https://example.test/{}", "x".repeat(MAX_DATASET_IRI_BYTES));
        assert!(DatasetId::new(&long).is_err());
    }

    #[test]
    fn dataset_id_as_str_returns_the_iri() {
        let id = DatasetId::new("https://example.test/d").unwrap();
        assert_eq!(id.as_str(), "https://example.test/d");
    }

    #[test]
    fn dot_new_rejects_zero_counter() {
        assert!(Dot::new(rid(b"a"), 0).is_err());
        assert!(Dot::new(rid(b"a"), 1).is_ok());
    }

    #[test]
    fn dot_accessors_expose_components() {
        let d = Dot::new(rid(b"a"), 7).unwrap();
        assert_eq!(d.replica(), &rid(b"a"));
        assert_eq!(d.counter(), 7);
    }

    #[test]
    fn dot_orders_by_replica_bytes_then_counter() {
        let a1 = Dot::new(rid(b"a"), 1).unwrap();
        let a2 = Dot::new(rid(b"a"), 2).unwrap();
        let b1 = Dot::new(rid(b"b"), 1).unwrap();
        assert!(a1 < a2);
        assert!(a2 < b1);
    }
}
