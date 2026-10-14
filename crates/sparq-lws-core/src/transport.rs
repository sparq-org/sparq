// AUTHORED-BY Claude Opus 4.8
//! Transport-layer DoS hardening — HTTP/2 stream/reset caps + slowloris header timeout + a global AND
//! a per-source concurrent-connection cap + TLS-handshake timeout + a per-connection idle-keepalive
//! timeout + a max-requests-per-connection reuse cap.
//!
//! ## Why (the transport-DoS gap the application layers leave open)
//! The overload layers ([`crate::overload`]) and the per-IP rate limiter ([`crate::rate_limit`]) gate
//! **admitted requests** — they sit ABOVE hyper, so they only ever see a fully-parsed `http::Request`.
//! Several DoS classes never produce such a request, so those layers never fire:
//! - **HTTP/2 Rapid-Reset (CVE-2023-44487).** A client opens a stream and immediately `RST_STREAM`s
//!   it, in a tight loop, over one connection. Each stream creates + cancels server-side work without
//!   the connection's concurrent-stream count ever rising — bypassing a `max_concurrent_streams`
//!   limit, and never reaching the application admission layer (the request is reset before it
//!   completes). The defence is at the h2 layer: cap the number of in-flight *reset* streams and
//!   `GOAWAY` when exceeded, AND bound `max_concurrent_streams` so a single connection cannot pin
//!   unbounded server work.
//! - **Slowloris (slow-header trickle).** A client opens a TCP connection and dribbles request-header
//!   bytes one at a time, never completing the head. The [`crate::overload`] request *timeout* covers
//!   total request time AFTER the request is parsed — it never starts, because hyper is still reading
//!   the head. The defence is a **header-read timeout** at the hyper layer (drop a connection that
//!   has not sent a complete header set in time) plus a **concurrent-connection cap** so a flood of
//!   such half-open connections cannot exhaust file descriptors / memory, plus a bounded **TLS
//!   handshake timeout** so a connection that stalls the handshake (never completing it) cannot pin a
//!   connection permit, and an **h2 keep-alive PING** (interval + ack-timeout) so a DEAD-peer h2
//!   connection (host gone without a FIN) that is holding a permit is reclaimed. The global cap alone
//!   lets ONE source hold ALL the slots, so it is paired with a **per-source (per-IP) connection cap**
//!   (default-on, internal-IP-exempt — see [`ConnectionLimiter::with_per_ip_cap`]) so a single IP's
//!   half-open flood is bounded well below the global ceiling.
//! - **Slow-body (slow-POST trickle).** A client sends a complete header set (so `header_read_timeout`
//!   is satisfied) then dribbles the request BODY one byte at a time. hyper exposes NO server-side
//!   body-read timeout knob, so this is bounded by the composition of the OTHER layers rather than a
//!   dedicated one: the [`crate::overload`] **request timeout** (default 30s) caps the TOTAL request
//!   time including the body read, the explicit [`crate::body_limit`] **body-size cap** (default 2 MiB)
//!   bounds how many bytes a single slow body can ever be, and the connection caps bound how many such
//!   trickles a source may hold at once. Together these bound a slow-body attacker to
//!   `≤ body_limit` bytes over `≤ request_timeout`, on `≤ max_per_ip` connections — a finite, small
//!   resource footprint. (A dedicated min-throughput body reader is deliberately NOT hand-rolled: it
//!   would duplicate what the request timeout already guarantees and add a bespoke body-stream wrapper
//!   to the hot path.)
//! - **Idle keep-alive hold (the two missing-guard companions).** Two further gaps the bounds above
//!   leave open, both enforced here (NOT hyper builder knobs — hyper 1.x exposes neither; see below):
//!   - a peer that completes a request, gets its response, then holds the keep-alive connection open
//!     sending NOTHING further — `header_read_timeout` only bounds a PARTIAL head (bytes started), and
//!     the h2 PING only reclaims a DEAD peer, so neither closes a LIVE peer on a fully-idle connection.
//!     The defence is a per-connection **idle-keepalive timeout** at the IO layer ([`IdleTimeoutStream`]):
//!     no read/write progress for the window ⇒ close + reclaim the permit. It is **DEFAULT-OFF** — an
//!     IO-layer idle bound cannot exclude an UPGRADED long-lived stream (a WebSocketChannel2023 receive
//!     socket that sits quiet until a notification), so it is opt-in (roborev High).
//!   - unbounded keep-alive REUSE of one connection — bounded by a **max-requests-per-connection** cap
//!     ([`MaxRequestsService`]) that sets `Connection: close` after N requests (HTTP/1.1 reuse
//!     rotation; default OFF).
//!
//! ## What hyper provides vs what we add (the rapid-reset accounting)
//! The in-tree `hyper` 1.x + `h2` 0.4.x already ship the CVE-2023-44487 reset-accounting:
//! - `max_pending_accept_reset_streams` (hyper issue #2877) — defaults to **20**: a connection that
//!   accumulates more than this many streams that were reset before being accepted gets a `GOAWAY`.
//!   This is the PRIMARY rapid-reset cap and is ON BY DEFAULT.
//! - `max_local_error_reset_streams` (RUSTSEC-2024-0003) — defaults to **1024**: bounds locally-reset
//!   streams kept for error propagation.
//!
//! So an UNCONFIGURED hyper is already not vulnerable to the original rapid-reset CVE. What this
//! module ADDS on top is: making the cap an EXPLICIT, env-tunable, TESTED invariant of THIS crate
//! (so a dependency bump cannot silently change it), and pairing it with an explicit
//! `max_concurrent_streams` ceiling (hyper's default is 200; we set it explicitly so it is owned +
//! documented). The reset cap stays at hyper's secure default unless an operator overrides it.
//!
//! ## Where it sits (TLS serve path ONLY)
//! These knobs live BELOW the application layers — they configure the hyper connection serving the
//! request. They are applied **only on the in-process TLS serve path**: the h2/header knobs via
//! `axum-server`'s `http_builder()` (the `hyper_util::server::conn::auto::Builder` it serves with),
//! and the connection cap + handshake timeout + the per-connection **idle-keepalive read timeout**
//! (IO-layer [`IdleTimeoutStream`]) + the **max-requests-per-connection** cap (service-layer
//! [`MaxRequestsService`]) via the [`ConnectionLimitAcceptor`] wrapping `axum-server`'s acceptor — the
//! last two are NOT hyper builder knobs (hyper 1.x exposes neither a per-connection idle timeout nor a
//! request-count cap), so they are owned here at the per-connection accept seam. The **plain-HTTP path
//! is intentionally NOT hardened here** — `axum::serve`
//! exposes neither the underlying hyper builder nor an acceptor seam, so these knobs cannot be wired
//! onto it; an operator who needs the transport caps must terminate TLS in-process (or front the
//! plain path with a reverse proxy that caps connections + terminates HTTP/2). The startup log states
//! which posture is active per serve mode. On the plain path the application-layer defences (admission
//! control, per-IP rate limit, request timeout) still apply.
//!
//! These knobs are purely transport-level: they change WHEN/WHETHER a connection is served, never the
//! LDP/auth/WAC semantics of a request that IS served — so conformance is unaffected (the caps are
//! deliberately lenient enough never to trip the harness's own concurrency; see the defaults).

use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use axum_server::accept::Accept;
// `hyper::body` re-exports the `http_body` trait items — no new dependency (hyper is already direct).
use hyper::body::{Body, Frame, SizeHint};
use hyper_util::rt::TokioTimer;
use hyper_util::server::conn::auto::Builder;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::rate_limit::is_internal_ip;

/// The per-source live-connection map: a count of currently-open connections keyed by source IP. Shared
/// (behind `Arc<Mutex<_>>`) between the [`ConnectionLimiter`] and the [`IpConnGuard`]s that decrement it
/// on connection close. Aliased so the (otherwise clippy-`type_complexity`) shared type is named once.
type PerIpConnMap = HashMap<IpAddr, usize>;

// --- Env var names --------------------------------------------------------------------------------

/// Env var: the HTTP/2 `SETTINGS_MAX_CONCURRENT_STREAMS` advertised to clients (max simultaneously
/// open streams per connection). Bounds the work a single h2 connection can pin. Unset / invalid ⇒
/// [`DEFAULT_H2_MAX_CONCURRENT_STREAMS`]. `0` is rejected (a 0-stream connection is useless) ⇒ default.
pub const ENV_H2_MAX_CONCURRENT_STREAMS: &str = "SOLID_SERVER_H2_MAX_CONCURRENT_STREAMS";

/// Env var: the max number of pending-accept RST_STREAM streams before a `GOAWAY` (the
/// CVE-2023-44487 rapid-reset cap, hyper #2877). Unset ⇒ hyper's secure default (currently 20) is
/// kept — we do NOT lower it. A positive value overrides; `0` is rejected (would `GOAWAY` instantly)
/// ⇒ keep hyper's default.
pub const ENV_H2_MAX_PENDING_RESET_STREAMS: &str = "SOLID_SERVER_H2_MAX_PENDING_RESET_STREAMS";

/// Env var: the slowloris header-read timeout in seconds — a connection that has not sent a COMPLETE
/// set of request headers within this window is dropped. Unset / invalid ⇒
/// [`DEFAULT_HEADER_READ_TIMEOUT_SECS`]; `0` ⇒ DISABLED (no header timeout).
pub const ENV_HEADER_READ_TIMEOUT_SECS: &str = "SOLID_SERVER_HEADER_READ_TIMEOUT_SECS";

/// Env var: the maximum number of concurrently-open TCP connections accepted. A flood of half-open
/// (slowloris) connections beyond this is not accepted, so it cannot exhaust file descriptors /
/// memory. Unset / invalid / `0` ⇒ [`DEFAULT_MAX_CONNECTIONS`] (a `0` would refuse all traffic — never
/// silently brick the server on a typo).
pub const ENV_MAX_CONNECTIONS: &str = "SOLID_SERVER_MAX_CONNECTIONS";

/// Env var: the HTTP/2 keep-alive ping interval+ack-timeout in seconds. hyper sends a keep-alive PING
/// every interval and DROPS the connection if the ack does not arrive within the timeout — this
/// reclaims a connection whose peer has gone away (a dead/half-open h2 connection holding a permit),
/// NOT a merely-idle one (a live client that keeps acking pings is not closed by this). Combined with
/// the connection cap + the bounded handshake timeout, it bounds the dead-peer leak. Unset / invalid ⇒
/// [`DEFAULT_KEEP_ALIVE_TIMEOUT_SECS`]; `0` ⇒ DISABLED (no h2 keep-alive ping).
pub const ENV_KEEP_ALIVE_TIMEOUT_SECS: &str = "SOLID_SERVER_KEEP_ALIVE_TIMEOUT_SECS";

/// Env var: the accept/handshake timeout in seconds — the maximum time the TLS handshake (the inner
/// accept) may take while HOLDING a connection permit. A client that opens a TCP connection and STALLS
/// the TLS handshake (never completing it) would otherwise pin a permit until the underlying acceptor's
/// own handshake timeout; bounding it HERE releases the permit promptly so a slow-handshake flood
/// cannot exhaust the connection cap before the HTTP-layer `header_read_timeout` can apply. Unset /
/// invalid ⇒ [`DEFAULT_HANDSHAKE_TIMEOUT_SECS`]; `0` ⇒ DISABLED (rely on the acceptor's own bound).
pub const ENV_HANDSHAKE_TIMEOUT_SECS: &str = "SOLID_SERVER_HANDSHAKE_TIMEOUT_SECS";

/// Env var: the maximum number of concurrently-open connections from a SINGLE source IP. The global
/// [`ENV_MAX_CONNECTIONS`] cap alone lets ONE source hold ALL the slots (a single-IP slowloris flood);
/// this bounds any one IP well below the global ceiling. Unset / empty / non-numeric ⇒
/// [`DEFAULT_MAX_CONNECTIONS_PER_IP`] (ENABLED at the default); the sentinel `off` (case-insensitive)
/// or `0` ⇒ DISABLED (no per-source cap — rely on the global cap only). See [`parse_max_connections_per_ip`].
pub const ENV_MAX_CONNECTIONS_PER_IP: &str = "SOLID_SERVER_MAX_CONNECTIONS_PER_IP";

/// Env var: whether to EXEMPT internal source IPs (loopback + RFC-1918 private + link-local + IPv6 ULA
/// — see [`crate::rate_limit::is_internal_ip`]) from the PER-SOURCE connection cap. `0`/`false`/`no`/
/// `off` (case-insensitive) disables the exemption; anything else / absent ⇒ exemption ON (**the
/// default**). ON by default for the SAME footgun-guard reason as the rate limiter: behind a reverse
/// proxy / docker-bridge / k8s service WITHOUT a real client-IP source, EVERY client shares one
/// internal hop IP, so a per-IP CONNECTION cap keyed on that hop would throttle ALL clients together
/// (and it covers the conformance harness's `host.docker.internal` private-IP hop). A directly-exposed
/// public deployment gets the per-source cap; an internal hop is exempt (bounded by the global cap).
pub const ENV_CONN_EXEMPT_INTERNAL: &str = "SOLID_SERVER_CONN_EXEMPT_INTERNAL";

/// Env var: the per-connection **idle-keepalive timeout** in seconds — a connection with NO read OR
/// write progress for this long is dropped, reclaiming its connection permit. The missing-guard
/// companion to the existing bounds: `header_read_timeout` bounds reading a COMPLETE header set once
/// bytes START arriving; the h2 keep-alive PING reclaims a DEAD peer (one that stopped acking). NEITHER
/// bounds a peer that completes a request then holds the keep-alive connection open sending nothing
/// further. This idle bound closes it (IO layer, TLS serve path only — see [`IdleTimeoutStream`]).
/// **DEFAULT-OFF**: unset / empty / invalid / `0` ⇒ DISABLED. It is OPT-IN because the IO-layer bound
/// cannot exclude an UPGRADED long-lived streaming connection (a WebSocketChannel2023 receive socket)
/// that legitimately sits quiet — enable it (e.g. `=75`) only where no such connection exists on this
/// listener. `>0` ⇒ that idle window. See [`DEFAULT_IDLE_TIMEOUT_SECS`] for the full rationale.
pub const ENV_IDLE_TIMEOUT_SECS: &str = "SOLID_SERVER_IDLE_TIMEOUT_SECS";

/// Env var: the **max requests per connection** cap — after serving this many requests on a single
/// HTTP/1.1 keep-alive connection the server sets `Connection: close` on the response, so hyper closes
/// the connection after it and the client must reconnect. This bounds how long one connection can be
/// reused (defence-in-depth: it forces periodic re-handshake/re-LB-balance and caps the work a single
/// long-lived connection can pin without ever re-entering the accept-time connection cap). Unset /
/// invalid / `0` ⇒ DISABLED (unlimited reuse — the default, so it never trips the conformance harness).
/// NOTE (h2): under HTTP/2 multiplexing "requests per connection" is fuzzy (no clean `Connection: close`
/// per stream); this cap is enforced on the per-request response header, which is an HTTP/1.1 construct —
/// see [`MaxRequestsService`]. The accept-time connection cap + the h2 stream/reset caps bound h2.
pub const ENV_MAX_REQUESTS_PER_CONN: &str = "SOLID_SERVER_MAX_REQUESTS_PER_CONN";

// --- Defaults -------------------------------------------------------------------------------------

/// Default `max_concurrent_streams`. hyper's own default is 200; we set 256 explicitly (a small,
/// round, generous ceiling — far above any legitimate browser/client's per-connection multiplexing,
/// well above the conformance harness's needs, but a hard bound against a single connection pinning
/// unbounded work). Owning it as a constant makes it a documented, tested invariant.
pub const DEFAULT_H2_MAX_CONCURRENT_STREAMS: u32 = 256;

/// Default slowloris header-read timeout (seconds). Generous enough for a legitimate client on a slow
/// link to transmit a normal (even large-cookie) header set, short enough that a byte-trickle
/// connection is reclaimed promptly. hyper's own default is ~30s; we set 15s explicitly + own it.
pub const DEFAULT_HEADER_READ_TIMEOUT_SECS: u64 = 15;

/// Maximum number of HTTP/1 request header fields accepted by the transport parser. Requests above
/// this fail closed with `431 Request Header Fields Too Large` before the service sees a body.
///
/// This is set explicitly instead of relying on hyper's current default, so dependency
/// upgrades cannot silently remove or relax the header-count guard.
pub const MAX_REQUEST_HEADER_COUNT: usize = 100;

