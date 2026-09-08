> 🤖 **SPARQ agent** — I am @jeswr's agent for the jeswr/sparq RDF/SPARQL engine. @jeswr runs multiple agents; this was written by the SPARQ agent, not the PSS agent (prod-solid-server).

Cancelled-gate recovery could not authenticate its durable rerun receipt: its query requested `id` on the `Actor` interface, and its consumer expected GraphQL's bot login to include REST's `[bot]` suffix. Select the ID on concrete `Bot` and derive the canonical author login from the already verified REST account. The authenticated viewer, stable node ID, bot type, exact run-attempt claim, current-head/queue/review guards and #6049 hold stay enforced.

The fixture now models GitHub's actual field projection and the two login spellings. Read-only GitHub checks accepted the exact production query and confirmed that the orchestrator's REST and GraphQL identities share a node ID. No production claim, rerun or dispatch was used for validation.

Validation: 80 arm-capability tests and the rearm self-test pass. Two calibrated controls independently restore the invalid query and the old login comparison; each makes the same positive recovery test fail with one assertion failure, zero errors and zero skips. Preflight's other checks pass; the existing macOS Bash 3 `mapfile` limitation in privacy-claims remains, so Linux CI is still required.

Independent review used actual Claude Opus 5 with extra-high reasoning. The complete change at `ed9689d5be5be28ccbc3300c1d90b250319f2c7c` and the focused identity correction at final head `5640d06b669f4a6cd52029d6b0dcc9ee48b6e692` both received `approve_with_nits`, with no blockers. The fixture intentionally supports the fixed Actor/Bot selection used here, rather than being a general GraphQL validator. Review packet SHA-256 values: `ebc73847ffc5c568f78fcda1bcffe3045c69ecb370985ee809753636e00b81c6` and `cbbed101eb48afd78a04ed9ae18e72294f9a3010417c7730ced407b7e96edf9f`.

Closes #6462.
