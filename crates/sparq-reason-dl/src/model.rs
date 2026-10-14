// sq-pbz04.4.1 (epic sq-pbz04.4, design record
// research/owl2-direct-semantics-scoping.md §3): the structural OWL model for the ALCH
// fragment.
//
// 🤖 SPARQ agent. This is a PURELY STRUCTURAL typed AST — it attaches NO semantics. The
// reverse RDF mapping (`crate::extract`) builds it; the downstream layers give it meaning:
// the syntactic profile checker (L2, `profile`), the ALCH tableau (L3, `nnf`/`tableau`), and
// the fragment-dispatch checker (L4, `check`). Keeping the model semantics-free is what lets
// each of those layers state its own soundness story over the same faithful representation.
//
// FRAGMENT (design record §3): ALCH class expressions (named classes, ⊤/⊥, ⊓, ⊔, ¬, ∃R.C,
// ∀R.C over NAMED object properties only), general TBoxes (GCIs + equivalence + disjointness),
// `rdfs:domain`/`rdfs:range`, `rdfs:subPropertyOf` role hierarchies, and a ground ABox (class
// and role assertions). Everything OUTSIDE this fragment is deferred with a named reason in
// the design record §5 deferral ledger — it is NOT modelled here, and the extractor
// fail-closes on it rather than dropping it silently.
//
// sq-zfwzq: the OPT-IN `dl_transitive` cargo feature (OFF by default) extends the
// fragment with TRANSITIVE roles (`owl:TransitiveProperty` → `Axiom::TransitiveObjectProperty`)
// — the design record's "first-in-line" §5 extension. Inverses / cardinality / nominals /
// datatypes remain fail-closed deferrals in BOTH feature states.

use sparq_core::dict::Id;

/// An object-property expression in the ALCH fragment.
///
/// OWL 2's grammar is `ObjectPropertyExpression := ObjectProperty | InverseObjectProperty`.
/// The L1 fragment admits ONLY named object properties; inverse properties are the
/// first-in-line deferral (design record §5 — subset blocking becomes incomplete once
/// constraints can propagate back through an inverse, so `owl:inverseOf` is a dedicated
/// follow-up bead, not a silent addition). The single variant is therefore deliberate: the
/// place an `InverseObjectProperty` variant would go is exactly where the L3 tableau's
/// termination/completeness argument would need to change.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ObjectPropertyExpression {
    /// A named object property, identified by its `sparq_core` dict id.
    ObjectProperty(Id),
}

impl ObjectPropertyExpression {
    /// The dict id of the underlying named object property.
    ///
    /// Total in L1 (the only variant is `ObjectProperty`); it is written as a method rather
    /// than a field access so an `InverseObjectProperty` variant can later return the base
    /// property id without breaking callers.
    #[must_use]
    pub fn named(&self) -> Id {
        match self {
            ObjectPropertyExpression::ObjectProperty(id) => *id,
        }
    }
}

/// A class expression in the ALCH fragment (design record §3). Boolean and quantifier nodes
/// nest arbitrarily; the leaves are named classes and the two constants ⊤/⊥.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ClassExpression {
    /// A named class `A`, identified by its dict id.
    Class(Id),
    /// `owl:Thing` — the top concept ⊤.
    Thing,
    /// `owl:Nothing` — the bottom concept ⊥.
    Nothing,
    /// `owl:intersectionOf` — a conjunction `C₁ ⊓ … ⊓ Cₙ` (`n ≥ 1`).
    ObjectIntersectionOf(Vec<ClassExpression>),
    /// `owl:unionOf` — a disjunction `C₁ ⊔ … ⊔ Cₙ` (`n ≥ 1`).
    ObjectUnionOf(Vec<ClassExpression>),
    /// `owl:complementOf` — a negation `¬C`.
    ObjectComplementOf(Box<ClassExpression>),
    /// `owl:someValuesFrom` on a named property — the existential restriction `∃R.C`.
    ObjectSomeValuesFrom(ObjectPropertyExpression, Box<ClassExpression>),
    /// `owl:allValuesFrom` on a named property — the universal restriction `∀R.C`.
    ObjectAllValuesFrom(ObjectPropertyExpression, Box<ClassExpression>),
}

impl ClassExpression {
    /// Convenience constructor for `∃R.C` over a named property id.
    #[must_use]
    pub fn some(property: Id, filler: ClassExpression) -> ClassExpression {
        ClassExpression::ObjectSomeValuesFrom(
            ObjectPropertyExpression::ObjectProperty(property),
            Box::new(filler),
        )
    }

    /// Convenience constructor for `∀R.C` over a named property id.
    #[must_use]
    pub fn only(property: Id, filler: ClassExpression) -> ClassExpression {
        ClassExpression::ObjectAllValuesFrom(
            ObjectPropertyExpression::ObjectProperty(property),
            Box::new(filler),
        )
    }

    /// `true` iff this expression is a leaf — a named class or a constant (⊤/⊥). Used by the
    /// downstream layers to decide when structural recursion bottoms out.
    #[must_use]
    pub fn is_atomic(&self) -> bool {
        matches!(
            self,
            ClassExpression::Class(_) | ClassExpression::Thing | ClassExpression::Nothing
        )
    }
}

