//! Round-trip validation: an HDT archive loaded via `sparq_hdt::load` must yield
//! EXACTLY the triple set sparq's own N-Triples loader produces from the same data.

use sparq_core::Graph;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// sq-117n: a UNIQUE, auto-cleaned scratch directory, one per call.
///
/// The tests here materialize `.hdt` archives by writing an N-Triples file to disk
/// (`src.nt` / `zoo.nt` / …) and reading it back with `hdt::Hdt::read_nt`. Previously
/// each test (and the shared `valid_corruptible_hdt()` / `nt_to_hdt_bytes` helpers)
/// joined `CARGO_TARGET_TMPDIR` with a FIXED subdir name, so sibling tests running
/// concurrently under `cargo test` raced on the same file — one test writing/replacing
/// `src.nt` while another read it back, which intermittently flaked the rejection-oracle
/// tests (`rejection_oracle_*` all funneled through one `sparq-hdt-corrupt` dir).
/// `tempfile::tempdir()` returns a fresh, uniquely-named directory under the OS temp
/// dir that is removed when the returned `TempDir` is dropped, so no two test
/// invocations can ever collide. Callers must keep the `TempDir` alive for as long as
/// they use the path (binding it to a local does this).
fn scratch_dir() -> TempDir {
    tempfile::tempdir().expect("creating a unique scratch directory")
}

/// Dumps a graph as a canonical, comparable triple set (N-Triples term rendering).
fn triple_set(g: &Graph) -> BTreeSet<[String; 3]> {
    let scan = g.store.scan(&[None, None, None]);
    scan.rows
        .iter()
        .map(|r| {
            let [s, p, o] = scan.to_spo(r);
            [
                g.dict.term(s).to_string(),
                g.dict.term(p).to_string(),
                g.dict.term(o).to_string(),
            ]
        })
        .collect()
}

/// snikmeta.hdt is a real-world archive vendored from the `hdt` crate's test suite
/// (MIT; SNIK meta ontology) — i.e. NOT produced by this code path. Its N-Triples
/// ground truth (snikmeta.nt, 328 triples — the same count the `hdt` crate's own
/// tests assert) must load to the identical triple set.
#[test]
fn snikmeta_hdt_matches_ntriples() {
    let hdt_path = fixture("snikmeta.hdt");
    if !hdt_path.exists() {
        eprintln!("skipping: fixture {} absent", hdt_path.display());
        return;
    }
    let from_hdt = sparq_hdt::load(&hdt_path).expect("loading snikmeta.hdt");
    let nt = std::fs::read_to_string(fixture("snikmeta.nt")).expect("reading snikmeta.nt");
    let from_nt = Graph::load_str(&nt, "ntriples").expect("loading snikmeta.nt");

    let h = triple_set(&from_hdt);
    let n = triple_set(&from_nt);
    assert_eq!(h.len(), 328, "snikmeta is 328 distinct triples");
    assert_eq!(h, n, "HDT and N-Triples loads must agree triple-for-triple");
    assert_eq!(from_hdt.store.len(), from_nt.store.len());
}

/// Full binary round trip with the term zoo: N-Triples -> HDT archive (via the
/// hdt crate's writer) -> `sparq_hdt::load` -> compare against sparq's own load
/// of the SOURCE N-Triples. Covers IRIs, blank nodes, plain / language-tagged /
/// datatyped literals, unicode, integers (sparq's inline-literal path), and terms
/// shared between subject and object position (HDT's shared dictionary section).
#[test]
fn generated_hdt_round_trips() {
    let nt = concat!(
        "<http://example.org/alice> <http://xmlns.com/foaf/0.1/knows> <http://example.org/bob> .\n",
        "<http://example.org/bob> <http://xmlns.com/foaf/0.1/knows> <http://example.org/alice> .\n",
        "<http://example.org/alice> <http://xmlns.com/foaf/0.1/name> \"Alice\" .\n",
        "<http://example.org/alice> <http://xmlns.com/foaf/0.1/age> \"42\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
        "<http://example.org/alice> <http://www.w3.org/2000/01/rdf-schema#label> \"Alice\"@en .\n",
        "<http://example.org/alice> <http://www.w3.org/2000/01/rdf-schema#label> \"Alicia\"@es .\n",
        "<http://example.org/bob> <http://www.w3.org/2000/01/rdf-schema#label> \"B\u{00f6}b \u{2014} caf\u{00e9}\"@de .\n",
        "<http://example.org/bob> <http://example.org/height> \"1.83\"^^<http://www.w3.org/2001/XMLSchema#decimal> .\n",
        "_:b0 <http://xmlns.com/foaf/0.1/knows> <http://example.org/alice> .\n",
        "_:b0 <http://example.org/link> _:b1 .\n",
        "_:b1 <http://xmlns.com/foaf/0.1/name> \"anon\" .\n",
    );

    let dir = scratch_dir();
    let nt_path = dir.path().join("zoo.nt");
    let hdt_path = dir.path().join("zoo.hdt");
    std::fs::write(&nt_path, nt).unwrap();

    // N-Triples -> HDT archive on disk (FourSectDict PFC + BitmapTriples).
    let written = hdt::Hdt::read_nt(&nt_path).expect("building HDT from N-Triples");
    let mut out = std::io::BufWriter::new(std::fs::File::create(&hdt_path).unwrap());
    written.write(&mut out).expect("writing HDT archive");
    drop(out);

    let from_hdt = sparq_hdt::load(&hdt_path).expect("loading generated HDT");
    let from_nt = Graph::load_str(nt, "ntriples").unwrap();

    assert_eq!(triple_set(&from_hdt), triple_set(&from_nt));
    assert_eq!(from_hdt.store.len(), 11);
}

