<!-- sq-4kr5: internal-stub README for a publish=false crate. -->
# sparq-zk-query

Internal expression-lowering skeleton for the Noir query method.
Research grade and not externally audited.

`lower::expr` defines a small expression IR and lowers it to a typed DAG.
Literal values live in a separate public-input array. Unsupported functions
return a named refusal. Runtime datatypes use the dynamic scalar lane.

The circuit gadgets are in `zk/noir-query/core/src/expr_*.nr`.
The checked-opening trait is a temporary integration contract for the term module;
this skeleton does not issue credentials or produce query proofs.

Run `cargo test -p sparq-zk-query` and the pinned `nargo test` in the Noir core.
Live tests use the existing ZK toolchain gate and its completion ledger.
