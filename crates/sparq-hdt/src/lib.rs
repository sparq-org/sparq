// sq-jxl0: single-source the crate overview from README.md so crates.io
// (package.readme) and the docs.rs front page render identical content. The README's
// quickstart fence is a `no_run` doctest (it opens a `.hdt` file that need not exist).
#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)] // sq-emay: crate has zero `unsafe`

use sparq_core::dict::{Dict, Id};
use sparq_core::Graph;

use hdt::containers::ControlInfo;
use hdt::header::Header;
use hdt::{Hdt, IdKind};
use std::io::BufRead;
use std::path::Path;

// Direct sparq-side decoder (plan levers H1–H4): one-shot SPO scan that
// skips the upstream `TriplesBitmap` query machinery (wavelet matrix + OP-index +
// rank/select) sparq never uses on a bulk load. This is the default decode path;
// `graph_from_hdt` (below) is kept as the upstream-backed differential oracle and
// for callers that already hold an `Hdt` (e.g. to also query its header).
mod decode;
pub use decode::graph_from_reader;
// (sq-q6a1) Measurement-only: per-stage timed decode for bench/parse's
// 3-way HDT split. Identical decode path; the production `graph_from_reader` times
// nothing.
pub use decode::{graph_from_reader_timed, StageTimings};

// sq-2te / sq-ashy: HDT write support (sparq `Graph` -> `.hdt`). Opt-in
// via the `write` feature (it pulls the wrapped crate's `sophia` feature for the
// section builders/writers). `save` (write.rs) encodes the FourSectDict PFC +
// BitmapTriples sections DIRECTLY from sparq's in-memory dict + triples via
// `encode::write_hdt` (the inverse of `decode`) — skipping the previous temporary
// N-Triples round-trip entirely.
#[cfg(feature = "write")]
mod encode;
#[cfg(feature = "write")]
mod write;
#[cfg(feature = "write")]
pub use write::save;

/// A subject/predicate/object filter for opt-in HDT loading.
///
/// `None` is a wildcard. Subjects use [`oxrdf::NamedOrBlankNode`], predicates
/// use [`oxrdf::NamedNode`], and objects use [`oxrdf::Term`], matching the RDF
/// term kinds allowed in each triple position.
#[cfg(feature = "load-filter")]
pub type TriplePattern = (
    Option<oxrdf::NamedOrBlankNode>,
    Option<oxrdf::NamedNode>,
    Option<oxrdf::Term>,
);

