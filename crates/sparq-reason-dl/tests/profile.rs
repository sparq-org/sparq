// sq-pbz04.4.2 — integration tests for the L2 syntactic OWL 2 EL/QL/RL
// profile-membership checker.
//
// 🤖 SPARQ agent. Tests use the STRUCTURAL model directly (no RDF parsing), building
// ontologies by pushing axioms to Ontology::new(). References to the W3C OWL 2 Profiles
// spec (https://www.w3.org/TR/owl2-profiles/) are cited in test names/comments.
//
// Spec cross-checked against W3C OWL 2 Profiles REC 2012-12-11. Five QL/RL grammar
// productions were corrected from the initial implementation; tests 9, 12, 13, 17,
// and assertions within 19/20/28 were rewritten to spec-correct verdicts.

use sparq_reason_dl::{Axiom, ClassExpression as CE, ExtractError, ObjectPropertyExpression as OPE, Ontology};
use sparq_reason_dl::profile::{profiles, profiles_from_extraction, Membership, ProfileSet};

// -------------------------------------------------------------------------------------------
// Tiny helper constructors for building structural axioms in tests.
// -------------------------------------------------------------------------------------------

/// Named class identified by a u32 dict id.
fn named(id: u32) -> CE {
    CE::Class(id)
}

/// Named object property.
fn prop(id: u32) -> OPE {
    OPE::ObjectProperty(id)
}

// Stable, human-readable class ids used across tests.
const A: u32 = 1;
const B: u32 = 2;
const C: u32 = 3;
const R: u32 = 10;

// -------------------------------------------------------------------------------------------
// Test 1: Empty ontology is In all profiles (all profiles admit the empty ontology)
// -------------------------------------------------------------------------------------------

#[test]
fn empty_ontology_in_all_profiles() {
    let onto = Ontology::new();
    let ps = profiles(&onto);
    assert_eq!(ps.el, Membership::In, "empty ontology must be In EL");
    assert_eq!(ps.ql, Membership::In, "empty ontology must be In QL");
    assert_eq!(ps.rl, Membership::In, "empty ontology must be In RL");
    assert!(ps.in_all(), "in_all() must hold for the empty ontology");
    assert!(ps.in_any(), "in_any() must hold for the empty ontology");
}

// -------------------------------------------------------------------------------------------
// Test 2: EL accept — SubClassOf(A, B) with named classes (EL §2 named class leaves)
// -------------------------------------------------------------------------------------------

#[test]
fn el_accept_subclassof_named_classes() {
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::SubClassOf { sub: named(A), sup: named(B) });
    let ps = profiles(&onto);
    assert_eq!(ps.el, Membership::In, "SubClassOf(A, B) with named classes must be In EL (§2)");
}

// -------------------------------------------------------------------------------------------
// Test 3: EL accept — SubClassOf(A ⊓ B, ∃R.A) (intersection and someValuesFrom, EL §2)
// -------------------------------------------------------------------------------------------

#[test]
fn el_accept_intersection_and_somefrom() {
    // EL-CE: ObjectIntersectionOf([A, B]) and ObjectSomeValuesFrom(R, A) both valid (§2)
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::SubClassOf {
        sub: CE::ObjectIntersectionOf(vec![named(A), named(B)]),
        sup: CE::ObjectSomeValuesFrom(prop(R), Box::new(named(A))),
    });
    let ps = profiles(&onto);
    assert_eq!(ps.el, Membership::In, "⊓ and ∃R.A are EL-CEs (EL §2)");
}

// -------------------------------------------------------------------------------------------
// Test 4: EL reject — SubClassOf(A, ∀R.B) (ObjectAllValuesFrom not EL, EL §2)
// -------------------------------------------------------------------------------------------

#[test]
fn el_reject_allvaluesfrom_not_el_ce() {
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::SubClassOf {
        sub: named(A),
        sup: CE::ObjectAllValuesFrom(prop(R), Box::new(named(B))),
    });
    let ps = profiles(&onto);
    assert!(ps.el.is_not_in(), "ObjectAllValuesFrom is not an EL-CE (EL §2)");
}

// -------------------------------------------------------------------------------------------
// Test 5: EL reject — SubClassOf(A, B ⊔ C) (ObjectUnionOf not EL, EL §2)
// -------------------------------------------------------------------------------------------

#[test]
fn el_reject_unionof_not_el_ce() {
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::SubClassOf {
        sub: named(A),
        sup: CE::ObjectUnionOf(vec![named(B), named(C)]),
    });
    let ps = profiles(&onto);
    assert!(ps.el.is_not_in(), "ObjectUnionOf is not an EL-CE (EL §2)");
}

