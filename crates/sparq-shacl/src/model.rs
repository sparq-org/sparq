//! The shapes model: parsing a shapes graph into [`Shape`] structs with their
//! targets, paths and constraint components.

use crate::path::Path;
use crate::view::{GraphView, RDF_TYPE};
use oxrdf::Term;
use rustc_hash::FxHashMap;
use sparq_core::Graph;

pub const SH: &str = "http://www.w3.org/ns/shacl#";
const RDFS_CLASS: &str = "http://www.w3.org/2000/01/rdf-schema#Class";
/// [FABLE-5] (sq-c1v3e) The datatype the SHACL syntax rules give every
/// count/length parameter (`sh:minCount`, `sh:maxLength`, …).
const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";

pub(crate) fn sh(local: &str) -> String {
    format!("{SH}{local}")
}

/// How a shape selects its focus nodes.
#[derive(Debug, Clone)]
pub enum Target {
    /// sh:targetNode — the node itself (need not occur in the data graph).
    Node(Term),
    /// sh:targetClass — all SHACL instances of the class in the data graph.
    Class(Term),
    /// Implicit class target: the shape is itself an rdfs:Class (or, SHACL 1.2,
    /// an `sh:ShapeClass` — a class that is also a node shape).
    ImplicitClass(Term),
    /// sh:targetSubjectsOf — all subjects of the predicate.
    SubjectsOf(String),
    /// sh:targetObjectsOf — all objects of the predicate.
    ObjectsOf(String),
    /// [OPUS-4.8] (sq-rnkdh) SHACL 1.2 `sh:targetWhere [ <inline shape> ]`: the
    /// focus nodes are every data-graph node that CONFORMS to the inline (object)
    /// shape. The `usize` is the inline shape's id in [`ShapesModel::shapes`].
    Where(usize),
    /// [OPUS-4.8] (sq-rnkdh) SHACL 1.2 SPARQL-valued target
    /// `sh:targetNode [ sh:select … ]` / `[ sh:sparqlExpr … ]`: the focus nodes are
    /// the output nodes of the SPARQL node expression (the bindings of its first
    /// result variable). The index is into the model's `select_exprs` table.
    Sparql(usize),
}

/// One occurrence of a constraint component on a shape.
#[derive(Debug, Clone)]
pub enum Component {
    Class(Term),
    /// [OPUS-4.8] (sq-sx15d) `sh:class` with a SHACL-list object
    /// (`sh:class ( ex:A ex:B )`, SHACL 1.2): a value node conforms iff it is a
    /// SHACL instance of ANY listed class (`sh:ClassConstraintComponent`).
    /// Mirrors the disjunctive `sh:datatype` / `sh:nodeKind` list spelling; a
    /// single IRI object stays [`Component::Class`].
    ClassIn(Vec<Term>),
    /// `sh:datatype` — the allowed-datatype set (SHACL §4.5.2). A single IRI
    /// object is a singleton set; the SHACL-1.2 disjunctive list form
    /// `sh:datatype ( xsd:string rdf:langString )` is the multi-element set. A
    /// value node conforms iff it is a literal whose (well-formed) datatype is in
    /// the set. [OPUS-4.8] (sq-vg3y) extended from a single IRI to the set form.
    Datatype(Vec<String>),
    /// `sh:nodeKind` — the allowed node-kind set (SHACL §4.6.1). A single IRI is a
    /// singleton set; the SHACL-1.2 disjunctive list form
    /// `sh:nodeKind ( sh:BlankNode sh:IRI )` is the multi-element set. A value
    /// node conforms iff its kind matches ANY listed kind. [OPUS-4.8] (sq-vg3y).
    NodeKind(Vec<String>),
    MinCount(u64),
    MaxCount(u64),
    MinExclusive(Term),
    MinInclusive(Term),
    MaxExclusive(Term),
    MaxInclusive(Term),
    MinLength(u64),
    MaxLength(u64),
    Pattern {
        source: String,
        flags: Option<String>,
    },
    LanguageIn(Vec<String>),
    UniqueLang,
    /// [OPUS-4.8] (sq-sx15d) `sh:equals` — the value set of the shape's path must
    /// equal the value set of the comparand. SHACL 1.2 lets the comparand be a
    /// full property [`Path`] (often an RDF-list sequence), not just a predicate
    /// IRI; a bare IRI parses to a trivial [`Path::Predicate`], so the SHACL 1.0
    /// `-001` predicate form stays backward-compatible.
    Equals(Path),
    /// [OPUS-4.8] (sq-sx15d) `sh:disjoint` — the value set of the shape's path
    /// must be disjoint from the comparand's. The comparand is a [`Path`] (SHACL
    /// 1.2 list/path form; a bare IRI is a trivial path).
    Disjoint(Path),
    /// [OPUS-4.8] (sq-sx15d) `sh:lessThan` — every path value must be `<` every
    /// comparand value. The comparand is a [`Path`] (SHACL 1.2).
    LessThan(Path),
    /// [OPUS-4.8] (sq-sx15d) `sh:lessThanOrEquals` — every path value must be
    /// `<=` every comparand value. The comparand is a [`Path`] (SHACL 1.2).
    LessThanOrEquals(Path),
    /// [OPUS-4.8] (sq-sx15d) `sh:subsetOf` (SHACL 1.2) — the value set of the
    /// shape's path must be a SUBSET of the comparand path's value set
    /// (`sh:SubsetOfConstraintComponent`). One result per path value absent from
    /// the comparand set. The comparand is a [`Path`].
    SubsetOf(Path),
    /// [OPUS-4.8] (sq-sx15d) `sh:someValue` (SHACL 1.2) — EXISTENTIAL: at least
    /// one value node must conform to the nested shape
    /// (`sh:SomeValueConstraintComponent`). The index is into
    /// [`ShapesModel::shapes`]. One result on the focus/path when NONE conform.
    SomeValue(usize),
    /// [OPUS-4.8] (sq-sx15d) `sh:singleLine true` (SHACL 1.2) — each string value
    /// must contain no line-break characters (LF/CR/FF/VT)
    /// (`sh:SingleLineConstraintComponent`). `sh:singleLine false` imposes no
    /// constraint (not parsed into a component).
    SingleLine,
    /// [OPUS-4.8] (sq-sx15d) `sh:rootClass` (SHACL 1.2) — each value node must be
    /// the named class or a transitive `rdfs:subClassOf`-descendant of it
    /// (`sh:RootClassConstraintComponent`). Reuses the `sh:class` subclass
    /// closure.
    RootClass(Term),
    Not(usize),
    And(Vec<usize>),
    Or(Vec<usize>),
    Xone(Vec<usize>),
    Node(usize),
    /// sh:property — a child property shape validated against the same focus.
    Property(usize),
    /// [OPUS-4.8] (sq-0mjfd) `sh:reifierShape` (SHACL 1.2 §4.x) — for each value
    /// node `v` of this PROPERTY shape's path `p`, the *reifiers* of the asserted
    /// triple `(focus, p, v)` (the subjects of `?r rdf:reifies <<(focus p v)>>`,
    /// the RDF-1.2 reified-annotation `{| … |}` form) must each conform to the
    /// referenced shape (`shape` indexes [`ShapesModel::shapes`]). With
    /// `required = true` (`sh:reificationRequired true`), a value that has NO
    /// reifier is ALSO a violation. One result (on focus/path, `sh:value` = the
    /// offending value) per value whose reifier set fails — component
    /// `sh:ReifierShapeConstraintComponent`. Only meaningful for a single-predicate
    /// path (the reified triple needs a predicate); a non-predicate path yields no
    /// reifiers and (unless required) conforms vacuously.
    ReifierShape { shape: usize, required: bool },
    Qualified {
        shape: usize,
        min: Option<u64>,
        max: Option<u64>,
        disjoint: bool,
        /// Sibling qualified value shapes (for sh:qualifiedValueShapesDisjoint).
        siblings: Vec<usize>,
    },
    Closed {
        ignored: Vec<Term>,
        /// [OPUS-4.8] (sq-vg3y) SHACL-1.2 "close by types" mode: `sh:closed
        /// sh:ByTypes`. When `false` (`sh:closed true`), the allowed predicate set
        /// P is the IRIs reachable from THIS shape via `sh:property/sh:path`. When
        /// `true`, P is recomputed PER value node from its `rdf:type`s via the
        /// `collectProperties` algorithm (SHACL §4.8.1), plus `rdf:type`.
        by_types: bool,
    },
    HasValue(Term),
    In(Vec<Term>),
    /// [OPUS-4.8] (sq-vg3y) `sh:memberShape` — SHACL-1.2 list-member shape
    /// (`sh:MemberShapeConstraintComponent`, SHACL §4.x). Each value node must be a
    /// well-formed SHACL list, and every member of that list must conform to the
    /// referenced shape. The index is into [`ShapesModel::shapes`].
    MemberShape(usize),
    /// [OPUS-4.8] (sq-vg3y) `sh:uniqueMembers true` — value nodes must be SHACL
    /// lists whose members are pairwise distinct (`sh:UniqueMembersConstraintComponent`).
    UniqueMembers,
    /// [OPUS-4.8] (sq-vg3y) `sh:maxListLength` — value nodes must be SHACL lists
    /// with at most N members (`sh:MaxListLengthConstraintComponent`).
    MaxListLength(u64),
    /// [OPUS-4.8] (sq-vg3y) `sh:minListLength` — value nodes must be SHACL lists
    /// with at least N members (`sh:MinListLengthConstraintComponent`).
    MinListLength(u64),
    /// [OPUS-4.8] (sq-vg3y) `sh:uniqueValuesFor` — the values of the listed
    /// properties of a value node must be unique across all target nodes of the
    /// shape (`sh:UniqueValuesForConstraintComponent`, SHACL §4.x). One or more
    /// property IRIs (a single IRI is a singleton; a SHACL list gives a composite
    /// key).
    UniqueValuesFor(Vec<String>),
    /// sh:sparql — a SPARQL-based constraint (SHACL §5.2). The index is into
    /// `ShapesModel::sparql`.
    Sparql(usize),
    /// [OPUS-4.8] A SPARQL-based constraint COMPONENT (SHACL §6) that activated on
    /// this shape because the shape uses the component's parameter predicates.
    /// `component` indexes `ShapesModel::components`; `args` are the bound
    /// parameter values, parallel to the component's `parameters` (one term each
    /// — the first object found for a mandatory parameter; `None` for an absent
    /// optional one). The validator pre-binds each as `$paramName`.
    CustomSparql {
        component: usize,
        args: Vec<Option<Term>>,
        /// [OPUS-4.8] Index into the model's `path_validators` store of a
        /// per-shape validator with the shape's property path substituted for `$PATH`
        /// query variable (SHACL §6.3), present only when the shape is a PROPERTY
        /// shape, the chosen validator references `$PATH`, and the substituted
        /// query re-parses. When set it OVERRIDES the component's shared
        /// (path-free) validator for this occurrence; otherwise the shared
        /// validator is used as-is (node shapes, or `$PATH`-free validators). An
        /// index (not the value) keeps the crate-private validator type off this
        /// public enum.
        path_validator: Option<usize>,
    },
    /// [OPUS-4.8] (sq-mk9n, `shacl-af`) `sh:expression` — the SHACL-AF
    /// *Expression Constraint* (`sh:ExpressionConstraintComponent`): a value node
    /// `v` is violated when the node expression does NOT evaluate to `{ true }`
    /// for `v` as focus. The index is into `ShapesModel::expressions` (the
    /// parsed node expression is held there, off the public `Component` enum).
    #[cfg(feature = "shacl-af")]
    Expression(usize),
    /// [OPUS-4.8] (sq-3w6n, `shacl-af`) `sh:nodeByExpression` — the SHACL-AF
    /// *Node-by-Expression Constraint* (`sh:NodeByExpressionConstraintComponent`):
    /// for each value node `v`, the node expression is evaluated against `v` as
    /// focus to a set of node-shape terms `s`; `v` is violated when it does NOT
    /// conform to some `s`. (Like `sh:node`, but the shape is computed by the
    /// expression rather than fixed.) The index is into
    /// `ShapesModel::expressions` (shared store with `sh:expression`).
    #[cfg(feature = "shacl-af")]
    NodeByExpression(usize),
}

/// [OPUS-4.8] (sq-pb0wm) Per-constraint-statement RDF-1.2 reified-annotation
/// overrides for ONE [`Component`] occurrence (SHACL 1.2 Core, the
/// `misc/{deactivated-003,message-002,severity-003}` entries). When a constraint
/// triple `(shapeNode, P, O)` carries a `{| … |}` annotation — parsed as
/// `_:r rdf:reifies <<( shapeNode P O )>> . _:r sh:deactivated|sh:message|sh:severity V`
/// — the annotation overrides JUST that occurrence: `deactivated` suppresses only
/// that constraint (not the whole shape), and `messages` / `severity` replace the
/// shape-level message / severity for ONLY that constraint's results. Held in a
/// vector PARALLEL to [`Shape::components`] (a [`Self::default`] entry — no
/// override — for every component built from something other than a single
/// reifiable constraint triple). Distinct from the shape-level
/// [`Shape::deactivated`] / [`Shape::messages`] / [`Shape::severity`].
#[derive(Debug, Clone, Default)]
pub(crate) struct ComponentMeta {
    /// `{| sh:deactivated true |}` on this constraint statement: suppress this
    /// occurrence only (the rest of the shape still validates).
    pub deactivated: bool,
    /// `{| sh:message "…" |}` on this statement: the result message(s) for this
    /// occurrence's violations, overriding the shape's `sh:message`. Empty ⇒ no
    /// per-statement message override (inherit the shape's).
    pub messages: Vec<Term>,
    /// `{| sh:severity sh:Warning |}` on this statement: the result severity for
    /// this occurrence's violations, overriding the shape's. `None` ⇒ inherit.
    pub severity: Option<String>,
}

