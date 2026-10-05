# Nullable alternatives and inverses in the exact evaluator

[GPT-6] This bounded admission slice uses the published
[SPARQL 1.1 path evaluation rules](https://www.w3.org/TR/2013/REC-sparql11-query-20130321/#defn_evalPP).
Inverse swaps the endpoints. Alternative is bag union, so `(p*|p*)` retains two
zero-length solutions for a concrete endpoint, including a term absent from the
active graph. Variable endpoints range over terms in subject or object positions;
a predicate-only term or a value supplied by VALUES does not enlarge that domain.

The [fixture matrix](../../../zk/sparql-evaluator/fixtures/conformance/nullable-alternatives.json)
checks complete result terms, unbound cells and multiplicities for both endpoint
roles, inverse, nested alternatives, repeated variables and OPTIONAL. The same
four-row query succeeds at its row budget and fails when the budget is smaller.
The original core-dataset discriminator and its two published mappings remain in
[the conformance corpus](../../../zk/sparql-evaluator/fixtures/conformance/cases.json).
Its historical empty engine result remains recorded separately in the coverage
ledger. The shared engine's earlier endpoint repair already resolves that case;
this change only widens the detached model's admission boundary.

The [independent-engine observations](../../../zk/sparql-evaluator/fixtures/conformance/nullable-alternatives-oracles.json)
record a concrete interoperability difference: PyOxigraph 0.5.11 deduplicates
these nullable alternatives and omits absent concrete seeds; RDFLib 7.6.0 instead
admits VALUES-supplied absent terms as variable endpoints. The original published
rules determine the goldens here; neither implementation is treated as consensus.
RDFLib results are read through its bindings table to preserve empty mappings.

Nullable sequences and lowered sequence intermediates remain rejected, including
when hidden inside an alternative or inverse. Repetition over a nullable child
also remains rejected. Those native implementations have separate regression
coverage; they are unfinished proof-profile coverage, not claimed SPARQL errors.
Residual paths inside EXISTS/NOT EXISTS remain excluded under the existing
correlation boundary. Validation traverses admitted alternatives and inverses;
these wrappers do not bypass a child's rejection.

The native and actual-guest runners use the same synthetic dataset and full-result
goldens. Actual guest execution must finish all successful cases before accepting
the typed relation rejections. The genuine false-ASK fixture additionally checks
that the absent-constant alternative's COUNT is two, preserving its older branches.
Test definitions alone do not attest a new guest image or receipt. Earlier proof
artifacts remain evidence for their recorded source only; new artifact execution
and review are required before this expanded slice is reported as validated.

No shared engine, parser, dependency, default feature or native hot path changes
in this admission slice. Admitted queries perform their actual bag work within
the existing row, AST and execution budgets; no timing neutrality is asserted for
queries which were previously refused.
