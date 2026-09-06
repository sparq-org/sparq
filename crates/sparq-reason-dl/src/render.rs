// [SONNET-4.6] sq-pbz04.4.7 (epic sq-pbz04.4, design record
// research/owl2-direct-semantics-scoping.md §L1): the FORWARD RDF mapping — the inverse of
// the fail-closed reverse mapping in `crate::extract`.
//
// 🤖 SPARQ agent. Maps a structural [`Ontology`] back to RDF triples, following the W3C
// OWL 2 *Mapping to RDF Graphs* tables restricted to the ALCH fragment. Mints fresh blank
// nodes for anonymous class-expression nodes (restrictions, boolean operand lists) using a
// monotone counter (no global state; counter is local to each `render_to_triples` call).
//
// PRIMARY USE CASES
// -----------------
// (a) **Round-trip testing**: the invariant `RDF → extract → render → extract` must yield
//     the same structural model (semantic round-trip; blank-node identity may differ, but
//     the model must be `==`). See the test suite at the bottom of this file.
// (b) **Human-readable diagnostics**: `render_to_turtle` converts the triple output back to
//     a minimal Turtle string so a developer can inspect a structural model without a
//     separate serialiser.
// (c) **The L4 refutation budget fallback** ([OPUS-5] sq-pbz04.4.10): the tableau reasons over
//     the structural model directly, but the RL/EL profile branches consume TRIPLES, so when a
//     refutation exhausts the tableau budget `check::refutation_profile_fallback` renders the
//     augmented model here to re-ask those branches. That path does NOT trust this module: it
//     re-extracts the rendering and compares axiom multisets per call, so a rendering gap costs
//     a fallback rather than feeding a reasoner the wrong graph. (Use (a)'s invariant as the
//     contract; this is the same invariant, checked at runtime.)
//
// OUT-OF-SCOPE
// ------------
// Not a production RDF serialiser: no performance guarantees and no stability commitments
// beyond the round-trip invariant, and nothing downstream may assume a particular blank-node
// labelling or triple order.

use crate::model::{Axiom, ClassExpression, ObjectPropertyExpression, Ontology};
use rustc_hash::FxHashSet;
use sparq_core::dict::Dict;
use sparq_core::dict::Id;

const OWL: &str = "http://www.w3.org/2002/07/owl#";
const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const RDFS: &str = "http://www.w3.org/2000/01/rdf-schema#";

// -------------------------------------------------------------------------------------------
// Fresh blank-node counter (call-local — no global state, no thread-local).
// -------------------------------------------------------------------------------------------

struct BnodeCounter(u64);

impl BnodeCounter {
    fn new() -> Self {
        BnodeCounter(0)
    }

    /// Mints a fresh anonymous blank node label and interns it into `dict`.
    fn fresh(&mut self, dict: &mut Dict) -> Id {
        let label = format!("_r{}", self.0);
        self.0 += 1;
        dict.intern_blank(&label)
    }
}

// -------------------------------------------------------------------------------------------
// Interning helpers for the fixed OWL/RDF/RDFS vocabulary.
// -------------------------------------------------------------------------------------------

fn owl(dict: &mut Dict, local: &str) -> Id {
    dict.intern_iri(&format!("{}{}", OWL, local))
}

fn rdf(dict: &mut Dict, local: &str) -> Id {
    dict.intern_iri(&format!("{}{}", RDF, local))
}

fn rdfs(dict: &mut Dict, local: &str) -> Id {
    dict.intern_iri(&format!("{}{}", RDFS, local))
}

// -------------------------------------------------------------------------------------------
// Public API
// -------------------------------------------------------------------------------------------

/// Renders the ALCH structural [`Ontology`] back into RDF triples per the W3C OWL 2
/// *Mapping to RDF Graphs* tables restricted to the ALCH fragment.
///
/// Fresh blank nodes are minted into `dict` for every anonymous class-expression node
/// (restriction subjects, operand-list cells). The blank-node labels are `_r0`, `_r1`, …
/// (unique within this call, but NOT across calls — IDs are stable WITHIN the same `dict`
/// across calls if the same dict is reused, but the round-trip invariant only requires
/// model equality, not blank-node identity).
///
/// # Round-trip invariant
///
/// For any ontology `onto` that was produced by a SUCCESSFUL [`crate::extract()`] call
/// (i.e. the graph was FULLY captured in the ALCH fragment), calling:
///
/// ```text
/// let triples = render_to_triples(&onto, &mut dict);
/// let onto2   = crate::extract(&dict, &triples).expect("render output is valid ALCH");
/// assert_eq!(onto, onto2);
/// ```
///
/// must hold. Blank-node identities differ (fresh nodes are minted), but the structural
/// models are equal (`Ontology: PartialEq`).
///
/// # Caveats
///
/// - The renderer emits `rdf:type owl:ObjectProperty` declaration triples for every named
///   object property that appears ONLY in `ObjectPropertyAssertion` axioms (i.e. is used
///   solely as a role-assertion predicate). The extractor requires such a declaration to
///   classify the predicate as a role assertion rather than treating it as `Unclassifiable`.
///   Properties in `SubObjectPropertyOf`, `ObjectPropertyDomain`, or `ObjectPropertyRange`
///   axioms are classified via usage-based typing by the extractor without needing an
///   explicit declaration, but the renderer emits a declaration for those too for uniformity
///   and to ensure the round-trip is closed even if the ordering of axiom triples changes.
/// - `EquivalentClasses(Class(A), expr)` is ALWAYS emitted in the explicit-triple shape
///   (`A owl:equivalentClass _:b` with the backbone on a fresh blank node), NEVER as an
///   inline backbone on `A` itself. The two shapes extract differently: an inline backbone
///   makes the extractor INLINE the composite at every use site of `A` (and synthesise the
///   equivalence via the M1 pass), so re-emitting a model that holds atomic `Class(A)`
///   references in the inline shape would CHANGE those references on re-extraction — and a
///   self-referential definition would render as a cyclic class expression the extractor
///   refuses. The explicit shape round-trips both origins; this was measured against the
///   W3C Direct-Semantics corpus (sq-pbz04.4.17, replacing the sq-pbz04.4.7 special case
///   that the corpus arm caught mis-rendering 15 documents).
pub fn render_to_triples(onto: &Ontology, dict: &mut Dict) -> Vec<[Id; 3]> {
    let mut out: Vec<[Id; 3]> = Vec::new();
    let mut counter = BnodeCounter::new();

    // Pre-pass: collect all named object-property ids so we can emit `rdf:type
    // owl:ObjectProperty` declarations before the axiom triples. The extractor uses these
    // declarations to classify role-assertion predicates (without a declaration, an undeclared
    // predicate with an IRI/blank object is `Unclassifiable`).
    //
    // We emit declarations for ALL named object properties to ensure the round-trip is closed
    // regardless of which axiom types reference the property.
    let ty_id = rdf(dict, "type");
    let owl_obj_prop = owl(dict, "ObjectProperty");
    let mut declared: FxHashSet<Id> = FxHashSet::default();
    for axiom in &onto.axioms {
        collect_property_ids(axiom, &mut declared);
    }
    for prop_id_val in declared {
        out.push([prop_id_val, ty_id, owl_obj_prop]);
    }

    for axiom in &onto.axioms {
        emit_axiom(axiom, dict, &mut counter, &mut out);
    }
    out
}

