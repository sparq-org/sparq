# Numeric evaluation capacity

[GPT-6] `QueryBudget.strict_numeric_capacity` is false by default and explicitly
enabled by the exact-dataset evaluator. It separates a valid RDF numeric lexical
from a computation the current engine can represent. This is a bounded numeric
profile, not arbitrary-precision SPARQL arithmetic.

Integer consumers use `i64`. Decimal consumers use a signed `i128` mantissa and
checked decimal scale/alignment. Decimal division retains the engine's existing
precision: exact results terminating within eighteen fractional digits when representable,
otherwise half-up rounding to eighteen fractional digits. Integer overflow, decimal overflow/alignment failure, and
otherwise valid conversions outside these lanes set a sticky
`query evaluation capacity exceeded (numeric-representation)` failure. Exact
integer/decimal operations cannot silently switch to a floating datatype in this
mode. Normal promotion involving a float/double operand retains that type's
semantics, including NaN. This guard does not promise exact real-number arithmetic
for floating types or expand the existing decimal division precision.

Scalar and compiled expressions, FILTER/equality, casts, ordering, MIN/MAX and
SUM/AVG share checked consumers. Numeric sargable shortcuts and parallel expression
work fall back when this option is active, so worker-local evaluation cannot lose
the capacity state. This may increase work; no latency neutrality is claimed.
Reentrant calls restore their parent's state. FILTER, BIND, COALESCE and ordinary
expression-error handling cannot turn a capacity failure into a completed false
ASK or unbound SELECT result. Malformed lexical forms and division by zero remain
ordinary expression errors that COALESCE can handle.

Returning a valid large numeric RDF literal, valid unary plus, `isNumeric`, and
`sameTerm` do not require numeric consumption and preserve term identity. Exact
SECONDS output likewise retains all bounded input fraction digits; a later
numeric consumer fails if the result exceeds the decimal lane. Default native
evaluation retains its existing finite-representation fallback behavior, which
is a known implementation limitation, not a normative datatype validity rule.

SUBSTR's integer arguments use an `i128` lane. Larger valid arguments reject the
bounded evaluation in this slice. The preserved normative examples with a huge
length and with canceling huge start/length would clip to `"abcd"` and `"a"`;
supporting these requires checked lexical arithmetic, not independently saturating
each argument. Their expected strings remain in the capacity corpus for follow-up.

The [21-case capacity corpus](../../../../zk/sparql-evaluator/fixtures/conformance/numeric-capacity.json)
contains explicit capacity rejections, not W3C conformance successes. Native
tests exercise scalar/compiled wrappers, stored dense/compressed consumers,
large parallel thresholds and reentrant state. The model also preserves all 167
normative goldens in the 169-case builtin corpus while rejecting its two separately
labeled legacy representation-capacity controls. These are native checks until
an actual guest campaign records the integrated source and artifact; historical
proofs do not establish the new guard.
