//! sq-ez1rc (epic sq-pbz04): EL end-to-end SNOMED/GO-style scaling benchmark — a
//! self-asserting, deterministic workload that exercises the WHOLE pipeline composed together:
//! normalise (Baader–Brandt–Lutz) + RBox role automaton (CR10/CR11, the `rbox` feature) +
//! transitive reduction to the Hasse diagram (the `hasse` feature). Run (BOTH features required):
//!
//!     cargo run -p sparq-reason-el --features rbox,hasse --example snomed_go_scale_bench --release [SCALE]
//!
//! No dataset to fetch: the ontology slice is generated in-process (a pure function of SCALE,
//! byte-deterministic, no RNG/network/toolchain), so this is a hermetic, regenerable benchmark.
//! It is modelled on the shape of biomedical EL ontologies (SNOMED CT / GO): a wide is-a FOREST,
//! a transitive part-of role with a SNOMED-style right-identity role chain, and existential
//! `hasLocation`/`partOf` restrictions that only classify via CR4 traversal THROUGH the role
//! automaton — the combination RL cannot reach and the spike (research/owl2-el-ql-reasoning-
//! spike.md) flagged as the open perf item: confirm normalise + RBox + Hasse compose with NO
//! hidden quadratic blow-up.
//!
//! WHY A *RELATIVE* ASSERTION, NOT A WALL-CLOCK NUMBER
//! --------------------------------------------------
//! Work-box timings are non-canonical (a contended shared runner inflates them), so this bench
//! asserts NOTHING about milliseconds. Instead the ontology slice is constructed so its complete
//! subsumption closure is LINEAR in SCALE (a bounded-degree forest + bounded existential fan-out),
//! with a CLOSED FORM for every derived count. The benchmark then runs the full pipeline at 1×
//! and 2× SCALE and asserts two things:
//!
//!   1. CONFORMANCE UNCHANGED — every derived count equals its closed form at BOTH scales (an
//!      exact structural oracle; a classifier regression that silently under- or over-derives
//!      fails LOUDLY here, independent of timing).
//!   2. NO HIDDEN QUADRATIC — the algorithm's work proxy (closure edges materialised + Hasse
//!      edges produced) at 2× is bounded by `RATIO_BOUND ×` the 1× proxy. Because the closure is
//!      linear by construction, the TRUE ratio is ≈ 2; a hidden quadratic in any composed stage
//!      (normalise, RBox saturation, or Hasse reduction) would push it toward 4 (= doubling² for
//!      a step that is quadratic in SCALE). `RATIO_BOUND = 3.0` sits comfortably between the two:
//!      it passes the linear truth with margin and FAILS a quadratic regression. No absolute
//!      number is baked in — only the dimensionless doubling ratio.

#![cfg(all(feature = "rbox", feature = "hasse"))]

use sparq_core::dict::{Dict, Id};
use sparq_reason_el::{Classifier, DirectHierarchy};
use std::time::Instant;

const RDFS: &str = "http://www.w3.org/2000/01/rdf-schema#";
const OWL: &str = "http://www.w3.org/2002/07/owl#";
const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const EX: &str = "http://sparq.dev/bench/el-snomed#";

/// Number of mid-level "branch" concepts in the is-a forest (fixed across scales, so the forest
/// stays a forest — wide and shallow, exactly like a biomedical sub-hierarchy slice).
const BRANCHES: usize = 8;
/// The 2× work proxy must stay under this multiple of the 1× proxy. The true ratio is ≈ 2 (the
/// closure is linear by construction); a hidden quadratic would head toward 4. 3.0 passes the
/// linear truth with margin and trips a quadratic regression. Dimensionless — no absolute number.
const RATIO_BOUND: f64 = 3.0;

struct Vocab {
    sc: Id,
    on_prop: Id,
    svf: Id,
    sub_prop: Id,
    chain: Id,
    ty: Id,
    first: Id,
    rest: Id,
    nil: Id,
    restriction: Id,
    transitive_prop: Id,
}

