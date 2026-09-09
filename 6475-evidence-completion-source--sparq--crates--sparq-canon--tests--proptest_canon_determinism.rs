// [FABLE-5] sq-3dyje.8 — Property-based tests for sparq-canon: RDFC-1.0 determinism
// under blank-node relabeling + quad permutation, idempotence, and API-path agreement.
//
// SCOPE: test-only (no production-code change). Uses proptest as a [dev-dependency]
// only; it does NOT appear in sparq-canon's [dependencies] or [features] and does not
// affect the shipped crate, any feature-OFF build, or the wasm artifact.
//
// The W3C rdf-canon suite (tests/rdf_canon_suite.rs) stays the conformance anchor;
// this file is the randomized SECOND mechanism over generated small datasets.
//
// ACCEPTANCE:
//   cargo test -p sparq-canon --test proptest_canon_determinism   (default features)
//
// ---- FOUR PROPERTY FAMILIES ------------------------------------------------
//
// (a) BLANK-NODE-RELABELING INVARIANCE
//     canonicalize_quads(d) is byte-identical when every blank-node label in d is
//     replaced through a random injective renaming (a permutation of the label pool
//     under a fresh prefix). This is the core RDFC-1.0 promise: the canonical form
//     is a function of the graph ISOMORPHISM class, not of the input labels.
//
// (b) QUAD-PERMUTATION INVARIANCE
//     canonicalize_quads(d) is byte-identical under a random permutation of the
//     input quad order (the order quads are fed through the bridge and into the
//     first-degree/N-degree hashing must not leak into the output).
//
// (c) IDEMPOTENCE
//     Canonical output is a fixpoint: parse the canonical N-Quads back and
//     canonicalize again — byte-identical.
//
// (d) API-PATH AGREEMENT (issue_quads_with <-> canonicalize_quads_with)
//     Relabeling the INPUT quads through the issued-identifier map returned by
//     issue_quads_with::<Sha256>, serializing each quad in N-Quads form, deduping,
//     and sorting in code point order reconstructs canonicalize_quads_with::<Sha256>
//     byte-for-byte. Also pins the alias/default paths to each other:
//     canonicalize == canonicalize_quads == canonicalize_quads_with::<Sha256>
//     (SHA-256 is the spec default), issue_quads == issue_quads_with::<Sha256>,
//     and the text path canonicalize_nquads(serialized d) agrees.
//
// ---- GENERATED DOMAIN --------------------------------------------------------
// Small datasets (1..=22 quads) over a pool of <=6 blank nodes, <=3 IRIs, <=3
// predicates and 4 literals, with blank nodes allowed in subject, object AND
// graph-name position. The pool is deliberately SMALL so blank nodes share
// first-degree hashes, and dedicated generators emit directed cycles, symmetric
// (bidirectional) cycles, and PAIRS of disjoint same-shape cycles — the structures
// that force the Hash-N-Degree-Quads recursion to disambiguate — while staying
// far below the HNDQ poison-blowup budget (rdf_canon's default call limit).
//
// ---- NON-VACUITY EVIDENCE ----------------------------------------------------
// Verified locally by inserting deliberate mutations into src/lib.rs, confirming
// proptest found and shrank a counterexample, then reverting (details + shrunk
// counterexamples in the PR body):
//
//   MUTATION 1 (bypass canonicalization): `canonicalize_quads` returned the
//     bridged input serialization sorted verbatim (no rdf_canon call, input labels
//     leak through). Property (a) failed immediately.
//
//   MUTATION 2 (order-sensitive bridge): `bridge_to_02` silently dropped the last
//     quad when len > 1, so a permuted copy canonicalizes a DIFFERENT sub-dataset.
//     Property (b) failed.
//
//   MUTATION 3 (perturb the canonical hash/sort ordering — the bead's named
//     mutation): `canonicalize_quads` reversed the canonical line order before
//     returning (perturbing the code-point ordering of the serialized, hashed
//     quads). Property (d)'s byte-exact reconstruction failed.
//
// After each mutation was confirmed caught, it was reverted; the tests pass
// against the real unmodified code.

