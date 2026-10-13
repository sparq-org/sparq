//! Shared ureq-3 SSRF-resolver helpers. [OPUS-4.8] sq-g2xs.
//!
//! ureq 3 (the migration off the unmaintained `rustls-pemfile`, RUSTSEC-2025-0134) changed the
//! [`Resolver`](ureq::unversioned::resolver::Resolver) trait: it now takes a parsed
//! [`http::Uri`](ureq::http::Uri) (plus the agent [`Config`](ureq::config::Config) and a
//! timeout) and returns an `ArrayVec<SocketAddr, 16>` ([`ResolvedSocketAddrs`]) rather than
//! ureq 2's `&str` netloc → `Vec<SocketAddr>`.
//!
//! The three sparq-fedclient transports (`source::HttpTransport`,
//! `source::HttpFragmentTransport`, `discovery::HttpFetcher`) install a custom resolver that
//! enforces the **same** default-deny SSRF policy: resolve the host, drop every private/internal
//! address (unless the bare host is explicitly allowlisted), and return only the survivors — so
//! ureq dials only vetted IPs and there is no DNS-rebinding re-resolve window. This module holds
//! that resolver ([`EgressFilterResolver`]) and the client built around it ([`GuardedHttp`]), plus the
//! ureq-3 helpers behind them (URI→host:port parse, the capacity-bounded filtered resolve, the
//! refusal error), so those three transports share one implementation. Native-only; only
//! built when the `fedclient` feature pulls ureq in.
//!
//! The engine SERVICE resolver (`sparq-engine`'s `service.rs`) applies the *same* egress/SSRF
//! logic, but keeps its own equivalent inline copy of these helpers because `sparq-engine` must
//! not depend on `sparq-fedclient`. The two implementations are equivalent and unit-tested (not
//! externally security-audited); they do not share this module.

use std::net::IpAddr;
use std::net::ToSocketAddrs;

/// Capacity of ureq-3's [`ResolvedSocketAddrs`] (`ArrayVec<SocketAddr, 16>`); matches ureq's own
/// `MAX_ADDRS`. We never push more than this, so `ArrayVec::push` cannot overrun its array.
const RESOLVED_ADDRS_CAP: usize = 16;

/// `host:port` (for resolution) + the bare host (the allowlist key — IPv6 brackets stripped,
/// lowercased) + the numeric `port` from a ureq-3 request [`Uri`](ureq::http::Uri). `port` falls
/// back to the scheme default (443 for https, 80 otherwise). `None` when the URI carries no host
/// authority. The numeric port lets the SSRF resolver apply a PORT-SCOPED allowlist entry (a
/// `host:port` allowlist entry permits only that exact dialled port), consistent with the engine's
/// SERVICE egress guard. [OPUS-4.8] sq-vbnyc.
pub(crate) fn uri_host_port(uri: &ureq::http::Uri) -> Option<(String, String, u16)> {
    let authority = uri.authority()?;
    let host = authority.host();
    if host.is_empty() {
        return None;
    }
    let port = authority.port_u16().unwrap_or(match uri.scheme_str() {
        Some("https") => 443,
        _ => 80,
    });
    // The authority host keeps IPv6 brackets (`[::1]`); strip them for the allowlist key and for
    // `to_socket_addrs` (which wants the bare host + a separate port).
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    Some((format!("{bare}:{port}"), bare.to_ascii_lowercase(), port))
}

/// True iff any allowlist `entry` permits dialling `host` on `port`, using the engine's shared
/// SERVICE-egress per-entry rule ([`sparq_engine::allowlist_entry_permits`]). The federation
/// SSRF resolvers call this instead of a bare `allow.contains(host)`, so a PORT-SCOPED allowlist
/// entry (`host:port`) re-opens a private host ONLY on its exact dialled port — byte-for-byte the
/// same host:port matching the engine guard applies (one source of truth, bead sq-vbnyc). A
/// host-level entry (no `:port`) still re-opens every port (backward compatible). [OPUS-4.8].
pub(crate) fn allowlist_permits(
    allow: &std::collections::HashSet<String>,
    host: &str,
    port: u16,
) -> bool {
    allow
        .iter()
        .any(|entry| sparq_engine::allowlist_entry_permits(entry, host, port))
}

