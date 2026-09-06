// [OPUS-4.8] sq-evb1: extract an EL+⊥ TBox from dict-encoded RDF triples.
//
// Reads the OWL axioms an EL classifier needs out of the `(Dict, Vec<[Id;3]>)` substrate and
// produces a `Vec<Normal>` plus the `Names` table. Recognised constructs (EL+⊥ minus RBox):
//
//   rdfs:subClassOf            C ⊑ D
//   owl:equivalentClass        C ≡ D  (both directions)
//   owl:intersectionOf (list)  C ⊓ … ⊓ Cn  (as a class expression node)
//   owl:Restriction + owl:onProperty + owl:someValuesFrom   ∃r.C
//   owl:Restriction + owl:onProperty + owl:hasValue         ∃r.{a}   (object value — CR6)
//   owl:Restriction + owl:onProperty + owl:hasSelf true      ∃r.Self  (local reflexivity — CR-Self)
//   owl:oneOf ( a )            {a}  — a SINGLETON enumeration (nominal, CR6)
//   owl:disjointWith           C ⊓ D ⊑ ⊥
//   owl:Thing / owl:Nothing    ⊤ / ⊥
//
// [FABLE-5] sq-pbz04.2.1: nominals are the OWL 2 EL profile's `ObjectOneOf` — which admits
// EXACTLY ONE individual — and `ObjectHasValue` (`∃r.{a}`). A multi-individual `owl:oneOf`
// is a disjunction `{a} ⊔ {b}` (outside EL), and a LITERAL-valued `owl:hasValue`/`owl:oneOf`
// is `DataHasValue`/`DataOneOf` (concrete domains, the deferred CR7–CR9 surface); both stay
// recorded as skips. Anything else NOT in this fragment (unionOf, complementOf,
// allValuesFrom, cardinality, property chains, …) is OUT OF the recognised fragment and is
// recorded as a skip in the [`crate::Report`] rather than silently dropped — so a user
// running `el` over a non-EL ontology gets an honest "n axioms outside the EL fragment were
// ignored" count.

use crate::normal::{Expr, Names, Normal, Normalizer, BOTTOM, TOP};
// [OPUS-4.8] sq-pbz04.2.8: the internal `Concept`/`Role` index types are also needed by the E4
// ABox extras (`owl:hasKey` / negative-property-assertion / `owl:differentFrom`) under `abox`.
#[cfg(any(feature = "cdomain", feature = "abox"))]
use crate::normal::Concept;
#[cfg(any(feature = "rbox", feature = "abox"))]
use crate::normal::Role;
#[cfg(feature = "rbox")]
use crate::normal::RoleAxiom;
use crate::Report;
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::{Dict, Id};

/// Knobs controlling what [`extract`] reads. Default = TBox only (the byte-identical E1 path
/// every existing caller uses). [OPUS-4.8] sq-pbz04.2.5: under the `abox` feature the ABox
/// realisation entry (`crate::abox::realize`) sets `abox = true` to ALSO internalize
/// `ClassAssertion`/`ObjectPropertyAssertion` triples as safe-nominal axioms. The struct is a
/// zero-sized unit without the feature, so `Classifier::classify` / `classify_graph` — which
/// always pass `ExtractOpts::default()` — are byte-identical regardless of the feature.
#[derive(Clone, Copy, Default)]
pub struct ExtractOpts {
    /// Internalize ABox assertions (only meaningful under the `abox` feature).
    #[cfg(feature = "abox")]
    pub abox: bool,
}

/// Everything the extractor reads out of the RDF substrate: the normalized concept axioms, the
/// name table, the [`Report`], and — only under the `rbox` feature — the normalized RBox role
/// axioms (E2). Keeping the role axioms in a feature-gated field means the default build's
/// extraction is byte-for-byte the E1 path.
pub struct Extracted {
    pub axioms: Vec<Normal>,
    pub names: Names,
    pub report: Report,
    /// Normalized RBox axioms (role inclusions + compositions). Empty/absent without `rbox`.
    #[cfg(feature = "rbox")]
    pub role_axioms: Vec<RoleAxiom>,
    /// [OPUS-4.8] sq-pbz04.2.8 (`abox`): the E4 ABox axioms (`owl:hasKey` / negative property
    /// assertions / asserted `owl:differentFrom`) the realiser reasons over post-saturation.
    /// Empty unless `opts.abox` (the `Classifier::classify` path never populates it).
    #[cfg(feature = "abox")]
    pub abox_extras: AboxExtras,
}

/// [OPUS-4.8] sq-pbz04.2.8 (`abox`): the E4 ABox axioms the realiser reads off the saturation on
/// top of the class/property assertions — `owl:hasKey` keys, negative property assertions, and
/// asserted `owl:differentFrom` inequalities. Every concept / role / nominal referenced here is
/// minted through the SAME [`Names`] table the saturation runs over, so the ids agree.
#[cfg(feature = "abox")]
#[derive(Default)]
pub struct AboxExtras {
    /// Well-formed `owl:hasKey` axioms (malformed ones are counted in `Report::skipped_assertions`).
    pub keys: Vec<AboxKey>,
    /// Negative object/data property assertions.
    pub npas: Vec<AboxNpa>,
    /// Asserted `owl:differentFrom` pairs `(a, b)` as found (one direction); the realiser emits the
    /// symmetric closure.
    pub different_from: Vec<(Id, Id)>,
}

/// [OPUS-4.8] sq-pbz04.2.8 (`abox`): a supported `owl:hasKey` axiom — the key CLASS (as a
/// saturation concept, so "a is derivably in the key class" is the membership test `class ∈ S({a})`)
/// and the key PROPERTIES, each as `(role, property dict id)`. Both key kinds are matched on a
/// SHARED value: an object key on a shared nominal successor (asserted or derived), a data key on a
/// shared literal TERM (sound — identical terms denote identical values; value-equal-but-lexically-
/// distinct literals are an honest incompleteness pending the `cdomain` numeric tower).
#[cfg(feature = "abox")]
pub struct AboxKey {
    pub class: Concept,
    pub props: Vec<(Role, Id)>,
}

/// [OPUS-4.8] sq-pbz04.2.8 (`abox`): a negative property assertion. An `Object` NPA is a clash iff
/// the positive `a p b` is ASSERTED or DERIVED (a nominal successor of `{a}` via `p`); a `Data` NPA
/// is a clash iff the positive `a p v` is ASSERTED (data-property positives are not internalized, so
/// a derived data positive is an honest incompleteness — never an unsound miss of a clash).
#[cfg(feature = "abox")]
pub enum AboxNpa {
    /// Negative OBJECT property assertion `¬(a p b)`.
    Object {
        source_dict: Id,
        prop_dict: Id,
        target_dict: Id,
        source_c: Concept,
        role: Role,
        target_c: Concept,
    },
    /// Negative DATA property assertion `¬(a p "v")`.
    Data {
        source_dict: Id,
        prop_dict: Id,
        value_dict: Id,
    },
}

const OWL: &str = "http://www.w3.org/2002/07/owl#";
const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const RDFS: &str = "http://www.w3.org/2000/01/rdf-schema#";
const XSD_BOOLEAN: &str = "http://www.w3.org/2001/XMLSchema#boolean";

/// OWL class-expression / restriction predicates that lie OUTSIDE the EL+⊥ fragment this
/// classifier implements. A class node carrying any of these is non-EL: [`decode`] returns
/// `None` so the enclosing axiom is recorded as a skip rather than the node being mistaken for
/// an opaque named class.
///
/// One family marks the concrete-domain fragment (CR7–CR9, spike §"Hard parts" / the EL
/// track of `research/owl2-el-ql-reasoning-spike.md`). Rather than silently treat those
/// nodes as opaque classes (which would drop their real semantics and risk a wrong
/// answer), we route them to `skipped_axioms`:
///   * `onDataRange` / `withRestrictions` / `onDatatype` / `datatypeComplementOf` — CONCRETE
///     DOMAINS (datatype restrictions / faceted ranges). [FABLE-5] sq-pbz04.2.2: these STAY
///     markers even under `cdomain` — the feature does not weaken the skip default; it
///     RESCUES exactly the nodes `resolve_cdomain` proves supported (an exact-numeric
///     faceted `onDatatype`/`withRestrictions` range) via the `decode` interception, while
///     `onDataRange` (qualified-cardinality vocabulary, outside EL) and
///     `datatypeComplementOf` (negation) are never rescued.
///
/// [FABLE-5] sq-pbz04.2.1: `oneOf`/`hasValue` are NO LONGER blanket markers — the safe-nominal
/// slice (a SINGLETON object `owl:oneOf` and an object-valued `owl:hasValue`) is now decoded
/// into nominal concepts and reasoned over by CR6. The out-of-fragment nominal shapes (a
/// multi-individual `oneOf` = disjunction; a literal-valued `oneOf`/`hasValue` = concrete
/// domain) are detected structurally in [`extract`]/[`decode`] and still counted as skips.
///
/// [OPUS-4.8] sq-pbz04.2.6: `hasSelf` is NO LONGER a marker — `owl:hasSelf "true"^^xsd:boolean`
/// with `owl:onProperty` is the OWL 2 EL profile's `ObjectHasSelf` (`∃r.Self`, the LOCAL
/// reflexivity concept), decoded into a self-restriction concept and reasoned over by the
/// completion rules CR-Self-1/CR-Self-2 (see `classify.rs`). Any OTHER `owl:hasSelf` shape (a
/// non-`true` / non-boolean object, a missing `owl:onProperty`, or extra structure on the node)
/// stays a COUNTED skip — fail-closed, never guessed.
///
/// The remainder (`unionOf`/`complementOf`/`allValuesFrom`/cardinality) are outside EL entirely
/// (they need ALC / Horn-SHIQ expressivity), not a deferred EL slice.
const NON_EL_MARKERS: &[&str] = &[
    "unionOf",       // disjunction — outside EL
    "complementOf",  // negation — outside EL
    "allValuesFrom", // universal restriction — outside EL
    "minCardinality",
    "maxCardinality",
    "cardinality",
    "minQualifiedCardinality",
    "maxQualifiedCardinality",
    "qualifiedCardinality",
    // --- Deferred EL fragment: concrete domains (CR7–CR9) ---------------------------------
    "onDataRange",          // qualified data-range restriction (concrete domain)
    "withRestrictions",     // faceted datatype restriction (concrete domain)
    "onDatatype",           // datatype-restriction base datatype (concrete domain)
    "datatypeComplementOf", // datatype negation (concrete domain)
];

