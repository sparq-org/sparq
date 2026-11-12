# SHACL Advanced Features

**SHACL Advanced Features rules (`sh:rule` + `sh:values`, SHACL-AF) — INFER
triples** *(opt-in feature `shacl-af`)*. A shape's rules infer new triples for that
shape's focus nodes (its targets). Three rule types: `sh:TripleRule` (`sh:subject`
/ `sh:predicate` / `sh:object` node expressions — the inferred triples are the
cartesian product of the three evaluated sets), `sh:SPARQLRule` (an `sh:construct`
CONSTRUCT run per focus node with `$this` pre-bound), and the `sh:values` value
rule (a property shape with a single-predicate `sh:path` and an `sh:values` node
expression infers `(focus, predicate, v)` per evaluated `v`). Rules honour
`sh:condition` (fire only for focus nodes conforming to every condition shape),
`sh:order` (ascending, a rule sees earlier groups' inferences), and
`sh:deactivated`. The engine **iterates to a fixpoint** (bounded by
`rules::MAX_ITERATIONS = 100`); the input graph is never mutated.

`Cargo.toml`: `sparq-shacl = { path = "...", features = ["shacl-af"] }`

```rust
use sparq_core::Graph;
let data = Graph::load_str(r#"
    @prefix ex: <http://example.org/> .
    ex:alice a ex:Person ; ex:firstName "Alice" .
"#, "turtle").unwrap();
let shapes = Graph::load_str(r#"
    @prefix sh:  <http://www.w3.org/ns/shacl#> .
    @prefix ex:  <http://example.org/> .
    @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
    ex:PersonShape a sh:NodeShape ;
      sh:targetClass ex:Person ;
      sh:rule [ a sh:TripleRule ;                  # infer (this, rdf:type, ex:Agent)
        sh:subject sh:this ; sh:predicate rdf:type ; sh:object ex:Agent ] ;
      sh:rule [ a sh:SPARQLRule ;                  # infer a label from the first name
        sh:construct "CONSTRUCT { $this <http://example.org/label> ?n } WHERE { $this <http://example.org/firstName> ?n }" ] .
"#, "turtle").unwrap();

// The INFERRED triples only (data is not mutated):
let inf = sparq_shacl::apply_rules(&data, &shapes);   // -> sparq_shacl::Inference
//   inf.triples : Vec<oxrdf::Triple>   inf.iterations : usize   inf.capped : bool
// Or get a fresh graph of data ∪ inferred, ready to query/validate:
let expanded: Graph = sparq_shacl::expand(&data, &shapes);

// Or select the validation fact domain directly. These APIs are also gated by
// `shacl-af`; the existing `validate` function remains asserted-only.
use sparq_shacl::{validate_with_domain, FactDomain};
let asserted = validate_with_domain(&data, &shapes, FactDomain::Asserted);
let closure = validate_with_domain(&data, &shapes, FactDomain::AssertedPlusInferred);
```

**Node-expression algebra** (operand of `sh:subject`/`sh:predicate`/`sh:object`,
the `sh:values` value rule, and `sh:expression`): `sh:this` (focus node); a
constant IRI/literal; a path expression `[ sh:path P ; sh:nodes N? ]` (any SHACL
property path; the optional `sh:nodes` is itself a node expression giving the start
nodes, default `sh:this`); a filter-shape expression `[ sh:filterShape S ; sh:nodes
N ]` (the nodes of `N` conforming to shape `S`); `[ sh:intersection ( … ) ]`; `[
sh:union ( … ) ]`; a bare `rdf:list` (a SHACL 1.2 list expression — its members in
order, preserving duplicates); and the **function-expression form** (sq-mk9n). These
nest.

**Function registry (sq-mk9n):** the SHACL 1.2 built-in node-expression operators
(`shnex:`/`sh:`) — `concat`, `count`, `sum`, `min`, `max`, `distinct`,
`if`/`then`/`else`, `exists`, `limit`, `offset`, `instancesOf`, `nodesMatching`,
`flatMap`, `findFirst`, `matchAll`, `remove`, `orderBy`, `var` (`"focusNode"` ⇒
the focus; any other name resolves against a caller-supplied `Scope`, sq-u5rxj) —
plus a custom `sh:SPARQLFunction` IRI applied to a `sh:list` of arguments
(dispatched through the SPARQL engine with the ordered `sh:parameter` variables
pre-bound). An unregistered function IRI is dropped (lenient), inferring nothing.

**Caller-supplied variable scope (sq-u5rxj):** `eval_node_expression_with_scope(data,
shapes, expr, focus, &Scope)` threads a `Scope` (`FxHashMap<String, Vec<Term>>`) of
variable name → bound node set that a `shnex:var "<name>"` resolves against (the W3C
suite's `sht:scope-<name>` injection); `eval_node_expression` is the empty-scope
wrapper.

**`sh:values` value rule:** a property shape with a single-predicate `sh:path` and
an `sh:values` node expression infers `(focus, predicate, v)` for each evaluated
`v`. A value rule on a `sh:property` child of a targeted node shape ranges over the
parent's focus nodes.

**`sh:expression` constraint** (`sh:ExpressionConstraintComponent`): a value node
violates when its `sh:expression` node expression does NOT evaluate to `{ true }`
(value = focus on a node shape; each path value on a property shape).

Expression constraints evaluate using the original shape focus and bind the
current value node to the scope variable `value`.

**`sh:nodeByExpression` constraint** (`sh:NodeByExpressionConstraintComponent`):
like `sh:node`, but the node shape is *computed* by a node expression. For each
value node `v`, the expression is evaluated against `v` as focus to a set of
node-shape terms; `v` violates when it does NOT conform to one of them. A constant
IRI expression is the `sh:node` special case; an expression result naming no parsed
shape is skipped (lenient).

**Explicit graph scopes + dataset-preserving expansion (gh-6614, a capability
addition — not a conformance fix).** `apply_rules` / `expand` see the DEFAULT graph
only and `expand` drops named graphs (unchanged). For a dataset (`Graph` with named
graphs, e.g. from `Graph::load_dataset(.., "nquads")`) choose the input scope and the
derived-triple destination explicitly:

```rust
use sparq_shacl::{apply_rules_in_scope, expand_dataset, Destination, GraphScope};
let g = |s: &str| oxrdf::Term::NamedNode(oxrdf::NamedNode::new(s).unwrap());
// Scope: GraphScope::Default | Named(name) | Union { default: bool, named: Vec<Term> }
let scope = GraphScope::Union { default: false, named: vec![g("urn:ex:g1"), g("urn:ex:g2")] };
let inf = apply_rules_in_scope(&data, &shapes, &scope);           // -> Inference
let dest = oxrdf::NamedOrBlankNode::NamedNode(oxrdf::NamedNode::new("urn:ex:derived").unwrap());
let out = expand_dataset(&data, &shapes, &scope, &Destination::Named(dest)); // IRI or blank node only
// out.dataset: Graph (ALL asserted graphs preserved, derived triples in the destination)
// out.inference: Inference (triples / iterations / capped — diagnostics are not hidden)
```

Semantics: the selected graphs are merged into the working default graph that
targets, paths, `sh:condition`, node expressions and SPARQL rules evaluate against
(cross-graph joins work); the selected NAMED graphs are also the only ones a
`sh:SPARQLRule` body reaches via `GRAPH <g>` / `GRAPH ?g` — an unselected graph is
indistinguishable from an absent one, and `FROM` / `FROM NAMED` cannot widen that.
Nothing is unioned implicitly (`Union { default: true, .. }` is the only way to add
the default graph to a union); an absent graph name is the empty graph. Inferred
triples feed later passes through the working scope regardless of `Destination`.
Materialization dedups against what the destination already asserts (an identical
triple in another graph is kept there); a `Destination::Named` graph the dataset
lacks is appended. The input is never mutated. `GraphScope::Default` ≡ `apply_rules`.

API: `apply_rules(data, shapes)`, `apply_rules_with_model(data, shapes, &model)`
(amortise shape parsing), `expand(data, shapes) -> Graph`,
`apply_rules_in_scope(data, shapes, &GraphScope)` /
`apply_rules_in_scope_with_model(data, shapes, &model, &GraphScope)`,
`expand_dataset(data, shapes, &GraphScope, &Destination) -> DatasetExpansion`,
`validate_with_domain(data, shapes, FactDomain)` and
`validate_with_domain_and_model(data, shapes, &model, FactDomain)` (choose asserted
facts or the data-plus-inferred closure for validation), the node-expression
seam `eval_node_expression(data, shapes, expr, focus) -> Option<Vec<Term>>`, and
the conformance primitive `conforms(data, shapes, shape_node) -> ConformanceCheck`
(call `.holds(node)` per focus). A gated W3C harness (`tests/w3c_node_expr.rs`)
drives the `sht:EvalNodeExpr` suite — all evaluation entries pass; a companion
harness (`tests/w3c_node_expr_constraints.rs`) drives the suite's two `sht:Validate`
entries (`sh:expression` / `sh:nodeByExpression`) end-to-end (both self-skip when
the suite is not fetched). Because those W3C harnesses self-skip on a fresh
checkout, the node-expression **function operators** are also pinned by a
fixture-independent unit suite (`tests/node_expr_operators.rs`, sq-qcnn) that drives
every built-in (`concat`/`count`/`sum`/`min`/`max`/`distinct`/`if`/`exists`/`limit`/
`offset`/`flatMap`/`orderBy`/`findFirst`/`matchAll`/`remove`/`instancesOf`/
`nodesMatching`/`var` + custom `sh:SPARQLFunction`) through the public
`eval_node_expression` seam and asserts hand-derived result sets — so the operator
semantics are gated even when the suite is absent. The SCS parser's fail-closed
error paths and the SHACL-SPARQL §5.2/§6 edge cases get the same treatment
(`tests/scs_error_paths.rs` under `scs`, `tests/sparql_edge_cases.rs`). The
pre-binding's deep-algebra arms (`push_values_down` over Group / Slice / Distinct /
Reduced / OrderBy / Minus-left / LeftJoin-left, plus the multi-scope arms — both
UNION branches, sibling joins, both LATERAL sides, and a projecting sub-SELECT,
sq-mue75 / GH #4291) and the
fail-closed runtime-error paths (an inexpressible blank-node focus, a `SERVICE`-clause
runtime query error) are pinned directly by the in-`src/sparql.rs` unit module
(`sparql::tests`, sq-qcnn.1 / sq-mue75): each arm is asserted both structurally (the
`VALUES` table lands BELOW the modifier / inside every branch so `$this`/`$value`/
`$param` stays in scope) and semantically (a real validator over real data yields the
SHACL-spec-correct conforms/violations).