/// Wrap a refusal `reason` as a `PermissionDenied` [`ureq::Error::Io`], preserving the kind (the
/// resolver tests assert on it) and the reason text.
pub(crate) fn egress_refused(reason: String) -> ureq::Error {
    ureq::Error::Io(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        reason,
    ))
}

/// An empty [`ResolvedSocketAddrs`] backing store (a fixed-capacity `ArrayVec`; logical length 0,
/// the same idiom ureq's own `DefaultResolver::empty` uses).
fn empty_resolved() -> ureq::unversioned::resolver::ResolvedSocketAddrs {
    use std::net::{Ipv4Addr, SocketAddr};
    ureq::unversioned::resolver::ArrayVec::from_fn(|_| {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0)
    })
}

/// Resolve `host_port`, keep only addresses that pass the SSRF policy, and return them as a
/// ureq-3 [`ResolvedSocketAddrs`]. An `allowed` host (on the resolver's allowlist) bypasses the
/// `forbidden` classifier; otherwise every `forbidden(ip)` address is dropped. If nothing
/// survives, refuse with a `PermissionDenied` error carrying `refusal()` — a HARD error, never an
/// empty set, so ureq cannot fall through to an unguarded dial.
pub(crate) fn filter_resolved(
    host_port: &str,
    allowed: bool,
    forbidden: fn(IpAddr) -> bool,
    refusal: impl FnOnce() -> String,
) -> Result<ureq::unversioned::resolver::ResolvedSocketAddrs, ureq::Error> {
    let resolved = host_port.to_socket_addrs().map_err(ureq::Error::Io)?;
    let mut permitted = empty_resolved();
    for sa in resolved
        .filter(|sa| allowed || !forbidden(sa.ip()))
        .take(RESOLVED_ADDRS_CAP)
    {
        permitted.push(sa);
    }
    if permitted.is_empty() {
        return Err(egress_refused(refusal()));
    }
    Ok(permitted)
}

/// Which client is refusing, for the refusal text: the federation transports or discovery.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Client {
    /// `source::HttpTransport` and `source::HttpFragmentTransport`.
    Federation,
    /// `discovery::HttpFetcher`.
    Discovery,
}

/// The ureq resolver every sparq-fedclient HTTP transport installs. It resolves the host, drops
/// every [`sparq_engine::is_forbidden_ip`] address unless an `allow_private` entry permits the
/// host on the dialled port, and returns only the survivors, so ureq dials only vetted IPs and
/// there is no resolve-then-re-resolve gap (sq-25xk).
#[derive(Debug)]
pub(crate) struct EgressFilterResolver {
    pub(crate) allow_private: std::sync::Arc<std::collections::HashSet<String>>,
    pub(crate) client: Client,
}

impl ureq::unversioned::resolver::Resolver for EgressFilterResolver {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        _config: &ureq::config::Config,
        _timeout: ureq::unversioned::transport::NextTimeout,
    ) -> Result<ureq::unversioned::resolver::ResolvedSocketAddrs, ureq::Error> {
        let (who, hint) = match self.client {
            Client::Federation => ("federation", "allowlist the host on the EgressGuard"),
            Client::Discovery => ("discovery", "allow it with HttpFetcher::allow_private_host"),
        };
        let (host_port, host, port) = uri_host_port(uri).ok_or_else(|| {
            egress_refused(format!(
                "{who} egress refused: request URI {uri} has no host authority to vet"
            ))
        })?;
        // Port-scoped allowlist: a `host:port` entry re-opens a private host only on its exact
        // dialled port (shared host:port rule with the engine SERVICE guard; sq-vbnyc).
        let allowed = allowlist_permits(&self.allow_private, &host, port);
        filter_resolved(&host_port, allowed, sparq_engine::is_forbidden_ip, || {
            format!(
                "{who} egress refused: {host_port} resolves only to private/internal \
                 addresses (default-deny SSRF policy; {hint})"
            )
        })
    }
}