use oxrdf::{BlankNode, GraphName, Literal, NamedNode, NamedOrBlankNode, Quad, Term};
use proptest::prelude::*;
use sha2::Sha256;
use sparq_canon::{
    canonicalize, canonicalize_nquads, canonicalize_quads, canonicalize_quads_with, issue_quads,
    issue_quads_with, parse_nquads,
};
use std::collections::{BTreeSet, HashMap, HashSet};

// ---------------------------------------------------------------------------
// STRUCTURED DATASET SPECS — blank nodes are POOL INDICES so the same spec can
// be materialized under different labelings (that is what makes the relabeling
// property directly expressible).
// ---------------------------------------------------------------------------

/// Blank-node pool size. Small on purpose: dense sharing of first-degree hashes.
const MAX_BNODES: usize = 6;

const PREDICATES: [&str; 3] = ["http://ex/p", "http://ex/q", "http://ex/r"];
const IRIS: [&str; 3] = ["http://ex/n0", "http://ex/n1", "http://ex/n2"];

#[derive(Clone, Debug)]
enum NodeSpec {
    Bnode(usize),
    Iri(usize),
}

#[derive(Clone, Debug)]
enum ObjSpec {
    Bnode(usize),
    Iri(usize),
    Lit(usize),
}

#[derive(Clone, Debug)]
enum GraphSpec {
    Default,
    Iri(usize),
    Bnode(usize),
}

#[derive(Clone, Debug)]
struct QuadSpec {
    subject: NodeSpec,
    predicate: usize,
    object: ObjSpec,
    graph: GraphSpec,
}

fn iri(i: usize) -> NamedNode {
    NamedNode::new_unchecked(IRIS[i % IRIS.len()])
}

fn predicate(i: usize) -> NamedNode {
    NamedNode::new_unchecked(PREDICATES[i % PREDICATES.len()])
}

fn literal(i: usize) -> Term {
    match i % 4 {
        0 => Term::Literal(Literal::new_simple_literal("v0")),
        1 => Term::Literal(Literal::new_simple_literal("v1")),
        2 => Term::Literal(Literal::new_language_tagged_literal_unchecked("hey", "en")),
        _ => Term::Literal(Literal::new_typed_literal(
            "42",
            NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#integer"),
        )),
    }
}

/// Materializes a spec under a labeling of the blank-node pool.
fn materialize(spec: &[QuadSpec], label: &dyn Fn(usize) -> String) -> Vec<Quad> {
    spec.iter()
        .map(|q| {
            let subject = match q.subject {
                NodeSpec::Bnode(i) => {
                    NamedOrBlankNode::BlankNode(BlankNode::new_unchecked(label(i)))
                }
                NodeSpec::Iri(i) => NamedOrBlankNode::NamedNode(iri(i)),
            };
            let object = match q.object {
                ObjSpec::Bnode(i) => Term::BlankNode(BlankNode::new_unchecked(label(i))),
                ObjSpec::Iri(i) => Term::NamedNode(iri(i)),
                ObjSpec::Lit(i) => literal(i),
            };
            let graph_name = match q.graph {
                GraphSpec::Default => GraphName::DefaultGraph,
                GraphSpec::Iri(i) => GraphName::NamedNode(iri(i)),
                GraphSpec::Bnode(i) => {
                    GraphName::BlankNode(BlankNode::new_unchecked(label(i)))
                }
            };
            Quad::new(subject, predicate(q.predicate), object, graph_name)
        })
        .collect()
}

fn base_label(i: usize) -> String {
    format!("b{}", i)
}

// ---------------------------------------------------------------------------
// GENERATORS
// ---------------------------------------------------------------------------

