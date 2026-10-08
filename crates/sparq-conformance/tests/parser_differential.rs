//! [FABLE-5] (sq-tonhr.2, epic sq-tonhr) The candidate-vs-incumbent parser DIFFERENTIAL
//! gate for N-Triples / N-Quads / TriG, plus the MUTATION proof that the harness is
//! non-vacuous. Three layers:
//!
//! 1. **Green on incumbents** — the REAL sparq ingest paths (native `nt.rs`, the
//!    chunk-parallel dataset loaders) differentially compared against serial oxttl over
//!    (a) every `mf:action` of the W3C rdf-n-triples / rdf-n-quads / rdf-trig suites and
//!    (b) the committed fuzz seed corpus (`fuzz/seeds/`). The known native-parser
//!    divergences (bead sq-w64x5 — the same cases the `rdf_line_syntax_ratchet` floors
//!    record) are pinned as an EXACT adjudicated set: a NEW divergence fails, and a
//!    divergence that disappears fails too (fixing sq-w64x5 must prune the list AND
//!    raise the ratchet floors — no silent drift in either direction).
//! 2. **Mutation non-vacuity** — deliberately seeded divergent parsers (a quad-dropping
//!    mutant, a leniently-accepting mutant, a term-mangling mutant) MUST be detected by
//!    the same harness entry points a real candidate will run through, and the reported
//!    minimal repro MUST shrink to the single diverging line.
//! 3. The W3C-data-dependent tests self-skip when `tests/w3c/rdf-tests` is not fetched
//!    (`scripts/fetch-conformance.sh`); the fuzz-seed + mutation tests are hermetic
//!    (repo-committed inputs only) and always run.
//!
//! This is the reusable zero-regression gate the rdf-shuttle generated parsers
//! (sq-tonhr.8/.9) slot into: replace one side with the candidate, require an empty
//! divergence set.

use sparq_conformance::differential::{
    compare_doc, run_dir, run_suite_actions, DiffParser, DivergenceKind,
};
use sparq_conformance::quadset::{dataset_quads, quad_strings};
use std::collections::BTreeSet;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// The parser pairs under differential test.
// ---------------------------------------------------------------------------

/// The REAL sparq N-Triples ingest path (native chunk-parallel `nt.rs` under the
/// default `parallel` feature).
fn sparq_nt() -> DiffParser<'static> {
    DiffParser::new("sparq native nt.rs", |text, _base| {
        let (dict, ids) = sparq_core::Graph::parse_to_triples(text, "ntriples")?;
        Ok(ids
            .iter()
            .map(|&[s, p, o]| {
                [
                    dict.term(s).to_string(),
                    dict.term(p).to_string(),
                    dict.term(o).to_string(),
                    String::new(),
                ]
            })
            .collect())
    })
}

/// Reference: serial oxttl N-Triples.
fn oxttl_nt() -> DiffParser<'static> {
    DiffParser::new("oxttl NTriplesParser (serial)", |text, _base| {
        let mut quads = Vec::new();
        for t in oxttl::NTriplesParser::new().for_slice(text.as_bytes()) {
            let t = t.map_err(|e| e.to_string())?;
            quads.push(quad_strings(&oxrdf::Quad::new(
                t.subject,
                t.predicate,
                t.object,
                oxrdf::GraphName::DefaultGraph,
            )));
        }
        Ok(quads)
    })
}

/// The REAL sparq N-Quads ingest path (chunk-parallel dataset loader, named graphs
/// preserved).
fn sparq_nq() -> DiffParser<'static> {
    DiffParser::new("sparq load_dataset nquads (parallel)", |text, _base| {
        sparq_core::Graph::load_dataset(text, "nquads").map(|g| dataset_quads(&g))
    })
}

/// Reference: serial oxttl N-Quads.
fn oxttl_nq() -> DiffParser<'static> {
    DiffParser::new("oxttl NQuadsParser (serial)", |text, _base| {
        let mut quads = Vec::new();
        for q in oxttl::NQuadsParser::new().for_slice(text.as_bytes()) {
            quads.push(quad_strings(&q.map_err(|e| e.to_string())?));
        }
        Ok(quads)
    })
}

/// The REAL sparq TriG ingest path with a base IRI (the serial with-base dataset
/// loader the rdf-trig conformance lane drives), named graphs preserved.
fn sparq_trig() -> DiffParser<'static> {
    DiffParser::new("sparq load_dataset_with_base trig", |text, base| {
        sparq_core::Graph::load_dataset_with_base(text, "trig", base).map(|g| dataset_quads(&g))
    })
}

