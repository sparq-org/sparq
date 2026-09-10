# PR6482 exact-head CI admission evidence

Candidate `fe3284199db0831f353d2ae401b8c69472764904` (one changed file, clean) was tested on GitHub's PR merge `8ff2bc3ceb54b29a870b3a555b1e11efaf4e182c` over main `4595388de9e389f5369d63828fbf90cfc16b62d9`.

**CI evidence is ready for root's admission decision.** This is a mechanical evidence audit, not another source review or merge approval.

- Linux CI 34488984319 actually ran all **20 UPDATE module tests** in bulk jobs 102914791113 (6), 102914791025 (7), and 102914791050 (7). The observed names exactly equal all 20 committed test names. `module-tests.json` records raw line/time associations.
- The disputed `lexical_adjudication_preserves_rows_and_blank_node_structure` passed in job **102914791025**, `test-bulk2.log:539`, at **14:44:21.5971878Z**. The separate identical-lexical witness was not executed and is not claimed here.
- Required `gate` check/job **102910459274**, app **15368**, run **34488983731**, exact PR head, completed SUCCESS. At `gate.log:305` the full tier reported 64 gating contexts green or skipped/neutral, six declared advisories excluded, stable set. This is not a claim of 64 executed suites.
- MSRV job **102910710715** actually checked the workspace/all-targets with **Rust 1.88.0**, excluding sparq-py and sparq-hdt. Clippy job **102910710949** passed workspace/all-targets default and all-features `-D warnings`, docs and bench/dict lint. Fmt is informational.
- Archive job **102911182277** built 742 nextest binaries from workspace/all-targets plus approx-ann,filtered-ann,vec-predicate, then workspace doctests. Test execution was narrowed to package(sparq-bench), not every workspace test.
- Hard docs job **102910460410** ran the real Linux privacy scan (784 files), 32 privacy selftests, preflight embedded selftest and 81 preflight unit tests. Hard flow job **102910459571** passed actual G1/G2/G6 diff checks. Preflight fixture FAIL/PASS output is not mistaken for a real failing/passing full candidate preflight.
- Source selection skipped W3C conformance/wasm/nightly heavy legs; matrix had seven successful check/guard/assembly jobs and four skipped execution jobs. Full statuses are preserved.
- Later label-only CI **34491437751** has `IS_NO_LEG_RUN=true`, one selector success and 30 skipped jobs. It explicitly retains earlier real authoritative CI evidence. It is not substituted for the executed run.

20 bounded logical GitHub reads including initial budget; no retries, writes, builds, or tests. All download commands terminated successfully; no command pending. Raw logs and metadata are retained; the 12.9MB Clippy log is losslessly gzip-compressed with verified uncompressed SHA-256. Root owns review resolution and queue actions.
