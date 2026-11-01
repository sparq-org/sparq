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
The ticket travels in the socket URL’s query string; deployments should keep query
strings for the socket path out of access logs.
The container must exist. Read permits observation; adding members also permits
sending document changes (WAC Append or Write in Solid, `create` in LWS).
With `odrl-authz` and an attached gate, Solid room access composes the ODRL
verdict: Deny overrides every room capability, and Permit extends Read only.
LWS room access uses its normal grants.
When a socket upgrade includes `Origin`, it must pass the configured CORS origin
policy. In Solid it must also equal the ticket request's recorded Origin, when
one was recorded. An upgrade without Origin is permitted for non-browser clients.
Read on the room container authorizes reading every log resource in it; the relay
does not evaluate per-resource ACLs on individual log files.

Sockets speak the y-websocket binary message protocol: sync step 1, sync step 2,
updates and awareness. Both endpoints exchange state vectors and missing updates
using Yjs v1 encoding. Read peers can send awareness. A read peer that sends new
document changes is disconnected. Access is checked again at connection and
every ten seconds. Losing read ends the connection; losing write changes it to
an observer for the rest of that connection. Reconnect with a fresh ticket after
an access change. Ping frames are sent every thirty seconds. Closing a connection
removes the awareness states it first announced and broadcasts the removal immediately.
The connection also releases those presence IDs, including previously cleared states.
Awareness byte accounting follows the protocol clocks and counts the retained states.
Peers may forward awareness updates. Replays follow the normal clock rules, and
forwarding does not transfer responsibility for close cleanup to the forwarding peer.
Clock tombstones survive disconnects separately from active presence slots, with
at most `AWARENESS_CLIENTS` remembered clocks per room, evicting oldest tombstones
first. A non-null state must advance a remembered clock.

| Close code | Meaning |
| --- | --- |
| 4401 | Invalid, expired, reused or mismatched ticket |
| 4403 | Access refused |
| 4413 | Message or room document exceeds its byte limit |
| 4429 | Room, peer or queue capacity reached |

Ticket capacity returns HTTP `429`. Invalid collaboration messages use `1002`;
storage failures use `1011`. Other socket receive failures drop the connection.
Clients should reconnect and perform a complete Yjs sync.

The relay broadcasts accepted updates before persistence. Live clients keep
those updates locally and fall back to appending log resources on socket loss;
reconnecting state-vector synchronization reconciles divergence. There is no
socket-level durable acknowledgement. Do not discard a client's local document
because a broadcast arrived.

Updates are coalesced for each author and flushed on a 500 ms schedule. Separate
authors produce separate resources through the mode's ordinary creation path.
LWS creator metadata records the author; Solid retains the same server metadata
beside the binary resource. A failed flush remains queued, with exponential
retry delays capped at thirty seconds while the room remains live. Retries wait
when another retained resource would exceed a replay or file bound. A persistence
retry must still pass the ordinary access check. Internal create notifications
bring log-mode appends into live rooms; no periodic container scan is used.
Queued updates are timestamped at least as late as the newest observed log header
when flushed, including after a snapshot arrives during a retry. If a newer
snapshot arrives during a successful create, the batch remains queued and is
persisted again at or after that snapshot's timestamp.

Every log resource has a 16-byte envelope before its Yjs v1 payload. The first
four bytes identify an update (`JYU1`) or snapshot (`JYS1`); the next eight hold
an unsigned epoch-millisecond timestamp in big-endian order; the final four hold
an unsigned ordering suffix in big-endian order. Relay updates use random
suffixes; replacement snapshots advance the folded snapshot key. Loaders order these headers,
regardless of filenames. Resource URIs follow each mode’s ordinary creation path.
Apply the newest snapshot first, then update resources
whose timestamp is at least the snapshot's. Orphaned older updates are harmless.
Ingest follows the same rules: it skips updates older than the selected snapshot's
timestamp and snapshots preceding its `(timestamp, suffix)` ordering key.
After the last peer leaves, the room remains loaded for sixty seconds. Unloading
waits for queued writes to complete. Compaction also runs with peers connected
when `COMPACT_AFTER` is reached or retained log bytes reach twice `DOCUMENT_BYTES`.
Compaction requires a remembered writer with current read, create and delete
authority, checked before refreshing the room log and again after the refresh. Attempts that make no progress
wait one second initially, doubling to at most sixty seconds. A change to the
remembered writer set resets this delay; further edits from an already remembered
writer do not. Otherwise admission remains bounded by the file and replay-byte limits.
The replacement snapshot sorts after all folded snapshots. If advancing a maximum
suffix requires a new timestamp, that timestamp must have no retained update;
compaction waits when an ordering boundary cannot be established.
Only confirmed snapshot creation permits ordinary authorized deletion. Folded
snapshots are deleted first; if any snapshot deletion fails, all folded update
files remain for a later cycle. Restart reconstructs a room from those same files.
Loading and compaction do not hold the service-wide room admission lock during
storage operations. Reload also reads storage outside the room lock, so connected
peers can edit, exchange awareness and sync while a listing is pending. Its refreshed
inventory preserves files recorded during the read and recomputes retained bytes
and update counts. A room being unloaded refuses new joins until retirement ends.

Limits can be set before router assembly. Library embedders may set
`LdpState::crdt_limits` or `LwsConfig::crdt_limits` using
`sparq_lws_ysync::Limits`. Native environment variables are positive integers no larger than 1073741824:

| Variable prefix `SOLID_SERVER_CRDT_` | Default | Effect |
| --- | --- | --- |
| `MAX_ROOMS` | 128 | Loaded rooms per service |
| `PEERS_PER_ROOM` | 32 | Connections, pending authors and remembered writers per room |
| `MESSAGE_BYTES` | 1048576 | Incoming binary messages; retained awareness bytes |
| `FRAME_MESSAGES` | 32 | Protocol records in one incoming binary WebSocket message |
| `DOCUMENT_BYTES` | 16777216 | Merged encoded room state and pending updates |
| `TICKETS_PER_AGENT` | 8 | Outstanding tickets for one subject |
| `MAX_TICKETS` | 1024 | Outstanding tickets for the service |
| `MAX_LOG_FILES` | 4096 | Room listing and retained log entries |
| `AWARENESS_CLIENTS` | 256 | Active awareness IDs; separately bounded clock tombstones |
| `COMPACT_AFTER` | 64 | Update files before live or unload compaction |

Retained log bodies, in-flight writes and queued updates reserve replay bytes
before admission. A new update exceeding four times `DOCUMENT_BYTES` is refused
with capacity before broadcast. Loading also bounds aggregate log bytes to the
same limit and
container-listing bytes to 8192 times `MAX_LOG_FILES`.
Replies to one binary message share a byte budget equal to `DOCUMENT_BYTES` plus
`MESSAGE_BYTES`; exceeding it ends the connection with the capacity code.
Ticket request bodies are limited to 8192 bytes as they are read. Pending authors
reserve log entries before their updates are admitted, including an additional
entry when an update overlaps that author's in-progress flush.

The crate `sparq-lws-ysync` is internal and has no HTTP framework dependency.
Its store/authorizer traits are supplied by the native server adapter; standalone
embedders must provide equivalent attributed creation and confirmed deletion.
