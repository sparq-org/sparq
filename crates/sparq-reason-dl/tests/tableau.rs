// sq-pbz04.4.3 (epic sq-pbz04.4) — integration tests for the L3 ALCH tableau
// (design record research/owl2-direct-semantics-scoping.md §3).
//
// 🤖 SPARQ agent. Four acceptance families (the bead's acceptance criteria):
//   (a) the BLOCKING-TERMINATION CANARY — cyclic TBoxes terminate with a verdict under a
//       tight node cap, proving ancestor subset blocking fired (budget exhaustion would
//       return Unknown, never a verdict), plus the converse canary: a cyclic TBox whose
//       inconsistency blocking must NOT mask;
//   (b) sat/unsat PAIRS exercising ⊔-branching with backtracking and ∀-propagation
//       THROUGH the subPropertyOf hierarchy (a ∀ on a super-role constraining an edge
//       asserted — or ∃-generated — on a sub-role);
//   (c) fragment rejection: out-of-fragment RDF yields Unknown(OutOfFragment) fail-closed
//       BEFORE the tableau starts (end-to-end through the real Dict → extract path);
//   (d) budget exhaustion yields Unknown(ResourceBudget), never a verdict, and which
//       budget trips is deterministic.
//
// Most tests build the STRUCTURAL model directly (the tableau's real input type, as in
// tests/profile.rs); the (c) family and one sat/unsat pair go end-to-end from Turtle.

use sparq_core::dict::Id;
use sparq_core::Graph;
use sparq_reason_dl::model::{Axiom, ClassExpression as CE};
use sparq_reason_dl::model::ObjectPropertyExpression as OPE;
use sparq_reason_dl::tableau::{
    class_satisfiability, consistency, consistency_from_extraction, Budget, ExhaustedBudget,
    UnknownReason, Verdict,
};
use sparq_reason_dl::{extract, Ontology};

// -------------------------------------------------------------------------------------------
// Helpers
// -------------------------------------------------------------------------------------------

const PRE: &str = r#"
    @prefix : <http://ex/> .
    @prefix owl: <http://www.w3.org/2002/07/owl#> .
    @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
    @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
    @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
"#;

/// Parse Turtle and run the REAL fail-closed pipeline: Dict → extract → tableau.
fn check_turtle(body: &str, budget: Budget) -> Verdict {
    let (dict, triples) =
        Graph::parse_to_triples(&format!("{}{}", PRE, body), "turtle").expect("parse");
    consistency_from_extraction(&extract(&dict, &triples), budget)
}

// Stable ids for hand-built structural models (dict ids are opaque u32s to the tableau).
const A: Id = 1;
const B: Id = 2;
const C: Id = 3;
const D: Id = 4;
const R: Id = 10; // roles
const S: Id = 11;
const T: Id = 12;
const IA: Id = 100; // individuals
const IB: Id = 101;

fn named(id: Id) -> CE {
    CE::Class(id)
}
fn not(ce: CE) -> CE {
    CE::ObjectComplementOf(Box::new(ce))
}
fn and(members: Vec<CE>) -> CE {
    CE::ObjectIntersectionOf(members)
}
fn or(members: Vec<CE>) -> CE {
    CE::ObjectUnionOf(members)
}
fn subclass(sub: CE, sup: CE) -> Axiom {
    Axiom::SubClassOf { sub, sup }
}
fn subrole(sub: Id, sup: Id) -> Axiom {
    Axiom::SubObjectPropertyOf {
        sub: OPE::ObjectProperty(sub),
        sup: OPE::ObjectProperty(sup),
    }
}
fn is_a(individual: Id, class: CE) -> Axiom {
    Axiom::ClassAssertion { class, individual }
}
fn edge(source: Id, role: Id, target: Id) -> Axiom {
    Axiom::ObjectPropertyAssertion {
        property: OPE::ObjectProperty(role),
        source,
        target,
    }
}
fn onto(axioms: Vec<Axiom>) -> Ontology {
    Ontology { axioms }
}

// -------------------------------------------------------------------------------------------
// (a) The blocking-termination canary
// -------------------------------------------------------------------------------------------