impl ComponentMeta {
    /// Whether this carries any per-statement override (cheap "is the default"
    /// probe so eval can skip the override path for the common un-annotated case).
    pub fn is_empty(&self) -> bool {
        !self.deactivated && self.messages.is_empty() && self.severity.is_none()
    }

    /// Whether this constraint occurrence is per-statement deactivated
    /// (`{| sh:deactivated true |}`) — eval skips just this occurrence.
    pub fn is_deactivated(&self) -> bool {
        self.deactivated
    }
}

/// [OPUS-4.8] A declared `sh:parameter` of a SPARQL-based constraint component
/// (SHACL §6.2): its predicate (`sh:path`), the pre-bound variable name and
/// whether it is `sh:optional`.
#[derive(Debug, Clone)]
pub(crate) struct ComponentParameter {
    /// The parameter's predicate IRI (`sh:path` of the parameter) — a shape
    /// "uses" the parameter by carrying a triple with this predicate.
    pub predicate: String,
    /// The pre-bound variable name (`$name`): the parameter's `sh:name`, else the
    /// local name of its predicate IRI (SHACL §6.2.1).
    pub var: String,
    /// `sh:optional true` — the parameter need not be present for the component
    /// to activate (a mandatory parameter must be present).
    pub optional: bool,
}

impl PreparedComponentValidator {
    /// Whether the validator query text references the `$PATH` / `?PATH` query
    /// variable — i.e. it must be re-parsed per property shape with the shape's
    /// path substituted (SHACL §6.3). A cheap textual probe (the whole-token
    /// match is done by `substitute_path_var`); a false positive only costs a
    /// re-parse that produces the same query.
    pub fn references_path(&self) -> bool {
        self.raw.contains("PATH")
    }

    /// Re-parses this validator with `$PATH` / `?PATH` substituted by the SPARQL
    /// property-path expression `pp` (SHACL §6.3 pre-binds `$PATH` to the property
    /// shape's path). Returns `None` if the substituted query no longer parses
    /// (ill-formed → the component occurrence is then skipped, lenient).
    pub fn with_path(&self, pp: &str) -> Option<PreparedComponentValidator> {
        let substituted = substitute_path_var(&self.raw, pp);
        let prepared = crate::sparql::PreparedValidator::build(&substituted, self.is_ask)?;
        Some(PreparedComponentValidator {
            prepared,
            message: self.message.clone(),
            raw: substituted,
            is_ask: self.is_ask,
        })
    }
}

/// [OPUS-4.8] A SPARQL-based constraint component declaration (SHACL §6.2): its
/// parameters and a validator. The validator is chosen by shape kind at
/// evaluation time: `sh:nodeValidator` for node shapes, `sh:propertyValidator`
/// for property shapes, falling back to the generic `sh:validator`. Each carries
/// a `sh:ask` or `sh:select` query (compiled into [`PreparedValidator`]).
#[derive(Debug, Clone)]
pub(crate) struct ComponentDef {
    /// The component's node (for diagnostics / `sh:sourceConstraintComponent`).
    pub node: Term,
    pub parameters: Vec<ComponentParameter>,
    /// Generic validator (`sh:validator`) — used when no kind-specific one fits.
    pub validator: Option<PreparedComponentValidator>,
    /// `sh:nodeValidator` — preferred for node shapes.
    pub node_validator: Option<PreparedComponentValidator>,
    /// `sh:propertyValidator` — preferred for property shapes.
    pub property_validator: Option<PreparedComponentValidator>,
    /// [SONNET-4.6] (sq-ou3) The component's `sh:labelTemplate` literals (SHACL
    /// §6.1): human-readable renderings of the component *with its parameters
    /// substituted in*, e.g. `"Value must have at most {$maxLength} characters"`.
    /// A component may declare several (typically one per language tag), so this
    /// keeps every value and [`label_template`](Self::label_template) picks one
    /// deterministically.
    ///
    /// `sh:labelTemplate` takes NO part in validation — it never affects whether
    /// a constraint fires, which results are produced, or `sh:conforms`. It is
    /// used only to render a result's *fallback* message: when neither the shape
    /// (`sh:message`) nor the validator (`sh:message`) supplies one, the label —
    /// which describes the constraint in the author's own words — is a far better
    /// message than the generic "does not satisfy constraint component <iri>".
    pub label_templates: Vec<Term>,
}

impl ComponentDef {
    /// [SONNET-4.6] (sq-ou3) The `sh:labelTemplate` to render for this component,
    /// chosen DETERMINISTICALLY (report output must be reproducible) from the
    /// possibly-several declared values: a plain (language-tag-less) literal
    /// wins — it is the language-neutral form — otherwise the
    /// lexicographically-smallest language tag. Non-literal values are ignored
    /// (ill-formed, and this crate is lenient about ill-formed shapes).
    ///
    /// Returns the raw template; `{$param}` / `{?param}` placeholders are
    /// substituted by the caller against the shape's bound parameter values.
    pub fn label_template(&self) -> Option<&str> {
        let literals = self.label_templates.iter().filter_map(|t| match t {
            Term::Literal(l) => Some(l),
            _ => None,
        });
        // Pick by (has-language, language) so a plain literal sorts first, then
        // the smallest tag. `min_by_key` keeps the FIRST of equal keys, which for
        // duplicate plain literals is stable in the shapes graph's own order.
        literals
            .min_by_key(|l| (l.language().is_some(), l.language().unwrap_or("")))
            .map(|l| l.value())
    }

    /// The validator to run for a shape of the given kind: the kind-specific one
    /// if present, else the generic `sh:validator` (SHACL §6.2.2).
    pub fn validator_for(&self, is_property_shape: bool) -> Option<&PreparedComponentValidator> {
        let specific = if is_property_shape {
            self.property_validator.as_ref()
        } else {
            self.node_validator.as_ref()
        };
        specific.or(self.validator.as_ref())
    }
}

/// [OPUS-4.8] A compiled component validator: the parsed ASK/SELECT query plus
/// its own `sh:message` template (SHACL §6.3 uses the validator's `sh:message`
/// for the produced results).
#[derive(Debug, Clone)]
pub(crate) struct PreparedComponentValidator {
    pub prepared: crate::sparql::PreparedValidator,
    pub message: Option<String>,
    /// The validator's full query text (prefixes already prepended) and whether
    /// it is an `sh:ask`. Retained so a `sh:propertyValidator` can be RE-PARSED
    /// per property shape with the shape's path substituted for the `$PATH`
    /// query variable (SHACL §6.3 pre-binds `$PATH` to the shape's property path,
    /// which — being a property PATH, not a term — cannot go through the VALUES
    /// table the other pre-bindings use, so it is a textual substitution like the
    /// §5.2 `sh:sparql` path). `None` for blank text would never compile.
    pub raw: String,
    pub is_ask: bool,
}

/// A `sh:sparql` constraint's components (SHACL §5.2): the `sh:select` query,
/// its `sh:prefixes` declarations and an optional constraint-level `sh:message`.
/// The parsed/validated query is held in [`crate::sparql::PreparedSparql`]; a
/// `None` prepared form means the query was ill-formed (and the constraint is
/// skipped, matching this crate's lenient handling of ill-formed shapes).
#[derive(Debug, Clone)]
pub(crate) struct SparqlConstraint {
    /// [OPUS-4.8] (sq-mue75) The `sh:SPARQLConstraint` node (the object of
    /// `sh:sparql`), stamped onto each result as `sh:sourceConstraint` (SHACL
    /// §5.2.2).
    pub node: Term,
    /// The raw `sh:select` text.
    pub select: String,
    /// PREFIX declarations assembled from `sh:prefixes` (`sh:declare` →
    /// `sh:prefix` / `sh:namespace`), prepended to `select` before parsing.
    pub prefixes: String,
    /// The constraint's own `sh:message` template, if any (takes precedence over
    /// a `?message` binding and over the shape's `sh:message`).
    pub message: Option<String>,
    /// [OPUS-4.8] (sq-rnkdh) The constraint node's own `sh:severity` IRI, if any.
    /// SHACL 1.2 lets a `sh:SPARQLConstraint` declare a `sh:severity` that
    /// OVERRIDES the enclosing shape's severity for the results it produces
    /// (`sparql/node/sparql-001`: a `sh:Warning` constraint on an otherwise
    /// default-`Violation` shape). `None` ⇒ inherit the shape's severity.
    pub severity: Option<String>,
    /// `sh:deactivated true` on the constraint node.
    pub deactivated: bool,
    /// The parsed query; `None` if `select` did not parse as a SELECT — the
    /// constraint is then skipped.
    pub prepared: Option<crate::sparql::PreparedSparql>,
}

#[derive(Debug)]
pub struct Shape {
    /// The shape's node in the shapes graph.
    pub node: Term,
    /// `Some` for property shapes (subjects of sh:path).
    pub path: Option<Path>,
    pub targets: Vec<Target>,
    pub components: Vec<Component>,
    /// [OPUS-4.8] (sq-pb0wm) Per-constraint-statement RDF-1.2 reified-annotation
    /// overrides, PARALLEL to [`Self::components`] (index `i` describes
    /// `components[i]`). A `ComponentMeta::default` entry means "no annotation";
    /// only components built from a single reifiable constraint triple
    /// (`(shapeNode, P, O)`) can carry a real override. See [`ComponentMeta`].
    pub(crate) component_meta: Vec<ComponentMeta>,
    /// [SONNET-4.6] (sq-7os4t) Overrides on the qualified minimum/maximum count
    /// statements. These are parallel to `components`, but kept separate because
    /// one `Component::Qualified` can emit results caused by either statement.
    pub(crate) qualified_min_meta: Vec<ComponentMeta>,
    pub(crate) qualified_max_meta: Vec<ComponentMeta>,
    /// Severity IRI (default sh:Violation).
    pub severity: String,
    /// sh:message literals, copied into results.
    pub messages: Vec<Term>,
    pub deactivated: bool,
    /// Child property shapes (sh:property) — also feeds sh:closed.
    pub property_children: Vec<usize>,
    /// [OPUS-4.8] (sq-rnkdh) SHACL 1.2 `sh:values [ sh:select … ]` /
    /// `[ sh:sparqlExpr … ]`: when present, the value nodes of this (property)
    /// shape are COMPUTED by the SPARQL node expression (index into
    /// [`ShapesModel::select_exprs`]) instead of derived by traversing `sh:path`.
    /// The reported `sh:resultPath` is still the shape's `sh:path`.
    pub(crate) value_expr: Option<usize>,
}

/// All shapes parsed from a shapes graph, indexed densely (cycle-safe).
pub struct ShapesModel {
    pub shapes: Vec<Shape>,
    by_node: FxHashMap<Term, usize>,
    /// Shapes that have at least one target — the validation entry points.
    pub targeted: Vec<usize>,
    /// SPARQL-based constraints (`sh:sparql`), referenced by [`Component::Sparql`].
    pub(crate) sparql: Vec<SparqlConstraint>,
    /// [OPUS-4.8] (sq-rnkdh) SHACL 1.2 SPARQL-based node expressions
    /// (`sh:select` / `sh:sparqlExpr`) used by SPARQL-valued targets
    /// ([`Target::Sparql`]) and SPARQL-valued value nodes
    /// ([`Shape::value_expr`]). Off the public surface (the prepared type is
    /// crate-private), like `sparql` / `path_validators`.
    pub(crate) select_exprs: Vec<crate::sparql::PreparedSelectExpr>,
    /// [OPUS-4.8] SPARQL-based constraint COMPONENTS (`sh:ConstraintComponent`,
    /// SHACL §6) declared in the shapes graph, referenced by
    /// [`Component::CustomSparql`]. The registry is keyed (for activation) on the
    /// mandatory parameter predicates each component declares.
    pub(crate) components: Vec<ComponentDef>,
    /// [OPUS-4.8] (sq-wys) Per-shape `sh:propertyValidator`s re-parsed with the
    /// shape's path substituted for the `$PATH` query variable (SHACL §6.3),
    /// referenced by [`Component::CustomSparql`]'s `path_validator` index. Kept
    /// off the public `Component` enum so the (crate-private) prepared-validator
    /// type is not exposed (same pattern as `sparql` / `expressions`).
    pub(crate) path_validators: Vec<PreparedComponentValidator>,
    /// [OPUS-4.8] (sq-vg3y) Precomputed `sh:closed sh:ByTypes` property closures
    /// (SHACL-1.2 §4.8.1 `collectProperties`): for each shapes-graph node, the
    /// IRI properties reachable via `sh:property/sh:path` transitively through
    /// `rdfs:subClassOf` / inbound `sh:targetClass` / `sh:node`. This
    /// shapes-graph traversal is data-independent, so it is resolved ONCE here
    /// (only when at least one ByTypes-closed shape exists) and unioned per value
    /// node at eval time. Empty when no shape uses `sh:closed sh:ByTypes`.
    pub(crate) by_types_closures: FxHashMap<Term, Vec<String>>,
    /// [OPUS-4.8] (sq-mk9n, `shacl-af`) Parsed `sh:expression` node expressions,
    /// referenced by [`Component::Expression`]. Kept off the public `Component`
    /// enum so the (crate-private) node-expression type is not exposed.
    #[cfg(feature = "shacl-af")]
    pub(crate) expressions: Vec<crate::rules::NodeExpr>,
    /// [OPUS-4.8] (sq-0mjfd) SHACL-SPARQL pre-binding FAILURES discovered at
    /// parse: a `sh:sparql` constraint (or a SPARQL-based constraint-component
    /// validator) whose query violates the pre-binding rules (MINUS / VALUES /
    /// SERVICE / a sub-SELECT that drops a pre-bound variable / a BIND re-binding
    /// one). A conformant processor MUST signal a *failure* for these (W3C SHACL
    /// §3.4), surfaced through [`crate::validate_strict`]. Empty in the common
    /// (well-formed) case; the lenient default [`crate::validate`] ignores them
    /// (the constraint is simply skipped, as for any other ill-formed shape).
    pub(crate) pre_binding_failures: Vec<PreBindingFailure>,
    /// [OPUS-4.8] (sq-5q76d) An explicit `sh:conformanceDisallows` severity set
    /// declared in the shapes graph (on any `sh:ValidationReport` node, SHACL 1.2
    /// Core §3.9). When present it OVERRIDES the default disallowed set
    /// ({Violation, Warning, Info}) used to compute `ValidationReport::conforms`,
    /// so e.g. a graph that only disallows `sh:Violation` conforms despite a
    /// `sh:Warning` result (`core/validation-reports/conformance-disallows-001`).
    /// `None` ⇒ the default set applies.
    pub(crate) conformance_disallows: Option<Vec<String>>,
    /// [FABLE-5] (sq-11a) Ill-formed shapes-graph CONSTRUCTS discovered at parse:
    /// a construct that violates the SHACL syntax rules (an unparsable `sh:path`,
    /// a non-integer `sh:minCount`, a malformed SHACL list, a literal where a
    /// shape or IRI is required, …). The lenient [`crate::validate`] skips each
    /// such construct exactly as before (this vec only RECORDS the skip);
    /// [`crate::validate_strict`] reports them as a failure (the W3C test-suite
    /// `sht:Failure` outcome). Empty for a well-formed shapes graph.
    pub(crate) ill_formed: Vec<IllFormedConstruct>,
}

