# SHACL 1.2 conformance at the November 2026 suite pin

The suite is `w3c/data-shapes` commit
`04ecbb4a5b930f4fa9df6c399ac5c98ccec672c9` (2026-11-12), pinned in
[`fetch-shacl-tests.sh`](../crates/sparq-shacl/fetch-shacl-tests.sh).
The local `shacl12-core`, `shacl12-sparql`, `shacl12-node-expr`,
`shacl12-inference-rules` and `sparql12-rl` spec texts
at that revision are the reference for the implementation and comparisons.
Addresses #7326.

## Measured ratchets

| Runner | Default: pass / fail / skip | `shacl-af`: pass / fail / skip | Earlier pass floor |
| --- | ---: | ---: | ---: |
| `w3c_core` (SHACL 1.0) | 98 / 0 / 0 | 98 / 0 / 0 | 98 |
| `w3c_core_full_shacl12` | 162 / 0 / 0 | 162 / 0 / 0 | 136 / 137 |
| `w3c_core_shacl12` (Core node subset) | 55 / 0 / 0 | 55 / 0 / 0 | 44 / 45 |
| `w3c_sparql_shacl12` | 98 / 0 / 0 | 98 / 0 / 0 | 24 |
| `w3c_node_expr_constraints` | feature disabled | 8 / 0 / 0 | 2 |
| `w3c_node_expr` | 272 / 4 / 8 | 272 / 4 / 8 | 123 |
| `w3c_inference_rules_shacl12` | 0 / 0 / 47 | 47 / 0 / 0 | new runner |
| `w3c_sparql_rl_shacl12` | 0 / 342 / 0 | 0 / 342 / 0 | new runner |
| `w3c_sparql` (SHACL 1.0 pre-binding subset) | 14 / 0 / 0 | 14 / 0 / 0 | 13 |

Pass floors equal the measured counts above; no pass floor was lowered.
The node-expression evaluation failure ceiling is 4. The inference-rules
ceiling is zero in both feature states. With `sparql-rl` enabled (alone or with
`shacl-af`), SPARQL-RL records 339 / 3 / 0 and pins that pass floor, failure
ceiling and the manifest total of 342. Without `sparql-rl`, its failure ceiling
is 342. All other runners have a failure ceiling of zero. The inference and
SPARQL-RL runners additionally pin their manifest totals and skip ceilings.
Feature-disabled inference entries emit failed EARL assertions. The
node-expression runner's eight skips are validation entries handled by the
separate constraint runner and omitted from evaluation EARL assertions. Every
evaluation entry is attempted with and without `shacl-af`. The table's feature
columns leave `sparql-rl` disabled; enabling it changes the SPARQL-RL row to
339 / 3 / 0 in either column.

## Closed Core and SPARQL gaps

Core now handles list-valued `sh:rootClass` disjunctively, emits one
`sh:uniqueValuesFor` result per offending value node without `sh:value`, unions
path values with computed/constant `sh:values`, and uses `sh:defaultValue` only
when that union is empty. Both term and dictionary-id node-kind checks support
`sh:TripleTerm`. `sh:reifierShape` emits one result per nonconforming reifier.

SHACL-SPARQL pre-binding reaches OPTIONAL branches, EXISTS graph patterns and
subqueries while preserving their projections. Nested solution modifiers are
executed separately for each focus. Blank nodes retain their identity through
an internal term-valued extension function; ground triple terms use algebraic
VALUES terms. Local VALUES and subqueries that hide a pre-bound variable are
admitted under SHACL 1.2. MINUS, SERVICE, VALUES mentioning a potentially
pre-bound variable, and AS rebinding one are rejected.

The explicit `validate_strict_shacl10` entry point retains the SHACL 1.0
Appendix A admission rules. Its suite therefore still checks the legacy
expected-rejection entries against that version's rules.

Prefix lookup includes default declaration owners, inverse `owl:versionIRI`
and transitive `owl:imports`. Query messages precede validator/constraint,
component and shape messages; language tags and template substitution are
preserved. A solution binding `failure` to boolean true produces a strict
processing failure instead of a validation result. SPARQL-valued deactivation
and defaults use the original focus. Custom SPARQL functions evaluate
`sh:ListParameterExpressionFunction` bodies containing argument access, count,
instance lookup, SELECT or SPARQL expressions, and require exactly one output.

Expression constraints keep the shape's original focus and bind the current
value node in the `value` scope variable (`expression-002`).

All entries requested by the Core/SPARQL brief pass at this pin. No requested
entry is left failing because of a test/spec divergence.

## Node-expression evaluation

The shared evaluator in `node_expr.rs` / `func.rs` now handles all expression
forms exercised at the pin: named operators, SPARQL function forms, SELECT and
SPARQL expressions, custom named/list-parameter bodies, argument access,
conformance checks and constant triple terms. Ordering and extrema use the shared
SPARQL total-order algorithm with exact numeric and temporal observations.
It preserves evaluation failures
through nesting, with SPARQL error handling for functions such as `coalesce`
and `if`. Named argument expressions use an empty scope when accessed; list
arguments use the caller's scope before entering the body. SPARQL callbacks use
this same evaluator and the validation-scoped registry/dataset.

`w3c_node_expr` has no unsupported evaluation entries. Four entries in
`node-expr/shnex-sparql` remain failing because the harness compares RDF terms,
including literal lexical forms:

