//! (sq-lhcot.1) **Embedding provenance** for the `.spqv` v3 store: the identity of the
//! embedding pipeline a store was built with — model identifier, model/content version, similarity
//! **metric**, vector **normalization**, **dimension**, and **verbalization regime** — plus a
//! **mandatory compatibility check** so a query issued with an incompatible embedder is REJECTED
//! rather than silently returning meaningless neighbours.
//!
//! # The foot-gun this closes (review gap 1; spec estate `sq-rvgr2.4`)
//!
//! An embedding is a coordinate in one model's vector space. A query vector produced by a
//! *different* model, a different content revision of the same model, under a different metric, a
//! different normalization, or a different verbalization of the entity text, lands in a DIFFERENT
//! space — its cosine against the stored vectors is arithmetically well-defined but semantically
//! meaningless, and the search returns plausible-looking but WRONG neighbours with no error. The v2
//! `.spqv` container persisted only the dimension and a graph fingerprint (which catches a shifted
//! dictionary, not a shifted embedding space). This module records the embedding provenance in the
//! v3 header and makes an incompatible query a hard, descriptive error — the reproducibility
//! obligation revision 2 of the vector spec normatively requires.
//!
//! # What is recorded (the compatibility axes)
//!
//! | field | why it must match |
//! |---|---|
//! | [`model_id`](EmbeddingProvenance::model_id) | different model ⇒ different space |
//! | [`model_version`](EmbeddingProvenance::model_version) | a model revision can shift the space |
//! | [`content_version`](EmbeddingProvenance::content_version) | the graph→text revision (verbaliser template etc.) can change what was embedded |
//! | [`metric`](Metric) | cosine vs dot vs L2 rank differently |
//! | [`normalization`](Normalization) | an un-normalized query against L2-normalized stored vectors mis-ranks |
//! | dimension | already in the header; a mismatch is a hard error |
//! | [`verbalization`](EmbeddingProvenance::verbalization) | the regime that turned an entity into text |
//!
//! Each string field is a caller-supplied opaque token — this crate does **not** define or
//! privilege any encoder, model, or verbaliser; it records what the caller declares and rejects a
//! mismatch. Two provenances are **compatible** iff every recorded axis is equal (an ABSENT axis on
//! both sides is equal; a value-vs-absent pair is a mismatch — fail-closed).
//!
//! # The reserved extension area (KERN BOUNDARY — do not implement ahead of the profile freeze)
//!
//! [`EmbeddingProvenance`] carries a versioned, **opaque** [`reserved`](EmbeddingProvenance::reserved)
//! byte area for **extension fields: reserved pending a cross-implementation profile (#1746)**. It
//! is a length-prefixed opaque blob with **NO fields defined here** — no encoder-version-hash, no
//! codebook-hash, no `D` annotation semantics. Those await the #1746 profile freeze; defining them
//! now would privilege one encoder before the cross-implementation profile is agreed. The area
//! round-trips byte-for-byte (an implementation that does understand a future field reads it out of
//! the reserved bytes) and, by default, is empty. Its presence/contents do NOT affect
//! [`compatible_with`](EmbeddingProvenance::compatible_with): compatibility is decided only over the
//! defined axes above, so a store written by a future implementation that populated the reserved
//! area stays queryable by this one.
//!
//! # Legacy (v2 / v1) containers — fail-closed by default
//!
//! A v2 (or v1) store carries NO embedding provenance. A query that demands provenance compatibility
//! ([`VectorStore::check_provenance`](crate::store::VectorStore::check_provenance)) against such a
//! store REJECTS by default — the store's embedding pipeline is unverifiable, so we cannot certify
//! the query vector was produced compatibly. A caller that KNOWS the store is compatible can opt into
//! [`LegacyMode::Allow`](crate::store::LegacyMode) to bypass the check for a legacy store; the
//! default is [`LegacyMode::Reject`](crate::store::LegacyMode) (fail-closed).

use std::fmt;

/// The similarity **metric** an embedding store's vectors are ranked under. A query vector produced
/// for one metric mis-ranks against a store built for another (dot-product and cosine agree only on
/// L2-normalized vectors; L2 distance ranks inversely). Recorded so an incompatible query is
/// rejected. This crate does not *apply* the metric here — the searchers use cosine — the field
/// records the embedding pipeline's intended metric for the compatibility check.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Metric {
    /// Cosine similarity.
    Cosine,
    /// Dot product (inner product).
    Dot,
    /// Euclidean (L2) distance.
    Euclidean,
}