// -------------------------------------------------------------------------------------------
// Test 6: EL reject — SubClassOf(A, ¬B) (ObjectComplementOf not EL, EL §2)
// -------------------------------------------------------------------------------------------

#[test]
fn el_reject_complementof_not_el_ce() {
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::SubClassOf {
        sub: named(A),
        sup: CE::ObjectComplementOf(Box::new(named(B))),
    });
    let ps = profiles(&onto);
    assert!(ps.el.is_not_in(), "ObjectComplementOf is not an EL-CE (EL §2)");
}

// -------------------------------------------------------------------------------------------
// Test 7: QL accept — SubClassOf(A, B) with named classes (QL §3 named class leaves)
// -------------------------------------------------------------------------------------------

#[test]
fn ql_accept_subclassof_named_classes() {
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::SubClassOf { sub: named(A), sup: named(B) });
    let ps = profiles(&onto);
    assert_eq!(ps.ql, Membership::In, "SubClassOf(A, B) with named classes must be In QL (§3)");
}

// -------------------------------------------------------------------------------------------
// Test 8: QL accept — SubClassOf(∃R.⊤, A): ∃R.⊤ is a valid QL sub-CE (QL §3)
// -------------------------------------------------------------------------------------------

#[test]
fn ql_accept_some_thing_as_sub_ce() {
    // QL §3 sub-CE includes ObjectSomeValuesFrom(OPE owl:Thing).
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::SubClassOf {
        sub: CE::ObjectSomeValuesFrom(prop(R), Box::new(CE::Thing)),
        sup: named(A),
    });
    let ps = profiles(&onto);
    assert_eq!(ps.ql, Membership::In, "∃R.⊤ ⊑ A is a valid QL axiom (QL §3 sub-CE)");
}

// -------------------------------------------------------------------------------------------
// Test 9: QL accept — SubClassOf(A, ∃R.B): ∃R.NamedClass IS a valid QL super-CE (QL §3)
// -------------------------------------------------------------------------------------------

#[test]
fn ql_accept_some_named_class_as_super_ce() {
    // QL §3 super-CE includes ObjectSomeValuesFrom(OPE Class) per superObjectSomeValuesFrom.
    // The old code wrongly rejected this; ∃R.B (B a named class) is valid in QL super position.
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::SubClassOf {
        sub: named(A),
        sup: CE::ObjectSomeValuesFrom(prop(R), Box::new(named(B))),
    });
    let ps = profiles(&onto);
    assert_eq!(
        ps.ql,
        Membership::In,
        "∃R.B (B named class) as QL super-CE is valid (QL §3 superObjectSomeValuesFrom)"
    );
}

// -------------------------------------------------------------------------------------------
// Test 10: QL reject — SubClassOf(∃R.B, A) where B ≠ owl:Thing: filler must be ⊤ (QL §3)
// -------------------------------------------------------------------------------------------

#[test]
fn ql_reject_some_non_thing_filler_as_sub_ce() {
    // QL §3 sub-CE admits ObjectSomeValuesFrom only with owl:Thing as filler.
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::SubClassOf {
        sub: CE::ObjectSomeValuesFrom(prop(R), Box::new(named(B))),
        sup: named(A),
    });
    let ps = profiles(&onto);
    assert!(
        ps.ql.is_not_in(),
        "∃R.B (B ≠ owl:Thing) as QL sub-CE is not valid (QL §3)"
    );
}

// -------------------------------------------------------------------------------------------
// Test 11: QL reject — SubClassOf(A, ∀R.B): allValuesFrom not a QL super-CE (QL §3)
// -------------------------------------------------------------------------------------------

#[test]
fn ql_reject_allvaluesfrom_as_super_ce() {
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::SubClassOf {
        sub: named(A),
        sup: CE::ObjectAllValuesFrom(prop(R), Box::new(named(B))),
    });
    let ps = profiles(&onto);
    assert!(ps.ql.is_not_in(), "∀R.B is not a valid QL super-CE (QL §3)");
}

// -------------------------------------------------------------------------------------------
// Test 12: QL accept — SubClassOf(⊥, A): owl:Nothing IS a valid QL sub-CE (QL §3)
// -------------------------------------------------------------------------------------------

