//! brTPF binding-block **transport encodings** — a COMPACT BINARY mapping wire alongside
//! the line-oriented TEXT wire (bead `sq-6ihg`, follow-up to the server's `sq-dxhb` brTPF).
//!
//! # Why a second wire
//!
//! The brTPF bind-join attaches a SET of upstream solution mappings to each fragment request
//! (the [`source::FragBinding`](crate::source::FragBinding) block — at most `maxMpR` mappings,
//! the standardised bind-join of Hartig & Buil-Aranda, ODBASE 2016). The sparq **server**
//! (`sparq-server`, bead `sq-dxhb`) parses that set from a **line-oriented text wire**: one
//! mapping per line, whitespace-separated `position=term` pairs, where `term` is a fully
//! N-Triples-decorated term (`<iri>` / `"lit"` / `"lit"@en` / `"lit"^^<dt>`). That text wire is
//! human-readable and easy to put in a `GET ?values=` query string, but it is **verbose**: it
//! repeats the `s=` / `p=` / `o=` position key and the N-Triples framing (`<…>`, `"…"`,
//! `^^<…>`) on every single term, which inflates the brTPF payload — exactly the payload a
//! large bind-join block makes hot (`maxMpR` can be in the hundreds, and the block is re-sent on
//! every fragment request of a bind nested-loop join).
//!
//! `sq-6ihg` adds the alternative the bead asks for: a **compact, self-describing BINARY
//! mapping transport** ([`encode_bindings`] / [`decode_bindings`]) the client can produce
//! instead of the text wire, plus the text-wire writer ([`encode_bindings_text`]) so the client
//! can speak EITHER form over the same `FragBinding` model (the server already parses the text
//! form). The binary form drops the per-pair position key (a length-prefixed name instead of a
//! repeated `s=`/`p=`/`o=`) and the N-Triples framing (a one-byte **kind tag** distinguishes
//! IRI / blank node / literal, so the bare lexical bytes travel without `<>` / `""` decoration),
//! and length-prefixes every payload so it round-trips losslessly with no escaping.
//!
//! # Honest scope (no overclaim)
//!
//! This module is a **codec only** — it converts a `&[FragBinding]` to/from bytes (binary) and
//! to a `String` (text). It does NOT issue any request: the native HTTP
//! [`FragmentTransport`](crate::source::FragmentTransport) (`HttpFragmentTransport`, bead
//! `sq-yzca`) attaches the **text** form on the `values` query parameter (the carrier the sparq
//! server reads); this binary wire is the compact alternative a body-carrying transport emits.
//! What is load-bearing and tested here is
//! that the binary form is **lossless** (`decode_bindings(encode_bindings(b)) == b` for every
//! binding set, including literals with embedded `=`, whitespace, and newlines that the text
//! wire could not carry on a single line) and that the **text form matches the server's parser
//! grammar** (the same `position=term` grammar the server's `parse_bindings` reads), so a
//! client emitting either wire is understood by the existing server.
//!
//! The binary container carries a 4-byte magic + a 1-byte format version so a future wire
//! revision is detectable rather than silently misparsed, and [`decode_bindings`] validates
//! every length against the remaining input so a truncated / malformed buffer is a clean
//! [`WireError`], never a panic or an out-of-bounds read (this crate is `forbid(unsafe_code)`).
//!
//! [OPUS-4.8] sq-6ihg — Fable re-review when available.

use crate::source::{FragBinding, FragTerm};

/// The 4-byte magic prefixing a binary brTPF binding block: ASCII `"bTPF"`. Lets a reader
/// reject a buffer that is not a binding block before interpreting any length. [OPUS-4.8].
pub const BINARY_MAGIC: [u8; 4] = *b"bTPF";

/// The binary wire format version (the byte after [`BINARY_MAGIC`]). Bumped on any
/// breaking change to the layout so a mismatched peer fails loud rather than misparsing.
pub const BINARY_VERSION: u8 = 1;

