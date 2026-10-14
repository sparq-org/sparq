//! The OWL 2 DL profile check: the typing constraints (OWL 2 Structural Specification §5.8.1),
//! the global restrictions on axioms (§11), and the datatype requirements (§4, §9.4) that an
//! ontology must meet beyond being syntactically OWL 2.
//!
//! The check is syntactic and terminates by construction. It reports every violation it
//! finds, each with the input triples it is about, so it can serve as a lint gate. A few
//! violations also bound what the reasoner may decide (see [`Violation::blocks_reasoning`]):
//! the tableau's correctness argument assumes simple roles under cardinality / self /
//! characteristics and a regular property hierarchy, so a reasoner run over an ontology that
//! breaks either returns `Unknown` instead of a verdict.

use super::datatypes::{self, builtin, Builtin, Facet};
use super::model::{
    AnnotationSubject, AnnotationValue, Axiom, AxiomRecord, ClassExpression, DataRange, EntityKind,
    Individual, Literal, ObjectProperty, Ontology,
};
use super::vocab::{iri_of, Vocab, OWL, RDF, RDFS, XSD};
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::{Dict, Id};
use std::collections::BTreeMap;

/// The kind of an OWL 2 DL violation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ViolationKind {
    /// An entity is used without a declaration (§5.8.1).
    Undeclared,
    /// An IRI is declared or used as two incompatible kinds of entity: object and data
    /// property, a property and an annotation property, or a class and a datatype (§5.8.1).
    IllegalPunning,
    /// A vocabulary IRI of `rdf:`, `rdfs:`, `owl:` or `xsd:` is used as a user entity (§2.4).
    ReservedVocabulary,
    /// A non-simple object property appears where only simple ones are allowed: a cardinality
    /// restriction, `ObjectHasSelf`, or a functional, inverse-functional, irreflexive,
    /// asymmetric or disjointness axiom (§11.1).
    NonSimpleProperty,
    /// The property hierarchy with its chains is not regular (§11.2).
    IrregularPropertyHierarchy,
    /// Object property assertions between anonymous individuals form a cycle (§11.2).
    AnonymousIndividualCycle,
    /// A literal's lexical form is outside its datatype's lexical space.
    IllTypedLiteral,
    /// A datatype is used that is neither in the OWL 2 datatype map nor declared.
    UnknownDatatype,
    /// A facet that the datatype does not allow, or a facet value of the wrong kind (§4).
    InvalidFacet,
    /// A datatype definition that redefines a built-in datatype, defines an undeclared one,
    /// or is cyclic (§9.4).
    InvalidDatatypeDefinition,
    /// `owl:topDataProperty` outside the super-property position of `SubDataPropertyOf`.
    TopDataPropertyMisuse,
    /// The extractor had to choose a reading (an undeclared predicate read as annotations).
    ExtractionNote,
    /// An n-ary axiom has fewer distinct operands than it needs (two): the structural
    /// specification reads its operands as a set, so `EquivalentClasses(A A)` has one.
    InsufficientOperands,
}

/// One OWL 2 DL violation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Violation {
    /// What is violated.
    pub kind: ViolationKind,
    /// A human-readable explanation naming the entities involved.
    pub message: String,
    /// The input triples the violation is about.
    pub source: Vec<usize>,
}

impl Violation {
    /// `true` when the violation takes the ontology outside what the reasoner's correctness
    /// argument covers (a non-simple property under a restricted construct, or an irregular
    /// property hierarchy). The reasoner returns `Unknown` for such ontologies.
    #[must_use]
    pub fn blocks_reasoning(&self) -> bool {
        matches!(
            self.kind,
            ViolationKind::NonSimpleProperty | ViolationKind::IrregularPropertyHierarchy
        )
    }
}

/// A global restriction the check did not verify for this ontology. While any remain the
/// check does not claim the ontology is in OWL 2 DL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unchecked {
    /// The restriction, e.g. `"xsd:pattern regular-expression syntax"`.
    pub rule: &'static str,
    /// The input triples it applies to (empty for ontology-wide rules).
    pub source: Vec<usize>,
}

/// The result of the OWL 2 DL profile check. Fail-closed: [`is_dl`](Self::is_dl) holds only
/// when no violation was found AND every restriction was checked.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DlProfileReport {
    /// Every violation, in input order.
    pub violations: Vec<Violation>,
    /// The restrictions that apply to this ontology but were not verified.
    pub unchecked: Vec<Unchecked>,
}

impl DlProfileReport {
    /// `true` when the ontology is shown to be in OWL 2 DL.
    #[must_use]
    pub fn is_dl(&self) -> bool {
        self.violations.is_empty() && self.unchecked.is_empty()
    }

    /// `Some(true)` in OWL 2 DL, `Some(false)` when a violation was found, `None` when no
    /// violation was found but some restriction was not checked.
    #[must_use]
    pub fn verdict(&self) -> Option<bool> {
        if !self.violations.is_empty() {
            Some(false)
        } else if self.unchecked.is_empty() {
            Some(true)
        } else {
            None
        }
    }

    /// The violations that prevent a reasoning verdict.
    pub fn blocking(&self) -> impl Iterator<Item = &Violation> {
        self.violations.iter().filter(|v| v.blocks_reasoning())
    }
}

/// Runs the OWL 2 DL profile check over an extracted ontology.
#[must_use]
pub fn check_dl(dict: &Dict, onto: &Ontology) -> DlProfileReport {
    let mut c = Checker::new(dict, onto);
    c.typing();
    c.simple_roles();
    c.regularity();
    c.anonymous_individuals();
    c.datatypes();
    c.operand_sets();
    if !onto.imports.is_empty() {
        c.unchecked(
            "the imports closure (owl:imports targets are not loaded)",
            &[],
        );
    }
    for note in &onto.notes {
        c.out.push(Violation {
            kind: ViolationKind::ExtractionNote,
            message: note.message.clone(),
            source: note.source.clone(),
        });
    }
    c.out.sort_by(|a, b| {
        a.source
            .first()
            .cmp(&b.source.first())
            .then(a.kind.cmp(&b.kind))
    });
    let mut unchecked = c.unchecked;
    unchecked.sort_by(|a, b| {
        a.source
            .first()
            .cmp(&b.source.first())
            .then(a.rule.cmp(b.rule))
    });
    unchecked.dedup();
    DlProfileReport {
        violations: c.out,
        unchecked,
    }
}

