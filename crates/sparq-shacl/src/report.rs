//! Validation reports: the result structs, the report RDF graph (as Turtle or
//! N-Triples, per the SHACL results vocabulary), deterministic JSON and a
//! human-readable text rendering.

use crate::model::SH;
use crate::path::Path;
use oxrdf::Term;
use std::fmt::Write as _;

/// (sq-sx15d) The DEFAULT `sh:conformanceDisallows` severity set
/// (SHACL 1.2 Core §3.9): a data graph fails to conform when ANY validation
/// result carries one of these severities. Results at `sh:Debug` / `sh:Trace`
/// (below `sh:Info` in the `sh:Trace < sh:Debug < sh:Info < sh:Warning <
/// sh:Violation` ordering) are reported but do not break conformance.
pub const DEFAULT_CONFORMANCE_DISALLOWS: &[&str] = &[
    "http://www.w3.org/ns/shacl#Violation",
    "http://www.w3.org/ns/shacl#Warning",
    "http://www.w3.org/ns/shacl#Info",
];

/// (sq-sx15d) `true` iff none of `results` carries a severity in the
/// `disallowed` set — the SHACL 1.2 conformance test (default or custom set).
fn conforms_over(results: &[ValidationResult], disallowed: &[&str]) -> bool {
    !results
        .iter()
        .any(|r| disallowed.iter().any(|d| r.severity == *d))
}

/// One validation result (one constraint violation/warning/info).
#[derive(Debug, Clone)]
pub struct ValidationResult {
    /// The node that was being validated.
    pub focus_node: Term,
    /// The property path of the source property shape (sh:resultPath), or the
    /// offending predicate for sh:closed.
    pub path: Option<Path>,
    /// The offending value node, when the component reports one.
    pub value: Option<Term>,
    /// The shape the failing constraint is declared on.
    pub source_shape: Term,
    /// (sq-mue75) The constraint node a SPARQL-based result came from
    /// (`sh:sourceConstraint`): the `sh:SPARQLConstraint` node (the object of
    /// `sh:sparql`). SHACL §5.2.2 stamps this on `sh:sparql` results so a report
    /// can point at the exact constraint, distinct from `sh:sourceShape` (the
    /// shape) and `sh:sourceConstraintComponent`. `None` for every non-`sh:sparql`
    /// component (Core constraints and SPARQL-based constraint COMPONENTS have no
    /// single `sh:SPARQLConstraint` node, so the spec omits it there).
    pub source_constraint: Option<Term>,
    /// The constraint-component IRI (e.g. `sh:MinCountConstraintComponent`).
    pub source_component: String,
    /// Severity IRI (sh:Violation / sh:Warning / sh:Info, or custom).
    pub severity: String,
    /// `sh:message`s of the source shape (literals, possibly language-tagged).
    pub messages: Vec<Term>,
    /// A generated message used when the shape declares none.
    pub default_message: String,
    /// (sq-f8gu) Nested `sh:detail` sub-results explaining WHY this
    /// result fired — currently the per-member (`sh:memberShape`) / per-duplicate
    /// (`sh:uniqueMembers`) results produced by validating each offending list
    /// member against the member shape. Empty for every other component. The
    /// SHACL spec keeps `sh:detail` non-normative (the W3C suite compares only
    /// top-level result fields), so these are informational and never affect
    /// `sh:conforms`.
    pub details: Vec<ValidationResult>,
}

/// (sq-lz99x) A non-fatal author-time diagnostic: a constraint the
/// validator could not evaluate and therefore SKIPPED (the crate's lenient
/// ill-formed-shape policy), surfaced so the skip is not silent. Two producers:
/// an uncompilable `sh:pattern` regex — the Rust `regex` crate has no
/// lookahead/lookbehind (neither does XML Schema regex, which the W3C SHACL
/// spec ties `sh:pattern` to), so a `(?!...)` pattern fails to compile — and
/// (sq-c1v3e) each parse-time **ill-formed shapes-graph construct**
/// ([`crate::IllFormedConstruct`]), which the lenient `validate` skips and the
/// strict channel rejects. A diagnostic never affects `conforms`: a skipped
/// constraint reports no violations.
#[derive(Debug, Clone)]
pub struct ShapeDiagnostic {
    /// The shape whose constraint was skipped (`sh:sourceShape`).
    pub source_shape: Term,
    /// The constraint-component IRI of the skipped constraint (e.g.
    /// `sh:PatternConstraintComponent`) — or, for an ill-formed-construct
    /// diagnostic (sq-c1v3e), the offending SHACL predicate IRI (e.g.
    /// `sh:minCount`), mirroring [`crate::IllFormedConstruct::predicate`].
    pub source_component: String,
    /// A human-readable explanation of why the constraint was skipped.
    pub message: String,
}

/// The outcome of validating a data graph against a shapes graph.
#[derive(Debug)]
pub struct ValidationReport {
    /// True iff there are no validation results.
    pub conforms: bool,
    pub results: Vec<ValidationResult>,
    /// (sq-lz99x) Non-fatal author-time diagnostics for constraints
    /// that could not be evaluated and were therefore SKIPPED (e.g. an
    /// uncompilable `sh:pattern` regex). These never affect `conforms` — a
    /// skipped constraint contributes no results — but surface the skip so it is
    /// not silent. Empty in the common (well-formed) case.
    pub diagnostics: Vec<ShapeDiagnostic>,
}