/// Maximum bytes hyper may buffer while parsing an HTTP/1 request head. This bounds the aggregate
/// request-line + header bytes retained for a connection; an incomplete head that exceeds the bound
/// is rejected before request/body dispatch.
///
/// Hyper requires this value to be at least 8 KiB. The chosen ceiling remains generous for
/// normal HTTP metadata while making the allocation bound explicit and dependency-version independent.
pub const MAX_REQUEST_HEADER_BYTES: usize = 64 * 1024;

/// Default concurrent-connection cap. Deliberately HIGH — a safety bound against a connection flood,
/// NOT a throughput throttle: it must never trip during normal use OR the conformance run (which uses
/// a handful of connections). An operator tunes it to their box's fd/memory budget. Sized to match
/// the spirit of [`crate::overload::DEFAULT_MAX_CONCURRENCY`] (10_000 in-flight requests) — connections
/// are cheaper than admitted requests, so an equal ceiling is conservative.
pub const DEFAULT_MAX_CONNECTIONS: usize = 10_000;

/// Default HTTP/2 keep-alive ping interval+ack-timeout (seconds). Long enough not to churn a healthy
/// reused connection, short enough to reclaim a DEAD-peer one (one whose host vanished without a FIN).
/// This detects a dead peer, NOT a live-but-idle client — see [`ENV_KEEP_ALIVE_TIMEOUT_SECS`].
pub const DEFAULT_KEEP_ALIVE_TIMEOUT_SECS: u64 = 60;

/// Default accept/handshake timeout (seconds). Generous enough for a legitimate TLS handshake on a
/// slow link, short enough that a stalled-handshake connection releases its permit promptly. Matches
/// the spirit of axum-server's own 10s handshake-timeout default, but is owned + tested here and
/// releases the CONNECTION PERMIT (not just the handshake) on expiry.
pub const DEFAULT_HANDSHAKE_TIMEOUT_SECS: u64 = 10;

/// Default per-source (per-IP) concurrent-connection cap. Deliberately GENEROUS — a safety bound
/// against a SINGLE source pinning the whole global pool, NOT a throughput throttle. `512` concurrent
/// connections from one IP is far above any normal client (a browser opens ~6 per origin) yet bounds a
/// single-IP half-open/slowloris flood to a small fraction of the global [`DEFAULT_MAX_CONNECTIONS`]
/// ceiling. Internal IPs are exempt by default ([`ENV_CONN_EXEMPT_INTERNAL`]), so a reverse-proxy /
/// docker / k8s / conformance hop is unaffected; a directly-exposed public deployment behind a large
/// NAT can raise it. An operator tunes it to their box's fd/memory budget vs. its client population.
pub const DEFAULT_MAX_CONNECTIONS_PER_IP: usize = 512;

/// Default per-connection idle-keepalive timeout (seconds). **`0` = DISABLED by default** — a
/// deliberate fail-safe (roborev High): the idle bound is enforced at the raw IO read/write layer, which
/// CANNOT distinguish an idle HTTP/1.1 keep-alive gap from an UPGRADED, long-lived streaming connection
/// (a WebSocketChannel2023 receive socket — see [`crate::notifications::ws`]) that is server→client only
/// and legitimately sits quiet (no inbound bytes, and no outbound until the first notification fires).
/// Closing such a connection after an idle window would tear down a LIVE subscription. So the guard is
/// OFF by default; an operator who has NO long-lived upgraded/streaming connections on this listener (or
/// fronts them elsewhere) opts IN via `SOLID_SERVER_IDLE_TIMEOUT_SECS` — e.g. 75s, the nginx
/// `keepalive_timeout` default. See [`IdleTimeoutStream`] for the full caveat. When enabled, the
/// deadline resets on read OR write progress (so an actively-pushing connection survives), but a
/// connection quiet in BOTH directions is still closed — which is exactly why the WS case needs OFF.
pub const DEFAULT_IDLE_TIMEOUT_SECS: u64 = 0;

/// Default max-requests-per-connection. `0` = DISABLED (unlimited keep-alive reuse) — the default,
/// because a bound here is a defence-in-depth knob (the connection cap + h2 stream caps already bound
/// the work a connection pins), and any finite value risks tripping the conformance harness / a
/// legitimate high-reuse client. An operator opts IN to connection rotation by setting a positive value.
pub const DEFAULT_MAX_REQUESTS_PER_CONN: u64 = 0;

// --- Resolved config ------------------------------------------------------------------------------

/// The resolved transport-hardening configuration. Built from the env via [`TransportConfig::from_env`]
/// (or directly in tests). Every field is a plain value so the struct is trivially testable; the
/// effect is applied to a hyper builder by [`TransportConfig::apply_to_builder`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportConfig {
    /// HTTP/2 `SETTINGS_MAX_CONCURRENT_STREAMS` (always set — owned ceiling).
    pub h2_max_concurrent_streams: u32,
    /// Override for the rapid-reset pending-accept cap. `None` ⇒ keep hyper's secure default (20).
    pub h2_max_pending_reset_streams: Option<usize>,
    /// Slowloris header-read timeout. `None` ⇒ disabled (no header timeout).
    pub header_read_timeout: Option<Duration>,
    /// Concurrent-connection cap (always set — a `0` env value falls back to the default, never 0).
    pub max_connections: usize,
    /// HTTP/2 keep-alive ping interval+ack-timeout — reclaims a DEAD-peer connection (not a live-idle
    /// one). `None` ⇒ disabled (no h2 keep-alive ping).
    pub keep_alive_timeout: Option<Duration>,
    /// Accept/handshake timeout: the max time the inner accept (TLS handshake) may take while holding a
    /// connection permit, after which the permit is released. `None` ⇒ disabled (rely on the underlying
    /// acceptor's own handshake bound).
    pub handshake_timeout: Option<Duration>,
    /// Per-source (per-IP) concurrent-connection cap. `Some(n)` ⇒ at most `n` simultaneous connections
    /// per source IP (over ⇒ the connection is refused at the acceptor); `None` ⇒ the per-source cap is
    /// disabled (rely on the global [`max_connections`](Self::max_connections) cap only).
    pub max_connections_per_ip: Option<usize>,
    /// Whether internal source IPs (loopback / private / link-local / ULA) are EXEMPT from the
    /// per-source connection cap (the default-on footgun guard for a proxy/docker/k8s/conformance hop —
    /// see [`ENV_CONN_EXEMPT_INTERNAL`]).
    pub conn_exempt_internal: bool,
    /// Per-connection idle-keepalive timeout: a kept-alive connection with no read activity for this
    /// long is dropped (reclaiming its permit). `None` ⇒ disabled. Enforced at the IO layer (see
    /// [`IdleTimeoutStream`]).
    pub idle_timeout: Option<Duration>,
    /// Max requests served on one connection before `Connection: close` is set (HTTP/1.1 reuse cap).
    /// `None` ⇒ disabled (unlimited reuse). Enforced at the per-connection service layer (see
    /// [`MaxRequestsService`]).
    pub max_requests_per_conn: Option<u64>,
}

impl TransportConfig {
    /// Resolve the transport config from the environment, applying each field's documented fallback.
    pub fn from_env() -> Self {
        Self {
            h2_max_concurrent_streams: parse_h2_max_concurrent_streams(
                std::env::var(ENV_H2_MAX_CONCURRENT_STREAMS).ok(),
            ),
            h2_max_pending_reset_streams: parse_h2_max_pending_reset_streams(
                std::env::var(ENV_H2_MAX_PENDING_RESET_STREAMS).ok(),
            ),
            header_read_timeout: parse_optional_secs(
                std::env::var(ENV_HEADER_READ_TIMEOUT_SECS).ok(),
                DEFAULT_HEADER_READ_TIMEOUT_SECS,
            ),
            max_connections: parse_max_connections(std::env::var(ENV_MAX_CONNECTIONS).ok()),
            keep_alive_timeout: parse_optional_secs(
                std::env::var(ENV_KEEP_ALIVE_TIMEOUT_SECS).ok(),
                DEFAULT_KEEP_ALIVE_TIMEOUT_SECS,
            ),
            handshake_timeout: parse_optional_secs(
                std::env::var(ENV_HANDSHAKE_TIMEOUT_SECS).ok(),
                DEFAULT_HANDSHAKE_TIMEOUT_SECS,
            ),
            max_connections_per_ip: parse_max_connections_per_ip(
                std::env::var(ENV_MAX_CONNECTIONS_PER_IP).ok(),
            ),
            conn_exempt_internal: parse_conn_exempt_internal(
                std::env::var(ENV_CONN_EXEMPT_INTERNAL).ok(),
            ),
            // Idle-keepalive is DEFAULT-OFF (opt-in): absent / empty / invalid / `0` ⇒ None (disabled).
            // Unlike the other timeouts, an absent value is DISABLED (not a default-enabled value) — see
            // `parse_idle_timeout` + `DEFAULT_IDLE_TIMEOUT_SECS` for why (the upgraded-WS-connection
            // fail-safe, roborev High).
            idle_timeout: parse_idle_timeout(std::env::var(ENV_IDLE_TIMEOUT_SECS).ok()),
            max_requests_per_conn: parse_max_requests_per_conn(
                std::env::var(ENV_MAX_REQUESTS_PER_CONN).ok(),
            ),
        }
    }

    /// Apply the HTTP/1.1 + HTTP/2 transport knobs to a hyper `auto::Builder` (the builder BOTH serve
    /// paths use). This sets:
    /// - h2 `max_concurrent_streams` (the owned ceiling),
    /// - h2 `max_pending_accept_reset_streams` when overridden (else hyper's secure default 20 stands),
    /// - the h2 keep-alive PING (interval + ack-timeout) — reclaims a DEAD-peer h2 connection (one
    ///   whose host vanished without a FIN); it does NOT close a live-but-idle client (that holds a
    ///   permit until it disconnects — bounded instead by the connection cap),
    /// - the h1 header-read (slowloris) timeout — which REQUIRES a [`TokioTimer`], so we install one
    ///   on both the h1 and h2 sub-builders whenever a timeout-bearing knob is active (hyper PANICS if
    ///   `header_read_timeout` is set without a timer),
    /// - the h1 request-header count and aggregate buffered-byte ceilings.
    ///
    /// It does NOT touch ALPN / TLS — those stay exactly as the caller built them (the TLS path's
    /// rustls config owns ALPN; see [`crate::tls`]).
    pub fn apply_to_builder<E>(&self, builder: &mut Builder<E>) {
        // A timer is REQUIRED before `header_read_timeout` (hyper panics otherwise) and is needed for
        // the keep-alive timeouts to fire. Install it on BOTH sub-builders unconditionally — it is
        // harmless when no timeout is set, and it keeps the "set timer before timeout" invariant local
        // and unmissable.
        builder.http1().timer(TokioTimer::new());
        builder.http2().timer(TokioTimer::new());

        // HTTP/2: the rapid-reset + concurrency caps.
        builder
            .http2()
            .max_concurrent_streams(self.h2_max_concurrent_streams);
        if let Some(max_pending) = self.h2_max_pending_reset_streams {
            // Override hyper's default-20 rapid-reset cap (only when explicitly configured higher/lower
            // by an operator who knows their workload; unset keeps the secure default).
            builder
                .http2()
                .max_pending_accept_reset_streams(max_pending);
        }

        // Slowloris header-read timeout (HTTP/1.1). A connection that has not sent a complete header
        // set within the window is dropped. We ALWAYS call the setter with the owned value so this
        // crate's value wins over hyper's default: `Some(d)` enforces the timeout, `None` disables it
        // (an explicit `0`/disable env value). The timer installed above makes it take effect.
        builder
            .http1()
            .max_headers(MAX_REQUEST_HEADER_COUNT)
            .max_buf_size(MAX_REQUEST_HEADER_BYTES)
            .header_read_timeout(self.header_read_timeout);

        // h2 keep-alive PING (interval + ack-timeout): hyper sends a PING every `ka` and DROPS the
        // connection if the ack does not arrive within `ka` — so a DEAD-peer h2 connection (host gone
        // without a FIN) is reclaimed, freeing its permit. NOTE this detects a dead peer, NOT a
        // live-but-idle client (one that keeps acking pings is NOT closed — it holds its permit until
        // it disconnects; the connection CAP is what bounds that). For h1, hyper reclaims an idle
        // keep-alive connection on the header-read timeout of the NEXT request, so the header-read
        // timeout above already bounds a dead/idle h1 keep-alive head.
        if let Some(ka) = self.keep_alive_timeout {
            builder.http2().keep_alive_interval(ka);
            builder.http2().keep_alive_timeout(ka);
        }
    }
}

// --- Concurrent-connection cap --------------------------------------------------------------------

/// A shared cap on the number of concurrently-served connections. A permit is acquired when a
/// connection is accepted and HELD for the connection's whole lifetime (released on drop), so a flood
/// of half-open (slowloris) connections beyond the cap cannot accumulate unbounded served connections
/// — exhausting file descriptors / memory. Cloning shares the same underlying semaphore.
#[derive(Clone)]
pub struct ConnectionLimiter {
    semaphore: Arc<Semaphore>,
    max_connections: usize,
    /// The per-source (per-IP) cap. `None` ⇒ the per-source cap is disabled (global cap only). Set via
    /// [`with_per_ip_cap`](Self::with_per_ip_cap).
    max_per_ip: Option<usize>,
    /// Whether internal source IPs are exempt from the per-source cap (default-on footgun guard — set
    /// with the per-IP cap).
    exempt_internal: bool,
    /// Live-connection counts per source IP (only populated when the per-source cap is enabled AND the
    /// peer IP is known + non-exempt). A single `Mutex` is fine: connection ACCEPTS are rare relative to
    /// requests, and each critical section is a tiny map lookup + integer step (no I/O, no `.await`).
    per_ip: Arc<Mutex<PerIpConnMap>>,
}

