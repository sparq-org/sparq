// [OPUS-4.8] sq-jxl0: single-source the crate overview from README.md so crates.io
// (package.readme) and the docs.rs front page render identical content. The README's
// `## Usage` rust fences are compiled as doctests (hidden `#`-scaffolding inside them).
#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)] // [OPUS-4.8] sq-emay: crate has zero `unsafe`

mod eval;
// [FABLE-5] (sq-uz0) sh:shapesGraph / owl:imports shapes-graph ASSEMBLY
// (W3C SHACL §§3.1/3.3 (shapes-graph)) — OPT-IN behind the `imports` cargo feature so the base
// validation path carries zero assembly code when off. Zero new dependencies:
// a pure traversal over `GraphView` + `Dict` the crate already uses.
#[cfg(feature = "imports")]
pub mod imports;
pub mod model;
pub mod path;
mod report;
// [OPUS-4.8] (sq-d1dw) SHACL-AF rules (`sh:rule`) — OPT-IN behind the `shacl-af`
// cargo feature so the base validation path carries zero rule code when off.
#[cfg(feature = "shacl-af")]
pub mod rules;
// [OPUS-4.8] (sq-v0b8, #796) SHACL Compact Syntax (SCS) PARSER (text → shapes
// triples) — OPT-IN behind the `scs` cargo feature so the base validation path
// (and the default + wasm bundles) carry zero parser code when off.
#[cfg(feature = "scs")]
pub mod scs;
mod sparql;
pub mod view;

// [FABLE-5] (sq-11a) `IllFormedConstruct` records a shapes-graph construct that
// violates the SHACL syntax rules; `validate_strict` reports the shapes graph as
// a failure (the test-suite `sht:Failure` outcome) when any were found.
pub use model::{Component, IllFormedConstruct, PreBindingFailure, Shape, ShapesModel, Target};
pub use path::Path;
// [OPUS-4.8] (sq-lz99x) `ShapeDiagnostic` surfaces a constraint the validator
// SKIPPED because it could not be evaluated (e.g. an uncompilable `sh:pattern`).
// [OPUS-4.8] (sq-sx15d) `DEFAULT_CONFORMANCE_DISALLOWS` is the SHACL 1.2 default
// disallowed-severity set used to compute `ValidationReport::conforms`.
pub use report::{
    ShapeDiagnostic, ValidationReport, ValidationResult, DEFAULT_CONFORMANCE_DISALLOWS,
};

// [OPUS-4.8] (sq-v0b8) SHACL Compact Syntax parser public surface (feature `scs`).
#[cfg(feature = "scs")]
pub use scs::{parse as parse_scs, parse_to_graph as parse_scs_to_graph, ScsError, DEFAULT_BASE};

// [FABLE-5] (sq-uz0) Shapes-graph assembly public surface (feature `imports`).
#[cfg(feature = "imports")]
pub use imports::{resolve_imports, resolve_shapes_graph, ShapesGraphResolution};

// [OPUS-4.8] SHACL-AF rules + node-expression public surface (feature `shacl-af`).
#[cfg(feature = "shacl-af")]
pub use rules::{
    apply_rules, apply_rules_in_scope, apply_rules_in_scope_with_model, apply_rules_with_model,
    conforms, eval_node_expression, eval_node_expression_with_scope, expand, expand_dataset,
    ConformanceCheck, DatasetExpansion, Destination, GraphScope, Inference, Scope,
};

use oxrdf::Triple;
use sparq_core::Graph;

/// [GPT-5.6] (sq-lsp7k.2.1) Selects which facts validation can observe.
///
/// This surface is available only with the `shacl-af` feature because
/// [`AssertedPlusInferred`](Self::AssertedPlusInferred) computes the SHACL-AF
/// rule closure before validating.
#[cfg(feature = "shacl-af")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactDomain {
    /// Validate the input data graph without applying SHACL-AF rules.
    Asserted,
    /// Validate a fresh graph containing the input data and its SHACL-AF rule
    /// closure.
    AssertedPlusInferred,
}

/// Validates `data` against the SHACL shapes in `shapes`, returning the
/// validation report. Constructs the shapes graph never declares (or declares
/// ill-formed — e.g. an unparsable path) are skipped rather than failing; use
/// [`validate_strict`] to report ill-formed constructs as a failure instead
/// (the W3C test-suite `sht:Failure` outcome). [FABLE-5] (sq-11a)
pub fn validate(data: &Graph, shapes: &Graph) -> ValidationReport {
    let model = ShapesModel::parse(shapes);
    validate_with_model(data, &model)
}

/// [`validate`] against an already-parsed shapes model (amortises shape
/// parsing across many data graphs).
///
/// [OPUS-4.8] (sq-5q76d) `ValidationReport::conforms` honours a shapes-graph
/// `sh:conformanceDisallows` declaration ([`ShapesModel::conformance_disallows`])
/// when present, falling back to the default {Violation, Warning, Info} set.
pub fn validate_with_model(data: &Graph, model: &ShapesModel) -> ValidationReport {
    let (results, mut diagnostics) = eval::validate_graph(data, model);
    // [FABLE-5] (sq-c1v3e) Surface the parse-time ill-formed-construct records
    // ([`ShapesModel::ill_formed`]) on the LENIENT path too, as report
    // diagnostics — visibility without failing: each construct is still skipped
    // and `conforms` is unaffected (diagnostics never contribute results). The
    // strict channel keeps turning the same records into `Err(ShaclFailure)`.
    diagnostics.extend(model.ill_formed().iter().map(|i| ShapeDiagnostic {
        source_shape: i.node.clone(),
        source_component: i.predicate.clone(),
        message: i.message.clone(),
    }));
    ValidationReport::with_diagnostics_and_disallows(
        results,
        diagnostics,
        model.conformance_disallows(),
    )
}

/// [GPT-5.6] (sq-lsp7k.2.1) Validates over the selected [`FactDomain`].
///
/// [`FactDomain::Asserted`] is identical to [`validate`].
/// [`FactDomain::AssertedPlusInferred`] first computes the SHACL-AF rule
/// fixpoint, then validates `data ∪ inferred`. The input graph is not mutated.
#[cfg(feature = "shacl-af")]
pub fn validate_with_domain(data: &Graph, shapes: &Graph, domain: FactDomain) -> ValidationReport {
    let model = ShapesModel::parse(shapes);
    validate_with_domain_and_model(data, shapes, &model, domain)
}

/// [GPT-5.6] (sq-lsp7k.2.1) [`validate_with_domain`] against an already-parsed
/// shapes model, amortising shape parsing across data graphs.
///
/// `shapes` remains required for the inferred domain because SHACL-AF rule node
/// expressions and CONSTRUCT prefixes are read directly from the shapes graph.
#[cfg(feature = "shacl-af")]
pub fn validate_with_domain_and_model(
    data: &Graph,
    shapes: &Graph,
    model: &ShapesModel,
    domain: FactDomain,
) -> ValidationReport {
    match domain {
        FactDomain::Asserted => validate_with_model(data, model),
        FactDomain::AssertedPlusInferred => {
            let inference = rules::apply_rules_with_model(data, shapes, model);
            let expanded = rules::expand_graph(data, &inference.triples);
            validate_with_model(&expanded, model)
        }
    }
}

/// [OPUS-4.8] (sq-0mjfd) A SHACL processing **failure** (W3C SHACL §3.4): the
/// shapes graph declares something a conformant processor cannot soundly
/// evaluate, so the spec says it signals a *failure* rather than produce a
/// validation report. Two producers: a SHACL-SPARQL pre-binding violation (a
/// `sh:sparql` constraint or constraint-component validator using `MINUS` /
/// `VALUES` / `SERVICE` / a sub-`SELECT` that drops a pre-bound variable / a
/// `BIND` that re-binds one), and [FABLE-5] (sq-11a) an **ill-formed
/// shapes-graph construct** (a violated SHACL syntax rule — an unparsable
/// `sh:path`, a non-integer `sh:minCount`, a malformed SHACL list, …). Carries
/// the offending nodes + the violated rules for diagnostics.
#[derive(Debug, Clone)]
pub struct ShaclFailure {
    /// The pre-binding failures found (may be empty when `ill_formed` is not).
    pub pre_binding: Vec<model::PreBindingFailure>,
    /// [FABLE-5] (sq-11a) The ill-formed shapes-graph constructs found (may be
    /// empty when `pre_binding` is not). At least one of the two vecs is non-empty.
    pub ill_formed: Vec<model::IllFormedConstruct>,
}

impl std::fmt::Display for ShaclFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // [OPUS-4.8] positional format args (rust/unused-variable CodeQL FP).
        write!(
            f,
            "SHACL failure: {} unsound SHACL-SPARQL pre-binding(s), {} ill-formed shapes-graph construct(s)",
            self.pre_binding.len(),
            self.ill_formed.len()
        )?;
        if let Some(first) = self.pre_binding.first() {
            write!(f, " — e.g. {}: {}", first.node, first.message)?;
        } else if let Some(first) = self.ill_formed.first() {
            write!(
                f,
                " — e.g. {} at {}: {}",
                first.predicate, first.node, first.message
            )?;
        }
        Ok(())
    }
}

impl std::error::Error for ShaclFailure {}

