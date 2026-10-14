//! (sq-o5bi) ONLINE consistent snapshot backup + restore for the serving
//! store — exporting an already-immutable pinned [`Generation`](crate::Generation) to a
//! single self-describing artifact WHILE SERVING, and re-hydrating a ring/writer from one.
//!
//! ## What this is (and what it is NOT)
//!
//! This is the **online** path: a backup reads from a generation the ring already pinned
//! (an `Arc<Generation<Graph>>`), so it does **not** stop the world — readers keep loading
//! [`GenerationRing::current`](crate::GenerationRing::current) lock-free and the writer
//! keeps publishing forward; the snapshot a backup serialises is frozen by its `Arc`, not by
//! a lock. It is **distinct** from the offline `sparq-cli save` (stop-the-world, full index
//! rebuild into a directory) and from the per-graph write-ahead log (the `--persist`
//! durability path): neither captures a *consistent point-in-time generation of the live
//! serving store* into one portable artifact.
//!
//! ## Artifact format — Option A (single self-describing file)
//!
//! v1 follows the Stardog backup-ID model: ONE artifact is the unit of backup/restore. The
//! file is a small textual header (the recorded *generation number = writer seq*, the
//! per-pod epoch vectors, the triple count, and an integrity digest of the body) followed by
//! the full dataset serialised as **N-Quads** (default graph + every named graph — the
//! self-describing, lossless, format-stable body that round-trips through
//! `Graph::load_dataset`). Choosing a self-describing body (over reusing the WAL-backed
//! directory layout) is deliberate: the artifact does not couple to the on-disk persisted
//! layout, so it stays a stable interchange + the base half of the future change-stream /
//! point-in-time-recovery companions.
//!
//! **Out of scope (a separate concern):** at-rest encryption of the artifact. The integrity
//! digest here is a NON-cryptographic checksum — it detects truncation/corruption/version
//! mismatch (fail-closed on restore), NOT tampering, and is not an authenticity or
//! confidentiality guarantee.
//!
//! ## Fail-closed restore
//!
//! `import` rejects — never partially loads — an artifact whose magic, version, header
//! shape or body digest does not check out. A corrupt or mismatched artifact yields a
//! [`BackupError`], so a restore-on-start that fails leaves the operator with a clean error
//! rather than a silently-wrong store.

use std::io::{self, BufRead, Read, Write};

use sparq_core::Graph;
use sparq_engine::serialize::graph_to_nquads;

use crate::epoch::{Epoch, PodEpochs, PodId};
use crate::ring::Generation;

/// Magic line that opens every sparq backup artifact. Distinguishes the artifact from a
/// bare N-Quads file (which would otherwise look like a valid body) and lets `import`
/// fail closed on a non-artifact input before it spends any work parsing RDF.
const MAGIC: &str = "SPARQ-BACKUP";

/// Artifact format version. v1 is the Stardog-style single self-describing file. A future
/// change-stream/PITR companion bumps this; `import` rejects an unknown version (fail-closed)
/// rather than mis-reading a newer layout.
const FORMAT_VERSION: u32 = 1;

/// The consistent point-in-time metadata captured ALONGSIDE the triples in a backup: the
/// recorded generation number (= the single writer's sequence — the read-your-writes token),
/// the per-pod epoch vectors at that generation, and the triple count. This is exactly the
/// "pinned generation (triples + dict + per-pod epoch vectors + writer seq)" the artifact
/// records so a restore re-establishes the same serving identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackupMeta {
    /// The generation number the backup was taken at — the single sequenced writer's seq, and
    /// the read-your-writes token a client round-trips. A restored ring is re-seeded so its
    /// generation 0 *carries* these epochs (see `import`); the recorded number lets an
    /// operator/PITR layer correlate the artifact with the writer history it came from.
    pub generation: u64,
    /// The per-pod epoch vector as of the backed-up generation. Restored verbatim onto the
    /// re-seeded ring's first generation so cache-invalidation epochs are continuous across a
    /// restore (a pod's epoch never appears to go backwards).
    pub epochs: PodEpochs,
    /// The number of triples in the body (default graph + named graphs). Cross-checked on
    /// import against the actual decoded count — a mismatch fails the restore closed.
    pub triples: u64,
}