/// Collects the named property ids mentioned in an axiom into `out`.
///
/// Used by `render_to_triples` to pre-emit `rdf:type owl:ObjectProperty` declarations so
/// the extractor can classify role-assertion predicates without a disambiguation failure.
fn collect_property_ids(axiom: &Axiom, out: &mut FxHashSet<Id>) {
    match axiom {
        Axiom::SubObjectPropertyOf { sub, sup } => {
            out.insert(prop_id(sub));
            out.insert(prop_id(sup));
        }
        Axiom::ObjectPropertyDomain { property, .. }
        | Axiom::ObjectPropertyRange { property, .. }
        | Axiom::ObjectPropertyAssertion { property, .. } => {
            out.insert(prop_id(property));
        }
        // Restrictions embedded in ClassExpressions also carry property ids; collect them
        // recursively so a standalone ClassAssertion with a restriction class expression has
        // the property declared.
        Axiom::SubClassOf { sub, sup } => {
            collect_property_ids_in_ce(sub, out);
            collect_property_ids_in_ce(sup, out);
        }
        Axiom::EquivalentClasses(left, right) | Axiom::DisjointClasses(left, right) => {
            collect_property_ids_in_ce(left, out);
            collect_property_ids_in_ce(right, out);
        }
        Axiom::ClassAssertion { class, .. } => {
            collect_property_ids_in_ce(class, out);
        }
        // [GPT-5.6] sq-zfwzq (opt-in `dl_transitive`): the transitive property is a named
        // object property — declare it like every other one.
        #[cfg(feature = "dl_transitive")]
        Axiom::TransitiveObjectProperty { property } => {
            out.insert(prop_id(property));
        }
    }
}

/// Recursively collects property ids from a class expression.
fn collect_property_ids_in_ce(ce: &ClassExpression, out: &mut FxHashSet<Id>) {
    match ce {
        ClassExpression::Class(_) | ClassExpression::Thing | ClassExpression::Nothing => {}
        ClassExpression::ObjectIntersectionOf(ops) | ClassExpression::ObjectUnionOf(ops) => {
            for op in ops {
                collect_property_ids_in_ce(op, out);
            }
        }
        ClassExpression::ObjectComplementOf(inner) => {
            collect_property_ids_in_ce(inner, out);
        }
        ClassExpression::ObjectSomeValuesFrom(ope, filler)
        | ClassExpression::ObjectAllValuesFrom(ope, filler) => {
            out.insert(prop_id(ope));
            collect_property_ids_in_ce(filler, out);
        }
    }
}

/// Renders the ALCH structural [`Ontology`] to a minimal Turtle string for human-readable
/// diagnostics.
///
/// The output is valid Turtle and can be re-parsed to recover the structural model via
/// [`crate::extract()`]. Intended for debug dumps and test diagnostics — not a
/// production-grade Turtle serialiser.
///
/// # Example
///
/// ```rust
/// use sparq_core::dict::Dict;
/// use sparq_reason_dl::model::{Axiom, ClassExpression, Ontology};
/// use sparq_reason_dl::render::render_to_turtle;
///
/// let mut dict = Dict::new();
/// let a_id = dict.intern_iri("http://ex/A");
/// let b_id = dict.intern_iri("http://ex/B");
/// let onto = Ontology {
///     axioms: vec![Axiom::SubClassOf {
///         sub: ClassExpression::Class(a_id),
///         sup: ClassExpression::Class(b_id),
///     }],
/// };
/// let ttl = render_to_turtle(&onto, &mut dict);
/// assert!(ttl.contains("subClassOf"), "expected rdfs:subClassOf in output");
/// ```
#[must_use]
pub fn render_to_turtle(onto: &Ontology, dict: &mut Dict) -> String {
    // Two-pass: first render to triples (which may intern new blank nodes into dict), then
    // serialise the triple slice to Turtle using the dict for IRI/blank lookups. The two-pass
    // approach avoids borrowing dict mutably while iterating the triples.
    let triples = render_to_triples(onto, dict);
    triples_to_turtle(&triples, dict)
}

// -------------------------------------------------------------------------------------------
// Axiom → triples
// -------------------------------------------------------------------------------------------

