# SHACL 1.2 conformance at the November 2026 suite pin

The suite is `w3c/data-shapes` commit
`04ecbb4a5b930f4fa9df6c399ac5c98ccec672c9` (2026-11-12), pinned in
[`fetch-shacl-tests.sh`](../crates/sparq-shacl/fetch-shacl-tests.sh).
The local `shacl12-core`, `shacl12-sparql` and `shacl12-node-expr` spec texts
at that revision are the reference for the implementation and comparisons.
Addresses #7326.

## Measured ratchets

| Runner | Default: pass / fail / skip | `shacl-af`: pass / fail / skip | Pass floor before this pin |
| --- | ---: | ---: | ---: |
| `w3c_core` (SHACL 1.0) | 98 / 0 / 0 | 98 / 0 / 0 | 98 |
| `w3c_core_full_shacl12` | 162 / 0 / 0 | 162 / 0 / 0 | 136 / 137 |
| `w3c_core_shacl12` (Core node subset) | 55 / 0 / 0 | 55 / 0 / 0 | 44 / 45 |
| `w3c_sparql_shacl12` | 98 / 0 / 0 | 98 / 0 / 0 | 24 |
| `w3c_node_expr_constraints` | feature disabled | 8 / 0 / 0 | 2 |
| `w3c_node_expr` | feature disabled | 123 / 9 / 152 | 63 |
| `w3c_sparql` (SHACL 1.0 pre-binding subset) | 14 / 0 / 0 | 14 / 0 / 0 | 13 |

Pass floors equal the measured counts above; no pass floor was lowered.
The node-expression evaluation failure ceiling is 9. All other runners in
this table have a failure ceiling of zero. Of the node-expression runner's
152 skips, eight are validation entries handled by the separate constraint
runner; 144 are unsupported expression forms. The EARL sink omits the eight
entries belonging to the other runner and records unsupported forms as failed.

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

## Remaining node-expression evaluation map

These gaps belong to the separate node-expression workstream. The evaluator
reports nine mismatches, all under `node-expr/shnex/`:

| Entries | Observed gap |
| --- | --- |
| `failure-nested-propagates`, `failure-if-condition` | Evaluation failure is consumed by a surrounding operation. |
| `instancesOf-two-classes`, `instancesOf-expression` | Dynamic/list-valued class input yields no instances. |
| `max-strings`, `max-dates`, `min-strings`, `min-dates` | Non-numeric extrema yield no output. |
| `orderBy-iris-before-literals` | Term ordering places literals before IRIs. |

Unsupported forms comprise 22 `node-expr/custom` entries (list/named parameter
functions), ten `node-expr/shnex` entries (seven `conformsToShape`,
`constant-tt`, `failure-list-parameter-multiple-values`, `if-else-only-true`),
and 112 `node-expr/shnex-sparql` entries. Their individual IDs and diagnostics
are emitted by `w3c_node_expr`; EARL records all 144 as failed assertions.
The node-expression brief owns these groups; they are not suppressed by the
Core/SPARQL ratchets.

## Reproduction

With the pinned suite already present, run each `tests/w3c*.rs` runner with
`cargo test -p sparq-shacl --test <runner> -- --nocapture`, then repeat with
`--features shacl-af`. The behavioral guards are in
[`shacl12_core_sparql.rs`](../crates/sparq-shacl/tests/shacl12_core_sparql.rs).
Generate reports without fetching an already-present pin using:

```sh
SHACL_TESTS_PIN=04ecbb4a5b930f4fa9df6c399ac5c98ccec672c9 \
  crates/sparq-shacl/earl-report.sh /tmp/shacl-earl
```

The Core and SHACL-SPARQL sections of that SHACL 1.2 report contain 162 and
98 passed assertions, respectively, with zero failures. Inference-rule entries
have no runner in this report. Node-expression evaluation has 123 passes and
153 failures (nine mismatches plus 144 unsupported forms); its separate
constraint runner contributes eight passes.
