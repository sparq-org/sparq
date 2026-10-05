// [FABLE-5] sq-pbz04.4.1 (epic sq-pbz04.4, design record
// research/owl2-direct-semantics-scoping.md §L1): the FAIL-CLOSED reverse RDF mapping.
//
// 🤖 SPARQ agent. Reads an OWL ontology in the ALCH fragment out of the `(Dict, [[Id; 3]])`
// substrate and returns a [`crate::model::Ontology`] — OR a typed [`ExtractError`] the moment
// it meets a triple it cannot map per the W3C OWL 2 *Mapping to RDF Graphs* tables restricted
// to ALCH.
//
// SOUNDNESS INVARIANT (the reason this is fail-closed, not skip-and-count):
// -------------------------------------------------------------------------
// The downstream consistency/satisfiability checker (L3/L4) must NEVER reason over a graph it
// only PARTIALLY understood. A single dropped axiom can be exactly the constraint that makes
// an ontology inconsistent (so silently dropping it yields an unsound "consistent" verdict) or
// exactly the assertion whose absence makes it consistent (an unsound "inconsistent" verdict).
// So — unlike `sparq-reason-el`/`-ql`, which extract what they can and TALLY the rest — this
// mapping aborts the WHOLE extraction with a structured error on the first out-of-fragment or
// malformed construct. "Understood in full, or refused" is the contract the checker relies on.
//
// The guarantee is realised by CLOSED classification: every triple is routed into exactly one
// of {recognised axiom, recognised class-expression backbone, ignorable declaration/annotation,
// ground assertion} or it produces an [`ExtractError`]. Nothing falls through silently.

use crate::model::{Axiom, ClassExpression, ObjectPropertyExpression, Ontology};
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::{is_inline, Dict, Id, TermParts, NO_ID};

const OWL: &str = "http://www.w3.org/2002/07/owl#";
const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const RDFS: &str = "http://www.w3.org/2000/01/rdf-schema#";

/// The structured, fail-closed rejection taxonomy of the reverse RDF mapping. Each variant is
/// a distinct reason the graph could not be mapped into the ALCH structural model; the payload
/// is a human-readable diagnostic (the offending IRI / triple), NOT part of the taxonomy —
/// callers and tests should match on the VARIANT.
///
/// Deferral rationale + unlock paths for every out-of-fragment construct: design record §5.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExtractError {
    /// A recognised OWL/RDF construct that carries logical meaning OUTSIDE the ALCH fragment:
    /// cardinality / qualified cardinality, nominals (`owl:oneOf`/`owl:hasValue`),
    /// `owl:hasSelf`, inverse properties (`owl:inverseOf`), property characteristics
    /// (`owl:TransitiveProperty` — recognised in-fragment instead under the opt-in
    /// `dl_transitive` feature, [GPT-5.6] sq-zfwzq — / `owl:FunctionalProperty` / symmetry /
    /// reflexivity / …),
    /// `owl:sameAs` / `owl:differentFrom`, property chains, keys, `owl:disjointUnionOf`,
    /// equivalent / disjoint properties, and the n-ary `owl:AllDifferent` /
    /// `owl:AllDisjoint*` / `owl:NegativePropertyAssertion` collections.
    OutOfFragment(String),
    /// A datatype / data-property construct — the L1 ALCH fragment has NO concrete domain: a
    /// literal in a class or individual position, an `owl:DatatypeProperty` assertion, or a
    /// data-range restriction (`owl:onDataRange` / `owl:onDatatype` / `owl:withRestrictions`).
    DataConstruct(String),
    /// An RDF list backing a boolean class expression (`owl:intersectionOf` / `owl:unionOf`)
    /// is ill-formed: unterminated (no `rdf:nil`), cyclic, branching (a cell with two
    /// `rdf:first`/`rdf:rest` edges), or empty.
    MalformedList(String),
    /// A class-expression node is structurally ill-formed: an `owl:Restriction` missing its
    /// `owl:onProperty` or its filler, a `owl:complementOf` / restriction carrying two
    /// conflicting shape predicates, or an over-deep / cyclic class-expression nesting.
    MalformedClassExpression(String),
    /// A triple that cannot be mapped SOUNDLY without a declaration — e.g. an undeclared
    /// predicate whose object is an IRI/blank (indistinguishable between a role assertion and
    /// an annotation), an `rdf:type` whose object is neither a recognised class expression nor
    /// a known declaration meta-class, or an RDF 1.2 triple term (quoted triple) anywhere.
    /// Fail-closed: refused rather than guessed.
    Unclassifiable(String),
}

impl std::fmt::Display for ExtractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Positional args (not inline) — avoids the CodeQL rust/unused-variable false
            // positive on `format!("{x}")` (SPARQ agent CodeQL note).
            ExtractError::OutOfFragment(d) => {
                write!(f, "construct outside the ALCH fragment: {}", d)
            }
            ExtractError::DataConstruct(d) => {
                write!(f, "datatype / data-property construct (no concrete domain in L1): {}", d)
            }
            ExtractError::MalformedList(d) => write!(f, "malformed RDF list: {}", d),
            ExtractError::MalformedClassExpression(d) => {
                write!(f, "malformed class expression: {}", d)
            }
            ExtractError::Unclassifiable(d) => {
                write!(f, "triple not soundly classifiable in the ALCH fragment: {}", d)
            }
        }
    }
}

impl std::error::Error for ExtractError {}

/// Maximum class-expression nesting depth. A well-formed OWL ontology never approaches this;
/// the cap turns a maliciously deep or (via a non-list back-edge) cyclic class-expression
/// encoding into a [`ExtractError::MalformedClassExpression`] instead of a stack overflow.
const MAX_CE_DEPTH: usize = 512;

/// Reads the ALCH-fragment ontology encoded in `triples` (keyed through `dict`).
///
/// Returns the structural [`Ontology`] on success, or the FIRST [`ExtractError`] encountered —
/// the mapping is fail-closed (see the module-level soundness invariant): a graph is mapped in
/// full or refused, never mapped in part.
///
/// # Errors
/// Returns an [`ExtractError`] whenever the graph contains a construct outside the ALCH
/// fragment, a datatype / data-property construct, a malformed list or class expression, or a
/// triple that cannot be classified without a declaration.
pub fn extract(dict: &Dict, triples: &[[Id; 3]]) -> Result<Ontology, ExtractError> {
    let v = Vocab::intern(dict);
    // [OPUS-4.8] Fix 1 + Fix 3: Index::build now returns Err on a branching list,
    // duplicate backbone value, or cross-type punned declaration.
    let idx = Index::build(triples, &v)?;

    // [OPUS-4.8] sq-pbz04.4.12 — M4 fail-closed validation:
    // Orphan/cyclic list cells and unconsumed class-expression backbones are NOT inert.
    // The original comment was WRONG: a bare `rdf:first`/`rdf:rest` cell that is never
    // consumed by an axiom, or a backbone node (`owl:unionOf` etc.) that is never
    // referenced as a class-expression argument, represents a MALFORMED or ILL-FORMED
    // graph structure that violates the "understood in full or refused" contract.
    //   - `rdf:nil rdf:rest _:b` or `rdf:nil rdf:first _:b`: `rdf:nil` cannot be a list
    //     cell (OWL I5.5-003/004 — Direct Semantics inconsistency).
    //   - An orphan list cell with no reachable path from any axiom-consumed backbone head
    //     (I5.5-006 cyclic orphan list).
    //   - A composite-backbone node never referenced by any non-backbone axiom triple
    //     (I5.5-007 unconsumed union backbone).
    // In every case: extracting an empty/partial ontology and calling it consistent would
    // be UNSOUND — the downstream reasoner must never reason over a partial picture of the
    // graph. Refuse (fail-closed) with a typed ExtractError.
    validate_no_orphan_structural_nodes(dict, triples, &v, &idx)?;

    let mut onto = Ontology::new();

    for &[s, p, o] in triples {
        // Class-expression backbone predicates are consumed WHEN the enclosing class node is
        // decoded (via `Index`), and are inert on their own — skip them here. After the
        // `validate_no_orphan_structural_nodes` pass above, every backbone node in the
        // index is guaranteed to be reachable from a non-backbone axiom triple, so the
        // skip here is safe: they were accounted for.
        if v.backbone.contains(&p) {
            continue;
        }
        // Recognised, mapped, or refused — closed classification (see module docs).
        classify_triple(dict, &v, &idx, s, p, o, &mut onto)?;
    }

    // [SONNET-4.6] sq-pbz04.4.11 — named-composite EquivalentClasses pass.
    //
    // CORRECTNESS GAP (M1, discovered by the conformance arm):
    // When a NAMED class IRI carries an inline backbone definition — `:A owl:intersectionOf
    // (…)`, `:A owl:unionOf (…)`, `:A owl:complementOf :B`, or `:A a owl:Restriction ;
    // owl:onProperty :r ; owl:someValuesFrom :C` — the main triple loop inlines the decoded
    // expression at every use site of `:A` (in subClassOf / equivalentClass / domain / range
    // / classAssertion / restriction objects etc.) but NEVER emits the name↔expression
    // binding `EquivalentClasses(A, expr)`.
    //
    // This violates L1's own "understood in full or refused" contract: a conclusion
    // `:A rdfs:subClassOf :x` (where x is an operand of `:A`'s intersection) is
    // underivable from the inlined representation alone — the entailment requires the axiom
    // `EquivalentClasses(A, x⊓y)` to exist in the structural model so that the downstream
    // tableau can reason `A ≡ x⊓y ⊨ A ⊑ x`.
    //
    // FIX: after the main loop, collect every NAMED class IRI (not blank, not inline, not
    // a triple-term) that has exactly one composite backbone predicate in the Index, decode
    // its expression, and prepend `EquivalentClasses(Class(A), expr)` to the axiom list.
    // Prepend (not append) so the name-binding axioms come before any axioms that use the
    // name — consistent with a "declare, then use" reading the downstream tableau expects.
    //
    // BLANK nodes are excluded: an anonymous blank node `:_x owl:intersectionOf (…)`
    // denotes an anonymous (unnamed) expression, not a named class binding, and has no
    // meaningful class IRI to equate.
    //
    // No new `is_inline` / `TermParts::Iri` check is needed: the index already records
    // exactly the backbone-carrying nodes; we just filter to IRI nodes among them.
    //
    // Deduplication: `EquivalentClasses(A, expr)` is emitted once per named class, even if
    // the backbone predicate appears multiple times (the Index de-duplicates backbone
    // values; a contradicting second value is already an error from `Index::build`).
    emit_named_composite_equiv(dict, &v, &idx, &mut onto)?;

    Ok(onto)
}