/// Reference: oxttl TriG with the same base.
fn oxttl_trig() -> DiffParser<'static> {
    DiffParser::new("oxttl TriGParser (serial, with base)", |text, base| {
        let parser = oxttl::TriGParser::new()
            .with_base_iri(base)
            .map_err(|e| format!("invalid base IRI {base:?}: {e}"))?;
        let mut quads = Vec::new();
        for q in parser.for_slice(text.as_bytes()) {
            quads.push(quad_strings(&q.map_err(|e| e.to_string())?));
        }
        Ok(quads)
    })
}

/// The REAL sparq chunk-parallel TriG path (no base — for absolute-IRI corpora like the
/// fuzz seeds; exercises the `trig_chunks` splitter differentially).
fn sparq_trig_parallel() -> DiffParser<'static> {
    DiffParser::new("sparq load_dataset trig (parallel)", |text, _base| {
        sparq_core::Graph::load_dataset(text, "trig").map(|g| dataset_quads(&g))
    })
}

/// The REAL sparq Turtle ingest path (chunk-parallel; triples land in the default graph).
fn sparq_turtle() -> DiffParser<'static> {
    DiffParser::new("sparq parse_to_triples turtle (parallel)", |text, _base| {
        let (dict, ids) = sparq_core::Graph::parse_to_triples(text, "turtle")?;
        Ok(ids
            .iter()
            .map(|&[s, p, o]| {
                [
                    dict.term(s).to_string(),
                    dict.term(p).to_string(),
                    dict.term(o).to_string(),
                    String::new(),
                ]
            })
            .collect())
    })
}

/// Reference: serial oxttl Turtle (no base — fuzz-seed corpus only).
fn oxttl_turtle() -> DiffParser<'static> {
    DiffParser::new("oxttl TurtleParser (serial)", |text, _base| {
        let mut quads = Vec::new();
        for t in oxttl::TurtleParser::new().for_slice(text.as_bytes()) {
            let t = t.map_err(|e| e.to_string())?;
            quads.push(quad_strings(&oxrdf::Quad::new(
                t.subject,
                t.predicate,
                t.object,
                oxrdf::GraphName::DefaultGraph,
            )));
        }
        Ok(quads)
    })
}

// ---------------------------------------------------------------------------
// Corpus locations.
// ---------------------------------------------------------------------------

fn w3c_suite_root(dir: &str) -> Option<PathBuf> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/w3c/rdf-tests/rdf/rdf11")
        .join(dir);
    match p.canonicalize() {
        Ok(p) if p.join("manifest.ttl").is_file() => Some(p),
        _ => {
            eprintln!("SKIP: W3C {dir} data not fetched (run scripts/fetch-conformance.sh)");
            None
        }
    }
}

fn fuzz_seeds_dir(target: &str) -> Option<PathBuf> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fuzz/seeds")
        .join(target);
    p.canonicalize().ok().filter(|p| p.is_dir())
}

/// The file stems of the divergences a report is ADJUDICATED to contain (exact set).
fn divergent_stems(report: &sparq_conformance::differential::DiffReport) -> BTreeSet<String> {
    report
        .divergences
        .iter()
        .map(|d| {
            PathBuf::from(&d.label)
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_else(|| d.label.clone())
        })
        .collect()
}

fn stems(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| s.to_string()).collect()
}

// ---------------------------------------------------------------------------
// 1. Green on incumbents — W3C suite actions.
// ---------------------------------------------------------------------------

/// The adjudicated native-nt.rs divergence set on the rdf-n-triples actions (bead
/// sq-w64x5; the SAME cases the `rdf_line_syntax_ratchet` NT floor records: 9 lenient
/// accepts + 1 strict reject).
const NT_ADJUDICATED: &[&str] = &[
    "nt-syntax-bad-uri-01.nt",
    "nt-syntax-bad-uri-04.nt",
    "nt-syntax-bad-uri-06.nt",
    "nt-syntax-bad-uri-07.nt",
    "nt-syntax-bad-uri-08.nt",
    "nt-syntax-bad-uri-09.nt",
    "nt-syntax-bad-bnode-01.nt",
    "nt-syntax-bad-bnode-02.nt",
    "nt-syntax-bad-lang-01.nt",
    "minimal_whitespace.nt",
];