/// [OPUS-4.8] (sq-0mjfd) One SHACL-SPARQL pre-binding failure recorded at parse:
/// the offending constraint/validator node and why pre-binding is unsound. See
/// [`ShapesModel::pre_binding_failures`].
#[derive(Debug, Clone)]
pub struct PreBindingFailure {
    /// The `sh:SPARQLConstraint` / constraint-component node whose query is unsound.
    pub node: Term,
    /// A human-readable explanation (the violated pre-binding rule).
    pub message: String,
}

/// [FABLE-5] (sq-11a) One ill-formed shapes-graph construct recorded at parse:
/// the node carrying it, the SHACL predicate whose value violates the SHACL
/// syntax rules, and why. See [`ShapesModel::ill_formed`].
#[derive(Debug, Clone)]
pub struct IllFormedConstruct {
    /// The shapes-graph node carrying the ill-formed construct (usually the shape).
    pub node: Term,
    /// The full IRI of the SHACL predicate whose value is ill-formed (e.g. `sh:minCount`).
    pub predicate: String,
    /// A human-readable explanation (the violated syntax rule).
    pub message: String,
}

impl ShapesModel {
    pub fn parse(shapes_graph: &Graph) -> ShapesModel {
        let g = GraphView::new(shapes_graph);
        let mut m = ShapesModel {
            shapes: Vec::new(),
            by_node: FxHashMap::default(),
            targeted: Vec::new(),
            sparql: Vec::new(),
            select_exprs: Vec::new(),
            components: Vec::new(),
            path_validators: Vec::new(),
            by_types_closures: FxHashMap::default(),
            #[cfg(feature = "shacl-af")]
            expressions: Vec::new(),
            pre_binding_failures: Vec::new(),
            conformance_disallows: parse_conformance_disallows(&g),
            ill_formed: Vec::new(),
        };

        // [OPUS-4.8] SHACL §6: discover SPARQL-based constraint components FIRST, so
        // shape parsing can activate them against the parameter predicates a shape uses.
        // [OPUS-4.8] (sq-0mjfd) A component validator with an unsound pre-binding is
        // recorded as a failure (surfaced by `validate_strict`).
        m.components = discover_components(&g, &mut m.pre_binding_failures);

        // [FABLE-5] (sq-c1v3e) `sh:entailment` (SHACL §1.5): a processor that does
        // not support a declared entailment regime MUST signal a failure (§3.4).
        // This engine supports NO entailment regime (data graphs are validated
        // as-asserted; SHACL-AF rule inference is a separate opt-in surface), so
        // EVERY `sh:entailment` declaration is recorded — the strict channel
        // fails, the lenient channel validates the asserted graph as before.
        for [s, _, o] in g.triples(None, Some(&sh("entailment")), None) {
            m.record_ill_formed(
                &s,
                "entailment",
                format!(
                    "unsupported entailment regime {} (this processor supports none)",
                    o
                ),
            );
        }

        // Top-level shape discovery: explicitly typed shapes plus anything with a target.
        // [OPUS-4.8] (sq-rnkdh) `sh:ShapeClass` (SHACL 1.2) is "a class that is also a
        // node shape" — discovered as a root here so its constraints are parsed and
        // its implicit class target registered (it carries neither `sh:NodeShape` nor
        // an explicit `sh:target*`).
        let mut roots: Vec<Term> = Vec::new();
        for class in ["NodeShape", "PropertyShape", "ShapeClass"] {
            for t in g.triples(None, Some(RDF_TYPE), Some(&iri(&sh(class)))) {
                roots.push(t[0].clone());
            }
        }
        for pred in [
            "targetNode",
            "targetClass",
            "targetSubjectsOf",
            "targetObjectsOf",
        ] {
            roots.extend(g.subjects_of(&sh(pred)));
        }
        // [OPUS-4.8] Implicit class shapes: a node that is an rdfs:Class with SHACL constraints is
        // itself a node shape (with an implicit class target) per the SHACL spec. Root discovery
        // previously only collected explicitly-typed shapes and shapes with sh:target* — so an
        // implicit class shape with neither was never parsed and its constraints silently ignored.
        // Include rdfs:Class subjects as root candidates; parse_shape only attaches the implicit
        // target to nodes actually typed rdfs:Class (and a constraint-free class parses to a
        // no-op shape, which validates nothing). See review 1616.
        for t in g.triples(None, Some(RDF_TYPE), Some(&iri(RDFS_CLASS))) {
            roots.push(t[0].clone());
        }
        for root in crate::view::dedup(roots) {
            m.shape_id(&g, &root);
        }

        // Fix up qualified-value-shape sibling lists: the qualified shapes of the
        // OTHER property shapes sharing a parent node shape.
        for parent in 0..m.shapes.len() {
            let children = m.shapes[parent].property_children.clone();
            if children.len() < 2 {
                continue;
            }
            let mut quals: Vec<(usize, Vec<usize>)> = Vec::new(); // (child, its qualified shapes)
            for &c in &children {
                let qs: Vec<usize> = m.shapes[c]
                    .components
                    .iter()
                    .filter_map(|comp| match comp {
                        Component::Qualified { shape, .. } => Some(*shape),
                        _ => None,
                    })
                    .collect();
                quals.push((c, qs));
            }
            for (child, _) in &quals {
                let siblings: Vec<usize> = quals
                    .iter()
                    .filter(|(c, _)| c != child)
                    .flat_map(|(_, qs)| qs.iter().copied())
                    .collect();
                for comp in &mut m.shapes[*child].components {
                    if let Component::Qualified { siblings: s, .. } = comp {
                        *s = siblings.clone();
                    }
                }
            }
        }

        // [OPUS-4.8] (sq-mk9n, `shacl-af`) Second pass: parse each shape's
        // `sh:expression` node expression into a component. Done after shape
        // discovery so a filter expression can reference any declared shape; an
        // inline anonymous filter shape inside the expression is registered on
        // demand here so it is parsed and conformance-checkable at eval time.
        #[cfg(feature = "shacl-af")]
        m.parse_expression_constraints(shapes_graph);

        // [OPUS-4.8] (sq-vg3y) Resolve the data-independent `sh:closed sh:ByTypes`
        // property closures once (only if any shape uses that mode).
        if m.shapes.iter().any(|s| {
            s.components
                .iter()
                .any(|c| matches!(c, Component::Closed { by_types: true, .. }))
        }) {
            m.by_types_closures = compute_by_types_closures(&g);
        }

        // Validation entry points: shapes with targets.
        m.targeted = (0..m.shapes.len())
            .filter(|&i| !m.shapes[i].targets.is_empty())
            .collect();
        m
    }

    /// [OPUS-4.8] (sq-vg3y) The precomputed `sh:closed sh:ByTypes` property closure
    /// for a shapes-graph node, or `None` if the node pulls in no properties (e.g.
    /// a data class with no shapes-graph footprint). See [`Self::by_types_closures`].
    pub(crate) fn by_types_closure(&self, node: &Term) -> Option<&Vec<String>> {
        self.by_types_closures.get(node)
    }

    /// [OPUS-4.8] (sq-0mjfd) The SHACL-SPARQL pre-binding FAILURES discovered while
    /// parsing this shapes graph: a `sh:sparql` constraint or constraint-component
    /// validator whose query the pre-binding rules forbid (MINUS / VALUES /
    /// SERVICE / a sub-SELECT dropping a pre-bound variable / a BIND re-binding
    /// one). A conformant processor MUST signal a *failure* for these (W3C SHACL
    /// §3.4); [`crate::validate_strict`] returns an `Err` when this is non-empty.
    /// Empty in the common (well-formed) case.
    pub fn pre_binding_failures(&self) -> &[PreBindingFailure] {
        &self.pre_binding_failures
    }

    /// [FABLE-5] (sq-11a) The ill-formed shapes-graph CONSTRUCTS discovered while
    /// parsing this shapes graph — constructs that violate the SHACL syntax rules
    /// (an unparsable `sh:path`, a non-integer `sh:minCount`, a malformed SHACL
    /// list, a literal where a shape or IRI is required, …). The lenient
    /// [`crate::validate`] SKIPS each such construct (unchanged policy; this only
    /// records the skip) and [FABLE-5] (sq-c1v3e) surfaces each record as a
    /// [`crate::ShapeDiagnostic`] on the report; [`crate::validate_strict`]
    /// returns an `Err` when this is non-empty — the W3C test-suite `sht:Failure`
    /// outcome. Empty for a well-formed shapes graph. Detection is parse-time and
    /// construct-local; it is NOT a full SHACL-of-SHACL syntax check (README).
    pub fn ill_formed(&self) -> &[IllFormedConstruct] {
        &self.ill_formed
    }

    /// [FABLE-5] (sq-11a) Records one ill-formed construct (see [`Self::ill_formed`]).
    fn record_ill_formed(&mut self, node: &Term, pred: &str, message: String) {
        self.ill_formed.push(IllFormedConstruct {
            node: node.clone(),
            predicate: sh(pred),
            message,
        });
    }

    /// [FABLE-5] (sq-11a) Syntax-rule check for a predicate whose value must be an
    /// IRI or (SHACL 1.2) a well-formed SHACL list of IRIs (`sh:datatype` /
    /// `sh:nodeKind` / `sh:uniqueValuesFor` / `sh:class`). Records an ill-formed
    /// construct for anything else; never alters what the caller builds.
    fn check_iri_or_iri_list(&mut self, g: &GraphView, node: &Term, pred: &str, o: &Term) {
        match o {
            Term::NamedNode(_) => {}
            Term::BlankNode(_) => match g.list_strict(o) {
                Some(members) if members.iter().all(|m| matches!(m, Term::NamedNode(_))) => {}
                Some(_) => self.record_ill_formed(
                    node,
                    pred,
                    format!("every member of the sh:{} list must be an IRI", pred),
                ),
                None => self.record_ill_formed(
                    node,
                    pred,
                    format!(
                        "the value of sh:{} must be an IRI or a well-formed SHACL list of IRIs",
                        pred
                    ),
                ),
            },
            _ => self.record_ill_formed(
                node,
                pred,
                format!("the value of sh:{} must be an IRI, got a literal", pred),
            ),
        }
    }

    /// [FABLE-5] (sq-c1v3e) Syntax-rule check for the count/length parameters
    /// (`sh:minCount` / `sh:maxCount` / `sh:minLength` / `sh:maxLength` /
    /// `sh:minListLength` / `sh:maxListLength` / `sh:qualifiedMinCount` /
    /// `sh:qualifiedMaxCount`): the value must be a literal with DATATYPE
    /// `xsd:integer` and an integer lexical form — `"3"^^xsd:string` is
    /// integer-lexical but ill-formed (bare Turtle `3` types as `xsd:integer`, so
    /// ordinary shapes graphs — including every W3C-suite fixture — are
    /// unaffected). A well-formed NEGATIVE (or u64-overflowing) integer stays a
    /// silent skip. Records only; never alters what the caller builds.
    fn check_integer_literal(&mut self, node: &Term, pred: &str, o: &Term) {
        let well_formed = matches!(o, Term::Literal(l)
            if l.datatype().as_str() == XSD_INTEGER && is_integer_lexical(l.value()));
        if !well_formed {
            self.record_ill_formed(
                node,
                pred,
                format!("the value of sh:{} must be an xsd:integer-typed literal", pred),
            );
        }
    }

    /// [FABLE-5] (sq-11a) Syntax-rule check for a shape-valued object (`sh:not` /
    /// `sh:node` / `sh:property` / an `sh:and` list member, …): a shape is an IRI
    /// or blank node, never a literal. Records only; the caller still interns the
    /// degenerate shape (which validates nothing) exactly as before.
    fn check_shape_ref(&mut self, node: &Term, pred: &str, o: &Term) {
        if matches!(o, Term::Literal(_)) {
            self.record_ill_formed(
                node,
                pred,
                format!(
                    "the value of sh:{} must be a shape (an IRI or blank node), got a literal",
                    pred
                ),
            );
        }
    }