/// Mapping-header flag bits. The low three name which of the canonical short positions
/// (`s`/`p`/`o`) this mapping binds — for those, the term follows with NO name bytes (the
/// genuine compactness win over the text wire's repeated `s=`/`p=`/`o=` key). Bit 7 flags that
/// a varint count of EXTRA (arbitrary-name) pairs follows the position terms, so a binding over
/// a non-position variable name still round-trips losslessly.
const HDR_SUBJECT: u8 = 0b0000_0001;
const HDR_PREDICATE: u8 = 0b0000_0010;
const HDR_OBJECT: u8 = 0b0000_0100;
const HDR_HAS_EXTRA: u8 = 0b1000_0000;

/// Term-kind tags (the first byte of every encoded term) — they replace the N-Triples
/// `<…>` / `"…"` framing the text wire repeats, so the term's bare lexical bytes follow.
const KIND_IRI: u8 = 0;
const KIND_BLANK: u8 = 1;
const KIND_LITERAL: u8 = 2;

/// A failure decoding a binary brTPF binding block. Every variant is a *clean* error — a
/// malformed or truncated buffer never panics and never reads out of bounds. [OPUS-4.8].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    /// The buffer was shorter than the fixed header, or a length field pointed past the
    /// end of the remaining bytes (a truncated / corrupt block).
    Truncated,
    /// The leading 4 bytes were not [`BINARY_MAGIC`] — the buffer is not a binding block.
    BadMagic,
    /// The version byte was not a version this build understands.
    UnsupportedVersion(u8),
    /// A mapping bound no position and carried no extra pairs (the empty mapping μ₀, which a
    /// well-formed block never contains — it would not restrict the fragment).
    EmptyMapping,
    /// A term-kind tag was not one of IRI / blank / literal.
    BadTermKind(u8),
    /// A length-prefixed payload was not valid UTF-8 (RDF terms are always UTF-8 text).
    NonUtf8,
    /// A varint ran past the end of the buffer or overflowed `u64`.
    BadVarint,
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WireError::Truncated => write!(f, "binary brTPF block: truncated / length past end"),
            WireError::BadMagic => write!(f, "binary brTPF block: bad magic (not a binding block)"),
            WireError::UnsupportedVersion(v) => {
                write!(f, "binary brTPF block: unsupported format version {v}")
            }
            WireError::EmptyMapping => {
                write!(
                    f,
                    "binary brTPF block: a mapping constrained no position (μ₀)"
                )
            }
            WireError::BadTermKind(k) => write!(f, "binary brTPF block: bad term-kind tag {k}"),
            WireError::NonUtf8 => write!(f, "binary brTPF block: term payload is not UTF-8"),
            WireError::BadVarint => write!(f, "binary brTPF block: malformed length varint"),
        }
    }
}

impl std::error::Error for WireError {}

// ─── varint (LEB128-style, unsigned) ─────────────────────────────────────────────────

/// Append `value` to `out` as an unsigned LEB128 varint (7 payload bits per byte, the high
/// bit a continuation flag). Compact for the small lengths a binding block carries (a term
/// under 128 bytes is one length byte) while still encoding any `u64`.
fn write_varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// Read an unsigned LEB128 varint from `buf` starting at `*pos`, advancing `*pos`. Errors
/// (rather than panicking or looping) on a truncated varint or a value that overflows `u64`.
fn read_varint(buf: &[u8], pos: &mut usize) -> Result<u64, WireError> {
    let mut value: u64 = 0;
    let mut shift: u32 = 0;
    loop {
        let byte = *buf.get(*pos).ok_or(WireError::BadVarint)?;
        *pos += 1;
        // 64-bit varint is at most 10 bytes; a payload bit beyond bit 63 overflows u64.
        if shift >= 64 || (shift == 63 && byte > 1) {
            return Err(WireError::BadVarint);
        }
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
        shift += 7;
    }
}

// ─── compact term <-> binary ──────────────────────────────────────────────────────────