impl ConnectionLimiter {
    /// Build a limiter capping concurrently-served connections at `max_connections`, clamped to
    /// `[1, Semaphore::MAX_PERMITS]`:
    /// - the **lower** clamp (>=1) makes a 0 safe (a 0-permit pool would refuse all traffic; the env
    ///   parser already rejects 0, but a direct construction must be safe too);
    /// - the **upper** clamp ([`Semaphore::MAX_PERMITS`] = `usize::MAX >> 3`) avoids a startup PANIC —
    ///   `Semaphore::new` panics if its initial permit count exceeds that bound, and an operator could
    ///   set an absurdly large `SOLID_SERVER_MAX_CONNECTIONS`. Clamping fails SAFE (a still-enormous,
    ///   never-reached cap) rather than crashing the boot (roborev Low).
    ///
    /// The per-source (per-IP) cap is DISABLED by default (global cap only); enable it with
    /// [`with_per_ip_cap`](Self::with_per_ip_cap) — so every existing caller/test keeps global-only
    /// behaviour unless it opts in.
    pub fn new(max_connections: usize) -> Self {
        let permits = max_connections.clamp(1, Semaphore::MAX_PERMITS);
        Self {
            semaphore: Arc::new(Semaphore::new(permits)),
            max_connections: permits,
            max_per_ip: None,
            exempt_internal: true,
            per_ip: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Enable (or disable) the per-source (per-IP) connection cap. `max_per_ip = Some(n)` caps each
    /// source IP at `n` simultaneous connections; `None` disables the per-source cap (global cap only).
    /// `exempt_internal` exempts internal source IPs (loopback / private / link-local / ULA) — the
    /// default-on footgun guard for a proxy/docker/k8s/conformance hop. A `Some(0)` is clamped to
    /// `Some(1)` so a direct construction can never accidentally refuse ALL connections (the env parser
    /// already maps `0` to "disabled").
    pub fn with_per_ip_cap(mut self, max_per_ip: Option<usize>, exempt_internal: bool) -> Self {
        self.max_per_ip = max_per_ip.map(|n| n.max(1));
        self.exempt_internal = exempt_internal;
        self
    }

    /// The configured connection ceiling (after the `[1, MAX_PERMITS]` clamp).
    pub fn max_connections(&self) -> usize {
        self.max_connections
    }

    /// The configured per-source (per-IP) cap, if enabled.
    pub fn max_per_ip(&self) -> Option<usize> {
        self.max_per_ip
    }

    /// Try to reserve one per-source connection slot for `ip`. Returns:
    /// - `Some(guard)` — admitted (the guard DECREMENTS the source's live count on drop). This covers
    ///   BOTH the per-source-cap-disabled case and the exempt-internal-IP case (a no-op guard), so the
    ///   caller always gets a guard to move into the served connection when admitted;
    /// - `None` — the source is AT its per-source cap ⇒ the connection must be REFUSED.
    ///
    /// 🔒 This can only REFUSE a connection earlier; it never admits one the global cap would reject
    /// (the caller takes the global permit FIRST). Internal IPs are exempt when configured (a
    /// proxy/docker/k8s hop must not be capped as one source). A poisoned lock is recovered so one
    /// panicking accept can never wedge the per-IP map.
    fn try_acquire_ip(&self, ip: IpAddr) -> Option<IpConnGuard> {
        let Some(max) = self.max_per_ip else {
            return Some(IpConnGuard::disabled()); // per-source cap off ⇒ always admit (no tracking)
        };
        if self.exempt_internal && is_internal_ip(&ip) {
            return Some(IpConnGuard::disabled()); // internal hop ⇒ exempt (never capped as one source)
        }
        let mut map = match self.per_ip.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let count = map.get(&ip).copied().unwrap_or(0);
        if count >= max {
            // At the per-source cap — REFUSE. We looked up without inserting, so a capped-out (or
            // brand-new-but-somehow-capped) IP never leaves a stray entry in the map.
            return None;
        }
        // Admit: raise the source's live count (creating the entry on its first connection). `max ≥ 1`
        // is guaranteed (`with_per_ip_cap` clamps, the parser maps 0 → disabled), so a fresh IP (count
        // 0) always has room for its first connection.
        map.insert(ip, count + 1);
        Some(IpConnGuard::tracked(self.per_ip.clone(), ip))
    }

    /// Currently-available connection permits (cap minus in-flight). Exposed for tests/metrics.
    pub fn available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }

    /// Try to acquire a permit WITHOUT blocking. `Some(permit)` ⇒ admitted (hold for the connection
    /// lifetime); `None` ⇒ at capacity — the caller must REFUSE/drop the connection immediately rather
    /// than queue it. Fail-fast (not awaiting) is the load-bearing choice: a slowloris/connection flood
    /// over the cap is dropped + its socket reclaimed at once, so over-cap connections cannot pile up
    /// as parked tasks holding accepted sockets (roborev Medium). The `None` case also covers the
    /// (impossible — we never close it) closed-semaphore state.
    fn try_acquire(&self) -> Option<OwnedSemaphorePermit> {
        self.semaphore.clone().try_acquire_owned().ok()
    }

    /// Drive the per-source cap for tests (exposes the [`try_acquire_ip`](Self::try_acquire_ip) core):
    /// `Some(guard)` ⇒ admitted (hold to keep the source's live count raised; drop to release);
    /// `None` ⇒ the source is at its per-source cap (refused).
    #[doc(hidden)]
    pub fn try_acquire_ip_for_test(&self, ip: IpAddr) -> Option<IpConnGuard> {
        self.try_acquire_ip(ip)
    }

    /// The current live-connection count tracked for `ip` (0 if untracked). Test-only.
    #[doc(hidden)]
    pub fn per_ip_count_for_test(&self, ip: IpAddr) -> usize {
        match self.per_ip.lock() {
            Ok(g) => g.get(&ip).copied().unwrap_or(0),
            Err(p) => p.into_inner().get(&ip).copied().unwrap_or(0),
        }
    }

    /// Wrap an `axum-server` acceptor so each accepted connection holds a connection permit for its
    /// lifetime — the connection-cap for the TLS serve path, with NO handshake timeout (rely on the
    /// underlying acceptor's own bound) and no idle / max-requests guards. Prefer
    /// [`wrap_acceptor_with_guards`](Self::wrap_acceptor_with_guards) (or
    /// [`wrap_acceptor_with_handshake_timeout`](Self::wrap_acceptor_with_handshake_timeout) for just the
    /// handshake bound).
    pub fn wrap_acceptor<A>(&self, inner: A) -> ConnectionLimitAcceptor<A> {
        self.wrap_acceptor_with_handshake_timeout(inner, None)
    }

    /// As [`wrap_acceptor`](Self::wrap_acceptor), but bounding the inner accept (TLS handshake) by
    /// `handshake_timeout`: a connection that stalls the handshake longer than this has its permit
    /// RELEASED (the accept resolves to an error, so axum-server drops it), so a slow-handshake flood
    /// cannot pin the connection cap before the HTTP-layer header-read timeout can apply. `None` ⇒ no
    /// added bound (the underlying acceptor's own handshake timeout still applies). Idle / max-requests
    /// guards are OFF — see `wrap_acceptor_with_guards` for those.
    pub fn wrap_acceptor_with_handshake_timeout<A>(
        &self,
        inner: A,
        handshake_timeout: Option<Duration>,
    ) -> ConnectionLimitAcceptor<A> {
        self.wrap_acceptor_with_guards(inner, handshake_timeout, None, None)
    }

    /// The full guard wiring: the connection-cap permit + the optional `handshake_timeout` (as above),
    /// PLUS the two missing-guard companions:
    /// - `idle_timeout`: the per-connection idle-keepalive read timeout (a connection sending no bytes
    ///   for this long is dropped, reclaiming its permit) — enforced by wrapping the served IO in an
    ///   [`IdleTimeoutStream`]; `None` ⇒ no idle bound;
    /// - `max_requests_per_conn`: after this many requests on one connection the response carries
    ///   `Connection: close` (HTTP/1.1 reuse cap) — enforced by wrapping the per-connection service in a
    ///   [`MaxRequestsService`]; `None` ⇒ unlimited reuse.
    ///
    /// These are owned, env-tunable, TESTED transport invariants of this crate (see the module docs).
    pub fn wrap_acceptor_with_guards<A>(
        &self,
        inner: A,
        handshake_timeout: Option<Duration>,
        idle_timeout: Option<Duration>,
        max_requests_per_conn: Option<u64>,
    ) -> ConnectionLimitAcceptor<A> {
        ConnectionLimitAcceptor {
            inner,
            limiter: self.clone(),
            handshake_timeout,
            idle_timeout,
            max_requests_per_conn,
        }
    }
}

/// A RAII guard for ONE per-source connection slot. Held for a connection's lifetime alongside the
/// global [`OwnedSemaphorePermit`]; on drop it DECREMENTS the source IP's live-connection count in the
/// [`ConnectionLimiter`]'s per-IP map (removing the entry at zero so the map stays bounded). A
/// `disabled()` guard (per-source cap off, or an exempt/unknown IP) tracks nothing and its drop is a
/// no-op. Never leaks: whether the connection is admitted, refused after the handshake, or times out,
/// the guard drops with the accept future / served stream, releasing the source's slot.
pub struct IpConnGuard {
    /// `Some((map, ip))` ⇒ this guard holds a tracked slot for `ip`; `None` ⇒ a no-op (disabled/exempt).
    tracked: Option<(Arc<Mutex<PerIpConnMap>>, IpAddr)>,
}

impl IpConnGuard {
    /// A no-op guard (per-source cap disabled, or the IP is exempt/unknown) — its drop does nothing.
    fn disabled() -> Self {
        Self { tracked: None }
    }

    /// A guard tracking one live connection for `ip`; drop decrements the count in `map`.
    fn tracked(map: Arc<Mutex<PerIpConnMap>>, ip: IpAddr) -> Self {
        Self {
            tracked: Some((map, ip)),
        }
    }
}

impl Drop for IpConnGuard {
    fn drop(&mut self) {
        if let Some((map, ip)) = &self.tracked {
            let mut map = match map.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            if let Some(count) = map.get_mut(ip) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    map.remove(ip); // keep the map bounded — drop the entry when a source has no live conns
                }
            }
        }
    }
}

/// Extract the source (peer) IP from a connection's INPUT stream (the raw TCP stream the acceptor
/// receives BEFORE the TLS handshake), for the per-source connection cap. Implemented for the concrete
/// stream types the serve path + tests instantiate ([`tokio::net::TcpStream`] on the live TLS path,
/// `()` in the acceptor unit tests). A `None` result means "peer IP unknown" ⇒ the per-source cap
/// FAILS OPEN for that connection (it is admitted, still bounded by the global cap) — never deny-all on
/// a missing address, mirroring the rate limiter's fail-open-to-the-next-gate stance.
pub trait PeerAddr {
    /// The peer (source) IP of this connection, or `None` if it cannot be determined.
    fn peer_ip(&self) -> Option<IpAddr>;
}

impl PeerAddr for tokio::net::TcpStream {
    fn peer_ip(&self) -> Option<IpAddr> {
        self.peer_addr().ok().map(|a| a.ip())
    }
}

impl PeerAddr for () {
    fn peer_ip(&self) -> Option<IpAddr> {
        None
    }
}

/// An [`Accept`] wrapper that bounds concurrently-served connections via a [`ConnectionLimiter`] AND
/// wires the per-connection idle / max-requests guards. It acquires BOTH a global permit AND a
/// per-source (per-IP) slot (FAIL-FAST — without blocking) BEFORE running the inner accept (TLS
/// handshake); over either cap it REFUSES the connection with an error so `axum-server` drops it and
/// reclaims the socket at once, rather than queueing it. The inner accept is bounded by an optional
/// `handshake_timeout` so a stalled handshake cannot pin a permit. On admission it hands back:
/// - a [`PermittedStream`] (holding the global permit + the per-source [`IpConnGuard`] slot for the
///   connection lifetime) wrapping an [`IdleTimeoutStream`] (the idle-keepalive read timeout) — so no
///   more than `max_connections` served connections exist at once (and no more than
///   `max_connections_per_ip` from any one source), a flood over the cap (or a stalled handshake) is
///   shed promptly, AND an idle keep-alive connection is reclaimed; and
/// - a [`MaxRequestsService`] wrapping the per-connection service — so an HTTP/1.1 connection is closed
///   after the configured number of requests (`Connection: close`). Each accepted connection gets its
///   OWN request counter (it is created here, per `accept`), so the cap is per-connection.
#[derive(Clone)]
pub struct ConnectionLimitAcceptor<A> {
    inner: A,
    limiter: ConnectionLimiter,
    handshake_timeout: Option<Duration>,
    /// Per-connection idle-keepalive read timeout (`None` ⇒ no idle bound). Applied to the served IO.
    idle_timeout: Option<Duration>,
    /// Max requests per connection before `Connection: close` (`None` ⇒ unlimited). Applied to the
    /// per-connection service.
    max_requests_per_conn: Option<u64>,
}

impl<A, I, S> Accept<I, S> for ConnectionLimitAcceptor<A>
where
    A: Accept<I, S> + Clone + Send + Sync + 'static,
    A::Stream: AsyncRead + AsyncWrite + Unpin + Send,
    A::Service: Send,
    A::Future: Send,
    I: PeerAddr + Send + 'static,
    S: Send + 'static,
{
    // The served IO is `PermittedStream` wrapping an `IdleTimeoutStream` (idle timeout is a no-op
    // passthrough when `None`), so the permit is held for the connection lifetime AND an idle connection
    // is reclaimed. The served service is wrapped in `MaxRequestsService` (a no-op passthrough when the
    // cap is `None`), so an HTTP/1.1 connection is closed after the configured number of requests.
    type Stream = PermittedStream<IdleTimeoutStream<A::Stream>>;
    type Service = MaxRequestsService<A::Service>;
    type Future = Pin<Box<dyn Future<Output = io::Result<(Self::Stream, Self::Service)>> + Send>>;

    fn accept(&self, stream: I, service: S) -> Self::Future {
        // Acquire the GLOBAL connection permit FAIL-FAST, OUTSIDE the async block, so an over-cap
        // connection is refused HERE (the socket `stream` is dropped at once, reclaiming it) instead of
        // being queued as a parked task awaiting a permit. `axum-server` drops a connection whose
        // `accept` future resolves to `Err`, so an over-cap `Err` sheds the connection immediately.
        let permit = match self.limiter.try_acquire() {
            Some(p) => p,
            None => {
                // At the GLOBAL capacity — refuse. `stream`/`service` are dropped with this future,
                // releasing the socket. The connection cap doing its job (a 503-equivalent at the
                // transport layer): strictly less than the connection would otherwise get, never a bypass.
                return Box::pin(std::future::ready(Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "connection cap reached — refusing connection",
                ))));
            }
        };
        // PER-SOURCE cap: read the peer IP from the raw input stream (BEFORE it is moved into the inner
        // accept) and reserve a per-source slot. Over the per-source cap ⇒ refuse (dropping `permit`
        // here RELEASES the global slot too, so a refused per-source connection frees BOTH counters).
        // An UNKNOWN peer IP (`None`) FAILS OPEN — admitted, still bounded by the global cap.
        let ip_guard = match stream.peer_ip() {
            Some(ip) => match self.limiter.try_acquire_ip(ip) {
                Some(g) => g,
                None => {
                    return Box::pin(std::future::ready(Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "per-source connection cap reached — refusing connection",
                    ))));
                }
            },
            None => IpConnGuard::disabled(), // peer IP unknown ⇒ per-source cap fails open (global cap holds)
        };
        let inner = self.inner.clone();
        let handshake_timeout = self.handshake_timeout;
        let idle_timeout = self.idle_timeout;
        let max_requests = self.max_requests_per_conn;
        Box::pin(async move {
            // Run the inner accept (e.g. the TLS handshake), BOUNDED by `handshake_timeout` when set —
            // a stalled handshake must not pin the permit. On timeout the inner accept future is
            // dropped (cancelling the handshake) and we return an error; `permit` AND `ip_guard` are
            // dropped with this future, RELEASING both the global slot and the per-source slot at once.
            // On a SUCCESSFUL accept both are moved into the returned stream, so they stay held for the
            // connection lifetime and are released together when it closes.
            let accept_fut = inner.accept(stream, service);
            let (io_stream, svc) = match handshake_timeout {
                Some(to) => match tokio::time::timeout(to, accept_fut).await {
                    Ok(result) => result?,
                    Err(_elapsed) => {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "TLS handshake exceeded the accept timeout — refusing connection",
                        ));
                    }
                },
                None => accept_fut.await?,
            };
            // The shared per-connection in-flight-exchange count that couples the idle guard to the
            // request lifecycle (see `InFlightGuard`). Created ONLY when the idle guard is enabled — so
            // when it is off (the default) neither the IO nor the service touches an atomic or wraps a
            // body, keeping the default hot path unaffected. A FRESH `Arc` per accepted connection, one
            // clone to the IO (reader) and one to the service (writer), so the count is per-connection.
            let in_flight = idle_timeout.map(|_| Arc::new(AtomicUsize::new(0)));
            // Wrap the served IO with the idle-keepalive timeout (no-op when `None`), then hold BOTH the
            // global permit AND the per-source `ip_guard` for the connection lifetime; and wrap the
            // per-connection service with the max-requests cap (no-op when `None`). The per-connection
            // request counter lives INSIDE `MaxRequestsService` (a fresh counter per accepted connection
            // — see its docs), so the cap is genuinely per-connection. `IdleTimeoutStream` forwards the
            // PoP peer-cert / TLS-exporter reads to its inner TLS stream, so the mTLS (Tier-1b) +
            // DPoP-SK (Tier-2) paths still read the certificate/exporter through the idle wrapper.
            let idle_io = IdleTimeoutStream::new(io_stream, idle_timeout, in_flight.clone());
            let capped_svc = MaxRequestsService::new(svc, max_requests, in_flight);
            Ok((PermittedStream::new(idle_io, permit, ip_guard), capped_svc))
        })
    }
}

/// An IO stream that holds an [`OwnedSemaphorePermit`] for its lifetime, delegating all read/write to
/// the inner stream. Dropping it releases the connection permit (back to the [`ConnectionLimiter`]).
/// Requires `Io: Unpin` (every stream we serve — `TcpStream`, rustls `TlsStream<TcpStream>` — is
/// `Unpin`), so the AsyncRead/AsyncWrite impls can project to the inner stream without pin machinery.
pub struct PermittedStream<Io> {
    inner: Io,
    // Held for the connection lifetime; released on drop. Never read — its Drop is the whole point.
    _permit: OwnedSemaphorePermit,
    // The per-source slot for this connection; its Drop decrements the source IP's live-connection
    // count (a no-op guard when the per-source cap is off / the IP is exempt / unknown). Held for the
    // connection lifetime alongside the global permit.
    _ip_guard: IpConnGuard,
}

