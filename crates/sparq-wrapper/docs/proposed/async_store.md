# Async store proposal

Implementation: `src/proposed/async_store.rs` (gated behind the default-off
`proposed-async-store` feature).
Feature: `proposed-async-store` (default off).

Streaming wrapper over a remote or disk-backed store: `AsyncStore` wraps an
`AsyncStoreBackend`, `AsyncNode::out` / `AsyncNode::r#in` return a `NodeStream`
that wraps each term as it arrives, and dropping that stream drops the backend
stream and never polls it again — which cancels the traversal for a backend
meeting the `AsyncStoreBackend` requirement to start no I/O before the first
poll and to abandon it on drop. The wrapper pulls in no async runtime — `TermStream` is a
`futures_core::Stream`-shaped trait over `std::task` only. Sources:
rdfjs/wrapper issue #10 and draft PR #97.