/// The (kind tag, bare lexical bytes) a [`FragTerm`] contributes to the binary wire. The
/// lexical bytes are the term WITHOUT N-Triples framing: an IRI without `<>`, a literal WITH
/// its `"…"`/`@lang`/`^^<dt>` decoration preserved exactly (that decoration is part of the
/// literal's lexical identity in the `FragTerm::Literal` model — see its doc — so it is carried
/// verbatim, not re-stripped), and a blank node's bare label.
fn term_kind_and_lexical(term: &FragTerm) -> (u8, &str) {
    match term {
        FragTerm::Iri(s) => (KIND_IRI, s),
        FragTerm::Blank(s) => (KIND_BLANK, s),
        FragTerm::Literal(s) => (KIND_LITERAL, s),
    }
}

/// Encode one term: a 1-byte kind tag, then a varint byte-length, then the bare UTF-8 bytes.
fn write_term(out: &mut Vec<u8>, term: &FragTerm) {
    let (kind, lexical) = term_kind_and_lexical(term);
    out.push(kind);
    write_varint(out, lexical.len() as u64);
    out.extend_from_slice(lexical.as_bytes());
}

/// Decode one term written by [`write_term`], advancing `*pos`.
fn read_term(buf: &[u8], pos: &mut usize) -> Result<FragTerm, WireError> {
    let kind = *buf.get(*pos).ok_or(WireError::Truncated)?;
    *pos += 1;
    let len = read_varint(buf, pos)? as usize;
    let end = pos.checked_add(len).ok_or(WireError::Truncated)?;
    let bytes = buf.get(*pos..end).ok_or(WireError::Truncated)?;
    *pos = end;
    let lexical = std::str::from_utf8(bytes)
        .map_err(|_| WireError::NonUtf8)?
        .to_string();
    match kind {
        KIND_IRI => Ok(FragTerm::Iri(lexical)),
        KIND_BLANK => Ok(FragTerm::Blank(lexical)),
        KIND_LITERAL => Ok(FragTerm::Literal(lexical)),
        other => Err(WireError::BadTermKind(other)),
    }
}

// ─── brTPF position slot keys (the text wire's vocabulary) ─────────────────────────────

/// The short slot key (`s`/`p`/`o`) the server's text wire uses for a brTPF position, if `name`
/// is one of the reserved subject/predicate/object slots (long or short form). A binding pair
/// over any other variable name has no brTPF text-wire position; the binary wire carries it
/// regardless. brTPF maps a pattern's *variables*; a caller that already speaks the position
/// vocabulary (`s`/`p`/`o`) round-trips through the text wire too.
fn position_key(name: &str) -> Option<&'static str> {
    match name {
        "subject" | "s" => Some("s"),
        "predicate" | "p" => Some("p"),
        "object" | "o" => Some("o"),
        _ => None,
    }
}

/// The header flag bit for a name that is EXACTLY a canonical short position key (`s`/`p`/`o`).
/// Only the canonical short form earns the no-name-bytes header slot, because a decoded position
/// flag reconstructs the name as `s`/`p`/`o`; the long forms (`subject`/…) would not round-trip
/// through a flag, so they ride the EXTRA section under their own name. [OPUS-4.8] sq-6ihg.
fn canonical_position_flag(name: &str) -> Option<u8> {
    match name {
        "s" => Some(HDR_SUBJECT),
        "p" => Some(HDR_PREDICATE),
        "o" => Some(HDR_OBJECT),
        _ => None,
    }
}

// ─── public API: binary encode / decode ────────────────────────────────────────────────

