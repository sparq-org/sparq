// [OPUS-4.8] sq-t5bne (epic sq-pbz04): the DL-Lite_R (OWL 2 QL) TBox model + extraction.
// [SONNET-4.6] sq-pbz04.3.3: total TBox-capture accounting — equivalentClass/equivalentProperty
// captured as inclusion pairs; consistency_relevant + unrecognised_schema tallies; fully_captured().
//
// OWL 2 QL is the syntactic restriction of OWL 2 whose DL counterpart is DL-Lite_R (Artale et
// al., JAIR 2009; W3C OWL 2 Profiles §3). The POSITIVE inclusions PerfectRef rewrites with are:
//
//   B ⊑ A            a "basic concept" B on the left of a named class A
//   R ⊑ S            role inclusion (incl. inverses, R ⊑ S⁻ etc.)
//
// where a basic concept B is one of:  A (named class) | ∃R | ∃R⁻  and a basic role R is a named
// property P or its inverse P⁻. NEGATIVE inclusions (B1 ⊑ ¬B2, disjointness, R1 ⊑ ¬R2) and
// functionality do NOT contribute to the certain-answer REWRITING of a positive CQ — they only
// affect consistency — so they are extracted but recorded as ignored-for-rewriting, never
// silently treated as positive (which would be unsound). The REWRITER never applies them; the
// opt-in `ql-consistency` check (sq-p6yb7, src/consistency.rs) consumes the structurally
// captured ones (`neg_incl`) to decide KB satisfiability by violation-query composition.
//
// Extraction reads OWL axioms out of an RDF graph given as oxrdf triples. Only the QL fragment
// is recognised; an axiom using a non-QL construct is COUNTED in `TBox::skipped` (the honest
// "N axioms outside the QL fragment were ignored" count), never misapplied.
//
// INVARIANT (sq-pbz04.3.3): NO silently-ignored rdfs:/owl: triple — every schema predicate in
// the rdfs:/owl: vocabulary is EITHER captured (positive inclusion), counted in `skipped` (non-QL
// axiom), counted in `consistency_relevant` (negative/disjointness axiom), OR classified as a
// known restriction sub-predicate or annotation predicate. Anything else goes to
// `unrecognised_schema`, which makes `fully_captured()` return false.

#[cfg(feature = "experimental")]
use oxrdf::NamedNode;
use oxrdf::{NamedOrBlankNode, Term, Triple};
use rustc_hash::{FxHashMap, FxHashSet};

/// A subject/object node reduced to the only two shapes TBox extraction cares about: a named
/// IRI or a blank node (literals are neither). `from_subject`/`from_object` normalise the two
/// distinct oxrdf position types (`NamedOrBlankNode` subject, `Term` object) into this.
#[derive(Clone, Debug)]
enum NodeKind {
    Iri(String),
    Blank(String),
}

impl NodeKind {
    fn from_subject(s: &NamedOrBlankNode) -> NodeKind {
        match s {
            NamedOrBlankNode::NamedNode(n) => NodeKind::Iri(n.as_str().to_string()),
            NamedOrBlankNode::BlankNode(b) => NodeKind::Blank(b.as_str().to_string()),
        }
    }

    fn from_object(o: &Term) -> Option<NodeKind> {
        match o {
            Term::NamedNode(n) => Some(NodeKind::Iri(n.as_str().to_string())),
            Term::BlankNode(b) => Some(NodeKind::Blank(b.as_str().to_string())),
            _ => None, // literal (or quoted triple) — never a TBox class/role node.
        }
    }
}

/// `rdf:type` — used by the (experimental) query-atom mapper to recognise class atoms.
#[cfg(feature = "experimental")]
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const RDFS: &str = "http://www.w3.org/2000/01/rdf-schema#";
const OWL: &str = "http://www.w3.org/2002/07/owl#";

/// OWL 2 characteristic property types — when used as the OBJECT of an `rdf:type` triple,
/// these signal a non-QL axiom (functionality, transitivity, symmetry, …). Each occurrence is
/// counted into `TBox::unrecognised_schema`. The predicate is `rdf:type` (not `rdfs:`/`owl:`),
/// so `is_unrecognised_schema(p)` (which checks the predicate namespace) cannot catch these;
/// the `rdf:type` extraction arm handles them by object-IRI inspection. [SONNET-4.6] sq-pbz04.3.3
const OWL_CHARACTERISTIC_TYPES: &[&str] = &[
    "FunctionalProperty",
    "InverseFunctionalProperty",
    "TransitiveProperty",
    "SymmetricProperty",
    "AsymmetricProperty",
    "ReflexiveProperty",
    "IrreflexiveProperty",
];

/// OWL 2 QL-legal `rdf:type` objects — entity-kind declarations that are metadata, not axioms.
/// These are silently ignored (not counted in any tally). A purely QL-legal TBox that carries
/// only these declarations (plus the recognised schema predicates) still yields
/// `fully_captured() == true`. [SONNET-4.6] sq-pbz04.3.3
const OWL_LEGAL_DECL_TYPES: &[&str] = &[
    "Class",
    "ObjectProperty",
    "DatatypeProperty",
    "NamedIndividual",
    "Ontology",
];

/// A basic role: a named property, or its inverse. Equality/ordering is by IRI + direction so
/// roles intern cleanly into the rewrite's working sets.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Role {
    /// The named property IRI.
    pub iri: String,
    /// `true` if this is the INVERSE role `P⁻` (object/subject swapped).
    pub inverse: bool,
}

impl Role {
    pub fn named(iri: impl Into<String>) -> Self {
        Role {
            iri: iri.into(),
            inverse: false,
        }
    }

    /// The inverse of this role (`P` ↔ `P⁻`).
    pub fn inv(&self) -> Role {
        Role {
            iri: self.iri.clone(),
            inverse: !self.inverse,
        }
    }
}

/// A "basic concept" B in DL-Lite_R: a named class, or an unqualified existential `∃R` over a
/// basic role (the domain of R). `∃R⁻` is `Exists` over the inverse role. (DL-Lite_R existentials
/// are UNqualified — `∃R.A` qualified fillers are NOT in QL, and are skipped in extraction.)
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Basic {
    /// A named class `A`.
    Class(String),
    /// `∃R` — the domain of basic role `R` (something R-related).
    Exists(Role),
}

/// A positive concept inclusion `B ⊑ A` (left a basic concept, right a NAMED class). The right
/// side being a named class is the shape PerfectRef rewrites a class atom with; QL also allows
/// `B ⊑ ∃R` on the right (range/existential generation), captured separately as
/// [`TBox::exists_super`] so the rewrite can apply it to ROLE atoms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConceptInclusion {
    pub sub: Basic,
    pub sup: String,
}