/// [OPUS-4.8] (sq-0mjfd) STRICT validation (W3C SHACL §3.4 failure outcome): like
/// [`validate`], but returns `Err(ShaclFailure)` when the shapes graph declares
/// something a conformant processor rejects — a SHACL-SPARQL pre-binding
/// violation, or [FABLE-5] (sq-11a) an ill-formed shapes-graph construct
/// ([`ShapesModel::ill_formed`]). The lenient [`validate`] instead skips such a
/// construct (its lenient ill-formed-shape policy), so use this when the
/// distinction matters (e.g. the W3C `mf:result sht:Failure` entries).
pub fn validate_strict(data: &Graph, shapes: &Graph) -> Result<ValidationReport, ShaclFailure> {
    let model = ShapesModel::parse(shapes);
    validate_strict_with_model(data, &model)
}

/// [OPUS-4.8] (sq-0mjfd) [`validate_strict`] against an already-parsed model: an
/// `Err` if the model recorded any pre-binding failure
/// ([`ShapesModel::pre_binding_failures`]) or [FABLE-5] (sq-11a) any ill-formed
/// shapes-graph construct ([`ShapesModel::ill_formed`]), else the lenient
/// validation report.
pub fn validate_strict_with_model(
    data: &Graph,
    model: &ShapesModel,
) -> Result<ValidationReport, ShaclFailure> {
    let pre_binding = model.pre_binding_failures();
    let ill_formed = model.ill_formed();
    if !pre_binding.is_empty() || !ill_formed.is_empty() {
        return Err(ShaclFailure {
            pre_binding: pre_binding.to_vec(),
            ill_formed: ill_formed.to_vec(),
        });
    }
    Ok(validate_with_model(data, model))
}

/// Builds a queryable [`Graph`] from already-parsed triples. The seam for
/// callers that need parser options [`Graph::load_str`] does not expose —
/// e.g. a base IRI for resolving relative IRIs (the W3C test-suite files).
pub fn graph_from_triples<I: IntoIterator<Item = Triple>>(triples: I) -> Graph {
    use oxrdf::{NamedOrBlankNode, Term};
    let mut dict = sparq_core::dict::Dict::new();
    let ids: Vec<[sparq_core::dict::Id; 3]> = triples
        .into_iter()
        .map(|t| {
            let s = match t.subject {
                NamedOrBlankNode::NamedNode(n) => Term::NamedNode(n),
                NamedOrBlankNode::BlankNode(b) => Term::BlankNode(b),
            };
            [
                dict.intern(&s),
                dict.intern(&Term::NamedNode(t.predicate)),
                dict.intern(&t.object),
            ]
        })
        .collect();
    Graph::from_parts(dict, ids)
}

/// [OPUS-4.8] (sq-7iai) The number of DISTINCT focus nodes the model's targeted
/// shapes select over `data` — i.e. the size of the union of every targeted
/// shape's focus-node set, deduplicated exactly as the validator's own
/// `focus_nodes` enumeration does. This is a deterministic *target-selection*
/// statistic (independent of how many constraints those nodes violate), used by
/// the SHACL benchmark suite as a target-selection regression detector alongside
/// the violation count.
///
/// It delegates to the validator's own focus-node enumeration (`eval`), so it
/// stays in lock-step with validation — including the SHACL-1.2 data-dependent
/// targets (`sh:targetWhere`, the SPARQL-valued `sh:targetNode`, and the
/// `sh:shape` data-graph link) that a pure shapes-graph scan cannot express.
pub fn count_focus_nodes(data: &Graph, model: &ShapesModel) -> usize {
    eval::count_focus_nodes(data, model)
}

/// [OPUS-4.8] (sq-7d3dj.33.1) The calling thread's monotonically-increasing count of
/// `sh:sparql` constraint query executions the validator has run (one per engine
/// query). Thread-local so a `validate` on one thread is not perturbed by a
/// concurrent `validate` on another; snapshot and read the delta on the SAME thread.
///
/// Its purpose is a perf / anti-vacuity check: with focus-node batching the delta
/// across one [`validate`] call is ~O(number of `sh:sparql` shapes) — one query per
/// ~10 000-focus chunk — instead of O(number of focus nodes). A test / benchmark can
/// snapshot it before and after [`validate`] and assert the batched path fired (a
/// small delta for a large focus set), guarding against a silent regression back to
/// the per-focus loop. The absolute value is not meaningful — only the delta.
///
/// ```
/// # use sparq_shacl::{sparql_constraint_executions, validate, graph_from_triples};
/// let before = sparql_constraint_executions();
/// // ... run validate() over a shapes graph with sh:sparql constraints ...
/// let executions = sparql_constraint_executions() - before;
/// # let _ = executions;
/// ```
#[must_use]
pub fn sparql_constraint_executions() -> u64 {
    sparql::exec_count()
}