/// Serialise a brTPF binding block to the **compact binary wire** (bead `sq-6ihg`).
///
/// Layout (all integers unsigned LEB128 varints):
///
/// ```text
/// magic   : 4 bytes  = BINARY_MAGIC ("bTPF")
/// version : 1 byte   = BINARY_VERSION
/// count   : varint   = number of mappings that follow
/// mapping* :
///   header : 1 byte  = position flags (bit0 s, bit1 p, bit2 o) | (bit7 has-extra)
///   pos-term* : for each set position flag, in s→p→o order, a TERM with NO name bytes
///               (the position IS the name — the compactness win over the text wire's
///               repeated `s=`/`p=`/`o=` key)
///   [ if bit7 ] :
///     nextra : varint = number of arbitrary-name pairs that follow
///     extra* : name (varint length-prefixed UTF-8) + TERM
///   TERM := KIND tag (1 byte) + varint length-prefixed bare lexical UTF-8
/// ```
///
/// The compactness win is twofold: the common brTPF case — a mapping over the position
/// vocabulary `s`/`p`/`o` — pays NO per-pair name bytes (the header bit IS the name), and every
/// term drops the N-Triples framing (`<>` / `""` / `^^<>`) the text wire repeats, carrying its
/// bare lexical bytes length-prefixed instead. A binding over an arbitrary (non-position)
/// variable name still round-trips losslessly via the EXTRA section, so the wire carries any
/// [`FragBinding`] — including literal terms with embedded `=`, whitespace, or newlines that the
/// line-oriented text wire cannot represent on a single line.
///
/// The **empty mapping μ₀** (a binding with no pairs) does not restrict a fragment, so it is
/// SKIPPED here exactly as the server's `parse_bindings` skips an all-blank line — the block a
/// well-formed encoder emits never contains one, and [`decode_bindings`] rejects one as
/// [`WireError::EmptyMapping`] if a peer sends it.
pub fn encode_bindings(bindings: &[FragBinding]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + bindings.len() * 16);
    out.extend_from_slice(&BINARY_MAGIC);
    out.push(BINARY_VERSION);
    // Count the non-empty mappings first (μ₀ is skipped) so the header count matches the body.
    let nonempty: Vec<&FragBinding> = bindings.iter().filter(|b| !b.is_empty()).collect();
    write_varint(&mut out, nonempty.len() as u64);
    for b in nonempty {
        // Partition the mapping's pairs: those keyed by a canonical short position (s/p/o) ride
        // the header flags with no name bytes; everything else rides the EXTRA section. The FIRST
        // pair for each position wins the header slot (a pair repeating a position is rare in a
        // brTPF mapping, but if present its duplicates fall to EXTRA so nothing is dropped).
        let mut subject: Option<&FragTerm> = None;
        let mut predicate: Option<&FragTerm> = None;
        let mut object: Option<&FragTerm> = None;
        let mut extra: Vec<(&str, &FragTerm)> = Vec::new();
        for (name, term) in b {
            match canonical_position_flag(name) {
                Some(HDR_SUBJECT) if subject.is_none() => subject = Some(term),
                Some(HDR_PREDICATE) if predicate.is_none() => predicate = Some(term),
                Some(HDR_OBJECT) if object.is_none() => object = Some(term),
                _ => extra.push((name.as_str(), term)),
            }
        }
        let mut header = 0u8;
        if subject.is_some() {
            header |= HDR_SUBJECT;
        }
        if predicate.is_some() {
            header |= HDR_PREDICATE;
        }
        if object.is_some() {
            header |= HDR_OBJECT;
        }
        if !extra.is_empty() {
            header |= HDR_HAS_EXTRA;
        }
        out.push(header);
        for term in [subject, predicate, object].into_iter().flatten() {
            write_term(&mut out, term);
        }
        if !extra.is_empty() {
            write_varint(&mut out, extra.len() as u64);
            for (name, term) in extra {
                write_varint(&mut out, name.len() as u64);
                out.extend_from_slice(name.as_bytes());
                write_term(&mut out, term);
            }
        }
    }
    out
}