/// A DL-Lite_R NEGATIVE inclusion, STRUCTURALLY captured (not just tallied) so the opt-in
/// `ql-consistency` violation-query check can compose a violation CQ per axiom. [FABLE-5]
/// sq-p6yb7.
///
/// - `Concept(B1, B2)` is `B1 ⊑ ¬B2` — from `owl:disjointWith` where BOTH operands resolve to
///   QL basic concepts (a named class, or an unqualified `∃R` restriction node), or from the
///   subClassOf-complement shape `B1 rdfs:subClassOf [ owl:complementOf B2 ]` (sq-fj8lj follow-up;
///   see [`TBox::extract`]).
/// - `Role(R1, R2)` is `R1 ⊑ ¬R2` — from `owl:propertyDisjointWith` between named properties.
///
/// A negative axiom whose operands do NOT resolve (a complex class expression, a poisoned /
/// qualified restriction, or a NAMED-subject `owl:complementOf` triple — `A owl:complementOf B`
/// asserts the EQUIVALENCE `A ≡ ¬B`, strictly stronger than the negative inclusion `A ⊑ ¬B`, and
/// its `¬B ⊑ A` half is not a DL-Lite_R negative inclusion) is counted in
/// `TBox::consistency_uncaptured` instead, which keeps every consistency verdict fail-closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NegativeInclusion {
    /// `B1 ⊑ ¬B2` (concept disjointness).
    Concept(Basic, Basic),
    /// `R1 ⊑ ¬R2` (role disjointness).
    Role(Role, Role),
}

impl std::fmt::Display for NegativeInclusion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fn basic(b: &Basic) -> String {
            match b {
                Basic::Class(c) => format!("<{c}>"),
                Basic::Exists(r) => format!("∃{}", role(r)),
            }
        }
        fn role(r: &Role) -> String {
            if r.inverse {
                format!("<{}>⁻", r.iri)
            } else {
                format!("<{}>", r.iri)
            }
        }
        match self {
            NegativeInclusion::Concept(b1, b2) => {
                write!(f, "{} ⊑ ¬{}", basic(b1), basic(b2))
            }
            NegativeInclusion::Role(r1, r2) => write!(f, "{} ⊑ ¬{}", role(r1), role(r2)),
        }
    }
}

/// The extracted DL-Lite_R TBox: positive inclusions PerfectRef needs, plus an honest tally of
/// what was outside the QL fragment, what was consistency-relevant, and what was unrecognised.
///
/// INVARIANT (sq-pbz04.3.3): every `rdfs:`/`owl:` predicate triple is classified into one of:
/// captured (adds to `concept_incl` / `exists_super` / `role_incl`), `skipped` (non-QL axiom),
/// `consistency_relevant` (negative/disjointness axiom), or a known restriction sub-predicate /
/// annotation predicate (silently ignored). Anything else increments `unrecognised_schema`.
/// Callers can therefore trust that `skipped == 0 && unrecognised_schema == 0` (i.e.
/// `fully_captured()`) is a decisive "nothing was missed" signal. [SONNET-4.6]
#[derive(Clone, Debug, Default)]
pub struct TBox {
    /// `B ⊑ A` with `A` a named class.
    pub concept_incl: Vec<ConceptInclusion>,
    /// `B ⊑ ∃R` (the existential-generating inclusions: a basic concept implies an R-successor).
    /// Keyed for the role-atom rewrite. Stored as `(sub basic concept, generated role)`.
    pub exists_super: Vec<(Basic, Role)>,
    /// Role inclusions `R ⊑ S` (both basic roles; inverses included).
    pub role_incl: Vec<(Role, Role)>,
    /// Count of axioms that used a construct OUTSIDE DL-Lite_R (and were therefore IGNORED for
    /// rewriting): qualified existentials, unionOf/intersection on the wrong side, cardinality,
    /// nominals, datatype constructs, `owl:equivalentClass` / `owl:equivalentProperty` with a
    /// complex-expression object (not a named class/property), … Surfaced so a caller can report
    /// "n axioms outside the QL fragment were ignored". Never counts annotation triples or
    /// restriction sub-predicate triples (those are silently ignored as expected). See also
    /// `consistency_relevant` and `unrecognised_schema`.
    pub skipped: usize,
    /// Count of QL-legal NEGATIVE axioms: one per `owl:disjointWith` /
    /// `owl:propertyDisjointWith` triple, one per subClassOf-complement axiom
    /// (`A rdfs:subClassOf [ owl:complementOf B ]` — counted at the `rdfs:subClassOf` triple, so
    /// a complement blank node shared by two subClassOf triples counts TWICE and its defining
    /// `owl:complementOf` triple counts zero), and one per `owl:complementOf` triple that does
    /// NOT define a consumed complement node (a named-subject `A owl:complementOf B`, an orphan,
    /// or a malformed complement blank node). These do NOT contribute to the certain-answer
    /// rewriting but ARE consistency-relevant. The REWRITER never applies them;
    /// a non-zero count signals that the UCQ answers may under-approximate the certain answers
    /// when the KB is inconsistent (an inconsistent KB certain-answers everything) — unless the
    /// opt-in `ql-consistency` check (`check_consistency`, sq-p6yb7) proves the KB consistent.
    /// Surfaced SEPARATELY from `skipped` so callers can reason about the consistency gap
    /// independently of the "axiom outside QL" gap. [SONNET-4.6]
    ///
    /// INVARIANT (sq-p6yb7): `consistency_relevant == neg_incl.len() + consistency_uncaptured`
    /// — every tallied negative axiom is either structurally captured or counted uncaptured.
    pub consistency_relevant: usize,
    /// The negative inclusions STRUCTURALLY captured from `owl:disjointWith` (both operands
    /// resolved to basic concepts), `owl:propertyDisjointWith` (both operands named
    /// properties), and the subClassOf-complement shape `B1 rdfs:subClassOf [ owl:complementOf
    /// B2 ]` (both operands resolved to basic concepts). Input to the opt-in `ql-consistency`
    /// violation-query composition; never applied by the rewriter. [FABLE-5] sq-p6yb7
    pub neg_incl: Vec<NegativeInclusion>,
    /// Count of consistency-relevant axioms that could NOT be structurally captured into
    /// `neg_incl`: every NAMED-subject `owl:complementOf` triple (an
    /// equivalence-with-complement, strictly stronger than a negative inclusion — see
    /// [`NegativeInclusion`]), every orphan/malformed complement blank node, and any
    /// `owl:disjointWith` / `owl:propertyDisjointWith` / subClassOf-complement whose operand is
    /// not a resolvable basic concept / named property. Non-zero means a consistency check can
    /// still soundly report INCONSISTENT (logic is monotonic) but must never report CONSISTENT
    /// (fail-closed). [FABLE-5] sq-p6yb7
    pub consistency_uncaptured: usize,
    /// Count of OWL/RDFS constructs this extractor does not recognise, tallied through two
    /// distinct paths: (1) triples whose *predicate* is in the `rdfs:`/`owl:` namespace and is
    /// not captured, skipped, consistency-relevant, or annotation-classified; and (2) `rdf:type`
    /// triples whose *object* is schema vocabulary this implementation does not model (e.g.
    /// `:p rdf:type owl:FunctionalProperty` — a non-QL characteristic-property axiom). A
    /// non-zero count means the TBox carries OWL/RDFS constructs beyond what this
    /// implementation models; `fully_captured()` returns `false` in that case. [SONNET-4.6]
    pub unrecognised_schema: usize,
}