/// ACCEPTANCE (a): the cyclic TBox `⊤ ⊑ ∃R.C` (every element needs an R-successor —
/// an infinite chain without blocking) must terminate with a VERDICT under a tight node
/// cap. Getting `Satisfiable` under max_nodes = 4 proves the ∃-rule was stopped by
/// ancestor subset blocking, NOT by budget exhaustion — exhaustion would return
/// `Unknown(ResourceBudget)`, never a verdict.
#[test]
fn canary_cyclic_tbox_terminates_by_blocking_under_node_cap() {
    let o = onto(vec![subclass(CE::Thing, CE::some(R, named(C)))]);
    let tight = Budget {
        max_nodes: 4,
        max_rule_applications: 10_000,
    };
    assert_eq!(
        consistency(&o, tight),
        Verdict::Satisfiable,
        "cyclic TBox must quiesce by subset blocking within 4 nodes"
    );
}

/// Canary variant: `C ⊑ ∃R.C` with `C(a)` — the generated successor's label equals the
/// ROOT's, so the root itself is the blocker (module docs §2/§5: a root may block, which
/// is sound precisely because ALCH has no inverse roles).
#[test]
fn canary_root_blocks_cyclic_class() {
    let o = onto(vec![
        subclass(named(C), CE::some(R, named(C))),
        is_a(IA, named(C)),
    ]);
    let tight = Budget {
        max_nodes: 3,
        max_rule_applications: 10_000,
    };
    assert_eq!(consistency(&o, tight), Verdict::Satisfiable);
}

/// Canary variant: a two-class ∃-cycle (A needs an R.B child, B needs an R.A child).
#[test]
fn canary_two_class_existential_cycle() {
    let o = onto(vec![
        subclass(named(A), CE::some(R, named(B))),
        subclass(named(B), CE::some(R, named(A))),
        is_a(IA, named(A)),
    ]);
    let tight = Budget {
        max_nodes: 8,
        max_rule_applications: 10_000,
    };
    assert_eq!(consistency(&o, tight), Verdict::Satisfiable);
}

/// The CONVERSE canary: blocking must not mask a real inconsistency. `⊤ ⊑ ∃R.A` and
/// `⊤ ⊑ ∀R.¬A` clash on every generated successor; an over-eager blocking
/// implementation would wrongly report Satisfiable.
#[test]
fn cyclic_tbox_with_universal_contradiction_is_unsatisfiable() {
    let o = onto(vec![
        subclass(CE::Thing, CE::some(R, named(A))),
        subclass(CE::Thing, CE::only(R, not(named(A)))),
    ]);
    assert_eq!(consistency(&o, Budget::default()), Verdict::Unsatisfiable);
}

// -------------------------------------------------------------------------------------------
// (e) Opt-in transitive roles (`dl_transitive`, sq-zfwzq) — the ∀₊-rule
// -------------------------------------------------------------------------------------------
//
// ACCEPTANCE (bead sq-zfwzq): hand-built ALCH+transitive fixtures with hand-derived
// verdicts, where transitivity is LOAD-BEARING — each unsat case is paired with the SAME
// ontology minus the transitivity declaration, which is satisfiable. The pair IS the
// mutation witness: knocking out the ∀₊-propagation (equivalently, dropping `Trans(R)`)
// flips the verdict, so the pinned Unsatisfiable is non-vacuous. Plus the termination
// stress case: a satisfiable cyclic transitive-role concept MUST block-and-halt.
#[cfg(feature = "dl_transitive")]
mod transitive {
    use super::*;

    const IC: Id = 102;

    fn transitive(role: Id) -> Axiom {
        Axiom::TransitiveObjectProperty {
            property: OPE::ObjectProperty(role),
        }
    }