/// Loads a Turtle document into a [`Graph`], resolving relative IRIs against
/// `base`.
pub fn load_turtle_with_base(text: &str, base: &str) -> Result<Graph, String> {
    let parser = oxttl::TurtleParser::new()
        .with_base_iri(base)
        .map_err(|e| e.to_string())?;
    let mut triples = Vec::new();
    for t in parser.for_slice(text.as_bytes()) {
        triples.push(t.map_err(|e| e.to_string())?);
    }
    Ok(graph_from_triples(triples))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHAPES: &str = r#"
        @prefix sh: <http://www.w3.org/ns/shacl#> .
        @prefix ex: <http://example.org/> .
        @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
        ex:PersonShape a sh:NodeShape ;
          sh:targetClass ex:Person ;
          sh:property [
            sh:path ex:age ;
            sh:datatype xsd:integer ;
            sh:maxCount 1 ;
            sh:minInclusive 0 ;
          ] ;
          sh:property [
            sh:path ex:name ;
            sh:minCount 1 ;
            sh:nodeKind sh:Literal ;
          ] .
    "#;

    fn check(data: &str) -> ValidationReport {
        let data = Graph::load_str(data, "turtle").unwrap();
        let shapes = Graph::load_str(SHAPES, "turtle").unwrap();
        validate(&data, &shapes)
    }

    #[test]
    fn conforming_data() {
        let r = check(
            r#"
            @prefix ex: <http://example.org/> .
            ex:alice a ex:Person ; ex:age 30 ; ex:name "Alice" .
        "#,
        );
        assert!(r.conforms, "unexpected results: {}", r.to_text());
        assert!(r.to_turtle().contains("sh:conforms true"));
    }

    /// [OPUS-4.8] (sq-5q76d) A shapes-graph `sh:conformanceDisallows` (here only
    /// `sh:Violation`) OVERRIDES the default {Violation,Warning,Info} set: a
    /// Warning-only result then conforms (mirrors conformance-disallows-001).
    #[test]
    fn shapes_graph_conformance_disallows_threaded() {
        let g = r#"
            @prefix sh: <http://www.w3.org/ns/shacl#> .
            @prefix ex: <http://example.org/> .
            @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
            ex:S a sh:NodeShape ; sh:targetNode ex:bob ;
              sh:property [ sh:path ex:age ; sh:severity sh:Warning ; sh:datatype xsd:integer ] .
            ex:bob ex:age "x" .
            [] a sh:ValidationReport ; sh:conformanceDisallows sh:Violation .
        "#;
        let graph = Graph::load_str(g, "turtle").unwrap();
        let r = validate(&graph, &graph);
        assert_eq!(r.results.len(), 1, "one Warning result expected");
        // Only sh:Violation is disallowed, so the Warning result still conforms.
        assert!(
            r.conforms,
            "Warning result must conform under the custom set"
        );
        // Without the override (default set) the same Warning would NOT conform.
        let g2 = g.replace("sh:conformanceDisallows sh:Violation", "");
        let graph2 = Graph::load_str(&g2, "turtle").unwrap();
        assert!(!validate(&graph2, &graph2).conforms);
    }

    /// [OPUS-4.8] (sq-0mjfd) `validate_strict` REJECTS a shapes graph whose
    /// `sh:sparql` constraint violates the pre-binding rules (here a MINUS), while
    /// the lenient `validate` skips it and returns a (conforming) report.
    #[test]
    fn validate_strict_rejects_pre_binding_violation() {
        let g = r#"
            @prefix sh: <http://www.w3.org/ns/shacl#> .
            @prefix ex: <http://example.org/> .
            ex:S a sh:NodeShape ; sh:targetNode ex:x ;
              sh:sparql [ a sh:SPARQLConstraint ;
                sh:select "SELECT $this WHERE { $this ?p ?o . MINUS { $this ?p \"v\" } }" ] .
        "#;
        let graph = Graph::load_str(g, "turtle").unwrap();
        assert!(
            validate_strict(&graph, &graph).is_err(),
            "strict validation must reject a MINUS pre-binding"
        );
        // Lenient validate skips the constraint (no panic, no failure).
        assert!(validate(&graph, &graph).conforms);
    }

    /// [OPUS-4.8] (sq-0mjfd) `sh:reifierShape`: a value whose reifier fails the
    /// reifier shape produces a `sh:ReifierShapeConstraintComponent` result.
    #[test]
    fn reifier_shape_validates_reifiers() {
        let g = r#"
            @prefix sh: <http://www.w3.org/ns/shacl#> .
            @prefix ex: <http://example.org/> .
            ex:r ex:p "v" {| ex:q false |} .
            ex:Reify a sh:NodeShape ; sh:property [ sh:path ex:q ; sh:in ( true ) ] .
            ex:S a sh:NodeShape ; sh:targetNode ex:r ;
              sh:property [ sh:path ex:p ; sh:reifierShape ex:Reify ] .
        "#;
        let graph = Graph::load_str(g, "turtle").unwrap();
        let r = validate(&graph, &graph);
        assert!(!r.conforms, "reifier (q=false) must fail q in (true)");
        assert!(r.results.iter().any(|res| res
            .source_component
            .ends_with("ReifierShapeConstraintComponent")));
    }

    /// [OPUS-4.8] (sq-0mjfd) `sh:uniqueLang` distinguishes base direction: two
    /// `@ar--ltr` values collide, but `@ar`, `@ar--ltr`, `@ar--rtl` are distinct.
    #[test]
    fn unique_lang_keys_on_base_direction() {
        let shapes = r#"
            @prefix sh: <http://www.w3.org/ns/shacl#> .
            @prefix ex: <http://example.org/> .
            ex:S a sh:NodeShape ; sh:targetNode ex:bad , ex:good ;
              sh:property [ sh:path ex:p ; sh:uniqueLang true ] .
        "#;
        let bad = r#"@prefix ex: <http://example.org/> .
            ex:bad ex:p "A"@ar--ltr , "B"@ar--ltr ."#;
        let good = r#"@prefix ex: <http://example.org/> .
            ex:good ex:p "A"@ar , "A"@ar--ltr , "A"@ar--rtl ."#;
        let sg = Graph::load_str(shapes, "turtle").unwrap();
        let bad_r = validate(&Graph::load_str(bad, "turtle").unwrap(), &sg);
        assert!(!bad_r.conforms, "two @ar--ltr values must collide");
        let good_r = validate(&Graph::load_str(good, "turtle").unwrap(), &sg);
        assert!(
            good_r.conforms,
            "@ar / @ar--ltr / @ar--rtl are distinct keys: {}",
            good_r.to_text()
        );
    }

    #[test]
    fn violations_reported() {
        let r = check(
            r#"
            @prefix ex: <http://example.org/> .
            ex:bob a ex:Person ; ex:age -1 .
        "#,
        );
        assert!(!r.conforms);
        // missing name (minCount) + negative age (minInclusive)
        assert_eq!(r.results.len(), 2, "{}", r.to_text());
        let comps: Vec<&str> = r
            .results
            .iter()
            .map(|x| x.source_component.as_str())
            .collect();
        assert!(comps
            .iter()
            .any(|c| c.ends_with("MinCountConstraintComponent")));
        assert!(comps
            .iter()
            .any(|c| c.ends_with("MinInclusiveConstraintComponent")));
    }

    /// [OPUS-4.8] (sq-7iai) `count_focus_nodes` counts the DISTINCT focus nodes the
    /// targeted shapes select (independent of how many constraints they violate) —
    /// the SHACL benchmark suite's target-selection gate.
    #[test]
    fn focus_node_count_is_distinct_target_union() {
        let data = Graph::load_str(
            r#"
            @prefix ex: <http://example.org/> .
            ex:alice a ex:Person ; ex:age 30 ; ex:name "Alice" .
            ex:bob   a ex:Person ; ex:age 40 ; ex:name "Bob" .
            ex:carol a ex:Person .
            ex:notaperson a ex:Robot .
        "#,
            "turtle",
        )
        .unwrap();
        let shapes = Graph::load_str(SHAPES, "turtle").unwrap();
        let model = ShapesModel::parse(&shapes);
        // Three ex:Person instances are the focus nodes; ex:Robot is not targeted.
        assert_eq!(count_focus_nodes(&data, &model), 3);
        // A graph with no instances of the targeted class selects zero focus nodes.
        let empty = Graph::load_str(
            "@prefix ex: <http://example.org/> . ex:x a ex:Other .",
            "turtle",
        )
        .unwrap();
        assert_eq!(count_focus_nodes(&empty, &model), 0);
    }

    /// [OPUS-4.8] (sq-bif) `count_focus_nodes` unions EVERY target kind — node,
    /// subjects-of, objects-of — deduplicated. A node selected by two distinct
    /// targets is counted once.
    #[test]
    fn focus_node_count_unions_all_target_kinds() {
        let data = Graph::load_str(
            r#"
            @prefix ex: <http://example.org/> .
            ex:alice ex:knows ex:bob .
            ex:carol ex:knows ex:alice .
        "#,
            "turtle",
        )
        .unwrap();
        // sh:targetNode ex:alice, sh:targetSubjectsOf ex:knows (= {alice, carol}),
        // sh:targetObjectsOf ex:knows (= {bob, alice}). The deduplicated union is
        // {alice, bob, carol} = 3 (alice appears via all three but counts once).
        let shapes = Graph::load_str(
            r#"
            @prefix sh: <http://www.w3.org/ns/shacl#> .
            @prefix ex: <http://example.org/> .
            ex:S a sh:NodeShape ;
              sh:targetNode ex:alice ;
              sh:targetSubjectsOf ex:knows ;
              sh:targetObjectsOf ex:knows ;
              sh:property [ sh:path ex:name ; sh:minCount 1 ] .
        "#,
            "turtle",
        )
        .unwrap();
        let model = ShapesModel::parse(&shapes);
        assert_eq!(count_focus_nodes(&data, &model), 3);
    }

    // --- [OPUS-4.8] (sq-rnkdh) SHACL 1.2 targets & SPARQL node expressions ------

    /// `sh:targetWhere [ <inline shape> ]`: the focus nodes are the data-graph
    /// nodes that CONFORM to the inline shape. Here only `ex:alice` (a Person with
    /// age ≥ 18) is in scope; `ex:bob` (age 17) is not, so its violation of the
    /// outer constraint is NOT reported. `count_focus_nodes` agrees.
    #[test]
    fn target_where_selects_conforming_nodes() {
        const TW_SHAPES: &str = r#"
            @prefix sh: <http://www.w3.org/ns/shacl#> .
            @prefix ex: <http://example.org/> .
            ex:AdultShape a sh:NodeShape ;
              sh:targetWhere [
                sh:class ex:Person ;
                sh:property [ sh:path ex:age ; sh:minInclusive 18 ] ;
              ] ;
              sh:property [ sh:path ex:votedFor ; sh:minCount 1 ] .
        "#;
        let shapes = Graph::load_str(TW_SHAPES, "turtle").unwrap();
        let data = Graph::load_str(
            r#"
            @prefix ex: <http://example.org/> .
            ex:alice a ex:Person ; ex:age 30 .
            ex:bob   a ex:Person ; ex:age 17 .
        "#,
            "turtle",
        )
        .unwrap();
        let model = ShapesModel::parse(&shapes);
        // alice is the only in-target node (≥18); bob (17) is out of target.
        assert_eq!(count_focus_nodes(&data, &model), 1);
        let r = validate(&data, &shapes);
        assert!(!r.conforms, "{}", r.to_text());
        assert_eq!(r.results.len(), 1, "{}", r.to_text());
        assert_eq!(
            r.results[0].focus_node,
            oxrdf::Term::NamedNode(oxrdf::NamedNode::new("http://example.org/alice").unwrap())
        );
    }

    /// `sh:shape` data-graph link: a triple `?n sh:shape ?S` in the DATA graph
    /// makes `?n` a focus node of shape `?S`.
    #[test]
    fn data_graph_sh_shape_target() {
        let shapes = Graph::load_str(
            r#"
            @prefix sh: <http://www.w3.org/ns/shacl#> .
            @prefix ex: <http://example.org/> .
            @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
            @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
            ex:TestShape a sh:NodeShape ;
              sh:property [ sh:path rdfs:label ; sh:datatype xsd:string ; sh:maxCount 0 ] .
        "#,
            "turtle",
        )
        .unwrap();
        let data = Graph::load_str(
            r#"
            @prefix ex: <http://example.org/> .
            @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
            ex:Invalid rdfs:label "x" ; <http://www.w3.org/ns/shacl#shape> ex:TestShape .
            ex:Valid <http://www.w3.org/ns/shacl#shape> ex:TestShape .
        "#,
            "turtle",
        )
        .unwrap();
        let r = validate(&data, &shapes);
        assert!(!r.conforms, "{}", r.to_text());
        // Only ex:Invalid has a label (maxCount 0 violated); ex:Valid conforms.
        assert_eq!(r.results.len(), 1, "{}", r.to_text());
        assert_eq!(
            r.results[0].focus_node,
            oxrdf::Term::NamedNode(oxrdf::NamedNode::new("http://example.org/Invalid").unwrap())
        );
    }

    /// `sh:ShapeClass` is a class that is ALSO a node shape (SHACL 1.2): instances
    /// (via the subclass closure) are implicit-class-targeted by it.
    #[test]
    fn shape_class_is_implicit_class_target() {
        let g = Graph::load_str(
            r#"
            @prefix sh: <http://www.w3.org/ns/shacl#> .
            @prefix ex: <http://example.org/> .
            @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
            ex:Super a sh:ShapeClass ; sh:in ( ex:Good ) .
            ex:Sub rdfs:subClassOf ex:Super .
            ex:Good a ex:Sub .
            ex:Bad  a ex:Sub .
        "#,
            "turtle",
        )
        .unwrap();
        let r = validate(&g, &g);
        assert!(!r.conforms, "{}", r.to_text());
        // ex:Bad is not in the sh:in list -> one InConstraintComponent violation.
        assert_eq!(r.results.len(), 1, "{}", r.to_text());
        assert!(r.results[0]
            .source_component
            .ends_with("InConstraintComponent"));
    }

    /// SPARQL-valued `sh:targetNode [ sh:select … ]` computes target nodes, and
    /// `sh:values [ sh:sparqlExpr … ]` computes value nodes; a constraint-level
    /// `sh:severity` overrides the shape default.
    #[test]
    fn sparql_valued_target_and_values_and_severity() {
        let shapes = Graph::load_str(
            r#"
            @prefix sh: <http://www.w3.org/ns/shacl#> .
            @prefix ex: <http://example.org/> .
            @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
            ex:S a sh:NodeShape ;
              sh:targetNode [ sh:select "SELECT ?p WHERE { ?p <http://example.org/flag> true }" ] ;
              sh:sparql [
                sh:select "SELECT $this WHERE { $this <http://example.org/bad> true }" ;
                sh:severity sh:Warning ;
              ] .
        "#,
            "turtle",
        )
        .unwrap();
        let data = Graph::load_str(
            r#"
            @prefix ex: <http://example.org/> .
            ex:x ex:flag true ; ex:bad true .
            ex:y ex:bad true .
        "#,
            "turtle",
        )
        .unwrap();
        let model = ShapesModel::parse(&shapes);
        // Only ex:x is selected by the SPARQL target (ex:y has no ex:flag).
        assert_eq!(count_focus_nodes(&data, &model), 1);
        let r = validate(&data, &shapes);
        assert!(!r.conforms, "{}", r.to_text());
        assert_eq!(r.results.len(), 1, "{}", r.to_text());
        // The constraint-level sh:severity overrode the shape's default Violation.
        assert!(
            r.results[0].severity.ends_with("Warning"),
            "{}",
            r.to_text()
        );
    }

    /// The Turtle rendering must itself be valid Turtle.
    #[test]
    fn report_turtle_parses() {
        let r = check(
            r#"
            @prefix ex: <http://example.org/> .
            ex:bob a ex:Person ; ex:age 200 ; ex:age 300 .
        "#,
        );
        assert!(!r.conforms);
        let ttl = r.to_turtle();
        let parsed: Result<Vec<_>, _> = oxttl::TurtleParser::new()
            .for_slice(ttl.as_bytes())
            .collect();
        let triples = parsed.unwrap_or_else(|e| panic!("report Turtle does not parse: {e}\n{ttl}"));
        assert!(triples
            .iter()
            .any(|t| t.predicate.as_str().ends_with("conforms")));
    }

    /// [OPUS-4.8] (sq-sx15d) `sh:conforms` fails when ANY result is in the SHACL-1.2
    /// default disallowed set {Violation, Warning, Info}; `conforms_violations_only`
    /// is the strictly-weaker CI toggle that fails only on `sh:Violation`. A
    /// Warning result fails `conforms` (Warning is disallowed by default) but
    /// passes the violations-only toggle.
    #[test]
    fn severity_aware_conformance_toggle() {
        let data = Graph::load_str(
            r#"
            @prefix ex: <http://example.org/> .
            ex:bob a ex:Person ; ex:age "old" .
        "#,
            "turtle",
        )
        .unwrap();
        let shapes = |severity: &str| {
            Graph::load_str(
                &format!(
                    r#"
            @prefix sh: <http://www.w3.org/ns/shacl#> .
            @prefix ex: <http://example.org/> .
            @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
            ex:S a sh:NodeShape ;
              sh:targetClass ex:Person ;
              sh:property [ sh:path ex:age ; sh:datatype xsd:integer ; sh:severity {severity} ] .
        "#
                ),
                "turtle",
            )
            .unwrap()
        };
        // A warning-severity result: reported, sh:conforms false, but the
        // violations-only toggle passes.
        let r = validate(&data, &shapes("sh:Warning"));
        assert!(!r.conforms);
        assert_eq!(r.results.len(), 1);
        assert!(r.conforms_violations_only());
        assert_eq!(
            r.results_with_severity("http://www.w3.org/ns/shacl#Warning")
                .count(),
            1
        );
        assert_eq!(
            r.results_with_severity("http://www.w3.org/ns/shacl#Violation")
                .count(),
            0
        );
        // Default severity is sh:Violation: both notions of conformance fail.
        let r = validate(&data, &shapes("sh:Violation"));
        assert!(!r.conforms);
        assert!(!r.conforms_violations_only());
        // A sh:Debug result is below the default threshold: conforms is true even
        // though a result is reported (the violations-only toggle agrees).
        let r = validate(&data, &shapes("sh:Debug"));
        assert!(
            r.conforms,
            "Debug result must not break default conformance"
        );
        assert_eq!(r.results.len(), 1);
        assert!(r.conforms_violations_only());
        // Conforming data conforms under both.
        let ok = Graph::load_str(
            "@prefix ex: <http://example.org/> . ex:a a ex:Person ; ex:age 3 .",
            "turtle",
        )
        .unwrap();
        let r = validate(&ok, &shapes("sh:Warning"));
        assert!(r.conforms && r.conforms_violations_only());
    }

    /// Cyclic sh:node references stay correct under the (focus, shape)
    /// conformance memo: re-entry counts as conforming, and results reached
    /// through a cycle are not wrongly reused.
    #[test]
    fn cyclic_node_shapes_with_memo() {
        let shapes = Graph::load_str(
            r#"
            @prefix sh: <http://www.w3.org/ns/shacl#> .
            @prefix ex: <http://example.org/> .
            ex:PS a sh:NodeShape ;
              sh:targetClass ex:Person ;
              sh:property [ sh:path ex:knows ; sh:node ex:PS ] ;
              sh:property [ sh:path ex:name ; sh:minCount 1 ] .
        "#,
            "turtle",
        )
        .unwrap();
        // A mutual cycle where everyone has a name: conforms (re-entry = true).
        let good = Graph::load_str(
            r#"
            @prefix ex: <http://example.org/> .
            ex:a a ex:Person ; ex:name "A" ; ex:knows ex:b .
            ex:b a ex:Person ; ex:name "B" ; ex:knows ex:a .
        "#,
            "turtle",
        )
        .unwrap();
        let r = validate(&good, &shapes);
        assert!(r.conforms, "{}", r.to_text());
        // The same cycle but b is nameless: a's sh:node value fails, b's own
        // minCount fails, and the SHARED conformance check of the nameless
        // node (reached from both a and c) reports per route.
        let bad = Graph::load_str(
            r#"
            @prefix ex: <http://example.org/> .
            ex:a a ex:Person ; ex:name "A" ; ex:knows ex:b .
            ex:b a ex:Person ; ex:knows ex:a .
            ex:c a ex:Person ; ex:name "C" ; ex:knows ex:b .
        "#,
            "turtle",
        )
        .unwrap();
        let r = validate(&bad, &shapes);
        assert!(!r.conforms);
        // b fails minCount as a target, and the nameless b makes THREE sh:node
        // checks fail: a's value b, c's value b, and b's value a (a does not
        // conform because ITS value b fails — the cycle propagates).
        let node_failures: Vec<_> = r
            .results
            .iter()
            .filter(|x| x.source_component.ends_with("NodeConstraintComponent"))
            .collect();
        assert_eq!(node_failures.len(), 3, "{}", r.to_text());
        assert!(r
            .results
            .iter()
            .any(|x| x.source_component.ends_with("MinCountConstraintComponent")));
    }

    // [OPUS-4.8] Regression for review 1616: a cyclic sh:property reference must NOT overflow the
    // stack. The Property component recurses through validate_shape directly, bypassing the
    // conforms() (focus, shape) guard; without an equivalent guard a self-referential property
    // shape over cyclic data recurses forever. Re-entry counts as conforming (SHACL leaves
    // recursion undefined), so the data conforms.
    #[test]
    fn cyclic_property_shape_does_not_overflow() {
        let shapes = Graph::load_str(
            r#"
            @prefix sh: <http://www.w3.org/ns/shacl#> .
            @prefix ex: <http://example.org/> .
            # A node shape A with a property to B, and B with a property back to A.
            ex:A a sh:NodeShape ;
              sh:targetNode ex:a ;
              sh:property ex:PtoB .
            ex:PtoB a sh:PropertyShape ;
              sh:path ex:next ;
              sh:property ex:PtoA .
            ex:PtoA a sh:PropertyShape ;
              sh:path ex:next ;
              sh:property ex:PtoB .
        "#,
            "turtle",
        )
        .unwrap();
        // Cyclic data: a -> b -> a. Without the guard this recurses unboundedly.
        let data = Graph::load_str(
            r#"
            @prefix ex: <http://example.org/> .
            ex:a ex:next ex:b .
            ex:b ex:next ex:a .
        "#,
            "turtle",
        )
        .unwrap();
        // The point of the test is that this returns at all (no stack overflow). The cycle is
        // treated as conforming, and no constraint forces a violation.
        let r = validate(&data, &shapes);
        assert!(
            r.conforms,
            "cyclic sh:property must terminate and conform: {}",
            r.to_text()
        );
    }

    // [OPUS-4.8] Regression for review 1616 (implicit class shape discovery): a node that is an
    // rdfs:Class with SHACL constraints — but NOT explicitly typed sh:NodeShape and with no
    // sh:target* — is an implicit node shape with an implicit class target. Root discovery
    // previously skipped it, silently ignoring its constraints; it must now be validated.
    #[test]
    fn implicit_class_shape_is_discovered() {
        let shapes = Graph::load_str(
            r#"
            @prefix sh: <http://www.w3.org/ns/shacl#> .
            @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
            @prefix ex: <http://example.org/> .
            # rdfs:Class + SHACL constraint, but NO sh:NodeShape type and NO sh:target*.
            ex:Person a rdfs:Class ;
              sh:property [ sh:path ex:name ; sh:minCount 1 ] .
        "#,
            "turtle",
        )
        .unwrap();
        // An instance missing ex:name must now fail (implicit class target on ex:Person).
        let data = Graph::load_str(
            r#"
            @prefix ex: <http://example.org/> .
            ex:alice a ex:Person .
        "#,
            "turtle",
        )
        .unwrap();
        let r = validate(&data, &shapes);
        assert!(
            !r.conforms,
            "implicit class shape must validate its instances: {}",
            r.to_text()
        );
        assert!(r
            .results
            .iter()
            .any(|x| x.source_component.ends_with("MinCountConstraintComponent")));
    }

    // [OPUS-4.8] (sq-bif) `load_turtle_with_base` resolves relative IRIs against
    // the supplied base (the seam `Graph::load_str` does not expose, used by the
    // W3C manifest loaders) and surfaces a parse error as an `Err(String)`.
    #[test]
    fn load_turtle_with_base_resolves_relatives_and_reports_errors() {
        // `<rel>` and `<#frag>` resolve against the base.
        let g = load_turtle_with_base(
            "<rel> <http://example.org/p> <#frag> .",
            "http://base.example/dir/",
        )
        .unwrap();
        let v = view::GraphView::new(&g);
        // The relative subject resolved to base + "rel".
        assert!(v.contains(
            &oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(
                "http://base.example/dir/rel"
            )),
            "http://example.org/p",
            &oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(
                "http://base.example/dir/#frag"
            )),
        ));
        // A malformed Turtle document is an Err, not a panic (Graph has no Debug,
        // so map to the error string before asserting).
        let err = load_turtle_with_base("this is not turtle @@@", "http://base.example/").err();
        assert!(err.is_some(), "malformed Turtle must Err");
        // An invalid base IRI is also a (different) Err.
        assert!(load_turtle_with_base("", "not a valid base").is_err());
    }

    // [OPUS-4.8] (sq-bif) `graph_from_triples` interns oxrdf triples — including
    // a BLANK-NODE subject (the subject match arm `Graph::load_str` round-trips
    // but the helper handles explicitly) — into a queryable Graph.
    #[test]
    fn graph_from_triples_interns_iri_and_blank_subjects() {
        use oxrdf::{BlankNode, NamedNode, NamedOrBlankNode, Term, Triple};
        let p = NamedNode::new_unchecked("http://example.org/p");
        let bnode = BlankNode::default();
        let triples = vec![
            // IRI subject.
            Triple::new(
                NamedOrBlankNode::NamedNode(NamedNode::new_unchecked("http://example.org/s")),
                p.clone(),
                Term::NamedNode(NamedNode::new_unchecked("http://example.org/o")),
            ),
            // Blank-node subject (the BlankNode arm of the subject match).
            Triple::new(
                NamedOrBlankNode::BlankNode(bnode.clone()),
                p.clone(),
                Term::Literal(oxrdf::Literal::new_simple_literal("x")),
            ),
        ];
        let g = graph_from_triples(triples);
        let v = view::GraphView::new(&g);
        // Both triples are present and queryable.
        assert!(v.contains(
            &Term::NamedNode(NamedNode::new_unchecked("http://example.org/s")),
            "http://example.org/p",
            &Term::NamedNode(NamedNode::new_unchecked("http://example.org/o")),
        ));
        assert!(v.contains(
            &Term::BlankNode(bnode),
            "http://example.org/p",
            &Term::Literal(oxrdf::Literal::new_simple_literal("x")),
        ));
        // An empty iterator yields an empty graph.
        let empty = graph_from_triples(std::iter::empty());
        assert!(view::GraphView::new(&empty)
            .triples(None, Some("http://example.org/p"), None)
            .is_empty());
    }

    /// [OPUS-4.8] (sq-mue75) A `sh:sparql` result carries `sh:sourceConstraint`
    /// pointing at the `sh:SPARQLConstraint` node — distinct from `sh:sourceShape`
    /// (the shape) — and the Turtle report emits it. A Core (non-sparql) result
    /// carries no `sh:sourceConstraint`.
    #[test]
    fn sparql_result_carries_source_constraint() {
        let shapes = Graph::load_str(
            r#"
            @prefix sh: <http://www.w3.org/ns/shacl#> .
            @prefix ex: <http://example.org/> .
            ex:TestShape a sh:NodeShape ;
              sh:targetNode ex:bad ;
              sh:sparql ex:TestShape-sparql .
            ex:TestShape-sparql a sh:SPARQLConstraint ;
              sh:select "SELECT $this WHERE { $this <http://example.org/flag> true }" .
        "#,
            "turtle",
        )
        .unwrap();
        let data = Graph::load_str(
            "@prefix ex: <http://example.org/> . ex:bad ex:flag true .",
            "turtle",
        )
        .unwrap();
        let r = validate(&data, &shapes);
        assert!(!r.conforms, "{}", r.to_text());
        assert_eq!(r.results.len(), 1, "{}", r.to_text());
        let res = &r.results[0];
        // sh:sourceConstraint = the sh:SPARQLConstraint node, NOT the shape.
        assert_eq!(
            res.source_constraint,
            Some(oxrdf::Term::NamedNode(
                oxrdf::NamedNode::new("http://example.org/TestShape-sparql").unwrap()
            )),
            "expected sh:sourceConstraint = the constraint node"
        );
        assert_eq!(
            res.source_shape,
            oxrdf::Term::NamedNode(oxrdf::NamedNode::new("http://example.org/TestShape").unwrap()),
            "sh:sourceShape must remain the shape node"
        );
        assert_ne!(
            res.source_constraint.as_ref(),
            Some(&res.source_shape),
            "sourceConstraint and sourceShape must be distinct here"
        );
        // The Turtle report emits sh:sourceConstraint.
        let ttl = r.to_turtle();
        assert!(
            ttl.contains("sh:sourceConstraint"),
            "report Turtle must emit sh:sourceConstraint:\n{ttl}"
        );
    }

    /// [OPUS-4.8] (sq-mue75) A Core (non-`sh:sparql`) constraint result carries no
    /// `sh:sourceConstraint` (the spec stamps it only on SPARQL-based results).
    #[test]
    fn core_result_has_no_source_constraint() {
        let r = check(
            r#"
            @prefix ex: <http://example.org/> .
            ex:bob a ex:Person ; ex:age -1 .
        "#,
        );
        assert!(!r.conforms);
        assert!(
            r.results.iter().all(|res| res.source_constraint.is_none()),
            "Core results must not carry sh:sourceConstraint"
        );
    }

    #[test]
    fn logical_and_paths() {
        let data = Graph::load_str(
            r#"
            @prefix ex: <http://example.org/> .
            ex:a ex:knows ex:b . ex:b ex:knows ex:c .
            ex:c a ex:Person .
        "#,
            "turtle",
        )
        .unwrap();
        let shapes = Graph::load_str(
            r#"
            @prefix sh: <http://www.w3.org/ns/shacl#> .
            @prefix ex: <http://example.org/> .
            ex:S a sh:NodeShape ;
              sh:targetNode ex:a ;
              sh:property [
                sh:path ( ex:knows ex:knows ) ;
                sh:class ex:Person ;
                sh:minCount 1 ;
              ] .
        "#,
            "turtle",
        )
        .unwrap();
        let r = validate(&data, &shapes);
        assert!(r.conforms, "{}", r.to_text());
    }

    // --- [OPUS-4.8] (sq-7d3dj.33.1) sh:sparql focus-node batching --------------

    /// A stable, order-independent key for one result, covering every field the
    /// W3C suite compares (focus / value / path / component / source shape+constraint
    /// / severity / message). Two reports are report-equivalent iff their sorted key
    /// multisets are equal.
    fn result_keys(r: &ValidationReport) -> Vec<String> {
        let mut ks: Vec<String> = r
            .results
            .iter()
            .map(|x| {
                format!(
                    "{}|{}|{}|{}|{}|{}|{}|{}",
                    x.focus_node,
                    x.value.as_ref().map(|v| v.to_string()).unwrap_or_default(),
                    x.path.as_ref().map(Path::to_turtle).unwrap_or_default(),
                    x.source_component,
                    x.source_shape,
                    x.source_constraint
                        .as_ref()
                        .map(|c| c.to_string())
                        .unwrap_or_default(),
                    x.severity,
                    x.default_message,
                )
            })
            .collect();
        ks.sort();
        ks
    }

    /// Data + shapes with a `sh:sparql` constraint over MANY focus nodes, a mix of
    /// violating and conforming, plus a non-`sh:sparql` (core) constraint on the same
    /// shape so batching must interleave correctly with the per-focus components.
    fn batch_case() -> (Graph, Graph) {
        let mut data = String::from("@prefix ex: <http://example.org/> .\n");
        for i in 0..60 {
            // Even i => age negative (violates the sh:sparql); every 7th also lacks a
            // name (violates the core minCount) — exercising mixed components/foci.
            let age = if i % 2 == 0 {
                -(i as i64) - 1
            } else {
                i as i64
            };
            data.push_str(&format!("ex:n{} a ex:Person ; ex:age {} .\n", i, age));
            if i % 7 != 0 {
                data.push_str(&format!("ex:n{} ex:name \"n{}\" .\n", i, i));
            }
        }
        let shapes = r#"
            @prefix sh: <http://www.w3.org/ns/shacl#> .
            @prefix ex: <http://example.org/> .
            @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
            ex:PersonShape a sh:NodeShape ;
              sh:targetClass ex:Person ;
              sh:property [ sh:path ex:name ; sh:minCount 1 ] ;
              sh:sparql [
                a sh:SPARQLConstraint ;
                sh:prefixes ex:p ;
                sh:message "age must not be negative" ;
                sh:select """SELECT $this ?value WHERE { $this <http://example.org/age> ?value . FILTER(?value < 0) }""" ;
              ] .
            ex:p sh:declare [ sh:prefix "ex" ; sh:namespace "http://example.org/"^^xsd:anyURI ] .
        "#;
        (
            Graph::load_str(&data, "turtle").unwrap(),
            Graph::load_str(shapes, "turtle").unwrap(),
        )
    }

    /// HARD invariant: the batched `sh:sparql` path produces a byte-for-byte
    /// report-equivalent result set (and `conforms`) to the per-focus path, over a
    /// many-focus mixed-component workload.
    #[test]
    fn batched_equals_per_focus_report() {
        let (data, shapes) = batch_case();
        let batched = validate(&data, &shapes);
        let per_focus = eval::with_sparql_batching_disabled(|| validate(&data, &shapes));
        assert_eq!(
            batched.conforms, per_focus.conforms,
            "conforms diverged between batched and per-focus"
        );
        assert_eq!(
            batched.results.len(),
            per_focus.results.len(),
            "result count diverged"
        );
        assert_eq!(
            result_keys(&batched),
            result_keys(&per_focus),
            "batched vs per-focus reports differ"
        );
        // The sh:sparql constraint alone flags the 30 even-index nodes.
        let sparql_hits = batched
            .results
            .iter()
            .filter(|r| r.source_component.ends_with("SPARQLConstraintComponent"))
            .count();
        assert_eq!(sparql_hits, 30, "expected 30 negative-age violations");
    }

    /// [OPUS-4.8] (sq-7d3dj.33.1) Local, NON-canonical measurement of the batched vs
    /// per-focus `sh:sparql` wall time on a synthetic workload shaped like the LUBM
    /// `sparql_constraint`/`sparql_heavy` benchmarks (a `SELECT $this ?value` with a
    /// two-triple BGP + type check, one solution per violating focus). Prints the
    /// speed-up ratio; run with `cargo test -p sparq-shacl --lib -- --ignored
    /// --nocapture perf_batched`. `#[ignore]` because it is a timing harness, not a
    /// pass/fail gate (the box is shared → numbers are directional only). It still
    /// asserts the reports match and the execution-count contrast holds.
    #[test]
    #[ignore = "local timing harness; non-canonical, run with --ignored --nocapture"]
    fn perf_batched_vs_per_focus() {
        use std::time::Instant;
        const N: usize = 2000;
        let mut data = String::from("@prefix ex: <http://example.org/> .\n");
        for i in 0..N {
            data.push_str(&format!(
                "ex:s{} a ex:Student ; ex:takesCourse ex:c{} .\n",
                i, i
            ));
            // Half the courses are graduate courses → half the students violate.
            if i % 2 == 0 {
                data.push_str(&format!("ex:c{} a ex:GraduateCourse .\n", i));
            }
        }
        let shapes = r#"
            @prefix sh: <http://www.w3.org/ns/shacl#> .
            @prefix ex: <http://example.org/> .
            @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
            ex:S a sh:NodeShape ;
              sh:targetClass ex:Student ;
              sh:sparql [ a sh:SPARQLConstraint ; sh:prefixes ex:p ; sh:message "grad course" ;
                sh:select """SELECT $this ?value WHERE { $this <http://example.org/takesCourse> ?value . ?value a <http://example.org/GraduateCourse> }""" ] .
            ex:p sh:declare [ sh:prefix "ex" ; sh:namespace "http://example.org/"^^xsd:anyURI ] .
        "#;
        let data = Graph::load_str(&data, "turtle").unwrap();
        let shapes = Graph::load_str(shapes, "turtle").unwrap();

        // Warm up (parse/plan caches) then time each path best-of-3.
        let mut batched_ns = u128::MAX;
        let mut per_focus_ns = u128::MAX;
        let mut batched_report = None;
        let mut per_focus_report = None;
        for _ in 0..3 {
            let t = Instant::now();
            let r = validate(&data, &shapes);
            batched_ns = batched_ns.min(t.elapsed().as_nanos());
            batched_report = Some(r);

            let t = Instant::now();
            let r = eval::with_sparql_batching_disabled(|| validate(&data, &shapes));
            per_focus_ns = per_focus_ns.min(t.elapsed().as_nanos());
            per_focus_report = Some(r);
        }
        let batched = batched_report.unwrap();
        let per_focus = per_focus_report.unwrap();
        assert_eq!(result_keys(&batched), result_keys(&per_focus));
        let ratio = per_focus_ns as f64 / batched_ns as f64;
        println!(
            "[sq-7d3dj.33.1] N={N} foci  batched={:.3}ms  per_focus={:.3}ms  speed-up={:.1}x  (violations={})",
            batched_ns as f64 / 1e6,
            per_focus_ns as f64 / 1e6,
            ratio,
            batched.results.len(),
        );
    }

    /// Anti-vacuity: the batched path FIRES — one query execution for 60 focus nodes,
    /// not 60. Uses the public per-thread execution counter.
    #[test]
    fn batched_path_fires_once_for_many_foci() {
        let (data, shapes) = batch_case();
        let before = sparql_constraint_executions();
        let _ = validate(&data, &shapes);
        let batched_execs = sparql_constraint_executions() - before;
        assert_eq!(
            batched_execs, 1,
            "expected ONE batched sh:sparql execution for 60 foci, got {}",
            batched_execs
        );
        // And the per-focus path really does run one query per focus (the contrast
        // that makes the '1' meaningful, not an artefact of e.g. zero executions).
        let before_pf = sparql_constraint_executions();
        eval::with_sparql_batching_disabled(|| {
            let _ = validate(&data, &shapes);
        });
        let per_focus_execs = sparql_constraint_executions() - before_pf;
        assert_eq!(
            per_focus_execs, 60,
            "per-focus path should run one query per focus node"
        );
    }
}