/// Why a backup export or restore failed.
#[derive(Debug)]
pub enum BackupError {
    /// An I/O error reading or writing the artifact stream.
    Io(io::Error),
    /// The artifact is not a sparq backup (bad/absent magic), is a version this build does
    /// not understand, or its header is malformed — the fail-closed cases on restore.
    Format(String),
    /// The body did not match the header (integrity digest or triple-count mismatch), or the
    /// RDF body failed to parse — a corrupt/truncated artifact. Restore is rejected.
    Corrupt(String),
}

impl std::fmt::Display for BackupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // positional format args (CodeQL rust/unused-variable false-positive).
            BackupError::Io(e) => write!(f, "backup I/O error: {}", e),
            BackupError::Format(m) => write!(f, "backup format error: {}", m),
            BackupError::Corrupt(m) => write!(f, "backup corrupt: {}", m),
        }
    }
}

impl std::error::Error for BackupError {}

impl From<io::Error> for BackupError {
    fn from(e: io::Error) -> Self {
        BackupError::Io(e)
    }
}

/// Total quad count of a [`Graph`] = the default-graph triples ([`Graph::len`], which counts
/// ONLY the default graph) plus every named graph's triples. This is the count the N-Quads
/// body has one line for, so it is what `triples` records and cross-checks on restore.
/// `pub(crate)` so the delta companion (`backup_delta`) cross-checks a replayed graph's quad
/// count the same way.
pub(crate) fn total_quads(graph: &Graph) -> u64 {
    let mut n = graph.len() as u64;
    for (_name, g) in &graph.named {
        n += g.len() as u64;
    }
    n
}

/// A small, self-contained, stable 64-bit FNV-1a digest of the body bytes. Stable across
/// builds/runs/platforms (unlike `std::hash::DefaultHasher`, which is intentionally
/// build-randomised), with NO external dependency — exactly what an integrity check across
/// two separate processes needs. NOT cryptographic: it detects accidental
/// corruption/truncation, not tampering (at-rest authenticity is out of scope — see the
/// module docs).
///
/// `pub(crate)` so the delta-stream companion (`backup_delta`) digests its insert / delete
/// bodies with the SAME function — one integrity scheme across the whole backup family.
pub(crate) fn body_digest(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = OFFSET;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(PRIME);
    }
    h
}

/// Exports the pinned `generation` to `out` as a single self-describing backup artifact
/// (the Option-A format described in the module docs). The generation is already immutable
/// (held by its `Arc`), so this serialises a consistent point-in-time snapshot **without
/// stopping the world**: nothing here touches the ring or the writer, and the caller pins
/// the generation lock-free.
///
/// The body is the full dataset (default + named graphs) as N-Quads; the header records the
/// generation/writer-seq, the per-pod epoch vectors, the triple count and the body digest.
pub fn export<W: Write>(generation: &Generation<Graph>, out: &mut W) -> Result<(), BackupError> {
    let graph = generation.snapshot();
    let body = graph_to_nquads(graph);
    let body_bytes = body.as_bytes();
    let digest = body_digest(body_bytes);

    // Header: one `key value` line per field, terminated by a blank line, then the body.
    // Line-oriented + textual so it is trivially inspectable and version-stable.
    writeln!(out, "{} {}", MAGIC, FORMAT_VERSION)?;
    writeln!(out, "generation {}", generation.number())?;
    writeln!(out, "triples {}", total_quads(graph))?;
    writeln!(out, "body-bytes {}", body_bytes.len())?;
    writeln!(out, "body-fnv1a64 {:016x}", digest)?;
    // Epoch vector: one `epoch <count>` count line, then `epoch-entry <epoch> <pod-iri>` per
    // touched pod. The pod IRI is last so it is read verbatim to end-of-line (graph IRIs are
    // whitespace-free, so the leading-space split is unambiguous).
    let epochs = generation.epochs();
    writeln!(out, "epoch {}", epochs.len())?;
    for (pod, epoch) in epochs.iter() {
        writeln!(out, "epoch-entry {} {}", epoch, pod.as_str())?;
    }
    writeln!(out)?; // blank line: end of header
    out.write_all(body_bytes)?;
    out.flush()?;
    Ok(())
}