    /// [OPUS-4.8] (sq-5q76d) The shapes-graph-declared `sh:conformanceDisallows`
    /// severity set (full IRIs), or `None` when the graph declares none (the
    /// default {Violation, Warning, Info} then applies). Used by
    /// [`crate::validate_with_model`] to compute `ValidationReport::conforms`.
    pub fn conformance_disallows(&self) -> Option<&[String]> {
        self.conformance_disallows.as_deref()
    }

    /// [OPUS-4.8] (sq-mk9n / sq-3w6n, `shacl-af`) Parses the two SHACL-AF
    /// node-expression constraints — `sh:expression`
    /// (→ [`Component::Expression`]) and `sh:nodeByExpression`
    /// (→ [`Component::NodeByExpression`]) — on every shape. Inline filter shapes
    /// used by the expressions, and the node shapes a `sh:nodeByExpression`
    /// expression names as a constant, are registered first (so they are
    /// conformance-checkable), then the expressions are parsed and attached.
    #[cfg(feature = "shacl-af")]
    fn parse_expression_constraints(&mut self, shapes_graph: &Graph) {
        let g = GraphView::new(shapes_graph);
        // Collect (shape_id, expression_term, is_node_by_expr) for every shape
        // carrying sh:expression or sh:nodeByExpression.
        let mut pending: Vec<(usize, Term, bool)> = Vec::new();
        for sid in 0..self.shapes.len() {
            let node = self.shapes[sid].node.clone();
            for expr in g.objects(&node, &sh("expression")) {
                pending.push((sid, expr, false));
            }
            for expr in g.objects(&node, &sh("nodeByExpression")) {
                pending.push((sid, expr, true));
            }
        }
        // Register any inline filter shapes the expressions reference (best-effort)
        // so `sh:filterShape` / function filter shapes resolve at eval time.
        for (_, expr, is_nbe) in &pending {
            self.register_expression_shapes(shapes_graph, expr, 0);
            // A `sh:nodeByExpression` whose expression names a node shape as a
            // constant IRI must have that shape parsed so conformance can be
            // checked at eval time (it may not be a target-bearing root).
            if *is_nbe && matches!(expr, Term::NamedNode(_)) {
                self.ensure_shape(shapes_graph, expr);
            }
        }
        // Parse each expression and attach the component. [FABLE-5] (sq-c1v3e) A
        // structural (blank-node) node expression that does NOT build — an
        // unsupported operator/function IRI, an ill-formed operand list, a filter
        // shape absent from the model — was previously dropped SILENTLY; it is now
        // recorded for the strict channel (fail-closed relative to THIS engine's
        // node-expression coverage, like the sq-ehq4g SPARQL-parser boundary).
        // The lenient skip is unchanged.
        let mut parsed: Vec<(usize, crate::rules::NodeExpr, bool)> = Vec::new();
        for (sid, expr, is_nbe) in pending {
            let pred = if is_nbe { "nodeByExpression" } else { "expression" };
            match crate::rules::parse_node_expr(&g, self, &expr) {
                Some(ne) => parsed.push((sid, ne, is_nbe)),
                None => {
                    let shape_node = self.shapes[sid].node.clone();
                    self.record_ill_formed(
                        &shape_node,
                        pred,
                        format!(
                            "the sh:{} node expression is unsupported or ill-formed",
                            pred
                        ),
                    );
                }
            }
        }
        for (sid, expr, is_nbe) in parsed {
            let idx = self.expressions.len();
            self.expressions.push(expr);
            self.shapes[sid].components.push(if is_nbe {
                Component::NodeByExpression(idx)
            } else {
                Component::Expression(idx)
            });
            // [OPUS-4.8] (sq-pb0wm) Keep the per-statement `component_meta` vector
            // index-aligned: this second pass pushes components AFTER
            // `attach_component_meta` sized the meta vector, so push a default
            // (no-override) meta for each — expression constraints carry no
            // reified-annotation override. (`validate_shape` is also padded
            // defensively, so a drift here can never silently drop a component.)
            self.shapes[sid].component_meta.push(ComponentMeta::default());
            self.shapes[sid]
                .qualified_min_meta
                .push(ComponentMeta::default());
            self.shapes[sid]
                .qualified_max_meta
                .push(ComponentMeta::default());
        }
    }

    /// [OPUS-4.8] (sq-mue75) Registers the inline filter/function shapes a
    /// standalone node expression references (the public `eval_node_expression`
    /// seam): the same on-demand inline-shape registration the `sh:expression`
    /// constraint parse does, so a `findFirst`/`matchAll`/`nodesMatching`/
    /// `filterShape` over an anonymous `[ … ]` shape resolves at eval time.
    #[cfg(feature = "shacl-af")]
    pub(crate) fn register_node_expr_shapes(&mut self, shapes_graph: &Graph, expr: &Term) {
        self.register_expression_shapes(shapes_graph, expr, 0);
    }

    /// Walks an expression term registering inline `sh:filterShape` / function
    /// filter shapes (`shnex:`/`sh:` `findFirst`/`matchAll`/`nodesMatching`) as
    /// shapes so they resolve. Depth-bounded against pathological cyclic graphs.
    #[cfg(feature = "shacl-af")]
    fn register_expression_shapes(&mut self, shapes_graph: &Graph, term: &Term, depth: usize) {
        if depth > 64 || matches!(term, Term::Literal(_)) {
            return;
        }
        let g = GraphView::new(shapes_graph);
        const SHNEX: &str = "http://www.w3.org/ns/shacl-node-expr#";
        for local in ["filterShape", "findFirst", "matchAll", "nodesMatching"] {
            for pred in [format!("{SHNEX}{local}"), sh(local)] {
                if let Some(shape_term) = g.object(term, &pred) {
                    self.ensure_shape(shapes_graph, &shape_term);
                }
            }
        }
        // Recurse into nested operand expressions / list members.
        for (_, obj) in g.predicate_objects(term) {
            if matches!(obj, Term::BlankNode(_)) {
                self.register_expression_shapes(shapes_graph, &obj, depth + 1);
            }
        }
    }

    pub fn by_node(&self, node: &Term) -> Option<usize> {
        self.by_node.get(node).copied()
    }

    /// [OPUS-4.8] (sq-mk9n, `shacl-af`) Ensures `node` is parsed as a shape and
    /// returns its id, parsing it on demand from `shapes_graph` if it was not
    /// discovered by top-level root discovery. SHACL-AF node-expression *filter
    /// shapes* (`sh:filterShape`, `shnex:findFirst`/`matchAll`/`nodesMatching`)
    /// may be **inline anonymous** shapes (e.g. `[ sh:minInclusive 3 ]`) that
    /// carry no `rdf:type`/target, so they are not roots; this lets the function
    /// registry register such a shape and then check conformance against it.
    /// `None` only if `node` is a literal (literals are never shapes).
    #[cfg(feature = "shacl-af")]
    pub(crate) fn ensure_shape(&mut self, shapes_graph: &Graph, node: &Term) -> Option<usize> {
        if matches!(node, Term::Literal(_)) {
            return None;
        }
        if let Some(id) = self.by_node(node) {
            return Some(id);
        }
        let g = GraphView::new(shapes_graph);
        Some(self.shape_id(&g, node))
    }

    /// The id of the shape rooted at `node`, parsing it (and, recursively, the
    /// shapes it references) on first sight. A placeholder breaks cycles.
    fn shape_id(&mut self, g: &GraphView, node: &Term) -> usize {
        if let Some(&id) = self.by_node.get(node) {
            return id;
        }
        let id = self.shapes.len();
        self.by_node.insert(node.clone(), id);
        self.shapes.push(Shape {
            node: node.clone(),
            path: None,
            targets: Vec::new(),
            components: Vec::new(),
            component_meta: Vec::new(),
            qualified_min_meta: Vec::new(),
            qualified_max_meta: Vec::new(),
            severity: sh("Violation"),
            messages: Vec::new(),
            deactivated: false,
            property_children: Vec::new(),
            value_expr: None,
        });
        let mut parsed = self.parse_shape(g, node);
        // [OPUS-4.8] (sq-pb0wm) Resolve per-constraint-statement RDF-1.2
        // reified-annotation overrides AFTER the components are built, so the
        // parallel `component_meta` vector is index-aligned with `components`. A
        // no-op (and cheap) when the shape carries no `{| … |}` annotation.
        self.attach_component_meta(g, node, &mut parsed);
        self.shapes[id] = parsed;
        id
    }

    /// [OPUS-4.8] (sq-pb0wm) Populates `shape.component_meta` (index-aligned with
    /// `shape.components`) from RDF-1.2 reified annotations on the constraint
    /// statements. SHACL 1.2 Core lets a `{| … |}` annotation on ONE constraint
    /// triple `(shapeNode, P, O)` — stored after parsing as
    /// `_:r rdf:reifies <<( shapeNode P O )>>` plus `_:r sh:deactivated|message|severity V`
    /// on the reifier `_:r` — override JUST that occurrence
    /// (`misc/{deactivated-003,message-002,severity-003}`).
    ///
    /// Every component gets a [`ComponentMeta`] (default = no override). A real
    /// override is attached only to components that map back to a single reifiable
    /// constraint triple ([`component_source_triple`]); composite / list-valued /
    /// expression components do not (the reified-annotation form annotates one
    /// statement). The common un-annotated case short-circuits: a shape with no
    /// reifier of any of its statements gets all-default metas with no per-triple
    /// scan.
    fn attach_component_meta(&self, g: &GraphView, node: &Term, shape: &mut Shape) {
        shape.component_meta = vec![ComponentMeta::default(); shape.components.len()];
        shape.qualified_min_meta = vec![ComponentMeta::default(); shape.components.len()];
        shape.qualified_max_meta = vec![ComponentMeta::default(); shape.components.len()];
        // Short-circuit: nothing reifies a statement of this shape ⇒ no overrides.
        // (`rdf:reifies` objects are triple-terms; we only need the cheap presence
        // check that SOME reifier names this shape node as the triple subject.)
        if !shape_has_reified_statement(g, node) {
            return;
        }
        for (i, comp) in shape.components.iter().enumerate() {
            if matches!(comp, Component::Qualified { .. }) {
                for (pred, metas) in [
                    ("qualifiedMinCount", &mut shape.qualified_min_meta),
                    ("qualifiedMaxCount", &mut shape.qualified_max_meta),
                ] {
                    if let Some(obj) = g.object(node, &sh(pred)) {
                        metas[i] = reified_meta(g, node, &sh(pred), &obj);
                    }
                }
            }
            let Some((pred, obj)) = self.component_source_triple(comp) else {
                continue;
            };
            let meta = reified_meta(g, node, &pred, &obj);
            if !meta.is_empty() {
                shape.component_meta[i] = meta;
            }
        }
    }

    /// [OPUS-4.8] (sq-pb0wm) The `(predicateIRI, objectTerm)` of the single
    /// constraint statement a [`Component`] was built from, for the variants that
    /// correspond to exactly one reifiable shapes-graph triple
    /// `(shapeNode, P, O)`. `None` for components built from several triples (a
    /// SHACL list / RDF-list operand), a transformed object whose original term is
    /// not faithfully recoverable (a numeric count, a parsed [`Path`]), or a
    /// feature-gated expression component — per-statement annotation overrides are
    /// defined for single-statement Core constraints, which is what the W3C 1.2
    /// suite (`misc/{deactivated-003,message-002,severity-003}`) exercises.
    fn component_source_triple(&self, comp: &Component) -> Option<(String, Term)> {
        let shape_node = |idx: usize| self.shapes.get(idx).map(|s| s.node.clone());
        match comp {
            Component::Class(t) => Some((sh("class"), t.clone())),
            // A SINGLE-element datatype/nodeKind set is the single-IRI spelling
            // `sh:datatype <iri>` (the SHACL-1.2 disjunctive LIST form is several
            // triples — its operand is an RDF list — so it is not annotated here).
            Component::Datatype(dts) if dts.len() == 1 => {
                Some((sh("datatype"), iri(&dts[0])))
            }
            Component::NodeKind(kinds) if kinds.len() == 1 => {
                Some((sh("nodeKind"), iri(&kinds[0])))
            }
            Component::HasValue(t) => Some((sh("hasValue"), t.clone())),
            Component::RootClass(t) => Some((sh("rootClass"), t.clone())),
            Component::Node(idx) => Some((sh("node"), shape_node(*idx)?)),
            Component::Property(idx) => Some((sh("property"), shape_node(*idx)?)),
            Component::SomeValue(idx) => Some((sh("someValue"), shape_node(*idx)?)),
            Component::Not(idx) => Some((sh("not"), shape_node(*idx)?)),
            Component::MemberShape(idx) => Some((sh("memberShape"), shape_node(*idx)?)),
            _ => None,
        }
    }

