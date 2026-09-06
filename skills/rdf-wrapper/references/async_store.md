# Async store extension

The implementation is isolated behind the default-off
`proposed-async-store` feature in `proposed::async_store`.

`AsyncStore` wraps an `AsyncStoreBackend` — a remote endpoint or an
out-of-core on-disk index — and keeps the focus/traverse shape of the
synchronous `Store`. `AsyncNode::out` / `AsyncNode::r#in` return a `NodeStream`
that wraps each term as it arrives rather than collecting a result set, and
dropping the stream drops the backend stream and never polls it again —
cancelling the traversal for a backend that meets `AsyncStoreBackend`'s
requirement to start no I/O before the first poll and to abandon it on drop.
No async runtime is pulled in:
`TermStream` is a `futures_core::Stream`-shaped trait built on `std::task`
only. Sources: rdfjs/wrapper issue #10 and draft PR #97.
