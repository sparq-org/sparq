//! Dict ids of the fixed RDF / RDFS / OWL / XSD vocabulary the OWL 2 mapping matches on, and
//! small term helpers shared by the extractor, the profile checker and the reasoner.

use super::model::Literal;
use oxrdf::{NamedNode, Term};
use sparq_core::dict::{is_inline, Dict, Id, TermParts, INLINE_BASE, NO_ID};

pub(crate) const OWL: &str = "http://www.w3.org/2002/07/owl#";
pub(crate) const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
pub(crate) const RDFS: &str = "http://www.w3.org/2000/01/rdf-schema#";
pub(crate) const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

macro_rules! vocab {
    ($( $field:ident = $ns:ident : $local:literal ),* $(,)?) => {
        /// Interned ids of the mapping vocabulary. A term absent from the dict is [`NO_ID`]
        /// and therefore matches no triple.
        #[allow(missing_docs)]
        #[derive(Clone, Debug)]
        pub(crate) struct Vocab {
            $( pub $field: Id, )*
        }

        impl Vocab {
            pub(crate) fn intern(dict: &Dict) -> Vocab {
                Vocab {
                    $( $field: lookup_iri(dict, &format!("{}{}", $ns, $local)), )*
                }
            }
        }
    };
}

vocab! {
    // rdf
    ty = RDF: "type",
    first = RDF: "first",
    rest = RDF: "rest",
    nil = RDF: "nil",
    rdf_list = RDF: "List",
    rdf_property = RDF: "Property",
    // rdfs
    sub_class_of = RDFS: "subClassOf",
    sub_property_of = RDFS: "subPropertyOf",
    domain = RDFS: "domain",
    range = RDFS: "range",
    rdfs_class = RDFS: "Class",
    rdfs_datatype = RDFS: "Datatype",
    rdfs_literal = RDFS: "Literal",
    rdfs_label = RDFS: "label",
    rdfs_comment = RDFS: "comment",
    rdfs_see_also = RDFS: "seeAlso",
    rdfs_is_defined_by = RDFS: "isDefinedBy",
    // owl entities
    owl_class = OWL: "Class",
    thing = OWL: "Thing",
    nothing = OWL: "Nothing",
    object_property = OWL: "ObjectProperty",
    datatype_property = OWL: "DatatypeProperty",
    annotation_property = OWL: "AnnotationProperty",
    ontology_property = OWL: "OntologyProperty",
    named_individual = OWL: "NamedIndividual",
    ontology = OWL: "Ontology",
    restriction = OWL: "Restriction",
    deprecated_class = OWL: "DeprecatedClass",
    deprecated_property = OWL: "DeprecatedProperty",
    top_object_property = OWL: "topObjectProperty",
    bottom_object_property = OWL: "bottomObjectProperty",
    top_data_property = OWL: "topDataProperty",
    bottom_data_property = OWL: "bottomDataProperty",
    // characteristics
    functional = OWL: "FunctionalProperty",
    inverse_functional = OWL: "InverseFunctionalProperty",
    transitive = OWL: "TransitiveProperty",
    symmetric = OWL: "SymmetricProperty",
    asymmetric = OWL: "AsymmetricProperty",
    reflexive = OWL: "ReflexiveProperty",
    irreflexive = OWL: "IrreflexiveProperty",
    // n-ary / reification
    all_different = OWL: "AllDifferent",
    all_disjoint_classes = OWL: "AllDisjointClasses",
    all_disjoint_properties = OWL: "AllDisjointProperties",
    negative_property_assertion = OWL: "NegativePropertyAssertion",
    owl_axiom = OWL: "Axiom",
    owl_annotation = OWL: "Annotation",
    members = OWL: "members",
    distinct_members = OWL: "distinctMembers",
    source_individual = OWL: "sourceIndividual",
    assertion_property = OWL: "assertionProperty",
    target_individual = OWL: "targetIndividual",
    target_value = OWL: "targetValue",
    annotated_source = OWL: "annotatedSource",
    annotated_property = OWL: "annotatedProperty",
    annotated_target = OWL: "annotatedTarget",
    // class expressions
    intersection_of = OWL: "intersectionOf",
    union_of = OWL: "unionOf",
    complement_of = OWL: "complementOf",
    one_of = OWL: "oneOf",
    on_property = OWL: "onProperty",
    on_properties = OWL: "onProperties",
    some_values_from = OWL: "someValuesFrom",
    all_values_from = OWL: "allValuesFrom",
    has_value = OWL: "hasValue",
    has_self = OWL: "hasSelf",
    min_cardinality = OWL: "minCardinality",
    max_cardinality = OWL: "maxCardinality",
    cardinality = OWL: "cardinality",
    min_qualified_cardinality = OWL: "minQualifiedCardinality",
    max_qualified_cardinality = OWL: "maxQualifiedCardinality",
    qualified_cardinality = OWL: "qualifiedCardinality",
    on_class = OWL: "onClass",
    on_data_range = OWL: "onDataRange",
    // data ranges
    on_datatype = OWL: "onDatatype",
    with_restrictions = OWL: "withRestrictions",
    datatype_complement_of = OWL: "datatypeComplementOf",
    // axioms
    equivalent_class = OWL: "equivalentClass",
    disjoint_with = OWL: "disjointWith",
    disjoint_union_of = OWL: "disjointUnionOf",
    equivalent_property = OWL: "equivalentProperty",
    property_disjoint_with = OWL: "propertyDisjointWith",
    property_chain_axiom = OWL: "propertyChainAxiom",
    inverse_of = OWL: "inverseOf",
    has_key = OWL: "hasKey",
    same_as = OWL: "sameAs",
    different_from = OWL: "differentFrom",
    imports = OWL: "imports",
    // built-in annotation / ontology properties
    version_info = OWL: "versionInfo",
    version_iri = OWL: "versionIRI",
    prior_version = OWL: "priorVersion",
    backward_compatible_with = OWL: "backwardCompatibleWith",
    incompatible_with = OWL: "incompatibleWith",
    owl_deprecated = OWL: "deprecated",

}