/// Emits one `EquivalentClasses(Class(A), expr)` axiom for every NAMED class IRI that
/// carries an inline composite backbone definition in the Index (sq-pbz04.4.11, M1 fix).
///
/// See the inline comment in [`extract`] for the full correctness rationale. This function
/// is a pure second-pass over the Index — it never looks at the raw triple slice again.
fn emit_named_composite_equiv(
    dict: &Dict,
    v: &Vocab,
    idx: &Index,
    onto: &mut Ontology,
) -> Result<(), ExtractError> {
    // Collect the set of named-class candidates: every node that carries at least one
    // composite backbone predicate and is an IRI (not blank, not inline, not triple-term).
    // Use a BTreeSet for sorted, deterministic iteration (no hash-map ordering
    // nondeterminism in the structural model).
    use std::collections::BTreeSet;

    let mut named_composite_ids: BTreeSet<Id> = BTreeSet::new();

    // Only IRI nodes. `is_named_iri` = not inline, TermParts::Iri.
    let is_named_iri = |id: Id| -> bool {
        !is_inline(id) && matches!(dict.term_parts(id), TermParts::Iri { .. })
    };

    for &id in idx.intersection_of.keys() {
        if is_named_iri(id) {
            named_composite_ids.insert(id);
        }
    }
    for &id in idx.union_of.keys() {
        if is_named_iri(id) {
            named_composite_ids.insert(id);
        }
    }
    for &id in idx.complement_of.keys() {
        if is_named_iri(id) {
            named_composite_ids.insert(id);
        }
    }
    // Restrictions: any node in is_restriction, some_values_from, or all_values_from
    // that is also a named IRI.
    for &id in &idx.is_restriction {
        if is_named_iri(id) {
            named_composite_ids.insert(id);
        }
    }
    for &id in idx.some_values_from.keys() {
        if is_named_iri(id) {
            named_composite_ids.insert(id);
        }
    }
    for &id in idx.all_values_from.keys() {
        if is_named_iri(id) {
            named_composite_ids.insert(id);
        }
    }

    if named_composite_ids.is_empty() {
        return Ok(());
    }

    // Prepend the EquivalentClasses axioms so they precede all use-site axioms.
    let mut prepend: Vec<Axiom> = Vec::with_capacity(named_composite_ids.len());
    for id in named_composite_ids {
        // Decode the composite expression for this named class. We use a fresh call
        // through decode_class (which creates a fresh `visiting` set), so the cycle guard
        // works correctly for each named class independently.
        let expr = decode_class(dict, v, idx, id)?;
        let name = ClassExpression::Class(id);
        // Emit only if the expression is not the named class itself (a trivially cyclic
        // case is caught by the visiting guard already; this guard just avoids emitting
        // EquivalentClasses(A, A) for any degenerate case that slips through).
        if expr != name {
            prepend.push(Axiom::EquivalentClasses(name, expr));
        }
    }

    // Prepend: insert the name-binding axioms before the use-site axioms so that
    // the structural model reads "declare name, then use name" — the downstream
    // tableau iterates axioms in order and sees the binding before any GCI/assertion
    // that mentions the named class.
    if !prepend.is_empty() {
        let tail = std::mem::replace(&mut onto.axioms, prepend);
        onto.axioms.extend(tail);
    }
    Ok(())
}

/// Validates that no structural (backbone) node is orphaned — i.e., every ANONYMOUS
/// blank-node carrying class-expression or list backbone predicates is reachable from
/// at least one non-backbone axiom triple. [OPUS-4.8] sq-pbz04.4.12 — the M4 fix.
///
/// # Soundness rationale
///
/// The "understood in full or refused" contract requires that the extractor cannot silently
/// produce an empty or lossy ontology from a graph that carries meaningful (but unconsumed)
/// structure. Four W3C Direct-Semantics corpus cases (M4) demonstrated the original gap:
///
/// - **I5.5-003/004**: `rdf:nil rdf:rest _:b` or `rdf:nil rdf:first _:b` — `rdf:nil` is
///   the list terminator and cannot legally be a list cell. The triple is malformed; the
///   inconsistency it encodes must be refused rather than producing an empty/consistent model.
/// - **I5.5-006**: A self-referential list `_:l rdf:first :a; rdf:rest _:l` that is never
///   connected to any axiom. Extracting to an empty ontology would wrongly certify consistency.
/// - **I5.5-007**: An anonymous `_:x owl:unionOf _:list` whose cyclic list is never consumed
///   by any axiom predicate. Same soundness trap.
///
/// # Named IRIs are excluded from this check
///
/// A named class IRI (e.g. `:A owl:intersectionOf (…)`) may appear in the backbone index
/// with no direct non-backbone triple referencing it in the SAME graph — that is the normal
/// pattern for a standalone named-composite class definition. The `emit_named_composite_equiv`
/// post-pass (M1 fix) synthesises `EquivalentClasses(A, expr)` for every such named IRI,
/// consuming it. Only **anonymous blank nodes** can be truly unconsumable (they have no
/// name to bind, so `emit_named_composite_equiv` produces nothing for them).
///
/// # Algorithm
///
/// Seed the reachable set from BOTH the subjects and objects of every non-backbone triple
/// that actually PLACES the node in an axiom or class-expression position (see
/// `is_consuming_triple`), then BFS through the backbone index (intersectionOf → list head →
/// list cells → members, etc.). Any anonymous blank structural node NOT reachable is an
/// orphan → refused.
///
/// **Ignorable declaration/annotation triples do NOT seed reachability** ([OPUS-4.8]
/// sq-pbz04.4.12): a triple whose only relationship to a structural node is a declaration
/// typing (`rdf:type` → an ignorable `DECLARATION_TYPE_LOCALS` meta-class such as `rdf:List`)
/// or an annotation predicate produces NO axiom in `classify_type`/`classify_triple`, so it
/// is not a consumer. Seeding from it wrongly rescued cyclic/orphan list cells carrying the
/// standard `_:list a rdf:List` typing (WebOnt-I5.5-006/-007), letting them slip past the
/// refusal into an empty (wrongly-consistent) ontology.
///
/// **rdf:nil special case**: `rdf:nil` is the list terminator. If it appears as a SUBJECT
/// of `rdf:first` or `rdf:rest`, the graph is unconditionally malformed — refused before
/// the reachability pass.
fn validate_no_orphan_structural_nodes(
    dict: &Dict,
    triples: &[[Id; 3]],
    v: &Vocab,
    idx: &Index,
) -> Result<(), ExtractError> {
    // --- rdf:nil cannot be a list cell (I5.5-003, I5.5-004) ----------------------------
    // `v.rdf_nil == NO_ID` when the IRI is absent from the dict (no triple about nil → no
    // check needed). When present, refuse if nil itself has a list-backbone predicate.
    if v.rdf_nil != sparq_core::dict::NO_ID {
        if idx.first.contains_key(&v.rdf_nil) {
            return Err(ExtractError::MalformedList(
                "rdf:nil cannot have an rdf:first property (ill-formed list cell on nil)"
                    .to_string(),
            ));
        }
        if idx.rest.contains_key(&v.rdf_nil) {
            return Err(ExtractError::MalformedList(
                "rdf:nil cannot have an rdf:rest property (ill-formed list cell on nil)"
                    .to_string(),
            ));
        }
    }

    // If there are no structural nodes at all, there is nothing further to validate.
    if idx.structural_nodes.is_empty() {
        return Ok(());
    }

    // --- Reachability pass (I5.5-006, I5.5-007) -----------------------------------------
    //
    // Helper: `is_anonymous_blank` — true iff `id` is a blank node (not an inline literal,
    // not a triple term, not a named IRI). Named IRIs are excluded from the orphan check
    // because `emit_named_composite_equiv` will synthesise axioms for them; only anonymous
    // blank structural nodes can be genuinely unconsumed.
    let is_anonymous_blank = |id: Id| -> bool {
        !is_inline(id) && matches!(dict.term_parts(id), TermParts::Blank(_))
    };

    // Seed: structural nodes that are considered "already consumed":
    //
    // 1. Named IRI structural nodes — they carry inline composite backbone definitions
    //    (e.g. `:A owl:intersectionOf (…)`) and the `emit_named_composite_equiv` post-pass
    //    synthesises `EquivalentClasses(A, expr)` for each one. Their list cells / filler
    //    nodes must therefore be treated as reachable too (they will be visited through the
    //    BFS expansion below). Named IRIs are always seeded to avoid falsely flagging list
    //    cells or fillers that are only reachable through them.
    //
    // 2. Every structural node that appears as the SUBJECT OR OBJECT of a non-backbone
    //    triple that actually PLACES the node in an axiom or class-expression position:
    //   - Subject: e.g. `[ owl:intersectionOf (…) ] rdfs:subClassOf :C` — the blank node
    //     is the SUBJECT of the axiom triple `rdfs:subClassOf`.
    //   - Object: e.g. `:C rdfs:subClassOf _:x` — `_:x` is an OBJECT.
    //
    // [OPUS-4.8] sq-pbz04.4.12 — SOUNDNESS FIX: an IGNORABLE declaration or annotation
    // triple is NOT a consumer and must NOT seed reachability. Crucially `_:list a rdf:List`
    // (rdf:type whose object is an ignorable DECLARATION_TYPE_LOCALS meta-class, exactly the
    // WebOnt-I5.5-006/-007 encoding) produces NO axiom in `classify_type` — treating it as a
    // consumer wrongly seeded the cyclic/orphan list cell reachable, letting it slip past the
    // refusal and extract to an empty ontology (a WRONG definitive verdict, not abstention).
    // Reachability is seeded ONLY from triples that route into an actual axiom / class-
    // expression position; `is_consuming_triple` mirrors `classify_triple`'s ignorable arms.
    let mut reachable: FxHashSet<Id> = FxHashSet::default();
    let mut worklist: Vec<Id> = Vec::new();

    // Pre-seed all NAMED IRI structural nodes as reachable (emit_named_composite_equiv
    // will synthesise axioms for them; their list cells and fillers are transitively OK).
    for &node in &idx.structural_nodes {
        if !is_anonymous_blank(node) && reachable.insert(node) {
            worklist.push(node);
        }
    }

    // Seed from subjects and objects of CONSUMING (axiom-placing) non-backbone triples only.
    for &[s, p, o] in triples {
        if v.backbone.contains(&p) {
            continue;
        }
        if !is_consuming_triple(v, idx, p, o) {
            continue;
        }
        for &id in &[s, o] {
            if idx.structural_nodes.contains(&id) && reachable.insert(id) {
                worklist.push(id);
            }
        }
    }

    // BFS: from every reachable structural node, follow backbone index edges to discover
    // which nested structural nodes are transitively reachable.
    while let Some(node) = worklist.pop() {
        // Expand each backbone index: the VALUE of these maps is a child node (either a
        // list head, list cell, or filler) that is reachable once the parent is reachable.
        macro_rules! follow_value {
            ($map:expr) => {
                if let Some(&target) = $map.get(&node) {
                    if idx.structural_nodes.contains(&target) && reachable.insert(target) {
                        worklist.push(target);
                    }
                }
            };
        }
        follow_value!(idx.intersection_of); // intersection head → list head
        follow_value!(idx.union_of);        // union head → list head
        follow_value!(idx.complement_of);   // complement node → operand
        follow_value!(idx.some_values_from); // restriction → filler
        follow_value!(idx.all_values_from);  // restriction → filler
        // List cell traversal (the chain of rdf:first/rdf:rest cells).
        if let Some(&first_val) = idx.first.get(&node) {
            if idx.structural_nodes.contains(&first_val) && reachable.insert(first_val) {
                worklist.push(first_val);
            }
        }
        if let Some(&rest_val) = idx.rest.get(&node) {
            if idx.structural_nodes.contains(&rest_val) && reachable.insert(rest_val) {
                worklist.push(rest_val);
            }
        }
    }

    // Refuse any ANONYMOUS BLANK structural node that is NOT reachable. Named IRIs are
    // excluded (handled by emit_named_composite_equiv). Non-blank structural nodes (e.g.
    // an IRI used as an owl:Restriction subject) are similarly excluded — they would have
    // been rejected by the bare-blank guard if they appeared in a class position, or by
    // the axiom-dispatch arms if they were not a known class shape.
    for &node in &idx.structural_nodes {
        if reachable.contains(&node) || !is_anonymous_blank(node) {
            continue;
        }
        let is_list_cell = idx.first.contains_key(&node) || idx.rest.contains_key(&node);
        if is_list_cell {
            return Err(ExtractError::MalformedList(format!(
                "orphan list cell {} is not reachable from any axiom \
                 (ill-formed or cyclic list structure not connected to any axiom)",
                term_iri(dict, node)
            )));
        } else {
            return Err(ExtractError::MalformedClassExpression(format!(
                "unconsumed anonymous class-expression backbone on {} — backbone \
                 predicates present but the node is never referenced by any axiom triple",
                term_iri(dict, node)
            )));
        }
    }
    Ok(())
}