impl Metric {
    /// The on-disk tag byte (stable — never renumber; append new variants).
    pub fn tag(self) -> u8 {
        match self {
            Metric::Cosine => 0,
            Metric::Dot => 1,
            Metric::Euclidean => 2,
        }
    }

    /// Parse a tag byte back to a `Metric`. `None` for an unknown tag (a store written by a future
    /// version with a metric this build does not know — fail-closed: an unrecognised metric is
    /// treated as "cannot verify", not silently accepted).
    pub fn from_tag(tag: u8) -> Option<Metric> {
        match tag {
            0 => Some(Metric::Cosine),
            1 => Some(Metric::Dot),
            2 => Some(Metric::Euclidean),
            _ => None,
        }
    }

    /// A short human name for error messages.
    fn name(self) -> &'static str {
        match self {
            Metric::Cosine => "cosine",
            Metric::Dot => "dot",
            Metric::Euclidean => "euclidean",
        }
    }
}

impl fmt::Display for Metric {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The **normalization** applied to a store's vectors at build time. An un-normalized query vector
/// against L2-normalized stored vectors (or vice-versa) produces mis-ranked cosine scores, so the
/// normalization is a compatibility axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Normalization {
    /// No normalization applied (raw model output).
    None,
    /// L2 (unit-length) normalized.
    L2,
}

impl Normalization {
    /// The on-disk tag byte (stable — never renumber; append new variants).
    pub fn tag(self) -> u8 {
        match self {
            Normalization::None => 0,
            Normalization::L2 => 1,
        }
    }

    /// Parse a tag byte back. `None` for an unknown tag (fail-closed — see [`Metric::from_tag`]).
    pub fn from_tag(tag: u8) -> Option<Normalization> {
        match tag {
            0 => Some(Normalization::None),
            1 => Some(Normalization::L2),
            _ => None,
        }
    }

    /// A short human name for error messages.
    fn name(self) -> &'static str {
        match self {
            Normalization::None => "none",
            Normalization::L2 => "l2",
        }
    }
}

impl fmt::Display for Normalization {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// (sq-lhcot.1) The embedding provenance recorded in a `.spqv` v3 header: the identity of
/// the embedding pipeline the store was built with, used for the mandatory compatibility check. See
/// the module docs for the axes and the KERN-boundary reserved area.
///
/// The string axes are caller-supplied opaque tokens (this crate defines/privileges no encoder); the
/// [`metric`](Self::metric) / [`normalization`](Self::normalization) are typed enums. `dim` is NOT
/// stored here (it is already the store's header dimension and checked there); the compatibility
/// check folds the store dimension in.
// Intentionally NOT `Default`: the metric and normalization are load-bearing compatibility
// axes, and a default (e.g. cosine/L2) would silently privilege one embedding convention — a
// foot-gun for the mandatory-rejection contract. Callers specify them explicitly via `new`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbeddingProvenance {
    /// The model identifier (e.g. `"text-embedding-3-small"`). Opaque; a different id ⇒ a different
    /// space ⇒ incompatible.
    pub model_id: String,
    /// The model version / revision, when the caller distinguishes one. Empty ⇒ unspecified.
    pub model_version: String,
    /// The content (graph→text) revision — the verbaliser template / regime version that produced
    /// the embedded text. Empty ⇒ unspecified.
    pub content_version: String,
    /// The similarity metric the vectors are ranked under.
    pub metric: Metric,
    /// The normalization applied to the vectors at build time.
    pub normalization: Normalization,
    /// The verbalization regime name (e.g. `"label-only"`, `"entity-verbalized"`). Opaque; the
    /// regime that turned an entity into the embedded text. Empty ⇒ unspecified.
    pub verbalization: String,
    /// KERN BOUNDARY: a versioned, **opaque** reserved area for **extension fields:
    /// reserved pending a cross-implementation profile (#1746)**. NO fields are defined here — no
    /// encoder-version-hash, no codebook-hash, no `D` annotation. It round-trips byte-for-byte and,
    /// by default, is empty. It does NOT participate in [`compatible_with`](Self::compatible_with).
    pub reserved: Vec<u8>,
}

/// The format version of the serialized [`EmbeddingProvenance`] block. Bumped only if the DEFINED
/// axis layout changes; the opaque [`reserved`](EmbeddingProvenance::reserved) area exists precisely
/// so a future extension field does NOT need a block-format bump (it is written into the reserved
/// bytes).
pub const PROVENANCE_BLOCK_VERSION: u16 = 1;

