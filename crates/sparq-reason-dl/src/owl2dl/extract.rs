//! The fail-closed reverse RDF mapping for the whole of OWL 2 (W3C *OWL 2 Web Ontology
//! Language Mapping to RDF Graphs*, §3).
//!
//! Reads an RDF graph `(Dict, [[Id; 3]])` into the full structural model of
//! [`super::model`], or refuses it with an [`ExtractError`] that names the triple it could not
//! read. The contract is the same as the ALCH extractor's ([`crate::extract()`]): the graph is
//! understood in full or refused. It is enforced by **consumption tracking** rather than by
//! case-by-case guards: every triple that a mapping rule reads is marked consumed, and after
//! all axioms are read any triple left unconsumed is refused. A triple that no rule matches
//! (an orphan list cell, a restriction nobody references, a stray triple on a structural
//! blank node) therefore can never be silently dropped.
//!
//! # Typing
//!
//! The mapping needs to know whether a property is an object, data or annotation property
//! (§3.1, the declarations). It reads the declarations first, then infers the kind of an
//! undeclared property from an unambiguous logical use (an `owl:inverseOf`, a restriction
//! whose filler is a class or a datatype, a range, a literal or IRI object in an assertion).
//! A property declared as two kinds is refused, as is an undeclared property whose kind no use
//! settles. An undeclared predicate used only in assertions with no logical use is read as an
//! annotation property and reported in [`super::model::Ontology::notes`], since the OWL 2 DL
//! profile check flags it as undeclared.
//!
//! # Scope
//!
//! Everything in the OWL 2 structural specification is read except: n-ary data restrictions
//! (`owl:onProperties`), RDF 1.2 triple terms, and `owl:imports` targets (recorded, never
//! dereferenced). Those are refused or reported, never guessed.

use super::model::{
    Annotation, AnnotationSubject, AnnotationTarget, AnnotationValue, Axiom, AxiomRecord,
    ClassExpression, DataRange, EntityKind, ExtractNote, Individual, Literal, ObjectProperty,
    Ontology,
};
use super::vocab::{
    iri_of, is_blank, is_iri, is_literal, is_triple_term, literal_of, render_triple, Vocab, OWL,
    RDF, RDFS, XSD,
};
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::{Dict, Id, NO_ID};

/// Nesting depth past which a class expression / data range is refused (guards the
/// recursive decoders against adversarially deep graphs).
const MAX_DEPTH: usize = 256;

/// Largest cardinality the mapping accepts. Larger values are refused as unsupported: the
/// tableau would have to create that many successors.
pub const MAX_CARDINALITY: u32 = 1_000_000;

/// A decoded expression with its provenance, and its size and height.
type Cached<T> = ((T, Vec<usize>), usize, usize);

/// The most model nodes and provenance entries extraction builds. A blank node shared by many
/// axioms is copied into each, so without a bound a small graph could expand without limit.
pub const MAX_EXPANSION: usize = 8_000_000;

/// Why the mapping refused a graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtractErrorKind {
    /// A triple no OWL 2 mapping rule reads.
    Unparsed,
    /// A structure that is recognisably OWL but ill-formed: a broken list, a restriction with
    /// two fillers, a cardinality that is not a non-negative integer, a literal where an
    /// individual is required, and so on.
    Malformed,
    /// A property whose kind (object, data or annotation) cannot be settled: declared as two
    /// kinds, or undeclared with no use that decides it.
    Ambiguous,
    /// Valid RDF that this mapping deliberately refuses (n-ary data restrictions, RDF 1.2
    /// triple terms, cardinalities above [`MAX_CARDINALITY`]).
    Unsupported,
}

/// A fail-closed refusal: the kind, a diagnostic, and the input triples it is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtractError {
    /// The kind of refusal.
    pub kind: ExtractErrorKind,
    /// A human-readable diagnostic (renders the offending triple when there is one).
    pub detail: String,
    /// Indices of the input triples the refusal is about (may be empty).
    pub triples: Vec<usize>,
}

impl std::fmt::Display for ExtractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self.kind {
            ExtractErrorKind::Unparsed => "unparsed triple",
            ExtractErrorKind::Malformed => "malformed OWL structure",
            ExtractErrorKind::Ambiguous => "ambiguous property kind",
            ExtractErrorKind::Unsupported => "unsupported construct",
        };
        write!(f, "{}: {}", kind, self.detail)
    }
}

impl std::error::Error for ExtractError {}

type Result<T> = std::result::Result<T, ExtractError>;

/// Reads the OWL 2 ontology encoded in `triples` (see the module docs for the contract).
///
/// # Errors
/// Returns an [`ExtractError`] naming the first triple or structure the mapping could not
/// read; nothing is returned for a graph understood only in part.
pub fn extract(dict: &Dict, triples: &[[Id; 3]]) -> Result<Ontology> {
    let mut x = Extractor::new(dict, triples)?;
    x.declarations()?;
    x.infer_property_kinds()?;
    x.axioms()?;
    x.finish()
}

/// The property kinds the mapping distinguishes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PropKind {
    Object,
    Data,
    Annotation,
}

struct Extractor<'a> {
    dict: &'a Dict,
    v: Vocab,
    /// Distinct triples, in first-occurrence order.
    triples: Vec<[Id; 3]>,
    /// For each distinct triple, the index of its first occurrence in the input.
    original: Vec<usize>,
    consumed: Vec<bool>,
    /// subject → distinct-triple indices, in order.
    by_subject: FxHashMap<Id, Vec<usize>>,
    /// Declared or inferred entity kinds.
    classes: FxHashSet<Id>,
    datatypes: FxHashSet<Id>,
    individuals: FxHashSet<Id>,
    prop_kinds: FxHashMap<Id, Vec<PropKind>>,
    /// Blank nodes that carry class-expression, data-range, list or other structural
    /// predicates: the axiom pass leaves their triples to the decoders.
    structural: FxHashSet<Id>,
    /// The predicates that only occur inside expressions (marking a blank subject structural).
    structural_preds: FxHashSet<Id>,
    /// The `_:f xsd:facet value` nodes of `owl:withRestrictions` lists.
    facet_nodes: FxHashSet<Id>,
    /// Decoded blank-node expressions with their provenance, size and height, so a node shared
    /// by several axioms is decoded once and its copies are charged to [`MAX_EXPANSION`].
    class_cache: FxHashMap<Id, Cached<ClassExpression>>,
    range_cache: FxHashMap<Id, Cached<DataRange>>,
    /// Model nodes and provenance entries built so far.
    expanded: usize,
    visiting: FxHashSet<Id>,
    out: Ontology,
}