    fn parse_shape(&mut self, g: &GraphView, node: &Term) -> Shape {
        // [FABLE-5] (sq-11a) An unparsable `sh:path` expression (or more than one
        // `sh:path` value — the syntax rules require exactly one) is ill-formed:
        // recorded for the strict channel, then skipped exactly as before (the
        // shape keeps `path: None`).
        let path_objects = g.objects(node, &sh("path"));
        if path_objects.len() > 1 {
            self.record_ill_formed(
                node,
                "path",
                "a property shape must have exactly one sh:path value".to_string(),
            );
        }
        // [FABLE-5] (sq-ehq4g) A node explicitly typed `sh:PropertyShape` with NO
        // `sh:path` is ill-formed (the syntax rules give a property shape exactly
        // one). Recorded, then parsed as the same degenerate no-path shape as
        // before. The untyped `sh:property [ … ]` spelling is checked at its use
        // site (the `sh:property` loop), which this type guard keeps single-shot.
        if path_objects.is_empty() && g.has_type(node, &sh("PropertyShape")) {
            self.record_ill_formed(
                node,
                "path",
                "a property shape must have exactly one sh:path value".to_string(),
            );
        }
        let path = path_objects
            .into_iter()
            .next()
            .and_then(|p| match Path::parse(g, &p) {
                Ok(parsed) => Some(parsed),
                Err(e) => {
                    self.record_ill_formed(node, "path", format!("ill-formed sh:path: {}", e));
                    None
                }
            });
        // [FABLE-5] (sq-c1v3e) A non-IRI `sh:severity` is ill-formed (syntax rule
        // severity-nodeKind: the value is an IRI — sh:Info/Warning/Violation or a
        // custom severity). Recorded; the shape keeps the default severity below,
        // exactly as before.
        for o in g.objects(node, &sh("severity")) {
            if !matches!(o, Term::NamedNode(_)) {
                self.record_ill_formed(
                    node,
                    "severity",
                    "the value of sh:severity must be an IRI".to_string(),
                );
            }
        }
        let mut shape = Shape {
            node: node.clone(),
            path,
            targets: Vec::new(),
            components: Vec::new(),
            component_meta: Vec::new(),
            qualified_min_meta: Vec::new(),
            qualified_max_meta: Vec::new(),
            severity: match g.object(node, &sh("severity")) {
                Some(Term::NamedNode(n)) => n.as_str().to_string(),
                _ => sh("Violation"),
            },
            messages: g.objects(node, &sh("message")),
            deactivated: matches!(
                g.object(node, &sh("deactivated")),
                Some(Term::Literal(l)) if l.value() == "true"
            ),
            property_children: Vec::new(),
            value_expr: None,
        };

        // Targets.
        for t in g.objects(node, &sh("targetNode")) {
            // [OPUS-4.8] (sq-rnkdh) SHACL 1.2: a `sh:targetNode` object that is a
            // SPARQL-based node expression (`[ sh:select … ]` / `[ sh:sparqlExpr … ]`)
            // COMPUTES the target nodes; any other object is a literal target node.
            match self.intern_select_expr(g, &t) {
                Some(idx) => shape.targets.push(Target::Sparql(idx)),
                None => shape.targets.push(Target::Node(t)),
            }
        }
        for t in g.objects(node, &sh("targetClass")) {
            // [FABLE-5] (sq-11a) A literal is never a class (syntax rule
            // targetClass-nodeKind); the target below matches nothing, as before.
            if matches!(t, Term::Literal(_)) {
                self.record_ill_formed(
                    node,
                    "targetClass",
                    "the value of sh:targetClass must be an IRI, got a literal".to_string(),
                );
            }
            shape.targets.push(Target::Class(t));
        }
        for t in g.objects(node, &sh("targetSubjectsOf")) {
            if let Term::NamedNode(n) = t {
                shape
                    .targets
                    .push(Target::SubjectsOf(n.as_str().to_string()));
            } else {
                // [FABLE-5] (sq-11a) Recorded, then skipped exactly as before.
                self.record_ill_formed(
                    node,
                    "targetSubjectsOf",
                    "the value of sh:targetSubjectsOf must be a predicate IRI".to_string(),
                );
            }
        }
        for t in g.objects(node, &sh("targetObjectsOf")) {
            if let Term::NamedNode(n) = t {
                shape
                    .targets
                    .push(Target::ObjectsOf(n.as_str().to_string()));
            } else {
                // [FABLE-5] (sq-11a) Recorded, then skipped exactly as before.
                self.record_ill_formed(
                    node,
                    "targetObjectsOf",
                    "the value of sh:targetObjectsOf must be a predicate IRI".to_string(),
                );
            }
        }
        // [OPUS-4.8] (sq-rnkdh) SHACL 1.2 `sh:targetWhere`: the focus nodes are the
        // data-graph nodes that CONFORM to the inline (object) shape. Parse the
        // object as a shape on demand (it may be an inline anonymous shape with no
        // `rdf:type`/target, so it is not a top-level root) and record its id.
        for t in g.objects(node, &sh("targetWhere")) {
            let inner = self.shape_id(g, &t);
            shape.targets.push(Target::Where(inner));
        }
        // Implicit class target: the shape node is itself an rdfs:Class, OR (SHACL
        // 1.2) an `sh:ShapeClass` — "a class that is also a node shape", usable as
        // `rdf:type` in place of the `rdfs:Class` + `sh:NodeShape` combination.
        if matches!(node, Term::NamedNode(_))
            && (g.has_type(node, RDFS_CLASS) || g.has_type(node, &sh("ShapeClass")))
        {
            shape.targets.push(Target::ImplicitClass(node.clone()));
        }

        // [OPUS-4.8] (sq-rnkdh) SHACL 1.2 `sh:values [ sh:select … ]` /
        // `[ sh:sparqlExpr … ]` on a property shape: the value nodes are COMPUTED by
        // the SPARQL node expression (`$this` = focus node) instead of derived by
        // traversing `sh:path`. The reported `sh:resultPath` stays the shape's path.
        // Only the SPARQL-valued `sh:values` form is handled here; the SHACL-AF
        // node-expression value RULE form (`apply_rules`) is a separate surface.
        if let Some(v) = g.object(node, &sh("values")) {
            if let Some(idx) = self.intern_select_expr(g, &v) {
                shape.value_expr = Some(idx);
            }
        }

        let c = &mut shape.components;
        // [OPUS-4.8] (sq-sx15d) `sh:class` accepts a single class IRI/blank node or
        // the SHACL-1.2 disjunctive SHACL-list form `( ex:A ex:B )` — a value node
        // conforms iff it is an instance of ANY listed class. A blank-node object
        // that is a well-formed SHACL list (≥1 member) is the disjunctive form;
        // any other object is a single class. Mirrors the `sh:datatype` /
        // `sh:nodeKind` disjunctive handling above.
        for o in g.objects(node, &sh("class")) {
            // [FABLE-5] (sq-11a) Recorded when ill-formed (a literal, or a blank
            // node that is not a well-formed IRI list — a shapes-graph blank node
            // never denotes a data-graph class); the component is built as before.
            self.check_iri_or_iri_list(g, node, "class", &o);
            match &o {
                Term::BlankNode(_) => {
                    let members = g.list(&o);
                    if members.is_empty() {
                        c.push(Component::Class(o));
                    } else {
                        c.push(Component::ClassIn(members));
                    }
                }
                _ => c.push(Component::Class(o)),
            }
        }
        // [OPUS-4.8] (sq-vg3y) `sh:datatype` / `sh:nodeKind` accept either a single
        // IRI or the SHACL-1.2 disjunctive SHACL-list form (`( a b )`). `iri_set`
        // returns the IRI set for either spelling; an empty set (e.g. a literal
        // object, or an ill-formed list) contributes no constraint.
        for o in g.objects(node, &sh("datatype")) {
            // [FABLE-5] (sq-11a) Recorded when ill-formed; built as before.
            self.check_iri_or_iri_list(g, node, "datatype", &o);
            let dts = iri_set(g, &o);
            if !dts.is_empty() {
                c.push(Component::Datatype(dts));
            }
        }
        for o in g.objects(node, &sh("nodeKind")) {
            // [FABLE-5] (sq-11a) Recorded when ill-formed; built as before.
            self.check_iri_or_iri_list(g, node, "nodeKind", &o);
            let kinds = iri_set(g, &o);
            // [FABLE-5] (sq-ehq4g) Each kind must be one of the SIX sh:* node
            // kinds (SHACL §4.6.1); recorded when not, then built as before (an
            // unknown kind matches no value node at eval, unchanged).
            for kind in &kinds {
                if !is_node_kind_iri(kind) {
                    self.record_ill_formed(
                        node,
                        "nodeKind",
                        format!(
                            "the value of sh:nodeKind must be one of the six sh:* node kinds, got <{}>",
                            kind
                        ),
                    );
                }
            }
            if !kinds.is_empty() {
                c.push(Component::NodeKind(kinds));
            }
        }
        for (pred, ctor) in [
            ("minCount", Component::MinCount as fn(u64) -> Component),
            ("maxCount", Component::MaxCount as fn(u64) -> Component),
            ("minLength", Component::MinLength as fn(u64) -> Component),
            ("maxLength", Component::MaxLength as fn(u64) -> Component),
        ] {
            for o in g.objects(node, &sh(pred)) {
                // [FABLE-5] (sq-11a / sq-c1v3e) Recorded when not an
                // xsd:integer-typed integer literal; built exactly as before (a
                // `"3"^^xsd:string` still builds the component leniently).
                self.check_integer_literal(node, pred, &o);
                if let Term::Literal(l) = &o {
                    if let Ok(n) = l.value().parse::<u64>() {
                        c.push(ctor(n));
                    }
                }
            }
        }
        for (pred, ctor) in [
            (
                "minExclusive",
                Component::MinExclusive as fn(Term) -> Component,
            ),
            (
                "minInclusive",
                Component::MinInclusive as fn(Term) -> Component,
            ),
            (
                "maxExclusive",
                Component::MaxExclusive as fn(Term) -> Component,
            ),
            (
                "maxInclusive",
                Component::MaxInclusive as fn(Term) -> Component,
            ),
        ] {
            for o in g.objects(node, &sh(pred)) {
                // [FABLE-5] (sq-11a) A non-literal range comparand is ill-formed
                // (syntax rule: a literal); recorded, and still built as before
                // (at eval nothing compares against it, unchanged).
                if !matches!(o, Term::Literal(_)) {
                    self.record_ill_formed(
                        node,
                        pred,
                        format!("the value of sh:{} must be a literal", pred),
                    );
                }
                c.push(ctor(o));
            }
        }
        // [FABLE-5] (sq-11a) A non-literal sh:flags is ill-formed (syntax rule: a
        // string literal); recorded, then ignored exactly as before. (An
        // uncompilable REGEX stays an eval-time diagnostic, NOT a failure — the
        // documented Rust-vs-XPath regex-dialect boundary, sq-lz99x.)
        if let Some(o) = g.object(node, &sh("flags")) {
            if !matches!(o, Term::Literal(_)) {
                self.record_ill_formed(
                    node,
                    "flags",
                    "the value of sh:flags must be a string literal".to_string(),
                );
            }
        }
        let flags = g.str_object(node, &sh("flags"));
        for o in g.objects(node, &sh("pattern")) {
            if let Term::Literal(l) = &o {
                c.push(Component::Pattern {
                    source: l.value().to_string(),
                    flags: flags.clone(),
                });
            } else {
                // [FABLE-5] (sq-11a) Recorded, then skipped exactly as before.
                self.record_ill_formed(
                    node,
                    "pattern",
                    "the value of sh:pattern must be a string literal".to_string(),
                );
            }
        }
        for o in g.objects(node, &sh("languageIn")) {
            // [FABLE-5] (sq-11a) The value must be a well-formed SHACL list of
            // literals; recorded when not, then built leniently as before.
            match g.list_strict(&o) {
                Some(members) if members.iter().all(|m| matches!(m, Term::Literal(_))) => {}
                Some(_) => self.record_ill_formed(
                    node,
                    "languageIn",
                    "every member of the sh:languageIn list must be a string literal".to_string(),
                ),
                None => self.record_ill_formed(
                    node,
                    "languageIn",
                    "the value of sh:languageIn must be a well-formed SHACL list".to_string(),
                ),
            }
            let tags: Vec<String> = g
                .list(&o)
                .into_iter()
                .filter_map(|t| match t {
                    Term::Literal(l) => Some(l.value().to_string()),
                    _ => None,
                })
                .collect();
            c.push(Component::LanguageIn(tags));
        }
        if matches!(g.object(node, &sh("uniqueLang")), Some(Term::Literal(l)) if l.value() == "true")
        {
            c.push(Component::UniqueLang);
        }
        // [OPUS-4.8] (sq-sx15d) `sh:equals` / `sh:disjoint` / `sh:lessThan` /
        // `sh:lessThanOrEquals` / `sh:subsetOf` carry a comparand SHACL property
        // PATH in SHACL 1.2 (often an RDF-list sequence), not just a predicate
        // IRI. Parse the comparand with the same `Path` parser used for `sh:path`
        // (a bare NamedNode → `Path::Predicate`, so the SHACL-1.0 predicate forms
        // stay backward-compatible); an ill-formed comparand is dropped (lenient).
        for (pred, ctor) in [
            ("equals", Component::Equals as fn(Path) -> Component),
            ("disjoint", Component::Disjoint as fn(Path) -> Component),
            ("lessThan", Component::LessThan as fn(Path) -> Component),
            (
                "lessThanOrEquals",
                Component::LessThanOrEquals as fn(Path) -> Component,
            ),
            ("subsetOf", Component::SubsetOf as fn(Path) -> Component),
        ] {
            for o in g.objects(node, &sh(pred)) {
                match Path::parse(g, &o) {
                    Ok(path) => c.push(ctor(path)),
                    Err(e) => {
                        // [FABLE-5] (sq-11a) Recorded, then dropped exactly as before.
                        self.record_ill_formed(
                            node,
                            pred,
                            format!("ill-formed sh:{} comparand path: {}", pred, e),
                        );
                    }
                }
            }
        }
        // [OPUS-4.8] (sq-sx15d) `sh:rootClass` (SHACL 1.2): each value node must be
        // the named class or a transitive `rdfs:subClassOf`-descendant of it. The
        // object is a single class term.
        for o in g.objects(node, &sh("rootClass")) {
            c.push(Component::RootClass(o));
        }
        // [OPUS-4.8] (sq-sx15d) `sh:singleLine true` (SHACL 1.2): string values must
        // contain no line-break characters. `sh:singleLine false` (or any non-true
        // object) imposes no constraint.
        if matches!(g.object(node, &sh("singleLine")), Some(Term::Literal(l)) if l.value() == "true")
        {
            c.push(Component::SingleLine);
        }
        for o in g.objects(node, &sh("hasValue")) {
            c.push(Component::HasValue(o));
        }
        for o in g.objects(node, &sh("in")) {
            // [FABLE-5] (sq-11a) The value must be a well-formed SHACL list;
            // recorded when not, then built leniently (truncated tail) as before.
            if g.list_strict(&o).is_none() {
                self.record_ill_formed(
                    node,
                    "in",
                    "the value of sh:in must be a well-formed SHACL list".to_string(),
                );
            }
            let members = g.list(&o);
            c.push(Component::In(members));
        }
        // [OPUS-4.8] (sq-vg3y) `sh:maxListLength` / `sh:minListLength` (SHACL-1.2):
        // value nodes must be SHACL lists with at most / at least N members.
        for (pred, ctor) in [
            (
                "maxListLength",
                Component::MaxListLength as fn(u64) -> Component,
            ),
            (
                "minListLength",
                Component::MinListLength as fn(u64) -> Component,
            ),
        ] {
            for o in g.objects(node, &sh(pred)) {
                // [FABLE-5] (sq-11a / sq-c1v3e) As for the count/length
                // constraints above: recorded when not an xsd:integer-typed
                // integer literal (a negative integer is a well-formed silent
                // skip); built exactly as before.
                self.check_integer_literal(node, pred, &o);
                if let Term::Literal(l) = &o {
                    if let Ok(n) = l.value().parse::<u64>() {
                        c.push(ctor(n));
                    }
                }
            }
        }
        // [OPUS-4.8] (sq-vg3y) `sh:uniqueMembers true` (SHACL-1.2): SHACL-list value
        // nodes must have pairwise-distinct members.
        if matches!(g.object(node, &sh("uniqueMembers")), Some(Term::Literal(l)) if l.value() == "true")
        {
            c.push(Component::UniqueMembers);
        }
        // [OPUS-4.8] (sq-vg3y) `sh:uniqueValuesFor` (SHACL-1.2): one property IRI or
        // a SHACL list of property IRIs forming a composite uniqueness key.
        for o in g.objects(node, &sh("uniqueValuesFor")) {
            // [FABLE-5] (sq-11a) Recorded when ill-formed; built as before.
            self.check_iri_or_iri_list(g, node, "uniqueValuesFor", &o);
            let props = iri_set(g, &o);
            if !props.is_empty() {
                c.push(Component::UniqueValuesFor(props));
            }
        }
        // `sh:closed`: SHACL-1.0 boolean (`true`) or SHACL-1.2 `sh:ByTypes`
        // (close-by-types). Any other object (or `false`) is not a closing form.
        // [OPUS-4.8] (sq-vg3y) added the `sh:ByTypes` spelling.
        match g.object(node, &sh("closed")) {
            Some(Term::Literal(l)) if l.value() == "true" => {
                shape.components.push(Component::Closed {
                    ignored: closed_ignored(g, node),
                    by_types: false,
                });
            }
            Some(Term::NamedNode(n)) if n.as_str() == sh("ByTypes") => {
                shape.components.push(Component::Closed {
                    ignored: closed_ignored(g, node),
                    by_types: true,
                });
            }
            // [FABLE-5] (sq-11a) Any other object is ill-formed (syntax rule: an
            // xsd:boolean literal, or the SHACL-1.2 sh:ByTypes IRI) — EXCEPT the
            // well-formed boolean lexicals "false"/"0"/"1" ("false"/"0" mean "not
            // closed"; the non-canonical true "1" is a pre-existing lenient
            // non-close in this parser, which is a limitation, not ill-formedness).
            Some(o) if !matches!(&o, Term::Literal(l) if is_boolean_lexical(l.value())) => {
                self.record_ill_formed(
                    node,
                    "closed",
                    "the value of sh:closed must be a boolean literal or sh:ByTypes".to_string(),
                );
            }
            _ => {}
        }
        // [FABLE-5] (sq-11a) `sh:ignoredProperties` must be a well-formed SHACL
        // list of IRIs; recorded when not (the closing forms above still consume
        // the lenient, tail-truncated list exactly as before).
        if let Some(o) = g.object(node, &sh("ignoredProperties")) {
            match g.list_strict(&o) {
                Some(members) if members.iter().all(|m| matches!(m, Term::NamedNode(_))) => {}
                Some(_) => self.record_ill_formed(
                    node,
                    "ignoredProperties",
                    "every member of the sh:ignoredProperties list must be an IRI".to_string(),
                ),
                None => self.record_ill_formed(
                    node,
                    "ignoredProperties",
                    "the value of sh:ignoredProperties must be a well-formed SHACL list"
                        .to_string(),
                ),
            }
        }
        // [OPUS-4.8] (sq-vg3y) `sh:memberShape` (SHACL-1.2): each value node must be
        // a well-formed SHACL list whose members all conform to the referenced
        // shape. Recursive (the member shape is parsed/interned like sh:node).
        for o in g.objects(node, &sh("memberShape")) {
            // [FABLE-5] (sq-11a) A literal shape ref is recorded as ill-formed;
            // the degenerate shape is still interned (validating nothing) as before.
            self.check_shape_ref(node, "memberShape", &o);
            let id = self.shape_id(g, &o);
            shape.components.push(Component::MemberShape(id));
        }

        // Shape-referencing components (recursive).
        let nots = g.objects(node, &sh("not"));
        for o in nots {
            self.check_shape_ref(node, "not", &o);
            let id = self.shape_id(g, &o);
            shape.components.push(Component::Not(id));
        }
        for (pred, ctor) in [
            ("and", Component::And as fn(Vec<usize>) -> Component),
            ("or", Component::Or as fn(Vec<usize>) -> Component),
            ("xone", Component::Xone as fn(Vec<usize>) -> Component),
        ] {
            for o in g.objects(node, &sh(pred)) {
                // [FABLE-5] (sq-11a) The value must be a well-formed SHACL list of
                // shapes; recorded when not (or when a member is a literal), then
                // built leniently (truncated tail / degenerate member) as before.
                match g.list_strict(&o) {
                    Some(members) => {
                        for m in &members {
                            self.check_shape_ref(node, pred, m);
                        }
                    }
                    None => self.record_ill_formed(
                        node,
                        pred,
                        format!("the value of sh:{} must be a well-formed SHACL list", pred),
                    ),
                }
                let ids: Vec<usize> = g.list(&o).iter().map(|s| self.shape_id(g, s)).collect();
                shape.components.push(ctor(ids));
            }
        }
        for o in g.objects(node, &sh("node")) {
            self.check_shape_ref(node, "node", &o);
            let id = self.shape_id(g, &o);
            shape.components.push(Component::Node(id));
        }
        // [OPUS-4.8] (sq-sx15d) `sh:someValue` (SHACL 1.2): EXISTENTIAL — at least
        // one value node must conform to the referenced (nested) shape. Parsed/
        // interned recursively like `sh:node`; the quantifier is inverted at eval
        // time (a violation iff NO value conforms).
        for o in g.objects(node, &sh("someValue")) {
            self.check_shape_ref(node, "someValue", &o);
            let id = self.shape_id(g, &o);
            shape.components.push(Component::SomeValue(id));
        }
        for o in g.objects(node, &sh("property")) {
            self.check_shape_ref(node, "property", &o);
            // [FABLE-5] (sq-ehq4g) The value of sh:property must be a PROPERTY
            // shape, i.e. carry an sh:path (syntax rules; a pathless child would
            // silently validate nothing). A literal value is already recorded
            // above; an explicitly-typed `sh:PropertyShape` records once in its
            // own parse instead. The degenerate child is still interned as before.
            if !matches!(o, Term::Literal(_))
                && !g.has_type(&o, &sh("PropertyShape"))
                && g.objects(&o, &sh("path")).is_empty()
            {
                self.record_ill_formed(
                    &o,
                    "path",
                    "a property shape (the value of sh:property) must have exactly one sh:path value"
                        .to_string(),
                );
            }
            let id = self.shape_id(g, &o);
            shape.components.push(Component::Property(id));
            shape.property_children.push(id);
        }
        // [OPUS-4.8] (sq-0mjfd) `sh:reifierShape` (SHACL 1.2): the reifiers of each
        // value's asserted triple must conform to the referenced shape. Parsed/
        // interned like `sh:node`; `sh:reificationRequired true` additionally
        // requires a reifier to exist.
        for o in g.objects(node, &sh("reifierShape")) {
            self.check_shape_ref(node, "reifierShape", &o);
            let id = self.shape_id(g, &o);
            let required = matches!(
                g.object(node, &sh("reificationRequired")),
                Some(Term::Literal(l)) if l.value() == "true"
            );
            shape.components.push(Component::ReifierShape {
                shape: id,
                required,
            });
        }
        // [FABLE-5] (sq-11a / sq-c1v3e) Non-xsd:integer qualified counts are
        // ill-formed (checked once per shape node, not per qualified shape);
        // negative integers stay a well-formed silent skip, as for the
        // count/length constraints above. [FABLE-5] (sq-c1v3e) A qualified count
        // WITHOUT `sh:qualifiedValueShape` is the symmetric partial-parameter
        // case of the sq-ehq4g rule below: the qualified-cardinality component
        // (SHACL §4.7.5–6) is then missing its mandatory `sh:qualifiedValueShape`
        // parameter (SHACL §2.3.2). Recorded; the count stays inert as before.
        let qualified_shapes = g.objects(node, &sh("qualifiedValueShape"));
        for pred in ["qualifiedMinCount", "qualifiedMaxCount"] {
            if let Some(o) = g.object(node, &sh(pred)) {
                self.check_integer_literal(node, pred, &o);
                if qualified_shapes.is_empty() {
                    self.record_ill_formed(
                        node,
                        pred,
                        format!(
                            "a shape with sh:{} must also have sh:qualifiedValueShape",
                            pred
                        ),
                    );
                }
            }
        }
        // [FABLE-5] (sq-ehq4g) `sh:qualifiedValueShape` with NEITHER
        // `sh:qualifiedMinCount` nor `sh:qualifiedMaxCount` is ill-formed: both
        // qualified-cardinality constraint components (SHACL §4.7.5–6) then miss a
        // mandatory parameter, and a shape with values for SOME but not all
        // mandatory parameters of a component is ill-formed (SHACL §2.3.2).
        // Recorded once per shape node; the inert component is built as before.
        if !qualified_shapes.is_empty()
            && g.object(node, &sh("qualifiedMinCount")).is_none()
            && g.object(node, &sh("qualifiedMaxCount")).is_none()
        {
            self.record_ill_formed(
                node,
                "qualifiedValueShape",
                "a shape with sh:qualifiedValueShape must also have sh:qualifiedMinCount or sh:qualifiedMaxCount"
                    .to_string(),
            );
        }
        for o in qualified_shapes {
            self.check_shape_ref(node, "qualifiedValueShape", &o);
            let id = self.shape_id(g, &o);
            let num = |p: &str| -> Option<u64> {
                match g.object(node, &sh(p)) {
                    Some(Term::Literal(l)) => l.value().parse().ok(),
                    _ => None,
                }
            };
            shape.components.push(Component::Qualified {
                shape: id,
                min: num("qualifiedMinCount"),
                max: num("qualifiedMaxCount"),
                disjoint: matches!(
                    g.object(node, &sh("qualifiedValueShapesDisjoint")),
                    Some(Term::Literal(l)) if l.value() == "true"
                ),
                siblings: Vec::new(),
            });
        }

        // sh:sparql — SPARQL-based constraints (SHACL §5.2). The object is a node
        // carrying sh:select (required), sh:prefixes, sh:message, sh:deactivated.
        // On a property shape, $PATH in the query is pre-bound to the path.
        // [FABLE-5] (sq-11a) The boolean-valued constraint predicates: a value
        // outside the xsd:boolean lexical space is ill-formed (recorded, then
        // ignored exactly as before — each site above/below only reacts to "true").
        for pred in [
            "uniqueLang",
            "uniqueMembers",
            "singleLine",
            "deactivated",
            "reificationRequired",
            "qualifiedValueShapesDisjoint",
        ] {
            for o in g.objects(node, &sh(pred)) {
                if !matches!(&o, Term::Literal(l) if is_boolean_lexical(l.value())) {
                    self.record_ill_formed(
                        node,
                        pred,
                        format!("the value of sh:{} must be a boolean literal", pred),
                    );
                }
            }
        }

        let shape_path = shape.path.clone();
        for sp in g.objects(node, &sh("sparql")) {
            if let Some(idx) = self.parse_sparql_constraint(g, &sp, shape_path.as_ref()) {
                shape.components.push(Component::Sparql(idx));
            } else {
                // [FABLE-5] (sq-11a) No `sh:select` string literal on the
                // constraint node (SHACL-SPARQL §5.2.1 requires exactly one):
                // recorded, then skipped exactly as before. (A PRESENT but
                // unparsable/non-SELECT query is recorded inside
                // `parse_sparql_constraint` instead — sq-ehq4g.)
                self.record_ill_formed(
                    &sp,
                    "select",
                    "an sh:sparql constraint must carry an sh:select string literal".to_string(),
                );
            }
        }

        // [OPUS-4.8] SHACL §6: activate each declared constraint component whose
        // MANDATORY parameter predicates the shape all uses. The bound parameter
        // values (one object per parameter; `None` for an absent optional one)
        // are captured now and pre-bound as `$paramName` at evaluation time.
        // The shape's SPARQL property-path form, used for `$PATH` pre-binding on
        // property-shape component activations (computed once).
        let path_pp = shape_path.as_ref().and_then(Path::to_sparql_property_path);
        // Collect activations first: the `self.components` borrow below is
        // immutable, so the per-shape `$PATH` re-parse (which pushes into
        // `self.path_validators`) is deferred to after the loop.
        let mut activations: Vec<(usize, Vec<Option<Term>>, Option<PreparedComponentValidator>)> =
            Vec::new();
        for (cidx, comp) in self.components.iter().enumerate() {
            let mut args: Vec<Option<Term>> = Vec::with_capacity(comp.parameters.len());
            let mut activates = true;
            for p in &comp.parameters {
                let value = g.object(node, &p.predicate);
                if value.is_none() && !p.optional {
                    activates = false;
                    break;
                }
                args.push(value);
            }
            if activates && !comp.parameters.is_empty() {
                // [OPUS-4.8] On a property shape, pre-bind `$PATH` (SHACL §6.3):
                // re-parse the chosen validator with the shape's property-path
                // expression substituted for the `$PATH` query variable. `$PATH`
                // is a property PATH (not a term), so — like the §5.2 `sh:sparql`
                // path — it is a textual substitution rather than a VALUES row.
                let path_validator = path_pp.as_deref().and_then(|pp| {
                    comp.validator_for(true)
                        .filter(|v| v.references_path())
                        .and_then(|v| v.with_path(pp))
                });
                activations.push((cidx, args, path_validator));
            }
        }
        for (cidx, args, path_validator) in activations {
            let path_validator = path_validator.map(|v| {
                let idx = self.path_validators.len();
                self.path_validators.push(v);
                idx
            });
            shape.components.push(Component::CustomSparql {
                component: cidx,
                args,
                path_validator,
            });
        }

        shape
    }