/// The settings of one SSRF-guarded HTTP client (the federation transports or the discovery
/// fetcher): its global timeout, the hosts its [`EgressFilterResolver`] lets resolve privately,
/// and which client it is, which sets the default timeout, the body cap and the error texts.
#[derive(Debug, Clone)]
pub(crate) struct GuardedHttp {
    pub(crate) timeout: std::time::Duration,
    pub(crate) allow_private: std::sync::Arc<std::collections::HashSet<String>>,
    client: Client,
}

impl GuardedHttp {
    /// A client with `client`'s default timeout, finite so a black-holed host fails rather than
    /// hangs: 30s for federation (the engine's SERVICE default) and 15s for discovery.
    pub(crate) fn new(client: Client, allow_private: std::collections::HashSet<String>) -> Self {
        let timeout = std::time::Duration::from_secs(match client {
            Client::Federation => 30,
            Client::Discovery => 15,
        });
        GuardedHttp {
            timeout,
            allow_private: std::sync::Arc::new(allow_private),
            client,
        }
    }

    /// A ureq agent with the global timeout, the sparq-fedclient user agent and the SSRF
    /// resolver. ureq connects only to the addresses the resolver returns, so the vetted IP is
    /// the dialled IP and there is no DNS-rebinding re-resolve window (sq-25xk, sq-g2xs).
    pub(crate) fn agent(&self) -> ureq::Agent {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(self.timeout))
            .user_agent(concat!("sparq-fedclient/", env!("CARGO_PKG_VERSION")))
            .build();
        let resolver = EgressFilterResolver {
            allow_private: std::sync::Arc::clone(&self.allow_private),
            client: self.client,
        };
        ureq::Agent::with_parts(
            config,
            ureq::unversioned::transport::DefaultConnector::new(),
            resolver,
        )
    }

    /// The body of the response to a request for `url`, or the error text for a transport
    /// failure or a non-2xx status (ureq 3 reports both as errors). ureq's default
    /// `read_to_string` cap of 10 MiB is raised because a federated SELECT or TPF page can
    /// exceed it; the cap stays finite so memory stays bounded: 1 GiB for federation, 256 MiB
    /// for a discovery document. `fragment` names a TPF fragment request in the error text.
    pub(crate) fn read_body(
        &self,
        response: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
        url: &str,
        fragment: bool,
    ) -> Result<String, String> {
        let (who, limit, server) = match (self.client, fragment) {
            (Client::Federation, false) => ("fedclient", 1 << 30, "endpoint "),
            (Client::Federation, true) => ("fedclient", 1 << 30, "fragment server "),
            (Client::Discovery, _) => ("discovery", 256 << 20, ""),
        };
        let body = if fragment { "fragment" } else { "response" };
        match response {
            Ok(mut r) => r
                .body_mut()
                .with_config()
                .limit(limit)
                .read_to_string()
                .map_err(|e| format!("{who}: reading {body} from {url}: {e}")),
            Err(ureq::Error::StatusCode(code)) => {
                Err(format!("{who}: {server}{url} returned HTTP {code}"))
            }
            Err(e) => Err(format!("{who}: request to {url} failed: {e}")),
        }
    }
}

