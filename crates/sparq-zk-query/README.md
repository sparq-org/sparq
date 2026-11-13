# sparq-zk-query

Research grade query compiler for the Noir query method; not externally audited.
This internal crate is opt-in through workspace package selection and is not published.

## ✨ Compiler contracts

The plan is bounded and canonical. Public request bounds choose the smallest fitting
credential, quad and table buckets; private data never selects a bucket. The key reveals
those capacities and the query structure. Exceeding either the request bound or the
compiled bound yields a named capacity refusal, never a truncated answer.

Constants are public parameters rather than compiled query structure. Plan encodings
include parameter kinds, operator wiring, schemas, canonical orders and compiled
capacities. Parameter values and variable names are absent from the shape hash.
The interpreter and circuit order rows lexicographically by term encoding in schema
order, preserving duplicates unless DISTINCT is requested.

Credential and expression integration follow the programme's shared contracts.
The composed manifest is superseded by the query method.

## 🚀 Host contracts

Run the host contracts with `cargo test -p sparq-zk-query`. The interpreter's
seeded differential test accepts `SPARQ_NOIR_QUERY_SEED` as an optional integer
seed; unset, it replays its fixed regression seeds. This setting affects test
inputs only. The fixture encoder supplies test cells under the stand-in contract.
Native execution tests use the shared live-toolchain gate and run explicitly
with `cargo test -p sparq-zk-query --test execution -- --ignored`.


## 📚 Architecture

See the [query compiler record](../../research/noir-query-compiler.md).

## License

MIT.