#[test]
fn ql_accept_nothing_as_sub_ce() {
    // QL §3 sub-CE production says "Class"; owl:Nothing is a Class and is therefore valid.
    // The old code wrongly rejected owl:Nothing in QL sub position.
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::SubClassOf {
        sub: CE::Nothing,
        sup: named(A),
    });
    let ps = profiles(&onto);
    assert_eq!(
        ps.ql,
        Membership::In,
        "owl:Nothing is a valid QL sub-CE (QL §3 'Class' includes owl:Nothing)"
    );
}

// -------------------------------------------------------------------------------------------
// Test 13: RL reject — SubClassOf(A, B ⊔ C): union is NOT a valid RL super-CE (RL §4)
// -------------------------------------------------------------------------------------------

#[test]
fn rl_reject_union_as_super_ce() {
    // ObjectUnionOf is NOT a valid RL super-CE (RL §4); it only appears in the sub-CE production
    // (subObjectUnionOf). The old code wrongly accepted union in super position.
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::SubClassOf {
        sub: named(A),
        sup: CE::ObjectUnionOf(vec![named(B), named(C)]),
    });
    let ps = profiles(&onto);
    assert!(
        ps.rl.is_not_in(),
        "ObjectUnionOf as RL super-CE is not valid (RL §4 — union is sub-only)"
    );
}

// -------------------------------------------------------------------------------------------
// Test 14: RL accept — SubClassOf(A, ∀R.B): allValuesFrom is a valid RL super-CE (RL §4)
// -------------------------------------------------------------------------------------------

#[test]
fn rl_accept_allvaluesfrom_as_super_ce() {
    // ObjectAllValuesFrom is a valid RL super-CE (RL §4).
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::SubClassOf {
        sub: named(A),
        sup: CE::ObjectAllValuesFrom(prop(R), Box::new(named(B))),
    });
    let ps = profiles(&onto);
    assert_eq!(ps.rl, Membership::In, "∀R.B as RL super-CE is valid (RL §4)");
}

// -------------------------------------------------------------------------------------------
// Test 15: RL accept — SubClassOf(∃R.A, B): someValuesFrom in sub position is RL (RL §4)
// -------------------------------------------------------------------------------------------

#[test]
fn rl_accept_some_in_sub_position() {
    // RL §4 sub-CE includes ObjectSomeValuesFrom(OPE sub-CE).
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::SubClassOf {
        sub: CE::ObjectSomeValuesFrom(prop(R), Box::new(named(A))),
        sup: named(B),
    });
    let ps = profiles(&onto);
    assert_eq!(ps.rl, Membership::In, "∃R.A in RL sub position is valid (RL §4)");
}

// -------------------------------------------------------------------------------------------
// Test 16: RL reject — SubClassOf(A, ∃R.B): someValuesFrom NOT a RL super-CE (RL §4)
// -------------------------------------------------------------------------------------------

#[test]
fn rl_reject_some_in_super_position() {
    // ObjectSomeValuesFrom is not a valid RL super-CE (RL §4).
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::SubClassOf {
        sub: named(A),
        sup: CE::ObjectSomeValuesFrom(prop(R), Box::new(named(B))),
    });
    let ps = profiles(&onto);
    assert!(ps.rl.is_not_in(), "∃R.B as RL super-CE is not valid (RL §4)");
}

// -------------------------------------------------------------------------------------------
// Test 17: RL accept — SubClassOf(A ⊔ B, C): union IS a valid RL sub-CE (RL §4)
// -------------------------------------------------------------------------------------------

#[test]
fn rl_accept_union_in_sub_position() {
    // RL §4 sub-CE includes ObjectUnionOf(sub-CE, sub-CE+) — union is on the SUB side.
    // The old code wrongly rejected union in sub position.
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::SubClassOf {
        sub: CE::ObjectUnionOf(vec![named(A), named(B)]),
        sup: named(C),
    });
    let ps = profiles(&onto);
    assert_eq!(
        ps.rl,
        Membership::In,
        "ObjectUnionOf in RL sub position is valid (RL §4 subObjectUnionOf)"
    );
}

// -------------------------------------------------------------------------------------------
// Test 18: Cross-profile — SubClassOf(A, ∀R.B): NotIn EL, NotIn QL, In RL
// -------------------------------------------------------------------------------------------

#[test]
fn cross_allvalues_not_el_not_ql_in_rl() {
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::SubClassOf {
        sub: named(A),
        sup: CE::ObjectAllValuesFrom(prop(R), Box::new(named(B))),
    });
    let ps = profiles(&onto);
    assert!(ps.el.is_not_in(), "∀R.B is not EL (EL §2)");
    assert!(ps.ql.is_not_in(), "∀R.B is not a valid QL super-CE (QL §3)");
    assert_eq!(ps.rl, Membership::In, "∀R.B is a valid RL super-CE (RL §4)");
    assert!(!ps.in_all());
    assert!(ps.in_any());
}

