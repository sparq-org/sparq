//! The outbound-address classifier every SSRF guard in sparq shares ([`is_forbidden_ip`]): one
//! copy of the IANA special-purpose registry tables and of the unwrapping of IPv6 forms that
//! carry an IPv4 address, so the guards cannot drift apart. Standard library only.

/// Whether a resolved [`IpAddr`](std::net::IpAddr) is one an outbound request must not be
/// made to reach: `true` unless the address is globally reachable unicast. The one predicate
/// every SSRF guard in sparq uses: SERVICE federation, the federation client's transports and
/// discovery, and the LWS server's outbound fetches (identity documents, OpenID Providers,
/// JWKS, webhook inboxes).
///
/// The rule is table-driven from the [IANA IPv4] and [IANA IPv6] Special-Purpose Address
/// Registries (`V4_DENY`, `V6_DENY`), honouring their "Globally Reachable" column
/// (`V4_GLOBAL`, `V6_GLOBAL` re-allow the reachable entries inside a denied block),
/// plus multicast, the reserved space, and on IPv6 everything outside global unicast
/// `2000::/3`. As a policy exclusion beyond the registry, ORCHIDv2 `2001:20::/28` and
/// drone remote ID tags `2001:30::/28` are refused too: they are identifiers, never
/// endpoint locators. Addresses that carry an IPv4 address are judged by it: IPv4-mapped
/// `::ffff:0:0/96`, IPv4-translated `::ffff:0:0:0/96`, NAT64 `64:ff9b::/96`, 6to4
/// `2002::/16` and Teredo `2001::/32` (forbidden if its server or de-obfuscated client
/// address is), so a private IPv4 cannot be smuggled through an IPv6 form.
///
/// [IANA IPv4]: https://www.iana.org/assignments/iana-ipv4-special-registry/
/// [IANA IPv6]: https://www.iana.org/assignments/iana-ipv6-special-registry/
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

/// IPv4 blocks that are not globally reachable: every IANA IPv4 Special-Purpose Address
/// Registry entry marked "Globally Reachable: False" (or deprecated), plus multicast.
const V4_DENY: &[(u32, u8)] = &[
    (v4(0, 0, 0, 0), 8),      // "this network" (incl. 0.0.0.0/32 "this host")
    (v4(10, 0, 0, 0), 8),     // private-use
    (v4(100, 64, 0, 0), 10),  // shared address space (CGNAT)
    (v4(127, 0, 0, 0), 8),    // loopback
    (v4(169, 254, 0, 0), 16), // link-local (incl. cloud metadata 169.254.169.254)
    (v4(172, 16, 0, 0), 12),  // private-use
    // IETF protocol assignments, incl. the /29 DS-Lite block, the .8 dummy address
    // and the .170/.171 NAT64 discovery addresses
    (v4(192, 0, 0, 0), 24),
    (v4(192, 0, 2, 0), 24),    // documentation TEST-NET-1
    (v4(192, 88, 99, 0), 24),  // deprecated 6to4 relay anycast
    (v4(192, 168, 0, 0), 16),  // private-use
    (v4(198, 18, 0, 0), 15),   // benchmarking
    (v4(198, 51, 100, 0), 24), // documentation TEST-NET-2
    (v4(203, 0, 113, 0), 24),  // documentation TEST-NET-3
    (v4(224, 0, 0, 0), 4),     // multicast
    (v4(240, 0, 0, 0), 4),     // reserved (incl. limited broadcast 255.255.255.255)
];

/// Registry entries marked globally reachable that sit inside a `V4_DENY` block.
const V4_GLOBAL: &[(u32, u8)] = &[
    (v4(192, 0, 0, 9), 32),  // Port Control Protocol anycast
    (v4(192, 0, 0, 10), 32), // TURN relay anycast
];

/// IPv6 blocks inside global unicast `2000::/3` that are not globally reachable (the
/// rest of the IPv6 registry lies outside `2000::/3`, which is refused wholesale).
const V6_DENY: &[(u128, u8)] = &[
    // IETF protocol assignments: benchmarking, ORCHID and the other identifier spaces
    (v6(0x2001, 0, 0, 0, 0, 0, 0, 0), 23),
    (v6(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0), 32), // documentation
    (v6(0x3fff, 0, 0, 0, 0, 0, 0, 0), 20),     // documentation
];

/// Registry entries marked globally reachable that sit inside a `V6_DENY` block.
const V6_GLOBAL: &[(u128, u8)] = &[
    (v6(0x2001, 1, 0, 0, 0, 0, 0, 1), 128), // Port Control Protocol anycast
    (v6(0x2001, 1, 0, 0, 0, 0, 0, 2), 128), // TURN relay anycast
    (v6(0x2001, 1, 0, 0, 0, 0, 0, 3), 128), // DNS-SD service registration protocol anycast
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
    // Forms that embed an IPv4 address are judged by that address.
    // IPv4-mapped, IPv4-translated and the NAT64 well-known prefix carry it in the low 32.
    const LOW32_V4: [(u128, u8); 3] = [
        (v6(0, 0, 0, 0, 0, 0xffff, 0, 0), 96),
        (v6(0, 0, 0, 0, 0xffff, 0, 0, 0), 96),
        (v6(0x64, 0xff9b, 0, 0, 0, 0, 0, 0), 96),
    ];
    let low32 = addr as u32;
    if LOW32_V4.iter().any(|b| in_v6(addr, b)) {
        return v4_forbidden(low32);
    }
    if in_v6(addr, &(v6(0x2002, 0, 0, 0, 0, 0, 0, 0), 16)) {
        return v4_forbidden((addr >> 80) as u32); // 6to4
    }
    if in_v6(addr, &(v6(0x2001, 0, 0, 0, 0, 0, 0, 0), 32)) {
        // Teredo: server IPv4 in bits 64..96, client IPv4 XOR-obfuscated in the low 32.
        return v4_forbidden((addr >> 64) as u32) || v4_forbidden(!low32);
    }
    // Only 2000::/3 is global unicast: loopback, unspecified, IPv4-compatible, discard,
    // local-use NAT64, SRv6 SIDs, unique-local, link-local, site-local and multicast all
    // fall outside it.
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
}