// [OPUS-4.8] (sq-mk9n / sq-3w6n, `shacl-af`) The two SHACL-AF node-expression
// constraints: `sh:expression` (`sh:ExpressionConstraintComponent`) — a value
// node violates when the node expression does not evaluate to `{ true }` — and
// `sh:nodeByExpression` (`sh:NodeByExpressionConstraintComponent`) — a value node
// violates when it does not conform to a node shape the expression computes.
#[cfg(feature = "shacl-af")]
#[cfg(test)]
mod expression_tests {
    use super::*;

    fn g(ttl: &str) -> Graph {
        let prelude = "@prefix sh: <http://www.w3.org/ns/shacl#> .\n\
            @prefix shnex: <http://www.w3.org/ns/shacl-node-expr#> .\n\
            @prefix ex: <http://example.org/> .\n\
            @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n\
            @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .\n\
            @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n";
        Graph::load_str(&format!("{prelude}{ttl}"), "turtle").unwrap()
    }

    #[test]
    fn expression_constant_false_violates() {
        // The W3C `expression-001` shape: `sh:expression false` at a node shape ⇒
        // every targeted node is a violation (false != true).
        let data = g("ex:i a ex:C .");
        let shapes = g("ex:C a rdfs:Class, sh:NodeShape ; sh:expression false .");
        let r = validate(&data, &shapes);
        assert!(!r.conforms, "{}", r.to_text());
        assert!(r.results.iter().any(|res| res
            .source_component
            .ends_with("ExpressionConstraintComponent")
            && res.value.as_ref().map(|v| v.to_string()).as_deref()
                == Some("<http://example.org/i>")));
    }