struct Checker<'a> {
    dict: &'a Dict,
    onto: &'a Ontology,
    v: Vocab,
    out: Vec<Violation>,
    unchecked: Vec<Unchecked>,
}

/// An entity use: its kind and the axiom it occurs in.
type Uses = BTreeMap<(Id, EntityKind), Vec<usize>>;

impl<'a> Checker<'a> {
    fn new(dict: &'a Dict, onto: &'a Ontology) -> Checker<'a> {
        Checker {
            dict,
            onto,
            v: Vocab::intern(dict),
            out: Vec::new(),
            unchecked: Vec::new(),
        }
    }

    fn name(&self, id: Id) -> String {
        iri_of(self.dict, id).unwrap_or_else(|| super::vocab::render_term(self.dict, id))
    }

    fn unchecked(&mut self, rule: &'static str, source: &[usize]) {
        self.unchecked.push(Unchecked {
            rule,
            source: source.to_vec(),
        });
    }

    fn report(&mut self, kind: ViolationKind, message: String, source: &[usize]) {
        self.out.push(Violation {
            kind,
            message,
            source: source.to_vec(),
        });
    }

    // ---------------------------------------------------------------------------------
    // typing constraints
    // ---------------------------------------------------------------------------------

    fn typing(&mut self) {
        let mut uses: Uses = BTreeMap::new();
        let mut declared: FxHashMap<Id, Vec<(EntityKind, usize)>> = FxHashMap::default();
        for (ai, rec) in self.onto.axioms.iter().enumerate() {
            if let Axiom::Declaration(k, id) = rec.axiom {
                declared.entry(id).or_default().push((k, ai));
            } else {
                collect_axiom(&rec.axiom, &mut |id, k| {
                    uses.entry((id, k)).or_default().push(ai);
                });
            }
        }
        // Undeclared and reserved entities.
        let mut seen_kinds: FxHashMap<Id, FxHashSet<EntityKind>> = FxHashMap::default();
        for (&(id, kind), axioms) in &uses {
            seen_kinds.entry(id).or_default().insert(kind);
            if self.is_builtin(id, kind) {
                continue;
            }
            let src = self.sources(axioms);
            // An unknown `xsd:` datatype is reported by the datatype pass.
            if self.is_reserved(id) && kind != EntityKind::Datatype {
                self.report(
                    ViolationKind::ReservedVocabulary,
                    format!(
                        "{} is reserved vocabulary used as a {}",
                        self.name(id),
                        kind_name(kind)
                    ),
                    &src,
                );
                // Not declared either (a declaration of it is a violation of its own).
            }
            let has_decl = declared
                .get(&id)
                .is_some_and(|ds| ds.iter().any(|(k, _)| *k == kind));
            // Undeclared datatypes are reported by the datatype pass (UnknownDatatype).
            if !has_decl && kind != EntityKind::NamedIndividual && kind != EntityKind::Datatype {
                self.report(
                    ViolationKind::Undeclared,
                    format!(
                        "{} is used as a {} but not declared",
                        self.name(id),
                        kind_name(kind)
                    ),
                    &src,
                );
            }
        }
        let mut decl_ids: Vec<Id> = declared.keys().copied().collect();
        decl_ids.sort_unstable();
        for id in decl_ids {
            for &(k, ai) in &declared[&id] {
                seen_kinds.entry(id).or_default().insert(k);
                // Declaring reserved vocabulary (other than a built-in entity of that kind) is
                // as illegal as using it.
                if !self.is_builtin(id, k) && self.is_reserved(id) && !uses.contains_key(&(id, k)) {
                    let src = self.sources(&[ai]);
                    self.report(
                        ViolationKind::ReservedVocabulary,
                        format!(
                            "{} is reserved vocabulary declared as a {}",
                            self.name(id),
                            kind_name(k)
                        ),
                        &src,
                    );
                }
            }
        }
        // Ontology and axiom annotations: their properties are annotation properties.
        for ann in &self.onto.annotations {
            let (id, k) = (ann.property, EntityKind::AnnotationProperty);
            seen_kinds.entry(id).or_default().insert(k);
            if self.is_builtin(id, k) {
                continue;
            }
            let has_decl = declared
                .get(&id)
                .is_some_and(|ds| ds.iter().any(|(dk, _)| *dk == k));
            if self.is_reserved(id) {
                self.out.push(Violation {
                    kind: ViolationKind::ReservedVocabulary,
                    message: format!(
                        "{} is reserved vocabulary used as an annotation property",
                        self.name(id)
                    ),
                    source: ann.source.clone(),
                });
            }
            if !has_decl {
                self.out.push(Violation {
                    kind: ViolationKind::Undeclared,
                    message: format!(
                        "{} is used as an annotation property but not declared",
                        self.name(id)
                    ),
                    source: ann.source.clone(),
                });
            }
        }
        // Illegal punning.
        let mut ids: Vec<Id> = seen_kinds.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            let ks = &seen_kinds[&id];
            let clash = [
                (EntityKind::ObjectProperty, EntityKind::DataProperty),
                (EntityKind::ObjectProperty, EntityKind::AnnotationProperty),
                (EntityKind::DataProperty, EntityKind::AnnotationProperty),
                (EntityKind::Class, EntityKind::Datatype),
            ]
            .into_iter()
            .find(|(a, b)| ks.contains(a) && ks.contains(b));
            if let Some((a, b)) = clash {
                let mut axioms: Vec<usize> = Vec::new();
                for k in [a, b] {
                    if let Some(u) = uses.get(&(id, k)) {
                        axioms.extend(u);
                    }
                    if let Some(ds) = declared.get(&id) {
                        axioms.extend(ds.iter().filter(|(dk, _)| *dk == k).map(|(_, ai)| *ai));
                    }
                }
                let src = self.sources(&axioms);
                self.report(
                    ViolationKind::IllegalPunning,
                    format!(
                        "{} is both a {} and a {}",
                        self.name(id),
                        kind_name(a),
                        kind_name(b)
                    ),
                    &src,
                );
            }
        }
        // owl:topDataProperty may only be the super-property of SubDataPropertyOf.
        let top = self.v.top_data_property;
        for rec in &self.onto.axioms {
            let misuse = match &rec.axiom {
                Axiom::SubDataPropertyOf(sub, _) => *sub == top,
                Axiom::Declaration(..) => false,
                ax => {
                    let mut hit = false;
                    collect_axiom(ax, &mut |id, k| {
                        hit |= id == top && k == EntityKind::DataProperty;
                    });
                    hit
                }
            };
            if misuse && top != sparq_core::dict::NO_ID {
                self.out.push(Violation {
                    kind: ViolationKind::TopDataPropertyMisuse,
                    message: "owl:topDataProperty may only appear as the super-property of SubDataPropertyOf".into(),
                    source: rec.source.clone(),
                });
            }
        }
    }

    /// N-ary axioms need two distinct operands.
    fn operand_sets(&mut self) {
        fn distinct<T: PartialEq>(xs: &[T]) -> usize {
            xs.iter()
                .enumerate()
                .filter(|(i, x)| !xs[..*i].contains(x))
                .count()
        }
        for rec in &self.onto.axioms {
            let n = match &rec.axiom {
                Axiom::EquivalentClasses(cs) | Axiom::DisjointClasses(cs) => distinct(cs),
                Axiom::DisjointUnion(_, cs) => distinct(cs),
                Axiom::EquivalentObjectProperties(ps) | Axiom::DisjointObjectProperties(ps) => {
                    distinct(ps)
                }
                Axiom::EquivalentDataProperties(ps) | Axiom::DisjointDataProperties(ps) => {
                    distinct(ps)
                }
                Axiom::SameIndividual(is) | Axiom::DifferentIndividuals(is) => distinct(is),
                _ => continue,
            };
            if n < 2 {
                self.out.push(Violation {
                    kind: ViolationKind::InsufficientOperands,
                    message: "the axiom needs at least two distinct operands".into(),
                    source: rec.source.clone(),
                });
            }
        }
    }

    fn sources(&self, axioms: &[usize]) -> Vec<usize> {
        let mut s: Vec<usize> = axioms
            .iter()
            .flat_map(|&ai| self.onto.axioms[ai].source.iter().copied())
            .collect();
        s.sort_unstable();
        s.dedup();
        s
    }

    fn is_builtin(&self, id: Id, kind: EntityKind) -> bool {
        let v = &self.v;
        match kind {
            EntityKind::Class => id == v.thing || id == v.nothing,
            EntityKind::ObjectProperty => {
                id == v.top_object_property || id == v.bottom_object_property
            }
            EntityKind::DataProperty => id == v.top_data_property || id == v.bottom_data_property,
            EntityKind::AnnotationProperty => v.is_builtin_annotation_property(id),
            EntityKind::Datatype => datatypes::is_datatype_map_iri(&self.name(id)),
            EntityKind::NamedIndividual => false,
        }
    }

    fn is_reserved(&self, id: Id) -> bool {
        iri_of(self.dict, id).is_some_and(|iri| {
            iri.starts_with(OWL)
                || iri.starts_with(RDF)
                || iri.starts_with(RDFS)
                || iri.starts_with(XSD)
        })
    }

    // ---------------------------------------------------------------------------------
    // simple roles (§11.1)
    // ---------------------------------------------------------------------------------

    /// The property hierarchy relation `→` (closed under inverses) and the composite set.
    fn hierarchy(
        &self,
    ) -> (
        FxHashMap<ObjectProperty, Vec<ObjectProperty>>,
        FxHashSet<ObjectProperty>,
    ) {
        let mut edges: FxHashMap<ObjectProperty, Vec<ObjectProperty>> = FxHashMap::default();
        let mut add = |a: ObjectProperty, b: ObjectProperty| {
            for (x, y) in [(a, b), (a.inverse(), b.inverse())] {
                let e = edges.entry(x).or_default();
                if !e.contains(&y) {
                    e.push(y);
                }
            }
        };
        let mut composite = FxHashSet::default();
        // §11.1: both universal object properties are composite.
        for p in [self.v.top_object_property, self.v.bottom_object_property] {
            if p != sparq_core::dict::NO_ID {
                composite.insert(ObjectProperty::Named(p));
                composite.insert(ObjectProperty::Inverse(p));
            }
        }
        for rec in &self.onto.axioms {
            match &rec.axiom {
                Axiom::SubObjectPropertyOf(chain, sup) if chain.len() == 1 => add(chain[0], *sup),
                Axiom::SubObjectPropertyOf(_, sup) => {
                    composite.insert(*sup);
                    composite.insert(sup.inverse());
                }
                Axiom::EquivalentObjectProperties(ps) => {
                    for a in ps {
                        for b in ps {
                            if a != b {
                                add(*a, *b);
                            }
                        }
                    }
                }
                Axiom::InverseObjectProperties(a, b) => {
                    add(*a, b.inverse());
                    add(b.inverse(), *a);
                }
                Axiom::SymmetricObjectProperty(p) => {
                    add(*p, p.inverse());
                }
                Axiom::TransitiveObjectProperty(p) => {
                    composite.insert(*p);
                    composite.insert(p.inverse());
                }
                _ => {}
            }
        }
        (edges, composite)
    }

    fn non_simple(&self) -> FxHashSet<ObjectProperty> {
        let (edges, composite) = self.hierarchy();
        // Everything reachable from a composite property through → is non-simple.
        let mut out: FxHashSet<ObjectProperty> = FxHashSet::default();
        let mut work: Vec<ObjectProperty> = composite.into_iter().collect();
        while let Some(p) = work.pop() {
            if out.insert(p) {
                if let Some(next) = edges.get(&p) {
                    work.extend(next.iter().copied());
                }
            }
        }
        out
    }

    fn simple_roles(&mut self) {
        let non_simple = self.non_simple();
        if non_simple.is_empty() {
            return;
        }
        let mut found: Vec<(ObjectProperty, &'static str, Vec<usize>)> = Vec::new();
        for rec in &self.onto.axioms {
            let mut check = |p: &ObjectProperty, what: &'static str| {
                if non_simple.contains(p) {
                    found.push((*p, what, rec.source.clone()));
                }
            };
            match &rec.axiom {
                Axiom::FunctionalObjectProperty(p) => check(p, "FunctionalObjectProperty"),
                Axiom::InverseFunctionalObjectProperty(p) => {
                    check(p, "InverseFunctionalObjectProperty")
                }
                Axiom::IrreflexiveObjectProperty(p) => check(p, "IrreflexiveObjectProperty"),
                Axiom::AsymmetricObjectProperty(p) => check(p, "AsymmetricObjectProperty"),
                Axiom::DisjointObjectProperties(ps) => {
                    for p in ps {
                        check(p, "DisjointObjectProperties");
                    }
                }
                ax => {
                    for_each_class_in_axiom(ax, &mut |ce| match ce {
                        ClassExpression::MinCardinality(_, p, _)
                        | ClassExpression::MaxCardinality(_, p, _)
                        | ClassExpression::ExactCardinality(_, p, _) => {
                            check(p, "an object cardinality restriction")
                        }
                        ClassExpression::HasSelf(p) => check(p, "ObjectHasSelf"),
                        _ => {}
                    });
                }
            }
        }
        for (p, what, src) in found {
            let name = self.ope_name(p);
            self.report(
                ViolationKind::NonSimpleProperty,
                format!("{name} is not simple (it is transitive, has a chain, or has such a sub-property) but is used in {what}"),
                &src,
            );
        }
    }

    fn ope_name(&self, p: ObjectProperty) -> String {
        match p {
            ObjectProperty::Named(id) => self.name(id),
            ObjectProperty::Inverse(id) => format!("ObjectInverseOf({})", self.name(id)),
        }
    }

    // ---------------------------------------------------------------------------------
    // regularity (§11.2)
    // ---------------------------------------------------------------------------------

    fn regularity(&mut self) {
        let (edges, _) = self.hierarchy();
        // The constraints `a < b` the order must satisfy, each with its source axiom.
        let mut less: Vec<(ObjectProperty, ObjectProperty, usize)> = Vec::new();
        let top = self.v.top_object_property;
        let mut has_chain = false;
        for (ai, rec) in self.onto.axioms.iter().enumerate() {
            let Axiom::SubObjectPropertyOf(chain, sup) = &rec.axiom else {
                continue;
            };
            if chain.len() < 2 || sup.named() == top {
                continue;
            }
            has_chain = true;
            let n = chain.len();
            let r = *sup;
            if n == 2 && chain[0] == r && chain[1] == r {
                continue;
            }
            let smaller: Vec<ObjectProperty> = if chain[0] == r {
                chain[1..].to_vec()
            } else if chain[n - 1] == r {
                chain[..n - 1].to_vec()
            } else {
                chain.clone()
            };
            for s in smaller {
                less.push((s, r, ai));
            }
        }
        if !has_chain {
            return;
        }
        // Mutually reachable properties are equivalent: one element of the order. Collapse
        // the hierarchy's strongly connected components, so a constraint `S < R` with `S`
        // equivalent to `R` is a violation, and a cycle through an equivalence class is found.
        let comp = components(&edges);
        let rep = |p: ObjectProperty| comp.get(&p).copied().unwrap_or(p);
        let mut cycle_axioms: Vec<usize> = Vec::new();
        // The order must extend the strict part of the hierarchy, and S < R iff Inv(S) < R.
        let mut graph: FxHashMap<ObjectProperty, Vec<(ObjectProperty, Option<usize>)>> =
            FxHashMap::default();
        for (&a, bs) in &edges {
            for &b in bs {
                if rep(a) != rep(b) {
                    graph.entry(rep(a)).or_default().push((rep(b), None));
                }
            }
        }
        for &(s, r, ai) in &less {
            for s in [s, s.inverse()] {
                if rep(s) == rep(r) {
                    cycle_axioms.push(ai);
                } else {
                    graph.entry(rep(s)).or_default().push((rep(r), Some(ai)));
                }
            }
        }
        for succ in graph.values_mut() {
            succ.sort();
            succ.dedup();
        }
        // A strict partial order exists iff the constraint graph has no cycle.
        let mut nodes: Vec<ObjectProperty> = graph.keys().copied().collect();
        nodes.sort();
        let mut state: FxHashMap<ObjectProperty, u8> = FxHashMap::default();
        for start in nodes {
            if state.get(&start).copied().unwrap_or(0) != 0 {
                continue;
            }
            // Iterative DFS with an explicit stack of (node, next-edge-index).
            let mut stack: Vec<(ObjectProperty, usize)> = vec![(start, 0)];
            let mut path_axioms: Vec<Option<usize>> = vec![None];
            state.insert(start, 1);
            while let Some((node, ei)) = stack.last().copied() {
                let succ = graph.get(&node).map(Vec::as_slice).unwrap_or(&[]);
                if ei >= succ.len() {
                    state.insert(node, 2);
                    stack.pop();
                    path_axioms.pop();
                    continue;
                }
                stack.last_mut().expect("non-empty").1 += 1;
                let (next, ax) = succ[ei];
                match state.get(&next).copied().unwrap_or(0) {
                    0 => {
                        state.insert(next, 1);
                        stack.push((next, 0));
                        path_axioms.push(ax);
                    }
                    1 => {
                        // A cycle: report the chain axioms on it.
                        let pos = stack.iter().position(|(n, _)| *n == next).unwrap_or(0);
                        cycle_axioms.extend(path_axioms[pos + 1..].iter().flatten());
                        cycle_axioms.extend(ax);
                    }
                    _ => {}
                }
            }
        }
        if !cycle_axioms.is_empty() {
            cycle_axioms.sort_unstable();
            cycle_axioms.dedup();
            let src = self.sources(&cycle_axioms);
            self.report(
                ViolationKind::IrregularPropertyHierarchy,
                "the property chains and the property hierarchy are not regular (no strict order on the properties satisfies §11.2)".into(),
                &src,
            );
        }
    }

    // ---------------------------------------------------------------------------------
    // anonymous individuals (§11.2)
    // ---------------------------------------------------------------------------------

    fn anonymous_individuals(&mut self) {
        // Union-find over anonymous individuals connected by object property assertions; an
        // edge between two already-connected individuals closes a cycle.
        let mut parent: FxHashMap<Id, Id> = FxHashMap::default();
        // Iterative, with path halving: a long chain of blank individuals must not recurse.
        fn find(parent: &mut FxHashMap<Id, Id>, mut x: Id) -> Id {
            loop {
                let p = *parent.entry(x).or_insert(x);
                if p == x {
                    return x;
                }
                let g = *parent.entry(p).or_insert(p);
                parent.insert(x, g);
                x = g;
            }
        }
        let mut size: FxHashMap<Id, usize> = FxHashMap::default();
        let mut seen_edges: FxHashSet<(Id, Id)> = FxHashSet::default();
        let mut cyclic: Vec<Vec<usize>> = Vec::new();
        for rec in &self.onto.axioms {
            if let Axiom::ObjectPropertyAssertion(
                _,
                Individual::Anonymous(a),
                Individual::Anonymous(b),
            ) = &rec.axiom
            {
                let key = ((*a).min(*b), (*a).max(*b));
                if a == b || !seen_edges.insert(key) {
                    cyclic.push(rec.source.clone());
                    continue;
                }
                let (ra, rb) = (find(&mut parent, *a), find(&mut parent, *b));
                if ra == rb {
                    cyclic.push(rec.source.clone());
                } else {
                    // Union by size keeps the trees shallow.
                    let (sa, sb) = (
                        size.get(&ra).copied().unwrap_or(1),
                        size.get(&rb).copied().unwrap_or(1),
                    );
                    let (small, big) = if sa < sb { (ra, rb) } else { (rb, ra) };
                    parent.insert(small, big);
                    size.insert(big, sa + sb);
                }
            }
        }
        // Fail-closed: the forest condition above is the only anonymous-individual
        // restriction checked; any other use of an anonymous individual leaves the rest of
        // §11.2 unverified.
        const RULE: &str = "anonymous-individual restrictions beyond the forest condition (§11.2)";
        let anon = |i: &Individual| matches!(i, Individual::Anonymous(_));
        let mut other_uses: Vec<Vec<usize>> = Vec::new();
        for rec in &self.onto.axioms {
            let mut hit = match &rec.axiom {
                Axiom::ObjectPropertyAssertion(_, a, b) => anon(a) != anon(b),
                Axiom::NegativeObjectPropertyAssertion(_, a, b) => anon(a) || anon(b),
                Axiom::NegativeDataPropertyAssertion(_, a, _) => anon(a),
                Axiom::SameIndividual(is) | Axiom::DifferentIndividuals(is) => is.iter().any(anon),
                Axiom::AnnotationAssertion(_, subject, value) => {
                    matches!(subject, AnnotationSubject::Anonymous(_))
                        || matches!(value, AnnotationValue::Anonymous(_))
                }
                _ => false,
            };
            for_each_class_in_axiom(&rec.axiom, &mut |ce| match ce {
                ClassExpression::OneOf(is) => hit |= is.iter().any(anon),
                ClassExpression::HasValue(_, i) => hit |= anon(i),
                _ => {}
            });
            if hit {
                other_uses.push(rec.source.clone());
            }
        }
        for ann in &self.onto.annotations {
            if matches!(ann.value, AnnotationValue::Anonymous(_)) {
                other_uses.push(ann.source.clone());
            }
        }
        for src in other_uses {
            self.unchecked(RULE, &src);
        }
        for src in cyclic {
            self.report(
                ViolationKind::AnonymousIndividualCycle,
                "object property assertions between anonymous individuals form a cycle".into(),
                &src,
            );
        }
    }

    // ---------------------------------------------------------------------------------
    // datatypes
    // ---------------------------------------------------------------------------------

    fn datatypes(&mut self) {
        let mut declared_dt: FxHashSet<Id> = FxHashSet::default();
        let mut definitions: FxHashMap<Id, Vec<usize>> = FxHashMap::default();
        for (ai, rec) in self.onto.axioms.iter().enumerate() {
            match &rec.axiom {
                Axiom::Declaration(EntityKind::Datatype, id) => {
                    declared_dt.insert(*id);
                }
                Axiom::DatatypeDefinition(id, _) => definitions.entry(*id).or_default().push(ai),
                _ => {}
            }
        }
        let mut found: Vec<(ViolationKind, String, Vec<usize>)> = Vec::new();
        let mut not_checked: Vec<(&'static str, Vec<usize>)> = Vec::new();
        let xml_literal = format!("{RDF}XMLLiteral");
        // Every literal of the ontology, with the triples it came from.
        let mut literals: Vec<(Literal, &[usize])> = Vec::new();
        for rec in &self.onto.axioms {
            for_each_literal_in_axiom(&rec.axiom, &mut |l| literals.push((l.clone(), &rec.source)));
        }
        for ann in &self.onto.annotations {
            if let AnnotationValue::Literal(l) = &ann.value {
                literals.push((l.clone(), &ann.source));
            }
        }
        for (l, source) in &literals {
            if l.datatype == xml_literal {
                not_checked.push(("rdf:XMLLiteral content", source.to_vec()));
                continue;
            }
            match datatypes::parse_value(&l.lexical, &l.datatype, l.lang.as_deref()) {
                Ok(_) | Err(datatypes::ValueError::Unsupported) => {}
                Err(datatypes::ValueError::IllTyped) => found.push((
                    ViolationKind::IllTypedLiteral,
                    format!(
                        "{} is not a valid {} literal",
                        super::vocab::render_literal(l),
                        l.datatype
                    ),
                    source.to_vec(),
                )),
                // §5.7 / §9.4: only datatype-map datatypes have lexical forms; a declared or
                // defined datatype has an empty lexical space.
                Err(datatypes::ValueError::UnknownDatatype) => found.push((
                    ViolationKind::UnknownDatatype,
                    format!(
                        "{} is not in the OWL 2 datatype map, so no literal can use it",
                        l.datatype
                    ),
                    source.to_vec(),
                )),
            }
        }
        for rec in &self.onto.axioms {
            let mut range = |dr: &DataRange| match dr {
                DataRange::Datatype(id) => {
                    // §11.2: a datatype outside the map needs exactly one definition.
                    let iri = iri_of(self.dict, *id).unwrap_or_default();
                    let defined = definitions.get(id).is_some_and(|d| d.len() == 1);
                    if !datatypes::is_datatype_map_iri(&iri) && !defined {
                        found.push((
                            ViolationKind::UnknownDatatype,
                            format!(
                                "{iri} is neither in the OWL 2 datatype map nor defined by exactly one DatatypeDefinition"
                            ),
                            rec.source.clone(),
                        ));
                    }
                }
                DataRange::Restriction(id, facets) => {
                    let iri = iri_of(self.dict, *id).unwrap_or_default();
                    let b = builtin(&iri);
                    for (f, value) in facets {
                        let firi = iri_of(self.dict, *f).unwrap_or_default();
                        let problem = match (b, datatypes::facet(&firi)) {
                            (None, _) => Some(format!(
                                "facets can only restrict datatypes of the OWL 2 datatype map, not {iri}"
                            )),
                            (_, None) => Some(format!("{firi} is not an OWL 2 facet")),
                            (Some(b), Some(facet)) if !b.allowed_facets().contains(&facet) => {
                                Some(format!("{firi} does not apply to {iri}"))
                            }
                            (Some(b), Some(facet)) => match facet_value_problem(b, facet, value) {
                                FacetCheck::Ok => None,
                                FacetCheck::Bad(m) => Some(m),
                                FacetCheck::Unchecked(rule) => {
                                    not_checked.push((rule, rec.source.clone()));
                                    None
                                }
                            },
                        };
                        if let Some(msg) = problem {
                            found.push((ViolationKind::InvalidFacet, msg, rec.source.clone()));
                        }
                    }
                }
                _ => {}
            };
            for_each_range_in_axiom(&rec.axiom, &mut range);
        }
        // Datatype definitions.
        let mut ids: Vec<Id> = definitions.keys().copied().collect();
        ids.sort_unstable();
        for id in &ids {
            let src = self.sources(&definitions[id]);
            let iri = iri_of(self.dict, *id).unwrap_or_default();
            if datatypes::is_datatype_map_iri(&iri) {
                found.push((
                    ViolationKind::InvalidDatatypeDefinition,
                    format!("{iri} is a built-in datatype and cannot be redefined"),
                    src,
                ));
            } else if definitions[id].len() > 1 {
                found.push((
                    ViolationKind::InvalidDatatypeDefinition,
                    format!("{iri} has more than one DatatypeDefinition"),
                    src,
                ));
            } else if !declared_dt.contains(id) {
                found.push((
                    ViolationKind::InvalidDatatypeDefinition,
                    format!("{iri} is defined but not declared as a datatype"),
                    src,
                ));
            }
        }
        // Cycles among definitions.
        let deps = |id: Id| -> Vec<Id> {
            let mut out = Vec::new();
            for &ai in definitions.get(&id).map(Vec::as_slice).unwrap_or(&[]) {
                if let Axiom::DatatypeDefinition(_, dr) = &self.onto.axioms[ai].axiom {
                    collect_range_datatypes(dr, &mut out);
                }
            }
            out
        };
        for &start in &ids {
            let mut seen = FxHashSet::default();
            let mut work = deps(start);
            let mut cyclic = false;
            while let Some(d) = work.pop() {
                if d == start {
                    cyclic = true;
                    break;
                }
                if seen.insert(d) {
                    work.extend(deps(d));
                }
            }
            if cyclic {
                let src = self.sources(&definitions[&start]);
                found.push((
                    ViolationKind::InvalidDatatypeDefinition,
                    format!("the definition of {} is cyclic", self.name(start)),
                    src,
                ));
            }
        }
        for (k, m, s) in found {
            self.report(k, m, &s);
        }
        for (rule, s) in not_checked {
            self.unchecked(rule, &s);
        }
    }
}

