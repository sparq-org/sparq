//! **Opt-in (`concept` feature), OFF by default.** `urn:concept:` record
//! verification — the *recompute-and-byte-compare* half of the ingestion guard
//! for content-addressed concept records (GitHub #1746, definition thread
//! #1683).
//!
//! # What this module is, precisely
//!
//! A concept record is named by a URN of the form
//! `urn:concept:<multibase-multihash>`. This module implements the two pieces
//! that are pinned by *published* specifications and nothing else:
//!
//! 1. the **[multibase]/[multihash] envelope** — decode the URN's
//!    name-specific string into `(multibase alphabet, multicodec hash code,
//!    declared digest length, declared digest bytes)`, and re-encode it; and
//! 2. the **guard** — recompute the RDFC-1.0 digest of a *caller-supplied*
//!    set of quads under the hash the URN declares, rebuild the full multihash,
//!    and compare it to the declared multihash byte-for-byte in constant time.
//!
//! [multibase]: https://github.com/multiformats/multibase
//! [multihash]: https://github.com/multiformats/multihash
//!
//! # What this module deliberately does NOT decide
//!
//! **It does not define which quads constitute the record.** The caller passes
//! the quads; the scope-extraction rule (whole named graph, node neighbourhood,
//! SCC, …) is owned by the concept-hash definition (#1683) and its profile
//! freeze (#1746), neither of which has been vendored into this repository.
//! Guessing that rule here would privilege sparq's guess as the de facto
//! profile, which is exactly what the KERN-boundary convention exists to
//! prevent. See `research/genai-urn-concept-verifier-design.md` — §3 of that
//! record demonstrates, with counterexamples produced by this crate's own
//! [`issue_quads`](crate::issue_quads), that a *node-level* scope hash is **not**
//! an RDFC-1.0 property of the surrounding graph, so the choice genuinely
//! matters and is not sparq's to make.
//!
//! Two further profile parameters this implementation fixes by *convention*,
//! documented so a disagreement with the frozen profile is a visible
//! configuration difference rather than a silent one:
//!
//! - **The hashed pre-image** is the exact RDFC-1.0 canonical N-Quads document
//!   of the supplied quads, every byte included — the trailing newline of the
//!   final line among them (see [`digest_quads_with`]).
//! - **The multihash code selects the outer digest only.** Canonicalization
//!   itself retains RDFC-1.0's spec-default SHA-256 hash profile, so a
//!   `sha2-512`-coded URN means "SHA-512 over the SHA-256-profile canonical
//!   document". A frozen profile that ties the two together would need
//!   [`canonicalize_quads_with`](crate::canonicalize_quads_with) plus an
//!   explicit outer digest instead.
//!
//! An **empty** quad set is rejected ([`ConceptError::EmptyRecord`]) rather
//! than hashed as the empty document: fail-closed is the safer reading of an
//! open question, and it stops an accidentally-empty ingest from being compared
//! at all.
//!
//! # Independence — what the second implementation buys, and what it does not
//!
//! Recomputing the digest at the ingestion boundary catches defects and
//! tampering **on the producer side**: sparq re-derives the declared digest with
//! its own code path rather than trusting the name it was handed. It does *not*
//! provide independence from RDFC-1.0 itself — `sparq-canon` delegates the
//! canonicalization algorithm to the third-party `rdf-canon` crate, so if a
//! producer is on the same lineage an algorithm-level bug there is reproduced
//! identically on both sides and cancels out. Phrasings such as
//! "double-implementation verified" or "independently proven" are therefore
//! **not** accurate for this surface.
//!
//! # Fail-closed contract
//!
//! Every one of the following is an `Err` with nothing accepted: a string that
//! is not a `urn:concept:` URN; an unknown multibase prefix; a name-specific
//! string longer than any well-formed multihash could encode to — checked
//! *before* the body is decoded at all, so a hostile name cannot buy decoder
//! work (`MAX_MULTIBASE_BODY_LEN`, in this module's source); a character
//! outside the declared alphabet or non-canonical padding; a malformed,
//! non-minimal, or trailing-byte multihash; a declared length that disagrees
//! with the digest that follows it or with the natural output size of the
//! declared code; a hash code this build cannot compute; an empty record;
//! canonicalization failure (including RDF 1.2 triple terms, which the standard
//! paths reject with [`CanonError::TripleTerm`]); and of course a digest
//! mismatch. Unknown never means accepted.
//!
//! ```
//! use sparq_canon::concept::{concept_urn, verify_concept_urn, ConceptHash, Multibase};
//! # use oxrdf::{Quad, NamedNode, NamedOrBlankNode, Term, Literal, GraphName};
//! let record = vec![Quad::new(
//!     NamedOrBlankNode::NamedNode(NamedNode::new("http://ex/c").unwrap()),
//!     NamedNode::new("http://ex/p").unwrap(),
//!     Term::Literal(Literal::new_simple_literal("v")),
//!     GraphName::DefaultGraph,
//! )];
//! let urn = concept_urn(&record, ConceptHash::Sha256, Multibase::Base58Btc).unwrap();
//! assert!(urn.starts_with("urn:concept:z"));
//! verify_concept_urn(&urn, &record).unwrap();
//! ```

use oxrdf::Quad;

use crate::{digest_quads_with, CanonError};

/// The URN prefix a concept record is named by. Compared case-insensitively:
/// RFC 8141 makes the scheme and the namespace identifier case-insensitive,
/// while the name-specific string (the multibase text) is case-sensitive.
const URN_PREFIX: &str = "urn:concept:";

/// The largest multihash this build will decode: a code varint and a length
/// varint at the multiformats 9-byte cap (`read_uvarint`) plus a 64-byte
/// SHA-512 digest, the widest digest [`ConceptHash`] can recompute. Every
/// multihash a *supported* code can carry is far smaller (`sha2-512` is
/// `0x13 || 64 || 64 bytes` = 66 bytes); the headroom is for the unsupported
/// codes [`ConceptUrn::parse`] still accepts structurally before
/// [`verify_concept`] refuses them.
const MAX_MULTIHASH_BYTES: usize = 9 + 9 + 64;