// [FABLE-5] sq-3dyje.6 (mutation-kill): DIRECT unit tests for this `pub(crate)` module.
// The module previously had NO inline tests — it was exercised only end-to-end through the
// three transports' loopback tests, which never pin these helpers' individual outputs, so
// cargo-mutants return-value/operator mutations here survived. Every assertion below pins a
// specific value: the URI→(host_port, host, port) triple including scheme-default ports and
// IPv6 bracket handling, the refusal error's exact `ErrorKind` + reason, and the resolved-
// address filtering decisions (allowlist bypass / forbidden drop / empty ⇒ hard error).
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::net::{Ipv4Addr, SocketAddr};

    fn uri(s: &str) -> ureq::http::Uri {
        s.parse().expect("test URI parses")
    }

    #[test]
    fn uri_host_port_explicit_port_and_lowercasing() {
        assert_eq!(
            uri_host_port(&uri("http://Example.ORG:8080/sparql")),
            Some(("Example.ORG:8080".to_string(), "example.org".to_string(), 8080)),
            "host_port keeps the authority spelling for resolution; the allowlist key is lowercased"
        );
    }

    #[test]
    fn uri_host_port_scheme_default_ports() {
        assert_eq!(
            uri_host_port(&uri("https://example.org/q")),
            Some(("example.org:443".to_string(), "example.org".to_string(), 443)),
            "https defaults to 443"
        );
        assert_eq!(
            uri_host_port(&uri("http://example.org/q")),
            Some(("example.org:80".to_string(), "example.org".to_string(), 80)),
            "http (and anything non-https) defaults to 80"
        );
    }

    #[test]
    fn uri_host_port_strips_ipv6_brackets() {
        assert_eq!(
            uri_host_port(&uri("http://[::1]:8053/q")),
            Some(("::1:8053".to_string(), "::1".to_string(), 8053)),
            "the bare (bracket-stripped) host is both the resolve target's host part and the allowlist key"
        );
    }

    #[test]
    fn uri_host_port_no_authority_is_none() {
        // A relative-form URI carries no authority to vet.
        assert_eq!(uri_host_port(&uri("/no-authority")), None);
    }

    #[test]
    fn egress_refused_is_permission_denied_with_reason() {
        let err = egress_refused("blocked: policy".to_string());
        match err {
            ureq::Error::Io(io) => {
                assert_eq!(io.kind(), std::io::ErrorKind::PermissionDenied);
                assert_eq!(io.to_string(), "blocked: policy");
            }
            other => panic!("expected Error::Io(PermissionDenied), got {:?}", other),
        }
    }

    /// 127.0.0.1:<port> resolves locally with no DNS, deterministically.
    const LOOP: &str = "127.0.0.1:8080";

    fn always(_: IpAddr) -> bool {
        true
    }
    fn never(_: IpAddr) -> bool {
        false
    }

    #[test]
    fn filter_resolved_allowed_bypasses_the_classifier() {
        // allowed=true must return the address EVEN THOUGH the classifier forbids it —
        // the allowlist bypass (`allowed || !forbidden`; the `&&` mutation fails here).
        let got = filter_resolved(LOOP, true, always, || unreachable!("no refusal"))
            .expect("allowlisted host resolves");
        let want: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8080);
        assert!(!got.is_empty(), "the resolved set must not be empty");
        assert_eq!(got[0], want, "the loopback address survives the filter");
    }

    #[test]
    fn filter_resolved_forbidden_addresses_are_a_hard_error() {
        // Not allowlisted + every address forbidden ⇒ a HARD PermissionDenied carrying the
        // refusal text — never an empty Ok (ureq would fall through to an unguarded dial).
        let err = filter_resolved(LOOP, false, always, || "refused: test-policy".to_string())
            .expect_err("all-forbidden must refuse");
        match err {
            ureq::Error::Io(io) => {
                assert_eq!(io.kind(), std::io::ErrorKind::PermissionDenied);
                assert_eq!(io.to_string(), "refused: test-policy");
            }
            other => panic!("expected Error::Io(PermissionDenied), got {:?}", other),
        }
    }

    #[test]
    fn filter_resolved_permitted_addresses_pass_unallowlisted() {
        // Not allowlisted but the classifier permits ⇒ the address flows through (the
        // `!forbidden` — deleting the `!` fails here).
        let got = filter_resolved(LOOP, false, never, || unreachable!("no refusal"))
            .expect("permitted address resolves");
        assert_eq!(
            got[0],
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8080)
        );
    }

    #[test]
    fn allowlist_permits_matches_host_and_port_scoped_entries() {
        let mut allow: HashSet<String> = HashSet::new();
        allow.insert("open.example".to_string());
        allow.insert("scoped.example:8443".to_string());
        // Host-level entry: every port.
        assert!(allowlist_permits(&allow, "open.example", 80));
        assert!(allowlist_permits(&allow, "open.example", 65535));
        // Port-scoped entry: exactly its port.
        assert!(allowlist_permits(&allow, "scoped.example", 8443));
        assert!(!allowlist_permits(&allow, "scoped.example", 8444));
        // Absent host: refused.
        assert!(!allowlist_permits(&allow, "absent.example", 8443));
        // Empty allowlist: nothing permitted.
        assert!(!allowlist_permits(&HashSet::new(), "open.example", 80));
    }
}