    /// [OPUS-4.8] (sq-rnkdh) Interns a SHACL 1.2 **SPARQL-based node expression**
    /// (`[ sh:select "…" ]` or `[ sh:sparqlExpr "…" ]`, with optional
    /// `sh:prefixes`) into [`Self::select_exprs`], returning its index. `None` when
    /// `node` carries neither (so the caller falls back to the literal/term reading
    /// — e.g. a plain `sh:targetNode` IRI), or when the derived query is ill-formed
    /// (dropped leniently, like an ill-formed `sh:sparql` — [FABLE-5] (sq-ehq4g)
    /// but ALSO recorded for the strict channel, fail-closed relative to this
    /// engine's SPARQL parser). `sh:select` takes precedence over `sh:sparqlExpr`
    /// when both are present.
    fn intern_select_expr(&mut self, g: &GraphView, node: &Term) -> Option<usize> {
        // Only blank-node / IRI node-expression resources carry these predicates; a
        // literal `sh:targetNode` is a target node, never an expression.
        if matches!(node, Term::Literal(_)) {
            return None;
        }
        let prefixes = collect_prefixes_from(g, &g.objects(node, &sh("prefixes")));
        let prepared = match g.object(node, &sh("select")) {
            Some(Term::Literal(l)) => {
                let built = crate::sparql::PreparedSelectExpr::build_select(&prefixes, l.value());
                if built.is_none() {
                    self.record_ill_formed(
                        node,
                        "select",
                        "the sh:select of a SPARQL-based node expression must parse as a SPARQL SELECT query"
                            .to_string(),
                    );
                }
                built
            }
            _ => match g.object(node, &sh("sparqlExpr")) {
                Some(Term::Literal(l)) => {
                    let built = crate::sparql::PreparedSelectExpr::build_expr(&prefixes, l.value());
                    if built.is_none() {
                        self.record_ill_formed(
                            node,
                            "sparqlExpr",
                            "the sh:sparqlExpr of a SPARQL-based node expression must parse as a SPARQL expression"
                                .to_string(),
                        );
                    }
                    built
                }
                _ => return None, // not a SPARQL node expression — caller's fallback
            },
        }?;
        let idx = self.select_exprs.len();
        self.select_exprs.push(prepared);
        Some(idx)
    }