fn emit_axiom(
    axiom: &Axiom,
    dict: &mut Dict,
    counter: &mut BnodeCounter,
    out: &mut Vec<[Id; 3]>,
) {
    match axiom {
        // GCI: `sub rdfs:subClassOf sup`
        Axiom::SubClassOf { sub, sup } => {
            let s = emit_class_expr(sub, dict, counter, out);
            let p = rdfs(dict, "subClassOf");
            let o = emit_class_expr(sup, dict, counter, out);
            out.push([s, p, o]);
        }

        // `left owl:equivalentClass right` — ALWAYS the explicit-triple shape, with any
        // composite side on a FRESH blank node.
        //
        // NAMED-COMPOSITE SHAPE CHOICE (sq-pbz04.4.17 fix; replaces the sq-pbz04.4.7
        // special case): the extractor accepts TWO RDF shapes for a named-composite
        // equivalence, and they have DIFFERENT model semantics —
        //
        //   (a) explicit: `A owl:equivalentClass _:b` + backbone on `_:b`. Use sites of
        //       `A` elsewhere in the graph extract as the ATOMIC `Class(A)`.
        //   (b) inline:   the backbone directly on `A` (`A owl:intersectionOf (…)`). The
        //       extractor INLINES the composite at every use site of `A`, and the M1 pass
        //       (`emit_named_composite_equiv`) synthesises `EquivalentClasses(A, expr)`.
        //
        // The renderer previously emitted shape (b) whenever the left side was a named
        // class and the right composite. The W3C corpus round-trip arm (sq-pbz04.4.17)
        // MEASURED that to be wrong whenever `A` is REFERENCED elsewhere in the model:
        // atomic `Class(A)` use sites re-extract as the inlined composite (14 corpus
        // mismatches, e.g. WebOnt-description-logic-201…209), and a SELF-referential
        // definition (`person ≡ ∃parent.person`, WebOnt-someValuesFrom-003) renders to a
        // cyclic class expression the extractor refuses outright.
        //
        // Shape (a) round-trips BOTH origins: an (a)-origin model is reproduced verbatim;
        // a (b)-origin model has its use sites ALREADY inlined as composites (they render
        // as anonymous nodes, never as references to `A`), and its M1-prepended
        // `EquivalentClasses` axioms re-extract from the explicit triples in the same
        // order — no duplicate axiom, because the rendered graph carries no inline
        // backbone on `A` for the M1 pass to fire on.
        Axiom::EquivalentClasses(left, right) => {
            let s = emit_class_expr(left, dict, counter, out);
            let p = owl(dict, "equivalentClass");
            let o = emit_class_expr(right, dict, counter, out);
            out.push([s, p, o]);
        }

        // `left owl:disjointWith right`
        Axiom::DisjointClasses(left, right) => {
            let s = emit_class_expr(left, dict, counter, out);
            let p = owl(dict, "disjointWith");
            let o = emit_class_expr(right, dict, counter, out);
            out.push([s, p, o]);
        }

        // `sub rdfs:subPropertyOf sup`
        Axiom::SubObjectPropertyOf { sub, sup } => {
            let s = prop_id(sub);
            let p = rdfs(dict, "subPropertyOf");
            let o = prop_id(sup);
            out.push([s, p, o]);
        }

        // `property rdfs:domain domain`
        Axiom::ObjectPropertyDomain { property, domain } => {
            let s = prop_id(property);
            let p = rdfs(dict, "domain");
            let o = emit_class_expr(domain, dict, counter, out);
            out.push([s, p, o]);
        }

        // `property rdfs:range range`
        Axiom::ObjectPropertyRange { property, range } => {
            let s = prop_id(property);
            let p = rdfs(dict, "range");
            let o = emit_class_expr(range, dict, counter, out);
            out.push([s, p, o]);
        }

        // `individual rdf:type class` (a class assertion)
        Axiom::ClassAssertion { class, individual } => {
            let s = *individual;
            let p = rdf(dict, "type");
            let o = emit_class_expr(class, dict, counter, out);
            out.push([s, p, o]);
        }

        // `source property target` (a role assertion)
        Axiom::ObjectPropertyAssertion { property, source, target } => {
            let p = prop_id(property);
            out.push([*source, p, *target]);
        }

        // `property rdf:type owl:TransitiveProperty` — the exact shape the extractor's
        // opt-in `dl_transitive` arm reads back (round-trip closed; the property's
        // `owl:ObjectProperty` declaration is emitted by the pre-pass like every other
        // property). [GPT-5.6] sq-zfwzq
        #[cfg(feature = "dl_transitive")]
        Axiom::TransitiveObjectProperty { property } => {
            let s = prop_id(property);
            let p = rdf(dict, "type");
            let o = owl(dict, "TransitiveProperty");
            out.push([s, p, o]);
        }
    }
}

// -------------------------------------------------------------------------------------------
// ClassExpression → node id (minting backbone triples into `out`)
// -------------------------------------------------------------------------------------------

/// Maps a class expression to its RDF node id, emitting any required backbone triples.
///
/// Atomic expressions (`Class`, `Thing`, `Nothing`) return the bare dict id without extra
/// triples. Composite expressions mint a fresh blank node, emit the backbone triples that
/// define its shape, and return the blank node's id.
fn emit_class_expr(
    expr: &ClassExpression,
    dict: &mut Dict,
    counter: &mut BnodeCounter,
    out: &mut Vec<[Id; 3]>,
) -> Id {
    match expr {
        ClassExpression::Class(id) => *id,
        ClassExpression::Thing => owl(dict, "Thing"),
        ClassExpression::Nothing => owl(dict, "Nothing"),
        _ => {
            // Composite: mint a blank node and emit backbone on it.
            let bnode = counter.fresh(dict);
            emit_class_expr_on(expr, bnode, dict, counter, out);
            bnode
        }
    }
}