/// Subject: blank nodes weighted 3:1 over IRIs so bnode structure dominates.
fn arb_subject() -> impl Strategy<Value = NodeSpec> {
    prop_oneof![
        3 => (0..MAX_BNODES).prop_map(NodeSpec::Bnode),
        1 => (0..IRIS.len()).prop_map(NodeSpec::Iri),
    ]
}

fn arb_object() -> impl Strategy<Value = ObjSpec> {
    prop_oneof![
        3 => (0..MAX_BNODES).prop_map(ObjSpec::Bnode),
        1 => (0..IRIS.len()).prop_map(ObjSpec::Iri),
        1 => (0..4usize).prop_map(ObjSpec::Lit),
    ]
}

/// Graph name: mostly default graph, but named-IRI and BLANK-NODE graph labels
/// are generated too (RDFC-1.0 relabels graph-position blank nodes as well).
fn arb_graph() -> impl Strategy<Value = GraphSpec> {
    prop_oneof![
        4 => Just(GraphSpec::Default),
        1 => (0..IRIS.len()).prop_map(GraphSpec::Iri),
        1 => (0..MAX_BNODES).prop_map(GraphSpec::Bnode),
    ]
}

fn arb_quad() -> impl Strategy<Value = QuadSpec> {
    (arb_subject(), 0..PREDICATES.len(), arb_object(), arb_graph()).prop_map(
        |(subject, predicate, object, graph)| QuadSpec {
            subject,
            predicate,
            object,
            graph,
        },
    )
}

/// Unstructured random datasets: 1..=10 quads over the small pools.
fn arb_random_quads() -> impl Strategy<Value = Vec<QuadSpec>> {
    prop::collection::vec(arb_quad(), 1..=10)
}

/// A directed cycle b0 -> b1 -> ... -> b(k-1) -> b0 over ONE predicate; when
/// `symmetric`, the reverse edge is added too (an undirected cycle with a
/// dihedral automorphism group). Every node shares its first-degree hash, so
/// canonicalization must take the Hash-N-Degree-Quads path. Bounded (k <= 6)
/// so the HNDQ recursion stays far below the poison-blowup call limit.
fn arb_cycle() -> impl Strategy<Value = Vec<QuadSpec>> {
    (2..=MAX_BNODES, 0..PREDICATES.len(), any::<bool>()).prop_map(|(k, p, symmetric)| {
        let mut quads = Vec::new();
        for i in 0..k {
            let j = (i + 1) % k;
            quads.push(QuadSpec {
                subject: NodeSpec::Bnode(i),
                predicate: p,
                object: ObjSpec::Bnode(j),
                graph: GraphSpec::Default,
            });
            if symmetric {
                quads.push(QuadSpec {
                    subject: NodeSpec::Bnode(j),
                    predicate: p,
                    object: ObjSpec::Bnode(i),
                    graph: GraphSpec::Default,
                });
            }
        }
        quads
    })
}

/// TWO disjoint cycles of the same length and predicate (indices 0..k and
/// k..2k) — isomorphic components whose nodes are mutually indistinguishable
/// by first-degree hash, the classic HNDQ disambiguation stressor.
fn arb_twin_cycles() -> impl Strategy<Value = Vec<QuadSpec>> {
    (2..=MAX_BNODES / 2, 0..PREDICATES.len()).prop_map(|(k, p)| {
        let mut quads = Vec::new();
        for c in 0..2 {
            for i in 0..k {
                let j = (i + 1) % k;
                quads.push(QuadSpec {
                    subject: NodeSpec::Bnode(c * k + i),
                    predicate: p,
                    object: ObjSpec::Bnode(c * k + j),
                    graph: GraphSpec::Default,
                });
            }
        }
        quads
    })
}