enum FacetCheck {
    Ok,
    Bad(String),
    Unchecked(&'static str),
}

fn facet_value_problem(b: Builtin, facet: Facet, value: &Literal) -> FacetCheck {
    let parsed = datatypes::parse_value(&value.lexical, &value.datatype, value.lang.as_deref());
    let bad = |what: &str| FacetCheck::Bad(format!("the value of this facet must be {what}"));
    match facet {
        Facet::Length | Facet::MinLength | Facet::MaxLength => match parsed {
            Ok(datatypes::Value::Real(r)) if r.0.is_integer() && r.0.num >= 0 => FacetCheck::Ok,
            _ => bad("a non-negative integer"),
        },
        Facet::Pattern => match parsed {
            Ok(datatypes::Value::String(_)) => {
                FacetCheck::Unchecked("xsd:pattern regular-expression syntax")
            }
            _ => bad("a string"),
        },
        Facet::LangRange => match parsed {
            Ok(datatypes::Value::String(r)) if is_language_range(&r) => FacetCheck::Ok,
            _ => bad("an RFC 4647 language range"),
        },
        Facet::MinInclusive | Facet::MaxInclusive | Facet::MinExclusive | Facet::MaxExclusive => {
            match parsed {
                // In the restricted datatype's own value space: always allowed.
                Ok(v) if datatypes::in_value_space(&v, b) => FacetCheck::Ok,
                // owl:real and owl:rational take bounds from their whole family.
                Ok(v)
                    if matches!(b, Builtin::OwlReal | Builtin::OwlRational)
                        && b.families().contains(&v.family()) =>
                {
                    FacetCheck::Ok
                }
                // Same family, outside the value space (an integer bounded by 0.5): the
                // facet space of that combination is not checked.
                Ok(v) if b.families().contains(&v.family()) => FacetCheck::Unchecked(
                    "the facet space of a bound outside the restricted datatype's value space",
                ),
                _ => bad("a value of the restricted datatype's value space"),
            }
        }
    }
}

/// RFC 4647 §2.2 extended language range: `*` or `(alpha{1,8}|*)(-(alphanum{1,8}|*))*`.
fn is_language_range(s: &str) -> bool {
    let mut parts = s.split('-');
    let first_ok = parts.next().is_some_and(|p| {
        p == "*" || (!p.is_empty() && p.len() <= 8 && p.bytes().all(|b| b.is_ascii_alphabetic()))
    });
    first_ok
        && parts.all(|p| {
            p == "*"
                || (!p.is_empty() && p.len() <= 8 && p.bytes().all(|b| b.is_ascii_alphanumeric()))
        })
}

fn kind_name(k: EntityKind) -> &'static str {
    match k {
        EntityKind::Class => "class",
        EntityKind::Datatype => "datatype",
        EntityKind::ObjectProperty => "object property",
        EntityKind::DataProperty => "data property",
        EntityKind::AnnotationProperty => "annotation property",
        EntityKind::NamedIndividual => "named individual",
    }
}