/// `load_reader` is the same decode from any buffered source.
#[test]
fn load_reader_from_memory() {
    let hdt_path = fixture("snikmeta.hdt");
    if !hdt_path.exists() {
        eprintln!("skipping: fixture {} absent", hdt_path.display());
        return;
    }
    let bytes = std::fs::read(&hdt_path).unwrap();
    let g = sparq_hdt::load_reader(std::io::Cursor::new(bytes)).unwrap();
    assert_eq!(g.store.len(), 328);
}

/// Garbage input must error, not panic.
#[test]
fn rejects_non_hdt_input() {
    let r = sparq_hdt::load_reader(std::io::Cursor::new(b"not an hdt file".to_vec()));
    assert!(r.is_err());
}

/// GZips the fixture in memory (single member; the decoder also accepts
/// multi-member streams).
fn gzipped_snikmeta() -> Option<Vec<u8>> {
    use std::io::Write;
    let hdt_path = fixture("snikmeta.hdt");
    if !hdt_path.exists() {
        eprintln!("skipping: fixture {} absent", hdt_path.display());
        return None;
    }
    let bytes = std::fs::read(&hdt_path).unwrap();
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    enc.write_all(&bytes).unwrap();
    Some(enc.finish().unwrap())
}

/// `.hdt.gz` containers are detected by MAGIC BYTES (not file name) and
/// decompressed transparently, from both a path and a reader.
#[test]
fn gzipped_hdt_loads_transparently() {
    let Some(gz) = gzipped_snikmeta() else { return };
    // Through a reader…
    let g = sparq_hdt::load_reader(std::io::Cursor::new(gz.clone())).unwrap();
    assert_eq!(g.store.len(), 328);
    assert_eq!(
        triple_set(&g),
        triple_set(&sparq_hdt::load(fixture("snikmeta.hdt")).unwrap())
    );
    // …and through a path with NO .gz extension (content sniffing, not names).
    let dir = scratch_dir();
    let path = dir.path().join("disguised.hdt");
    std::fs::write(&path, &gz).unwrap();
    assert_eq!(sparq_hdt::load(&path).unwrap().store.len(), 328);
}

// H5: zstd / bzip2 compressed-HDT container compressors, mirroring
// `gzipped_snikmeta`. Both decode in the STREAMING path (`with_hdt_stream`),
// selected by magic bytes (zstd `28 b5 2f fd`, bzip2 "BZh"), never file name.

/// zstd-compresses the snikmeta fixture in memory.
fn zstd_snikmeta() -> Option<Vec<u8>> {
    let hdt_path = fixture("snikmeta.hdt");
    if !hdt_path.exists() {
        eprintln!("skipping: fixture {} absent", hdt_path.display());
        return None;
    }
    let bytes = std::fs::read(&hdt_path).unwrap();
    Some(zstd::stream::encode_all(std::io::Cursor::new(bytes), 3).unwrap())
}

/// bzip2-compresses the snikmeta fixture in memory.
fn bzip2_snikmeta() -> Option<Vec<u8>> {
    use std::io::Write;
    let hdt_path = fixture("snikmeta.hdt");
    if !hdt_path.exists() {
        eprintln!("skipping: fixture {} absent", hdt_path.display());
        return None;
    }
    let bytes = std::fs::read(&hdt_path).unwrap();
    let mut enc = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::fast());
    enc.write_all(&bytes).unwrap();
    Some(enc.finish().unwrap())
}

/// `.hdt.zst` containers are sniffed by magic bytes and streamed-decoded, from a
/// reader and a path with no `.zst` extension. Triple set == plain `.hdt`.
#[test]
fn zstd_hdt_loads_transparently() {
    let Some(zst) = zstd_snikmeta() else { return };
    let plain = sparq_hdt::load(fixture("snikmeta.hdt")).unwrap();
    let g = sparq_hdt::load_reader(std::io::Cursor::new(zst.clone())).unwrap();
    assert_eq!(g.store.len(), 328);
    assert_eq!(triple_set(&g), triple_set(&plain));
    let dir = scratch_dir();
    let path = dir.path().join("disguised.hdt"); // sniffed by content, not name
    std::fs::write(&path, &zst).unwrap();
    assert_eq!(sparq_hdt::load(&path).unwrap().store.len(), 328);
}

/// `.hdt.bz2` containers are sniffed by magic bytes and streamed-decoded, from a
/// reader and a path with no `.bz2` extension. Triple set == plain `.hdt`.
#[test]
fn bzip2_hdt_loads_transparently() {
    let Some(bz) = bzip2_snikmeta() else { return };
    let plain = sparq_hdt::load(fixture("snikmeta.hdt")).unwrap();
    let g = sparq_hdt::load_reader(std::io::Cursor::new(bz.clone())).unwrap();
    assert_eq!(g.store.len(), 328);
    assert_eq!(triple_set(&g), triple_set(&plain));
    let dir = scratch_dir();
    let path = dir.path().join("disguised.hdt"); // sniffed by content, not name
    std::fs::write(&path, &bz).unwrap();
    assert_eq!(sparq_hdt::load(&path).unwrap().store.len(), 328);
}

