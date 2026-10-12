//! The fedclient SSRF guards (the source adapter's and discovery's) classify every address
//! range exactly as the engine's SERVICE guard: they are the same function.
#![cfg(all(feature = "fedclient", not(target_arch = "wasm32")))]

use std::net::IpAddr;

/// (address, refused?) for each range the default-deny policy covers, plus the public
/// neighbours on either side of each IPv4 range.
const TABLE: &[(&str, bool)] = &[
    // loopback
    ("127.0.0.1", true),
    ("127.255.255.254", true),
    ("::1", true),
    // RFC1918 private
    ("10.0.0.1", true),
    ("10.255.255.255", true),
    ("172.16.0.1", true),
    ("172.31.255.255", true),
    ("172.15.255.255", false),
    ("172.32.0.1", false),
    ("192.168.0.1", true),
    ("192.169.0.1", false),
    // link-local, including cloud metadata
    ("169.254.0.1", true),
    ("169.254.169.254", true),
    ("fe80::1", true),
    ("febf::1", true),
    ("fec0::1", false),
    // unique-local IPv6
    ("fc00::1", true),
    ("fdff::1", true),
    ("fe00::1", false),
    // unspecified
    ("0.0.0.0", true),
    ("::", true),
    // broadcast and CGNAT
    ("255.255.255.255", true),
    ("100.64.0.1", true),
    ("100.127.255.255", true),
    ("100.63.255.255", false),
    ("100.128.0.1", false),
    // IPv4-mapped IPv6 is classified as the embedded IPv4
    ("::ffff:127.0.0.1", true),
    ("::ffff:10.0.0.1", true),
    ("::ffff:169.254.169.254", true),
    ("::ffff:100.64.0.1", true),
    ("::ffff:8.8.8.8", false),
    // public
    ("8.8.8.8", false),
    ("1.1.1.1", false),
    ("2001:4860:4860::8888", false),
];

#[test]
fn fedclient_guards_classify_like_the_engine() {
    for &(addr, refused) in TABLE {
        let ip: IpAddr = addr.parse().unwrap();
        assert_eq!(sparq_engine::is_forbidden_ip(ip), refused, "engine: {addr}");
        assert_eq!(sparq_fedclient::is_forbidden_ip(ip), refused, "source: {addr}");
        assert_eq!(sparq_fedclient::discovery::is_forbidden_ip(ip), refused, "discovery: {addr}");
    }
}