#[test]
fn differential_nt_native_vs_oxttl_over_w3c_actions() {
    let Some(root) = w3c_suite_root("rdf-n-triples") else {
        return;
    };
    let report = run_suite_actions(&sparq_nt(), &oxttl_nt(), &root).expect("suite walked");
    assert!(
        report.compared >= 70,
        "suite shrank: {} actions",
        report.compared
    );
    assert!(
        report.unverified.is_empty(),
        "unverified inputs: {:?}",
        report.unverified
    );
    assert_eq!(
        divergent_stems(&report),
        stems(NT_ADJUDICATED),
        "adjudicated divergence set drifted (new divergence = regression; disappeared \
         divergence = sq-w64x5 progress — prune the list AND raise the ratchet floor).\n{}",
        report.describe()
    );
    // Every adjudicated divergence must carry a non-empty repro that itself diverges.
    for d in &report.divergences {
        assert!(!d.minimal_repro.is_empty(), "empty repro for {}", d.label);
    }
}

#[test]
fn differential_nq_native_vs_oxttl_over_w3c_actions() {
    let Some(root) = w3c_suite_root("rdf-n-quads") else {
        return;
    };
    let report = run_suite_actions(&sparq_nq(), &oxttl_nq(), &root).expect("suite walked");
    assert!(
        report.compared >= 87,
        "suite shrank: {} actions",
        report.compared
    );
    assert!(
        report.unverified.is_empty(),
        "unverified inputs: {:?}",
        report.unverified
    );
    // The N-Quads manifest embeds the N-Triples cases (N-Quads is a superset), so the
    // adjudicated set is the NT set (as .nq copies where the manifest uses them) plus
    // the graph-position IRI case.
    let expected = stems(&[
        "nt-syntax-bad-uri-01.nq",
        "nt-syntax-bad-uri-04.nq",
        "nt-syntax-bad-uri-06.nq",
        "nt-syntax-bad-uri-07.nq",
        "nt-syntax-bad-uri-08.nq",
        "nt-syntax-bad-uri-09.nq",
        "nt-syntax-bad-bnode-01.nq",
        "nt-syntax-bad-bnode-02.nq",
        "nt-syntax-bad-lang-01.nq",
        "minimal_whitespace.nq",
        "nq-syntax-bad-uri-01.nq",
    ]);
    assert_eq!(
        divergent_stems(&report),
        expected,
        "adjudicated divergence set drifted.\n{}",
        report.describe()
    );
}

#[test]
fn differential_trig_sparq_vs_oxttl_over_w3c_actions() {
    let Some(root) = w3c_suite_root("rdf-trig") else {
        return;
    };
    let report = run_suite_actions(&sparq_trig(), &oxttl_trig(), &root).expect("suite walked");
    assert!(
        report.compared >= 300,
        "suite shrank: {} actions",
        report.compared
    );
    assert!(
        report.unverified.is_empty(),
        "unverified inputs: {:?}",
        report.unverified
    );
    assert!(
        report.divergences.is_empty(),
        "sparq TriG dataset path diverged from oxttl:\n{}",
        report.describe()
    );
}

// ---------------------------------------------------------------------------
// 1b. Green on incumbents — the committed fuzz seed corpus.
// ---------------------------------------------------------------------------

/// `fuzz/seeds/parse_rdf_str` + `fuzz/seeds/load_reader_parallel` seeds carry a leading
/// FORMAT-SELECTOR byte (0=ntriples, 1=nquads, 2=turtle, 3=trig — see
/// `fuzz/fuzz_targets/parse_rdf_str.rs`); route each seed to its format's parser pair.
#[test]
fn differential_fuzz_seeds_format_prefixed() {
    let pairs: [(DiffParser, DiffParser); 4] = [
        (sparq_nt(), oxttl_nt()),
        (sparq_nq(), oxttl_nq()),
        (sparq_turtle(), oxttl_turtle()),
        (sparq_trig_parallel(), oxttl_trig()),
    ];
    let mut compared = 0usize;
    for target in ["parse_rdf_str", "load_reader_parallel"] {
        let Some(dir) = fuzz_seeds_dir(target) else {
            panic!("committed fuzz seed dir fuzz/seeds/{target} missing");
        };
        let mut entries: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .collect();
        entries.sort();
        for path in entries.into_iter().filter(|p| p.is_file()) {
            let bytes = std::fs::read(&path).unwrap();
            let Some((&sel, rest)) = bytes.split_first() else {
                continue;
            };
            let Some((candidate, incumbent)) = pairs.get(sel as usize % 4).map(|(c, i)| (c, i))
            else {
                continue;
            };
            let text = String::from_utf8_lossy(rest).into_owned();
            let base = sparq_conformance::rdf::file_iri(&path);
            match compare_doc(candidate, incumbent, &text, &base) {
                Ok(None) => compared += 1,
                Ok(Some(kind)) => panic!(
                    "fuzz seed {} diverged ({} vs {}): {kind:?}",
                    path.display(),
                    candidate.name,
                    incumbent.name
                ),
                Err(e) => panic!("fuzz seed {} unverified: {e}", path.display()),
            }
        }
    }
    assert!(
        compared >= 8,
        "expected the committed seed corpus to be compared, got {compared}"
    );
}