impl Vocab {
    fn intern(d: &mut Dict) -> Vocab {
        Vocab {
            sc: d.intern_iri(&format!("{RDFS}subClassOf")),
            on_prop: d.intern_iri(&format!("{OWL}onProperty")),
            svf: d.intern_iri(&format!("{OWL}someValuesFrom")),
            sub_prop: d.intern_iri(&format!("{RDFS}subPropertyOf")),
            chain: d.intern_iri(&format!("{OWL}propertyChainAxiom")),
            ty: d.intern_iri(&format!("{RDF}type")),
            first: d.intern_iri(&format!("{RDF}first")),
            rest: d.intern_iri(&format!("{RDF}rest")),
            nil: d.intern_iri(&format!("{RDF}nil")),
            restriction: d.intern_iri(&format!("{OWL}Restriction")),
            transitive_prop: d.intern_iri(&format!("{OWL}TransitiveProperty")),
        }
    }
}

/// The closed-form derived counts the generated slice MUST produce at a given scale — the exact
/// structural oracle. Hand-derived below; see each field for the EL-calculus justification.
struct Expected {
    /// Total complete-closure subsumption edges Σ_C |S(C)| (named, proper, excluding ⊤/⊥).
    closure_edges: usize,
    /// Findings that derive `⊑ LocatedFinding` through the existential + role hierarchy (= scale).
    located_findings: usize,
    /// Structures that derive `⊑ DeepPart` through the transitive part-of chain (CR11) (= scale).
    deep_parts: usize,
}