// -------------------------------------------------------------------------------------------
// Test 19: Cross-profile — SubClassOf(A, ∃R.B): In EL, In QL, NotIn RL
// -------------------------------------------------------------------------------------------

#[test]
fn cross_some_in_el_in_ql_not_rl() {
    // SubClassOf(A, ∃R.B):
    // - EL: In (∃R.B is a valid EL-CE, §2)
    // - QL: In (∃R.B is a valid QL super-CE via superObjectSomeValuesFrom, §3)
    // - RL: NotIn (∃R.B is not a valid RL super-CE, §4)
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::SubClassOf {
        sub: named(A),
        sup: CE::ObjectSomeValuesFrom(prop(R), Box::new(named(B))),
    });
    let ps = profiles(&onto);
    assert_eq!(ps.el, Membership::In, "∃R.B as super is a valid EL-CE (EL §2)");
    assert_eq!(
        ps.ql,
        Membership::In,
        "∃R.B as QL super-CE is valid (QL §3 superObjectSomeValuesFrom)"
    );
    assert!(ps.rl.is_not_in(), "∃R.B as RL super-CE is not valid (RL §4)");
    assert!(!ps.in_all());
    assert!(ps.in_any());
}

// -------------------------------------------------------------------------------------------
// Test 20: Cross-profile — SubClassOf(A, B ⊔ C): NotIn EL, NotIn QL, NotIn RL
// -------------------------------------------------------------------------------------------

#[test]
fn cross_union_not_el_not_ql_not_rl() {
    // SubClassOf(A, B ⊔ C): union in super position is invalid in all three profiles.
    // - EL: NotIn (ObjectUnionOf is not EL, §2)
    // - QL: NotIn (ObjectUnionOf is not a valid QL super-CE, §3)
    // - RL: NotIn (ObjectUnionOf is not a valid RL super-CE, §4; union is sub-side only)
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::SubClassOf {
        sub: named(A),
        sup: CE::ObjectUnionOf(vec![named(B), named(C)]),
    });
    let ps = profiles(&onto);
    assert!(ps.el.is_not_in(), "ObjectUnionOf is not EL (EL §2)");
    assert!(ps.ql.is_not_in(), "ObjectUnionOf is not a valid QL super-CE (QL §3)");
    assert!(
        ps.rl.is_not_in(),
        "ObjectUnionOf in super position is not valid RL (RL §4 — union is sub-only)"
    );
    assert!(!ps.in_all());
    assert!(!ps.in_any());
}

// -------------------------------------------------------------------------------------------
// Test 21: profiles_from_extraction with Ok ontology delegates to profiles()
// -------------------------------------------------------------------------------------------

#[test]
fn profiles_from_extraction_ok_delegates() {
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::SubClassOf { sub: named(A), sup: named(B) });
    let result: Result<Ontology, ExtractError> = Ok(onto.clone());
    let ps_direct = profiles(&onto);
    let ps_via = profiles_from_extraction(&result);
    assert_eq!(ps_direct.el, ps_via.el);
    assert_eq!(ps_direct.ql, ps_via.ql);
    assert_eq!(ps_direct.rl, ps_via.rl);
}

// -------------------------------------------------------------------------------------------
// Test 22: profiles_from_extraction with Err yields Unknown for all three profiles
// -------------------------------------------------------------------------------------------

#[test]
fn profiles_from_extraction_err_all_unknown() {
    let result: Result<Ontology, ExtractError> =
        Err(ExtractError::OutOfFragment("test-fragment".into()));
    let ps = profiles_from_extraction(&result);
    assert!(ps.el.is_unknown(), "extraction failure must yield Unknown EL");
    assert!(ps.ql.is_unknown(), "extraction failure must yield Unknown QL");
    assert!(ps.rl.is_unknown(), "extraction failure must yield Unknown RL");
    // Unknown is not In, so in_all and in_any should both be false.
    assert!(!ps.in_all());
    assert!(!ps.in_any());
}

// -------------------------------------------------------------------------------------------
// Test 23: EquivalentClasses(A, B) with named classes is In RL (§4 requires sub∧super)
// -------------------------------------------------------------------------------------------