impl ValidationReport {
    // (sq-lz99x) Test-only convenience: the report tests below build
    // reports from hand-rolled results (no diagnostics). Production code calls
    // `with_diagnostics_and_disallows`, so gate this to `test` to avoid a
    // dead-code warning in the plain lib build.
    #[cfg(test)]
    pub(crate) fn new(results: Vec<ValidationResult>) -> Self {
        Self::with_diagnostics_and_disallows(results, Vec::new(), None)
    }

    /// (sq-lz99x / sq-5q76d) Build a report carrying validation results
    /// and skipped-constraint diagnostics (e.g. an uncompilable `sh:pattern`),
    /// computing `conforms` against an EXPLICIT `sh:conformanceDisallows` set (the
    /// shapes-graph-declared override, SHACL 1.2 Core §3.9) — falling back to the
    /// default set ([`DEFAULT_CONFORMANCE_DISALLOWS`]: Violation / Warning / Info)
    /// when `disallowed` is `None`. Results at `sh:Debug` / `sh:Trace` (and any
    /// custom severity outside the active set) are reported but do not break
    /// conformance. (A caller can still recompute against another set via
    /// [`conforms_with_disallowed`](Self::conforms_with_disallowed).)
    pub(crate) fn with_diagnostics_and_disallows(
        results: Vec<ValidationResult>,
        diagnostics: Vec<ShapeDiagnostic>,
        disallowed: Option<&[String]>,
    ) -> Self {
        let conforms = match disallowed {
            Some(set) => {
                let refs: Vec<&str> = set.iter().map(String::as_str).collect();
                conforms_over(&results, &refs)
            }
            None => conforms_over(&results, DEFAULT_CONFORMANCE_DISALLOWS),
        };
        ValidationReport {
            conforms,
            results,
            diagnostics,
        }
    }

    /// (sq-sx15d) Recompute conformance against a CUSTOM disallowed
    /// severity set (the SHACL 1.2 `sh:conformanceDisallows` of a results graph,
    /// SHACL Core §3.9): `true` iff no result carries a severity in `disallowed`.
    /// `disallowed` holds full severity IRIs (e.g.
    /// `http://www.w3.org/ns/shacl#Violation`). The public [`conforms`](Self::conforms)
    /// field uses the default set [`DEFAULT_CONFORMANCE_DISALLOWS`]; this lets a
    /// caller that read an explicit `sh:conformanceDisallows` apply it instead
    /// (e.g. a Warning result conforms when only `sh:Violation` is disallowed).
    pub fn conforms_with_disallowed(&self, disallowed: &[&str]) -> bool {
        conforms_over(&self.results, disallowed)
    }

    /// Severity-aware conformance: `true` iff no result carries
    /// `sh:Violation` severity — i.e. `sh:Warning` / `sh:Info` (and custom
    /// severities) are reported but not conformance-breaking. This is the
    /// "violations only" toggle several implementations expose for CI-style
    /// gating (equivalent to `conforms_with_disallowed(&[sh:Violation])`). It is
    /// strictly weaker than the spec default [`conforms`](Self::conforms) field,
    /// which also disallows `sh:Warning` / `sh:Info`.
    pub fn conforms_violations_only(&self) -> bool {
        self.results_with_severity(&format!("{SH}Violation"))
            .next()
            .is_none()
    }