/// Emits the backbone triples that define `expr` with `subject` as the node.
///
/// Called for composite class expressions when the node is already known (either a freshly
/// minted blank node, or a named class IRI in the M1 EquivalentClasses path).
fn emit_class_expr_on(
    expr: &ClassExpression,
    subject: Id,
    dict: &mut Dict,
    counter: &mut BnodeCounter,
    out: &mut Vec<[Id; 3]>,
) {
    match expr {
        // Atomic — no backbone triples; the node itself IS the IRI/constant.
        ClassExpression::Class(_) | ClassExpression::Thing | ClassExpression::Nothing => {
            // Nothing to emit; the subject already encodes the expression.
        }

        // `subject owl:intersectionOf ( C1 C2 … Cn )`
        ClassExpression::ObjectIntersectionOf(operands) => {
            let intersection_of_p = owl(dict, "intersectionOf");
            let list_head = emit_class_list(operands, dict, counter, out);
            out.push([subject, intersection_of_p, list_head]);
        }

        // `subject owl:unionOf ( C1 C2 … Cn )`
        ClassExpression::ObjectUnionOf(operands) => {
            let union_of_p = owl(dict, "unionOf");
            let list_head = emit_class_list(operands, dict, counter, out);
            out.push([subject, union_of_p, list_head]);
        }

        // `subject owl:complementOf C`
        ClassExpression::ObjectComplementOf(operand) => {
            let complement_of_p = owl(dict, "complementOf");
            let operand_id = emit_class_expr(operand, dict, counter, out);
            out.push([subject, complement_of_p, operand_id]);
        }

        // `subject a owl:Restriction; owl:onProperty p; owl:someValuesFrom C`
        ClassExpression::ObjectSomeValuesFrom(property, filler) => {
            emit_restriction(subject, property, filler, true, dict, counter, out);
        }

        // `subject a owl:Restriction; owl:onProperty p; owl:allValuesFrom C`
        ClassExpression::ObjectAllValuesFrom(property, filler) => {
            emit_restriction(subject, property, filler, false, dict, counter, out);
        }
    }
}

/// Emits the three triples that define an `owl:Restriction` node:
/// ```text
/// subject  rdf:type          owl:Restriction .
/// subject  owl:onProperty    property .
/// subject  owl:someValuesFrom / owl:allValuesFrom  filler .
/// ```
fn emit_restriction(
    subject: Id,
    property: &ObjectPropertyExpression,
    filler: &ClassExpression,
    is_some: bool,
    dict: &mut Dict,
    counter: &mut BnodeCounter,
    out: &mut Vec<[Id; 3]>,
) {
    let ty = rdf(dict, "type");
    let restriction_ty = owl(dict, "Restriction");
    let on_property_p = owl(dict, "onProperty");
    let filler_p = if is_some {
        owl(dict, "someValuesFrom")
    } else {
        owl(dict, "allValuesFrom")
    };

    let prop_id_val = prop_id(property);
    let filler_id = emit_class_expr(filler, dict, counter, out);

    out.push([subject, ty, restriction_ty]);
    out.push([subject, on_property_p, prop_id_val]);
    out.push([subject, filler_p, filler_id]);
}

/// Emits an `rdf:List` cell chain for `operands` and returns the head node id.
///
/// Each cell is `[ rdf:first member; rdf:rest next ]`; the last cell's `rdf:rest` is
/// `rdf:nil`.
fn emit_class_list(
    operands: &[ClassExpression],
    dict: &mut Dict,
    counter: &mut BnodeCounter,
    out: &mut Vec<[Id; 3]>,
) -> Id {
    let rdf_first_p = rdf(dict, "first");
    let rdf_rest_p = rdf(dict, "rest");
    let rdf_nil = rdf(dict, "nil");

    // Emit cells right-to-left so each cell's `rdf:rest` is already known.
    let mut next = rdf_nil;
    let mut head = rdf_nil;
    for operand in operands.iter().rev() {
        let cell = counter.fresh(dict);
        let member = emit_class_expr(operand, dict, counter, out);
        out.push([cell, rdf_first_p, member]);
        out.push([cell, rdf_rest_p, next]);
        next = cell;
        head = cell;
    }
    head
}

// -------------------------------------------------------------------------------------------
// Object-property helper
// -------------------------------------------------------------------------------------------

fn prop_id(ope: &ObjectPropertyExpression) -> Id {
    match ope {
        ObjectPropertyExpression::ObjectProperty(id) => *id,
    }
}

// -------------------------------------------------------------------------------------------
// Turtle serialiser (diagnostic only)
// -------------------------------------------------------------------------------------------

/// Serialises a triple slice to a minimal Turtle string using `dict` for id-to-term lookups.
///
/// This is a diagnostic-only serialiser: it uses explicit `<IRI>` notation (no prefix
/// declarations), blank-node `_:label` notation, and one triple per line with a `.`
/// terminator. The output is valid Turtle.
#[must_use]
pub fn triples_to_turtle(triples: &[[Id; 3]], dict: &Dict) -> String {
    use sparq_core::dict::{is_inline, TermParts};

    let term_str = |id: Id| -> String {
        if is_inline(id) {
            // Inline integer literals — rare in OWL ontologies but possible for bnode ids
            // that happen to be in the inline range; render as a diagnostic tag.
            return format!("\"{}\"^^<http://www.w3.org/2001/XMLSchema#integer>", id as i64);
        }
        match dict.term_parts(id) {
            TermParts::Iri { prefix, suffix } => format!("<{}{}>", prefix, suffix),
            TermParts::Blank(label) => format!("_:{}", label),
            TermParts::Lit { value, datatype, lang } => {
                if let Some(lang) = lang {
                    format!("\"{}\"@{}", value, lang)
                } else {
                    format!("\"{}\"^^<{}>", value, datatype)
                }
            }
            TermParts::Triple(_) => "\"<<triple term>>\"".to_string(),
        }
    };

    let mut buf = String::new();
    for [s, p, o] in triples {
        buf.push_str(&term_str(*s));
        buf.push(' ');
        buf.push_str(&term_str(*p));
        buf.push(' ');
        buf.push_str(&term_str(*o));
        buf.push_str(" .\n");
    }
    buf
}