// -------------------------------------------------------------------------------------------
// structural walkers (shared with the reasoner)
// -------------------------------------------------------------------------------------------

fn collect_ope(p: &ObjectProperty, f: &mut impl FnMut(Id, EntityKind)) {
    f(p.named(), EntityKind::ObjectProperty);
}

/// The strongly connected components of `edges`, each node mapped to its component's least
/// member (Kosaraju, iterative).
fn components(
    edges: &FxHashMap<ObjectProperty, Vec<ObjectProperty>>,
) -> FxHashMap<ObjectProperty, ObjectProperty> {
    let mut nodes: Vec<ObjectProperty> = edges
        .iter()
        .flat_map(|(a, bs)| std::iter::once(*a).chain(bs.iter().copied()))
        .collect();
    nodes.sort();
    nodes.dedup();
    let mut reverse: FxHashMap<ObjectProperty, Vec<ObjectProperty>> = FxHashMap::default();
    for (&a, bs) in edges {
        for &b in bs {
            reverse.entry(b).or_default().push(a);
        }
    }
    // Pass 1: finishing order.
    let mut seen: FxHashSet<ObjectProperty> = FxHashSet::default();
    let mut order: Vec<ObjectProperty> = Vec::with_capacity(nodes.len());
    for &start in &nodes {
        if !seen.insert(start) {
            continue;
        }
        let mut stack: Vec<(ObjectProperty, usize)> = vec![(start, 0)];
        while let Some((n, i)) = stack.last().copied() {
            let succ = edges.get(&n).map(Vec::as_slice).unwrap_or(&[]);
            if i < succ.len() {
                stack.last_mut().expect("non-empty").1 += 1;
                if seen.insert(succ[i]) {
                    stack.push((succ[i], 0));
                }
            } else {
                order.push(n);
                stack.pop();
            }
        }
    }
    // Pass 2: components on the reverse graph, in reverse finishing order.
    let mut comp: FxHashMap<ObjectProperty, ObjectProperty> = FxHashMap::default();
    for &start in order.iter().rev() {
        if comp.contains_key(&start) {
            continue;
        }
        let mut members = vec![start];
        comp.insert(start, start);
        let mut i = 0;
        while i < members.len() {
            for &p in reverse.get(&members[i]).map(Vec::as_slice).unwrap_or(&[]) {
                if let std::collections::hash_map::Entry::Vacant(e) = comp.entry(p) {
                    e.insert(start);
                    members.push(p);
                }
            }
            i += 1;
        }
        let least = *members.iter().min().expect("non-empty");
        for m in members {
            comp.insert(m, least);
        }
    }
    comp
}

