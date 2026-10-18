//! Differential guard for the SPILLED-dictionary external build (`dict-spill`):
//! the same N-Triples document is built through
//!
//!   1. the sharded in-RAM external build (`build_external_opts(.., true)`, the
//!      default path), and
//!   2. the spilled-dictionary external build (`build_external_spill`) with a TINY
//!      memory budget — so the dedup caches clear constantly (many epochs) and every
//!      external sort spills across many runs,
//!
//! and every output file must be BYTE-IDENTICAL (same id assignment, same dictionary
//! blob, same permutations, same numeric/temporal caches) — the strongest form of the
//! repo's `build_external_matches_in_memory` pattern. The opened spilled store must
//! also answer queries identically to a plain in-memory load.

use sparq_core::{dictspill::SpillConfig, Graph};
use sparq_engine::query;

/// Synthetic data with HIGH distinct-term variety: unique IRIs and literals per line
/// (the dict-heavy regime), plus every term-kind edge case the dictionary stores —
/// language tags, typed literals (numeric, temporal, decimal, large integers), inline
/// integers, escaped strings, blank nodes, multiple namespaces, an IRI with no
/// prefix split, and exact duplicate lines (must dedup identically).
fn synthetic_nt(n: u32) -> String {
    let mut s = String::new();
    for i in 0..n {
        s.push_str(&format!("<http://ex/s{i}> <http://ex/name> \"unique value number {i} \\\"q\\\" \\u00e9\" .\n"));
        s.push_str(&format!("<http://ex/s{i}> <http://ex/age> \"{}\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n", i % 90));
        s.push_str(&format!("<http://ex/s{i}> <http://other.org/v#big> \"{}\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n", 9_007_199_254_740_000u64 + i as u64));
        s.push_str(&format!("<http://ex/s{i}> <http://ex/dec> \"{}.25\"^^<http://www.w3.org/2001/XMLSchema#decimal> .\n", i));
        s.push_str(&format!("<http://ex/s{i}> <http://ex/when> \"2026-01-{:02}T0{}:00:00Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime> .\n", 1 + i % 28, i % 10));
        s.push_str(&format!("<http://ex/s{i}> <http://ex/day> \"2026-02-{:02}\"^^<http://www.w3.org/2001/XMLSchema#date> .\n", 1 + i % 28));
        s.push_str(&format!("<http://ex/s{i}> <http://ex/label> \"étiquette {i}\"@fr .\n"));
        s.push_str(&format!("<http://ex/s{i}> <http://ex/follows> <http://ex/s{}> .\n", (i * 7 + 3) % n.max(1)));
        s.push_str(&format!("_:b{i} <http://ex/about> <http://ex/s{i}> .\n"));
        if i % 3 == 0 {
            s.push_str(&format!("<urn:uuid:item-{i}> <http://ex/idx> \"{i}\" .\n"));
        }
    }
    // Exact duplicates + shared terms across distant lines (cache-epoch crossers).
    s.push_str("<http://ex/s0> <http://ex/name> \"unique value number 0 \\\"q\\\" \\u00e9\" .\n");
    s.push_str("<http://ex/s0> <http://ex/age> \"0\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n");
    for i in 0..n.min(50) {
        s.push_str(&format!("<http://ex/s{i}> <http://ex/name> \"unique value number {i} \\\"q\\\" \\u00e9\" .\n"));
    }
    s
}

/// The data files of the store at `dir`: every file but its marker (`sparq-store`), which
/// records the store's own random identity and so differs between any two stores. The marker is
/// checked on its own instead ([`assert_root_marker`]).
fn dir_files(dir: &std::path::Path) -> Vec<String> {
    assert_root_marker(dir);
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name != "sparq-store")
        .collect();
    v.sort();
    v
}

/// The store at `dir` carries its marker, naming it a root store with an identity
/// (`root <32 lowercase hex digits>`).
fn assert_root_marker(dir: &std::path::Path) {
    let marker = std::fs::read_to_string(dir.join("sparq-store"))
        .unwrap_or_else(|e| panic!("{} has no store marker: {e}", dir.display()));
    let id = marker
        .strip_suffix('\n')
        .and_then(|m| m.strip_prefix("root "))
        .unwrap_or_else(|| panic!("{}: not a root marker: {marker:?}", dir.display()));
    assert!(
        id.len() == 32 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')),
        "{}: not a store identity: {id:?}",
        dir.display()
    );
}