/// The HDT header (dataset metadata triples) is exposed as a queryable graph,
/// for plain and gzipped archives alike.
#[test]
fn header_exposes_dataset_metadata() {
    let hdt_path = fixture("snikmeta.hdt");
    if !hdt_path.exists() {
        eprintln!("skipping: fixture {} absent", hdt_path.display());
        return;
    }
    let h = sparq_hdt::header(&hdt_path).unwrap();
    assert!(!h.is_empty(), "snikmeta carries header metadata");
    // The VoID statistics agree with the actual triple count.
    let triples: Vec<[String; 3]> = triple_set(&h)
        .into_iter()
        .filter(|[_, p, _]| p == "<http://rdfs.org/ns/void#triples>")
        .collect();
    assert_eq!(triples.len(), 1);
    assert_eq!(triples[0][2], "\"328\"");
    // Same through a gzipped stream.
    if let Some(gz) = gzipped_snikmeta() {
        let h2 = sparq_hdt::header_reader(std::io::Cursor::new(gz)).unwrap();
        assert_eq!(triple_set(&h2), triple_set(&h));
    }
}

// ============================================================================
// Differential correctness: the direct sparq-side decoder (H1–H4,
// the default `load_reader` path) must produce EXACTLY the same triple set AND
// the same number of distinct dictionary terms as the upstream-backed path
// (`Hdt::read` + per-id `id_to_string` translation), on the same bytes. This is
// the load-bearing gate from the optimization plan's HDT §oracle.
// ============================================================================

/// Loads `bytes` both ways (direct decoder vs upstream oracle) and asserts the two
/// graphs are triple-for-triple identical and have the same distinct-term count.
fn assert_direct_matches_upstream(bytes: &[u8], min_triples: usize) {
    let direct =
        sparq_hdt::load_reader(std::io::Cursor::new(bytes.to_vec())).expect("direct decoder load");
    let upstream = sparq_hdt::load_reader_via_upstream(std::io::Cursor::new(bytes.to_vec()))
        .expect("upstream-oracle load");

    let d = triple_set(&direct);
    let u = triple_set(&upstream);
    assert!(
        d.len() >= min_triples,
        "expected >= {min_triples} triples, got {}",
        d.len()
    );
    assert_eq!(
        d, u,
        "direct decoder and upstream oracle must agree triple-for-triple"
    );
    assert_eq!(
        direct.store.len(),
        upstream.store.len(),
        "same stored triple count"
    );
    // The id-translation must intern the same set of distinct terms. The exact
    // sparq ids depend only on first-appearance order, which is identical because
    // both walk SPO order; comparing distinct-term counts plus the triple-set
    // equality above pins the dictionary translation.
    assert_eq!(
        direct.dict.len(),
        upstream.dict.len(),
        "same number of distinct interned terms"
    );
}

/// snikmeta.hdt: a real archive (NOT produced by our writer), 328 triples, whose
/// object section (133 strings, block_size 16) already spans MULTIPLE PFC blocks
/// and whose shared section (43 strings) exercises the shared subject/object range.
#[test]
fn direct_decoder_matches_upstream_snikmeta() {
    let hdt_path = fixture("snikmeta.hdt");
    if !hdt_path.exists() {
        eprintln!("skipping: fixture {} absent", hdt_path.display());
        return;
    }
    let bytes = std::fs::read(&hdt_path).unwrap();
    assert_direct_matches_upstream(&bytes, 328);
}

/// Builds an HDT archive in-process from N-Triples designed to stress the levers:
///  * a SHARED-section-heavy graph: every entity is used as both subject and object
///    (`eN knows eN+1`), so almost all IRIs land in the shared section;
///  * MULTI-BLOCK PFC: > 16 distinct strings per section (block_size 16) so the
///    block-sequential decode (H3) crosses block boundaries with running prefixes;
///  * a term zoo across blocks: langtag/datatyped/plain literals, blank nodes.
///
/// Then asserts the direct decoder == upstream oracle on those exact bytes.
#[test]
fn direct_decoder_matches_upstream_generated_multiblock() {
    use std::fmt::Write as _;
    let mut nt = String::new();
    const N: usize = 200; // >> block_size 16 -> many blocks in every section
    let knows = "<http://xmlns.com/foaf/0.1/knows>";
    let label = "<http://www.w3.org/2000/01/rdf-schema#label>";
    let age = "<http://example.org/age>";
    let height = "<http://example.org/height>";
    for i in 0..N {
        // chain: e{i} knows e{i+1} -> e{i} appears as both subject and object (shared)
        writeln!(
            nt,
            "<http://example.org/e{i}> {knows} <http://example.org/e{}> .",
            i + 1
        )
        .unwrap();
        // language-tagged + datatyped + plain literals, object-only section, many blocks
        writeln!(nt, "<http://example.org/e{i}> {label} \"name {i}\"@en .").unwrap();
        writeln!(
            nt,
            "<http://example.org/e{i}> {age} \"{i}\"^^<http://www.w3.org/2001/XMLSchema#integer> ."
        )
        .unwrap();
        writeln!(nt, "<http://example.org/e{i}> {height} \"{i}.5\"^^<http://www.w3.org/2001/XMLSchema#decimal> .").unwrap();
    }
    // A couple of blank nodes (subject + object positions).
    writeln!(nt, "_:anon0 {knows} <http://example.org/e0> .").unwrap();
    writeln!(nt, "<http://example.org/e0> {knows} _:anon1 .").unwrap();

    let dir = scratch_dir();
    let nt_path = dir.path().join("multiblock.nt");
    std::fs::write(&nt_path, &nt).unwrap();

    let written = hdt::Hdt::read_nt(&nt_path).expect("building multi-block HDT");
    let mut buf: Vec<u8> = Vec::new();
    written.write(&mut buf).expect("serializing HDT");

    // Sanity: confirm the fixture really is multi-block + shared-heavy.
    assert!(
        written.dict.objects.num_strings > written.dict.objects.block_size,
        "objects must span >1 block"
    );
    assert!(
        written.dict.shared.num_strings > written.dict.shared.block_size,
        "shared must span >1 block"
    );

    assert_direct_matches_upstream(&buf, 4 * N);

    // The graph must also equal sparq's own N-Triples load of the SAME source.
    let from_direct = sparq_hdt::load_reader(std::io::Cursor::new(buf)).unwrap();
    let from_nt = Graph::load_str(&nt, "ntriples").unwrap();
    assert_eq!(triple_set(&from_direct), triple_set(&from_nt));
}