/// An axiom in the ALCH fragment (design record §3): TBox GCIs / equivalence / disjointness,
/// the RBox role hierarchy, `rdfs:domain`/`rdfs:range`, and the ground ABox.
///
/// `owl:equivalentClass` and `owl:disjointWith` are kept as first-class
/// [`Axiom::EquivalentClasses`] / [`Axiom::DisjointClasses`] rather than desugared into GCIs
/// at extraction time, and `rdfs:domain`/`rdfs:range` are kept as first-class
/// [`Axiom::ObjectPropertyDomain`] / [`Axiom::ObjectPropertyRange`]. L1 is a FAITHFUL
/// structural view of the graph; the semantic desugarings the design record §3 lists
/// (`equivalentClass` → two GCIs, `disjointWith` → `C ⊓ D ⊑ ⊥`, `domain`/`range` →
/// `∃R.⊤ ⊑ C` / `⊤ ⊑ ∀R.C`) are performed by the reasoning layers (L3/L4), not the mapping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Axiom {
    /// A general class inclusion (GCI) `sub ⊑ sup`.
    SubClassOf {
        /// The subsumed (left) class expression.
        sub: ClassExpression,
        /// The subsuming (right) class expression.
        sup: ClassExpression,
    },
    /// `owl:equivalentClass` — `left ≡ right` (binary, as `owl:equivalentClass` is encoded in
    /// RDF).
    EquivalentClasses(ClassExpression, ClassExpression),
    /// `owl:disjointWith` — `left ⊓ right ⊑ ⊥` (binary).
    DisjointClasses(ClassExpression, ClassExpression),
    /// `rdfs:subPropertyOf` — a role inclusion `sub ⊑ sup` over named object properties.
    SubObjectPropertyOf {
        /// The subsumed (left) property.
        sub: ObjectPropertyExpression,
        /// The subsuming (right) property.
        sup: ObjectPropertyExpression,
    },
    /// `rdfs:domain` — the subject domain of a property (`∃property.⊤ ⊑ domain`).
    ObjectPropertyDomain {
        /// The constrained property.
        property: ObjectPropertyExpression,
        /// The class every subject of the property must belong to.
        domain: ClassExpression,
    },
    /// `rdfs:range` — the object range of a property (`⊤ ⊑ ∀property.range`).
    ObjectPropertyRange {
        /// The constrained property.
        property: ObjectPropertyExpression,
        /// The class every object of the property must belong to.
        range: ClassExpression,
    },
    /// A ground class assertion `class(individual)`.
    ClassAssertion {
        /// The asserted class expression.
        class: ClassExpression,
        /// The dict id of the asserted individual.
        individual: Id,
    },
    /// A ground role assertion `property(source, target)`.
    ObjectPropertyAssertion {
        /// The asserting property.
        property: ObjectPropertyExpression,
        /// The dict id of the source individual.
        source: Id,
        /// The dict id of the target individual.
        target: Id,
    },
    /// `owl:TransitiveProperty` — the role is transitive (`R ∘ R ⊑ R`). OPT-IN: this variant
    /// exists only under the `dl_transitive` cargo feature (sq-zfwzq); with the
    /// feature OFF the extractor keeps refusing `owl:TransitiveProperty` fail-closed, exactly
    /// as before. The L3 tableau consumes it via the ∀₊-propagation rule, whose extended
    /// termination/soundness/completeness argument is written in the `tableau` module docs
    /// (Horrocks & Sattler 1999 — sound with subset blocking precisely because the fragment
    /// still has NO inverse roles).
    #[cfg(feature = "dl_transitive")]
    TransitiveObjectProperty {
        /// The transitive (named) object property.
        property: ObjectPropertyExpression,
    },
}

/// A structural OWL ontology restricted to the ALCH fragment — the output of the fail-closed
/// reverse RDF mapping ([`crate::extract()`]).
///
/// Purely structural: L1 attaches NO semantics. The axioms are emitted in the order the
/// backing triples were iterated, so the mapping is deterministic for a given triple slice.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ontology {
    /// The recognised ALCH axioms.
    pub axioms: Vec<Axiom>,
}

impl Ontology {
    /// An empty ontology (no axioms).
    #[must_use]
    pub fn new() -> Ontology {
        Ontology { axioms: Vec::new() }
    }

    /// The recognised axioms.
    #[must_use]
    pub fn axioms(&self) -> &[Axiom] {
        &self.axioms
    }

    /// `true` iff no axiom was recognised.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.axioms.is_empty()
    }

    /// The number of recognised axioms.
    #[must_use]
    pub fn len(&self) -> usize {
        self.axioms.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_property_expression_named_returns_id() {
        let p = ObjectPropertyExpression::ObjectProperty(42);
        assert_eq!(p.named(), 42);
    }

    #[test]
    fn class_expression_constructors_and_atomicity() {
        assert!(ClassExpression::Class(1).is_atomic());
        assert!(ClassExpression::Thing.is_atomic());
        assert!(ClassExpression::Nothing.is_atomic());
        let some = ClassExpression::some(7, ClassExpression::Class(3));
        assert!(!some.is_atomic());
        assert_eq!(
            some,
            ClassExpression::ObjectSomeValuesFrom(
                ObjectPropertyExpression::ObjectProperty(7),
                Box::new(ClassExpression::Class(3))
            )
        );
        let only = ClassExpression::only(7, ClassExpression::Thing);
        assert!(!only.is_atomic());
        assert_eq!(
            only,
            ClassExpression::ObjectAllValuesFrom(
                ObjectPropertyExpression::ObjectProperty(7),
                Box::new(ClassExpression::Thing)
            )
        );
    }

    #[test]
    fn ontology_len_empty_and_new() {
        let mut o = Ontology::new();
        assert!(o.is_empty());
        assert_eq!(o.len(), 0);
        assert_eq!(o.axioms(), &[]);
        o.axioms.push(Axiom::SubClassOf {
            sub: ClassExpression::Class(1),
            sup: ClassExpression::Thing,
        });
        assert!(!o.is_empty());
        assert_eq!(o.len(), 1);
        assert_eq!(Ontology::default(), Ontology::new());
    }
}
