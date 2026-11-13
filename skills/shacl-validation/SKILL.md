---
name: shacl-validation
description: "Validate RDF data against SHACL shapes with the sparq engine: SHACL Core constraints (class, datatype incl. the SHACL-1.2 disjunctive list form, cardinality, ranges, paths, logical, node/property, qualified, closed incl. sh:ByTypes, in/hasValue, the SHACL-1.2 list constraints sh:memberShape / sh:uniqueMembers / sh:min+maxListLength / sh:uniqueValuesFor, and the SHACL-1.2 value constraints sh:subsetOf / sh:someValue / sh:singleLine / sh:rootClass with path-valued sh:equals/disjoint/lessThan comparands and severity-threshold sh:conforms), SHACL-SPARQL sh:sparql constraints (§5.2), and custom SPARQL-based constraint components (sh:ConstraintComponent, §6) — then read the conformance/violations validation report as N-Triples, deterministic JSON, W3C report-vocabulary Turtle, or human text. Also runs opt-in SHACL Advanced Features (SHACL-AF) rules — sh:rule (sh:TripleRule + sh:SPARQLRule) — to INFER triples (feature `shacl-af`), and assembles the shapes graph itself — sh:shapesGraph discovery + transitive owl:imports closure via a caller-supplied loader (feature `imports`). Use when an agent needs to check whether a sparq_core::Graph conforms to shapes, run shape validation, produce a SHACL validation report, assemble a shapes graph from sh:shapesGraph/owl:imports references, or apply SHACL rules to infer/expand a graph (or a dataset over explicit named-graph scopes) in Rust."
---

# sparq-shacl-validation

Validate a data `Graph` against a shapes `Graph` and get back a `ValidationReport`
(conformance flag + per-violation results, renderable as N-Triples, deterministic
JSON, W3C-vocabulary Turtle, or plain text). Covers the full SHACL Core component set,
SHACL-SPARQL (`sh:sparql`, §5.2), and custom SPARQL-based constraint components
(`sh:ConstraintComponent`, §6).