    /// The results whose `sh:resultSeverity` is exactly `severity` (a full
    /// IRI, e.g. `http://www.w3.org/ns/shacl#Warning` or a custom severity).
    pub fn results_with_severity<'a>(
        &'a self,
        severity: &'a str,
    ) -> impl Iterator<Item = &'a ValidationResult> + 'a {
        self.results.iter().filter(move |r| r.severity == severity)
    }

    /// The report as an RDF graph serialised to Turtle, using the SHACL
    /// validation-report vocabulary.
    pub fn to_turtle(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "@prefix sh: <{SH}> .\n");
        let _ = writeln!(out, "[] a sh:ValidationReport ;");
        let _ = write!(out, "  sh:conforms {}", self.conforms);
        for r in &self.results {
            let _ = writeln!(out, " ;");
            let _ = write!(out, "  sh:result ");
            write_result_node(&mut out, r);
        }
        let _ = writeln!(out, " .");
        out
    }

    /// The report RDF graph serialised as one N-Triples statement per line.
    ///
    /// This emits the same validation-report graph as [`to_turtle`](Self::to_turtle),
    /// including nested `sh:detail` results.
    pub fn to_ntriples(&self) -> String {
        // (sq-qcuhh) Keep the Turtle renderer as the single report-graph
        // construction path, then use oxttl for complete N-Triples term escaping.
        let turtle = self.to_turtle();
        let triples = oxttl::TurtleParser::new().for_slice(turtle.as_bytes());
        let mut serializer = oxttl::NTriplesSerializer::new().low_level();
        let mut out = Vec::new();

        for triple in triples {
            let triple = triple.expect("ValidationReport::to_turtle must emit valid Turtle");
            serializer
                .serialize_triple(triple.as_ref(), &mut out)
                .expect("serialising N-Triples into memory must succeed");
        }

        String::from_utf8(out).expect("N-Triples output must be valid UTF-8")
    }

    /// A human-readable rendering of the report.
    pub fn to_text(&self) -> String {
        let mut out = if self.conforms {
            "Conforms: data graph satisfies all shapes.\n".to_string()
        } else {
            format!("Does not conform: {} result(s)\n", self.results.len())
        };
        for r in &self.results {
            let sev = r.severity.rsplit(['#', '/']).next().unwrap_or(&r.severity);
            let comp = r
                .source_component
                .rsplit(['#', '/'])
                .next()
                .unwrap_or(&r.source_component);
            let _ = write!(out, "- [{sev}] focus {}", r.focus_node);
            if let Some(p) = &r.path {
                let _ = write!(out, " | path {}", p.to_turtle());
            }
            if let Some(v) = &r.value {
                let _ = write!(out, " | value {v}");
            }
            let _ = writeln!(out, "\n    {comp}: {}", r.message_text());
            // (sq-f8gu) Render nested sh:detail sub-results, indented.
            for d in &r.details {
                let dcomp = d
                    .source_component
                    .rsplit(['#', '/'])
                    .next()
                    .unwrap_or(&d.source_component);
                let _ = write!(out, "      detail: {dcomp}");
                if let Some(v) = &d.value {
                    let _ = write!(out, " | value {v}");
                }
                let _ = writeln!(out);
            }
        }
        // (sq-lz99x) Surface skipped-constraint diagnostics so the
        // lenient skip is not silent. They never affect `conforms`.
        for d in &self.diagnostics {
            let comp = d
                .source_component
                .rsplit(['#', '/'])
                .next()
                .unwrap_or(&d.source_component);
            let _ = writeln!(out, "! [diagnostic] shape {} | {comp}", d.source_shape);
            let _ = writeln!(out, "    {}", d.message);
        }
        out
    }

    /// A deterministic JSON rendering of the report for machine consumers.
    ///
    /// Object keys have a stable order, validation results retain their report
    /// order, and absent paths, values, and diagnostics are omitted.
    pub fn to_json(&self) -> String {
        let mut out = String::from("{\n");
        let _ = write!(out, "  \"conforms\": {},\n  \"results\": [", self.conforms);

        if self.results.is_empty() {
            out.push(']');
        } else {
            out.push('\n');
            for (index, result) in self.results.iter().enumerate() {
                if index > 0 {
                    out.push_str(",\n");
                }
                let _ = writeln!(out, "    {{");
                let _ = writeln!(
                    out,
                    "      \"focusNode\": \"{}\",",
                    escape_json(&term_to_json_string(&result.focus_node))
                );
                if let Some(path) = &result.path {
                    let _ = writeln!(
                        out,
                        "      \"resultPath\": \"{}\",",
                        escape_json(&path.to_turtle())
                    );
                }
                if let Some(value) = &result.value {
                    let _ = writeln!(
                        out,
                        "      \"value\": \"{}\",",
                        escape_json(&term_to_json_string(value))
                    );
                }
                let _ = writeln!(
                    out,
                    "      \"sourceShape\": \"{}\",",
                    escape_json(&term_to_json_string(&result.source_shape))
                );
                let _ = writeln!(
                    out,
                    "      \"sourceConstraintComponent\": \"{}\",",
                    escape_json(&result.source_component)
                );
                let _ = writeln!(
                    out,
                    "      \"severity\": \"{}\",",
                    escape_json(&result.severity)
                );
                let _ = write!(
                    out,
                    "      \"resultMessage\": \"{}\"\n    }}",
                    escape_json(result.message_text())
                );
            }
            out.push_str("\n  ]");
        }

        if !self.diagnostics.is_empty() {
            out.push_str(",\n  \"diagnostics\": [\n");
            for (index, diagnostic) in self.diagnostics.iter().enumerate() {
                if index > 0 {
                    out.push_str(",\n");
                }
                let _ = writeln!(out, "    {{");
                let _ = writeln!(
                    out,
                    "      \"sourceShape\": \"{}\",",
                    escape_json(&term_to_json_string(&diagnostic.source_shape))
                );
                let _ = writeln!(
                    out,
                    "      \"sourceConstraintComponent\": \"{}\",",
                    escape_json(&diagnostic.source_component)
                );
                let _ = write!(
                    out,
                    "      \"message\": \"{}\"\n    }}",
                    escape_json(&diagnostic.message)
                );
            }
            out.push_str("\n  ]");
        }

        out.push_str("\n}");
        out
    }
}

