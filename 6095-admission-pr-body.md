> 🤖 **SPARQ agent** — I am @jeswr's agent for the sparq-org/sparq RDF/SPARQL engine. @jeswr runs multiple agents; this was written by the SPARQ agent, not the PSS agent (prod-solid-server).

## What / why

A declaration keyword could match the tail of a longer word: `tool-surface:`, `subcrates:` and `my_crates:` entered the T0 author-declaration path. The negative lookbehind now rejects preceding word characters and ASCII hyphen-minus. Genuine field forms remain accepted, including bullet-prefixed declarations. Lower-priority classification still applies; this is not a general rejection of prose or an authentication boundary.

Both title/text scope properties now require a nonempty checked partition whose size, combined with its counterpart, covers the rule table. Empty or truncated helper lists can no longer make those checks pass vacuously. This does not claim protection against arbitrary duplicate or overlapping helper lists.

Addresses #4567. Issue closure follows verified merge and a resolving-PR comment.

## Recovery and validation

The original Claude Opus 5 source commit `0883db681c73ddc338643f135c770c5a036ac3a4` is preserved. GPT-6 Astra with extra-high reasoning merged main `e53464c73f31f7aca800f3867ac054c36408e346` locally into that ancestry without conflicts or source repairs. Recovery head: `86b8dfa232ad7d315dd56df28759ac5fca7b365c`. The net two-file patch is unchanged from the original PR; the newly landed #6473 diagnostics and zero-write guard remain intact.

- 47 classifier tests, 17 classifier self-test checks and 114 migration self-test checks pass locally. All six compiled/executed negative controls ran the full 47-test suite and failed through assertions, with no errors or survivors. They cover absent/insufficient boundary protection, disabling valid declarations, empty title/text partitions and a truncated title partition.
- Root verified the clean exact head, both parents, all frozen evidence hashes and the full diff. The classifier's main function is byte-identical to main, and the #6473 diagnostic test class is unchanged. Repository consumer searches found the intended classifier caller/tests; the same-named PR-label policy method is a separate API.
- Author preflight passed its applicable checks except the privacy checker, which could not execute under local Bash 3 because `mapfile` is unavailable. Exact-head Linux/Python 3.12 validation and the real privacy gate remain required.
- Fresh independent Claude Opus 5 with extra-high reasoning approved this recovery for validation, with no blocking findings and `perf_affecting: false`. It reviewed public source plus minimal structured test evidence; it did not rerun the tests. Final review result SHA256: `0460b354e9ff7f54e8a7caa5e3cee2ab85c0dd8294d6bf20603beb932fcac949`.

## Admission and limits

The previous aggregate failure was an exhausted single redispatch of a cancelled `auto-arm` workflow, not evidence of a source defect. Its old success/failure rows do not substitute for new-head checks. Registry Actions and automatic dispatch remain disabled; this direct coordinator follows the user's Astra implementation / Opus review instruction.

The bot's `review:needs` hold remains. Normal ready-for-review transition is needed to produce the required full `gate` context, because draft runs produce only `gate, draft-tier`; it does not authorize clearing the hold or arming the PR. Exact-head full CI, Copilot review, resolved threads, live hold reconciliation and the protected merge queue are still required. No native human approval, canonical `review:pass` verdict or roborev result is claimed.

The scope remains the existing lexical boundary contract. Other punctuation and Unicode-dash neighbours are unchanged; no new line-only declaration policy is introduced. Fixtures intentionally use real workspace crate names and the existing T2 fallback. No workflow, partition, write budget, protection, performance floor or release setting changes.

Historical registry provenance below remains attached to its original source/reviewed commit; it is not a current-head admission receipt.

<!-- sparq-impl-provider:anthropic model:opus5 -->
<!-- sparq-reviewed-sha:0883db681c73ddc338643f135c770c5a036ac3a4 -->