/// The larger committed N-Quads seed set (`fuzz/seeds/canonicalize_nquads`) — plain
/// N-Quads documents, no format-selector byte. The fuzz-mangled seeds hit the SAME
/// adjudicated native-parser leniency class the W3C suites record (bead sq-w64x5: no
/// IRI character/scheme validation, so the native path ACCEPTS mangled IRIs oxttl
/// rejects) — pinned by exact count and kind; any OTHER divergence kind (quad-set
/// difference, native rejecting what oxttl accepts) fails immediately, and fixing
/// sq-w64x5 must drop the count to 0.
#[test]
fn differential_fuzz_seeds_nquads_corpus() {
    /// MEASURED adjudicated count at the committed seed corpus (all CandidateAccepts,
    /// all "Invalid IRI code point / no scheme / invalid character" — sq-w64x5).
    const NQ_SEED_ADJUDICATED_LENIENT_ACCEPTS: usize = 16;
    let Some(dir) = fuzz_seeds_dir("canonicalize_nquads") else {
        panic!("committed fuzz seed dir fuzz/seeds/canonicalize_nquads missing");
    };
    let report = run_dir(&sparq_nq(), &oxttl_nq(), &dir, &|p| p.is_file());
    assert!(
        report.compared >= 100,
        "seed corpus shrank: {}",
        report.compared
    );
    assert!(
        report.unverified.is_empty(),
        "unverified: {:?}",
        report.unverified
    );
    for d in &report.divergences {
        assert!(
            matches!(d.kind, DivergenceKind::CandidateAccepts(_)),
            "NON-adjudicated divergence kind on the seed corpus (only the sq-w64x5 \
             lenient-accept class is adjudicated):\n{}",
            report.describe()
        );
    }
    assert_eq!(
        report.divergences.len(),
        NQ_SEED_ADJUDICATED_LENIENT_ACCEPTS,
        "adjudicated sq-w64x5 lenient-accept count drifted (rose = new leniency; fell = \
         sq-w64x5 progress — re-pin and raise the syntax-ratchet floors):\n{}",
        report.describe()
    );
}

// ---------------------------------------------------------------------------
// 2. Mutation non-vacuity: seeded divergent parsers MUST be caught, with a
//    minimal repro. Hermetic — always runs.
// ---------------------------------------------------------------------------

/// A candidate that silently DROPS every quad whose object contains the poison marker —
/// the "generated parser loses data" failure mode.
fn quad_drop_mutant() -> DiffParser<'static> {
    DiffParser::new("mutant: drops poisoned quads", |text, base| {
        (oxttl_nt().parse)(text, base).map(|qs| {
            qs.into_iter()
                .filter(|q| !q[2].contains("POISON"))
                .collect()
        })
    })
}

/// A candidate that leniently ACCEPTS any document (returning the quads of the lines
/// that do parse) — the "generated parser accepts invalid input" failure mode.
fn lenient_accept_mutant() -> DiffParser<'static> {
    DiffParser::new("mutant: accepts anything", |text, base| {
        let mut quads = Vec::new();
        for line in text.lines() {
            if let Ok(mut q) = (oxttl_nt().parse)(&format!("{line}\n"), base) {
                quads.append(&mut q);
            }
        }
        Ok(quads)
    })
}

/// A candidate that MANGLES one term kind (upper-cases language tags) — the "generated
/// parser produces a different term" failure mode.
fn term_mangle_mutant() -> DiffParser<'static> {
    DiffParser::new("mutant: mangles lang tags", |text, base| {
        (oxttl_nt().parse)(text, base).map(|qs| {
            qs.into_iter()
                .map(|mut q| {
                    if let Some(at) = q[2].rfind('@') {
                        let tag = q[2][at..].to_uppercase();
                        q[2].truncate(at);
                        q[2].push_str(&tag);
                    }
                    q
                })
                .collect()
        })
    })
}

fn write_corpus(dir: &std::path::Path, files: &[(&str, &str)]) {
    std::fs::create_dir_all(dir).unwrap();
    for (name, content) in files {
        std::fs::write(dir.join(name), content).unwrap();
    }
}

