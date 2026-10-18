//! The V2 component-witnessed members are NOT verifier-acceptable (sq-j506).
//!
//! `zk/compose/scan_v2dl_k1_n16_r4` and `zk/compose/join_eq_v2dl_na16_nb16`
//! recompute their graphs' term encodings from witnessed `DualLeafV2` leaf
//! components. They are shipped as circuit packages and nothing more: this PR
//! wires no verifier acceptance for them, and the value-lane / V2-scan gates stay
//! closed.
//!
//! The ONE thing that makes that true is [`CircuitId::package`] — the single
//! function the verifier's member resolution, id derivation and canonical-vk
//! lookup all name a member through. No `CircuitId` variant produces either
//! package name, so no `ProofManifest` can name a V2 member and no sub-proof can
//! be routed to one. This file asserts exactly that, over a sweep of the shipped
//! id space, and compiles in each member's own `main.nr` so the claim is about
//! real members rather than absent ones -- a moved or renamed member is a COMPILE
//! error here, not a silently-passing test.
//!
//! When sq-j506 opens the V2 scan lane it will add a `CircuitId` variant for
//! these members. This test is what turns red at that moment, so the gates are
//! opened deliberately rather than by a member becoming nameable.

use sparq_zk_compose::manifest::CircuitId;

/// The two V2 members' on-disk package names.
const V2_MEMBERS: [&str; 2] = ["scan_v2dl_k1_n16_r4", "join_eq_v2dl_na16_nb16"];

/// The two V2 members' own sources, compiled in. A moved or renamed member
/// breaks the build rather than silently disabling this file's claim.
const SCAN_V2DL_MAIN: &str =
    include_str!("../../../zk/compose/scan_v2dl_k1_n16_r4/src/main.nr");
const JOIN_V2DL_MAIN: &str =
    include_str!("../../../zk/compose/join_eq_v2dl_na16_nb16/src/main.nr");

/// Every `CircuitId` the shipped id space can form over a sweep that covers each
/// variant's parameters well past the compiled members. A name no `CircuitId`
/// produces cannot be reached by any manifest.
fn swept_package_names() -> Vec<String> {
    let mut out = Vec::new();
    for k in 0..6u32 {
        for n in [0u32, 1, 2, 4, 8, 16, 32, 64] {
            for r in 0..8u32 {
                out.push(CircuitId::Scan { k, n, r }.package());
            }
        }
    }
    for d in 0..24u32 {
        out.push(CircuitId::FilterInt { d }.package());
        out.push(CircuitId::FilterF64 { d }.package());
    }
    for na in [0u32, 1, 2, 4, 8, 16, 32] {
        for nb in [0u32, 1, 2, 4, 8, 16, 32] {
            out.push(CircuitId::JoinEq { n_a: na, n_b: nb }.package());
        }
    }
    out
}

/// The guard. Neither V2 member is nameable by any swept `CircuitId`, so neither
/// is reachable through the verifier's member resolution.
#[test]
fn no_circuit_id_names_a_v2_dual_leaf_member() {
    let swept = swept_package_names();
    for member in V2_MEMBERS {
        assert!(
            !swept.contains(&member.to_string()),
            "{} is nameable by a CircuitId: the V2 component-witnessed member has become verifier-reachable, which sq-j506 must open deliberately",
            member
        );
    }
}

/// The CONTROL. The sweep genuinely produces the SHIPPED members' names, so the
/// assertion above is about a name the sweep could have found and did not — not
/// about a sweep that produces nothing.
#[test]
fn the_sweep_names_the_shipped_scan_and_join_members() {
    let swept = swept_package_names();
    for member in ["scan_k1_n16_r4", "join_eq_na16_nb16"] {
        assert!(
            swept.contains(&member.to_string()),
            "the sweep must name the shipped member {}, or the unreachability assertion is vacuous",
            member
        );
    }
}

/// And the two V2 members are REAL members whose relations go through the
/// component recompute, so the unreachability above is a statement about members
/// that exist and do the work the gates are being kept closed over.
#[test]
fn both_v2_dual_leaf_members_call_the_component_recompute() {
    for (member, source, relation) in [
        (V2_MEMBERS[0], SCAN_V2DL_MAIN, "scan_v2_dual_leaf_check"),
        (V2_MEMBERS[1], JOIN_V2DL_MAIN, "join_eq_v2_dual_leaf_check"),
    ] {
        assert!(
            source.contains(relation),
            "{} must run the component-witnessed relation {}",
            member,
            relation
        );
    }
}