impl TBox {
    /// Extract the DL-Lite_R positive inclusions from an RDF graph (oxrdf triples). Recognised:
    ///
    /// - `A rdfs:subClassOf B` with A,B named classes → `A ⊑ B`;
    /// - `R rdfs:subPropertyOf S` → `R ⊑ S`;
    /// - `R rdfs:domain A` → `∃R ⊑ A`;
    /// - `R rdfs:range A` → `∃R⁻ ⊑ A`;
    /// - `R owl:inverseOf S` → `R ⊑ S⁻` and `S⁻ ⊑ R` (bi-directional, as DL-Lite_R inverses);
    /// - `A owl:equivalentClass B` with both named classes → `A ⊑ B` AND `B ⊑ A` (inclusion
    ///   pair); a complex-expression (blank-node) side goes to `skipped`;
    /// - `R owl:equivalentProperty S` with both named properties → `R ⊑ S` AND `S ⊑ R`;
    /// - `A rdfs:subClassOf [ owl:onProperty R ; owl:someValuesFrom owl:Thing ]` → `A ⊑ ∃R`
    ///   (UNqualified existential — a QL-legal range generator);
    /// - `[ owl:onProperty R ; owl:someValuesFrom owl:Thing ] rdfs:subClassOf A` → `∃R ⊑ A`.
    ///
    /// `owl:disjointWith`, `owl:propertyDisjointWith`, and `owl:complementOf` are counted in
    /// `consistency_relevant` (QL-legal but not used for rewriting — see `TBox::consistency_relevant`).
    ///
    /// The **subClassOf-complement** shape `B1 rdfs:subClassOf [ owl:complementOf B2 ]` is the
    /// OWL 2 QL `superClassExpression ::= ObjectComplementOf(subClassExpression)` production —
    /// exactly the DL-Lite_R negative inclusion `B1 ⊑ ¬B2`. Both operands resolve through the
    /// same basic-concept resolution as `owl:disjointWith` (a named class, or an UNqualified
    /// `∃R` restriction node), so `A ⊑ ¬B`, `∃R ⊑ ¬B`, and `A ⊑ ¬∃R` are all captured into
    /// `neg_incl`; an unresolvable operand goes to `consistency_uncaptured`. The complement
    /// blank node's own `owl:complementOf` triple is then NOT tallied separately (its axiom
    /// content is accounted at each `rdfs:subClassOf` triple that consumes it) — see
    /// `TBox::consistency_relevant`. A NAMED-subject `A owl:complementOf B` still asserts the
    /// biconditional `A ≡ ¬B` and stays `consistency_uncaptured`, unchanged.
    ///
    /// `rdf:type` triples whose OBJECT is one of the 7 OWL characteristic property classes
    /// (`owl:FunctionalProperty`, `owl:InverseFunctionalProperty`, `owl:TransitiveProperty`,
    /// `owl:SymmetricProperty`, `owl:AsymmetricProperty`, `owl:ReflexiveProperty`,
    /// `owl:IrreflexiveProperty`) are counted in `unrecognised_schema` — they are non-QL axioms
    /// that cannot be applied in certain-answer rewriting. `rdf:type` declarations that are
    /// QL-legal entity-kind metadata (`owl:Class`, `owl:ObjectProperty`, `owl:DatatypeProperty`,
    /// `owl:NamedIndividual`, `owl:Ontology`, `rdfs:Class`, `rdf:Property`) are silently ignored
    /// (not schema axioms). [SONNET-4.6] sq-pbz04.3.3
    ///
    /// A QUALIFIED `owl:someValuesFrom` (filler other than `owl:Thing`) on the LEFT is QL-legal
    /// (`∃R.C ⊑ A` is in QL via `∃R ⊑ A` weakening? — NO: that would be unsound), so it is
    /// conservatively SKIPPED. Anything else (unionOf, allValuesFrom, cardinality, hasValue,
    /// oneOf, qualified RHS existentials) is skipped and counted. Any `rdfs:`/`owl:`-vocabulary
    /// predicate not handled above and not a known restriction sub-predicate or annotation goes
    /// to `unrecognised_schema`. [SONNET-4.6]
    pub fn extract(triples: &[Triple]) -> TBox {
        let mut tbox = TBox::default();
        // Index restriction blank-nodes: subject -> (onProperty, someValuesFrom-filler).
        let restrictions = index_restrictions(triples);
        // Index complement blank-nodes: subject -> complemented node. Only nodes CONSUMED by an
        // `rdfs:subClassOf` RHS are here (see `index_complements`), which is exactly the set whose
        // tally moves from the `owl:complementOf` triple to the consuming subClassOf axiom.
        let complements = index_complements(triples);

        for t in triples {
            let p = t.predicate.as_str();
            match p {
                _ if p == sub_class_of() => {
                    tbox.add_subclass(&t.subject, &t.object, &restrictions, &complements);
                }
                _ if p == sub_property_of() => {
                    if let (Some(r), Some(s)) =
                        (role_of_subject(&t.subject), role_of_object(&t.object))
                    {
                        tbox.role_incl.push((r, s));
                    } else {
                        tbox.skipped += 1;
                    }
                }
                _ if p == domain() => {
                    if let (Some(r), Some(NodeKind::Iri(a))) = (
                        role_of_subject(&t.subject),
                        NodeKind::from_object(&t.object),
                    ) {
                        // ∃R ⊑ A
                        tbox.concept_incl.push(ConceptInclusion {
                            sub: Basic::Exists(r),
                            sup: a,
                        });
                    } else {
                        tbox.skipped += 1;
                    }
                }
                _ if p == range() => {
                    if let (Some(r), Some(NodeKind::Iri(a))) = (
                        role_of_subject(&t.subject),
                        NodeKind::from_object(&t.object),
                    ) {
                        // ∃R⁻ ⊑ A
                        tbox.concept_incl.push(ConceptInclusion {
                            sub: Basic::Exists(r.inv()),
                            sup: a,
                        });
                    } else {
                        tbox.skipped += 1;
                    }
                }
                _ if p == inverse_of() => {
                    if let (Some(r), Some(s)) =
                        (role_of_subject(&t.subject), role_of_object(&t.object))
                    {
                        // R ≡ S⁻ : add both role inclusions.
                        tbox.role_incl.push((r.clone(), s.inv()));
                        tbox.role_incl.push((s.inv(), r));
                    } else {
                        tbox.skipped += 1;
                    }
                }
                // [SONNET-4.6] sq-pbz04.3.3 — equivalentClass: A ≡ B decomposes to A ⊑ B AND
                // B ⊑ A (both directions). QL-legal with named-class operands only; a blank-node
                // complex expression on either side → skipped (outside QL rewriting scope).
                _ if p == equivalent_class() => {
                    match (
                        NodeKind::from_subject(&t.subject),
                        NodeKind::from_object(&t.object),
                    ) {
                        (NodeKind::Iri(a), Some(NodeKind::Iri(b))) => {
                            tbox.concept_incl.push(ConceptInclusion {
                                sub: Basic::Class(a.clone()),
                                sup: b.clone(),
                            });
                            tbox.concept_incl.push(ConceptInclusion {
                                sub: Basic::Class(b),
                                sup: a,
                            });
                        }
                        _ => tbox.skipped += 1,
                    }
                }
                // [SONNET-4.6] sq-pbz04.3.3 — equivalentProperty: R ≡ S decomposes to R ⊑ S
                // AND S ⊑ R. Named roles only; blank-node or non-role → skipped.
                _ if p == equivalent_property() => {
                    match (role_of_subject(&t.subject), role_of_object(&t.object)) {
                        (Some(r), Some(s)) => {
                            tbox.role_incl.push((r.clone(), s.clone()));
                            tbox.role_incl.push((s, r));
                        }
                        _ => tbox.skipped += 1,
                    }
                }
                // [SONNET-4.6] sq-pbz04.3.3 — negative/disjointness axioms: QL-legal but
                // consistency-relevant. Counted separately (never applied to the rewrite).
                // [FABLE-5] sq-p6yb7 — additionally STRUCTURALLY captured where both operands
                // resolve to QL basic shapes, so the opt-in `ql-consistency` check can compose
                // a violation query per axiom; unresolvable shapes go to
                // `consistency_uncaptured` (fail-closed for CONSISTENT verdicts).
                _ if p == disjoint_with() => {
                    tbox.consistency_relevant += 1;
                    let b1 = basic_of(&NodeKind::from_subject(&t.subject), &restrictions);
                    let b2 =
                        NodeKind::from_object(&t.object).and_then(|n| basic_of(&n, &restrictions));
                    match (b1, b2) {
                        (Some(b1), Some(b2)) => {
                            tbox.neg_incl.push(NegativeInclusion::Concept(b1, b2));
                        }
                        _ => tbox.consistency_uncaptured += 1,
                    }
                }
                _ if p == property_disjoint_with() => {
                    tbox.consistency_relevant += 1;
                    match (role_of_subject(&t.subject), role_of_object(&t.object)) {
                        (Some(r1), Some(r2)) => {
                            tbox.neg_incl.push(NegativeInclusion::Role(r1, r2));
                        }
                        _ => tbox.consistency_uncaptured += 1,
                    }
                }
                // A NAMED-subject `A owl:complementOf B` asserts A ≡ ¬B — STRICTLY stronger than
                // the negative inclusion A ⊑ ¬B (its ¬B ⊑ A half is not a DL-Lite_R negative
                // inclusion, and treating only the negative half as captured would make a
                // CONSISTENT verdict incomplete). Still uncaptured. [FABLE-5] sq-p6yb7
                //
                // A BLANK-subject complement node that some `rdfs:subClassOf` RHS consumes is an
                // ANONYMOUS class expression, not a named class with an asserted biconditional:
                // it carries no axiom of its own, so it is tallied at each consuming subClassOf
                // axiom instead (`add_subclass`) and contributes nothing here — otherwise one
                // `owl:complementOf` triple would be double-counted, or (shared by two subClassOf
                // triples) under-counted, breaking the `consistency_relevant == neg_incl.len() +
                // consistency_uncaptured` invariant. An orphan or malformed complement blank node
                // is not in `complements` and stays uncaptured here.
                _ if p == complement_of() => {
                    let consumed = matches!(
                        NodeKind::from_subject(&t.subject),
                        NodeKind::Blank(bn) if complements.contains_key(&bn)
                    );
                    if !consumed {
                        tbox.consistency_relevant += 1;
                        tbox.consistency_uncaptured += 1;
                    }
                }
                // [SONNET-4.6] sq-pbz04.3.3 — `rdf:type` triples encode OWL characteristic-
                // property axioms via the OBJECT position (e.g. `:p rdf:type
                // owl:FunctionalProperty`). The predicate `rdf:type` is in the `rdf:` namespace,
                // so `is_unrecognised_schema(p)` returns false and these would be silently
                // dropped. We inspect the OBJECT IRI to classify:
                //   • one of the 7 non-QL characteristic types → `unrecognised_schema += 1`
                //   • QL-legal entity-kind declarations → silently ignore (metadata, not axioms)
                //   • other `owl:`/`rdfs:` object → `unrecognised_schema += 1` (unknown vocab)
                //   • non-`owl:`/`rdfs:` object → silently ignore (ABox or custom IRI)
                _ if p == rdf_type_str() => {
                    if let Some(NodeKind::Iri(obj)) = NodeKind::from_object(&t.object) {
                        // Use strip_prefix + slice::contains to avoid per-iteration String
                        // allocation — allocation-free membership test. [SONNET-4.6]
                        if let Some(local) = obj.strip_prefix(OWL) {
                            if OWL_CHARACTERISTIC_TYPES.contains(&local) {
                                // Non-QL property characteristic: not applicable for rewriting.
                                tbox.unrecognised_schema += 1;
                            } else if !OWL_LEGAL_DECL_TYPES.contains(&local) {
                                // Unknown owl: object in rdf:type: count as unrecognised.
                                tbox.unrecognised_schema += 1;
                            }
                            // else: QL-legal owl: declaration — silently ignore (not a TBox axiom).
                        } else if obj == "http://www.w3.org/2000/01/rdf-schema#Class"
                            || obj == "http://www.w3.org/1999/02/22-rdf-syntax-ns#Property"
                        {
                            // QL-legal rdfs:/rdf: declaration — silently ignore (not a TBox axiom).
                        } else if obj.starts_with(RDFS) {
                            // Unknown rdfs: object in rdf:type: count as unrecognised.
                            tbox.unrecognised_schema += 1;
                        }
                        // else: non-owl/rdfs object (custom class or rdf: vocab) — ignore.
                    }
                    // Literal or blank-node rdf:type objects are not TBox targets — ignore.
                }
                _ => {
                    // Not a TBox-shaping predicate recognised above. If the predicate is in the
                    // rdfs:/owl: vocabulary namespace and is NOT a known restriction sub-predicate
                    // or annotation predicate, count it as unrecognised schema vocabulary.
                    // [SONNET-4.6] sq-pbz04.3.3
                    if is_unrecognised_schema(p) {
                        tbox.unrecognised_schema += 1;
                    }
                    // Otherwise: ABox triple, rdf: predicate, or known annotation — silently
                    // ignored (expected, not a gap in TBox accounting).
                }
            }
        }
        tbox
    }

