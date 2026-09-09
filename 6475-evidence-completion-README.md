# Issue6475 evidence completion

[GPT-6 Astra] Production source is unchanged. report.json records scope and conclusions; public-review-packet.md is the portable reviewer input.

The standalone Rust harness uses exact cached registry versions and an APFS clone of this task's own warm replay target. No dependency source was modified. build.py resolves offline then builds locked/offline with twojobs, a300s cap and6GiB free floor. run-cases.py executes the declared matrix/budgets and every entry in fixtures/manifest.jsonld through unmodified Rust and JS5 modules, without retries. See protocol.json for fixed limits; raw stdout, expected output and errors are in commands-cases.json/suite-results.json. Exit0 of a collection harness is not an invariance verdict: matrix-summary.json explicitly records label dependence.

Only the86-entry packaged RDFC10 manifest was scored; legacy URDNA/support files present in the package were not independently treated as extra tests. The Rust upstream test module, Sparq property binary, workspace/engine and broader random campaign were not built or rerun. No source fix, gate change, remote action or model call occurred. Earlier three bundles were rehashed unchanged. Clone cache target/ is regenerable and excluded; binary/canon-baseline is the actual exported executable.

Reproduction writes logs: copy this evidence directory first. Python3 build.py then Python3 run-cases.py reproduces the logged commands under the recorded local toolchain/runtime paths; portable Rust harness and Node case-driver source are included.