/// The maximum length of any single serialized provenance block, as a defence against a corrupt
/// header claiming an absurd length. 64 KiB is far above any real provenance (a few short strings +
/// an empty reserved area) yet bounds the allocation.
pub const MAX_PROVENANCE_BLOCK_LEN: usize = 64 * 1024;

impl EmbeddingProvenance {
    /// A provenance with the given model id and typed axes, all string sub-axes empty and no reserved
    /// extension bytes. The most common constructor; set the other fields directly as needed.
    pub fn new(
        model_id: impl Into<String>,
        metric: Metric,
        normalization: Normalization,
    ) -> EmbeddingProvenance {
        EmbeddingProvenance {
            model_id: model_id.into(),
            model_version: String::new(),
            content_version: String::new(),
            metric,
            normalization,
            verbalization: String::new(),
            reserved: Vec::new(),
        }
    }

    /// Whether a query issued under `query` provenance is **compatible** with a store built under
    /// `self`. Compatible iff **every defined axis is equal**: model id, model version, content
    /// version, metric, normalization, and verbalization regime. The opaque
    /// [`reserved`](Self::reserved) area is deliberately EXCLUDED (KERN boundary — it carries no
    /// defined semantics yet), so a store written by a future implementation that populated it stays
    /// queryable. An absent (empty) string axis on BOTH sides is equal; a value-vs-absent pair is a
    /// mismatch (fail-closed — we do not assume an unspecified axis matches a specified one).
    ///
    /// Note the caller passes the store dimension separately to the store-level check; dimension is
    /// not an [`EmbeddingProvenance`] field.
    pub fn compatible_with(&self, query: &EmbeddingProvenance) -> Result<(), String> {
        let mut mismatches: Vec<String> = Vec::new();
        if self.model_id != query.model_id {
            mismatches.push(format!(
                "model id (store {:?}, query {:?})",
                self.model_id, query.model_id
            ));
        }
        if self.model_version != query.model_version {
            mismatches.push(format!(
                "model version (store {:?}, query {:?})",
                self.model_version, query.model_version
            ));
        }
        if self.content_version != query.content_version {
            mismatches.push(format!(
                "content version (store {:?}, query {:?})",
                self.content_version, query.content_version
            ));
        }
        if self.metric != query.metric {
            mismatches.push(format!(
                "metric (store {}, query {})",
                self.metric, query.metric
            ));
        }
        if self.normalization != query.normalization {
            mismatches.push(format!(
                "normalization (store {}, query {})",
                self.normalization, query.normalization
            ));
        }
        if self.verbalization != query.verbalization {
            mismatches.push(format!(
                "verbalization regime (store {:?}, query {:?})",
                self.verbalization, query.verbalization
            ));
        }
        if mismatches.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "embedding provenance mismatch — the query embedder is incompatible with this \
                 store: {}. Re-embed the query with the store's embedding pipeline, or rebuild the \
                 store.",
                mismatches.join("; ")
            ))
        }
    }

    /// (sq-tb9p0) **Expresses this provenance record as RDF** (spec assertion
    /// VG-PROV-5 of `site/specs/sparql-vector-genai.typ`): triples about `store` — the IRI the
    /// caller uses to name the vector store/index — in the [`crate::prov_vocab`] (`spqvp:`)
    /// vocabulary, so a dataset can describe its vector indexes. Emitted unconditionally:
    /// `spqvp:model`, `spqvp:metric`, `spqvp:normalization` (their short token names), and
    /// `spqvp:dimension` (the store header dimension, passed by the caller since it is not an
    /// [`EmbeddingProvenance`] field — an `xsd:integer` literal). Emitted only when non-empty
    /// (an absent axis is NOT asserted as an empty string): `spqvp:modelVersion`,
    /// `spqvp:contentVersion`, `spqvp:verbalization`. The opaque [`reserved`](Self::reserved)
    /// area has no defined semantics (KERN boundary, #1746) and is never expressed.
    pub fn to_rdf(&self, store: oxrdf::NamedNodeRef<'_>, dim: usize) -> Vec<oxrdf::Triple> {
        use oxrdf::{Literal, NamedNode, Triple};
        let pred = |iri: &str| NamedNode::new_unchecked(iri);
        let lit = |p: &str, v: &str| {
            Triple::new(store.into_owned(), pred(p), Literal::new_simple_literal(v))
        };
        let mut out = vec![
            lit(crate::prov_vocab::MODEL, &self.model_id),
            lit(crate::prov_vocab::METRIC, self.metric.name()),
            lit(crate::prov_vocab::NORMALIZATION, self.normalization.name()),
            Triple::new(
                store.into_owned(),
                pred(crate::prov_vocab::DIMENSION),
                Literal::new_typed_literal(dim.to_string(), oxrdf::vocab::xsd::INTEGER),
            ),
        ];
        if !self.model_version.is_empty() {
            out.push(lit(crate::prov_vocab::MODEL_VERSION, &self.model_version));
        }
        if !self.content_version.is_empty() {
            out.push(lit(crate::prov_vocab::CONTENT_VERSION, &self.content_version));
        }
        if !self.verbalization.is_empty() {
            out.push(lit(crate::prov_vocab::VERBALIZATION, &self.verbalization));
        }
        out
    }

    /// Serializes the provenance to a length-prefixed, self-describing byte block for the `.spqv` v3
    /// header. Layout (all fixed-width little-endian; strings are `u32`-length-prefixed UTF-8):
    ///
    /// ```text
    /// version        u16   = PROVENANCE_BLOCK_VERSION
    /// metric         u8    (Metric::tag)
    /// normalization  u8    (Normalization::tag)
    /// model_id       u32 len + bytes
    /// model_version  u32 len + bytes
    /// content_version u32 len + bytes
    /// verbalization  u32 len + bytes
    /// reserved       u32 len + bytes   (opaque — KERN boundary, empty by default)
    /// ```
    ///
    /// The caller length-prefixes the WHOLE block in the header (see `store.rs`), so this method's
    /// output is exactly the block bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&PROVENANCE_BLOCK_VERSION.to_le_bytes());
        out.push(self.metric.tag());
        out.push(self.normalization.tag());
        write_str(&mut out, &self.model_id);
        write_str(&mut out, &self.model_version);
        write_str(&mut out, &self.content_version);
        write_str(&mut out, &self.verbalization);
        // The opaque reserved (KERN) area — a length-prefixed blob, no defined fields.
        write_bytes(&mut out, &self.reserved);
        out
    }

    /// Parses a provenance block previously written by [`to_bytes`](Self::to_bytes). `bytes` is the
    /// exact block (the header already sliced it out by its length prefix). Fail-closed: a truncated
    /// block, a string length that overruns the block, an unknown metric/normalization tag, or a
    /// block-format version this build does not understand is a descriptive `Err` — never a panic or
    /// a silent partial parse. Trailing bytes after the reserved area are rejected (a malformed block).
    pub fn from_bytes(bytes: &[u8]) -> Result<EmbeddingProvenance, String> {
        let mut r = Reader { bytes, pos: 0 };
        let version = r.read_u16()?;
        if version != PROVENANCE_BLOCK_VERSION {
            return Err(format!(
                "unsupported embedding-provenance block version {} (this build writes/reads {})",
                version, PROVENANCE_BLOCK_VERSION
            ));
        }
        let metric_tag = r.read_u8()?;
        let metric = Metric::from_tag(metric_tag)
            .ok_or_else(|| format!("unknown embedding metric tag {}", metric_tag))?;
        let norm_tag = r.read_u8()?;
        let normalization = Normalization::from_tag(norm_tag)
            .ok_or_else(|| format!("unknown embedding normalization tag {}", norm_tag))?;
        let model_id = r.read_str()?;
        let model_version = r.read_str()?;
        let content_version = r.read_str()?;
        let verbalization = r.read_str()?;
        let reserved = r.read_bytes()?;
        if r.pos != r.bytes.len() {
            return Err(format!(
                "embedding-provenance block has {} trailing byte(s) after the reserved area",
                r.bytes.len() - r.pos
            ));
        }
        Ok(EmbeddingProvenance {
            model_id,
            model_version,
            content_version,
            metric,
            normalization,
            verbalization,
            reserved,
        })
    }
}

