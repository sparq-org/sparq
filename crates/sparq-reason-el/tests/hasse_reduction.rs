// sq-s2nob: differential-correctness fixtures for the EL+⊥ transitive reduction
// (Phase E3: direct-subsumer Hasse diagram + equivalence-class collapse).
//
// `#![cfg(feature = "hasse")]` — these run ONLY with the `hasse` feature on. A feature-matrix.yml
// leg (`sparq-reason-el (hasse — transitive reduction / Hasse)`) builds + tests + clippies this
// file with the feature enabled, so it is NOT a compiled-empty-and-never-run suite (the gap #1171
// flagged); the default build (and ci.yml's `--workspace --all-targets` archive, which passes no
// `--features`) compiles it empty.
//
// ORACLE NOTE (same discipline as tests/differential.rs + tests/rbox_differential.rs): the
// expected DIRECT-subsumer (Hasse) diagrams are hand-derived from the FULL closure ELK's CR1–CR5
// calculus produces, then transitively reduced by hand. Each fixture is small enough to verify
// the Hasse edges + equivalence cliques by eye. The two invariants asserted are the ones that
// make a transitive reduction CORRECT: (1) the closure of the direct edges (chased through
// equivalence cliques) RE-DERIVES the full subsumption relation — no edge is wrongly dropped;
// and (2) NO direct edge is redundant — every direct super is immediate. Where ELK is available
// locally its taxonomy can cross-check the same Turtle offline (a developer step, not a gate).

#![cfg(feature = "hasse")]

use oxrdf::{NamedNode, Term as OTerm};
use sparq_core::dict::{Dict, Id};
use sparq_core::Graph;
use sparq_reason_el::{classify_hasse_graph, Classifier, DirectHierarchy};

const PRE: &str = r#"
    @prefix : <http://ex/> .
    @prefix owl: <http://www.w3.org/2002/07/owl#> .
    @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
"#;

fn iri(dict: &Dict, frag: &str) -> Id {
    dict.lookup(&OTerm::NamedNode(NamedNode::new_unchecked(format!(
        "http://ex/{frag}"
    ))))
}

/// Asserts the DIRECT-subsumer (Hasse) diagram EXACTLY equals `expected` over `classes`: for each
/// class C, `direct_super_classes(C)` (mapped back to fragment names) is the set in `expected`.
/// Then independently re-checks the two transitive-reduction invariants against the full closure.
fn assert_hasse(ttl: &str, classes: &[&str], expected: &[(&str, &[&str])]) {
    let (dict, triples) = Graph::parse_to_triples(ttl, "turtle").expect("parse");
    let closure = Classifier::classify(&dict, &triples);
    let direct = DirectHierarchy::from_closure(&closure);

    // (a) Each class's direct supers match the hand-derived Hasse row exactly (as name SETS;
    // a non-representative clique member shares its rep's row, handled by the API).
    let exp: std::collections::HashMap<&str, std::collections::HashSet<&str>> = expected
        .iter()
        .map(|(c, sups)| (*c, sups.iter().copied().collect()))
        .collect();
    for &c in classes {
        let cid = iri(&dict, c);
        // Skip non-representative members for the row check (their row is the rep's, asserted via
        // the rep's own entry); the closure re-derivation invariant below covers them.
        if direct.representative(cid) != cid {
            continue;
        }
        let got: std::collections::HashSet<&str> = direct
            .direct_super_classes(cid)
            .iter()
            .map(|&d| name_of(&dict, classes, d))
            .collect();
        let want = exp.get(c).cloned().unwrap_or_default();
        assert_eq!(
            got, want,
            "direct supers of {c}: got {got:?}, want {want:?}"
        );
    }

    // (b) Transitive-reduction invariant: chasing direct edges (through equivalence cliques) must
    // re-derive EXACTLY the full subsumption closure — nothing dropped, nothing spurious.
    for &sub in classes {
        for &sup in classes {
            if sub == sup {
                continue;
            }
            let want = closure.is_subclass_of(iri(&dict, sub), iri(&dict, sup));
            let got = reachable_via_direct(&dict, classes, &direct, sub, sup);
            assert_eq!(
                got, want,
                "closure of direct edges {sub} ⊑ {sup}: chased {got}, closure {want}"
            );
        }
    }
}