impl Vocab {
    /// The built-in annotation properties of OWL 2 (Structural Specification §10.1) plus the
    /// ontology properties, which carry no logical meaning.
    pub(crate) fn is_builtin_annotation_property(&self, p: Id) -> bool {
        p != NO_ID
            && [
                self.rdfs_label,
                self.rdfs_comment,
                self.rdfs_see_also,
                self.rdfs_is_defined_by,
                self.version_info,
                self.owl_deprecated,
                self.prior_version,
                self.backward_compatible_with,
                self.incompatible_with,
            ]
            .contains(&p)
    }
}

/// The dict id of an IRI, or [`NO_ID`] when the dict does not contain it.
pub(crate) fn lookup_iri(dict: &Dict, iri: &str) -> Id {
    dict.lookup(&Term::NamedNode(NamedNode::new_unchecked(iri.to_string())))
}

/// `Some(iri)` when `id` is an IRI.
pub(crate) fn iri_of(dict: &Dict, id: Id) -> Option<String> {
    if id == NO_ID || is_inline(id) {
        return None;
    }
    match dict.term_parts(id) {
        TermParts::Iri { prefix, suffix } => Some(format!("{prefix}{suffix}")),
        _ => None,
    }
}

/// `true` when `id` is an IRI.
pub(crate) fn is_iri(dict: &Dict, id: Id) -> bool {
    id != NO_ID && !is_inline(id) && matches!(dict.term_parts(id), TermParts::Iri { .. })
}

/// `true` when `id` is a blank node.
pub(crate) fn is_blank(dict: &Dict, id: Id) -> bool {
    id != NO_ID && !is_inline(id) && matches!(dict.term_parts(id), TermParts::Blank(_))
}

/// `true` when `id` is a literal (interned or inline integer).
pub(crate) fn is_literal(dict: &Dict, id: Id) -> bool {
    id != NO_ID && (is_inline(id) || matches!(dict.term_parts(id), TermParts::Lit { .. }))
}

/// `true` when `id` is an RDF 1.2 triple term.
pub(crate) fn is_triple_term(dict: &Dict, id: Id) -> bool {
    id != NO_ID && !is_inline(id) && matches!(dict.term_parts(id), TermParts::Triple(_))
}

/// The parsed literal behind `id`, or `None` when `id` is not a literal.
pub(crate) fn literal_of(dict: &Dict, id: Id) -> Option<Literal> {
    if id == NO_ID {
        return None;
    }
    if is_inline(id) {
        return Some(Literal {
            lexical: (id - INLINE_BASE).to_string(),
            datatype: format!("{XSD}integer"),
            lang: None,
        });
    }
    match dict.term_parts(id) {
        TermParts::Lit {
            value,
            datatype,
            lang,
        } => Some(Literal {
            lexical: value.to_string(),
            datatype: datatype.to_string(),
            lang: lang.map(str::to_string),
        }),
        _ => None,
    }
}

/// Renders a term in N-Triples syntax (diagnostics and explanations).
pub(crate) fn render_term(dict: &Dict, id: Id) -> String {
    if id == NO_ID {
        return "<?>".to_string();
    }
    if let Some(lit) = literal_of(dict, id) {
        return render_literal(&lit);
    }
    match dict.term_parts(id) {
        TermParts::Iri { prefix, suffix } => format!("<{prefix}{suffix}>"),
        TermParts::Blank(label) => format!("_:{label}"),
        TermParts::Triple([s, p, o]) => format!(
            "<<( {} {} {} )>>",
            render_term(dict, s),
            render_term(dict, p),
            render_term(dict, o)
        ),
        TermParts::Lit { .. } => unreachable!("literals are rendered above"),
    }
}

/// Renders a literal in N-Triples syntax.
pub(crate) fn render_literal(lit: &Literal) -> String {
    let mut escaped = String::with_capacity(lit.lexical.len() + 2);
    for c in lit.lexical.chars() {
        match c {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            _ => escaped.push(c),
        }
    }
    match &lit.lang {
        Some(lang) => format!("\"{escaped}\"@{lang}"),
        None if lit.datatype == format!("{XSD}string") => format!("\"{escaped}\""),
        None => format!("\"{escaped}\"^^<{}>", lit.datatype),
    }
}

/// Renders a triple in N-Triples syntax (without the trailing dot).
pub(crate) fn render_triple(dict: &Dict, t: [Id; 3]) -> String {
    format!(
        "{} {} {}",
        render_term(dict, t[0]),
        render_term(dict, t[1]),
        render_term(dict, t[2])
    )
}