#[test]
fn mutation_quad_drop_detected_with_minimal_repro() {
    let dir = std::env::temp_dir().join(format!("sq-tonhr2-diff-{}", std::process::id()));
    let mut clean = String::new();
    for i in 0..30 {
        clean.push_str(&format!("<http://ex/s{i}> <http://ex/p> \"v{i}\" .\n"));
    }
    let mut poisoned = clean.clone();
    poisoned.push_str("<http://ex/s> <http://ex/p> \"POISON\" .\n");
    write_corpus(&dir, &[("clean.nt", &clean), ("poisoned.nt", &poisoned)]);

    let report = run_dir(&quad_drop_mutant(), &oxttl_nt(), &dir, &|p| {
        p.extension().is_some_and(|e| e == "nt")
    });
    std::fs::remove_dir_all(&dir).ok();

    assert_eq!(report.compared, 2);
    assert_eq!(report.agreements, 1, "the clean file must agree");
    assert_eq!(
        report.divergences.len(),
        1,
        "the seeded mutant MUST be detected"
    );
    let d = &report.divergences[0];
    assert!(
        d.label.ends_with("poisoned.nt"),
        "wrong file blamed: {}",
        d.label
    );
    assert!(
        matches!(&d.kind, DivergenceKind::QuadSet { only_candidate, only_incumbent }
            if only_candidate.is_empty() && only_incumbent.len() == 1),
        "wrong divergence kind: {:?}",
        d.kind
    );
    // The minimal repro must shrink 31 lines to exactly the one diverging statement.
    assert_eq!(
        d.minimal_repro,
        "<http://ex/s> <http://ex/p> \"POISON\" .\n"
    );
}

#[test]
fn mutation_lenient_accept_detected() {
    // The incumbent rejects the document (bad IRI); the mutant accepts it.
    let doc = "<http://ex/s> <http://ex/p> <http://ex/o> .\n<http://ex/ bad> <http://ex/p> <http://ex/o> .\n";
    match compare_doc(&lenient_accept_mutant(), &oxttl_nt(), doc, "http://ex/") {
        Ok(Some(DivergenceKind::CandidateAccepts(_))) => {}
        other => panic!("lenient mutant not detected as CandidateAccepts: {other:?}"),
    }
    // And the shrunken repro still diverges — soundness of the shrinker on reject-diffs.
    let repro = sparq_conformance::differential::shrink_repro(
        &lenient_accept_mutant(),
        &oxttl_nt(),
        doc,
        "http://ex/",
    );
    assert!(matches!(
        compare_doc(&lenient_accept_mutant(), &oxttl_nt(), &repro, "http://ex/"),
        Ok(Some(_))
    ));
    assert_eq!(repro, "<http://ex/ bad> <http://ex/p> <http://ex/o> .\n");
}

#[test]
fn mutation_term_mangle_detected() {
    let doc = "<http://ex/s> <http://ex/p> \"chat\"@fr .\n";
    match compare_doc(&term_mangle_mutant(), &oxttl_nt(), doc, "http://ex/") {
        Ok(Some(DivergenceKind::QuadSet {
            only_candidate,
            only_incumbent,
        })) => {
            assert_eq!(only_candidate.len(), 1);
            assert_eq!(only_incumbent.len(), 1);
            assert!(only_candidate[0][2].ends_with("@FR"));
        }
        other => panic!("term-mangle mutant not detected: {other:?}"),
    }
}

/// The suite-driven mode must ALSO catch a seeded mutant (not just `run_dir`): run the
/// quad-drop mutant against oxttl over the real rdf-n-triples actions — every action
/// whose triples survive on the incumbent but contain a literal the mutant drops
/// diverges. Self-skips without the fetched data.
#[test]
fn mutation_detected_through_suite_mode() {
    let Some(root) = w3c_suite_root("rdf-n-triples") else {
        return;
    };
    // Mutant drops quads whose object contains "x" — several suite actions contain one.
    let mutant = DiffParser::new("mutant: drops objects containing x", |text, base| {
        (oxttl_nt().parse)(text, base)
            .map(|qs| qs.into_iter().filter(|q| !q[2].contains('x')).collect())
    });
    let baseline = run_suite_actions(&oxttl_nt(), &oxttl_nt(), &root).expect("suite walked");
    assert!(
        baseline.divergences.is_empty(),
        "oxttl-vs-oxttl must be divergence-free"
    );
    let mutated = run_suite_actions(&mutant, &oxttl_nt(), &root).expect("suite walked");
    assert!(
        !mutated.divergences.is_empty(),
        "the suite-driven differential failed to detect a seeded quad-dropping mutant"
    );
}
