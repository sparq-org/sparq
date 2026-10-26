# LWS notification streams

The native LWS NotificationService at `/.lws/subscriptions/` advertises
`WebhookSubscription`, `SseSubscription` and `WebSocketSubscription`. The Solid
notification service uses its existing protocol. These LWS bindings follow the
[JLWS subscription API](https://jeswr.github.io/lws-spec/#subscription-api).

Create a subscription with an authenticated `POST application/lws+json`:

```json
{"type":"SseSubscription","topic":["https://pod.example/projects/"]}
```

`topic` is a nonempty array; the subscriber must read every topic. Container
subscriptions cover descendants recursively, with each changed resource checked
separately. `expires` is an optional RFC 3339 datetime. SSE and WebSocket
subscriptions without it expire after 24 hours. These two types reject `inbox`.
They require an HTTPS base, except when `allow_insecure_fetch` is explicitly set
for local testing (HTTP then produces HTTP SSE and `ws:` socket URLs).

A creation response includes `id`, `subscription` (the same inspection/cancellation
URI), `type`, `topic`, `expires`, and `authMode: "capability"`. SSE includes
`source` and `receiveFrom`, both the same HTTPS stream URL, and
`retention: "PT10M"`. WebSocket includes a `wss:` `receiveFrom`.
Stream documents carry inline JSON-LD definitions for the JLWS binding terms.
The service accepts short type names and their LWS, JLWS and solid-sd IRI forms.

Connection URLs are disclosed only in the creation response. Keep that response:
GET of the subscription and GET of its listing never return the URL or token.
The URL carries 32 random bytes encoded as base64url. The record holds its SHA-256
hash. The capability authenticates its own connection; send no bearer token.
An in-memory hash-to-subscription index is maintained with the subscription records,
including on restart. A connection probes this digest index without scanning other
principals' subscriptions; no raw token is retained in the index.
Expiry, cancellation, deactivation and quarantine invalidate it. DELETE the `id`
to cancel; subscription records and quotas are shared with webhooks.

GET `source` with `Accept: text/event-stream`. A browser EventSource may connect
cross-origin without credentials through the LWS CORS layer. Each event is:

```text
id: <epoch>-<sequence>
event: notification
data: <the same Notification JSON envelope used by webhooks>

```

There is one activity per event, with no resource content or batching. The epoch
changes on restart and the sequence increases per subscription. A heartbeat comment
is sent every 30 seconds. Each subscription retains at most its last 256 events,
10 minutes of history, and 128 KiB of serialized envelope bodies
(`RETAINED_BYTES`); all limits apply, with the oldest events evicted first. Reconnect with
`Last-Event-ID`: a retained id replays subsequent events after rechecking their
resource access. An unknown, evicted, aged-out or other-epoch id sends `event: reset` with
an empty data field, then continues live. Read the topics again after reset.

Only one connection may be open per subscription; connecting again closes the
previous one. Each connection's live queue holds at most 64 events. A slow consumer
is closed and can reconnect with its last id. Replay uses a separate bounded queue,
which is drained completely before live frames. Registration and the replay snapshot
share the publication gate; replay authorization and queue sends happen after releasing it.
An unread replay therefore permits live publication and webhook dispatch to proceed.
Webhooks for each change enter their delivery queue before stream publication is awaited.
There are at most 1024 open connections across both bindings per storage instance,
and a subscriber cannot exceed its subscription quota in open connections.
Excess connections return 429. Open bodies do not retain request admission permits
and are outside the request-response timeout.

WebSocket sends one Notification envelope per text frame, pings every 30 seconds,
and ignores client application frames. It has no replay. A `1013` close asks the client
to reconnect with the same capability and reconcile the topics; `4403` means subscribe
again. After another close or a transport end without a close frame, inspect the
subscription: reconnect with the same capability if it remains, or subscribe again
if it was withdrawn. Replacing a connection keeps the capability.

Both bindings prepare authorization while the change's locks are held and check
it again before retaining or sending. Delete checks use the pre-delete access
snapshot. Open connections recheck topic access every 10 seconds; losing some
topics filters their events. Losing all topic access sends terminal `event: revoked`
(SSE) or close code 4403 (WebSocket), closes the connection and deactivates the
subscription. Connect returns `403` for that confirmed denial. Subscribe again after
withdrawal. An unreadable absent topic is denied just like an unreadable stored one.

A set-aside topic, a store error or an access-check timeout makes the check undecided
when no other topic is confirmed readable. On connect this returns `503` with
`Retry-After` and the body `authorization could not be evaluated`. An open connection
ends without withdrawing the subscription: SSE ends without `event: revoked`, and
WebSocket sends `1013`. Keep the capability and reconnect after recovery. An SSE
client can inspect the subscription to confirm withdrawal when it receives an end
without a control event. Timer checks have a bounded deadline; an unfinished check
ends the connection for retry rather than deciding revocation.

Stream records add `kind` (`Sse` or `WebSocket`) and `capability_hash` and omit `inbox`. Webhook records
retain their prior encoding; the reader accepts that legacy shape. Event history
and connections are in memory and are not restored after restart.
