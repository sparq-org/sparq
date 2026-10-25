//! Only the two dedicated ids may name the compiled V2 component members.
//! The opaque families never alias their packages, regardless of parameters.

use sparq_zk_compose::manifest::CircuitId;

/// The two V2 members' on-disk package names.
const V2_MEMBERS: [&str; 2] = ["scan_v2dl_k1_n16_r4", "join_eq_v2dl_na16_nb16"];

/// The two V2 members' own sources, compiled in. A moved or renamed member
/// breaks the build rather than silently disabling this file's claim.
const SCAN_V2DL_MAIN: &str =
    include_str!("../../../zk/compose/scan_v2dl_k1_n16_r4/src/main.nr");
const JOIN_V2DL_MAIN: &str =
    include_str!("../../../zk/compose/join_eq_v2dl_na16_nb16/src/main.nr");

/// Opaque scan/join and token FILTER names over parameters extending beyond
/// the compiled buckets. Dedicated V2 ids are checked separately below.
fn swept_package_names() -> Vec<String> {
    let mut out = Vec::new();
    for k in 0..6u32 {
        for n in [0u32, 1, 2, 4, 8, 16, 32, 64] {
            for r in 0..8u32 {
                out.push(CircuitId::Scan { k, n, r }.package().unwrap());
            }
        }
    }
    for d in 0..24u32 {
        out.push(CircuitId::FilterInt { d }.package().unwrap());
        out.push(CircuitId::FilterF64 { d }.package().unwrap());
    }
    for na in [0u32, 1, 2, 4, 8, 16, 32] {
        for nb in [0u32, 1, 2, 4, 8, 16, 32] {
            out.push(CircuitId::JoinEq { n_a: na, n_b: nb }.package().unwrap());
        }
    }
    out
}

/// No opaque family parameterization aliases a V2 package.
#[test]
fn no_opaque_circuit_id_names_a_v2_dual_leaf_member() {
    let swept = swept_package_names();
    for member in V2_MEMBERS {
        assert!(!swept.contains(&member.to_string()), "opaque family aliases {member}");
    }
}

/// Exactly the two dedicated variants produce the two package names.
#[cfg(feature = "dual-leaf")]
#[test]
fn exactly_the_two_v2_variants_name_the_v2_members() {
    let ids = [
        CircuitId::scan_v2_dl(1, 16, 4).unwrap(),
        CircuitId::join_eq_v2_dl(16, 16).unwrap(),
    ];
    for (id, name) in ids.iter().zip(V2_MEMBERS) {
        assert_eq!(id.package().unwrap(), name);
        assert_eq!(name.parse::<CircuitId>().unwrap(), *id);
        let json = serde_json::to_string(id).unwrap();
        assert_eq!(serde_json::from_str::<CircuitId>(&json).unwrap(), *id);
    }
    let mut names = Vec::new();
    for k in 0..6 {
        for n in [0, 8, 16, 32, 64] {
            for r in 0..9 {
                if let Ok(name) = (CircuitId::ScanV2Dl { k, n, r }).package() { names.push(name); }
            }
        }
    }
    for n_a in [0, 8, 16, 32, 64] {
        for n_b in [0, 8, 16, 32, 64] {
            if let Ok(name) = (CircuitId::JoinEqV2Dl { n_a, n_b }).package() { names.push(name); }
        }
    }
    names.sort();
    assert_eq!(names, vec![V2_MEMBERS[1], V2_MEMBERS[0]]);
}

#[cfg(feature = "dual-leaf")]
#[test]
fn every_checked_construction_path_refuses_uncompiled_v2_shapes() {
    use sparq_zk_compose::manifest::CircuitIdError;
    assert!(matches!(CircuitId::scan_v2_dl(2, 16, 4), Err(CircuitIdError::UncompiledV2Member(_))));
    assert!(matches!(CircuitId::join_eq_v2_dl(8, 16), Err(CircuitIdError::UncompiledV2Member(_))));
    for (id, name, json) in [
        (CircuitId::ScanV2Dl { k: 2, n: 16, r: 4 }, "scan_v2dl_k2_n16_r4",
            r#"{"kind":"scan_v2_dl","k":2,"n":16,"r":4}"#),
        (CircuitId::JoinEqV2Dl { n_a: 8, n_b: 16 }, "join_eq_v2dl_na8_nb16",
            r#"{"kind":"join_eq_v2_dl","n_a":8,"n_b":16}"#),
    ] {
        assert!(matches!(id.package(), Err(CircuitIdError::UncompiledV2Member(_))));
        assert!(matches!(name.parse::<CircuitId>(), Err(CircuitIdError::UncompiledV2Member(_))));
        assert!(serde_json::from_str::<CircuitId>(json).unwrap_err().to_string().contains("UncompiledV2Member"));
        assert!(serde_json::to_string(&id).unwrap_err().to_string().contains("UncompiledV2Member"));
        for &method in sparq_zk::commit::CommitmentMethod::recognised() {
            assert!(matches!(sparq_zk_compose::dispatch::resolve_circuit(method, &id),
                Err(sparq_zk_compose::dispatch::DispatchError::UncompiledCircuit(_))));
        }
    }
    assert!(CircuitId::scan_v2_dl(1, 16, 4).is_ok());
    assert!(CircuitId::join_eq_v2_dl(16, 16).is_ok());
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
            "the sweep must name the shipped member {}, or the non-alias assertion is vacuous",
            member
        );
    }
}

/// Both named packages execute their component-witnessed relations.
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