/// The longest multibase body (the text *after* the prefix) any alphabet in
/// this build will look at, and the cheap fail-closed bound on decoder work.
///
/// base16 is the least dense alphabet here at exactly two characters per byte,
/// so two characters per `MAX_MULTIHASH_BYTES` bounds every alphabet at once —
/// 164, against 132 for the longest name a supported code can produce (SHA-512
/// written in base16).
///
/// The bound matters because base58btc has no byte-aligned digit boundary: its
/// decoder carries a big-integer multiply across the whole output for every
/// character (and grows that output with `insert(0, …)`), so it is quadratic in
/// the body length. Checking the length *first* holds a remotely supplied
/// `urn:concept:` name — the expected threat surface, since this parser guards
/// received records before they are indexed — to a bounded, tiny amount of work
/// whatever it contains. Rejecting only once the decoded bytes turn out not to
/// be a well-formed multihash would come after the cost, not instead of it.
const MAX_MULTIBASE_BODY_LEN: usize = 2 * MAX_MULTIHASH_BYTES;

/// A [multibase] alphabet this build can decode and encode.
///
/// Deliberately a small set — every prefix outside it fails closed with
/// [`ConceptError::UnknownMultibase`] rather than being guessed at.
///
/// [multibase]: https://github.com/multiformats/multibase
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Multibase {
    /// `f` — base16 (hex), lower case.
    Base16Lower,
    /// `F` — base16 (hex), upper case.
    Base16Upper,
    /// `b` — RFC 4648 base32, lower case, no padding.
    Base32Lower,
    /// `B` — RFC 4648 base32, upper case, no padding.
    Base32Upper,
    /// `z` — base58btc (the Bitcoin alphabet).
    Base58Btc,
    /// `u` — RFC 4648 base64url, no padding.
    Base64Url,
}

impl Multibase {
    /// The single-character multibase prefix for this alphabet.
    pub fn prefix(self) -> char {
        match self {
            Multibase::Base16Lower => 'f',
            Multibase::Base16Upper => 'F',
            Multibase::Base32Lower => 'b',
            Multibase::Base32Upper => 'B',
            Multibase::Base58Btc => 'z',
            Multibase::Base64Url => 'u',
        }
    }

    /// The alphabet a multibase prefix selects, or `None` if this build does
    /// not implement it (which callers must treat as a rejection, never as a
    /// pass-through).
    pub fn from_prefix(c: char) -> Option<Multibase> {
        match c {
            'f' => Some(Multibase::Base16Lower),
            'F' => Some(Multibase::Base16Upper),
            'b' => Some(Multibase::Base32Lower),
            'B' => Some(Multibase::Base32Upper),
            'z' => Some(Multibase::Base58Btc),
            'u' => Some(Multibase::Base64Url),
            _ => None,
        }
    }

    /// Decodes the body of a multibase string (the text *after* the prefix).
    ///
    /// The length is bounded *before* decoding — see `MAX_MULTIBASE_BODY_LEN`
    /// for why that ordering is the security-relevant part.
    fn decode(self, body: &str) -> Result<Vec<u8>, ConceptError> {
        // Byte length, not character count: it is an O(1) upper bound on the
        // character count, and any body with a non-ASCII character is outside
        // every alphabet here and rejected regardless.
        if body.len() > MAX_MULTIBASE_BODY_LEN {
            return Err(ConceptError::Multibase(format!(
                "body is {} bytes, longer than the {}-byte maximum any well-formed multihash \
                 encodes to; rejected before decoding",
                body.len(),
                MAX_MULTIBASE_BODY_LEN
            )));
        }
        match self {
            Multibase::Base16Lower => base16_decode(body, false),
            Multibase::Base16Upper => base16_decode(body, true),
            Multibase::Base32Lower => base32_decode(body, false),
            Multibase::Base32Upper => base32_decode(body, true),
            Multibase::Base58Btc => base58btc_decode(body),
            Multibase::Base64Url => base64url_decode(body),
        }
    }

    /// Encodes bytes as a full multibase string (prefix included).
    fn encode(self, bytes: &[u8]) -> String {
        let mut s = String::new();
        s.push(self.prefix());
        match self {
            Multibase::Base16Lower => base16_encode(bytes, false, &mut s),
            Multibase::Base16Upper => base16_encode(bytes, true, &mut s),
            Multibase::Base32Lower => base32_encode(bytes, false, &mut s),
            Multibase::Base32Upper => base32_encode(bytes, true, &mut s),
            Multibase::Base58Btc => base58btc_encode(bytes, &mut s),
            Multibase::Base64Url => base64url_encode(bytes, &mut s),
        }
        s
    }
}

/// A digest algorithm this build can recompute, identified by its [multicodec]
/// code as it appears in a multihash.
///
/// [multicodec]: https://github.com/multiformats/multicodec
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ConceptHash {
    /// `sha2-256`, multicodec `0x12`, 32-byte digest.
    Sha256,
    /// `sha2-512`, multicodec `0x13`, 64-byte digest.
    Sha512,
    /// `sha2-384`, multicodec `0x20`, 48-byte digest.
    Sha384,
}

impl ConceptHash {
    /// The multicodec code written into the multihash prefix.
    pub fn code(self) -> u64 {
        match self {
            ConceptHash::Sha256 => 0x12,
            ConceptHash::Sha512 => 0x13,
            ConceptHash::Sha384 => 0x20,
        }
    }

    /// The natural (untruncated) digest length in bytes. A multihash whose
    /// declared length differs is rejected: truncated digests are not accepted.
    pub fn digest_len(self) -> usize {
        match self {
            ConceptHash::Sha256 => 32,
            ConceptHash::Sha512 => 64,
            ConceptHash::Sha384 => 48,
        }
    }

    /// The algorithm a multicodec code selects, or `None` if this build cannot
    /// compute it — which [`verify_concept_urn`] turns into
    /// [`ConceptError::UnsupportedHash`], never into acceptance.
    pub fn from_code(code: u64) -> Option<ConceptHash> {
        match code {
            0x12 => Some(ConceptHash::Sha256),
            0x13 => Some(ConceptHash::Sha512),
            0x20 => Some(ConceptHash::Sha384),
            _ => None,
        }
    }