    /// Parses one `sh:sparql` constraint node into a [`SparqlConstraint`],
    /// interning it into [`Self::sparql`] and returning its index. `None` when the
    /// node has no `sh:select` literal (an ill-formed constraint — skipped).
    /// `shape_path` is the enclosing (property) shape's path, used to pre-bind the
    /// `$PATH` query variable (SHACL §5.2.1) when present.
    fn parse_sparql_constraint(
        &mut self,
        g: &GraphView,
        node: &Term,
        shape_path: Option<&Path>,
    ) -> Option<usize> {
        let raw_select = match g.object(node, &sh("select")) {
            Some(Term::Literal(l)) => l.value().to_string(),
            _ => return None,
        };
        // $PATH pre-binding: substitute the property path's SPARQL property-path
        // form for $PATH / ?PATH in the query text (a property-shape feature).
        let select = match shape_path.and_then(Path::to_sparql_property_path) {
            Some(pp) => substitute_path_var(&raw_select, &pp),
            None => raw_select,
        };
        let prefixes = self.collect_prefixes(g, node);
        let message = match g.object(node, &sh("message")) {
            Some(Term::Literal(l)) => Some(l.value().to_string()),
            _ => None,
        };
        let deactivated = matches!(
            g.object(node, &sh("deactivated")),
            Some(Term::Literal(l)) if l.value() == "true"
        );
        // [OPUS-4.8] (sq-rnkdh) A constraint-level `sh:severity` IRI overrides the
        // shape's default severity for this constraint's results (SHACL 1.2).
        // [FABLE-5] (sq-c1v3e) A non-IRI value is ill-formed, as at shape level;
        // recorded, then the shape's severity is inherited exactly as before.
        let severity = match g.object(node, &sh("severity")) {
            Some(Term::NamedNode(n)) => Some(n.as_str().to_string()),
            Some(_) => {
                self.record_ill_formed(
                    node,
                    "severity",
                    "the value of sh:severity must be an IRI".to_string(),
                );
                None
            }
            None => None,
        };
        let mut constraint = SparqlConstraint {
            node: node.clone(),
            select,
            prefixes,
            message,
            severity,
            deactivated,
            prepared: None,
        };
        // [OPUS-4.8] (sq-0mjfd) A pre-binding violation is recorded as a FAILURE
        // (surfaced by `validate_strict`) and the constraint is then dropped
        // (`prepared = None`), so the lenient `validate` simply skips it.
        constraint.prepared = match crate::sparql::PreparedSparql::build(&constraint) {
            Ok(prepared) => {
                // [FABLE-5] (sq-ehq4g) A PRESENT `sh:select` whose (post-`$PATH`-
                // substitution) text does not parse as a SPARQL SELECT query is
                // ill-formed (SHACL-SPARQL §5.2.1): recorded for the strict
                // channel, then skipped exactly as before (`prepared: None`).
                // FAIL-CLOSED boundary: "does not parse" is relative to THIS
                // engine's vendored SPARQL parser (see the crate README).
                if prepared.is_none() {
                    self.record_ill_formed(
                        node,
                        "select",
                        "the sh:select query of an sh:sparql constraint must parse as a SPARQL SELECT query"
                            .to_string(),
                    );
                }
                prepared
            }
            Err(violation) => {
                self.pre_binding_failures.push(PreBindingFailure {
                    node: node.clone(),
                    message: violation.message(),
                });
                None
            }
        };
        let idx = self.sparql.len();
        self.sparql.push(constraint);
        Some(idx)
    }

    /// Assembles SPARQL `PREFIX` declarations from a constraint node's
    /// `sh:prefixes` (SHACL §5.2.1): each `sh:prefixes` object is a prefix
    /// declarations resource that, directly or via `owl:imports`, declares
    /// `sh:declare` nodes carrying `sh:prefix` (the short name) and `sh:namespace`
    /// (the IRI). owl:imports chasing is followed one level (cycle-guarded).
    fn collect_prefixes(&self, g: &GraphView, node: &Term) -> String {
        collect_prefixes_from(g, &g.objects(node, &sh("prefixes")))
    }
}

/// [OPUS-4.8] (sq-d1dw, `shacl-af`) Crate-internal accessor to the shared
/// `sh:prefixes` collector for the SHACL-AF rules module (`sh:SPARQLRule`'s
/// `sh:construct` reuses the same `sh:declare`/`owl:imports` prefix machinery as
/// `sh:sparql`). Gated to the feature so it adds nothing when SHACL-AF is off.
#[cfg(feature = "shacl-af")]
pub(crate) fn collect_prefixes_for(g: &GraphView, prefix_roots: &[Term]) -> String {
    collect_prefixes_from(g, prefix_roots)
}

/// [OPUS-4.8] Assembles SPARQL `PREFIX` declarations from a set of `sh:prefixes`
/// declaration resources (SHACL §5.2.1 / §6.3): each root, directly or via
/// `owl:imports`, declares `sh:declare` nodes carrying `sh:prefix` (short name)
/// and `sh:namespace` (IRI). owl:imports chasing is followed transitively
/// (cycle-guarded). Shared by the `sh:sparql` and constraint-component paths.
fn collect_prefixes_from(g: &GraphView, prefix_roots: &[Term]) -> String {
    const OWL_IMPORTS: &str = "http://www.w3.org/2002/07/owl#imports";
    let mut out = String::new();
    let mut seen_decls: rustc_hash::FxHashSet<Term> = rustc_hash::FxHashSet::default();
    let mut roots: Vec<Term> = prefix_roots.to_vec();
    let mut visited_roots: rustc_hash::FxHashSet<Term> = rustc_hash::FxHashSet::default();
    let mut i = 0;
    while i < roots.len() {
        let root = roots[i].clone();
        i += 1;
        if !visited_roots.insert(root.clone()) {
            continue;
        }
        // Follow owl:imports transitively (cycle-guarded by visited_roots).
        for imp in g.objects(&root, OWL_IMPORTS) {
            roots.push(imp);
        }
        for decl in g.objects(&root, &sh("declare")) {
            if !seen_decls.insert(decl.clone()) {
                continue;
            }
            let prefix = match g.object(&decl, &sh("prefix")) {
                Some(Term::Literal(l)) => l.value().to_string(),
                _ => continue,
            };
            let ns = match g.object(&decl, &sh("namespace")) {
                Some(Term::Literal(l)) => l.value().to_string(),
                Some(Term::NamedNode(n)) => n.as_str().to_string(),
                _ => continue,
            };
            out.push_str(&format!("PREFIX {prefix}: <{ns}>\n"));
        }
    }
    out
}

fn iri(s: &str) -> Term {
    Term::NamedNode(oxrdf::NamedNode::new_unchecked(s))
}

/// [OPUS-4.8] (sq-pb0wm) The RDF-1.2 reification predicate: a reifier `?r` reifies
/// a triple-term via `?r rdf:reifies <<( s p o )>>` (the `{| … |}` annotation).
const RDF_REIFIES: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies";

/// [OPUS-4.8] (sq-pb0wm) Cheap presence check: does ANY reifier in the shapes
/// graph reify a triple whose subject is this shape node? `rdf:reifies` objects
/// are triple-terms ([`Term::Triple`]); we scan the (few) `rdf:reifies` triples
/// and test the embedded triple's subject. Lets [`ShapesModel::attach_component_meta`]
/// skip the per-component triple reconstruction for the common un-annotated shape.
fn shape_has_reified_statement(g: &GraphView, node: &Term) -> bool {
    let want: oxrdf::NamedOrBlankNode = match node {
        Term::NamedNode(n) => n.clone().into(),
        Term::BlankNode(b) => b.clone().into(),
        _ => return false,
    };
    g.triples(None, Some(RDF_REIFIES), None)
        .into_iter()
        .any(|[_, _, o]| matches!(&o, Term::Triple(t) if t.subject == want))
}

/// [OPUS-4.8] (sq-pb0wm) The per-statement [`ComponentMeta`] for the constraint
/// triple `(shapeNode, predicate, object)`: scans the reifiers of that exact
/// triple (the subjects of `?r rdf:reifies <<( shapeNode predicate object )>>`)
/// and reads any `sh:deactivated` / `sh:message` / `sh:severity` they carry. The
/// triple-term is reconstructed identically to the eval-side `reifiers_of`, so it
/// matches the term oxttl/oxrdf stores for the `{| … |}` form. A non-IRI/blank
/// shape node, or no reifier, yields the default (no override).
fn reified_meta(g: &GraphView, node: &Term, predicate: &str, object: &Term) -> ComponentMeta {
    let subj: oxrdf::NamedOrBlankNode = match node {
        Term::NamedNode(n) => n.clone().into(),
        Term::BlankNode(b) => b.clone().into(),
        _ => return ComponentMeta::default(),
    };
    let Ok(pred) = oxrdf::NamedNode::new(predicate) else {
        return ComponentMeta::default();
    };
    let triple_term = Term::Triple(Box::new(oxrdf::Triple::new(subj, pred, object.clone())));
    let reifiers = g.subjects(RDF_REIFIES, &triple_term);
    let mut meta = ComponentMeta::default();
    for r in &reifiers {
        if matches!(g.object(r, &sh("deactivated")), Some(Term::Literal(l)) if l.value() == "true") {
            meta.deactivated = true;
        }
        for m in g.objects(r, &sh("message")) {
            meta.messages.push(m);
        }
        if let Some(Term::NamedNode(n)) = g.object(r, &sh("severity")) {
            meta.severity = Some(n.as_str().to_string());
        }
    }
    meta
}

