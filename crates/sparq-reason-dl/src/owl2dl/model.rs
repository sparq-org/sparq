//! The full OWL 2 structural model (OWL 2 Structural Specification §5–§9), as read from RDF
//! by [`super::extract()`].
//!
//! Unlike the ALCH model in [`crate::model`], this model covers the whole OWL 2 DL syntax:
//! inverse properties, cardinality, nominals, self restrictions, data properties and data
//! ranges, keys, property chains, every property characteristic, and the full ABox
//! (equality, inequality, negative assertions, data assertions). It is purely structural:
//! no semantics are attached here. Entities are identified by their `sparq_core` dict id,
//! literals by their parsed lexical form, datatype IRI and language tag.
//!
//! Every axiom carries its provenance: the indices of the input triples it was read from
//! (the axiom triple and every triple of the blank-node structure it consumed). That is
//! what lets the reasoner report a justification, or the offending construct, as triples.

use sparq_core::dict::Id;

/// An individual: a named individual (an IRI) or an anonymous individual (a blank node in an
/// assertion position). Both are identified by their dict id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Individual {
    /// A named individual (IRI).
    Named(Id),
    /// An anonymous individual (blank node).
    Anonymous(Id),
}

impl Individual {
    /// The dict id of the individual.
    #[must_use]
    pub fn id(&self) -> Id {
        match self {
            Individual::Named(id) | Individual::Anonymous(id) => *id,
        }
    }
}

/// An RDF literal: lexical form, datatype IRI (plain literals carry `xsd:string`, language-tagged
/// ones `rdf:langString`), and an optional language tag.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Literal {
    /// The lexical form.
    pub lexical: String,
    /// The full datatype IRI.
    pub datatype: String,
    /// The language tag, if any (lower-cased as stored by the parser).
    pub lang: Option<String>,
}

/// An object property expression: a named object property or the inverse of one
/// (OWL 2 Structural Specification §6.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ObjectProperty {
    /// A named object property.
    Named(Id),
    /// `ObjectInverseOf(P)` for a named object property `P`.
    Inverse(Id),
}

impl ObjectProperty {
    /// The underlying named property.
    #[must_use]
    pub fn named(&self) -> Id {
        match self {
            ObjectProperty::Named(id) | ObjectProperty::Inverse(id) => *id,
        }
    }

    /// The inverse expression (`Inv(Inv(P)) = P`).
    #[must_use]
    pub fn inverse(&self) -> ObjectProperty {
        match self {
            ObjectProperty::Named(id) => ObjectProperty::Inverse(*id),
            ObjectProperty::Inverse(id) => ObjectProperty::Named(*id),
        }
    }

    /// `true` for `ObjectInverseOf(P)`.
    #[must_use]
    pub fn is_inverse(&self) -> bool {
        matches!(self, ObjectProperty::Inverse(_))
    }
}

/// A data range (OWL 2 Structural Specification §7).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DataRange {
    /// `rdfs:Literal`, the whole data domain: the filler of an unqualified data cardinality,
    /// or `rdfs:Literal` named in the input.
    TopLiteral,
    /// A named datatype (a datatype-map datatype, or a user datatype).
    Datatype(Id),
    /// `DataIntersectionOf`.
    IntersectionOf(Vec<DataRange>),
    /// `DataUnionOf`.
    UnionOf(Vec<DataRange>),
    /// `DataComplementOf`.
    ComplementOf(Box<DataRange>),
    /// `DataOneOf`.
    OneOf(Vec<Literal>),
    /// `DatatypeRestriction(datatype, (facet, value)*)`; facets are dict ids of the facet IRIs.
    Restriction(Id, Vec<(Id, Literal)>),
}