impl<'a> Extractor<'a> {
    fn new(dict: &'a Dict, input: &[[Id; 3]]) -> Result<Extractor<'a>> {
        let v = Vocab::intern(dict);
        let mut seen: FxHashMap<[Id; 3], usize> = FxHashMap::default();
        let mut triples = Vec::new();
        let mut original = Vec::new();
        for (i, t) in input.iter().enumerate() {
            if t.iter().any(|&id| is_triple_term(dict, id)) {
                return Err(ExtractError {
                    kind: ExtractErrorKind::Unsupported,
                    detail: format!(
                        "RDF 1.2 triple terms have no OWL 2 mapping: {}",
                        render_triple(dict, *t)
                    ),
                    triples: vec![i],
                });
            }
            if seen.insert(*t, triples.len()).is_none() {
                triples.push(*t);
                original.push(i);
            }
        }
        let mut by_subject: FxHashMap<Id, Vec<usize>> = FxHashMap::default();
        for (i, t) in triples.iter().enumerate() {
            by_subject.entry(t[0]).or_default().push(i);
        }
        let n = triples.len();
        let mut x = Extractor {
            dict,
            v,
            triples,
            original,
            consumed: vec![false; n],
            by_subject,
            classes: FxHashSet::default(),
            datatypes: FxHashSet::default(),
            individuals: FxHashSet::default(),
            prop_kinds: FxHashMap::default(),
            structural: FxHashSet::default(),
            structural_preds: FxHashSet::default(),
            facet_nodes: FxHashSet::default(),
            class_cache: FxHashMap::default(),
            range_cache: FxHashMap::default(),
            expanded: 0,
            visiting: FxHashSet::default(),
            out: Ontology::default(),
        };
        x.mark_structural();
        Ok(x)
    }

    // ---------------------------------------------------------------------------------
    // helpers
    // ---------------------------------------------------------------------------------

    fn err(&self, kind: ExtractErrorKind, detail: impl Into<String>, at: &[usize]) -> ExtractError {
        let mut detail = detail.into();
        if let Some(&i) = at.first() {
            detail = format!(
                "{} (at {})",
                detail,
                render_triple(self.dict, self.triples[i])
            );
        }
        ExtractError {
            kind,
            detail,
            triples: at.iter().map(|&i| self.original[i]).collect(),
        }
    }

    /// Distinct-triple indices with subject `s` and predicate `p`.
    fn sp(&self, s: Id, p: Id) -> Vec<usize> {
        if p == NO_ID {
            return Vec::new();
        }
        self.by_subject
            .get(&s)
            .map(|ts| {
                ts.iter()
                    .copied()
                    .filter(|&i| self.triples[i][1] == p)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The single object of `s p ?`, if any; refuses two different objects.
    fn sp_one(&self, s: Id, p: Id) -> Result<Option<(Id, usize)>> {
        let ts = self.sp(s, p);
        match ts.as_slice() {
            [] => Ok(None),
            [i] => Ok(Some((self.triples[*i][2], *i))),
            [i, ..] => Err(self.err(
                ExtractErrorKind::Malformed,
                "a structural predicate has two values on the same node",
                &[*i, ts[1]],
            )),
        }
    }

    fn has_type(&self, s: Id, ty: Id) -> Option<usize> {
        self.sp(s, self.v.ty)
            .into_iter()
            .find(|&i| ty != NO_ID && self.triples[i][2] == ty)
    }

    fn consume(&mut self, i: usize) {
        self.consumed[i] = true;
    }

    fn src(&self, local: &[usize]) -> Vec<usize> {
        let mut s: Vec<usize> = local.iter().map(|&i| self.original[i]).collect();
        s.sort_unstable();
        s.dedup();
        s
    }

    fn push(&mut self, axiom: Axiom, mut source: Vec<usize>) {
        source.sort_unstable();
        source.dedup();
        self.out.axioms.push(AxiomRecord { axiom, source });
    }

    fn kinds(&self, p: Id) -> &[PropKind] {
        self.prop_kinds.get(&p).map(Vec::as_slice).unwrap_or(&[])
    }

    fn add_kind(&mut self, p: Id, k: PropKind) {
        let ks = self.prop_kinds.entry(p).or_default();
        if !ks.contains(&k) {
            ks.push(k);
        }
    }

    /// The settled kind of `p`, refusing a property declared or used as two kinds.
    fn kind_of(&self, p: Id, at: usize) -> Result<Option<PropKind>> {
        match self.kinds(p) {
            [] => Ok(None),
            [k] => Ok(Some(*k)),
            _ => Err(self.err(
                ExtractErrorKind::Ambiguous,
                format!(
                    "{} is used as more than one kind of property (object / data / annotation)",
                    iri_of(self.dict, p).unwrap_or_default()
                ),
                &[at],
            )),
        }
    }

    fn is_builtin_datatype(&self, id: Id) -> bool {
        iri_of(self.dict, id).is_some_and(|iri| super::datatypes::is_datatype_map_iri(&iri))
    }

    fn is_known_datatype(&self, id: Id) -> bool {
        self.datatypes.contains(&id) || self.is_builtin_datatype(id)
    }

    // ---------------------------------------------------------------------------------
    // pass 0: structural blank nodes
    // ---------------------------------------------------------------------------------

    fn mark_structural(&mut self) {
        let v = &self.v;
        let structural_preds: FxHashSet<Id> = [
            v.first,
            v.rest,
            v.intersection_of,
            v.union_of,
            v.complement_of,
            v.one_of,
            v.on_property,
            v.on_properties,
            v.some_values_from,
            v.all_values_from,
            v.has_value,
            v.has_self,
            v.min_cardinality,
            v.max_cardinality,
            v.cardinality,
            v.min_qualified_cardinality,
            v.max_qualified_cardinality,
            v.qualified_cardinality,
            v.on_class,
            v.on_data_range,
            v.on_datatype,
            v.with_restrictions,
            v.datatype_complement_of,
            v.inverse_of,
        ]
        .into_iter()
        .filter(|&p| p != NO_ID)
        .collect();
        self.structural_preds = structural_preds.clone();
        let mut structural = FxHashSet::default();
        for t in &self.triples {
            if structural_preds.contains(&t[1]) && is_blank(self.dict, t[0]) {
                structural.insert(t[0]);
            }
            // `_:x a owl:Restriction` / `_:x a rdfs:Datatype` on a blank node.
            if t[1] == v.ty
                && (t[2] == v.restriction || t[2] == v.rdfs_datatype)
                && is_blank(self.dict, t[0])
            {
                structural.insert(t[0]);
            }
        }
        // Facet nodes: the members of every `owl:withRestrictions` list.
        let mut facet_lists: Vec<Id> = self
            .triples
            .iter()
            .filter(|t| t[1] == v.with_restrictions)
            .map(|t| t[2])
            .collect();
        let mut seen = FxHashSet::default();
        let mut facet_nodes = FxHashSet::default();
        while let Some(cell) = facet_lists.pop() {
            if !seen.insert(cell) {
                continue;
            }
            for &i in self.by_subject.get(&cell).map(Vec::as_slice).unwrap_or(&[]) {
                let t = self.triples[i];
                if t[1] == v.first && is_blank(self.dict, t[2]) {
                    structural.insert(t[2]);
                    facet_nodes.insert(t[2]);
                }
                if t[1] == v.rest {
                    facet_lists.push(t[2]);
                }
            }
        }
        // `owl:inverseOf` on a blank subject is an inverse property expression; on an IRI it
        // is an axiom, handled in the axiom pass.
        self.structural = structural;
        self.facet_nodes = facet_nodes;
    }

    // ---------------------------------------------------------------------------------
    // pass 1: declarations (Mapping §3.1, Tables 4 and 5)
    // ---------------------------------------------------------------------------------

    fn declarations(&mut self) -> Result<()> {
        let v = self.v.clone();
        // Mapping Table 6: a characteristic only object properties have also declares one.
        let mut implied: Vec<(Id, usize)> = Vec::new();
        for i in 0..self.triples.len() {
            let [s, p, o] = self.triples[i];
            if p != v.ty || !is_iri(self.dict, s) {
                continue;
            }
            let kind = if o == v.owl_class || o == v.rdfs_class || o == v.deprecated_class {
                Some(EntityKind::Class)
            } else if o == v.rdfs_datatype {
                Some(EntityKind::Datatype)
            } else if o == v.object_property {
                Some(EntityKind::ObjectProperty)
            } else if o == v.datatype_property {
                Some(EntityKind::DataProperty)
            } else if o == v.annotation_property || o == v.ontology_property {
                Some(EntityKind::AnnotationProperty)
            } else if o == v.named_individual {
                Some(EntityKind::NamedIndividual)
            } else {
                None
            };
            let Some(kind) = kind else {
                // Characteristics that only object properties can have (Table 5).
                if [
                    v.inverse_functional,
                    v.transitive,
                    v.symmetric,
                    v.asymmetric,
                    v.reflexive,
                    v.irreflexive,
                ]
                .contains(&o)
                    && o != NO_ID
                {
                    self.add_kind(s, PropKind::Object);
                    implied.push((s, i));
                }
                continue;
            };
            match kind {
                EntityKind::Class => {
                    self.classes.insert(s);
                }
                EntityKind::Datatype => {
                    self.datatypes.insert(s);
                }
                EntityKind::ObjectProperty => self.add_kind(s, PropKind::Object),
                EntityKind::DataProperty => self.add_kind(s, PropKind::Data),
                EntityKind::AnnotationProperty => self.add_kind(s, PropKind::Annotation),
                EntityKind::NamedIndividual => {
                    self.individuals.insert(s);
                }
            }
            self.consume(i);
            let source = self.src(&[i]);
            self.push(Axiom::Declaration(kind, s), source);
        }
        let mut declared_op: FxHashSet<Id> = self
            .out
            .axioms
            .iter()
            .filter_map(|r| match r.axiom {
                Axiom::Declaration(EntityKind::ObjectProperty, p) => Some(p),
                _ => None,
            })
            .collect();
        for (p, i) in implied {
            if declared_op.insert(p) {
                // Not consumed: the typing triple is also read as the characteristic axiom.
                let source = self.src(&[i]);
                self.push(Axiom::Declaration(EntityKind::ObjectProperty, p), source);
            }
        }
        // The intrinsic kinds of the built-in properties.
        for (p, k) in [
            (v.top_object_property, PropKind::Object),
            (v.bottom_object_property, PropKind::Object),
            (v.top_data_property, PropKind::Data),
            (v.bottom_data_property, PropKind::Data),
        ] {
            if p != NO_ID {
                self.add_kind(p, k);
            }
        }
        for p in [
            v.rdfs_label,
            v.rdfs_comment,
            v.rdfs_see_also,
            v.rdfs_is_defined_by,
            v.version_info,
            v.owl_deprecated,
            v.prior_version,
            v.backward_compatible_with,
            v.incompatible_with,
        ] {
            if p != NO_ID {
                self.add_kind(p, PropKind::Annotation);
            }
        }
        Ok(())
    }

    // ---------------------------------------------------------------------------------
    // pass 2: property-kind inference for undeclared properties
    // ---------------------------------------------------------------------------------

    /// Whether `node` reads as a data range in a filler position (a known datatype, or a
    /// blank node with a data-range shape).
    fn looks_like_data_range(&self, node: Id) -> bool {
        if is_iri(self.dict, node) {
            return self.is_known_datatype(node);
        }
        if !is_blank(self.dict, node) {
            return false;
        }
        let v = &self.v;
        if self.has_type(node, v.rdfs_datatype).is_some()
            || !self.sp(node, v.on_datatype).is_empty()
            || !self.sp(node, v.datatype_complement_of).is_empty()
        {
            return true;
        }
        // `owl:oneOf` over literals.
        if let Ok(Some((list, _))) = self.sp_one(node, v.one_of) {
            if let Ok((items, _)) = self.peek_list(list) {
                return !items.is_empty() && items.iter().all(|&m| is_literal(self.dict, m));
            }
        }
        false
    }

    /// Whether `node` reads as a class expression in a filler position.
    fn looks_like_class(&self, node: Id) -> bool {
        if is_iri(self.dict, node) {
            return self.classes.contains(&node) || node == self.v.thing || node == self.v.nothing;
        }
        if !is_blank(self.dict, node) {
            return false;
        }
        let v = &self.v;
        self.has_type(node, v.restriction).is_some()
            || self.has_type(node, v.owl_class).is_some()
            || !self.sp(node, v.complement_of).is_empty()
            || self
                .sp_one(node, v.one_of)
                .ok()
                .flatten()
                .and_then(|(list, _)| self.peek_list(list).ok())
                .is_some_and(|(items, _)| {
                    !items.is_empty() && items.iter().all(|&m| !is_literal(self.dict, m))
                })
    }

    fn infer_property_kinds(&mut self) -> Result<()> {
        let v = self.v.clone();
        // Iterate to a fixpoint: a kind settled by one use can settle another (a
        // sub-property of an object property is an object property).
        loop {
            let mut changed = false;
            for i in 0..self.triples.len() {
                let [s, p, o] = self.triples[i];
                let mut learn = |x: &mut Self, prop: Id, k: PropKind| {
                    if prop != NO_ID && is_iri(x.dict, prop) && x.kinds(prop).is_empty() {
                        x.add_kind(prop, k);
                        changed = true;
                    }
                };
                if p == v.inverse_of || p == v.property_chain_axiom {
                    learn(self, s, PropKind::Object);
                    if p == v.inverse_of {
                        learn(self, o, PropKind::Object);
                    }
                    if p == v.property_chain_axiom {
                        if let Ok((items, _)) = self.peek_list(o) {
                            for m in items {
                                learn(self, m, PropKind::Object);
                            }
                        }
                    }
                } else if p == v.on_property {
                    let fillers: Vec<Id> = [v.some_values_from, v.all_values_from]
                        .iter()
                        .flat_map(|&fp| self.sp(s, fp))
                        .map(|j| self.triples[j][2])
                        .collect();
                    if !self.sp(s, v.on_class).is_empty() || !self.sp(s, v.has_self).is_empty() {
                        learn(self, o, PropKind::Object);
                    } else if !self.sp(s, v.on_data_range).is_empty() {
                        learn(self, o, PropKind::Data);
                    }
                    for f in fillers {
                        if self.looks_like_data_range(f) {
                            learn(self, o, PropKind::Data);
                        } else if self.looks_like_class(f) {
                            learn(self, o, PropKind::Object);
                        }
                    }
                    for j in self.sp(s, v.has_value) {
                        if is_literal(self.dict, self.triples[j][2]) {
                            learn(self, o, PropKind::Data);
                        } else {
                            learn(self, o, PropKind::Object);
                        }
                    }
                } else if p == v.range && is_iri(self.dict, s) {
                    if self.looks_like_data_range(o) {
                        learn(self, s, PropKind::Data);
                    } else if self.looks_like_class(o) {
                        learn(self, s, PropKind::Object);
                    }
                } else if p == v.sub_property_of
                    || p == v.equivalent_property
                    || p == v.property_disjoint_with
                {
                    if let [k] = self.kinds(o) {
                        let k = *k;
                        learn(self, s, k);
                    }
                    if let [k] = self.kinds(s) {
                        let k = *k;
                        learn(self, o, k);
                    }
                }
            }
            if !changed {
                break;
            }
        }
        // Undeclared predicates used in assertions: a literal object makes a data property,
        // an IRI / blank object an object property — but only when the predicate has some
        // logical use; a predicate used ONLY in assertions is read as an annotation property
        // (OWL 2 DL requires its declaration; the profile check reports it).
        let mut logical_use: FxHashSet<Id> = FxHashSet::default();
        for t in &self.triples {
            let [s, p, o] = *t;
            if p == v.on_property
                || p == v.inverse_of
                || p == v.assertion_property
                || p == v.annotated_property
            {
                logical_use.insert(o);
            }
            if p == v.domain
                || p == v.range
                || p == v.sub_property_of
                || p == v.equivalent_property
                || p == v.property_disjoint_with
                || p == v.property_chain_axiom
                || p == v.inverse_of
            {
                logical_use.insert(s);
                logical_use.insert(o);
            }
            if p == v.ty
                && [
                    v.functional,
                    v.inverse_functional,
                    v.transitive,
                    v.symmetric,
                    v.asymmetric,
                    v.reflexive,
                    v.irreflexive,
                ]
                .contains(&o)
            {
                logical_use.insert(s);
            }
        }
        let mut undeclared: FxHashMap<Id, Vec<usize>> = FxHashMap::default();
        for i in 0..self.triples.len() {
            let [s, p, o] = self.triples[i];
            if !self.is_assertion_predicate(p) || self.structural.contains(&s) {
                continue;
            }
            if !self.kinds(p).is_empty() {
                continue;
            }
            if logical_use.contains(&p) {
                // Settle the kind from the object.
                if is_literal(self.dict, o) {
                    self.add_kind(p, PropKind::Data);
                } else {
                    self.add_kind(p, PropKind::Object);
                }
            } else {
                undeclared.entry(p).or_default().push(i);
            }
        }
        let mut undeclared: Vec<(Id, Vec<usize>)> = undeclared.into_iter().collect();
        undeclared.sort_by_key(|(_, ts)| ts[0]);
        for (p, ts) in undeclared {
            self.add_kind(p, PropKind::Annotation);
            let source = self.src(&ts);
            self.out.notes.push(ExtractNote {
                message: format!(
                    "{} is not declared; its {} triple(s) were read as annotations",
                    iri_of(self.dict, p).unwrap_or_default(),
                    ts.len()
                ),
                source,
            });
        }
        Ok(())
    }

    /// `true` for a predicate that is not part of the mapping vocabulary (so a triple using it
    /// is a property assertion or an annotation).
    fn is_assertion_predicate(&self, p: Id) -> bool {
        let Some(iri) = iri_of(self.dict, p) else {
            return false;
        };
        if self.v.is_builtin_annotation_property(p) {
            return true;
        }
        // The reserved vocabularies are never user properties.
        !(iri.starts_with(OWL) || iri.starts_with(RDF) || iri.starts_with(RDFS))
            && !(iri.starts_with(XSD) && super::datatypes::is_facet_iri(&iri))
    }

    // ---------------------------------------------------------------------------------
    // lists
    // ---------------------------------------------------------------------------------

    /// Reads an RDF list without consuming it (used by the inference pass).
    fn peek_list(&self, head: Id) -> Result<(Vec<Id>, Vec<usize>)> {
        let mut items = Vec::new();
        let mut used = Vec::new();
        let mut seen = FxHashSet::default();
        let mut cell = head;
        while cell != self.v.nil {
            if cell == NO_ID || !is_blank(self.dict, cell) {
                return Err(self.err(
                    ExtractErrorKind::Malformed,
                    "an RDF list cell is not a blank node (lists must end in rdf:nil)",
                    &used,
                ));
            }
            if !seen.insert(cell) {
                return Err(self.err(ExtractErrorKind::Malformed, "cyclic RDF list", &used));
            }
            let first = self.sp(cell, self.v.first);
            let rest = self.sp(cell, self.v.rest);
            if first.len() != 1 || rest.len() != 1 {
                let at: Vec<usize> = first.iter().chain(rest.iter()).copied().collect();
                return Err(self.err(
                    ExtractErrorKind::Malformed,
                    "an RDF list cell must have exactly one rdf:first and one rdf:rest",
                    if at.is_empty() { &used } else { &at },
                ));
            }
            items.push(self.triples[first[0]][2]);
            used.push(first[0]);
            used.push(rest[0]);
            if let Some(t) = self.has_type(cell, self.v.rdf_list) {
                used.push(t);
            }
            cell = self.triples[rest[0]][2];
        }
        Ok((items, used))
    }

    /// Reads and consumes an RDF list.
    fn list(&mut self, head: Id) -> Result<(Vec<Id>, Vec<usize>)> {
        let (items, used) = self.peek_list(head)?;
        for &i in &used {
            self.consume(i);
        }
        Ok((items, used))
    }

    // ---------------------------------------------------------------------------------
    // terms
    // ---------------------------------------------------------------------------------

    fn individual(&mut self, node: Id, at: usize) -> Result<Individual> {
        if is_iri(self.dict, node) {
            Ok(Individual::Named(node))
        } else if is_blank(self.dict, node) && !self.structural.contains(&node) {
            Ok(Individual::Anonymous(node))
        } else {
            Err(self.err(
                ExtractErrorKind::Malformed,
                "an individual is required here (an IRI or a blank node)",
                &[at],
            ))
        }
    }

    fn literal(&self, node: Id, at: usize) -> Result<Literal> {
        literal_of(self.dict, node).ok_or_else(|| {
            self.err(
                ExtractErrorKind::Malformed,
                "a literal is required here",
                &[at],
            )
        })
    }

    fn cardinality(&self, node: Id, at: usize) -> Result<u32> {
        let lit = self.literal(node, at)?;
        let integer_like = lit.datatype == format!("{XSD}nonNegativeInteger")
            || lit.datatype == format!("{XSD}integer")
            || lit.datatype == format!("{XSD}positiveInteger")
            || lit.datatype == format!("{XSD}int")
            || lit.datatype == format!("{XSD}long")
            || lit.datatype == format!("{XSD}short")
            || lit.datatype == format!("{XSD}unsignedInt")
            || lit.datatype == format!("{XSD}unsignedLong");
        let parsed = super::datatypes::parse_integer(&lit.lexical);
        match (integer_like, parsed) {
            (true, Some(n)) if n >= 0 => {
                if n > i128::from(MAX_CARDINALITY) {
                    Err(self.err(
                        ExtractErrorKind::Unsupported,
                        format!("cardinality {n} is above the supported maximum {MAX_CARDINALITY}"),
                        &[at],
                    ))
                } else {
                    Ok(n as u32)
                }
            }
            _ => Err(self.err(
                ExtractErrorKind::Malformed,
                "a cardinality must be a non-negative integer literal",
                &[at],
            )),
        }
    }

    // ---------------------------------------------------------------------------------
    // property expressions
    // ---------------------------------------------------------------------------------

    fn object_property(&mut self, node: Id, at: usize) -> Result<(ObjectProperty, Vec<usize>)> {
        if is_iri(self.dict, node) {
            return match self.kind_of(node, at)? {
                Some(PropKind::Object) => Ok((ObjectProperty::Named(node), Vec::new())),
                Some(_) => Err(self.err(
                    ExtractErrorKind::Ambiguous,
                    format!(
                        "{} is used as an object property but declared as another kind",
                        iri_of(self.dict, node).unwrap_or_default()
                    ),
                    &[at],
                )),
                None => Err(self.err(
                    ExtractErrorKind::Ambiguous,
                    format!(
                        "{} is used as a property but its kind is undeclared and cannot be inferred",
                        iri_of(self.dict, node).unwrap_or_default()
                    ),
                    &[at],
                )),
            };
        }
        if is_blank(self.dict, node) {
            if let Some((target, i)) = self.sp_one(node, self.v.inverse_of)? {
                // The node's other triples are axioms about the inverse (sub-property,
                // characteristics), read by the axiom pass.
                if !is_iri(self.dict, target) {
                    return Err(self.err(
                        ExtractErrorKind::Malformed,
                        "an inverse property expression must be `_:x owl:inverseOf <property>`",
                        &[i],
                    ));
                }
                let (inner, _) = self.object_property(target, i)?;
                self.consume(i);
                return Ok((inner.inverse(), vec![i]));
            }
        }
        Err(self.err(
            ExtractErrorKind::Malformed,
            "an object property expression is required here",
            &[at],
        ))
    }

    fn data_property(&mut self, node: Id, at: usize) -> Result<Id> {
        if is_iri(self.dict, node) && self.kind_of(node, at)? == Some(PropKind::Data) {
            return Ok(node);
        }
        Err(self.err(
            ExtractErrorKind::Ambiguous,
            "a data property is required here",
            &[at],
        ))
    }

    // ---------------------------------------------------------------------------------
    // class expressions (Mapping §3.2.4, Table 13)
    // ---------------------------------------------------------------------------------

    /// Charges `size` model nodes to the expansion budget and checks the nesting `height`.
    fn charge(&mut self, size: usize, height: usize, at: usize) -> Result<()> {
        self.expanded = self.expanded.saturating_add(size);
        if self.expanded > MAX_EXPANSION {
            return Err(self.err(
                ExtractErrorKind::Unsupported,
                format!("the ontology expands beyond {MAX_EXPANSION} model nodes"),
                &[at],
            ));
        }
        if height > MAX_DEPTH {
            return Err(self.err(
                ExtractErrorKind::Unsupported,
                "expression nested too deeply",
                &[at],
            ));
        }
        Ok(())
    }

    fn class(&mut self, node: Id, at: usize) -> Result<(ClassExpression, Vec<usize>)> {
        self.class_depth(node, at, 0)
    }

    fn class_depth(
        &mut self,
        node: Id,
        at: usize,
        depth: usize,
    ) -> Result<(ClassExpression, Vec<usize>)> {
        if is_iri(self.dict, node) {
            if node == self.v.thing {
                return Ok((ClassExpression::Thing, Vec::new()));
            }
            if node == self.v.nothing {
                return Ok((ClassExpression::Nothing, Vec::new()));
            }
            if self.is_known_datatype(node) {
                return Err(self.err(
                    ExtractErrorKind::Malformed,
                    format!(
                        "the datatype {} is used where a class is required",
                        iri_of(self.dict, node).unwrap_or_default()
                    ),
                    &[at],
                ));
            }
            if !self.kinds(node).is_empty() && !self.classes.contains(&node) {
                // A property IRI in a class position is not OWL 2 DL punning we can read
                // safely without a class declaration.
                return Err(self.err(
                    ExtractErrorKind::Malformed,
                    format!(
                        "the property {} is used where a class is required (declare it as owl:Class to pun)",
                        iri_of(self.dict, node).unwrap_or_default()
                    ),
                    &[at],
                ));
            }
            return Ok((ClassExpression::Class(node), Vec::new()));
        }
        if !is_blank(self.dict, node) {
            return Err(self.err(
                ExtractErrorKind::Malformed,
                "a class expression is required here (an IRI or a blank node)",
                &[at],
            ));
        }
        if self.range_cache.contains_key(&node) {
            return Err(self.err(
                ExtractErrorKind::Ambiguous,
                "a blank node is read both as a class expression and as a data range",
                &[at],
            ));
        }
        if let Some((hit, size, height)) = self.class_cache.get(&node).cloned() {
            self.charge(size, depth + height, at)?;
            return Ok(hit);
        }
        if depth > MAX_DEPTH {
            return Err(self.err(
                ExtractErrorKind::Unsupported,
                "class expression nested too deeply",
                &[at],
            ));
        }
        if !self.visiting.insert(node) {
            return Err(self.err(
                ExtractErrorKind::Malformed,
                "cyclic class expression",
                &[at],
            ));
        }
        let r = self.class_blank(node, at, depth);
        self.visiting.remove(&node);
        let r = r?;
        let (size, height) = class_size(&r.0);
        self.charge(size + r.1.len(), depth, at)?;
        self.class_cache
            .insert(node, (r.clone(), size + r.1.len(), height));
        Ok(r)
    }

    fn class_blank(
        &mut self,
        node: Id,
        at: usize,
        depth: usize,
    ) -> Result<(ClassExpression, Vec<usize>)> {
        let v = self.v.clone();
        let mut used = Vec::new();
        if let Some(i) = self.has_type(node, v.owl_class) {
            used.push(i);
        }
        let boolean = [
            (v.intersection_of, 0u8),
            (v.union_of, 1),
            (v.complement_of, 2),
            (v.one_of, 3),
        ];
        let mut shape: Option<(u8, Id, usize)> = None;
        for (p, tag) in boolean {
            if let Some((o, i)) = self.sp_one(node, p)? {
                if shape.is_some() {
                    return Err(self.err(
                        ExtractErrorKind::Malformed,
                        "a class expression node has two shapes",
                        &[i],
                    ));
                }
                shape = Some((tag, o, i));
            }
        }
        let restriction = self.has_type(node, v.restriction);
        if let Some((tag, o, i)) = shape {
            if restriction.is_some() {
                return Err(self.err(
                    ExtractErrorKind::Malformed,
                    "a node is both a restriction and a boolean class expression",
                    &[i],
                ));
            }
            used.push(i);
            let ce = match tag {
                0 | 1 => {
                    let (items, lu) = self.list(o)?;
                    used.extend(lu);
                    let mut members = Vec::with_capacity(items.len());
                    for m in items {
                        let (ce, mu) = self.class_depth(m, i, depth + 1)?;
                        used.extend(mu);
                        members.push(ce);
                    }
                    // Mapping Table 15: the empty and singleton forms, as for named classes.
                    match (members.len(), tag == 0) {
                        (0, true) => ClassExpression::Thing,
                        (0, false) => ClassExpression::Nothing,
                        (1, _) => members.pop().expect("one member"),
                        (_, true) => ClassExpression::IntersectionOf(members),
                        (_, false) => ClassExpression::UnionOf(members),
                    }
                }
                2 => {
                    let (inner, iu) = self.class_depth(o, i, depth + 1)?;
                    used.extend(iu);
                    ClassExpression::ComplementOf(Box::new(inner))
                }
                _ => {
                    let (items, lu) = self.list(o)?;
                    used.extend(lu);
                    if items.is_empty() {
                        // ObjectOneOf() is the empty class.
                        ClassExpression::Nothing
                    } else {
                        let mut inds = Vec::with_capacity(items.len());
                        for m in items {
                            inds.push(self.individual(m, i)?);
                        }
                        ClassExpression::OneOf(inds)
                    }
                }
            };
            for &u in &used {
                self.consume(u);
            }
            return Ok((ce, used));
        }
        let Some(rt) = restriction else {
            return Err(self.err(
                ExtractErrorKind::Malformed,
                "a blank node in a class position is not a class expression",
                &[at],
            ));
        };
        used.push(rt);
        self.restriction(node, depth, used)
    }

    fn restriction(
        &mut self,
        node: Id,
        depth: usize,
        mut used: Vec<usize>,
    ) -> Result<(ClassExpression, Vec<usize>)> {
        let v = self.v.clone();
        if let Some((_, i)) = self.sp_one(node, v.on_properties)? {
            return Err(self.err(
                ExtractErrorKind::Unsupported,
                "n-ary data restrictions (owl:onProperties) are not supported",
                &[i],
            ));
        }
        let Some((prop, pi)) = self.sp_one(node, v.on_property)? else {
            return Err(self.err(
                ExtractErrorKind::Malformed,
                "a restriction has no owl:onProperty",
                &used[..1],
            ));
        };
        used.push(pi);
        let kinds = [
            v.some_values_from,
            v.all_values_from,
            v.has_value,
            v.has_self,
            v.min_cardinality,
            v.max_cardinality,
            v.cardinality,
            v.min_qualified_cardinality,
            v.max_qualified_cardinality,
            v.qualified_cardinality,
        ];
        let mut found: Option<(Id, Id, usize)> = None;
        for p in kinds {
            if let Some((o, i)) = self.sp_one(node, p)? {
                if found.is_some() {
                    return Err(self.err(
                        ExtractErrorKind::Malformed,
                        "a restriction has two shape predicates",
                        &[i],
                    ));
                }
                found = Some((p, o, i));
            }
        }
        let Some((shape, filler, fi)) = found else {
            return Err(self.err(
                ExtractErrorKind::Malformed,
                "a restriction has no filler or cardinality",
                &[pi],
            ));
        };
        used.push(fi);
        // Qualified cardinalities take their filler from owl:onClass / owl:onDataRange.
        let qualified = [
            v.min_qualified_cardinality,
            v.max_qualified_cardinality,
            v.qualified_cardinality,
        ]
        .contains(&shape);
        let on_class = self.sp_one(node, v.on_class)?;
        let on_range = self.sp_one(node, v.on_data_range)?;
        if !qualified && (on_class.is_some() || on_range.is_some()) {
            let i = on_class.or(on_range).map(|(_, i)| i).unwrap_or(fi);
            return Err(self.err(
                ExtractErrorKind::Malformed,
                "owl:onClass / owl:onDataRange require a qualified cardinality",
                &[i],
            ));
        }
        if qualified && on_class.is_some() == on_range.is_some() {
            return Err(self.err(
                ExtractErrorKind::Malformed,
                "a qualified cardinality needs exactly one of owl:onClass / owl:onDataRange",
                &[fi],
            ));
        }
        let is_data = match self.kind_of(prop, pi)? {
            Some(PropKind::Data) => true,
            Some(PropKind::Object) => false,
            Some(PropKind::Annotation) => {
                return Err(self.err(
                    ExtractErrorKind::Ambiguous,
                    "an annotation property is used in a restriction",
                    &[pi],
                ))
            }
            None if is_blank(self.dict, prop) => false, // owl:inverseOf expression
            None => {
                return Err(self.err(
                    ExtractErrorKind::Ambiguous,
                    format!(
                        "the kind of {} is undeclared and cannot be inferred",
                        iri_of(self.dict, prop).unwrap_or_default()
                    ),
                    &[pi],
                ))
            }
        };
        let ce = if is_data {
            let dp = self.data_property(prop, pi)?;
            let range_of = |x: &mut Self, n: Id, i: usize, used: &mut Vec<usize>| {
                x.data_range_depth(n, i, depth + 1).map(|(r, u)| {
                    used.extend(u);
                    r
                })
            };
            if shape == v.some_values_from {
                ClassExpression::DataSomeValuesFrom(dp, range_of(self, filler, fi, &mut used)?)
            } else if shape == v.all_values_from {
                ClassExpression::DataAllValuesFrom(dp, range_of(self, filler, fi, &mut used)?)
            } else if shape == v.has_value {
                ClassExpression::DataHasValue(dp, self.literal(filler, fi)?)
            } else if shape == v.has_self {
                return Err(self.err(
                    ExtractErrorKind::Malformed,
                    "owl:hasSelf needs an object property",
                    &[fi],
                ));
            } else {
                let n = self.cardinality(filler, fi)?;
                let dr = match on_range {
                    Some((r, ri)) => {
                        used.push(ri);
                        range_of(self, r, ri, &mut used)?
                    }
                    None if on_class.is_some() => {
                        return Err(self.err(
                            ExtractErrorKind::Malformed,
                            "owl:onClass on a data property restriction",
                            &[fi],
                        ))
                    }
                    None => DataRange::TopLiteral,
                };
                if shape == v.min_cardinality || shape == v.min_qualified_cardinality {
                    ClassExpression::DataMinCardinality(n, dp, dr)
                } else if shape == v.max_cardinality || shape == v.max_qualified_cardinality {
                    ClassExpression::DataMaxCardinality(n, dp, dr)
                } else {
                    ClassExpression::DataExactCardinality(n, dp, dr)
                }
            }
        } else {
            let (op, ou) = self.object_property(prop, pi)?;
            used.extend(ou);
            let class_of = |x: &mut Self, n: Id, i: usize, used: &mut Vec<usize>| {
                x.class_depth(n, i, depth + 1).map(|(c, u)| {
                    used.extend(u);
                    c
                })
            };
            if shape == v.some_values_from {
                ClassExpression::SomeValuesFrom(
                    op,
                    Box::new(class_of(self, filler, fi, &mut used)?),
                )
            } else if shape == v.all_values_from {
                ClassExpression::AllValuesFrom(op, Box::new(class_of(self, filler, fi, &mut used)?))
            } else if shape == v.has_value {
                ClassExpression::HasValue(op, self.individual(filler, fi)?)
            } else if shape == v.has_self {
                let lit = self.literal(filler, fi)?;
                let is_true = lit.datatype == format!("{XSD}boolean")
                    && (lit.lexical == "true" || lit.lexical == "1");
                if !is_true {
                    return Err(self.err(
                        ExtractErrorKind::Malformed,
                        "owl:hasSelf must be \"true\"^^xsd:boolean",
                        &[fi],
                    ));
                }
                ClassExpression::HasSelf(op)
            } else {
                let n = self.cardinality(filler, fi)?;
                let c = match on_class {
                    Some((c, ci)) => {
                        used.push(ci);
                        class_of(self, c, ci, &mut used)?
                    }
                    None if on_range.is_some() => {
                        return Err(self.err(
                            ExtractErrorKind::Malformed,
                            "owl:onDataRange on an object property restriction",
                            &[fi],
                        ))
                    }
                    None => ClassExpression::Thing,
                };
                let c = Box::new(c);
                if shape == v.min_cardinality || shape == v.min_qualified_cardinality {
                    ClassExpression::MinCardinality(n, op, c)
                } else if shape == v.max_cardinality || shape == v.max_qualified_cardinality {
                    ClassExpression::MaxCardinality(n, op, c)
                } else {
                    ClassExpression::ExactCardinality(n, op, c)
                }
            }
        };
        for &u in &used {
            self.consume(u);
        }
        Ok((ce, used))
    }

    // ---------------------------------------------------------------------------------
    // data ranges (Mapping Table 12)
    // ---------------------------------------------------------------------------------

    fn data_range(&mut self, node: Id, at: usize) -> Result<(DataRange, Vec<usize>)> {
        self.data_range_depth(node, at, 0)
    }

    fn data_range_depth(
        &mut self,
        node: Id,
        at: usize,
        depth: usize,
    ) -> Result<(DataRange, Vec<usize>)> {
        if is_iri(self.dict, node) {
            if self.classes.contains(&node) && !self.is_known_datatype(node) {
                return Err(self.err(
                    ExtractErrorKind::Malformed,
                    format!(
                        "the class {} is used where a data range is required",
                        iri_of(self.dict, node).unwrap_or_default()
                    ),
                    &[at],
                ));
            }
            if node == self.v.rdfs_literal {
                return Ok((DataRange::TopLiteral, Vec::new()));
            }
            return Ok((DataRange::Datatype(node), Vec::new()));
        }
        if !is_blank(self.dict, node) {
            return Err(self.err(
                ExtractErrorKind::Malformed,
                "a data range is required here (an IRI or a blank node)",
                &[at],
            ));
        }
        if self.class_cache.contains_key(&node) {
            return Err(self.err(
                ExtractErrorKind::Ambiguous,
                "a blank node is read both as a class expression and as a data range",
                &[at],
            ));
        }
        if let Some((hit, size, height)) = self.range_cache.get(&node).cloned() {
            self.charge(size, depth + height, at)?;
            return Ok(hit);
        }
        if depth > MAX_DEPTH {
            return Err(self.err(
                ExtractErrorKind::Unsupported,
                "data range nested too deeply",
                &[at],
            ));
        }
        if !self.visiting.insert(node) {
            return Err(self.err(ExtractErrorKind::Malformed, "cyclic data range", &[at]));
        }
        let r = self.data_range_blank(node, at, depth);
        self.visiting.remove(&node);
        let r = r?;
        let (size, height) = range_size(&r.0);
        self.charge(size + r.1.len(), depth, at)?;
        self.range_cache
            .insert(node, (r.clone(), size + r.1.len(), height));
        Ok(r)
    }

    fn data_range_blank(
        &mut self,
        node: Id,
        at: usize,
        depth: usize,
    ) -> Result<(DataRange, Vec<usize>)> {
        let v = self.v.clone();
        let mut used = Vec::new();
        if let Some(i) = self.has_type(node, v.rdfs_datatype) {
            used.push(i);
        }
        let shapes = [
            (v.intersection_of, 0u8),
            (v.union_of, 1),
            (v.datatype_complement_of, 2),
            (v.one_of, 3),
            (v.on_datatype, 4),
        ];
        let mut shape: Option<(u8, Id, usize)> = None;
        for (p, tag) in shapes {
            if let Some((o, i)) = self.sp_one(node, p)? {
                if shape.is_some() {
                    return Err(self.err(
                        ExtractErrorKind::Malformed,
                        "a data range node has two shapes",
                        &[i],
                    ));
                }
                shape = Some((tag, o, i));
            }
        }
        let Some((tag, o, i)) = shape else {
            return Err(self.err(
                ExtractErrorKind::Malformed,
                "a blank node in a data range position is not a data range",
                &[at],
            ));
        };
        used.push(i);
        let dr = match tag {
            0 | 1 => {
                let (items, lu) = self.list(o)?;
                used.extend(lu);
                if items.is_empty() {
                    return Err(self.err(
                        ExtractErrorKind::Malformed,
                        "a data intersection or union needs at least one member",
                        &[i],
                    ));
                }
                let mut members = Vec::with_capacity(items.len());
                for m in items {
                    let (r, ru) = self.data_range_depth(m, i, depth + 1)?;
                    used.extend(ru);
                    members.push(r);
                }
                if members.len() == 1 {
                    members.pop().expect("one member")
                } else if tag == 0 {
                    DataRange::IntersectionOf(members)
                } else {
                    DataRange::UnionOf(members)
                }
            }
            2 => {
                let (inner, iu) = self.data_range_depth(o, i, depth + 1)?;
                used.extend(iu);
                DataRange::ComplementOf(Box::new(inner))
            }
            3 => {
                let (items, lu) = self.list(o)?;
                used.extend(lu);
                let mut lits = Vec::with_capacity(items.len());
                self.arity(items.len(), 1, "DataOneOf", i)?;
                for m in items {
                    lits.push(self.literal(m, i)?);
                }
                DataRange::OneOf(lits)
            }
            _ => {
                if !is_iri(self.dict, o) {
                    return Err(self.err(
                        ExtractErrorKind::Malformed,
                        "owl:onDatatype must name a datatype",
                        &[i],
                    ));
                }
                let Some((list, wi)) = self.sp_one(node, v.with_restrictions)? else {
                    return Err(self.err(
                        ExtractErrorKind::Malformed,
                        "a datatype restriction needs owl:withRestrictions",
                        &[i],
                    ));
                };
                used.push(wi);
                let (items, lu) = self.list(list)?;
                used.extend(lu);
                let mut facets = Vec::with_capacity(items.len());
                for f in items {
                    let ts = self.by_subject.get(&f).cloned().unwrap_or_default();
                    if ts.len() != 1 || !is_blank(self.dict, f) {
                        return Err(self.err(
                            ExtractErrorKind::Malformed,
                            "each owl:withRestrictions member must be a blank node with exactly one facet",
                            &[wi],
                        ));
                    }
                    let [_, facet, value] = self.triples[ts[0]];
                    let lit = self.literal(value, ts[0])?;
                    used.push(ts[0]);
                    facets.push((facet, lit));
                }
                self.arity(facets.len(), 1, "DatatypeRestriction", i)?;
                DataRange::Restriction(o, facets)
            }
        };
        if tag != 4 {
            if let Some((_, wi)) = self.sp_one(node, v.with_restrictions)? {
                return Err(self.err(
                    ExtractErrorKind::Malformed,
                    "owl:withRestrictions without owl:onDatatype",
                    &[wi],
                ));
            }
        }
        for &u in &used {
            self.consume(u);
        }
        Ok((dr, used))
    }

    // ---------------------------------------------------------------------------------
    // pass 3: axioms (Mapping §3.2.5–§3.2.7, Tables 16–18)
    // ---------------------------------------------------------------------------------

    fn axioms(&mut self) -> Result<()> {
        let v = self.v.clone();
        // Ontology headers (Mapping §3.1.1): the typing, imports, the version IRI and
        // annotations. Any other triple on the header node is read as an ordinary axiom.
        let headers: Vec<Id> = self
            .triples
            .iter()
            .filter(|t| t[1] == v.ty && t[2] == v.ontology)
            .map(|t| t[0])
            .collect();
        for h in headers {
            for i in self.by_subject.get(&h).cloned().unwrap_or_default() {
                let [_, p, o] = self.triples[i];
                if p == v.ty && o == v.ontology {
                    self.consume(i);
                } else if p == v.imports {
                    self.out.imports.push(o);
                    self.consume(i);
                } else if p == v.version_iri {
                    self.consume(i);
                } else if self.is_annotation_predicate(p) {
                    let value = self.annotation_value(o, i)?;
                    self.out.annotations.push(Annotation {
                        target: AnnotationTarget::Ontology(h),
                        property: p,
                        value,
                        source: vec![i],
                    });
                    self.consume(i);
                }
            }
        }
        self.reifications()?;
        // Typing triples first, so an n-ary axiom or negative assertion node is read from its
        // `rdf:type` whatever order its triples come in.
        let order: Vec<usize> = (0..self.triples.len())
            .filter(|&i| self.triples[i][1] == v.ty)
            .chain((0..self.triples.len()).filter(|&i| self.triples[i][1] != v.ty))
            .collect();
        for i in order {
            if self.consumed[i] {
                continue;
            }
            let [s, p, o] = self.triples[i];
            let structural_part = self.structural_preds.contains(&p)
                || (p == v.ty && (o == v.restriction || o == v.rdfs_datatype || o == v.owl_class));
            if self.facet_nodes.contains(&s) || (self.structural.contains(&s) && structural_part) {
                // Read by the decoders when (if) an axiom references the node. Other triples
                // on the node are axioms about the expression (a GCI's `rdfs:subClassOf`, an
                // inverse property's `rdfs:subPropertyOf`) and are read below.
                continue;
            }
            self.axiom_triple(i)?;
        }
        Ok(())
    }

    /// `true` for a predicate read as an annotation property.
    fn is_annotation_predicate(&self, p: Id) -> bool {
        self.v.is_builtin_annotation_property(p) || self.kinds(p).contains(&PropKind::Annotation)
    }

    fn annotation_value(&self, o: Id, at: usize) -> Result<AnnotationValue> {
        if is_iri(self.dict, o) {
            Ok(AnnotationValue::Iri(o))
        } else if is_blank(self.dict, o) {
            Ok(AnnotationValue::Anonymous(o))
        } else {
            Ok(AnnotationValue::Literal(self.literal(o, at)?))
        }
    }

    /// Axiom and annotation reifications (Mapping §3.2.5, Table 17): an `owl:Axiom` node
    /// names one triple of the graph and carries its annotations; an `owl:Annotation` node
    /// names one annotation and carries annotations of it. Anything else on such a node is an
    /// error.
    fn reifications(&mut self) -> Result<()> {
        let v = self.v.clone();
        let nodes = |ty: Id| -> Vec<(Id, usize)> {
            self.triples
                .iter()
                .enumerate()
                .filter(|(_, t)| t[1] == v.ty && t[2] == ty && ty != NO_ID)
                .map(|(i, t)| (t[0], i))
                .collect()
        };
        let mut axioms = nodes(v.owl_axiom);
        let mut nested = nodes(v.owl_annotation);
        axioms.sort_unstable();
        nested.sort_unstable();
        // Annotations read from each reification node: (node, property, value) → index.
        let mut by_node: FxHashMap<(Id, Id, AnnotationValue), usize> = FxHashMap::default();
        for (node, ti) in axioms {
            let (sv, si) = self.required(node, v.annotated_source, ti)?;
            let (pv, pi) = self.required(node, v.annotated_property, ti)?;
            let (ov, oi) = self.required(node, v.annotated_target, ti)?;
            let Some(main) = self.by_subject.get(&sv).and_then(|is| {
                is.iter()
                    .copied()
                    .find(|&j| self.triples[j] == [sv, pv, ov])
            }) else {
                return Err(self.err(
                    ExtractErrorKind::Malformed,
                    "an owl:Axiom reification names a triple that is not in the graph",
                    &[ti, si, pi, oi],
                ));
            };
            self.reified_annotations(
                node,
                AnnotationTarget::Axiom(main),
                &[ti, si, pi, oi],
                &mut by_node,
            )?;
        }
        // Nested annotations may themselves be annotated: resolve to a fixpoint.
        let mut pending = nested;
        loop {
            let before = pending.len();
            let mut rest = Vec::new();
            for (node, ti) in pending {
                let (sv, si) = self.required(node, v.annotated_source, ti)?;
                let (pv, pi) = self.required(node, v.annotated_property, ti)?;
                let (ov, oi) = self.required(node, v.annotated_target, ti)?;
                let value = self.annotation_value(ov, oi)?;
                match by_node.get(&(sv, pv, value)).copied() {
                    Some(k) => self.reified_annotations(
                        node,
                        AnnotationTarget::Annotation(k),
                        &[ti, si, pi, oi],
                        &mut by_node,
                    )?,
                    None => rest.push((node, ti)),
                }
            }
            if rest.is_empty() {
                return Ok(());
            }
            if rest.len() == before {
                let (_, ti) = rest[0];
                return Err(self.err(
                    ExtractErrorKind::Malformed,
                    "an owl:Annotation reification names an annotation that is not in the graph",
                    &[ti],
                ));
            }
            pending = rest;
        }
    }

    fn reified_annotations(
        &mut self,
        node: Id,
        target: AnnotationTarget,
        header: &[usize],
        by_node: &mut FxHashMap<(Id, Id, AnnotationValue), usize>,
    ) -> Result<()> {
        for &u in header {
            self.consume(u);
        }
        for i in self.by_subject.get(&node).cloned().unwrap_or_default() {
            if header.contains(&i) {
                continue;
            }
            let [_, p, o] = self.triples[i];
            if !self.is_annotation_predicate(p) {
                return Err(self.err(
                    ExtractErrorKind::Malformed,
                    "a reification node may only carry annotations",
                    &[i],
                ));
            }
            let value = self.annotation_value(o, i)?;
            by_node.insert((node, p, value.clone()), self.out.annotations.len());
            self.out.annotations.push(Annotation {
                target,
                property: p,
                value,
                source: vec![i],
            });
            self.consume(i);
        }
        Ok(())
    }

    fn axiom_triple(&mut self, i: usize) -> Result<()> {
        let v = self.v.clone();
        let [s, p, o] = self.triples[i];
        if p == v.sub_class_of {
            let (sub, su) = self.class(s, i)?;
            let (sup, ou) = self.class(o, i)?;
            self.emit(i, Axiom::SubClassOf(sub, sup), [su, ou]);
        } else if p == v.equivalent_class {
            if self.datatypes.contains(&s) {
                let (dr, ou) = self.data_range(o, i)?;
                self.emit(i, Axiom::DatatypeDefinition(s, dr), [ou]);
            } else {
                let (a, su) = self.class(s, i)?;
                let (b, ou) = self.class(o, i)?;
                self.emit(i, Axiom::EquivalentClasses(vec![a, b]), [su, ou]);
            }
        } else if p == v.disjoint_with {
            let (a, su) = self.class(s, i)?;
            let (b, ou) = self.class(o, i)?;
            self.emit(i, Axiom::DisjointClasses(vec![a, b]), [su, ou]);
        } else if p == v.disjoint_union_of {
            if !is_iri(self.dict, s) {
                return Err(self.err(
                    ExtractErrorKind::Malformed,
                    "owl:disjointUnionOf needs a named class subject",
                    &[i],
                ));
            }
            let (items, mut used) = self.list(o)?;
            let mut members = Vec::new();
            for m in items {
                let (c, cu) = self.class(m, i)?;
                used.extend(cu);
                members.push(c);
            }
            self.arity(members.len(), 2, "DisjointUnion", i)?;
            self.emit(i, Axiom::DisjointUnion(s, members), [used]);
        } else if p == v.intersection_of || p == v.union_of || p == v.complement_of || p == v.one_of
        {
            // OWL 1 style: a named class with an inline definition (`:A owl:unionOf (…)`)
            // means EquivalentClasses(A, …).
            self.named_class_definition(i)?;
        } else if p == v.sub_property_of {
            self.sub_property(i)?;
        } else if p == v.property_chain_axiom {
            let (sup, su) = self.object_property(s, i)?;
            let (items, mut used) = self.list(o)?;
            if items.len() < 2 {
                return Err(self.err(
                    ExtractErrorKind::Malformed,
                    "a property chain needs at least two properties",
                    &[i],
                ));
            }
            let mut chain = Vec::new();
            for m in items {
                let (op, ou) = self.object_property(m, i)?;
                used.extend(ou);
                chain.push(op);
            }
            used.extend(su);
            self.emit(i, Axiom::SubObjectPropertyOf(chain, sup), [used]);
        } else if p == v.equivalent_property || p == v.property_disjoint_with {
            let equivalent = p == v.equivalent_property;
            match self.pair_kind(s, o, i)? {
                PropKind::Object => {
                    let (a, au) = self.object_property(s, i)?;
                    let (b, bu) = self.object_property(o, i)?;
                    let ax = if equivalent {
                        Axiom::EquivalentObjectProperties(vec![a, b])
                    } else {
                        Axiom::DisjointObjectProperties(vec![a, b])
                    };
                    self.emit(i, ax, [au, bu]);
                }
                PropKind::Data => {
                    let a = self.data_property(s, i)?;
                    let b = self.data_property(o, i)?;
                    let ax = if equivalent {
                        Axiom::EquivalentDataProperties(vec![a, b])
                    } else {
                        Axiom::DisjointDataProperties(vec![a, b])
                    };
                    self.emit(i, ax, []);
                }
                PropKind::Annotation => {
                    return Err(self.err(
                        ExtractErrorKind::Malformed,
                        "property equivalence / disjointness on annotation properties",
                        &[i],
                    ))
                }
            }
        } else if p == v.domain || p == v.range {
            self.domain_range(i, p == v.domain)?;
        } else if p == v.inverse_of {
            let (a, au) = self.object_property(s, i)?;
            let (b, bu) = self.object_property(o, i)?;
            self.emit(i, Axiom::InverseObjectProperties(a, b), [au, bu]);
        } else if p == v.has_key {
            let (c, mut used) = self.class(s, i)?;
            let (items, lu) = self.list(o)?;
            used.extend(lu);
            let mut ops = Vec::new();
            let mut dps = Vec::new();
            for m in items {
                match self.kind_of(m, i)? {
                    Some(PropKind::Data) => dps.push(self.data_property(m, i)?),
                    _ => {
                        let (op, ou) = self.object_property(m, i)?;
                        used.extend(ou);
                        ops.push(op);
                    }
                }
            }
            self.arity(ops.len() + dps.len(), 1, "HasKey", i)?;
            self.emit(i, Axiom::HasKey(c, ops, dps), [used]);
        } else if p == v.same_as || p == v.different_from {
            let a = self.individual(s, i)?;
            let b = self.individual(o, i)?;
            let ax = if p == v.same_as {
                Axiom::SameIndividual(vec![a, b])
            } else {
                Axiom::DifferentIndividuals(vec![a, b])
            };
            self.emit(i, ax, []);
        } else if p == v.ty {
            self.typing(i)?;
        } else if p == v.imports {
            // An owl:imports outside an ontology header.
            return Err(self.err(
                ExtractErrorKind::Malformed,
                "owl:imports on a node that is not an owl:Ontology",
                &[i],
            ));
        } else {
            self.assertion(i)?;
        }
        Ok(())
    }

    /// Records an axiom read from triple `i` plus the triples in `parts`, consuming them all.
    fn emit<const N: usize>(&mut self, i: usize, axiom: Axiom, parts: [Vec<usize>; N]) {
        let mut local = vec![i];
        for part in parts {
            local.extend(part);
        }
        for &u in &local {
            self.consume(u);
        }
        let source = self.src(&local);
        self.push(axiom, source);
    }

    fn named_class_definition(&mut self, i: usize) -> Result<()> {
        let v = self.v.clone();
        let [s, p, o] = self.triples[i];
        if !is_iri(self.dict, s) {
            return Err(self.err(
                ExtractErrorKind::Unparsed,
                "no OWL 2 mapping rule reads this triple",
                &[i],
            ));
        }
        if self.datatypes.contains(&s) {
            return Err(self.err(
                ExtractErrorKind::Malformed,
                "a datatype with an inline class definition",
                &[i],
            ));
        }
        let mut used = vec![i];
        let def = if p == v.complement_of {
            let (c, cu) = self.class(o, i)?;
            used.extend(cu);
            ClassExpression::ComplementOf(Box::new(c))
        } else {
            let (items, lu) = self.list(o)?;
            used.extend(lu);
            if p == v.one_of {
                let mut inds = Vec::new();
                for m in items {
                    inds.push(self.individual(m, i)?);
                }
                if inds.is_empty() {
                    ClassExpression::Nothing
                } else {
                    ClassExpression::OneOf(inds)
                }
            } else {
                let mut members = Vec::new();
                for m in items {
                    let (c, cu) = self.class(m, i)?;
                    used.extend(cu);
                    members.push(c);
                }
                match (members.len(), p == v.intersection_of) {
                    (0, true) => ClassExpression::Thing,
                    (0, false) => ClassExpression::Nothing,
                    (1, _) => members.pop().expect("one member"),
                    (_, true) => ClassExpression::IntersectionOf(members),
                    (_, false) => ClassExpression::UnionOf(members),
                }
            }
        };
        self.classes.insert(s);
        for &u in &used {
            self.consume(u);
        }
        let source = self.src(&used);
        self.push(
            Axiom::EquivalentClasses(vec![ClassExpression::Class(s), def]),
            source,
        );
        Ok(())
    }

    fn pair_kind(&self, s: Id, o: Id, i: usize) -> Result<PropKind> {
        let ks = self.kind_of(s, i)?;
        let ko = if is_iri(self.dict, o) {
            self.kind_of(o, i)?
        } else {
            Some(PropKind::Object)
        };
        let ks = if is_blank(self.dict, s) {
            Some(PropKind::Object)
        } else {
            ks
        };
        match (ks, ko) {
            (Some(a), Some(b)) if a == b => Ok(a),
            (Some(a), None) | (None, Some(a)) => Ok(a),
            (None, None) => Err(self.err(
                ExtractErrorKind::Ambiguous,
                "neither property's kind is declared or inferable",
                &[i],
            )),
            _ => Err(self.err(
                ExtractErrorKind::Ambiguous,
                "a property axiom relates properties of different kinds",
                &[i],
            )),
        }
    }

    fn sub_property(&mut self, i: usize) -> Result<()> {
        let [s, _, o] = self.triples[i];
        match self.pair_kind(s, o, i)? {
            PropKind::Object => {
                let (a, au) = self.object_property(s, i)?;
                let (b, bu) = self.object_property(o, i)?;
                self.emit(i, Axiom::SubObjectPropertyOf(vec![a], b), [au, bu]);
            }
            PropKind::Data => {
                let a = self.data_property(s, i)?;
                let b = self.data_property(o, i)?;
                self.emit(i, Axiom::SubDataPropertyOf(a, b), []);
            }
            PropKind::Annotation => {
                self.annotation_iris(i)?;
                self.emit(i, Axiom::SubAnnotationPropertyOf(s, o), []);
            }
        }
        Ok(())
    }

    /// Refuses a construct with fewer than `min` operands (Structural Specification arities).
    fn arity(&self, n: usize, min: usize, construct: &str, at: usize) -> Result<()> {
        if n < min {
            return Err(self.err(
                ExtractErrorKind::Malformed,
                format!("{construct} needs at least {min} operand(s), found {n}"),
                &[at],
            ));
        }
        Ok(())
    }

    /// Annotation-property axioms relate IRIs only.
    fn annotation_iris(&self, i: usize) -> Result<()> {
        let [s, _, o] = self.triples[i];
        if is_iri(self.dict, s) && is_iri(self.dict, o) {
            Ok(())
        } else {
            Err(self.err(
                ExtractErrorKind::Malformed,
                "an annotation property axiom relates two IRIs",
                &[i],
            ))
        }
    }

    fn domain_range(&mut self, i: usize, is_domain: bool) -> Result<()> {
        let [s, _, o] = self.triples[i];
        let kind = if is_blank(self.dict, s) {
            Some(PropKind::Object)
        } else {
            self.kind_of(s, i)?
        };
        match kind {
            Some(PropKind::Object) => {
                let (op, ou) = self.object_property(s, i)?;
                let (c, cu) = self.class(o, i)?;
                let ax = if is_domain {
                    Axiom::ObjectPropertyDomain(op, c)
                } else {
                    Axiom::ObjectPropertyRange(op, c)
                };
                self.emit(i, ax, [ou, cu]);
            }
            Some(PropKind::Data) => {
                if is_domain {
                    let (c, cu) = self.class(o, i)?;
                    self.emit(i, Axiom::DataPropertyDomain(s, c), [cu]);
                } else {
                    let (dr, ru) = self.data_range(o, i)?;
                    self.emit(i, Axiom::DataPropertyRange(s, dr), [ru]);
                }
            }
            Some(PropKind::Annotation) => {
                self.annotation_iris(i)?;
                let ax = if is_domain {
                    Axiom::AnnotationPropertyDomain(s, o)
                } else {
                    Axiom::AnnotationPropertyRange(s, o)
                };
                self.emit(i, ax, []);
            }
            None => {
                return Err(self.err(
                    ExtractErrorKind::Ambiguous,
                    format!(
                        "the kind of {} is undeclared and cannot be inferred",
                        iri_of(self.dict, s).unwrap_or_default()
                    ),
                    &[i],
                ))
            }
        }
        Ok(())
    }

    fn typing(&mut self, i: usize) -> Result<()> {
        let v = self.v.clone();
        let [s, _, o] = self.triples[i];
        // Property characteristics.
        let characteristic = [
            v.functional,
            v.inverse_functional,
            v.transitive,
            v.symmetric,
            v.asymmetric,
            v.reflexive,
            v.irreflexive,
        ];
        if o != NO_ID && characteristic.contains(&o) {
            if o == v.functional && self.kind_of(s, i)? == Some(PropKind::Data) {
                let d = self.data_property(s, i)?;
                self.emit(i, Axiom::FunctionalDataProperty(d), []);
                return Ok(());
            }
            let (op, ou) = self.object_property(s, i)?;
            let ax = if o == v.functional {
                Axiom::FunctionalObjectProperty(op)
            } else if o == v.inverse_functional {
                Axiom::InverseFunctionalObjectProperty(op)
            } else if o == v.transitive {
                Axiom::TransitiveObjectProperty(op)
            } else if o == v.symmetric {
                Axiom::SymmetricObjectProperty(op)
            } else if o == v.asymmetric {
                Axiom::AsymmetricObjectProperty(op)
            } else if o == v.reflexive {
                Axiom::ReflexiveObjectProperty(op)
            } else {
                Axiom::IrreflexiveObjectProperty(op)
            };
            self.emit(i, ax, [ou]);
            return Ok(());
        }
        // n-ary axioms and negative assertions on blank nodes.
        if o != NO_ID && o == v.all_disjoint_classes {
            let (list, li) = self.required(s, v.members, i)?;
            let (items, mut used) = self.list(list)?;
            used.push(li);
            let mut cs = Vec::new();
            for m in items {
                let (c, cu) = self.class(m, li)?;
                used.extend(cu);
                cs.push(c);
            }
            self.arity(cs.len(), 2, "AllDisjointClasses", li)?;
            self.only_these(s, i, &used)?;
            self.emit(i, Axiom::DisjointClasses(cs), [used]);
            return Ok(());
        }
        if o != NO_ID && o == v.all_disjoint_properties {
            let (list, li) = self.required(s, v.members, i)?;
            let (items, mut used) = self.list(list)?;
            used.push(li);
            self.arity(items.len(), 2, "AllDisjointProperties", li)?;
            let data = items
                .first()
                .map(|&m| self.kind_of(m, li))
                .transpose()?
                .flatten()
                == Some(PropKind::Data);
            let ax = if data {
                let mut ds = Vec::new();
                for m in items {
                    ds.push(self.data_property(m, li)?);
                }
                Axiom::DisjointDataProperties(ds)
            } else {
                let mut ops = Vec::new();
                for m in items {
                    let (op, ou) = self.object_property(m, li)?;
                    used.extend(ou);
                    ops.push(op);
                }
                Axiom::DisjointObjectProperties(ops)
            };
            self.only_these(s, i, &used)?;
            self.emit(i, ax, [used]);
            return Ok(());
        }
        if o != NO_ID && o == v.all_different {
            let found = match self.sp_one(s, v.members)? {
                Some(x) => Some(x),
                None => self.sp_one(s, v.distinct_members)?,
            };
            let Some((list, li)) = found else {
                return Err(self.err(
                    ExtractErrorKind::Malformed,
                    "owl:AllDifferent needs owl:members or owl:distinctMembers",
                    &[i],
                ));
            };
            let (items, mut used) = self.list(list)?;
            used.push(li);
            let mut inds = Vec::new();
            for m in items {
                inds.push(self.individual(m, li)?);
            }
            self.arity(inds.len(), 2, "AllDifferent", li)?;
            self.only_these(s, i, &used)?;
            self.emit(i, Axiom::DifferentIndividuals(inds), [used]);
            return Ok(());
        }
        if o != NO_ID && o == v.negative_property_assertion {
            return self.negative_assertion(i);
        }
        // Ignorable typings.
        if o == v.rdf_property
            || o == v.deprecated_property
            || o == v.rdf_list
            || (o == v.owl_class && is_blank(self.dict, s))
        {
            self.consume(i);
            return Ok(());
        }
        // A class assertion.
        let ind = self.individual(s, i)?;
        let (c, cu) = self.class(o, i)?;
        self.emit(i, Axiom::ClassAssertion(c, ind), [cu]);
        Ok(())
    }

    fn required(&self, s: Id, p: Id, at: usize) -> Result<(Id, usize)> {
        self.sp_one(s, p)?.ok_or_else(|| {
            self.err(
                ExtractErrorKind::Malformed,
                "a required structural triple is missing",
                &[at],
            )
        })
    }

    /// An n-ary axiom node may carry only its typing and member triples (annotations of
    /// the axiom are allowed and carry no logic).
    fn only_these(&mut self, s: Id, typing: usize, used: &[usize]) -> Result<()> {
        for j in self.by_subject.get(&s).cloned().unwrap_or_default() {
            if j == typing || used.contains(&j) {
                continue;
            }
            let p = self.triples[j][1];
            if self.kinds(p) == [PropKind::Annotation] {
                self.consume(j);
                continue;
            }
            return Err(self.err(
                ExtractErrorKind::Unparsed,
                "an unexpected triple on an n-ary axiom node",
                &[j],
            ));
        }
        Ok(())
    }

    fn negative_assertion(&mut self, i: usize) -> Result<()> {
        let v = self.v.clone();
        let [s, _, _] = self.triples[i];
        let (src, si) = self.required(s, v.source_individual, i)?;
        let (prop, pi) = self.required(s, v.assertion_property, i)?;
        let target_ind = self.sp_one(s, v.target_individual)?;
        let target_val = self.sp_one(s, v.target_value)?;
        let a = self.individual(src, si)?;
        let mut used = vec![si, pi];
        let ax = match (target_ind, target_val) {
            (Some((t, ti)), None) => {
                used.push(ti);
                let (op, ou) = self.object_property(prop, pi)?;
                used.extend(ou);
                let b = self.individual(t, ti)?;
                Axiom::NegativeObjectPropertyAssertion(op, a, b)
            }
            (None, Some((t, ti))) => {
                used.push(ti);
                let d = self.data_property(prop, pi)?;
                let lit = self.literal(t, ti)?;
                Axiom::NegativeDataPropertyAssertion(d, a, lit)
            }
            _ => {
                return Err(self.err(
                    ExtractErrorKind::Malformed,
                    "a negative property assertion needs exactly one of owl:targetIndividual / owl:targetValue",
                    &[i],
                ))
            }
        };
        for &u in &used {
            self.consume(u);
        }
        self.only_these(s, i, &used)?;
        self.emit(i, ax, [used]);
        Ok(())
    }

    fn assertion(&mut self, i: usize) -> Result<()> {
        let [s, p, o] = self.triples[i];
        if !self.is_assertion_predicate(p) {
            return Err(self.err(
                ExtractErrorKind::Unparsed,
                "no OWL 2 mapping rule reads this triple",
                &[i],
            ));
        }
        match self.kind_of(p, i)? {
            Some(PropKind::Annotation) => {
                let subject = if is_blank(self.dict, s) {
                    AnnotationSubject::Anonymous(s)
                } else {
                    AnnotationSubject::Iri(s)
                };
                let value = self.annotation_value(o, i)?;
                self.emit(i, Axiom::AnnotationAssertion(p, subject, value), []);
            }
            Some(PropKind::Object) => {
                let a = self.individual(s, i)?;
                let b = self.individual(o, i)?;
                self.emit(
                    i,
                    Axiom::ObjectPropertyAssertion(ObjectProperty::Named(p), a, b),
                    [],
                );
            }
            Some(PropKind::Data) => {
                let a = self.individual(s, i)?;
                let lit = self.literal(o, i)?;
                self.emit(i, Axiom::DataPropertyAssertion(p, a, lit), []);
            }
            None => {
                return Err(self.err(
                    ExtractErrorKind::Ambiguous,
                    "the predicate's kind is undeclared",
                    &[i],
                ))
            }
        }
        Ok(())
    }

    // ---------------------------------------------------------------------------------
    // pass 4: the fail-closed check
    // ---------------------------------------------------------------------------------

    fn finish(mut self) -> Result<Ontology> {
        if let Some(i) = self.consumed.iter().position(|c| !c) {
            let [s, _, _] = self.triples[i];
            let detail = if self.structural.contains(&s) {
                "a structural triple (class expression, data range or list) that no axiom uses"
            } else {
                "no OWL 2 mapping rule reads this triple"
            };
            return Err(self.err(ExtractErrorKind::Unparsed, detail, &[i]));
        }
        // Deterministic order: by first source triple.
        self.out
            .axioms
            .sort_by(|a, b| a.source.first().cmp(&b.source.first()));
        Ok(self.out)
    }
}

/// The node count and height of a class expression (bounded by [`MAX_DEPTH`] when built).
fn class_size(ce: &ClassExpression) -> (usize, usize) {
    use ClassExpression as C;
    let kids = |cs: &[ClassExpression]| {
        cs.iter()
            .map(class_size)
            .fold((0, 0), |(n, h), (cn, ch)| (n + cn, h.max(ch)))
    };
    let (n, h) = match ce {
        C::IntersectionOf(cs) | C::UnionOf(cs) => kids(cs),
        C::ComplementOf(c)
        | C::SomeValuesFrom(_, c)
        | C::AllValuesFrom(_, c)
        | C::MinCardinality(_, _, c)
        | C::MaxCardinality(_, _, c)
        | C::ExactCardinality(_, _, c) => class_size(c),
        C::OneOf(is) => (is.len(), 0),
        C::DataSomeValuesFrom(_, r)
        | C::DataAllValuesFrom(_, r)
        | C::DataMinCardinality(_, _, r)
        | C::DataMaxCardinality(_, _, r)
        | C::DataExactCardinality(_, _, r) => range_size(r),
        C::Class(_)
        | C::Thing
        | C::Nothing
        | C::HasValue(..)
        | C::HasSelf(_)
        | C::DataHasValue(..) => (0, 0),
    };
    (n + 1, h + 1)
}

/// The node count and height of a data range.
fn range_size(dr: &DataRange) -> (usize, usize) {
    let (n, h) = match dr {
        DataRange::IntersectionOf(rs) | DataRange::UnionOf(rs) => rs
            .iter()
            .map(range_size)
            .fold((0, 0), |(n, h), (cn, ch)| (n + cn, h.max(ch))),
        DataRange::ComplementOf(r) => range_size(r),
        DataRange::OneOf(ls) => (ls.len(), 0),
        DataRange::Restriction(_, fs) => (fs.len(), 0),
        DataRange::TopLiteral | DataRange::Datatype(_) => (0, 0),
    };
    (n + 1, h + 1)
}
