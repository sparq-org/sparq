# sparq-lws-ysync

<!-- internal-stub -->

Internal, unpublished Yjs relay shared by the native Solid and LWS servers.
`yrs` supplies Yjs v1 update merging, state vectors and sync/awareness encoding.
The crate has no HTTP framework dependency. `RoomStore` and `Authorizer` adapt
ordinary resource operations and the active protocol's access decisions.

Rooms broadcast accepted edits before asynchronous, per-author log writes.
Each log resource begins with a fixed envelope: a four-byte update or snapshot
marker, a big-endian millisecond timestamp and a big-endian random suffix,
followed by a Yjs v1 update. Loading orders envelopes, selects the newest
snapshot and applies updates at or after its timestamp. Names are only hints.
Compaction confirms snapshot creation before deleting any folded resources.

`Relay`, `Limits`, `Session` and `Changes` support bounded room admission,
single-use tickets, presence cleanup, retried flushes and event-driven ingestion.
The native HTTP adapter is enabled with `sparq-lws-core/crdt-sync`.
See `skills/solid-lws-server/references/collaboration.md` for the wire contract.

Validation: `cargo test -p sparq-lws-ysync` and
`cargo test -p sparq-lws-core --features crdt-sync`.
The optional JavaScript peer fixture is documented under
`crates/sparq-lws-core/tests/fixtures/crdt-interop/README.md`.