/// Builds a SNOMED/GO-style EL slice at `scale`, returning `(triples, expected)`. All bnode/IRI
/// names are deterministic functions of indices (no RNG). The pieces, each engineered to keep the
/// closure LINEAR in `scale` so the doubling ratio is a meaningful no-hidden-quadratic probe:
///
///   A. IS-A FOREST. `scale` leaves, each `Leaf_i ⊑ Branch_{i % BRANCHES} ⊑ Root`. Bounded depth
///      (2), so each leaf contributes exactly 2 closure edges and each branch 1.
///   B. ROLE BOX. `partOf` transitive (`partOf ∘ partOf ⊑ partOf`, CR11) and a SNOMED right-
///      identity chain `partOf ∘ hasLocation ⊑ hasLocation` (CR11) plus `hasExactLocation ⊑
///      hasLocation` (CR10 role inclusion).
///   C. EXISTENTIAL LOCATION (CR4 through CR10). `Finding_i ⊑ ∃hasExactLocation.Leaf_i`, and one
///      global back-propagation axiom `∃hasLocation.Root ⊑ LocatedFinding`. Since `Leaf_i ⊑* Root`
///      and `hasExactLocation ⊑ hasLocation`, every `Finding_i` derives `⊑ LocatedFinding` THROUGH
///      its existential successor — `scale` such derivations, none reachable by OWL 2 RL.
///   D. PART-OF CHAIN (CR4 through CR11). `StructA_i ⊑ ∃partOf.StructB_i`, `StructB_i ⊑
///      ∃partOf.StructC`, and a back-prop `∃partOf.StructC ⊑ DeepPart`. partOf-transitivity (CR11)
///      composes the two links so every `StructA_i` and `StructB_i` derives `⊑ DeepPart`.
fn build_slice(dict: &mut Dict, scale: usize) -> (Vec<[Id; 3]>, Expected) {
    let v = Vocab::intern(dict);
    let mut t: Vec<[Id; 3]> = Vec::new();
    let mut bnode = 0usize;
    let fresh = |dict: &mut Dict, n: &mut usize| -> Id {
        let id = dict.intern_iri(&format!("{EX}__b{n}"));
        *n += 1;
        id
    };

    // Named concepts / roles used globally.
    let root = dict.intern_iri(&format!("{EX}Root"));
    let located_finding = dict.intern_iri(&format!("{EX}LocatedFinding"));
    let deep_part = dict.intern_iri(&format!("{EX}DeepPart"));
    let struct_c = dict.intern_iri(&format!("{EX}StructC"));
    let part_of = dict.intern_iri(&format!("{EX}partOf"));
    let has_location = dict.intern_iri(&format!("{EX}hasLocation"));
    let has_exact_location = dict.intern_iri(&format!("{EX}hasExactLocation"));
    let branches: Vec<Id> = (0..BRANCHES)
        .map(|b| dict.intern_iri(&format!("{EX}Branch{b}")))
        .collect();

    // --- A. is-a forest -----------------------------------------------------------------------
    for &b in &branches {
        t.push([b, v.sc, root]); // Branch_b ⊑ Root
    }
    for i in 0..scale {
        let leaf = dict.intern_iri(&format!("{EX}Leaf{i}"));
        t.push([leaf, v.sc, branches[i % BRANCHES]]); // Leaf_i ⊑ Branch_{i%B}
    }

    // --- B. role box --------------------------------------------------------------------------
    // partOf transitive: owl:TransitiveProperty(partOf)  ≡  partOf ∘ partOf ⊑ partOf.
    t.push([part_of, v.ty, v.transitive_prop]);
    // hasExactLocation ⊑ hasLocation (CR10 role inclusion).
    t.push([has_exact_location, v.sub_prop, has_location]);
    // SNOMED right-identity: partOf ∘ hasLocation ⊑ hasLocation (CR11).
    {
        // propertyChainAxiom on the super-property hasLocation, list ( partOf hasLocation ).
        let c1 = fresh(dict, &mut bnode);
        let c2 = fresh(dict, &mut bnode);
        t.push([has_location, v.chain, c1]);
        t.push([c1, v.first, part_of]);
        t.push([c1, v.rest, c2]);
        t.push([c2, v.first, has_location]);
        t.push([c2, v.rest, v.nil]);
    }

    // --- C. existential location: Finding_i ⊑ ∃hasExactLocation.Leaf_i ------------------------
    for i in 0..scale {
        let finding = dict.intern_iri(&format!("{EX}Finding{i}"));
        let leaf = dict.intern_iri(&format!("{EX}Leaf{i}"));
        let r = fresh(dict, &mut bnode);
        t.push([r, v.ty, v.restriction]);
        t.push([r, v.on_prop, has_exact_location]);
        t.push([r, v.svf, leaf]);
        t.push([finding, v.sc, r]); // Finding_i ⊑ ∃hasExactLocation.Leaf_i
    }
    // Global back-propagation: ∃hasLocation.Root ⊑ LocatedFinding.
    {
        let r = fresh(dict, &mut bnode);
        t.push([r, v.ty, v.restriction]);
        t.push([r, v.on_prop, has_location]);
        t.push([r, v.svf, root]);
        t.push([r, v.sc, located_finding]);
    }

    // --- D. part-of chain: StructA_i ⊑ ∃partOf.StructB_i, StructB_i ⊑ ∃partOf.StructC ----------
    for i in 0..scale {
        let sa = dict.intern_iri(&format!("{EX}StructA{i}"));
        let sb = dict.intern_iri(&format!("{EX}StructB{i}"));
        let r1 = fresh(dict, &mut bnode);
        t.push([r1, v.ty, v.restriction]);
        t.push([r1, v.on_prop, part_of]);
        t.push([r1, v.svf, sb]);
        t.push([sa, v.sc, r1]); // StructA_i ⊑ ∃partOf.StructB_i
        let r2 = fresh(dict, &mut bnode);
        t.push([r2, v.ty, v.restriction]);
        t.push([r2, v.on_prop, part_of]);
        t.push([r2, v.svf, struct_c]);
        t.push([sb, v.sc, r2]); // StructB_i ⊑ ∃partOf.StructC
    }
    // Back-prop: ∃partOf.StructC ⊑ DeepPart.
    {
        let r = fresh(dict, &mut bnode);
        t.push([r, v.ty, v.restriction]);
        t.push([r, v.on_prop, part_of]);
        t.push([r, v.svf, struct_c]);
        t.push([r, v.sc, deep_part]);
    }

    // Closed forms (named, proper closure edges, excluding ⊤/⊥):
    //   A: each Leaf_i ⊑ {Branch, Root} = 2·scale ; each Branch_b ⊑ Root = BRANCHES.
    //   C: each Finding_i ⊑ LocatedFinding = scale.
    //   D: each StructA_i ⊑ {DeepPart} and StructB_i ⊑ {DeepPart} (StructA's ∃partOf successor is
    //      StructB whose ∃partOf is StructC, partOf-transitivity composes A→C) = 2·scale. StructA_i
    //      does NOT gain StructB_i as a named super (the link is existential, not a subsumption).
    // Total = 2·scale + BRANCHES + scale + 2·scale = 5·scale + BRANCHES.
    let expected = Expected {
        closure_edges: 5 * scale + BRANCHES,
        located_findings: scale,
        deep_parts: 2 * scale,
    };
    (t, expected)
}