impl<Io> PermittedStream<Io> {
    fn new(inner: Io, permit: OwnedSemaphorePermit, ip_guard: IpConnGuard) -> Self {
        Self {
            inner,
            _permit: permit,
            _ip_guard: ip_guard,
        }
    }
}

/// Forward the peer-certificate read through the connection-cap wrapper so the PoP Tier-1b acceptor
/// ([`crate::pop::conn::ConnPopAcceptor`]) can read the client cert from a `PermittedStream<TlsStream>`
/// exactly as it would from the bare TLS stream. Purely a delegation — the permit/per-IP guards do not
/// affect the certificate.
impl<Io: crate::pop::conn::PeerCertDer> crate::pop::conn::PeerCertDer for PermittedStream<Io> {
    fn peer_cert_der(&self) -> Option<Vec<u8>> {
        self.inner.peer_cert_der()
    }
}

/// Forward the DPoP-SK TLS-exporter read (PoP Tier 2) through the connection-cap wrapper, exactly
/// like the peer-certificate delegation above — purely a delegation to the inner TLS stream.
impl<Io: crate::pop::conn::TlsExporter> crate::pop::conn::TlsExporter for PermittedStream<Io> {
    fn export_ekm_32(&self, label: &[u8]) -> Option<[u8; 32]> {
        self.inner.export_ekm_32(label)
    }
}

/// Forward the PoP peer-certificate read through the idle-timeout wrapper. The served IO is nested
/// `PermittedStream<IdleTimeoutStream<TlsStream>>` (the idle-keepalive guard sits BETWEEN the permit
/// wrapper and the TLS stream), so — for the mTLS Tier-1b path ([`crate::pop::conn::ConnPopAcceptor`])
/// to read the client certificate off the served stream — `IdleTimeoutStream` must delegate the read to
/// its inner TLS stream, exactly like `PermittedStream` does. Purely a delegation; the idle timeout does
/// not affect the certificate.
impl<Io: crate::pop::conn::PeerCertDer> crate::pop::conn::PeerCertDer for IdleTimeoutStream<Io> {
    fn peer_cert_der(&self) -> Option<Vec<u8>> {
        self.inner.peer_cert_der()
    }
}

/// Forward the DPoP-SK TLS-exporter read (PoP Tier 2) through the idle-timeout wrapper — the companion
/// to the peer-cert delegation above, for the same nested-stream reason. Purely a delegation to the
/// inner TLS stream.
impl<Io: crate::pop::conn::TlsExporter> crate::pop::conn::TlsExporter for IdleTimeoutStream<Io> {
    fn export_ekm_32(&self, label: &[u8]) -> Option<[u8; 32]> {
        self.inner.export_ekm_32(label)
    }
}

impl<Io: AsyncRead + Unpin> AsyncRead for PermittedStream<Io> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<Io: AsyncWrite + Unpin> AsyncWrite for PermittedStream<Io> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }
}

// --- Idle-keepalive timeout (IO-layer read-inactivity bound) --------------------------------------

/// An IO stream that enforces a per-connection **idle-activity timeout**: if NO read OR write progress
/// is made for the configured window, the next `poll_read` resolves to an `io::Error` (kind `TimedOut`),
/// which hyper surfaces as a connection close — reclaiming the connection (and its [`ConnectionLimiter`]
/// permit, via the outer [`PermittedStream`]). When the timeout is `None` it is a transparent
/// pass-through (zero overhead — no timer is created).
///
/// ## Request-aware — the idle clock is PAUSED while an HTTP exchange is in flight
/// The bound is genuinely an *idle-BETWEEN-requests* bound: it fires ONLY when NO HTTP exchange is in
/// flight. Via a shared per-connection [`InFlightGuard`] count (raised by [`MaxRequestsService`] from the
/// moment a request is dispatched until its response body has fully drained — see `in_flight`), the idle
/// clock is PAUSED for the whole exchange. So a slow handler (a >window SPARQ/S3 backend call), a slow
/// PUT that trickles its body between chunks, or a streaming GET stalled on client backpressure is NEVER
/// severed by this bound — it can only close a connection that is quiet AND has no request/response in
/// flight. (This request-awareness is what the earlier "never kills a legitimately slow link" claim now
/// actually delivers.)
///
/// ## ⚠️ DEFAULT-OFF — the POST-UPGRADE streaming-connection caveat (roborev High)
/// The request-awareness above covers ordinary HTTP exchanges, but this bound lives at the RAW IO layer,
/// BELOW hyper, so once a connection has UPGRADED to a long-lived bidirectional stream it can no longer
/// see an application-level "exchange" at all — the upgrade's originating HTTP request has completed, so
/// the in-flight count is back to 0. A **WebSocketChannel2023 receive socket**
/// ([`crate::notifications::ws`]) is **server→client only** and legitimately sits quiet — no inbound
/// bytes, and no outbound until the first notification fires — for as long as the client subscribes.
/// An idle bound that closed it would tear down a LIVE subscription. So this guard is **disabled by
/// default** ([`DEFAULT_IDLE_TIMEOUT_SECS`] = 0) and is for operators who terminate no long-lived
/// upgraded/streaming connections on this listener (or front them elsewhere). The deadline also resets on
/// read OR write activity (so an actively-pushing connection survives even when the guard is on), but a
/// post-upgrade connection quiet in BOTH directions is still closed — which is exactly why the WS case
/// requires OFF.
///
/// ## The gap it closes (when an operator DOES enable it)
/// hyper exposes NO native idle-connection timeout (verified against hyper 1.x). The existing bounds do
/// NOT cover an idle keep-alive connection:
/// - `http1::Builder::header_read_timeout` bounds reading a COMPLETE header set once bytes START
///   arriving — it does not fire on a connection that has sent a full request, got its response, and now
///   holds the keep-alive connection open sending nothing;
/// - the h2 keep-alive PING reclaims a DEAD peer (one that stops acking), not a live peer holding an
///   idle connection.
///
/// ## The timer is reset on READ OR WRITE ACTIVITY (any progress) AND paused while a request is in flight
/// The `Sleep` deadline is reset whenever a `poll_read` OR a `poll_write`/`poll_write_vectored` makes
/// progress (returns `Ready`), AND held fresh (never fired) while an HTTP exchange is in flight (the
/// request-aware pause above). It fires ONLY after a full window with NO progress in EITHER direction AND
/// NO exchange in flight. So a slow-but-progressing reader/writer resets the deadline on each chunk, and a
/// stalled-but-in-flight request/response is paused, so this never kills a legitimately slow link; the
/// slowloris HEADER trickle is bounded separately by `header_read_timeout` (a complete-head deadline that
/// a per-byte reset cannot defeat).
///
/// ## HTTP/2 interaction
/// Under HTTP/2 the server's keep-alive PING (when configured) elicits a client PONG — a READ — and the
/// PING itself is a WRITE, so an h2 connection with the PING on is never seen as idle by this bound; the
/// PING/PONG liveness probe is the h2 dead-peer reclaim. So when both are enabled, set the idle window
/// LARGER than the PING interval so they do not race.
///
/// Requires `Io: Unpin` (every stream we serve is `Unpin`); the boxed `Sleep` keeps the wrapper `Unpin`.
pub struct IdleTimeoutStream<Io> {
    inner: Io,
    /// `None` ⇒ no idle bound (transparent pass-through). `Some((dur, sleep))` ⇒ enforce `dur` of
    /// read+write inactivity; `sleep` is the live deadline, reset to `now + dur` on each read OR write
    /// that progresses.
    idle: Option<(Duration, Pin<Box<tokio::time::Sleep>>)>,
    /// Shared per-connection count of HTTP exchanges IN FLIGHT (see [`InFlightGuard`]). While it is `> 0`
    /// the connection is NOT idle — a request is being handled and/or its response body is still
    /// streaming — so the idle clock is PAUSED (the deadline is held fresh and the timeout never fires).
    /// The idle timeout therefore fires ONLY when the connection is genuinely idle BETWEEN exchanges.
    /// `None` ⇒ no tracking (the idle guard is off), so the default hot path is untouched.
    in_flight: Option<Arc<AtomicUsize>>,
    /// Whether the LAST `poll_read` observed an exchange in flight. Used to detect the in-flight→idle
    /// TRANSITION: while an exchange is in flight the deadline is only refreshed when `poll_read` is
    /// actually called, so a long exchange that makes no read progress for more than a window leaves the
    /// deadline STALE. On the first idle poll after such an exchange we must grant a FULL FRESH window
    /// (reset the deadline once) rather than fire immediately on that stale deadline — the connection
    /// becomes idle at the moment the exchange ENDS, not when it started. Only meaningful when
    /// [`in_flight`](Self::in_flight) is `Some`; starts `false`.
    prev_in_flight: bool,
}

impl<Io> IdleTimeoutStream<Io> {
    /// Reset the idle deadline to `now + dur` — called on any read OR write progress so an actively
    /// transferring connection is never closed by the idle bound. A no-op when the guard is disabled.
    fn touch(idle: &mut Option<(Duration, Pin<Box<tokio::time::Sleep>>)>) {
        if let Some((dur, sleep)) = idle.as_mut() {
            sleep.as_mut().reset(tokio::time::Instant::now() + *dur);
        }
    }
}

impl<Io> IdleTimeoutStream<Io> {
    /// Wrap `inner` enforcing an idle-read timeout of `idle_timeout` (or a transparent pass-through when
    /// `None`). The deadline starts at `now + idle_timeout` (so a connection that sends nothing AT ALL
    /// after the handshake is still bounded). `in_flight` is the shared per-connection in-flight-exchange
    /// count (see the field docs); pass `None` to disable request-aware pausing.
    pub fn new(
        inner: Io,
        idle_timeout: Option<Duration>,
        in_flight: Option<Arc<AtomicUsize>>,
    ) -> Self {
        let idle = idle_timeout.map(|dur| (dur, Box::pin(tokio::time::sleep(dur))));
        Self {
            inner,
            idle,
            in_flight,
            prev_in_flight: false,
        }
    }
}

impl<Io: AsyncRead + Unpin> AsyncRead for IdleTimeoutStream<Io> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = &mut *self;
        // Poll the inner read first. On any progress (Ready), reset the idle deadline and return it.
        match Pin::new(&mut this.inner).poll_read(cx, buf) {
            Poll::Ready(result) => {
                // Read progressed (bytes or EOF) ⇒ the connection is active ⇒ push the deadline out.
                Self::touch(&mut this.idle);
                Poll::Ready(result)
            }
            Poll::Pending => {
                // Inner read is pending (no bytes available). Two sub-cases:
                //
                //  (a) an HTTP exchange is IN FLIGHT (a request dispatched to the service, its response
                //      not yet fully drained — see `in_flight`): the connection is NOT idle even though no
                //      bytes are moving right now (a slow handler, a slow-uploading PUT between chunks, or
                //      a streaming GET stalled on client backpressure). PAUSE the idle clock — reset the
                //      deadline to a fresh window and stay Pending. This is what keeps the idle bound from
                //      severing a legitimately slow request/response; the timeout fires only BETWEEN
                //      exchanges. (When the exchange ends the count drops to 0 and the next poll_read —
                //      hyper reading the following keep-alive request — arms a full fresh idle window via
                //      the transition-reset below.)
                //
                //  (b) genuinely idle (no exchange in flight): if the deadline has elapsed, the connection
                //      has made NO progress for the whole window ⇒ close it with a TimedOut error. (A read
                //      or write since the last reset would have pushed the deadline out.) BUT the very
                //      first idle poll AFTER an in-flight exchange must not fire on a STALE deadline: while
                //      in flight the deadline is only refreshed when poll_read is actually called, so a
                //      long exchange with no read progress for >window leaves it in the past. On the
                //      in-flight→idle TRANSITION we therefore reset the deadline ONCE (a full fresh window
                //      counting from now — the connection becomes idle at exchange-END), then evaluate.
                let exchange_in_flight = this
                    .in_flight
                    .as_ref()
                    .is_some_and(|count| count.load(Ordering::Relaxed) > 0);
                if let Some((dur, sleep)) = this.idle.as_mut() {
                    if exchange_in_flight {
                        // (a) Hold the deadline fresh; do NOT poll/fire while the exchange runs.
                        sleep.as_mut().reset(tokio::time::Instant::now() + *dur);
                        this.prev_in_flight = true;
                    } else {
                        if this.prev_in_flight {
                            // (a→b) The exchange JUST ended: grant a FULL FRESH idle window from now, so a
                            // stale in-flight deadline can never fire immediately on this first idle poll.
                            sleep.as_mut().reset(tokio::time::Instant::now() + *dur);
                            this.prev_in_flight = false;
                        }
                        // (b) Idle past the window ⇒ close.
                        if sleep.as_mut().poll(cx).is_ready() {
                            return Poll::Ready(Err(io::Error::new(
                                io::ErrorKind::TimedOut,
                                "connection idle between requests past the idle timeout — closing",
                            )));
                        }
                    }
                }
                Poll::Pending
            }
        }
    }
}

impl<Io: AsyncWrite + Unpin> AsyncWrite for IdleTimeoutStream<Io> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = &mut *self;
        let r = Pin::new(&mut this.inner).poll_write(cx, buf);
        // A write that progressed is connection ACTIVITY (e.g. a server→client WS push / response body)
        // ⇒ reset the idle deadline so an actively-pushing connection is never closed by the idle bound.
        if r.is_ready() {
            Self::touch(&mut this.idle);
        }
        r
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = &mut *self;
        let r = Pin::new(&mut this.inner).poll_write_vectored(cx, bufs);
        if r.is_ready() {
            Self::touch(&mut this.idle); // vectored write progress is connection activity too
        }
        r
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }
}

// --- In-flight-exchange tracking (couples the idle guard to the request lifecycle) ----------------

/// RAII guard over the shared per-connection in-flight-exchange count. [`MaxRequestsService`] constructs
/// one when a request is dispatched (raising the count) and holds it across the request future AND the
/// response body ([`TrackedBody`]); it is released when the body finishes streaming or is dropped
/// (lowering the count). While the count is `> 0` the connection is NOT idle, which the
/// [`IdleTimeoutStream`] reads to pause its idle clock (so a slow/streaming exchange is never severed).
/// Cancellation-safe: dropping the future or the body always decrements. Only created when the idle
/// guard is enabled, so the default hot path never touches the atomic.
pub struct InFlightGuard {
    count: Arc<AtomicUsize>,
}

impl InFlightGuard {
    /// Raise the in-flight-exchange count; the matching decrement happens on drop.
    fn new(count: Arc<AtomicUsize>) -> Self {
        count.fetch_add(1, Ordering::Relaxed);
        Self { count }
    }
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.count.fetch_sub(1, Ordering::Relaxed);
    }
}