`sparq-shacl` is an **opt-in** crate: depending on it is what turns on SHACL. It is
NOT a dependency of any other sparq crate by default, so the core engine and the
default wasm bundle carry zero SHACL code/cost unless you pull it in. The browser/JS
consumer opts in through `sparq-wasm`'s non-default `shacl` feature, which exposes
`validate` as a stateless `Store.validate(data, shapes, format)` wasm binding
returning a JSON report — a drop-in for `rdf-validate-shacl` (sq-yqi1, #162) — plus
the store-backed `Store.validateStore(shapes, format)`, which validates the triples
the store already holds (same report, only the shapes are re-parsed; gh-2520). On that
wasm32 build the `sparq-engine` dep drops its defaults so rayon never enters the
bundle; see the `javascript-wasm` skill for the JS API + report shape.

For the showcase site there is also a **standalone, lazy-loaded** wasm bundle,
`sparq-shacl-wasm` (the tier-b "W-shacl" artifact, sq-lfmf), kept separate from the lean
default bundle so SHACL never ships on the landing page. It exposes a stateless
`Validator` with the FULL report surface — `Validator.validate(data, shapes, format)`
(JSON report), `validateTurtle` (report-RDF in the `sh:ValidationReport` vocabulary),
`validateText` (human-readable), and `conforms(..., violationsOnly)` (the W3C
`sh:conforms` flag, or a violations-only gate). SHACL-AF `sh:rule` validation is behind
its opt-in `shacl-af` feature; for repeat validation without re-parse (a directional,
non-canonical measurement found data-graph parsing dominating the one-shot at
scale-tier corpora — sq-01xlp, `research/shacl-wasm-stateful-2026-07.md`) its opt-in
`stateful` feature adds a
pre-parsed `ParsedGraph` handle — parse once, validate many times, same report
surface. See `crates/sparq-shacl-wasm/README.md`.

Query rewrites and pre-binding retain VERSION announcements and their
[version-pinned EBV rules](../sparql-query/ebv-dialects.md). Unknown labels follow
the surface's existing query/ill-formed-input error policy.

## Quickstart

`Cargo.toml`:

```toml
[dependencies]
sparq-core  = { path = "../sparq-core" }   # or version = "0.1"
sparq-shacl = { path = "../sparq-shacl" }  # or version = "0.1"
```

```rust
use sparq_core::Graph;

let data = Graph::load_str(r#"
    @prefix ex: <http://example.org/> .
    ex:alice a ex:Person ; ex:age "thirty" .   // age is a string, not an integer
"#, "turtle").unwrap();

let shapes = Graph::load_str(r#"
    @prefix sh:  <http://www.w3.org/ns/shacl#> .
    @prefix ex:  <http://example.org/> .
    @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
    ex:PersonShape a sh:NodeShape ;
      sh:targetClass ex:Person ;
      sh:property [ sh:path ex:age ; sh:datatype xsd:integer ; sh:minCount 1 ] .
"#, "turtle").unwrap();

let report = sparq_shacl::validate(&data, &shapes);
assert!(!report.conforms);                 // sh:conforms = false
assert_eq!(report.results.len(), 1);       // one DatatypeConstraintComponent violation
eprintln!("{}", report.to_text());         // human-readable
println!("{}", report.to_turtle());        // W3C sh:ValidationReport graph
println!("{}", report.to_ntriples());      // same graph, one N-Triples statement per line
println!("{}", report.to_json());          // deterministic machine-readable object
```

CLI-style end-to-end run via the bundled example (exits 0 iff the data conforms):

```sh
cargo run -p sparq-shacl --example validate -- data.ttl shapes.ttl
cargo run -p sparq-shacl --example validate -- data.ttl shapes.ttl --turtle
```

## SPARQL-RL (opt-in)

The default-off `sparql-rl` feature exposes parsing, inScope/inUse checks,
stratification and `infer`/`infer_with_limits`. Inference returns only new
triples and uses one incremental working graph; IMPORTS is refused. It adds no
dependencies and is independent of `shacl-af`. See the [API, semantics and limits](references/sparql-rl.md).

## Key APIs

Top-level functions (`sparq_shacl::…`):

```rust
// Parse shapes + validate in one call.
pub fn validate(data: &Graph, shapes: &Graph) -> ValidationReport;

// Validate against an ALREADY-parsed shapes model (amortise parsing across many graphs).
pub fn validate_with_model(data: &Graph, model: &ShapesModel) -> ValidationReport;

// STRICT validation (sq-0mjfd): returns Err(ShaclFailure) for what a conformant
// processor REJECTS (the W3C `sht:Failure` outcome) — an unsound SHACL-SPARQL
// pre-binding (MINUS / SERVICE / VALUES mentioning a pre-bound name / AS
// re-binding one), and (sq-11a) an ILL-FORMED shapes-graph construct: an unparsable
// sh:path (or >1 sh:path values), a non-integer count/length (a NEGATIVE integer is
// well-formed and stays a silent skip), a literal sh:datatype/sh:class/sh:nodeKind/
// sh:pattern or a non-IRI list member, a malformed SHACL list (sh:in/and/or/xone/
// languageIn/ignoredProperties), a literal shape ref (sh:node/not/property/…), a
// non-boolean sh:closed/uniqueLang/…, a non-literal range comparand, an ill-formed
// comparand path (sh:equals/lessThan/…), a non-IRI sh:target{Class,SubjectsOf,
// ObjectsOf}, an sh:sparql node with no sh:select literal. (sq-ehq4g) adds: a
// PROPERTY shape with NO sh:path (an sh:property value, or a node typed
// sh:PropertyShape), sh:qualifiedValueShape with NEITHER sh:qualifiedMinCount nor
// sh:qualifiedMaxCount (both qualified components then miss a mandatory parameter),
// an sh:nodeKind value outside the supported sh:* kinds, and a PRESENT sh:select /
// sh:sparqlExpr (on sh:sparql constraints AND on the SPARQL-based node expressions
// of sh:targetNode/sh:values) whose text does not parse as the required query form
// — FAIL-CLOSED relative to this engine's vendored SPARQL parser: a valid query
// beyond the parser's coverage is also rejected strictly. (sq-c1v3e) adds: a non-IRI
// sh:severity (shape-level AND on an sh:SPARQLConstraint node), the count/length
// DATATYPE check (integer-lexical but non-xsd:integer-typed, e.g. "3"^^xsd:string —
// bare Turtle 3 types as xsd:integer, so ordinary graphs are unaffected), a
// qualified COUNT without sh:qualifiedValueShape (the symmetric partial-parameter
// case), ANY sh:entailment declaration (no entailment regime is supported — the
// SHACL §3.4 unsupported-regime failure), and (feature `shacl-af`) an
// sh:expression / sh:nodeByExpression structural node expression that does not
// build. Construct-local checks, NOT a full SHACL-of-SHACL pass;
// ShaclFailure.ill_formed / ShapesModel::ill_formed() carry (node, predicate,
// message). `validate` instead SKIPS all of the above unchanged (its never-fails
// contract) and (sq-c1v3e) surfaces each record as a ShapeDiagnostic in
// report.diagnostics (source_component = the offending SHACL predicate IRI).
pub fn validate_strict(data: &Graph, shapes: &Graph) -> Result<ValidationReport, ShaclFailure>;

// SHACL 1.0 Appendix A admission: also reject local VALUES and subqueries
// omitting a pre-bound name. validate_strict uses the SHACL 1.2 rules.
pub fn validate_strict_shacl10(data: &Graph, shapes: &Graph) -> Result<ValidationReport, ShaclFailure>;
// ShaclFailure.pre_binding carries admission failures and runtime solution-
// declared failures; its historical field name is retained.
pub fn validate_strict_with_model(data: &Graph, model: &ShapesModel)
    -> Result<ValidationReport, ShaclFailure>;

// Load Turtle resolving relative IRIs against a base (Graph::load_str has no base param).
pub fn load_turtle_with_base(text: &str, base: &str) -> Result<Graph, String>;

// Build a Graph from already-parsed oxrdf::Triples.
pub fn graph_from_triples<I: IntoIterator<Item = oxrdf::Triple>>(triples: I) -> Graph;

// Per-thread monotonic count of sh:sparql query executions (sq-7d3dj.33.1). Snapshot
// the delta across a `validate` call to assert focus-node batching fired (a small
// delta for a large focus set) — see "Focus-node batching" under SHACL-SPARQL below.
pub fn sparql_constraint_executions() -> u64;
```

`ValidationReport::conforms` honours a shapes-graph `sh:conformanceDisallows`
declaration (SHACL 1.2 Core §3.9, sq-5q76d) — e.g. a graph that disallows only
`sh:Violation` conforms despite a `sh:Warning` result — falling back to the default
{Violation, Warning, Info} set. `sh:reifierShape` / `sh:reificationRequired` validate
the RDF-1.2 reifiers of a value's asserted triple, and `sh:uniqueLang` keys on the
`rdf:dirLangString` base direction (`@ar`, `@ar--ltr`, `@ar--rtl` are distinct keys).

**SHACL Compact Syntax (SCS) parser** *(opt-in feature `scs`)* — the *parse*
direction of the W3C SCS (`sparq_shacl::scs::…`, re-exported at the crate root):

```rust
// Parse SCS text -> SHACL shapes triples (relative IRIs + the owl:Ontology subject
// resolve against `base`; pass DEFAULT_BASE for the no-`BASE` convention).
pub fn parse_scs(text: &str, base: &str) -> Result<Vec<oxrdf::Triple>, ScsError>;

// Same, then build a queryable Graph ready to feed `validate`.
pub fn parse_scs_to_graph(text: &str, base: &str) -> Result<Graph, ScsError>;

pub const DEFAULT_BASE: &str;   // "urn:x-base:default"
pub struct ScsError { pub line: usize, pub message: String }   // typed; never a silent mis-parse
```

It emits the SAME shapes triples `validate` consumes, so an SCS document validates
data identically to the equivalent Turtle. Covers the grammar the W3C `shacl12-cs`
corpus exercises (32/32 fixtures round-trip graph-isomorphically): directives,
`shape`/`shapeClass`, full path expressions, `[min..max]`, `nodeKind`, bare-IRI
`sh:datatype`-vs-`sh:class`, `@`shape-refs (`sh:node`), `param=value`, `!` (`sh:not`),
`|` (`sh:or`), nested `{...}` shapes (`sh:node`), and `[ ... ]` arrays (`sh:in` /
`sh:ignoredProperties`). The browser/JS surface exposes this as the opt-in
`Store.parseShaclCompact(text, base?)` wasm binding (sq-quly) — SCS text → the shapes
graph as a Turtle string, behind `sparq-wasm`'s non-default `scs` feature; see the
`javascript-wasm` skill for the JS API.

**Shapes-graph assembly: `sh:shapesGraph` + `owl:imports`** *(opt-in feature
`imports`, sq-uz0)* — W3C SHACL §§3.1/3.3 (shapes-graph) shapes-graph *discovery and union*, so
callers no longer hand-assemble the shapes graph. Dereferencing an IRI to a
document stays a caller concern (the engine never touches the network): you
supply a loader callback, the library owns the traversal, the per-IRI dedupe /
cycle guard, the RDF-merge discipline (each loaded document's blank nodes are
standardised apart so labels reused across documents never collapse), and the
final deduplicated union: Note the graph IRIs handed to your loader originate from (possibly untrusted) input data — a network-dereferencing loader should allowlist hosts/schemes.

```rust
// The union of every graph the data graph references via sh:shapesGraph, plus
// the transitive owl:imports closure of each loaded document (data triples are
// NOT included). Loader contract: Ok(Some(g)) = fetched, Ok(None) = cannot
// resolve (recorded in `unresolved`, not fatal — SHACL keeps imports support
// optional), Err = hard failure (aborts, naming the IRI).
pub fn resolve_shapes_graph(data: &Graph, loader: impl FnMut(&str) -> Result<Option<Graph>, String>)
    -> Result<ShapesGraphResolution, String>;

// The caller already HAS a shapes graph: its own triples (blank labels intact)
// unioned with its transitive, cycle-guarded owl:imports closure.
pub fn resolve_imports(seed: &Graph, loader: impl FnMut(&str) -> Result<Option<Graph>, String>)
    -> Result<ShapesGraphResolution, String>;

pub struct ShapesGraphResolution {
    pub shapes: Graph,           // feed this to `validate` / `ShapesModel::parse`
    pub resolved: Vec<String>,   // IRIs loaded, breadth-first traversal order
    pub unresolved: Vec<String>, // IRIs the loader declined (honest record)
}
```

**Generated SCS 1.2 + extended parser — the `sparq-shaclc` crate** *(opt-in by
being a separate crate; epic sq-tonhr)* — rdf-shuttle-generated strict/extended
parsers from one Shuttle grammar, COEXISTING with (not replacing) the `scs`
feature above and differential-tested against it:

```rust
use sparq_shaclc::{parse, parse_strict, parse_extended, Profile, DEFAULT_BASE};
// -> Result<(Vec<oxrdf::Triple>, Outcome), ShaclcError>; Outcome carries
//    prefixes (5 predeclared first) + final base. Profile::Strict = W3C CG
//    surface + RDF 1.2 layer (triple terms, dir-lang tags, TripleTerm
//    nodeKind, reifierShape/reificationRequired) and PROVABLY rejects the
//    four shaclc-js extensions; Profile::Extended accepts them.
// sparq_shaclc::raw::{shaclc12, shaclc12ext} — streaming + chunked push
// parsing on the generated zero-dependency term model.
use sparq_shaclc::write; // (triples, base, prefixes, Profile) -> Result<String, ShaclcWriteError>
// The derived residual-consumption printer (first Rust-side SHACL-CS
// writer): all-or-nothing — a non-expressible graph returns the typed
// residual verdict (exact unconsumed triples), never a lossy document.
```

`ShapesModel` (`sparq_shacl::ShapesModel`):

```rust
pub fn ShapesModel::parse(shapes_graph: &Graph) -> ShapesModel;   // parse once, reuse
```

`ValidationReport` (`sparq_shacl::ValidationReport`):

```rust
pub conforms: bool;                         // SHACL-1.2 sh:conforms: false iff any result is in the
                                            // default disallowed set {Violation,Warning,Info}; Debug/Trace conform
pub results: Vec<ValidationResult>;
pub diagnostics: Vec<ShapeDiagnostic>;      // skipped-constraint diagnostics (e.g. uncompilable sh:pattern); never affect `conforms`
pub fn conforms_violations_only(&self) -> bool;                       // stricter-threshold toggle: ignore sh:Warning / sh:Info
pub fn conforms_with_disallowed(&self, disallowed: &[&str]) -> bool;  // custom sh:conformanceDisallows set (full severity IRIs)
pub fn results_with_severity<'a>(&'a self, severity: &'a str)         // full IRI, e.g. ".../shacl#Warning"
    -> impl Iterator<Item = &'a ValidationResult>;
pub fn to_turtle(&self) -> String;          // W3C report vocabulary (valid, round-trippable Turtle)
pub fn to_ntriples(&self) -> String;        // same report graph, one N-Triples statement per line
pub fn to_text(&self) -> String;            // human-readable summary
pub fn to_json(&self) -> String;            // deterministic JSON; fixed keys and result order
pub fn to_js_json(&self) -> String;         // compact JSON the wasm bindings return (focusNode/path/value/…/message; null when absent)
```

`ValidationResult` (`sparq_shacl::ValidationResult`) — all fields public:

```rust
pub focus_node: oxrdf::Term;
pub path: Option<sparq_shacl::Path>;        // sh:resultPath (property shapes / sh:closed)
pub value: Option<oxrdf::Term>;             // offending value node
pub source_shape: oxrdf::Term;
pub source_constraint: Option<oxrdf::Term>; // sh:sourceConstraint — the sh:SPARQLConstraint node (sh:sparql results only; None for Core + §6 components)
pub source_component: String;               // constraint-component IRI, e.g. ".../MinCountConstraintComponent"
pub severity: String;                       // severity IRI (default ".../shacl#Violation")
pub messages: Vec<oxrdf::Term>;             // sh:message literals
pub default_message: String;
pub details: Vec<ValidationResult>;         // nested sh:detail sub-results (see below); empty for most components
pub fn effective_messages(&self) -> Vec<oxrdf::Term>;   // messages, or a generated default
pub fn message_text(&self) -> &str;                     // first sh:message's text, or the generated default
```

`details` carries non-normative `sh:detail` sub-results that explain WHY a result
fired: a `sh:memberShape` violation lists one sub-result per non-conforming list
member (the actual results of validating that member against the member shape),
and a `sh:uniqueMembers` violation lists one sub-result per duplicated member
(`sh:value` = the duplicated term). `sh:detail` is non-normative — it never
affects `sh:conforms` and the W3C suite compares only top-level result fields —
so it is empty for every other component. `to_turtle` nests each detail as a
`sh:ValidationResult` blank node under `sh:detail`; `to_text` indents them.

`Path` (`sparq_shacl::Path`) — `Predicate | Inverse | Sequence | Alternative |
ZeroOrMore | OneOrMore | ZeroOrOne`; `path.to_turtle()` gives the Turtle path
expression used in `sh:resultPath`.

## Common recipes

**CI gating — fail on violations, allow warnings.** `report.conforms` follows the
SHACL-1.2 default (also disallows `sh:Warning`/`sh:Info`); use the stricter-threshold
toggle so only `sh:Violation` fails the build (or `conforms_with_disallowed` for a
custom `sh:conformanceDisallows` set):

```rust
let report = sparq_shacl::validate(&data, &shapes);
if !report.conforms_violations_only() {
    eprintln!("{}", report.to_text());
    std::process::exit(1);
}
```

**Validate many data graphs against one shapes graph** — parse the shapes once:

```rust
let model = sparq_shacl::ShapesModel::parse(&shapes);
for data in data_graphs {
    let report = sparq_shacl::validate_with_model(&data, &model);
    // ...
}
```

**Inspect failures programmatically** instead of rendering:

```rust
for r in &report.results {
    let comp = r.source_component.rsplit(['#', '/']).next().unwrap();   // "MinCountConstraintComponent"
    println!("focus={} comp={comp} value={:?}", r.focus_node, r.value);
}
```

**SHACL-SPARQL (`sh:sparql`, §5.2)** — a constraint node carries an `sh:select`; it
runs per focus node with `$this` pre-bound (and `$PATH` on property shapes), and EACH
returned solution with `failure` other than boolean true is one violation. A true
`failure` binding makes strict validation return `Err`; lenient validation records
a diagnostic and omits that row. `?value`→`sh:value` (defaults to the focus node
when unprojected), `?path`→`sh:resultPath`, `?message`→`sh:resultMessage`; `{?var}` /
`{$var}` templating in messages. A bound `?message` takes priority over constraint
or validator messages, then component messages, then shape messages. Language tags
are retained. `$currentShape` and `$shapesGraph` are pre-bound for `sh:sparql`;
the latter identifies a queryable named graph containing the supplied shapes.
Prefix declarations follow `owl:imports` and inverse `owl:versionIRI`; absent
`sh:prefixes`, declarations on ontology/data/shapes/rules graph owners are used.
For example:

```rust
let shapes = Graph::load_str(r#"
    @prefix sh:  <http://www.w3.org/ns/shacl#> .
    @prefix ex:  <http://example.org/> .
    @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
    ex:PersonShape a sh:NodeShape ;
      sh:targetClass ex:Person ;
      sh:sparql [
        a sh:SPARQLConstraint ;
        sh:prefixes ex:p ;
        sh:message "Age must not be negative" ;
        sh:select """SELECT $this ?value WHERE {
            $this <http://example.org/age> ?value . FILTER (?value < 0) }""" ;
      ] .
    ex:p sh:declare [ sh:prefix "ex" ; sh:namespace "http://example.org/"^^xsd:anyURI ] .
"#, "turtle").unwrap();
let report = sparq_shacl::validate(&data, &shapes);   // source_component ends with "SPARQLConstraintComponent"
```

*Focus-node batching (perf, sq-7d3dj.33.1).* Semantically each `sh:sparql` constraint
is "run per focus node", but the engine evaluates it for **all** of a shape's focus
nodes in ONE query: a single multi-row `VALUES ?this { … }` is injected (chunked at
10 000 foci), executed once, and the solution rows are grouped by `?this` to build the
per-focus results. This replaces the old O(N_focus × full-query) per-focus loop (which
re-materialised the whole BGP for every focus node — quadratic) with O(1) queries per
shape; the report is byte-for-byte identical. A constraint whose TOP-level form is
NOT per-focus-equivalent — a `LIMIT`/`OFFSET`, a `GROUP BY`/aggregate not keyed on
`$this` (an implicit single group or `GROUP BY ?other`), or `REDUCED` — falls back to
the per-focus path automatically. Queries with subqueries, EXISTS or nested
solution modifiers also use the per-focus path, as do queries using
`$currentShape` or `$shapesGraph`. `sparq_shacl::sparql_constraint_executions()`
exposes a per-thread `sh:sparql` query-execution counter (snapshot the delta across a
`validate` call) so a perf guard can assert the batched path fired.

**SHACL-1.2 core constraints (always on, no feature flag).** The disjunctive
*set* spellings of `sh:datatype` / `sh:nodeKind` — `sh:datatype ( xsd:string
rdf:langString )`, `sh:nodeKind ( sh:BlankNode sh:IRI )` — conform a value node
when it matches ANY listed datatype / kind (the single-IRI form is the singleton
case). `sh:closed sh:ByTypes` is the "close by types" mode: the allowed-predicate
set is recomputed per value node from its `rdf:type`s (transitively through
`rdfs:subClassOf` / inbound `sh:targetClass` / `sh:node`, SHACL §4.8.1), unlike
`sh:closed true` which fixes it to the shape's own `sh:property` paths. The four
SHACL list constraints validate that each value node is a well-formed SHACL list:
`sh:memberShape` (every member conforms to a shape), `sh:uniqueMembers true`
(members pairwise distinct), `sh:min`/`sh:maxListLength` (member-count bounds),
and `sh:uniqueValuesFor` (the listed properties' values are unique across the
shape's target nodes — one IRI, or a SHACL list for a composite key). A value that
is not a well-formed SHACL list violates the list constraints; a node with no
values for any `sh:uniqueValuesFor` property is never reported. Each offending
value node produces one result without `sh:value`, regardless of how many peers
share the key. `sh:TripleTerm` is a supported node kind.

**SHACL-1.2 value constraints (always on, sq-sx15d).** `sh:class` also takes a
disjunctive SHACL-list object (`sh:class ( ex:A ex:B )` — a value conforms iff it is a
SHACL instance of ANY listed class, subclass-aware). The comparand of `sh:equals` /
`sh:disjoint` / `sh:lessThan` / `sh:lessThanOrEquals` — and the new `sh:subsetOf`
(path value set ⊆ comparand value set) — is a full SHACL property PATH (often an
RDF-list sequence `( ex:p ex:q )`), not just a predicate IRI; a bare IRI parses to a
trivial predicate path, so the SHACL-1.0 forms stay unchanged. `sh:someValue [ shape ]`
is EXISTENTIAL (at least one value node must conform to the nested shape; one result on
the focus/path when none do). `sh:singleLine true` flags string values containing a
line break (LF/CR/FF/VT). `sh:rootClass C` requires each value node to be `C` or a
transitive `rdfs:subClassOf`-descendant of it. A list-valued `sh:rootClass`
accepts an IRI equal to or below any listed class. Each nonconforming reifier
produces its own `sh:ReifierShapeConstraintComponent` result.

**SHACL-1.2 per-constraint-statement reified-annotation overrides (always on, sq-pb0wm).**
An RDF-1.2 reified annotation on a single constraint statement —
`ex:S sh:datatype xsd:integer {| sh:deactivated true |}` (likewise `{| sh:message … |}` /
`{| sh:severity … |}`) — overrides JUST that constraint occurrence, distinct from the
shape-level `sh:deactivated`/`sh:message`/`sh:severity` (which apply to the whole shape).
`{| sh:deactivated true |}` suppresses ONLY that constraint (the shape's other constraints
still validate); `{| sh:message "…"@en |}` sets `sh:resultMessage` for ONLY that
constraint's results; `{| sh:severity sh:Warning |}` sets `sh:resultSeverity` for ONLY that
constraint's violations. The `{| … |}` is parsed by oxttl's rdf-12 Turtle support and stored
as `_:r rdf:reifies <<( ex:S sh:datatype xsd:integer )>> . _:r sh:deactivated|message|severity V`;
the override resolves per occurrence from that reifier (`misc/{deactivated-003,message-002,
severity-003}`). Supported on single-statement Core constraints (`sh:datatype`, `sh:nodeKind`,
`sh:class`, `sh:hasValue`, `sh:rootClass`, `sh:node`, `sh:property`, `sh:not`, `sh:someValue`,
`sh:memberShape`); list-/path-valued operands are not single statements and carry no override.
On a RECURSING composite (`sh:node` / `sh:not` / `sh:someValue` / `sh:memberShape`) the
message/severity override governs the composite component's OWN result and survives the
nested shape evaluation (sq-1jemy); it does NOT govern the nested shape's results — those
carry the nested shape's own metas (the 1.2 severity precedence keys on the reifier of the
constraint statement that caused each result). On `sh:property` — which reports the nested
property shape's results directly, with no composite result — only `{| sh:deactivated |}`
is observable.

**SHACL-1.2 targets & SPARQL node expressions (always on, no feature flag, sq-rnkdh).**
Beyond `sh:targetNode`/`Class`/`SubjectsOf`/`ObjectsOf` + implicit class targets:
- **`sh:targetWhere [ <inline shape> ]`** — focus nodes are every data-graph node that
  CONFORMS to the inline (object) shape (conformance is checked through the validator).
- **`sh:shape`** — a DATA-graph triple `?n sh:shape ?S` makes `?n` a focus node of
  shape `?S` (the data-driven dual of `sh:targetNode`).
- **`sh:ShapeClass`** — a class that is ALSO a node shape; its instances (via the
  subclass closure) are implicit-class-targeted, no `rdfs:Class`+`sh:NodeShape` pair.
- **SPARQL-valued targets / value nodes** — `sh:targetNode [ sh:select "…" ]` computes
  focus nodes from the first result variable; on a property shape `sh:values [ sh:select
  "…" ]` / `[ sh:sparqlExpr "EXPR" ]` COMPUTES the value nodes (with `$this` = focus
  node) in addition to the nodes reached by `sh:path`. Constant IRI/literal
  `sh:values` also supplement the path. `sh:defaultValue` (constant or SPARQL
  expression) applies only when the union is empty; values are deduplicated.
  The reported `sh:resultPath` stays the path. SPARQL-valued `sh:deactivated`
  suppresses validation when its only output is boolean true.
- A constraint-level **`sh:severity`** on a `sh:SPARQLConstraint` overrides the shape's
  default severity for the results it produces.

`$shapesGraph` is bound to a fresh blank node naming the shapes graph for this
validation. That named graph is exposed only to queries referencing `$shapesGraph`;
other queries, including `GRAPH ?g`, see the original data dataset.

Validation installs a scoped engine function registry. It temporarily replaces any
ambient registry installed with `sparq_engine::with_functions`; ambient functions
are unavailable to SHACL queries and the previous registry is restored afterwards.

Custom SPARQL functions declared as `sh:ListParameterExpressionFunction` are
available during validation. Supported body expressions include `shnex:arg`,
`shnex:count`, `shnex:instancesOf`, `sh:select` and `sh:sparqlExpr`. A call must
produce exactly one node; blank arguments, missing required arguments and other
output cardinalities are SPARQL errors.

**Custom SPARQL-based constraint component (`sh:ConstraintComponent`, §6).** Declare
the component (parameters + an `sh:ask`/`sh:select` validator) IN THE SHAPES GRAPH; it
activates on any shape that uses all its mandatory parameter predicates. Each parameter
value is pre-bound as `$paramName` alongside `$this`/`$value`:

```rust
let shapes = Graph::load_str(r#"
    @prefix sh:   <http://www.w3.org/ns/shacl#> .
    @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
    @prefix ex:   <http://example.org/> .
    # Component typed via a subclass of sh:ConstraintComponent (discovery follows rdfs:subClassOf*).
    ex:MyCC rdfs:subClassOf sh:ConstraintComponent .
    ex:MaxLenComponent a ex:MyCC ;
      sh:parameter [ sh:path ex:maxLen ] ;
      sh:validator [ a sh:SPARQLAskValidator ;
        sh:message "Value is longer than {$maxLen} characters" ;
        sh:ask "ASK { FILTER (STRLEN(STR($value)) <= $maxLen) }" ] .
    ex:S a sh:NodeShape ;
      sh:targetNode "abcdef", "ab" ;
      ex:maxLen 3 .                # using ex:maxLen activates the component on ex:S
"#, "turtle").unwrap();
// source_component is the component IRI; ASK=false → violation. sh:nodeValidator /
// sh:propertyValidator are preferred over the generic sh:validator by shape kind (§6.2.2).
```

On a PROPERTY shape, a validator that references the `$PATH` variable gets it
pre-bound to the shape's property path (SHACL §6.3). Because `$PATH` is a SPARQL
property PATH (not a term), it is bound — like the §5.2 `sh:sparql` path — by
re-parsing the validator per property shape with the path's property-path form
textually substituted, rather than via the VALUES table the term bindings
(`$this` / `$value` / `$paramName`) use. The re-parsed per-shape validator is
held off the public `Component` enum in a crate-private store; the public
`Component::CustomSparql { component, args, path_validator }` variant carries
only an `Option<usize>` index into it (`path_validator`), present when the
shape is a property shape, the chosen validator references `$PATH`, and the
substituted query re-parses — otherwise `None` and the component's shared
(path-free) validator is used as-is. Each `$paramName` variable is the LOCAL
NAME of the parameter's `sh:path` IRI (not its `sh:name` display label, §6.2.1).

**Relative-IRI test files / a base IRI** — `Graph::load_str` exposes no base, so use:

```rust
let g = sparq_shacl::load_turtle_with_base(&text, &format!("file://{path}")).unwrap();
```

## Advanced features and validation details

See [Advanced Features](references/advanced-features.md) for SHACL-AF rules,
SHACL 1.2 inference rules and declared rule entailment, node-expression APIs and graph scopes, and [validation details](references/validation-details.md)
for constraint coverage, prerequisites and conformance ratchets. The 1.2 rule
executor rejects unsupported processors anywhere among the shapes graph's rules
and rule sets, including unselected and deactivated nodes and included sets.

## See also

- `sparql-query` — running standalone SPARQL through `sparq-engine` (what `sh:sparql`
  routes through).
- `data-formats` — building the `sparq_core::Graph` you validate.
- `fused-decompress-parse`, `hdt-format` — alternative ingest paths feeding a `Graph`.
