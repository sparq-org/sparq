//! sq-2te / sq-ashy — HDT WRITE support: serialise a sparq [`Graph`] to
//! an `.hdt` archive (the mirror of [`crate::load`]).
//!
//! ## The write path (direct in-memory encode — no N-Triples round-trip)
//!
//! [`save`] encodes the HDT sections DIRECTLY from sparq's in-memory dictionary +
//! triple ids via [`crate::encode::write_hdt`] — the inverse of [`crate::decode`].
//! It builds the FourSectionDictionary (Plain Front Coding) and the BitmapTriples
//! (SPO) from the data sparq already holds and writes them in the exact byte layout
//! [`hdt::Hdt::write`] produces.
//!
//! ### What this replaced (bead sq-ashy)
//!
//! The original path (bead sq-2te, the LOW/EASY route) round-tripped through a
//! TEMPORARY N-Triples file: it rendered the whole graph as N-Triples text on disk,
//! then re-parsed it through [`hdt::Hdt::read_nt`] (re-interning every term and
//! re-sorting the triples sparq already had) before serialising. That materialised
//! the whole graph as text and re-did sparq's ingest work; the direct encoder skips
//! both the temp file and the text parse. The upstream-backed reader
//! ([`crate::load_reader_via_upstream`]) is kept as the differential oracle the
//! write round-trip tests check the direct encoder against.
//!
//! Output containers (`.hdt.gz` / `.hdt.zst` / `.hdt.bz2`) are chosen by the OUTPUT
//! path's extension (the write side cannot sniff content that does not exist yet, so
//! it honours the file name the caller chose); the encoder streams its bytes through
//! the matching compressing writer — the bare `.hdt` is never fully materialised.

use crate::Error;
use sparq_core::Graph;
use std::io::Write;
use std::path::Path;

/// The compression container to wrap the written `.hdt` bytes in, chosen by the
/// OUTPUT path's extension (the write side, unlike the read side, cannot sniff
/// content that does not exist yet — so it honours the file name the caller chose).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutContainer {
    /// `.hdt.gz`
    Gzip,
    /// `.hdt.zst`
    Zstd,
    /// `.hdt.bz2`
    Bzip2,
    /// bare `.hdt` (any other extension)
    None,
}

/// Picks the output container from the path's final extension. Recognises the same
/// `.gz` / `.zst` / `.bz2` suffixes the reader sniffs by magic bytes; anything else
/// (including a bare `.hdt`) writes an uncompressed archive.
fn out_container(path: &Path) -> OutContainer {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("gz") => OutContainer::Gzip,
        Some("zst") | Some("zstd") => OutContainer::Zstd,
        Some("bz2") => OutContainer::Bzip2,
        _ => OutContainer::None,
    }
}

/// Saves a sparq [`Graph`] as an HDT archive at `path`.
///
/// The on-disk layout is the standard HDT v1.0 (FourSectionDictionary with Plain
/// Front Coding + BitmapTriples in SPO order) — the same layout [`crate::load`]
/// reads, and the one hdt-cpp / hdt-java emit, so the result is interoperable. The
/// sections are encoded DIRECTLY from sparq's in-memory dictionary + triples (no
/// temporary N-Triples file, no text re-parse — see `crate::encode`). If `path`
/// ends in `.gz` / `.zst` / `.bz2` the archive bytes are wrapped in that container
/// (matching the reader's magic-byte sniffing); any other extension writes a bare
/// `.hdt`.
///
/// HDT carries a SINGLE default graph, so only the default graph's triples are
/// written; any named graphs on `graph` are ignored (HDT has no place for them).
///
/// Requires the `write` feature. Returns [`Error::Term`] for a graph containing an
/// RDF 1.2 triple term, which standard HDT cannot represent.
pub fn save(graph: &Graph, path: impl AsRef<Path>) -> Result<(), Error> {
    let path = path.as_ref();
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    match out_container(path) {
        OutContainer::None => {
            let mut w = file;
            crate::encode::write_hdt(graph, &mut w)?;
            w.flush()?;
        }
        OutContainer::Gzip => {
            let mut w = flate2::write::GzEncoder::new(file, flate2::Compression::default());
            crate::encode::write_hdt(graph, &mut w)?;
            w.finish()?.flush()?;
        }
        OutContainer::Zstd => {
            let mut w = zstd::stream::write::Encoder::new(file, 3)
                .map_err(Error::Io)?
                .auto_finish();
            crate::encode::write_hdt(graph, &mut w)?;
            w.flush()?;
        }
        OutContainer::Bzip2 => {
            let mut w = bzip2::write::BzEncoder::new(file, bzip2::Compression::default());
            crate::encode::write_hdt(graph, &mut w)?;
            w.finish()?.flush()?;
        }
    }
    Ok(())
}