#[test]
fn rl_accept_equivalentclasses_named() {
    // Named classes are BOTH RL sub-CE AND RL super-CE, so EquivalentClasses(A, B) is RL.
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::EquivalentClasses(named(A), named(B)));
    let ps = profiles(&onto);
    assert_eq!(ps.rl, Membership::In, "EquivalentClasses(A, B) must be In RL (RL §4)");
}

// -------------------------------------------------------------------------------------------
// Test 24: EquivalentClasses(A, ∃R.C) is NotIn RL (∃R.C is RL sub-CE but NOT super-CE)
// -------------------------------------------------------------------------------------------

#[test]
fn rl_reject_equivalentclasses_some_not_super_ce() {
    // RL §4: each CE in EquivalentClasses must be BOTH sub-CE and super-CE.
    // ∃R.C is a valid RL sub-CE but NOT a valid RL super-CE → the equivalence fails.
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::EquivalentClasses(
        named(A),
        CE::ObjectSomeValuesFrom(prop(R), Box::new(named(C))),
    ));
    let ps = profiles(&onto);
    assert!(
        ps.rl.is_not_in(),
        "EquivalentClasses(A, ∃R.C) must be NotIn RL because ∃R.C is not RL super-CE (RL §4)"
    );
}

// -------------------------------------------------------------------------------------------
// Test 25: Membership helpers (is_in / is_not_in / is_unknown) and ProfileSet (in_all / in_any)
// -------------------------------------------------------------------------------------------

#[test]
fn membership_helpers_and_profileset_helpers() {
    let m_in = Membership::In;
    let m_not = Membership::NotIn("violation".into());
    let m_unk = Membership::Unknown("err".into());

    assert!(m_in.is_in());
    assert!(!m_in.is_not_in());
    assert!(!m_in.is_unknown());

    assert!(!m_not.is_in());
    assert!(m_not.is_not_in());
    assert!(!m_not.is_unknown());

    assert!(!m_unk.is_in());
    assert!(!m_unk.is_not_in());
    assert!(m_unk.is_unknown());

    // in_all: all In
    let all_in = ProfileSet {
        el: Membership::In,
        ql: Membership::In,
        rl: Membership::In,
    };
    assert!(all_in.in_all());
    assert!(all_in.in_any());

    // in_all: none In
    let none_in = ProfileSet {
        el: Membership::NotIn("x".into()),
        ql: Membership::NotIn("x".into()),
        rl: Membership::NotIn("x".into()),
    };
    assert!(!none_in.in_all());
    assert!(!none_in.in_any());

    // in_any: one In
    let one_in = ProfileSet {
        el: Membership::In,
        ql: Membership::NotIn("x".into()),
        rl: Membership::NotIn("x".into()),
    };
    assert!(!one_in.in_all());
    assert!(one_in.in_any());
}

// -------------------------------------------------------------------------------------------
// Test 26: DisjointClasses(A, B) is In all profiles (named classes valid everywhere)
// -------------------------------------------------------------------------------------------

#[test]
fn disjoint_classes_named_in_all_profiles() {
    // DisjointClasses requires both operands to be EL-CE (EL), QL sub-CE (QL), RL sub-CE (RL).
    // Named classes satisfy all of these.
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::DisjointClasses(named(A), named(B)));
    let ps = profiles(&onto);
    assert_eq!(ps.el, Membership::In, "DisjointClasses(A, B) must be In EL (§2)");
    assert_eq!(ps.ql, Membership::In, "DisjointClasses(A, B) must be In QL (§3)");
    assert_eq!(ps.rl, Membership::In, "DisjointClasses(A, B) must be In RL (§4)");
    assert!(ps.in_all());
}

// -------------------------------------------------------------------------------------------
// Test 27: ObjectPropertyDomain / ObjectPropertyRange axioms
// -------------------------------------------------------------------------------------------

#[test]
fn property_domain_range_named_in_all_profiles() {
    // Domain/range with a named class: valid EL-CE (EL §2), QL super-CE (QL §3), RL super-CE (RL §4).
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::ObjectPropertyDomain {
        property: prop(R),
        domain: named(A),
    });
    onto.axioms.push(Axiom::ObjectPropertyRange {
        property: prop(R),
        range: named(A),
    });
    let ps = profiles(&onto);
    assert_eq!(ps.el, Membership::In, "PropertyDomain/Range(R, A) must be In EL (§2)");
    assert_eq!(ps.ql, Membership::In, "PropertyDomain/Range(R, A) must be In QL (§3)");
    assert_eq!(ps.rl, Membership::In, "PropertyDomain/Range(R, A) must be In RL (§4)");
    assert!(ps.in_all());
}