fn collect_ind(i: &Individual, f: &mut impl FnMut(Id, EntityKind)) {
    if let Individual::Named(id) = i {
        f(*id, EntityKind::NamedIndividual);
    }
}

fn collect_range(dr: &DataRange, f: &mut impl FnMut(Id, EntityKind)) {
    match dr {
        DataRange::Datatype(id) | DataRange::Restriction(id, _) => f(*id, EntityKind::Datatype),
        DataRange::IntersectionOf(rs) | DataRange::UnionOf(rs) => {
            for r in rs {
                collect_range(r, f);
            }
        }
        DataRange::ComplementOf(r) => collect_range(r, f),
        DataRange::OneOf(_) | DataRange::TopLiteral => {}
    }
}

fn collect_range_datatypes(dr: &DataRange, out: &mut Vec<Id>) {
    collect_range(dr, &mut |id, _| out.push(id));
}

fn collect_class(ce: &ClassExpression, f: &mut impl FnMut(Id, EntityKind)) {
    use ClassExpression as C;
    match ce {
        C::Class(id) => f(*id, EntityKind::Class),
        C::Thing | C::Nothing => {}
        C::IntersectionOf(cs) | C::UnionOf(cs) => {
            for c in cs {
                collect_class(c, f);
            }
        }
        C::ComplementOf(c) => collect_class(c, f),
        C::OneOf(is) => {
            for i in is {
                collect_ind(i, f);
            }
        }
        C::SomeValuesFrom(p, c) | C::AllValuesFrom(p, c) => {
            collect_ope(p, f);
            collect_class(c, f);
        }
        C::HasValue(p, i) => {
            collect_ope(p, f);
            collect_ind(i, f);
        }
        C::HasSelf(p) => collect_ope(p, f),
        C::MinCardinality(_, p, c) | C::MaxCardinality(_, p, c) | C::ExactCardinality(_, p, c) => {
            collect_ope(p, f);
            collect_class(c, f);
        }
        C::DataSomeValuesFrom(d, r)
        | C::DataAllValuesFrom(d, r)
        | C::DataMinCardinality(_, d, r)
        | C::DataMaxCardinality(_, d, r)
        | C::DataExactCardinality(_, d, r) => {
            f(*d, EntityKind::DataProperty);
            collect_range(r, f);
        }
        C::DataHasValue(d, _) => f(*d, EntityKind::DataProperty),
    }
}