    #[test]
    fn expression_true_conforms() {
        // A node expression that DOES evaluate to { true } conforms — here a
        // `shnex:exists` over an existing path value.
        let data = g("ex:i a ex:C ; ex:p ex:v .");
        let shapes = g("ex:C a rdfs:Class, sh:NodeShape ; \
             sh:expression [ shnex:exists [ sh:path ex:p ] ] .");
        let r = validate(&data, &shapes);
        assert!(
            r.conforms,
            "exists ex:p => true => conforms: {}",
            r.to_text()
        );
    }

    #[test]
    fn expression_on_property_shape_per_value() {
        // `sh:expression` on a property shape applies per path value node; here the
        // value (5) satisfies a matchAll-over-minInclusive-3 ⇒ exists ⇒ true.
        let data = g("ex:i a ex:C ; ex:age 5 .");
        let shapes = g("ex:C a rdfs:Class, sh:NodeShape ; \
             sh:property [ sh:path ex:age ; \
               sh:expression [ shnex:exists [ shnex:nodes [ shnex:var \"focusNode\" ] ; \
                 shnex:matchAll [ sh:minInclusive 3 ] ] ] ] .");
        let r = validate(&data, &shapes);
        assert!(
            r.conforms,
            "age 5 >= 3 => matchAll true => exists true: {}",
            r.to_text()
        );
    }

