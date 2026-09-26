# Exact-dataset aggregate profile

[GPT-6] This boundary applies only to the experimental exact-dataset evaluator.
The general shared engine's aggregate behavior is unchanged.

COUNT accepts expressions and nullable bindings: its definition explicitly removes
errors. Other built-in aggregates accept an RDF constant or a variable established
as bound by the inner algebra. A statically empty input also remains admitted.
BGP matches and fully bound VALUES establish bindings; UNION requires them on both
sides; OPTIONAL establishes only its left-side bindings. Projection removes hidden
bindings. BIND establishes a new binding only for a constant or an already bound
variable alias. Nested aggregate outputs currently require a separate binding
analysis and are conservatively excluded as outer aggregate operands.

This excludes even valid expressions such as SUM(?x + 1). It is not a claim of
complete aggregate support. The executable cases, including that deliberate
restriction, live in [aggregate-boundaries.json](../../../zk/sparql-evaluator/fixtures/conformance/aggregate-boundaries.json).
The same checks run when evaluating the authenticated source inside the guest;
a host-only admission check cannot bypass them.

A bound term can still cause the defined aggregate type error: SUM over an invalid
numeric facet remains unbound. COUNT ignores an errored operand. SUM propagates
floating NaN. MIN/MAX over NaN have no unique result imposed by the Recommendation's
partial order; the pinned engine chooses an input term, and that choice is not
an assertion that NaN has a mathematical minimum or maximum position.

The published [ListEval definition](https://www.w3.org/TR/2013/REC-sparql11-query-20130321/#aggregateAlgebra)
retains unbound/error results; [COUNT](https://www.w3.org/TR/2013/REC-sparql11-query-20130321/#defn_aggCount)
explicitly removes them. The [official aggregate erratum](https://www.w3.org/2013/sparql-errata#errata-query-6)
records ambiguity for MAX. This profile avoids choosing a global interpretation
of that disputed error case. MIN(?x + 0) over VALUES {1 "bad"} and SUM(?x) over
VALUES {1 UNDEF} are concrete excluded cases, not counted as successful conformance.

[GPT-6] `host/tests/actual_aggregates.rs` executes all twelve admitted cases before
checking the twelve strict guest relation rejections. These authored tests are
separate from the native corpus; they establish execution only when the final
artifact's hosted or local campaign records them, not when this file is added.
