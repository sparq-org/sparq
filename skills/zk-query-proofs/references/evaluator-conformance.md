# Exact-evaluator query coverage

[GPT-6] The [coverage manifest](../../../zk/sparql-evaluator/coverage.json) maps the
bounded evaluator profile to executable semantic fixtures and explicit gaps.
It is a coverage inventory, not a claim of complete SPARQL conformance. Its
`partial_fixture_coverage` entries mean only that the linked examples are tested;
`known_gap` entries retain a standards-based expected result that currently fails.
`rejected` and `unassessed` entries never count as supported features.

The [original synthetic corpus](../../../zk/sparql-evaluator/fixtures/conformance/cases.json)
contains SELECT bags/sequences, ASK answers, and separate admission/data rejection
cases. Every case cites its normative rule. Expectations were written from those
rules before evaluator execution, rather than generated from the engine's output.
No user credential data appears in the fixtures.

## Run the host semantic corpus

From the repository root:

```sh
cargo test --locked --manifest-path zk/sparql-evaluator/Cargo.toml \
  -p sparq-proved-evaluator-model --features evaluate --test conformance
```

The existing [exact-evaluator CI workflow](../../../.github/workflows/zk-exact-evaluator.yml)
runs the model package with `evaluate`, which includes this integration test.
The runner checks every positive result against its exact expected cells and
multiplicities. Rejection cases check the specified boundary and error. Manifest
validation prevents a rejected case from being listed as positive evidence.
A failing semantic expectation remains a failing test; the manifest is not an
exclusion list.

This command executes the model on the host. It does **not** generate or verify a
zkVM receipt, establish guest compatibility for each example, or add a measured
proof-performance result. The separate guest tests remain the source of actual
proof evidence. The same query/data fixtures can be reused by an explicitly
selected guest campaign with pinned program identity and genuine receipts.

The shared engine's `tests/fixtures/builtin_edges.json` also runs through native
model admission/evaluation and `host/tests/actual_builtin_edges.rs`. The latter
executes the actual guest and compares its journal with the unchanged shared
expected cells. `published_recommendation` cases and `implementation_capacity`
controls are counted separately; the existing bounded integer-constructor lane
does not establish support for arbitrary XSD integer magnitudes. These guest
executions are distinct from receipts. The genuine false-ASK fixture includes a
SUBSTR position case alongside its numeric, nullable-path and MINUS branches.

## Exact result expectations

A SELECT fixture declares its projected variables, `Bag` or `Sequence`, and every
row. Bag comparison sorts complete encoded rows while retaining duplicates.
Sequence comparison preserves the declared order. `null` is unbound, while an
encoded empty literal is a bound RDF term. Input graph duplicates collapse as RDF
graph duplicates; repeated query derivations remain unless the algebra removes
them.

Exact sequence fixtures use explicit sort keys without ambiguous ties. REDUCED,
SAMPLE and GROUP_CONCAT examples use inputs with unambiguous permitted results.
The suite does not equate one engine's arbitrary ordering, representative choice,
or duplicate suppression with the only standards-permitted answer. Function
fixtures often compare values inside ASK so numeric lexical serialization does
not obscure the property being checked.

## Coverage boundaries

The negated-property-set examples have a bounded cross-engine disagreement. They
are original REC-derived expectations, not imported official test vectors.
The opt-in [reproduction script](../../../zk/sparql-evaluator/scripts/reproduce_nps_differential.py)
loads the exact committed query/data pairs and reports the observed rows or error
alongside their existing expected rows. It requires the named Python package at
the exact version recorded in the script; it never installs dependencies or edits
the corpus. Run either engine in an environment where that version is installed:

```sh
python3 zk/sparql-evaluator/scripts/reproduce_nps_differential.py --engine pyoxigraph
python3 zk/sparql-evaluator/scripts/reproduce_nps_differential.py --engine rdflib
```

Its JSON is implementation-observation evidence, not a conformance oracle, guest
execution or cryptographic receipt. An engine exception remains an explicit error
and never becomes an empty result. The coverage manifest records the exact engine
versions and the primary REC interpretation behind the retained expectations.

The [SPARQL 1.1 Recommendation](https://www.w3.org/TR/2013/REC-sparql11-query-20130321/)
is the normative baseline. The manifest separately records query algebra,
individual built-ins, named graphs, blank nodes, graph outputs, dataset clauses,
volatile/custom functions and external services. It gives an implementation
requirement for each unsupported area. A complete W3C syntax, evaluation and
error-suite campaign remains a different level of evidence from this focused
original corpus.

The [September 2026 SPARQL 1.2 draft](https://www.w3.org/TR/2026/WD-sparql12-query-20260910/)
is optional and still a Working Draft. Its triple terms, direction-aware literals,
version announcement and value-comparison changes have separate inventory entries.
The current profile's rejection of a 1.2 form is recorded as rejection behavior,
not 1.2 support.

The inventory tracks work under `zkp-10.2`. It does not expand the input-authority
contract: exact evaluation of a holder-selected graph does not establish issuer
authenticity, completeness of an outside database, or correctness of off-circuit
signature checks. See the [evaluator contract](../../../zk/sparql-evaluator/README.md)
for the accepted dataset scope, receipt requirements and experimental status.