/// Maps a dict id back to its fragment name within `classes` (panics if unknown — fixtures list
/// every named class).
fn name_of<'a>(dict: &Dict, classes: &[&'a str], id: Id) -> &'a str {
    classes
        .iter()
        .copied()
        .find(|&c| iri(dict, c) == id)
        .unwrap_or_else(|| panic!("dict id {id} not a named fixture class"))
}

/// Is `sup` reachable from `sub` by following DIRECT edges, treating each equivalence clique as a
/// single node (an equivalent of a reached node is itself reached)? This independently reconstructs
/// the closure from the Hasse diagram — the reduction is correct iff this matches the saturator.
fn reachable_via_direct(
    dict: &Dict,
    classes: &[&str],
    direct: &DirectHierarchy,
    sub: &str,
    sup: &str,
) -> bool {
    let sup_id = iri(dict, sup);
    let sup_rep = direct.representative(sup_id);
    let mut stack = vec![iri(dict, sub)];
    let mut seen = std::collections::HashSet::new();
    while let Some(node) = stack.pop() {
        let node_rep = direct.representative(node);
        if !seen.insert(node_rep) {
            continue;
        }
        // An equivalent of the target counts as reaching it (clique = one taxonomy node).
        if node_rep == sup_rep && node != iri(dict, sub) {
            return true;
        }
        // Also: equivalents reached "for free" — if sub ≡ sup they are the same node, but sub ≠ sup
        // here, so only a proper hop counts.
        for &d in direct.direct_super_classes(node_rep) {
            stack.push(d);
        }
        // Mark equivalents seen too (so we don't loop), and check if sup is among them.
        for &e in &direct.equivalent_classes(node_rep) {
            if e == sup_id {
                return true;
            }
            let _ = classes; // (classes kept for symmetry with name_of; equivalents are ids)
            seen.insert(direct.representative(e));
        }
    }
    false
}

#[test]
fn chain_reduces_to_single_direct_edges() {
    // A ⊑ B ⊑ C ⊑ D. Full closure: 6 edges (A⊑B,A⊑C,A⊑D,B⊑C,B⊑D,C⊑D). Hasse: 3 edges
    // (A→B, B→C, C→D) — each class's ONLY direct super is the next.
    let ttl =
        format!("{PRE} :A rdfs:subClassOf :B . :B rdfs:subClassOf :C . :C rdfs:subClassOf :D .");
    assert_hasse(
        &ttl,
        &["A", "B", "C", "D"],
        &[("A", &["B"]), ("B", &["C"]), ("C", &["D"])],
    );
}

#[test]
fn diamond_drops_the_transitive_edge() {
    // A ⊑ B, A ⊑ C, B ⊑ D, C ⊑ D. Closure adds A⊑D. Hasse: A→B, A→C, B→D, C→D — the A→D edge
    // is REDUNDANT (implied by A→B→D) and must NOT be a direct edge.
    let ttl = format!(
        "{PRE}
         :A rdfs:subClassOf :B . :A rdfs:subClassOf :C .
         :B rdfs:subClassOf :D . :C rdfs:subClassOf :D ."
    );
    assert_hasse(
        &ttl,
        &["A", "B", "C", "D"],
        &[("A", &["B", "C"]), ("B", &["D"]), ("C", &["D"])],
    );
}