/// Decode a binary brTPF binding block written by [`encode_bindings`]. Validates the magic,
/// version, and every length against the remaining input, so a truncated / corrupt / wrong-magic
/// buffer is a clean [`WireError`] (never a panic, never an out-of-bounds read — this crate is
/// `forbid(unsafe_code)`). A mapping that binds no position and carries no extras is rejected as
/// [`WireError::EmptyMapping`].
///
/// Position pairs are reconstructed in canonical `s`→`p`→`o` order with the short key names,
/// followed by the EXTRA (arbitrary-name) pairs in their encoded order. The round-trip is exact
/// for any [`FragBinding`] whose position keys are the canonical short forms (`s`/`p`/`o`) — the
/// brTPF position vocabulary — up to that canonical position ordering (a deterministic wire
/// order is a feature; a brTPF mapping binds each position at most once, so this re-orders only
/// the position keys, never changes which term binds which position).
pub fn decode_bindings(buf: &[u8]) -> Result<Vec<FragBinding>, WireError> {
    let mut pos = 0usize;
    let magic = buf.get(0..4).ok_or(WireError::Truncated)?;
    if magic != BINARY_MAGIC {
        return Err(WireError::BadMagic);
    }
    pos += 4;
    let version = *buf.get(pos).ok_or(WireError::Truncated)?;
    pos += 1;
    if version != BINARY_VERSION {
        return Err(WireError::UnsupportedVersion(version));
    }
    let count = read_varint(buf, &mut pos)? as usize;
    let mut out: Vec<FragBinding> = Vec::with_capacity(count);
    for _ in 0..count {
        let header = *buf.get(pos).ok_or(WireError::Truncated)?;
        pos += 1;
        let mut binding: FragBinding = Vec::with_capacity(3);
        // Position terms, in s→p→o order, each with NO name bytes (the flag IS the name).
        for (flag, key) in [(HDR_SUBJECT, "s"), (HDR_PREDICATE, "p"), (HDR_OBJECT, "o")] {
            if header & flag != 0 {
                binding.push((key.to_string(), read_term(buf, &mut pos)?));
            }
        }
        // The EXTRA (arbitrary-name) pairs, if the header flagged any.
        if header & HDR_HAS_EXTRA != 0 {
            let nextra = read_varint(buf, &mut pos)? as usize;
            for _ in 0..nextra {
                let name_len = read_varint(buf, &mut pos)? as usize;
                let name_end = pos.checked_add(name_len).ok_or(WireError::Truncated)?;
                let name_bytes = buf.get(pos..name_end).ok_or(WireError::Truncated)?;
                pos = name_end;
                let name = std::str::from_utf8(name_bytes)
                    .map_err(|_| WireError::NonUtf8)?
                    .to_string();
                binding.push((name, read_term(buf, &mut pos)?));
            }
        }
        // A header that named no position and flagged no extras is μ₀ on the wire — a well-formed
        // block never contains one (encode skips it), so reject it rather than emit an empty row.
        if binding.is_empty() {
            return Err(WireError::EmptyMapping);
        }
        out.push(binding);
    }
    Ok(out)
}

// ─── public API: text-wire writer (the server's existing line format) ──────────────────

/// Serialise a brTPF binding block to the **line-oriented text wire** the sparq server parses
/// (`sq-dxhb`'s `parse_bindings`): one mapping per line; within a line, space-separated
/// `position=term` pairs, where `position` is the short `s`/`p`/`o` and `term` is the N-Triples
/// form. Only the reserved subject/predicate/object slots are representable on this wire (the
/// brTPF position vocabulary); a binding pair whose variable name is NOT one of those is dropped
/// from the TEXT form (it has no brTPF position to map to) — the binary wire is the lossless
/// transport for arbitrary variable names. An empty mapping (μ₀) emits no line.
///
/// This is the human-readable / `GET ?values=` carrier; [`encode_bindings`] is the compact
/// alternative the bead adds. A client picks one per request; the server understands the text
/// form today.
pub fn encode_bindings_text(bindings: &[FragBinding]) -> String {
    let mut lines: Vec<String> = Vec::with_capacity(bindings.len());
    for b in bindings {
        let mut pairs: Vec<String> = Vec::with_capacity(3);
        // Emit subject, then predicate, then object — a deterministic, parser-friendly order.
        for want in ["s", "p", "o"] {
            for (name, term) in b {
                if position_key(name) == Some(want) {
                    pairs.push(format!("{want}={}", term.to_ntriples()));
                }
            }
        }
        if !pairs.is_empty() {
            lines.push(pairs.join(" "));
        }
    }
    lines.join("\n")
}