/// The dict ids of the vocabulary terms the extractor matches on. A term absent from the
/// store gets `NO_ID` from `lookup` (matching nothing), exactly like the RL path.
struct Vocab {
    ty: Id,
    sub_class_of: Id,
    equivalent_class: Id,
    intersection_of: Id,
    on_property: Id,
    some_values_from: Id,
    disjoint_with: Id,
    restriction: Id,
    /// [FABLE-5] sq-pbz04.2.1: the safe-nominal (CR6) vocabulary.
    one_of: Id,
    has_value: Id,
    /// [OPUS-4.8] sq-pbz04.2.6: `owl:hasSelf` — the local-reflexivity restriction predicate
    /// (`ObjectHasSelf`). A node carrying `owl:hasSelf "true"^^xsd:boolean` + `owl:onProperty r`
    /// decodes to the self-restriction concept `∃r.Self` (CR-Self); any other object/shape is a
    /// counted skip.
    has_self: Id,
    rdf_first: Id,
    rdf_rest: Id,
    rdf_nil: Id,
    thing: Id,
    nothing: Id,
    /// [FABLE-5] sq-pbz04.2.2: the concrete-domain (CR7–CR9) vocabulary. Both predicates
    /// STAY in [`NON_EL_MARKERS`] (so the default skip path is untouched); under `cdomain`
    /// their structure is ALSO collected and supported ranges are rescued in [`decode`].
    #[cfg(feature = "cdomain")]
    on_datatype: Id,
    #[cfg(feature = "cdomain")]
    with_restrictions: Id,
    /// [OPUS-4.8] sq-pbz04.2.5 (`abox`): `owl:bottomObjectProperty` — the empty object property.
    /// When it occurs as a role, the extractor appends `∃⊥.⊤ ⊑ ⊥` so any `∃⊥.C` obligation
    /// (typically an ABox instance of `∃⊥.⊤`) collapses to `⊥` (`NO_ID` if the term is absent,
    /// which `Names::role_of` never resolves).
    #[cfg(feature = "abox")]
    bottom_object_property: Id,
    /// [OPUS-4.8] sq-pbz04.2.8 (`abox`): the E4 ABox vocabulary — `owl:hasKey` (keys), the
    /// negative-property-assertion reification predicates (`owl:sourceIndividual` /
    /// `owl:assertionProperty` / `owl:targetIndividual` / `owl:targetValue`), and
    /// `owl:differentFrom`. Only consulted by [`decode_abox_e4`] under `opts.abox`; interned
    /// (read-only) but unused on the `Classifier::classify` path, so that path stays byte-identical.
    #[cfg(feature = "abox")]
    has_key: Id,
    #[cfg(feature = "abox")]
    source_individual: Id,
    #[cfg(feature = "abox")]
    assertion_property: Id,
    #[cfg(feature = "abox")]
    target_individual: Id,
    #[cfg(feature = "abox")]
    target_value: Id,
    #[cfg(feature = "abox")]
    different_from: Id,
    /// Predicate ids whose presence on a class node makes it non-EL (see [`NON_EL_MARKERS`]).
    non_el: FxHashSet<Id>,
    /// RBox vocabulary — only consulted under the `rbox` feature (E2).
    #[cfg(feature = "rbox")]
    sub_property_of: Id,
    #[cfg(feature = "rbox")]
    property_chain_axiom: Id,
    #[cfg(feature = "rbox")]
    transitive_property: Id,
    /// `owl:topObjectProperty` — the universal object property. The OWL 2 property-hierarchy
    /// restriction unconditionally admits every chain axiom whose SUPERPROPERTY is top, so its
    /// role id (when minted) is threaded into [`crate::rbox::told_rbox_regular`] (`NO_ID` if
    /// the term is absent, which `Names::role_of` never resolves).
    #[cfg(feature = "rbox")]
    top_object_property: Id,
}

impl Vocab {
    fn intern(dict: &Dict) -> Vocab {
        use oxrdf::{NamedNode, Term as OTerm};
        let look = |iri: String| dict.lookup(&OTerm::NamedNode(NamedNode::new_unchecked(iri)));
        Vocab {
            ty: look(format!("{}type", RDF)),
            sub_class_of: look(format!("{}subClassOf", RDFS)),
            equivalent_class: look(format!("{}equivalentClass", OWL)),
            intersection_of: look(format!("{}intersectionOf", OWL)),
            on_property: look(format!("{}onProperty", OWL)),
            some_values_from: look(format!("{}someValuesFrom", OWL)),
            disjoint_with: look(format!("{}disjointWith", OWL)),
            restriction: look(format!("{}Restriction", OWL)),
            one_of: look(format!("{}oneOf", OWL)),
            has_value: look(format!("{}hasValue", OWL)),
            has_self: look(format!("{}hasSelf", OWL)),
            rdf_first: look(format!("{}first", RDF)),
            rdf_rest: look(format!("{}rest", RDF)),
            rdf_nil: look(format!("{}nil", RDF)),
            thing: look(format!("{}Thing", OWL)),
            nothing: look(format!("{}Nothing", OWL)),
            #[cfg(feature = "cdomain")]
            on_datatype: look(format!("{}onDatatype", OWL)),
            #[cfg(feature = "cdomain")]
            with_restrictions: look(format!("{}withRestrictions", OWL)),
            #[cfg(feature = "abox")]
            bottom_object_property: look(format!("{}bottomObjectProperty", OWL)),
            #[cfg(feature = "abox")]
            has_key: look(format!("{}hasKey", OWL)),
            #[cfg(feature = "abox")]
            source_individual: look(format!("{}sourceIndividual", OWL)),
            #[cfg(feature = "abox")]
            assertion_property: look(format!("{}assertionProperty", OWL)),
            #[cfg(feature = "abox")]
            target_individual: look(format!("{}targetIndividual", OWL)),
            #[cfg(feature = "abox")]
            target_value: look(format!("{}targetValue", OWL)),
            #[cfg(feature = "abox")]
            different_from: look(format!("{}differentFrom", OWL)),
            non_el: NON_EL_MARKERS
                .iter()
                .map(|m| look(format!("{}{}", OWL, m)))
                .collect(),
            #[cfg(feature = "rbox")]
            sub_property_of: look(format!("{}subPropertyOf", RDFS)),
            #[cfg(feature = "rbox")]
            property_chain_axiom: look(format!("{}propertyChainAxiom", OWL)),
            #[cfg(feature = "rbox")]
            transitive_property: look(format!("{}TransitiveProperty", OWL)),
            #[cfg(feature = "rbox")]
            top_object_property: look(format!("{}topObjectProperty", OWL)),
        }
    }
}

/// The structural index the extractor builds in one pass over the triples: enough to decode
/// restrictions and intersection lists into [`Expr`] trees without re-scanning.
#[derive(Default)]
struct Idx {
    sub_class: Vec<(Id, Id)>,      // (C, D) from rdfs:subClassOf
    equiv_class: Vec<(Id, Id)>,    // (C, D) from owl:equivalentClass
    disjoint: Vec<(Id, Id)>,       // (C, D) from owl:disjointWith
    on_prop: FxHashMap<Id, Id>,    // restriction node -> property
    svf: FxHashMap<Id, Id>,        // restriction node -> someValuesFrom filler
    inter_head: FxHashMap<Id, Id>, // class node -> intersectionOf list head
    first: FxHashMap<Id, Id>,      // rdf:first edges
    rest: FxHashMap<Id, Id>,       // rdf:rest edges
    is_restriction: FxHashSet<Id>, // nodes typed owl:Restriction
    non_el_node: FxHashSet<Id>,    // nodes carrying a non-EL marker predicate
    // [FABLE-5] sq-pbz04.2.1 — safe nominals (CR6):
    one_of_head: FxHashMap<Id, Id>, // class node -> owl:oneOf list head (resolved after the pass)
    one_of: FxHashMap<Id, Option<Id>>, // class node -> the SINGLETON individual (None = skip)
    has_value: FxHashMap<Id, Id>,   // restriction node -> owl:hasValue INDIVIDUAL (never a literal)
    // [OPUS-4.8] sq-pbz04.2.6 — self-restrictions (ObjectHasSelf, CR-Self): restriction nodes
    // carrying a VALID `owl:hasSelf "true"^^xsd:boolean`. A `false`/non-boolean/non-`true` object
    // never lands here — it is routed to `non_el_node` (a counted skip) during the pass. `decode`
    // turns a node in this set WITH `owl:onProperty` and no other filler into `∃r.Self`.
    self_true: FxHashSet<Id>,
    // [FABLE-5] sq-pbz04.2.2 — concrete domains (CR7–CR9), collected only under `cdomain`.
    // The RAW structure (filled during the pass; candidate nodes stay non_el-marked so an
    // UNSUPPORTED shape keeps the exact pre-cdomain skip):
    #[cfg(feature = "cdomain")]
    cd_on_datatype: FxHashMap<Id, Id>, // range node -> owl:onDatatype base datatype
    #[cfg(feature = "cdomain")]
    cd_with_restrictions: FxHashMap<Id, Id>, // range node -> owl:withRestrictions list head
    #[cfg(feature = "cdomain")]
    cd_has_value_lit: FxHashMap<Id, Id>, // restriction node -> LITERAL owl:hasValue object
    #[cfg(feature = "cdomain")]
    cd_one_of_lit: FxHashMap<Id, Id>, // enumeration node -> its SINGLETON literal member
    // [FABLE-5] soundness guard (PR #1434 adversarial-verify fix): nodes carrying a non-EL
    // marker OTHER than onDatatype/withRestrictions (unionOf, complementOf, allValuesFrom,
    // cardinality, onDataRange, datatypeComplementOf). Such a node must NEVER be
    // rescued as a concrete-domain range/point — decoding just its range half would DROP the
    // foreign structure and STRENGTHEN an LHS axiom. `resolve_cdomain` refuses these.
    // [OPUS-4.8] sq-pbz04.2.6: `self_true` nodes are likewise refused (a mixed hasSelf/range
    // node would drop the reflexivity half); the poison lives in `resolve_cdomain`'s `structure`.
    #[cfg(feature = "cdomain")]
    cd_foreign_marker: FxHashSet<Id>,
    // The RESOLVED supported nodes (filled by `resolve_cdomain` before axiom decoding):
    #[cfg(feature = "cdomain")]
    cd_range: FxHashMap<Id, Concept>, // supported range / DataOneOf node -> range concept
    #[cfg(feature = "cdomain")]
    cd_exists: FxHashMap<Id, Concept>, // supported DataHasValue restriction node -> point concept
    // [SONNET-4.6] sq-vkq9u (`abox` + `cdomain`): DataPropertyAssertion LITERAL -> its minted
    // POINT-range concept. Filled by `resolve_cdomain` from the literals `abox_data_literals`
    // pre-collected; read by `decode_abox` to internalize `a q 5` as `{a} ⊑ ∃q.{5}`. Empty
    // unless the caller asked for ABox internalization (`ExtractOpts::abox`).
    #[cfg(all(feature = "abox", feature = "cdomain"))]
    cd_abox_point: FxHashMap<Id, Concept>,
    #[cfg(feature = "rbox")]
    sub_property: Vec<(Id, Id)>, // (r, s) from rdfs:subPropertyOf — a role inclusion r ⊑ s
    #[cfg(feature = "rbox")]
    chain_head: FxHashMap<Id, Id>, // super-property -> owl:propertyChainAxiom list head
    #[cfg(feature = "rbox")]
    transitive: Vec<Id>, // properties typed owl:TransitiveProperty
}

