# Native Ark tracing dependency patch

[GPT-6] The detached native workspace selects this exact registry-derived
`ark-relations` package through its own patch table. The evaluator SDK's separate
Ark package and the ordinary root workspace are unaffected.

The patch updates the optional tracing-subscriber dependency to the maintained
API with default features still disabled. It renames the existing **empty**
`Layer::new_span` callback to `on_new_span`; its body and all constraint arithmetic
are unchanged. A manifest-only experiment failed at that exact callback with
E0407. This is instrumentation compatibility, not a new cryptographic algorithm.

`UPSTREAM.json` pins the original registry archive, VCS revision, all original and
patched file hashes and the exact patch. Both original licenses are preserved.
Only the two manifests and that callback identifier differ from the archive.
The zero-context patch uses one stripped `a/` or `b/` path component; it is
reversed and reapplied in a private copy to verify complete reconstruction.

```sh
cargo fetch --locked --manifest-path zk/native-composition/Cargo.toml
python3 zk/native-composition/vendor-support/verify.py
python3 zk/native-composition/vendor-support/test_verify.py
python3 zk/native-composition/vendor-support/verify.py --smoke --offline
```

The explicit fetch populates the locked native graph, including optional RDF
dependencies that default tuple tests do not build. The verifier always resolves
that graph with `--offline --locked --all-features`; failed resolution retains
Cargo's stderr and fails without a network fallback or skipped provenance check.
The verifier checks both declared and actually resolved native vendor paths,
lock selection, unchanged default features and the maintained subscriber version.
The smoke command uses an isolated test-only graph, starting from the native lock:
it compiles the no-default Ark API and executes real Registry span capture and
satisfied/unsatisfied constraint controls. Registry dependencies are enabled only
for that smoke; they are not claimed as the native runtime feature selection.
Set `CARGO_TARGET_DIR` to an idle compatible cache. Omit `--offline` only when
public test dependencies need fetching. The smoke is not a guest or prover test;
the separate native tuple/RDF suites provide actual proof regression evidence.

The upstream package's existing unsafe instrumentation implementation is retained
except for the empty callback name; no authored unsafe code is added. The original
source and licenses keep upstream formatting. This patch's source review does
not certify the unreviewed upstream package. Registry audit obligations, license
policy and maintenance advisories remain separate. The historical unpatched
[native gate failures](../evidence/native-rdf-dependency-gates.json) are preserved.
Do not interpret successful patch checks as clean dependency gates or transfer
the earlier proof artifact's source identity to this successor.