    /// Digests a record's canonical N-Quads document under this algorithm.
    fn digest_record(self, record: &[Quad]) -> Result<Vec<u8>, CanonError> {
        match self {
            ConceptHash::Sha256 => digest_quads_with::<sha2::Sha256>(record),
            ConceptHash::Sha512 => digest_quads_with::<sha2::Sha512>(record),
            ConceptHash::Sha384 => digest_quads_with::<sha2::Sha384>(record),
        }
    }
}

/// Why a `urn:concept:` record was **not** accepted.
///
/// `#[non_exhaustive]`: the frozen profile (#1746) may add rejection reasons,
/// so downstream `match`es keep a wildcard arm.
#[derive(Debug)]
#[non_exhaustive]
pub enum ConceptError {
    /// The string is not a `urn:concept:` URN (wrong scheme/NID, or nothing
    /// after the prefix).
    NotConceptUrn,
    /// The multibase prefix character is not one this build implements.
    UnknownMultibase(char),
    /// The multibase body is longer than any well-formed multihash encodes to
    /// (rejected before it is decoded — see the [module docs](self)), or is not
    /// valid text in its declared alphabet (bad character, or non-canonical
    /// trailing bits).
    Multibase(String),
    /// The decoded bytes are not a well-formed multihash (truncated, a
    /// non-minimal varint, a declared length that disagrees with the digest
    /// that follows, or trailing bytes after the digest).
    Multihash(String),
    /// The multihash declares a hash code this build cannot recompute.
    UnsupportedHash(u64),
    /// The record carries no quads. Fail-closed rather than hashing the empty
    /// canonical document — see the [module docs](self).
    EmptyRecord,
    /// The recomputed multihash differs from the one the URN declares. The
    /// record must not be indexed.
    Mismatch,
    /// Canonicalizing the record failed (including [`CanonError::TripleTerm`]
    /// for RDF 1.2 triple terms, which the standard paths reject).
    Canon(CanonError),
}

impl From<CanonError> for ConceptError {
    fn from(e: CanonError) -> Self {
        ConceptError::Canon(e)
    }
}

impl std::fmt::Display for ConceptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConceptError::NotConceptUrn => {
                write!(f, "not a `{}<multibase-multihash>` URN", URN_PREFIX)
            }
            ConceptError::UnknownMultibase(c) => write!(
                f,
                "unknown multibase prefix {:?}; failing closed rather than guessing the alphabet",
                c
            ),
            ConceptError::Multibase(e) => write!(f, "multibase decode failed: {}", e),
            ConceptError::Multihash(e) => write!(f, "malformed multihash: {}", e),
            ConceptError::UnsupportedHash(code) => write!(
                f,
                "multihash declares hash code {:#x}, which this build cannot recompute",
                code
            ),
            ConceptError::EmptyRecord => write!(
                f,
                "concept record is empty; failing closed rather than hashing the empty \
                 canonical document"
            ),
            ConceptError::Mismatch => write!(
                f,
                "recomputed concept multihash does not match the one declared by the URN; \
                 the record must not be indexed"
            ),
            ConceptError::Canon(e) => write!(f, "canonicalization failed: {}", e),
        }
    }
}

impl std::error::Error for ConceptError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ConceptError::Canon(e) => Some(e),
            _ => None,
        }
    }
}

/// A parsed `urn:concept:` name: the multibase alphabet it was written in and
/// the multihash it declares.
///
/// Parsing validates the envelope only. It says nothing about whether any
/// record matches — that is [`verify_concept_urn`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConceptUrn {
    multibase: Multibase,
    /// The full multihash: `varint(code) || varint(len) || digest`.
    multihash: Vec<u8>,
    code: u64,
    digest_offset: usize,
}

impl ConceptUrn {
    /// Parses `urn:concept:<multibase-multihash>`, failing closed on every
    /// malformed or unrecognised part (see the [module docs](self)).
    ///
    /// ```
    /// use sparq_canon::concept::ConceptUrn;
    /// // 0x12 0x20 || 32 zero bytes, base16-lower.
    /// let urn = format!("urn:concept:f1220{}", "00".repeat(32));
    /// let parsed = ConceptUrn::parse(&urn).unwrap();
    /// assert_eq!(parsed.hash_code(), 0x12);
    /// assert_eq!(parsed.digest().len(), 32);
    /// ```
    pub fn parse(urn: &str) -> Result<ConceptUrn, ConceptError> {
        if urn.len() <= URN_PREFIX.len()
            || !urn.is_char_boundary(URN_PREFIX.len())
            || !urn[..URN_PREFIX.len()].eq_ignore_ascii_case(URN_PREFIX)
        {
            return Err(ConceptError::NotConceptUrn);
        }
        let nss = &urn[URN_PREFIX.len()..];
        let mut chars = nss.chars();
        // `nss` is non-empty (length checked above), so this cannot fail.
        let prefix = chars.next().ok_or(ConceptError::NotConceptUrn)?;
        let multibase =
            Multibase::from_prefix(prefix).ok_or(ConceptError::UnknownMultibase(prefix))?;
        let multihash = multibase.decode(chars.as_str())?;
        let (code, digest_offset) = parse_multihash(&multihash)?;
        Ok(ConceptUrn {
            multibase,
            multihash,
            code,
            digest_offset,
        })
    }

    /// The multibase alphabet the URN was written in.
    pub fn multibase(&self) -> Multibase {
        self.multibase
    }

    /// The full multihash bytes — `varint(code) || varint(len) || digest`.
    /// This, not the bare digest, is what [`verify_concept_urn`] compares, so a
    /// record cannot declare a weaker code than the one actually used.
    pub fn multihash(&self) -> &[u8] {
        &self.multihash
    }

    /// The declared multicodec hash code. A caller that must pin one algorithm
    /// (rather than accept any this build supports) checks this before
    /// verifying.
    pub fn hash_code(&self) -> u64 {
        self.code
    }

    /// The declared digest bytes (the multihash with its prefix stripped).
    pub fn digest(&self) -> &[u8] {
        &self.multihash[self.digest_offset..]
    }

    /// Re-encodes this URN in its original multibase alphabet. Round-trips
    /// byte-exactly for any URN [`parse`](Self::parse) accepted.
    pub fn to_urn(&self) -> String {
        let mut s = String::from(URN_PREFIX);
        s.push_str(&self.multibase.encode(&self.multihash));
        s
    }
}