/// Extracts and normalizes the EL+⊥ TBox from `triples`, returning the [`Extracted`] bundle
/// (normal-form concept axioms, the name table, the [`Report`], and — under `rbox` — the
/// normalized RBox role axioms). Single-threaded.
pub fn extract(dict: &Dict, triples: &[[Id; 3]], opts: ExtractOpts) -> Extracted {
    // `opts` only steers the `abox` internalization below; without that feature it is inert.
    #[cfg(not(feature = "abox"))]
    let _ = opts;
    let v = Vocab::intern(dict);
    #[cfg_attr(not(feature = "cdomain"), allow(unused_mut))]
    let mut idx = build_idx(dict, triples, &v);

    let mut names = Names::new();
    // [FABLE-5] sq-pbz04.2.2 (CR7–CR9): resolve the concrete-domain candidates BEFORE
    // decoding axioms, so `decode` can route supported faceted-range / point nodes to
    // their minted concepts. Returns the CR7/CR8 axioms appended after normalization.
    // [SONNET-4.6] sq-vkq9u (`abox` + `cdomain`): the ABox data-property-assertion literals must
    // be minted in the SAME `Mint` registry as the TBox ranges — that is what makes `{5}` from
    // `a q 5` and a TBox `[5, 5]` / `DataHasValue 5` ONE concept, so CR8 relates them — which
    // means collecting them BEFORE this pre-pass. Empty unless the caller asked for ABox
    // internalization, so `Classifier::classify` mints exactly what it minted before.
    #[cfg(feature = "cdomain")]
    let cd_axioms = {
        #[cfg(feature = "abox")]
        let abox_lits =
            if opts.abox { abox_data_literals(dict, triples, &idx, &v) } else { Vec::new() };
        #[cfg(not(feature = "abox"))]
        let abox_lits: Vec<Id> = Vec::new();
        resolve_cdomain(dict, triples, &mut idx, &v, &mut names, &abox_lits)
    };
    // Pre-seed ⊤/⊥ so the recognisers route owl:Thing/owl:Nothing dict ids to TOP/BOTTOM.
    let mut report = Report::default();
    let mut norm = Normalizer::new(&mut names);

    // Decode every subClassOf / equivalentClass / disjointWith axiom into Expr -> Expr and
    // hand each to the normalizer. `add_class_axiom` resolves both class nodes into an `Expr`,
    // returning false (so the caller bumps the skip count) when either is a non-EL construct.
    for &(a, b) in &idx.sub_class {
        if !add_class_axiom(a, b, false, &idx, &v, &mut norm) {
            report.skipped_axioms += 1;
        }
    }
    for &(a, b) in &idx.equiv_class {
        if !add_class_axiom(a, b, true, &idx, &v, &mut norm) {
            report.skipped_axioms += 1;
        }
    }
    // disjointWith(C, D)  ⇒  C ⊓ D ⊑ ⊥.
    for &(a, b) in &idx.disjoint {
        if !add_disjoint_axiom(a, b, &idx, &v, &mut norm) {
            report.skipped_axioms += 1;
        }
    }

    // [OPUS-4.8] sq-pbz04.2.5 (`abox`): internalize ABox assertions as safe-nominal axioms over
    // the SAME normalizer + name table, so nominal + role ids stay consistent with the TBox
    // concepts. `Classifier::classify` / `classify_graph` pass `abox = false`, so they never see
    // these axioms (byte-identical, whatever the feature). `bottom_op_axiom` is the
    // `owl:bottomObjectProperty` empty-role fact `∃⊥.⊤ ⊑ ⊥`, appended after `finish` only when
    // that property actually occurred as a role (so `New-Feature-BottomObjectProperty-001`-shaped
    // `{a} ⊑ ∃⊥.⊤` collapses to `{a} ⊑ ⊥`).
    #[cfg(feature = "abox")]
    let (bottom_op_axiom, abox_extras) = if opts.abox {
        report.skipped_assertions = decode_abox(dict, triples, &idx, &v, &mut norm);
        let bottom = norm
            .names
            .role_of(v.bottom_object_property)
            .map(|r| Normal::ExistsSub(r, TOP, BOTTOM));
        // [OPUS-4.8] sq-pbz04.2.8: E4 — extract `owl:hasKey` / negative property assertions /
        // asserted `owl:differentFrom`, minting every referenced concept/role/nominal into the SAME
        // name table BEFORE saturation so the readoff's S-set / R-link lookups are well-sized.
        let extras =
            decode_abox_e4(dict, triples, &idx, &v, norm.names, &mut report.skipped_assertions);
        (bottom, extras)
    } else {
        (None, AboxExtras::default())
    };

    let axioms = norm.finish();
    #[cfg(feature = "abox")]
    let axioms = {
        let mut axioms = axioms;
        if let Some(ax) = bottom_op_axiom {
            axioms.push(ax);
        }
        axioms
    };
    // [FABLE-5] sq-pbz04.2.2: append the concrete-domain axioms — `d ⊑ ⊥` for an EMPTY
    // value space (CR7; the clash then reaches classes with an `∃p.d` obligation via CR5)
    // and `d1 ⊑ d2` for every PROVEN value-space containment (CR8/CR9, threaded through
    // data-property existentials by the ordinary CR1/CR3/CR4 machinery).
    #[cfg(feature = "cdomain")]
    let axioms = {
        let mut axioms = axioms;
        axioms.extend(cd_axioms);
        axioms
    };

    // RBox normalization (E2): role inclusions + property chains + transitive roles into
    // [`RoleAxiom`] forms. Done here while `names` is still borrowable so role ids agree with
    // the existential links minted during concept decoding. No-op without the `rbox` feature.
    // [SONNET-4.6] sq-oj06v: the told-RBox regularity verdict lands in the report — CR10/CR11
    // stay sound + terminating on a non-regular (spec-illegal) RBox, but completeness is only
    // argued for regular ones, so the flag is the honest "may be incomplete" surface.
    #[cfg(feature = "rbox")]
    let role_axioms = {
        let (role_axioms, regular) = normalize_rbox(&idx, &v, &mut names);
        report.rbox_non_regular = !regular;
        role_axioms
    };

    report.named_classes = names.concept_count();
    Extracted {
        axioms,
        names,
        report,
        #[cfg(feature = "rbox")]
        role_axioms,
        #[cfg(feature = "abox")]
        abox_extras,
    }
}

