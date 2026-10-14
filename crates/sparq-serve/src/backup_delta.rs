//! (sq-bu1a) The INCREMENTAL DELTA-STREAM companion to the Option-A base backup
//! ([`crate::backup`]) — the change-stream / point-in-time-recovery (PITR) other half.
//!
//! ## What this is
//!
//! [`backup::export`](crate::backup::export) captures a single consistent generation as a
//! self-describing BASE artifact. This module captures the **change** between two generations
//! of the SAME ring lineage as a self-describing DELTA artifact, keyed off the recorded
//! generation/writer-seq (`from-generation` → `to-generation`). A restore then **replays
//! forward**: load the base, apply an ordered chain of deltas, and arrive at any later point in
//! the writer history that the chain covers — PITR.
//!
//! ```text
//!   base@G0 ──(delta G0→G3)──▶ G3 ──(delta G3→G7)──▶ G7   (= a chosen recovery point)
//! ```
//!
//! ## Artifact format — delta (a distinct kind, deliberately)
//!
//! A delta is its OWN artifact kind with its own magic ([`DELTA_MAGIC`]) and version, so it can
//! never be confused with a base ([`backup`](crate::backup) keeps `SPARQ-BACKUP` v1) and so an
//! `import_base`/`import_delta` mismatch fails closed at the magic line. The layout mirrors the
//! base's line-oriented textual header (trivially inspectable, version-stable), followed by TWO
//! N-Quads bodies — the quads INSERTED and the quads DELETED between `from` and `to`:
//!
//! ```text
//!   SPARQ-BACKUP-DELTA 1
//!   from-generation <u64>          ← the base/predecessor seq this delta applies onto
//!   to-generation   <u64>          ← the seq this delta advances to
//!   inserts <count>                ← quad lines in the inserts body
//!   deletes <count>                ← quad lines in the deletes body
//!   inserts-bytes <len> ; inserts-fnv1a64 <16hex>
//!   deletes-bytes <len> ; deletes-fnv1a64 <16hex>
//!   epoch <n> ; epoch-entry <epoch> <pod-iri> …   ← the per-pod vector AT `to`
//!   <blank line>
//!   <inserts N-Quads body><deletes N-Quads body>   ← inserts first, then deletes
//! ```
//!
//! The delta is computed as the **quad-set difference** of the two generations' N-Quads
//! serialisations: a quad in `to` but not in `from` is an insert; a quad in `from` but not in
//! `to` is a delete. This stays inside the Option-A spirit — self-describing, lossless,
//! format-stable RDF that round-trips through [`Graph::apply_delta_nquads`](sparq_core::Graph::apply_delta_nquads).
//!
//! ## Load-bearing boundary: SAME-LINEAGE deltas only (honest scope)
//!
//! A quad-set diff of two N-Quads serialisations is correct **only when blank-node labels are
//! stable between the two snapshots** — i.e. the two generations come from the **same ring
//! lineage** (the writer forks a generation and applies updates, preserving every surviving
//! term's identity, blank nodes included). That is exactly the PITR use case: a delta is always
//! taken between two points of one writer history, and replayed onto the base from THAT history.
//! Diffing snapshots from two unrelated load paths (where the same logical blank node may carry
//! different labels) is NOT supported and is not what the change stream does.
//! [`replay`](crate::backup_delta::replay) enforces the *generation*-continuity half of this
//! (each delta's `from` must equal the running seq); the lineage assumption itself is a
//! documented operator contract, not a runtime check.
//!
//! ## Fail-closed, same as the base
//!
//! [`import_delta`](crate::backup_delta::import_delta) rejects — never partially applies — a delta
//! whose magic, version, header shape, body lengths or body digests do not check out;
//! [`replay`](crate::backup_delta::replay) additionally rejects a
//! gapped/overlapping/out-of-order chain. A corrupt or discontinuous stream yields a
//! [`BackupError`] and the in-progress graph is discarded, so a PITR that fails leaves a clean
//! error rather than a silently-wrong store.
//!
//! **Out of scope (a separate concern), same as the base:** at-rest encryption + cryptographic
//! authenticity. The digests are the SAME non-cryptographic FNV-1a checksum the base uses — they
//! detect truncation/corruption/version-mismatch, not tampering.

