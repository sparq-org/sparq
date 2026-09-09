# Issue6475 phase2 evidence

[GPT-6 Astra] No production changes. `report.json` records confirmed findings and limits. `public-review-packet.md` excludes host/private execution details and is the intended independent review input.

Reproduction: `python3 run.py` recreates the instrumented offline Rust diagnostic in this directory and expects the saved pair to fail with exit1. `node reference-replay.cjs` uses the unmodified extracted pinned reference package; `node reference-fixtures.cjs` runs the three fixture controls. Use the frozen command JSON for the exact installed executables and environment actually used. Re-execution would overwrite diagnostic log paths: copy this bundle first.

The only upstream source-copy mutation is the instrumentation diff; no algorithm correction was attempted. Package SHA512 verifies the reference download, and phase1 package provenance verifies the Rust source. Do not interpret the JS default complexity error as successful canonicalization, or the diagnostic explicit iteration budget as a production policy change.

Private target is regenerable and excluded from the manifest; the exact traced executable is exported under binary/. Parent phase1 evidence remains immutable.