/// [SONNET-4.6] sq-clsv6: the ONE structural pass over `triples` — the axiom lists plus the
/// restriction / list / nominal / self / (under `cdomain`) faceted-range edges every later
/// decode step reads. Factored out of [`extract`] VERBATIM (same predicate dispatch, same
/// `owl:oneOf` resolution tail) so the incremental delta path (`extract_added`) can build the
/// same full-graph index without duplicating the recognition rules — any drift between the two
/// would be a fragment-recognition bug.
fn build_idx(dict: &Dict, triples: &[[Id; 3]], v: &Vocab) -> Idx {
    let mut idx = Idx::default();
    for &[s, p, o] in triples {
        if p == v.sub_class_of {
            idx.sub_class.push((s, o));
        } else if p == v.equivalent_class {
            idx.equiv_class.push((s, o));
        } else if p == v.disjoint_with {
            idx.disjoint.push((s, o));
        } else if p == v.on_property {
            idx.on_prop.insert(s, o);
        } else if p == v.some_values_from {
            idx.svf.insert(s, o);
        } else if p == v.intersection_of {
            idx.inter_head.insert(s, o);
        } else if p == v.rdf_first {
            idx.first.insert(s, o);
        } else if p == v.rdf_rest {
            idx.rest.insert(s, o);
        } else if p == v.ty && o == v.restriction {
            idx.is_restriction.insert(s);
        } else if p == v.one_of {
            // [FABLE-5] sq-pbz04.2.1: an enumeration node. The list is resolved to a singleton
            // nominal (or a skip) after this pass, once every rdf:first/rdf:rest edge is known.
            idx.one_of_head.insert(s, o);
        } else if p == v.has_value {
            // [FABLE-5] sq-pbz04.2.1: an object-valued hasValue is the nominal restriction
            // ∃r.{a} (CR6). A LITERAL value is DataHasValue — a concrete-domain restriction
            // (deferred CR7–CR9 surface, sibling bead sq-pbz04.2.2/.3) — so the node stays
            // non-EL and the enclosing axiom is counted as a skip, exactly as before.
            if is_individual(dict, o) {
                idx.has_value.insert(s, o);
            } else {
                idx.non_el_node.insert(s);
                // [FABLE-5] sq-pbz04.2.2: under `cdomain` a SUPPORTED exact-numeric
                // literal value (DataHasValue = ∃p.{v}, a point range) is rescued by
                // `resolve_cdomain`/`decode`; anything else keeps the skip above.
                #[cfg(feature = "cdomain")]
                idx.cd_has_value_lit.insert(s, o);
            }
        } else if p == v.has_self {
            // [OPUS-4.8] sq-pbz04.2.6: ObjectHasSelf. ONLY `"true"^^xsd:boolean` denotes the
            // self-restriction ∃r.Self; any other object (`false`, a non-boolean literal, an
            // IRI/blank) is a malformed/unsupported `owl:hasSelf` and the enclosing axiom stays a
            // COUNTED skip (fail-closed — never guessed). `decode` further requires `owl:onProperty`
            // and no other filler on the node before minting the self-concept.
            if is_boolean_true(dict, o) {
                idx.self_true.insert(s);
            } else {
                idx.non_el_node.insert(s);
            }
        } else if v.non_el.contains(&p) {
            idx.non_el_node.insert(s);
            // [FABLE-5] sq-pbz04.2.2: under `cdomain` ALSO record the faceted-datatype
            // structure; nodes resolving to a SUPPORTED range are rescued in `decode`
            // (unsupported ones keep the exact pre-cdomain skip path above).
            #[cfg(feature = "cdomain")]
            if p == v.on_datatype {
                idx.cd_on_datatype.insert(s, o);
            } else if p == v.with_restrictions {
                idx.cd_with_restrictions.insert(s, o);
            } else {
                // [FABLE-5] soundness (PR #1434 adversarial-verify fix): ANY other non-EL
                // marker (unionOf / complementOf / allValuesFrom / cardinality / hasSelf /
                // onDataRange / datatypeComplementOf) POISONS the node as a concrete-domain
                // candidate — rescuing its range half would silently drop this structure.
                idx.cd_foreign_marker.insert(s);
            }
        } else {
            #[cfg(feature = "rbox")]
            extract_rbox_triple(&mut idx, v, s, p, o);
        }
    }

    // [FABLE-5] sq-pbz04.2.1: resolve each owl:oneOf list. The OWL 2 EL profile's ObjectOneOf
    // admits EXACTLY ONE individual: a singleton list of a non-literal member becomes the
    // nominal {a}; an empty/multi-member list (a disjunction — outside EL) or a literal member
    // (DataOneOf — a concrete-domain range, deferred CR7–CR9) resolves to None so the enclosing
    // axiom is recorded as a skip, never misapplied.
    let resolved: Vec<(Id, Option<Id>, Option<Id>)> = idx
        .one_of_head
        .iter()
        .map(|(&node, &head)| {
            let members = decode_list(head, &idx, v);
            // [FABLE-5] sq-pbz04.2.2: a singleton NON-individual member (third slot) is
            // DataOneOf — a concrete-domain point range, resolvable under `cdomain` when
            // it is a supported exact-numeric literal; it stays a skip otherwise.
            let (single, lit) = match members[..] {
                [m] if is_individual(dict, m) => (Some(m), None),
                [m] => (None, Some(m)),
                _ => (None, None),
            };
            (node, single, lit)
        })
        .collect();
    for (node, single, _lit) in resolved {
        idx.one_of.insert(node, single);
        #[cfg(feature = "cdomain")]
        if let Some(l) = _lit {
            idx.cd_one_of_lit.insert(node, l);
        }
    }

    idx
}

/// [SONNET-4.6] sq-clsv6: decodes ONE top-level class axiom `a ⊑ b` (or, when `equiv`, the two
/// inclusions of `a ≡ b`) into normal form through `norm`. Returns `false` when either side is
/// outside the recognised fragment, so the caller counts the skip — shared by [`extract`] and
/// (under `incremental`) [`extract_added`] so the full and delta paths cannot drift on WHICH
/// axioms are recognised.
fn add_class_axiom(a: Id, b: Id, equiv: bool, idx: &Idx, v: &Vocab, norm: &mut Normalizer) -> bool {
    let mut cache = FxHashMap::default();
    let lhs = decode(a, idx, v, norm.names, &mut cache, 0);
    let rhs = decode(b, idx, v, norm.names, &mut cache, 0);
    match (lhs, rhs) {
        (Some(l), Some(r)) => {
            norm.add_sub(&l, &r);
            if equiv {
                norm.add_sub(&r, &l);
            }
            true
        }
        _ => false,
    }
}

/// [SONNET-4.6] sq-clsv6: `owl:disjointWith(C, D)` ⇒ `C ⊓ D ⊑ ⊥`. Returns `false` (a counted
/// skip) when either side is outside the recognised fragment. Companion of [`add_class_axiom`].
fn add_disjoint_axiom(a: Id, b: Id, idx: &Idx, v: &Vocab, norm: &mut Normalizer) -> bool {
    let mut cache = FxHashMap::default();
    let lhs = decode(a, idx, v, norm.names, &mut cache, 0);
    let rhs = decode(b, idx, v, norm.names, &mut cache, 0);
    match (lhs, rhs) {
        (Some(l), Some(r)) => {
            norm.add_sub(&Expr::And(vec![l, r]), &Expr::Atom(BOTTOM));
            true
        }
        _ => false,
    }
}

/// [SONNET-4.6] sq-clsv6 (Phase E5, `incremental`): the class axioms carried by an ADDED triple
/// batch, normalized into the CALLER'S existing [`Names`] so concept ids stay stable across the
/// edit (the whole basis of resuming a saturation instead of rebuilding it).
#[cfg(feature = "incremental")]
pub(crate) struct AddedAxioms {
    /// The normal-form axioms the added triples contribute.
    pub axioms: Vec<Normal>,
    /// Added class axioms outside the recognised fragment — folded into the running
    /// `Report::skipped_axioms`, which then equals a from-scratch extraction's count because every
    /// class-axiom triple is decoded exactly once over the graph's lifetime.
    pub skipped: usize,
}

/// [SONNET-4.6] sq-clsv6 (Phase E5, `incremental`): extracts and normalizes ONLY the class axioms
/// carried by `added`, minting into the EXISTING `names`.
///
/// `all_triples` is the FULL post-edit graph and is used to build the structural index, so a
/// restriction / intersection / enumeration node is decoded with the same neighbourhood a
/// from-scratch [`extract`] would see. `added` supplies the top-level axiom triples to decode —
/// each must be genuinely new (the caller de-duplicates against the pre-edit graph), otherwise its
/// axiom would be contributed twice.
///
/// PRECONDITION (the caller — `crate::incremental` — enforces it and otherwise falls back to a
/// full re-classification): the edit only adds class axioms and brand-new class-expression nodes,
/// touches no RBox vocabulary, and the graph carries no concrete-domain vocabulary. That is what
/// makes it sound to skip `resolve_cdomain` / `normalize_rbox` here — neither has anything left to
/// resolve, so `idx.cd_range` / `idx.cd_exists` being empty costs no recognition.
#[cfg(feature = "incremental")]
pub(crate) fn extract_added(
    dict: &Dict,
    all_triples: &[[Id; 3]],
    added: &[[Id; 3]],
    names: &mut Names,
) -> AddedAxioms {
    let v = Vocab::intern(dict);
    let idx = build_idx(dict, all_triples, &v);
    let mut skipped = 0usize;
    let mut norm = Normalizer::new(names);
    for &[s, p, o] in added {
        let ok = if p == v.sub_class_of {
            add_class_axiom(s, o, false, &idx, &v, &mut norm)
        } else if p == v.equivalent_class {
            add_class_axiom(s, o, true, &idx, &v, &mut norm)
        } else if p == v.disjoint_with {
            add_disjoint_axiom(s, o, &idx, &v, &mut norm)
        } else {
            continue; // a structural triple: it carries no axiom of its own.
        };
        if !ok {
            skipped += 1;
        }
    }
    AddedAxioms {
        axioms: norm.finish(),
        skipped,
    }
}

/// [SONNET-4.6] sq-clsv6 (Phase E5, `incremental`): why an added-triple batch is NOT safe to fold
/// into an existing saturation. Returned by [`addition_is_incrementally_safe`]; the classifier
/// surfaces it as the honest reason a full re-classification was run instead.
#[cfg(feature = "incremental")]
pub(crate) enum AdditionBlocker {
    /// A triple attaches structure to a node the graph ALREADY mentions, so it can CHANGE what an
    /// existing axiom means rather than only adding axioms (a second `owl:someValuesFrom` on a live
    /// restriction; an `owl:unionOf` that turns an in-fragment axiom into a skip; the first
    /// structure on a node an existing axiom currently reads as an opaque class atom). The retained
    /// closure could then hold a subsumption the post-edit TBox no longer entails — non-monotone.
    ExistingNode,
    /// A triple carries vocabulary whose effect is not delta-local: RBox role axioms (they change
    /// the role automaton every EXISTING link is closed under) or concrete-domain facets (resolved
    /// in a whole-graph pre-pass that mints datatype concepts).
    Vocabulary,
}

