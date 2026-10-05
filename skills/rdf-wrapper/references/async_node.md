# Async node extension

Implemented behind the default-off `proposed-async-node` feature in
`proposed::async_node`.

Async mapped reads over `AsyncNode`: `required` / `optional` pull at most two
streamed values (the second already proves a violation, so the remote result
set is abandoned, not drained) and reuse the crate's `CardinalityError`;
`many` maps every streamed value. `live_set` returns an `AsyncLiveSet` whose
`values` / `contains` / `insert` / `remove` await the backend on every call.
Mappers receive an `AsyncNode`, so term identity stays synchronous. The
feature implies `proposed-async-store` and `proposed-cardinality`. Source:
rdfjs/wrapper draft PR #98. <!-- [FABLE-5] sq-1rg2q.9 -->
