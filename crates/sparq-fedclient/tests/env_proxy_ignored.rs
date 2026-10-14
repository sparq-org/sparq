//! The federation clients ignore proxy environment variables. Through a proxy, the proxy
//! resolves and dials the destination, so the SSRF resolver would vet the proxy's address
//! instead of the destination's. With `HTTP_PROXY` and friends pointing at a local listener
//! (allowlisted, so a proxied request would reach it), a private destination must still be
//! refused by the resolver and the listener must see no connection.
//!
//! Each proxy variable is tried alone, for `http` and `https` destinations, and once more
//! with all of them set and a populated `NO_PROXY` that does not match the destination.
//!
//! This binary sets process environment variables, so it holds this one test only and runs
//! its cases in sequence.
#![cfg(all(feature = "fedclient", not(target_arch = "wasm32")))]

use sparq_fedclient::discovery::{Fetcher, HttpFetcher};
use std::net::TcpListener;
use std::time::Duration;

const PROXY_VARS: [&str; 6] = [
    "ALL_PROXY",
    "all_proxy",
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
];

/// Points `vars` at a fresh local listener (clearing the other proxy variables and setting
/// `NO_PROXY` to `no_proxy`), fetches the private `dest`, and checks the resolver refused
/// the destination itself without contacting the listener.
fn assert_proxy_ignored(case: &str, vars: &[&str], no_proxy: Option<&str>, dest: &str) {
    let proxy = TcpListener::bind("127.0.0.1:0").expect("bind proxy listener");
    proxy.set_nonblocking(true).expect("nonblocking listener");
    let proxy_addr = proxy.local_addr().expect("listener address");
    for var in PROXY_VARS {
        std::env::remove_var(var);
    }
    for var in vars {
        std::env::set_var(var, format!("http://{proxy_addr}"));
    }
    for var in ["NO_PROXY", "no_proxy"] {
        match no_proxy {
            Some(list) => std::env::set_var(var, list),
            None => std::env::remove_var(var),
        }
    }

    // 10.0.0.1 is private, so the resolver refuses it; the proxy itself is allowlisted.
    let fetcher = HttpFetcher::new()
        .with_timeout(Duration::from_secs(5))
        .allow_private_host(proxy_addr.to_string());
    let err = fetcher
        .get(dest, "text/turtle")
        .expect_err("a private destination must be refused");
    assert!(
        err.contains("private/internal") && err.contains("10.0.0.1"),
        "{case}: the resolver must refuse the destination itself, got: {err}"
    );
    assert!(
        proxy.accept().is_err(),
        "{case}: the request must not be sent through the environment proxy"
    );
}

#[test]
fn discovery_ignores_proxy_environment() {
    for dest in ["http://10.0.0.1/sparql", "https://10.0.0.1/sparql"] {
        for var in PROXY_VARS {
            assert_proxy_ignored(&format!("{var} {dest}"), &[var], None, dest);
        }
    }
    assert_proxy_ignored(
        "all variables, unrelated NO_PROXY",
        &PROXY_VARS,
        Some("example.org,.internal.test"),
        "http://10.0.0.1/sparql",
    );
}