/// A class expression (OWL 2 Structural Specification §8). Unqualified cardinality
/// restrictions carry `owl:Thing` (object) or `rdfs:Literal` (data) as their filler, as the
/// specification defines them.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ClassExpression {
    /// A named class.
    Class(Id),
    /// `owl:Thing`.
    Thing,
    /// `owl:Nothing`.
    Nothing,
    /// `ObjectIntersectionOf`.
    IntersectionOf(Vec<ClassExpression>),
    /// `ObjectUnionOf`.
    UnionOf(Vec<ClassExpression>),
    /// `ObjectComplementOf`.
    ComplementOf(Box<ClassExpression>),
    /// `ObjectOneOf`.
    OneOf(Vec<Individual>),
    /// `ObjectSomeValuesFrom`.
    SomeValuesFrom(ObjectProperty, Box<ClassExpression>),
    /// `ObjectAllValuesFrom`.
    AllValuesFrom(ObjectProperty, Box<ClassExpression>),
    /// `ObjectHasValue`.
    HasValue(ObjectProperty, Individual),
    /// `ObjectHasSelf`.
    HasSelf(ObjectProperty),
    /// `ObjectMinCardinality(n P C)`.
    MinCardinality(u32, ObjectProperty, Box<ClassExpression>),
    /// `ObjectMaxCardinality(n P C)`.
    MaxCardinality(u32, ObjectProperty, Box<ClassExpression>),
    /// `ObjectExactCardinality(n P C)`.
    ExactCardinality(u32, ObjectProperty, Box<ClassExpression>),
    /// `DataSomeValuesFrom(D DR)` (single data property; n-ary data restrictions are refused).
    DataSomeValuesFrom(Id, DataRange),
    /// `DataAllValuesFrom(D DR)`.
    DataAllValuesFrom(Id, DataRange),
    /// `DataHasValue(D lt)`.
    DataHasValue(Id, Literal),
    /// `DataMinCardinality(n D DR)`.
    DataMinCardinality(u32, Id, DataRange),
    /// `DataMaxCardinality(n D DR)`.
    DataMaxCardinality(u32, Id, DataRange),
    /// `DataExactCardinality(n D DR)`.
    DataExactCardinality(u32, Id, DataRange),
}

/// The kind of an entity declaration (OWL 2 Structural Specification §5.8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EntityKind {
    /// `Declaration(Class(..))`.
    Class,
    /// `Declaration(Datatype(..))`.
    Datatype,
    /// `Declaration(ObjectProperty(..))`.
    ObjectProperty,
    /// `Declaration(DataProperty(..))`.
    DataProperty,
    /// `Declaration(AnnotationProperty(..))`.
    AnnotationProperty,
    /// `Declaration(NamedIndividual(..))`.
    NamedIndividual,
}

