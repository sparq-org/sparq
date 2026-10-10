# Proof methods outside the zero-knowledge circuits

A detached Cargo workspace for proof methods that are not Noir circuits or
zkVM programs, and for the baselines the zero-knowledge methods are compared
against. The methods take their identifiers from section 9 of the answer
specification (`site/specs/zksparql-answers.typ`), but they do not yet
implement its full proof-method profile: they work on the internal v5 request
and statement, which do not bind the protocol request's audience, acceptance
window, accepted methods or signature mode, and they define no canonical
statement or proof encoding. The workspace is not a member of the root
workspace, so it does not change the engine, the default build or the wasm
bundle.

| Crate | Method | Evidence kind |
|---|---|---|
| `disclosed` | `urn:sparq:vcq:method:disclosed-reevaluation` v1 | disclosed credentials |

`disclosed` runs the relation that the `risc0-authenticated-rdf` v5 guest
proves (`zk/sparql-evaluator/model`, `authenticated_rdf::evaluate`) natively,
on credentials the holder discloses. Its dataset commitment is the one the
zkVM method computes, so one verifier-agreed commitment serves both methods.
It is the baseline for the cost of the zero-knowledge methods: the same
statement, checked without a proof. It rejects DESCRIBE and FROM / FROM NAMED
before evaluation, and checks credential counts and sizes before copying them.

Where SPARQL 1.1 permits more than one result (OFFSET or LIMIT without an
ORDER BY that strictly orders the solutions, REDUCED, SAMPLE, GROUP_CONCAT,
floating-point aggregates), `disclosed` fixes the choice as follows: the holder
chooses through the credentials it sends and their order, and the verifier
accepts only the result the shared evaluator computes for that input. The
method publishes no rule for which permitted result that is, so a verifier must
not rely on it.

Research prototypes, not externally audited.

```sh
cargo test --release --manifest-path proof-methods/Cargo.toml
cargo run --release --manifest-path proof-methods/Cargo.toml \
  -p sparq-vcq-disclosed --example measure --features fixtures -- 31
```

`measure` prints one JSON object per query and credential count: issuer,
holder and verifier times (median and quartiles) and presentation sizes,
over the Q1–Q5 set in `disclosed/src/fixtures.rs`.