    /// LOAD-BEARING chain propagation (hand-derived): `Trans(R)`, `R(a,b)`, `R(b,c)`,
    /// `a : ∀R.B`, `c : ¬B`. Transitivity gives `R(a,c)`, so `B(c)` clashes with `¬B(c)`
    /// — Unsatisfiable. In the tableau this is exactly the ∀₊-rule: `∀R.B ∈ L(a)` pushes
    /// `∀R.B` (not just `B`) into `L(b)`, whose `R`-edge to `c` then forces the clash.
    #[test]
    fn transitive_chain_forall_unsat_and_control_sat() {
        let with_trans = onto(vec![
            transitive(R),
            edge(IA, R, IB),
            edge(IB, R, IC),
            is_a(IA, CE::only(R, named(B))),
            is_a(IC, not(named(B))),
        ]);
        assert_eq!(
            consistency(&with_trans, Budget::default()),
            Verdict::Unsatisfiable,
            "Trans(R) must carry ∀R.B two steps down the chain"
        );

        // MUTATION WITNESS (the control): the SAME ontology without `Trans(R)` is
        // satisfiable — `∀R.B` constrains only the direct successor `b`, never `c`. A
        // tableau whose ∀₊-rule is knocked out returns Satisfiable on the fixture above,
        // so the pinned Unsatisfiable is non-vacuous.
        let without_trans = onto(vec![
            edge(IA, R, IB),
            edge(IB, R, IC),
            is_a(IA, CE::only(R, named(B))),
            is_a(IC, not(named(B))),
        ]);
        assert_eq!(
            consistency(&without_trans, Budget::default()),
            Verdict::Satisfiable,
            "without Trans(R) the two-step successor is unconstrained"
        );
    }

    /// The ∀₊-rule on ∃-GENERATED tree successors: `(∃R.∃R.¬B) ⊓ ∀R.B` is unsatisfiable
    /// w.r.t. `{Trans(R)}` (the generated grandchild holds ¬B but transitivity forces B)
    /// and satisfiable w.r.t. the empty ontology (control — the mutation witness).
    #[test]
    fn transitive_generated_successors_unsat_and_control_sat() {
        let concept = and(vec![
            CE::some(R, CE::some(R, not(named(B)))),
            CE::only(R, named(B)),
        ]);
        assert_eq!(
            class_satisfiability(&concept, &onto(vec![transitive(R)]), Budget::default()),
            Verdict::Unsatisfiable
        );
        assert_eq!(
            class_satisfiability(&concept, &onto(vec![]), Budget::default()),
            Verdict::Satisfiable
        );
    }

    /// ∀₊ THROUGH the role hierarchy (the `S ⊑* T ⊑* R` side conditions, hand-derived):
    /// `S ⊑ T ⊑ R`, `Trans(T)`, `a : ∀R.B`, `S(a,b)`, `T(b,c)`. Semantically
    /// `(a,b) ∈ S ⊆ T` and `(b,c) ∈ T`, so `Trans(T)` gives `(a,c) ∈ T ⊆ R`, forcing
    /// `B(c)` — clash with `c : ¬B`. In the tableau: the ∀₊-rule (S ⊑* T, T ⊑* R) puts
    /// `∀T.B` into `L(b)`, whose `T`-edge delivers `B` to `c`. Without `Trans(T)` the
    /// same ontology is satisfiable (control / mutation witness).
    #[test]
    fn transitive_subrole_routes_forall_through_hierarchy() {
        let axioms = |with_trans: bool| {
            let mut v = vec![
                subrole(S, T),
                subrole(T, R),
                is_a(IA, CE::only(R, named(B))),
                edge(IA, S, IB),
                edge(IB, T, IC),
                is_a(IC, not(named(B))),
            ];
            if with_trans {
                v.push(transitive(T));
            }
            onto(v)
        };
        assert_eq!(
            consistency(&axioms(true), Budget::default()),
            Verdict::Unsatisfiable
        );
        assert_eq!(
            consistency(&axioms(false), Budget::default()),
            Verdict::Satisfiable
        );
    }

    /// A transitive role must NOT leak constraints onto an UNRELATED role: `Trans(T)`,
    /// `R(a,b)`, `R(b,c)`, `a : ∀R.B`, `c : ¬B` stays satisfiable when `R` is neither
    /// transitive nor `⊑*`-related to `T` (the ∀₊ side conditions are role-specific).
    #[test]
    fn unrelated_transitive_role_does_not_leak() {
        let o = onto(vec![
            transitive(T),
            edge(IA, R, IB),
            edge(IB, R, IC),
            is_a(IA, CE::only(R, named(B))),
            is_a(IC, not(named(B))),
        ]);
        assert_eq!(consistency(&o, Budget::default()), Verdict::Satisfiable);
    }