    /// Returns `true` if and only if `self.skipped == 0 && self.unrecognised_schema == 0`.
    /// `skipped` counts axioms outside the QL fragment; `unrecognised_schema` counts any
    /// OWL/RDFS construct the extractor did not classify — whether expressed as an
    /// `rdfs:`/`owl:`-predicate triple or as an `rdf:type` declaration of unmodelled schema
    /// vocabulary (e.g. `:p rdf:type owl:FunctionalProperty`). Consistency-relevant axioms
    /// (`owl:disjointWith` etc.) are NOT counted against full capture — they are QL-legal;
    /// the crate's documented boundary is that it does not CHECK consistency, not that it
    /// misses their presence. [SONNET-4.6]
    pub fn fully_captured(&self) -> bool {
        self.skipped == 0 && self.unrecognised_schema == 0
    }

    /// Add an `A ⊑ B`-derived inclusion, resolving restriction blank-nodes on either side into
    /// the QL basic-concept forms. Non-QL shapes are skipped + counted. A complement blank node
    /// on the RHS is the QL NEGATIVE inclusion `B1 ⊑ ¬B2` — captured into `neg_incl`, never a
    /// positive inclusion, and tallied in `consistency_relevant` (never in `skipped`: the shape
    /// is QL-legal).
    fn add_subclass(
        &mut self,
        sub: &NamedOrBlankNode,
        sup: &Term,
        restrictions: &FxHashMap<String, Restriction>,
        complements: &FxHashMap<String, NodeKind>,
    ) {
        let sub_b = basic_of(&NodeKind::from_subject(sub), restrictions);
        // Right side: a named class A (the PerfectRef class-atom rewrite target), an
        // unqualified ∃R (range generator → exists_super), or ¬B (negative inclusion).
        match NodeKind::from_object(sup) {
            Some(NodeKind::Iri(a)) => {
                if let Some(b) = sub_b {
                    self.concept_incl.push(ConceptInclusion { sub: b, sup: a });
                } else {
                    self.skipped += 1;
                }
            }
            Some(NodeKind::Blank(bn)) => {
                // `B1 ⊑ ¬B2` — QL-legal negative inclusion, so it is consistency-relevant, never
                // `skipped`. `index_complements` guarantees a complement node is not also a
                // restriction node, so this arm and the restriction arm below are disjoint.
                if let Some(complemented) = complements.get(&bn) {
                    self.consistency_relevant += 1;
                    match (sub_b, basic_of(complemented, restrictions)) {
                        (Some(b1), Some(b2)) => {
                            self.neg_incl.push(NegativeInclusion::Concept(b1, b2));
                        }
                        _ => self.consistency_uncaptured += 1,
                    }
                    return;
                }
                match restrictions.get(&bn) {
                    Some(Restriction {
                        on_property,
                        some_values_thing: true,
                    }) => {
                        // B ⊑ ∃R (unqualified existential on the RHS).
                        if let Some(b) = sub_b {
                            self.exists_super
                                .push((b, Role::named(on_property.clone())));
                        } else {
                            self.skipped += 1;
                        }
                    }
                    // Qualified RHS existential or non-someValues restriction: outside QL rewriting.
                    _ => self.skipped += 1,
                }
            }
            None => self.skipped += 1,
        }
    }
}