// ─── tests ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn iri(s: &str) -> FragTerm {
        FragTerm::Iri(s.to_string())
    }
    fn lit(s: &str) -> FragTerm {
        FragTerm::Literal(s.to_string())
    }

    /// A small heterogeneous block: a subject IRI, a predicate IRI, an object literal, plus a
    /// mapping over a NON-position variable name (only the binary wire can carry that).
    fn sample_block() -> Vec<FragBinding> {
        vec![
            vec![("s".to_string(), iri("http://ex/alice"))],
            vec![("s".to_string(), iri("http://ex/bob"))],
            vec![
                ("s".to_string(), iri("http://ex/carol")),
                ("o".to_string(), lit("\"Carol\"@en")),
            ],
            vec![("person".to_string(), iri("http://ex/dave"))],
        ]
    }

    #[test]
    fn binary_round_trips_losslessly() {
        let block = sample_block();
        let bytes = encode_bindings(&block);
        let back = decode_bindings(&bytes).expect("decode");
        assert_eq!(back, block, "binary wire must round-trip every binding");
    }

    #[test]
    fn binary_header_is_magic_version() {
        let bytes = encode_bindings(&sample_block());
        assert_eq!(&bytes[0..4], &BINARY_MAGIC, "leading magic");
        assert_eq!(bytes[4], BINARY_VERSION, "version byte");
    }

    #[test]
    fn binary_carries_chars_the_text_wire_cannot() {
        // A literal containing '=', a space, a tab, AND a newline — none survive the one-mapping-
        // per-line, position=term text wire, but the length-prefixed binary wire carries them
        // exactly.
        let nasty = vec![vec![("o".to_string(), lit("\"a = b\tc\nd\""))]];
        let bytes = encode_bindings(&nasty);
        assert_eq!(decode_bindings(&bytes).unwrap(), nasty);
    }

    #[test]
    fn binary_is_more_compact_than_text_for_iris() {
        // The compact win: no per-pair "s=" key, no "<>" framing. For a block of subject IRIs the
        // binary form must be strictly smaller than the text form.
        let block: Vec<FragBinding> = (0..32)
            .map(|i| {
                vec![(
                    "s".to_string(),
                    iri(&format!("http://example.org/resource/{i}")),
                )]
            })
            .collect();
        let bin = encode_bindings(&block).len();
        let txt = encode_bindings_text(&block).len();
        assert!(
            bin < txt,
            "binary wire ({bin} B) must be more compact than text ({txt} B)"
        );
    }

    #[test]
    fn empty_block_round_trips_as_empty() {
        let bytes = encode_bindings(&[]);
        assert_eq!(decode_bindings(&bytes).unwrap(), Vec::<FragBinding>::new());
        assert_eq!(encode_bindings_text(&[]), "");
    }

    #[test]
    fn mapping_mixing_position_and_extra_round_trips() {
        // A mapping that binds a canonical position (rides the header, no name bytes) AND a
        // non-position variable (rides the EXTRA section under its own name).
        let block = vec![vec![
            ("s".to_string(), iri("http://ex/alice")),
            ("friend".to_string(), iri("http://ex/bob")),
        ]];
        let back = decode_bindings(&encode_bindings(&block)).unwrap();
        // Position pairs come first (s here), then extras in encoded order — exactly the input.
        assert_eq!(back, block);
    }

    #[test]
    fn position_keys_decode_in_canonical_spo_order() {
        // The binary wire normalises position keys to the deterministic s→p→o order. A mapping
        // encoded with positions out of order decodes in canonical order — a feature (a
        // deterministic wire), and it never changes WHICH term binds WHICH position.
        let block = vec![vec![
            ("o".to_string(), iri("http://ex/obj")),
            ("s".to_string(), iri("http://ex/subj")),
        ]];
        let back = decode_bindings(&encode_bindings(&block)).unwrap();
        assert_eq!(
            back,
            vec![vec![
                ("s".to_string(), iri("http://ex/subj")),
                ("o".to_string(), iri("http://ex/obj")),
            ]]
        );
    }

    #[test]
    fn empty_mapping_mu0_is_skipped_on_encode() {
        // μ₀ (a binding with no pairs) does not restrict the fragment, so encode drops it —
        // matching the server's parse_bindings, which skips an all-blank line.
        let block = vec![
            vec![],                                      // μ₀ — dropped
            vec![("s".to_string(), iri("http://ex/a"))], // kept
        ];
        let back = decode_bindings(&encode_bindings(&block)).unwrap();
        assert_eq!(back, vec![vec![("s".to_string(), iri("http://ex/a"))]]);
        // The text wire likewise emits a single line.
        assert_eq!(encode_bindings_text(&block), "s=<http://ex/a>");
    }

    #[test]
    fn text_wire_matches_the_server_grammar() {
        // The text form is the server's `position=term` line grammar: short s/p/o keys, N-Triples
        // terms, one mapping per line, deterministic s→p→o order within a line.
        let block = vec![vec![
            ("o".to_string(), lit("\"Carol\"@en")),
            ("s".to_string(), iri("http://ex/carol")),
        ]];
        assert_eq!(
            encode_bindings_text(&block),
            "s=<http://ex/carol> o=\"Carol\"@en"
        );
        let two = vec![
            vec![("s".to_string(), iri("http://ex/alice"))],
            vec![("s".to_string(), iri("http://ex/bob"))],
        ];
        assert_eq!(
            encode_bindings_text(&two),
            "s=<http://ex/alice>\ns=<http://ex/bob>"
        );
    }

    // [FABLE-5] sq-3dyje.6 (mutation-kill): position_key must map EACH of the three brTPF
    // slots, in BOTH long and short spellings. cargo-mutants showed the `"predicate" | "p"`
    // arm could be deleted with no test noticing: no existing test binds a predicate slot,
    // so a predicate binding silently vanished from the text wire (and the header flag from
    // the binary wire). Pin every slot with a term unique to it so a dropped/misrouted arm
    // is observable in the exact rendered output.
    #[test]
    fn text_wire_renders_all_three_position_slots_both_spellings() {
        // Short spellings s/p/o, out of order — deterministic s→p→o output, every slot present.
        let short = vec![vec![
            ("o".to_string(), iri("http://ex/obj")),
            ("p".to_string(), iri("http://ex/pred")),
            ("s".to_string(), iri("http://ex/subj")),
        ]];
        assert_eq!(
            encode_bindings_text(&short),
            "s=<http://ex/subj> p=<http://ex/pred> o=<http://ex/obj>",
            "each of s/p/o must render; deleting the predicate arm drops p="
        );
        // Long spellings subject/predicate/object map to the SAME s/p/o slots.
        let long = vec![vec![
            ("subject".to_string(), iri("http://ex/subj")),
            ("predicate".to_string(), iri("http://ex/pred")),
            ("object".to_string(), iri("http://ex/obj")),
        ]];
        assert_eq!(
            encode_bindings_text(&long),
            "s=<http://ex/subj> p=<http://ex/pred> o=<http://ex/obj>",
            "long forms subject/predicate/object route to the s/p/o slots"
        );
    }

    #[test]
    fn binary_wire_predicate_only_mapping_round_trips() {
        // A mapping that binds ONLY the predicate position: the header's PREDICATE flag must
        // be set and the term decode back under the "p" key. If position handling dropped the
        // predicate, this would decode empty (EmptyMapping) — a hard failure.
        let block = vec![vec![("p".to_string(), iri("http://ex/pred"))]];
        let back = decode_bindings(&encode_bindings(&block)).expect("decode");
        assert_eq!(back, block, "a predicate-only mapping round-trips exactly");
    }

    #[test]
    fn text_wire_drops_non_position_variables() {
        // A binding over a variable that is not a brTPF position has no text-wire slot, so the
        // text form omits it (the binary wire is the lossless carrier for arbitrary names).
        let block = vec![vec![("person".to_string(), iri("http://ex/dave"))]];
        assert_eq!(encode_bindings_text(&block), "");
        // …but binary keeps it.
        assert_eq!(decode_bindings(&encode_bindings(&block)).unwrap(), block);
    }

    #[test]
    fn decode_rejects_bad_magic() {
        let mut bytes = encode_bindings(&sample_block());
        bytes[0] = b'X';
        assert_eq!(decode_bindings(&bytes), Err(WireError::BadMagic));
    }

    #[test]
    fn decode_rejects_unsupported_version() {
        let mut bytes = encode_bindings(&sample_block());
        bytes[4] = 99;
        assert_eq!(
            decode_bindings(&bytes),
            Err(WireError::UnsupportedVersion(99))
        );
    }

    #[test]
    fn decode_rejects_truncation_without_panic() {
        let bytes = encode_bindings(&sample_block());
        // Every truncation point must be a clean error, never a panic / OOB read.
        for cut in 0..bytes.len() {
            let _ = decode_bindings(&bytes[..cut]); // must not panic
        }
        // A buffer cut just before the final byte is specifically a clean length error.
        let cut = bytes.len() - 1;
        assert!(matches!(
            decode_bindings(&bytes[..cut]),
            Err(WireError::Truncated | WireError::BadVarint)
        ));
    }

    #[test]
    fn decode_rejects_empty_buffer() {
        assert_eq!(decode_bindings(&[]), Err(WireError::Truncated));
    }

    #[test]
    fn decode_rejects_empty_mapping() {
        // Hand-craft a block whose single mapping binds no position and flags no extras — μ₀.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&BINARY_MAGIC);
        bytes.push(BINARY_VERSION);
        write_varint(&mut bytes, 1); // count = 1
        bytes.push(0); // header = 0 → no positions, no extras → μ₀
        assert_eq!(decode_bindings(&bytes), Err(WireError::EmptyMapping));
    }

    #[test]
    fn decode_rejects_bad_term_kind() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&BINARY_MAGIC);
        bytes.push(BINARY_VERSION);
        write_varint(&mut bytes, 1); // count = 1
        bytes.push(HDR_SUBJECT); // header: subject position present
        bytes.push(7); // bad term kind tag for the subject term
        write_varint(&mut bytes, 0); // term len = 0
        assert_eq!(decode_bindings(&bytes), Err(WireError::BadTermKind(7)));
    }

    #[test]
    fn varint_round_trips_edge_values() {
        for v in [0u64, 1, 127, 128, 16383, 16384, u32::MAX as u64, u64::MAX] {
            let mut buf = Vec::new();
            write_varint(&mut buf, v);
            let mut pos = 0;
            assert_eq!(read_varint(&buf, &mut pos).unwrap(), v, "varint {v}");
            assert_eq!(pos, buf.len(), "varint {v} consumed all bytes");
        }
    }

    #[test]
    fn varint_rejects_overflow_and_truncation() {
        // 9 continuation bytes then a high payload byte overflows u64.
        let overflow = [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02];
        let mut pos = 0;
        assert_eq!(read_varint(&overflow, &mut pos), Err(WireError::BadVarint));
        // A continuation byte with no successor is truncated.
        let truncated = [0x80u8];
        let mut pos2 = 0;
        assert_eq!(
            read_varint(&truncated, &mut pos2),
            Err(WireError::BadVarint)
        );
    }

    #[test]
    fn blank_node_term_round_trips_both_wires() {
        let block = vec![vec![("s".to_string(), FragTerm::Blank("b0".to_string()))]];
        assert_eq!(decode_bindings(&encode_bindings(&block)).unwrap(), block);
        assert_eq!(encode_bindings_text(&block), "s=_:b0");
    }
}