    /// ACCEPTANCE (termination stress): a SATISFIABLE cyclic transitive-role concept must
    /// BLOCK-AND-HALT with a definitive verdict — not loop, and not exhaust the budget.
    /// `A ⊑ ∃R.A`, `A ⊑ ∀R.A`, `Trans(R)`: every fresh successor carries `{A, ∃R.A, ∀R.A,
    /// the GCI internalisations}` — the ∀₊-rule re-propagates `∀R.A` down the chain, the
    /// labels equalise, and ancestor subset blocking gates the ∃-rule. A budget-shaped
    /// Unknown here would mean the ∀₊-closure broke the §5a termination argument.
    #[test]
    fn transitive_cycle_blocks_and_halts_satisfiable() {
        let o = onto(vec![
            transitive(R),
            subclass(named(A), CE::some(R, named(A))),
            subclass(named(A), CE::only(R, named(A))),
        ]);
        let blocking_canary = Budget {
            max_nodes: 4,
            max_rule_applications: 10_000,
        };
        assert_eq!(
            class_satisfiability(&named(A), &o, blocking_canary),
            Verdict::Satisfiable,
            "must block-and-halt below the node cap, never exhaust it"
        );
        // The same with a hierarchy above the transitive role (a bigger ∀₊-closure and a
        // ⊔ in the filler) still halts satisfiable.
        let o = onto(vec![
            transitive(S),
            subrole(S, R),
            subclass(named(A), CE::some(S, named(A))),
            subclass(CE::Thing, CE::only(R, or(vec![named(A), named(B)]))),
        ]);
        assert_eq!(
            class_satisfiability(&named(A), &o, Budget::default()),
            Verdict::Satisfiable
        );
    }

    /// End-to-end through the REAL pipeline (Turtle → Dict → extract → tableau): with the
    /// feature ON, `owl:TransitiveProperty` EXTRACTS (no longer fail-closed) and the
    /// chain fixture is decided Unsatisfiable; the same graph minus the transitivity
    /// typing is Satisfiable.
    #[test]
    fn end_to_end_turtle_transitive_chain() {
        let base = ":r a owl:ObjectProperty .\n\
             :a :r :b . :b :r :c .\n\
             :a a [ a owl:Restriction ; owl:onProperty :r ; owl:allValuesFrom :B ] .\n\
             :c a [ owl:complementOf :B ] .";
        let v = check_turtle(
            &format!(":r a owl:TransitiveProperty .\n{}", base),
            Budget::default(),
        );
        assert_eq!(v, Verdict::Unsatisfiable);
        assert_eq!(check_turtle(base, Budget::default()), Verdict::Satisfiable);
    }
}

/// Blocking must not mask an inconsistency that only becomes visible BEYOND depth 1:
/// `A ⊑ ∃R.B`, `B ⊑ ∃R.Q`, `Q ⊑ ⊥` with `A(a)` clashes only when the depth-1 node is
/// expanded to depth 2 — an implementation that wrongly blocks non-root nodes (whose
/// labels are NOT subsets of an ancestor's) would report Satisfiable. (Mutation-derived
/// canary: an over-eager `is_blocked` passed every other unsat test in this suite.)
#[test]
fn unsat_beyond_depth_one_is_not_masked_by_blocking() {
    let o = onto(vec![
        subclass(named(A), CE::some(R, named(B))),
        subclass(named(B), CE::some(R, named(C))),
        subclass(named(C), CE::Nothing),
        is_a(IA, named(A)),
    ]);
    assert_eq!(consistency(&o, Budget::default()), Verdict::Unsatisfiable);
}

/// The blocking test must be `L(descendant) ⊆ L(ancestor)` and not the REVERSE: with
/// `⊤ ⊑ ∃R.(∃S.Q)` and `Q ⊑ ⊥`, the depth-1 node's label is a STRICT SUPERSET of the
/// root's (it carries the extra `∃S.Q`), so superset-"blocking" would freeze it and
/// wrongly report Satisfiable; correct subset blocking must expand it and find the `Q`
/// clash on every branch.
#[test]
fn strict_superset_of_ancestor_must_not_block() {
    let o = onto(vec![
        subclass(CE::Thing, CE::some(R, CE::some(S, named(C)))),
        subclass(named(C), CE::Nothing),
    ]);
    assert_eq!(consistency(&o, Budget::default()), Verdict::Unsatisfiable);
}

