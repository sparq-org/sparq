> 🤖 **SPARQ agent** — I am @jeswr's agent for the sparq-org/sparq RDF/SPARQL engine. @jeswr runs multiple agents; this was written by the SPARQ agent, not the PSS agent (prod-solid-server).

A failed or cancelled scheduled run can currently make the next same-head run skip unfinished coverage and mutation work, then finish green. Freshness now requires completed coverage plus all 51 named mutation jobs, bound to the same head, run and attempt, with successful completion markers. A green skipped follow-up cannot supply that proof.

The read-only, bounded helper allows incomplete work to run at the next ordinary schedule. API/quota errors, unreadable or contradictory evidence, active competitors and truncated active inventories fail closed without retries or dispatches. Manual dispatch retains its explicit force behavior; the heavy tier stays outside merge-group execution. Completed mutation runs retain their advisory ratchet policy: allowed exits 0/2/3 still require strict counts matching the complete planned mutant list. Two original, hash-pinned CI artifact fixtures cover actual unsharded and sharded cargo-mutants output, including baseline accounting.

Validation:

- 47 focused tests, 88 selection-wiring tests and 19 runner-reservation tests pass. All 15 calibrated follow-up controls cause assertion failures, with 47 tests and zero errors each; the discarded first harness attempt is excluded from that evidence.
- Actionlint and diff checks pass. Local preflight passes its other checks but hits the existing macOS Bash 3 `mapfile` limitation; the required Linux CI gate remains mandatory.
- The docs-quality hard-gate job runs the new tests on every normal PR and merge group, with no path filter. Its capacity test bounds the current workflow at 101 jobs within the 200-job read limit, including reusable jobs; the observed scheduled run had 85 jobs.
- Real artifacts came from existing run [34099890455](https://github.com/sparq-org/sparq/actions/runs/34099890455). The parse job log independently confirms cargo-mutants 25.3.1 and the fixture counts. No new heavy measurement or production recovery request was executed to obtain this evidence.

Implementation: GPT-6 Astra, extra-high reasoning. Independent source review: actual Claude Opus 5, extra-high reasoning. Whole-change review at `2f3d1773bb3cebd36186692025912a8377c85db6` requested changes; the remediation review of final head `0fc2faf6d4255e9e27b8a17bc43e8ba848c29368` concluded **approve with nits**, with no remaining substantiated blockers. The reviewed input SHA-256 is `6b67133cd821ae5ba3c4b0cddacb67e5abc830bb5fbe9e327b8739045ec8566b`; source has not changed since review. Optional observability refinements do not affect completion proof or the fail-closed error policy.

Closes #6436.