/// [SONNET-4.6] sq-clsv6 (Phase E5, `incremental`): decides whether `added` is a MONOTONE
/// EXTENSION of the graph whose terms are `pre_mentioned` (every id occurring in ANY position) —
/// i.e. whether folding it into an existing saturation via `crate::classify::resaturate` yields
/// exactly what a from-scratch classification of the post-edit graph would.
///
/// Two shapes are safe, and ONLY these two:
///
/// 1. **A top-level class axiom** (`rdfs:subClassOf` / `owl:equivalentClass` /
///    `owl:disjointWith`). Adding one never changes how any OTHER axiom decodes — [`decode`] walks
///    a class node's OUTGOING structure and an axiom triple adds none — so it can only ADD
///    normal-form axioms. Safe whatever its subject is (a named class or a live restriction node).
/// 2. **A class-expression edge on a BRAND-NEW node** (`owl:onProperty`, `owl:someValuesFrom`,
///    `owl:hasValue`, `owl:hasSelf`, `owl:intersectionOf`, `owl:oneOf`, `rdf:first`, `rdf:rest`,
///    `rdf:type owl:Restriction`, or a non-EL marker that merely makes the node a counted skip).
///    Because the subject was not mentioned ANYWHERE before, no existing axiom's decode can reach
///    it, so again the effect is purely additive. "Not a subject" would NOT be enough: a node that
///    so far appears only as an OBJECT (`:A rdfs:subClassOf _:b`) decodes as an opaque class atom
///    until it gains structure, and gaining it CHANGES that existing axiom rather than adding one.
///
/// Everything else is rejected — conservatively, so a `Vocabulary` / `ExistingNode` verdict on a
/// technically-harmless triple costs a full re-classification, never correctness. Under `cdomain`
/// the literal-valued `owl:hasValue` / `rdf:first` forms (`DataHasValue` / `DataOneOf` points) are
/// rejected too: they are rescued by the whole-graph `resolve_cdomain` pre-pass the delta path
/// does not run.
#[cfg(feature = "incremental")]
pub(crate) fn addition_is_incrementally_safe(
    dict: &Dict,
    pre_mentioned: &FxHashSet<Id>,
    added: &[[Id; 3]],
) -> Result<(), AdditionBlocker> {
    let v = Vocab::intern(dict);
    for &[s, p, o] in added {
        if is_term(p, v.sub_class_of) || is_term(p, v.equivalent_class) || is_term(p, v.disjoint_with)
        {
            continue; // (1) a top-level class axiom — always additive.
        }
        if pre_mentioned.contains(&s) {
            return Err(AdditionBlocker::ExistingNode);
        }
        // (2) a class-expression edge on a brand-new node. `owl:hasValue` / `rdf:first` carry an
        // object-vs-literal distinction that decides the CR6-nominal / concrete-domain split, so
        // under `cdomain` only the OBJECT form (which needs no whole-graph resolution) is safe.
        let structural = is_term(p, v.on_property)
            || is_term(p, v.some_values_from)
            || is_term(p, v.has_self)
            || is_term(p, v.intersection_of)
            || is_term(p, v.one_of)
            || is_term(p, v.rdf_rest)
            || (is_term(p, v.ty) && is_term(o, v.restriction))
            || is_delta_safe_marker(&v, p)
            || ((is_term(p, v.has_value) || is_term(p, v.rdf_first))
                && is_delta_safe_value(dict, o));
        if !structural {
            return Err(AdditionBlocker::Vocabulary);
        }
    }
    Ok(())
}

/// Whether `id` IS the vocabulary term `term`. A term ABSENT from the dict interns to
/// [`sparq_core::dict::NO_ID`], so a bare `id == term` would wave EVERY unresolvable id through as
/// that term — `Vocab::intern` leaves absent terms at `NO_ID` precisely so they match nothing during
/// extraction, and the fast-path whitelist must keep that property (a whitelist that accidentally
/// matched would admit a non-delta-local triple as safe, which is the one direction that costs
/// correctness rather than a wasted rebuild).
#[cfg(feature = "incremental")]
fn is_term(id: Id, term: Id) -> bool {
    id == term && term != sparq_core::dict::NO_ID
}

/// Whether `p` is a non-EL marker whose ONLY effect is to make its node a counted skip, in EVERY
/// feature state. Under `cdomain` the two faceted-range predicates are excluded: they feed the
/// whole-graph `resolve_cdomain` pre-pass, so a delta carrying them is not delta-local.
#[cfg(feature = "incremental")]
fn is_delta_safe_marker(v: &Vocab, p: Id) -> bool {
    // `Vocab::intern` leaves an ABSENT marker at `NO_ID`, so `non_el` can hold `NO_ID` — see
    // [`is_term`] for why matching it would be the one unsafe direction.
    if p == sparq_core::dict::NO_ID || !v.non_el.contains(&p) {
        return false;
    }
    #[cfg(feature = "cdomain")]
    if is_term(p, v.on_datatype) || is_term(p, v.with_restrictions) {
        return false;
    }
    true
}

/// Whether an `owl:hasValue` / `rdf:first` object is delta-local. Without `cdomain` every object
/// is (a literal simply makes the node a counted skip); with it, a LITERAL is a `DataHasValue` /
/// `DataOneOf` point resolved by the whole-graph concrete-domain pre-pass, so only individuals
/// (IRIs / blank nodes) stay delta-local.
#[cfg(feature = "incremental")]
fn is_delta_safe_value(dict: &Dict, o: Id) -> bool {
    #[cfg(feature = "cdomain")]
    {
        is_individual(dict, o)
    }
    #[cfg(not(feature = "cdomain"))]
    {
        let _ = (dict, o);
        true
    }
}

/// [SONNET-4.6] sq-clsv6 (Phase E5, `incremental` + `cdomain`): whether `triples` carry ANY
/// concrete-domain vocabulary — a faceted range (`owl:onDatatype` / `owl:withRestrictions`) or a
/// literal-valued `owl:hasValue` / `rdf:first` (a `DataHasValue` / `DataOneOf` point). Those nodes
/// are minted by the whole-graph `resolve_cdomain` pre-pass, whose node-to-concept map the delta
/// path cannot reconstruct, so their presence disables the incremental fast path outright.
/// Deliberately a conservative SUPERSET of what `resolve_cdomain` actually rescues.
#[cfg(all(feature = "incremental", feature = "cdomain"))]
pub(crate) fn has_concrete_domain_vocab(dict: &Dict, triples: &[[Id; 3]]) -> bool {
    let v = Vocab::intern(dict);
    triples.iter().any(|&[_, p, o]| {
        is_term(p, v.on_datatype)
            || is_term(p, v.with_restrictions)
            || ((is_term(p, v.has_value) || is_term(p, v.rdf_first)) && !is_individual(dict, o))
    })
}

/// [FABLE-5] sq-pbz04.2.2 (CR7–CR9): pre-screens the concrete-domain candidates and hands
/// the CLEAN sets to [`crate::cdomain::resolve`], storing the node → concept maps back into
/// `idx` for [`decode`] and returning the CR7/CR8 axioms.
///
/// STRICTNESS GUARD (soundness): a candidate node carrying ANY other class-expression
/// structure — a restriction part, an intersection, an object enumeration, a mixed
/// range/enumeration/hasValue shape, or ([FABLE-5] PR #1434 adversarial-verify fix) any
/// OTHER non-EL marker (`owl:unionOf`, `owl:complementOf`, `owl:allValuesFrom`,
/// cardinality, `owl:onDataRange`, `owl:datatypeComplementOf`, tracked in `cd_foreign_marker`;
/// or `owl:hasSelf`, tracked in `self_true`) — is REFUSED here and falls back to the ordinary
/// non-EL skip.
/// Decoding just its range half would DROP that structure and STRENGTHEN the asserted
/// axiom in a subclass (LHS) position, which is unsound.
///
/// [SONNET-4.6] sq-vkq9u: `abox_lits` carries the `DataPropertyAssertion` literals to rescue as
/// point ranges (empty unless `ExtractOpts::abox`). They go through the SAME mint registry, so a
/// value already minted by the TBox shares its concept; an out-of-tier literal is simply absent
/// from `idx.cd_abox_point` and `decode_abox` keeps its counted skip.
#[cfg(feature = "cdomain")]
fn resolve_cdomain(
    dict: &Dict,
    triples: &[[Id; 3]],
    idx: &mut Idx,
    v: &Vocab,
    names: &mut Names,
    abox_lits: &[Id],
) -> Vec<Normal> {
    let structure = |n: Id| {
        idx.on_prop.contains_key(&n)
            || idx.svf.contains_key(&n)
            || idx.has_value.contains_key(&n)
            || idx.inter_head.contains_key(&n)
            || idx.cd_foreign_marker.contains(&n)
            // [OPUS-4.8] sq-pbz04.2.6: a hasSelf-true node is a self-restriction, not a data
            // range — rescuing its range half would drop the reflexivity, so it is refused here.
            || idx.self_true.contains(&n)
    };
    // Faceted ranges: need BOTH onDatatype and withRestrictions, and nothing else.
    let mut ranges: Vec<(Id, Id, Id)> = idx
        .cd_on_datatype
        .iter()
        .filter(|&(&n, _)| {
            !structure(n)
                && !idx.one_of_head.contains_key(&n)
                && !idx.cd_has_value_lit.contains_key(&n)
        })
        .filter_map(|(&n, &dt)| idx.cd_with_restrictions.get(&n).map(|&h| (n, dt, h)))
        .collect();
    ranges.sort_unstable(); // deterministic mint order (concept ids per run)
    // Singleton-literal enumerations (DataOneOf): a PURE enumeration node only.
    let mut points: Vec<(Id, Id)> = idx
        .cd_one_of_lit
        .iter()
        .filter(|&(&n, _)| {
            !structure(n)
                && !idx.cd_on_datatype.contains_key(&n)
                && !idx.cd_with_restrictions.contains_key(&n)
                && !idx.cd_has_value_lit.contains_key(&n)
        })
        .map(|(&n, &l)| (n, l))
        .collect();
    points.sort_unstable();
    // Literal-hasValue restrictions (DataHasValue): need onProperty and nothing else.
    let mut exists_points: Vec<(Id, Id)> = idx
        .cd_has_value_lit
        .iter()
        .filter(|&(&n, _)| {
            idx.on_prop.contains_key(&n)
                && !idx.svf.contains_key(&n)
                && !idx.has_value.contains_key(&n)
                && !idx.inter_head.contains_key(&n)
                && !idx.one_of_head.contains_key(&n)
                && !idx.cd_on_datatype.contains_key(&n)
                && !idx.cd_with_restrictions.contains_key(&n)
                && !idx.cd_foreign_marker.contains(&n) // [FABLE-5] PR #1434 soundness fix
                && !idx.self_true.contains(&n) // [OPUS-4.8] sq-pbz04.2.6: not a self-restriction
        })
        .map(|(&n, &l)| (n, l))
        .collect();
    exists_points.sort_unstable();
    let out = crate::cdomain::resolve(
        dict,
        triples,
        names,
        &ranges,
        &points,
        &exists_points,
        abox_lits,
        |h| decode_list(h, idx, v),
    );
    idx.cd_range = out.node_range;
    idx.cd_exists = out.node_exists;
    #[cfg(feature = "abox")]
    {
        idx.cd_abox_point = out.lit_point;
    }
    out.axioms
}

/// Routes one RBox triple into the structural index: `rdfs:subPropertyOf` (role inclusion),
/// `owl:propertyChainAxiom` (the super-property is the SUBJECT, the chain list is the object),
/// or an `owl:TransitiveProperty` type assertion. Bare datatype-property `subPropertyOf`s are
/// captured too but never fire — they cannot appear in an `∃r.C` link, so a spurious inclusion
/// among data properties is inert (kept simple rather than requiring property-type declarations).
#[cfg(feature = "rbox")]
fn extract_rbox_triple(idx: &mut Idx, v: &Vocab, s: Id, p: Id, o: Id) {
    if p == v.sub_property_of {
        idx.sub_property.push((s, o));
    } else if p == v.property_chain_axiom {
        idx.chain_head.insert(s, o);
    } else if p == v.ty && o == v.transitive_property {
        idx.transitive.push(s);
    }
}