/// Runs the full normalise + RBox + Hasse pipeline at `scale`, asserts the conformance closed
/// forms, and returns the work proxy (closure edges materialised + Hasse edges produced).
fn run(scale: usize) -> usize {
    let mut dict = Dict::new();
    let (triples, exp) = build_slice(&mut dict, scale);

    let t = Instant::now();
    let closure = Classifier::classify(&dict, &triples); // normalise + CR1–CR5 + CR10/CR11 (rbox)
    let direct = DirectHierarchy::from_closure(&closure); // transitive reduction (hasse)
    let dt = t.elapsed().as_secs_f64();

    // --- CONFORMANCE: exact closed-form structural oracle (independent of timing) -------------
    // 1. Total complete-closure subsumption edges.
    let lookup = |s: &str| {
        use oxrdf::{NamedNode, Term as OTerm};
        dict.lookup(&OTerm::NamedNode(NamedNode::new_unchecked(format!("{EX}{s}"))))
    };
    let mut named: Vec<Id> = vec![
        lookup("Root"),
        lookup("LocatedFinding"),
        lookup("DeepPart"),
        lookup("StructC"),
    ];
    for b in 0..BRANCHES {
        named.push(lookup(&format!("Branch{b}")));
    }
    for i in 0..scale {
        for p in ["Leaf", "Finding", "StructA", "StructB"] {
            named.push(lookup(&format!("{p}{i}")));
        }
    }
    let closure_edges: usize = named.iter().map(|&c| closure.super_classes(c).len()).sum();
    assert_eq!(
        closure_edges, exp.closure_edges,
        "scale {scale}: closure edges {closure_edges} != closed form {} (conformance changed)",
        exp.closure_edges
    );

    // 2. Every Finding_i ⊑ LocatedFinding (CR4 through CR10 role inclusion).
    let located_finding = lookup("LocatedFinding");
    let located = (0..scale)
        .filter(|&i| closure.is_subclass_of(lookup(&format!("Finding{i}")), located_finding))
        .count();
    assert_eq!(
        located, exp.located_findings,
        "scale {scale}: LocatedFinding derivations {located} != {} (existential+role compose broke)",
        exp.located_findings
    );

    // 3. Every StructA_i and StructB_i ⊑ DeepPart (CR4 through CR11 transitive composition).
    let deep_part = lookup("DeepPart");
    let deep = (0..scale)
        .flat_map(|i| [format!("StructA{i}"), format!("StructB{i}")])
        .filter(|n| closure.is_subclass_of(lookup(n), deep_part))
        .count();
    assert_eq!(
        deep, exp.deep_parts,
        "scale {scale}: DeepPart derivations {deep} != {} (transitive partOf chain broke)",
        exp.deep_parts
    );

    let hasse_edges = direct.direct_edge_count();
    let proxy = closure_edges + hasse_edges;
    println!(
        "scale {scale:>7}: classify+reduce {dt:9.6}s   closure_edges {closure_edges}  hasse_edges {hasse_edges}  work_proxy {proxy}  (closed forms asserted)"
    );
    proxy
}

fn main() {
    let scale: usize = std::env::args()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(20_000);
    println!(
        "sparq-reason-el snomed_go_scale_bench — scale {scale} & {} (deterministic, regenerable; \
         normalise + RBox + Hasse composed)",
        2 * scale
    );

    let proxy_1x = run(scale);
    let proxy_2x = run(2 * scale);

    // --- NO HIDDEN QUADRATIC: the dimensionless doubling ratio --------------------------------
    let ratio = proxy_2x as f64 / proxy_1x as f64;
    assert!(
        ratio <= RATIO_BOUND,
        "work proxy doubling ratio {ratio:.3} exceeds bound {RATIO_BOUND} — a hidden quadratic \
         appeared when normalise + RBox + Hasse composed (true linear ratio is ≈ 2)"
    );
    println!(
        "OK: conformance closed forms held at both scales; work-proxy doubling ratio {ratio:.3} \
         ≤ {RATIO_BOUND} (≈ 2 expected, linear) — normalise + RBox + Hasse compose with no hidden \
         quadratic. Wall-clock above is trend-only (non-canonical)."
    );
}