pin_project_lite::pin_project! {
    /// The response body produced by [`MaxRequestsService`], carrying an [`InFlightGuard`] so the
    /// in-flight-exchange count stays raised until the response body has fully streamed (or is dropped).
    /// This is what makes the idle guard safe for a slow/streaming RESPONSE: while this body is alive the
    /// connection is not idle. When in-flight tracking is off (the idle guard is disabled) the `Plain`
    /// variant is a zero-cost pass-through — no guard, no atomics, just a delegated `poll_frame` — so the
    /// default hot path is unaffected.
    #[project = TrackedBodyProj]
    pub enum TrackedBody<B> {
        /// Tracking OFF (idle guard disabled): a transparent pass-through — the default hot path.
        Plain { #[pin] inner: B },
        /// Tracking ON: holds the in-flight guard until the body ends (released eagerly on end-of-stream,
        /// and on drop as a fallback if the body is never fully polled).
        Tracked {
            #[pin]
            inner: B,
            guard: Option<InFlightGuard>,
        },
    }
}

impl<B: Body> Body for TrackedBody<B> {
    type Data = B::Data;
    type Error = B::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        match self.project() {
            TrackedBodyProj::Plain { inner } => inner.poll_frame(cx),
            TrackedBodyProj::Tracked { inner, guard } => {
                let polled = inner.poll_frame(cx);
                // End of stream ⇒ the response has fully drained ⇒ release the in-flight guard eagerly so
                // the connection is treated as idle at once (not only once hyper drops the body).
                if matches!(polled, Poll::Ready(None)) {
                    *guard = None;
                }
                polled
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        match self {
            TrackedBody::Plain { inner } => inner.is_end_stream(),
            TrackedBody::Tracked { inner, .. } => inner.is_end_stream(),
        }
    }

    fn size_hint(&self) -> SizeHint {
        match self {
            TrackedBody::Plain { inner } => inner.size_hint(),
            TrackedBody::Tracked { inner, .. } => inner.size_hint(),
        }
    }
}

// --- Max-requests-per-connection (per-connection service-layer reuse cap) --------------------------

/// A `tower::Service` wrapper that caps how many requests are served on ONE connection: after `cap`
/// requests it sets `Connection: close` on the response, so hyper closes the (HTTP/1.1) connection
/// after that response and the client must reconnect. When `cap` is `None` it is a transparent
/// pass-through (the request count is not even tracked).
///
/// ## Per-connection counter (the load-bearing scoping)
/// The acceptor creates ONE `MaxRequestsService` per accepted connection (in `accept`), so its counter
/// is genuinely PER-CONNECTION. hyper CLONES the service per request to serve concurrent/pipelined
/// requests on the connection, so the counter is an `Arc<AtomicU64>` SHARED across those clones (the
/// `Clone` impl shares the `Arc`) — counting all requests on this one connection — but NOT shared with
/// any OTHER connection's service (each `accept` makes a fresh `Arc`). A `fetch_add` on each call gives
/// the request's 1-based ordinal; at/after the cap the response gets `Connection: close`.
///
/// ## Protocol-upgrade exemption (the WebSocketChannel2023 receive socket)
/// The `Connection: close` stamp is SKIPPED for a COMPLETED protocol-UPGRADE response — a `101 Switching
/// Protocols` (see `is_upgrade_response`). The WS receive-socket handshake (`GET …/receive`) is served
/// through this same guarded acceptor, so with a small cap the upgrade IS the cap-th (or a later)
/// request; stamping `Connection: close` on its 101 would strip the required `Connection: Upgrade` token
/// and the client would FAIL the handshake (RFC 6455 §4.1). The exemption is keyed on the `101` STATUS,
/// not the mere presence of an `Upgrade` header — a non-101 response that merely advertises `Upgrade`
/// (e.g. a `426 Upgrade Required`) is an ordinary keep-alive response the cap still closes, so it cannot
/// bypass the reuse cap. The cap therefore applies to every ordinary keep-alive response; it simply
/// never clobbers a real (101) upgrade.
///
/// ## Why a header, not `graceful_shutdown` (the correctness choice)
/// Setting `Connection: close` lets the IN-FLIGHT response complete cleanly and THEN closes the
/// connection — the HTTP-correct way to end keep-alive reuse. Calling hyper's per-connection
/// `graceful_shutdown` instead would be coarser (it tears the whole connection state down). The header
/// is exactly what a reverse proxy's `MaxKeepAliveRequests` does.
///
/// ## HTTP/2 (documented limitation)
/// `Connection: close` is an HTTP/1.1 hop-by-hop header; under HTTP/2 there is no per-stream
/// `Connection: close` (the equivalent is a connection-level GOAWAY, which hyper's SERVER builder does
/// not expose a hook for). So this cap is meaningful for HTTP/1.1 keep-alive reuse; h2 connection-pinning
/// is bounded instead by the accept-time connection cap + the h2 stream/reset caps. Setting the header on
/// an h2 response is harmless (hyper drops connection-specific headers on h2) — it simply has no effect
/// there. This is acknowledged in the env-var docs ([`ENV_MAX_REQUESTS_PER_CONN`]).
#[derive(Clone)]
pub struct MaxRequestsService<S> {
    inner: S,
    /// `None` ⇒ unlimited (pass-through). `Some((cap, count))` ⇒ close after `cap` requests; `count` is
    /// the shared per-connection request counter (shared across this connection's service clones).
    cap: Option<(u64, Arc<std::sync::atomic::AtomicU64>)>,
    /// Shared per-connection in-flight-exchange count (see [`InFlightGuard`] / [`IdleTimeoutStream`]).
    /// `Some` ⇒ raise it for each dispatched request and hold the guard across the response body so the
    /// idle guard can pause its clock; `None` ⇒ no tracking (the idle guard is off — the default hot
    /// path). Shared with this connection's [`IdleTimeoutStream`] via a clone of the same `Arc`.
    in_flight: Option<Arc<AtomicUsize>>,
}

impl<S> MaxRequestsService<S> {
    /// Wrap `inner` with a per-connection request cap of `max_requests` (or a transparent pass-through
    /// when `None`). `in_flight` is the shared per-connection in-flight-exchange count (pass the same
    /// `Arc` given to this connection's [`IdleTimeoutStream`], or `None` to disable tracking). Creates a
    /// FRESH cap counter, so this must be called ONCE per connection (the acceptor does) for the cap to be
    /// per-connection.
    pub fn new(inner: S, max_requests: Option<u64>, in_flight: Option<Arc<AtomicUsize>>) -> Self {
        let cap = max_requests.map(|c| (c, Arc::new(std::sync::atomic::AtomicU64::new(0))));
        Self {
            inner,
            cap,
            in_flight,
        }
    }
}

impl<S, ReqBody, ResBody> tower::Service<http::Request<ReqBody>> for MaxRequestsService<S>
where
    S: tower::Service<http::Request<ReqBody>, Response = http::Response<ResBody>>,
{
    // The response body is wrapped in [`TrackedBody`] so an [`InFlightGuard`] spans the response body
    // (the `Plain` variant is a zero-cost pass-through when in-flight tracking is off — the default path).
    type Response = http::Response<TrackedBody<ResBody>>;
    type Error = S::Error;
    type Future = MaxRequestsFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: http::Request<ReqBody>) -> Self::Future {
        // Compute whether THIS request is at/over the per-connection cap BEFORE calling the inner
        // service. `fetch_add` returns the prior value, so `n+1` is this request's 1-based ordinal; we
        // close on the cap-th request and every one after (the connection should already be closing, but
        // being monotone is robust if a race lets one more in).
        let at_cap = match &self.cap {
            Some((cap, count)) => {
                let ordinal = count.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                ordinal >= *cap
            }
            None => false,
        };
        // Raise the in-flight-exchange count for the whole exchange (dispatch → response body drained).
        // `None` when the idle guard is off ⇒ no atomic, no body wrapping cost on the default hot path.
        let guard = self
            .in_flight
            .as_ref()
            .map(|c| InFlightGuard::new(c.clone()));
        MaxRequestsFuture {
            inner: self.inner.call(req),
            close: at_cap,
            guard,
        }
    }
}

pin_project_lite::pin_project! {
    /// The future for [`MaxRequestsService`]: awaits the inner response, then (a) — when this request hit
    /// the per-connection cap and it is not a protocol upgrade — sets `Connection: close` so hyper ends
    /// keep-alive reuse after it, and (b) moves the [`InFlightGuard`] into the response [`TrackedBody`] so
    /// the in-flight-exchange count stays raised until the body drains. The inner future is structurally
    /// pinned via `pin_project_lite` (a SAFE, declarative-macro projection — the crate is
    /// `#![forbid(unsafe_code)]`, so no hand-rolled `unsafe` pin). No allocation: the inner future is held
    /// inline, not boxed.
    pub struct MaxRequestsFuture<F> {
        #[pin]
        inner: F,
        // Whether to stamp `Connection: close` on the produced response (this request hit the cap).
        close: bool,
        // The in-flight guard, moved into the response body when the inner future resolves (or dropped
        // with this future if the request is cancelled). `None` when in-flight tracking is off.
        guard: Option<InFlightGuard>,
    }
}

/// Whether `resp` is a COMPLETED protocol-UPGRADE response whose `Connection` header MUST be preserved —
/// a `101 Switching Protocols` (the WebSocketChannel2023 receive-socket handshake). A genuine HTTP
/// protocol switch is ALWAYS signalled by the `101` status; the `Upgrade` / `Connection: upgrade` tokens
/// are the handshake tokens the 101 CARRIES (which we must not clobber), not the discriminator.
///
/// The status is checked, NOT the presence of an `Upgrade` header, DELIBERATELY: an `Upgrade` header (or
/// a `Connection: upgrade` token) on a NON-101 response is merely ADVISORY — e.g. a `426 Upgrade
/// Required`, or a `200` advertising `Upgrade: h2c` — and that response is an ordinary keep-alive
/// response the max-requests cap MUST still be able to close. Exempting every `Upgrade`-carrying response
/// (the earlier, over-broad predicate) would let such responses bypass the reuse cap indefinitely
/// (adversarial-review MED, PR #6). The cap MUST NOT stamp `Connection: close` on a 101, though: RFC 6455
/// §4.1 requires the client to FAIL the WebSocket handshake when `Connection` lacks the `upgrade` token,
/// so clobbering it would break every live notification receive socket that happens to be the cap-th (or
/// a later) request on its connection.
fn is_upgrade_response<B>(resp: &http::Response<B>) -> bool {
    resp.status() == http::StatusCode::SWITCHING_PROTOCOLS
}

impl<F, ResBody, E> Future for MaxRequestsFuture<F>
where
    F: Future<Output = Result<http::Response<ResBody>, E>>,
{
    type Output = Result<http::Response<TrackedBody<ResBody>>, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        match this.inner.poll(cx) {
            Poll::Ready(Ok(mut resp)) => {
                // End keep-alive reuse after this response — UNLESS it is a protocol upgrade (a 101 WS
                // receive-socket handshake), whose `Connection: Upgrade` we must never clobber (else the
                // client fails the WebSocket handshake — RFC 6455 §4.1). See [`is_upgrade_response`].
                if *this.close && !is_upgrade_response(&resp) {
                    // `HeaderValue::from_static` is infallible for this constant. (On h2 hyper ignores
                    // this connection-specific header — see the service docs; harmless.)
                    resp.headers_mut().insert(
                        http::header::CONNECTION,
                        http::HeaderValue::from_static("close"),
                    );
                }
                // Move the in-flight guard into the response body so the exchange stays counted until the
                // body has fully streamed (the request-aware idle pause). `None` ⇒ `Plain` pass-through.
                let guard = this.guard.take();
                let resp = resp.map(|body| match guard {
                    Some(guard) => TrackedBody::Tracked {
                        inner: body,
                        guard: Some(guard),
                    },
                    None => TrackedBody::Plain { inner: body },
                });
                Poll::Ready(Ok(resp))
            }
            // Preserve the inner future's Pending / Err outcome (the guard is held until we resolve, or
            // dropped with this future on cancellation — either way the in-flight count is correct).
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Pending => Poll::Pending,
        }
    }
}

// --- Pure parsers (testable cores) ----------------------------------------------------------------

/// Resolve `max_concurrent_streams` from the env value. absent / empty / non-numeric / `0` ⇒ the
/// default (a `0` would make every h2 connection useless). `>0` ⇒ that literal ceiling.
pub fn parse_h2_max_concurrent_streams(raw: Option<String>) -> u32 {
    match raw.as_deref().map(str::trim) {
        None | Some("") => DEFAULT_H2_MAX_CONCURRENT_STREAMS,
        Some(s) => match s.parse::<u32>() {
            Ok(0) | Err(_) => DEFAULT_H2_MAX_CONCURRENT_STREAMS,
            Ok(n) => n,
        },
    }
}

/// Resolve the rapid-reset pending-accept override. absent / empty / non-numeric / `0` ⇒ `None` (keep
/// hyper's secure default of 20 — NEVER lower it to 0, which would `GOAWAY` on the first reset). `>0`
/// ⇒ `Some(n)`. The override exists so an operator with an unusual but legitimate reset pattern can
/// RAISE the cap; the default path leaves the CVE-2023-44487 mitigation at hyper's value.
pub fn parse_h2_max_pending_reset_streams(raw: Option<String>) -> Option<usize> {
    match raw.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(s) => match s.parse::<usize>() {
            Ok(0) | Err(_) => None,
            Ok(n) => Some(n),
        },
    }
}

/// Resolve the max-connections cap. absent / empty / non-numeric / `0` ⇒ the default (a `0` would
/// refuse ALL traffic). `>0` ⇒ that literal cap.
pub fn parse_max_connections(raw: Option<String>) -> usize {
    match raw.as_deref().map(str::trim) {
        None | Some("") => DEFAULT_MAX_CONNECTIONS,
        Some(s) => match s.parse::<usize>() {
            Ok(0) | Err(_) => DEFAULT_MAX_CONNECTIONS,
            Ok(n) => n,
        },
    }
}

/// Resolve the per-source (per-IP) connection cap. absent / empty / non-numeric ⇒
/// `Some(DEFAULT_MAX_CONNECTIONS_PER_IP)` (ENABLED at the default); the sentinel `off`/`disabled`
/// (case-insensitive) or `0` ⇒ `None` (DISABLED — a `0` per-IP cap would refuse EVERY connection, so
/// it means "disable the per-source cap", never "allow none"); `>0` ⇒ `Some(n)`.
pub fn parse_max_connections_per_ip(raw: Option<String>) -> Option<usize> {
    match raw.as_deref().map(str::trim) {
        None | Some("") => Some(DEFAULT_MAX_CONNECTIONS_PER_IP),
        Some(s) if s.eq_ignore_ascii_case("off") || s.eq_ignore_ascii_case("disabled") => None,
        Some(s) => match s.parse::<usize>() {
            Ok(0) => None,                                  // 0 ⇒ disable (never "allow none")
            Ok(n) => Some(n),                               // explicit positive cap
            Err(_) => Some(DEFAULT_MAX_CONNECTIONS_PER_IP), // typo ⇒ safe default (never silently disable)
        },
    }
}

/// Resolve the max-requests-per-connection cap. absent / empty / non-numeric / `0` ⇒ `None` (DISABLED —
/// unlimited keep-alive reuse, the safe default that never trips the conformance harness). `>0` ⇒
/// `Some(n)` (close the connection after `n` requests). A `0` mapping to disabled (NOT a 0-request
/// connection) is deliberate: a typo must never brick keep-alive, and a 0-request cap would be useless.
pub fn parse_max_requests_per_conn(raw: Option<String>) -> Option<u64> {
    match raw.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(s) => match s.parse::<u64>() {
            Ok(0) | Err(_) => None,
            Ok(n) => Some(n),
        },
    }
}

/// Resolve whether internal source IPs are EXEMPT from the per-source connection cap. `0`/`false`/
/// `no`/`off` (case-insensitive) ⇒ NOT exempt; anything else / absent ⇒ exempt (the default). Matches
/// the rate limiter's internal-exemption grammar so the two agree.
pub fn parse_conn_exempt_internal(raw: Option<String>) -> bool {
    match raw.as_deref().map(str::trim) {
        Some(s) => {
            !(s.eq_ignore_ascii_case("0")
                || s.eq_ignore_ascii_case("false")
                || s.eq_ignore_ascii_case("no")
                || s.eq_ignore_ascii_case("off"))
        }
        None => true,
    }
}

/// Resolve the idle-keepalive timeout. This guard is **DEFAULT-OFF / OPT-IN** (unlike the header-read /
/// keep-alive timeouts), so the absent case is DISABLED, not a default-enabled value: absent / empty /
/// non-numeric / `0` ⇒ `None` (DISABLED); `>0` ⇒ `Some(n)` (enabled with that idle window). The
/// fail-safe default is load-bearing — an IO-layer idle bound cannot exclude an UPGRADED long-lived
/// streaming connection (a WebSocketChannel2023 receive socket), so it must be opted into only where no
/// such connection exists (see [`DEFAULT_IDLE_TIMEOUT_SECS`] / [`IdleTimeoutStream`], roborev High).
pub fn parse_idle_timeout(raw: Option<String>) -> Option<Duration> {
    match raw.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(s) => match s.parse::<u64>() {
            Ok(0) | Err(_) => None,
            Ok(n) => Some(Duration::from_secs(n)),
        },
    }
}

