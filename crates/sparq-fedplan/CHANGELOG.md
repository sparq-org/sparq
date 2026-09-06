# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/sparq-org/sparq/releases/tag/v0.1.0) - 2026-09-03

### Added

- *(sparq-fedplan)* Port the federated adaptive re-planner to the LOCAL evaluator ([#4817](https://github.com/sparq-org/sparq/pull/4817))
- *(sparq-fedplan)* fedplan: wire the RetrievalCapability static hint into selection.rs (sq-3uijg) ([#2470](https://github.com/sparq-org/sparq/pull/2470))
- *(fedplan)* optional static retrieval-capability descriptor for GenAI endpoints (sq-222my) [FABLE-5] ([#2031](https://github.com/sparq-org/sparq/pull/2031))
- *(fedplan)* opt-in per-source (cardinality-weighted) latency aggregation in the adaptive cost model (sq-s5kd) [FABLE-5] ([#1963](https://github.com/sparq-org/sparq/pull/1963))
- *(site)* federation /capabilities row + captured-output fedplan walkthrough + /surface/federation stub (sq-vw3ax.13) [FABLE-5] ([#1959](https://github.com/sparq-org/sparq/pull/1959))
- *(fedplan)* make sparq-fedplan publishable + planner bench + SKILL (sq-my8wd.3) [SONNET-4.6] ([#1423](https://github.com/sparq-org/sparq/pull/1423))
- *(fedplan)* opt-in predicate-selectivity non-star join cardinality (sq-jsuzr) [OPUS-4.8] ([#1251](https://github.com/sparq-org/sparq/pull/1251))
- *(sparq-fedplan)* EWMA refinements — per-source α, time-aware decay, staleness eviction (sq-3xkz) [OPUS-4.8] ([#893](https://github.com/sparq-org/sparq/pull/893))
- *(sparq-fedplan)* EWMA-smoothed latency in adaptive-replan cost model (sq-b51o follow-up) ([#371](https://github.com/sparq-org/sparq/pull/371))
- *(sparq-fedplan)* fold per-source latency into adaptive-replan cost model (sq-b51o) ([#356](https://github.com/sparq-org/sparq/pull/356))
- *(sparq-fedplan)* adaptive mid-execution re-planning, opt-in (sq-7s4z) ([#325](https://github.com/sparq-org/sparq/pull/325))
- *(sparq-fedplan)* ANAPSID-style non-blocking streaming join + spill (sq-vf7q) ([#298](https://github.com/sparq-org/sparq/pull/298))
- *(sparq-fedplan)* cost-based federated source selection + bind-vs-hash join planner (sq-a35t) ([#278](https://github.com/sparq-org/sparq/pull/278))

### Other

- *(deps)* Bump foldhash from 0.1.5 to 0.2.0
- *(fedplan)* sweep cardinality postcondition [GPT-5.6] ([#2206](https://github.com/sparq-org/sparq/pull/2206))
- *(fedplan)* add foldhash descriptor-map A/B [GPT-5.6]
- *(fedplan)* mutation-kill — features-ON mutants matrix + assertions (sq-3dyje.7) [SONNET-4.6] ([#1924](https://github.com/sparq-org/sparq/pull/1924))
- *(deps)* Bump criterion from 0.5.1 to 0.8.2 ([#1634](https://github.com/sparq-org/sparq/pull/1634))
- *(sparq-fedplan)* correctness coverage (sq-bif) [OPUS-4.8] ([#1226](https://github.com/sparq-org/sparq/pull/1226))
- *(sparq-fedplan)* correctness suite for source-selection / plan / adaptive / descriptor pruning (sq-bif.3) [OPUS-4.8] ([#782](https://github.com/sparq-org/sparq/pull/782))
- *(readme)* bring 20 crate READMEs to the readme-template (sq-inzv) [OPUS-4.8] ([#751](https://github.com/sparq-org/sparq/pull/751))
- *(sparq-fedplan)* sweep residual private_intra_doc_links on descriptor.rs:321 (sq-qik4) [OPUS-4.8] ([#484](https://github.com/sparq-org/sparq/pull/484))
- *(sparq-fedplan)* fix crate-doc broken intra-doc links in default (fedplan OFF) build (sq-gxx7) ([#474](https://github.com/sparq-org/sparq/pull/474))