/// Builds the normalized [`RoleAxiom`] list from the RBox structural index, minting role ids
/// through the shared [`Names`] table. A property chain of length `n` is left-folded into `n-1`
/// binary compositions over fresh intermediate roles; a transitive property `r` becomes the
/// composition `r ∘ r ⊑ r`. A degenerate chain (length 0/1) is treated as a plain inclusion.
///
/// [SONNET-4.6] sq-oj06v: also returns the TOLD-RBox regularity verdict (`true` = regular),
/// computed by [`crate::rbox::told_rbox_regular`] over the PRE-binarization axioms — the told
/// n-ary chains, not the left-folded binary forms (binarization introduces apparent cycles a
/// told left-identity chain does not have). The caller surfaces `!regular` as
/// [`crate::Report::rbox_non_regular`].
#[cfg(feature = "rbox")]
fn normalize_rbox(idx: &Idx, v: &Vocab, names: &mut Names) -> (Vec<RoleAxiom>, bool) {
    let mut out: Vec<RoleAxiom> = Vec::new();
    let mut told_incl: Vec<(Role, Role)> = Vec::new();
    let mut told_chains: Vec<(Vec<Role>, Role)> = Vec::new();
    // r ⊑ s.
    for &(r, s) in &idx.sub_property {
        let (ri, si) = (names.role(r), names.role(s));
        out.push(RoleAxiom::Sub(ri, si));
        told_incl.push((ri, si));
    }
    // owl:propertyChainAxiom: r1 ∘ … ∘ rn ⊑ super.
    for (&super_prop, &head) in &idx.chain_head {
        let members = decode_list(head, idx, v);
        let chain: Vec<Role> = members.iter().map(|&m| names.role(m)).collect();
        let sup = names.role(super_prop);
        match chain[..] {
            [] => {}
            [r1] => told_incl.push((r1, sup)),
            _ => told_chains.push((chain.clone(), sup)),
        }
        fold_chain(&chain, sup, names, &mut out);
    }
    // owl:TransitiveProperty(r) ≡ r ∘ r ⊑ r.
    for &r in &idx.transitive {
        let ri = names.role(r);
        out.push(RoleAxiom::Chain(ri, ri, ri));
        told_chains.push((vec![ri, ri], ri));
    }
    // `owl:topObjectProperty`'s identity survives normalization (it is minted like any role),
    // so the regularity check can apply the restriction's unconditional top-superproperty
    // exemption. `role_of` is `None` when top never occurs as a role — then no chain can name
    // it as superproperty and the exemption is vacuous.
    let top = names.role_of(v.top_object_property);
    let regular = crate::rbox::told_rbox_regular(&told_incl, &told_chains, top);
    (out, regular)
}

/// Left-folds a property chain `r1 ∘ … ∘ rn ⊑ sup` into binary [`RoleAxiom::Chain`]s:
/// `r1 ∘ r2 ⊑ f1`, `f1 ∘ r3 ⊑ f2`, …, `f_{n-2} ∘ rn ⊑ sup`. Length-1 chains degenerate to an
/// inclusion `r1 ⊑ sup`; a length-0 chain (malformed) is dropped.
#[cfg(feature = "rbox")]
fn fold_chain(chain: &[Role], sup: Role, names: &mut Names, out: &mut Vec<RoleAxiom>) {
    match chain {
        [] => {}
        [r1] => out.push(RoleAxiom::Sub(*r1, sup)),
        [r1, r2] => out.push(RoleAxiom::Chain(*r1, *r2, sup)),
        [first, rest @ ..] => {
            let mut acc = *first;
            // rest has len ≥ 2 here; compose all but the last over fresh roles, last lands on sup.
            for &next in &rest[..rest.len() - 1] {
                let f = names.fresh_role();
                out.push(RoleAxiom::Chain(acc, next, f));
                acc = f;
            }
            let last = rest[rest.len() - 1];
            out.push(RoleAxiom::Chain(acc, last, sup));
        }
    }
}

/// Resolves a class node (named class, ⊤, ⊥, an `owl:Restriction` node, an
/// `owl:intersectionOf` node, or — sq-pbz04.2.1 — a singleton `owl:oneOf` nominal) into an
/// [`Expr`]. Returns `None` for any node that uses a construct outside the recognised
/// fragment (so the caller can record a skip). `depth` guards against cyclic blank-node
/// structures (a malformed list pointing at itself).
fn decode(
    node: Id,
    idx: &Idx,
    v: &Vocab,
    names: &mut Names,
    cache: &mut FxHashMap<Id, Option<Expr>>,
    depth: u32,
) -> Option<Expr> {
    if depth > 256 {
        return None; // pathological nesting / cycle: treat as non-EL.
    }
    if node == v.thing {
        return Some(Expr::Atom(TOP));
    }
    if node == v.nothing {
        return Some(Expr::Atom(BOTTOM));
    }
    if let Some(hit) = cache.get(&node) {
        return hit.clone();
    }

    // [FABLE-5] sq-pbz04.2.2 (CR7–CR9): a node RESOLVED as a SUPPORTED concrete-domain
    // range (faceted onDatatype/withRestrictions or singleton-literal oneOf = DataOneOf)
    // decodes to its range concept, and a resolved literal-hasValue restriction
    // (DataHasValue) to ∃p.{point}. This runs BEFORE the non-EL check below because the
    // candidates stay non_el-marked: an UNSUPPORTED concrete-domain node is absent from
    // these maps and falls through to the skip — exactly the pre-cdomain behaviour.
    #[cfg(feature = "cdomain")]
    {
        if let Some(&c) = idx.cd_range.get(&node) {
            let result = Some(Expr::Atom(c));
            cache.insert(node, result.clone());
            return result;
        }
        if let (Some(&c), Some(&p)) = (idx.cd_exists.get(&node), idx.on_prop.get(&node)) {
            let role = names.role(p);
            let result = Some(Expr::Exists(role, Box::new(Expr::Atom(c))));
            cache.insert(node, result.clone());
            return result;
        }
    }

    // A node carrying a non-EL class-expression marker (unionOf / complementOf / cardinality /
    // allValuesFrom / a literal-valued hasValue / …) is outside the recognised fragment:
    // report it as a skip, do NOT mistake it for an opaque named class (that would silently
    // drop its real semantics).
    if idx.non_el_node.contains(&node) {
        cache.insert(node, None);
        return None;
    }

    // [OPUS-4.8] sq-pbz04.2.6: a self-restriction node ∃r.Self (`ObjectHasSelf`). A VALID node
    // carries `owl:hasSelf "true"^^xsd:boolean` (recorded in `self_true`), `owl:onProperty r`,
    // and NOTHING else structural. Fail-closed: a missing `owl:onProperty`, or a co-occurring
    // someValuesFrom / hasValue / intersectionOf on the SAME node, yields None so the enclosing
    // axiom is skipped (never a half-decoded, structure-dropping guess). A co-occurring non-EL /
    // concrete-domain marker already routed the node to `non_el_node` (handled above), so a mixed
    // hasSelf/range node never reaches here.
    if idx.self_true.contains(&node) {
        let clean = !idx.svf.contains_key(&node)
            && !idx.has_value.contains_key(&node)
            && !idx.inter_head.contains_key(&node);
        let result = match idx.on_prop.get(&node) {
            Some(&p) if clean => {
                let role = names.role(p);
                Some(Expr::Atom(names.self_concept(role)))
            }
            _ => None,
        };
        cache.insert(node, result.clone());
        return result;
    }

    // [FABLE-5] sq-pbz04.2.1: an owl:oneOf enumeration node. A resolved SINGLETON individual
    // is the nominal {a} — a basic concept in the EL++ normal form (CR6). An unresolvable
    // enumeration (empty / multi-member / literal member) is None → the axiom is skipped.
    if let Some(&single) = idx.one_of.get(&node) {
        let result = single.map(|ind| Expr::Atom(names.nominal(ind)));
        cache.insert(node, result.clone());
        return result;
    }

    // A restriction node: must carry onProperty + exactly one of someValuesFrom (∃r.C) or an
    // object-valued hasValue (∃r.{a}, CR6) for the recognised fragment.
    if idx.is_restriction.contains(&node)
        || idx.svf.contains_key(&node)
        || idx.on_prop.contains_key(&node)
        || idx.has_value.contains_key(&node)
    {
        let result = match (
            idx.on_prop.get(&node),
            idx.svf.get(&node),
            idx.has_value.get(&node),
        ) {
            (Some(&p), Some(&filler), None) => {
                let role = names.role(p);
                decode(filler, idx, v, names, cache, depth + 1)
                    .map(|f| Expr::Exists(role, Box::new(f)))
            }
            // [FABLE-5] sq-pbz04.2.1: ObjectHasValue(r, a) ≡ ∃r.{a} — the nominal-filler
            // restriction. `idx.has_value` only ever holds INDIVIDUAL values (a literal value
            // marked the node non-EL during extraction), so minting the nominal is safe here.
            (Some(&p), None, Some(&value)) => {
                let role = names.role(p);
                Some(Expr::Exists(role, Box::new(Expr::Atom(names.nominal(value)))))
            }
            // A restriction node with onProperty but a body outside the fragment
            // (allValuesFrom, cardinality, …), with BOTH someValuesFrom and hasValue
            // (malformed), or with a body and no property — skipped, never guessed.
            _ => None,
        };
        cache.insert(node, result.clone());
        return result;
    }

    // An intersection node: decode the RDF list, recursively decoding each member.
    if let Some(&head) = idx.inter_head.get(&node) {
        let members_ids = decode_list(head, idx, v);
        let mut parts = Vec::with_capacity(members_ids.len());
        for m in members_ids {
            match decode(m, idx, v, names, cache, depth + 1) {
                Some(e) => parts.push(e),
                None => {
                    cache.insert(node, None);
                    return None;
                }
            }
        }
        let result = if parts.len() >= 2 {
            Some(Expr::And(parts))
        } else if parts.len() == 1 {
            parts.into_iter().next()
        } else {
            None
        };
        cache.insert(node, result.clone());
        return result;
    }

    // Otherwise it is a plain named class.
    let c = names.class(node);
    let result = Some(Expr::Atom(c));
    cache.insert(node, result.clone());
    result
}