use std::collections::BTreeSet;
use std::io::{self, Read, Write};

use sparq_core::Graph;
use sparq_engine::serialize::graph_to_nquads;

use crate::backup::{body_digest, parse_u64, read_line, total_quads, BackupError, BackupMeta};
use crate::epoch::{Epoch, PodEpochs, PodId};
use crate::ring::Generation;

/// Magic line that opens every sparq DELTA artifact — distinct from the base's `SPARQ-BACKUP`
/// so the two kinds can never be confused (an `import_delta` of a base, or vice-versa, fails
/// closed at this line).
pub const DELTA_MAGIC: &str = "SPARQ-BACKUP-DELTA";

/// Delta artifact format version. Bumped independently of the base's version; [`import_delta`]
/// rejects an unknown version (fail-closed) rather than mis-reading a newer layout.
const DELTA_FORMAT_VERSION: u32 = 1;

/// The point-in-time metadata captured ALONGSIDE the change-set in a delta artifact: the
/// `from`/`to` generation/writer-seq range it bridges, the per-pod epoch vector AT `to`, and the
/// insert / delete quad counts. Replayed in order, the `to` of one delta is the `from` of the
/// next, so a chain reconstructs the writer history between a base and a chosen recovery point.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeltaMeta {
    /// The generation/writer-seq this delta applies ONTO — the base (or predecessor delta's
    /// `to`). [`replay`] checks this equals the running generation before applying, so a gap or
    /// an out-of-order delta fails closed.
    pub from_generation: u64,
    /// The generation/writer-seq this delta advances the store TO. Strictly greater than
    /// `from_generation` (a delta always moves forward; equal/backward is rejected on import).
    pub to_generation: u64,
    /// The per-pod epoch vector as of `to_generation`. Restored verbatim so cache-invalidation
    /// epochs stay continuous across a replayed restore (a pod's epoch never goes backwards).
    pub epochs: PodEpochs,
    /// Number of quads inserted between `from` and `to` (lines in the inserts body).
    pub inserts: u64,
    /// Number of quads deleted between `from` and `to` (lines in the deletes body).
    pub deletes: u64,
}

/// The N-Quads line sets of a graph's full dataset (default + named graphs), one `String` per
/// quad, in a `BTreeSet` so set-difference is deterministic and the result is stably ordered.
/// Blank-node labels are rendered verbatim by the serialiser, so this is a faithful per-quad set
/// WITHIN a lineage (see the module's same-lineage boundary).
fn quad_lines(graph: &Graph) -> BTreeSet<String> {
    graph_to_nquads(graph)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(String::from)
        .collect()
}

/// Computes the change-set between `from` and `to` (both same-lineage generations) and writes a
/// single self-describing DELTA artifact to `out`. `to` must be strictly later than `from`
/// (`to.number() > from.number()`); a non-increasing range is a caller error
/// ([`BackupError::Format`]).
///
/// Both generations are already immutable (held by their `Arc`s), so this runs **while serving**:
/// nothing here touches the ring or the writer. The delta's epoch vector is taken from `to`.
pub fn export_delta<W: Write>(
    from: &Generation<Graph>,
    to: &Generation<Graph>,
    out: &mut W,
) -> Result<DeltaMeta, BackupError> {
    if to.number() <= from.number() {
        return Err(BackupError::Format(format!(
            "delta range must move forward: from-generation {} >= to-generation {}",
            from.number(),
            to.number()
        )));
    }
    let from_lines = quad_lines(from.snapshot());
    let to_lines = quad_lines(to.snapshot());

    // Inserts = in `to` but not `from`; deletes = in `from` but not `to`. BTreeSet difference is
    // sorted+deterministic, so the bodies (and thus the digests) are reproducible.
    let mut inserts = String::new();
    let mut n_inserts: u64 = 0;
    for line in to_lines.difference(&from_lines) {
        inserts.push_str(line);
        inserts.push('\n');
        n_inserts += 1;
    }
    let mut deletes = String::new();
    let mut n_deletes: u64 = 0;
    for line in from_lines.difference(&to_lines) {
        deletes.push_str(line);
        deletes.push('\n');
        n_deletes += 1;
    }

    let ins_bytes = inserts.as_bytes();
    let del_bytes = deletes.as_bytes();
    let ins_digest = body_digest(ins_bytes);
    let del_digest = body_digest(del_bytes);

    writeln!(out, "{} {}", DELTA_MAGIC, DELTA_FORMAT_VERSION)?;
    writeln!(out, "from-generation {}", from.number())?;
    writeln!(out, "to-generation {}", to.number())?;
    writeln!(out, "inserts {}", n_inserts)?;
    writeln!(out, "deletes {}", n_deletes)?;
    writeln!(out, "inserts-bytes {}", ins_bytes.len())?;
    writeln!(out, "inserts-fnv1a64 {:016x}", ins_digest)?;
    writeln!(out, "deletes-bytes {}", del_bytes.len())?;
    writeln!(out, "deletes-fnv1a64 {:016x}", del_digest)?;
    // Epoch vector AT `to` — same line shape as the base header.
    let epochs = to.epochs();
    writeln!(out, "epoch {}", epochs.len())?;
    for (pod, epoch) in epochs.iter() {
        writeln!(out, "epoch-entry {} {}", epoch, pod.as_str())?;
    }
    writeln!(out)?; // blank line: end of header
    out.write_all(ins_bytes)?;
    out.write_all(del_bytes)?;
    out.flush()?;

    Ok(DeltaMeta {
        from_generation: from.number(),
        to_generation: to.number(),
        epochs: epochs.clone(),
        inserts: n_inserts,
        deletes: n_deletes,
    })
}

