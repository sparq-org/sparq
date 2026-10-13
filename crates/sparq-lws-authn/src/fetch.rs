//! Fetching for the authentication suites: identity documents, OpenID Provider metadata and
//! JWKS, under one policy. Unless [`Policy::allow_insecure_fetch`] is set, only `https:` URLs are
//! fetched, and never from loopback, private, link-local or otherwise non-global addresses: every
//! redirect hop is checked before it is requested (the client follows none itself), the client's
//! resolver hands out public addresses only, so the connection goes where the check looked, and
//! no proxy is used.

use std::net::IpAddr;
use std::sync::Arc;

/// Largest document a fetch reads.
pub const MAX_DOC: usize = 1024 * 1024;

/// What a fetch may reach.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Policy {
    /// Let a fetch reach `http:` and loopback or private addresses. Development and conformance
    /// testing only.
    pub allow_insecure_fetch: bool,
}

/// The scheme and host checks of the fetch policy, without name resolution.
pub fn check_url_static(cfg: &Policy, raw: &str) -> Result<url::Url, String> {
    let u = url::Url::parse(raw).map_err(|_| format!("not a URL: {raw}"))?;
    match u.scheme() {
        "https" => {}
        "http" if cfg.allow_insecure_fetch => {}
        _ => {
            return Err(format!(
                "refusing to fetch {raw}: only https URLs are dereferenced"
            ))
        }
    }
    if cfg.allow_insecure_fetch {
        return Ok(u);
    }
    match u.host() {
        Some(url::Host::Ipv4(ip)) if is_forbidden_ip(IpAddr::V4(ip)) => {
            Err(format!("refusing to fetch {raw}: non-public address"))
        }
        Some(url::Host::Ipv6(ip)) if is_forbidden_ip(IpAddr::V6(ip)) => {
            Err(format!("refusing to fetch {raw}: non-public address"))
        }
        Some(url::Host::Domain(d))
            if d.eq_ignore_ascii_case("localhost")
                || d.to_ascii_lowercase().ends_with(".localhost") =>
        {
            Err(format!("refusing to fetch {raw}: non-public address"))
        }
        None => Err(format!("not a URL with a host: {raw}")),
        _ => Ok(u),
    }
}

/// The full fetch policy: the static checks, then every address the host resolves to.
pub async fn check_url(cfg: &Policy, raw: &str) -> Result<url::Url, String> {
    let u = check_url_static(cfg, raw)?;
    if cfg.allow_insecure_fetch {
        return Ok(u);
    }
    if let Some(url::Host::Domain(d)) = u.host() {
        let port = u.port_or_known_default().unwrap_or(443);
        let addrs = tokio::net::lookup_host((d, port))
            .await
            .map_err(|_| format!("cannot resolve {d}"))?;
        let mut any = false;
        for a in addrs {
            any = true;
            if is_forbidden_ip(a.ip()) {
                return Err(format!(
                    "refusing to fetch {raw}: {d} resolves to a non-public address"
                ));
            }
        }
        if !any {
            return Err(format!("cannot resolve {d}"));
        }
    }
    Ok(u)
}

/// Redirects a fetch follows, each hop checked against the fetch policy before it is requested.
pub const MAX_REDIRECTS: usize = 3;

/// Where a redirect from `from` with `location` goes, resolved against `from`.
pub fn redirect_target(from: &url::Url, location: Option<&str>) -> Result<url::Url, String> {
    let location = location.ok_or_else(|| format!("{from} redirects without a Location"))?;
    from.join(location)
        .map_err(|_| format!("{from} redirects to an invalid location"))
}

/// GET `url` under the fetch policy: a 200 answer's media type and body (at most 1 MiB).
///
/// `http` must be built by [`fetch_client`]: it follows no redirects itself, so each hop is
/// checked (scheme, host and resolved addresses) before it is requested, and its resolver only
/// connects to public addresses, so a name cannot be re-pointed between check and connect.
pub async fn fetch(
    cfg: &Policy,
    http: &reqwest::Client,
    url: &str,
    accept: &str,
) -> Result<(String, Vec<u8>), String> {
    let mut u = check_url(cfg, url).await?;
    u.set_fragment(None);
    let mut hops = 0;
    let mut resp = loop {
        let resp = http
            .get(u.clone())
            .header(reqwest::header::ACCEPT, accept)
            .send()
            .await
            .map_err(|e| format!("cannot dereference {url}: {}", e.without_url()))?;
        if !resp.status().is_redirection() {
            break resp;
        }
        hops += 1;
        if hops > MAX_REDIRECTS {
            return Err(format!("{url} redirects too many times"));
        }
        let next = redirect_target(
            &u,
            resp.headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok()),
        )?;
        u = check_url(cfg, next.as_str()).await?;
        u.set_fragment(None);
    };
    if resp.status() != reqwest::StatusCode::OK {
        return Err(format!("GET {url} answered {}", resp.status().as_u16()));
    }
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    if resp.content_length().is_some_and(|n| n as usize > MAX_DOC) {
        return Err(format!("{url} is too large"));
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|_| format!("cannot read {url}"))?
    {
        body.extend_from_slice(&chunk);
        if body.len() > MAX_DOC {
            return Err(format!("{url} is too large"));
        }
    }
    Ok((content_type, body))
}

