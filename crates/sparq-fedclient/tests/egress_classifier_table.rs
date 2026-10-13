//! The fedclient SSRF guards (the source adapter's and discovery's) classify every address
//! exactly as the engine's SERVICE guard: they are the same function. One row per IANA
//! special-purpose registry entry, plus the public neighbours of the IPv4 blocks and every
//! IPv6 form that carries an IPv4 address.
#![cfg(all(feature = "fedclient", not(target_arch = "wasm32")))]

use std::net::IpAddr;

/// (address, refused?)
const TABLE: &[(&str, bool)] = &[
    // ── IPv4 Special-Purpose Address Registry ──
    ("0.0.0.0", true),           // this host
    ("0.1.2.3", true),           // this network 0/8
    ("10.0.0.1", true),          // private-use
    ("10.255.255.255", true),
    ("100.64.0.1", true),        // shared address space
    ("100.127.255.255", true),
    ("127.0.0.1", true),         // loopback
    ("127.255.255.254", true),
    ("169.254.0.1", true),       // link-local
    ("169.254.169.254", true),   // cloud metadata
    ("172.16.0.1", true),        // private-use
    ("172.31.255.255", true),
    ("192.0.0.1", true),         // IETF protocol assignments / DS-Lite
    ("192.0.0.8", true),         // IPv4 dummy address
    ("192.0.0.9", false),        // PCP anycast (globally reachable)
    ("192.0.0.10", false),       // TURN relay anycast (globally reachable)
    ("192.0.0.170", true),       // NAT64/DNS64 discovery
    ("192.0.0.171", true),
    ("192.0.2.1", true),         // TEST-NET-1
    ("192.31.196.1", false),     // AS112-v4 (globally reachable)
    ("192.52.193.1", false),     // AMT (globally reachable)
    ("192.88.99.1", true),       // deprecated 6to4 relay anycast
    ("192.88.99.2", true),       // 6a44 relay anycast (inside the deprecated block)
    ("192.168.0.1", true),       // private-use
    ("192.175.48.1", false),     // direct delegation AS112 (globally reachable)
    ("198.18.0.1", true),        // benchmarking
    ("198.19.255.255", true),
    ("198.51.100.1", true),      // TEST-NET-2
    ("203.0.113.1", true),       // TEST-NET-3
    ("240.0.0.1", true),         // reserved
    ("255.255.255.255", true),   // limited broadcast
    ("224.0.0.1", true),         // multicast
    ("239.255.255.255", true),
    // public neighbours of the IPv4 blocks
    ("1.0.0.1", false),
    ("9.255.255.255", false),
    ("11.0.0.0", false),
    ("100.63.255.255", false),
    ("100.128.0.1", false),
    ("126.255.255.255", false),
    ("128.0.0.1", false),
    ("169.253.0.1", false),
    ("172.15.255.255", false),
    ("172.32.0.1", false),
    ("192.0.1.1", false),
    ("192.88.98.1", false),
    ("192.169.0.1", false),
    ("198.17.255.255", false),
    ("198.20.0.1", false),
    ("203.0.114.1", false),
    ("223.255.255.255", false),
    ("8.8.8.8", false),
    ("1.1.1.1", false),
    ("93.184.216.34", false),
    // ── IPv6 Special-Purpose Address Registry ──
    ("::1", true),                    // loopback
    ("::", true),                     // unspecified
    ("::ffff:127.0.0.1", true),       // IPv4-mapped, judged by its IPv4
    ("::ffff:10.0.0.1", true),
    ("::ffff:169.254.169.254", true),
    ("::ffff:192.0.2.1", true),
    ("::ffff:100.64.0.1", true),
    ("::ffff:8.8.8.8", false),
    ("::ffff:0:7f00:1", true),        // IPv4-translated ::ffff:0:127.0.0.1
    ("::ffff:0:808:808", false),      // IPv4-translated public
    ("64:ff9b::7f00:1", true),        // NAT64 well-known prefix, judged by its IPv4
    ("64:ff9b::a00:1", true),
    ("64:ff9b::5db8:d822", false),    // NAT64 of public 93.184.216.34 (DNS64 networks)
    ("64:ff9b:1::808:808", true),     // local-use IPv4/IPv6 translation
    ("100::1", true),                 // discard-only
    ("100:0:0:1::1", true),           // dummy IPv6 prefix
    ("2001::1", true),                // Teredo with server 0.0.0.0
    ("2001:0:4136:e378:8000:63bf:f7f7:f7f7", false), // Teredo, public server + client 8.8.8.8
    ("2001:0:4136:e378:8000:63bf:80ff:fffe", true),  // Teredo, client 127.0.0.1
    ("2001:0:a00:1:8000:63bf:f7f7:f7f7", true),      // Teredo, server 10.0.0.1
    ("2001:1::1", false),             // PCP anycast (globally reachable)
    ("2001:1::2", false),             // TURN relay anycast (globally reachable)
    ("2001:1::3", false),             // DNS-SD SRP anycast (globally reachable)
    ("2001:1::4", true),
    ("2001:2::1", true),              // benchmarking
    ("2001:3::1", false),             // AMT (globally reachable)
    ("2001:4:112::1", false),         // AS112-v6 (globally reachable)
    ("2001:4:113::1", true),
    ("2001:10::1", true),             // deprecated ORCHID
    ("2001:20::1", true),             // ORCHIDv2 (an identifier, never a locator)
    ("2001:30::1", true),             // drone remote ID entity tags
    ("2001:1ff:ffff::1", true),       // end of IETF protocol assignments 2001::/23
    ("2001:200::1", false),           // first address past 2001::/23
    ("2001:db8::1", true),            // documentation
    ("2001:db9::1", false),
    ("2002:7f00:1::1", true),         // 6to4, judged by its IPv4
    ("2002:a9fe:a9fe::1", true),
    ("2002:808:808::1", false),
    ("2620:4f:8000::1", false),       // direct delegation AS112 (globally reachable)
    ("3fff::1", true),                // documentation
    ("3fff:fff::1", true),
    ("3fff:1000::1", false),
    ("5f00::1", true),                // SRv6 SIDs
    ("fc00::1", true),                // unique-local
    ("fdff::1", true),
    ("fe80::1", true),                // link-local
    ("febf::1", true),
    ("fec0::1", true),                // deprecated site-local
    ("ff02::1", true),                // multicast
    ("::127.0.0.1", true),            // deprecated IPv4-compatible
    ("::8.8.8.8", true),
    ("1fff:ffff::1", true),           // outside global unicast 2000::/3
    ("4000::1", true),
    ("2001:4860:4860::8888", false),  // public
    ("2606:4700:4700::1111", false),
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
