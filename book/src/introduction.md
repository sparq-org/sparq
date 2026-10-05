<!-- [OPUS-4.8] sq-im8u — single-source include wrapper. The lead paragraph is
{{#include}}d verbatim from the canonical README.md `lead` anchor (build-time content
injection), so it cannot drift from the README.

[OPUS-5] sq-w9sr — the experimental-status caveat is now {{#include}}d from the README's
`status-caveat` anchor too. It was previously a hand-maintained COPY, and that copy had
already drifted from the README: it had lost the conformance-report clause and two of the
links. That is the exact failure mode this docs-site bead exists to eliminate, and it is
worse for a load-bearing honesty caveat (AGENTS.md) than for ordinary prose. The earlier
claim in this comment that "no prose is duplicated from the README" was therefore false;
it is true now. The caveat's repo-relative links are made mount-portable at build time by
the `link-fixup` preprocessor (scripts/mdbook-rewrite-links.py). Design record:
research/docs-site-single-sourcing-anti-drift.md.

The only original prose left on this page is the one-line "next" navigation, whose link
targets are mount-point-specific (absolute GitHub URLs that resolve under the Pages
mount). -->

# Introduction

{{#include ../../README.md:lead}}

{{#include ../../README.md:status-caveat}}

## Where to go next

- [Install & build from source](./getting-started/install.md) — get a working build.
- [Capabilities at a glance](./getting-started/capabilities.md) — what each opt-in surface does.

Per-surface how-to guides live in the
[usage skills](https://github.com/sparq-org/sparq/blob/main/skills/SKILL.md) router, and the full crate
map is in [`AGENTS.md`](https://github.com/sparq-org/sparq/blob/main/AGENTS.md). Live per-commit
performance metrics are on the
[benchmarks dashboard](https://sparq.jeswr.org/dev/bench) — numbers are deliberately **not**
baked into these docs, because they drift.