/// The direct decoder over a `.hdt.gz` stream yields the same triple set as over
/// the plain bytes (the streaming gzip path feeds the same decoder).
#[test]
fn direct_decoder_gzip_matches_plain() {
    let Some(gz) = gzipped_snikmeta() else { return };
    let from_gz = sparq_hdt::load_reader(std::io::Cursor::new(gz)).unwrap();
    let from_plain = sparq_hdt::load(fixture("snikmeta.hdt")).unwrap();
    assert_eq!(triple_set(&from_gz), triple_set(&from_plain));
    assert_eq!(from_gz.store.len(), from_plain.store.len());
}

/// H5 differential gate: the SAME bytes compressed as gzip, zstd and
/// bzip2 must all decode (streaming) to the IDENTICAL triple set as the plain
/// `.hdt`. Run on the multi-block generated archive so the codec path is fed a
/// realistic-shaped, multi-section dictionary (not just the 10 KB snikmeta).
#[test]
fn all_codecs_decode_to_identical_triple_set() {
    use std::fmt::Write as _;
    use std::io::Write as _;

    // A multi-block, shared-section-heavy graph (same shape as the multiblock
    // oracle test) so the compressed `.hdt` is non-trivial.
    let mut nt = String::new();
    const N: usize = 150;
    let knows = "<http://xmlns.com/foaf/0.1/knows>";
    let label = "<http://www.w3.org/2000/01/rdf-schema#label>";
    for i in 0..N {
        writeln!(
            nt,
            "<http://example.org/e{i}> {knows} <http://example.org/e{}> .",
            i + 1
        )
        .unwrap();
        writeln!(nt, "<http://example.org/e{i}> {label} \"name {i}\"@en .").unwrap();
    }

    let dir = scratch_dir();
    let nt_path = dir.path().join("codecs.nt");
    std::fs::write(&nt_path, &nt).unwrap();
    let written = hdt::Hdt::read_nt(&nt_path).expect("building HDT");
    let mut plain: Vec<u8> = Vec::new();
    written.write(&mut plain).expect("serializing HDT");

    let reference =
        triple_set(&sparq_hdt::load_reader(std::io::Cursor::new(plain.clone())).unwrap());
    assert!(reference.len() >= 2 * N);

    // gzip
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(&plain).unwrap();
    let gz = gz.finish().unwrap();
    // zstd
    let zst = zstd::stream::encode_all(std::io::Cursor::new(plain.clone()), 3).unwrap();
    // bzip2
    let mut bz = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::fast());
    bz.write_all(&plain).unwrap();
    let bz = bz.finish().unwrap();

    for (codec, bytes) in [("gzip", gz), ("zstd", zst), ("bzip2", bz)] {
        let g = sparq_hdt::load_reader(std::io::Cursor::new(bytes))
            .unwrap_or_else(|e| panic!("{codec} decode failed: {e}"));
        assert_eq!(
            triple_set(&g),
            reference,
            "{codec} must decode to the identical triple set as plain .hdt"
        );
    }
}

/// Empty + single-triple HDT archives decode identically on both paths (exercises
/// the zero-section and single-block-of-one edge of the PFC + SPO walk).
#[test]
fn direct_decoder_matches_upstream_tiny() {
    for nt in [
        "", // empty graph: zero strings in every section, zero triples
        "<http://example.org/s> <http://example.org/p> <http://example.org/o> .\n",
    ] {
        let dir = scratch_dir();
        let nt_path = dir.path().join("tiny.nt");
        std::fs::write(&nt_path, nt).unwrap();
        let written = hdt::Hdt::read_nt(&nt_path).expect("building tiny HDT");
        let mut buf: Vec<u8> = Vec::new();
        written.write(&mut buf).expect("serializing tiny HDT");

        let direct = sparq_hdt::load_reader(std::io::Cursor::new(buf.clone())).unwrap();
        let upstream = sparq_hdt::load_reader_via_upstream(std::io::Cursor::new(buf)).unwrap();
        assert_eq!(triple_set(&direct), triple_set(&upstream));
        assert_eq!(direct.store.len(), upstream.store.len());
    }
}