/// [FABLE-5] sq-pbz04.2.1: whether a dict id denotes an INDIVIDUAL (an IRI or a blank node —
/// blank nodes are OWL's anonymous individuals). Literals (including the dict's inline-integer
/// ids, which `term_parts` must not be asked about) and RDF 1.2 triple terms are NOT
/// individuals: a literal in `owl:hasValue`/`owl:oneOf` position is the concrete-domain form
/// (`DataHasValue`/`DataOneOf`, the deferred CR7–CR9 surface), so those axioms stay skipped.
fn is_individual(dict: &Dict, id: Id) -> bool {
    !sparq_core::dict::is_inline(id)
        && matches!(
            dict.term_parts(id),
            sparq_core::dict::TermParts::Iri { .. } | sparq_core::dict::TermParts::Blank(_)
        )
}

/// [OPUS-4.8] sq-pbz04.2.6: whether a dict id is the boolean-true literal `owl:hasSelf` requires
/// (`"true"^^xsd:boolean`, or its equivalent lexical form `"1"^^xsd:boolean` — both denote the
/// XSD boolean value *true*). An inline id is always an `xsd:integer`, never a boolean, so it is
/// rejected outright. Everything else — `false`/`0`, a non-boolean literal, an IRI/blank — is
/// NOT true, so the enclosing `owl:hasSelf` axiom stays a counted skip (fail-closed).
fn is_boolean_true(dict: &Dict, id: Id) -> bool {
    if sparq_core::dict::is_inline(id) {
        return false;
    }
    matches!(
        dict.term_parts(id),
        sparq_core::dict::TermParts::Lit { value, datatype, lang: None }
            if datatype == XSD_BOOLEAN && (value == "true" || value == "1")
    )
}

/// [OPUS-4.8] sq-pbz04.2.5 (`abox`): the RDF/RDFS/OWL/XSD "structural" IRI namespaces. A
/// predicate in one of these is NEVER an ABox object/data-property assertion — it is TBox/RBox
/// vocabulary (`rdfs:subClassOf`, `owl:someValuesFrom`, `rdf:first`, …) or an annotation/facet.
/// The recognisers below skip it (fail-closed): an in-namespace predicate is not misread as an
/// object-property assertion, and a facet/annotation literal is not miscounted as a data skip.
#[cfg(feature = "abox")]
const STRUCTURAL_NS: &[&str] = &[OWL, RDF, RDFS, "http://www.w3.org/2001/XMLSchema#"];

/// OWL/RDF/RDFS built-in classes that, as the OBJECT of an `rdf:type` triple, denote a
/// DECLARATION (`X a owl:Class`, `p a owl:ObjectProperty`, …) rather than a Direct-Semantics
/// `ClassAssertion`. Excluded so the extractor mints no spurious individual for a
/// class/property/ontology/axiom node. `owl:Thing` / `owl:Nothing` are DELIBERATELY absent:
/// `a rdf:type owl:Thing` is a valid (trivial) assertion and `a rdf:type owl:Nothing` a valid
/// (inconsistency-forcing) one. `owl:NamedIndividual` is handled separately (register, no axiom).
#[cfg(feature = "abox")]
const META_TYPE_OBJECTS: &[(&str, &str)] = &[
    (OWL, "Class"),
    (OWL, "Restriction"),
    (OWL, "ObjectProperty"),
    (OWL, "DatatypeProperty"),
    (OWL, "AnnotationProperty"),
    (OWL, "OntologyProperty"),
    (OWL, "Ontology"),
    (OWL, "AllDifferent"),
    (OWL, "AllDisjointClasses"),
    (OWL, "AllDisjointProperties"),
    (OWL, "NegativePropertyAssertion"),
    (OWL, "FunctionalProperty"),
    (OWL, "InverseFunctionalProperty"),
    (OWL, "TransitiveProperty"),
    (OWL, "SymmetricProperty"),
    (OWL, "AsymmetricProperty"),
    (OWL, "ReflexiveProperty"),
    (OWL, "IrreflexiveProperty"),
    (OWL, "DeprecatedClass"),
    (OWL, "DeprecatedProperty"),
    (OWL, "DataRange"),
    (OWL, "Axiom"),
    (RDFS, "Class"),
    (RDFS, "Datatype"),
    (RDFS, "ContainerMembershipProperty"),
    (RDF, "Property"),
    (RDF, "List"),
    (RDF, "Statement"),
];

/// Whether `id` is an IRI in one of the [`STRUCTURAL_NS`] namespaces (or is not a usable
/// property IRI at all: an inline literal / blank / literal predicate). Used to keep the ABox
/// property-assertion recogniser off every TBox/RBox/annotation/facet predicate.
#[cfg(feature = "abox")]
fn is_structural_predicate(dict: &Dict, id: Id) -> bool {
    if sparq_core::dict::is_inline(id) {
        return true;
    }
    match dict.term_parts(id) {
        sparq_core::dict::TermParts::Iri { prefix, suffix } => {
            let iri = format!("{}{}", prefix, suffix);
            STRUCTURAL_NS.iter().any(|ns| iri.starts_with(ns))
        }
        _ => true,
    }
}

/// Whether a dict id denotes a LITERAL (an inline integer or a stored `Lit`) — the object shape
/// of a `DataPropertyAssertion`. The complement of the individual test for object position.
#[cfg(feature = "abox")]
fn is_literal(dict: &Dict, id: Id) -> bool {
    sparq_core::dict::is_inline(id)
        || matches!(dict.term_parts(id), sparq_core::dict::TermParts::Lit { .. })
}

/// [OPUS-4.8] sq-pbz04.2.5 (`abox`): internalize the ABox assertions of `triples` into `norm` as
/// SAFE-NOMINAL axioms over the CR6 machinery, minting every individual through `Names::nominal`
/// so `owl:hasValue`/`owl:oneOf` fillers and asserted individuals share one concept:
///
/// - `a rdf:type C`  (C a decodable EL class, not a built-in meta class) ⇒ `{a} ⊑ C`;
/// - `a p b`         (p a user-namespace object property, b an individual) ⇒ `{a} ⊑ ∃p.{b}`;
/// - `a rdf:type owl:NamedIndividual` ⇒ register `a` (no axiom — a declaration).
///
/// [SONNET-4.6] sq-vkq9u (`abox` + `cdomain`): additionally
/// - `a q v` (v a LITERAL in the exact numeric tier) ⇒ `{a} ⊑ ∃q.{v}`, with `{v}` the point range
///   `resolve_cdomain` minted for it. SOUNDNESS: `a q v` asserts `(a^I, v) ∈ q^I` and
///   `v ∈ {v}^D`, so `a^I ∈ (∃q.{v})^I`. Without `cdomain` — or for a literal outside the exact
///   tier — there is no point concept, so the assertion keeps the fail-closed skip below.
///
/// Returns the count of assertions DEFERRED as counted skips (`Report::skipped_assertions`): a
/// `DataPropertyAssertion` whose literal has no minted point range, and a `ClassAssertion` whose
/// class expression is outside the EL fragment.
/// SOUNDNESS: `{a} ⊑ C` holds because `{a}^I = {a^I} ⊆ C^I`; `{a} ⊑ ∃p.{b}` because
/// `(a^I,b^I) ∈ p^I` with `b^I ∈ {b}^I`. Structural bnodes (restriction / intersection / list
/// nodes) are never minted as individuals — realising one would be noise, not an entailment.
#[cfg(feature = "abox")]
fn decode_abox(dict: &Dict, triples: &[[Id; 3]], idx: &Idx, v: &Vocab, norm: &mut Normalizer) -> usize {
    use oxrdf::{NamedNode, Term as OTerm};
    let look = |iri: String| dict.lookup(&OTerm::NamedNode(NamedNode::new_unchecked(iri)));
    let named_individual = look(format!("{}NamedIndividual", OWL));
    let meta: FxHashSet<Id> = META_TYPE_OBJECTS
        .iter()
        .map(|(ns, name)| look(format!("{}{}", ns, name)))
        .filter(|&id| id != sparq_core::dict::NO_ID)
        .collect();

    let structural = |n: Id| is_class_expression_node(idx, n);

    let mut skipped = 0usize;
    for &[s, p, o] in triples {
        if p == v.ty {
            if o == named_individual && named_individual != sparq_core::dict::NO_ID {
                if is_individual(dict, s) && !structural(s) {
                    let _ = norm.names.nominal(s); // register the declared individual only
                }
                continue;
            }
            if meta.contains(&o) || !is_individual(dict, s) || structural(s) {
                continue; // a declaration, or a structural/non-individual subject
            }
            // ClassAssertion: {s} ⊑ decode(o). A non-EL class expression is a fail-closed skip.
            let mut cache = FxHashMap::default();
            match decode(o, idx, v, norm.names, &mut cache, 0) {
                Some(cls) => {
                    let a = norm.names.nominal(s);
                    norm.add_sub(&Expr::Atom(a), &cls);
                }
                None => skipped += 1,
            }
        } else if !is_structural_predicate(dict, p) && is_individual(dict, s) && !structural(s) {
            if is_individual(dict, o) && !structural(o) {
                // ObjectPropertyAssertion: {s} ⊑ ∃p.{o}.
                let role = norm.names.role(p);
                let a = norm.names.nominal(s);
                let b = norm.names.nominal(o);
                norm.add_sub(&Expr::Atom(a), &Expr::Exists(role, Box::new(Expr::Atom(b))));
            } else if is_literal(dict, o) {
                // DataPropertyAssertion. [SONNET-4.6] sq-vkq9u: under `cdomain` an exact-numeric
                // literal was minted as the point range `{o}` by the `resolve_cdomain` pre-pass,
                // so the assertion internalizes as `{s} ⊑ ∃p.{o}` and CR8 threads the asserted
                // VALUE into the TBox's data-range obligations. Everything else — no `cdomain`, a
                // string / lang-tagged / float-tier / ill-formed literal — keeps the fail-closed
                // counted skip: no value space is ever guessed.
                match abox_point(idx, o) {
                    Some(point) => {
                        let role = norm.names.role(p);
                        let a = norm.names.nominal(s);
                        norm.add_sub(
                            &Expr::Atom(a),
                            &Expr::Exists(role, Box::new(Expr::Atom(point))),
                        );
                    }
                    None => skipped += 1,
                }
            } else {
                // ObjectPropertyAssertion whose OBJECT is a structural blank node (a
                // restriction / intersection / list cell / non-EL node) — not a plain
                // individual. [OPUS-4.8] sq-pbz04.2.5: still sound and fail-closed (we
                // never guess a typing), but the skip was previously untallied; now counted
                // so `Report::skipped_assertions` gives an honest "n assertions not
                // internalized" total.
                skipped += 1;
            }
        }
    }
    skipped
}