    // ---- [OPUS-4.8] (sq-3w6n) sh:nodeByExpression ----

    #[test]
    fn node_by_expression_constant_iri_violates_nonconforming_value() {
        // The W3C `nodeByExpression-001` shape: a property shape whose value nodes
        // must conform to the node shape an expression computes. Here the expression
        // is the constant IRI `ex:AssignedToShape` (per SHACL-AF, an IRI expression
        // evaluating to { ex:AssignedToShape } — the `sh:node` special case). The
        // assignee lacks the required `ex:email`, so it does NOT conform => one
        // violation with the value node and component IRI.
        let data = g("ex:issue a ex:Issue ; ex:assignedTo ex:anon . ex:anon a ex:Person .");
        let shapes = g("ex:AssignedToShape a sh:NodeShape ; \
               sh:property [ sh:path ex:email ; sh:minCount 1 ] . \
             ex:Issue a rdfs:Class, sh:NodeShape ; \
               sh:property [ sh:path ex:assignedTo ; sh:nodeByExpression ex:AssignedToShape ] .");
        let r = validate(&data, &shapes);
        assert!(!r.conforms, "{}", r.to_text());
        assert!(
            r.results.iter().any(|res| res
                .source_component
                .ends_with("NodeByExpressionConstraintComponent")
                && res.value.as_ref().map(|v| v.to_string()).as_deref()
                    == Some("<http://example.org/anon>")),
            "expected a NodeByExpression violation for ex:anon: {}",
            r.to_text()
        );
    }