// -------------------------------------------------------------------------------------------
// Test 28: ObjectPropertyDomain with ∃R.A is a valid QL super-CE (QL §3)
// -------------------------------------------------------------------------------------------

#[test]
fn property_domain_some_valid_ql_super_ce() {
    // ObjectPropertyDomain(R, ∃S.A): domain is a super-CE; ∃S.A (A named class) is a valid
    // QL super-CE per QL §3 superObjectSomeValuesFrom. The old code wrongly rejected this.
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::ObjectPropertyDomain {
        property: prop(R),
        domain: CE::ObjectSomeValuesFrom(prop(R), Box::new(named(A))),
    });
    let ps = profiles(&onto);
    assert_eq!(
        ps.ql,
        Membership::In,
        "∃R.A as QL property domain (super-CE) is valid (QL §3 superObjectSomeValuesFrom)"
    );
}

// -------------------------------------------------------------------------------------------
// Test 29: RL rejects owl:Thing in sub position ("Class other than owl:Thing", RL §4)
// -------------------------------------------------------------------------------------------

#[test]
fn rl_reject_thing_in_sub_position() {
    // RL §4 sub-CE says "Class other than owl:Thing"; owl:Thing is explicitly excluded.
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::SubClassOf {
        sub: CE::Thing,
        sup: named(A),
    });
    let ps = profiles(&onto);
    assert!(
        ps.rl.is_not_in(),
        "owl:Thing in RL sub position is not valid (RL §4 'Class other than owl:Thing')"
    );
}

// -------------------------------------------------------------------------------------------
// Test 30: RL accepts owl:Nothing in sub position ("Class other than owl:Thing", RL §4)
// -------------------------------------------------------------------------------------------

#[test]
fn rl_accept_nothing_in_sub_position() {
    // RL §4 sub-CE says "Class other than owl:Thing"; owl:Nothing satisfies this constraint.
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::SubClassOf {
        sub: CE::Nothing,
        sup: named(A),
    });
    let ps = profiles(&onto);
    assert_eq!(
        ps.rl,
        Membership::In,
        "owl:Nothing in RL sub position is valid (RL §4 — satisfies 'Class other than owl:Thing')"
    );
}

// -------------------------------------------------------------------------------------------
// Test 31: RL rejects owl:Thing in super position ("Class other than owl:Thing", RL §4)
// -------------------------------------------------------------------------------------------

#[test]
fn rl_reject_thing_in_super_position() {
    // RL §4 super-CE says "Class other than owl:Thing"; owl:Thing is explicitly excluded.
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::SubClassOf {
        sub: named(A),
        sup: CE::Thing,
    });
    let ps = profiles(&onto);
    assert!(
        ps.rl.is_not_in(),
        "owl:Thing in RL super position is not valid (RL §4 'Class other than owl:Thing')"
    );
}

// -------------------------------------------------------------------------------------------
// Test 32: RL accepts owl:Nothing in super position ("Class other than owl:Thing", RL §4)
// -------------------------------------------------------------------------------------------

#[test]
fn rl_accept_nothing_in_super_position() {
    // RL §4 super-CE says "Class other than owl:Thing"; owl:Nothing satisfies this constraint.
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::SubClassOf {
        sub: named(A),
        sup: CE::Nothing,
    });
    let ps = profiles(&onto);
    assert_eq!(
        ps.rl,
        Membership::In,
        "owl:Nothing in RL super position is valid (RL §4 — satisfies 'Class other than owl:Thing')"
    );
}

// -------------------------------------------------------------------------------------------
// Test 33: RL accepts ∃R.owl:Thing in sub position (§4 subObjectSomeValuesFrom allows ⊤ filler)
// -------------------------------------------------------------------------------------------

#[test]
fn rl_accept_some_thing_filler_in_sub() {
    // RL §4 subObjectSomeValuesFrom explicitly allows owl:Thing as an alternative filler
    // alongside subClassExpression, so ∃R.⊤ is a valid RL sub-CE.
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::SubClassOf {
        sub: CE::ObjectSomeValuesFrom(prop(R), Box::new(CE::Thing)),
        sup: named(A),
    });
    let ps = profiles(&onto);
    assert_eq!(
        ps.rl,
        Membership::In,
        "∃R.owl:Thing in RL sub position is valid (RL §4 subObjectSomeValuesFrom allows ⊤ filler)"
    );
}

// -------------------------------------------------------------------------------------------
// Test 34: QL rejects ClassAssertion with a complex CE (only atomic Class allowed, QL §3)
// -------------------------------------------------------------------------------------------

