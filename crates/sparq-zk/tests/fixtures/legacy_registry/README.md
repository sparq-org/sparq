# Legacy registry fixtures

`base-5e7e008e.nq` holds four `<urn:sparq:zk>` registry entries signed by the
issuance path of commit `5e7e008e`, before the issuer signature covered the
commitment method. Each block starts with a `# name` line.

| Block | Method recorded |
| --- | --- |
| `string-canonical-plain` | `string-canonical` |
| `string-canonical-status` | `string-canonical` |
| `dual-leaf-v1-plain` | `dual-leaf` v1 |
| `dual-leaf-v1-status` | `dual-leaf` v1 |

Every entry here fails verification today, as recorded and under any other
`zk:scheme` (including a `dual-leaf` v1 entry relabelled `string-canonical`),
because a registry signature is verified only over the method-bound message.
The `registry` unit tests check this, and also that each signature is still
valid over the method-independent message it was made over.

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