impl std::fmt::Display for ConceptUrn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_urn())
    }
}

/// Recomputes the multihash of a concept record: the RDFC-1.0 canonical
/// N-Quads document of `record`, digested under `hash`, wrapped in a multihash
/// envelope.
///
/// The caller decides which quads make up `record` — see the
/// [module docs](self) on what this module does not decide.
///
/// ```
/// use sparq_canon::concept::{concept_multihash, ConceptHash};
/// # use oxrdf::{Quad, NamedNode, NamedOrBlankNode, Term, Literal, GraphName};
/// let record = vec![Quad::new(
///     NamedOrBlankNode::NamedNode(NamedNode::new("http://ex/c").unwrap()),
///     NamedNode::new("http://ex/p").unwrap(),
///     Term::Literal(Literal::new_simple_literal("v")),
///     GraphName::DefaultGraph,
/// )];
/// let mh = concept_multihash(&record, ConceptHash::Sha256).unwrap();
/// assert_eq!(mh[0], 0x12);
/// assert_eq!(mh[1], 32);
/// assert_eq!(mh.len(), 34);
/// ```
pub fn concept_multihash(record: &[Quad], hash: ConceptHash) -> Result<Vec<u8>, ConceptError> {
    if record.is_empty() {
        return Err(ConceptError::EmptyRecord);
    }
    let digest = hash.digest_record(record)?;
    let mut out = Vec::with_capacity(digest.len() + 4);
    write_uvarint(hash.code(), &mut out);
    write_uvarint(digest.len() as u64, &mut out);
    out.extend_from_slice(&digest);
    Ok(out)
}

/// Builds the `urn:concept:` name for a record — the producing side of
/// [`verify_concept_urn`].
///
/// Provided so a caller can mint names with the same envelope it verifies
/// against; it does **not** make sparq the authority on the record's scope
/// (see the [module docs](self)).
pub fn concept_urn(
    record: &[Quad],
    hash: ConceptHash,
    base: Multibase,
) -> Result<String, ConceptError> {
    let multihash = concept_multihash(record, hash)?;
    let mut s = String::from(URN_PREFIX);
    s.push_str(&base.encode(&multihash));
    Ok(s)
}

/// **The ingestion guard.** Recomputes the concept multihash from `record` and
/// compares it, in constant time and including the multihash prefix, with the
/// one `urn` declares.
///
/// `Ok(())` means and only means: the supplied quads canonicalize to a document
/// whose digest, under the algorithm the URN itself declares, reproduces the
/// declared multihash byte-for-byte. Any other outcome is an `Err` and the
/// record **must not be indexed** — this check belongs *before* indexing, not
/// as a warning after it.
///
/// The comparison is over the full multihash, so a record cannot pass by
/// declaring a different (or weaker) hash code than the one that produced its
/// digest. Comparison time does not depend on how many bytes match; the length
/// is not hidden, which is not secret here.
///
/// ```
/// use sparq_canon::concept::{concept_urn, verify_concept_urn, ConceptHash, Multibase};
/// # use oxrdf::{Quad, NamedNode, NamedOrBlankNode, Term, Literal, GraphName};
/// # let q = |o: &str| Quad::new(
/// #     NamedOrBlankNode::NamedNode(NamedNode::new("http://ex/c").unwrap()),
/// #     NamedNode::new("http://ex/p").unwrap(),
/// #     Term::Literal(Literal::new_simple_literal(o)),
/// #     GraphName::DefaultGraph,
/// # );
/// let record = vec![q("v")];
/// let urn = concept_urn(&record, ConceptHash::Sha256, Multibase::Base32Lower).unwrap();
/// assert!(verify_concept_urn(&urn, &record).is_ok());
/// // A different record under the same name is rejected.
/// assert!(verify_concept_urn(&urn, &[q("tampered")]).is_err());
/// ```
pub fn verify_concept_urn(urn: &str, record: &[Quad]) -> Result<(), ConceptError> {
    let parsed = ConceptUrn::parse(urn)?;
    verify_concept(&parsed, record)
}

/// [`verify_concept_urn`] for an already-[parsed](ConceptUrn::parse) name, so a
/// caller that pins [`ConceptUrn::hash_code`] first does not parse twice.
pub fn verify_concept(urn: &ConceptUrn, record: &[Quad]) -> Result<(), ConceptError> {
    let hash = ConceptHash::from_code(urn.code).ok_or(ConceptError::UnsupportedHash(urn.code))?;
    let recomputed = concept_multihash(record, hash)?;
    if ct_eq(&recomputed, &urn.multihash) {
        Ok(())
    } else {
        Err(ConceptError::Mismatch)
    }
}

// ---------------------------------------------------------------------------
// Multihash envelope.
// ---------------------------------------------------------------------------

/// Validates `varint(code) || varint(len) || digest`, returning
/// `(code, digest offset)`. Rejects trailing bytes and a declared length that
/// disagrees either with the bytes present or with the code's natural size.
fn parse_multihash(bytes: &[u8]) -> Result<(u64, usize), ConceptError> {
    let (code, n1) = read_uvarint(bytes)?;
    let (len, n2) = read_uvarint(&bytes[n1..])?;
    let offset = n1 + n2;
    let declared = usize::try_from(len)
        .map_err(|_| ConceptError::Multihash(format!("declared digest length {} is absurd", len)))?;
    let actual = bytes.len() - offset;
    if actual != declared {
        return Err(ConceptError::Multihash(format!(
            "declares a {}-byte digest but carries {} bytes",
            declared, actual
        )));
    }
    if declared == 0 {
        return Err(ConceptError::Multihash("zero-length digest".to_string()));
    }
    if let Some(h) = ConceptHash::from_code(code) {
        if declared != h.digest_len() {
            return Err(ConceptError::Multihash(format!(
                "hash code {:#x} produces {} bytes but the multihash declares {}",
                code,
                h.digest_len(),
                declared
            )));
        }
    }
    Ok((code, offset))
}