/// Calls `f` with every entity (and the kind it is used as) in a logical axiom.
pub(crate) fn collect_axiom(ax: &Axiom, f: &mut impl FnMut(Id, EntityKind)) {
    use Axiom as A;
    match ax {
        A::Declaration(..) => {}
        A::SubClassOf(a, b) => {
            collect_class(a, f);
            collect_class(b, f);
        }
        A::EquivalentClasses(cs) | A::DisjointClasses(cs) => {
            for c in cs {
                collect_class(c, f);
            }
        }
        A::DisjointUnion(c, cs) => {
            f(*c, EntityKind::Class);
            for c in cs {
                collect_class(c, f);
            }
        }
        A::SubObjectPropertyOf(chain, sup) => {
            for p in chain {
                collect_ope(p, f);
            }
            collect_ope(sup, f);
        }
        A::EquivalentObjectProperties(ps) | A::DisjointObjectProperties(ps) => {
            for p in ps {
                collect_ope(p, f);
            }
        }
        A::InverseObjectProperties(a, b) => {
            collect_ope(a, f);
            collect_ope(b, f);
        }
        A::ObjectPropertyDomain(p, c) | A::ObjectPropertyRange(p, c) => {
            collect_ope(p, f);
            collect_class(c, f);
        }
        A::FunctionalObjectProperty(p)
        | A::InverseFunctionalObjectProperty(p)
        | A::ReflexiveObjectProperty(p)
        | A::IrreflexiveObjectProperty(p)
        | A::SymmetricObjectProperty(p)
        | A::AsymmetricObjectProperty(p)
        | A::TransitiveObjectProperty(p) => collect_ope(p, f),
        A::SubDataPropertyOf(a, b) => {
            f(*a, EntityKind::DataProperty);
            f(*b, EntityKind::DataProperty);
        }
        A::EquivalentDataProperties(ds) | A::DisjointDataProperties(ds) => {
            for d in ds {
                f(*d, EntityKind::DataProperty);
            }
        }
        A::DataPropertyDomain(d, c) => {
            f(*d, EntityKind::DataProperty);
            collect_class(c, f);
        }
        A::DataPropertyRange(d, r) => {
            f(*d, EntityKind::DataProperty);
            collect_range(r, f);
        }
        A::FunctionalDataProperty(d) => f(*d, EntityKind::DataProperty),
        A::DatatypeDefinition(dt, r) => {
            f(*dt, EntityKind::Datatype);
            collect_range(r, f);
        }
        A::HasKey(c, ps, ds) => {
            collect_class(c, f);
            for p in ps {
                collect_ope(p, f);
            }
            for d in ds {
                f(*d, EntityKind::DataProperty);
            }
        }
        A::SameIndividual(is) | A::DifferentIndividuals(is) => {
            for i in is {
                collect_ind(i, f);
            }
        }
        A::ClassAssertion(c, i) => {
            collect_class(c, f);
            collect_ind(i, f);
        }
        A::ObjectPropertyAssertion(p, a, b) | A::NegativeObjectPropertyAssertion(p, a, b) => {
            collect_ope(p, f);
            collect_ind(a, f);
            collect_ind(b, f);
        }
        A::DataPropertyAssertion(d, a, _) | A::NegativeDataPropertyAssertion(d, a, _) => {
            f(*d, EntityKind::DataProperty);
            collect_ind(a, f);
        }
        // Annotation subjects, values and domains / ranges are plain IRIs, not entities.
        A::AnnotationAssertion(p, _, _)
        | A::AnnotationPropertyDomain(p, _)
        | A::AnnotationPropertyRange(p, _) => f(*p, EntityKind::AnnotationProperty),
        A::SubAnnotationPropertyOf(p, q) => {
            f(*p, EntityKind::AnnotationProperty);
            f(*q, EntityKind::AnnotationProperty);
        }
    }
}