#[test]
fn ql_reject_class_assertion_complex_ce() {
    // QL §3.2.5: ClassAssertion admits only an atomic Class, not any complex CE.
    // ∃R.⊤ is a valid QL sub-CE but NOT a valid QL class assertion class.
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::ClassAssertion {
        class: CE::ObjectSomeValuesFrom(prop(R), Box::new(CE::Thing)),
        individual: 42,
    });
    let ps = profiles(&onto);
    assert!(
        ps.ql.is_not_in(),
        "∃R.⊤ in QL ClassAssertion is not valid (QL §3 requires atomic Class)"
    );
}

// -------------------------------------------------------------------------------------------
// Test 35: RL accepts ClassAssertion with ∀R.B (super-CE is valid for RL ClassAssertion)
// -------------------------------------------------------------------------------------------

#[test]
fn rl_accept_class_assertion_allvaluesfrom() {
    // RL §4.2.5: ClassAssertion uses a superClassExpression; ∀R.B is a valid RL super-CE.
    // The old code wrongly routed ClassAssertion to sub-CE, rejecting ∀R.B.
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::ClassAssertion {
        class: CE::ObjectAllValuesFrom(prop(R), Box::new(named(B))),
        individual: 42,
    });
    let ps = profiles(&onto);
    assert_eq!(
        ps.rl,
        Membership::In,
        "ClassAssertion(∀R.B, x) is valid RL (RL §4.2.5 uses superClassExpression)"
    );
}

// -------------------------------------------------------------------------------------------
// Test 36: RL accepts ClassAssertion with ¬B (super-CE complement is valid, RL §4)
// -------------------------------------------------------------------------------------------

#[test]
fn rl_accept_class_assertion_complementof() {
    // RL §4.2.5: ClassAssertion uses a superClassExpression; ObjectComplementOf(sub-CE) is valid.
    // The old code wrongly routed ClassAssertion to sub-CE, rejecting ¬B.
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::ClassAssertion {
        class: CE::ObjectComplementOf(Box::new(named(B))),
        individual: 42,
    });
    let ps = profiles(&onto);
    assert_eq!(
        ps.rl,
        Membership::In,
        "ClassAssertion(¬B, x) is valid RL (RL §4.2.5 superClassExpression includes ComplementOf)"
    );
}

// -------------------------------------------------------------------------------------------
// Test 37: Arity guard — QL super-CE intersection with 1 member is rejected
// -------------------------------------------------------------------------------------------

#[test]
fn ql_reject_intersection_arity_one_in_super_ce() {
    // The QL super-CE grammar requires ObjectIntersectionOf to have ≥ 2 conjuncts.
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::SubClassOf {
        sub: named(A),
        sup: CE::ObjectIntersectionOf(vec![named(B)]),
    });
    let ps = profiles(&onto);
    assert!(
        ps.ql.is_not_in(),
        "ObjectIntersectionOf with 1 member is not a valid QL super-CE (arity ≥ 2 required)"
    );
}

// -------------------------------------------------------------------------------------------
// Test 38: Arity guard — RL sub-CE union with 1 member is rejected
// -------------------------------------------------------------------------------------------

#[test]
fn rl_reject_union_arity_one_in_sub_ce() {
    // The RL sub-CE grammar requires ObjectUnionOf to have ≥ 2 disjuncts.
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::SubClassOf {
        sub: CE::ObjectUnionOf(vec![named(A)]),
        sup: named(B),
    });
    let ps = profiles(&onto);
    assert!(
        ps.rl.is_not_in(),
        "ObjectUnionOf with 1 member is not a valid RL sub-CE (arity ≥ 2 required)"
    );
}

// -------------------------------------------------------------------------------------------
// Test 39: QL accept — SubClassOf(A, ∃R.owl:Thing): ∃R.⊤ IS a valid QL super-CE (QL §3.2.3)
//
// W3C OWL 2 Profiles §3.2.3:
//   superObjectSomeValuesFrom := ObjectSomeValuesFrom(ObjectPropertyExpression Class)
//   Class := IRI                                               (OWL 2 Syntax §5.1)
// "Class" is any IRI — it INCLUDES owl:Thing and owl:Nothing.
// "SubClassOf(A, ∃R.⊤)" is the canonical DL-Lite_R "every A-instance has an R-successor"
// axiom (cf. Calvanese et al., PerfectRef §3).
// W3C Table 1 (OWL 2 Profiles §3.1, Superclass column) reads: "existential quantification
// to a class" with NO carve-out of owl:Thing from the class filler — it is unrestricted.
// Asymmetry: the QL SUB-∃ (§3.2.3 subObjectSomeValuesFrom) restricts the filler to
// EXACTLY owl:Thing; the QL SUPER-∃ allows ANY Class filler, owl:Thing included.
// -------------------------------------------------------------------------------------------

