# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/sparq-org/sparq/releases/tag/v0.1.0) - 2026-09-03

### Added

- *(sparq-shaclc)* write direction — residual-consumption SHACL-CS printer, first Rust-side SCS writer (sq-tonhr.6) [FABLE-5]
- *(sparq-shaclc)* SHACL Compact Syntax 1.2+extended parser crate — rdf-shuttle-generated, oracle+differential tested (sq-tonhr.6) [FABLE-5]

### Fixed

- *(bench)* consolidate shaclc G1 harness [GPT-5.6]
- *(ci)* complete sparq-shaclc G1 artifacts [GPT-5.6]

### Other

- *(sparq-shaclc)* pin parse outcome contract (sq-bif.31) ([#2358](https://github.com/sparq-org/sparq/pull/2358))
- *(shaclc)* stress push parser chunk boundaries (sq-jwdwi) ([#2306](https://github.com/sparq-org/sparq/pull/2306))
- Fix SCS ontology empty-reference resolution [GPT-5.6]
- Merge remote-tracking branch 'origin/feat/sq-tonhr6-sparq-shaclc-fable' into HEAD
