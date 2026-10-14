//! sq-hmd7l.8 (epic sq-hmd7l) — real-ontology OWL 2 EL classification bench.
//!
//! The crate's other bench (`snomed_go_scale_bench`) is a SYNTHETIC, closed-form scaling
//! probe. This example instead classifies an ONTOLOGY FILE end-to-end (parse → extract →
//! CR1–CR5 saturate → project) and reports the ORACLE METRIC used to cross-check sparq
//! against ELK on real ontologies: the number of PROPER NAMED SUBSUMPTION PAIRS
//! (`C ⊑ D`, both named IRIs, `C ≠ D`, `D ≠ owl:Thing`) in the complete classification.
//!
//! TWO MODES
//! ---------
//!   * SMOKE (no path arg, or the sole arg `--smoke`): classify the small VENDORED EL
//!     fixture `examples/data/el_smoke.ttl` (compiled in via `include_str!`, so the run is
//!     hermetic and cwd-independent) and ASSERT its pinned oracle counts. This is the
//!     acceptance gate: `cargo run -p sparq-reason-el --release --example
//!     reason_el_real_bench` exits 0 iff classification still derives exactly the pinned
//!     lattice. The pinned count includes a CR4 existential subsumption OWL 2 RL cannot
//!     reach, so a classifier regression flips it red (non-vacuous).
//!
//!   * GATHER (`<path> [format]`): classify an arbitrary ontology converted to RDF
//!     (`format` defaults to `turtle`; pass `ntriples` for a riot-converted GO/OpenGALEN
//!     dump) and PRINT `classes=<n> subsumptions=<n> emitted=<n> skipped_axioms=<n>
//!     classify_s=<t>` — NO assertion (real-ontology counts are recorded + cross-checked
//!     vs ELK by scripts/bench/reason-el-same-box.sh, never baked in here). The
//!     subsumption count is emitted BEFORE any timing claim so the harness can enforce the
//!     "no timing row without a subsumption-count agreement" invariant.
//!
//! The fixture uses only is-a + role-EQUALITY existentials (no property chains), so its
//! complete closure is identical with or without the `rbox` feature; the same example
//! built `--features rbox` (as the GO/OpenGALEN gather does, for part-of role chains)
//! gets role reasoning for free via `Classifier::classify` and the pinned smoke count is
//! unchanged. Work-box wall-clock is non-canonical — printed as a trend only.

use sparq_core::dict::{Id, TermParts};
use sparq_core::Graph;
use sparq_reason_el::classify_graph;
use std::time::Instant;

const RDFS_SUBCLASSOF: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";
const OWL_THING: &str = "http://www.w3.org/2002/07/owl#Thing";

/// The vendored EL smoke ontology, compiled in for a hermetic, cwd-independent smoke run.
const SMOKE_TTL: &str = include_str!("data/el_smoke.ttl");

// --- pinned smoke oracle (hand-verified; see examples/data/el_smoke.ttl) -----------------
// Named classes participating in the subsumption lattice (an endpoint of some proper named
// C ⊑ D): AnatomicalEntity, Cell, Neuron, CellComponent, Nucleus, NucleatedCell = 6.
const SMOKE_LATTICE_CLASSES: usize = 6;
// Proper named subsumption pairs in the complete closure (C ⊑ D, both named, C≠D, D≠Thing):
//   Cell⊑AnatomicalEntity, Neuron⊑Cell, CellComponent⊑AnatomicalEntity,
//   Nucleus⊑CellComponent, NucleatedCell⊑Cell                            (5 told)
//   Nucleus⊑AnatomicalEntity, NucleatedCell⊑AnatomicalEntity,
//   Neuron⊑AnatomicalEntity, Neuron⊑NucleatedCell                        (4 derived; the
//   last is the CR4 existential edge OWL 2 RL cannot reach)              → 9 total.
const SMOKE_SUBSUMPTIONS: usize = 9;

/// Classification metrics for one ontology — the ORACLE the ELK cross-check compares on.
struct Metrics {
    /// Distinct named classes that are an ENDPOINT of some proper named subsumption pair
    /// (self-consistent with `subsumptions`; excludes anonymous restriction classes and
    /// ⊤/⊥, unlike the extractor's raw `Report::named_classes`).
    lattice_classes: usize,
    /// PROPER NAMED subsumption pairs (`C ⊑ D`, both named IRIs, `C ≠ D`, `D ≠ owl:Thing`)
    /// in the COMPLETE classification (told + derived). The engine-vs-ELK oracle metric.
    subsumptions: usize,
    /// NEW `rdfs:subClassOf` triples `classify_graph` materialised (`Report::emitted_subsumptions`).
    emitted: usize,
    /// Class-axioms outside the recognised EL fragment that were IGNORED (`Report::skipped_axioms`).
    /// A non-zero value is the honest "the lattice may be incomplete for those axioms" signal.
    skipped_axioms: usize,
    /// Wall-clock of `classify_graph` only (parse excluded). Non-canonical on a shared box.
    classify_s: f64,
}