// -------------------------------------------------------------------------------------------
// (b) sat/unsat pairs — ⊔-branching with backtracking
// -------------------------------------------------------------------------------------------

/// ACCEPTANCE (b): `A ⊑ B ⊔ C` — asserting `a : A ⊓ ¬B ⊓ ¬C` kills BOTH branches
/// (unsat); dropping the `¬C` conjunct leaves the second branch alive (sat).
#[test]
fn or_branching_sat_unsat_pair() {
    let tbox = subclass(named(A), or(vec![named(B), named(C)]));
    let unsat = onto(vec![
        tbox.clone(),
        is_a(IA, named(A)),
        is_a(IA, not(named(B))),
        is_a(IA, not(named(C))),
    ]);
    assert_eq!(consistency(&unsat, Budget::default()), Verdict::Unsatisfiable);

    let sat = onto(vec![tbox, is_a(IA, named(A)), is_a(IA, not(named(B)))]);
    assert_eq!(consistency(&sat, Budget::default()), Verdict::Satisfiable);
}

/// Backtracking correctness: the FIRST disjunct clashes (`B ⊑ ⊥`), the second survives —
/// the search must backtrack past the clash and find it. Adding `C ⊑ ⊥` too closes every
/// branch.
#[test]
fn or_branch_backtracks_past_first_clash() {
    let sat = onto(vec![
        subclass(named(A), or(vec![named(B), named(C)])),
        subclass(named(B), CE::Nothing),
        is_a(IA, named(A)),
    ]);
    assert_eq!(consistency(&sat, Budget::default()), Verdict::Satisfiable);

    let unsat = onto(vec![
        subclass(named(A), or(vec![named(B), named(C)])),
        subclass(named(B), CE::Nothing),
        subclass(named(C), CE::Nothing),
        is_a(IA, named(A)),
    ]);
    assert_eq!(consistency(&unsat, Budget::default()), Verdict::Unsatisfiable);
}

// -------------------------------------------------------------------------------------------
// (b) sat/unsat pairs — ∀-propagation THROUGH the subPropertyOf hierarchy
// -------------------------------------------------------------------------------------------

/// ACCEPTANCE (b): a ∀ on a SUPER-role constrains an edge asserted on a SUB-role.
/// With `S ⊑ R`: `(∀R.B)(a)`, `S(a,b)`, `(¬B)(b)` is unsat — the ∀R.B must propagate
/// B across the S-edge because S ⊑* R. WITHOUT the hierarchy axiom the same ABox is sat
/// (∀R.B says nothing about a plain S-edge).
#[test]
fn forall_on_super_role_constrains_sub_role_edge_pair() {
    let abox = vec![
        is_a(IA, CE::only(R, named(B))),
        edge(IA, S, IB),
        is_a(IB, not(named(B))),
    ];

    let mut with_hierarchy = abox.clone();
    with_hierarchy.push(subrole(S, R));
    assert_eq!(
        consistency(&onto(with_hierarchy), Budget::default()),
        Verdict::Unsatisfiable,
        "S ⊑ R must route ∀R.B onto the S-edge"
    );

    assert_eq!(
        consistency(&onto(abox), Budget::default()),
        Verdict::Satisfiable,
        "without S ⊑ R the ∀R.B does not constrain an S-edge"
    );
}

/// The hierarchy is closed TRANSITIVELY: `S ⊑ T`, `T ⊑ R` routes `∀R.B` onto an S-edge.
#[test]
fn forall_routes_through_transitive_role_hierarchy() {
    let o = onto(vec![
        subrole(S, T),
        subrole(T, R),
        is_a(IA, CE::only(R, named(B))),
        edge(IA, S, IB),
        is_a(IB, not(named(B))),
    ]);
    assert_eq!(consistency(&o, Budget::default()), Verdict::Unsatisfiable);
}