/// A decoded delta artifact: the insert / delete N-Quads bodies (ready for
/// [`Graph::apply_delta_nquads`]) and the [`DeltaMeta`] describing the seq range + epoch vector.
/// Kept separate from application so a caller can inspect / chain / validate a stream before any
/// graph is mutated.
#[derive(Clone, Debug)]
pub struct Delta {
    /// The change-stream metadata (seq range, epoch vector, counts).
    pub meta: DeltaMeta,
    /// The N-Quads of the quads to INSERT (default + named graphs).
    pub inserts: String,
    /// The N-Quads of the quads to DELETE (default + named graphs).
    pub deletes: String,
}

/// Imports one delta artifact from `input`, returning the decoded [`Delta`] WITHOUT applying it.
/// **Fail-closed**: a bad/absent magic, an unknown version, a malformed header, a non-forward
/// range, or a body whose declared length / count / digest does not match all yield a
/// [`BackupError`]. Applying a chain of these is [`replay`].
pub fn import_delta<R: Read>(input: R) -> Result<Delta, BackupError> {
    let mut reader = io::BufReader::new(input);

    // --- Magic + version ---
    let first = read_line(&mut reader)?;
    let mut parts = first.splitn(2, ' ');
    let magic = parts.next().unwrap_or("");
    if magic != DELTA_MAGIC {
        return Err(BackupError::Format(format!(
            "not a sparq delta artifact (expected magic {:?})",
            DELTA_MAGIC
        )));
    }
    let version: u32 = parts
        .next()
        .and_then(|v| v.trim().parse().ok())
        .ok_or_else(|| BackupError::Format("missing/invalid delta format version".into()))?;
    if version != DELTA_FORMAT_VERSION {
        return Err(BackupError::Format(format!(
            "unsupported delta format version {} (this build reads/writes {})",
            version, DELTA_FORMAT_VERSION
        )));
    }

    // --- Header ---
    let mut from_generation: Option<u64> = None;
    let mut to_generation: Option<u64> = None;
    let mut inserts: Option<u64> = None;
    let mut deletes: Option<u64> = None;
    let mut inserts_bytes: Option<usize> = None;
    let mut deletes_bytes: Option<usize> = None;
    let mut inserts_digest_hdr: Option<u64> = None;
    let mut deletes_digest_hdr: Option<u64> = None;
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
            "from-generation" => from_generation = Some(parse_u64(val, "from-generation")?),
            "to-generation" => to_generation = Some(parse_u64(val, "to-generation")?),
            "inserts" => inserts = Some(parse_u64(val, "inserts")?),
            "deletes" => deletes = Some(parse_u64(val, "deletes")?),
            "inserts-bytes" => {
                inserts_bytes = Some(usize::try_from(parse_u64(val, "inserts-bytes")?).map_err(
                    |_| BackupError::Format("inserts-bytes exceeds platform usize".into()),
                )?)
            }
            "deletes-bytes" => {
                deletes_bytes = Some(usize::try_from(parse_u64(val, "deletes-bytes")?).map_err(
                    |_| BackupError::Format("deletes-bytes exceeds platform usize".into()),
                )?)
            }
            "inserts-fnv1a64" => {
                inserts_digest_hdr = Some(u64::from_str_radix(val, 16).map_err(|_| {
                    BackupError::Format("invalid inserts-fnv1a64 digest (not 16-hex)".into())
                })?)
            }
            "deletes-fnv1a64" => {
                deletes_digest_hdr = Some(u64::from_str_radix(val, 16).map_err(|_| {
                    BackupError::Format("invalid deletes-fnv1a64 digest (not 16-hex)".into())
                })?)
            }
            "epoch" => {
                epoch_count = Some(usize::try_from(parse_u64(val, "epoch")?).map_err(|_| {
                    BackupError::Format("epoch count exceeds platform usize".into())
                })?)
            }
            "epoch-entry" => {
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
            // Unknown header keys are rejected (fail-closed), same as the base importer.
            other => {
                return Err(BackupError::Format(format!(
                    "unknown delta header key {:?}",
                    other
                )))
            }
        }
    }

    let from_generation = from_generation
        .ok_or_else(|| BackupError::Format("missing `from-generation` header".into()))?;
    let to_generation = to_generation
        .ok_or_else(|| BackupError::Format("missing `to-generation` header".into()))?;
    if to_generation <= from_generation {
        return Err(BackupError::Format(format!(
            "delta range must move forward: from-generation {} >= to-generation {}",
            from_generation, to_generation
        )));
    }
    let inserts = inserts.ok_or_else(|| BackupError::Format("missing `inserts` header".into()))?;
    let deletes = deletes.ok_or_else(|| BackupError::Format("missing `deletes` header".into()))?;
    let inserts_bytes = inserts_bytes
        .ok_or_else(|| BackupError::Format("missing `inserts-bytes` header".into()))?;
    let deletes_bytes = deletes_bytes
        .ok_or_else(|| BackupError::Format("missing `deletes-bytes` header".into()))?;
    let inserts_digest_hdr = inserts_digest_hdr
        .ok_or_else(|| BackupError::Format("missing `inserts-fnv1a64` header".into()))?;
    let deletes_digest_hdr = deletes_digest_hdr
        .ok_or_else(|| BackupError::Format("missing `deletes-fnv1a64` header".into()))?;
    if let Some(n) = epoch_count {
        if n != epoch_entries {
            return Err(BackupError::Format(format!(
                "epoch count {} != {} epoch-entry lines",
                n, epoch_entries
            )));
        }
    }

    // --- Bodies (inserts first, then deletes; each exactly its declared byte length) ---
    let inserts_body = read_body(&mut reader, inserts_bytes, inserts_digest_hdr, "inserts")?;
    let deletes_body = read_body(&mut reader, deletes_bytes, deletes_digest_hdr, "deletes")?;
    // Cross-check the declared quad counts against the actual non-empty line counts.
    check_line_count(&inserts_body, inserts, "inserts")?;
    check_line_count(&deletes_body, deletes, "deletes")?;

    Ok(Delta {
        meta: DeltaMeta {
            from_generation,
            to_generation,
            epochs,
            inserts,
            deletes,
        },
        inserts: inserts_body,
        deletes: deletes_body,
    })
}