/// The client the authorization server fetches identity documents, OpenID provider metadata and
/// JWKS with (see [`fetch`]): no automatic redirects (each hop is checked before it
/// is requested), no proxy, and, unless `allow_insecure_fetch` is set, `https:` only through a
/// resolver that hands out public addresses alone, so the connection goes to an address that
/// passed the check (no DNS rebinding between check and connect).
pub fn fetch_client(cfg: &Policy) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .connect_timeout(std::time::Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy();
    if !cfg.allow_insecure_fetch {
        builder = builder
            .dns_resolver(Arc::new(PublicOnlyResolver))
            .https_only(true);
    }
    builder.build().map_err(|e| format!("http client: {e}"))
}

/// Resolves names to their public addresses only (see [`is_forbidden_ip`]), so an
/// outbound request (a fetch or a notification delivery) cannot be pointed at the server's own
/// network by a name that resolves there.
pub struct PublicOnlyResolver;

impl reqwest::dns::Resolve for PublicOnlyResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let addrs = public_addrs(tokio::net::lookup_host((host.as_str(), 0)).await?);
            if addrs.is_empty() {
                return Err(format!("{host} has no public address").into());
            }
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// The public addresses among `addrs`.
pub fn public_addrs(
    addrs: impl IntoIterator<Item = std::net::SocketAddr>,
) -> Vec<std::net::SocketAddr> {
    addrs
        .into_iter()
        .filter(|a| !is_forbidden_ip(a.ip()))
        .collect()
}

/// Whether an address is one the server must not be made to reach: anything but a globally
/// reachable unicast address. The one predicate every outbound request uses: identity
/// documents, OpenID Providers, JWKS and webhook inboxes alike.
///
/// IPv4: every IANA IPv4 Special-Purpose Address Registry block not globally reachable, plus
/// multicast and the reserved `240.0.0.0/4`. IPv6: everything outside global unicast
/// `2000::/3` (loopback, unspecified, IPv4-compatible, discard, unique-local, link-local,
/// site-local, local-use NAT64 `64:ff9b:1::/48`, multicast, ...) and the non-global registry
/// blocks inside it; a form that embeds an IPv4 address (IPv4-mapped or -translated, the NAT64
/// well-known prefix `64:ff9b::/96`, 6to4, Teredo) is refused when that address is. The same
/// set as the federation client's `is_forbidden_ip` (#6852).
pub fn is_forbidden_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => v4_forbidden(v4.to_bits()),
        std::net::IpAddr::V6(v6) => v6_forbidden(v6.to_bits()),
    }
}

const fn v4(a: u8, b: u8, c: u8, d: u8) -> u32 {
    std::net::Ipv4Addr::new(a, b, c, d).to_bits()
}

#[allow(clippy::too_many_arguments)]
const fn v6(a: u16, b: u16, c: u16, d: u16, e: u16, f: u16, g: u16, h: u16) -> u128 {
    std::net::Ipv6Addr::new(a, b, c, d, e, f, g, h).to_bits()
}

/// IPv4 blocks that are not globally reachable: the registry's "Globally Reachable: False" (or
/// deprecated) entries, plus multicast.
const V4_DENY: &[(u32, u8)] = &[
    (v4(0, 0, 0, 0), 8),       // "this network"
    (v4(10, 0, 0, 0), 8),      // private-use
    (v4(100, 64, 0, 0), 10),   // shared address space
    (v4(127, 0, 0, 0), 8),     // loopback
    (v4(169, 254, 0, 0), 16),  // link-local (cloud metadata included)
    (v4(172, 16, 0, 0), 12),   // private-use
    (v4(192, 0, 0, 0), 24),    // IETF protocol assignments
    (v4(192, 0, 2, 0), 24),    // documentation
    (v4(192, 88, 99, 0), 24),  // deprecated 6to4 relay anycast
    (v4(192, 168, 0, 0), 16),  // private-use
    (v4(198, 18, 0, 0), 15),   // benchmarking
    (v4(198, 51, 100, 0), 24), // documentation
    (v4(203, 0, 113, 0), 24),  // documentation
    (v4(224, 0, 0, 0), 4),     // multicast
    (v4(240, 0, 0, 0), 4),     // reserved, limited broadcast included
];

