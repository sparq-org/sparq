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

The draft proof primitive is `method::Compiler` (`prepare`, `prove`, `verify`).
Choose an artifact root outside an existing Nargo workspace. Use `with_crs_path`
to select a writable local CRS dependency cache. It verifies
only the explicitly marked supplied-cell profile while the suite lane is pending.
The public protocol adapter, credential authentication and envelope are separate
integration work; this draft primitive does not advertise those obligations.

`Request::new` stores the exact native key with the shared protocol request and
request bounds. The holder compares that key with its own compilation before
reading private inputs. The verifier derives its plan and constants from its
stored request, re-encodes the RDF answer and consumes the original shared
challenge only after proof verification and a final validity check.

Public inputs, in declaration order: request digest, salted input commitment,
signature mode, request bounds, constant cells, slice indices, canonical result
and statement binding. Public scalars are canonical big endian field words.
Term cells flatten as bound, kind, encoding, key class, key, datatype, language
and credential scope. SELECT tables flatten their padded rows before their length.
`statement::Layout` defines the declaration schema and
reconstruction and audits the locally compiled ABI. The salt, credential cells
and all intermediate tables are private. The supplied-cell dataset commitment
is a draft integration stand-in, replaced with the suite's signed messages.

The digest is SHA-256 over RDFC canonical N-Quads of the method-specific query request.
The exact typed protocol encoding is an explicitly named method parameter inside
that RDF. It never substitutes for request RDF canonicalization.

`SPARQ_NOIR_QUERY_EVIDENCE`, unset by default, accepts a local output path for the
native proof test's JSONL evidence. It records plans and native measurements only
when explicitly requested. Keep that output outside the repository. The corpus
and native differential tests use the shared live gate and run with
`cargo test -p sparq-zk-query -- --include-ignored`.

A complete answer is relative to the opened dataset. Without a verifier-agreed
commitment the holder chooses that dataset. Repeated questions can reveal more
through their answers; the holder service supplies the policy hook.

## 📚 Architecture

See the [query compiler record](../../research/noir-query-compiler.md).

## License

MIT.
