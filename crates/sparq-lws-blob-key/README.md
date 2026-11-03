# sparq-lws-blob-key

Internal, dependency-free blob-key grammar shared by the experimental LWS server
and its filesystem and object backends (`publish = false`). Supports native and
wasm targets.

## 🚀 Quickstart

```rust
let key = sparq_lws_blob_key::format([0xab; 16], true);
assert_eq!(sparq_lws_blob_key::parse(&key).unwrap().1, true);
```

## ✨ Features

`POOL_MARK` is `$pool$`; `PREFIX` is `b1-`. `parse(&str)` accepts exactly
`($pool$)?b1-<32 lowercase hex>` and returns the bare key and pool flag.
`format([u8; 16], bool)` formats caller-supplied suffix bytes as a canonical key.
Callers supply randomness and retain responsibility for never reusing keys.

## 📚 Learn more

See the [server usage guide](../../skills/solid-lws-server/SKILL.md).
The benchmark registry contains a correctness-only stub for this crate.

## License

MIT OR Apache-2.0.