#[test]
fn equivalence_clique_collapses() {
    // A ≡ B (mutual subsumption), both ⊑ C. The Hasse has ONE node {A,B} with a single direct
    // edge to C; A and B are equivalents, not super/sub of each other.
    let ttl = format!(
        "{PRE}
         :A rdfs:subClassOf :B . :B rdfs:subClassOf :A .
         :A rdfs:subClassOf :C ."
    );
    let (dict, triples) = Graph::parse_to_triples(&ttl, "turtle").expect("parse");
    let closure = Classifier::classify(&dict, &triples);
    let direct = DirectHierarchy::from_closure(&closure);
    let (a, b, c) = (iri(&dict, "A"), iri(&dict, "B"), iri(&dict, "C"));
    // A and B share one representative.
    assert_eq!(direct.representative(a), direct.representative(b));
    // Each sees the other as its equivalent.
    assert_eq!(direct.equivalent_classes(a), vec![b]);
    assert_eq!(direct.equivalent_classes(b), vec![a]);
    // The clique's only direct super is C (asserted via the representative's row).
    assert_eq!(direct.direct_super_classes(a), &[c][..]);
    assert_eq!(direct.equivalence_class_count(), 1);
}

#[test]
fn cr4_existential_closure_reduces_correctly() {
    // The spike CR4 ontology: A ⊑ ∃r.B, B ⊑ C, ∃r.C ⊑ D, D ⊑ E. Full closure: B⊑C, A⊑D, A⊑E,
    // D⊑E. Hasse over {A,B,C,D,E}: B→C, A→D, D→E (A⊑E is implied by A→D→E and must be dropped).
    let ttl = format!(
        "{PRE}
         :A rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :B ] .
         :B rdfs:subClassOf :C .
         [ owl:onProperty :r ; owl:someValuesFrom :C ] rdfs:subClassOf :D .
         :D rdfs:subClassOf :E ."
    );
    assert_hasse(
        &ttl,
        &["A", "B", "C", "D", "E"],
        &[("A", &["D"]), ("B", &["C"]), ("D", &["E"])],
    );
}

#[test]
fn deep_chain_edge_count_is_linear() {
    // A depth-N chain has N·(N+1)/2 CLOSURE edges but exactly N DIRECT edges — the whole point of
    // the reduction. Assert the deterministic Hasse edge COUNT (the E3 acceptance metric).
    const N: usize = 200;
    let mut dict = Dict::new();
    let sc = dict.intern_iri("http://www.w3.org/2000/01/rdf-schema#subClassOf");
    let classes: Vec<Id> = (0..=N)
        .map(|i| dict.intern_iri(&format!("http://ex/C{i}")))
        .collect();
    let triples: Vec<[Id; 3]> = (0..N).map(|i| [classes[i], sc, classes[i + 1]]).collect();
    let closure = Classifier::classify(&dict, &triples);
    let direct = DirectHierarchy::from_closure(&closure);
    // Closure cross-check: total closure edges = N·(N+1)/2.
    let closure_edges: usize = (0..=N)
        .map(|i| closure.super_classes(classes[i]).len())
        .sum();
    assert_eq!(
        closure_edges,
        N * (N + 1) / 2,
        "closure edge count closed form"
    );
    // Hasse cross-check: exactly N direct edges (each Ci → C{i+1}).
    assert_eq!(
        direct.direct_edge_count(),
        N,
        "Hasse edge count must be linear (N)"
    );
    for i in 0..N {
        assert_eq!(
            direct.direct_super_classes(classes[i]),
            &[classes[i + 1]][..],
            "C{i}'s only direct super is C{}",
            i + 1
        );
    }
}