/// Resolve an optional-seconds timeout (header-read / keep-alive): absent / empty / non-numeric ⇒
/// `Some(default)` (ENABLED at the default); `0` ⇒ `None` (explicitly DISABLED); `>0` ⇒ `Some(n)`.
pub fn parse_optional_secs(raw: Option<String>, default_secs: u64) -> Option<Duration> {
    let secs = match raw.as_deref().map(str::trim) {
        None | Some("") => default_secs,
        Some(s) => match s.parse::<u64>() {
            Ok(0) => return None, // explicit disable
            Ok(n) => n,
            Err(_) => default_secs,
        },
    };
    Some(Duration::from_secs(secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn h2_max_concurrent_streams_default_on_absent_empty_invalid_or_zero() {
        // absent / empty / non-numeric / 0 ⇒ default (a 0-stream connection is useless).
        assert_eq!(
            parse_h2_max_concurrent_streams(None),
            DEFAULT_H2_MAX_CONCURRENT_STREAMS
        );
        assert_eq!(
            parse_h2_max_concurrent_streams(Some("".into())),
            DEFAULT_H2_MAX_CONCURRENT_STREAMS
        );
        assert_eq!(
            parse_h2_max_concurrent_streams(Some("  ".into())),
            DEFAULT_H2_MAX_CONCURRENT_STREAMS
        );
        assert_eq!(
            parse_h2_max_concurrent_streams(Some("abc".into())),
            DEFAULT_H2_MAX_CONCURRENT_STREAMS
        );
        assert_eq!(
            parse_h2_max_concurrent_streams(Some("0".into())),
            DEFAULT_H2_MAX_CONCURRENT_STREAMS
        );
    }

    #[test]
    fn h2_max_concurrent_streams_explicit_positive_is_honoured() {
        assert_eq!(parse_h2_max_concurrent_streams(Some("1".into())), 1);
        assert_eq!(parse_h2_max_concurrent_streams(Some("128".into())), 128);
        assert_eq!(parse_h2_max_concurrent_streams(Some("  512  ".into())), 512);
    }

    #[test]
    fn h2_pending_reset_keeps_hyper_default_unless_positive_override() {
        // absent / empty / non-numeric / 0 ⇒ None (KEEP hyper's secure default 20). The load-bearing
        // safety property: a typo / `0` can NEVER lower the rapid-reset cap below hyper's default.
        assert_eq!(parse_h2_max_pending_reset_streams(None), None);
        assert_eq!(parse_h2_max_pending_reset_streams(Some("".into())), None);
        assert_eq!(parse_h2_max_pending_reset_streams(Some("abc".into())), None);
        assert_eq!(parse_h2_max_pending_reset_streams(Some("0".into())), None);
        // A positive value overrides.
        assert_eq!(
            parse_h2_max_pending_reset_streams(Some("50".into())),
            Some(50)
        );
        assert_eq!(
            parse_h2_max_pending_reset_streams(Some("  100  ".into())),
            Some(100)
        );
    }

    #[test]
    fn max_connections_default_on_absent_empty_invalid_or_zero() {
        // absent / empty / non-numeric / 0 ⇒ default (a 0 would refuse all traffic).
        assert_eq!(parse_max_connections(None), DEFAULT_MAX_CONNECTIONS);
        assert_eq!(
            parse_max_connections(Some("".into())),
            DEFAULT_MAX_CONNECTIONS
        );
        assert_eq!(
            parse_max_connections(Some("abc".into())),
            DEFAULT_MAX_CONNECTIONS
        );
        assert_eq!(
            parse_max_connections(Some("0".into())),
            DEFAULT_MAX_CONNECTIONS
        );
    }

    #[test]
    fn max_connections_explicit_positive_is_honoured() {
        assert_eq!(parse_max_connections(Some("1".into())), 1);
        assert_eq!(parse_max_connections(Some("4096".into())), 4096);
        assert_eq!(parse_max_connections(Some("  64  ".into())), 64);
    }

    #[test]
    fn optional_secs_rules() {
        // default when absent/empty/invalid; disabled (None) on 0; honoured on >0.
        assert_eq!(
            parse_optional_secs(None, DEFAULT_HEADER_READ_TIMEOUT_SECS),
            Some(Duration::from_secs(DEFAULT_HEADER_READ_TIMEOUT_SECS))
        );
        assert_eq!(
            parse_optional_secs(Some("".into()), 15),
            Some(Duration::from_secs(15))
        );
        assert_eq!(
            parse_optional_secs(Some("garbage".into()), 15),
            Some(Duration::from_secs(15))
        );
        assert_eq!(parse_optional_secs(Some("0".into()), 15), None);
        assert_eq!(
            parse_optional_secs(Some("5".into()), 15),
            Some(Duration::from_secs(5))
        );
    }

    #[test]
    fn from_env_with_unset_vars_uses_documented_defaults() {
        // Snapshot + clear the vars so the test sees the absent-defaults regardless of the ambient env,
        // then restore. (Each var is process-global; this test owns them for its duration.)
        let keys = [
            ENV_H2_MAX_CONCURRENT_STREAMS,
            ENV_H2_MAX_PENDING_RESET_STREAMS,
            ENV_HEADER_READ_TIMEOUT_SECS,
            ENV_MAX_CONNECTIONS,
            ENV_KEEP_ALIVE_TIMEOUT_SECS,
            ENV_HANDSHAKE_TIMEOUT_SECS,
            ENV_MAX_CONNECTIONS_PER_IP,
            ENV_CONN_EXEMPT_INTERNAL,
        ];
        let saved: Vec<(&str, Option<String>)> =
            keys.iter().map(|k| (*k, std::env::var(k).ok())).collect();
        for k in keys {
            std::env::remove_var(k);
        }
        let cfg = TransportConfig::from_env();
        for (k, v) in saved {
            match v {
                Some(val) => std::env::set_var(k, val),
                None => std::env::remove_var(k),
            }
        }
        assert_eq!(
            cfg,
            TransportConfig {
                h2_max_concurrent_streams: DEFAULT_H2_MAX_CONCURRENT_STREAMS,
                h2_max_pending_reset_streams: None,
                header_read_timeout: Some(Duration::from_secs(DEFAULT_HEADER_READ_TIMEOUT_SECS)),
                max_connections: DEFAULT_MAX_CONNECTIONS,
                keep_alive_timeout: Some(Duration::from_secs(DEFAULT_KEEP_ALIVE_TIMEOUT_SECS)),
                handshake_timeout: Some(Duration::from_secs(DEFAULT_HANDSHAKE_TIMEOUT_SECS)),
                max_connections_per_ip: Some(DEFAULT_MAX_CONNECTIONS_PER_IP),
                conn_exempt_internal: true,
                // Idle-keepalive is DEFAULT-OFF (the upgraded-WS-connection fail-safe, roborev High):
                // an absent var ⇒ None (disabled), NOT a default-enabled value.
                idle_timeout: None,
                // Default-OFF: a finite reuse cap is opt-in (it could trip a high-reuse client / the
                // harness), so the documented default is `None` (unlimited keep-alive reuse).
                max_requests_per_conn: None,
            }
        );
    }

    #[test]
    fn apply_to_builder_does_not_panic_with_timeout_set() {
        // The load-bearing crash-guard: `header_read_timeout` PANICS in hyper if no `Timer` is set.
        // `apply_to_builder` installs a TokioTimer first, so applying a config WITH a header timeout
        // must not panic. (We build a real auto::Builder and apply — if the timer ordering were wrong
        // this would panic here.) Run inside a tokio context because TokioTimer expects a runtime.
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let cfg = TransportConfig {
                h2_max_concurrent_streams: 256,
                h2_max_pending_reset_streams: Some(20),
                header_read_timeout: Some(Duration::from_secs(15)),
                max_connections: 10_000,
                keep_alive_timeout: Some(Duration::from_secs(60)),
                handshake_timeout: Some(Duration::from_secs(10)),
                max_connections_per_ip: Some(512),
                conn_exempt_internal: true,
                idle_timeout: Some(Duration::from_secs(75)),
                max_requests_per_conn: Some(10_000),
            };
            let mut builder = Builder::new(hyper_util::rt::TokioExecutor::new());
            cfg.apply_to_builder(&mut builder);
        });
    }

    #[test]
    fn connection_limiter_clamps_to_at_least_one_permit() {
        // A 0 cap would refuse ALL connections; the type clamps to >=1 (the env parser already rejects
        // 0, but a direct construction must be safe too).
        let limiter = ConnectionLimiter::new(0);
        assert_eq!(limiter.max_connections(), 1);
        assert_eq!(limiter.available_permits(), 1);
    }

    #[test]
    fn connection_limiter_bounds_in_flight_then_recovers() {
        // The load-bearing property: at most `max_connections` permits are available at once; a held
        // permit reduces the pool; dropping it restores capacity; AND a 3rd acquire over the cap FAILS
        // FAST (None — refused, not queued). Tested deterministically without sockets. No async runtime
        // needed — `try_acquire` is synchronous (the fail-fast connection cap).
        let limiter = ConnectionLimiter::new(2);
        assert_eq!(limiter.available_permits(), 2);

        let p1 = limiter.try_acquire().expect("permit 1");
        assert_eq!(limiter.available_permits(), 1);
        let p2 = limiter.try_acquire().expect("permit 2");
        assert_eq!(
            limiter.available_permits(),
            0,
            "at capacity ⇒ no permits left"
        );

        // A third acquire over the cap must FAIL FAST (None) — the connection is REFUSED, not parked.
        assert!(
            limiter.try_acquire().is_none(),
            "a 3rd acquire must be REFUSED (None) while at capacity (the fail-fast connection cap)"
        );

        // Free one slot ⇒ the next acquire succeeds again.
        drop(p1);
        assert_eq!(limiter.available_permits(), 1);
        let p3 = limiter
            .try_acquire()
            .expect("a freed slot must admit the next connection");
        assert_eq!(limiter.available_permits(), 0);

        drop(p2);
        drop(p3);
        assert_eq!(
            limiter.available_permits(),
            2,
            "all slots released ⇒ full pool"
        );
    }

    #[test]
    fn acceptor_releases_permit_when_handshake_stalls() {
        // The handshake-timeout fix (roborev Medium): a stalled inner accept (TLS handshake) must NOT
        // pin a connection permit. With a mock acceptor whose `accept` NEVER resolves and a short
        // handshake timeout, the wrapped acceptor's future must resolve to Err WITHIN the timeout AND
        // RELEASE the permit (so the cap recovers). Deterministic, no sockets.
        use std::future::pending;

        // A mock `Accept` whose accept future never completes (models a stalled TLS handshake).
        #[derive(Clone)]
        struct StallingAcceptor;
        impl Accept<(), ()> for StallingAcceptor {
            // `TcpStream` is only NAMED as the associated Stream type (a connection is never produced —
            // the accept future never resolves), so no socket is opened. Using it avoids needing the
            // `io-util` feature a `DuplexStream` would require.
            type Stream = tokio::net::TcpStream;
            type Service = ();
            type Future =
                Pin<Box<dyn Future<Output = io::Result<(Self::Stream, Self::Service)>> + Send>>;
            fn accept(&self, _stream: (), _service: ()) -> Self::Future {
                // Never resolves — the handshake stalls forever.
                Box::pin(async { pending::<io::Result<(Self::Stream, Self::Service)>>().await })
            }
        }

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let limiter = ConnectionLimiter::new(1);
            assert_eq!(limiter.available_permits(), 1);
            let acceptor = limiter.wrap_acceptor_with_handshake_timeout(
                StallingAcceptor,
                Some(Duration::from_millis(100)),
            );

            // The accept future holds the permit while it awaits the (never-completing) handshake, then
            // must time out → Err. While in flight the permit is taken.
            let accept_fut = acceptor.accept((), ());
            let result = tokio::time::timeout(Duration::from_secs(2), accept_fut)
                .await
                .expect(
                    "the wrapped accept must itself time out (not hang) on a stalled handshake",
                );
            assert!(
                result.is_err(),
                "a stalled handshake must resolve the accept to Err (handshake timeout)"
            );
            // CRITICAL: the permit was released when the Err future was dropped, so the cap recovered.
            assert_eq!(
                limiter.available_permits(),
                1,
                "a stalled-handshake connection must RELEASE its permit on timeout (no permit leak)"
            );
        });
    }

    #[test]
    fn connection_limiter_clamps_huge_value_below_semaphore_max() {
        // A max_connections above tokio's Semaphore::MAX_PERMITS would PANIC in `Semaphore::new`; the
        // clamp fails SAFE to MAX_PERMITS instead of crashing the boot (roborev Low).
        let limiter = ConnectionLimiter::new(usize::MAX);
        assert_eq!(limiter.max_connections(), Semaphore::MAX_PERMITS);
        assert_eq!(limiter.available_permits(), Semaphore::MAX_PERMITS);
        // And one permit is acquirable (the pool is valid, not poisoned).
        assert!(limiter.try_acquire().is_some());
    }

    #[test]
    fn apply_to_builder_with_disabled_timeouts_does_not_panic() {
        // header_read_timeout = None (disabled) + keep_alive = None: still installs the timer, still
        // sets the h2 caps, and must not panic.
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let cfg = TransportConfig {
                h2_max_concurrent_streams: 256,
                h2_max_pending_reset_streams: None,
                header_read_timeout: None,
                max_connections: 10_000,
                keep_alive_timeout: None,
                handshake_timeout: None,
                max_connections_per_ip: None,
                conn_exempt_internal: true,
                idle_timeout: None,
                max_requests_per_conn: None,
            };
            let mut builder = Builder::new(hyper_util::rt::TokioExecutor::new());
            cfg.apply_to_builder(&mut builder);
        });
    }

    // --- per-source (per-IP) connection cap ---

    use std::net::Ipv4Addr;

    fn pub_ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    #[test]
    fn max_connections_per_ip_parse_rules() {
        // absent / empty / non-numeric ⇒ default (ENABLED); off/disabled/0 ⇒ None (DISABLED); >0 ⇒ that.
        assert_eq!(
            parse_max_connections_per_ip(None),
            Some(DEFAULT_MAX_CONNECTIONS_PER_IP)
        );
        assert_eq!(
            parse_max_connections_per_ip(Some("".into())),
            Some(DEFAULT_MAX_CONNECTIONS_PER_IP)
        );
        assert_eq!(
            parse_max_connections_per_ip(Some("abc".into())),
            Some(DEFAULT_MAX_CONNECTIONS_PER_IP),
            "a typo must NOT silently disable the per-source cap"
        );
        assert_eq!(parse_max_connections_per_ip(Some("off".into())), None);
        assert_eq!(parse_max_connections_per_ip(Some("OFF".into())), None);
        assert_eq!(parse_max_connections_per_ip(Some("disabled".into())), None);
        assert_eq!(
            parse_max_connections_per_ip(Some("0".into())),
            None,
            "0 means DISABLE the per-source cap (never 'allow none')"
        );
        assert_eq!(parse_max_connections_per_ip(Some("1".into())), Some(1));
        assert_eq!(
            parse_max_connections_per_ip(Some("  256 ".into())),
            Some(256)
        );
    }

    #[test]
    fn conn_exempt_internal_parse_rules() {
        // Default ON; falsey values (any casing) turn it off.
        assert!(parse_conn_exempt_internal(None));
        assert!(parse_conn_exempt_internal(Some("1".into())));
        assert!(parse_conn_exempt_internal(Some("true".into())));
        assert!(!parse_conn_exempt_internal(Some("0".into())));
        assert!(!parse_conn_exempt_internal(Some("false".into())));
        assert!(!parse_conn_exempt_internal(Some("no".into())));
        assert!(!parse_conn_exempt_internal(Some("OFF".into())));
        assert!(!parse_conn_exempt_internal(Some(" No ".into())));
    }

    #[test]
    fn per_ip_cap_bounds_a_single_source_and_recovers_on_drop() {
        // The core per-source invariant: at most `max_per_ip` live connections per source IP; a held
        // guard raises the count; dropping it restores capacity; an over-cap acquire is REFUSED (None).
        // exempt_internal=false so we can drive a public IP. Global pool is huge (isolate the per-IP cap).
        let limiter = ConnectionLimiter::new(10_000).with_per_ip_cap(Some(2), false);
        let ip = pub_ip(203, 0, 113, 7);
        assert_eq!(limiter.per_ip_count_for_test(ip), 0);

        let g1 = limiter.try_acquire_ip_for_test(ip).expect("slot 1");
        let g2 = limiter.try_acquire_ip_for_test(ip).expect("slot 2");
        assert_eq!(
            limiter.per_ip_count_for_test(ip),
            2,
            "two live conns for the IP"
        );
        assert!(
            limiter.try_acquire_ip_for_test(ip).is_none(),
            "a 3rd connection from the same IP is REFUSED (per-source cap = 2)"
        );

        // Dropping one guard frees a slot for that IP; the map entry is removed only at zero.
        drop(g1);
        assert_eq!(limiter.per_ip_count_for_test(ip), 1);
        let g3 = limiter
            .try_acquire_ip_for_test(ip)
            .expect("a freed per-source slot re-admits the IP");
        assert_eq!(limiter.per_ip_count_for_test(ip), 2);

        drop(g2);
        drop(g3);
        assert_eq!(
            limiter.per_ip_count_for_test(ip),
            0,
            "all guards dropped ⇒ the IP's count is 0 (entry removed — map stays bounded)"
        );
        // And the source can connect afresh (its throttle is not permanently stuck).
        assert!(limiter.try_acquire_ip_for_test(ip).is_some());
    }

    #[test]
    fn per_ip_cap_isolates_sources() {
        // MUTATION KILL: one IP exhausting its per-source cap must NOT affect a DIFFERENT IP.
        let limiter = ConnectionLimiter::new(10_000).with_per_ip_cap(Some(1), false);
        let a = pub_ip(198, 51, 100, 1);
        let b = pub_ip(198, 51, 100, 2);
        let _ga = limiter.try_acquire_ip_for_test(a).expect("A slot 1");
        assert!(
            limiter.try_acquire_ip_for_test(a).is_none(),
            "A is at its cap"
        );
        // B is unaffected — its own independent per-source slot.
        assert!(
            limiter.try_acquire_ip_for_test(b).is_some(),
            "B has its own per-source slot (a shared counter would refuse it)"
        );
    }

    #[test]
    fn per_ip_cap_exempts_internal_ips_by_default() {
        // exempt_internal=true (the default): internal source IPs are NEVER per-source-capped (the
        // proxy/docker/k8s/conformance hop footgun guard) — a public IP still is.
        let limiter = ConnectionLimiter::new(10_000).with_per_ip_cap(Some(1), true);
        for internal in [
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            pub_ip(10, 1, 2, 3),
            pub_ip(192, 168, 0, 1),
            "::1".parse().unwrap(),
            "fe80::1".parse().unwrap(),
        ] {
            // Many acquires from an internal IP all succeed (exempt); none are tracked.
            for _ in 0..5 {
                assert!(
                    limiter.try_acquire_ip_for_test(internal).is_some(),
                    "internal IP {internal} must be exempt from the per-source cap"
                );
            }
            assert_eq!(
                limiter.per_ip_count_for_test(internal),
                0,
                "an exempt internal IP is not tracked in the map"
            );
        }
        // A PUBLIC IP is still capped at 1.
        let public = pub_ip(203, 0, 113, 9);
        let _g = limiter
            .try_acquire_ip_for_test(public)
            .expect("public slot 1");
        assert!(
            limiter.try_acquire_ip_for_test(public).is_none(),
            "a public IP is still per-source-capped"
        );
    }

    #[test]
    fn per_ip_cap_disabled_admits_everything() {
        // `with_per_ip_cap(None, ..)` (the default from `new`) ⇒ the per-source cap is off: every
        // acquire is admitted and nothing is tracked (global cap only).
        let limiter = ConnectionLimiter::new(10_000); // per-IP disabled by default
        assert_eq!(limiter.max_per_ip(), None);
        let ip = pub_ip(203, 0, 113, 1);
        for _ in 0..1000 {
            assert!(limiter.try_acquire_ip_for_test(ip).is_some());
        }
        assert_eq!(
            limiter.per_ip_count_for_test(ip),
            0,
            "per-source cap off ⇒ no per-IP tracking"
        );
    }

    #[test]
    fn per_ip_cap_zero_is_clamped_to_one_never_refuses_all() {
        // A direct `Some(0)` construction must NOT refuse every connection — it is clamped to 1 (the env
        // parser already maps `0` to disabled; this guards a direct mis-construction).
        let limiter = ConnectionLimiter::new(10_000).with_per_ip_cap(Some(0), false);
        assert_eq!(limiter.max_per_ip(), Some(1));
        let ip = pub_ip(203, 0, 113, 2);
        assert!(
            limiter.try_acquire_ip_for_test(ip).is_some(),
            "a fresh IP always gets its first connection (cap clamped to >=1)"
        );
    }

    #[test]
    fn max_requests_per_conn_disabled_on_absent_empty_invalid_or_zero() {
        // The DEFAULT must be disabled (None) — a finite reuse cap is opt-in. A `0`/typo must NEVER
        // become a 0-request cap (which would brick keep-alive); it maps to None (unlimited).
        assert_eq!(parse_max_requests_per_conn(None), None);
        assert_eq!(parse_max_requests_per_conn(Some("".into())), None);
        assert_eq!(parse_max_requests_per_conn(Some("  ".into())), None);
        assert_eq!(parse_max_requests_per_conn(Some("abc".into())), None);
        assert_eq!(parse_max_requests_per_conn(Some("0".into())), None);
        // A positive value enables the cap.
        assert_eq!(parse_max_requests_per_conn(Some("1".into())), Some(1));
        assert_eq!(parse_max_requests_per_conn(Some("1000".into())), Some(1000));
        assert_eq!(parse_max_requests_per_conn(Some("  64  ".into())), Some(64));
    }

    #[test]
    fn idle_timeout_is_default_off_and_zero_disables() {
        // DEFAULT-OFF (the upgraded-WS-connection fail-safe, roborev High): absent / empty / invalid /
        // `0` ⇒ None (DISABLED). Only a positive value enables it. This is the load-bearing safety
        // property — an absent or typo'd value must NEVER silently turn on an IO-layer idle bound that
        // could close a WebSocketChannel2023 receive socket.
        assert_eq!(parse_idle_timeout(None), None);
        assert_eq!(parse_idle_timeout(Some("".into())), None);
        assert_eq!(parse_idle_timeout(Some("  ".into())), None);
        assert_eq!(parse_idle_timeout(Some("abc".into())), None);
        assert_eq!(parse_idle_timeout(Some("0".into())), None);
        // The default constant is 0 (disabled) — pin it so a future bump is a conscious choice.
        assert_eq!(DEFAULT_IDLE_TIMEOUT_SECS, 0);
        // A positive value enables it.
        assert_eq!(
            parse_idle_timeout(Some("75".into())),
            Some(Duration::from_secs(75))
        );
        assert_eq!(
            parse_idle_timeout(Some("  30  ".into())),
            Some(Duration::from_secs(30))
        );
    }

    #[tokio::test]
    async fn idle_timeout_stream_resets_on_write_activity_does_not_close_active_pusher() {
        // An actively-PUSHING connection (server→client writes, no inbound reads — the WS-push shape)
        // must NOT be closed by the idle bound: a write resets the deadline. We poll READ and WRITE on
        // the SAME stream from one task (no lock — as hyper drives a connection): the read stays PENDING
        // while periodic writes keep the deadline fresh, across a TOTAL time > the idle window. Without
        // the write-reset, the read would resolve to a TimedOut error within one window.
        use std::future::poll_fn;
        use tokio::io::ReadBuf;
        use tokio::time::{sleep, Duration as TDur};

        let idle_window = TDur::from_millis(120);
        let (client, server) = tokio::io::duplex(1024);
        // Keep the client end open so the duplex never EOFs (an EOF would resolve the read independently
        // of the idle bound and mask what we're testing).
        let _client_keepalive = client;
        let mut idle = IdleTimeoutStream::new(server, Some(idle_window), None);

        // Drive 5 write+poll-read cycles, each half a window apart (total ~5*60ms = 300ms > 120ms idle).
        // Each iteration: WRITE (resets the deadline), then poll READ ONCE (must be Pending, NOT a
        // TimedOut error). The reads never get bytes (client sends nothing), so the ONLY way a read here
        // becomes Ready is the idle bound firing — which the writes must prevent.
        let mut rbuf = [0u8; 1];
        for i in 0..5 {
            // Write (server→client push) — resets the idle deadline.
            poll_fn(|cx| Pin::new(&mut idle).poll_write(cx, b"push"))
                .await
                .expect("server push write");
            // Poll the read ONCE; it must be Pending (no data, and the deadline was just reset).
            let read_state = poll_fn(|cx| {
                let mut buf = ReadBuf::new(&mut rbuf);
                match Pin::new(&mut idle).poll_read(cx, &mut buf) {
                    Poll::Pending => Poll::Ready(Ok::<bool, io::Error>(false)), // pending = healthy
                    Poll::Ready(Ok(())) => Poll::Ready(Ok(true)), // EOF/data (unexpected)
                    Poll::Ready(Err(e)) => Poll::Ready(Err(e)),   // idle fired (the guard)
                }
            })
            .await;
            match read_state {
                Ok(false) => { /* pending — healthy; the write kept it alive */ }
                Ok(true) => panic!("read iteration {i} got data/EOF unexpectedly (test setup wrong)"),
                Err(e) => panic!(
                    "the idle bound FIRED on an actively-PUSHING connection at iteration {i} ({e}) — a \
                     write must reset the deadline (the WS-push fail-safe)"
                ),
            }
            sleep(idle_window / 2).await;
        }
    }

    #[tokio::test]
    async fn idle_timeout_stream_passes_through_then_fires_when_idle() {
        // Drive the IdleTimeoutStream over an in-memory duplex pair with a SHORT real idle window. A read
        // that gets bytes succeeds and RESETS the deadline; after a full idle window with no bytes, the
        // next read resolves to a TimedOut error (NOT a hang).
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let idle_window = Duration::from_millis(150);
        let (mut client, server) = tokio::io::duplex(64);
        let mut idle = IdleTimeoutStream::new(server, Some(idle_window), None);

        // 1) A read that gets bytes succeeds (pass-through) and resets the idle deadline.
        client.write_all(b"hello").await.unwrap();
        let mut buf = [0u8; 5];
        idle.read_exact(&mut buf)
            .await
            .expect("read passes through");
        assert_eq!(&buf, b"hello");

        // 2) Now go idle (no further writes). The next read must resolve to a TimedOut error within a
        // few idle windows (the idle guard fired) rather than hang. The outer timeout is generously
        // larger than the idle window so it only trips if the guard FAILED to fire (a real hang).
        let mut buf2 = [0u8; 1];
        let res = tokio::time::timeout(idle_window * 20, idle.read(&mut buf2)).await;
        let inner =
            res.expect("the idle read must RESOLVE (the guard fires), not hang past the window");
        let err = inner.expect_err("an idle connection past the window must resolve to an error");
        assert_eq!(
            err.kind(),
            io::ErrorKind::TimedOut,
            "the idle-keepalive guard must close with a TimedOut error, got {err:?}"
        );
    }

    #[tokio::test]
    async fn tcpstream_peer_ip_is_extracted_for_the_per_source_cap() {
        // The PeerAddr wiring the acceptor relies on: a real TcpStream reports its peer IP (loopback
        // here), which is what the per-source cap keys on. Proves the extraction end (the `()` test
        // stream returns None → fail-open; this proves the live path returns Some).
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let client = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (server_stream, _peer) = listener.accept().await.unwrap();
        // Server side sees the client's loopback IP as the peer.
        assert_eq!(
            PeerAddr::peer_ip(&server_stream),
            Some(IpAddr::V4(Ipv4Addr::LOCALHOST)),
            "a live TcpStream must expose its peer IP for the per-source cap"
        );
        // The `()` test stream fails open (no peer IP).
        assert_eq!(PeerAddr::peer_ip(&()), None);
        drop(client);
    }

    #[tokio::test]
    async fn idle_timeout_stream_none_is_transparent_passthrough() {
        // With idle = None the wrapper is a transparent pass-through: a delayed write is still read fine
        // (no timer, no spurious close) even though a real delay elapses between the read starting and
        // the bytes arriving.
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (mut client, server) = tokio::io::duplex(64);
        let mut idle = IdleTimeoutStream::new(server, None, None);

        // Spawn a delayed writer; the read must wait for it (no idle bound closes it early).
        let writer = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            client.write_all(b"x").await.unwrap();
            client // keep the client end alive until the read completes
        });
        let mut buf = [0u8; 1];
        idle.read_exact(&mut buf)
            .await
            .expect("None ⇒ transparent pass-through (a delayed read still succeeds)");
        assert_eq!(&buf, b"x");
        let _client = writer.await.unwrap();
    }

    #[tokio::test]
    async fn max_requests_service_stamps_connection_close_at_the_cap_only() {
        // A stub tower::Service returning a 200 with no body. Wrap it with a cap of 2 and call it 3
        // times (simulating 3 requests on ONE connection — the per-connection counter is shared across
        // the wrapper's clones the way hyper clones the service per request). The 1st response must NOT
        // carry `Connection: close`; the 2nd (== cap) and 3rd (over cap) MUST.
        use std::task::Poll;
        use tower::Service;

        #[derive(Clone)]
        struct Ok200;
        impl Service<http::Request<()>> for Ok200 {
            type Response = http::Response<()>;
            type Error = std::convert::Infallible;
            type Future =
                std::pin::Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;
            fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
                Poll::Ready(Ok(()))
            }
            fn call(&mut self, _req: http::Request<()>) -> Self::Future {
                Box::pin(async { Ok(http::Response::new(())) })
            }
        }

        let mut svc = MaxRequestsService::new(Ok200, Some(2), None);
        // The service now yields `TrackedBody`-wrapped responses; headers are inspected the same way.
        let has_close = |resp: &http::Response<TrackedBody<()>>| {
            resp.headers()
                .get(http::header::CONNECTION)
                .map(|v| v.as_bytes().eq_ignore_ascii_case(b"close"))
                .unwrap_or(false)
        };

        let r1 = svc.call(http::Request::new(())).await.unwrap();
        assert!(
            !has_close(&r1),
            "request 1 (< cap) must NOT carry Connection: close"
        );
        let r2 = svc.call(http::Request::new(())).await.unwrap();
        assert!(
            has_close(&r2),
            "request 2 (== cap) MUST carry Connection: close"
        );
        let r3 = svc.call(http::Request::new(())).await.unwrap();
        assert!(
            has_close(&r3),
            "request 3 (> cap) MUST also carry Connection: close (monotone after the cap)"
        );
    }

    #[tokio::test]
    async fn max_requests_service_none_never_stamps_close() {
        // With the cap disabled (None), no response ever gets `Connection: close` — unlimited reuse.
        use std::task::Poll;
        use tower::Service;

        #[derive(Clone)]
        struct Ok200;
        impl Service<http::Request<()>> for Ok200 {
            type Response = http::Response<()>;
            type Error = std::convert::Infallible;
            type Future =
                std::pin::Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;
            fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
                Poll::Ready(Ok(()))
            }
            fn call(&mut self, _req: http::Request<()>) -> Self::Future {
                Box::pin(async { Ok(http::Response::new(())) })
            }
        }

        let mut svc = MaxRequestsService::new(Ok200, None, None);
        for _ in 0..5 {
            let resp = svc.call(http::Request::new(())).await.unwrap();
            assert!(
                resp.headers().get(http::header::CONNECTION).is_none(),
                "with the cap disabled, no response may carry Connection: close"
            );
        }
    }

    #[tokio::test]
    async fn max_requests_service_never_clobbers_a_101_websocket_upgrade() {
        // The WebSocketChannel2023 receive-socket handshake is a `101 Switching Protocols` served through
        // the SAME guarded acceptor. With cap=1 the VERY FIRST request on a fresh connection is already
        // at the cap, so without the exemption every WS upgrade would be stamped `Connection: close` and
        // the client would FAIL the RFC 6455 handshake. The guard must leave the upgrade response's
        // `Connection: Upgrade` intact.
        use std::task::Poll;
        use tower::Service;

        // A stub service returning the axum-shaped WS upgrade response: 101 + `Connection: Upgrade` +
        // `Upgrade: websocket` (exactly what `WebSocketUpgrade` emits).
        #[derive(Clone)]
        struct Ws101;
        impl Service<http::Request<()>> for Ws101 {
            type Response = http::Response<()>;
            type Error = std::convert::Infallible;
            type Future =
                std::pin::Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;
            fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
                Poll::Ready(Ok(()))
            }
            fn call(&mut self, _req: http::Request<()>) -> Self::Future {
                Box::pin(async {
                    let mut resp = http::Response::new(());
                    *resp.status_mut() = http::StatusCode::SWITCHING_PROTOCOLS;
                    resp.headers_mut().insert(
                        http::header::CONNECTION,
                        http::HeaderValue::from_static("Upgrade"),
                    );
                    resp.headers_mut().insert(
                        http::header::UPGRADE,
                        http::HeaderValue::from_static("websocket"),
                    );
                    Ok(resp)
                })
            }
        }

        // cap=1 ⇒ the first (and every) request is at/over the cap ⇒ the guard WOULD stamp close.
        let mut svc = MaxRequestsService::new(Ws101, Some(1), None);
        let resp = svc.call(http::Request::new(())).await.unwrap();

        assert_eq!(
            resp.status(),
            http::StatusCode::SWITCHING_PROTOCOLS,
            "the 101 status must be preserved"
        );
        let conn = resp
            .headers()
            .get(http::header::CONNECTION)
            .expect("the upgrade response must retain its Connection header");
        assert!(
            conn.as_bytes().eq_ignore_ascii_case(b"upgrade"),
            "the max-requests cap MUST NOT clobber `Connection: Upgrade` on a 101 — got {conn:?} (WS \
             handshake would fail, RFC 6455 §4.1)"
        );
        assert!(
            resp.headers().get(http::header::UPGRADE).is_some(),
            "the Upgrade header must survive the guard"
        );
    }

    #[tokio::test]
    async fn max_requests_service_still_stamps_close_on_a_non_101_upgrade_advertising_response() {
        // Adversarial-review MED (PR #6): the upgrade exemption must key on the `101` STATUS, not the mere
        // presence of an `Upgrade` header. A `426 Upgrade Required` (or any non-101 response advertising
        // `Upgrade`) is an ORDINARY keep-alive response — it is NOT a completed protocol switch — so the
        // max-requests cap must still stamp `Connection: close` on it at/after the cap. The earlier,
        // over-broad predicate exempted every `Upgrade`-carrying response, letting such responses bypass
        // the reuse cap indefinitely.
        use std::task::Poll;
        use tower::Service;

        // A stub returning a `426 Upgrade Required` that advertises `Upgrade` (but is NOT a 101 switch).
        #[derive(Clone)]
        struct Upgrade426;
        impl Service<http::Request<()>> for Upgrade426 {
            type Response = http::Response<()>;
            type Error = std::convert::Infallible;
            type Future =
                std::pin::Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;
            fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
                Poll::Ready(Ok(()))
            }
            fn call(&mut self, _req: http::Request<()>) -> Self::Future {
                Box::pin(async {
                    let mut resp = http::Response::new(());
                    *resp.status_mut() = http::StatusCode::UPGRADE_REQUIRED;
                    resp.headers_mut().insert(
                        http::header::UPGRADE,
                        http::HeaderValue::from_static("HTTP/2.0"),
                    );
                    Ok(resp)
                })
            }
        }

        // cap=1 ⇒ the first request is at the cap ⇒ the guard must stamp close on this non-101 response.
        let mut svc = MaxRequestsService::new(Upgrade426, Some(1), None);
        let resp = svc.call(http::Request::new(())).await.unwrap();

        let conn = resp
            .headers()
            .get(http::header::CONNECTION)
            .expect("a non-101 Upgrade-advertising response at the cap MUST get Connection: close");
        assert!(
            conn.as_bytes().eq_ignore_ascii_case(b"close"),
            "the max-requests cap MUST still close a non-101 (426) response that merely advertises \
             Upgrade — got {conn:?}; exempting it would let it bypass the reuse cap"
        );
    }

    #[test]
    fn is_upgrade_response_exempts_only_101_not_advisory_upgrade_headers() {
        // 101 Switching Protocols ⇒ a real completed upgrade (exempt from the close stamp).
        let mut r = http::Response::new(());
        *r.status_mut() = http::StatusCode::SWITCHING_PROTOCOLS;
        assert!(is_upgrade_response(&r));

        // A 101 is exempt on its STATUS alone — even the (malformed) case with no upgrade tokens: we must
        // never stamp `Connection: close` on a Switching-Protocols response.
        let mut r = http::Response::new(());
        *r.status_mut() = http::StatusCode::SWITCHING_PROTOCOLS;
        r.headers_mut().insert(
            http::header::CONNECTION,
            http::HeaderValue::from_static("close"),
        );
        assert!(is_upgrade_response(&r));

        // NON-101 with a `Connection: upgrade` token ⇒ NOT exempt (adversarial-review MED, PR #6): the
        // token is only advisory here; this is an ordinary keep-alive response the cap must still close.
        let mut r = http::Response::new(());
        r.headers_mut().insert(
            http::header::CONNECTION,
            http::HeaderValue::from_static("keep-alive, Upgrade"),
        );
        assert!(!is_upgrade_response(&r));

        // NON-101 with a bare `Upgrade` header (e.g. a `426 Upgrade Required`) ⇒ NOT exempt: the
        // over-broad predicate used to let such responses bypass the reuse cap indefinitely.
        let mut r = http::Response::new(());
        *r.status_mut() = http::StatusCode::UPGRADE_REQUIRED;
        r.headers_mut().insert(
            http::header::UPGRADE,
            http::HeaderValue::from_static("websocket"),
        );
        assert!(!is_upgrade_response(&r));

        // A `200` advertising `Upgrade: h2c` ⇒ NOT exempt (advisory, not a completed switch).
        let mut r = http::Response::new(());
        r.headers_mut()
            .insert(http::header::UPGRADE, http::HeaderValue::from_static("h2c"));
        assert!(!is_upgrade_response(&r));

        // A plain 200 with no upgrade signals ⇒ NOT an upgrade (the cap may stamp close).
        let r = http::Response::new(());
        assert!(!is_upgrade_response(&r));

        // `Connection: close` on a non-101 is not an upgrade token.
        let mut r = http::Response::new(());
        r.headers_mut().insert(
            http::header::CONNECTION,
            http::HeaderValue::from_static("close"),
        );
        assert!(!is_upgrade_response(&r));
    }

    #[tokio::test]
    async fn idle_timeout_stream_pauses_while_a_request_is_in_flight_then_fires_when_idle() {
        // The idle guard must be request-AWARE: while an HTTP exchange is in flight (a slow handler, a
        // trickling PUT, or a stalled streaming GET — modelled here by the shared in-flight count > 0) the
        // idle clock is PAUSED, so the connection is NEVER severed no matter how long the exchange takes.
        // Once the exchange ends (count back to 0) the connection is genuinely idle and MUST close after a
        // window. Without the fix the in-flight read would resolve to a TimedOut error within one window.
        use std::future::poll_fn;
        use tokio::io::{AsyncReadExt, ReadBuf};
        use tokio::time::{sleep, Duration as TDur};

        let idle_window = TDur::from_millis(100);
        let (client, server) = tokio::io::duplex(64);
        // Keep the client end open so the duplex never EOFs (an EOF would resolve the read on its own).
        let _client_keepalive = client;
        let in_flight = Arc::new(AtomicUsize::new(0));
        let mut idle = IdleTimeoutStream::new(server, Some(idle_window), Some(in_flight.clone()));

        // A request is dispatched and its handler is slow / its response is streaming: mark it in flight.
        in_flight.store(1, Ordering::Relaxed);

        // Poll the read repeatedly across MUCH more than one idle window (6 * 50ms = 300ms >> 100ms). No
        // bytes ever arrive, so the ONLY way a read becomes Ready is the idle bound firing — which the
        // in-flight pause must prevent for the whole duration.
        let mut rbuf = [0u8; 1];
        for i in 0..6 {
            sleep(idle_window / 2).await;
            let read_state = poll_fn(|cx| {
                let mut buf = ReadBuf::new(&mut rbuf);
                match Pin::new(&mut idle).poll_read(cx, &mut buf) {
                    Poll::Pending => Poll::Ready(Ok::<bool, io::Error>(false)), // pending = healthy
                    Poll::Ready(Ok(())) => Poll::Ready(Ok(true)), // data/EOF (unexpected)
                    Poll::Ready(Err(e)) => Poll::Ready(Err(e)),   // idle FIRED (the bug)
                }
            })
            .await;
            match read_state {
                Ok(false) => { /* pending — healthy; the in-flight pause held */ }
                Ok(true) => panic!("read iteration {i} got data/EOF (test setup wrong)"),
                Err(e) => panic!(
                    "the idle bound FIRED on an IN-FLIGHT request at iteration {i} ({e}) — the idle \
                     clock must be paused while an HTTP exchange is in flight"
                ),
            }
        }

        // The exchange finishes ⇒ no longer in flight ⇒ the connection is now genuinely idle and MUST
        // close after a window (the guard is not disabled, it was only paused).
        in_flight.store(0, Ordering::Relaxed);
        let mut buf2 = [0u8; 1];
        let res = tokio::time::timeout(idle_window * 20, idle.read(&mut buf2)).await;
        let inner = res.expect("once idle the read must RESOLVE (the guard fires), not hang");
        let err = inner.expect_err("a genuinely idle connection past the window must close");
        assert_eq!(
            err.kind(),
            io::ErrorKind::TimedOut,
            "an idle connection (no in-flight exchange) past the window must close with TimedOut, got \
             {err:?}"
        );
    }

    #[tokio::test]
    async fn idle_timeout_stream_grants_a_full_fresh_window_after_an_exchange_ends() {
        // Regression for the in-flight→idle TRANSITION (adversarial-review MED, PR #6). While an exchange
        // is in flight the idle deadline is only refreshed when `poll_read` is actually CALLED. A long
        // exchange that makes NO read progress for more than a window (a slow handler with a quiet client)
        // therefore leaves the deadline STALE (in the past). When the exchange then ends, the FIRST idle
        // `poll_read` must NOT fire on that stale deadline — the connection becomes idle at exchange-END
        // and is owed a FULL FRESH window. Without the transition-reset the read resolves to TimedOut
        // immediately, prematurely severing a keep-alive connection right after a slow request.
        use std::future::poll_fn;
        use tokio::io::{AsyncReadExt, ReadBuf};
        use tokio::time::{sleep, Duration as TDur};

        let idle_window = TDur::from_millis(100);
        let (client, server) = tokio::io::duplex(64);
        let _client_keepalive = client; // keep the client end open so the duplex never EOFs
        let in_flight = Arc::new(AtomicUsize::new(0));
        let mut idle = IdleTimeoutStream::new(server, Some(idle_window), Some(in_flight.clone()));

        // An exchange is dispatched (in flight). ONE poll_read arms/refreshes the deadline, then there are
        // NO further reads while the slow exchange runs (the quiet-client case).
        in_flight.store(1, Ordering::Relaxed);
        let mut rbuf = [0u8; 1];
        let first = poll_fn(|cx| {
            let mut buf = ReadBuf::new(&mut rbuf);
            Poll::Ready(Pin::new(&mut idle).poll_read(cx, &mut buf))
        })
        .await;
        assert!(
            matches!(first, Poll::Pending),
            "the in-flight read must be Pending (no bytes, exchange in flight)"
        );

        // Let MORE than a full window pass with the exchange STILL in flight and NO intervening poll_read,
        // so the last-set deadline is now in the PAST (stale).
        sleep(idle_window + idle_window / 2).await; // 150ms > the 100ms window

        // The exchange ends. The very first idle poll_read must treat the connection as idle-from-NOW and
        // must NOT immediately fire on the stale in-flight deadline.
        in_flight.store(0, Ordering::Relaxed);
        let transition = poll_fn(|cx| {
            let mut buf = ReadBuf::new(&mut rbuf);
            Poll::Ready(Pin::new(&mut idle).poll_read(cx, &mut buf))
        })
        .await;
        match transition {
            Poll::Pending => { /* healthy: a full fresh window was granted at the transition */ }
            Poll::Ready(Ok(())) => panic!("unexpected data/EOF (test setup wrong)"),
            Poll::Ready(Err(e)) => panic!(
                "the idle bound FIRED on the FIRST idle poll after an exchange ended ({e}) — the \
                 in-flight→idle transition must grant a full fresh idle window, not fire on the stale \
                 in-flight deadline"
            ),
        }

        // The fresh window IS still enforced: once genuinely idle past the NEW window, the connection
        // closes (the guard was only reset, never disabled).
        let mut buf2 = [0u8; 1];
        let res = tokio::time::timeout(idle_window * 20, idle.read(&mut buf2)).await;
        let inner =
            res.expect("once idle the read must resolve (the guard fires after the fresh window)");
        let err = inner.expect_err("a genuinely idle connection past the fresh window must close");
        assert_eq!(
            err.kind(),
            io::ErrorKind::TimedOut,
            "after the fresh window elapses with no activity, the idle connection must close with \
             TimedOut, got {err:?}"
        );
    }

    #[tokio::test]
    async fn max_requests_service_in_flight_guard_spans_request_and_response_body() {
        // The in-flight-exchange count must stay raised for the WHOLE exchange — from request dispatch
        // through the response body streaming — and drop back to 0 exactly at end-of-stream, so the idle
        // guard treats a slow/streaming RESPONSE as "not idle" (never severing it) and reclaims the
        // connection promptly once the body has drained.
        use bytes::Bytes;
        use http_body_util::Full;
        use std::future::poll_fn;
        use std::task::Poll;
        use tower::Service;

        // A stub service returning a body with content (one DATA frame, then end-of-stream).
        #[derive(Clone)]
        struct BodySvc;
        impl Service<http::Request<()>> for BodySvc {
            type Response = http::Response<Full<Bytes>>;
            type Error = std::convert::Infallible;
            type Future =
                std::pin::Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;
            fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
                Poll::Ready(Ok(()))
            }
            fn call(&mut self, _req: http::Request<()>) -> Self::Future {
                Box::pin(async { Ok(http::Response::new(Full::new(Bytes::from_static(b"hello")))) })
            }
        }

        let in_flight = Arc::new(AtomicUsize::new(0));
        let mut svc = MaxRequestsService::new(BodySvc, None, Some(in_flight.clone()));

        // Dispatch: the exchange is counted from dispatch and remains counted at the response head.
        let resp = svc.call(http::Request::new(())).await.unwrap();
        assert_eq!(
            in_flight.load(Ordering::Relaxed),
            1,
            "the exchange must be counted from dispatch through the produced response"
        );

        let mut body = resp.into_body();
        // First frame is data — still in flight while the body streams.
        let frame1 = poll_fn(|cx| Pin::new(&mut body).poll_frame(cx)).await;
        assert!(
            frame1.is_some_and(|f| f.is_ok()),
            "expected a data frame from the streaming body"
        );
        assert_eq!(
            in_flight.load(Ordering::Relaxed),
            1,
            "the exchange must STILL be counted while the response body is streaming"
        );

        // Next poll is end-of-stream — the guard releases eagerly, count returns to 0 (idle again).
        let frame2 = poll_fn(|cx| Pin::new(&mut body).poll_frame(cx)).await;
        assert!(frame2.is_none(), "expected end-of-stream");
        assert_eq!(
            in_flight.load(Ordering::Relaxed),
            0,
            "the in-flight guard must release at end-of-stream so the connection is idle again"
        );
    }

    #[tokio::test]
    async fn max_requests_service_in_flight_guard_releases_on_dropped_body() {
        // Cancellation-safety: if the response body is DROPPED before it finishes streaming (client
        // disconnect, connection reset), the in-flight guard must still release — the count must not leak
        // and wedge the idle guard permanently paused.
        use bytes::Bytes;
        use http_body_util::Full;
        use std::task::Poll;
        use tower::Service;

        #[derive(Clone)]
        struct BodySvc;
        impl Service<http::Request<()>> for BodySvc {
            type Response = http::Response<Full<Bytes>>;
            type Error = std::convert::Infallible;
            type Future =
                std::pin::Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;
            fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
                Poll::Ready(Ok(()))
            }
            fn call(&mut self, _req: http::Request<()>) -> Self::Future {
                Box::pin(async { Ok(http::Response::new(Full::new(Bytes::from_static(b"hello")))) })
            }
        }

        let in_flight = Arc::new(AtomicUsize::new(0));
        let mut svc = MaxRequestsService::new(BodySvc, None, Some(in_flight.clone()));
        let resp = svc.call(http::Request::new(())).await.unwrap();
        assert_eq!(in_flight.load(Ordering::Relaxed), 1);

        // Drop the response (and its body) WITHOUT draining it — the guard must decrement.
        drop(resp);
        assert_eq!(
            in_flight.load(Ordering::Relaxed),
            0,
            "dropping an un-drained response body must release the in-flight guard (no leak)"
        );
    }
}