fn walk_class(ce: &ClassExpression, f: &mut impl FnMut(&ClassExpression)) {
    use ClassExpression as C;
    f(ce);
    match ce {
        C::IntersectionOf(cs) | C::UnionOf(cs) => {
            for c in cs {
                walk_class(c, f);
            }
        }
        C::ComplementOf(c)
        | C::SomeValuesFrom(_, c)
        | C::AllValuesFrom(_, c)
        | C::MinCardinality(_, _, c)
        | C::MaxCardinality(_, _, c)
        | C::ExactCardinality(_, _, c) => walk_class(c, f),
        _ => {}
    }
}

/// Calls `f` with every class expression (recursively) in an axiom.
pub(crate) fn for_each_class_in_axiom(ax: &Axiom, f: &mut impl FnMut(&ClassExpression)) {
    use Axiom as A;
    match ax {
        A::SubClassOf(a, b) => {
            walk_class(a, f);
            walk_class(b, f);
        }
        A::EquivalentClasses(cs) | A::DisjointClasses(cs) | A::DisjointUnion(_, cs) => {
            for c in cs {
                walk_class(c, f);
            }
        }
        A::ObjectPropertyDomain(_, c)
        | A::ObjectPropertyRange(_, c)
        | A::DataPropertyDomain(_, c)
        | A::HasKey(c, _, _)
        | A::ClassAssertion(c, _) => walk_class(c, f),
        _ => {}
    }
}

