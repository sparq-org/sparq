Head `5f758d2afed6524cf191820463934c2213ddcfda` is a clean, two-file Astra-authored fix (+135/-3). It adds the title-only classifier rule and escaped row/label/tier diagnostics before the unchanged whole-plan zero-write barrier.

Validation:44 classifier tests,17 classifier self-test assertions and114 migration assertions pass. All8 compiled/executed controls are killed. The frozen6095 patch applies cleanly; T0 and existing property loops remain unchanged.

Author preflight fails only at the existing Bash3 privacy/mapfile limitation. No gate was weakened and no install, network replay, Rust build or remote mutation was performed. Linux privacy validation and actual independent Opus review remain required.