impl ValidationReport {
    /// The compact JSON the JavaScript bindings (`sparq-wasm`'s `Store.validate` and
    /// `sparq-shacl-wasm`) return:
    ///
    /// ```json
    /// {"conforms":false,"results":[{"focusNode":"<http://ex/bob>",
    ///   "path":"<http://ex/age>","value":"\"-1\"^^<http://www.w3.org/2001/XMLSchema#integer>",
    ///   "sourceShape":"_:b0","sourceConstraintComponent":"http://www.w3.org/ns/shacl#…",
    ///   "severity":"http://www.w3.org/ns/shacl#Violation","message":"Value is not >= 0"}]}
    /// ```
    ///
    /// `focusNode`, `value` and `sourceShape` are N-Triples term strings, `path` is a
    /// SHACL Turtle path expression, and `message` is [`ValidationResult::message_text`].
    /// `path` and `value` are `null` when absent. Diagnostics are not included.
    pub fn to_js_json(&self) -> String {
        use sparq_core::json::push_string;
        fn field(out: &mut String, key: &str, val: Option<&str>) {
            out.push('"');
            out.push_str(key);
            out.push_str("\":");
            match val {
                Some(v) => push_string(out, v),
                None => out.push_str("null"),
            }
        }
        let mut out = String::from("{\"conforms\":");
        out.push_str(if self.conforms { "true" } else { "false" });
        out.push_str(",\"results\":[");
        for (i, r) in self.results.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push('{');
            field(&mut out, "focusNode", Some(&r.focus_node.to_string()));
            out.push(',');
            let path = r.path.as_ref().map(|p| p.to_turtle());
            field(&mut out, "path", path.as_deref());
            out.push(',');
            let value = r.value.as_ref().map(|v| v.to_string());
            field(&mut out, "value", value.as_deref());
            out.push(',');
            field(&mut out, "sourceShape", Some(&r.source_shape.to_string()));
            out.push(',');
            field(&mut out, "sourceConstraintComponent", Some(&r.source_component));
            out.push(',');
            field(&mut out, "severity", Some(&r.severity));
            out.push(',');
            field(&mut out, "message", Some(r.message_text()));
            out.push('}');
        }
        out.push_str("]}");
        out
    }
}

/// Returns a JSON-safe string body without surrounding quotes.
fn escape_json(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    sparq_core::json::escape_into(&mut escaped, value);
    escaped
}

/// Returns the report string representation for an RDF term.
fn term_to_json_string(term: &Term) -> String {
    match term {
        Term::NamedNode(node) => node.as_str().to_string(),
        Term::BlankNode(node) => format!("_:{}", node.as_str()),
        Term::Literal(literal) => literal.to_string(),
        Term::Triple(_) => term.to_string(),
    }
}

/// (sq-f8gu) Writes one `sh:ValidationResult` as an anonymous
/// blank-node block, recursively nesting `sh:detail` sub-results. Emitted as a
/// continuation of an already-written predicate (`sh:result ` / `sh:detail `),
/// and closed with `]` (no trailing `.` — the caller owns statement framing).
fn write_result_node(out: &mut String, r: &ValidationResult) {
    let _ = writeln!(out, "[");
    let _ = writeln!(out, "    a sh:ValidationResult ;");
    let _ = writeln!(out, "    sh:focusNode {} ;", r.focus_node);
    if let Some(p) = &r.path {
        let _ = writeln!(out, "    sh:resultPath {} ;", p.to_turtle());
    }
    if let Some(v) = &r.value {
        let _ = writeln!(out, "    sh:value {v} ;");
    }
    // A blank-node source shape has no graph-independent identity; emit its
    // label so the report stays self-consistent and parseable.
    let _ = writeln!(out, "    sh:sourceShape {} ;", r.source_shape);
    // (sq-mue75) `sh:sourceConstraint` — the originating
    // `sh:SPARQLConstraint` node, present only on `sh:sparql` results.
    if let Some(c) = &r.source_constraint {
        let _ = writeln!(out, "    sh:sourceConstraint {c} ;");
    }
    for m in r.effective_messages() {
        let _ = writeln!(out, "    sh:resultMessage {m} ;");
    }
    let _ = writeln!(out, "    sh:resultSeverity <{}> ;", r.severity);
    for d in &r.details {
        let _ = write!(out, "    sh:detail ");
        write_result_node(out, d);
        let _ = writeln!(out, " ;");
    }
    let _ = write!(
        out,
        "    sh:sourceConstraintComponent <{}> ]",
        r.source_component
    );
}

impl ValidationResult {
    /// The plain text a UI shows for this result: the lexical form of the first
    /// `sh:message` literal, or the generated default.
    pub fn message_text(&self) -> &str {
        match self.messages.first() {
            Some(Term::Literal(l)) => l.value(),
            _ => &self.default_message,
        }
    }

    /// The result messages: the source shape's sh:message literals, or the
    /// generated default when the shape declares none.
    pub fn effective_messages(&self) -> Vec<Term> {
        if self.messages.is_empty() {
            vec![Term::Literal(oxrdf::Literal::new_simple_literal(
                &self.default_message,
            ))]
        } else {
            self.messages.clone()
        }
    }
}