/// Map a subclass-side node to a basic concept, resolving a restriction blank-node into `∃R`.
/// Returns `None` (→ skip) for any non-QL shape.
fn basic_of(t: &NodeKind, restrictions: &FxHashMap<String, Restriction>) -> Option<Basic> {
    match t {
        NodeKind::Iri(a) => Some(Basic::Class(a.clone())),
        NodeKind::Blank(bn) => match restrictions.get(bn) {
            // ∃R on the LEFT requires the UNQUALIFIED form (someValuesFrom owl:Thing). A
            // QUALIFIED ∃R.C on the left is NOT equivalent to ∃R ⊑ … (weakening it would be
            // unsound), so it is skipped.
            Some(Restriction {
                on_property,
                some_values_thing: true,
            }) => Some(Basic::Exists(Role::named(on_property.clone()))),
            _ => None,
        },
    }
}

/// A parsed owl:Restriction blank-node: `[ owl:onProperty R ; owl:someValuesFrom F ]`.
struct Restriction {
    on_property: String,
    /// True iff the someValuesFrom filler is exactly `owl:Thing` (the UNqualified existential).
    some_values_thing: bool,
}

/// Index every owl:Restriction-shaped blank node by its id. We only retain `onProperty` +
/// whether `someValuesFrom` is `owl:Thing`; any restriction lacking a single onProperty, or
/// carrying a non-QL restriction predicate, is simply not indexed (callers then skip it).
fn index_restrictions(triples: &[Triple]) -> FxHashMap<String, Restriction> {
    let mut on_property: FxHashMap<String, String> = FxHashMap::default();
    let mut some_values: FxHashMap<String, bool> = FxHashMap::default();
    // A restriction node carrying any of these predicates is NON-QL — mark it poisoned so it is
    // never resolved into a basic concept even if it also has onProperty.
    let mut poisoned: FxHashSet<String> = FxHashSet::default();
    let owl_thing = format!("{OWL}Thing");

    for t in triples {
        let NamedOrBlankNode::BlankNode(s) = &t.subject else {
            continue;
        };
        let s = s.as_str().to_string();
        let p = t.predicate.as_str();
        if p == format!("{OWL}onProperty") {
            if let Term::NamedNode(r) = &t.object {
                on_property.insert(s, r.as_str().to_string());
            } else {
                poisoned.insert(s);
            }
        } else if p == format!("{OWL}someValuesFrom") {
            match &t.object {
                Term::NamedNode(f) => {
                    some_values.insert(s, f.as_str() == owl_thing);
                }
                _ => {
                    poisoned.insert(s);
                }
            }
        } else if NON_QL_RESTRICTION.iter().any(|m| p == format!("{OWL}{m}")) {
            poisoned.insert(s);
        }
    }

    let mut out = FxHashMap::default();
    for (bn, prop) in on_property {
        if poisoned.contains(&bn) {
            continue;
        }
        // someValuesFrom present? record whether it is owl:Thing. Absent → not a someValues
        // restriction we rewrite (could be allValues/cardinality already poisoned, or a bare
        // onProperty) → treat as NON-unqualified-existential (some_values_thing = false), which
        // makes both the LHS ∃R and the RHS ∃R resolution skip it (sound: we only generate/
        // consume UNqualified existentials).
        let some_values_thing = some_values.get(&bn).copied().unwrap_or(false);
        out.insert(
            bn,
            Restriction {
                on_property: prop,
                some_values_thing,
            },
        );
    }
    out
}

