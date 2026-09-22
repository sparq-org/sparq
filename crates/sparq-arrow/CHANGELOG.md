# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/sparq-org/sparq/releases/tag/v0.1.0) - 2026-09-03

### Added

- *(arrow)* CSV connector round-trip (sq-lsp7k) [FABLE-5] ([#2376](https://github.com/sparq-org/sparq/pull/2376))
- *(arrow)* add schema-only variable readers [GPT-5.6] ([#2245](https://github.com/sparq-org/sparq/pull/2245))
- *(arrow)* add IPC stream byte round-trip [GPT-5.6] ([#2214](https://github.com/sparq-org/sparq/pull/2214))
- *(arrow)* import record batches [GPT-5.6]
- *(sparq-py)* opt-in Graph.query_arrow() -> pyarrow.Table over sparq-arrow (sq-lt1ml) [OPUS-4.8] ([#1238](https://github.com/sparq-org/sparq/pull/1238))
- *(sparq-arrow)* opt-in Apache Arrow columnar export of SELECT results (sq-v78l4) [OPUS-4.8] ([#1224](https://github.com/sparq-org/sparq/pull/1224))

### Other

- *(sparq-arrow)* cover malformed import errors (sq-bif.18) [GPT-5.6] ([#2337](https://github.com/sparq-org/sparq/pull/2337))
- *(arrow)* property-test IPC round trips (sq-s0dub) [GPT-5.6] ([#2301](https://github.com/sparq-org/sparq/pull/2301))
- Add metadata-only Parquet row count [GPT-5.6] ([#2261](https://github.com/sparq-org/sparq/pull/2261))
- Add Parquet round-trip to sparq-arrow [GPT-5.6]
- [FABLE-5] merge origin/main into bench-arrow-interop-gpt56 (resolve sparq-arrow Cargo.toml)
- *(arrow)* property-test RecordBatch schema fidelity [GPT-5.6] ([#2079](https://github.com/sparq-org/sparq/pull/2079))
- *(deps)* Bump arrow-array from 55.2.0 to 59.0.0 ([#1280](https://github.com/sparq-org/sparq/pull/1280))