/// The full generated domain: random quads, cycles, twin cycles, and a random
/// dataset with a cycle woven in (the cycle's node indices overlap the random
/// quads', attaching arbitrary structure to the symmetric core).
fn arb_dataset() -> impl Strategy<Value = Vec<QuadSpec>> {
    prop_oneof![
        3 => arb_random_quads(),
        2 => arb_cycle(),
        1 => arb_twin_cycles(),
        2 => (arb_random_quads(), arb_cycle()).prop_map(|(mut a, c)| {
            a.extend(c);
            a
        }),
    ]
}

/// A random permutation of the blank-node pool indices (an injective relabeling
/// when composed with a fresh prefix).
fn arb_pool_permutation() -> impl Strategy<Value = Vec<usize>> {
    Just((0..MAX_BNODES).collect::<Vec<usize>>()).prop_shuffle()
}

// ---------------------------------------------------------------------------
// RECONSTRUCTION ORACLE for property (d): relabel the INPUT quads through the
// issued map and rebuild the canonical document independently — dedupe (an RDF
// dataset is a set), sort in code point order, one `{quad} .\n` line each,
// mirroring the RDFC-1.0 canonical N-Quads form.
// ---------------------------------------------------------------------------

fn bnode_str(b: &BlankNode, map: &HashMap<String, String>) -> String {
    match map.get(b.as_str()) {
        Some(mapped) => format!("_:{}", mapped),
        None => format!("_:{}", b.as_str()),
    }
}

/// The quad's N-Quads serialization WITHOUT the trailing " ." (the RDFC-1.0
/// sort key), with blank-node labels substituted through `map`.
fn quad_base_string(q: &Quad, map: &HashMap<String, String>) -> String {
    let s = match &q.subject {
        NamedOrBlankNode::NamedNode(n) => n.to_string(),
        NamedOrBlankNode::BlankNode(b) => bnode_str(b, map),
    };
    let o = match &q.object {
        Term::BlankNode(b) => bnode_str(b, map),
        other => other.to_string(),
    };
    match &q.graph_name {
        GraphName::DefaultGraph => format!("{} {} {}", s, q.predicate, o),
        GraphName::NamedNode(n) => format!("{} {} {} {}", s, q.predicate, o, n),
        GraphName::BlankNode(b) => {
            format!("{} {} {} {}", s, q.predicate, o, bnode_str(b, map))
        }
    }
}

/// Independent canonical-document reconstruction from input quads + issued map.
fn reconstruct_canonical(quads: &[Quad], map: &HashMap<String, String>) -> String {
    let lines: BTreeSet<String> = quads.iter().map(|q| quad_base_string(q, map)).collect();
    lines
        .iter()
        .map(|base| format!("{} .\n", base))
        .collect()
}

/// N-Quads document for the quads verbatim (input order, original labels) —
/// the input side of the `canonicalize_nquads` text path.
fn to_input_nquads(quads: &[Quad]) -> String {
    let empty = HashMap::new();
    quads
        .iter()
        .map(|q| format!("{} .\n", quad_base_string(q, &empty)))
        .collect()
}

/// Distinct blank-node labels appearing anywhere in the quads.
fn bnode_labels(quads: &[Quad]) -> HashSet<String> {
    let mut labels = HashSet::new();
    for q in quads {
        if let NamedOrBlankNode::BlankNode(b) = &q.subject {
            labels.insert(b.as_str().to_string());
        }
        if let Term::BlankNode(b) = &q.object {
            labels.insert(b.as_str().to_string());
        }
        if let GraphName::BlankNode(b) = &q.graph_name {
            labels.insert(b.as_str().to_string());
        }
    }
    labels
}