    #[test]
    fn node_by_expression_conforming_value_passes() {
        // The same constraint, but the assignee carries the required email, so it
        // conforms to the computed node shape and there is no violation.
        let data = g("ex:issue a ex:Issue ; ex:assignedTo ex:jane . \
             ex:jane a ex:Person ; ex:email \"jane@ex.org\" .");
        let shapes = g("ex:AssignedToShape a sh:NodeShape ; \
               sh:property [ sh:path ex:email ; sh:minCount 1 ] . \
             ex:Issue a rdfs:Class, sh:NodeShape ; \
               sh:property [ sh:path ex:assignedTo ; sh:nodeByExpression ex:AssignedToShape ] .");
        let r = validate(&data, &shapes);
        assert!(
            r.conforms,
            "ex:jane has an email => conforms: {}",
            r.to_text()
        );
    }

    #[test]
    fn node_by_expression_path_computed_shape() {
        // A dynamically-computed shape: the node expression is a path-values
        // expression that locates the shape at `ex:hasShape` from the value node.
        // ex:obs/ex:hasShape => ex:RangeShape (minInclusive 0); ex:obs's measure is
        // -1, so it must NOT conform => one violation on the value node.
        let data = g(
            "ex:obs a ex:Observation ; ex:value -1 ; ex:hasShape ex:RangeShape . \
             ex:s a ex:DataShape ; ex:item ex:obs .",
        );
        let shapes = g("ex:RangeShape a sh:NodeShape ; \
               sh:property [ sh:path ex:value ; sh:minInclusive 0 ] . \
             ex:DataShape a rdfs:Class, sh:NodeShape ; \
               sh:property [ sh:path ex:item ; \
                 sh:nodeByExpression [ sh:path ex:hasShape ] ] .");
        let r = validate(&data, &shapes);
        assert!(!r.conforms, "value -1 < 0 must violate: {}", r.to_text());
        assert!(
            r.results.iter().any(|res| res
                .source_component
                .ends_with("NodeByExpressionConstraintComponent")
                && res.value.as_ref().map(|v| v.to_string()).as_deref()
                    == Some("<http://example.org/obs>")),
            "expected a NodeByExpression violation for ex:obs: {}",
            r.to_text()
        );
    }
}

// [FABLE-5] (sq-7d3dj.33.4) Report-equivalence tests for the id-level
// core-constraint fast path: the SAME (data, shapes) pair validated with the
// fast path ON (the default) and OFF (`eval::with_id_fastpath_disabled`) must
// produce BYTE-IDENTICAL reports — result order included (stricter than the
// order-independent `result_keys`) — and the fast path must actually FIRE
// (non-vacuity via the id-walk counter, mirroring the sq-7d3dj.33.1 idiom).
#[cfg(test)]
mod idfast_tests {
    use super::*;

    fn g(ttl: &str) -> Graph {
        let prelude = "@prefix sh: <http://www.w3.org/ns/shacl#> .\n\
            @prefix ex: <http://example.org/> .\n\
            @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n\
            @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .\n";
        Graph::load_str(&format!("{prelude}{ttl}"), "turtle").unwrap()
    }

    /// Validates twice — id fast path ON then OFF — asserting (a) the fast path
    /// fired at least `min_walks` id-level value walks and `min_ext` extended
    /// id-level surface checks (sq-j9hls — anti-vacuity), (b) the forced-off run
    /// fired NONE of either (the toggle works), and (c) the two reports are
    /// byte-identical including order (`Debug` covers every field of every
    /// result, recursively through `sh:detail`).
    fn assert_identical_reports(data: &Graph, shapes: &Graph, min_walks: u64, min_ext: u64) {
        let (walks_before, ext_before) = (eval::idfast_walks(), eval::idfast_ext_checks());
        let fast = validate(data, shapes);
        let walks = eval::idfast_walks() - walks_before;
        let ext = eval::idfast_ext_checks() - ext_before;
        assert!(
            walks >= min_walks,
            "id fast path fired {walks} id walks, expected >= {min_walks} (vacuous differential)"
        );
        assert!(
            ext >= min_ext,
            "extended id surfaces fired {ext} checks, expected >= {min_ext} (vacuous differential)"
        );
        let (walks_before, ext_before) = (eval::idfast_walks(), eval::idfast_ext_checks());
        let slow = eval::with_id_fastpath_disabled(|| validate(data, shapes));
        assert_eq!(
            eval::idfast_walks(),
            walks_before,
            "with_id_fastpath_disabled failed: the forced run still took id walks"
        );
        assert_eq!(
            eval::idfast_ext_checks(),
            ext_before,
            "with_id_fastpath_disabled failed: the forced run still took extended id checks"
        );
        assert_eq!(fast.conforms, slow.conforms, "conforms diverged");
        assert_eq!(
            format!("{:?}", fast.results),
            format!("{:?}", slow.results),
            "id-fast-path report differs from the Term-level report"
        );
    }