/// An axiom of the full OWL 2 structural model.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Axiom {
    /// `Declaration(kind(iri))`.
    Declaration(EntityKind, Id),
    /// `SubClassOf(sub sup)`.
    SubClassOf(ClassExpression, ClassExpression),
    /// `EquivalentClasses(C1 … Cn)`.
    EquivalentClasses(Vec<ClassExpression>),
    /// `DisjointClasses(C1 … Cn)`.
    DisjointClasses(Vec<ClassExpression>),
    /// `DisjointUnion(C C1 … Cn)`.
    DisjointUnion(Id, Vec<ClassExpression>),
    /// `SubObjectPropertyOf(P Q)` (`chain.len() == 1`) or
    /// `SubObjectPropertyOf(ObjectPropertyChain(P1 … Pn) Q)` (`chain.len() >= 2`).
    SubObjectPropertyOf(Vec<ObjectProperty>, ObjectProperty),
    /// `EquivalentObjectProperties(P1 … Pn)`.
    EquivalentObjectProperties(Vec<ObjectProperty>),
    /// `DisjointObjectProperties(P1 … Pn)`.
    DisjointObjectProperties(Vec<ObjectProperty>),
    /// `InverseObjectProperties(P Q)`.
    InverseObjectProperties(ObjectProperty, ObjectProperty),
    /// `ObjectPropertyDomain(P C)`.
    ObjectPropertyDomain(ObjectProperty, ClassExpression),
    /// `ObjectPropertyRange(P C)`.
    ObjectPropertyRange(ObjectProperty, ClassExpression),
    /// `FunctionalObjectProperty(P)`.
    FunctionalObjectProperty(ObjectProperty),
    /// `InverseFunctionalObjectProperty(P)`.
    InverseFunctionalObjectProperty(ObjectProperty),
    /// `ReflexiveObjectProperty(P)`.
    ReflexiveObjectProperty(ObjectProperty),
    /// `IrreflexiveObjectProperty(P)`.
    IrreflexiveObjectProperty(ObjectProperty),
    /// `SymmetricObjectProperty(P)`.
    SymmetricObjectProperty(ObjectProperty),
    /// `AsymmetricObjectProperty(P)`.
    AsymmetricObjectProperty(ObjectProperty),
    /// `TransitiveObjectProperty(P)`.
    TransitiveObjectProperty(ObjectProperty),
    /// `SubDataPropertyOf(D E)`.
    SubDataPropertyOf(Id, Id),
    /// `EquivalentDataProperties(D1 … Dn)`.
    EquivalentDataProperties(Vec<Id>),
    /// `DisjointDataProperties(D1 … Dn)`.
    DisjointDataProperties(Vec<Id>),
    /// `DataPropertyDomain(D C)`.
    DataPropertyDomain(Id, ClassExpression),
    /// `DataPropertyRange(D DR)`.
    DataPropertyRange(Id, DataRange),
    /// `FunctionalDataProperty(D)`.
    FunctionalDataProperty(Id),
    /// `DatatypeDefinition(DT DR)`.
    DatatypeDefinition(Id, DataRange),
    /// `HasKey(C (P1 … Pm) (D1 … Dn))`.
    HasKey(ClassExpression, Vec<ObjectProperty>, Vec<Id>),
    /// `SameIndividual(a1 … an)`.
    SameIndividual(Vec<Individual>),
    /// `DifferentIndividuals(a1 … an)`.
    DifferentIndividuals(Vec<Individual>),
    /// `ClassAssertion(C a)`.
    ClassAssertion(ClassExpression, Individual),
    /// `ObjectPropertyAssertion(P a b)`.
    ObjectPropertyAssertion(ObjectProperty, Individual, Individual),
    /// `NegativeObjectPropertyAssertion(P a b)`.
    NegativeObjectPropertyAssertion(ObjectProperty, Individual, Individual),
    /// `DataPropertyAssertion(D a lt)`.
    DataPropertyAssertion(Id, Individual, Literal),
    /// `NegativeDataPropertyAssertion(D a lt)`.
    NegativeDataPropertyAssertion(Id, Individual, Literal),
    /// `AnnotationAssertion(AP s v)`. No logical meaning; kept so the profile check sees it.
    AnnotationAssertion(Id, AnnotationSubject, AnnotationValue),
    /// `SubAnnotationPropertyOf(AP1 AP2)`. No logical meaning.
    SubAnnotationPropertyOf(Id, Id),
    /// `AnnotationPropertyDomain(AP U)`. No logical meaning.
    AnnotationPropertyDomain(Id, Id),
    /// `AnnotationPropertyRange(AP U)`. No logical meaning.
    AnnotationPropertyRange(Id, Id),
}

/// The subject of an annotation assertion (Structural Specification §10.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AnnotationSubject {
    /// An IRI.
    Iri(Id),
    /// An anonymous individual.
    Anonymous(Id),
}

/// The value of an annotation (Structural Specification §10.1).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AnnotationValue {
    /// An IRI.
    Iri(Id),
    /// An anonymous individual.
    Anonymous(Id),
    /// A literal.
    Literal(Literal),
}

/// What an [`Annotation`] annotates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AnnotationTarget {
    /// The ontology (an annotation on the `owl:Ontology` header node).
    Ontology(Id),
    /// The axiom whose main triple has this input index (an `owl:Axiom` reification).
    Axiom(usize),
    /// Another annotation, by index into [`Ontology::annotations`] (an `owl:Annotation`
    /// reification).
    Annotation(usize),
}

/// An ontology or axiom annotation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Annotation {
    /// What is annotated.
    pub target: AnnotationTarget,
    /// The annotation property.
    pub property: Id,
    /// The value.
    pub value: AnnotationValue,
    /// The input triples it was read from.
    pub source: Vec<usize>,
}

impl Axiom {
    /// `true` for the logical axioms the reasoner reads (everything but declarations and
    /// annotation axioms).
    #[must_use]
    pub fn is_logical(&self) -> bool {
        !matches!(
            self,
            Axiom::Declaration(..)
                | Axiom::AnnotationAssertion(..)
                | Axiom::SubAnnotationPropertyOf(..)
                | Axiom::AnnotationPropertyDomain(..)
                | Axiom::AnnotationPropertyRange(..)
        )
    }