/// The error type for HDT loading.
#[derive(Debug)]
pub enum Error {
    /// The file could not be opened / read.
    Io(std::io::Error),
    /// The archive could not be decoded (unsupported section type, checksum
    /// mismatch, truncation, …).
    Hdt(hdt::hdt::Error),
    /// A dictionary entry could not be translated to an RDF term.
    Term(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error reading HDT file: {e}"),
            Error::Hdt(e) => write!(f, "error decoding HDT archive: {e}"),
            Error::Term(e) => write!(f, "invalid term in HDT dictionary: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::Hdt(e) => Some(e),
            Error::Term(_) => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<hdt::hdt::Error> for Error {
    fn from(e: hdt::hdt::Error) -> Self {
        Error::Hdt(e)
    }
}

/// The compression container an HDT byte stream is wrapped in, recognised by its
/// leading MAGIC BYTES (never the file name). `None` is a bare `$HDT` archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Container {
    /// gzip (`.hdt.gz`), magic `1f 8b`.
    Gzip,
    /// zstd (`.hdt.zst`), magic `28 b5 2f fd`.
    Zstd,
    /// bzip2 (`.hdt.bz2`), magic `42 5a 68` ("BZh").
    Bzip2,
    /// Uncompressed HDT (or any other content — let the decoder reject it).
    None,
}

/// The longest magic prefix we need to classify a container (zstd's `28 b5 2f
/// fd`). gzip is 2, bzip2 3, zstd 4 — reading 4 covers all of them.
const MAGIC_PREFIX_LEN: usize = 4;

/// Reads up to [`MAGIC_PREFIX_LEN`] bytes off the front of `reader` into an owned
/// buffer and classifies the container by its magic bytes.
///
/// roborev 2272: a streaming [`BufRead`] may legally return fewer
/// bytes than requested per `fill_buf()` — even a single byte — so peeking the
/// buffer once (`buf.len() >= N`) can misclassify a real gzip/zstd/bzip2 stream
/// as bare HDT and fail the load. We instead drain a fixed-size prefix robustly
/// (looping `read` until EOF or the buffer is full, the same contract as
/// `read_exact` but tolerant of a short stream), and the caller chains that
/// owned prefix back onto the remaining reader for EVERY path — so detection
/// never depends on how the underlying reader chunks its data.
fn sniff_prefix<R: BufRead>(reader: &mut R) -> std::io::Result<(Vec<u8>, Container)> {
    // `BufRead: Read`, so `read` is available without an extra import.
    let mut prefix = [0u8; MAGIC_PREFIX_LEN];
    let mut filled = 0;
    // Loop because a single `read` may return fewer bytes than requested even
    // when more data is available (it is not an error). Stop on a 0-length read
    // (genuine EOF) so an HDT shorter than the prefix still classifies correctly.
    while filled < MAGIC_PREFIX_LEN {
        let n = reader.read(&mut prefix[filled..])?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    let prefix = prefix[..filled].to_vec();
    let container = if filled >= 2 && prefix[0] == 0x1f && prefix[1] == 0x8b {
        Container::Gzip
    } else if filled >= 4 && prefix[..4] == [0x28, 0xb5, 0x2f, 0xfd] {
        Container::Zstd
    } else if filled >= 3 && prefix[..3] == [0x42, 0x5a, 0x68] {
        Container::Bzip2
    } else {
        Container::None
    };
    Ok((prefix, container))
}

/// Runs `f` over the (transparently decompressed) HDT byte stream. A stream
/// whose magic bytes identify a `.hdt.gz` / `.hdt.zst` / `.hdt.bz2` container is
/// wrapped in the matching STREAMING decoder — the decompressed `.hdt` is never
/// fully materialized, it is decoded on demand as `f` reads it; anything else is
/// passed through unchanged. Detection is by CONTENT, not file name.
fn with_hdt_stream<T>(
    reader: impl BufRead,
    f: impl FnOnce(&mut dyn BufRead) -> Result<T, Error>,
) -> Result<T, Error> {
    // roborev 2272: consume a fixed magic-byte prefix robustly, then
    // chain it back so the decoder/HDT reader sees the full, unmodified stream —
    // both the compressed and the bare-HDT path. `Read::chain` re-prepends the
    // owned prefix; we wrap the chain in a `BufReader` to satisfy the `BufRead`
    // bound the decoders need (the prefix is at most 4 bytes, so this adds no
    // meaningful buffering cost on the bare-HDT path).
    let mut reader = reader;
    let (prefix, container) = sniff_prefix(&mut reader)?;
    let stream = std::io::Read::chain(std::io::Cursor::new(prefix), reader);
    let stream = std::io::BufReader::new(stream);
    match container {
        // `MultiGzDecoder` / `MultiBzDecoder` already buffer per the `bufread`
        // input, but their `Read` output needs a `BufRead` for the HDT reader;
        // zstd's `Decoder` likewise. One wrapping `BufReader` each.
        Container::Gzip => {
            let mut d = std::io::BufReader::new(flate2::bufread::MultiGzDecoder::new(stream));
            f(&mut d)
        }
        Container::Zstd => {
            // `zstd::stream::read::Decoder::with_buffer` takes a `BufRead` source
            // directly (no inner re-buffer); its output is wrapped once.
            let dec = zstd::stream::read::Decoder::with_buffer(stream).map_err(Error::Io)?;
            let mut d = std::io::BufReader::new(dec);
            f(&mut d)
        }
        Container::Bzip2 => {
            let mut d = std::io::BufReader::new(bzip2::bufread::MultiBzDecoder::new(stream));
            f(&mut d)
        }
        Container::None => {
            let mut d = stream;
            f(&mut d)
        }
    }
}

/// Loads an HDT archive from a file path into a sparq [`Graph`].
///
/// Supports the standard HDT v1.0 layout (FourSectionDictionary with Plain Front
/// Coding + BitmapTriples in SPO order) as written by hdt-cpp, hdt-java and the
/// `hdt` crate; GZipped containers (`.hdt.gz`) are detected by magic bytes and
/// decompressed on the fly. HDT carries a single graph, so the result has no
/// named graphs.
pub fn load(path: impl AsRef<Path>) -> Result<Graph, Error> {
    let file = std::fs::File::open(path)?;
    load_reader(std::io::BufReader::new(file))
}

/// Loads an HDT archive from any buffered reader into a sparq [`Graph`].
///
/// The reader must be positioned at the start of the HDT data: the `$HDT`
/// cookie, or the gzip magic of a compressed container (decompressed
/// transparently, as in [`load`]).
pub fn load_reader<R: BufRead>(reader: R) -> Result<Graph, Error> {
    // Default path is the direct decoder (H1–H4): it never builds the
    // upstream wavelet matrix / OP-index that a one-shot SPO load throws away.
    with_hdt_stream(reader, |r| decode::graph_from_reader(r))
}

/// Loads only the triples matching `pattern` from an HDT archive.
///
/// Filtering happens during the one-shot SPO walk, before the result's
/// dictionary and permutation indexes are built. Consequently, terms used only
/// by rejected triples are not interned into the returned [`Graph`]. An
/// all-wildcard pattern is exactly equivalent to [`load_reader`]. Compressed HDT
/// containers are detected and streamed as in [`load_reader`].
///
/// # Example
///
/// ```no_run
/// use oxrdf::NamedNode;
/// use sparq_hdt::TriplePattern;
///
/// let predicate = NamedNode::new("http://xmlns.com/foaf/0.1/knows")?;
/// let pattern: TriplePattern = (None, Some(predicate), None);
/// let file = std::io::BufReader::new(std::fs::File::open("dataset.hdt")?);
/// let graph = sparq_hdt::load_reader_filtered(file, &pattern)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[cfg(feature = "load-filter")]
pub fn load_reader_filtered<R: BufRead>(
    reader: R,
    pattern: &TriplePattern,
) -> Result<Graph, Error> {
    // Preserve the existing loader byte-for-byte for the wildcard identity case.
    if pattern.0.is_none() && pattern.1.is_none() && pattern.2.is_none() {
        return load_reader(reader);
    }
    with_hdt_stream(reader, |r| decode::graph_from_reader_filtered(r, pattern))
}

/// Loads via the WRAPPED `hdt` crate's full `Hdt::read` (which builds the query
/// indexes) and the per-id `id_to_string` translation — the path the direct
/// decoder ([`load_reader`]) replaces. Kept as the in-process differential oracle
/// for the round-trip tests and for callers that need an [`Hdt`] for other reasons.
pub fn load_reader_via_upstream<R: BufRead>(reader: R) -> Result<Graph, Error> {
    with_hdt_stream(reader, |r| graph_from_hdt(&Hdt::read(r)?))
}

/// Reads ONLY the HDT header — the dataset-metadata triples every archive
/// carries (the "H" in HDT: VoID statistics, format/provenance notes) — as a
/// queryable sparq [`Graph`], without decoding the dictionary or triples
/// sections. GZipped containers are handled as in [`load`].
///
/// (The wrapped `hdt` crate decodes the header during `Hdt::read` but keeps it
/// private, so this re-reads the head of the stream — it is a few KB.)
pub fn header(path: impl AsRef<Path>) -> Result<Graph, Error> {
    let file = std::fs::File::open(path)?;
    header_reader(std::io::BufReader::new(file))
}

/// [`header`] from any buffered reader positioned at the start of the HDT data.
pub fn header_reader<R: BufRead>(reader: R) -> Result<Graph, Error> {
    with_hdt_stream(reader, |mut r| {
        ControlInfo::read(&mut r).map_err(hdt::hdt::Error::from)?;
        let header = Header::read(&mut r).map_err(hdt::hdt::Error::from)?;
        // The in-crate Header stores parsed triples whose Display is their
        // N-Triples line (Header::write round-trips through it): render and
        // hand them to sparq's own N-Triples loader.
        let mut nt = String::new();
        for triple in &header.body {
            nt.push_str(&triple.to_string());
            nt.push('\n');
        }
        Graph::load_str(&nt, "ntriples").map_err(Error::Term)
    })
}

/// Cardinalities stored in an HDT archive's dictionary and triples sections.
///
/// These values are read directly from the validated HDT metadata without
/// decoding dictionary strings, materializing triples, or building indexes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HdtStats {
    /// The exact number of triples in the archive.
    pub triples: usize,
    /// Terms used in both subject and object position.
    pub shared: usize,
    /// Terms used only in subject position.
    pub subjects_only: usize,
    /// Terms used only in object position.
    pub objects_only: usize,
    /// Distinct predicates.
    pub predicates: usize,
}

impl HdtStats {
    /// Returns the number of distinct subjects.
    pub const fn distinct_subjects(&self) -> usize {
        self.shared + self.subjects_only
    }

    /// Returns the number of distinct objects.
    pub const fn distinct_objects(&self) -> usize {
        self.shared + self.objects_only
    }
}

/// Reads HDT dictionary cardinalities and the exact triple count from a file.
///
/// This validates the header, dictionary, and triples-section checksums but does
/// not decode dictionary strings, materialize triples, or build HDT query indexes.
/// Compressed containers are detected and streamed as in [`load`].
pub fn stats(path: impl AsRef<Path>) -> Result<HdtStats, Error> {
    let file = std::fs::File::open(path)?;
    stats_reader(std::io::BufReader::new(file))
}

/// [`stats`] from any buffered reader positioned at the start of the HDT data.
pub fn stats_reader<R: BufRead>(reader: R) -> Result<HdtStats, Error> {
    with_hdt_stream(reader, |r| decode::stats_from_reader(r))
}

// sq-obhf1: filtered count parity with `load_reader_filtered`.
/// Counts triples matching `pattern` without constructing a result [`Graph`].
///
/// Matching uses the same one-shot SPO walk and resolved [`TriplePattern`] as
/// [`load_reader_filtered`]. The returned [`HdtStats::triples`] is the number of
/// matches; its dictionary cardinalities still describe the complete archive.
/// An all-wildcard pattern is exactly equivalent to [`stats_reader`]. Compressed
/// containers are detected and streamed as in [`load_reader`].
#[cfg(feature = "load-filter")]
pub fn stats_reader_filtered<R: BufRead>(
    reader: R,
    pattern: &TriplePattern,
) -> Result<HdtStats, Error> {
    if pattern.0.is_none() && pattern.1.is_none() && pattern.2.is_none() {
        return stats_reader(reader);
    }
    with_hdt_stream(reader, |r| decode::stats_from_reader_filtered(r, pattern))
}

/// Converts an already-decoded [`hdt::Hdt`] into a sparq [`Graph`] — the seam for
/// callers that hold an `Hdt` (e.g. to also query its header metadata).
pub fn graph_from_hdt(hdt: &Hdt) -> Result<Graph, Error> {
    // HDT id spaces (all 1-based):
    //   subject id  s: 1..=n_shared          -> shared section (terms used as S and O)
    //               s: n_shared+1..=n_subj   -> subject-only section
    //   predicate p: 1..=n_pred              -> predicate section (own numbering)
    //   object id  o: 1..=n_shared           -> shared section (same terms as subject ids)
    //              o: n_shared+1..=n_obj     -> object-only section
    let n_shared = hdt.dict.shared.num_strings;
    let n_subj = n_shared + hdt.dict.subjects.num_strings;
    let n_pred = hdt.dict.predicates.num_strings;
    let n_obj_only = hdt.dict.objects.num_strings;

    let mut dict = Dict::new();
    // Memo tables: HDT id -> sparq id, 0 = not yet translated (sparq ids are never 0).
    // Object ids in the shared range reuse `subj` entries, so each shared term is
    // decompressed + interned exactly once even when it appears in both positions.
    let mut subj: Vec<Id> = vec![0; n_subj + 1];
    let mut pred: Vec<Id> = vec![0; n_pred + 1];
    let mut obj: Vec<Id> = vec![0; n_obj_only + 1]; // indexed by (o - n_shared)

    let mut triples: Vec<[Id; 3]> = Vec::with_capacity(hdt.triples.adjlist_z.sequence.entries);
    for [s, p, o] in &hdt.triples {
        let sid = match subj[s] {
            0 => {
                let id = translate(&mut dict, hdt, s, IdKind::Subject)?;
                subj[s] = id;
                id
            }
            id => id,
        };
        let pid = match pred[p] {
            0 => {
                let id = translate(&mut dict, hdt, p, IdKind::Predicate)?;
                pred[p] = id;
                id
            }
            id => id,
        };
        // Shared-section object ids denote the same term as the equal subject id.
        let oid = if o <= n_shared {
            match subj[o] {
                0 => {
                    let id = translate(&mut dict, hdt, o, IdKind::Object)?;
                    subj[o] = id;
                    id
                }
                id => id,
            }
        } else {
            match obj[o - n_shared] {
                0 => {
                    let id = translate(&mut dict, hdt, o, IdKind::Object)?;
                    obj[o - n_shared] = id;
                    id
                }
                id => id,
            }
        };
        triples.push([sid, pid, oid]);
    }

    Ok(Graph::from_parts(dict, triples))
}

/// Decompresses one HDT dictionary entry and interns it into the sparq dictionary.
fn translate(dict: &mut Dict, hdt: &Hdt, id: usize, kind: IdKind) -> Result<Id, Error> {
    let s = hdt
        .dict
        .id_to_string(id, kind)
        .map_err(|e| Error::Term(e.to_string()))?;
    intern_hdt_term(dict, &s)
}

const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";
const RDF_LANG_STRING: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString";

/// Interns an HDT dictionary string as a sparq term.
///
/// HDT dictionary encoding (per the spec's dictionary section and the reference
/// implementations): IRIs are stored bare (no angle brackets), blank nodes as
/// `_:label`, and literals in N-Triples-like shape — `"lexical"`,
/// `"lexical"@lang`, or `"lexical"^^<datatype>` — with the lexical form stored
/// raw (NOT N-Triples-escaped).
pub(crate) fn intern_hdt_term(dict: &mut Dict, s: &str) -> Result<Id, Error> {
    if let Some(rest) = s.strip_prefix('"') {
        // Literal. The lexical form may itself contain '"', so split at the LAST
        // quote (the lang tag / datatype suffix cannot contain one) — same rule as
        // the reference readers.
        let end = rest
            .rfind('"')
            .ok_or_else(|| Error::Term(format!("unterminated literal: {s}")))?;
        let lex = &rest[..end];
        let suffix = &rest[end + 1..];
        if suffix.is_empty() {
            return Ok(dict.intern_lit(lex, XSD_STRING, None));
        }
        if let Some(tag) = suffix.strip_prefix('@') {
            // BCP47 tags are case-insensitive; oxrdf (and thus every other sparq
            // loader) normalizes to lowercase, so do the same for round-trip equality.
            let tag = tag.to_ascii_lowercase();
            return Ok(dict.intern_lit(lex, RDF_LANG_STRING, Some(&tag)));
        }
        if let Some(dt) = suffix.strip_prefix("^^") {
            // hdt-cpp / hdt-java / the hdt crate store the datatype as <iri>;
            // tolerate a bare IRI too.
            let dt = dt
                .strip_prefix('<')
                .and_then(|d| d.strip_suffix('>'))
                .unwrap_or(dt);
            if dt.is_empty() {
                return Err(Error::Term(format!("empty datatype in literal: {s}")));
            }
            return Ok(dict.intern_lit(lex, dt, None));
        }
        return Err(Error::Term(format!("malformed literal suffix: {s}")));
    }
    if let Some(label) = s.strip_prefix("_:") {
        return Ok(dict.intern_blank(label));
    }
    if s.is_empty() {
        return Err(Error::Term("empty dictionary entry".into()));
    }
    Ok(dict.intern_iri(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;

    /// sq-cafc: the public [`Error`] type's `Display` and `source()` are part of
    /// the crate's error contract (callers print and chain these) but were never asserted.
    /// Each of the three variants must render its documented prefix, and `source()` must
    /// expose the wrapped cause for the I/O variant and report `None` for the leaf `Term`
    /// variant — so a `?`-chained caller can walk the cause chain.
    #[test]
    fn error_display_and_source_chain() {
        // `Io`: renders the I/O prefix and EXPOSES the wrapped error as its `source`.
        let io = Error::Io(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "boom",
        ));
        assert!(
            io.to_string().starts_with("I/O error reading HDT file:"),
            "Io Display prefix, got: {io}"
        );
        assert!(io.source().is_some(), "Io must expose its wrapped cause");

        // `Term`: renders the dictionary-term prefix and is a LEAF (no source).
        let term = Error::Term("bad term".to_owned());
        assert_eq!(term.to_string(), "invalid term in HDT dictionary: bad term");
        assert!(term.source().is_none(), "Term is a leaf error");

        // `From<std::io::Error>` builds the `Io` variant (the `?` conversion path).
        let converted: Error =
            std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied").into();
        assert!(matches!(converted, Error::Io(_)));
        assert!(converted.to_string().contains("denied"));
    }

    /// The `Hdt` variant's `Display` + `source()` (the wrapped `hdt` decode error) are
    /// reached on any malformed archive. Drive a real decode failure (random bytes) and,
    /// when it surfaces as the `Hdt` variant, assert its prefix + that it chains a source.
    #[test]
    fn hdt_error_variant_displays_and_chains() {
        let Err(err) = load_reader(std::io::Cursor::new(
            b"$HDT\x01 not really an archive".to_vec(),
        )) else {
            panic!("garbage after the cookie must fail to decode");
        };
        if let Error::Hdt(_) = err {
            assert!(
                err.to_string().starts_with("error decoding HDT archive:"),
                "Hdt Display prefix, got: {err}"
            );
            assert!(err.source().is_some(), "Hdt must expose its wrapped cause");
        }
        // (If the failure classified as Io/Term instead, those variants are covered by
        // `error_display_and_source_chain`; the point here is to EXERCISE the Hdt arm,
        // which a corrupt control-info read reaches.)
    }

    /// The dictionary-string parser must map every HDT term shape onto the term
    /// sparq's own loaders would produce (checked via the N-Triples rendering).
    #[test]
    fn term_shapes() {
        let mut dict = Dict::new();
        for (hdt_str, term_str) in [
            ("http://e.org/s", "<http://e.org/s>"),
            ("\"plain\"", "\"plain\""),
            ("\"hallo\"@de-DE", "\"hallo\"@de-de"),
            (
                "\"3.14\"^^<http://www.w3.org/2001/XMLSchema#double>",
                "\"3.14\"^^<http://www.w3.org/2001/XMLSchema#double>",
            ),
            ("_:b1", "_:b1"),
        ] {
            let id = intern_hdt_term(&mut dict, hdt_str).unwrap();
            assert_eq!(dict.term(id).to_string(), term_str);
        }
        // Lexical forms containing quotes split at the LAST quote.
        let id = intern_hdt_term(&mut dict, "\"a\"b\"@en").unwrap();
        assert_eq!(dict.term(id).to_string(), "\"a\\\"b\"@en");
        // Bare (bracket-less) datatype IRIs are tolerated.
        let a = intern_hdt_term(&mut dict, "\"x\"^^http://e.org/dt").unwrap();
        let b = intern_hdt_term(&mut dict, "\"x\"^^<http://e.org/dt>").unwrap();
        assert_eq!(a, b);
        // Malformed entries are reported, not panicked on.
        assert!(intern_hdt_term(&mut dict, "\"open").is_err());
        assert!(intern_hdt_term(&mut dict, "\"x\"^^<>").is_err());
        // sq-bif: the BARE (bracket-less) empty datatype `"x"^^` is the
        // distinct sibling of the bracketed-empty `"x"^^<>` above: the `strip_prefix('<')`
        // does NOT fire, so `dt` is the empty string straight from `^^`, and the
        // `dt.is_empty()` guard must still reject it (an empty datatype IRI is never a
        // valid literal). Without this, only the bracketed branch of the empty-datatype
        // check was exercised.
        assert!(
            intern_hdt_term(&mut dict, "\"x\"^^").is_err(),
            "a bare empty datatype `\"x\"^^` must be rejected, like the bracketed `\"x\"^^<>`"
        );
        assert!(intern_hdt_term(&mut dict, "\"x\"!!").is_err());
        assert!(intern_hdt_term(&mut dict, "").is_err());
    }

    // roborev 2272 regression: a `BufRead` that hands back exactly
    // ONE byte per `fill_buf()`/`read()` (a legal streaming reader) must not
    // defeat magic-byte container detection, and the consumed prefix must be
    // chained back so the downstream reader sees the full, unmodified stream.
    struct OneByteAtATime<'a> {
        data: &'a [u8],
        pos: usize,
    }

    impl std::io::Read for OneByteAtATime<'_> {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            if self.pos >= self.data.len() || out.is_empty() {
                return Ok(0);
            }
            out[0] = self.data[self.pos];
            self.pos += 1;
            Ok(1) // never more than one byte, even when more is available
        }
    }

    impl BufRead for OneByteAtATime<'_> {
        fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
            // Expose at most ONE byte of the available buffer — the worst case a
            // streaming reader may legally present.
            let end = (self.pos + 1).min(self.data.len());
            Ok(&self.data[self.pos..end])
        }
        fn consume(&mut self, amt: usize) {
            self.pos = (self.pos + amt).min(self.data.len());
        }
    }

    #[test]
    fn one_byte_at_a_time_detects_containers() {
        // gzip / zstd / bzip2 magic + a bare-HDT head must all classify the same
        // way they would if delivered in one chunk, even at 1 byte per read.
        for (bytes, want) in [
            (vec![0x1f, 0x8b, 0x00, 0x00], Container::Gzip),
            (vec![0x28, 0xb5, 0x2f, 0xfd], Container::Zstd),
            (vec![0x42, 0x5a, 0x68, 0x39], Container::Bzip2),
            (b"$HDT".to_vec(), Container::None),
        ] {
            let mut r = OneByteAtATime {
                data: &bytes,
                pos: 0,
            };
            let (prefix, got) = sniff_prefix(&mut r).unwrap();
            assert_eq!(got, want, "classification for {bytes:02x?}");
            // The whole 4-byte prefix is preserved (none dropped or duplicated).
            assert_eq!(prefix, &bytes[..MAGIC_PREFIX_LEN.min(bytes.len())]);
        }
    }

    #[test]
    fn prefix_is_chained_back_for_passthrough() {
        // A bare-HDT stream delivered 1 byte at a time must round-trip BYTE-FOR-BYTE
        // through `with_hdt_stream` (the prefix is re-prepended, not lost).
        let payload = b"$HDT then some more dictionary/triples bytes here".to_vec();
        let r = OneByteAtATime {
            data: &payload,
            pos: 0,
        };
        let round_tripped = with_hdt_stream(r, |reader| {
            let mut v = Vec::new();
            std::io::Read::read_to_end(reader, &mut v).map_err(Error::Io)?;
            Ok(v)
        })
        .unwrap();
        assert_eq!(round_tripped, payload);
    }

    #[test]
    fn short_stream_below_prefix_len_classifies_as_none() {
        // A stream shorter than the magic prefix must not hang or misclassify.
        for bytes in [vec![], vec![0x24], vec![0x24, 0x48], vec![0x24, 0x48, 0x44]] {
            let mut r = OneByteAtATime {
                data: &bytes,
                pos: 0,
            };
            let (prefix, got) = sniff_prefix(&mut r).unwrap();
            assert_eq!(got, Container::None);
            assert_eq!(prefix, bytes);
        }
    }
}