// ============================================================================
// sq-fj7a — (1) HDT-load == N-Triples-load differential and
// (3) a truncated/corrupt-archive REJECTION ORACLE for the id-translation layer.
//
// Audit context: the .hdt -> sparq `Dict` id-translation (decode.rs / lib.rs) is
// the correctness risk and was only round-trip-tested. (1) pins it against a
// fully independent ingest path (sparq's own N-Triples loader) so a translation
// bug that happened to round-trip the same way through both HDT paths would still
// be caught; (3) pins that corruption is REJECTED cleanly (`Result::Err`) — never
// a panic, never a silently mis-decoded / partial Graph. Modeled on sparq-core's
// `build_external_matches_in_memory` (two ingest paths -> identical store) and
// `turtle_path_rejection_oracle` (a corpus of malformed inputs, each rejected).
// ============================================================================

/// Materializes `nt` as an in-process `.hdt` archive (the crate's existing
/// fixture-write path: N-Triples -> FourSectDict PFC + BitmapTriples via the `hdt`
/// crate's `read_nt` + `write`) and returns the archive bytes.
///
/// sq-117n: each call gets its OWN unique, auto-cleaned scratch dir
/// (`scratch_dir()`), so concurrent callers can never race on the same `src.nt`.
/// The `TempDir` lives only for this call: `read_nt` reads the whole file and the
/// archive is returned in memory, so the directory is safely removed on return.
fn nt_to_hdt_bytes(nt: &str) -> Vec<u8> {
    let dir = scratch_dir();
    let nt_path = dir.path().join("src.nt");
    std::fs::write(&nt_path, nt).unwrap();
    let written = hdt::Hdt::read_nt(&nt_path).expect("building HDT from N-Triples");
    let mut buf: Vec<u8> = Vec::new();
    written.write(&mut buf).expect("serializing HDT archive");
    buf
}

/// (1) DIFFERENTIAL: the SAME small graph, materialized as (a) an `.hdt` archive
/// and (b) N-Triples, must load through sparq to the IDENTICAL term set — proving
/// the HDT id-translation produces exactly the triples a direct N-Triples ingest
/// does. Several graphs spanning the dictionary shapes that drive the translation:
/// shared subject/object terms (HDT's shared section), the literal zoo
/// (plain / langtag / datatyped, incl. sparq's inline-integer path), blank nodes
/// in subject AND object position, and a non-ASCII / shared-prefix IRI cluster.
///
/// Blank nodes: both paths derive from the SAME N-Triples source, and HDT stores
/// `_:label` verbatim while sparq's NT loader interns the identical labels, so the
/// bnode labels coincide and a direct term-set comparison is exact — no separate
/// isomorphism step is needed for these fixtures (the labels ARE preserved).
#[test]
fn hdt_load_matches_ntriples_load() {
    let graphs: &[(&str, &str)] = &[
        // shared subject/object terms: each entity is both S and O (HDT shared section)
        (
            "shared",
            "<http://example.org/alice> <http://xmlns.com/foaf/0.1/knows> <http://example.org/bob> .\n\
             <http://example.org/bob> <http://xmlns.com/foaf/0.1/knows> <http://example.org/carol> .\n\
             <http://example.org/carol> <http://xmlns.com/foaf/0.1/knows> <http://example.org/alice> .\n",
        ),
        // literal zoo: plain, language-tagged, xsd:string, datatyped decimal, and an
        // integer (sparq's inline-literal path). NB: the lang tag is lowercase in the
        // source — the upstream `hdt` writer stores tags VERBATIM (no BCP47
        // case-normalization), whereas BOTH sparq loaders lowercase, so a mixed-case
        // source tests the writer's behaviour, not the id-translation under test.
        (
            "literals",
            "<http://example.org/s> <http://example.org/plain> \"plain\" .\n\
             <http://example.org/s> <http://www.w3.org/2000/01/rdf-schema#label> \"Caf\u{00e9}\"@fr .\n\
             <http://example.org/s> <http://example.org/n> \"42\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n\
             <http://example.org/s> <http://example.org/h> \"1.83\"^^<http://www.w3.org/2001/XMLSchema#decimal> .\n\
             <http://example.org/s> <http://example.org/u> \"snowman \u{2603}\" .\n",
        ),
        // blank nodes in subject AND object position, plus a shared IRI.
        (
            "blanks",
            "_:b0 <http://xmlns.com/foaf/0.1/knows> <http://example.org/alice> .\n\
             <http://example.org/alice> <http://example.org/link> _:b1 .\n\
             _:b0 <http://example.org/link> _:b1 .\n\
             _:b1 <http://xmlns.com/foaf/0.1/name> \"anon\" .\n",
        ),
        // shared-prefix IRIs (PFC front-coding does real work) + non-ASCII path.
        (
            "prefixes",
            "<http://example.org/path/to/a> <http://example.org/p> <http://example.org/path/to/b> .\n\
             <http://example.org/path/to/b> <http://example.org/p> <http://example.org/path/to/caf\u{00e9}> .\n\
             <http://example.org/path/to/caf\u{00e9}> <http://example.org/p> <http://example.org/path/to/a> .\n",
        ),
    ];

    for (name, nt) in graphs {
        let hdt_bytes = nt_to_hdt_bytes(nt);
        let from_hdt = sparq_hdt::load_reader(std::io::Cursor::new(hdt_bytes))
            .unwrap_or_else(|e| panic!("[{name}] loading generated HDT: {e}"));
        let from_nt = Graph::load_str(nt, "ntriples")
            .unwrap_or_else(|e| panic!("[{name}] loading source N-Triples: {e}"));
        assert_eq!(
            triple_set(&from_hdt),
            triple_set(&from_nt),
            "[{name}] HDT id-translation must yield the same term set as direct N-Triples ingest",
        );
        assert_eq!(
            from_hdt.store.len(),
            from_nt.store.len(),
            "[{name}] same stored triple count",
        );
        assert_eq!(
            from_hdt.dict.len(),
            from_nt.dict.len(),
            "[{name}] same number of distinct interned terms",
        );

        // (2) The SAME archive gzipped must term-equal the plain archive (the
        // streaming `.hdt.gz` path feeds the identical decoder), exercised on each
        // differential graph rather than only the snikmeta fixture.
        let plain = nt_to_hdt_bytes(nt);
        use std::io::Write as _;
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&plain).unwrap();
        let gz = gz.finish().unwrap();
        let from_gz = sparq_hdt::load_reader(std::io::Cursor::new(gz))
            .unwrap_or_else(|e| panic!("[{name}] gz decode: {e}"));
        assert_eq!(
            triple_set(&from_gz),
            triple_set(&from_hdt),
            "[{name}] .hdt.gz must decode to the identical term set as plain .hdt",
        );
    }
}

