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
    Axiom, AxiomRecord, ClassExpression, DataRange, EntityKind, Individual, Literal,
    ObjectProperty, Ontology,
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

/// The result of the OWL 2 DL profile check.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DlProfileReport {
    /// Every violation, in input order.
    pub violations: Vec<Violation>,
}

impl DlProfileReport {
    /// `true` when the ontology is in OWL 2 DL.
    #[must_use]
    pub fn is_dl(&self) -> bool {
        self.violations.is_empty()
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
    DlProfileReport { violations: c.out }
}

struct Checker<'a> {
    dict: &'a Dict,
    onto: &'a Ontology,
    v: Vocab,
    out: Vec<Violation>,
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
        }
    }

    fn name(&self, id: Id) -> String {
        iri_of(self.dict, id).unwrap_or_else(|| super::vocab::render_term(self.dict, id))
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
                continue;
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
        for (&id, ds) in &declared {
            for &(k, _) in ds {
                seen_kinds.entry(id).or_default().insert(k);
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
        if self.v.top_object_property != sparq_core::dict::NO_ID {
            composite.insert(ObjectProperty::Named(self.v.top_object_property));
            composite.insert(ObjectProperty::Inverse(self.v.top_object_property));
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
        let reach = |from: ObjectProperty, to: ObjectProperty| -> bool {
            let mut seen = FxHashSet::default();
            let mut work = vec![from];
            while let Some(p) = work.pop() {
                if p == to {
                    return true;
                }
                if seen.insert(p) {
                    if let Some(next) = edges.get(&p) {
                        work.extend(next.iter().copied());
                    }
                }
            }
            false
        };
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
        // The order must extend the strict part of the hierarchy, and S < R iff Inv(S) < R.
        let mut graph: FxHashMap<ObjectProperty, Vec<(ObjectProperty, Option<usize>)>> =
            FxHashMap::default();
        for (&a, bs) in &edges {
            for &b in bs {
                if a != b && !reach(b, a) {
                    graph.entry(a).or_default().push((b, None));
                }
            }
        }
        for &(s, r, ai) in &less {
            graph.entry(s).or_default().push((r, Some(ai)));
            graph.entry(s.inverse()).or_default().push((r, Some(ai)));
        }
        // A strict partial order exists iff the constraint graph has no cycle.
        let mut nodes: Vec<ObjectProperty> = graph.keys().copied().collect();
        nodes.sort();
        let mut state: FxHashMap<ObjectProperty, u8> = FxHashMap::default();
        let mut cycle_axioms: Vec<usize> = Vec::new();
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
        fn find(parent: &mut FxHashMap<Id, Id>, x: Id) -> Id {
            let p = *parent.entry(x).or_insert(x);
            if p == x {
                return x;
            }
            let r = find(parent, p);
            parent.insert(x, r);
            r
        }
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
                    parent.insert(ra, rb);
                }
            }
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
        for rec in &self.onto.axioms {
            let mut lit = |l: &Literal| {
                if let Err(e) = datatypes::parse_value(&l.lexical, &l.datatype, l.lang.as_deref()) {
                    match e {
                        datatypes::ValueError::IllTyped => found.push((
                            ViolationKind::IllTypedLiteral,
                            format!(
                                "{} is not a valid {} literal",
                                super::vocab::render_literal(l),
                                l.datatype
                            ),
                            rec.source.clone(),
                        )),
                        datatypes::ValueError::UnknownDatatype => {
                            let known = super::vocab::lookup_iri(self.dict, &l.datatype);
                            if !declared_dt.contains(&known) {
                                found.push((
                                    ViolationKind::UnknownDatatype,
                                    format!(
                                        "{} is not in the OWL 2 datatype map and is not declared",
                                        l.datatype
                                    ),
                                    rec.source.clone(),
                                ));
                            }
                        }
                        datatypes::ValueError::Unsupported => {}
                    }
                }
            };
            for_each_literal_in_axiom(&rec.axiom, &mut lit);
            let mut range = |dr: &DataRange| match dr {
                DataRange::Datatype(id) => {
                    let iri = iri_of(self.dict, *id).unwrap_or_default();
                    if !datatypes::is_datatype_map_iri(&iri) && !declared_dt.contains(id) {
                        found.push((
                            ViolationKind::UnknownDatatype,
                            format!("{iri} is not in the OWL 2 datatype map and is not declared"),
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
                            (Some(b), Some(facet)) => facet_value_problem(b, facet, value),
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
    }
}

fn facet_value_problem(b: Builtin, facet: Facet, value: &Literal) -> Option<String> {
    let parsed = datatypes::parse_value(&value.lexical, &value.datatype, value.lang.as_deref());
    let bad = |what: &str| Some(format!("the value of this facet must be {what}"));
    match facet {
        Facet::Length | Facet::MinLength | Facet::MaxLength => match parsed {
            Ok(datatypes::Value::Real(r)) if r.0.is_integer() && r.0.num >= 0 => None,
            _ => bad("a non-negative integer"),
        },
        Facet::Pattern => match parsed {
            Ok(datatypes::Value::String(_)) => None,
            _ => bad("a string"),
        },
        Facet::LangRange => match parsed {
            Ok(datatypes::Value::String(_)) => None,
            _ => bad("a string"),
        },
        Facet::MinInclusive | Facet::MaxInclusive | Facet::MinExclusive | Facet::MaxExclusive => {
            match parsed {
                Ok(v) if b.families().contains(&v.family()) => None,
                _ => bad("a value of the restricted datatype's value space"),
            }
        }
    }
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
        DataRange::OneOf(_) => {}
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
        Axiom::DataPropertyAssertion(_, _, l) | Axiom::NegativeDataPropertyAssertion(_, _, l) => {
            f(l)
        }
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