/// [OPUS-4.8] sq-pbz04.2.5 (`abox`): whether the TBox pass uses `n` as a CLASS EXPRESSION — a
/// restriction / intersection / enumeration node, an RDF list cell, a non-EL-marked node, or
/// ([OPUS-4.8] sq-pbz04.2.6) a self-restriction. Such a node must never be minted as an ABox
/// individual: realising one would be noise, not an entailment.
///
/// [SONNET-4.6] sq-vkq9u: shared by [`decode_abox`] and [`abox_data_literals`] so the point-range
/// pre-pass mints exactly the literals `decode_abox` will go on to internalize — a divergence
/// would either mint an unused concept or leave a rescuable assertion counted as a skip.
#[cfg(feature = "abox")]
fn is_class_expression_node(idx: &Idx, n: Id) -> bool {
    idx.is_restriction.contains(&n)
        || idx.inter_head.contains_key(&n)
        || idx.one_of_head.contains_key(&n)
        || idx.first.contains_key(&n)
        || idx.non_el_node.contains(&n)
        || idx.self_true.contains(&n)
}

/// [SONNET-4.6] sq-vkq9u (`abox` + `cdomain`): the LITERALS of the graph's `DataPropertyAssertion`s
/// — the objects [`decode_abox`] will reach through its data branch — deduplicated and sorted so
/// the mint order (and therefore every minted concept id) depends only on the graph, not on triple
/// order. The recogniser is deliberately IDENTICAL to `decode_abox`'s data branch: a non-`rdf:type`
/// non-structural predicate, an individual non-class-expression subject, a literal object.
/// Over-collecting would mint a concept for a literal that is never internalized (harmless but
/// wasteful); under-collecting would silently keep a rescuable assertion as a skip.
#[cfg(all(feature = "abox", feature = "cdomain"))]
fn abox_data_literals(dict: &Dict, triples: &[[Id; 3]], idx: &Idx, v: &Vocab) -> Vec<Id> {
    let mut lits: Vec<Id> = triples
        .iter()
        .filter(|&&[s, p, o]| {
            p != v.ty
                && !is_structural_predicate(dict, p)
                && is_individual(dict, s)
                && !is_class_expression_node(idx, s)
                && is_literal(dict, o)
        })
        .map(|&[_, _, o]| o)
        .collect();
    lits.sort_unstable();
    lits.dedup();
    lits
}

/// [SONNET-4.6] sq-vkq9u: the minted POINT-range concept of a `DataPropertyAssertion` literal
/// (`{5}` for `a q 5`), or `None` — which keeps [`decode_abox`]'s fail-closed counted skip.
/// Always `None` without `cdomain`: there is no concrete-domain value tower to mint a point on,
/// so the pre-sq-vkq9u skip behaviour is byte-identical in that feature state.
#[cfg(all(feature = "abox", feature = "cdomain"))]
fn abox_point(idx: &Idx, lit: Id) -> Option<Concept> {
    idx.cd_abox_point.get(&lit).copied()
}

/// The no-`cdomain` counterpart of [`abox_point`]: no point range exists, so every
/// `DataPropertyAssertion` stays a counted skip.
#[cfg(all(feature = "abox", not(feature = "cdomain")))]
fn abox_point(_idx: &Idx, _lit: Id) -> Option<Concept> {
    None
}

/// [OPUS-4.8] sq-pbz04.2.8 (`abox`): whether a dict id is a NAMED (IRI) individual/property. Keys
/// apply only to NAMED individuals (OWL 2 Direct Semantics), and blank-node key VALUES are treated
/// conservatively (not matched), so the E4 extractor filters on this.
#[cfg(feature = "abox")]
fn is_named_iri(dict: &Dict, id: Id) -> bool {
    !sparq_core::dict::is_inline(id)
        && matches!(dict.term_parts(id), sparq_core::dict::TermParts::Iri { .. })
}

/// [OPUS-4.8] sq-pbz04.2.8 (`abox`): extract the E4 ABox axioms — `owl:hasKey` keys, negative
/// property assertions, and asserted `owl:differentFrom` inequalities — minting every referenced
/// class concept / property role / individual nominal into `names` so the post-saturation readoff
/// (see `abox.rs`) can look them up in the saturated S-sets and R-links.
///
/// FAIL-CLOSED: a malformed shape is discarded and counted in `*skipped` — never a guessed
/// derivation. Specifically a key whose class is not a decodable ATOMIC concept, whose list is
/// empty, or whose list carries a non-IRI member; and an NPA missing its source / property / target
/// or carrying a non-individual source / structural property / non-individual object.
#[cfg(feature = "abox")]
fn decode_abox_e4(
    dict: &Dict,
    triples: &[[Id; 3]],
    idx: &Idx,
    v: &Vocab,
    names: &mut Names,
    skipped: &mut usize,
) -> AboxExtras {
    let mut extras = AboxExtras::default();

    // One pass: collect key heads (class -> hasKey list head), NPA reification parts (keyed by the
    // NPA node), and asserted differentFrom pairs.
    let mut key_heads: Vec<(Id, Id)> = Vec::new();
    let mut npa_source: FxHashMap<Id, Id> = FxHashMap::default();
    let mut npa_prop: FxHashMap<Id, Id> = FxHashMap::default();
    let mut npa_target_ind: FxHashMap<Id, Id> = FxHashMap::default();
    let mut npa_target_val: FxHashMap<Id, Id> = FxHashMap::default();
    // NPA nodes in first-seen order (deduplicated) for a deterministic scan.
    let mut npa_nodes: Vec<Id> = Vec::new();
    let mut seen_npa: FxHashSet<Id> = FxHashSet::default();
    for &[s, p, o] in triples {
        let is_npa_part = p == v.source_individual
            || p == v.assertion_property
            || p == v.target_individual
            || p == v.target_value;
        if p == v.has_key {
            key_heads.push((s, o));
        } else if is_npa_part {
            if seen_npa.insert(s) {
                npa_nodes.push(s);
            }
            if p == v.source_individual {
                npa_source.insert(s, o);
            } else if p == v.assertion_property {
                npa_prop.insert(s, o);
            } else if p == v.target_individual {
                npa_target_ind.insert(s, o);
            } else {
                npa_target_val.insert(s, o);
            }
        } else if p == v.different_from && is_individual(dict, s) && is_individual(dict, o) {
            extras.different_from.push((s, o));
        }
    }

    // Keys: decode the class to an ATOMIC concept and the list to IRI properties.
    for (class_node, head) in key_heads {
        let mut cache = FxHashMap::default();
        let class = match decode(class_node, idx, v, names, &mut cache, 0) {
            Some(Expr::Atom(c)) => c,
            // A complex/undecodable key class (intersection, restriction, non-EL) is deferred.
            _ => {
                *skipped += 1;
                continue;
            }
        };
        let members = decode_list(head, idx, v);
        if members.is_empty() || members.iter().any(|&m| !is_named_iri(dict, m)) {
            *skipped += 1;
            continue;
        }
        let props: Vec<(Role, Id)> = members.iter().map(|&m| (names.role(m), m)).collect();
        extras.keys.push(AboxKey { class, props });
    }

    // [OPUS-4.8] sq-pbz04.2.8: mint every NAMED subject of a key-property assertion as a nominal, so
    // the readoff's key analysis sees individuals that appear ONLY in a (skipped) DATA-property
    // assertion (e.g. `Peter hasSSN "…"`). Object-key subjects are already minted by `decode_abox`
    // (the OPA path); this is idempotent for them. Only runs when a well-formed key exists.
    if !extras.keys.is_empty() {
        let key_props: FxHashSet<Id> = extras
            .keys
            .iter()
            .flat_map(|k| k.props.iter().map(|&(_, p)| p))
            .collect();
        for &[s, p, _] in triples {
            if key_props.contains(&p) && is_named_iri(dict, s) {
                let _ = names.nominal(s);
            }
        }
    }

    // Negative property assertions.
    for node in npa_nodes {
        let (Some(&src), Some(&prop)) = (npa_source.get(&node), npa_prop.get(&node)) else {
            *skipped += 1; // an NPA node missing its source or property
            continue;
        };
        if let Some(&tind) = npa_target_ind.get(&node) {
            // Negative OBJECT property assertion.
            if is_individual(dict, src)
                && is_individual(dict, tind)
                && !is_structural_predicate(dict, prop)
            {
                let role = names.role(prop);
                let source_c = names.nominal(src);
                let target_c = names.nominal(tind);
                extras.npas.push(AboxNpa::Object {
                    source_dict: src,
                    prop_dict: prop,
                    target_dict: tind,
                    source_c,
                    role,
                    target_c,
                });
            } else {
                *skipped += 1;
            }
        } else if let Some(&tval) = npa_target_val.get(&node) {
            // Negative DATA property assertion.
            if is_individual(dict, src) && is_literal(dict, tval) {
                extras.npas.push(AboxNpa::Data {
                    source_dict: src,
                    prop_dict: prop,
                    value_dict: tval,
                });
            } else {
                *skipped += 1;
            }
        } else {
            *skipped += 1; // an NPA node with no target
        }
    }

    extras
}

/// Walks an `rdf:first`/`rdf:rest` list from `head`, returning member node ids. Bounded by
/// the number of list cells seen (guards a malformed self-referential `rdf:rest`).
fn decode_list(head: Id, idx: &Idx, v: &Vocab) -> Vec<Id> {
    let mut out = Vec::new();
    let mut cur = head;
    let mut seen = FxHashSet::default();
    while cur != v.rdf_nil && seen.insert(cur) {
        match idx.first.get(&cur) {
            Some(&m) => out.push(m),
            None => break,
        }
        match idx.rest.get(&cur) {
            Some(&n) => cur = n,
            None => break,
        }
    }
    out
}
