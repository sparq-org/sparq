# Exact temporal values and evaluation capacity

[GPT-6] `sparq-core::temporal::ExactTimeline` / `ExactTemporal` borrow validated
lexical fractions and compare checked integer whole seconds followed by decimal
digits. They preserve nanoseconds, fractions near a minute boundary and separate
seconds at large years. Parsing/comparison allocate no memory and take linear
work in literal length. Trailing fractional zeros do not change the value.

`Graph::exact_temporal_value(id)` borrows the original dictionary literal, including
compressed storage. Engine scalar/compiled comparison, sargable scan/COUNT, ORDER
BY, MIN/MAX and the reasoner's opt-in `substrate-compare` use this exact key.
Source literal identity remains unchanged, including the original MIN/MAX term.
Sorting reparses borrowed temporal keys; no unmeasured performance claim is made.
The existing `Timeline` floating fraction, `Temporal` f64 epoch/cache files and
approximate vector representations remain available for their representation
purposes. Their floating comparisons are not exact value or equality oracles.

The checked shared calendar range, including BCE, remains available natively.
The proof profile admits positive lexical years **1 through 1,000,000,000** for
`xsd:date`, `xsd:dateTime` and timezone-bearing `xsd:dateTimeStamp`. Fractions can
use the remaining bounded input bytes. The existing calendar grammar rejects year
zero; this is an explicit temporal subset. Query syntax/operators target the
published SPARQL 1.1 Recommendation, while numeric facets follow RDF 1.1/XSD 1.1;
this is not a claim of uniform XSD 1.0 or complete XSD 1.1 datatype support.
`xsd:time` is outside this date/dateTime comparison family.

Both-present/both-absent timezone values compare directly. Mixed presence remains
indeterminate in the inclusive fourteen-hour uncertainty interval. ORDER BY uses
a deterministic total extension (exact zero-offset instant, then timezone presence)
where the partial value order is indeterminate; this extension does not establish
relational equality or a uniquely required ordering for those values.

`QueryBudget.temporal_year_range: Some((min, max))` enforces an inclusive capacity
when a temporal value is evaluated or constructed. Dynamic STRDT/casts and stored
comparison paths share the same guard. A violation sets a separate sticky
`query evaluation capacity exceeded (temporal-year)` failure; FILTER, BIND,
COALESCE, and SERVICE byte rollback cannot convert it to false ASK or a successful
unbound result. Ordinary malformed-literal errors retain SPARQL error handling.
Input/query/result admission also checks literal years in the proof model. Native
`None` retains checked-range behavior. Constrained expression work runs on the
calling thread, preserving the per-query state across parallel-capable APIs;
nested engine calls use their own budget and restore the parent afterward.

SECONDS returns a validated exact decimal lexical even when its fraction exceeds
the finite numeric mantissa. This extends accessor output, not decimal arithmetic.
The separate numeric-capacity follow-up must guard long-decimal consumers before
this combined profile is frozen for a new guest campaign; existing historical
receipts do not establish those updated semantics.

## Evidence boundaries

The original 26-case matrix is in
[`exact_temporal.json`](../../../../crates/sparq-engine/tests/fixtures/exact_temporal.json).
It checks literal, VALUES, stored dense/compressed data, scan/COUNT, ORDER BY,
MIN/MAX, dynamic constructors, long SECONDS output and the dateTimeStamp timezone facet. Eight separately labeled
capacity rejections live in
[`temporal-capacity.json`](../../../../zk/sparql-evaluator/fixtures/conformance/temporal-capacity.json).
These are authored REC-derived expectations and explicit capacity controls,
not official W3C test vectors or a complete conformance claim.

Native core, engine, reasoner and model tests are separate from
`host/tests/actual_temporal.rs`, which must execute the same positive matrix and
match actual guest relation aborts for the capacity cases. The false-ASK genuine
proof fixture includes a nanosecond equality discriminator while retaining the
prior numeric, path-domain, MINUS and SUBSTR branches. New source/host tests are
not evidence of a new guest execution until a pinned-artifact campaign records it.

The [native checkpoint record](../../../../zk/sparql-evaluator/temporal-native-evidence.json)
contains source hashes, guard mutations and scoped test results. Its explicit
pending guest status must not be replaced by historical receipt evidence.
