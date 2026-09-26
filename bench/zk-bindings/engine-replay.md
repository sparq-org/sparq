# Existing engine and storage replay

[GPT-6] `engine_replay.py` retains the original `sparq-bench` seed generator,
Oxigraph oracle and engine-independent `sparq-difftest` comparisons. The
[matrix](engine-matrix.json) maps existing optimizer, join, storage and worker
tests. Its small configured matrix does not replace the original fixed or nightly
windows, and compiling a feature does not show that its execution branch fired.

Build each matrix profile remotely in a separate artifact namespace. The shipped
profile keeps the existing bench defaults, including algebra rewriting. The
optimized profile adds exactly the dependency features listed in the matrix.
Keep the actual build command, toolchain, lock, source commit and executable hash
in a caller-approved build record. No ambient executable is selected.

```sh
cargo build --locked -p sparq-bench --profile release-fast
cargo test --locked -p sparq-bench fuzz::replay
python3 bench/zk-bindings/engine_replay.py \
  --accepted-binaries /path/to/accepted-binaries.json --output /path/to/new-run
```

The accepted JSON contains `source_commit` and a `profiles` object keyed by the
matrix profile. Each profile has absolute `binary`, its `sha256`, `build_record`
and `build_record_sha256`. The build record binds `source_commit`, `binary_sha256`
and the exact `features` list. These are explicit caller-accepted provenance;
hash agreement alone is not proof of how a binary was built.

Each cell executes `sparq-bench fuzz-replay SEED CATEGORY MODE NEW_DIRECTORY`.
Its retained `query.rq` and `data.ttl` are unchanged generator bytes. `record.json`
keeps their contents, raw native and reference RDF terms, full result headers,
duplicate rows, observed order and the oracle's actual comparison category.
The reference store can normalize RDF lexicals; its output is a differential
observation, never automatically an independent normative proof golden.
`observed_ntriples` is separately labelled native materialization, not a rewrite
of the retained original input or an independent commitment oracle.

The existing comparator preserves bags, sort-key equivalence classes and global
blank-node isomorphism. Arbitrary window row choice, isomorphism exhaustion,
adjudicated count-only differences and unclassified errors remain non-agreement.
Unexpected load/decode errors are not profile exclusions. Controller timeouts,
infrastructure failures and its own record capacity limit have separate counts.
The record size limit is a post-exit read bound, not a hard subprocess disk quota.
Any incomplete matrix or non-agreement prevents a complete oracle verdict.

Raw identity is reported independently. A different valid tie order or fresh
blank label may preserve semantic agreement while preventing byte identity;
the controller does not call that an engine defect or silently rewrite it.
Native replay produces no cryptographic proofs and authorizes no proof reuse.
The retained bridge fields stay null until the selected backend actually admits
the exact case, binds an authority scope and prepares its public statement and
private relation. Reuse requires those actual identities, not displayed rows or
normalized oracle agreement. Selected support and complete exact-result semantics
remain the distinct contracts in [inventory.json](inventory.json).

Replay failures retain every input and available partial result. Replay a cell
using its original seed/category and exact accepted binary before minimizing it.
This first slice preserves the original minimizers and fuzz targets; it does not
claim a minimized counterexample or per-backend admission/proof execution.

[OPUS-5.5] The separate [engine proof replay bridge](engine-proof-replay.md)
prepares one retained cell's unchanged originals for the exact V3 relation and,
in real mode, proves and independently verifies them. It writes its own status
record; this controller, its null record bridge fields, its denominators and its
classification are unchanged, and no proof is reused across cells.
