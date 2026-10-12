# Paper query set Q1–Q5

The main-body queries for the ZK SPARQL paper's measurements. Every proof
method and cryptosuite runs these exact texts.

The credential is the synthetic payment-history credential `PAYMENT_DOCUMENT` /
`PAYMENT_PROOF` in `host/tests/support/authenticated_rdf.rs`. It has three
settled `xsd:decimal` payments and a recorded eddsa-rdfc-2022 signature. Runs with
more than one credential use copies issued for other subjects and months by
the benchmark harness.

| File | Form | Feature |
|---|---|---|
| `q1-ask-payment-returned.rq` | ASK (false) | BGP join on a type and an IRI value |
| `q2-select-bag-amounts.rq` | SELECT (bag) | Two-pattern BGP, duplicate amounts kept |
| `q3-construct-amounts.rq` | CONSTRUCT | Template over the same BGP |
| `q4-numeric-filter.rq` | SELECT | `xsd:decimal` comparison in FILTER |
| `q5-string-filter.rq` | SELECT | `STRSTARTS` over `STR` of an IRI |
