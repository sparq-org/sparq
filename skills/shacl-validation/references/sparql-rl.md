# SPARQL-RL (opt-in)

Enable the default-off `sparql-rl` feature on `sparq-shacl`. It is independent of
`shacl-af`, adds no dependencies and does not change validation behavior.

```rust,ignore
use sparq_shacl::sparql_rl::{parse, check_well_formed, stratify, infer, Limits, infer_with_limits};
let source = "PREFIX : <http://example/> RULE { ?x :q ?y } WHERE { ?x :p ?y }";
let base = "http://example/rules.srl";
let syntax = parse(source, base)?;
check_well_formed(source, base)?;
let strata = stratify(source, base)?;
let data = sparq_core::Graph::load_str("<http://example/a> <http://example/p> 1 .", "turtle")?;
let triples = infer(&data, source, base)?;
assert_eq!(triples.len(), 1);
let same = infer_with_limits(&data, source, base, &Limits::default())?;
assert_eq!(same, triples);
# Ok::<(), Box<dyn std::error::Error>>(())
```

`parse(source, base) -> Result<Document<'_>, ParseError>` retains the round 1
borrowed syntax tree. `Document.statements` preserves BASE, PREFIX, VERSION,
IMPORTS, DATA and RULE in document order. Rules carry name, head, `base_only` and
ordered `Element` values (triples, FILTER, SET, NOT/NOT DATA, GROUP). GROUP carries
keys, inner body and `Aggregate` assignments. Source slices retain delimiters
and spelling. The base must be absolute. Prefixes must precede use; subsequent
BASE/prefix redefinitions do not change earlier rules. `ParseError` carries
`kind`, byte `offset`, one-based `line`/character `column`, and `message`.
`ErrorKind::Limit` does not establish invalid syntax. Parsing accepts IMPORTS
without loading documents.

`check_well_formed(source, base) -> Result<(), RuleError>` checks the spec's
inScope/inUse conditions. Head and expression variables must be in scope; SET
and aggregate targets must be fresh. NOT locals do not escape. GROUP imports
only keys and exports keys plus aggregate targets. Names marked in-use cannot
be rebound, including inside a later GROUP.

`stratify(source, base) -> Result<Vec<usize>, RuleError>` returns a stratum number
for each rule in source order. It checks dependency stratification independently
of well-formedness. Closed cycles are rejected. SET, GROUP, head blank nodes and
variable-bearing head triple terms make a rule run once; its dependencies are
closed. WHERE DATA has no inference dependencies; NOT DATA adds no edge.

`infer(data, source, base) -> Result<Vec<oxrdf::Triple>, RuleError>` performs
both preparation checks and returns only triples absent from the base graph.
DATA seeds the working/inference graphs but never the immutable base graph.
Rules publish additions immediately, run once where required, then saturate each
stratum. SET expression errors drop solutions. NOT is correlated at its position
in the sequence. GROUP runs per outer solution with only key bindings imported;
without BY an empty pattern result still produces the empty-key group. Aggregate
errors discard that group. Head blank nodes are fresh per solution and retain
identity within that head. NOW is fixed for the complete inference call.
Expressions such as `IRI("child")` retain their rule's BASE.

All three processing APIs refuse IMPORTS, including declarations with no rules.
No loader or network access is implicit. `RuleError` carries `kind` and
`message`; `RuleErrorKind` distinguishes Syntax (including rejected lowering),
WellFormedness, Stratification, Imports, Limit and Evaluation. Only rejection at
the corresponding semantic stage satisfies a negative conformance entry.

`infer_with_limits(data, source, base, &Limits)` overrides operational budgets:
`max_rules`, `max_dependency_checks`, `max_iterations` (per stratum),
`max_queries` (whole operation), `max_rows` (query/table),
`max_inferred_triples`, `max_inferred_bytes` (serialized new triples),
`max_construct_triples` (template instances), `max_construct_bytes` (conservative
template-expansion estimate before deduplication), `max_query_bytes` (engine
working set) and `timeout` (whole-operation native deadline). Exhaustion returns an error without successful partial output or
changes to the input. Defaults and parser admission bounds are documented in
`sparq_shacl::sparql_rl`'s module documentation.
Wasm applies the deterministic budgets without a wall-clock deadline.

At data-shapes `04ecbb4`, the runner records 339 passes and three failures out of
342 entries. Three GROUP fixtures permit name reuse forbidden by the pinned
spec's inUse definitions: `wellformed-group-04`, `wellformed-group-09`, and
`eval-group-09`. Their failures remain visible. See the
[architecture and census](../../../research/sparql-rl.md) and
[gap analysis](../../../research/shacl12-conformance-gap.md).
