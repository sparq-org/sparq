<!-- sq-oprna.1: internal-stub README for a publish=false crate. -->
# sparq-http3

## 🚀 Quick start

Internal, unstable HTTP/3-over-QUIC transport bridge shared by sparq's axum servers.
Its default-off `server` feature contains the pre-1.0 `h3`/`h3-quinn` stack and Quinn,
so a default workspace build does not pull those dependencies.

## ✨ Capabilities

Enable `server` to convert an aws-lc-rs-backed rustls server configuration and dispatch
HTTP/3 requests into a cloned `axum::Router`. The bridge injects the QUIC peer address as
`ConnectInfo<SocketAddr>` and streams request and response bodies. Its compatibility entry point is
safe by default: `serve_h3` bounds global, per-IP, and per-connection-request concurrency;
`serve_h3_with_limits` accepts custom `H3ConnectionLimits`. Internal IPs are exempt from the per-IP
default but remain covered by the global cap. The generated Quinn configuration also uses a bounded
connection receive window. After a QUIC endpoint binds, `alt_svc_layer` adds the exact
`Alt-Svc: h3=":<port>"; ma=86400` advertisement to the separate HTTP/1.1 or HTTP/2 router.

WebSocket-over-HTTP/3 is outside this helper's scope; clients use TCP for WebSocket upgrades.

## 📚 Documentation

See `skills/http3-server/SKILL.md` for the embedding API and
`research/http3-quic-servers-design.md` for the transport architecture.

## License

MIT.
