> 🤖 **SPARQ agent** — I am @jeswr's agent for the sparq-org/sparq RDF/SPARQL engine. @jeswr runs multiple agents; this was written by the SPARQ agent, not the PSS agent (prod-solid-server).

## Summary

An unknown area label currently aborts triage without identifying the issue that produced it. This adds one JSON-escaped `UNKNOWN_AREA` record per offending issue/label pair, including its classification tier. The error names the fetched label set and possible incomplete enumeration, prohibits creating a label from the failure, and directs exact-label verification before a separate reviewed maintenance action. The global check still exits before every write, unpark, write-budget cut, JSON report or dry-run path.

Titles beginning with `triage-area` or `triage-area.py` now route to `ci` before generic topic rules. Author-declared T0 ownership still wins, and actual generator work such as #5016 continues to route to `sparq-wrapper-gen`.

Addresses #6468. The missing existing-crate label was repaired through separately reviewed maintenance. The first later [ordinary scheduled run](https://github.com/sparq-org/sparq/actions/runs/34395557649) succeeded on existing main and completed its bounded writes; #5016 was correctly classified but deferred by the write budget. That run does not validate this unmerged patch. Issue closure follows merge and recovery verification. Related enumeration issue #6335, older PRs #5457/#6095 and policy documentation issue #5412 remain separate.

## Validation and review

- 45 classifier tests and both embedded self-test suites pass locally. Ten executed negative controls were killed, including global-guard removal, checking only the writable prefix, late rule placement, anchor metadata removal, row/label/tier corruption and newline injection. No compile-only kills or test errors.
- Author preflight passed its other applicable checks. The local privacy checker could not execute under Bash 3.2 (`mapfile`); the existing Linux privacy gate must pass before admission. Linux CI must also execute the classifier suite under its pinned Python 3.12.
- Actual GPT-6 Astra, extra-high reasoning, authored the two-file change. Actual Claude Opus 5, extra-high reasoning, approved the original source and final focused delta for validation, with no blocking findings and `perf_affecting: false`. Test outputs were reviewed as supplied evidence, not independently rerun by Opus. Copilot review, resolved threads and all protected checks remain required.
- Root verified the clean exact head, all frozen evidence hashes and both committed diffs. Tracked-source searches found no consumers of the removed error strings. The frozen #6095 patch passes a read-only applicability check; this is not combined testing or approval of that PR.

Review head: `afa9c6489db0351855acd9297a6aa2c845434106`. Final Opus result SHA256: `58b405c9018ef10c161b37c32517cb2c88de284506b057e69e2342b69c4000d4`.

## Limits and conventions

The fixtures intentionally use real workspace crate discovery; a future wrapper/generator rename requires updating their ownership contract. Complete finite offender records are retained. The classifier does not create labels or log raw issue bodies. No workflow, write budget, pacing, T0 parser, partition policy, protection, conformance/performance/coverage floor or registry dispatch setting changes. Registry Actions remain disabled. No native human review or roborev result is claimed.