/// Whether a non-backbone triple `_ p o` actually CONSUMES a structural node — i.e. places it
/// in an axiom or class-expression position — for the orphan-reachability seeding in
/// [`validate_no_orphan_structural_nodes`]. [OPUS-4.8] sq-pbz04.4.12.
///
/// It is `false` for exactly the triples `classify_triple`/`classify_type` treat as IGNORABLE
/// (produce no axiom):
/// - a declaration typing `_ rdf:type o` where `o` is an ignorable `DECLARATION_TYPE_LOCALS`
///   meta-class (`rdf:List`, `owl:Class`, `owl:Restriction`, …) — notably `_:list a rdf:List`,
///   the WebOnt-I5.5-006/-007 encoding whose presence must NOT rescue a cyclic/orphan list;
/// - an annotation triple (a built-in annotation predicate or a declared annotation property).
///
/// Every other non-backbone triple routes into a real axiom (subClassOf / equivalentClass /
/// disjointWith / domain / range / a ClassAssertion via a non-declaration `rdf:type` object /
/// a role assertion / an out-of-fragment refusal), so it genuinely references the node. The
/// caller (the backbone predicates are already skipped) passes only non-backbone triples.
fn is_consuming_triple(v: &Vocab, idx: &Index, p: Id, o: Id) -> bool {
    // Declaration typing (`rdf:type` → ignorable meta-class) — no axiom emitted.
    if p == v.ty && v.declaration_types.contains(&o) {
        return false;
    }
    // Built-in or declared annotation predicate — no axiom emitted.
    if v.annotation.contains(&p) || idx.annotation_props.contains(&p) {
        return false;
    }
    true
}

/// Routes one non-backbone triple: into an [`Axiom`] on `onto`, silently past an ignorable
/// declaration / annotation / header, or into an [`ExtractError`].
fn classify_triple(
    dict: &Dict,
    v: &Vocab,
    idx: &Index,
    s: Id,
    p: Id,
    o: Id,
    onto: &mut Ontology,
) -> Result<(), ExtractError> {
    // --- Hoisted punned-predicate guard — fires BEFORE the axiom-dispatch arms ----------
    // [SONNET-4.6] sq-pbz04.4.1 soundness closure: a predicate simultaneously declared as
    // AnnotationProperty AND ObjectProperty (or any other cross-type punning) is
    // Unclassifiable — the downstream checker must never reason over an axiom whose
    // classification was ambiguous. Hoisted here so no axiom is produced from an
    // ambiguous predicate regardless of which arm would have matched.
    // The companion chokepoint in `decode_object_property` covers punned IRIs that arrive
    // in the property-ARGUMENT position (domain/range subject, subPropertyOf operands,
    // owl:onProperty object) rather than as the triple's predicate.
    if idx.punned_props.contains(&p) {
        return Err(ExtractError::Unclassifiable(format!(
            "predicate {} is declared as multiple property types (ambiguous classification)",
            term_iri(dict, p)
        )));
    }

    // --- Axiom predicates ---------------------------------------------------------------
    if p == v.sub_class_of {
        let sub = decode_class(dict, v, idx, s)?;
        let sup = decode_class(dict, v, idx, o)?;
        onto.axioms.push(Axiom::SubClassOf { sub, sup });
        return Ok(());
    }
    if p == v.equivalent_class {
        let left = decode_class(dict, v, idx, s)?;
        let right = decode_class(dict, v, idx, o)?;
        onto.axioms.push(Axiom::EquivalentClasses(left, right));
        return Ok(());
    }
    if p == v.disjoint_with {
        let left = decode_class(dict, v, idx, s)?;
        let right = decode_class(dict, v, idx, o)?;
        onto.axioms.push(Axiom::DisjointClasses(left, right));
        return Ok(());
    }
    if p == v.sub_property_of {
        let sub = decode_object_property(dict, idx, s)?;
        let sup = decode_object_property(dict, idx, o)?;
        onto.axioms.push(Axiom::SubObjectPropertyOf { sub, sup });
        return Ok(());
    }
    if p == v.domain {
        let property = decode_object_property(dict, idx, s)?;
        let domain = decode_class(dict, v, idx, o)?;
        onto.axioms.push(Axiom::ObjectPropertyDomain { property, domain });
        return Ok(());
    }
    if p == v.range {
        let property = decode_object_property(dict, idx, s)?;
        let range = decode_class(dict, v, idx, o)?;
        onto.axioms.push(Axiom::ObjectPropertyRange { property, range });
        return Ok(());
    }

    // --- rdf:type — declaration, class assertion, or out-of-fragment meta-class ----------
    if p == v.ty {
        return classify_type(dict, v, idx, s, o, onto);
    }

    // --- Recognised out-of-fragment logical / datatype predicates ------------------------
    if let Some(name) = v.out_of_fragment.get(&p) {
        return Err(ExtractError::OutOfFragment(format!(
            "predicate {} on {}",
            name,
            term_iri(dict, s)
        )));
    }
    if let Some(name) = v.data_predicate.get(&p) {
        return Err(ExtractError::DataConstruct(format!(
            "predicate {} on {}",
            name,
            term_iri(dict, s)
        )));
    }

    // --- Ignorable built-in annotations --------------------------------------------------
    if v.annotation.contains(&p) {
        return Ok(());
    }

    // (Punned-predicate guard has already fired at the top of this function before the axiom
    // arms — no duplicate check needed here.)

    // --- Declared annotation properties (from the declaration pass) — ignorable ----------
    if idx.annotation_props.contains(&p) {
        return Ok(());
    }

    // --- Declared datatype properties — a data assertion, out of the concrete-domain-free
    //     ALCH fragment ----------------------------------------------------------------
    if idx.data_props.contains(&p) {
        return Err(ExtractError::DataConstruct(format!(
            "assertion using data property {}",
            term_iri(dict, p)
        )));
    }

    // --- Role assertion: a property known to be an object property, over two individuals --
    if idx.object_props.contains(&p) {
        return role_assertion(dict, idx, s, p, o, onto);
    }

    // --- Otherwise: cannot classify soundly without a declaration -------------------------
    // A literal object looks like a data-property assertion (no concrete domain in L1); an
    // IRI/blank object is ambiguous between a role assertion and an annotation. Either way we
    // refuse rather than guess (fail-closed).
    if is_literal(dict, o) {
        Err(ExtractError::DataConstruct(format!(
            "literal-valued triple with undeclared predicate {}",
            term_iri(dict, p)
        )))
    } else {
        Err(ExtractError::Unclassifiable(format!(
            "undeclared predicate {} (cannot tell a role assertion from an annotation)",
            term_iri(dict, p)
        )))
    }
}