/// Index the complement blank-nodes that an `rdfs:subClassOf` RHS CONSUMES: `bn -> B` for
/// `_:bn owl:complementOf B`, i.e. the anonymous class expression `¬B`, keyed by blank-node id.
///
/// Only nodes that (a) are the object of some `rdfs:subClassOf` triple, (b) carry EXACTLY ONE
/// `owl:complementOf` triple with a non-literal object, and (c) carry NO restriction predicate
/// are indexed. Everything else — a NAMED-subject `owl:complementOf` (the biconditional
/// `A ≡ ¬B`), an orphan complement node asserting no axiom, an ambiguous multi-complement node,
/// and a malformed complement-and-restriction node — is deliberately left OUT, so its
/// `owl:complementOf` triple keeps its conservative `consistency_relevant` +
/// `consistency_uncaptured` tally in `TBox::extract` (fail-closed for `Consistent`).
///
/// The target `B` is returned UNRESOLVED (a `NodeKind`) because resolving it to a `Basic` needs
/// the restriction index — `add_subclass` does that, and counts an unresolvable target as
/// `consistency_uncaptured`.
fn index_complements(triples: &[Triple]) -> FxHashMap<String, NodeKind> {
    let mut candidate: FxHashMap<String, NodeKind> = FxHashMap::default();
    let mut poisoned: FxHashSet<String> = FxHashSet::default();
    let mut consumed: FxHashSet<String> = FxHashSet::default();

    for t in triples {
        // Consumption is a property of the subClassOf RHS, not of the blank node's own triples.
        if t.predicate.as_str() == sub_class_of() {
            if let Term::BlankNode(b) = &t.object {
                consumed.insert(b.as_str().to_string());
            }
        }
        let NamedOrBlankNode::BlankNode(s) = &t.subject else {
            continue;
        };
        let p = t.predicate.as_str();
        if p == complement_of() {
            let s = s.as_str().to_string();
            match NodeKind::from_object(&t.object) {
                Some(n) => {
                    if candidate.insert(s.clone(), n).is_some() {
                        // A second owl:complementOf on the same node: ambiguous, not OWL 2 DL.
                        poisoned.insert(s);
                    }
                }
                // A literal complement filler is not a class expression.
                None => {
                    poisoned.insert(s);
                }
            }
        } else if is_restriction_predicate(p) {
            // Both a restriction and a complement: not a well-formed OWL 2 class expression.
            // Poisoned HERE only — `index_restrictions` is left alone, so such a node keeps its
            // existing (sound) `∃R` reading and its complementOf triple stays uncaptured.
            poisoned.insert(s.as_str().to_string());
        }
    }

    candidate.retain(|bn, _| consumed.contains(bn) && !poisoned.contains(bn));
    candidate
}

/// `true` if `p` is one of the `owl:` restriction sub-predicates — the predicates whose presence
/// makes a blank node a RESTRICTION node rather than a complement node.
fn is_restriction_predicate(p: &str) -> bool {
    p.strip_prefix(OWL)
        .is_some_and(|local| RESTRICTION_SUB_PREDICATES.contains(&local))
}

/// Restriction predicates that, if present on a node, make it non-QL (so the node is poisoned).
const NON_QL_RESTRICTION: &[&str] = &[
    "allValuesFrom",
    "hasValue",
    "minCardinality",
    "maxCardinality",
    "cardinality",
    "minQualifiedCardinality",
    "maxQualifiedCardinality",
    "qualifiedCardinality",
    "hasSelf",
    "onClass",
];

/// `owl:` restriction sub-predicates excluded from the `unrecognised_schema` tally.
///
/// Three groups, each handled differently at the blank-node level:
/// - `onProperty` / `someValuesFrom` — consumed by `index_restrictions` (build the restriction
///   map) and resolved to basic concepts via `add_subclass`.
/// - `allValuesFrom` / `hasValue` / cardinality variants / `hasSelf` / `onClass` — consumed
///   by `index_restrictions` (poison the blank node so the containing `subClassOf` is counted
///   as `skipped`; these shapes are non-QL).
/// - `onDatatype` / `withRestrictions` — NOT consumed by `index_restrictions`; they appear on
///   OWL DL/Full datatype-restriction blank nodes that are never resolved into the restriction
///   map, so any parent `subClassOf` is counted as `skipped` at the axiom level.  The
///   individual sub-predicate triples are silently dropped (expected vocabulary on restriction
///   blank nodes, not an independent gap in accounting).
///
/// None of the entries above are counted as `unrecognised_schema`. [SONNET-4.6] sq-pbz04.3.3
const RESTRICTION_SUB_PREDICATES: &[&str] = &[
    "onProperty",
    "someValuesFrom",
    "onDatatype",
    "withRestrictions",
    // All NON_QL_RESTRICTION entries (those poison the blank node via index_restrictions):
    "allValuesFrom",
    "hasValue",
    "minCardinality",
    "maxCardinality",
    "cardinality",
    "minQualifiedCardinality",
    "maxQualifiedCardinality",
    "qualifiedCardinality",
    "hasSelf",
    "onClass",
];

/// OWL 2 built-in annotation-property local names. Triples whose predicate is `{OWL}{name}`
/// carry human-readable / versioning / import metadata and are NOT TBox axioms — silently
/// ignored (classified-annotation bucket per §3 of the design record), not counted in
/// `unrecognised_schema`. Covers: `owl:imports` (ontology import metadata), `owl:versionIRI`
/// (ontology version identifier), and the standard OWL 2 annotation properties. An ontology
/// carrying any of these as predicates must still yield `fully_captured() == true` when its
/// actual schema axioms are QL-legal. [SONNET-4.6] sq-pbz04.3.3
const ANNOTATION_OWL: &[&str] = &[
    // Ontology-metadata predicates (header triples, not schema axioms):
    "imports",
    "versionIRI",
    // OWL 2 built-in annotation properties (human-readable / versioning):
    "deprecated",
    "versionInfo",
    "priorVersion",
    "backwardCompatibleWith",
    "incompatibleWith",
    // Annotation re-ification sub-predicates (owl:Annotation axiom triples):
    "annotatedSource",
    "annotatedProperty",
    "annotatedTarget",
];

/// RDFS built-in annotation-property local names. Triples whose predicate is `{RDFS}{name}`
/// are not TBox axioms — silently ignored, not counted in `unrecognised_schema`.
/// [SONNET-4.6] sq-pbz04.3.3
const ANNOTATION_RDFS: &[&str] = &["comment", "label", "isDefinedBy", "seeAlso"];

/// Returns `true` if the predicate `p` — which has ALREADY failed to match every explicit TBox
/// extraction arm — is in the `rdfs:`/`owl:` vocabulary namespace AND is NOT a restriction
/// sub-predicate (handled by `index_restrictions`) or a known annotation predicate. These are
/// the triples that contribute to `TBox::unrecognised_schema`. [SONNET-4.6] sq-pbz04.3.3
fn is_unrecognised_schema(p: &str) -> bool {
    // Use strip_prefix + slice::contains to avoid allocating a String per membership test.
    // [SONNET-4.6]
    if let Some(local) = p.strip_prefix(OWL) {
        // Restriction sub-predicates (excluded from unrecognised_schema — see
        // RESTRICTION_SUB_PREDICATES doc) and owl: annotation predicates → not unrecognised.
        !RESTRICTION_SUB_PREDICATES.contains(&local) && !ANNOTATION_OWL.contains(&local)
    } else if let Some(local) = p.strip_prefix(RDFS) {
        // Known rdfs: annotation predicates → not unrecognised.
        !ANNOTATION_RDFS.contains(&local)
    } else {
        // Not in rdfs:/owl: namespace → ABox predicate, not a schema gap.
        false
    }
}

