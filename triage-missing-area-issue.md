> 🤖 **SPARQ agent** — I am @jeswr's agent for the sparq-org/sparq RDF/SPARQL engine. @jeswr runs multiple agents; this was written by the SPARQ agent, not the PSS agent (prod-solid-server).

The scheduled area classifier aborts its entire classification pass when its generated plan includes `area:sparq-wrapper-gen`, which is absent from the repository's labels. This prevents otherwise valid rows from reaching the write loop.

Evidence: [run 34299934012, job 102304479961](https://github.com/sparq-org/sparq/actions/runs/34299934012/job/102304479961), on main `a42a9e89dec485f6a319c47cb3635c59cb5a2270`, attempt 1. The internal fixtures and independent tests passed; the live classification step then exited 2 with `ERROR: rule table produced labels that do not exist: ['area:sparq-wrapper-gen']`. The [unknown-label guard](https://github.com/sparq-org/sparq/blob/a42a9e89dec485f6a319c47cb3635c59cb5a2270/scripts/triage-area.py#L738-L746) returns before the classification/write loop, so this occurrence performed no classification or unpark writes.

A direct label lookup returned 404, and a fresh GraphQL lookup on 2026-09-09 returned null while main remained at the same commit. Exact-label and broader triage-area searches found no matching issue. Related #6335 concerns truncating enumeration at 500 labels; it does not explain this currently absent label and should remain a separate repair. The failed run does not identify the originating issue row, so the precise classification path and intended supported owner area still need to be established.

The focused repair should identify that path, use the appropriate existing supported area mapping if one is justified, and add a regression fixture for the actual input. Preserve the unknown-label fail-closed guard and prove an unsupported label still causes zero writes. Improve the diagnostic to identify the offending row/path if needed. Do not automatically create labels, guess a broad routing replacement, or replay live work as part of diagnosis.

Discovered during bounded operational triage alongside #4246; no routing change has been made.
