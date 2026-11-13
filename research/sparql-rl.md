# SPARQL-RL parser and inference architecture

## Scope and grammar source

This implements the approved #7326 design: syntax, variable well-formedness,
dependency stratification and the `infer` operation. It does not expose a
separate goal-directed query operation. The default-off `sparql-rl` feature is
independent of `shacl-af`; neither validation nor the default wasm bundle gains
an execution hook or dependency when it is off.

The source is W3C data-shapes `04ecbb4`, `sparql12-rl/index.html` and its included
`sparql-rl-grammar.bnf`, productions 1–161. The parser retains ordered
BASE/PREFIX/VERSION/IMPORTS declarations, DATA, rule heads, base-only flags and
ordered rule elements. Keywords are case insensitive except `a`; tokens use
longest match. Comments end at CR/LF/EOF. Unicode escapes must designate scalar
values. The grammar excludes OPTIONAL, UNION, nested NOT/GROUP and unrestricted
paths. IRI/prologue handling reuses the existing SPARQL parser.

Syntax parsing retains IMPORTS without loading anything. Processing refuses all
IMPORTS declarations; no loader or implicit document access is provided. The
processor accepts the version hint `1.2`. Source slices remain lexical and
borrow the caller's input. Each rule's resolved BASE is retained for runtime
expressions such as `IRI("child")`, including after later declarations change it.

## Pinned manifest census

Only `mf:entries` members count. Unlisted definitions in eval/eval2 do not add
entries. None of the listed entries is withdrawn.

| Manifest | Type | Expected outcome | Entries | Pass / fail / skip with sparql-rl |
| --- | --- | --- | ---: | ---: |
| syntax | RulesPositiveSyntaxTest | Accept syntax | 146 | 146 / 0 / 0 |
| syntax | RulesNegativeSyntaxTest | Reject syntax | 66 | 66 / 0 / 0 |
| wellformed | RulesPositiveWellFormednessTest | Accept scope | 14 | 12 / 2 / 0 |
| wellformed | RulesNegativeWellFormednessTest | Reject scope | 19 | 19 / 0 / 0 |
| stratification | RulesPositiveStratificationTest | Stratification exists | 11 | 11 / 0 / 0 |
| stratification | RulesNegativeStratificationTest | Reject closed cycle | 10 | 10 / 0 / 0 |
| eval | RulesEvalTest | Inference graph isomorphic to result | 65 | 64 / 1 / 0 |
| eval2 | RulesEvalTest | Inference graph isomorphic to result | 6 | 6 / 0 / 0 |
| examples | RulesEvalTest | Inference graph isomorphic to result | 5 | 5 / 0 / 0 |
| Total | | | 342 | 339 / 3 / 0 |

The parser alone settles the 212 syntax entries. The other stages now execute;
negative tests pass only on rejection by their corresponding stage. Resource
exhaustion or an earlier syntax error cannot satisfy a negative semantic test.
The feature-off census remains 0 / 342 / 0. Manifest totals, pass floors, failure
ceilings and zero-skip ceilings are pinned in the runner. EARL includes all 342
outcomes when both rule features are enabled.

Three fixtures disagree with the pinned spec: `wellformed-group-04` and
`eval-group-09` reuse an earlier non-key variable inside GROUP, while
`wellformed-group-09` reuses a local name in a second GROUP. The spec's initial
`GUin = Uin union (Vin minus keys)` and subsequent inUse propagation forbid these
reuses. The processor follows those definitions and reports the failures rather
than changing the fixtures or treating them as passes.

## Preparation

The compiler lowers source slices to the existing SPARQL algebra. Labels for
body blank nodes become fresh internal variables consistently across the rule,
including nested triple terms. Anonymous pattern nodes and restricted sequence/
inverse paths are expanded through the same term vocabulary. Internal names
cannot collide with source variables.

The well-formedness pass implements inScope/inUse in source order. Head,
FILTER and SET expression variables must have values at the required position;
assignment/aggregate targets must be fresh. NOT locals do not escape. GROUP
imports only keys, checks its inner pattern with the prescribed initial in-use
set, and exports keys and aggregate targets. Repeated group keys denote a set.

Dependency analysis unifies body patterns and head templates with variables
standardized apart. It preserves repeated-variable equality and recursively
matches triple terms, enforces RDF subject/predicate domains, and gives template
blank nodes a blank-node domain. NOT
contributes closed dependencies unless it is NOT DATA. SET, GROUP, head blank
nodes and variable-bearing head triple terms make a rule run once, closing all
its dependencies. WHERE DATA has no inference dependencies. Stratum relaxation
rejects any cycle containing a closed edge. The standalone stratification API
is independent of variable well-formedness; inference performs both checks.

## Evaluation and shared seams

An inference call makes one mutable working copy of the base default graph.
DATA facts seed it, excluding facts already in the base from the returned
inference. Each new triple is inserted into that same graph. Rules query it and
publish additions before the next rule runs; no rule, iteration or query forks
a graph. A test-only counter measures the production copy helper across a
multi-rule recursive closure and asserts at most one copy per inference call.

The private `rule_schedule::saturate` loop is shared with rules12. Consumers own
layer ordering and publication; rules12 retains its snapshot-per-order-group
semantics and unchanged public API. The shared clock visitor freezes NOW in
both CONSTRUCT and SELECT algebra. The existing binding-table helper preserves
blank-node and nested-triple identity through engine calls. Its query-local
binding registry is installed once per inference operation.

SPARQL-RL run-once rules execute first in each stratum, then general rules run
to a fixed point. Matching, FILTER, SET, correlated NOT, GROUP, aggregate
functions, joins and template instantiation execute through `sparq-engine`.
The driver stages binding tables; it does not implement a second join or
expression executor. SET lowers to Extend plus a bound-result filter, so an
expression error drops the solution. Separate engine calls preserve sequential
FILTER placement. NOT uses correlated EXISTS over the selected graph.

GROUP is evaluated per outer solution. Only key bindings enter its inner
pattern; the resulting engine Group is joined back to that outer solution.
Without BY there is an empty-key group even for an empty inner result; with BY
there is no group in that case. An aggregate error drops its group. Duplicates
are retained in solution multisets and eliminated only when publishing triples.
Template blank nodes are allocated per solution and shared within its head.

## Termination and resource policy

For a prepared finite rule set, stratification runs term-generating rules once
after their dependencies. General recursive rules then range over a finite term
domain, so deduplicated saturation terminates. Operational bounds additionally
cover source admission, preparation comparisons, passes, query calls, rows,
new triples/serialized bytes, candidate template instances/estimated bytes
before deduplication, engine working-set bytes and a shared native
deadline. Exhaustion returns an error; the caller's graph remains unchanged and
no successful partial inference is returned. Defaults live in the public module
documentation and the private review companion.

Naive re-evaluation remains the implementation. The measured corpus does not
require a delta-driven evaluator to run its conformance checks. A separate
semi-naive follow-up accompanies the owner handoff; it is not claimed here.
