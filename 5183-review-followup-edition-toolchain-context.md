
### Cargo.toml

```toml
[workspace.package]
version = "0.1.1"
edition = "2021"
license = "MIT"
# [OPUS-4.8] (sq-qmth) MSRV floor 1.88. Upstream-driven: `geo@0.33.1` (a transitive dep
# of sparq-geo) requires rustc 1.88, and the released oxigraph parser stack
# (oxrdf/oxttl/oxrdfio/spargebra/… 0.2–0.4) requires 1.87. Verified empirically: the
# MSRV-scope workspace (--exclude sparq-py --exclude sparq-hdt) fails on 1.87 with ONLY
# `geo@0.33.1 requires rustc 1.88`, and builds clean on 1.88. Bump this AND the `msrv`
# job's toolchain pin in .github/workflows/ci.yml together; lower it only if geo (and the
# ox* stack) lower theirs.
rust-version = "1.88"
# Shared crates.io metadata (T20). Each publishable crate inherits these via
# `<field>.workspace = true` and adds its own `description`.
repository = "https://github.com/sparq-org/sparq"
homepage = "https://github.com/sparq-org/sparq"
keywords = ["rdf", "sparql", "semantic-web", "triplestore", "query-engine"]
categories = ["database-implementations", "parsing", "science"]

```

### crates/sparq-bench/Cargo.toml

```toml
[package]
name = "sparq-bench"
version.workspace = true
edition.workspace = true
license.workspace = true
publish = false

```

### rust-toolchain.toml — actual configuration (explanatory comments omitted)

```toml
[toolchain]
channel = "1.97.1"
profile = "minimal"
components = ["clippy", "rustfmt"]
targets = ["wasm32-unknown-unknown"]
```