/// Handles an `rdf:type` triple `s a o`.
fn classify_type(
    dict: &Dict,
    v: &Vocab,
    idx: &Index,
    s: Id,
    o: Id,
    onto: &mut Ontology,
) -> Result<(), ExtractError> {
    // Declaration / structural typings carry no ALCH-logical import — ignore.
    if v.declaration_types.contains(&o) {
        return Ok(());
    }
    // [GPT-5.6] sq-zfwzq (opt-in `dl_transitive`): `s rdf:type owl:TransitiveProperty` is an
    // in-fragment RBox axiom — `TransitiveObjectProperty(s)`. The subject goes through the
    // same `decode_object_property` chokepoint as every other property position (refusing
    // literals, blank nodes, triple terms, inverse expressions, and punned IRIs), so a
    // malformed transitivity typing is refused, never guessed. With the feature OFF this arm
    // does not exist and the typing stays in `out_of_fragment_types` below (fail-closed).
    #[cfg(feature = "dl_transitive")]
    if o == v.owl_transitive_property {
        let property = decode_object_property(dict, idx, s)?;
        onto.axioms.push(Axiom::TransitiveObjectProperty { property });
        return Ok(());
    }
    // Property characteristics and n-ary collection meta-classes are logical but out of
    // fragment — refuse (they are not silently droppable).
    if let Some(name) = v.out_of_fragment_types.get(&o) {
        return Err(ExtractError::OutOfFragment(format!(
            "rdf:type {} on {}",
            name,
            term_iri(dict, s)
        )));
    }
    // owl:Thing / owl:Nothing as a type are legitimate (trivial / bottoming) class assertions.
    // Otherwise the object must decode as an ALCH class expression → ClassAssertion. A literal
    // object is malformed; an undeclared IRI/blank that does not decode is Unclassifiable.
    if is_literal(dict, o) {
        return Err(ExtractError::MalformedClassExpression(format!(
            "rdf:type with a literal class on {}",
            term_iri(dict, s)
        )));
    }
    let class = decode_class(dict, v, idx, o)?;
    onto.axioms.push(Axiom::ClassAssertion {
        class,
        individual: s,
    });
    Ok(())
}

/// Builds an [`Axiom::ObjectPropertyAssertion`] from `s p o`, or refuses.
fn role_assertion(
    dict: &Dict,
    idx: &Index,
    s: Id,
    p: Id,
    o: Id,
    onto: &mut Ontology,
) -> Result<(), ExtractError> {
    if is_literal(dict, o) {
        // An object property with a literal object is a data assertion / a punning clash — out
        // of the datatype-free ALCH fragment.
        return Err(ExtractError::DataConstruct(format!(
            "object property {} asserted with a literal value",
            term_iri(dict, p)
        )));
    }
    // The operands of a ground role assertion must be individuals, not class-expression
    // backbone nodes (a restriction / list cell used as an individual is malformed).
    if idx.structural_nodes.contains(&s) || idx.structural_nodes.contains(&o) {
        return Err(ExtractError::MalformedClassExpression(format!(
            "role assertion {} over a class-expression node",
            term_iri(dict, p)
        )));
    }
    if is_triple_term(dict, s) || is_triple_term(dict, o) {
        return Err(ExtractError::Unclassifiable(
            "RDF 1.2 triple term as a role-assertion individual".to_string(),
        ));
    }
    // [SONNET-4.6] sq-pbz04.4.8: this path builds the property expression directly rather
    // than through `decode_object_property`, so the built-in fixed-extension refusal is
    // applied here too — the check is only sound if it is UNIFORM over property positions.
    if let Some(err) = builtin_property_refusal(dict, p) {
        return Err(err);
    }
    onto.axioms.push(Axiom::ObjectPropertyAssertion {
        property: ObjectPropertyExpression::ObjectProperty(p),
        source: s,
        target: o,
    });
    Ok(())
}

/// Decodes the dict node `id` into an ALCH [`ClassExpression`], or refuses. Cycle- and
/// depth-guarded (see [`MAX_CE_DEPTH`]).
fn decode_class(
    dict: &Dict,
    v: &Vocab,
    idx: &Index,
    id: Id,
) -> Result<ClassExpression, ExtractError> {
    let mut visiting = FxHashSet::default();
    decode_class_inner(dict, v, idx, id, &mut visiting, 0)
}

fn decode_class_inner(
    dict: &Dict,
    v: &Vocab,
    idx: &Index,
    id: Id,
    visiting: &mut FxHashSet<Id>,
    depth: usize,
) -> Result<ClassExpression, ExtractError> {
    if depth > MAX_CE_DEPTH {
        return Err(ExtractError::MalformedClassExpression(
            "class expression nested beyond the depth cap (cyclic or adversarial)".to_string(),
        ));
    }
    if is_literal(dict, id) {
        return Err(ExtractError::DataConstruct(
            "literal in a class-expression position".to_string(),
        ));
    }
    if is_triple_term(dict, id) {
        return Err(ExtractError::Unclassifiable(
            "RDF 1.2 triple term in a class-expression position".to_string(),
        ));
    }
    // [FABLE-5] sq-pbz04.4.9: a bare OWL 2 datatype-map IRI (`xsd:integer`, `rdfs:Literal`, …)
    // in a class-expression position — including an `rdfs:range` / `rdfs:domain` object — carries
    // concrete-domain meaning the datatype-free ALCH fragment cannot model. Reading it as an
    // opaque object class would silently drop that meaning (e.g. `p rdfs:range xsd:integer` +
    // `p rdfs:range xsd:string` are only jointly unsatisfiable because the value spaces are
    // DISJOINT — invisible under an opaque-class reading, WebOnt-I5.3-015). Refuse fail-closed,
    // the same discipline as the literal / `owl:onDatatype` / declared-datatype-property
    // refusals; datatypes are the design-record §5 deferral pending a concrete-domain oracle.
    if let Some(dt) = datatype_iri_name(dict, id) {
        return Err(ExtractError::DataConstruct(format!(
            "datatype IRI {} in a class-expression position (no concrete domain in L1)",
            dt
        )));
    }
    if id == v.thing {
        return Ok(ClassExpression::Thing);
    }
    if id == v.nothing {
        return Ok(ClassExpression::Nothing);
    }

    // Reject a node carrying an out-of-fragment class shape (nominal / cardinality / hasSelf /
    // data-range restriction) BEFORE treating it as a named class, so `∃r.{a}` or a cardinality
    // restriction is never mistaken for an opaque named class (which would drop its meaning).
    if let Some(name) = idx.out_of_fragment_shape.get(&id) {
        return Err(ExtractError::OutOfFragment(format!(
            "class node using {}",
            name
        )));
    }
    if let Some(name) = idx.data_shape.get(&id) {
        return Err(ExtractError::DataConstruct(format!(
            "class node using {}",
            name
        )));
    }

    // Boolean / quantifier shapes, detected by the presence of their defining predicate. This
    // fires for BLANK anonymous nodes AND for a NAMED class that carries an inline definition
    // (`:A owl:intersectionOf (…)`): `owl:intersectionOf`/`unionOf`/`complementOf`/a
    // `owl:Restriction` on a class node denotes a COMPLETE equivalence.
    //
    // ANONYMOUS blank nodes: inlining the decoded expression at every use site is
    // model-preserving and sufficient — there is no class NAME to bind.
    //
    // NAMED class IRIs: inlining alone is NOT sufficient (sq-pbz04.4.11, M1 gap). A
    // conclusion that mentions the NAME as a class constant is underivable from inlined
    // expressions alone — the entailment `:A subClassOf :x` (where :x is an operand of
    // `:A owl:intersectionOf (…)`) requires the axiom `EquivalentClasses(A, x⊓y)` in
    // the structural model. `extract()` emits that axiom in the post-loop pass
    // `emit_named_composite_equiv`; this function still inlines so that use-site axioms
    // (where `:A` appears as a class argument) already hold the full decoded expression.
    // Decoding is deterministic, so the inlined form and the EquivalentClasses expression
    // are always the same expression structurally.
    let is_inter = idx.intersection_of.contains_key(&id);
    let is_union = idx.union_of.contains_key(&id);
    let is_compl = idx.complement_of.contains_key(&id);
    let is_restr = idx.is_restriction.contains(&id)
        || idx.some_values_from.contains_key(&id)
        || idx.all_values_from.contains_key(&id);
    let shape_count =
        usize::from(is_inter) + usize::from(is_union) + usize::from(is_compl) + usize::from(is_restr);
    if shape_count > 1 {
        return Err(ExtractError::MalformedClassExpression(format!(
            "class node combines multiple class-expression shapes: {}",
            term_iri(dict, id)
        )));
    }
    if shape_count == 0 {
        // [SONNET-4.6] sq-pbz04.4.1 bare-blank fix (Copilot thread 4): blank nodes in a
        // class-expression position with no backbone predicates have no sound OWL mapping.
        // Per the W3C OWL 2 RDF Mapping tables, blank nodes denote *structured* anonymous
        // class expressions; a bare blank node has no recoverable class expression to decode.
        // Taxonomy arm: MalformedClassExpression — the node IS in a class-expression position
        // but is structurally incomplete (no intersection/union/complement/restriction backbone).
        // Refuse fail-closed rather than returning an opaque Class(blank) that the downstream
        // checker would treat as a real class constant. Named-class IRIs are valid leaves.
        if !is_inline(id) && matches!(dict.term_parts(id), TermParts::Blank(_)) {
            return Err(ExtractError::MalformedClassExpression(format!(
                "bare blank node {} in class-expression position (no class-expression backbone)",
                term_iri(dict, id)
            )));
        }
        return Ok(ClassExpression::Class(id));
    }

    // Composite node — guard direct/indirect self-reference (e.g. `_:x complementOf _:x`,
    // `:A owl:intersectionOf (:A …)`) so a cyclic encoding fails closed rather than recursing.
    if !visiting.insert(id) {
        return Err(ExtractError::MalformedClassExpression(format!(
            "cyclic class expression at {}",
            term_iri(dict, id)
        )));
    }
    let result = if is_inter {
        let head = idx.intersection_of[&id];
        // [FABLE-5] sq-pbz04.4.16 — singleton `owl:intersectionOf` normalization (M7).
        // The OWL-1-era RDF encoding `_:B owl:intersectionOf ( :A )` — a one-member operand
        // list — denotes the class `⊓{A}`, whose Direct-Semantics interpretation is EXACTLY
        // that of its sole member `A` (OWL 2 Structural Specification §8.1: the extension of a
        // 1-ary ObjectIntersectionOf is the intersection of the single conjunct's extension,
        // i.e. the conjunct itself). The official test-suite tagging normalizes this singleton
        // to its member before profile-checking, so `_:B owl:intersectionOf ( :A )` is tagged
        // in EL/QL/RL. L2's structural-spec arity rule (≥ 2 members) would otherwise reject it
        // as `NotIn`. Normalizing HERE — at L1 extraction, model-preservingly — makes the
        // downstream profile checker AND the tableau see the equivalent atomic member, so the
        // 27 pinned M7 profile divergences resolve without weakening the arity rule for genuine
        // ≥ 2-member intersections. `decode_class_list` guarantees a non-empty list (an empty
        // one is `MalformedList`), so a 1-element result is the only singleton case.
        decode_class_list(dict, v, idx, head, visiting, depth).map(normalize_intersection)
    } else if is_union {
        let head = idx.union_of[&id];
        decode_class_list(dict, v, idx, head, visiting, depth).map(ClassExpression::ObjectUnionOf)
    } else if is_compl {
        let operand = idx.complement_of[&id];
        decode_class_inner(dict, v, idx, operand, visiting, depth + 1)
            .map(|inner| ClassExpression::ObjectComplementOf(Box::new(inner)))
    } else {
        decode_restriction(dict, v, idx, id, visiting, depth)
    };
    visiting.remove(&id);
    result
}