/// (1, PFC edges) The translation must survive the Plain-Front-Coding dictionary
/// edges the audit calls out: runs of terms sharing LONG common prefixes (so most
/// entries are pure front-coded deltas — `vbyte(prefix_len) ++ suffix`, the path
/// `decode_section` reconstructs against its running buffer) and an EMPTY-STRING
/// lexical form (the empty literal `""`, a zero-length dictionary string).
///
/// (The empty IRI `<>` — the other zero-length-string edge — is NOT reachable
/// through this fixture path: the `hdt` writer's N-Triples reader (oxttl) rejects
/// `<>` as `NoScheme`, so no archive can be built containing it. The empty-string
/// term-intern path is unit-tested directly in `lib.rs::tests::term_shapes`.)
#[test]
fn pfc_dictionary_edges_translate_identically() {
    // A deep shared-prefix cluster: consecutive sorted terms differ only in their
    // last byte, so PFC stores each as a 1-byte suffix off a long common prefix.
    // Plus an empty-string literal `""` (zero-length lexical form).
    let nt = "<http://example.org/path/to/resource/a> <http://example.org/p> <http://example.org/path/to/resource/aa> .\n\
              <http://example.org/path/to/resource/aa> <http://example.org/p> <http://example.org/path/to/resource/aaa> .\n\
              <http://example.org/path/to/resource/aaa> <http://example.org/p> <http://example.org/path/to/resource/aaab> .\n\
              <http://example.org/path/to/resource/aaab> <http://example.org/p> <http://example.org/path/to/resource/aaac> .\n\
              <http://example.org/path/to/resource/a> <http://example.org/empty> \"\" .\n";

    let hdt_bytes = nt_to_hdt_bytes(nt);
    let from_hdt = sparq_hdt::load_reader(std::io::Cursor::new(hdt_bytes.clone())).unwrap();
    let from_nt = Graph::load_str(nt, "ntriples").unwrap();
    assert_eq!(
        triple_set(&from_hdt),
        triple_set(&from_nt),
        "shared-prefix + empty-string PFC entries must translate to the same terms",
    );
    // The empty literal must survive as `""`, not be dropped or mangled.
    assert!(
        triple_set(&from_hdt).iter().any(|[_, _, o]| o == "\"\""),
        "the empty literal `\"\"` must be preserved",
    );
    // And direct == upstream on these exact bytes (the id-translation oracle).
    assert_direct_matches_upstream(&hdt_bytes, 5);
}