/// [OPUS-4.8] (sq-vg3y) The set of IRIs an object denotes when a constraint
/// parameter accepts "an IRI or a SHACL list of IRIs" (`sh:datatype` /
/// `sh:nodeKind` / `sh:uniqueValuesFor`, SHACL-1.2). A single IRI → singleton; a
/// SHACL-list head → its IRI members (non-IRI members dropped). Anything else →
/// empty (no constraint contributed).
/// [FABLE-5] (sq-11a) True iff `s` is a valid `xsd:integer` lexical form (an
/// optional sign then one or more ASCII digits). Distinguishes a WELL-formed but
/// out-of-`u64`-range count (a silent skip, e.g. a negative `sh:minCount`) from
/// an ill-formed one (recorded for the strict failure channel).
fn is_integer_lexical(s: &str) -> bool {
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

/// [FABLE-5] (sq-11a) True iff `s` is a valid `xsd:boolean` lexical form.
fn is_boolean_lexical(s: &str) -> bool {
    matches!(s, "true" | "false" | "0" | "1")
}

/// [FABLE-5] (sq-ehq4g) True iff `iri` is one of the SIX `sh:nodeKind` values
/// (SHACL §4.6.1): `sh:BlankNode` / `sh:IRI` / `sh:Literal` and the three
/// pairwise unions. Anything else — including another `sh:`-namespace IRI — is
/// an ill-formed `sh:nodeKind` value.
fn is_node_kind_iri(iri: &str) -> bool {
    iri.strip_prefix(SH).is_some_and(|local| {
        matches!(
            local,
            "BlankNode"
                | "IRI"
                | "Literal"
                | "BlankNodeOrIRI"
                | "BlankNodeOrLiteral"
                | "IRIOrLiteral"
        )
    })
}

fn iri_set(g: &GraphView, o: &Term) -> Vec<String> {
    match o {
        Term::NamedNode(n) => vec![n.as_str().to_string()],
        Term::BlankNode(_) => g
            .list(o)
            .into_iter()
            .filter_map(|m| match m {
                Term::NamedNode(n) => Some(n.as_str().to_string()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// The `sh:ignoredProperties` SHACL list of a (closed) shape node, or empty.
fn closed_ignored(g: &GraphView, node: &Term) -> Vec<Term> {
    match g.object(node, &sh("ignoredProperties")) {
        Some(list) => g.list(&list),
        None => Vec::new(),
    }
}

/// [OPUS-4.8] (sq-vg3y) Precompute the `sh:closed sh:ByTypes` `collectProperties`
/// closure (SHACL-1.2 §4.8.1) for every node mentioned in the shapes graph. For a
/// starting node S the closure is the union of the IRI properties reachable via
/// `sh:property/sh:path` from S and, recursively, from S's `rdfs:subClassOf`
/// objects, the shapes that target S via `sh:targetClass`, and the node shapes S
/// references via `sh:node`. The traversal is cycle-guarded (each node expanded at
/// most once per starting node) and reads ONLY the shapes graph (the per-value
/// `rdf:type` step happens at eval time). Nodes whose closure is empty are
/// omitted, so a `get` miss means "no properties".
fn compute_by_types_closures(g: &GraphView) -> FxHashMap<Term, Vec<String>> {
    // Candidate starting nodes: every subject and object in the shapes graph (a
    // data class T is looked up here too, so we must cover object positions).
    let mut nodes: rustc_hash::FxHashSet<Term> = rustc_hash::FxHashSet::default();
    for [s, _, o] in g.triples(None, None, None) {
        if !matches!(s, Term::Literal(_)) {
            nodes.insert(s);
        }
        if !matches!(o, Term::Literal(_)) {
            nodes.insert(o);
        }
    }
    let mut out: FxHashMap<Term, Vec<String>> = FxHashMap::default();
    for start in nodes {
        let mut props: rustc_hash::FxHashSet<String> = rustc_hash::FxHashSet::default();
        let mut visited: rustc_hash::FxHashSet<Term> = rustc_hash::FxHashSet::default();
        let mut stack = vec![start.clone()];
        while let Some(s) = stack.pop() {
            if !visited.insert(s.clone()) {
                continue;
            }
            collect_properties(g, &s, &mut props, &mut stack);
        }
        if !props.is_empty() {
            out.insert(start, props.into_iter().collect());
        }
    }
    out
}

/// One step of the `collectProperties` algorithm (SHACL-1.2 §4.8.1): add the IRI
/// properties reachable from `s` via `sh:property/sh:path`, and push the nodes the
/// recursion continues into (`rdfs:subClassOf` objects, inbound `sh:targetClass`
/// subjects, `sh:node` objects) onto `stack`. Cycle-avoidance is the caller's
/// `visited` set. Reads only the shapes graph.
fn collect_properties(
    g: &GraphView,
    s: &Term,
    props: &mut rustc_hash::FxHashSet<String>,
    stack: &mut Vec<Term>,
) {
    const RDFS_SUBCLASS_OF: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";
    // IRI properties reached via sh:property/sh:path.
    for ps in g.objects(s, &sh("property")) {
        if let Some(Term::NamedNode(n)) = g.object(&ps, &sh("path")) {
            props.insert(n.as_str().to_string());
        }
    }
    // rdfs:subClassOf objects (superclasses).
    for sup in g.objects(s, RDFS_SUBCLASS_OF) {
        stack.push(sup);
    }
    // Shapes that target s via sh:targetClass.
    for sub in g.subjects(&sh("targetClass"), s) {
        stack.push(sub);
    }
    // Node shapes referenced via sh:node.
    for n in g.objects(s, &sh("node")) {
        stack.push(n);
    }
}

/// [OPUS-4.8] SHACL §6.2: discover the `sh:ConstraintComponent` declarations in
/// the shapes graph and compile their parameters + validators. A component is
/// kept only if it has at least one parameter and at least one usable validator
/// (a generic / node / property validator with a parsable `sh:ask`/`sh:select`).
fn discover_components(
    g: &GraphView,
    failures: &mut Vec<PreBindingFailure>,
) -> Vec<ComponentDef> {
    let mut out = Vec::new();
    // SHACL §6.2: a component node is a SHACL instance of sh:ConstraintComponent —
    // i.e. typed sh:ConstraintComponent OR any rdfs:subClassOf-descendant of it
    // (the W3C suite declares `ex:ConstraintComponent rdfs:subClassOf
    // sh:ConstraintComponent` and types components with that subclass).
    for node in g.instances_of(&iri(&sh("ConstraintComponent"))) {
        let parameters = parse_component_parameters(g, &node);
        if parameters.is_empty() {
            continue; // a parameter-less component cannot activate by predicate use
        }
        // Validators parse under the component's own `sh:prefixes` (SHACL §6.3),
        // reusing the `sh:declare`/`owl:imports` chasing of the `sh:sparql` path.
        let prefixes = collect_prefixes_from(g, &g.objects(&node, &sh("prefixes")));
        // [OPUS-4.8] (sq-0mjfd) A validator query pre-binds `$this`, `$value` and
        // each parameter (§6.3); a pre-binding violation in any of the three
        // validator slots is a FAILURE for this component (the ASK
        // `unsupported-sparql-006` re-binds `?value`).
        let mut pre_bound: Vec<&str> = vec!["this", "value"];
        for p in &parameters {
            pre_bound.push(p.var.as_str());
        }
        check_component_validators(g, &node, &prefixes, &pre_bound, failures);
        let validator = parse_validator(g, &node, &sh("validator"), &prefixes);
        let node_validator = parse_validator(g, &node, &sh("nodeValidator"), &prefixes);
        let property_validator = parse_validator(g, &node, &sh("propertyValidator"), &prefixes);
        if validator.is_none() && node_validator.is_none() && property_validator.is_none() {
            continue; // no usable validator — skip (lenient)
        }
        // [SONNET-4.6] (sq-ou3) `sh:labelTemplate` (SHACL §6.1) — display only,
        // never consulted for whether a constraint fires. Collected AFTER the
        // validator check so a component with no usable validator is still
        // skipped above (an unrunnable component has no results to label).
        let label_templates = g.objects(&node, &sh("labelTemplate"));
        out.push(ComponentDef {
            node,
            parameters,
            validator,
            node_validator,
            property_validator,
            label_templates,
        });
    }
    out
}

/// [OPUS-4.8] (sq-5q76d) An explicit `sh:conformanceDisallows` severity set
/// declared in the shapes graph (SHACL 1.2 Core §3.9). Collects the IRI objects
/// of every `sh:conformanceDisallows` triple (a `sh:ValidationReport` may declare
/// several, e.g. `sh:Violation`, `sh:Warning`). `None` when none is declared (the
/// default disallowed set then applies). The W3C
/// `conformance-disallows-001` entry declares it on the EXPECTED report node,
/// which — because that test's shapes graph is the whole file — is visible here.
fn parse_conformance_disallows(g: &GraphView) -> Option<Vec<String>> {
    let mut set: Vec<String> = Vec::new();
    for [_, _, o] in g.triples(None, Some(&sh("conformanceDisallows")), None) {
        if let Term::NamedNode(n) = o {
            let iri = n.as_str().to_string();
            if !set.contains(&iri) {
                set.push(iri);
            }
        }
    }
    if set.is_empty() {
        None
    } else {
        Some(set)
    }
}

/// [OPUS-4.8] (sq-0mjfd) Records a [`PreBindingFailure`] for each of a component's
/// validator slots (`sh:validator` / `sh:nodeValidator` / `sh:propertyValidator`)
/// whose `sh:ask` / `sh:select` query violates the SHACL-SPARQL pre-binding rules.
fn check_component_validators(
    g: &GraphView,
    node: &Term,
    prefixes: &str,
    pre_bound: &[&str],
    failures: &mut Vec<PreBindingFailure>,
) {
    for slot in ["validator", "nodeValidator", "propertyValidator"] {
        let Some(v) = g.object(node, &sh(slot)) else {
            continue;
        };
        let text = match g.object(&v, &sh("ask")) {
            Some(Term::Literal(l)) => l.value().to_string(),
            _ => match g.object(&v, &sh("select")) {
                Some(Term::Literal(l)) => l.value().to_string(),
                _ => continue,
            },
        };
        let full = format!("{prefixes}\n{text}");
        if let Err(violation) = crate::sparql::check_validator_pre_binding(&full, pre_bound) {
            failures.push(PreBindingFailure {
                node: node.clone(),
                message: violation.message(),
            });
        }
    }
}

/// Parses a component's `sh:parameter` list (SHACL §6.2.1). Each parameter node
/// carries `sh:path` (its predicate) and optionally `sh:optional`/`sh:name`. A
/// parameter with no IRI `sh:path` is skipped (a component cannot key on it).
fn parse_component_parameters(g: &GraphView, node: &Term) -> Vec<ComponentParameter> {
    let mut params = Vec::new();
    for p in g.objects(node, &sh("parameter")) {
        let predicate = match g.object(&p, &sh("path")) {
            Some(Term::NamedNode(n)) => n.as_str().to_string(),
            _ => continue,
        };
        // [OPUS-4.8] The pre-bound variable name is the LOCAL NAME of the
        // parameter's `sh:path` IRI (SHACL §6.2.1: "the values of these parameters
        // [...] pre-bound [...] using the local name of the IRI of sh:path"), NOT
        // `sh:name` — which is only a human-readable display label. The W3C
        // `propertyValidator-select-001` test makes this load-bearing: its
        // parameter is `sh:path ex:lang ; sh:name "language"` yet the validator
        // query references `$lang` (the path local name), so binding `$language`
        // would leave `$lang` unbound and the constraint would never fire.
        let var = local_name(&predicate);
        let optional = matches!(
            g.object(&p, &sh("optional")),
            Some(Term::Literal(l)) if l.value() == "true"
        );
        params.push(ComponentParameter {
            predicate,
            var,
            optional,
        });
    }
    params
}

/// The local name of an IRI: the substring after the last `#` or `/`.
fn local_name(iri: &str) -> String {
    iri.rsplit(['#', '/']).next().unwrap_or(iri).to_string()
}

/// Parses one validator (`pred` = `sh:validator` / `sh:nodeValidator` /
/// `sh:propertyValidator`) of a component: its `sh:ask` (ASK validator) or
/// `sh:select` (SELECT validator), with `prefixes` prepended and an optional
/// `sh:message`. Returns `None` if the validator carries neither query or the
/// query is unparsable / of the wrong form (ill-formed → skipped).
fn parse_validator(
    g: &GraphView,
    node: &Term,
    pred: &str,
    prefixes: &str,
) -> Option<PreparedComponentValidator> {
    let v = g.object(node, pred)?;
    // sh:ask takes precedence; fall back to sh:select.
    let (text, is_ask) = match g.object(&v, &sh("ask")) {
        Some(Term::Literal(l)) => (l.value().to_string(), true),
        _ => match g.object(&v, &sh("select")) {
            Some(Term::Literal(l)) => (l.value().to_string(), false),
            _ => return None,
        },
    };
    let full = format!("{prefixes}\n{text}");
    let prepared = crate::sparql::PreparedValidator::build(&full, is_ask)?;
    let message = match g.object(&v, &sh("message")) {
        Some(Term::Literal(l)) => Some(l.value().to_string()),
        _ => None,
    };
    Some(PreparedComponentValidator {
        prepared,
        message,
        raw: full,
        is_ask,
    })
}

/// Substitutes the `$PATH` / `?PATH` query variable (a SHACL property-shape
/// SPARQL pre-binding) with the SPARQL property-path expression `pp`, replacing
/// only WHOLE variable tokens (so `$PATHWAY` is left alone). SHACL §5.2.1.
fn substitute_path_var(select: &str, pp: &str) -> String {
    let mut out = String::with_capacity(select.len());
    let mut rest = select;
    while let Some(pos) = rest.find(['$', '?']) {
        out.push_str(&rest[..pos]);
        let tail = &rest[pos..]; // starts with $ or ?
        let body = &tail[1..];
        // Whole-token match of "PATH" delimited by a non-identifier char.
        let is_path_var = body.strip_prefix("PATH").is_some_and(|after| {
            after
                .chars()
                .next()
                .map(|ch| !(ch.is_ascii_alphanumeric() || ch == '_'))
                .unwrap_or(true)
        });
        if is_path_var {
            out.push_str(pp);
            rest = &body[4..]; // past "PATH"
        } else {
            // Not the PATH var: keep the sigil and continue past it.
            out.push_str(&tail[..1]);
            rest = body;
        }
    }
    out.push_str(rest);
    out
}
