<!-- [OPUS-4.8] sq-im8u — single-source include wrapper. The quickstart command block is
{{#include}}d verbatim from the canonical README.md `quickstart-cli` anchor, and the library
example is {{#rustdoc_include}}d from the tested examples/quickstart.rs (sq-384j) — so neither
can drift. Only one-line intros and the contributor-gate block (whose canonical prose lives in
CONTRIBUTING.md) are written here; no prose is duplicated from the README. -->

# Install & build from source

sparq is a Rust workspace. The minimum supported Rust version (MSRV) is tracked in CI; a recent
stable toolchain works. Build the release binaries, then query a file, build a persistent on-disk
store, or run the HTTP server:

{{#include ../../../README.md:quickstart-cli}}

## Use it as a library

The snippet below is **not hand-written prose** — it is the `quickstart` region of
[`crates/sparq-engine/examples/quickstart.rs`](https://github.com/sparq-org/sparq/blob/main/crates/sparq-engine/examples/quickstart.rs),
embedded verbatim via mdBook's `{{#rustdoc_include}}`. That file is compiled and run by
`cargo test -p sparq-engine --examples`, so this example cannot silently drift from the
public API:

<!-- [OPUS-4.8] sq-384j — tested-example embedding. The fence is `rust,ignore` so
`mdbook test` does NOT recompile the snippet standalone: it references the workspace
crates `sparq-core`/`sparq-engine`, which a bare rustdoc invocation cannot resolve
(that is the "mdbook-keeper only if inline blocks need workspace deps" case from the
bead — avoided here by letting `cargo test` be the compile-and-run gate on the real
file). The anchor keeps the guide a fragment of a passing test. -->
```rust,ignore
{{#rustdoc_include ../../../crates/sparq-engine/examples/quickstart.rs:quickstart}}
```

## The contributor gate

If you are working on the engine itself, the gate for landing a change is a green full-workspace
build, test, lint, and the conformance / performance ratchets:

```sh
cargo build --workspace
cargo test  --workspace
cargo clippy --workspace --exclude sparq-py --all-targets -- -D warnings
```

`cargo fmt --all --check` runs in CI too, but it is **informational, not part of the gate**:
the one-time workspace reformat is still pending, so the check reports pre-existing diffs in
files you did not touch. Format what you touched and leave the rest.

See [`CONTRIBUTING.md`](https://github.com/sparq-org/sparq/blob/main/CONTRIBUTING.md) for the full
contributor workflow and the conformance ratchet details — that file is the canonical source for
the gate.