#[test]
fn classify_hasse_graph_emits_compact_taxonomy_and_is_idempotent() {
    // A ⊑ B ⊑ C plus A ≡ A2. Emitted: 2 direct subClassOf edges (A→B asserted already? no: A⊑B
    // and B⊑C are asserted, so the only NEW direct edge would be none for the chain) + 1 equiv.
    // Use a chain where the transitive edge is NOT asserted to see a fresh direct edge.
    let ttl = format!(
        "{PRE}
         :A rdfs:subClassOf :B . :B rdfs:subClassOf :C .
         :X rdfs:subClassOf :Y . :Y rdfs:subClassOf :X ."
    );
    let (mut dict, mut triples) = Graph::parse_to_triples(&ttl, "turtle").expect("parse");
    let before = triples.len();
    let (subs, equivs) = classify_hasse_graph(&mut dict, &mut triples);
    // Direct subClassOf edges: A→B and B→C are the Hasse edges; both are already asserted, so 0
    // NEW subClassOf triples. The equivalence X≡Y is NEW: 1 owl:equivalentClass triple.
    assert_eq!(subs, 0, "all direct subClassOf edges were already asserted");
    assert_eq!(equivs, 1, "the X≡Y equivalence is emitted once");
    assert_eq!(triples.len(), before + 1);
    // Idempotent.
    let (s2, e2) = classify_hasse_graph(&mut dict, &mut triples);
    assert_eq!((s2, e2), (0, 0), "second call adds nothing");
}

#[test]
fn classify_hasse_graph_emits_unasserted_direct_edges() {
    // A ⊑ ∃r.B, B ⊑ C, ∃r.C ⊑ D: the DERIVED A⊑D is a Hasse edge NOT in the source — it must be
    // emitted by classify_hasse_graph (the whole value of EL classification surfacing as a triple).
    let ttl = format!(
        "{PRE}
         :A rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :B ] .
         :B rdfs:subClassOf :C .
         [ owl:onProperty :r ; owl:someValuesFrom :C ] rdfs:subClassOf :D ."
    );
    let (mut dict, mut triples) = Graph::parse_to_triples(&ttl, "turtle").expect("parse");
    let sub_class_of = dict.intern_iri("http://www.w3.org/2000/01/rdf-schema#subClassOf");
    let (a, d) = (iri(&dict, "A"), iri(&dict, "D"));
    let (subs, equivs) = classify_hasse_graph(&mut dict, &mut triples);
    assert!(subs >= 1, "at least the derived A⊑D direct edge is emitted");
    assert_eq!(equivs, 0, "no equivalences in this fixture");
    assert!(
        triples.contains(&[a, sub_class_of, d]),
        "the derived A⊑D edge must be materialized as a triple"
    );
}

#[test]
fn three_cycle_clique_and_shared_parent() {
    // A ≡ B ≡ C (a 3-cycle), all ⊑ P, and a SECOND clique X ≡ Y also ⊑ P. The reduction must
    // collapse {A,B,C} to one node and {X,Y} to another, each with ONE direct edge to P — and the
    // re-derived closure must still report every cross-clique subsumption (A⊑P, X⊑P, …).
    let ttl = format!(
        "{PRE}
         :A rdfs:subClassOf :B . :B rdfs:subClassOf :C . :C rdfs:subClassOf :A .
         :X rdfs:subClassOf :Y . :Y rdfs:subClassOf :X .
         :A rdfs:subClassOf :P . :X rdfs:subClassOf :P ."
    );
    let (dict, triples) = Graph::parse_to_triples(&ttl, "turtle").expect("parse");
    let closure = Classifier::classify(&dict, &triples);
    let direct = DirectHierarchy::from_closure(&closure);
    let p = iri(&dict, "P");
    // Two non-trivial cliques.
    assert_eq!(direct.equivalence_class_count(), 2, "{{A,B,C}} and {{X,Y}}");
    // Every member of the ABC clique shares the SAME representative and the SAME equivalents set.
    for frag in ["A", "B", "C"] {
        let id = iri(&dict, frag);
        assert_eq!(
            direct.representative(id),
            direct.representative(iri(&dict, "A"))
        );
        assert_eq!(
            direct.equivalent_classes(id).len(),
            2,
            "{frag} has 2 equivalents"
        );
        // The clique's only direct super is P.
        assert_eq!(direct.direct_super_classes(id), &[p][..]);
    }
    for frag in ["X", "Y"] {
        let id = iri(&dict, frag);
        assert_eq!(
            direct.equivalent_classes(id).len(),
            1,
            "{frag} has 1 equivalent"
        );
        assert_eq!(direct.direct_super_classes(id), &[p][..]);
    }
    // Hasse edges: one per clique to P = 2 (NOT one per member: collapse worked).
    assert_eq!(direct.direct_edge_count(), 2, "one direct edge per clique");
    // Closure re-derivation: every member ⊑ P, but members of DIFFERENT cliques are not related.
    assert!(closure.is_subclass_of(iri(&dict, "A"), p));
    assert!(closure.is_subclass_of(iri(&dict, "X"), p));
    assert!(!closure.is_subclass_of(iri(&dict, "A"), iri(&dict, "X")));
}