fn walk_range(dr: &DataRange, f: &mut impl FnMut(&DataRange)) {
    f(dr);
    match dr {
        DataRange::IntersectionOf(rs) | DataRange::UnionOf(rs) => {
            for r in rs {
                walk_range(r, f);
            }
        }
        DataRange::ComplementOf(r) => walk_range(r, f),
        _ => {}
    }
}

/// Calls `f` with every data range (recursively) in an axiom.
pub(crate) fn for_each_range_in_axiom(ax: &Axiom, f: &mut impl FnMut(&DataRange)) {
    let mut in_class = |ce: &ClassExpression| match ce {
        ClassExpression::DataSomeValuesFrom(_, r)
        | ClassExpression::DataAllValuesFrom(_, r)
        | ClassExpression::DataMinCardinality(_, _, r)
        | ClassExpression::DataMaxCardinality(_, _, r)
        | ClassExpression::DataExactCardinality(_, _, r) => walk_range(r, f),
        _ => {}
    };
    for_each_class_in_axiom(ax, &mut in_class);
    match ax {
        Axiom::DataPropertyRange(_, r) | Axiom::DatatypeDefinition(_, r) => walk_range(r, f),
        _ => {}
    }
}

/// Calls `f` with every literal in an axiom (assertions, has-value, one-of, facet values).
pub(crate) fn for_each_literal_in_axiom(ax: &Axiom, f: &mut impl FnMut(&Literal)) {
    match ax {
        Axiom::DataPropertyAssertion(_, _, l)
        | Axiom::NegativeDataPropertyAssertion(_, _, l)
        | Axiom::AnnotationAssertion(_, _, AnnotationValue::Literal(l)) => f(l),
        _ => {}
    }
    for_each_class_in_axiom(ax, &mut |ce| {
        if let ClassExpression::DataHasValue(_, l) = ce {
            f(l);
        }
    });
    for_each_range_in_axiom(ax, &mut |dr| match dr {
        DataRange::OneOf(ls) => {
            for l in ls {
                f(l);
            }
        }
        DataRange::Restriction(_, facets) => {
            for (_, l) in facets {
                f(l);
            }
        }
        _ => {}
    });
}

/// The source triples of a set of axiom records (sorted, de-duplicated).
#[allow(dead_code)]
pub(crate) fn sources_of(records: &[&AxiomRecord]) -> Vec<usize> {
    let mut s: Vec<usize> = records
        .iter()
        .flat_map(|r| r.source.iter().copied())
        .collect();
    s.sort_unstable();
    s.dedup();
    s
}