    /// `true` for ABox (assertional) axioms.
    #[must_use]
    pub fn is_assertion(&self) -> bool {
        matches!(
            self,
            Axiom::SameIndividual(_)
                | Axiom::DifferentIndividuals(_)
                | Axiom::ClassAssertion(..)
                | Axiom::ObjectPropertyAssertion(..)
                | Axiom::NegativeObjectPropertyAssertion(..)
                | Axiom::DataPropertyAssertion(..)
                | Axiom::NegativeDataPropertyAssertion(..)
        )
    }
}

/// An axiom with the input triples it was read from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AxiomRecord {
    /// The axiom.
    pub axiom: Axiom,
    /// Indices into the input triple slice of every triple the axiom was read from, sorted
    /// and de-duplicated.
    pub source: Vec<usize>,
}

/// A note the extractor attaches to an otherwise accepted graph: a reading it had to choose
/// that the OWL 2 DL profile check reports as a violation (for example an undeclared
/// property read as an annotation property).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtractNote {
    /// What was assumed.
    pub message: String,
    /// The triples the note is about.
    pub source: Vec<usize>,
}

/// A full OWL 2 ontology read from RDF.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ontology {
    /// Every axiom, in a deterministic order (the order of the first triple of each axiom).
    pub axioms: Vec<AxiomRecord>,
    /// The ontology IRI (`None` for an anonymous ontology or a graph without a header).
    pub iri: Option<Id>,
    /// The version IRI, if the header gives one.
    pub version_iri: Option<Id>,
    /// The input triples of the ontology header (typing and version IRI).
    pub header_source: Vec<usize>,
    /// Parts of the graph the model cannot represent although they are valid (e.g. an
    /// `owl:deprecated` annotation when the dictionary has no id for `owl:deprecated`). The
    /// profile check reports each as unchecked.
    pub unrepresented: Vec<ExtractNote>,
    /// IRIs of `owl:imports` targets. The extractor does not dereference them.
    pub imports: Vec<Id>,
    /// Ontology and axiom annotations.
    pub annotations: Vec<Annotation>,
    /// Readings the extractor chose that the profile check reports.
    pub notes: Vec<ExtractNote>,
}

impl Ontology {
    /// The logical (non-declaration) axioms.
    pub fn logical_axioms(&self) -> impl Iterator<Item = &AxiomRecord> {
        self.axioms.iter().filter(|a| a.axiom.is_logical())
    }

    /// Whether axiom `ai` carries an annotation (an `owl:Axiom` reification of one of its
    /// triples).
    pub fn is_annotated(&self, ai: usize) -> bool {
        let source = &self.axioms[ai].source;
        self.annotations
            .iter()
            .any(|a| matches!(a.target, AnnotationTarget::Axiom(t) if source.contains(&t)))
    }
}

/// Every entity, individual and datatype id of a model value, mapped by `f`: the same
/// structure over another dictionary (to compare ontologies read into different ones).
pub trait MapIds: Sized {
    #[must_use]
    fn map_ids(&self, f: &mut impl FnMut(Id) -> Id) -> Self;
}

impl MapIds for Id {
    fn map_ids(&self, f: &mut impl FnMut(Id) -> Id) -> Id {
        f(*self)
    }
}

impl<T: MapIds> MapIds for Vec<T> {
    fn map_ids(&self, f: &mut impl FnMut(Id) -> Id) -> Vec<T> {
        self.iter().map(|x| x.map_ids(f)).collect()
    }
}

impl<T: MapIds> MapIds for Box<T> {
    fn map_ids(&self, f: &mut impl FnMut(Id) -> Id) -> Box<T> {
        Box::new((**self).map_ids(f))
    }
}

impl MapIds for Literal {
    fn map_ids(&self, _: &mut impl FnMut(Id) -> Id) -> Literal {
        self.clone()
    }
}

impl<A: MapIds, B: MapIds> MapIds for (A, B) {
    fn map_ids(&self, f: &mut impl FnMut(Id) -> Id) -> (A, B) {
        (self.0.map_ids(f), self.1.map_ids(f))
    }
}

impl MapIds for Individual {
    fn map_ids(&self, f: &mut impl FnMut(Id) -> Id) -> Individual {
        match self {
            Individual::Named(i) => Individual::Named(f(*i)),
            Individual::Anonymous(i) => Individual::Anonymous(f(*i)),
        }
    }
}

