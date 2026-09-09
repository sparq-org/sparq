Local head `ccded1b4898cf5b317a6591f6ff6108ced23123c` fixes allocation-window ownership in two bench-only files. The allocator hot path, workload callers, core source and feature-off declaration are unchanged.

Exact formatted source: one serial regression passes. Unchanged old production with the identical test body compiles and fails the post-denial allocation probes. Both pass four clean calibrations. Fixed probes record one 128-byte request after each denied begin; old probes record none.

The original and formatted verification pairs used 13.852 seconds total. All task-private output stays below 128 MiB and observed free disk above 8 GiB. Pinned targeted rustfmt and whitespace checks pass. Xcode linker warnings are preserved; compiler success is not a Clippy result. Ordinary workspace CI does not reach this detached regression.

Existing measurements remain exact-old-head evidence: current callers have one balanced coordinator and joined workers; no trigger for this bug was found there. No benchmark was rerun. Ownership/quiescence preconditions remain, and the test does not force every initialization/readout interleaving.
