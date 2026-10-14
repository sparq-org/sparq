// sq-ez1rc (epic sq-pbz04): EL end-to-end SNOMED/GO-style scaling TEST — the executed,
// CI-gated counterpart of `examples/snomed_go_scale_bench.rs`. It generates a biomedical-shaped EL
// slice (wide is-a forest + transitive part-of with a SNOMED right-identity role chain + existential
// hasLocation/partOf restrictions), runs the WHOLE pipeline composed — normalise + RBox (CR10/CR11)
// + Hasse transitive reduction — at 1x and 2x scale, and asserts:
//
//   1. CONFORMANCE UNCHANGED — every derived count equals its hand-derived closed form at BOTH
//      scales (an exact structural oracle; a silent under/over-derivation fails here).
//   2. NO HIDDEN QUADRATIC — the algorithm's work proxy (closure edges + Hasse edges) at 2x is
//      bounded by RATIO_BOUND x the 1x proxy. The slice's closure is LINEAR by construction so the
//      true ratio is ~2; a hidden quadratic in any composed stage heads toward 4. The assertion is
//      RELATIVE/dimensionless — no wall-clock, no absolute number baked in (work-box timings are
//      non-canonical).
//
// `#![cfg(all(feature = "rbox", feature = "hasse"))]` — runs ONLY with BOTH features on, so the two
// existing feature-matrix legs (`sparq-reason-el (rbox …)` and `(hasse …)`) and a developer running
// `cargo test --features rbox,hasse` execute it; the default `--workspace --all-targets` archive
// (which passes no `--features`) compiles it empty. No new gating leg, no golden change.

#![cfg(all(feature = "rbox", feature = "hasse"))]

use oxrdf::{NamedNode, Term as OTerm};
use sparq_core::dict::{Dict, Id};
use sparq_reason_el::{Classifier, DirectHierarchy};

const RDFS: &str = "http://www.w3.org/2000/01/rdf-schema#";
const OWL: &str = "http://www.w3.org/2002/07/owl#";
const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const EX: &str = "http://sparq.dev/bench/el-snomed#";

const BRANCHES: usize = 8;
// The 2x work proxy must stay under this multiple of the 1x proxy. True linear ratio is ~2; a
// hidden quadratic heads toward 4. 3.0 passes linear with margin and trips a quadratic regression.
const RATIO_BOUND: f64 = 3.0;

fn iri(d: &mut Dict, s: &str) -> Id {
    d.intern_iri(&format!("{EX}{s}"))
}

/// Builds the SNOMED/GO-style EL slice at `scale` (deterministic). See the example file's `build_slice`
/// for the full annotated construction; this mirrors it. Returns `(triples, closure_edges_closed_form)`.
fn build_slice(d: &mut Dict, scale: usize) -> (Vec<[Id; 3]>, usize) {
    let sc = d.intern_iri(&format!("{RDFS}subClassOf"));
    let on_prop = d.intern_iri(&format!("{OWL}onProperty"));
    let svf = d.intern_iri(&format!("{OWL}someValuesFrom"));
    let sub_prop = d.intern_iri(&format!("{RDFS}subPropertyOf"));
    let chain = d.intern_iri(&format!("{OWL}propertyChainAxiom"));
    let ty = d.intern_iri(&format!("{RDF}type"));
    let first = d.intern_iri(&format!("{RDF}first"));
    let rest = d.intern_iri(&format!("{RDF}rest"));
    let nil = d.intern_iri(&format!("{RDF}nil"));
    let restriction = d.intern_iri(&format!("{OWL}Restriction"));
    let transitive_prop = d.intern_iri(&format!("{OWL}TransitiveProperty"));

    let root = iri(d, "Root");
    let located_finding = iri(d, "LocatedFinding");
    let deep_part = iri(d, "DeepPart");
    let struct_c = iri(d, "StructC");
    let part_of = iri(d, "partOf");
    let has_location = iri(d, "hasLocation");
    let has_exact_location = iri(d, "hasExactLocation");
    let branches: Vec<Id> = (0..BRANCHES).map(|b| iri(d, &format!("Branch{b}"))).collect();

    let mut t: Vec<[Id; 3]> = Vec::new();
    let mut bnode = 0usize;
    let mut fresh = |d: &mut Dict| -> Id {
        let id = iri(d, &format!("__b{bnode}"));
        bnode += 1;
        id
    };

    // A. is-a forest.
    for &b in &branches {
        t.push([b, sc, root]);
    }
    for i in 0..scale {
        let leaf = iri(d, &format!("Leaf{i}"));
        t.push([leaf, sc, branches[i % BRANCHES]]);
    }

    // B. role box: partOf transitive, hasExactLocation ⊑ hasLocation, partOf ∘ hasLocation ⊑ hasLocation.
    t.push([part_of, ty, transitive_prop]);
    t.push([has_exact_location, sub_prop, has_location]);
    {
        let c1 = fresh(d);
        let c2 = fresh(d);
        t.push([has_location, chain, c1]);
        t.push([c1, first, part_of]);
        t.push([c1, rest, c2]);
        t.push([c2, first, has_location]);
        t.push([c2, rest, nil]);
    }

    // C. existential location.
    for i in 0..scale {
        let finding = iri(d, &format!("Finding{i}"));
        let leaf = iri(d, &format!("Leaf{i}"));
        let r = fresh(d);
        t.push([r, ty, restriction]);
        t.push([r, on_prop, has_exact_location]);
        t.push([r, svf, leaf]);
        t.push([finding, sc, r]);
    }
    {
        let r = fresh(d);
        t.push([r, ty, restriction]);
        t.push([r, on_prop, has_location]);
        t.push([r, svf, root]);
        t.push([r, sc, located_finding]);
    }

    // D. part-of chain.
    for i in 0..scale {
        let sa = iri(d, &format!("StructA{i}"));
        let sb = iri(d, &format!("StructB{i}"));
        let r1 = fresh(d);
        t.push([r1, ty, restriction]);
        t.push([r1, on_prop, part_of]);
        t.push([r1, svf, sb]);
        t.push([sa, sc, r1]);
        let r2 = fresh(d);
        t.push([r2, ty, restriction]);
        t.push([r2, on_prop, part_of]);
        t.push([r2, svf, struct_c]);
        t.push([sb, sc, r2]);
    }
    {
        let r = fresh(d);
        t.push([r, ty, restriction]);
        t.push([r, on_prop, part_of]);
        t.push([r, svf, struct_c]);
        t.push([r, sc, deep_part]);
    }

    // Closed form: 2·scale (leaves) + BRANCHES (branches⊑root) + scale (findings⊑located)
    //              + 2·scale (StructA/B ⊑ DeepPart) = 5·scale + BRANCHES.
    (t, 5 * scale + BRANCHES)
}

