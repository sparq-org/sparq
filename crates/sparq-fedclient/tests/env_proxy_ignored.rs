//! The federation clients ignore proxy environment variables. Through a proxy, the proxy
//! resolves and dials the destination, so the SSRF resolver would vet the proxy's address
//! instead of the destination's. With `HTTP_PROXY` and friends pointing at a local listener
//! (allowlisted, so a proxied request would reach it), a private destination must still be
//! refused by the resolver and the listener must see no connection.
//!
//! This binary sets process environment variables, so it holds this one test only.
#![cfg(all(feature = "fedclient", not(target_arch = "wasm32")))]

use sparq_fedclient::discovery::{Fetcher, HttpFetcher};
use std::net::TcpListener;
use std::time::Duration;

#[test]
fn discovery_ignores_proxy_environment() {
    let proxy = TcpListener::bind("127.0.0.1:0").expect("bind proxy listener");
    proxy.set_nonblocking(true).expect("nonblocking listener");
    let proxy_addr = proxy.local_addr().expect("listener address");
    let proxy_url = format!("http://{proxy_addr}");
    for var in [
        "ALL_PROXY",
        "all_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
    ] {
        std::env::set_var(var, &proxy_url);
    }
    for var in ["NO_PROXY", "no_proxy"] {
        std::env::remove_var(var);
    }

    // 10.0.0.1 is private, so the resolver refuses it; the proxy itself is allowlisted.
    let fetcher = HttpFetcher::new()
        .with_timeout(Duration::from_secs(5))
        .allow_private_host(proxy_addr.to_string());
    let err = fetcher
        .get("http://10.0.0.1/sparql", "text/turtle")
        .expect_err("a private destination must be refused");

    assert!(
        err.contains("private/internal"),
        "the resolver must refuse the destination itself, got: {err}"
    );
    assert!(
        proxy.accept().is_err(),
        "the request must not be sent through the environment proxy"
    );
}