/// Reads a minimally-encoded unsigned LEB128 varint (multiformats caps these at
/// 9 bytes / 63 bits).
fn read_uvarint(bytes: &[u8]) -> Result<(u64, usize), ConceptError> {
    let mut value: u64 = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if i >= 9 {
            return Err(ConceptError::Multihash(
                "varint longer than the 9-byte multiformats limit".to_string(),
            ));
        }
        value |= u64::from(b & 0x7f) << (7 * i);
        if b & 0x80 == 0 {
            if i > 0 && b == 0 {
                return Err(ConceptError::Multihash(
                    "non-minimal varint encoding".to_string(),
                ));
            }
            return Ok((value, i + 1));
        }
    }
    Err(ConceptError::Multihash(
        "varint truncated before its final byte".to_string(),
    ))
}

fn write_uvarint(mut value: u64, out: &mut Vec<u8>) {
    while value >= 0x80 {
        out.push((value as u8) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

/// Length-checked, data-independent byte comparison. The digest is an integrity
/// claim over possibly attacker-supplied input, so the loop must not short
/// circuit on the first differing byte.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

// ---------------------------------------------------------------------------
// Multibase alphabets. Hand-rolled (as `sparq-trust` does for status lists)
// rather than pulled in as a dependency: this crate's leanness is deliberate,
// and the four alphabets below are a few dozen lines each.
// ---------------------------------------------------------------------------

const BASE58BTC: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
const BASE32_LOWER: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
const BASE32_UPPER: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
const BASE64URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn bad_char(c: char, alphabet: &str) -> ConceptError {
    ConceptError::Multibase(format!("character {:?} is not valid {}", c, alphabet))
}

fn base16_decode(s: &str, upper: bool) -> Result<Vec<u8>, ConceptError> {
    let name = if upper { "base16 (upper)" } else { "base16" };
    if !s.len().is_multiple_of(2) {
        return Err(ConceptError::Multibase(
            "base16 body has an odd number of digits".to_string(),
        ));
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks(2) {
        let mut v: u8 = 0;
        for &b in pair {
            let d = match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' if !upper => b - b'a' + 10,
                b'A'..=b'F' if upper => b - b'A' + 10,
                _ => return Err(bad_char(b as char, name)),
            };
            v = (v << 4) | d;
        }
        out.push(v);
    }
    Ok(out)
}

fn base16_encode(bytes: &[u8], upper: bool, out: &mut String) {
    const LOWER: &[u8; 16] = b"0123456789abcdef";
    const UPPER: &[u8; 16] = b"0123456789ABCDEF";
    let table = if upper { UPPER } else { LOWER };
    for &b in bytes {
        out.push(table[usize::from(b >> 4)] as char);
        out.push(table[usize::from(b & 0x0f)] as char);
    }
}

fn base32_decode(s: &str, upper: bool) -> Result<Vec<u8>, ConceptError> {
    let table = if upper { BASE32_UPPER } else { BASE32_LOWER };
    let name = if upper { "base32 (upper)" } else { "base32" };
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    let mut out = Vec::with_capacity(s.len() * 5 / 8);
    for c in s.chars() {
        let v = table
            .iter()
            .position(|&t| t as char == c)
            .ok_or_else(|| bad_char(c, name))?;
        acc = (acc << 5) | v as u32;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    if bits >= 5 || acc & ((1u32 << bits) - 1) != 0 {
        return Err(ConceptError::Multibase(
            "non-canonical base32 padding bits".to_string(),
        ));
    }
    Ok(out)
}

fn base32_encode(bytes: &[u8], upper: bool, out: &mut String) {
    let table = if upper { BASE32_UPPER } else { BASE32_LOWER };
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    for &b in bytes {
        acc = (acc << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(table[((acc >> bits) & 0x1f) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(table[((acc << (5 - bits)) & 0x1f) as usize] as char);
    }
}

fn base64url_decode(s: &str) -> Result<Vec<u8>, ConceptError> {
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    for c in s.chars() {
        let v = BASE64URL
            .iter()
            .position(|&t| t as char == c)
            .ok_or_else(|| bad_char(c, "base64url (unpadded)"))?;
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    if bits >= 6 || acc & ((1u32 << bits) - 1) != 0 {
        return Err(ConceptError::Multibase(
            "non-canonical base64url padding bits".to_string(),
        ));
    }
    Ok(out)
}

fn base64url_encode(bytes: &[u8], out: &mut String) {
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    for &b in bytes {
        acc = (acc << 8) | u32::from(b);
        bits += 8;
        while bits >= 6 {
            bits -= 6;
            out.push(BASE64URL[((acc >> bits) & 0x3f) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(BASE64URL[((acc << (6 - bits)) & 0x3f) as usize] as char);
    }
}

fn base58btc_decode(s: &str) -> Result<Vec<u8>, ConceptError> {
    let mut out: Vec<u8> = Vec::new();
    for c in s.chars() {
        let v = BASE58BTC
            .iter()
            .position(|&t| t as char == c)
            .ok_or_else(|| bad_char(c, "base58btc"))?;
        let mut carry = v as u32;
        for b in out.iter_mut().rev() {
            carry += 58 * u32::from(*b);
            *b = (carry & 0xff) as u8;
            carry >>= 8;
        }
        while carry > 0 {
            out.insert(0, (carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    // Every leading '1' is one leading zero byte.
    let leading = s.chars().take_while(|&c| c == '1').count();
    let mut result = vec![0u8; leading];
    result.extend_from_slice(&out);
    Ok(result)
}

fn base58btc_encode(bytes: &[u8], out: &mut String) {
    let leading = bytes.iter().take_while(|&&b| b == 0).count();
    let mut digits: Vec<u8> = Vec::new();
    for &b in &bytes[leading..] {
        let mut carry = u32::from(b);
        for d in &mut digits {
            carry += u32::from(*d) << 8;
            *d = (carry % 58) as u8;
            carry /= 58;
        }
        while carry > 0 {
            digits.push((carry % 58) as u8);
            carry /= 58;
        }
    }
    for _ in 0..leading {
        out.push('1');
    }
    for &d in digits.iter().rev() {
        out.push(BASE58BTC[usize::from(d)] as char);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::{GraphName, Literal, NamedNode, NamedOrBlankNode, Term};

    fn quad(object: &str) -> Quad {
        Quad::new(
            NamedOrBlankNode::NamedNode(NamedNode::new("http://ex/concept").unwrap()),
            NamedNode::new("http://ex/label").unwrap(),
            Term::Literal(Literal::new_simple_literal(object)),
            GraphName::DefaultGraph,
        )
    }

    fn record() -> Vec<Quad> {
        vec![quad("a"), quad("b")]
    }

    // -- Multibase --------------------------------------------------------

    #[test]
    fn multibase_prefix_and_from_prefix_round_trip() {
        for b in [
            Multibase::Base16Lower,
            Multibase::Base16Upper,
            Multibase::Base32Lower,
            Multibase::Base32Upper,
            Multibase::Base58Btc,
            Multibase::Base64Url,
        ] {
            assert_eq!(Multibase::from_prefix(b.prefix()), Some(b));
        }
        assert_eq!(Multibase::from_prefix('q'), None);
    }

    #[test]
    fn multibase_encode_decode_round_trips_every_alphabet() {
        let payloads: [&[u8]; 5] = [
            &[0x12, 0x20],
            &[0x00, 0x00, 0x01],
            &[0xff; 33],
            &[0x42],
            &[0x00, 0xff, 0x7f, 0x80, 0x01],
        ];
        for b in [
            Multibase::Base16Lower,
            Multibase::Base16Upper,
            Multibase::Base32Lower,
            Multibase::Base32Upper,
            Multibase::Base58Btc,
            Multibase::Base64Url,
        ] {
            for p in payloads {
                let s = b.encode(p);
                assert_eq!(s.chars().next(), Some(b.prefix()));
                assert_eq!(b.decode(&s[1..]).unwrap(), p, "round trip for {:?}", b);
            }
        }
    }

    /// Known-answer vectors, so the codecs are pinned against an external
    /// definition rather than only against their own inverse.
    #[test]
    fn multibase_known_answer_vectors() {
        // RFC 4648 §10 test vector "foobar" (base16/base32/base64url) and the
        // base58btc encoding of the same bytes.
        let foobar = b"foobar";
        assert_eq!(Multibase::Base16Lower.encode(foobar), "f666f6f626172");
        assert_eq!(Multibase::Base16Upper.encode(foobar), "F666F6F626172");
        assert_eq!(Multibase::Base32Lower.encode(foobar), "bmzxw6ytboi");
        assert_eq!(Multibase::Base32Upper.encode(foobar), "BMZXW6YTBOI");
        assert_eq!(Multibase::Base64Url.encode(foobar), "uZm9vYmFy");
        assert_eq!(Multibase::Base58Btc.encode(foobar), "zt1Zv2yaZ");
        // Leading zero bytes are '1's in base58btc, not dropped.
        assert_eq!(Multibase::Base58Btc.encode(&[0, 0, 1]), "z112");
        assert_eq!(Multibase::Base58Btc.decode("112").unwrap(), vec![0, 0, 1]);
    }

    #[test]
    fn multibase_rejects_out_of_alphabet_and_wrong_case() {
        assert!(Multibase::Base16Lower.decode("00FF").is_err());
        assert!(Multibase::Base16Upper.decode("00ff").is_err());
        assert!(Multibase::Base16Lower.decode("0").is_err());
        assert!(Multibase::Base32Lower.decode("MZXW6YTBOI").is_err());
        assert!(Multibase::Base32Lower.decode("mzxw6ytb01").is_err()); // 0/1 not in base32
        assert!(Multibase::Base58Btc.decode("0OIl").is_err());
        assert!(Multibase::Base64Url.decode("Zm9v+mFy").is_err()); // '+' is base64, not url
        assert!(Multibase::Base32Lower.decode("mzxw6ytbo\u{00e9}").is_err()); // non-ascii
    }

    /// A hostile name must be refused on its LENGTH, before any alphabet is
    /// decoded: base58btc's decoder is quadratic in the body length, so
    /// rejecting only once the multihash turns out malformed would already have
    /// paid the cost. Asserting the specific length message (not merely
    /// `is_err`) is what makes this red if the check is removed — an oversized
    /// body still errors eventually, just far too late.
    #[test]
    fn multibase_rejects_an_oversized_body_before_decoding() {
        let long = "z".repeat(MAX_MULTIBASE_BODY_LEN + 1);
        for b in [
            Multibase::Base58Btc,
            Multibase::Base16Lower,
            Multibase::Base32Lower,
            Multibase::Base64Url,
        ] {
            let err = b.decode(&long).unwrap_err();
            assert!(
                matches!(&err, ConceptError::Multibase(m) if m.contains("rejected before decoding")),
                "{:?} did not reject an oversized body on length: {:?}",
                b,
                err
            );
        }
        // Same for a body that is entirely valid base58btc characters and long
        // enough that decoding it would be the expensive path.
        let urn = format!("{}z{}", URN_PREFIX, "Q".repeat(4096));
        assert!(matches!(
            ConceptUrn::parse(&urn),
            Err(ConceptError::Multibase(_))
        ));
        // The bound leaves every real name room: SHA-512 in base16, the widest
        // digest in the least dense alphabet, is the worst case and still parses.
        let widest = concept_urn(&record(), ConceptHash::Sha512, Multibase::Base16Upper).unwrap();
        assert!(widest.len() - URN_PREFIX.len() - 1 <= MAX_MULTIBASE_BODY_LEN);
        ConceptUrn::parse(&widest).unwrap();
    }

    #[test]
    fn multibase_rejects_non_canonical_padding_bits() {
        // 'a' = 0 followed by 'b' = 1: 10 bits, one byte emitted, 2 residual
        // bits set -> not canonical.
        assert!(Multibase::Base32Lower.decode("ab").is_err());
        // 'A' = 0: 12 bits, one byte emitted, 4 residual bits clear -> canonical.
        assert_eq!(Multibase::Base64Url.decode("AA").unwrap(), vec![0u8]);
        assert!(Multibase::Base64Url.decode("AB").is_err()); // residual bits set
    }

    // -- Varint / multihash ----------------------------------------------

    #[test]
    fn uvarint_round_trips_and_rejects_non_minimal() {
        for v in [0u64, 1, 0x12, 0x7f, 0x80, 0x3fff, 0x4000, u64::MAX >> 1] {
            let mut buf = Vec::new();
            write_uvarint(v, &mut buf);
            assert_eq!(read_uvarint(&buf).unwrap(), (v, buf.len()));
        }
        // 0x80 0x00 is a non-minimal encoding of 0.
        assert!(read_uvarint(&[0x80, 0x00]).is_err());
        // Continuation bit set with nothing following.
        assert!(read_uvarint(&[0x80]).is_err());
        // Longer than the 9-byte multiformats limit.
        assert!(read_uvarint(&[0x80; 12]).is_err());
    }

    #[test]
    fn parse_multihash_accepts_well_formed_and_reports_offset() {
        let mut mh = vec![0x12, 0x20];
        mh.extend_from_slice(&[7u8; 32]);
        assert_eq!(parse_multihash(&mh).unwrap(), (0x12, 2));
    }

    #[test]
    fn parse_multihash_fails_closed_on_every_malformation() {
        // Declared length disagrees with the bytes present (truncated digest).
        let mut short = vec![0x12, 0x20];
        short.extend_from_slice(&[7u8; 31]);
        assert!(parse_multihash(&short).is_err());
        // Trailing bytes after the digest.
        let mut long = vec![0x12, 0x20];
        long.extend_from_slice(&[7u8; 33]);
        assert!(parse_multihash(&long).is_err());
        // Declared length disagrees with the code's natural size.
        let mut wrong = vec![0x12, 0x10];
        wrong.extend_from_slice(&[7u8; 16]);
        assert!(parse_multihash(&wrong).is_err());
        // Zero-length digest.
        assert!(parse_multihash(&[0x12, 0x00]).is_err());
        // An unknown code is *structurally* fine here; it is rejected later by
        // `ConceptHash::from_code`, which is what turns it into a refusal.
        let mut unknown = vec![0x99, 0x01, 0x04];
        unknown.extend_from_slice(&[7u8; 4]);
        assert_eq!(parse_multihash(&unknown).unwrap().0, 0x99);
    }

    // -- ConceptHash ------------------------------------------------------

    #[test]
    fn concept_hash_codes_lengths_and_lookup_agree() {
        for h in [ConceptHash::Sha256, ConceptHash::Sha512, ConceptHash::Sha384] {
            assert_eq!(ConceptHash::from_code(h.code()), Some(h));
            assert_eq!(h.digest_record(&record()).unwrap().len(), h.digest_len());
        }
        assert_eq!(ConceptHash::Sha256.code(), 0x12);
        assert_eq!(ConceptHash::Sha512.code(), 0x13);
        assert_eq!(ConceptHash::Sha384.code(), 0x20);
        assert_eq!(ConceptHash::from_code(0x11), None); // sha1 — not accepted
    }

    // -- ct_eq ------------------------------------------------------------

    #[test]
    fn ct_eq_matches_equality_including_length() {
        assert!(ct_eq(&[1, 2, 3], &[1, 2, 3]));
        assert!(!ct_eq(&[1, 2, 3], &[1, 2, 4]));
        assert!(!ct_eq(&[1, 2, 3], &[1, 2]));
        assert!(ct_eq(&[], &[]));
    }

    // -- ConceptUrn -------------------------------------------------------

    #[test]
    fn concept_urn_parses_and_round_trips() {
        let urn = concept_urn(&record(), ConceptHash::Sha256, Multibase::Base58Btc).unwrap();
        let parsed = ConceptUrn::parse(&urn).unwrap();
        assert_eq!(parsed.to_urn(), urn);
        assert_eq!(parsed.to_string(), urn);
        assert_eq!(parsed.multibase(), Multibase::Base58Btc);
        assert_eq!(parsed.hash_code(), 0x12);
        assert_eq!(parsed.digest().len(), 32);
        assert_eq!(parsed.multihash().len(), 34);
        assert_eq!(&parsed.multihash()[2..], parsed.digest());
    }

    #[test]
    fn concept_urn_prefix_is_case_insensitive_but_body_is_not() {
        let urn = concept_urn(&record(), ConceptHash::Sha256, Multibase::Base16Lower).unwrap();
        let upper_prefix = format!("URN:Concept:{}", &urn[URN_PREFIX.len()..]);
        assert_eq!(
            ConceptUrn::parse(&upper_prefix).unwrap(),
            ConceptUrn::parse(&urn).unwrap()
        );
        // The body is read case-SENSITIVELY: upper-casing an `f` (base16-lower)
        // body flips the prefix to `F`, which is a different — here still
        // valid, and same-bytes — alphabet, not the same string.
        let upper_body = format!("{}{}", URN_PREFIX, urn[URN_PREFIX.len()..].to_uppercase());
        let upper = ConceptUrn::parse(&upper_body).unwrap();
        assert_eq!(upper.multibase(), Multibase::Base16Upper);
        assert_eq!(upper.multihash(), ConceptUrn::parse(&urn).unwrap().multihash());
        // Where the alphabet is case-significant, the same edit is a rejection.
        let b58 = concept_urn(&record(), ConceptHash::Sha256, Multibase::Base58Btc).unwrap();
        let b58_upper = format!("{}{}", URN_PREFIX, b58[URN_PREFIX.len()..].to_uppercase());
        assert!(matches!(
            ConceptUrn::parse(&b58_upper),
            Err(ConceptError::UnknownMultibase('Z'))
        ));
    }

    #[test]
    fn concept_urn_parse_rejects_bad_envelopes() {
        assert!(matches!(
            ConceptUrn::parse("http://ex/not-a-urn"),
            Err(ConceptError::NotConceptUrn)
        ));
        assert!(matches!(
            ConceptUrn::parse("urn:concept:"),
            Err(ConceptError::NotConceptUrn)
        ));
        assert!(matches!(
            ConceptUrn::parse("urn:concept:Q1220ff"),
            Err(ConceptError::UnknownMultibase('Q'))
        ));
        assert!(matches!(
            ConceptUrn::parse("urn:concept:fzz"),
            Err(ConceptError::Multibase(_))
        ));
        assert!(matches!(
            ConceptUrn::parse("urn:concept:f1220ff"),
            Err(ConceptError::Multihash(_))
        ));
        // A multi-byte character straddling the prefix boundary must not panic.
        assert!(ConceptUrn::parse("urn:conce\u{00e9}t:f00").is_err());
    }

    // -- concept_multihash / concept_urn ---------------------------------

    #[test]
    fn concept_multihash_wraps_the_canonical_digest() {
        let mh = concept_multihash(&record(), ConceptHash::Sha256).unwrap();
        assert_eq!(mh[0], 0x12);
        assert_eq!(mh[1], 32);
        assert_eq!(
            &mh[2..],
            &digest_quads_with::<sha2::Sha256>(&record()).unwrap()[..]
        );
    }

    #[test]
    fn concept_multihash_and_urn_reject_an_empty_record() {
        assert!(matches!(
            concept_multihash(&[], ConceptHash::Sha256),
            Err(ConceptError::EmptyRecord)
        ));
        assert!(matches!(
            concept_urn(&[], ConceptHash::Sha256, Multibase::Base58Btc),
            Err(ConceptError::EmptyRecord)
        ));
    }

    #[test]
    fn concept_urn_is_stable_under_quad_reordering() {
        let forward = concept_urn(&record(), ConceptHash::Sha256, Multibase::Base32Lower).unwrap();
        let reversed: Vec<Quad> = record().into_iter().rev().collect();
        let backward = concept_urn(&reversed, ConceptHash::Sha256, Multibase::Base32Lower).unwrap();
        assert_eq!(forward, backward);
    }

    // -- verify ------------------------------------------------------------

    #[test]
    fn verify_accepts_the_matching_record_in_every_alphabet_and_hash() {
        for base in [
            Multibase::Base16Lower,
            Multibase::Base16Upper,
            Multibase::Base32Lower,
            Multibase::Base32Upper,
            Multibase::Base58Btc,
            Multibase::Base64Url,
        ] {
            for h in [ConceptHash::Sha256, ConceptHash::Sha384, ConceptHash::Sha512] {
                let urn = concept_urn(&record(), h, base).unwrap();
                verify_concept_urn(&urn, &record()).unwrap();
                let parsed = ConceptUrn::parse(&urn).unwrap();
                verify_concept(&parsed, &record()).unwrap();
            }
        }
    }

    #[test]
    fn verify_rejects_a_substituted_record() {
        let urn = concept_urn(&record(), ConceptHash::Sha256, Multibase::Base58Btc).unwrap();
        assert!(matches!(
            verify_concept_urn(&urn, &[quad("a")]),
            Err(ConceptError::Mismatch)
        ));
        assert!(matches!(
            verify_concept_urn(&urn, &[quad("a"), quad("c")]),
            Err(ConceptError::Mismatch)
        ));
        assert!(matches!(
            verify_concept_urn(&urn, &[]),
            Err(ConceptError::EmptyRecord)
        ));
    }

    /// A flipped digest byte must be rejected — the guard's whole purpose.
    #[test]
    fn verify_rejects_a_single_flipped_digest_bit() {
        let mut mh = concept_multihash(&record(), ConceptHash::Sha256).unwrap();
        mh[10] ^= 0x01;
        let urn = format!(
            "{}{}",
            URN_PREFIX,
            Multibase::Base58Btc.encode(&mh).as_str()
        );
        assert!(matches!(
            verify_concept_urn(&urn, &record()),
            Err(ConceptError::Mismatch)
        ));
    }

    /// The multihash PREFIX is part of the comparison: a URN carrying the
    /// correct SHA-256 digest under a SHA-512 code must not verify.
    #[test]
    fn verify_rejects_a_downgraded_hash_code_carrying_a_valid_digest() {
        let digest = digest_quads_with::<sha2::Sha256>(&record()).unwrap();
        let mut mh = vec![0x13, digest.len() as u8]; // sha2-512 code, 32-byte digest
        mh.extend_from_slice(&digest);
        let urn = format!("{}{}", URN_PREFIX, Multibase::Base58Btc.encode(&mh));
        // Rejected at envelope-parse time: 0x13 must carry 64 bytes.
        assert!(matches!(
            verify_concept_urn(&urn, &record()),
            Err(ConceptError::Multihash(_))
        ));
    }

    #[test]
    fn verify_rejects_an_unsupported_hash_code() {
        // 0x11 = sha1, structurally valid (20 bytes) but not recomputable here.
        let mut mh = vec![0x11, 20];
        mh.extend_from_slice(&[0u8; 20]);
        let urn = format!("{}{}", URN_PREFIX, Multibase::Base58Btc.encode(&mh));
        assert!(matches!(
            verify_concept_urn(&urn, &record()),
            Err(ConceptError::UnsupportedHash(0x11))
        ));
    }

    #[test]
    fn verify_fails_closed_on_rdf12_triple_terms() {
        use oxrdf::Triple;
        let inner = Triple::new(
            NamedNode::new("http://ex/s").unwrap(),
            NamedNode::new("http://ex/p").unwrap(),
            Term::Literal(Literal::new_simple_literal("v")),
        );
        let tt = vec![Quad::new(
            NamedOrBlankNode::NamedNode(NamedNode::new("http://ex/concept").unwrap()),
            NamedNode::new("http://ex/says").unwrap(),
            Term::Triple(Box::new(inner)),
            GraphName::DefaultGraph,
        )];
        assert!(matches!(
            concept_multihash(&tt, ConceptHash::Sha256),
            Err(ConceptError::Canon(CanonError::TripleTerm))
        ));
    }

    #[test]
    fn errors_display_and_expose_a_canon_source() {
        use std::error::Error as _;
        let e = ConceptError::from(CanonError::TripleTerm);
        assert!(e.source().is_some());
        assert!(e.to_string().contains("canonicalization failed"));
        for e in [
            ConceptError::NotConceptUrn,
            ConceptError::UnknownMultibase('q'),
            ConceptError::Multibase("x".to_string()),
            ConceptError::Multihash("x".to_string()),
            ConceptError::UnsupportedHash(0x11),
            ConceptError::EmptyRecord,
            ConceptError::Mismatch,
        ] {
            assert!(!e.to_string().is_empty());
            assert!(e.source().is_none());
        }
    }
}