impl MapIds for ObjectProperty {
    fn map_ids(&self, f: &mut impl FnMut(Id) -> Id) -> ObjectProperty {
        match self {
            ObjectProperty::Named(i) => ObjectProperty::Named(f(*i)),
            ObjectProperty::Inverse(i) => ObjectProperty::Inverse(f(*i)),
        }
    }
}

impl MapIds for DataRange {
    fn map_ids(&self, f: &mut impl FnMut(Id) -> Id) -> DataRange {
        use DataRange as D;
        match self {
            D::TopLiteral => D::TopLiteral,
            D::Datatype(i) => D::Datatype(f(*i)),
            D::IntersectionOf(rs) => D::IntersectionOf(rs.map_ids(f)),
            D::UnionOf(rs) => D::UnionOf(rs.map_ids(f)),
            D::ComplementOf(r) => D::ComplementOf(r.map_ids(f)),
            D::OneOf(ls) => D::OneOf(ls.clone()),
            D::Restriction(i, fs) => D::Restriction(f(*i), fs.map_ids(f)),
        }
    }
}

impl MapIds for ClassExpression {
    fn map_ids(&self, f: &mut impl FnMut(Id) -> Id) -> ClassExpression {
        use ClassExpression as C;
        match self {
            C::Class(i) => C::Class(f(*i)),
            C::Thing => C::Thing,
            C::Nothing => C::Nothing,
            C::IntersectionOf(cs) => C::IntersectionOf(cs.map_ids(f)),
            C::UnionOf(cs) => C::UnionOf(cs.map_ids(f)),
            C::ComplementOf(c) => C::ComplementOf(c.map_ids(f)),
            C::OneOf(is) => C::OneOf(is.map_ids(f)),
            C::SomeValuesFrom(p, c) => C::SomeValuesFrom(p.map_ids(f), c.map_ids(f)),
            C::AllValuesFrom(p, c) => C::AllValuesFrom(p.map_ids(f), c.map_ids(f)),
            C::HasValue(p, i) => C::HasValue(p.map_ids(f), i.map_ids(f)),
            C::HasSelf(p) => C::HasSelf(p.map_ids(f)),
            C::MinCardinality(n, p, c) => C::MinCardinality(*n, p.map_ids(f), c.map_ids(f)),
            C::MaxCardinality(n, p, c) => C::MaxCardinality(*n, p.map_ids(f), c.map_ids(f)),
            C::ExactCardinality(n, p, c) => C::ExactCardinality(*n, p.map_ids(f), c.map_ids(f)),
            C::DataSomeValuesFrom(d, r) => C::DataSomeValuesFrom(f(*d), r.map_ids(f)),
            C::DataAllValuesFrom(d, r) => C::DataAllValuesFrom(f(*d), r.map_ids(f)),
            C::DataHasValue(d, l) => C::DataHasValue(f(*d), l.clone()),
            C::DataMinCardinality(n, d, r) => C::DataMinCardinality(*n, f(*d), r.map_ids(f)),
            C::DataMaxCardinality(n, d, r) => C::DataMaxCardinality(*n, f(*d), r.map_ids(f)),
            C::DataExactCardinality(n, d, r) => C::DataExactCardinality(*n, f(*d), r.map_ids(f)),
        }
    }
}

impl MapIds for AnnotationSubject {
    fn map_ids(&self, f: &mut impl FnMut(Id) -> Id) -> AnnotationSubject {
        match self {
            AnnotationSubject::Iri(i) => AnnotationSubject::Iri(f(*i)),
            AnnotationSubject::Anonymous(i) => AnnotationSubject::Anonymous(f(*i)),
        }
    }
}

impl MapIds for AnnotationValue {
    fn map_ids(&self, f: &mut impl FnMut(Id) -> Id) -> AnnotationValue {
        match self {
            AnnotationValue::Iri(i) => AnnotationValue::Iri(f(*i)),
            AnnotationValue::Anonymous(i) => AnnotationValue::Anonymous(f(*i)),
            AnnotationValue::Literal(l) => AnnotationValue::Literal(l.clone()),
        }
    }
}

