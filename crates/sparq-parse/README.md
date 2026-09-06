<!-- [OPUS-4.8] sq-inzv: internal-stub README for a publish=false crate. -->
# sparq-parse

**Compressed serialization of query results** for [sparq](../../README.md):
chunk-at-a-time gzip / zstd encoding of serialized result chunks, built around
one format property — *multi-member gzip and multi-frame zstd are valid single
streams*. Each chunk is compressed as an independent gzip member / zstd frame and
the members are concatenated in order, so the result decodes as **one stream with
stock decoders** (browser `Content-Encoding: gzip`, `MultiGzDecoder`, `gzip -d`,
the `zstd` CLI) while chunks are compressed in parallel as they are produced —
compression overlaps serialization instead of following it.

Hard constraint: this crate must **not** enter `sparq-wasm`'s dependency graph
(flate2 / zstd / rayon stay out of the browser bundle).

<!-- [GPT-5.6] sq-98w7z.2; [OPUS-5] measured verdict + precedence live in rustdoc. -->
Gzip encoder backend is a build-time choice: `miniz_oxide` by default, opt-in
`zlib-rs` (measured: much faster at `-6`, trades ratio at `-1`) or `zlib-ng`.

> **Internal crate — not on crates.io** (`publish = false`). The design is gated
> on a measured baseline; it is consumed inside the workspace, not as a
> standalone public API.

Design + baseline:
[`research/custom-parsers-D4-compressed-serialization.md`](../../research/custom-parsers-D4-compressed-serialization.md).
Contributing: [`AGENTS.md`](../../AGENTS.md).

## License

[MIT](../../LICENSE).
