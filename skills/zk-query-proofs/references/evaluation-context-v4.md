# Request-bound evaluation context (V4)

[GPT-6] V4 adds deterministic `NOW()` to the bounded V3 read-form relation. It
uses a separate request, witness and journal schema and separate request/source
commitment domains. V1/V2/V3 requests retain their prior wire interpretation.
This implementation is in preparation; native, actual guest and genuine receipt
evidence must be reported separately for its eventual frozen artifact.

## Meaning of the context

[SPARQL 1.1 NOW](https://www.w3.org/TR/2013/REC-sparql11-query-20130321/#func-now)
requires one dateTime value throughout a query execution without selecting an
exact clock instant. This profile makes that value an explicit agreed input.

The verifier supplies the independently expected `v4::Request`, including
`context.now.datetime`. This is the exact lexical form of one `xsd:dateTime` with
a required timezone. The guest validates the context even if the query has no
NOW call or places NOW only in an unselected expression branch. All NOW calls
use that same typed literal, including calls inside subqueries, aggregate
arguments, ORDER BY, FILTER, OPTIONAL conditions and nested expressions.

The context is an agreed input. It is not an observation of a machine clock,
an attestation of current time, or a freshness guarantee. Both source-authority
modes still require independent agreement on the expected query and context.
A holder-declared dataset does not make the holder's preferred time authoritative.
Applications establish any clock trust or freshness policy outside this relation.

The original query bytes are retained in the public request digest. Inside the
guest, a bounded structural pass replaces only the built-in zero-argument NOW
expression with the validated typed literal. It does not rewrite query text,
strings, comments or custom functions. Ordinary admission then checks the entire
resulting algebra and all normal capacity and source-blank restrictions apply.
No host-evaluated result or pre-specialized algebra is accepted as the witness.

## Using the versioned APIs

Build `v4::Witness { request, dataset }` with the same complete source/catalog
container and result policy as V3. Set `version = v4::VERSION`, the V4 dialect,
`ProofContract::ExactDataset`, a nonzero challenge, and:

```rust,ignore
context: v4::ExecutionContext {
    now: v4::NowContext {
        datetime: "2026-09-13T00:02:03.000000001Z".into(),
    },
},
```

For verifier-agreed source scope, compute `v4::dataset_commitment` from the
independently accepted complete source and policy. This anchor is separate from
V3 and independent of the current NOW value; request binding authenticates time
separately. Retain provenance in holder-declared mode. Neither mode supplies
issuer authentication, status validation or wallet completeness by itself.

The host `v4::prove_with_artifact` and `v4::verify_with_artifact` use the existing
independently accepted artifact and pin flow. An older guest cannot prove V4.
The verifier checks the exact program, complete expected request and explicit
journal context before consuming its nonce. Equivalent dateTime values with
different lexical spellings are distinct contexts. Replay and cross-version
journal interpretation fail even when the returned result happens to match.

## Bounds and inherited exclusions

NOW context is at most 1,024 UTF-8 bytes, with positive years 1 through
1,000,000,000 and input-bounded fractional precision. Calendar and timezone
validation uses the shared exact temporal parser after rejecting boundary
whitespace. This typed context never applies string-constructor normalization.
No floating conversion
is used to compare dateTime values. Consuming long fractional seconds through
finite numeric arithmetic retains the separate whole-query capacity guard.

The original query is at most 8 KiB and retains the parser's nesting bound.
Specialization visits at most 1,024 AST nodes and clones at most 65,536 context
lexical bytes. Exceeding any limit rejects the entire relation, even if a branch
would not have been evaluated. Source, result, graph canonicalization and guest
execution bounds remain those of V3; resource failure never becomes false ASK.

`BNODE`, `RAND`, `UUID` and `STRUUID` remain excluded. The context does not create
an unbiased random source or fix the separate native BNODE freshness/collision
problem. Source blank nodes retain V3's complete-source EXISTS boundary, including
unselected named graphs. Captured BOUND and other published-2013 ambiguities keep
their executable admission exclusions. CONSTRUCT template blank nodes are still
created after WHERE evaluation under V3's explicit freshness rules.

SERVICE remains excluded. A future endpoint-snapshot relation must authenticate
complete local snapshots and define SILENT/failure behavior; calling a remote
endpoint on the host and proving only its output hash would not establish query
execution. No network callback, endpoint freshness claim or hidden clock trust is
introduced by V4.

## Evidence

`model/tests/now_context.rs` defines native semantic, context-binding, authority,
capacity and inherited-rejection controls. Ten complete goldens are shared in
`fixtures/now-context.json`; `host/tests/actual_now_context.rs` executes them under
both authorities before invalid-context and capacity controls. Two genuine
receipts in `host/tests/real_now_context.rs` add holder-table and verifier-graph
contexts. The mandatory campaign requires all ten V1/V2/V3/V4 receipts on one
frozen V4 artifact and retains every earlier actual guest regression. Source-level API
availability is not completed execution evidence. RISC Zero's upstream privacy
assurance limits, disclosed receipt metadata and independent artifact deployment
requirements remain unchanged; no complete SPARQL or performance claim follows.