/// Registry entries marked globally reachable inside a [`V4_DENY`] block.
const V4_GLOBAL: &[(u32, u8)] = &[
    (v4(192, 0, 0, 9), 32),  // Port Control Protocol anycast
    (v4(192, 0, 0, 10), 32), // TURN relay anycast
];

/// IPv6 blocks inside `2000::/3` that are not globally reachable.
const V6_DENY: &[(u128, u8)] = &[
    (v6(0x2001, 0, 0, 0, 0, 0, 0, 0), 23), // IETF protocol assignments
    (v6(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0), 32), // documentation
    (v6(0x3fff, 0, 0, 0, 0, 0, 0, 0), 20), // documentation
];

/// Registry entries marked globally reachable inside a [`V6_DENY`] block.
const V6_GLOBAL: &[(u128, u8)] = &[
    (v6(0x2001, 1, 0, 0, 0, 0, 0, 1), 128), // Port Control Protocol anycast
    (v6(0x2001, 1, 0, 0, 0, 0, 0, 2), 128), // TURN relay anycast
    (v6(0x2001, 1, 0, 0, 0, 0, 0, 3), 128), // DNS-SD service registration anycast
    (v6(0x2001, 3, 0, 0, 0, 0, 0, 0), 32),  // AMT
    (v6(0x2001, 4, 0x112, 0, 0, 0, 0, 0), 48), // AS112-v6
];

fn in_v4(addr: u32, &(net, len): &(u32, u8)) -> bool {
    (addr ^ net).checked_shr(32 - u32::from(len)).unwrap_or(0) == 0
}

fn in_v6(addr: u128, &(net, len): &(u128, u8)) -> bool {
    (addr ^ net).checked_shr(128 - u32::from(len)).unwrap_or(0) == 0
}

fn v4_forbidden(addr: u32) -> bool {
    V4_DENY.iter().any(|b| in_v4(addr, b)) && !V4_GLOBAL.iter().any(|b| in_v4(addr, b))
}