/// Reads exactly `len` bytes, verifies the digest, and decodes UTF-8 — the shared body reader for
/// both the inserts and the deletes sections. Fail-closed on a short read, a digest mismatch, or
/// invalid UTF-8.
fn read_body<R: Read>(
    reader: &mut R,
    len: usize,
    digest_hdr: u64,
    which: &str,
) -> Result<String, BackupError> {
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).map_err(|_| {
        BackupError::Corrupt(format!("{} body shorter than declared byte length", which))
    })?;
    let actual = body_digest(&body);
    if actual != digest_hdr {
        return Err(BackupError::Corrupt(format!(
            "{} body digest mismatch (header {:016x}, computed {:016x})",
            which, digest_hdr, actual
        )));
    }
    String::from_utf8(body)
        .map_err(|_| BackupError::Corrupt(format!("{} body is not valid UTF-8", which)))
}

/// Cross-checks the number of non-empty quad lines in `body` against the declared `count`.
fn check_line_count(body: &str, count: u64, which: &str) -> Result<(), BackupError> {
    let actual = body.lines().filter(|l| !l.trim().is_empty()).count() as u64;
    if actual != count {
        return Err(BackupError::Corrupt(format!(
            "{} line count mismatch (header {}, decoded {})",
            which, count, actual
        )));
    }
    Ok(())
}

