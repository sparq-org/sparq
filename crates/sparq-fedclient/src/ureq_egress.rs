//! Shared ureq-3 SSRF-resolver helpers. sq-g2xs.
//!
//! ureq 3 (the migration off the unmaintained `rustls-pemfile`, RUSTSEC-2025-0134) changed the
//! [`Resolver`](ureq::unversioned::resolver::Resolver) trait: it now takes a parsed
//! [`http::Uri`](ureq::http::Uri) (plus the agent [`Config`](ureq::config::Config) and a
//! timeout) and returns an `ArrayVec<SocketAddr, 16>` ([`ResolvedSocketAddrs`]) rather than
//! ureq 2's `&str` netloc → `Vec<SocketAddr>`.
//!
//! The three sparq-fedclient transports (`source::HttpTransport`,
//! `source::HttpFragmentTransport`, `discovery::HttpFetcher`) install a custom resolver that
//! enforces the **same** default-deny SSRF policy as the engine's SERVICE transport: resolve the
//! host, drop every private/internal address (unless an allowlist entry permits the host on the
//! dialled port), and return only the survivors — so ureq dials only vetted IPs and there is no
//! DNS-rebinding re-resolve window. The resolve itself is the engine's
//! [`sparq_engine::resolve_vetted`], so both clients run one implementation; this module holds
//! the fedclient resolver ([`EgressFilterResolver`]) and the client built around it
//! ([`GuardedHttp`]). Native-only; only built when the `fedclient` feature pulls ureq in.

/// True iff any allowlist `entry` permits dialling `host` on `port`, using the engine's shared
/// SERVICE-egress per-entry rule ([`sparq_engine::allowlist_entry_permits`]). The federation
/// SSRF resolvers call this instead of a bare `allow.contains(host)`, so a PORT-SCOPED allowlist
/// entry (`host:port`) re-opens a private host ONLY on its exact dialled port — byte-for-byte the
/// same host:port matching the engine guard applies (one source of truth, bead sq-vbnyc). A
/// host-level entry (no `:port`) still re-opens every port (backward compatible).
pub(crate) fn allowlist_permits(
    allow: &std::collections::HashSet<String>,
    host: &str,
    port: u16,
) -> bool {
    allow
        .iter()
        .any(|entry| sparq_engine::allowlist_entry_permits(entry, host, port))
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
        // Port-scoped allowlist: a `host:port` entry re-opens a private host only on its exact
        // dialled port (shared host:port rule with the engine SERVICE guard; sq-vbnyc).
        let allowed = |host: &str, port: u16| allowlist_permits(&self.allow_private, host, port);
        sparq_engine::resolve_vetted(uri, allowed, false, |refusal| match refusal {
            sparq_engine::EgressRefusal::PrivateOnly { host_port } => format!(
                "{who} egress refused: {host_port} resolves only to private/internal \
                 addresses (default-deny SSRF policy; {hint})"
            ),
            sparq_engine::EgressRefusal::NoAuthority => {
                format!("{who} egress refused: request URI {uri} has no host authority to vet")
            }
            // Only a strict resolve refuses before DNS; this one is not strict.
            sparq_engine::EgressRefusal::NotAllowlisted { host } => {
                format!("{who} egress refused: host {host:?} is not allowlisted")
            }
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

// sq-3dyje.6 (mutation-kill): direct unit tests for this `pub(crate)` module. The
// shared resolve's own tests (URI parse, address filter, refusal kinds) live with
// `resolve_vetted` in sparq-engine-service.
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

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