#[test]
fn spilled_dict_build_is_byte_identical_to_sharded() {
    let nt = synthetic_nt(700);
    let base = std::env::temp_dir().join(format!("sparq_dsp_diff_{}", std::process::id()));
    std::fs::remove_dir_all(&base).ok();
    let sharded_dir = base.join("sharded");
    let spill_dir = base.join("spill");

    // Reference: the default sharded in-RAM external build.
    Graph::build_external_opts(nt.as_bytes(), "ntriples", &sharded_dir, 512, true).unwrap();

    // Spilled build with a TINY budget: per-shard caches floor at 4 KiB (constant
    // epoch clears), the variable-record sorter floors at 64 KiB runs (many runs),
    // and `chunk = 512` spills triples across many runs too.
    let cfg = SpillConfig { mem_budget: 1, disk_floor: 0 };
    Graph::build_external_spill(nt.as_bytes(), "ntriples", &spill_dir, 512, &cfg).unwrap();

    // Identical file SETS, identical BYTES.
    let files = dir_files(&sharded_dir);
    assert_eq!(files, dir_files(&spill_dir), "output file sets differ");
    assert!(files.iter().any(|f| f == "dict-terms.bin"));
    for f in &files {
        let a = std::fs::read(sharded_dir.join(f)).unwrap();
        let b = std::fs::read(spill_dir.join(f)).unwrap();
        assert!(a == b, "{f} differs between sharded and spilled builds ({} vs {} bytes)", a.len(), b.len());
    }

    // The spilled store answers like a plain in-memory load (semantic guard on top of
    // the byte guard).
    let mem = Graph::load_str(&nt, "ntriples").unwrap();
    let ext = Graph::open(&spill_dir).unwrap();
    assert_eq!(mem.len(), ext.len(), "triple count differs");
    assert_eq!(mem.dict().len(), ext.dict().len(), "dict size differs");
    let dump = |g: &Graph| -> Vec<String> {
        let scan = g.store().scan(&[None, None, None]);
        let mut v: Vec<String> = scan
            .rows
            .iter()
            .map(|r| {
                let spo = scan.to_spo(r);
                format!("{} {} {}", g.dict().term(spo[0]), g.dict().term(spo[1]), g.dict().term(spo[2]))
            })
            .collect();
        v.sort();
        v
    };
    assert_eq!(dump(&mem), dump(&ext), "term-level content differs");

    for (q, expect_nonzero) in [
        ("SELECT (COUNT(*) AS ?c) WHERE { ?s ?p ?o }", true),
        ("SELECT ?s WHERE { ?s <http://ex/age> ?a . FILTER(?a > 40) }", true),
        ("SELECT ?s WHERE { ?s <http://ex/when> ?t . FILTER(?t > \"2026-01-15T00:00:00Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime>) }", true),
        ("SELECT ?l WHERE { <http://ex/s3> <http://ex/label> ?l }", true),
    ] {
        let a = query(&mem, q).unwrap().rows.len();
        let b = query(&ext, q).unwrap().rows.len();
        assert_eq!(a, b, "query result differs for {q}");
        if expect_nonzero {
            assert!(b > 0, "query unexpectedly empty: {q}");
        }
    }

    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn spilled_dict_build_comfortable_budget_matches_too() {
    // A budget large enough that the caches never clear (single epoch, in-RAM-sized
    // sorts): the degenerate no-spill configuration must produce the same bytes.
    let nt = synthetic_nt(120);
    let base = std::env::temp_dir().join(format!("sparq_dsp_diff2_{}", std::process::id()));
    std::fs::remove_dir_all(&base).ok();
    let sharded_dir = base.join("sharded");
    let spill_dir = base.join("spill");
    Graph::build_external_opts(nt.as_bytes(), "ntriples", &sharded_dir, 1 << 20, true).unwrap();
    let cfg = SpillConfig { mem_budget: 256 << 20, disk_floor: 0 };
    Graph::build_external_spill(nt.as_bytes(), "ntriples", &spill_dir, 1 << 20, &cfg).unwrap();
    for f in dir_files(&sharded_dir) {
        let a = std::fs::read(sharded_dir.join(&f)).unwrap();
        let b = std::fs::read(spill_dir.join(&f)).unwrap();
        assert!(a == b, "{f} differs (comfortable budget)");
    }
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn spilled_dict_build_rejects_non_ntriples() {
    let base = std::env::temp_dir().join(format!("sparq_dsp_fmt_{}", std::process::id()));
    let cfg = SpillConfig { mem_budget: 1 << 20, disk_floor: 0 };
    let err = Graph::build_external_spill("<a> <b> <c> .".as_bytes(), "turtle", &base, 16, &cfg).unwrap_err();
    assert!(err.contains("N-Triples"), "unexpected error: {err}");
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn spilled_dict_build_empty_input() {
    let base = std::env::temp_dir().join(format!("sparq_dsp_empty_{}", std::process::id()));
    std::fs::remove_dir_all(&base).ok();
    let cfg = SpillConfig { mem_budget: 1 << 20, disk_floor: 0 };
    Graph::build_external_spill("".as_bytes(), "ntriples", &base, 16, &cfg).unwrap();
    let g = Graph::open(&base).unwrap();
    assert_eq!(g.len(), 0);
    assert_eq!(g.dict().len(), 0);
    std::fs::remove_dir_all(&base).ok();
}