// -------------------------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{extract, model::Ontology};
    use sparq_core::Graph;

    /// Parse a Turtle string into (dict, triples) then extract and render back to triples,
    /// re-extract, and assert model equality. This is the CORE round-trip invariant check.
    fn round_trip(ttl: &str) -> (Ontology, Ontology) {
        let (dict_orig, triples_orig) =
            Graph::parse_to_triples(ttl, "turtle").expect("parse_to_triples");
        let onto1 = extract(&dict_orig, &triples_orig).expect("extract (pass 1)");

        // Re-use dict_orig as the render target (it already has all named IRI ids interned).
        // Blank nodes minted by the renderer get new ids, but model equality is id-invariant
        // for the composite class-expression structure (the round-trip invariant).
        let mut dict_render = dict_orig;
        let triples2 = render_to_triples(&onto1, &mut dict_render);
        let onto2 = extract(&dict_render, &triples2).expect("extract (pass 2)");
        (onto1, onto2)
    }

    // --- Atomic SubClassOf ---

    #[test]
    fn render_to_triples_subclass_of_named() {
        // Direct unit test of render_to_triples: a single named SubClassOf emits exactly one
        // triple, and the rendered triple slice is parseable back to the same axiom.
        let mut dict = Dict::new();
        let a = dict.intern_iri("http://ex/A");
        let b = dict.intern_iri("http://ex/B");
        let onto = Ontology {
            axioms: vec![Axiom::SubClassOf {
                sub: ClassExpression::Class(a),
                sup: ClassExpression::Class(b),
            }],
        };
        let triples = render_to_triples(&onto, &mut dict);
        // Expect exactly one triple: A rdfs:subClassOf B
        assert_eq!(triples.len(), 1, "one SubClassOf triple expected");
        let [s, p, o] = triples[0];
        assert_eq!(s, a);
        assert_eq!(o, b);
        let sub_class_of = dict.intern_iri("http://www.w3.org/2000/01/rdf-schema#subClassOf");
        assert_eq!(p, sub_class_of, "predicate must be rdfs:subClassOf");
    }

    #[test]
    fn round_trip_subclass_of_named() {
        let (onto1, onto2) = round_trip(
            "@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
             <http://ex/A> rdfs:subClassOf <http://ex/B> .",
        );
        assert_eq!(onto1, onto2, "round-trip: SubClassOf(A, B)");
    }

    // --- owl:Thing / owl:Nothing ---

    #[test]
    fn round_trip_thing_nothing() {
        let (onto1, onto2) = round_trip(
            "@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
             @prefix owl:  <http://www.w3.org/2002/07/owl#> .\n\
             <http://ex/A> rdfs:subClassOf owl:Thing .\n\
             owl:Nothing  rdfs:subClassOf <http://ex/A> .",
        );
        assert_eq!(onto1, onto2, "round-trip: Thing/Nothing");
    }

    // --- ObjectIntersectionOf (subClassOf super) ---

    #[test]
    fn round_trip_intersection() {
        let ttl = "@prefix owl:  <http://www.w3.org/2002/07/owl#> .\n\
                   @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
                   <http://ex/S> rdfs:subClassOf [\n\
                     owl:intersectionOf ( <http://ex/A> <http://ex/B> )\n\
                   ] .";
        let (onto1, onto2) = round_trip(ttl);
        assert_eq!(onto1, onto2, "round-trip: SubClassOf(S, A ⊓ B)");
        // The super-expression must be an ObjectIntersectionOf with 2 members.
        let sup = onto1.axioms.iter().find_map(|ax| match ax {
            Axiom::SubClassOf { sup, .. } => Some(sup.clone()),
            _ => None,
        });
        assert!(
            matches!(sup, Some(ClassExpression::ObjectIntersectionOf(ref m)) if m.len() == 2),
            "expected 2-member ObjectIntersectionOf, got {:?}",
            sup
        );
    }

    // --- ObjectUnionOf ---

    #[test]
    fn round_trip_union() {
        let ttl = "@prefix owl:  <http://www.w3.org/2002/07/owl#> .\n\
                   @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
                   <http://ex/S> rdfs:subClassOf [\n\
                     owl:unionOf ( <http://ex/A> <http://ex/B> <http://ex/C> )\n\
                   ] .";
        let (onto1, onto2) = round_trip(ttl);
        assert_eq!(onto1, onto2, "round-trip: SubClassOf(S, A ⊔ B ⊔ C)");
        let sup = onto1.axioms.iter().find_map(|ax| match ax {
            Axiom::SubClassOf { sup, .. } => Some(sup.clone()),
            _ => None,
        });
        assert!(
            matches!(sup, Some(ClassExpression::ObjectUnionOf(ref m)) if m.len() == 3),
            "expected 3-member ObjectUnionOf, got {:?}",
            sup
        );
    }

    // --- ObjectComplementOf ---

    #[test]
    fn round_trip_complement() {
        let ttl = "@prefix owl:  <http://www.w3.org/2002/07/owl#> .\n\
                   @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
                   <http://ex/S> rdfs:subClassOf [ owl:complementOf <http://ex/A> ] .";
        let (onto1, onto2) = round_trip(ttl);
        assert_eq!(onto1, onto2, "round-trip: SubClassOf(S, ¬A)");
    }

    // --- ObjectSomeValuesFrom (existential restriction) ---

    #[test]
    fn round_trip_some_values_from() {
        // Note: blank-node property lists in Turtle must NOT have a trailing '.' inside the
        // brackets; the '.' terminates the outer statement after the closing ']'.
        let ttl = "@prefix owl:  <http://www.w3.org/2002/07/owl#> .\n\
                   @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
                   <http://ex/S> rdfs:subClassOf [\n\
                     a owl:Restriction ;\n\
                     owl:onProperty   <http://ex/r> ;\n\
                     owl:someValuesFrom <http://ex/C>\n\
                   ] .";
        let (onto1, onto2) = round_trip(ttl);
        assert_eq!(onto1, onto2, "round-trip: SubClassOf(S, ∃r.C)");
        let sup = onto1.axioms.iter().find_map(|ax| match ax {
            Axiom::SubClassOf { sup, .. } => Some(sup.clone()),
            _ => None,
        });
        assert!(
            matches!(sup, Some(ClassExpression::ObjectSomeValuesFrom(..))),
            "expected ObjectSomeValuesFrom, got {:?}",
            sup
        );
    }

    // --- ObjectAllValuesFrom (universal restriction) ---

    #[test]
    fn round_trip_all_values_from() {
        // Note: blank-node property lists in Turtle must NOT have a trailing '.' inside the
        // brackets; the '.' terminates the outer statement after the closing ']'.
        let ttl = "@prefix owl:  <http://www.w3.org/2002/07/owl#> .\n\
                   @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
                   <http://ex/S> rdfs:subClassOf [\n\
                     a owl:Restriction ;\n\
                     owl:onProperty   <http://ex/r> ;\n\
                     owl:allValuesFrom <http://ex/C>\n\
                   ] .";
        let (onto1, onto2) = round_trip(ttl);
        assert_eq!(onto1, onto2, "round-trip: SubClassOf(S, ∀r.C)");
        let sup = onto1.axioms.iter().find_map(|ax| match ax {
            Axiom::SubClassOf { sup, .. } => Some(sup.clone()),
            _ => None,
        });
        assert!(
            matches!(sup, Some(ClassExpression::ObjectAllValuesFrom(..))),
            "expected ObjectAllValuesFrom, got {:?}",
            sup
        );
    }

    // --- Nested class expression: ∃r.(A ⊓ B) ---

    #[test]
    fn round_trip_nested_some_intersection() {
        let ttl = "@prefix owl:  <http://www.w3.org/2002/07/owl#> .\n\
                   @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
                   <http://ex/S> rdfs:subClassOf [\n\
                     a owl:Restriction ;\n\
                     owl:onProperty <http://ex/r> ;\n\
                     owl:someValuesFrom [\n\
                       owl:intersectionOf ( <http://ex/A> <http://ex/B> )\n\
                     ]\n\
                   ] .";
        let (onto1, onto2) = round_trip(ttl);
        assert_eq!(onto1, onto2, "round-trip: SubClassOf(S, ∃r.(A ⊓ B))");
    }

    // --- EquivalentClasses (both named) ---

    #[test]
    fn round_trip_equivalent_classes_named() {
        let ttl = "@prefix owl:  <http://www.w3.org/2002/07/owl#> .\n\
                   <http://ex/A> owl:equivalentClass <http://ex/B> .";
        let (onto1, onto2) = round_trip(ttl);
        assert_eq!(onto1, onto2, "round-trip: EquivalentClasses(A, B)");
    }

    // --- EquivalentClasses (named defined by intersection — M1 path) ---

    #[test]
    fn round_trip_equivalent_classes_named_composite() {
        let ttl = "@prefix owl:  <http://www.w3.org/2002/07/owl#> .\n\
                   @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
                   <http://ex/AB> owl:intersectionOf ( <http://ex/A> <http://ex/B> ) .\n\
                   <http://ex/AB> rdfs:subClassOf <http://ex/A> .";
        let (onto1, onto2) = round_trip(ttl);
        assert_eq!(onto1, onto2, "round-trip: EquivalentClasses(AB, A⊓B) + SubClassOf(AB, A)");
    }

    // --- EquivalentClasses regression tests for the sq-pbz04.4.17 shape fix -------------
    //
    // The W3C corpus round-trip arm (sparq-conformance `dl-direct`) caught the old
    // inline-backbone emission mis-rendering 15 documents; these two hand-written cases
    // reproduce both mechanisms so the fix stays guarded WITHOUT the (gitignored) corpus.

    /// Mechanism 1 (14 corpus mismatches, e.g. WebOnt-description-logic-201…209): a named
    /// class `A` DEFINED by an explicit `owl:equivalentClass` composite AND REFERENCED
    /// atomically elsewhere. The old inline-backbone emission made the re-extraction
    /// inline `A`'s definition at the reference site, changing the model.
    #[test]
    fn round_trip_named_composite_referenced_elsewhere() {
        let ttl = "@prefix owl:  <http://www.w3.org/2002/07/owl#> .\n\
                   @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
                   <http://ex/A> owl:equivalentClass [\n\
                     owl:intersectionOf ( <http://ex/P> <http://ex/Q> )\n\
                   ] .\n\
                   <http://ex/S> rdfs:subClassOf [\n\
                     a owl:Restriction ;\n\
                     owl:onProperty <http://ex/r> ;\n\
                     owl:someValuesFrom <http://ex/A>\n\
                   ] .";
        let (onto1, onto2) = round_trip(ttl);
        assert_eq!(onto1, onto2, "round-trip: EquivalentClasses(A, P⊓Q) + SubClassOf(S, ∃r.A)");
        // The reference site must stay ATOMIC in BOTH models — the load-bearing half of
        // the invariant the corpus arm caught the old emission violating.
        for onto in [&onto1, &onto2] {
            let filler_is_atomic = onto.axioms.iter().any(|ax| {
                matches!(
                    ax,
                    Axiom::SubClassOf {
                        sup: ClassExpression::ObjectSomeValuesFrom(_, filler),
                        ..
                    } if matches!(**filler, ClassExpression::Class(_))
                )
            });
            assert!(filler_is_atomic, "∃r.A filler must stay the atomic Class(A): {:?}", onto);
        }
    }

    /// Mechanism 2 (WebOnt-someValuesFrom-003): a SELF-referential definition
    /// `person ≡ ∃parent.person`. The old inline-backbone emission put the restriction
    /// backbone on `person` itself, whose filler is `person` — a cyclic class expression
    /// the extractor REFUSES, so the round-trip died at re-extraction.
    #[test]
    fn round_trip_self_referential_equivalence() {
        let ttl = "@prefix owl: <http://www.w3.org/2002/07/owl#> .\n\
                   <http://ex/person> owl:equivalentClass [\n\
                     a owl:Restriction ;\n\
                     owl:onProperty <http://ex/parent> ;\n\
                     owl:someValuesFrom <http://ex/person>\n\
                   ] .";
        let (onto1, onto2) = round_trip(ttl);
        assert_eq!(onto1, onto2, "round-trip: EquivalentClasses(person, ∃parent.person)");
    }

    /// Direct unit test of the emission shape: `EquivalentClasses(Class(A), composite)`
    /// emits an explicit `owl:equivalentClass` triple whose OBJECT is a fresh blank node
    /// carrying the backbone — never an inline backbone on `A` (sq-pbz04.4.17).
    #[test]
    fn render_to_triples_named_composite_equiv_is_explicit_shape() {
        let mut dict = Dict::new();
        let a = dict.intern_iri("http://ex/A");
        let p = dict.intern_iri("http://ex/P");
        let q = dict.intern_iri("http://ex/Q");
        let onto = Ontology {
            axioms: vec![Axiom::EquivalentClasses(
                ClassExpression::Class(a),
                ClassExpression::ObjectIntersectionOf(vec![
                    ClassExpression::Class(p),
                    ClassExpression::Class(q),
                ]),
            )],
        };
        let triples = render_to_triples(&onto, &mut dict);
        let equivalent_class = dict.intern_iri("http://www.w3.org/2002/07/owl#equivalentClass");
        let equiv: Vec<_> = triples.iter().filter(|[_, pred, _]| *pred == equivalent_class).collect();
        assert_eq!(equiv.len(), 1, "exactly one owl:equivalentClass triple");
        let [s, _, o] = equiv[0];
        assert_eq!(*s, a, "subject is the named class A");
        assert_ne!(*o, a, "object is a fresh node, not A");
        // The backbone must hang off the OBJECT node, never off A.
        let intersection_of = dict.intern_iri("http://www.w3.org/2002/07/owl#intersectionOf");
        assert!(
            triples.iter().any(|[s2, p2, _]| s2 == o && *p2 == intersection_of),
            "the intersection backbone sits on the fresh object node"
        );
        assert!(
            !triples.iter().any(|[s2, p2, _]| *s2 == a && *p2 == intersection_of),
            "no inline backbone on the named class A"
        );
    }

    // --- DisjointClasses ---

    #[test]
    fn round_trip_disjoint_classes() {
        let ttl = "@prefix owl: <http://www.w3.org/2002/07/owl#> .\n\
                   <http://ex/A> owl:disjointWith <http://ex/B> .";
        let (onto1, onto2) = round_trip(ttl);
        assert_eq!(onto1, onto2, "round-trip: DisjointClasses(A, B)");
    }

    // --- SubObjectPropertyOf ---

    #[test]
    fn round_trip_sub_object_property_of() {
        let ttl = "@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
                   @prefix owl:  <http://www.w3.org/2002/07/owl#> .\n\
                   <http://ex/r> a owl:ObjectProperty .\n\
                   <http://ex/s> a owl:ObjectProperty .\n\
                   <http://ex/r> rdfs:subPropertyOf <http://ex/s> .";
        let (onto1, onto2) = round_trip(ttl);
        assert_eq!(onto1, onto2, "round-trip: SubObjectPropertyOf(r, s)");
    }

    // --- ObjectPropertyDomain ---

    #[test]
    fn round_trip_domain() {
        let ttl = "@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
                   @prefix owl:  <http://www.w3.org/2002/07/owl#> .\n\
                   <http://ex/r> a owl:ObjectProperty .\n\
                   <http://ex/r> rdfs:domain <http://ex/A> .";
        let (onto1, onto2) = round_trip(ttl);
        assert_eq!(onto1, onto2, "round-trip: ObjectPropertyDomain(r, A)");
    }

    // --- ObjectPropertyRange ---

    #[test]
    fn round_trip_range() {
        let ttl = "@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
                   @prefix owl:  <http://www.w3.org/2002/07/owl#> .\n\
                   <http://ex/r> a owl:ObjectProperty .\n\
                   <http://ex/r> rdfs:range <http://ex/A> .";
        let (onto1, onto2) = round_trip(ttl);
        assert_eq!(onto1, onto2, "round-trip: ObjectPropertyRange(r, A)");
    }

    // --- ClassAssertion ---

    #[test]
    fn round_trip_class_assertion() {
        // A class assertion with a named class.
        let ttl = "@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .\n\
                   @prefix owl: <http://www.w3.org/2002/07/owl#> .\n\
                   <http://ex/a> rdf:type <http://ex/A> .\n\
                   <http://ex/A> a owl:Class .";
        let (onto1, onto2) = round_trip(ttl);
        assert_eq!(onto1, onto2, "round-trip: ClassAssertion(A, a)");
    }

    // --- ObjectPropertyAssertion ---

    #[test]
    fn round_trip_role_assertion() {
        let ttl = "@prefix owl: <http://www.w3.org/2002/07/owl#> .\n\
                   <http://ex/r> a owl:ObjectProperty .\n\
                   <http://ex/a> <http://ex/r> <http://ex/b> .";
        let (onto1, onto2) = round_trip(ttl);
        assert_eq!(onto1, onto2, "round-trip: ObjectPropertyAssertion(r, a, b)");
    }

    // --- Subclass chain (multiple SubClassOf axioms) ---

    #[test]
    fn round_trip_subclass_chain() {
        let ttl = "@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
                   <http://ex/A> rdfs:subClassOf <http://ex/B> .\n\
                   <http://ex/B> rdfs:subClassOf <http://ex/C> .\n\
                   <http://ex/C> rdfs:subClassOf <http://ex/D> .";
        let (onto1, onto2) = round_trip(ttl);
        assert_eq!(onto1, onto2, "round-trip: SubClassOf chain A⊑B⊑C⊑D");
        assert_eq!(onto1.len(), 3, "expected 3 SubClassOf axioms");
    }

    // --- render_to_turtle smoke test ---

    #[test]
    fn render_to_turtle_smoke() {
        let mut dict = Dict::new();
        let a = dict.intern_iri("http://ex/A");
        let b = dict.intern_iri("http://ex/B");
        let onto = Ontology {
            axioms: vec![Axiom::SubClassOf {
                sub: ClassExpression::Class(a),
                sup: ClassExpression::Class(b),
            }],
        };
        let ttl = render_to_turtle(&onto, &mut dict);
        // Must contain the key vocabulary IRI fragment.
        assert!(ttl.contains("subClassOf"), "Turtle output must mention rdfs:subClassOf");
        assert!(ttl.contains("ex/A"), "Turtle output must mention ex/A");
        assert!(ttl.contains("ex/B"), "Turtle output must mention ex/B");
        // Must end with a '.' terminator line.
        let trimmed = ttl.trim_end();
        assert!(trimmed.ends_with('.'), "Turtle output must end with '.'");
    }

    // --- triples_to_turtle direct unit test ---

    #[test]
    fn triples_to_turtle_direct_unit() {
        let mut dict = Dict::new();
        let s = dict.intern_iri("http://ex/s");
        let p = dict.intern_iri("http://ex/p");
        let o = dict.intern_iri("http://ex/o");
        let triples = vec![[s, p, o]];
        let ttl = triples_to_turtle(&triples, &dict);
        assert!(ttl.contains("<http://ex/s>"), "subject IRI in output");
        assert!(ttl.contains("<http://ex/p>"), "predicate IRI in output");
        assert!(ttl.contains("<http://ex/o>"), "object IRI in output");
        assert!(ttl.contains(" .\n"), "triple dot-terminator in output");
    }

    // --- render_to_triples direct unit test for existential restriction ---

    #[test]
    fn render_to_triples_some_values_from_direct() {
        // Build a synthetic ontology with ∃r.C and check the emitted triple count.
        // Expected triples:
        //   r rdf:type owl:ObjectProperty                     [1]  (declaration pre-pass)
        //   bnode rdf:type owl:Restriction                    [2]
        //   bnode owl:onProperty r                            [3]
        //   bnode owl:someValuesFrom C                        [4]
        //   S rdfs:subClassOf bnode                           [5]
        // total = 5
        let mut dict = Dict::new();
        let s = dict.intern_iri("http://ex/S");
        let r = dict.intern_iri("http://ex/r");
        let c = dict.intern_iri("http://ex/C");
        let onto = Ontology {
            axioms: vec![Axiom::SubClassOf {
                sub: ClassExpression::Class(s),
                sup: ClassExpression::some(r, ClassExpression::Class(c)),
            }],
        };
        let triples = render_to_triples(&onto, &mut dict);
        assert_eq!(triples.len(), 5, "expected 5 triples for SubClassOf(S, ∃r.C)");
    }

    // --- render_to_triples direct unit test for intersection ---

    #[test]
    fn render_to_triples_intersection_direct() {
        // ⊓ list for 2 operands:
        //   cell0 rdf:first A      [1]
        //   cell0 rdf:rest  cell1  [2]
        //   cell1 rdf:first B      [3]
        //   cell1 rdf:rest  nil    [4]
        //   bnode owl:intersectionOf cell0  [5]
        //   S rdfs:subClassOf bnode         [6]
        // total = 6
        let mut dict = Dict::new();
        let s = dict.intern_iri("http://ex/S");
        let a = dict.intern_iri("http://ex/A");
        let b = dict.intern_iri("http://ex/B");
        let onto = Ontology {
            axioms: vec![Axiom::SubClassOf {
                sub: ClassExpression::Class(s),
                sup: ClassExpression::ObjectIntersectionOf(vec![
                    ClassExpression::Class(a),
                    ClassExpression::Class(b),
                ]),
            }],
        };
        let triples = render_to_triples(&onto, &mut dict);
        assert_eq!(triples.len(), 6, "expected 6 triples for SubClassOf(S, A⊓B)");
    }

    /// Opt-in transitive roles ([GPT-5.6] sq-zfwzq, feature `dl_transitive`): a
    /// `TransitiveObjectProperty` axiom renders as `p rdf:type owl:TransitiveProperty`
    /// (plus the uniform `owl:ObjectProperty` declaration) and round-trips to an EQUAL
    /// structural model — including alongside a role assertion over the same property.
    #[cfg(feature = "dl_transitive")]
    #[test]
    fn round_trip_transitive_object_property() {
        let (onto1, onto2) = round_trip(
            "@prefix owl: <http://www.w3.org/2002/07/owl#> .\n\
             <http://ex/r> a owl:TransitiveProperty .\n\
             <http://ex/a> <http://ex/r> <http://ex/b> .",
        );
        assert_eq!(onto1, onto2, "round-trip: TransitiveObjectProperty + role assertion");
        assert!(
            onto1
                .axioms
                .iter()
                .any(|a| matches!(a, Axiom::TransitiveObjectProperty { .. })),
            "the transitivity axiom must survive the round trip"
        );
    }

}