// Report-serialisation coverage (sq-qap0). report.rs (~64% covered)
// has branches the validate()-level tests don't reliably hit in isolation: the
// path/value-present vs -absent arms of to_turtle / to_text, the
// conforming-vs-not text rendering, the default-vs-declared message path, and
// the severity helpers. Constructing ValidationResults directly (all fields are
// pub) exercises each arm with a hand-built report.
#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::{BlankNode, Literal, NamedNode, Term, Triple};

    const EX: &str = "http://example.org/";

    fn iri(s: &str) -> Term {
        Term::NamedNode(NamedNode::new_unchecked(s.to_string()))
    }

    fn lit(s: &str) -> Term {
        Term::Literal(Literal::new_simple_literal(s))
    }

    /// A violation result with controllable path / value / messages.
    fn result(
        focus: Term,
        path: Option<Path>,
        value: Option<Term>,
        component: &str,
        severity: &str,
        messages: Vec<Term>,
    ) -> ValidationResult {
        ValidationResult {
            focus_node: focus,
            path,
            value,
            source_shape: iri(&format!("{EX}Shape")),
            source_constraint: None,
            source_component: format!("{SH}{component}"),
            severity: format!("{SH}{severity}"),
            messages,
            default_message: "generated default".into(),
            details: Vec::new(),
        }
    }

    /// Parses the (Turtle) report back to triples — both validates the syntax
    /// and lets a test assert on emitted predicates/objects.
    fn parse_report(ttl: &str) -> Vec<oxrdf::Triple> {
        oxttl::TurtleParser::new()
            .for_slice(ttl.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_else(|e| panic!("report Turtle does not parse: {e}\n{ttl}"))
    }

    #[test]
    fn js_json_shape_nulls_escaping_and_message_choice() {
        assert_eq!(
            ValidationReport::new(vec![]).to_js_json(),
            r#"{"conforms":true,"results":[]}"#
        );
        let declared = result(
            iri(&format!("{EX}bob")),
            Some(Path::Predicate(format!("{EX}age"))),
            Some(lit("say \"hi\"\n")),
            "MinInclusiveConstraintComponent",
            "Violation",
            vec![lit("tab\there"), lit("second")],
        );
        let bare = result(
            iri(&format!("{EX}eve")),
            None,
            None,
            "NodeConstraintComponent",
            "Warning",
            vec![],
        );
        let json = ValidationReport::new(vec![declared, bare]).to_js_json();
        assert_eq!(
            json,
            format!(
                concat!(
                    r#"{{"conforms":false,"results":["#,
                    r#"{{"focusNode":"<{ex}bob>","path":"<{ex}age>","value":"\"say \\\"hi\\\"\\n\"","#,
                    r#""sourceShape":"<{ex}Shape>","#,
                    r#""sourceConstraintComponent":"{sh}MinInclusiveConstraintComponent","#,
                    r#""severity":"{sh}Violation","message":"tab\there"}},"#,
                    r#"{{"focusNode":"<{ex}eve>","path":null,"value":null,"#,
                    r#""sourceShape":"<{ex}Shape>","#,
                    r#""sourceConstraintComponent":"{sh}NodeConstraintComponent","#,
                    r#""severity":"{sh}Warning","message":"generated default"}}]}}"#
                ),
                ex = EX,
                sh = SH
            )
        );
    }

    #[test]
    fn conforming_report_turtle_and_text() {
        let r = ValidationReport::new(vec![]);
        assert!(r.conforms);
        let ttl = r.to_turtle();
        assert!(ttl.contains("sh:conforms true"));
        // No sh:result blocks when conforming.
        assert!(!ttl.contains("sh:result"));
        parse_report(&ttl);
        assert_eq!(r.to_text(), "Conforms: data graph satisfies all shapes.\n");
    }

    #[test]
    fn conforming_report_json_is_pinned() {
        let report = ValidationReport::new(vec![]);

        assert_eq!(
            report.to_json(),
            "{\n  \"conforms\": true,\n  \"results\": []\n}"
        );
    }

    #[test]
    fn json_result_is_pinned_with_fixed_key_order() {
        let report = ValidationReport::new(vec![result(
            iri(&format!("{EX}alice")),
            Some(Path::Predicate(format!("{EX}age"))),
            Some(lit("thirty")),
            "MinCountConstraintComponent",
            "Violation",
            vec![lit("age is required")],
        )]);

        assert_eq!(
            report.to_json(),
            concat!(
                "{\n",
                "  \"conforms\": false,\n",
                "  \"results\": [\n",
                "    {\n",
                "      \"focusNode\": \"http://example.org/alice\",\n",
                "      \"resultPath\": \"<http://example.org/age>\",\n",
                "      \"value\": \"\\\"thirty\\\"\",\n",
                "      \"sourceShape\": \"http://example.org/Shape\",\n",
                "      \"sourceConstraintComponent\": ",
                "\"http://www.w3.org/ns/shacl#MinCountConstraintComponent\",\n",
                "      \"severity\": \"http://www.w3.org/ns/shacl#Violation\",\n",
                "      \"resultMessage\": \"age is required\"\n",
                "    }\n",
                "  ]\n",
                "}"
            )
        );
    }

    #[test]
    fn json_omits_absent_path_value_and_diagnostics() {
        let report = ValidationReport::new(vec![result(
            iri(&format!("{EX}bob")),
            None,
            None,
            "MinCountConstraintComponent",
            "Violation",
            vec![],
        )]);
        let json = report.to_json();

        assert!(!json.contains("resultPath"), "{json}");
        assert!(!json.contains("\"value\""), "{json}");
        assert!(!json.contains("diagnostics"), "{json}");
        assert!(json.contains("\"resultMessage\": \"generated default\""));
    }

    #[test]
    fn json_escapes_messages_and_term_strings() {
        let report = ValidationReport::new(vec![result(
            iri(&format!("{EX}alice")),
            None,
            None,
            "NodeConstraintComponent",
            "Violation",
            vec![lit("quoted \"line\"\nnext")],
        )]);
        let json = report.to_json();

        assert!(
            json.contains(r#""resultMessage": "quoted \"line\"\nnext""#),
            "{json}"
        );
        assert!(!json.contains("quoted \"line\"\nnext"));
        assert_eq!(escape_json("\\\t\u{1}"), "\\\\\\t\\u0001");
        assert_eq!(escape_json("\u{8}\u{c}\u{7f}"), "\\u0008\\u000c\u{7f}");
        assert_eq!(
            term_to_json_string(&Term::NamedNode(NamedNode::new_unchecked(
                "http://example.org/node"
            ))),
            "http://example.org/node"
        );
        assert_eq!(
            term_to_json_string(&Term::BlankNode(BlankNode::new_unchecked("node"))),
            "_:node"
        );
        assert_eq!(term_to_json_string(&lit("literal")), r#""literal""#);
        assert_eq!(
            term_to_json_string(&Term::Triple(Box::new(Triple::new(
                NamedNode::new_unchecked("http://example.org/subject"),
                NamedNode::new_unchecked("http://example.org/predicate"),
                Literal::new_simple_literal("object"),
            )))),
            r#"<<( <http://example.org/subject> <http://example.org/predicate> "object" )>>"#
        );
    }

    #[test]
    fn json_preserves_result_order() {
        let first = result(
            iri(&format!("{EX}first")),
            None,
            None,
            "MinCountConstraintComponent",
            "Violation",
            vec![],
        );
        let second = result(
            iri(&format!("{EX}second")),
            None,
            None,
            "MaxCountConstraintComponent",
            "Violation",
            vec![],
        );

        let forward = ValidationReport::new(vec![first.clone(), second.clone()]).to_json();
        let reversed = ValidationReport::new(vec![second, first]).to_json();
        assert_ne!(forward, reversed);
        assert!(forward.find("first").unwrap() < forward.find("second").unwrap());
    }

    #[test]
    fn json_emits_diagnostics_in_fixed_shape() {
        let report = ValidationReport {
            conforms: true,
            results: Vec::new(),
            diagnostics: vec![ShapeDiagnostic {
                source_shape: iri(&format!("{EX}Shape")),
                source_component: format!("{SH}PatternConstraintComponent"),
                message: "pattern did not compile".into(),
            }],
        };

        assert_eq!(
            report.to_json(),
            concat!(
                "{\n",
                "  \"conforms\": true,\n",
                "  \"results\": [],\n",
                "  \"diagnostics\": [\n",
                "    {\n",
                "      \"sourceShape\": \"http://example.org/Shape\",\n",
                "      \"sourceConstraintComponent\": ",
                "\"http://www.w3.org/ns/shacl#PatternConstraintComponent\",\n",
                "      \"message\": \"pattern did not compile\"\n",
                "    }\n",
                "  ]\n",
                "}"
            )
        );
    }

    #[test]
    fn turtle_emits_path_value_and_message_branches() {
        // A property-shape result WITH a path, value and a declared message.
        let r = ValidationReport::new(vec![result(
            iri(&format!("{EX}alice")),
            Some(Path::Predicate(format!("{EX}age"))),
            Some(lit("thirty")),
            "DatatypeConstraintComponent",
            "Violation",
            vec![lit("bad age")],
        )]);
        assert!(!r.conforms);
        let ttl = r.to_turtle();
        assert!(ttl.contains("sh:conforms false"));
        assert!(ttl.contains("sh:resultPath"), "path branch: {ttl}");
        assert!(ttl.contains("sh:value"), "value branch: {ttl}");
        assert!(
            ttl.contains(r#"sh:resultMessage "bad age""#),
            "declared message: {ttl}"
        );
        assert!(ttl.contains("DatatypeConstraintComponent"));
        let triples = parse_report(&ttl);
        assert!(triples
            .iter()
            .any(|t| t.predicate.as_str().ends_with("resultPath")));
        assert!(triples
            .iter()
            .any(|t| t.predicate.as_str().ends_with("value")));
    }

    #[test]
    fn turtle_omits_absent_path_and_value() {
        // A node-shape / count result: NO path, NO value, NO declared message —
        // the default message is emitted instead.
        let r = ValidationReport::new(vec![result(
            iri(&format!("{EX}bob")),
            None,
            None,
            "MinCountConstraintComponent",
            "Violation",
            vec![],
        )]);
        let ttl = r.to_turtle();
        assert!(!ttl.contains("sh:resultPath"), "no path expected: {ttl}");
        assert!(!ttl.contains("sh:value"), "no value expected: {ttl}");
        // effective_messages falls back to the default message.
        assert!(
            ttl.contains(r#"sh:resultMessage "generated default""#),
            "{ttl}"
        );
        parse_report(&ttl);
    }

    #[test]
    fn turtle_handles_multiple_results() {
        let r = ValidationReport::new(vec![
            result(
                iri(&format!("{EX}a")),
                None,
                None,
                "MinCountConstraintComponent",
                "Violation",
                vec![],
            ),
            result(
                iri(&format!("{EX}b")),
                Some(Path::Predicate(format!("{EX}p"))),
                Some(iri(&format!("{EX}v"))),
                "ClassConstraintComponent",
                "Warning",
                vec![],
            ),
        ]);
        let ttl = r.to_turtle();
        let triples = parse_report(&ttl);
        // Two sh:result links from the report node.
        let n_results = triples
            .iter()
            .filter(|t| t.predicate.as_str().ends_with("#result"))
            .count();
        assert_eq!(n_results, 2, "{ttl}");
    }

    // (sq-qcuhh) Value-pinned direct coverage for the public N-Triples
    // serializer: two violations must remain two report links and conformity must
    // be the boolean false in the emitted report graph.
    #[test]
    fn ntriples_emits_two_result_nonconforming_report_graph() {
        let report = ValidationReport::new(vec![
            result(
                iri(&format!("{EX}alice")),
                None,
                None,
                "MinCountConstraintComponent",
                "Violation",
                vec![],
            ),
            result(
                iri(&format!("{EX}bob")),
                Some(Path::Predicate(format!("{EX}age"))),
                Some(lit("thirty")),
                "DatatypeConstraintComponent",
                "Violation",
                vec![],
            ),
        ]);

        assert!(!report.conforms);
        assert_eq!(report.results.len(), 2);
        let nt = report.to_ntriples();
        assert!(
            nt.lines()
                .filter(|line| !line.trim().is_empty())
                .all(|line| line.trim_end().ends_with(" .")),
            "{nt}"
        );

        let triples = oxttl::NTriplesParser::new()
            .for_slice(nt.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_else(|error| panic!("report N-Triples does not parse: {error}\n{nt}"));
        let report_type = format!("{SH}ValidationReport");
        let report_nodes: Vec<_> = triples
            .iter()
            .filter(|triple| {
                triple.predicate.as_str() == "http://www.w3.org/1999/02/22-rdf-syntax-ns#type"
                    && matches!(&triple.object, Term::NamedNode(node) if node.as_str() == report_type)
            })
            .map(|triple| triple.subject.clone())
            .collect();
        assert_eq!(report_nodes.len(), 1, "{nt}");
        let report_node = &report_nodes[0];

        let conforms: Vec<_> = triples
            .iter()
            .filter(|triple| {
                &triple.subject == report_node
                    && triple.predicate.as_str() == format!("{SH}conforms")
            })
            .collect();
        assert_eq!(conforms.len(), 1, "{nt}");
        assert!(
            matches!(&conforms[0].object, Term::Literal(literal) if literal.value() == "false"),
            "{nt}"
        );
        assert_eq!(
            triples
                .iter()
                .filter(|triple| {
                    &triple.subject == report_node
                        && triple.predicate.as_str() == format!("{SH}result")
                })
                .count(),
            2,
            "{nt}"
        );
    }

    #[test]
    fn text_rendering_shortens_iris_and_uses_messages() {
        let r = ValidationReport::new(vec![
            result(
                iri(&format!("{EX}alice")),
                Some(Path::Predicate(format!("{EX}age"))),
                Some(lit("thirty")),
                "DatatypeConstraintComponent",
                "Violation",
                vec![lit("custom msg")],
            ),
            result(
                iri(&format!("{EX}bob")),
                None,
                None,
                "MinCountConstraintComponent",
                "Warning",
                vec![],
            ),
        ]);
        let text = r.to_text();
        assert!(text.starts_with("Does not conform: 2 result(s)"), "{text}");
        // Severity + component are rendered as their local names.
        assert!(text.contains("[Violation]"), "{text}");
        assert!(text.contains("[Warning]"), "{text}");
        assert!(text.contains("DatatypeConstraintComponent:"), "{text}");
        assert!(text.contains("MinCountConstraintComponent:"), "{text}");
        // path + value rendered when present.
        assert!(text.contains("| path "), "{text}");
        assert!(text.contains("| value "), "{text}");
        // Declared message wins for the first; default for the second.
        assert!(text.contains("custom msg"), "{text}");
        assert!(text.contains("generated default"), "{text}");
    }

    #[test]
    fn effective_messages_default_and_declared() {
        let no_msg = result(
            iri(&format!("{EX}x")),
            None,
            None,
            "MinCountConstraintComponent",
            "Violation",
            vec![],
        );
        let m = no_msg.effective_messages();
        assert_eq!(m.len(), 1);
        assert!(matches!(&m[0], Term::Literal(l) if l.value() == "generated default"));

        let with_msg = result(
            iri(&format!("{EX}x")),
            None,
            None,
            "MinCountConstraintComponent",
            "Violation",
            vec![lit("m1"), lit("m2")],
        );
        let m = with_msg.effective_messages();
        assert_eq!(m.len(), 2);
        assert_eq!(m, vec![lit("m1"), lit("m2")]);
    }

    // (sq-f8gu) A result carrying sh:detail sub-results nests each as a
    // sh:ValidationResult blank node under sh:detail; both Turtle and text render
    // them, and the Turtle round-trips.
    #[test]
    fn turtle_and_text_nest_detail_sub_results() {
        let mut top = result(
            iri(&format!("{EX}list")),
            None,
            Some(iri(&format!("{EX}list"))),
            "MemberShapeConstraintComponent",
            "Violation",
            vec![],
        );
        top.details = vec![
            result(
                iri(&format!("{EX}list")),
                None,
                Some(lit("bad1")),
                "NodeKindConstraintComponent",
                "Violation",
                vec![],
            ),
            result(
                iri(&format!("{EX}list")),
                None,
                Some(lit("bad2")),
                "NodeKindConstraintComponent",
                "Violation",
                vec![],
            ),
        ];
        let r = ValidationReport::new(vec![top]);
        let ttl = r.to_turtle();
        assert!(ttl.contains("sh:detail"), "{ttl}");
        let triples = parse_report(&ttl);
        // Two sh:detail links, and three sh:ValidationResult nodes (top + 2 details).
        assert_eq!(
            triples
                .iter()
                .filter(|t| t.predicate.as_str().ends_with("#detail"))
                .count(),
            2,
            "{ttl}"
        );
        assert_eq!(
            triples
                .iter()
                .filter(|t| t.predicate.as_str().ends_with("#type")
                    && t.object.to_string().ends_with("ValidationResult>"))
                .count(),
            3,
            "{ttl}"
        );
        // The text rendering surfaces the nested details.
        let text = r.to_text();
        assert_eq!(
            text.matches("detail: NodeKindConstraintComponent").count(),
            2,
            "{text}"
        );
    }

    #[test]
    fn severity_helpers() {
        let r = ValidationReport::new(vec![
            result(
                iri(&format!("{EX}a")),
                None,
                None,
                "MinCountConstraintComponent",
                "Warning",
                vec![],
            ),
            result(
                iri(&format!("{EX}b")),
                None,
                None,
                "MaxCountConstraintComponent",
                "Info",
                vec![],
            ),
        ]);
        // (sq-sx15d) sh:conforms uses the DEFAULT disallowed set
        // {Violation, Warning, Info}: a Warning/Info report does NOT conform.
        // The violations-only toggle is strictly weaker and passes here (no
        // sh:Violation).
        assert!(!r.conforms);
        assert!(r.conforms_violations_only());
        assert_eq!(r.results_with_severity(&format!("{SH}Warning")).count(), 1);
        assert_eq!(r.results_with_severity(&format!("{SH}Info")).count(), 1);
        assert_eq!(
            r.results_with_severity(&format!("{SH}Violation")).count(),
            0
        );

        // Add a Violation: the toggle now fails too.
        let r = ValidationReport::new(vec![result(
            iri(&format!("{EX}c")),
            None,
            None,
            "NodeConstraintComponent",
            "Violation",
            vec![],
        )]);
        assert!(!r.conforms_violations_only());
    }

    // (sq-sx15d) SHACL-1.2 conformance threshold: results below
    // sh:Info (sh:Debug / sh:Trace) are reported but DO NOT break the default
    // `conforms`; a custom disallowed set is applied via
    // `conforms_with_disallowed`.
    #[test]
    fn severity_threshold_default_and_custom_disallowed() {
        // A Debug-only and a Trace-only report each conform under the default set.
        for sev in ["Debug", "Trace"] {
            let r = ValidationReport::new(vec![result(
                iri(&format!("{EX}x")),
                None,
                None,
                "DatatypeConstraintComponent",
                sev,
                vec![],
            )]);
            assert!(r.conforms, "a {sev}-only report must conform by default");
            // Still reported (one result), just below the threshold.
            assert_eq!(r.results.len(), 1);
        }

        // A Warning result breaks the default conformance...
        let r = ValidationReport::new(vec![result(
            iri(&format!("{EX}w")),
            None,
            None,
            "DatatypeConstraintComponent",
            "Warning",
            vec![],
        )]);
        assert!(!r.conforms);
        // ...but conforms under the custom set {sh:Violation} only.
        assert!(r.conforms_with_disallowed(&[&format!("{SH}Violation")]));
        // The default set is exactly {Violation, Warning, Info}.
        assert_eq!(DEFAULT_CONFORMANCE_DISALLOWS.len(), 3);
        assert!(DEFAULT_CONFORMANCE_DISALLOWS.contains(&format!("{SH}Violation").as_str()));
        assert!(DEFAULT_CONFORMANCE_DISALLOWS.contains(&format!("{SH}Info").as_str()));
        // A custom set that disallows Debug catches a Debug result.
        let r = ValidationReport::new(vec![result(
            iri(&format!("{EX}d")),
            None,
            None,
            "DatatypeConstraintComponent",
            "Debug",
            vec![],
        )]);
        assert!(r.conforms);
        assert!(!r.conforms_with_disallowed(&[&format!("{SH}Debug")]));
    }
}