/// Decodes an `owl:Restriction` node (`∃R.C` / `∀R.C`) or refuses.
fn decode_restriction(
    dict: &Dict,
    v: &Vocab,
    idx: &Index,
    id: Id,
    visiting: &mut FxHashSet<Id>,
    depth: usize,
) -> Result<ClassExpression, ExtractError> {
    let prop = idx.on_property.get(&id).copied().ok_or_else(|| {
        ExtractError::MalformedClassExpression(format!(
            "restriction {} missing owl:onProperty",
            term_iri(dict, id)
        ))
    })?;
    let property = decode_object_property(dict, idx, prop)?;
    let some = idx.some_values_from.get(&id).copied();
    let all = idx.all_values_from.get(&id).copied();
    match (some, all) {
        (Some(_), Some(_)) => Err(ExtractError::MalformedClassExpression(format!(
            "restriction {} has both someValuesFrom and allValuesFrom",
            term_iri(dict, id)
        ))),
        (Some(filler), None) => {
            let inner = decode_class_inner(dict, v, idx, filler, visiting, depth + 1)?;
            Ok(ClassExpression::ObjectSomeValuesFrom(property, Box::new(inner)))
        }
        (None, Some(filler)) => {
            let inner = decode_class_inner(dict, v, idx, filler, visiting, depth + 1)?;
            Ok(ClassExpression::ObjectAllValuesFrom(property, Box::new(inner)))
        }
        (None, None) => Err(ExtractError::MalformedClassExpression(format!(
            "restriction {} missing its someValuesFrom/allValuesFrom filler",
            term_iri(dict, id)
        ))),
    }
}

/// Normalize a decoded `owl:intersectionOf` operand list into a class expression, collapsing
/// the OWL-1-era SINGLETON encoding `⊓{A}` to its sole member `A` (M7, sq-pbz04.4.16).
///
/// A one-member `ObjectIntersectionOf` has, under OWL 2 Direct Semantics, exactly the
/// interpretation of its single conjunct (OWL 2 Structural Specification §8.1), so returning
/// the member directly is MODEL-PRESERVING. The official OWL WG profile tagging performs this
/// same normalization before deciding EL/QL/RL membership; without it, L2's structural-spec
/// arity rule (≥ 2 members) would spuriously reject these positively-tagged cases as `NotIn`.
///
/// `members` is always non-empty ([`decode_class_list`] refuses an empty operand list with
/// `MalformedList`), so the only singleton case is `len == 1`. A ≥ 2-member list is wrapped
/// unchanged in `ObjectIntersectionOf`, preserving the arity rule for genuine intersections.
fn normalize_intersection(members: Vec<ClassExpression>) -> ClassExpression {
    let mut members = members;
    if members.len() == 1 {
        // `swap_remove(0)` moves the sole member out without a clone; the vec is discarded.
        members.swap_remove(0)
    } else {
        ClassExpression::ObjectIntersectionOf(members)
    }
}

/// Decodes an `owl:intersectionOf`/`owl:unionOf` operand list into a non-empty vector of
/// class expressions, or refuses (malformed list / member).
fn decode_class_list(
    dict: &Dict,
    v: &Vocab,
    idx: &Index,
    head: Id,
    visiting: &mut FxHashSet<Id>,
    depth: usize,
) -> Result<Vec<ClassExpression>, ExtractError> {
    let mut out = Vec::new();
    let mut cursor = head;
    let mut seen = FxHashSet::default();
    loop {
        if cursor == v.rdf_nil {
            break;
        }
        if !seen.insert(cursor) {
            return Err(ExtractError::MalformedList(
                "cyclic rdf:rest chain".to_string(),
            ));
        }
        if seen.len() > MAX_CE_DEPTH {
            return Err(ExtractError::MalformedList(
                "list longer than the depth cap".to_string(),
            ));
        }
        let first = idx.first.get(&cursor).copied().ok_or_else(|| {
            ExtractError::MalformedList(format!(
                "list cell {} missing rdf:first",
                term_iri(dict, cursor)
            ))
        })?;
        let member = decode_class_inner(dict, v, idx, first, visiting, depth + 1)?;
        out.push(member);
        cursor = idx.rest.get(&cursor).copied().ok_or_else(|| {
            ExtractError::MalformedList(format!(
                "list cell {} missing rdf:rest (unterminated list)",
                term_iri(dict, cursor)
            ))
        })?;
    }
    if out.is_empty() {
        return Err(ExtractError::MalformedList(
            "empty operand list (owl:intersectionOf/owl:unionOf require at least one member)"
                .to_string(),
        ));
    }
    Ok(out)
}

/// Decodes a node used in an object-property position. In L1 only a NAMED object property is
/// admitted; an `owl:inverseOf` node (an inverse property expression) is refused
/// (`OutOfFragment`), a literal is refused (`MalformedClassExpression`), and — per
/// model.rs:20-33 — a blank node or RDF 1.2 triple term is refused (`Unclassifiable`).
fn decode_object_property(
    dict: &Dict,
    idx: &Index,
    id: Id,
) -> Result<ObjectPropertyExpression, ExtractError> {
    if is_literal(dict, id) {
        return Err(ExtractError::MalformedClassExpression(
            "literal in an object-property position".to_string(),
        ));
    }
    // [OPUS-4.8] Fix 2: blank nodes and triple terms are not named object properties.
    // The ALCH fragment admits ONLY named (IRI) object properties — model.rs:20-33.
    if is_triple_term(dict, id) {
        return Err(ExtractError::Unclassifiable(
            "RDF 1.2 triple term in an object-property position".to_string(),
        ));
    }
    if !is_inline(id) && matches!(dict.term_parts(id), TermParts::Blank(_)) {
        return Err(ExtractError::Unclassifiable(format!(
            "blank node {} in an object-property position (named properties only in L1)",
            term_iri(dict, id)
        )));
    }
    if idx.inverse_of.contains(&id) {
        return Err(ExtractError::OutOfFragment(format!(
            "inverse object property expression {}",
            term_iri(dict, id)
        )));
    }
    // [SONNET-4.6] sq-pbz04.4.1 soundness closure: structural chokepoint for punned IRIs
    // arriving in the property-ARGUMENT position (domain/range subject, subPropertyOf
    // operands, owl:onProperty object). A property IRI declared under multiple property
    // types is Unclassifiable at this call site — the downstream checker must never reason
    // over an axiom whose classification was ambiguous. This covers all current and future
    // callers (decode_restriction for owl:onProperty; classify_triple for axiom arms)
    // without requiring per-call-site discipline.
    if idx.punned_props.contains(&id) {
        return Err(ExtractError::Unclassifiable(format!(
            "property IRI {} is declared as multiple property types (ambiguous classification)",
            term_iri(dict, id)
        )));
    }
    if let Some(err) = builtin_property_refusal(dict, id) {
        return Err(err);
    }
    Ok(ObjectPropertyExpression::ObjectProperty(id))
}

/// `Some(refusal)` when `id` is one of OWL 2's four BUILT-IN property IRIs, whose
/// interpretation is FIXED by the Direct Semantics rather than constrained by axioms:
/// `owl:topObjectProperty` (the universal relation ΔI × ΔI), `owl:bottomObjectProperty`
/// (the empty relation), and their `*DataProperty` counterparts. L1 has no way to represent
/// a role whose extension is fixed, so reading one as an ORDINARY named role silently drops
/// that meaning — and the drop is not conservative, it manufactures models. Both W3C
/// `New-Feature-{Top,Bottom}ObjectProperty-001` cases are inconsistent for exactly this
/// reason and NOTHING else: `∃owl:bottomObjectProperty.⊤(i)` is unsatisfiable because the
/// relation is empty, and `¬∃owl:topObjectProperty.⊤(i)` is unsatisfiable because it relates
/// everything (domains are non-empty). Under the opaque-role reading both look satisfiable.
/// Refuse fail-closed, uniformly at every property position — the same discipline as the
/// datatype-map IRI refusal ([`datatype_iri_name`], sq-pbz04.4.9), which addressed the
/// identical failure mode for built-in DATATYPE IRIs. [SONNET-4.6] sq-pbz04.4.8
fn builtin_property_refusal(dict: &Dict, id: Id) -> Option<ExtractError> {
    if is_inline(id) {
        return None;
    }
    let TermParts::Iri { prefix, suffix } = dict.term_parts(id) else {
        return None;
    };
    let iri = format!("{}{}", prefix, suffix);
    match iri.as_str() {
        "http://www.w3.org/2002/07/owl#topObjectProperty"
        | "http://www.w3.org/2002/07/owl#bottomObjectProperty" => {
            Some(ExtractError::OutOfFragment(format!(
                "built-in object property {} (its extension is fixed by the Direct Semantics; \
                 L1 can only represent axiom-constrained named roles)",
                iri
            )))
        }
        "http://www.w3.org/2002/07/owl#topDataProperty"
        | "http://www.w3.org/2002/07/owl#bottomDataProperty" => {
            Some(ExtractError::DataConstruct(format!(
                "built-in data property {} in a property position (no concrete domain in L1)",
                iri
            )))
        }
        _ => None,
    }
}