/// A one-line human summary for error messages / introspection.
impl fmt::Display for EmbeddingProvenance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "model={:?} model_version={:?} content_version={:?} metric={} normalization={} \
             verbalization={:?}",
            self.model_id,
            self.model_version,
            self.content_version,
            self.metric,
            self.normalization,
            self.verbalization
        )
    }
}

// ---- byte helpers (fixed-width little-endian, cross-machine stable) ----------------------------

fn write_str(out: &mut Vec<u8>, s: &str) {
    write_bytes(out, s.as_bytes());
}

fn write_bytes(out: &mut Vec<u8>, b: &[u8]) {
    out.extend_from_slice(&(b.len() as u32).to_le_bytes());
    out.extend_from_slice(b);
}

/// A bounds-checked forward reader over the provenance block. Every read validates the length before
/// slicing, so a malformed block is a descriptive `Err`, never an out-of-bounds panic.
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| "embedding-provenance block length overflow".to_string())?;
        if end > self.bytes.len() {
            return Err(format!(
                "truncated embedding-provenance block (need {} byte(s) at offset {}, have {})",
                n,
                self.pos,
                self.bytes.len() - self.pos
            ));
        }
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn read_u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn read_u16(&mut self) -> Result<u16, String> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn read_u32(&mut self) -> Result<u32, String> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn read_bytes(&mut self) -> Result<Vec<u8>, String> {
        let len = self.read_u32()? as usize;
        // Bound the declared length against a sane cap AND the remaining block before allocating.
        if len > MAX_PROVENANCE_BLOCK_LEN {
            return Err(format!(
                "embedding-provenance field length {} exceeds the {}-byte cap",
                len, MAX_PROVENANCE_BLOCK_LEN
            ));
        }
        Ok(self.take(len)?.to_vec())
    }

    fn read_str(&mut self) -> Result<String, String> {
        let raw = self.read_bytes()?;
        String::from_utf8(raw)
            .map_err(|_| "embedding-provenance string field is not valid UTF-8".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full() -> EmbeddingProvenance {
        EmbeddingProvenance {
            model_id: "text-embedding-3-small".into(),
            model_version: "2024-01".into(),
            content_version: "verb-v2".into(),
            metric: Metric::Cosine,
            normalization: Normalization::L2,
            verbalization: "entity-verbalized".into(),
            reserved: vec![],
        }
    }

    #[test]
    fn round_trips_through_bytes() {
        let p = full();
        let bytes = p.to_bytes();
        let back = EmbeddingProvenance::from_bytes(&bytes).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn round_trips_with_reserved_extension_bytes() {
        // KERN boundary: the opaque reserved area round-trips byte-for-byte even though no
        // field is defined over it (a future implementation's bytes survive a read/write here).
        let mut p = full();
        p.reserved = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01];
        let back = EmbeddingProvenance::from_bytes(&p.to_bytes()).unwrap();
        assert_eq!(p.reserved, back.reserved);
        assert_eq!(p, back);
    }

    #[test]
    fn reserved_area_does_not_affect_compatibility() {
        // KERN boundary: compatibility is decided ONLY over the defined axes. Two
        // provenances that agree on every axis but differ in the opaque reserved area are compatible.
        let a = full();
        let mut b = full();
        b.reserved = vec![1, 2, 3];
        assert!(a.compatible_with(&b).is_ok(), "reserved area must not gate compatibility");
        assert!(b.compatible_with(&a).is_ok());
    }

    #[test]
    fn identical_provenance_is_compatible() {
        assert!(full().compatible_with(&full()).is_ok());
    }

    #[test]
    fn empty_axes_are_compatible_only_with_empty() {
        // An absent axis matches an absent axis; a value-vs-absent pair is a mismatch (fail-closed).
        let bare = EmbeddingProvenance::new("m", Metric::Cosine, Normalization::L2);
        assert!(bare.compatible_with(&bare).is_ok());
        let mut versioned = bare.clone();
        versioned.model_version = "v1".into();
        assert!(bare.compatible_with(&versioned).is_err(), "value vs absent must mismatch");
    }

    #[test]
    fn from_bytes_rejects_unknown_metric_tag() {
        let mut bytes = full().to_bytes();
        // The metric tag is at offset 2 (after the u16 version). 200 is not a known metric.
        bytes[2] = 200;
        let err = EmbeddingProvenance::from_bytes(&bytes).unwrap_err();
        assert!(err.contains("metric tag"), "got: {}", err);
    }

    #[test]
    fn from_bytes_rejects_unknown_version() {
        let mut bytes = full().to_bytes();
        bytes[0] = 0xFF;
        bytes[1] = 0xFF;
        let err = EmbeddingProvenance::from_bytes(&bytes).unwrap_err();
        assert!(err.contains("version"), "got: {}", err);
    }

    #[test]
    fn from_bytes_rejects_truncation() {
        let bytes = full().to_bytes();
        let err = EmbeddingProvenance::from_bytes(&bytes[..bytes.len() - 3]).unwrap_err();
        assert!(err.contains("trunc") || err.contains("trailing"), "got: {}", err);
    }

    #[test]
    fn from_bytes_rejects_trailing_garbage() {
        let mut bytes = full().to_bytes();
        bytes.push(0x00);
        let err = EmbeddingProvenance::from_bytes(&bytes).unwrap_err();
        assert!(err.contains("trailing"), "got: {}", err);
    }

    #[test]
    fn from_bytes_rejects_oversized_field_length() {
        // A corrupt u32 length far above the cap must be rejected before any allocation.
        let mut bytes = full().to_bytes();
        // model_id length prefix starts at offset 4 (version u16 + metric u8 + norm u8).
        bytes[4..8].copy_from_slice(&(u32::MAX).to_le_bytes());
        let err = EmbeddingProvenance::from_bytes(&bytes).unwrap_err();
        assert!(err.contains("cap") || err.contains("trunc"), "got: {}", err);
    }

    #[test]
    fn to_rdf_expresses_the_record_and_omits_absent_axes() {
        // (sq-tb9p0) VG-PROV-5: the full record (all axes set) emits every term; an
        // absent string axis is OMITTED, never asserted as an empty-string literal.
        use oxrdf::NamedNodeRef;
        let subject = NamedNodeRef::new("http://ex/store").unwrap();
        let triples = full().to_rdf(subject, 384);
        let nt: Vec<String> = triples.iter().map(|t| t.to_string()).collect();
        assert_eq!(triples.len(), 7, "all axes set => 7 triples: {nt:?}");
        for t in &triples {
            assert_eq!(t.subject.to_string(), "<http://ex/store>");
        }
        let has = |p: &str, o: &str| {
            nt.iter().any(|t| t.contains(&format!("<{}>", p)) && t.contains(o))
        };
        assert!(has(crate::prov_vocab::MODEL, "\"text-embedding-3-small\""));
        assert!(has(crate::prov_vocab::MODEL_VERSION, "\"2024-01\""));
        assert!(has(crate::prov_vocab::CONTENT_VERSION, "\"verb-v2\""));
        assert!(has(crate::prov_vocab::METRIC, "\"cosine\""));
        assert!(has(crate::prov_vocab::NORMALIZATION, "\"l2\""));
        assert!(has(crate::prov_vocab::VERBALIZATION, "\"entity-verbalized\""));
        assert!(has(crate::prov_vocab::DIMENSION, "\"384\""));

        // The bare record (empty version/verbalization axes) emits only the mandatory four.
        let bare = EmbeddingProvenance::new("m", Metric::Dot, Normalization::None);
        let triples = bare.to_rdf(subject, 2);
        assert_eq!(triples.len(), 4, "absent axes must be omitted");
        let nt: Vec<String> = triples.iter().map(|t| t.to_string()).collect();
        assert!(!nt.iter().any(|t| t.contains("\"\"")), "no empty-string assertions: {nt:?}");
        let has_bare = |p: &str, o: &str| {
            nt.iter().any(|t| t.contains(&format!("<{}>", p)) && t.contains(o))
        };
        assert!(has_bare(crate::prov_vocab::MODEL, "\"m\""));
        assert!(has_bare(crate::prov_vocab::METRIC, "\"dot\""));
        assert!(has_bare(crate::prov_vocab::NORMALIZATION, "\"none\""));
        assert!(has_bare(crate::prov_vocab::DIMENSION, "\"2\""));
    }

    #[test]
    fn metric_and_normalization_tags_round_trip() {
        for m in [Metric::Cosine, Metric::Dot, Metric::Euclidean] {
            assert_eq!(Metric::from_tag(m.tag()), Some(m));
        }
        for n in [Normalization::None, Normalization::L2] {
            assert_eq!(Normalization::from_tag(n.tag()), Some(n));
        }
        assert_eq!(Metric::from_tag(99), None);
        assert_eq!(Normalization::from_tag(99), None);
    }

    #[test]
    fn distinct_axes_each_cause_a_mismatch() {
        let base = full();
        let mut wrong_model = base.clone();
        wrong_model.model_id = "other-model".into();
        assert!(base.compatible_with(&wrong_model).is_err());

        let mut wrong_metric = base.clone();
        wrong_metric.metric = Metric::Dot;
        assert!(base.compatible_with(&wrong_metric).is_err());

        let mut wrong_norm = base.clone();
        wrong_norm.normalization = Normalization::None;
        assert!(base.compatible_with(&wrong_norm).is_err());

        let mut wrong_verb = base.clone();
        wrong_verb.verbalization = "label-only".into();
        assert!(base.compatible_with(&wrong_verb).is_err());
    }
}
