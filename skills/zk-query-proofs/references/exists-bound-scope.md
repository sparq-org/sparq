# EXISTS and captured BOUND

<!-- [GPT-6] Experimental proof-profile boundary; not an engine conformance claim. -->

The published [SPARQL 1.1 substitution rule](https://www.w3.org/TR/2013/REC-sparql11-query-20130321/#defn_substitute)
replaces a bound variable occurrence with its RDF term. However,
[BOUND](https://www.w3.org/TR/2013/REC-sparql11-query-20130321/#func-bound)
takes a variable argument. The [EXISTS community report](https://w3c.github.io/sparql-exists/docs/sparql-exists.html)
identifies this variable-only position as Issue 2; its suggested repairs are
proposals, not a silently enabled replacement dialect here.

For example, the native engine currently returns the `?n = 1` mapping for:

```sparql
SELECT ?n { VALUES ?n {1} FILTER EXISTS { FILTER(BOUND(?n)) } }
```

The proof profile rejects this ambiguous Recommendation shape. The rejection
asserts no normative Boolean result, and does not establish a cryptographic
failure in earlier receipts. Native practical behavior remains unchanged.

Admission carries the input scope of each expression into EXISTS/NOT EXISTS.
A BOUND argument inside that body is rejected if its name is in the outer input
scope. This applies through expression wrappers, UNION branches, SELECT/BIND,
ORDER BY and OPTIONAL conditions. The input scope observes subquery projection
and the output domain of MINUS. Variables inside the body that are absent from
the outer input remain local: both `BOUND(?local)` and `!BOUND(?missing)` are
admitted. Ordinary BOUND outside EXISTS retains its existing behavior.

This is conservative potential-capture analysis: an outer variable declared by
VALUES or OPTIONAL counts even if a particular row leaves it unbound. An
all-UNDEF VALUES column is therefore also rejected when used by inner BOUND.
Admission does not run the query to infer row-specific binding presence. Nested
EXISTS and the prior complex-body exclusions remain in force. This boundary
does not settle substituted blank-node or subquery/binding-target ambiguities.

The synthetic [fixture inventory](../../../zk/sparql-evaluator/fixtures/conformance/exists-bound-boundaries.json)
contains eight admitted cases and twelve profile rejections. The model test
checks both independent admission and the complete evaluation entry point; a
separate native control records the unchanged practical outcome above. The
`actual_exists_bound` host test submits the same complete datasets to the real
guest, checks successful journals first, and then requires the guest's specific
relation-rejection panic. Other execution failures cannot satisfy that check.
Its source definition alone is not guest-execution evidence: the next integrated
artifact campaign must execute it, and historical artifact records retain their
original source identities.