// ---------------------------------------------------------------------------
// THE PROPERTIES
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        ..Default::default()
    })]

    /// (a) Canonical output is byte-identical under blank-node relabeling: the
    /// same structural spec materialized under `b{i}` and under `zz{perm(i)}`
    /// (an injective renaming) canonicalizes identically.
    #[test]
    fn canonical_output_invariant_under_bnode_relabeling(
        spec in arb_dataset(),
        perm in arb_pool_permutation(),
    ) {
        let d1 = materialize(&spec, &base_label);
        let d2 = materialize(&spec, &|i| format!("zz{}", perm[i]));
        let c1 = canonicalize_quads(&d1).expect("canonicalize d1");
        let c2 = canonicalize_quads(&d2).expect("canonicalize d2");
        prop_assert_eq!(
            &c1, &c2,
            "canonical output must not depend on input blank-node labels\nspec: {:?}",
            spec
        );
    }

    /// (b) Canonical output is byte-identical under a random permutation of the
    /// input quad order.
    #[test]
    fn canonical_output_invariant_under_quad_permutation(
        (original, shuffled) in arb_dataset().prop_flat_map(|spec| {
            let d = materialize(&spec, &base_label);
            (Just(d.clone()), Just(d).prop_shuffle())
        })
    ) {
        let c1 = canonicalize_quads(&original).expect("canonicalize original");
        let c2 = canonicalize_quads(&shuffled).expect("canonicalize shuffled");
        prop_assert_eq!(
            &c1, &c2,
            "canonical output must not depend on input quad order\noriginal: {:?}\nshuffled: {:?}",
            original, shuffled
        );
    }

    /// (c) Idempotence: canonicalizing the (parsed-back) canonical output is a
    /// fixpoint — byte-identical to the first canonicalization.
    #[test]
    fn canonicalization_is_idempotent(spec in arb_dataset()) {
        let d = materialize(&spec, &base_label);
        let c1 = canonicalize_quads(&d).expect("first canonicalization");
        let reparsed = parse_nquads(&c1).expect("canonical output must reparse");
        let c2 = canonicalize_quads(&reparsed).expect("second canonicalization");
        prop_assert_eq!(
            &c1, &c2,
            "canonicalize(parse(canonicalize(d))) must equal canonicalize(d)"
        );
    }

    /// (d) `issue_quads_with` <-> `canonicalize_quads_with` agreement: relabeling
    /// the input through the issued map reconstructs the canonical document
    /// byte-for-byte; plus alias/default/text API-path agreement.
    #[test]
    fn issued_map_agrees_with_canonical_output(spec in arb_dataset()) {
        let d = materialize(&spec, &base_label);
        let canon = canonicalize_quads_with::<Sha256>(&d).expect("canonicalize_quads_with");
        let map = issue_quads_with::<Sha256>(&d).expect("issue_quads_with");

        // The issued map covers exactly the blank-node labels of the dataset.
        let labels = bnode_labels(&d);
        prop_assert_eq!(
            &labels,
            &map.keys().cloned().collect::<HashSet<_>>(),
            "issued map must cover exactly the input blank-node labels"
        );

        // Independent reconstruction must be byte-identical.
        let reconstructed = reconstruct_canonical(&d, &map);
        prop_assert_eq!(
            &canon, &reconstructed,
            "input quads relabelled via the issued map must reconstruct the canonical output"
        );

        // API-path agreement: SHA-256 is the spec default; aliases agree; the
        // text path over the serialized input agrees.
        let c_default = canonicalize_quads(&d).expect("canonicalize_quads");
        prop_assert_eq!(&canon, &c_default, "default hash must be SHA-256");
        let c_alias = canonicalize(&d).expect("canonicalize");
        prop_assert_eq!(&c_default, &c_alias, "canonicalize must alias canonicalize_quads");
        let map_default = issue_quads(&d).expect("issue_quads");
        prop_assert_eq!(&map, &map_default, "issue_quads must alias issue_quads_with::<Sha256>");
        let c_text = canonicalize_nquads(&to_input_nquads(&d)).expect("canonicalize_nquads");
        prop_assert_eq!(&c_default, &c_text, "text path must agree with the quads path");
    }
}