/// sq-qalqs — escaped-literal round-trip oracle: pins WHICH side mangles
/// a literal containing escaped quotes / backslashes on the `Hdt::read_nt` -> `write`
/// -> `sparq_hdt::load` fixture path, and encodes the UPSTREAM limitation exactly.
///
/// Finding: the WRAPPED writer is the mangler, not sparq's decode. The HDT spec (and
/// hdt-cpp / hdt-java) stores literal lexical forms RAW in the dictionary; upstream
/// `hdt`'s `FourSectDict::read_nt` instead stores the parsed term rendering —
/// the N-Triples-ESCAPED form (`\"` / `\\` as literal backslash sequences). A
/// spec-conformant reader (sparq's direct decoder AND the upstream-backed oracle
/// alike — both treat the stored bytes as raw) therefore decodes a lexical form that
/// still carries those backslashes, i.e. the source lexical escaped ONCE more than
/// `Graph::load_str` yields. Same family as the writer's verbatim (non-lowercased)
/// language tags noted in `hdt_load_matches_ntriples_load`. Documented in
/// `UPSTREAM.md` item 3; archives written by hdt-cpp/hdt-java or by sparq's own
/// `save` (see `write_roundtrip.rs`) are NOT affected.
#[test]
fn escaped_literal_pins_upstream_writer_as_the_mangler() {
    // Raw lexical form under test: he said "hi" and \ done
    let raw = r#"he said "hi" and \ done"#;
    // One round of N-Triples escaping (\ -> \\, " -> \").
    let nt_escape = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let escaped = nt_escape(raw);
    let nt = format!("<http://example.org/s> <http://example.org/p> \"{escaped}\" .\n");

    // Ground truth: sparq's own N-Triples loader unescapes to the raw lexical form
    // (its N-Triples term rendering re-escapes exactly once).
    let from_nt = Graph::load_str(&nt, "ntriples").unwrap();
    let want = format!("\"{escaped}\"");
    assert_eq!(
        triple_set(&from_nt).iter().next().unwrap()[2],
        want,
        "ground truth: Graph::load_str object rendering"
    );

    // (1) The WRITER is the mangler: the dictionary string `read_nt` stores is the
    // ESCAPED rendering, not the raw lexical form the HDT spec prescribes.
    let dir = scratch_dir();
    let nt_path = dir.path().join("escaped.nt");
    std::fs::write(&nt_path, &nt).unwrap();
    let written = hdt::Hdt::read_nt(&nt_path).expect("building HDT from N-Triples");
    let stored = written
        .dict
        .id_to_string(1, hdt::IdKind::Object)
        .expect("the single object term");
    assert_eq!(
        stored,
        format!("\"{escaped}\""),
        "upstream read_nt stores the N-Triples-ESCAPED lexical form (raw per spec); \
         if this fails with the RAW form instead, upstream fixed it — drop UPSTREAM.md \
         item 3 and turn this test into an exact round-trip equality"
    );

    // (2) sparq's decode is NOT a second mangler: the direct decoder and the
    // upstream-backed oracle agree byte-faithfully on the same archive…
    let mut buf: Vec<u8> = Vec::new();
    written.write(&mut buf).expect("serializing HDT archive");
    let direct = sparq_hdt::load_reader(std::io::Cursor::new(buf.clone())).unwrap();
    let upstream = sparq_hdt::load_reader_via_upstream(std::io::Cursor::new(buf)).unwrap();
    assert_eq!(
        triple_set(&direct),
        triple_set(&upstream),
        "both decode paths must agree on the same bytes (the divergence is in the writer)"
    );

    // (3) …and the round-tripped object is the source lexical escaped ONE extra
    // time (the stored escapes survive as literal characters), NOT the ground truth.
    let got = triple_set(&direct).iter().next().unwrap()[2].clone();
    assert_eq!(
        got,
        format!("\"{}\"", nt_escape(&escaped)),
        "round trip through the upstream writer yields the double-escaped rendering"
    );
    assert_ne!(got, want, "the documented mismatch against Graph::load_str");
}

// (3) REJECTION ORACLE: a CORRUPT archive must be rejected with a clean
// `Result::Err` — never a panic, never a silently mis-decoded / partial Graph.
//
// `classify` wraps each load in `catch_unwind` so a PANIC is observed (and fails
// the test) rather than aborting the run, exactly as a robustness oracle must:
// "rejected" means `Err`, and a panic is a DISTINCT, worse failure.
//
// Empirical finding (sq-fj7a): on the default direct decoder (`load_reader`) and
// the upstream-backed oracle (`load_reader_via_upstream`) alike, EVERY prefix
// truncation and EVERY single-byte flip of a valid archive is rejected with
// `Err` — no panic, and no truncation ever yields a partial graph. So these
// assertions are exact, not `#[ignore]`d; no robustness bug was found.

/// Loads `bytes` via the direct decoder, classifying the outcome. A panic is
/// caught and surfaced as a separate variant so the oracle can forbid it.
fn classify(bytes: &[u8]) -> Result<usize, String> {
    // Cursor<Vec<u8>> is UnwindSafe; the loader holds no cross-unwind state.
    let owned = bytes.to_vec();
    match std::panic::catch_unwind(move || sparq_hdt::load_reader(std::io::Cursor::new(owned))) {
        Ok(Ok(g)) => Ok(g.store.len()),
        Ok(Err(e)) => Err(format!("Err: {e}")),
        Err(_) => Err("PANIC".to_string()),
    }
}

/// Same, via the upstream-backed oracle path (the per-id `id_to_string` translation
/// over a fully-built `Hdt`) — the path the audit flags as id-translation risk.
fn classify_upstream(bytes: &[u8]) -> Result<usize, String> {
    let owned = bytes.to_vec();
    match std::panic::catch_unwind(move || {
        sparq_hdt::load_reader_via_upstream(std::io::Cursor::new(owned))
    }) {
        Ok(Ok(g)) => Ok(g.store.len()),
        Ok(Err(e)) => Err(format!("Err: {e}")),
        Err(_) => Err("PANIC".to_string()),
    }
    // (kept multi-line: the upstream path name is too long for a single-line closure)
}