#[test]
fn ql_accept_some_thing_as_super_ce() {
    // sq-pbz04.4.2 — TDD: written RED first, then production fixed.
    // QL §3.2.3 superObjectSomeValuesFrom := ∃R.Class; Class ∋ owl:Thing (OWL 2 Syntax §5.1).
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::SubClassOf {
        sub: named(A),
        sup: CE::ObjectSomeValuesFrom(prop(R), Box::new(CE::Thing)),
    });
    let ps = profiles(&onto);
    assert_eq!(
        ps.ql,
        Membership::In,
        "∃R.owl:Thing as QL super-CE must be In (QL §3.2.3 superObjectSomeValuesFrom; \
         DL-Lite_R canonical has-successor axiom)"
    );
}

// -------------------------------------------------------------------------------------------
// Test 40: QL accept — SubClassOf(A, ∃R.owl:Nothing): ∃R.⊥ IS a valid QL super-CE (QL §3.2.3)
//
// Same rationale as Test 39: Class := IRI includes owl:Nothing.
// QL SUPER-∃ allows ANY Class filler (the sub-∃ is the restricted form).
// -------------------------------------------------------------------------------------------

#[test]
fn ql_accept_some_nothing_as_super_ce() {
    // sq-pbz04.4.2 — TDD: written RED first, then production fixed.
    // QL §3.2.3 superObjectSomeValuesFrom := ∃R.Class; Class ∋ owl:Nothing (OWL 2 Syntax §5.1).
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::SubClassOf {
        sub: named(A),
        sup: CE::ObjectSomeValuesFrom(prop(R), Box::new(CE::Nothing)),
    });
    let ps = profiles(&onto);
    assert_eq!(
        ps.ql,
        Membership::In,
        "∃R.owl:Nothing as QL super-CE must be In (QL §3.2.3 superObjectSomeValuesFrom; \
         Class := IRI includes owl:Nothing)"
    );
}

// -------------------------------------------------------------------------------------------
// Test 41: QL accept — ObjectPropertyDomain(R, ∃S.owl:Thing): domain is a super-CE (QL §3.2.5)
//
// QL §3.2.5 ObjectPropertyDomain := ObjectPropertyDomain(ObjectPropertyExpression superCE).
// ∃S.owl:Thing is a valid QL super-CE per §3.2.3 (see Tests 39/40).
// -------------------------------------------------------------------------------------------

#[test]
fn ql_accept_property_domain_some_thing() {
    // sq-pbz04.4.2 — TDD: written RED first, then production fixed.
    // QL §3.2.5 domain := superCE; ∃S.⊤ is a valid QL super-CE per §3.2.3.
    let s: u32 = 11;
    let mut onto = Ontology::new();
    onto.axioms.push(Axiom::ObjectPropertyDomain {
        property: prop(R),
        domain: CE::ObjectSomeValuesFrom(prop(s), Box::new(CE::Thing)),
    });
    let ps = profiles(&onto);
    assert_eq!(
        ps.ql,
        Membership::In,
        "ObjectPropertyDomain(R, ∃S.owl:Thing) must be In QL \
         (QL §3.2.5 domain := superCE; ∃S.⊤ is valid per §3.2.3)"
    );
}

// -------------------------------------------------------------------------------------------
// Opt-in transitive roles (sq-zfwzq, feature `dl_transitive`)
// -------------------------------------------------------------------------------------------

/// TransitiveObjectProperty per the W3C profile grammars: IN EL (§2), NOT in QL (§3 —
/// transitivity is excluded), IN RL (§4).
#[cfg(feature = "dl_transitive")]
#[test]
fn transitive_object_property_profile_membership() {
    let onto = Ontology {
        axioms: vec![Axiom::TransitiveObjectProperty {
            property: OPE::ObjectProperty(10),
        }],
    };
    let ps = profiles(&onto);
    assert!(ps.el.is_in(), "TransitiveObjectProperty is EL (§2)");
    assert!(
        matches!(&ps.ql, Membership::NotIn(reason) if reason.contains("Transitive")),
        "TransitiveObjectProperty is NOT QL (§3), got {:?}",
        ps.ql
    );
    assert!(ps.rl.is_in(), "TransitiveObjectProperty is RL (§4)");
}
