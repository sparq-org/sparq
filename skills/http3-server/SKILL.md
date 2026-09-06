---
name: http3-server
description: Embed sparq's internal opt-in HTTP/3-over-QUIC bridge into an axum server with rustls 0.23/aws-lc-rs, Quinn, peer ConnectInfo injection, and graceful shutdown.
license: MIT
metadata:
  version: "0.1.0"
  homepage: https://github.com/sparq-org/sparq
---

# sparq HTTP/3 server bridge

Use `sparq-http3` when a sparq HTTP server needs a second, encrypted UDP listener that
dispatches into the same `axum::Router` as its existing HTTP/1.1 or HTTP/2 listener. The
crate is internal and unstable (`publish = false`), and its `server` feature is off by
default so Quinn and the pre-1.0 h3 stack do not enter ordinary workspace builds.

## Add the opt-in dependency

```toml
[features]
default = []
http3 = ["dep:sparq-http3"]

[dependencies]
sparq-http3 = { path = "../sparq-http3", optional = true, features = ["server"] }
```

The downstream crate owns certificate/key loading and runtime configuration. Build a
`rustls::ServerConfig` with the aws-lc-rs provider, TLS 1.3, and the exact `h3` ALPN token:

```rust
use std::sync::Arc;

let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
let mut tls = rustls::ServerConfig::builder_with_provider(provider)
    .with_protocol_versions(&[&rustls::version::TLS13])?
    .with_no_client_auth()
    .with_single_cert(certificates, private_key)?;
tls.alpn_protocols = vec![b"h3".to_vec()];

let quic = sparq_http3::quic_server_config(tls)?;
let endpoint = quinn::Endpoint::server(quic, udp_addr)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

`quic_server_config` fails closed when `h3` is missing from ALPN or the provider lacks
QUIC's required initial cipher suite. It also replaces Quinn's effectively unbounded default
connection receive window with an owned finite bound; a caller may still replace the returned
config's transport settings before binding the endpoint.

## Serve the shared router

```rust
sparq_http3::serve_h3(endpoint, router.clone(), async move {
    shutdown_signal.await;
})
.await?;
# Ok::<(), std::io::Error>(())
```

The compatibility entry point is safe by default: it limits total concurrent connections,
connections from one public IP, and concurrent requests on each connection. Internal addresses are
exempt from the per-IP cap to avoid treating a proxy, container bridge, or conformance runner as
one client, but the global cap still applies.
Existing callers need no changes. To tune the caps, use the additive variant:

```rust
let limits = sparq_http3::H3ConnectionLimits {
    max_connections: 2_000,
    max_connections_per_ip: Some(128),
    exempt_internal_ips: true,
    max_requests_per_connection: 64,
};
sparq_http3::serve_h3_with_limits(endpoint, router.clone(), limits, shutdown_signal).await?;
# Ok::<(), std::io::Error>(())
```

Set `max_connections_per_ip` to `None` only when the global cap is sufficient for the deployment.
Zero numeric caps are clamped to one rather than disabling the listener.
`max_requests_per_connection` is a stream-exhaustion bound ([FABLE-5] sq-4rkcc): each connection
acquires a semaphore permit before accepting its next request stream, and the permit is held by
the request task until it completes, so one connection cannot fan out unbounded concurrent
request tasks. At the cap, acceptance of further request streams on that connection back-pressures
until an in-flight request finishes; already-running requests keep progressing, and other
connections are unaffected. The bridge clones the
router per connection and request, streams request and response bodies, and inserts the Quinn peer
address as `axum::extract::ConnectInfo<SocketAddr>`.
That extension is load-bearing for request policies keyed by the remote socket address.
Protocol failures are isolated to their connection or stream; resolving the shutdown
future closes the endpoint and waits for its QUIC connections to drain.

## Advertise a bound endpoint to TCP clients

After the QUIC endpoint has bound successfully, apply the shared response layer to the separate
HTTP/1.1 or HTTP/2 router. Derive the advertised port from `Endpoint::local_addr`; do not use the
requested configuration because binding port `0` may select a different live port:

```rust
let endpoint = quinn::Endpoint::server(quic, udp_addr)?;
let bound_addr = endpoint.local_addr()?;
let h3_router = router.clone();
let tcp_router = router.layer(sparq_http3::alt_svc_layer(bound_addr.port()));
```

`alt_svc_layer` overwrites `Alt-Svc` with `h3=":<port>"; ma=86400`. Serve `tcp_router` only
while the bound endpoint is also being served; use the unlayered `h3_router` for `serve_h3`.
Feature-enabled but unconfigured servers must keep serving the original router without this layer.

## Solid/LDP server integration

`sparq-lws-core` carries this bridge behind its default-off `http3` feature
([GPT-5.6] sq-oprna.2):

```sh
SOLID_SERVER_TLS_CERT=/path/to/cert.pem \
SOLID_SERVER_TLS_KEY=/path/to/key.pem \
cargo run -p sparq-lws-core --features http3
```

When both TLS paths are configured, the binary binds QUIC/UDP to the same resolved
`SOLID_SERVER_BIND` address and port as TLS/TCP and serves one cloned
`build_router_with_overload` router. The TCP rustls configuration remains exactly
`[h2, http/1.1]`; a clone used only by Quinn advertises the exact `h3` token. Every
QUIC request receives peer `ConnectInfo`, so the pre-crypto per-IP rate limiter does
not fail open to the ordinary auth stack. With no TLS configuration, enabling the
Cargo feature does not create a QUIC listener. WebSocket notifications continue over
HTTP/1.1 or HTTP/2 because RFC 9220 extended CONNECT is outside this integration.

## Boundaries

- The caller owns rustls policy, certificates, client authentication, any Quinn transport
  tuning beyond the helper's bounded receive window, listener binding, and process-wide provider
  installation.
- HTTP/3 is a separate UDP listener. Keep the existing TCP listener running for HTTP/1.1
  and HTTP/2 clients.
- `h3` and `h3-quinn` are pre-1.0 dependencies; keep all direct use inside this helper.
- WebSocket-over-HTTP/3 extended CONNECT is not implemented. WebSocket clients fall back
  to the existing TCP listener.
- Do not call `alt_svc_layer` until the UDP listener has successfully bound, and remove the
  advertisement whenever that listener is not served.

## Related material

- `crates/sparq-http3/README.md` — crate scope and opt-in posture.
- `research/http3-quic-servers-design.md` — shared-listener architecture and maturity
  boundary.
- `skills/http-server/SKILL.md` — the SPARQL Protocol server surface.
