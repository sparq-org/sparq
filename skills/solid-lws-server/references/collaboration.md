# Yjs collaboration

Build the native server with `--features crdt-sync`. This default-off feature
mounts the same Yjs service in both modes; the native container image enables it.
It is independent of the RDF CRDT capability crates. A mounted service advertises
`https://jeswr.org/ns/solid-sd#CrdtSyncService`, with `profile` pointing to
`https://jeswr.org/ns/solid-sd#y-protocol-v1`, its ticket URL as `serviceEndpoint`
and its socket template as `roomTemplate`.

| Mode | Ticket endpoint | Room template |
| --- | --- | --- |
| Solid | `POST <base>/.crdt/tickets` | `wss://<host>/.crdt/socket?room={room}` |
| LWS | `POST <base>/.lws/crdt/tickets` | `wss://<host>/.lws/crdt/socket?room={room}` |

The template follows the configured public origin. HTTP origins produce `ws:`
URLs. Substitute the percent-encoded container URL for `{room}`.
Send an authenticated JSON request such as `{"room":"https://pod.example/room/"}`
to the ticket endpoint using the mode's ordinary authentication. A successful
response is `201 application/json`, with `url` containing the socket URL and
its `ticket` query parameter, and `expires` containing a UTC RFC 3339 timestamp.
The response carries `Cache-Control: no-store`. Open exactly that returned URL.
Tickets contain 32 random bytes, expire after 60 seconds and can be used once.
The container must exist. Read permits observation; adding members also permits
sending document changes (WAC Append or Write in Solid, `create` in LWS).

Sockets speak the y-websocket binary message protocol: sync step 1, sync step 2,
updates and awareness. Both endpoints exchange state vectors and missing updates
using Yjs v1 encoding. Read peers can send awareness. A read peer that sends new
document changes is disconnected. Access is checked again at connection and
every ten seconds. Losing read ends the connection; losing write changes it to
an observer for the rest of that connection. Reconnect with a fresh ticket after
an access change. Ping frames are sent every thirty seconds. Closing a connection
removes the awareness states it announced and broadcasts the removal immediately.

| Close code | Meaning |
| --- | --- |
| 4401 | Invalid, expired, reused or mismatched ticket |
| 4403 | Access refused |
| 4413 | Message or room document exceeds its byte limit |
| 4429 | Room, peer or queue capacity reached |

Ticket capacity returns HTTP `429`. Invalid protocol input uses `1002`; storage
failures use `1011`. Clients should reconnect and perform a complete Yjs sync.

The relay broadcasts accepted updates before persistence. Live clients keep
those updates locally and fall back to appending log resources on socket loss;
reconnecting state-vector synchronization reconciles divergence. There is no
socket-level durable acknowledgement. Do not discard a client's local document
because a broadcast arrived.

Updates are coalesced for each author and flushed on a 500 ms schedule. Separate
authors produce separate resources through the mode's ordinary creation path.
LWS creator metadata records the author; Solid retains the same server metadata
beside the binary resource. A failed flush remains queued, with exponential
retry delays capped at thirty seconds while the room remains live. A persistence
retry must still pass the ordinary access check. Internal create notifications
bring log-mode appends into live rooms; no periodic container scan is used.

Every log resource has a 16-byte envelope before its Yjs v1 payload. The first
four bytes identify an update (`JYU1`) or snapshot (`JYS1`); the next eight hold
an unsigned epoch-millisecond timestamp in big-endian order; the final four hold
a random unsigned suffix in big-endian order. Loaders order these headers,
regardless of filenames. Apply the newest snapshot first, then update resources
whose timestamp is at least the snapshot's. Orphaned older updates are harmless.
After the last peer leaves, the room remains loaded for sixty seconds. At the
compaction threshold, an admitted writer is used to create a snapshot during
unloading. Only confirmed creation permits ordinary authorized deletion of the
folded files. Without confirmation the original resources remain. Restart reconstructs
a room from those same files.

Limits can be set before router assembly. Library embedders may set
`LdpState::crdt_limits` or `LwsConfig::crdt_limits` using
`sparq_lws_ysync::Limits`. Native environment variables are positive integers:

| Variable prefix `SOLID_SERVER_CRDT_` | Default | Effect |
| --- | --- | --- |
| `MAX_ROOMS` | 128 | Loaded rooms per service |
| `PEERS_PER_ROOM` | 32 | Connections per room; pending authors per room |
| `MESSAGE_BYTES` | 1048576 | Incoming binary messages; retained awareness bytes |
| `DOCUMENT_BYTES` | 16777216 | Merged encoded room state and pending updates |
| `TICKETS_PER_AGENT` | 8 | Outstanding tickets for one subject |
| `MAX_TICKETS` | 1024 | Outstanding tickets for the service |
| `MAX_LOG_FILES` | 4096 | Room listing and retained log entries |
| `AWARENESS_CLIENTS` | 256 | Distinct awareness IDs retained per loaded room |
| `COMPACT_AFTER` | 64 | Update files before unload compaction |

Loading also bounds aggregate log bytes to four times `DOCUMENT_BYTES` and
container-listing bytes to 8192 times `MAX_LOG_FILES`.

The crate `sparq-lws-ysync` is internal and has no HTTP framework dependency.
Its store/authorizer traits are supplied by the native server adapter; standalone
embedders must provide equivalent attributed creation and confirmed deletion.