/// The hierarchy also applies to ∃-GENERATED tree edges, and the ∃-rule's satisfaction
/// check matches modulo ⊑* as well: with `S ⊑ R`, `a : ∃S.A ⊓ ∀R.¬A` is unsat (the
/// generated S-successor holds A and receives ¬A); without the hierarchy it is sat.
#[test]
fn forall_on_super_role_constrains_generated_sub_role_edge_pair() {
    let concept = and(vec![CE::some(S, named(A)), CE::only(R, not(named(A)))]);

    let unsat = onto(vec![subrole(S, R), is_a(IA, concept.clone())]);
    assert_eq!(consistency(&unsat, Budget::default()), Verdict::Unsatisfiable);

    let sat = onto(vec![is_a(IA, concept)]);
    assert_eq!(consistency(&sat, Budget::default()), Verdict::Satisfiable);
}

// -------------------------------------------------------------------------------------------
// (b) further sat/unsat pairs — ∃/∀ interplay, desugared axioms
// -------------------------------------------------------------------------------------------

/// `∃R.A ⊓ ∀R.¬A` is the textbook local clash after one expansion; `∃R.A ⊓ ∀R.B` is sat
/// (the successor simply holds both A and B).
#[test]
fn exists_forall_interaction_pair() {
    let unsat = onto(vec![is_a(
        IA,
        and(vec![CE::some(R, named(A)), CE::only(R, not(named(A)))]),
    )]);
    assert_eq!(consistency(&unsat, Budget::default()), Verdict::Unsatisfiable);

    let sat = onto(vec![is_a(
        IA,
        and(vec![CE::some(R, named(A)), CE::only(R, named(B))]),
    )]);
    assert_eq!(consistency(&sat, Budget::default()), Verdict::Satisfiable);
}

/// `owl:disjointWith` desugars to `A ⊓ B ⊑ ⊥`.
#[test]
fn disjointness_pair() {
    let unsat = onto(vec![
        Axiom::DisjointClasses(named(A), named(B)),
        is_a(IA, named(A)),
        is_a(IA, named(B)),
    ]);
    assert_eq!(consistency(&unsat, Budget::default()), Verdict::Unsatisfiable);

    let sat = onto(vec![
        Axiom::DisjointClasses(named(A), named(B)),
        is_a(IA, named(A)),
    ]);
    assert_eq!(consistency(&sat, Budget::default()), Verdict::Satisfiable);
}

/// `owl:equivalentClass` desugars to a GCI in EACH direction — both must bite.
#[test]
fn equivalence_desugars_to_both_gcis() {
    let equiv = Axiom::EquivalentClasses(named(A), and(vec![named(B), named(C)]));
    // Forward (A ⊑ B ⊓ C): a : A ⊓ ¬B is unsat.
    let forward = onto(vec![
        equiv.clone(),
        is_a(IA, named(A)),
        is_a(IA, not(named(B))),
    ]);
    assert_eq!(consistency(&forward, Budget::default()), Verdict::Unsatisfiable);
    // Backward (B ⊓ C ⊑ A): a : B ⊓ C ⊓ ¬A is unsat.
    let backward = onto(vec![
        equiv.clone(),
        is_a(IA, named(B)),
        is_a(IA, named(C)),
        is_a(IA, not(named(A))),
    ]);
    assert_eq!(consistency(&backward, Budget::default()), Verdict::Unsatisfiable);
    // Consistent use.
    let sat = onto(vec![equiv, is_a(IA, named(A))]);
    assert_eq!(consistency(&sat, Budget::default()), Verdict::Satisfiable);
}

/// `rdfs:domain` (∃R.⊤ ⊑ D) and `rdfs:range` (⊤ ⊑ ∀R.B) both constrain asserted edges.
#[test]
fn domain_and_range_pairs() {
    let domain = Axiom::ObjectPropertyDomain {
        property: OPE::ObjectProperty(R),
        domain: named(D),
    };
    let range = Axiom::ObjectPropertyRange {
        property: OPE::ObjectProperty(R),
        range: named(B),
    };
    let unsat_domain = onto(vec![
        domain.clone(),
        edge(IA, R, IB),
        is_a(IA, not(named(D))),
    ]);
    assert_eq!(
        consistency(&unsat_domain, Budget::default()),
        Verdict::Unsatisfiable
    );
    let unsat_range = onto(vec![
        range.clone(),
        edge(IA, R, IB),
        is_a(IB, not(named(B))),
    ]);
    assert_eq!(
        consistency(&unsat_range, Budget::default()),
        Verdict::Unsatisfiable
    );
    let sat = onto(vec![domain, range, edge(IA, R, IB)]);
    assert_eq!(consistency(&sat, Budget::default()), Verdict::Satisfiable);
}