fn role_of_subject(t: &NamedOrBlankNode) -> Option<Role> {
    match t {
        NamedOrBlankNode::NamedNode(n) => Some(Role::named(n.as_str())),
        NamedOrBlankNode::BlankNode(_) => None,
    }
}

fn role_of_object(t: &Term) -> Option<Role> {
    match t {
        Term::NamedNode(n) => Some(Role::named(n.as_str())),
        _ => None,
    }
}

// Vocabulary IRI helpers — return `&'static str` so the per-triple extraction loop incurs no
// heap allocation per comparison. [SONNET-4.6] sq-pbz04.3.3
fn sub_class_of() -> &'static str {
    "http://www.w3.org/2000/01/rdf-schema#subClassOf"
}
fn sub_property_of() -> &'static str {
    "http://www.w3.org/2000/01/rdf-schema#subPropertyOf"
}
fn domain() -> &'static str {
    "http://www.w3.org/2000/01/rdf-schema#domain"
}
fn range() -> &'static str {
    "http://www.w3.org/2000/01/rdf-schema#range"
}
fn inverse_of() -> &'static str {
    "http://www.w3.org/2002/07/owl#inverseOf"
}
fn equivalent_class() -> &'static str {
    "http://www.w3.org/2002/07/owl#equivalentClass"
}
fn equivalent_property() -> &'static str {
    "http://www.w3.org/2002/07/owl#equivalentProperty"
}
fn disjoint_with() -> &'static str {
    "http://www.w3.org/2002/07/owl#disjointWith"
}
fn property_disjoint_with() -> &'static str {
    "http://www.w3.org/2002/07/owl#propertyDisjointWith"
}
fn complement_of() -> &'static str {
    "http://www.w3.org/2002/07/owl#complementOf"
}
/// `rdf:type` IRI as a `&'static str` — used in the `TBox::extract` match arm that classifies
/// characteristic-property typing triples by OBJECT position. [SONNET-4.6] sq-pbz04.3.3
fn rdf_type_str() -> &'static str {
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#type"
}

