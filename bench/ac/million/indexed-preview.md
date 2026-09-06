# Existing indexed-storage diagnostic

This bounded diagnostic compares the same generated Pod through an in-memory graph,
the existing `Graph::save` raw indexes, and `Graph::save_compressed` indexes. It uses
`Graph::open` without changing or bypassing dictionary and compressed-block validation,
then `PodStore::new` and WAC or ACP materialization. The index format opens every named
graph and reconstructs authorization from policies; it does not trust a saved auth view.

```sh
cargo build --release -p sparq-lws-core --example indexed_population_preview
target/release/examples/indexed_population_preview \
  --output-dir /tmp/indexed-history-wac --profile history --model wac --pods 8 \
  > /tmp/indexed-history-wac.jsonl
```

The output directory must not exist and its parent must exist. Repeat with
`--model acp` in a different directory. Profiles are `smoke`, `history`, and `entropy`;
`--pods` is limited to 1–16 and defaults to eight. `--max-pod-bytes` bounds each
generated N-Quads buffer (512 MiB by default, maximum 2 GiB). Rejection is a failed
diagnostic run, not a missing Pod silently omitted from its results. The first eight
Pods do not cover every activity class and cannot establish population capacity.

JSONL reports generation, parse/index construction, raw/compressed save, indexed open
including validation, `PodStore::new`, authorization materialization, and query plus
serialization time separately. The full open-to-authorized-ready interval includes
opening, construction, and materialization; filesystem accounting and result
comparison run afterward. The six queries are point lookup, count, transaction list, contact
join, media optional field, and graph enumeration, under owner, recipient, and
anonymous identities. Raw and compressed results must match the memory reference
exactly as ordered rows or result bags, including duplicate multiplicities; count
queries also use the independent record-count oracle. This tests persistence and
authorization selection. It is not an independent SPARQL evaluator or an HTTP test.

Each format records actual file/directory counts, logical file bytes, and Unix
allocated bytes from `st_blocks * 512` (including directory blocks). File plus
directory count gives inode entries in this generated tree; no symlinks or hard
links are created. Open keeps two journal descriptors per Graph object, reported
as an explicit estimate rather than a process-wide descriptor measurement. The
current format creates fourteen base files per Graph object, plus named-graph
manifests/directories; opening adds `wal.log` and `txn.log`. These costs must be
included before extrapolating directory-based storage to millions of Pods.

Runs process one Pod at a time. After preparing both formats and the memory
reference, that source graph is dropped before opening each persisted copy. Files
were just generated, OS caches are uncontrolled, and format order is fixed. Results
therefore describe an exploratory open-path comparison, **not cold-disk latency,
p95 response latency, or a capacity claim**. Query costs omit HTTP, token validation,
network transfer, concurrency, and updates. Saving uses the existing API's completion
contract; the example adds no durability or crash-recovery guarantee. Capture the
source revision, build profile, machine, resource limits, and full JSONL with any
cloud-run result. Do not treat local development timings as paper evidence.
