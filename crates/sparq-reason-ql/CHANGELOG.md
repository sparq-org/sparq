# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/sparq-org/sparq/releases/tag/v0.1.0) - 2026-09-03

### Added

- resolve target issue #2513 [opus5] ([#4282](https://github.com/sparq-org/sparq/pull/4282))
- resolve target issue #3238 [opus5] ([#3860](https://github.com/sparq-org/sparq/pull/3860))
- *(sparq-reason-ql)* DL-Lite_R consistency checking via violation-query composition, opt-in ql-consistency (sq-p6yb7) [FABLE-5]
- *(reason/ql)* branch-aware emitter — per-branch FILTER/VALUES for multi-branch UCQ (sq-sg542) [OPUS-4.8] ([#1677](https://github.com/sparq-org/sparq/pull/1677))
- *(reason/ql)* desugar non-recursive property paths (/, ^, |) ahead of the CQ gate (sq-pbz04.3.2) [OPUS-4.8] ([#1671](https://github.com/sparq-org/sparq/pull/1671))
- *(reason-ql)* lift body blank nodes to fresh existential variables (sq-pbz04.3.6) ([#1653](https://github.com/sparq-org/sparq/pull/1653))
- *(reason-ql)* broadened sound input fragment — UCQ, literal atoms, FILTER/VALUES pass-through, intensional-atom guard (sq-pbz04.3.1) ([#1647](https://github.com/sparq-org/sparq/pull/1647))
- *(reason-ql)* total TBox-capture accounting + fully_captured() (sq-pbz04.3.3) [SONNET-4.6] ([#1438](https://github.com/sparq-org/sparq/pull/1438))
- *(conformance)* graduate OWL 2 QL (DL-Lite_R) certain-answer oracle to a pinned sparq-extension floor (sq-qo1a9) [OPUS-4.8] ([#1316](https://github.com/sparq-org/sparq/pull/1316))
- *(conformance)* wire EXPERIMENTAL OWL 2 QL query-rewriting arm into the entailment suite as OutOfScope, not a graduated floor (sq-kuvu3) [OPUS-4.8] ([#1312](https://github.com/sparq-org/sparq/pull/1312))
- *(reason-ql)* OWL 2 QL production path — tree-witness folding + UCQ-containment minimisation (sq-g19x0) [OPUS-4.8] ([#1297](https://github.com/sparq-org/sparq/pull/1297))
- *(reason-ql)* EXPERIMENTAL OWL 2 QL PerfectRef rewriter + fail-closed CQ-shape gate (sq-t5bne) [OPUS-4.8] ([#1292](https://github.com/sparq-org/sparq/pull/1292))

### Other

- *(sparq-reason-ql)* QL rewriting comparison vs Ontop on NPD + Requiem suites (rewrit (sq-hmd7l.9) ([#3442](https://github.com/sparq-org/sparq/pull/3442))
- *(reason-ql)* add end-to-end answer gate [GPT-5.6]
- *(reason-ql)* combined-approach evaluate-first spike — reasoned non-adoption (measured UCQ ≤4) (sq-pbz04.3.5) [OPUS-4.8] ([#1687](https://github.com/sparq-org/sparq/pull/1687))