/// Imports a backup artifact from `input`, returning the rehydrated [`Graph`] (the restored
/// store, ready to seed a ring) and its [`BackupMeta`]. **Fail-closed**: a bad magic, an
/// unknown version, a malformed header, a body whose digest or triple count does not match,
/// or an unparseable RDF body all yield a [`BackupError`] and load NOTHING — the caller is
/// never handed a silently-partial or silently-wrong store.
pub fn import<R: Read>(input: R) -> Result<(Graph, BackupMeta), BackupError> {
    let mut reader = io::BufReader::new(input);

    // --- Header (line-oriented, blank-line terminated) ---
    let first = read_line(&mut reader)?;
    let mut parts = first.splitn(2, ' ');
    let magic = parts.next().unwrap_or("");
    if magic != MAGIC {
        return Err(BackupError::Format(format!(
            "not a sparq backup artifact (expected magic {:?})",
            MAGIC
        )));
    }
    let version: u32 = parts
        .next()
        .and_then(|v| v.trim().parse().ok())
        .ok_or_else(|| BackupError::Format("missing/invalid format version".into()))?;
    if version != FORMAT_VERSION {
        return Err(BackupError::Format(format!(
            "unsupported backup format version {} (this build reads/writes {})",
            version, FORMAT_VERSION
        )));
    }

    let mut generation: Option<u64> = None;
    let mut triples: Option<u64> = None;
    let mut body_bytes_len: Option<usize> = None;
    let mut body_digest_hdr: Option<u64> = None;
    let mut epoch_count: Option<usize> = None;
    let mut epochs = PodEpochs::default();
    let mut epoch_entries: usize = 0;

    loop {
        let line = read_line(&mut reader)?;
        if line.is_empty() {
            break; // blank line: end of header
        }
        let mut kv = line.splitn(2, ' ');
        let key = kv.next().unwrap_or("");
        let val = kv.next().unwrap_or("").trim();
        match key {
            "generation" => generation = Some(parse_u64(val, "generation")?),
            "triples" => triples = Some(parse_u64(val, "triples")?),
            "body-bytes" => {
                body_bytes_len =
                    Some(usize::try_from(parse_u64(val, "body-bytes")?).map_err(|_| {
                        BackupError::Format("body-bytes exceeds platform usize".into())
                    })?)
            }
            "body-fnv1a64" => {
                body_digest_hdr = Some(u64::from_str_radix(val, 16).map_err(|_| {
                    BackupError::Format("invalid body-fnv1a64 digest (not 16-hex)".into())
                })?)
            }
            "epoch" => {
                epoch_count = Some(usize::try_from(parse_u64(val, "epoch")?).map_err(|_| {
                    BackupError::Format("epoch count exceeds platform usize".into())
                })?)
            }
            "epoch-entry" => {
                // `epoch-entry <epoch> <pod-iri>` — split off the leading epoch number.
                let mut ev = val.splitn(2, ' ');
                let e: Epoch = ev
                    .next()
                    .and_then(|s| s.parse().ok())
                    .ok_or_else(|| BackupError::Format("invalid epoch-entry epoch".into()))?;
                let pod = ev
                    .next()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| BackupError::Format("missing epoch-entry pod IRI".into()))?;
                epochs.set(PodId::new(pod), e);
                epoch_entries += 1;
            }
            // Unknown header keys are rejected (fail-closed) rather than ignored: an unknown
            // key in a v1-tagged artifact means the writer and reader disagree on the format.
            other => {
                return Err(BackupError::Format(format!(
                    "unknown backup header key {:?}",
                    other
                )))
            }
        }
    }

    let generation =
        generation.ok_or_else(|| BackupError::Format("missing `generation` header".into()))?;
    let triples = triples.ok_or_else(|| BackupError::Format("missing `triples` header".into()))?;
    let body_bytes_len =
        body_bytes_len.ok_or_else(|| BackupError::Format("missing `body-bytes` header".into()))?;
    let body_digest_hdr = body_digest_hdr
        .ok_or_else(|| BackupError::Format("missing `body-fnv1a64` header".into()))?;
    if let Some(n) = epoch_count {
        if n != epoch_entries {
            return Err(BackupError::Format(format!(
                "epoch count {} != {} epoch-entry lines",
                n, epoch_entries
            )));
        }
    }

    // --- Body (exactly `body-bytes` bytes; digest + triple count cross-checked) ---
    let mut body = vec![0u8; body_bytes_len];
    reader
        .read_exact(&mut body)
        .map_err(|_| BackupError::Corrupt("body shorter than declared `body-bytes`".into()))?;
    let actual_digest = body_digest(&body);
    if actual_digest != body_digest_hdr {
        return Err(BackupError::Corrupt(format!(
            "body digest mismatch (header {:016x}, computed {:016x})",
            body_digest_hdr, actual_digest
        )));
    }
    let body_str = String::from_utf8(body)
        .map_err(|_| BackupError::Corrupt("body is not valid UTF-8".into()))?;
    let graph = Graph::load_dataset(&body_str, "nquads")
        .map_err(|e| BackupError::Corrupt(format!("body failed to parse as N-Quads: {}", e)))?;
    let actual_triples = total_quads(&graph);
    if actual_triples != triples {
        return Err(BackupError::Corrupt(format!(
            "triple count mismatch (header {}, decoded {})",
            triples, actual_triples
        )));
    }

    Ok((
        graph,
        BackupMeta {
            generation,
            epochs,
            triples,
        },
    ))
}