/// Replays an ordered chain of delta artifacts forward onto a restored BASE graph, in place,
/// reconstructing the store as of the LAST delta's `to_generation` — point-in-time recovery.
///
/// `base_meta` is the [`BackupMeta`] the base was imported with ([`backup::import`](crate::backup::import));
/// its `generation` is the seq the chain must start from. Each delta `d` in `deltas` is applied
/// only if `d.meta.from_generation` equals the running generation (the base's generation, then
/// the previous delta's `to_generation`); a gap, an overlap, or an out-of-order delta is rejected
/// **fail-closed** ([`BackupError::Corrupt`]) and `graph` is left as it was at the last good step
/// — the caller discards a partially-replayed graph rather than serving a wrong store. Returns
/// the [`BackupMeta`] of the recovered point (the last delta's epoch vector + `to_generation`, or
/// the unchanged `base_meta` for an empty chain).
///
/// The chain must be **same-lineage** with the base (see the module boundary): the deltas are the
/// writer history of that base, replayed onto it.
pub fn replay<'a, I>(
    graph: &mut Graph,
    base_meta: &BackupMeta,
    deltas: I,
) -> Result<BackupMeta, BackupError>
where
    I: IntoIterator<Item = &'a Delta>,
{
    let mut current = base_meta.generation;
    let mut epochs = base_meta.epochs.clone();
    for d in deltas {
        if d.meta.from_generation != current {
            return Err(BackupError::Corrupt(format!(
                "delta chain discontinuity: expected a delta from generation {}, got one from {} (to {})",
                current, d.meta.from_generation, d.meta.to_generation
            )));
        }
        graph
            .apply_delta_nquads(&d.inserts, &d.deletes)
            .map_err(|e| BackupError::Corrupt(format!("delta replay failed: {}", e)))?;
        current = d.meta.to_generation;
        epochs = d.meta.epochs.clone();
    }
    Ok(BackupMeta {
        generation: current,
        epochs,
        triples: total_quads(graph),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::{export as export_base, import as import_base};
    use crate::ring::GenerationRing;

    fn graph_with(nq: &str) -> Graph {
        Graph::load_dataset(nq, "nquads").expect("seed parses")
    }

    /// Sorted N-Quads line set of a graph — the equality oracle for "same dataset".
    fn lines_sorted(g: &Graph) -> Vec<String> {
        let mut v: Vec<String> = graph_to_nquads(g)
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(String::from)
            .collect();
        v.sort();
        v
    }

    /// `Graph` is not `Debug`, so a `Result<Delta, _>::unwrap_err` is fine but a
    /// `Result<(Graph, _), _>` is not — extract the error without Debug-formatting Ok.
    fn err_of<T>(res: Result<T, BackupError>) -> BackupError {
        match res {
            Ok(_) => panic!("expected a fail-closed error, but it succeeded"),
            Err(e) => e,
        }
    }

    /// The load-bearing invariant: base + replayed delta chain == the live generation it was
    /// taken at (RESULT-EQUIVALENCE). Build a ring, publish a few generations with real
    /// inserts/deletes across default + named graphs, back up the base at G0, export deltas, and
    /// replay them — the recovered graph must equal the live target generation quad-for-quad.
    #[test]
    fn pitr_replay_equals_live_target() {
        let pod_a = PodId::new("http://ex/g");
        let ring: GenerationRing<Graph> = GenerationRing::new(graph_with(
            "<http://ex/s0> <http://ex/p> <http://ex/o0> .\n",
        ));
        let base = ring.current();

        // G1: add a default + a named-graph triple.
        let g1 = ring.publish(
            graph_with(
                "<http://ex/s0> <http://ex/p> <http://ex/o0> .\n\
                 <http://ex/s1> <http://ex/p> <http://ex/o1> .\n\
                 <http://ex/s1> <http://ex/p> \"in-g\" <http://ex/g> .\n",
            ),
            [pod_a.clone()],
        );
        // G2: delete the original default triple, add another named-graph triple.
        let g2 = ring.publish(
            graph_with(
                "<http://ex/s1> <http://ex/p> <http://ex/o1> .\n\
                 <http://ex/s1> <http://ex/p> \"in-g\" <http://ex/g> .\n\
                 <http://ex/s2> <http://ex/p> \"in-g\" <http://ex/g> .\n",
            ),
            [pod_a.clone()],
        );

        // Base artifact @G0.
        let mut base_buf = Vec::new();
        export_base(&base, &mut base_buf).expect("export base");
        // Two deltas: G0→G1 and G1→G2.
        let mut d01 = Vec::new();
        let m01 = export_delta(&base, &g1, &mut d01).expect("export delta 0->1");
        assert_eq!(m01.from_generation, 0);
        assert_eq!(m01.to_generation, 1);
        let mut d12 = Vec::new();
        export_delta(&g1, &g2, &mut d12).expect("export delta 1->2");

        // Restore: import base, decode the chain, replay.
        let (mut restored, base_meta) = import_base(&base_buf[..]).expect("import base");
        let dec01 = import_delta(&d01[..]).expect("import delta 0->1");
        let dec12 = import_delta(&d12[..]).expect("import delta 1->2");
        let recovered = replay(&mut restored, &base_meta, [&dec01, &dec12]).expect("replay chain");

        // The recovered point is generation 2 with the named-pod epoch from g2.
        assert_eq!(recovered.generation, 2);
        assert_eq!(recovered.epochs.epoch(&pod_a), g2.epochs().epoch(&pod_a));
        // Quad-for-quad equality with the live G2 snapshot.
        assert_eq!(
            lines_sorted(&restored),
            lines_sorted(g2.snapshot()),
            "PITR replay must equal the live target generation"
        );
    }

    /// Replaying PART of the chain recovers the INTERMEDIATE point (a chosen earlier PITR target).
    #[test]
    fn partial_replay_recovers_intermediate_point() {
        let ring: GenerationRing<Graph> = GenerationRing::new(graph_with(
            "<http://ex/s0> <http://ex/p> <http://ex/o0> .\n",
        ));
        let base = ring.current();
        let g1 = ring.publish(
            graph_with(
                "<http://ex/s0> <http://ex/p> <http://ex/o0> .\n\
                 <http://ex/s1> <http://ex/p> <http://ex/o1> .\n",
            ),
            [],
        );
        let g2 = ring.publish(
            graph_with("<http://ex/s9> <http://ex/p> <http://ex/o9> .\n"),
            [],
        );
        let _ = g2;

        let mut base_buf = Vec::new();
        export_base(&base, &mut base_buf).expect("export base");
        let mut d01 = Vec::new();
        export_delta(&base, &g1, &mut d01).expect("export 0->1");

        let (mut restored, base_meta) = import_base(&base_buf[..]).expect("import base");
        let dec01 = import_delta(&d01[..]).expect("import 0->1");
        let recovered = replay(&mut restored, &base_meta, [&dec01]).expect("replay 0->1");
        assert_eq!(recovered.generation, 1);
        assert_eq!(lines_sorted(&restored), lines_sorted(g1.snapshot()));
    }

    /// An EMPTY chain is the base itself (PITR target == the base point).
    #[test]
    fn empty_chain_is_the_base() {
        let ring: GenerationRing<Graph> =
            GenerationRing::new(graph_with("<http://ex/s> <http://ex/p> <http://ex/o> .\n"));
        let base = ring.current();
        let mut base_buf = Vec::new();
        export_base(&base, &mut base_buf).expect("export base");
        let (mut restored, base_meta) = import_base(&base_buf[..]).expect("import base");
        let recovered =
            replay(&mut restored, &base_meta, std::iter::empty()).expect("empty replay");
        assert_eq!(recovered.generation, base_meta.generation);
        assert_eq!(recovered.triples, base_meta.triples);
    }

    /// A round-tripped delta preserves its metadata (seq range + epoch vector + counts).
    #[test]
    fn delta_roundtrip_preserves_meta() {
        let pod = PodId::new("http://ex/g");
        let ring: GenerationRing<Graph> = GenerationRing::new(graph_with(""));
        let base = ring.current();
        let g3 = {
            ring.publish(
                graph_with("<http://ex/a> <http://ex/p> <http://ex/b> .\n"),
                [pod.clone()],
            );
            ring.publish(
                graph_with("<http://ex/a> <http://ex/p> <http://ex/b> .\n"),
                [pod.clone()],
            );
            ring.publish(
                graph_with(
                    "<http://ex/a> <http://ex/p> <http://ex/b> .\n\
                     <http://ex/c> <http://ex/p> \"x\" <http://ex/g> .\n",
                ),
                [pod.clone()],
            )
        };
        assert_eq!(g3.number(), 3);
        let mut buf = Vec::new();
        let exported = export_delta(&base, &g3, &mut buf).expect("export");
        let decoded = import_delta(&buf[..]).expect("import");
        assert_eq!(decoded.meta, exported);
        assert_eq!(decoded.meta.from_generation, 0);
        assert_eq!(decoded.meta.to_generation, 3);
        assert_eq!(decoded.meta.epochs.epoch(&pod), 3);
        // 2 quads added (one default, one named), nothing deleted.
        assert_eq!(decoded.meta.inserts, 2);
        assert_eq!(decoded.meta.deletes, 0);
    }

    /// A non-forward range is rejected on export AND on import (fail-closed).
    #[test]
    fn rejects_non_forward_range() {
        let ring: GenerationRing<Graph> = GenerationRing::new(graph_with(""));
        let base = ring.current();
        let g1 = ring.publish(
            graph_with("<http://ex/a> <http://ex/p> <http://ex/b> .\n"),
            [],
        );
        // Export with from=g1, to=base (backwards) is rejected.
        let mut buf = Vec::new();
        let err = err_of(export_delta(&g1, &base, &mut buf));
        assert!(matches!(err, BackupError::Format(_)), "{:?}", err);
        // A hand-crafted backwards artifact is rejected on import too.
        let art = "SPARQ-BACKUP-DELTA 1\nfrom-generation 5\nto-generation 5\ninserts 0\n\
                   deletes 0\ninserts-bytes 0\ninserts-fnv1a64 cbf29ce484222325\n\
                   deletes-bytes 0\ndeletes-fnv1a64 cbf29ce484222325\nepoch 0\n\n";
        let err = err_of(import_delta(art.as_bytes()));
        assert!(matches!(err, BackupError::Format(_)), "{:?}", err);
    }

    /// A delta is NOT a base and vice-versa: import_delta of a base, and import (base) of a
    /// delta, both fail closed at the magic line.
    #[test]
    fn delta_and_base_are_distinct_kinds() {
        let ring: GenerationRing<Graph> =
            GenerationRing::new(graph_with("<http://ex/s> <http://ex/p> <http://ex/o> .\n"));
        let base = ring.current();
        let g1 = ring.publish(
            graph_with("<http://ex/s> <http://ex/p> <http://ex/o2> .\n"),
            [],
        );
        let mut base_buf = Vec::new();
        export_base(&base, &mut base_buf).expect("export base");
        let mut delta_buf = Vec::new();
        export_delta(&base, &g1, &mut delta_buf).expect("export delta");
        // import_delta of a base artifact -> Format error (wrong magic).
        let err = err_of(import_delta(&base_buf[..]));
        assert!(matches!(err, BackupError::Format(_)), "{:?}", err);
        // import (base) of a delta artifact -> Format error (wrong magic).
        let err = err_of(import_base(&delta_buf[..]));
        assert!(matches!(err, BackupError::Format(_)), "{:?}", err);
    }

    /// A gapped chain is rejected fail-closed on replay (the base is G0 but the chain starts at G5).
    #[test]
    fn replay_rejects_chain_gap() {
        let ring: GenerationRing<Graph> = GenerationRing::new(graph_with(""));
        let base = ring.current();
        let g1 = ring.publish(
            graph_with("<http://ex/a> <http://ex/p> <http://ex/b> .\n"),
            [],
        );
        let mut base_buf = Vec::new();
        export_base(&base, &mut base_buf).expect("export base");
        let mut d = Vec::new();
        export_delta(&base, &g1, &mut d).expect("export 0->1");
        let mut dec = import_delta(&d[..]).expect("import 0->1");
        // Forge a gap: claim this delta applies from generation 5, not 0.
        dec.meta.from_generation = 5;
        let (mut restored, base_meta) = import_base(&base_buf[..]).expect("import base");
        let err = err_of(replay(&mut restored, &base_meta, [&dec]));
        assert!(matches!(err, BackupError::Corrupt(_)), "{:?}", err);
    }

    /// An out-of-order chain (correct individually, wrong sequence) is rejected fail-closed.
    #[test]
    fn replay_rejects_out_of_order() {
        let ring: GenerationRing<Graph> = GenerationRing::new(graph_with(""));
        let base = ring.current();
        let g1 = ring.publish(
            graph_with("<http://ex/a> <http://ex/p> <http://ex/b> .\n"),
            [],
        );
        let g2 = ring.publish(
            graph_with("<http://ex/c> <http://ex/p> <http://ex/d> .\n"),
            [],
        );
        let mut base_buf = Vec::new();
        export_base(&base, &mut base_buf).expect("export base");
        let mut d01 = Vec::new();
        export_delta(&base, &g1, &mut d01).expect("0->1");
        let mut d12 = Vec::new();
        export_delta(&g1, &g2, &mut d12).expect("1->2");
        let dec01 = import_delta(&d01[..]).expect("import 0->1");
        let dec12 = import_delta(&d12[..]).expect("import 1->2");
        let (mut restored, base_meta) = import_base(&base_buf[..]).expect("import base");
        // 1->2 before 0->1: the first applied delta's from-generation (1) != base generation (0).
        let err = err_of(replay(&mut restored, &base_meta, [&dec12, &dec01]));
        assert!(matches!(err, BackupError::Corrupt(_)), "{:?}", err);
    }

    /// A corrupt body (digest mismatch) is rejected fail-closed on import.
    #[test]
    fn import_rejects_corrupt_body() {
        let ring: GenerationRing<Graph> = GenerationRing::new(graph_with(""));
        let base = ring.current();
        let g1 = ring.publish(
            graph_with("<http://ex/a> <http://ex/p> <http://ex/b> .\n"),
            [],
        );
        let mut buf = Vec::new();
        export_delta(&base, &g1, &mut buf).expect("export");
        // Flip a byte inside the bodies (after the blank-line header terminator).
        let header_end = buf
            .windows(2)
            .position(|w| w == b"\n\n")
            .expect("blank-line terminator")
            + 2;
        buf[header_end + 1] ^= 0xff;
        let err = err_of(import_delta(&buf[..]));
        assert!(matches!(err, BackupError::Corrupt(_)), "{:?}", err);
    }

    /// A truncated delta (body shorter than declared) is rejected fail-closed.
    #[test]
    fn import_rejects_truncated() {
        let ring: GenerationRing<Graph> = GenerationRing::new(graph_with(""));
        let base = ring.current();
        let g1 = ring.publish(
            graph_with("<http://ex/a> <http://ex/p> <http://ex/b> .\n"),
            [],
        );
        let mut buf = Vec::new();
        export_delta(&base, &g1, &mut buf).expect("export");
        buf.truncate(buf.len() - 3);
        let err = err_of(import_delta(&buf[..]));
        assert!(matches!(err, BackupError::Corrupt(_)), "{:?}", err);
    }

    /// An unknown delta format version is rejected fail-closed.
    #[test]
    fn import_rejects_unknown_version() {
        let art = "SPARQ-BACKUP-DELTA 999\nfrom-generation 0\nto-generation 1\ninserts 0\n\n";
        let err = err_of(import_delta(art.as_bytes()));
        assert!(matches!(err, BackupError::Format(_)), "{:?}", err);
    }
}