    /// Datatype (well-/ill-formed lexicals, language-tagged, IRI, blank-node and
    /// INLINE-integer values), pattern (+flags, IRI/blank subjects), lengths
    /// (multi-byte chars), counts and nodeKind — the id arms of the benchmarked
    /// `datatype_range` workload shape.
    #[test]
    fn idfast_value_constraints_report_equivalent() {
        let data = g(r#"
            ex:a a ex:T ; ex:age 42 ; ex:name "Alice" ; ex:mail "a@ex.org" .
            ex:b a ex:T ; ex:age "4.2"^^xsd:integer ; ex:name "Bo"@en ; ex:mail "nope" .
            ex:c a ex:T ; ex:age "007"^^xsd:integer ; ex:name ex:iriName ; ex:mail _:b1 .
            ex:d a ex:T ; ex:age "-3"^^xsd:integer , 7 ; ex:name "Δδ" ; ex:mail "X@Y.Z" .
            ex:e a ex:T ; ex:age <<( ex:s ex:p ex:o )>> ; ex:name _:b2 .
        "#);
        let shapes = g(r#"
            ex:S a sh:NodeShape ; sh:targetClass ex:T ;
              sh:property [ sh:path ex:age ; sh:datatype xsd:integer ; sh:maxCount 1 ;
                            sh:pattern "^4" ; sh:minLength 2 ] ;
              sh:property [ sh:path ex:name ; sh:datatype xsd:string ; sh:minCount 1 ;
                            sh:minLength 3 ; sh:maxLength 5 ; sh:nodeKind sh:Literal ;
                            sh:pattern "^[A-Za-zΔδ]+$" ] ;
              sh:property [ sh:path ex:mail ; sh:pattern "^[^@]+@.+$" ; sh:flags "i" ;
                            sh:nodeKind ( sh:Literal sh:BlankNode ) ] .
        "#);
        let r = validate(&data, &shapes);
        assert!(!r.conforms, "fixture must produce violations to compare");
        assert_identical_reports(&data, &shapes, 4, 1);
    }

    /// Every path form (predicate / inverse / sequence / alternative /
    /// zeroOrMore / oneOrMore / zeroOrOne / absent predicate) plus `sh:node`
    /// through the id-keyed conformance memo — the benchmarked `node_paths`
    /// workload shape, with a knows-cycle exercising the id closure.
    #[test]
    fn idfast_paths_and_node_report_equivalent() {
        let data = g(r#"
            ex:s1 a ex:Student ; ex:advisor ex:profHead ; ex:memberOf ex:dept ;
                  ex:email "s1@u.edu" ; ex:knows ex:s2 ; ex:nick "n1" .
            ex:s2 a ex:Student ; ex:advisor ex:profPlain ; ex:knows ex:s1 .
            ex:s3 a ex:Student ; ex:memberOf ex:orphanDept ; ex:phone "555" .
            ex:profHead ex:headOf ex:dept .
            ex:dept ex:subOrgOf ex:univ .
            ex:t1 ex:teaches ex:s1 . ex:t2 ex:teaches ex:s1 .
        "#);
        let shapes = g(r#"
            ex:P a sh:NodeShape ; sh:targetClass ex:Student ;
              sh:property [ sh:path ex:advisor ; sh:node ex:HeadShape ] ;
              sh:property [ sh:path ( ex:memberOf ex:subOrgOf ) ; sh:minCount 1 ] ;
              sh:property [ sh:path [ sh:inversePath ex:teaches ] ; sh:maxCount 1 ] ;
              sh:property [ sh:path [ sh:alternativePath ( ex:email ex:phone ) ] ; sh:minCount 1 ] ;
              sh:property [ sh:path [ sh:zeroOrMorePath ex:knows ] ; sh:maxCount 1 ] ;
              sh:property [ sh:path [ sh:oneOrMorePath ex:knows ] ; sh:minCount 1 ] ;
              sh:property [ sh:path [ sh:zeroOrOnePath ex:nick ] ; sh:minCount 2 ] ;
              sh:property [ sh:path ex:absentPredicate ; sh:minCount 1 ] .
            ex:HeadShape a sh:NodeShape ; sh:property [ sh:path ex:headOf ; sh:minCount 1 ] .
        "#);
        let r = validate(&data, &shapes);
        assert!(!r.conforms, "fixture must produce violations to compare");
        assert_identical_reports(&data, &shapes, 8, 1);
    }

    /// A cyclic `sh:node` reference: the recursion guard treats re-entry as
    /// conforming and the context-free memo rule decides what may be cached —
    /// `conforms_id`'s id-keyed memo layer must mirror the Term-keyed verdicts.
    #[test]
    fn idfast_cyclic_node_report_equivalent() {
        let data = g(r#"
            ex:n1 a ex:N ; ex:next ex:n2 ; ex:label "one" .
            ex:n2 a ex:N ; ex:next ex:n1 .
            ex:n3 a ex:N ; ex:next ex:n3 ; ex:label "three" .
        "#);
        let shapes = g(r#"
            ex:A a sh:NodeShape ; sh:targetClass ex:N ;
              sh:property [ sh:path ex:next ; sh:node ex:A ] ;
              sh:property [ sh:path ex:label ; sh:minCount 1 ] .
        "#);
        let r = validate(&data, &shapes);
        assert!(!r.conforms, "fixture must produce violations to compare");
        assert_identical_reports(&data, &shapes, 3, 1);
    }

    /// A `sh:targetNode` focus ABSENT from the data graph: no id resolves, so the
    /// whole validation of that focus takes the Term-level fallback — including
    /// the zeroOrOne path whose value set contains the (dictionary-less) focus
    /// term itself. `min_walks = 0`: no id walk is expected here.
    #[test]
    fn idfast_absent_focus_falls_back_to_term_route() {
        let data = g("ex:present ex:p 1 .");
        let shapes = g(r#"
            ex:G a sh:NodeShape ; sh:targetNode ex:ghost ;
              sh:property [ sh:path [ sh:zeroOrOnePath ex:p ] ; sh:datatype xsd:integer ] .
        "#);
        let r = validate(&data, &shapes);
        assert!(
            !r.conforms,
            "the ghost focus itself flows through the zeroOrOne path and violates xsd:integer"
        );
        assert_identical_reports(&data, &shapes, 0, 0);
    }

    /// [FABLE-5] (sq-j9hls) `sh:class` / `sh:class (list)` instance checks and
    /// `sh:closed` on the extended id routes: a subclass-closure hit, a typeless
    /// node, a literal value, a class ABSENT from the data dictionary (every
    /// value violates on both routes), a `sh:targetClass` enumeration through
    /// the subclass closure, node-shape `sh:class` (Term-form values under a
    /// resolved focus id), and a closed node shape with `sh:ignoredProperties`
    /// plus disallowed-predicate violations.
    #[test]
    fn idfast_class_and_closed_report_equivalent() {
        let data = g(r#"
            ex:Employee <http://www.w3.org/2000/01/rdf-schema#subClassOf> ex:Person .
            ex:alice a ex:Employee ; ex:worksFor ex:acme ; ex:name "Alice" ;
                     ex:hobby "chess" .
            ex:bob a ex:Robot ; ex:worksFor ex:acme ; ex:name "Bob" .
            ex:carol ex:worksFor ex:ghostOrg ; ex:name "Carol" .
            ex:acme a ex:Org .
            ex:dan a ex:Person ; ex:knows ex:acme , "lit" , ex:nobody ; ex:name "Dan" .
        "#);
        let shapes = g(r#"
            ex:PersonShape a sh:NodeShape ; sh:targetClass ex:Person ;
              sh:closed true ; sh:ignoredProperties ( rdf:type ex:knows ) ;
              sh:property [ sh:path ex:worksFor ; sh:class ex:Org ] ;
              sh:property [ sh:path ex:name ; sh:minCount 1 ] .
            ex:WorkerShape a sh:NodeShape ; sh:targetSubjectsOf ex:worksFor ;
              sh:class ex:Person ;
              sh:property [ sh:path ex:worksFor ; sh:class ex:Absent ] .
            ex:KnowsShape a sh:NodeShape ; sh:targetSubjectsOf ex:knows ;
              sh:property [ sh:path ex:knows ; sh:class ( ex:Person ex:Org ) ] .
        "#);
        let r = validate(&data, &shapes);
        assert!(!r.conforms, "fixture must produce violations to compare");
        assert_identical_reports(&data, &shapes, 2, 4);
    }

    /// [FABLE-5] (sq-j9hls) The comparand-path components on the extended id
    /// routes: `sh:equals` (both missing directions), `sh:disjoint` (shared
    /// value + an absent comparand predicate), `sh:lessThan(OrEquals)` (violating
    /// pairs + a non-comparable pair; the comparand walk is id-level, the pair
    /// comparison stays term-level), `sh:subsetOf`, and an INVERSE comparand
    /// path exercising the compiled non-predicate form.
    #[test]
    fn idfast_comparand_paths_report_equivalent() {
        let data = g(r#"
            ex:a ex:p 1 , 2 ; ex:q 2 , 3 ; ex:r 1 ; ex:below 5 ; ex:sub 1 , 4 .
            ex:b ex:p ex:x ; ex:q ex:x ; ex:below "abc" ; ex:sub ex:x .
            ex:z ex:emp ex:a .
        "#);
        let shapes = g(r#"
            ex:Cmp a sh:NodeShape ; sh:targetSubjectsOf ex:p ;
              sh:property [ sh:path ex:p ; sh:equals ex:q ] ;
              sh:property [ sh:path ex:p ; sh:disjoint ex:r ] ;
              sh:property [ sh:path ex:p ; sh:disjoint ex:absent ] ;
              sh:property [ sh:path ex:below ; sh:lessThan ex:p ] ;
              sh:property [ sh:path ex:below ; sh:lessThanOrEquals ex:q ] ;
              sh:property [ sh:path ex:sub ; sh:subsetOf ex:q ] ;
              sh:property [ sh:path ex:p ; sh:equals [ sh:inversePath ex:emp ] ] .
        "#);
        let r = validate(&data, &shapes);
        assert!(!r.conforms, "fixture must produce violations to compare");
        assert_identical_reports(&data, &shapes, 7, 10);
    }

    /// [FABLE-5] (sq-j9hls) `sh:hasValue` / `sh:in` / `sh:uniqueLang` on the
    /// extended id routes: exact (id-equality) membership hits, a VALUE-equal
    /// literal that is not term-equal (`5` vs `"5.0"^^xsd:decimal` — the
    /// literal-only materialising branch), a required value over an empty value
    /// set, and uniqueLang duplicates including a case-insensitive language pair
    /// (keys computed from the stored slot, zero materialisation).
    #[test]
    fn idfast_value_enumerations_report_equivalent() {
        let data = g(r#"
            ex:s1 ex:status "active" ; ex:tag ex:red ; ex:score 5 ;
                  ex:label "one"@en , "two"@en , "drei"@de .
            ex:s2 ex:status "retired" ; ex:tag ex:blue ; ex:score "5.0"^^xsd:decimal ;
                  ex:label "a"@fr , "b"@FR .
        "#);
        let shapes = g(r#"
            ex:E a sh:NodeShape ; sh:targetSubjectsOf ex:status ;
              sh:property [ sh:path ex:status ; sh:in ( "active" "dormant" ) ] ;
              sh:property [ sh:path ex:tag ; sh:in ( ex:red ex:green ) ] ;
              sh:property [ sh:path ex:score ; sh:in ( 5 ) ] ;
              sh:property [ sh:path ex:score ; sh:hasValue 5 ] ;
              sh:property [ sh:path ex:tag ; sh:hasValue ex:red ] ;
              sh:property [ sh:path ex:missing ; sh:hasValue ex:red ] ;
              sh:property [ sh:path ex:label ; sh:uniqueLang true ] .
        "#);
        let r = validate(&data, &shapes);
        assert!(!r.conforms, "fixture must produce violations to compare");
        assert_identical_reports(&data, &shapes, 7, 10);
    }
}