/// Reads one trimmed line (without the trailing newline). An EOF mid-header is a format
/// error (the header must be blank-line terminated). `pub(crate)` so the delta companion
/// (`backup_delta`) parses its header with the exact same line discipline.
pub(crate) fn read_line<R: BufRead>(reader: &mut R) -> Result<String, BackupError> {
    let mut s = String::new();
    let n = reader.read_line(&mut s)?;
    if n == 0 {
        return Err(BackupError::Format(
            "unexpected end of artifact in header".into(),
        ));
    }
    // Strip the trailing newline (and a CR before it, if any).
    while s.ends_with('\n') || s.ends_with('\r') {
        s.pop();
    }
    Ok(s)
}

/// Parses a `u64` header value, mapping a parse failure to a [`BackupError::Format`].
/// `pub(crate)` so the delta companion (`backup_delta`) parses its numeric header fields
/// with identical error reporting.
pub(crate) fn parse_u64(val: &str, field: &str) -> Result<u64, BackupError> {
    val.parse()
        .map_err(|_| BackupError::Format(format!("invalid `{}` header value {:?}", field, val)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ring::GenerationRing;

    fn graph_with(nq: &str) -> Graph {
        Graph::load_dataset(nq, "nquads").expect("seed parses")
    }

    /// `Graph` does not implement `Debug`, so `Result::unwrap_err` cannot format the Ok arm.
    /// This extracts the `BackupError` (panicking with a clear message if import unexpectedly
    /// succeeded) without ever needing to `Debug`-format the restored graph.
    fn expect_err(res: Result<(Graph, BackupMeta), BackupError>) -> BackupError {
        match res {
            Ok((_g, _m)) => panic!("expected import to fail closed, but it succeeded"),
            Err(e) => e,
        }
    }

    /// A round-tripped artifact yields an identical queryable generation (same triples).
    #[test]
    fn roundtrip_default_and_named_graphs() {
        let nq = "<http://ex/s> <http://ex/p> <http://ex/o> .\n\
                  <http://ex/s2> <http://ex/p2> \"lit\" <http://ex/g> .\n";
        let g = graph_with(nq);
        let ring: GenerationRing<Graph> = GenerationRing::new(g);
        let gen = ring.current();

        let mut buf = Vec::new();
        export(&gen, &mut buf).expect("export");

        let (restored, meta) = import(&buf[..]).expect("import");
        assert_eq!(meta.generation, 0);
        assert_eq!(meta.triples, 2);
        // Total quads = 1 default-graph + 1 named-graph (Graph::len counts default only).
        assert_eq!(total_quads(&restored), 2);
        assert_eq!(restored.named.len(), 1, "named graph restored");
        // Equivalent queryable content: same triple set serialises identically.
        let mut a = graph_to_nquads(&restored)
            .lines()
            .map(String::from)
            .collect::<Vec<_>>();
        let mut b = graph_to_nquads(gen.snapshot())
            .lines()
            .map(String::from)
            .collect::<Vec<_>>();
        a.sort();
        b.sort();
        assert_eq!(a, b, "restored dataset must equal the source dataset");
    }

    /// The per-pod epoch vector and the generation/writer-seq survive a round trip.
    #[test]
    fn roundtrip_preserves_epochs_and_seq() {
        let g = graph_with("<http://ex/s> <http://ex/p> <http://ex/o> .\n");
        let ring: GenerationRing<Graph> = GenerationRing::new(g);
        // Publish a couple of generations that touch named-graph pods.
        let pod_a = PodId::new("http://ex/g1");
        let pod_b = PodId::new("http://ex/g2");
        ring.publish(
            graph_with("<http://ex/s> <http://ex/p> <http://ex/o> .\n"),
            [pod_a.clone()],
        );
        let gen = ring.publish(
            graph_with("<http://ex/s> <http://ex/p> <http://ex/o> .\n"),
            [pod_a.clone(), pod_b.clone()],
        );
        assert_eq!(gen.number(), 2);
        assert_eq!(gen.epochs().epoch(&pod_a), 2);
        assert_eq!(gen.epochs().epoch(&pod_b), 1);

        let mut buf = Vec::new();
        export(&gen, &mut buf).expect("export");
        let (_g, meta) = import(&buf[..]).expect("import");
        assert_eq!(meta.generation, 2, "writer seq preserved");
        assert_eq!(meta.epochs.epoch(&pod_a), 2, "epoch vector preserved");
        assert_eq!(meta.epochs.epoch(&pod_b), 1, "epoch vector preserved");
    }

    /// Restore is fail-closed on a non-artifact input (bad magic).
    #[test]
    fn import_rejects_non_artifact() {
        let err = expect_err(import(
            &b"<http://ex/s> <http://ex/p> <http://ex/o> .\n"[..],
        ));
        assert!(matches!(err, BackupError::Format(_)), "{:?}", err);
    }

    /// Restore is fail-closed on an unknown format version.
    #[test]
    fn import_rejects_unknown_version() {
        let art = "SPARQ-BACKUP 999\ngeneration 0\ntriples 0\nbody-bytes 0\n\
                   body-fnv1a64 cbf29ce484222325\nepoch 0\n\n";
        let err = expect_err(import(art.as_bytes()));
        assert!(matches!(err, BackupError::Format(_)), "{:?}", err);
    }

    /// Restore is fail-closed when the body is corrupted (digest mismatch).
    #[test]
    fn import_rejects_corrupt_body() {
        let g = graph_with("<http://ex/s> <http://ex/p> <http://ex/o> .\n");
        let ring: GenerationRing<Graph> = GenerationRing::new(g);
        let gen = ring.current();
        let mut buf = Vec::new();
        export(&gen, &mut buf).expect("export");
        // Flip a byte inside the body (after the blank-line header terminator).
        let header_end = buf
            .windows(2)
            .position(|w| w == b"\n\n")
            .expect("blank-line terminator")
            + 2;
        let flip = header_end + 1;
        buf[flip] ^= 0xff;
        let err = expect_err(import(&buf[..]));
        assert!(matches!(err, BackupError::Corrupt(_)), "{:?}", err);
    }

    /// Restore is fail-closed when the triple count header disagrees with the body.
    #[test]
    fn import_rejects_triple_count_mismatch() {
        let g = graph_with("<http://ex/s> <http://ex/p> <http://ex/o> .\n");
        let ring: GenerationRing<Graph> = GenerationRing::new(g);
        let gen = ring.current();
        let mut buf = Vec::new();
        export(&gen, &mut buf).expect("export");
        let s = String::from_utf8(buf)
            .unwrap()
            .replace("triples 1", "triples 5");
        let err = expect_err(import(s.as_bytes()));
        assert!(matches!(err, BackupError::Corrupt(_)), "{:?}", err);
    }

    /// A truncated body (shorter than `body-bytes`) is fail-closed.
    #[test]
    fn import_rejects_truncated_body() {
        let g = graph_with("<http://ex/s> <http://ex/p> <http://ex/o> .\n");
        let ring: GenerationRing<Graph> = GenerationRing::new(g);
        let gen = ring.current();
        let mut buf = Vec::new();
        export(&gen, &mut buf).expect("export");
        buf.truncate(buf.len() - 5); // chop the tail of the body
        let err = expect_err(import(&buf[..]));
        assert!(matches!(err, BackupError::Corrupt(_)), "{:?}", err);
    }
}