// ---------------------------------------------------------------------------
// NON-VACUITY: the generator really produces the structures the properties
// claim to exercise. Sampled deterministically alongside the property runs.
// ---------------------------------------------------------------------------

/// Directed-cycle detection over the bnode->bnode edges (Kahn-style trimming:
/// repeatedly remove nodes without incoming edges; a non-empty remainder that
/// still has edges contains a directed cycle).
fn has_bnode_cycle(spec: &[QuadSpec]) -> bool {
    let mut edges: Vec<(usize, usize)> = Vec::new();
    for q in spec {
        if let (NodeSpec::Bnode(s), ObjSpec::Bnode(o)) = (&q.subject, &q.object) {
            edges.push((*s, *o));
        }
    }
    loop {
        if edges.is_empty() {
            return false;
        }
        let targets: HashSet<usize> = edges.iter().map(|&(_, o)| o).collect();
        let before = edges.len();
        // Keep only edges whose source is itself a target (has an incoming edge).
        edges.retain(|&(s, _)| targets.contains(&s));
        if edges.len() == before {
            return true; // no source could be trimmed: a cycle remains
        }
    }
}

#[test]
fn generator_produces_the_claimed_structures() {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::TestRunner;

    let mut runner = TestRunner::deterministic();
    let strat = arb_dataset();

    let mut saw_multi_bnode = false;
    let mut saw_bnode_graph_name = false;
    let mut saw_symmetric_pair = false;
    let mut saw_cycle = false;
    let mut saw_multi_quad = false;

    for _ in 0..300 {
        let spec = strat.new_tree(&mut runner).unwrap().current();
        let d = materialize(&spec, &base_label);
        if bnode_labels(&d).len() >= 2 {
            saw_multi_bnode = true;
        }
        if d.iter().any(|q| matches!(q.graph_name, GraphName::BlankNode(_))) {
            saw_bnode_graph_name = true;
        }
        let mut fwd = HashSet::new();
        for q in &spec {
            if let (NodeSpec::Bnode(s), ObjSpec::Bnode(o)) = (&q.subject, &q.object) {
                if fwd.contains(&(*o, *s, q.predicate)) && s != o {
                    saw_symmetric_pair = true;
                }
                fwd.insert((*s, *o, q.predicate));
            }
        }
        if has_bnode_cycle(&spec) {
            saw_cycle = true;
        }
        if d.len() >= 4 {
            saw_multi_quad = true;
        }
    }

    assert!(saw_multi_bnode, "generator never produced >=2 blank nodes — relabeling property is VACUOUS");
    assert!(saw_bnode_graph_name, "generator never produced a blank-node graph name — graph-position relabeling untested");
    assert!(saw_symmetric_pair, "generator never produced a symmetric edge pair — HNDQ symmetry untested");
    assert!(saw_cycle, "generator never produced a blank-node cycle — HNDQ recursion untested");
    assert!(saw_multi_quad, "generator never produced >=4 quads — permutation property is near-VACUOUS");
}

// ---------------------------------------------------------------------------
// DETERMINISTIC UNIT ANCHORS — pin one concrete instance of each property and
// a NEGATIVE control (kills a degenerate "return a constant" canonicalizer,
// which would satisfy every equality property above vacuously).
// ---------------------------------------------------------------------------

/// A symmetric (bidirectional) 3-cycle over one predicate: all nodes share
/// their first-degree hash, forcing the HNDQ path.
fn symmetric_three_cycle() -> Vec<QuadSpec> {
    let mut quads = Vec::new();
    for i in 0..3 {
        let j = (i + 1) % 3;
        for (s, o) in [(i, j), (j, i)] {
            quads.push(QuadSpec {
                subject: NodeSpec::Bnode(s),
                predicate: 0,
                object: ObjSpec::Bnode(o),
                graph: GraphSpec::Default,
            });
        }
    }
    quads
}