// -------------------------------------------------------------------------------------------
// Vocabulary + structural index
// -------------------------------------------------------------------------------------------

/// Interned dict ids for the fixed OWL/RDFS/rdf vocabulary the mapping matches on. A term
/// absent from the store gets [`NO_ID`] from `lookup` (matching nothing), exactly like the
/// `sparq-reason` path.
struct Vocab {
    ty: Id,
    sub_class_of: Id,
    equivalent_class: Id,
    disjoint_with: Id,
    sub_property_of: Id,
    domain: Id,
    range: Id,
    thing: Id,
    nothing: Id,
    rdf_nil: Id,
    /// Class-expression / list backbone predicates (inert on their own; consumed in-context).
    backbone: FxHashSet<Id>,
    /// Ignorable built-in annotation / ontology-header predicates.
    annotation: FxHashSet<Id>,
    /// `rdf:type` objects that are ignorable declaration / structural meta-classes.
    declaration_types: FxHashSet<Id>,
    /// `rdf:type` objects that are out-of-fragment meta-classes (property characteristics,
    /// n-ary collections), mapped to a display name.
    out_of_fragment_types: FxHashMap<Id, &'static str>,
    /// Predicates recognised as out-of-fragment LOGICAL constructs, mapped to a display name.
    out_of_fragment: FxHashMap<Id, &'static str>,
    /// Predicates recognised as datatype / data-range constructs, mapped to a display name.
    data_predicate: FxHashMap<Id, &'static str>,
    /// `owl:onProperty` id (used both as backbone and to build the restriction index).
    on_property: Id,
    some_values_from: Id,
    all_values_from: Id,
    intersection_of: Id,
    union_of: Id,
    complement_of: Id,
    inverse_of: Id,
    rdf_first: Id,
    rdf_rest: Id,
    restriction_ty: Id,
    /// Declaration-typing objects that *classify entities* for the disambiguation pass.
    owl_object_property: Id,
    owl_datatype_property: Id,
    owl_annotation_property: Id,
    /// `owl:TransitiveProperty` — recognised as an in-fragment axiom typing ONLY under the
    /// opt-in `dl_transitive` feature ([GPT-5.6] sq-zfwzq); with the feature OFF it stays in
    /// [`Vocab::out_of_fragment_types`] and is refused fail-closed, exactly as before.
    #[cfg(feature = "dl_transitive")]
    owl_transitive_property: Id,
}

/// OWL predicates whose presence is an out-of-fragment LOGICAL construct (design record §5).
const OWL_OUT_OF_FRAGMENT_PREDS: &[&str] = &[
    "inverseOf",
    "sameAs",
    "differentFrom",
    "hasValue",
    "oneOf",
    "hasSelf",
    "onClass",
    "onProperties",
    "propertyChainAxiom",
    "hasKey",
    "disjointUnionOf",
    "equivalentProperty",
    "propertyDisjointWith",
    "members",
    "distinctMembers",
    "sourceIndividual",
    "assertionProperty",
    "targetIndividual",
    "targetValue",
    "maxCardinality",
    "minCardinality",
    "cardinality",
    "maxQualifiedCardinality",
    "minQualifiedCardinality",
    "qualifiedCardinality",
];

/// OWL predicates whose presence is a datatype / data-range construct (no concrete domain in
/// L1 — design record §5).
const OWL_DATA_PREDS: &[&str] = &[
    "onDataRange",
    "onDatatype",
    "withRestrictions",
    "datatypeComplementOf",
];

/// `rdf:type` objects that are ignorable declaration / structural meta-classes.
const DECLARATION_TYPE_LOCALS: &[(&str, &str)] = &[
    (OWL, "Class"),
    (OWL, "ObjectProperty"),
    (OWL, "DatatypeProperty"),
    (OWL, "AnnotationProperty"),
    (OWL, "NamedIndividual"),
    (OWL, "Ontology"),
    (OWL, "Restriction"),
    (OWL, "OntologyProperty"),
    // [OPUS-4.8] Fix 4: owl:DeprecatedClass and owl:DeprecatedProperty are structural
    // meta-classes (OWL 2 §11.2) with no logical content in ALCH — treat as ignorable
    // declarations rather than producing a spurious ClassAssertion.
    (OWL, "DeprecatedClass"),
    (OWL, "DeprecatedProperty"),
    (RDFS, "Class"),
    (RDFS, "Datatype"),
    (RDF, "List"),
    (RDF, "Property"),
];

/// `rdf:type` objects that are out-of-fragment meta-classes (property characteristics / n-ary
/// collections). `TransitiveProperty` is a member ONLY when the opt-in `dl_transitive`
/// feature is OFF — with the feature ON it is an in-fragment axiom typing recognised by
/// [`classify_type`] instead ([GPT-5.6] sq-zfwzq).
const OUT_OF_FRAGMENT_TYPE_LOCALS: &[&str] = &[
    #[cfg(not(feature = "dl_transitive"))]
    "TransitiveProperty",
    "FunctionalProperty",
    "InverseFunctionalProperty",
    "SymmetricProperty",
    "AsymmetricProperty",
    "ReflexiveProperty",
    "IrreflexiveProperty",
    "AllDifferent",
    "AllDisjointClasses",
    "AllDisjointProperties",
    "NegativePropertyAssertion",
];

/// Ignorable built-in annotation / ontology-header predicates.
const ANNOTATION_LOCALS: &[(&str, &str)] = &[
    (RDFS, "label"),
    (RDFS, "comment"),
    (RDFS, "seeAlso"),
    (RDFS, "isDefinedBy"),
    (OWL, "versionInfo"),
    (OWL, "versionIRI"),
    (OWL, "priorVersion"),
    (OWL, "backwardCompatibleWith"),
    (OWL, "incompatibleWith"),
    (OWL, "deprecated"),
    (OWL, "imports"),
];

impl Vocab {
    fn intern(dict: &Dict) -> Vocab {
        use oxrdf::{NamedNode, Term as OTerm};
        let look = |iri: String| dict.lookup(&OTerm::NamedNode(NamedNode::new_unchecked(iri)));
        let owl = |local: &str| look(format!("{}{}", OWL, local));

        let on_property = owl("onProperty");
        let some_values_from = owl("someValuesFrom");
        let all_values_from = owl("allValuesFrom");
        let intersection_of = owl("intersectionOf");
        let union_of = owl("unionOf");
        let complement_of = owl("complementOf");
        let inverse_of = owl("inverseOf");
        let rdf_first = look(format!("{}first", RDF));
        let rdf_rest = look(format!("{}rest", RDF));

        let mut backbone = FxHashSet::default();
        for id in [
            on_property,
            some_values_from,
            all_values_from,
            intersection_of,
            union_of,
            complement_of,
            rdf_first,
            rdf_rest,
        ] {
            if id != NO_ID {
                backbone.insert(id);
            }
        }

        let mut annotation = FxHashSet::default();
        for &(ns, local) in ANNOTATION_LOCALS {
            let id = look(format!("{}{}", ns, local));
            if id != NO_ID {
                annotation.insert(id);
            }
        }

        let mut declaration_types = FxHashSet::default();
        for &(ns, local) in DECLARATION_TYPE_LOCALS {
            let id = look(format!("{}{}", ns, local));
            if id != NO_ID {
                declaration_types.insert(id);
            }
        }

        let mut out_of_fragment_types = FxHashMap::default();
        for &local in OUT_OF_FRAGMENT_TYPE_LOCALS {
            let id = owl(local);
            if id != NO_ID {
                out_of_fragment_types.insert(id, local);
            }
        }

        let mut out_of_fragment = FxHashMap::default();
        for &local in OWL_OUT_OF_FRAGMENT_PREDS {
            let id = owl(local);
            if id != NO_ID {
                out_of_fragment.insert(id, local);
            }
        }

        let mut data_predicate = FxHashMap::default();
        for &local in OWL_DATA_PREDS {
            let id = owl(local);
            if id != NO_ID {
                data_predicate.insert(id, local);
            }
        }

        Vocab {
            ty: look(format!("{}type", RDF)),
            sub_class_of: look(format!("{}subClassOf", RDFS)),
            equivalent_class: owl("equivalentClass"),
            disjoint_with: owl("disjointWith"),
            sub_property_of: look(format!("{}subPropertyOf", RDFS)),
            domain: look(format!("{}domain", RDFS)),
            range: look(format!("{}range", RDFS)),
            thing: owl("Thing"),
            nothing: owl("Nothing"),
            rdf_nil: look(format!("{}nil", RDF)),
            backbone,
            annotation,
            declaration_types,
            out_of_fragment_types,
            out_of_fragment,
            data_predicate,
            on_property,
            some_values_from,
            all_values_from,
            intersection_of,
            union_of,
            complement_of,
            inverse_of,
            rdf_first,
            rdf_rest,
            restriction_ty: owl("Restriction"),
            owl_object_property: owl("ObjectProperty"),
            owl_datatype_property: owl("DatatypeProperty"),
            owl_annotation_property: owl("AnnotationProperty"),
            #[cfg(feature = "dl_transitive")]
            owl_transitive_property: owl("TransitiveProperty"),
        }
    }
}