/// Runs the composed pipeline at `scale`, asserts every closed-form count, returns the work proxy.
fn run_and_assert(scale: usize) -> usize {
    let mut dict = Dict::new();
    let (triples, expected_closure) = build_slice(&mut dict, scale);

    let closure = Classifier::classify(&dict, &triples);
    let direct = DirectHierarchy::from_closure(&closure);

    let look = |s: &str| dict.lookup(&OTerm::NamedNode(NamedNode::new_unchecked(format!("{EX}{s}"))));

    // 1. total closure edges over the named concept set.
    let mut named: Vec<Id> = vec![look("Root"), look("LocatedFinding"), look("DeepPart"), look("StructC")];
    for b in 0..BRANCHES {
        named.push(look(&format!("Branch{b}")));
    }
    for i in 0..scale {
        for p in ["Leaf", "Finding", "StructA", "StructB"] {
            named.push(look(&format!("{p}{i}")));
        }
    }
    let closure_edges: usize = named.iter().map(|&c| closure.super_classes(c).len()).sum();
    assert_eq!(
        closure_edges, expected_closure,
        "scale {scale}: closure edges != closed form (conformance changed)"
    );

    // 2. every Finding_i ⊑ LocatedFinding (CR4 through CR10).
    let located_finding = look("LocatedFinding");
    let located = (0..scale)
        .filter(|&i| closure.is_subclass_of(look(&format!("Finding{i}")), located_finding))
        .count();
    assert_eq!(located, scale, "scale {scale}: not every Finding derived LocatedFinding");

    // 3. every StructA_i / StructB_i ⊑ DeepPart (CR4 through CR11 transitive composition).
    let deep_part = look("DeepPart");
    let deep = (0..scale)
        .flat_map(|i| [format!("StructA{i}"), format!("StructB{i}")])
        .filter(|n| closure.is_subclass_of(look(n), deep_part))
        .count();
    assert_eq!(deep, 2 * scale, "scale {scale}: not every Struct derived DeepPart");

    closure_edges + direct.direct_edge_count()
}

#[test]
fn conformance_closed_forms_hold_at_both_scales() {
    // A modest scale keeps the test fast; the closed forms are scale-independent invariants.
    let scale = 500;
    let _ = run_and_assert(scale);
    let _ = run_and_assert(2 * scale);
}

#[test]
fn composed_pipeline_has_no_hidden_quadratic() {
    // Relative/dimensionless: doubling the input must not more-than-RATIO_BOUND the work proxy.
    let scale = 500;
    let proxy_1x = run_and_assert(scale);
    let proxy_2x = run_and_assert(2 * scale);
    let ratio = proxy_2x as f64 / proxy_1x as f64;
    assert!(
        ratio <= RATIO_BOUND,
        "work-proxy doubling ratio {ratio:.3} > {RATIO_BOUND}: a hidden quadratic appeared when \
         normalise + RBox + Hasse composed (true linear ratio is ~2)"
    );
}