| Entry | Expected decimal lexical | Actual decimal lexical |
| --- | --- | --- |
| `ceil-example` | `4.0` | `4` |
| `floor-example` | `3.0` | `3` |
| `round-example` | `4.0` | `4` |
| `seconds-example` | `00` | `0` |

The pinned Node Expressions spec, “Evaluation of SPARQL Function Expressions”,
requires the corresponding SPARQL function semantics. These results have the
required `xsd:decimal` datatype and value. Neither that section nor the SPARQL
function definitions prescribe the particular lexical forms asserted here.
The runner keeps these term comparisons failing; it does not normalize expected
or actual literals to hide the differences.

Per-folder evaluation counts are `custom` 22 passed, `shnex` 142 passed, and
`shnex-sparql` 108 passed / 4 failed. The API is available in both feature states;
rule execution and expression constraints retain their `shacl-af` feature gate.

## Inference rules and SPARQL-RL

The separate `rules12::apply_rules` API implements the pinned SHACL 1.2
execution model behind `shacl-af`; the existing SHACL-AF APIs retain their
behavior. Rules are selected by identity, with global rules and the union of
shape targets, and scheduled by exact decimal layer/order values. Run-once
rules execute before iterating rules; equal-order rules read one snapshot.
Each layer settles before advancing. Explicit rule sets follow transitive
`sh:includesRuleSet`. Unsupported processors on any rule or rule set in the
shapes graph produce a failure before selection or deactivation, including
requirements inherited from included sets.

SPARQL rule templates bind required/optional parameters through the shared
algebraic pre-binding machinery. Triple-rule operands default to the focus
node. The supported strict expression subset propagates failures, including
path-focus cardinality failures. Custom expression functions called from rule
queries reuse the shared evaluator. Conditions include shape-class superclasses.
Expected derived values are frozen at layer entry and removed unless a rule
also infers them. Temporary triples and their annotations are removed after
all layers. Declared rule entailments run before validation; strict validation
returns rule failures through its existing failure channel.

The inference runner visits all 47 manifest entries: all 41 entries in the top
folder and all six `rdfs/` entries pass. Without `shacl-af`, all 47 entries skip
locally and emit failures to EARL. Inferred graphs are compared by complete RDF
graph isomorphism, including shared blank-node identity and nested triple terms.
The folder's validation entries expect empty reports; the runner checks both
conformance and absence of results. Expected failures require a production error.

There are no identified spec/test divergences in the inference entries.
Unknown processors on unselected rule sets are processing failures. The engine
now emits the canonical decimal spelling required by
`SPARQLRuleTemplate-example-Multiply`; its prior allowance is removed and the
inference pass floor is raised to 47, with zero failures.

SPARQL-RL now has separate syntax, variable inScope/inUse, dependency
stratification and inference stages behind the default-off `sparql-rl` feature.
Its six manifests list 342 entries: `eval` 65, `eval2` 6, `examples` 5,
`stratification` 21, `syntax` 212 and `wellformed` 33. With the feature enabled,
all 212 syntax and 21 stratification entries pass, as do 31 well-formedness and
75 evaluation/example entries. Without it, all 342 entries remain failures.
Unlisted evaluation resource descriptions do not add manifest entries.

The remaining three failures are fixture/spec disagreements at this pin:

- `wellformed-group-04` and `eval-group-09` reuse a previously bound non-key
  variable inside GROUP. The spec initializes the inner in-use set with
  `Uin union (Vin minus keys)`, so a triple pattern cannot bind that name again.
- `wellformed-group-09` reuses an inner variable name in a later GROUP. The
  first GROUP's inUse propagation retains that local name, forbidding its reuse.

These entries expect acceptance under different name-shadowing rules. The
processor follows the pinned inScope/inUse definitions and emits failed EARL
assertions for them. No fixture, oracle or floor is weakened. Stage-specific
negative tests require that stage's rejection, not resource exhaustion or an
unrelated syntax error. All 342 outcomes are included in the SHACL 1.2 EARL
report, with a complete-census guard. The architecture and full type census are
in [sparql-rl.md](sparql-rl.md).

## Reproduction

With the pinned suite already present, run each `tests/w3c*.rs` runner with
`cargo test -p sparq-shacl --test <runner> -- --nocapture`, then repeat with
`--features shacl-af`; SPARQL-RL additionally requires `--features sparql-rl` or
`--features shacl-af,sparql-rl`. The behavioral guards are in
[`shacl12_core_sparql.rs`](../crates/sparq-shacl/tests/shacl12_core_sparql.rs),
[`shacl12_node_expr.rs`](../crates/sparq-shacl/tests/shacl12_node_expr.rs),
[`shacl12_rules.rs`](../crates/sparq-shacl/tests/shacl12_rules.rs) and
[`sparql_rl_eval.rs`](../crates/sparq-shacl/tests/sparql_rl_eval.rs).
Generate reports without fetching an already-present pin using:

```sh
SHACL_TESTS_PIN=04ecbb4a5b930f4fa9df6c399ac5c98ccec672c9 \
  crates/sparq-shacl/earl-report.sh /tmp/shacl-earl
```

The Core and SHACL-SPARQL sections of that SHACL 1.2 report contain 162 and
98 passed assertions, respectively, with zero failures. Inference rules contribute
47 passed assertions. SPARQL-RL contributes 339 passed and three failed
assertions. Node-expression evaluation has 272 passes and four failures; its
separate constraint runner contributes eight passes.