/// The one-pass structural index the extractor builds over the triples: the class-expression
/// backbone edges, the restriction typing, the entity declarations used to disambiguate
/// assertions, and the per-node out-of-fragment / datatype shape markers.
#[derive(Default)]
struct Index {
    on_property: FxHashMap<Id, Id>,
    some_values_from: FxHashMap<Id, Id>,
    all_values_from: FxHashMap<Id, Id>,
    intersection_of: FxHashMap<Id, Id>,
    union_of: FxHashMap<Id, Id>,
    complement_of: FxHashMap<Id, Id>,
    first: FxHashMap<Id, Id>,
    rest: FxHashMap<Id, Id>,
    is_restriction: FxHashSet<Id>,
    /// Nodes that back a class expression / list — never valid as a ground individual.
    structural_nodes: FxHashSet<Id>,
    /// Nodes carrying an out-of-fragment class shape predicate (`owl:oneOf`, cardinality, …),
    /// mapped to a display name.
    out_of_fragment_shape: FxHashMap<Id, &'static str>,
    /// Nodes carrying a datatype / data-range shape predicate, mapped to a display name.
    data_shape: FxHashMap<Id, &'static str>,
    /// Nodes appearing as the object of `owl:inverseOf` — inverse property expressions.
    inverse_of: FxHashSet<Id>,
    /// Entities declared (or used) as object properties — enables sound role-assertion mapping.
    object_props: FxHashSet<Id>,
    /// Entities declared as datatype properties.
    data_props: FxHashSet<Id>,
    /// Entities declared as annotation properties.
    annotation_props: FxHashSet<Id>,
    /// Entities declared as MORE THAN ONE property type (e.g. both AnnotationProperty and
    /// ObjectProperty) — a punning clash that makes their use in triples Unclassifiable.
    /// [OPUS-4.8] Fix 3.
    punned_props: FxHashSet<Id>,
}

impl Index {
    // [OPUS-4.8] Fix 1 + Fix 3: returns Err on a branching list / duplicate backbone
    // value (different value for same functional-position predicate on same node), or on a
    // cross-type punned declaration. Identical re-assertion (same value) is silently allowed.
    fn build(triples: &[[Id; 3]], v: &Vocab) -> Result<Index, ExtractError> {
        let mut idx = Index::default();
        for &[s, p, o] in triples {
            if p == v.on_property {
                // owl:onProperty is functional on a restriction node — exactly one value.
                if let Some(prev) = idx.on_property.insert(s, o) {
                    if prev != o {
                        return Err(ExtractError::MalformedClassExpression(
                            "restriction node has two conflicting owl:onProperty values"
                                .to_string(),
                        ));
                    }
                }
                idx.structural_nodes.insert(s);
                // [SONNET-4.6] sq-pbz04.4.1 order-asymmetry fix: when usage-typing would
                // add `o` to object_props, check whether `o` is already in annotation_props
                // or data_props (a declaration that arrived earlier in the triples slice).
                // Symmetric to the declaration arms: records the pun so the verdict is
                // order-independent (RDF graphs are sets; declaration before usage must
                // produce the same Unclassifiable refusal as usage before declaration).
                if idx.annotation_props.contains(&o) || idx.data_props.contains(&o) {
                    idx.punned_props.insert(o);
                }
                idx.object_props.insert(o); // usage-based typing
            } else if p == v.some_values_from {
                if let Some(prev) = idx.some_values_from.insert(s, o) {
                    if prev != o {
                        return Err(ExtractError::MalformedClassExpression(
                            "restriction node has two conflicting owl:someValuesFrom values"
                                .to_string(),
                        ));
                    }
                }
                idx.structural_nodes.insert(s);
            } else if p == v.all_values_from {
                if let Some(prev) = idx.all_values_from.insert(s, o) {
                    if prev != o {
                        return Err(ExtractError::MalformedClassExpression(
                            "restriction node has two conflicting owl:allValuesFrom values"
                                .to_string(),
                        ));
                    }
                }
                idx.structural_nodes.insert(s);
            } else if p == v.intersection_of {
                if let Some(prev) = idx.intersection_of.insert(s, o) {
                    if prev != o {
                        return Err(ExtractError::MalformedClassExpression(
                            "class node has two conflicting owl:intersectionOf values"
                                .to_string(),
                        ));
                    }
                }
                idx.structural_nodes.insert(s);
            } else if p == v.union_of {
                if let Some(prev) = idx.union_of.insert(s, o) {
                    if prev != o {
                        return Err(ExtractError::MalformedClassExpression(
                            "class node has two conflicting owl:unionOf values".to_string(),
                        ));
                    }
                }
                idx.structural_nodes.insert(s);
            } else if p == v.complement_of {
                if let Some(prev) = idx.complement_of.insert(s, o) {
                    if prev != o {
                        return Err(ExtractError::MalformedClassExpression(
                            "class node has two conflicting owl:complementOf values".to_string(),
                        ));
                    }
                }
                idx.structural_nodes.insert(s);
            } else if p == v.inverse_of {
                idx.inverse_of.insert(s);
            } else if p == v.rdf_first {
                // rdf:first is functional — each list cell carries exactly one member.
                if let Some(prev) = idx.first.insert(s, o) {
                    if prev != o {
                        return Err(ExtractError::MalformedList(
                            "list cell has two conflicting rdf:first values (branching list)"
                                .to_string(),
                        ));
                    }
                }
                idx.structural_nodes.insert(s);
            } else if p == v.rdf_rest {
                // rdf:rest is functional — each list cell carries exactly one continuation.
                if let Some(prev) = idx.rest.insert(s, o) {
                    if prev != o {
                        return Err(ExtractError::MalformedList(
                            "list cell has two conflicting rdf:rest values (branching list)"
                                .to_string(),
                        ));
                    }
                }
                idx.structural_nodes.insert(s);
            } else if p == v.ty {
                if o == v.restriction_ty {
                    idx.is_restriction.insert(s);
                    idx.structural_nodes.insert(s);
                } else if o == v.owl_object_property {
                    // Fix 3: track cross-type punning on declaration.
                    if idx.data_props.contains(&s) || idx.annotation_props.contains(&s) {
                        idx.punned_props.insert(s);
                    }
                    idx.object_props.insert(s);
                } else if o == v.owl_datatype_property {
                    if idx.object_props.contains(&s) || idx.annotation_props.contains(&s) {
                        idx.punned_props.insert(s);
                    }
                    idx.data_props.insert(s);
                } else if o == v.owl_annotation_property {
                    if idx.object_props.contains(&s) || idx.data_props.contains(&s) {
                        idx.punned_props.insert(s);
                    }
                    idx.annotation_props.insert(s);
                }
                // [GPT-5.6] sq-zfwzq (opt-in `dl_transitive`): `s a owl:TransitiveProperty`
                // types `s` as an OBJECT property (OWL 2 RDF mapping — the typing declares an
                // object property), with the same symmetric cross-type pun-check as the
                // declaration arms above so the Unclassifiable verdict is order-independent.
                #[cfg(feature = "dl_transitive")]
                if o == v.owl_transitive_property {
                    if idx.data_props.contains(&s) || idx.annotation_props.contains(&s) {
                        idx.punned_props.insert(s);
                    }
                    idx.object_props.insert(s);
                }
            } else if let Some(name) = v.out_of_fragment.get(&p) {
                // An out-of-fragment predicate makes its subject an out-of-fragment class
                // shape (so a `∃r.{a}` node is refused, not read as a named class), AND is
                // caught again standalone in `classify_triple` (so a top-level `owl:sameAs`
                // is refused even if its subject is never referenced as a class).
                idx.out_of_fragment_shape.insert(s, name);
            } else if let Some(name) = v.data_predicate.get(&p) {
                idx.data_shape.insert(s, name);
            } else if p == v.sub_property_of || p == v.domain || p == v.range {
                // A property used in an object-property axiom position is known to be an
                // object property (usage-based typing) — but only the subject/relevant operand.
                // [SONNET-4.6] sq-pbz04.4.1 order-asymmetry fix: symmetric pun-check for
                // each IRI being typed as object property via usage, matching the on_property arm.
                if p == v.sub_property_of {
                    if idx.annotation_props.contains(&s) || idx.data_props.contains(&s) {
                        idx.punned_props.insert(s);
                    }
                    idx.object_props.insert(s);
                    if idx.annotation_props.contains(&o) || idx.data_props.contains(&o) {
                        idx.punned_props.insert(o);
                    }
                    idx.object_props.insert(o);
                } else {
                    if idx.annotation_props.contains(&s) || idx.data_props.contains(&s) {
                        idx.punned_props.insert(s);
                    }
                    idx.object_props.insert(s);
                }
            }
        }
        Ok(idx)
    }
}

// -------------------------------------------------------------------------------------------
// Node-kind helpers
// -------------------------------------------------------------------------------------------

/// `true` iff `id` denotes an RDF literal (inline integer or an interned literal).
fn is_literal(dict: &Dict, id: Id) -> bool {
    is_inline(id) || matches!(dict.term_parts(id), TermParts::Lit { .. })
}

/// `true` iff `id` denotes an RDF 1.2 triple term (quoted triple).
fn is_triple_term(dict: &Dict, id: Id) -> bool {
    !is_inline(id) && matches!(dict.term_parts(id), TermParts::Triple(_))
}

/// Renders `id` as a short diagnostic string (the IRI/blank/literal text). Error-path only.
fn term_iri(dict: &Dict, id: Id) -> String {
    if is_inline(id) {
        return format!("(inline literal id {})", id);
    }
    match dict.term_parts(id) {
        TermParts::Iri { prefix, suffix } => format!("<{}{}>", prefix, suffix),
        TermParts::Lit { value, .. } => format!("\"{}\"", value),
        TermParts::Blank(label) => format!("_:{}", label),
        TermParts::Triple(_) => "<<triple term>>".to_string(),
    }
}

/// The XSD namespace: every IRI under it is an OWL 2 datatype-map datatype ([FABLE-5]
/// sq-pbz04.4.9, per the escalated soundness verdict).
const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

/// The non-XSD members of the OWL 2 datatype map recognised by name (design record §5): the
/// datatypes whose IRIs live outside the XSD namespace.
const NON_XSD_DATATYPE_IRIS: &[&str] = &[
    "http://www.w3.org/2000/01/rdf-schema#Literal",
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#PlainLiteral",
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#XMLLiteral",
    "http://www.w3.org/2002/07/owl#real",
    "http://www.w3.org/2002/07/owl#rational",
];