#[test]
fn empty_graph_is_safe() {
    let (dict, triples) = (Dict::new(), Vec::<[Id; 3]>::new());
    let closure = Classifier::classify(&dict, &triples);
    let direct = DirectHierarchy::from_closure(&closure);
    assert_eq!(direct.direct_edge_count(), 0);
    assert_eq!(direct.equivalence_class_count(), 0);
}

// ---------------------------------------------------------------------------------------------
// sq-bif: Hasse public-accessor edge-case coverage. The fixtures above assert the
// reduction over REPRESENTATIVE rows + the count metrics; the additions below pin the documented
// fallbacks of the three lookup accessors on inputs the existing suite never passes them: a class
// equivalent to nothing, an unknown dict id, and a NON-representative clique member (whose row and
// equivalents must be derived through its representative, not its own absent entry).

#[test]
fn accessor_fallbacks_for_non_equivalent_unknown_and_clique_member() {
    // A ⊑ B, plus an equivalence clique X ≡ Y, all under nothing else. Exercises every accessor's
    // documented fallback in one fixture.
    let ttl = format!(
        "{PRE}
         :A rdfs:subClassOf :B .
         :X rdfs:subClassOf :Y . :Y rdfs:subClassOf :X . :X rdfs:subClassOf :B ."
    );
    let (dict, triples) = Graph::parse_to_triples(&ttl, "turtle").expect("parse");
    let closure = Classifier::classify(&dict, &triples);
    let direct = DirectHierarchy::from_closure(&closure);
    let (a, b, x, y) = (
        iri(&dict, "A"),
        iri(&dict, "B"),
        iri(&dict, "X"),
        iri(&dict, "Y"),
    );

    // (1) A class equivalent to nothing is its OWN representative and has an EMPTY equivalents set.
    assert_eq!(direct.representative(a), a, "A is its own representative");
    assert!(
        direct.equivalent_classes(a).is_empty(),
        "A has no equivalents"
    );

    // (2) An unknown dict id falls back to itself as representative, with empty direct supers /
    // equivalents — no panic, no spurious membership.
    let absent = dict.lookup(&OTerm::NamedNode(NamedNode::new_unchecked(
        "http://ex/absent".to_string(),
    )));
    assert_eq!(
        direct.representative(absent),
        absent,
        "unknown id maps to itself"
    );
    assert!(
        direct.direct_super_classes(absent).is_empty(),
        "unknown id has no direct supers"
    );
    assert!(
        direct.equivalent_classes(absent).is_empty(),
        "unknown id has no equivalents"
    );

    // (3) X and Y form one clique. WHICHEVER is the non-representative member must still report the
    // SAME representative, the SAME direct supers (the clique's shared parent B, looked up via the
    // rep), and the OTHER member as its equivalent (rep-relative, minus itself).
    let rep = direct.representative(x);
    assert_eq!(
        direct.representative(y),
        rep,
        "X and Y share a representative"
    );
    assert!(rep == x || rep == y, "the rep is one of the clique members");
    let non_rep = if rep == x { y } else { x };
    assert_eq!(
        direct.direct_super_classes(non_rep),
        &[b][..],
        "the non-representative member sees the clique's direct super B"
    );
    assert_eq!(
        direct.equivalent_classes(non_rep),
        vec![rep],
        "the non-representative member's only equivalent is the representative"
    );
    assert_eq!(
        direct.equivalent_classes(rep),
        vec![non_rep],
        "the representative's only equivalent is the other member"
    );
}