/// Parses `text` as `format`, runs the full classifier, and reads off the oracle metrics.
fn classify_text(text: &str, format: &str) -> Result<Metrics, String> {
    let (mut dict, mut triples) = Graph::parse_to_triples(text, format)?;
    let sco = dict.lookup(&oxrdf::Term::NamedNode(
        oxrdf::NamedNode::new_unchecked(RDFS_SUBCLASSOF),
    ));
    let thing = dict.lookup(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(OWL_THING)));

    let t = Instant::now();
    let report = classify_graph(&mut dict, &mut triples);
    let classify_s = t.elapsed().as_secs_f64();

    // Count PROPER NAMED subsumption pairs from the materialised closure. classify_graph has
    // appended every derived named edge, so a scan of the subClassOf triples with named-IRI
    // endpoints (dropping self-loops, owl:Thing supers, and edges to anonymous restriction
    // bnodes) is exactly the engine-vs-ELK oracle metric. A set dedupes told∩derived overlap.
    let named = |id: Id| matches!(dict.term_parts(id), TermParts::Iri { .. });
    let mut pairs: rustc_hash::FxHashSet<(Id, Id)> = rustc_hash::FxHashSet::default();
    let mut classes: rustc_hash::FxHashSet<Id> = rustc_hash::FxHashSet::default();
    for &[s, p, o] in &triples {
        if p == sco && s != o && o != thing && named(s) && named(o) {
            pairs.insert((s, o));
            classes.insert(s);
            classes.insert(o);
        }
    }

    Ok(Metrics {
        lattice_classes: classes.len(),
        subsumptions: pairs.len(),
        emitted: report.emitted_subsumptions,
        skipped_axioms: report.skipped_axioms,
        classify_s,
    })
}

fn run_smoke() {
    let m = classify_text(SMOKE_TTL, "turtle").expect("smoke fixture parses");
    println!(
        "smoke (examples/data/el_smoke.ttl): lattice_classes={} subsumptions={} emitted={} \
         skipped_axioms={} classify_s={:.6}",
        m.lattice_classes, m.subsumptions, m.emitted, m.skipped_axioms, m.classify_s
    );
    assert_eq!(
        m.lattice_classes, SMOKE_LATTICE_CLASSES,
        "lattice class count {} != pinned {SMOKE_LATTICE_CLASSES} (classification changed)",
        m.lattice_classes
    );
    assert_eq!(
        m.subsumptions, SMOKE_SUBSUMPTIONS,
        "proper named subsumption count {} != pinned {SMOKE_SUBSUMPTIONS} — a classification \
         regression (e.g. the CR4 existential edge Neuron ⊑ NucleatedCell no longer derives)",
        m.subsumptions
    );
    assert_eq!(
        m.skipped_axioms, 0,
        "smoke fixture is pure EL — {} axioms unexpectedly skipped",
        m.skipped_axioms
    );
    println!(
        "OK: pinned oracle held — {SMOKE_LATTICE_CLASSES} lattice classes, {SMOKE_SUBSUMPTIONS} \
         proper named subsumptions (incl. the RL-unreachable CR4 edge). Wall-clock is \
         trend-only (non-canonical shared box)."
    );
}

fn run_gather(path: &str, format: &str) {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read ontology {path}: {e}"));
    let m = classify_text(&text, format)
        .unwrap_or_else(|e| panic!("cannot classify {path} as {format}: {e}"));
    // Subsumption count FIRST (the cross-check oracle), timing after — the harness records the
    // count per ontology before it trusts any wall-clock (the sq-hmd7l.8 invariant).
    println!(
        "ontology={path} lattice_classes={} subsumptions={} emitted={} skipped_axioms={} classify_s={:.6}",
        m.lattice_classes, m.subsumptions, m.emitted, m.skipped_axioms, m.classify_s
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("--smoke") => run_smoke(),
        Some(path) => {
            let format = args.get(1).map(String::as_str).unwrap_or("turtle");
            run_gather(path, format);
        }
    }
}