impl MapIds for Axiom {
    fn map_ids(&self, f: &mut impl FnMut(Id) -> Id) -> Axiom {
        use Axiom as A;
        match self {
            A::Declaration(k, i) => A::Declaration(*k, f(*i)),
            A::SubClassOf(a, b) => A::SubClassOf(a.map_ids(f), b.map_ids(f)),
            A::EquivalentClasses(cs) => A::EquivalentClasses(cs.map_ids(f)),
            A::DisjointClasses(cs) => A::DisjointClasses(cs.map_ids(f)),
            A::DisjointUnion(i, cs) => A::DisjointUnion(f(*i), cs.map_ids(f)),
            A::SubObjectPropertyOf(w, p) => A::SubObjectPropertyOf(w.map_ids(f), p.map_ids(f)),
            A::EquivalentObjectProperties(ps) => A::EquivalentObjectProperties(ps.map_ids(f)),
            A::DisjointObjectProperties(ps) => A::DisjointObjectProperties(ps.map_ids(f)),
            A::InverseObjectProperties(p, q) => {
                A::InverseObjectProperties(p.map_ids(f), q.map_ids(f))
            }
            A::ObjectPropertyDomain(p, c) => A::ObjectPropertyDomain(p.map_ids(f), c.map_ids(f)),
            A::ObjectPropertyRange(p, c) => A::ObjectPropertyRange(p.map_ids(f), c.map_ids(f)),
            A::FunctionalObjectProperty(p) => A::FunctionalObjectProperty(p.map_ids(f)),
            A::InverseFunctionalObjectProperty(p) => {
                A::InverseFunctionalObjectProperty(p.map_ids(f))
            }
            A::ReflexiveObjectProperty(p) => A::ReflexiveObjectProperty(p.map_ids(f)),
            A::IrreflexiveObjectProperty(p) => A::IrreflexiveObjectProperty(p.map_ids(f)),
            A::SymmetricObjectProperty(p) => A::SymmetricObjectProperty(p.map_ids(f)),
            A::AsymmetricObjectProperty(p) => A::AsymmetricObjectProperty(p.map_ids(f)),
            A::TransitiveObjectProperty(p) => A::TransitiveObjectProperty(p.map_ids(f)),
            A::SubDataPropertyOf(d, e) => A::SubDataPropertyOf(f(*d), f(*e)),
            A::EquivalentDataProperties(ds) => A::EquivalentDataProperties(ds.map_ids(f)),
            A::DisjointDataProperties(ds) => A::DisjointDataProperties(ds.map_ids(f)),
            A::DataPropertyDomain(d, c) => A::DataPropertyDomain(f(*d), c.map_ids(f)),
            A::DataPropertyRange(d, r) => A::DataPropertyRange(f(*d), r.map_ids(f)),
            A::FunctionalDataProperty(d) => A::FunctionalDataProperty(f(*d)),
            A::DatatypeDefinition(i, r) => A::DatatypeDefinition(f(*i), r.map_ids(f)),
            A::HasKey(c, ps, ds) => A::HasKey(c.map_ids(f), ps.map_ids(f), ds.map_ids(f)),
            A::SameIndividual(is) => A::SameIndividual(is.map_ids(f)),
            A::DifferentIndividuals(is) => A::DifferentIndividuals(is.map_ids(f)),
            A::ClassAssertion(c, i) => A::ClassAssertion(c.map_ids(f), i.map_ids(f)),
            A::ObjectPropertyAssertion(p, a, b) => {
                A::ObjectPropertyAssertion(p.map_ids(f), a.map_ids(f), b.map_ids(f))
            }
            A::NegativeObjectPropertyAssertion(p, a, b) => {
                A::NegativeObjectPropertyAssertion(p.map_ids(f), a.map_ids(f), b.map_ids(f))
            }
            A::DataPropertyAssertion(d, a, l) => {
                A::DataPropertyAssertion(f(*d), a.map_ids(f), l.clone())
            }
            A::NegativeDataPropertyAssertion(d, a, l) => {
                A::NegativeDataPropertyAssertion(f(*d), a.map_ids(f), l.clone())
            }
            A::AnnotationAssertion(p, s, v) => {
                A::AnnotationAssertion(f(*p), s.map_ids(f), v.map_ids(f))
            }
            A::SubAnnotationPropertyOf(p, q) => A::SubAnnotationPropertyOf(f(*p), f(*q)),
            A::AnnotationPropertyDomain(p, i) => A::AnnotationPropertyDomain(f(*p), f(*i)),
            A::AnnotationPropertyRange(p, i) => A::AnnotationPropertyRange(f(*p), f(*i)),
        }
    }
}