/// A valid multi-section archive for the corruption corpus: shared terms, a literal
/// zoo, blank nodes — so truncations land in non-trivial dictionary + triples bytes.
fn valid_corruptible_hdt() -> Vec<u8> {
    let nt = "<http://example.org/alice> <http://xmlns.com/foaf/0.1/knows> <http://example.org/bob> .\n\
              <http://example.org/bob> <http://xmlns.com/foaf/0.1/knows> <http://example.org/alice> .\n\
              <http://example.org/alice> <http://xmlns.com/foaf/0.1/name> \"Alice\" .\n\
              <http://example.org/alice> <http://example.org/age> \"42\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n\
              <http://example.org/alice> <http://www.w3.org/2000/01/rdf-schema#label> \"Alice\"@en .\n\
              _:b0 <http://xmlns.com/foaf/0.1/knows> <http://example.org/alice> .\n\
              _:b0 <http://example.org/link> _:b1 .\n\
              _:b1 <http://xmlns.com/foaf/0.1/name> \"anon\" .\n";
    nt_to_hdt_bytes(nt)
}

/// Named, targeted corruptions: bad magic and a truncation landing in each of the
/// header / dictionary / triples regions. The valid archive is laid out
/// `$HDT`-cookie -> Global+Header control info -> FourSectDict -> BitmapTriples, so
/// fractional cut points fall, in order, in the header, the dictionary, and the
/// triples section; each MUST be `Err` (not Ok, not panic).
#[test]
fn rejection_oracle_named_corruptions() {
    let valid = valid_corruptible_hdt();
    // Sanity: the pristine archive loads (so rejections below are not vacuous).
    assert!(classify(&valid).is_ok(), "the pristine archive must load");

    // --- bad magic: clobber the `$HDT` cookie ---
    let mut bad_magic = valid.clone();
    bad_magic[0] = b'X';
    assert!(
        matches!(classify(&bad_magic), Err(ref e) if e != "PANIC"),
        "bad magic must be a clean Err, got {:?}",
        classify(&bad_magic),
    );
    assert!(matches!(classify_upstream(&bad_magic), Err(ref e) if e != "PANIC"));

    // --- empty / sub-cookie input ---
    for n in [0usize, 1, 2, 3, 4] {
        assert!(
            matches!(classify(&valid[..n]), Err(ref e) if e != "PANIC"),
            "{n}-byte prefix must be a clean Err",
        );
    }

    // --- truncated header / dictionary / triples (fractional cut points) ---
    for (region, num, den) in [("header", 1, 10), ("dictionary", 1, 2), ("triples", 9, 10)] {
        let cut = valid.len() * num / den;
        let truncated = &valid[..cut];
        let r = classify(truncated);
        assert!(
            matches!(r, Err(ref e) if e != "PANIC"),
            "truncated {region} (cut at {cut}/{}) must be a clean Err, got {r:?}",
            valid.len(),
        );
        // The id-translation oracle path must also reject it cleanly.
        let ru = classify_upstream(truncated);
        assert!(
            matches!(ru, Err(ref e) if e != "PANIC"),
            "truncated {region} via upstream oracle must be a clean Err, got {ru:?}",
        );
    }

    // --- a bare valid header but no dictionary/triples at all (cut right after the
    //     header region) must NOT produce a (partial) graph ---
    let head_only = &valid[..valid.len() / 5];
    assert!(
        matches!(classify(head_only), Err(ref e) if e != "PANIC"),
        "header-only stream must be rejected, never a partial graph",
    );
}

/// (3) EXHAUSTIVE truncation invariant: NO prefix of a valid archive may panic,
/// and none may silently decode to a (necessarily partial) graph — every strict
/// prefix must be `Err`. This is the strong form of "never a silently mis-decoded
/// / partial Graph": a truncation that the CRCs / length fields fail to catch would
/// show up here as an `Ok`, and any out-of-bounds slice as a `PANIC`.
#[test]
fn rejection_oracle_no_truncation_panics_or_partial_decodes() {
    let valid = valid_corruptible_hdt();
    let n = valid.len();
    let full = classify(&valid).expect("pristine archive loads");

    let mut ok_lengths: Vec<usize> = Vec::new();
    let mut panic_lengths: Vec<usize> = Vec::new();
    for len in 0..n {
        match classify(&valid[..len]) {
            Ok(_) => ok_lengths.push(len),
            Err(ref e) if e == "PANIC" => panic_lengths.push(len),
            Err(_) => {}
        }
    }
    assert!(
        panic_lengths.is_empty(),
        "truncation must never panic; panicked at prefix lengths {:?}",
        &panic_lengths[..panic_lengths.len().min(20)],
    );
    assert!(
        ok_lengths.is_empty(),
        "no STRICT prefix may decode to a (partial) graph; accepted at lengths {:?}",
        &ok_lengths[..ok_lengths.len().min(20)],
    );
    // The only length that loads is the full archive.
    assert!(classify(&valid).is_ok() && full > 0);
}

/// (3) Single-byte-flip invariant: flipping any one byte of a valid archive must
/// never PANIC — corruption that defeats the CRCs may either error or (rarely, for
/// a flip in a benign field) still decode, but a panic is never acceptable. (The
/// dictionary/triples bytes are CRC-protected, so most flips are caught as `Err`.)
#[test]
fn rejection_oracle_no_single_byte_flip_panics() {
    let valid = valid_corruptible_hdt();
    let mut panicked: Vec<usize> = Vec::new();
    for i in 0..valid.len() {
        let mut b = valid.clone();
        b[i] ^= 0xFF;
        if matches!(classify(&b), Err(ref e) if e == "PANIC") {
            panicked.push(i);
        }
    }
    assert!(
        panicked.is_empty(),
        "single-byte corruption must never panic; panicked at byte offsets {:?}",
        &panicked[..panicked.len().min(20)],
    );
}
