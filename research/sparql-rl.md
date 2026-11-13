# SPARQL-RL parser and processor design

## Scope and source

This design addresses #7326. Round 1 provides an opt-in SPARQL-RL syntax parser
and runs the syntax tests. Evaluation, well-formedness and stratification remain
unsupported outcomes in the runner. Round 2 requires owner review of this design
before implementing the processor. No validation or engine query path changes
are part of round 1.

The source is the local W3C data-shapes checkout at `04ecbb4`, specifically
`sparql12-rl/index.html` and its included `sparql-rl-grammar.bnf`, productions
1–161. The entry point is `RuleSet`. This pin supersedes the earlier pin in the
brief. Keywords are case insensitive except `a`; lexical tokens use longest
match. Comments end at CR, LF or EOF. Unicode escapes must designate scalar
values. Prefixes and BASE declarations apply in document order. VERSION is a
syntax/semantics hint, retained in the syntax tree. IMPORTS is parsed without
loading a document.

The parser uses recursive descent and a longest-match lexer over the existing
`regex` dependency. It does not extend the engine's broader SPARQL grammar:
SPARQL-RL excludes OPTIONAL, UNION, nested negation/group patterns, unrestricted
property paths, and several SPARQL functions. The existing `spargebra` parser
checks IRI references and prologue resolution; it does not decide rule variable
scope. A borrowed syntax tree retains ordered declarations, DATA blocks, rule
identifiers, head templates, base-only flags and ordered body elements. Triple
blocks and expressions retain validated source slices for later lowering.
Syntax success makes no assertion about well-formedness or evaluability.

## Pinned manifest census

Only members of `mf:entries` count, following manifest includes. Unlisted test
definitions are not entries. None of the listed entries is withdrawn.

| Manifest | Manifest type | Expected outcome | Entries | Parser settles |
| --- | --- | --- | ---: | --- |
| syntax | RulesPositiveSyntaxTest | Accept syntax | 146 | Yes |
| syntax | RulesNegativeSyntaxTest | Reject syntax | 66 | Yes |
| wellformed | RulesPositiveWellFormednessTest | Accept variable scope | 14 | No |
| wellformed | RulesNegativeWellFormednessTest | Reject variable scope | 19 | No |
| stratification | RulesPositiveStratificationTest | Stratification exists | 11 | No |
| stratification | RulesNegativeStratificationTest | Reject closed cycle | 10 | No |
| eval | RulesEvalTest | Inference graph isomorphic to result | 65 | No |
| eval2 | RulesEvalTest | Inference graph isomorphic to result | 6 | No |
| examples | RulesEvalTest | Inference graph isomorphic to result | 5 | No |
| **Total** | | | **342** | **212** |

The runner keeps every entry in its census, including unsupported negative
tests. Only a syntax error can satisfy a negative syntax test; a resource-limit
error cannot. Feature-off counts remain at the existing census floor. Feature-on
floors are based on the measured syntax results. EARL publication remains
unchanged until the processor is reviewed and evaluation coverage is meaningful.

## Evaluation model proposed for round 2

Implement the `infer` operation first: immutable base graph plus a resolved rule
set produces only inferred triples absent from the base graph. DATA facts seed
the inference graph and evaluation graph, but do not enter the base graph.
IMPORTS requires an explicit caller-supplied loader, document deduplication and
RDF merge with blank nodes standardized apart; without import support the
processor returns an error for any IMPORTS declaration.

Preparation checks the spec's `inScope` and `inUse` conditions, then constructs
open/closed dependency edges using triple-pattern/template compatibility,
including repeated variables and nested triple terms. Reject cycles containing
a closed edge. A run-once rule has SET, GROUP, head blank nodes, or a variable
inside a head triple term. Its dependencies are closed. WHERE DATA has no
inference dependencies; NOT DATA contributes no dependency edge.

Evaluate strata in order, run-once rules first, then general rules to a fixed
point. Deduplicate inferred triples against both the base and inference graphs.
Rule-body elements consume and produce multisets in source order. FILTER and
SET errors discard the current solution. NOT is correlated at its position in
the sequence; its local variables do not escape. GROUP imports only group keys
into its inner pattern and exports keys and aggregate assignments. Without BY,
an empty input still has the empty-key group; with BY it has no group. An
aggregate error discards that group. Fresh template blank nodes are allocated
per solution, with identity shared within that solution's head.

## Mapping to rules12 and the engine

Reuse the `rules12` layer/run-once/fixed-point scheduling concepts, immutable
input handling, inference vocabulary and graph expansion helpers. Extract a
small private scheduler seam only in round 2, with regressions against both
consumers. Do not encode dependency analysis as user-authored `sh:order` values.
The existing snapshot-per-order-group behavior is not a blanket translation of
SPARQL-RL's immediately available inferences: a scheduler adapter must publish
each rule's additions before the next rule runs.

Lower accepted triple blocks, restricted paths and expressions through the
existing `spargebra` algebra and `sparq-engine` query path. Reuse
`sparql::construct_bound` where its semantics agree. SET needs BIND followed by
a bound-result filter; bare SPARQL BIND passes expression-error rows onward.
Keep sequential FILTER placement explicit rather than relying on SPARQL group
filter placement. NOT must preserve correlation and graph selection. GROUP
needs per-input key projection and the specified empty-group behavior; a whole
body translated into an ordinary SPARQL GROUP BY is insufficient. Blank-node
scope spans the rule body, including triple terms. Freeze NOW across the full
inference operation, following the existing rules12 clock seam. No second join
or expression executor belongs in this module.

## Feature gating, termination and bounds

`sparq-shacl/sparql-rl` is default off and independent of `shacl-af` in round 1.
It compiles only the parser and adds no dependencies. There are no hooks in
validation, the engine, or wasm bindings when it is off. Its own feature-matrix
leg runs parser behavioral tests and the manifest runner. Round 2 may reuse
private scheduler helpers without making SHACL-AF a required public API.

The parser accepts at most 1 MiB of UTF-8 source and 64 levels of recursive
grammar nesting. It returns a distinct limit error, never a successful parse of
a truncated document. Flat repetitions are iterative; every accepted iteration
consumes input. Returned slices borrow the caller's source.

For a prepared finite rule set, stratification places term-generating rules
after their dependencies and runs them once. Remaining recursive rules range
over a finite term domain, so deduplicated saturation terminates. This semantic
argument does not substitute for operational bounds: round 2 must combine
explicit import, preparation, query, solution, triple and iteration budgets,
with errors on exhaustion and no successful partial inference. The private
review companion specifies the resource controls and their review obligations.
