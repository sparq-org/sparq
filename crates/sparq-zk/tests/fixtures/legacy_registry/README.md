# Legacy registry fixtures

`base-5e7e008e.nq` holds four `<urn:sparq:zk>` registry entries signed by the
issuance path of commit `5e7e008e`, before the issuer signature covered the
commitment method. Each block starts with a `# name` line.

| Block | Method recorded | Default verification today |
| --- | --- | --- |
| `string-canonical-plain` | `string-canonical` | verifies (pinned) |
| `string-canonical-status` | `string-canonical` | verifies (pinned) |
| `dual-leaf-v1-plain` | `dual-leaf` v1 | refused, also when relabelled |
| `dual-leaf-v1-status` | `dual-leaf` v1 | refused, also when relabelled |

The two `string-canonical` blocks are the entries pinned in
`registry::LEGACY_METHOD_UNBOUND_ENTRIES`; the `registry` unit tests check the
pinned digests against this file.

## How the bytes were made

`generate_at_5e7e008e.rs` is the whole program. It was built as the `main.rs`
of a standalone crate outside the repository, against a checkout of `5e7e008e`:

```toml
[package]
name = "legacy-registry-gen"
version = "0.0.0"
edition = "2021"
publish = false

[dependencies]
sparq-zk = { path = "<checkout of 5e7e008e>/crates/sparq-zk" }
oxrdf = { version = "0.3", features = ["rdf-12"] }

[workspace]
```

with that checkout's `Cargo.lock`, and `cargo run -q > base-5e7e008e.nq`. The
program asserts that every entry verifies under the code it ran against.