fn v6_forbidden(addr: u128) -> bool {
    let low32 = addr as u32;
    // IPv4-mapped, IPv4-translated and NAT64 well-known-prefix addresses carry an IPv4 address
    // in the low 32 bits (on an IPv6-only network with DNS64, a public host is reached so).
    if in_v6(addr, &(v6(0, 0, 0, 0, 0, 0xffff, 0, 0), 96))
        || in_v6(addr, &(v6(0, 0, 0, 0, 0xffff, 0, 0, 0), 96))
        || in_v6(addr, &(v6(0x64, 0xff9b, 0, 0, 0, 0, 0, 0), 96))
    {
        return v4_forbidden(low32);
    }
    if in_v6(addr, &(v6(0x2002, 0, 0, 0, 0, 0, 0, 0), 16)) {
        return v4_forbidden((addr >> 80) as u32); // 6to4
    }
    if in_v6(addr, &(v6(0x2001, 0, 0, 0, 0, 0, 0, 0), 32)) {
        // Teredo: the server's IPv4 address in bits 64..96, the client's inverted in the low 32.
        return v4_forbidden((addr >> 64) as u32) || v4_forbidden(!low32);
    }
    !in_v6(addr, &(v6(0x2000, 0, 0, 0, 0, 0, 0, 0), 3))
        || (V6_DENY.iter().any(|b| in_v6(addr, b)) && !V6_GLOBAL.iter().any(|b| in_v6(addr, b)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Review note (#6852): the classifier refuses every non-global block, and judges a form
    /// that embeds an IPv4 address (mapped, translated, 6to4, Teredo) by that address.
    #[test]
    fn only_globally_reachable_addresses_are_allowed() {
        let forbidden = [
            "0.1.2.3",
            "10.0.0.1",
            "100.64.0.1",
            "127.0.0.1",
            "169.254.169.254",
            "172.16.0.1",
            "192.0.0.8",
            "192.0.2.1",
            "192.88.99.1",
            "192.168.1.1",
            "198.18.0.1",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "240.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "::127.0.0.1",
            "::ffff:10.0.0.1",
            "::ffff:0:a00:1",
            "64:ff9b::a00:1",
            "64:ff9b:1::808:808",
            "100::1",
            "2001:2::1",
            "2001:db8::1",
            "3fff::1",
            "fc00::1",
            "fe80::1",
            "fec0::1",
            "ff02::1",
            "2002:7f00:1::1",
            "2002:a9fe:a9fe::1",
            "2001:0:7f00:1::1",
            "2001:0:808:808::f5ff:fffe",
            "5f00::1",
        ];
        for a in forbidden {
            let ip: std::net::IpAddr = a.parse().unwrap();
            assert!(is_forbidden_ip(ip), "{a} should be forbidden");
        }
        let allowed = [
            "8.8.8.8",
            "1.1.1.1",
            "192.0.0.9",
            "192.0.0.10",
            "2606:4700::1111",
            "::ffff:8.8.8.8",
            "64:ff9b::5db8:d822",
            "2002:808:808::1",
            "2001:0:808:808::f7f7:f7f7",
            "2001:1::1",
            "2001:3::1",
            "2001:4:112::1",
        ];
        for a in allowed {
            let ip: std::net::IpAddr = a.parse().unwrap();
            assert!(!is_forbidden_ip(ip), "{a} should be allowed");
        }
    }

    fn cfg() -> Policy {
        Policy::default()
    }

    #[test]
    fn redirect_hops_resolve_against_the_current_url() {
        let from = url::Url::parse("https://id.example/a/b").unwrap();
        assert_eq!(
            redirect_target(&from, Some("/c")).unwrap().as_str(),
            "https://id.example/c"
        );
        assert_eq!(
            redirect_target(&from, Some("http://127.0.0.1/x"))
                .unwrap()
                .as_str(),
            "http://127.0.0.1/x"
        );
        assert!(redirect_target(&from, None).is_err());
        // Each hop then faces the policy: a redirect into loopback is refused before it is
        // requested.
        let cfg = cfg();
        assert!(check_url_static(&cfg, "http://127.0.0.1/x").is_err());
        assert!(check_url_static(&cfg, "https://127.0.0.1/x").is_err());
        assert!(check_url_static(&cfg, "https://[::1]/x").is_err());
    }

    /// A redirect to a loopback address is never requested: the fetch client follows no
    /// redirect, and the hop fails the policy check before any request goes out.
    #[tokio::test]
    async fn fetch_refuses_a_redirect_into_loopback() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        // The internal target: counts the requests that reach it.
        let internal = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let internal_addr = internal.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let h = Arc::clone(&hits);
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = internal.accept().await {
                h.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                let _ = sock
                    .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n{}")
                    .await;
            }
        });
        // The "attacker's" server redirects there.
        let outer = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let outer_addr = outer.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = outer.accept().await {
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                let resp = format!(
                    "HTTP/1.1 302 Found\r\nlocation: http://{internal_addr}/secret\r\ncontent-length: 0\r\n\r\n"
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            }
        });
        // The first hop is let through (as the insecure escape hatch would), but the policy is the
        // secure one for every hop after: model that by checking the redirect against a secure
        // configuration with the client the insecure one builds.
        let mut insecure = cfg();
        insecure.allow_insecure_fetch = true;
        let client = fetch_client(&insecure).unwrap();
        let resp = client
            .get(format!("http://{outer_addr}/me"))
            .send()
            .await
            .unwrap();
        // The client does not follow the redirect itself.
        assert_eq!(resp.status().as_u16(), 302);
        let next = redirect_target(
            &url::Url::parse(&format!("http://{outer_addr}/me")).unwrap(),
            resp.headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok()),
        )
        .unwrap();
        assert!(check_url(&cfg(), next.as_str()).await.is_err());
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        // And a secure fetch of a non-https or loopback URL never leaves the process.
        assert!(
            fetch(&cfg(), &client, &format!("http://{outer_addr}/me"), "*/*")
                .await
                .is_err()
        );
        assert_eq!(hits.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn the_resolver_keeps_public_addresses_only() {
        let addrs: Vec<std::net::SocketAddr> = [
            "127.0.0.1:443",
            "10.0.0.1:443",
            "169.254.169.254:80",
            "[::1]:443",
            "93.184.216.34:443",
        ]
        .iter()
        .map(|a| a.parse().unwrap())
        .collect();
        assert_eq!(
            public_addrs(addrs),
            vec!["93.184.216.34:443".parse::<std::net::SocketAddr>().unwrap()]
        );
    }

    #[test]
    fn fetch_policy() {
        let mut cfg = cfg();
        assert!(check_url_static(&cfg, "http://example.org/").is_err());
        assert!(check_url_static(&cfg, "https://127.0.0.1/").is_err());
        assert!(check_url_static(&cfg, "https://[::1]/").is_err());
        assert!(check_url_static(&cfg, "https://10.1.2.3/").is_err());
        assert!(check_url_static(&cfg, "https://169.254.169.254/").is_err());
        assert!(check_url_static(&cfg, "https://localhost/").is_err());
        assert!(check_url_static(&cfg, "file:///etc/passwd").is_err());
        assert!(check_url_static(&cfg, "https://example.org/").is_ok());
        assert!(check_url_static(&cfg, "https://8.8.8.8/").is_ok());
        cfg.allow_insecure_fetch = true;
        assert!(check_url_static(&cfg, "http://localhost:3918/agents/alice").is_ok());
        assert!(check_url_static(&cfg, "file:///etc/passwd").is_err());
    }
}