/// The `rdf:type` IRI (a role-atom predicate that means "class membership", handled specially by
/// the query atom mapper, never as an ordinary role). Only the experimental rewriter needs it.
#[cfg(feature = "experimental")]
pub(crate) fn rdf_type_iri() -> NamedNode {
    NamedNode::new_unchecked(RDF_TYPE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_inverse_roundtrips() {
        let r = Role::named("http://ex/r");
        assert_eq!(r.inv().inv(), r);
        assert!(r.inv().inverse);
    }

    // [SONNET-4.6] sq-pbz04.3.3 — direct unit test for fully_captured() covering both the
    // true-case (clean simple TBox) and false-case (skipped or unrecognised_schema > 0).
    // Required for the per-crate coverage floor (reference-coverage-ratchet-direct-tests).
    #[test]
    fn fully_captured_true_iff_no_skipped_and_no_unrecognised() {
        let mut tbox = TBox::default();
        assert!(tbox.fully_captured(), "empty TBox is fully captured");

        tbox.skipped = 1;
        assert!(!tbox.fully_captured(), "skipped > 0 => not fully captured");

        tbox.skipped = 0;
        tbox.unrecognised_schema = 1;
        assert!(
            !tbox.fully_captured(),
            "unrecognised_schema > 0 => not fully captured"
        );

        tbox.unrecognised_schema = 0;
        tbox.consistency_relevant = 99;
        assert!(
            tbox.fully_captured(),
            "consistency_relevant alone does NOT block fully_captured"
        );
    }

    // [FABLE-5] sq-p6yb7 — structural negative-inclusion capture: disjointWith /
    // propertyDisjointWith resolve into `neg_incl`; complementOf and unresolvable operands go
    // to `consistency_uncaptured`; the tally invariant
    // `consistency_relevant == neg_incl.len() + consistency_uncaptured` holds.
    #[test]
    fn negative_inclusions_structurally_captured_with_tally_invariant() {
        use oxrdf::NamedNode;
        let iri = |s: &str| NamedNode::new(s).unwrap();
        let triples = vec![
            // Captured: named-class disjointness → Concept NI.
            Triple::new(
                iri("http://ex/A"),
                iri(&format!("{OWL}disjointWith")),
                iri("http://ex/B"),
            ),
            // Captured: property disjointness → Role NI.
            Triple::new(
                iri("http://ex/p"),
                iri(&format!("{OWL}propertyDisjointWith")),
                iri("http://ex/q"),
            ),
            // Uncaptured: complementOf is an equivalence-with-complement, never a mere NI.
            Triple::new(
                iri("http://ex/C"),
                iri(&format!("{OWL}complementOf")),
                iri("http://ex/D"),
            ),
            // Uncaptured: disjointWith whose object is a literal-free blank node that is NOT a
            // resolvable restriction.
            Triple::new(
                iri("http://ex/E"),
                iri(&format!("{OWL}disjointWith")),
                oxrdf::BlankNode::new("nores").unwrap(),
            ),
        ];
        let tbox = TBox::extract(&triples);
        assert_eq!(tbox.consistency_relevant, 4);
        assert_eq!(tbox.consistency_uncaptured, 2);
        assert_eq!(
            tbox.neg_incl,
            vec![
                NegativeInclusion::Concept(
                    Basic::Class("http://ex/A".into()),
                    Basic::Class("http://ex/B".into())
                ),
                NegativeInclusion::Role(Role::named("http://ex/p"), Role::named("http://ex/q")),
            ]
        );
        assert_eq!(
            tbox.consistency_relevant,
            tbox.neg_incl.len() + tbox.consistency_uncaptured,
            "tally invariant (sq-p6yb7)"
        );
        // Negative axioms are QL-legal: they never block fully_captured (unchanged semantics).
        assert!(tbox.fully_captured());
    }

    // sq-fj8lj follow-up — the subClassOf-complement shape `A ⊑ ¬B` is a DL-Lite_R negative
    // inclusion (QL's `superClassExpression ::= ObjectComplementOf(subClassExpression)`), so it
    // is captured structurally instead of landing in `skipped` + `consistency_uncaptured`.
    #[test]
    fn subclass_complement_captured_as_negative_inclusion() {
        use oxrdf::{BlankNode, NamedNode};
        let iri = |s: &str| NamedNode::new(s).unwrap();
        let c = BlankNode::new("c0").unwrap();
        let triples = vec![
            Triple::new(iri("http://ex/A"), iri(&format!("{RDFS}subClassOf")), c.clone()),
            Triple::new(c, iri(&format!("{OWL}complementOf")), iri("http://ex/B")),
        ];
        let tbox = TBox::extract(&triples);
        assert_eq!(
            tbox.neg_incl,
            vec![NegativeInclusion::Concept(
                Basic::Class("http://ex/A".into()),
                Basic::Class("http://ex/B".into())
            )]
        );
        // The shape is QL-legal, so it is consistency-relevant — never `skipped` — and the
        // complement node's own owl:complementOf triple is NOT tallied a second time.
        assert_eq!(tbox.consistency_relevant, 1);
        assert_eq!(tbox.consistency_uncaptured, 0);
        assert_eq!(tbox.skipped, 0);
        assert!(tbox.concept_incl.is_empty(), "¬B is not a positive superclass");
        assert!(tbox.fully_captured());
    }

    // Both operands may be UNQUALIFIED existentials: `∃R ⊑ ¬B` and `A ⊑ ¬∃R` are both QL.
    #[test]
    fn subclass_complement_resolves_existential_operands() {
        use oxrdf::{BlankNode, NamedNode};
        let iri = |s: &str| NamedNode::new(s).unwrap();
        let restriction = |bn: &BlankNode, r: &str| {
            vec![
                Triple::new(bn.clone(), iri(&format!("{OWL}onProperty")), iri(r)),
                Triple::new(
                    bn.clone(),
                    iri(&format!("{OWL}someValuesFrom")),
                    iri(&format!("{OWL}Thing")),
                ),
            ]
        };
        let (lhs, rhs, c0, c1) = (
            BlankNode::new("lhs").unwrap(),
            BlankNode::new("rhs").unwrap(),
            BlankNode::new("c0").unwrap(),
            BlankNode::new("c1").unwrap(),
        );
        let mut triples = restriction(&lhs, "http://ex/r");
        triples.extend(restriction(&rhs, "http://ex/s"));
        triples.extend([
            // ∃r ⊑ ¬B
            Triple::new(lhs, iri(&format!("{RDFS}subClassOf")), c0.clone()),
            Triple::new(c0, iri(&format!("{OWL}complementOf")), iri("http://ex/B")),
            // A ⊑ ¬∃s
            Triple::new(iri("http://ex/A"), iri(&format!("{RDFS}subClassOf")), c1.clone()),
            Triple::new(c1, iri(&format!("{OWL}complementOf")), rhs),
        ]);
        let tbox = TBox::extract(&triples);
        assert_eq!(
            tbox.neg_incl,
            vec![
                NegativeInclusion::Concept(
                    Basic::Exists(Role::named("http://ex/r")),
                    Basic::Class("http://ex/B".into())
                ),
                NegativeInclusion::Concept(
                    Basic::Class("http://ex/A".into()),
                    Basic::Exists(Role::named("http://ex/s"))
                ),
            ]
        );
        assert_eq!(tbox.consistency_relevant, 2);
        assert_eq!(tbox.consistency_uncaptured, 0);
        assert!(tbox.fully_captured());
    }

    // The tally is per AXIOM, not per owl:complementOf triple: a complement node SHARED by two
    // subClassOf triples is two negative inclusions, and the invariant
    // `consistency_relevant == neg_incl.len() + consistency_uncaptured` must still hold.
    // An unresolvable subject on a third one is the uncaptured half of the same invariant.
    #[test]
    fn shared_and_unresolvable_subclass_complements_keep_the_tally_invariant() {
        use oxrdf::{BlankNode, NamedNode};
        let iri = |s: &str| NamedNode::new(s).unwrap();
        let sub_class_of = iri(&format!("{RDFS}subClassOf"));
        let c = BlankNode::new("c0").unwrap();
        let triples = vec![
            Triple::new(iri("http://ex/A"), sub_class_of.clone(), c.clone()),
            Triple::new(iri("http://ex/A2"), sub_class_of.clone(), c.clone()),
            // Unresolvable LHS: a blank node that is not a QL basic concept.
            Triple::new(BlankNode::new("opaque").unwrap(), sub_class_of, c.clone()),
            Triple::new(c, iri(&format!("{OWL}complementOf")), iri("http://ex/B")),
        ];
        let tbox = TBox::extract(&triples);
        assert_eq!(tbox.neg_incl.len(), 2);
        assert_eq!(tbox.consistency_relevant, 3);
        assert_eq!(tbox.consistency_uncaptured, 1);
        assert_eq!(
            tbox.consistency_relevant,
            tbox.neg_incl.len() + tbox.consistency_uncaptured,
            "tally invariant (sq-p6yb7)"
        );
        assert_eq!(tbox.skipped, 0, "a QL-legal negative axiom is never `skipped`");
    }

    // An ORPHAN complement node (defined but never used as a subClassOf RHS) asserts no axiom
    // this extractor models, so it keeps the conservative uncaptured tally — fail-closed.
    // Likewise an AMBIGUOUS node carrying two owl:complementOf triples.
    #[test]
    fn orphan_and_ambiguous_complement_nodes_stay_uncaptured() {
        use oxrdf::{BlankNode, NamedNode};
        let iri = |s: &str| NamedNode::new(s).unwrap();
        let orphan = vec![Triple::new(
            BlankNode::new("c0").unwrap(),
            iri(&format!("{OWL}complementOf")),
            iri("http://ex/B"),
        )];
        let tbox = TBox::extract(&orphan);
        assert!(tbox.neg_incl.is_empty());
        assert_eq!(tbox.consistency_relevant, 1);
        assert_eq!(tbox.consistency_uncaptured, 1);

        let c = BlankNode::new("c0").unwrap();
        let ambiguous = vec![
            Triple::new(
                iri("http://ex/A"),
                iri(&format!("{RDFS}subClassOf")),
                c.clone(),
            ),
            Triple::new(
                c.clone(),
                iri(&format!("{OWL}complementOf")),
                iri("http://ex/B"),
            ),
            Triple::new(c, iri(&format!("{OWL}complementOf")), iri("http://ex/C")),
        ];
        let tbox = TBox::extract(&ambiguous);
        assert!(tbox.neg_incl.is_empty(), "ambiguous complement node: capture nothing");
        assert_eq!(tbox.consistency_uncaptured, 2, "one per owl:complementOf triple");
        assert_eq!(tbox.skipped, 1, "the subClassOf RHS resolves to neither shape");
        assert!(!tbox.fully_captured(), "fail-closed on a malformed complement node");
    }

    // [FABLE-5] sq-p6yb7 — a disjointness against an UNQUALIFIED ∃R restriction node resolves
    // through the restriction index into `Basic::Exists` (the QL-legal ∃R ⊑ ¬B shape).
    #[test]
    fn disjointness_with_unqualified_restriction_resolves_to_exists() {
        use oxrdf::{BlankNode, NamedNode};
        let iri = |s: &str| NamedNode::new(s).unwrap();
        let bn = BlankNode::new("r0").unwrap();
        let triples = vec![
            Triple::new(
                bn.clone(),
                iri(&format!("{OWL}onProperty")),
                iri("http://ex/r"),
            ),
            Triple::new(
                bn.clone(),
                iri(&format!("{OWL}someValuesFrom")),
                iri(&format!("{OWL}Thing")),
            ),
            Triple::new(bn, iri(&format!("{OWL}disjointWith")), iri("http://ex/B")),
        ];
        let tbox = TBox::extract(&triples);
        assert_eq!(tbox.consistency_uncaptured, 0);
        assert_eq!(
            tbox.neg_incl,
            vec![NegativeInclusion::Concept(
                Basic::Exists(Role::named("http://ex/r")),
                Basic::Class("http://ex/B".into())
            )]
        );
    }
}
