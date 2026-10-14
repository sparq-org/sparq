//! (sq-r5bv, gh-50) **The stable HTTP error/status contract** — what a sparq-server
//! client can rely on when classifying a response, and in particular **which statuses are
//! transient (retry may succeed) vs permanent (retrying the identical request cannot help)**.
//!
//! This module is **documentation only** — it exports no code. It is the single
//! consumer-facing source of truth for the response contract that was previously spread
//! across the README "Security posture", `skills/http-server/SKILL.md` and the per-handler
//! comments in [`crate::http`]. It is asserted end-to-end by
//! `tests/status_contract.rs`, so the table below cannot silently drift from the server's
//! actual behaviour.
//!
//! # Why this exists
//!
//! A consumer that retries on "transient" failures must know exactly which statuses sparq
//! emits for which conditions, and which of those are worth retrying. The motivating case
//! (gh-50): a federation client whose transient-error classifier was written against a
//! *different* SPARQL server keyed on that server's status mix and error text — `5xx` plus a
//! server-specific message substring. Against sparq that classifier is wrong in two ways:
//!
//! 1. sparq returns **`503` on a query timeout** and **`413` on a working-set row cap** —
//!    `413` is a status a `5xx`-only classifier never models, so a capped result would be
//!    misclassified as a *permanent* hard error when the correct action is "narrow the query"
//!    (and the cap, unlike a stream that simply stops, is an *honest refusal* — see below).
//! 2. sparq's error **bodies are sanitised** to a stable generic class string
//!    ([`crate::http`] `sanitized_error`): a classifier MUST NOT key on a free-text substring
//!    drawn from the engine, because sparq deliberately never emits engine/RDF/path detail in
//!    the body. **Classify on the status code** (and, where noted, a documented stable
//!    sentinel substring of the *generic* message), never on echoed input.
//!
//! # The contract
//!
//! Every error response is the structured envelope `{"error":"<message>"}` with
//! `Content-Type: application/json` (the one exception is `405`, which additionally carries an
//! `Allow` header). `<message>` is always a **stable generic class** — never the caller's
//! submitted query/update/RDF text, a fragment of the loaded graph, a server-side filesystem
//! path, or a secret. See the README "Error responses do not leak internals" section.
//!
//! ## Status -> meaning -> transient?
//!
//! | Status | Condition | Transient? | Client action |
//! | --- | --- | --- | --- |
//! | `200 OK` | query / GSP-read / EXPLAIN success | -- | consume body |
//! | `201 Created` | GSP `PUT`/`POST` created a graph that did not exist (advisory -- see note) | -- | success |
//! | `204 No Content` | SPARQL Update / GSP write committed (durable once acked, if `--persist`) | -- | success |
//! | `400 Bad Request` | malformed query / malformed UPDATE / malformed RDF body / malformed TPF pattern / `GET /sparql` with no `query` / unparsable `?generation` / not-yet-published generation | **permanent** | fix the request; do **not** retry as-is |
//! | `401 Unauthorized` | a write (or, with `--auth-token-read`, a read) without a valid Bearer token; carries `WWW-Authenticate: Bearer` + `Cache-Control: no-store`; **byte-identical for a missing vs a wrong token** | **permanent** | obtain/correct the token; do **not** retry as-is |
//! | `403 Forbidden` | (`service` feature) a non-`SILENT` `SERVICE` to a host the egress **allowlist** / default-deny SSRF policy refused (`--service-allow` / `SPARQ_SERVICE_ALLOW`, `sq-4w18`/`sq-iu0c`) -- a **policy** refusal, NOT a server fault | **permanent** | the endpoint is off the server's egress allowlist; do **not** retry -- ask the operator to allowlist it (or use `SERVICE SILENT` to tolerate the refusal) |
//! | `404 Not Found` | unknown route / GSP `DELETE` of a named graph that does not exist (a GSP `GET` of an absent named graph serves the EMPTY graph -- `200`, empty body -- it does not `404`; pinned by `tests/protocol.rs` + `tests/wire_contract.rs`) sq-fdurb | **permanent** | -- |
//! | `405 Method Not Allowed` | method not allowed on the route; carries `Allow` | **permanent** | use a listed method |
//! | `406 Not Acceptable` | a PRESENT, non-empty `Accept` naming no supported result media type and no wildcard (both result classes: SELECT/ASK and CONSTRUCT/DESCRIBE; absent / empty / `*/*` keeps the default, never a `406`) sq-fdurb | **permanent** | send a supported `Accept` (or none) |
//! | `410 Gone` | `?generation=N` aged out of the retention window -- default build (`sq-ci2d6`): the generation ring's **concurrency-retention window** (the last K generations older than current, always kept); WIDER under the opt-in `time-travel` feature | **permanent** | the snapshot is gone; re-read the current generation and restart pagination -- do not retry that pin |
//! | `413 Payload Too Large` | request **body** over `--max-body-bytes`; **result** over `--max-results` / `--max-query-rows` (row) / `--max-query-bytes` (byte-accounted, `sq-s5is`) working-set cap; gzip body over the `--max-decompress-ratio` zip-bomb cap | **permanent for the identical request** | narrow the query (e.g. add `LIMIT` / project fewer variables) or shrink the body -- retrying the same request fails identically |
//! | `415 Unsupported Media Type` | POST query/update with an unsupported `Content-Type` | **permanent** | send a supported content type |
//! | `429 Too Many Requests` | in-flight concurrency cap (`--max-concurrent`) tripped; the request was **shed, never started** | **TRANSIENT** | back off and retry -- the work was not done |
//! | `503 Service Unavailable` | query/UPDATE **timeout** (`--query-timeout`); **durable-write** error on the `--persist` mirror (write refused, NOT applied); a subscription registration refused for capacity | **TRANSIENT** | back off and retry; a timeout may also warrant simplifying the query |
//! | `500 Internal Server Error` | a handler panic (caught, connection kept alive) or an unclassified engine execution error | **treat as permanent** | a `500` is a server bug, not a load signal -- do not hot-retry; surface it |
//!
//! ## Transient vs permanent -- the rule a classifier should encode
//!
//! - **Transient (a retry of the identical request may succeed):** `429` and `503` ONLY.
//!   - `429` -- the request was load-shed *before it ran*; nothing happened, so a back-off
//!     retry is exactly right.
//!   - `503` -- three sub-causes, all retryable: a cooperative query/UPDATE **timeout**, a
//!     **durable-write** refusal (the write was *not* applied and not acked -- fail-closed, so a
//!     retry once durability recovers is safe and will not double-apply), and a subscription
//!     **capacity** refusal.
//! - **Permanent (retrying the identical request cannot help):** everything else --
//!   `400` / `401` / `403` / `404` / `405` / `406` / `410` / `413` / `415`.
//!   - **`403` is a policy refusal, not a fault.** A blocked `SERVICE` (egress allowlist) is a
//!     deliberate authorization decision; the same query against the same server config is
//!     refused every time. Retrying does not help -- the host must be allowlisted (or the
//!     query must use `SERVICE SILENT`, which swallows the refusal to the empty relation).
//!   - **`413` is the load-bearing one a `5xx`-only classifier gets wrong.** A result-cap
//!     `413` is an **honest refusal**, not a truncation and not a transient load signal: the
//!     same query against the same data crosses the same cap every time. The correct action is
//!     to *narrow the query* (add `LIMIT`, tighten the pattern) or raise the operator's cap --
//!     never a blind retry. A body-size or zip-bomb `413` is likewise permanent for the
//!     identical body.
//! - **`500` is NOT a transient class here.** It is a caught panic or an unclassified internal
//!     error -- a defect, not back-pressure. A client may surface it, but should not treat it as
//!     "retry will clear it" the way it treats `503`. (A classifier MAY choose a single bounded
//!     retry for `500` to ride out a flaky deploy, but it must not loop on it.)
//!
//! ## Honest boundaries (do not over-rely)
//!
//! - **No `Retry-After` header.** sparq does not currently emit `Retry-After` on `429` or
//!   `503`; a client picks its own back-off. (If one is added later it will only *narrow* the
//!   advice, never contradict the status class above.)
//! - **Classify on status, not body text.** Bodies are sanitised generic class strings. The
//!   only message substrings a client may match are the *stable generic* ones this contract
//!   names (e.g. a `503` timeout body contains `timed out`; a row-cap `413` body contains
//!   `row limit`; a byte-cap `413` body contains `byte limit`) -- and even those are a
//!   convenience on top of the authoritative status code,
//!   never a substitute for it. Never match on engine/RDF/path detail: sparq never puts it in
//!   the body.
//! - **GSP created-vs-replaced (`201` vs `200`/`204`) is advisory.** PUT/POST sample graph
//!   existence racily; treat the create/replace distinction as a hint, never as correctness.
//! - **A timeout is wall-clock-guaranteed.** The `503` for `--query-timeout` is enforced at the
//!   HTTP layer with a hard grace even if the engine is mid-stretch, on *all* request forms
//!   (query, GSP-read and UPDATE), so a slow request cannot hang a client past the deadline.
//!
//! See also: `docs/http-wire-contract.md` (the versioned v1 HTTP wire contract this table is
//! part of, pinned end-to-end by `tests/wire_contract.rs` -- sq-fdurb, gh-1416),
//! the README "Security posture" (auth, bind, header and error-leak posture),
//! `skills/http-server/SKILL.md` (the full endpoint + hardening reference), and the inline
//! `engine_error_response` / `update_rejection_response` / `timeout_response` mappers in
//! [`crate::http`] that this contract describes.