/// `Some(display)` when `id` names a well-known OWL 2 datatype-map datatype (an `xsd:*` IRI or
/// one of the [`NON_XSD_DATATYPE_IRIS`]). A bare datatype IRI carries CONCRETE-DOMAIN meaning
/// (a fixed value space) that the datatype-free ALCH fragment cannot model; reading it as an
/// opaque object class would SILENTLY DROP that meaning (design record §5 defers datatypes
/// pending a concrete-domain oracle). Detected by IRI identity, so a datatype reaching ANY
/// class-expression position fails closed uniformly — the same discipline as the literal /
/// `owl:onDatatype` / declared-datatype-property refusals. [FABLE-5] sq-pbz04.4.9
fn datatype_iri_name(dict: &Dict, id: Id) -> Option<String> {
    if is_inline(id) {
        return None;
    }
    if let TermParts::Iri { prefix, suffix } = dict.term_parts(id) {
        let iri = format!("{}{}", prefix, suffix);
        if iri.starts_with(XSD) || NON_XSD_DATATYPE_IRIS.contains(&iri.as_str()) {
            return Some(iri);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Axiom;
    use sparq_core::Graph;

    /// The five taxonomy arms each render a non-empty diagnostic (direct coverage of the
    /// `Display` impl, which the fail-closed error paths rely on for operator diagnostics).
    #[test]
    fn extract_error_display_covers_every_arm() {
        let arms = [
            ExtractError::OutOfFragment("x".into()),
            ExtractError::DataConstruct("x".into()),
            ExtractError::MalformedList("x".into()),
            ExtractError::MalformedClassExpression("x".into()),
            ExtractError::Unclassifiable("x".into()),
        ];
        for e in arms {
            let s = format!("{}", e);
            assert!(s.contains('x'), "diagnostic should carry the detail: {}", s);
            // Exercise the std::error::Error impl too.
            let _dyn: &dyn std::error::Error = &e;
        }
    }

    /// Direct unit test of the public `extract` entry point: a one-axiom accept and a
    /// fail-closed reject, so the entry point is covered independently of the integration suite.
    #[test]
    fn extract_direct_accept_and_reject() {
        let (dict, triples) = Graph::parse_to_triples(
            "@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
             <http://ex/A> rdfs:subClassOf <http://ex/B> .",
            "turtle",
        )
        .expect("parse");
        let onto = extract(&dict, &triples).expect("accept");
        assert_eq!(onto.len(), 1);
        assert!(matches!(onto.axioms[0], Axiom::SubClassOf { .. }));

        let (dict, triples) = Graph::parse_to_triples(
            "@prefix owl: <http://www.w3.org/2002/07/owl#> .\n\
             <http://ex/a> owl:sameAs <http://ex/b> .",
            "turtle",
        )
        .expect("parse");
        assert!(matches!(
            extract(&dict, &triples),
            Err(ExtractError::OutOfFragment(_))
        ));
    }

    /// Direct coverage of the node-kind helpers over a graph with an IRI, a literal, and a
    /// blank node.
    #[test]
    fn node_kind_helpers() {
        let (dict, _t) = Graph::parse_to_triples(
            "<http://ex/s> <http://ex/p> \"lit\" .\n<http://ex/s> <http://ex/q> _:b .",
            "turtle",
        )
        .expect("parse");
        use oxrdf::{Literal, NamedNode, Term as OTerm};
        let s = dict.lookup(&OTerm::NamedNode(NamedNode::new_unchecked("http://ex/s")));
        let lit = dict.lookup(&OTerm::Literal(Literal::new_simple_literal("lit")));
        assert!(!is_literal(&dict, s));
        assert!(is_literal(&dict, lit));
        assert!(!is_triple_term(&dict, s));
        assert!(term_iri(&dict, s).contains("ex/s"));
        assert!(term_iri(&dict, lit).contains("lit"));
    }

    /// Direct unit test of `datatype_iri_name` ([FABLE-5] sq-pbz04.4.9): `xsd:*` IRIs and the
    /// named non-XSD datatype-map members are recognised; an ordinary class IRI, a literal, and
    /// `owl:Thing` are NOT.
    #[test]
    fn datatype_iri_name_recognises_the_datatype_map() {
        let (dict, _t) = Graph::parse_to_triples(
            "@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n\
             @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
             @prefix owl: <http://www.w3.org/2002/07/owl#> .\n\
             <http://ex/s> <http://ex/p> xsd:integer , xsd:string , rdfs:Literal , owl:real .\n\
             <http://ex/s> <http://ex/q> <http://ex/A> , owl:Thing , \"lit\" .",
            "turtle",
        )
        .expect("parse");
        use oxrdf::{Literal, NamedNode, Term as OTerm};
        let look = |iri: &str| dict.lookup(&OTerm::NamedNode(NamedNode::new_unchecked(iri)));
        // Recognised datatype-map members.
        for iri in [
            "http://www.w3.org/2001/XMLSchema#integer",
            "http://www.w3.org/2001/XMLSchema#string",
            "http://www.w3.org/2000/01/rdf-schema#Literal",
            "http://www.w3.org/2002/07/owl#real",
        ] {
            assert_eq!(datatype_iri_name(&dict, look(iri)).as_deref(), Some(iri));
        }
        // NOT datatypes: an ordinary class IRI, owl:Thing, a literal.
        assert_eq!(datatype_iri_name(&dict, look("http://ex/A")), None);
        assert_eq!(
            datatype_iri_name(&dict, look("http://www.w3.org/2002/07/owl#Thing")),
            None
        );
        let lit = dict.lookup(&OTerm::Literal(Literal::new_simple_literal("lit")));
        assert_eq!(datatype_iri_name(&dict, lit), None);
    }

    /// End-to-end fail-closed refusal ([FABLE-5] sq-pbz04.4.9): a datatype IRI reaching a
    /// class-expression position (an `rdfs:range` object here, but the check is position-uniform)
    /// refuses extraction as a `DataConstruct` — it is NOT read as an opaque object class. This
    /// is the WebOnt-I5.3-015 mechanism (two disjoint datatype ranges must not silently extract
    /// to two unrelated opaque classes).
    #[test]
    fn datatype_range_object_refuses_extraction() {
        let (dict, triples) = Graph::parse_to_triples(
            "@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
             @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n\
             <http://ex/p> rdfs:range xsd:integer .",
            "turtle",
        )
        .expect("parse");
        assert!(
            matches!(extract(&dict, &triples), Err(ExtractError::DataConstruct(_))),
            "a datatype range object must refuse extraction (DataConstruct)"
        );
        // A datatype as a subClassOf super-CE refuses just the same (position-uniform).
        let (dict, triples) = Graph::parse_to_triples(
            "@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
             @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n\
             <http://ex/A> rdfs:subClassOf xsd:string .",
            "turtle",
        )
        .expect("parse");
        assert!(matches!(
            extract(&dict, &triples),
            Err(ExtractError::DataConstruct(_))
        ));
    }

    /// Direct unit test of `normalize_intersection` (sq-pbz04.4.16 / M7): a one-member
    /// operand list collapses to its sole member; a ≥ 2-member list is preserved as an
    /// `ObjectIntersectionOf`, keeping the profile-grammar arity rule intact.
    #[test]
    fn normalize_intersection_collapses_singleton_only() {
        // Singleton → the member itself (using dict-free synthetic ids for the members).
        let a = ClassExpression::Class(7);
        assert_eq!(normalize_intersection(vec![a.clone()]), a);
        // Two members → preserved as an intersection (arity rule unaffected).
        let b = ClassExpression::Class(9);
        let two = normalize_intersection(vec![a.clone(), b.clone()]);
        assert_eq!(two, ClassExpression::ObjectIntersectionOf(vec![a, b]));
    }

    /// End-to-end M7: the OWL-1-era singleton encoding `_:B owl:intersectionOf ( :A )` used
    /// as a subClassOf super-CE extracts to the atomic member `Class(A)` — NOT a 1-ary
    /// `ObjectIntersectionOf` the profile-grammar arity rule would reject (sq-pbz04.4.16).
    /// A genuine 2-member intersection still extracts as an `ObjectIntersectionOf`.
    #[test]
    fn extract_singleton_intersection_normalizes_to_member() {
        // `:S rdfs:subClassOf [ owl:intersectionOf ( :A ) ]`.
        let (dict, triples) = Graph::parse_to_triples(
            "@prefix owl: <http://www.w3.org/2002/07/owl#> .\n\
             @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
             <http://ex/S> rdfs:subClassOf [ owl:intersectionOf ( <http://ex/A> ) ] .",
            "turtle",
        )
        .expect("parse");
        let onto = extract(&dict, &triples).expect("accept");
        let sup = onto
            .axioms
            .iter()
            .find_map(|ax| match ax {
                Axiom::SubClassOf { sup, .. } => Some(sup),
                _ => None,
            })
            .expect("a SubClassOf axiom");
        // The singleton intersection collapsed to the atomic member (never a 1-ary
        // ObjectIntersectionOf).
        assert!(
            matches!(sup, ClassExpression::Class(_)),
            "singleton intersection should normalize to its member, got {:?}",
            sup
        );

        // A genuine 2-member intersection is preserved.
        let (dict, triples) = Graph::parse_to_triples(
            "@prefix owl: <http://www.w3.org/2002/07/owl#> .\n\
             @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
             <http://ex/S> rdfs:subClassOf [ owl:intersectionOf ( <http://ex/A> <http://ex/B> ) ] .",
            "turtle",
        )
        .expect("parse");
        let onto = extract(&dict, &triples).expect("accept");
        let sup = onto
            .axioms
            .iter()
            .find_map(|ax| match ax {
                Axiom::SubClassOf { sup, .. } => Some(sup),
                _ => None,
            })
            .expect("a SubClassOf axiom");
        assert!(
            matches!(sup, ClassExpression::ObjectIntersectionOf(m) if m.len() == 2),
            "2-member intersection should be preserved, got {:?}",
            sup
        );
    }
}