/// TBox-only inconsistency: Direct-Semantics domains are NON-EMPTY, so `⊤ ⊑ ⊥` with an
/// empty ABox is inconsistent — while `A ⊑ ⊥` alone is fine (A is simply empty).
#[test]
fn tbox_only_ontology_checked_over_nonempty_domain() {
    let unsat = onto(vec![subclass(CE::Thing, CE::Nothing)]);
    assert_eq!(consistency(&unsat, Budget::default()), Verdict::Unsatisfiable);

    let sat = onto(vec![subclass(named(A), CE::Nothing)]);
    assert_eq!(consistency(&sat, Budget::default()), Verdict::Satisfiable);
}

/// Class satisfiability w.r.t. a TBox: `A ⊑ B`, `A ⊑ ¬B` makes A unsatisfiable while B
/// stays satisfiable.
#[test]
fn class_satisfiability_pair() {
    let o = onto(vec![
        subclass(named(A), named(B)),
        subclass(named(A), not(named(B))),
    ]);
    assert_eq!(
        class_satisfiability(&named(A), &o, Budget::default()),
        Verdict::Unsatisfiable
    );
    assert_eq!(
        class_satisfiability(&named(B), &o, Budget::default()),
        Verdict::Satisfiable
    );
}

// -------------------------------------------------------------------------------------------
// (c) Fragment rejection — fail-closed Unknown(OutOfFragment) BEFORE the tableau
// -------------------------------------------------------------------------------------------

/// ACCEPTANCE (c): `owl:inverseOf` is outside ALCH (the deferral the module docs argue is
/// load-bearing for subset blocking) — the real pipeline must abstain, not guess.
#[test]
fn fragment_rejection_inverse_roles_yield_unknown_out_of_fragment() {
    let v = check_turtle(
        ":r owl:inverseOf :s .\n:A rdfs:subClassOf :B .",
        Budget::default(),
    );
    match v {
        Verdict::Unknown(UnknownReason::OutOfFragment(msg)) => {
            assert!(!msg.is_empty(), "diagnostic must not be empty");
        }
        other => panic!("expected Unknown(OutOfFragment), got {:?}", other),
    }
}

/// Cardinality restrictions are outside ALCH: fail-closed abstention, and the presence of
/// otherwise-mappable axioms in the same graph must NOT rescue a verdict (the checker
/// never reasons over a partially-understood graph).
#[test]
fn fragment_rejection_cardinality_yields_unknown_out_of_fragment() {
    let v = check_turtle(
        ":A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :p ; \
         owl:minCardinality \"1\"^^xsd:nonNegativeInteger ] .\n\
         :x a :A .",
        Budget::default(),
    );
    assert!(
        matches!(v, Verdict::Unknown(UnknownReason::OutOfFragment(_))),
        "expected Unknown(OutOfFragment), got {:?}",
        v
    );
}

// -------------------------------------------------------------------------------------------
// (d) Budget exhaustion — Unknown(ResourceBudget), never a verdict
// -------------------------------------------------------------------------------------------

/// ACCEPTANCE (d): the same cyclic TBox as the canary, but with a node cap too small for
/// blocking to be reached: the run must abstain with the NODES budget — it must not
/// fabricate a verdict from a truncated search.
#[test]
fn node_budget_exhaustion_yields_unknown_not_a_verdict() {
    let o = onto(vec![subclass(CE::Thing, CE::some(R, named(C)))]);
    let starved = Budget {
        max_nodes: 2,
        max_rule_applications: 10_000,
    };
    assert_eq!(
        consistency(&o, starved),
        Verdict::Unknown(UnknownReason::ResourceBudget(ExhaustedBudget::Nodes))
    );
}