/// Relabeling a symmetric cycle changes the raw serialization but NOT the
/// canonical output (a concrete, deterministic instance of property (a)).
#[test]
fn symmetric_cycle_relabeling_unit() {
    let spec = symmetric_three_cycle();
    let d1 = materialize(&spec, &base_label);
    let d2 = materialize(&spec, &|i| format!("zz{}", (i + 1) % MAX_BNODES));
    assert_ne!(
        to_input_nquads(&d1),
        to_input_nquads(&d2),
        "the two materializations must differ BEFORE canonicalization (else this test is vacuous)"
    );
    let c1 = canonicalize_quads(&d1).unwrap();
    let c2 = canonicalize_quads(&d2).unwrap();
    assert_eq!(c1, c2, "relabelled symmetric cycle must canonicalize identically");
    assert!(c1.contains("_:c14n"), "blank nodes must be relabelled to c14nN: {}", c1);
}

/// NEGATIVE control: non-isomorphic datasets (3-cycle vs 4-cycle) must NOT
/// canonicalize identically.
#[test]
fn non_isomorphic_datasets_differ_unit() {
    let cycle = |k: usize| -> Vec<Quad> {
        let spec: Vec<QuadSpec> = (0..k)
            .map(|i| QuadSpec {
                subject: NodeSpec::Bnode(i),
                predicate: 0,
                object: ObjSpec::Bnode((i + 1) % k),
                graph: GraphSpec::Default,
            })
            .collect();
        materialize(&spec, &base_label)
    };
    let c3 = canonicalize_quads(&cycle(3)).unwrap();
    let c4 = canonicalize_quads(&cycle(4)).unwrap();
    assert_ne!(c3, c4, "non-isomorphic cycles must canonicalize differently");
}

/// Deterministic instance of property (c): idempotence on the symmetric cycle.
#[test]
fn idempotence_unit() {
    let d = materialize(&symmetric_three_cycle(), &base_label);
    let c1 = canonicalize_quads(&d).unwrap();
    let reparsed = parse_nquads(&c1).unwrap();
    let c2 = canonicalize_quads(&reparsed).unwrap();
    assert_eq!(c1, c2, "canonicalization must be a fixpoint");
}

/// Deterministic instance of property (d): the issued map reconstructs the
/// canonical document for a dataset mixing default graph, named graph, a
/// blank-node graph name, literals, and a duplicate quad (dataset = SET).
#[test]
fn issued_map_agreement_unit() {
    let spec = vec![
        QuadSpec {
            subject: NodeSpec::Bnode(0),
            predicate: 0,
            object: ObjSpec::Bnode(1),
            graph: GraphSpec::Default,
        },
        QuadSpec {
            subject: NodeSpec::Bnode(1),
            predicate: 1,
            object: ObjSpec::Lit(2),
            graph: GraphSpec::Iri(0),
        },
        QuadSpec {
            subject: NodeSpec::Iri(1),
            predicate: 2,
            object: ObjSpec::Lit(3),
            graph: GraphSpec::Bnode(2),
        },
        // Duplicate of the first quad: the canonical form must dedupe it.
        QuadSpec {
            subject: NodeSpec::Bnode(0),
            predicate: 0,
            object: ObjSpec::Bnode(1),
            graph: GraphSpec::Default,
        },
    ];
    let d = materialize(&spec, &base_label);
    let canon = canonicalize_quads_with::<Sha256>(&d).unwrap();
    let map = issue_quads_with::<Sha256>(&d).unwrap();
    assert_eq!(map.len(), 3, "three distinct blank nodes must be issued: {:?}", map);
    let reconstructed = reconstruct_canonical(&d, &map);
    assert_eq!(canon, reconstructed, "issued-map reconstruction must be byte-identical");
    assert_eq!(
        canon.lines().count(),
        3,
        "the duplicate quad must be deduped in the canonical form:\n{}",
        canon
    );
}