/// Rule-application starvation trips the other budget arm — again an abstention, never a
/// verdict, even though the ontology is in fact satisfiable.
#[test]
fn rule_budget_exhaustion_yields_unknown_not_a_verdict() {
    let o = onto(vec![subclass(CE::Thing, CE::some(R, named(C)))]);
    let starved = Budget {
        max_nodes: 1_000,
        max_rule_applications: 1,
    };
    assert_eq!(
        consistency(&o, starved),
        Verdict::Unknown(UnknownReason::ResourceBudget(
            ExhaustedBudget::RuleApplications
        ))
    );
}

/// Budgets and search order are deterministic: identical inputs give identical outcomes,
/// including WHICH budget arm trips (module docs §6; wall-clock budgets are banned).
#[test]
fn outcomes_are_deterministic_across_runs() {
    let branching = onto(vec![
        subclass(named(A), or(vec![named(B), named(C)])),
        subclass(named(B), CE::Nothing),
        subclass(named(C), CE::Nothing),
        is_a(IA, named(A)),
    ]);
    assert_eq!(
        consistency(&branching, Budget::default()),
        consistency(&branching, Budget::default())
    );
    let cyclic = onto(vec![subclass(CE::Thing, CE::some(R, named(C)))]);
    let starved = Budget {
        max_nodes: 2,
        max_rule_applications: 10_000,
    };
    assert_eq!(consistency(&cyclic, starved), consistency(&cyclic, starved));
}

// -------------------------------------------------------------------------------------------
// End-to-end through the real RDF pipeline (Turtle → Dict → extract → tableau)
// -------------------------------------------------------------------------------------------

/// A real-path sat/unsat pair: disjoint classes with an individual asserted into both.
#[test]
fn end_to_end_turtle_consistency_pair() {
    let unsat = check_turtle(
        ":A owl:disjointWith :B .\n:x a :A .\n:x a :B .",
        Budget::default(),
    );
    assert_eq!(unsat, Verdict::Unsatisfiable);

    let sat = check_turtle(":A owl:disjointWith :B .\n:x a :A .", Budget::default());
    assert_eq!(sat, Verdict::Satisfiable);
}

/// A real-path role-hierarchy pair mirroring the structural-model test: a range on the
/// SUPER-role constrains an edge asserted on the SUB-role.
#[test]
fn end_to_end_turtle_range_through_subproperty() {
    let unsat = check_turtle(
        ":p a owl:ObjectProperty .\n:q a owl:ObjectProperty .\n\
         :p rdfs:subPropertyOf :q .\n:q rdfs:range :B .\n\
         :x :p :y .\n:y a [ owl:complementOf :B ] .",
        Budget::default(),
    );
    assert_eq!(unsat, Verdict::Unsatisfiable);

    let sat = check_turtle(
        ":p a owl:ObjectProperty .\n:q a owl:ObjectProperty .\n\
         :q rdfs:range :B .\n:x :p :y .\n:y a [ owl:complementOf :B ] .",
        Budget::default(),
    );
    assert_eq!(sat, Verdict::Satisfiable);
}

/// Adversarial-verify canary (PR #1475 round-2): blocking must gate ONLY the generating
/// (∃) rule — the ∀-rule must keep firing into an already-blocked node. The unsound
/// mutant that gates the ∀-rule on blocked targets passes every earlier test but returns
/// a wrong `Satisfiable` here: with `A(a)` seeded, the ∃-child of the root is blocked
/// (its label is a subset of the root's) exactly when the ∀-propagation of `¬A` into it
/// is what exposes the clash. Truly Unsatisfiable: A ⊑ ∃R.A forces an R-successor in A,
/// while A ⊑ ∀R.¬A forces every R-successor out of A.
#[test]
fn forall_must_fire_into_already_blocked_node() {
    let o = onto(vec![
        subclass(named(A), CE::some(R, named(A))),
        subclass(named(A), CE::only(R, not(named(A)))),
        is_a(IA, named(A)),
    ]);
    assert_eq!(consistency(&o, Budget::default()), Verdict::Unsatisfiable);
}
